//! 托盘图标与菜单：面板显示/隐藏、打开工作台、退出。
//!
//! 桌宠定位为「引擎进度控制台」，与工作台重复的入口（小工具子菜单等）
//! 已移除；工作台通过菜单或桌宠单击打开。

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::state::MutexExt;

pub(crate) fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    eprintln!("[lingxi] install_tray: creating menu...");
    let show_panel = MenuItem::with_id(app, "tray:show", "显示面板", true, None::<&str>)?;
    let hide_panel = MenuItem::with_id(app, "tray:hide", "隐藏面板", true, None::<&str>)?;
    let workbench = MenuItem::with_id(app, "tray:workbench", "打开工作台", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray:quit", "退出灵犀", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&show_panel, &hide_panel, &separator, &workbench, &separator, &quit],
    )?;
    eprintln!("[lingxi] install_tray: menu created, getting icon...");
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;
    eprintln!("[lingxi] install_tray: icon ok, building tray...");
    let _tray = TrayIconBuilder::with_id("lingxi-tray")
        .icon(icon)
        .tooltip("灵犀 · L3 跨应用 AI 助手")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray:show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                }
            }
            "tray:hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "tray:workbench" => {
                // A8-2：打开 OwO Agent 工作台（与桌宠单击/右键菜单同一入口）。
                let port = app
                    .state::<crate::state::AppState>()
                    .backend
                    .safe_lock()
                    .owo_agent_port;
                let url = format!("http://127.0.0.1:{port}/");
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &url])
                    .spawn();
            }
            "tray:quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                    }
                }
            }
        })
        .build(app)?;
    eprintln!("[lingxi] install_tray: tray built successfully");
    Ok(())
}
