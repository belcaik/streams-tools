use serde_json::{Value, json};
use std::sync::atomic::AtomicU64;
use streams_tools_adapters::kick::map::event;
use streams_tools_core::{
    ChatData, EventData, FollowData, GiftSubData, Platform, RaidData, SubData,
};

fn mapped(source: &str) -> streams_tools_core::Event {
    let frame: Value = serde_json::from_str(source).unwrap();
    event(&frame, &AtomicU64::new(0)).unwrap().unwrap()
}

#[test]
fn maps_every_supported_event() {
    let chat = mapped(include_str!("fixtures/kick/chat.json"));
    assert_eq!(chat.platform, Platform::Kick);
    assert_eq!(chat.id, "chat-1");
    assert_eq!(chat.ts, "2026-01-02T03:04:05.678Z");
    assert_eq!(
        chat.user.roles,
        ["broadcaster", "moderator", "vip", "subscriber"]
    );
    assert_eq!(
        chat.event,
        EventData::Chat(ChatData {
            message: "buenas".into()
        })
    );
    assert_eq!(
        mapped(include_str!("fixtures/kick/sub.json")).event,
        EventData::Sub(SubData {
            tier: "1".into(),
            months: 7
        })
    );
    assert_eq!(
        mapped(include_str!("fixtures/kick/gift_sub.json")).event,
        EventData::GiftSub(GiftSubData {
            tier: "1".into(),
            count: 5
        })
    );
    assert_eq!(
        mapped(include_str!("fixtures/kick/raid.json")).event,
        EventData::Raid(RaidData { viewers: 42 })
    );
    assert_eq!(
        mapped(include_str!("fixtures/kick/follow.json")).event,
        EventData::Follow(FollowData {})
    );
}

#[test]
fn supports_channel_subscription_and_fallback_identity() {
    let frame = json!({"event":"App\\Events\\ChannelSubscriptionEvent","channel":"channel.20",
        "data":"{\"username\":\"Gabi\"}"});
    let value = event(&frame, &AtomicU64::new(9)).unwrap().unwrap();
    assert_eq!(value.id, "kick-9");
    assert_eq!(value.user.id, "Gabi");
    assert!(value.ts.ends_with('Z'));
}

#[test]
fn ignores_unknown_unfollow_and_lucky_recipients() {
    let counter = AtomicU64::new(0);
    assert_eq!(event(&json!({"event":"future"}), &counter).unwrap(), None);
    assert_eq!(
        event(
            &json!({"event":"App\\Events\\LuckyUsersWhoGotGiftSubscriptionsEvent"}),
            &counter
        )
        .unwrap(),
        None
    );
    let frame = json!({"event":"App\\Events\\FollowersUpdated","data":"{\"username\":\"Ana\",\"followed\":false}"});
    assert_eq!(event(&frame, &counter).unwrap(), None);
}
