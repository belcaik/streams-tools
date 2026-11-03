use super::{YoutubeEvent, YoutubeSettings, map, resolve};
use crate::common::{RecentIds, next_backoff, system};
use serde_json::{Value, json};
use std::{error::Error, fmt, time::Duration};
use streams_tools_core::{Bus, Event, EventData, Platform};
use tokio::{sync::watch, task::JoinHandle};

pub struct YoutubeAdapter {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

#[derive(Debug)]
pub struct AdapterError(String);

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for AdapterError {}

impl YoutubeAdapter {
    pub fn start(bus: Bus, settings: YoutubeSettings) -> Result<Self, AdapterError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("streams-tools/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| AdapterError(error.to_string()))?;
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(run(bus, settings, http, receiver));
        Ok(Self { shutdown, task })
    }

    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        let _ = (&mut self.task).await;
    }
}

impl Drop for YoutubeAdapter {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}

enum SessionEnd {
    Offline,
    Dropped(String),
    Shutdown,
}

async fn run(
    bus: Bus,
    settings: YoutubeSettings,
    http: reqwest::Client,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut dedupe = RecentIds::new();
    let mut backoff = Duration::ZERO;
    loop {
        if *shutdown.borrow() {
            return;
        }
        let resolved = tokio::select! {
            value = resolve::live(&settings, &http) => value,
            _ = shutdown.changed() => return,
        };
        let video_id = match resolved {
            Ok(resolve::Live::Online(video_id)) => video_id,
            Ok(resolve::Live::Offline) => {
                let _ = bus.publish(status("offline", "El canal no está en directo".into()));
                backoff = Duration::ZERO;
                if wait(settings.offline_retry, &mut shutdown).await {
                    return;
                }
                continue;
            }
            Err(error) => {
                disconnected(&bus, &error);
                backoff = next_backoff(backoff);
                if wait(backoff, &mut shutdown).await {
                    return;
                }
                continue;
            }
        };
        let opened = tokio::select! {
            value = resolve::bootstrap(&settings, &http, &video_id) => value,
            _ = shutdown.changed() => return,
        };
        let mut bootstrap = match opened {
            Ok(bootstrap) => bootstrap,
            Err(error) => {
                disconnected(&bus, &error);
                backoff = next_backoff(backoff);
                if wait(backoff, &mut shutdown).await {
                    return;
                }
                continue;
            }
        };
        for event in bootstrap.history.drain(..) {
            dedupe.insert(event.id);
        }
        backoff = Duration::ZERO;
        let _ = bus.publish(status("connected", "Chat en vivo conectado".into()));
        match drive(
            &settings,
            &http,
            &bus,
            &mut dedupe,
            bootstrap,
            &mut shutdown,
        )
        .await
        {
            SessionEnd::Shutdown => return,
            SessionEnd::Offline => {
                let _ = bus.publish(status("offline", "El directo terminó".into()));
                if wait(settings.offline_retry, &mut shutdown).await {
                    return;
                }
            }
            SessionEnd::Dropped(error) => {
                disconnected(&bus, &error);
                backoff = next_backoff(backoff);
                if wait(backoff, &mut shutdown).await {
                    return;
                }
            }
        }
    }
}

async fn drive(
    settings: &YoutubeSettings,
    http: &reqwest::Client,
    bus: &Bus,
    dedupe: &mut RecentIds,
    mut bootstrap: resolve::Bootstrap,
    shutdown: &mut watch::Receiver<bool>,
) -> SessionEnd {
    loop {
        let response = tokio::select! {
            value = poll(settings, http, &bootstrap) => value,
            _ = shutdown.changed() => return SessionEnd::Shutdown,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => return SessionEnd::Dropped(error),
        };
        let Some(chat) = response.pointer("/continuationContents/liveChatContinuation") else {
            return SessionEnd::Offline;
        };
        let Some((continuation, timeout_ms)) = resolve::next_continuation(chat) else {
            return SessionEnd::Offline;
        };
        let actions = chat
            .get("actions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for action in actions {
            let event = match map::event(action) {
                Ok(event) => event,
                Err(error) => {
                    eprintln!("YouTube: evento inválido: {error}");
                    continue;
                }
            };
            if let Some(event) = event
                && enabled(settings, &event)
                && dedupe.insert(event.id.clone())
            {
                let _ = bus.publish(event);
            }
        }
        bootstrap.continuation = continuation;
        let delay = Duration::from_millis(timeout_ms).clamp(settings.poll_min, settings.poll_max);
        if wait(delay, shutdown).await {
            return SessionEnd::Shutdown;
        }
    }
}

async fn poll(
    settings: &YoutubeSettings,
    http: &reqwest::Client,
    bootstrap: &resolve::Bootstrap,
) -> Result<Value, String> {
    let url = format!(
        "{}/youtubei/v1/live_chat/get_live_chat?key={}&prettyPrint=false",
        settings.urls.innertube.trim_end_matches('/'),
        bootstrap.key
    );
    let response = http
        .post(url)
        .json(&json!({
            "context": {"client": {
                "clientName": "WEB",
                "clientVersion": bootstrap.client_version
            }},
            "continuation": bootstrap.continuation
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response
        .url()
        .host_str()
        .is_some_and(|host| host == "consent.youtube.com")
    {
        return Err("YouTube exige consentimiento o bloqueó Innertube; configura youtube.video_id si conoces el directo; streams-tools no evade controles de consentimiento ni anti-bot".into());
    }
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) {
        return Err("YouTube bloqueó Innertube; configura youtube.video_id si conoces el directo; streams-tools no evade controles de consentimiento ni anti-bot".into());
    }
    if !status.is_success() {
        return Err(format!("Innertube devolvió HTTP {status}"));
    }
    response
        .json()
        .await
        .map_err(|error| format!("respuesta Innertube inválida: {error}"))
}

fn enabled(settings: &YoutubeSettings, event: &Event) -> bool {
    let kind = match &event.event {
        EventData::Chat(_) => YoutubeEvent::Chat,
        EventData::Donation(_) => YoutubeEvent::Donation,
        EventData::Sub(_) => YoutubeEvent::Sub,
        EventData::GiftSub(_) => YoutubeEvent::GiftSub,
        _ => return false,
    };
    settings.events.contains(&kind)
}

async fn wait(duration: Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = shutdown.changed() => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

fn disconnected(bus: &Bus, error: &str) {
    eprintln!("YouTube: sesión terminada: {error}");
    let _ = bus.publish(status("disconnected", error.into()));
}

fn status(kind: &str, message: String) -> Event {
    system(Platform::Youtube, "youtube", "YouTube", kind, message)
}
