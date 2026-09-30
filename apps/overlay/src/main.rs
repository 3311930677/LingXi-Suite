//! LingXi overlay：桌宠形态的 agent 显示工具板块。
//!
//! 职责收敛（主面板/改写/QQ/小工具/市场/本地推理已退役，交互归工作台）：
//! - 桌宠窗口加载引擎服务的 `/pet/index.html`（免构建迭代：前端改动
//!   只碰磁盘文件，刷新即生效）；引擎离线时回落本地 boot 引导页；
//! - Rust 侧只保留原生桥：窗口拖动/显隐、引擎托管（Job Object）、
//!   审批回传与任务中止、进度快照。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod owo_bridge;
mod panel;
mod pet;
mod pet_skin;
mod placement;
mod settings;
mod state;
mod tray;
mod window_state;

use state::{AppState, MutexExt};

use tauri::{Manager, WindowEvent};

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            // 桌宠：状态仲裁 / 皮肤 / 可见性。
            pet::pet_status,
            pet::set_pet_status,
            pet::list_pet_skins,
            pet::current_pet_config,
            pet::set_pet_skin,
            pet::set_pet_options,
            pet::set_pet_visible,
            // 原生窗口行为。
            panel::move_pet_by,
            panel::quit_app,
            // 引擎桥接：托管 / 探测 / 选项 / 进度 / 审批 / 中止。
            owo_bridge::pick_engine_exe,
            owo_bridge::owo_status,
            owo_bridge::owo_start_service,
            owo_bridge::owo_stop_service,
            owo_bridge::owo_restart_service,
            owo_bridge::get_engine_options,
            owo_bridge::save_engine_options,
            owo_bridge::owo_activity,
            owo_bridge::owo_pet_sync,
            owo_bridge::open_workbench,
            owo_bridge::owo_permission,
            owo_bridge::owo_abort,
            owo_bridge::owo_abort_current
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Moved(pos) = event {
                if window.label() == "pet" {
                    placement::handle_window_moved(&window.app_handle().clone(), "pet", *pos);
                }
            }
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let (pet_visible, saved) = {
                let settings = state.backend.safe_lock();
                (settings.pet_visible, state.window_state.safe_lock().clone())
            };
            if let Some(window) = app.get_webview_window("pet") {
                panel::make_non_activating(&window).map_err(std::io::Error::other)?;
                let size = window.outer_size().unwrap_or(tauri::PhysicalSize::new(220, 260));
                let restored = saved.pet.filter(|pos| {
                    placement::position_on_screen(
                        pos.x,
                        pos.y,
                        size.width as i32,
                        size.height as i32,
                    )
                });
                match restored {
                    Some(pos) => {
                        placement::set_position_tracked(app.handle(), &window, pos.x, pos.y)
                    }
                    None => placement::position_pet(app.handle(), &window),
                }
                // 配置里 visible:false 避免先在默认位置闪现再跳到恢复位置；
                // 摆好之后按设置决定是否显示。
                if pet_visible {
                    let _ = window.show();
                }
            }
            // A8-3：桌宠显隐与工作台开关的常驻同步（隐藏态也可靠上报心跳）；
            // 同时承担桌宠页面导航仲裁（boot 引导页 ↔ 引擎侧 /pet 页面）。
            owo_bridge::spawn_pet_sync_worker(app.handle().clone());
            tray::install_tray(app.handle())?;
            // owo-agent 引擎自动启动（仅设置开启时；失败只记日志，不阻塞桌宠启动）。
            if app.state::<AppState>().backend.safe_lock().owo_agent_auto_start {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let state = handle.state::<AppState>();
                    let settings = state.backend.safe_lock().clone();
                    match owo_bridge::spawn_managed_service(&state, &settings) {
                        Ok(()) => eprintln!("[lingxi] owo-agent 引擎已托管启动"),
                        Err(error) => eprintln!("[lingxi] owo-agent 自动启动失败：{error}"),
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to launch LingXi overlay");
}
