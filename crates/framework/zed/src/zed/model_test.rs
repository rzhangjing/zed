//! `Help > Model Test`: build, train and test the `crates/models` Burn demo from
//! a window, and watch the child process's own output while it runs.
//!
//! `crates/models` is a **separate** cargo workspace (Burn 0.22.1 + ndarray, its
//! own `vendor/`, `artifacts/` and `target/`) with a CLI: `models train --epochs N
//! --seed S` and `models test`, writing and reading `artifacts/model.bpk`. This
//! module is a supervisor for that binary and nothing else: Zed never links Burn,
//! and the dependency tree of `zed` does not change.
//!
//! ## Lifecycle
//!
//! `Build` / `Train` / `Test` spawn a child with `std::process::Command` and
//! return immediately. Two reader threads pump the child's stdout and stderr,
//! line by line, into an unbounded channel; a supervisor thread polls the child
//! with `try_wait` (never holding the handle across a wait) and reports the exit
//! status once both pipes have reached EOF. The window drains the channel on
//! GPUI's executor, so no pipe read and no `wait` ever runs on the UI thread.
//!
//! `Train` / `Test` auto-build first when
//! `crates/models/target/debug/models.exe` is missing: the window switches to
//! [`Status::Building`], runs
//! `cargo build --offline --manifest-path crates/models/Cargo.toml` **with
//! `crates/models` as the working directory** (that is where the crate's
//! vendored-sources `.cargo/config.toml`, `artifacts/` and `target/` resolve
//! from), and starts the requested job only if the build succeeded.
//!
//! `Stop` kills the child through the shared handle; closing the window does the
//! same in `Drop`, so a closed window never leaves a training run behind.
//!
//! ## The log
//!
//! The newest [`LOG_CAPACITY`] lines of the child's two streams plus this
//! module's own lifecycle lines. Every line is also written through `log::info!`
//! / `log::error!` with a `[model-test]` prefix, so the same sequence is in
//! `Zed.log`.
//!
//! ## Verified shapes
//!
//! The status machine, the ring, the accuracy parser, the path derivation and the
//! command lines are pure and are unit-tested below. `ZED_MODELTEST_OPEN=1` opens
//! the window at startup (the menu path without the menu),
//! `ZED_MODELTEST_RUN=train|test|cluster` presses the matching button once the
//! window is up, and `ZED_MODELTEST_EPOCHS` overrides the epoch count for a
//! scripted check. For the cluster hook, `ZED_MODELTEST_ALGO` selects the
//! algorithm (`all` by default).

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, ClipboardItem, Context, IntoElement, Render, ScrollHandle, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, px, size,
};
use release_channel::ReleaseChannel;
use theme::ActiveTheme as _;
use ui::{Button, ButtonStyle, Color, Label, LabelSize, TintColor, prelude::*};

/// `ZED_MODELTEST_OPEN` — `1`/anything but `0` opens the window as Zed starts.
///
/// The menu item's path without the menu, for verifying the window on a machine
/// where clicking through `Help` is not what is being tested. Mirrors
/// `ZED_ADMIN_DEMO_OPEN`.
pub const ENV_OPEN: &str = "ZED_MODELTEST_OPEN";

/// `ZED_MODELTEST_RUN` — `train`, `test` or `cluster`; press that button as soon
/// as the window opens.
///
/// A scripted-verification hook, like [`ENV_OPEN`]: it runs the button's own
/// handler, so checking that a real training run reaches `Done` does not need a
/// click. Off by default.
pub const ENV_RUN: &str = "ZED_MODELTEST_RUN";

/// `ZED_MODELTEST_ALGO` — the clustering algorithm used by `ZED_MODELTEST_RUN=cluster`.
///
/// Defaults to `all`, which runs the whole comparison table. Any value the
/// `models cluster --algo` subcommand accepts works (`kmeans`, `linfa-kmeans`,
/// `linfa-dbscan`, `linfa-optics`, `linfa-gmm`, `smartcore-kmeans`,
/// `smartcore-dbscan`, `hdbscan`, `all`).
pub const ENV_ALGO: &str = "ZED_MODELTEST_ALGO";

/// `ZED_MODELTEST_EPOCHS` — the epoch count the window starts with.
///
/// Overrides the default 3 so a scripted check can train for two epochs instead
/// of three. Ignored when it is not a positive integer.
pub const ENV_EPOCHS: &str = "ZED_MODELTEST_EPOCHS";

/// How many log lines the window keeps.
pub const LOG_CAPACITY: usize = 500;

/// The epoch count a fresh window starts with, and the seed every run passes.
const DEFAULT_EPOCHS: usize = 3;
const DEFAULT_SEED: u64 = 42;
/// Points the cluster buttons pass to `models cluster --points`.
const DEFAULT_CLUSTER_POINTS: usize = 600;
/// Requested cluster count the cluster buttons pass to `models cluster --clusters`.
const DEFAULT_CLUSTER_CLUSTERS: usize = 3;

/// How long the `ZED_MODELTEST_RUN` hook waits after the window opens before
/// pressing its button, so the window is mapped and the theme is up.
const AUTO_START_DELAY: Duration = Duration::from_millis(400);

/// How often the supervisor thread polls the child for its exit status.
///
/// `try_wait` instead of `wait` is what lets `Stop` take the handle and kill
/// without waiting on a process that is still running.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// How many stderr lines a failure message quotes.
const STDERR_TAIL: usize = 3;

/// How long one quoted stderr line may be, so a compiler dump does not turn the
/// failure label into a wall of text.
const STDERR_LINE_LIMIT: usize = 220;

/// `CREATE_NO_WINDOW`: `cargo` and `models` are console-subsystem programs, and
/// a GUI process spawning them would otherwise flash a console window.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The repository root: `crates/framework/zed` three levels up.
///
/// `env!("CARGO_MANIFEST_DIR")` is fixed at compile time and is the path this
/// checkout was built from, which is the checkout whose `crates/models` the
/// window is meant to drive.
pub fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(3)
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

/// `crates/models`: the separate workspace the window supervises.
pub fn models_dir() -> PathBuf {
    repo_root().join("crates").join("models")
}

/// `crates/models/target/debug/models.exe` (`.exe` only where the platform has
/// that suffix).
pub fn models_exe() -> PathBuf {
    models_dir()
        .join("target")
        .join("debug")
        .join(format!("models{}", std::env::consts::EXE_SUFFIX))
}

/// `crates/models/artifacts/model.bpk`, for the footer.
pub fn checkpoint_path() -> PathBuf {
    models_dir().join("artifacts").join("model.bpk")
}

/// What the window is showing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Nothing is running; `Build`, `Train` and `Test` are available.
    Idle,
    /// `cargo build` is running because `models.exe` was missing.
    Building,
    /// `models train` is running.
    Training,
    /// `models test` is running.
    Testing,
    /// The last train/test run succeeded. `acc` is the percentage the run last
    /// reported, when a line could be parsed.
    Done { acc: Option<f32> },
    /// The last start (or the last run) failed; nothing is running.
    Failed { reason: String },
}

impl Status {
    /// The one-line state shown next to the title.
    pub fn summary(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Building => "Building",
            Self::Training => "Training",
            Self::Testing => "Testing",
            Self::Done { .. } => "Done",
            Self::Failed { .. } => "Failed",
        }
    }

    /// The second line: what is happening, the accuracy, or the failure.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Idle => Some("ready: Build, Train, Test or a clustering algorithm".to_owned()),
            Self::Building => Some("running cargo build --offline in crates/models".to_owned()),
            Self::Training => Some("running models train".to_owned()),
            Self::Testing => Some("running models test or cluster".to_owned()),
            Self::Done { acc: Some(acc) } => Some(format!("accuracy {acc:.2}%")),
            Self::Done { acc: None } => Some("finished".to_owned()),
            Self::Failed { reason } => Some(reason.clone()),
        }
    }

    /// The colour of [`Status::summary`].
    pub fn color(&self) -> Color {
        match self {
            Self::Idle => Color::Muted,
            Self::Building | Self::Training | Self::Testing => Color::Warning,
            Self::Done { .. } => Color::Success,
            Self::Failed { .. } => Color::Error,
        }
    }

    /// Whether a child process belongs to this status.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Building | Self::Training | Self::Testing)
    }
}

/// Which buttons are enabled for a status (and a non-empty log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub build: bool,
    pub train: bool,
    pub test: bool,
    /// The cluster test buttons (`K-Means`, `DBSCAN`, ..., `All`).
    pub cluster: bool,
    pub stop: bool,
    pub clear_log: bool,
    pub copy_log: bool,
}

/// The button state for `status`.
///
/// `log_is_empty` drives `Clear log` / `Copy log`: they act on the ring, so they
/// are available whenever there is something in it.
pub fn buttons_for(status: &Status, log_is_empty: bool) -> Buttons {
    let idle = matches!(
        status,
        Status::Idle | Status::Done { .. } | Status::Failed { .. }
    );
    let busy = status.is_busy();
    let log = !log_is_empty;
    Buttons {
        build: idle,
        train: idle,
        test: idle,
        cluster: idle,
        stop: busy,
        clear_log: log,
        copy_log: log,
    }
}

/// The newest `capacity` lines, oldest first.
#[derive(Debug)]
pub struct LogRing {
    capacity: usize,
    lines: VecDeque<String>,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: VecDeque::with_capacity(capacity),
        }
    }

    /// Appends `line`, dropping the oldest line when the ring is full.
    pub fn push(&mut self, line: impl Into<String>) {
        if self.capacity == 0 {
            return;
        }
        while self.lines.len() >= self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    /// The whole ring as one string, for `Copy log`.
    pub fn joined(&self) -> String {
        let mut joined = String::new();
        for line in self.lines() {
            joined.push_str(line);
            joined.push('\n');
        }
        joined
    }
}

/// The jobs the window can run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Build,
    Train,
    Test,
    /// `models cluster --algo <algo>`; `algo` is `all` for the comparison table.
    Cluster { algo: String },
}

impl JobKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Train => "train",
            Self::Test => "test",
            Self::Cluster { .. } => "cluster",
        }
    }

    /// The status the window shows while this job runs.
    ///
    /// Clustering reuses [`Status::Testing`]: it is another `models` subcommand
    /// whose child is watched by the same state machine.
    pub fn status(&self) -> Status {
        match self {
            Self::Build => Status::Building,
            Self::Train => Status::Training,
            Self::Test | Self::Cluster { .. } => Status::Testing,
        }
    }

    /// Parses the `ZED_MODELTEST_RUN` value.
    ///
    /// `cluster` returns the `all` default; [`ModelTestView::new`] replaces it
    /// with the `ZED_MODELTEST_ALGO` value when that variable is set.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "train" => Some(Self::Train),
            "test" => Some(Self::Test),
            "cluster" => Some(Self::Cluster {
                algo: "all".to_owned(),
            }),
            _ => None,
        }
    }
}

/// One fully resolved child invocation.
#[derive(Clone, Debug)]
pub struct JobPlan {
    pub kind: JobKind,
    pub program: String,
    pub args: Vec<String>,
    /// Always `crates/models`: the crate's `.cargo/config.toml`, `vendor/`,
    /// `artifacts/` and `target/` are all relative to it, so both the build and
    /// the binary have to start there.
    pub cwd: PathBuf,
}

impl JobPlan {
    /// The command as one line, for the log and for failure messages.
    pub fn command_line(&self) -> String {
        let mut line = quote(&self.program);
        for arg in &self.args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        line
    }
}

fn quote(word: &str) -> String {
    if word.contains(' ') || word.contains('\t') {
        format!("\"{word}\"")
    } else {
        word.to_owned()
    }
}

/// The command for `kind`: the debug `models.exe` for train/test, `cargo build
/// --offline` for the build.
///
/// The manifest path is passed explicitly *and* the working directory is
/// `crates/models`, which is what makes the crate's vendored sources apply: its
/// `.cargo/config.toml` is found from the working directory, and `--offline`
/// keeps the build off the network.
pub fn plan_for(kind: JobKind, epochs: usize, seed: u64) -> JobPlan {
    let cwd = models_dir();
    match kind {
        JobKind::Build => JobPlan {
            kind,
            program: "cargo".to_owned(),
            args: vec![
                "build".to_owned(),
                "--offline".to_owned(),
                "--manifest-path".to_owned(),
                cwd.join("Cargo.toml").display().to_string(),
            ],
            cwd,
        },
        JobKind::Train => JobPlan {
            kind,
            program: models_exe().display().to_string(),
            args: vec![
                "train".to_owned(),
                "--epochs".to_owned(),
                epochs.to_string(),
                "--seed".to_owned(),
                seed.to_string(),
            ],
            cwd,
        },
        JobKind::Test => JobPlan {
            kind,
            program: models_exe().display().to_string(),
            args: vec!["test".to_owned(), "--seed".to_owned(), seed.to_string()],
            cwd,
        },
        JobKind::Cluster { algo } => {
            let plan_kind = JobKind::Cluster { algo: algo.clone() };
            JobPlan {
                kind: plan_kind,
                program: models_exe().display().to_string(),
                args: vec![
                    "cluster".to_owned(),
                    "--algo".to_owned(),
                    algo,
                    "--points".to_owned(),
                    DEFAULT_CLUSTER_POINTS.to_string(),
                    "--clusters".to_owned(),
                    DEFAULT_CLUSTER_CLUSTERS.to_string(),
                    "--seed".to_owned(),
                    seed.to_string(),
                ],
                cwd,
            }
        }
    }
}

/// Which of the child's streams a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// The percentage on the first `accuracy` in `line`, if the line has one.
///
/// One parser covers every shape the demo prints:
///
/// * `epoch 1/2 | train loss 1.0986 | test accuracy 25.00% | 0.2s`
/// * `final test accuracy: 87.50%`
/// * `test accuracy    : 87.50% (35 / 40)`
pub fn accuracy_percent(line: &str) -> Option<f32> {
    let lowered = line.to_ascii_lowercase();
    let rest = &line[lowered.find("accuracy")? + "accuracy".len()..];
    let percent = rest.find('%')?;
    let digits: String = rest[..percent]
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits
        .parse::<f32>()
        .ok()
        .filter(|value| (0.0..=100.0).contains(value))
}

/// The pure part of the window: status, log, and the run's parameters.
///
/// Every transition appends the line that explains it, so the log always
/// accounts for what the status says.
#[derive(Debug)]
pub struct ModelTestState {
    pub status: Status,
    pub log: LogRing,
    pub epochs: usize,
    pub seed: u64,
    /// The last accuracy seen in the running job's output.
    last_acc: Option<f32>,
    /// The newest stderr lines, quoted by a failure message.
    stderr_tail: VecDeque<String>,
    /// The command of the current (or last) job, quoted by a failure message.
    last_command: String,
}

impl ModelTestState {
    pub fn new(epochs: usize, seed: u64) -> Self {
        Self {
            status: Status::Idle,
            log: LogRing::new(LOG_CAPACITY),
            epochs,
            seed,
            last_acc: None,
            stderr_tail: VecDeque::new(),
            last_command: String::new(),
        }
    }

    pub fn buttons(&self) -> Buttons {
        buttons_for(&self.status, self.log.is_empty())
    }

    /// Appends a line to the ring and to `Zed.log`.
    pub fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        log::info!("{line}");
        self.log.push(line);
    }

    /// Appends a failure line to the ring and to `Zed.log` at error level.
    fn log_error(&mut self, line: impl Into<String>) {
        let line = line.into();
        log::error!("{line}");
        self.log.push(line);
    }

    /// A job was accepted: the child is spawned and its command is recorded.
    pub fn begin_job(&mut self, kind: JobKind, command_line: &str) {
        self.status = kind.status();
        self.last_acc = None;
        self.stderr_tail.clear();
        self.last_command = command_line.to_owned();
        self.log(format!(
            "[model-test] {} in {}: {}",
            kind.label(),
            models_dir().display(),
            command_line
        ));
    }

    /// One line of child output: into the ring, and through the accuracy parser.
    pub fn observe_line(&mut self, stream: Stream, line: &str) {
        let line = line.trim_end();
        if line.is_empty() {
            return;
        }
        let displayed = match stream {
            Stream::Stdout => line.to_owned(),
            Stream::Stderr => format!("stderr: {line}"),
        };
        self.log(displayed);

        if stream == Stream::Stderr {
            while self.stderr_tail.len() >= STDERR_TAIL {
                self.stderr_tail.pop_front();
            }
            self.stderr_tail.push_back(clip(line));
        }
        if let Some(accuracy) = accuracy_percent(line) {
            self.last_acc = Some(accuracy);
        }
    }

    /// The last job succeeded.
    pub fn succeeded(&mut self, kind: JobKind) {
        match kind {
            JobKind::Build => {
                self.status = Status::Idle;
                self.log("[model-test] build succeeded");
            }
            JobKind::Train | JobKind::Test | JobKind::Cluster { .. } => {
                let acc = self.last_acc.take();
                self.status = Status::Done { acc };
                match acc {
                    Some(acc) => self.log(format!(
                        "[model-test] {} done: accuracy {acc:.2}%",
                        kind.label()
                    )),
                    None => self.log(format!("[model-test] {} done", kind.label())),
                }
            }
        }
    }

    /// The last job failed; nothing is running.
    pub fn failed(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.log_error(format!("[model-test] failed: {reason}"));
        self.status = Status::Failed { reason };
    }

    /// `Stop` killed the running job.
    pub fn stopped(&mut self, kind: JobKind) {
        self.status = Status::Idle;
        self.log(format!("[model-test] stopped the {} process", kind.label()));
    }

    /// The failure message for an exit that was not a success: the kind, the
    /// code, the stderr tail and the command that produced it.
    pub fn describe_failure(&self, kind: JobKind, code: Option<i32>) -> String {
        let mut reason = match code {
            Some(code) => format!("{} failed with exit code {code}", kind.label()),
            None => format!("{} was terminated", kind.label()),
        };
        if !self.stderr_tail.is_empty() {
            let tail: Vec<&str> = self.stderr_tail.iter().map(String::as_str).collect();
            reason.push_str(&format!("; stderr: {}", tail.join(" / ")));
        }
        if !self.last_command.is_empty() {
            reason.push_str(&format!("; command: {}", self.last_command));
        }
        reason
    }
}

impl Default for ModelTestState {
    fn default() -> Self {
        Self::new(DEFAULT_EPOCHS, DEFAULT_SEED)
    }
}

/// One line, shortened to [`STDERR_LINE_LIMIT`] characters.
fn clip(line: &str) -> String {
    if line.chars().count() <= STDERR_LINE_LIMIT {
        return line.to_owned();
    }
    let mut clipped: String = line.chars().take(STDERR_LINE_LIMIT).collect();
    clipped.push('…');
    clipped
}

/// What the supervisor thread reports back to the window.
#[derive(Debug)]
enum Event {
    /// One line of the child's stdout or stderr.
    Line { stream: Stream, line: String },
    /// The child ended, or could not be waited for.
    Finished { outcome: Result<ExitStatus, String> },
}

/// A spawned child, shared between the window (`Stop`, `Drop`) and the
/// supervisor thread (which reaps it).
struct Running {
    kind: JobKind,
    command_line: String,
    child: Arc<Mutex<Option<Child>>>,
}

/// The mutex is only ever held for a `try_wait` or a `kill`; a poisoned lock
/// still holds a valid child handle, so recover instead of unwrapping.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads `pipe` line by line on its own thread and forwards every line to the
/// window.
///
/// `stdout` and `stderr` have different types (`ChildStdout` / `ChildStderr`),
/// so the body is generic over the pipe and shared; the thread ends when the
/// pipe reaches EOF, which is when the child has exited and closed it.
fn spawn_reader<R>(
    pipe: R,
    stream: Stream,
    events: mpsc::UnboundedSender<Event>,
) -> std::thread::JoinHandle<()>
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if events.unbounded_send(Event::Line { stream, line }).is_err() {
                        // The window is gone; nobody is reading.
                        return;
                    }
                }
                Err(error) => {
                    let _ = events.unbounded_send(Event::Line {
                        stream,
                        line: format!("<{} read error: {error}>", stream.label()),
                    });
                    return;
                }
            }
        }
    })
}

/// Spawns `plan` and starts its reader and supervisor threads.
///
/// The child is created on the calling (UI) thread — `spawn` is a single process
/// creation — and every read, every wait and the exit-status poll happen on
/// threads. The returned [`Running`] shares the child handle so `Stop` can kill
/// it without waiting for it.
fn spawn_job(plan: JobPlan, events: mpsc::UnboundedSender<Event>) -> Result<Running, String> {
    let command_line = plan.command_line();
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // `--offline` is on the build command line already; the variable covers
        // a cargo that decides to resolve anything else.
        .env("CARGO_NET_OFFLINE", "true")
        // `crates/models` has its own `target/`; a `CARGO_TARGET_DIR` inherited
        // from Zed's own build would put `models.exe` somewhere else.
        .env_remove("CARGO_TARGET_DIR");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("could not run `{command_line}`: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));

    let mut readers = Vec::new();
    if let Some(stdout) = stdout {
        readers.push(spawn_reader(stdout, Stream::Stdout, events.clone()));
    }
    if let Some(stderr) = stderr {
        readers.push(spawn_reader(stderr, Stream::Stderr, events.clone()));
    }

    let supervisor = child.clone();
    let kind = plan.kind;
    let supervisor_kind = kind.clone();
    std::thread::spawn(move || {
        let outcome = loop {
            let polled = {
                let mut guard = lock(&supervisor);
                match guard.as_mut() {
                    Some(child) => child.try_wait(),
                    // Only this thread takes the handle, so this is unreachable;
                    // reporting it is still better than polling forever.
                    None => break Err("the child handle is gone".to_owned()),
                }
            };
            match polled {
                Ok(Some(status)) => {
                    // `try_wait` already collected and reaped it; dropping the
                    // handle now closes the last one this process holds.
                    let _ = lock(&supervisor).take();
                    break Ok(status);
                }
                Ok(None) => std::thread::sleep(EXIT_POLL_INTERVAL),
                Err(error) => {
                    let _ = lock(&supervisor).take();
                    break Err(format!(
                        "could not wait for the {} process: {error}",
                        supervisor_kind.label()
                    ));
                }
            }
        };
        // Both pipes reach EOF once the process is gone; joining here keeps the
        // exit status *after* the last line, so `Done` never overtakes output.
        for reader in readers {
            let _ = reader.join();
        }
        let _ = events.unbounded_send(Event::Finished { outcome });
    });

    Ok(Running {
        kind,
        command_line,
        child,
    })
}

/// Kills the child behind `child`, if it is still there.
fn kill_child(child: &Arc<Mutex<Option<Child>>>) -> std::io::Result<()> {
    match lock(child).as_mut() {
        Some(child) => child.kill(),
        None => Ok(()),
    }
}

/// The window's root view.
pub struct ModelTestView {
    state: ModelTestState,
    running: Option<Running>,
    /// When the current job is the auto-build of a missing `models.exe`, the job
    /// to start once the build succeeds.
    pending_after_build: Option<JobKind>,
    scroll: ScrollHandle,
}

impl ModelTestView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let epochs = std::env::var(ENV_EPOCHS)
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|epochs| *epochs > 0)
            .unwrap_or(DEFAULT_EPOCHS);
        if epochs != DEFAULT_EPOCHS {
            log::info!("[model-test] epoch count from {ENV_EPOCHS}: {epochs}");
        }

        let mut auto_run = std::env::var(ENV_RUN)
            .ok()
            .and_then(|value| JobKind::parse(&value));
        if auto_run.is_none() && std::env::var(ENV_RUN).is_ok() {
            log::warn!(
                "[model-test] ignoring {ENV_RUN}: expected `train`, `test` or `cluster`"
            );
        }
        if let Some(JobKind::Cluster { algo }) = auto_run.as_mut() {
            let requested = std::env::var(ENV_ALGO).unwrap_or_else(|_| "all".to_owned());
            let requested = requested.trim();
            if !requested.is_empty() {
                *algo = requested.to_ascii_lowercase();
            }
            log::info!(
                "[model-test] cluster algorithm from {ENV_ALGO}: {algo} (default `all`)"
            );
        }
        if let Some(kind) = auto_run {
            log::info!("[model-test] auto-running {} ({ENV_RUN})", kind.label());
            cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(AUTO_START_DELAY).await;
                cx.update(|cx| {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |view, cx| match kind {
                            JobKind::Build => view.build(cx),
                            JobKind::Train => view.train(cx),
                            JobKind::Test => view.test(cx),
                            JobKind::Cluster { algo } => view.cluster(&algo, cx),
                        });
                    }
                });
            })
            .detach();
        }

        Self {
            state: ModelTestState::new(epochs, DEFAULT_SEED),
            running: None,
            pending_after_build: None,
            scroll: ScrollHandle::new(),
        }
    }

    fn build(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().build {
            return;
        }
        self.start_job(JobKind::Build, None, cx);
    }

    fn train(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().train {
            return;
        }
        if models_exe().is_file() {
            self.start_job(JobKind::Train, None, cx);
        } else {
            self.start_job(JobKind::Build, Some(JobKind::Train), cx);
        }
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().test {
            return;
        }
        if models_exe().is_file() {
            self.start_job(JobKind::Test, None, cx);
        } else {
            self.start_job(JobKind::Build, Some(JobKind::Test), cx);
        }
    }

    /// One of the cluster buttons: `algo` is a `models cluster --algo` value.
    fn cluster(&mut self, algo: &str, cx: &mut Context<Self>) {
        if !self.state.buttons().cluster {
            return;
        }
        let kind = JobKind::Cluster {
            algo: algo.to_owned(),
        };
        if models_exe().is_file() {
            self.start_job(kind, None, cx);
        } else {
            self.start_job(JobKind::Build, Some(kind), cx);
        }
    }

    /// Spawns `kind`; `then` is the job to run if this is an auto-build.
    fn start_job(&mut self, kind: JobKind, then: Option<JobKind>, cx: &mut Context<Self>) {
        // Defensive: one child at a time, whatever the buttons said.
        if let Some(running) = self.running.take() {
            let _ = kill_child(&running.child);
        }

        if then.is_some() {
            self.state.log(format!(
                "[model-test] {} does not exist yet; building it first",
                models_exe().display()
            ));
        }

        let plan = plan_for(kind.clone(), self.state.epochs, self.state.seed);
        let command_line = plan.command_line();
        let (events, mut events_rx) = mpsc::unbounded::<Event>();
        match spawn_job(plan, events) {
            Ok(running) => {
                self.state.begin_job(kind, &command_line);
                self.pending_after_build = then;
                self.running = Some(running);
            }
            Err(reason) => {
                self.state.failed(reason);
                cx.notify();
                return;
            }
        }

        // The threads report on an unbounded channel; the window drains it on
        // GPUI's executor, so no pipe and no `wait` is ever polled here.
        cx.spawn(async move |weak, cx| {
            while let Some(event) = events_rx.next().await {
                let applied = cx.update(|cx| {
                    let Some(view) = weak.upgrade() else {
                        return false;
                    };
                    view.update(cx, |view, cx| view.apply(event, cx));
                    true
                });
                if !applied {
                    break;
                }
            }
        })
        .detach();

        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().stop {
            return;
        }
        let Some(running) = self.running.take() else {
            return;
        };
        self.pending_after_build = None;

        if let Err(error) = kill_child(&running.child) {
            let reason = format!("could not stop `{}`: {error}", running.command_line);
            self.state.failed(reason);
            cx.notify();
            return;
        }
        self.state.stopped(running.kind);
        cx.notify();
    }

    /// A run's `epochs` was picked before starting it.
    fn set_epochs(&mut self, epochs: usize, cx: &mut Context<Self>) {
        if self.state.status.is_busy() || self.state.epochs == epochs {
            return;
        }
        self.state.epochs = epochs;
        self.state
            .log(format!("[model-test] epochs set to {epochs}"));
        cx.notify();
    }

    fn clear_log(&mut self, cx: &mut Context<Self>) {
        self.state.log.clear();
        cx.notify();
    }

    fn copy_log(&mut self, cx: &mut Context<Self>) {
        let text = self.state.log.joined();
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Line { stream, line } => self.state.observe_line(stream, &line),
            Event::Finished { outcome } => {
                self.finish(outcome, cx);
                // `finish` may already have notified for the next job.
                self.scroll.scroll_to_bottom();
                return;
            }
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// The child is gone: move the status on, and chain the auto-build's job.
    fn finish(&mut self, outcome: Result<ExitStatus, String>, cx: &mut Context<Self>) {
        let Some(running) = self.running.take() else {
            // A `Stop` already took the handle; the exit it asked for is
            // reported there, not here.
            return;
        };
        let next = self.pending_after_build.take();

        match outcome {
            Ok(status) if status.success() => {
                if running.kind == JobKind::Build {
                    if let Some(next) = next {
                        self.state.log(format!(
                            "[model-test] build done; starting {}",
                            next.label()
                        ));
                        self.start_job(next, None, cx);
                        return;
                    }
                }
                self.state.succeeded(running.kind);
            }
            Ok(status) => {
                let reason = self.state.describe_failure(running.kind, status.code());
                self.state.failed(reason);
            }
            Err(reason) => self.state.failed(reason),
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }
}

impl Drop for ModelTestView {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            let _ = kill_child(&running.child);
        }
    }
}

impl Render for ModelTestView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = self.state.buttons();
        let status = self.state.status.clone();
        let status_color = status.color();
        let summary = SharedString::from(status.summary());
        let detail = status.detail().map(SharedString::from);
        let command = if self.state.last_command.is_empty() {
            None
        } else {
            Some(SharedString::from(self.state.last_command.clone()))
        };
        let epochs = self.state.epochs;
        let busy = status.is_busy();
        let line_count = self.state.log.len();

        let log_rows: Vec<gpui::AnyElement> = self
            .state
            .log
            .lines()
            .map(|line| {
                Label::new(SharedString::from(line.to_owned()))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .into_any_element()
            })
            .collect();

        let epoch_buttons: Vec<gpui::AnyElement> = [1usize, 3, 10]
            .into_iter()
            .map(|preset| {
                Button::new(
                    ("model-test-epochs", preset),
                    SharedString::from(preset.to_string()),
                )
                .style(if preset == epochs {
                    ButtonStyle::Tinted(TintColor::Accent)
                } else {
                    ButtonStyle::OutlinedGhost
                })
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.set_epochs(preset, cx)))
                .into_any_element()
            })
            .collect();

        // The eight clustering buttons. `DBSCAN` is the linfa implementation and
        // `smartcore` is smartcore's K-Means; the smartcore DBSCAN variant is
        // still reachable through `All` and `models cluster --algo
        // smartcore-dbscan`.
        let cluster_buttons: Vec<gpui::AnyElement> = [
            ("model-test-cluster-kmeans", "K-Means", "kmeans"),
            ("model-test-cluster-linfa-kmeans", "linfa KMeans", "linfa-kmeans"),
            ("model-test-cluster-dbscan", "DBSCAN", "linfa-dbscan"),
            ("model-test-cluster-optics", "OPTICS", "linfa-optics"),
            ("model-test-cluster-gmm", "GMM", "linfa-gmm"),
            ("model-test-cluster-hdbscan", "HDBSCAN", "hdbscan"),
            ("model-test-cluster-smartcore", "smartcore", "smartcore-kmeans"),
            ("model-test-cluster-all", "All", "all"),
        ]
        .into_iter()
        .map(|(id, label, algo)| {
            Button::new(id, label)
                .style(ButtonStyle::Outlined)
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.cluster(algo, cx)))
                .into_any_element()
        })
        .collect();

        v_flex()
            .id("model-test")
            .size_full()
            .gap_3()
            .p_4()
            .bg(cx.theme().colors().editor_background)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(Label::new("Model Test").size(LabelSize::Large))
                    .child(Label::new(summary).color(status_color)),
            )
            .child(
                Label::new(detail.unwrap_or_else(|| SharedString::from("idle")))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new(
                    command
                        .unwrap_or_else(|| SharedString::from("no command has run yet".to_owned())),
                )
                .size(LabelSize::XSmall)
                .color(Color::Hidden),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(
                        Button::new("model-test-build", "Build")
                            .style(ButtonStyle::Outlined)
                            .disabled(!buttons.build)
                            .on_click(cx.listener(|this, _, _, cx| this.build(cx))),
                    )
                    .child(
                        Button::new("model-test-train", "Train")
                            .style(ButtonStyle::Tinted(TintColor::Accent))
                            .disabled(!buttons.train)
                            .on_click(cx.listener(|this, _, _, cx| this.train(cx))),
                    )
                    .child(
                        Button::new("model-test-test", "Test")
                            .style(ButtonStyle::Outlined)
                            .disabled(!buttons.test)
                            .on_click(cx.listener(|this, _, _, cx| this.test(cx))),
                    )
                    .child(
                        Button::new("model-test-stop", "Stop")
                            .style(ButtonStyle::Outlined)
                            .disabled(!buttons.stop)
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Label::new(SharedString::from(format!("epochs: {epochs}")))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .children(epoch_buttons),
                    )
                    .child(
                        Button::new("model-test-clear-log", "Clear log")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!buttons.clear_log)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_log(cx))),
                    )
                    .child(
                        Button::new("model-test-copy-log", "Copy log")
                            .style(ButtonStyle::OutlinedGhost)
                            .disabled(!buttons.copy_log)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_log(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(
                        Label::new("clustering:")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .children(cluster_buttons),
            )
            .child(
                Label::new(SharedString::from(format!(
                    "log: {line_count}/{LOG_CAPACITY} lines (newest last)"
                )))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
            )
            .child(
                div()
                    .id("model-test-log")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded_md()
                    .p_2()
                    .child(v_flex().gap_0p5().children(log_rows)),
            )
            .child(
                Label::new(SharedString::from(format!(
                    "Spawns {} with {} as the working directory; Zed does not link \
                     Burn or the clustering crates. Checkpoint: {}. Closing the window \
                     stops a running job.",
                    models_exe().display(),
                    models_dir().display(),
                    checkpoint_path().display()
                )))
                .size(LabelSize::XSmall)
                .color(Color::Hidden),
            )
    }
}

/// Opens the window, or focuses the one that is already open.
///
/// Rendering needs `theme::GlobalTheme` (`cx.theme()` panics without it). The
/// menu path always runs long after the theme is registered, but the
/// `ZED_MODELTEST_OPEN` startup path can run before that, so wait for it rather
/// than opening a window that cannot draw. Mirrors `zed::admin_demo`.
pub fn open(cx: &mut App) {
    if cx.try_global::<theme::GlobalTheme>().is_none() {
        log::info!("[model-test] waiting for the theme before opening the window");
        cx.spawn(async move |cx| {
            for _ in 0..300 {
                if cx.update(|cx| cx.try_global::<theme::GlobalTheme>().is_some()) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            cx.update(show_window);
        })
        .detach();
        return;
    }

    show_window(cx);
}

/// Opens the window, or focuses it if it is already open.
fn show_window(cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<ModelTestView>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[model-test] could not focus the model test window: {error}");
        }
        return;
    }

    let window_size = size(px(1120.), px(680.));
    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Model Test".into()),
                // The system title bar; the window draws no chrome of its own.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |_window, cx| cx.new(ModelTestView::new).into(),
    );

    match opened {
        Ok(_) => log::info!("[model-test] window opened"),
        Err(error) => log::error!("[model-test] could not open the window: {error}"),
    }
}

/// Registers the startup override, if any. Called once from `zed::init`.
pub fn init(cx: &mut App) {
    if std::env::var(ENV_OPEN).is_ok_and(|value| value != "0") {
        log::info!("[model-test] opening the window at startup ({ENV_OPEN})");
        cx.defer(open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_drops_the_oldest_lines() {
        let mut ring = LogRing::new(3);
        assert!(ring.is_empty());

        for line in ["one", "two", "three"] {
            ring.push(line);
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.lines().collect::<Vec<_>>(), ["one", "two", "three"]);

        ring.push("four");
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.lines().collect::<Vec<_>>(), ["two", "three", "four"]);
        assert_eq!(ring.joined(), "two\nthree\nfour\n");

        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.joined(), "");
    }

    #[test]
    fn the_state_machine_moves_between_its_states() {
        let mut state = ModelTestState::new(3, 42);
        assert_eq!(state.status, Status::Idle);
        assert_eq!(state.status.summary(), "Idle");
        assert!(!state.status.is_busy());

        state.begin_job(JobKind::Build, "cargo build --offline");
        assert_eq!(state.status, Status::Building);
        assert_eq!(state.status.summary(), "Building");
        assert!(state.status.is_busy());
        assert_eq!(state.status.color(), Color::Warning);

        state.succeeded(JobKind::Build);
        assert_eq!(state.status, Status::Idle);

        state.begin_job(JobKind::Train, "models train --epochs 3 --seed 42");
        assert_eq!(state.status, Status::Training);
        state.observe_line(
            Stream::Stdout,
            "epoch 1/3 | train loss 1.0986 | test accuracy 25.00% | 0.2s",
        );
        state.observe_line(Stream::Stdout, "final test accuracy: 87.50%");
        state.succeeded(JobKind::Train);
        assert_eq!(state.status, Status::Done { acc: Some(87.5) });
        assert_eq!(state.status.summary(), "Done");
        assert_eq!(state.status.detail().as_deref(), Some("accuracy 87.50%"));
        assert_eq!(state.status.color(), Color::Success);

        state.begin_job(JobKind::Test, "models test --seed 42");
        assert_eq!(state.status, Status::Testing);
        state.failed("test failed with exit code 1; stderr: checkpoint missing");
        assert!(matches!(state.status, Status::Failed { .. }));
        assert_eq!(state.status.color(), Color::Error);
        assert!(!state.status.is_busy());

        state.begin_job(
            JobKind::Cluster {
                algo: "linfa-dbscan".to_owned(),
            },
            "models.exe cluster --algo linfa-dbscan --points 600 --clusters 3 --seed 42",
        );
        assert_eq!(state.status, Status::Testing);
        state.succeeded(JobKind::Cluster {
            algo: "linfa-dbscan".to_owned(),
        });
        assert_eq!(state.status, Status::Done { acc: None });
        assert_eq!(state.status.detail().as_deref(), Some("finished"));

        state.begin_job(JobKind::Test, "models test --seed 42");
        state.stopped(JobKind::Test);
        assert_eq!(state.status, Status::Idle);

        // Every transition left its own line behind.
        let lines: Vec<&str> = state.log.lines().collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("build in") && line.contains("cargo build"))
        );
        assert!(lines.iter().any(|line| line.contains("build succeeded")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("train in") && line.contains("--epochs 3"))
        );
        assert!(lines.iter().any(|line| line.contains("epoch 1/3")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("train done: accuracy 87.50%"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("failed: test failed"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("cluster in") && line.contains("linfa-dbscan"))
        );
        assert!(lines.iter().any(|line| line.contains("cluster done")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("stopped the test process"))
        );
    }

    #[test]
    fn the_accuracy_parser_covers_every_line_the_demo_prints() {
        assert_eq!(
            accuracy_percent("epoch 1/3 | train loss 1.0986 | test accuracy 25.00% | 0.2s"),
            Some(25.0)
        );
        assert_eq!(accuracy_percent("final test accuracy: 87.50%"), Some(87.5));
        assert_eq!(
            accuracy_percent("test accuracy    : 87.50% (35 / 40)"),
            Some(87.5)
        );
        assert_eq!(
            accuracy_percent("test accuracy    : 100.00% (40 / 40)"),
            Some(100.0)
        );
        assert_eq!(accuracy_percent("parameters       : 3540"), None);
        assert_eq!(accuracy_percent("accuracy: n/a"), None);
        assert_eq!(accuracy_percent(""), None);
    }

    #[test]
    fn a_failure_message_quotes_the_command_and_the_stderr_tail() {
        let mut state = ModelTestState::new(3, 42);
        state.begin_job(JobKind::Test, "models.exe test --seed 42");
        for line in ["one", "two", "three", "four"] {
            state.observe_line(Stream::Stderr, line);
        }
        let reason = state.describe_failure(JobKind::Test, Some(1));
        assert!(reason.starts_with("test failed with exit code 1"));
        // Only the newest three lines survive, and the oldest is dropped.
        assert!(reason.contains("stderr: two / three / four"));
        assert!(!reason.contains("stderr: one"));
        assert!(reason.contains("command: models.exe test --seed 42"));

        let terminated = state.describe_failure(JobKind::Test, None);
        assert!(terminated.starts_with("test was terminated"));
    }

    #[test]
    fn buttons_follow_the_status() {
        let idle = buttons_for(&Status::Idle, true);
        assert_eq!(
            idle,
            Buttons {
                build: true,
                train: true,
                test: true,
                cluster: true,
                stop: false,
                clear_log: false,
                copy_log: false
            }
        );

        for busy in [Status::Building, Status::Training, Status::Testing] {
            let buttons = buttons_for(&busy, false);
            assert!(!buttons.build && !buttons.train && !buttons.test && !buttons.cluster);
            assert!(buttons.stop);
            assert!(buttons.clear_log && buttons.copy_log);
        }

        let done = buttons_for(&Status::Done { acc: Some(90.0) }, true);
        assert!(done.build && done.train && done.test && done.cluster && !done.stop);
        assert!(!done.clear_log && !done.copy_log);

        let failed = buttons_for(
            &Status::Failed {
                reason: "x".to_owned(),
            },
            false,
        );
        assert!(failed.build && failed.train && failed.test && failed.cluster && !failed.stop);
        assert!(failed.clear_log && failed.copy_log);
    }

    #[test]
    fn the_paths_are_derived_from_the_manifest_dir() {
        assert!(repo_root().is_absolute());
        assert!(models_dir().ends_with(Path::new("crates").join("models")));
        assert_eq!(models_dir(), repo_root().join("crates").join("models"));
        assert!(models_exe().starts_with(models_dir()));
        assert!(
            models_exe().ends_with(
                Path::new("target")
                    .join("debug")
                    .join(format!("models{}", std::env::consts::EXE_SUFFIX))
            )
        );
        assert!(checkpoint_path().ends_with(Path::new("artifacts").join("model.bpk")));
    }

    #[test]
    fn the_commands_run_where_the_crate_expects_them() {
        let train = plan_for(JobKind::Train, 2, 42);
        assert_eq!(train.cwd, models_dir());
        assert_eq!(train.program, models_exe().display().to_string());
        assert_eq!(
            train.args,
            vec!["train", "--epochs", "2", "--seed", "42"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            train.command_line(),
            format!(
                "{} train --epochs 2 --seed 42",
                quote(&models_exe().display().to_string())
            )
        );

        let test = plan_for(JobKind::Test, 2, 42);
        assert_eq!(test.cwd, models_dir());
        assert_eq!(
            test.args,
            vec!["test", "--seed", "42"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );

        let build = plan_for(JobKind::Build, 2, 42);
        assert_eq!(build.program, "cargo");
        assert_eq!(build.cwd, models_dir());
        assert_eq!(build.args[0], "build");
        assert_eq!(build.args[1], "--offline");
        assert_eq!(build.args[2], "--manifest-path");
        assert_eq!(
            build.args[3],
            models_dir().join("Cargo.toml").display().to_string()
        );

        let cluster = plan_for(
            JobKind::Cluster {
                algo: "linfa-dbscan".to_owned(),
            },
            2,
            42,
        );
        assert_eq!(cluster.program, models_exe().display().to_string());
        assert_eq!(cluster.cwd, models_dir());
        assert_eq!(
            cluster.args,
            vec![
                "cluster",
                "--algo",
                "linfa-dbscan",
                "--points",
                "600",
                "--clusters",
                "3",
                "--seed",
                "42",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
        assert!(cluster.command_line().contains("cluster --algo linfa-dbscan"));
    }

    #[test]
    fn the_run_hook_parses_the_three_expected_actions() {
        assert_eq!(JobKind::parse("train"), Some(JobKind::Train));
        assert_eq!(JobKind::parse(" TEST "), Some(JobKind::Test));
        assert_eq!(
            JobKind::parse(" CLUSTER "),
            Some(JobKind::Cluster {
                algo: "all".to_owned()
            })
        );
        assert_eq!(JobKind::parse("build"), None);
        assert_eq!(JobKind::parse(""), None);
    }
}
