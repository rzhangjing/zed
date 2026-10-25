//! Build script for the embedded Admin demo (`Help > Admin Demo`).
//!
//! Topcoat generates two build artefacts here:
//!
//! * `$OUT_DIR/tailwind.css`, compiled from `styles.css` and referenced by
//!   `tailwind::stylesheet!()`.
//! * `$OUT_DIR/topcoat-icon-iconify/lucide.json`, the Iconify set that
//!   `iconify_icon!("lucide:…")` reads while its macros expand.
//!
//! Neither may break compilation: the Tailwind CLI is downloaded on demand and
//! that download can fail on an offline or rate-limited machine (it did, with
//! `Http(Timeout(Connect))`). This script reports such failures as
//! `cargo:warning=` and keeps going.
//!
//! ## Providing the Tailwind CLI offline
//!
//! The CLI is looked up in this order:
//!
//! 1. `ADMIN_TAILWIND_CLI`: the path (or bare `PATH` command name) of a Tailwind
//!    CLI executable. Relative paths resolve against this package's root. This
//!    is the switch to use on a machine without network access.
//! 2. `<target>/topcoat/cache/tailwind/tailwindcss-<version>-<platform>`, the
//!    shared cache Topcoat downloads into, for example
//!    `target\topcoat\cache\tailwind\tailwindcss-4.3.2-windows-x64.exe`.
//!    Topcoat reuses a cached release without any network access.
//! 3. `<target>/admin-tw-cache/tailwindcss-*`, a hand-kept copy of the same
//!    binary (it is not part of the repository).
//! 4. Otherwise Topcoat downloads the pinned release from GitHub. When
//!    `CARGO_NET_OFFLINE` is set, the download is skipped immediately instead
//!    of waiting for a connect timeout.
//!
//! `<target>` is derived from `OUT_DIR`, which Cargo lays out as
//! `<target>[/<triple>]/<profile>/build/<package>-<hash>/out`.
//!
//! ## Providing the Iconify set offline
//!
//! Topcoat caches `@iconify-json/lucide` at
//! `<target>/topcoat/cache/iconify/lucide.json`. Put a real Iconify JSON file
//! there to stage real icons offline. If it is missing and cannot be
//! downloaded, this script writes a placeholder set so the crate still
//! compiles (see [`write_placeholder_set`]).
//!
//! ## Degradation without either artefact
//!
//! * Missing Tailwind CLI: `$OUT_DIR/tailwind.css` is not written. `asset!`
//!   only records paths, so the crate compiles; the demo then serves a page
//!   without CSS (and `admin-bundle-assets` reports the missing stylesheet when
//!   the asset bundle is generated).
//! * Missing Lucide set: every icon named by `iconify_icon!("lucide:…")` in this
//!   crate is given an empty body, so icons render as blank SVGs instead of
//!   failing the build.

use std::{
    env,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

/// Environment variable naming an existing Tailwind CLI executable.
const TAILWIND_CLI_ENV: &str = "ADMIN_TAILWIND_CLI";

/// The Iconify set staged for the `iconify_icon!` macros in this crate.
const ICON_SET: &str = "lucide";

/// Marker written into a placeholder icon set so a later run can tell it apart
/// from a real, downloaded set.
const PLACEHOLDER_MARKER: &str = "x-admin-icon-placeholder";

fn main() {
    rerun_directives();
    render_tailwind();
    stage_icons();
}

/// Prints the `cargo:rerun-if-*` directives.
///
/// Printing any directive replaces Cargo's default "rerun if a package file
/// changed" detection, so every input is named explicitly. Do not use `.`:
/// building this crate as its own workspace puts `target/` inside the package,
/// and watching that would rerun the script on every build.
fn rerun_directives() {
    println!("cargo:rerun-if-env-changed={TAILWIND_CLI_ENV}");
    println!("cargo:rerun-if-env-changed=CARGO_NET_OFFLINE");

    // The Tailwind class scan and the icon-name scan read the whole package,
    // not only `build.rs`. Extend this list if new scanned inputs are added.
    for path in [
        ".gitignore",
        "Cargo.lock",
        "Cargo.toml",
        "README.md",
        "build.rs",
        "components.toml",
        "src",
        "styles.css",
        "tests",
        "toasty",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// Renders `styles.css` to `$OUT_DIR/tailwind.css`, downgrading every failure
/// to a warning.
fn render_tailwind() {
    let mut config = topcoat::tailwind::BuildConfig::new().input("styles.css");

    // `ADMIN_TAILWIND_CLI` goes through Topcoat's own environment-variable
    // source, so the value is interpreted exactly like `BuildConfig::executable`
    // (a bare command name is resolved through `PATH`).
    let mut have_cli = false;
    if env::var_os(TAILWIND_CLI_ENV).is_some_and(|value| !value.is_empty()) {
        config = config.executable_env(TAILWIND_CLI_ENV);
        have_cli = true;
    } else if let Some(path) = cached_tailwind_cli() {
        config = config.executable(path);
        have_cli = true;
    }

    if !have_cli {
        if offline() {
            warn_tailwind(
                "no Tailwind CLI was found in the Topcoat cache and \
                 `CARGO_NET_OFFLINE` is set, so the GitHub download was skipped",
            );
            return;
        }
        // Fall through to Topcoat's default: download the pinned release.
    }

    if let Err(error) = config.render() {
        warn_tailwind(&format!("running the Tailwind CLI failed: {error}"));
    }
}

/// The Tailwind CLI in the shared Topcoat cache or the hand-kept backup,
/// preferring the pinned version. Topcoat reuses a cached binary without
/// touching the network, so finding it here lets an offline build succeed and
/// lets the warning name the file it expected.
fn cached_tailwind_cli() -> Option<PathBuf> {
    let target = target_dir()?;
    let dirs = [
        target.join("topcoat").join("cache").join("tailwind"),
        target.join("admin-tw-cache"),
    ];
    let preferred = format!("tailwindcss-{}-", topcoat::tailwind::DEFAULT_VERSION);

    let mut fallback = None;
    for dir in dirs {
        for path in executables_in(&dir) {
            let matches_version = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&preferred));
            if matches_version {
                return Some(path);
            }
            fallback.get_or_insert(path);
        }
    }
    fallback
}

/// The files in `dir` that look like a downloaded Tailwind CLI, sorted.
fn executables_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_tailwind_executable(path))
        .collect();
    found.sort();
    found
}

/// Whether `path` looks like a complete Tailwind CLI download rather than a
/// lock file, a partial `.download`, or a directory.
fn is_tailwind_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.starts_with("tailwindcss-") || name.ends_with(".lock") || name.ends_with(".download") {
        return false;
    }
    if cfg!(windows) && !name.ends_with(".exe") {
        return false;
    }
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

/// Stages the Lucide Iconify set, falling back to a placeholder set with empty
/// icon bodies when it cannot be downloaded.
fn stage_icons() {
    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        println!(
            "cargo:warning=admin: `OUT_DIR` is not set, so the `{ICON_SET}` Iconify set was \
             not staged; `iconify_icon!` calls in this crate will fail to compile"
        );
        return;
    };
    let staged = out_dir
        .join(topcoat::icon::iconify::STAGE_DIR)
        .join(format!("{ICON_SET}.json"));
    let cache = iconify_cache_path();
    let has_cache = cache.as_deref().is_some_and(Path::exists);

    // A placeholder from an earlier build must not be mistaken for a cached
    // set: Topcoat reuses any existing file. Drop it as soon as real data can
    // be obtained again (a cache file exists, or the network is allowed).
    if is_placeholder(&staged) {
        if has_cache || !offline() {
            let _ = fs::remove_file(&staged);
        } else {
            warn_icons(None, &staged);
            return;
        }
    }

    // Staging downloads without knowing about `CARGO_NET_OFFLINE`, so skip it
    // when there is nothing cached to reuse: otherwise an offline machine waits
    // for a connect timeout before failing.
    if offline() && !has_cache {
        let names = referenced_icons();
        if write_placeholder_set(&staged, &names).is_ok() {
            warn_icons(
                Some(
                    "`CARGO_NET_OFFLINE` is set and the set is not cached, so the \
                     jsDelivr download was skipped",
                ),
                &staged,
            );
        } else {
            println!(
                "cargo:warning=admin: offline, the `{ICON_SET}` Iconify set is not cached, \
                 and a placeholder could not be written; `iconify_icon!` calls in this crate \
                 will fail to compile"
            );
        }
        return;
    }

    if let Err(error) = topcoat::icon::iconify::BuildConfig::new()
        .icon_set(ICON_SET)
        .stage()
    {
        let names = referenced_icons();
        if let Err(write_error) = write_placeholder_set(&staged, &names) {
            println!(
                "cargo:warning=admin: the `{ICON_SET}` Iconify set could not be staged \
                 ({error}) and a placeholder could not be written either ({write_error}); \
                 `iconify_icon!` calls in this crate will fail to compile"
            );
        } else {
            warn_icons(Some(&error.to_string()), &staged);
        }
    }
}

/// The shared Topcoat cache file for the Iconify set.
fn iconify_cache_path() -> Option<PathBuf> {
    Some(
        target_dir()?
            .join("topcoat")
            .join("cache")
            .join("iconify")
            .join(format!("{ICON_SET}.json")),
    )
}

/// Whether the staged set is the placeholder written by [`write_placeholder_set`].
fn is_placeholder(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|json| json.contains(PLACEHOLDER_MARKER))
}

/// Writes a valid Iconify JSON set in which every icon referenced by this
/// crate exists with an empty body, so `iconify_icon!` expands instead of
/// failing the build. Icons then render as blank SVGs.
fn write_placeholder_set(path: &Path, names: &[String]) -> io::Result<()> {
    let mut icons = String::new();
    for name in names {
        if !icons.is_empty() {
            icons.push(',');
        }
        // Names come from string literals in the crate's sources; only plain
        // kebab-case names are accepted as a guard against malformed JSON.
        let _ = write!(icons, "\"{name}\":{{\"body\":\"\"}}");
    }
    let json = format!(
        "{{\"prefix\":\"{ICON_SET}\",\"width\":24,\"height\":24,\
         \"{PLACEHOLDER_MARKER}\":true,\"icons\":{{{icons}}}}}"
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json)
}

/// The `lucide:<name>` arguments of the `iconify_icon!` invocations in this
/// crate's sources, sorted and deduplicated.
///
/// Scanning the sources keeps the placeholder complete when icons are added or
/// removed, instead of hard-coding a list that can drift.
fn referenced_icons() -> Vec<String> {
    let mut names = Vec::new();
    if let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest_dir);
        for dir in ["src", "tests", "benches", "examples"] {
            collect_icon_names(&manifest_dir.join(dir), 0, &mut names);
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Recursively collects the `lucide:` icon names from the Rust files under
/// `dir`.
fn collect_icon_names(dir: &Path, depth: usize, names: &mut Vec<String>) {
    // A depth cap keeps a symlinked tree from recursing forever.
    const MAX_DEPTH: usize = 16;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_icon_names(&path, depth + 1, names);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            if let Ok(source) = fs::read_to_string(&path) {
                icon_names_in_source(&source, names);
            }
        }
    }
}

/// Collects the `lucide:` names from `iconify_icon!("lucide:…")` invocations
/// in `source`.
fn icon_names_in_source(source: &str, names: &mut Vec<String>) {
    const INVOCATION: &str = "iconify_icon!";

    let mut rest = source;
    while let Some(index) = rest.find(INVOCATION) {
        rest = &rest[index + INVOCATION.len()..];
        // The first string literal after the macro name is its argument; doc
        // comments and macro definitions can add extra names, which is
        // harmless for a placeholder set.
        let Some(open) = rest.find('"') else {
            break;
        };
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('"') else {
            break;
        };
        if let Some(name) = after_open[..close].strip_prefix("lucide:") {
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if valid {
                names.push(name.to_owned());
            }
        }
        rest = &after_open[close..];
    }
}

/// The Cargo target directory, derived from `OUT_DIR`'s
/// `<target>[/<triple>]/<profile>/build/<package>-<hash>/out` layout.
fn target_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR")?);
    let build = out_dir.parent()?.parent()?;
    if build.file_name()? != "build" {
        return None;
    }
    Some(build.parent()?.parent()?.to_path_buf())
}

/// Whether the build runs without network access.
fn offline() -> bool {
    match env::var_os("CARGO_NET_OFFLINE") {
        Some(value) => {
            let value = value.to_string_lossy();
            !(value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
        }
        None => false,
    }
}

/// Warns that no stylesheet was generated and explains how to provide the CLI.
fn warn_tailwind(reason: &str) {
    let version = topcoat::tailwind::DEFAULT_VERSION;
    let cache = match target_dir() {
        Some(target) => format!(
            "`{}`",
            target
                .join("topcoat")
                .join("cache")
                .join("tailwind")
                .join(format!("tailwindcss-{version}-<platform>"))
                .display()
        ),
        None => "`<target>/topcoat/cache/tailwind/`".to_owned(),
    };
    println!(
        "cargo:warning=admin: the Tailwind stylesheet was not generated ({reason}). \
         The demo still compiles, but serves no CSS. To build offline, set \
         {TAILWIND_CLI_ENV} to a Tailwind CLI v{version} executable, or put the \
         release binary in the Topcoat cache at {cache} \
         (e.g. tailwindcss-{version}-windows-x64.exe). Rerun this build script \
         afterwards (touch build.rs, or set/change {TAILWIND_CLI_ENV}).",
        reason = one_line(reason),
    );
}

/// Warns that the placeholder icon set is in use and explains how to replace
/// it with the real one.
fn warn_icons(reason: Option<&str>, staged: &Path) {
    let reason = reason
        .map(one_line)
        .unwrap_or_else(|| "reusing the placeholder from an earlier build".to_owned());
    let cache = iconify_cache_path()
        .map(|path| format!("`{}`", path.display()))
        .unwrap_or_else(|| "`<target>/topcoat/cache/iconify/lucide.json`".to_owned());
    println!(
        "cargo:warning=admin: the `{ICON_SET}` Iconify set is a placeholder ({reason}). \
         The crate still compiles, but icons render as empty SVGs. To use real \
         icons offline, put the `@iconify-json/{ICON_SET}` `icons.json` at {cache}, \
         delete `{staged}`, and rerun this build script.",
        staged = staged.display(),
    );
}

/// Collapses an error message to a single line so it fits one
/// `cargo:warning=` directive.
fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}
