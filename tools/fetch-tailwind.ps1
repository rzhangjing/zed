#Requires -Version 5.1
<#
.SYNOPSIS
    Fetches the pinned Tailwind CLI into the versioned Topcoat cache.

.DESCRIPTION
    `crates/admin/build.rs` compiles `styles.css` with the standalone Tailwind
    CLI. The release binary is 112,477,184 bytes (107.27 MiB), which is above
    GitHub's 100 MB per-file limit, so it cannot be committed. The repository
    only ships the small caches under `tools/topcoat-cache/` (the Iconify
    Lucide set and four `.woff2` fonts); this script fetches the CLI into

        tools/topcoat-cache/tailwind/tailwindcss-4.3.2-windows-x64.exe

    `crates/admin/build.rs` and `tools/admin-bundle.ps1` read that path by
    default, so once this script has run no `ADMIN_TAILWIND_CLI` is needed.

    Behaviour:

    * If the destination already matches the pinned size and SHA-256, exit 0
      without touching the network (pass -Force to re-download).
    * Otherwise try each candidate URL in turn: the GitHub release first, then
      GitHub mirrors. Every URL is probed with `curl -I` before downloading,
      with a connect timeout and retries; a mirror that answers 403/404 (for
      example `cdn.jsdelivr.net`, which does not carry release assets) is
      skipped like an unreachable host.
    * A download is only moved into place after its size and SHA-256 match the
      pin, so an interrupted or truncated transfer never becomes a cache entry.
      A mismatch is deleted and the next candidate is tried.
    * If every candidate fails, print the manual instructions and exit 1.

    The scripts in this directory are unsigned. With the default `AllSigned`
    execution policy PowerShell refuses to run them, so start it with
    `-ExecutionPolicy Bypass`:

        powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-tailwind.ps1

.PARAMETER Destination
    Where to put the CLI. Defaults to the path above.

.PARAMETER Source
    Candidate URLs to use instead of the built-in GitHub + mirror list. Each
    value is used verbatim, in order.

.PARAMETER Force
    Download again even when the destination already matches the pin.

.PARAMETER Retries
    `curl --retry` count per candidate URL. Defaults to 3.

.PARAMETER ProbeTimeoutSeconds
    Timeout for the `curl -I` probe, reused as the per-connection timeout of
    the download. Defaults to 20.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-tailwind.ps1

.EXAMPLE
    # Use a specific mirror, replacing a stale cache entry.
    $url = 'https://ghproxy.net/https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.2/tailwindcss-windows-x64.exe'
    powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-tailwind.ps1 -Source $url -Force
#>
[CmdletBinding()]
param(
    [string] $Destination,
    [string[]] $Source,
    [switch] $Force,
    [int] $Retries = 3,
    [int] $ProbeTimeoutSeconds = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The pinned release. Keep in sync with `DEFAULT_VERSION` and
# `ExecutableSource::asset_name` in
# `crates/topcoat/crates/topcoat-tailwind/src/build/executable.rs`.
$Version = '4.3.2'
# The GitHub release asset name carries no version...
$AssetName = 'tailwindcss-windows-x64.exe'
# ...while Topcoat names its cache entries `tailwindcss-<version>-<platform>`,
# which is the name `crates/admin/build.rs` looks for.
$CacheFileName = "tailwindcss-$Version-windows-x64.exe"
$ExpectedSize = 112477184
$ExpectedSha256 = '224A62A8351D3B8DA9D950A4EB1D7176DC901DC4735B47F816F3DFCBC67D8654'

# The URL template Topcoat itself downloads from:
# `https://github.com/tailwindlabs/tailwindcss/releases/download/v<version>/<asset>`.
$GitHubUrl = "https://github.com/tailwindlabs/tailwindcss/releases/download/v$Version/$AssetName"

# GitHub proxies tried after the direct URL, in probe order. `ghproxy.net` is
# the one verified from this network; the rest are fallbacks. A prefix is
# prepended to the GitHub URL, so a proxy that does not resolve (or answers
# 403/404) is skipped exactly like an unreachable host.
$MirrorPrefixes = @(
    'https://ghproxy.net/'
    'https://gh-proxy.com/'
    'https://mirror.ghproxy.com/'
    'https://gh.llkk.cc/'
    'https://ghfast.top/'
)

if (-not $Destination) {
    $Destination = Join-Path $PSScriptRoot (Join-Path 'topcoat-cache\tailwind' $CacheFileName)
}
if (-not [System.IO.Path]::IsPathRooted($Destination)) {
    $Destination = Join-Path (Get-Location).Path $Destination
}

function Test-PinnedCli {
    <#
    .SYNOPSIS
        Checks that `Path` is a complete copy of the pinned CLI.
    .DESCRIPTION
        Returns a result with `Ok`, `Reason`, `Size` and `Sha256`. A size check
        comes first because it is free, and it rejects truncated downloads and
        HTML error pages without hashing 107 MB.
    #>
    param([Parameter(Mandatory)] [string] $Path)

    $result = [ordered]@{ Ok = $false; Reason = ''; Size = [long] 0; Sha256 = '' }

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        $result.Reason = 'file does not exist'
        return $result
    }

    $size = (Get-Item -LiteralPath $Path).Length
    if ($size -ne $ExpectedSize) {
        $result.Reason = "size is $size bytes, expected $ExpectedSize"
        return $result
    }

    $sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($sha256 -ne $ExpectedSha256) {
        $result.Reason = "sha256 is $sha256, expected $ExpectedSha256"
        return $result
    }

    $result.Ok = $true
    $result.Size = $size
    $result.Sha256 = $sha256
    return $result
}

function Get-UrlStatus {
    <#
    .SYNOPSIS
        Probes `Url` with a HEAD request and returns the HTTP status, or 0.
    .DESCRIPTION
        Redirects are followed, because the GitHub release URL answers with a
        302 to `objects.githubusercontent.com`. Native stderr is deliberately
        not redirected: PowerShell 5.1 turns a redirected native stderr stream
        into a NativeCommandError when the error action preference is `Stop`.
    #>
    param([Parameter(Mandatory)] [string] $Url)

    $status = & curl.exe --silent --show-error --head --location `
        --max-time $ProbeTimeoutSeconds --output NUL --write-out '%{http_code}' $Url
    if ($LASTEXITCODE -ne 0) {
        return 0
    }

    $text = ($status | ForEach-Object { "$_" } | Select-Object -Last 1)
    if (-not $text) {
        return 0
    }

    $code = 0
    if (-not [int]::TryParse($text.Trim(), [ref] $code)) {
        return 0
    }
    return $code
}

function Save-PinnedCli {
    <#
    .SYNOPSIS
        Downloads `Url` to `Path`, keeping it only if size and hash match.
    .DESCRIPTION
        The transfer lands in `<Path>.download` and is verified before the
        rename, so a failed or truncated transfer never replaces a good cache
        entry and never leaves a half-written executable behind.
    #>
    param(
        [Parameter(Mandatory)] [string] $Url,
        [Parameter(Mandatory)] [string] $Path
    )

    $temp = "$Path.download"
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue

    Write-Host '  downloading (107.27 MiB, no progress bar)...'
    & curl.exe --silent --show-error --location --fail --retry $Retries --retry-delay 2 `
        --connect-timeout $ProbeTimeoutSeconds --max-time 1800 --output $temp $Url
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  download failed (curl exit code $LASTEXITCODE)" -ForegroundColor DarkYellow
        Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
        return $false
    }

    $check = Test-PinnedCli -Path $temp
    if (-not $check.Ok) {
        Write-Host "  rejected: $($check.Reason)" -ForegroundColor DarkYellow
        Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
        return $false
    }

    Move-Item -LiteralPath $temp -Destination $Path -Force
    return $true
}

Write-Host "tailwind cli  $Version ($AssetName)"
Write-Host "destination   $Destination"

if ($env:PROCESSOR_ARCHITECTURE -and $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Write-Warning "only the windows-x64 asset is pinned; this host reports $env:PROCESSOR_ARCHITECTURE"
}

$current = Test-PinnedCli -Path $Destination
if ($current.Ok -and -not $Force) {
    Write-Host 'cache hit     size and sha256 match; nothing to download'
    Write-Host "sha256        $($current.Sha256)"
    exit 0
}
if (Test-Path -LiteralPath $Destination) {
    Write-Host "stale entry   $($current.Reason)"
}

$parent = Split-Path -Parent $Destination
if (-not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
}

if ($Source) {
    $candidates = @($Source)
}
else {
    $candidates = @($GitHubUrl)
    foreach ($prefix in $MirrorPrefixes) {
        $candidates += "$prefix$GitHubUrl"
    }
}

$index = 0
foreach ($url in $candidates) {
    $index++
    Write-Host "[$index/$($candidates.Count)] $url"

    $status = Get-UrlStatus -Url $url
    if ($status -lt 200 -or $status -ge 300) {
        Write-Host "  probe failed (HTTP $status); trying the next mirror" -ForegroundColor DarkYellow
        continue
    }
    Write-Host "  probe ok (HTTP $status)"

    if (Save-PinnedCli -Url $url -Path $Destination) {
        $saved = Test-PinnedCli -Path $Destination
        Write-Host "saved         $Destination"
        Write-Host "size          $($saved.Size) bytes"
        Write-Host "sha256        $($saved.Sha256)"
        Write-Host "source        $url"
        exit 0
    }

    Write-Host '  verification failed; trying the next mirror' -ForegroundColor DarkYellow
}

Write-Host ''
Write-Host "error: could not download the Tailwind CLI $Version ($AssetName)." -ForegroundColor Red
Write-Host 'Every candidate URL was unreachable, or returned a file that did not match the pin.'
Write-Host ''
Write-Host 'Put the release binary in place by hand:'
Write-Host "  path:            $Destination"
Write-Host "  expected size:   $ExpectedSize bytes"
Write-Host "  expected sha256: $ExpectedSha256"
Write-Host ''
Write-Host 'Download it with a browser from:'
Write-Host "  $GitHubUrl"
Write-Host 'and verify it with:'
Write-Host '  Get-FileHash -LiteralPath "<downloaded file>" -Algorithm SHA256'
Write-Host ''
Write-Host 'Or point ADMIN_TAILWIND_CLI at any copy of the same release:'
Write-Host '  crates/admin/build.rs and tools/admin-bundle.ps1 both honour that variable.'
exit 1
