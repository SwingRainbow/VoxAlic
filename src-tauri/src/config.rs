use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROISettings {
    #[serde(default = "default_roi_x")]
    pub x: f64,
    #[serde(default = "default_roi_y")]
    pub y: f64,
    #[serde(default = "default_roi_w")]
    pub w: f64,
    #[serde(default = "default_roi_h")]
    pub h: f64,
}

fn default_roi_x() -> f64 {
    0.005
}
fn default_roi_y() -> f64 {
    normal_roi_y()
}
fn default_roi_w() -> f64 {
    0.07
}
fn default_roi_h() -> f64 {
    0.03
}

fn normal_roi_y() -> f64 {
    0.415
}
fn fissure_roi_y() -> f64 {
    0.46
}
fn fissure_roi_h() -> f64 {
    0.030
}
fn life_support_x() -> f64 {
    0.035
}
fn life_support_y() -> f64 {
    0.300
}
fn fissure_life_support_y() -> f64 {
    0.385
}
fn life_support_w() -> f64 {
    0.095
}
fn life_support_h() -> f64 {
    0.050
}

impl Default for ROISettings {
    fn default() -> Self {
        Self {
            x: default_roi_x(),
            y: default_roi_y(),
            w: default_roi_w(),
            h: default_roi_h(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTimerConfig {
    #[serde(default = "default_timer_mode")]
    pub mode: String,
    #[serde(default)]
    pub normal_roi: ROISettings,
    #[serde(default = "default_fissure_roi")]
    pub fissure_roi: ROISettings,
    #[serde(default = "default_life_support_roi")]
    pub life_support_roi: ROISettings,
    #[serde(default = "default_ocr_interval")]
    pub ocr_interval_secs: u32,
    #[serde(default = "default_true")]
    pub checkpoint_auto_focus: bool,
    #[serde(default = "default_true")]
    pub hp_alert_enabled: bool,
    /// How reminders are delivered: "focus" (force the game window to front)
    /// or "toast" (Windows notification). Shared by checkpoint + HP alerts.
    #[serde(default = "default_alert_method")]
    pub alert_method: String,
    /// Reminder text for the 5-minute checkpoint. `{min}` is substituted with
    /// the reached milestone (5, 10, 15, …). Empty falls back to the default.
    #[serde(default = "default_checkpoint_text")]
    pub checkpoint_alert_text: String,
    /// Reminder text for the 维生≤20% HP alert. Empty falls back to the default.
    #[serde(default = "default_hp_alert_text")]
    pub hp_alert_text: String,
    #[serde(default = "default_fissure_hp_roi")]
    pub fissure_hp_roi: ROISettings,
    #[serde(default = "default_true")]
    pub strip_frame: bool,
    #[serde(default)]
    pub selected_hwnd: usize,
    #[serde(default = "default_window_title")]
    pub window_title: String,
}

fn default_timer_mode() -> String {
    "normal".into()
}
fn default_fissure_roi() -> ROISettings {
    ROISettings {
        y: fissure_roi_y(),
        h: fissure_roi_h(),
        ..Default::default()
    }
}
fn default_life_support_roi() -> ROISettings {
    ROISettings {
        x: life_support_x(),
        y: life_support_y(),
        w: life_support_w(),
        h: life_support_h(),
    }
}

fn default_ocr_interval() -> u32 {
    2
}
fn default_true() -> bool {
    true
}
fn default_fissure_hp_roi() -> ROISettings {
    ROISettings {
        x: life_support_x(),
        y: fissure_life_support_y(),
        w: life_support_w(),
        h: life_support_h(),
    }
}
fn default_window_title() -> String {
    "Warframe".into()
}
fn default_alert_method() -> String {
    "focus".into()
}
pub fn default_checkpoint_text() -> String {
    "⚠ 到达 {min} 分钟节点 — 请切回游戏".into()
}
pub fn default_hp_alert_text() -> String {
    "🚨 维生系统 ≤ 20% — 请补充维生胶囊".into()
}

impl Default for MissionTimerConfig {
    fn default() -> Self {
        Self {
            mode: "normal".into(),
            normal_roi: ROISettings::default(),
            fissure_roi: default_fissure_roi(),
            life_support_roi: default_life_support_roi(),
            ocr_interval_secs: default_ocr_interval(),
            checkpoint_auto_focus: default_true(),
            hp_alert_enabled: default_true(),
            alert_method: default_alert_method(),
            checkpoint_alert_text: default_checkpoint_text(),
            hp_alert_text: default_hp_alert_text(),
            fissure_hp_roi: default_fissure_hp_roi(),
            strip_frame: default_true(),
            selected_hwnd: 0,
            window_title: default_window_title(),
        }
    }
}

/// One fissure subscription rule. Empty string = match any.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FissureAlert {
    #[serde(default)]
    pub tier: String,          // "古纪"|"前纪"|"中纪"|"后纪"|"安魂"|"全能"|""
    #[serde(default)]
    pub mission_type: String,  // "生存"|"防御"|...|""
    #[serde(default)]
    pub difficulty: String,    // "normal"|"hard"|"storm"|"" = any
}

/// One cycle subscription rule. Triggers when `location`'s `state` matches.
/// `advance_minutes`: fire N minutes BEFORE the state transition (0 = on transition).
/// Currently only effective for 夜灵平野 (Plains of Eidolon).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CycleAlert {
    #[serde(default)]
    pub location: String,  // CycleInfo.name
    #[serde(default)]
    pub state: String,     // CycleInfo.state value to match
    #[serde(default)]
    pub advance_minutes: u32,  // 0 = on transition, 5/10/15 = advance notice
}

/// One arbitration subscription rule. Both fields optional (empty = any).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArbitrationAlert {
    #[serde(default)]
    pub mission_type: String,  // ArbitrationSlot.mission, "" = any
    #[serde(default)]
    pub planet: String,        // ArbitrationSlot.planet (Chinese name), "" = any
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    #[serde(default = "default_worldstate_source")]
    pub worldstate_source: String,
    #[serde(default)]
    pub mission_timer: MissionTimerConfig,
    /// Fissure notification rules (toast when a matching fissure appears).
    #[serde(default)]
    pub fissure_alerts: Vec<FissureAlert>,
    /// Cycle-state notification rules (toast on state transition).
    #[serde(default)]
    pub cycle_alerts: Vec<CycleAlert>,
    /// Arbitration notification rules (toast when arbitration changes to a match).
    #[serde(default)]
    pub arbitration_alerts: Vec<ArbitrationAlert>,
    /// Bark push URL for phone notifications (empty = disabled).
    #[serde(default)]
    pub notify_bark_url: String,
    /// Update check source: "gitee" (fast in China) or "github".
    #[serde(default = "default_update_source")]
    pub update_source: String,
    /// Market display language: "en" or "zh".
    #[serde(default = "default_market_language")]
    pub market_language: String,
    /// Global hotkey string, e.g. "Alt+Shift+W". None = unset (disabled).
    #[serde(default)]
    pub hotkey: Option<String>,
}

fn default_update_source() -> String {
    "gitee".into()
}

fn default_market_language() -> String {
    "en".into()
}

fn default_close_to_tray() -> bool {
    true
}
fn default_worldstate_source() -> String {
    "official".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            worldstate_source: "official".into(),
            mission_timer: MissionTimerConfig::default(),
            fissure_alerts: Vec::new(),
            cycle_alerts: Vec::new(),
            arbitration_alerts: Vec::new(),
            notify_bark_url: String::new(),
            update_source: default_update_source(),
            market_language: default_market_language(),
            hotkey: None,
        }
    }
}

pub fn config_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("config.json")
}

pub fn load_config(app_data_dir: &PathBuf) -> AppConfig {
    let path = config_path(app_data_dir);
    if path.exists() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(mut cfg) = serde_json::from_str::<AppConfig>(&json) {
                if migrate_old_default_rois(&mut cfg) {
                    let _ = save_config(app_data_dir, &cfg);
                }
                return cfg;
            }
        }
    }
    let cfg = AppConfig::default();
    let _ = std::fs::create_dir_all(app_data_dir);
    if let Ok(json) = serde_json::to_string_pretty(&cfg) {
        let _ = std::fs::write(&path, json);
    }
    cfg
}

fn migrate_old_default_rois(cfg: &mut AppConfig) -> bool {
    let mut changed = false;
    let mt = &mut cfg.mission_timer;

    if roi_matches(&mt.normal_roi, 0.005, 0.395, 0.06, 0.025) {
        mt.normal_roi = ROISettings::default();
        changed = true;
    }
    if roi_matches(&mt.fissure_roi, 0.005, 0.465, 0.06, 0.025)
        || roi_matches(&mt.fissure_roi, 0.005, 0.46, 0.07, 0.075)
    {
        mt.fissure_roi = default_fissure_roi();
        changed = true;
    }
    if roi_matches(&mt.life_support_roi, 0.04, 0.305, 0.08, 0.04) {
        mt.life_support_roi = default_life_support_roi();
        changed = true;
    }
    if roi_matches(&mt.fissure_hp_roi, 0.04, 0.375, 0.08, 0.04) {
        mt.fissure_hp_roi = default_fissure_hp_roi();
        changed = true;
    }

    changed
}

fn roi_matches(roi: &ROISettings, x: f64, y: f64, w: f64, h: f64) -> bool {
    const EPS: f64 = 1e-6;
    (roi.x - x).abs() < EPS
        && (roi.y - y).abs() < EPS
        && (roi.w - w).abs() < EPS
        && (roi.h - h).abs() < EPS
}

pub fn save_config(app_data_dir: &PathBuf, config: &AppConfig) -> Result<(), String> {
    let _ = std::fs::create_dir_all(app_data_dir);
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(app_data_dir), json).map_err(|e| e.to_string())
}
