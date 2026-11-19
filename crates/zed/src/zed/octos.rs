//! `Help > Octos`: runs the Octos CLI in a terminal tab.
//!
//! This is the menu path from the original `zed_my` integration: `octos chat
//! --profile _main --no-session-persistence` is spawned into the workspace's
//! terminal panel, with `OCTOS_HOME`/`OCTOS_CONFIG_DIR` pointing at the same
//! home the embedded kernel used, so a profile configured once works in either
//! place. The embedded kernel itself (the Agent Panel bridge over ACP) is not
//! part of this port; the CLI is its own process with its own session.
//!
//! `ZED_OCTOS_CHAT_MESSAGE` appends `--message <text>`, which makes `chat`
//! answer one turn and exit: that is how a script proves the terminal really
//! reaches the model. It is not part of the normal path.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use collections::HashMap;
use gpui::{App, AppContext as _, Context, TaskExt as _, Window};
use task::{
    HideStrategy, RevealStrategy, RevealTarget, SaveStrategy, Shell, SpawnInTerminal, TaskId,
};
use terminal_view::terminal_panel::TerminalPanel;
use workspace::Workspace;
use workspace::notifications::{NotificationId, simple_message_notification::MessageNotification};

/// `ZED_OCTOS_HOME` overrides the Octos home; the default sits under Zed's data
/// directory, next to the CLI's own default.
const ENV_HOME: &str = "ZED_OCTOS_HOME";

/// `ZED_OCTOS_BINARY` points at `octos.exe` when it is neither next to Zed nor
/// on `PATH`.
const ENV_BINARY: &str = "ZED_OCTOS_BINARY";

/// `ZED_OCTOS_CHAT_MESSAGE` sends one message and exits, for scripted checks.
const ENV_CHAT_MESSAGE: &str = "ZED_OCTOS_CHAT_MESSAGE";

/// Mirrors `octos_core::MAIN_PROFILE_ID`, which this crate does not depend on.
const MAIN_PROFILE_ID: &str = "_main";

/// Opens the Octos terminal, or reports why it cannot.
pub fn open(cx: &mut App) {
    log::info!("[octos] opening the Octos terminal");

    let binary = match resolve_binary() {
        Ok(binary) => binary,
        Err(reason) => {
            let message = format!(
                "Octos was not started: {reason}.\n\n\
                 Build it with `cargo build -p octos-cli --release --no-default-features \
                 --features api`, put `octos.exe` next to Zed, or set `{ENV_BINARY}` to \
                 the executable."
            );
            log::error!("[octos] {message}");
            with_active_workspace(cx, move |workspace, _window, cx| {
                show_failure(workspace, cx, message.clone());
            });
            return;
        }
    };

    log::info!("[octos] using Octos binary {}", describe_binary(&binary));
    with_active_workspace(cx, move |workspace, window, cx| {
        spawn_terminal(workspace, window, cx, &binary);
    });
}

/// Runs `f` in the active workspace, waiting for one if Zed is still starting.
///
/// Not `workspace::with_active_or_new_workspace`: that opens a new window when
/// there is none, and during `zed::init` it dereferences `GlobalAppState` before
/// that global exists, which panics.
fn with_active_workspace(
    cx: &mut App,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + Send + 'static,
) {
    if let Some(window) = active_workspace_window(cx) {
        cx.defer(move |cx| apply_in_workspace(window, cx, f));
        return;
    }

    log::info!("[octos] waiting for a workspace window before opening the Octos terminal");
    cx.spawn(async move |cx| {
        let mut f = Some(f);
        for _ in 0..300 {
            if let Some(window) = cx.update(|cx| active_workspace_window(cx)) {
                cx.update(|cx| {
                    if let Some(f) = f.take() {
                        apply_in_workspace(window, cx, f);
                    }
                });
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
        }
        log::error!("[octos] no workspace window appeared; the Octos terminal was not opened");
    })
    .detach();
}

fn active_workspace_window(cx: &App) -> Option<gpui::WindowHandle<workspace::MultiWorkspace>> {
    cx.active_window()
        .and_then(|window| window.downcast::<workspace::MultiWorkspace>())
}

fn apply_in_workspace(
    window: gpui::WindowHandle<workspace::MultiWorkspace>,
    cx: &mut App,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) {
    if let Err(error) = window.update(cx, |multi_workspace, window, cx| {
        let workspace = multi_workspace.workspace().clone();
        workspace.update(cx, |workspace, cx| f(workspace, window, cx))
    }) {
        log::error!("[octos] could not reach the workspace window: {error}");
    }
}

/// Starts one `octos chat --profile _main` tab in the workspace's terminal panel.
fn spawn_terminal(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
    binary: &Path,
) {
    let Some(terminal_panel) = workspace.panel::<TerminalPanel>(cx) else {
        let reason = "the terminal panel is unavailable in this workspace".to_owned();
        log::error!("[octos] {reason}");
        show_failure(workspace, cx, reason);
        return;
    };

    let home = home();
    let data_dir = home.join(".octos");
    let config_dir = home.join(".config").join("octos");
    let args = octos_chat_args(env::var(ENV_CHAT_MESSAGE).ok().as_deref());
    log::info!(
        "[octos] running `{} {}` in a terminal tab labelled Octos \
         (OCTOS_HOME={}, OCTOS_CONFIG_DIR={})",
        binary.display(),
        args.join(" "),
        data_dir.display(),
        config_dir.display()
    );

    let task = octos_task(binary, &home, &args);
    // `TerminalPanel::spawn_task` reads the workspace back
    // (`self.workspace.read(cx)`), and we are inside `Workspace::update`;
    // calling it here would double-lease the workspace and panic in debug
    // builds. Defer one frame so the read happens after this update ends.
    window.defer(cx, move |window, cx| {
        let spawned = terminal_panel.update(cx, |terminal_panel, cx| {
            terminal_panel.spawn_task(&task, window, cx)
        });
        spawned.detach_and_log_err(cx);
    });
}

/// The arguments `Help > Octos` passes to the CLI.
///
/// `--no-session-persistence` keeps the shared profile, memory, tools and
/// skills while using an in-memory episode handle instead of fighting over the
/// episode database lock.
fn octos_chat_args(message: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "chat".to_owned(),
        "--profile".to_owned(),
        MAIN_PROFILE_ID.to_owned(),
        "--no-session-persistence".to_owned(),
    ];
    if let Some(message) = message.map(str::trim).filter(|message| !message.is_empty()) {
        args.push("--message".to_owned());
        args.push(message.to_owned());
    }
    args
}

/// The [`SpawnInTerminal`] for `octos chat`.
///
/// `command`/`args` move into [`Shell::WithArguments`], which is how the
/// terminal runs the absolute path directly instead of passing it through a
/// shell that would have to quote spaces in it.
fn octos_task(binary: &Path, home: &Path, args: &[String]) -> SpawnInTerminal {
    let mut env = HashMap::default();
    // `octos_cli::embedded::serve_io` derives both paths from `home` the same
    // way. Setting the CLI's env to those exact paths is what makes the
    // terminal see the profile (`<home>/.octos/profiles/_main.json`).
    env.insert(
        "OCTOS_HOME".to_owned(),
        home.join(".octos").to_string_lossy().into_owned(),
    );
    env.insert(
        "OCTOS_CONFIG_DIR".to_owned(),
        home.join(".config")
            .join("octos")
            .to_string_lossy()
            .into_owned(),
    );
    let program = binary.to_string_lossy().into_owned();

    SpawnInTerminal {
        id: TaskId("octos-chat".to_owned()),
        full_label: "Octos".to_owned(),
        label: "Octos".to_owned(),
        command: None,
        args: Vec::new(),
        command_label: program.clone(),
        cwd: None,
        env,
        use_new_terminal: true,
        allow_concurrent_runs: true,
        reveal: RevealStrategy::Always,
        reveal_target: RevealTarget::Dock,
        hide: HideStrategy::Never,
        shell: Shell::WithArguments {
            program,
            args: args.to_vec(),
            title_override: Some("Octos".to_owned()),
        },
        show_summary: false,
        show_command: true,
        show_rerun: false,
        save: SaveStrategy::None,
    }
}

/// `ZED_OCTOS_BINARY`, then `octos[.exe]` next to Zed, then `octos[.exe]` on PATH.
///
/// `Err` carries a reason fit to show the user; nothing here panics.
fn resolve_binary() -> Result<PathBuf, String> {
    if let Some(configured) = env::var_os(ENV_BINARY) {
        let configured = PathBuf::from(configured);
        if configured.is_file() {
            return Ok(absolute(configured));
        }
        return Err(format!(
            "{ENV_BINARY} is set to {}, which is not a file",
            configured.display()
        ));
    }

    let sibling = env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(format!("octos{}", env::consts::EXE_SUFFIX)));
    if let Some(sibling) = sibling.as_ref().filter(|path| path.is_file()) {
        return Ok(sibling.clone());
    }

    if let Some(on_path) = find_on_path() {
        return Ok(on_path);
    }

    Err(format!(
        "no Octos executable found ({ENV_BINARY} is unset, {}, and `octos{}` is not on PATH)",
        sibling
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "the directory next to Zed".to_owned()),
        env::consts::EXE_SUFFIX
    ))
}

/// The `octos[.exe]` on `PATH`, if any.
fn find_on_path() -> Option<PathBuf> {
    let name = format!("octos{}", env::consts::EXE_SUFFIX);
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|directory| directory.join(&name))
            .find(|candidate| candidate.is_file())
    })
}

/// Absolute without resolving symlinks, so the log names the file that was run.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// The selected binary as an absolute path and its mtime.
///
/// The mtime is the point: `target/debug/octos.exe` can be an older build than
/// the source tree, and that is otherwise invisible until the CLI behaves
/// unlike the code in front of you.
fn describe_binary(path: &Path) -> String {
    match fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified) => {
            let modified: chrono::DateTime<chrono::Local> = modified.into();
            format!(
                "{} (modified {})",
                path.display(),
                modified.format("%Y-%m-%d %H:%M:%S")
            )
        }
        Err(error) => format!("{} (mtime unavailable: {error})", path.display()),
    }
}

/// Shows a terminal-start failure in the workspace, not just in the log.
fn show_failure(workspace: &mut Workspace, cx: &mut Context<Workspace>, message: String) {
    struct OctosTerminalError;
    workspace.show_notification(NotificationId::unique::<OctosTerminalError>(), cx, |cx| {
        cx.new(|cx| MessageNotification::new(message, cx))
    });
}

fn home() -> PathBuf {
    match env::var_os(ENV_HOME) {
        Some(home) => PathBuf::from(home),
        None => paths::data_dir().join("octos"),
    }
}
