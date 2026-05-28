use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
}

fn default_close_to_tray() -> bool { true }

impl Default for AppConfig {
    fn default() -> Self {
        Self { close_to_tray: true }
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
