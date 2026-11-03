mod pusher;
mod resolve;
pub use pusher::KickAdapter;
pub mod map;

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const DEFAULT_APP_KEY: &str = "32cbd69e4b950bf97679";
pub const DEFAULT_CLUSTER: &str = "us2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KickEvent {
    Chat,
    Sub,
    GiftSub,
    Raid,
    Follow,
}

impl KickEvent {
    pub const ALL: [Self; 5] = [
        Self::Chat,
        Self::Sub,
        Self::GiftSub,
        Self::Raid,
        Self::Follow,
    ];
}

#[derive(Debug, Clone)]
pub struct KickUrls {
    /// Base alternativa para tests; `None` construye el host Pusher público.
    pub websocket_base: Option<String>,
    pub api: String,
}

impl Default for KickUrls {
    fn default() -> Self {
        Self {
            websocket_base: None,
            api: "https://kick.com".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct KickSettings {
    pub channel: String,
    pub chatroom_id: Option<u64>,
    pub channel_id: Option<u64>,
    pub events: BTreeSet<KickEvent>,
    pub app_key: String,
    pub cluster: String,
    pub urls: KickUrls,
}

impl KickSettings {
    pub fn new(channel: String) -> Self {
        Self {
            channel,
            chatroom_id: None,
            channel_id: None,
            events: KickEvent::ALL.into_iter().collect(),
            app_key: DEFAULT_APP_KEY.into(),
            cluster: DEFAULT_CLUSTER.into(),
            urls: KickUrls::default(),
        }
    }

    pub fn websocket_url(&self) -> String {
        let base = self
            .urls
            .websocket_base
            .clone()
            .unwrap_or_else(|| format!("wss://ws-{}.pusher.com", self.cluster));
        format!(
            "{}/app/{}?protocol=7&client=js&version=8.4.0&flash=false",
            base.trim_end_matches('/'),
            self.app_key
        )
    }
}
