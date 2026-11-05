use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use streams_tools_plugins::Registry;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-registry-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temp dir should be created");
        Self(path)
    }

    fn plugin(&self, name: &str, source: Option<&str>) -> PathBuf {
        let root = self.0.join(name);
        fs::create_dir(&root).expect("plugin dir should be created");
        if let Some(source) = source {
            fs::write(root.join("plugin.toml"), source).expect("manifest should be written");
        }
        root
    }
}

#[test]
fn deep_merges_config_without_mutating_manifest_defaults() {
    let temp = TempDir::new();
    temp.plugin("demo", Some("name='demo'\nversion='1'\n[permissions]\nlistens=[]\nemits=[]\n[config]\ntags=['a']\nvolume=1.0\n[config.engine]\nkind='default'\nrate=1"));
    let overrides: BTreeMap<String, toml::Table> =
        toml::from_str("[demo]\ntags=['b','c']\nvolume=0.5\n[demo.engine]\nkind='custom'").unwrap();
    let registry = Registry::discover_with_config(&temp.0, &overrides).unwrap();
    let plugin = &registry.plugins()[0];
    assert_eq!(
        plugin.effective_config["tags"],
        serde_json::json!(["b", "c"])
    );
    assert_eq!(
        plugin.effective_config["engine"],
        serde_json::json!({"kind":"custom","rate":1})
    );
    assert_eq!(
        plugin.manifest.config["engine"]["kind"].as_str(),
        Some("default")
    );
}

#[test]
fn rejects_unknown_and_mismatched_overrides_and_warns_for_missing_plugin() {
    for source in [
        "[demo]\nunknown=true",
        "[demo.engine]\nkind=1",
        "[demo]\nvolume=1",
    ] {
        let temp = TempDir::new();
        temp.plugin("demo", Some("name='demo'\nversion='1'\n[permissions]\nlistens=[]\nemits=[]\n[config]\nvolume=1.0\n[config.engine]\nkind='default'"));
        let overrides: BTreeMap<String, toml::Table> = toml::from_str(source).unwrap();
        let registry = Registry::discover_with_config(&temp.0, &overrides).unwrap();
        assert!(registry.plugins().is_empty());
        assert!(
            registry.errors()[0]
                .field()
                .starts_with("plugins.config.demo")
        );
    }
    let temp = TempDir::new();
    let overrides: BTreeMap<String, toml::Table> =
        toml::from_str("[future]\nenabled=true").unwrap();
    let registry = Registry::discover_with_config(&temp.0, &overrides).unwrap();
    assert!(registry.errors().is_empty());
    assert!(registry.warnings()[0].contains("plugins.config.future"));
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn valid(name: &str) -> String {
    format!("name = \"{name}\"\nversion = \"1\"\n[permissions]\nlistens = []\nemits = []\n")
}

fn discover(root: &Path) -> Registry {
    Registry::discover(root).expect("registry root should be readable")
}

#[test]
fn loads_valid_plugins_in_stable_order() {
    let temp = TempDir::new();
    temp.plugin("zeta", Some(&valid("zeta")));
    temp.plugin("alpha", Some(&valid("alpha")));
    let registry = discover(&temp.0);
    let names: Vec<_> = registry
        .plugins()
        .iter()
        .map(|plugin| plugin.manifest.name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
    assert!(registry.errors().is_empty());
}

#[test]
fn ignores_folders_without_manifest_and_loose_files() {
    let temp = TempDir::new();
    temp.plugin("readme", None);
    fs::write(temp.0.join("readme/README.md"), "docs").expect("README should be written");
    fs::write(temp.0.join("loose"), valid("loose")).expect("loose file should be written");
    temp.plugin("valid", Some(&valid("valid")));
    let registry = discover(&temp.0);
    assert_eq!(registry.plugins().len(), 1);
    assert!(registry.errors().is_empty());
}

#[test]
fn isolates_broken_manifest_from_valid_plugin() {
    let temp = TempDir::new();
    let broken = temp.plugin("broken", Some("not valid TOML"));
    temp.plugin("valid", Some(&valid("valid")));
    let registry = discover(&temp.0);
    assert_eq!(registry.plugins().len(), 1);
    assert_eq!(registry.errors().len(), 1);
    let message = registry.errors()[0].to_string();
    assert!(message.contains(broken.join("plugin.toml").to_str().expect("UTF-8 path")));
    assert!(message.contains("campo"));
}

#[test]
fn missing_root_produces_empty_registry() {
    let temp = TempDir::new();
    let registry = discover(&temp.0.join("missing"));
    assert!(registry.plugins().is_empty());
    assert!(registry.errors().is_empty());
}

#[test]
fn overlay_dir_requires_a_directory() {
    let temp = TempDir::new();
    for (name, overlay, expected) in [
        ("dir", Some(true), true),
        ("file", Some(false), false),
        ("none", None, false),
    ] {
        let root = temp.plugin(name, Some(&valid(name)));
        if let Some(directory) = overlay {
            if directory {
                fs::create_dir(root.join("overlay")).expect("overlay dir should be created");
            } else {
                fs::write(root.join("overlay"), "not a dir")
                    .expect("overlay file should be written");
            }
        }
        let registry = discover(&temp.0);
        let plugin = registry
            .plugins()
            .iter()
            .find(|plugin| plugin.manifest.name == name)
            .expect("plugin");
        assert_eq!(plugin.overlay_dir().is_some(), expected);
    }
}
