# 桌宠接入 Agent 框架 — 实现方案

> 目标：把 `pet` 桌宠从「会动的装饰」升级为「常驻的 Agent 前台」——能说话、能接活、能请示、能被打断。
> 前置阅读：`docs/agent-optimization-plan.md`（§〇 决定：UI 一律走重环 `owo-bridge`，不再走 `agent.rs` 的 `DenyAll` 轻环）
> 约束：`docs/engineering-standards.md` §3 窗口六条铁律、§4 UI 规范

---

## 一、现状盘点（已核实的事实）

### 1.1 桌宠侧

| 项 | 位置 | 现状 |
|---|---|---|
| 前端 | `apps/overlay/ui/pet.html`(21) / `pet.js`(506) / `pet.css` | 原生 JS，**零构建** |
| 状态机 | `pet.js` + `apps/overlay/src/pet.rs` | 4 态：`idle / thinking / speaking / alert` |
| 动画 | `pet.js:75-121` | spritesheet 帧动画（`analyzeSheet` 采样 alpha 剔空白帧）+ 静态图回退 |
| 气泡 | `bubble` + `sayTemp()` | 单行临时台词，已有复用基础 |
| 皮肤 | `pet.js:253-294`、`pet_skin.rs` | 右键菜单切换，15 套 `petdex-*`，默认 `petdex-ai-hoshino` |
| 通信 | `pet.js:3-5` `TAURI.core.invoke` | ✔ 已有 |
| 轮询 | `pet.js:478-502` | **1.8s 轮询** `pet_status` + `qq_poll_latest` |
| 事件 | `pet.js:503-505` | 已 `listen("pet-config-changed", …)` |
| 窗口 | `tauri.conf.json:31-45` | label `pet`，220×260，`transparent / alwaysOnTop / visible:false` |
| 权限 | `capabilities/default.json:5` | windows **已含 `"pet"`** → 调 `owo_*` 命令无需改权限 |
| 现有联动 | `ui/app.js:1607` `owoSetPet(status)` | 主面板已在切桌宠状态，说明通道是通的 |

### 1.2 Agent 侧（重环，已具备）

`apps/overlay/src/owo_bridge.rs` 已经把脏活干完了：

- `OwoService` 用 **Job Object `KILL_ON_JOB_CLOSE`** 托管 `owo-agent.exe serve --port P --workspace W`（`:41-56, 522-571`）
- `owo_ensure_session` / `owo_sessions` / `owo_use_session`
- `owo_send`：**后台线程收 SSE，逐帧 `app.emit("owo://turn", …)`**（`:440-471`）
- `owo_permission` / `owo_diff` / `owo_revert` / `owo_abort`
- 鉴权：bridge 内部 `GET /auth/token` + Bearer，401 自动刷新重试（`crates/owo-bridge/src/client.rs:83-160`）

`TurnEvent`（`crates/owo-bridge/src/types.rs`）是 `#[serde(tag="type")]` 枚举：
`Progress / ToolUse / ToolResult / PermissionRequest / Final / TokenDelta / Compaction / Other`

**结论：桌宠接入 80% 是前端工作，Rust 侧只需补「状态广播」和少量组合命令。**

### 1.3 三个必须记住的运行时约束

1. **CSP**：`tauri.conf.json:48` `connect-src` 只有 `ipc:`/`ipc.localhost` → **前端禁止 `fetch("http://127.0.0.1:4096")`，一切走 `invoke`**（HTTP 由 Rust 侧 `ureq` 发出）。
2. **审批会阻塞**：服务端 `ChannelApprover` 等待 **300s 超时默认 `Deny`**（`owo-agent-server/src/lib.rs:3845-3871`）。桌宠必须给审批 UI 并倒计时。
3. **同会话并发 turn 返回 409**（`lib.rs:1383-1385`）。桌宠要处理「上一轮还在跑」。

---

## 二、目标架构

```
┌────────── 桌宠（常驻 220×260, alwaysOnTop, transparent）──────────┐
│  pet.js                                                          │
│   ├─ listen("owo://turn")        ← 全局广播，无需轮询             │
│   ├─ listen("owo://pet-status")  ← 新增：Rust 侧仲裁后的状态      │
│   ├─ invoke("owo_send"/"owo_permission"/"owo_abort")             │
│   └─ 单击 → 展开 pet-chat 面板（新窗口 520×420）                  │
└──────────────────────────┬───────────────────────────────────────┘
                           │ Tauri invoke / event
┌──────────────────────────▼───────────────────────────────────────┐
│  apps/overlay/src/owo_bridge.rs（改造：SSE → 状态仲裁 → emit）    │
└──────────────────────────┬───────────────────────────────────────┘
                           │ ureq + SSE（127.0.0.1:4096，Bearer）
┌──────────────────────────▼───────────────────────────────────────┐
│  owo-agent 服务：session / turn(SSE) / permission / diff          │
│  / revert / abort + 审批 + Windows 沙箱 + 42 工具 + MCP           │
└──────────────────────────────────────────────────────────────────┘
```

**状态仲裁放在 Rust 侧**（而不是 pet.js 里 switch），好处：主面板与桌宠看到的状态永远一致，避免 `app.js:1607` 与桌宠各自为政。

---

## 三、分阶段实施

### M1 — 事件驱动状态，干掉 1.8s 轮询（0.5 天）

**改 `owo_bridge.rs`**：在 `owo_send` 的 SSE 循环里按事件推导桌宠状态并广播。

```rust
// apps/overlay/src/owo_bridge.rs（owo_send 的后台线程内）
fn pet_state_for(ev: &TurnEvent) -> Option<&'static str> {
    Some(match ev {
        TurnEvent::ToolUse { .. }               => "thinking",
        TurnEvent::TokenDelta { .. }            => "speaking",
        TurnEvent::PermissionRequest { .. }     => "alert",   // 需要人
        TurnEvent::Final { .. }                 => "speaking",
        TurnEvent::ToolResult { ok: false, .. } => "alert",
        _ => return None,
    })
}

// 每帧处理时：
if let Some(s) = pet_state_for(&frame) {
    let _ = app.emit("owo://pet-status", serde_json::json!({ "status": s }));
}
// 回合结束（Final / turn_failed / 流关闭）统一回落 idle
let _ = app.emit("owo://pet-status", serde_json::json!({ "status": "idle" }));
```

**状态优先级**（避免 `alert` 被后续 `token_delta` 冲掉）：Rust 侧维护 `alert > thinking > speaking > idle`，`alert` 只能由「审批已响应」或「回合结束」清除。

**改 `pet.js`**：删掉 `pet_status` 的 1.8s 轮询（`qq_poll_latest` 是否保留见下），改为：

```js
const { listen } = TAURI.event;
await listen("owo://pet-status", (e) => setPetState(e.payload.status));
```

窗口重建时仍读一次 `pet_status` 作为兜底。

**审查补充的三个易漏点**：

1. **清除既有双写源**：现状有两个状态写入方——`app.js:1607` `owoSetPet()`（注释原话「轻环与重环共用同一状态通道」，内部 `invoke("set_pet_status")`）和 `pet.js:501` 的 1.8s 轮询。加 Rust 仲裁后若不删旧路径，就是三个源打架。M1 必须同时：删除 `app.js` 中所有 `owoSetPet(...)` 调用点（状态改由 `owo_bridge.rs` 统一推导），轮询移除。
2. **QQ 新消息是第四个状态源**：`pet.js:482-485` 注释明确 `qq_poll_latest` 会把桌宠切到 `alert`（仅前台 QQ 窗口、不自动回复）。仲裁表必须给它定义生命周期：`alert(QQ)` 由「用户点击桌宠」清除，`alert(审批)` 由「审批响应/回合结束」清除，二者不能互相覆盖。
3. **不要每次状态切换都落盘**：`pet.rs` 的 `set_pet_status` 走 `persist_backend_settings` 写盘；thinking/speaking 按帧切换后频率会到每秒多次。新的 `owo://pet-status` 事件只进内存与 emit，**持久化仅保留皮肤/可见性**（重启恢复为 idle 即可）。

**验收**：开一轮对话，桌宠在「工具执行=thinking / 出字=speaking / 要审批=alert / 结束=idle」间自动切换；`pet_status` 轮询消失（`qq_poll_latest` 若保留则 1.8s 周期仍在，属预期）。

---

### M2 — 桌宠对话输入 + 流式气泡（1.5 天）

桌宠 220×260 放不下输入框，新增 `pet-chat` 窗口（520×420，深色、圆角 12px、`decorations:false`）。遵循铁律：

- 铁律①：`capabilities/default.json` 的 windows 数组**必须加 `"pet-chat"`**，`tauri.conf.json` 加窗口定义，否则白屏；
- 铁律③：关闭用 `destroy()`，不用 `close()`；
- 铁律②：`alwaysOnTop` 默认 false（`pet` 主窗口保留置顶，聊天面板不置顶以免遮挡工作）；
- 铁律⑤：面板内只允许一个滚动容器（消息列表滚，输入框固定）；
- §4：等待态 `setStatus()` + spinner；Esc 关窗、Ctrl+Enter 提交；剪贴板走 `widget.js` 后端命令（WebView2 下 `navigator.clipboard` 静默失败）。

**手势映射（审查修订：现状 single→开主面板、double→pokeReact，直接改单击语义会顶掉主面板入口和彩蛋）**：

| 手势 | 现状 | 改为 |
|---|---|---|
| 单击 | 开主面板 `toggle_panel()` | 开/关 `pet-chat` |
| 双击 | `pokeReact` 彩蛋 | 回合运行中=中止（默认关，设置可开）；否则保留彩蛋 |
| 右键 | 皮肤菜单 | 不变 |
| 拖拽 >420px | `petted()` | 不变 |

主面板入口迁移到托盘菜单（已有 `core:tray:default` 权限）；再次单击或 Esc 关闭 `pet-chat` 用 `destroy()`。

```js
async function send(text) {
  const st = await invoke("owo_status");
  if (!st.running) { await invoke("owo_start"); /* 轮询 health 直到 ready */ }
  await invoke("owo_ensure_session");           // 工作区变更会自动重建会话
  await invoke("owo_send", { text });           // 内部开后台线程收 SSE，立即返回
}
```

**流式气泡**：`listen("owo://turn")` → `token_delta` 追加到当前气泡（打字机），`final` 定稿并触发 `sayTemp()` 让桌宠本体「开口」，`turn_stats` 在角落显示「12 步 · 8.4s · ¥0.03」。

**验收**：输入「读一下 README 前 20 行」→ 气泡逐字出现、桌宠同步 speaking、结束回落 idle 并显示用量；关掉面板重开，历史仍在（读 `owo_sessions` + `session_detail`）。

---

### M3 — 审批 UI（1 天）🔴 最关键的一环

审批不响应 = 300s 后 Deny = 任务白跑；桌宠是 alwaysOnTop 常驻，是最不会被忽略的入口。

```js
await listen("owo://turn", (e) => {
  if (e.payload.type === "permission_request") showApprovalCard(e.payload);
});

async function decide(req, allow) {
  await invoke("owo_permission", { requestId: req.request_id, allow });
  hideApprovalCard(req.request_id);
}
```

审批卡片文案（§4：中文口语化 + 错误带下一步建议）：

> **需要你同意**：`run_command`
> `cargo test -p owo-agent-core`
> [允许] [拒绝] · 剩余 04:32（超时将按拒绝处理）

**两个现状缺陷，UI 要如实反映**：

1. `remember` 目前**服务端不消费**（`lib.rs:1671-1675` 只读 `allow`）→ 「总是允许」按钮先不上线或标注「暂未生效」，随 `agent-optimization-plan.md` P2-1 一起开。
2. turn SSE **无 keep-alive**（区别于 `workflow_api.rs:551` 的 15s 心跳）→ 前端禁用「N 秒无帧 = 断开」，只认 `final`/`turn_failed`/流关闭。

**验收**：触发一次写文件 → 弹审批卡并倒计时；点允许 → 继续执行；不点 → 300s 后收到 `ok:false` 的 `tool_result`，卡片提示「已超时并按拒绝处理」。

---

### M4 — 工具活动可视化 + 打断（0.5 天）

- `tool_use` → 活动行 `🔧 run_command…`（spinner）；`tool_result` → ✔/✘，`ok:false` 时折叠展示 `error`。
- 面板常驻 **[停止]** → `invoke("owo_abort")`；桌宠本体双击也可中止（复用 `pokeReact` 手势通道，语义改为「紧急停止」，设置里可关，防误触）。
- 409 并发：`owo_send` 返回冲突时提示「上一轮还在跑」+「中止并重开」按钮。

---

### M5 — 会话管理与文件附件（1.5 天）

- 面板顶部会话下拉：`owo_sessions` → `owo_use_session`，支持新建会话。
- **拖拽文件到桌宠** → 附件。当前 `crates/owo-bridge` **没有附件命令**（只有 `TurnRequest{prompt, attachments}` 类型定义），需新增：
  ```rust
  // crates/owo-bridge/src/client.rs
  pub fn upload_attachment(&self, session: &str, name: &str, b64: &str)
      -> Result<(), BridgeError>;   // POST /session/{id}/attachments（base64 JSON，50MB 上限）
  ```
  overlay 侧包一个 `owo_upload_attachment`，内部走 `spawn_blocking` + timeout（§2 编码规范）。
- **拖拽实现注意**（审查补充）：Tauri v2 的 `dragDropEnabled` 默认拦截 HTML5 drop 事件——外部文件拖入**不能用** `addEventListener('drop')`，要用 `getCurrentWebview().onDragDropEvent`（或 Rust 侧 `on_window_event(DragDrop)`）转发给前端。
- 回滚入口：`owo_diff` → 列出改动文件 → `owo_revert`；桌宠气泡提示「已撤销 N 个文件改动」。

**验收（审查修订）**：拖一个**文本文件**（如 .md）进桌宠 → 问「这个文件讲了什么」→ Agent 通过文件工具读到并总结——附件只是**路径注入**，模型层**无多模态输入**（优化方案 P1-1 已确认 `gateway.rs` 无 `image_url`），「拖截图问图里有什么」要等 P1-2 落地后才可行，初版验收标准属过度承诺；`diff` 能列出并撤销改动。

---

### M6 — 主动搭话（2 天，可选）

OwO 侧已有 `/memory/observations`、`/proactive/suggestions`、`/skills/health`。做一个低频（建议 60s）Rust 轮询线程：有建议 → `emit("owo://proactive", {text, actions})` → 桌宠 `alert` 态 + `sayTemp()` 冒泡；点气泡展开 `pet-chat` 处理。频控与静默沿用 OwO 现有阈值；敏感面（密码/支付/验证码）一律不提示。

轮询放 Rust 线程，**不要**用 PowerShell 子进程（§4 已知坑：约 1s/次开销）。服务未运行/健康检查失败时指数退避（60s → 最长 5min），收到任何 turn 事件即重置，避免每分钟刷一次失败日志。

---

## 四、改动清单

| 文件 | 改动 |
|---|---|
| `apps/overlay/src/owo_bridge.rs` | SSE 帧 → 状态仲裁 + `emit("owo://pet-status")`；回合结束回落 idle |
| `apps/overlay/src/pet.rs` | 新增 `pet_open_chat` / `pet_close_chat`（`destroy()`）；`set_pet_status` 支持优先级仲裁 |
| `crates/owo-bridge/src/client.rs` | `upload_attachment`、`proactive_suggestions` |
| `crates/owo-bridge/src/types.rs` | 补 `AttachmentInfo`、`Suggestion` |
| `apps/overlay/tauri.conf.json` | 新增 `pet-chat` 窗口（520×420） |
| `apps/overlay/capabilities/default.json` | windows 加 `"pet-chat"`（铁律①，漏了必白屏） |
| `ui/pet.js` | 去轮询 → `listen`；手势表改造；审批/工具/用量/附件拖拽 |
| `ui/app.js` | **删除 `owoSetPet` 调用点**（状态改由 Rust 侧 `owo://pet-status` 统一驱动，M1） |
| `ui/pet-chat.html` / `pet-chat.js` / `pet-chat.css` | 新增聊天面板（`--surface`/`--stroke`/`--brand:#8aadf4`，圆角 12px） |
| `ui/pet.css` | 审批卡、活动行、Toast；保持单层滚动 |

---

## 五、风险与对策

| 风险 | 对策 |
|---|---|
| `pet-chat` 忘注册 capabilities → 白屏 | 铁律①；出问题按 §3.6 顺序排查（杀孤儿进程 → 重建缓存目录） |
| 审批 300s 静默 Deny | 倒计时卡片 + `alert` 态 + 桌宠抖动提醒 |
| SSE 无心跳导致误判断连 | 禁用「无帧超时」，只认 `final`/`turn_failed`/流关闭 |
| 窗口太小挤爆布局 | 对话一律走 `pet-chat`，桌宠本体只留气泡与状态 |
| 误触中止 | 「双击=停止」默认关闭，设置可开 |
| `remember` 无效被用户发现 | M3 暂不上线该按钮，随 P2-1 一起开 |
| 主程序退出后服务僵死 | 保持 Job Object `KILL_ON_JOB_CLOSE`，勿改 `CREATE_NEW_PROCESS_GROUP` |
| **owo-agent.exe 分发**（审查新增，原方案未覆盖的最大工程缺口） | `resolve_exe_path` 候选链硬编码了开发机路径 `D:\working …\OwO\agent-sdk\target\`，用户机器上不存在。发布前必须：NSIS/zip 把 `owo-agent.exe` + `onnxruntime.dll` + OCR 模型三件套打进安装包，版本与 bridge 协议对齐；缺失时 UI 给「引擎未安装/损坏」的明确引导而非静默失败。`tauri.conf.json:56` `bundle.active: false`——打包链路整体待建 |
| 锁与阻塞 | 一律 `.safe_lock()`；`invoke` 内阻塞走 `spawn_blocking` + timeout |

---

## 六、排期与验收

| 里程碑 | 内容 | 估时 | 依赖 |
|---|---|---|---|
| M1 | 事件驱动状态，去轮询 | 0.5 d | 无 |
| M2 | 对话面板 + 流式气泡 | 1.5 d | M1；服务能起（P0-2 已修） |
| M3 | 审批 UI + 倒计时 | 1 d | M2 |
| M4 | 工具可视化 + 中止 | 0.5 d | M2 |
| M5 | 会话管理 + 附件 + 回滚 | 1.5 d | M2；需新增 bridge 附件命令 |
| M6 | 主动搭话（可选） | 2 d | M1 + OwO 感知链路 |
| **合计** | | **≈ 7 人日**（M6 不计则 5 人日） | |

**端到端验收脚本**（建议做成 `scripts/pet-agent-smoke.ps1`，真断言、非零退出）：

1. 启动 overlay → 桌宠可见且为 `idle`；
2. 单击展开 `pet-chat` → 窗口出现、无白屏、`alwaysOnTop=false`；
3. 发一句「列出当前目录文件」→ 断言收到 `tool_use` 且桌宠切到 `thinking`；
4. 触发一次写操作 → 断言 `permission_request` 到达、倒计时在跑、点允许后收到 `ok:true`；
5. 点「停止」→ 断言 5s 内回合终止、桌宠回 `idle`；
6. 关面板 → 断言窗口已 `destroy`（进程列表里无残留 WebView）；
7. 拖拽文件 → 断言附件上传成功且 Agent 能读到。

---

## 七、与优化方案的依赖关系

桌宠方案**不阻塞**于 OwO 的大改造，但有三处会被优化方案解锁：

| 优化项 | 解锁后桌宠可增强的能力 |
|---|---|
| P2-1 权限规则持久化 + `remember` 生效 | 「总是允许」「本会话免询问」按钮可上线 |
| P2-3 hooks | 桌宠可在 `PostToolUse` 自动播报（如「已格式化 3 个文件」） |
| P1-1 精确 tokenizer | 桌宠用量气泡的 token 数才可信（现在是字符/2+4 粗估） |

建议实施顺序：**P0（1–2 天，先让仓库能编译、验收可信）→ M1~M4（3.5 天，桌宠先跑通主链路）→ P1/P2**。这样第 4 天就能看到一个「会说话、会请示」的桌宠，而不是等两周。
