@echo off
chcp 65001 >nul
cd /d "%~dp0"
echo [OwO Agent IME] 启动 serve-ime（管道 \\.\pipe\OwO.Agent.External.v1 + HTTP 127.0.0.1:4096）
echo [OwO Agent IME] 沙箱工作区 = 本目录；如需改动请在下方命令加 --workspace 路径
owo-agent.exe serve-ime --port 4096 --workspace .
pause