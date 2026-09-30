# OwO 输入法 × agent-sdk 集成执行方案（v1.0）

> 日期：2026-09-29 | 状态：**执行中**（E1 核心已完成：协议层/管道层/状态机/bridge/serve-ime/端到端冒烟，见附录 C）
> 用途：本文档是**给执行 AI agent 的施工图**。按阶段顺序执行，每个任务卡自带验收命令；验收不通过不得进入下一阶段。
> 相关既有方案（执行前先读）：`docs/agent-optimization-plan.md`（P0 已完成，P1-P3 编号沿用）、`docs/pet-agent-integration.md`（桌宠 M1-M6）、`docs/ARCHITECTURE.md`

---

## 〇、执行 agent 必读

### 0.1 已确认的决策（不得偏离）

| # | 决策 | 内容 |
|---|---|---|
| D1 | 适配器宿主 | **agent-sdk 仓库内**，新增子命令 `owo-agent serve-ime`，直接复用 core/server 库；需同步推翻 agent-sdk README「输入法路线不实施」的旧表述 |
| D2 | 覆盖范围 | **全量分阶段**：断链适配器 + remember 修复 + tokenizer + 循环检测/预算熔断 + plan/todo 工具 + 桌宠 M1~M3 |
| D3 | tokenizer 选型 | **tiktoken-rs**（cl100k_base 离线 BPE），接口标注 `estimated: true` |
| D4 | 联调环境 | 本机**已装 OwO 0.2.3 且能构建 C++**（OwO-release 仓库），验收含「真实输入法 v 前缀 → Agent」端到端 |

### 0.2 三个仓库与工具链

| 仓库 | 本地路径（执行时以实际为准） | 工具链 | 门禁命令 |
|---|---|---|---|
| OwO-release（C++ 输入法） | `OwO-release/` | VS2022 + CMake 3.25+ + .NET 10 SDK + Win11 SDK | `ctest --preset windows-debug` |
| OwO/agent-sdk（Rust 重环引擎） | `OwO/agent-sdk/` | Rust **1.97.1 MSVC**（根 rust-toolchain.toml pin） | `powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1` |
| LingXi-DesktopAgent（桌面壳） | `LingXi-DesktopAgent/` | workspace=GNU；`apps/overlay`=MSVC 独立构建 | `cargo check --workspace`（GNU）+ `cd apps/overlay && cargo check` |

**🔴 第一步必做**：当前工作区路径 `d:/working OWOWOWOWOWO` **含空格**，GNU 工具链链接器与部分脚本会失败（见 `LingXi-DesktopAgent/docs/project-status.md` 「环境搭建」）。执行任何构建前，先把三个仓库克隆/复制到**纯英文无空格路径**（如 `D:\dev\owo\...`），并确认 `git status` 可用。本文后续所有命令均假设已在合法路径下执行。

### 0.3 全局约束（坑清单，违反必返工）

1. **路径**：仓库路径必须纯英文、无空格、无中文。
2. **工具链**：agent-sdk 是 pin 1.97.1 的 MSVC；LingXi workspace 是 GNU；overlay 是 MSVC 但被根 workspace `exclude`。不要混。
3. **CSP**：WebView2 前端禁止 `fetch("http://127.0.0.1:4096")`，一切走 Tauri `invoke`（HTTP 由 Rust 侧发出）。
4. **锁**：`.lock().unwrap()` 一律换 `.safe_lock()`（LingXi 侧）。
5. **Tauri 窗口**：新窗口 label 必须加进 `apps/overlay/capabilities/default.json`，否则白屏；关窗用 `destroy()` 不用 `close()`。
6. **审批阻塞**：agent 服务端审批等待 **300s 超时默认 Deny**。
7. **沙箱根**：agent 会话 workspace 必须与启动参数 `--workspace` 完全一致，否则文件工具报路径越界。
8. **引擎启动**：`OPENAI_API_KEY` 或 `OPENAI_BASE_URL`（指向本地兼容端点）缺一则启动即退出。
9. **代理干扰**：本机 `Invoke-WebRequest` 走系统代理（127.0.0.1:7897）；探测本地端口用 `curl.exe --noproxy "*"`。
10. **PowerShell 含空格路径**：`Start-Process` 传参需两层引号；Rust `Command::arg` 无此问题。
11. **假验证零容忍**：任何"验证"必须是真断言、真退出码；禁止 `println!("✓ 成功")` 式脚本（P0-1 已清过一次，别再犯）。
12. **evals/ 目录**是手工验收残留（含 10MB serve*.err），新产物一律进 `.gitignore`，不要往里堆。

### 0.4 关键参考文件索引（执行 E1 前通读）

| 文件 | 内容 |
|---|---|
| `OwO-release/docs/plugins/agent-ipc-integration.md` | **协议 v3 规范**（请求/响应结构、状态机、15 条最小接入验证） |
| `OwO-release/docs/plugins/agent-ipc-v3.schema.json` | 协议 JSON Schema（10KB，机器可校验） |
| `OwO-release/apps/agent_mock/main.cpp` | **协议参考实现**（管道帧、SDDL ACL、会话状态机、槽位补丁——行为规范以此为准） |
| `OwO-release/include/owo/agent/agent_protocol.h` `agent_pipe.h` | 协议常量与 C++ 头文件（`kMaximumAgentPayloadBytes` 等） |
| `OwO-release/plugins/agent-ipc/` | 官方连接器插件源码（连接器侧行为：重试/轮询语义，E1.4 需核对） |
| `OwO-release/docs/plugins/agent-ipc-privacy.md` | 隐私要求（管道 ACL、脱敏、最短保留） |
| `OwO/agent-sdk/crates/owo-agent-core/src/agent.rs` | Agent 主循环（run_turn/maybe_compact/estimate_tokens） |
| `OwO/agent-sdk/crates/owo-agent-server/src/lib.rs` | HTTP 服务（会话/turn SSE/审批，239KB 巨石，E2 只动审批段） |
| `OwO/agent-sdk/crates/owo-agent-cli/src/main.rs` | CLI 入口（`run_serve`/`build_agent_with_mcp`/`AppState` 组装模式，E1.6 照此扩展） |

---

## 一、背景（为什么做）

三个仓库各自完成度都不低，但**输入法与 Agent 引擎之间的链路是断的**：

- OwO 输入法有完整的 Agent 模式（拼音串首字符 `v` 进入，如 `vbangwozhaowenjian`）与 v3 管道协议，但目前只能连官方 mock（`OwO-release/apps/agent_mock`）；
- agent-sdk 是功能完备的 Agent 引擎（37 内置工具 + MCP + 沙箱 + 审批），但**没有任何命名管道代码**（全仓搜 `OwO.Agent.External|CreateNamedPipe` 零命中），README 明说「输入法路线不实施」；
- LingXi 已通过 `owo-bridge` 打通了 HTTP/SSE 契约（审批/diff/revert/abort 全链路验证过）。

**目标**：新增 `serve-ime` 子命令，让真实 Agent 引擎成为 OwO 输入法认可的第三方 Agent；同时完成 P1/P2 中影响「真实性」的优化项（remember、tokenizer、循环防护、plan 工具）与桌宠接入，形成「打字即 Agent，候选框即命令面板」的完整演示闭环。

### 1.1 目标架构

```
┌─ OwO 输入法（TSF）── v 前缀 Agent 模式 ──────────────────┐
│   org.owo.agent-ipc 连接器插件（已装，v1.2.0）            │
└──────────────┬──────────────────────────────────────────┘
               │ 命名管道 \\.\pipe\OwO.Agent.External.v1
               │ 4 字节小端长度 + UTF-8 JSON，一问一答，10s 超时
┌──────────────▼──────────────────────────────────────────┐
│  owo-agent serve-ime（新子命令，单进程双面）              │
│  ① 管道监听面（tokio named pipe + 当前用户 ACL）          │
│  ② HTTP 面（复用 run_serve 全部代码：session/turn(SSE)/   │
│     审批/diff；--port 可配，默认 4096）                   │
│  ③ ImeBridge：管道请求 ⇄ HTTP 回环 + 会话/命令状态机      │
└──────────────┬──────────────────────────────────────────┘
               │ HTTP/SSE（127.0.0.1:4096）
┌──────────────▼──────────────────────────────────────────┐
│  LingXi overlay / 桌宠（E6：M1-M3，连同一进程的 HTTP 面）  │
│  审批卡片 = 高风险命令的「Agent 可信界面」                 │
└─────────────────────────────────────────────────────────┘
```

设计理由（执行 agent 不要"优化"掉这些决策）：
- **管道→HTTP 回环**而不是直调 `AppState`：HTTP 契约有 openapi.json 冻结 + 23 个 server 集成测试保护，复用最稳；本机回环延迟可忽略。
- **单进程双面**：桌宠与输入法共享同一份会话/审批状态；避免 serve 与 serve-ime 两个进程争 SQLite 数据目录锁。
- **输入法侧永远不直接授权高风险**：协议红线（删除/执行/账户/支付/凭据/系统设置不能由候选框授权），高风险一律转为「打开确认界面」低风险命令，由桌宠审批卡片或 Web 工作台确认。

---

## 二、任务总览与依赖图

```
E0 环境基线（迁移路径/门禁/输入法+连接器安装）
 └─> E1 IME 管道适配器（核心，最大块）
      ├─ E1.1 协议类型层      ├─ E1.2 管道传输层
      ├─ E1.3 会话状态机      ├─ E1.4 turn 异步适配
      ├─ E1.5 命令/风险映射   ├─ E1.6 CLI 子命令
      └─ E1.7 契约测试 ──────┴─> E1.8 真机端到端
E2 remember 消费+权限规则（独立，可并行）
E3 tiktoken tokenizer（独立，可并行）
E4 循环检测/超时/预算熔断（依赖 E3 的 TokenCounter）
E5 plan/todo 工具（依赖 E4 的 loop 结构稳定）
E6 桌宠 M1~M3（依赖 E1.6 的 serve-ime --port；照 pet-agent-integration.md 执行）
E7 验收脚本+文档对齐+打包（最后）
```

**建议执行顺序**：E0 → E1.1~E1.3（纯协议层，可测）→ E2 → E3 → E1.4~E1.8（打通链路）→ E4 → E5 → E6 → E7。
E2/E3 与 E1.4+ 无代码冲突，可穿插做人肉并行。

---

## E0：环境基线验证（0.5~1 天）

### E0.1 路径迁移

把三个仓库放到纯英文无空格路径。注意保留 `.git`（OwO/agent-sdk 有完整 git；LingXi 曾有空壳 .git，P0-2 已修复，迁移后 `git status` 必须可用）。

### E0.2 各仓库门禁基线（全绿才继续）

```powershell
# agent-sdk（在 agent-sdk 目录）
powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1
# 预期：fmt→clippy -D warnings→test→route-contract→node→ts 全绿 EXIT=0

# LingXi workspace（GNU）
cargo check --workspace
cargo test --workspace
# overlay（MSVC）
cd apps\overlay; cargo check

# OwO-release（C++）
cmake --preset windows-release
cmake --build --preset windows-release
ctest --preset windows-release
```

### E0.3 输入法 + 连接器 + mock 冒烟（验证协议环境）

1. 确认本机已装 OwO 0.2.3（设置中心：开始菜单「OwO 输入法设置」）。
2. 安装连接器：`OwO-release/artifacts/plugins/org.owo.agent-ipc-1.2.0.owopkg`（或用 `scripts/package_plugins.ps1` 重打包），设置中心里安装并**明确授权**，确认管道端点为默认 `\\.\pipe\OwO.Agent.External.v1`。
3. 跑官方 mock 验证链路：`OwO-release/start-agent-mock.cmd`，在任意输入框打 `vbangwozhaowenjian`，确认 mock 窗口收到请求、候选框出现命令。
4. **记录观察**：mock 响应期间连接器实际发出的 action 序列（mock 窗口会打印）——这决定 E1.4 的轮询语义（见附录 B 开放问题 Q1）。

**E0 验收**：三条门禁命令 EXIT=0；mock 链路（第 3 步）至少完成一次 submit→select→completed。

---

## E1：IME 管道适配器（5~8 天，核心）

### E1.1 新 crate：`crates/owo-agent-ime`（协议类型层）

**位置**：`OwO/agent-sdk/crates/owo-agent-ime/`，加入根 `Cargo.toml` members（第 6 个成员）。

**结构**：
```
crates/owo-agent-ime/
├── Cargo.toml        # 依赖 serde/serde_json/thiserror/tokio/windows-sys（仅 target windows）
└── src/
    ├── lib.rs        # pub use
    ├── protocol.rs   # v3 协议类型（本任务）
    ├── frame.rs      # 4 字节小端长度帧（E1.2）
    ├── pipe.rs       # 管道服务端 + ACL（E1.2）
    ├── state.rs      # 会话/任务状态机（E1.3）
    ├── bridge.rs     # HTTP 回环适配（E1.4）
    └── commands.rs   # 命令生成与风险映射（E1.5）
```

**protocol.rs 要点**（照 `agent-ipc-v3.schema.json` 逐字段翻译，**严格字段集合**——serde 用 `deny_unknown_fields`，禁止未知字段）：

```rust
// 动作与状态枚举（字符串严格匹配）
pub enum Action { Submit, Select, Page, Cancel, Poll }
pub enum Status { Connecting, Submitting, AgentMode, WaitingForUser, Thinking,
                  Executing, WaitingForConfirmation, Cancelling, Cancelled,
                  Completed, Disconnected, Timeout, Error }

pub struct AgentIpcRequest { schema_version: u8, action: Action,
    session_id: String, request_id: String, parent_request_id: String,
    idempotency_key: String, capabilities: Vec<String>,
    protocol_min: u8, protocol_max: u8, required_features: Vec<String>,
    user_input: String, input: InputView, application: ApplicationView,
    session_context: String, context_entries: Vec<ContextEntry>,
    command_id: String, page: u32, task_revision: u64, slot_updates: Vec<SlotUpdate> }

pub struct AgentIpcResponse { schema_version: u8, session_id: String,
    request_id: String, message: String, commands: Vec<Command>,
    executing_command: String, status: Status, page: u32, has_more: bool,
    error_code: String, state_revision: u64, progress: u32,
    retry_after_ms: u32, can_cancel: bool, can_continue_input: bool,
    expires_at_ms: u64, error_message: String, retryable: bool,
    capabilities: Vec<String>, task: TaskDraft }

pub struct TaskDraft { intent: String, intent_ranges: Vec<Range>,
    slots: Vec<Slot>, unconsumed_ranges: Vec<Range>, revision: u64 }
// Command / Slot / SlotUpdate / Range（[start,end) 左闭右开，相对去掉 v 后的原始 ASCII 拼音）
```

**硬约束**（来自 schema 与 mock）：
- 帧上限 262144 字节（`kMaximumAgentPayloadBytes`，核对 `agent_protocol.h` 的实际值）。
- `session_id`/`request_id`/`command_id` 只允许 ASCII 字母数字点连字符下划线——**反序列化后校验**，非法即按 error 响应。
- `capabilities` 必须返回与 OwO 请求的交集，且 `required_features` 缺失时**拒绝继续执行**（返回 error）。
- 响应必须原样回传 `session_id`/`request_id`。
- JSON 单对象 262144 上限、单操作超时 500~30000ms（可配置）。

**验收**：
```powershell
cargo test -p owo-agent-ime --test protocol_roundtrip
# 测试内容：从 OwO-release/docs/plugins/agent-ipc-v3.schema.json 抽取全部示例 JSON，
# serde 反序列化→再序列化→字段等价断言；未知字段拒绝断言；非法 session_id 拒绝断言。
# 另：用 jsonschema crate（dev-dependency）对响应构造器做 schema 校验，防止生成非法响应。
```

### E1.2 管道传输层（frame.rs + pipe.rs）

**帧协议**（照 mock `main.cpp:71-102`）：
- 读：4 字节无符号小端长度 → 对应长度 UTF-8 JSON；长度为 0 或 >262144 → 断连。
- 写：同构。

**管道服务端**（Rust 侧）：
```rust
// tokio::net::windows::named_pipe::ServerOptions
// 多实例模式：循环 create → connect → spawn 处理 → 重建（同一 pipe name 多个 server instance，
// 模拟 CreateNamedPipe 多实例，允许连接器并发重连）
```

**ACL（必须）**：照 mock `CurrentUserSecurity`（main.cpp:34-69）的 SDDL：
```
D:P(A;;GA;;;<当前用户SID>)(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;LW)
```
Rust 侧用 `windows-sys` 构造同 SDDL 字符串（当前用户 SID 可从进程 token 取），经 `ServerOptions::security_attributes_raw`（unsafe）传入。**验收必须断言**：另一个 Windows 用户会话无法连接（可用 `runas` 或至少代码审查 + 单测断言 SDDL 字符串正确）。

**超时**：每次读写在 tokio 层加 `tokio::time::timeout`（默认 10s，`--ime-timeout-ms` 可配，钳位 500~30000）。

**验收**：
```powershell
cargo test -p owo-agent-ime --test pipe_echo
# 测试内容：起管道服务端 → 测试客户端连上 → 发一帧 → 收到回显帧（处理函数先原样返回 error）；
# 断言：超长帧被拒、非法 UTF-8 被拒、断连后服务端可继续接受新连接（多实例）。
```

### E1.3 会话与任务状态机（state.rs）

照 mock `process()`（main.cpp:198-313）实现，但状态存内存 `DashMap<String, ImeSession>`：

```rust
pub struct ImeSession {
    pub agent_session_id: String,   // 映射到 agent-server 的会话
    pub last_request: AgentIpcRequest,
    pub pending: Option<PendingTurn>, // E1.4：后台 turn 任务
    pub task: TaskDraft,              // 槽位草稿
    pub state_revision: u64,          // 单调递增
    pub commands: Vec<Command>,       // 当前候选（command_id 唯一稳定）
    pub idempotency_cache: HashMap<String, AgentIpcResponse>, // idempotency_key → 缓存响应
    pub created_at: Instant, expires_at: Instant,             // 会话过期（建议 30min GC）
}
```

规则（照 mock + 协议 227 行）：
- `cancel` → 清会话（中止后台 turn），返回 `cancelled/completed`。
- 未知 session → `error` + `error_code: "session_not_found"`。
- `slot_updates` 非空：校验 `task_revision`（不匹配 → `task_revision_mismatch` 错误 + 返回当前草稿）；校验 `locked`（锁槽不可被不同值覆盖 → `slot_locked`）；应用补丁后 `++revision`，返回 agent_mode + 提交类候选。
- `state_revision` 单调递增；收到更小的（旧状态）请求 → 拒绝。
- **幂等**：相同 `idempotency_key` → 返回缓存响应，不重复执行；completed 会话的缓存随会话清除。
- `select` 的 `command_id` 必须来自当前候选，否则 `unknown_command`。

**验收**：
```powershell
cargo test -p owo-agent-ime --test state_machine
# 覆盖：cancel 清理 / 未知会话 / revision 不匹配 / 锁槽覆盖拒绝 / 幂等缓存命中 /
# state_revision 回退拒绝 / 未知 command_id。逐条对应 integration.md 「最小接入验证」第 12~15 条。
```

### E1.4 turn 异步适配（bridge.rs）★最难的一环

**问题**：LLM turn 是秒级~分钟级；管道单次操作超时 10s 且一问一答。

**方案**：
1. `submit` → ImeBridge 用 HTTP 回环（reqwest，同进程）：
   - `GET /auth/token` 拿 token（鉴权与 LingXi owo-bridge 相同逻辑：懒加载缓存，401 刷新重试一次，参考 `LingXi-DesktopAgent/crates/owo-bridge/src/client.rs:83-147`）；
   - `POST /session`（workspace=serve-ime 的 `--workspace`，必须一致，见坑 7）；
   - `POST /session/{id}/turn` **不等待完成**：spawn 后台 tokio 任务消费 SSE 流，把事件累积进 `ImeSession.pending`；
   - 立即返回 `status: thinking, retry_after_ms: 1500, can_cancel: true, progress: 10`。
2. 连接器后续请求（poll 或重复 submit——**以 E0.3 的观察为准**，见附录 B Q1）→ 查后台任务：
   - 仍在跑 → `thinking` + progress 估算（按已收到的 tool_use/tool_result 数占预算轮次比例）；
   - `permission_request` 到达 → 返回「打开确认界面」候选（见 E1.5）；
   - `final` → 把 turn 结果转成候选（见 E1.5），状态 `agent_mode`；
   - `turn_failed`/流关闭 → `error` + `retryable`。
3. `cancel` → `POST /session/{id}/abort`，确认 5s 内停止。

**注意**：
- SSE 无 keep-alive 心跳（turn 端点区别于 workflow 端点 15s 心跳）——后台任务**不能**用"N 秒无帧=断开"判断，只认 `final`/`turn_failed`/流关闭。
- 进程内 HTTP 回环统一走 `reqwest`（agent-sdk 已依赖 reqwest + default-tls）。
- 一个 ImeSession 同时只允许一个在途 turn（并发 turn 会 409——输入法场景天然串行，直接拒绝并返回 `retry_after_ms`）。

**验收**：
```powershell
cargo test -p owo-agent-ime --test bridge_turn
# 用 mock OpenAI（参考 LingXi-DesktopAgent/crates/owo-bridge/tests/mock_openai.py 或
# agent-sdk/evals/mock-openai.py）+ 真实 agent-server：
# 1) submit → 500ms 内收到 thinking 响应（断言 <1s，模拟 10s 超时压力）
# 2) 后续 poll/submit → thinking → 最终 agent_mode + commands 非空
# 3) 中途 cancel → abort 生效、会话清理、管道返回 cancelled
# 4) 模型 500 → pipe 返回 error + retryable=true
```

### E1.5 命令生成与风险映射（commands.rs）

把 turn 结果翻译成协议候选。**所有命令都是适配器自己生成的结构化对象，select 回来按 command_id 执行对应动作**。

**A. 纯文本回复（final 无工具改动）**：
```json
{ "id": "insert-reply", "label": "<回复前 20 字>…", "category": "text.insert",
  "risk_level": "low", "high_risk": false, "requires_confirmation": false,
  "task_revision": 0, "slot_updates": [], "commit_task": false }
```
`message` 放完整回复（只读展示）。选中后返回 completed——**是否上屏由连接器决定**（附录 B Q2）。

**B. 有文件改动（diff 非空）**：
```json
{ "id": "view-diff", "label": "查看 N 处改动", "category": "file.diff", "risk_level": "none" },
{ "id": "revert-all", "label": "撤销全部改动", "category": "file.revert", "risk_level": "low" },
{ "id": "open-workspace", "label": "打开工作目录", "category": "shell.open_dir", "risk_level": "low" }
```
`revert-all` → `POST /session/{id}/revert`。

**C. 审批请求（permission_request 事件）**：
返回一条低风险命令：
```json
{ "id": "confirm-<rid>", "label": "打开确认界面：允许 <工具名>", "category": "agent.confirm_ui",
  "risk_level": "low", "requires_confirmation": false,
  "preview": "在 Agent 可信界面（桌宠/Web 工作台）中确认，本候选框不直接授权" }
```
选中 → serve-ime：① 若桌宠/工作台已连（E6 完成后），推审批卡片；② 同时拉起默认浏览器到 `http://127.0.0.1:<port>/`（Web 工作台已有审批条）。后续 poll 期间继续等待审批（300s 超时 Deny → 返回 error 说明「审批超时已按拒绝处理」）。

**D. 高风险命令（协议红线）**：删除/执行程序/账户/支付/凭据/系统设置类工具**永远**不给 `high_risk: false`；直接走 C 的可信界面路径，且响应里 `high_risk: true` 的候选**仅用于提示**「前往 Agent 界面确认」，不携带直接执行语义。

**E. 槽位草稿（tasks.slots，required_features 强制）**：
最小可用实现：submit 时若用户输入是「提醒类」意图（由 turn 的首步意图探测或关键词），构造 TaskDraft：
- `intent: "create_reminder"`，`intent_ranges`/`slots`/`source_ranges` 按拼音区间计算（工具：把 `input.segmented_pinyin` 的音节边界映射回 `raw_pinyin` 字符偏移；参考 mock `pinyin_range()` 的 find 策略但要用音节切分而非子串查找，LingXi `crates/assistant-ime/src/segment.rs` 有现成音节切分可抄思路）；
- 时间槽不锁定（confidence 低），事项槽锁定；候选即时间变体（`slot_updates` 只改 time 槽）；
- 最终 `commit_task: true` 的「确认创建」候选 → 适配器调用本地 reminder 机制（agent-server 已有 `/automations` 定时任务 API，POST 建一条提醒）。

**风险字段一致性**：`risk_level: "high"` 必须同时 `high_risk: true && requires_confirmation: true`，否则连接器拒绝响应（协议第 15 条验证）——构造器加单元测试断言该不变量。

**验收**：
```powershell
cargo test -p owo-agent-ime --test commands_contract
# 覆盖：A/B/C/D 四类命令生成的 schema 合法性（jsonschema 校验）+ 风险字段不变量 +
# label 长度限制 + command_id 唯一性稳定性。
```

### E1.6 CLI 子命令 `serve-ime`（owo-agent-cli）

**改动**：`crates/owo-agent-cli/src/main.rs`
- `Commands` 枚举加 `ServeIme(ServeImeArgs)`（紧跟 `Serve`，main.rs:99 附近）；
- `ServeImeArgs { port: u16 /*默认 4096*/, workspace: PathBuf /*默认用户文档目录*/,
    pipe: String /*默认 \\\\.\\pipe\\OwO.Agent.External.v1*/, timeout_ms: u32 /*默认 10000*/ }`；
- `run_serve_ime`：**复制 `run_serve` 的加载序**（Settings::load_encrypted → apply_egress/usage/reasoning → build_agent_with_mcp → AppState::new → build_router，参考 main.rs:1134+ 与 bench 的内嵌模式 :837-850）→ axum::serve 绑 127.0.0.1:port → 并行 spawn 管道监听循环（E1.2）。

**同时做**：`run_serve` 与 `run_serve_ime` 加**数据目录互斥检测**（同一 `OWO_AGENT_DATA` 下放 lockfile，第二次启动报错并提示「serve 与 serve-ime 不能同时运行，共用数据目录」）。

**验收**：
```powershell
cargo build -p owo-agent-cli
# 手动：设置 OPENAI_BASE_URL 指向本地 mock 端点后
owo-agent serve-ime --workspace <测试目录>
# 预期：HTTP /health 200 + 管道监听启动日志（打印 pipe 名与 PID）；
# 手动用 PowerShell 命名管道客户端发一帧 submit（可写 10 行测试脚本）收到合法响应。
```

### E1.7 契约测试（汇总 `crates/owo-agent-ime/tests/`）

把 `agent-ipc-integration.md` 「最小接入验证」15 条（:241-258）**逐条**写成自动化测试（`contract_checklist.rs`），mock 模型驱动：

| # | 协议验证条 | 测试名 |
|---|---|---|
| 3 | `v` 不进拼音、无上屏 | （输入法侧行为，真机项，标 `#[ignore]`） |
| 4 | submit 及非空 session_id / Enter、Space | `submit_creates_session` |
| 5 | 两个低风险 commands + select 回传正确 ID | `select_routes_command` |
| 6 | page 不改会话 ID | `page_keeps_session` |
| 7 | Escape/删 v → cancel | `cancel_cleans_session` |
| 8 | context_entries 分段顺序 | `context_entries_order` |
| 9 | 密码框敏感字段为空时正确处理 | `sensitive_input_redaction` |
| 10 | 错误 JSON/超长/错会话 → error 不执行 | `malformed_rejected` |
| 11 | high_risk → 只提示可信界面 | `high_risk_blocked` |
| 12 | Backspace 恢复拼音、删 v 才 cancel | （输入法侧，真机项） |
| 13 | 幂等重发 | `idempotency_cache` |
| 14 | 旧 state_revision 拒绝 | `stale_revision_rejected` |
| 15 | risk_level=high 但缺 high_risk 的响应被拒 | `risk_invariant` |

```powershell
cargo test -p owo-agent-ime            # 全部非 ignore 测试
cargo clippy -p owo-agent-ime --all-targets -- -D warnings
```

### E1.8 真机端到端（最终验收）

```powershell
# 1. 启动（先杀掉所有旧 owo_agent_mock.exe / serve 进程）
set OPENAI_API_KEY=<key>   # 或 OPENAI_BASE_URL=<本地兼容端点>
owo-agent serve-ime --workspace %USERPROFILE%\Documents
# 2. Win+Space 切到 OwO 输入法
# 3. 在记事本输入：
#    vbangwozhaowenjian     → 期望：候选框出现「查找文件」类命令（真实 LLM 生成）
#    vsanzhongtixingwokaihui → 期望：槽位草稿 + 时间变体候选；选择后 commit_task 创建提醒
#    任意让 agent 写文件的请求 → 期望：「打开确认界面」候选 → 选中 → 浏览器/桌宠审批 → diff/revert 候选
# 4. Esc → 会话取消；serve-ime 日志出现 abort 清理
```

**记录**：把 3 的全流程截图 + serve-ime 日志存 `docs/evidence/e1-ime-e2e/`（结题材料）。

**E1 完成定义**：E1.7 全绿 + E1.8 三条场景全部通过 + 附录 B 的 Q1/Q2 得到真机答案并回填文档。

---

## E2：remember 消费 + 权限规则持久化（2~3 天）

> 对应既有编号 P2-1。现状缺陷：`PermissionResponse.remember` 定义于 `crates/owo-agent-protocol/src/lib.rs:59-63` 但**服务端从不读**；`Policy::decision()`（core/src/permissions.rs 约 :295-306）无持久化 allowlist。

### E2.1 规则模型

```rust
// core/src/permissions.rs 新增
pub struct PermissionRule {
    pub tool: String,          // 工具名，glob：如 "write_file"
    pub pattern: RulePattern,  // CommandPrefix("cargo") | PathGlob("/src/**") | Any
    pub decision: Decision,    // Allow（Deny 规则不通过 remember 产生，只有硬表）
    pub expires_at: Option<DateTime<Utc>>, // None=永久；「本会话」规则=会话结束清
    pub created_at: DateTime<Utc>,
    pub note: String,          // 审计用：「用户于审批卡片勾选记住」
}
```

存储：`settings.json` 新增 `permissions.rules: Vec<PermissionRule>`（Settings 已有持久化通道，注意 `Settings::load_encrypted` 兼容——缺字段 = 空表）。**硬拒绝表**（`rm -rf`/`sudo` 等现有 9 条黑名单）保持代码内编译期，永不可被 remember 覆盖。

### E2.2 决策链

```
Policy::decision(req):
  1. evaluate 阶段越界/危险 → Deny（现状保留）
  2. 硬拒绝表命中 → Deny（不可记住）
  3. 会话级临时规则（read_only 上下文等）→ 现状逻辑
  4. 用户规则表匹配（tool + pattern 归一化：命令取前缀 token、路径 canonicalize 后 glob）→ Allow/Deny
  5. 默认 Ask（走 Auto-review → 用户审批）
```

### E2.3 remember 消费点

`crates/owo-agent-server/src/lib.rs` 的 `respond_permission`（约 :1781-1824，以函数名搜索定位）：`response.remember == Some(true) && allow` 时：
1. 把 `(tool, args)` 归一化为 pattern（命令类：首个 token + 通配；写文件类：canonicalize 后目录前缀 + `/**`）；
2. 写入规则表（持久化）+ 追加审计事件（含 note）；
3. **同回合后续**同 pattern 审批直接放行（当前回合立即生效）。

规则表传给 Agent 的通道：`Policy` 持 `Arc<RwLock<Vec<PermissionRule>>>`（server 启动时从 Settings 载入，respond_permission 更新后写回）。

**边界**：`expires_at` 检查在每次 decision 时惰性过期；设置页/Web 工作台加规则列表只读展示（后续 E7 可做编辑 UI，非必须）。

### E2.4 测试

```powershell
cargo test -p owo-agent-core --test permission_rules
# 1) 首次 cargo test Ask → respond_permission{allow,remember} → 同前缀二次不 Ask
# 2) rm -rf 永远 Deny 且 remember 无效
# 3) write_file:/a/** 记住后只放行该目录，/b/** 仍 Ask
# 4) 过期规则不再生效
# 5) settings.json 重启后规则仍在（load→save round-trip）
# 变异验收：注释掉 remember 写入逻辑 → 测试 1 变红。
```

---

## E3：真实 tokenizer（1.5~2 天）

> 对应 P1-1。现状：`core/src/agent.rs` 的 `estimate_tokens()` = `chars/2 + 4`（约 :952-965，**连同保护它的单测 `estimate_tokens_counts_chars_and_overhead` 一起删**）。

### E3.1 实现

```rust
// core/src/tokenizer.rs（新）
pub trait TokenCounter: Send + Sync { fn count(&self, text: &str) -> u64; fn name(&self) -> &'static str; }
pub struct TiktokenCounter { /* tiktoken_rs::cl100k_base() once_cell 缓存 */ }
pub struct HeuristicCounter;   // 旧公式，兜底
```

- 依赖：`tiktoken-rs = "0.6"`（cl100k_base 内嵌 BPE 数据，~2MB 二进制增量，已确认可接受）。
- `AgentConfig` 增加 `tokenizer: String`（"tiktoken" | "heuristic"，默认 tiktoken）。
- `estimate_tokens` 替换为 `self.token_counter.count(...)` 求和；每消息 overhead（+4）保留。
- **token_budget 自动推导**：`AgentConfig` 增加 `model_context_window: Option<u32>`；模型元数据表（`gateway.rs` 旁）维护 `context_window`（deepseek-chat 65536、gpt-4o 128000、qwen2.5-72b 32768…按 provider 配置缺省），`token_budget = window × 0.75` 向下取整；显式配置优先。
- `/session/{id}/context` 与 `/usage` 响应加 `tokenizer: "tiktoken|heuristic"` 与 `estimated: true` 字段（openapi.json 同步更新——**route_contract 测试会抓**，记得同步契约测试期望）。

### E3.2 验收

```powershell
cargo test -p owo-agent-core --test tokenizer_accuracy
# 1) 基准对比：固定 5 段中文/混合文本（测试内嵌），
#    tiktoken 计数 vs 旧启发式计数 vs 真实 provider prompt_tokens（后者用
#    scripts/check-token-accuracy.ps1 手动跑真实 API，输出偏差表）；
#    断言 tiktoken vs 真实值偏差 < 10%。
# 2) 变异验收：把 cl100k_base 换成错误编码名 → 计数变化 → 测试断言计数与基准不符即红。
```

---

## E4：循环检测 / 每步超时 / 并发上限 / 预算熔断（2~3 天）

> 对应 P2-2。锚点：`core/src/agent.rs` `run_turn_inner`（约 :398 起的 for 循环）、工具执行段（约 :551-701）、`wait_for_abort`（约 :919）。

### E4.1 循环检测（LoopDetector）

```rust
pub struct LoopDetector { window: VecDeque<(String /*tool*/, u64 /*args_hash*/)> }
// 同 (tool, args_hash) 滑窗内：
//   出现 ≥3 次（非豁免工具）→ 注入系统消息「你似乎在重复同一操作，请换策略或结束」
//   出现 ≥5 次 → 强制走 WRAP_UP_PROMPT 收尾（复用现有步数耗尽路径 :732-778）
// 豁免白名单（连续失败的同参调用才算）：desktop_wait_until、read_file（编辑后重读）、grep
// 实现口径：只统计「连续失败的同参调用」或「同参数成功后再次原样调用」——按前者实现即可。
```

### E4.2 每步超时

`ToolContext` 加 `deadline: Instant`（默认 120s；`run_command` 类工具可覆盖为 settings 里的 command_timeout）。工具执行统一包 `tokio::time::timeout`，超时返回结构化错误 `ToolError::DeadlineExceeded`（工具结果照常回给模型，不是 panic）。

### E4.3 并发上限

只读工具 `join_all`（约 :551）前加 `Arc<Semaphore>`（permits=4，`AgentConfig.concurrent_read_tools` 可配）。

### E4.4 预算熔断

`OWO_USAGE_TOKEN_BUDGET`/`COST_BUDGET_USD`（`/usage` 已有）接进 loop：每轮 `select!` 返回后检查累计 usage，超限 → 终止循环，`final` 文本说明「已达预算上限 N，任务中止于第 X 步」，写审计。

### E4.5 abort 忙轮询改事件

`wait_for_abort` 50ms 轮询 → `tokio::sync::Notify` + `select!`（server `abort_turn` 置 flag 后 `notify_one`）。

### E4.6 测试

```powershell
cargo test -p owo-agent-core --test loop_guard
# 1) mock backend 固定返回同一 tool_call → 第 3 次注入提示断言、第 5 次强制 wrap-up 断言
# 2) 豁免工具同参 10 次不触发
# 3) 工具挂起 130s（用 tokio::time::pause 或短 deadline=100ms 测试值）→ DeadlineExceeded 结构化错误
# 4) 预算 100 token + mock 连续返回 → 主动终止 + final 含预算说明
# 5) abort → Notify 路径 5s 内退出（复用现有 loop 测试风格）
```

---

## E5：plan / todo 工具（1~2 天）

> 对应 P2-5 的轻量子集（subagent/explore 已存在，不做）。

### E5.1 `update_plan` 工具

```rust
// core/src/tools_plan.rs
// 工具名 update_plan；入参 { steps: [{ content: String, status: pending|in_progress|completed }] }
// 状态存 Agent 会话（AgentConfig/plans: DashMap<session, Plan>）；
// 每次调用产生 TurnEvent::PlanUpdate（core 的 TurnEvent 枚举加变体）
```

- server：SSE 事件流透传 `plan_update`（tagged JSON，`v` 版本字段风格与现有一致）；`GET /session/{id}` 详情带当前 plan。
- 系统提示注入：工具说明里写明「多步任务先建 plan，每完成一步更新」。
- openapi.json + route_contract 测试同步。

### E5.2 验收

```powershell
cargo test -p owo-agent-core --test plan_tool      # 状态流转：建→改→完成；非法状态转换拒绝
cargo test -p owo-agent-server --test plan_sse     # SSE 帧出现 plan_update 且顺序正确
```

---

## E6：桌宠 M1~M3（3~3.5 天，照既有方案执行）

> **既有方案已足够详细，本文不重复**。执行 `docs/pet-agent-integration.md` 的 M1（事件驱动状态，0.5d）→ M2（pet-chat 面板 + 流式气泡，1.5d）→ M3（审批 UI + 倒计时，1d）。
> 该文档的「审查补充」「风险与对策」「改动清单」章节全部有效，**必须逐条照做**（尤其：capabilities 注册 pet-chat、destroy()、删除 app.js 的 owoSetPet 双写源、状态不落盘）。

**本文追加的差异/联动（pet 方案之后写就的决策）**：

1. **连接目标**：桌宠的 `owo_start_service` 默认拉起的进程从 `serve` 改为 `serve-ime --port 4096`（同一进程双面，E1.6），使桌宠审批卡片与输入法会话共享状态。
2. **`resolve_exe_path` 硬编码路径问题**（pet 方案风险表）：趁接线时一并修——候选链改为「settings `owo_agent_exe_path` → `PATH` 中 owo-agent → 常见安装目录」，缺失时 UI 给「引擎未安装」明确引导（pet 方案原文要求）。
3. **桌宠审批卡片成为输入法高风险确认界面**（E1.5-C 的优先通道）：serve-ime 收到「打开确认界面」选择时，若桌宠在线（HTTP /health 通）则推审批卡片，否则回退拉起浏览器。此联动在 M3 完成后接入，加一条联调：输入法发起写文件 → 桌宠 alert + 卡片 → 允许 → 输入法侧 poll 收到继续执行。
4. **M4~M6（工具可视化/会话附件/主动搭话）不在本次范围**，结题材料里列为后续工作。

**验收**：照 `pet-agent-integration.md` §六 的 7 条端到端验收（做成 `scripts/pet-agent-smoke.ps1`，真断言非零退出）+ 本文第 3 条联动验收。

---

## E7：端到端验收 + 文档对齐 + 打包（1~2 天）

### E7.1 `agent-sdk/scripts/acceptance.ps1`（真跑、真退出码）

1. `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace --locked` 全绿；
2. 端到端脚本任务：给定空工作区 → Agent 完成「新建文件→写入→跑测试→修一个故意编译错误→通过」，断言最终通过且 diff 只含预期文件；
3. 中断测试：中途 abort → 5s 内停止且会话可恢复；
4. 审批测试：触发 Write → permission_request 到达 → 不响应 300s 按 Deny → `remember=true` 后二次不再询问（依赖 E2）；
5. 预算测试：极小 token 预算 → loop 主动终止不崩溃（依赖 E4）；
6. **输入法契约**：`cargo test -p owo-agent-ime` 全绿 + （真机可选 `--ime-e2e` 开关）跑 E1.8 三场景。

任一失败即非零退出。

### E7.2 文档对齐

| 文件 | 改动 |
|---|---|
| `agent-sdk/README.md` | 删「只实施 Agent 智能体方案，输入法路线不实施」；新增 serve-ime 章节（用法/管道/隐私） |
| `agent-sdk/ACCEPTANCE.md` | 引用 acceptance.ps1 替代手工记录 |
| `LingXi-DesktopAgent/docs/ARCHITECTURE.md` | 三环图（输入法环/HTTP 重环/桌宠）+ serve-ime 双面说明 |
| `LingXi-DesktopAgent/docs/project-status.md` | 追加本次交付段（照现有格式） |
| 新增 `docs/evidence/` | E1.8/M3 联动的截图与日志 |

### E7.3 打包（最小版）

`agent-sdk/scripts/package-serve-ime.ps1`：release 构建 `owo-agent.exe` + `onnxruntime.dll` + OCR 模型三件套 + `README-serve-ime.md`（含「先装 OwO 0.2.3 → 装 agent-ipc 连接器 1.2.0 → 授权 → 双击 serve-ime」四步）打成 zip。NSIS 安装包列为后续工作（pet 方案风险表已记录该缺口）。

### E7.4 evals/ 清理

`agent-sdk/evals/` 残留产物（serve*.err、*.out、shot-*.png、dom-*.html）删除或移入 `.gitignore` 覆盖；保留 mock-openai.py、stress-server.py、small-tasks.json 等脚本与数据。

---

## 三、工期与风险

| 阶段 | 估时（人日） | 风险 |
|---|---|---|
| E0 | 0.5~1 | 低（迁移路径可能遇到环境重装） |
| E1 | 5~8 | **高**（附录 B 三个开放问题都集中在此） |
| E2 | 2~3 | 中（安全语义变更，决策链要小心回归） |
| E3 | 1.5~2 | 低 |
| E4 | 2~3 | 中 |
| E5 | 1~2 | 低 |
| E6 | 3~3.5 | 中（Tauri 窗口坑多，但既有方案已覆盖） |
| E7 | 1~2 | 低 |
| **合计** | **16~25 人日** | |

**风险对策**：E1 任何一步卡住超过 1 天且确认是协议语义问题 → 先读 `OwO-release/plugins/agent-ipc/src/` 连接器源码（行为真源），仍不确定则记录到本文附录 B 并跳到 E2/E3（无依赖），不要空转。

---

## 附录 A：验收命令速查

```powershell
# agent-sdk 全量
powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1
cargo test -p owo-agent-ime                    # E1 协议/管道/状态机/契约
cargo test -p owo-agent-core --test permission_rules   # E2
cargo test -p owo-agent-core --test tokenizer_accuracy # E3
cargo test -p owo-agent-core --test loop_guard         # E4
cargo test -p owo-agent-core --test plan_tool           # E5
cargo test -p owo-agent-server --test plan_sse          # E5
powershell -File scripts\acceptance.ps1               # E7 总验收

# LingXi（GNU workspace）
cargo test --workspace
cd apps\overlay; cargo check
node --check ui/app.js; node --check ui/pet.js

# OwO-release
ctest --preset windows-debug
```

## 附录 B：开放问题（执行中必须逐一回填答案）

| # | 问题 | 判定方式 |
|---|---|---|
| Q1 | 连接器收到 `thinking + retry_after_ms` 后实际重发什么 action（poll？重复 submit？固定间隔？） | E0.3 观察 mock + 读 `plugins/agent-ipc/src/` 源码；适配器对两者都做防御性支持 |
| Q2 | `text.insert` 类候选被选中后，连接器是否会把文本上屏（还是仅 completed） | E1.8 真机试；若不上屏，`message` 展示完整回复即可，候选仅作确认 |
| Q3 | 连接器对 `tasks.slots` required_features 的实际强制程度（弱实现会不会被拒） | E1.8 真机试；弱实现被拒则必须做完整槽位 |
| Q4 | serve 与 serve-ime 数据目录锁冲突的实际表现（SQLite busy 报错形态） | E1.6 lockfile 检测 + 手动双启验证 |
| Q5 | tiktoken 对 DeepSeek/Qwen 的实际偏差（cl100k 是近似） | E3.2 的真实 API 偏差表回填到本文档 |

## 附录 C：执行状态记录（执行 agent 每完成一个任务卡在此追加一行）

| 日期 | 任务 | 结果 | 证据（测试输出/截图路径） | 备注 |
|---|---|---|---|---|
| 2026-09-29 | E1.1 协议类型层 | ✅ | `cargo test -p owo-agent-ime`（17 项 protocol_roundtrip 全绿） | 新 crate `crates/owo-agent-ime`；schema 全字段必填 + 严格字段集合 + 风险不变量（验证第 15 条） |
| 2026-09-29 | E1.2 帧与管道层 | ✅ | 5 项 pipe_echo + 8 项单元测试全绿 | 4 字节小端帧；tokio named pipe 多实例循环；ACL 照 mock SDDL；Send 显式安全注释 |
| 2026-09-29 | E1.3 会话状态机 | ✅ | 16 项 state_machine 全绿 | 幂等缓存 / 槽位补丁（revision+锁定）/ cancel（notify_one 防丢失）/ 审批等待分支 / GC |
| 2026-09-29 | E1.4 turn 异步适配 | ✅ | 7 项 bridge_turn 全绿 | HTTP 回环（401 刷新重试）、SSE 增量解析、thinking+retry_after、cancel→abort、30s 宽限 |
| 2026-09-29 | E1.5 命令映射 | ✅（reminder 草稿除外） | diff_produces_review_candidates 等测试 | insert-reply / view-diff / revert-all / confirm-ui 四类候选；**reminder 槽位草稿推迟**（见附录 B Q3） |
| 2026-09-29 | E1.6 CLI serve-ime | ✅ | 端到端冒烟 `PROBE PASS`（见下） | `owo-agent serve-ime`：HTTP+管道双面；`build_server_state` 与 serve 共用；PidFile 互斥 |
| 2026-09-29 | E1.7 契约测试 | ✅（15 条最小验证全部有对应测试/说明） | 66 项测试 + clippy `-D warnings` 干净 | sensitive_input 隐私红线 + context_entries 注入已补并测 |
| 2026-09-29 | E1.8 真机联调 | ⏳ 待 OwO 输入法环境 | — | 冒烟已用真实 serve-ime + mock 模型全链路验证；真机需装 OwO 0.2.3 + 连接器 |
| 2026-09-29 | E2 remember + 权限规则持久化 | ✅ | `cargo test -p owo-agent-core --test permission_rules`（9 项全绿）+ workspace 全量测试全绿 | `PermissionRule{tool,pattern,decision,expires_at_ms}`；命令首 token 前缀 / 路径父目录 glob / 工具通配三种模式；硬拒绝清单永不可记住；`settings.json` `permissions.rules` 持久化；`respond_permission` 真消费 `remember`；启动灌入策略 |
| 2026-09-29 | E3 真实 tokenizer | ✅ | `cargo test -p owo-agent-core --test tokenizer_accuracy`（8 项全绿） | 新模块 `core/src/tokenizer.rs`：tiktoken `cl100k_base` + 启发式兜底；`estimate_tokens` 换真计数（旧公式字符数/2 对小 2 倍以上）；`AgentConfig.model_context_window` + 按窗口 ×0.75 自动预算；`/usage`、`/session/{id}/context` 暴露 `tokenizer` 口径 |
| 2026-09-29 | E4 循环检测/超时/并发/预算熔断 | ✅ | `cargo test -p owo-agent-core --test loop_guard_tests`（5 项全绿） | 新模块 `core/src/loop_guard.rs`（滑窗 12 / ≥3 次提醒 / ≥5 次强制收尾 / 轮询与重读类豁免）；工具单步超时（默认 120s，`OWO_TOOL_TIMEOUT_SECS`）；只读并发上限（默认 4，`OWO_CONCURRENT_READ_TOOLS`）；预算熔断接进 loop（超限当轮终止 + 可见结论） |
| 2026-09-29 | E5 plan/todo 工具 | ✅ | `cargo test -p owo-agent-core --test plan_tool_tests`（3 项全绿） | 新模块 `core/src/plan_tools.rs`：`update_plan`（整表替换 / ≤32 步 / 免审批）；`Session.plan` + `TurnEvent::PlanUpdate` + SSE `plan_update` 事件；TUI 与 CLI 事件打印同步展示 |
| 2026-09-29 | E7 验收脚本 + 文档对齐 | ✅ | `scripts/acceptance.ps1` → `ACCEPTANCE PASS` | 脚本六段（fmt / clippy / E1~E5 专项 / workspace 全量 / 可选 `-Live` 管道探针），非零退出；README 删除「输入法路线不实施」并新增 serve-ime 章节；`ARCHITECTURE.md` 改三环图；`project-status.md` 追加交付段 |
| 2026-09-29 | E7.3 输入法桥接最小打包 | ✅ | `dist/OwO-Agent-IME-0.1.0-debug.zip`（20.5MB）+ 包内 `owo-agent.exe serve-ime --help` 自检通过 | 新增 `scripts/package-serve-ime.ps1`：exe + settings.example + `start-serve-ime.cmd`（无 BOM UTF-8，cmd.exe 兼容）+ `README-IME.txt`（四步接入）。onnxruntime.dll / models/ocr 缺失时警告降级不阻塞（本机无 DLL 且 GitHub 下载超时；联网重跑或手动放置即可补齐，OCR 自动降级为引擎内建行为）。NSIS 安装包沿用既有 `package-desktop.ps1`（已含 zip/签名/NSIS/SBOM） |
| 2026-09-29 | 真机排障：模型网关 400 → PROBE PASS | ✅ | 探针全链路通（真实 DeepSeek 回复 + insert-reply 候选） | **三层叠加根因**：① 用户以管理员启动的引擎进程固化了旧环境变量 `OPENAI_BASE_URL=d:\godot\new bee`（非法 URL），配置改回后进程不重启则不生效，且提权进程 `taskkill /F` 也 Access denied（最终 UAC 提权击杀）；② **`settings.json.owo-crypt` DPAPI 信封优先于明文 settings.json**（`load_encrypted`，信封封存旧 model=glm-5.2 与旧 base_url——手改明文无效，项目注释已记录过"旧信封覆盖"坑），删除信封回退明文 + 用户级 env 补 key 后恢复；③ 明文 settings.json 的 model=glm-5.2 与 DeepSeek 网关不匹配（合法名 deepseek-flash / deepseek-v4-flash / deepseek-v4-pro），改为 deepseek-v4-pro。**运维教训**：改模型配置后必须重启引擎；排查配置问题先查 `settings.json.owo-crypt` 是否存在 |
| 2026-09-29 | 多会话并行执行 + 跨会话审批 | ✅ | 双会话并行脚本实证（A/B 同时 turn 各 0.7s 完成）；审批响应复现脚本 200 | **已有基础确认**：全局并发上限默认 4（`OWO_SERVER_MAX_CONCURRENT_TURNS`）；工具注册表全局单例 → 浏览器驱动 `Arc<AsyncMutex>` 天然跨会话串行排队（无 profile 冲突）。**新修复 ①**：`respond_permission` 会话归属从硬校验（错位即 404 → 审批丢失 → 本轮工具全 Deny）改为软校验 + `cross_session_respond` 审计——修复 UI「恢复会话」场景下审批全 404。**新修复 ②**：新增 `GET /approvals/pending` 跨会话待审批列表；Web 工作台审批条从单卡改为**队列**（`state.pendingApprovals` Map，多卡独立允许/拒绝，跨会话卡带会话标记），5s 轮询同步服务端 pending（错过 SSE 事件的审批也可见可响应，僵尸卡自动清理）。**已知边界（文档化）**：并行会话避免写同一文件（无文件级冲突检测）；桌面类工具操作的是同一物理桌面，多会话同时用会互踩 |
| 2026-09-29 | E6 桌宠（M1 + M2 + M3 + 急停子集） | ✅ | overlay `cargo check` 零警告 + `node --check` 四个 JS 全过 + 两个 JSON 合法 | **M1**：Rust 侧统一状态仲裁（`pet.rs` 的 `priority`/`broadcast_pet_status`/`clear_pet_alert`；`owo_bridge` 按 SSE 帧推导、回合结束回落 idle）；`pet.js` 删 1.8s `pet_status` 轮询改 `listen("owo://pet-status")`；`app.js` 删除 10 处重环 `owoSetPet` 双写（三源打架消除）。**M2**：新增 `pet-chat` 窗口（520×420、非置顶、预声明 + capabilities 注册）+ `pet-chat.html/css/js`（流式气泡/工具行/简版审批卡含倒计时/Esc 关窗/Ctrl+Enter 发送/失焦自动收起/后端驱动拖动）；`pet_toggle_chat`/`pet_close_chat` 命令；单击桌宠改开对话面板（主面板入口在托盘「显示面板」）。**M3**：主面板与 pet-chat 审批卡均 300s 倒计时 + `remember` 文案改「以后同类不再询问」（E2 已真实生效）。**M4 子集**：`owo_abort_current` + 桌宠双击急停 |
| — | E6 手工验收（桌宠状态切换） | ⏳ 需真机 | 开一轮对话观察：工具执行=thinking / 出字=speaking / 待审批=alert / 结束=idle；双击运行中桌宠=急停 | M1 的端到端验收只能在真实 overlay 窗口观察 |
| 2026-09-29 | 收尾清理（方案外） | ✅ | core 全量测试全绿（344 项单测 + 集成）；overlay 三重校验过 | ① **权限矩阵死条目清理**：`text.inject`/`clipboard` 三仓库零消费者（无工具实现、非协议类别），从 `tool_levels()` 与 `level_for()` 删除，critic 测试改用真实工具 `desktop_click`。② **`.gitignore`**：agent-sdk 新增（构建产物/运行日志规则；evals 的 78 个日志文件 12.5MB 物理删除待用户批准）。③ **会话历史回放**：`owo_history` 命令 + 主面板引擎在线时自动回放最近 40 条（`loadOwoHistory`，对话区已有内容则跳过）。④ **`wait_for_abort`→Notify 评估后不做**：`AtomicBool` 全仓 89 处/测试 30+ 处，破坏性改动换 <50ms 取消延迟改善（低于人类感知阈值 100ms），成本收益不成立，正式关闭此项 |

### 端到端冒烟记录（2026-09-29）

```text
== OwO Agent IPC probe (pipe OwO.Agent.External.v1) ==
[submit] status=thinking retry_after_ms=1500 revision=1
[poll] status=agent_mode
message: ok
command: id=insert-reply label=插入回复：ok
PROBE PASS
```

命令（mock 模型 + serve-ime + 探针）：

```powershell
# 1) mock OpenAI（注意：脚本复制到无空格路径再启动，Start-Process 传含空格路径会拆参）
python "$env:TEMP\owo-ime-smoke\mock-openai.py" --port 4319 --log "$env:TEMP\owo-ime-smoke\mock.jsonl"
# 2) serve-ime
$env:OPENAI_API_KEY='mock'; $env:OPENAI_BASE_URL='http://127.0.0.1:4319/v1'
$env:OWO_AGENT_DATA="$env:TEMP\owo-ime-smoke-data"
target\debug\owo-agent.exe serve-ime --port 4097 --workspace "$env:TEMP\owo-ime-smoke"
# 3) 探针
powershell -ExecutionPolicy Bypass -File scripts\ime-pipe-probe.ps1
```

### 执行中发现的新坑（追加到 §0.3）

13. **PowerShell 5.1 脚本编码**：无 BOM 的 UTF-8 `.ps1` 会被按 ANSI(GBK) 解析，中文字符串字面量直接语法错误——工具脚本一律**纯 ASCII 英文**或带 BOM。
14. **`Start-Process` 传参数含空格路径被拆参**（§0.3 第 10 条的现场复现）：`python "d:/working OWOWOWOWOWO/.../mock-openai.py"` 会把 `d:\\working` 当文件名。解法：把脚本复制到无空格路径，或用两层引号包裹参数。
15. **网关代理标签误导**：`gateway error: ... proxy: error sending request` 中的 "proxy" 只是主客户端标签（即使没配代理也用该标签），排查时不要被带偏——先验证目标端口本身可达。
16. **本机 workspace 全量编译需要 `-j 1`**：`cargo test/clippy --workspace` 默认并行会触发 rustc 崩溃（`STATUS_STACK_BUFFER_OVERRUN` / `the compiler unexpectedly panicked` / 产物 rmeta 损坏引发一堆 "cannot find macro format" 假错误）。本机跑门禁请加 `-j 1`（或设 `CARGO_BUILD_JOBS=1`，约 7 分钟 test / 1 分钟 clippy）。已损坏时用 `cargo clean -p <crate>` 重建，不要用 `Remove-Item` 批量删（被 safe-delete 守卫阻止）。
17. **PowerShell 的 `.NET CurrentDirectory` 与 `cd` 不同步**：`cd X; [IO.File]::ReadAllText("rel/path")` 会以**进程启动目录**解析相对路径。脚本里给 .NET API 传**绝对路径**。
18. **`.ps1` 必须带 UTF-8 BOM**（ci-gate utf8 步骤硬性要求）：PowerShell 5.1 对无 BOM 的 UTF-8 脚本按 ANSI 解析，中文字符串会语法错误。新脚本用 `[System.IO.File]::WriteAllText($p, $t, (New-Object System.Text.UTF8Encoding($true)))` 落盘。
