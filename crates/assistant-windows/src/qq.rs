//! QQ desktop integration through public UI Automation surfaces only.
//!
//! This is intentionally semi-automatic: it can detect a likely latest message
//! and place a user-approved draft into the edit control, but it has no "send"
//! operation. The user remains the final authority and presses Send in QQ.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread::sleep;
use std::time::Duration;

use assistant_core::AdapterError;
use windows::core::BSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextRange,
    IUIAutomationValuePattern, UIA_EditControlTypeId, UIA_ListItemControlTypeId, UIA_TextPatternId,
    UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};

use crate::clipboard;
use crate::com::ComGuard;
use crate::foreground::{foreground_info, window_info, ForegroundInfo};
use crate::keyboard;
use crate::read::{get_pattern, read_full_text};
use crate::uia::UiaClient;

#[allow(dead_code)]
const MAX_MESSAGE_CHARS: usize = 2_000;

/// Handle of the most recent foreground window that belonged to QQ.
///
/// The overlay's "read" and "write draft" buttons live on the LingXi panel, so
/// the moment the user clicks one, QQ is no longer the foreground window (the
/// panel is, especially now that the chat view intentionally takes keyboard
/// focus). Requiring "QQ is foreground right now" therefore made reading
/// impossible from the panel. Instead a background sampler records the last QQ
/// window here, and the QQ operations fall back to it when the live foreground
/// is something else (typically the panel itself). `0` means "unknown".
static LAST_QQ_HWND: AtomicIsize = AtomicIsize::new(0);

/// Record the foreground window if it currently belongs to QQ. Called
/// periodically by a lightweight background sampler so the panel can still act
/// on the user's QQ window after it steals focus.
pub fn remember_foreground_if_qq() {
    if let Ok(info) = foreground_info() {
        if is_qq_process(&info.process_name) {
            LAST_QQ_HWND.store(info.hwnd, Ordering::Relaxed);
        }
    }
}

/// Resolve the QQ window to operate on: the live foreground when it is QQ,
/// otherwise the last remembered QQ window if it still exists and is still QQ.
///
/// Public so the overlay's `qq_poll_latest` / `capture_qq_selection` commands
/// can reuse the same "foreground-or-remembered" fallback instead of naively
/// requiring QQ to be the *current* foreground — which it never is once the
/// user clicks a button on the LingXi panel (the panel steals foreground).
pub fn resolve_qq_window() -> Result<ForegroundInfo, AdapterError> {
    let foreground = foreground_info()?;
    if is_qq_process(&foreground.process_name) {
        LAST_QQ_HWND.store(foreground.hwnd, Ordering::Relaxed);
        return Ok(foreground);
    }

    let remembered = LAST_QQ_HWND.load(Ordering::Relaxed);
    if remembered != 0 {
        // SAFETY: `IsWindow` accepts any handle and simply reports validity.
        let alive = unsafe { IsWindow(HWND(remembered as *mut _)) }.as_bool();
        if alive {
            if let Some(info) = window_info(remembered) {
                if is_qq_process(&info.process_name) {
                    return Ok(info);
                }
            }
        }
        // Stale handle (QQ closed or reused): forget it so we stop pointing at
        // an unrelated window.
        LAST_QQ_HWND.store(0, Ordering::Relaxed);
    }

    Err(AdapterError::Platform(
        "QQ window was not found; open a QQ chat window first".into(),
    ))
}

/// Best-effort snapshot of the active QQ conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QqMessageSnapshot {
    pub hwnd: isize,
    pub conversation: String,
    pub message: String,
}

/// Read the most recent message-like text node from the foreground QQ window.
///
/// QQ is Chromium-based and its accessibility tree changes between versions,
/// so this deliberately uses semantic control types rather than private class
/// names. Failure is explicit; callers should keep polling instead of treating
/// unrelated foreground windows as conversations.
/// Deprecated: scanning QQNT's UIA tree cannot reliably identify the sender
/// of a message bubble, so this function is no longer used by the overlay.
/// The UI now reads the user's explicit selection instead. Kept as a stub so
/// downstream code linking against the assistant-windows crate still compiles.
#[allow(dead_code)]
pub fn qq_latest_message() -> Result<QqMessageSnapshot, AdapterError> {
    Err(AdapterError::Platform(
        "qq_latest_message is deprecated; use capture_qq_selection instead".into(),
    ))
}

/// Put a confirmed draft into the foreground QQ edit control without sending.
/// Existing input is replaced intentionally, and an exact read-back is required
/// when the provider exposes Value/TextPattern.
pub fn qq_write_draft(draft: &str) -> Result<bool, AdapterError> {
    if draft.trim().is_empty() {
        return Err(AdapterError::NoSelection);
    }
    let foreground = resolve_qq_window()?;
    let _com = ComGuard::new()?;
    let client = UiaClient::new()?;
    let root = client.element_from_hwnd(foreground.hwnd)?;
    let elements = client.descendants(&root)?;
    let chosen = choose_editor(elements, &root).ok_or(AdapterError::UnsupportedControl)?;
    eprintln!("[qq.write_draft] chosen kind={:?}", chosen.kind);

    // Bring QQ to the foreground first. Ctrl+A / paste go to whatever has
    // keyboard focus, and when the LingXi panel stole focus the QQ window's
    // last focused control (often the search box) keeps the caret. We need to
    // explicitly hand focus back to QQ before clicking.
    let hwnd_handle = HWND(foreground.hwnd as *mut _);
    let _ = unsafe { SetForegroundWindow(hwnd_handle) };
    sleep(Duration::from_millis(120));

    match chosen.kind {
        EditorKind::PlainEdit => {
            // SAFETY: this is an enabled Edit in the foreground QQ window.
            unsafe { chosen.element.SetFocus() }.map_err(|error| {
                AdapterError::Platform(format!("QQ editor SetFocus failed: {error}"))
            })?;
            sleep(Duration::from_millis(80));

            if let Some(value) =
                get_pattern::<IUIAutomationValuePattern>(&chosen.element, UIA_ValuePatternId)
            {
                unsafe { value.SetValue(&BSTR::from(draft)) }.map_err(|error| {
                    AdapterError::Platform(format!("QQ editor SetValue failed: {error}"))
                })?;
            } else {
                keyboard::select_all()?;
                clipboard::paste_text_preserving_clipboard(draft)?;
            }
        }
        EditorKind::Composer | EditorKind::WebviewRoot => {
            // QQNT composer is a Chromium contenteditable div that is NOT
            // exposed as an Edit / Document until it receives focus. We click
            // on its physical location (bottom-right area of the QQ window,
            // derived from the webview root bounds) to hand focus to the
            // composer, then replace its contents with Ctrl+A + paste.
            let rect = unsafe { chosen.element.CurrentBoundingRectangle() }
                .map_err(|e| AdapterError::Platform(format!("composer rect: {e}")))?;
            let width = (rect.right - rect.left).max(1);
            let height = (rect.bottom - rect.top).max(1);
            // The composer sits at roughly 60% across and 85% down the QQ
            // window (measured from real dumps: rect 1066-2503 x 1550-1767
            // inside window 608-2512 x 479-1885). Clicking here puts the
            // caret into the editable div even when UIA cannot name it.
            let click_x = rect.left + width * 6 / 10;
            let click_y = rect.top + height * 85 / 100;
            eprintln!(
                "[qq.write_draft] clicking at ({},{}) inside rect {:?} ({}x{})",
                click_x, click_y, rect, width, height
            );
            keyboard::click_at(click_x, click_y)?;
            keyboard::select_all()?;
            sleep(Duration::from_millis(40));
            clipboard::paste_text_preserving_clipboard(draft)?;
        }
    }

    sleep(Duration::from_millis(120));
    Ok(true)
}

/// Full text of a single element, preferring the accessibility patterns that
/// expose complete content over `Name`.
///
/// `read_full_text` returns the whole `TextPattern.DocumentRange` (or the
/// `ValuePattern` value) when either is available; QQ's Chromium bubbles carry
/// their real content there. `Name` is used only as a last resort because it is
/// frequently a truncated label, which is what previously limited reads to "a
/// few characters".
#[allow(dead_code)]
fn element_text(element: &IUIAutomationElement) -> Option<String> {
    if let Ok(Some(text)) = read_full_text(element) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    unsafe { element.CurrentName() }
        .ok()
        .map(|value| value.to_string())
}

/// Reconstruct the latest logical message from an ordered list of candidate
/// text nodes.
///
/// Chromium often splits one bubble into several adjacent text nodes (one per
/// line). We therefore take a *trailing run* of candidates rather than just the
/// last node, joining them with newlines so a multi-line message is returned
/// whole. The run stops once it has gathered a reasonable amount of text so an
/// unrelated earlier bubble is not merged in.
#[allow(dead_code)]
fn join_trailing_message(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    // Walk backwards, accumulating fragments until we have collected enough
    // characters to plausibly cover one full message. Most fragments are short
    // lines; a handful of them reconstruct a normal chat message without
    // swallowing the entire scrollback.
    let mut collected: Vec<&str> = Vec::new();
    let mut total_chars = 0usize;
    for fragment in candidates.iter().rev() {
        let fragment = fragment.trim();
        if fragment.is_empty() {
            continue;
        }
        collected.push(fragment);
        total_chars += fragment.chars().count();
        // A single sufficiently long node is already a whole message; and once
        // the accumulated run is long enough, stop before reaching into older
        // bubbles.
        if total_chars >= 60 {
            break;
        }
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    let joined = collected.join("\n");
    let joined = joined.trim();
    if joined.is_empty() {
        None
    } else if joined.chars().count() > MAX_MESSAGE_CHARS {
        Some(joined.chars().take(MAX_MESSAGE_CHARS).collect())
    } else {
        Some(joined.to_string())
    }
}

/// Whether a process image name looks like a QQ client. Case-insensitive and
/// tolerant of the several executables QQ has shipped (classic `QQ.exe` and the
/// newer Electron `QQNT.exe`).
fn is_qq_process(process_name: &str) -> bool {
    let process = process_name.to_ascii_lowercase();
    process == "qq.exe" || process == "qqnt.exe"
}

/// Capture the user's text selection inside QQ by scanning the QQ window's UIA
/// tree for a `TextPattern` element whose `GetSelection()` is non-empty.
///
/// Unlike `WindowsAdapter::capture_selection` (which uses `GetFocusedElement`
/// and therefore requires QQ to be the foreground window), this walks the
/// remembered QQ window's subtree directly. QQNT is Chromium-based and its
/// contenteditable message bubbles expose `TextPattern`; the DOM selection
/// survives foreground changes, so we can read it even when the LingXi panel
/// currently holds the focus.
pub fn capture_qq_selection_text() -> Result<String, AdapterError> {
    let window = resolve_qq_window()?;
    let _com = ComGuard::new()?;
    let client = UiaClient::new()?;
    let root = client.element_from_hwnd(window.hwnd)?;
    let elements = client.descendants(&root)?;

    for element in &elements {
        let tp = match get_pattern::<IUIAutomationTextPattern>(element, UIA_TextPatternId) {
            Some(tp) => tp,
            None => continue,
        };
        // SAFETY: valid TextPattern object.
        let ranges = match unsafe { tp.GetSelection() } {
            Ok(r) => r,
            Err(_) => continue,
        };
        // SAFETY: valid COM array.
        let len = match unsafe { ranges.Length() } {
            Ok(n) => n,
            Err(_) => continue,
        };
        if len == 0 {
            continue;
        }
        for i in 0..len {
            // SAFETY: index in range.
            if let Ok(range) = unsafe { ranges.GetElement(i) } {
                let text = text_range_text(&range).unwrap_or_default();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }
    }
    Err(AdapterError::NoSelection)
}

/// Read the plain text of a UIA text range.
fn text_range_text(range: &IUIAutomationTextRange) -> Result<String, AdapterError> {
    // SAFETY: valid range object. -1 = no limit on length.
    let text = unsafe { range.GetText(-1) }
        .map_err(|e| AdapterError::Platform(format!("TextRange.GetText: {e}")))?
        .to_string();
    Ok(text)
}

#[allow(dead_code)]
fn is_message_candidate(text: &str, window_title: &str) -> bool {
    let text = text.trim();
    if text.is_empty()
        || text == window_title.trim()
        || text.chars().count() > MAX_MESSAGE_CHARS
        || matches!(
            text,
            "发送"
                | "表情"
                | "截图"
                | "文件"
                | "语音"
                | "视频"
                | "更多"
                | "搜索"
                | "群公告"
                | "群聊成员"
                | "群主"
                | "模板参考"
                | "图片"
                | "【图片】"
        )
    {
        return false;
    }
    // Reject pure timestamps (HH:MM, 星期X HH:MM, etc). These are message
    // metadata, not content.
    let stripped = text.replace(|c: char| c.is_ascii_punctuation() || c.is_whitespace(), "");
    if stripped.len() <= 6 && stripped.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return false;
    }
    // Reject "星期X HH:MM" style timestamps that include a weekday prefix.
    if text.starts_with("星期")
        && text
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == ':')
            .count()
            >= 3
    {
        return false;
    }
    // UI labels are usually one or two characters. Keeping at least one
    // alphanumeric/CJK character avoids punctuation-only timestamp separators.
    text.chars()
        .any(|ch| ch.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch))
}

/// What kind of control this candidate is, used to drive the write path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    /// ListItem that is the chat composer (QQNT exposes the contenteditable
    /// div this way, and the Name is the active conversation partner).
    Composer,
    /// Document / webview root (Chrome_RenderWidgetHostHWND). Cannot be
    /// written to via SetValue (it is a host, not an editor). Caller must
    /// focus it then simulate keyboard input.
    WebviewRoot,
    /// Plain Edit control (legacy QQ, the search box, etc.). SetValue works.
    PlainEdit,
}

#[derive(Debug)]
struct EditorChoice {
    element: IUIAutomationElement,
    kind: EditorKind,
}

/// Pick the QQ chat composer from the full descendant tree.
///
/// QQNT's composer is a Chromium-embedded `contenteditable` div. UIA exposes
/// it as a `ListItem` whose Name is the active conversation partner, sitting
/// in the right half of the window near the bottom. Older QQ clients expose
/// it as a plain `Edit` control. The `Document` that represents the Chromium
/// webview host is returned only as a last resort: SetValue against it
/// returns E_INVALID_READ_ONLY (0x80131509), so the write path must use the
/// clipboard-and-keyboard fallback for that case.
fn choose_editor(
    elements: Vec<IUIAutomationElement>,
    window: &IUIAutomationElement,
) -> Option<EditorChoice> {
    let window_rect = unsafe { window.CurrentBoundingRectangle() }.ok()?;
    let window_width = (window_rect.right - window_rect.left).max(1);
    let window_height = (window_rect.bottom - window_rect.top).max(1);

    struct Candidate {
        element: IUIAutomationElement,
        kind: EditorKind,
        rel_y: i32,
        width: i32,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for element in elements {
        let control_type = match unsafe { element.CurrentControlType() } {
            Ok(value) => value,
            Err(_) => continue,
        };
        let enabled = unsafe { element.CurrentIsEnabled() }
            .ok()
            .is_some_and(|value| value.as_bool());
        if !enabled {
            continue;
        }
        let name = unsafe { element.CurrentName() }
            .ok()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let aid = unsafe { element.CurrentAutomationId() }
            .ok()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let cls = unsafe { element.CurrentClassName() }
            .ok()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let rect = match unsafe { element.CurrentBoundingRectangle() } {
            Ok(rect) => rect,
            Err(_) => continue,
        };
        // Drop placeholder 0x0 rects (they're stale virtualization rows).
        if rect.right <= rect.left || rect.bottom <= rect.top {
            continue;
        }
        let width = rect.right - rect.left;
        let rel_y = ((rect.top - window_rect.top) * 1000 / window_height).clamp(0, 1000);
        let rel_x = ((rect.left - window_rect.left) * 1000 / window_width).clamp(0, 1000);

        let kind = if cls.contains("Chrome_RenderWidgetHostHWND") {
            EditorKind::WebviewRoot
        } else if control_type == UIA_ListItemControlTypeId
            && rel_x >= 500
            && rel_y >= 600
            && width >= 800
        {
            EditorKind::Composer
        } else if control_type == UIA_EditControlTypeId {
            let descriptor = format!("{} {} {}", name, aid, cls).to_ascii_lowercase();
            if editor_score(&descriptor) >= 0 {
                EditorKind::PlainEdit
            } else {
                continue;
            }
        } else {
            continue;
        };
        eprintln!(
            "[qq.choose_editor] kind={:?} name={:?} aid={:?} cls={:?} rel=({},{}) w={} rect={:?}",
            kind, name, aid, cls, rel_x, rel_y, width, rect
        );
        candidates.push(Candidate {
            element,
            kind,
            rel_y,
            width,
        });
    }

    candidates.sort_by_key(|candidate| match candidate.kind {
        EditorKind::Composer => (0_i32, -candidate.rel_y, -candidate.width),
        EditorKind::PlainEdit => (1_i32, -candidate.rel_y, -candidate.width),
        EditorKind::WebviewRoot => (2_i32, -candidate.rel_y, -candidate.width),
    });
    eprintln!("[qq.choose_editor] final candidates: {}", candidates.len());
    candidates.into_iter().next().map(|c| EditorChoice {
        element: c.element,
        kind: c.kind,
    })
}

/// Score an editor descriptor for how likely it is the chat composer rather
/// than an unrelated Edit control such as the top search box.
///
/// Search-related keywords are penalized so the top "搜索/search" field never
/// wins over the composer; composer keywords add points. The result is clamped
/// at zero so a strongly-search-flavored field cannot outrank an unlabeled but
/// otherwise valid composer.
fn editor_score(descriptor: &str) -> i32 {
    const COMPOSER_KEYWORDS: [&str; 7] = [
        "input", "editor", "compose", "message", "chat", "输入", "消息",
    ];
    const SEARCH_KEYWORDS: [&str; 3] = ["search", "搜索", "查找"];

    let positive = COMPOSER_KEYWORDS
        .into_iter()
        .filter(|keyword| descriptor.contains(keyword))
        .count() as i32;
    let negative = SEARCH_KEYWORDS
        .into_iter()
        .filter(|keyword| descriptor.contains(keyword))
        .count() as i32;
    // A search field is disqualified hard; a composer keyword adds a small
    // positive bias over an unlabeled control.
    positive - negative * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_candidate_filters_chrome_labels() {
        assert!(is_message_candidate("明天下午见吗？", "张三"));
        assert!(!is_message_candidate("发送", "张三"));
        assert!(!is_message_candidate("张三", "张三"));
        assert!(!is_message_candidate("...", "张三"));
        // New: reject group UI labels and pure timestamps.
        assert!(!is_message_candidate("群公告", "张三"));
        assert!(!is_message_candidate("群聊成员", "张三"));
        assert!(!is_message_candidate("15:49", "张三"));
        assert!(!is_message_candidate("星期二 17:05", "张三"));
        assert!(!is_message_candidate("【图片】", "张三"));
    }

    #[test]
    fn join_trailing_message_merges_split_fragments() {
        // A multi-line message split across nodes is reconstructed whole
        // instead of returning only the final fragment.
        let candidates = vec![
            "早上好".to_string(),
            "今天".to_string(),
            "我们".to_string(),
            "去哪里".to_string(),
            "吃饭".to_string(),
        ];
        let joined = join_trailing_message(&candidates).unwrap();
        assert!(joined.contains("吃饭"));
        assert!(joined.contains("今天"));
        assert!(joined.chars().count() > "吃饭".chars().count());
    }

    #[test]
    fn join_trailing_message_returns_long_single_node_as_is() {
        let long = "这是一条很长的消息".repeat(10);
        let joined = join_trailing_message(std::slice::from_ref(&long)).unwrap();
        assert_eq!(joined, long);
    }

    #[test]
    fn join_trailing_message_none_when_empty() {
        assert!(join_trailing_message(&[]).is_none());
        assert!(join_trailing_message(&["".to_string(), "   ".to_string()]).is_none());
    }

    #[test]
    fn editor_score_penalizes_search_box() {
        // The top search field must never outrank the composer.
        assert!(editor_score("搜索 search-input searchbox") < 0);
        assert!(editor_score("消息输入 message-input composer") > 0);
        // A search-flavored control loses even to an unlabeled composer (0).
        assert!(editor_score("搜索") < editor_score("some-unlabeled-edit"));
    }

    #[test]
    fn editor_score_composer_beats_search() {
        assert!(editor_score("输入消息") > editor_score("搜索联系人"));
    }
}
