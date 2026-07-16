use reqwest::Client;
use std::sync::OnceLock;
use std::time::Duration;

fn push_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("push client")
    })
}

/// POST a notification to the user's Bark endpoint. `bark_url` should be the
/// full base URL copied from the Bark app (e.g. `https://api.day.app/xxx`).
/// Title and body are appended as path segments per the Bark v2 API.
pub async fn push(bark_url: &str, title: &str, body: &str) {
    let url = format!(
        "{}/{}/{}",
        bark_url.trim_end_matches('/'),
        pct_encode(title),
        pct_encode(body),
    );
    let _ = push_client().get(&url).send().await;
}

/// Percent-encode per RFC 3986: everything except unreserved chars
/// (A–Z a–z 0–9 - _ . ~) gets %XX'd.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}
