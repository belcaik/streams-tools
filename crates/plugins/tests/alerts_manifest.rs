use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use streams_tools_plugins::{AlertEvent, AlertManifest, PlatformName};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-alert-manifest-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temp dir should be created");
        Self(path)
    }

    fn alert(&self, folder: &str, source: &str) -> PathBuf {
        let root = self.0.join(folder);
        fs::create_dir(&root).expect("alert dir should be created");
        fs::write(root.join("alert.toml"), source).expect("manifest should be written");
        fs::write(root.join("style.css"), "css").expect("style should be written");
        root
    }

    fn asset(&self, alert: &Path, name: &str) {
        let path = alert.join(name);
        fs::create_dir_all(path.parent().expect("asset parent")).expect("asset dir");
        fs::write(path, "asset").expect("asset should be written");
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

fn error(path: &Path, source: &str) -> streams_tools_plugins::AlertLoadError {
    AlertManifest::load(path).expect_err(source)
}

#[test]
fn loads_complete_manifest() {
    let temp = TempDir::new();
    let root = temp.alert(
        "complete",
        r#"event = "donation"
template = "{user}: {amount} {currency} — {message}"
priority = 10
duration_ms = 6000
volume = 0.8
sound = "audio/donation.ogg"
[conditions]
currency = "USD"
min_amount = 5.0
max_amount = 100.0
platforms = ["twitch", "youtube"]
[[variations]]
template = "¡{user} aportó {amount} {currency}!"
image = "images/confetti.webp"
sound = "audio/big.ogg"
weight = 3
"#,
    );
    for asset in [
        "audio/donation.ogg",
        "audio/big.ogg",
        "images/confetti.webp",
    ] {
        temp.asset(&root, asset);
    }
    let manifest = AlertManifest::load(&root).expect("complete manifest should load");
    assert_eq!(manifest.event, AlertEvent::Donation);
    assert_eq!(manifest.priority, 10);
    assert_eq!(
        manifest.conditions.platforms,
        Some(vec![PlatformName::Twitch, PlatformName::Youtube])
    );
    assert_eq!(manifest.variations[0].weight, 3);
}

#[test]
fn loads_minimal_manifest_with_defaults() {
    let temp = TempDir::new();
    let root = temp.alert("minimal", &minimal("follow"));
    let manifest = AlertManifest::load(&root).expect("minimal manifest should load");
    assert_eq!(manifest.priority, 0);
    assert_eq!(manifest.duration_ms, 5000);
    assert_eq!(manifest.volume, 1.0);
    assert_eq!(manifest.variations, []);
}

#[test]
fn rejects_unknown_root_field_with_exact_path() {
    let temp = TempDir::new();
    let root = temp.alert("root", &(minimal("follow") + "unknown = true\n"));
    assert_eq!(error(&root, "root").field(), "unknown");
}

#[test]
fn rejects_unknown_conditions_field_with_exact_path() {
    let temp = TempDir::new();
    let root = temp.alert(
        "conditions",
        &(minimal("follow") + "[conditions]\nunknown = 1\n"),
    );
    assert_eq!(error(&root, "conditions").field(), "conditions.unknown");
}

#[test]
fn rejects_unknown_variation_field_with_exact_path() {
    let temp = TempDir::new();
    let source = minimal("follow") + "[[variations]]\ntemplate = \"hi\"\nunknown = 1\n";
    let root = temp.alert("variation", &source);
    assert_eq!(error(&root, "variation").field(), "variations[0].unknown");
}

#[test]
fn rejects_non_core_event() {
    let temp = TempDir::new();
    let root = temp.alert("event", &minimal("now_playing"));
    assert_eq!(error(&root, "event").field(), "event");
}

#[test]
fn rejects_inverted_amount_limits() {
    let temp = TempDir::new();
    let source = minimal("donation") + "[conditions]\nmin_amount = 10\nmax_amount = 5\n";
    let root = temp.alert("amounts", &source);
    assert_eq!(error(&root, "amounts").field(), "conditions.max_amount");
}

#[test]
fn rejects_non_finite_numbers() {
    let temp = TempDir::new();
    for (folder, suffix, field) in [
        (
            "nan",
            "[conditions]\nmin_amount = nan\n",
            "conditions.min_amount",
        ),
        (
            "inf",
            "[conditions]\nmax_amount = inf\n",
            "conditions.max_amount",
        ),
        ("volume", "volume = nan\n", "volume"),
    ] {
        let root = temp.alert(folder, &(minimal("donation") + suffix));
        assert_eq!(error(&root, folder).field(), field);
    }
}

#[test]
fn rejects_out_of_range_volume_and_duration() {
    let temp = TempDir::new();
    for (folder, suffix, field) in [
        ("volume-low", "volume = -0.1\n", "volume"),
        ("volume-high", "volume = 1.1\n", "volume"),
        ("duration-zero", "duration_ms = 0\n", "duration_ms"),
        ("duration-high", "duration_ms = 300001\n", "duration_ms"),
    ] {
        let root = temp.alert(folder, &(minimal("follow") + suffix));
        assert_eq!(error(&root, folder).field(), field);
    }
}

#[test]
fn rejects_conditions_for_the_wrong_event() {
    let temp = TempDir::new();
    for (index, condition) in [
        "currency = \"USD\"",
        "min_amount = 1",
        "max_amount = 2",
        "min_months = 1",
        "min_count = 1",
        "min_viewers = 1",
        "reward_title = \"Hydrate\"",
    ]
    .into_iter()
    .enumerate()
    {
        let folder = format!("condition-{index}");
        let source = minimal("follow") + "[conditions]\n" + condition + "\n";
        let root = temp.alert(&folder, &source);
        assert!(error(&root, &folder).field().starts_with("conditions."));
    }
}

#[test]
fn rejects_unknown_and_unbalanced_placeholders() {
    let temp = TempDir::new();
    for (folder, template) in [
        ("unknown", "{unknown}"),
        ("open", "{user"),
        ("close", "user}"),
        ("variation", "base"),
    ] {
        let mut source = minimal("follow");
        if folder == "variation" {
            source += "[[variations]]\ntemplate = \"{unknown}\"\n";
        } else {
            source = format!("event = \"follow\"\ntemplate = \"{template}\"\n");
        }
        let root = temp.alert(folder, &source);
        assert!(error(&root, folder).field().ends_with("template"));
    }
}

#[test]
fn rejects_variation_that_only_changes_weight() {
    let temp = TempDir::new();
    let root = temp.alert(
        "weight",
        &(minimal("follow") + "[[variations]]\nweight = 2\n"),
    );
    assert_eq!(error(&root, "weight").field(), "variations.0");
}

#[test]
fn rejects_missing_variation_image() {
    let temp = TempDir::new();
    let source = minimal("follow") + "[[variations]]\nimage = \"images/missing.webp\"\n";
    let root = temp.alert("image", &source);
    assert_eq!(error(&root, "image").field(), "variations.0.image");
}

#[test]
fn rejects_parent_component_in_asset() {
    let temp = TempDir::new();
    let root = temp.alert(
        "traversal",
        &(minimal("follow") + "sound = \"../outside.ogg\"\n"),
    );
    assert_eq!(error(&root, "traversal").field(), "sound");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_that_escapes_alert_directory() {
    let temp = TempDir::new();
    let outside = temp.0.join("outside.ogg");
    fs::write(&outside, "audio").expect("outside asset");
    let root = temp.alert(
        "symlink",
        &(minimal("follow") + "sound = \"audio/escape.ogg\"\n"),
    );
    fs::create_dir(root.join("audio")).expect("audio dir");
    std::os::unix::fs::symlink(&outside, root.join("audio/escape.ogg")).expect("symlink");
    assert_eq!(error(&root, "symlink").field(), "sound");
}

#[test]
fn toml_errors_use_a_single_line_reason() {
    let dir = TempDir::new();
    let alert = dir.alert("rota", "event = \"now_playing\"\ntemplate = \"x\"\n");

    let error = AlertManifest::load(&alert).expect_err("unknown event should fail");

    assert_eq!(error.field(), "event");
    assert!(!error.reason().contains('\n'), "{:?}", error.reason());
    assert!(error.reason().contains("now_playing"));
}
