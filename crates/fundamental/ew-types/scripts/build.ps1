# 构建脚本。Windows 上若源码路径含中文，msys2 链接器会失败，
# 这里默认把产物目录指到纯英文路径；已设置 CARGO_TARGET_DIR 时尊重该设置。
param(
    [switch]$Release,
    [switch]$Test
)

$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')

try {
    if (-not $env:CARGO_TARGET_DIR) {
        $env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ew-types-target'
    }
    Write-Host "CARGO_TARGET_DIR = $env:CARGO_TARGET_DIR"

    if ($Test) {
        cargo test
    } else {
        if ($Release) { cargo build --release } else { cargo build }
        $profile = if ($Release) { 'release' } else { 'debug' }
        Write-Host "产物：$env:CARGO_TARGET_DIR\$profile\ew-types.exe"
    }
}
finally {
    Pop-Location
}
