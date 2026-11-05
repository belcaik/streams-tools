use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-paths-{}-{}",
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

#[test]
fn relative_plugin_override_is_resolved_against_process_cwd() {
    let temp = Temp::new();
    let plugin = temp.0.join("relative/demo");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(
        plugin.join("plugin.toml"),
        "name='demo'\nversion='1'\n[permissions]\nlistens=[]\nemits=[]",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_streams-tools"))
        .current_dir(&temp.0)
        .env("STREAMS_PLUGINS_DIR", "relative")
        .args(["plugin", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("demo\t1\tunmanaged")
    );
}
