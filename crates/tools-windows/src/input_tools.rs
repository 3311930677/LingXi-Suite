//! Input simulation tools: type text, send key chords, click screen points.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;

/// Type Unicode text into the currently focused control.
pub struct TypeTextTool;

#[async_trait]
impl Tool for TypeTextTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "type_text".into(),
            description: "在当前焦点控件中逐字符输入文本（支持中文等 Unicode）。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "要输入的文本"}
                },
                "required": ["text"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let text = match params.get("text").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => return ToolResult::err("缺少有效的 text 参数"),
        };

        #[cfg(windows)]
        {
            match assistant_windows::type_unicode(text) {
                Ok(()) => ToolResult::ok(format!("已输入 {} 个字符", text.chars().count())),
                Err(e) => ToolResult::err(format!("输入失败: {e}")),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = text;
            ToolResult::err("type_text 仅支持 Windows".to_string())
        }
    }
}

/// Send a key chord such as `ctrl+c`, `alt+tab`, `enter`.
pub struct SendKeysTool;

#[async_trait]
impl Tool for SendKeysTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "send_keys".into(),
            description: "发送键盘快捷键组合。格式：ctrl+c、ctrl+shift+v、alt+tab、enter、esc 等。支持修饰键 ctrl/alt/shift/win 与主键 a-z、0-9、f1-f12、enter、esc、tab、space、backspace、delete、方向键。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "keys": {"type": "string", "description": "快捷键组合，用 + 连接"}
                },
                "required": ["keys"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let keys = match params.get("keys").and_then(|v| v.as_str()) {
            Some(k) if !k.trim().is_empty() => k.trim(),
            _ => return ToolResult::err("缺少有效的 keys 参数"),
        };

        #[cfg(windows)]
        {
            match send_keys_impl(keys) {
                Ok(()) => ToolResult::ok(format!("已发送快捷键: {keys}")),
                Err(e) => ToolResult::err(format!("发送快捷键失败: {e}")),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = keys;
            ToolResult::err("send_keys 仅支持 Windows".to_string())
        }
    }
}

/// Click a screen point after validating coordinates against the primary screen.
pub struct ClickAtTool;

#[async_trait]
impl Tool for ClickAtTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "click_at".into(),
            description: "点击屏幕上的指定坐标（像素），执行前会校验坐标位于主屏幕范围内。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "x": {"type": "integer", "description": "屏幕 X 坐标（像素）"},
                    "y": {"type": "integer", "description": "屏幕 Y 坐标（像素）"}
                },
                "required": ["x", "y"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let x = match params.get("x").and_then(|v| v.as_i64()) {
            Some(value) if (0..=i32::MAX as i64).contains(&value) => value as i32,
            _ => return ToolResult::err("x 必须是非负整数坐标"),
        };
        let y = match params.get("y").and_then(|v| v.as_i64()) {
            Some(value) if (0..=i32::MAX as i64).contains(&value) => value as i32,
            _ => return ToolResult::err("y 必须是非负整数坐标"),
        };

        #[cfg(windows)]
        {
            let width = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
                )
            };
            let height = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN,
                )
            };
            if width <= 0 || height <= 0 || x >= width || y >= height {
                return ToolResult::err(format!("坐标超出主屏幕范围: ({x}, {y})"));
            }
            return match assistant_windows::click_at(x, y) {
                Ok(()) => ToolResult::ok(format!("已点击坐标 ({x}, {y})")),
                Err(error) => ToolResult::err(format!("点击失败: {error}")),
            };
        }

        #[cfg(not(windows))]
        {
            let _ = (x, y);
            ToolResult::err("click_at 仅支持 Windows")
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn send_keys_impl(keys: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10,
        VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT,
        VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
        VK_TAB, VK_UP,
    };

    /// Parse a key name into its virtual-key code.
    fn parse_key(name: &str) -> Result<VIRTUAL_KEY, String> {
        let lower = name.trim().to_lowercase();
        let key = match lower.as_str() {
            "ctrl" | "control" => VK_CONTROL.0,
            "alt" | "menu" => VK_MENU.0,
            "shift" => VK_SHIFT.0,
            "win" | "windows" | "meta" => VK_LWIN.0,
            "enter" | "return" => VK_RETURN.0,
            "esc" | "escape" => VK_ESCAPE.0,
            "tab" => VK_TAB.0,
            "space" => VK_SPACE.0,
            "backspace" | "back" => VK_BACK.0,
            "delete" | "del" => VK_DELETE.0,
            "insert" | "ins" => VK_INSERT.0,
            "home" => VK_HOME.0,
            "end" => VK_END.0,
            "pageup" | "prior" => VK_PRIOR.0,
            "pagedown" | "next" => VK_NEXT.0,
            "up" => VK_UP.0,
            "down" => VK_DOWN.0,
            "left" => VK_LEFT.0,
            "right" => VK_RIGHT.0,
            "capslock" => VK_CAPITAL.0,
            "f1" => VK_F1.0,
            "f2" => VK_F2.0,
            "f3" => VK_F3.0,
            "f4" => VK_F4.0,
            "f5" => VK_F5.0,
            "f6" => VK_F6.0,
            "f7" => VK_F7.0,
            "f8" => VK_F8.0,
            "f9" => VK_F9.0,
            "f10" => VK_F10.0,
            "f11" => VK_F11.0,
            "f12" => VK_F12.0,
            s if s.len() == 1 => {
                let c = s.chars().next().unwrap().to_ascii_uppercase();
                if c.is_ascii_alphanumeric() {
                    c as u16
                } else {
                    return Err(format!("不支持的单字符按键: {s}"));
                }
            }
            other => return Err(format!("未知按键: {other}")),
        };
        Ok(VIRTUAL_KEY(key))
    }

    let parts: Vec<&str> = keys.split('+').collect();
    if parts.is_empty() {
        return Err("keys 不能为空".to_string());
    }

    let mut vks = Vec::with_capacity(parts.len());
    for part in parts {
        vks.push(parse_key(part)?);
    }

    // Build press events for all keys, then release events in reverse order.
    let mut inputs = Vec::with_capacity(vks.len() * 2);
    for vk in &vks {
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: *vk,
                    wScan: 0,
                    dwFlags: Default::default(),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
    for vk in vks.iter().rev() {
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: *vk,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }

    // SAFETY: inputs is a valid contiguous INPUT array and cbSize matches.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(format!("SendInput sent {sent}/{} events", inputs.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn parse_common_chords() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_C, VK_CONTROL, VK_RETURN};

        // Internal helper is private; exercise via the public error surface
        // of a known-good chord and a known-bad one.
        assert!(send_keys_impl("ctrl+c").is_ok());
        assert!(send_keys_impl("enter").is_ok());
        assert!(send_keys_impl("ctrl+shift+delete").is_ok());
        assert!(send_keys_impl("not_a_key").is_err());
        assert!(send_keys_impl("").is_err());

        // Sanity: VK constants are distinct.
        assert_ne!(VK_CONTROL.0, VK_C.0);
        assert_ne!(VK_RETURN.0, VK_C.0);
    }
}
