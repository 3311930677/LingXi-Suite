# owo-bridge live test launcher: mock LLM endpoint + real engine + live tests.
#
# Usage (from anywhere):
#   powershell -ExecutionPolicy Bypass -File crates\owo-bridge\tests\run_live.ps1
#
# Requires: OwO/agent-sdk CLI built (cargo build -p owo-agent-cli).
# The built-in mock OpenAI endpoint means no real API key is needed.
# NOTE: keep this file ASCII-only: PowerShell 5.1 reads .ps1 as ANSI/GBK,
# so non-ASCII text would corrupt the script (see docs environment notes).

$ErrorActionPreference = "Stop"

$testsDir = $PSScriptRoot
$lingxiRoot = Split-Path -Parent (Split-Path -Parent $testsDir)
$agentSdk = "D:\working OWOWOWOWOWO\OwO\agent-sdk"
$engineExe = Join-Path $agentSdk "target\debug\owo-agent.exe"
$workspace = Join-Path $env:TEMP "owo-bridge-live-ws"
New-Item -ItemType Directory -Force -Path $workspace | Out-Null

if (-not (Test-Path $engineExe)) {
    Write-Host "[live] missing engine build: $engineExe" -ForegroundColor Red
    Write-Host "[live] run first: cd $agentSdk; cargo build -p owo-agent-cli" -ForegroundColor Yellow
    exit 1
}

# The engine sandbox root is fixed at startup (serve --workspace); the session
# workspace must match it exactly, so export the same path for the tests.
$env:OWO_AGENT_WORKSPACE = $workspace

Write-Host "[live] starting mock LLM endpoint (127.0.0.1:8990)..." -ForegroundColor Cyan
$mock = Start-Process -FilePath "python" `
    -ArgumentList "`"$testsDir\mock_openai.py`"", "8990" `
    -PassThru -WindowStyle Hidden
Start-Sleep 1

$env:OPENAI_API_KEY = "mock"
$env:OPENAI_BASE_URL = "http://127.0.0.1:8990/v1"
$env:OPENAI_MODEL = "mock-model"

Write-Host "[live] starting engine (127.0.0.1:4096, workspace=$workspace)..." -ForegroundColor Cyan
$engine = Start-Process -FilePath $engineExe `
    -ArgumentList "serve", "--port", "4096", "--workspace", ('"' + $workspace + '"') `
    -PassThru -WindowStyle Hidden

try {
    $up = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        $probe = curl.exe -s --noproxy "*" --max-time 2 "http://127.0.0.1:4096/health" 2>$null
        if ($probe -match "healthy") { $up = $true; break }
    }
    if (-not $up) { throw "engine not ready within 15s (check $env:TEMP\owo-agent*.log)" }

    Push-Location $lingxiRoot
    try {
        cargo test -p owo-bridge --test live_engine -- --ignored --nocapture --test-threads=1
    }
    finally {
        Pop-Location
    }
}
finally {
    Write-Host "[live] stopping mock and engine..." -ForegroundColor Cyan
    Stop-Process -Id $engine.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $mock.Id -Force -ErrorAction SilentlyContinue
}
