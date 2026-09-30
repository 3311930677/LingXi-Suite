# 项目状态交接文档

> 更新时间：2026-08-25 晚 | 仓库：https://github.com/3311930677/LingXi-DesktopAgent
> 用途：换电脑继续开发。新机器 clone 后按「环境搭建」操作即可接续。

## 一、当前进度总览

### 2026-09-30 收尾 10：桌宠改造完成（定位=agent 显示工具板块，阶段1-4 全量落地）

**阶段1 免构建迭代（核心痛点解决）**：
- 引擎新增 `/pet`（桌宠页面）与 `/pet-assets`（皮肤资产）静态路由：环境变量 `OWO_PET_UI_DIR` / `OWO_PET_ASSETS_DIR` 指向桌面端 overlay 的 `ui/pet` 与 `ui/assets`（未设置回落 `desktop/web/pet`）；磁盘直读 + 全局 no-store → **改桌宠前端只碰磁盘文件，刷新窗口即生效，零 cargo build**。
- overlay 的 pet 窗口加载本地 `pet-boot.html`（"连接引擎中"引导页）；pet-sync worker（2.5s）探测引擎：在线 → `navigate("/pet/index.html")`，离线 → 回 boot 页。remote capability（`pet-remote.json`，`http://127.0.0.1:*`）授权远程页面 IPC（拖动/审批/中止 10 个原生命令照常走 invoke）。
- `skin_file_url` 改根相对路径 `/pet-assets/skins/<id>/<file>`；托管引擎时注入两个 env（exe 旁 ui/ 目录优先，编译期路径兜底）；`prepare-engine-dist.ps1` 复制前自动停掉运行中的引擎（解锁 os error 32）。
- **实测**：`/pet/index.html` 200、皮肤图 200（218KB）；改 index.html 加标记 → HTTP 响应立即变化（全程未编译）。

**阶段2 定位瘦身（桌宠=显示板块）**：
- 删除 7 个 Rust 模块：`widgets`(45KB 小工具/剪贴板监听)、`market`(28KB)、`rewrite`(10KB)、`qq`、`agent`(本地 GGUF 推理)、`secret_store`、`hotkeys`；`panel.rs` 只留非激活样式/拖动/退出；`settings.rs` 收敛为桌宠+引擎桥配置（模型凭据归引擎侧 settings.json，OPENAI_* 注入移除）；主面板窗口与前端（index.html/app.js/styles.css 等 -213KB）退役，托盘只留桌宠显隐/工作台/退出。
- overlay 编译 **28.4s → 14.5s**；依赖清理（assistant-*/lingxi-tools/zip/sha2/base64/uuid/futures/ureq 全删），windows crate 只补回 `Win32_Security`（JobObjects 的 CreateJobObjectW 需要，原先由 Cryptography 传递启用）。

**阶段3 进度实时化**：pet-sync worker 在线时拉 `/activity` 快照，**内容变化才** emit `owo://activity`；前端事件驱动 + 10s 兜底轮询（原 2.5s 盲轮询废弃；隐藏态不受 webview 节流）。

**阶段4 CI**：engine 分支 `engine-release.yml`（`engine-v*` 标签 → 引擎 zip）；desktop 分支 `desktop-release.yml`（`desktop-v*` → 引擎+桌宠 exe 按发布布局打包：exe 旁 ui/pet + ui/assets + owo-agent.exe）。仓库：github.com/3311930677/LingXi-Suite（desktop/engine 双分支）。

**已知边界**：手动直接启动引擎（不经桌宠托管）时需自行设置 `OWO_PET_UI_DIR`/`OWO_PET_ASSETS_DIR`，否则 /pet 404（桌宠场景不受影响——托管时自动注入）；桌宠前端最终视觉效果待用户真机确认。


### 2026-09-30 收尾 5：工作台「每次打开都停在工具页」+ 长页面排版优化

用户反馈两点：① 每次进入工作台都先看到「工作区/自动化」这类页面而不是会话；② 这些页面是一长条往下拉、找不到内容。

**① 根因（前端路由自钉）**：`initPanels()` 在无 hash 时默认 `mountPanel("notes")`，而 `mountPanel` 内部**无条件** `history.replaceState(null,"","#notes")` → 地址栏被钉上 `#notes`；下次打开/刷新 `applyDeepLink()` 命中该 hash → `openPanelById()` 里 `setToolsVisible(true)` 展开**整个工具视图**并让 `initPanels` 提前 return（会话视图不再渲染）。与 localStorage 无关（没有路由持久化）。

修复：`mountPanel(id, writeHash=true)` 增加开关，**引导时的默认挂载传 `writeHash=false`**；`initPanels` 首屏清掉地址栏残留 hash 并直接挂第一个面板（`if (location.hash) history.replaceState(null,"",location.pathname+location.search)`）。手动改 hash 仍走 `hashchange` → 深链不失效（实测 `location.hash="eval"` 仍打开 eval 面板 ✓）。

**② 排版优化**（工具视图：工作区/智能/自动化/插件 + 扩展面板）：
- **吸顶分组导航**（`#toolsJump`，`index.html` 工具视图顶部）：全部/工作区/智能/自动化/插件 五个胶囊按钮；点击 = 复用既有分组聚焦机制（`body.tools-group-X`）+ 滚动回顶，`clearCodexGroup`/侧栏分组按钮同步高亮（`syncToolsJump`）。
- **分组聚焦不再塌成 760px 单列**：`style.css` 的 `body.tools-open[class*="tools-group-"] #sidebar` 改 `repeat(auto-fit, minmax(420px,1fr))` + `max-width:1280px` → 宽屏两列。
- **卡片内长列表限高内滚**（`.list` / `#reminderList` / `#evalReport` / `#traceReplay`，`max-height:340px`），列表再长也不会把整页撑成"一长条"。
- **自动化表单「内容」字段补 `.tool-field-full`**（此前只占半列，按钮行满宽显得错位）。

**验证（playwright + 本机 Edge，1440×900）**：带 `#notes` 打开 → `location.hash` 为空、`body.tools-open=false`（**首屏落在会话** ✓）；点「自动化」→ `tools-group-automation` 生效、导航高亮、`navDisplay=flex`、`#sidebar` 两列 `420px 420px`、表单两列 `405px 405px`、内容字段 822px=表单满宽、**文档总高 900px（一屏放下，无需滚动）**；点「全部」→ 聚焦类清空、页面仍 900px；手动 `#eval` 深链仍能打开面板。`node --check` 通过（纯前端，引擎 `no-store` 已保证刷新即最新）。

**未做（后续可继续）**：见下方「收尾 6」——扩展面板的统一网格已落地。

### 2026-09-30 收尾 6：扩展面板统一卡片网格（`panels/*.js` 零改动）

7 个面板源码共 2000+ 行、各自注入 `<style>`、根容器类名不一（`.stack` / 自有类 / 直接挂在 `section` 下），逐个改 HTML 成本高且易错。改为**挂载后自动排版**：

- **`app.js::layoutPanel(root)`**（`mountPanel` 末尾调用）：取面板容器（优先单一根容器 `.stack`，其子元素太少时退回 `section` 自身——覆盖 notes/team/fleet/plugin-market），对每个子块调用 `panelBlockIsWide()` 判定：
  - 整行跨列（`.owo-span-all`）：H1~H4 / 按钮 / 表单 / 表格 / 文本域 / 预格式块 / 含表格·编辑区·代码块的容器 / 类名含 `sub|toolbar|actions|-head|-bar` 的块；
  - 其余按卡片入网格；**卡片占比 <20% 或卡片 <2 个则放弃分栏**（保持原单列，避免越分越乱）。
- **`style.css`**：`#panelRoot .owo-cols { display:grid; grid-template-columns:repeat(auto-fit,minmax(360px,1fr)); gap:14px }` + `.owo-span-all { grid-column:1/-1 }` + `.owo-tall { max-height:620px; overflow:auto }`；面板内 `ul/ol/.list` 一律限高 320px 内滚（短列表不受影响）。
- **自愈（ResizeObserver + rAF 去抖）**：面板内容异步加载后才长出来，因此持续检查——① 内容横向溢出（`scrollWidth > clientWidth`）的卡片改整行跨列；② 高度 >700px 的块（长表/图表/长编辑器）加限高内滚。

**验证（playwright + 本机 Edge，1600×900，逐个挂载 11 个面板）**：全部进入网格、**零横向溢出**；memory 单块失控 1414px → 限高后**面板 2327px → 1533px（-34%）**；team 500→437（-13%）、fleet 688→535（-22%）、goal 369→287（-22%）、workflow 604 / plugin-market 355；notes 116 / eval 231 / command 424 等本来就不长。`node --check` 通过（纯前端，刷新即生效）。

**结论与剩余**：卡片型面板已明显变短；`observability`（1820px）与 `about`（2064px）变长的主因是**内容本身密集**（无单个失控块、块多为 200~400px），布局手段已用尽——要进一步缩短需要精简内容或逐个重构面板（下一步可选）。

### 2026-09-30 收尾 7：面板分区目录（锚点侧栏）+ 长分区折叠

**分区目录（TOC）**——解决"长面板找不到内容在哪"：
- `buildPanelToc(section, container)`：分区标题（`.sub` / H2~H4）**≥3 个**的面板，在左侧生成吸顶目录（`section.owo-toc-layout` 两列网格 172px + 内容列；窄屏回退横排）。
- 点击平滑滚到分区（`scroll-margin-top: 64px` 让开吸顶分组导航）；**滚动联动高亮**：滚动发生在内层滚动容器（`body.tools-open #sidebar`，html/body 不滚）→ 用 **document 捕获阶段**监听（scroll 不冒泡但可捕获），`panelTocSync` 全局单例 + rAF 节流，联动阈值 90px（略高于吸顶条，点击跳转后该分区即为当前项）。
- 实测（memory，7 分区）：点击第 5 项 `#sidebar.scrollTop` 0→4821 ✓；滚到底 active=最后分区、回顶 active=第 1 分区 ✓；observability 10 项、about 7 项；零横向溢出。

**长分区折叠（内容级"精简"的安全做法，不删内容）**：
- `addSectionFolding(headings)`：某分区（标题→下一标题之间）块总高 >700px 且 ≥2 块时，标题尾部出现「收起/展开」（`.owo-fold`，折叠态加 `.owo-folded { display:none !important }`）；幂等（已加不重复）。
- 挂载时 + ResizeObserver 复查（内容异步加载后才长出来）。
- **实测结论**：memory/observability/about 逐分区测量后**没有任何 ≥700px 的分区**（高度来自许多 100~620px 小分区堆叠）→ 折叠按钮不出现是**预期行为**；该功能留作安全网（将来出现超长分区——如超长 trace 列表——会自动出现「收起」）。这两个面板要再短只能做内容级重构（合并分区），已向用户说明。

### 2026-09-30 收尾 9：桌宠交互简化为「点击即菜单」

用户反馈「单击双击记不清楚」——交互模型从「单击无操作 / 双击停止或逗玩 / 右键菜单」收敛为**一条规则**：**点桌宠（左键单击或右键）就弹出操作菜单**，菜单按当前状态列出可做的事（引擎信息行 / 待审批允许·拒绝 / 中止当前任务 / 打开工作台 / 隐藏桌宠），再点一下或点别处关闭。删除双击判定（clickTimer）、双击紧急停止（能力保留在菜单「中止当前任务」）与逗玩彩蛋（pokeReact/POKE_LINES/`jump` 动画）；拖动移动 + 拖远摸头保留（无按键记忆负担）；pet.html tooltip 同步为「点我打开操作菜单」。重建 overlay 并随行拉起引擎（构建时 engine-dist 被运行中引擎锁定，需先停进程再编译）。

### 2026-09-30 收尾 8：自动化迁入扩展面板体系（统一网格 + TOC）

**背景**：用户发现「自动化」页面与其他功能页不一致——根因是**两套 UI 体系并存**：自动化是工具视图里的静态卡片（index.html 手写表单 + app.js 零散函数），而其余功能页是 `OwoPanels` 模块（挂载走 `layoutPanel` 统一网格 + 分区目录）。用户拍板迁移（方案 A）。

**改动**：
- 新建 `panels/automations.panel.js`（第 12 个面板，PANEL_ORDER 位列笔记之后）：新建任务表单（间隔/每天/单次 × 提醒/跑任务，复用全局 `.tool-form-grid`）+ 任务列表（启停/记录/删除，A8-1 执行记录展开原样保留）+ 提醒（列表 + 清除）。三个 `.sub` 分区 → 自动获得 TOC。
- `panelHelpers()` 补 `call(path, options)`（无 body 的 POST——toggle/clear 需要，此前散用 `api(..., {method:"POST"})`）。
- app.js 删旧函数（describeSchedule/describeAction/refreshAutomations/createAutomation/refreshReminders，-146 行）+ 旧监听 + **全局轮询**（automations 10s/reminders 5s 移入面板，仅挂载期运行、元素卸载自停——不再空转）。
- index.html：自动化卡片换为入口卡（「打开自动化面板」按钮 → `openPanelById("automations")`）；加 script 标签。style.css 死选择器 `#reminderList` → `#owo-aut-reminders`（保留 340px 限高滚动）。

**验证（真机）**：`node --check` 双 JS 通过；浏览器端到端——挂载/TOC 3 项/零横向溢出/创建任务（列表出现「每 3600 秒 ｜ 提醒 ｜ 启用」+ 状态行「已创建」）/记录按钮/删除清理，全通过；入口按钮点击正确挂载面板；导航 12 项含「自动化」；控制台零错误。

### 2026-09-30 收尾 4：桌宠 ↔ 工作台接线纠偏（单向误触 → 双向可控，消除「两只桌宠」）

用户反馈三点：单击桌宠极易误触打开工作台、方向反了（应「工作台控制桌宠」）、屏幕上同时出现两只桌宠。

**根因**：① 桌宠单击（`apps/overlay/ui/pet.js` 240ms 计时器分支）`invoke("open_workbench")`；② 工作台是引擎静态托管的 Web 页（`ServeDir::new(desktop_web_dir())`），与 Tauri 桌宠**零通道**——它自己在页面里内嵌了一份「LingXi 移植版桌宠」（`desktop/web/pet.js` + `index.html#pet`，显隐存浏览器 `localStorage`），与桌面端 `settings.pet_visible` 完全独立 → 两只叠加。

**改动**：
- **引擎新增 A8-3 桌宠通道**（`owo-agent-server`）：`AppState.pet_state: Mutex<PetState>`（desired / actual / 心跳时刻）；`GET /desktop/pet`（状态 + `overlay_online` = 15 秒内有心跳）、`POST /desktop/pet`（写期望值）、`POST /desktop/pet/report`（桌面端心跳，回传期望值）；OpenAPI 三条已登记。GET 在桌面端在线时以实际值回写期望（自愈，避免旧期望把桌宠「复活」）。
- **桌面端（overlay）**：bridge 加 `report_pet_visible` / `pet_state` / `set_pet_desired`；新命令 `owo_pet_sync`（上报实际 + 应用期望差异，复用 `set_pet_visible` 落盘）；`ui/pet.js` 轮询循环每 2.5 秒调用一次（与 `/activity` 同节奏）→ 工作台开关约 2.5 秒内生效。
- **桌宠交互纠偏**：单击不再打开工作台（计时器仅用于单击/双击区分）；「打开工作台」保留在右键菜单与托盘；`pet.html` / 主面板文案同步。
- **工作台（`desktop/web`）**：删除内嵌桌宠（`pet.js`/`pet.css` 文件 + `index.html#pet` DOM + `app.js` 里 `petSetState`/`applyPetVisibility`/`initPet` 及 9 处调用点）；「设置 → 外观 → 显示桌面桌宠」开关改为读 `GET /desktop/pet`、写 `POST /desktop/pet`，并显示桌面端在线状态（离线时提示「开关会在它启动后生效」），状态每 15 秒刷新。素材 `assets/pet/skins/` 暂留（未引用，后续清理或复用）。

**验证（真机）**：`POST /desktop/pet {visible:false}` → 5 秒后 `actual:false` 且桌面端 `%APPDATA%\lingxi\settings.json` 的 `pet_visible=False`（桌宠隐藏）；`{visible:true}` → 恢复显示；`overlay_online:true` 心跳正常。bridge 18 测试、server 28 测试、overlay clippy 零警告、`node --check app.js` 通过。

**注意（改这两层的前置知识）**：工作台由引擎 ServeDir **直读磁盘** `desktop/web/`——改完刷新即最新（引擎现已全局 `Cache-Control: no-store`，不会再被浏览器缓存骗）；但桌面端 UI（`apps/overlay/ui/*`）会编译进 overlay.exe，改动必须 `cargo build` 并重启桌宠（且构建前要先停引擎进程，否则 `target/debug/resources/owo-agent.exe` 被占用导致 tauri-build 失败）。

**踩坑修复（开关「点了没反应」的三个真凶，均实测复现+修复）**：
1. **浏览器启发式缓存**：ServeDir 不发 `Cache-Control`，Edge 按 Last-Modified 启发式缓存 `app.js` → 后端改了前端没生效。修复：引擎最外层加 `no_store_middleware`（API + 静态一律 `no-store, no-cache, must-revalidate`）。
2. **自愈逻辑吞命令**：`GET /desktop/pet` 原本只要「桌面端在线且 desired ≠ actual」就把 desired 回写成 actual——工作台刚写入期望、桌面端还没跟进（心跳 5s）时，命令被立即抹掉，表现为开关拨不动。修复：**仅当实际值时间戳更新**（说明用户确实在桌宠侧改过）才回写。
3. **隐藏态心跳不可靠**：桌宠窗口隐藏后 WebView2 节流其定时器 → 纯前端 `pollActivity` 轮询停摆 → 工作台显示「桌面端未运行」且无法再从隐藏态唤醒。修复：overlay 增加 Rust 侧常驻线程 `spawn_pet_sync_worker`（5 秒一次心跳 + 应用期望值），前端轮询降级为「快速通道」。

**验证（真机 + 浏览器自动化）**：用 playwright-cli 驱动本机 Edge 打开 `http://127.0.0.1:4096/` → 控制台 0 报错；页面状态 `桌面端在线 · 桌宠已隐藏`；点击开关 → 开关即时拨动、约 5 秒后 `actual:true` 且桌面端 `settings.json pet_visible=True`（桌宠真的出现）；再点 → 隐藏，且**隐藏态下 `overlay_online` 仍为 true**（Rust 心跳线程生效）。

### 2026-09-30 收尾 4b：多对话并行被打断的修复（全局单例 → 按会话隔离）

用户反馈：两个对话不能同时跑，启动一个另一个就被打断。**根因**：工作台的「运行中」状态是全局单例（`state.reading`），而并行回合的基建（`activeTurns: Map<sessionId, {controller, startedAt}>`、`writeTargetSid` 分流、会话列表 running 徽标）早已就绪——全局守卫把设计好的能力堵死了：

1. 切到会话 B 按回车 → `sendPrompt()` 的 `if (state.reading) return` → **发送被丢弃**（无任何提示）；
2. 点发送钮（A 在跑，按钮全局显示成"停止"）→ `abortTurn()` → B 没有回合 → 落到 `else if (state.abortController)`（**A 的控制器**）→ **A 被掐掉**；
3. 任一回合的 `finally` 会 `state.turn=null / state.reading=false / stopRunStatus() / resetRunBlocks() / hideApproval() / settlePendingQuestion("aborted")` → 后台会话结束会清掉前台会话的流式句柄与审批卡。

**修复（desktop/web/app.js）**：新增 `currentTurn()`（按 `state.sessionId` 取活跃回合）并替换全部全局守卫——发送钮双态、`sendPrompt` 守卫（改为 toast 提示"本会话已有回合在运行"）、`abortTurn`（**删除全局 controller 回落**，只中断当前会话）、`finally`（视图级收尾仅当 `state.sessionId === turnSessionId`；`state.reading = activeTurns.size > 0`）、`selectSession` 切回时按该会话恢复运行状态条（`state.runStartedAt = turn.startedAt`）。

**验证**：引擎侧并发 API 实测 `/activity` 同时 2 个活跃会话、各自独立完成；浏览器自动化（playwright + 本机 Edge）实测：A 会话在跑时切到 B，**发送钮显示"发送"（此前是"停止"）**，B 发起后 **`state.activeTurns` 峰值 = 2**（两回合真并行），A/B 各自产出工具调用 + 完整回复，**A 未被中断**。

### 2026-09-30 收尾 4c：推理档位（reasoning_effort）实测结论

链路：设置页档位 → `settings.reasoning_effort` → `POST /settings` 调 `apply_reasoning_env()` → 进程环境 `OWO_REASONING_EFFORT` → `OpenAiCompatibleProvider` 每次请求前读取 → 请求体 `reasoning_effort`（仅 minimal/low/medium/high；默认不发送该字段，避免兼容端点 400）。

**实测（模型 `deepseek-flash`，同一提示词，从 SSE 抓 `turn_stats` + `reasoning_delta`）**：

| 实验 | 结果 |
|---|---|
| 简单题（3人3天3桶水）× 5 档各 1 次 | 全部答对 27；思考字数 148–317、输出 103–174 tokens —— **单次采样完全看不出规律**（"极简"反而最多） |
| 简单题 × 3 档各 3 次（重复采样） | **单调趋势成立**：`minimal` 输出 105.7 tokens / 思考 183 字；`default` 126.7 / 281；`high` **182.3 / 451**（约为 minimal 的 2~2.5 倍）→ **参数确实生效** |
| 难题（1≤n≤100 且 n(n+1) 被 6 整除的个数，答案 66）× minimal/high 各 2 次 | 四次的答案**都正确**（66）；但 `high` 第 2 次**失控**：**13884 tokens / 303 秒**（约为同档位另一次的 35 倍 tokens、100 倍耗时），答案仍正确 |

**结论**：档位影响的是**思考量 / token 成本 / 延迟**，不保证"更准"；同档位内噪声极大（单次对比会得出相反结论），需要重复采样；`high` 存在偶发**长尾失控**（成本/延迟代价真实）。建议日常用默认或 medium，难题再临时切 high。

**注**：`GET /settings` 不返回 `base_url`（脱敏），验证时只打印 model；调用 SSE 端点时 PowerShell 5.1 必须加 `-UseBasicParsing`（否则 IE 解析器对 `text/event-stream` 抛 NullReferenceException）。

### 2026-09-30 收尾 3：CLI 对标 Codex（审批三档 / 输入体验 / 渲染统计 / 管道模式）

CLI 此前「后端能力极全、交互层几乎为零」。本次补齐四组（全部真机冒烟通过）：

**B1 审批三档 + TUI 弹窗 + FIFO 修复**
- `ConsoleApprover` 与新增 `TuiApprover` 共用 `apply_approval_scope(agent, workspace, request, scope)`：`y=允许一次 / a=本会话内允许 / s=总是允许 / n=拒绝`——会话级写 `remember_session_rule`（内存），总是允许写 `remember_rule` 并**持久化到 workspace settings.json**（与 server `persist_permission_rules` 同口径）；危险命令两类均返回 None → 提示「不可记住，仅本次允许」。
- **修复此前只写不读**：`build_agent_with_mcp` 新增 `permission_rules` 参数，启动/重建时 `policy.replace_rules(...)` 灌入 settings 规则（此前选「总是允许」重启即失效）；6 个调用点同步（serve 路径原本手工 replace_rules，已去重）。
- TUI：新增**模态审批弹窗**（`Clear` + 居中块：档位/工具/原因/入参 400 字 + 键位提示 + 待处理计数）；`pending_order` 由 `Vec::pop()`（**LIFO 乱序**）改 `VecDeque::pop_front()`（FIFO，失效条目自动跳过）；按键 y/a/s/n，回执区分档位。
- 测试：`permission_request_queues_approval_and_responds`（档位通道）、`approvals_are_answered_in_arrival_order`（FIFO）。

**B2 终端 Markdown + 回合统计**
- 新模块 `markdown.rs`（自实现、零新依赖）：`MarkdownStream` 按行缓冲的**流式**渲染——标题/列表/引用/分隔线/行内 code/粗体/链接着色，代码围栏内整段原样（避免 `*`/`_` 被误解析）；`strip_markdown` 供 TUI（ratatui 不消费 ANSI）。4 项单测（含 token 边界拆分）。
- REPL/`turn` 的 `EventPrinter` 接流式渲染；TUI 流式与最终文本走 `strip_markdown`。
- `turn_stats`：`⏱ 12.3s · ↑1.2k ↓0.8k tokens · $0.004`（成本按 `OWO_MODEL_*_PRICE_PER_MTOK` 估算，未配置则省略）；REPL 收尾 / `turn` 结束 / TUI 完成行三处展示。

**B3 输入体验**
- REPL：rustyline 接 `ReplHelper`（Completer + Highlighter + Hinter + Validator + 显式 `Helper`）——**Tab 补全 `/命令` 与文件路径**（`completion_candidates`：命令前缀 / 目录项 / `@` 前缀保留；多候选补最长公共前缀）；**行尾 `\` 续行**（多行输入）；历史沿用 history.txt。
- TUI：**Tab 补全**（唯一候选直补，多候选列前 6 个到状态栏）；**↑/↓ 输入历史**（200 条、连续去重）；**Shift+Enter / Ctrl+J 换行**；模式切换默认键由 Tab 改 **F2**（`settings.keybinds` 可覆盖）；提示与状态栏同步。

**B4 管道/CI 友好**
- `turn` 新增 `--print`（仅最终文本）与 `--json`（`{final_text, steps, duration_ms, usage{prompt/completion/total/cost_usd}, diffs, session_id, trace}`）；`--prompt -` 或省略且 stdin 非终端时从 stdin 整段读入；两种静默模式经 `EventPrinter::quiet()` 屏蔽流式回显与状态行。

**验证（真机冒烟）**：`turn --help` 新参数齐全；`"…what is 1+1?" | owo-agent turn --print` → 仅输出 `2`（1.4s）；`--json` 输出完整结构；默认模式渲染出 `• 列表 / ┌─ rust ─ 代码块 / [统计] ⏱ 1.4s · ↑5.7k ↓97 tokens`。`cargo test -p owo-agent-cli` **17 通过**；cli clippy `--all-targets` 零警告；core lib **378 全绿**无回归。

### 2026-09-30 收尾 2：OwO 工作台补齐 + 桌宠改「引擎进度控制台」

**引擎：`GET /activity`（活跃回合快照）**——`AppState.activities`（session_id → {phase, tool, request_id, reason, started_at}）由 turn 事件回调 `begin/update/end_activity` 维护（thinking / speaking / tool / waiting_approval；Final 或回合失败即清理）；`/activity` 返回 `{active:[{…, title, workspace}], pending_approvals}`，OpenAPI 已登记。用途：桌宠与任何外部面板的只读进度源。

**工作台（`desktop/web/`）补齐三处**：
1. `TOOL_LABELS` 补 `git_status/git_diff/git_log/fan_out_subagents/update_plan`，`toolStepSummary` 补对应入参摘要（查询词 / URL / 路径 / 任务数）。
2. 审批终态：新增 `permission_resolved` 分支 → `markApprovalResolved` 立即出队（此前要等 5 秒轮询）并在时间线留「审批超时/作废」事件 chip。
3. 自动化：新建表单加「动作」（提醒 / **跑任务**）+ 内容；任务行「**记录**」按钮展开最近 10 次执行（`/automations/runs`）；**修复 `oneshot` → `one_shot` 的 serde tag 错误**（此前「单次」任务创建必失败）。

**桌宠（LingXi overlay）→ 引擎进度控制台**：
- 气泡跟随 `/activity` 进度（2.5s 轮询：思考中 / 用工具 / 回复中 / 等你审批）；**单击 = 打开 OwO 工作台**（新命令 `open_workbench`）。
- 右键 = 操作菜单：待审批一键**允许/拒绝**（直连 `owo_permission`，工作台审批卡同步收尾）、有回合时**中止当前任务**、常驻「打开工作台 / 隐藏桌宠」；顶部信息行显示当前会话与阶段。
- 删除与工作台重复的入口：pet-chat 对话窗（窗口 / 命令 / 文件 / `capabilities` 全删）、工具页「QQ 草稿」「选中改写」卡、托盘「小工具」子菜单与 5 个 widget 全局热键、设置页皮肤网格、市场「皮肤商店」tab；托盘新增「打开工作台」。保留但已无入口：改写/QQ 后端与 `Ctrl+Alt+Space` 热键（IME 集成按决定暂缓）。
- 新增 bridge `activity()` + 命令 `owo_activity`；`tray.rs` 菜单重建；`hotkeys.rs` 精简为「面板呼出热键」单绑定（`spawn_global_hotkey_worker`）。
- 验证：0.5s 粒度实测 `/activity` 抓取 `thinking → speaking` 流转、回合后自动清空；overlay `cargo build` 零警告；bridge 18 测试通过；桌宠与工作台真机联调通过。

### 2026-09-30 收尾：B1-5 审批终态同步 + 真实端到端验收

**B1-5 审批终态回执（补齐批次 1 遗留）**：此前审批只有 `permission_request`，**300s 超时/回合中止后卡片仍留在界面上可点**（点了只会得到无意义的回传错误）。现补 `SseEvent::PermissionResolved { request_id, source, allowed }`（与提问的 `user_answered` 对称；`source` = user/timeout/aborted，`allowed` 为最终决定）：
- `ChannelApprover` 加 SSE 发送通道，`decide` 判定来源（用户提交 / 超时 / abort 标志）并在清理 pending 后回执事件；契约登记与 `to_event` 事件名同步。
- 主面板：审批卡挂 `data-request-id` + `permissionCards` 索引；新增 `owoResolvePermission`（移除行动区、写终态文案、停倒计时——倒计时遇 `resolved` 即停）；本地点击路径先写 resolved，事件重复到达时幂等跳过。
- pet-chat：审批卡同样挂 `data-request-id`；`resolveApprovalFromServer` 关闭行动区并**重置待审批计数**（计数不归位会导致失焦永不收起）。
- 测试：protocol 新增 `permission_resolved` 序列化/缺省反序列化测试；新注册表完整性测试（19 个能力工具 + headless 白名单/黑名单）。

**真实端到端验收（本机引擎，无 UI）**：
1. `owo-agent serve --port=4097 --workspace=<tmp>`（新构建的 debug 引擎）→ `/health` 正常（`auto_approve:false`）。
2. `GET /openapi.json` 含新路由 `/automations/runs`；`POST /automations`（`action.kind=run_prompt`）→ 列表可见 → `toggle` 返回 `enabled:false` → `DELETE` 成功；`GET /automations/runs?task_id=…` 可用（空数组）；`GET /permissions/rules` 返回 `{rules:[],session_rules:[]}`；`POST /session` 建会话正常。
3. **真实模型回合**（环境已配置模型）：workspace 指向本仓库，prompt 要求「只读调用 git_status/git_log」→ 模型自主调用 `git_status`（返回分支 master + 10 项未跟踪变更）与 `git_log`（空仓库 → 工具优雅返回 `fatal: … no commits yet`），**只读工具免审批零 permission_request**，最终给出准确中文总结；全帧带 `v:1`。验证了「注册 → 模型可见 → 调用 → 执行 → 结果回喂 → 终态」全链路。

**构建验收**：overlay `cargo build` 成功（1m52s，产出可执行文件，含 tauri.conf resources 解析）；引擎 `cargo build -p owo-agent-cli` 成功并刷新 `engine-dist/owo-agent.exe`（66.4MB，供 `tauri build` 打包）。

**验证汇总**：`cargo test -p owo-agent-core --lib` **378 全绿**；server 全套测试通过；clippy core+server+protocol `--all-targets` 零警告；overlay `cargo check` 零警告；bridge 18；`node --check` app.js/pet-chat.js 通过。

**环境提示**：`owo-agent serve` 无模型配置也能启动（DeferredProvider 设计），适合无 key 的自动化验收；本机 `LingXi-DesktopAgent` 仓库尚无首次提交（`git_log` 返回 no commits yet），首次提交后 git 工具输出会更完整。

### 2026-09-30 批次 7：分发与产品化（对应 docs/agent-capability-gap-review-2026-09.md）

**B4-2 引擎随包分发**：
- `resolve_exe_path` 重排（删掉硬编码的开发机盘符）：`OWO_AGENT_EXE` 环境变量 > 用户配置（设置页手选）> 应用同目录 / `resources/` / `engine/` 子目录（安装形态）> 按 **build 时 crate 目录**推导兄弟仓库 `OwO/agent-sdk/target/{debug,release}`（开发形态；发布机上路径不存在自然跳过）。
- 设置页引擎卡新增「引擎程序」输入 + 「选择…」（新命令 `pick_engine_exe`，rfd 文件选择器）；`save_engine_options` 接受 `exe_path`（校验存在且为 .exe；留空=恢复自动探测）。
- `tauri.conf.json` `bundle.active=true` + `resources: {"engine-dist/owo-agent.exe": "resources/owo-agent.exe"}`；新增 `apps/overlay/scripts/prepare-engine-dist.ps1`（发布前把引擎拷进 engine-dist；纯 ASCII 兼容 PS5.1）；engine-dist 已进 .gitignore。未找到引擎时设置页给出可操作的引导文案（含开发机构建提示）。

**B4-3 首启引导（onboarding）**：`BackendSettings.onboarding_done`（serde default 零迁移）+ 命令 `complete_onboarding` + 视图暴露；首启浮层 3 步（①配置模型并显示「已配置 ✓/尚未配置」②启动引擎并显示运行态、一键拉起 ③热键说明 Ctrl+Alt+Space / Ctrl+Alt+D），完成或启动引擎后写标记不再出现；「去配置模型」跳设置页（不写标记，下次仍提示）。

**B5-1/B5-2 主面板呼出热键 + 可配置**：
- `panel.rs` 抽出 `toggle_panel_inner(&AppHandle)`（命令与热键共用；`toggle_panel` 命令签名简化）。
- `hotkeys.rs` 新增 `parse_hotkey`（Win32 组合语法：Ctrl/Alt/Shift/Win + 字母数字/F1~F24/space/enter/tab/backspace/esc，含校验）；widget 消息泵新增 `PANEL_HK` 注册与 dispatch（异步 spawn 调 `toggle_panel_inner`）。
- `settings.hotkey_panel`（默认 `Ctrl+Alt+D`）+ 命令 `set_panel_hotkey`（Rust 侧校验格式）+ 设置页输入与保存；**注册失败/格式无效时 emit `lingxi://hotkey-conflict`**，前端 showStatus 可见提示（不再静默 eprintln）。

**B4-4 剪贴板监听开关**：`widgets.rs` 加进程级 `CLIPBOARD_LISTENER_ENABLED` gate（关闭时线程空转、**完全不读剪贴板**）；命令 `set_clipboard_listener` 即时生效 + 持久化；启动时按设置初始化；设置页「窗口行为与快捷键」卡加勾选。

**B6-1/B6-2/B6-3 桌宠补全**：
- pet-chat 支持 `ask_user` 内联回答（`appendQuestion`：输入条 + 回车提交 `owo_answer`，回答后原位显示结果），不再只提示「去主面板回答」。
- 审批豁免：`pendingApprovals` 计数（审批卡出现 +1、响应 -1），>0 时失焦不自动收起——审批卡不会再被 blur 藏掉。
- 状态联动修复：`rewrite.rs`（preview 三处）与 `qq.rs`（draft 三处）从**直写 mutex 改为 `broadcast_pet_status`**（轻环任务时桌宠终于有 thinking/speaking 反应；两命令签名注入 AppHandle）。

**B7-3 死代码清理**：删除 `ui/ime.html|js|css`、`src/removed_ime_hook.rs`（14KB 编译期排除）+ main.rs 的 `#[cfg(any())]` 引用、app.js `owoSetPet` 死函数、源码目录误留的 `overlay.exe`（42MB 构建产物）；`owo_stop_service` 接入设置页「停止引擎」按钮（此前注册零调用）；`toggle_panel` 被 B5-1 热键复用（不再是死命令）。

**验证**：overlay `cargo check`（MSVC）通过零警告；`cargo test -p owo-bridge` 18 通过；`node --check` app.js/pet-chat.js 通过。真机验收待做：`tauri build` 产安装包（引擎随 resources 分发）、首启向导流程、Ctrl+Alt+D 呼出、剪贴板开关行为。

### 2026-09-30 批次 6：多代理与自动化（对应 docs/agent-capability-gap-review-2026-09.md）

**A5-1 fleet/worker_pool 接线**：
- `subagent.rs` 新增 `FanOutRunner`（owned + `'static`，为 `fan_out_cfg` 的闭包约束而设）与 `fan_out_subagents`：复用 `fleet::fan_out_cfg` 的**并发上限/单任务超时/整体时长预算/取消传播/部分成功仲裁**；子代理为**只读**（`Policy::read_only` 天然拒非 Read → 不写文件/不执行/不联网；写类任务仍串行走 `subagent`）；独立会话、独立失败、结果按输入序返回；取消标志缺省时内建。
- `tools.rs` 新增 `FanOutSubagentsTool`（`fan_out_subagents`）：tasks 2~6 条、`max_parallel` 1~4（默认 3）、`timeout_secs` 30~900（单任务超时；整体预算 = 2×）；**取消桥**——`select!` 循环每 150ms 检查主回合 abort 标志并置位 cancelled（主回合急停 → fan_out 停调度 + 在飞子代理 abort，已成功结果保留）。
- `ToolContext` 新增 `fanout: Option<FanOutRunner>` + `abort: Option<&AtomicBool>`（4 个构造点同步：agent.rs 主会话注入、tools.rs 测试 helper、tool_efficiency/mcp 测试）。
- 测试 4 项：并行+失败隔离（3 任务 300ms wall<750ms 且 2 成 1 败）／`max_parallel=1` 串行（峰值并发=1）／取消保留已完成／深度超限与空列表拒绝。

**A8-1 定时/后台任务**：
- `automation.rs`：`AutomationAction` 新增 `RunPrompt { prompt, session_id? }`；新增 `AutomationRun`（task_id/task_name/at/status/output）+ `runs` 持久化（封顶 500）+ `record_run`/`runs(task_id?, limit)` 查询；`fire` 改为返回动作（Reminder 照旧压列表，RunPrompt 由上层执行）。
- server：`CreateAutomationRequest` 支持 `action`（兼容旧 `reminder` 字段）；`run_automation_prompt`——续跑指定会话或新建独立会话，**审批策略 `AutoApprover{allow:false}`**（无人在场不能授权：Read 档免审批、Write/Execute/Inject 默认拒绝），输出截断 4000 字符，落 run 记录 + 审计；`start_automation_loop` 按动作分支（RunPrompt 后台 spawn，不阻塞调度）；新路由 `GET /automations/runs?task_id=&limit=` + OpenAPI。
- overlay：bridge 客户端 5 方法（含新 `delete_json` helper + 401 刷新重试）；5 个 Tauri 命令（注册齐全）；设置页「定时任务」卡——列表（调度/动作/最近执行/启停/删除）+ 新建（每天 HH:MM / 每 N 分钟 × 跑任务|提醒）+ 执行记录展开（最近 10 条）。
- 测试：automation 7 项（含 fire 动作分支、RunPrompt 不压提醒、runs 持久化/过滤/limit）。

**A2-2 MCP resources/prompts**：
- `mcp.rs`：新增 `McpResource`/`McpPrompt` + 客户端缓存字段；**能力协商条件拉取**（initialize 响应声明 resources/prompts 才请求，老服务器不受影响）；`reload_resources`/`read_resource`/`reload_prompts`/`get_prompt`（复用 request 分派 → stdio/HTTP 双传输自动可用）。
- `tools.rs` `register_mcp_extras`：每服务器至多 2 个**泛化工具**（`{server}_read_resource` / `{server}_get_prompt`，资源/模板目录进描述；逐资源开工具会撑爆工具表）+ 两个 adapter；`agent.rs` `register_mcp_extras` + `connect_mcp_server` 热连接时一并注册（插件启用路径自动生效）。
- 测试基建：mock stdio server 扩展 resources/list+read、prompts/list+get；新增 2 项集成测试（协商拉取+读取往返+泛化工具注册调用；Agent 热连接后可见工具含 extras）。

**验证**：`cargo test -p owo-agent-core` **lib 377 全绿**（+6）、mcp_tests 27、automation 7；`cargo check -p owo-agent-server -p owo-agent-cli` 通过；clippy core+server `--all-targets` 零警告。中途修复：宏 fragment `:json`→`:expr`、raw string 截断、regex 不支持 `\1`、mock 断言误匹配 system prompt、`&limit.to_string()` 临时值等。

### 2026-09-30 批次 5：代码工程能力（对应 docs/agent-capability-gap-review-2026-09.md）

**A3-2 multi_edit 批量编辑**（`tools.rs` 新 `MultiEditTool`）：单文件多处替换，`edits: [{old_str,new_str,replace_all?}]` **按顺序应用、原子生效**——先整体预解析参数，内存中顺序替换（后一处 old_str 匹配前一处替换后的内容），任一处失败（未找到/多处歧义）立即报「第 i/n 处替换失败，整批未应用」且原文件不动，全成功才 `ensure_snapshot` + 落盘；返回 applied 计数 + 最后一处修改点预览（复用 `locate_edit_preview`）。上限 20 个替换/调用。主注册表 + headless 双接入。

**A3-3 web_search / web_fetch API 工具**（新文件 `web_tools.rs`，取代浏览器抓搜索引擎形态）：
- `web_search`：配置 `BRAVE_API_KEY` 走 Brave Search API（`web.results` 解析），否则回落 DuckDuckGo HTML 端点（免 key，POST form）；`/l/?uddg=` 跳转链接还原 + 手写 percent_decode（UTF-8/`+`）；结果 title/url/snippet 三元组，无结果给换词提示。无需 Playwright 冷启动（亚秒级），不占浏览器会话。
- `web_fetch`：抓 URL → 3MB 响应上限（bytes_stream 累计）→ HTML 抽正文（去 script/style/noscript/svg/iframe、块级标签转换行、去标签、实体解码、空白压缩；JSON/plain 只做实体解码）→ max_chars 截断（默认 20k）。
- **SSRF 防护**：仅放行公网 http(s)——拒绝 localhost/.local/.internal、IP 字面量私网/回环/链路本地/CGNAT(100.64/10)/基准(198.18/15)，域名解析后逐一校验（防 DNS 指向内网）；HTTP 客户端复用 `build_model_http_client`（代理+NO_PROXY 同口径）。
- 纯函数全单测（SSRF 12 例、html 抽取、DDG 解析、Brave 映射、percent 解码），网络路径不做单测。

**A3-4 git 只读三件套**（新文件 `git_tools.rs`，`readonly_git_tool!` 宏生成）：`git_status`（porcelain + 分支/变更计数）、`git_diff`（工作区 vs HEAD，`staged=true` 看暂存区，可按文件过滤、context_lines 0-10）、`git_log`（oneline+decorate，limit≤100，可按文件过滤）。设计边界：**固定子命令白名单、不收模型拼任意参数**，无 commit/push（写操作保持 run_command 的 Execute 审批出口）；`-c core.quotepath=false`（中文文件名）+ tokio 超时 20/30s + 输出截断；git 不存在/非 git 仓库给可读错误。`Policy::level_for` 映射 **Level::Read**（免审批）+ 实现 ReadOnlyTool（并行通道），并接入 read_only() 子代理表。

**A4-1/A4-2 压缩决策骨架 + keep_recent 按 token**（`agent.rs`）：
- 压缩 prompt 重写为**决策骨架**结构（Markdown 小节：已完成/关键决策及理由/失败与教训/未完成/上下文备忘，≤800 字、禁编造）——纯散文摘要会丢「为什么这么做/踩过什么坑」，压缩后模型重蹈覆辙。
- `AgentConfig` 新增 `keep_recent_tokens: usize`（默认 20_000，构造点全用 `..Default::default()` 零迁移）；`maybe_compact` 切点改为 `compute_keep_start`（从尾往前按 token 累计选取保留段，keep_recent 条数降为兜底上限，最新消息无条件保留）→ 再 `align_keep_start` 对齐 tool 群组。此前按条数（20 条）在长工具结果/带图消息下会显著超预算。
- 3 项新单测：token 预算收敛、条数上限、最新消息必保留。

**A3-1 LSP 侧车——评估后推迟**：rust-analyzer/tsserver 进程管理 + JSON-RPC + 首次索引等待，且语言服务器二进制分发复杂（几十 MB/语言），与批次 7（引擎随包分发）强耦合。过渡：grep（regex 自动降级字面匹配）+ read_file 精准行读已可用；LSP 工具列入批次 7 后规划。

**验证**：`cargo test -p owo-agent-core --lib` **371 全绿**（+14）；`cargo check -p owo-agent-server` 通过（AgentConfig 构造点全部 `..Default::default()` 零迁移）；clippy core+server `--all-targets` 零警告。中途修复：宏 fragment specifier `:json`→`:expr`、raw string 被内容 `"`#` 提前终止（改 r##）、regex 不支持 `\1` 反向引用、titles 元组 (url,title)/(title,url) 顺序、`unwrap_or_default()` 不适用于 regex Match、`&limit.to_string()` 临时值生命周期。前端零改动（工具 specs 协议动态下发，权限卡/工具卡自动生效）。

**待办**：真机验收（大型仓库定位/重构任务对比基线命中率与 token 消耗 ≥30% 改善；`BRAVE_API_KEY` 质量对比 DDG）；web_search 免 key 路径被 DDG 限流时的降级文案是否足够；批次 6（多代理与自动化：fleet 接线、定时任务、MCP resources/prompts）。

### 2026-09-30 批次 4：权限体验 + Hooks 系统（对应 docs/agent-capability-gap-review-2026-09.md）

**A6-1 会话级授权三档**：`PermissionResponse` 加 `remember_scope`（`session`/`forever`，serde 可选字段非破坏）；Policy 新增 `session_rules` 内存临时表（Arc 跨 scoped_to 共享、不落盘重启失效），`remember_session_rule` 记忆 + `matching_rule` **会话表优先于持久表**（硬拒绝仍最优先）；bridge/协议镜像同步；`owo_permission` Tauri 命令加 `rememberScope`；主面板与 pet-chat 审批卡的勾选框升级为三档下拉（每次都问 / 本会话内允许 / 总是允许）。

**B4-1 权限规则管理**：引擎新 API `GET /permissions/rules`（持久+会话临时）与 `POST /permissions/rules/remove`（按 tool+pattern+scope 删除，持久删除同步写回 settings.json + 审计）；bridge `permission_rules`/`remove_permission_rule`；Tauri `owo_permission_rules`/`owo_remove_permission_rule`（invoke_handler 已注册）；设置页新增「权限规则」卡片（列表：工具·模式+拒绝标记+范围徽标+撤销按钮；设置页打开时自动加载）——修复「remember 规则一旦产生无法查看/撤销」。

**A2-1 Hooks 系统**（新模块 `owo-agent-core/src/hooks.rs`，5 测试）：5 事件（PreToolUse/PostToolUse/UserPromptSubmit/Stop/PreCompact）+ matcher 工具名通配（`desktop_*`）+ settings.json `hooks` 数组配置。执行：命令经系统 shell（cmd /C 或 sh -c）、事件上下文 JSON 写 stdin、单 hook 10s 超时、**exit 2 = 阻断且 stderr 原样回喂模型**，其余非零码仅告警。接线：PreToolUse 在权限评估前（阻断走既有 deny 路径回喂模型不终止回合）；UserPromptSubmit 可拒绝整回合（`AgentError::HookBlocked`）；PostToolUse/Stop/PreCompact 通知性质。`Agent.hooks` 用 RwLock（`set_hooks(&self)` 快照语义），**启动灌入 + settings 保存后热重灌**。安全边界：hooks 是用户自配命令，等同用户手动执行，不过沙箱（沙箱门卫只管模型触发的进程）。

**验证**：agent-sdk 四 crate 编译通过；core `cargo test --lib` **357 全绿**（hooks 5 项含真实子进程阻断/stdin 校验）；clippy 零警告；owo-bridge 18 测试全绿；overlay（MSVC）check 通过；app.js/pet-chat.js 语法通过。

**本批未做（记入差距文档）**：A6-2 工具 mutating 元数据 + 审批模式档位（现有 Level 分级已覆盖大半语义）；A6-3 沙箱档位设置页；CLI/TUI 审批的 remember 选项（HTTP 端三档已全通）。Hooks 设置页 UI（当前改 settings.json 生效）。

**待办**：批次 5（代码工程能力：LSP/multi-edit/web_search/git/压缩骨架）。

### 2026-09-30 批次 3：模型层（多模态 / Anthropic / prompt caching / NO_PROXY）

**A1-2 多模态消息模型（非破坏性）**：`ChatMessage` 新增 `images: Vec<MessageImage>`（serde default，老会话记录零迁移）；`content` 保持纯文本，所有既有调用点不受影响。OpenAI 请求构造：user 角色 images 非空时 content 升级为 `[{type:text},{type:image_url}]` parts 数组（tool/system 角色忽略 images——兼容端点对数组 content 支持不一）。`estimate_tokens` 计入 `IMAGE_TOKEN_ESTIMATE=1100/张` 保守开销。新增 `ChatMessage::user_with_images` 与 `Agent::run_turn_with_images`（run_turn_inner 加 images 参数，`#[allow(clippy::too_many_arguments)]`）。**附件入上下文接线**：server turn handler 中图片附件（png/jpg/jpeg/webp/gif，≤5MB）→ 读文件 base64 data URL → images 真正进视觉上下文；文本附件维持路径注入。

**A1-2 前端贴图链路（overlay 补全）**：bridge `OwoBridgeClient::upload_attachment`（`POST /session/{id}/attachments`，JSON `{name,mime,data_b64}`，复用 401 刷新重试）；Tauri 命令 `owo_upload_attachment`（`blocking().await??` 惯例，返回服务端 sanitize 后文件名，invoke_handler 已注册）；app.js 输入区新增「图片」按钮（SVG）+ `Ctrl+V` 粘贴截图直入附件条（`paste` 事件拦截 clipboardData.files，防止当文本粘贴）+ chips 缩略图可移除（≤4 张 × ≤4MB，前端闸门先于引擎 5MB/50MB）；`sendChatMessage` 收集贴图 → `sendOwoTask` 逐张上传 → `owo_send` 携带文件名数组；轻环模式带图发送被拦截提示（轻环无多模态通道）。index.html/styles.css 同步（`.chat-attach-bar[hidden]` 防覆盖）。

**A1-1 AnthropicProvider**（新模块 `owo-agent-core/src/anthropic.rs`，约 620 行含 5 测试）：`/v1/messages` 原生协议——`x-api-key`+`anthropic-version`（`ANTHROPIC_VERSION` 覆盖，OnceLock 缓存）、system 独立顶层、`tool_use`/`tool_result` 内容块（tool 结果合并进 user 消息、紧随 tool_use）、图片块（http url 直传 / data URL 解析 base64 source）。流式：`content_block_delta` 的 `text_delta`/`input_json_delta`/`thinking_delta` 分别映射正文/工具参数累积/思考通道（与现有 `ReasoningDelta` SSE 对接）；`message_start`/`message_delta` 的 usage 入账。非流式 stop_reason=max_tokens 时仍返回截断文本。代理+直连双通道与 OpenAI provider 一致（复用 `build_model_http_client`）。`max_tokens` 默认 8192（`OWO_ANTHROPIC_MAX_TOKENS` 覆盖）。模型默认 `claude-sonnet-4-5`（`ANTHROPIC_MODEL` 覆盖）。

**A1-3 prompt caching**：请求构造时 `cache_control:{type:ephemeral}` 打在 system 尾块 + 最后一条 user 消息尾块（≤4 断点约束内）；`cache_read_input_tokens>0` 时 tracing 日志明示命中（验证口径），CacheRead 计数入 `last_cache_read`。

**A1-3 provider 选择接线**：`DeferredProvider` 缓存改为 `(指纹, Arc<dyn ModelProvider>)`——`OWO_PROVIDER=anthropic` 且 `ANTHROPIC_API_KEY` 可用 → Anthropic 原生；否则 OpenAI-compatible。指纹含端点/密钥/模型/出境开关，变化即重建（设置页热切换语义不变）。`provider_ready()` 同步感知。CLI 主 agent 走 `ResilientProvider::from_deferred()` 已自动生效；Auto-review 旁路小模型仍走 OpenAI 兼容配置（可配 OWO_REVIEW_MODEL）。

**A1-4 NO_PROXY**：抽公共 `build_model_http_client()`；配置代理时读 `NO_PROXY`/`no_proxy`（`reqwest::Proxy::no_proxy(Some(NoProxy::from_string(..)))`），本地网关 127.0.0.1 等回环端点不再被推进代理。

**验证**：`cargo check` core+server 通过；`cargo test -p owo-agent-core --lib` **357 全绿**（含 anthropic 5 项：system 提取+tool_result 合并+缓存断点、图片块转换、流式三通道累积、非流式 blocks 解析、错误 payload 映射）；server 13 全绿；overlay `cargo check`（MSVC）通过；`cargo test -p owo-bridge` 19 通过；`node --check` app.js 通过；read_lints 零告警。中途修复 3 个点：`blocking().await??` 双层解包、闭包 move 后 `name` 所有权（fallback 克隆）、`ParentNode.append` 返回 void。

**待办**：真机验收（ANTHROPIC_API_KEY + `OWO_PROVIDER=anthropic` 启引擎跑 10 轮带工具任务、第二轮 cache_read>0、贴图提问理解）；设置页无 Anthropic/provider 选择项（`OWO_PROVIDER`/`ANTHROPIC_API_KEY`/`ANTHROPIC_MODEL` 暂只能环境变量配置，UI 纳入后续批次）；pet-chat 不支持贴图（保持轻量）。

### 2026-09-30 批次 2：呈现与会话（对应 docs/agent-capability-gap-review-2026-09.md）

**B2-1 markdown 渲染**：新增 `ui/markdown.js` 轻量渲染器（零依赖、全 DOM API 构建无 innerHTML 注入面）。支持：围栏代码块（语言标签+复制按钮，复制用 navigator.clipboard 失败回退 execCommand）、标题 h1-h4、有序/无序列表（嵌套一层）、引用、分隔线、行内 `code`/粗/斜/删除线/链接。链接不产生真实导航（WebView2 内导航会劫持面板），点击复制 URL。`appendChatBubble` 的 assistant 气泡走 markdown（user/error 保持纯文本）；**流式期间仍为纯文本追加，final 定型时才 markdown 渲染**（避免逐 delta 重渲染卡顿）。主面板与 pet-chat 共用（两个 html 均引入）。超长输入 60k 字符截断保护。

**B2-2 工具结果可见**：引擎 `ToolResult.preview`（已截断 1600 字符，R10 附加可选字段）经透传直接可用，无需改 bridge。`owoToolCardContent` 结果行改为 `<pre>` 展示 preview/错误详情，不再只显示「已执行」。

**B2-3 行级 diff**：`renderOwoDiff` 重写——`lineDiffRows`（LCS DP，行数 >300 退化整删整增）做行级 +/- 着色（绿/红底色），连续未改动行 >2 行折叠为「…N 行未改动」。**单文件回滚未做**：引擎 `/session/{id}/revert` 只有整体回滚（server lib.rs:271），需引擎侧扩展，已记入差距文档。

**B2-4 计划/思考/统计渲染**：`plan_update` → `chat-plan-card`（`已完成/总数` 标题，○/◐/● 状态图标，同回合复用一张卡整表重绘）；`reasoning_delta` → `chat-reasoning` 折叠区（默认收起、流式追加）；`turn_stats` → 一行小字（耗时/步数/tokens/成本）。`owoFinishTurn` 清理 planCard/reasoningBox/pendingTools 引用。

**B3-1 会话列表**：对话区加「历史」按钮 → `sessions-panel`（`owo_sessions` 列表：标题/更新时间/pin 标记/当前高亮，最多 30 条）→ 点击 `owo_use_session` 切换（前端同步 `owo.sessionId`）并强制回放该会话历史。注意补了 `.sessions-panel[hidden]{display:none!important}`（项目已知坑：author 级 display 压过 [hidden]）与 `.agent-view{position:relative}` 锚点。

**B3-2/3 历史完整回放**：`replaySessionDetail` 基于 `/session/{id}` 的 messages（ChatMessage 数组）重建完整对话流：user/assistant markdown 气泡、assistant(tool_calls) 重建工具卡、tool 消息按 call_id 填充结果（`工具错误` 前缀判失败）、「历史摘要」system 显示压缩提示；上限 80 条。主面板首开与切会话共用；pet-chat 打开时回放最近 20 条文本（修复「重开空白」）。

**验证**：`node --check` markdown.js/app.js/pet-chat.js 全过；行内解析正则冒烟（5 类标记全命中）通过；Rust 侧零改动（SSE 帧透传）。markdown 块级渲染与 UI 联调待真机 `cargo run` 冒烟。

**待办**：批次 3（模型层：多模态/Anthropic/缓存），见差距文档 §三；单文件回滚需引擎支持已记入。

### 2026-09-30 批次 1：P0 可靠性速修（对应 docs/agent-capability-gap-review-2026-09.md）

**B1-1 turn_failed 卡死**：引擎失败终态事件此前落入前端 default 分支被静默忽略，`owo.busy` 永真、发送按钮永久禁用。`app.js owoHandleFrame` 补 `turn_failed` case（错误气泡 + busy 复位）。

**B1-2 回复重复显示**：`token_delta` 流式气泡 + `final.text` 完整气泡并存两遍。`final` 分支改为「定型」：有流式气泡时把 `final.text` 写回该气泡（同时修正 pet-chat.js 同款问题——原先只删空气泡）。

**B1-3 工具卡刷屏**：同一调用产生「执行中+完成」两张卡。`owoAppendToolCard` 拆为创建+`owoToolCardContent` 更新两段式；`tool_use` 暂存卡 DOM 到 `owo.pendingTools`（值结构改为 `{use, card}`），`tool_result` 按 call_id 更新原卡；新增 `.tool-running` 样式（原先执行中误用失败红边）。

**B1-4 409 误导文案**：`TurnEventPayload` 增加 `kind` 字段（`unreachable/busy_409/unauthorized/http/stream`），`owo_send` 错误分支按 `BridgeError` 分类；前端 `bridge_error` 按类别渲染：409 → 「上一轮还在跑」+「中止上一轮」按钮（走 `owo_abort`），unreachable/unauthorized → 引导启动/重启引擎。

**B1-6 Esc 误关**：主面板与 pet-chat 在输入框聚焦时 Esc 只清空/失焦输入，不再直接关面板/窗口（防取消 IME 组词误触）。

**A4-3 压缩失败兜底**（agent-sdk）：`maybe_compact` 压缩模型调用失败时不再静默 `Ok(None)`（会带超预算历史直接撞 400）——新增 `compact_truncate_to_budget`（按 token 从尾部硬裁剪、`align_keep_start` 对齐 tool 群组切点、越界保守不裁），写 `compaction_fallback` 审计并返回可见提示；前端 compaction 气泡透出摘要前 120 字。新增 3 项单测。

**验证**：`cargo test -p owo-agent-core --lib` 347 全绿（-j 1 规避 rustc 并行崩溃坑）；`cargo check -p owo-bridge` + `cargo test -p owo-bridge` 19 全绿；overlay `cargo check`（MSVC）通过；`node --check` app.js/pet-chat.js 通过。

**待办**：批次 2（markdown 渲染/工具结果可见/行级 diff/会话列表 UI），见 `docs/agent-capability-gap-review-2026-09.md` §三。

### 2026-09-29 输入法 × Agent 集成（E1~E5，对应 `docs/ime-agent-integration-plan.md`）

**核心成果：打通「OwO 输入法 → Agent 引擎」断链**（此前输入法 `v` 模式只能连官方 mock）。

- **E1 IME 管道适配器**（新 crate `OwO/agent-sdk/crates/owo-agent-ime`，约 2600 行含测试）：
  协议 v3 类型与校验（严格字段集合 / 风险不变量 / 262144 载荷上限）、4 字节小端帧 +
  tokio 命名管道服务端（ACL 限当前用户，SDDL 与官方 mock 一致）、会话状态机
  （幂等缓存 / 槽位补丁 / cancel`notify_one` 防丢失 / 审批等待 / GC）、SSE 增量解析 +
  HTTP 回环（401 刷新重试）、turn 异步适配（`thinking`+`retry_after_ms` / cancel→abort 30s 宽限 /
  diff 审阅候选）、命令映射（insert-reply / view-diff / revert-all / 可信界面确认）。
- **`owo-agent serve-ime` 子命令**：单进程双面（HTTP + 管道），与 `serve` 共用
  `build_server_state`（抽公共启动流程）与数据目录 PidFile 互斥。
- **端到端冒烟通过**（真实 serve-ime + mock 模型 + 管道探针）：
  `submit → thinking → agent_mode → message=ok + insert-reply 候选 → PROBE PASS`。
- **E2 remember 修复**：`PermissionRule`（命令前缀 / 路径 glob / 工具通配）+ 硬拒绝
  不可记住 + `settings.json permissions.rules` 持久化 + `respond_permission` 真消费
  `remember` + 启动灌入策略（9 项测试）。
- **E3 真实 tokenizer**：tiktoken `cl100k_base`（启发式兜底）替换「字符数/2+4」；
  按模型窗口 ×0.75 自动预算；`/usage` 与 `/session/{id}/context` 暴露口径（8 项测试）。
- **E4 循环防护**：滑窗循环检测（≥3 提醒 / ≥5 强制收尾 / 轮询与重读豁免）、
  单步工具超时（默认 120s）、只读并发上限（默认 4）、预算熔断接进 loop（5 项测试）。
- **E5 计划工具**：`update_plan`（整表替换 / 免审批）+ `PlanUpdate` SSE 事件 +
  TUI/CLI 展示（3 项测试）。
- **E7 验收脚本**：`scripts/acceptance.ps1`（fmt/clippy/E1~E5 专项/全量测试，非零退出）；
  工具脚本 `scripts/ime-pipe-probe.ps1`（管道探针）。
- **环境坑（本机特有，已入方案 §0.3）**：workspace 编译需 `-j 1`（默认并行会 rustc 崩溃）；
  `.ps1` 必须带 BOM；`[IO.File]` 相对路径按进程启动目录解析。

**待办（下一步）**：E1.8 真机联调（装 OwO 0.2.3 + `org.owo.agent-ipc` 1.2.0 连接器，
在记事本打 `v` 前缀验证）；E6 桌宠 M1~M3（照 `docs/pet-agent-integration.md` 执行）。

### 2026-09-28 P0 修复（可信度地基，对应 docs/agent-optimization-plan.md）

- **根清单恢复（P0-2，未拉 GitHub，全部本地完成）**：从 `.git` 悬空对象找回原版根 `Cargo.toml`
  （members 原样恢复并补入 `crates/owo-bridge`；`apps/overlay` 与 `crates/assistant-inference`
  维持 exclude——GNU 环境缺 dlltool 无法 codegen，理由见根清单注释）、根 `rust-toolchain.toml`
  （GNU pin）与 `.gitignore`；`.git` 空壳已用 `git init` 修复（重建 HEAD/config，objects 保留）。
- **假验证脚本清除（P0-1，agent-sdk 侧）**：删除 `validation_report.rs` 与
  `crates/owo-agent-core/final_verification.rs`（纯 println 零断言）；新增真契约测试
  `crates/owo-agent-core/tests/crypto_contract.rs`——v4 round-trip / 随机 nonce / 篡改拒绝 /
  错误 DEK / v1-v3 兼容，验收命令 `cargo test -p owo-agent-core --test crypto_contract`。
  注：`storage_crypto.rs` 单元测试早已覆盖等价内容且 `legacy_v3_tag` 已被使用（dead_code 已不存在）。
- **CI 噪音（P0-3）**：`ci-gate.ps1`/`gate.ps1` 现行脚本本身正确（`-D warnings` + 退出码处理）；
  问题出在 2026-08-20 遗留的 7 份过时日志（clippy.log 等，旧机器产物）——已全部删除。
- **文档对齐（P0-4）**：agent-sdk README「尚未实现」段改写为 Roadmap；新增 `docs/ARCHITECTURE.md`
  （轻环/重环/overlay/桌宠 双环数据流总览）。

### 已完成（本次会话，commit 5de3f9e + 本次新提交）

**小工具 5 轮优化全部完成：**

1. **第1轮 UI 基础修复**：工具页单层滚动容器、计算器结果区限高可滚动、
   天气"今天/明天/周X"标签 + °C + 更新时间戳
2. **第2轮 翻译配置打通**：`translation_config` 优先读设置页 API Key/端点/模型，
   环境变量 `LINGXI_OPENAI_*` 兜底，DeepSeek 默认
3. **第3轮 剪贴板真实监听**：后端 1.5s 轮询线程（接入 setup 启动）、
   `chrono_like_now` 改 Win32 `GetLocalTime`（原 PowerShell 1s/次）、
   去重方向修复、`widget_clipboard_remove` 持久化删除
4. **第4轮 UI 统一**：widget.js 统一剪贴板助手（WebView2 里
   `navigator.clipboard` 会静默失败）、天气页双 display bug 修复
5. **第5轮 工程标准 + 回归**：`docs/engineering-standards.md` 建立，
   全量构建通过，`LINGXI_OPEN_ALL_WIDGETS=1` 冒烟 6 个小工具全部正常

**追加修复（本提交）：**

- **取色器交互式取色**：原实现在点按钮瞬间读光标处像素（永远是按钮自己的颜色）。
  改为：点击后隐藏窗口 → 鼠标移到目标 → 左键确认 / Esc 取消 / 60s 超时。
  涉及 `widget_pick_color`（main.rs）+ colorpicker.html 文案。
- **改写模式 chip 恒显 bug**：`.modes { display: grid }` 压过 `[hidden]`，
  导致「润色/纠错/提示词增强」在所有页面显示。补
  `.modes[hidden] { display: none !important; }` 并把 chip 行挪到
  功能切换行下方（视觉归属改写页）。

### 待办（下次继续的方向）

- [ ] **热键冲突**：Ctrl+Alt+O / T / C 三组热键在本机被其他程序占用
  （RegisterHotKey 返回 0x80070581，非灵犀 bug）。可考虑做成设置页可配置。
- [ ] **取色器体验增强**：可加放大镜预览（当前只能盲选位置）
- [ ] **插件市场按钮**（工具页 tools-market-btn）目前 disabled，未实现
- [ ] **桌宠对话**（pet.js）与 Agent 会话打通尚在初期
- [ ] **NSIS 安装包**：ime-server + overlay 合并打包流程待建立
- [ ] Rime/小狼毫集成（见 docs/weasel-integration.md，未动工）

## 二、环境搭建（新电脑必读）

1. **路径硬约束**：仓库必须放在**纯英文路径**（如 `D:\dev\LingXi-DesktopAgent`），
   GNU 工具链链接器不支持中文路径。
2. **工具链**（两套并存）：
   - workspace（crates/* + ime-server）：**GNU** toolchain，见根 `rust-toolchain.toml`
   - overlay：**MSVC** toolchain + WebView2（独立 crate，不在 workspace），
     见 `apps/overlay/rust-toolchain.toml`，需 VS Build Tools
3. **Node.js**：仅用于前端语法校验脚本，非必需
4. **克隆后验证**：
   ```powershell
   cd apps\overlay
   cargo check        # 应无错误
   cargo run          # 主面板 + 桌宠启动
   # 冒烟测试全部小工具：
   $env:LINGXI_OPEN_ALL_WIDGETS=1; cargo run
   ```

## 三、关键文件索引

| 文件 | 职责 |
| --- | --- |
| `apps/overlay/src/main.rs` | 主入口：所有 Tauri 命令、热键、托盘、剪贴板监听（约 2300 行） |
| `apps/overlay/src/widgets.rs` | 小工具 manifest 目录 + 窗口生命周期（`destroy()` 关闭） |
| `apps/overlay/ui/` | 主面板（index/app.js/styles.css）+ widgets/ 六个小工具 |
| `apps/overlay/ui/widgets/widget.js` | 小工具共享：关闭逻辑 + 剪贴板助手（`writeClipboard/readClipboard`） |
| `apps/overlay/capabilities/default.json` | **窗口权限注册**——新小工具窗口 label 必须加进来，否则白屏 |
| `crates/tools-windows/` | 截屏/取色/输入模拟等 Windows 工具 |
| `docs/engineering-standards.md` | 工程标准（窗口管理六条铁律、UI 规范、验证清单）**必读** |

## 四、已知坑（踩过的，别再踩）

1. 小工具窗口**必须在 capabilities/default.json 注册**，否则安全隔离→白屏
2. 关窗口用 `destroy()` 不用 `close()`（后者 JS 忙时卡死 = "关不掉"）
3. 不要在 `open_widget` 里开 DevTools（Windows 上会最小化宿主窗口）
4. 主线程创建 WebView2 窗口会死锁——一律从子线程/托盘子线程创建
5. WebView2 白屏排查顺序：杀孤儿进程 → 重建缓存目录 → 重装
6. CSS 里 author 级 `display` 会压过 `[hidden]`（已修 .modes 和 .status，
   写新视图时注意同样陷阱）
7. PowerShell 子进程开销约 1s/次——热路径（轮询循环）禁止用
8. `.lock().unwrap()` 一律换 `.safe_lock()`

## 五、运行时状态

- 当前本机有运行中的 overlay 实例（cargo run 后台），换机前无需处理
- API Key 等配置存放在本机 DPAPI/内存，**不随仓库走**——
  新电脑首次使用需在「模型设置」重新填写（或设 LINGXI_OPENAI_* 环境变量）
