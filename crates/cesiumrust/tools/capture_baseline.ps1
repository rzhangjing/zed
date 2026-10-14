# capture_baseline.ps1 — M0.1 Visual Baseline Capture Harness
# ============================================================
# Captures deterministic globe screenshots for pixel-level regression testing.
# Idempotent: re-running overwrites previous baselines with fresh captures.
#
# Usage:
#   .\tools\capture_baseline.ps1              # capture all baselines
#   .\tools\capture_baseline.ps1 -Build       # rebuild release binary first
#   .\tools\capture_baseline.ps1 -Only globe_default,globe_pole_north
#
# Requirements:
#   - Windows PowerShell 5.1+ (no && operator)
#   - NVIDIA GPU with Vulkan support (Bevy wgpu backend)
#   - Network access for Bing Maps tile downloads
#
# Note: Due to async tile loading + GPU non-determinism, consecutive runs may
# produce files differing by a few bytes. Visual content is stable at frame 600.

param(
    [switch]$Build,
    [string[]]$Only = @(),
    [int]$Frame = 600,
    [string]$OutputDir = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# ── Paths ────────────────────────────────────────────────────────────────────
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Binary = Join-Path $RepoRoot "target\release\cesium-app.exe"
if ($OutputDir -eq "") {
    $OutputDir = Join-Path $RepoRoot "specs\baselines\v0"
}

# ── Git SHA + dirty state (MK6) ──────────────────────────────────────────────
Push-Location $RepoRoot
$prevEAPgit = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$GitSha = (git rev-parse HEAD 2>&1 | Where-Object { $_ -notmatch '^warning:' }) -join ''
if (-not $GitSha) { $GitSha = "unknown" }
$GitDirty = [bool](git status --porcelain 2>&1 | Where-Object { $_ -notmatch '^warning:' })
$GitDiffContent = ((git diff HEAD 2>&1) | Where-Object { $_ -notmatch '^warning:' }) -join "`n"
$ErrorActionPreference = $prevEAPgit
if ($GitDiffContent) {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($GitDiffContent)
    $GitDiffSha256 = ([BitConverter]::ToString($sha256.ComputeHash($bytes)) -replace '-','').ToUpper()
} else {
    $GitDiffSha256 = "clean"
}
Pop-Location

# ── Baseline Definitions ─────────────────────────────────────────────────────
# Each entry: Name, Frame, EnvOverrides (hashtable), Description
$Baselines = @(
    @{
        Name = "globe_default"
        Frame = 180
        Env = @{}
        Description = "Default camera (lon=0, lat=23, dist=3.0), no terrain/tileset. Pristine regression anchor."
        Camera = @{ lon = 0; lat = 22.9; height = 2.0; heading = 0; pitch = 22.9 }
    },
    @{
        Name = "globe_pole_north"
        Frame = 600
        Env = @{ CESIUM_CAM_LON = "0"; CESIUM_CAM_LAT = "80"; CESIUM_CAM_HEIGHT = "2.0" }
        Description = "North pole view (lat=80N), Arctic ice cap + polar cap mesh."
        Camera = @{ lon = 0; lat = 80; height = 2.0; heading = 0; pitch = 80 }
    },
    @{
        Name = "globe_pole_south"
        Frame = 600
        Env = @{ CESIUM_CAM_LON = "0"; CESIUM_CAM_LAT = "-80"; CESIUM_CAM_HEIGHT = "2.0" }
        Description = "South pole view (lat=80S), Antarctic ice + polar cap mesh."
        Camera = @{ lon = 0; lat = -80; height = 2.0; heading = 0; pitch = -80 }
    },
    @{
        Name = "globe_zoom_mid"
        Frame = 600
        Env = @{ CESIUM_CAM_LON = "0"; CESIUM_CAM_LAT = "23"; CESIUM_CAM_HEIGHT = "1.0" }
        Description = "Medium zoom (height=1.0 above surface), Africa/Atlantic view."
        Camera = @{ lon = 0; lat = 23; height = 1.0; heading = 0; pitch = 23 }
    },
    @{
        Name = "globe_zoom_close"
        Frame = 600
        Env = @{ CESIUM_CAM_LON = "0"; CESIUM_CAM_LAT = "23"; CESIUM_CAM_HEIGHT = "0.2" }
        Description = "Close zoom (height=0.2 above surface), high-detail tile LOD."
        Camera = @{ lon = 0; lat = 23; height = 0.2; heading = 0; pitch = 23 }
    },
    @{
        Name = "globe_terrain_off"
        Frame = 600
        Env = @{ CESIUM_ENABLE_TERRAIN = "0" }
        Description = "Default camera with terrain explicitly disabled (same as default in current config)."
        Camera = @{ lon = 0; lat = 22.9; height = 2.0; heading = 0; pitch = 22.9 }
    },
    @{
        Name = "globe_pacific"
        Frame = 600
        Env = @{ CESIUM_CAM_LON = "180"; CESIUM_CAM_LAT = "0"; CESIUM_CAM_HEIGHT = "2.0" }
        Description = "Pacific Ocean view (lon=180, equator), mostly water tiles."
        Camera = @{ lon = 180; lat = 0; height = 2.0; heading = 180; pitch = 0 }
    },
    @{
        Name = "globe_europe"
        Frame = 600
        Env = @{ CESIUM_CAM_LON = "10"; CESIUM_CAM_LAT = "48"; CESIUM_CAM_HEIGHT = "1.5" }
        Description = "Europe view (lon=10E, lat=48N), mixed land/ocean tiles."
        Camera = @{ lon = 10; lat = 48; height = 1.5; heading = 10; pitch = 48 }
    }
)

# ── Filter if -Only specified ────────────────────────────────────────────────
if ($Only.Count -gt 0) {
    $Baselines = $Baselines | Where-Object { $Only -contains $_.Name }
}

# ── Build if requested ───────────────────────────────────────────────────────
if ($Build) {
    Write-Host "[capture] Building release binary..." -ForegroundColor Cyan
    Push-Location $RepoRoot
    cargo build --release -p cesium-app
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw "Build failed" }
    Pop-Location
}

if (-not (Test-Path $Binary)) {
    throw "Binary not found: $Binary. Run with -Build or build manually first."
}

# ── Ensure output directory ──────────────────────────────────────────────────
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

# ── F1: Helper to read real PNG dimensions from IHDR chunk ────────────────────
function Get-PngDimensions([string]$Path) {
    $fs = [System.IO.File]::OpenRead($Path)
    try {
        $buf = New-Object byte[] 24
        $null = $fs.Read($buf, 0, 24)
        # PNG IHDR: bytes 16-19 = width (big-endian u32), 20-23 = height
        $w = ([uint32]$buf[16] -shl 24) -bor ([uint32]$buf[17] -shl 16) -bor ([uint32]$buf[18] -shl 8) -bor [uint32]$buf[19]
        $h = ([uint32]$buf[20] -shl 24) -bor ([uint32]$buf[21] -shl 16) -bor ([uint32]$buf[22] -shl 8) -bor [uint32]$buf[23]
        return @{ Width = $w; Height = $h }
    } finally { $fs.Close() }
}

# ── MK6: Temp file for stderr capture (GPU adapter info) ──────────────────────
$StderrLog = Join-Path $OutputDir "_capture_stderr.log"

# ── Capture Loop ─────────────────────────────────────────────────────────────
$Timestamp = Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ"
$Captured = @()

foreach ($bl in $Baselines) {
    $name = $bl.Name
    $pngPath = Join-Path $OutputDir "$name.png"
    $metaPath = Join-Path $OutputDir "$name.metadata.json"

    Write-Host "[capture] $name (frame=$($bl.Frame))..." -ForegroundColor Yellow

    # Set common env
    $env:CESIUM_ENABLE_NEW_CHAINS = "0"
    $env:CESIUM_SCREENSHOT_AT_FRAME = "$($bl.Frame)"
    $env:CESIUM_SCREENSHOT = $pngPath

    # Clear all camera env vars first (ensure no leakage between captures)
    @("CESIUM_CAM_LON", "CESIUM_CAM_LAT", "CESIUM_CAM_HEIGHT",
      "CESIUM_CAM_HEADING", "CESIUM_CAM_PITCH", "CESIUM_CAM_DISTANCE",
      "CESIUM_ENABLE_TERRAIN", "CESIUM_ENABLE_TILESET") | ForEach-Object {
        Remove-Item "Env:\$_" -ErrorAction SilentlyContinue
    }

    # Apply baseline-specific env overrides
    foreach ($kv in $bl.Env.GetEnumerator()) {
        Set-Item "Env:\$($kv.Key)" -Value $kv.Value
    }

    # Run the app directly (inherits env vars; auto-exits at frame+30 via AppExit)
    # Capture stderr for GPU adapter info (MK6).
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $Binary 2>$StderrLog | Out-Null
    $ErrorActionPreference = $prevEAP

    # Verify output
    if (-not (Test-Path $pngPath)) {
        Write-Host "[capture] ERROR: $pngPath not created!" -ForegroundColor Red
        continue
    }

    $fileSize = (Get-Item $pngPath).Length
    $fileHash = (Get-FileHash $pngPath -Algorithm SHA256).Hash

    # F1: Read real PNG dimensions (not hardcoded)
    $dims = Get-PngDimensions $pngPath
    $resolution = "$($dims.Width)x$($dims.Height)"

    # MK6: Extract GPU adapter from Bevy stderr log
    $gpuName = "unknown"
    if (Test-Path $StderrLog) {
        $gpuMatch = Select-String -Path $StderrLog -Pattern 'AdapterInfo.*?name:\s*"([^"]+)"' | Select-Object -First 1
        if ($gpuMatch) {
            $gpuName = $gpuMatch.Matches[0].Groups[1].Value
        } else {
            # Fallback: try simpler Bevy 0.15 log pattern
            $gpuMatch2 = Select-String -Path $StderrLog -Pattern 'adapter.*?:\s*(.+?)(?:\s*\(|$)' | Select-Object -First 1
            if ($gpuMatch2) { $gpuName = $gpuMatch2.Matches[0].Groups[1].Value.Trim() }
        }
    }

    # M3: Read camera JSON written by the app at screenshot frame
    $camJsonPath = [System.IO.Path]::ChangeExtension($pngPath, $null).TrimEnd('.') + ".camera.json"
    $cameraState = $null
    if (Test-Path $camJsonPath) {
        $cameraState = Get-Content $camJsonPath -Raw | ConvertFrom-Json
    }

    # MK6: Binary path relative to repo root
    $binaryRel = $Binary.Replace("$RepoRoot\", "").Replace("\", "/")

    # Write metadata JSON
    $meta = @{
        name = $name
        description = $bl.Description
        captured_at = $Timestamp
        git_sha = $GitSha
        git_dirty = $GitDirty
        git_diff_sha256 = $GitDiffSha256
        frame = $bl.Frame
        resolution = $resolution
        file_size_bytes = $fileSize
        sha256 = $fileHash
        camera = $bl.Camera
        camera_state = $cameraState
        env = ($bl.Env + @{
            CESIUM_ENABLE_NEW_CHAINS = "0"
            CESIUM_SCREENSHOT_AT_FRAME = "$($bl.Frame)"
            CESIUM_SCREENSHOT = "specs/baselines/v0/$name.png"
        })
        binary = $binaryRel
        gpu = $gpuName
        notes = "Async Bing tile loading causes minor run-to-run variance (~0.001%). Visual content is stable. Resolution reflects physical pixels (DPI-dependent; see deferred DEFER-M0-DPI)."
    } | ConvertTo-Json -Depth 5

    Set-Content -Path $metaPath -Value $meta -Encoding UTF8
    $Captured += $name
    Write-Host "[capture] $name OK ($fileSize bytes, sha256=$($fileHash.Substring(0,12))...)" -ForegroundColor Green
}

# ── Cleanup env vars ─────────────────────────────────────────────────────────
@("CESIUM_CAM_LON", "CESIUM_CAM_LAT", "CESIUM_CAM_HEIGHT",
  "CESIUM_CAM_HEADING", "CESIUM_CAM_PITCH", "CESIUM_CAM_DISTANCE",
  "CESIUM_ENABLE_NEW_CHAINS", "CESIUM_ENABLE_TERRAIN", "CESIUM_ENABLE_TILESET",
  "CESIUM_SCREENSHOT_AT_FRAME", "CESIUM_SCREENSHOT") | ForEach-Object {
    Remove-Item "Env:\$_" -ErrorAction SilentlyContinue
}
# Remove temp stderr log
Remove-Item $StderrLog -ErrorAction SilentlyContinue

# ── Summary ──────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host " Baseline Capture Complete" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host " Captured: $($Captured.Count) / $($Baselines.Count)"
Write-Host " Output:   $OutputDir"
Write-Host " Git SHA:  $GitSha"
Write-Host " Time:     $Timestamp"
Write-Host ""
foreach ($n in $Captured) {
    Write-Host "   [OK] $n.png" -ForegroundColor Green
}
Write-Host ""
