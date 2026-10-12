# Provenance

`crates/gpui-kit-showcase` is the GPUI Kit storybook — the component gallery
that upstream ships as `crates/story` (crate `gpui-component-story`) — vendored
so that a host application can mount it in one of its own windows.

## Why it exists instead of using `gpui-component-story` directly

`crates/gpui-component-story` is present in this repository, complete, but kept
out of the workspace (see the commented `members` entries in the root
`Cargo.toml`). The reason is a single unreachable dependency:

* On native targets it hard-depends on `gpui-shell`.
* `gpui-shell` pins `quickjs-jit` at rev
  `96593cd7a54440ac060a391b8baea7e9a3cf1393`.
* That object is not in this machine's cargo git db
  (`~/.cargo/git/db/quickjs-jit-85f77f3fd15b7d92`, which only holds
  `605da483611a3548edb8c33fdff602e3f5f42076`), and re-fetching it fails:

  ```
  fatal: unable to access 'https://github.com/longbridge/quickjs-jit/':
         Recv failure: Connection was reset
  ```

  so `cargo metadata --offline` exits 101 for that crate.

## How this crate was produced

`crates/gpui-kit-showcase/src` and `locales/` are a copy of
`crates/gpui-component-story/src` and `locales/`, with the changes below. The
original is left untouched, so restoring the workspace members once the pinned
QuickJS revision is fetchable requires no work on its side.

## Delta against upstream

1. **Removed** `src/stories/shell_story.rs`. It is the only file in the whole
   crate that names `gpui_shell` (verified with
   `git grep -n gpui_shell -- crates/gpui-component-story/`, which matches that
   file plus `Cargo.toml` and generated `js/*.d.ts` declarations). Removing it
   removes exactly one gallery page, `ShellStory`, from the sidebar.
2. `src/stories/mod.rs`: the `mod shell_story;` declaration and
   `pub use shell_story::ShellStory;` re-export are gone.
3. `src/gallery.rs`: the `StoryContainer::panel::<ShellStory>` entry is gone
   from the sidebar list.
4. `Cargo.toml`: `gpui-shell`, `reqwest_client` and `smol` are dropped.
   `reqwest_client` existed only to install a `ReqwestClient` as the app's HTTP
   client in `init`; a host application has its own, and overwriting it would
   break the host. `smol` was unused by this crate's sources.
5. `src/lib.rs`: `init` is split into
   * `init` — the state the gallery needs, idempotent, with no
     application-global side effects; and
   * `init_app` — `init` plus the storybook binary's own key bindings,
     `Quit`/`About` handlers, tracing subscriber, HTTP client and
     `cx.activate(true)`.

   The split is what makes embedding safe: upstream `init` binds `alt-f4`/`cmd-q`
   to a `Quit` action whose application-level handler calls `cx.quit()`, so
   mounting the storybook in a host without the split would let those keys quit
   the host.

   `init` also registers `/`, `ctrl-k` and `ctrl-shift-p` against the new
   `KEY_CONTEXT` ("GpuiKitShowcase"), and `StoryRoot`'s root element carries that
   context. They are the gallery's search and its two command palettes, they are
   how the theme is switched now that the in-window title bar is not rendered,
   and being scoped to that context they cannot reach the host's key map.
6. `src/main.rs`: the standalone binary calls `init_app` and this crate's name.

## Two fixes the vendored snapshot needed

The snapshot in `crates/gpui-component-story` has never been compiled in this
repository — `gpui-shell` cannot be fetched, so it has never been a workspace
member — and it does not compile as-is. Two classes of problem were found and
fixed here; both are recorded because they are *not* deliberate design choices,
and the same fixes would be needed if upstream's copy is ever built.

1. **`gpui::` in modules that do not have it in scope.** `#[derive(Action)]`
   and `#[gpui_kit::test]` expand to unqualified `gpui::…` paths
   (`gpui_macros::derive_action`, `gpui_macros::test`). A crate that depends on
   the `gpui` crate directly gets `gpui` in its extern prelude for free;
   `gpui-component-story` depends only on `gpui-kit`, and only the modules that
   glob-import `gpui_kit::*` pick up the Kit's `pub use crate as gpui` alias.
   Four files do not, and needed an explicit `use gpui_kit::gpui;`:
   `stories/diff_story.rs`, `stories/toolbar_story.rs`, and the test module of
   `stories/input_tokens.rs`. (`stories/theme_story/color_theme_story.rs` looks
   like a fifth but its `use gpui_kit::{prelude::FluentBuilder, *}` already
   covers it.) Depending on `gpui` directly instead is *not* a fix: the modules
   that do glob-import `gpui_kit::*` would then see two `gpui`s and fail with
   E0659.
2. **`Cargo.toml`**: `async-channel` and `tracing-subscriber` are needed by
   `data_table_story.rs` and by `init_app` respectively, and the
   `[dev-dependencies] gpui-kit = { features = ["test-support"] }` entry that
   the `#[gpui_kit::test]` unit tests need was missing.

## Features deliberately left off

Upstream's `gpui-component-story` enables `gpui-kit`'s `speech` feature and
defaults to `tree-sitter` (that is, `gpui-kit/tree-sitter-languages`). Neither
compiles against this repository's `Cargo.lock`:

```
crates/gpui-component/src/highlighter/languages.rs:404: error[E0425]:
    cannot find value `HIGHLIGHTS_QUERY` in crate `tree_sitter_yaml`
crates/gpui-component/src/speech/system/winrt.rs:19: error[E0432]:
    unresolved imports `windows::Foundation::AsyncActionCompletedHandler`,
    `windows::Foundation::IAsyncAction`, ... 
```

Both are version drift in the lock file, not code this crate owns: the locked
`tree-sitter-yaml 0.7.2` does not export the constant `gpui-component` asks for,
and the locked `windows` crate does not expose the WinRT async types its speech
backend imports. `crates/gpui-component` is a workspace member whose *current*
feature set compiles, so the fix belongs upstream or in a lock-file update, not
here.

They are therefore opt-in features of this crate (`--features tree-sitter`,
`--features speech`) rather than defaults, and the default build carries the
whole gallery except the syntax highlighting inside the Editor story's language
table and the Speech story's recognizer. Nothing in the gallery fails to render
because of it: `gpui_component::highlighter` and `gpui_component::speech` are
not feature-gated modules upstream, so the stories still build and run.

If you do turn them on, the failures above are what to expect until the lock
file or `gpui-component` is updated.

Everything else — all other stories, the theme registry, the locale layer, the
dock panel factory and the `AppState` plumbing — is upstream code unchanged.
