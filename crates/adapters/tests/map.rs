use serde_json::{Value, json};
use streams_tools_adapters::twitch::map::notification;
use streams_tools_core::{
    ChatData, DonationData, Event, EventData, FollowData, GiftSubData, Platform, RaidData,
    RedemptionData, SubData, User,
};

fn assert_fixture(fixture: &str, id: &str, user: User, event: EventData) {
    let envelope: Value = serde_json::from_str(fixture).expect("fixture should be valid JSON");
    let expected = Event {
        id: id.into(),
        platform: Platform::Twitch,
        event,
        user,
        ts: "2026-01-02T03:04:05.678Z".into(),
    };
    assert_eq!(notification(&envelope), Ok(Some(expected)));
}

fn user(id: &str, name: &str) -> User {
    User {
        id: id.into(),
        name: name.into(),
        roles: Vec::new(),
    }
}

#[test]
fn maps_follow() {
    assert_fixture(
        include_str!("fixtures/twitch/follow.json"),
        "msg-follow",
        user("123", "Ana"),
        EventData::Follow(FollowData {}),
    );
}

#[test]
fn maps_subscription_renewal() {
    assert_fixture(
        include_str!("fixtures/twitch/sub.json"),
        "msg-sub",
        user("456", "Bea"),
        EventData::Sub(SubData {
            tier: "1".into(),
            months: 7,
        }),
    );
}

#[test]
fn maps_gift_sub() {
    assert_fixture(
        include_str!("fixtures/twitch/gift_sub.json"),
        "msg-gift",
        user("789", "Carla"),
        EventData::GiftSub(GiftSubData {
            tier: "2".into(),
            count: 5,
        }),
    );
}

#[test]
fn maps_raid() {
    assert_fixture(
        include_str!("fixtures/twitch/raid.json"),
        "msg-raid",
        user("111", "Diego"),
        EventData::Raid(RaidData { viewers: 42 }),
    );
}

#[test]
fn maps_redemption() {
    assert_fixture(
        include_str!("fixtures/twitch/redemption.json"),
        "msg-redemption",
        user("222", "Eva"),
        EventData::Redemption(RedemptionData {
            reward_id: "reward-1".into(),
            reward_title: "Di mi nombre".into(),
            cost: 500,
            input: "hola".into(),
        }),
    );
}

#[test]
fn maps_chat_with_ordered_unique_roles() {
    let mut expected_user = user("333", "Fede");
    expected_user.roles = ["broadcaster", "moderator", "vip", "subscriber"]
        .map(str::to_owned)
        .into();
    assert_fixture(
        include_str!("fixtures/twitch/chat.json"),
        "msg-chat",
        expected_user,
        EventData::Chat(ChatData {
            message: "buenas".into(),
        }),
    );
}

#[test]
fn maps_anonymous_cheer_as_bits_donation() {
    assert_fixture(
        include_str!("fixtures/twitch/cheer.json"),
        "msg-cheer",
        user("anonymous", "Anonymous"),
        EventData::Donation(DonationData {
            amount: 250.0,
            currency: "bits".into(),
            message: "gran stream".into(),
        }),
    );
}

#[test]
fn maps_new_subscription_and_ignores_gifts() {
    let mut envelope = json!({
        "metadata": {
            "message_id": "msg-new-sub",
            "message_timestamp": "2026-01-02T03:04:05.678Z",
            "subscription_type": "channel.subscribe"
        },
        "payload": {"event": {
            "user_id": "444", "user_name": "Gabi", "tier": "3000", "is_gift": false
        }}
    });
    let expected = Event {
        id: "msg-new-sub".into(),
        platform: Platform::Twitch,
        event: EventData::Sub(SubData {
            tier: "3".into(),
            months: 1,
        }),
        user: user("444", "Gabi"),
        ts: "2026-01-02T03:04:05.678Z".into(),
    };
    assert_eq!(notification(&envelope).unwrap(), Some(expected));
    envelope["payload"]["event"]["is_gift"] = Value::Bool(true);
    assert_eq!(notification(&envelope).unwrap(), None);
}

#[test]
fn ignores_unknown_type_without_payload() {
    let envelope = json!({"metadata": {"subscription_type": "channel.unknown"}});
    assert_eq!(notification(&envelope).unwrap(), None);
}
