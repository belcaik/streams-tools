use std::{error::Error, fmt, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, redirect};
use serde::Deserialize;

use super::{PluginSource, RequestedRelease};

const JSON_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Release {
    #[serde(rename = "tag_name")]
    pub tag: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubError(String);

#[derive(Clone)]
pub struct GithubClient {
    client: Client,
    api_base: String,
}

impl GithubClient {
    pub fn new() -> Result<Self, GithubError> {
        Self::with_api_base("https://api.github.com")
    }

    #[doc(hidden)]
    pub fn with_api_base(api_base: &str) -> Result<Self, GithubError> {
        let local =
            api_base.starts_with("http://127.0.0.1:") || api_base.starts_with("http://[::1]:");
        if !api_base.starts_with("https://") && !local {
            return Err(GithubError(
                "la base API de prueba debe ser HTTPS o localhost".into(),
            ));
        }
        let client = Client::builder()
            .user_agent(concat!("streams-tools/", env!("CARGO_PKG_VERSION")))
            .default_headers(
                [(
                    reqwest::header::ACCEPT,
                    "application/vnd.github+json"
                        .parse()
                        .expect("static header"),
                )]
                .into_iter()
                .collect(),
            )
            .timeout(Duration::from_secs(30))
            .redirect(redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("demasiados redirects")
                } else if attempt.url().scheme() != "https" {
                    attempt.error("redirect no HTTPS")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|error| GithubError(error.to_string()))?;
        Ok(Self {
            client,
            api_base: api_base.trim_end_matches('/').into(),
        })
    }

    pub async fn release(&self, source: &PluginSource) -> Result<Release, GithubError> {
        let endpoint = match source.requested() {
            RequestedRelease::Latest => "releases/latest".into(),
            RequestedRelease::Tag(tag) => format!("releases/tags/{tag}"),
        };
        let url = format!(
            "{}/repos/{}/{}/{}",
            self.api_base,
            source.owner(),
            source.repo(),
            endpoint
        );
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| GithubError(format!("GitHub API: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let rate = status == StatusCode::FORBIDDEN
                && response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|value| value == "0");
            return Err(GithubError(if rate {
                "GitHub API: límite de tasa agotado".into()
            } else {
                format!("GitHub API respondió {status}")
            }));
        }
        if response
            .content_length()
            .is_some_and(|length| length > JSON_LIMIT as u64)
        {
            return Err(GithubError("respuesta JSON supera 1 MiB".into()));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|error| GithubError(format!("respuesta truncada: {error}")))?;
            if bytes.len() + chunk.len() > JSON_LIMIT {
                return Err(GithubError("respuesta JSON supera 1 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let release: Release = serde_json::from_slice(&bytes)
            .map_err(|error| GithubError(format!("JSON de release inválido: {error}")))?;
        if release.draft
            || matches!(source.requested(), RequestedRelease::Latest) && release.prerelease
        {
            return Err(GithubError(
                "el release solicitado es draft o prerelease".into(),
            ));
        }
        Ok(release)
    }
}

impl Release {
    pub fn select_asset<'a>(
        &'a self,
        source: &PluginSource,
        target: &str,
    ) -> Result<&'a Asset, GithubError> {
        let specific = format!("-{target}.zip");
        let mut candidates: Vec<_> = self
            .assets
            .iter()
            .filter(|asset| asset.name.ends_with(&specific))
            .collect();
        if candidates.is_empty() {
            candidates = self
                .assets
                .iter()
                .filter(|asset| asset.name.ends_with("-any.zip"))
                .collect();
        }
        if candidates.len() != 1 {
            return Err(GithubError(format!(
                "se esperó un asset para {target} o any; encontrados {}",
                candidates.len()
            )));
        }
        let asset = candidates[0];
        let expected = format!(
            "https://github.com/{}/{}/releases/download/{}/{}",
            source.owner(),
            source.repo(),
            self.tag,
            asset.name
        );
        if asset.url != expected {
            return Err(GithubError(
                "browser_download_url no pertenece al release".into(),
            ));
        }
        Ok(asset)
    }
}

impl fmt::Display for GithubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for GithubError {}
