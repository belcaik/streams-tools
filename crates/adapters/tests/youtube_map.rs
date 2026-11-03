use serde_json::{Value, json};
use streams_tools_adapters::youtube::map::event;
use streams_tools_core::{ChatData, DonationData, EventData, GiftSubData, Platform, SubData};

fn actions() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("fixtures/youtube/actions.json"))
        .unwrap()
        .pointer("/continuationContents/liveChatContinuation/actions")
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn maps_every_supported_renderer_and_ignores_other_actions() {
    let values: Vec<_> = actions()
        .iter()
        .map(event)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(values.len(), 5);
    assert_eq!(values[0].platform, Platform::Youtube);
    assert_eq!(values[0].id, "chat-new");
    assert_eq!(values[0].ts, "2026-01-01T00:04:05.678Z");
    assert_eq!(
        values[0].user.roles,
        ["broadcaster", "moderator", "subscriber"]
    );
    assert_eq!(
        values[0].event,
        EventData::Chat(ChatData {
            message: "hola :wave:custom-id".into()
        })
    );
    assert_eq!(
        values[1].event,
        EventData::Donation(DonationData {
            amount: 5.0,
            currency: "EUR".into(),
            message: "gran stream".into()
        })
    );
    assert_eq!(
        values[2].event,
        EventData::Donation(DonationData {
            amount: 100.0,
            currency: "MXN".into(),
            message: String::new()
        })
    );
    assert_eq!(
        values[3].event,
        EventData::Sub(SubData {
            tier: "Nivel oro".into(),
            months: 12
        })
    );
    assert_eq!(
        values[4].event,
        EventData::GiftSub(GiftSubData {
            tier: "member".into(),
            count: 5
        })
    );
}

#[test]
fn parses_currency_symbols_localized_numbers_and_unknown_prefixes() {
    for (source, expected_amount, expected_currency) in [
        ("$5.00", 5.0, "USD"),
        ("€5,00", 5.0, "EUR"),
        ("MX$100.00", 100.0, "MXN"),
        ("¥500", 500.0, "JPY"),
        ("CA$2.00", 2.0, "CAD"),
        ("R$1.234,50", 1234.5, "BRL"),
        ("XYZ 1,234.50", 1234.5, "XYZ"),
    ] {
        let action = json!({"addChatItemAction":{"item":{"liveChatPaidMessageRenderer":{
            "id":"paid", "timestampUsec":"0", "authorExternalChannelId":"UC-user",
            "authorName":{"simpleText":"User"}, "purchaseAmountText":{"simpleText":source}
        }}}});
        let mapped = event(&action).unwrap().unwrap();
        let EventData::Donation(data) = mapped.event else {
            panic!("expected donation")
        };
        assert_eq!(
            (data.amount, data.currency.as_str()),
            (expected_amount, expected_currency)
        );
    }
}

#[test]
fn parses_first_integer_without_depending_on_language() {
    for (text, expected) in [("Member for 24 months", 24), ("Miembro durante 7 meses", 7)] {
        let action = json!({"addChatItemAction":{"item":{"liveChatMembershipItemRenderer":{
            "id":"member", "timestampUsec":"0", "authorExternalChannelId":"UC-user",
            "authorName":{"simpleText":"User"}, "headerPrimaryText":{"simpleText":text}
        }}}});
        let mapped = event(&action).unwrap().unwrap();
        assert_eq!(
            mapped.event,
            EventData::Sub(SubData {
                tier: "member".into(),
                months: expected
            })
        );
    }
}
