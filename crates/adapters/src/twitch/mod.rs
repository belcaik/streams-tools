pub mod map;

use std::{collections::BTreeSet, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TwitchEvent {
    Follow,
    Sub,
    GiftSub,
    Raid,
    Redemption,
    Chat,
    Cheer,
}

impl TwitchEvent {
    pub const ALL: [Self; 7] = [
        Self::Follow,
        Self::Sub,
        Self::GiftSub,
        Self::Raid,
        Self::Redemption,
        Self::Chat,
        Self::Cheer,
    ];
}

#[derive(Debug, Clone)]
pub struct TwitchUrls {
    pub websocket: String,
    pub helix: String,
    pub identity: String,
}

impl Default for TwitchUrls {
    fn default() -> Self {
        Self {
            websocket: "wss://eventsub.wss.twitch.tv/ws".into(),
            helix: "https://api.twitch.tv".into(),
            identity: "https://id.twitch.tv".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TwitchSettings {
    pub client_id: String,
    pub channel: String,
    pub events: BTreeSet<TwitchEvent>,
    pub data_dir: PathBuf,
    pub urls: TwitchUrls,
    pub keepalive_margin: Duration,
}

impl TwitchSettings {
    pub fn new(client_id: String, channel: String, data_dir: PathBuf) -> Self {
        Self {
            client_id,
            channel,
            events: TwitchEvent::ALL.into_iter().collect(),
            data_dir,
            urls: TwitchUrls::default(),
            keepalive_margin: Duration::from_secs(5),
        }
    }
}
