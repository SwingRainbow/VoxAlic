//! Item-name localisation for Baro's manifest (and any other asset-path item).
//!
//! Source of truth is WFCD/warframe-items `data/json/i18n.json`, keyed by the
//! game's `uniqueName` asset path. We ship a compact `uniqueName -> 简中` map
//! (`resources/baro_zh.json`, embedded at compile time) so translation works
//! offline on first run, and allow the user to refresh it from GitHub at any
//! time — the fresh map is persisted to the app data dir and hot-swapped in.
//!
//! Lookup handles Baro's `/Lotus/StoreItems/...` paths by also trying the form
//! with the `StoreItems/` segment removed, which is how the i18n keys are named.

use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// gzip-compressed `uniqueName -> 简中名` map (build.rs compresses resources/baro_zh.json).
static EMBEDDED_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/baro_zh_compressed.bin"));
/// Upstream full i18n table (51 MB, 14 languages). master branch only.
const REMOTE_URL: &str =
    "https://raw.githubusercontent.com/WFCD/warframe-items/master/data/json/i18n.json";
/// Override file written to the app data dir by `update_from_remote`.
const FILE_NAME: &str = "baro_zh.json";

static MAP: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

fn cell() -> &'static RwLock<HashMap<String, String>> {
    MAP.get_or_init(|| RwLock::new(load_compressed().unwrap_or_default()))
}

fn load_compressed() -> Result<HashMap<String, String>, String> {
    let decoder = GzDecoder::new(EMBEDDED_GZ);
    serde_json::from_reader(decoder).map_err(|e| format!("decompress baro_zh: {}", e))
}

/// Load the user's override map from the app data dir if present; otherwise the
/// embedded default is used. Call once at startup.
pub fn init(app_data_dir: &Path) {
    let path = app_data_dir.join(FILE_NAME);
    if let Ok(s) = std::fs::read_to_string(&path) {
        if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(&s) {
            if !m.is_empty() {
                *cell().write().unwrap() = m;
                return;
            }
        }
    }
    // Force lazy init of the embedded default so `count()` is meaningful.
    let _ = cell();
}

/// Translate an asset path (e.g. a Baro `ItemType`) to 简中, if known.
/// Tries the path verbatim, then with the `StoreItems/` segment stripped.
pub fn translate(path: &str) -> Option<String> {
    let map = cell().read().unwrap();
    if let Some(v) = map.get(path) {
        return Some(v.clone());
    }
    let stripped = path.replacen("/StoreItems/", "/", 1);
    if stripped != path {
        if let Some(v) = map.get(&stripped) {
            return Some(v.clone());
        }
    }
    None
}

/// Number of entries currently loaded.
pub fn count() -> usize {
    cell().read().unwrap().len()
}

#[derive(serde::Deserialize)]
struct RawEntry {
    zh: Option<LangName>,
}

#[derive(serde::Deserialize)]
struct LangName {
    name: Option<String>,
}

/// Download the latest i18n.json from WFCD, extract the 简中 names into a compact
/// map, persist it to `{app_data_dir}/baro_zh.json`, and hot-swap the in-memory
/// table. Returns the number of translated entries. Heavy (~51 MB download).
pub async fn update_from_remote(app_data_dir: PathBuf) -> Result<usize, String> {
    let client = reqwest::Client::builder()
        .user_agent("Warframe/1.0")
        // Generous: the i18n payload is ~51 MB.
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(REMOTE_URL)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("下载失败: {e}"))?;

    // Parse only the `zh.name` field of each entry; serde ignores the rest.
    let raw: HashMap<String, RawEntry> =
        serde_json::from_slice(&bytes).map_err(|e| format!("解析失败: {e}"))?;
    let compact: HashMap<String, String> = raw
        .into_iter()
        .filter_map(|(k, v)| v.zh.and_then(|z| z.name).map(|n| (k, n)))
        .collect();
    if compact.is_empty() {
        return Err("解析结果为空（数据源格式可能已变更）".into());
    }
    let count = compact.len();

    std::fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&compact).map_err(|e| e.to_string())?;
    std::fs::write(app_data_dir.join(FILE_NAME), json).map_err(|e| e.to_string())?;

    *cell().write().unwrap() = compact;
    Ok(count)
}
