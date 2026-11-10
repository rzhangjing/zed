# check_deviations.ps1 - M13 Wave A porting-deviation & ledger audit tool
# ==========================================================================
# Cross-verifies the code anchors recorded in docs/deviations.md against the
# live source tree, and audits docs/deferred.md ledger self-consistency.
#
# Read-only: this tool NEVER edits any document or source file. It only scans
# and reports. It touches no Rust/WGSL/Cargo sources (it only stats line counts
# and searches for symbol presence in files referenced by the ledgers).
#
# What it checks
# --------------
# deviations.md (per DEV-NNN row):
#   D1 [error]   referenced file path does not exist (renamed/moved/deleted)
#   D2 [error]   "full-file" range end (1..N) exceeds the file's real line count
#                (file shrank below the claimed anchor -> anchor invalid)
#   D2 [warn]    file grew far beyond the claimed full-file end (stale anchor)
#   D3 [error]   a specific line reference points beyond EOF (line cannot exist)
#   D3 [warn]    an approximate (~) line reference points beyond EOF
#   D4 [warn]    a referenced backtick symbol is not found in any existing file
#   D5 [error]   duplicate DEV-NNN id in the main table
#   D6 [warn]    DEV-NNN id present in main table but missing from anchor index
#
#   Retired rows: a row whose text carries the retired marker (U+1F6D1) is a
#   HISTORICAL audit record kept after a deliberate scope cut -- its anchors
#   point at files that were intentionally deleted. Such rows are skipped by
#   D1-D4 (they carry a [retired] ok line instead) but still participate in
#   D5/D6, so id hygiene and the anchor index stay enforced. This is not an
#   escape hatch for broken anchors: the marker is only applied together with
#   a documented "已废止" outcome cell naming the cut that removed the code.
#
# deferred.md:
#   F1 [error]   duplicate numeric entry id (#N)
#   F2 [warn]    a status cell carries both resolved and pending markers
#   F3 [warn]    the statistics section counts disagree with the actual entries
#
# Usage
# -----
#   .\tools\check_deviations.ps1                 # human report, exit 1 on error
#   .\tools\check_deviations.ps1 -FailOnWarning  # promote warnings to exit 1
#   .\tools\check_deviations.ps1 -Json           # machine-readable report
#   .\tools\check_deviations.ps1 -RepoRoot D:\path\to\cesiumrust
#
# CI (soft gate): pwsh -NoLogo -NoProfile -File ./tools/check_deviations.ps1
#
# Compatibility
# -------------
#   - Windows PowerShell 5.1 AND pwsh 7 (ubuntu CI). No '&&', no ternary, no
#     null-coalescing operator. ASCII-only script body (non-ASCII markers from
#     the ledgers are matched via [char]0xNNNN / \uNNNN regex escapes so that a
#     BOM-less UTF-8 read on PS 5.1 cannot corrupt them).

param(
    [string]$RepoRoot = "",
    [switch]$FailOnWarning,
    [switch]$Json
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# -- Paths -------------------------------------------------------------------
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    # tools/ -> cesiumrust/
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
$DocsDir = Join-Path $RepoRoot "docs"
$DevPath = Join-Path $DocsDir "deviations.md"
$DefPath = Join-Path $DocsDir "deferred.md"

# Non-ASCII markers used by the ledgers, built from code points so the script
# body stays pure ASCII (robust across PS 5.1 / pwsh 7 encodings).
$MarkResolved = [char]0x2705   # white heavy check mark  -> resolved
$MarkPending  = [char]0x23F3   # hourglass with flowing sand -> pending
$MarkRetired  = [char]::ConvertFromUtf32(0x1F6D1)  # stop sign -> entry retired after a scope cut

# Regex building blocks (escaped, ASCII source).
$ReDash   = "[\u2013\u2014\-]"       # en-dash / em-dash / hyphen
$ReRange  = "(\d+)\s*$ReDash\s*(\d+)"
$ReByte   = "\d[\d,]*\s*B(?![A-Za-z])"   # "33501 B" byte-count guard
$ReSymbol = "^[A-Za-z_][A-Za-z0-9_]{3,}$"
$RePathExt = "\.(rs|wgsl|toml|js|glsl|json|md|txt|glb|gltf)$"

# -- Caches ------------------------------------------------------------------
$script:LineCountCache = @{}
$script:RawContentCache = @{}

function Get-ActualLineCount([string]$AbsPath) {
    if ($script:LineCountCache.ContainsKey($AbsPath)) { return $script:LineCountCache[$AbsPath] }
    $n = -1
    if (Test-Path -LiteralPath $AbsPath) {
        try {
            $c = Get-Content -LiteralPath $AbsPath -Encoding UTF8
            if ($null -eq $c) { $n = 0 } else { $n = @($c).Count }
        } catch { $n = -1 }
    }
    $script:LineCountCache[$AbsPath] = $n
    return $n
}

function Get-RawContent([string]$AbsPath) {
    if ($script:RawContentCache.ContainsKey($AbsPath)) { return $script:RawContentCache[$AbsPath] }
    $t = ""
    if (Test-Path -LiteralPath $AbsPath) {
        try { $t = Get-Content -LiteralPath $AbsPath -Raw -Encoding UTF8 } catch { $t = "" }
        if ($null -eq $t) { $t = "" }
    }
    $script:RawContentCache[$AbsPath] = $t
    return $t
}

# -- Findings ----------------------------------------------------------------
$script:Findings = New-Object System.Collections.ArrayList
$script:ErrCount = 0
$script:WarnCount = 0
$script:OkCount = 0
$script:RetiredCount = 0
$script:IsGHA = ($env:GITHUB_ACTIONS -eq "true")

function Add-Finding([string]$Severity, [string]$Check, [string]$File, [int]$Line, [string]$Msg) {
    $obj = [pscustomobject]@{
        severity = $Severity
        check    = $Check
        file     = $File
        line     = $Line
        message  = $Msg
    }
    [void]$script:Findings.Add($obj)
    if ($Severity -eq "error") { $script:ErrCount++ } else { $script:WarnCount++ }

    $relFile = $File
    if ($script:IsGHA) {
        $tag = "error"
        if ($Severity -eq "warning") { $tag = "warning" }
        Write-Host ("::{0} file={1},line={2}::[{3}] {4}" -f $tag, $relFile, $Line, $Check, $Msg)
    }
}

function Add-Ok([string]$Msg) {
    $script:OkCount++
    if (-not $Json) { Write-Host ("  [ok] {0}" -f $Msg) -ForegroundColor DarkGreen }
}

# -- Cell / number parsing helpers ------------------------------------------
function Get-BacktickTokens([string]$Text) {
    $out = New-Object System.Collections.ArrayList
    if ([string]::IsNullOrEmpty($Text)) { return ,$out }
    foreach ($m in [regex]::Matches($Text, '`([^`]*)`')) {
        [void]$out.Add($m.Groups[1].Value)
    }
    return ,$out
}

function Get-PathTokens([string]$Text) {
    $out = New-Object System.Collections.ArrayList
    foreach ($tok in (Get-BacktickTokens $Text)) {
        $t = ($tok -replace ':\d+$', '').Trim()
        $t = $t -replace '\\', '/'
        if ($t -match '/' -and $t -match $RePathExt) {
            [void]$out.Add($t)
        }
    }
    return ,$out
}

function Get-SymbolTokens([string]$Text) {
    $out = New-Object System.Collections.ArrayList
    foreach ($tok in (Get-BacktickTokens $Text)) {
        $t = $tok.Trim()
        if ($t -match $ReSymbol) { [void]$out.Add($t) }
    }
    return ,$out
}

# Strip backtick content and byte-count guards, then return all integers.
function Get-ClaimNumbers([string]$Text) {
    $out = New-Object System.Collections.ArrayList
    if ([string]::IsNullOrEmpty($Text)) { return ,$out }
    $s = [regex]::Replace($Text, '`([^`]*)`', " ")
    $s = [regex]::Replace($s, $ReByte, " ")
    foreach ($m in [regex]::Matches($s, "\d+")) {
        [void]$out.Add([int]$m.Value)
    }
    return ,$out
}

# Return list of @{Start;End;Approx} ranges found in text (backticks/bytes stripped).
function Get-ClaimRanges([string]$Text) {
    $out = New-Object System.Collections.ArrayList
    if ([string]::IsNullOrEmpty($Text)) { return ,$out }
    $s = [regex]::Replace($Text, '`([^`]*)`', " ")
    $s = [regex]::Replace($s, $ReByte, " ")
    foreach ($m in [regex]::Matches($s, $ReRange)) {
        $st = [int]$m.Groups[1].Value
        $en = [int]$m.Groups[2].Value
        $pre = ""
        if ($m.Index -gt 0) { $pre = $s.Substring([Math]::Max(0, $m.Index - 2), [Math]::Min(2, $m.Index)) }
        $approx = $pre.Contains("~")
        [void]$out.Add(@{ Start = $st; End = $en; Approx = $approx })
    }
    return ,$out
}

function Test-NumberIsPlausibleLine([int]$n) {
    # Ignore absurd values (byte counts that slipped through, ids, etc.)
    return ($n -ge 1 -and $n -le 1000000)
}

# -- Anchor line checks against a resolved file -----------------------------
# $Ranges: full-file ranges (Start==1) and specific ranges; $LooseNums: standalone numbers.
function Test-AnchorsAgainstFile([string]$RelPath, [int]$ActualLines, $Ranges, $LooseNums, [string]$DevId, [string]$SrcFile, [int]$SrcLine) {
    if ($ActualLines -lt 0) { return }   # file missing already reported by D1

    foreach ($r in $Ranges) {
        if ($r.Start -eq 1) {
            # Full-file range: the claimed end should approximate the real line count.
            # Small deltas in EITHER direction are tolerated (docs round / trailing-newline
            # differences). A dramatic shrink means the anchor is invalid (error); a
            # dramatic growth means it is stale but still resolvable (warning).
            $claimEnd = $r.End
            $tolerance = [Math]::Max(50, [int][Math]::Ceiling($claimEnd * 0.15))
            $delta = $ActualLines - $claimEnd
            if ([Math]::Abs($delta) -le $tolerance) {
                Add-Ok ("{0}: {1} full-file 1-{2} within tolerance (actual {3})" -f $DevId, $RelPath, $claimEnd, $ActualLines)
            } elseif ($delta -lt 0) {
                Add-Finding "error" "D2" $SrcFile $SrcLine ("{0}: full-file anchor 1-{1} far exceeds real length {2} of {3} (file shrank by {4}; anchor invalid)" -f $DevId, $claimEnd, $ActualLines, $RelPath, (-$delta))
            } else {
                Add-Finding "warning" "D2" $SrcFile $SrcLine ("{0}: {1} grew to {2} lines vs claimed full-file 1-{3} (+{4}); anchor stale" -f $DevId, $RelPath, $ActualLines, $claimEnd, $delta)
            }
        } else {
            # specific range: end must not exceed EOF
            $hi = $r.End
            if ($hi -gt $ActualLines) {
                if ($r.Approx) {
                    Add-Finding "warning" "D3" $SrcFile $SrcLine ("{0}: approximate range {1}-{2} exceeds EOF ({3} lines) of {4}" -f $DevId, $r.Start, $hi, $ActualLines, $RelPath)
                } else {
                    Add-Finding "error" "D3" $SrcFile $SrcLine ("{0}: range {1}-{2} exceeds EOF ({3} lines) of {4}; referenced line cannot exist" -f $DevId, $r.Start, $hi, $ActualLines, $RelPath)
                }
            } else {
                Add-Ok ("{0}: {1} range {2}-{3} within EOF ({4})" -f $DevId, $RelPath, $r.Start, $hi, $ActualLines)
            }
        }
    }

    foreach ($n in $LooseNums) {
        if (-not (Test-NumberIsPlausibleLine $n)) { continue }
        if ($n -gt $ActualLines) {
            Add-Finding "error" "D3" $SrcFile $SrcLine ("{0}: line reference {1} exceeds EOF ({2} lines) of {3}" -f $DevId, $n, $ActualLines, $RelPath)
        }
    }
}

# ==========================================================================
# deviations.md audit
# ==========================================================================
function Invoke-DeviationsAudit {
    if (-not $Json) {
        Write-Host ""
        Write-Host "== deviations.md: code-anchor audit ==" -ForegroundColor Cyan
    }
    if (-not (Test-Path -LiteralPath $DevPath)) {
        Add-Finding "error" "D0" "docs/deviations.md" 0 "deviations.md not found"
        return
    }

    $lines = @(Get-Content -LiteralPath $DevPath -Encoding UTF8)
    $seenIds = @{}
    $mainIds = New-Object System.Collections.ArrayList

    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        $lineNo = $i + 1
        if ($line -notmatch '^\|\s*(DEV-\d+)\s*\|') { continue }

        $id = $Matches[1]

        # D5 duplicate id
        if ($seenIds.ContainsKey($id)) {
            Add-Finding "error" "D5" "docs/deviations.md" $lineNo ("duplicate id {0} (first seen at line {1})" -f $id, $seenIds[$id])
        } else {
            $seenIds[$id] = $lineNo
        }
        [void]$mainIds.Add($id)

        # Split row into cells (drop leading/trailing empty from the outer pipes)
        # Split on UNESCAPED pipes only: markdown cells may contain literal '\|'
        # (e.g. a shell grep pattern), which must not be treated as a separator.
        $parts = [regex]::Split($line, '(?<!\\)\|')
        $cells = New-Object System.Collections.ArrayList
        foreach ($p in $parts) { [void]$cells.Add($p.Trim()) }
        while ($cells.Count -gt 0 -and $cells[0] -eq "") { $cells.RemoveAt(0) }
        while ($cells.Count -gt 0 -and $cells[$cells.Count - 1] -eq "") { $cells.RemoveAt($cells.Count - 1) }

        if ($cells.Count -lt 2) {
            Add-Finding "warning" "D0" "docs/deviations.md" $lineNo ("{0}: malformed row (only {1} cells)" -f $id, $cells.Count)
            continue
        }

        # Retired rows (scope-cut history) are not anchor-checked: their files were
        # deliberately deleted. D5/D6 still apply (id hygiene + anchor index).
        if ($line.IndexOf($MarkRetired) -ge 0) {
            $script:RetiredCount++
            Add-Ok ("{0}: retired entry - anchors intentionally not re-verified (historical record)" -f $id)
            continue
        }

        $fileCell = [string]$cells[1]
        $lineCell = ""
        if ($cells.Count -ge 5) { $lineCell = [string]$cells[2] }   # 5-col variant has a Line(s) cell

        # Collect all referenced paths + symbols from the whole row (file + line cells)
        $rowText = $fileCell + " " + $lineCell
        $paths = Get-PathTokens $rowText
        $symbols = Get-SymbolTokens $rowText

        if ($paths.Count -eq 0) {
            Add-Finding "warning" "D1" "docs/deviations.md" $lineNo ("{0}: no resolvable file path in File/Line cells (anchor not machine-checkable)" -f $id)
        }

        # Resolve paths, report missing (D1), gather existing absolute paths
        $existing = New-Object System.Collections.ArrayList
        foreach ($rel in $paths) {
            $abs = Join-Path $RepoRoot ($rel -replace '/', [System.IO.Path]::DirectorySeparatorChar)
            if (Test-Path -LiteralPath $abs) {
                [void]$existing.Add(@{ Rel = $rel; Abs = $abs; Lines = (Get-ActualLineCount $abs) })
            } else {
                Add-Finding "error" "D1" "docs/deviations.md" $lineNo ("{0}: referenced file does not exist: {1}" -f $id, $rel)
            }
        }

        # Per-segment line-claim checks on the File cell (handles 4-col inline ranges)
        foreach ($seg in ($fileCell -split ';')) {
            $segPaths = Get-PathTokens $seg
            if ($segPaths.Count -ne 1) { continue }
            $rel = $segPaths[0]
            $abs = Join-Path $RepoRoot ($rel -replace '/', [System.IO.Path]::DirectorySeparatorChar)
            if (-not (Test-Path -LiteralPath $abs)) { continue }
            $actual = Get-ActualLineCount $abs
            $ranges = Get-ClaimRanges $seg
            $nums = Get-ClaimNumbers $seg
            # Remove numbers already covered by a range to avoid double-counting
            $rangeNums = @{}
            foreach ($r in $ranges) { $rangeNums[$r.Start] = $true; $rangeNums[$r.End] = $true }
            $loose = New-Object System.Collections.ArrayList
            foreach ($n in $nums) { if (-not $rangeNums.ContainsKey($n)) { [void]$loose.Add($n) } }
            if ($ranges.Count -gt 0 -or $loose.Count -gt 0) {
                Test-AnchorsAgainstFile $rel $actual $ranges $loose $id "docs/deviations.md" $lineNo
            }
        }

        # Line-cell numbers (5-col variant): check against the single referenced file
        if ($lineCell -ne "") {
            $ranges = Get-ClaimRanges $lineCell
            $nums = Get-ClaimNumbers $lineCell
            $rangeNums = @{}
            foreach ($r in $ranges) { $rangeNums[$r.Start] = $true; $rangeNums[$r.End] = $true }
            $loose = New-Object System.Collections.ArrayList
            foreach ($n in $nums) { if (-not $rangeNums.ContainsKey($n)) { [void]$loose.Add($n) } }

            if (($ranges.Count -gt 0 -or $loose.Count -gt 0) -and $existing.Count -ge 1) {
                if ($existing.Count -eq 1) {
                    $e = $existing[0]
                    Test-AnchorsAgainstFile $e.Rel $e.Lines $ranges $loose $id "docs/deviations.md" $lineNo
                } else {
                    # multiple files: use the max real length as a conservative bound
                    $maxLines = 0
                    $label = ""
                    foreach ($e in $existing) { if ($e.Lines -gt $maxLines) { $maxLines = $e.Lines; $label = $e.Rel } }
                    Test-AnchorsAgainstFile $label $maxLines $ranges $loose $id "docs/deviations.md" $lineNo
                }
            }
        }

        # D4 symbol presence across existing files for this row
        foreach ($sym in $symbols) {
            $found = $false
            foreach ($e in $existing) {
                $raw = Get-RawContent $e.Abs
                if ($raw.Contains($sym)) { $found = $true; break }
            }
            if (-not $found -and $existing.Count -gt 0) {
                Add-Finding "warning" "D4" "docs/deviations.md" $lineNo ("{0}: symbol '{1}' not found in any referenced file (renamed/removed?)" -f $id, $sym)
            }
        }
    }

    # D6: main-table ids present in the anchor index (#dev-nnn)
    $rawAll = Get-RawContent $DevPath
    foreach ($id in $mainIds) {
        $anchor = "#" + $id.ToLower()
        if (-not $rawAll.Contains($anchor)) {
            Add-Finding "warning" "D6" "docs/deviations.md" 0 ("{0} present in main table but missing from anchor index ({1})" -f $id, $anchor)
        }
    }

    if (-not $Json) {
        Write-Host ("  scanned {0} DEV entries ({1} retired)" -f $mainIds.Count, $script:RetiredCount) -ForegroundColor Gray
    }
}

# ==========================================================================
# deferred.md audit
# ==========================================================================
function Invoke-DeferredAudit {
    if (-not $Json) {
        Write-Host ""
        Write-Host "== deferred.md: ledger consistency audit ==" -ForegroundColor Cyan
    }
    if (-not (Test-Path -LiteralPath $DefPath)) {
        Add-Finding "error" "F0" "docs/deferred.md" 0 "deferred.md not found"
        return
    }

    $lines = @(Get-Content -LiteralPath $DefPath -Encoding UTF8)
    $seenNums = @{}
    $actualTotal = 0
    $actualResolved = 0
    $actualPending = 0

    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        $lineNo = $i + 1
        # Only numbered data rows: | <int> | ...
        if ($line -notmatch '^\|\s*(\d+)\s*\|') { continue }
        $num = [int]$Matches[1]

        # Split on UNESCAPED pipes only: markdown cells may contain literal '\|'
        # (e.g. a shell grep pattern), which must not be treated as a separator.
        $parts = [regex]::Split($line, '(?<!\\)\|')
        $cells = New-Object System.Collections.ArrayList
        foreach ($p in $parts) { [void]$cells.Add($p.Trim()) }
        while ($cells.Count -gt 0 -and $cells[0] -eq "") { $cells.RemoveAt(0) }
        while ($cells.Count -gt 0 -and $cells[$cells.Count - 1] -eq "") { $cells.RemoveAt($cells.Count - 1) }

        # Status is the last cell of a data row (6-col tables)
        $status = ""
        if ($cells.Count -ge 2) { $status = [string]$cells[$cells.Count - 1] }

        $actualTotal++
        if ($seenNums.ContainsKey($num)) {
            Add-Finding "error" "F1" "docs/deferred.md" $lineNo ("duplicate entry #{0} (first seen at line {1})" -f $num, $seenNums[$num])
        } else {
            $seenNums[$num] = $lineNo
        }

        $hasRes = $status.Contains($MarkResolved)
        $hasPen = $status.Contains($MarkPending)
        if ($hasRes -and $hasPen) {
            Add-Finding "warning" "F2" "docs/deferred.md" $lineNo ("entry #{0} status carries both resolved and pending markers" -f $num)
        }
        if ($hasRes) { $actualResolved++ }
        elseif ($hasPen) { $actualPending++ }
    }

    # Statistics section: parse declared counts from "- **...**: <N> ..." lines
    $declResolved = -1; $declPending = -1; $declTotal = -1
    $script:DeclResLine = 0; $script:DeclPenLine = 0; $script:DeclTotLine = 0
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        $lineNo = $i + 1
        if ($line -notmatch '^-\s+\*\*(.+?)\*\*\s*:\s*(\d+)') { continue }
        $label = $Matches[1]
        $val = [int]$Matches[2]
        if ($label.Contains($MarkResolved)) { $declResolved = $val; $script:DeclResLine = $lineNo }
        elseif ($label.Contains($MarkPending)) { $declPending = $val; $script:DeclPenLine = $lineNo }
        elseif ($label -match '\u603b\u6761\u76ee' -or $label.ToLower().Contains("total")) { $declTotal = $val; $script:DeclTotLine = $lineNo }
    }

    if ($declResolved -ge 0 -and $declResolved -ne $actualResolved) {
        Add-Finding "warning" "F3" "docs/deferred.md" $script:DeclResLine ("statistics declares {0} resolved but found {1}" -f $declResolved, $actualResolved)
    }
    if ($declPending -ge 0 -and $declPending -ne $actualPending) {
        Add-Finding "warning" "F3" "docs/deferred.md" $script:DeclPenLine ("statistics declares {0} pending but found {1}" -f $declPending, $actualPending)
    }
    if ($declTotal -ge 0 -and $declTotal -ne $actualTotal) {
        Add-Finding "warning" "F3" "docs/deferred.md" $script:DeclTotLine ("statistics declares {0} total entries but found {1}" -f $declTotal, $actualTotal)
    }
    if ($declResolved -ge 0 -and $declPending -ge 0 -and $declTotal -ge 0) {
        if (($declResolved + $declPending) -ne $declTotal) {
            Add-Finding "warning" "F3" "docs/deferred.md" $script:DeclTotLine ("declared resolved({0}) + pending({1}) != declared total({2})" -f $declResolved, $declPending, $declTotal)
        }
    }

    if (-not $Json) {
        Write-Host ("  entries: total={0} resolved={1} pending={2}" -f $actualTotal, $actualResolved, $actualPending) -ForegroundColor Gray
        if ($declTotal -ge 0) {
            Write-Host ("  declared: total={0} resolved={1} pending={2}" -f $declTotal, $declResolved, $declPending) -ForegroundColor Gray
        }
    }
    if ($actualTotal -gt 0 -and $declTotal -eq $actualTotal -and $declResolved -eq $actualResolved -and $declPending -eq $actualPending) {
        Add-Ok ("deferred.md statistics consistent (total={0})" -f $actualTotal)
    }
}

# ==========================================================================
# Main
# ==========================================================================
if (-not $Json) {
    Write-Host "check_deviations.ps1 - porting-deviation & ledger audit" -ForegroundColor Cyan
    Write-Host ("repo root: {0}" -f $RepoRoot) -ForegroundColor Gray
}

Invoke-DeviationsAudit
Invoke-DeferredAudit

# -- Report ------------------------------------------------------------------
if ($Json) {
    $report = [pscustomobject]@{
        repo_root = ($RepoRoot -replace '\\', '/')
        errors    = $script:ErrCount
        warnings  = $script:WarnCount
        ok        = $script:OkCount
        retired   = $script:RetiredCount
        fail_on_warning = [bool]$FailOnWarning
        findings  = $script:Findings
    }
    $report | ConvertTo-Json -Depth 5
} else {
    Write-Host ""
    if ($script:Findings.Count -gt 0) {
        Write-Host "== findings ==" -ForegroundColor Cyan
        foreach ($f in $script:Findings) {
            $color = "Red"
            if ($f.severity -eq "warning") { $color = "Yellow" }
            $loc = "{0}:{1}" -f $f.file, $f.line
            Write-Host ("  [{0}] {1} {2} - {3}" -f $f.severity.ToUpper(), $f.check, $loc, $f.message) -ForegroundColor $color
        }
    }
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host " check_deviations summary" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ("  errors  : {0}" -f $script:ErrCount) -ForegroundColor ($(if ($script:ErrCount -gt 0) { "Red" } else { "Green" }))
    Write-Host ("  warnings: {0}" -f $script:WarnCount) -ForegroundColor ($(if ($script:WarnCount -gt 0) { "Yellow" } else { "Green" }))
    Write-Host ("  ok      : {0}" -f $script:OkCount) -ForegroundColor Green
    Write-Host ("  retired : {0} (scope-cut history, anchors not re-verified)" -f $script:RetiredCount) -ForegroundColor Gray
}

$exit = 0
if ($script:ErrCount -gt 0) { $exit = 1 }
if ($FailOnWarning -and $script:WarnCount -gt 0) { $exit = 1 }
exit $exit
