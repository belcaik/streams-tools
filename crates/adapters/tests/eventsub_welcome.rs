mod support;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use streams_tools_adapters::twitch::TwitchAdapter;
use streams_tools_core::{Bus, EventData, Platform};
use tokio::{net::TcpListener, sync::oneshot, time::timeout};
use tokio_tungstenite::{
    WebSocketStream, accept_async,
    tungstenite::{
        Message,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

use support::{context, message, welcome, ws_listener};

type Server = WebSocketStream<tokio::net::TcpStream>;

async fn accept(listener: &TcpListener) -> Server {
    let (stream, _) = listener.accept().await.unwrap();
    accept_async(stream).await.unwrap()
}

/// Returns the next `connected` or `disconnected` system event.
async fn next_status(subscriber: &mut streams_tools_core::Subscriber) -> (String, String) {
    loop {
        let event = subscriber.recv().await.unwrap();
        assert_eq!(event.platform, Platform::Twitch);
        if let EventData::Plugin(data) = &event.event {
            let status = data.data["status"].as_str().unwrap_or_default();
            if data.name == "system" && matches!(status, "connected" | "disconnected") {
                let text = data.data["message"].as_str().unwrap_or_default();
                return (status.to_string(), text.to_string());
            }
        }
    }
}

#[tokio::test]
async fn ping_before_welcome_is_answered_and_session_connects() {
    timeout(Duration::from_secs(2), async {
        let (listener, url) = ws_listener().await;
        let (finish_tx, finish_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            socket
                .send(Message::Ping(b"hola".to_vec().into()))
                .await
                .unwrap();
            let pong = socket.next().await.unwrap().unwrap();
            assert_eq!(pong, Message::Pong(b"hola".to_vec().into()));
            socket.send(Message::Pong(Vec::new().into())).await.unwrap();
            socket
                .send(Message::Text(welcome("first", 30).into()))
                .await
                .unwrap();
            let _ = finish_rx.await;
        });
        let mut context = context(Duration::from_secs(5)).await;
        context.settings.urls.websocket = url;
        let bus = Bus::new(32).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = TwitchAdapter::start(bus, context.settings.clone()).unwrap();
        assert_eq!(next_status(&mut subscriber).await.0, "connected");
        assert_eq!(context.posts.lock().unwrap().len(), 8);
        adapter.shutdown().await;
        let _ = finish_tx.send(());
        server.await.unwrap();
    })
    .await
    .expect("ping scenario should finish within two seconds");
}

#[tokio::test]
async fn close_before_welcome_reports_code_and_reason() {
    timeout(Duration::from_secs(2), async {
        let (listener, url) = ws_listener().await;
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            socket
                .close(Some(CloseFrame {
                    code: CloseCode::Policy,
                    reason: "sin suscripciones".into(),
                }))
                .await
                .unwrap();
        });
        let mut context = context(Duration::from_secs(5)).await;
        context.settings.urls.websocket = url;
        let bus = Bus::new(32).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = TwitchAdapter::start(bus, context.settings.clone()).unwrap();
        let (status, text) = next_status(&mut subscriber).await;
        assert_eq!(status, "disconnected");
        assert!(text.contains("código 1008"), "{text}");
        assert!(text.contains("sin suscripciones"), "{text}");
        adapter.shutdown().await;
        server.await.unwrap();
    })
    .await
    .expect("close scenario should finish within two seconds");
}

#[tokio::test]
async fn unexpected_messages_before_welcome_are_reported() {
    timeout(Duration::from_secs(2), async {
        let (listener, url) = ws_listener().await;
        let (finish_tx, finish_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let mut first = accept(&listener).await;
            first
                .send(Message::Binary(vec![1, 2, 3].into()))
                .await
                .unwrap();
            let mut second = accept(&listener).await;
            second
                .send(Message::Text(
                    message("keepalive", "session_keepalive", serde_json::json!({})).into(),
                ))
                .await
                .unwrap();
            let _ = finish_rx.await;
            drop((first, second));
        });
        let mut context = context(Duration::from_secs(5)).await;
        context.settings.urls.websocket = url;
        let bus = Bus::new(32).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = TwitchAdapter::start(bus, context.settings.clone()).unwrap();
        let (_, binary) = next_status(&mut subscriber).await;
        assert!(
            binary.contains("frame binario inesperado (3 bytes)"),
            "{binary}"
        );
        let (_, keepalive) = next_status(&mut subscriber).await;
        assert!(keepalive.contains("llegó session_keepalive"), "{keepalive}");
        assert!(context.posts.lock().unwrap().is_empty());
        adapter.shutdown().await;
        let _ = finish_tx.send(());
        server.await.unwrap();
    })
    .await
    .expect("unexpected message scenario should finish within two seconds");
}
