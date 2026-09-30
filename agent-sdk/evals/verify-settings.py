#!/usr/bin/env python3
"""验证「思考强度」「模型选择」是否真正进入模型请求（抓包实验）。

做法：起本地 mock OpenAI 端点 + 用**临时工作区**起独立服务实例（端口 4071/4072，
数据目录也指向临时目录），跑真实回合，然后读 mock 日志断言请求体字段。
不改动用户现有 settings.json、不消耗真实模型 token。

断言：
  S1 设置页 model（进程无 OPENAI_MODEL）      → 请求体 model = 设置值
  S2 设置页 model（进程有 OPENAI_MODEL）      → 请求体 model = 环境变量值（设置被压过）
  S3 设置页 reasoning_effort=high              → 请求体带 reasoning_effort=high
  S4 同一进程内改回默认（POST /settings）      → 下一条请求不带该字段（热生效）
  S5 非法档位 unsupported                      → 不下发
  S6 会话级 model（POST /session {model}）     → 不影响请求体 model
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = r"d:\working OWOWOWOWOWO\OwO\agent-sdk"
BIN = os.path.join(ROOT, "target", "debug", "owo-agent.exe")
MOCK_PORT = 4319
BASE = f"http://127.0.0.1:{MOCK_PORT}/v1"


def http(method: str, url: str, body=None, token=None, timeout=30):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, f"{type(e).__name__}: {e}"


def wait_health(port: int, timeout=40):
    for _ in range(timeout * 2):
        status, _ = http("GET", f"http://127.0.0.1:{port}/health", timeout=3)
        if status == 200:
            return True
        time.sleep(0.5)
    return False


def write_settings(ws: str, model: str, reasoning, base_url=BASE):
    payload = {
        "model": model,
        "provider": {"base_url": base_url, "api_key": "mock-key"},
        "reasoning_effort": reasoning,
    }
    os.makedirs(ws, exist_ok=True)
    with open(os.path.join(ws, "settings.json"), "w", encoding="utf-8") as fh:
        json.dump(payload, fh, ensure_ascii=False, indent=2)


def start_server(port: int, ws: str, data_root: str, extra_env=None) -> subprocess.Popen:
    env = dict(os.environ)
    for key in ("OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_MODEL", "OWO_REASONING_EFFORT",
                "OWO_BROWSER_NODE", "OWO_BROWSER_NODE_PATH"):
        env.pop(key, None)
    env["OWO_AGENT_DATA"] = data_root
    env.update(extra_env or {})
    proc = subprocess.Popen(
        [BIN, "serve", "--port", str(port), "--workspace", ws],
        cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    if not wait_health(port):
        proc.kill()
        raise RuntimeError(f"服务 {port} 未就绪")
    return proc


def run_turn(port: int, ws: str, prompt="只回复 ok", session_model=None):
    _, text = http("GET", f"http://127.0.0.1:{port}/auth/token")
    token = json.loads(text)["token"]
    body = {"workspace": ws}
    if session_model:
        body["model"] = session_model
    status, text = http("POST", f"http://127.0.0.1:{port}/session", body, token)
    sid = json.loads(text)["id"]
    # SSE：mock 立即回复，读完即结束
    http("POST", f"http://127.0.0.1:{port}/session/{sid}/turn", {"prompt": prompt}, token, timeout=60)
    http("DELETE", f"http://127.0.0.1:{port}/session/{sid}", token)
    return sid


def mock_entries(log_path: str):
    entries = []
    with open(log_path, "r", encoding="utf-8") as fh:
        for line in fh:
            data = json.loads(line)
            if "full" not in data:
                entries.append(data)
    return entries


def main() -> int:
    work = tempfile.mkdtemp(prefix="owo-settings-verify-")
    ws = os.path.join(work, "ws")
    data_root = os.path.join(work, "data")
    log_path = os.path.join(work, "mock.jsonl")
    mock = subprocess.Popen(
        [sys.executable, os.path.join(ROOT, "evals", "mock-openai.py"),
         "--port", str(MOCK_PORT), "--log", log_path],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(1.5)

    results = []
    server_a = None
    server_b = None
    server_c = None
    try:
        # ---- 服务器 A：设置 model=mock-model-A + reasoning_effort=high，进程无 OPENAI_MODEL ----
        write_settings(ws, "mock-model-A", "high")
        server_a = start_server(4071, ws, data_root)
        before = len(mock_entries(log_path)) if os.path.exists(log_path) else 0

        run_turn(4071, ws)
        run_turn(4071, ws, session_model="session-model-X")
        entries = mock_entries(log_path)
        e1, e2 = entries[before], entries[before + 1]
        results.append(("S1 设置页 model（无 env）", "mock-model-A", e1.get("model"), e1.get("model") == "mock-model-A"))
        results.append(("S6 会话级 model 不影响请求", "mock-model-A", e2.get("model"), e2.get("model") == "mock-model-A"))
        results.append(("S3 reasoning_effort=high 下发", "high", e1.get("reasoning_effort"), e1.get("reasoning_effort") == "high"))
        results.append(("S3 请求体 keys 含 reasoning_effort", True, "reasoning_effort" in (e1.get("keys") or []), "reasoning_effort" in (e1.get("keys") or [])))

        # ---- S4：同一进程内改回默认（走 POST /settings，UI 保存同路径）----
        _, text = http("GET", "http://127.0.0.1:4071/auth/token")
        token = json.loads(text)["token"]
        status, cur = http("GET", "http://127.0.0.1:4071/settings", token=token)
        settings = json.loads(cur)
        settings["reasoning_effort"] = None
        status, resp = http("POST", "http://127.0.0.1:4071/settings", settings, token)
        run_turn(4071, ws)
        e3 = mock_entries(log_path)[-1]
        results.append(("S4 改回默认后不再下发（不重启）", None, e3.get("reasoning_effort"), e3.get("reasoning_effort") is None))

        # ---- S5：非法档位 ----
        status, cur = http("GET", "http://127.0.0.1:4071/settings", token=token)
        settings = json.loads(cur)
        settings["reasoning_effort"] = "unsupported"
        http("POST", "http://127.0.0.1:4071/settings", settings, token)
        run_turn(4071, ws)
        e4 = mock_entries(log_path)[-1]
        results.append(("S5 非法档位不下发", None, e4.get("reasoning_effort"), e4.get("reasoning_effort") is None))

        # ---- 服务器 B：设置 model=mock-model-A + 进程带 OPENAI_MODEL=env-model-Z ----
        # 修复后预期：设置页保存值优先（修复前是 env 把设置页压掉）。
        ws_b = os.path.join(work, "ws-b")
        write_settings(ws_b, "mock-model-A", None)
        server_b = start_server(4072, ws_b, os.path.join(work, "data-b"), {"OPENAI_MODEL": "env-model-Z"})
        run_turn(4072, ws_b)
        e5 = mock_entries(log_path)[-1]
        results.append(("S2 设置页 model 压过 env", "mock-model-A", e5.get("model"), e5.get("model") == "mock-model-A"))

        # ---- 服务器 C：设置里没配 model（null）+ env OPENAI_MODEL → env 仍生效 ----
        ws_c = os.path.join(work, "ws-c")
        write_settings(ws_c, None, None)
        server_c = start_server(4073, ws_c, os.path.join(work, "data-c"), {"OPENAI_MODEL": "env-model-Z"})
        run_turn(4073, ws_c)
        e6 = mock_entries(log_path)[-1]
        results.append(("S8 未配置时回退 env", "env-model-Z", e6.get("model"), e6.get("model") == "env-model-Z"))

        # ---- S7：CLI `--model` 必须真正进请求体（此前只写进会话记录）----
        ws_cli = os.path.join(work, "ws-cli")
        write_settings(ws_cli, "mock-model-A", None)
        env_cli = dict(os.environ)
        env_cli["OPENAI_MODEL"] = "env-model-Z"
        subprocess.run(
            [BIN, "turn", "--workspace", ws_cli, "--model", "cli-model-Y",
             "--prompt", "只回复 ok", "--no-approval"],
            cwd=ROOT, env=env_cli, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180,
        )
        e7 = mock_entries(log_path)[-1]
        results.append(("S7 CLI --model 进请求体", "cli-model-Y", e7.get("model"), e7.get("model") == "cli-model-Y"))

        print()
        print(f"{'用例':<34} {'期望':<16} {'实际':<18} 结论")
        ok = True
        for name, expect, actual, passed in results:
            ok = ok and bool(passed)
            print(f"{name:<34} {str(expect):<16} {str(actual):<18} {'通过' if passed else '未通过'}")
        print()
        print("全部通过" if ok else "存在未通过项，见上表")
        return 0 if ok else 1
    finally:
        for proc in (server_a, server_b, server_c, mock):
            if proc:
                proc.kill()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
