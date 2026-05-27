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
        "MT_EXTERMINATION" => "歼灭",
        "MT_SURVIVAL" => "生存",
        "MT_CAPTURE" => "捕获",
        "MT_DEFENSE" => "防御",
        "MT_MOBILE_DEFENSE" => "移防",
        "MT_RESCUE" => "救援",
        "MT_SABOTAGE" => "破坏",
        "MT_SPY" => "间谍",
        "MT_INTERCEPTION" => "拦截",
        "MT_EXCAVATE" => "挖掘",
        "MT_HIVE" => "清巢",
        "MT_TERRITORY" => "中断",
        "MT_ARENA" => "竞技场",
        "MT_PURSUIT" => "追击",
        "MT_RUSH" => "强袭",
        "MT_ASSAULT" => "突击",
        "MT_SALVAGE" => "清剿",
        "MT_EVACUATION" => "撤离",
        "MT_VOID_CASCADE" => "虚空洪流",
        "MT_VOID_ARMAGEDDON" => "虚空决战",
        "MT_VOID_FLOOD" => "虚空覆涌",
        "MT_ALCHEMY" => "炼金",
        "MT_LANDSCAPE" => "自由漫步",
        "MT_SURVIVAL_DARK" => "暗影生存",
        "MT_DEFENSE_DARK" => "暗影防御",
        "MT_ASSAULT_TILESET" => "突击",
        "MT_RETRIEVAL" => "夺回",
        "MT_ARENA_SEDNA" => "竞技场",
        _ => key,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. NODE LOOKUP  (100+ entries)
// ═══════════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_lines)]
fn node_lookup(node_key: &str) -> (&str, &str) {
    match node_key {
        // ── Hubs ──────────────────────────────────────────────────────────
        "VenusHUB" => ("希图斯", "地球"),
        "EarthHUB" => ("钢铁守望", "地球"),
        "CetusHUB" => ("希图斯", "地球"),
        "FortunaHUB" => ("福尔图娜", "金星"),
        "DeimosHUB" => ("亡骸殿", "火卫二"),
        "NecraliskHUB" => ("殁世幽都", "火卫二"),
        "ZarimanHUB" => ("扎里曼", "扎里曼"),
        "ChrysalithHUB" => ("蛹壳号", "扎里曼"),
        "RelayHUB" => ("中继站", "中继站"),
        "DojoHUB" => ("氏族道场", "道场"),

        // ── 地球 ──────────────────────────────────────────────────────────
        "Earth节点1" => ("E Prime", "地球"),
        "Earth节点2" => ("Lith", "地球"),
        "Earth节点3" => ("Everest", "地球"),
        "Earth节点4" => ("Cervantes", "地球"),
        "Earth节点5" => ("Gaia", "地球"),
        "Earth节点6" => ("Mariana", "地球"),
        "Earth节点7" => ("Tikal", "地球"),
        "Earth节点8" => ("Cambria", "地球"),
        "Earth节点9" => ("Mantle", "地球"),
        "Earth节点10" => ("Pacific", "地球"),
        "Earth节点11" => ("Coba", "地球"),
        "Earth节点12" => ("Erpo", "地球"),

        // ── 金星 ──────────────────────────────────────────────────────────
        "Venus节点1" => ("Unda", "金星"),
        "Venus节点2" => ("Venera", "金星"),
        "Venus节点3" => ("Kiliken", "金星"),
        "Venus节点4" => ("Tessera", "金星"),
        "Venus节点5" => ("Ishtar", "金星"),
        "Venus节点6" => ("Cytherean", "金星"),
        "Venus节点7" => ("Romula", "金星"),
        "Venus节点8" => ("Fossa", "金星"),
        "Venus节点9" => ("Linea", "金星"),
        "Venus节点10" => ("Vesper", "金星"),
        "Venus节点11" => ("Montes", "金星"),
        "Venus节点12" => ("Malva", "金星"),

        // ── 水星 ──────────────────────────────────────────────────────────
        "Mercury节点1" => ("Tolstoj", "水星"),
        "Mercury节点2" => ("Laplace", "水星"),
        "Mercury节点3" => ("Elion", "水星"),
        "Mercury节点4" => ("Caloris", "水星"),
        "Mercury节点5" => ("Boethius", "水星"),
        "Mercury节点6" => ("Odin", "水星"),
        "Mercury节点7" => ("Suisei", "水星"),
        "Mercury节点8" => ("Apollodorus", "水星"),
        "Mercury节点9" => ("Pantheon", "水星"),
        "Mercury节点10" => ("M Prime", "水星"),

        // ── 火星 ──────────────────────────────────────────────────────────
        "Mars节点1" => ("Ara", "火星"),
        "Mars节点2" => ("Alator", "火星"),
        "Mars节点3" => ("Olympus", "火星"),
        "Mars节点4" => ("Spear", "火星"),
        "Mars节点5" => ("Augustus", "火星"),
        "Mars节点6" => ("Martialis", "火星"),
        "Mars节点7" => ("Hellas", "火星"),
        "Mars节点8" => ("Syrtis", "火星"),
        "Mars节点9" => ("Ultor", "火星"),
        "Mars节点10" => ("Wahiba", "火星"),
        "Mars节点11" => ("Kadesh", "火星"),
        "Mars节点12" => ("Tharsis", "火星"),
        "Mars节点13" => ("Gradivus", "火星"),
        "Mars节点14" => ("Quirinus", "火星"),
        "Mars节点15" => ("Ares", "火星"),

        // ── 火卫一 ────────────────────────────────────────────────────────
        "Phobos节点1" => ("Roche", "火卫一"),
        "Phobos节点2" => ("Stickney", "火卫一"),
        "Phobos节点3" => ("Gulliver", "火卫一"),
        "Phobos节点4" => ("Kepler", "火卫一"),
        "Phobos节点5" => ("Skyresh", "火卫一"),
        "Phobos节点6" => ("Monolith", "火卫一"),
        "Phobos节点7" => ("Wendell", "火卫一"),
        "Phobos节点8" => ("Sharpless", "火卫一"),
        "Phobos节点9" => ("Memphis", "火卫一"),
        "Phobos节点10" => ("Iliad", "火卫一"),
        "Phobos节点11" => ("Zeugma", "火卫一"),
        "Phobos节点12" => ("Grildrig", "火卫一"),
        "Phobos节点13" => ("Flimnap", "火卫一"),

        // ── 谷神星 ────────────────────────────────────────────────────────
        "Ceres节点1" => ("Pallas", "谷神星"),
        "Ceres节点2" => ("Bode", "谷神星"),
        "Ceres节点3" => ("Draco", "谷神星"),
        "Ceres节点4" => ("Ludi", "谷神星"),
        "Ceres节点5" => ("Cinxia", "谷神星"),
        "Ceres节点6" => ("Ker", "谷神星"),
        "Ceres节点7" => ("Seimeni", "谷神星"),
        "Ceres节点8" => ("Lex", "谷神星"),
        "Ceres节点9" => ("Egeria", "谷神星"),
        "Ceres节点10" => ("Kiste", "谷神星"),
        "Ceres节点11" => ("Exta", "谷神星"),
        "Ceres节点12" => ("Nuovo", "谷神星"),
        "Ceres节点13" => ("Varro", "谷神星"),
        "Ceres节点14" => ("Thon", "谷神星"),
        "Ceres节点15" => ("Casta", "谷神星"),

        // ── 木星 ──────────────────────────────────────────────────────────
        "Jupiter节点1" => ("Carme", "木星"),
        "Jupiter节点2" => ("Io", "木星"),
        "Jupiter节点3" => ("Sinope", "木星"),
        "Jupiter节点4" => ("Adrastea", "木星"),
        "Jupiter节点5" => ("Ganymede", "木星"),
        "Jupiter节点6" => ("Themisto", "木星"),
        "Jupiter节点7" => ("Ananke", "木星"),
        "Jupiter节点8" => ("Elara", "木星"),
        "Jupiter节点9" => ("Metis", "木星"),
        "Jupiter节点10" => ("Callisto", "木星"),
        "Jupiter节点11" => ("Amalthea", "木星"),
        "Jupiter节点12" => ("Himalia", "木星"),

        // ── 欧罗巴 ────────────────────────────────────────────────────────
        "Europa节点1" => ("Abaddon", "欧罗巴"),
        "Europa节点2" => ("Armaros", "欧罗巴"),
        "Europa节点3" => ("Gamygyn", "欧罗巴"),
        "Europa节点4" => ("Kokabiel", "欧罗巴"),
        "Europa节点5" => ("Morax", "欧罗巴"),
        "Europa节点6" => ("Valac", "欧罗巴"),
        "Europa节点7" => ("Valefor", "欧罗巴"),
        "Europa节点8" => ("Ose", "欧罗巴"),
        "Europa节点9" => ("Paimon", "欧罗巴"),
        "Europa节点10" => ("Bael", "欧罗巴"),
        "Europa节点11" => ("Naamah", "欧罗巴"),

        // ── 土星 ──────────────────────────────────────────────────────────
        "Saturn节点1" => ("Mimas", "土星"),
        "Saturn节点2" => ("Rhea", "土星"),
        "Saturn节点3" => ("Enceladus", "土星"),
        "Saturn节点4" => ("Tethys", "土星"),
        "Saturn节点5" => ("Cassini", "土星"),
        "Saturn节点6" => ("Titan", "土星"),
        "Saturn节点7" => ("Numa", "土星"),
        "Saturn节点8" => ("Dione", "土星"),
        "Saturn节点9" => ("Phoebe", "土星"),
        "Saturn节点10" => ("Pandora", "土星"),
        "Saturn节点11" => ("Iapetus", "土星"),
        "Saturn节点12" => ("Calypso", "土星"),
        "Saturn节点13" => ("Keeler", "土星"),
        "Saturn节点14" => ("Telesto", "土星"),
        "Saturn节点15" => ("Hyperion", "土星"),
        "Saturn节点16" => ("Helene", "土星"),
        "Saturn节点17" => ("Aegaeon", "土星"),
        "Saturn节点18" => ("Carpo", "土星"),
        "Saturn节点19" => ("Anthe", "土星"),
        "Saturn节点20" => ("Pallene", "土星"),
        "Saturn节点21" => ("Epimetheus", "土星"),
        "Saturn节点22" => ("Janus", "土星"),
        "Saturn节点23" => ("Atlas", "土星"),
        "Saturn节点24" => ("Prometheus", "土星"),

        // ── 天王星 ────────────────────────────────────────────────────────
        "Uranus节点1" => ("Ariel", "天王星"),
        "Uranus节点2" => ("Umbriel", "天王星"),
        "Uranus节点3" => ("Miranda", "天王星"),
        "Uranus节点4" => ("Titania", "天王星"),
        "Uranus节点5" => ("Oberon", "天王星"),
        "Uranus节点6" => ("Caliban", "天王星"),
        "Uranus节点7" => ("Prospero", "天王星"),
        "Uranus节点8" => ("Rosalind", "天王星"),
        "Uranus节点9" => ("Desdemona", "天王星"),
        "Uranus节点10" => ("Portia", "天王星"),
        "Uranus节点11" => ("Cressida", "天王星"),
        "Uranus节点12" => ("Ophelia", "天王星"),
        "Uranus节点13" => ("Cordelia", "天王星"),
        "Uranus节点14" => ("Bianca", "天王星"),
        "Uranus节点15" => ("Mab", "天王星"),
        "Uranus节点16" => ("Trinculo", "天王星"),
        "Uranus节点17" => ("Stephano", "天王星"),
        "Uranus节点18" => ("Sycorax", "天王星"),
        "Uranus节点19" => ("Cupid", "天王星"),
        "Uranus节点20" => ("Puck", "天王星"),
        "Uranus节点21" => ("Setebos", "天王星"),
        "Uranus节点22" => ("Perdita", "天王星"),
        "Uranus节点23" => ("Juliet", "天王星"),
        "Uranus节点24" => ("Francisco", "天王星"),

        // ── 海王星 ────────────────────────────────────────────────────────
        "Neptune节点1" => ("Galatea", "海王星"),
        "Neptune节点2" => ("Triton", "海王星"),
        "Neptune节点3" => ("Despina", "海王星"),
        "Neptune节点4" => ("Thalassa", "海王星"),
        "Neptune节点5" => ("Proteus", "海王星"),
        "Neptune节点6" => ("Neso", "海王星"),
        "Neptune节点7" => ("Yursa", "海王星"),
        "Neptune节点8" => ("Laomedeia", "海王星"),
        "Neptune节点9" => ("Larissa", "海王星"),
        "Neptune节点10" => ("Naiad", "海王星"),
        "Neptune节点11" => ("Salacia", "海王星"),
        "Neptune节点12" => ("Sao", "海王星"),
        "Neptune节点13" => ("Halimede", "海王星"),
        "Neptune节点14" => ("Psamathe", "海王星"),

        // ── 冥王星 ────────────────────────────────────────────────────────
        "Pluto节点1" => ("Cerberus", "冥王星"),
        "Pluto节点2" => ("Acheron", "冥王星"),
        "Pluto节点3" => ("Oceanum", "冥王星"),
        "Pluto节点4" => ("Hades", "冥王星"),
        "Pluto节点5" => ("Hydra", "冥王星"),
        "Pluto节点6" => ("Palus", "冥王星"),
        "Pluto节点7" => ("Narcissus", "冥王星"),
        "Pluto节点8" => ("Cypress", "冥王星"),
        "Pluto节点9" => ("Minthe", "冥王星"),
        "Pluto节点10" => ("Regna", "冥王星"),
        "Pluto节点11" => ("Sechura", "冥王星"),
        "Pluto节点12" => ("Hieracon", "冥王星"),

        // ── 赛德娜 ────────────────────────────────────────────────────────
        "Sedna节点1" => ("Sangeru", "赛德娜"),
        "Sedna节点2" => ("Rusalka", "赛德娜"),
        "Sedna节点3" => ("Nakki", "赛德娜"),
        "Sedna节点4" => ("Phithale", "赛德娜"),
        "Sedna节点5" => ("Yemaja", "赛德娜"),
        "Sedna节点6" => ("Charybdis", "赛德娜"),
        "Sedna节点7" => ("Kelpie", "赛德娜"),
        "Sedna节点8" => ("Adaro", "赛德娜"),
        "Sedna节点9" => ("Merrow", "赛德娜"),
        "Sedna节点10" => ("Vodyanoi", "赛德娜"),
        "Sedna节点11" => ("Naga", "赛德娜"),
        "Sedna节点12" => ("Selkie", "赛德娜"),
        "Sedna节点13" => ("Berehynia", "赛德娜"),
        "Sedna节点14" => ("Hyndra", "赛德娜"),
        "Sedna节点15" => ("Tiamat", "赛德娜"),
        "Sedna节点16" => ("Undine", "赛德娜"),
        "Sedna节点17" => ("Kappa", "赛德娜"),

        // ── 火卫二 ────────────────────────────────────────────────────────
        "Deimos节点1" => ("Horend", "火卫二"),
        "Deimos节点2" => ("Phlegyas", "火卫二"),
        "Deimos节点3" => ("Cambion", "火卫二"),
        "Deimos节点4" => ("Iacorus", "火卫二"),
        "Deimos节点5" => ("Formido", "火卫二"),
        "Deimos节点6" => ("Arx", "火卫二"),
        "Deimos节点7" => ("Magnacidium", "火卫二"),
        "Deimos节点8" => ("Terrorem", "火卫二"),
        "Deimos节点9" => ("Exequias", "火卫二"),
        "Deimos节点10" => ("Cosisper", "火卫二"),
        "Deimos节点11" => ("Dirus", "火卫二"),
        "Deimos节点12" => ("Hyf", "火卫二"),
        "Deimos节点13" => ("Zealot", "火卫二"),
        "Deimos节点14" => ("Abscess", "火卫二"),

        // ── 赤毒要塞 ──────────────────────────────────────────────────────
        "KuvaFortress节点1" => ("Nabuk", "赤毒要塞"),
        "KuvaFortress节点2" => ("Taveuni", "赤毒要塞"),
        "KuvaFortress节点3" => ("Rotuma", "赤毒要塞"),
        "KuvaFortress节点4" => ("Pago", "赤毒要塞"),
        "KuvaFortress节点5" => ("Garus", "赤毒要塞"),
        "KuvaFortress节点6" => ("Dakata", "赤毒要塞"),
        "KuvaFortress节点7" => ("Tamu", "赤毒要塞"),
        "KuvaFortress节点8" => ("Nimrod", "赤毒要塞"),

        // ── 虚空 ──────────────────────────────────────────────────────────
        "Void节点1" => ("Teshub", "虚空"),
        "Void节点2" => ("Hepit", "虚空"),
        "Void节点3" => ("TiwaZ", "虚空"),
        "Void节点4" => ("Stribog", "虚空"),
        "Void节点5" => ("Ani", "虚空"),
        "Void节点6" => ("Oxomoco", "虚空"),
        "Void节点7" => ("Ukko", "虚空"),
        "Void节点8" => ("Belenus", "虚空"),
        "Void节点9" => ("Marduk", "虚空"),
        "Void节点10" => ("Mot", "虚空"),
        "Void节点11" => ("Aten", "虚空"),
        "Void节点12" => ("Mithra", "虚空"),
        "Void节点13" => ("Taranis", "虚空"),

        // ── 阋神星 ────────────────────────────────────────────────────────
        "Eris节点1" => ("Isos", "阋神星"),
        "Eris节点2" => ("Nimus", "阋神星"),
        "Eris节点3" => ("Saxis", "阋神星"),
        "Eris节点4" => ("Brugia", "阋神星"),
        "Eris节点5" => ("Cyath", "阋神星"),
        "Eris节点6" => ("Sparga", "阋神星"),
        "Eris节点7" => ("Xini", "阋神星"),
        "Eris节点8" => ("Naeglar", "阋神星"),
        "Eris节点9" => ("Kala-Azar", "阋神星"),
        "Eris节点10" => ("Gamygyn", "阋神星"),
        "Eris节点11" => ("Akkad", "阋神星"),
        "Eris节点12" => ("Zabala", "阋神星"),
        "Eris节点13" => ("Solium", "阋神星"),
        "Eris节点14" => ("Candiru", "阋神星"),
        "Eris节点15" => ("Oestrus", "阋神星"),
        "Eris节点16" => ("Meso", "阋神星"),

        // ── 月球 ──────────────────────────────────────────────────────────
        "Moon节点1" => ("Copernicus", "月球"),
        "Moon节点2" => ("Tycho", "月球"),
        "Moon节点3" => ("Grimaldi", "月球"),
        "Moon节点4" => ("Plato", "月球"),
        "Moon节点5" => ("Pavlov", "月球"),
        "Moon节点6" => ("Zeipel", "月球"),
        "Moon节点7" => ("Stöfler", "月球"),
        "Moon节点8" => ("Oceanus", "月球"),
        "Moon节点9" => ("Galileo", "月球"),
        "Moon节点10" => ("Ardath", "月球"),
        "Moon节点11" => ("Apollo", "月球"),
        "Moon节点12" => ("Lares", "月球"),
        "Moon节点13" => ("Cassiopeia", "月球"),
        "Moon节点14" => ("Junction", "月球"),

        // ── 扎里曼 ────────────────────────────────────────────────────────
        "Zariman节点1" => ("永视", "扎里曼"),
        "Zariman节点2" => ("福地", "扎里曼"),
        "Zariman节点3" => ("栖息地", "扎里曼"),
        "Zariman节点4" => ("长廊", "扎里曼"),
        "Zariman节点5" => ("苍穹", "扎里曼"),
        "Zariman节点6" => ("星舞", "扎里曼"),
        "Zariman节点7" => ("绿洲", "扎里曼"),
        "Zariman节点8" => ("收获者", "扎里曼"),
        "Zariman节点9" => ("Halo", "扎里曼"),
        "Zariman节点10" => ("Hyperion", "扎里曼"),
        "Zariman节点11" => ("TheGreenway", "扎里曼"),
        "Zariman节点12" => ("TuvulCommons", "扎里曼"),
        "Zariman节点13" => ("OroWorks", "扎里曼"),
        "Zariman节点14" => ("LunaroArena", "扎里曼"),
        "Zariman节点15" => ("AeonianPlaza", "扎里曼"),

        // ── 双衍王境 (Duviri) ─────────────────────────────────────────────
        "Duviri节点1" => ("王境", "双衍王境"),
        "Duviri节点2" => ("TheCircuit", "双衍王境"),
        "Duviri节点3" => ("TheLoneStory", "双衍王境"),
        "Duviri节点4" => ("TheDuviriExperience", "双衍王境"),

        // ── 圣殿突袭 ──────────────────────────────────────────────────────
        "Sanctuary节点1" => ("圣殿突袭", "圣殿"),
        "Sanctuary节点2" => ("精英圣殿突袭", "圣殿"),

        // ── 钢铁之路 ──────────────────────────────────────────────────────
        "SteelPath节点1" => ("钢铁之路", "全星系"),

        _ => (node_key, "未知"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. FISSURE PARSING
// ═══════════════════════════════════════════════════════════════════════════════

/// Parse a single fissure entry from JSON
fn parse_fissure(m: &Value, is_storm: bool) -> Fissure {
    let node_key = m["Node"].as_str().unwrap_or("").to_string();
    let (node_name, planet) = node_lookup(&node_key);
    let node_name = node_name.to_string();
    let planet = planet.to_string();

    let mission_type_key = m["MissionType"].as_str().unwrap_or("");
    let mission_type_name = mission_type(mission_type_key).to_string();

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
            fissure.is_hard = m["isHard"]
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
