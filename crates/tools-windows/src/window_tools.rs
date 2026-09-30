//! Window management tools: list visible top-level windows and focus one.
//!
//! Uses `windows` crate directly (Win32 `EnumWindows`, `SetForegroundWindow`).
//! On non-Windows targets every tool reports a graceful error so the crate
//! stays compilable for CI on other platforms.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;

/// List all visible top-level windows, optionally filtered by title substring.
pub struct ListWindowsTool;

#[async_trait]
impl Tool for ListWindowsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_windows".into(),
            description: "列出当前所有可见的顶层窗口，返回窗口标题和所属进程名。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filter": {"type": "string", "description": "窗口标题包含的子串过滤（可选，不区分大小写）"}
                }
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let filter = params
            .get("filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());

        #[cfg(windows)]
        {
            match list_windows_impl(filter.as_deref()) {
                Ok(windows) => {
                    if windows.is_empty() {
                        ToolResult::ok("没有找到符合条件的窗口".to_string())
                    } else {
                        let mut output = format!("共找到 {} 个窗口：\n", windows.len());
                        for w in &windows {
                            output.push_str(&format!("- [{}] {}\n", w.process_name, w.title));
                        }
                        ToolResult::ok(output)
                    }
                }
                Err(e) => ToolResult::err(format!("枚举窗口失败: {e}")),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = filter;
            ToolResult::err("list_windows 仅支持 Windows".to_string())
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

/// Bring a window matching the given title substring to the foreground.
pub struct FocusWindowTool;

#[async_trait]
impl Tool for FocusWindowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "focus_window".into(),
            description: "将标题包含指定文字的窗口切换到前台并激活。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title_contains": {"type": "string", "description": "目标窗口标题包含的文字"}
                },
                "required": ["title_contains"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let title = match params.get("title_contains").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim(),
            _ => return ToolResult::err("缺少有效的 title_contains 参数"),
        };

        #[cfg(windows)]
        {
            match focus_window_impl(title) {
                Ok(true) => ToolResult::ok(format!("已聚焦到包含 \"{title}\" 的窗口")),
                Ok(false) => ToolResult::err(format!("没有找到标题包含 \"{title}\" 的窗口")),
                Err(e) => ToolResult::err(format!("聚焦窗口失败: {e}")),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = title;
            ToolResult::err("focus_window 仅支持 Windows".to_string())
        }
    }
}

/// Capture a screenshot of the full screen or a region, returned as a PNG
/// data URL that the model or frontend can render directly.
pub struct CaptureScreenTool;

#[async_trait]
impl Tool for CaptureScreenTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "capture_screen".into(),
            description: "截取屏幕截图，返回 PNG data URL。可截取整个屏幕或指定区域。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "x": {"type": "integer", "description": "区域左上角 X 坐标（可选，默认 0）"},
                    "y": {"type": "integer", "description": "区域左上角 Y 坐标（可选，默认 0）"},
                    "width": {"type": "integer", "description": "区域宽度（可选，默认全屏宽度）"},
                    "height": {"type": "integer", "description": "区域高度（可选，默认全屏高度）"}
                }
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let w = params.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let h = params.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        let has_region = w > 0 && h > 0;

        #[cfg(windows)]
        {
            let result = if has_region {
                crate::screen_capture::capture_region_as_data_url(x, y, w, h)
            } else {
                crate::screen_capture::capture_screen_as_data_url()
            };
            match result {
                Ok(data_url) => {
                    let size_kb = data_url.len() / 1024;
                    ToolResult::ok_with_data(
                        format!("已截取屏幕（{}KB data URL）", size_kb),
                        json!({ "image": data_url }),
                    )
                }
                Err(e) => ToolResult::err(format!("截图失败: {e}")),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (x, y, w, h, has_region);
            ToolResult::err("capture_screen 仅支持 Windows".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
struct WindowEntry {
    title: String,
    process_name: String,
}

#[cfg(windows)]
fn list_windows_impl(filter: Option<&str>) -> Result<Vec<WindowEntry>, String> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};

    // EnumWindows is callback-based; collect into a Mutex<Vec> shared via LPARAM.
    let results: Mutex<Vec<(HWND, String)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let results = unsafe { &*(lparam.0 as *const Mutex<Vec<(HWND, String)>>) };
        // Only visible top-level windows with a non-empty title.
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            let mut buf = [0u16; 512];
            let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
            if len > 0 {
                let title = String::from_utf16_lossy(&buf[..len as usize]);
                results.lock().unwrap().push((hwnd, title));
            }
        }
        BOOL(1) // continue enumeration
    }

    unsafe {
        EnumWindows(
            Some(enum_proc),
            LPARAM(&results as *const Mutex<Vec<(HWND, String)>> as isize),
        )
        .map_err(|e| format!("EnumWindows: {e}"))?;
    }

    let entries = results.into_inner().unwrap();
    let mut out = Vec::new();
    for (hwnd, title) in entries {
        if let Some(f) = filter {
            if !title.to_lowercase().contains(f) {
                continue;
            }
        }
        let info = assistant_windows::window_info(hwnd.0 as isize);
        out.push(WindowEntry {
            title,
            process_name: info.map(|i| i.process_name).unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(out)
}

#[cfg(windows)]
fn focus_window_impl(title_contains: &str) -> Result<bool, String> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
        SW_RESTORE,
    };

    let needle = title_contains.to_lowercase();
    let found: Mutex<Option<HWND>> = Mutex::new(None);

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = unsafe { &*(lparam.0 as *const (Mutex<Option<HWND>>, String)) };
        let (found, needle) = state;
        if found.lock().unwrap().is_some() {
            return BOOL(0); // already found, stop
        }
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            let mut buf = [0u16; 512];
            let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
            if len > 0 {
                let title = String::from_utf16_lossy(&buf[..len as usize]);
                if title.to_lowercase().contains(needle.as_str()) {
                    *found.lock().unwrap() = Some(hwnd);
                    return BOOL(0); // stop enumeration
                }
            }
        }
        BOOL(1)
    }

    let state = (found, needle);
    // EnumWindows returns Err only when the callback stops early; that's our
    // success path, so ignore the result and just inspect `state`.
    let _ = unsafe {
        EnumWindows(
            Some(enum_proc),
            LPARAM(&state as *const (Mutex<Option<HWND>>, String) as isize),
        )
    };

    let hwnd = match state.0.into_inner().unwrap() {
        Some(h) => h,
        None => return Ok(false),
    };

    unsafe {
        // Restore if minimized so SetForegroundWindow can take effect.
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        // SetForegroundWindow may be refused if our process isn't in the
        // foreground; AllowSetForegroundWindow tricks are deliberately avoided
        // — the caller can retry or focus manually.
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(true)
}
