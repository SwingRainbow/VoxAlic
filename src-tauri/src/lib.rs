mod api;
mod capture;
mod config;
mod item_i18n;
mod mission_timer;
mod models;
mod ocr;
mod state;
mod window;

use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::time::Duration;
use state::{AppState, SharedState};
use models::AppStatePayload;
use config::{AppConfig, FissureAlert, CycleAlert, ArbitrationAlert, load_config, save_config};
use api::{fetch_worldstate, parse_fissures, parse_cycles, parse_void_trader, parse_bounties, parse_circuit, parse_arbitration, parse_vallis_cycle, parse_duviri_cycle, fmt_remain, fmt_remain_baro, fmt_remain_days, now_ms};
use std::sync::mpsc;
use mission_timer::{AlertMsg, MissionTimerState, TimerCommand, start_timer_thread};
use tauri_plugin_notification::NotificationExt;
use ocr::DigitTemplates;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Emitter,
};

const REFRESH_SEC: u32 = 1800;

// ── Subscription alert helpers ──────────────────────────────────────────────

/// Check fissure subscriptions after each worldstate refresh. Sends a toast
/// for each new matching fissure (keyed by node+expiry so re-checks don't
/// re-notify). Cleans up expired keys so the set stays small.
fn check_fissure_alerts(
    fissures: &[models::Fissure],
    alerts: &[FissureAlert],
    notified: &mut std::collections::HashSet<String>,
    tx: &mpsc::Sender<AlertMsg>,
) {
    if alerts.is_empty() { return; }
    for f in fissures {
        if f.remain_ms <= 0 { continue; }
        let slot_diff = if f.is_hard { "hard" } else if f.is_storm { "storm" } else { "normal" };
        for alert in alerts {
            let tier_ok = alert.tier.is_empty() || alert.tier == f.tier_label;
            let type_ok = alert.mission_type.is_empty() || alert.mission_type == f.mission_type;
            let diff_ok = alert.difficulty.is_empty() || alert.difficulty == slot_diff;
            if !tier_ok || !type_ok || !diff_ok { continue; }
            let key = format!("{}:{}", f.node_key, f.expiry_ms);
            if notified.insert(key) {
                let kind = if f.is_hard { " [钢铁]" } else if f.is_storm { " [风暴]" } else { "" };
                let _ = tx.send(AlertMsg {
                    title: format!("裂缝出现 · {}{}{}", f.tier_label, f.mission_type, kind),
                    body: format!("{} · {} · 剩余 {}", f.node_name, f.planet, f.remain_str),
                });
            }
        }
    }
    // Remove keys whose fissures have already expired.
    let active: std::collections::HashSet<String> = fissures.iter()
        .filter(|f| f.remain_ms > 0)
        .map(|f| format!("{}:{}", f.node_key, f.expiry_ms))
        .collect();
    notified.retain(|k| active.contains(k));
}

/// Check arbitration subscriptions. Fires when the arbitration node changes
/// (tracked by node+expiry key) and the new slot matches the rule.
fn check_arbitration_alerts(
    arb: Option<&models::ArbitrationInfo>,
    alerts: &[ArbitrationAlert],
    prev_key: &mut String,
    tx: &mpsc::Sender<AlertMsg>,
) {
    if alerts.is_empty() { return; }
    let Some(arb) = arb else { return };
    let key = format!("{}:{}", arb.current.node, arb.expiry_ms);
    if key == *prev_key { return; }
    *prev_key = key;
    for alert in alerts {
        let m_ok = alert.mission_type.is_empty() || alert.mission_type == arb.current.mission;
        let n_ok = alert.planet.is_empty() || alert.planet == arb.current.planet;
        if m_ok && n_ok {
            let _ = tx.send(AlertMsg {
                title: format!("仲裁 · {}", arb.current.mission),
                body: format!("{} · {} · Lv {}-{} · 剩余 {}",
                    arb.current.node, arb.current.planet,
                    arb.current.min_level, arb.current.max_level,
                    arb.remain_str),
            });
        }
    }
}

/// Check cycle subscriptions after each worldstate refresh. Only fires on the
/// tick when the cycle state *transitions into* the subscribed state.
fn check_cycle_alerts(
    cycles: &[models::CycleInfo],
    alerts: &[CycleAlert],
    prev_states: &mut std::collections::HashMap<String, String>,
    tx: &mpsc::Sender<AlertMsg>,
) {
    if alerts.is_empty() { return; }
    for cycle in cycles {
        let prev = prev_states.get(&cycle.name).cloned().unwrap_or_default();
        if cycle.state == prev { continue; }
        // State just changed — record and check subscriptions.
        prev_states.insert(cycle.name.clone(), cycle.state.clone());
        for alert in alerts {
            if alert.location == cycle.name && alert.state == cycle.state {
                let _ = tx.send(AlertMsg {
                    title: format!("{} · {}", cycle.name, cycle.state),
                    body: format!("剩余 {}", cycle.remain_str),
                });
            }
        }
    }
}

fn build_payload(state: &AppState, timer_state: &MissionTimerState) -> AppStatePayload {
    let mut normal = state.normal_fissures.clone();
    let mut hard = state.hard_fissures.clone();
    let mut storms = state.storm_fissures.clone();
    let mut cycles = state.cycles.clone();
    let mut baro = state.baro.clone();
    let mut bounties = state.bounties.clone();
    let mut circuit = state.circuit.clone();

    let now = now_ms();
    for f in normal.iter_mut().chain(hard.iter_mut()).chain(storms.iter_mut()) {
        let remain = f.expiry_ms - now;
        f.remain_ms = remain;
        f.remain_str = fmt_remain(remain);
        f.is_expiring = remain > 0 && remain < 300_000;
    }
    for c in &mut cycles {
        let remain = c.expiry_ms - now;
        if remain <= 0 {
            // Recompute expired cycles locally so they don't sit at "切换中"
            // until the next 30-min API poll.
            if c.name == "奥布山谷" {
                *c = parse_vallis_cycle();
            } else if c.name == "双衍王境" {
                *c = parse_duviri_cycle();
            } else if let Some(rolled) = api::roll_forward_cycle(c, now) {
                *c = rolled;
            } else {
                c.remain_ms = 0;
                c.remain_str = "切换中".to_string();
            }
        } else {
            c.remain_ms = remain;
            c.remain_str = fmt_remain(remain);
        }
    }

    // Recompute Baro's countdown every tick. When the arrival timer elapses we
    // flip to the departure countdown locally so it doesn't sit stale until the
    // next 30-min API poll refreshes the manifest.
    if let Some(b) = &mut baro {
        if !b.active && now >= b.start_ms {
            b.active = true;
        }
        let target = if b.active { b.end_ms } else { b.start_ms };
        b.remain_ms = target - now;
        b.remain_str = fmt_remain_baro(b.remain_ms);
    }

    // Refresh each bounty board's countdown every tick.
    for b in &mut bounties {
        b.remain_ms = b.expiry_ms - now;
        b.remain_str = fmt_remain(b.remain_ms);
    }

    // Refresh the Circuit's weekly countdown every tick.
    if let Some(c) = &mut circuit {
        c.remain_ms = c.expiry_ms - now;
        c.remain_str = fmt_remain_days(c.remain_ms);
    }

    // Arbitration is epoch-computed each tick — no state to carry, just recalculate.
    let arbitration = parse_arbitration(now);

    AppStatePayload {
        normal_fissures: normal,
        hard_fissures: hard,
        storm_fissures: storms,
        cycles,
        last_update: state.last_update.clone(),
        countdown_secs: state.countdown_secs,
        mission_timer: timer_state.payload.clone(),
        baro,
        bounties,
        circuit,
        arbitration,
    }
}

type SharedConfig = Arc<StdRwLock<AppConfig>>;
type MissionTimerShared = Arc<StdRwLock<MissionTimerState>>;

/// Fetch worldstate, store it into `state`, then emit `worldstate-update`.
/// Network/parse errors are returned; background loops ignore them, the
/// `refresh_now` command surfaces them to the frontend.
async fn fetch_store_emit(
    state: &SharedState,
    timer: &MissionTimerShared,
    handle: &tauri::AppHandle,
) -> Result<(), String> {
    let data = fetch_worldstate().await?;
    let (normal, hard, storms) = parse_fissures(&data);
    let cycles = parse_cycles(&data);
    let baro = parse_void_trader(&data);
    let bounties = parse_bounties(&data);
    let circuit = parse_circuit(&data);
    let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
    {
        let mut s = state.write().await;
        s.normal_fissures = normal;
        s.hard_fissures = hard;
        s.storm_fissures = storms;
        s.cycles = cycles;
        s.baro = baro;
        s.bounties = bounties;
        s.circuit = circuit;
        s.last_update = now_str;
        s.countdown_secs = REFRESH_SEC;
    }
    let payload = {
        let s = state.read().await;
        let t = timer.read().unwrap();
        build_payload(&s, &t)
    };
    let _ = handle.emit("worldstate-update", payload);
    Ok(())
}

#[tauri::command]
async fn refresh_now(state: tauri::State<'_, SharedState>, timer_state: tauri::State<'_, MissionTimerShared>, app: tauri::AppHandle) -> Result<(), String> {
    fetch_store_emit(&state, &timer_state, &app).await
}

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

#[tauri::command]
fn timer_command(
    cmd_tx: tauri::State<'_, mpsc::Sender<TimerCommand>>,
    timer_state: tauri::State<'_, MissionTimerShared>,
    command: String,
    mode: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    match command.as_str() {
        "set_mode" => {
            let m = mode.unwrap_or_else(|| "normal".into());
            timer_state.write().unwrap().handle_command(&TimerCommand::SetMode(m.clone()));
            let now = chrono::Local::now().format("%H:%M:%S").to_string();
            let _ = app.emit("timer-log", format!("[{}] 模式切换: {}", now, m));
            Ok(())
        }
        _ => {
            let cmd = match command.as_str() {
                "start" => TimerCommand::Start,
                "stop" => TimerCommand::Stop,
                "reset" => TimerCommand::Reset,
                _ => return Err(format!("Unknown command: {}", command)),
            };
            cmd_tx.send(cmd).map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
fn list_windows(config: tauri::State<'_, SharedConfig>) -> Vec<window::WindowInfo> {
    let cfg = config.read().unwrap();
    window::list_windows(&cfg.mission_timer.window_title)
}

#[tauri::command]
fn select_window(config: tauri::State<'_, SharedConfig>, hwnd: usize, app: tauri::AppHandle) -> Result<(), String> {
    let mut cfg = config.write().unwrap();
    cfg.mission_timer.selected_hwnd = hwnd;
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::config::save_config(&app_data_dir, &cfg)
}

#[tauri::command]
fn single_capture(cmd_tx: tauri::State<'_, mpsc::Sender<TimerCommand>>) -> Result<(), String> {
    cmd_tx.send(TimerCommand::SingleCapture).map_err(|e| e.to_string())
}

/// Resolve the game window the same way the OCR thread does: first visible
/// window whose title matches the configured keyword.
fn resolve_hwnd(cfg: &AppConfig) -> isize {
    window::list_windows(&cfg.mission_timer.window_title)
        .first()
        .map(|w| w.hwnd as isize)
        .unwrap_or(0)
}

/// Capture the current game frame for the ROI calibration overlay. Returns a
/// `data:image/png;base64,...` URL. The frame is processed identically to the
/// OCR pipeline (same `strip_frame`), so drawn ROI fractions map 1:1.
#[tauri::command]
fn capture_preview(config: tauri::State<'_, SharedConfig>) -> Result<String, String> {
    let cfg = config.read().unwrap();
    let hwnd = resolve_hwnd(&cfg);
    if hwnd == 0 || !window::is_valid(hwnd) {
        return Err("未检测到游戏窗口".into());
    }
    let strip_frame = cfg.mission_timer.strip_frame;
    drop(cfg);
    capture::capture_preview_data_url(hwnd, strip_frame)
        .ok_or_else(|| "截图失败（窗口最小化或画面为黑帧）".into())
}

/// Run OCR once on an explicit fractional ROI (the box the user just drew),
/// without modifying timer state or saving config. Lets the user verify a box
/// reads the right digits before committing it.
#[tauri::command]
fn test_recognize(
    config: tauri::State<'_, SharedConfig>,
    templates: tauri::State<'_, Arc<DigitTemplates>>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<String, String> {
    let cfg = config.read().unwrap();
    let hwnd = resolve_hwnd(&cfg);
    if hwnd == 0 || !window::is_valid(hwnd) {
        return Err("未检测到游戏窗口".into());
    }
    let strip_frame = cfg.mission_timer.strip_frame;
    drop(cfg);

    let roi = capture::ROIConfig { x, y, w, h };
    let (pixels, cw, ch) = capture::capture_roi_stripped(hwnd, &roi, strip_frame)
        .ok_or_else(|| "截图失败".to_string())?;
    let tmpl: &DigitTemplates = templates.inner();
    Ok(ocr::recognize_digits(&pixels, cw, ch, tmpl, mission_timer::MATCH_THRESHOLD)
        .unwrap_or_else(|| "无结果".into()))
}

// ── Autostart (Windows registry HKCU\...\Run) ───────────────────────────────

const AUTOSTART_SUBKEY: windows::core::PCWSTR = windows::core::w!(
    r"Software\Microsoft\Windows\CurrentVersion\Run"
);
const AUTOSTART_VALUE: windows::core::PCWSTR = windows::core::w!("VoxAlic");

#[tauri::command]
fn get_autostart() -> bool {
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, AUTOSTART_SUBKEY, 0, KEY_READ, &mut hkey).is_err() {
            return false;
        }
        let exists = RegQueryValueExW(hkey, AUTOSTART_VALUE, None, None, None, None).is_ok();
        let _ = RegCloseKey(hkey);
        exists
    }
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut hkey = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, AUTOSTART_SUBKEY, 0, KEY_SET_VALUE, &mut hkey)
            .ok().map_err(|e| e.to_string())?;
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let mut wide: Vec<u16> = exe.to_string_lossy().encode_utf16().collect();
            wide.push(0);
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(hkey, AUTOSTART_VALUE, 0, REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(hkey, AUTOSTART_VALUE)
        };
        let _ = RegCloseKey(hkey);
        result.ok().map_err(|e| e.to_string())
    }
}

// ── Clean uninstall ──────────────────────────────────────────────────────────

/// Remove user data, autostart entry, then launch the system uninstaller and exit.
/// The system uninstaller (registered by NSIS/MSI) removes the exe and shortcuts.
#[tauri::command]
async fn uninstall_clean(app: tauri::AppHandle) -> Result<(), String> {
    // 1. Delete app data directory (config.json, baro_zh.json, etc.)
    if let Ok(data_dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    // 2. Remove autostart registry entry
    unsafe {
        use windows::Win32::System::Registry::*;
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, AUTOSTART_SUBKEY, 0, KEY_SET_VALUE, &mut hkey)
            .ok().is_ok()
        {
            let _ = RegDeleteValueW(hkey, AUTOSTART_VALUE).ok();
            let _ = RegCloseKey(hkey);
        }
    }

    // 3. Look up UninstallString from the system registry and launch it.
    //    NSIS registers under HKCU\...\Uninstall\{productName}.
    let uninstall_cmd = unsafe {
        use windows::Win32::System::Registry::*;
        let subkey = windows::core::w!(
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\VoxAlic"
        );
        let value_name = windows::core::w!("UninstallString");
        let mut hkey = HKEY::default();
        let mut result: Option<String> = None;
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_READ, &mut hkey).ok().is_ok() {
            let mut buf = vec![0u16; 1024];
            let mut size = (buf.len() * 2) as u32;
            if RegQueryValueExW(hkey, value_name, None, None,
                Some(buf.as_mut_ptr() as *mut u8), Some(&mut size)).ok().is_ok()
            {
                let len = (size / 2) as usize;
                result = Some(String::from_utf16_lossy(&buf[..len.saturating_sub(1)]));
            }
            let _ = RegCloseKey(hkey);
        }
        result
    };

    if let Some(cmd) = uninstall_cmd {
        // UninstallString from NSIS is a quoted path like `"C:\path\uninstall.exe"`.
        // Strip the surrounding quotes — Command::new calls CreateProcess directly
        // so spaces in the path are fine without quoting.
        let exe = cmd.trim().trim_matches('"');
        std::process::Command::new(exe)
            .spawn()
            .map_err(|e| format!("无法启动卸载程序 [{exe}]: {e}"))?;
    } else {
        return Err("未找到卸载程序，请通过「设置 → 应用」手动卸载".into());
    }

    // 4. Exit this app so the uninstaller can remove the exe
    app.exit(0);
    Ok(())
}

/// Show a Windows toast notification. Safe to call from any thread that owns an
/// `AppHandle`; failures (e.g. uninstalled dev build with no AppUserModelID) are
/// swallowed.
fn show_toast(app: &tauri::AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Fire a sample reminder so the user can confirm their chosen alert method
/// works. Honors the saved `alert_method`: "toast" shows a notification,
/// otherwise the game window is forced to the foreground.
#[tauri::command]
fn test_alert(app: tauri::AppHandle, config: tauri::State<'_, SharedConfig>) -> Result<(), String> {
    let (method, hwnd, sample) = {
        let cfg = config.read().unwrap();
        // Preview the configured checkpoint wording with a sample milestone.
        let tpl = if cfg.mission_timer.checkpoint_alert_text.trim().is_empty() {
            config::default_checkpoint_text()
        } else {
            cfg.mission_timer.checkpoint_alert_text.clone()
        };
        (
            cfg.mission_timer.alert_method.clone(),
            resolve_hwnd(&cfg),
            tpl.replace("{min}", "5"),
        )
    };
    if method == "toast" {
        show_toast(&app, "Warframe 计时器", &sample);
        Ok(())
    } else {
        if hwnd == 0 || !window::is_valid(hwnd) {
            return Err("未检测到游戏窗口，无法测试强制弹窗".into());
        }
        window::bring_to_front(hwnd);
        Ok(())
    }
}

/// Refresh the item-name 中文 table from WFCD/warframe-items. Downloads ~51 MB,
/// extracts the 简中 names, persists a compact map to the app data dir, and
/// hot-swaps it in. Returns the number of translated entries.
#[tauri::command]
async fn update_item_names(app: tauri::AppHandle) -> Result<usize, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    item_i18n::update_from_remote(dir).await
}

/// Number of item-name translations currently loaded (embedded or updated).
#[tauri::command]
fn item_names_count() -> usize {
    item_i18n::count()
}

// ── In-app updater ───────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct UpdateInfo {
    version: String,
    notes: String,
}

#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo {
            version: u.version.clone(),
            notes: u.body.clone().unwrap_or_default(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
        app.exit(0);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let state: SharedState = SharedState::default();

            // Load config
            let app_data_dir = app.path().app_data_dir().expect("app data dir");
            let config: SharedConfig = Arc::new(StdRwLock::new(load_config(&app_data_dir)));

            // Load item-name 中文 table (user override file, else embedded default).
            item_i18n::init(&app_data_dir);

            // Load digit templates for OCR
            let templates_arc = Arc::new(DigitTemplates::load());
            // Shared with the timer thread AND the test_recognize command.
            let templates_for_cmd = templates_arc.clone();

            // Mission timer shared state + command channel
            let timer_state: MissionTimerShared = Arc::new(StdRwLock::new(MissionTimerState::new()));
            let timer_config = config.clone();
            let timer_shared = timer_state.clone();
            let (log_tx, log_rx) = mpsc::channel::<String>();
            // Alert channel: the OCR thread has no AppHandle, so toast requests
            // are forwarded here for delivery. Cloned so the worldstate loop can
            // also send subscription alerts through the same channel.
            let (alert_tx, alert_rx) = mpsc::channel::<AlertMsg>();
            let ws_alert_tx = alert_tx.clone();
            let cmd_tx = start_timer_thread(timer_shared, timer_config, templates_arc, log_tx, alert_tx);

            // Log forwarding thread
            let log_handle = app.handle().clone();
            std::thread::spawn(move || {
                while let Ok(msg) = log_rx.recv() {
                    let _ = log_handle.emit("timer-log", msg);
                }
            });

            // Alert (toast) forwarding thread
            let alert_handle = app.handle().clone();
            std::thread::spawn(move || {
                while let Ok(msg) = alert_rx.recv() {
                    show_toast(&alert_handle, &msg.title, &msg.body);
                }
            });

            // Background fetch loop (every REFRESH_SEC)
            let fetch_state = state.clone();
            let fetch_timer = timer_state.clone();
            let fetch_handle = app.handle().clone();
            let fetch_config = config.clone();
            tauri::async_runtime::spawn(async move {
                let mut notified_fissures: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut prev_cycle_states: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                let mut prev_arb_key = String::new();
                // `interval`'s first tick fires immediately → fetch once at
                // startup, then every REFRESH_SEC.
                let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_SEC as u64));
                loop {
                    interval.tick().await;
                    if fetch_store_emit(&fetch_state, &fetch_timer, &fetch_handle).await.is_ok() {
                        let now = now_ms();
                        let (all_fissures, cycles, fissure_alerts, cycle_alerts, arb_alerts) = {
                            let s = fetch_state.read().await;
                            let cfg = fetch_config.read().unwrap();
                            let all: Vec<_> = s.normal_fissures.iter()
                                .chain(&s.hard_fissures)
                                .chain(&s.storm_fissures)
                                .cloned()
                                .collect();
                            (all, s.cycles.clone(), cfg.fissure_alerts.clone(), cfg.cycle_alerts.clone(), cfg.arbitration_alerts.clone())
                        };
                        check_fissure_alerts(&all_fissures, &fissure_alerts, &mut notified_fissures, &ws_alert_tx);
                        check_cycle_alerts(&cycles, &cycle_alerts, &mut prev_cycle_states, &ws_alert_tx);
                        let arb = parse_arbitration(now);
                        check_arbitration_alerts(arb.as_ref(), &arb_alerts, &mut prev_arb_key, &ws_alert_tx);
                    }
                }
            });

            // Per-second tick
            let tick_state = state.clone();
            let tick_timer = timer_state.clone();
            let tick_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;
                    let payload = {
                        let mut s = tick_state.write().await;
                        s.countdown_secs = s.countdown_secs.saturating_sub(1);
                        {
                            let mut t = tick_timer.write().unwrap();
                            t.update_elapsed();
                        }
                        let t = tick_timer.read().unwrap();
                        build_payload(&s, &t)
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
            let tray_timer = timer_state.clone();
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
                        let timer = tray_timer.clone();
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = fetch_store_emit(&state, &timer, &handle).await;
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
            app.manage(timer_state);
            app.manage(cmd_tx);
            app.manage(templates_for_cmd);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![refresh_now, get_config, set_config, timer_command, list_windows, select_window, single_capture, capture_preview, test_recognize, test_alert, update_item_names, item_names_count, get_autostart, set_autostart, uninstall_clean, check_for_update, install_update])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
