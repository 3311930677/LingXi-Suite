# LingXi × OwO Agent 架构总览

> 目的：一张图说清 轻环 / 重环 / overlay / 桌宠 的数据流，避免再摸索两套 Agent。
> 决策基线：`docs/agent-optimization-plan.md` §〇 —— **UI 一律走重环，轻环只做离线文本改写兜底**。

## 一、三环数据流

```
┌─ apps/overlay（Tauri 2，MSVC，workspace 外独立构建）──────────────┐
│  主面板（对话/工具/设置）+ 桌宠 pet.js（4 态状态机 + 皮肤）        │
│  小工具 widgets/（计算器/天气/翻译/剪贴板/取色器…）                │
│                                                                  │
│  轻环: crates/lingxi-agent + agent.rs（选区改写快路径，DenyAll，   │
│        无流式/无取消/10 轮 —— 只保留离线润色用途）                 │
│  重环: apps/overlay/src/owo_bridge.rs → crates/owo-bridge         │
└──────────────────────────┬───────────────────────────────────────┘
                           │ HTTP + SSE（127.0.0.1:4096，Bearer 鉴权）
                           │ 契约 = owo-agent /openapi.json（冻结）
                ┌──────────▼──────────────────────────────────┐
                │ OwO/agent-sdk（重环执行引擎，工具链 pin 1.97.1）│
                │  owo-agent-server：session/turn(SSE)/审批/diff │
                │  /revert/abort；Windows Job/AppContainer 沙箱  │
                │  42 工具 + MCP + 感知/学习 + 审计 + traces     │
                │  ───────────────────────────────────────────  │
                │  owo-agent serve-ime（2026-09-29 新增，同进程）│
                │   管道面 crates/owo-agent-ime（输入法调用）    │
                └──────────┬──────────────────────────────────┘
                           │ 命名管道（协议 v3，一问一答，10s 超时）
                           │ \\.\pipe\OwO.Agent.External.v1
                ┌──────────▼──────────────────────────────────┐
                │ OwO 输入法（TSF，"v" 前缀 Agent 模式）         │
                │  org.owo.agent-ipc 连接器 → 适配器 → 引擎      │
                └─────────────────────────────────────────────┘
```

**输入法环要点**（实现于 `OwO/agent-sdk/crates/owo-agent-ime` + `serve-ime` 子命令）：

- 单进程双面：`serve-ime` 同时监听 HTTP（桌宠/工作台）与命名管道（输入法），
  会话与审批状态共享；与 `serve` 通过数据目录 PidFile 互斥；
- 长回合适配：模型回合秒级～分钟级，管道 10s 一问一答 → `submit` 立即回
  `thinking` + `retry_after_ms`，轮询取结果；cancel 触发 `POST /abort`；
- 可信界面：高风险命令不返回可执行候选，只给「打开确认界面」，由桌宠审批卡
  或 Web 工作台确认（协议红线）；
- 隐私：密码框（`sensitive_input=true`）时 prompt 不携带任何上下文。

## 二、重环契约要点（桥接只依赖这些）

| 用途 | 端点 / 机制 |
|---|---|
| 健康与配对 | `GET /health`；`GET /auth/token`（公开配对，客户端缓存 token，401 自动刷新重试一次） |
| 回合 | `POST /session/{id}/turn` → SSE tagged JSON（`token_delta`/`tool_use`/`tool_result`/`permission_request`/`final`…） |
| 审批 | `POST /session/{id}/permission/{rid}`；服务端等待 **300s 超时默认 Deny** |
| 改动审阅 | `GET /session/{id}/diff`、`POST /session/{id}/revert`、`POST /session/{id}/abort` |

运行时硬约束（前端必须遵守）：

1. **CSP**：WebView2 里禁止 `fetch("http://127.0.0.1:4096")`，一切走 Tauri `invoke`（HTTP 由 Rust 侧 ureq 发出）。
2. **审批会阻塞**：不响应 = 300s 后 Deny = 任务白跑；UI 必须给审批卡片 + 倒计时。
3. **同会话并发 turn 返回 409**：提示「上一轮还在跑」，提供中止入口。
4. **沙箱根约束**：会话 workspace 必须与引擎 `--workspace` 启动参数完全一致，否则文件工具报路径越界。

## 三、仓库地图

| 位置 | 内容 |
|---|---|
| `OwO/agent-sdk` | 重环引擎（core/server/cli/sim workspace）。存储加密契约测试 `crates/owo-agent-core/tests/crypto_contract.rs`（P0-1 门禁，替代已删除的假验证脚本） |
| `OwO/agent-sdk/crates/owo-agent-ime` | **输入法 Agent IPC v3 适配**（协议/管道/状态机/bridge，`serve-ime` 支撑）。契约测试 `cargo test -p owo-agent-ime`；端到端冒烟 `scripts/ime-pipe-probe.ps1`；集成验收 `scripts/acceptance.ps1` |
| `crates/owo-bridge` | 引擎 HTTP/SSE 客户端（ureq + native-tls，与 overlay 同 TLS 栈避免 ring 被 Smart App Control 拦截） |
| `crates/lingxi-agent` | 轻环 Agent（离线兜底，不承担工具型任务） |
| `apps/overlay` | Tauri 2 桌面壳（MSVC，workspace `exclude`；`cargo run` 启动主面板+桌宠） |
| `apps/ime-server` 等 | 输入法相关 apps（GNU workspace 成员） |
| `crates/assistant-inference` | candle 本地推理（workspace `exclude`：GNU 环境缺 dlltool 无法 codegen，随 overlay 走 MSVC） |

## 四、工具链与门禁

- **LingXi workspace（GNU）**：根 `rust-toolchain.toml` pin `stable-x86_64-pc-windows-gnu`；`apps/overlay` 用嵌套 `rust-toolchain.toml` 覆盖为 MSVC。
- **OwO/agent-sdk**：根 `rust-toolchain.toml` pin `1.97.1`（MSVC）。
- LingXi 门禁：`cargo check/test --workspace`（GNU）；`cd apps/overlay && cargo check`（MSVC）。
- agent-sdk 门禁：`powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1`（utf8→fmt→clippy `-D warnings`→test→route-contract→node→ts）；存储加密契约：`cargo test -p owo-agent-core --test crypto_contract`。
- 仓库路径必须**纯英文且不含空格**（GNU 链接器与部分脚本对中文/空格路径敏感；`Start-Process` 传含空格路径需两层引号）。
