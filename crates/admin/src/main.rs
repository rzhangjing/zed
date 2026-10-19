//! The standalone admin demo.
//!
//! The application itself lives in `src/lib.rs` and its modules, so a host
//! process can run it in-process (`admin::serve_on`). This binary is only the
//! CLI shell: `admin toasty <command>` runs the toasty migration/schema CLI, and
//! no arguments serves the demo on `HOST`/`PORT` (default `127.0.0.1:3000`),
//! exactly as before the library target existed.

use admin::{app, db, seed};

#[tokio::main]
async fn main() {
    let db = db::connect().await.unwrap();

    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "toasty") {
        if let Err(error) = db::cli(db).parse_from(args).await {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }

    db::migrate(&db).await.unwrap();
    seed::seed(&db).await.unwrap();

    topcoat::start(app::router().app_context(db).build())
        .await
        .unwrap();
}
