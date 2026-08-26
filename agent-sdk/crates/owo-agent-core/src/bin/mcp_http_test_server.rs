//! HTTP MCP 测试夹具服务器：argv[1] = 端口，POST /mcp 走 JSON-RPC 2.0。
//!
//! 供 mcp_tests.rs 的 `CARGO_BIN_EXE_owo-mcp-http-test-server` 引用。
//! 每连接处理一个请求后关闭（Connection: close），提供 echo 工具。
//! 不引入 HTTP 框架依赖，用 tokio::net 手写最小 HTTP/1.1 子集。

use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

fn tools_response() -> Value {
    json!({
        "tools": [
            {
                "name": "echo",
                "description": "回显输入文本",
                "inputSchema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }
        ]
    })
}

fn handle_message(message: &Value) -> Value {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    if id.is_null() {
        // 通知：不响应。
        return Value::Null;
    }
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "owo-mcp-http-test-server",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "tools/list" => Ok(tools_response()),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            match name {
                "echo" => Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": arguments
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                        }
                    ]
                })),
                other => Err(json!({
                    "code": -32602,
                    "message": format!("未知工具：{other}")
                })),
            }
        }
        other => Err(json!({
            "code": -32601,
            "message": format!("未知方法：{other}")
        })),
    };
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    }
}

/// 读取一个 HTTP/1.1 请求（头 + Content-Length 定长的 body）。
async fn read_request(
    reader: &mut BufReader<impl AsyncReadExt + Unpin>,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    // 读到 \r\n\r\n 为止。
    loop {
        let n = reader.read(&mut byte).await?;
        if n == 0 {
            return Ok(None);
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head_text = String::from_utf8_lossy(&head);
    let content_length = head_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }
    Ok(Some(body))
}

async fn serve_connection(stream: TcpStream) {
    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let body = match read_request(&mut reader).await {
        Ok(Some(body)) => body,
        _ => return,
    };
    let response = serde_json::from_slice::<Value>(&body)
        .map(|message| handle_message(&message))
        .unwrap_or(Value::Null);
    let payload = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
    let http = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    let _ = writer.write_all(http.as_bytes()).await;
    let _ = writer.flush().await;
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(9527);
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("owo-mcp-http-test-server 绑定 127.0.0.1:{port} 失败：{error}");
            std::process::exit(1);
        }
    };
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
        };
        tokio::spawn(serve_connection(stream));
    }
}
