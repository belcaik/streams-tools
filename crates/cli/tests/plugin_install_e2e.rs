mod support;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use streams_tools_plugins::install::{InstallOptions, Installer};
use support::{
    http::Server,
    releases::{self, TARGET},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-e2e-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn installer(temp: &Temp, server: &Server) -> Installer {
    Installer::for_local_tests(
        InstallOptions {
            plugins_root: temp.0.join("plugins"),
            target: TARGET.into(),
        },
        server.base(),
    )
    .unwrap()
}

#[tokio::test]
async fn installs_target_any_and_pinned_then_updates_lists_and_removes() {
    let temp = Temp::new();
    let server = Server::new();
    releases::publish(
        &server,
        "acme",
        "target",
        "v1.0.0",
        &format!("target-{TARGET}.zip"),
        releases::zip("target", "1.0.0", true, &[]),
        true,
    );
    releases::publish(
        &server,
        "acme",
        "portable",
        "v1.0.0",
        "portable-any.zip",
        releases::zip(
            "portable",
            "1.0.0",
            false,
            &[("portable/overlay/index.html", b"ok")],
        ),
        true,
    );
    let installer = installer(&temp, &server);
    installer
        .add("https://github.com/acme/target")
        .await
        .unwrap();
    installer
        .add("https://github.com/acme/portable@v1.0.0")
        .await
        .unwrap();
    assert_eq!(
        installer
            .list()
            .unwrap()
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["portable", "target"]
    );
    assert!(
        !installer
            .update(Some("portable"))
            .await
            .pop()
            .unwrap()
            .unwrap()
            .changed
    );
    installer.remove("target").unwrap();
    assert_eq!(installer.list().unwrap().len(), 1);
    assert!(!temp.0.join("executed").exists());
    assert!(
        server
            .requests()
            .iter()
            .any(|path| path.contains("releases/tags/v1.0.0"))
    );
}

#[tokio::test]
#[rustfmt::skip]
async fn rejects_malicious_archives_checksums_identity_target_and_limits() {
    let cases: Vec<(&str, &str, Vec<u8>, bool)> = vec![
        ("slip", "demo-any.zip", releases::raw_zip(&[("../escape", b"x")]), true),
        ("symlink", "demo-any.zip", releases::symlink_zip(), true),
        ("checksum", "demo-any.zip", releases::zip("demo", "1.0.0", false, &[]), false),
        ("target", "demo-aarch64-apple-darwin.zip", releases::zip("demo", "1.0.0", true, &[]), true),
        ("folder", "demo-any.zip", releases::zip("other", "1.0.0", false, &[]), true),
        ("version", "demo-any.zip", releases::zip("demo", "2.0.0", false, &[]), true),
        ("entries", "demo-any.zip", releases::many_entries(), true),
    ];
    for (repo, asset, bytes, checksum) in cases {
        let temp = Temp::new(); let server = Server::new(); releases::publish(&server, "acme", repo, "v1.0.0", asset, bytes, checksum);
        assert!(installer(&temp, &server).add(&format!("https://github.com/acme/{repo}")).await.is_err(), "{repo}");
        assert!(!temp.0.join("escape").exists());
    }
}

#[tokio::test]
async fn ensure_missing_continues_after_one_failure_and_network_loss() {
    let temp = Temp::new();
    let server = Server::new();
    releases::publish(
        &server,
        "acme",
        "good",
        "v1.0.0",
        "good-any.zip",
        releases::zip("good", "1.0.0", false, &[]),
        true,
    );
    releases::publish(
        &server,
        "acme",
        "bad",
        "v1.0.0",
        "bad-any.zip",
        releases::zip("bad", "1.0.0", false, &[]),
        false,
    );
    let results = installer(&temp, &server)
        .ensure_missing([
            "https://github.com/acme/bad",
            "https://github.com/acme/good",
        ])
        .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(temp.0.join("plugins/good/plugin.toml").is_file());
    let dead_base = server.base().to_owned();
    drop(server);
    let offline = Installer::for_local_tests(
        InstallOptions {
            plugins_root: temp.0.join("plugins"),
            target: TARGET.into(),
        },
        &dead_base,
    )
    .unwrap();
    assert!(
        offline
            .ensure_missing(["https://github.com/acme/offline"])
            .await[0]
            .is_err()
    );
    assert!(temp.0.join("plugins/good/plugin.toml").is_file());
}

#[tokio::test]
async fn update_rejects_a_release_that_changes_the_plugin_name() {
    let temp = Temp::new();
    let server = Server::new();
    releases::publish(
        &server,
        "acme",
        "source-a",
        "v1.0.0",
        "plugin-a-any.zip",
        releases::zip("plugin-a", "1.0.0", false, &[]),
        true,
    );
    releases::publish(
        &server,
        "acme",
        "source-b",
        "v1.0.0",
        "plugin-b-any.zip",
        releases::zip("plugin-b", "1.0.0", false, &[]),
        true,
    );
    let installer = installer(&temp, &server);
    installer
        .add("https://github.com/acme/source-a")
        .await
        .unwrap();
    installer
        .add("https://github.com/acme/source-b")
        .await
        .unwrap();
    releases::publish(
        &server,
        "acme",
        "source-a",
        "v2.0.0",
        "plugin-b-any.zip",
        releases::zip("plugin-b", "2.0.0", false, &[]),
        true,
    );

    let error = installer
        .update(Some("plugin-a"))
        .await
        .pop()
        .unwrap()
        .unwrap_err();

    assert!(error.to_string().contains("plugin-a"));
    assert!(
        fs::read_to_string(temp.0.join("plugins/plugin-b/plugin.toml"))
            .unwrap()
            .contains("1.0.0")
    );
}
