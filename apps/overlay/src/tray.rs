//! 托盘图标与菜单：桌宠显隐、打开工作台、退出。
//!
//! 桌宠定位为「agent 的显示工具板块」——托盘只保留与该定位相关的最小入口。

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::pet;
use crate::state::{AppState, MutexExt};

pub(crate) fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_pet = MenuItem::with_id(app, "tray:show-pet", "显示桌宠", true, None::<&str>)?;
    let hide_pet = MenuItem::with_id(app, "tray:hide-pet", "隐藏桌宠", true, None::<&str>)?;
    let workbench = MenuItem::with_id(app, "tray:workbench", "打开工作台", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray:quit", "退出灵犀", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&show_pet, &hide_pet, &separator, &workbench, &separator, &quit],
    )?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;
    let _tray = TrayIconBuilder::with_id("lingxi-tray")
        .icon(icon)
        .tooltip("灵犀 · 引擎进度桌宠")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray:show-pet" => {
                let state = app.state::<AppState>();
                let _ = pet::set_pet_visible_inner(app, &state, true);
            }
            "tray:hide-pet" => {
                let state = app.state::<AppState>();
                let _ = pet::set_pet_visible_inner(app, &state, false);
            }
            "tray:workbench" => {
                // 与桌宠单击菜单同一入口：打开 OwO Agent 工作台。
                let port = app
                    .state::<AppState>()
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
        .build(app)?;
    Ok(())
}
