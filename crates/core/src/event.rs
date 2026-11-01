use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{DeserializeOwned, Error as _},
    ser::SerializeStruct,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Platform {
    Twitch,
    Kick,
    Youtube,
    Plugin(String),
}

impl Serialize for Platform {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Twitch => serializer.serialize_str("twitch"),
            Self::Kick => serializer.serialize_str("kick"),
            Self::Youtube => serializer.serialize_str("youtube"),
            Self::Plugin(name) => serializer.serialize_str(&format!("plugin:{name}")),
        }
    }
}

impl<'de> Deserialize<'de> for Platform {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "twitch" => Ok(Self::Twitch),
            "kick" => Ok(Self::Kick),
            "youtube" => Ok(Self::Youtube),
            _ => match value.strip_prefix("plugin:") {
                Some("") | None => Err(D::Error::custom(format!(
                    "invalid platform `{value}`; expected twitch, kick, youtube, or plugin:<name>"
                ))),
                Some(name) => Ok(Self::Plugin(name.to_owned())),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowData {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubData {
    pub tier: String,
    #[serde(deserialize_with = "deserialize_months")]
    pub months: u64,
}

fn deserialize_months<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    u64::deserialize(deserializer)
        .map_err(|error| D::Error::custom(format!("invalid field `months`: {error}")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GiftSubData {
    pub tier: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaidData {
    pub viewers: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DonationData {
    pub amount: f64,
    pub currency: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedemptionData {
    pub reward_id: String,
    pub reward_title: String,
    pub cost: u64,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatData {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginData {
    pub name: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventData {
    Follow(FollowData),
    Sub(SubData),
    GiftSub(GiftSubData),
    Raid(RaidData),
    Donation(DonationData),
    Redemption(RedemptionData),
    Chat(ChatData),
    Plugin(PluginData),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub platform: Platform,
    #[serde(flatten)]
    pub event: EventData,
    pub user: User,
    pub ts: String,
}

impl Serialize for EventData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("EventData", 2)?;
        match self {
            Self::Follow(data) => {
                state.serialize_field("type", "follow")?;
                state.serialize_field("data", data)?;
            }
            Self::Sub(data) => {
                state.serialize_field("type", "sub")?;
                state.serialize_field("data", data)?;
            }
            Self::GiftSub(data) => {
                state.serialize_field("type", "gift_sub")?;
                state.serialize_field("data", data)?;
            }
            Self::Raid(data) => {
                state.serialize_field("type", "raid")?;
                state.serialize_field("data", data)?;
            }
            Self::Donation(data) => {
                state.serialize_field("type", "donation")?;
                state.serialize_field("data", data)?;
            }
            Self::Redemption(data) => {
                state.serialize_field("type", "redemption")?;
                state.serialize_field("data", data)?;
            }
            Self::Chat(data) => {
                state.serialize_field("type", "chat")?;
                state.serialize_field("data", data)?;
            }
            Self::Plugin(plugin) => {
                state.serialize_field("type", &plugin.name)?;
                state.serialize_field("data", &plugin.data)?;
            }
        }
        state.end()
    }
}

#[derive(Deserialize)]
struct RawEventData {
    #[serde(rename = "type")]
    kind: String,
    data: Value,
}

fn deserialize_data<T, E>(kind: &str, data: Value) -> Result<T, E>
where
    T: DeserializeOwned,
    E: serde::de::Error,
{
    serde_json::from_value(data)
        .map_err(|error| E::custom(format!("invalid data for event type `{kind}`: {error}")))
}

impl<'de> Deserialize<'de> for EventData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let RawEventData { kind, data } = RawEventData::deserialize(deserializer)?;
        match kind.as_str() {
            "follow" => deserialize_data(&kind, data).map(Self::Follow),
            "sub" => deserialize_data(&kind, data).map(Self::Sub),
            "gift_sub" => deserialize_data(&kind, data).map(Self::GiftSub),
            "raid" => deserialize_data(&kind, data).map(Self::Raid),
            "donation" => deserialize_data(&kind, data).map(Self::Donation),
            "redemption" => deserialize_data(&kind, data).map(Self::Redemption),
            "chat" => deserialize_data(&kind, data).map(Self::Chat),
            "" => Err(D::Error::custom("event type cannot be empty")),
            _ => Ok(Self::Plugin(PluginData { name: kind, data })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLLOW: &str = r#"{"id":"follow-1","platform":"twitch","type":"follow","user":{"id":"1","name":"ana","roles":["mod"]},"data":{},"ts":"2026-10-31T20:00:00Z"}"#;
    const SUB: &str = r#"{"id":"sub-1","platform":"twitch","type":"sub","user":{"id":"2","name":"bea","roles":["subscriber"]},"data":{"tier":"1000","months":3},"ts":"2026-10-31T20:01:00Z"}"#;
    const GIFT_SUB: &str = r#"{"id":"gift-1","platform":"kick","type":"gift_sub","user":{"id":"3","name":"carla","roles":[]},"data":{"tier":"gold","count":5},"ts":"2026-10-31T20:02:00Z"}"#;
    const RAID: &str = r#"{"id":"raid-1","platform":"twitch","type":"raid","user":{"id":"4","name":"diego","roles":["broadcaster"]},"data":{"viewers":42},"ts":"2026-10-31T20:03:00Z"}"#;
    const DONATION: &str = r#"{"id":"donation-1","platform":"youtube","type":"donation","user":{"id":"5","name":"eva","roles":[]},"data":{"amount":12.5,"currency":"USD","message":"gran stream"},"ts":"2026-10-31T20:04:00Z"}"#;
    const REDEMPTION: &str = r#"{"id":"redemption-1","platform":"twitch","type":"redemption","user":{"id":"6","name":"fede","roles":[]},"data":{"reward_id":"reward-7","reward_title":"Di mi nombre","cost":500,"input":"hola"},"ts":"2026-10-31T20:05:00Z"}"#;
    const CHAT: &str = r#"{"id":"chat-1","platform":"kick","type":"chat","user":{"id":"7","name":"gabi","roles":["mod","subscriber"]},"data":{"message":"buenas"},"ts":"2026-10-31T20:06:00Z"}"#;
    const PLUGIN: &str = r#"{"id":"plugin-1","platform":"plugin:spotify","type":"now_playing","user":{"id":"plugin","name":"spotify","roles":[]},"data":{"artist":"Ana","track":"Luz","position":17},"ts":"2026-10-31T20:07:00Z"}"#;

    fn assert_roundtrip(fixture: &str) -> Event {
        let event: Event = serde_json::from_str(fixture).expect("fixture should deserialize");
        let actual = serde_json::to_value(&event).expect("event should serialize");
        let expected: Value = serde_json::from_str(fixture).expect("fixture should be valid JSON");
        assert_eq!(actual, expected);
        event
    }

    #[test]
    fn roundtrips_every_core_event_fixture() {
        let follow = assert_roundtrip(FOLLOW);
        assert!(matches!(follow.event, EventData::Follow(_)));
        let sub = assert_roundtrip(SUB);
        assert!(matches!(sub.event, EventData::Sub(_)));
        let gift_sub = assert_roundtrip(GIFT_SUB);
        assert!(matches!(gift_sub.event, EventData::GiftSub(_)));
        let raid = assert_roundtrip(RAID);
        assert!(matches!(raid.event, EventData::Raid(_)));
        let donation = assert_roundtrip(DONATION);
        assert!(matches!(donation.event, EventData::Donation(_)));
        let redemption = assert_roundtrip(REDEMPTION);
        assert!(matches!(redemption.event, EventData::Redemption(_)));
        let chat = assert_roundtrip(CHAT);
        assert!(matches!(chat.event, EventData::Chat(_)));
    }

    #[test]
    fn roundtrips_plugin_event_and_platform() {
        let event = assert_roundtrip(PLUGIN);
        assert_eq!(event.platform, Platform::Plugin("spotify".into()));
        assert_eq!(
            event.event,
            EventData::Plugin(PluginData {
                name: "now_playing".into(),
                data: serde_json::from_str(r#"{"artist":"Ana","track":"Luz","position":17}"#,)
                    .expect("plugin data should be valid JSON"),
            })
        );
    }

    #[test]
    fn rejects_invalid_core_payload_with_clear_error() {
        let fixture = r#"{"id":"sub-1","platform":"twitch","type":"sub","user":{"id":"2","name":"bea","roles":[]},"data":{"tier":"1000","months":"three"},"ts":"2026-10-31T20:01:00Z"}"#;
        let error = serde_json::from_str::<Event>(fixture)
            .expect_err("invalid core payload should be rejected")
            .to_string();
        assert!(error.contains("invalid data for event type `sub`"));
        assert!(error.contains("months"));
    }

    #[test]
    fn rejects_invalid_platform_with_clear_error() {
        let fixture = r#"{"id":"follow-1","platform":"discord","type":"follow","user":{"id":"1","name":"ana","roles":[]},"data":{},"ts":"2026-10-31T20:00:00Z"}"#;
        let error = serde_json::from_str::<Event>(fixture)
            .expect_err("unknown platform should be rejected")
            .to_string();
        assert!(error.contains("invalid platform `discord`"));
    }

    #[test]
    fn rejects_empty_plugin_platform_name() {
        let fixture = r#"{"id":"follow-1","platform":"plugin:","type":"follow","user":{"id":"1","name":"ana","roles":[]},"data":{},"ts":"2026-10-31T20:00:00Z"}"#;
        let error = serde_json::from_str::<Event>(fixture)
            .expect_err("empty plugin platform should be rejected")
            .to_string();
        assert!(error.contains("invalid platform `plugin:`"));
    }
}
