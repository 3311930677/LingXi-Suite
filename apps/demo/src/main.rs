//! End-to-end console demonstration.
//!
//! Ctrl+Alt+Space captures the current selection, applies the selected
//! transformer, prints a diff preview, then validates the target, writes back
//! and verifies it. Ctrl+Alt+Backspace safely undoes the last write.
//!
//! Usage: `cargo run -p demo -- [prefix|tidy|upper]` (default: prefix).

use std::time::Duration;

use assistant_core::{
    diff::render_inline, transform_selection_with, InputAdapter, Transformer, WriteReceipt,
};
use assistant_windows::{
    run_assistant_hotkey_loop, wait_for_trigger_release, AssistantHotkey, WindowsAdapter,
};

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "prefix".to_string());
    let Some(transformer) = assistant_core::transformer_by_name(&mode) else {
        eprintln!("unknown mode '{mode}'. Use one of: prefix, tidy, upper");
        std::process::exit(2);
    };

    println!("cross-app-assistant :: end-to-end demo");
    println!("transformer     : {}", transformer.name());
    println!("Ctrl+Alt+Space  capture -> transform -> diff -> safe write -> verify");
    println!("Ctrl+Alt+Backspace  undo the last successful write");
    println!("Close this window (or Ctrl+C) to stop.\n");

    let adapter = WindowsAdapter::new();
    let mut last_receipt: Option<WriteReceipt> = None;
    let result = run_assistant_hotkey_loop(|command| {
        // WM_HOTKEY may fire before Ctrl/Alt are physically released.
        wait_for_trigger_release(Duration::from_millis(800));
        match command {
            AssistantHotkey::Transform => {
                transform(&adapter, transformer.as_ref(), &mut last_receipt)
            }
            AssistantHotkey::Undo => undo(&adapter, &mut last_receipt),
        }
    });

    if let Err(error) = result {
        eprintln!("demo failed: {error}");
        std::process::exit(1);
    }
}

fn transform(
    adapter: &WindowsAdapter,
    transformer: &dyn Transformer,
    last_receipt: &mut Option<WriteReceipt>,
) {
    println!("---- transform ----");
    match transform_selection_with(adapter, transformer) {
        Ok(outcome) => {
            let stats = outcome.stats();
            println!("diff  : {}", render_inline(&outcome.diff));
            println!("change: +{} -{} chars", stats.inserted, stats.deleted);
            println!(
                "write : {} chars via {:?}; verified={}",
                outcome.receipt.wrote_len, outcome.receipt.strategy_used, outcome.receipt.verified
            );
            *last_receipt = Some(outcome.receipt);
        }
        Err(error) => println!("capture/write rejected: {error}"),
    }
    println!();
}

fn undo(adapter: &WindowsAdapter, last_receipt: &mut Option<WriteReceipt>) {
    println!("---- undo ----");
    let Some(receipt) = last_receipt.as_ref() else {
        println!("nothing to undo\n");
        return;
    };
    match adapter.undo(receipt) {
        Ok(result) => {
            println!(
                "restored {} chars; verified={}",
                result.restored_len, result.verified
            );
            *last_receipt = None;
        }
        Err(error) => println!("undo rejected/failed: {error}"),
    }
    println!();
}
