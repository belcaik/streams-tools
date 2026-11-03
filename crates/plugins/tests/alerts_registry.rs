use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::json;
use streams_tools_plugins::AlertRegistry;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-alert-registry-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temp dir should be created");
        Self(path)
    }

    fn alert(&self, pack: &str, alert: &str, source: &str) -> PathBuf {
        let root = self.0.join(pack).join(alert);
        fs::create_dir_all(&root).expect("alert dir should be created");
        fs::write(root.join("alert.toml"), source).expect("manifest should be written");
        fs::write(root.join("style.css"), "css").expect("style should be written");
        root
    }

    fn registry(&self) -> AlertRegistry {
        AlertRegistry::discover(&self.0).expect("registry should load")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn minimal(event: &str) -> String {
    format!("event = \"{event}\"\ntemplate = \"{{user}}\"\n")
}

#[test]
fn discovers_packs_and_alerts_in_stable_name_order() {
    let temp = TempDir::new();
    temp.alert("z-pack", "z-alert", &minimal("follow"));
    temp.alert("a-pack", "z-alert", &minimal("follow"));
    temp.alert("a-pack", "a-alert", &minimal("chat"));
    let registry = temp.registry();
    let pack_names: Vec<_> = registry
        .index()
        .packs
        .iter()
        .map(|pack| pack.name.as_str())
        .collect();
    let alert_names: Vec<_> = registry.index().packs[0]
        .alerts
        .iter()
        .map(|alert| alert.name.as_str())
        .collect();
    assert_eq!(pack_names, ["a-pack", "z-pack"]);
    assert_eq!(alert_names, ["a-alert", "z-alert"]);
}

#[test]
fn missing_root_produces_an_empty_catalog() {
    let temp = TempDir::new();
    let missing = temp.0.join("missing");
    let registry = AlertRegistry::discover(&missing).expect("missing root should be empty");
    assert_eq!(registry.index_json(), json!({"packs": [], "errors": []}));
    assert_eq!(registry.asset_roots().count(), 0);
}

#[test]
fn ignores_loose_files_and_alert_folders_without_manifest() {
    let temp = TempDir::new();
    fs::write(temp.0.join("loose.txt"), "ignored").expect("loose file");
    fs::create_dir_all(temp.0.join("pack/empty-alert")).expect("empty alert");
    fs::create_dir(temp.0.join("pack/nested-file")).expect("nested file dir");
    let registry = temp.registry();
    assert_eq!(registry.index().packs[0].alerts, []);
    assert_eq!(registry.index().errors, []);
}

#[test]
fn reports_invalid_pack_and_alert_names() {
    let temp = TempDir::new();
    temp.alert("Bad-Pack", "valid", &minimal("follow"));
    temp.alert("valid-pack", "Bad Alert", &minimal("follow"));
    let registry = temp.registry();
    let errors = registry.errors();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].pack, "Bad-Pack");
    assert_eq!(errors[0].alert, None);
    assert_eq!(errors[1].file, "valid-pack/Bad Alert/alert.toml");
    assert_eq!(errors[1].alert.as_deref(), Some("Bad Alert"));
}

#[test]
fn isolates_a_broken_alert_from_valid_alerts() {
    let temp = TempDir::new();
    temp.alert("a-pack", "bad", "event = \"follow\"\nunknown = 1\n");
    temp.alert("a-pack", "good", &minimal("sub"));
    temp.alert("b-pack", "good", &minimal("raid"));
    let registry = temp.registry();
    assert_eq!(registry.valid_alert_count(), 2);
    assert_eq!(registry.errors().len(), 1);
    assert_eq!(registry.errors()[0].file, "a-pack/bad/alert.toml");
}

#[test]
fn builds_exact_http_paths_and_relative_error_files() {
    let temp = TempDir::new();
    let root = temp.alert(
        "neon",
        "big-donation",
        r#"event = "donation"
template = "{user}: {amount} {currency}"
priority = 10
duration_ms = 6000
volume = 0.8
sound = "audio/donation.ogg"
[conditions]
currency = "USD"
min_amount = 5.0
[[variations]]
image = "images/confetti.webp"
weight = 3
"#,
    );
    fs::create_dir_all(root.join("audio")).expect("audio dir");
    fs::write(root.join("audio/donation.ogg"), "audio").expect("audio asset");
    fs::create_dir(root.join("images")).expect("images dir");
    fs::write(root.join("images/confetti.webp"), "image").expect("image asset");
    temp.alert(
        "broken-pack",
        "bad-alert",
        "event = \"follow\"\n[conditions]\nmin_amount = 1\n",
    );
    let registry = temp.registry();
    let alert = &registry.index().packs[1].alerts[0];
    assert_eq!(alert.name, "big-donation");
    assert_eq!(
        alert.sound.as_deref(),
        Some("/alerts/neon/big-donation/audio/donation.ogg")
    );
    assert_eq!(alert.css, "/alerts/neon/big-donation/style.css");
    assert_eq!(
        alert.conditions,
        json!({"currency": "USD", "min_amount": 5.0})
    );
    assert_eq!(
        alert.variations[0].image.as_deref(),
        Some("/alerts/neon/big-donation/images/confetti.webp")
    );
    assert_eq!(
        registry.errors()[0].file,
        "broken-pack/bad-alert/alert.toml"
    );
}

#[test]
fn detects_optional_html_and_javascript() {
    let temp = TempDir::new();
    let root = temp.alert("pack", "alert", &minimal("follow"));
    fs::write(root.join("index.html"), "html").expect("html asset");
    fs::write(root.join("script.js"), "js").expect("js asset");
    let registry = temp.registry();
    let alert = &registry.index().packs[0].alerts[0];
    assert_eq!(alert.html.as_deref(), Some("/alerts/pack/alert/index.html"));
    assert_eq!(alert.js.as_deref(), Some("/alerts/pack/alert/script.js"));
    assert_eq!(alert.sound, None);
}

#[test]
fn preserves_a_valid_pack_with_no_valid_alerts() {
    let temp = TempDir::new();
    temp.alert(
        "empty-pack",
        "broken",
        "event = \"invalid\"\ntemplate = \"x\"\n",
    );
    let registry = temp.registry();
    assert_eq!(registry.index().packs.len(), 1);
    assert_eq!(registry.index().packs[0].name, "empty-pack");
    assert_eq!(registry.index().packs[0].alerts, []);
    assert_eq!(registry.asset_roots().count(), 1);
    assert_eq!(registry.errors().len(), 1);
}
