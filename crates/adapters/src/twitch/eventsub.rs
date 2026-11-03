use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use streams_tools_core::{Bus, Event, EventData, Platform, PluginData, User};
use tokio::{sync::watch, task::JoinHandle, time::Instant};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

use super::{
    TwitchEvent, TwitchSettings,
    auth::{Auth, Prompt},
    helix::Helix,
    map,
};

pub struct TwitchAdapter {
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

impl TwitchAdapter {
    pub fn start(bus: Bus, settings: TwitchSettings) -> Result<Self, AdapterError> {
        let http = reqwest::Client::builder()
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

impl Drop for TwitchAdapter {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct Session {
    socket: Socket,
    keepalive: Duration,
}

enum SessionEnd {
    Reconnect(String),
    Dropped(String),
    Shutdown,
}

async fn run(
    bus: Bus,
    settings: TwitchSettings,
    http: reqwest::Client,
    mut shutdown: watch::Receiver<bool>,
) {
    let prompt_bus = bus.clone();
    let prompt: Prompt = Arc::new(move |authorization| {
        let _ = prompt_bus.publish(system(
            "auth_required",
            "Autoriza streams-tools en Twitch para conectar el canal".into(),
        ));
        println!(
            "Twitch: abre {} e introduce el código {}",
            authorization.verification_uri, authorization.user_code
        );
    });
    let auth = Auth::new(
        settings.client_id.clone(),
        required_scopes(&settings.events),
        settings.data_dir.join("token.json"),
        settings.urls.identity.clone(),
        http.clone(),
        prompt,
    );
    let helix = Helix::new(
        settings.client_id.clone(),
        settings.urls.helix.clone(),
        http,
        auth.clone(),
    );
    let mut dedupe = Dedupe::new();
    let mut backoff = Duration::ZERO;
    let mut reconnect_url = None;
    loop {
        if *shutdown.borrow() {
            return;
        }
        let requested = reconnect_url.take();
        let migrating = requested.is_some();
        let url = requested
            .as_deref()
            .unwrap_or(settings.urls.websocket.as_str());
        let opened = tokio::select! {
            result = open_session(url, &settings, &auth, &helix, migrating) => result,
            _ = shutdown.changed() => return,
        };
        match opened {
            Ok(session) => {
                backoff = Duration::ZERO;
                let _ = bus.publish(system("connected", "EventSub conectado".into()));
                match drive(session, &bus, &mut dedupe, &mut shutdown).await {
                    SessionEnd::Reconnect(url) => {
                        reconnect_url = Some(url);
                        continue;
                    }
                    SessionEnd::Dropped(reason) => {
                        eprintln!("Twitch: sesión terminada: {reason}");
                        let _ = bus.publish(system("disconnected", reason));
                    }
                    SessionEnd::Shutdown => return,
                }
            }
            Err(error) => {
                eprintln!("Twitch: no se pudo conectar: {error}");
                let _ = bus.publish(system("disconnected", error));
            }
        }
        backoff = next_backoff(backoff);
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
    }
}

async fn open_session(
    url: &str,
    settings: &TwitchSettings,
    auth: &Auth,
    helix: &Helix,
    migrating: bool,
) -> Result<Session, String> {
    let broadcaster = if migrating {
        None
    } else {
        auth.requires_login()
            .await
            .map_err(|error| error.to_string())?;
        auth.access_token(false)
            .await
            .map_err(|error| error.to_string())?;
        Some(
            helix
                .resolve_broadcaster(&settings.channel)
                .await
                .map_err(|error| error.to_string())?,
        )
    };
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|error| format!("websocket: {error}"))?;
    let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .map_err(|_| "timeout esperando session_welcome".to_string())?
        .ok_or_else(|| "websocket cerrado antes de session_welcome".to_string())?
        .map_err(|error| error.to_string())?;
    let Message::Text(text) = frame else {
        return Err("el primer frame no es session_welcome de texto".into());
    };
    let incoming = parse_message(text.as_str())?;
    if incoming.metadata.message_type != "session_welcome" {
        return Err("el primer mensaje no es session_welcome".into());
    }
    let session = incoming
        .payload
        .get("session")
        .and_then(Value::as_object)
        .ok_or_else(|| "welcome sin payload.session".to_string())?;
    let session_id = session
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "welcome sin session.id".to_string())?;
    let timeout = session
        .get("keepalive_timeout_seconds")
        .and_then(Value::as_u64)
        .ok_or_else(|| "welcome sin keepalive_timeout_seconds".to_string())?;
    if let Some(broadcaster) = broadcaster {
        helix
            .create_subscriptions(&broadcaster.id, session_id, &settings.events)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(Session {
        socket,
        keepalive: Duration::from_secs(timeout).saturating_add(settings.keepalive_margin),
    })
}

async fn drive(
    mut session: Session,
    bus: &Bus,
    dedupe: &mut Dedupe,
    shutdown: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let mut deadline = Instant::now() + session.keepalive;
    loop {
        tokio::select! {
            _ = shutdown.changed() => return SessionEnd::Shutdown,
            _ = tokio::time::sleep_until(deadline) => {
                return SessionEnd::Dropped("timeout de keepalive".into());
            }
            frame = session.socket.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(error)) => return SessionEnd::Dropped(error.to_string()),
                    None => return SessionEnd::Dropped("websocket cerrado".into()),
                };
                match frame {
                    Message::Text(text) => {
                        let incoming = match parse_message(text.as_str()) {
                            Ok(incoming) => incoming,
                            Err(error) => {
                                eprintln!("Twitch: mensaje inválido: {error}");
                                continue;
                            }
                        };
                        let notification = incoming.metadata.message_type == "notification";
                        let action = match process_incoming(incoming, dedupe) {
                            Ok(action) => action,
                            Err(error) => return SessionEnd::Dropped(error),
                        };
                        if notification {
                            deadline = Instant::now() + session.keepalive;
                        }
                        match action {
                            FrameAction::None => {}
                            FrameAction::Keepalive => {
                                deadline = Instant::now() + session.keepalive;
                            }
                            FrameAction::Event(event) => {
                                let _ = bus.publish(event);
                            }
                            FrameAction::Reconnect(url) => return SessionEnd::Reconnect(url),
                        }
                    }
                    Message::Ping(payload) => {
                        if let Err(error) = session.socket.send(Message::Pong(payload)).await {
                            return SessionEnd::Dropped(error.to_string());
                        }
                    }
                    Message::Close(_) => {
                        return SessionEnd::Dropped("websocket cerrado por Twitch".into());
                    }
                    Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct Wire {
    metadata: Metadata,
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    message_id: String,
    message_type: String,
    message_timestamp: String,
    subscription_type: Option<String>,
}

#[derive(Debug)]
struct Incoming {
    metadata: Metadata,
    payload: Value,
    raw: Value,
}

#[derive(Debug, PartialEq)]
enum FrameAction {
    None,
    Keepalive,
    Event(Event),
    Reconnect(String),
}

struct Dedupe {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl Dedupe {
    fn new() -> Self {
        Self {
            ids: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, id: String) -> bool {
        if !self.ids.insert(id.clone()) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > 512
            && let Some(oldest) = self.order.pop_front()
        {
            self.ids.remove(&oldest);
        }
        true
    }
}

pub(crate) fn required_scopes(events: &BTreeSet<TwitchEvent>) -> Vec<String> {
    let mut scopes = BTreeSet::new();
    for event in events {
        match event {
            TwitchEvent::Follow => {
                scopes.insert("moderator:read:followers");
            }
            TwitchEvent::Sub | TwitchEvent::GiftSub => {
                scopes.insert("channel:read:subscriptions");
            }
            TwitchEvent::Raid => {}
            TwitchEvent::Redemption => {
                scopes.insert("channel:read:redemptions");
            }
            TwitchEvent::Chat => {
                scopes.insert("user:read:chat");
            }
            TwitchEvent::Cheer => {
                scopes.insert("bits:read");
            }
        }
    }
    scopes.into_iter().map(str::to_owned).collect()
}

fn next_backoff(current: Duration) -> Duration {
    Duration::from_secs(match current.as_secs() {
        0 => 1,
        1 => 2,
        2 => 4,
        4 => 8,
        8 => 16,
        16 => 32,
        _ => 60,
    })
}

fn parse_message(text: &str) -> Result<Incoming, String> {
    let raw: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let wire: Wire = serde_json::from_value(raw.clone()).map_err(|error| error.to_string())?;
    Ok(Incoming {
        metadata: wire.metadata,
        payload: wire.payload,
        raw,
    })
}

fn process_incoming(incoming: Incoming, dedupe: &mut Dedupe) -> Result<FrameAction, String> {
    match incoming.metadata.message_type.as_str() {
        "notification" => {
            if !dedupe.insert(incoming.metadata.message_id) {
                return Ok(FrameAction::None);
            }
            match map::notification(&incoming.raw) {
                Ok(Some(event)) => Ok(FrameAction::Event(event)),
                Ok(None) => Ok(FrameAction::None),
                Err(error) => {
                    eprintln!("Twitch: notificación inválida: {error}");
                    Ok(FrameAction::None)
                }
            }
        }
        "session_keepalive" => Ok(FrameAction::Keepalive),
        "session_reconnect" => incoming
            .payload
            .get("session")
            .and_then(|session| session.get("reconnect_url"))
            .and_then(Value::as_str)
            .map(|url| FrameAction::Reconnect(url.into()))
            .ok_or_else(|| "session_reconnect sin payload.session.reconnect_url".into()),
        "revocation" => {
            let subscription = incoming.payload.get("subscription");
            let field = |name| {
                subscription
                    .and_then(|value| value.get(name))
                    .and_then(Value::as_str)
                    .unwrap_or("desconocido")
            };
            eprintln!(
                "Twitch: revocación tipo={} estado={} motivo={} id={} suscripción={} ts={}",
                field("type"),
                field("status"),
                field("reason"),
                field("id"),
                incoming
                    .metadata
                    .subscription_type
                    .as_deref()
                    .unwrap_or("desconocido"),
                incoming.metadata.message_timestamp
            );
            Ok(FrameAction::None)
        }
        _ => Ok(FrameAction::None),
    }
}

fn system(status: &str, message: String) -> Event {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    Event {
        id: format!("twitch-system-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)),
        platform: Platform::Twitch,
        event: EventData::Plugin(PluginData {
            name: "system".into(),
            data: json!({"status": status, "message": message}),
        }),
        user: User {
            id: "twitch".into(),
            name: "Twitch".into(),
            roles: Vec::new(),
        },
        ts: format_utc(elapsed),
    }
}

fn format_utc(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64;
    let within_day = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        month,
        within_day / 3_600,
        (within_day / 60) % 60,
        within_day % 60,
        elapsed.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_ordered_unique_and_limited_to_enabled_events() {
        assert_eq!(
            required_scopes(&TwitchEvent::ALL.into_iter().collect()),
            [
                "bits:read",
                "channel:read:redemptions",
                "channel:read:subscriptions",
                "moderator:read:followers",
                "user:read:chat",
            ]
        );
        assert_eq!(required_scopes(&BTreeSet::new()), Vec::<String>::new());
        assert_eq!(
            required_scopes(&[TwitchEvent::Raid].into_iter().collect()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn backoff_saturates_at_sixty_seconds() {
        let mut backoff = Duration::ZERO;
        let observed: Vec<_> = (0..8)
            .map(|_| {
                backoff = next_backoff(backoff);
                backoff.as_secs()
            })
            .collect();
        assert_eq!(observed, [1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn dedupe_retains_the_latest_512_ids() {
        let mut dedupe = Dedupe::new();
        assert!(dedupe.insert("same".into()));
        assert!(!dedupe.insert("same".into()));
        for index in 0..512 {
            assert!(dedupe.insert(format!("id-{index}")));
        }
        assert!(!dedupe.ids.contains("same"));
        assert_eq!(dedupe.ids.len(), 512);
        assert!(dedupe.insert("same".into()));
    }

    #[test]
    fn parser_accepts_extra_fields_and_reads_wire_metadata() {
        let incoming = parse_message(
            r#"{"metadata":{"message_id":"id","message_type":"session_keepalive","message_timestamp":"now","subscription_type":null,"new_field":true},"payload":{},"extra":1}"#,
        )
        .unwrap();
        assert_eq!(incoming.metadata.message_id, "id");
        assert_eq!(
            process_incoming(incoming, &mut Dedupe::new()).unwrap(),
            FrameAction::Keepalive
        );
    }

    #[test]
    fn reconnect_requires_url() {
        let incoming = parse_message(
            r#"{"metadata":{"message_id":"id","message_type":"session_reconnect","message_timestamp":"now"},"payload":{"session":{"reconnect_url":"ws://localhost"}}}"#,
        )
        .unwrap();
        assert_eq!(
            process_incoming(incoming, &mut Dedupe::new()).unwrap(),
            FrameAction::Reconnect("ws://localhost".into())
        );
    }

    #[test]
    fn utc_conversion_matches_the_existing_plugin_algorithm() {
        assert_eq!(format_utc(Duration::ZERO), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_utc(Duration::from_millis(1_582_934_423_456)),
            "2020-02-29T00:00:23.456Z"
        );
    }
}
