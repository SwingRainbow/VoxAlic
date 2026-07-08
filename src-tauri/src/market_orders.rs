//! Warframe.Market profile orders — CRUD.
//!
//! All functions require a valid auth token (obtained via `market_auth::ensure_valid_token`).
//! Item names/slugs are resolved from the `MarketCache` so the frontend receives
//! display-ready `ProfileOrder` objects.

use crate::market::SharedMarketCache;
use crate::market_auth::SharedMarketAuth;
use crate::market_auth::ensure_valid_token;
use crate::models::{MarketError, ProfileOrder, CreateOrderRequest, UpdateOrderRequest};
use tauri::Manager;

const MARKET_API_V1: &str = "https://api.warframe.market/v1";

// ── Helpers ───────────────────────────────────────────────────────────────

/// Map a `reqwest::Error` to a `MarketError` based on whether it's a timeout or connect error.
fn map_reqwest_err(e: reqwest::Error) -> MarketError {
    if e.is_timeout() || e.is_connect() {
        MarketError {
            code: "network_timeout".into(),
            message: "网络连接超时，请检查网络后重试".into(),
        }
    } else {
        MarketError {
            code: "network_timeout".into(),
            message: "网络连接超时，请检查网络后重试".into(),
        }
    }
}

/// Resolve item slug + display name from an item_id by searching the MarketCache.
fn resolve_item(cache: &crate::market::MarketCache, item_id: &str) -> (String, String) {
    // Look up slug from id_to_slug map.
    if let Some(slug) = cache.id_to_slug.get(item_id) {
        if let Some(item) = cache.items.get(slug) {
            let name = if item.name_zh.is_empty() {
                item.name.clone()
            } else {
                item.name_zh.clone()
            };
            return (slug.clone(), name);
        }
    }
    // Fallback: the slug might be the item_id itself (rare).
    // Or try scanning items for matching id field.
    for (_slug, item) in &cache.items {
        if item.id == item_id {
            let name = if item.name_zh.is_empty() {
                item.name.clone()
            } else {
                item.name_zh.clone()
            };
            return (item.slug.clone(), name);
        }
    }
    (String::new(), item_id.to_string())
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

/// Parse a single order from the /v1/profile/orders JSON element.
fn parse_profile_order(o: &serde_json::Value, cache: &crate::market::MarketCache) -> Option<ProfileOrder> {
    let id = o["id"].as_str().unwrap_or("");
    if id.is_empty() { return None; }
    let item_id = o["item_id"].as_str().unwrap_or("").to_string();
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
        creation_date: o["creation_date"].as_str().unwrap_or("").to_string(),
    })
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

    let token = ensure_valid_token(&auth, &app_data_dir).await?;

    let resp = crate::market::market_client()
        .get(format!("{}/profile/orders", MARKET_API_V1))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(map_reqwest_err)?;

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
            .filter_map(|o| parse_profile_order(o, &cache_read))
            .collect();
        // Sort: sells first, then by creation date descending
        // (actually keep API order — most recent first seems fine)
        Ok(orders)
    } else {
        let me = map_http_err(status, &body_text);
        // If auth expired, clear state.
        if me.code == "auth_expired" {
            let mut a = auth.inner.write().await;
            a.access_token = None;
            a.refresh_token = None;
            a.ingame_name = None;
            a.logged_in = false;
            crate::market_auth::clear_persisted(&app_data_dir);
        }
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

    let token = ensure_valid_token(&auth, &app_data_dir).await?;

    // Resolve item_id: frontend sends slug, API expects MongoDB ObjectId.
    let item_id = {
        let c = cache.read().await;
        if let Some(item) = c.items.get(&req.item_id) {
            if item.id.len() >= 20 {
                // ObjectId (24 hex chars) or similar long ID.
                item.id.clone()
            } else {
                // Embedded default: id == slug — pass through and hope.
                req.item_id.clone()
            }
        } else {
            // Not in cache — pass through slug as-is.
            req.item_id.clone()
        }
    };

    let resp = crate::market::market_client()
        .post(format!("{}/profile/orders", MARKET_API_V1))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .json(&serde_json::json!({
            "order_type": req.order_type,
            "item_id": item_id,
            "platinum": req.platinum,
            "quantity": req.quantity,
            "rank": req.rank,
            "visible": req.visible,
        }))
        .send()
        .await
        .map_err(map_reqwest_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();
    eprintln!("[market_orders] create_order status={} body[..300]={}", status, &body_text[..body_text.len().min(300)]);

    if status == 200 || status == 201 {
        let body: serde_json::Value =
            serde_json::from_str(&body_text).map_err(|_| MarketError {
                code: "server_error".into(),
                message: "服务器响应异常，请稍后再试".into(),
            })?;
        let data = body.get("data").unwrap_or(&body);
        let cache_read = cache.read().await;
        parse_profile_order(data, &cache_read).ok_or_else(|| MarketError {
            code: "server_error".into(),
            message: "服务器响应异常，请稍后再试".into(),
        })
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            let mut a = auth.inner.write().await;
            a.access_token = None;
            a.refresh_token = None;
            a.ingame_name = None;
            a.logged_in = false;
            crate::market_auth::clear_persisted(&app_data_dir);
        }
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

    let token = ensure_valid_token(&auth, &app_data_dir).await?;

    // Build a partial JSON body — only send fields that are Some.
    let mut body_map = serde_json::Map::new();
    if let Some(p) = req.platinum {
        body_map.insert("platinum".into(), serde_json::Value::from(p));
    }
    if let Some(q) = req.quantity {
        body_map.insert("quantity".into(), serde_json::Value::from(q));
    }
    if let Some(v) = req.visible {
        body_map.insert("visible".into(), serde_json::Value::from(v));
    }
    if let Some(r) = req.rank {
        body_map.insert("rank".into(), serde_json::Value::from(r));
    }

    let resp = crate::market::market_client()
        .put(format!("{}/profile/orders/{}", MARKET_API_V1, req.order_id))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .json(&serde_json::Value::Object(body_map))
        .send()
        .await
        .map_err(map_reqwest_err)?;

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
        parse_profile_order(data, &cache_read).ok_or_else(|| MarketError {
            code: "server_error".into(),
            message: "服务器响应异常，请稍后再试".into(),
        })
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            let mut a = auth.inner.write().await;
            a.access_token = None;
            a.refresh_token = None;
            a.ingame_name = None;
            a.logged_in = false;
            crate::market_auth::clear_persisted(&app_data_dir);
        }
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

    let token = ensure_valid_token(&auth, &app_data_dir).await?;

    let resp = crate::market::market_client()
        .delete(format!("{}/profile/orders/{}", MARKET_API_V1, order_id))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(map_reqwest_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();

    if status == 200 || status == 204 {
        Ok(())
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            let mut a = auth.inner.write().await;
            a.access_token = None;
            a.refresh_token = None;
            a.ingame_name = None;
            a.logged_in = false;
            crate::market_auth::clear_persisted(&app_data_dir);
        }
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

    let token = ensure_valid_token(&auth, &app_data_dir).await?;

    let resp = crate::market::market_client()
        .put(format!("{}/profile/orders/close/{}", MARKET_API_V1, order_id))
        .header("Authorization", &token)
        .header("Language", "zh-hans")
        .header("Platform", "pc")
        .send()
        .await
        .map_err(map_reqwest_err)?;

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();

    if status == 200 || status == 204 {
        Ok(())
    } else {
        let me = map_http_err(status, &body_text);
        if me.code == "auth_expired" {
            let mut a = auth.inner.write().await;
            a.access_token = None;
            a.refresh_token = None;
            a.ingame_name = None;
            a.logged_in = false;
            crate::market_auth::clear_persisted(&app_data_dir);
        }
        Err(me)
    }
}
