//! Current date/time tool.
//!
//! No external crates: uses `SystemTime` for the UTC timestamp and reads the
//! Windows local UTC offset via the `time-zone` feature of the `windows`
//! crate when available, falling back to plain UTC elsewhere.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

/// Get the current date and time.
pub struct GetTimeTool;

#[async_trait]
impl Tool for GetTimeTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "get_time".into(),
            description:
                "获取当前日期和时间。默认返回本地时间，也可指定 utc 或 utc+8 这类固定偏移。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "timezone": {"type": "string", "description": "时区：local（默认）、utc、或 utc+N / utc-N（如 utc+8）"}
                }
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let tz = params
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("local")
            .trim()
            .to_lowercase();

        let offset_seconds = match parse_offset(&tz) {
            Ok(o) => o,
            Err(e) => return ToolResult::err(e),
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let civil = CivilDateTime::from_unix(now + offset_seconds);
        let tz_label = if tz == "local" {
            format!("本地时间（UTC{}）", format_offset(offset_seconds))
        } else if offset_seconds == 0 {
            "UTC".to_string()
        } else {
            format!("UTC{}", format_offset(offset_seconds))
        };

        ToolResult::ok(format!(
            "{}（{}，星期{}）",
            civil.format(),
            tz_label,
            weekday_cn(civil.weekday)
        ))
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

/// Parse a timezone spec into an offset in seconds east of UTC.
fn parse_offset(tz: &str) -> Result<i64, String> {
    match tz {
        "utc" | "z" | "gmt" => Ok(0),
        "local" => Ok(local_offset_seconds()),
        s if s.starts_with("utc+") || s.starts_with("gmt+") => {
            parse_hours(&s[4..]).map(|h| h * 3600)
        }
        s if s.starts_with("utc-") || s.starts_with("gmt-") => {
            parse_hours(&s[4..]).map(|h| -h * 3600)
        }
        other => Err(format!(
            "无法识别的时区 \"{other}\"。支持 local、utc、utc+8、utc-5 等格式"
        )),
    }
}

/// Parse an integer hour offset ("8", "5"). Fractional/colon forms like
/// "8.5" or "5:30" are rejected — keep the surface small and predictable.
fn parse_hours(s: &str) -> Result<i64, String> {
    let hours: i64 = s
        .parse()
        .map_err(|_| format!("无效的时区偏移: \"{s}\"（仅支持整数小时，如 utc+8）"))?;
    if !(0..=14).contains(&hours) {
        return Err(format!("时区偏移超出 ±14 小时: {s}"));
    }
    Ok(hours)
}

/// Local UTC offset in seconds. Windows: `GetTimeZoneInformation`.
/// Other platforms: 0 (UTC) — accurate local-time zones need `chrono-tz`,
/// which we deliberately avoid.
#[cfg(windows)]
fn local_offset_seconds() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};

    // SAFETY: writes into a caller-provided struct; no preconditions.
    let mut info = TIME_ZONE_INFORMATION::default();
    let _ = unsafe { GetTimeZoneInformation(&mut info) };
    // Bias is minutes to ADD to local time to get UTC, so invert the sign.
    -(info.Bias as i64) * 60
}

#[cfg(not(windows))]
fn local_offset_seconds() -> i64 {
    0
}

/// Days-from-epoch → civil calendar conversion (Howard Hinnant's algorithm),
/// avoiding a chrono dependency.
struct CivilDateTime {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    /// 0 = Sunday … 6 = Saturday
    weekday: u32,
}

impl CivilDateTime {
    fn from_unix(secs: i64) -> Self {
        let days = secs.div_euclid(86_400);
        let secs_of_day = secs.rem_euclid(86_400);

        // civil_from_days
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097); // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
        let year = if m <= 2 { y + 1 } else { y };

        // 1970-01-01 was a Thursday (4).
        let weekday = (days.rem_euclid(7) + 4) % 7;

        Self {
            year,
            month: m,
            day: d,
            hour: (secs_of_day / 3600) as u32,
            minute: ((secs_of_day % 3600) / 60) as u32,
            second: (secs_of_day % 60) as u32,
            weekday: weekday as u32,
        }
    }

    fn format(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

fn weekday_cn(weekday: u32) -> &'static str {
    match weekday {
        0 => "日",
        1 => "一",
        2 => "二",
        3 => "三",
        4 => "四",
        5 => "五",
        6 => "六",
        _ => "?",
    }
}

fn format_offset(seconds: i64) -> String {
    if seconds == 0 {
        return "+00:00".to_string();
    }
    let sign = if seconds > 0 { "+" } else { "-" };
    let abs = seconds.abs();
    format!("{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970_01_01_thursday() {
        let dt = CivilDateTime::from_unix(0);
        assert_eq!(dt.format(), "1970-01-01 00:00:00");
        assert_eq!(dt.weekday, 4); // Thursday
    }

    #[test]
    fn known_timestamp() {
        // 2026-08-11 12:34:56 UTC = unix 1786451696
        let dt = CivilDateTime::from_unix(1_786_451_696);
        assert_eq!(dt.year, 2026);
        assert_eq!(dt.month, 8);
        assert_eq!(dt.day, 11);
        assert_eq!(dt.weekday, 2); // Tuesday
    }

    #[test]
    fn beijing_offset_shifts_date_correctly() {
        // 2026-08-11 00:30 UTC + 8h → 08:30 same day
        let utc = CivilDateTime::from_unix(1_786_406_400 + 1_800);
        let bj = CivilDateTime::from_unix(1_786_406_400 + 1_800 + 8 * 3600);
        assert_eq!(bj.hour, utc.hour + 8);
    }

    #[test]
    fn parse_offsets() {
        assert_eq!(parse_offset("utc").unwrap(), 0);
        assert_eq!(parse_offset("utc+8").unwrap(), 8 * 3600);
        assert_eq!(parse_offset("utc-5").unwrap(), -5 * 3600);
        assert!(parse_offset("mars").is_err());
        assert!(parse_offset("utc+99").is_err());
    }
}
