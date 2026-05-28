mod api;
mod capture;
mod config;
mod mission_timer;
mod models;
mod ocr;
mod state;

use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::time::Duration;
use state::{AppState, SharedState};
use models::{AppStatePayload, MissionTimerPayload};
use config::{AppConfig, load_config, save_config};
use api::{fetch_worldstate, parse_fissures, parse_cycles, fmt_remain, now_ms};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Emitter,
};

const REFRESH_SEC: u32 = 1800;

fn build_payload(state: &AppState) -> AppStatePayload {
    let mut normal = state.normal_fissures.clone();
    let mut hard = state.hard_fissures.clone();
    let mut storms = state.storm_fissures.clone();
    let mut cycles = state.cycles.clone();

    let now = now_ms();
    for f in normal.iter_mut().chain(hard.iter_mut()).chain(storms.iter_mut()) {
        let remain = f.expiry_ms - now;
        f.remain_ms = remain;
        f.remain_str = fmt_remain(remain);
        f.is_expiring = remain > 0 && remain < 300_000;
    }
    for c in &mut cycles {
        let remain = c.expiry_ms - now;
        c.remain_ms = remain;
        c.remain_str = fmt_remain(remain);
    }

    AppStatePayload {
        normal_fissures: normal,
        hard_fissures: hard,
        storm_fissures: storms,
        cycles,
        last_update: state.last_update.clone(),
        countdown_secs: state.countdown_secs,
        mission_timer: MissionTimerPayload::default(),
    }
}

#[tauri::command]
async fn refresh_now(state: tauri::State<'_, SharedState>, app: tauri::AppHandle) -> Result<(), String> {
    let data = fetch_worldstate().await?;
    let (normal, hard, storms) = parse_fissures(&data);
    let cycles = parse_cycles(&data);
    let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
    {
        let mut s = state.write().await;
        s.normal_fissures = normal;
        s.hard_fissures = hard;
        s.storm_fissures = storms;
        s.cycles = cycles;
        s.last_update = now_str;
        s.countdown_secs = REFRESH_SEC;
    }
    let payload = { let s = state.read().await; build_payload(&s) };
    let _ = app.emit("worldstate-update", payload);
    Ok(())
}

type SharedConfig = Arc<StdRwLock<AppConfig>>;

#[tauri::command]
fn get_config(cfg: tauri::State<'_, SharedConfig>) -> AppConfig {
    cfg.read().unwrap().clone()
}

#[tauri::command]
fn set_config(cfg: tauri::State<'_, SharedConfig>, config: AppConfig, app: tauri::AppHandle) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    save_config(&app_data_dir, &config)?;
    *cfg.write().unwrap() = config;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state: SharedState = SharedState::default();

            // Load config
            let app_data_dir = app.path().app_data_dir().expect("app data dir");
            let config: SharedConfig = Arc::new(StdRwLock::new(load_config(&app_data_dir)));

            // Background fetch loop (every REFRESH_SEC)
            let fetch_state = state.clone();
            let fetch_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Do initial fetch immediately
                {
                    if let Ok(data) = fetch_worldstate().await {
                        let (normal, hard, storms) = parse_fissures(&data);
                        let cycles = parse_cycles(&data);
                        let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                        {
                            let mut s = fetch_state.write().await;
                            s.normal_fissures = normal;
                            s.hard_fissures = hard;
                            s.storm_fissures = storms;
                            s.cycles = cycles;
                            s.last_update = now_str;
                            s.countdown_secs = REFRESH_SEC;
                        }
                    }
                    let payload = { let s = fetch_state.read().await; build_payload(&s) };
                    let _ = fetch_handle.emit("worldstate-update", payload);
                }
                // Then loop
                let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_SEC as u64));
                loop {
                    interval.tick().await;
                    if let Ok(data) = fetch_worldstate().await {
                        let (normal, hard, storms) = parse_fissures(&data);
                        let cycles = parse_cycles(&data);
                        let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                        {
                            let mut s = fetch_state.write().await;
                            s.normal_fissures = normal;
                            s.hard_fissures = hard;
                            s.storm_fissures = storms;
                            s.cycles = cycles;
                            s.last_update = now_str;
                            s.countdown_secs = REFRESH_SEC;
                        }
                    }
                    let payload = { let s = fetch_state.read().await; build_payload(&s) };
                    let _ = fetch_handle.emit("worldstate-update", payload);
                }
            });

            // Per-second tick
            let tick_state = state.clone();
            let tick_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;
                    let payload = {
                        let mut s = tick_state.write().await;
                        s.countdown_secs = s.countdown_secs.saturating_sub(1);
                        build_payload(&s)
                    };
                    let _ = tick_handle.emit("tick-update", payload);
                }
            });

            // System tray
            let show_item = MenuItemBuilder::with_id("show", "显示").build(app)?;
            let refresh_item = MenuItemBuilder::with_id("refresh", "立即刷新").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&refresh_item)
                .item(&quit_item)
                .build()?;

            let tray_state = state.clone();
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "refresh" => {
                        let state = tray_state.clone();
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Ok(data) = fetch_worldstate().await {
                                let (normal, hard, storms) = parse_fissures(&data);
                                let cycles = parse_cycles(&data);
                                let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                                {
                                    let mut s = state.write().await;
                                    s.normal_fissures = normal;
                                    s.hard_fissures = hard;
                                    s.storm_fissures = storms;
                                    s.cycles = cycles;
                                    s.last_update = now_str;
                                    s.countdown_secs = REFRESH_SEC;
                                }
                                let payload = { let s = state.read().await; build_payload(&s) };
                                let _ = handle.emit("worldstate-update", payload);
                            }
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up, ..
                    } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Window close -> hide to tray or exit based on config
            let close_config = config.clone();
            if let Some(window) = app.get_webview_window("main") {
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let cfg = close_config.read().unwrap();
                        if cfg.close_to_tray {
                            api.prevent_close();
                            let _ = window_clone.hide();
                        }
                    }
                });
            }

            app.manage(state);
            app.manage(config);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![refresh_now, get_config, set_config])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
