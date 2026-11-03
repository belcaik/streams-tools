use crate::common::format_utc;
use serde_json::{Map, Value};
use std::{error::Error, fmt, time::Duration};
use streams_tools_core::{
    ChatData, DonationData, Event, EventData, GiftSubData, Platform, SubData, User,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapError(String);

impl fmt::Display for MapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MapError {}

pub fn event(action: &Value) -> Result<Option<Event>, MapError> {
    let Some(item) = action.pointer("/addChatItemAction/item") else {
        return Ok(None);
    };
    let Some((kind, renderer)) = renderer(item) else {
        return Ok(None);
    };
    let renderer = object(Some(renderer), kind)?;
    let source = renderer
        .get("header")
        .and_then(|header| header.get("liveChatSponsorshipsHeaderRenderer"))
        .and_then(Value::as_object)
        .unwrap_or(renderer);
    let user = user(source)?;
    let data = match kind {
        "liveChatTextMessageRenderer" => EventData::Chat(ChatData {
            message: text(renderer.get("message"), "message")?,
        }),
        "liveChatPaidMessageRenderer" | "liveChatPaidStickerRenderer" => {
            let purchase = simple(renderer.get("purchaseAmountText"), "purchaseAmountText")?;
            let (amount, currency) = amount(purchase)?;
            EventData::Donation(DonationData {
                amount,
                currency,
                message: optional_text(renderer.get("message")).unwrap_or_default(),
            })
        }
        "liveChatMembershipItemRenderer" => EventData::Sub(SubData {
            tier: optional_text(renderer.get("headerSubtext"))
                .or_else(|| optional_text(renderer.get("membershipLevelName")))
                .unwrap_or_else(|| "member".into()),
            months: optional_text(renderer.get("headerPrimaryText"))
                .and_then(|value| first_integer(&value))
                .unwrap_or(1),
        }),
        "liveChatSponsorshipsGiftPurchaseAnnouncementRenderer" => {
            let count = optional_text(source.get("primaryText"))
                .and_then(|value| first_integer(&value))
                .ok_or_else(|| invalid("primaryText gift count"))?;
            EventData::GiftSub(GiftSubData {
                tier: "member".into(),
                count,
            })
        }
        _ => return Ok(None),
    };
    let id = string(renderer.get("id"), "renderer.id")?.into();
    let micros = string(renderer.get("timestampUsec"), "renderer.timestampUsec")?
        .parse::<u64>()
        .map_err(|_| invalid("renderer.timestampUsec"))?;
    Ok(Some(Event {
        id,
        platform: Platform::Youtube,
        event: data,
        user,
        ts: format_utc(Duration::from_micros(micros)),
    }))
}

fn renderer(item: &Value) -> Option<(&'static str, &Value)> {
    [
        "liveChatTextMessageRenderer",
        "liveChatPaidMessageRenderer",
        "liveChatPaidStickerRenderer",
        "liveChatMembershipItemRenderer",
        "liveChatSponsorshipsGiftPurchaseAnnouncementRenderer",
    ]
    .into_iter()
    .find_map(|kind| item.get(kind).map(|value| (kind, value)))
}

fn user(renderer: &Map<String, Value>) -> Result<User, MapError> {
    Ok(User {
        id: string(
            renderer.get("authorExternalChannelId"),
            "authorExternalChannelId",
        )?
        .into(),
        name: simple(renderer.get("authorName"), "authorName")?.into(),
        roles: roles(renderer.get("authorBadges")),
    })
}

fn roles(value: Option<&Value>) -> Vec<String> {
    let badges = value.and_then(Value::as_array);
    ["broadcaster", "moderator", "vip", "subscriber"]
        .into_iter()
        .filter(|role| {
            badges.is_some_and(|badges| {
                badges.iter().any(|badge| {
                    let renderer = badge.get("liveChatAuthorBadgeRenderer");
                    match *role {
                        "broadcaster" => icon(renderer) == Some("OWNER"),
                        "moderator" => icon(renderer) == Some("MODERATOR"),
                        "subscriber" => {
                            renderer.is_some_and(|value| value.get("customThumbnail").is_some())
                        }
                        "vip" => false,
                        _ => unreachable!(),
                    }
                })
            })
        })
        .map(str::to_owned)
        .collect()
}

fn icon(renderer: Option<&Value>) -> Option<&str> {
    renderer?.get("icon")?.get("iconType")?.as_str()
}

fn optional_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(text.into());
    }
    let runs = value.get("runs")?.as_array()?;
    Some(
        runs.iter()
            .filter_map(|run| {
                run.get("text").and_then(Value::as_str).or_else(|| {
                    let emoji = run.get("emoji")?;
                    emoji
                        .get("shortcuts")
                        .and_then(Value::as_array)
                        .and_then(|values| values.first())
                        .and_then(Value::as_str)
                        .or_else(|| emoji.get("emojiId").and_then(Value::as_str))
                })
            })
            .collect(),
    )
}

fn text(value: Option<&Value>, path: &str) -> Result<String, MapError> {
    optional_text(value).ok_or_else(|| invalid(path))
}

fn simple<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a str, MapError> {
    value
        .and_then(|value| value.get("simpleText"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(path))
}

fn amount(text: &str) -> Result<(f64, String), MapError> {
    let start = text
        .char_indices()
        .find_map(|(index, value)| value.is_ascii_digit().then_some(index))
        .ok_or_else(|| invalid("purchaseAmountText amount"))?;
    let prefix = text[..start].trim();
    let number = text[start..].trim();
    let dot = number.rfind('.');
    let comma = number.rfind(',');
    let decimal = match (dot, comma) {
        (Some(dot), Some(comma)) => Some(dot.max(comma)),
        (Some(index), None) | (None, Some(index)) => {
            let digits = number[index + 1..]
                .chars()
                .filter(char::is_ascii_digit)
                .count();
            (digits == 1 || digits == 2).then_some(index)
        }
        (None, None) => None,
    };
    let normalized: String = number
        .char_indices()
        .filter_map(|(index, value)| {
            value
                .is_ascii_digit()
                .then_some(value)
                .or_else(|| (Some(index) == decimal).then_some('.'))
        })
        .collect();
    let value = normalized
        .parse()
        .map_err(|_| invalid("purchaseAmountText amount"))?;
    let currency = match prefix {
        "$" | "US$" => "USD",
        "€" => "EUR",
        "£" => "GBP",
        "¥" => "JPY",
        "₩" => "KRW",
        "₹" => "INR",
        "A$" => "AUD",
        "CA$" => "CAD",
        "MX$" => "MXN",
        "NZ$" => "NZD",
        "R$" => "BRL",
        other => other,
    };
    Ok((value, currency.into()))
}

fn first_integer(text: &str) -> Option<u64> {
    let digits: String = text
        .chars()
        .skip_while(|value| !value.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn object<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a Map<String, Value>, MapError> {
    value
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(path))
}

fn string<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a str, MapError> {
    value.and_then(Value::as_str).ok_or_else(|| invalid(path))
}

fn invalid(path: &str) -> MapError {
    MapError(format!("missing or invalid {path}"))
}
