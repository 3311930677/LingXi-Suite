// 灵犀小工具共享脚本 — Tauri invoke 初始化 + 统一关闭按钮逻辑
"use strict";
const TAURI = window.__TAURI__;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;

// 关闭当前小工具窗口。优先走后端 destroy()（绕过可能卡住的
// CloseRequested 往返），失败时兜底 window.close()。
function closeSelf(widgetId) {
  if (invoke) {
    invoke("close_widget", { id: widgetId })
      .catch(() => {
        // 后端关闭失败（如 IPC 忙）时，尝试浏览器原生关闭
        try { window.close(); } catch (_) { /* ignore */ }
      });
  } else {
    window.close();
  }
}

// 统一关闭按钮
function setupWidgetClose(widgetId) {
  const btn = document.getElementById("widget-close");
  if (!btn) return;
  btn.addEventListener("click", () => closeSelf(widgetId));
}

// Esc 关闭窗口
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    const btn = document.getElementById("widget-close");
    if (btn) btn.click();
  }
});

// 显示状态文字（兼容各 widget 的 status 元素）
function setStatus(text, kind) {
  const el = document.getElementById("status");
  if (!el) return;
  el.className = "status" + (kind ? " " + kind : "");
  if (text.includes("...") || text.includes("中")) {
    el.innerHTML = '<span class="spinner"></span> ' + text;
  } else {
    el.textContent = text;
  }
}

// 统一剪贴板读写：navigator.clipboard 在 WebView2 里经常因权限静默失败，
// 优先走后端命令（写入还会进入剪贴板历史），失败再回退 Web API。
async function writeClipboard(text) {
  if (invoke) {
    try { await invoke("widget_clipboard_write", { text }); return; } catch (_) { /* fall through */ }
  }
  await navigator.clipboard.writeText(text);
}

async function readClipboard() {
  if (invoke) {
    try { return await invoke("widget_read_clipboard"); } catch (_) { /* fall through */ }
  }
  return await navigator.clipboard.readText();
}
