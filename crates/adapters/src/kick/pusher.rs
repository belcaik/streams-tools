use super::{
    KickEvent, KickSettings, map,
    resolve::{self, Ids},
};
use crate::common::{RecentIds, next_backoff, system};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{error::Error, fmt, sync::atomic::AtomicU64, time::Duration};
use streams_tools_core::{Bus, Event, EventData, Platform};
use tokio::{sync::watch, task::JoinHandle, time::Instant};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

pub struct KickAdapter {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}
#[derive(Debug)]
pub struct AdapterError(String);
impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for AdapterError {}

impl KickAdapter {
    pub fn start(bus: Bus, settings: KickSettings) -> Result<Self, AdapterError> {
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
impl Drop for KickAdapter {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
struct Session {
    socket: Socket,
    activity: Duration,
}
enum SessionEnd {
    Dropped(String),
    Shutdown,
}

async fn run(
    bus: Bus,
    settings: KickSettings,
    http: reqwest::Client,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ids = None;
    let mut dedupe = RecentIds::new();
    let counter = AtomicU64::new(0);
    let mut backoff = Duration::ZERO;
    loop {
        if *shutdown.borrow() {
            return;
        }
        let resolved = match ids {
            Some(ids) => Ok(ids),
            None => {
                tokio::select! { value = resolve::resolve(&settings, &http) => value, _ = shutdown.changed() => return }
            }
        };
        let opened = match resolved {
            Ok(value) => {
                ids = Some(value);
                tokio::select! {
                    value = open_session(&settings, value) => value,
                    _ = shutdown.changed() => return,
                }
            }
            Err(error) => Err(error),
        };
        match opened {
            Ok(session) => {
                backoff = Duration::ZERO;
                let _ = bus.publish(status("connected", "Pusher conectado".into()));
                match drive(
                    session,
                    &settings,
                    &bus,
                    &mut dedupe,
                    &counter,
                    &mut shutdown,
                )
                .await
                {
                    SessionEnd::Shutdown => return,
                    SessionEnd::Dropped(reason) => {
                        eprintln!("Kick: sesión terminada: {reason}");
                        let _ = bus.publish(status("disconnected", reason));
                    }
                }
            }
            Err(error) => {
                eprintln!("Kick: no se pudo conectar: {error}");
                let _ = bus.publish(status("disconnected", error));
            }
        }
        backoff = next_backoff(backoff);
        tokio::select! { _ = shutdown.changed() => return, _ = tokio::time::sleep(backoff) => {} }
    }
}

async fn open_session(settings: &KickSettings, ids: Ids) -> Result<Session, String> {
    let (mut socket, _) = tokio_tungstenite::connect_async(settings.websocket_url())
        .await
        .map_err(|error| format!("websocket: {error}"))?;
    let first = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .map_err(|_| "timeout esperando pusher:connection_established".to_string())?
        .ok_or_else(|| "websocket cerrado antes de pusher:connection_established".to_string())?
        .map_err(|error| error.to_string())?;
    let Message::Text(text) = first else {
        return Err("el primer frame Pusher no es texto".into());
    };
    let frame = parse(text.as_str())?;
    if event_name(&frame)? != "pusher:connection_established" {
        return Err("el primer mensaje no es pusher:connection_established".into());
    }
    let established: Value = serde_json::from_str(data_string(&frame)?)
        .map_err(|error| format!("connection_established inválido: {error}"))?;
    let _socket_id = established
        .get("socket_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "connection_established sin socket_id".to_string())?;
    let timeout = established
        .get("activity_timeout")
        .and_then(Value::as_u64)
        .ok_or_else(|| "connection_established sin activity_timeout".to_string())?;
    let mut pending = Vec::new();
    if settings.events.contains(&KickEvent::Chat) {
        pending.push(format!("chatrooms.{}.v2", ids.chatroom));
    }
    if settings
        .events
        .iter()
        .any(|event| *event != KickEvent::Chat)
    {
        pending.push(format!("channel.{}", ids.channel));
    }
    for channel in &pending {
        send_json(
            &mut socket,
            json!({"event":"pusher:subscribe","data":{"channel":channel}}),
        )
        .await?;
    }
    while !pending.is_empty() {
        let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .map_err(|_| "timeout esperando subscription_succeeded".to_string())?
            .ok_or_else(|| "websocket cerrado esperando subscription_succeeded".to_string())?
            .map_err(|error| error.to_string())?;
        let Message::Text(text) = frame else {
            continue;
        };
        let value = parse(text.as_str())?;
        match event_name(&value)? {
            "pusher_internal:subscription_succeeded" => {
                let channel = value
                    .get("channel")
                    .and_then(Value::as_str)
                    .ok_or("subscription_succeeded sin channel")?;
                pending.retain(|item| item != channel);
            }
            "pusher:ping" => {
                send_json(&mut socket, json!({"event":"pusher:pong","data":{}})).await?
            }
            "pusher:error" => return Err(pusher_error(&value)),
            _ => {}
        }
    }
    Ok(Session {
        socket,
        activity: Duration::from_secs(timeout),
    })
}

async fn drive(
    mut session: Session,
    settings: &KickSettings,
    bus: &Bus,
    dedupe: &mut RecentIds,
    counter: &AtomicU64,
    shutdown: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let mut activity_deadline = Instant::now() + session.activity;
    let mut pong_deadline = None;
    loop {
        let deadline = pong_deadline.unwrap_or(activity_deadline);
        tokio::select! {
            _ = shutdown.changed() => return SessionEnd::Shutdown,
            _ = tokio::time::sleep_until(deadline) => {
                if pong_deadline.is_some() { return SessionEnd::Dropped("timeout esperando pusher:pong".into()); }
                if let Err(error) = send_json(&mut session.socket, json!({"event":"pusher:ping","data":{}})).await {
                    return SessionEnd::Dropped(error);
                }
                pong_deadline = Some(Instant::now() + Duration::from_secs(30));
            }
            frame = session.socket.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(error)) => return SessionEnd::Dropped(error.to_string()),
                    None => return SessionEnd::Dropped("websocket cerrado".into()),
                };
                activity_deadline = Instant::now() + session.activity;
                match frame {
                    Message::Text(text) => {
                        let value = match parse(text.as_str()) { Ok(value) => value, Err(error) => { eprintln!("Kick: mensaje inválido: {error}"); continue; } };
                        let kind = match event_name(&value) { Ok(kind) => kind, Err(error) => { eprintln!("Kick: mensaje inválido: {error}"); continue; } };
                        match kind {
                            "pusher:ping" => if let Err(error) = send_json(&mut session.socket, json!({"event":"pusher:pong","data":{}})).await { return SessionEnd::Dropped(error); },
                            "pusher:pong" => pong_deadline = None,
                            "pusher:error" => return SessionEnd::Dropped(pusher_error(&value)),
                            _ => match map::event(&value, counter) {
                                Ok(Some(event)) if enabled(settings, &event) => {
                                    if !matches!(&event.event, EventData::Chat(_)) || dedupe.insert(event.id.clone()) { let _ = bus.publish(event); }
                                }
                                Ok(_) => {}
                                Err(error) => eprintln!("Kick: evento inválido: {error}"),
                            }
                        }
                    }
                    Message::Ping(data) => if let Err(error) = session.socket.send(Message::Pong(data)).await { return SessionEnd::Dropped(error.to_string()); },
                    Message::Close(_) => return SessionEnd::Dropped("websocket cerrado por Kick".into()),
                    Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

fn enabled(settings: &KickSettings, event: &Event) -> bool {
    let kind = match &event.event {
        EventData::Chat(_) => KickEvent::Chat,
        EventData::Sub(_) => KickEvent::Sub,
        EventData::GiftSub(_) => KickEvent::GiftSub,
        EventData::Raid(_) => KickEvent::Raid,
        EventData::Follow(_) => KickEvent::Follow,
        _ => return false,
    };
    settings.events.contains(&kind)
}
fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|error| error.to_string())
}
fn event_name(value: &Value) -> Result<&str, String> {
    value
        .get("event")
        .and_then(Value::as_str)
        .ok_or_else(|| "frame sin event".into())
}
fn data_string(value: &Value) -> Result<&str, String> {
    value
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "frame sin data string".into())
}
fn pusher_error(value: &Value) -> String {
    format!(
        "pusher:error: {}",
        value.get("data").unwrap_or(&Value::Null)
    )
}
async fn send_json(socket: &mut Socket, value: Value) -> Result<(), String> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|error| error.to_string())
}
fn status(kind: &str, message: String) -> Event {
    system(Platform::Kick, "kick", "Kick", kind, message)
}
