"""最小 OpenAI 兼容 mock 端点：仅供 owo-bridge 联调（live_engine.rs）与验收演示。

行为（确定性，便于断言）：
- 首轮：返回 tool_calls → write_file(hello.txt, "hi")
- 含 tool 结果的轮次：返回 final 文本
- 同时支持非流式与 `stream: true`（SSE chunk）；不需要真实 API Key。

用法：
    python mock_openai.py [port]      # 默认 8990
    # 引擎侧：
    set OPENAI_API_KEY=mock
    set OPENAI_BASE_URL=http://127.0.0.1:8990/v1
    set OPENAI_MODEL=mock-model
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8990
FINAL_TEXT = "已完成：已写入 hello.txt。"


def build_tool_call():
    return {
        "id": "call-mock-1",
        "type": "function",
        "function": {
            "name": "write_file",
            "arguments": json.dumps({"path": "hello.txt", "content": "hi"}),
        },
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):  # 静音访问日志
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        payload = json.loads(self.rfile.read(length) or b"{}")
        messages = payload.get("messages", [])
        has_tool_result = any(item.get("role") == "tool" for item in messages)
        stream = bool(payload.get("stream"))

        if stream:
            self.stream_response(has_tool_result, payload.get("model", "mock"))
        else:
            self.json_response(has_tool_result, payload.get("model", "mock"))

    def json_response(self, has_tool_result, model):
        message = (
            {"role": "assistant", "content": FINAL_TEXT}
            if has_tool_result
            else {"role": "assistant", "content": None, "tool_calls": [build_tool_call()]}
        )
        body = json.dumps(
            {
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "model": model,
                "choices": [{"index": 0, "message": message, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def stream_response(self, has_tool_result, model):
        def chunk(delta, finish_reason=None):
            return (
                "data: "
                + json.dumps(
                    {
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [
                            {"index": 0, "delta": delta, "finish_reason": finish_reason}
                        ],
                    }
                )
                + "\n\n"
            ).encode()

        pieces = []
        if has_tool_result:
            pieces.append(chunk({"role": "assistant", "content": FINAL_TEXT}))
        else:
            pieces.append(
                chunk(
                    {
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call-mock-1",
                                "type": "function",
                                "function": {
                                    "name": "write_file",
                                    "arguments": json.dumps(
                                        {"path": "hello.txt", "content": "hi"}
                                    ),
                                },
                            }
                        ],
                    }
                )
            )
        pieces.append(chunk({}, finish_reason="stop"))
        pieces.append(b"data: [DONE]\n\n")

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for piece in pieces:
            self.wfile.write(piece)
            self.wfile.flush()


if __name__ == "__main__":
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"mock openai listening on http://127.0.0.1:{PORT}/v1", flush=True)
    server.serve_forever()
