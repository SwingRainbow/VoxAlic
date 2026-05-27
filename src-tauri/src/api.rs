use crate::models::{ArbitrationInfo, ArbitrationSlot, BaroInfo, BaroItem, BountyInfo, BountyJob, CircuitInfo, CycleInfo, Fissure, RewardItem, RewardRotation};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

// ── Constants ────────────────────────────────────────────────────────────────

const SEC_MS_THRESHOLD: i64 = 10_i64.pow(11);
const EXPIRY_WARN_MS: i64 = 300_000;
const API_URL: &str = "https://api.warframe.com/cdn/worldState.php";

const VALLIS_EPOCH: i64 = 1_541_837_628_000;
const VALLIS_CYCLE: i64 = 1_600_000;
const VALLIS_WARM: i64 = 400_000;

const DUVIRI_CYCLE: i64 = 7_200_000; // 7200 seconds = 2h per mood

/// HexSyndicate / Zariman bounty rotation period — 150 min, confirmed from live
/// worldState (`duration 150.0min` for HexSyndicate & ZarimanSyndicate).
const HEX_CYCLE_MS: i64 = 9_000_000;

/// Anchor for Zariman faction parity: the activation (ms) of a known **Corpus**
/// 150-min window — 2026-06-02T07:08:00Z, verified against warframestat
/// `/pc/zarimanCycle` (`isCorpus:true`). The faction flips every window, so it
/// is derivable locally from any window's activation by parity against this.
const ZARIMAN_CORPUS_ANCHOR_MS: i64 = 1_780_384_080_000;

/// Whether the Zariman window starting at `activation_ms` is Corpus-controlled.
/// Faction alternates Grineer/Corpus each 150-min window; even parity from the
/// known-Corpus anchor = Corpus.
fn zariman_is_corpus(activation_ms: i64) -> bool {
    ((activation_ms - ZARIMAN_CORPUS_ANCHOR_MS) / HEX_CYCLE_MS).rem_euclid(2) == 0
}

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

/// Format Baro countdown with days in Chinese.
/// 0 or negative → "已离开"
/// ≥1 day   → "X天X小时X分钟X秒"
/// ≥1 hour  → "X小时X分钟X秒"
/// otherwise → "X分钟X秒"
pub fn fmt_remain_baro(ms: i64) -> String {
    if ms <= 0 {
        return "已离开".to_string();
    }
    fmt_dhms(ms)
}

/// Day-aware Chinese countdown for multi-day timers (e.g. the weekly Circuit).
/// 0 or negative → "刷新中".
pub fn fmt_remain_days(ms: i64) -> String {
    if ms <= 0 {
        return "刷新中".to_string();
    }
    fmt_dhms(ms)
}

/// Shared "X天X小时X分钟X秒" body (assumes `ms > 0`), trimming leading zero units.
fn fmt_dhms(ms: i64) -> String {
    let total_secs = (ms / 1000) as u64;
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if days > 0 {
        format!("{}天{}小时{}分钟{}秒", days, hours, minutes, seconds)
    } else if hours > 0 {
        format!("{}小时{}分钟{}秒", hours, minutes, seconds)
    } else {
        format!("{}分钟{}秒", minutes, seconds)
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

struct HexCycleDef {
    name: &'static str,
    day_state: &'static str,
    night_state: &'static str,
    day_icon: &'static str,
    night_icon: &'static str,
}

/// Build a HexSyndicate-based cycle. Night = 50 min before bounty expiry.
fn build_hex_cycle(def: &HexCycleDef, hex_expiry: i64) -> CycleInfo {
    let now = now_ms();
    let night_start = hex_expiry - 3_000_000;
    if now >= night_start {
        let remain = hex_expiry - now;
        CycleInfo {
            name: def.name.to_string(),
            state: def.night_state.to_string(),
            state_icon: def.night_icon.to_string(),
            remain_ms: remain,
            is_day: false,
            remain_str: fmt_remain(remain),
            expiry_ms: hex_expiry,
        }
    } else {
        let remain = night_start - now;
        CycleInfo {
            name: def.name.to_string(),
            state: def.day_state.to_string(),
            state_icon: def.day_icon.to_string(),
            remain_ms: remain,
            is_day: true,
            remain_str: fmt_remain(remain),
            expiry_ms: night_start,
        }
    }
}

/// Self-heal a syndicate-based cycle (Plains/Cambion/Zariman) that has run past
/// its stored phase-end, without waiting for the next 30-min API poll.
///
/// `CycleInfo::expiry_ms` holds the *current phase* end (for Hex day-phase that
/// is `night_start`, not the bounty expiry), so we first reconstruct the 150-min
/// bounty expiry, roll it forward by whole periods until it is in the future,
/// then rebuild via [`build_hex_cycle`] — exactly mirroring how Vallis/Duviri
/// recompute locally. Returns `None` for cycles we can't roll without the API.
pub fn roll_forward_cycle(c: &CycleInfo, now: i64) -> Option<CycleInfo> {
    match c.name.as_str() {
        "夜灵平野" | "魔胎之境" => {
            let def = if c.name == "夜灵平野" { &PLAINS_DEF } else { &CAMBION_DEF };
            // Day phase stored night_start (= bounty - 50min); night phase stored bounty.
            let mut bounty_expiry = if c.is_day { c.expiry_ms + 3_000_000 } else { c.expiry_ms };
            if bounty_expiry <= 0 {
                return None;
            }
            while bounty_expiry <= now {
                bounty_expiry += HEX_CYCLE_MS;
            }
            Some(build_hex_cycle(def, bounty_expiry))
        }
        "扎里曼" => {
            // Roll the 150-min window forward until its end is in the future,
            // then recompute the faction from the rolled window's activation
            // (= its end − one period), since windows are contiguous.
            let mut exp = c.expiry_ms;
            if exp <= 0 {
                return None;
            }
            while exp <= now {
                exp += HEX_CYCLE_MS;
            }
            let mut rolled = c.clone();
            rolled.state = if zariman_is_corpus(exp - HEX_CYCLE_MS) {
                "Corpus".to_string()
            } else {
                "Grineer".to_string()
            };
            rolled.expiry_ms = exp;
            rolled.remain_ms = exp - now;
            rolled.remain_str = fmt_remain(exp - now);
            Some(rolled)
        }
        "霍瓦尼亚" => {
            // No state to recompute — just roll the refresh window forward.
            let mut exp = c.expiry_ms;
            if exp <= 0 {
                return None;
            }
            while exp <= now {
                exp += HEX_CYCLE_MS;
            }
            let mut rolled = c.clone();
            rolled.expiry_ms = exp;
            rolled.remain_ms = exp - now;
            rolled.remain_str = fmt_remain(exp - now);
            Some(rolled)
        }
        _ => None,
    }
}

fn unknown_cycle(name: &str) -> CycleInfo {
    CycleInfo {
        name: name.to_string(),
        state: "未知".to_string(),
        state_icon: "☀️".to_string(),
        remain_ms: 0,
        is_day: true,
        remain_str: "切换中".to_string(),
        expiry_ms: now_ms(),
    }
}

/// Parse Orb Vallis (奥布山谷) cycle — hardcoded epoch.
/// Cycle: 400s warm (温度上升) + 1200s cold (极寒) = 1600s repeating.
pub fn parse_vallis_cycle() -> CycleInfo {
    let now = now_ms();
    let elapsed = (now - VALLIS_EPOCH) % VALLIS_CYCLE;

    if elapsed < VALLIS_WARM {
        // Warm phase — temperature rising
        let remain = VALLIS_WARM - elapsed;
        CycleInfo {
            name: "奥布山谷".to_string(),
            state: "温暖".to_string(),
            state_icon: "🌡️".to_string(),
            remain_ms: remain,
            is_day: true,
            remain_str: fmt_remain(remain),
            expiry_ms: now + remain,
        }
    } else {
        // Cold phase — extreme cold
        let remain = VALLIS_CYCLE - elapsed;
        CycleInfo {
            name: "奥布山谷".to_string(),
            state: "寒冷".to_string(),
            state_icon: "❄️".to_string(),
            remain_ms: remain,
            is_day: false,
            remain_str: fmt_remain(remain),
            expiry_ms: now + remain,
        }
    }
}

/// Parse Duviri (双衍王境) cycle — 7200s per mood, 5 moods rotating.
/// Mood index = absolute 2h blocks since Unix epoch (matches Python/game).
pub fn parse_duviri_cycle() -> CycleInfo {
    let now = now_ms();
    let mood_i = now / DUVIRI_CYCLE;
    let mood_end = (mood_i + 1) * DUVIRI_CYCLE;
    let moods = ["悲伤", "恐惧", "喜悦", "愤怒", "嫉妒"];
    let mood = moods.get((mood_i % 5) as usize).unwrap_or(&"未知");
    let remain = mood_end - now;

    CycleInfo {
        name: "双衍王境".to_string(),
        state: mood.to_string(),
        state_icon: "🌀".to_string(),
        remain_ms: remain,
        is_day: true,
        remain_str: fmt_remain(remain),
        expiry_ms: now + remain,
    }
}

/// Parse Zariman (扎里曼) cycle via ZarimanSyndicate.
fn parse_zariman_cycle(data: &Value) -> CycleInfo {
    let now = now_ms();
    if let Some(synd) = find_active_syndicate(data, "ZarimanSyndicate") {
        let expiry = get_ms(&synd["Expiry"]);
        let activation = get_ms(&synd["Activation"]);

        // Zariman alternates Grineer/Corpus control each 150-min window. The
        // worldState carries no faction flag, so derive it locally by parity of
        // the window's activation against a known-Corpus anchor. (The old
        // `duration > 30min` heuristic was always true → always "Grineer".)
        let state = if zariman_is_corpus(activation) {
            "Corpus".to_string()
        } else {
            "Grineer".to_string()
        };

        let remain = expiry - now;
        CycleInfo {
            name: "扎里曼".to_string(),
            state,
            state_icon: "🛡️".to_string(),
            remain_ms: remain,
            is_day: true,
            remain_str: fmt_remain(remain),
            expiry_ms: now + remain,
        }
    } else {
        CycleInfo {
            name: "扎里曼".to_string(),
            state: "未知".to_string(),
            state_icon: "🛡️".to_string(),
            remain_ms: 0,
            is_day: true,
            remain_str: "切换中".to_string(),
            expiry_ms: now,
        }
    }
}

const PLAINS_DEF: HexCycleDef = HexCycleDef {
    name: "夜灵平野", day_state: "白天", night_state: "黑夜",
    day_icon: "☀️", night_icon: "🌙",
};
const CAMBION_DEF: HexCycleDef = HexCycleDef {
    name: "魔胎之境", day_state: "Fass", night_state: "Vome",
    day_icon: "☀️", night_icon: "🌙",
};

/// Parse all open-world cycles from worldstate data.
pub fn parse_cycles(data: &Value) -> Vec<CycleInfo> {
    let mut cycles = Vec::with_capacity(5);

    // Plains + Cambion share HexSyndicate — lookup once
    let hex_expiry = find_active_syndicate(data, "HexSyndicate")
        .map(|s| get_ms(&s["Expiry"]));

    match hex_expiry {
        Some(exp) => {
            cycles.push(build_hex_cycle(&PLAINS_DEF, exp));
            cycles.push(build_hex_cycle(&CAMBION_DEF, exp));
        }
        None => {
            cycles.push(unknown_cycle("夜灵平野"));
            cycles.push(unknown_cycle("魔胎之境"));
        }
    }

    // 3. 奥布山谷 (Orb Vallis) — hardcoded epoch
    cycles.push(parse_vallis_cycle());

    // 4. 双衍王境 (Duviri) — hardcoded absolute 2h blocks
    cycles.push(parse_duviri_cycle());

    // 5. 扎里曼 (Zariman) — ZarimanSyndicate
    cycles.push(parse_zariman_cycle(data));

    // 6. 霍瓦尼亚 (Höllvania / 1999) — HexSyndicate bounty-refresh window
    cycles.push(parse_hex_cycle(data));

    cycles
}

/// Parse 霍瓦尼亚 (1999 / 六人组). There is no day/night gameplay cycle to track,
/// so the card simply surfaces the HexSyndicate bounty-refresh window (150-min,
/// shared boundary with the other syndicates) as the entry point for its board.
fn parse_hex_cycle(data: &Value) -> CycleInfo {
    match find_active_syndicate(data, "HexSyndicate").map(|s| get_ms(&s["Expiry"])) {
        Some(exp) => {
            let now = now_ms();
            CycleInfo {
                name: "霍瓦尼亚".to_string(),
                state: "六人组".to_string(),
                state_icon: "🌃".to_string(),
                remain_ms: exp - now,
                is_day: false,
                remain_str: fmt_remain(exp - now),
                expiry_ms: exp,
            }
        }
        None => unknown_cycle("霍瓦尼亚"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. VOID TRADER (BARO)
// ═══════════════════════════════════════════════════════════════════════════════

/// Turn an asset path into a readable English name.
/// `/Lotus/StoreItems/Weapons/Corpus/LongGuns/CrpShockRifle/QuantaVandal`
///   → last segment `QuantaVandal` → split CamelCase → `Quanta Vandal`.
/// Strips a leading `StoreItem` artefact and a trailing `Item`/`Blueprint`
/// noise word only when it leaves something behind.
fn name_from_path(path: &str) -> String {
    let seg = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path);

    // Split CamelCase / digit boundaries into spaced words.
    let mut out = String::new();
    let mut prev_lower = false;
    let mut prev_digit = false;
    for ch in seg.chars() {
        let is_upper = ch.is_ascii_uppercase();
        let is_digit = ch.is_ascii_digit();
        if !out.is_empty() && ((is_upper && prev_lower) || (is_digit != prev_digit && !out.ends_with(' '))) {
            out.push(' ');
        }
        out.push(ch);
        prev_lower = ch.is_ascii_lowercase();
        prev_digit = is_digit;
    }
    out.trim().to_string()
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6b. OPEN-WORLD BOUNTIES
// ═══════════════════════════════════════════════════════════════════════════════

/// Open-world bounty sources: (SyndicateMissions tag, 中文地点名 matching CycleInfo.name).
/// Cetus/Solaris/Entrati read live `Jobs`; Zariman is synthesised (empty Jobs).
const BOUNTY_SOURCES: &[(&str, &str)] = &[
    ("CetusSyndicate", "夜灵平野"),
    ("SolarisSyndicate", "奥布山谷"),
    ("ZarimanSyndicate", "扎里曼"),
    ("EntratiSyndicate", "魔胎之境"),
    ("HexSyndicate", "霍瓦尼亚"),
    ("EntratiLabSyndicate", "解剖圣所"),
];

/// The cycle card a bounty location is reached from. Usually the location itself,
/// but 解剖圣所 (Sanctum Anatomica) shares Cambion Drift in-game, so it hangs under
/// the 魔胎之境 card instead of getting its own (findings 续32/续40).
fn bounty_card(zh: &str) -> &str {
    match zh {
        "解剖圣所" => "魔胎之境",
        other => other,
    }
}

/// Map a bounty `jobType` asset path to a 中文 objective label by dominant keyword.
/// Best-effort: the raw worldState carries no display string, only the asset path.
fn bounty_type_zh(path: &str) -> String {
    const MAP: &[(&str, &str)] = &[
        ("Assassinate", "刺杀"),
        ("Capture", "捕获"),
        ("Rescue", "救援"),
        ("Sabotage", "破坏"),
        ("Hijack", "劫持"),
        ("Reclamation", "搜寻缓存"),
        ("Cache", "搜寻缓存"),
        ("Hunt", "狩猎"),
        ("Excavation", "挖掘"),
        ("Exterminate", "歼灭"),
        ("Attrition", "歼灭"),
        ("Landscape", "歼灭"),
        ("Defense", "防御"),
        ("Defend", "防御"),
        ("Spy", "间谍"),
        ("Theft", "夺取"),
        ("Resource", "采集"),
        ("Recovery", "回收"),
        ("Ambush", "伏击"),
    ];
    for (k, zh) in MAP {
        if path.contains(k) {
            return zh.to_string();
        }
    }
    "赏金任务".to_string()
}

/// Extract the currently-active rotation ("A"/"B"/"C") from a reward-table path
/// like `.../TierATableCRewards` — the letter after `Table`. All jobs in a
/// refresh share the same rotation; it advances each refresh (= each day/night
/// cycle). This is the only place the live rotation is exposed by worldState.
fn active_rotation_of(rewards: &str) -> String {
    rewards
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split("Table")
        .nth(1)
        .and_then(|s| s.chars().next())
        .filter(|c| ('A'..='C').contains(c))
        .map(|c| c.to_string())
        .unwrap_or_default()
}

/// Extract the reward tier ("A".."E" / "Narmer") from a reward-table asset path
/// like `.../EidolonJobMissionRewards/TierATableCRewards`.
fn reward_tier(path: &str) -> String {
    let seg = path.rsplit('/').next().unwrap_or("");
    if seg.contains("Narmer") {
        return "Narmer".to_string();
    }
    // Cetus uses `TierATable...`; Solaris uses `VenusTierATable...` — match the
    // `Tier` token wherever it appears and take the letter after it.
    seg.split("Tier")
        .nth(1)
        .and_then(|rest| rest.chars().next())
        .map(|c| c.to_string())
        .unwrap_or_default()
}

/// 中文 title for a bounty. Official titles live in DE's localization dict (not
/// the item i18n), so we hard-map the known Ostron `jobType` vocabulary by exact
/// last-segment (titles verified against the CN community board). Narmer variants
/// have their own distinct titles; the lvl-100 bracket gets a 钢铁之路 suffix.
fn bounty_title(tag: &str, job_type: &str, min_level: i64) -> String {
    // Zariman (坚守者) bounties: the live worldState carries no Jobs between
    // rotations, so titles are mapped by the stable level→mission pairing
    // (verified against 坚守者.png) rather than jobType. Steel-path variants run
    // at +100 levels (150-215); normalise to the base bracket and add the suffix.
    if tag == "ZarimanSyndicate" {
        let steel = min_level >= 150;
        let n = if steel { min_level - 100 } else { min_level };
        let base = match n {
            50 => "移动防御",
            60 => "虚空覆涌",
            70 => "虚空决战",
            90 => "虚空洪流",
            110 => "歼灭",
            _ => "坚守者赏金",
        };
        let mut title = base.to_string();
        if steel {
            title.push_str("（钢铁之路）");
        }
        return title;
    }
    // Höllvania / 1999 (六人组): worldState carries no Jobs (like Zariman). Unlike the
    // reward pool + level bracket (which ARE locked per bracket — verified across two
    // community-board snapshots), the mission TYPE / title rotates every 150-min
    // refresh (the same title↔level pairing differs between snapshots). Since our
    // worldState gives no Jobs, the live title is unknowable → keep it generic rather
    // than show a stale per-level guess. Level + reward pool already disambiguate.
    if tag == "HexSyndicate" {
        return "六人组赏金".to_string();
    }
    // 解剖圣所 (Cavia): empty Jobs → no jobType; mission type is seed-rotated and
    // unknowable (same as Hex). Generic title; level + reward pool disambiguate.
    if tag == "EntratiLabSyndicate" {
        return "解剖圣所赏金".to_string();
    }
    // Cambion Drift (魔胎之境): narrative titles mapped by jobType last-segment,
    // verified against 英择谛.png. Isolation-Vault jobs carry an empty jobType →
    // 隔离库. lvl-100 (重夺领地) gets the 钢铁之路 suffix.
    if tag == "EntratiSyndicate" {
        let seg = job_type.rsplit('/').next().unwrap_or("");
        let base = match seg {
            "DeimosAssassinateBounty" => "清净之地",
            "DeimosCrpSurvivorBounty" => "为了科学！",
            "DeimosEndlessPurifyBounty" => "古物猎人（无尽）",
            "DeimosGrnSurvivorBounty" => "蛮暴之力",
            "DeimosExcavateBounty" => "核心样本",
            "DeimosAreaDefenseBounty" => "重夺领地",
            "" => "隔离库",
            _ => "魔胎赏金",
        };
        let mut title = base.to_string();
        if min_level >= 100 {
            title.push_str("（钢铁之路）");
        }
        return title;
    }
    let narmer = job_type.contains("/Narmer/");
    let seg = job_type.rsplit('/').next().unwrap_or("");
    let base = if narmer {
        // Narmer bounties carry their own narrative titles regardless of objective.
        match seg {
            "AttritionBountyLib" => "带他们回家",
            "AssassinateBountyAss" => "大起大落",
            "NarmerVenusCullJobExterminate" => "粉碎邪教",
            _ => "",
        }
    } else {
        match seg {
            "AttritionBountyLib" => "削弱 Grineer 据点",
            "AttritionBountySab" => "破坏 Grineer 补给线",
            "AttritionBountyExt" => "宰杀敌人",
            "AssassinateBountyAss" => "刺杀指挥官",
            "RescueBountyResc" => "搜索并救援",
            "CaptureBountyCapOne" => "捕获 Grineer 指挥官",
            "CaptureBountyCapTwo" => "间谍捕手",
            "SabotageBountySab" => "破坏原型机",
            "ReclamationBountyCache" => "找出遗失的器物",
            "ReclamationBountyTheft" => "取回被偷的器物",
            "HuntBountyHunt" => "狩猎",
            // Orb Vallis (Solaris United) jobs — titles verified against 索拉里斯.png.
            "VenusCullJobExterminate" => "猎人杀手",
            "VenusWetworkJobAssassinate" => "冷餐",
            "VenusIntelJobRecovery" => "存活证明",
            "VenusArtifactJobAmbush" => "伏击信使",
            "VenusHelpingJobResource" => "尘土部队",
            "VenusChaosJobAssassinate" => "焦土大地",
            _ => "",
        }
    };
    let mut title = if !base.is_empty() {
        base.to_string()
    } else {
        // keyword fallback for unmapped types
        const KW: &[(&str, &str)] = &[
            ("Assassinate", "刺杀指挥官"),
            ("Capture", "捕获目标"),
            ("Rescue", "搜索并救援"),
            ("Sabotage", "破坏原型机"),
            ("Reclamation", "取回器物"),
            ("Cache", "搜寻隐藏物资"),
            ("Hunt", "狩猎"),
            ("Excavation", "发掘遗物"),
            ("Exterminate", "清剿据点"),
            ("Attrition", "削弱据点"),
            ("Theft", "夺取物资"),
            ("Spy", "窃取情报"),
            ("Defense", "保卫目标"),
        ];
        KW.iter()
            .find(|(k, _)| job_type.contains(k))
            .map(|(_, zh)| zh.to_string())
            .unwrap_or_else(|| "赏金任务".to_string())
    };
    if narmer {
        title.push_str("（合一众）");
    }
    if min_level >= 100 {
        title.push_str("（钢铁之路）");
    }
    title
}

/// Pre-translated Cetus bounty reward pools, embedded at compile time.
/// Shape: `{ "min-max": { "A": [RewardItem], "B": [...], "C": [...] } }`.
type RewardTable = HashMap<String, HashMap<String, Vec<RewardItem>>>;
static CETUS_REWARDS: OnceLock<RewardTable> = OnceLock::new();
fn cetus_rewards() -> &'static RewardTable {
    CETUS_REWARDS.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/cetus_bounty_rewards.json"))
            .unwrap_or_default()
    })
}

static SOLARIS_REWARDS: OnceLock<RewardTable> = OnceLock::new();
fn solaris_rewards() -> &'static RewardTable {
    SOLARIS_REWARDS.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/solaris_bounty_rewards.json"))
            .unwrap_or_default()
    })
}

static ZARIMAN_REWARDS: OnceLock<RewardTable> = OnceLock::new();
fn zariman_rewards() -> &'static RewardTable {
    ZARIMAN_REWARDS.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/zariman_bounty_rewards.json"))
            .unwrap_or_default()
    })
}

static DEIMOS_REWARDS: OnceLock<RewardTable> = OnceLock::new();
fn deimos_rewards() -> &'static RewardTable {
    DEIMOS_REWARDS.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/deimos_bounty_rewards.json"))
            .unwrap_or_default()
    })
}

static HEX_REWARDS: OnceLock<RewardTable> = OnceLock::new();
fn hex_rewards() -> &'static RewardTable {
    HEX_REWARDS.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/hex_bounty_rewards.json"))
            .unwrap_or_default()
    })
}

static ENTRATI_LAB_REWARDS: OnceLock<RewardTable> = OnceLock::new();
fn entrati_lab_rewards() -> &'static RewardTable {
    ENTRATI_LAB_REWARDS.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/entrati_lab_bounty_rewards.json"))
            .unwrap_or_default()
    })
}

/// Pick the embedded reward table for a syndicate tag.
fn rewards_for(tag: &str) -> &'static RewardTable {
    match tag {
        "SolarisSyndicate" => solaris_rewards(),
        "ZarimanSyndicate" => zariman_rewards(),
        "HexSyndicate" => hex_rewards(),
        "EntratiLabSyndicate" => entrati_lab_rewards(),
        _ => cetus_rewards(),
    }
}

fn rarity_rank(r: &str) -> u8 {
    match r {
        "Common" => 0,
        "Uncommon" => 1,
        "Rare" => 2,
        "Legendary" => 3,
        _ => 4,
    }
}

/// The three reward pools (rotations A/B/C) for a level range, each sorted by
/// rarity (common → rare) then descending chance. Pools are kept separate
/// because only one is active per bounty refresh.
fn reward_rotations(tag: &str, min: i64, max: i64) -> Vec<RewardRotation> {
    let table = rewards_for(tag);
    let key = format!("{min}-{max}");
    // Steel-path variants run at +100 levels (e.g. Zariman 150-155) and share the
    // base bracket's pool; fall back to the normalised key when no exact match.
    let rots = table.get(&key).or_else(|| {
        if min >= 150 {
            table.get(&format!("{}-{}", min - 100, max - 100))
        } else {
            None
        }
    });
    let Some(rots) = rots else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for label in ["A", "B", "C"] {
        if let Some(items) = rots.get(label) {
            let mut v = items.clone();
            sort_pool(&mut v);
            out.push(RewardRotation { label: label.to_string(), items: v });
        }
    }
    out
}

/// Sort a reward pool in place: rarity (common → rare) then descending chance.
fn sort_pool(v: &mut [RewardItem]) {
    v.sort_by(|a, b| {
        rarity_rank(&a.rarity)
            .cmp(&rarity_rank(&b.rarity))
            .then(b.chance.partial_cmp(&a.chance).unwrap_or(std::cmp::Ordering::Equal))
    });
}

/// Cambion Drift (魔胎之境) exposes ONE flattened pool per bounty — each Cambion
/// Drift / Isolation Vault job shows only its own active `Table` letter's pool,
/// not a board-wide A/B/C rotation (verified against 英择谛.png). Vault jobs key
/// separately (`v{min}-{max}` regular, `av{min}-{max}` Arcana) so the regular and
/// vault 30-40 brackets don't collide. The single returned rotation makes the
/// frontend render it as 单一奖励池 (same path as Zariman).
fn deimos_rotations(rewards_path: &str, min: i64, max: i64) -> Vec<RewardRotation> {
    let prefix = if rewards_path.contains("Arcana") {
        "av"
    } else if rewards_path.contains("Vault") {
        "v"
    } else {
        ""
    };
    let key = format!("{prefix}{min}-{max}");
    let Some(rots) = deimos_rewards().get(&key) else {
        return Vec::new();
    };
    // The job's own `Table` letter selects the single live pool; fall back to A
    // then whatever exists if the letter is missing from the snapshot.
    let letter = active_rotation_of(rewards_path);
    let Some(items) = rots
        .get(&letter)
        .or_else(|| rots.get("A"))
        .or_else(|| rots.values().next())
    else {
        return Vec::new();
    };
    let mut v = items.clone();
    sort_pool(&mut v);
    vec![RewardRotation { label: letter, items: v }]
}

fn parse_bounty_job(tag: &str, j: &Value) -> BountyJob {
    let xp = j["xpAmounts"].as_array();
    let job_type = j["jobType"].as_str().unwrap_or("");
    let min_level = j["minEnemyLevel"].as_i64().unwrap_or(0);
    let max_level = j["maxEnemyLevel"].as_i64().unwrap_or(0);
    let rewards_path = j["rewards"].as_str().unwrap_or("");
    let tier = reward_tier(rewards_path);
    let rotations = if tag == "EntratiSyndicate" {
        deimos_rotations(rewards_path, min_level, max_level)
    } else {
        reward_rotations(tag, min_level, max_level)
    };
    BountyJob {
        title: bounty_title(tag, job_type, min_level),
        desc: String::new(),
        name: bounty_type_zh(job_type),
        min_level,
        max_level,
        mastery_req: j["masteryReq"].as_i64().unwrap_or(0),
        stages: xp.map(|a| a.len()).unwrap_or(0),
        standing: xp
            .map(|a| a.iter().filter_map(|v| v.as_i64()).sum())
            .unwrap_or(0),
        rotations,
        tier,
    }
}

/// Build a `BountyJob` from a static (min, max) bracket — used for sources whose
/// `Jobs` array DE never populates in the public worldState (see below).
fn static_bounty_job(tag: &str, min: i64, max: i64) -> BountyJob {
    BountyJob {
        title: bounty_title(tag, "", min),
        desc: String::new(),
        name: String::new(),
        min_level: min,
        max_level: max,
        mastery_req: 0,
        stages: 0,
        standing: 0,
        rotations: reward_rotations(tag, min, max),
        tier: String::new(),
    }
}

/// Zariman (坚守者) offers a fixed set of 5 bounties (mission type + level are
/// constant; single reward pool). Synthesised locally because the worldState
/// `ZarimanSyndicate.Jobs` array is always empty (findings 续18).
fn synthesize_zariman_jobs() -> Vec<BountyJob> {
    const LEVELS: &[(i64, i64)] = &[(50, 55), (60, 65), (70, 75), (90, 95), (110, 115)];
    LEVELS
        .iter()
        .map(|&(min, max)| static_bounty_job("ZarimanSyndicate", min, max))
        .collect()
}

/// Höllvania / 1999 (六人组) offers a fixed set of 7 bounties (single reward pool).
/// Synthesised locally because the worldState `HexSyndicate.Jobs` array is always
/// empty even while active (findings 续29), exactly like Zariman.
fn synthesize_hex_jobs() -> Vec<BountyJob> {
    // (min_level, max_level, standing) — the fixed framework (wiki + WFCD, findings
    // 续35). In-game level = WFCD drop-table label + 10. Standing is per-tier and
    // stable (1000…7500); steel-path runs ×1.5 but we only surface the base 7.
    const SLOTS: &[(i64, i64, i64)] = &[
        (65, 70, 1000), (75, 80, 2000), (85, 90, 3000), (95, 100, 4000),
        (105, 110, 5000), (115, 120, 6000), (125, 130, 7500),
    ];
    SLOTS
        .iter()
        .map(|&(min, max, standing)| {
            let mut job = static_bounty_job("HexSyndicate", min, max);
            job.standing = standing;
            job
        })
        .collect()
}

/// 解剖圣所 / 阿尔布雷希特实验室 (EntratiLabSyndicate / Cavia) — 5 fixed bounties,
/// single reward pool (WFCD populates only one rotation, like Hex/Zariman).
/// worldState Jobs are always empty (only a Seed); levels from WFCD `entratiLabRewards`.
/// Standing is a currency (音魂/Voca), not a flat number → left 0 (chip hidden).
fn synthesize_entrati_lab_jobs() -> Vec<BountyJob> {
    const LEVELS: &[(i64, i64)] = &[(55, 60), (65, 70), (75, 80), (95, 100), (115, 120)];
    LEVELS
        .iter()
        .map(|&(min, max)| static_bounty_job("EntratiLabSyndicate", min, max))
        .collect()
}

/// Parse open-world bounty boards from `SyndicateMissions`. Sources listed in
/// [`BOUNTY_SOURCES`] are read from their `Jobs` array; Zariman is synthesised
/// from a static template because DE leaves its `Jobs` empty even while active.
pub fn parse_bounties(data: &Value) -> Vec<BountyInfo> {
    let now = now_ms();
    let mut out = Vec::new();
    for (tag, zh) in BOUNTY_SOURCES {
        let Some(entry) = find_active_syndicate(data, tag) else {
            continue;
        };
        let job_arr = entry["Jobs"].as_array();
        let (jobs, active_rotation) = if *tag == "ZarimanSyndicate" {
            // Static, single reward pool → no live Jobs/rotation needed.
            (synthesize_zariman_jobs(), String::new())
        } else if *tag == "HexSyndicate" {
            // Höllvania/1999 — same static-synthesis case as Zariman.
            (synthesize_hex_jobs(), String::new())
        } else if *tag == "EntratiLabSyndicate" {
            // 解剖圣所/Cavia — empty Jobs, single pool; hangs under 魔胎之境 card.
            (synthesize_entrati_lab_jobs(), String::new())
        } else {
            let jobs: Vec<BountyJob> = job_arr
                .map(|arr| arr.iter().map(|j| parse_bounty_job(tag, j)).collect())
                .unwrap_or_default();
            // The active rotation is the `Table?` letter, shared by all jobs.
            let active_rotation = job_arr
                .and_then(|a| a.first())
                .and_then(|j| j["rewards"].as_str())
                .map(active_rotation_of)
                .unwrap_or_default();
            (jobs, active_rotation)
        };
        if jobs.is_empty() {
            continue;
        }
        let expiry = get_ms(&entry["Expiry"]);
        out.push(BountyInfo {
            syndicate: zh.to_string(),
            card: bounty_card(zh).to_string(),
            expiry_ms: expiry,
            remain_ms: expiry - now,
            remain_str: fmt_remain(expiry - now),
            active_rotation,
            jobs,
        });
    }
    out
}

/// Parse the first Void Trader (Baro) entry into a `BaroInfo`.
/// Returns `None` when the section is absent.
pub fn parse_void_trader(data: &Value) -> Option<BaroInfo> {
    let trader = data["VoidTraders"].as_array()?.first()?;
    let now = now_ms();
    let start = get_ms(&trader["Activation"]);
    let end = get_ms(&trader["Expiry"]);
    let active = start <= now && now < end;

    let node_key = trader["Node"].as_str().unwrap_or("");
    let info = node_lookup(node_key);
    let location = if info.name.is_empty() {
        node_key.to_string()
    } else {
        format!("{} / {}", info.name, info.planet)
    };

    // Manifest is only populated while Baro is present.
    let mut items = Vec::new();
    if active {
        if let Some(manifest) = trader["Manifest"].as_array() {
            for it in manifest {
                let path = it["ItemType"].as_str().unwrap_or("");
                let name = crate::item_i18n::translate(path)
                    .unwrap_or_else(|| name_from_path(path));
                items.push(BaroItem {
                    name,
                    ducats: it["PrimePrice"].as_i64().unwrap_or(0),
                    credits: it["RegularPrice"].as_i64().unwrap_or(0),
                });
            }
        }
    }

    let remain_ms = if active { end - now } else { start - now };

    Some(BaroInfo {
        active,
        location,
        start_ms: start,
        end_ms: end,
        remain_ms,
        remain_str: fmt_remain_baro(remain_ms),
        items,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6c. DUVIRI CIRCUIT (无限回廊)
// ═══════════════════════════════════════════════════════════════════════════════

/// Circuit choice token → 简中 display name, pre-generated offline via
/// `_gen_circuit.py` (name→uniqueName→baro_zh). Warframe tokens resolve to their
/// English name (the CN client doesn't translate Warframe names). Embedded.
static CIRCUIT_NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();
fn circuit_names() -> &'static HashMap<String, String> {
    CIRCUIT_NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/circuit_names.json"))
            .unwrap_or_default()
    })
}

/// Resolve a worldState `EndlessXpChoices` token (e.g. "Soma", "NamiSolo") to a
/// display name; falls back to the raw token when unmapped.
fn circuit_zh(token: &str) -> String {
    circuit_names()
        .get(token)
        .cloned()
        .unwrap_or_else(|| token.to_string())
}

/// Parse the Duviri Circuit weekly rotation from `EndlessXpChoices`
/// (EXC_NORMAL = 战甲, EXC_HARD = Incarnon 武器) + `EndlessXpSchedule` (expiry).
/// Returns `None` when the section is absent/empty.
pub fn parse_circuit(data: &Value) -> Option<CircuitInfo> {
    let choices = data["EndlessXpChoices"].as_array()?;
    let mut normal = Vec::new();
    let mut hard = Vec::new();
    for c in choices {
        let names: Vec<String> = c["Choices"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).map(circuit_zh).collect())
            .unwrap_or_default();
        match c["Category"].as_str().unwrap_or("") {
            "EXC_NORMAL" => normal = names,
            "EXC_HARD" => hard = names,
            _ => {}
        }
    }
    if normal.is_empty() && hard.is_empty() {
        return None;
    }
    let expiry = data["EndlessXpSchedule"]
        .as_array()
        .and_then(|a| a.first())
        .map(|s| get_ms(&s["Expiry"]))
        .unwrap_or(0);
    let now = now_ms();
    Some(CircuitInfo {
        normal,
        hard,
        expiry_ms: expiry,
        remain_ms: expiry - now,
        remain_str: fmt_remain_days(expiry - now),
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// 7. HTTP FETCHING
// ═══════════════════════════════════════════════════════════════════════════════

// ── Arbitration ──────────────────────────────────────────────────────────────
// Data from arbi.wf.wiki (https://arbi.wf.wiki/data/arbys.schedule.v2.json and
// arbys.nodes.zh.json). The schedule is a 88-node pool with a pre-computed seq
// of 44056 hourly slots (stored as one byte per slot in arbitration_seq.bin).

struct ArbData {
    start_ts: i64,
    step_sec: i64,
    nodes: Vec<String>,               // 88 node keys
    seq: &'static [u8],               // 44056 indices into nodes[]
    node_info: HashMap<String, ArbNodeInfo>,
}

struct ArbNodeInfo {
    name_zh: String,
    system_zh: String,
    mission_zh: String,
    faction_zh: String,
    min_level: i32,
    max_level: i32,
    archwing: bool,
}

static ARB_DATA: OnceLock<ArbData> = OnceLock::new();

fn arb_data() -> &'static ArbData {
    ARB_DATA.get_or_init(|| {
        let meta_raw = include_str!("../resources/arbitration_meta.json");
        let nodes_raw = include_str!("../resources/arbitration_nodes_zh.json");
        let seq_bytes: &'static [u8] = include_bytes!("../resources/arbitration_seq.bin");

        let meta: Value = serde_json::from_str(meta_raw).unwrap_or(Value::Null);
        let nodes_val: Value = serde_json::from_str(nodes_raw).unwrap_or(Value::Null);

        let start_ts = meta["startTs"].as_i64().unwrap_or(1_727_884_800);
        let step_sec = meta["stepSec"].as_i64().unwrap_or(3600);
        let nodes: Vec<String> = meta["nodes"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let mut node_info = HashMap::new();
        if let Some(obj) = nodes_val["nodes"].as_object() {
            for (k, v) in obj {
                let mt = v["missionType"].as_str().unwrap_or("");
                let archwing = mt == "MT_ARCHWING" || mt == "MT_ARCHWING_MOBILE"
                    || mt == "MT_ARCHWING_EXTERMINATION" || mt == "MT_SHARKWING";
                node_info.insert(k.clone(), ArbNodeInfo {
                    name_zh: v["nameZh"].as_str().unwrap_or("").to_string(),
                    system_zh: v["systemNameZh"].as_str().unwrap_or("").to_string(),
                    mission_zh: v["missionNameZh"].as_str().unwrap_or("").to_string(),
                    faction_zh: v["factionNameZh"].as_str().unwrap_or("").to_string(),
                    min_level: v["minEnemyLevel"].as_i64().unwrap_or(0) as i32,
                    max_level: v["maxEnemyLevel"].as_i64().unwrap_or(0) as i32,
                    archwing,
                });
            }
        }

        ArbData { start_ts, step_sec, nodes, seq: seq_bytes, node_info }
    })
}

fn arb_slot_at(d: &ArbData, hour_idx: usize) -> Option<ArbitrationSlot> {
    let seq_idx = *d.seq.get(hour_idx)? as usize;
    let node_key = d.nodes.get(seq_idx)?;
    let info = d.node_info.get(node_key)?;
    Some(ArbitrationSlot {
        node: info.name_zh.clone(),
        planet: info.system_zh.clone(),
        mission: info.mission_zh.clone(),
        faction: info.faction_zh.clone(),
        min_level: info.min_level,
        max_level: info.max_level,
        archwing: info.archwing,
    })
}

pub fn parse_arbitration(now_ms: i64) -> Option<ArbitrationInfo> {
    let d = arb_data();
    let now_s = now_ms / 1000;
    if now_s < d.start_ts { return None; }

    let hour_idx = ((now_s - d.start_ts) / d.step_sec) as usize;
    if hour_idx >= d.seq.len() { return None; }

    let current = arb_slot_at(d, hour_idx)?;
    let expiry_ms = (d.start_ts + (hour_idx as i64 + 1) * d.step_sec) * 1000;
    let remain_ms = expiry_ms - now_ms;

    let upcoming: Vec<ArbitrationSlot> = (1..=3)
        .filter_map(|i| arb_slot_at(d, hour_idx + i))
        .collect();

    Some(ArbitrationInfo {
        current,
        upcoming,
        expiry_ms,
        remain_ms,
        remain_str: fmt_remain(remain_ms),
    })
}

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
