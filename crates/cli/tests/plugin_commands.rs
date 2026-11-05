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
            "streams-cli-{}-{}",
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

fn command(temp: &Temp) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_streams-tools"));
    command.env("STREAMS_PLUGINS_DIR", temp.0.join("plugins"));
    command
}

#[test]
fn help_version_and_usage_have_stable_exit_codes() {
    let temp = Temp::new();
    let output = command(&temp).arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("plugin add <url>")
    );
    let output = command(&temp).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("streams-tools 0.1.0")
    );
    let output = command(&temp).args(["plugin", "add"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn list_is_offline_and_reports_unmanaged_plugins() {
    let temp = Temp::new();
    let plugin = temp.0.join("plugins/local");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(
        plugin.join("plugin.toml"),
        "name='local'\nversion='1'\n[permissions]\nlistens=[]\nemits=[]",
    )
    .unwrap();
    let output = command(&temp).args(["plugin", "list"]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "local\t1\tunmanaged\t-\n"
    );
}
