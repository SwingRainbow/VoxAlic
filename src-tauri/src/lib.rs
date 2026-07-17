mod api;
mod capture;
mod config;
mod item_i18n;
mod log_init;
mod market;
mod market_auth;
mod market_orders;
mod mission_timer;
mod models;
mod ocr;
mod phone_push;
mod state;
mod window;

use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use log::{warn, error};
use std::time::Duration;
use state::{AppState, SharedState};
use models::{AppStatePayload, MissionTimerPayload};
use config::{AppConfig, FissureAlert, CycleAlert, ArbitrationAlert, load_config, save_config};
use api::{fetch_worldstate, parse_fissures, parse_cycles, parse_void_trader, parse_bounties, parse_circuit, parse_arbitration, parse_vallis_cycle, parse_duviri_cycle, fmt_remain, fmt_remain_baro, fmt_remain_days, now_ms};
use market::{search_market_items, get_market_item, refresh_market_cache, market_cache_status, translate_items};
use market_auth::{market_signin, market_signout, market_auth_status, market_set_status, SharedMarketAuth, load_or_create_auth};
use market_orders::{market_list_orders, market_create_order, market_update_order, market_delete_order, market_close_order};
use std::sync::mpsc;
use mission_timer::{AlertMsg, MissionTimerState, TimerCommand, start_timer_thread};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use ocr::DigitTemplates;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Emitter,
};

const REFRESH_SEC: u32 = 300;

// ── SMTP feedback credentials (local-only, never committed) ────────────────────
include!("smtp_creds.rs");

// ── Subscription notifications ───────────────────────────────────────────────

/// A fired subscription, surfaced via the tray red-dot + click popup (NOT a
/// Windows toast — toasts are reserved for the mission timer). Serialized to the
/// popup webview ("notify" window).
#[derive(Clone, serde::Serialize)]
pub struct SubNotify {
    pub kind: String,    // "fissure" | "cycle" | "arbitration"
    pub icon: String,    // emoji shown in the popup
    pub title: String,
    pub detail: String,
    pub ts: i64,         // fired-at (ms) for relative time in the popup
    pub node: String,    // locate key for click-through (fissure/arb node, cycle location)
    pub sub: String,     // fissure sub-tab hint: "normal"|"hard"|"storm" (else "")
}

/// Sent to the main window when a popup item is clicked, to navigate the UI to
/// the matching entry (e.g. the fissure tab + sub-tab + highlighted row).
#[derive(Clone, serde::Serialize)]
struct NavigateMsg {
    kind: String,
    node: String,
    sub: String,
}

/// Check fissure subscriptions after each worldstate refresh. Emits a
/// notification for each new matching fissure (keyed by node+expiry so re-checks
/// don't re-notify). Cleans up expired keys so the set stays small.
fn check_fissure_alerts(
    fissures: &[models::Fissure],
    alerts: &[FissureAlert],
    notified: &mut std::collections::HashSet<String>,
    tx: &mpsc::Sender<SubNotify>,
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
                // Title format: 难度 纪元 任务模式 (no leading icon). 难度 leads.
                let difficulty = if f.is_hard { "钢铁" } else if f.is_storm { "风暴" } else { "普通" };
                let sub = if f.is_hard { "hard" } else if f.is_storm { "storm" } else { "normal" };
                let _ = tx.send(SubNotify {
                    kind: "fissure".into(),
                    icon: "".into(),
                    title: format!("{} {} {}", difficulty, f.tier_label, f.mission_type),
                    detail: format!("{} · {} · 剩余 {}", f.node_name, f.planet, f.remain_str),
                    ts: now_ms(),
                    node: f.node_name.clone(),
                    sub: sub.into(),
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
    tx: &mpsc::Sender<SubNotify>,
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
            let _ = tx.send(SubNotify {
                kind: "arbitration".into(),
                icon: "⚔".into(),
                title: format!("仲裁 · {}", arb.current.mission),
                detail: format!("{} · {} · Lv {}-{} · 剩余 {}",
                    arb.current.node, arb.current.planet,
                    arb.current.min_level, arb.current.max_level,
                    arb.remain_str),
                ts: now_ms(),
                node: arb.current.node.clone(),
                sub: "".into(),
            });
        }
    }
}

/// Check cycle advance-notice thresholds on every tick (per-second). Fires once
/// per phase when `remain_ms` of the current phase drops below the configured
/// advance, before the target state begins. Currently only processes 夜灵平野.
fn check_cycle_advance_alerts(
    cycles: &[models::CycleInfo],
    alerts: &[CycleAlert],
    fired: &mut std::collections::HashSet<String>,
    tx: &mpsc::Sender<SubNotify>,
) {
    for alert in alerts {
        if alert.advance_minutes == 0 { continue; }
        if alert.location != "夜灵平野" { continue; }
        let Some(cycle) = cycles.iter().find(|c| c.name == alert.location) else { continue; };
        let key = format!("{}|{}|{}", alert.location, alert.state, alert.advance_minutes);

        if cycle.state == alert.state {
            // Already in the target state — clear the flag so the next cycle's
            // advance window can fire fresh.
            fired.remove(&key);
        } else {
            // Currently in the "before" phase — check if we've crossed the threshold.
            let threshold_ms = (alert.advance_minutes as i64) * 60 * 1000;
            if cycle.remain_ms <= threshold_ms && !fired.contains(&key) {
                fired.insert(key.clone());
                let next = &alert.state;
                let mins = alert.advance_minutes;
                let _ = tx.send(SubNotify {
                    kind: "cycle".into(),
                    icon: "⏰".into(),
                    title: format!("{} · 即将进入{}", cycle.name, next),
                    detail: format!("还有 {} 分钟，请做好准备", mins),
                    ts: now_ms(),
                    node: cycle.name.clone(),
                    sub: "".into(),
                });
            }
        }
    }
}

/// Check cycle subscriptions after each worldstate refresh. Only fires on the
/// tick when the cycle state *transitions into* the subscribed state.
fn check_cycle_alerts(
    cycles: &[models::CycleInfo],
    alerts: &[CycleAlert],
    prev_states: &mut std::collections::HashMap<String, String>,
    tx: &mpsc::Sender<SubNotify>,
) {
    if alerts.is_empty() { return; }
    for cycle in cycles {
        let prev = prev_states.get(&cycle.name).cloned().unwrap_or_default();
        if cycle.state == prev { continue; }
        // State just changed — record and check subscriptions.
        prev_states.insert(cycle.name.clone(), cycle.state.clone());
        for alert in alerts {
            if alert.location == cycle.name && alert.state == cycle.state {
                let _ = tx.send(SubNotify {
                    kind: "cycle".into(),
                    icon: "🌓".into(),
                    title: format!("{} · {}", cycle.name, cycle.state),
                    detail: format!("剩余 {}", cycle.remain_str),
                    ts: now_ms(),
                    node: cycle.name.clone(),
                    sub: "".into(),
                });
            }
        }
    }
}

/// A type-tag so Tauri can tell this `Arc<AtomicBool>` apart from `flashing`
/// (both would otherwise be `Arc<AtomicBool>` → "state already managed" panic).
struct RefreshGuard(Arc<std::sync::atomic::AtomicBool>);

/// Type aliases for the managed notification state.
type NotifyList = Arc<StdRwLock<Vec<SubNotify>>>;
type FlashFlag = Arc<std::sync::atomic::AtomicBool>;
/// Shared cycle-state history for `check_cycle_alerts` — used by both the
/// fetch loop and the tick loop (gap recovery) so a sleep→wake doesn't miss
/// a state transition.
type PrevCycleStates = Arc<StdRwLock<std::collections::HashMap<String, String>>>;
/// Generation counter that owns the lifetime of the popup-watch thread. Bumped by
/// a new tray `Enter` and by every explicit hide (left-click, item click-through,
/// empty-list), so any stale watcher exits on its next poll.
type HideGen = Arc<std::sync::atomic::AtomicU64>;

/// Authoritative auto-hide for the notify popup: a thread that polls the *real*
/// cursor position (Win32 `GetCursorPos`, thread-safe) every 120ms and hides the
/// popup only after the cursor has stayed continuously outside both the popup
/// rect and a small box around the tray icon for ~360ms.
///
/// This replaces an earlier scheme based on the popup's DOM `mouseenter`/`leave`
/// plus the tray `Leave` event. That scheme was unreliable: the transparent,
/// non-focused popup frequently dropped those events, so the cancel-the-hide
/// signal never arrived and the popup vanished while the cursor was travelling to
/// it. Polling the OS cursor doesn't depend on any webview/event delivery.
///
/// `px/py/pw/ph` = popup rect (physical px), captured on the main thread at show
/// time (the popup doesn't move while open). `tray_x/tray_y` = the Enter cursor
/// position, used to keep the popup alive across the tiny icon→popup gap.
#[allow(clippy::too_many_arguments)]
fn start_popup_watch(
    app: tauri::AppHandle,
    gen: HideGen,
    tray_x: i32,
    tray_y: i32,
    px: i32,
    py: i32,
    pw: i32,
    ph: i32,
) {
    use std::sync::atomic::Ordering;
    let g = gen.load(Ordering::SeqCst);
    std::thread::spawn(move || {
        let mut misses = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(120));
            if gen.load(Ordering::SeqCst) != g {
                return; // superseded by a newer Enter, or an explicit hide
            }
            let mut pt = windows::Win32::Foundation::POINT::default();
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
            }
            let over_popup =
                pt.x >= px && pt.x <= px + pw && pt.y >= py && pt.y <= py + ph;
            let near_tray =
                (pt.x - tray_x).abs() <= 30 && (pt.y - tray_y).abs() <= 30;
            if over_popup || near_tray {
                misses = 0;
            } else {
                misses += 1;
                if misses >= 3 {
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(p) = app2.get_webview_window("notify") {
                            let _ = p.hide();
                        }
                    });
                    return;
                }
            }
        }
    });
}

/// Mark the logo's *interior* transparent pixels (the hollow center). Transparent
/// pixels reachable from the image border via flood-fill are exterior background;
/// the remaining transparent pixels are the enclosed hole.
fn hole_mask(base: &tauri::image::Image) -> Vec<bool> {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let rgba = base.rgba();
    let n = w * h;
    let is_t = |i: usize| rgba[i * 4 + 3] <= 32;
    let mut exterior = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    // Seed from all border transparent pixels.
    for x in 0..w {
        for &i in &[x, (h - 1) * w + x] {
            if is_t(i) && !exterior[i] { exterior[i] = true; stack.push(i); }
        }
    }
    for y in 0..h {
        for &i in &[y * w, y * w + (w - 1)] {
            if is_t(i) && !exterior[i] { exterior[i] = true; stack.push(i); }
        }
    }
    while let Some(i) = stack.pop() {
        let x = i % w;
        let y = i / w;
        if x > 0 { let j = i - 1; if is_t(j) && !exterior[j] { exterior[j] = true; stack.push(j); } }
        if x + 1 < w { let j = i + 1; if is_t(j) && !exterior[j] { exterior[j] = true; stack.push(j); } }
        if y > 0 { let j = i - w; if is_t(j) && !exterior[j] { exterior[j] = true; stack.push(j); } }
        if y + 1 < h { let j = i + w; if is_t(j) && !exterior[j] { exterior[j] = true; stack.push(j); } }
    }
    (0..n).map(|i| is_t(i) && !exterior[i]).collect()
}

/// Precompute tray frames that pulse the logo's hollow center with a bright,
/// high-contrast light that blooms ~2px outward (center brightest, fading out)
/// so it's eye-catching despite the tiny tray size — while the logo body stays
/// clean. Cycling these is the "unread subscription" indicator.
fn make_center_pulse_frames(base: &tauri::image::Image) -> Vec<tauri::image::Image<'static>> {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let src = base.rgba();
    let hole = hole_mask(base);
    let has_hole = hole.iter().any(|&b| b);
    let (cr, cg, cb) = (255.0f32, 45.0, 45.0); // vivid alert red

    // Glow weight per pixel: 1.0 in the hole, fading out 2px (a soft bloom).
    let mut weight = vec![0.0f32; w * h];
    if has_hole {
        use std::collections::VecDeque;
        let mut dist = vec![u8::MAX; w * h];
        let mut q: VecDeque<usize> = VecDeque::new();
        for i in 0..w * h {
            if hole[i] { dist[i] = 0; q.push_back(i); }
        }
        while let Some(i) = q.pop_front() {
            let d = dist[i];
            if d >= 2 { continue; }
            let (x, y) = (i % w, i / w);
            let mut nb = [usize::MAX; 4];
            if x > 0 { nb[0] = i - 1; }
            if x + 1 < w { nb[1] = i + 1; }
            if y > 0 { nb[2] = i - w; }
            if y + 1 < h { nb[3] = i + w; }
            for &j in nb.iter() {
                if j != usize::MAX && dist[j] == u8::MAX {
                    dist[j] = d + 1;
                    q.push_back(j);
                }
            }
        }
        for i in 0..w * h {
            weight[i] = match dist[i] { 0 => 1.0, 1 => 0.65, 2 => 0.35, _ => 0.0 };
        }
    }

    // Two frames only → simple bright/dark alternation (no gradient).
    let levels = [1.0f32, 0.0];
    levels
        .iter()
        .map(|&t| {
            let mut rgba = src.to_vec();
            if has_hole {
                for (i, &ww) in weight.iter().enumerate() {
                    if ww <= 0.0 { continue; }
                    let p = i * 4;
                    let k = (t * ww).min(1.0);
                    rgba[p] = (rgba[p] as f32 * (1.0 - k) + cr * k) as u8;
                    rgba[p + 1] = (rgba[p + 1] as f32 * (1.0 - k) + cg * k) as u8;
                    rgba[p + 2] = (rgba[p + 2] as f32 * (1.0 - k) + cb * k) as u8;
                    // Light up the transparent hole; logo pixels stay opaque.
                    rgba[p + 3] = (rgba[p + 3] as f32).max(255.0 * k) as u8;
                }
            } else {
                // Fallback (no detectable hole): tint the whole logo.
                for px in rgba.chunks_exact_mut(4) {
                    if px[3] > 32 {
                        px[0] = (px[0] as f32 * (1.0 - t) + cr * t) as u8;
                        px[1] = (px[1] as f32 * (1.0 - t) + cg * t) as u8;
                        px[2] = (px[2] as f32 * (1.0 - t) + cb * t) as u8;
                    }
                }
            }
            tauri::image::Image::new_owned(rgba, w as u32, h as u32)
        })
        .collect()
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
    for b in &mut baro {
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

    // Arbitration is epoch-computed each tick. Suppress until the first
    // worldstate fetch completes so it doesn't render ahead of other panels.
    let arbitration = if state.initialized { parse_arbitration(now) } else { None };

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

/// Update all time-dependent fields of a cached `AppStatePayload` in-place.
/// Called every tick instead of rebuilding from scratch via [`build_payload`],
/// so the 7 Vec clones + full re-allocation are avoided. Only `fmt_remain`
/// strings are re-allocated (they change every second regardless).
fn refresh_cached_payload(
    p: &mut AppStatePayload,
    now: i64,
    timer_payload: &MissionTimerPayload,
    countdown_secs: u32,
    last_update: &str,
    initialized: bool,
) {
    for f in p.normal_fissures.iter_mut()
        .chain(p.hard_fissures.iter_mut())
        .chain(p.storm_fissures.iter_mut())
    {
        let remain = f.expiry_ms - now;
        f.remain_ms = remain;
        f.remain_str = fmt_remain(remain);
        f.is_expiring = remain > 0 && remain < 300_000;
    }
    for c in &mut p.cycles {
        let remain = c.expiry_ms - now;
        if remain <= 0 {
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
    for b in &mut p.baro {
        if !b.active && now >= b.start_ms {
            b.active = true;
        }
        let target = if b.active { b.end_ms } else { b.start_ms };
        b.remain_ms = target - now;
        b.remain_str = fmt_remain_baro(b.remain_ms);
    }
    for b in &mut p.bounties {
        b.remain_ms = b.expiry_ms - now;
        b.remain_str = fmt_remain(b.remain_ms);
    }
    if let Some(c) = &mut p.circuit {
        c.remain_ms = c.expiry_ms - now;
        c.remain_str = fmt_remain_days(c.remain_ms);
    }
    p.arbitration = if initialized { parse_arbitration(now) } else { None };
    p.mission_timer = timer_payload.clone();
    p.countdown_secs = countdown_secs;
    p.last_update = last_update.to_string();
}

type SharedConfig = Arc<StdRwLock<AppConfig>>;
type MissionTimerShared = Arc<StdRwLock<MissionTimerState>>;

// ── Hotkey parsing ──────────────────────────────────────────────────────────

fn parse_key_code(s: &str) -> Option<Code> {
    match s {
        "A" => Some(Code::KeyA), "B" => Some(Code::KeyB), "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD), "E" => Some(Code::KeyE), "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG), "H" => Some(Code::KeyH), "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ), "K" => Some(Code::KeyK), "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM), "N" => Some(Code::KeyN), "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP), "Q" => Some(Code::KeyQ), "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS), "T" => Some(Code::KeyT), "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV), "W" => Some(Code::KeyW), "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY), "Z" => Some(Code::KeyZ),
        "0" => Some(Code::Digit0), "1" => Some(Code::Digit1), "2" => Some(Code::Digit2),
        "3" => Some(Code::Digit3), "4" => Some(Code::Digit4), "5" => Some(Code::Digit5),
        "6" => Some(Code::Digit6), "7" => Some(Code::Digit7), "8" => Some(Code::Digit8),
        "9" => Some(Code::Digit9),
        "F1" => Some(Code::F1), "F2" => Some(Code::F2), "F3" => Some(Code::F3),
        "F4" => Some(Code::F4), "F5" => Some(Code::F5), "F6" => Some(Code::F6),
        "F7" => Some(Code::F7), "F8" => Some(Code::F8), "F9" => Some(Code::F9),
        "F10" => Some(Code::F10), "F11" => Some(Code::F11), "F12" => Some(Code::F12),
        "Space" => Some(Code::Space),
        "Tab" => Some(Code::Tab),
        "Escape" | "Esc" => Some(Code::Escape),
        "Backspace" => Some(Code::Backspace),
        "\\" | "Backslash" => Some(Code::Backslash),
        "Enter" | "Return" => Some(Code::Enter),
        "Insert" => Some(Code::Insert),
        "Delete" => Some(Code::Delete),
        "Home" => Some(Code::Home),
        "End" => Some(Code::End),
        "PageUp" => Some(Code::PageUp),
        "PageDown" => Some(Code::PageDown),
        "Up" => Some(Code::ArrowUp),
        "Down" => Some(Code::ArrowDown),
        "Left" => Some(Code::ArrowLeft),
        "Right" => Some(Code::ArrowRight),
        _ => None,
    }
}

fn parse_hotkey_string(s: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    let key_str = parts[parts.len() - 1];
    if key_str.is_empty() {
        return Err("缺少按键，格式应为 Mod+Key 或 Key".into());
    }
    let key = parse_key_code(key_str).ok_or_else(|| format!("未知按键: {}", key_str))?;

    let mut mods = Modifiers::empty();
    for m in &parts[..parts.len() - 1] {
        match *m {
            "Ctrl" | "Control" => mods |= Modifiers::CONTROL,
            "Alt" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            "Meta" | "Win" | "Super" => mods |= Modifiers::META,
            other => return Err(format!("未知修饰键: {}", other)),
        }
    }
    Ok(Shortcut::new(Some(mods), key))
}

/// Register a hotkey by string. Returns Ok(()) or Err(conflict message).
fn register_hotkey_str(app: &tauri::AppHandle, s: &str) -> Result<(), String> {
    let sc = parse_hotkey_string(s)?;
    app.global_shortcut().register(sc).map_err(|e| format!("热键冲突: {}", e))
}

/// Unregister a hotkey by string. If parse fails, silently ignore.
fn unregister_hotkey_str(app: &tauri::AppHandle, s: &str) {
    if let Ok(sc) = parse_hotkey_string(s) {
        let _ = app.global_shortcut().unregister(sc);
    }
}

/// Fetch worldstate, store it into `state`, then emit `worldstate-update`.
/// Network/parse errors are returned; background loops ignore them, the
/// `refresh_now` command surfaces them to the frontend.
async fn fetch_store_emit(
    state: &SharedState,
    timer: &MissionTimerShared,
    handle: &tauri::AppHandle,
    cfg: &SharedConfig,
) -> Result<(), String> {
    let source = cfg.read().unwrap().worldstate_source.clone();
    let result = fetch_worldstate(&source).await;
    match result {
        Ok(data) => {
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
                s.last_fetch_wall_ms = now_ms(); // wall-clock anchor for tick-loop countdown derivation
                s.initialized = true; // data is ready — allow local computation (arbitration) to join
                // Reset Baro arrival flag when none are active, so the next
                // arrival triggers the auto-refresh task again.
                let any_active = s.baro.iter().any(|b| b.active);
                if !any_active {
                    s.baro_arrival_handled = false;
                }
                // Build a fresh payload while we hold the write lock, cache it in
                // state so the per-second tick can refresh it in-place without
                // cloning every fissure/cycle/bounty vec all over again.
                let t = timer.read().unwrap();
                s.cached_payload = build_payload(&s, &t);
                let _ = handle.emit("worldstate-update", &s.cached_payload);
            }
            Ok(())
        }
        Err(e) => {
            // Network failed, but allow locally-computed data to show anyway.
            let mut s = state.write().await;
            s.initialized = true;
            Err(e)
        }
    }
}

#[tauri::command]
async fn refresh_now(
    state: tauri::State<'_, SharedState>,
    timer_state: tauri::State<'_, MissionTimerShared>,
    app: tauri::AppHandle,
    config: tauri::State<'_, SharedConfig>,
    refreshing: tauri::State<'_, Arc<RefreshGuard>>,
) -> Result<(), String> {
    if refreshing.0.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Relaxed).is_err() {
        return Err("正在刷新中，请稍后再试".into());
    }
    let result = fetch_store_emit(&state, &timer_state, &app, &config).await;
    refreshing.0.store(false, std::sync::atomic::Ordering::Release);
    result
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
fn get_hotkey(cfg: tauri::State<'_, SharedConfig>) -> Option<String> {
    cfg.read().unwrap().hotkey.clone()
}

#[tauri::command]
fn set_hotkey(
    cfg: tauri::State<'_, SharedConfig>,
    app: tauri::AppHandle,
    hotkey: Option<String>,
) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;

    // Hold the write lock for the entire read→unregister→register→persist
    // sequence so two concurrent set_hotkey calls cannot leave stale shortcuts
    // registered at the OS level (TOCTOU).
    let mut c = cfg.write().unwrap();
    let old = c.hotkey.clone();

    // Unregister old hotkey if one was set.
    if let Some(ref old_hk) = old {
        unregister_hotkey_str(&app, old_hk);
    }

    // Register new hotkey if provided.  On failure, re-register the old one
    // so the user isn't left with no hotkey at all.
    if let Some(ref hk) = hotkey {
        if let Err(e) = register_hotkey_str(&app, hk) {
            if let Some(ref old_hk) = old {
                let _ = register_hotkey_str(&app, old_hk);
            }
            return Err(e);
        }
    }

    // Persist.
    c.hotkey = hotkey;
    save_config(&app_data_dir, &c)?;

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

/// Capture the current game frame for the ROI calibration overlay. Returns a
/// `data:image/png;base64,...` URL. The frame is processed identically to the
/// OCR pipeline (same `strip_frame`), so drawn ROI fractions map 1:1.
#[tauri::command]
fn capture_preview(config: tauri::State<'_, SharedConfig>) -> Result<String, String> {
    let cfg = config.read().unwrap();
    let hwnd = window::resolve_hwnd(&cfg.mission_timer.window_title);
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
    let hwnd = window::resolve_hwnd(&cfg.mission_timer.window_title);
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
            window::resolve_hwnd(&cfg.mission_timer.window_title),
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

/// The Warframe game update the bundled item-name table (`baro_zh.json`) was
/// generated from. Updated by hand at release time (alongside refreshing the
/// table), since the bundled table is a frozen snapshot per release.
const GAME_DATA_VERSION: &str = "更新 43《Jade 之影：众星》";

/// Surface the game version the bundled item library corresponds to (设置 → 物品库).
#[tauri::command]
fn game_data_version() -> &'static str {
    GAME_DATA_VERSION
}

/// Open the log file's directory in Explorer (设置 → 联系作者).
#[tauri::command]
fn open_log_folder(app: tauri::AppHandle) -> Result<(), String> {
    let path = app.path().app_data_dir().map_err(|e| e.to_string())?.join("voxalic.log");
    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path.to_str().unwrap_or(""))
        .spawn();
    Ok(())
}

/// Open a QQ chat window via tencent:// protocol (uses ShellExecuteW directly,
/// bypassing WebView2 protocol restrictions).
#[tauri::command]
fn open_qq_chat(uin: String) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    use windows::Win32::Foundation::HINSTANCE;
    use windows::core::HSTRING;

    let url = format!("tencent://message/?uin={uin}&Site=VoxAlic&Menu=yes");
    let url_hstring = HSTRING::from(&url);

    // HINSTANCE > 32 means success (per Win32 convention)
    unsafe {
        let ret: HINSTANCE = ShellExecuteW(
            None,
            windows::core::w!("open"),
            &url_hstring,
            None,
            None,
            SW_SHOW,
        );
        (ret.0 as isize) > 32
    }
}

/// Gather system diagnostics for feedback emails (Windows only).
fn collect_diagnostics() -> String {
    let raw = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NoLogo", "-Command", r#"
$os  = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor | Select -First 1
$gpu = Get-CimInstance Win32_VideoController | Where {$_.Name -notlike "*Virtual*" -and $_.Name -notlike "*Remote*"} | Select -First 1
$loc = (Get-Culture).Name
$tz  = (Get-TimeZone).Id
Write-Output "$($os.Caption.Trim())|$($os.Version)|$($os.OSArchitecture)|$loc|$tz|$($cpu.Name.Trim())|$($gpu.Name.Trim())|$($gpu.DriverVersion)"
"#.trim()])
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })
        .unwrap_or_default();

    let mut p = raw.splitn(8, '|');
    let os_name = p.next().filter(|s| !s.is_empty()).unwrap_or("Windows");
    let os_ver  = p.next().filter(|s| !s.is_empty()).unwrap_or("?");
    let os_arch = p.next().filter(|s| !s.is_empty()).unwrap_or("?");
    let locale  = p.next().filter(|s| !s.is_empty()).unwrap_or("?");
    let tz      = p.next().filter(|s| !s.is_empty()).unwrap_or("?");
    let cpu     = p.next().filter(|s| !s.is_empty()).unwrap_or("?");
    let gpu     = p.next().filter(|s| !s.is_empty()).unwrap_or("?");
    let gpu_drv = p.next().filter(|s| !s.is_empty()).unwrap_or("?");

    format!(
        "操作系统：{os_name} ({os_ver}) {os_arch}\n\
         系统语言：{locale}\n\
         时区：{tz}\n\
         CPU：{cpu}\n\
         显卡：{gpu}（驱动 {gpu_drv}）",
    )
}

/// Send user feedback via SMTP to the developer's QQ mailbox.
/// Uses the existing tokio runtime (same as the rest of the app).
#[tauri::command]
async fn send_feedback(message: String) -> Result<String, String> {
    use lettre::message::Message;
    use lettre::message::header::ContentType;
    use lettre::{AsyncTransport, Tokio1Executor};
    use lettre::transport::smtp::{AsyncSmtpTransport, authentication::Credentials};

    let app_version = env!("CARGO_PKG_VERSION");
    let log_dir = std::env::var("APPDATA")
        .map(|roaming| format!("{}\\com.voxalic.app", roaming))
        .unwrap_or_else(|_| "（无法获取路径）".into());
    let diag = collect_diagnostics();
    let body = format!(
        "【VoxAlic 用户反馈】\n\
         ───────────────────\n\
         {}\n\
         ───────────────────\n\
         版本：v{}\n\
         {diag}\n\
         架构：{}\n\
         时间：{}\n\
         \n\
         📂 日志文件位于：{}\\voxalic.log\n\
         （请附上日志文件，便于排查问题）",
        message,
        app_version,
        std::env::consts::ARCH,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        log_dir,
    );

    let email = Message::builder()
        .from(SMTP_USER.parse().map_err(|e| format!("发件人格式错误：{e}"))?)
        .to(SMTP_TO.parse().map_err(|e| format!("收件人格式错误：{e}"))?)
        .subject(format!("[VoxAlic v{app_version}] 用户反馈"))
        .header(ContentType::parse("text/plain; charset=utf-8").unwrap())
        .body(body)
        .map_err(|e| format!("邮件构建失败：{e}"))?;

    let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay("smtp.qq.com")
        .map_err(|e| format!("SMTP 地址解析失败：{e}"))?
        .credentials(Credentials::new(
            SMTP_USER.to_string(),
            smtp_auth_code(),
        ))
        .build();

    tokio::time::timeout(
        std::time::Duration::from_secs(SMTP_TIMEOUT_SECS),
        mailer.send(email),
    )
    .await
    .map_err(|_| "发送超时，请检查网络连接".to_string())?
    .map_err(|e| format!("发送失败：{e}"))?;

    Ok("ok".to_string())
}

/// Current accumulated subscription notifications (newest first) for the popup.
#[tauri::command]
fn get_notifications(list: tauri::State<'_, NotifyList>) -> Vec<SubNotify> {
    list.read().unwrap().clone()
}

/// Click-through from a popup item: raise the main window, acknowledge (stop the
/// tray flash), hide the popup, and tell the main UI to navigate to the matching
/// entry (fissure tab + sub-tab + highlighted row).
#[tauri::command]
fn open_main_navigate(
    kind: String,
    node: String,
    sub: String,
    app: tauri::AppHandle,
    flashing: tauri::State<'_, FlashFlag>,
    hide_gen: tauri::State<'_, HideGen>,
) {
    flashing.store(false, std::sync::atomic::Ordering::Relaxed);
    hide_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // stop the watcher
    if let Some(popup) = app.get_webview_window("notify") {
        let _ = popup.hide();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit_to("main", "navigate", NavigateMsg { kind, node, sub });
}

/// Clear all subscription notifications and stop the tray flashing.
#[tauri::command]
fn clear_notifications(list: tauri::State<'_, NotifyList>, flashing: tauri::State<'_, FlashFlag>) {
    list.write().unwrap().clear();
    flashing.store(false, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
fn get_bark_url(cfg: tauri::State<'_, SharedConfig>) -> String {
    cfg.read().unwrap().notify_bark_url.clone()
}

#[tauri::command]
async fn test_phone_push(cfg: tauri::State<'_, SharedConfig>) -> Result<String, String> {
    let u = cfg.read().unwrap().notify_bark_url.clone();
    if u.is_empty() {
        return Err("未配置 Bark URL".into());
    }
    phone_push::push(&u, "VoxAlic", "✅ 手机通知测试成功").await;
    Ok("已发送".into())
}

/// Inject a sample subscription notification so the user can preview the tray
/// flash + hover popup without waiting for a real fissure to appear.
// ── In-app updater ───────────────────────────────────────────────────────────

#[derive(Clone, serde::Serialize)]
struct UpdateInfo {
    version: String,
    notes: String,
}

/// Build an updater pointed at the chosen release source. `"gitee"` checks the
/// Gitee raw `latest.json` (fast in China); any other value falls back to
/// GitHub. The pubkey + signature verification are inherited from config, and
/// the same key signs both mirrors so either source verifies.
fn build_source_updater(
    app: &tauri::AppHandle,
    source: &str,
) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    let endpoint = match source {
        "gitee" => "https://gitee.com/Swing_Rainbow/vox-alic/raw/master/latest.json",
        _ => "https://github.com/SwingRainbow/VoxAlic/releases/latest/download/latest.json",
    };
    let url = url::Url::parse(endpoint).map_err(|e| e.to_string())?;
    app.updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_for_update(app: tauri::AppHandle, source: String) -> Result<Option<UpdateInfo>, String> {
    let updater = build_source_updater(&app, &source)?;
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
async fn install_update(app: tauri::AppHandle, source: String) -> Result<(), String> {
    let updater = build_source_updater(&app, &source)?;
    if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
        app.exit(0);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ── Resolve app_data_dir before Tauri creates any window ──
    // On Windows Tauri v2's app_data_dir() uses RoamingAppData (%APPDATA%).
    // We compute it manually so we can load config and manage ALL state via
    // Builder::manage() BEFORE .setup() — eliminating the race where
    // webviews load and invoke commands before app.manage() inside setup.
    let app_data_dir = {
        let roaming = std::env::var("APPDATA").unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE").unwrap_or_default();
            format!("{}\\AppData\\Roaming", home)
        });
        std::path::PathBuf::from(roaming).join("com.voxalic.app")
    };
    let _ = std::fs::create_dir_all(&app_data_dir); // ensure exists

    // ── Load config before Tauri creates any window ──
    let config: SharedConfig = Arc::new(StdRwLock::new(load_config(&app_data_dir)));

    // ── Create all managed state before Tauri creates any window ──
    let state: SharedState = SharedState::default();
    let templates_arc: Arc<DigitTemplates> = Arc::new(DigitTemplates::load());
    let timer_state: MissionTimerShared = Arc::new(StdRwLock::new(MissionTimerState::new()));
    let notify_list: NotifyList = Arc::new(StdRwLock::new(Vec::new()));
    let flashing: FlashFlag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hide_gen: HideGen = Arc::new(std::sync::atomic::AtomicU64::new(0));
    // Guards against concurrent manual refresh requests (button spam / tray spam).
    let refreshing: Arc<RefreshGuard> = Arc::new(RefreshGuard(Arc::new(std::sync::atomic::AtomicBool::new(false))));
    let market_cache: market::SharedMarketCache = Arc::new(tokio::sync::RwLock::new(market::build_cache(&app_data_dir)));
    let market_auth: SharedMarketAuth = load_or_create_auth(&app_data_dir);

    tauri::Builder::default()
        .manage(state.clone())
        .manage(config.clone())
        .manage(templates_arc.clone())
        .manage(timer_state.clone())
        .manage(notify_list.clone())
        .manage(flashing.clone())
        .manage(hide_gen.clone())
        .manage(refreshing.clone())
        .manage(market_cache.clone())
        .manage(market_auth.clone())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(
            |app: &tauri::AppHandle,
             _shortcut: &tauri_plugin_global_shortcut::Shortcut,
             event: tauri_plugin_global_shortcut::ShortcutEvent| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                // Any registered hotkey toggles the main window.
                if let Some(w) = app.get_webview_window("main") {
                    let vis = w.is_visible().unwrap_or(false);
                    let foc = w.is_focused().unwrap_or(false);
                    if !vis {
                        let _ = w.show();
                        let _ = w.set_focus();
                    } else if !foc {
                        let _ = w.set_focus();
                    } else {
                        let _ = w.hide();
                    }
                }
            },
        ).build())
        .setup(move |app| {
            // Load item-name 中文 table (user override file, else embedded default).
            item_i18n::init(&app_data_dir);
            log_init::init(&app_data_dir);

            // Market cache background init: notify frontend when ready.
            let mc = market_cache.clone();
            let mc_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let count = mc.read().await.items.len();
                let _ = mc_handle.emit("market-cache-ready", count as i32);
            });

            // Register global hotkey from config (if set).
            {
                let cfg = config.read().unwrap();
                if let Some(ref hk) = cfg.hotkey {
                    match parse_hotkey_string(hk) {
                        Ok(sc) => {
                            if let Err(e) = app.handle().global_shortcut().register(sc) {
                                warn!("hotkey register '{hk}' failed: {e}");
                            }
                        }
                        Err(e) => warn!("hotkey parse '{hk}' failed: {e}"),
                    }
                }
            }

            // Mission timer command channel
            let timer_config = config.clone();
            let timer_shared = timer_state.clone();
            let (log_tx, log_rx) = mpsc::channel::<String>();
            // Alert channel: the OCR thread has no AppHandle, so mission-timer
            // toast requests are forwarded here for delivery.
            let (alert_tx, alert_rx) = mpsc::channel::<AlertMsg>();
            let cmd_tx = start_timer_thread(timer_shared, timer_config, templates_arc, log_tx, alert_tx);
            app.manage(cmd_tx);

            // Subscription notifications: fissure/cycle/arbitration matches go to
            // the tray (red-dot + flashing + click popup), NOT a toast.
            let (notify_tx, notify_rx) = mpsc::channel::<SubNotify>();

            // Phone push config: Bark URL. Empty = disabled.
            let bark_cfg = config.clone();

            // Tray icons: normal + a red-dot variant, alternated to "flash".
            // Build an *owned* (`'static`) copy — the borrowed `default_window_icon`
            // is tied to `app` and can't move into the flash thread.
            let base_ref = app.default_window_icon().unwrap();
            let base_icon = tauri::image::Image::new_owned(
                base_ref.rgba().to_vec(),
                base_ref.width(),
                base_ref.height(),
            );
            let glow_frames = make_center_pulse_frames(&base_icon);

            // Log forwarding thread
            let log_handle = app.handle().clone();
            std::thread::spawn(move || {
                while let Ok(msg) = log_rx.recv() {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = log_handle.emit("timer-log", msg);
                    }));
                    if result.is_err() {
                        let _ = log_handle.emit(
                            "timer-log",
                            format!("[{}] ⚠ 日志转发异常已恢复", chrono::Local::now().format("%H:%M:%S")),
                        );
                    }
                }
            });

            // Alert (toast) forwarding thread — mission timer only.
            let alert_handle = app.handle().clone();
            let alert_bark = bark_cfg.clone();
            std::thread::spawn(move || {
                while let Ok(msg) = alert_rx.recv() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        show_toast(&alert_handle, &msg.title, &msg.body);
                        let u = alert_bark.read().unwrap().notify_bark_url.clone();
                        if !u.is_empty() {
                            let title = msg.title.clone();
                            let body = msg.body.clone();
                            tauri::async_runtime::spawn(async move {
                                phone_push::push(&u, &title, &body).await;
                            });
                        }
                    }));
                }
            });

            // Subscription-notify forwarding thread: accumulate the list, push it
            // to the popup webview, update the tray tooltip, and start flashing.
            let notify_handle = app.handle().clone();
            let notify_store = notify_list.clone();
            let notify_flag = flashing.clone();
            let sub_bark = bark_cfg.clone();
            std::thread::spawn(move || {
                while let Ok(msg) = notify_rx.recv() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let snapshot = {
                            let mut list = notify_store.write().unwrap();
                            list.insert(0, msg.clone());
                        if list.len() > 50 { list.truncate(50); }
                        list.clone()
                    };
                    let _ = notify_handle.emit("sub-notify", snapshot);
                    notify_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    let u = sub_bark.read().unwrap().notify_bark_url.clone();
                    if !u.is_empty() {
                        let title = msg.title.clone();
                        let body = msg.detail.clone();
                        tauri::async_runtime::spawn(async move {
                            phone_push::push(&u, &title, &body).await;
                        });
                    }
                    }));
                }
            });

            // Tray flash thread: while `flashing`, alternate the tray icon every
            // 600ms; reset to the normal icon once when flashing stops. Tray ops
            // must run on the main thread.
            let flash_handle = app.handle().clone();
            let flash_flag = flashing.clone();
            let flash_normal = base_icon.clone();
            let flash_frames = glow_frames;
            std::thread::spawn(move || {
                let mut idx = 0usize;
                let mut was_active = false;
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let active = flash_flag.load(std::sync::atomic::Ordering::Relaxed);
                        if active {
                            was_active = true;
                            let icon = flash_frames[idx % flash_frames.len()].clone();
                            idx = idx.wrapping_add(1);
                            let h = flash_handle.clone();
                            let _ = flash_handle.run_on_main_thread(move || {
                                if let Some(tray) = h.tray_by_id("main") {
                                    let _ = tray.set_icon(Some(icon));
                                }
                            });
                        } else if was_active {
                            was_active = false;
                            idx = 0;
                            let icon = flash_normal.clone();
                            let h = flash_handle.clone();
                            let _ = flash_handle.run_on_main_thread(move || {
                                if let Some(tray) = h.tray_by_id("main") {
                                    let _ = tray.set_icon(Some(icon));
                                }
                            });
                        }
                    }));
                }
            });

            // Shared cycle-state history for check_cycle_alerts — both the
            // fetch loop and the tick loop (gap recovery) write to it so a
            // sleep→wake doesn't leave prev states stuck in the past.
            let prev_cycle_states: PrevCycleStates =
                Arc::new(StdRwLock::new(std::collections::HashMap::new()));

            // Background fetch loop (every REFRESH_SEC)
            let fetch_state = state.clone();
            let fetch_timer = timer_state.clone();
            let fetch_handle = app.handle().clone();
            let fetch_config = config.clone();
            let fetch_notify_tx = notify_tx.clone();
            let fetch_prev_cycle_states = prev_cycle_states.clone();
            tauri::async_runtime::spawn(async move {
                let mut notified_fissures: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut prev_arb_key = String::new();
                // `interval`'s first tick fires immediately → fetch once at
                // startup, then every REFRESH_SEC.
                let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_SEC as u64));
                loop {
                    interval.tick().await;
                    if let Err(e) = fetch_store_emit(&fetch_state, &fetch_timer, &fetch_handle, &fetch_config).await {
                        error!("fetch-loop worldstate fetch failed: {e}");
                        continue;
                    }
                    {
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
                        check_fissure_alerts(&all_fissures, &fissure_alerts, &mut notified_fissures, &fetch_notify_tx);
                        check_cycle_alerts(&cycles, &cycle_alerts, &mut *fetch_prev_cycle_states.write().unwrap(), &fetch_notify_tx);
                        let arb = parse_arbitration(now);
                        check_arbitration_alerts(arb.as_ref(), &arb_alerts, &mut prev_arb_key, &fetch_notify_tx);
                    }
                }
            });

            // Per-second tick
            let tick_state = state.clone();
            let tick_timer = timer_state.clone();
            let tick_handle = app.handle().clone();
            let tick_notify_tx = notify_tx.clone();
            let tick_config = config.clone();
            let tick_prev_cycle_states = prev_cycle_states.clone();
            let tick_advance_fired: Arc<StdRwLock<std::collections::HashSet<String>>> =
                Arc::new(StdRwLock::new(std::collections::HashSet::new()));
            tauri::async_runtime::spawn(async move {
                let mut last_wall = std::time::SystemTime::now();
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;

                    let now = now_ms();
                    // Wall-clock gap detection — catches tick loss from sleep,
                    // CPU starvation, or any other cause.
                    let elapsed_wall = std::time::SystemTime::now()
                        .duration_since(last_wall)
                        .unwrap_or_default();
                    let need_cycle_check = elapsed_wall > Duration::from_secs(2);

                    // ── Async lock acquisition + synchronous state update ──
                    // Detect Baro arrival transition BEFORE refresh so we can
                    // trigger a worldstate fetch after the write lock is released.
                    let baro_was_inactive: bool;
                    let mut baro_just_arrived = false;
                    let need_fetch: bool;  // self-healing: tick-loop gap recovery
                    {
                        let mut s = tick_state.write().await;
                        // ── Countdown from wall clock (not an accumulator) ──
                        let diff_ms = now - s.last_fetch_wall_ms;
                        let elapsed_s = if diff_ms > 0 {
                            (diff_ms / 1000) as u32
                        } else {
                            0 // NTP backward jump — stay conservative
                        };
                        s.countdown_secs = REFRESH_SEC.saturating_sub(elapsed_s);
                        // Self-healing: if the periodic fetch loop missed a cycle
                        // (network blip, sleep, etc.), trigger a fetch from the tick.
                        need_fetch = elapsed_s >= REFRESH_SEC;
                        if need_fetch {
                            s.last_fetch_wall_ms = now; // reset timer now so we don't re-fire every tick
                        }
                        {
                            let mut t = tick_timer.write().unwrap();
                            t.update_elapsed();
                        }
                        let t = tick_timer.read().unwrap();
                        let countdown = s.countdown_secs;
                        let last_update = s.last_update.clone();
                        let initialized = s.initialized;
                        // Snapshot Baro's active flag before the refresh — if any
                        // flips from false→true this tick, we need a data refresh.
                        baro_was_inactive = !s.cached_payload.baro.iter().any(|b| b.active);
                        refresh_cached_payload(
                            &mut s.cached_payload,
                            now,
                            &t.payload,
                            countdown,
                            &last_update,
                            initialized,
                        );
                        // Did any Baro just arrive this tick?
                        let baro_now_active = s.cached_payload.baro.iter().any(|b| b.active);
                        if baro_was_inactive && baro_now_active && !s.baro_arrival_handled {
                            s.baro_arrival_handled = true;
                            baro_just_arrived = true;
                        }
                    }

                    // ── Baro arrival auto-refresh: fire immediately + 5 s retry.
                    // The API may need a few seconds to update after Baro lands.
                    if baro_just_arrived {
                        let fetch_state = tick_state.clone();
                        let fetch_timer = tick_timer.clone();
                        let fetch_handle = tick_handle.clone();
                        let fetch_config = tick_config.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = fetch_store_emit(&fetch_state, &fetch_timer, &fetch_handle, &fetch_config).await;
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            let _ = fetch_store_emit(&fetch_state, &fetch_timer, &fetch_handle, &fetch_config).await;
                        });
                    }

                    // ── Self-healing fetch: periodic loop missed a cycle.
                    // When the countdown expires without a refresh (network blip,
                    // sleep gap, etc.), the tick loop triggers a fetch so fissures
                    // don't dwindle.  Normal-path fetches happen in the background
                    // loop; this is a safety net.
                    if need_fetch && !baro_just_arrived {
                        let fetch_state = tick_state.clone();
                        let fetch_timer = tick_timer.clone();
                        let fetch_handle = tick_handle.clone();
                        let fetch_config = tick_config.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = fetch_store_emit(&fetch_state, &fetch_timer, &fetch_handle, &fetch_config).await {
                                error!("tick self-healing fetch failed: {e}");
                            }
                        });
                    }

                    // ── Read-only emit + alert checks (purely synchronous) ──
                    let s = tick_state.read().await;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = tick_handle.emit("tick-update", &s.cached_payload);

                        let cycle_alerts = tick_config.read().unwrap().cycle_alerts.clone();
                        if !cycle_alerts.is_empty() {
                            let mut fired = tick_advance_fired.write().unwrap();
                            check_cycle_advance_alerts(
                                &s.cached_payload.cycles,
                                &cycle_alerts,
                                &mut fired,
                                &tick_notify_tx,
                            );
                        }

                        // After a large gap (sleep / long pause), also run the
                        // state-transition check so we don't wait for the next
                        // fetch to notice a phase change.  Always sync the
                        // baseline so the next fetch doesn't compare against a
                        // stale prev.
                        if need_cycle_check {
                            let alerts = tick_config.read().unwrap().cycle_alerts.clone();
                            if !alerts.is_empty() {
                                let mut prev = tick_prev_cycle_states.write().unwrap();
                                check_cycle_alerts(
                                    &s.cached_payload.cycles,
                                    &alerts,
                                    &mut prev,
                                    &tick_notify_tx,
                                );
                            }
                        }
                    }));
                    if result.is_err() {
                        let _ = tick_handle.emit(
                            "timer-log",
                            format!(
                                "[{}] ⚠ 时钟循环异常已恢复",
                                chrono::Local::now().format("%H:%M:%S")
                            ),
                        );
                    }

                    // Always advance the wall-clock anchor — even if the tick
                    // body panicked, so the next iteration sees the real gap.
                    last_wall = std::time::SystemTime::now();
                }
            });

            // Auto-check for updates 3s after startup. Emits "update-available"
            // to the main window if a newer version is found.
            let update_handle = app.handle().clone();
            let update_cfg = config.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let source = update_cfg.read().unwrap().update_source.clone();
                match build_source_updater(&update_handle, &source) {
                    Ok(updater) => {
                        if let Ok(Some(u)) = updater.check().await {
                            let _ = update_handle.emit_to("main", "update-available",
                                UpdateInfo { version: u.version, notes: u.body.unwrap_or_default() });
                        }
                    }
                    Err(_) => {} // silent — network may not be ready
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
            let tray_config = config.clone();
            let tray_refreshing = refreshing.clone();
            let menu_flash = flashing.clone();
            let tray_flash = flashing.clone();
            let tray_notify = notify_list.clone();
            let tray_gen = hide_gen.clone();
            let _tray = TrayIconBuilder::with_id("main")
                .icon(base_icon.clone())
                .tooltip("VoxAlic")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        // Opening the main window counts as acknowledging.
                        menu_flash.store(false, std::sync::atomic::Ordering::Relaxed);
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "refresh" => {
                        if tray_refreshing.0.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Relaxed).is_err() {
                            return; // already refreshing — ignore duplicate
                        }
                        let flag = tray_refreshing.clone();
                        let state = tray_state.clone();
                        let timer = tray_timer.clone();
                        let handle = app.clone();
                        let cfg = tray_config.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = fetch_store_emit(&state, &timer, &handle, &cfg).await;
                            flag.0.store(false, std::sync::atomic::Ordering::Release);
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(move |tray, event| {
                    let app = tray.app_handle();
                    match event {
                        // Hover in → show the popup anchored to the *tray icon*
                        // (centered directly above it), not the cursor — so its
                        // position is fixed regardless of where on the icon the
                        // cursor entered. Then start the cursor-poll auto-hide.
                        TrayIconEvent::Enter { rect, .. } => {
                            // Bump generation: supersede any running watcher.
                            tray_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let snapshot = tray_notify.read().unwrap().clone();
                            // Nothing subscribed-and-fired → don't pop an empty box.
                            if snapshot.is_empty() {
                                if let Some(popup) = app.get_webview_window("notify") {
                                    let _ = popup.hide();
                                }
                                return;
                            }
                            if let Some(popup) = app.get_webview_window("notify") {
                                // Push the current list in case the popup webview
                                // missed earlier `sub-notify` events (startup race).
                                let _ = app.emit_to("notify", "sub-notify", snapshot);
                                // Tray icon rect → physical px. Center the popup on
                                // the icon's horizontal midpoint, sitting above it.
                                let scale = popup.scale_factor().unwrap_or(1.0);
                                let ipos = rect.position.to_physical::<i32>(scale);
                                let isz = rect.size.to_physical::<i32>(scale);
                                let icon_cx = ipos.x + isz.width / 2;
                                let icon_top = ipos.y;
                                let mut prect = (0i32, 0i32, 0i32, 0i32);
                                if let Ok(size) = popup.outer_size() {
                                    let (pw, ph) = (size.width as i32, size.height as i32);
                                    let mut x = icon_cx - pw / 2;
                                    let mut y = icon_top - ph - 8;
                                    // Clamp to the current monitor so a right-edge
                                    // tray icon doesn't push the box off-screen.
                                    if let Ok(Some(mon)) = popup.current_monitor() {
                                        let mp = mon.position();
                                        let ms = mon.size();
                                        let left = mp.x;
                                        let right = mp.x + ms.width as i32;
                                        x = x.clamp(left, (right - pw).max(left));
                                        y = y.max(mp.y);
                                    } else {
                                        x = x.max(0);
                                        y = y.max(0);
                                    }
                                    let _ = popup.set_position(tauri::PhysicalPosition::new(x, y));
                                    prect = (x, y, pw, ph);
                                }
                                let _ = popup.show();
                                start_popup_watch(
                                    app.clone(), tray_gen.clone(),
                                    icon_cx, icon_top, prect.0, prect.1, prect.2, prect.3,
                                );
                            }
                        }
                        // Hover out is handled by the cursor-poll watcher, not this
                        // (flaky) tray Leave event — intentionally a no-op.
                        TrayIconEvent::Leave { .. } => {}
                        // Left click → open the main window + acknowledge (stop flash).
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } => {
                            tray_flash.store(false, std::sync::atomic::Ordering::Relaxed);
                            tray_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // stop watcher
                            if let Some(popup) = app.get_webview_window("notify") {
                                let _ = popup.hide();
                            }
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // NOTE: deliberately NO `Focused(false) → hide` handler. The popup's
            // lifecycle is owned entirely by hover-intent (tray Leave + the
            // popup's own mouseenter/leave → `schedule_popup_hide`). A focus-loss
            // hide is an independent path that fires spuriously right after
            // `show()` (the popup never reliably holds focus) and bypasses the
            // grace delay, causing the popup to vanish mid-travel ("断触").

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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![refresh_now, get_config, set_config, get_hotkey, set_hotkey, timer_command, list_windows, select_window, single_capture, capture_preview, test_recognize, test_alert, update_item_names, item_names_count, game_data_version, open_log_folder, open_qq_chat, send_feedback, get_notifications, clear_notifications, open_main_navigate, get_autostart, set_autostart, uninstall_clean, check_for_update, install_update, get_bark_url, test_phone_push, search_market_items, get_market_item, refresh_market_cache, market_cache_status, translate_items, market_signin, market_signout, market_auth_status, market_set_status, market_list_orders, market_create_order, market_update_order, market_delete_order, market_close_order])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
