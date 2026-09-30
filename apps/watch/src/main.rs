//! `watch` — W1 interactive probe.
//!
//! Runs resident in the console. Whenever you press `Ctrl+Alt+Space` (from any
//! application, without losing your selection), it prints the current
//! foreground window and the focused control's UIA capabilities.
//!
//! This is the manual test harness for W1: window/focus detection + global
//! hotkey. Later milestones will replace the "print" action with the real
//! capture -> generate -> write-back pipeline.

use assistant_windows::{foreground_info, probe_focused, run_hotkey_loop};

fn main() {
    println!("cross-app-assistant :: watch (W1)");
    println!("Press Ctrl+Alt+Space in any app to inspect its focused control.");
    println!("Close this window (or Ctrl+C) to stop.\n");

    let result = run_hotkey_loop(|| {
        println!("---- Ctrl+Alt+Space ----");
        report();
        println!();
    });

    if let Err(e) = result {
        eprintln!("watch failed: {e}");
        std::process::exit(1);
    }
}

/// Print the foreground window and the focused control's capabilities.
fn report() {
    match foreground_info() {
        Ok(fg) => {
            let proc = if fg.process_name.is_empty() {
                "<unknown>".to_string()
            } else {
                fg.process_name.clone()
            };
            println!(
                "foreground : {} ({}, pid={})",
                show(&fg.title),
                proc,
                fg.pid
            );
        }
        Err(e) => println!("foreground : <unavailable: {e}>"),
    }

    match probe_focused() {
        Ok(p) => {
            println!("focused    : {} / {}", show(&p.name), p.control_type);
            println!(
                "caps       : Text={} Value={}  read={:?} write={:?}",
                p.capability.has_text_pattern,
                p.capability.has_value_pattern,
                p.capability.recommended_read,
                p.capability.recommended_write
            );
            let sel = &p.selection;
            let chars = sel.text.chars().count();
            println!(
                "selection  : [{:?}] {} chars{}: {}",
                sel.strategy,
                chars,
                if sel.truncated { " (truncated)" } else { "" },
                preview(&sel.text)
            );
        }
        Err(e) => println!("focused    : <unavailable: {e}>"),
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

fn show(s: &str) -> String {
    if s.is_empty() {
        "<empty>".to_string()
    } else {
        s.to_string()
    }
}
