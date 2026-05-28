use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

fn default_roi_x() -> f64 { 0.005 }
fn default_roi_y() -> f64 { 0.395 }
fn default_roi_w() -> f64 { 0.06 }
fn default_roi_h() -> f64 { 0.025 }

fn fissure_roi_y() -> f64 { 0.405 }
fn life_support_y() -> f64 { 0.43 }
fn life_support_w() -> f64 { 0.15 }
fn life_support_h() -> f64 { 0.03 }

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
    #[serde(default = "default_fissure_hp_roi")]
    pub fissure_hp_roi: ROISettings,
    #[serde(default = "default_true")]
    pub strip_frame: bool,
    #[serde(default)]
    pub selected_hwnd: usize,
    #[serde(default = "default_window_title")]
    pub window_title: String,
}

fn default_timer_mode() -> String { "normal".into() }
fn default_fissure_roi() -> ROISettings {
    ROISettings { y: fissure_roi_y(), ..Default::default() }
}
fn default_life_support_roi() -> ROISettings {
    ROISettings { y: life_support_y(), w: life_support_w(), h: life_support_h(), ..Default::default() }
}

fn default_ocr_interval() -> u32 { 2 }
fn default_true() -> bool { true }
fn default_fissure_hp_roi() -> ROISettings {
    ROISettings { y: 0.45, w: 0.15, h: 0.03, ..Default::default() }
}
fn default_window_title() -> String { "Warframe".into() }

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
            fissure_hp_roi: default_fissure_hp_roi(),
            strip_frame: default_true(),
            selected_hwnd: 0,
            window_title: default_window_title(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub mission_timer: MissionTimerConfig,
}

fn default_close_to_tray() -> bool { true }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            mission_timer: MissionTimerConfig::default(),
        }
    }
}

pub fn config_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("config.json")
}

pub fn load_config(app_data_dir: &PathBuf) -> AppConfig {
    let path = config_path(app_data_dir);
    if path.exists() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(&json) {
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

pub fn save_config(app_data_dir: &PathBuf, config: &AppConfig) -> Result<(), String> {
    let _ = std::fs::create_dir_all(app_data_dir);
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(app_data_dir), json).map_err(|e| e.to_string())
}
