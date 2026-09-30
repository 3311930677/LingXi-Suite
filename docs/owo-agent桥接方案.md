# LingXi × owo-agent 桥接方案（桌宠接 Codex 级执行引擎）

> 版本：v1.0 实施稿 | 日期：2026-09-23
> 依赖：`OwO/agent-sdk`（已按其 `docs/效能核心改造方案.md` 完成效能核心改造，946 测试全绿）
> 原则：**只消费 HTTP/SSE 契约，不改 agent-sdk 源码**；桌宠保留轻量内环，重任务委托 owo-agent

---

## 一、双环架构

```
┌─ LingXi 桌宠（皮 + 入口 + 陪伴层）───────────────┐
│  对话视图 / 审批卡片 / diff 卡片 / 桌宠状态机      │
│                                                  │
│  轻环: lingxi-agent（选区改写快路径，保留不动）    │
│  重环: owo-bridge → owo-agent serve（本机 4096）  │
└──────────────────────────┬───────────────────────┘
                           │ HTTP + SSE（openapi.json 契约冻结）
                ┌──────────▼──────────┐
                │ owo-agent.exe serve  │  ← 桌宠子进程托管（Job Object）
                │ 文件/shell/审批/沙箱/diff│
                └─────────────────────┘
```

## 二、契约清单（桥接只依赖这些）

| 用途 | 端点 | 结构 |
|---|---|---|
| 健康 | `GET /health` | `{healthy, version, auto_approve}` |
| 建会话 | `POST /session` | 请求 `{workspace, model?, system_prompt?}` → `SessionInfo` |
| 会话列表 | `GET /sessions` | `Vec<SessionInfo>` |
| 会话详情 | `GET /session/{id}` | 历史消息 |
| 回合 | `POST /session/{id}/turn` | 请求 `{prompt, attachments?}` → **SSE 流** |
| 审批回传 | `POST /session/{id}/permission/{rid}` | 请求 `{allow, remember?}` |
| diff/回滚 | `GET /session/{id}/diff`・`POST /session/{id}/revert` | `Vec<FileDiff>` |
| 中止 | `POST /session/{id}/abort` | — |

SSE 事件为 tagged JSON（`{"type":"token_delta",...}` / `tool_start` / `tool_result` / `permission_request` / `final` / `model_call` / `compaction`），统一带 `v` 版本字段。

## 三、新增 crate：`crates/owo-bridge`（W1，本次）

```
crates/owo-bridge/
├── Cargo.toml          # serde/serde_json/ureq(native-tls)/thiserror
└── src/
    ├── lib.rs          # pub use
    ├── types.rs        # 协议镜像（HealthResponse/SessionInfo/TurnRequest/PermissionResponse/FileDiff/SseEvent）
    ├── sse.rs          # 增量 SSE 解析器（独立可测）
    ├── client.rs       # OwoBridgeClient：health/session/turn(流式)/permission/diff/revert/abort
    └── proc.rs         # 服务托管（W2：探测端口 + 拉起 exe + Job Object）
```

- **选 ureq 而非 reqwest**：overlay 已用 ureq+native-tls（rustls 的 ring 会被 Smart App Control 拦截，os error 4551）；避免再引入一套 TLS 栈。
- **独立 crate 而非写在 overlay 里**：overlay 是 MSVC-only（excluded），独立 crate 能在 GNU 工作区跑 `cargo test`，契约解析逻辑可回归。
- 加入 LingXi workspace `members`（纯 Rust，无 MSVC 依赖）。

## 四、overlay 接线（W2）

新增 `apps/overlay/src/owo_bridge.rs`（Tauri commands）：

| 命令 | 行为 |
|---|---|
| `owo_status` | 健康检查 + 进程状态 + 端口，前端顶栏显示"引擎在线" |
| `owo_start_service` | 按设置拉起 `owo-agent.exe serve --port N --workspace W`；Job Object 绑定（桌宠退出不留孤儿进程） |
| `owo_new_session` / `owo_sessions` / `owo_history` | 会话管理（默认 workspace = 用户文档目录，可改） |
| `owo_send` | 后台线程跑 `turn_stream`，逐帧 `app.emit("owo://turn", frame)`；命令立即返回，不阻塞 UI |
| `owo_permission` | 审批卡片按钮回传 `{allow, remember}` |
| `owo_diff` / `owo_revert` / `owo_abort` | 改动审阅与中止 |
| `open_path` | diff 卡片"打开文件" |

设置项（`settings.rs` 扩展）：`owo_agent_exe_path`、`owo_agent_port`（默认 4096）、`owo_agent_workspace`、`owo_agent_auto_start`。

## 五、UI（W3）

主面板新增「任务」视图（与既有的选区/工具/设置并列）：
- 消息流：user / assistant(markdown) / `tool_start`+`tool_result` 折叠卡片（工具名、耗时、错误）
- **审批卡片**：`permission_request` 事件 → 卡片（工具名/参数/风险）→ [允许] [拒绝] [本会话记住]
- **diff 卡片**：`final` 后拉 `/diff`，逐文件展示 before/after，[回滚] 按钮
- 桌宠状态联动：干活中 / 等待审批 / 完成（桌宠差异化，Codex 没有的）

## 六、验收

- W1：`cargo test -p owo-bridge` 全绿（SSE 解析、类型往返、mock 服务器 health/session/turn 集成）
- W2：桌宠内发起一次真实 turn（读文件→改文件→跑测试），审批卡片弹出并可放行；`/diff` 可见改动；桌宠退出后 `owo-agent.exe` 不残留
- W3：UI 三件套（消息流/审批/diff）在同一次任务里全部出现

## 六点五、实施记录

### W1（crates/owo-bridge，已完成）

- 客户端：`health` / `create_session` / `list_sessions` / `session_detail` / `turn_stream`（SSE）/ `respond_permission` / `diff` / `revert` / `abort`
- SSE 增量解析器（多行 data、注释、id/retry、UTF-8 安全）+ 事件类型 `TurnEvent`（未知类型向前兼容）
- **鉴权（实施中发现，必须支持）**：引擎 X03 安全边界要求 `Authorization: Bearer <token>`；`GET /auth/token` 为公开配对端点。客户端懒加载并缓存 token，**401 时自动刷新重试一次**（引擎重启轮换 token 的场景）。
- 单测 18 项全绿（含鉴权配对、401 刷新、SSE 帧序、错误映射）+ 1 doc-test。

### W2（overlay 接线，已完成）

- `owo_bridge.rs`：11 个 Tauri 命令（status/start/stop/ensure_session/sessions/use_session/send/permission/diff/revert/abort）
- 服务托管：`serve --port N --workspace W` + `CREATE_NO_WINDOW` + **Job Object（KILL_ON_JOB_CLOSE）**；桌宠退出不留孤儿进程
- 事件转发：后台线程消费 SSE，逐帧 `app.emit("owo://turn", {session_id, event, data})`
- 设置项：`owo_agent_exe_path` / `owo_agent_port` / `owo_agent_workspace` / `owo_agent_auto_start`
- **沙箱根约束（实施中发现，必须遵守）**：引擎的 policy workspace 在启动时由 `--workspace` 确定；**会话 workspace 必须与之完全一致**，否则文件工具报"路径越界"（实测 `write_file → 路径越界：hello.txt`）。overlay 的 `owo_ensure_session` 已实现工作区一致性校验：设置变更时自动重建会话；`owo_use_session` 从服务端详情回填工作区。

### 真实链路实测（mock 模型，确定性）

`crates/owo-bridge/tests/live_engine.rs` + `mock_openai.py`（一键：`tests\run_live.ps1`）：

```
事件序列 = [progress, permission_request, tool_use, tool_result, progress, token_delta, final]
审批 = 1 放行 = 1
改动 = [FileDiff { path: "hello.txt", before: None, after: Some("hi") }]
workspace 落盘文件 = ["hello.txt"]
```

即：真实引擎 + 鉴权配对 + SSE 流式 + 审批回传 + 文件写入 + diff 全链路验证通过。

### W3（任务视图 UI，已完成）

- 「对话」视图顶部新增**引擎状态条**：在线指示点 + 版本 + 未连接时的 [启动引擎] 按钮 + 「重任务」模式开关；tooltip 展示工作区 / 引擎程序路径 / 是否由灵犀托管。
- **重任务模式**：勾选后发送分流到 `owo_ensure_session` + `owo_send`；SSE 帧经 `owo://turn` 事件回流（未勾选 = 原有轻环 `agent_chat`，两条路径共存互不影响）。
- **卡片化呈现**：
  - 工具卡片：`tool_use` + `tool_result` 合并为可折叠卡片（工具名 / 参数 / 成功或失败 / 错误详情）；
  - **审批卡片**：`permission_request` → 工具名 + 原因 + 参数 JSON + [允许] [拒绝] [本会话记住] → `owo_permission`；
  - **改动卡片**：`final` 后自动拉 `owo_diff`，逐文件 before/after 预览 + [全部回滚]（`owo_revert`）。
- 流式输出：`token_delta` 累积到同一条助手气泡；`progress` 复用 thinking 气泡。
- **桌宠状态联动**：发送 → `thinking`；审批卡片出现 → `alert`；完成 → `speaking`（2.5s 后回 `idle`）；错误 → `idle`。
- 验证：`node --check app.js` 通过；UI 契约交叉检查——app.js 引用的 **79 个 DOM id 全部存在**、**43 个 invoke 命令全部已在 `main.rs` 注册**（`set_window_options` 为检查脚本正则误报，实为已注册）。

### W4（界面改版，已完成）

- **桌宠新皮肤 `codex-cloud`**（`ui/assets/skins/codex-cloud/`）：模仿 Codex 桌宠的蓝色云朵机器人（深色面罩 + 青色 `>_`），4 个状态为手写 SVG + SMIL 动画——思考时 `_` 呼吸 + 头顶三点轮流亮、回答时整体弹跳 + 声波条、提醒时左右抖动 + 警示黄。皮肤可在桌宠右键菜单 / 设置页切换。
- **视图收敛**：顶部 tab 从 5 个精简为 **对话 / 工具 / 设置**；「改写」「QQ 草稿」收进工具页顶部**快捷卡片**（显示快捷键徽标）。热键 `Ctrl+Alt+Space` 抓到选区时仍自动切到改写视图（前端轮询检测 revision 变化）。
- **对话 Codex 化**：
  - 头部**模式分段**：「闲聊 | ⚡重任务」并列胶囊 + 右侧引擎状态点与 [启动] 按钮（引擎入口从隐蔽的勾选框升级为主切换项）；
  - **胶囊输入条**：左侧 [+]（一键把剪贴板内容加入输入框）、圆角输入区、右圆形 ↑ 发送；
  - 输入条下方两个图标：**打字**（聚焦输入框）与**语音**（Web Speech API，`is-recording` 脉冲态；环境不支持时提示改用打字或选区）。
- **显示/隐藏桌宠**：面板标题栏新增宠物按钮，新增后端命令 `set_pet_visible`（只改可见性，不动气泡覆盖），与设置页开关共用同一持久化字段。
- 默认进入**对话视图**（此前默认是改写视图）。
- 验证：`node --check` 通过；UI 契约交叉检查——**85 个 DOM id 全部存在、45 个 invoke 命令全部已注册**（含新增 `set_pet_visible`）。

### 环境备忘（新增）

- **引擎启动强制要求模型配置**：`OPENAI_API_KEY` 或 `OPENAI_BASE_URL` 指向本地兼容端点，缺一则启动即退出。
- **PowerShell 传含空格路径给 Start-Process 会被拆参**：`D:\working OWOWOWOWOWO\...` 必须用两层引号包裹（`'"D:\path with space"'`）。Rust 侧 `Command::arg` 无此问题。
- 引擎插件从 `<workspace>/plugins/` 发现；用源码目录当工作区会加载其插件清单（含已删除插件时会有 MCP 超时）。桌宠默认工作区为文档目录。
- 本机 `Invoke-WebRequest` 走系统代理（127.0.0.1:7897），探测本地端口请用 `curl.exe --noproxy "*"`；ureq 只读环境变量代理，不受影响。

## 七、环境备忘

- GNU 侧测试：`cd LingXi-DesktopAgent && cargo test -p owo-bridge`
- overlay（MSVC）构建：`cd apps/overlay && cargo run`（需 WebView2）
- agent 服务：`start-agent-server.cmd`（需先 `set OPENAI_API_KEY=...`，密钥不落盘）
