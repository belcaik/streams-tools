use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use streams_tools_adapters::kick::{KickAdapter, KickSettings};
use streams_tools_core::{Bus, Event, EventData, Platform};
use tokio::{net::TcpListener, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

fn text(value: Value) -> Message {
    Message::Text(value.to_string().into())
}
fn established() -> Message {
    text(
        json!({"event":"pusher:connection_established","data":"{\"socket_id\":\"1.2\",\"activity_timeout\":30}"}),
    )
}
fn succeeded(channel: &str) -> Message {
    text(json!({"event":"pusher_internal:subscription_succeeded","channel":channel,"data":"{}"}))
}
fn is_system(event: &Event, status: &str) -> bool {
    event.platform == Platform::Kick
        && matches!(&event.event, EventData::Plugin(data) if data.name == "system" && data.data["status"] == status)
}

async fn subscriptions(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
    let mut channels = Vec::new();
    for _ in 0..2 {
        let Message::Text(value) = socket.next().await.unwrap().unwrap() else {
            panic!("text subscription")
        };
        let frame: Value = serde_json::from_str(value.as_str()).unwrap();
        assert_eq!(frame["event"], "pusher:subscribe");
        channels.push(frame["data"]["channel"].as_str().unwrap().to_owned());
    }
    channels.sort();
    assert_eq!(channels, ["channel.20", "chatrooms.10.v2"]);
    for channel in channels {
        socket.send(succeeded(&channel)).await.unwrap();
    }
}

#[tokio::test]
async fn subscribes_pongs_deduplicates_reconnects_and_shuts_down() {
    timeout(Duration::from_secs(4), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut first = accept_async(stream).await.unwrap();
            first.send(established()).await.unwrap();
            subscriptions(&mut first).await;
            let chat = include_str!("fixtures/kick/chat.json");
            first.send(Message::Text(chat.into())).await.unwrap();
            first.send(Message::Text(chat.into())).await.unwrap();
            first
                .send(text(json!({"event":"pusher:ping","data":{}})))
                .await
                .unwrap();
            let Message::Text(pong) = first.next().await.unwrap().unwrap() else {
                panic!("text pong")
            };
            assert_eq!(
                serde_json::from_str::<Value>(pong.as_str()).unwrap()["event"],
                "pusher:pong"
            );
            first.close(None).await.unwrap();

            let (stream, _) = listener.accept().await.unwrap();
            let mut second = accept_async(stream).await.unwrap();
            second.send(established()).await.unwrap();
            subscriptions(&mut second).await;
            let _ = second.next().await;
        });
        let mut settings = KickSettings::new("canal".into());
        settings.chatroom_id = Some(10);
        settings.channel_id = Some(20);
        settings.urls.websocket_base = Some(url);
        let bus = Bus::new(32).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = KickAdapter::start(bus, settings).unwrap();
        let mut connected = 0;
        let mut disconnected = 0;
        let mut chats = 0;
        while connected < 2 {
            let event = subscriber.recv().await.unwrap();
            connected += usize::from(is_system(&event, "connected"));
            disconnected += usize::from(is_system(&event, "disconnected"));
            chats += usize::from(matches!(event.event, EventData::Chat(_)));
        }
        assert_eq!((connected, disconnected, chats), (2, 1, 1));
        adapter.shutdown().await;
        server.await.unwrap();
    })
    .await
    .expect("local Pusher scenario should finish");
}
