use axum::{
    Router,
    body::Bytes,
    http::HeaderMap,
    response::Html,
    routing::{get, post},
};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use streams_tools_adapters::youtube::{YoutubeAdapter, YoutubeSettings};
use streams_tools_core::{Bus, Event, EventData, Platform};
use tokio::{net::TcpListener, time::timeout};

fn is_system(event: &Event, status: &str) -> bool {
    event.platform == Platform::Youtube
        && matches!(&event.event, EventData::Plugin(data)
            if data.name == "system" && data.data["status"] == status)
}

#[tokio::test]
async fn skips_history_deduplicates_detects_offline_and_reconnects() {
    timeout(Duration::from_secs(2), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let posts = Arc::new(AtomicUsize::new(0));
        let captured = posts.clone();
        let router = Router::new()
            .route(
                "/@canal/live",
                get(|| async { Html(include_str!("fixtures/youtube/live.html")) }),
            )
            .route(
                "/live_chat",
                get(|| async { Html(include_str!("fixtures/youtube/live_chat.html")) }),
            )
            .route(
                "/youtubei/v1/live_chat/get_live_chat",
                post(move |headers: HeaderMap, body: Bytes| {
                    let captured = captured.clone();
                    async move {
                        assert!(
                            headers["user-agent"]
                                .to_str()
                                .unwrap()
                                .starts_with("streams-tools/")
                        );
                        let body: Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(body["context"]["client"]["clientName"], "WEB");
                        assert_eq!(
                            body["context"]["client"]["clientVersion"],
                            "2.20261103.00.00"
                        );
                        let index = captured.fetch_add(1, Ordering::Relaxed);
                        if index == 0 {
                            include_str!("fixtures/youtube/poll.json")
                        } else {
                            r#"{"continuationContents":{"liveChatContinuation":{"actions":[]}}}"#
                        }
                    }
                }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let mut settings = YoutubeSettings::new(Some("canal".into()), None);
        settings.urls.web = base.clone();
        settings.urls.innertube = base;
        settings.offline_retry = Duration::ZERO;
        settings.poll_min = Duration::ZERO;
        settings.poll_max = Duration::ZERO;
        let bus = Bus::new(64).unwrap();
        let mut subscriber = bus.subscribe();
        let adapter = YoutubeAdapter::start(bus, settings).unwrap();

        let mut connected = 0;
        let mut offline = 0;
        let mut chats = Vec::new();
        while connected < 2 {
            let event = subscriber.recv().await.unwrap();
            connected += usize::from(is_system(&event, "connected"));
            offline += usize::from(is_system(&event, "offline"));
            if let EventData::Chat(data) = event.event {
                chats.push(data.message);
            }
        }
        assert_eq!(connected, 2);
        assert!(offline >= 1);
        assert_eq!(chats, ["mensaje nuevo"]);
        assert!(posts.load(Ordering::Relaxed) >= 2);

        adapter.shutdown().await;
        server.abort();
    })
    .await
    .expect("local YouTube scenario should finish without real sleeps");
}
