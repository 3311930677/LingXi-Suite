param(
    [int]$Port = 9527
)

$ErrorActionPreference = "Stop"

function Invoke-ImeQuery {
    param([string]$Pinyin)

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $client.Connect("127.0.0.1", $Port)
        $stream = $client.GetStream()
        $encoding = [System.Text.UTF8Encoding]::new($false)
        $writer = [System.IO.StreamWriter]::new($stream, $encoding)
        $reader = [System.IO.StreamReader]::new($stream, $encoding)
        $writer.AutoFlush = $true
        $request = @{ type = "query"; pinyin = $Pinyin; limit = 8 } | ConvertTo-Json -Compress
        $writer.WriteLine($request)
        $line = $reader.ReadLine()
        return $line | ConvertFrom-Json
    }
    finally {
        $client.Dispose()
    }
}

# Keep this Windows PowerShell 5 compatible: ASCII source avoids UTF-8 BOM
# ambiguity; expected Chinese strings are constructed from Unicode code points.
$Cases = @(
    @{ Pinyin = "nihao"; Expected = -join ([char]0x4F60, [char]0x597D); First = $true },
    @{ Pinyin = "suoyi"; Expected = -join ([char]0x6240, [char]0x4EE5); First = $true },
    @{ Pinyin = "dang"; Expected = [string][char]0x5F53; First = $true },
    @{ Pinyin = "zhong"; Expected = [string][char]0x4E2D; First = $true },
    @{ Pinyin = "ren"; Expected = [string][char]0x4EBA; First = $true },
    @{ Pinyin = "de"; Expected = [string][char]0x7684; First = $true },
    # Simplified-pinyin checks: nh contains 你好; zgr ranks 中国人 first.
    @{ Pinyin = "nh"; Expected = -join ([char]0x4F60, [char]0x597D); First = $false },
    @{ Pinyin = "zgr"; Expected = -join ([char]0x4E2D, [char]0x56FD, [char]0x4EBA); First = $true }
)

$Failed = 0
foreach ($case in $Cases) {
    try {
        $response = Invoke-ImeQuery -Pinyin $case.Pinyin
        $texts = @($response.candidates | ForEach-Object { $_.text })
        $first = if ($texts.Count -gt 0) { $texts[0] } else { "EMPTY" }
        $passed = if ($case.First) {
            $first -eq $case.Expected
        } else {
            $texts -contains $case.Expected
        }
        if ($passed) {
            Write-Host ("PASS  {0,-8} -> {1}" -f $case.Pinyin, ($texts -join " / ")) -ForegroundColor Green
        } else {
            $mode = if ($case.First) { "first" } else { "contained" }
            Write-Host ("FAIL  {0,-8} expected {1} to be {2}, got {3}" -f $case.Pinyin, $case.Expected, $mode, ($texts -join " / ")) -ForegroundColor Red
            $Failed++
        }
    }
    catch {
        Write-Host "Cannot connect to ime-server on 127.0.0.1:$Port" -ForegroundColor Red
        Write-Host "Start the project with .\scripts\run-dev.ps1 first."
        exit 1
    }
}

if ($Failed -gt 0) {
    Write-Host "$Failed candidate checks failed." -ForegroundColor Red
    exit 1
}

Write-Host "All IME candidate checks passed." -ForegroundColor Green
