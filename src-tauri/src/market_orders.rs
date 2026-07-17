//! Warframe.Market profile orders — CRUD via v2 API.
//!
//! All functions require a valid auth token (obtained via `market_auth::ensure_valid_token`).
//! Item names/slugs are resolved from the `MarketCache` so the frontend receives
//! display-ready `ProfileOrder` objects.
//!
//! **v2 endpoint mapping** (June 2026):
//!   GET    /v2/orders/my        — list own orders
//!   POST   /v2/order            — create
//!   PATCH  /v2/order/{id}       — update
//!   DELETE /v2/order/{id}       — delete
//!   PUT    /v2/order/close/{id} — close (mark invisible)

use crate::market::SharedMarketCache;
use crate::market_auth::SharedMarketAuth;
use crate::market_auth::ensure_valid_token;
use crate::models::{MarketError, ProfileOrder, CreateOrderRequest, UpdateOrderRequest};
use log::{debug, warn};
use tauri::Manager;

const MARKET_V2: &str = "https://api.warframe.market/v2";

// ── Helpers ───────────────────────────────────────────────────────────────

/// Map a `reqwest::Error` to a `MarketError`.
pub(crate) fn network_err(_e: reqwest::Error) -> MarketError {
    MarketError {
        code: "network_timeout".into(),
        message: "网络连接超时，请检查网络后重试".into(),
    }
}

/// Resolve item slug + display name from an ObjectId by searching the MarketCache.
fn resolve_item(cache: &crate::market::MarketCache, object_id: &str) -> (String, String) {
    // Look up slug from id_to_slug map.
    if let Some(slug) = cache.id_to_slug.get(object_id) {
        if let Some(item) = cache.items.get(slug) {
            let name = if item.name_zh.is_empty() {
                item.name.clone()
            } else {
                item.name_zh.clone()
            };
            return (slug.clone(), name);
        }
    }
    // Fallback: scan items for matching id field.
    for (_slug, item) in &cache.items {
        if item.id == object_id {
            let name = if item.name_zh.is_empty() {
                item.name.clone()
            } else {
                item.name_zh.clone()
            };
            return (item.slug.clone(), name);
        }
    }
    (String::new(), object_id.to_string())
}

/// Resolve a slug to an ObjectId for API calls.
/// First tries cache, then falls back to GET /v2/item/{slug}.
async fn resolve_object_id(
    slug: &str,
    cache: &SharedMarketCache,
) -> Result<String, MarketError> {
    // 1. Try cache first.
    {
        let c = cache.read().await;
        if let Some(item) = c.items.get(slug) {
            if item.id.len() >= 20 {
                return Ok(item.id.clone());
            }
        }
    }

    // 2. Fetch from v2 API.
    let url = format!("{}/item/{}", crate::market::MARKET_API_BASE, slug);
    let resp = crate::market::market_client()
        .get(&url)
        .header("Language", "zh-hans")
        .send()
        .await
        .map_err(|_| MarketError {
            code: "network_timeout".into(),
            message: "获取物品信息超时，请检查网络后重试".into(),
        })?;

    if !resp.status().is_success() {
        warn!("[wm] resolve_object_id '{slug}' HTTP {}", resp.status().as_u16());
        return Err(MarketError {
            code: "invalid_input".into(),
            message: "该物品在 Warframe.Market 上不存在".into(),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|_| MarketError {
        code: "server_error".into(),
        message: "服务器响应异常".into(),
    })?;
    let object_id = body["data"]["id"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if object_id.is_empty() {
        return Err(MarketError {
            code: "invalid_input".into(),
            message: "无法获取物品 ID，请刷新物品库后重试".into(),
        });
    }

    // 3. Write back to cache so next time hits fast path.
    {
        let mut c = cache.write().await;
        if let Some(item) = c.items.get_mut(slug) {
            item.id = object_id.clone();
        }
        c.id_to_slug.insert(object_id.clone(), slug.to_string());
    }

    Ok(object_id)
}

/// Map HTTP status codes from order endpoints to MarketError.
fn map_http_err(status: u16, body: &str) -> MarketError {
    match status {
        400 => {
            let lower = body.to_lowercase();
            if lower.contains("duplicate") || lower.contains("already") {
                MarketError {
                    code: "duplicate_price".into(),
                    message: "此物品已有相同价格的挂单，请修改价格".into(),
                }
            } else if lower.contains("limit") || lower.contains("max") {
                MarketError {
                    code: "order_limit_reached".into(),
                    message: "此物品最多同时挂 3 单（不同价格），已达上限".into(),
                }
            } else if lower.contains("price") || lower.contains("platinum") {
                MarketError {
                    code: "invalid_price".into(),
                    message: "价格必须在 1 ～ 999,999 白金之间".into(),
                }
            } else if lower.contains("quantity") {
                MarketError {
                    code: "invalid_quantity".into(),
                    message: "数量必须在 1 ～ 100 之间".into(),
                }
            } else {
                MarketError {
                    code: "invalid_input".into(),
                    message: format!("请求参数无效: {}", body.chars().take(80).collect::<String>()),
                }
            }
        }
        401 => MarketError {
            code: "auth_expired".into(),
            message: "登录已过期，请重新登录".into(),
        },
        404 => MarketError {
            code: "order_not_found".into(),
            message: "该订单已不存在（可能已被删除）".into(),
        },
        429 => MarketError {
            code: "rate_limited".into(),
            message: "请求过于频繁，请稍候再试".into(),
        },
        code if code >= 500 => MarketError {
            code: "server_error".into(),
            message: "Warframe.Market 服务暂时不可用，请稍后再试".into(),
        },
        _ => MarketError {
            code: "unknown".into(),
            message: format!("服务器返回 HTTP {}", status),
        },
    }
}

/// Parse a single order from the v2 API JSON response (camelCase fields).
fn parse_order(o: &serde_json::Value, cache: &crate::market::MarketCache) -> Option<ProfileOrder> {
    let id = o["id"].as_str().unwrap_or("");
    if id.is_empty() {
        return None;
    }
    let item_id = o["itemId"].as_str().unwrap_or("").to_string();
    let (item_slug, item_name) = resolve_item(cache, &item_id);
    Some(ProfileOrder {
        id: id.to_string(),
        order_type: o["type"].as_str().unwrap_or("?").to_string(),
        item_id,
        item_slug,
        item_name,
        platinum: o["platinum"].as_u64().unwrap_or(0) as u32,
        quantity: o["quantity"].as_u64().unwrap_or(1) as u32,
        rank: o["rank"].as_u64().unwrap_or(0) as u8,
        visible: o["visible"].as_bool().unwrap_or(true),
        platform: o["platform"].as_str().unwrap_or("pc").to_string(),
        creation_date: o["createdAt"].as_str().unwrap_or("").to_string(),
    })
}

/// Clear auth state when token is expired.
async fn clear_auth(auth: &SharedMarketAuth, app_data_dir: &std::path::Path) {
    warn!("[wm] auth cleared — token rejected by API");
    let mut a = auth.inner.write().await;
    a.access_token = None;
    a.refresh_token = None;
    a.ingame_name = None;
    a.logged_in = false;
    crate::market_auth::clear_persisted(app_data_dir);
}

// ── Tauri commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn market_list_orders(
    auth: tauri::State<'_, SharedMarketAuth>,
    cache: tauri::State<'_, SharedMarketCache>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<ProfileOrder>, MarketError> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| MarketError {
            code: "unknown".into(),
            message: "无法获取应用数据目录".into(),
        })?;

    let token = ensure_valid_token(&auth, &app_data_dir, &app_handle).await?;

    let resp = crate::market::market_client()
        .get(format!("{}/orders/my", MARKET_V2))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(network_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();

    if status == 200 {
        let body: serde_json::Value =
            serde_json::from_str(&body_text).map_err(|_| MarketError {
                code: "server_error".into(),
                message: "服务器响应异常，请稍后再试".into(),
            })?;
        let data = body["data"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        let cache_read = cache.read().await;
        let orders: Vec<ProfileOrder> = data
            .iter()
            .filter_map(|o| parse_order(o, &cache_read))
            .collect();
        Ok(orders)
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            clear_auth(&auth, &app_data_dir).await;
        }
        warn!("[wm] list_orders HTTP {status}: {}", me.code);
        Err(me)
    }
}

#[tauri::command]
pub async fn market_create_order(
    auth: tauri::State<'_, SharedMarketAuth>,
    cache: tauri::State<'_, SharedMarketCache>,
    app_handle: tauri::AppHandle,
    req: CreateOrderRequest,
) -> Result<ProfileOrder, MarketError> {
    // Basic validation.
    if req.platinum < 1 || req.platinum > 999_999 {
        return Err(MarketError {
            code: "invalid_price".into(),
            message: "价格必须在 1 ～ 999,999 白金之间".into(),
        });
    }
    if req.quantity < 1 || req.quantity > 100 {
        return Err(MarketError {
            code: "invalid_quantity".into(),
            message: "数量必须在 1 ～ 100 之间".into(),
        });
    }

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| MarketError {
            code: "unknown".into(),
            message: "无法获取应用数据目录".into(),
        })?;

    let token = ensure_valid_token(&auth, &app_data_dir, &app_handle).await?;

    // Resolve slug → ObjectId (cache-first, API fallback).
    let object_id = resolve_object_id(&req.item_id, &cache).await?;

    // Build JSON body (camelCase keys for v2 API).
    let mut body_map = serde_json::Map::new();
    body_map.insert("type".into(), serde_json::Value::String(req.order_type.clone()));
    body_map.insert("itemId".into(), serde_json::Value::String(object_id));
    body_map.insert("platinum".into(), serde_json::Value::from(req.platinum));
    body_map.insert("quantity".into(), serde_json::Value::from(req.quantity));
    body_map.insert("visible".into(), serde_json::Value::Bool(req.visible));
    // perTrade + rank: only include when the item supports them.
    {
        let c = cache.read().await;
        if let Some(item) = c.items.get(&req.item_id) {
            if item.bulk_tradable {
                body_map.insert("perTrade".into(), serde_json::Value::from(1));
            }
            if item.max_rank.unwrap_or(0) > 0 {
                body_map.insert("rank".into(), serde_json::Value::from(req.rank));
            }
        }
    }

    let resp = crate::market::market_client()
        .post(format!("{}/order", MARKET_V2))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .json(&serde_json::Value::Object(body_map))
        .send()
        .await
        .map_err(network_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();
    debug!(
        "[market_orders] create_order status={} body[..300]={}",
        status,
        &body_text[..body_text.len().min(300)]
    );

    if status == 200 || status == 201 {
        let body: serde_json::Value =
            serde_json::from_str(&body_text).map_err(|_| MarketError {
                code: "server_error".into(),
                message: "服务器响应异常，请稍后再试".into(),
            })?;
        let data = body.get("data").unwrap_or(&body);
        let cache_read = cache.read().await;
        parse_order(data, &cache_read).ok_or_else(|| MarketError {
            code: "server_error".into(),
            message: "服务器响应异常，请稍后再试".into(),
        })
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            clear_auth(&auth, &app_data_dir).await;
        }
        warn!("[wm] create_order HTTP {status}: {}", me.code);
        Err(me)
    }
}

#[tauri::command]
pub async fn market_update_order(
    auth: tauri::State<'_, SharedMarketAuth>,
    cache: tauri::State<'_, SharedMarketCache>,
    app_handle: tauri::AppHandle,
    req: UpdateOrderRequest,
) -> Result<ProfileOrder, MarketError> {
    if let Some(p) = req.platinum {
        if p < 1 || p > 999_999 {
            return Err(MarketError {
                code: "invalid_price".into(),
                message: "价格必须在 1 ～ 999,999 白金之间".into(),
            });
        }
    }
    if let Some(q) = req.quantity {
        if q < 1 || q > 100 {
            return Err(MarketError {
                code: "invalid_quantity".into(),
                message: "数量必须在 1 ～ 100 之间".into(),
            });
        }
    }

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| MarketError {
            code: "unknown".into(),
            message: "无法获取应用数据目录".into(),
        })?;

    let token = ensure_valid_token(&auth, &app_data_dir, &app_handle).await?;

    // Build a partial JSON body — only send fields that are Some.
    let mut body_map = serde_json::Map::new();
    if let Some(p) = req.platinum {
        body_map.insert("platinum".into(), serde_json::Value::from(p));
    }
    if let Some(q) = req.quantity {
        body_map.insert("quantity".into(), serde_json::Value::from(q));
    }
    if let Some(v) = req.visible {
        body_map.insert("visible".into(), serde_json::Value::Bool(v));
    }
    if let Some(r) = req.rank {
        if r > 0 {
            body_map.insert("rank".into(), serde_json::Value::from(r));
        }
    }

    let resp = crate::market::market_client()
        .request(
            reqwest::Method::PATCH,
            format!("{}/order/{}", MARKET_V2, req.order_id),
        )
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .json(&serde_json::Value::Object(body_map))
        .send()
        .await
        .map_err(network_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();

    if status == 200 {
        let body: serde_json::Value =
            serde_json::from_str(&body_text).map_err(|_| MarketError {
                code: "server_error".into(),
                message: "服务器响应异常，请稍后再试".into(),
            })?;
        let data = body.get("data").unwrap_or(&body);
        let cache_read = cache.read().await;
        parse_order(data, &cache_read).ok_or_else(|| MarketError {
            code: "server_error".into(),
            message: "服务器响应异常，请稍后再试".into(),
        })
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            clear_auth(&auth, &app_data_dir).await;
        }
        warn!("[wm] update_order HTTP {status}: {}", me.code);
        Err(me)
    }
}

#[tauri::command]
pub async fn market_delete_order(
    auth: tauri::State<'_, SharedMarketAuth>,
    app_handle: tauri::AppHandle,
    order_id: String,
) -> Result<(), MarketError> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| MarketError {
            code: "unknown".into(),
            message: "无法获取应用数据目录".into(),
        })?;

    let token = ensure_valid_token(&auth, &app_data_dir, &app_handle).await?;

    let resp = crate::market::market_client()
        .delete(format!("{}/order/{}", MARKET_V2, order_id))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(network_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();

    if status == 200 || status == 204 {
        Ok(())
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            clear_auth(&auth, &app_data_dir).await;
        }
        warn!("[wm] delete_order HTTP {status}: {}", me.code);
        Err(me)
    }
}

#[tauri::command]
pub async fn market_close_order(
    auth: tauri::State<'_, SharedMarketAuth>,
    app_handle: tauri::AppHandle,
    order_id: String,
) -> Result<(), MarketError> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| MarketError {
            code: "unknown".into(),
            message: "无法获取应用数据目录".into(),
        })?;

    let token = ensure_valid_token(&auth, &app_data_dir, &app_handle).await?;

    let resp = crate::market::market_client()
        .put(format!("{}/order/close/{}", MARKET_V2, order_id))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(network_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();

    if status == 200 || status == 204 {
        Ok(())
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            clear_auth(&auth, &app_data_dir).await;
        }
        warn!("[wm] close_order HTTP {status}: {}", me.code);
        Err(me)
    }
}
