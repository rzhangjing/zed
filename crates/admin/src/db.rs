use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use toasty::{Db, migration::MigrationSet};
use toasty_cli::{Config, MigrationConfig, ToastyCli};
use topcoat::context::{Cx, app_context};

/// Environment variable that overrides where `data.db` lives.
///
/// The standalone binary keeps its database next to the crate
/// ([`DEFAULT_DATA_DIR`]); when the demo is embedded in another process (Zed's
/// `Help > Admin Demo`) that directory is the host's own data directory, so it
/// is passed explicitly or through this variable instead of landing in the
/// host's working directory.
pub const DATA_DIR_ENV: &str = "ADMIN_DATA_DIR";

/// The database file's name inside the data directory.
pub const DATABASE_FILE: &str = "data.db";

/// Use the package directory so the database path does not depend on where the
/// standalone app starts.
const DEFAULT_DATA_DIR: &str = env!("CARGO_MANIFEST_DIR");

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/toasty");

const LATENCY: Duration = Duration::from_millis(500);

const MIGRATIONS: MigrationSet = toasty::embed_migrations!();

/// The data directory from [`DATA_DIR_ENV`], if it is set and non-empty.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os(DATA_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The `sqlite:` connection URL for `dir`, or the default one when `dir` and
/// [`DATA_DIR_ENV`] are both unset.
///
/// `sqlite:` URLs take forward slashes, so the Windows path is normalized
/// rather than passed through `Path::display`.
pub fn database_url(dir: Option<&Path>) -> String {
    let dir = dir
        .map(Path::to_path_buf)
        .or_else(data_dir)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));
    let file = dir.join(DATABASE_FILE).to_string_lossy().replace('\\', "/");
    format!("sqlite:{file}")
}

/// Connects using the default location (or [`DATA_DIR_ENV`]).
///
/// The override is created if it does not exist yet, so `ADMIN_DATA_DIR` works
/// for the standalone binary exactly as it does for an embedded host (which
/// creates its directory in [`crate::prepare`]). A failure to create it is not
/// fatal here: SQLite reports the same path with its own error.
pub async fn connect() -> toasty::Result<Db> {
    if let Some(dir) = data_dir() {
        let _ = std::fs::create_dir_all(dir);
    }
    connect_in(None).await
}

/// Connects to the demo's database in `dir` (see [`database_url`]).
pub async fn connect_in(dir: Option<&Path>) -> toasty::Result<Db> {
    Db::builder()
        .models(toasty::models!(crate::*))
        .connect(&database_url(dir))
        .await
}

pub async fn migrate(db: &Db) -> toasty::Result<()> {
    MIGRATIONS.apply(db).await?;
    Ok(())
}

pub fn cli(db: Db) -> ToastyCli {
    let config = Config::new().migration(MigrationConfig::new().path(MIGRATIONS_DIR));
    ToastyCli::with_config(db, config)
}

pub fn db(cx: &Cx) -> Db {
    app_context::<Db>(cx).clone()
}

/// Adds a short delay so the demo's loading placeholders stay visible.
/// Call before queries whose results are shown with a loading state.
pub async fn latency() {
    tokio::time::sleep(LATENCY).await;
}

/// Builds a SQL LIKE pattern for a literal substring search.
/// Use with `like_with_escape` and a backslash escape character. SQLite LIKE
/// ignores case for ASCII letters.
pub fn contains_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for char in text.chars() {
        if matches!(char, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(char);
    }
    pattern.push('%');
    pattern
}
