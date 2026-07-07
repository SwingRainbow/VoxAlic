//! Warframe.Market integration — item search, pricing, and details.
//!
//! Data flow:
//!   Embedded default (hardcoded popular items) → disk cache (market_items.json)
//!   → hot-swap on refresh. Detail + orders are fetched live from the API and
//!   session-cached (detail only — orders are always fresh).
//!
//! HTTP pattern follows api.rs: resp.json::<Value>() with no custom Accept-Encoding.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tauri::{Emitter, Manager};

use crate::models::{MarketItemSummary, MarketOrder, MarketItemFull, MarketCacheStatus};

const ICON_BASE: &str = "https://warframe.market/static/assets/";
const ITEMS_URL: &str = "https://api.warframe.market/v2/items";
const MARKET_API_BASE: &str = "https://api.warframe.market/v2";
const FILE_NAME: &str = "market_items.json";

// ── Shared HTTP client (reused — TLS / DNS overhead paid once) ──────────────

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("VoxAlic/1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest::Client::build")
    })
}

// ── Internal cache entry ─────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarketCachedItem {
    #[serde(default)]
    pub id: String,
    pub slug: String,
    pub name: String,       // en
    #[serde(default)]
    pub name_zh: String,    // zh-hans (for translation)
    pub icon: String,
    pub mr: Option<u8>,
    #[serde(default)]
    pub max_rank: Option<u8>, // max mod/arcane rank
    pub tags: Vec<String>,
}

// ── Hardcoded embedded default (~100 popular items, offline fallback) ───────

fn ci(slug: &str, name: &str, icon: &str, mr: Option<u8>, max_rank: Option<u8>, tags: &[&str]) -> MarketCachedItem {
    let name_zh: String = if name.ends_with(" Set") {
        name.replace(" Set", " 一套")
    } else {
        match slug {
            "adaptation" => "适应".into(),
            "blind_rage" => "盲怒".into(),
            "blood_rush" => "急进猛突".into(),
            "condition_overload" => "异况超量".into(),
            "continuity_prime" => "持久力 Prime".into(),
            "flow_prime" => "川流不息 Prime".into(),
            "heavy_caliber" => "重口径".into(),
            "hells_chamber" => "地狱弹膛".into(),
            "narrow_minded" => "心志偏狭".into(),
            "overextended" => "过度延伸".into(),
            "primed_chamber" => "高级膛室".into(),
            "primed_cryo_rounds" => "低温弹头 Prime".into(),
            "primed_flow" => "川流不息 Prime".into(),
            "rolling_guard" => "翻滚防护".into(),
            "serration_prime" => "膛线 Prime".into(),
            "split_chamber" => "分裂膛室".into(),
            "transient_fortitude" => "瞬时坚毅".into(),
            "vital_sense" => "要害".into(),
            "weeping_wounds" => "创口溃烂".into(),
            "arcane_aegis" => "赋能·神盾".into(),
            "arcane_avenger" => "赋能·复仇者".into(),
            "arcane_barrier" => "赋能·壁垒".into(),
            "arcane_energize" => "赋能·充沛".into(),
            "arcane_fury" => "赋能·狂怒".into(),
            "arcane_grace" => "赋能·优雅".into(),
            "arcane_guardian" => "赋能·保卫者".into(),
            "arcane_precision" => "赋能·精确".into(),
            "arcane_strike" => "赋能·打击".into(),
            "arcane_velocity" => "赋能·敏捷".into(),
            _ => "".into(),
        }
    };
    MarketCachedItem {
        id: slug.into(),
        slug: slug.into(),
        name: name.into(),
        name_zh,
        icon: icon.into(),
        mr,
        max_rank,
        tags: tags.iter().map(|s| s.to_string()).collect(),
    }
}

fn embedded_items() -> Vec<MarketCachedItem> {
    vec![
        // ── Prime Warframe sets ──
        ci("ash_prime_set", "Ash Prime Set", "items/images/en/ash_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("atlas_prime_set", "Atlas Prime Set", "items/images/en/atlas_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("banshee_prime_set", "Banshee Prime Set", "items/images/en/banshee_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","warframe"]),
        ci("chroma_prime_set", "Chroma Prime Set", "items/images/en/chroma_prime_set.thumb.128x128.png", Some(6), None, &["set","prime","warframe"]),
        ci("ember_prime_set", "Ember Prime Set", "items/images/en/ember_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("equinox_prime_set", "Equinox Prime Set", "items/images/en/equinox_prime_set.thumb.128x128.png", Some(5), None, &["set","prime","warframe"]),
        ci("frost_prime_set", "Frost Prime Set", "items/images/en/frost_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("gara_prime_set", "Gara Prime Set", "items/images/en/gara_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("garuda_prime_set", "Garuda Prime Set", "items/images/en/garuda_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("hydroid_prime_set", "Hydroid Prime Set", "items/images/en/hydroid_prime_set.thumb.128x128.png", Some(5), None, &["set","prime","warframe"]),
        ci("inaros_prime_set", "Inaros Prime Set", "items/images/en/inaros_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("ivara_prime_set", "Ivara Prime Set", "items/images/en/ivara_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("khora_prime_set", "Khora Prime Set", "items/images/en/khora_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("limbo_prime_set", "Limbo Prime Set", "items/images/en/limbo_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("loki_prime_set", "Loki Prime Set", "items/images/en/loki_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("mag_prime_set", "Mag Prime Set", "items/images/en/mag_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("mesa_prime_set", "Mesa Prime Set", "items/images/en/mesa_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("mirage_prime_set", "Mirage Prime Set", "items/images/en/mirage_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("nekros_prime_set", "Nekros Prime Set", "items/images/en/nekros_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("nezha_prime_set", "Nezha Prime Set", "items/images/en/nezha_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("nidus_prime_set", "Nidus Prime Set", "items/images/en/nidus_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("nova_prime_set", "Nova Prime Set", "items/images/en/nova_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("nyx_prime_set", "Nyx Prime Set", "items/images/en/nyx_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("oberon_prime_set", "Oberon Prime Set", "items/images/en/oberon_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("octavia_prime_set", "Octavia Prime Set", "items/images/en/octavia_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("revenant_prime_set", "Revenant Prime Set", "items/images/en/revenant_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("rhino_prime_set", "Rhino Prime Set", "items/images/en/rhino_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("saryn_prime_set", "Saryn Prime Set", "items/images/en/saryn_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("titania_prime_set", "Titania Prime Set", "items/images/en/titania_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("trinity_prime_set", "Trinity Prime Set", "items/images/en/trinity_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("valkyr_prime_set", "Valkyr Prime Set", "items/images/en/valkyr_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("vauban_prime_set", "Vauban Prime Set", "items/images/en/vauban_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("volt_prime_set", "Volt Prime Set", "items/images/en/volt_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("wisp_prime_set", "Wisp Prime Set", "items/images/en/wisp_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        ci("wukong_prime_set", "Wukong Prime Set", "items/images/en/wukong_prime_set.thumb.128x128.png", Some(5), None, &["set","prime","warframe"]),
        ci("zephyr_prime_set", "Zephyr Prime Set", "items/images/en/zephyr_prime_set.thumb.128x128.png", Some(0), None, &["set","prime","warframe"]),
        // ── Prime weapon sets ──
        ci("acceltra_prime_set", "Acceltra Prime Set", "items/images/en/acceltra_prime_set.thumb.128x128.png", Some(14), None, &["set","prime","primary"]),
        ci("bo_prime_set", "Bo Prime Set", "items/images/en/bo_prime_set.thumb.128x128.png", Some(5), None, &["set","prime","melee"]),
        ci("boltor_prime_set", "Boltor Prime Set", "items/images/en/boltor_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","primary"]),
        ci("braton_prime_set", "Braton Prime Set", "items/images/en/braton_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","primary"]),
        ci("burston_prime_set", "Burston Prime Set", "items/images/en/burston_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","primary"]),
        ci("cernos_prime_set", "Cernos Prime Set", "items/images/en/cernos_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","primary"]),
        ci("corinth_prime_set", "Corinth Prime Set", "items/images/en/corinth_prime_set.thumb.128x128.png", Some(14), None, &["set","prime","primary"]),
        ci("dakra_prime_set", "Dakra Prime Set", "items/images/en/dakra_prime_set.thumb.128x128.png", Some(10), None, &["set","prime","melee"]),
        ci("destreza_prime_set", "Destreza Prime Set", "items/images/en/destreza_prime_set.thumb.128x128.png", Some(7), None, &["set","prime","melee"]),
        ci("euphona_prime_set", "Euphona Prime Set", "items/images/en/euphona_prime_set.thumb.128x128.png", Some(14), None, &["set","prime","secondary"]),
        ci("fang_prime_set", "Fang Prime Set", "items/images/en/fang_prime_set.thumb.128x128.png", Some(2), None, &["set","prime","melee"]),
        ci("fulmin_prime_set", "Fulmin Prime Set", "items/images/en/fulmin_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","primary"]),
        ci("glaive_prime_set", "Glaive Prime Set", "items/images/en/glaive_prime_set.thumb.128x128.png", Some(10), None, &["set","prime","melee"]),
        ci("gram_prime_set", "Gram Prime Set", "items/images/en/gram_prime_set.thumb.128x128.png", Some(14), None, &["set","prime","melee"]),
        ci("guandao_prime_set", "Guandao Prime Set", "items/images/en/guandao_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","melee"]),
        ci("kronen_prime_set", "Kronen Prime Set", "items/images/en/kronen_prime_set.thumb.128x128.png", Some(13), None, &["set","prime","melee"]),
        ci("lex_prime_set", "Lex Prime Set", "items/images/en/lex_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","secondary"]),
        ci("nami_skyla_prime_set", "Nami Skyla Prime Set", "items/images/en/nami_skyla_prime_set.thumb.128x128.png", Some(11), None, &["set","prime","melee"]),
        ci("nikana_prime_set", "Nikana Prime Set", "items/images/en/nikana_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","melee"]),
        ci("orthos_prime_set", "Orthos Prime Set", "items/images/en/orthos_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","melee"]),
        ci("paris_prime_set", "Paris Prime Set", "items/images/en/paris_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","primary"]),
        ci("pyrana_prime_set", "Pyrana Prime Set", "items/images/en/pyrana_prime_set.thumb.128x128.png", Some(13), None, &["set","prime","secondary"]),
        ci("redeemer_prime_set", "Redeemer Prime Set", "items/images/en/redeemer_prime_set.thumb.128x128.png", Some(10), None, &["set","prime","melee"]),
        ci("reaper_prime_set", "Reaper Prime Set", "items/images/en/reaper_prime_set.thumb.128x128.png", Some(10), None, &["set","prime","melee"]),
        ci("rubico_prime_set", "Rubico Prime Set", "items/images/en/rubico_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","primary"]),
        ci("scindo_prime_set", "Scindo Prime Set", "items/images/en/scindo_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","melee"]),
        ci("soma_prime_set", "Soma Prime Set", "items/images/en/soma_prime_set.thumb.128x128.png", Some(6), None, &["set","prime","primary"]),
        ci("sybaris_prime_set", "Sybaris Prime Set", "items/images/en/sybaris_prime_set.thumb.128x128.png", Some(12), None, &["set","prime","primary"]),
        ci("tiberon_prime_set", "Tiberon Prime Set", "items/images/en/tiberon_prime_set.thumb.128x128.png", Some(14), None, &["set","prime","primary"]),
        ci("tigris_prime_set", "Tigris Prime Set", "items/images/en/tigris_prime_set.thumb.128x128.png", Some(13), None, &["set","prime","primary"]),
        ci("tipedo_prime_set", "Tipedo Prime Set", "items/images/en/tipedo_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","melee"]),
        ci("vectis_prime_set", "Vectis Prime Set", "items/images/en/vectis_prime_set.thumb.128x128.png", Some(14), None, &["set","prime","primary"]),
        ci("venka_prime_set", "Venka Prime Set", "items/images/en/venka_prime_set.thumb.128x128.png", Some(8), None, &["set","prime","melee"]),
        // ── Popular mods ──
        ci("adaptation", "Adaptation", "items/images/en/adaptation.thumb.128x128.png", Some(10), None, &["mod","rare","warframe"]),
        ci("blind_rage", "Blind Rage", "items/images/en/blind_rage.thumb.128x128.png", Some(10), None, &["mod","rare","warframe"]),
        ci("blood_rush", "Blood Rush", "items/images/en/blood_rush.thumb.128x128.png", Some(10), None, &["mod","rare","melee"]),
        ci("condition_overload", "Condition Overload", "items/images/en/condition_overload.thumb.128x128.png", Some(10), None, &["mod","rare","melee"]),
        ci("continuity_prime", "Primed Continuity", "items/images/en/continuity_prime.thumb.128x128.png", Some(10), None, &["mod","legendary","warframe"]),
        ci("flow_prime", "Primed Flow", "items/images/en/flow_prime.thumb.128x128.png", Some(10), None, &["mod","legendary","warframe"]),
        ci("heavy_caliber", "Heavy Caliber", "items/images/en/heavy_caliber.thumb.128x128.png", Some(10), None, &["mod","rare","primary"]),
        ci("hells_chamber", "Hell's Chamber", "items/images/en/hells_chamber.thumb.128x128.png", Some(8), None, &["mod","uncommon","primary"]),
        ci("narrow_minded", "Narrow Minded", "items/images/en/narrow_minded.thumb.128x128.png", Some(10), None, &["mod","rare","warframe"]),
        ci("overextended", "Overextended", "items/images/en/overextended.thumb.128x128.png", Some(10), None, &["mod","rare","warframe"]),
        ci("primed_chamber", "Primed Chamber", "items/images/en/primed_chamber.thumb.128x128.png", Some(16), None, &["mod","legendary","primary"]),
        ci("primed_cryo_rounds", "Primed Cryo Rounds", "items/images/en/primed_cryo_rounds.thumb.128x128.png", Some(10), None, &["mod","legendary","primary"]),
        ci("primed_flow", "Primed Flow", "items/images/en/primed_flow.thumb.128x128.png", Some(10), None, &["mod","legendary","warframe"]),
        ci("rolling_guard", "Rolling Guard", "items/images/en/rolling_guard.thumb.128x128.png", Some(10), None, &["mod","rare","warframe"]),
        ci("serration_prime", "Primed Serration", "items/images/en/serration_prime.thumb.128x128.png", Some(10), None, &["mod","legendary","primary"]),
        ci("split_chamber", "Split Chamber", "items/images/en/split_chamber.thumb.128x128.png", Some(5), None, &["mod","rare","primary"]),
        ci("transient_fortitude", "Transient Fortitude", "items/images/en/transient_fortitude.thumb.128x128.png", Some(10), None, &["mod","rare","warframe"]),
        ci("vital_sense", "Vital Sense", "items/images/en/vital_sense.thumb.128x128.png", Some(5), None, &["mod","rare","primary"]),
        ci("weeping_wounds", "Weeping Wounds", "items/images/en/weeping_wounds.thumb.128x128.png", Some(10), None, &["mod","rare","melee"]),
        // ── Arcanes ──
        ci("arcane_aegis", "Arcane Aegis", "items/images/en/arcane_aegis.thumb.128x128.png", Some(5), None, &["legendary","arcane_enhancement"]),
        ci("arcane_avenger", "Arcane Avenger", "items/images/en/arcane_avenger.thumb.128x128.png", Some(5), None, &["rare","arcane_enhancement"]),
        ci("arcane_barrier", "Arcane Barrier", "items/images/en/arcane_barrier.thumb.128x128.png", Some(5), None, &["legendary","arcane_enhancement"]),
        ci("arcane_energize", "Arcane Energize", "items/images/en/arcane_energize.thumb.128x128.png", Some(5), None, &["legendary","arcane_enhancement"]),
        ci("arcane_fury", "Arcane Fury", "items/images/en/arcane_fury.thumb.128x128.png", Some(5), None, &["rare","arcane_enhancement"]),
        ci("arcane_grace", "Arcane Grace", "items/images/en/arcane_grace.thumb.128x128.png", Some(5), None, &["legendary","arcane_enhancement"]),
        ci("arcane_guardian", "Arcane Guardian", "items/images/en/arcane_guardian.thumb.128x128.png", Some(5), None, &["rare","arcane_enhancement"]),
        ci("arcane_precision", "Arcane Precision", "items/images/en/arcane_precision.thumb.128x128.png", Some(5), None, &["rare","arcane_enhancement"]),
        ci("arcane_strike", "Arcane Strike", "items/images/en/arcane_strike.thumb.128x128.png", Some(5), None, &["common","arcane_enhancement"]),
        ci("arcane_velocity", "Arcane Velocity", "items/images/en/arcane_velocity.thumb.128x128.png", Some(5), None, &["rare","arcane_enhancement"]),
    ]
}

// ── MarketCache ──────────────────────────────────────────────────────────────

/// Downloaded item detail (for session cache).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(private_interfaces)]
struct CachedDetail {
    ducats: Option<u32>,
    trading_tax: Option<u32>,
    set_root: bool,
    set_parts: Vec<String>,  // item IDs (excluding self)
}

pub struct MarketCache {
    /// slug → cached item (always non-empty — embedded default is the floor).
    pub items: HashMap<String, MarketCachedItem>,
    /// id → slug (for set-parts lookups).
    pub id_to_slug: HashMap<String, String>,
    /// slug → detail (session cache, cleared on restart).
    pub detail_cache: HashMap<String, CachedDetail>,
    /// ms timestamp of last successful disk-cache refresh (None = never).
    pub last_updated: Option<i64>,
}

pub type SharedMarketCache = Arc<RwLock<MarketCache>>;

// ── Build cache (sync, called before window creation) ───────────────────────

fn build_id_map(items: &HashMap<String, MarketCachedItem>) -> HashMap<String, String> {
    items.iter()
        .filter(|(_, item)| !item.id.is_empty())
        .map(|(slug, item)| (item.id.clone(), slug.clone()))
        .collect()
}

pub fn build_cache(app_data_dir: &std::path::Path) -> MarketCache {
    let path = app_data_dir.join(FILE_NAME);
    if let Ok(s) = std::fs::read_to_string(&path) {
        if let Ok(items) = serde_json::from_str::<Vec<MarketCachedItem>>(&s) {
            if !items.is_empty() {
                let map: HashMap<String, MarketCachedItem> = items
                    .into_iter()
                    .map(|i| (i.slug.clone(), i))
                    .collect();
                let id_to_slug = build_id_map(&map);
                let last_updated = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64);
                return MarketCache {
                    items: map,
                    id_to_slug,
                    detail_cache: HashMap::new(),
                    last_updated,
                };
            }
        }
    }
    // Fallback: embedded default (always non-empty).
    let items: HashMap<String, MarketCachedItem> = embedded_items()
        .into_iter()
        .map(|i| (i.slug.clone(), i))
        .collect();
    let id_to_slug = build_id_map(&items);
    MarketCache {
        items,
        id_to_slug,
        detail_cache: HashMap::new(),
        last_updated: None,
    }
}

// ── Search ───────────────────────────────────────────────────────────────────

fn search_local(cache: &MarketCache, query: &str, lang: &str) -> Vec<MarketItemSummary> {
    let q = query.to_lowercase();
    let q_trimmed = q.trim();
    if q_trimmed.is_empty() || q_trimmed.len() < 1 {
        return Vec::new();
    }
    cache
        .items
        .values()
        .filter(|item| {
            item.slug.to_lowercase().contains(q_trimmed)
                || item.name.to_lowercase().contains(q_trimmed)
                || (lang == "zh" && !item.name_zh.is_empty() && item.name_zh.contains(q_trimmed))
        })
        .take(50)
        .map(|item| MarketItemSummary {
            slug: item.slug.clone(),
            name: item.name.clone(),
            name_zh: item.name_zh.clone(),
            icon_url: format!("{}{}", ICON_BASE, item.icon),
            mr: item.mr,
            max_rank: item.max_rank,
            tags: item.tags.clone(),
        })
        .collect()
}

// ── API fetch helpers ────────────────────────────────────────────────────────

async fn fetch_detail(slug: &str) -> Result<CachedDetail, String> {
    let url = format!("{}/item/{}", MARKET_API_BASE, slug);
    let resp = client()
        .get(&url)
        .header("Language", "zh-hans")
        .send()
        .await
        .map_err(|e| format!("API 无响应: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("API 返回 HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;
    let data = body.get("data").ok_or("API 响应缺少 data 字段")?;
    let ducats = data["ducats"].as_u64().map(|v| v as u32);
    let tax = data["tradingTax"].as_u64().map(|v| v as u32);
    let set_root = data["setRoot"].as_bool().unwrap_or(false);
    let self_id = data["id"].as_str().unwrap_or("").to_string();
    let set_parts: Vec<String> = data["setParts"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).filter(|id| *id != self_id).collect())
        .unwrap_or_default();
    Ok(CachedDetail { ducats, trading_tax: tax, set_root, set_parts })
}

async fn fetch_orders(slug: &str) -> Result<(Vec<MarketOrder>, Vec<MarketOrder>), String> {
    let url = format!("{}/orders/item/{}", MARKET_API_BASE, slug);
    let resp = client()
        .get(&url)
        .timeout(std::time::Duration::from_secs(120))
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(|e| format!("API 无响应: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("API 返回 HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;
    let data = body["data"].as_array().ok_or("API 响应格式异常")?;

    let mut sell = Vec::new();
    let mut buy = Vec::new();
    for o in data {
        let order = MarketOrder {
            order_type: o["type"].as_str().unwrap_or("?").to_string(),
            platinum: o["platinum"].as_u64().unwrap_or(0) as u32,
            quantity: o["quantity"].as_u64().unwrap_or(1) as u32,
            player_name: o["user"]["ingameName"]
                .as_str()
                .unwrap_or("?")
                .to_string(),
            reputation: o["user"]["reputation"].as_i64().unwrap_or(0) as i32,
            status: o["user"]["status"]
                .as_str()
                .unwrap_or("offline")
                .to_string(),
            mod_rank: o["rank"].as_u64().map(|v| v as u8),
        };
        match order.order_type.as_str() {
            "sell" => sell.push(order),
            "buy" => buy.push(order),
            _ => {}
        }
    }
    // Sort: sells by price asc (cheapest first), buys by price desc (highest first)
    sell.sort_by_key(|o| o.platinum);
    buy.sort_by_key(|o| std::cmp::Reverse(o.platinum));
    Ok((sell, buy))
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn search_market_items(
    cache: tauri::State<'_, SharedMarketCache>,
    query: String,
    lang: String,
) -> Result<Vec<MarketItemSummary>, String> {
    let c = cache.read().await;
    Ok(search_local(&c, &query, &lang))
}

#[tauri::command]
pub async fn get_market_item(
    cache: tauri::State<'_, SharedMarketCache>,
    slug: String,
) -> Result<MarketItemFull, String> {
    // Check session cache for detail.
    let cached_detail: Option<CachedDetail> = {
        let c = cache.read().await;
        c.detail_cache.get(&slug).cloned()
    };

    let (ducats, tax, set_root, set_part_ids, sell_orders, buy_orders) = if let Some(d) = cached_detail {
        let (sell, buy) = fetch_orders(&slug).await?;
        (d.ducats, d.trading_tax, d.set_root, d.set_parts, sell, buy)
    } else {
        let (detail_res, orders_res) = tokio::join!(
            fetch_detail(&slug),
            fetch_orders(&slug),
        );
        let d = detail_res?;
        let (sell, buy) = orders_res?;
        {
            let mut c = cache.write().await;
            c.detail_cache.insert(slug.clone(), d.clone());
        }
        (d.ducats, d.trading_tax, d.set_root, d.set_parts, sell, buy)
    };

    // Build item summary + set parts from cache.
    let (item, set_parts) = {
        let c = cache.read().await;
        let item = c.items.get(&slug).map(|i| MarketItemSummary {
            slug: i.slug.clone(),
            name: i.name.clone(),
            name_zh: i.name_zh.clone(),
            icon_url: format!("{}{}", ICON_BASE, i.icon),
            mr: i.mr,
            max_rank: i.max_rank,
            tags: i.tags.clone(),
        });
        let parts: Vec<MarketItemSummary> = set_part_ids
            .iter()
            .filter_map(|pid| {
                c.id_to_slug.get(pid).and_then(|s| c.items.get(s))
            })
            .map(|i| MarketItemSummary {
                slug: i.slug.clone(),
                name: i.name.clone(),
                name_zh: i.name_zh.clone(),
                icon_url: format!("{}{}", ICON_BASE, i.icon),
                mr: i.mr,
                max_rank: i.max_rank,
                tags: i.tags.clone(),
            })
            .collect();
        (item, parts)
    };

    match item {
        Some(item) => Ok(MarketItemFull {
            item,
            ducats,
            trading_tax: tax,
            set_root,
            set_parts,
            sell_orders,
            buy_orders,
        }),
        None => Err("物品不在缓存中".into()),
    }
}

#[tauri::command]
pub async fn refresh_market_cache(
    cache: tauri::State<'_, SharedMarketCache>,
    app_handle: tauri::AppHandle,
) -> Result<usize, String> {
    let resp = client()
        .get(ITEMS_URL)
        .timeout(std::time::Duration::from_secs(1800))
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("API 返回 HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;
    let data = body["data"].as_array().ok_or("API 响应格式异常")?;

    let items: Vec<MarketCachedItem> = data
        .iter()
        .map(|v| {
            let id = v["id"].as_str().unwrap_or("").to_string();
            let slug = v["slug"].as_str().unwrap_or("").to_string();
            let name = v["i18n"]["en"]["name"]
                .as_str()
                .unwrap_or_else(|| v["i18n"]["zh-hans"]["name"].as_str().unwrap_or(&slug))
                .to_string();
            let icon = v["i18n"]["en"]["icon"]
                .as_str()
                .unwrap_or_else(|| v["i18n"]["zh-hans"]["icon"].as_str().unwrap_or(""))
                .to_string();
            let mr = v["reqMasteryRank"].as_u64().map(|v| v as u8);
            let max_rank = v["maxRank"].as_u64().map(|v| v as u8);
            let tags: Vec<String> = v["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let name_zh = v["i18n"]["zh-hans"]["name"]
                .as_str()
                .unwrap_or_else(|| v["i18n"]["en"]["name"].as_str().unwrap_or(&slug))
                .to_string();
            MarketCachedItem { id, slug, name, name_zh, icon, mr, max_rank, tags }
        })
        .collect();

    let count = items.len();
    let map: HashMap<String, MarketCachedItem> = items
        .into_iter()
        .map(|i| (i.slug.clone(), i))
        .collect();

    // Persist to app data dir.
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&map.values().collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    let tmp = app_data_dir.join("market_items.json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("写入失败: {e}"))?;
    std::fs::rename(&tmp, app_data_dir.join(FILE_NAME)).map_err(|e| format!("保存失败: {e}"))?;

    let now = crate::api::now_ms();
    {
        let mut c = cache.write().await;
        c.id_to_slug = build_id_map(&map);
        c.items = map;
        c.last_updated = Some(now);
    }

    let _ = app_handle.emit("market-cache-ready", count);
    Ok(count)
}

#[tauri::command]
pub async fn market_cache_status(
    cache: tauri::State<'_, SharedMarketCache>,
) -> Result<MarketCacheStatus, String> {
    let c = cache.read().await;
    let last = c.last_updated.map_or_else(
        || "--".into(),
        |ts| {
            let secs = (crate::api::now_ms() - ts) / 1000;
            if secs < 60 {
                "刚刚".into()
            } else if secs < 3600 {
                format!("{} 分钟前", secs / 60)
            } else if secs < 86400 {
                format!("{} 小时前", secs / 3600)
            } else {
                format!("{} 天前", secs / 86400)
            }
        },
    );
    Ok(MarketCacheStatus {
        count: c.items.len(),
        last_updated: last,
    })
}

#[tauri::command]
pub async fn translate_items(
    cache: tauri::State<'_, SharedMarketCache>,
    query: String,
) -> Result<Vec<MarketItemSummary>, String> {
    let c = cache.read().await;
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    // Strip punctuation so "充沛" matches "赋能·充沛" regardless of separator.
    fn is_cjk(c: char) -> bool {
        matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x3000..=0x303F)
    }
    let nq: String = q.chars().filter(|c| c.is_alphanumeric() || is_cjk(*c)).collect();
    if nq.is_empty() {
        return Ok(Vec::new());
    }
    let mut matched: Vec<(&MarketCachedItem, String)> = c.items
        .values()
        .filter_map(|item| {
            if item.name_zh.is_empty() { return None; }
            let nz: String = item.name_zh.chars().filter(|c| c.is_alphanumeric() || is_cjk(*c)).collect();
            if nz.contains(&nq) { Some((item, nz)) } else { None }
        })
        .collect();
    // Sort by relevance: exact match first, then shortest Chinese name
    matched.sort_by(|(_, a_nz), (_, b_nz)| {
        let a_exact = *a_nz == nq;
        let b_exact = *b_nz == nq;
        b_exact.cmp(&a_exact)
            .then_with(|| a_nz.len().cmp(&b_nz.len()))
    });
    Ok(matched.into_iter().take(20).map(|(item, _)| MarketItemSummary {
        slug: item.slug.clone(),
        name: item.name.clone(),
        name_zh: item.name_zh.clone(),
        icon_url: format!("{}{}", ICON_BASE, item.icon),
        mr: item.mr,
        max_rank: item.max_rank,
        tags: item.tags.clone(),
    }).collect())
}
