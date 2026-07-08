//! Warframe.Market authentication — signin, signout, token lifecycle.
//!
//! **CSRF flow**: warframe.market requires a CSRF token retrieved from
//! `GET warframe.market/v1/auth/signin` before POSTing credentials. The same
//! CSRF token + session cookie is needed for token refresh.
//!
//! Tokens + device_id are persisted to `{app_data_dir}/market_auth.json`.
//! CSRF tokens / session cookies are ephemeral — fetched on-demand, never stored.

use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tauri::Manager;

use crate::models::{MarketError, MarketAuthStatus};

const MARKET_WEB: &str = "https://warframe.market";
const MARKET_API_V1: &str = "https://api.warframe.market/v1";
const AUTH_FILE: &str = "market_auth.json";

// ── HTTP clients ──────────────────────────────────────────────────────────

/// Shared client for API calls (signin POST, refresh POST, signout GET).
/// No timeout so slow connections from China can complete.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("VoxAlic/1.0")
            .build()
            .expect("reqwest::Client::build")
    })
}

/// One-off client for CSRF page fetch — no timeout because
/// warframe.market website can be very slow from China.
fn csrf_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("VoxAlic/1.0")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

// ── Auth file schema ──────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AuthFile {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    ingame_name: Option<String>,
}

// ── Runtime state ─────────────────────────────────────────────────────────

pub struct MarketAuthInner {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub device_id: String,
    pub ingame_name: Option<String>,
    pub logged_in: bool,
}

pub struct MarketAuth {
    pub inner: RwLock<MarketAuthInner>,
    refresh_mu: tokio::sync::Mutex<()>,
}

pub type SharedMarketAuth = Arc<MarketAuth>;

// ── CSRF helper: shared by signin and refresh ────────────────────────────

/// Result of fetching the warframe.market signin page for CSRF token + cookie.
struct CsrfContext {
    csrf_token: String,
    session_cookie: String, // "name=value; name2=value2" for Cookie header
}

/// GET warframe.market/v1/auth/signin → extract CSRF token + session cookie(s).
async fn fetch_csrf() -> Result<CsrfContext, MarketError> {
    let url = format!("{}/auth/signin", MARKET_WEB);
    eprintln!("[market_auth] CSRF GET {} ...", url);
    let resp = csrf_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| {
            let detail = format!("{}", e);
            eprintln!("[market_auth] CSRF GET failed: {}", detail);
            if e.is_timeout() {
                MarketError { code: "network_timeout".into(), message: "连接 warframe.market 超时，请检查网络".into() }
            } else {
                MarketError { code: "network_timeout".into(), message: format!("网络错误: {}", detail) }
            }
        })?;

    let status = resp.status().as_u16();
    // Snapshot set-cookie headers before consuming body (get_all keeps multiples).
    let set_cookie_vals: Vec<String> = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();
    let body = resp.text().await.unwrap_or_default();

    eprintln!("[market_auth] CSRF GET status={} body_len={}", status, body.len());
    if status != 200 {
        eprintln!("[market_auth] CSRF GET error body: {}", &body[..body.len().min(300)]);
        return Err(MarketError {
            code: "server_error".into(),
            message: format!("CSRF 页面返回 HTTP {}", status),
        });
    }

    // Extract cookies from Set-Cookie response headers.
    let mut cookie_parts: Vec<String> = Vec::new();
    for v in &set_cookie_vals {
        // Take only "name=value" (before first ';') — the rest is attributes.
        if let Some(pair) = v.split(';').next() {
            cookie_parts.push(pair.trim().to_string());
        }
    }
    let session_cookie = cookie_parts.join("; ");
    eprintln!("[market_auth] CSRF cookies: {} parts, each:", cookie_parts.len());
    for (i, p) in cookie_parts.iter().enumerate() {
        // Log first 40 chars of each cookie value (truncate JWT values).
        let preview: String = p.chars().take(40).collect();
        eprintln!("[market_auth]   cookie[{}]: {}", i, preview);
    }

    // Extract CSRF token from HTML.
    let csrf_token = extract_csrf_from_html(&body).ok_or_else(|| {
        eprintln!("[market_auth] CSRF token NOT found in body[..500]={}", &body[..body.len().min(500)]);
        MarketError {
            code: "server_error".into(),
            message: "无法获取 CSRF token，请稍后重试".into(),
        }
    })?;
    eprintln!("[market_auth] CSRF token (already has ## prefix): '{}'", &csrf_token);

    Ok(CsrfContext {
        csrf_token,
        session_cookie,
    })
}

fn extract_csrf_from_html(html: &str) -> Option<String> {
    // Search for <meta name="csrf-token" content="VALUE">.
    let needle = r#"name="csrf-token""#;
    let pos = html.find(needle)?;
    let after = &html[pos..];

    // Log context around the match for debugging.
    let ctx_start = pos.saturating_sub(20);
    let ctx_end = (pos + 200).min(html.len());
    eprintln!("[market_auth] CSRF HTML context: ...{}...", &html[ctx_start..ctx_end]);

    // Find content= attribute after the csrf-token name.
    let content_needle = "content=";
    let content_pos = after.find(content_needle)?;
    let after_content = &after[content_pos + content_needle.len()..];
    let quote = after_content.chars().next()?;
    let value_start = if quote == '"' || quote == '\'' { 1 } else { 0 };
    let value_end = if value_start == 1 {
        after_content[value_start..].find(quote)?
    } else {
        after_content.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(after_content.len())
    };
    let raw = &after_content[value_start..value_start + value_end];
    eprintln!("[market_auth] CSRF raw token: '{}' (len={})", raw, raw.len());
    Some(raw.to_string())
}

// ── JWT helpers ───────────────────────────────────────────────────────────

fn jwt_sub(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let padded = match payload.len() % 4 {
        2 => format!("{}==", payload),
        3 => format!("{}=", payload),
        _ => payload.to_string(),
    };
    let decoded = base64_url_decode(&padded)?;
    let v: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    v.get("sub").and_then(|s| s.as_str()).map(String::from)
}

// ── Disk persistence ──────────────────────────────────────────────────────

fn auth_path(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    app_data_dir.join(AUTH_FILE)
}

fn generate_device_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    format!("voxalic-{:016x}", h.finish())
}

fn persist(ai: &MarketAuthInner, app_data_dir: &std::path::Path) {
    let af = AuthFile {
        access_token: ai.access_token.clone(),
        refresh_token: ai.refresh_token.clone(),
        device_id: ai.device_id.clone(),
        ingame_name: ai.ingame_name.clone(),
    };
    let path = auth_path(app_data_dir);
    if let Ok(json) = serde_json::to_string(&af) {
        let tmp = path.with_extension("json.tmp");
        let _ = std::fs::write(&tmp, &json);
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub(crate) fn clear_persisted(app_data_dir: &std::path::Path) {
    let _ = std::fs::remove_file(auth_path(app_data_dir));
}

// ── Construction ──────────────────────────────────────────────────────────

pub fn load_or_create_auth(app_data_dir: &std::path::Path) -> SharedMarketAuth {
    let path = auth_path(app_data_dir);
    let (access_token, refresh_token, device_id, ingame_name) =
        if let Ok(s) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<AuthFile>(&s) {
                Ok(a) => (a.access_token, a.refresh_token, a.device_id, a.ingame_name),
                Err(_) => (None, None, String::new(), None),
            }
        } else {
            (None, None, String::new(), None)
        };

    let device_id = if device_id.is_empty() {
        generate_device_id()
    } else {
        device_id
    };

    let logged_in = access_token.is_some();

    Arc::new(MarketAuth {
        inner: RwLock::new(MarketAuthInner {
            access_token,
            refresh_token,
            device_id,
            ingame_name,
            logged_in,
        }),
        refresh_mu: tokio::sync::Mutex::new(()),
    })
}

// ── Token lifecycle ───────────────────────────────────────────────────────

pub async fn ensure_valid_token(
    auth: &SharedMarketAuth,
    app_data_dir: &std::path::Path,
) -> Result<String, MarketError> {
    // Fast path: token present.
    {
        let a = auth.inner.read().await;
        if let Some(ref token) = a.access_token {
            return Ok(format!("Bearer {}", token));
        }
        if a.refresh_token.is_none() {
            return Err(MarketError {
                code: "auth_expired".into(),
                message: "登录已过期，请重新登录".into(),
            });
        }
    }

    let _guard = auth.refresh_mu.lock().await;

    // Double-check.
    {
        let a = auth.inner.read().await;
        if let Some(ref token) = a.access_token {
            return Ok(format!("Bearer {}", token));
        }
    }

    let refresh_token = {
        let a = auth.inner.read().await;
        a.refresh_token.clone()
    };

    match refresh_token {
        Some(rt) => {
            match do_refresh_with_csrf(&rt).await {
                Ok(new_access) => {
                    let mut a = auth.inner.write().await;
                    a.access_token = Some(new_access.clone());
                    persist(&a, app_data_dir);
                    Ok(format!("Bearer {}", new_access))
                }
                Err(_) => {
                    let mut a = auth.inner.write().await;
                    a.access_token = None;
                    a.refresh_token = None;
                    a.ingame_name = None;
                    a.logged_in = false;
                    clear_persisted(app_data_dir);
                    Err(MarketError {
                        code: "auth_expired".into(),
                        message: "登录已过期，请重新登录".into(),
                    })
                }
            }
        }
        None => Err(MarketError {
            code: "auth_expired".into(),
            message: "登录已过期，请重新登录".into(),
        }),
    }
}

/// Refresh with CSRF protection — same 2-step flow as signin.
async fn do_refresh_with_csrf(refresh_token: &str) -> Result<String, MarketError> {
    let ctx = fetch_csrf().await?;

    let resp = client()
        .post(format!("{}/auth/refresh", MARKET_API_V1))
        .header("Cookie", &ctx.session_cookie)
        .header("x-csrf-token", &ctx.csrf_token)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await
        .map_err(|_e| MarketError {
            code: "network_timeout".into(),
            message: "网络连接超时，请检查网络后重试".into(),
        })?;

    let status = resp.status().as_u16();
    // Collect Set-Cookie from the refresh response (new JWT).
    let set_cookie_headers: Vec<String> = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .collect();

    match status {
        200 => {
            // Try Set-Cookie first, then JSON body as fallback.
            let new_jwt = extract_jwt_from_cookies(&set_cookie_headers)
                .or_else(|| {
                    // Fallback: try JSON body for access_token.
                    // We can't re-read the body, so just try the cookies.
                    None
                });
            new_jwt.ok_or_else(|| MarketError {
                code: "server_error".into(),
                message: "服务器响应异常，请稍后再试".into(),
            })
        }
        401 => Err(MarketError {
            code: "auth_expired".into(),
            message: "登录已过期，请重新登录".into(),
        }),
        429 => Err(MarketError {
            code: "rate_limited".into(),
            message: "请求过于频繁，请稍候再试".into(),
        }),
        code if code >= 500 => Err(MarketError {
            code: "server_error".into(),
            message: "Warframe.Market 服务暂时不可用，请稍后再试".into(),
        }),
        _ => Err(MarketError {
            code: "unknown".into(),
            message: format!("刷新 token 失败 (HTTP {})", status),
        }),
    }
}

/// Extract the JWT value from Set-Cookie headers.
fn extract_jwt_from_cookies(headers: &[String]) -> Option<String> {
    for h in headers {
        let parts: Vec<&str> = h.split(';').collect();
        for part in &parts {
            let trimmed = part.trim();
            if let Some((name, value)) = trimmed.split_once('=') {
                // Warframe.market uses "JWT" as the cookie name.
                if name.eq_ignore_ascii_case("JWT") {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

// ── Tauri commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn market_signin(
    auth: tauri::State<'_, SharedMarketAuth>,
    app_handle: tauri::AppHandle,
    email: String,
    password: String,
) -> Result<MarketAuthStatus, MarketError> {
    if email.trim().is_empty() || !email.contains('@') {
        return Err(MarketError { code: "invalid_input".into(), message: "请输入有效的邮箱地址".into() });
    }
    if password.is_empty() {
        return Err(MarketError { code: "invalid_input".into(), message: "请输入密码".into() });
    }

    let ctx = fetch_csrf().await?;
    let device_id = { auth.inner.read().await.device_id.clone() };

    let resp = client()
        .post(format!("{}/auth/signin", MARKET_API_V1))
        .header("Cookie", &ctx.session_cookie)
        .header("x-csrf-token", &ctx.csrf_token)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "email": email.trim(),
            "password": &password,
            "device_id": &device_id,
            "auth_type": "cookie",
        }))
        .send()
        .await
        .map_err(|e| {
            eprintln!("[market_auth] signin POST failed: {:?}", e);
            MarketError { code: "network_timeout".into(), message: "网络连接超时，请检查网络后重试".into() }
        })?;

    let status = resp.status().as_u16();
    let set_cookie_headers: Vec<String> = resp.headers().get_all("set-cookie").iter()
        .filter_map(|v| v.to_str().ok()).map(|s| s.to_string()).collect();
    let body_text = resp.text().await.unwrap_or_default();
    eprintln!("[market_auth] signin POST status={} set-cookie-count={} body[..300]={}",
        status, set_cookie_headers.len(), &body_text[..body_text.len().min(300)]);

    match status {
        200 | 302 => {
            // Try Set-Cookie first, fall back to JSON body.
            let access_token = extract_jwt_from_cookies(&set_cookie_headers)
                .or_else(|| {
                    serde_json::from_str::<serde_json::Value>(&body_text).ok()
                        .and_then(|v| {
                            v["access_token"].as_str()
                                .or_else(|| v["payload"]["access_token"].as_str())
                                .or_else(|| v["token"].as_str())
                                .or_else(|| v["payload"]["token"].as_str())
                                .or_else(|| v["id_token"].as_str())
                                .map(String::from)
                        })
                });
            let ingame_name = access_token.as_deref().and_then(jwt_sub)
                .or_else(|| {
                    serde_json::from_str::<serde_json::Value>(&body_text).ok()
                        .and_then(|v| {
                            v["payload"]["user"]["ingame_name"].as_str().map(String::from)
                        })
                });
            match (access_token, ingame_name) {
                (Some(at), Some(name)) => {
                    let app_data_dir = app_handle.path().app_data_dir().map_err(|_| MarketError {
                        code: "unknown".into(), message: "无法获取应用数据目录".into(),
                    })?;
                    let mut a = auth.inner.write().await;
                    a.access_token = Some(at);
                    a.ingame_name = Some(name.clone());
                    a.logged_in = true;
                    persist(&a, &app_data_dir);
                    Ok(MarketAuthStatus { logged_in: true, ingame_name: Some(name) })
                }
                _ => Err(MarketError {
                    code: "server_error".into(),
                    message: format!("服务器响应异常 body:{}", &body_text.chars().take(200).collect::<String>()),
                }),
            }
        }
        401 => {
            let body_lower = body_text.to_lowercase();
            let code = if body_lower.contains("email") || body_lower.contains("user") { "email_not_found" } else { "wrong_password" };
            Err(MarketError {
                code: code.into(),
                message: if code == "email_not_found" { "该邮箱未注册 Warframe.Market 账号".into() } else { "密码错误，请重试".into() },
            })
        }
        429 => Err(MarketError { code: "rate_limited".into(), message: "请求过于频繁，请稍候再试".into() }),
        code if code >= 500 => Err(MarketError { code: "server_error".into(), message: "Warframe.Market 服务暂时不可用，请稍后再试".into() }),
        _ => Err(MarketError { code: "unknown".into(), message: format!("登录失败 (HTTP {})", status) }),
    }
}

#[tauri::command]
pub async fn market_signout(
    auth: tauri::State<'_, SharedMarketAuth>,
    app_handle: tauri::AppHandle,
) -> Result<(), MarketError> {
    let access_token = {
        let a = auth.inner.read().await;
        a.access_token.clone()
    };

    if let Some(token) = access_token {
        let _ = client()
            .get(format!("{}/auth/signout", MARKET_API_V1))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await;
    }

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| MarketError {
            code: "unknown".into(),
            message: "无法获取应用数据目录".into(),
        })?;

    {
        let mut a = auth.inner.write().await;
        a.access_token = None;
        a.refresh_token = None;
        a.ingame_name = None;
        a.logged_in = false;
    }
    clear_persisted(&app_data_dir);

    Ok(())
}

#[tauri::command]
pub async fn market_auth_status(
    auth: tauri::State<'_, SharedMarketAuth>,
) -> Result<MarketAuthStatus, MarketError> {
    let a = auth.inner.read().await;
    Ok(MarketAuthStatus {
        logged_in: a.logged_in,
        ingame_name: a.ingame_name.clone(),
    })
}

// ── base64url decoder ─────────────────────────────────────────────────────

fn base64_url_decode(input: &str) -> Option<Vec<u8>> {
    use std::collections::HashMap;
    let alphabet: Vec<char> =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
            .chars()
            .collect();
    let mut char_to_val = HashMap::new();
    for (i, c) in alphabet.iter().enumerate() {
        char_to_val.insert(*c, i as u8);
    }

    let in_bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut buf = vec![0u8; (in_bytes.len() * 3) / 4 + 4];
    let n_blocks = in_bytes.len() / 4;
    let mut write_pos = 0usize;

    for block in 0..n_blocks {
        let ni: Vec<u8> = in_bytes[block * 4..(block + 1) * 4]
            .iter()
            .filter_map(|b| char_to_val.get(&(*b as char)).copied())
            .collect();
        if ni.len() != 4 {
            return None;
        }
        let triple =
            (ni[0] as u32) << 18 | (ni[1] as u32) << 12 | (ni[2] as u32) << 6 | (ni[3] as u32);
        buf[write_pos] = (triple >> 16) as u8;
        write_pos += 1;
        buf[write_pos] = (triple >> 8) as u8;
        write_pos += 1;
        buf[write_pos] = triple as u8;
        write_pos += 1;
    }

    let rem = in_bytes.len() % 4;
    if rem > 0 {
        let mut ni: Vec<u8> = in_bytes[n_blocks * 4..]
            .iter()
            .filter_map(|b| char_to_val.get(&(*b as char)).copied())
            .collect();
        while ni.len() < 4 {
            ni.push(0);
        }
        let triple =
            (ni[0] as u32) << 18 | (ni[1] as u32) << 12 | (ni[2] as u32) << 6 | (ni[3] as u32);
        buf[write_pos] = (triple >> 16) as u8;
        write_pos += 1;
        if rem >= 3 {
            buf[write_pos] = (triple >> 8) as u8;
            write_pos += 1;
        }
    }

    buf.truncate(write_pos);
    Some(buf)
}
