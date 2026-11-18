use std::path::Path;
use std::process::Command;

/// Run `git -C <dir> <args>`; Ok(stdout) only when the command exited 0.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Resolve an ABSOLUTE, actually-existing git metadata path via
/// `git rev-parse --path-format=absolute --git-path <name>`.
/// Returns None when git is unavailable, the repo doesn't exist, or the
/// resolved path doesn't exist on disk (e.g. packed refs have no loose
/// ref file) — a rerun-if-changed on a nonexistent path makes cargo mark
/// the unit dirty on EVERY build (the linked-worktree rebuild bug).
fn existing_git_path(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    let out = git_out(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-path", name],
    )?;
    let path = Path::new(&out);
    (path.is_file() || path.is_dir()).then(|| path.to_path_buf())
}

/// The current branch name from HEAD (`ref: refs/heads/<b>`), or None for
/// detached HEAD / unreadable HEAD.
fn head_branch(dir: &Path) -> Option<String> {
    let head = existing_git_path(dir, "HEAD")?;
    let text = std::fs::read_to_string(head).ok()?;
    let branch = text.trim().strip_prefix("ref: ")?;
    if branch == "HEAD" || !branch.starts_with("refs/heads/") {
        return None; // detached
    }
    Some(branch.to_string())
}

/// This package belongs to `<project>/crates/octos/octos-cli`. An archive nested
/// in another repository must not inherit that outer repository's version, so
/// the manifest must live under the owning repository's own `crates/` tree
/// (`<project>/crates/…`), and that repository must be a real ancestor whose
/// `.git` entry exists. Let Git resolve `.git` files (including relative
/// worktree pointers).
///
/// The position is read by walking path components (not by counting `..`
/// parents): counting parents silently disabled the version stamp once this
/// crate moved one level deeper inside `<project>/crates/`, while walking keeps
/// every depth inside `<project>/crates/…` working — and still refuses an
/// archive extracted inside someone else's repository
/// (`<outer>/extracted/crates/probe`), which is what
/// `task-build-git-metadata-watch` §archive requires.
fn owning_repo(dir: &Path) -> Option<()> {
    let manifest = dir.canonicalize().ok()?;
    let toplevel = Path::new(&git_out(dir, &["rev-parse", "--show-toplevel"])?)
        .canonicalize()
        .ok()?;
    if !toplevel.join(".git").exists() {
        return None;
    }
    let under_root = manifest.strip_prefix(&toplevel).ok()?;
    let crates = std::ffi::OsStr::new("crates");
    matches!(
        under_root.components().next(),
        Some(std::path::Component::Normal(name)) if name == crates
    )
    .then_some(())
}

fn main() {
    // The build script's CWD is the package manifest dir.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Git short hash — only when this package genuinely belongs to the
    // repository that owns its manifest dir (see owning_repo).
    let trusted_git = owning_repo(manifest).is_some();
    let hash = if trusted_git {
        git_out(manifest, &["rev-parse", "--short", "HEAD"]).unwrap_or_default()
    } else {
        String::new()
    };
    println!("cargo:rustc-env=OCTOS_GIT_HASH={hash}");

    // Build date (YYYY-MM-DD)
    let date = Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=OCTOS_BUILD_DATE={date}");

    // Re-run only on REAL metadata paths, each verified to exist:
    //  * HEAD — text changes on branch switch / detached checkout.
    //  * the current branch ref file — rewritten on every same-branch
    //    commit (HEAD text does NOT change then; watching HEAD alone
    //    leaves the hash stale).
    //  * packed-refs — when refs are packed there is no loose ref file;
    //    packing/pruning changes this file. A later commit may instead
    //    create a loose ref, detected by its nearest existing parent below.
    // Watching a nonexistent path (the old `../../.git/HEAD` shapes) makes
    // cargo treat the unit as dirty on every no-change build — the
    // linked-worktree rebuild bug. In git-less trees (and non-adopted
    // archives) we emit no git watch lines at all.
    // A nonempty rerun set REPLACES cargo's default (watch the whole
    // package). In git-less trees an EMPTY set would fall back to
    // watch-everything; pin the build script itself instead so unrelated
    // source edits still behave and the set is never empty.
    if trusted_git {
        if let Some(head) = existing_git_path(manifest, "HEAD") {
            println!("cargo:rerun-if-changed={}", head.display());
        }
        if let Some(branch) = head_branch(manifest) {
            if let Some(ref_file) = existing_git_path(manifest, &branch) {
                println!("cargo:rerun-if-changed={}", ref_file.display());
            } else {
                // Packed refs: the loose ref file does not exist YET. A
                // same-branch commit then creates it WITHOUT touching HEAD
                // or packed-refs. Watch its nearest existing parent so the
                // creation is detected, without watching unrelated remote
                // refs when refs/heads (or a narrower parent) exists. Walk
                // upward because packing may remove nested branch dirs.
                // Direct refs also work when HEAD reflogs are disabled.
                let mut ancestor = branch.as_str();
                while let Some((parent, _)) = ancestor.rsplit_once('/') {
                    if let Some(directory) = existing_git_path(manifest, parent) {
                        println!("cargo:rerun-if-changed={}", directory.display());
                        break;
                    }
                    ancestor = parent;
                }
            }
        }
        if let Some(packed) = existing_git_path(manifest, "packed-refs") {
            println!("cargo:rerun-if-changed={}", packed.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
