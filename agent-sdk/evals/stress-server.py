#!/usr/bin/env python3
"""服务卡死复现脚本：并发 SSE + 面板 API，同时监测 /health 是否失联。

用法：
    python evals/stress-server.py --base http://127.0.0.1:4096 --rounds 3 --sse 8 --api 8

判定：每轮期间主线程持续打 /health（2s 超时）。若某次 /health 超时/失败即记为"卡死"，
打印当时的并发情况与最后响应时间，供定位（日志停在哪个请求 = 卡死入口）。
"""

import argparse
import json
import socket
import threading
import time
import urllib.request

socket.setdefaulttimeout(3)


def get(url, token=None, timeout=3):
    req = urllib.request.Request(url)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read(4096).decode("utf-8", "replace")
    except Exception as e:
        return 0, f"{type(e).__name__}: {e}"


def sse_worker(base, stop, log, idx):
    """开一条 SSE 连接，读一点就粗暴关闭（模拟浏览器/页面刷新）。"""
    try:
        req = urllib.request.Request(base + "/events/stream")
        with urllib.request.urlopen(req, timeout=30) as resp:
            resp.read(256)
            time.sleep(0.2)
    except Exception as e:
        log.append(f"sse#{idx}: {type(e).__name__}")
    finally:
        stop.append(idx)


def api_worker(base, token, log, idx):
    for path in ("/metrics/overview", "/automations", "/sessions", "/plugins", "/skills"):
        status, _ = get(base + path, token)
        if status != 200:
            log.append(f"api#{idx} {path} => {status}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:4096")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--sse", type=int, default=8)
    parser.add_argument("--api", type=int, default=8)
    args = parser.parse_args()

    status, text = get(args.base + "/auth/token")
    token = json.loads(text)["token"]
    print(f"起始 /health = {get(args.base + '/health')[0]}")

    stuck = False
    for r in range(1, args.rounds + 1):
        log, stop = [], []
        threads = []
        for i in range(args.sse):
            threads.append(threading.Thread(target=sse_worker, args=(args.base, stop, log, i), daemon=True))
        for i in range(args.api):
            threads.append(threading.Thread(target=api_worker, args=(args.base, token, log, i), daemon=True))
        for t in threads:
            t.start()
        # 主线程边跑边探活
        for _ in range(10):
            st, _ = get(args.base + "/health", timeout=2)
            if st != 200:
                stuck = True
                print(f"❌ 第 {r} 轮：/health 失联（status={st}），alive_threads={threading.active_count()}")
                break
            time.sleep(0.4)
        for t in threads:
            t.join(timeout=5)
        alive = sum(1 for t in threads if t.is_alive())
        print(f"轮次 {r}: sse={args.sse} api={args.api} 残留未结束线程={alive} 日志={log[:3]}")
        if stuck:
            break
        time.sleep(1)

    st, body = get(args.base + "/health", timeout=3)
    print(f"\n结束 /health = {st} {body[:80]}")
    if stuck:
        print("结论：复现成功——服务在高并发 SSE + API 期间失去响应。")
        return 1
    print("结论：本轮未复现卡死（需换更长时间/更多连接，或开启 tracing 抓现场）。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
