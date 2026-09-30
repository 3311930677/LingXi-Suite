//! `probe` — a small diagnostic tool.
//!
//! Focus a control in any application, then run this tool to see which UIA
//! patterns that control exposes. Use it during W0 to survey target
//! applications before building the real read/write channels.

use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    println!("cross-app-assistant :: UIA capability probe (W0)");
    println!("Focus a control in the target app during the countdown.\n");

    // Give the user a moment to switch to the target window and click into a
    // control, since otherwise the focused element would be this console.
    print!("Inspecting in 3s ");
    for _ in 0..3 {
        sleep(Duration::from_secs(1));
        print!(".");
        let _ = std::io::stdout().flush();
    }
    println!("\n");

    match assistant_windows::probe_focused() {
        Ok(p) => {
            println!("Focused element");
            println!("  name         : {}", show(&p.name));
            println!("  class name   : {}", show(&p.class_name));
            println!("  control type : {}", p.control_type);
            println!("  is password  : {}", p.is_password);
            println!("  is enabled   : {}", p.is_enabled);
            println!("  is readonly  : {}", p.is_readonly);
            println!();
            println!("Capabilities");
            println!("  TextPattern       : {}", p.capability.has_text_pattern);
            println!("  ValuePattern      : {}", p.capability.has_value_pattern);
            println!("  recommended read  : {:?}", p.capability.recommended_read);
            println!("  recommended write : {:?}", p.capability.recommended_write);
            println!();
            println!("Selection");
            println!("  strategy     : {:?}", p.selection.strategy);
            println!(
                "  length       : {} chars{}",
                p.selection.text.chars().count(),
                if p.selection.truncated {
                    " (truncated)"
                } else {
                    ""
                }
            );
            println!("  text         : {}", preview(&p.selection.text));
        }
        Err(e) => {
            eprintln!("probe failed: {e}");
            std::process::exit(1);
        }
    }
}

fn show(s: &str) -> String {
    if s.is_empty() {
        "<empty>".to_string()
    } else {
        s.to_string()
    }
}

/// One-line, escaped preview of read text (first 80 chars).
fn preview(s: &str) -> String {
    if s.is_empty() {
        return "<none>".to_string();
    }
    let snippet: String = s.chars().take(80).collect();
    let escaped = snippet.replace('\r', "\\r").replace('\n', "\\n");
    let ellipsis = if s.chars().count() > 80 { "..." } else { "" };
    format!("\"{escaped}{ellipsis}\"")
}
