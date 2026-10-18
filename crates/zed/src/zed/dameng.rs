//! The Dameng (达梦) test-data window: `Help > Dameng`.
//!
//! The Dameng driver stack was merged into this workspace whole
//! (`crates/dameng/*`, seven members — `dm-error`, `dm-core`, `dm-macros`,
//! `dm-odbc`, `dm-dpi`, `dm-pool`, `dm-query`). It had no consumer here: those
//! crates have their own integration tests, but nothing in Zed connected to a
//! real instance, so "the merge compiles" was the only thing the workspace
//! proved. This window is that consumer. It is deliberately a *test* surface,
//! not an editor feature: it configures a connection, seeds a table of rows and
//! runs arbitrary SQL, printing what came back, so a Dameng instance can be
//! exercised from the running app on the machine it is deployed on.
//!
//! ## Why a window rather than a settings page
//!
//! The values (server, port, user, password, schema, table, row count) are
//! local to a throwaway test run and change from run to run. A settings entry
//! would make them persistent, schema'd and validated, and the run's *output* —
//! the part that says whether the driver actually works — has nowhere to go in
//! a settings page. A window can own both: the form and the transcript of what
//! happened. It is also reachable without a project open, which is the state a
//! host is in when it is checking that the Dameng stack links and connects.
//!
//! The configuration is still remembered across launches, in
//! `dameng_test.json` under [`paths::config_dir`], so a working connection does
//! not have to be retyped. It is written by hand here rather than declared as a
//! settings schema because nothing but this window reads it.
//!
//! ## Where the work happens
//!
//! The driver is synchronous per connection: `DmPool` hands out `PooledConn`s
//! that block a thread for the duration of a query, and `connect_with` wants a
//! Tokio runtime handle of its own. Zed's executor is smol-based, and running
//! blocking ODBC/DPI calls on it would stall the UI. So each job runs on a
//! `std::thread` with a one-worker Tokio runtime built inside it, and the result
//! comes back over a `oneshot` channel to be rendered. The window stays
//! responsive while a query runs, and `running` disables the buttons so two
//! jobs cannot overlap on the same form.
//!
//! Both backends are offered because the merged stack has two and they fail
//! differently: [`dm_odbc::OdbcBackend`] goes through the ODBC driver manager
//! (and needs a registered `DM8 ODBC DRIVER`), while [`dm_dpi::DpiBackend`]
//! loads `dmdpi.dll` at runtime and talks to it directly. Which one is
//! configured on a given machine is exactly the thing a test window is for.
//!
//! ## Keyboard
//!
//! The form is text drawn by hand, not a set of `TextInput`s — Zed's UI crate
//! has none, and the GPUI Kit's `Input` needs a window root this window does not
//! install. The root element tracks focus and handles keys, so the window
//! behaves like a form: `Tab`/`Shift-Tab` move between fields, `Enter` runs the
//! job that matches the selected field (SQL runs the statement, anything else
//! tests the connection), `Backspace` deletes, and everything else that is a
//! printable character is appended. Clicking a row selects it.

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext as _, Context, FocusHandle, KeyDownEvent, Render, SharedString, Size,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, px,
};
use release_channel::ReleaseChannel;
use serde::{Deserialize, Serialize};
use theme::ActiveTheme as _;
use ui::{Button, ButtonStyle, Color, Label, LabelSize, TintColor, prelude::*};

/// The configuration file, relative to [`paths::config_dir`].
const CONFIG_FILE: &str = "dameng_test.json";

/// How many transcript lines the window keeps on screen.
const VISIBLE_LOG: usize = 18;

/// How many transcript lines are kept at all. Long enough to read a seed run,
/// short enough that a chatty job cannot grow the window without bound.
const MAX_LOG: usize = 400;

/// How many rows of a `select` are printed. A test query that returns a whole
/// table would otherwise fill the transcript and hide the point of it.
const VISIBLE_ROWS: usize = 50;

/// The single-line fields of the form, in the order `Tab` visits them.
///
/// [`Field::Sql`] is the one multiline field and is visited last; it is kept out
/// of this list because it is laid out differently.
const FIELDS: [Field; 8] = [
    Field::Driver,
    Field::Server,
    Field::Port,
    Field::User,
    Field::Password,
    Field::Schema,
    Field::Table,
    Field::Rows,
];

/// One editable value in the window's form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Driver,
    Server,
    Port,
    User,
    Password,
    Schema,
    Table,
    Rows,
    /// The statement run by `执行 SQL`.
    Sql,
}

impl Field {
    /// The field's label in the form.
    fn label(self) -> &'static str {
        match self {
            Field::Driver => "驱动",
            Field::Server => "服务器",
            Field::Port => "端口",
            Field::User => "用户名",
            Field::Password => "密码",
            Field::Schema => "模式",
            Field::Table => "表名",
            Field::Rows => "行数",
            Field::Sql => "SQL",
        }
    }

    /// The hint shown while the field is empty. Hints carry the defaults the
    /// driver would use, so an empty field is never ambiguous.
    fn hint(self) -> &'static str {
        match self {
            Field::Driver => "DM8 ODBC DRIVER（仅 ODBC 后端使用）",
            Field::Server => "127.0.0.1",
            Field::Port => "5236",
            Field::User => "SYSDBA",
            Field::Password => "SYSDBA",
            Field::Schema => "留空则用连接的默认模式",
            Field::Table => "ZED_DAMENG_TEST",
            Field::Rows => "32",
            Field::Sql => "select 1 from dual",
        }
    }

    /// Whether the value is echoed back as dots.
    fn secret(self) -> bool {
        matches!(self, Field::Password)
    }
}

/// What a run should do once the connection is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// Connect, run `select 1 from dual`, and read the server banner.
    TestConnection,
    /// Create, clear and refill the test table with `Rows` rows.
    Seed,
    /// Run whatever is in the SQL field.
    RunSql,
}

impl Job {
    fn label(self) -> &'static str {
        match self {
            Job::TestConnection => "连接测试",
            Job::Seed => "生成测试数据",
            Job::RunSql => "执行 SQL",
        }
    }
}

/// One line of the transcript.
#[derive(Clone)]
struct LogLine {
    text: SharedString,
    kind: LogKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogKind {
    /// Context and results.
    Info,
    /// A step that succeeded, worth colouring but not worth a headline.
    Done,
    /// Something failed. Any error in a job makes the job's last line say so.
    Error,
}

/// The form's values, as keystrokes left them.
///
/// Everything is a `String`, including the two numbers, so that a half-typed
/// port survives a redraw and a bad value is reported by [`parse`] rather than
/// silently becoming `0`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    /// Use `dmdpi.dll` (DPI) instead of the ODBC driver manager.
    dpi: bool,
    driver: String,
    server: String,
    port: String,
    user: String,
    password: String,
    schema: String,
    table: String,
    rows: String,
    sql: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dpi: false,
            driver: dm_core::DEFAULT_DRIVER.to_owned(),
            server: "127.0.0.1".to_owned(),
            port: dm_core::DEFAULT_PORT.to_string(),
            user: "SYSDBA".to_owned(),
            password: "SYSDBA".to_owned(),
            schema: String::new(),
            table: "ZED_DAMENG_TEST".to_owned(),
            rows: "32".to_owned(),
            sql: "select 1 from dual".to_owned(),
        }
    }
}

impl Config {
    /// Where the configuration is remembered.
    fn path() -> std::path::PathBuf {
        paths::config_dir().join(CONFIG_FILE)
    }

    /// Reads the remembered configuration, falling back to the defaults.
    ///
    /// A missing or unreadable file is not an error worth showing: the window's
    /// defaults are a working local connection, and a corrupted file is
    /// overwritten on the next save.
    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| match serde_json::from_str(&text) {
                Ok(config) => Some(config),
                Err(error) => {
                    log::warn!("[dameng] ignoring unreadable {}: {error}", Self::path().display());
                    None
                }
            })
            .unwrap_or_default()
    }

    /// Remembers the current values.
    fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            log::warn!("[dameng] could not create {}: {error}", parent.display());
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(error) = std::fs::write(&path, text) {
                    log::warn!("[dameng] could not write {}: {error}", path.display());
                }
            }
            Err(error) => log::warn!("[dameng] could not encode the configuration: {error}"),
        }
    }

    /// The value of `field`, or the SQL field for [`Field::Sql`].
    fn field(&self, field: Field) -> &str {
        match field {
            Field::Driver => &self.driver,
            Field::Server => &self.server,
            Field::Port => &self.port,
            Field::User => &self.user,
            Field::Password => &self.password,
            Field::Schema => &self.schema,
            Field::Table => &self.table,
            Field::Rows => &self.rows,
            Field::Sql => &self.sql,
        }
    }

    /// The value of `field`, mutably.
    fn field_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Driver => &mut self.driver,
            Field::Server => &mut self.server,
            Field::Port => &mut self.port,
            Field::User => &mut self.user,
            Field::Password => &mut self.password,
            Field::Schema => &mut self.schema,
            Field::Table => &mut self.table,
            Field::Rows => &mut self.rows,
            Field::Sql => &mut self.sql,
        }
    }
}

/// The form's values, checked and converted.
///
/// Built once when a job starts, so the worker thread never has to look at the
/// form again and a bad port is a message in the transcript rather than a panic
/// in the driver.
#[derive(Clone, Debug)]
struct Params {
    dpi: bool,
    driver: String,
    server: String,
    port: u16,
    user: String,
    password: String,
    schema: String,
    table: String,
    rows: usize,
    sql: String,
}

/// Checks the form and converts the two numbers.
fn parse(config: &Config) -> Result<Params, String> {
    let server = config.server.trim();
    if server.is_empty() {
        return Err("服务器不能为空".to_owned());
    }

    let port = config
        .port
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("端口 `{}` 不是 1..=65535 之间的数字", config.port.trim()))?;
    if port == 0 {
        return Err("端口不能为 0".to_owned());
    }

    let rows = config
        .rows
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("行数 `{}` 不是数字", config.rows.trim()))?;
    if rows == 0 || rows > 10_000 {
        return Err("行数必须在 1..=10000 之间".to_owned());
    }

    if config.table.trim().is_empty() {
        return Err("表名不能为空".to_owned());
    }

    Ok(Params {
        dpi: config.dpi,
        driver: config.driver.trim().to_owned(),
        server: server.to_owned(),
        port,
        user: config.user.trim().to_owned(),
        password: config.password.clone(),
        schema: config.schema.trim().to_owned(),
        table: config.table.trim().to_owned(),
        rows,
        sql: config.sql.clone(),
    })
}

/// What a finished job produced: the lines to append, and whether anything in
/// them was an error.
struct Outcome {
    lines: Vec<LogLine>,
    failed: bool,
}

impl Outcome {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            failed: false,
        }
    }

    /// A job that never started, for when the worker itself went away.
    fn failure(message: impl Into<String>) -> Self {
        let mut outcome = Self::new();
        outcome.fail(message.into());
        outcome
    }

    fn info(&mut self, text: impl Into<SharedString>) {
        self.lines.push(LogLine {
            text: text.into(),
            kind: LogKind::Info,
        });
    }

    fn done(&mut self, text: impl Into<SharedString>) {
        self.lines.push(LogLine {
            text: text.into(),
            kind: LogKind::Done,
        });
    }

    fn fail(&mut self, text: impl Into<SharedString>) {
        self.failed = true;
        self.lines.push(LogLine {
            text: text.into(),
            kind: LogKind::Error,
        });
    }
}

/// The window's root view.
pub struct DamengWindow {
    config: Config,
    focus: FocusHandle,
    /// The field keystrokes go to.
    focused: Field,
    log: Vec<LogLine>,
    /// A job is on the worker thread. Buttons are disabled while it is.
    running: bool,
}

impl DamengWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            config: Config::load(),
            focus: cx.focus_handle(),
            focused: Field::Server,
            log: Vec::new(),
            running: false,
        }
    }

    /// Appends a line to the transcript, dropping the oldest ones past
    /// [`MAX_LOG`].
    fn push_line(&mut self, kind: LogKind, text: impl Into<SharedString>) {
        self.log.push(LogLine {
            text: text.into(),
            kind,
        });
        if self.log.len() > MAX_LOG {
            let excess = self.log.len() - MAX_LOG;
            self.log.drain(..excess);
        }
    }

    /// Selects `field` and gives it the keyboard.
    fn select(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        self.focused = field;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Moves to the previous or next field, wrapping around.
    fn select_relative(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order: Vec<Field> = FIELDS.iter().copied().chain([Field::Sql]).collect();
        let current = order
            .iter()
            .position(|field| *field == self.focused)
            .unwrap_or(0);
        let next = if backwards {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        self.select(order[next], window, cx);
    }

    /// Appends typed text to the selected field.
    fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.config.field_mut(self.focused).push_str(text);
        cx.notify();
    }

    /// Deletes the last character of the selected field.
    fn delete_last(&mut self, cx: &mut Context<Self>) {
        self.config.field_mut(self.focused).pop();
        cx.notify();
    }

    /// Appends the outcome of a finished job and re-enables the buttons.
    fn finish(&mut self, job: Job, outcome: Outcome, elapsed: Duration, cx: &mut Context<Self>) {
        self.running = false;
        let failed = outcome.failed;
        let lines = outcome.lines.len();
        for line in outcome.lines {
            self.push_line(line.kind, line.text);
        }
        let seconds = elapsed.as_secs_f64();
        if failed {
            self.push_line(
                LogKind::Error,
                SharedString::from(format!("{} 失败（{seconds:.1}s，{lines} 行输出）", job.label())),
            );
        } else {
            self.push_line(
                LogKind::Done,
                SharedString::from(format!("{} 完成（{seconds:.1}s，{lines} 行输出）", job.label())),
            );
        }
        cx.notify();
    }

    /// Runs `job` on a worker thread.
    ///
    /// The pool, the connection and the queries all live on that thread's
    /// runtime; what crosses back is one [`Outcome`]. Nothing here blocks the UI
    /// thread, which is the entire reason for the thread: a hung connection must
    /// not hang Zed.
    fn run(&mut self, job: Job, cx: &mut Context<Self>) {
        if self.running {
            self.push_line(LogKind::Info, "上一个任务还在运行，等它结束");
            cx.notify();
            return;
        }

        let params = match parse(&self.config) {
            Ok(params) => params,
            Err(error) => {
                self.push_line(LogKind::Error, error);
                cx.notify();
                return;
            }
        };

        // Remember what is on screen now, not what was there when it last
        // worked: the configuration that runs is the configuration that is
        // restored.
        self.config.save();

        self.running = true;
        let backend = if params.dpi {
            "DPI（dmdpi.dll）"
        } else {
            "ODBC（驱动管理器）"
        };
        self.push_line(
            LogKind::Info,
            SharedString::from(format!(
                "{}：{} 后端，{}:{}，用户 {}",
                job.label(),
                backend,
                params.server,
                params.port,
                params.user
            )),
        );
        cx.notify();

        let started = Instant::now();
        // `std::sync::mpsc` rather than a futures channel: the worker thread is
        // not on an executor, and the receiving end only ever needs one value.
        let (sender, receiver) = mpsc::channel::<Outcome>();
        let spawned = std::thread::Builder::new()
            .name("dameng-test".to_owned())
            .spawn(move || {
                let outcome = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(run_job(params, job)),
                    Err(error) => Outcome::failure(format!("无法启动运行时：{error}")),
                };
                // The receiver is dropped if the window went away; that is not
                // an error, so the send result is ignored.
                let _ = sender.send(outcome);
            });

        if let Err(error) = spawned {
            self.running = false;
            self.push_line(LogKind::Error, SharedString::from(format!("无法启动工作线程：{error}")));
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            // Wait on the thread without blocking the UI thread: `recv` on a
            // channel with a sender that is alive is what the executor parks
            // on, so the window keeps drawing while the query runs.
            let outcome = blocking_recv(receiver)
                .await
                .unwrap_or_else(|| Outcome::failure("工作线程没有返回结果"));
            this.update(cx, |this, cx| this.finish(job, outcome, started.elapsed(), cx))
                .ok();
        })
        .detach();
    }
}

/// Awaits a `std::sync::mpsc` receiver from a gpui task.
///
/// `std::sync::mpsc::Receiver` is blocking, and `recv` on the UI thread would
/// stall it — which is the failure mode this window exists to avoid. The
/// recommended shape is a `oneshot`, but its error type says nothing about the
/// job, so the channel is awaited on a background task instead and its result
/// handed over. `futures::channel::oneshot` would be the direct answer; it is
/// not used because `mpsc` keeps the sender typed as the job's own outcome
/// without a second wrapper.
async fn blocking_recv(receiver: mpsc::Receiver<Outcome>) -> Option<Outcome> {
    // `smol::unblock` moves the blocking receive onto the blocking pool and
    // awaits the value, so nothing blocks the executor's thread.
    smol::unblock(move || receiver.recv().ok()).await
}

/// Runs `job` against `params` on the calling runtime, with the backend chosen
/// by the form.
async fn run_job(params: Params, job: Job) -> Outcome {
    if params.dpi {
        run_with::<dm_dpi::DpiBackend>(&params, job).await
    } else {
        run_with::<dm_odbc::OdbcBackend>(&params, job).await
    }
}

/// Builds the configuration, connects, runs `job`, and always closes the pool.
///
/// The pool is closed on every path, including the failure ones: `DmPool` keeps
/// maintenance tasks and one blocking thread per idle connection alive, and a
/// test window that leaked a pool per press would accumulate threads.
async fn run_with<B: dm_core::Backend>(params: &Params, job: Job) -> Outcome {
    let mut outcome = Outcome::new();

    let mut builder = dm_core::DmConfig::builder()
        .driver(params.driver.clone())
        .server(params.server.clone())
        .port(params.port)
        .user(params.user.clone())
        .password(params.password.clone())
        // One connection is all a test window needs, and it keeps the pool from
        // opening more if the driver decides to.
        .pool_max_size(1);
    if !params.schema.is_empty() {
        builder = builder.schema(params.schema.clone());
    }
    let config = match builder.build() {
        Ok(config) => config,
        Err(error) => {
            outcome.fail(format!("配置被拒绝：{error}"));
            return outcome;
        }
    };

    let started = Instant::now();
    let pool = match dm_pool::DmPool::<B>::connect_with(
        config,
        dm_pool::ExecutorConfig::for_pool(1),
    )
    .await
    {
        Ok(pool) => pool,
        Err(error) => {
            outcome.fail(format!("连接失败：{}", one_line(&error.to_string())));
            return outcome;
        }
    };
    outcome.done(format!("已连接（{} ms）", started.elapsed().as_millis()));

    let result = match job {
        Job::TestConnection => test_connection(&pool, &mut outcome).await,
        Job::Seed => seed(&pool, params, &mut outcome).await,
        Job::RunSql => run_sql(&pool, &params.sql, &mut outcome).await,
    };
    if let Err(error) = result {
        outcome.fail(error);
    }

    // Close before returning: the pool's tasks and blocking threads are owned by
    // it, and `wait_closed` is what makes sure they are gone before this
    // runtime is dropped.
    if let Err(error) = pool.close().await {
        outcome.fail(format!("关闭连接池失败：{}", one_line(&error.to_string())));
    }
    pool.wait_closed().await;
    outcome
}

/// `select 1 from dual`, then the server's own version banner.
async fn test_connection<B: dm_core::Backend>(
    pool: &dm_pool::DmPool<B>,
    outcome: &mut Outcome,
) -> Result<(), String> {
    let mut connection = dm_query::PooledConn::<B>::acquire(pool)
        .await
        .map_err(|error| format!("取连接失败：{}", one_line(&error.to_string())))?;

    let started = Instant::now();
    let rows = connection
        .query("select 1 from dual", ())
        .await
        .map_err(|error| format!("`select 1 from dual` 失败：{}", one_line(&error.to_string())))?;
    let first = rows.first().map(row_text).unwrap_or_else(|| "（无行）".to_owned());
    outcome.done(format!(
        "select 1 from dual → {first}（{} ms）",
        started.elapsed().as_millis()
    ));

    // Not fatal: `v$version` needs a privilege that a test login may not have,
    // and the connection is already proven by the line above.
    match connection.query("select banner from v$version", ()).await {
        Ok(rows) => {
            for row in rows.rows().iter().take(3) {
                outcome.info(row_text(row));
            }
        }
        Err(error) => outcome.info(format!(
            "读不到 v$version（不影响连接）：{}",
            one_line(&error.to_string())
        )),
    }

    Ok(())
}

/// Creates the test table if it is not there, clears it, and refills it.
async fn seed<B: dm_core::Backend>(
    pool: &dm_pool::DmPool<B>,
    params: &Params,
    outcome: &mut Outcome,
) -> Result<(), String> {
    let table = &params.table;
    let mut connection = dm_query::PooledConn::<B>::acquire(pool)
        .await
        .map_err(|error| format!("取连接失败：{}", one_line(&error.to_string())))?;

    // Creating a table that already exists is an error in Dameng, and a second
    // run of this window must not fail because the first one worked.
    let create = format!(
        "create table {table} (id int primary key, name varchar(64), note varchar(256))"
    );
    match connection.query(&create, ()).await {
        Ok(_) => outcome.done(format!("已建表 {table}")),
        Err(error) => outcome.info(format!(
            "建表跳过（可能已存在）：{}",
            one_line(&error.to_string())
        )),
    }

    match connection.query(&format!("delete from {table}"), ()).await {
        Ok(rows) => outcome.done(format!(
            "已清空 {table}（{} 行）",
            rows.affected().unwrap_or_default()
        )),
        Err(error) => {
            return Err(format!(
                "清空 {table} 失败：{}",
                one_line(&error.to_string())
            ));
        }
    }

    let insert = format!("insert into {table} (id, name, note) values (?, ?, ?)");
    let started = Instant::now();
    let mut inserted = 0usize;
    for index in 0..params.rows {
        let id = index as i64 + 1;
        let name = format!("row-{id:05}");
        let note = format!("zed dameng test row {id}");
        match connection.execute(&insert, (id, name, note)).await {
            Ok(_) => inserted += 1,
            Err(error) => {
                outcome.fail(format!(
                    "插入第 {id} 行失败：{}",
                    one_line(&error.to_string())
                ));
                break;
            }
        }
    }
    outcome.done(format!(
        "已插入 {inserted}/{} 行（{} ms）",
        params.rows,
        started.elapsed().as_millis()
    ));

    let count = connection
        .query(&format!("select count(*) from {table}"), ())
        .await
        .map_err(|error| format!("统计行数失败：{}", one_line(&error.to_string())))?;
    outcome.info(format!(
        "select count(*) → {}",
        count.first().map(row_text).unwrap_or_else(|| "（无行）".to_owned())
    ));

    let sample = connection
        .query(&format!("select id, name from {table} order by id"), ())
        .await
        .map_err(|error| format!("读回数据失败：{}", one_line(&error.to_string())))?;
    outcome.info(format!(
        "读回 {} 行，首行 → {}",
        sample.len(),
        sample
            .first()
            .map(row_text)
            .unwrap_or_else(|| "（无行）".to_owned())
    ));

    Ok(())
}

/// Runs whatever is in the SQL field.
async fn run_sql<B: dm_core::Backend>(
    pool: &dm_pool::DmPool<B>,
    sql: &str,
    outcome: &mut Outcome,
) -> Result<(), String> {
    let statement = sql.trim();
    if statement.is_empty() {
        return Err("SQL 框是空的".to_owned());
    }

    let mut connection = dm_query::PooledConn::<B>::acquire(pool)
        .await
        .map_err(|error| format!("取连接失败：{}", one_line(&error.to_string())))?;

    let started = Instant::now();
    let rows = connection
        .query(statement, ())
        .await
        .map_err(|error| one_line(&error.to_string()))?;

    if let Some(affected) = rows.affected() {
        outcome.done(format!("{affected} 行受影响（{} ms）", started.elapsed().as_millis()));
    } else {
        outcome.done(format!(
            "{} 行（{} ms）",
            rows.len(),
            started.elapsed().as_millis()
        ));
    }

    if !rows.columns().is_empty() {
        outcome.info(
            rows.columns()
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        );
        for row in rows.rows().iter().take(VISIBLE_ROWS) {
            outcome.info(row_text(row));
        }
        if rows.len() > VISIBLE_ROWS {
            outcome.info(format!(
                "… 还有 {} 行未显示",
                rows.len().saturating_sub(VISIBLE_ROWS)
            ));
        }
    }

    Ok(())
}

/// One row as `a | b | c`.
fn row_text(row: &dm_query::Row) -> String {
    row.values()
        .iter()
        .map(value_text)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// One value as it should read in the transcript.
///
/// The interesting cases are spelled out; everything else falls back to `Debug`
/// so a variant added to the driver shows up as itself instead of being
/// silently blanked. `Debug` keeps this compiling without naming the
/// chrono/decimal payload types, which are behind features.
fn value_text(value: &dm_core::DmValue) -> String {
    match value {
        dm_core::DmValue::Null => "NULL".to_owned(),
        dm_core::DmValue::Text(text) => flatten(text),
        dm_core::DmValue::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        other => format!("{other:?}"),
    }
}

/// Collapses a value onto one line and shortens it.
///
/// The transcript is a list of one-line labels; a `varchar` holding newlines
/// would otherwise reflow the window.
fn flatten(text: &str) -> String {
    let mut flattened = String::with_capacity(text.len());
    for character in text.chars().take(400) {
        match character {
            '\n' | '\r' => flattened.push('⏎'),
            '\t' => flattened.push(' '),
            other => flattened.push(other),
        }
    }
    if text.chars().nth(400).is_some() {
        flattened.push('…');
    }
    flattened
}

/// Strips the multi-line shape of a driver error down to one line, since it is
/// printed inline in the transcript.
fn one_line(error: &str) -> String {
    error
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Draws the form and the transcript.
impl Render for DamengWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = cx.theme().colors().border_focused;
        let border = cx.theme().colors().border;
        let field_background = cx.theme().colors().element_background;
        let pressed_background = cx.theme().colors().element_hover;

        let mut fields = Vec::with_capacity(FIELDS.len());
        for field in FIELDS {
            let selected = self.focused == field;
            let raw = self.config.field(field);
            let empty = raw.is_empty();
            let shown = if empty {
                SharedString::from(field.hint())
            } else if field.secret() {
                SharedString::from("•".repeat(raw.chars().count().min(32)))
            } else {
                SharedString::from(flatten(raw))
            };
            fields.push(
                h_flex()
                    .id(SharedString::from(format!("dameng-field-{}", field.label())))
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(px(52.))
                            .child(Label::new(field.label()).color(Color::Muted).size(LabelSize::XSmall)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(if selected { accent } else { border })
                            .bg(if selected { pressed_background } else { field_background })
                            .child(
                                Label::new(shown)
                                    .color(if empty { Color::Placeholder } else { Color::Default })
                                    .size(LabelSize::Small),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(field, window, cx)))
                    .into_any_element(),
            );
        }

        let sql_selected = self.focused == Field::Sql;
        let sql_shown: SharedString = if self.config.sql.is_empty() {
            SharedString::from(Field::Sql.hint())
        } else {
            SharedString::from(format!("{}▌", self.config.sql))
        };

        let running = self.running;

        let mut transcript = self
            .log
            .iter()
            .rev()
            .take(VISIBLE_LOG)
            .rev()
            .map(|line| {
                Label::new(line.text.clone())
                    .color(match line.kind {
                        LogKind::Info => Color::Muted,
                        LogKind::Done => Color::Success,
                        LogKind::Error => Color::Error,
                    })
                    .size(LabelSize::XSmall)
            })
            .collect::<Vec<_>>();
        if transcript.is_empty() {
            transcript.push(
                Label::new("还没有运行过任何任务：先「测试连接」，再「生成测试数据」")
                    .color(Color::Muted)
                    .size(LabelSize::XSmall),
            );
        }

        let backend = if self.config.dpi { "DPI" } else { "ODBC" };
        let status = if running { "运行中…" } else { "空闲" };

        v_flex()
            .id("dameng-window")
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().editor_background)
            .text_color(cx.theme().colors().text)
            .p_4()
            .gap_3()
            // The root owns the keyboard, the way the Octos console does: key
            // events bubble up from whatever was clicked, so typing keeps
            // working after a button has taken focus.
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if keystroke.modifiers.control
                    || keystroke.modifiers.alt
                    || keystroke.modifiers.platform
                {
                    return;
                }
                match keystroke.key.as_str() {
                    "tab" => this.select_relative(keystroke.modifiers.shift, window, cx),
                    "enter" => {
                        let job = if this.focused == Field::Sql {
                            Job::RunSql
                        } else {
                            Job::TestConnection
                        };
                        this.run(job, cx);
                    }
                    "backspace" => this.delete_last(cx),
                    _ => {
                        if let Some(text) = keystroke.key_char.as_ref() {
                            this.type_text(text, cx);
                        }
                    }
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Headline::new("Dameng"))
                            .child(
                                Label::new("达梦测试数据")
                                    .color(Color::Muted)
                                    .size(LabelSize::XSmall),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Label::new("后端")
                                    .color(Color::Muted)
                                    .size(LabelSize::XSmall),
                            )
                            .child(
                                Button::new("dameng-backend", backend)
                                    .style(ButtonStyle::OutlinedGhost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.config.dpi = !this.config.dpi;
                                        let backend = if this.config.dpi {
                                            "DPI（dmdpi.dll）：驱动字段不再使用"
                                        } else {
                                            "ODBC（驱动管理器）：需要使用已注册的 ODBC 驱动"
                                        };
                                        this.push_line(LogKind::Info, backend);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Label::new(status)
                                    .color(if running { Color::Accent } else { Color::Muted })
                                    .size(LabelSize::XSmall),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("连接")
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    )
                    .children(fields),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("SQL")
                            .color(Color::Muted)
                            .size(LabelSize::XSmall),
                    )
                    .child(
                        div()
                            .id("dameng-sql")
                            .h(px(64.))
                            .overflow_hidden()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(if sql_selected { accent } else { border })
                            .bg(if sql_selected {
                                pressed_background
                            } else {
                                field_background
                            })
                            .child(
                                Label::new(sql_shown)
                                    .color(if self.config.sql.is_empty() {
                                        Color::Placeholder
                                    } else {
                                        Color::Default
                                    })
                                    .size(LabelSize::Small),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select(Field::Sql, window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("dameng-test", "测试连接")
                            .style(ButtonStyle::Tinted(TintColor::Accent))
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run(Job::TestConnection, cx)
                            })),
                    )
                    .child(
                        Button::new("dameng-seed", "生成测试数据")
                            .style(ButtonStyle::Outlined)
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| this.run(Job::Seed, cx))),
                    )
                    .child(
                        Button::new("dameng-run", "执行 SQL")
                            .style(ButtonStyle::Outlined)
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| this.run(Job::RunSql, cx))),
                    )
                    .child(
                        Button::new("dameng-save", "保存配置")
                            .style(ButtonStyle::OutlinedGhost)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.save();
                                this.push_line(
                                    LogKind::Info,
                                    SharedString::from(format!(
                                        "配置已保存到 {}",
                                        Config::path().display()
                                    )),
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("dameng-clear", "清空日志")
                            .style(ButtonStyle::OutlinedGhost)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.log.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id("dameng-log")
                    .flex_1()
                    .gap_1()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(border)
                    .pt_2()
                    .children(transcript),
            )
    }
}

/// Opens the window, or focuses the one that is already open.
pub fn open(cx: &mut App) {
    // Rendering needs `theme::GlobalTheme`, and `render` would panic without it
    // ("no state of type `theme::GlobalTheme` exists"). A menu click always
    // happens long after the theme is registered, but the startup override below
    // can run before that, so wait for it rather than open a window that cannot
    // draw. Same guard, same reason, as the Octos console.
    if cx.try_global::<theme::GlobalTheme>().is_none() {
        log::info!("[dameng] waiting for the theme before opening the test window");
        cx.spawn(async move |cx| {
            for _ in 0..300 {
                if cx.update(|cx| cx.try_global::<theme::GlobalTheme>().is_some()) {
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
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
        .find_map(|window| window.downcast::<DamengWindow>())
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[dameng] could not focus the test window: {error}");
        }
        return;
    }

    let window_size = Size {
        width: px(720.),
        height: px(760.),
    };

    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Dameng 达梦测试".into()),
                // The system title bar, as elsewhere: the window draws no
                // chrome of its own, so a transparent bar would leave it with no
                // close button.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::centered(window_size, cx)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(DamengWindow::new);
            let focus = view.read(cx).focus.clone();
            window.activate_window();
            // Focus after the first frame: an element that has not been rendered
            // yet cannot take focus.
            window.defer(cx, move |window, cx| focus.focus(window, cx));
            view.into()
        },
    );

    match opened {
        Ok(_) => log::info!(
            "[dameng] test window opened ({} window(s) now open)",
            cx.windows().len()
        ),
        Err(error) => log::error!("[dameng] could not open the test window: {error}"),
    }
}

/// Opens the test window at startup when `ZED_DAMENG_OPEN` is set.
///
/// The same escape hatch `ZED_OCTOS_OPEN_STATUS`, `ZED_CESIUMRUST_OPEN` and
/// `ZED_ABOUT_OPEN` provide: a machine where the window matters has a way to open
/// it without clicking through a menu.
pub fn init(cx: &mut App) {
    if std::env::var_os("ZED_DAMENG_OPEN").is_some() {
        open(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are a usable local connection, so a first run with no
    /// remembered configuration is not an error message.
    #[test]
    fn the_defaults_are_connectable() {
        let params = parse(&Config::default()).expect("the defaults parse");
        assert_eq!(params.port, dm_core::DEFAULT_PORT);
        assert_eq!(params.server, "127.0.0.1");
        assert_eq!(params.rows, 32);
        assert_eq!(params.table, "ZED_DAMENG_TEST");
        assert!(!params.dpi, "ODBC is the default backend");
    }

    /// A port that is not a port is reported as one, rather than becoming `0`
    /// and failing later inside the driver.
    #[test]
    fn a_bad_port_is_rejected() {
        let mut config = Config::default();
        config.port = "not a port".to_owned();
        let error = parse(&config).expect_err("a bad port is rejected");
        assert!(error.contains("端口"), "{error}");

        config.port = "0".to_owned();
        assert!(parse(&config).is_err(), "port 0 is rejected");

        config.port = "70000".to_owned();
        assert!(parse(&config).is_err(), "a port above u16 is rejected");
    }

    /// The row count is bounded, because each row is a round trip to the server
    /// and a stray extra digit would look like a hang.
    #[test]
    fn the_row_count_is_bounded() {
        let mut config = Config::default();
        assert!(parse(&config).is_ok());

        config.rows = "0".to_owned();
        assert!(parse(&config).is_err(), "zero rows is rejected");

        config.rows = "10001".to_owned();
        assert!(parse(&config).is_err(), "more than ten thousand rows is rejected");

        config.rows = "10000".to_owned();
        assert!(parse(&config).is_ok(), "ten thousand rows is allowed");
    }

    /// The server and the table name are what the SQL is built from, so neither
    /// may be blank.
    #[test]
    fn blank_values_are_rejected() {
        let mut config = Config::default();
        config.server = "  ".to_owned();
        assert!(parse(&config).is_err(), "a blank server is rejected");

        let mut config = Config::default();
        config.table = String::new();
        assert!(parse(&config).is_err(), "a blank table is rejected");
    }

    /// A value with newlines is collapsed onto one line, because the transcript
    /// is a list of one-line labels, and long values are shortened.
    #[test]
    fn values_are_flattened() {
        assert_eq!(flatten("a\nb\tc"), "a⏎b c");

        let long = "x".repeat(500);
        let flattened = flatten(&long);
        assert_eq!(flattened.chars().count(), 401, "400 characters plus an ellipsis");
        assert!(flattened.ends_with('…'));
    }

    /// A driver error is printed inline, so its line breaks are folded.
    #[test]
    fn errors_are_folded_to_one_line() {
        assert_eq!(one_line("  bad\n\n  thing  \n"), "bad / thing");
        assert_eq!(one_line(""), "");
    }
}
