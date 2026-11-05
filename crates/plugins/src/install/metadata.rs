use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use semver::Version;
use serde::{Deserialize, Serialize};

use super::PluginSource;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledMetadata {
    pub schema: u32,
    pub name: String,
    pub version: String,
    pub source: String,
    pub requested: String,
    pub tag: String,
    pub asset: String,
    pub target: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataRecord {
    Managed(InstalledMetadata),
    Broken { name: String, reason: String },
}

#[derive(Debug)]
pub struct MetadataError(String);

pub struct MetadataStore {
    root: PathBuf,
}

impl MetadataStore {
    pub fn new(plugins_root: &Path) -> Self {
        Self {
            root: plugins_root.join(".streams-tools/installed"),
        }
    }
    pub fn path(&self, name: &str) -> Result<PathBuf, MetadataError> {
        validate_name(name)?;
        Ok(self.root.join(format!("{name}.toml")))
    }
    pub fn read(&self, name: &str) -> Result<Option<InstalledMetadata>, MetadataError> {
        let path = self.path(name)?;
        let source = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(MetadataError(error.to_string())),
        };
        let metadata: InstalledMetadata = toml::from_str(&source)
            .map_err(|error| MetadataError(format!("{}: {error}", path.display())))?;
        metadata.validate(Some(name))?;
        Ok(Some(metadata))
    }
    pub fn list(&self) -> Result<Vec<MetadataRecord>, MetadataError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(MetadataError(error.to_string())),
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
            .collect();
        paths.sort();
        Ok(paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("<inválido>")
                    .to_owned();
                match fs::read_to_string(&path)
                    .map_err(|error| error.to_string())
                    .and_then(|source| {
                        toml::from_str::<InstalledMetadata>(&source)
                            .map_err(|error| error.to_string())
                    })
                    .and_then(|metadata| {
                        metadata
                            .validate(Some(&name))
                            .map(|_| metadata)
                            .map_err(|error| error.to_string())
                    }) {
                    Ok(metadata) => MetadataRecord::Managed(metadata),
                    Err(reason) => MetadataRecord::Broken { name, reason },
                }
            })
            .collect())
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}

impl InstalledMetadata {
    pub fn validate(&self, expected_name: Option<&str>) -> Result<(), MetadataError> {
        if self.schema != 1 {
            return Err(MetadataError("schema de metadata no soportado".into()));
        }
        validate_name(&self.name)?;
        if expected_name.is_some_and(|name| name != self.name) {
            return Err(MetadataError(
                "nombre de metadata no coincide con archivo".into(),
            ));
        }
        PluginSource::parse(&self.source)
            .map_err(|error| MetadataError(format!("source inválida: {error}")))?;
        Version::parse(&self.version)
            .map_err(|error| MetadataError(format!("version inválida: {error}")))?;
        let tag = self
            .tag
            .strip_prefix('v')
            .ok_or_else(|| MetadataError("tag inválido".into()))?;
        if Version::parse(tag)
            .map_err(|error| MetadataError(format!("tag inválido: {error}")))?
            .to_string()
            != self.version
        {
            return Err(MetadataError("tag y version no coinciden".into()));
        }
        if self.requested != "latest" && self.requested != self.tag {
            return Err(MetadataError("requested inválido".into()));
        }
        if self.asset.is_empty() || self.asset.contains(['/', '\\']) {
            return Err(MetadataError("asset inválido".into()));
        }
        if self.target != "any"
            && !matches!(
                self.target.as_str(),
                "x86_64-unknown-linux-gnu"
                    | "aarch64-unknown-linux-gnu"
                    | "x86_64-pc-windows-msvc"
                    | "x86_64-apple-darwin"
                    | "aarch64-apple-darwin"
            )
        {
            return Err(MetadataError("target inválido".into()));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(MetadataError("sha256 inválido".into()));
        }
        Ok(())
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), MetadataError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Err(MetadataError("nombre de plugin inválido".into()))
    } else {
        Ok(())
    }
}
impl fmt::Display for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for MetadataError {}
