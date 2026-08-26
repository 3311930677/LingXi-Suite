//! stdio MCP 测试夹具服务器：按行 JSON-RPC 2.0。
//!
//! 供 mcp_tests.rs 的 `CARGO_BIN_EXE_owo-mcp-test-server` 引用，提供：
//! - echo(text)：原样回显
//! - add(a, b)：求和并以文本返回
//! - hang(sleep_ms)：挂起指定毫秒（超时/重连测试用）
//! 未知工具返回 JSON-RPC 错误。收到 `exit` 通知或 stdin EOF 后退出。

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
            },
            {
                "name": "add",
                "description": "两数求和",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "a": { "type": "integer" },
                        "b": { "type": "integer" }
                    },
                    "required": ["a", "b"]
                }
            },
            {
                "name": "hang",
                "description": "挂起指定毫秒（超时测试用）",
                "inputSchema": {
                    "type": "object",
                    "properties": { "sleep_ms": { "type": "integer" } },
                    "required": ["sleep_ms"]
                }
            }
        ]
    })
}

fn text_content(text: String) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ] })
}

fn call_tool(name: &str, arguments: &Value) -> Result<Value, Value> {
    match name {
        "echo" => {
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(text_content(text.to_string()))
        }
        "add" => {
            let a = arguments.get("a").and_then(Value::as_i64).unwrap_or(0);
            let b = arguments.get("b").and_then(Value::as_i64).unwrap_or(0);
            Ok(text_content((a + b).to_string()))
        }
        "hang" => Ok(json!({ "hang": true })),
        other => Err(json!({
            "code": -32602,
            "message": format!("未知工具：{other}")
        })),
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let message: Value = match serde_json::from_str(line.trim()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == "exit" {
            break;
        }
        let Some(id) = id else {
            // 通知（无 id）：无需响应。
            continue;
        };
        let result = match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "owo-mcp-test-server",
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
                if name == "hang" {
                    let sleep_ms = arguments
                        .get("sleep_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(5_000);
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                }
                call_tool(name, &arguments)
            }
            other => Err(json!({
                "code": -32601,
                "message": format!("未知方法：{other}")
            })),
        };
        let response = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        let payload = serde_json::to_string(&response).unwrap_or_default();
        let out = format!("{payload}\n");
        if stdout.write_all(out.as_bytes()).await.is_err() {
            break;
        }
        if stdout.flush().await.is_err() {
            break;
        }
    }
}
