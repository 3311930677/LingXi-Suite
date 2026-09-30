#!/usr/bin/env python3
"""最小 OpenAI 兼容 mock 端点：把收到的请求体原样追加到 JSONL，返回固定回复。

用途：抓包验证「思考强度」「模型选择」这类设置是否真的进了请求体，不消耗真实模型 token。

用法：
    python evals/mock-openai.py --port 4319 --log evals/mock-requests.jsonl

返回：
    - 非流式（stream 缺省/false）：标准 chat.completion JSON。
    - 流式（stream=true）：SSE 分片，最后 [DONE]。
    - GET /v1/models：返回模型列表（部分客户端会探测）。
"""

import argparse
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    log_path = "mock-requests.jsonl"
    protocol_version = "HTTP/1.1"

    def _record(self, raw: bytes) -> dict:
        body = {}
        try:
            body = json.loads(raw.decode("utf-8", "replace") or "{}")
        except Exception:
            body = {"_raw": raw.decode("utf-8", "replace")[:2000]}
        entry = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "path": self.path,
            "model": body.get("model"),
            "stream": bool(body.get("stream")),
            "reasoning_effort": body.get("reasoning_effort"),
            "reasoning": body.get("reasoning"),
            "temperature": body.get("temperature"),
            "max_tokens": body.get("max_tokens"),
            "tools": len(body.get("tools") or []),
            "messages": len(body.get("messages") or []),
            "keys": sorted(body.keys()),
        }
        with open(self.log_path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry, ensure_ascii=False) + "\n")
            fh.write(json.dumps({"full": body}, ensure_ascii=False) + "\n")
        return body

    def do_GET(self):
        payload = json.dumps({"object": "list", "data": [{"id": "mock-model", "object": "model"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b"{}"
        body = self._record(raw)
        model = body.get("model") or "mock-model"
        if body.get("stream"):
            chunks = [
                {"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0, "model": model,
                 "choices": [{"index": 0, "delta": {"role": "assistant", "content": "ok"}, "finish_reason": None}]},
                {"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0, "model": model,
                 "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
            ]
            payload = b"".join(
                b"data: " + json.dumps(chunk).encode() + b"\n\n" for chunk in chunks
            ) + b"data: [DONE]\n\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        payload = json.dumps({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "created": 0,
            "model": model,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, fmt, *args):  # 静默：请求明细只写 JSONL
        return


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=4319)
    parser.add_argument("--log", default="mock-requests.jsonl")
    args = parser.parse_args()
    Handler.log_path = args.log
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"mock openai listening on http://127.0.0.1:{args.port}/v1 (log={args.log})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
