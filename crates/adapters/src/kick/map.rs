use crate::common::now_utc;
use serde_json::{Map, Value};
use std::{
    error::Error,
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};
use streams_tools_core::{
    ChatData, Event, EventData, FollowData, GiftSubData, Platform, RaidData, SubData, User,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapError(String);
impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for MapError {}

pub fn event(frame: &Value, counter: &AtomicU64) -> Result<Option<Event>, MapError> {
    let kind = string(frame.get("event"), "event")?;
    if !matches!(
        kind,
        "App\\Events\\ChatMessageEvent"
            | "App\\Events\\SubscriptionEvent"
            | "App\\Events\\ChannelSubscriptionEvent"
            | "App\\Events\\GiftedSubscriptionsEvent"
            | "App\\Events\\StreamHostEvent"
            | "App\\Events\\FollowersUpdated"
    ) {
        return Ok(None);
    }
    let encoded = string(frame.get("data"), "data")?;
    let decoded: Value = serde_json::from_str(encoded).map_err(|_| invalid("data JSON"))?;
    let data = object(Some(&decoded), "data JSON")?;
    if kind == "App\\Events\\FollowersUpdated" && !boolean(data.get("followed"), "data.followed")? {
        return Ok(None);
    }
    let (user, payload) = match kind {
        "App\\Events\\ChatMessageEvent" => {
            let sender = object(data.get("sender"), "data.sender")?;
            let identity = object(sender.get("identity"), "data.sender.identity")?;
            (
                User {
                    id: identifier(sender.get("id"), "data.sender.id")?,
                    name: string(sender.get("username"), "data.sender.username")?.into(),
                    roles: roles(identity)?,
                },
                EventData::Chat(ChatData {
                    message: string(data.get("content"), "data.content")?.into(),
                }),
            )
        }
        "App\\Events\\SubscriptionEvent" | "App\\Events\\ChannelSubscriptionEvent" => {
            let name = string(data.get("username"), "data.username")?;
            (
                named_user(data, name),
                EventData::Sub(SubData {
                    tier: "1".into(),
                    months: data.get("months").and_then(Value::as_u64).unwrap_or(1),
                }),
            )
        }
        "App\\Events\\GiftedSubscriptionsEvent" => {
            let name = string(data.get("gifter_username"), "data.gifter_username")?;
            let recipients = data
                .get("gifted_usernames")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("data.gifted_usernames"))?;
            (
                named_user(data, name),
                EventData::GiftSub(GiftSubData {
                    tier: "1".into(),
                    count: recipients.len() as u64,
                }),
            )
        }
        "App\\Events\\StreamHostEvent" => {
            let name = string(data.get("host_username"), "data.host_username")?;
            (
                named_user(data, name),
                EventData::Raid(RaidData {
                    viewers: data
                        .get("number_viewers")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid("data.number_viewers"))?,
                }),
            )
        }
        "App\\Events\\FollowersUpdated" => {
            let name = string(data.get("username"), "data.username")?;
            (named_user(data, name), EventData::Follow(FollowData {}))
        }
        _ => unreachable!(),
    };
    let id = data
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("kick-{}", counter.fetch_add(1, Ordering::Relaxed)));
    let ts = data
        .get("created_at")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(now_utc);
    Ok(Some(Event {
        id,
        platform: Platform::Kick,
        event: payload,
        user,
        ts,
    }))
}

fn invalid(path: &str) -> MapError {
    MapError(format!("missing or invalid {path}"))
}
fn object<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a Map<String, Value>, MapError> {
    value
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(path))
}
fn string<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a str, MapError> {
    value.and_then(Value::as_str).ok_or_else(|| invalid(path))
}
fn boolean(value: Option<&Value>, path: &str) -> Result<bool, MapError> {
    value.and_then(Value::as_bool).ok_or_else(|| invalid(path))
}
fn identifier(value: Option<&Value>, path: &str) -> Result<String, MapError> {
    value
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|id| id.to_string()))
        })
        .ok_or_else(|| invalid(path))
}
fn named_user(data: &Map<String, Value>, name: &str) -> User {
    User {
        id: data
            .get("user_id")
            .and_then(|v| identifier(Some(v), "data.user_id").ok())
            .unwrap_or_else(|| name.into()),
        name: name.into(),
        roles: Vec::new(),
    }
}
fn roles(identity: &Map<String, Value>) -> Result<Vec<String>, MapError> {
    let badges = identity
        .get("badges")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("data.sender.identity.badges"))?;
    Ok(["broadcaster", "moderator", "vip", "subscriber"]
        .into_iter()
        .filter(|role| {
            badges
                .iter()
                .any(|badge| badge.get("type").and_then(Value::as_str) == Some(*role))
        })
        .map(str::to_owned)
        .collect())
}
