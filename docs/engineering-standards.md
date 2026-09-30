# 灵犀工程标准文档

> 定位：团队与 AI 协作者共用的工程基线。核心导向是**用户端体验与实用性**——
> 规范服务于"能打开、能用、好用"，而非形式化审查。

## 1. 项目结构与模块划分

```
LingXi-DesktopAgent/
├── apps/
│   ├── overlay/          # 主面板 + 小工具（Tauri 2 / MSVC 工具链，独立于 workspace）
│   ├── ime-server/       # 输入法服务（GNU 工具链，随 NSIS 安装包分发）
│   └── demo|probe|smoke|watch/  # 验证与诊断小程序
├── crates/
│   ├── assistant-core/       # Transformer trait、diff 等核心抽象
│   ├── assistant-ime/        # 拼音切分 / Viterbi / 重排
│   ├── assistant-inference/  # 本地 Qwen2.5（candle）与云端后端
│   ├── assistant-windows/    # Win32 交互：剪贴板、热键、UIA、键盘写入
│   ├── lingxi-agent/         # Agent 引擎与会话
│   └── tools/ + tools-windows/  # 内置工具与 Windows 专用工具
└── docs/                 # 工程文档（本文件所在）
```

**硬性约束**

- 项目必须位于**纯英文路径**（Rust GNU 链接器限制），目录 `D:\项目` 下各仓库名保持英文。
- overlay（MSVC + WebView2）与 ime-server（GNU）工具链不同，overlay 以独立 crate 存在，
  不并入 workspace。
- ime-server 与 overlay 必须打进同一个 NSIS 安装包，保证一键安装。

## 2. Rust 编码规范

| 规则 | 说明 |
| --- | --- |
| 互斥锁 | 一律 `.safe_lock()`，禁止 `.lock().unwrap()`，避免毒化级联 |
| 二进制读取 | 用 `tokio::fs::read` + `from_utf8_lossy`；`read_to_string` 对二进制必失败 |
| 阻塞操作 | PowerShell / OCR / 截图 / 网络请求必须 `spawn_blocking` + `tokio::time::timeout` 包裹 |
| 子进程 | 所有 PowerShell 调用加 `-NoProfile -NonInteractive`，HTTP 加 `-TimeoutSec` |
| 时间获取 | 禁止为取时间启动 PowerShell（约 1s/次）；用 Win32 `GetLocalTime` 零开销调用 |
| 会话数据 | 禁止 `std::mem::take` 直接拿走会话数据导致丢失；用 clone + 条件写回 |
| 错误信息 | 面向用户输出中文、可操作（如"请在设置中填写 API Key"），而非裸错误码 |
| 输出截断 | 外部命令输出在 50KB（UTF-8）处安全截断，防溢出 |

## 3. 窗口与小工具标准

这是历次"弹窗关不掉 / 全白 / 置顶霸屏"问题的结晶：

1. **注册即可见**：每个小工具窗口的 label 必须写入
   `apps/overlay/capabilities/default.json` 的 windows 列表，
   否则安全隔离导致资源加载失败 → 白屏。
2. **置顶是特例**：`WidgetManifest.always_on_top` 默认 `false`。
   只有即用即走的浮层（取色器）允许置顶；计算器、天气等常驻工具按普通窗口对待。
3. **关闭用 `destroy()`**：`close()` 走 CloseRequested 往返，
   webview JS 忙时会卡死（用户遇到"关不掉"的根因）。
   前端兜底顺序：后端 `close_widget` → `window.close()` → Esc 快捷键。
4. **尺寸自适应**：窗口默认尺寸必须能完整展示内容（计算器教训）。
   内容区用 `min-height` + `overflow-y: auto`，键盘网格 `grid-auto-rows: 1fr`。
5. **滚动单层原则**：一个视图只允许一个滚动容器（外层），
   内部网格保持自然高度；嵌套滚动 + flex 压缩会让列表"划不动"。
6. **WebView2 稳定性**：白屏时先清理孤儿 WebView2 进程（会锁数据目录），
   再隔离重建缓存目录，最后才考虑重装。

## 4. 前端 UI 规范

**设计语言（widget.css / styles.css 已实现）**

- 深色扁平、低饱和：背景 `#1b1d21`，卡片 `--surface`，描边 `--stroke`。
- 品牌紫 `--brand: #8aadf4`（overlay 主面板为 `#7c8cff`），
  强调靠字重与层级，不靠颜色堆砌；标题栏用左侧 3px 色条代替图标。
- 统一圆角 6–8px（主面板 12–16px），统一细滚动条样式。

**交互标准**

- 所有可等待操作必须给状态反馈：`setStatus()` + spinner，禁止静默。
- 剪贴板读写一律走 `widget.js` 的 `writeClipboard()/readClipboard()`
  （后端命令优先）：`navigator.clipboard` 在 WebView2 会因权限静默失败。
- 危险操作（清空、退出）用面板内 confirm 弹层，不用系统对话框（会在小窗外）。
- 快捷键统一：Esc 关闭窗口，Ctrl+Enter 提交，全局热键注册在 WidgetManifest。

**布局红线**

- 不允许内容截断：显示区 `max-height` + 滚动，或直接让窗口可拉伸。
- 不允许双 `display` 内联样式互相覆盖（天气页教训）。
- 中文文案口语化、去机器感；错误提示必须包含下一步建议。

## 5. 输入法管线标准

候选词生成固定流水线：

```
拼音切分 → Viterbi 最短路词典查找 → 多源候选收集 → 上下文重排
```

- 前端候选刷新间隔 16ms（一帧）。
- 回退词典必须含 **8105 常用汉字**（35 词的回退曾导致离线完全无法输入）。

## 6. 验证清单（每轮迭代必做）

```powershell
# 1. Rust 编译（overlay 用 MSVC）
cd apps/overlay; cargo check; cargo build

# 2. 前端语法（独立 JS + HTML 内嵌脚本）
node -e "/* 见 scripts 或会话中的检查脚本 */"

# 3. 小工具冒烟：一次性打开全部小工具
$env:LINGXI_OPEN_ALL_WIDGETS=1; cargo run
#    观察 stderr 每个窗口的 "page_load: Finished"，并逐个 Esc 关闭验证
```

人工验收要点：每个小工具能打开、内容不截断、能关闭；
弹窗不强制置顶；剪贴板历史能在复制后 ≤2s 出现新条目。

## 7. 文档与协作

- 用户偏好：中文交流；重视实用性与代码规范；期望"委托执行 + 自验完成"，
  次日直接看结果。
- 每个关键修复在代码注释中保留根因说明（本文档多处规则即来自这些注释），
  供后续协作者避免回退。
- 安全策略从宽：个人桌面工具，默认信任本地配置；但 API Key 仍走 DPAPI 加密存储，
  不写明文。
