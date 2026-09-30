# Agent 能力差距审查与分批修改计划（2026-09-30）

> 审查对象：`LingXi-DesktopAgent`（轻环 + overlay + 桌宠）与 `OwO/agent-sdk`（重环引擎）
> 对标基线：Codex CLI 0.144 / Claude Code / CodeBuddy（2026 年调研，见文末参考）
> 姊妹文档：`agent-optimization-plan.md`（P0 已完成；E1~E5 已完成 tokenizer/remember/循环防护/update_plan）
> 本文档为全面差距清单，取代单点优化视角，供后续任务规划直接拆条引用。

---

## 〇、总体判断

引擎层：**桌面自动化（17 工具）+ 会话管理（fork/rewind/加密存储）+ 权限模型（分级/hard-deny/Auto-review）+ Windows 沙箱已属一线水平**；但**模型接入层（单协议、无多模态、无 prompt caching）、扩展生态（无 Hooks、MCP 只有 tools）、代码工程能力（无 LSP、无 multi-edit、无 web_search API）明显滞后**。

UX 层：SSE 骨架、审批卡、桌宠状态机底子不错，但**可靠性细节（turn_failed 卡死、消息重复、双工具卡）和信息呈现（无 markdown、工具黑盒）**距离成熟桌面 Agent 有明显差距；会话管理与引擎分发是"后端就绪、前端/打包缺失"的断层。

---

## 一、A 系列：引擎能力问题（对标 Codex / Claude Code / CodeBuddy）

### A1 模型接入层（影响能力天花板，最高优先级）

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A1-1 | **无 Anthropic 原生 provider** | `gateway.rs` 仅 `OpenAiCompatibleProvider`（gateway.rs:440），全仓无 `/v1/messages` | Claude Code 原生；prompt caching 对长会话成本是数量级差异 | 新增 `AnthropicProvider`：`/v1/messages` + `tool_use` 流式 + `cache_control` ephemeral 打 system 与最近一轮；`ANTHROPIC_API_KEY` 走现有代理配置 |
| A1-2 | **主链路无多模态** | `ChatMessage.content: Option<String>`（gateway.rs:17-25）；`image_url` 仅旁路 vision.rs:226 | Codex/Claude Code/CodeBuddy 均支持贴图/截图进对话；桌面 Agent 截图理解是刚需 | `content` 升级 `MessageContent`（`Text` / `Parts(Vec<ContentPart>)`），OpenAI 分支字符串兼容；附件端点图像真正入上下文；OCR/vision 旁路结果可作为 ContentPart 注入 |
| A1-3 | **无 prompt caching** | 同 A1-1（OpenAI-compatible 分支未做缓存标记） | Claude Code 长会话依赖 cache_read_input_tokens | 随 A1-1 落地；验收指标：第二轮 cache 命中 |
| A1-4 | 代理配置缺 `NO_PROXY` | gateway.rs:452-469 支持 5 个代理变量但无 NO_PROXY | 基础运维项 | 补 `NO_PROXY`/`no_proxy` 解析（本地网关 127.0.0.1 必须直连） |

### A2 扩展生态（Hooks / MCP）

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A2-1 | **Hooks 系统完全缺失** | 全 crates 搜 `hook` 零命中；`plugin.rs` 只是插件市场治理+工具桥 | Claude Code：PreToolUse/PostToolUse/UserPromptSubmit/Stop/PreCompact/SessionStart，exit code 2 = 阻断并回喂 stderr；Codex 0.144 也有 postToolUse hooks | 实现事件总线 + settings.json `hooks` 配置（matcher 支持工具名 glob）；MCP stdio 子进程复用沙箱门卫 |
| A2-2 | **MCP 只有 tools** | mcp.rs 仅 `tools/list`+`tools/call`（:138、:289-306） | Claude Code/CodeBuddy 全能力 MCP；filesystem/observability 类 server 依赖 resources/prompts | 补 `resources/list|read`、`prompts/list|get`、`roots` 声明工作区 |
| A2-3 | MCP 无认证 | 无 OAuth/密钥管理 | Codex 0.144 MCP 交互式认证已稳定（Keychain/Credential Manager） | 远期：MCP server 凭据入 DPAPI/凭据管理器 |

### A3 代码工程能力（coding agent 分界线）

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A3-1 | **无 LSP 代码导航** | 42 工具无 go_to_definition/references/workspace_symbol | Claude Code/CodeBuddy 内建；定位效率数量级差异 | LSP 侧车：把 definition/references/symbol 包成工具（原 plan P3 已列，未动工） |
| A3-2 | **编辑工具单条替换** | `edit_file` old_str/new_str 单次替换（tools.rs:643-672） | Codex apply-patch；Claude Code 单条但稳定；CodeBuddy 多文件批量 | ① multi_edit（数组原子应用，任一失败全回滚）② 可选 apply-patch 容错（fuzzy 匹配缩进）③ 依托快照天然可回滚 |
| A3-3 | **无 API 型 web_search/web_fetch** | 全仓零命中；靠 `browser_search` 抓 Bing/Baidu | Codex/Gemini CLI 内建 web 工具 | 新增 `web_search`（可配 SearXNG/Tavily/Bocha）+ `web_fetch`（HTML→markdown）；浏览器桥保留作 fallback |
| A3-4 | 无 Git 专项工具 | 42 工具无 git 类，靠 run_command | Claude Code/Codex/Cursor 均有 Git/PR 集成 | 新增 `git_status/diff/commit/log` 包装（只读免审批），PR loop 远期 |

### A4 上下文工程

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A4-1 | 压缩无结构化决策骨架 | 摘要仅靠 prompt 约定（agent.rs:923-927） | Codex Guardian 上下文敏感检查；压缩后重复踩坑是通病 | 摘要模板强制保留「用户目标/已修改文件/失败尝试」三段结构化 JSON，写回 system |
| A4-2 | keep_recent 按条数 | agent.rs:46/59 默认 20 条 | 超长工具结果一条吃光预算 | 改 `keep_recent_tokens ≈ budget × 0.25` |
| A4-3 | 压缩失败静默返回 | agent.rs:928-935 失败 `return Ok(None)`，本回合可能直接超窗 400；`compact_truncate`（:1174）存在但未接线 | Codex 0.144：压缩中断回滚到最近一致检查点+哈希校验 | 压缩失败 → 调 `compact_truncate` 兜底 + 事件上报 |
| A4-4 | 压缩摘要审计不含全文 | agent.rs:942-951 只记条数 | 排障需要 | 审计事件附摘要 hash，全文入 SQLite session 表 |

### A5 并行与子代理

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A5-1 | **多代理基建未接线** | `worker_pool.rs`/`fleet.rs`/`fleet_transport.rs` 已实现但"待主控接线"（文件头自注） | Codex 0.144 多代理并发控制（agents 池/per_agent_tool_calls/rate_limit_tpm + 预算硬上限安全收尾） | 把 fleet 接进主循环：工具 `spawn_agents`（带并发上限/预算/超时），explore/subagent 升级为 fleet worker |
| A5-2 | 并行写冲突无隔离 | 只读并发（Semaphore 4），写串行但无 worktree | 2026 共识：subagents without worktree isolation = merge hell | 远期：写型子代理各自 git worktree |
| A5-3 | 子代理深度/轮数硬限 | MAX_SUBAGENT_DEPTH=2、子会话 12 turns（subagent.rs:12/45） | 合理起点 | 按任务类型放宽 explore 深度 |

### A6 权限/审批/沙箱

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A6-1 | **无会话级临时授权** | `remember_rule(&tool, &args, None)` 只生成永久规则（server lib.rs:1862）；`expires_at_ms` 机制在但无人用 | Claude Code 三档：单次/本会话/总是 | 审批卡加「本会话内允许」→ `expires_at_ms = session_end`；CLI/TUI 补 remember 选项（现只有 y/N，main.rs:2614-2624） |
| A6-2 | 工具无能力声明 | 审批按 Level(Read/Write/Execute/Inject) 分级 | Codex 0.144 writes 模式：工具元数据声明只读/可变，只读自动执行；策略即代码（path/branch/capability match） | 工具注册表加 `mutating: bool` 元数据；「只读自动放行 + 写入确认」作为新审批模式选项 |
| A6-3 | run_command 默认 JobOnly | tools.rs:1061-1069 `JobOnly + allow_degraded`，未开 AppContainer 网络隔离 | 桌面 Agent 常驻，网络白名单价值高 | 设置页加沙箱档位（None/Job/AppContainer），命令类默认提 Job+AppContainer，失败再降级并审计 |
| A6-4 | Linux/macOS 沙箱 Unsupported | sandbox.rs:377-395 仅探测 | 桌面产品当前 Windows-only，低优先 | `/health` 已暴露，维持"不假装隔离"即可 |

### A7 记忆

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A7-1 | 无向量召回 | `SemanticMemory` 倒排+token 重叠（memory.rs:139-156） | CodeBuddy Memory 自动记录偏好，越用越懂你 | 本地 embedding（ONNX，与 OCR 打包方式一致）做 `recall()` 召回，无模型优雅降级 |
| A7-2 | 无用户偏好自动沉淀 | experience_store 只归因技能 | Claude Code/CodeBuddy 均有用户级记忆 | 会话结束自动提取「偏好/惯例」入记忆库（免审批、可查看、可删除） |

### A8 任务自动化

| # | 问题 | 现状证据 | 对标差距 | 修改方向 |
|---|---|---|---|---|
| A8-1 | 无定时/后台任务 | 无 cron/schedule 概念 | 2026 标配（Hermes 类主打；Claude Code 也有后台任务） | 桌面常驻进程天然适合：`schedule` 配置（时间/事件触发）+ 后台任务列表 UI + 预算硬上限；依赖 A5 预算熔断已就绪 |
| A8-2 | 附件仅路径注入 | `/session/{id}/attachments`（server lib.rs:262）只把路径塞进 prompt | 应与 A1-2 打通：图片真正进上下文 | 同 A1-2 |

---

## 二、B 系列：用户体验问题（overlay / 桌宠 / 小工具）

### B1 可靠性缺陷（阻塞级，最优先修）

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B1-1 | **turn_failed 静默忽略 → UI 卡死**：busy 永真、发送按钮永久禁用、无错误气泡 | app.js:1981-1984 switch 无 turn_failed case；owoFinishTurn 仅在 final/bridge_error 调用（app.js:1659-1664） | 补 turn_failed 分支：错误气泡 + busy 复位；switch 加 default 兜底 finish |
| B1-2 | **回复重复显示两遍**：token_delta 流式气泡 + final.text 完整气泡并存 | app.js:1930-1939 vs 1968-1973；pet-chat.js:205-211 只删空气泡 | final 到达时若已有流式气泡则替换而非追加 |
| B1-3 | **同一工具调用两张卡**：tool_use 追加"执行中"卡，tool_result 再加一张新卡 | app.js:1941-1950 每次 appendChild | tool_result 按 call_id 更新原卡片（状态翻转+结果填充） |
| B1-4 | 409 显示成"引擎连接中断"，误导 | client.rs `map_ureq` → Http{409} → 统一文案（app.js:1975-1977） | `is_unreachable()/is_unauthorized()` 分类文案；409 → "上一轮还在跑 + 中止并重开"按钮；跨窗口（主面板/pet-chat）共享 busy 状态通知 |
| B1-5 | 审批 300s 倒计时前后端不同步 | 前端硬编码 300s（app.js:1734），服务端超时 Deny 后无事件回传 | 引擎补 `permission_resolved` SSE 事件；前端收到即销毁卡片 |
| B1-6 | Esc 直接关面板，聊天中误触 | app.js:2327-2330 | 输入框聚焦时 Esc 只清空输入 |

### B2 信息呈现

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B2-1 | **无 markdown/代码高亮**：`bubble.textContent = text` 直出 | app.js:1225-1233；全 ui/ 无渲染库 | 引入 marked + highlight.js + DOMPurify（Tauri WebView2 可本地打包资源，无 CSP 风险）；代码块带复制按钮 |
| B2-2 | **工具结果黑盒**：只显示"已执行/错误" | app.js:1693-1696 | 工具卡展开可见截断结果（复用轻环 `<details>` 模式 app.js:1235-1251）；失败显示 stderr 摘要 |
| B2-3 | diff 非行级、截尾 400 字符 | app.js:1890-1892 `before.slice(-400)+"\n↓\n"+after` | 行级 +/- 着色（前端 diff 库或后端给结构化 diff）；支持单文件回滚（引擎 revert 已支持） |
| B2-4 | plan_update / reasoning_delta / turn_stats 事件被忽略 | owo-bridge types.rs:96-164 无 PlanUpdate 变体，落 Other | bridge 补事件类型；主面板渲染计划进度条与思考折叠区（引擎侧 plan SSE 已就绪） |

### B3 会话管理（后端就绪、前端零调用）

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B3-1 | **多会话列表/切换/fork/rewind 无任何 UI** | `owo_sessions`/`owo_use_session` 命令就绪（owo_bridge.rs:424-454），ui/ 零调用 | 侧栏会话列表（标题/时间/归档/pin），复用引擎 /sessions 全套 REST |
| B3-2 | 历史回放丢工具卡与审批记录 | loadOwoHistory 只回放纯文本（app.js:1538-1557） | 按 messages_json 完整重建（工具卡/计划/审批记录） |
| B3-3 | pet-chat 重开空白 | pet-chat.js:7 明确不回放 | 与 B3-2 共用回放逻辑 |

### B4 设置与引导

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B4-1 | **remember 权限规则无管理界面**：一旦记住无法查看/撤销 | app.js:1721 注释承诺"可在设置中清除"，设置页无此区（index.html:88-179） | 设置页加规则列表（工具/模式/有效期/删除）；对应 A6-1 会话级选项 |
| B4-2 | **引擎分发断层**：exe 路径硬编码开发机目录、bundle.active=false，普通用户装不了 | owo_bridge.rs:100-106 硬编码 `D:\working OWOWOWOWOWO\...`；tauri.conf.json:74 | bundle 启用 + 引擎随安装包分发（NSIS 侧车）或首次运行下载器；"未找到引擎"给出行动指引 |
| B4-3 | 无 onboarding：首启零引导 | 面板 hidden 启动；唯一教程是桌宠 tooltip | 首启向导 3 步：配 Key → 启动引擎 → 热键介绍 |
| B4-4 | 剪贴板后台监听无开关（隐私） | widgets.rs:938-970 无条件 1.5s 轮询 | 设置页开关 + 例外名单；默认关闭改手动开启 |
| B4-5 | 无沙箱/权限模式选择 | 设置页无（沙箱=启动参数硬边界 owo_bridge.rs:260-262） | 对应 A6-3 落地后暴露到设置页 |
| B4-6 | 语音按钮依赖 WebView2 Web Speech 基本不可用 | app.js:1474-1480 | 移除或改 Whisper 本地侧车 |

### B5 快捷操作

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B5-1 | **主面板无呼出热键**：toggle_panel 无 RegisterHotKey 绑定 | main.rs:77 | 绑定全局热键（默认 Ctrl+Alt+Space 已被划词占用则另选，如 Ctrl+Alt+D），面板收起后一键召回 |
| B5-2 | 热键不可配置、冲突静默失败 | hotkeys.rs:64 仅 eprintln | 设置页热键配置 + 冲突时托盘气泡提示（project-status.md 待办已列） |
| B5-3 | 热键标签与实际绑定不符 | app.js:68-73 标 Ctrl+Alt+V=read_clipboard，实际开剪贴板小工具（hotkeys.rs:56） | 文案对齐 |

### B6 桌宠

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B6-1 | pet-chat 无法回答引擎提问：user_question 只提示"去主面板" | pet-chat.js:202-204 | pet-chat 内联输入框直接回传（引擎 ask_user 通道已通） |
| B6-2 | 审批卡可能被收起/遮挡 | pet-chat alwaysOnTop:false（tauri.conf.json:57）+ blur 400ms 自动收起（pet-chat.js:266-276） | 有待审批/待回答时窗口置顶并禁用自动收起 |
| B6-3 | **轻环状态联动断裂**：rewrite/qq 直接写 Mutex 不广播，桌宠无反应 | rewrite.rs:138/149/155、qq.rs:104/115/119；pet.js:514 已纯事件驱动 | 改走 broadcast_pet_status（统一事件源） |
| B6-4 | 双聊天入口心智混乱（主面板 vs pet-chat 能力不对等） | pet-chat 无 diff/无 question | 近期：能力对齐；远期：桌宠单击开主面板，pet-chat 只做轻交互 |

### B7 其他体验债

| # | 问题 | 现状证据 | 修改方向 |
|---|---|---|---|
| B7-1 | 轻环闲聊+本地后端直接报错，无引导 | agent.rs:73-75 | 报错时附"切到重任务模式/配置云端"按钮 |
| B7-2 | 轻环 DenyAll 静默拒、无流式无取消、10 步上限用户无感知 | engine.rs:43/83、agent.rs:111 | 输入区模式徽标注明限制；轻环仅保留离线改写（架构决策已定），闲聊路径逐步迁重环 |
| B7-3 | ime.html/ime.js 死代码（调用不存在的命令） | ime.js:19/59；main.rs:30-32 已 cfg 排除 | 删除或接入 assistant-ime |
| B7-4 | 只有暗色、无 i18n、无障碍缺失（chat 无 aria-live） | styles.css:4-43 硬编码 | 主题跟随系统 + prefers-color-scheme；aria-live 补齐；i18n 远期 |
| B7-5 | 桌宠 M6 主动搭话未做；M4 双击急停默认行为不可配 | docs/pet-agent-integration.md | 按 pet 方案排期执行 |

---

## 三、分批次修改计划

> 原则：先修"卡死/误导/重复"这类可信度缺陷，再做呈现升级，然后补模型层能力，最后扩生态。每批可独立验收。

### 批次 1：P0 可靠性速修（1~2 天）
- B1-1 turn_failed 处理 + busy 复位
- B1-2 重复消息修复（final 替换流式气泡）
- B1-3 工具卡合并更新（按 call_id）
- B1-4 错误分类文案 + 409「中止并重开」
- B1-6 Esc 误关面板
- A4-3 压缩失败 compact_truncate 兜底
- 验收：人为杀引擎/断网/制造 409/触发超时，UI 均不卡死且有正确提示。

### 批次 2：呈现与会话（3~5 天）
- B2-1 markdown + 代码高亮 + DOMPurify + 代码块复制
- B2-2 工具结果可见（折叠展开）
- B2-3 行级 diff + 单文件回滚
- B2-4 plan_update/reasoning_delta 渲染（bridge 补事件类型）
- B3-1 会话列表/切换 UI；B3-2/B3-3 历史完整回放
- 验收：跑一个真实多文件任务，全程（计划/工具/思考/结果/diff）可见、可回溯。

### 批次 3：模型层（1~2 周）
- A1-2 ChatMessage 多模态（ContentPart）+ 附件入上下文
- A1-1/A1-3 AnthropicProvider + prompt caching
- A1-4 NO_PROXY
- 验收：贴截图提问能答；ANTHROPIC 接入 10 轮带工具任务；第二轮 cache_read_input_tokens > 0。

### 批次 4：权限体验与扩展点（1 周）
- A6-1 会话级授权三档（单次/本会话/总是）+ CLI remember
- B4-1 权限规则管理 UI
- A6-2 工具 mutating 元数据 + 「只读自动放行」模式
- A6-3 沙箱档位设置（含 B4-5）
- A2-1 Hooks 系统（PreToolUse/PostToolUse/UserPromptSubmit/Stop/PreCompact，exit 2 阻断）
- 验收：首次 cargo test 弹审批→本会话记住→不再弹；rm -rf 永不可记；hook exit 2 能阻断并把 stderr 回喂模型。

### 批次 5：代码工程能力（1~2 周）
- A3-1 LSP 侧车工具（definition/references/symbol）
- A3-2 multi_edit + apply-patch 容错
- A3-3 web_search/web_fetch API 工具
- A3-4 git 只读工具包装
- A4-1/A4-2 压缩决策骨架 + keep_recent 按 token
- 验收：在大型仓库做定位/重构任务，工具命中率与 token 消耗对比基线改善 ≥30%。

### 批次 6：多代理与自动化（2~4 周）
- A5-1 fleet/worker_pool 接线（并发上限/预算硬上限/安全收尾）
- A8-1 定时/后台任务（schedule + 任务中心 UI）
- A2-2 MCP resources/prompts/roots
- 验收：3 个并行子代理完成 fan-out 任务，单代理失败不影响其余；定时任务次日可查执行记录。

### 批次 7：分发与产品化（2 周，可与 5/6 并行）
- B4-2 引擎随安装包分发 + bundle 启用 + 引导文案
- B4-3 首启 onboarding
- B5-1 主面板呼出热键；B5-2 热键可配置
- B4-4 剪贴板监听开关；B6-1/B6-2/B6-3 桌宠补全；B7-3 死代码清理
- 验收：全新机器安装→首启向导→贴图提问→审批→diff 回滚，全流程无开发者路径依赖。

### 远期池（择机排期）
- A7-1/A7-2 记忆向量与偏好沉淀；A2-3 MCP 认证；A5-2 worktree 隔离；B7-4 主题/i18n/无障碍；B7-5 桌宠 M6；Rime 集成；NSIS 合并打包（含 E1.8 真机联调）

---

## 四、参考基线（2026-09 调研摘要）

- **Claude Code**：Hooks（PreToolUse/PostToolUse/Stop 等，非零退出码阻断）；Subagent（独立 context window/工具权限，只回报结论）；MCP（tools/resources/prompts）；Skills（SKILL.md + 斜杠触发）；Memory；Rules（CLAUDE.md）；Plan mode；权限模式（default/acceptEdits/bypassPermissions）。
- **Codex CLI 0.144**：writes 审批模式（工具能力声明：只读自动/写入确认；作用域单次/类型/路径/项目 codex.yml 策略即代码）；MCP 交互式认证（OAuth2 + Keychain）；多代理并发控制（agents/per_agent_tool_calls/rate_limit_tpm + `--budget.hard-cap` 安全收尾）；Guardian 自动审查；模型压缩恢复（检查点+哈希校验）；apply-patch；AGENTS.md；诊断报告；沙箱分级 read-only/workspace-write/full-auto。
- **CodeBuddy**：Ask/Craft/Plan 三模式；自定义 Agent；Memory（自动偏好沉淀）；Rules（.codebuddy/rules）；Skills；MCP；多模型 API 接入；NES 前瞻补全；@ 引用上下文；多模态贴图/Figma；检查点；智能提交；Subagents；Hooks。
- **2026 共识**（TerminalBlog 能力矩阵）：截图视觉、cron/后台任务、multi-provider 路由、Git/PR 集成、plugins/skills、subagents+worktree 隔离、local-first 已成分水岭级能力；「subagents 无隔离 = merge hell」「cron 无预算上限 = foot-gun」。
