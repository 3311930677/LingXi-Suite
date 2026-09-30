# prepare-engine-dist.ps1 - copy the owo-agent engine into apps/overlay/engine-dist/
# for `tauri build` (referenced by bundle.resources in tauri.conf.json).
#
# Usage (any working directory):
#   powershell -File apps/overlay/scripts/prepare-engine-dist.ps1
#   powershell -File apps/overlay/scripts/prepare-engine-dist.ps1 -Profile debug
#
# Note: kept ASCII-only on purpose - Windows PowerShell 5.1 misparses
# BOM-less UTF-8 scripts containing non-ASCII characters.
param(
    [ValidateSet("release", "debug")]
    [string]$Profile = "release",
    # Set to $true to skip stopping a running engine (then engine-dist copy may fail with os error 32).
    [switch]$KeepEngineRunning
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$overlayDir = Split-Path -Parent $scriptDir
# overlayDir     = <workspace>/LingXi-DesktopAgent/apps/overlay
# workspace root = overlayDir minus three levels (app -> apps -> LingXi-DesktopAgent)
$workspaceRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $overlayDir))
$engineExe = Join-Path $workspaceRoot "OwO\agent-sdk\target\$Profile\owo-agent.exe"

if (-not (Test-Path $engineExe)) {
    $profileFlag = if ($Profile -eq "release") { "--release" } else { "" }
    Write-Error "Engine binary not found: $engineExe`nBuild it first in OwO/agent-sdk: cargo build -p owo-agent-cli $profileFlag"
}

$distDir = Join-Path $overlayDir "engine-dist"
New-Item -ItemType Directory -Force -Path $distDir | Out-Null
$target = Join-Path $distDir "owo-agent.exe"

# A running engine locks the destination (and tauri's build script copies this
# file too - os error 32). Stop it unless explicitly kept alive.
if (-not $KeepEngineRunning) {
    $running = Get-Process owo-agent -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host "Stopping running owo-agent (pid $($running.Id -join ',')) to unlock engine-dist..."
        $running | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
}

Copy-Item -Force $engineExe $target
$sizeMb = [math]::Round((Get-Item $target).Length / 1MB, 1)
Write-Host "Ready: $target ($sizeMb MB, from $Profile)"
