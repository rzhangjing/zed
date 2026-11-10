# Capture a top-level window by process name (foreground + CopyFromScreen crop),
# plus its console buffer lines. Usage: cap-window.ps1 -Proc cesium-app -Out target\cap.png
param(
    [string]$Proc = "cesium-app",
    [string]$Out = "target\cap-window.png"
)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WinCap {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lpEnumFunc, IntPtr lParam);
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int n);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$target = Get-Process $Proc -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $target) { "no process $Proc"; exit 1 }
$wins = New-Object System.Collections.ArrayList
$cb = [WinCap+EnumProc]{
    param($h, $l)
    $pid2 = 0
    [void][WinCap]::GetWindowThreadProcessId($h, [ref]$pid2)
    if ($pid2 -eq $target.Id -and [WinCap]::IsWindowVisible($h)) {
        $sb = New-Object System.Text.StringBuilder 512
        [void][WinCap]::GetWindowTextW($h, $sb, 512)
        $r = New-Object WinCap+RECT
        [void][WinCap]::GetWindowRect($h, [ref]$r)
        [void]$wins.Add([pscustomobject]@{ Hwnd = $h; Title = $sb.ToString(); Rect = $r; W = $r.Right - $r.Left; H = $r.Bottom - $r.Top })
    }
    return $true
}
[void][WinCap]::EnumWindows($cb, [IntPtr]::Zero)
$wins | ForEach-Object { "HWND=$($_.Hwnd) $($_.W)x$($_.H) @($($_.Rect.Left),$($_.Rect.Top)) :: $($_.Title)" }
# main window = widest visible one with a non-path title
$main = $wins | Sort-Object -Property W -Descending | Select-Object -First 1
if (-not $main) { "no window"; exit 1 }
[void][WinCap]::ShowWindow($main.Hwnd, 9)     # SW_RESTORE
[void][WinCap]::SetForegroundWindow($main.Hwnd)
Start-Sleep -Milliseconds 600
$bmp = New-Object System.Drawing.Bitmap $main.W, $main.H
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($main.Rect.Left, $main.Rect.Top, 0, 0, (New-Object System.Drawing.Size $main.W, $main.H))
$path = Join-Path (Get-Location) $Out
$bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"saved $path ($($main.W)x$($main.H)) hwnd=$($main.Hwnd)"
