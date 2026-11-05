use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::PathBuf,
};

use super::{
    Downloader, FsTransaction, GithubClient, InstalledMetadata, MetadataRecord, MetadataStore,
    PluginSource, Portability, RequestedRelease, Staging, validate_package,
};

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub plugins_root: PathBuf,
    pub target: String,
}

pub struct Installer {
    options: InstallOptions,
    github: GithubClient,
    downloader: Downloader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub name: String,
    pub changed: bool,
    pub warning: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub source: String,
    pub stage: &'static str,
    pub reason: String,
}

impl Installer {
    pub fn new(options: InstallOptions) -> Result<Self, InstallError> {
        let source = "<configuración>";
        let github =
            GithubClient::new().map_err(|error| InstallError::new(source, "cliente", error))?;
        let downloader =
            Downloader::new().map_err(|error| InstallError::new(source, "cliente", error))?;
        Ok(Self {
            options,
            github,
            downloader,
        })
    }

    #[doc(hidden)]
    pub fn for_local_tests(options: InstallOptions, api_base: &str) -> Result<Self, InstallError> {
        let source = "<test>";
        let github = GithubClient::with_api_base(api_base)
            .map_err(|error| InstallError::new(source, "cliente", error))?;
        let downloader = Downloader::for_local_tests()
            .map_err(|error| InstallError::new(source, "cliente", error))?;
        Ok(Self {
            options,
            github,
            downloader,
        })
    }

    pub async fn add(&self, source: &str) -> Result<InstallOutcome, InstallError> {
        self.install(source, false).await
    }

    pub async fn update(&self, name: Option<&str>) -> Vec<Result<InstallOutcome, InstallError>> {
        let store = MetadataStore::new(&self.options.plugins_root);
        let records = match store.list() {
            Ok(records) => records,
            Err(error) => return vec![Err(InstallError::new("<metadata>", "listar", error))],
        };
        let mut managed: Vec<_> = records
            .into_iter()
            .filter_map(|record| match record {
                MetadataRecord::Managed(metadata)
                    if name.is_none_or(|wanted| wanted == metadata.name) =>
                {
                    Some(metadata)
                }
                _ => None,
            })
            .collect();
        managed.sort_by(|left, right| left.name.cmp(&right.name));
        if name.is_some() && managed.is_empty() {
            return vec![Err(InstallError::new(
                name.unwrap_or_default(),
                "actualizar",
                "plugin no administrado",
            ))];
        }
        let mut results = Vec::new();
        for metadata in managed {
            results.push(self.install(&metadata.source, true).await);
        }
        results
    }

    pub async fn ensure_missing<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a str>,
    ) -> Vec<Result<InstallOutcome, InstallError>> {
        let installed: BTreeSet<_> = MetadataStore::new(&self.options.plugins_root)
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| match record {
                MetadataRecord::Managed(value) => Some(value.source),
                _ => None,
            })
            .collect();
        let mut normalized = BTreeMap::new();
        let mut errors = Vec::new();
        for raw in sources {
            match PluginSource::parse(raw) {
                Ok(source) => {
                    normalized.entry(source.to_string()).or_insert(source);
                }
                Err(error) => errors.push(Err(InstallError::new(raw, "fuente", error))),
            }
        }
        for (canonical, _) in normalized {
            if !installed.contains(&canonical) {
                errors.push(self.add(&canonical).await);
            }
        }
        errors
    }

    async fn install(
        &self,
        raw_source: &str,
        updating: bool,
    ) -> Result<InstallOutcome, InstallError> {
        let source = PluginSource::parse(raw_source)
            .map_err(|error| InstallError::new(raw_source, "fuente", error))?;
        let release = self
            .github
            .release(&source)
            .await
            .map_err(|error| InstallError::new(raw_source, "release", error))?;
        let asset = self
            .github
            .release_asset(&release, &source, &self.options.target)
            .map_err(|error| InstallError::new(raw_source, "asset", error))?;
        let checksums = self
            .github
            .checksum_asset(&release, &source)
            .map_err(|error| InstallError::new(raw_source, "checksum", error))?;
        let staging = Staging::create(&self.options.plugins_root)
            .map_err(|error| InstallError::new(raw_source, "staging", error))?;
        let downloaded = self
            .downloader
            .fetch_verified(
                &checksums.url,
                &asset.url,
                &asset.name,
                &staging.archive_path(),
            )
            .await
            .map_err(|error| InstallError::new(raw_source, "descarga", error))?;
        let package = validate_package(
            staging,
            downloaded,
            &asset.name,
            &release.tag,
            &self.options.target,
        )
        .map_err(|error| InstallError::new(raw_source, "validación", error))?;
        let requested = match source.requested() {
            RequestedRelease::Latest => "latest",
            RequestedRelease::Tag(tag) => tag,
        }
        .to_owned();
        let target = if package.portability == Portability::Any {
            "any"
        } else {
            &self.options.target
        }
        .to_owned();
        let metadata = InstalledMetadata {
            schema: 1,
            name: package.name.clone(),
            version: package.version.to_string(),
            source: source.to_string(),
            requested,
            tag: release.tag.clone(),
            asset: asset.name.clone(),
            target,
            sha256: package.sha256.clone(),
        };
        let warning = format!(
            "ADVERTENCIA: instalar {}/{} permite que streams-tools ejecute su worker y sirva su overlay; instala sólo código de confianza.",
            source.owner(),
            source.repo()
        );
        let transaction = FsTransaction::new(&self.options.plugins_root)
            .map_err(|error| InstallError::new(raw_source, "publicación", error))?;
        if updating {
            if MetadataStore::new(&self.options.plugins_root)
                .read(&package.name)
                .ok()
                .flatten()
                .is_some_and(|old| old.tag == metadata.tag && old.sha256 == metadata.sha256)
            {
                return Ok(InstallOutcome {
                    name: package.name,
                    changed: false,
                    warning: None,
                    detail: format!("ya está en {}", metadata.tag),
                });
            }
            transaction
                .update(&package, &metadata)
                .map_err(|error| InstallError::new(raw_source, "actualización", error))?;
        } else {
            transaction
                .publish(&package, &metadata)
                .map_err(|error| InstallError::new(raw_source, "instalación", error))?;
        }
        Ok(InstallOutcome {
            name: metadata.name,
            changed: true,
            warning: Some(warning),
            detail: metadata.tag,
        })
    }
}

impl InstallError {
    fn new(source: impl Into<String>, stage: &'static str, error: impl fmt::Display) -> Self {
        Self {
            source: source.into(),
            stage,
            reason: error.to_string(),
        }
    }
}
impl fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}: {}",
            self.source, self.stage, self.reason
        )
    }
}
impl Error for InstallError {}
