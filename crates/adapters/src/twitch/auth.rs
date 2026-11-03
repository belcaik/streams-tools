use std::{
    error::Error,
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::Instant};

#[derive(Debug, Clone)]
pub(crate) struct Auth {
    inner: Arc<Inner>,
}

struct Inner {
    client_id: String,
    scopes: Vec<String>,
    token_path: PathBuf,
    identity_base: String,
    http: reqwest::Client,
    prompt: Prompt,
    state: Mutex<State>,
}

impl fmt::Debug for Inner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Inner")
            .field("client_id", &self.client_id)
            .field("scopes", &self.scopes)
            .field("token_path", &self.token_path)
            .field("identity_base", &self.identity_base)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct State {
    loaded: bool,
    token: Option<Token>,
}

#[derive(Serialize, Deserialize)]
struct Token {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthError(pub(crate) String);

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for AuthError {}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

pub(crate) type Prompt = Arc<dyn Fn(&DeviceAuthorization) + Send + Sync>;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Vec<String>,
}

impl TokenResponse {
    fn into_token(self, previous_refresh: Option<&str>) -> Result<Token, AuthError> {
        let refresh_token = self
            .refresh_token
            .or_else(|| previous_refresh.map(str::to_owned))
            .ok_or_else(|| AuthError("Twitch no devolvió un refresh token".into()))?;
        Ok(Token {
            access_token: self.access_token,
            refresh_token,
            expires_at: unix_now().saturating_add(self.expires_in),
            scopes: self.scope,
        })
    }
}

impl Auth {
    pub(crate) fn new(
        client_id: String,
        scopes: Vec<String>,
        token_path: PathBuf,
        identity_base: String,
        http: reqwest::Client,
        prompt: Prompt,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                client_id,
                scopes,
                token_path,
                identity_base: identity_base.trim_end_matches('/').into(),
                http,
                prompt,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub(crate) async fn requires_login(&self) -> Result<bool, AuthError> {
        let mut state = self.inner.state.lock().await;
        self.load(&mut state).await?;
        Ok(state
            .token
            .as_ref()
            .is_none_or(|token| token.refresh_token.is_empty()))
    }

    pub(crate) async fn access_token(&self, force_refresh: bool) -> Result<String, AuthError> {
        let mut state = self.inner.state.lock().await;
        self.load(&mut state).await?;
        if let Some(token) = &state.token {
            if !force_refresh && token.expires_at > unix_now().saturating_add(30) {
                return Ok(token.access_token.clone());
            }
            if !token.refresh_token.is_empty() {
                if let Some(response) = self.refresh(&token.refresh_token).await? {
                    let token = response.into_token(Some(&token.refresh_token))?;
                    let access = token.access_token.clone();
                    self.persist(&token).await?;
                    state.token = Some(token);
                    return Ok(access);
                }
                state.token = None;
            }
        }
        self.device_flow(&mut state).await
    }

    async fn load(&self, state: &mut State) -> Result<(), AuthError> {
        if state.loaded {
            return Ok(());
        }
        state.token = match tokio::fs::read(&self.inner.token_path).await {
            Ok(bytes) => {
                let token: Token = serde_json::from_slice(&bytes).map_err(|error| {
                    AuthError(format!("{}: {error}", self.inner.token_path.display()))
                })?;
                self.inner
                    .scopes
                    .iter()
                    .all(|scope| token.scopes.contains(scope))
                    .then_some(token)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AuthError(format!(
                    "{}: {error}",
                    self.inner.token_path.display()
                )));
            }
        };
        state.loaded = true;
        Ok(())
    }

    async fn refresh(&self, refresh_token: &str) -> Result<Option<TokenResponse>, AuthError> {
        let response = self
            .post(
                "/oauth2/token",
                &[
                    ("client_id", &self.inner.client_id),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh_token),
                ],
            )
            .await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        response
            .json()
            .await
            .map(Some)
            .map_err(|error| AuthError(format!("respuesta de renovación inválida: {error}")))
    }

    async fn device_flow(&self, state: &mut State) -> Result<String, AuthError> {
        let scopes = self.inner.scopes.join(" ");
        let response = self
            .post(
                "/oauth2/device",
                &[("client_id", &self.inner.client_id), ("scopes", &scopes)],
            )
            .await?;
        let authorization = parse_success(response).await?;
        (self.inner.prompt)(&authorization);
        let deadline = Instant::now() + Duration::from_secs(authorization.expires_in);
        let mut interval = Duration::from_secs(authorization.interval);
        loop {
            if Instant::now() >= deadline {
                return Err(AuthError(
                    "el código de dispositivo de Twitch expiró".into(),
                ));
            }
            let response = self
                .post(
                    "/oauth2/token",
                    &[
                        ("client_id", &self.inner.client_id),
                        ("device_code", &authorization.device_code),
                        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ("scopes", &scopes),
                    ],
                )
                .await?;
            if response.status().is_success() {
                let token = parse_success::<TokenResponse>(response)
                    .await?
                    .into_token(None)?;
                let access = token.access_token.clone();
                self.persist(&token).await?;
                state.token = Some(token);
                return Ok(access);
            }
            let status = response.status();
            let body = response.text().await.map_err(|error| {
                AuthError(format!("no se pudo leer el error OAuth {status}: {error}"))
            })?;
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
                .unwrap_or(body);
            match message.as_str() {
                "authorization_pending" => {}
                "slow_down" => interval += Duration::from_secs(5),
                "expired_token" | "access_denied" => {
                    return Err(AuthError(format!(
                        "autorización Twitch terminada: {message}"
                    )));
                }
                _ => return Err(AuthError(format!("error OAuth {status}: {message}"))),
            }
            let wake = Instant::now() + interval;
            if wake >= deadline {
                return Err(AuthError(
                    "el código de dispositivo de Twitch expiró".into(),
                ));
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn post(
        &self,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<reqwest::Response, AuthError> {
        self.inner
            .http
            .post(format!("{}{path}", self.inner.identity_base))
            .form(form)
            .send()
            .await
            .map_err(|error| AuthError(format!("solicitud OAuth {path}: {error}")))
    }

    async fn persist(&self, token: &Token) -> Result<(), AuthError> {
        let path = &self.inner.token_path;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| path_error(parent, error))?;
        }
        let temporary = path.with_file_name("token.json.tmp");
        let bytes = serde_json::to_vec(token)
            .map_err(|error| AuthError(format!("no se pudo serializar el token: {error}")))?;
        tokio::fs::write(&temporary, bytes)
            .await
            .map_err(|error| path_error(&temporary, error))?;
        #[cfg(not(windows))]
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|error| path_error(path, error))?;
        #[cfg(windows)]
        if let Err(error) = tokio::fs::rename(&temporary, path).await {
            if !matches!(tokio::fs::try_exists(path).await, Ok(true)) {
                return Err(path_error(path, error));
            }
            tokio::fs::remove_file(path)
                .await
                .map_err(|error| path_error(path, error))?;
            tokio::fs::rename(&temporary, path)
                .await
                .map_err(|error| path_error(path, error))?;
        }
        Ok(())
    }
}

async fn parse_success<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, AuthError> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| error.to_string());
        return Err(AuthError(format!("error OAuth {status}: {body}")));
    }
    response
        .json()
        .await
        .map_err(|error| AuthError(format!("respuesta OAuth inválida: {error}")))
}

fn path_error(path: &Path, error: std::io::Error) -> AuthError {
    AuthError(format!("{}: {error}", path.display()))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use axum::{Json, Router, http::StatusCode, routing::post};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use super::*;

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TempDir(pub(crate) PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "streams-tools-auth-{}-{}",
                std::process::id(),
                TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("temp dir should be created");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) struct Server {
        pub(crate) base: String,
        task: JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    pub(crate) async fn server(router: Router) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener should bind");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Server { base, task }
    }

    pub(crate) fn auth(server: &Server, path: PathBuf, prompt: Prompt) -> Auth {
        Auth::new(
            "client-id".into(),
            vec!["user:read:chat".into(), "bits:read".into()],
            path,
            server.base.clone(),
            reqwest::Client::new(),
            prompt,
        )
    }

    fn token_file(dir: &TempDir) -> PathBuf {
        dir.0.join("twitch").join("token.json")
    }

    #[tokio::test]
    async fn device_flow_polls_persists_and_reloads_without_network() {
        timeout(Duration::from_secs(2), async {
            let dir = TempDir::new();
            let path = token_file(&dir);
            let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = prompts.clone();
            let prompt: Prompt = Arc::new(move |authorization| {
                captured.lock().unwrap().push(authorization.clone());
            });
            let polls = Arc::new(AtomicUsize::new(0));
            let poll_count = polls.clone();
            let router = Router::new()
                .route(
                    "/oauth2/device",
                    post(|body: String| async move {
                        assert!(body.contains("client_id=client-id"));
                        assert!(body.contains("scopes=user%3Aread%3Achat+bits%3Aread"));
                        Json(json!({
                            "device_code": "device-code",
                            "user_code": "USER-CODE",
                            "verification_uri": "https://twitch.test/activate",
                            "expires_in": 30,
                            "interval": 0
                        }))
                    }),
                )
                .route(
                    "/oauth2/token",
                    post(move |body: String| {
                        let polls = poll_count.clone();
                        async move {
                            assert!(body.contains("device_code=device-code"));
                            assert!(body.contains(
                                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"
                            ));
                            assert!(body.contains("scopes=user%3Aread%3Achat+bits%3Aread"));
                            if polls.fetch_add(1, Ordering::Relaxed) == 0 {
                                (
                                    StatusCode::BAD_REQUEST,
                                    Json(json!({"message": "authorization_pending"})),
                                )
                            } else {
                                (
                                    StatusCode::OK,
                                    Json(json!({
                                        "access_token": "new-access",
                                        "refresh_token": "new-refresh",
                                        "expires_in": 3600,
                                        "scope": ["user:read:chat", "bits:read"]
                                    })),
                                )
                            }
                        }
                    }),
                );
            let server = server(router).await;
            let auth = auth(&server, path.clone(), prompt);
            assert!(auth.requires_login().await.unwrap());
            assert_eq!(auth.access_token(false).await.unwrap(), "new-access");
            assert_eq!(polls.load(Ordering::Relaxed), 2);
            assert_eq!(prompts.lock().unwrap().len(), 1);
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["access_token"], "new-access");
            assert_eq!(saved["refresh_token"], "new-refresh");
            assert!(saved["expires_at"].as_u64().unwrap() > unix_now());
            let reloaded = self::auth(&server, path, Arc::new(|_| panic!("no prompt expected")));
            drop(server);

            assert!(!reloaded.requires_login().await.unwrap());
            assert_eq!(reloaded.access_token(false).await.unwrap(), "new-access");
        })
        .await
        .expect("device flow should finish within two seconds");
    }

    #[tokio::test]
    async fn missing_scopes_discard_saved_token_and_start_device_flow() {
        timeout(Duration::from_secs(2), async {
            let dir = TempDir::new();
            let path = token_file(&dir);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                serde_json::to_vec(&Token {
                    access_token: "old-access".into(),
                    refresh_token: "old-refresh".into(),
                    expires_at: unix_now() + 3600,
                    scopes: vec!["user:read:chat".into()],
                })
                .unwrap(),
            )
            .unwrap();
            let prompts = Arc::new(AtomicUsize::new(0));
            let prompt_count = prompts.clone();
            let prompt: Prompt = Arc::new(move |_| {
                prompt_count.fetch_add(1, Ordering::Relaxed);
            });
            let router = Router::new()
                .route(
                    "/oauth2/device",
                    post(|body: String| async move {
                        assert!(body.contains("scopes=bits%3Aread+user%3Aread%3Achat"));
                        Json(json!({
                            "device_code": "device-code",
                            "user_code": "USER-CODE",
                            "verification_uri": "https://twitch.test/activate",
                            "expires_in": 30,
                            "interval": 0
                        }))
                    }),
                )
                .route(
                    "/oauth2/token",
                    post(|body: String| async move {
                        assert!(body.contains("device_code=device-code"));
                        Json(json!({
                            "access_token": "new-access",
                            "refresh_token": "new-refresh",
                            "expires_in": 3600,
                            "scope": ["bits:read", "user:read:chat"]
                        }))
                    }),
                );
            let server = server(router).await;
            let auth = Auth::new(
                "client-id".into(),
                vec!["bits:read".into(), "user:read:chat".into()],
                path,
                server.base.clone(),
                reqwest::Client::new(),
                prompt,
            );

            assert!(auth.requires_login().await.unwrap());
            assert_eq!(auth.access_token(false).await.unwrap(), "new-access");
            assert_eq!(prompts.load(Ordering::Relaxed), 1);
        })
        .await
        .expect("device flow should finish within two seconds");
    }

    #[tokio::test]
    async fn expired_token_refreshes_and_persists_new_credentials() {
        timeout(Duration::from_secs(2), async {
            let dir = TempDir::new();
            let path = token_file(&dir);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                serde_json::to_vec(&Token {
                    access_token: "old-access".into(),
                    refresh_token: "old-refresh".into(),
                    expires_at: 0,
                    scopes: vec!["user:read:chat".into(), "bits:read".into()],
                })
                .unwrap(),
            )
            .unwrap();
            let router = Router::new().route(
                "/oauth2/token",
                post(|body: String| async move {
                    assert!(body.contains("grant_type=refresh_token"));
                    assert!(body.contains("refresh_token=old-refresh"));
                    Json(json!({
                        "access_token": "refreshed-access",
                        "refresh_token": "refreshed-refresh",
                        "expires_in": 3600,
                        "scope": ["user:read:chat", "bits:read"]
                    }))
                }),
            );
            let server = server(router).await;
            let auth = auth(
                &server,
                path.clone(),
                Arc::new(|_| panic!("no prompt expected")),
            );
            assert_eq!(auth.access_token(false).await.unwrap(), "refreshed-access");
            let saved: Token = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved.access_token, "refreshed-access");
            assert_eq!(saved.refresh_token, "refreshed-refresh");
        })
        .await
        .expect("refresh should finish within two seconds");
    }
}
