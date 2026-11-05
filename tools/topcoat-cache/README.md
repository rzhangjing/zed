# Versioned Topcoat cache for the admin demo

`crates/admin` (the embedded `Help > Admin Demo`) compiles its stylesheet and
stages its Iconify set through Topcoat's build scripts, and bundles its fonts
with `admin-bundle-assets`. Both normally read `<target>/topcoat/cache/…`,
which `cargo clean` deletes and which is never committed. This directory keeps
the small inputs under version control so a fresh checkout builds the demo
offline, and it is what the build reads by default:

| Path | Contents | Size | Committed |
| --- | --- | --- | --- |
| `iconify/lucide.json` | `@iconify-json/lucide` icon set | 627,576 B | yes |
| `asset/*.woff2` | four Fontsource Latin subsets | ~13 KB each | yes |
| `tailwind/tailwindcss-4.3.2-windows-x64.exe` | pinned Tailwind CLI | 112,477,184 B | **no** |

The CLI is above GitHub's 100 MB per-file limit, so it is not committed. The
repository's root `.gitignore` keeps any local copy (and its `.download`/`.lock`
files) out of `git status`; only `tailwind/.gitkeep` is tracked.

## Populating the CLI

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-tailwind.ps1
```

The script is a no-op (exit 0) when the destination already matches the pin.
Otherwise it probes the GitHub release and a list of GitHub mirrors with
`curl -I`, downloads the first reachable one, and verifies:

* size `112,477,184` bytes
* SHA-256 `224A62A8351D3B8DA9D950A4EB1D7176DC901DC4735B47F816F3DFCBC67D8654`

A file that fails verification is deleted and the next mirror is tried. With
`-Source <url>` you can supply your own URL(s), and `-Force` re-downloads over
an existing entry.

## Building and bundling

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools\admin-bundle.ps1
```

It exports `ADMIN_TAILWIND_CLI` and `ADMIN_ASSETS_CACHE` to this directory,
builds `admin-bundle-assets` if needed, and regenerates `target\debug\assets`
from `target\debug\zed.exe`. `-SkipFetch` never touches the network, and
`-SkipBuild` reuses the existing bundler binary.

## Reading order in the build

`crates/admin/build.rs` resolves the Tailwind CLI as:

1. `ADMIN_TAILWIND_CLI` (explicit override)
2. `tools/topcoat-cache/tailwind/tailwindcss-<version>-<platform>` — this
   directory, when the file has been fetched
3. `<target>/topcoat/cache/tailwind/…`
4. `<target>/admin-tw-cache/…`
5. otherwise Topcoat downloads the pinned release

`admin-bundle-assets` resolves its asset cache as:

1. `ADMIN_ASSETS_CACHE`
2. `tools/topcoat-cache/asset` — found by walking up from the working
   directory, then from the binary's directory
3. `<target>/topcoat/cache/asset`

## Refreshing

Copy the two small files back from `<target>/topcoat/cache/` when the pinned
Topcoat version changes, and update `$Version`, `$AssetName`, `$ExpectedSize`
and `$ExpectedSha256` in `tools/fetch-tailwind.ps1` together with the file name
in `tools/admin-bundle.ps1`.
