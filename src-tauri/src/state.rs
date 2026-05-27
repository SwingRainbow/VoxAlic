use std::sync::Arc;
use tokio::sync::RwLock;
use crate::models::{Fissure, CycleInfo};

pub struct AppState {
    pub normal_fissures: Vec<Fissure>,
    pub hard_fissures: Vec<Fissure>,
    pub storm_fissures: Vec<Fissure>,
    pub cycles: Vec<CycleInfo>,
    pub last_update: String,
    pub countdown_secs: u32,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            normal_fissures: Vec::new(),
            hard_fissures: Vec::new(),
            storm_fissures: Vec::new(),
            cycles: Vec::new(),
            last_update: String::new(),
            countdown_secs: 0,
        }
    }
}

impl Default for AppState {
    fn default() -> Self { Self::new() }
}

pub type SharedState = Arc<RwLock<AppState>>;
