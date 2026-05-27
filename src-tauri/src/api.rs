use crate::models::{CycleInfo, Fissure};
use serde_json::Value;

// ── Constants ────────────────────────────────────────────────────────────────

const SEC_MS_THRESHOLD: i64 = 10_i64.pow(11);
const EXPIRY_WARN_MS: i64 = 300_000;
const API_URL: &str = "https://api.warframe.com/cdn/worldState.php";

const VALLIS_EPOCH: i64 = 1_541_837_628_000;
const VALLIS_CYCLE: i64 = 1_600_000;
const VALLIS_WARM: i64 = 400_000;

const DUVIRI_EPOCH: i64 = 1_675_000_000_000;
const DUVIRI_CYCLE: i64 = 7_200_000; // 7200 seconds

const PLAINS_DAY_LEN: i64 = 6_000_000; // 100 minutes
const PLAINS_NIGHT_LEN: i64 = 3_000_000; // 50 minutes
const PLAINS_CYCLE: i64 = PLAINS_DAY_LEN + PLAINS_NIGHT_LEN; // 150 minutes

// ═══════════════════════════════════════════════════════════════════════════════
// 1. TIME UTILITIES
// ═══════════════════════════════════════════════════════════════════════════════

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Convert seconds to milliseconds if needed (threshold 10^11)
fn to_ms(v: i64) -> i64 {
    if v < SEC_MS_THRESHOLD {
        v * 1000
    } else {
        v
    }
}

/// Parse a MongoDB extended JSON date value into a millisecond timestamp.
///
/// Handles:
/// - `{"$date": {"$numberLong": "1234567890123"}}`
/// - `{"$date": 1234567890}` (plain number, seconds or ms)
/// - plain numbers
fn get_ms(val: &Value) -> i64 {
    match val {
        Value::Number(n) => {
            to_ms(n.as_i64().unwrap_or(0))
        }
        Value::Object(obj) => {
            // Check for MongoDB $date wrapper
            if let Some(date_val) = obj.get("$date") {
                match date_val {
                    Value::Number(n) => to_ms(n.as_i64().unwrap_or(0)),
                    Value::Object(inner) => {
                        if let Some(num_long) = inner.get("$numberLong") {
                            if let Some(s) = num_long.as_str() {
                                return to_ms(s.parse::<i64>().unwrap_or(0));
                            }
                            if let Some(n) = num_long.as_i64() {
                                return to_ms(n);
                            }
                        }
                        if let Some(num_double) = inner.get("$numberDouble") {
                            if let Some(s) = num_double.as_str() {
                                return to_ms(s.parse::<i64>().unwrap_or(0));
                            }
                        }
                        0
                    }
                    Value::String(s) => to_ms(s.parse::<i64>().unwrap_or(0)),
                    _ => 0,
                }
            } else {
                0
            }
        }
        Value::String(s) => to_ms(s.parse::<i64>().unwrap_or(0)),
        _ => 0,
    }
}

/// Check if a mission is currently active: activation <= now < expiry
fn is_active(activation: &Value, expiry: &Value) -> bool {
    let now = now_ms();
    get_ms(activation) <= now && now < get_ms(expiry)
}

/// Format remaining milliseconds to a Chinese time string.
/// 0 or negative → "切换中"
/// Otherwise → "Xh XXm XXs" / "Xm XXs" / "Xs"
pub fn fmt_remain(ms: i64) -> String {
    if ms <= 0 {
        return "切换中".to_string();
    }
    let total_secs = (ms / 1000) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}h {:02}m {:02}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {:02}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. MAPPING TABLES
// ═══════════════════════════════════════════════════════════════════════════════

/// Translate tier key to Chinese label
pub fn tier_label(key: &str) -> String {
    match key {
        "VoidT1" => "古纪".to_string(),
        "VoidT2" => "前纪".to_string(),
        "VoidT3" => "中纪".to_string(),
        "VoidT4" => "后纪".to_string(),
        "VoidT5" => "安魂".to_string(),
        "VoidT6" => "全能".to_string(),
        _ => key.to_string(),
    }
}

/// Order for sorting tiers (lower = displayed first)
fn tier_order(key: &str) -> usize {
    match key {
        "VoidT1" => 1,
        "VoidT2" => 2,
        "VoidT3" => 3,
        "VoidT4" => 4,
        "VoidT5" => 5,
        "VoidT6" => 6,
        _ => 99,
    }
}

/// Translate mission type key to Chinese label
fn mission_type(key: &str) -> &str {
    match key {
        "MT_ARENA" => "竞技场",
        "MT_ARTIFACT" => "中断",
        "MT_ASSAULT" => "强袭",
        "MT_ASSASSINATION" => "刺杀",
        "MT_CAPTURE" => "捕获",
        "MT_CORRUPTION" => "虚空洪流",
        "MT_DEFENSE" => "防御",
        "MT_DISRUPTION" => "中断",
        "MT_EVACUATION" => "叛逃",
        "MT_EXCAVATE" => "挖掘",
        "MT_EXTERMINATION" => "歼灭",
        "MT_HIVE" => "清巢",
        "MT_INTEL" => "间谍",
        "MT_LANDSCAPE" => "自由探索",
        "MT_MOBILE_DEFENSE" => "移动防御",
        "MT_PVP" => "武形秘仪",
        "MT_RESCUE" => "救援",
        "MT_RETRIEVAL" => "劫持",
        "MT_SABOTAGE" => "破坏",
        "MT_SECTOR" => "黑暗地带",
        "MT_SURVIVAL" => "生存",
        "MT_TERRITORY" => "拦截",
        "MT_VOID_CASCADE" => "虚空覆涌",
        "MT_ASCENSION" => "Ascension",
        "MT_ALCHEMY" => "元素转换",
        "MT_ENDLESS_CAPTURE" => "Legacyte Harvest",
        "MT_DEFAULT" => "未知",
        _ => key,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. NODE LOOKUP  (100+ entries)
// ═══════════════════════════════════════════════════════════════════════════════

struct NodeInfo {
    name: &'static str,
    planet: &'static str,
    mission: &'static str,
}

#[allow(clippy::too_many_lines)]
fn node_lookup(node_key: &str) -> NodeInfo {
    match node_key {
        // ── Hubs & Relays ─────────────────────────────────────────────────
        "MercuryHUB" => NodeInfo { name: "Larunda Relay", planet: "水星", mission: "" },
        "SolarisUnitedHub1" => NodeInfo { name: "福尔图娜", planet: "金星", mission: "" },
        "CetusHub4" => NodeInfo { name: "希图斯", planet: "地球", mission: "" },
        "ToggleBootLevel" => NodeInfo { name: "漂泊者营地", planet: "地球", mission: "" },
        "EarthHUB" => NodeInfo { name: "Strata Relay", planet: "地球", mission: "" },
        "TradeHUB1" => NodeInfo { name: "Maroo的市集", planet: "火星", mission: "" },
        "DeimosHub" => NodeInfo { name: "殁世幽都", planet: "火卫二", mission: "" },
        "EntratiLabHub" => NodeInfo { name: "解剖圣所", planet: "火卫二", mission: "" },
        "SaturnHUB" => NodeInfo { name: "Kronia Relay", planet: "土星", mission: "" },
        "PlutoHUB" => NodeInfo { name: "Orcus Relay", planet: "冥王星", mission: "" },
        "ZarimanHub" => NodeInfo { name: "羽化之穹", planet: "扎里曼 10-0 号", mission: "" },

        // ── Junctions ─────────────────────────────────────────────────────
        "VenusToMercuryJunction" => NodeInfo { name: "Mercury Junction", planet: "金星", mission: "" },
        "EarthToMarsJunction" => NodeInfo { name: "火星接合点", planet: "地球", mission: "" },
        "EarthToVenusJunction" => NodeInfo { name: "Venus Junction", planet: "地球", mission: "" },
        "MarsToCeresJunction" => NodeInfo { name: "Ceres Junction", planet: "火星", mission: "" },
        "MarsToPhobosJunction" => NodeInfo { name: "Phobos Junction", planet: "火星", mission: "" },
        "CeresToJupiterJunction" => NodeInfo { name: "Jupiter Junction", planet: "火卫二", mission: "" },
        "JupiterToEuropaJunction" => NodeInfo { name: "Europa Junction", planet: "木星", mission: "" },
        "JupiterToSaturnJunction" => NodeInfo { name: "Saturn Junction", planet: "木星", mission: "" },
        "SaturnToUranusJunction" => NodeInfo { name: "Uranus Junction", planet: "土星", mission: "" },
        "UranusToNeptuneJunction" => NodeInfo { name: "Neptune Junction", planet: "天王星", mission: "" },
        "NeptuneToPlutoJunction" => NodeInfo { name: "Pluto Junction", planet: "海王星", mission: "" },
        "ErisToSednaJunction" => NodeInfo { name: "Sedna Junction", planet: "冥王星", mission: "" },
        "PlutoToErisJunction" => NodeInfo { name: "Eris Junction", planet: "冥王星", mission: "" },

        // ── 水星 (Mercury) ────────────────────────────────────────────────
        "SolNode119" => NodeInfo { name: "Caloris", planet: "水星", mission: "" },
        "SolNode226" => NodeInfo { name: "Pantheon", planet: "水星", mission: "" },
        "SolNode130" => NodeInfo { name: "Lares", planet: "水星", mission: "" },
        "SolNode94" => NodeInfo { name: "Apollodorus", planet: "水星", mission: "" },
        "SolNode224" => NodeInfo { name: "Odin", planet: "水星", mission: "" },
        "SolNode103" => NodeInfo { name: "M Prime", planet: "水星", mission: "" },
        "SolNode12" => NodeInfo { name: "Elion", planet: "水星", mission: "" },
        "SolNode108" => NodeInfo { name: "Tolstoj", planet: "水星", mission: "" },
        "SolNode28" => NodeInfo { name: "Terminus", planet: "水星", mission: "" },
        "SolNode223" => NodeInfo { name: "Boethius", planet: "水星", mission: "" },
        "SolNode225" => NodeInfo { name: "Suisei", planet: "水星", mission: "" },

        // ── 金星 (Venus) ──────────────────────────────────────────────────
        "SolNode128" => NodeInfo { name: "E Gate", planet: "金星", mission: "" },
        "SolNode23" => NodeInfo { name: "Cytherean", planet: "金星", mission: "" },
        "SolNode123" => NodeInfo { name: "V Prime", planet: "金星", mission: "" },
        "SolNode22" => NodeInfo { name: "Tessera", planet: "金星", mission: "" },
        "SolNode101" => NodeInfo { name: "Kiliken", planet: "金星", mission: "" },
        "SolNode902" => NodeInfo { name: "Montes", planet: "金星", mission: "" },
        "SolNode66" => NodeInfo { name: "Unda", planet: "金星", mission: "" },
        "SolNode107" => NodeInfo { name: "Venera", planet: "金星", mission: "" },
        "SolNode109" => NodeInfo { name: "Linea", planet: "金星", mission: "" },
        "SolNode2" => NodeInfo { name: "Aphrodite", planet: "金星", mission: "" },
        "SolNode61" => NodeInfo { name: "Ishtar", planet: "金星", mission: "" },
        "SolNode104" => NodeInfo { name: "Fossa", planet: "金星", mission: "" },
        "ClanNode1" => NodeInfo { name: "Malva", planet: "金星", mission: "" },
        "ClanNode0" => NodeInfo { name: "Romula", planet: "金星", mission: "" },
        "SolNode129" => NodeInfo { name: "奥布山谷", planet: "金星", mission: "" },
        "NokkoColony" => NodeInfo { name: "深矿", planet: "金星", mission: "" },

        // ── 地球 (Earth) ──────────────────────────────────────────────────
        "SolNode89" => NodeInfo { name: "Mariana", planet: "地球", mission: "" },
        "SolNode27" => NodeInfo { name: "E Prime", planet: "地球", mission: "" },
        "SolNode39" => NodeInfo { name: "Everest", planet: "地球", mission: "" },
        "SolNode85" => NodeInfo { name: "Gaia", planet: "地球", mission: "" },
        "SolNode903" => NodeInfo { name: "Erpo", planet: "地球", mission: "" },
        "SolNode26" => NodeInfo { name: "Lith", planet: "地球", mission: "" },
        "SolNode79" => NodeInfo { name: "Cambria", planet: "地球", mission: "" },
        "SolNode63" => NodeInfo { name: "Mantle", planet: "地球", mission: "" },
        "SolNode59" => NodeInfo { name: "Eurasia", planet: "地球", mission: "" },
        "SolNode15" => NodeInfo { name: "Pacific", planet: "地球", mission: "" },
        "SolNode75" => NodeInfo { name: "Cervantes", planet: "地球", mission: "" },
        "SolNode451" => NodeInfo { name: "萨娅的异象", planet: "地球", mission: "" },
        "ClanNode3" => NodeInfo { name: "Tikal", planet: "地球", mission: "" },
        "ClanNode2" => NodeInfo { name: "Coba", planet: "地球", mission: "" },
        "SolNode228" => NodeInfo { name: "夜灵平野", planet: "地球", mission: "" },
        "SolNode24" => NodeInfo { name: "Oro", planet: "地球", mission: "" },

        // ── 月球 (Moon) ───────────────────────────────────────────────────
        "SolNode304" => NodeInfo { name: "Copernicus", planet: "月球", mission: "" },
        "SolNode306" => NodeInfo { name: "Pavlov", planet: "月球", mission: "" },
        "SolNode302" => NodeInfo { name: "Tycho", planet: "月球", mission: "" },
        "SolNode305" => NodeInfo { name: "Stöfler", planet: "月球", mission: "" },
        "SolNode300" => NodeInfo { name: "Plato", planet: "月球", mission: "" },
        "SolNode307" => NodeInfo { name: "Zeipel", planet: "月球", mission: "" },
        "SolNode309" => NodeInfo { name: "Yuvarium", planet: "月球", mission: "" },
        "SolNode301" => NodeInfo { name: "Grimaldi", planet: "月球", mission: "" },
        "SolNode308" => NodeInfo { name: "Apollo", planet: "月球", mission: "" },
        "SolNode310" => NodeInfo { name: "Circulus", planet: "月球", mission: "" },

        // ── 火星 (Mars) ───────────────────────────────────────────────────
        "SolNode58" => NodeInfo { name: "Hellas", planet: "火星", mission: "" },
        "SolNode11" => NodeInfo { name: "Tharsis", planet: "火星", mission: "" },
        "SolNode106" => NodeInfo { name: "Alator", planet: "火星", mission: "" },
        "SolNode46" => NodeInfo { name: "Spear", planet: "火星", mission: "" },
        "SolNode904" => NodeInfo { name: "Syrtis", planet: "火星", mission: "" },
        "SolNode113" => NodeInfo { name: "Ares", planet: "火星", mission: "" },
        "SolNode65" => NodeInfo { name: "Gradivus", planet: "火星", mission: "" },
        "SolNode41" => NodeInfo { name: "Arval", planet: "火星", mission: "" },
        "SolNode16" => NodeInfo { name: "Augustus", planet: "火星", mission: "" },
        "SolNode36" => NodeInfo { name: "Martialis", planet: "火星", mission: "" },
        "SolNode45" => NodeInfo { name: "Ara", planet: "火星", mission: "" },
        "ClanNode9" => NodeInfo { name: "Wahiba", planet: "火星", mission: "" },
        "ClanNode8" => NodeInfo { name: "Kadesh", planet: "火星", mission: "" },
        "SolNode68" => NodeInfo { name: "Vallis", planet: "火星", mission: "" },
        "SolNode99" => NodeInfo { name: "War", planet: "火星", mission: "" },
        "SolNode14" => NodeInfo { name: "Ultor", planet: "火星", mission: "" },
        "SolNode30" => NodeInfo { name: "Olympus", planet: "火星", mission: "" },
        "SolNode450" => NodeInfo { name: "Tyana Pass", planet: "火星", mission: "" },

        // ── 火卫一 (Phobos) ───────────────────────────────────────────────
        "SettlementNode1" => NodeInfo { name: "Roche", planet: "火卫一", mission: "" },
        "SettlementNode3" => NodeInfo { name: "Stickney", planet: "火卫一", mission: "" },
        "SettlementNode11" => NodeInfo { name: "Gulliver", planet: "火卫一", mission: "" },
        "SettlementNode14" => NodeInfo { name: "Shklovsky", planet: "火卫一", mission: "" },
        "SettlementNode15" => NodeInfo { name: "Sharpless", planet: "火卫一", mission: "" },
        "SettlementNode10" => NodeInfo { name: "Kepler", planet: "火卫一", mission: "" },
        "SettlementNode2" => NodeInfo { name: "Skyresh", planet: "火卫一", mission: "" },
        "SettlementNode12" => NodeInfo { name: "Monolith", planet: "火卫一", mission: "" },
        "SettlementNode20" => NodeInfo { name: "Iliad", planet: "火卫一", mission: "" },
        "ClanNode11" => NodeInfo { name: "Zeugma", planet: "火卫一", mission: "" },
        "ClanNode10" => NodeInfo { name: "Memphis", planet: "火卫一", mission: "" },

        // ── 火卫二 (Deimos) ───────────────────────────────────────────────
        "SolNode706" => NodeInfo { name: "Horend", planet: "火卫二", mission: "" },
        "SolNode708" => NodeInfo { name: "Phlegyas", planet: "火卫二", mission: "" },
        "SolNode710" => NodeInfo { name: "Formido", planet: "火卫二", mission: "" },
        "SolNode709" => NodeInfo { name: "Dirus", planet: "火卫二", mission: "" },
        "SolNode707" => NodeInfo { name: "Hyf", planet: "火卫二", mission: "" },
        "SolNode712" => NodeInfo { name: "Magnacidium", planet: "火卫二", mission: "" },
        "SolNode229" => NodeInfo { name: "魔胎之境", planet: "火卫二", mission: "" },
        "SolNode711" => NodeInfo { name: "Terrorem", planet: "火卫二", mission: "" },
        "SolNode713" => NodeInfo { name: "Exequias", planet: "火卫二", mission: "" },
        "SolNode721" => NodeInfo { name: "卫城区", planet: "火卫二", mission: "" },
        "SolNode716" => NodeInfo { name: "孽杀", planet: "火卫二", mission: "" },
        "SolNode719" => NodeInfo { name: "墓垒", planet: "火卫二", mission: "" },
        "SolNode715" => NodeInfo { name: "恶涌", planet: "火卫二", mission: "" },
        "SolNode718" => NodeInfo { name: "异化区", planet: "火卫二", mission: "" },
        "SolNode717" => NodeInfo { name: "不灭之地", planet: "火卫二", mission: "" },
        "SolNode720" => NodeInfo { name: "弧冢", planet: "火卫二", mission: "" },

        // ── 谷神星 (Ceres) ────────────────────────────────────────────────
        "SolNode132" => NodeInfo { name: "Bode", planet: "谷神星", mission: "" },
        "SolNode131" => NodeInfo { name: "Pallas", planet: "谷神星", mission: "" },
        "SolNode149" => NodeInfo { name: "Casta", planet: "谷神星", mission: "" },
        "SolNode147" => NodeInfo { name: "Cinxia", planet: "谷神星", mission: "" },
        "SolNode146" => NodeInfo { name: "Draco", planet: "谷神星", mission: "" },
        "SolNode137" => NodeInfo { name: "Nuovo", planet: "谷神星", mission: "" },
        "SolNode140" => NodeInfo { name: "Kiste", planet: "谷神星", mission: "" },
        "SolNode144" => NodeInfo { name: "Exta", planet: "谷神星", mission: "" },
        "SolNode141" => NodeInfo { name: "Ker", planet: "谷神星", mission: "" },
        "SolNode139" => NodeInfo { name: "Lex", planet: "谷神星", mission: "" },
        "SolNode138" => NodeInfo { name: "Ludi", planet: "谷神星", mission: "" },
        "SolNode135" => NodeInfo { name: "Thon", planet: "谷神星", mission: "" },
        "ClanNode22" => NodeInfo { name: "Seimeni", planet: "谷神星", mission: "" },
        "ClanNode23" => NodeInfo { name: "Gabii", planet: "谷神星", mission: "" },

        // ── 木星 (Jupiter) ────────────────────────────────────────────────
        "SolNode126" => NodeInfo { name: "Metis", planet: "木星", mission: "" },
        "SolNode905" => NodeInfo { name: "Galilea", planet: "木星", mission: "" },
        "SolNode100" => NodeInfo { name: "Elara", planet: "木星", mission: "" },
        "SolNode25" => NodeInfo { name: "Callisto", planet: "木星", mission: "" },
        "SolNode125" => NodeInfo { name: "Io", planet: "木星", mission: "" },
        "SolNode73" => NodeInfo { name: "Ananke", planet: "木星", mission: "" },
        "SolNode74" => NodeInfo { name: "Carme", planet: "木星", mission: "" },
        "SolNode121" => NodeInfo { name: "Carpo", planet: "木星", mission: "" },
        "SolNode97" => NodeInfo { name: "Amalthea", planet: "木星", mission: "" },
        "SolNode10" => NodeInfo { name: "Thebe", planet: "木星", mission: "" },
        "SolNode53" => NodeInfo { name: "Themisto", planet: "木星", mission: "" },
        "SolNode88" => NodeInfo { name: "Adrastea", planet: "木星", mission: "" },
        "ClanNode5" => NodeInfo { name: "Cameria", planet: "木星", mission: "" },
        "ClanNode4" => NodeInfo { name: "Sinai", planet: "木星", mission: "" },
        "SolNode87" => NodeInfo { name: "Ganymede", planet: "木星", mission: "" },
        "SolNode740" => NodeInfo { name: "蝠力使", planet: "木星", mission: "" },

        // ── 欧罗巴 (Europa) ───────────────────────────────────────────────
        "SolNode209" => NodeInfo { name: "Morax", planet: "欧罗巴", mission: "" },
        "SolNode215" => NodeInfo { name: "Valac", planet: "欧罗巴", mission: "" },
        "SolNode204" => NodeInfo { name: "Armaros", planet: "欧罗巴", mission: "" },
        "SolNode212" => NodeInfo { name: "Paimon", planet: "欧罗巴", mission: "" },
        "SolNode216" => NodeInfo { name: "Valefor", planet: "欧罗巴", mission: "" },
        "SolNode211" => NodeInfo { name: "Ose", planet: "欧罗巴", mission: "" },
        "SolNode214" => NodeInfo { name: "Sorath", planet: "欧罗巴", mission: "" },
        "SolNode220" => NodeInfo { name: "Kokabiel", planet: "欧罗巴", mission: "" },
        "SolNode217" => NodeInfo { name: "Orias", planet: "欧罗巴", mission: "" },
        "SolNode205" => NodeInfo { name: "Baal", planet: "欧罗巴", mission: "" },
        "SolNode203" => NodeInfo { name: "Abaddon", planet: "欧罗巴", mission: "" },
        "SolNode210" => NodeInfo { name: "Naamah", planet: "欧罗巴", mission: "" },
        "ClanNode6" => NodeInfo { name: "Larzac", planet: "欧罗巴", mission: "" },
        "ClanNode7" => NodeInfo { name: "Cholistan", planet: "欧罗巴", mission: "" },
        "GrendelKeyBMissionName" => NodeInfo { name: "上古货船", planet: "欧罗巴", mission: "" },
        "GrendelKeyCMissionName" => NodeInfo { name: "KARISHH之矿", planet: "欧罗巴", mission: "" },
        "GrendelKeyAMissionName" => NodeInfo { name: "RIDDAH冰原", planet: "欧罗巴", mission: "" },

        // ── 土星 (Saturn) ─────────────────────────────────────────────────
        "PvpNode10" => NodeInfo { name: "歼夺", planet: "土星", mission: "" },
        "PvpNode9" => NodeInfo { name: "团队歼夺", planet: "土星", mission: "" },
        "PvpNode0" => NodeInfo { name: "夺取中枢", planet: "土星", mission: "" },
        "SolNode67" => NodeInfo { name: "Dione", planet: "土星", mission: "" },
        "SolNode906" => NodeInfo { name: "Pandora", planet: "土星", mission: "" },
        "SolNode70" => NodeInfo { name: "Cassini", planet: "土星", mission: "" },
        "SolNode96" => NodeInfo { name: "Titan", planet: "土星", mission: "" },
        "SolNode42" => NodeInfo { name: "Helene", planet: "土星", mission: "" },
        "SolNode18" => NodeInfo { name: "Rhea", planet: "土星", mission: "" },
        "SolNode31" => NodeInfo { name: "Anthe", planet: "土星", mission: "" },
        "SolNode50" => NodeInfo { name: "Numa", planet: "土星", mission: "" },
        "SolNode20" => NodeInfo { name: "Telesto", planet: "土星", mission: "" },
        "SolNode19" => NodeInfo { name: "Enceladus", planet: "土星", mission: "" },
        "SolNode93" => NodeInfo { name: "Keeler", planet: "土星", mission: "" },
        "SolNode82" => NodeInfo { name: "Calypso", planet: "土星", mission: "" },
        "SolNode32" => NodeInfo { name: "Tethys", planet: "土星", mission: "" },
        "ClanNode13" => NodeInfo { name: "Piscinas", planet: "土星", mission: "" },
        "ClanNode12" => NodeInfo { name: "Caracol", planet: "土星", mission: "" },

        // ── 天王星 (Uranus) ───────────────────────────────────────────────
        "SolNode34" => NodeInfo { name: "Sycorax", planet: "天王星", mission: "" },
        "SolNode907" => NodeInfo { name: "Caelus", planet: "天王星", mission: "" },
        "SolNode69" => NodeInfo { name: "Ophelia", planet: "天王星", mission: "" },
        "SolNode64" => NodeInfo { name: "Umbriel", planet: "天王星", mission: "" },
        "SolNode122" => NodeInfo { name: "Stephano", planet: "天王星", mission: "" },
        "SolNode60" => NodeInfo { name: "Caliban", planet: "天王星", mission: "" },
        "SolNode33" => NodeInfo { name: "Ariel", planet: "天王星", mission: "" },
        "ClanNode17" => NodeInfo { name: "Assur", planet: "天王星", mission: "" },
        "SolNode98" => NodeInfo { name: "Desdemona", planet: "天王星", mission: "" },
        "SolNode83" => NodeInfo { name: "Cressida", planet: "天王星", mission: "" },
        "SolNode105" => NodeInfo { name: "Titania", planet: "天王星", mission: "" },
        "SolNode9" => NodeInfo { name: "Rosalind", planet: "天王星", mission: "" },
        "SolNode114" => NodeInfo { name: "Puck", planet: "天王星", mission: "" },
        "ClanNode16" => NodeInfo { name: "Ur", planet: "天王星", mission: "" },
        "SolNode723" => NodeInfo { name: "布鲁图斯", planet: "天王星", mission: "" },

        // ── 海王星 (Neptune) ──────────────────────────────────────────────
        "SolNode118" => NodeInfo { name: "Laomedeia", planet: "海王星", mission: "" },
        "SolNode1" => NodeInfo { name: "Galatea", planet: "海王星", mission: "" },
        "SolNode6" => NodeInfo { name: "Despina", planet: "海王星", mission: "" },
        "SolNode17" => NodeInfo { name: "Proteus", planet: "海王星", mission: "" },
        "SolNode908" => NodeInfo { name: "Salacia", planet: "海王星", mission: "" },
        "SolNode78" => NodeInfo { name: "Triton", planet: "海王星", mission: "" },
        "SolNode49" => NodeInfo { name: "Larissa", planet: "海王星", mission: "" },
        "SolNode57" => NodeInfo { name: "Sao", planet: "海王星", mission: "" },
        "SolNode62" => NodeInfo { name: "Neso", planet: "海王星", mission: "" },
        "EventNode763" => NodeInfo { name: "《指数之场》：挑战", planet: "海王星", mission: "" },
        "SolNode84" => NodeInfo { name: "Nereid", planet: "海王星", mission: "" },
        "SolNode127" => NodeInfo { name: "Psamathe", planet: "海王星", mission: "" },
        "ClanNode20" => NodeInfo { name: "Yursa", planet: "海王星", mission: "" },
        "ClanNode21" => NodeInfo { name: "Kelashin", planet: "海王星", mission: "" },

        // ── 冥王星 (Pluto) ────────────────────────────────────────────────
        "SolNode38" => NodeInfo { name: "Minthe", planet: "冥王星", mission: "" },
        "SolNode76" => NodeInfo { name: "Hydra", planet: "冥王星", mission: "" },
        "SolNode81" => NodeInfo { name: "Palus", planet: "冥王星", mission: "" },
        "SolNode72" => NodeInfo { name: "Outer Terminus", planet: "冥王星", mission: "" },
        "SolNode43" => NodeInfo { name: "Cerberus", planet: "冥王星", mission: "" },
        "SolNode21" => NodeInfo { name: "Narcissus", planet: "冥王星", mission: "" },
        "SolNode102" => NodeInfo { name: "Oceanum", planet: "冥王星", mission: "" },
        "SolNode4" => NodeInfo { name: "Acheron", planet: "冥王星", mission: "" },
        "SolNode56" => NodeInfo { name: "Cypress", planet: "冥王星", mission: "" },
        "SolNode48" => NodeInfo { name: "Regna", planet: "冥王星", mission: "" },
        "ClanNode25" => NodeInfo { name: "Hieracon", planet: "冥王星", mission: "" },
        "ClanNode24" => NodeInfo { name: "Sechura", planet: "冥王星", mission: "" },
        "SolNode51" => NodeInfo { name: "Hades", planet: "冥王星", mission: "" },

        // ── 赛德娜 (Sedna) ────────────────────────────────────────────────
        "SolNode189" => NodeInfo { name: "Naga", planet: "赛德娜", mission: "" },
        "SolNode195" => NodeInfo { name: "Hydron", planet: "赛德娜", mission: "" },
        "SolNode187" => NodeInfo { name: "Selkie", planet: "赛德娜", mission: "" },
        "SolNode185" => NodeInfo { name: "Berehynia", planet: "赛德娜", mission: "" },
        "SolNode184" => NodeInfo { name: "Rusalka", planet: "赛德娜", mission: "" },
        "SolNode181" => NodeInfo { name: "Adaro", planet: "赛德娜", mission: "" },
        "SolNode177" => NodeInfo { name: "Kappa", planet: "赛德娜", mission: "" },
        "SolNode191" => NodeInfo { name: "Marid", planet: "赛德娜", mission: "" },
        "SolNode196" => NodeInfo { name: "Charybdis", planet: "赛德娜", mission: "" },
        "SolNode188" => NodeInfo { name: "Kelpie", planet: "赛德娜", mission: "" },
        "SolNode193" => NodeInfo { name: "Merrow", planet: "赛德娜", mission: "" },
        "ClanNode15" => NodeInfo { name: "Sangeru", planet: "赛德娜", mission: "" },
        "ClanNode14" => NodeInfo { name: "Amarna", planet: "赛德娜", mission: "" },
        "SolNode190" => NodeInfo { name: "Nakki", planet: "赛德娜", mission: "" },
        "SolNode199" => NodeInfo { name: "Yam", planet: "赛德娜", mission: "" },
        "SolNode183" => NodeInfo { name: "Vodyanoi", planet: "赛德娜", mission: "" },

        // ── 赤毒要塞 (Kuva Fortress) ──────────────────────────────────────
        "SolNode746" => NodeInfo { name: "Dakata", planet: "赤毒要塞", mission: "" },
        "SolNode741" => NodeInfo { name: "Koro", planet: "赤毒要塞", mission: "" },
        "SolNode743" => NodeInfo { name: "Rotuma", planet: "赤毒要塞", mission: "" },
        "SolNode742" => NodeInfo { name: "Nabuk", planet: "赤毒要塞", mission: "" },
        "SolNode748" => NodeInfo { name: "Garus", planet: "赤毒要塞", mission: "" },
        "SolNode747" => NodeInfo { name: "Pago", planet: "赤毒要塞", mission: "" },
        "SolNode744" => NodeInfo { name: "Taveuni", planet: "赤毒要塞", mission: "" },
        "SolNode745" => NodeInfo { name: "Tamu", planet: "赤毒要塞", mission: "" },

        // ── 阋神星 (Eris) ─────────────────────────────────────────────────
        "SolNode175" => NodeInfo { name: "Naeglar", planet: "阋神星", mission: "" },
        "SolNode705" => NodeInfo { name: "异融Alad V刺杀", planet: "阋神星", mission: "" },
        "SolNode166" => NodeInfo { name: "Nimus", planet: "阋神星", mission: "" },
        "SolNode164" => NodeInfo { name: "Kala-azar", planet: "阋神星", mission: "" },
        "SolNode172" => NodeInfo { name: "Xini", planet: "阋神星", mission: "" },
        "SolNode701" => NodeInfo { name: "Jordas魔像", planet: "阋神星", mission: "" },
        "SolNode153" => NodeInfo { name: "Brugia", planet: "阋神星", mission: "" },
        "SolNode162" => NodeInfo { name: "Isos", planet: "阋神星", mission: "" },
        "SolNode167" => NodeInfo { name: "Oestrus", planet: "阋神星", mission: "" },
        "SolNode171" => NodeInfo { name: "Saxis", planet: "阋神星", mission: "" },
        "SolNode173" => NodeInfo { name: "Solium", planet: "阋神星", mission: "" },
        "ClanNode19" => NodeInfo { name: "Zabala", planet: "阋神星", mission: "" },
        "ClanNode18" => NodeInfo { name: "Akkad", planet: "阋神星", mission: "" },

        // ── 虚空 (Void) ───────────────────────────────────────────────────
        "SolNode402" => NodeInfo { name: "雷争塔", planet: "虚空", mission: "" },
        "SolNode400" => NodeInfo { name: "神王塔", planet: "虚空", mission: "" },
        "SolNode401" => NodeInfo { name: "神后塔", planet: "虚空", mission: "" },
        "SolNode404" => NodeInfo { name: "风王塔", planet: "虚空", mission: "" },
        "SolNode403" => NodeInfo { name: "法神塔", planet: "虚空", mission: "" },
        "SolNode405" => NodeInfo { name: "阿尼塔", planet: "虚空", mission: "" },
        "SolNode406" => NodeInfo { name: "乌戈塔", planet: "虚空", mission: "" },
        "SolNode408" => NodeInfo { name: "光神塔", planet: "虚空", mission: "" },
        "SolNode407" => NodeInfo { name: "母神塔", planet: "虚空", mission: "" },
        "SolNode410" => NodeInfo { name: "太阳塔", planet: "虚空", mission: "" },
        "SolNode412" => NodeInfo { name: "光理塔", planet: "虚空", mission: "" },
        "SolNode411" => NodeInfo { name: "守护神塔", planet: "虚空", mission: "" },
        "SolNode409" => NodeInfo { name: "死灵塔", planet: "虚空", mission: "" },

        // ── 扎里曼 (Zariman) ──────────────────────────────────────────────
        "SolNode234" => NodeInfo { name: "居住舱", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode238" => NodeInfo { name: "无尽回廊", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode236" => NodeInfo { name: "双衍历程", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode237" => NodeInfo { name: "孤独纪事", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode233" => NodeInfo { name: "奥金工场", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode231" => NodeInfo { name: "哈拉科防线", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode232" => NodeInfo { name: "涂沃主厅", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode235" => NodeInfo { name: "翠径", planet: "扎里曼 10-0 号", mission: "" },
        "SolNode230" => NodeInfo { name: "永视弧域", planet: "扎里曼 10-0 号", mission: "" },

        // ── 地球比邻星域 (Earth Proxima) ──────────────────────────────────
        "CrewBattleNode502" => NodeInfo { name: "深眠峡道", planet: "地球比邻星域", mission: "前哨战" },
        "CrewBattleNode509" => NodeInfo { name: "虚无神殿", planet: "地球比邻星域", mission: "前哨战" },
        "CrewBattleNode518" => NodeInfo { name: "奥加尔星团", planet: "地球比邻星域", mission: "前哨战" },
        "CrewBattleNode519" => NodeInfo { name: "克姆地带", planet: "地球比邻星域", mission: "前哨战" },
        "CrewBattleNode522" => NodeInfo { name: "本达尔星团", planet: "地球比邻星域", mission: "前哨战" },

        // ── 金星比邻星域 (Venus Proxima) ──────────────────────────────────
        "CrewBattleNode503" => NodeInfo { name: "虹桥回声", planet: "金星比邻星域", mission: "歼灭" },
        "CrewBattleNode511" => NodeInfo { name: "卫标星环", planet: "金星比邻星域", mission: "爆发" },
        "CrewBattleNode512" => NodeInfo { name: "欧文－哈克", planet: "金星比邻星域", mission: "间谍" },
        "CrewBattleNode513" => NodeInfo { name: "维斯珀峡道", planet: "金星比邻星域", mission: "奥菲斯" },
        "CrewBattleNode514" => NodeInfo { name: "落没之耀", planet: "金星比邻星域", mission: "防御" },
        "CrewBattleNode515" => NodeInfo { name: "无垠华盖", planet: "金星比邻星域", mission: "生存" },

        // ── 土星比邻星域 (Saturn Proxima) ─────────────────────────────────
        "CrewBattleNode501" => NodeInfo { name: "魔多星团", planet: "土星比邻星域", mission: "前哨战" },
        "CrewBattleNode530" => NodeInfo { name: "卡希欧安息处", planet: "土星比邻星域", mission: "前哨战" },
        "CrewBattleNode533" => NodeInfo { name: "诺朵星峡", planet: "土星比邻星域", mission: "前哨战" },
        "CrewBattleNode534" => NodeInfo { name: "卢帕星道", planet: "土星比邻星域", mission: "前哨战" },
        "CrewBattleNode535" => NodeInfo { name: "水域星团", planet: "土星比邻星域", mission: "前哨战" },

        // ── 海王星比邻星域 (Neptune Proxima) ──────────────────────────────
        "CrewBattleNode504" => NodeInfo { name: "时空坐标", planet: "海王星比邻星域", mission: "防御" },
        "CrewBattleNode516" => NodeInfo { name: "女娲之矿", planet: "海王星比邻星域", mission: "歼灭" },
        "CrewBattleNode521" => NodeInfo { name: "初裔冰渍区", planet: "海王星比邻星域", mission: "生存" },
        "CrewBattleNode523" => NodeInfo { name: "诱惑之景", planet: "海王星比邻星域", mission: "奥菲斯" },
        "CrewBattleNode524" => NodeInfo { name: "星主之握", planet: "海王星比邻星域", mission: "爆发" },
        "CrewBattleNode525" => NodeInfo { name: "薄暮星团", planet: "海王星比邻星域", mission: "间谍" },

        // ── 冥王星比邻星域 (Pluto Proxima) ────────────────────────────────
        "CrewBattleNode526" => NodeInfo { name: "胡夫之遣", planet: "冥王星比邻星域", mission: "奥菲斯" },
        "CrewBattleNode527" => NodeInfo { name: "七魅之息", planet: "冥王星比邻星域", mission: "歼灭" },
        "CrewBattleNode528" => NodeInfo { name: "冥渡", planet: "冥王星比邻星域", mission: "防御" },
        "CrewBattleNode529" => NodeInfo { name: "利益外缘", planet: "冥王星比邻星域", mission: "爆发" },
        "CrewBattleNode531" => NodeInfo { name: "芬顿之地", planet: "冥王星比邻星域", mission: "生存" },
        "CrewBattleNode536" => NodeInfo { name: "外域星轴", planet: "冥王星比邻星域", mission: "间谍" },

        // ── 面纱比邻星域 (Veil Proxima) ───────────────────────────────────
        "CrewBattleNode505" => NodeInfo { name: "Ruse 战场", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode510" => NodeInfo { name: "Gian 点", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode538" => NodeInfo { name: "蒲芦", planet: "面纱比邻星域", mission: "歼灭" },
        "CrewBattleNode539" => NodeInfo { name: "努秘", planet: "面纱比邻星域", mission: "爆发" },
        "CrewBattleNode540" => NodeInfo { name: "曲银之地", planet: "面纱比邻星域", mission: "防御" },
        "CrewBattleNode541" => NodeInfo { name: "深情之域", planet: "面纱比邻星域", mission: "奥菲斯" },
        "CrewBattleNode542" => NodeInfo { name: "鹿岩", planet: "面纱比邻星域", mission: "生存" },
        "CrewBattleNode543" => NodeInfo { name: "萨米尔星云", planet: "面纱比邻星域", mission: "间谍" },
        "CrewBattleNode550" => NodeInfo { name: "恩斯尤区格", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode551" => NodeInfo { name: "Ganalen 之墓", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode552" => NodeInfo { name: "Rya", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode553" => NodeInfo { name: "弗雷沙", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode554" => NodeInfo { name: "H-2 星云", planet: "面纱比邻星域", mission: "前哨战" },
        "CrewBattleNode555" => NodeInfo { name: "R-9 星云", planet: "面纱比邻星域", mission: "前哨战" },

        _ => NodeInfo { name: "", planet: "未知", mission: "" },
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. FISSURE PARSING
// ═══════════════════════════════════════════════════════════════════════════════

/// Parse a single fissure entry from JSON
fn parse_fissure(m: &Value, is_storm: bool) -> Fissure {
    let node_key = m["Node"].as_str().unwrap_or("").to_string();
    let info = node_lookup(&node_key);
    let node_name = if info.name.is_empty() { node_key.clone() } else { info.name.to_string() };
    let planet = info.planet.to_string();

    let mission_type_name = if is_storm {
        if info.mission.is_empty() {
            "--".to_string()
        } else {
            info.mission.to_string()
        }
    } else {
        let mission_type_key = m["MissionType"].as_str().unwrap_or("");
        mission_type(mission_type_key).to_string()
    };

    let tier_key = if is_storm {
        m["ActiveMissionTier"]
            .as_str()
            .unwrap_or("")
            .to_string()
    } else {
        m["Modifier"].as_str().unwrap_or("").to_string()
    };

    let tier_label_val = tier_label(&tier_key);

    let expiry_ms = get_ms(&m["Expiry"]);
    let remain_ms = expiry_ms - now_ms();
    let remain_str = fmt_remain(remain_ms);
    let is_expiring = remain_ms > 0 && remain_ms < EXPIRY_WARN_MS;

    Fissure {
        node_key,
        node_name: node_name.to_string(),
        planet: planet.to_string(),
        mission_type: mission_type_name,
        tier_key,
        tier_label: tier_label_val,
        expiry_ms,
        is_hard: false,
        is_storm,
        remain_ms,
        remain_str,
        is_expiring,
    }
}

/// Parse all fissures from worldstate data.
/// Returns (normal, hard, storm) vectors, each sorted by tier_order.
pub fn parse_fissures(data: &Value) -> (Vec<Fissure>, Vec<Fissure>, Vec<Fissure>) {
    let mut normal = Vec::new();
    let mut hard = Vec::new();
    let mut storm = Vec::new();

    // Parse ActiveMissions (normal + hard fissures)
    if let Some(missions) = data["ActiveMissions"].as_array() {
        for m in missions {
            let modifier = m["Modifier"].as_str().unwrap_or("");
            if !modifier.starts_with("VoidT") {
                continue;
            }
            if !is_active(&m["Activation"], &m["Expiry"]) {
                continue;
            }

            let mut fissure = parse_fissure(m, false);
            fissure.is_hard = m["Hard"]
                .as_bool()
                .unwrap_or(false);

            if fissure.is_hard {
                hard.push(fissure);
            } else {
                normal.push(fissure);
            }
        }
    }

    // Parse VoidStorms (railjack storms)
    if let Some(storms) = data["VoidStorms"].as_array() {
        for s in storms {
            if !is_active(&s["Activation"], &s["Expiry"]) {
                continue;
            }
            storm.push(parse_fissure(s, true));
        }
    }

    // Sort each vec by tier_order
    normal.sort_by_key(|f| tier_order(&f.tier_key));
    hard.sort_by_key(|f| tier_order(&f.tier_key));
    storm.sort_by_key(|f| tier_order(&f.tier_key));

    (normal, hard, storm)
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. CYCLE PARSING
// ═══════════════════════════════════════════════════════════════════════════════

/// Find an active syndicate mission entry by tag and return a reference to it.
fn find_active_syndicate<'a>(data: &'a Value, tag: &str) -> Option<&'a Value> {
    let arr = data["SyndicateMissions"].as_array()?;
    let now = now_ms();
    for entry in arr {
        if entry["Tag"].as_str() == Some(tag) {
            let activation = get_ms(&entry["Activation"]);
            let expiry = get_ms(&entry["Expiry"]);
            if activation <= now && now < expiry {
                return Some(entry);
            }
        }
    }
    None
}

/// Get expiry timestamp of the active syndicate mission with given tag.
/// Returns 0 if no active mission found.
#[allow(dead_code)]
fn active_syndicate_expiry(data: &Value, tag: &str) -> i64 {
    find_active_syndicate(data, tag)
        .map(|s| get_ms(&s["Expiry"]))
        .unwrap_or(0)
}

/// Build cycle info for Plains of Eidolon and Cambion Drift.
/// Both share HexSyndicate. Determines day/night via syndicate mission duration.
fn parse_plains_cycle(data: &Value, name: &str) -> CycleInfo {
    let now = now_ms();
    if let Some(synd) = find_active_syndicate(data, "HexSyndicate") {
        let activation = get_ms(&synd["Activation"]);
        let expiry = get_ms(&synd["Expiry"]);
        // Duration indicates phase: ~100min = day bounties, ~50min = night bounties
        let duration = expiry - activation;

        if duration > 5_400_000 {
            // Day bounties active (duration ~100 min), night approaching
            // Night starts 3 minutes before bounty expiry
            let night_start = expiry - 180_000;
            let remain = if now < night_start { night_start - now } else { 0 };
            let remain_str = fmt_remain(remain);
            CycleInfo {
                name: name.to_string(),
                state: "白天".to_string(),
                state_icon: "☀️".to_string(),
                remain_ms: remain,
                is_day: true,
                remain_str,
            }
        } else {
            // Night bounties active (duration ~50 min), day approaching
            let remain = expiry - now;
            let remain_str = fmt_remain(remain);
            CycleInfo {
                name: name.to_string(),
                state: "黑夜".to_string(),
                state_icon: "🌙".to_string(),
                remain_ms: remain,
                is_day: false,
                remain_str,
            }
        }
    } else {
        // Fallback: calculate based on fixed cycle from epoch
        let cycle_pos = (now - VALLIS_EPOCH) % PLAINS_CYCLE;
        if cycle_pos < PLAINS_DAY_LEN {
            let remain = PLAINS_DAY_LEN - cycle_pos;
            CycleInfo {
                name: name.to_string(),
                state: "白天".to_string(),
                state_icon: "☀️".to_string(),
                remain_ms: remain,
                is_day: true,
                remain_str: fmt_remain(remain),
            }
        } else {
            let remain = PLAINS_CYCLE - cycle_pos;
            CycleInfo {
                name: name.to_string(),
                state: "黑夜".to_string(),
                state_icon: "🌙".to_string(),
                remain_ms: remain,
                is_day: false,
                remain_str: fmt_remain(remain),
            }
        }
    }
}

/// Parse Orb Vallis (奥布山谷) cycle — hardcoded epoch.
/// Cycle: 400s warm (温度上升) + 1200s cold (极寒) = 1600s repeating.
fn parse_vallis_cycle() -> CycleInfo {
    let now = now_ms();
    let elapsed = (now - VALLIS_EPOCH) % VALLIS_CYCLE;

    if elapsed < VALLIS_WARM {
        // Warm phase — temperature rising
        let remain = VALLIS_WARM - elapsed;
        CycleInfo {
            name: "奥布山谷".to_string(),
            state: "温度上升".to_string(),
            state_icon: "🌡️".to_string(),
            remain_ms: remain,
            is_day: true,
            remain_str: fmt_remain(remain),
        }
    } else {
        // Cold phase — extreme cold
        let remain = VALLIS_CYCLE - elapsed;
        CycleInfo {
            name: "奥布山谷".to_string(),
            state: "极寒".to_string(),
            state_icon: "❄️".to_string(),
            remain_ms: remain,
            is_day: false,
            remain_str: fmt_remain(remain),
        }
    }
}

/// Parse Duviri (双衍王境) cycle — hardcoded 7200s cycle.
/// 5 moods: 悲惧喜怒妒 (Sorrow, Fear, Joy, Anger, Envy)
fn parse_duviri_cycle() -> CycleInfo {
    let now = now_ms();
    let elapsed = (now - DUVIRI_EPOCH) % DUVIRI_CYCLE;
    let mood_index = ((now - DUVIRI_EPOCH) / DUVIRI_CYCLE % 5) as usize;
    let moods = ["悲伤", "恐惧", "喜悦", "愤怒", "嫉妒"];
    let mood = moods.get(mood_index).unwrap_or(&"未知");
    let remain = DUVIRI_CYCLE - elapsed;

    CycleInfo {
        name: "双衍王境".to_string(),
        state: mood.to_string(),
        state_icon: "🌀".to_string(),
        remain_ms: remain,
        is_day: true,
        remain_str: fmt_remain(remain),
    }
}

/// Parse Zariman (扎里曼) cycle via ZarimanSyndicate.
fn parse_zariman_cycle(data: &Value) -> CycleInfo {
    let now = now_ms();
    if let Some(synd) = find_active_syndicate(data, "ZarimanSyndicate") {
        let expiry = get_ms(&synd["Expiry"]);
        let activation = get_ms(&synd["Activation"]);
        let duration = expiry - activation;

        // Zariman rotates between Grineer and Corpus control
        // Determine state by checking which faction is active
        let state = if duration > 1_800_000 {
            "Grineer占领".to_string()
        } else {
            "Corpus占领".to_string()
        };

        let remain = expiry - now;
        CycleInfo {
            name: "扎里曼".to_string(),
            state,
            state_icon: "🛡️".to_string(),
            remain_ms: remain,
            is_day: true,
            remain_str: fmt_remain(remain),
        }
    } else {
        CycleInfo {
            name: "扎里曼".to_string(),
            state: "未知".to_string(),
            state_icon: "🛡️".to_string(),
            remain_ms: 0,
            is_day: true,
            remain_str: "切换中".to_string(),
        }
    }
}

/// Parse all open-world cycles from worldstate data.
pub fn parse_cycles(data: &Value) -> Vec<CycleInfo> {
    let mut cycles = Vec::with_capacity(5);

    // 1. 夜灵平野 (Plains of Eidolon)
    cycles.push(parse_plains_cycle(data, "夜灵平野"));

    // 2. 魔胎之境 (Cambion Drift)
    cycles.push(parse_plains_cycle(data, "魔胎之境"));

    // 3. 奥布山谷 (Orb Vallis) — hardcoded
    cycles.push(parse_vallis_cycle());

    // 4. 双衍王境 (Duviri) — hardcoded
    cycles.push(parse_duviri_cycle());

    // 5. 扎里曼 (Zariman)
    cycles.push(parse_zariman_cycle(data));

    cycles
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. HTTP FETCHING
// ═══════════════════════════════════════════════════════════════════════════════

/// Fetch the Warframe worldstate JSON from the CDN.
pub async fn fetch_worldstate() -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .user_agent("Warframe/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(API_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: Value = resp.json().await.map_err(|e| e.to_string())?;

    Ok(json)
}
