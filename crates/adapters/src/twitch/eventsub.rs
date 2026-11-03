use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::{Value, json};
use streams_tools_core::{Event, EventData, Platform, PluginData, User};
use tokio::{sync::watch, task::JoinHandle};

use super::{TwitchEvent, map};

pub struct TwitchAdapter {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
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
                "Twitch: revocación tipo={} estado={} motivo={} id={}",
                field("type"),
                field("status"),
                field("reason"),
                field("id")
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
