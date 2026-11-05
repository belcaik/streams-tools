use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::{Manifest, ManifestError};

#[derive(Debug, Clone)]
pub struct Plugin {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub effective_config: serde_json::Value,
}

#[derive(Debug, Default)]
pub struct Registry {
    plugins: Vec<Plugin>,
    errors: Vec<RegistryError>,
    warnings: Vec<String>,
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
        Self::discover_with_config(root, &BTreeMap::new())
    }

    pub fn discover_with_config(
        root: &Path,
        overrides: &BTreeMap<String, toml::Table>,
    ) -> io::Result<Self> {
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
                Ok(manifest) => match effective_config(&manifest, overrides.get(&manifest.name)) {
                    Ok(effective_config) => registry.plugins.push(Plugin {
                        root: plugin_root,
                        manifest,
                        effective_config,
                    }),
                    Err((field, reason)) => registry.errors.push(RegistryError {
                        path: manifest_path,
                        field,
                        reason,
                    }),
                },
                Err(error) => registry.errors.push(error.into()),
            }
        }
        for name in overrides.keys() {
            if !registry
                .plugins
                .iter()
                .any(|plugin| &plugin.manifest.name == name)
            {
                registry.warnings.push(format!(
                    "plugins.config.{name}: no hay un plugin instalado con ese nombre"
                ));
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

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn into_plugins(self) -> Vec<Plugin> {
        self.plugins
    }
}

fn effective_config(
    manifest: &Manifest,
    overrides: Option<&toml::Table>,
) -> Result<serde_json::Value, (String, String)> {
    let mut effective = manifest.config.clone();
    if let Some(overrides) = overrides {
        merge_table(
            &mut effective,
            overrides,
            &format!("plugins.config.{}", manifest.name),
        )?;
    }
    serde_json::to_value(effective).map_err(|error| {
        (
            format!("plugins.config.{}", manifest.name),
            error.to_string(),
        )
    })
}

fn merge_table(
    defaults: &mut toml::Table,
    overrides: &toml::Table,
    path: &str,
) -> Result<(), (String, String)> {
    for (key, value) in overrides {
        let field = format!("{path}.{key}");
        let default = defaults
            .get_mut(key)
            .ok_or_else(|| (field.clone(), "clave desconocida".into()))?;
        match (default, value) {
            (toml::Value::Table(default), toml::Value::Table(value)) => {
                merge_table(default, value, &field)?
            }
            (default, value) if same_category(default, value) => *default = value.clone(),
            _ => return Err((field, "tipo incompatible con el valor por defecto".into())),
        }
    }
    Ok(())
}

fn same_category(left: &toml::Value, right: &toml::Value) -> bool {
    matches!(
        (left, right),
        (toml::Value::String(_), toml::Value::String(_))
            | (toml::Value::Integer(_), toml::Value::Integer(_))
            | (toml::Value::Float(_), toml::Value::Float(_))
            | (toml::Value::Boolean(_), toml::Value::Boolean(_))
            | (toml::Value::Array(_), toml::Value::Array(_))
    )
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
