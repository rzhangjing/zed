#!/usr/bin/env pwsh
<#
.SYNOPSIS
    把 `docs/设计方案.md` §10.5 的 CI 流水线在**本机**可复现地跑一遍（Windows 侧）。

.DESCRIPTION
    每个阶段都是「一条真命令 + 一个非零退出码即判红」，日志落在 `target/gates/NN-<阶段>.log`。
    脚本本身不造假证据：阶段失败就打印尾部输出并以非零码退出。

    阶段分组：
      * 离线组（默认全跑，不需要数据库）：依赖方向、feature 矩阵、fmt、clippy、L1 单测、
        L2 契约、文档链接、doctest、toasty fork golden、toasty 驱动离线测试。
      * 交叉编译组（默认跑，目标未安装则跳过）：aarch64 / x86_64 `cargo check`（AC-08 的类型级验证，
        本机没有交叉链接器 ⇒ 只能 `check`，见 §10.5 `cross` 行）。
      * 真机组（需要 `DM_TEST_DSN`，默认跑；`-SkipReal` 可跳过）：官方一致性套件、toasty 真机、
        `dm-odbc` 集成、`dm-query`、`dm-migrate`、`dm-pool` 长稳、示例。
      * 可选组：`-Coverage`（AC-01 覆盖率，需要 cargo-llvm-cov）。

.EXAMPLE
    pwsh scripts/gates.ps1                         # 离线 + 交叉 + 真机（DSN 已设时）
    pwsh scripts/gates.ps1 -SkipReal               # 只跑离线 + 交叉（CI 的默认形态）
    pwsh scripts/gates.ps1 -Only fmt,clippy        # 只跑名字匹配的阶段
    pwsh scripts/gates.ps1 -SkipCross -SkipSoak    # 跳过交叉与 70 s 的池长稳

.NOTES
    ① 本文件必须以 **UTF-8 with BOM** 保存：Windows PowerShell 5.1 对无 BOM 的 UTF-8 会按 ANSI（本机 CP936）
       解码，中文字节被打碎后会把字符串/括号一起弄坏，脚本直接语法错误（实测踩过）。用 `pwsh`（7.x）不受影响。
    ② 若本机策略是 Restricted，用 `powershell -ExecutionPolicy Bypass -File scripts/gates.ps1 …`；
       PowerShell 7 与放开策略的机器上可直接 `pwsh scripts/gates.ps1 …`。
    ③ 每个阶段是一条真命令，非零退出码即判红；脚本不造假证据（不做「失败也记通过」的兼容）。
       唯一**显式**的例外是官方一致性套件：它有一条已登记在册的缺陷（非 BMP 文本，
       `type_primitives::ty_str`，见 `docs/toasty-fork.md`「真机实测发现」第 16 条），
       所以该阶段按「失败条数恰为 1 且失败名在白名单里」判 PASS，并把放行的那一条打印出来；
       要全绿请加 `-StrictSuite`。
    ④ 真机阶段的 `--test-threads=1` 不可省：这些用例共用固定表名（`DM_Q_REAL_SEED`、迁移记录表…），
       并行会互相 DROP/建表 ⇒ 假失败（实测不加时 `dm-query` 17 条红、`dm-migrate` 2 条红、
       `dm-pool` 的会话数被别的用例抬高）。README「真机」段是同一口径。
#>
[CmdletBinding()]
param(
    # 跳过所有需要 DM_TEST_DSN 的真机阶段。
    [switch]$SkipReal,
    # 跳过交叉编译阶段。
    [switch]$SkipCross,
    # 跳过 `dm-pool` 的 70 s 长稳用例（其余真机阶段照跑）。
    [switch]$SkipSoak,
    # 额外跑 AC-01 的覆盖率采集（需要 cargo-llvm-cov，慢）。
    [switch]$Coverage,
    # 只跑名字里含这些子串的阶段（逗号分隔）。
    [string[]]$Only = @(),
    # 官方一致性套件要求**全绿**（关掉「已知缺陷白名单」，审计/发布时用）。
    [switch]$StrictSuite,
    # cargo 工具链选择子；Windows 上必须用 msvc 宿主（见 README「环境要求」）。
    [string]$Toolchain = '+stable-x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# `pwsh -File script.ps1 -Only a,b` 会把 `a,b` 当**一个**字符串传进来（`-File` 不做数组绑定），
# 所以在脚本里再按逗号/空白拆一次，保证两种调用方式（-File / 会话内 &）语义一致。
$Only = @($Only | ForEach-Object { $_ -split '[,\s]+' } | Where-Object { $_ })

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$logDir = Join-Path $repo 'target/gates'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

$script:CurrentLog = $null
$script:CurrentName = $null

function Write-Head([string]$text) {
    Write-Host ''
    Write-Host "=== $text ===" -ForegroundColor Cyan
}

# 跑一条 cargo 命令；日志追加进当前阶段的日志文件；非零退出码即抛。
function Invoke-Cargo {
    param(
        [Parameter(Mandatory)][string[]]$CargoArgs,
        [hashtable]$Env
    )
    # 工具链选择子必须紧跟 `cargo`（`cargo +toolchain fmt`）⇒ 统一挪到最前面；
    # 调用点把它写在子命令之后也能容忍（`cargo fmt +toolchain` 会被 cargo 拒绝）。
    $prefix = @()
    if (-not [string]::IsNullOrWhiteSpace($Toolchain)) { $prefix = @($Toolchain) }
    $argsList = $prefix + @($CargoArgs | Where-Object { $_ -ne $Toolchain })
    $saved = @{}
    if ($Env) {
        foreach ($k in $Env.Keys) {
            $saved[$k] = [Environment]::GetEnvironmentVariable($k)
            Set-Item -Path "env:$k" -Value $Env[$k]
        }
    }
    # native 命令的 stderr 在 `2>&1` + `$ErrorActionPreference='Stop'` 下会被 PowerShell 当成
    # terminating error（cargo 的 `Compiling …` / `Updating …` 走的正是 stderr）⇒ 调用期间显式放宽，
    # 否则任何一次真编译都会被误判成阶段失败。
    $oldEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & cargo @argsList 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $oldEap
        if ($Env) {
            foreach ($k in $Env.Keys) {
                if ($null -eq $saved[$k]) {
                    Remove-Item "env:$k" -ErrorAction SilentlyContinue
                } else {
                    Set-Item -Path "env:$k" -Value $saved[$k]
                }
            }
        }
    }
    $out | Out-File -FilePath $script:CurrentLog -Append -Encoding UTF8
    if ($code -ne 0) {
        Write-Host "  cargo $($argsList -join ' ')" -ForegroundColor DarkGray
        ($out | Select-Object -Last 40) | ForEach-Object { Write-Host "  | $_" }
        throw "cargo $($argsList -join ' ') 退出码 $code"
    }
}

# 跑一条 cargo 命令但**不抛异常**：返回退出码，输出留在 `$script:LastRawOut`。
# （不要用「返回 @(码, 输出) 再解构」的写法 —— Windows PowerShell 5.1 里
#  `$a, $b = <单个数组对象>` **不会**解构，会把整个数组塞给 $a、$b 留空，实测踩过。）
$script:LastRawOut = @()
function Invoke-CargoRaw {
    param(
        [Parameter(Mandatory)][string[]]$CargoArgs,
        [hashtable]$Env
    )
    $prefix = @()
    if (-not [string]::IsNullOrWhiteSpace($Toolchain)) { $prefix = @($Toolchain) }
    $argsList = $prefix + @($CargoArgs | Where-Object { $_ -ne $Toolchain })
    $saved = @{}
    if ($Env) {
        foreach ($k in $Env.Keys) {
            $saved[$k] = [Environment]::GetEnvironmentVariable($k)
            Set-Item -Path "env:$k" -Value $Env[$k]
        }
    }
    $oldEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & cargo @argsList 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $oldEap
        if ($Env) {
            foreach ($k in $Env.Keys) {
                if ($null -eq $saved[$k]) { Remove-Item "env:$k" -ErrorAction SilentlyContinue }
                else { Set-Item -Path "env:$k" -Value $saved[$k] }
            }
        }
    }
    $out | Out-File -FilePath $script:CurrentLog -Append -Encoding UTF8
    Write-Host "  cargo $($argsList -join ' ')" -ForegroundColor DarkGray
    $script:LastRawOut = @($out)
    return [int]$code
}

# 跑一条任意命令（示例程序等）；同样非零即抛。
function Invoke-Tool {
    param(
        [Parameter(Mandatory)][string]$Exe,
        [Parameter(Mandatory)][string[]]$Args,
        [hashtable]$Env
    )
    $prefix = @()
    if (-not [string]::IsNullOrWhiteSpace($Toolchain)) { $prefix = @($Toolchain) }
    $argsList = $prefix + @($Args | Where-Object { $_ -ne $Toolchain })
    $saved = @{}
    if ($Env) {
        foreach ($k in $Env.Keys) {
            $saved[$k] = [Environment]::GetEnvironmentVariable($k)
            Set-Item -Path "env:$k" -Value $Env[$k]
        }
    }
    # 同 Invoke-Cargo：native stderr 在 Stop 偏好下会被误判为终止错误。
    $oldEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & $Exe @argsList 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $oldEap
        if ($Env) {
            foreach ($k in $Env.Keys) {
                if ($null -eq $saved[$k]) { Remove-Item "env:$k" -ErrorAction SilentlyContinue }
                else { Set-Item -Path "env:$k" -Value $saved[$k] }
            }
        }
    }
    $out | Out-File -FilePath $script:CurrentLog -Append -Encoding UTF8
    if ($code -ne 0) {
        ($out | Select-Object -Last 40) | ForEach-Object { Write-Host "  | $_" }
        throw "$Exe $($argsList -join ' ') 退出码 $code"
    }
}

function Test-TargetInstalled([string]$target) {
    # 没有 rustup（例如用发行版包管理器装的 Rust）时按「未安装」处理 ⇒ 交叉阶段 SKIP。
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) { return $false }
    # ⚠️ 必须问**同一个**工具链：`rustup target list --installed` 默认只看 default 工具链，
    # 而这里跑的是 `+stable-x86_64-pc-windows-msvc`，两者装的目标可以不同（实测踩过：
    # 交叉阶段被误判成 SKIP，而 `cargo check --target …` 其实能跑）。
    $tcArgs = @('target', 'list', '--installed')
    if (-not [string]::IsNullOrWhiteSpace($Toolchain)) {
        $tcArgs += @('--toolchain', $Toolchain.TrimStart('+'))
    }
    $oldEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $installed = & rustup @tcArgs 2>&1 }
    finally { $ErrorActionPreference = $oldEap }
    return ($installed | Select-String -SimpleMatch $target) -ne $null
}

$dsn = $env:DM_TEST_DSN
$hasDsn = -not [string]::IsNullOrWhiteSpace($dsn)

# 阶段表：名字 / 分组 / 需要的开关 / 动作。动作是脚本块，失败用 throw。
$stages = [ordered]@{}

$stages['deps-l1'] = @{
    Group = 'offline'
    Body  = {
        # §10.5 `deps` 行：L1 运行时无关（不得把 tokio / odbc 拖进 dm-core 的正常依赖）。
        $oldEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { $tree = & cargo tree -p dm-core 2>&1; $treeCode = $LASTEXITCODE }
        finally { $ErrorActionPreference = $oldEap }
        $tree | Out-File -FilePath $script:CurrentLog -Append -Encoding UTF8
        if ($treeCode -ne 0) { throw 'cargo tree -p dm-core 退出码非零' }
        $bad = $tree | Select-String -Pattern '(^|\s)(tokio|odbc)(\s|$)' | Select-Object -First 5
        if ($bad) {
            $bad | ForEach-Object { Write-Host "  | $_" }
            throw 'dm-core 的正常依赖里出现了 tokio / odbc'
        }
        Write-Host '  dm-core 依赖树无 tokio / odbc ✓'
    }
}

$stages['feature-matrix'] = @{
    Group = 'offline'
    Body  = {
        # §10.5「Feature 矩阵」：这些组合必须编译通过（跨 feature 组合是本项目的高发坑）。
        Invoke-Cargo @('check', $Toolchain, '--workspace', '--all-targets')
        Invoke-Cargo @('check', $Toolchain, '--workspace', '--all-targets', '--no-default-features')
        Invoke-Cargo @('check', $Toolchain, '--workspace', '--all-targets', '--all-features')
        Invoke-Cargo @('check', $Toolchain, '-p', 'dm-core', '--all-targets', '--no-default-features', '--features', 'decimal')
        Invoke-Cargo @('check', $Toolchain, '-p', 'dm-core', '--all-targets', '--no-default-features', '--features', 'chrono')
        Invoke-Cargo @('check', $Toolchain, '-p', 'dm-query', '--all-targets', '--no-default-features', '--features', 'decimal')
        Invoke-Cargo @('check', $Toolchain, '-p', 'dm-query', '--all-targets', '--no-default-features', '--features', 'chrono')
        Invoke-Cargo @('check', $Toolchain, '-p', 'dm-pool', '--all-targets', '--features', 'rw-split')
    }
}

$stages['fmt'] = @{
    Group = 'offline'
    Body  = { Invoke-Cargo @('fmt', $Toolchain, '--all', '--', '--check') }
}

$stages['clippy'] = @{
    Group = 'offline'
    Body  = { Invoke-Cargo @('clippy', $Toolchain, '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings') }
}

$stages['unit-l1'] = @{
    Group = 'offline'
    Body  = { Invoke-Cargo @('test', $Toolchain, '--workspace', '--lib', '--all-features') }
}

$stages['l2-contract'] = @{
    Group = 'offline'
    Body  = { Invoke-Cargo @('test', $Toolchain, '--workspace', '--all-features', '--exclude', 'toasty-driver-dameng') }
}

$stages['doc-links'] = @{
    Group = 'offline'
    Body  = {
        Invoke-Cargo @('doc', $Toolchain, '--workspace', '--all-features', '--no-deps') `
            -Env @{ RUSTDOCFLAGS = '-D rustdoc::broken_intra_doc_links' }
    }
}

$stages['doctest'] = @{
    Group = 'offline'
    Body  = { Invoke-Cargo @('test', $Toolchain, '--workspace', '--doc', '--all-features') }
}

$stages['golden-dameng'] = @{
    Group = 'offline'
    Body  = {
        # fork 的 golden 必须带 `--all-features`：jiff / rust_decimal / bigdecimal 三条被 cfg 门控，
        # 默认特性下根本不编译（Round 26 的教训）。
        Invoke-Cargo @('test', $Toolchain, '-p', 'toasty-sql', '--test', 'serialize_dameng', '--all-features')
    }
}

$stages['toasty-offline'] = @{
    Group = 'offline'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'toasty-driver-dameng') }
}

# Round 41 起 `dpi` 是默认特性，`dm-odbc` 是回退后端 ⇒ 必须有一条**关掉 dpi** 的编译门禁，
# 否则回退路径会在没人察觉时烂掉（这正是 Round 41 发现 `DefaultBackend` 被写死的地方）。
$stages['offline-odbc-backend'] = @{
    Group = 'offline'
    Body  = {
        Invoke-Cargo @('check', $Toolchain, '-p', 'dm-query', '--no-default-features', '--features', 'chrono decimal', '--all-targets')
        Invoke-Cargo @('check', $Toolchain, '-p', 'dameng', '--no-default-features', '--all-targets')
    }
}

$stages['cross-aarch64'] = @{
    Group = 'cross'
    Body  = {
        if (-not (Test-TargetInstalled 'aarch64-unknown-linux-gnu')) { throw 'SKIP: 未安装 aarch64-unknown-linux-gnu' }
        Invoke-Cargo @('check', $Toolchain, '--workspace', '--all-targets', '--all-features', '--target', 'aarch64-unknown-linux-gnu')
    }
}

$stages['cross-x86_64'] = @{
    Group = 'cross'
    Body  = {
        if (-not (Test-TargetInstalled 'x86_64-unknown-linux-gnu')) { throw 'SKIP: 未安装 x86_64-unknown-linux-gnu' }
        Invoke-Cargo @('check', $Toolchain, '--workspace', '--all-targets', '--all-features', '--target', 'x86_64-unknown-linux-gnu')
    }
}

$stages['real-official-suite'] = @{
    Group = 'real'
    # 期望结果是**全绿**：Round 41 起默认后端是 `dm-dpi`，原先唯一失败的非 BMP 文本缺陷
    # （`type_primitives::ty_str`，见 docs/toasty-fork.md「真机实测发现」第 16 条）在 DPI 上已不复现。
    # 白名单只为「审计 ODBC 回退后端」保留：手工把本阶段的 cargo 参数加上
    # `--no-default-features`（= 走 `dm-odbc`）时，允许那 1 条已知缺陷被放行。
    # ⚠️ `--test-threads=1` 是必须的：本机达梦是开发版许可（`v$license` 的
    #   `AUTHORIZED_USER_NUMBER = 1`），并行测试进程会让后起的会话变只读并报
    #   `-6540 … only do readonly`（Round 41 实测：并行时出现 15 条假失败，串行 851/0）。
    Body  = {
        $code = Invoke-CargoRaw @('test', $Toolchain, '-p', 'toasty-driver-dameng', '--features', 'real', '--test', 'driver_suite', '--', '--test-threads=1')
        # 本阶段自己判定结果 ⇒ 清掉 cargo 的退出码，否则运行器的「阶段末尾退出码」兜底检查会再判红
        # （实测：白名单放行打印了 PASS，却仍以 101 收场）。
        $global:LASTEXITCODE = 0
        $out = $script:LastRawOut
        $text = (@($out) | Out-String)
        $m = [regex]::Matches($text, '(\d+) passed; (\d+) failed')
        if ($m.Count -eq 0) { throw "官方一致性套件没有输出 test result 行（退出码 $code）" }
        $last = $m[$m.Count - 1]
        $passed = [int]$last.Groups[1].Value
        $failed = [int]$last.Groups[2].Value
        # ⚠️ 这行正则不能带 `$` 锚：`Out-String` 在 Windows 上用 `\r\n` 分行，
        # 行尾那个 `\r` 会让 `FAILED$` 永远匹配不上（实测踩过：计数抓到了、失败名却是空）。
        $failedNames = @([regex]::Matches($text, '(?m)^test (\S+) \.\.\. FAILED') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)
        $known = @('type_primitives::ty_str')
        Write-Host "  官方套件：$passed passed / $failed failed（总 $($passed + $failed)）；失败：$(if ($failedNames.Count -gt 0) { $failedNames -join ', ' } else { '无' })"
        if ($StrictSuite) {
            if ($code -ne 0) { throw "官方一致性套件未全绿（-StrictSuite）：$passed passed / $failed failed" }
            return
        }
        if ($failed -eq 0 -and $code -eq 0) { return }
        if ($failed -eq 1 -and $failedNames.Count -eq 1 -and $known -contains $failedNames[0]) {
            Write-Host "  PASS（放行 1 条已知缺陷：$($failedNames[0]) —— 非 BMP 文本，见 docs/toasty-fork.md 第 16 条）" -ForegroundColor Yellow
            return
        }
        throw "官方一致性套件的失败集合与已知缺陷白名单不符：$passed passed / $failed failed（失败：$($failedNames -join ', ')）"
    }
}

$stages['real-toasty'] = @{
    Group = 'real'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'toasty-driver-dameng', '--features', 'real', '--test', 'toasty_real', '--', '--include-ignored', '--test-threads=1') }
}

# ⚠️ 下面四个真机阶段的 `--test-threads=1` 是**必须**的（README「真机」段同款口径）：
# 这些用例共用固定表名（`DM_Q_REAL_SEED`、迁移记录表等），并行时会互相 DROP/建表 ⇒
# 假失败（实测：不加时 `dm-query` 17 条红、`dm-migrate` 2 条红、`dm-pool` 会话数被别的用例抬高）。
$stages['real-l3-odbc'] = @{
    Group = 'real'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'dm-odbc', '--test', 'integration', '--', '--include-ignored', '--test-threads=1') }
}

$stages['real-dm-query'] = @{
    Group = 'real'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'dm-query', '--test', 'query_real', '--', '--include-ignored', '--test-threads=1') }
}

# Round 34 起新增的模式级功能测试（DBTESTA / DBTESTB / 小写模式），Round 36/37 扩到 16 条。
$stages['real-dm-schema'] = @{
    Group = 'real'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'dm-query', '--test', 'schema_real', '--', '--include-ignored', '--test-threads=1') }
}

# Round 41 起默认后端是 `dm-dpi`：这一阶段直接测驱动后端本身（8 条：连接/类型/NULL/事务/LOB/
# 数组绑定/错误码/execute_lob）。`dm-odbc` 的 74 条由上面的 `real-l3-odbc` 覆盖。
$stages['real-dpi-backend'] = @{
    Group = 'real'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'dm-dpi', '--test', 'backend_real', '--', '--include-ignored', '--test-threads=1') }
}

$stages['real-dm-migrate'] = @{
    Group = 'real'
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'dm-migrate', '--test', 'migrate_real', '--', '--include-ignored', '--test-threads=1') }
}

$stages['real-pool-soak'] = @{
    Group = 'real'
    Soak  = $true
    Body  = { Invoke-Cargo @('test', $Toolchain, '-p', 'dm-pool', '--test', 'g4_real', '--', '--include-ignored', '--test-threads=1', '--nocapture') }
}

$stages['real-example'] = @{
    Group = 'real'
    Body  = {
        Invoke-Tool -Exe 'cargo' -Args @('run', $Toolchain, '-p', 'toasty-driver-dameng', '--example', 'toasty_crud')
    }
}

if ($Coverage) {
    $stages['coverage-ac01'] = @{
        Group = 'coverage'
        Body  = {
            Invoke-Tool -Exe 'cargo' -Args @('llvm-cov', 'clean', '--workspace')
            Invoke-Tool -Exe 'cargo' -Args @('llvm-cov', '-p', 'dm-error', '-p', 'dm-core', '-p', 'dm-pool', '-p', 'dm-query', '--all-features')
        }
    }
}

# ---- 运行 ----
$results = @()
$index = 0
foreach ($name in $stages.Keys) {
    $stage = $stages[$name]
    $index++

    if ($Only.Count -gt 0 -and -not ($Only | Where-Object { $name -like "*$_*" })) { continue }
    if ($stage.Group -eq 'cross' -and $SkipCross) { continue }
    if ($stage.Group -eq 'real' -and ($SkipReal -or -not $hasDsn)) { continue }
    if ($stage.Soak -and $SkipSoak) { continue }

    $script:CurrentName = $name
    $script:CurrentLog = Join-Path $logDir ("{0:00}-{1}.log" -f $index, $name)
    Set-Content -Path $script:CurrentLog -Value "# cargo 阶段 $name（$Toolchain）`n" -Encoding UTF8
    Write-Head "[$index] $name"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $status = 'PASS'
    $note = ''
    try {
        & $stage.Body
        if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) { throw "阶段末尾退出码 $LASTEXITCODE" }
    } catch {
        $msg = $_.Exception.Message
        if ($msg -like 'SKIP:*') {
            $status = 'SKIP'
            $note = $msg.Substring(5).Trim()
            Write-Host "  SKIP: $note" -ForegroundColor Yellow
        } else {
            $status = 'FAIL'
            $note = $msg
            Write-Host "  FAIL: $msg" -ForegroundColor Red
        }
    }
    $sw.Stop()
    $elapsed = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    if ($status -eq 'PASS') { Write-Host "  PASS ($elapsed s) → $($script:CurrentLog)" -ForegroundColor Green }
    $results += [pscustomobject]@{ Stage = $name; Group = $stage.Group; Status = $status; Seconds = $elapsed; Log = $script:CurrentLog; Note = $note }
}

# ---- 汇总 ----
Write-Head '门禁汇总'
$results | Format-Table -AutoSize Stage, Group, Status, Seconds | Out-String -Width 200 | Write-Host
foreach ($r in $results) {
    if ($r.Status -eq 'FAIL') { Write-Host "FAIL  $($r.Stage): $($r.Note)" -ForegroundColor Red }
    if ($r.Status -eq 'SKIP') { Write-Host "SKIP  $($r.Stage): $($r.Note)" -ForegroundColor Yellow }
}

$failed = @($results | Where-Object { $_.Status -eq 'FAIL' })
$passed = @($results | Where-Object { $_.Status -eq 'PASS' })
Write-Host ''
Write-Host ("通过 {0} / 失败 {1} / 跳过 {2}" -f $passed.Count, $failed.Count, ($results.Count - $passed.Count - $failed.Count))
if ($failed.Count -gt 0) {
    Write-Host '门禁未通过。' -ForegroundColor Red
    exit 1
}
Write-Host '门禁全部通过。' -ForegroundColor Green
exit 0
