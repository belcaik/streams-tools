use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::{Manifest, ManifestError};

#[derive(Debug, Clone)]
pub struct Plugin {
    pub root: PathBuf,
    pub manifest: Manifest,
}

#[derive(Debug, Default)]
pub struct Registry {
    plugins: Vec<Plugin>,
    errors: Vec<RegistryError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryError {
    path: PathBuf,
    field: String,
    reason: String,
}

impl Plugin {
    pub fn overlay_dir(&self) -> Option<PathBuf> {
        let path = self.root.join("overlay");
        path.is_dir().then_some(path)
    }

    pub fn has_worker(&self) -> bool {
        self.manifest.worker.is_some()
    }
}

impl Registry {
    pub fn discover(root: &Path) -> io::Result<Self> {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let mut registry = Self::default();
        let mut directories = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => match entry.file_type() {
                    Ok(kind) if kind.is_dir() => directories.push(entry),
                    Ok(_) => {}
                    Err(error) => registry.errors.push(RegistryError::io(entry.path(), error)),
                },
                Err(error) => registry.errors.push(RegistryError::io(root, error)),
            }
        }
        directories.sort_by_key(|entry| entry.file_name());
        for entry in directories {
            let plugin_root = entry.path();
            let manifest_path = plugin_root.join("plugin.toml");
            match manifest_path.try_exists() {
                Ok(false) => continue,
                Err(error) => {
                    registry
                        .errors
                        .push(RegistryError::io(manifest_path, error));
                    continue;
                }
                Ok(true) => {}
            }
            match Manifest::load(&plugin_root) {
                Ok(manifest) => registry.plugins.push(Plugin {
                    root: plugin_root,
                    manifest,
                }),
                Err(error) => registry.errors.push(error.into()),
            }
        }
        Ok(registry)
    }

    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    pub fn errors(&self) -> &[RegistryError] {
        &self.errors
    }

    pub fn into_plugins(self) -> Vec<Plugin> {
        self.plugins
    }
}

impl RegistryError {
    fn io(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self {
            path: path.into(),
            field: "<file>".into(),
            reason: error.to_string(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl From<ManifestError> for RegistryError {
    fn from(error: ManifestError) -> Self {
        Self {
            path: error.path().into(),
            field: error.field().into(),
            reason: error.reason().into(),
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: campo {}: {}",
            self.path.display(),
            self.field,
            self.reason
        )
    }
}

impl Error for RegistryError {}
