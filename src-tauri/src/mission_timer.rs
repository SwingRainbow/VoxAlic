use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use chrono::Local;
use crate::capture::{self, ROIConfig};
use crate::config::AppConfig;
use crate::models::MissionTimerPayload;
use crate::ocr::{DigitTemplates, recognize_digits};

const MATCH_THRESHOLD: f32 = 0.55;
const MAX_REJECT: u32 = 3;
const CHECKPOINT_STALE_SECS: u64 = 30;
const DANGER_RED_PCT: f32 = 20.0;
const WARN_RED_PCT: f32 = 10.0;

#[derive(Debug, Clone)]
pub enum TimerCommand {
    Start,
    Stop,
    Reset,
    SetMode(String),
    SingleCapture,
}

pub struct TimerChannels {
    pub cmd: mpsc::Sender<TimerCommand>,
    pub log: mpsc::Sender<String>,
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
        }
    }

    pub fn detection_rate(&self) -> f32 {
        if self.ocr_attempts == 0 {
            0.0
        } else {
            self.ocr_successes as f32 / self.ocr_attempts as f32 * 100.0
        }
    }

    fn handle_command(&mut self, cmd: &TimerCommand) {
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
        checkpoint_auto_focus: bool,
        log_tx: &mpsc::Sender<String>,
    ) {
        // Auto-resume from checkpoint: new OCR value detected
        if self.inner_state == TimerState::Checkpoint {
            if let Some(ref result) = ocr_result {
                if Some(result) != self.last_valid_ocr.as_ref() {
                    // New OCR value detected – resume timer
                    self.inner_state = TimerState::Running;
                    self.payload.state = "running".into();
                    self.payload.status_text = "运行中".into();
                    self.start_instant = Some(Instant::now());
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
                diff >= -10 && diff <= 30
            } else {
                let wall = self.payload.elapsed_secs;
                (ocr_secs as i64 - wall as i64).abs() <= 60
            };

            if valid {
                self.ocr_successes += 1;
                self.consecutive_rejects = 0;
                log(log_tx, &format!("同步: {}", result));

                // Checkpoint detection: OCR value unchanged for >30s
                if let (Some(last_ocr), Some(last_inst)) =
                    (&self.last_valid_ocr, self.last_valid_instant)
                {
                    if result == last_ocr && last_inst.elapsed().as_secs() > CHECKPOINT_STALE_SECS {
                        self.inner_state = TimerState::Checkpoint;
                        self.payload.state = "checkpoint".into();
                        self.payload.status_text = "5分钟截点 — 请切回游戏".into();
                        log(log_tx, "⚠ 5分钟截点");
                        if checkpoint_auto_focus {
                            crate::window::bring_to_front(hwnd);
                        }
                        return;
                    }
                }

                self.last_valid_ocr = Some(result.clone());
                self.last_valid_instant = Some(Instant::now());
            } else {
                log(log_tx, &format!("拒绝: {} (跳变过大)", result));
                self.consecutive_rejects += 1;
                if self.consecutive_rejects >= MAX_REJECT {
                    self.inner_state = TimerState::Idle;
                    self.payload = MissionTimerPayload::default();
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

/// Detect life support percentage from a separate ROI via HSV red detection.
/// High red pixel % = life support bar is red = danger (inverted: high value = danger).
pub fn detect_life_support(pixels: &[u8], width: u32, height: u32) -> f32 {
    let total = (width * height) as usize;
    if total == 0 {
        return 100.0;
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
            60.0 * ((g - b) / delta % 6.0)
        } else if (max - g).abs() < 1e-4 {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };

        let s = if max < 1e-4 { 0.0 } else { delta / max };
        let v = max;

        let is_red = (h >= 0.0 && h <= 10.0) || (h >= 160.0 && h <= 180.0);
        if is_red && s > 0.4 && v > 0.3 {
            red_pixels += 1;
        }
    }

    let pct = red_pixels as f32 / total as f32 * 100.0;
    pct.min(100.0)
}

/// Resolve the target window handle from config.
fn resolve_hwnd(config: &AppConfig) -> isize {
    if config.mission_timer.selected_hwnd != 0 {
        config.mission_timer.selected_hwnd as isize
    } else {
        let windows = crate::window::list_windows(&config.mission_timer.window_title);
        windows.first().map(|w| w.hwnd as isize).unwrap_or(0)
    }
}

/// Spawn the OCR polling thread. Returns both command and log channels.
pub fn start_timer_thread(
    shared: Arc<std::sync::RwLock<MissionTimerState>>,
    config: Arc<std::sync::RwLock<AppConfig>>,
    templates: Arc<DigitTemplates>,
) -> TimerChannels {
    let (cmd_tx, rx) = mpsc::channel::<TimerCommand>();
    let (log_tx, _log_rx) = mpsc::channel::<String>();
    let thread_log_tx = log_tx.clone();

    std::thread::spawn(move || {
        let log_tx = thread_log_tx;
        // Resolve hwnd at start
        let mut hwnd: isize = {
            let cfg = config.read().unwrap();
            resolve_hwnd(&cfg)
        };

        if hwnd == 0 {
            log(&log_tx, "未找到游戏窗口");
        }

        let mut was_minimized = false;

        loop {
            // Re-resolve hwnd if the window was closed / handle invalidated
            if hwnd != 0 && !crate::window::is_valid(hwnd) {
                log(&log_tx, "⚠ 窗口已关闭");
                let cfg = config.read().unwrap();
                hwnd = resolve_hwnd(&cfg);
                if hwnd != 0 {
                    log(&log_tx, "窗口已重新获取");
                }
                was_minimized = false;
            }

            // Process any pending commands (non-blocking)
            let mut do_single_capture = false;
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    TimerCommand::SingleCapture => {
                        do_single_capture = true;
                    }
                    TimerCommand::Start => {
                        log(&log_tx, "计时已启动");
                        shared.write().unwrap().handle_command(&TimerCommand::Start);
                    }
                    TimerCommand::Stop => {
                        log(&log_tx, "计时已暂停");
                        shared.write().unwrap().handle_command(&TimerCommand::Stop);
                    }
                    TimerCommand::Reset => {
                        log(&log_tx, "计时已重置");
                        shared.write().unwrap().handle_command(&TimerCommand::Reset);
                    }
                    TimerCommand::SetMode(ref mode) => {
                        log(&log_tx, &format!("模式切换: {}", mode));
                        shared
                            .write()
                            .unwrap()
                            .handle_command(&TimerCommand::SetMode(mode.clone()));
                    }
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

                let (is_running, mode, checkpoint_auto_focus, hp_alert_enabled, strip_frame) = {
                    let state = shared.read().unwrap();
                    let running = state.inner_state == TimerState::Running
                        || state.inner_state == TimerState::Checkpoint;
                    (
                        running,
                        state.payload.mode.clone(),
                        cfg.mission_timer.checkpoint_auto_focus,
                        cfg.mission_timer.hp_alert_enabled,
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
                        state.apply_ocr(ocr_result, hwnd, checkpoint_auto_focus, &log_tx);
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
                        let ls_pct = detect_life_support(&ls_pixels, ls_w, ls_h);
                        let mut state = shared.write().unwrap();
                        state.payload.life_support_pct = ls_pct;
                        state.payload.life_support_level = if ls_pct > DANGER_RED_PCT {
                            "danger".into()
                        } else if ls_pct > WARN_RED_PCT {
                            "warning".into()
                        } else {
                            "normal".into()
                        };

                        // HP alert
                        if hp_alert_enabled && ls_pct > DANGER_RED_PCT {
                            log(&log_tx, "⚠ 维生≤20%");
                        }
                    }
                }

                drop(cfg);
                std::thread::sleep(Duration::from_secs(ocr_interval));
            } else {
                // No target window — sleep a bit then try to resolve again
                std::thread::sleep(Duration::from_secs(5));
                let cfg = config.read().unwrap();
                hwnd = resolve_hwnd(&cfg);
                if hwnd != 0 {
                    log(&log_tx, "找到游戏窗口");
                }
            }
        }
    });

    TimerChannels {
        cmd: cmd_tx,
        log: log_tx,
    }
}
