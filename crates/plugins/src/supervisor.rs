use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::Duration,
};

use serde_json::json;
use streams_tools_core::{Bus, EventData, ReceiveError};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::watch,
    task::JoinHandle,
    time::Instant,
};

use crate::{Plugin, PluginOs, protocol};

const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct SupervisorOptions {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub failure_window: Duration,
    pub max_failures: usize,
    pub data_root: PathBuf,
}

impl Default for SupervisorOptions {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            failure_window: Duration::from_secs(60),
            max_failures: 5,
            data_root: PathBuf::from("./data"),
        }
    }
}

pub struct Supervisor {
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    disabled: Arc<Mutex<HashSet<String>>>,
}

impl Supervisor {
    pub fn start(bus: Bus, plugins: Vec<Plugin>, options: SupervisorOptions) -> Self {
        let (shutdown, _) = watch::channel(false);
        let disabled = Arc::new(Mutex::new(HashSet::new()));
        let counter = Arc::new(AtomicU64::new(0));
        let tasks = plugins
            .into_iter()
            .filter(|plugin| plugin.has_worker())
            .map(|plugin| {
                let task = WorkerTask {
                    bus: bus.clone(),
                    plugin,
                    options: options.clone(),
                    shutdown: shutdown.subscribe(),
                    disabled: disabled.clone(),
                    counter: counter.clone(),
                };
                tokio::spawn(task.run())
            })
            .collect();
        Self {
            shutdown,
            tasks,
            disabled,
        }
    }

    pub fn is_disabled(&self, name: &str) -> bool {
        lock(&self.disabled).contains(name)
    }

    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        for task in &mut self.tasks {
            let _ = task.await;
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for task in &self.tasks {
            task.abort();
        }
    }
}

struct WorkerTask {
    bus: Bus,
    plugin: Plugin,
    options: SupervisorOptions,
    shutdown: watch::Receiver<bool>,
    disabled: Arc<Mutex<HashSet<String>>>,
    counter: Arc<AtomicU64>,
}

impl WorkerTask {
    async fn run(mut self) {
        let name = self.plugin.manifest.name.clone();
        let mut failures = VecDeque::new();
        loop {
            if *self.shutdown.borrow() {
                return;
            }
            prune_failures(&mut failures, self.options.failure_window);
            if !failures.is_empty()
                && wait_or_shutdown(&mut self.shutdown, backoff(&self.options, failures.len()))
                    .await
            {
                return;
            }
            match run_worker(&mut self).await {
                RunOutcome::Shutdown | RunOutcome::BusClosed => return,
                RunOutcome::Failure(reason) => eprintln!("[plugin:{name}] {reason}"),
            }
            prune_failures(&mut failures, self.options.failure_window);
            failures.push_back(Instant::now());
            if exceeds_failures(failures.len(), self.options.max_failures) {
                lock(&self.disabled).insert(name.clone());
                eprintln!("[plugin:{name}] disabled after repeated worker failures");
                return;
            }
        }
    }
}

enum RunOutcome {
    Shutdown,
    BusClosed,
    Failure(String),
}

async fn run_worker(task: &mut WorkerTask) -> RunOutcome {
    let plugin = &task.plugin;
    let name = &plugin.manifest.name;
    let worker = plugin
        .manifest
        .worker
        .as_ref()
        .expect("filtered workers exist");
    let data_dir = task.options.data_root.join(name);
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        return RunOutcome::Failure(format!(
            "cannot create data dir {}: {error}",
            data_dir.display()
        ));
    }
    let data_dir = if data_dir.is_absolute() {
        data_dir
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(data_dir),
            Err(error) => return RunOutcome::Failure(format!("cannot resolve data dir: {error}")),
        }
    };
    let command = worker.command_for(PluginOs::current());
    let Some((program, args)) = command.split_first() else {
        return RunOutcome::Failure("worker command is empty".into());
    };
    let mut subscriber = task.bus.subscribe();
    let mut child = match Command::new(resolve_program(&plugin.root, program))
        .args(args)
        .current_dir(&plugin.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return RunOutcome::Failure(format!("cannot spawn `{program}`: {error}")),
    };
    let Some(stdin) = child.stdin.take() else {
        return RunOutcome::Failure("worker stdin is unavailable".into());
    };
    let Some(stdout) = child.stdout.take() else {
        return RunOutcome::Failure("worker stdout is unavailable".into());
    };
    let Some(stderr) = child.stderr.take() else {
        return RunOutcome::Failure("worker stderr is unavailable".into());
    };
    let mut stdin = stdin;
    let mut stdout = BufReader::new(stdout).lines();
    let mut stderr = BufReader::new(stderr).lines();
    let init = json!({"type":"init","config":plugin.effective_config.clone(),"data_dir":data_dir});
    let outcome = match write_json(&mut stdin, &init).await {
        Err(error) => RunOutcome::Failure(format!("cannot write init: {error}")),
        Ok(()) => {
            run_child(
                task,
                &mut child,
                &mut stdin,
                &mut stdout,
                &mut stderr,
                &mut subscriber,
            )
            .await
        }
    };
    terminate(&mut child).await;
    outcome
}

async fn run_child(
    task: &mut WorkerTask,
    child: &mut Child,
    stdin: &mut tokio::process::ChildStdin,
    stdout: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    stderr: &mut tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
    subscriber: &mut streams_tools_core::Subscriber,
) -> RunOutcome {
    let name = &task.plugin.manifest.name;
    let mut stdout_open = true;
    let mut stderr_open = true;
    loop {
        tokio::select! {
            changed = task.shutdown.changed() => {
                if changed.is_err() || *task.shutdown.borrow() { return RunOutcome::Shutdown; }
            }
            status = child.wait() => {
                drain_output(task, stdout, stderr, stdout_open, stderr_open).await;
                return RunOutcome::Failure(match status {
                    Ok(status) => format!("worker exited with {status}"),
                    Err(error) => format!("cannot wait for worker: {error}"),
                });
            }
            line = stdout.next_line(), if stdout_open => match line {
                Ok(Some(line)) => publish_line(task, &line),
                Ok(None) => stdout_open = false,
                Err(error) => return RunOutcome::Failure(format!("cannot read stdout: {error}")),
            },
            line = stderr.next_line(), if stderr_open => match line {
                Ok(Some(line)) => eprintln!("[plugin:{name}] {line}"),
                Ok(None) => stderr_open = false,
                Err(error) => return RunOutcome::Failure(format!("cannot read stderr: {error}")),
            },
            received = subscriber.recv() => match received {
                Ok(event) if task.plugin.manifest.permissions.listens_to(event_type(&event.event)) => {
                    if let Err(error) = write_json(stdin, &event).await {
                        return RunOutcome::Failure(format!("cannot write event: {error}"));
                    }
                }
                Ok(_) => {}
                Err(ReceiveError::Lagged(skipped)) => eprintln!("[plugin:{name}] subscriber lagged by {skipped} events"),
                Err(ReceiveError::Closed) => return RunOutcome::BusClosed,
            },
        }
    }
}

fn publish_line(task: &WorkerTask, line: &str) {
    let name = &task.plugin.manifest.name;
    match protocol::parse_output(line, name, &task.plugin.manifest.permissions, &task.counter) {
        Ok(event) => {
            let _ = task.bus.publish(event);
        }
        Err(error) => eprintln!("[plugin:{name}] {error}"),
    }
}

/// Lee la salida pendiente de un worker que ya terminó: el proceso puede salir
/// antes de que se hayan leído sus últimas líneas de stdout/stderr.
async fn drain_output(
    task: &WorkerTask,
    stdout: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    stderr: &mut tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
    mut stdout_open: bool,
    mut stderr_open: bool,
) {
    let name = &task.plugin.manifest.name;
    let drain = async {
        while stdout_open || stderr_open {
            tokio::select! {
                line = stdout.next_line(), if stdout_open => match line {
                    Ok(Some(line)) => publish_line(task, &line),
                    _ => stdout_open = false,
                },
                line = stderr.next_line(), if stderr_open => match line {
                    Ok(Some(line)) => eprintln!("[plugin:{name}] {line}"),
                    _ => stderr_open = false,
                },
            }
        }
    };
    // Un proceso nieto puede heredar los pipes y mantenerlos abiertos.
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, drain).await;
}

fn event_type(event: &EventData) -> &str {
    match event {
        EventData::Follow(_) => "follow",
        EventData::Sub(_) => "sub",
        EventData::GiftSub(_) => "gift_sub",
        EventData::Raid(_) => "raid",
        EventData::Donation(_) => "donation",
        EventData::Redemption(_) => "redemption",
        EventData::Chat(_) => "chat",
        EventData::Plugin(data) => &data.name,
    }
}

async fn write_json(
    stdin: &mut tokio::process::ChildStdin,
    value: &impl serde::Serialize,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await
}

async fn terminate(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn resolve_program(root: &Path, program: &str) -> PathBuf {
    if !program.contains(['/', '\\']) && !program.starts_with('.') {
        return PathBuf::from(program);
    }
    let mut path = root.join(program);
    if cfg!(windows) && path.extension().is_none() {
        path.set_extension("exe");
    }
    path
}

fn prune_failures(failures: &mut VecDeque<Instant>, window: Duration) {
    let now = Instant::now();
    while failures
        .front()
        .is_some_and(|failure| now.duration_since(*failure) > window)
    {
        failures.pop_front();
    }
}

fn backoff(options: &SupervisorOptions, failures: usize) -> Duration {
    let exponent = failures.saturating_sub(1).min(31) as u32;
    options
        .initial_backoff
        .saturating_mul(2_u32.saturating_pow(exponent))
        .min(options.max_backoff)
}

async fn wait_or_shutdown(shutdown: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.changed() => true,
    }
}

fn exceeds_failures(count: usize, maximum: usize) -> bool {
    count > maximum
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_dot_and_path_programs() {
        let root = Path::new("plugin");
        let expected = if cfg!(windows) {
            "plugin/bin/worker.exe"
        } else {
            "plugin/bin/worker"
        };
        assert_eq!(resolve_program(root, "bin/worker"), PathBuf::from(expected));
        let dot = if cfg!(windows) {
            "plugin/./worker.exe"
        } else {
            "plugin/./worker"
        };
        assert_eq!(resolve_program(root, "./worker"), PathBuf::from(dot));
        assert_eq!(resolve_program(root, "node"), PathBuf::from("node"));
    }

    #[test]
    fn backoff_saturates_and_disable_requires_more_than_max_failures() {
        let options = SupervisorOptions {
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(45),
            ..SupervisorOptions::default()
        };
        assert_eq!(backoff(&options, 1), Duration::from_millis(10));
        assert_eq!(backoff(&options, 3), Duration::from_millis(40));
        assert_eq!(backoff(&options, 4), Duration::from_millis(45));
        assert_eq!(backoff(&options, usize::MAX), Duration::from_millis(45));
        assert!(!exceeds_failures(5, 5));
        assert!(exceeds_failures(6, 5));
    }
}
