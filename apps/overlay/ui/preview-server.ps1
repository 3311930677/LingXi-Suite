# 临时静态文件服务器：用于浏览器预览灵犀 overlay UI（不依赖 Node）
$root = $PSScriptRoot
$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("http://localhost:8777/")
$listener.Start()
Write-Host "overlay preview on http://localhost:8777"
$types = @{
  ".html" = "text/html; charset=utf-8"
  ".css"  = "text/css; charset=utf-8"
  ".js"   = "text/javascript; charset=utf-8"
  ".png"  = "image/png"
  ".jpg"  = "image/jpeg"
  ".gif"  = "image/gif"
  ".svg"  = "image/svg+xml"
  ".json" = "application/json"
  ".ico"  = "image/x-icon"
}
while ($listener.IsListening) {
  $ctx = $listener.GetContext()
  $p = [System.Uri]::UnescapeDataString($ctx.Request.Url.AbsolutePath)
  if ($p -eq "/") { $p = "/index.html" }
  $file = Join-Path $root ($p.TrimStart("/").Replace("/", "\"))
  if (Test-Path $file -PathType Leaf) {
    $bytes = [System.IO.File]::ReadAllBytes($file)
    $ext = [System.IO.Path]::GetExtension($file).ToLower()
    $ct = if ($types.ContainsKey($ext)) { $types[$ext] } else { "application/octet-stream" }
    $ctx.Response.StatusCode = 200
    $ctx.Response.ContentType = $ct
    $ctx.Response.ContentLength64 = $bytes.Length
    $ctx.Response.OutputStream.Write($bytes, 0, $bytes.Length)
  } else {
    $ctx.Response.StatusCode = 404
    $b = [Text.Encoding]::UTF8.GetBytes("not found")
    $ctx.Response.OutputStream.Write($b, 0, $b.Length)
  }
  $ctx.Response.Close()
}
