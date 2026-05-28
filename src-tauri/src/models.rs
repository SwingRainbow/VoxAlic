use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fissure {
    pub node_key: String,
    pub node_name: String,
    pub planet: String,
    pub mission_type: String,
    pub tier_key: String,      // VoidT1~VoidT6
    pub tier_label: String,    // 古纪~全能
    pub expiry_ms: i64,
    pub is_hard: bool,
    pub is_storm: bool,
    pub remain_ms: i64,
    pub remain_str: String,
    pub is_expiring: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleInfo {
    pub name: String,        // 地点名
    pub state: String,        // 当前状态
    pub state_icon: String,   // emoji
    pub remain_ms: i64,
    pub is_day: bool,
    pub remain_str: String,   // like "1h 02m 30s"
    pub expiry_ms: i64,       // when this phase ends (ms timestamp)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTimerPayload {
    pub elapsed_secs: u32,
    pub elapsed_str: String,
    pub state: String,
    pub mode: String,
    pub life_support_pct: f32,
    pub life_support_level: String,
    pub status_text: String,
    #[serde(default)]
    pub detection_rate: f32,
}

impl Default for MissionTimerPayload {
    fn default() -> Self {
        Self {
            elapsed_secs: 0,
            elapsed_str: "00:00".into(),
            state: "idle".into(),
            mode: "normal".into(),
            life_support_pct: 0.0,
            life_support_level: "unknown".into(),
            status_text: "等待任务开始".into(),
            detection_rate: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatePayload {
    pub normal_fissures: Vec<Fissure>,
    pub hard_fissures: Vec<Fissure>,
    pub storm_fissures: Vec<Fissure>,
    pub cycles: Vec<CycleInfo>,
    pub last_update: String,
    pub countdown_secs: u32,
    pub mission_timer: MissionTimerPayload,
}
