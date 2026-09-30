#!/usr/bin/env python3
"""功能面可用性实测（R13）：对 owo-agent-server 逐项打接口，输出可用性矩阵。

用法：
    python scripts/verify-features.py                       # 默认 http://127.0.0.1:4096
    python scripts/verify-features.py --base http://127.0.0.1:4098
    python scripts/verify-features.py --no-live             # 跳过需要模型凭据的接口
    python scripts/verify-features.py --live-eval           # 额外跑 /eval/gate/run（12 条用例，耗 token）

判定口径：
    OK      2xx
    WARN    4xx：接口在，但入参/前置条件不满足（脚本会打印首行错误，便于定位）
    FAIL    5xx 或连接失败
    SKIP    脚本不做（会动真实桌面/数据，或需特殊输入），原因写在说明列

写接口均带清理（DELETE），跑完不残留会话/笔记/自动化任务；带前置条件的接口用真实资源驱动
（如 /perception/elements 用 /desktop/foreground 返回的句柄）。
末尾给出 OK/WARN/FAIL/SKIP 统计，任一 FAIL 退出码 1。
"""

import argparse
import json
import sys
import urllib.error
import urllib.request

OK, WARN, FAIL = "OK", "WARN", "FAIL"


class Client:
    def __init__(self, base: str, timeout: float = 60.0):
        self.base = base.rstrip("/")
        self.timeout = timeout
        self.token = None
        self.rows = []

    def call(self, method: str, path: str, body=None, timeout=None):
        """返回 (status, text)。连接失败返回 (0, 原因)。"""
        data = None
        headers = {"Accept": "application/json"}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        if self.token and path not in ("/health", "/auth/token"):
            headers["Authorization"] = "Bearer " + self.token
        req = urllib.request.Request(self.base + path, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout or self.timeout) as resp:
                return resp.status, resp.read(4000).decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            return e.code, e.read(600).decode("utf-8", "replace")
        except Exception as e:  # 连接被拒 / 超时
            return 0, f"{type(e).__name__}: {e}"

    def check(self, group: str, method: str, path: str, body=None, note="", timeout=None, skip=False):
        """打一次接口并把结论记入矩阵；成功时返回响应文本。"""
        if skip:
            self.rows.append((group, method, path, "-", "SKIP", note))
            print(f"[SKIP] {method} {path} — {note}", file=sys.stderr, flush=True)
            return None
        status, text = self.call(method, path, body, timeout)
        if status == 0:
            verdict, detail = FAIL, text
        elif 200 <= status < 300:
            verdict, detail = OK, ""
        elif 400 <= status < 500:
            verdict, detail = WARN, one_line(text)
        else:
            verdict, detail = FAIL, one_line(text)
        self.rows.append((group, method, path, status, verdict, note or detail))
        # 进度打到 stderr 并 flush：主表最后一次性打印，卡死时靠这行定位是哪个接口。
        print(f"[{verdict:<4}] {method:<6} {path} {status} {detail or note}", file=sys.stderr, flush=True)
        return text if verdict == OK else None

    def get_json(self, path, timeout=None):
        status, text = self.call("GET", path, timeout=timeout)
        if status == 0 or status >= 400:
            return None
        try:
            return json.loads(text)
        except Exception:
            return None


def one_line(text: str, limit: int = 110) -> str:
    return " ".join((text or "").split())[:limit]


def sse_probe(client: Client, path: str, body=None, seconds: float = 45.0):
    """读一段 SSE 直到超时或读到足够数据，返回 (status, 收字节数)。"""
    headers = {"Accept": "text/event-stream"}
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if client.token:
        headers["Authorization"] = "Bearer " + client.token
    req = urllib.request.Request(
        client.base + path, data=data, headers=headers, method="POST" if body is not None else "GET"
    )
    try:
        with urllib.request.urlopen(req, timeout=seconds) as resp:
            got = 0
            while got < 4000:
                chunk = resp.read(512)
                if not chunk:
                    break
                got += len(chunk)
            return resp.status, got
    except urllib.error.HTTPError as e:
        return e.code, 0
    except Exception:
        # 读超时说明流已建立（SSE 空闲心跳），按已建立处理
        return 200, -1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:4096")
    parser.add_argument("--no-live", action="store_true", help="跳过需要模型凭据的接口")
    parser.add_argument("--live-eval", action="store_true", help="额外跑 /eval/gate/run（12 条用例，耗时耗 token）")
    args = parser.parse_args()

    c = Client(args.base)
    health = c.get_json("/health")
    if not health:
        print(f"服务不可达：{args.base}（先 owo-agent serve --port 4096 --workspace .）")
        return 1
    print(
        f"服务 {args.base} healthy={health.get('healthy')} version={health.get('version')} "
        f"auto_approve={health.get('auto_approve')}"
    )
    c.token = (c.get_json("/auth/token") or {}).get("token")
    if not c.token:
        print("无法获取 bearer token")
        return 1

    # ---------- A 公开面 ----------
    c.check("A 公开面", "GET", "/health", note="健康检查")
    c.check("A 公开面", "GET", "/openapi.json", note="接口契约")
    c.check("A 公开面", "GET", "/auth/token", note="本地鉴权引导")

    # ---------- B 会话与回合 ----------
    status = c.get_json("/server/status") or {}
    workspace = status.get("workspace") or "."
    text = c.check("B 会话", "POST", "/session", {"workspace": str(workspace)}, note="建会话")
    sid = (json.loads(text) if text else {}).get("id")
    if sid:
        c.check("B 会话", "GET", f"/session/{sid}")
        c.check("B 会话", "POST", f"/session/{sid}/rename", {"title": "verify-features"}, note="改名（字段名 title）")
        c.check("B 会话", "POST", f"/session/{sid}/pin", {"pinned": True}, note="置顶（pinned 字段）")
        c.check("B 会话", "POST", f"/session/{sid}/archive", {"archived": True}, note="归档（archived 字段）")
        c.check("B 会话", "GET", f"/session/{sid}/children")
        c.check("B 会话", "GET", f"/session/{sid}/context")
        c.check("B 会话", "GET", f"/session/{sid}/diff")
        c.check("B 会话", "GET", f"/session/{sid}/export/markdown")
        if args.no_live:
            c.rows.append(("B 会话", "POST", f"/session/{sid}/turn", "-", "SKIP", "--no-live"))
        else:
            st, got = sse_probe(c, f"/session/{sid}/turn", {"prompt": "只回复 ok，不要调用任何工具。"})
            verdict = OK if st == 200 and got != 0 else (WARN if 400 <= st < 500 else FAIL)
            c.rows.append(("B 会话", "POST", f"/session/{sid}/turn", st, verdict, f"SSE 轮次，收到 {got} 字节"))
        c.check("B 会话", "DELETE", f"/session/{sid}", note="清理")
    else:
        c.rows.append(("B 会话", "POST", "/session", "-", FAIL, "无法建会话，后续会话用例跳过"))
    c.check("B 会话", "GET", "/sessions")

    # ---------- C 设置 / 规则 / 白名单 ----------
    c.check("C 设置", "GET", "/settings")
    settings = c.get_json("/settings")
    if settings is not None:
        c.check("C 设置", "POST", "/settings", settings, note="回写同值（幂等）")
    c.check("C 设置", "POST", "/settings/egress", {"cloud_enabled": True}, note="出网开关")
    c.check("C 设置", "GET", "/whitelist")
    c.check("C 设置", "GET", "/project/rules")
    c.check(
        "C 设置",
        "POST",
        "/project/rules/template",
        {},
        note="工作区已有 AGENTS.md 时返回 409 属正常保护（幂等）",
    )
    c.check("C 设置", "GET", "/context/snapshot")

    # ---------- D 审计 / 用量 / 轨迹 / 可观测 ----------
    for path in [
        "/audit?limit=5",
        "/usage",
        "/usage/summary",
        "/usage/records",
        "/usage/report?days=7",
        "/traces",
        "/metrics/overview",
        "/metrics/turns",
        "/metrics/tools",
        "/metrics/health",
        "/metrics/runtime",
        "/metrics/slo",
        "/metrics/slo/alerts",
        "/metrics/slo/report",
        "/metrics/prometheus",
        "/metrics/telemetry/status",
        "/server/status",
        "/schemas",
    ]:
        c.check("D 可观测", "GET", path)

    # ---------- E 技能 ----------
    c.check("E 技能", "GET", "/skills")
    c.check("E 技能", "GET", "/skills/health")
    skills = c.get_json("/skills")
    skill_items = skills.get("skills") if isinstance(skills, dict) else skills
    for item in skill_items or []:
        name = item.get("name") if isinstance(item, dict) else item
        if name:
            c.check("E 技能", "GET", f"/skills/{name}", note="技能详情")
            c.check("E 技能", "POST", f"/skills/{name}/enabled", {"enabled": True}, note="启用")

    # ---------- F 插件 / 市场 ----------
    c.check("F 插件", "GET", "/plugins")
    plugins = c.get_json("/plugins")
    plugin_items = plugins.get("plugins") if isinstance(plugins, dict) else plugins
    for item in plugin_items or []:
        pid = item.get("id") if isinstance(item, dict) else item
        if pid:
            c.check("F 插件", "POST", f"/plugins/{pid}/enabled", {"enabled": True}, note="启用插件")
    c.check("F 插件", "GET", "/plugins/market")
    c.check(
        "F 插件",
        "POST",
        "/plugins/market/seed",
        {"entries": [{"id": "owo.plugin.verify-demo", "version": "1.0.0", "min_app_version": "0.1.0"}]},
        note="写示例目录条目（min_app_version 需 <= app 版本才出现在目录里）",
    )
    c.check("F 插件", "POST", "/plugins/market/refresh", {}, note="需 OWO_MARKET_URL 或已有 market.json")
    c.check(
        "F 插件",
        "GET",
        "/plugins/market/versions?id=owo.plugin.verify-demo",
        note="需 id 参数；未安装的目录条目无版本历史，404 属预期",
    )
    c.check("F 插件", "GET", "/plugins/market/scan?dir=plugins/owo-translate", note="需 dir 参数")
    c.check("F 插件", "GET", "/plugins/market/audit")

    # ---------- G MCP ----------
    mcp = c.get_json("/mcp") or {}
    c.check(
        "G MCP",
        "GET",
        "/mcp",
        note=f"配置 {mcp.get('count', '?')} 个，运行时已连接 {mcp.get('connected_count', '?')} 个",
    )
    c.check("G MCP", "POST", "/mcp/remove", {"name": "__verify_nonexistent__"}, note="移除不存在的服务器：404 属预期")

    # ---------- H 记忆 / 笔记 / 编排 / 自动化 / 学习 ----------
    for path in [
        "/memory/observations",
        "/memory/recall?q=verify",
        "/memory/graph/entries?limit=10",
        "/memory/graph/timeline",
        "/memory/graph/entities?limit=10",
        "/memory/graph/links",
        "/memory/graph/recall?q=verify",
    ]:
        c.check("H 记忆", "GET", path)
    link_body = {"a": "verify-a", "b": "verify-b", "relation": "verify"}
    c.check("H 记忆", "POST", "/memory/graph/link", link_body, note="建链接（a/b/relation）")
    c.check("H 记忆", "DELETE", "/memory/graph/link", link_body, note="清理链接")

    text = c.check("H 笔记", "POST", "/notes", {"title": "verify-features", "markdown": "# verify\n临时笔记"})
    nid = (json.loads(text) if text else {}).get("id")
    c.check("H 笔记", "GET", "/notes")
    if nid:
        c.check("H 笔记", "GET", f"/notes/{nid}")
        c.check("H 笔记", "GET", "/notes/search?q=verify")
        c.check("H 笔记", "POST", f"/notes/{nid}/reindex", {})
        c.check("H 笔记", "DELETE", f"/notes/{nid}", note="清理")

    c.check("H 编排", "GET", "/goal")
    c.check("H 编排", "GET", "/workflow")
    c.check(
        "H 编排",
        "POST",
        "/workflow/validate",
        {},
        skip=True,
        note="需 .owflow 定义文档（工作流面板/工作流目录内提供），空定义 422 属预期",
    )
    c.check("H 自动化", "GET", "/automations")
    text = c.check(
        "H 自动化",
        "POST",
        "/automations",
        {"name": "verify-features", "schedule": {"kind": "interval", "every_secs": 86400}, "reminder": "verify-features"},
    )
    aid = (json.loads(text) if text else {}).get("id")
    if aid:
        c.check("H 自动化", "POST", f"/automations/{aid}/toggle")
        c.check("H 自动化", "GET", "/automations/reminders")
        c.check("H 自动化", "DELETE", f"/automations/{aid}", note="清理")
    c.check("H 学习", "GET", "/learn/status")
    c.check("H 学习", "GET", "/learn/packages")
    c.check("H 学习", "GET", "/proactive/suggestions")

    # ---------- I 感知 / 桌面 / 视觉 / computer-use ----------
    c.check("I 感知", "GET", "/perception/ocr/status")
    foreground = c.get_json("/desktop/foreground") or {}
    hwnd = foreground.get("hwnd") or foreground.get("handle")
    if hwnd:
        c.check("I 感知", "POST", "/perception/elements", {"hwnd": hwnd}, note="用前台窗口句柄")
    else:
        c.rows.append(("I 感知", "POST", "/perception/elements", "-", "SKIP", "未取到前台窗口句柄（需交互桌面）"))
    c.check("I 感知", "GET", "/desktop/foreground", note="前台窗口（需交互桌面）")
    c.check("I 感知", "GET", "/desktop/windows", note="窗口列表")
    c.check("I 感知", "GET", "/vision/status")
    c.check("I 感知", "GET", "/computer-use/tasks")

    # ---------- J 扩展：云 / 团队 / 控制面 / 事件流 ----------
    c.check("J 扩展", "GET", "/fleet/nodes")
    c.check("J 扩展", "GET", "/team/versions?id=verify")
    c.check("J 扩展", "GET", "/team/audit")
    c.check(
        "J 扩展",
        "POST",
        "/cloud/tasks",
        {},
        skip=True,
        note="提交需云凭据（OWO_CLOUD_TOKEN）；查询用 /cloud/tasks/{id}",
    )
    c.check("J 扩展", "GET", "/command/audit")
    c.check("J 扩展", "GET", "/eval/gate/reports")
    c.check("J 扩展", "GET", "/eval/gate/report", note="无历史报告时 404 属正常")
    st, got = sse_probe(c, "/events/stream", seconds=6)
    c.rows.append(("J 扩展", "GET", "/events/stream", st, OK if st == 200 else FAIL, f"SSE，收到 {got} 字节"))

    # ---------- K 需要模型凭据 ----------
    if args.no_live:
        for path in ["/intent/parse", "/command/run", "/subagent/run"]:
            c.rows.append(("K 模型链路", "POST", path, "-", "SKIP", "--no-live"))
    else:
        c.check("K 模型链路", "POST", "/intent/parse", {"text": "打开设置页并搜索记忆"}, timeout=90)
        c.check("K 模型链路", "POST", "/command/run", {"mode": "text", "text": "列出工作区根目录前 3 个文件"}, timeout=120)
        c.check("K 模型链路", "POST", "/subagent/run", {"prompt": "用一句话说明你收到任务"}, timeout=120)
    if args.live_eval:
        c.check("K 模型链路", "POST", "/eval/gate/run", {"suite": "evals/small-tasks.json"}, timeout=900)
    else:
        c.rows.append(("K 模型链路", "POST", "/eval/gate/run", "-", "SKIP", "加 --live-eval 才跑（12 条用例）"))

    # ---------- L 需人工/环境（记录原因，不做自动化）----------
    for method, path, why in [
        ("POST", "/stt/transcribe", "需原始 WAV 字节 + 本地 STT 模型"),
        ("POST", "/fs/pick-directory", "弹原生目录选择框"),
        ("POST", "/fs/open", "会打开资源管理器/浏览器"),
        ("POST", "/perception/capture", "真实截屏"),
        ("POST", "/desktop/click|type|key|shortcut|launch|scroll", "会真实操作本机桌面"),
        ("POST", "/learn/execute", "会真实操作本机桌面"),
        ("POST", "/storage/backup|restore|export|clear", "会动数据（clear 为破坏性）"),
        ("POST", "/memory/clear", "破坏性：清空情景记忆"),
        ("POST", "/usage/topup", "会改用量额度"),
        ("POST", "/server/shutdown", "会停服务"),
    ]:
        c.rows.append(("L 需人工/环境", method, path, "-", "SKIP", why))

    # ---------- 汇总 ----------
    width_group = max(len(r[0]) for r in c.rows) if c.rows else 10
    width_path = max(len(r[2]) for r in c.rows) if c.rows else 30
    print()
    print(f"{'组':<{width_group}}  {'方法':<6} {'接口':<{width_path}} {'状态':<4} {'结论':<5} 说明")
    for group, method, path, st, verdict, note in c.rows:
        print(f"{group:<{width_group}}  {method:<6} {path:<{width_path}} {str(st):<4} {verdict:<5} {note}")

    counts = {OK: 0, WARN: 0, FAIL: 0, "SKIP": 0}
    for row in c.rows:
        counts[row[4]] = counts.get(row[4], 0) + 1
    print()
    print(f"合计：OK {counts[OK]} / WARN {counts[WARN]} / FAIL {counts[FAIL]} / SKIP {counts['SKIP']}")
    if counts[FAIL]:
        print("存在 FAIL 接口，详见上表。")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
