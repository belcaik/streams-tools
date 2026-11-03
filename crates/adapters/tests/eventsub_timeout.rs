mod support;

use std::time::Duration;

use futures_util::SinkExt;
use streams_tools_adapters::twitch::TwitchAdapter;
use streams_tools_core::{Bus, EventData, Platform};
use tokio::{sync::oneshot, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use support::{context, welcome, ws_listener};

#[tokio::test]
async fn keepalive_timeout_reconnects_and_resubscribes() {
    timeout(Duration::from_secs(2), async {
        let (listener, url) = ws_listener().await;
        let (finish_tx, finish_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut first = accept_async(stream).await.unwrap();
            first
                .send(Message::Text(welcome("first", 0).into()))
                .await
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut second = accept_async(stream).await.unwrap();
            second
                .send(Message::Text(welcome("second", 30).into()))
                .await
                .unwrap();
            let _ = finish_rx.await;
        });
        let mut context = context(Duration::from_millis(20)).await;
        context.settings.urls.websocket = url;
        let bus = Bus::new(32).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = TwitchAdapter::start(bus, context.settings.clone()).unwrap();
        let mut connected = 0;
        let mut disconnected = 0;
        while connected < 2 {
            let event = subscriber.recv().await.unwrap();
            assert_eq!(event.platform, Platform::Twitch);
            if let EventData::Plugin(data) = &event.event {
                if data.name != "system" {
                    continue;
                }
                match data.data["status"].as_str() {
                    Some("connected") => connected += 1,
                    Some("disconnected") => {
                        disconnected += 1;
                        assert_eq!(data.data["message"], "timeout de keepalive");
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(disconnected, 1);
        {
            let bodies = context.posts.lock().unwrap();
            assert_eq!(bodies.len(), 16);
            assert_eq!(
                bodies
                    .iter()
                    .filter(|body| body["transport"]["session_id"] == "first")
                    .count(),
                8
            );
            assert_eq!(
                bodies
                    .iter()
                    .filter(|body| body["transport"]["session_id"] == "second")
                    .count(),
                8
            );
        }
        adapter.shutdown().await;
        let _ = finish_tx.send(());
        server.await.unwrap();
    })
    .await
    .expect("timeout scenario should finish within two seconds");
}
