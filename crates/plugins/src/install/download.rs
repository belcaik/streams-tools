use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::StreamExt;
use reqwest::{Client, redirect};
use sha2::{Digest, Sha256};

pub const CHECKSUM_LIMIT: usize = 1024 * 1024;
pub const ZIP_LIMIT: usize = 100 * 1024 * 1024;

#[derive(Debug)]
pub struct DownloadedPackage {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadError(String);

#[derive(Clone)]
pub struct Downloader {
    client: Client,
    allow_local: bool,
}

impl Downloader {
    pub fn new() -> Result<Self, DownloadError> {
        Self::build(false)
    }
    #[doc(hidden)]
    pub fn for_local_tests() -> Result<Self, DownloadError> {
        Self::build(true)
    }

    fn build(allow_local: bool) -> Result<Self, DownloadError> {
        let client = Client::builder()
            .user_agent(concat!("streams-tools/", env!("CARGO_PKG_VERSION")))
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
            .map_err(|error| DownloadError(error.to_string()))?;
        Ok(Self {
            client,
            allow_local,
        })
    }

    pub async fn fetch_verified(
        &self,
        checksums_url: &str,
        asset_url: &str,
        asset_name: &str,
        destination: &Path,
    ) -> Result<DownloadedPackage, DownloadError> {
        self.validate_url(checksums_url)?;
        self.validate_url(asset_url)?;
        let checksums = self.bytes(checksums_url, CHECKSUM_LIMIT).await?;
        let expected = parse_checksums(&checksums)?
            .remove(asset_name)
            .ok_or_else(|| DownloadError(format!("SHA256SUMS no contiene {asset_name}")))?;
        let response = self
            .client
            .get(asset_url)
            .send()
            .await
            .map_err(http)?
            .error_for_status()
            .map_err(http)?;
        if response
            .content_length()
            .is_some_and(|size| size > ZIP_LIMIT as u64)
        {
            return Err(DownloadError("ZIP supera 100 MiB".into()));
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(io)?;
        let mut sha = Sha256::new();
        let mut size = 0;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(http)?;
            size += chunk.len();
            if size > ZIP_LIMIT {
                return Err(DownloadError("ZIP supera 100 MiB".into()));
            }
            sha.update(&chunk);
            file.write_all(&chunk).map_err(io)?;
        }
        file.sync_all().map_err(io)?;
        let actual = format!("{:x}", sha.finalize());
        if !constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
            drop(file);
            let _ = std::fs::remove_file(destination);
            return Err(DownloadError("checksum SHA-256 no coincide".into()));
        }
        Ok(DownloadedPackage {
            path: destination.into(),
            sha256: actual,
        })
    }

    async fn bytes(&self, url: &str, limit: usize) -> Result<Vec<u8>, DownloadError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(http)?
            .error_for_status()
            .map_err(http)?;
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(DownloadError("respuesta supera el límite".into()));
        }
        let mut result = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(http)?;
            if result.len() + chunk.len() > limit {
                return Err(DownloadError("respuesta supera el límite".into()));
            }
            result.extend_from_slice(&chunk);
        }
        Ok(result)
    }

    fn validate_url(&self, url: &str) -> Result<(), DownloadError> {
        let local = url.starts_with("http://127.0.0.1:") || url.starts_with("http://[::1]:");
        if url.starts_with("https://") || self.allow_local && local {
            Ok(())
        } else {
            Err(DownloadError("descarga inicial no HTTPS".into()))
        }
    }
}

fn parse_checksums(bytes: &[u8]) -> Result<BTreeMap<String, String>, DownloadError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| DownloadError("SHA256SUMS no es ASCII".into()))?;
    if !text.is_ascii() {
        return Err(DownloadError("SHA256SUMS no es ASCII".into()));
    }
    let mut entries = BTreeMap::new();
    for line in text.lines() {
        let (sha, name) = line
            .split_once("  ")
            .ok_or_else(|| DownloadError("línea SHA256SUMS inválida".into()))?;
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || name.is_empty()
            || name.contains(char::is_whitespace)
            || entries.insert(name.into(), sha.into()).is_some()
        {
            return Err(DownloadError(
                "línea SHA256SUMS inválida o duplicada".into(),
            ));
        }
    }
    Ok(entries)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}
fn http(error: reqwest::Error) -> DownloadError {
    DownloadError(format!("descarga HTTP: {error}"))
}
fn io(error: std::io::Error) -> DownloadError {
    DownloadError(format!("archivo temporal: {error}"))
}
impl fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for DownloadError {}
