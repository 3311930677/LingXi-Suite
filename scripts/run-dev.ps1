$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$OverlayDir = Join-Path $ProjectRoot "apps\overlay"

# LingXi is an AI selection assistant only. OwO owns all system input-method behavior;
# never start the retired ime-server or install a low-level keyboard hook here.
$OldProcesses = Get-Process -Name "ime-server", "overlay" -ErrorAction SilentlyContinue
if ($OldProcesses) {
    Write-Host "Stopping old LingXi processes..." -ForegroundColor Yellow
    $OldProcesses | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

$overlayCommand = "Set-Location -LiteralPath '$OverlayDir'; cargo run"
Start-Process powershell.exe -ArgumentList @("-NoExit", "-Command", $overlayCommand)

Write-Host "LingXi AI assistant started." -ForegroundColor Green
Write-Host "  Ctrl+Alt+Space captures the selected text."
Write-Host "  Ctrl+Alt+Backspace undoes the last write-back."
Write-Host "  Keyboard input is never intercepted; OwO is the only IME."
