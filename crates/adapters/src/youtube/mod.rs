mod resolve;
mod session;

pub use session::YoutubeAdapter;

pub mod map;

use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YoutubeEvent {
    Chat,
    Donation,
    Sub,
    GiftSub,
}

impl YoutubeEvent {
    pub const ALL: [Self; 4] = [Self::Chat, Self::Donation, Self::Sub, Self::GiftSub];
}

#[derive(Debug, Clone)]
pub struct YoutubeUrls {
    pub web: String,
    pub innertube: String,
}

impl Default for YoutubeUrls {
    fn default() -> Self {
        Self {
            web: "https://www.youtube.com".into(),
            innertube: "https://www.youtube.com".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct YoutubeSettings {
    pub channel: Option<String>,
    pub video_id: Option<String>,
    pub events: BTreeSet<YoutubeEvent>,
    pub urls: YoutubeUrls,
    pub offline_retry: Duration,
    pub poll_min: Duration,
    pub poll_max: Duration,
}

impl YoutubeSettings {
    pub fn new(channel: Option<String>, video_id: Option<String>) -> Self {
        Self {
            channel,
            video_id,
            events: YoutubeEvent::ALL.into_iter().collect(),
            urls: YoutubeUrls::default(),
            offline_retry: Duration::from_secs(60),
            poll_min: Duration::from_secs(1),
            poll_max: Duration::from_secs(10),
        }
    }
}
