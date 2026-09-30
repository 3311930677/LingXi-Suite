//! 循环检测（P2-2）：同 `(工具, 参数)` 反复调用的防护。
//!
//! 判定口径：滑动窗口内统计同 `(tool, args_hash)` 出现次数——
//! - ≥ [`NUDGE_AT`]：注入提醒（模型应换策略或结束）；
//! - ≥ [`WRAP_UP_AT`]：强制收尾（agent 循环 break，走无工具总结路径）。
//!
//! 豁免名单（本就设计为同参重复调用）：轮询类 `desktop_wait_until` / `desktop_wait`、
//! 编辑后重读的 `read_file`、搜索类 `grep` / `search_files`。

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use serde_json::Value;

/// 滑窗容量。
pub const WINDOW: usize = 12;
/// 提醒阈值。
pub const NUDGE_AT: usize = 3;
/// 强制收尾阈值。
pub const WRAP_UP_AT: usize = 5;

/// 豁免工具名单。
pub const EXEMPT_TOOLS: &[&str] = &[
    "desktop_wait_until",
    "desktop_wait",
    "read_file",
    "grep",
    "search_files",
];

/// 提醒注入文本（模型可见）。
pub const LOOP_NUDGE_PROMPT: &str = "（系统提示）你似乎在重复调用同一个工具且参数相同。\
     请不要继续重复同一操作：换一种策略（改参数/换工具/先分析已有结果），或直接给出当前结论。";
/// 强制收尾注入文本。
pub const LOOP_WRAP_UP_PROMPT: &str =
    "（系统提示）检测到同一工具调用重复次数过多，已强制结束工具调用阶段。\
     请不要再调用工具，立即基于已有信息给出最终结论。";

/// 循环判定级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopVerdict {
    /// 正常。
    Ok,
    /// 注入提醒。
    Nudge,
    /// 强制收尾。
    WrapUp,
}

/// 滑窗循环检测器。
#[derive(Debug)]
pub struct LoopDetector {
    window: VecDeque<(String, u64)>,
}

impl Default for LoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopDetector {
    pub fn new() -> Self {
        Self {
            window: VecDeque::with_capacity(WINDOW),
        }
    }

    /// 观察一次工具调用，返回应对级别。
    pub fn observe(&mut self, tool: &str, args: &Value) -> LoopVerdict {
        if EXEMPT_TOOLS.contains(&tool) {
            return LoopVerdict::Ok;
        }
        let key = (tool.to_string(), hash_args(args));
        if self.window.len() >= WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(key.clone());
        let count = self.window.iter().filter(|entry| **entry == key).count();
        if count >= WRAP_UP_AT {
            LoopVerdict::WrapUp
        } else if count >= NUDGE_AT {
            LoopVerdict::Nudge
        } else {
            LoopVerdict::Ok
        }
    }
}

/// 参数哈希（对象键排序后归一，键顺序不同视为同参）。
pub fn hash_args(args: &Value) -> u64 {
    let normalized = normalize(args);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}

fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), normalize(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nudges_at_threshold_and_wraps_up_at_limit() {
        let mut detector = LoopDetector::new();
        let args = json!({ "path": "a.txt" });
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::Ok);
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::Ok);
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::Nudge);
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::Nudge);
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::WrapUp);
    }

    #[test]
    fn exempt_tools_never_trigger() {
        let mut detector = LoopDetector::new();
        let args = json!({ "path": "a.txt" });
        for _ in 0..20 {
            assert_eq!(detector.observe("read_file", &args), LoopVerdict::Ok);
            assert_eq!(
                detector.observe("desktop_wait_until", &json!({ "text": "完成" })),
                LoopVerdict::Ok
            );
        }
    }

    #[test]
    fn different_args_do_not_count_together() {
        let mut detector = LoopDetector::new();
        for index in 0..10 {
            let args = json!({ "path": format!("file-{index}.txt") });
            assert_eq!(detector.observe("write_file", &args), LoopVerdict::Ok);
        }
    }

    #[test]
    fn same_args_different_tools_are_separate() {
        let mut detector = LoopDetector::new();
        let args = json!({ "path": "a.txt" });
        expect_ok(detector.observe("write_file", &args));
        expect_ok(detector.observe("edit_file", &args));
        expect_ok(detector.observe("write_file", &args));
        expect_ok(detector.observe("edit_file", &args));
        // 各自第 3 次才提醒。
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::Nudge);
        assert_eq!(detector.observe("edit_file", &args), LoopVerdict::Nudge);
    }

    #[test]
    fn hash_is_key_order_insensitive() {
        let left = json!({ "a": 1, "b": { "x": 2, "y": 3 } });
        let right = json!({ "b": { "y": 3, "x": 2 }, "a": 1 });
        assert_eq!(hash_args(&left), hash_args(&right));
        assert_ne!(hash_args(&left), hash_args(&json!({ "a": 1, "b": 2 })));
    }

    #[test]
    fn window_slides_out_old_entries() {
        let mut detector = LoopDetector::new();
        let args = json!({ "path": "a.txt" });
        // 先制造 2 次同参调用（未达阈值）。
        expect_ok(detector.observe("write_file", &args));
        expect_ok(detector.observe("write_file", &args));
        // 用不同参数填满滑窗，旧记录被挤出。
        for index in 0..WINDOW {
            let other = json!({ "path": format!("f{index}.txt") });
            expect_ok(detector.observe("write_file", &other));
        }
        // 同参计数清零，重新计数。
        expect_ok(detector.observe("write_file", &args));
        expect_ok(detector.observe("write_file", &args));
        assert_eq!(detector.observe("write_file", &args), LoopVerdict::Nudge);
    }

    fn expect_ok(verdict: LoopVerdict) {
        assert_eq!(verdict, LoopVerdict::Ok);
    }
}
