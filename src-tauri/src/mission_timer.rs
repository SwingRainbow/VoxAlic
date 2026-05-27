use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use chrono::Local;
use crate::capture::{self, ROIConfig};
use crate::config::AppConfig;
use crate::models::MissionTimerPayload;
use crate::ocr::{DigitTemplates, recognize_digits};

pub const MATCH_THRESHOLD: f32 = 0.70;
const MAX_REJECT: u32 = 3;
const CHECKPOINT_INTERVAL_SECS: u32 = 300;
const LIFE_SUPPORT_RED_THRESHOLD: f32 = 1.0;
/// Consecutive failed/black captures before forcing a window re-scan.
const CAPTURE_FAIL_RESCAN: u32 = 5;

#[derive(Debug, Clone)]
pub enum TimerCommand {
    Start,
    Stop,
    Reset,
    SetMode(String),
    SingleCapture,
}

pub const ALERT_TITLE: &str = "Warframe 计时器";

/// A reminder raised by the OCR thread, with its text already resolved (the
/// thread has config access; `lib.rs` does not). The thread has no `AppHandle`,
/// so toast delivery is forwarded over a channel to a handler in `lib.rs`.
#[derive(Debug, Clone)]
pub struct AlertMsg {
    pub title: String,
    pub body: String,
}

/// Everything `apply_ocr` needs to raise a checkpoint reminder, bundled so the
/// signature stays small.
struct AlertParams<'a> {
    checkpoint_enabled: bool,
    checkpoint_text: &'a str,
    method: &'a str,
    tx: &'a mpsc::Sender<AlertMsg>,
}

/// Substitute `{min}` in a user-configured reminder template with the reached
/// milestone in minutes. Falls back to the built-in default when blank.
fn render_alert_text(template: &str, default: &str, minutes: u32) -> String {
    let t = if template.trim().is_empty() { default } else { template };
    t.replace("{min}", &minutes.to_string())
}

/// Deliver a reminder using the configured method: "toast" forwards a
/// notification request to the `lib.rs` handler; anything else (default
/// "focus") forces the game window to the foreground.
fn dispatch_alert(
    body: String,
    hwnd: isize,
    alert_method: &str,
    alert_tx: &mpsc::Sender<AlertMsg>,
) {
    if alert_method == "toast" {
        let _ = alert_tx.send(AlertMsg {
            title: ALERT_TITLE.into(),
            body,
        });
    } else {
        crate::window::bring_to_front(hwnd);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TimerState {
    Idle,
    Running,
    Paused,
    Checkpoint,
}

pub struct MissionTimerState {
    pub payload: MissionTimerPayload,
    inner_state: TimerState,
    start_instant: Option<Instant>,
    paused_elapsed: Duration,
    last_valid_ocr: Option<String>,
    last_valid_instant: Option<Instant>,
    consecutive_rejects: u32,
    last_mode: String,
    ocr_attempts: u32,
    ocr_successes: u32,
    hp_alert_triggered: bool,
}

impl MissionTimerState {
    pub fn new() -> Self {
        Self {
            payload: MissionTimerPayload::default(),
            inner_state: TimerState::Idle,
            start_instant: None,
            paused_elapsed: Duration::ZERO,
            last_valid_ocr: None,
            last_valid_instant: None,
            consecutive_rejects: 0,
            last_mode: "normal".into(),
            ocr_attempts: 0,
            ocr_successes: 0,
            hp_alert_triggered: false,
        }
    }

    pub fn detection_rate(&self) -> f32 {
        if self.ocr_attempts == 0 {
            0.0
        } else {
            self.ocr_successes as f32 / self.ocr_attempts as f32 * 100.0
        }
    }

    pub fn handle_command(&mut self, cmd: &TimerCommand) {
        match cmd {
            TimerCommand::Start => {
                if self.inner_state == TimerState::Idle
                    || self.inner_state == TimerState::Paused
                    || self.inner_state == TimerState::Checkpoint
                {
                    let is_fresh = self.inner_state == TimerState::Idle;
                    self.inner_state = TimerState::Running;
                    if self.start_instant.is_none() {
                        self.start_instant = Some(Instant::now());
                        if is_fresh {
                            self.paused_elapsed = Duration::ZERO;
                        }
                    }
                    self.payload.status_text = "运行中".into();
                }
            }
            TimerCommand::Stop => {
                if self.inner_state == TimerState::Running
                    || self.inner_state == TimerState::Checkpoint
                {
                    self.inner_state = TimerState::Paused;
                    if let Some(start) = self.start_instant {
                        self.paused_elapsed += start.elapsed();
                    }
                    self.start_instant = None;
                    self.payload.status_text = "已暂停".into();
                }
            }
            TimerCommand::Reset => {
                self.inner_state = TimerState::Idle;
                self.start_instant = None;
                self.paused_elapsed = Duration::ZERO;
                self.last_valid_ocr = None;
                self.last_valid_instant = None;
                self.consecutive_rejects = 0;
                self.ocr_attempts = 0;
                self.ocr_successes = 0;
                self.payload = MissionTimerPayload::default();
            }
            TimerCommand::SetMode(mode) => {
                self.last_mode = mode.clone();
                self.payload.mode = mode.clone();
            }
            TimerCommand::SingleCapture => {
                // no-op – handled in polling loop
            }
        }
        self.payload.state = state_str(self.inner_state);
        self.payload.detection_rate = self.detection_rate();
    }

    pub fn update_elapsed(&mut self) {
        let elapsed = if let Some(start) = self.start_instant {
            if self.inner_state == TimerState::Running {
                start.elapsed() + self.paused_elapsed
            } else {
                self.paused_elapsed
            }
        } else {
            self.paused_elapsed
        };

        self.payload.elapsed_secs = elapsed.as_secs() as u32;
        let m = self.payload.elapsed_secs / 60;
        let s = self.payload.elapsed_secs % 60;
        self.payload.elapsed_str = format!("{}:{:02}", m, s);
        self.payload.detection_rate = self.detection_rate();
    }

    fn apply_ocr(
        &mut self,
        ocr_result: Option<String>,
        hwnd: isize,
        alert: &AlertParams,
        log_tx: &mpsc::Sender<String>,
    ) {
        // Auto-resume from checkpoint: new OCR value detected
        if self.inner_state == TimerState::Checkpoint {
            if let Some(ref result) = ocr_result {
                if Some(result) != self.last_valid_ocr.as_ref() {
                    let ocr_secs = parse_time_to_secs(result);
                    self.inner_state = TimerState::Running;
                    self.payload.state = "running".into();
                    self.payload.status_text = "运行中".into();
                    self.paused_elapsed = Duration::from_secs(ocr_secs as u64);
                    self.start_instant = Some(Instant::now());
                    self.last_valid_ocr = Some(result.clone());
                    self.last_valid_instant = Some(Instant::now());
                    log(log_tx, &format!("同步: {} (从截点恢复)", result));
                }
            }
            return;
        }

        if self.inner_state != TimerState::Running {
            return;
        }

        self.ocr_attempts += 1;

        if let Some(ref result) = ocr_result {
            let ocr_secs = parse_time_to_secs(result);

            let valid = if let (Some(last_ocr), Some(last_inst)) =
                (&self.last_valid_ocr, self.last_valid_instant)
            {
                let last_secs = parse_time_to_secs(last_ocr);
                let wall_delta = last_inst.elapsed().as_secs() as i64;
                let ocr_delta = ocr_secs as i64 - last_secs as i64;
                let diff = ocr_delta - wall_delta;
                (-10..=30).contains(&diff)
            } else {
                let wall = self.payload.elapsed_secs;
                (ocr_secs as i64 - wall as i64).abs() <= 60
            };

            if valid {
                self.ocr_successes += 1;
                self.consecutive_rejects = 0;
                log(log_tx, &format!("同步: {}", result));
                self.payload.ocr_raw = result.clone();

                // Hard sync: overwrite internal timer with OCR time
                self.paused_elapsed = Duration::from_secs(ocr_secs as u64);
                self.start_instant = Some(Instant::now());
                // Milestone in minutes for the reached bucket (5, 10, 15, …).
                let (checkpoint_crossed, milestone_min) = if let Some(last_ocr) = &self.last_valid_ocr {
                    let last_secs = parse_time_to_secs(last_ocr);
                    let last_bucket = last_secs / CHECKPOINT_INTERVAL_SECS;
                    let current_bucket = ocr_secs / CHECKPOINT_INTERVAL_SECS;
                    (
                        current_bucket > last_bucket && current_bucket > 0,
                        current_bucket * (CHECKPOINT_INTERVAL_SECS / 60),
                    )
                } else {
                    (false, 0)
                };

                self.last_valid_ocr = Some(result.clone());
                self.last_valid_instant = Some(Instant::now());
                if checkpoint_crossed {
                    self.inner_state = TimerState::Checkpoint;
                    self.payload.state = "checkpoint".into();
                    self.payload.status_text =
                        format!("{}分钟节点 — 请切回游戏", milestone_min);
                    log(log_tx, &format!("⚠ {}分钟节点: {}", milestone_min, result));
                    if alert.checkpoint_enabled {
                        let body = render_alert_text(
                            alert.checkpoint_text,
                            "⚠ 到达 {min} 分钟节点 — 请切回游戏",
                            milestone_min,
                        );
                        dispatch_alert(body, hwnd, alert.method, alert.tx);
                    }
                    return;
                }
            } else {
                log(log_tx, &format!("拒绝: {} (跳变过大)", result));
                self.consecutive_rejects += 1;
                if self.consecutive_rejects >= MAX_REJECT {
                    // Accept current value as new baseline instead of resetting
                    self.ocr_successes += 1;
                    self.consecutive_rejects = 0;
                    self.paused_elapsed = Duration::from_secs(ocr_secs as u64);
                    self.start_instant = Some(Instant::now());
                    self.last_valid_ocr = Some(result.clone());
                    self.last_valid_instant = Some(Instant::now());
                    self.payload.ocr_raw = result.clone();
                    log(log_tx, &format!("⟳ 重置基准: {}", result));
                }
            }
        }

        self.payload.detection_rate = self.detection_rate();
    }
}

fn state_str(s: TimerState) -> String {
    match s {
        TimerState::Idle => "idle".into(),
        TimerState::Running => "running".into(),
        TimerState::Paused => "paused".into(),
        TimerState::Checkpoint => "checkpoint".into(),
    }
}

fn parse_time_to_secs(s: &str) -> u32 {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 2 {
        let m: u32 = parts[0].parse().unwrap_or(0);
        let sec: u32 = parts[1].parse().unwrap_or(0);
        m * 60 + sec
    } else {
        0
    }
}

fn log(tx: &mpsc::Sender<String>, msg: &str) {
    let now = Local::now().format("%H:%M:%S").to_string();
    let _ = tx.send(format!("[{}] {}", now, msg));
}

/// Apply a queued timer command to shared state (with logging). Returns true if
/// the command was a `SingleCapture` request, which the caller handles via its
/// own capture path. Shared by the main poll loop and the inner wait loop so the
/// command-handling logic lives in exactly one place.
fn apply_timer_command(
    cmd: &TimerCommand,
    shared: &Arc<std::sync::RwLock<MissionTimerState>>,
    log_tx: &mpsc::Sender<String>,
) -> bool {
    match cmd {
        TimerCommand::SingleCapture => return true,
        TimerCommand::Start => {
            log(log_tx, "计时已启动");
            shared.write().unwrap().handle_command(&TimerCommand::Start);
        }
        TimerCommand::Stop => {
            log(log_tx, "计时已暂停");
            shared.write().unwrap().handle_command(&TimerCommand::Stop);
        }
        TimerCommand::Reset => {
            log(log_tx, "计时已重置");
            shared.write().unwrap().handle_command(&TimerCommand::Reset);
        }
        TimerCommand::SetMode(mode) => {
            log(log_tx, &format!("模式切换: {}", mode));
            shared
                .write()
                .unwrap()
                .handle_command(&TimerCommand::SetMode(mode.clone()));
        }
    }
    false
}

/// Detect life support percentage from a separate ROI via HSV red detection.
/// High red pixel % = life support bar is red = danger (inverted: high value = danger).
pub fn detect_life_support(pixels: &[u8], width: u32, height: u32) -> f32 {
    let total = (width * height) as usize;
    if total == 0 {
        // Empty/failed capture — report safe (0% red), never fake a danger.
        return 0.0;
    }

    let mut red_pixels = 0usize;
    for chunk in pixels.chunks(3) {
        let b = chunk[0] as f32 / 255.0;
        let g = chunk[1] as f32 / 255.0;
        let r = chunk[2] as f32 / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let h = if delta < 1e-4 {
            0.0
        } else if (max - r).abs() < 1e-4 {
            // rem_euclid ensures non-negative result when g < b
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if (max - g).abs() < 1e-4 {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };

        let s = if max < 1e-4 { 0.0 } else { delta / max };
        let v = max;

        // Red hue spans 0–15° and the wraparound 345–360°
        let is_red = h <= 15.0 || h >= 345.0;
        if is_red && s > 0.31 && v > 0.47 {
            red_pixels += 1;
        }
    }

    let pct = red_pixels as f32 / total as f32 * 100.0;
    pct.min(100.0)
}

/// Resolve the target Warframe game window handle by scanning visible windows.
fn resolve_hwnd(config: &AppConfig) -> isize {
    let windows = crate::window::list_windows(&config.mission_timer.window_title);
    windows.first().map(|w| w.hwnd as isize).unwrap_or(0)
}

/// Spawn the OCR polling thread. Returns the command channel sender.
pub fn start_timer_thread(
    shared: Arc<std::sync::RwLock<MissionTimerState>>,
    config: Arc<std::sync::RwLock<AppConfig>>,
    templates: Arc<DigitTemplates>,
    log_tx: mpsc::Sender<String>,
    alert_tx: mpsc::Sender<AlertMsg>,
) -> mpsc::Sender<TimerCommand> {
    let (cmd_tx, rx) = mpsc::channel::<TimerCommand>();
    let thread_log_tx = log_tx.clone();

    std::thread::spawn(move || {
        let log_tx = thread_log_tx;
        let alert_tx = alert_tx;
        // Resolve hwnd at start
        let mut hwnd: isize = {
            let cfg = config.read().unwrap();
            resolve_hwnd(&cfg)
        };

        if hwnd == 0 {
            log(&log_tx, "未找到游戏窗口");
            shared.write().unwrap().payload.window_status = "未检测到游戏窗口".into();
        } else {
            log(&log_tx, &format!("检测到游戏窗口 (HWND={})", hwnd));
            shared.write().unwrap().payload.window_status = "检测到游戏窗口".into();
        }

        let mut was_minimized = false;
        let mut had_valid_window = hwnd != 0;
        let mut consecutive_capture_fails = 0u32;
        // Carries a SingleCapture request that arrived during the inner wait
        // loop over to the next iteration's capture path.
        let mut pending_single_capture = false;

        loop {
            // Re-resolve hwnd if the window was closed / handle invalidated
            if had_valid_window && hwnd != 0 && !crate::window::is_valid(hwnd) {
                log(&log_tx, "⚠ 窗口已关闭");
                shared.write().unwrap().payload.window_status = "未检测到游戏窗口".into();
                let cfg = config.read().unwrap();
                hwnd = resolve_hwnd(&cfg);
                if hwnd != 0 {
                    log(&log_tx, "窗口已重新获取");
                    shared.write().unwrap().payload.window_status = "检测到游戏窗口".into();
                    had_valid_window = true;
                } else {
                    had_valid_window = false;
                }
                was_minimized = false;
            }

            // Process any pending commands (non-blocking). Picks up a single
            // capture deferred from the previous iteration's wait loop.
            let mut do_single_capture = pending_single_capture;
            pending_single_capture = false;
            while let Ok(cmd) = rx.try_recv() {
                if apply_timer_command(&cmd, &shared, &log_tx) {
                    do_single_capture = true;
                }
            }

            // ── Single capture: capture once, OCR, log, don't modify state ──
            if do_single_capture && hwnd != 0 && !crate::window::is_minimized(hwnd) {
                let cfg = config.read().unwrap();
                let strip_frame = cfg.mission_timer.strip_frame;

                let mode = {
                    let state = shared.read().unwrap();
                    state.payload.mode.clone()
                };

                let roi = if mode == "fissure" {
                    &cfg.mission_timer.fissure_roi
                } else {
                    &cfg.mission_timer.normal_roi
                };

                let roi_config = ROIConfig {
                    x: roi.x,
                    y: roi.y,
                    w: roi.w,
                    h: roi.h,
                };

                let capture_result = if strip_frame {
                    capture::capture_roi_stripped(hwnd, &roi_config, strip_frame)
                } else {
                    capture::capture_roi(hwnd, &roi_config)
                };

                if let Some((pixels, w, h)) = capture_result {
                    let ocr_result = recognize_digits(&pixels, w, h, &templates, MATCH_THRESHOLD);
                    if let Some(ref result) = ocr_result {
                        log(&log_tx, &format!("OCR: {} (识别)", result));
                    } else {
                        log(&log_tx, "OCR: 无结果");
                    }
                }

                drop(cfg);
            }

            // ── Main OCR poll cycle (when window is available) ──
            if hwnd != 0 {
                let cfg = config.read().unwrap();
                let ocr_interval = cfg.mission_timer.ocr_interval_secs.clamp(1, 30) as u64;

                let is_minimized = crate::window::is_minimized(hwnd);
                if is_minimized && !was_minimized {
                    log(&log_tx, "⚠ 窗口已最小化，暂停捕获");
                }
                was_minimized = is_minimized;

                let (is_running, mode, checkpoint_enabled, hp_alert_enabled, alert_method, checkpoint_text, hp_text, strip_frame) = {
                    let state = shared.read().unwrap();
                    let running = state.inner_state == TimerState::Running
                        || state.inner_state == TimerState::Checkpoint;
                    (
                        running,
                        state.payload.mode.clone(),
                        cfg.mission_timer.checkpoint_auto_focus,
                        cfg.mission_timer.hp_alert_enabled,
                        cfg.mission_timer.alert_method.clone(),
                        cfg.mission_timer.checkpoint_alert_text.clone(),
                        cfg.mission_timer.hp_alert_text.clone(),
                        cfg.mission_timer.strip_frame,
                    )
                };

                if is_running && !is_minimized {
                    // ── Capture main timer ROI ──
                    let roi = if mode == "fissure" {
                        &cfg.mission_timer.fissure_roi
                    } else {
                        &cfg.mission_timer.normal_roi
                    };

                    let roi_config = ROIConfig {
                        x: roi.x,
                        y: roi.y,
                        w: roi.w,
                        h: roi.h,
                    };

                    let capture_result = if strip_frame {
                        capture::capture_roi_stripped(hwnd, &roi_config, strip_frame)
                    } else {
                        capture::capture_roi(hwnd, &roi_config)
                    };

                    if let Some((pixels, w, h)) = capture_result {
                        consecutive_capture_fails = 0;
                        let ocr_result =
                            recognize_digits(&pixels, w, h, &templates, MATCH_THRESHOLD);

                        // Log raw OCR result (even if it will be rejected)
                        if let Some(ref result) = ocr_result {
                            let state = shared.read().unwrap();
                            let dr = state.detection_rate();
                            if dr > 0.0 {
                                log(&log_tx, &format!("OCR: {} (识别率: {:.0}%)", result, dr));
                            } else {
                                log(&log_tx, &format!("OCR: {} (识别)", result));
                            }
                        }

                        let mut state = shared.write().unwrap();
                        state.apply_ocr(
                            ocr_result,
                            hwnd,
                            &AlertParams {
                                checkpoint_enabled,
                                checkpoint_text: &checkpoint_text,
                                method: &alert_method,
                                tx: &alert_tx,
                            },
                            &log_tx,
                        );
                    } else {
                        // Capture failed or returned a black frame.
                        consecutive_capture_fails += 1;
                    }

                    // ── Capture HP / life support ROI (dual ROI by mode) ──
                    let ls_roi = if mode == "fissure" {
                        &cfg.mission_timer.fissure_hp_roi
                    } else {
                        &cfg.mission_timer.life_support_roi
                    };

                    let ls_config = ROIConfig {
                        x: ls_roi.x,
                        y: ls_roi.y,
                        w: ls_roi.w,
                        h: ls_roi.h,
                    };

                    let ls_capture = if strip_frame {
                        capture::capture_roi_stripped(hwnd, &ls_config, strip_frame)
                    } else {
                        capture::capture_roi(hwnd, &ls_config)
                    };

                    if let Some((ls_pixels, ls_w, ls_h)) = ls_capture {
                        let red_pct = detect_life_support(&ls_pixels, ls_w, ls_h);
                        let is_critical = red_pct > LIFE_SUPPORT_RED_THRESHOLD;
                        log(&log_tx, &format!("维生红像素: {:.2}%", red_pct));
                        let mut state = shared.write().unwrap();
                        if is_critical {
                            state.payload.life_support_pct = 15.0;
                            state.payload.life_support_level = "danger".into();
                            if hp_alert_enabled && !state.hp_alert_triggered {
                                log(&log_tx, "🚨 维生系统≤20%！");
                                state.hp_alert_triggered = true;
                                let body = render_alert_text(
                                    &hp_text,
                                    "🚨 维生系统 ≤ 20% — 请补充维生胶囊",
                                    0,
                                );
                                dispatch_alert(body, hwnd, &alert_method, &alert_tx);
                            }
                        } else {
                            state.payload.life_support_pct = 0.0;
                            state.payload.life_support_level = "normal".into();
                            state.hp_alert_triggered = false;
                        }
                    }
                }

                drop(cfg);

                // Repeated capture failures while running usually mean the
                // window handle is stale (game restarted, HWND recycled) or the
                // window is no longer composited — force a re-scan.
                if consecutive_capture_fails >= CAPTURE_FAIL_RESCAN {
                    log(&log_tx, "⚠ 连续捕获失败，重新扫描窗口");
                    consecutive_capture_fails = 0;
                    let cfg = config.read().unwrap();
                    hwnd = resolve_hwnd(&cfg);
                    drop(cfg);
                    was_minimized = false;
                    if hwnd != 0 {
                        shared.write().unwrap().payload.window_status = "检测到游戏窗口".into();
                        had_valid_window = true;
                    } else {
                        shared.write().unwrap().payload.window_status = "未检测到游戏窗口".into();
                        had_valid_window = false;
                    }
                    continue;
                }

                // Short-interval polling so commands are responsive
                let deadline = Instant::now() + Duration::from_secs(ocr_interval);
                while Instant::now() < deadline {
                    let mut got_single = false;
                    while let Ok(cmd) = rx.try_recv() {
                        if apply_timer_command(&cmd, &shared, &log_tx) {
                            got_single = true;
                        }
                    }
                    // A single-capture request can't run mid-wait (capture path
                    // is above); defer it and end the wait early.
                    if got_single {
                        pending_single_capture = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            } else {
                // No target window — retry resolve every 2 seconds
                std::thread::sleep(Duration::from_secs(2));
                let cfg = config.read().unwrap();
                hwnd = resolve_hwnd(&cfg);
                if hwnd != 0 {
                    log(&log_tx, "找到游戏窗口");
                    shared.write().unwrap().payload.window_status = "检测到游戏窗口".into();
                    had_valid_window = true;
                }
            }
        }
    });

    cmd_tx
}
