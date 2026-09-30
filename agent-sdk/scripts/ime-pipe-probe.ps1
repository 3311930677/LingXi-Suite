# OwO Agent IPC pipe probe: send one `submit` frame to serve-ime and poll until final.
# Protocol v3 (see OwO-release/docs/plugins/agent-ipc-integration.md).
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\ime-pipe-probe.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\ime-pipe-probe.ps1 -UserInput "bangwozhaowenjian"
#
# Requires: `owo-agent serve-ime` running (default pipe \\.\pipe\OwO.Agent.External.v1).
# Exit code: 0 = final/waiting response received; 1 = timeout or error.

[CmdletBinding()]
param(
    [string]$PipeName = 'OwO.Agent.External.v1',
    [string]$UserInput = 'bangwozhaowenjian',
    [string]$SessionId = '0123456789abcdef0123456789abcdef',
    [int]$PollSeconds = 60
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

function Send-Frame {
    param([System.IO.Pipes.NamedPipeClientStream]$Pipe, [string]$Json)
    $bytes = [Text.Encoding]::UTF8.GetBytes($Json)
    $header = [BitConverter]::GetBytes([uint32]$bytes.Length)
    $Pipe.Write($header, 0, 4)
    $Pipe.Write($bytes, 0, $bytes.Length)
    $Pipe.Flush()
}

function Receive-Frame {
    param([System.IO.Pipes.NamedPipeClientStream]$Pipe)
    $header = New-Object byte[] 4
    $read = 0
    while ($read -lt 4) {
        $n = $Pipe.Read($header, $read, 4 - $read)
        if ($n -le 0) { throw 'Pipe closed while reading frame header' }
        $read += $n
    }
    $size = [BitConverter]::ToUInt32($header, 0)
    $payload = New-Object byte[] $size
    $read = 0
    while ($read -lt $size) {
        $n = $Pipe.Read($payload, $read, $size - $read)
        if ($n -le 0) { throw 'Pipe closed while reading frame payload' }
        $read += $n
    }
    [Text.Encoding]::UTF8.GetString($payload)
}

function Invoke-AgentFrame {
    param([string]$Json)
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(
        '.', $PipeName, [System.IO.Pipes.PipeDirection]::InOut)
    try {
        $pipe.Connect(5000)
        Send-Frame -Pipe $pipe -Json $Json
        Receive-Frame -Pipe $pipe
    }
    finally {
        $pipe.Dispose()
    }
}

function New-Request {
    param([string]$Action, [string]$RequestId, [string]$CommandId = '')
    $request = [ordered]@{
        schema_version    = 3
        action            = $Action
        session_id        = $SessionId
        request_id        = $RequestId
        parent_request_id = ''
        idempotency_key   = $RequestId
        capabilities      = @('protocol.negotiation', 'input.structured', 'context.entries',
                              'commands.risk', 'tasks.slots', 'tasks.incremental')
        protocol_min      = 2
        protocol_max      = 3
        required_features = @('protocol.negotiation', 'input.structured', 'commands.risk', 'tasks.slots')
        user_input        = ''
        input             = [ordered]@{
            raw_pinyin         = $UserInput
            segmented_pinyin   = $UserInput
            selected_text      = ''
            pending_pinyin     = ''
            natural_language   = $UserInput
            input_mode         = 'agent'
            correction_enabled = $true
        }
        application       = [ordered]@{
            process_id         = 1
            thread_id          = 1
            executable         = 'notepad.exe'
            window_class       = 'Notepad'
            focus_window_class = 'Edit'
            context_id         = 'ctx-probe-0001'
            sensitive_input    = $false
        }
        session_context   = ''
        context_entries   = @()
        command_id        = $CommandId
        page              = 0
        task_revision     = 0
        slot_updates      = @()
    }
    if ($Action -eq 'submit') { $request['user_input'] = $UserInput }
    ($request | ConvertTo-Json -Depth 8 -Compress)
}

Write-Host "== OwO Agent IPC probe (pipe $PipeName) =="

# 1. submit
$response = Invoke-AgentFrame -Json (New-Request -Action 'submit' -RequestId 'req-probe-1') | ConvertFrom-Json
Write-Host ("[submit] status={0} retry_after_ms={1} revision={2}" -f `
        $response.status, $response.retry_after_ms, $response.state_revision)

# 2. poll until final
$deadline = (Get-Date).AddSeconds($PollSeconds)
$index = 2
while ($response.status -eq 'thinking' -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    $response = Invoke-AgentFrame -Json (New-Request -Action 'poll' -RequestId "req-probe-$index") | ConvertFrom-Json
    $index++
    Write-Host ("[poll] status={0} progress={1}" -f $response.status, $response.progress)
}

Write-Host ("[final] status={0} error_code={1}" -f $response.status, $response.error_code)
Write-Host ("message: {0}" -f $response.message)
foreach ($command in @($response.commands)) {
    if ($null -ne $command) {
        Write-Host ("command: id={0} label={1}" -f $command.id, $command.label)
    }
}

if ($response.status -in @('agent_mode', 'waiting_for_confirmation', 'completed', 'cancelled')) {
    Write-Host 'PROBE PASS'
    exit 0
}
Write-Host 'PROBE FAIL'
exit 1
