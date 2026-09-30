# OwO Agent 输入法桥接最小包：owo-agent.exe（serve-ime）+ onnxruntime.dll + OCR 三件套
#   + README-IME.txt（四步接入）+ start-serve-ime.cmd（双击启动）→ dist/OwO-Agent-IME-<版本>-<配置>.zip
# 与 package-desktop.ps1 的差异：不含桌面壳/skills/NSIS/SBOM——输入法用户只需要核心服务。
# 用法：powershell -ExecutionPolicy Bypass -File scripts\package-serve-ime.ps1 [-Configuration release|debug]
param(
    [ValidateSet("release", "debug")]
    [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$cargo = if ($env:OWO_CARGO) {
    $env:OWO_CARGO
} elseif (Test-Path (Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe")) {
    Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
} else {
    "cargo"
}

$root = Split-Path $PSScriptRoot -Parent

function Get-WorkspaceVersion {
    $cargoToml = Get-Content -LiteralPath (Join-Path $root "Cargo.toml") -Encoding UTF8
    foreach ($line in $cargoToml) {
        if ($line -match '^version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    return "0.0.0"
}
$version = Get-WorkspaceVersion
Write-Host "[package-ime] 版本（来自 Cargo.toml）：$version"

$dist = Join-Path $root "dist\OwO-Agent-IME"
$configArgs = @()
if ($Configuration -eq "release") {
    $configArgs = @("--release")
}
if (Test-Path $dist) {
    Remove-Item -LiteralPath $dist -Recurse -Force
}
New-Item -ItemType Directory -Path $dist -Force | Out-Null

Push-Location $root
try {
    Write-Host "[package-ime] 构建核心服务（$Configuration）..."
    & $cargo build -p owo-agent-cli @configArgs
    if ($LASTEXITCODE -ne 0) { throw "核心服务构建失败" }
} finally {
    Pop-Location
}

$targetDir = Join-Path $root "target\$Configuration"
Copy-Item -LiteralPath (Join-Path $targetDir "owo-agent.exe") -Destination $dist
if (Test-Path (Join-Path $root "settings.example.json")) {
    Copy-Item -LiteralPath (Join-Path $root "settings.example.json") -Destination $dist
}
if (Test-Path (Join-Path $root "models\ocr")) {
    Write-Host "[package-ime] 附带本地 ONNX OCR 模型（models/ocr，无网可用）..."
    Copy-Item -LiteralPath (Join-Path $root "models") -Destination $dist -Recurse
}

# onnxruntime.dll：本地构建产物优先，缺失则从官方 release 下载。
# 下载失败不阻塞打包：ort 按 load-dynamic 加载，DLL 缺失时本地 OCR 自动降级
# Paddle 云 / Media.Ocr（onnx_ocr.rs 的内建降级链），其余能力不受影响。
$onnxRuntimeDll = Join-Path $dist "onnxruntime.dll"
if (-not (Test-Path $onnxRuntimeDll)) {
    $builtDll = Join-Path $targetDir "onnxruntime.dll"
    if (Test-Path $builtDll) {
        Copy-Item -LiteralPath $builtDll -Destination $onnxRuntimeDll
    } else {
        try {
            Write-Host "[package-ime] 下载 ONNX Runtime 1.28.0 x64（约 22MB）..."
            $ortZip = Join-Path $env:TEMP "onnxruntime-win-x64-1.28.0.zip"
            Invoke-WebRequest -Uri "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-win-x64-1.28.0.zip" -OutFile $ortZip -UseBasicParsing -TimeoutSec 60
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $ortArchive = [System.IO.Compression.ZipFile]::OpenRead($ortZip)
            try {
                $entry = $ortArchive.GetEntry("onnxruntime-win-x64-1.28.0/lib/onnxruntime.dll")
                if ($null -eq $entry) { throw "onnxruntime 包内缺少 lib\onnxruntime.dll" }
                [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $onnxRuntimeDll, $true)
            } finally {
                $ortArchive.Dispose()
            }
            Remove-Item $ortZip -Force
        } catch {
            Write-Warning "[package-ime] onnxruntime.dll 获取失败（$($_.Exception.Message)）——继续打包；该包内本地 ONNX OCR 将不可用（自动降级 Paddle 云 / Media.Ocr）。联网后重跑本脚本或手动放置 onnxruntime.dll 到包根即可补齐。"
        }
    }
    if (Test-Path $onnxRuntimeDll) {
        Write-Host "[package-ime] onnxruntime.dll 已就位"
    }
}

# 一键启动：双击后在本目录起 serve-ime（管道 + HTTP 双面）；窗口保持显示日志。
# 注意：cmd.exe 不认 UTF-8 BOM（会把 BOM 当命令），必须写无 BOM UTF-8。
$cmdContent = @"
@echo off
chcp 65001 >nul
cd /d "%~dp0"
echo [OwO Agent IME] 启动 serve-ime（管道 \\.\pipe\OwO.Agent.External.v1 + HTTP 127.0.0.1:4096）
echo [OwO Agent IME] 沙箱工作区 = 本目录；如需改动请在下方命令加 --workspace 路径
owo-agent.exe serve-ime --port 4096 --workspace .
pause
"@
[System.IO.File]::WriteAllText(
    (Join-Path $dist "start-serve-ime.cmd"),
    $cmdContent,
    (New-Object System.Text.UTF8Encoding $false)
)

@"
OwO Agent 输入法桥接包（serve-ime，v$version）

四步接入：
  1. 安装 OwO 输入法 0.2.3 及以上（官网安装包）。
  2. 安装 Agent IPC 连接器：OwO 设置中心 → 插件 → 安装
     org.owo.agent-ipc-1.2.0.owopkg（随 OwO-release 仓库 artifacts/plugins/ 分发）。
  3. 在 OwO 设置中心对该插件【明确授权】；管道端点保持默认
     \\.\pipe\OwO.Agent.External.v1。
  4. 双击 start-serve-ime.cmd，在任意输入框键入 v 前缀拼音
     （如 vbangwozhaowenjian），候选框将出现 Agent 命令。

环境变量（可选，启动前设置）：
  OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_MODEL   模型凭据（需自行配置）
  OWO_AGENT_DATA                                    数据目录（默认 %LOCALAPPDATA%\OwO\Agent）
  OWO_ONNX_OCR_MODEL_DIR                            本地 OCR 模型目录（默认随包 models/ocr）

说明：
  - 管道面：OwO 输入法连接器调用（协议 v3）；HTTP 面：桌宠/Web 工作台（127.0.0.1:4096）。
  - 高风险命令不会直接执行——候选框返回「打开确认界面」，在 HTTP 面审批。
  - 与 owo-agent serve 共用数据目录，二者不可同时运行（PidFile 互斥）。
"@ | Set-Content -LiteralPath (Join-Path $dist "README-IME.txt") -Encoding UTF8

$zip = Join-Path $root "dist\OwO-Agent-IME-$version-$Configuration.zip"
if (Test-Path $zip) {
    Remove-Item -LiteralPath $zip -Force
}
Compress-Archive -Path (Join-Path $dist "*") -DestinationPath $zip
Write-Host "[package-ime] 输入法桥接包完成：$zip"
