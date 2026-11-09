//! `Help > Model Test`: train, evaluate and cluster the Burn demo **in process**
//! from a window, and watch its own progress while it runs.
//!
//! The demo's implementation is the root-workspace crate `models_core`
//! (`crates/fundamental/models-core`): the same library the separate
//! `crates/models` workspace's CLI is a thin wrapper over. Zed now links Burn
//! through that crate instead of spawning `models.exe`.
//!
//! ## Lifecycle
//!
//! `Train` / `Test` / each clustering button start an async task with
//! `cx.background_spawn`; **no Burn call ever runs on the UI thread**. The task
//! calls `models_core::train`, `models_core::evaluate` or
//! `models_core::run_cluster` with a progress closure that forwards every line
//! to an unbounded channel, and a `Stop`-visible cancellation token. The window
//! drains the channel on GPUI's executor and appends the lines to [`LogRing`].
//!
//! `Stop` flips the `Arc<AtomicBool>` the training loop checks before every
//! epoch and every mini-batch (and the clustering loop checks between
//! algorithms); closing the window does the same in `Drop`.
//!
//! ## The log
//!
//! The newest [`LOG_CAPACITY`] lines of progress plus this module's own
//! lifecycle lines. Every line is also written through `log::info!` /
//! `log::error!` with a `[model-test]` prefix, so the same sequence is in
//! `Zed.log` when the build writes one.
//!
//! ## Verified shapes
//!
//! The status machine, the ring, the accuracy parser, the path derivation and
//! the job descriptions are pure and are unit-tested below.
//! `ZED_MODELTEST_OPEN=1` opens the window at startup (the menu path without the
//! menu), `ZED_MODELTEST_RUN=train|test|cluster` presses the matching button
//! once the window is up, and `ZED_MODELTEST_EPOCHS` overrides the epoch count
//! for a scripted check. For the cluster hook, `ZED_MODELTEST_ALGO` selects the
//! algorithm (`all` by default).

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
/// handler, so checking that a real in-process training run reaches `Done` does
/// not need a click. Off by default.
pub const ENV_RUN: &str = "ZED_MODELTEST_RUN";

/// `ZED_MODELTEST_ALGO` — the clustering algorithm used by `ZED_MODELTEST_RUN=cluster`.
///
/// Defaults to `all`, which runs the whole comparison table. Any value
/// `models_core::run_cluster` accepts works (`kmeans`, `linfa-kmeans`,
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
/// Mini-batch size and Adam learning rate the window passes to `train`.
const DEFAULT_BATCH_SIZE: usize = 32;
const DEFAULT_LEARNING_RATE: f64 = 3e-3;
/// Points the cluster buttons pass to `run_cluster`.
const DEFAULT_CLUSTER_POINTS: usize = 600;
/// Requested cluster count the cluster buttons pass to `run_cluster`.
const DEFAULT_CLUSTER_CLUSTERS: usize = 3;

/// How long the `ZED_MODELTEST_RUN` hook waits after the window opens before
/// pressing its button, so the window is mapped and the theme is up.
const AUTO_START_DELAY: Duration = Duration::from_millis(400);

/// The repository root: `crates/framework/zed` three levels up.
///
/// `env!("CARGO_MANIFEST_DIR")` is fixed at compile time and is the path this
/// checkout was built from, which is the checkout whose `crates/models`
/// artifacts the window shares with the CLI.
pub fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(3)
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

/// `crates/models/artifacts/model.bpk`: the checkpoint the CLI and this window
/// both read and write.
pub fn checkpoint_path() -> PathBuf {
    repo_root()
        .join("crates")
        .join("models")
        .join("artifacts")
        .join("model.bpk")
}

/// What the window is showing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Nothing is running; `Train`, `Test` and the clustering buttons are
    /// available.
    Idle,
    /// `models_core::train` is running on a background executor.
    Training,
    /// `models_core::evaluate` or `models_core::run_cluster` is running.
    Testing,
    /// The last run succeeded. `acc` is the percentage the run last reported,
    /// when a line could be parsed.
    Done { acc: Option<f32> },
    /// The last start (or the last run) failed; nothing is running.
    Failed { reason: String },
}

impl Status {
    /// The one-line state shown next to the title.
    pub fn summary(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Training => "Training",
            Self::Testing => "Testing",
            Self::Done { .. } => "Done",
            Self::Failed { .. } => "Failed",
        }
    }

    /// The second line: what is happening, the accuracy, or the failure.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Idle => Some("ready: Train, Test or a clustering algorithm".to_owned()),
            Self::Training => {
                Some("training models_core in process (background executor)".to_owned())
            }
            Self::Testing => {
                Some("running models_core evaluation/clustering in process".to_owned())
            }
            Self::Done { acc: Some(acc) } => Some(format!("accuracy {acc:.2}%")),
            Self::Done { acc: None } => Some("finished".to_owned()),
            Self::Failed { reason } => Some(reason.clone()),
        }
    }

    /// The colour of [`Status::summary`].
    pub fn color(&self) -> Color {
        match self {
            Self::Idle => Color::Muted,
            Self::Training | Self::Testing => Color::Warning,
            Self::Done { .. } => Color::Success,
            Self::Failed { .. } => Color::Error,
        }
    }

    /// Whether a background job belongs to this status.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Training | Self::Testing)
    }
}

/// Which buttons are enabled for a status (and a non-empty log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
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
    Train,
    Test,
    /// `models_core::run_cluster`; `algo` is `all` for the comparison table.
    Cluster { algo: String },
}

impl JobKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Test => "test",
            Self::Cluster { .. } => "cluster",
        }
    }

    /// The status the window shows while this job runs.
    ///
    /// Clustering reuses [`Status::Testing`]: it is another models job whose
    /// progress is watched by the same state machine.
    pub fn status(&self) -> Status {
        match self {
            Self::Train => Status::Training,
            Self::Test | Self::Cluster { .. } => Status::Testing,
        }
    }

    /// The in-process call this job makes, for the log and the window footer.
    pub fn description(&self, epochs: usize, seed: u64) -> String {
        match self {
            Self::Train => format!(
                "models_core::train(epochs={epochs}, batch_size={DEFAULT_BATCH_SIZE}, \
                 seed={seed}, checkpoint={})",
                checkpoint_path().display()
            ),
            Self::Test => format!(
                "models_core::evaluate(seed={seed}, checkpoint={})",
                checkpoint_path().display()
            ),
            Self::Cluster { algo } => format!(
                "models_core::run_cluster(algo={algo}, points={DEFAULT_CLUSTER_POINTS}, \
                 clusters={DEFAULT_CLUSTER_CLUSTERS}, seed={seed})"
            ),
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
    /// The call of the current (or last) job, quoted by the footer and failure.
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

    /// A job was accepted: the background task starts and its call is recorded.
    pub fn begin_job(&mut self, kind: JobKind, command_line: &str) {
        self.status = kind.status();
        self.last_acc = None;
        self.last_command = command_line.to_owned();
        self.log(format!(
            "[model-test] {} in process: {command_line}",
            kind.label()
        ));
    }

    /// One progress line: into the ring, and through the accuracy parser.
    pub fn observe_line(&mut self, line: &str) {
        let line = line.trim_end();
        if line.is_empty() {
            return;
        }
        self.log(line.to_owned());
        if let Some(accuracy) = accuracy_percent(line) {
            self.last_acc = Some(accuracy);
        }
    }

    /// The last job succeeded.
    pub fn succeeded(&mut self, kind: JobKind) {
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

    /// The last job failed; nothing is running.
    pub fn failed(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.log_error(format!("[model-test] failed: {reason}"));
        self.status = Status::Failed { reason };
    }

    /// `Stop` cancelled the running job.
    pub fn stopped(&mut self, kind: JobKind) {
        self.status = Status::Idle;
        self.log(format!(
            "[model-test] stopped the {} run (cancelled)",
            kind.label()
        ));
    }

    /// The call of the current (or last) job, for the footer.
    pub fn last_command(&self) -> &str {
        &self.last_command
    }
}

impl Default for ModelTestState {
    fn default() -> Self {
        Self::new(DEFAULT_EPOCHS, DEFAULT_SEED)
    }
}

/// What the background task reports back to the window.
#[derive(Debug)]
enum Event {
    /// One progress line from `models_core`.
    Line(String),
    /// The job ended. `accuracy` is the percentage to show under `Done`;
    /// `cancelled` says whether `Stop` (or window teardown) asked for it.
    Finished {
        job_id: u64,
        kind: JobKind,
        accuracy: Result<Option<f32>, String>,
        cancelled: bool,
    },
}

/// A running background job, shared between the window (`Stop`, `Drop`) and the
/// async task (which checks the token).
struct Running {
    kind: JobKind,
    cancel: Arc<AtomicBool>,
}

/// The window's root view.
pub struct ModelTestView {
    state: ModelTestState,
    running: Option<Running>,
    /// Incremented whenever a job starts or is abandoned; a [`Event::Finished`]
    /// with an older id is stale and ignored.
    job_id: u64,
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
            job_id: 0,
            scroll: ScrollHandle::new(),
        }
    }

    fn train(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().train {
            return;
        }
        self.start_job(JobKind::Train, cx);
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        if !self.state.buttons().test {
            return;
        }
        self.start_job(JobKind::Test, cx);
    }

    /// One of the cluster buttons: `algo` is a `run_cluster` value.
    fn cluster(&mut self, algo: &str, cx: &mut Context<Self>) {
        if !self.state.buttons().cluster {
            return;
        }
        self.start_job(
            JobKind::Cluster {
                algo: algo.to_owned(),
            },
            cx,
        );
    }

    /// Starts `kind` on a background executor.
    ///
    /// Any previous task is cancelled first (defensive: the buttons are disabled
    /// while busy), then the cancellable call is moved off the UI thread. The
    /// window drains progress on GPUI's executor.
    fn start_job(&mut self, kind: JobKind, cx: &mut Context<Self>) {
        if let Some(previous) = self.running.take() {
            previous.cancel.store(true, Ordering::SeqCst);
        }
        self.job_id = self.job_id.wrapping_add(1);
        let job_id = self.job_id;

        let epochs = self.state.epochs;
        let seed = self.state.seed;
        let description = kind.description(epochs, seed);
        self.state.begin_job(kind.clone(), &description);

        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some(Running {
            kind: kind.clone(),
            cancel: cancel.clone(),
        });

        let (events, mut events_rx) = mpsc::unbounded::<Event>();
        let line_events = events.clone();
        let checkpoint = checkpoint_path();
        let task_kind = kind.clone();
        cx.background_spawn(async move {
            let mut progress = |line: &str| {
                let _ = line_events.unbounded_send(Event::Line(line.to_owned()));
            };
            let result: Result<Option<f32>, String> = match &task_kind {
                JobKind::Train => models_core::train(
                    &models_core::TrainArgs::new(
                        epochs,
                        DEFAULT_BATCH_SIZE,
                        seed,
                        DEFAULT_LEARNING_RATE,
                        checkpoint,
                    ),
                    &mut progress,
                    &cancel,
                )
                .map(|summary| Some(summary.final_accuracy * 100.0))
                .map_err(|error| error.message().to_owned()),
                JobKind::Test => models_core::evaluate(
                    &models_core::TestArgs::new(seed, checkpoint),
                    &mut progress,
                )
                .map(|eval| Some(eval.accuracy * 100.0))
                .map_err(|error| error.message().to_owned()),
                JobKind::Cluster { algo } => models_core::run_cluster(
                    Some(algo.as_str()),
                    DEFAULT_CLUSTER_POINTS,
                    DEFAULT_CLUSTER_CLUSTERS,
                    seed,
                    &mut progress,
                    &cancel,
                )
                .map(|_| None)
                .map_err(|error| error.message().to_owned()),
            };
            let cancelled = cancel.load(Ordering::Relaxed);
            let _ = events.unbounded_send(Event::Finished {
                job_id,
                kind: task_kind,
                accuracy: result,
                cancelled,
            });
        })
        .detach();

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
        // The training loop checks this before every batch/epoch; a cluster run
        // checks it between algorithms. Bumping the id makes its eventual
        // `Finished` stale, so it cannot overwrite the `Idle` below.
        running.cancel.store(true, Ordering::SeqCst);
        self.job_id = self.job_id.wrapping_add(1);
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
            Event::Line(line) => self.state.observe_line(&line),
            Event::Finished {
                job_id,
                kind,
                accuracy,
                cancelled,
            } => {
                // A `Stop` (or a newer job) already moved the window on; the
                // task's result is no longer interesting.
                if job_id != self.job_id {
                    return;
                }
                self.running = None;
                if cancelled {
                    self.state.stopped(kind);
                } else {
                    match accuracy {
                        Ok(acc) => {
                            // The progress lines already reported the accuracy;
                            // keep the structured value as a fallback for the
                            // `Done` badge without logging a duplicate line.
                            if let Some(acc) = acc {
                                self.state.last_acc = Some(acc);
                            }
                            self.state.succeeded(kind);
                        }
                        Err(reason) => self.state.failed(reason),
                    }
                }
            }
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }
}

impl Drop for ModelTestView {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.store(true, Ordering::SeqCst);
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
        let command = if self.state.last_command().is_empty() {
            None
        } else {
            Some(SharedString::from(self.state.last_command().to_owned()))
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
        // still reachable through `All` and the `ZED_MODELTEST_ALGO` hook.
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
                    command.unwrap_or_else(|| {
                        SharedString::from("no models_core call has run yet".to_owned())
                    }),
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
                    "Calls models_core::train/evaluate/run_cluster in this process on a \
                     background executor; Zed links Burn through models_core. Checkpoint: {}. \
                     Closing the window cancels a running job.",
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

        state.begin_job(JobKind::Train, "models_core::train(epochs=3, seed=42)");
        assert_eq!(state.status, Status::Training);
        assert_eq!(state.status.summary(), "Training");
        assert!(state.status.is_busy());
        assert_eq!(state.status.color(), Color::Warning);

        state.observe_line("epoch 1/3 | train loss 1.0986 | test accuracy 25.00% | 0.2s");
        state.observe_line("final test accuracy: 87.50%");
        state.succeeded(JobKind::Train);
        assert_eq!(state.status, Status::Done { acc: Some(87.5) });
        assert_eq!(state.status.summary(), "Done");
        assert_eq!(state.status.detail().as_deref(), Some("accuracy 87.50%"));
        assert_eq!(state.status.color(), Color::Success);

        state.begin_job(JobKind::Test, "models_core::evaluate(seed=42)");
        assert_eq!(state.status, Status::Testing);
        state.failed("test failed: checkpoint missing");
        assert!(matches!(state.status, Status::Failed { .. }));
        assert_eq!(state.status.color(), Color::Error);
        assert!(!state.status.is_busy());

        state.begin_job(
            JobKind::Cluster {
                algo: "linfa-dbscan".to_owned(),
            },
            "models_core::run_cluster(algo=linfa-dbscan, seed=42)",
        );
        assert_eq!(state.status, Status::Testing);
        state.succeeded(JobKind::Cluster {
            algo: "linfa-dbscan".to_owned(),
        });
        assert_eq!(state.status, Status::Done { acc: None });
        assert_eq!(state.status.detail().as_deref(), Some("finished"));

        state.begin_job(JobKind::Test, "models_core::evaluate(seed=42)");
        state.stopped(JobKind::Test);
        assert_eq!(state.status, Status::Idle);

        // Every transition left its own line behind.
        let lines: Vec<&str> = state.log.lines().collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("train in process") && line.contains("epochs=3"))
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
                .any(|line| line.contains("cluster in process") && line.contains("linfa-dbscan"))
        );
        assert!(lines.iter().any(|line| line.contains("cluster done")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("stopped the test run"))
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
    fn a_failure_is_logged_and_shown() {
        let mut state = ModelTestState::new(3, 42);
        state.begin_job(JobKind::Test, "models_core::evaluate(seed=42)");
        state.failed("checkpoint missing; run `models train` first");
        assert!(matches!(state.status, Status::Failed { .. }));
        assert_eq!(
            state.status.detail().as_deref(),
            Some("checkpoint missing; run `models train` first")
        );
        let lines: Vec<&str> = state.log.lines().collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("failed: checkpoint missing"))
        );
        assert_eq!(
            state.last_command(),
            "models_core::evaluate(seed=42)"
        );
    }

    #[test]
    fn buttons_follow_the_status() {
        let idle = buttons_for(&Status::Idle, true);
        assert_eq!(
            idle,
            Buttons {
                train: true,
                test: true,
                cluster: true,
                stop: false,
                clear_log: false,
                copy_log: false
            }
        );

        for busy in [Status::Training, Status::Testing] {
            let buttons = buttons_for(&busy, false);
            assert!(!buttons.train && !buttons.test && !buttons.cluster);
            assert!(buttons.stop);
            assert!(buttons.clear_log && buttons.copy_log);
        }

        let done = buttons_for(&Status::Done { acc: Some(90.0) }, true);
        assert!(done.train && done.test && done.cluster && !done.stop);
        assert!(!done.clear_log && !done.copy_log);

        let failed = buttons_for(
            &Status::Failed {
                reason: "x".to_owned(),
            },
            false,
        );
        assert!(failed.train && failed.test && failed.cluster && !failed.stop);
        assert!(failed.clear_log && failed.copy_log);
    }

    #[test]
    fn the_paths_are_derived_from_the_manifest_dir() {
        assert!(repo_root().is_absolute());
        assert_eq!(
            checkpoint_path(),
            repo_root()
                .join("crates")
                .join("models")
                .join("artifacts")
                .join("model.bpk")
        );
        assert!(checkpoint_path().ends_with(Path::new("artifacts").join("model.bpk")));
    }

    #[test]
    fn the_jobs_describe_the_in_process_calls() {
        let train = JobKind::Train.description(2, 42);
        assert!(train.contains("models_core::train(epochs=2"));
        assert!(train.contains("seed=42"));
        assert!(train.contains("model.bpk"));

        let test = JobKind::Test.description(2, 42);
        assert!(test.contains("models_core::evaluate(seed=42"));

        let cluster = JobKind::Cluster {
            algo: "linfa-dbscan".to_owned(),
        }
        .description(2, 42);
        assert!(cluster.contains("models_core::run_cluster(algo=linfa-dbscan"));
        assert!(cluster.contains("points=600"));
        assert!(cluster.contains("clusters=3"));
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
