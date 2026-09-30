//! 增量 SSE（Server-Sent Events）解析：独立于传输层，可单测。
//!
//! 规范要点（W3C EventSource）：
//! - `field: value`，字段名后的一个前导空格要去掉；
//! - 多行 `data:` 以 `\n` 连接；
//! - 空行结束一个帧；没有 `data` 的帧不派发；
//! - `:` 开头是注释（常用于 keep-alive），忽略。

use serde::de::DeserializeOwned;
use serde::Deserialize;

/// 一个完整的 SSE 帧。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SseFrame {
    /// `event:` 字段（如 `token_delta` / `permission_request`）。
    pub event: Option<String>,
    /// `id:` 字段（可作 Last-Event-ID 续传）。
    pub id: Option<String>,
    /// `retry:` 字段（重连毫秒，服务端可选）。
    pub retry: Option<u64>,
    /// 合并后的 `data:` 负载。
    pub data: String,
}

impl SseFrame {
    /// 把 `data` 解析为指定类型；解析失败返回 None（不 panic，便于 UI 忽略未知帧）。
    pub fn json<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(&self.data).ok()
    }

    pub fn event_name(&self) -> &str {
        self.event.as_deref().unwrap_or_default()
    }
}

/// 增量解析器：逐行喂入，帧结束时返回 [`SseFrame`]。
#[derive(Debug, Default)]
pub struct SseParser {
    frame: SseFrame,
    data_lines: Vec<String>,
    has_field: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一行（不含行尾换行符）。返回 `Some` 表示一个帧已完成。
    pub fn push_line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            if !self.has_field {
                return None;
            }
            let mut frame = std::mem::take(&mut self.frame);
            frame.data = self.data_lines.join("\n");
            self.data_lines.clear();
            self.has_field = false;
            // 没有 data 的帧按规范不派发（例如仅有 event 名）。
            if frame.data.is_empty() {
                return None;
            }
            return Some(frame);
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        self.has_field = true;
        match field {
            "event" => self.frame.event = Some(value.to_string()),
            "id" => self.frame.id = Some(value.to_string()),
            "retry" => self.frame.retry = value.parse().ok(),
            "data" => self.data_lines.push(value.to_string()),
            // 未知字段按规范忽略。
            _ => {}
        }
        None
    }
}

/// 从任意 `BufRead` 驱动解析，直到流结束；每完成一帧回调一次。
pub fn read_frames<R: std::io::BufRead>(
    reader: R,
    mut on_frame: impl FnMut(SseFrame),
) -> std::io::Result<()> {
    let mut parser = SseParser::new();
    for line in reader.lines() {
        let line = line?;
        if let Some(frame) = parser.push_line(&line) {
            on_frame(frame);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &[&str]) -> Vec<SseFrame> {
        let mut parser = SseParser::new();
        let mut frames = Vec::new();
        for line in input {
            if let Some(frame) = parser.push_line(line) {
                frames.push(frame);
            }
        }
        frames
    }

    #[test]
    fn parses_event_and_data_frame() {
        let frames = collect(&[
            "event: token_delta",
            r#"data: {"type":"token_delta","delta":"你"}"#,
            "",
        ]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event_name(), "token_delta");
        let value: serde_json::Value = frames[0].json().unwrap();
        assert_eq!(value["delta"], "你");
    }

    #[test]
    fn joins_multiline_data_with_newline() {
        let frames = collect(&["data: line-1", "data: line-2", ""]);
        assert_eq!(frames[0].data, "line-1\nline-2");
    }

    #[test]
    fn ignores_comments_keepalive_and_empty_dispatches() {
        // 注释行与空行不产生帧；event-only 帧（无 data）按规范不派发。
        let frames = collect(&[": keep-alive", "", "event: ping", "", "data: {}", ""]);
        assert_eq!(frames.len(), 1);
        // event: ping 在第一个空行处已结束（无 data 不派发），
        // 随后的 data 帧是独立的一帧，不带 event 名。
        assert_eq!(frames[0].event_name(), "");
        assert_eq!(frames[0].data, "{}");
    }

    #[test]
    fn event_and_data_in_same_frame_keep_event_name() {
        let frames = collect(&["event: final", "data: {\"type\":\"final\"}", ""]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event_name(), "final");
    }

    #[test]
    fn supports_id_and_retry_fields_and_space_stripping() {
        let frames = collect(&["id: 42", "retry: 3000", "data:hello", ""]);
        assert_eq!(frames[0].id.as_deref(), Some("42"));
        assert_eq!(frames[0].retry, Some(3000));
        assert_eq!(frames[0].data, "hello");
    }

    #[test]
    fn read_frames_drives_from_reader() {
        let payload = "event: progress\ndata: {\"type\":\"progress\",\"message\":\"跑测试\"}\n\n";
        let mut frames = Vec::new();
        read_frames(std::io::Cursor::new(payload), |frame| frames.push(frame)).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data.contains("跑测试"), true);
    }

    #[test]
    fn unknown_event_kind_parses_as_other_via_frame_json() {
        let frames = collect(&["data: {\"type\":\"future_thing\",\"v\":1}", ""]);
        let event: crate::TurnEvent = frames[0].json().unwrap();
        assert_eq!(event, crate::TurnEvent::Other);
    }
}
