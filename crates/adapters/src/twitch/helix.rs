use std::{collections::BTreeSet, error::Error, fmt};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{TwitchEvent, auth::Auth};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct User {
    pub id: String,
    pub login: String,
    #[serde(rename = "display_name")]
    pub name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Helix {
    client_id: String,
    base: String,
    http: reqwest::Client,
    auth: Auth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelixError(pub(crate) String);

impl fmt::Display for HelixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HelixError {}

#[derive(Deserialize)]
struct UsersResponse {
    data: Vec<User>,
}

impl Helix {
    pub(crate) fn new(client_id: String, base: String, http: reqwest::Client, auth: Auth) -> Self {
        Self {
            client_id,
            base: base.trim_end_matches('/').into(),
            http,
            auth,
        }
    }

    pub(crate) async fn resolve_broadcaster(&self, login: &str) -> Result<User, HelixError> {
        let channel = self.user(Some(login)).await?;
        let account = self.user(None).await?;
        if channel.id != account.id {
            return Err(HelixError(
                "la cuenta autorizada no corresponde al canal configurado".into(),
            ));
        }
        Ok(channel)
    }

    async fn user(&self, login: Option<&str>) -> Result<User, HelixError> {
        let query = login.map(|login| vec![("login", login)]);
        let response = self
            .send(reqwest::Method::GET, "/helix/users", query.as_deref(), None)
            .await?;
        let response = success(response).await?;
        let mut users = response
            .json::<UsersResponse>()
            .await
            .map_err(|error| HelixError(format!("respuesta Helix inválida: {error}")))?
            .data;
        if users.len() != 1 {
            return Err(HelixError(format!(
                "Helix debe devolver exactamente un usuario, recibió {}",
                users.len()
            )));
        }
        Ok(users.remove(0))
    }

    pub(crate) async fn create_subscriptions(
        &self,
        broadcaster_id: &str,
        session_id: &str,
        events: &BTreeSet<TwitchEvent>,
    ) -> Result<(), HelixError> {
        for event in events {
            let specs: &[(&str, &str, Value)] = match event {
                TwitchEvent::Follow => &[(
                    "channel.follow",
                    "2",
                    json!({
                        "broadcaster_user_id": broadcaster_id,
                        "moderator_user_id": broadcaster_id,
                    }),
                )],
                TwitchEvent::Sub => &[
                    (
                        "channel.subscribe",
                        "1",
                        json!({"broadcaster_user_id": broadcaster_id}),
                    ),
                    (
                        "channel.subscription.message",
                        "1",
                        json!({"broadcaster_user_id": broadcaster_id}),
                    ),
                ],
                TwitchEvent::GiftSub => &[(
                    "channel.subscription.gift",
                    "1",
                    json!({"broadcaster_user_id": broadcaster_id}),
                )],
                TwitchEvent::Raid => &[(
                    "channel.raid",
                    "1",
                    json!({"to_broadcaster_user_id": broadcaster_id}),
                )],
                TwitchEvent::Redemption => &[(
                    "channel.channel_points_custom_reward_redemption.add",
                    "1",
                    json!({"broadcaster_user_id": broadcaster_id}),
                )],
                TwitchEvent::Chat => &[(
                    "channel.chat.message",
                    "1",
                    json!({
                        "broadcaster_user_id": broadcaster_id,
                        "user_id": broadcaster_id,
                    }),
                )],
                TwitchEvent::Cheer => &[(
                    "channel.cheer",
                    "1",
                    json!({"broadcaster_user_id": broadcaster_id}),
                )],
            };
            for (kind, version, condition) in specs {
                let body = json!({
                    "type": kind,
                    "version": version,
                    "condition": condition,
                    "transport": {"method": "websocket", "session_id": session_id},
                });
                let response = self
                    .send(
                        reqwest::Method::POST,
                        "/helix/eventsub/subscriptions",
                        None,
                        Some(&body),
                    )
                    .await?;
                success(response).await?;
            }
        }
        Ok(())
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Option<&[(&str, &str)]>,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, HelixError> {
        for attempt in 0..2 {
            let token = self
                .auth
                .access_token(attempt == 1)
                .await
                .map_err(|error| HelixError(error.to_string()))?;
            let mut request = self
                .http
                .request(method.clone(), format!("{}{path}", self.base))
                .header("Client-Id", &self.client_id)
                .bearer_auth(token);
            if let Some(query) = query {
                request = request.query(query);
            }
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request
                .send()
                .await
                .map_err(|error| HelixError(format!("solicitud Helix {path}: {error}")))?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                continue;
            }
            return Ok(response);
        }
        unreachable!("el bucle de autenticación siempre devuelve una respuesta")
    }
}

async fn success(response: reqwest::Response) -> Result<reqwest::Response, HelixError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("no se pudo leer el cuerpo: {error}"));
    Err(HelixError(format!("error Helix {status}: {body}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::auth::tests::{Server, TempDir, auth as test_auth, server};
    use axum::{
        Json, Router,
        extract::Query,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    fn helix(server: &Server, dir: &TempDir) -> Helix {
        let path = dir.0.join("token.json");
        std::fs::write(&path, r#"{"access_token":"old-token","refresh_token":"old-refresh","expires_at":18446744073709551615,"scopes":["user:read:chat","bits:read"]}"#).unwrap();
        let auth = test_auth(server, path, Arc::new(|_| panic!("no prompt expected")));
        Helix::new(
            "client-id".into(),
            server.base.clone(),
            reqwest::Client::new(),
            auth,
        )
    }
    fn users(id: &str) -> Json<Value> {
        Json(json!({"data": [{"id": id, "login": "canal", "display_name": "Canal"}]}))
    }
    async fn users_server(account: &'static str) -> Server {
        let router = Router::new().route(
            "/helix/users",
            get(
                move |headers: HeaderMap, query: Query<HashMap<String, String>>| async move {
                    assert_eq!(headers["client-id"], "client-id");
                    assert_eq!(headers["authorization"], "Bearer old-token");
                    if let Some(login) = query.get("login") {
                        assert_eq!(login, "canal");
                    }
                    users(if query.contains_key("login") {
                        "42"
                    } else {
                        account
                    })
                },
            ),
        );
        server(router).await
    }
    #[tokio::test]
    async fn resolves_channel_and_rejects_a_different_account() {
        let server = users_server("42").await;
        let dir = TempDir::new();
        helix(&server, &dir)
            .resolve_broadcaster("canal")
            .await
            .unwrap();
        let server = users_server("different").await;
        let dir = TempDir::new();
        let error = helix(&server, &dir)
            .resolve_broadcaster("canal")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "la cuenta autorizada no corresponde al canal configurado"
        );
    }
    #[tokio::test]
    async fn refreshes_once_after_unauthorized() {
        let tokens = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured = tokens.clone();
        let router = Router::new()
            .route("/helix/users", get(move |headers: HeaderMap| {
                let captured = captured.clone();
                async move {
                    let mut tokens = captured.lock().unwrap();
                    tokens.push(headers["authorization"].to_str().unwrap().into());
                    if tokens.len() == 1 {
                        StatusCode::UNAUTHORIZED.into_response()
                    } else {
                        users("42").into_response()
                    }
                }
            }))
            .route("/oauth2/token", post(|body: String| async move {
                assert!(body.contains("refresh_token=old-refresh"));
                Json(json!({"access_token": "new-token", "expires_in": 3600, "scope": ["user:read:chat", "bits:read"]}))
            }));
        let server = server(router).await;
        let dir = TempDir::new();
        helix(&server, &dir)
            .resolve_broadcaster("canal")
            .await
            .unwrap();
        assert_eq!(
            *tokens.lock().unwrap(),
            ["Bearer old-token", "Bearer new-token", "Bearer new-token"]
        );
    }
    #[tokio::test]
    async fn creates_all_eight_subscription_bodies() {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let captured = bodies.clone();
        let router = Router::new().route(
            "/helix/eventsub/subscriptions",
            post(move |Json(body): Json<Value>| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push(body);
                    StatusCode::ACCEPTED
                }
            }),
        );
        let server = server(router).await;
        let dir = TempDir::new();
        helix(&server, &dir)
            .create_subscriptions("42", "session-1", &TwitchEvent::ALL.into_iter().collect())
            .await
            .unwrap();
        let expected = [
            ("channel.follow", "2"),
            ("channel.subscribe", "1"),
            ("channel.subscription.message", "1"),
            ("channel.subscription.gift", "1"),
            ("channel.raid", "1"),
            ("channel.channel_points_custom_reward_redemption.add", "1"),
            ("channel.chat.message", "1"),
            ("channel.cheer", "1"),
        ];
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), expected.len());
        for (body, (kind, version)) in bodies.iter().zip(expected) {
            let condition = match kind {
                "channel.follow" => json!({"broadcaster_user_id": "42", "moderator_user_id": "42"}),
                "channel.raid" => json!({"to_broadcaster_user_id": "42"}),
                "channel.chat.message" => json!({"broadcaster_user_id": "42", "user_id": "42"}),
                _ => json!({"broadcaster_user_id": "42"}),
            };
            assert_eq!(
                body,
                &json!({
                    "type": kind, "version": version, "condition": condition,
                    "transport": {"method": "websocket", "session_id": "session-1"}
                })
            );
        }
    }
}
