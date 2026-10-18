//! Helpers for locating and identifying Git repository directories on disk.
//!
//! These functions are the path-only portion of the former `git_store` module:
//! they read `.git` files and `commondir` files, and derive the on-disk
//! directory that a user thinks of as "the project". They do not talk to a
//! running repository and do not track any Git state.

use fs::Fs;
use gpui::SharedString;
use std::path::{Path, PathBuf};
use util::paths::PathStyle;

/// If `path` is a git linked worktree checkout, resolves it to the main
/// repository's identity path. For regular linked worktrees this is the main
/// repository's working directory; for linked worktrees backed by a bare repo
/// such as `.bare`, this is the parent project directory users think of as the
/// repository root. Returns `None` if `path` is a normal repository, not a git
/// repo, or if resolution fails.
///
/// Resolution works by:
/// 1. Reading the `.git` file to get the `gitdir:` pointer
/// 2. Following that to the worktree-specific git directory
/// 3. Reading the `commondir` file to find the shared `.git` directory
/// 4. Deriving the main repo's identity path from the common dir
pub async fn resolve_git_worktree_to_main_repo(fs: &dyn Fs, path: &Path) -> Option<PathBuf> {
    let dot_git = path.join(".git");
    let metadata = fs.metadata(&dot_git).await.ok()??;
    if metadata.is_dir {
        return None; // Normal repo, not a linked worktree
    }
    // It's a .git file — parse the gitdir: pointer
    let content = fs.load(&dot_git).await.ok()?;
    let gitdir_rel = content.strip_prefix("gitdir:")?.trim();
    let gitdir_abs = fs.canonicalize(&path.join(gitdir_rel)).await.ok()?;
    // Submodules also use a `.git` file, but they are independent projects whose
    // identity is their own working directory (`path`), not the superproject's
    // `.git/modules/<name>` git dir. Leave them unresolved.
    if is_submodule_git_dir(&gitdir_abs) {
        return None;
    }
    // Read commondir to find the main .git directory
    let commondir_content = fs.load(&gitdir_abs.join("commondir")).await.ok()?;
    let common_dir = fs
        .canonicalize(&gitdir_abs.join(commondir_content.trim()))
        .await
        .ok()?;
    Some(repo_identity_path(&common_dir, PathStyle::local()).to_path_buf())
}

/// Returns the repository's identity path given its common Git directory.
///
/// This is the canonical, on-disk path used for project grouping and as the
/// basis for display names. The goal is to return the directory the user
/// thinks of as "the project":
///
/// - If `common_dir`'s last component starts with `.` (e.g. `.git` for a
///   normal checkout, or `.bare` for a bare clone), the parent directory is
///   returned. Both of these are internal Git directories; the parent is the
///   meaningful project root.
/// - Otherwise (e.g. `zed.git` for a bare clone), `common_dir` itself is
///   returned — it is already a meaningful on-disk path.
pub fn repo_identity_path(common_dir: &Path, path_style: PathStyle) -> &Path {
    let is_dot_entry = path_style
        .file_name(common_dir)
        .is_some_and(|n| n.starts_with('.'));
    if is_dot_entry {
        path_style.parent(common_dir).unwrap_or(common_dir)
    } else {
        common_dir
    }
}

/// Returns the repository identity only when `std::path` can interpret the path correctly.
///
/// Callers whose downstream path operations are not yet `PathStyle`-aware use this to preserve
/// their existing behavior for foreign path styles.
pub fn repo_identity_path_if_local(common_dir: &Path, path_style: PathStyle) -> Option<&Path> {
    (path_style == PathStyle::local()).then(|| repo_identity_path(common_dir, path_style))
}

/// Returns true if `git_dir` is a Git submodule's git directory.
///
/// Submodules store their git directory inside the superproject at
/// `<superproject>/.git/modules/<name>`. Unlike a linked worktree, a submodule
/// is an independent project whose identity is its own working directory, so its
/// path must not be resolved to the superproject's `.git/modules/...` directory
/// (from which the working directory cannot be derived).
pub fn is_submodule_git_dir(git_dir: &Path) -> bool {
    let mut previous_was_git_dir = false;
    for component in git_dir.components() {
        if let std::path::Component::Normal(name) = component {
            if previous_was_git_dir && name == std::ffi::OsStr::new("modules") {
                return true;
            }
            previous_was_git_dir = name.to_string_lossy().ends_with(".git");
        } else {
            previous_was_git_dir = false;
        }
    }
    false
}

/// Returns a short name for a linked worktree suitable for UI display
///
/// Uses the main worktree path to come up with a short name that disambiguates
/// the linked worktree from the main worktree.
pub fn linked_worktree_short_name(
    main_worktree_path: &Path,
    linked_worktree_path: &Path,
) -> Option<SharedString> {
    if main_worktree_path == linked_worktree_path {
        return None;
    }

    let project_name = main_worktree_path.file_name()?.to_str()?;
    let directory_name = linked_worktree_path.file_name()?.to_str()?;
    let name = if directory_name != project_name {
        directory_name.to_string()
    } else {
        linked_worktree_path
            .parent()?
            .file_name()?
            .to_str()?
            .to_string()
    };
    Some(name.into())
}
