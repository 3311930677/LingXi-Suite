param(
    [string]$RimeDir = ""
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path $PSScriptRoot -Parent
if ([string]::IsNullOrWhiteSpace($RimeDir)) {
    $RimeDir = Join-Path (Split-Path $ProjectRoot -Parent) "rime-ice"
}

$BaseDict = Join-Path $RimeDir "cn_dicts\base.dict.yaml"
$CharDict = Join-Path $RimeDir "cn_dicts\8105.dict.yaml"

if (-not (Test-Path $RimeDir)) {
    Write-Host "Cloning rime-ice into $RimeDir ..." -ForegroundColor Cyan
    git clone https://github.com/iDvel/rime-ice.git $RimeDir --depth 1
    if ($LASTEXITCODE -ne 0) {
        throw "git clone rime-ice failed"
    }
} else {
    Write-Host "Using existing rime-ice at $RimeDir" -ForegroundColor Green
}

if (-not (Test-Path $BaseDict) -or -not (Test-Path $CharDict)) {
    throw "Required dictionaries were not found under $RimeDir\cn_dicts"
}

Write-Host "rime-ice is ready:" -ForegroundColor Green
Write-Host "  $CharDict"
Write-Host "  $BaseDict"
