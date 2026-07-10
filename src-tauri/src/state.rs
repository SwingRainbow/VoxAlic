use std::sync::Arc;
use tokio::sync::RwLock;
use crate::models::{Fissure, CycleInfo, BaroInfo, BountyInfo, CircuitInfo, AppStatePayload};

pub struct AppState {
    pub normal_fissures: Vec<Fissure>,
    pub hard_fissures: Vec<Fissure>,
    pub storm_fissures: Vec<Fissure>,
    pub cycles: Vec<CycleInfo>,
    pub baro: Vec<BaroInfo>,
    pub bounties: Vec<BountyInfo>,
    pub circuit: Option<CircuitInfo>,
    pub last_update: String,
    pub countdown_secs: u32,
    /// False until the first worldstate fetch completes (success or failure).
    /// Until then, locally-computed data (arbitration) is suppressed so it
    /// doesn't render ahead of API-dependent panels.
    pub initialized: bool,
    /// Wall-clock timestamp (ms) of the last completed worldstate fetch.
    /// 0 before the first fetch — tick loop derives countdown as 0
    /// ("refresh overdue"), which is semantically correct at startup.
    pub last_fetch_wall_ms: i64,
    /// Cached payload rebuilt on each API fetch and mutated in-place on each
    /// tick — avoids cloning the full fissure/cycle/bounty vecs every second.
    pub cached_payload: AppStatePayload,
    /// Set to true when the tick loop detects Baro's arrival and triggers the
    /// auto-refresh task. Reset when Baro leaves, so the next arrival triggers
    /// again. Prevents spawning duplicate refresh tasks on every tick.
    pub baro_arrival_handled: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            normal_fissures: Vec::new(),
            hard_fissures: Vec::new(),
            storm_fissures: Vec::new(),
            cycles: Vec::new(),
            baro: Vec::new(),
            bounties: Vec::new(),
            circuit: None,
            last_update: String::new(),
            countdown_secs: 0,
            initialized: false,
            last_fetch_wall_ms: 0,
            cached_payload: AppStatePayload::default(),
            baro_arrival_handled: false,
        }
    }
}

impl Default for AppState {
    fn default() -> Self { Self::new() }
}

pub type SharedState = Arc<RwLock<AppState>>;
