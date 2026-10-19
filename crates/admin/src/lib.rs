//! The topcoat admin demo, as a library so a host process can run it in-process.
//!
//! `crates/admin` is topcoat's `demos/admin` application: a small back-office
//! (users, products, orders) built with topcoat views, the topcoat router and
//! toasty over SQLite. `src/main.rs` is the standalone binary and behaves
//! exactly as it did under topcoat's own workspace — `admin toasty ...` still
//! runs the toasty CLI, and the plain binary still serves on `HOST`/`PORT`.
//!
//! The library target exists for hosts that embed the demo instead of running
//! it as a child process. Zed's `Help > Admin Demo` window is such a host
//! (`crates/framework/zed/src/zed/admin_demo.rs`): it binds a loopback listener
//! on an ephemeral port, hands the listener to [`serve_on`], and stops the
//! server again by resolving the shutdown future it passed in.
//!
//! ## What [`serve_on`] owns
//!
//! The whole start-up chain the binary performs: connect to SQLite, apply the
//! embedded migrations, seed the demo data (both are idempotent), then serve
//! the router until `shutdown` resolves. The listener is created by the host,
//! which is what lets it learn the port before the server starts and report
//! bind failures without ever losing track of a half-started server.
//!
//! ## The database location
//!
//! The standalone binary keeps `data.db` next to the crate
//! ([`db::database_url`]). An embedded host should not write into the current
//! working directory, so [`serve_in`] takes a data directory — Zed passes
//! `<its data dir>/admin` — and [`db::DATA_DIR_ENV`] (`ADMIN_DATA_DIR`) can
//! override it from the environment.

pub mod app;
// Registry components are copied in full, including parts this demo does not use.
#[allow(dead_code)]
pub mod components;
pub mod db;
pub mod features;
pub mod seed;

use std::{
    future::Future,
    io,
    path::PathBuf,
};

use tokio::net::TcpListener;

/// The router type the host passes back to [`serve_prepared`].
///
/// Re-exported so a host can hold the prepared router without naming topcoat
/// itself.
pub use topcoat::router::Router;

/// Builds the demo's router with a connected, migrated and seeded database.
///
/// `data_dir` is created when it is `Some`; `None` keeps the binary's default
/// location ([`db::database_url`]). This is the first half of [`serve_in`], and
/// exists so a host can report a database or migration failure *before* it
/// announces a port that is already listening.
///
/// # Errors
///
/// Returns `Err` when the data directory cannot be created, the database cannot
/// be opened, or the migrations or the seed fail.
pub async fn prepare(data_dir: Option<PathBuf>) -> io::Result<Router> {
    if let Some(dir) = data_dir.as_deref() {
        std::fs::create_dir_all(dir).map_err(|error| {
            io::Error::other(format!(
                "could not create the admin data directory {}: {error}",
                dir.display()
            ))
        })?;
    }

    let db = db::connect_in(data_dir.as_deref())
        .await
        .map_err(|error| io::Error::other(format!("could not open the admin database: {error:#}")))?;

    db::migrate(&db)
        .await
        .map_err(|error| io::Error::other(format!("could not migrate the admin database: {error:#}")))?;

    seed::seed(&db)
        .await
        .map_err(|error| io::Error::other(format!("could not seed the admin database: {error:#}")))?;

    Ok(app::router().app_context(db).build())
}

/// Serves an already prepared router on `listener` until `shutdown` resolves.
///
/// # Errors
///
/// Returns `Err` when accepting a connection fails.
pub async fn serve_prepared(
    service: Router,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    topcoat::serve_until(listener, service, shutdown).await
}

/// Serves the admin demo on `listener` until `shutdown` resolves.
///
/// The database is the one [`db::connect`] resolves (the crate directory, or
/// [`db::DATA_DIR_ENV`] when it is set). The directory already has to exist;
/// use [`serve_in`] to have it created.
///
/// # Errors
///
/// Returns `Err` when the database cannot be opened, the migrations or the seed
/// fail, or serving the router fails. Every failure except the last one is
/// reported before the server starts accepting connections.
pub async fn serve_on(
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    serve_in(None, listener, shutdown).await
}

/// Serves the admin demo with its database in `data_dir`.
///
/// `data_dir` is created when it is `Some`; `None` keeps the binary's default
/// location. `shutdown` resolving does not abort requests that are already
/// running: topcoat's router finishes them until its own shutdown timeout.
///
/// # Errors
///
/// See [`serve_on`].
pub async fn serve_in(
    data_dir: Option<PathBuf>,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    let service = prepare(data_dir).await?;
    serve_prepared(service, listener, shutdown).await
}
