"use strict";

// Tauri 2 exposes invoke on window.__TAURI__.core when withGlobalTauri is on.
const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
document.documentElement.classList.toggle("tauri-runtime", Boolean(invoke));

// Sample text used only for the in-browser preview (no Tauri backend).
const MOCK_SOURCE = "圆圆的月亮真好看";

// Linear stroke icons (24 viewBox) rendered via currentColor so they inherit
// the panel's text color; replacing emoji keeps the tool grid monochrome.
const icon = (d, size = 16) =>
  `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;

const TOOL_ICONS = {
  _default: icon('<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/>'),
  read_file: icon('<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v5h5"/><path d="M16 13H8"/><path d="M16 17H8"/>'),
  write_file: icon('<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/>'),
  list_dir: icon('<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>'),
  search_files: icon('<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>'),
  run_command: icon('<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/>'),
  read_clipboard: icon('<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>'),
  write_clipboard: icon('<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>'),
  list_windows: icon('<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 8h20"/><path d="M6 4v4"/><path d="M10 4v4"/>'),
  focus_window: icon('<circle cx="12" cy="12" r="10"/><path d="M22 12h-4"/><path d="M6 12H2"/><path d="M12 6V2"/><path d="M12 22v-4"/>'),
  capture_screen: icon('<path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3Z"/><circle cx="12" cy="13" r="3"/>'),
  open_app: icon('<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>'),
  type_text: icon('<path d="M4 7V4h16v3"/><path d="M9 20h6"/><path d="M12 4v16"/>'),
  send_keys: icon('<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01"/><path d="M10 10h.01"/><path d="M14 10h.01"/><path d="M18 10h.01"/><path d="M8 14h8"/>'),
  click_at: icon('<path d="m3 3 7.07 16.97 2.51-7.39 7.39-2.51L3 3Z"/>'),
  web_fetch: icon('<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>'),
  web_search: icon('<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>'),
  translate: icon('<path d="m5 8 6 6"/><path d="m4 14 6-6 2-3"/><path d="M2 5h12"/><path d="M7 2h1"/><path d="m22 22-5-10-5 10"/><path d="M14 18h6"/>'),
  calculate: icon('<rect x="4" y="2" width="16" height="20" rx="2"/><path d="M8 6h8"/><path d="M16 14v4"/><path d="M8 10h.01"/><path d="M12 10h.01"/><path d="M16 10h.01"/><path d="M8 14h.01"/><path d="M12 14h.01"/><path d="M8 18h.01"/><path d="M12 18h.01"/>'),
  get_time: icon('<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>'),
  set_reminder: icon('<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>'),
  list_reminders: icon('<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>'),
  cancel_reminder: icon('<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>'),
  qq_read_selection: icon('<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>'),
  qq_write_draft: icon('<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>'),
};

// Widget catalog icons keyed by widget id (cards no longer use the emoji field).
const WIDGET_ICONS = {
  "widget-ocr": icon('<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/><path d="M7 8h8"/><path d="M7 12h10"/><path d="M7 16h6"/>', 18),
  "widget-translate": icon('<path d="m5 8 6 6"/><path d="m4 14 6-6 2-3"/><path d="M2 5h12"/><path d="M7 2h1"/><path d="m22 22-5-10-5 10"/><path d="M14 18h6"/>', 18),
  "widget-colorpicker": icon('<path d="m2 22 1-1h3l9-9"/><path d="M3 21v-3l9-9"/><path d="m15 6 3.4-3.4a2.1 2.1 0 1 1 3 3L18 9l.4.4a2.1 2.1 0 1 1-3 3l-3.8-3.8a2.1 2.1 0 1 1 3-3l.4.4Z"/>', 18),
  "widget-weather": icon('<path d="M12 2v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="M20 12h2"/><path d="m19.07 4.93-1.41 1.41"/><path d="M15.947 12.65a4 4 0 0 0-5.925-4.128"/><path d="M13 22H7a5 5 0 1 1 4.9-6H13a3 3 0 0 1 0 6Z"/>', 18),
  "widget-calculator": icon('<rect x="4" y="2" width="16" height="20" rx="2"/><path d="M8 6h8"/><path d="M16 14v4"/><path d="M8 10h.01"/><path d="M12 10h.01"/><path d="M16 10h.01"/><path d="M8 14h.01"/><path d="M12 14h.01"/><path d="M8 18h.01"/><path d="M12 18h.01"/>', 18),
  "widget-clipboard": icon('<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>', 18),
};

// Tool category mapping for the filter tabs.
const TOOL_CATEGORIES = {
  read_file: "capability", write_file: "capability", list_dir: "capability",
  search_files: "capability", read_clipboard: "capability", write_clipboard: "capability",
  run_command: "capability", capture_screen: "capability",
  list_windows: "adapter", focus_window: "adapter", open_app: "adapter",
  type_text: "adapter", send_keys: "adapter", click_at: "adapter",
  qq_read_selection: "adapter", qq_write_draft: "adapter",
  web_fetch: "source", web_search: "source", translate: "source",
  get_time: "source", set_reminder: "source", list_reminders: "source", cancel_reminder: "source",
  calculate: "transform",
};

// Global hotkey shortcuts for tools that expose one (from VISION.md/ROADMAP.md).
const TOOL_SHORTCUTS = {
  capture_screen: "Ctrl+Alt+O",
  translate: "Ctrl+Alt+T",
  read_clipboard: "Ctrl+Alt+V",
  write_clipboard: "Ctrl+Alt+V",
};

// Sample tools shown only in the browser preview (no Tauri backend).
const MOCK_TOOLS = [
  { name: "read_file", description: "读取文件内容", risk_level: "safe", enabled: true },
  { name: "write_file", description: "写入文件", risk_level: "moderate", enabled: true },
  { name: "list_dir", description: "列出目录内容", risk_level: "safe", enabled: true },
  { name: "search_files", description: "搜索文件内容", risk_level: "safe", enabled: true },
  { name: "run_command", description: "执行系统命令", risk_level: "dangerous", enabled: false },
  { name: "read_clipboard", description: "读取剪贴板", risk_level: "safe", enabled: true },
  { name: "write_clipboard", description: "写入剪贴板", risk_level: "moderate", enabled: true },
  { name: "list_windows", description: "列出所有窗口", risk_level: "safe", enabled: true },
  { name: "focus_window", description: "聚焦指定窗口", risk_level: "moderate", enabled: true },
  { name: "capture_screen", description: "截取屏幕区域", risk_level: "safe", enabled: true },
  { name: "open_app", description: "打开应用程序", risk_level: "moderate", enabled: true },
  { name: "type_text", description: "输入文字", risk_level: "moderate", enabled: true },
  { name: "send_keys", description: "发送快捷键", risk_level: "moderate", enabled: true },
  { name: "click_at", description: "点击屏幕坐标", risk_level: "moderate", enabled: false },
  { name: "web_fetch", description: "抓取网页内容", risk_level: "safe", enabled: true },
  { name: "web_search", description: "搜索网络", risk_level: "safe", enabled: true },
  { name: "translate", description: "翻译文本", risk_level: "safe", enabled: true },
  { name: "calculate", description: "数学计算", risk_level: "safe", enabled: true },
  { name: "get_time", description: "获取当前时间", risk_level: "safe", enabled: true },
  { name: "set_reminder", description: "设置提醒", risk_level: "safe", enabled: true },
  { name: "list_reminders", description: "列出提醒", risk_level: "safe", enabled: true },
  { name: "cancel_reminder", description: "取消提醒", risk_level: "safe", enabled: true },
  { name: "qq_read_selection", description: "读取QQ选区消息", risk_level: "safe", enabled: true },
  { name: "qq_write_draft", description: "写入QQ回复草稿", risk_level: "moderate", enabled: true },
];

const state = {
  mode: "polish",
  source: MOCK_SOURCE,
  transformed: "",
  diff: [],
  warning: null,
};

const el = {
  card: document.getElementById("card"),
  titlebar: document.getElementById("titlebar"),
  modes: document.getElementById("modes"),
  diff: document.getElementById("diff"),
  statAdd: document.getElementById("stat-add"),
  statDel: document.getElementById("stat-del"),
  status: document.getElementById("status"),
  applyBtn: document.getElementById("apply-btn"),
  undoBtn: document.getElementById("undo-btn"),
  recaptureBtn: document.getElementById("recapture-btn"),
  closeBtn: document.getElementById("close-btn"),
  pinBtn: document.getElementById("pin-btn"),
  quickRewrite: document.getElementById("quick-rewrite"),
  quickQq: document.getElementById("quick-qq"),
  settingsBtn: document.getElementById("settings-btn"),
  backendBadge: document.getElementById("backend-badge"),
  rewriteView: document.getElementById("rewrite-view"),
  rewriteActions: document.getElementById("rewrite-actions"),
  qqView: document.getElementById("qq-view"),
  qqConversation: document.getElementById("qq-conversation"),
  qqMessage: document.getElementById("qq-message"),
  qqDraft: document.getElementById("qq-draft"),
  qqRefresh: document.getElementById("qq-refresh"),
  qqGenerate: document.getElementById("qq-generate"),
  qqWrite: document.getElementById("qq-write"),
  settingsPanel: document.getElementById("settings-panel"),
  providerPreset: document.getElementById("provider-preset"),
  backendSelect: document.getElementById("backend-select"),
  endpointInput: document.getElementById("endpoint-input"),
  modelInput: document.getElementById("model-input"),
  apiKeyInput: document.getElementById("api-key-input"),
  rememberApiKey: document.getElementById("remember-api-key"),
  keyNote: document.getElementById("key-note"),
  keyStatus: document.getElementById("key-status"),
  saveSettings: document.getElementById("save-settings"),
  // Pet skin settings
  skinGrid: document.getElementById("skin-grid"),
  bubbleIdle: document.getElementById("bubble-idle"),
  bubbleThinking: document.getElementById("bubble-thinking"),
  bubbleSpeaking: document.getElementById("bubble-speaking"),
  bubbleAlert: document.getElementById("bubble-alert"),
  petVisible: document.getElementById("pet-visible"),
  savePetOptions: document.getElementById("save-pet-options"),
  // Window behavior settings
  panelAutoHide: document.getElementById("panel-auto-hide"),
  panelRememberPos: document.getElementById("panel-remember-pos"),
  saveWindowOptions: document.getElementById("save-window-options"),
  // 历史会话（B3-1）
  agentSessionsBtn: document.getElementById("agent-sessions-btn"),
  sessionsPanel: document.getElementById("sessions-panel"),
  sessionsList: document.getElementById("sessions-list"),
  sessionsRefresh: document.getElementById("sessions-refresh"),
  // 权限规则管理（B4-1）
  permRulesList: document.getElementById("perm-rules-list"),
  permRulesRefresh: document.getElementById("perm-rules-refresh"),
  // 定时任务（A8-1）
  autoList: document.getElementById("auto-list"),
  autoRefresh: document.getElementById("auto-refresh"),
  autoName: document.getElementById("auto-name"),
  autoSchedule: document.getElementById("auto-schedule"),
  autoTime: document.getElementById("auto-time"),
  autoEvery: document.getElementById("auto-every"),
  autoEveryUnit: document.getElementById("auto-every-unit"),
  autoKind: document.getElementById("auto-kind"),
  autoContent: document.getElementById("auto-content"),
  autoCreate: document.getElementById("auto-create"),
  // 重任务引擎设置
  engineWorkspace: document.getElementById("engine-workspace"),
  enginePickFolder: document.getElementById("engine-pick-folder"),
  enginePort: document.getElementById("engine-port"),
  engineAutoStart: document.getElementById("engine-auto-start"),
  engineModelNote: document.getElementById("engine-model-note"),
  saveEngineOptions: document.getElementById("save-engine-options"),
  engineRestart: document.getElementById("engine-restart"),
  engineStop: document.getElementById("engine-stop"),
  // B4-2 引擎路径 / B5-2 热键 / B4-4 剪贴板
  engineExe: document.getElementById("engine-exe"),
  enginePickExe: document.getElementById("engine-pick-exe"),
  panelHotkey: document.getElementById("panel-hotkey"),
  savePanelHotkey: document.getElementById("save-panel-hotkey"),
  clipboardWatch: document.getElementById("clipboard-watch"),
  // B4-3 首启引导
  onboarding: document.getElementById("onboarding"),
  onboardingKeyState: document.getElementById("onboarding-key-state"),
  onboardingEngineState: document.getElementById("onboarding-engine-state"),
  onboardingStartEngine: document.getElementById("onboarding-start-engine"),
  onboardingSettings: document.getElementById("onboarding-settings"),
  onboardingDone: document.getElementById("onboarding-done"),
  engineStatusInline: document.getElementById("engine-status-inline"),
  resizeGrip: document.getElementById("resize-grip"),
  taskProgress: document.getElementById("task-progress"),
  progressStage: document.getElementById("progress-stage"),
  progressTime: document.getElementById("progress-time"),
  progressBar: document.getElementById("progress-bar"),
  progressDetail: document.getElementById("progress-detail"),
  // Agent chat
  agentTab: document.getElementById("agent-tab"),
  agentView: document.getElementById("agent-view"),
  chatMessages: document.getElementById("chat-messages"),
  chatInput: document.getElementById("chat-input"),
  chatSend: document.getElementById("chat-send"),
  // 贴图附件（B3：多模态入上下文）
  chatAttach: document.getElementById("chat-attach"),
  chatFilePick: document.getElementById("chat-file-pick"),
  attachBar: document.getElementById("chat-attach-bar"),
  agentReset: document.getElementById("agent-reset"),
  chatSuggestions: document.getElementById("chat-suggestions"),
  chatEmptyTitle: document.getElementById("chat-empty-title"),
  chatEmptySub: document.getElementById("chat-empty-sub"),
  // 重任务引擎（owo-agent 桥接）+ 对话输入区
  modeSwitch: document.getElementById("mode-switch"),
  modeChat: document.getElementById("mode-chat"),
  modeTask: document.getElementById("mode-task"),
  modeEngine: document.getElementById("mode-engine"),
  owoDot: document.getElementById("owo-dot"),
  owoStatusText: document.getElementById("owo-status-text"),
  owoStart: document.getElementById("owo-start"),
  composerPlus: document.getElementById("composer-plus"),
  toolType: document.getElementById("tool-type"),
  toolVoice: document.getElementById("tool-voice"),
  petToggleBtn: document.getElementById("pet-toggle-btn"),
  // Tools
  toolsTab: document.getElementById("tools-tab"),
  toolsView: document.getElementById("tools-view"),
  toolsGrid: document.getElementById("tools-grid"),
  toolsCount: document.getElementById("tools-count"),
  toolsSearch: document.getElementById("tools-search"),
  toolsTabs: document.getElementById("tools-tabs"),
  toolsEmpty: document.getElementById("tools-empty"),
  // Widgets
  widgetsGrid: document.getElementById("widgets-grid"),
  // Market
  marketView: document.getElementById("market-view"),
  marketGrid: document.getElementById("market-grid"),
  marketEmpty: document.getElementById("market-empty"),
  skinsEmpty: document.getElementById("skins-empty"),
  marketError: document.getElementById("market-error"),
  marketBack: document.getElementById("market-back"),
  marketRefresh: document.getElementById("market-refresh"),
  marketTabs: document.getElementById("market-tabs"),
  marketBanner: document.getElementById("tools-market-btn"),
};

let qqMessage = "";
let currentBackend = "local";
let agentHistoryLoaded = false;
let toolsCache = [];
let toolsSearchQuery = "";
let toolsActiveCat = "all";
let marketCache = [];
let installedCache = [];
let currentSkinId = "";
let marketTab = "plugins";
let marketBusyId = "";
let toolPluginsCache = [];

// Browser-only examples for visual preview. Real transformations always come
// from the selected local/cloud model through Tauri.
function transform(mode, text) {
  if (mode === "polish") return "一轮圆月皎洁明亮，格外动人。";
  if (mode === "proofread") return text.replace(/[.。]*$/, "。");
  if (mode === "prompt-enhance") return "# 目标\n请围绕以下内容完成任务：\n" + text;
  return text;
}

// ---- Character-level LCS diff (mirror assistant-core::diff) ----

function diffChars(oldStr, newStr) {
  const a = Array.from(oldStr);
  const b = Array.from(newStr);
  const n = a.length;
  const m = b.length;
  const dp = Array.from({ length: n + 1 }, () => new Int32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const ops = [];
  const push = (kind, ch) => {
    const last = ops[ops.length - 1];
    if (last && last.kind === kind) last.text += ch;
    else ops.push({ kind, text: ch });
  };
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) push("equal", a[i++]), j++;
    else if (dp[i + 1][j] >= dp[i][j + 1]) push("del", a[i++]);
    else push("ins", b[j++]);
  }
  while (i < n) push("del", a[i++]);
  while (j < m) push("ins", b[j++]);
  return ops;
}

// ---- Rendering ----

function renderDiff(ops) {
  el.diff.replaceChildren();
  let inserted = 0;
  let deleted = 0;
  for (const op of ops) {
    if (op.kind === "equal") {
      el.diff.appendChild(document.createTextNode(op.text));
    } else {
      const span = document.createElement("span");
      span.className = op.kind === "ins" ? "ins" : "del";
      span.textContent = op.text;
      el.diff.appendChild(span);
      if (op.kind === "ins") inserted += Array.from(op.text).length;
      else deleted += Array.from(op.text).length;
    }
  }
  if (state.warning) {
    const note = document.createElement("div");
    note.className = "quality-note";
    const text = document.createElement("span");
    text.textContent = state.warning;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "换一个版本";
    retry.addEventListener("click", refreshPreview);
    note.append(text, retry);
    el.diff.prepend(note);
  }
  el.statAdd.textContent = "+" + inserted;
  el.statDel.textContent = "-" + deleted;
}

function renderPreviewState(kind, title, detail = "", source = "", retry = false) {
  el.diff.replaceChildren();
  const box = document.createElement("div");
  box.className = "preview-state " + kind;
  const heading = document.createElement("strong");
  heading.textContent = title;
  box.appendChild(heading);
  if (detail) {
    const text = document.createElement("span");
    text.textContent = detail;
    box.appendChild(text);
  }
  if (source) {
    const original = document.createElement("p");
    original.className = "preview-source";
    original.textContent = "原文：" + source;
    box.appendChild(original);
  }
  if (retry) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "preview-retry";
    button.textContent = "重新生成";
    button.addEventListener("click", refreshPreview);
    box.appendChild(button);
  }
  el.diff.appendChild(box);
  el.statAdd.textContent = "+0";
  el.statDel.textContent = "-0";
}

let loadingDismissed = false;

function dismissStatus() {
  clearTimeout(showStatus._t);
  if (el.status.querySelector(".spinner")) loadingDismissed = true;
  el.status.hidden = true;
}

function statusContents(message, spinner = false) {
  el.status.replaceChildren();
  if (spinner) {
    const spin = document.createElement("span");
    spin.className = "spinner";
    el.status.appendChild(spin);
  }
  const text = document.createElement("span");
  text.className = "status-message";
  text.textContent = message;
  const close = document.createElement("button");
  close.type = "button";
  close.className = "status-close";
  close.setAttribute("aria-label", "关闭提示");
  close.textContent = "×";
  close.addEventListener("click", dismissStatus, { once: true });
  el.status.append(text, close);
}

function showStatus(message, kind) {
  clearTimeout(showStatus._t);
  el.status.className = "status " + kind;
  statusContents(message);
  el.status.hidden = false;
  showStatus._t = setTimeout(dismissStatus, kind === "err" ? 5000 : 2600);
}

// Persistent loading state: the banner can be dismissed without cancelling the
// task, while the apply lock remains until the backend reports readiness.
function showLoading(message, lockApply = true) {
  clearTimeout(showStatus._t);
  if (!loadingDismissed) {
    el.status.className = "status warn";
    statusContents(message, true);
    el.status.hidden = false;
  }
  if (lockApply) setApplyLocked(true);
}

function hideLoading(unlockApply = true) {
  clearTimeout(showStatus._t);
  el.status.hidden = true;
  loadingDismissed = false;
  if (unlockApply) setApplyLocked(false);
}

/// Enable/disable the apply action (button + Ctrl+Enter) as one switch.
let applyLocked = false;
function setApplyLocked(locked) {
  applyLocked = locked;
  el.applyBtn.disabled = locked;
}

// ---- Model task progress ----

let progressTimer = null;
let progressPoll = null;
let progressStartedAt = 0;
let progressKey = "";
let progressMode = "polish";
let progressSourceChars = 0;

function progressAction() {
  return {
    polish: "润色",
    proofread: "纠错",
    "prompt-enhance": "提示词增强",
  }[progressMode] || "改写";
}

function setProgress(stage, detail, percent = null) {
  el.progressStage.textContent = stage;
  el.progressDetail.textContent = detail;
  if (percent == null) {
    el.progressBar.classList.add("indeterminate");
    el.progressBar.style.width = "28%";
  } else {
    el.progressBar.classList.remove("indeterminate");
    el.progressBar.style.transform = "none";
    el.progressBar.style.width = Math.max(2, Math.min(100, percent)) + "%";
  }
}

async function updateModelProgress() {
  const elapsed = (performance.now() - progressStartedAt) / 1000;
  el.progressTime.textContent = elapsed.toFixed(1) + "s";

  if (invoke && currentBackend === "local") {
    try {
      const progress = await invoke("model_progress");
      if (progress.phase === "download") {
        const percent = progress.total > 0 ? progress.current / progress.total * 100 : null;
        const current = (progress.current / 1024 / 1024).toFixed(0);
        const total = progress.total > 0 ? (progress.total / 1024 / 1024).toFixed(0) : "?";
        setProgress("首次使用：正在下载本地模型", `已下载 ${current} / ${total} MB，下载完成后会自动继续`, percent);
        return;
      }
      if (progress.phase === "load") {
        setProgress("正在载入 Qwen2.5 1.5B", "正在解析约 1.1GB 权重，通常需要数秒", null);
        return;
      }
      if (progress.phase === "inference") {
        const percent = progress.total > 0 ? progress.current / progress.total * 100 : null;
        const detail = progressSourceChars >= 80
          ? `已生成 ${progress.current} 个 token；正在保留全部信息并丰富表达，本地最长等待 120 秒`
          : `已生成 ${progress.current} 个 token；正在检查内容是否比原文更丰富且原意不变`;
        setProgress(`正在生成${progressAction()}结果`, detail, percent);
        return;
      }
      if (progress.phase === "error") {
        setProgress("本地模型加载失败", "请检查网络或在模型设置中切换云端后端", null);
        return;
      }
    } catch {
      /* Keep the staged fallback below if progress IPC is temporarily busy. */
    }
  }

  if (elapsed < 1.2) setProgress("正在读取原文", "识别句式、对象和表达意图", null);
  else if (elapsed < 3.5) setProgress("正在判断语言场景", "匹配聊天、描写、工作、技术或正式语体", null);
  else if (elapsed < 7) setProgress("正在保留原意并丰富表达", "补充表达层次、逻辑衔接和细节，同时检查事实不被改变", null);
  else if (currentBackend === "cloud") setProgress("云端模型正在丰富润色", "等待模型返回更完整、更充实的表达并执行质量检查", null);
  else setProgress("本地模型正在丰富润色", progressSourceChars >= 80 ? "长文本最长等待 120 秒；追求更高质量和速度建议切换云端" : "正在生成比原文更丰富的表达，请稍候", null);
}

function startTaskProgress(mode, source) {
  const key = mode + "\u0000" + source;
  if (!el.taskProgress.hidden && progressKey === key) return;
  stopTaskProgress(false);
  renderPreviewState("working", "正在生成预览…", `已读取 ${Array.from(source).length} 个字符，处理完成后将在这里显示结果`, source);
  progressKey = key;
  progressMode = mode;
  progressSourceChars = Array.from(source.trim()).length;
  progressStartedAt = performance.now();
  el.taskProgress.hidden = false;
  el.diff.setAttribute("aria-busy", "true");
  setApplyLocked(true);
  setProgress("正在准备丰富润色", "灵犀会先识别原文场景，再在保留全部原意的前提下扩展表达", null);
  updateModelProgress();
  progressTimer = setInterval(() => {
    const elapsed = (performance.now() - progressStartedAt) / 1000;
    el.progressTime.textContent = elapsed.toFixed(1) + "s";
  }, 100);
  progressPoll = setInterval(updateModelProgress, 450);
}

function stopTaskProgress(unlockApply = true) {
  clearInterval(progressTimer);
  clearInterval(progressPoll);
  progressTimer = null;
  progressPoll = null;
  progressKey = "";
  el.taskProgress.hidden = true;
  el.diff.removeAttribute("aria-busy");
  if (unlockApply) setApplyLocked(false);
}

// ---- Data flow: real backend via Tauri, or local mock in the browser ----

async function refreshPreview() {
  const version = (refreshPreview._version || 0) + 1;
  refreshPreview._version = version;
  if (invoke) {
    const mode = state.mode;
    const source = state.source;
    state.warning = null;
    startTaskProgress(mode, source);
    try {
      const res = await invoke("preview_transform", { mode, text: source });
      // A slower previous mode must never overwrite the newer selection/mode.
      if (version !== refreshPreview._version) return;
      state.transformed = res.transformed;
      state.diff = res.diff.map((d) => ({ kind: d.kind, text: d.text }));
      state.warning = res.warning || null;
      if (res.pending) {
        renderPreviewState("working", "本地模型仍在准备", "首次使用需下载并载入约 1.1GB 模型，进度会显示在上方", source);
        clearTimeout(refreshPreview._retry);
        refreshPreview._retry = setTimeout(refreshPreview, 1200);
      } else {
        clearTimeout(refreshPreview._retry);
        stopTaskProgress();
      }
    } catch (e) {
      if (version !== refreshPreview._version) return;
      clearTimeout(refreshPreview._retry);
      stopTaskProgress();
      const message = String(e);
      const timedOut = message.includes("exceeded") || message.includes("seconds");
      const rejected = message.includes("rejected") || message.includes("truncated");
      const title = timedOut ? "本地模型处理超时" : rejected ? "结果未通过质量检查" : "预览生成失败";
      const detail = timedOut
        ? "不是字数超过限制，而是本地 CPU 在时限内未生成完整结果。可缩短段落或在“模型设置”切换云端。"
        : rejected
          ? "模型返回了截断、句式功能改变或异常扩写的内容，因此已拦截，不会写回原文；请重新生成或切换云端。"
          : message.replace(/^.*?:\s*/, "") || "请重试或切换模型后端。";
      renderPreviewState("error", title, detail, source, true);
      showStatus(title, "err");
      return;
    }
  } else {
    state.transformed = transform(state.mode, state.source);
    state.diff = diffChars(state.source, state.transformed);
  }
  renderDiff(state.diff);
}

// Poll the backend for a freshly captured selection (replaces event listening
// so we do not depend on the Emitter API).
let lastSelectionRevision = null;
async function pollSelection() {
  if (!invoke) return;
  try {
    const selection = await invoke("current_selection");
    if (selection.revision !== lastSelectionRevision) {
      lastSelectionRevision = selection.revision;
      state.source = selection.text;
      if (state.source.trim()) {
        // 面板默认停在「对话」，热键抓到新选区时主动切到改写视图看预览。
        showView("rewrite");
        await refreshPreview();
      } else {
        renderPreviewState("empty", "未读取到选中的文字", "在任意应用里选中一段文字，再按 Ctrl+Alt+Space 打开面板");
      }
    }
  } catch {
    renderPreviewState("error", "无法读取当前选区", "请关闭浮窗后重新选择文字并按快捷键。");
  }
}

// Manual button: grab the current selection without the hotkey. Safe while the
// panel is visible — the rewrite view keeps it non-activating, so the selection
// focus stays in the source app; pollSelection picks up the new revision.
async function recaptureSelection() {
  if (!invoke) return;
  el.recaptureBtn.disabled = true;
  try {
    const ok = await invoke("trigger_transform");
    if (ok) {
      showStatus("已抓取选中文字，正在生成预览…", "ok");
    } else {
      showStatus("没抓到选区，请先在别的应用里选中一段文字", "warn");
    }
  } catch (e) {
    showStatus("读取选区失败: " + e, "err");
  } finally {
    el.recaptureBtn.disabled = false;
  }
}

// ---- Backend settings + QQ semi-automatic assistant ----

function showView(view) {
  dismissStatus();
  const rewrite = view === "rewrite";
  el.rewriteView.hidden = !rewrite;
  el.rewriteActions.hidden = !rewrite;
  el.qqView.hidden = view !== "qq";
  el.settingsPanel.hidden = view !== "settings";
  el.agentView.hidden = view !== "agent";
  el.toolsView.hidden = view !== "tools";
  el.marketView.hidden = view !== "market";
  el.modes.hidden = !rewrite;
  // B4-1：设置页打开时刷新权限规则列表。
  if (view === "settings") loadPermissionRules();
  const tabs = [el.agentTab, el.toolsTab, el.settingsBtn];
  for (const tab of tabs) tab.classList.remove("is-active");
  // 改写 / QQ 草稿已收进「工具」页的快捷卡片，打开时高亮「工具」tab。
  const map = { rewrite: el.toolsTab, qq: el.toolsTab, agent: el.agentTab, tools: el.toolsTab, market: el.toolsTab, settings: el.settingsBtn };
  if (map[view]) map[view].classList.add("is-active");
  // The rewrite panel must stay non-activating so write-back's focus-drift
  // check passes; settings/QQ/agent/tools need real keyboard focus.
  if (invoke) {
    invoke("set_panel_focusable", { focusable: !rewrite }).catch(() => {});
  }
  if (view === "tools") { loadWidgets(); loadTools(); }
  if (view === "market") loadMarket();
  if (view === "agent") loadAgentHistory();
}

/// 「云端 API」相关字段只在云端接入时显示（本地小模型不需要 Endpoint / Key）。
function syncCloudFields() {
  const cloud = el.backendSelect.value === "cloud";
  for (const node of document.querySelectorAll("[data-cloud-only]")) {
    node.classList.toggle("is-hidden", !cloud);
  }
}

/// Key 状态徽标：一眼看清"有没有配"。
function renderKeyStatus(configured) {
  if (!el.keyStatus) return;
  el.keyStatus.textContent = configured ? "已配置" : "未配置";
  el.keyStatus.classList.toggle("is-ok", Boolean(configured));
}

async function loadSettings() {
  if (!invoke) return;
  try {
    const settings = await invoke("get_backend_settings");
    currentBackend = settings.backend;
    el.providerPreset.value = "";
    el.backendSelect.value = settings.backend;
    el.endpointInput.value = settings.endpoint;
    el.modelInput.value = settings.model;
    el.rememberApiKey.checked = Boolean(settings.remember_api_key);
    el.backendBadge.textContent = settings.backend === "cloud" ? "云端" : "本地";
    renderKeyStatus(settings.api_key_configured);
    syncCloudFields();
    el.keyNote.textContent = settings.api_key_configured
      ? settings.remember_api_key
        ? "API Key 已由当前 Windows 账户加密保存；留空可保持不变。"
        : "API Key 已配置（仅本次运行内存中；留空可保持不变）。"
      : "未配置 API Key；也可使用 LINGXI_OPENAI_API_KEY 环境变量。";
    // B5-2 / B4-4：热键与剪贴板监听开关回填。
    if (el.panelHotkey) el.panelHotkey.value = settings.hotkey_panel || "Ctrl+Alt+D";
    if (el.clipboardWatch)
      el.clipboardWatch.checked = settings.clipboard_history_enabled !== false;
    // A8-1：设置页打开时同步定时任务（引擎未运行时静默失败有专门文案）。
    if (el.autoList) refreshAutomations();
  } catch (e) {
    showStatus("读取设置失败: " + e, "err");
  }
}

// ---- B4-3 首启引导（onboarding）：仅首次运行弹出，完成/跳过后不再出现 ----

async function initOnboarding() {
  if (!invoke || !el.onboarding) return;
  try {
    const settings = await invoke("get_backend_settings");
    if (settings.onboarding_done) return;
    el.onboardingKeyState.textContent = settings.api_key_configured
      ? "已配置 ✓"
      : "尚未配置";
    try {
      const status = await invoke("owo_status");
      const reachable = Boolean(status && status.reachable);
      el.onboardingEngineState.textContent = reachable ? "运行中 ✓" : "未启动";
      if (reachable) {
        el.onboardingStartEngine.disabled = true;
        el.onboardingStartEngine.textContent = "引擎运行中";
      }
    } catch {
      el.onboardingEngineState.textContent = "未启动";
    }
    el.onboarding.hidden = false;
  } catch {
    /* 设置读取失败：不打扰用户 */
  }
}

async function finishOnboarding() {
  if (el.onboarding) el.onboarding.hidden = true;
  try {
    await invoke("complete_onboarding");
  } catch {
    /* 标记失败：下次启动会再提示，不影响使用 */
  }
}

// One-click cloud provider presets. Selecting one fills the endpoint/model and
// switches the backend to cloud; the user only needs to paste their API key.
// All targets speak the OpenAI-compatible chat/completions protocol.
const PROVIDER_PRESETS = {
  deepseek: { endpoint: "https://api.deepseek.com", model: "deepseek-chat" },
  dashscope: {
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus",
  },
  openai: { endpoint: "https://api.openai.com", model: "gpt-4o-mini" },
};

function applyProviderPreset() {
  const preset = PROVIDER_PRESETS[el.providerPreset.value];
  if (!preset) return;
  el.backendSelect.value = "cloud";
  syncCloudFields();
  el.endpointInput.value = preset.endpoint;
  el.modelInput.value = preset.model;
  el.apiKeyInput.focus();
  showStatus("已填入预设，粘贴 API Key 后点保存即可", "ok");
}

async function saveSettings() {
  if (!invoke) return;
  try {
    const settings = await invoke("save_backend_settings", { input: {
      backend: el.backendSelect.value,
      endpoint: el.endpointInput.value,
      model: el.modelInput.value,
      api_key: el.apiKeyInput.value,
      remember_api_key: el.rememberApiKey.checked,
    }});
    el.apiKeyInput.value = "";
    currentBackend = settings.backend;
    el.backendBadge.textContent = settings.backend === "cloud" ? "云端" : "本地";
    renderKeyStatus(settings.api_key_configured);
    // 保存后留在设置页（用户可能还要配引擎），仅提示成功。
    showStatus("模型设置已保存", "ok");
  } catch (e) {
    showStatus("保存失败: " + e, "err");
  }
}

// ---- Pet skin settings ----

let petSkinsCache = [];

function renderSkinGrid(skins, activeId) {
  // 皮肤网格已从设置页移除（A8-2 桌宠精简：形象由工作台/配置决定，不再提供切换入口）。
  if (!el.skinGrid) return;
  el.skinGrid.replaceChildren();
  if (!skins.length) {
    const hint = document.createElement("div");
    hint.className = "skin-empty";
    hint.textContent = "未找到皮肤包。将皮肤文件夹放入 assets/skins/ 后重新打开";
    el.skinGrid.appendChild(hint);
    return;
  }
  for (const skin of skins) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "skin-card" + (skin.id === activeId ? " is-active" : "");
    card.title = skin.description || skin.name;
    // spritesheet 皮肤缩略图是 `<sheet>#<row>[:cols[:rows]]`，
    // 用 div 背景取该行首列第一帧；缺省网格按 8×9 兜底。
    const hash = skin.thumbnail ? skin.thumbnail.indexOf("#") : -1;
    let thumb;
    if (hash === -1) {
      thumb = document.createElement("img");
      thumb.src = skin.thumbnail;
      thumb.alt = skin.name;
      thumb.draggable = false;
    } else {
      const meta = skin.thumbnail.slice(hash + 1).split(":");
      const row = parseInt(meta[0], 10) || 0;
      const cols = parseInt(meta[1], 10) || 8;
      const rows = parseInt(meta[2], 10) || 9;
      thumb = document.createElement("div");
      thumb.className = "thumb-anim";
      thumb.style.backgroundImage = `url("${skin.thumbnail.slice(0, hash)}")`;
      thumb.style.backgroundSize = `${cols * 100}% ${rows * 100}%`;
      thumb.style.backgroundPosition =
        `0% ${(rows > 1 ? (row / (rows - 1)) * 100 : 0).toFixed(2)}%`;
    }
    const name = document.createElement("span");
    name.textContent = skin.name;
    const meta = document.createElement("small");
    meta.textContent = [skin.author, skin.version].filter(Boolean).join(" · ");
    card.append(thumb, name, meta);
    card.addEventListener("click", async () => {
      if (skin.id === activeId) return;
      try {
        // 切换即时生效（后端会广播 pet-config-changed，桌宠窗口热换）。
        const view = await invoke("set_pet_skin", { skinId: skin.id });
        renderSkinGrid(petSkinsCache, view.skin.id);
        showStatus("已切换为「" + skin.name + "」", "ok");
      } catch (e) {
        showStatus("切换皮肤失败: " + e, "err");
      }
    });
    el.skinGrid.appendChild(card);
  }
}

// ---- 插件市场 ----

async function loadMarket() {
  if (!invoke) return;
  el.marketRefresh.disabled = true;
  try {
    const [items, skins, config, tools] = await Promise.all([
      invoke("market_list"),
      invoke("list_pet_skins"),
      invoke("current_pet_config"),
      invoke("list_tool_plugins"),
    ]);
    marketCache = items;
    installedCache = skins;
    currentSkinId = config.skin.id;
    toolPluginsCache = tools;
    el.marketError.hidden = true;
  } catch (e) {
    el.marketError.textContent = "市场加载失败：" + e;
    el.marketError.hidden = false;
  } finally {
    el.marketRefresh.disabled = false;
  }
  renderMarketView();
}

function renderMarketView() {
  el.marketGrid.textContent = "";
  // 同步页签激活态（含初始状态与刷新后的重渲染）
  for (const node of el.marketTabs.children) {
    node.classList.toggle("is-active", node.dataset.tab === marketTab);
  }
  // 已安装融入各自板块：renderMarketCard 按 installed_source 显示状态与操作
  if (marketTab === "plugins") {
    const plugins = marketCache.filter((item) => item.kind === "tool");
    el.marketEmpty.hidden = plugins.length > 0;
    el.skinsEmpty.hidden = true;
    for (const item of plugins) el.marketGrid.appendChild(renderMarketCard(item));
  } else {
    const skins = marketCache.filter((item) => item.kind !== "tool");
    el.marketEmpty.hidden = true;
    el.skinsEmpty.hidden = skins.length > 0;
    for (const item of skins) el.marketGrid.appendChild(renderMarketCard(item));
  }
}

// 缩略图兼容两种格式：纯图片 URL，或「URL#row,cols,rows」雪碧图帧
//（与 renderSkinGrid 的解析规则一致）。
function applyThumbnail(thumb, thumbnail) {
  if (!thumbnail) return;
  const hash = thumbnail.indexOf("#");
  if (hash === -1) {
    thumb.style.backgroundImage = `url("${thumbnail}")`;
    return;
  }
  const meta = thumbnail.slice(hash + 1).split(",").map((n) => parseInt(n, 10) || 0);
  const row = meta[0] || 0;
  const cols = meta[1] || 8;
  const rows = meta[2] || 9;
  thumb.style.backgroundImage = `url("${thumbnail.slice(0, hash)}")`;
  thumb.style.backgroundSize = `${cols * 100}% ${rows * 100}%`;
  thumb.style.backgroundPosition = `0% ${(rows > 1 ? (row / (rows - 1)) * 100 : 0).toFixed(2)}%`;
}

function renderMarketCard(item) {
  const card = document.createElement("div");
  card.className = "market-card";
  const thumb = document.createElement("div");
  thumb.className = "market-card-thumb";
  applyThumbnail(thumb, item.thumbnail);
  const name = document.createElement("span");
  name.className = "market-card-name";
  name.textContent = item.name;
  const meta = document.createElement("small");
  meta.className = "market-card-meta";
  meta.textContent = [item.kind === "tool" ? "工具插件" : "皮肤", item.author, item.version].filter(Boolean).join(" · ");
  const desc = document.createElement("p");
  desc.className = "market-card-desc";
  desc.textContent = item.description || "";
  const foot = document.createElement("div");
  foot.className = "market-card-foot";
  if (item.installed_source === "builtin" || item.updatable || item.installed_source === "user") {
    const badge = document.createElement("span");
    badge.className = "market-state-badge" + (item.updatable ? " updatable" : "");
    if (item.updatable) {
      badge.textContent = "可更新";
    } else if (item.installed_source === "builtin") {
      badge.textContent = "内置";
    } else {
      badge.textContent =
        item.kind !== "tool" && item.id === currentSkinId ? "使用中" : "已安装";
      if (item.kind !== "tool" && item.id === currentSkinId) badge.classList.add("current");
    }
    foot.append(badge);
  }
  const btn = document.createElement("button");
  btn.className = "market-action";
  if (marketBusyId === item.id) {
    btn.textContent = "下载中…";
    btn.disabled = true;
  } else if (item.installed_source === "builtin") {
    btn.textContent = "已内置";
    btn.disabled = true;
  } else if (item.installed_source === "user" && !item.updatable) {
    btn.textContent = "删除";
    btn.classList.add("danger");
    btn.addEventListener("click", () =>
      item.kind === "tool" ? uninstallPlugin(item) : uninstallSkin(item)
    );
  } else {
    btn.textContent = item.updatable ? "更新" : "安装";
    btn.addEventListener("click", () => installSkin(item.id));
  }
  foot.append(btn);
  card.append(thumb, name, meta, desc, foot);
  return card;
}

function renderInstalledCard(skin) {
  const card = document.createElement("div");
  card.className = "market-card";
  const thumb = document.createElement("div");
  thumb.className = "market-card-thumb";
  applyThumbnail(thumb, skin.thumbnail);
  const name = document.createElement("span");
  name.className = "market-card-name";
  name.textContent = skin.name;
  const meta = document.createElement("small");
  meta.className = "market-card-meta";
  meta.textContent = [skin.author, skin.version].filter(Boolean).join(" · ");
  const foot = document.createElement("div");
  foot.className = "market-card-foot";
  if (skin.source !== "user") {
    const badge = document.createElement("span");
    badge.className = "market-state-badge";
    badge.textContent = "内置";
    foot.append(badge);
  } else {
    if (skin.id === currentSkinId) {
      const badge = document.createElement("span");
      badge.className = "market-state-badge current";
      badge.textContent = "使用中";
      foot.append(badge);
    }
    const del = document.createElement("button");
    del.className = "market-action danger";
    del.textContent = marketBusyId === skin.id ? "删除中…" : "删除";
    del.disabled = Boolean(marketBusyId);
    del.addEventListener("click", () => uninstallSkin(skin));
    foot.append(del);
  }
  card.append(thumb, name, meta, foot);
  return card;
}

function renderToolPluginCard(plugin) {
  const card = document.createElement("div");
  card.className = "market-card";
  const thumb = document.createElement("div");
  thumb.className = "market-card-thumb is-tool";
  const name = document.createElement("span");
  name.className = "market-card-name";
  name.textContent = plugin.display_name || plugin.name;
  const meta = document.createElement("small");
  meta.className = "market-card-meta";
  meta.textContent = ["工具插件", plugin.author, plugin.version].filter(Boolean).join(" · ");
  const desc = document.createElement("p");
  desc.className = "market-card-desc";
  desc.textContent = plugin.description || "";
  const foot = document.createElement("div");
  foot.className = "market-card-foot";
  const del = document.createElement("button");
  del.className = "market-action danger";
  del.textContent = marketBusyId === plugin.id ? "删除中…" : "删除";
  del.disabled = Boolean(marketBusyId);
  del.addEventListener("click", () => uninstallPlugin(plugin));
  foot.append(del);
  card.append(thumb, name, meta, desc, foot);
  return card;
}

async function uninstallPlugin(plugin) {
  if (!invoke || marketBusyId) return;
  const label = plugin.display_name || plugin.name;
  if (!confirm(`确定删除工具插件「${label}」吗？此操作不可撤销。`)) return;
  marketBusyId = plugin.id;
  renderMarketView();
  try {
    await invoke("market_uninstall", { id: plugin.id });
    showStatus("已删除「" + label + "」", "ok");
  } catch (e) {
    showStatus("删除失败: " + e, "err");
  } finally {
    marketBusyId = "";
    await loadMarket();
  }
}

async function installSkin(id) {
  if (!invoke || marketBusyId) return;
  marketBusyId = id;
  renderMarketView();
  try {
    await invoke("market_install", { id });
    showStatus("安装完成", "ok");
  } catch (e) {
    showStatus("安装失败: " + e, "err");
  } finally {
    marketBusyId = "";
    await loadMarket();
  }
}

async function uninstallSkin(skin) {
  if (!invoke || marketBusyId) return;
  const hint = skin.id === currentSkinId ? "该皮肤正在使用中，删除后将回落到默认皮肤。" : "";
  if (!confirm(`确定删除皮肤「${skin.name}」吗？${hint}此操作不可撤销。`)) return;
  marketBusyId = skin.id;
  renderMarketView();
  try {
    await invoke("market_uninstall", { id: skin.id });
    showStatus("已删除「" + skin.name + "」", "ok");
  } catch (e) {
    showStatus("删除失败: " + e, "err");
  } finally {
    marketBusyId = "";
    await loadMarket();
  }
}

el.marketBanner.addEventListener("click", () => showView("market"));
el.marketBack.addEventListener("click", () => showView("tools"));
el.marketRefresh.addEventListener("click", () => loadMarket());
el.marketTabs.addEventListener("click", (event) => {
  const tab = event.target.closest(".market-tab");
  if (!tab || tab.dataset.tab === marketTab) return;
  marketTab = tab.dataset.tab;
  for (const node of el.marketTabs.children) {
    node.classList.toggle("is-active", node === tab);
  }
  renderMarketView();
});

function fillPetSettings(view) {
  if (!el.bubbleIdle) return; // 桌宠设置卡缺失时跳过（精简形态）
  renderSkinGrid(petSkinsCache, view.skin.id);
  el.bubbleIdle.value = (view.overrides && view.overrides.idle) || "";
  el.bubbleThinking.value = (view.overrides && view.overrides.thinking) || "";
  el.bubbleSpeaking.value = (view.overrides && view.overrides.speaking) || "";
  el.bubbleAlert.value = (view.overrides && view.overrides.alert) || "";
  el.petVisible.checked = Boolean(view.visible);
}

async function loadPetSettings() {
  if (!invoke) {
    // 浏览器预览没有皮肤数据，至少把空态占位画出来，避免区域塌陷。
    renderSkinGrid([], "");
    return;
  }
  try {
    const [skins, view] = await Promise.all([
      invoke("list_pet_skins"),
      invoke("current_pet_config"),
    ]);
    petSkinsCache = skins;
    fillPetSettings(view);
  } catch (e) {
    showStatus("读取桌宠设置失败: " + e, "err");
  }
}

async function savePetOptions() {
  if (!invoke) return;
  try {
    const overrides = {
      idle: el.bubbleIdle.value,
      thinking: el.bubbleThinking.value,
      speaking: el.bubbleSpeaking.value,
      alert: el.bubbleAlert.value,
    };
    await invoke("set_pet_options", { overrides, visible: el.petVisible.checked });
    showStatus("桌宠设置已保存", "ok");
  } catch (e) {
    showStatus("保存桌宠设置失败: " + e, "err");
  }
}

// ---- Window behavior settings ----

async function loadWindowOptions() {
  if (!invoke) return;
  try {
    const options = await invoke("get_window_options");
    el.panelAutoHide.checked = Boolean(options.panel_auto_hide);
    el.panelRememberPos.checked = Boolean(options.panel_remember_position);
  } catch (e) {
    showStatus("读取窗口设置失败: " + e, "err");
  }
}

async function saveWindowOptions() {
  if (!invoke) return;
  try {
    await invoke("set_window_options", {
      panelAutoHide: el.panelAutoHide.checked,
      panelRememberPos: el.panelRememberPos.checked,
    });
    showStatus("窗口设置已保存", "ok");
  } catch (e) {
    showStatus("保存窗口设置失败: " + e, "err");
  }
}

async function readQqMessage() {
  if (!invoke) return;
  // First check QQ is foreground so we can show a friendly error instead of
  // capturing a selection from some other application.
  try {
    const poll = await invoke("qq_poll_latest");
    if (!poll) {
      showStatus("请先把 QQ 聊天窗口切到前台", "warn");
      return;
    }
    el.qqConversation.textContent = poll.conversation || "QQ 会话";
  } catch (e) {
    showStatus("QQ 状态检查失败: " + e, "err");
    return;
  }
  // Now read the user's current selection. The user must have selected the
  // message they want to reply to before clicking this button.
  try {
    showLoading("正在读取选区…", false);
    const result = await invoke("capture_qq_selection");
    hideLoading(false);
    qqMessage = result.message;
    el.qqConversation.textContent = result.conversation || "QQ 会话";
    el.qqMessage.textContent = result.message;
    if (result.message) {
      showStatus("已读取选中消息，可生成回复草稿", "ok");
    }
  } catch (e) {
    hideLoading(false);
    showStatus("读取失败: " + e + "（请先在 QQ 里选中对方消息）", "err");
  }
}

async function generateQqDraft() {
  if (!invoke || !qqMessage) {
    showStatus("请先读取一条 QQ 消息", "warn");
    return;
  }
  el.qqGenerate.disabled = true;
  showLoading("正在生成回复草稿…", false);
  try {
    el.qqDraft.value = await invoke("generate_qq_draft", { message: qqMessage });
    hideLoading(false);
  } catch (e) {
    hideLoading(false);
    showStatus("草稿生成失败: " + e, "err");
  } finally {
    el.qqGenerate.disabled = false;
  }
}

async function writeQqDraft() {
  const draft = el.qqDraft.value.trim();
  if (!invoke || !draft) {
    showStatus("请先生成或填写草稿", "warn");
    return;
  }
  try {
    const result = await invoke("write_qq_draft", { draft });
    showStatus(result.verified ? "草稿已写入 QQ，请确认后手动发送" : "已尝试写入，请在 QQ 中确认", result.verified ? "ok" : "warn");
  } catch (e) {
    showStatus("写入 QQ 失败: " + e, "err");
  }
}

// ---- Actions ----

async function apply() {
  // Blocked while the model is still loading: applying now would write back a
  // not-yet-ready (no-op) result.
  if (applyLocked) {
    showStatus("模型仍在加载，请稍候…", "warn");
    return;
  }
  if (invoke) {
    try {
      await invoke("apply_transform", { mode: state.mode });
      showStatus("已应用改写", "ok");
      // Clear the workspace quickly so another selection can be made without
      // the always-on-top panel covering the editor.
      setTimeout(close, 650);
    } catch (e) {
      showStatus("应用失败: " + e, "err");
    }
  } else {
    showStatus("已应用改写 (预览模式)", "ok");
  }
}

async function undo() {
  if (invoke) {
    try {
      await invoke("undo_last");
      showStatus("已撤销", "ok");
      setTimeout(close, 650);
    } catch (e) {
      showStatus("撤销失败: " + e, "err");
    }
  } else {
    showStatus("已撤销 (预览模式)", "ok");
  }
}

function close() {
  if (invoke) {
    invoke("hide_overlay").catch(() => {});
  } else {
    el.card.style.display = "none";
  }
}

// ---- Wiring ----

el.modes.addEventListener("click", (e) => {
  const chip = e.target.closest(".chip");
  if (!chip || chip.disabled) return;
  for (const c of el.modes.querySelectorAll(".chip")) c.classList.remove("is-active");
  chip.classList.add("is-active");
  state.mode = chip.dataset.mode;
  refreshPreview();
});

el.applyBtn.addEventListener("click", apply);
el.undoBtn.addEventListener("click", undo);
el.closeBtn.addEventListener("click", close);
el.pinBtn.addEventListener("click", () => el.pinBtn.classList.toggle("is-on"));
const quitBtn = document.getElementById("quit-btn");
if (quitBtn) {
  quitBtn.addEventListener("click", async () => {
    const ok = await showConfirmDialog("退出灵犀", "确定退出灵犀？退出后桌宠和快捷键都会关闭。");
    if (ok) {
      if (invoke) invoke("quit_app").catch(() => {});
      else window.close();
    }
  });
}
// ---- Agent chat ----

async function loadAgentHistory() {
  if (!invoke || agentHistoryLoaded) return;
  try {
    const history = await invoke("agent_history");
    agentHistoryLoaded = true;
    if (!history.length) return;
    el.chatMessages.replaceChildren();
    for (const item of history) appendChatBubble(item.role, item.content);
  } catch (err) {
    showStatus("加载对话历史失败: " + err, "err");
  }
}

function appendChatBubble(role, text, opts) {
  const empty = el.chatMessages.querySelector(".chat-empty");
  if (empty) empty.remove();
  const bubble = document.createElement("div");
  bubble.className = "chat-bubble chat-" + role;
  // B2-1：assistant 回复用 markdown 渲染（代码块/列表/粗体），user 与系统提示保持纯文本。
  if (role === "assistant" && !(opts && opts.plain) && window.renderMarkdownInto) {
    const body = document.createElement("div");
    body.className = "chat-md";
    window.renderMarkdownInto(body, text);
    bubble.append(body);
  } else {
    bubble.textContent = text;
  }
  el.chatMessages.appendChild(bubble);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
  return bubble;
}

function appendToolCall(call) {
  const card = document.createElement("details");
  card.className = "chat-tool-card " + (call.success ? "tool-success" : "tool-failed");
  const summary = document.createElement("summary");
  const state = call.success ? "完成" : "未执行";
  summary.textContent = `${call.name} · ${state}`;
  const body = document.createElement("div");
  body.className = "chat-tool-body";
  const args = document.createElement("pre");
  args.textContent = JSON.stringify(call.arguments || {}, null, 2);
  const result = document.createElement("div");
  result.className = "chat-tool-result";
  result.textContent = call.result || "（无输出）";
  body.append(args, result);
  card.append(summary, body);
  el.chatMessages.appendChild(card);
}

async function sendChatMessage() {
  const msg = el.chatInput.value.trim();
  const hasAttachments = owo.pendingAttachments.length > 0;
  if (!msg && !hasAttachments) return;
  // 贴图走引擎多模态通道（/v1 messages 的 image 块 / OpenAI image_url），轻环不支持。
  if (hasAttachments && !owo.mode) {
    showStatus("带图片的任务需在「重任务」模式下发送", "err");
    return;
  }
  const attachments = owo.pendingAttachments.slice();
  owo.pendingAttachments = [];
  renderAttachChips();
  el.chatInput.value = "";
  const display = msg || "（见图片）";
  appendChatBubble(
    "user",
    display + (attachments.length ? `\n\n[图片 ×${attachments.length}]` : "")
  );
  // 重任务模式：交给本地 owo-agent 引擎（SSE 事件异步回流到聊天区）。
  if (owo.mode) {
    await sendOwoTask(display, attachments);
    return;
  }
  // Show a thinking indicator
  const thinking = document.createElement("div");
  thinking.className = "chat-bubble chat-assistant chat-thinking";
  thinking.textContent = "让我想想…";
  el.chatMessages.appendChild(thinking);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
  el.chatSend.disabled = true;
  try {
    if (!invoke) {
      thinking.textContent = "（浏览器预览模式，无法调用模型）";
      return;
    }
    const report = await invoke("agent_chat", { message: msg });
    thinking.remove();
    for (const call of report.tool_calls || []) appendToolCall(call);
    appendChatBubble("assistant", report.reply || "（模型未返回文字）");
  } catch (err) {
    thinking.remove();
    const message = String(err);
    appendChatBubble("error", message);
    if (message.includes("云端模型") || message.includes("Endpoint 和 API Key")) {
      const openSettings = document.createElement("button");
      openSettings.className = "mini-btn chat-settings-link";
      openSettings.textContent = "打开模型设置";
      openSettings.addEventListener("click", () => {
        showView("settings");
        loadSettings();
      });
      el.chatMessages.appendChild(openSettings);
    }
  } finally {
    el.chatSend.disabled = false;
    el.chatInput.focus();
  }
}

async function resetAgentChat() {
  if (!invoke) return;
  if (owo.mode) {
    // 重任务模式：丢弃当前会话引用，下次发送时自动新建（引擎侧历史保留在会话列表里）。
    owo.sessionId = null;
    owo.pendingTools.clear();
    owo.permissionCards.clear();
    el.chatMessages.innerHTML =
      '<div class="chat-empty">重任务新会话已就绪。描述你想完成的事，例如：<br>“在本机工作区里找一份日志并汇总 ERROR 行”</div>';
    return;
  }
  try {
    await invoke("agent_reset");
    agentHistoryLoaded = true;
    el.chatMessages.innerHTML = '<div class="chat-empty">新对话已开始。向灵犀描述你想做的事…</div>';
  } catch (err) {
    showStatus("重置失败: " + err, "err");
  }
}

// ---- 重任务引擎（owo-agent 桥接）----
//
// 与上面的轻环（agent_chat：单次变换/轻问答）并列：重任务把活交给本机
// owo-agent 引擎执行（可读写文件、跑命令、多步规划），每个危险工具调用都会
// 先弹审批卡片，由用户放行；结束后拉取 diff，可一键回滚。
// 后端命令：owo_status / owo_start_service / owo_ensure_session / owo_send /
// owo_permission / owo_diff / owo_revert；事件：owo://turn（SSE 帧）。

const owo = {
  mode: false,
  sessionId: null,
  busy: false,
  /// 当前进行中的回合：进度气泡与流式气泡引用。
  progressBubble: null,
  streamBubble: null,
  /// B2-4：当前回合的计划卡与思考折叠区引用（回合结束清理）。
  planCard: null,
  reasoningBox: null,
  /// tool_use 暂存（tool_result 到达时合并成一张卡片）。
  pendingTools: new Map(),
  permissionCards: new Map(),
  /// B3：待发送的贴图（{name, mime, dataB64, dataUrl}），发送时先上传引擎。
  pendingAttachments: [],
};

/// 桌宠可见性缓存（标题栏按钮状态用）。
let petVisibleCache = true;

// ---- 重任务引擎设置（工作文件夹 / 端口 / 自启 / 重启）----

async function loadEngineOptions() {
  if (!invoke) return;
  try {
    const options = await invoke("get_engine_options");
    el.engineWorkspace.value = options.workspace || "";
    el.engineWorkspace.placeholder = `留空 = ${options.workspace_effective}`;
    el.enginePort.value = options.port;
    el.engineAutoStart.checked = Boolean(options.auto_start);
    el.engineExe.value = options.exe_configured || "";
    el.engineModelNote.textContent = options.api_key_configured
      ? `引擎将使用：${options.model_endpoint || "（未配置端点）"} · ${options.model_name || "（未配置模型）"}（API Key 已配置）`
      : "尚未配置 API Key：请在上方「模型服务」里填写并保存，引擎启动时会自动带上。";
    if (options.exe_path) {
      el.engineStatusInline.textContent = `引擎程序：${options.exe_path}`;
    } else {
      el.engineStatusInline.textContent =
        "未找到引擎程序：安装包应带 resources/owo-agent.exe；开发机请先构建引擎或点右侧「选择…」指定路径。";
    }
  } catch (err) {
    el.engineStatusInline.textContent = "读取引擎设置失败：" + err;
  }
}

async function pickEngineExe() {
  if (!invoke) return;
  try {
    const path = await invoke("pick_engine_exe");
    if (path) {
      el.engineExe.value = path;
      el.engineStatusInline.textContent = `已选择：${path} —— 点「保存引擎设置」后生效`;
    }
  } catch (err) {
    showStatus("打开文件选择器失败: " + err, "err");
  }
}

async function saveEngineSettings() {
  if (!invoke) return;
  const port = parseInt(el.enginePort.value, 10);
  if (!Number.isFinite(port) || port <= 0 || port > 65535) {
    showStatus("端口需在 1-65535 之间", "err");
    return;
  }
  try {
    const options = await invoke("save_engine_options", {
      workspace: el.engineWorkspace.value.trim(),
      port,
      autoStart: el.engineAutoStart.checked,
      exePath: el.engineExe.value.trim(),
    });
    el.engineWorkspace.placeholder = `留空 = ${options.workspace_effective}`;
    if (options.exe_path) {
      el.engineStatusInline.textContent = `引擎程序：${options.exe_path}。模型/工作文件夹变更后请点「重启引擎」生效。`;
    }
    showStatus("引擎设置已保存", "ok");
  } catch (err) {
    showStatus("保存失败: " + err, "err");
  }
}

async function pickEngineFolder() {
  if (!invoke) return;
  try {
    const folder = await invoke("pick_workspace_folder");
    if (folder) {
      el.engineWorkspace.value = folder;
      el.engineStatusInline.textContent = `已选择：${folder} —— 保存后点「重启引擎」生效`;
    }
  } catch (err) {
    showStatus("打开文件夹选择器失败: " + err, "err");
  }
}

async function restartEngine() {
  if (!invoke) return;
  el.engineRestart.disabled = true;
  el.engineStatusInline.textContent = "正在重启引擎…";
  try {
    const status = await invoke("owo_restart_service");
    owoStatusCache = {
      at: Date.now(),
      reachable: Boolean(status.reachable),
      version: status.version || "",
      canStart: Boolean(status.exe_path),
    };
    owoRenderStatus(
      owoStatusCache.reachable,
      owoStatusCache.version,
      owoStatusCache.canStart
    );
    el.engineStatusInline.textContent = status.reachable
      ? `引擎已重启并在运行（v${status.version}），工作区已切换`
      : "引擎重启后未就绪：" + (status.error || "未知原因");
  } catch (err) {
    el.engineStatusInline.textContent = "重启失败：" + err;
  } finally {
    el.engineRestart.disabled = false;
  }
}

/// 空状态示例（点击直接填入输入框）。
const CHAT_SUGGESTIONS = {
  chat: ["读取剪贴板并帮我总结", "把这句话润色得正式一点", "帮我把这段话翻成英文"],
  task: [
    "把这个文件夹里的日志错误汇总成报告",
    "写一个 hello.py 并运行给我看结果",
    "找出项目里所有 TODO 注释并列出来",
  ],
};

function renderChatSuggestions() {
  if (!el.chatSuggestions) return;
  const task = owo.mode;
  if (el.chatEmptyTitle) el.chatEmptyTitle.textContent = task ? "重任务" : "和灵犀说话";
  if (el.chatEmptySub) {
    el.chatEmptySub.textContent = task
      ? "它会读写文件、跑命令，危险操作先请你批准"
      : "直接问，或让它帮你处理文字";
  }
  el.chatSuggestions.replaceChildren();
  for (const text of CHAT_SUGGESTIONS[task ? "task" : "chat"]) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "chat-suggestion";
    chip.textContent = text;
    chip.addEventListener("click", () => {
      el.chatInput.value = text;
      el.chatInput.focus();
    });
    el.chatSuggestions.append(chip);
  }
}

/// 模式分段：闲聊（轻环 agent_chat）/ 重任务（引擎 owo-agent）。
function setOwoMode(task) {
  owo.mode = Boolean(task);
  el.modeChat.classList.toggle("is-active", !owo.mode);
  el.modeTask.classList.toggle("is-active", owo.mode);
  el.modeEngine.classList.toggle("is-task", owo.mode);
  el.chatInput.placeholder = owo.mode
    ? "描述要完成的活：可读写文件、跑命令…"
    : "开始新对话…";
  renderChatSuggestions();
  refreshOwoStatus();
}

// ---- 语音输入（WebView2 的 Web Speech API；不可用时引导打字）----
let voiceRec = null;
let voiceActive = false;

function toggleVoiceInput() {
  const Ctor = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!Ctor) {
    showStatus(
      "当前环境不支持语音识别：可直接打字，或用 Ctrl+Alt+Space 抓取选区文字",
      "warn"
    );
    return;
  }
  if (voiceActive && voiceRec) {
    try {
      voiceRec.stop();
    } catch {
      /* 已停止 */
    }
    return;
  }
  voiceRec = new Ctor();
  voiceRec.lang = "zh-CN";
  voiceRec.interimResults = true;
  voiceRec.continuous = false;
  const base = el.chatInput.value;
  voiceActive = true;
  el.toolVoice.classList.add("is-recording");
  voiceRec.onresult = (event) => {
    let text = "";
    for (const result of event.results) text += result[0].transcript;
    el.chatInput.value = base ? `${base} ${text}` : text;
  };
  voiceRec.onerror = (event) => {
    showStatus("语音识别失败：" + (event.error || "未知错误"), "err");
  };
  voiceRec.onend = () => {
    voiceActive = false;
    el.toolVoice.classList.remove("is-recording");
    el.chatInput.focus();
  };
  try {
    voiceRec.start();
    showStatus("正在听…说完会自动结束", "ok");
  } catch (err) {
    voiceActive = false;
    el.toolVoice.classList.remove("is-recording");
    showStatus("语音启动失败: " + err, "err");
  }
}

function owoRenderStatus(reachable, version, canStart) {
  el.owoDot.classList.toggle("online", Boolean(reachable));
  el.owoStatusText.textContent = reachable
    ? `引擎在线${version ? ` · v${version}` : ""}`
    : canStart
      ? "引擎未启动"
      : "未找到引擎程序";
  el.owoStart.hidden = Boolean(reachable) || !canStart;
  // 引擎条只在「重任务模式」或「引擎不可用」时出现；闲聊且在线时不占空间。
  el.modeEngine.hidden = Boolean(reachable) && !owo.mode;
}

/// 5 秒内的探测结果缓存：切模式/切视图时避免重复网络往返。
let owoStatusCache = { at: 0, reachable: false, version: "", canStart: false };

/// 历史回放：引擎在线时拉一次当前会话的消息（对话区还是空的才回放）。
let owoHistoryLoaded = false;
async function loadOwoHistory() {
  if (!invoke || owoHistoryLoaded) return;
  owoHistoryLoaded = true;
  try {
    const detail = await invoke("owo_history");
    replaySessionDetail(detail, false);
  } catch {
    /* 引擎未启动 / 无会话时静默 */
  }
}

/// B3-2：把 /session/{id} 的 messages（ChatMessage 数组）重建为完整对话流——
/// 不再只回放纯文本：assistant(tool_calls) 重建为工具卡，tool 消息按
/// call_id 填充结果，压缩摘要显示为提示。
function replaySessionDetail(detail, force) {
  const messages = Array.isArray(detail && detail.messages) ? detail.messages : [];
  if (!messages.length) return false;
  if (
    !force &&
    el.chatMessages.querySelector(".chat-bubble, .chat-tool-card, .chat-plan-card")
  ) {
    return false; // 已有内容（例如正在进行的回合），不重复回放。
  }
  const MAX_MESSAGES = 80;
  const toolCards = new Map();
  for (const message of messages.slice(-MAX_MESSAGES)) {
    if (message.role === "user") {
      if (typeof message.content === "string" && message.content.trim()) {
        appendChatBubble("user", message.content);
      }
    } else if (message.role === "assistant") {
      if (Array.isArray(message.tool_calls)) {
        for (const call of message.tool_calls) {
          const entry = { tool: call.name || "工具", args: call.arguments || {} };
          const card = owoAppendToolCard(entry, null);
          toolCards.set(call.id, { entry, card });
        }
      }
      if (typeof message.content === "string" && message.content.trim()) {
        appendChatBubble("assistant", message.content);
      }
    } else if (message.role === "tool") {
      const found = toolCards.get(message.tool_call_id);
      const content = typeof message.content === "string" ? message.content : "";
      if (found && content) {
        const failed = content.startsWith("工具错误");
        owoToolCardContent(
          found.card,
          found.entry,
          failed
            ? { ok: false, error: content }
            : { ok: true, preview: content }
        );
      }
    } else if (
      message.role === "system" &&
      typeof message.content === "string" &&
      message.content.startsWith("历史摘要")
    ) {
      appendChatBubble("assistant", "（更早的历史已压缩为摘要，可在会话详情查看）");
    }
  }
  return true;
}

// ---- 历史会话面板（B3-1）：列表 / 切换 ----

async function refreshSessionsPanel() {
  if (!invoke) return;
  el.sessionsList.textContent = "加载中…";
  try {
    const sessions = await invoke("owo_sessions");
    el.sessionsList.replaceChildren();
    const visible = (Array.isArray(sessions) ? sessions : [])
      .filter((session) => !session.archived)
      .slice(0, 30);
    if (!visible.length) {
      el.sessionsList.textContent = "还没有历史会话";
      return;
    }
    for (const session of visible) {
      const row = document.createElement("button");
      row.type = "button";
      row.className =
        "session-row" + (session.id === owo.sessionId ? " is-active" : "");
      const title = document.createElement("span");
      title.className = "session-title";
      title.textContent =
        (session.pinned ? "📌 " : "") + (session.title || session.id.slice(0, 8));
      const meta = document.createElement("span");
      meta.className = "session-meta";
      meta.textContent = String(session.updated_at || "").replace("T", " ").slice(0, 16);
      row.append(title, meta);
      row.addEventListener("click", () => switchSession(session.id));
      el.sessionsList.append(row);
    }
  } catch (err) {
    el.sessionsList.textContent = "加载失败：" + err;
  }
}

async function switchSession(sessionId) {
  try {
    await invoke("owo_use_session", { sessionId });
    owo.sessionId = sessionId;
    el.sessionsPanel.hidden = true;
    el.chatMessages.replaceChildren();
    el.chatMessages.textContent = "正在载入会话…";
    const detail = await invoke("owo_history");
    el.chatMessages.replaceChildren();
    if (!replaySessionDetail(detail, true)) {
      el.chatMessages.innerHTML =
        '<div class="chat-empty">会话已切换，继续描述你的任务。</div>';
    }
  } catch (err) {
    showStatus("切换会话失败: " + err, "err");
  }
}

function toggleSessionsPanel() {
  const show = el.sessionsPanel.hidden;
  el.sessionsPanel.hidden = !show;
  if (show) refreshSessionsPanel();
}

// ---- 权限规则管理（B4-1）：查看 / 撤销审批卡记住的授权 ----

async function loadPermissionRules() {
  if (!invoke || !el.permRulesList) return;
  el.permRulesList.textContent = "加载中…";
  try {
    const data = await invoke("owo_permission_rules");
    const rules = Array.isArray(data && data.rules) ? data.rules : [];
    const sessionRules = Array.isArray(data && data.session_rules)
      ? data.session_rules
      : [];
    el.permRulesList.replaceChildren();
    if (!rules.length && !sessionRules.length) {
      el.permRulesList.textContent = "还没有记住的授权。审批卡里选择「本会话内允许」或「总是允许」后会出现在这里。";
      return;
    }
    for (const rule of sessionRules) {
      el.permRulesList.append(
        buildPermissionRuleRow(rule, true)
      );
    }
    for (const rule of rules) {
      el.permRulesList.append(buildPermissionRuleRow(rule, false));
    }
  } catch (err) {
    el.permRulesList.textContent =
      "加载失败（引擎未运行时不可见）：" + err;
  }
}

function buildPermissionRuleRow(rule, session) {
  const row = document.createElement("div");
  row.className = "perm-rule-row";
  const label = document.createElement("span");
  label.className = "perm-rule-label";
  const decision = rule.decision === "deny" ? "（拒绝）" : "";
  label.textContent = `${rule.tool} · ${rule.pattern}${decision}`;
  const scope = document.createElement("span");
  scope.className = "perm-rule-scope";
  scope.textContent = session ? "本会话" : "永久";
  const remove = document.createElement("button");
  remove.className = "mini-btn";
  remove.textContent = "撤销";
  remove.addEventListener("click", async () => {
    remove.disabled = true;
    try {
      await invoke("owo_remove_permission_rule", {
        tool: rule.tool,
        pattern: rule.pattern,
        session,
      });
      row.remove();
      if (!el.permRulesList.children.length) loadPermissionRules();
    } catch (err) {
      remove.disabled = false;
      showStatus("撤销失败: " + err, "err");
    }
  });
  row.append(label, scope, remove);
  return row;
}

// ---- 定时任务（A8-1）：到点自动跑只读任务/提醒，执行记录可查 ----

async function refreshAutomations() {
  if (!invoke || !el.autoList) return;
  el.autoList.textContent = "加载中…";
  try {
    const tasks = await invoke("owo_automations");
    const runs = await invoke("owo_automation_runs", { limit: 50 });
    renderAutomationList(
      Array.isArray(tasks) ? tasks : [],
      Array.isArray(runs) ? runs : []
    );
  } catch (err) {
    el.autoList.textContent = "加载失败（引擎未运行时不可用）：" + err;
  }
}

function automationScheduleLabel(schedule) {
  if (!schedule) return "";
  if (schedule.kind === "daily") return "每天 " + (schedule.time || "");
  if (schedule.kind === "interval") {
    const secs = Number(schedule.every_secs || 0);
    if (secs >= 3600 && secs % 3600 === 0) return `每 ${secs / 3600} 小时`;
    if (secs % 60 === 0) return `每 ${secs / 60} 分钟`;
    return `每 ${secs} 秒`;
  }
  if (schedule.kind === "one_shot") {
    return "一次 · " + String(schedule.at || "").replace("T", " ").slice(0, 16);
  }
  return String(schedule.kind || "");
}

function automationActionLabel(action) {
  if (!action) return "";
  if (action.kind === "run_prompt") return "跑任务";
  if (action.kind === "reminder") return "提醒";
  return String(action.kind || "");
}

function renderAutomationList(tasks, runs) {
  el.autoList.replaceChildren();
  if (!tasks.length) {
    el.autoList.textContent =
      "还没有定时任务。在下面添加：到点自动跑一个只读分析任务，或弹一条提醒。";
    return;
  }
  const runsByTask = new Map();
  for (const run of runs) {
    const list = runsByTask.get(run.task_id) || [];
    list.push(run);
    runsByTask.set(run.task_id, list);
  }
  for (const task of tasks) {
    el.autoList.append(buildAutomationRow(task, runsByTask.get(task.id) || []));
  }
}

function buildAutomationRow(task, runs) {
  const wrap = document.createElement("div");
  wrap.className = "auto-row";
  const head = document.createElement("div");
  head.className = "auto-row-head";
  const title = document.createElement("span");
  title.className = "auto-row-title";
  title.textContent = task.name;
  title.title = (task.action && (task.action.prompt || task.action.text)) || "";
  const meta = document.createElement("span");
  meta.className = "auto-row-meta";
  meta.textContent =
    automationScheduleLabel(task.schedule) +
    " · " +
    automationActionLabel(task.action) +
    (task.enabled === false ? " · 已停用" : "");
  const toggle = document.createElement("button");
  toggle.className = "mini-btn";
  toggle.type = "button";
  toggle.textContent = task.enabled === false ? "启用" : "停用";
  toggle.addEventListener("click", async () => {
    toggle.disabled = true;
    try {
      await invoke("owo_automation_toggle", { id: task.id });
      refreshAutomations();
    } catch (err) {
      toggle.disabled = false;
      showStatus("切换失败: " + err, "err");
    }
  });
  const remove = document.createElement("button");
  remove.className = "mini-btn";
  remove.type = "button";
  remove.textContent = "删除";
  remove.addEventListener("click", async () => {
    remove.disabled = true;
    try {
      await invoke("owo_automation_delete", { id: task.id });
      refreshAutomations();
    } catch (err) {
      remove.disabled = false;
      showStatus("删除失败: " + err, "err");
    }
  });
  head.append(title, meta, toggle, remove);
  wrap.append(head);

  const last = runs[0];
  const lastLine = document.createElement("div");
  lastLine.className = "auto-row-last";
  lastLine.textContent = last
    ? `最近执行：${String(last.at || "").replace("T", " ").slice(0, 16)} · ${
        last.status === "ok" ? "成功" : "失败"
      }`
    : "尚未执行";
  wrap.append(lastLine);

  if (runs.length) {
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = `执行记录（${runs.length}）`;
    const body = document.createElement("div");
    body.className = "auto-runs";
    for (const run of runs.slice(0, 10)) {
      const item = document.createElement("div");
      item.className = "auto-run-item";
      const itemHead = document.createElement("div");
      itemHead.className = "auto-run-head";
      itemHead.textContent = `${String(run.at || "").replace("T", " ").slice(0, 16)} · ${
        run.status === "ok" ? "成功" : "失败"
      }`;
      const output = document.createElement("pre");
      output.className = "auto-run-output";
      output.textContent = (run.output || "（无输出）").slice(0, 800);
      item.append(itemHead, output);
      body.append(item);
    }
    details.append(summary, body);
    wrap.append(details);
  }
  return wrap;
}

function syncAutomationScheduleFields() {
  const daily = el.autoSchedule.value === "daily";
  el.autoTime.hidden = !daily;
  el.autoEvery.hidden = daily;
  el.autoEveryUnit.hidden = daily;
}

async function createAutomationFromForm() {
  if (!invoke) return;
  const name = el.autoName.value.trim();
  const content = el.autoContent.value.trim();
  if (!name || !content) {
    showStatus("请填写任务名和内容", "err");
    return;
  }
  let schedule;
  if (el.autoSchedule.value === "daily") {
    schedule = { kind: "daily", time: el.autoTime.value || "09:00" };
  } else {
    const minutes = Math.min(1440, Math.max(1, Number(el.autoEvery.value) || 60));
    schedule = { kind: "interval", every_secs: minutes * 60 };
  }
  const action =
    el.autoKind.value === "reminder"
      ? { kind: "reminder", text: content }
      : { kind: "run_prompt", prompt: content };
  el.autoCreate.disabled = true;
  try {
    await invoke("owo_automation_create", { name, schedule, action });
    el.autoName.value = "";
    el.autoContent.value = "";
    showStatus("定时任务已添加", "ok");
    refreshAutomations();
  } catch (err) {
    showStatus("添加失败: " + err, "err");
  } finally {
    el.autoCreate.disabled = false;
  }
}

async function refreshOwoStatus(force = false) {
  if (!invoke) {
    el.owoStatusText.textContent = "浏览器预览：引擎不可用";
    el.owoStart.hidden = true;
    el.modeEngine.hidden = !owo.mode;
    return;
  }
  if (!force && Date.now() - owoStatusCache.at < 5000) {
    owoRenderStatus(
      owoStatusCache.reachable,
      owoStatusCache.version,
      owoStatusCache.canStart
    );
    return;
  }
  try {
    const status = await invoke("owo_status");
    owoStatusCache = {
      at: Date.now(),
      reachable: Boolean(status.reachable),
      version: status.version || "",
      canStart: Boolean(status.exe_path),
    };
    owoRenderStatus(
      owoStatusCache.reachable,
      owoStatusCache.version,
      owoStatusCache.canStart
    );
    if (owoStatusCache.reachable) loadOwoHistory();
    el.modeEngine.title = [
      `工作文件夹：${status.workspace}`,
      `引擎程序：${status.exe_path || "未找到 owo-agent.exe（可在设置页填写路径）"}`,
      status.service_managed ? "由灵犀托管运行" : "外部服务或未启动",
    ].join("\n");
  } catch (err) {
    owoRenderStatus(false, "", true);
  }
}

async function startOwoEngine() {
  if (!invoke) return;
  el.owoStart.disabled = true;
  el.owoStatusText.textContent = "正在启动引擎…";
  try {
    const status = await invoke("owo_start_service");
    owoStatusCache = {
      at: Date.now(),
      reachable: Boolean(status.reachable),
      version: status.version || "",
      canStart: Boolean(status.exe_path),
    };
    owoRenderStatus(
      owoStatusCache.reachable,
      owoStatusCache.version,
      owoStatusCache.canStart
    );
    if (status.error) showStatus("引擎启动失败: " + status.error, "err");
  } catch (err) {
    owoRenderStatus(false, "", true);
    showStatus("引擎启动失败: " + err, "err");
  } finally {
    el.owoStart.disabled = false;
  }
}

async function ensureOwoSession() {
  if (owo.sessionId) return owo.sessionId;
  owo.sessionId = await invoke("owo_ensure_session");
  return owo.sessionId;
}

function owoEnsureProgressBubble(label = "引擎正在执行任务…") {
  if (owo.progressBubble) {
    owo.progressBubble.textContent = label;
    return owo.progressBubble;
  }
  const bubble = document.createElement("div");
  bubble.className = "chat-bubble chat-assistant chat-thinking";
  bubble.textContent = label;
  el.chatMessages.appendChild(bubble);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
  owo.progressBubble = bubble;
  return bubble;
}

function owoDropProgressBubble() {
  if (owo.progressBubble) {
    owo.progressBubble.remove();
    owo.progressBubble = null;
  }
}

/// B2-4：计划卡（update_plan 工具驱动的任务清单）。同一回合复用一张卡，
/// 每次 plan_update 整表重绘。
function owoRenderPlan(parsed) {
  const steps = Array.isArray(parsed && parsed.steps) ? parsed.steps : [];
  if (!steps.length) return;
  if (!owo.planCard || !owo.planCard.isConnected) {
    owo.planCard = document.createElement("details");
    owo.planCard.className = "chat-plan-card";
    owo.planCard.open = true;
    owo.planCard.append(document.createElement("summary"));
    el.chatMessages.appendChild(owo.planCard);
  }
  owo.planCard.querySelector("summary").textContent = `任务计划（${steps.filter((s) => s.status === "completed").length}/${steps.length}）`;
  const body = document.createElement("div");
  body.className = "chat-plan-body";
  const ICONS = { pending: "○", in_progress: "◐", completed: "●" };
  for (const step of steps) {
    const row = document.createElement("div");
    row.className = "chat-plan-step chat-plan-" + (step.status || "pending");
    row.textContent = (ICONS[step.status] || "○") + " " + (step.content || "");
    body.append(row);
  }
  const old = owo.planCard.querySelector(".chat-plan-body");
  if (old) old.replaceWith(body);
  else owo.planCard.append(body);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
}

/// B2-4：深度思考折叠区（reasoning 增量）。同一回合复用一个 <details>，
/// 默认收起（不打扰），可展开查看推理过程。
function owoEnsureReasoningBox() {
  if (owo.reasoningBox && owo.reasoningBox.isConnected) return owo.reasoningBox;
  const box = document.createElement("details");
  box.className = "chat-reasoning";
  const summary = document.createElement("summary");
  summary.textContent = "思考过程";
  const body = document.createElement("div");
  body.className = "chat-reasoning-body";
  box.append(summary, body);
  el.chatMessages.appendChild(box);
  owo.reasoningBox = box;
  return box;
}

/// B2-4：回合统计（turn_stats）——一行小字汇报耗时/步数/消耗。
function owoAppendTurnStats(parsed) {
  if (!parsed) return;
  const row = document.createElement("div");
  row.className = "chat-turn-stats";
  const parts = [];
  if (parsed.duration_ms) parts.push((parsed.duration_ms / 1000).toFixed(1) + "s");
  if (parsed.steps) parts.push(parsed.steps + " 步");
  if (parsed.total_tokens) parts.push(parsed.total_tokens + " tokens");
  if (typeof parsed.cost_usd === "number" && parsed.cost_usd > 0)
    parts.push("$" + parsed.cost_usd.toFixed(4));
  if (!parts.length) return;
  row.textContent = "本回合：" + parts.join(" · ");
  el.chatMessages.appendChild(row);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
}

function owoFinishTurn() {
  owo.busy = false;
  el.chatSend.disabled = false;
  owoDropProgressBubble();
  owo.streamBubble = null;
  owo.planCard = null;
  owo.reasoningBox = null;
  owo.pendingTools.clear();
}

async function sendOwoTask(message, attachments) {
  try {
    const sessionId = await ensureOwoSession();
    owo.busy = true;
    el.chatSend.disabled = true;
    // M1：桌宠状态由 Rust 侧按 SSE 帧统一广播，前端不再自推。
    owoEnsureProgressBubble("已交给本地引擎，正在理解任务…");
    // B3 贴图入上下文：先上传到引擎附件目录，再把文件名随回合携带；
    // 引擎把图片附件转为 base64 data URL 进入模型视觉上下文。
    const names = [];
    for (const item of attachments || []) {
      names.push(
        await invoke("owo_upload_attachment", {
          sessionId,
          name: item.name,
          mime: item.mime,
          dataB64: item.dataB64,
        })
      );
    }
    await invoke("owo_send", { sessionId, message, attachments: names });
  } catch (err) {
    owoFinishTurn();
    appendChatBubble("error", "重任务发送失败：" + String(err));
    refreshOwoStatus();
  }
}

// ---- 贴图附件（B3：截图/图片 → 引擎多模态上下文）----

function owoAddAttachment(file) {
  if (!file || !String(file.type || "").startsWith("image/")) return;
  if (owo.pendingAttachments.length >= 4) {
    showStatus("一次最多附带 4 张图片", "err");
    return;
  }
  if (file.size > 4 * 1024 * 1024) {
    showStatus("图片过大（超过 4MB）：" + (file.name || "剪贴板图片"), "err");
    return;
  }
  const reader = new FileReader();
  reader.onload = () => {
    const match = String(reader.result || "").match(
      /^data:([^;]+);base64,(.+)$/
    );
    if (!match) return;
    owo.pendingAttachments.push({
      name: attachmentName(file.name || "paste.png"),
      mime: match[1],
      dataB64: match[2],
      dataUrl: String(reader.result),
    });
    renderAttachChips();
  };
  reader.readAsDataURL(file);
}

/// 附件名：服务端会再做 sanitize，这里生成稳定 ASCII 名避免编码问题。
function attachmentName(original) {
  const ext =
    (String(original).split(".").pop() || "png")
      .toLowerCase()
      .replace(/[^a-z0-9]/g, "") || "png";
  const pad = (n) => String(n).padStart(2, "0");
  const d = new Date();
  return (
    `img-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}` +
    `-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}` +
    `-${Math.random().toString(36).slice(2, 6)}.${ext}`
  );
}

function renderAttachChips() {
  el.attachBar.replaceChildren();
  el.attachBar.hidden = owo.pendingAttachments.length === 0;
  owo.pendingAttachments.forEach((item, index) => {
    const chip = document.createElement("div");
    chip.className = "chat-attach-chip";
    const thumb = document.createElement("img");
    thumb.src = item.dataUrl;
    thumb.alt = item.name;
    const name = document.createElement("span");
    name.textContent = item.name;
    name.title = item.name;
    const remove = document.createElement("button");
    remove.className = "chat-attach-remove";
    remove.textContent = "×";
    remove.title = "移除图片";
    remove.addEventListener("click", () => {
      owo.pendingAttachments.splice(index, 1);
      renderAttachChips();
    });
    chip.append(thumb, name, remove);
    el.attachBar.append(chip);
  });
}

/// 填充/更新工具卡内容：tool_use 时创建「执行中」，tool_result 时按 call_id
/// 更新同一张卡（B1-3：不再追加第二张卡）。
function owoToolCardContent(card, use, result) {
  const ok = result ? Boolean(result.ok) : false;
  card.className =
    "chat-tool-card " +
    (result ? (ok ? "tool-success" : "tool-failed") : "tool-running");
  card.querySelector("summary").textContent = `${use ? use.tool : "工具"} · ${
    result ? (ok ? "完成" : "失败") : "执行中"
  }`;
  if (result) {
    // B2-2：结果可见——展示引擎下发的 preview（已截断），失败显示错误。
    let line = card.querySelector(".chat-tool-result");
    if (!line) {
      line = document.createElement("pre");
      line.className = "chat-tool-result";
      card.querySelector(".chat-tool-body").append(line);
    }
    const preview = result.preview || "";
    line.textContent = result.error
      ? `错误：${result.error}${preview ? "\n" + preview : ""}`
      : preview || "已执行（无输出）";
  }
}

function owoAppendToolCard(use, result) {
  const card = document.createElement("details");
  const body = document.createElement("div");
  body.className = "chat-tool-body";
  const args = document.createElement("pre");
  args.textContent = JSON.stringify((use && use.args) || {}, null, 2);
  body.append(args);
  card.append(document.createElement("summary"), body);
  owoToolCardContent(card, use, result);
  el.chatMessages.appendChild(card);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
  return card;
}

/// 409 冲突提示卡（B1-4）：上一轮还在跑，给出「中止上一轮」行动入口，
/// 而不是误导性的「引擎连接中断」。
function appendBusyConflict() {
  const bubble = document.createElement("div");
  bubble.className = "chat-bubble chat-error";
  bubble.textContent =
    "上一轮任务还在执行中（同一会话一次只能跑一个回合），本次发送未开始。";
  const row = document.createElement("div");
  row.className = "chat-approval-actions";
  const abort = document.createElement("button");
  abort.className = "btn ghost";
  abort.textContent = "中止上一轮";
  abort.addEventListener("click", async () => {
    abort.disabled = true;
    try {
      if (owo.sessionId) await invoke("owo_abort", { sessionId: owo.sessionId });
      bubble.textContent = "已请求中止上一轮，稍候可重新发送任务。";
    } catch (err) {
      abort.disabled = false;
      showStatus("中止失败: " + err, "err");
    }
  });
  row.append(abort);
  bubble.append(row);
  el.chatMessages.appendChild(bubble);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
}

function owoAppendApproval(parsed) {
  const card = document.createElement("div");
  card.className = "chat-approval";
  // B1-5：按 request_id 挂索引，引擎下发 permission_resolved 时定位并关闭。
  card.dataset.requestId = (parsed && parsed.request_id) || "";
  if (parsed && parsed.request_id) owo.permissionCards.set(parsed.request_id, card);
  const head = document.createElement("div");
  head.className = "chat-approval-head";
  head.textContent = `需要你的批准：${parsed.tool || "危险操作"}`;
  const reason = document.createElement("div");
  reason.className = "chat-approval-reason";
  reason.textContent = parsed.reason || "该操作可能修改文件或执行命令";
  const args = document.createElement("pre");
  args.className = "chat-approval-args";
  args.textContent = JSON.stringify(parsed.args || {}, null, 2);
  const row = document.createElement("div");
  row.className = "chat-approval-actions";
  // A6-1 三档授权：每次都问 / 本会话内允许 / 总是允许（可在设置中查看与撤销）。
  const rememberScope = document.createElement("select");
  rememberScope.className = "approval-remember-scope";
  rememberScope.setAttribute("aria-label", "记住选择");
  for (const [value, label] of [
    ["", "每次都问"],
    ["session", "本会话内允许"],
    ["forever", "总是允许"],
  ]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    rememberScope.append(option);
  }
  const allow = document.createElement("button");
  allow.className = "btn primary";
  allow.textContent = "允许";
  const deny = document.createElement("button");
  deny.className = "btn ghost";
  deny.textContent = "拒绝";
  row.append(allow, deny, rememberScope);

  // M3：引擎 300s 内未收到审批即按拒绝处理——卡片显示倒计时，避免任务静默白跑。
  const countdown = document.createElement("div");
  countdown.className = "chat-approval-countdown";
  let remaining = 300;
  const renderCountdown = () => {
    // 引擎已下发终态（permission_resolved）时停止倒计时，避免覆盖终态文案。
    if (card.classList.contains("resolved")) return false;
    if (remaining <= 0) {
      countdown.textContent = "已超时：引擎将按拒绝处理";
      return false;
    }
    const mm = String(Math.floor(remaining / 60)).padStart(2, "0");
    const ss = String(remaining % 60).padStart(2, "0");
    countdown.textContent = `剩余 ${mm}:${ss}（超时按拒绝处理）`;
    remaining -= 1;
    return true;
  };
  renderCountdown();
  const countdownTimer = setInterval(() => {
    if (!renderCountdown()) clearInterval(countdownTimer);
  }, 1000);

  card.append(head, reason, args, row, countdown);

  const resolve = async (ok) => {
    clearInterval(countdownTimer);
    allow.disabled = true;
    deny.disabled = true;
    try {
      await invoke("owo_permission", {
        sessionId: owo.sessionId,
        requestId: parsed.request_id,
        allow: ok,
        remember: rememberScope.value ? true : null,
        rememberScope: rememberScope.value || null,
      });
      row.remove();
      const verdict = document.createElement("div");
      verdict.className = "chat-approval-reason";
      verdict.textContent = ok ? "已允许。" : "已拒绝；引擎会尝试更安全的替代路径。";
      card.append(verdict);
      card.classList.add("resolved");
      owo.permissionCards.delete(parsed.request_id);
    } catch (err) {
      allow.disabled = false;
      deny.disabled = false;
      showStatus("审批回传失败: " + err, "err");
    }
  };
  allow.addEventListener("click", () => resolve(true));
  deny.addEventListener("click", () => resolve(false));

  el.chatMessages.appendChild(card);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
}

/// B1-5：审批终态（引擎侧已解决：用户提交 / 300s 超时 / 回合中止）。
/// 与提问的 `owoResolveQuestion` 对称——此前超时后卡片仍留在界面上可点，
/// 用户点了只会得到「回传失败」。本地点击路径先写 resolved，重复到达时直接跳过。
function owoResolvePermission(parsed) {
  const requestId = (parsed && parsed.request_id) || "";
  const card =
    owo.permissionCards.get(requestId) ||
    document.querySelector(
      '.chat-approval[data-request-id="' + requestId + '"]'
    );
  if (!card || card.classList.contains("resolved")) return;
  owo.permissionCards.delete(requestId);
  card.classList.add("resolved");
  const actions = card.querySelector(".chat-approval-actions");
  if (actions) actions.remove();
  const source = (parsed && parsed.source) || "user";
  const allowed = Boolean(parsed && parsed.allowed);
  const verdict =
    source === "timeout"
      ? "已超时：引擎按拒绝处理，本轮跳过该操作。"
      : source === "aborted"
        ? "回合已中止，该审批作废。"
        : allowed
          ? "已允许。"
          : "已拒绝。";
  const countdown = card.querySelector(".chat-approval-countdown");
  if (countdown) countdown.textContent = verdict;
  else {
    const stamp = document.createElement("div");
    stamp.className = "chat-approval-reason";
    stamp.textContent = verdict;
    card.append(stamp);
  }
}

/// ask_user 提问卡：引擎挂起等待用户回答（与审批卡同一视觉通道）。
function owoAppendQuestion(parsed) {
  const card = document.createElement("div");
  card.className = "chat-approval";
  card.dataset.questionId = (parsed && parsed.question_id) || "";
  const head = document.createElement("div");
  head.className = "chat-approval-head";
  head.textContent = "引擎需要你确认";
  const reason = document.createElement("div");
  reason.className = "chat-approval-reason";
  reason.textContent = (parsed && parsed.question) || "";
  card.append(head, reason);

  const options = Array.isArray(parsed && parsed.options) ? parsed.options : [];
  const actions = document.createElement("div");
  actions.className = "chat-approval-actions";
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = options.length ? "或输入其他回答…" : "输入你的回答后回车…";
  input.style.cssText =
    "flex:1;min-width:0;padding:4px 8px;border-radius:6px;border:1px solid rgba(255,255,255,0.2);background:transparent;color:inherit;";
  const send = document.createElement("button");
  send.className = "btn primary";
  send.textContent = "回答";

  const resolve = async (value) => {
    const answer = String(value || "").trim();
    if (!answer || card.dataset.answered === "1") return;
    card.dataset.answered = "1";
    send.disabled = true;
    input.disabled = true;
    try {
      await invoke("owo_answer", {
        sessionId: owo.sessionId,
        questionId: card.dataset.questionId,
        answer,
      });
      const verdict = document.createElement("div");
      verdict.className = "chat-approval-reason";
      verdict.textContent = "已回答：" + answer;
      card.append(verdict);
      card.classList.add("resolved");
    } catch (err) {
      card.dataset.answered = "";
      send.disabled = false;
      input.disabled = false;
      showStatus("回答回传失败: " + err, "err");
    }
  };
  actions.append(input, send);
  for (const option of options.slice(0, 4)) {
    const button = document.createElement("button");
    button.className = "btn ghost";
    button.textContent = option;
    button.addEventListener("click", () => resolve(option));
    actions.insertBefore(button, input);
  }
  send.addEventListener("click", () => resolve(input.value));
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") resolve(input.value);
  });
  card.append(actions);
  el.chatMessages.appendChild(card);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
}

/// 提问终态（用户已答/超时/中止）：补状态说明并恢复桌宠状态。
function owoResolveQuestion(parsed) {
  const questionId = (parsed && parsed.question_id) || "";
  const card = document.querySelector('.chat-approval[data-question-id="' + questionId + '"]');
  if (!card) {
    return;
  }
  card.classList.add("resolved");
  if (!card.dataset.answered) {
    const stamp = document.createElement("div");
    stamp.className = "chat-approval-reason";
    const source = (parsed && parsed.source) || "user";
    if (source === "user") stamp.textContent = "已回答。";
    else if (source === "timeout") stamp.textContent = "提问超时未答，引擎已按已有信息继续。";
    else stamp.textContent = "回合已中止，该提问作废。";
    card.append(stamp);
  }
}

/// B2-3：行级对比（LCS）。返回 [{type: "same"|"add"|"del", text}]。
/// 行数超限时退化为「整删整增」，保证 UI 不卡。
function lineDiffRows(beforeText, afterText) {
  const MAX_LINES = 300;
  const before = String(beforeText ?? "").split("\n");
  const after = String(afterText ?? "").split("\n");
  if (before.length > MAX_LINES || after.length > MAX_LINES) {
    return [
      ...before.map((text) => ({ type: "del", text })),
      ...after.map((text) => ({ type: "add", text })),
    ];
  }
  // LCS DP 表。
  const rows = before.length;
  const cols = after.length;
  const table = Array.from({ length: rows + 1 }, () => new Uint16Array(cols + 1));
  for (let i = rows - 1; i >= 0; i -= 1) {
    for (let j = cols - 1; j >= 0; j -= 1) {
      table[i][j] =
        before[i] === after[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }
  const out = [];
  let i = 0;
  let j = 0;
  while (i < rows && j < cols) {
    if (before[i] === after[j]) {
      out.push({ type: "same", text: before[i] });
      i += 1;
      j += 1;
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      out.push({ type: "del", text: before[i] });
      i += 1;
    } else {
      out.push({ type: "add", text: after[j] });
      j += 1;
    }
  }
  while (i < rows) out.push({ type: "del", text: before[i++] });
  while (j < cols) out.push({ type: "add", text: after[j++] });
  return out;
}

async function renderOwoDiff(sessionId) {
  if (!invoke || !sessionId) return;
  let diffs = [];
  try {
    diffs = await invoke("owo_diff", { sessionId });
  } catch (err) {
    return;
  }
  if (!diffs || !diffs.length) return;
  const card = document.createElement("details");
  card.className = "chat-diff-card";
  const summary = document.createElement("summary");
  summary.textContent = `本回合改动 ${diffs.length} 个文件（展开查看，可回滚）`;
  const body = document.createElement("div");
  body.className = "chat-diff-body";
  for (const diff of diffs) {
    const item = document.createElement("div");
    const path = document.createElement("div");
    path.className = "chat-diff-path";
    path.textContent = diff.path + (diff.before ? "" : "（新文件）");
    const preview = document.createElement("pre");
    preview.className = "chat-diff-preview chat-diff-lines";
    // B2-3：行级 +/- 着色；仅渲染改动行上下文（same 行压缩显示）。
    const rows = lineDiffRows(diff.before || "", diff.after || "");
    let sameRun = 0;
    const flushSame = () => {
      if (sameRun > 0) {
        const skip = document.createElement("span");
        skip.className = "diff-line diff-same-skip";
        skip.textContent = `  …（${sameRun} 行未改动）`;
        preview.append(skip);
        sameRun = 0;
      }
    };
    for (const row of rows) {
      if (row.type === "same") {
        // 连续未改动行超过 2 行时折叠（只保留可读上下文）。
        sameRun += 1;
        continue;
      }
      flushSame();
      const line = document.createElement("span");
      line.className = "diff-line diff-" + (row.type === "add" ? "add" : "del");
      line.textContent = (row.type === "add" ? "+ " : "- ") + row.text;
      preview.append(line);
    }
    flushSame();
    if (!preview.childNodes.length) {
      const same = document.createElement("span");
      same.className = "diff-line diff-same-skip";
      same.textContent = "（内容无变化）";
      preview.append(same);
    }
    item.append(path, preview);
    body.append(item);
  }
  const revert = document.createElement("button");
  revert.className = "btn ghost";
  revert.textContent = "全部回滚";
  revert.addEventListener("click", async () => {
    revert.disabled = true;
    try {
      await invoke("owo_revert", { sessionId });
      card.remove();
      appendChatBubble("assistant", "已回滚本回合的文件改动。");
    } catch (err) {
      revert.disabled = false;
      showStatus("回滚失败: " + err, "err");
    }
  });
  body.append(revert);
  card.append(summary, body);
  el.chatMessages.appendChild(card);
  el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
}

function owoHandleFrame(payload) {
  let parsed = null;
  try {
    parsed = JSON.parse(payload.data);
  } catch (err) {
    // 非 JSON 帧（如纯文本错误）按错误提示。
    parsed = null;
  }
  const type = (parsed && parsed.type) || payload.event || "unknown";
  switch (type) {
    case "progress": {
      owoEnsureProgressBubble((parsed && parsed.message) || "引擎正在执行…");
      break;
    }
    case "token_delta": {
      owoDropProgressBubble();
      if (!owo.streamBubble) {
        owo.streamBubble = document.createElement("div");
        owo.streamBubble.className = "chat-bubble chat-assistant";
        el.chatMessages.appendChild(owo.streamBubble);
      }
      owo.streamBubble.textContent += (parsed && parsed.delta) || "";
      el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
      break;
    }
    case "tool_use": {
      const card = owoAppendToolCard(parsed, null);
      owo.pendingTools.set(parsed.id, { use: parsed, card });
      break;
    }
    case "tool_result": {
      const pending = owo.pendingTools.get(parsed.id);
      owo.pendingTools.delete(parsed.id);
      if (pending && pending.card && pending.card.isConnected) {
        owoToolCardContent(pending.card, pending.use, parsed);
      } else {
        // 历史帧乱序/卡片已被清理：退回独立卡片，保证结果不丢。
        owoAppendToolCard(pending ? pending.use : null, parsed);
      }
      break;
    }
    case "permission_request": {
      owoAppendApproval(parsed);
      break;
    }
    case "permission_resolved": {
      // B1-5：引擎侧终态（超时/中止/其他通道响应）→ 关闭卡片行动区。
      owoResolvePermission(parsed);
      break;
    }
    case "user_question": {
      owoAppendQuestion(parsed);
      break;
    }
    case "user_answered": {
      owoResolveQuestion(parsed);
      break;
    }
    case "compaction": {
      const summary = (parsed && parsed.summary) || "";
      appendChatBubble(
        "assistant",
        summary
          ? "（会话过长，历史已压缩：" + summary.slice(0, 120) + "…）"
          : "（会话过长，历史已自动压缩为摘要）"
      );
      break;
    }
    case "plan_update": {
      // B2-4：update_plan 工具的任务清单进度。
      owoRenderPlan(parsed);
      break;
    }
    case "reasoning_delta": {
      // B2-4：深度思考增量 → 折叠区流式追加（默认收起）。
      const body = owoEnsureReasoningBox().querySelector(".chat-reasoning-body");
      body.textContent += (parsed && parsed.delta) || "";
      el.chatMessages.scrollTop = el.chatMessages.scrollHeight;
      break;
    }
    case "turn_stats": {
      owoAppendTurnStats(parsed);
      break;
    }
    case "final": {
      // B1-2：token_delta 流式气泡已承载回复正文，final 到达时「定型」而非追加，
      // 避免同一段内容显示两遍；定型时用 markdown 渲染完整内容（B2-1）。
      if (owo.streamBubble) {
        const bubble = owo.streamBubble;
        owoFinishTurn();
        if (parsed && parsed.text) {
          if (window.renderMarkdownInto) {
            bubble.replaceChildren();
            const body = document.createElement("div");
            body.className = "chat-md";
            window.renderMarkdownInto(body, parsed.text);
            bubble.append(body);
          } else {
            bubble.textContent = parsed.text;
          }
        }
      } else {
        owoFinishTurn();
        if (parsed && parsed.text) appendChatBubble("assistant", parsed.text);
      }
      // 状态由 Rust 侧广播（final → speaking，流结束 → idle）。
      renderOwoDiff(payload.session_id);
      break;
    }
    case "turn_failed": {
      // B1-1：引擎失败终态（模型 500/中断等）。此前落入 default 被静默忽略，
      // busy 永真 → 发送按钮永久禁用、UI 卡死。
      owoFinishTurn();
      appendChatBubble(
        "error",
        "回合失败：" + ((parsed && parsed.message) || "未知原因")
      );
      break;
    }
    case "bridge_error": {
      owoFinishTurn();
      // B1-4：按 Rust 侧分类渲染，409 是「上一轮还在跑」而非断连。
      if (payload.kind === "busy_409") {
        appendBusyConflict();
      } else if (payload.kind === "unreachable") {
        appendChatBubble("error", "引擎不可达：请先启动本地引擎（设置 → 重任务引擎）。");
        refreshOwoStatus();
      } else if (payload.kind === "unauthorized") {
        appendChatBubble("error", "引擎配对失效：请重启引擎后重试（设置 → 重任务引擎）。");
        refreshOwoStatus();
      } else {
        appendChatBubble("error", "引擎连接中断：" + payload.data);
        refreshOwoStatus();
      }
      break;
    }
    default: {
      // 未知事件类型：静默忽略，保证向前兼容。
      break;
    }
  }
}

// ---- Tools management ----

// Browser-preview widgets so the cards are visible without a Tauri backend.
// Rendering resolves icons via WIDGET_ICONS[id]; no icon field needed here.
const MOCK_WIDGETS = [
  { id: "widget-ocr", label: "屏幕识别", shortcut: "Ctrl+Alt+O", description: "框选屏幕区域，OCR 提取文字" },
  { id: "widget-translate", label: "全屏翻译", shortcut: "Ctrl+Alt+T", description: "框选区域识别并翻译" },
  { id: "widget-colorpicker", label: "取色器", shortcut: "Ctrl+Alt+C", description: "屏幕取色，HEX/RGB/HSL" },
  { id: "widget-weather", label: "天气", shortcut: "", description: "当前天气与 3 日预报" },
  { id: "widget-calculator", label: "计算器", shortcut: "Ctrl+Alt+=", description: "输入即算，支持单位换算" },
  { id: "widget-clipboard", label: "剪贴板历史", shortcut: "Ctrl+Alt+V", description: "最近剪贴板记录" },
];

let widgetsCache = [];
let widgetsOpenIds = new Set();

async function loadWidgets() {
  if (!invoke) {
    renderWidgets(MOCK_WIDGETS);
    return;
  }
  try {
    const [widgets, openIds] = await Promise.all([
      invoke("list_widgets"),
      invoke("list_open_widgets").catch(() => []),
    ]);
    widgetsOpenIds = new Set(openIds);
    renderWidgets(widgets);
  } catch (err) {
    el.widgetsGrid.replaceChildren();
    el.widgetsGrid.textContent = "小工具加载失败: " + err;
  }
}

function renderWidgets(widgets) {
  widgetsCache = widgets;
  el.widgetsGrid.replaceChildren();
  for (const w of widgets) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "widget-card" + (widgetsOpenIds.has(w.id) ? " is-open" : "");
    card.dataset.widgetId = w.id;
    card.setAttribute("aria-label", "打开 " + w.label);

    const icon = document.createElement("span");
    icon.className = "widget-card-icon";
    icon.innerHTML = WIDGET_ICONS[w.id] || TOOL_ICONS._default;

    const body = document.createElement("div");
    body.className = "widget-card-body";
    const name = document.createElement("span");
    name.className = "widget-card-name";
    name.textContent = w.label;
    const desc = document.createElement("span");
    desc.className = "widget-card-desc";
    desc.textContent = w.description || "";
    body.append(name, desc);

    card.append(icon, body);

    if (w.shortcut) {
      const shortcut = document.createElement("span");
      shortcut.className = "widget-card-shortcut";
      shortcut.textContent = w.shortcut;
      card.append(shortcut);
    }

    el.widgetsGrid.appendChild(card);
  }
}

async function openWidgetById(id) {
  if (!invoke) {
    showStatus("小工具需要 Tauri 运行环境", "warn");
    return;
  }
  try {
    await invoke("open_widget", { id });
    widgetsOpenIds.add(id);
    // Mark the card as open without a full reload.
    const card = el.widgetsGrid.querySelector('[data-widget-id="' + CSS.escape(id) + '"]');
    if (card) card.classList.add("is-open");
    showStatus("已打开 " + (widgetsCache.find((w) => w.id === id) || {}).label, "ok");
  } catch (err) {
    showStatus("打开小工具失败: " + err, "err");
  }
}

async function loadTools() {
  if (!invoke) {
    // Browser preview: render sample tools so the card grid is visible.
    renderTools(MOCK_TOOLS);
    return;
  }
  try {
    const tools = await invoke("list_tools");
    renderTools(tools);
  } catch (err) {
    toolsCache = [];
    el.toolsGrid.replaceChildren();
    el.toolsEmpty.textContent = "加载失败: " + err;
    el.toolsEmpty.hidden = false;
  }
}

function renderTools(tools) {
  toolsCache = tools;
  el.toolsCount.hidden = tools.length === 0;
  applyToolsFilter();
}

function applyToolsFilter() {
  const q = toolsSearchQuery.trim().toLowerCase();
  const cat = toolsActiveCat;
  const filtered = toolsCache.filter((t) => {
    if (cat !== "all" && (TOOL_CATEGORIES[t.name] || "capability") !== cat) return false;
    if (!q) return true;
    return t.name.toLowerCase().includes(q) || (t.description || "").toLowerCase().includes(q);
  });

  el.toolsGrid.replaceChildren();
  el.toolsCount.textContent = filtered.length;
  if (!filtered.length) {
    el.toolsEmpty.textContent = toolsCache.length ? "未找到匹配的工具" : "暂无已注册的工具";
    el.toolsEmpty.hidden = false;
    return;
  }
  el.toolsEmpty.hidden = true;

  const riskLabels = { safe: "安全", moderate: "中等", dangerous: "危险" };
  for (const t of filtered) {
    const card = document.createElement("div");
    card.className = "tool-card" + (t.enabled ? "" : " disabled");
    card.dataset.tool = t.name;

    const head = document.createElement("div");
    head.className = "tool-card-head";
    const icon = document.createElement("span");
    icon.className = "tool-card-icon";
    icon.innerHTML = TOOL_ICONS[t.name] || TOOL_ICONS._default;
    const name = document.createElement("span");
    name.className = "tool-card-name";
    name.textContent = t.name;
    name.title = t.description || "";
    head.append(icon, name);
    if (t.source === "plugin") {
      const tag = document.createElement("span");
      tag.className = "tool-card-plugin";
      tag.textContent = "插件";
      head.append(tag);
    }

    const shortcut = TOOL_SHORTCUTS[t.name];
    let shortcutEl = null;
    if (shortcut) {
      shortcutEl = document.createElement("div");
      shortcutEl.className = "tool-card-shortcut";
      shortcutEl.textContent = shortcut;
    }

    const footer = document.createElement("div");
    footer.className = "tool-card-footer";
    const badge = document.createElement("span");
    badge.className = "tool-badge " + (t.risk_level || "safe");
    badge.textContent = riskLabels[t.risk_level] || t.risk_level || "安全";
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "tool-toggle" + (t.enabled ? "" : " off");
    toggle.dataset.tool = t.name;
    toggle.dataset.enabled = t.enabled ? "1" : "0";
    toggle.setAttribute("aria-label", t.enabled ? "禁用" : "启用");
    if (t.risk_level === "dangerous") toggle.disabled = true;
    footer.append(badge, toggle);

    card.append(head);
    if (shortcutEl) card.append(shortcutEl);
    card.append(footer);
    el.toolsGrid.appendChild(card);
  }
}

// 改写 / QQ 草稿入口已移除（与工作台重复，A8-2 桌宠精简一并清理）；
// 后端能力保留，如需恢复只需恢复 index.html 的快捷卡片。
if (el.quickRewrite) el.quickRewrite.addEventListener("click", () => showView("rewrite"));
if (el.quickQq) {
  el.quickQq.addEventListener("click", () => {
    showView("qq");
    readQqMessage();
  });
}
el.agentTab.addEventListener("click", () => {
  showView("agent");
  refreshOwoStatus();
});
el.toolsTab.addEventListener("click", () => showView("tools"));
el.toolsSearch.addEventListener("input", () => {
  toolsSearchQuery = el.toolsSearch.value;
  applyToolsFilter();
});
el.toolsTabs.addEventListener("click", (e) => {
  const tab = e.target.closest(".tools-tab");
  if (!tab) return;
  for (const t of el.toolsTabs.querySelectorAll(".tools-tab")) t.classList.remove("is-active");
  tab.classList.add("is-active");
  toolsActiveCat = tab.dataset.cat;
  applyToolsFilter();
});
el.widgetsGrid.addEventListener("click", (e) => {
  const card = e.target.closest(".widget-card");
  if (!card) return;
  openWidgetById(card.dataset.widgetId);
});
el.toolsGrid.addEventListener("click", async (e) => {
  const toggle = e.target.closest(".tool-toggle");
  if (!toggle || toggle.disabled) return;
  const name = toggle.dataset.tool;
  const newEnabled = toggle.dataset.enabled !== "1";
  if (invoke) {
    try {
      await invoke("toggle_tool", { name, enabled: newEnabled });
    } catch (err) {
      showStatus("切换失败: " + err, "err");
      return;
    }
  }
  toggle.dataset.enabled = newEnabled ? "1" : "0";
  toggle.classList.toggle("off", !newEnabled);
  toggle.setAttribute("aria-label", newEnabled ? "禁用" : "启用");
  const card = toggle.closest(".tool-card");
  if (card) card.classList.toggle("disabled", !newEnabled);
  const cached = toolsCache.find((t) => t.name === name);
  if (cached) cached.enabled = newEnabled;
});
el.settingsBtn.addEventListener("click", () => { showView("settings"); loadSettings(); loadPetSettings(); loadWindowOptions(); loadEngineOptions(); });
el.providerPreset.addEventListener("change", applyProviderPreset);
el.backendSelect.addEventListener("change", syncCloudFields);
el.saveSettings.addEventListener("click", saveSettings);
if (el.savePetOptions) el.savePetOptions.addEventListener("click", savePetOptions);
el.saveWindowOptions.addEventListener("click", saveWindowOptions);
el.saveEngineOptions.addEventListener("click", saveEngineSettings);
el.enginePickFolder.addEventListener("click", pickEngineFolder);
el.enginePickExe.addEventListener("click", pickEngineExe);
el.engineRestart.addEventListener("click", restartEngine);
// B7-3：停止托管引擎（此前 owo_stop_service 注册但无入口）。
el.engineStop.addEventListener("click", async () => {
  if (!invoke) return;
  el.engineStop.disabled = true;
  try {
    await invoke("owo_stop_service");
    el.engineStatusInline.textContent = "已停止托管引擎。再次点「重启引擎」可拉起。";
    refreshOwoStatus(true);
  } catch (err) {
    showStatus("停止引擎失败: " + err, "err");
  } finally {
    el.engineStop.disabled = false;
  }
});
// B5-2：保存主面板呼出热键（Win32 组合语法校验在 Rust 侧）。
if (el.savePanelHotkey) {
  el.savePanelHotkey.addEventListener("click", async () => {
    if (!invoke) return;
    el.savePanelHotkey.disabled = true;
    try {
      const saved = await invoke("set_panel_hotkey", {
        hotkey: el.panelHotkey.value.trim(),
      });
      el.panelHotkey.value = saved;
      showStatus(`主面板热键已保存：${saved}（重启灵犀后完全生效）`, "ok");
    } catch (err) {
      showStatus("保存热键失败: " + err, "err");
    } finally {
      el.savePanelHotkey.disabled = false;
    }
  });
}
// B4-4：剪贴板监听隐私开关（即时生效）。
if (el.clipboardWatch) {
  el.clipboardWatch.addEventListener("change", async () => {
    if (!invoke) return;
    try {
      const enabled = await invoke("set_clipboard_listener", {
        enabled: el.clipboardWatch.checked,
      });
      showStatus(
        enabled ? "剪贴板历史已开启" : "剪贴板监听已关闭（不再读取剪贴板内容）",
        "ok"
      );
    } catch (err) {
      el.clipboardWatch.checked = !el.clipboardWatch.checked;
      showStatus("切换剪贴板监听失败: " + err, "err");
    }
  });
}
// B4-3：首启引导按钮。
if (el.onboarding) {
  el.onboardingStartEngine.addEventListener("click", async () => {
    el.onboardingStartEngine.disabled = true;
    el.onboardingStartEngine.textContent = "启动中…";
    await startOwoEngine();
    el.onboardingEngineState.textContent = "已请求启动";
    el.onboardingStartEngine.textContent = "启动引擎";
    finishOnboarding();
  });
  el.onboardingSettings.addEventListener("click", () => {
    if (el.onboarding) el.onboarding.hidden = true;
    showView("settings");
    loadSettings();
  });
  el.onboardingDone.addEventListener("click", finishOnboarding);
}
// 皮肤也可能从桌宠右键菜单切换：设置页打开期间同步高亮状态。
if (TAURI && TAURI.event && TAURI.event.listen) {
  TAURI.event.listen("pet-config-changed", (event) => {
    if (event.payload && event.payload.skin && petSkinsCache.length) {
      renderSkinGrid(petSkinsCache, event.payload.skin.id);
    }
  }).catch(() => {});
}
el.qqRefresh.addEventListener("click", readQqMessage);
el.recaptureBtn.addEventListener("click", recaptureSelection);
el.qqGenerate.addEventListener("click", generateQqDraft);
el.qqWrite.addEventListener("click", writeQqDraft);
el.chatSend.addEventListener("click", sendChatMessage);
el.agentReset.addEventListener("click", resetAgentChat);
// B3-1：历史会话面板。
el.agentSessionsBtn.addEventListener("click", toggleSessionsPanel);
el.sessionsRefresh.addEventListener("click", refreshSessionsPanel);
// B3：贴图附件——截图后 Ctrl+V 直接入列，或点按钮选文件。
el.chatAttach.addEventListener("click", () => el.chatFilePick.click());
el.chatFilePick.addEventListener("change", () => {
  for (const file of el.chatFilePick.files || []) owoAddAttachment(file);
  el.chatFilePick.value = "";
});
el.chatInput.addEventListener("paste", (event) => {
  const files = Array.from(event.clipboardData?.files || []).filter((file) =>
    String(file.type || "").startsWith("image/")
  );
  if (!files.length) return;
  event.preventDefault(); // 阻止把图片当文本/路径粘贴进输入框
  for (const file of files) owoAddAttachment(file);
});
// B4-1：权限规则管理。
el.permRulesRefresh.addEventListener("click", loadPermissionRules);
// A8-1：定时任务。
if (el.autoRefresh) {
  el.autoRefresh.addEventListener("click", refreshAutomations);
  el.autoCreate.addEventListener("click", createAutomationFromForm);
  el.autoSchedule.addEventListener("change", syncAutomationScheduleFields);
  syncAutomationScheduleFields();
}
// 重任务引擎：启动按钮 + 模式开关 + SSE 事件回流。
el.owoStart.addEventListener("click", startOwoEngine);
el.modeChat.addEventListener("click", () => setOwoMode(false));
el.modeTask.addEventListener("click", () => setOwoMode(true));
// 输入区：+ 把剪贴板内容加进输入框；打字聚焦；语音识别。
el.composerPlus.addEventListener("click", async () => {
  if (!invoke) return;
  try {
    const text = await invoke("widget_read_clipboard");
    const trimmed = String(text || "").trim();
    if (!trimmed) {
      showStatus("剪贴板里没有可用文本", "warn");
      return;
    }
    const current = el.chatInput.value.trimEnd();
    el.chatInput.value = current ? `${current}\n${trimmed}` : trimmed;
    el.chatInput.focus();
  } catch (err) {
    showStatus("读取剪贴板失败: " + err, "err");
  }
});
el.toolType.addEventListener("click", () => el.chatInput.focus());
el.toolVoice.addEventListener("click", toggleVoiceInput);
// 标题栏：显示 / 隐藏桌宠（与设置页的可见性开关共用同一持久化字段）。
el.petToggleBtn.addEventListener("click", async () => {
  if (!invoke) return;
  const next = !petVisibleCache;
  try {
    await invoke("set_pet_visible", { visible: next });
    petVisibleCache = next;
    el.petToggleBtn.classList.toggle("is-off", !next);
    el.petToggleBtn.title = next ? "隐藏桌宠" : "显示桌宠";
    showStatus(next ? "桌宠已显示" : "已隐藏桌宠（点同一按钮可恢复）", "ok");
  } catch (err) {
    showStatus("切换桌宠失败: " + err, "err");
  }
});
if (TAURI && TAURI.event && TAURI.event.listen) {
  TAURI.event.listen("owo://turn", (event) => {
    if (event.payload) owoHandleFrame(event.payload);
  }).catch(() => {});
}
// 初始视图：对话（改写 / QQ 草稿收进工具页；热键抓选区时自动切到改写）。
// 预览调试：?view=tools|settings|rewrite|qq 可直接打开对应视图。
const initialView =
  new URLSearchParams(window.location.search).get("view") || "agent";
showView(initialView);
renderChatSuggestions();
refreshOwoStatus();
(async () => {
  if (!invoke) return;
  try {
    const config = await invoke("current_pet_config");
    petVisibleCache = Boolean(config && config.visible);
    el.petToggleBtn.classList.toggle("is-off", !petVisibleCache);
    el.petToggleBtn.title = petVisibleCache ? "隐藏桌宠" : "显示桌宠";
  } catch {
    /* 后端未就绪时保持默认（显示） */
  }
})();
el.chatInput.addEventListener("keydown", (e) => {
  if (e.ctrlKey && e.key === "Enter") {
    // preventDefault + stopPropagation so the document-level Ctrl+Enter
    // handler (which calls apply()) does not also fire — that would show
    // misleading "no captured selection" errors in the chat view.
    e.preventDefault();
    e.stopPropagation();
    sendChatMessage();
  }
});

// Native drag: begin a Win32 window move on the titlebar. `data-tauri-drag-region`
// is unreliable for a non-activating window, so we drive it explicitly. Ignore
// presses that land on the titlebar's own buttons.
el.titlebar.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || e.target.closest("button")) return;
  if (invoke) {
    e.preventDefault();
    invoke("start_window_drag").catch(() => {});
  }
});

el.resizeGrip.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || !invoke) return;
  e.preventDefault();
  e.stopPropagation();
  invoke("start_window_resize").catch((error) => showStatus("无法调整窗口大小: " + error, "err"));
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    // B1-6：输入中 Esc 只清空/失焦输入框，不关整个面板（防止取消 IME 组词时误关）。
    const active = document.activeElement;
    const typing =
      active &&
      (active.tagName === "INPUT" ||
        active.tagName === "TEXTAREA" ||
        active.isContentEditable);
    if (typing) {
      if (active.value) active.value = "";
      else active.blur();
      return;
    }
    close();
  } else if (e.ctrlKey && e.key === "Enter") apply();
});

// Lightweight in-window modal confirm. The browser's native `confirm()` opens
// a system dialog that often renders outside the small overlay window (the
// "取消" button ends up off-screen). This keeps everything inside the panel.
function showConfirmDialog(title, message) {
  return new Promise((resolve) => {
    // Block re-entry: if a dialog is already open, treat as cancel.
    const existing = document.querySelector(".confirm-overlay");
    if (existing) {
      resolve(false);
      return;
    }
    const overlay = document.createElement("div");
    overlay.className = "confirm-overlay";
    overlay.innerHTML = `
      <div class="confirm-card">
        <div class="confirm-title">${title}</div>
        <div class="confirm-message">${message}</div>
        <div class="confirm-actions">
          <button type="button" class="confirm-cancel">取消</button>
          <button type="button" class="confirm-ok">确定</button>
        </div>
      </div>
    `;
    document.body.appendChild(overlay);
    const cleanup = (result) => {
      overlay.remove();
      resolve(result);
    };
    overlay.querySelector(".confirm-cancel").addEventListener("click", () => cleanup(false));
    overlay.querySelector(".confirm-ok").addEventListener("click", () => cleanup(true));
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) cleanup(false);
    });
    const keyHandler = (e) => {
      if (e.key === "Escape") {
        // stopPropagation prevents the document-level Escape handler from
        // closing the entire overlay panel when the user only meant to
        // dismiss this dialog.
        e.stopPropagation();
        document.removeEventListener("keydown", keyHandler, true);
        cleanup(false);
      } else if (e.key === "Enter") {
        e.stopPropagation();
        document.removeEventListener("keydown", keyHandler, true);
        cleanup(true);
      }
    };
    document.addEventListener("keydown", keyHandler, true);
    // Focus the cancel button so Enter does not accidentally confirm.
    setTimeout(() => overlay.querySelector(".confirm-cancel").focus(), 10);
  });
}

// Adapt the synchronous call sites that still use a boolean return.
window.showConfirmDialog = showConfirmDialog;

// Kick off: poll in Tauri, or render the mock once in a browser.
if (invoke) {
  loadSettings();
  pollSelection();
  setInterval(pollSelection, 350);
  // B4-3：首启引导（首次运行才弹；完成标记写在设置里）。
  setTimeout(initOnboarding, 600);
  // B5-2：热键注册冲突的可见提示（注册发生在启动早期，事件可能早于本监听，
  // 因此同时提供一个轮询兜底不需要——showStatus 会随设置页打开再次可见）。
  if (TAURI && TAURI.event && TAURI.event.listen) {
    TAURI.event.listen("lingxi://hotkey-conflict", (event) => {
      if (event.payload) showStatus(String(event.payload), "err");
    }).catch(() => {});
  }
} else {
  refreshPreview();
}
