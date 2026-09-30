# Agent 框架优化方案（对齐 Codex / Claude Code / CodeBuddy）

> 适用范围：`LingXi-DesktopAgent`（轻环 `crates/lingxi-agent` + 桌面 overlay）与 `OwO/agent-sdk`（重环 Agent 服务）
> 基线：2026-09-27 代码快照
> 目标：把「能跑的 Agent」升级为「可托付的 Agent」
>
> **状态（2026-09-28）：§一 P0 四项已全部落地并验证通过。**
> - P0-1：假脚本已删；`crates/owo-agent-core/tests/crypto_contract.rs` 11 项契约测试落地，变异验收（改坏实现一字节 → v2/v3 测试变红 → 还原复绿）通过。
> - P0-2：根清单/工具链/.gitignore 从 .git 悬空对象恢复（members 补 `crates/owo-bridge`；`apps/overlay` 与 `crates/assistant-inference` **维持 exclude**，修正本方案原清单把 inference 列入 members 的错误）。验收：GNU `cargo check/test --workspace` 全绿 + overlay `cargo check`（MSVC）EXIT=0。
> - P0-3：workspace `clippy --all-targets -D warnings` 清债后 EXIT=0（顺带清了 6 处既有 lint）；发现并修复 `cloud_exec_tests` 既有 env 竞态 flaky。
> - P0-4：agent-sdk README「尚未实现」→ Roadmap；新增 `docs/ARCHITECTURE.md`。
> - 明细见 `docs/project-status.md` 2026-09-28 段。§二起（P1/P2/P3）未开始。

---

## 〇、先定坐标：两条 Agent 环路要收敛

当前仓库里实质存在**两套 Agent 心智**，这是所有混乱的根源：

| | 轻环（内置） | 重环（外接） |
|---|---|---|
| 代码 | `crates/lingxi-agent` + `apps/overlay/src/agent.rs` | `crates/owo-bridge` → 外部 `owo-agent.exe serve` |
| 轮数 | 10 轮 | 60 轮 |
| 流式 | 无 | SSE 全事件 |
| 审批 | `Arc::new(DenyAll)`（`agent.rs:107-111`） | 真审批 + 300s 超时 Deny |
| 取消 | 无 | `POST /session/{id}/abort` |
| 沙箱 | 无 | Windows Job/AppContainer |
| 回滚 | 无 | `diff/revert/fork/rewind/redo` |
| 工具 | 桌面操作为主（QQ/剪贴板/窗口/截屏） | 42 个（含文件/搜索/命令/浏览器/MCP） |

**决策：统一到重环。** 轻环降级为「离线兜底的文本改写/润色」，不再承担工具型任务；所有 UI（主面板、桌宠、小工具）只走 `owo-bridge`。这样 §一 的大部分优化只需做在 OwO 一处，LingXi 侧只做接入与 UI。

---

## 一、P0：先把「可信度」修好（1～2 天）

这一级不做，后面所有优化都无法验证成效。

### P0-1 删除/重写假验证脚本 🔴

**问题**：`OwO/agent-sdk/validation_report.rs` 与 `final_verification.rs` 是纯 `println!`，零断言：

```rust
// validation_report.rs:9-11
println!("1. 正在执行 v4 round-trip 测试...");
println!("   ✓ 加密/解密成功");
```

它们制造「已验证」的假象，比没有验证更危险。

**改动**：
1. 删除这两个文件（或改名 `*.rs.bak` 留档）。
2. 把声称的 5 项（round-trip / 密文不同 / 篡改拒绝 / 错误 DEK / v1-v3 兼容）写成 `crates/owo-agent-core/tests/crypto_contract.rs` 里的真 `#[test]`，直接调 `storage_crypto` 的真实 API。
3. `ACCEPTANCE.md` 中引用这两脚本的段落同步改写为引用 `cargo test -p owo-agent-core --test crypto_contract`。

**验收**：`cargo test -p owo-agent-core` 真实执行；故意改坏一个字节，测试必须**变红**。

### P0-2 修复 LingXi workspace 根清单缺失 🔴

**问题**（2026-09-27 复核修正）：本地 `LingXi-DesktopAgent` 是一份**不完整快照**，不是「仓库本身缺清单」：

- 根目录无任何文件（无 `Cargo.toml` / `rust-toolchain.toml` / `.gitignore`），而 `docs/project-status.md:49` 明确说根 `rust-toolchain.toml` 应存在——快照丢了根级文件；
- `.git` 是空壳（只有 `objects/`、`refs/`，无 `HEAD`/`config`），`git` 直接报 `not a git repository`；
- 多个 crate 依赖 workspace 继承（`lingxi-agent` 的 `version.workspace = true`、`owo-bridge` 的 `serde = { workspace = true }`），而 overlay 又以 path 依赖 `owo-bridge` → **没有根清单时整个 crates 树和 overlay 都无法构建**，`.github/workflows/ci.yml` 的 `cargo test --workspace --locked` 自然跑不动。

**改动（按优先级）**：

1. **优先从 origin 恢复**（`project-status.md` 记录了仓库地址 `github.com/3311930677/LingXi-DesktopAgent`）：重新 clone 到**纯英文、无空格**路径，把本地新增/修改（本方案、桌宠方案等）迁过去。注意当前目录 `working OWOWOWOWO` 含空格，与 `project-status.md:46-47`「必须纯英文路径」的约束冲突，GNU 工具链验证前先迁目录。
2. **无法联网时手工重建**根 `Cargo.toml`：

```toml
[workspace]
resolver = "2"
members = [
    "crates/assistant-core",
    "crates/assistant-inference",
    "crates/assistant-ime",
    "crates/assistant-windows",
    "crates/lingxi-agent",
    "crates/tools",
    "crates/tools-windows",
    "crates/owo-bridge",
    "apps/demo",
    "apps/ime-repl",
    "apps/ime-server",
    "apps/probe",
    "apps/smoke",
    "apps/watch",
]
exclude = ["apps/overlay"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
# 上提各 crate 已用到的 serde / serde_json / tokio / async-trait / thiserror / reqwest 等
```

   复核时确认的三个易错点：
   - `apps/` 下 **7 个子目录全部是 crate**（demo/ime-repl/ime-server/overlay/probe/smoke/watch），初版方案只列了 ime-server，漏了 5 个；
   - `apps/overlay/Cargo.toml:7-8` 只有**注释**声称 excluded，没有 `[workspace]` 空表——根 workspace 出现后，cargo 会因「不在 members 又未被 exclude」直接报错。必须在根清单写 `exclude = ["apps/overlay"]`（或给 overlay 补 `[workspace]` 空表，二选一，推荐前者）；
   - 同时补回根 `rust-toolchain.toml`（GNU pin）与 `.gitignore`（至少含 `target/`）。

**验收**：在纯英文无空格路径下 `cargo check --workspace`（GNU）+ `cd apps/overlay && cargo check`（MSVC）均通过；`git status` 可用。

### P0-3 修 CI 噪音，让 clippy 真正生效

**问题**：`clippy.log` 内容只有 PowerShell `NativeCommandError` + `Finished`，**没有任何 clippy 诊断**——脚本把 stderr 当错误吞了，等于门禁失效。`cargo_check.log` 有 1 个 `dead_code` 警告（`storage_crypto.rs:324 legacy_v3_tag`）。

**改动**：
- `scripts/` 里跑 clippy 的 PowerShell 加 `-ErrorAction Continue` / `2>&1 | Out-File`，不要用 `$ErrorActionPreference='Stop'` 包 NativeCommand。
- CI 用 `cargo clippy --workspace -- -D warnings`，先清掉 `legacy_v3_tag` 的 dead_code（加 `#[allow(dead_code)]` 并注明保留原因，或删除）。
- 加一条门禁：**CI 里出现 `warning: ` 即失败**（`-D warnings` 已覆盖，关键是让输出真的被抓到）。

### P0-4 文档与代码对齐

- `OwO/agent-sdk/README.md:299-302`「尚未实现」段已过期（SQLite / 沙箱 / traces 均已落地）→ 改写为「Roadmap」并标注对应章节。
- 补一份 `docs/ARCHITECTURE.md`：一张图说明 轻环/重环/overlay/桌宠 的数据流，避免下一个人再摸索两套 Agent。

---

## 二、P1：模型层与上下文（决定能力天花板）

### P1-1 真实 tokenizer（🔴 根因级）

**问题**：`owo-agent-core/src/agent.rs:769` `estimate_tokens()` = `字符数/2 + 4`。中文场景下误差极大（中文 1 字 ≈ 1~1.5 token，而非 0.5），导致：
- 压缩触发过晚 → 超出模型上下文 → 400；
- 或过早 → 丢掉关键历史。

**改动**：
1. 引入 `tiktoken`（`tiktoken-rs`）作为默认计数器，`cl100k_base` 兜底；无法识别的模型退回粗估并在 `/usage` 标注 `estimated: true`。
2. `AgentConfig.token_budget`（现 60_000）改为「按模型上下文窗口 × 0.75」自动推导，模型元数据表维护 `context_window`。
3. `/session/{id}/context` 返回 `tokenizer: "tiktoken|heuristic"` 字段，便于排查。

**验收**：同一段中文/混合文本，估算值与 provider 返回的 `prompt_tokens` 偏差 < 10%。

### P1-2 模型协议扩展：Anthropic 原生 + 多模态

**问题**：`gateway.rs` 只有 `OpenAiCompatibleProvider`；全仓搜 `image_url` **零命中**——模型侧无多模态输入，视觉能力全靠旁路 OCR/ONNX。而这正是 Codex/Claude Code 的核心区。

**改动（分两步，先易后难）**：
1. **消息模型加 `ContentPart`**：把 `ChatMessage.content: Option<String>` 升级为 `content: Option<MessageContent>`（`Text(String)` | `Parts(Vec<ContentPart>)`），`ContentPart` 含 `text` / `image_url` / `image_base64`；OpenAI 分支保持字符串兼容。
2. **新增 `AnthropicProvider`**：原生 `/v1/messages`，支持 `tool_use`、`streaming`、`prompt_caching`（`cache_control: {type:"ephemeral"}` 打在 system 与最近一轮）。prompt caching 对长会话成本是数量级差异，这是必做项。同时 provider 必须支持代理配置（`HTTPS_PROXY`/`ALL_PROXY` 或设置项）——本项目用户群在国内网络环境，直连 anthropic.com 大概率不通。

**验收**：`ANTHROPIC_API_KEY` 接入后可跑通 10 轮带工具的任务；第二轮 prompt 命中 cache（看 `cache_read_input_tokens`）。

### P1-3 压缩策略增强

现状 `maybe_compact()`（`agent.rs:696-745`）已做得不错（模型生成摘要 + `align_keep_start()` 保证 tool 群组不被切碎）。补三点：

1. **保留决策骨架**：压缩时强制留下「用户目标 / 已修改文件清单 / 失败过的尝试」，而不是纯自然语言摘要——否则压缩后 Agent 会重复踩坑。
2. **`keep_recent=20` 按 token 而非条数**：一条超长工具结果就吃光预算，改为 `keep_recent_tokens ≈ budget * 0.25`。
3. **压缩事件持久化**：`compaction` 摘要写进 SQLite session 表，重启后可恢复（目前只在内存/`context` 端点只读展示）。

---

## 三、P2：可控性与安全（决定用户敢不敢托付）

### P2-1 权限策略持久化 🔴

**问题（两个具体缺陷）**：
1. `permissions.rs:292-303` `Policy::decision()`：Read→Allow，Write/Execute/Inject→**每次都 Ask**，没有按工具/路径/命令前缀的持久化 allowlist。用户体验是「要么烦死，要么 `OWO_AUTO_APPROVE=1` 裸奔」。
2. `PermissionResponse { allow, remember }` 的 **`remember` 字段服务端根本没读**（`lib.rs:1671-1675` 只取 `allow`）——前端「总是允许」按钮是假的。

**改动**：
1. 新增 `PermissionRule { scope, tool, pattern, decision, expires_at }`，持久化到 `settings.json` 的 `permissions.rules`（或独立 SQLite 表）。
2. `Policy::decision()` 查表优先级：**危险命令 hard-deny 表 > 会话级临时授权 > 用户规则表 > 默认 Ask**。
3. 服务端真正消费 `remember`：为 true 时把该 `(tool, args 归一化后的 pattern)` 写入规则表，并回写审计。
4. 规则匹配用「命令前缀 + 路径 glob」而非整串相等，例如 `cargo *`、`write_file:/src/**`。

**验收**：首次 `cargo test` 弹审批 → 勾「记住」→ 后续同一前缀不再弹；`rm -rf` 永远弹且不可记住。

### P2-2 循环检测与预算防护 🔴

**问题**：`run_turn` 无循环检测、无每步 wall-clock 超时、无并发上限。模型反复调同一工具可以烧光 60 轮 + 全部预算。

**改动**：
1. **循环检测**：滑窗记录最近 N 次 `(tool, args_hash)`，同一组合重复 ≥3 次 → 注入系统提示「你似乎在重复，请换策略或结束」；≥5 次 → 强制 wrap-up。**豁免轮询型工具**（审查补充）：`desktop_wait_until` 本就设计为同参重复调用，`read_file` 在编辑后重读同一路径也正常——按工具类型白名单豁免，或只统计「连续失败的同参调用」。
2. **每步超时**：`ToolContext` 增加 `deadline: Instant`（默认 120s，命令类可由工具覆盖），超时返回结构化错误而非挂起（现有只有 provider HTTP 120/180s 与 `stream.next()` 60s）。
3. **并发上限**：并行只读工具加 `Semaphore`（建议 4），避免 `join_all` 一次放飞 20 个 `grep`。
4. **预算熔断**：`/usage` 已有 `OWO_USAGE_TOKEN_BUDGET`/`COST_BUDGET_USD`，把它接进 loop——超预算直接终止并告知用户，而不是事后统计。
5. `wait_for_abort` 现在是 50ms 忙轮询，改为 `tokio::sync::Notify` + `select!`。

### P2-3 Hooks 系统（与 Claude Code 对齐）

现状全仓无 hooks。建议最小可用集：

| 事件 | 时机 | 典型用途 |
|---|---|---|
| `PreToolUse` | 工具执行前 | 拦截危险参数、注入额外上下文 |
| `PostToolUse` | 工具执行后 | 自动 `cargo fmt`、格式化输出 |
| `UserPromptSubmit` | 用户输入提交前 | 注入项目规则/当前文件 |
| `Stop` | 回合结束 | 校验是否通过测试 |
| `PreCompact` | 压缩前 | 把关键事实捞出来 |

配置放 `settings.json` 的 `hooks`（`matcher` 支持工具名 glob），exit code 2 = 阻断并把 stderr 回喂给模型。MCP stdio 子进程同样复用 `plugin.rs` 的沙箱门卫。

### P2-4 沙箱跨平台

`sandbox.rs:379-388` 在 Linux/macOS 只探测 `bwrap`/`sandbox-exec` 然后降级 `Unsupported`——等于裸奔。

**改动**：Linux 用 `unshare -Urn --map-root-user` + `bwrap`（`--ro-bind /usr / --bind workspace` + seccomp 可选）；macOS 用 `sandbox-exec` profile。实在不行就明确「非 Windows 仅支持 `IsolationLevel::None` 且在 `/health` 里暴露」，**不要假装隔离**。

### P2-5 plan / todo 工具

补两个工具：`update_plan`（结构化步骤列表，UI 可渲染进度）与 `task`（派发子任务到 `subagent`，带 `depth` 与只读约束）。这是把「能跑」变成「能托付长任务」的分水岭。

---

## 四、P3：生态与体验

| 项 | 现状 | 改动 |
|---|---|---|
| MCP | 仅 `tools`（stdio + streamable HTTP） | 补 `resources/list|read`、`prompts/list|get`；`roots` 至少声明工作区 |
| 记忆 | 关键词 + 二元字符组，无向量 | 接入本地 embedding（ONNX，与现有 OCR 打包方式一致）做 `recall()` 召回；无模型时优雅降级 |
| IDE 集成 | 无 | 先做 LSP 侧车：把 `go_to_definition/references` 包成工具，这是 coding agent 与普通 shell agent 的分界线 |
| 可观测性 | 仅 tracing | 结构化 `turn` span（model/steps/tokens/cost/工具耗时 Top3）落 SQLite，`/traces` 已有可复用 |
| 并发 | 单会话并发 turn 返回 409 | 409 时前端给出明确提示（「上一轮还在跑」）并提供「中止并开新一轮」按钮 |

---

## 五、排期建议

| 阶段 | 内容 | 估时 | 风险 |
|---|---|---|---|
| P0 | 假脚本/根清单/CI 噪音/文档 | 1–2 天 | 低 |
| P1 | tokenizer + 多模态消息模型 + Anthropic provider + 压缩增强 | 1–2 周 | 中（消息模型改造面广） |
| P2 | 权限持久化 + 循环检测/超时 + hooks + 沙箱跨平台 + plan/todo | 2–3 周 | 中高（安全语义变更） |
| P3 | MCP 全能力 + 记忆向量 + LSP + 可观测性 | 2–4 周 | 低 |

**建议顺序**：P0 → P2-1（权限，用户体验最痛）→ P1-1（tokenizer，稳定性根因）→ P2-2 → P1-2 → 其余。

---

## 六、验收口径（替代现在的假脚本）

新增 `scripts/acceptance.ps1`，真跑并输出非零退出码：

1. `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test --workspace --locked` 全绿；
2. **端到端脚本任务**：给定工作区，让 Agent 完成「新建文件 → 写入 → 跑测试 → 修一个故意的编译错误 → 通过」，断言最终测试通过且 `diff` 只涉及预期文件；
3. **中断测试**：中途 `abort`，断言 loop 在 5s 内停止且会话可恢复；
4. **审批测试**：触发一次 Write，断言收到 `permission_request`、不响应 300s 后按 Deny 处理、`remember=true` 后二次不再询问；
5. **预算测试**：设置极小 token 预算，断言 loop 主动终止而非报错崩溃。

以上 5 条任一失败即 CI 红。
