mod support;

use std::time::Duration;

use futures_util::SinkExt;
use serde_json::{Value, json};
use streams_tools_adapters::twitch::TwitchAdapter;
use streams_tools_core::{Bus, Event, EventData, Platform};
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use support::{context, message, welcome, ws_listener};

fn is_system(event: &Event, status: &str) -> bool {
    assert_eq!(event.platform, Platform::Twitch);
    matches!(&event.event, EventData::Plugin(data)
        if data.name == "system" && data.data["status"] == status)
}

#[tokio::test]
async fn migrates_deduplicates_and_shuts_down_cleanly() {
    timeout(Duration::from_secs(2), async {
        let (first_listener, first_url) = ws_listener().await;
        let (second_listener, second_url) = ws_listener().await;
        let first = tokio::spawn(async move {
            let (stream, _) = first_listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(welcome("first", 30).into()))
                .await
                .unwrap();
            let chat = include_str!("fixtures/twitch/chat.json").to_string();
            socket
                .send(Message::Text(chat.clone().into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    message("keepalive-1", "session_keepalive", json!({})).into(),
                ))
                .await
                .unwrap();
            socket.send(Message::Text(chat.into())).await.unwrap();
            socket
                .send(Message::Text(
                    message(
                        "reconnect-1",
                        "session_reconnect",
                        json!({"session": {"reconnect_url": second_url}}),
                    )
                    .into(),
                ))
                .await
                .unwrap();
        });
        let second = tokio::spawn(async move {
            let (stream, _) = second_listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(welcome("second", 30).into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    include_str!("fixtures/twitch/follow.json").into(),
                ))
                .await
                .unwrap();
        });
        let mut context = context(Duration::from_secs(5)).await;
        context.settings.urls.websocket = first_url;
        let bus = Bus::new(32).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = TwitchAdapter::start(bus, context.settings.clone()).unwrap();
        let mut connected = 0;
        let mut chats = 0;
        let mut disconnected = 0;
        let follow = loop {
            let event = subscriber.recv().await.unwrap();
            connected += usize::from(is_system(&event, "connected"));
            disconnected += usize::from(is_system(&event, "disconnected"));
            if matches!(&event.event, EventData::Chat(data) if data.message == "buenas") {
                chats += 1;
            }
            if matches!(event.event, EventData::Follow(_)) {
                break event;
            }
        };
        assert_eq!(follow.id, "msg-follow");
        assert_eq!(connected, 2);
        assert_eq!(chats, 1);
        assert_eq!(disconnected, 0);
        {
            let bodies = context.posts.lock().unwrap();
            assert_eq!(bodies.len(), 8);
            assert!(
                bodies.iter().all(|body| {
                    body["transport"]["session_id"] == Value::String("first".into())
                })
            );
        }
        adapter.shutdown().await;
        first.await.unwrap();
        second.await.unwrap();
    })
    .await
    .expect("session scenario should finish within two seconds");
}
