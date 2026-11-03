use std::{error::Error, fmt};

use serde_json::{Map, Value};
use streams_tools_core::{
    ChatData, DonationData, Event, EventData, FollowData, GiftSubData, Platform, RaidData,
    RedemptionData, SubData, User,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapError(String);

impl fmt::Display for MapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MapError {}

pub fn notification(envelope: &Value) -> Result<Option<Event>, MapError> {
    let metadata = object(envelope.get("metadata"), "metadata")?;
    let kind = string(
        metadata.get("subscription_type"),
        "metadata.subscription_type",
    )?;
    if !matches!(
        kind,
        "channel.follow"
            | "channel.subscribe"
            | "channel.subscription.message"
            | "channel.subscription.gift"
            | "channel.raid"
            | "channel.channel_points_custom_reward_redemption.add"
            | "channel.chat.message"
            | "channel.cheer"
    ) {
        return Ok(None);
    }
    let id = string(metadata.get("message_id"), "metadata.message_id")?.into();
    let ts = string(
        metadata.get("message_timestamp"),
        "metadata.message_timestamp",
    )?
    .into();
    let payload = object(envelope.get("payload"), "payload")?;
    let event = object(payload.get("event"), "payload.event")?;
    let (user, data) = match kind {
        "channel.follow" => (
            user(event, "user_id", "user_name", Vec::new())?,
            EventData::Follow(FollowData {}),
        ),
        "channel.subscribe" => {
            if boolean(event.get("is_gift"), "payload.event.is_gift")? {
                return Ok(None);
            }
            (
                user(event, "user_id", "user_name", Vec::new())?,
                EventData::Sub(SubData {
                    tier: normalize_tier(string(event.get("tier"), "payload.event.tier")?),
                    months: 1,
                }),
            )
        }
        "channel.subscription.message" => (
            user(event, "user_id", "user_name", Vec::new())?,
            EventData::Sub(SubData {
                tier: normalize_tier(string(event.get("tier"), "payload.event.tier")?),
                months: u64(
                    event.get("cumulative_months"),
                    "payload.event.cumulative_months",
                )?,
            }),
        ),
        "channel.subscription.gift" => {
            let user = if boolean(event.get("is_anonymous"), "payload.event.is_anonymous")? {
                anonymous()
            } else {
                user(event, "user_id", "user_name", Vec::new())?
            };
            (
                user,
                EventData::GiftSub(GiftSubData {
                    tier: normalize_tier(string(event.get("tier"), "payload.event.tier")?),
                    count: u64(event.get("total"), "payload.event.total")?,
                }),
            )
        }
        "channel.raid" => (
            user(
                event,
                "from_broadcaster_user_id",
                "from_broadcaster_user_name",
                Vec::new(),
            )?,
            EventData::Raid(RaidData {
                viewers: u64(event.get("viewers"), "payload.event.viewers")?,
            }),
        ),
        "channel.channel_points_custom_reward_redemption.add" => {
            let reward = object(event.get("reward"), "payload.event.reward")?;
            (
                user(event, "user_id", "user_name", Vec::new())?,
                EventData::Redemption(RedemptionData {
                    reward_id: string(reward.get("id"), "payload.event.reward.id")?.into(),
                    reward_title: string(reward.get("title"), "payload.event.reward.title")?.into(),
                    cost: u64(reward.get("cost"), "payload.event.reward.cost")?,
                    input: string(event.get("user_input"), "payload.event.user_input")?.into(),
                }),
            )
        }
        "channel.chat.message" => (
            user(event, "chatter_user_id", "chatter_user_name", roles(event)?)?,
            EventData::Chat(ChatData {
                message: string(
                    object(event.get("message"), "payload.event.message")?.get("text"),
                    "payload.event.message.text",
                )?
                .into(),
            }),
        ),
        "channel.cheer" => {
            let user = if boolean(event.get("is_anonymous"), "payload.event.is_anonymous")? {
                anonymous()
            } else {
                user(event, "user_id", "user_name", Vec::new())?
            };
            (
                user,
                EventData::Donation(DonationData {
                    amount: u64(event.get("bits"), "payload.event.bits")? as f64,
                    currency: "bits".into(),
                    message: string(event.get("message"), "payload.event.message")?.into(),
                }),
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(Event {
        id,
        platform: Platform::Twitch,
        event: data,
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

fn u64(value: Option<&Value>, path: &str) -> Result<u64, MapError> {
    value.and_then(Value::as_u64).ok_or_else(|| invalid(path))
}

fn boolean(value: Option<&Value>, path: &str) -> Result<bool, MapError> {
    value.and_then(Value::as_bool).ok_or_else(|| invalid(path))
}

fn user(
    event: &Map<String, Value>,
    id: &str,
    name: &str,
    roles: Vec<String>,
) -> Result<User, MapError> {
    Ok(User {
        id: string(event.get(id), &format!("payload.event.{id}"))?.into(),
        name: string(event.get(name), &format!("payload.event.{name}"))?.into(),
        roles,
    })
}

fn anonymous() -> User {
    User {
        id: "anonymous".into(),
        name: "Anonymous".into(),
        roles: Vec::new(),
    }
}

fn roles(event: &Map<String, Value>) -> Result<Vec<String>, MapError> {
    let badges = event
        .get("badges")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("payload.event.badges"))?;
    Ok(["broadcaster", "moderator", "vip", "subscriber"]
        .into_iter()
        .filter(|role| {
            badges
                .iter()
                .any(|badge| badge.get("set_id").and_then(Value::as_str) == Some(*role))
        })
        .map(str::to_owned)
        .collect())
}

fn normalize_tier(tier: &str) -> String {
    match tier {
        "1000" => "1",
        "2000" => "2",
        "3000" => "3",
        other => other,
    }
    .into()
}
