use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use streams_tools_core::{Bus, ChatData, Event, EventData, FollowData, Platform, Subscriber, User};
use streams_tools_plugins::{Manifest, Permissions, Plugin, Supervisor, SupervisorOptions, Worker};
use tokio::time::timeout;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-supervisor-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temp dir should be created");
        Self(path)
    }

    fn plugin(&self, mode: &str, listens: &[&str]) -> Plugin {
        let name = format!("worker-{mode}");
        let root = self.0.join(&name);
        fs::create_dir(&root).expect("plugin dir should be created");
        let mut config = toml::Table::new();
        config.insert("title".into(), toml::Value::String("exact config".into()));
        Plugin {
            root,
            effective_config: serde_json::json!({"title":"effective config"}),
            manifest: Manifest {
                name,
                version: "1".into(),
                description: None,
                permissions: Permissions {
                    listens: listens.iter().map(|kind| (*kind).into()).collect(),
                    emits: vec!["observed".into()],
                },
                worker: Some(Worker {
                    command: vec![env!("CARGO_BIN_EXE_plugin-test-worker").into(), mode.into()],
                    command_windows: None,
                    command_macos: None,
                    command_linux: None,
                }),
                config,
            },
        }
    }

    fn options(&self, max_failures: usize) -> SupervisorOptions {
        SupervisorOptions {
            initial_backoff: Duration::from_millis(2),
            max_backoff: Duration::from_millis(5),
            failure_window: Duration::from_secs(60),
            max_failures,
            data_root: self.0.join("data"),
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn core_event(kind: &str) -> Event {
    Event {
        id: kind.into(),
        platform: Platform::Twitch,
        event: if kind == "chat" {
            EventData::Chat(ChatData {
                message: "hello".into(),
            })
        } else {
            EventData::Follow(FollowData {})
        },
        user: User {
            id: "u".into(),
            name: "User".into(),
            roles: Vec::new(),
        },
        ts: "2026-01-01T00:00:00Z".into(),
    }
}

async fn next_plugin(subscriber: &mut Subscriber, name: &str) -> Event {
    timeout(Duration::from_secs(2), async {
        loop {
            let event = subscriber.recv().await.expect("bus should remain open");
            if event.platform == Platform::Plugin(name.into()) {
                return event;
            }
        }
    })
    .await
    .expect("plugin event should arrive before timeout")
}

fn observed_input(event: &Event) -> &serde_json::Value {
    let EventData::Plugin(data) = &event.event else {
        panic!("expected plugin event");
    };
    assert_eq!(data.name, "observed");
    &data.data["input"]
}

async fn shutdown(supervisor: Supervisor) {
    timeout(Duration::from_secs(2), supervisor.shutdown())
        .await
        .expect("supervisor should shut down before timeout");
}

#[tokio::test]
async fn init_reaches_worker_and_only_declared_input_is_forwarded() {
    let temp = TempDir::new();
    let plugin = temp.plugin("normal", &["chat"]);
    let name = plugin.manifest.name.clone();
    let bus = Bus::new(32).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let supervisor = Supervisor::start(bus.clone(), vec![plugin], temp.options(5));
    let init = next_plugin(&mut subscriber, &name).await;
    let input = observed_input(&init);
    assert_eq!(input["config"]["title"], "effective config");
    let data_dir = PathBuf::from(input["data_dir"].as_str().expect("data_dir string"));
    assert!(data_dir.is_absolute() && data_dir.is_dir());
    assert_eq!(
        data_dir.file_name().and_then(|part| part.to_str()),
        Some(name.as_str())
    );
    assert!(!init.id.is_empty() && init.ts.ends_with('Z'));
    assert_eq!(init.user.id, name);

    bus.publish(core_event("follow"))
        .expect("subscriber exists");
    bus.publish(core_event("chat")).expect("subscriber exists");
    let chat = next_plugin(&mut subscriber, &name).await;
    assert_eq!(observed_input(&chat)["type"], "chat");
    assert!(
        timeout(
            Duration::from_millis(100),
            next_plugin(&mut subscriber, &name)
        )
        .await
        .is_err()
    );
    shutdown(supervisor).await;
}

#[tokio::test]
async fn forbidden_output_is_discarded_without_stopping_worker() {
    let temp = TempDir::new();
    let plugin = temp.plugin("forbidden", &[]);
    let name = plugin.manifest.name.clone();
    let bus = Bus::new(32).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let supervisor = Supervisor::start(bus, vec![plugin], temp.options(5));
    next_plugin(&mut subscriber, &name).await;
    assert!(
        timeout(
            Duration::from_millis(100),
            next_plugin(&mut subscriber, &name)
        )
        .await
        .is_err()
    );
    shutdown(supervisor).await;
}

#[tokio::test]
async fn crashed_after_output_restarts_and_emits_again() {
    let temp = TempDir::new();
    let plugin = temp.plugin("restart", &[]);
    let name = plugin.manifest.name.clone();
    let bus = Bus::new(32).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let supervisor = Supervisor::start(bus, vec![plugin], temp.options(5));
    next_plugin(&mut subscriber, &name).await;
    next_plugin(&mut subscriber, &name).await;
    shutdown(supervisor).await;
}

#[tokio::test]
async fn output_written_right_before_exit_is_not_lost() {
    let temp = TempDir::new();
    let plugin = temp.plugin("burst", &[]);
    let name = plugin.manifest.name.clone();
    let bus = Bus::new(256).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let supervisor = Supervisor::start(bus, vec![plugin], temp.options(5));
    // La primera línea es el eco de init; después deben llegar las 50 en orden
    // antes de que un reinicio vuelva a empezar desde cero.
    next_plugin(&mut subscriber, &name).await;
    for seq in 0..50 {
        let event = next_plugin(&mut subscriber, &name).await;
        assert_eq!(observed_input(&event)["seq"], seq);
    }
    shutdown(supervisor).await;
}

#[tokio::test]
async fn repeated_crashes_disable_only_after_exceeding_limit() {
    let temp = TempDir::new();
    let plugin = temp.plugin("crash", &[]);
    let name = plugin.manifest.name.clone();
    let bus = Bus::new(32).expect("capacity should be valid");
    let supervisor = Supervisor::start(bus, vec![plugin], temp.options(1));
    timeout(Duration::from_secs(2), async {
        while !supervisor.is_disabled(&name) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker should be disabled after two failures");
    shutdown(supervisor).await;
}

#[tokio::test]
async fn shutdown_kills_worker_blocked_on_stdin() {
    let temp = TempDir::new();
    let plugin = temp.plugin("normal", &[]);
    let name = plugin.manifest.name.clone();
    let bus = Bus::new(32).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let supervisor = Supervisor::start(bus, vec![plugin], temp.options(5));
    next_plugin(&mut subscriber, &name).await;
    shutdown(supervisor).await;
}
