use super::KickSettings;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ids {
    pub chatroom: u64,
    pub channel: u64,
}

#[derive(Deserialize)]
struct Channel {
    id: u64,
    chatroom: Chatroom,
}
#[derive(Deserialize)]
struct Chatroom {
    id: u64,
}

pub(crate) async fn resolve(
    settings: &KickSettings,
    http: &reqwest::Client,
) -> Result<Ids, String> {
    if let (Some(chatroom), Some(channel)) = (settings.chatroom_id, settings.channel_id) {
        return Ok(Ids { chatroom, channel });
    }
    let url = format!(
        "{}/api/v2/channels/{}",
        settings.urls.api.trim_end_matches('/'),
        settings.channel
    );
    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|error| manual(&settings.channel, &error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(manual(&settings.channel, &format!("HTTP {status}")));
    }
    let found: Channel = response
        .json()
        .await
        .map_err(|error| manual(&settings.channel, &format!("respuesta inválida: {error}")))?;
    Ok(Ids {
        chatroom: settings.chatroom_id.unwrap_or(found.chatroom.id),
        channel: settings.channel_id.unwrap_or(found.id),
    })
}

fn manual(slug: &str, reason: &str) -> String {
    format!(
        "no se pudieron resolver los ids de Kick ({reason}); abre https://kick.com/api/v2/channels/{slug} en el navegador y configura kick.channel_id con `id` y kick.chatroom_id con `chatroom.id`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::StatusCode, routing::get};
    use serde_json::json;
    use tokio::net::TcpListener;

    async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (base, task)
    }

    #[tokio::test]
    async fn resolves_missing_ids_and_preserves_explicit_values() {
        let (base, task) = server(Router::new().route(
            "/api/v2/channels/canal",
            get(|| async { Json(json!({"id":20,"chatroom":{"id":10}})) }),
        ))
        .await;
        let mut settings = KickSettings::new("canal".into());
        settings.channel_id = Some(99);
        settings.urls.api = base;
        assert_eq!(
            resolve(&settings, &reqwest::Client::new()).await.unwrap(),
            Ids {
                chatroom: 10,
                channel: 99
            }
        );
        let mut explicit = KickSettings::new("canal".into());
        explicit.chatroom_id = Some(10);
        explicit.channel_id = Some(20);
        explicit.urls.api = ":// URL deliberadamente inválida".into();
        assert_eq!(
            resolve(&explicit, &reqwest::Client::new()).await.unwrap(),
            Ids {
                chatroom: 10,
                channel: 20
            }
        );
        task.abort();
    }

    #[tokio::test]
    async fn reports_manual_instructions_after_403() {
        let (base, task) = server(Router::new().route(
            "/api/v2/channels/canal",
            get(|| async { StatusCode::FORBIDDEN }),
        ))
        .await;
        let mut settings = KickSettings::new("canal".into());
        settings.urls.api = base;
        let error = resolve(&settings, &reqwest::Client::new())
            .await
            .unwrap_err();
        assert!(error.contains("HTTP 403 Forbidden"));
        assert!(error.contains("kick.channel_id") && error.contains("kick.chatroom_id"));
        task.abort();
    }
}
