//! Warframe.Market authentication — signin, signout, token lifecycle.
//!
//! **CSRF flow**: warframe.market requires a CSRF token retrieved from the
//! signin page HTML `<meta name="csrf-token">`.  Because Cloudflare serves
//! a JS Challenge to non-browser HTTP clients, we use a hidden WebView
//! (real browser TLS fingerprint) to fetch the page and extract the token.
//!
//! Tokens + device_id are persisted to `{app_data_dir}/market_auth.json`.
//! CSRF tokens / session cookies are ephemeral — fetched on-demand, never stored.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::RwLock;
use tauri::Manager;
use tauri::Emitter;
use futures_util::SinkExt;
use futures_util::StreamExt;

use base64::Engine;
use crate::models::{MarketError, MarketAuthStatus};
use log::debug;

const MARKET_WEB: &str = "https://warframe.market";
const MARKET_API_V1: &str = "https://api.warframe.market/v1";
const AUTH_FILE: &str = "market_auth.json";
const WS_URL: &str = "wss://ws.warframe.market/socket";

// ── WebSocket command channel ───────────────────────────────────────────────

/// Commands sent from Tauri command handlers to the persistent WebSocket task.
pub(crate) enum WsCommand {
    SetStatus(String),
    Shutdown,
}

// ── HTTP clients ──────────────────────────────────────────────────────────

/// Shared client for API calls with DNS-hijack bypass.
/// Hardcodes real Cloudflare IPs as fallback for warframe.market domains.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let real_ips: Vec<SocketAddr> = [
            "104.26.0.182", "104.26.1.182", "172.67.75.162",
        ].iter().map(|ip| SocketAddr::new(IpAddr::V4(ip.parse::<Ipv4Addr>().unwrap()), 443)).collect();

        reqwest::Client::builder()
            .user_agent("VoxAlic/1.0")
            .resolve_to_addrs("warframe.market", &real_ips)
            .resolve_to_addrs("api.warframe.market", &real_ips)
            .build()
            .expect("reqwest::Client::build")
    })
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
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    reputation: Option<i32>,
}

// ── Runtime state ─────────────────────────────────────────────────────────

pub struct MarketAuthInner {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub device_id: String,
    pub ingame_name: Option<String>,
    pub avatar: Option<String>,
    pub reputation: Option<i32>,
    pub logged_in: bool,
    pub current_status: Option<String>,
    pub ws_tx: Option<tokio::sync::mpsc::Sender<WsCommand>>,
    pub login_in_progress: bool,
}

pub struct MarketAuth {
    pub inner: RwLock<MarketAuthInner>,
    refresh_mu: tokio::sync::Mutex<()>,
}

pub type SharedMarketAuth = Arc<MarketAuth>;

// ── JWT helpers ───────────────────────────────────────────────────────────

fn jwt_sub(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
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
        avatar: ai.avatar.clone(),
        reputation: ai.reputation,
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
    let (access_token, refresh_token, device_id, ingame_name, avatar, reputation) =
        if let Ok(s) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<AuthFile>(&s) {
                Ok(a) => (a.access_token, a.refresh_token, a.device_id, a.ingame_name, a.avatar, a.reputation),
                Err(_) => (None, None, String::new(), None, None, None),
            }
        } else {
            (None, None, String::new(), None, None, None)
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
            avatar,
            reputation,
            logged_in,
            current_status: None,
            ws_tx: None,
            login_in_progress: false,
        }),
        refresh_mu: tokio::sync::Mutex::new(()),
    })
}

// ── Token lifecycle ───────────────────────────────────────────────────────

pub async fn ensure_valid_token(
    auth: &SharedMarketAuth,
    app_data_dir: &std::path::Path,
    app_handle: &tauri::AppHandle,
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
            match do_refresh_with_csrf(&rt, app_handle).await {
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

async fn do_refresh_with_csrf(
    refresh_token: &str,
    _app_handle: &tauri::AppHandle,
) -> Result<String, MarketError> {
    let ctx = fetch_csrf().await?;
    let resp = client()
        .post(format!("{}/auth/refresh", MARKET_API_V1))
        .header("Cookie", &ctx.session_cookie)
        .header("x-csrf-token", &ctx.csrf_token)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send().await.map_err(crate::market_orders::network_err)?;

    let status = resp.status().as_u16();
    let set_cookie_headers: Vec<String> = resp.headers().get_all("set-cookie").iter()
        .filter_map(|v| v.to_str().ok()).map(|s| s.to_string()).collect();

    match status {
        200 => extract_jwt_from_cookies(&set_cookie_headers).ok_or_else(|| MarketError {
            code: "server_error".into(), message: "服务器响应异常".into(),
        }),
        401 => Err(MarketError { code: "auth_expired".into(), message: "登录已过期".into() }),
        429 => Err(MarketError { code: "rate_limited".into(), message: "请求过于频繁".into() }),
        c if c >= 500 => Err(MarketError { code: "server_error".into(), message: "服务暂时不可用".into() }),
        _ => Err(MarketError { code: "unknown".into(), message: format!("刷新失败 (HTTP {})", status) }),
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

// ── Persistent WebSocket (status sync) ─────────────────────────────────────

/// Read the Windows system proxy setting from registry:
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
/// Returns `host:port` if proxy is enabled, empty string otherwise.
fn read_windows_proxy() -> String {
    use windows::Win32::System::Registry::*;
    use windows::core::PCWSTR;
    const SUBKEY: PCWSTR = windows::core::w!(
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings"
    );
    const VALUE: PCWSTR = windows::core::w!("ProxyServer");
    const ENABLE: PCWSTR = windows::core::w!("ProxyEnable");

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, SUBKEY, 0, KEY_READ, &mut hkey).is_err() {
            return String::new();
        }

        // Check ProxyEnable first.
        let mut enabled: u32 = 0;
        let mut size = 4u32;
        if RegQueryValueExW(
            hkey,
            ENABLE,
            None,
            None,
            Some(&mut enabled as *mut _ as *mut u8),
            Some(&mut size),
        ).is_err() || enabled == 0
        {
            let _ = RegCloseKey(hkey);
            return String::new();
        }

        // Read ProxyServer value.
        let mut buf = vec![0u16; 256];
        let mut size = (buf.len() * 2) as u32;
        if RegQueryValueExW(
            hkey,
            VALUE,
            None,
            None,
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut size),
        ).is_err()
        {
            let _ = RegCloseKey(hkey);
            return String::new();
        }
        let _ = RegCloseKey(hkey);

        let len = (size as usize / 2).saturating_sub(1); // strip null terminator
        let raw = String::from_utf16_lossy(&buf[..len]);
        // ProxyServer can be "host:port" or "http=host:port;https=host:port".
        // Extract the first host:port-looking part.
        raw.split(';')
            .next()
            .unwrap_or(&raw)
            .split('=')
            .last()
            .unwrap_or(&raw)
            .trim()
            .to_string()
    }
}

/// Build a TLS stream to `ws.warframe.market:443`. Tries multiple strategies:
/// 1. Windows registry proxy (HTTP CONNECT)
/// 2. Direct TCP + TLS
/// 3. Common local proxy ports (7890, 1080, 10808)
async fn ws_tls_connect(
    app: &tauri::AppHandle,
) -> Result<tokio_native_tls::TlsStream<tokio::net::TcpStream>, String> {
    // Strategy 1: registry proxy.
    let registry_proxy = read_windows_proxy();
    if !registry_proxy.is_empty() {
        ws_log(app, &format!("ws_tls: try registry proxy {}", registry_proxy));
        match try_proxy_tls(app, &registry_proxy).await {
            Ok(s) => return Ok(s),
            Err(e) => ws_log(app, &format!("ws_tls: registry proxy failed: {}", e)),
        }
    } else {
        ws_log(app, "ws_tls: no registry proxy");
    }

    // Strategy 2: direct TCP + TLS.
    ws_log(app, "ws_tls: try direct TLS...");
    match try_direct_tls(app).await {
        Ok(s) => return Ok(s),
        Err(e) => ws_log(app, &format!("ws_tls: direct TLS failed: {}", e)),
    }

    // Strategy 3: common proxy ports.
    for port in &["7890", "1080", "10808"] {
        let proxy = format!("127.0.0.1:{}", port);
        if proxy == registry_proxy { continue; } // already tried
        ws_log(app, &format!("ws_tls: try fallback proxy {}", proxy));
        match try_proxy_tls(app, &proxy).await {
            Ok(s) => return Ok(s),
            Err(e) => ws_log(app, &format!("ws_tls: fallback {} failed: {}", proxy, e)),
        }
    }

    Err("all connection strategies exhausted".to_string())
}

/// Known-good Cloudflare IPs for ws.warframe.market (DNS may be hijacked).
const WS_REAL_IPS: &[&str] = &[
    "104.26.0.182:443",
    "104.26.1.182:443",
    "172.67.75.162:443",
];

/// Direct TCP + TLS to ws.warframe.market:443.
/// Tries DNS first, then falls back to known Cloudflare IPs (DNS hijacking bypass).
async fn try_direct_tls(
    app: &tauri::AppHandle,
) -> Result<tokio_native_tls::TlsStream<tokio::net::TcpStream>, String> {
    // Try DNS-based connection first.
    let tcp = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::net::TcpStream::connect("ws.warframe.market:443").await
    }).await;
    match tcp {
        Ok(Ok(tcp)) => {
            ws_log(app, "ws_tls: DNS TCP ok, TLS handshake...");
            let tls = tokio_native_tls::native_tls::TlsConnector::builder().build()
                .map_err(|e| format!("TlsConnector: {}", e))?;
            let tls = tokio_native_tls::TlsConnector::from(tls);
            return tls.connect("ws.warframe.market", tcp).await
                .map_err(|e| format!("TLS handshake failed: {}", e));
        }
        Ok(Err(ref e)) => ws_log(app, &format!("ws_tls: DNS TCP failed: {}", e)),
        Err(_) => ws_log(app, "ws_tls: DNS TCP timed out"),
    }

    // DNS hijacking fallback: try known Cloudflare IPs with correct SNI.
    for &ip in WS_REAL_IPS {
        ws_log(app, &format!("ws_tls: try real IP {}...", ip));
        let tcp = match tokio::time::timeout(Duration::from_secs(8), async {
            tokio::net::TcpStream::connect(ip).await
        }).await {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => { ws_log(app, &format!("ws_tls: {} failed: {}", ip, e)); continue; }
            Err(_) => { ws_log(app, &format!("ws_tls: {} timed out", ip)); continue; }
        };
        ws_log(app, &format!("ws_tls: {} TCP ok, TLS...", ip));
        let tls = tokio_native_tls::native_tls::TlsConnector::builder().build()
            .map_err(|e| format!("TlsConnector: {}", e))?;
        let tls = tokio_native_tls::TlsConnector::from(tls);
        match tokio::time::timeout(Duration::from_secs(10), async {
            tls.connect("ws.warframe.market", tcp).await
        }).await {
            Ok(Ok(s)) => {
                ws_log(app, "ws_tls: real IP TLS ok");
                return Ok(s);
            }
            Ok(Err(e)) => ws_log(app, &format!("ws_tls: {} TLS failed: {}", ip, e)),
            Err(_) => ws_log(app, &format!("ws_tls: {} TLS timed out", ip)),
        }
    }

    Err("all direct strategies exhausted".to_string())
}

/// HTTP CONNECT tunnel through `proxy` → TLS to ws.warframe.market:443.
async fn try_proxy_tls(
    app: &tauri::AppHandle,
    proxy: &str,
) -> Result<tokio_native_tls::TlsStream<tokio::net::TcpStream>, String> {
    use tokio::io::{AsyncWriteExt, AsyncReadExt};

    let tcp = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::net::TcpStream::connect(proxy).await
    }).await
        .map_err(|_| format!("proxy TCP {} timed out", proxy))?
        .map_err(|e| format!("proxy TCP {}: {}", proxy, e))?;
    ws_log(app, &format!("ws_tls: proxy {} TCP ok, CONNECT...", proxy));

    let (mut rd, mut wr) = tcp.into_split();
    let connect_req = format!(
        "CONNECT ws.warframe.market:443 HTTP/1.1\r\n\
         Host: ws.warframe.market:443\r\n\r\n"
    );
    wr.write_all(connect_req.as_bytes()).await
        .map_err(|e| format!("proxy write: {}", e))?;

    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(5), rd.read(&mut buf)).await
        .map_err(|_| "proxy read timeout (5s)".to_string())?
        .map_err(|e| format!("proxy read: {}", e))?;

    let response = std::str::from_utf8(&buf[..n]).unwrap_or("");
    ws_log(app, &format!("ws_tls: proxy {} response: {}", proxy, &response[..response.len().min(80)]));
    if !response.contains("200") {
        return Err(format!("proxy {} rejected: {}", proxy, &response[..response.len().min(80)]));
    }

    let tcp = rd.reunite(wr).map_err(|_| "proxy reunite failed".to_string())?;

    let tls = tokio_native_tls::native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TlsConnector: {}", e))?;
    let tls = tokio_native_tls::TlsConnector::from(tls);
    let stream = tokio::time::timeout(Duration::from_secs(8), async {
        tls.connect("ws.warframe.market", tcp).await
    }).await
        .map_err(|_| "TLS through proxy timed out (8s)".to_string())?
        .map_err(|e| format!("TLS through proxy: {}", e))?;
    ws_log(app, &format!("ws_tls: proxy {} TLS ok", proxy));
    Ok(stream)
}

/// Emit a log line to both stderr (for `cargo run`) and the frontend
/// `market-ws-log` event (for release builds without a console).
fn ws_log(app: &tauri::AppHandle, msg: &str) {
    debug!("market_ws: {}", msg);
    let _ = app.emit("market-ws-log", msg.to_string());
}

/// Spawn a persistent WebSocket task for bidirectional status sync.
///
/// Called after successful login. The task runs until it receives `Shutdown`
/// (on logout) or the JWT disappears. On disconnect it reconnects with
/// exponential backoff.
async fn spawn_ws(auth: &SharedMarketAuth, app_handle: &tauri::AppHandle) {
    ws_log(app_handle, "spawn_ws: creating channel");
    let (tx, rx) = tokio::sync::mpsc::channel::<WsCommand>(8);

    // Store sender so commands can reach the task.
    {
        let mut a = auth.inner.write().await;
        // Shut down any previous task (double-login without logout).
        if let Some(old) = a.ws_tx.take() {
            let _ = old.try_send(WsCommand::Shutdown);
        }
        a.ws_tx = Some(tx);
    }
    ws_log(app_handle, "spawn_ws: tx stored, spawning task");

    let auth2 = Arc::clone(auth);
    let handle = app_handle.clone();
    let handle2 = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        ws_log(&handle, "run_ws_loop: task started");
        run_ws_loop(auth2, handle, rx).await;
        ws_log(&handle2, "run_ws_loop: task exited");
    });
    ws_log(app_handle, "spawn_ws: done");
}

/// Core WebSocket loop: connect, send/receive, reconnect on failure.
async fn run_ws_loop(
    auth: SharedMarketAuth,
    app_handle: tauri::AppHandle,
    mut cmd_rx: tokio::sync::mpsc::Receiver<WsCommand>,
) {
    let mut backoff: u64 = 1;

    loop {
        // Read JWT — exit if logged out.
        let jwt = {
            let a = auth.inner.read().await;
            a.access_token.clone()
        };
        let Some(jwt) = jwt else {
            ws_log(&app_handle, "JWT gone, exiting");
            break;
        };

        ws_log(&app_handle, "building TLS stream...");
        // 40s total: direct(15s) + proxy(5s) + proxy(5s) + proxy(5s) + slack
        let tls_stream = match tokio::time::timeout(
            Duration::from_secs(40),
            ws_tls_connect(&app_handle),
        ).await {
            Ok(Ok(s)) => {
                ws_log(&app_handle, "TLS stream ready");
                s
            }
            Ok(Err(e)) => {
                ws_log(&app_handle, &format!("TLS connect failed: {}", e));
                break; // will reconnect
            }
            Err(_) => {
                ws_log(&app_handle, "TLS connect timed out (35s)");
                break;
            }
        };

        // Build request via ClientRequestBuilder — auto-generates all WS headers.
        let uri: tokio_tungstenite::tungstenite::http::Uri = WS_URL.parse().unwrap();
        let request = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri)
            .with_header("Sec-WebSocket-Protocol", "wfm");

        ws_log(&app_handle, "calling client_async...");
        match tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::client_async(request, tls_stream),
        ).await {
            Ok(Ok((mut ws, resp))) => {
                ws_log(&app_handle, &format!("ws connected — status={}", resp.status()));
                let _ = app_handle.emit("market-ws-state", "connected");
                for (name, val) in resp.headers() {
                    ws_log(&app_handle, &format!("  resp header {}: {:?}", name, val));
                }
                backoff = 1; // reset on successful connect

                // Send auth message (wf-market protocol: auth is a WS message, not a cookie).
                let auth_msg = serde_json::json!({
                    "route": "@wfm|cmd/auth/signIn",
                    "payload": {"token": jwt},
                    "id": "auth-1"
                });
                ws_log(&app_handle, &format!("sending auth: {}", auth_msg));
                if let Err(e) = ws.send(
                    tokio_tungstenite::tungstenite::Message::Text(auth_msg.to_string().into())
                ).await {
                    ws_log(&app_handle, &format!("auth send error: {}", e));
                    break; // reconnect
                }
                // Read auth response (or first message).
                match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
                    Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                        ws_log(&app_handle, &format!("auth response: {}", text));
                        handle_ws_message(&text, &auth, &app_handle).await;
                    }
                    other => {
                        ws_log(&app_handle, &format!("auth response unexpected: {:?}", other
                            .map(|o| o.map(|m| format!("{:?}", m)))));
                    }
                }

                // Replay last known status so server state matches app state.
                {
                    let status = auth.inner.read().await.current_status.clone()
                        .unwrap_or_else(|| "online".to_string());
                    let msg = serde_json::json!({
                        "route": "@wfm|cmd/status/set",
                        "payload": {"status": &status},
                        "id": "replay-1"
                    });
                    ws_log(&app_handle, &format!("replay status: {}", status));
                    if let Err(e) = ws.send(
                        tokio_tungstenite::tungstenite::Message::Text(msg.to_string().into())
                    ).await {
                        ws_log(&app_handle, &format!("replay send error: {}", e));
                    }
                }

                // Main select loop: read WS messages + listen for commands.
                let mut initial_sync_done = false;
                loop {
                    tokio::select! {
                        // ── Incoming WebSocket message ──
                        msg = ws.next() => {
                            // Log every frame so we can discover the protocol.
                            match &msg {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    ws_log(&app_handle, &format!("recv Text ({}b): {}", text.len(), text));
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(data))) => {
                                    ws_log(&app_handle, &format!("recv Binary ({}b): {:?}", data.len(), &data[..data.len().min(64)]));
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data))) => {
                                    ws_log(&app_handle, &format!("recv Ping ({}b)", data.len()));
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(data))) => {
                                    ws_log(&app_handle, &format!("recv Pong ({}b)", data.len()));
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
                                    ws_log(&app_handle, &format!("recv Close frame={:?}", frame));
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Frame(_))) => {
                                    ws_log(&app_handle, "recv raw Frame");
                                }
                                Some(Err(e)) => {
                                    ws_log(&app_handle, &format!("recv error: {}", e));
                                }
                                None => {
                                    ws_log(&app_handle, "recv None (stream ended)");
                                }
                            }

                            match msg {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    handle_ws_message(&text, &auth, &app_handle).await;
                                    // First status event after (re)connect → frontend can unlock radios.
                                    if !initial_sync_done {
                                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                            if v["route"].as_str() == Some("@wfm|event/status/set") {
                                                initial_sync_done = true;
                                                let _ = app_handle.emit("market-ws-state", "ready");
                                                ws_log(&app_handle, "initial status sync done → ready");
                                            }
                                        }
                                    }
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data))) => {
                                    let _ = ws.send(tokio_tungstenite::tungstenite::Message::Pong(data)).await;
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => {
                                    ws_log(&app_handle, "server closed connection");
                                    break; // reconnect
                                }
                                Some(Ok(_)) => {
                                    // Binary/Pong/Frame — logged above, no action.
                                }
                                Some(Err(_)) | None => {
                                    break; // reconnect
                                }
                            }
                        }
                        // ── Command from Tauri command handler ──
                        cmd = cmd_rx.recv() => {
                            match cmd {
                                Some(WsCommand::SetStatus(status)) => {
                                    let payload = match status.as_str() {
                                        "online" | "ingame" | "invisible" => status.as_str(),
                                        _ => "online",
                                    };
                                    let msg = serde_json::json!({
                                        "route": "@wfm|cmd/status/set",
                                        "payload": {"status": payload},
                                        "id": "status-1"
                                    });
                                    ws_log(&app_handle, &format!("sending: {}", msg));
                                    if let Err(e) = ws.send(
                                        tokio_tungstenite::tungstenite::Message::Text(msg.to_string().into())
                                    ).await {
                                        ws_log(&app_handle, &format!("send error: {}", e));
                                        break; // reconnect
                                    }
                                    auth.inner.write().await.current_status = Some(payload.to_string());
                                }
                                Some(WsCommand::Shutdown) => {
                                    ws_log(&app_handle, "shutdown requested");
                                    let _ = ws.close(None).await;
                                    auth.inner.write().await.ws_tx = None;
                                    return;
                                }
                                None => {
                                    ws_log(&app_handle, "command channel closed");
                                    let _ = ws.close(None).await;
                                    auth.inner.write().await.ws_tx = None;
                                    return;
                                }
                            }
                        }
                        // ── Heartbeat (55s) to prevent firewall NAT timeout ──
                        _ = tokio::time::sleep(Duration::from_secs(55)) => {
                            if let Err(e) = ws.send(
                                tokio_tungstenite::tungstenite::Message::Ping(vec![])
                            ).await {
                                ws_log(&app_handle, &format!("ping error: {}", e));
                                break;
                            }
                        }
                    }
                }
                // Inner loop exited — reconnect after backoff.
            }
            Ok(Err(e)) => {
                ws_log(&app_handle, &format!("ws handshake failed: {}", e));
            }
            Err(_elapsed) => {
                ws_log(&app_handle, "ws handshake timed out (10s)");
            }
        }

        // Backoff before reconnect — but listen for Shutdown so logout isn't blocked.
        ws_log(&app_handle, &format!("reconnecting in {}s ...", backoff));
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
            cmd = cmd_rx.recv() => {
                if matches!(cmd, Some(WsCommand::Shutdown)) {
                    ws_log(&app_handle, "shutdown during backoff");
                    auth.inner.write().await.ws_tx = None;
                    return;
                }
            }
        }
        backoff = (backoff * 2).min(30);
    }

    // Outer loop exited — clean up.
    let _ = app_handle.emit("market-ws-state", "disconnected");
    auth.inner.write().await.ws_tx = None;
}

/// Process one text message from the WebSocket (wf-market protocol, @wfm| routes).
async fn handle_ws_message(text: &str, auth: &SharedMarketAuth, app_handle: &tauri::AppHandle) {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            ws_log(app_handle, &format!("unparseable JSON: {}", &text[..text.len().min(120)]));
            return;
        }
    };

    // wf-market protocol uses "route" instead of "type".
    let route = v["route"].as_str().unwrap_or("");
    match route {
        // Auth signin response.
        "@wfm|event/auth/signIn" => {
            ws_log(app_handle, &format!("auth signIn response: payload={:?}", v["payload"]));
        }
        // Status change event (pushed by server or echo of our own set).
        "@wfm|event/status/set" => {
            if let Some(status) = v["payload"]["status"].as_str() {
                ws_log(app_handle, &format!("status event: {}", status));
                auth.inner.write().await.current_status = Some(status.to_string());
                let _ = app_handle.emit("market-status-change", status);
            }
        }
        // Online count.
        "@wfm|event/online/count" => {
            let total = v["payload"]["total"].as_u64().unwrap_or(0);
            ws_log(app_handle, &format!("online count: {}", total));
        }
        // Legacy format (old @WS/USER, @WS/MESSAGE/ONLINE_COUNT) — fallback.
        _ => {
            // Try old "type" field as fallback.
            let msg_type = v["type"].as_str().unwrap_or("");
            match msg_type {
                "@WS/USER" => {
                    if let Some(status) = v["payload"].as_str() {
                        ws_log(app_handle, &format!("legacy @WS/USER status={}", status));
                        auth.inner.write().await.current_status = Some(status.to_string());
                        let _ = app_handle.emit("market-status-change", status);
                    }
                }
                "@WS/MESSAGE/ONLINE_COUNT" => {
                    let total = v["payload"]["total_users"].as_u64().unwrap_or(0);
                    ws_log(app_handle, &format!("legacy online_count: {}", total));
                }
                "" if route.is_empty() => {
                    ws_log(app_handle, &format!("message with no route/type: {}", &text[..text.len().min(150)]));
                }
                _ => {
                    ws_log(app_handle, &format!("unknown route={} type={} payload={:?}", route, msg_type, v["payload"]));
                }
            }
        }
    }
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

    // Prevent concurrent login attempts.
    {
        let mut inner = auth.inner.write().await;
        if inner.login_in_progress {
            return Err(MarketError {
                code: "in_progress".into(),
                message: "登录处理中，请稍后重试".into(),
            });
        }
        inner.login_in_progress = true;
    }

    let result = do_signin(&auth, &app_handle, &email, &password).await;

    // Always reset the guard.
    auth.inner.write().await.login_in_progress = false;

    result
}

// ── CSRF helper ──────────────────────────────────────────────────────────

struct CsrfContext {
    csrf_token: String,
    session_cookie: String,
}

async fn fetch_csrf() -> Result<CsrfContext, MarketError> {
    let url = format!("{}/auth/signin", MARKET_WEB);
    let resp = client().get(&url).send().await.map_err(crate::market_orders::network_err)?;
    let status = resp.status().as_u16();
    let cookies: Vec<String> = resp.headers().get_all("set-cookie").iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string())).collect();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(MarketError {
            code: "server_error".into(),
            message: format!("CSRF 页面返回 HTTP {}", status),
        });
    }
    let mut parts = Vec::new();
    for v in &cookies {
        if let Some(p) = v.split(';').next() { parts.push(p.trim().to_string()); }
    }
    let csrf = extract_csrf_from_html(&body).ok_or_else(|| MarketError {
        code: "server_error".into(), message: "无法获取 CSRF token".into(),
    })?;
    Ok(CsrfContext { csrf_token: csrf, session_cookie: parts.join("; ") })
}

fn extract_csrf_from_html(html: &str) -> Option<String> {
    let pos = html.find(r#"name="csrf-token""#)?;
    let after = &html[pos..];
    let cp = after.find("content=")?;
    let ac = &after[cp + 8..];
    let q = ac.chars().next()?;
    let vs = if q == '"' || q == '\'' { 1 } else { 0 };
    let ve = if vs == 1 { ac[vs..].find(q)? } else { ac.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(ac.len()) };
    Some(ac[vs..vs + ve].to_string())
}

async fn do_signin(
    auth: &SharedMarketAuth,
    app_handle: &tauri::AppHandle,
    email: &str,
    password: &str,
) -> Result<MarketAuthStatus, MarketError> {
    let _ = app_handle.emit("market-login-phase", "正在获取安全令牌…");
    let ctx = fetch_csrf().await?;
    let device_id = { auth.inner.read().await.device_id.clone() };

    let _ = app_handle.emit("market-login-phase", "正在登录…");
    let resp = client()
        .post(format!("{}/auth/signin", MARKET_API_V1))
        .header("Cookie", &ctx.session_cookie)
        .header("x-csrf-token", &ctx.csrf_token)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "email": email.trim(), "password": password,
            "device_id": &device_id, "auth_type": "cookie",
        }))
        .send().await.map_err(crate::market_orders::network_err)?;

    let status = resp.status().as_u16();
    let set_cookie_headers: Vec<String> = resp.headers().get_all("set-cookie").iter()
        .filter_map(|v| v.to_str().ok()).map(|s| s.to_string()).collect();
    let body_text = resp.text().await.unwrap_or_default();

    match status {
        200 | 302 => {
            let access_token = extract_jwt_from_cookies(&set_cookie_headers)
                .or_else(|| serde_json::from_str::<serde_json::Value>(&body_text).ok()
                    .and_then(|v| v["access_token"].as_str().or(v["payload"]["access_token"].as_str())
                        .or(v["token"].as_str()).or(v["payload"]["token"].as_str()).map(String::from)));
            let ingame_name = access_token.as_deref().and_then(jwt_sub)
                .or_else(|| serde_json::from_str::<serde_json::Value>(&body_text).ok()
                    .and_then(|v| v["payload"]["user"]["ingame_name"].as_str().map(String::from)));
            let avatar = serde_json::from_str::<serde_json::Value>(&body_text).ok()
                .and_then(|v| v["payload"]["user"]["avatar"].as_str().map(String::from));
            let reputation = serde_json::from_str::<serde_json::Value>(&body_text).ok()
                .and_then(|v| v["payload"]["user"]["reputation"].as_i64().map(|r| r as i32));
            match (access_token, ingame_name) {
                (Some(at), Some(name)) => {
                    let _ = app_handle.emit("market-login-phase", "登录成功");
                    let app_data_dir = app_handle.path().app_data_dir().map_err(|_| MarketError {
                        code: "unknown".into(), message: "无法获取应用数据目录".into(),
                    })?;
                    let mut a = auth.inner.write().await;
                    a.access_token = Some(at); a.ingame_name = Some(name.clone());
                    a.avatar = avatar; a.reputation = reputation;
                    a.logged_in = true; persist(&a, &app_data_dir); drop(a);
                    spawn_ws(auth, app_handle).await;
                    let a = auth.inner.read().await;
                    Ok(MarketAuthStatus { logged_in: true, ingame_name: Some(name),
                        avatar: a.avatar.clone(), reputation: a.reputation,
                        current_status: a.current_status.clone() })
                }
                _ => Err(MarketError { code: "server_error".into(),
                    message: "服务器响应异常".into() }),
            }
        }
        401 => {
            let bl = body_text.to_lowercase();
            let code = if bl.contains("email") || bl.contains("user") { "email_not_found" } else { "wrong_password" };
            Err(MarketError { code: code.into(),
                message: if code == "email_not_found" { "该邮箱未注册".into() } else { "密码错误".into() } })
        }
        429 => Err(MarketError { code: "rate_limited".into(), message: "请求过于频繁".into() }),
        c if c >= 500 => Err(MarketError { code: "server_error".into(), message: "Warframe.Market 服务暂时不可用".into() }),
        _ => {
            // Try to parse the Warframe.Market validation error format:
            // {"error": {"password": ["app.account.password_invalid"]}}
            let hint = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body_text) {
                if let Some(err_obj) = v["error"].as_object() {
                    // Collect all error codes from all fields.
                    let codes: Vec<&str> = err_obj.values()
                        .filter_map(|a| a.as_array())
                        .flatten()
                        .filter_map(|s| s.as_str())
                        .collect();
                    let friendly = codes.iter().map(|c| match *c {
                        "app.account.password_invalid" => "密码错误",
                        "app.account.email_invalid" => "邮箱格式不正确",
                        "app.account.email_not_found" => "该邮箱未注册",
                        other => other,
                    }).collect::<Vec<_>>().join("；");
                    if !friendly.is_empty() { friendly }
                    else { v.to_string() }
                } else {
                    v["error"].as_str().or(v["message"].as_str())
                        .unwrap_or(&body_text)
                        .to_string()
                }
            } else {
                body_text.chars().take(120).collect()
            };
            Err(MarketError { code: "unknown".into(), message: format!("登录失败：{hint}") })
        }
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

    // Shut down the persistent WebSocket before clearing auth state.
    let ws_tx = {
        let a = auth.inner.read().await;
        a.ws_tx.clone()
    };
    if let Some(ref tx) = ws_tx {
        let _ = tx.send(WsCommand::Shutdown).await;
    }

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
    app_handle: tauri::AppHandle,
) -> Result<MarketAuthStatus, MarketError> {
    let need_spawn = {
        let a = auth.inner.read().await;
        ws_log(&app_handle, &format!("market_auth_status: logged_in={} ws_tx={}", a.logged_in, a.ws_tx.is_some()));
        a.logged_in && a.ws_tx.is_none()
    };
    if need_spawn {
        ws_log(&app_handle, "market_auth_status: spawning WS");
        spawn_ws(&auth, &app_handle).await;
    } else {
        ws_log(&app_handle, "market_auth_status: WS already running or not logged in");
    }
    let a = auth.inner.read().await;
    Ok(MarketAuthStatus {
        logged_in: a.logged_in,
        ingame_name: a.ingame_name.clone(),
        avatar: a.avatar.clone(),
        reputation: a.reputation,
        current_status: a.current_status.clone(),
    })
}

/// Set the user's online status via the persistent WebSocket.
///
/// The status change is sent through the mpsc channel to the background
/// WebSocket task, which forwards it to `wss://ws.warframe.market/socket`.
#[tauri::command]
pub async fn market_set_status(
    auth: tauri::State<'_, SharedMarketAuth>,
    app_handle: tauri::AppHandle,
    status: String,
) -> Result<(), MarketError> {
    let tx = {
        let a = auth.inner.read().await;
        a.ws_tx.clone()
    };

    let tx = match tx {
        Some(t) => t,
        None => {
            // No WebSocket task — user may have stale login, try to recover.
            let app_data_dir = app_handle.path().app_data_dir().map_err(|_| MarketError {
                code: "unknown".into(),
                message: "无法获取应用数据目录".into(),
            })?;
            // Ensure we still have a valid token.
            let _token = ensure_valid_token(&auth, &app_data_dir, &app_handle).await?;
            // Spawn a new WS task.
            spawn_ws(&auth, &app_handle).await;
            // Retry read.
            let a2 = auth.inner.read().await;
            a2.ws_tx.clone().ok_or_else(|| MarketError {
                code: "ws_disconnected".into(),
                message: "WebSocket 未连接，请重新登录".into(),
            })?
        }
    };

    let payload = match status.as_str() {
        "online" | "ingame" | "invisible" => status.to_string(),
        _ => "online".to_string(),
    };

    tx.send(WsCommand::SetStatus(payload)).await.map_err(|_| MarketError {
        code: "ws_disconnected".into(),
        message: "WebSocket 已断开，请重新登录".into(),
    })?;

    Ok(())
}
