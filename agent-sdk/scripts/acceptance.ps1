# acceptance.ps1 - OwO x agent-sdk integration acceptance gate (E7).
#
# Real assertions, non-zero exit on any failure. Mirrors plan section E7.1:
#   1) fmt + clippy (workspace, all targets, -D warnings)         -> static gate
#   2) E1 IME protocol + pipe + bridge contract tests              -> cargo test -p owo-agent-ime
#   3) E2 permission rules (remember persistence + hard-deny)      -> permission_rules
#   4) E3 real tokenizer (tiktoken vs legacy formula)              -> tokenizer_accuracy
#   5) E4 loop guard / step timeout / budget circuit breaker       -> loop_guard_tests
#   6) E5 plan tool + PlanUpdate event                             -> plan_tool_tests
#   7) workspace-wide test suite (skip with -SkipFull)
#   8) live pipe probe against a running serve-ime (opt-in: -Live)
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\acceptance.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\acceptance.ps1 -SkipFull
#   powershell -ExecutionPolicy Bypass -File scripts\acceptance.ps1 -Live
#
# Note: this machine's rustc can crash on default parallelism; the script uses
# -j 1 by default (override with -Jobs N).

[CmdletBinding()]
param(
    [switch]$SkipFmt,
    [switch]$SkipClippy,
    [switch]$SkipFull,
    [switch]$Live,
    [int]$Jobs = 1
)

$ErrorActionPreference = 'Continue'
$global:Failures = @()
$root = Split-Path -Parent $PSScriptRoot

function Invoke-Step {
    param([string]$Name, [string]$Evidence, [scriptblock]$Block)
    Write-Host ("== {0} ==" -f $Name) -ForegroundColor Cyan
    Write-Host ("   evidence: {0}" -f $Evidence) -ForegroundColor DarkGray
    & $Block
    if ($LASTEXITCODE -ne 0) {
        Write-Host ("FAIL: {0} (exit {1})" -f $Name, $LASTEXITCODE) -ForegroundColor Red
        $global:Failures += $Name
    }
    else {
        Write-Host ("PASS: {0}" -f $Name) -ForegroundColor Green
    }
}

Push-Location $root
try {
    if (-not $SkipFmt) {
        Invoke-Step -Name "rustfmt check" -Evidence "cargo fmt --all -- --check" -Block {
            cargo fmt --all -- --check
        }
    }

    if (-not $SkipClippy) {
        Invoke-Step -Name "clippy strict" -Evidence "cargo clippy --workspace --all-targets --locked -j $Jobs -- -D warnings" -Block {
            cargo clippy --workspace --all-targets --locked -j $Jobs -- -D warnings
        }
    }

    Invoke-Step -Name "E1 ime contract" -Evidence "cargo test -p owo-agent-ime" -Block {
        cargo test -p owo-agent-ime --locked -j $Jobs
    }

    Invoke-Step -Name "E2 permission rules" -Evidence "cargo test -p owo-agent-core --test permission_rules" -Block {
        cargo test -p owo-agent-core --test permission_rules --locked -j $Jobs
    }

    Invoke-Step -Name "E3 tokenizer accuracy" -Evidence "cargo test -p owo-agent-core --test tokenizer_accuracy" -Block {
        cargo test -p owo-agent-core --test tokenizer_accuracy --locked -j $Jobs
    }

    Invoke-Step -Name "E4 loop guard / timeout / budget" -Evidence "cargo test -p owo-agent-core --test loop_guard_tests" -Block {
        cargo test -p owo-agent-core --test loop_guard_tests --locked -j $Jobs
    }

    Invoke-Step -Name "E5 plan tool" -Evidence "cargo test -p owo-agent-core --test plan_tool_tests" -Block {
        cargo test -p owo-agent-core --test plan_tool_tests --locked -j $Jobs
    }

    if (-not $SkipFull) {
        Invoke-Step -Name "workspace test suite" -Evidence "cargo test --workspace --locked -j $Jobs" -Block {
            cargo test --workspace --locked -j $Jobs
        }
    }

    if ($Live) {
        Invoke-Step -Name "live pipe probe" -Evidence "scripts\ime-pipe-probe.ps1 (serve-ime must be running)" -Block {
            & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "ime-pipe-probe.ps1")
        }
    }
    else {
        Write-Host "SKIP: live pipe probe (use -Live with serve-ime running)" -ForegroundColor Yellow
    }
}
finally {
    Pop-Location
}

Write-Host ""
if ($global:Failures.Count -gt 0) {
    Write-Host ("ACCEPTANCE FAIL: {0}" -f ($global:Failures -join ', ')) -ForegroundColor Red
    exit 1
}
Write-Host "ACCEPTANCE PASS" -ForegroundColor Green
exit 0
