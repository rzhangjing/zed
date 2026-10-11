# GPUI Kit, vendored into this fork

This directory holds the repo-level files of [GPUI Kit](https://github.com/longbridge/gpui-kit)
that the crates under `crates/gpui-kit*` embed with `include_str!` / `include_bytes!`.
They live here, rather than inside a crate, so the `../../..`-style paths the
upstream sources use keep their original depth.

| Path | Upstream source | Used by |
| --- | --- | --- |
| `README.md`, `README.zh-CN.md` | `README.md`, `README.zh-CN.md` | `crates/gpui-base` (`TextView` markdown test), `crates/gpui-component-story` (welcome/dialog stories) |
| `themes/*.json` | `themes/*.json` (21 presets) | `crates/gpui-component` theme-schema test, `crates/gpui-component-story` `embedded_themes.rs` |
| `fixtures/*` | `examples/fixtures/*` (17 language samples) | `crates/gpui-component` `text::compat` markdown test |
| `LICENSE-APACHE` | `LICENSE-APACHE` | Apache-2.0 terms for the vendored GPUI Kit code |
| `LICENSE-DOCS.md` | `LICENSE-DOCS.md` | CC-BY terms for GPUI Kit documentation |

Icon assets are **not** duplicated here: they are the payload of
`crates/gpui-kit-assets` (`assets/icons/*.svg`, 1830 files), which keeps
`LICENSE-LUCIDE` next to them.

## Licensing

GPUI Kit is Apache-2.0; the bundled Lucide icons are ISC. Both notices are
preserved (`LICENSE-APACHE` here and in `crates/gpui-base`,
`crates/gpui-component`, `crates/gpui-component-macros`,
`crates/gpui-kit`, `crates/gpui-kit-assets`, plus
`crates/gpui-kit-assets/LICENSE-LUCIDE`). Apache-2.0 is compatible with this
repository's own licenses, so no relicensing is required.

`LICENSE-DOCS.md` (CC-BY) is kept for completeness; no GPUI Kit documentation
pages were copied, only the code and the data files listed above.

## What changed on the way in

The vendored crates were retargeted from the published `gpui-pre-*` snapshots
(crates.io) onto this fork's own `gpui*` crates. That required a handful of
source adaptations, all recorded in the merge report:

- `line_wrapper` gained upstream's `IndentAdjustment` (backported *into*
  `crates/gpui`, so GPUI Kit's soft-wrap indentation is unchanged);
- `App::register_inspector_element` gained upstream's two-stage factory
  (backported *into* `crates/gpui`, with `crates/inspector_ui` updated);
- `Window::reuse_paint` gained upstream's debug-bounds replay for cached
  subtrees (`crates/gpui/src/window.rs` + `elements/div.rs`, all under
  `cfg(any(test, feature = "test-support"))`), which GPUI Kit's cached-panel
  selection test depends on;
- `lsp-types` is this fork's pinned fork rather than crates.io 0.97, so
  `Diagnostic.message`, `SemanticTokens.data`, `Url::scheme()` and a few
  Windows `windows`-crate signatures were adapted;
- `tree-sitter*` grams are taken from this fork's `[workspace.dependencies]`
  because every grammar crate sets `links = "tree-sitter-<lang>"`.

## Not built: the script host

`crates/gpui-shell`, `crates/gpui-component-shell` and
`crates/gpui-component-story` (plus the `dock` example, which needs the story
crate) are vendored in full but commented out of the workspace `members` list.

Their QuickJS dependencies are git-only, and `quickjs-jit`'s
`96593cd7a54440ac060a391b8baea7e9a3cf1393` never made it into cargo's git
database on the machine this was merged on: `git cat-file -t` reports "could
not get object info", so `--offline` resolution fails and every online retry
died with `Recv failure: Connection was reset` / `early EOF`. The checkout
directory `~/.cargo/git/checkouts/quickjs-jit-85f77f3fd15b7d92/96593cd` exists,
but cargo does not trust a checkout whose revision is missing from the database.

To re-enable, fetch that one revision and then restore the four commented
members, `crates/gpui-kit-examples/dock`, the three commented
`[workspace.dependencies]` entries and the commented `rquickjs` patch — the
exact command is in the comment next to the `members` list.

`crates/gpui-webview` was **not** vendored: GPUI Kit's webview layer requires
the GPUI Fast engine (a second, parallel GPUI fork published as `gpui-fast`),
and the `vendor/` directory the merge plan expected does not exist in the
source tree.

## Files brought in from the upstream repository root

| Path | Used by |
| --- | --- |
| `fonts/IBMPlexSans-Regular.ttf`, `fonts/OFL.txt` | `crates/gpui-base/examples/{motion,showcase}` |
