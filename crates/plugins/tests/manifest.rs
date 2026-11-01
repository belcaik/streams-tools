use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use streams_tools_plugins::{Manifest, PluginOs};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-manifest-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temp dir should be created");
        Self(path)
    }

    fn plugin(&self, folder: &str, source: &str) -> PathBuf {
        let root = self.0.join(folder);
        fs::create_dir(&root).expect("plugin dir should be created");
        fs::write(root.join("plugin.toml"), source).expect("manifest should be written");
        root
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn minimal(name: &str) -> String {
    format!("name = \"{name}\"\nversion = \"1\"\n[permissions]\nlistens = []\nemits = []\n")
}

fn load(path: &Path) -> Manifest {
    Manifest::load(path).expect("manifest should load")
}

#[test]
fn loads_complete_manifest_and_json_config() {
    let temp = TempDir::new();
    let root = temp.plugin(
        "example",
        r#"name = "example"
version = "0.1.0"
description = "Example"
[permissions]
listens = ["chat"]
emits = ["pong"]
[worker]
command = ["run"]
command_windows = ["run.exe", "arg"]
command_macos = ["mac"]
command_linux = ["linux"]
[config]
title = "Echo"
[config.nested]
enabled = true
"#,
    );
    let manifest = load(&root);
    let worker = manifest.worker.as_ref().expect("worker");
    assert_eq!(manifest.description.as_deref(), Some("Example"));
    assert_eq!(worker.command_linux.as_deref(), Some(&["linux".into()][..]));
    assert_eq!(manifest.config_json()["nested"]["enabled"], true);
}

#[test]
fn loads_minimal_manifest() {
    let temp = TempDir::new();
    let manifest = load(&temp.plugin("minimal", &minimal("minimal")));
    assert!(manifest.worker.is_none());
    assert_eq!(manifest.config_json(), serde_json::json!({}));
}

#[test]
fn rejects_folder_mismatch_and_invalid_name_with_context() {
    let temp = TempDir::new();
    for (folder, name) in [("actual", "other"), ("bad", "Bad")] {
        let root = temp.plugin(folder, &minimal(name));
        let error = Manifest::load(&root)
            .expect_err("name should be rejected")
            .to_string();
        assert!(error.contains(root.join("plugin.toml").to_str().expect("UTF-8 path")));
        assert!(error.contains("campo name") && error.contains(name));
    }
}

#[test]
fn rejects_unknown_fields_at_root_and_worker() {
    let temp = TempDir::new();
    let root =
        "name = \"root\"\nunknown = 1\nversion = \"1\"\n[permissions]\nlistens = []\nemits = []\n";
    let worker = minimal("worker") + "[worker]\ncommand = [\"run\"]\nunknown = 1\n";
    for (folder, source, field) in [
        ("root", root, "unknown"),
        ("worker", &worker, "worker.unknown"),
    ] {
        let error =
            Manifest::load(&temp.plugin(folder, source)).expect_err("unknown field should fail");
        assert_eq!(error.field(), field);
    }
}

#[test]
fn rejects_empty_commands_and_arguments() {
    let temp = TempDir::new();
    for (folder, command) in [("empty", "[]"), ("argument", "[\"run\", \"\"]")] {
        let source = minimal(folder) + &format!("[worker]\ncommand = {command}\n");
        let error = Manifest::load(&temp.plugin(folder, &source)).expect_err("command should fail");
        assert_eq!(error.field(), "worker.command");
    }
}

#[test]
fn selects_platform_commands_with_fallback() {
    let worker = streams_tools_plugins::Worker {
        command: vec!["default".into()],
        command_windows: Some(vec!["windows".into()]),
        command_macos: Some(vec!["macos".into()]),
        command_linux: None,
    };
    assert_eq!(worker.command_for(PluginOs::Windows), ["windows"]);
    assert_eq!(worker.command_for(PluginOs::Macos), ["macos"]);
    assert_eq!(worker.command_for(PluginOs::Linux), ["default"]);
    assert_eq!(worker.command_for(PluginOs::Other), ["default"]);
}

#[test]
fn validates_wildcards_and_duplicate_permissions() {
    let temp = TempDir::new();
    for (index, listens, emits, valid) in [
        (0, "[\"*\"]", "[]", true),
        (1, "[\"*\", \"chat\"]", "[]", false),
        (2, "[]", "[\"*\"]", false),
        (3, "[\"chat\", \"chat\"]", "[]", false),
    ] {
        let folder = format!("case-{index}");
        let source = format!(
            "name = \"{folder}\"\nversion = \"1\"\n[permissions]\nlistens = {listens}\nemits = {emits}\n"
        );
        let result = Manifest::load(&temp.plugin(&folder, &source));
        assert_eq!(result.is_ok(), valid, "case {index}: {result:?}");
    }
}
