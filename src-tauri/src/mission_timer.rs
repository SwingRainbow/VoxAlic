use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use crate::capture::{self, ROIConfig};
use crate::config::AppConfig;
use crate::models::MissionTimerPayload;
use crate::ocr::{DigitTemplates, recognize_digits};

const OCR_INTERVAL_MS: u64 = 2000;
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
        }
    }

    fn handle_command(&mut self, cmd: &TimerCommand) {
        match cmd {
            TimerCommand::Start => {
                if self.inner_state == TimerState::Idle
                    || self.inner_state == TimerState::Paused
                    || self.inner_state == TimerState::Checkpoint
                {
                    self.inner_state = TimerState::Running;
                    if self.start_instant.is_none() {
                        self.start_instant = Some(Instant::now());
                        self.paused_elapsed = Duration::ZERO;
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
                self.payload = MissionTimerPayload::default();
            }
            TimerCommand::SetMode(mode) => {
                self.last_mode = mode.clone();
                self.payload.mode = mode.clone();
            }
        }
        self.payload.state = state_str(self.inner_state);
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
    }

    fn apply_ocr(&mut self, ocr_result: Option<String>) {
        if self.inner_state != TimerState::Running {
            return;
        }

        if let Some(ref result) = ocr_result {
            let ocr_secs = parse_time_to_secs(result);

            let valid = if let (Some(last_ocr), Some(last_inst)) =
                (&self.last_valid_ocr, self.last_valid_instant)
            {
                let last_secs = parse_time_to_secs(last_ocr);
                let wall_delta = last_inst.elapsed().as_secs() as i64;
                let ocr_delta = ocr_secs as i64 - last_secs as i64;
                (ocr_delta - wall_delta).abs() <= 30
            } else {
                let wall = self.payload.elapsed_secs;
                (ocr_secs as i64 - wall as i64).abs() <= 60
            };

            if valid {
                self.consecutive_rejects = 0;
                // Checkpoint detection: OCR value unchanged for >30s
                if let (Some(last_ocr), Some(last_inst)) =
                    (&self.last_valid_ocr, self.last_valid_instant)
                {
                    if result == last_ocr && last_inst.elapsed().as_secs() > CHECKPOINT_STALE_SECS {
                        self.inner_state = TimerState::Checkpoint;
                        self.payload.state = "checkpoint".into();
                        self.payload.status_text = "5分钟截点 — 请切回游戏".into();
                        return;
                    }
                }
                self.last_valid_ocr = Some(result.clone());
                self.last_valid_instant = Some(Instant::now());
            } else {
                self.consecutive_rejects += 1;
                if self.consecutive_rejects >= MAX_REJECT {
                    self.inner_state = TimerState::Idle;
                    self.payload = MissionTimerPayload::default();
                }
            }
        }
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

/// Spawn the OCR polling thread. Returns command sender.
pub fn start_timer_thread(
    shared: Arc<std::sync::RwLock<MissionTimerState>>,
    config: Arc<std::sync::RwLock<AppConfig>>,
    templates: Arc<DigitTemplates>,
) -> mpsc::Sender<TimerCommand> {
    let (tx, rx) = mpsc::channel::<TimerCommand>();

    std::thread::spawn(move || {
        loop {
            // Process any pending commands (non-blocking)
            while let Ok(cmd) = rx.try_recv() {
                let mut state = shared.write().unwrap();
                state.handle_command(&cmd);
            }

            // OCR capture cycle
            {
                let state = shared.read().unwrap();
                if state.inner_state == TimerState::Running {
                    let mode = state.payload.mode.clone();
                    drop(state);

                    let cfg = config.read().unwrap();
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

                    // Capture main timer ROI
                    if let Some((pixels, w, h)) = capture::capture_roi(&roi_config) {
                        let ocr_result = recognize_digits(&pixels, w, h, &templates, MATCH_THRESHOLD);
                        let mut state = shared.write().unwrap();
                        state.apply_ocr(ocr_result);
                    }

                    // Capture life support ROI
                    let ls_roi = &cfg.mission_timer.life_support_roi;
                    let ls_config = ROIConfig {
                        x: ls_roi.x,
                        y: ls_roi.y,
                        w: ls_roi.w,
                        h: ls_roi.h,
                    };
                    if let Some((ls_pixels, ls_w, ls_h)) = capture::capture_roi(&ls_config) {
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
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(OCR_INTERVAL_MS));
        }
    });

    tx
}
