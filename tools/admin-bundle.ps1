#Requires -Version 5.1
<#
.SYNOPSIS
    Regenerates the admin demo's asset bundle from the versioned Topcoat cache.

.DESCRIPTION
    The admin demo (`Help > Admin Demo`) loads `<exe dir>\assets`. The asset
    declarations live in `zed.exe`, so the bundle is produced from that binary
    by the `admin-bundle-assets` tool. This script wires the versioned caches
    under `tools/topcoat-cache` into that run and keeps it offline:

    * sets `ADMIN_TAILWIND_CLI` to
      `tools/topcoat-cache/tailwind/tailwindcss-4.3.2-windows-x64.exe` when
      that file exists, so `crates/admin/build.rs` compiles `styles.css` from
      the repository cache instead of downloading the CLI;
    * sets `ADMIN_ASSETS_CACHE` to `tools/topcoat-cache/asset`, the committed
      font cache, so the bundler reuses fonts instead of downloading them;
    * builds `admin-bundle-assets` with `cargo build --offline` when the
      executable is missing or older than the crate's sources;
    * runs it against `target\debug\zed.exe` and `target\debug\assets`, then
      prints the bundle and whether the run downloaded anything.

    The Tailwind CLI is fetched by `tools/fetch-tailwind.ps1` when it is absent
    (unless -SkipFetch is given). It is not committed: at 107.27 MiB it is
    above GitHub's 100 MB per-file limit.

    The scripts in this directory are unsigned. With the default `AllSigned`
    execution policy PowerShell refuses to run them, so start it with
    `-ExecutionPolicy Bypass`:

        powershell -NoProfile -ExecutionPolicy Bypass -File tools\admin-bundle.ps1

.PARAMETER SkipFetch
    Never touch the network for the Tailwind CLI: skip `fetch-tailwind.ps1`
    even when the cache entry is missing. Everything else stays offline too
    (cargo runs with `--offline` and both caches are read locally).

.PARAMETER SkipBuild
    Use the existing `admin-bundle-assets.exe` even when it is missing or
    stale, and never invoke cargo.

.PARAMETER TargetDir
    Cargo target directory holding `debug\zed.exe`. Defaults to
    `<repo>\target`.

.PARAMETER TailwindCli
    Tailwind CLI executable to export as `ADMIN_TAILWIND_CLI`. Defaults to
    `tools/topcoat-cache/tailwind/tailwindcss-4.3.2-windows-x64.exe`.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\admin-bundle.ps1 -SkipFetch
#>
[CmdletBinding()]
param(
    [switch] $SkipFetch,
    [switch] $SkipBuild,
    [string] $TargetDir,
    [string] $TailwindCli
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Keep in sync with `DEFAULT_VERSION` in
# `crates/topcoat/crates/topcoat-tailwind/src/build/executable.rs`.
$CliName = 'tailwindcss-4.3.2-windows-x64.exe'

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$CacheRoot = Join-Path $PSScriptRoot 'topcoat-cache'
$AdminDir = Join-Path $RepoRoot 'crates\admin'

if (-not $TargetDir) {
    $TargetDir = Join-Path $RepoRoot 'target'
}
if (-not (Test-Path -LiteralPath $TargetDir -PathType Container)) {
    throw "target directory not found: $TargetDir (pass -TargetDir <dir>)"
}
$TargetDir = (Resolve-Path -LiteralPath $TargetDir).Path

$ZedExe = Join-Path $TargetDir 'debug\zed.exe'
$AssetsDir = Join-Path $TargetDir 'debug\assets'

function Test-BundlerStale {
    <#
    .SYNOPSIS
        Whether `Exe` is older than the crate's manifest, build script or
        sources.
    .DESCRIPTION
        `cargo build` would notice the same thing, but running it only when it
        has work keeps `-SkipBuild` honest and keeps the common case (nothing
        changed) fast.
    #>
    param([Parameter(Mandatory)] [string] $Exe)

    $exeTime = (Get-Item -LiteralPath $Exe).LastWriteTimeUtc
    $inputs = New-Object System.Collections.Generic.List[object]

    foreach ($name in @('Cargo.toml', 'Cargo.lock', 'build.rs')) {
        $path = Join-Path $AdminDir $name
        if (Test-Path -LiteralPath $path) {
            $inputs.Add((Get-Item -LiteralPath $path))
        }
    }
    $src = Join-Path $AdminDir 'src'
    if (Test-Path -LiteralPath $src) {
        $inputs.AddRange([object[]] (Get-ChildItem -LiteralPath $src -Recurse -File))
    }

    $newest = ($inputs | Sort-Object -Property LastWriteTimeUtc -Descending | Select-Object -First 1)
    return ($newest.LastWriteTimeUtc -gt $exeTime)
}

function Get-CacheSnapshot {
    <#
    .SYNOPSIS
        Maps each file name in `Path` to its size, to detect new downloads.
    #>
    param([Parameter(Mandatory)] [string] $Path)

    $snapshot = @{}
    if (Test-Path -LiteralPath $Path -PathType Container) {
        Get-ChildItem -LiteralPath $Path -File | ForEach-Object {
            $snapshot[$_.Name] = $_.Length
        }
    }
    return $snapshot
}

# 1. The Tailwind CLI, used by `crates/admin/build.rs` while cargo builds the
#    bundler. The bundler itself bundles the stylesheet already compiled into
#    zed.exe, so a missing CLI only degrades a rebuild, never the bundling.
if (-not $TailwindCli) {
    $TailwindCli = Join-Path $CacheRoot ('tailwind\' + $CliName)
}

if (-not (Test-Path -LiteralPath $TailwindCli -PathType Leaf)) {
    if ($SkipFetch) {
        Write-Warning "Tailwind CLI not found at $TailwindCli (-SkipFetch); existing stylesheet is kept."
    }
    else {
        Write-Host 'fetching the Tailwind CLI into tools\topcoat-cache\tailwind...'
        try {
            & (Join-Path $PSScriptRoot 'fetch-tailwind.ps1')
        }
        catch {
            Write-Warning "fetch-tailwind.ps1 failed: $($_.Exception.Message)"
        }
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $TailwindCli -PathType Leaf)) {
            Write-Warning "Tailwind CLI still missing after the fetch; existing stylesheet is kept."
        }
    }
}

if (Test-Path -LiteralPath $TailwindCli -PathType Leaf) {
    $env:ADMIN_TAILWIND_CLI = (Resolve-Path -LiteralPath $TailwindCli).Path
    Write-Host "ADMIN_TAILWIND_CLI   $env:ADMIN_TAILWIND_CLI"
}
else {
    # Without a CLI, `crates/admin/build.rs` falls through to Topcoat, which
    # downloads the release from GitHub without a timeout and can block a build
    # for many minutes on a slow network. `CARGO_NET_OFFLINE` makes the build
    # script skip that download immediately and warn instead.
    $env:CARGO_NET_OFFLINE = 'true'
    Write-Host 'ADMIN_TAILWIND_CLI   (unset; CARGO_NET_OFFLINE=true, so the build script skips the download)'
}

# 2. The font/icon cache the bundler reads.
$AssetCache = Join-Path $CacheRoot 'asset'
if (-not (Test-Path -LiteralPath $AssetCache -PathType Container)) {
    New-Item -ItemType Directory -Path $AssetCache -Force | Out-Null
}
$env:ADMIN_ASSETS_CACHE = (Resolve-Path -LiteralPath $AssetCache).Path
Write-Host "ADMIN_ASSETS_CACHE   $env:ADMIN_ASSETS_CACHE"

if (-not (Test-Path -LiteralPath $ZedExe -PathType Leaf)) {
    throw "zed.exe not found: $ZedExe (run 'cargo build -p zed', or pass -TargetDir)"
}

# 3. Locate or build `admin-bundle-assets`. `crates/admin` is its own cargo
#    workspace, so its binary may sit under either target layout, depending on
#    how it was built.
$BundlerExe = $null
foreach ($candidate in @(
        (Join-Path $TargetDir 'debug\admin-bundle-assets.exe'),
        (Join-Path $AdminDir 'target\debug\admin-bundle-assets.exe'))) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        $BundlerExe = (Resolve-Path -LiteralPath $candidate).Path
        break
    }
}

$needsBuild = $true
if ($BundlerExe -and -not (Test-BundlerStale -Exe $BundlerExe)) {
    $needsBuild = $false
}

if ($needsBuild -and -not $SkipBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo is not on PATH; install Rust or pass -SkipBuild'
    }
    Write-Host 'building admin-bundle-assets (cargo build --offline --bin admin-bundle-assets)'
    Push-Location -LiteralPath $AdminDir
    $buildExit = 1
    try {
        & cargo build --offline --bin admin-bundle-assets
        $buildExit = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }
    if ($buildExit -ne 0) {
        throw "cargo build failed with exit code $buildExit"
    }
    $BundlerExe = (Resolve-Path -LiteralPath (Join-Path $AdminDir 'target\debug\admin-bundle-assets.exe')).Path
}

if (-not $BundlerExe) {
    throw 'admin-bundle-assets.exe not found; build it with `cargo build --offline --bin admin-bundle-assets` in crates\admin'
}

# 4. Bundle, watching the cache so the run can report downloads.
Write-Host "bundler              $BundlerExe"
Write-Host "binary               $ZedExe"
Write-Host "out dir              $AssetsDir"

$before = Get-CacheSnapshot -Path $env:ADMIN_ASSETS_CACHE
& $BundlerExe $ZedExe $AssetsDir
$bundleExit = $LASTEXITCODE
if ($bundleExit -ne 0) {
    throw "admin-bundle-assets failed with exit code $bundleExit"
}

$after = Get-CacheSnapshot -Path $env:ADMIN_ASSETS_CACHE
$added = @($after.Keys | Where-Object { -not $before.ContainsKey($_) })
if ($added.Count -eq 0) {
    Write-Host "cache                $($after.Count) file(s) in the cache, 0 downloaded (cache hit)"
}
else {
    Write-Host "cache                $($added.Count) file(s) downloaded: $($added -join ', ')"
}

# 5. Report the bundle.
Write-Host ''
Write-Host "assets in $AssetsDir"
Get-ChildItem -LiteralPath $AssetsDir -File | Sort-Object Name | ForEach-Object {
    Write-Host ('  {0,-42} {1,10} bytes' -f $_.Name, $_.Length)
}

$Manifest = Join-Path $AssetsDir 'manifest.toml'
if (-not (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
    throw "manifest.toml was not written: $Manifest"
}
$versionLine = Get-Content -LiteralPath $Manifest |
    Where-Object { $_ -match '^\s*version\s*=' } |
    Select-Object -First 1
Write-Host "manifest             $Manifest ($versionLine)"

exit 0
