# Island resource measurement (plan stage 5): idle CPU, RAM, handles, leaks after repeated expand/collapse.
#
# Usage:  powershell -ExecutionPolicy Bypass -File tools\measure.ps1 [-Tabs Stats,Apps] [-Idle 10] [-Cycles 100]
#
# The island is started with a separate temporary APPDATA, so your config, clipboard history and log
# are not touched. Close the running island first (single-instance mutex).
# The script moves the mouse during the run - do not touch it.
# (ASCII only on purpose: Windows PowerShell 5.1 reads BOM-less files as ANSI.)
param(
    [string[]]$Tabs = @("Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"),
    [int]$Idle = 10,
    [int]$Cycles = 100,
    [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\island.exe")
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Native {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
}
"@
[void][Native]::SetProcessDPIAware()
$sw = [Native]::GetSystemMetrics(0)
$sh = [Native]::GetSystemMetrics(1)
$cx = [int]($sw / 2)

function Move-Mouse($x, $y) { [void][Native]::SetCursorPos($x, $y) }

function Sample($p, $seconds) {
    $p.Refresh()
    $cpu0 = $p.TotalProcessorTime.TotalSeconds
    Start-Sleep -Seconds $seconds
    $p.Refresh()
    [pscustomobject]@{
        CpuPct  = [math]::Round(($p.TotalProcessorTime.TotalSeconds - $cpu0) / $seconds * 100, 2)
        WsMB    = [math]::Round($p.WorkingSet64 / 1MB, 1)
        Handles = $p.HandleCount
        Threads = $p.Threads.Count
    }
}

$exe = (Resolve-Path $Exe).Path
$root = Join-Path $env:TEMP ("island-measure-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$rows = @()

foreach ($tab in $Tabs) {
    $appdata = Join-Path $root $tab
    New-Item -ItemType Directory -Force (Join-Path $appdata "island") | Out-Null
    Set-Content -Encoding UTF8 (Join-Path $appdata "island\config.toml") "tabs = [`"$tab`"]`nlive_media = 0`nlive_toasts = 0`n"

    $psi = New-Object System.Diagnostics.ProcessStartInfo $exe
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["APPDATA"] = $appdata
    $psi.EnvironmentVariables["ISLAND_PERF"] = "1"
    Move-Mouse $cx ($sh - 50)
    $p = [System.Diagnostics.Process]::Start($psi)
    try {
        Start-Sleep -Seconds 4

        $collapsed = Sample $p $Idle

        Move-Mouse $cx 1
        Start-Sleep -Milliseconds 300
        Move-Mouse $cx 40
        Start-Sleep -Seconds 3
        $open = Sample $p $Idle
        # Second window: separates one-time first-show work (index, icons, GPU counters) from steady state.
        $open2 = Sample $p $Idle

        $h0 = $p.HandleCount
        for ($i = 0; $i -lt $Cycles; $i++) {
            Move-Mouse $cx 1
            Start-Sleep -Milliseconds 120
            Move-Mouse $cx 40
            Start-Sleep -Milliseconds 450
            Move-Mouse $cx ($sh - 50)
            Start-Sleep -Milliseconds 550
        }
        Start-Sleep -Seconds 2
        $after = Sample $p 3

        $rows += [pscustomobject]@{
            Tab = $tab
            CollapsedCpu = $collapsed.CpuPct; CollapsedMB = $collapsed.WsMB
            OpenCpu = $open.CpuPct; OpenMB = $open.WsMB; Open2Cpu = $open2.CpuPct; Open2MB = $open2.WsMB
            HandlesBefore = $h0; HandlesAfter = $after.Handles; MBAfter = $after.WsMB
            Threads = $after.Threads
        }

        $log = Join-Path $appdata "island\island.log"
        Write-Host "=== ${tab}: log ==="
        if (Test-Path $log) { Get-Content $log -Tail 12 -Encoding UTF8 } else { Write-Host "(no log)" }
    } finally {
        # Do not leave island.exe running if the measurement throws (single-instance mutex).
        if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() } else { Write-Host "!!! process exited on its own" }
    }
}

Move-Mouse $cx ($sh - 50)
$rows | Format-Table -AutoSize
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
