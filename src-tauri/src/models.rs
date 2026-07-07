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

/// One item Baro is selling this rotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaroItem {
    pub name: String,      // 简中 name (WFCD i18n) if known, else English from the asset path
    pub ducats: i64,       // PrimePrice (杜卡德)
    pub credits: i64,      // RegularPrice (现金)
}

/// Void Trader (Baro Ki'Teer) state for the 世界时间 tab.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BaroInfo {
    pub active: bool,          // currently at the relay (Activation <= now < Expiry)
    pub location: String,      // relay name (e.g. "Larunda Relay / 水星")
    pub start_ms: i64,         // Activation timestamp
    pub end_ms: i64,           // Expiry timestamp
    pub remain_ms: i64,        // ms until arrival (if !active) or departure (if active)
    pub remain_str: String,
    pub items: Vec<BaroItem>,  // empty until Baro arrives
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
    #[serde(default)]
    pub ocr_raw: String,
    #[serde(default)]
    pub window_status: String,
}

impl Default for MissionTimerPayload {
    fn default() -> Self {
        Self {
            elapsed_secs: 0,
            elapsed_str: "0:00".into(),
            state: "idle".into(),
            mode: "normal".into(),
            life_support_pct: 0.0,
            life_support_level: "normal".into(),
            status_text: "未启动".into(),
            detection_rate: 0.0,
            ocr_raw: "--:--".into(),
            window_status: "未检测到游戏窗口".into(),
        }
    }
}

/// One possible reward in a bounty's drop pool (pre-translated to 简中).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardItem {
    pub name: String,       // 中文物品名（含数量，如 "50 内融核心"）
    pub rarity: String,     // Common / Uncommon / Rare / Legendary
    #[serde(default)]
    pub chance: f64,        // 掉率 %
}

/// One reward pool (rotation A/B/C). Pools rotate on each bounty refresh; only
/// one is active at a time. Items are sorted by rarity (common → rare).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardRotation {
    pub label: String,            // "A" / "B" / "C"
    pub items: Vec<RewardItem>,
}

/// One bounty (赏金) offered by an open-world syndicate this rotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyJob {
    pub title: String,      // 中文赏金标题（best-effort）
    #[serde(default)]
    pub desc: String,       // 节点 - 目标描述（如"奥金工场 - 作为指挥官击杀 10 名敌人"）
    pub name: String,       // 中文赏金类型（按 jobType 关键词推断）
    pub min_level: i64,     // minEnemyLevel
    pub max_level: i64,     // maxEnemyLevel
    pub mastery_req: i64,   // masteryReq
    pub stages: usize,      // xpAmounts.len()
    pub standing: i64,      // 总声望（xpAmounts 求和）
    pub tier: String,       // 奖励档位 "A".."E" / "Narmer"
    pub rotations: Vec<RewardRotation>, // 三个奖励池 A/B/C（轮换生效）
}

/// Open-world bounty board for one location (currently 夜灵平野/Cetus).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyInfo {
    pub syndicate: String,  // 地点名，板块标题用（如 "夜灵平野" / "解剖圣所"）
    pub card: String,       // 点开本面板的周期卡名（多数=syndicate；解剖圣所=魔胎之境，与火卫二同地点）
    pub expiry_ms: i64,     // 本轮赏金刷新时间
    pub remain_ms: i64,
    pub remain_str: String,
    pub active_rotation: String, // 当前生效轮次 "A"/"B"/"C"（来自 rewards 的 Table 字母）
    pub jobs: Vec<BountyJob>,
}

/// Duviri Circuit (双衍王境无限回廊) weekly **reward** rotation (NOT the playable
/// roster — that isn't in worldState). `normal` = 普通回廊可获战甲 (Warframe names,
/// kept English as the CN client doesn't translate them); `hard` = 钢铁之路回廊
/// 灵化之源 (the weapons whose Incarnon Genesis adapters drop, 简中). Weekly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CircuitInfo {
    pub normal: Vec<String>,   // 普通回廊·战甲奖励
    pub hard: Vec<String>,     // 钢铁之路回廊·灵化之源（Incarnon 武器）
    pub expiry_ms: i64,        // 本周轮换到期（每周一刷新）
    pub remain_ms: i64,
    pub remain_str: String,
}

/// One arbitration slot (current or upcoming).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArbitrationSlot {
    pub node: String,
    pub planet: String,
    pub mission: String,
    pub faction: String,
    pub min_level: i32,
    pub max_level: i32,
    pub archwing: bool,
}

/// Current Arbitration mission + next few slots (epoch-indexed from embedded schedule).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArbitrationInfo {
    pub current: ArbitrationSlot,
    pub upcoming: Vec<ArbitrationSlot>, // next 3 slots
    pub expiry_ms: i64,      // when current slot ends
    pub remain_ms: i64,
    pub remain_str: String,
    /// All distinct mission types across every arbitration node (not just the
    /// current rotation). Populated once so the alert-rule dropdown isn't
    /// limited to whatever happens to be in the next 4 hours.
    pub all_missions: Vec<String>,
    pub all_planets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppStatePayload {
    pub normal_fissures: Vec<Fissure>,
    pub hard_fissures: Vec<Fissure>,
    pub storm_fissures: Vec<Fissure>,
    pub cycles: Vec<CycleInfo>,
    pub last_update: String,
    pub countdown_secs: u32,
    pub mission_timer: MissionTimerPayload,
    #[serde(default)]
    pub baro: Option<BaroInfo>,
    #[serde(default)]
    pub bounties: Vec<BountyInfo>,
    #[serde(default)]
    pub circuit: Option<CircuitInfo>,
    #[serde(default)]
    pub arbitration: Option<ArbitrationInfo>,
}

// ── Warframe.Market ─────────────────────────────────────────────────────────

/// Search result row sent to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketItemSummary {
    pub slug: String,
    pub name: String,       // en
    #[serde(default)]
    pub name_zh: String,    // zh-hans (for translation display)
    pub icon_url: String,   // resolved full URL
    pub mr: Option<u8>,     // None = unknown
    #[serde(default)]
    pub max_rank: Option<u8>, // max mod/arcane rank
    pub tags: Vec<String>,
}

/// One buy or sell order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketOrder {
    pub order_type: String,  // "sell" | "buy"
    pub platinum: u32,
    pub quantity: u32,
    pub player_name: String,
    pub reputation: i32,
    pub status: String,      // "ingame" | "online" | "offline"
    #[serde(default)]
    pub mod_rank: Option<u8>, // rank of this mod/arcane listing
}

/// Detail panel payload (item info + orders).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketItemFull {
    pub item: MarketItemSummary,
    pub ducats: Option<u32>,
    pub trading_tax: Option<u32>,
    pub set_root: bool,
    #[serde(default)]
    pub set_parts: Vec<MarketItemSummary>,
    pub sell_orders: Vec<MarketOrder>,
    pub buy_orders: Vec<MarketOrder>,
}

/// Cache status sent to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketCacheStatus {
    pub count: usize,
    pub last_updated: String,
}
