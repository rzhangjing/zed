# dm-dialect-probe.ps1 -- DM (DaMeng) DPI dialect acceptance probe
# =============================================================================
# Purpose
#   Call the local DM DPI library D:\dmdbms\bin\dmdpi.dll directly through
#   Add-Type P/Invoke, and run a fixed list of MySQL / Oracle / SQL Server
#   flavoured SQL statements against the RUNNING DM instance
#   (localhost:5236, SYSDBA/SYSDBA) to decide whether the DM SERVER accepts
#   those dialect constructs natively.
#
# Judgement discipline
#   * query : PASS only if rc == 0 AND number_columns > 0
#   * dml   : PASS only if rc == 0 AND row_count is available
#   * ddl   : PASS only if rc == 0 (creates are cross-checked via user_tables)
#   * failures record err_code + raw diagnostic text (mojibake kept verbatim)
#   * the original SQL text is NEVER rewritten to make it pass; any variant
#     probe is reported as a separate "ext" row
#
# Signatures come from D:\dmdbms\include\DPI.h + DPItypes.h (header wins):
#   DPIRETURN            = sdint2  = 16-bit
#   DM64 build           => slength/ulength = __int64, so the length args of
#                           dpi_fetch / dpi_row_count / dpi_get_data are Int64
#   dpi_number_columns(dhstmt, sdint2* col_cnt)   <-- column count is an OUT
#                           parameter, it is NOT the return value
#
# Self-check: probes both readings of dpi_number_columns and reports which one
# works, so the harness never silently mis-detects "not a result set".
#
# RUN WITH (Windows PowerShell 5.1 refuses unsigned scripts by default):
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -File <this file>
#
# NOTE: this file is deliberately 100% ASCII. Windows PowerShell 5.1 decodes a
# BOM-less .ps1 as ANSI/GBK, which corrupts non-ASCII source. Chinese prose for
# this probe lives in docs/dm-dialect-probe.md, not here.
#
# The script is re-runnable and leaves NO objects behind in the database.
# =============================================================================
[CmdletBinding()]
param(
    [string]$DpiPath  = 'D:\dmdbms\bin\dmdpi.dll',
    [string]$Server   = 'localhost:5236',
    [string]$User     = 'SYSDBA',
    [string]$Password = 'SYSDBA',
    [string]$OutJson  = 'D:\Rust\dameng\bench\dm-dialect-probe.result.json',
    [string]$OutTable = 'D:\Rust\dameng\bench\dm-dialect-probe.results.md'
)

$ErrorActionPreference = 'Stop'
$OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$env:PATH = (Split-Path -Parent $DpiPath) + ';' + $env:PATH

# --------------------------------------------------------------------------
# 1. P/Invoke bindings
# --------------------------------------------------------------------------
$csTemplate = @'
using System;
using System.Runtime.InteropServices;

public static class DmDpi
{
    public const short H_ENV  = 1;
    public const short H_CON  = 2;
    public const short H_STMT = 3;

    public const short C_CHAR  = 1003;   // DSQL_C_CHAR
    public const short C_SLONG = 3;      // DSQL_C_SLONG

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_module_init")]
    public static extern short dpi_module_init();

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_alloc_env")]
    public static extern short dpi_alloc_env(out IntPtr env);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_alloc_con")]
    public static extern short dpi_alloc_con(IntPtr env, out IntPtr con);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_login")]
    public static extern short dpi_login(IntPtr con, byte[] svr, byte[] user, byte[] pwd);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_logout")]
    public static extern short dpi_logout(IntPtr con);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_alloc_stmt")]
    public static extern short dpi_alloc_stmt(IntPtr con, out IntPtr stmt);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_exec_direct")]
    public static extern short dpi_exec_direct(IntPtr stmt, byte[] sql);

    // header-faithful: column count is the OUT parameter sdint2*
    // NOTE: do NOT add a second DllImport for the same EntryPoint name with a
    // different signature. The CLR (at least .NET Framework / PS 5.1) caches the
    // P/Invoke stub per entry point, so the second managed signature silently
    // reuses the first native stub and throws NullReferenceException.
    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_number_columns")]
    public static extern short dpi_number_columns(IntPtr stmt, out short col_cnt);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_fetch")]
    public static extern short dpi_fetch(IntPtr stmt, out long rows);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_get_data")]
    public static extern short dpi_get_data(IntPtr stmt, ushort icol, short ctype, byte[] val, long buf_len, out long val_len);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_row_count")]
    public static extern short dpi_row_count(IntPtr stmt, out long n);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_close_cursor")]
    public static extern short dpi_close_cursor(IntPtr stmt);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_get_diag_rec")]
    public static extern short dpi_get_diag_rec(short hndl_type, IntPtr hndl, short rec_num, out int err_code, byte[] err_msg, short buf_sz, out short msg_len);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_free_stmt")]
    public static extern short dpi_free_stmt(IntPtr stmt);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_free_con")]
    public static extern short dpi_free_con(IntPtr con);

    [DllImport(@"__DPI_PATH__", CallingConvention = CallingConvention.Cdecl, EntryPoint = "dpi_free_env")]
    public static extern short dpi_free_env(IntPtr env);
}
'@

$cs = $csTemplate.Replace('__DPI_PATH__', $DpiPath)
Add-Type -TypeDefinition $cs -Language CSharp

# --------------------------------------------------------------------------
# 2. Helpers
# --------------------------------------------------------------------------
$GBK = [System.Text.Encoding]::GetEncoding(936)

function AsciiZ([string]$s) { [byte[]]([System.Text.Encoding]::ASCII.GetBytes($s + [char]0)) }

function Decode-Bytes([byte[]]$b, [int]$len) {
    if ($len -le 0) { return '' }
    try   { return $GBK.GetString($b, 0, $len) }
    catch { return [System.Text.Encoding]::ASCII.GetString($b, 0, $len) }
}

# read diagnostic records (hndl_type: ENV=1 CON=2 STMT=3)
function Get-Diag([Int16]$hType, [IntPtr]$h) {
    $recs = New-Object System.Collections.Generic.List[object]
    $code = $null; $msg = $null
    for ($rec = 1; $rec -le 6; $rec++) {
        $ec  = 0
        $len = [Int16]0
        $buf = New-Object byte[] 2048
        $rc  = [DmDpi]::dpi_get_diag_rec($hType, $h, [Int16]$rec, [ref]$ec, $buf, [Int16]2048, [ref]$len)
        if ($rc -ne 0) { break }
        if ($len -eq 0 -and $ec -eq 0) { break }
        $text = Decode-Bytes $buf $len
        $recs.Add([pscustomobject]@{ rec = $rec; code = $ec; msg = $text })
        if ($null -eq $code) { $code = $ec; $msg = $text }
    }
    [pscustomobject]@{ Code = $code; Msg = $msg; Recs = $recs }
}

function Close-Cursor-Safe([IntPtr]$h) { try { [void][DmDpi]::dpi_close_cursor($h) } catch { } }

# read one result column as text (CHAR conversion)
function Read-Cell([IntPtr]$h, [int]$col) {
    $buf = New-Object byte[] 4096
    [long]$vlen = 0
    $rc = [DmDpi]::dpi_get_data($h, [UInt16]$col, [Int16]1003, $buf, [long]4096, [ref]$vlen)
    if ($rc -ne 0) { return "<get_data rc=$rc>" }
    $t = Decode-Bytes $buf ([int]$vlen)
    if ($null -eq $t) { return '' }
    return $t.Trim()
}

function Write-Text-File([string]$path, [string]$text) {
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
}

# --------------------------------------------------------------------------
# 3. Connect
# --------------------------------------------------------------------------
$gEnv = [IntPtr]::Zero; $gCon = [IntPtr]::Zero; $gStmt = [IntPtr]::Zero

$rc = [DmDpi]::dpi_module_init()
if ($rc -ne 0) { throw "dpi_module_init failed rc=$rc" }

$rc = [DmDpi]::dpi_alloc_env([ref]$gEnv)
if ($rc -ne 0) { throw "dpi_alloc_env failed rc=$rc" }

$rc = [DmDpi]::dpi_alloc_con($gEnv, [ref]$gCon)
if ($rc -ne 0) { throw "dpi_alloc_con failed rc=$rc" }

$rc = [DmDpi]::dpi_login($gCon, (AsciiZ $Server), (AsciiZ $User), (AsciiZ $Password))
if ($rc -ne 0) {
    $d = Get-Diag ([Int16]2) $gCon
    throw "dpi_login failed rc=$rc err_code=$($d.Code) msg=$($d.Msg)"
}

$rc = [DmDpi]::dpi_alloc_stmt($gCon, [ref]$gStmt)
if ($rc -ne 0) { throw "dpi_alloc_stmt failed rc=$rc" }

Write-Host "[ok] connected to $Server as $User via $DpiPath" -ForegroundColor Green

# Self-check the column-count binding against selects of KNOWN arity. If the
# counts do not match exactly, the harness is mis-detecting result sets, so we
# abort instead of producing false PASS/FAIL verdicts.
$selfCheck = New-Object System.Collections.Generic.List[object]
$modeOk = $true
foreach ($case in @(@('select 1 from dual', 1), @('select 1 as a, 2 as b, 3 as c from dual', 3))) {
    $q = $case[0]; $want = $case[1]
    Close-Cursor-Safe $gStmt
    $erc = [DmDpi]::dpi_exec_direct($gStmt, (AsciiZ $q))
    [Int16]$got = -1
    $nrc = [DmDpi]::dpi_number_columns($gStmt, [ref]$got)
    Close-Cursor-Safe $gStmt
    $ok = ($erc -eq 0 -and $nrc -eq 0 -and [int]$got -eq $want)
    if (-not $ok) { $modeOk = $false }
    $selfCheck.Add((New-Object psobject -Property @{
        sql     = $q
        want    = $want
        exec_rc = [int]$erc
        nc_rc   = [int]$nrc
        got     = [int]$got
        ok      = $ok
    }))
    Write-Host ("[self-check] '{0}' -> want ncols={1}, exec rc={2}, dpi_number_columns rc={3}, ncols={4} => {5}" -f $q, $want, $erc, $nrc, $got, $(if ($ok) { 'OK' } else { 'MISMATCH' }))
}

if (-not $modeOk) {
    throw "dpi_number_columns self-check FAILED; column counts are unreliable, refusing to emit verdicts"
}
$NumberColumnsMode = 'out-param (sdint2*, header-faithful, self-checked)'
Write-Host "[self-check] column-count binding verified: $NumberColumnsMode"

# --------------------------------------------------------------------------
# 4. Execute + judge
# --------------------------------------------------------------------------
$script:Results = New-Object System.Collections.Generic.List[object]
$script:Index   = 0

function Invoke-Probe {
    param(
        [Parameter(Mandatory)][string]$Group,
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][string]$Sql,
        [ValidateSet('query', 'dml', 'ddl')][string]$Kind = 'query',
        [int]$MaxRows = 5,
        [switch]$Ext
    )

    $script:Index++
    $idx = if ($Ext) { "e$($script:Index)" } else { "$($script:Index)" }

    # pitfall 1: a stmt handle must have its cursor closed before re-executing
    Close-Cursor-Safe $gStmt

    $rc   = [DmDpi]::dpi_exec_direct($gStmt, (AsciiZ $Sql))
    $diag = Get-Diag ([Int16]3) $gStmt

    $ncols    = 0
    $rows     = 0
    $rowCount = $null
    $sample   = ''
    $note     = ''

    if ($rc -eq 0) {
        # header-faithful binding, verified by the self-check before the run
        [Int16]$nc = 0
        $nrc = [DmDpi]::dpi_number_columns($gStmt, [ref]$nc)
        if ($nrc -eq 0) { $ncols = [int]$nc }

        if ($ncols -gt 0) {
            # pitfall 2: only a result set may be fetched
            $rowTexts = New-Object System.Collections.Generic.List[string]
            for ($f = 0; $f -lt $MaxRows; $f++) {
                [long]$rn = 0
                $frc = [DmDpi]::dpi_fetch($gStmt, [ref]$rn)
                if ($frc -gt 0) { break }
                $rows++
                $maxc = [Math]::Min($ncols, 8)
                $cells = New-Object System.Collections.Generic.List[string]
                for ($c = 1; $c -le $maxc; $c++) { $cells.Add((Read-Cell $gStmt $c)) }
                $rowTexts.Add(($cells -join ' | '))
            }
            # all fetched rows are kept: a single-row sample hides multi-row
            # results (e.g. whether COMPATIBLE_MODE exists in v$dm_ini at all)
            $sample = ($rowTexts -join ' ;; ')
            if ($rows -ge $MaxRows) { $note += "row cap $MaxRows hit; " }
        } else {
            [long]$rcn = 0
            $rrc = [DmDpi]::dpi_row_count($gStmt, [ref]$rcn)
            if ($rrc -eq 0) { $rowCount = $rcn } else { $note = "row_count rc=$rrc;" }
        }
    }

    Close-Cursor-Safe $gStmt

    $verdict = switch ($Kind) {
        'query' {
            if ($rc -ne 0) { 'FAIL' }
            elseif ($ncols -le 0) { 'ODD' }   # rc==0 but no column: NOT a pass
            else { 'PASS' }
        }
        default {
            if ($rc -ne 0) { 'FAIL' }
            elseif ($Kind -eq 'dml' -and $null -eq $rowCount) { 'ODD' }
            else { 'PASS' }
        }
    }

    $o = New-Object psobject -Property @{
        idx      = $idx
        group    = $Group
        label    = $Label
        sql      = $Sql
        kind     = $Kind
        rc       = [int]$rc
        err_code = $diag.Code
        diag     = $diag.Msg
        ncols    = $ncols
        rows     = $rows
        rowcount = $rowCount
        sample   = $sample
        verdict  = $verdict
        note     = $note.Trim()
        ext      = [bool]$Ext
    }
    $script:Results.Add($o)

    $color = switch ($verdict) { 'PASS' { 'Green' } 'ODD' { 'Yellow' } default { 'Red' } }
    $msgPart = if ($null -ne $diag.Code) { " [$($diag.Code)] $($diag.Msg)" } else { '' }
    $valPart = if ($sample) { " => $sample" } elseif ($null -ne $rowCount) { " => rowcount=$rowCount" } else { '' }
    Write-Host ("{0,3} {1,-4} rc={2,-3}{3}{4} :: {5}" -f $idx, $verdict, $rc, $msgPart, $valPart, $Sql) -ForegroundColor $color
}

# --------------------------------------------------------------------------
# 5. Probe list (original SQL verbatim, in the requested order)
# --------------------------------------------------------------------------
Write-Host ""
Write-Host "=== A. identifier quoting ===" -ForegroundColor Cyan
Invoke-Probe -Group 'A' -Label 'baseline: constant + dual'  -Sql "select 1 as a from dual"                      -Kind query
Invoke-Probe -Group 'A' -Label 'MySQL backtick'             -Sql "select ``a`` from (select 1 as a from dual) t" -Kind query
Invoke-Probe -Group 'A' -Label 'ANSI double quote'          -Sql 'select "a" from (select 1 as a from dual) t'   -Kind query
Invoke-Probe -Group 'A' -Label 'SQL Server brackets'        -Sql 'select [a] from (select 1 as a from dual) t'   -Kind query
# A-ext: probe 2/3 above failed at DIFFERENT levels. Backtick/bracket are pure
# SYNTAX errors, while double quote parsed fine and failed only on the NAME
# ("invalid column name [a]"), i.e. double quotes ARE identifier delimiters and
# the name is case-exact. These variants separate the two mechanisms.
Invoke-Probe -Group 'A-ext' -Label 'double quote, exact case' -Ext -Kind query -Sql 'select "A" from (select 1 as a from dual) t'
Invoke-Probe -Group 'A-ext' -Label 'double-quoted alias'      -Ext -Kind query -Sql 'select 1 as "lc" from dual'
Invoke-Probe -Group 'A-ext' -Label 'double-quoted round trip' -Ext -Kind query -Sql 'select "lc" from (select 1 as "lc" from dual) t'
# NOTE: single-quoted here on purpose. In a single-quoted PS string a backtick is
# literal, so this sends exactly ONE backtick pair. (In a double-quoted string a
# backtick is the escape character and would be swallowed.)
Invoke-Probe -Group 'A-ext' -Label 'backtick on simple alias' -Ext -Kind query -Sql 'select 1 as `a` from dual'

Write-Host ""
Write-Host "=== B. pagination ===" -ForegroundColor Cyan
Invoke-Probe -Group 'B' -Label 'LIMIT n'                   -Sql 'select 1 from dual limit 1'          -Kind query
Invoke-Probe -Group 'B' -Label 'LIMIT off,cnt (MySQL)'     -Sql 'select 1 from dual limit 0, 1'       -Kind query
Invoke-Probe -Group 'B' -Label 'LIMIT n OFFSET m'          -Sql 'select 1 from dual limit 1 offset 0' -Kind query
Invoke-Probe -Group 'B' -Label 'TOP n (SQL Server)'        -Sql 'select top 1 1 from dual'            -Kind query
Invoke-Probe -Group 'B' -Label 'ROWNUM (Oracle)'           -Sql 'select 1 from dual where rownum = 1' -Kind query
# B-ext: parsing LIMIT is not the same as honouring MySQL offset semantics.
# On a 3-row set, "limit 1, 1" must yield the SECOND row if off,cnt is real.
$pageQ = 'select x from (select 1 as x from dual union all select 2 as x from dual union all select 3 as x from dual) t'
Invoke-Probe -Group 'B-ext' -Label 'LIMIT 1,1 must return row 2'        -Ext -Kind query -Sql "$pageQ limit 1, 1"
Invoke-Probe -Group 'B-ext' -Label 'LIMIT 1 OFFSET 1 must return row 2' -Ext -Kind query -Sql "$pageQ limit 1 offset 1"

Write-Host ""
Write-Host "=== pre-cleanup ===" -ForegroundColor DarkGray
foreach ($t in @('T_MPROBE_A', 'T_MPROBE_B', 'T_MPROBE_C', 'T_MPROBE_D', 'T_MPROBE_U')) {
    Close-Cursor-Safe $gStmt
    [void][DmDpi]::dpi_exec_direct($gStmt, (AsciiZ "drop table $t"))
    Close-Cursor-Safe $gStmt
}

Write-Host ""
Write-Host "=== C. auto-increment primary key ===" -ForegroundColor Cyan
Invoke-Probe -Group 'C' -Label 'AUTO_INCREMENT (MySQL)'            -Sql 'create table t_mprobe_a(id int auto_increment primary key, v varchar(20))'                   -Kind ddl
Invoke-Probe -Group 'C' -Label 'IDENTITY(1,1) (SQL Server)'        -Sql 'create table t_mprobe_b(id int identity(1,1) primary key, v varchar(20))'                    -Kind ddl
Invoke-Probe -Group 'C' -Label 'GENERATED BY DEFAULT AS IDENTITY'  -Sql 'create table t_mprobe_c(id int generated by default as identity primary key, v varchar(20))' -Kind ddl
Invoke-Probe -Group 'C' -Label 'GENERATED ALWAYS AS IDENTITY'      -Sql 'create table t_mprobe_d(id int generated always as identity, v varchar(20))'                 -Kind ddl

# after each create: cross-check via user_tables, then drop immediately
foreach ($t in @('t_mprobe_a', 't_mprobe_b', 't_mprobe_c', 't_mprobe_d')) {
    $crt = $script:Results | Where-Object { $_.sql -eq "create table $t" -or $_.sql -like "create table $t(*" } | Select-Object -First 1
    if ($null -ne $crt -and $crt.rc -eq 0) {
        Invoke-Probe -Group 'C-ext' -Label "verify $t exists" -Ext -Kind query `
            -Sql "select count(*) as n from user_tables where table_name = '$($t.ToUpper())'"
    }
    Invoke-Probe -Group 'C-ext' -Label "drop $t right away" -Ext -Kind ddl -Sql "drop table $t"
}

Write-Host ""
Write-Host "=== D-prep. upsert fixture ===" -ForegroundColor Cyan
Invoke-Probe -Group 'D-prep' -Label 'create t_mprobe_u' -Ext -Kind ddl -Sql 'create table t_mprobe_u(id int primary key, v varchar(20))'
Invoke-Probe -Group 'D-prep' -Label 'seed row'          -Ext -Kind dml -Sql "insert into t_mprobe_u(id,v) values(1,'a')"

Write-Host ""
Write-Host "=== D. upsert ===" -ForegroundColor Cyan
Invoke-Probe -Group 'D' -Label 'ON DUPLICATE KEY UPDATE (MySQL)' -Sql "insert into t_mprobe_u(id,v) values(1,'b') on duplicate key update v='b'" -Kind dml
Invoke-Probe -Group 'D' -Label 'MERGE (ANSI/Oracle)'             -Sql "merge into t_mprobe_u t using (select 1 as id, 'c' as v from dual) s on (t.id=s.id) when matched then update set t.v=s.v when not matched then insert (id,v) values(s.id,s.v)" -Kind dml
Invoke-Probe -Group 'D-ext' -Label 'read back t_mprobe_u'        -Ext -Kind query -Sql 'select id, v from t_mprobe_u order by id'

Write-Host ""
Write-Host "=== E. functions and operators ===" -ForegroundColor Cyan
Invoke-Probe -Group 'E' -Label 'concat()'          -Sql "select concat('a','b') as c from dual"   -Kind query
Invoke-Probe -Group 'E' -Label '|| concatenation'  -Sql "select 'a' || 'b' as c from dual"         -Kind query
Invoke-Probe -Group 'E' -Label 'now()'             -Sql 'select now() as c from dual'              -Kind query
Invoke-Probe -Group 'E' -Label 'ifnull() MySQL'    -Sql "select ifnull(null,'x') as c from dual"   -Kind query
Invoke-Probe -Group 'E' -Label 'nvl() Oracle'      -Sql "select nvl(null,'x') as c from dual"      -Kind query
Invoke-Probe -Group 'E' -Label 'cast(int as char)' -Sql 'select cast(1 as char) as c from dual'    -Kind query
Invoke-Probe -Group 'E' -Label 'json_object()'     -Sql "select json_object('k',1) as c from dual" -Kind query

Write-Host ""
Write-Host "=== F. misc ===" -ForegroundColor Cyan
Invoke-Probe -Group 'F' -Label 'where exists subquery'            -Sql 'select 1 from dual where exists (select 1 from dual)' -Kind query
Invoke-Probe -Group 'F' -Label 'drop table if exists (absent)'    -Sql 'drop table if exists t_mprobe_a'                       -Kind ddl
Invoke-Probe -Group 'F-ext' -Label 'recreate t_mprobe_a'          -Ext -Kind ddl   -Sql 'create table t_mprobe_a(id int primary key)'
Invoke-Probe -Group 'F-ext' -Label 'drop table if exists (present)' -Ext -Kind ddl -Sql 'drop table if exists t_mprobe_a'
Invoke-Probe -Group 'F-ext' -Label 'verify t_mprobe_a gone'       -Ext -Kind query -Sql "select count(*) as n from user_tables where table_name = 'T_MPROBE_A'"
Invoke-Probe -Group 'F' -Label 'truncate table'                   -Sql 'truncate table t_mprobe_u'                             -Kind ddl

Write-Host ""
Write-Host "=== G. server parameters ===" -ForegroundColor Cyan
Invoke-Probe -Group 'G' -Label 'v$dm_ini key params' -Sql "select para_name, para_value from v`$dm_ini where para_name in ('COMPATIBLE_MODE','ENABLE_ENCRYPT','PORT_NUM')" -Kind query
Invoke-Probe -Group 'G' -Label 'v$version'           -Sql 'select * from v$version' -Kind query
Invoke-Probe -Group 'G-ext' -Label 'v$dm_ini COMPAT/encryption params' -Ext -Kind query -MaxRows 50 `
    -Sql "select para_name, para_value from v`$dm_ini where para_name in ('COMPATIBLE_MODE','CASE_SENSITIVE','BLANK_PAD_MODE','LENGTH_IN_CHAR','ENABLE_ENCRYPT','PORT_NUM')"
Invoke-Probe -Group 'G-ext' -Label 'v$dm_ini rows matching COMPATIBLE' -Ext -Kind query -MaxRows 50 `
    -Sql "select para_name, para_value from v`$dm_ini where para_name like '%COMPATIBLE%'"
Invoke-Probe -Group 'G-ext' -Label 'count of v$dm_ini rows (sanity)'   -Ext -Kind query `
    -Sql 'select count(*) as n from v$dm_ini'

Write-Host ""
Write-Host "=== H-ext. does AUTO_INCREMENT actually generate values ===" -ForegroundColor Cyan
Invoke-Probe -Group 'H-ext' -Label 'recreate t_mprobe_a(auto_increment)' -Ext -Kind ddl   -Sql 'create table t_mprobe_a(id int auto_increment primary key, v varchar(20))'
Invoke-Probe -Group 'H-ext' -Label 'insert without id (1)'              -Ext -Kind dml   -Sql "insert into t_mprobe_a(v) values('p')"
Invoke-Probe -Group 'H-ext' -Label 'insert without id (2)'              -Ext -Kind dml   -Sql "insert into t_mprobe_a(v) values('q')"
Invoke-Probe -Group 'H-ext' -Label 'read generated ids'                 -Ext -Kind query -Sql 'select id, v from t_mprobe_a order by id'

# --------------------------------------------------------------------------
# 6. Cleanup -- leave no objects behind
# --------------------------------------------------------------------------
Write-Host ""
Write-Host "=== cleanup ===" -ForegroundColor DarkGray
$cleanup = New-Object System.Collections.Generic.List[object]
foreach ($t in @('T_MPROBE_A', 'T_MPROBE_B', 'T_MPROBE_C', 'T_MPROBE_D', 'T_MPROBE_U')) {
    Close-Cursor-Safe $gStmt
    $rcIf = [DmDpi]::dpi_exec_direct($gStmt, (AsciiZ "drop table if exists $t"))
    $dIf  = Get-Diag ([Int16]3) $gStmt
    Close-Cursor-Safe $gStmt
    $how = 'drop table if exists: ok'
    if ($rcIf -ne 0) {
        $codesIf = ($dIf.Recs | ForEach-Object { $_.code }) -join ','
        $rcPl = [DmDpi]::dpi_exec_direct($gStmt, (AsciiZ "drop table $t"))
        $dPl  = Get-Diag ([Int16]3) $gStmt
        Close-Cursor-Safe $gStmt
        if ($rcPl -eq 0) { $how = "drop-if-exists rc=$rcIf [$codesIf] -> plain drop ok" }
        else { $how = "both failed: drop-if-exists rc=$rcIf [$codesIf] / drop rc=$rcPl [$((($dPl.Recs | ForEach-Object { $_.code }) -join ','))]" }
    }
    $cleanup.Add((New-Object psobject -Property @{ table = $t; how = $how }))
}

# final check: no T_MPROBE objects may remain
Close-Cursor-Safe $gStmt
[void][DmDpi]::dpi_exec_direct($gStmt, (AsciiZ "select count(*) as n from user_tables where table_name like 'T_MPROBE%'"))
[Int16]$nc0 = 0
[void][DmDpi]::dpi_number_columns($gStmt, [ref]$nc0)
$leftover = '<n/a>'
[long]$rn0 = 0
if ([DmDpi]::dpi_fetch($gStmt, [ref]$rn0) -eq 0) { $leftover = Read-Cell $gStmt 1 }
Close-Cursor-Safe $gStmt
Write-Host ("[cleanup] leftover T_MPROBE* objects = {0}" -f $leftover)

# --------------------------------------------------------------------------
# 7. Release + emit
# --------------------------------------------------------------------------
[void][DmDpi]::dpi_free_stmt($gStmt)
[void][DmDpi]::dpi_logout($gCon)
[void][DmDpi]::dpi_free_con($gCon)
[void][DmDpi]::dpi_free_env($gEnv)

$summary = New-Object psobject -Property @{
    generated_at        = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
    server              = $Server
    user                = $User
    dpi_path            = $DpiPath
    number_columns_mode = $NumberColumnsMode
    leftover_objects    = $leftover
    cleanup             = $cleanup
    results             = $script:Results
}
Write-Text-File $OutJson ($summary | ConvertTo-Json -Depth 6)
Write-Host "[ok] JSON -> $OutJson" -ForegroundColor Green

# ---- machine-readable markdown table (ASCII; Chinese prose lives in docs/) ----
function Esc([string]$s) {
    if ($null -eq $s) { return '' }
    return (($s -replace '\|', '\|') -replace "`r?`n", ' ')
}

$md = New-Object System.Collections.Generic.List[string]
$md.Add('<!-- generated by D:\Rust\dameng\bench\dm-dialect-probe.ps1 -->')
$md.Add('')
$md.Add("- generated_at: $($summary.generated_at)")
$md.Add("- server: $Server   user: $User   dpi: $DpiPath")
$md.Add("- dpi_number_columns mode used: $NumberColumnsMode")
$md.Add("- leftover T_MPROBE* objects after cleanup: $leftover")
$md.Add('')
$md.Add('| # | grp | label | SQL | kind | rc | err_code | diag (raw) | ncols | rows | row_count | first-row sample | verdict |')
$md.Add('|---|-----|-------|-----|------|----|----------|------------|-------|------|-----------|------------------|---------|')

foreach ($r in $script:Results) {
    $ec    = if ($null -eq $r.err_code) { '' } else { $r.err_code }
    $diag  = if ([string]::IsNullOrEmpty($r.diag)) { '' } else { Esc $r.diag }
    $rcv   = if ($null -eq $r.rowcount) { '' } else { $r.rowcount }
    $md.Add("| $($r.idx) | $($r.group) | $($r.label) | $(Esc $r.sql) | $($r.kind) | $($r.rc) | $ec | $diag | $($r.ncols) | $($r.rows) | $rcv | $(Esc $r.sample) | $($r.verdict) |")
}

$md.Add('')
$md.Add('cleanup:')
$md.Add('')
$md.Add('| table | result |')
$md.Add('|-------|--------|')
foreach ($c in $cleanup) { $md.Add("| $($c.table) | $(Esc $c.how) |") }

Write-Text-File $OutTable ($md -join "`r`n")
Write-Host "[ok] table -> $OutTable" -ForegroundColor Green

Write-Host ""
Write-Host "=== verdict tally ===" -ForegroundColor Cyan
$script:Results | Group-Object verdict | ForEach-Object { Write-Host ("  {0}: {1}" -f $_.Name, $_.Count) }
