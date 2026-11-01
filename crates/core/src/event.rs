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
    pub months: u64,
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
