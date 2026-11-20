//! Subscription limits read straight from the provider, a port of the macOS
//! `NativeUsageProbe` (Claude half) + `JcodeClaudeAuth`.
//!
//! Token sources, best first: Claude Code's `~/.claude/.credentials.json`, then
//! jcode's active Claude account in `~/.jcode/auth.json` (reached over
//! `\\wsl.localhost` when jcode runs in WSL). Read-only: only the short-lived
//! access token is used, never the refresh token, nothing is written back, and
//! the token stays in Rust (the webview only sees percentages and reset times).
//!
//! Codex (`~/.codex/auth.json` or `~/.config/codex/auth.json`) is probed the same way
//! against its fixed provider endpoint; each probe fails closed and silently.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LimitWindow {
    /// "Session" / "Weekly" (translated in the webview).
    pub label: String,
    /// 0..1 of the budget still left.
    pub fraction_left: f64,
    /// The provider's reset timestamp, passed through for `Date.parse`.
    pub resets_at: Option<String>,
    /// Window length in seconds (5h / 7d), for the "5h" / "7d" short label.
    pub period: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LimitProvider {
    pub id: String,
    pub display_name: String,
    pub windows: Vec<LimitWindow>,
}

/// True only when `expiry` is a number (epoch seconds or milliseconds) in the
/// past. A missing/unknown expiry is not treated as expired: the API call is
/// the real check (mirrors `OAuthExpiry.isExpired`).
pub fn is_expired(expiry: Option<&Value>, now_secs: f64) -> bool {
    let Some(raw) = expiry.and_then(Value::as_f64) else { return false };
    // Heuristic: epoch-ms values are > 1e11 (1973+ in ms, year 5138 in s).
    let secs = if raw > 1e11 { raw / 1000.0 } else { raw };
    secs <= now_secs
}

/// jcode `auth.json`: `{"anthropic_accounts": [{"label", "access", "expires"
/// (epoch ms), ...}], "active_anthropic_account": "<label>"}`. The active
/// account's token, else the first account's; None if missing/expired.
pub fn jcode_access_token(data: &[u8], now_secs: f64) -> Option<String> {
    let json: Value = serde_json::from_slice(data).ok()?;
    let accounts = json.get("anthropic_accounts")?.as_array()?;
    let first = accounts.first()?;
    let active = json.get("active_anthropic_account").and_then(Value::as_str);
    let account = accounts
        .iter()
        .find(|a| active.is_some() && a.get("label").and_then(Value::as_str) == active)
        .unwrap_or(first);
    let token = account.get("access")?.as_str()?;
    if token.is_empty() || is_expired(account.get("expires"), now_secs) {
        return None;
    }
    Some(token.to_string())
}

/// Claude Code `.credentials.json`: `{"claudeAiOauth": {"accessToken",
/// "expiresAt"}}`.
pub fn claude_code_access_token(data: &[u8], now_secs: f64) -> Option<String> {
    let json: Value = serde_json::from_slice(data).ok()?;
    let oauth = json.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?;
    if token.is_empty() || is_expired(oauth.get("expiresAt"), now_secs) {
        return None;
    }
    Some(token.to_string())
}

/// Parses an `/api/oauth/usage` body. Windows are `{"utilization": <percent
/// used>, "resets_at": ...}`.
pub fn claude_provider(json: &Value) -> Option<LimitProvider> {
    let mut windows = Vec::new();
    for (key, label, period) in [("five_hour", "Session", 18_000.0), ("seven_day", "Weekly", 604_800.0)] {
        let Some(w) = json.get(key) else { continue };
        let Some(used) = w.get("utilization").and_then(Value::as_f64) else { continue };
        let resets_at = match w.get("resets_at") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        windows.push(LimitWindow {
            label: label.into(),
            fraction_left: ((100.0 - used) / 100.0).clamp(0.0, 1.0),
            resets_at,
            period,
        });
    }
    if windows.is_empty() {
        return None;
    }
    Some(LimitProvider { id: "claude".into(), display_name: "Claude".into(), windows })
}

/// Unexpired candidate tokens from the given files, deduped, in order. Each
/// file is tried with both shapes, so the caller just lists paths.
pub fn candidate_tokens(paths: &[PathBuf], now_secs: f64) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        let Ok(data) = std::fs::read(p) else { continue };
        let token = claude_code_access_token(&data, now_secs).or_else(|| jcode_access_token(&data, now_secs));
        if let Some(t) = token {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// Claude Code (native home) and jcode (`hooks::jcode_home`: the WSL home only
/// once jcode was found there, see the note in hooks.rs; never spawns wsl.exe).
pub fn default_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude").join(".credentials.json"));
    }
    if let Some((home, _)) = crate::hooks::jcode_home() {
        paths.push(home.join(".claude").join(".credentials.json"));
        paths.push(home.join(".jcode").join("auth.json"));
    }
    paths
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Tries every unexpired token in order (each source is refreshed only by its
/// own CLI, so one can be stale while another is live). None when no token
/// works or the network is down; the UI then just shows nothing.
pub async fn probe_claude(paths: Vec<PathBuf>) -> Option<LimitProvider> {
    // Blocking file reads (a `\\wsl.localhost` UNC read can stall) must not run
    // on the async executor: move them to the blocking pool.
    let tokens = tauri::async_runtime::spawn_blocking(move || candidate_tokens(&paths, now_secs()))
        .await
        .ok()?;
    if tokens.is_empty() {
        return None;
    }
    // The updater installs the same provider lazily; whoever runs first wins.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder().timeout(Duration::from_secs(8)).build().ok()?;
    for token in tokens {
        if let Some(p) = probe_with(&client, &token).await {
            return Some(p);
        }
    }
    None
}

async fn probe_with(client: &reqwest::Client, token: &str) -> Option<LimitProvider> {
    for attempt in 0..2 {
        let res = client
            .get(USAGE_URL)
            .bearer_auth(token)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("User-Agent", "claude-code/2.1.69")
            .send()
            .await;
        let retry = match res {
            Ok(r) if r.status().is_success() => {
                let body = r.bytes().await.ok()?;
                let json: Value = serde_json::from_slice(&body).ok()?;
                return claude_provider(&json);
            }
            Ok(r) => r.status().is_server_error(),
            Err(_) => true,
        };
        if !retry || attempt == 1 {
            return None;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    None
}

// ----- Codex (`~/.codex/auth.json` -> chatgpt.com wham/usage) ---------------

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const MAX_CRED_BYTES: u64 = 1024 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
/// 2100-01-01: a reset further out than this is a bogus value, not a date.
const MAX_RESET_SECS: f64 = 4_102_444_800.0;

/// Never Debug/Serialize: the token and account id must not be printable.
pub struct CodexCreds {
    token: String,
    account_id: Option<String>,
}

/// Codex `auth.json`: `{"tokens": {"access_token", "account_id"?}}`.
pub fn parse_codex_auth(text: &str) -> Option<CodexCreds> {
    let v: Value = serde_json::from_str(text).ok()?;
    let tokens = v.get("tokens")?;
    let token = tokens.get("access_token")?.as_str()?;
    if token.is_empty() {
        return None;
    }
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(CodexCreds { token: token.to_string(), account_id })
}

fn epoch_to_secs(n: f64) -> f64 {
    if n > 1e11 {
        n / 1000.0
    } else {
        n
    }
}

fn codex_window(
    headers: &[(&str, &str)],
    header: &str,
    body: Option<&Value>,
    label: &str,
    period: f64,
    now_secs: f64,
) -> Option<LimitWindow> {
    let from_header = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(header))
        .and_then(|(_, v)| v.trim().parse::<f64>().ok());
    let used = from_header
        .or_else(|| body?.get("used_percent")?.as_f64())
        .filter(|u| u.is_finite())?;
    let relative = body.and_then(|w| {
        ["reset_after_seconds", "resets_in_seconds"]
            .iter()
            .find_map(|k| w.get(*k)?.as_f64())
    });
    let reset = body
        .and_then(|w| w.get("reset_at")?.as_f64())
        .map(epoch_to_secs)
        .or_else(|| relative.filter(|s| s.is_finite() && *s >= 0.0).map(|s| now_secs + s))
        .filter(|s| s.is_finite())
        .map(|s| s.min(MAX_RESET_SECS));
    Some(LimitWindow {
        label: label.to_string(),
        fraction_left: ((100.0 - used) / 100.0).clamp(0.0, 1.0),
        resets_at: reset.map(|s| (s as i64).to_string()),
        period,
    })
}

/// Parses a `wham/usage` answer: the `x-codex-*-used-percent` headers win over
/// the JSON body, which also supplies the reset times.
pub fn parse_codex_usage(headers: &[(&str, &str)], body: &[u8], now_secs: f64) -> Option<LimitProvider> {
    let v: Option<Value> = serde_json::from_slice(body).ok();
    let rate = v.as_ref().and_then(|v| v.get("rate_limit"));
    let windows: Vec<LimitWindow> = [
        ("x-codex-primary-used-percent", "primary_window", "Session", 18_000.0),
        ("x-codex-secondary-used-percent", "secondary_window", "Weekly", 604_800.0),
    ]
    .iter()
    .filter_map(|(h, key, label, period)| {
        codex_window(headers, h, rate.and_then(|r| r.get(*key)), label, *period, now_secs)
    })
    .collect();
    if windows.is_empty() {
        return None;
    }
    Some(LimitProvider { id: "codex".into(), display_name: "Codex".into(), windows })
}

fn read_capped(path: PathBuf) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_CRED_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_CRED_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

fn read_codex_creds() -> Option<CodexCreds> {
    let home = dirs::home_dir()?;
    let text = read_capped(home.join(".codex").join("auth.json"))
        .or_else(|| read_capped(home.join(".config").join("codex").join("auth.json")))?;
    parse_codex_auth(&text)
}

type Fetched = (Vec<(String, String)>, Vec<u8>);

/// Outer None: retryable failure. Some(None): final failure. Some(Some(r)): response.
async fn fetch_codex_once(req: reqwest::RequestBuilder) -> Option<Option<Fetched>> {
    let mut resp = req.send().await.ok()?;
    let status = resp.status();
    if status.is_server_error() {
        return None;
    }
    if !status.is_success() {
        return Some(None);
    }
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter(|(k, _)| k.as_str().starts_with("x-codex-"))
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(c)) => {
                buf.extend_from_slice(&c);
                if buf.len() > MAX_BODY_BYTES {
                    buf.truncate(MAX_BODY_BYTES);
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => return None,
        }
    }
    Some(Some((headers, buf)))
}

fn codex_request(client: &reqwest::Client, creds: &CodexCreds) -> reqwest::RequestBuilder {
    let mut r = client
        .get(CODEX_USAGE_URL)
        .bearer_auth(&creds.token)
        .header("Accept", "application/json");
    if let Some(id) = &creds.account_id {
        r = match reqwest::header::HeaderValue::from_str(id) {
            Ok(mut v) => {
                v.set_sensitive(true);
                r.header("ChatGPT-Account-Id", v)
            }
            Err(_) => r,
        };
    }
    r
}

/// Codex probe: the token is only ever sent over HTTPS to its fixed endpoint
/// (no redirects), and any failure silently omits the provider.
pub async fn probe_codex() -> Option<LimitProvider> {
    let creds = tauri::async_runtime::spawn_blocking(read_codex_creds).await.ok()??;
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .user_agent(concat!("AgentPet/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let mut fetched = fetch_codex_once(codex_request(&client, &creds)).await;
    if fetched.is_none() {
        tokio::time::sleep(Duration::from_secs(1)).await;
        fetched = fetch_codex_once(codex_request(&client, &creds)).await;
    }
    let (headers, body) = fetched??;
    let pairs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    parse_codex_usage(&pairs, &body, now_secs())
}

/// Probes the requested providers (`None` = all) concurrently; a failing
/// provider is omitted, never an error.
pub async fn probe_selected(only: Option<Vec<String>>) -> Vec<LimitProvider> {
    let want = |id: &str| only.as_ref().map_or(true, |o| o.iter().any(|x| x == id));
    let codex = want("codex").then(|| tauri::async_runtime::spawn(probe_codex()));
    let claude = if want("claude") { probe_claude(default_paths()).await } else { None };
    let codex = match codex {
        Some(handle) => handle.await.ok().flatten(),
        None => None,
    };
    claude.into_iter().chain(codex).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: f64 = 1_800_000_000.0;
    fn future_ms() -> f64 { (NOW + 3600.0) * 1000.0 }
    fn past_ms() -> f64 { (NOW - 60.0) * 1000.0 }

    fn auth(accounts: Value, active: Option<&str>) -> Vec<u8> {
        let mut j = json!({ "anthropic_accounts": accounts });
        if let Some(a) = active {
            j["active_anthropic_account"] = json!(a);
        }
        serde_json::to_vec(&j).unwrap()
    }

    #[test]
    fn active_account_wins() {
        let d = auth(json!([
            {"label": "a", "access": "tok-a", "expires": future_ms()},
            {"label": "b", "access": "tok-b", "expires": future_ms()}
        ]), Some("b"));
        assert_eq!(jcode_access_token(&d, NOW).as_deref(), Some("tok-b"));
    }

    #[test]
    fn falls_back_to_first_account_when_active_unknown() {
        let one = json!([{"label": "a", "access": "tok-a", "expires": future_ms()}]);
        assert_eq!(jcode_access_token(&auth(one.clone(), Some("gone")), NOW).as_deref(), Some("tok-a"));
        assert_eq!(jcode_access_token(&auth(one, None), NOW).as_deref(), Some("tok-a"));
    }

    #[test]
    fn expired_token_is_skipped() {
        let d = auth(json!([{"label": "a", "access": "tok-a", "expires": past_ms()}]), Some("a"));
        assert_eq!(jcode_access_token(&d, NOW), None);
    }

    #[test]
    fn malformed_or_empty_is_none() {
        assert_eq!(jcode_access_token(b"not json", NOW), None);
        assert_eq!(jcode_access_token(b"{}", NOW), None);
        assert_eq!(jcode_access_token(&auth(json!([]), None), NOW), None);
        let blank = auth(json!([{"label": "a", "access": "", "expires": future_ms()}]), Some("a"));
        assert_eq!(jcode_access_token(&blank, NOW), None);
    }

    #[test]
    fn expiry_units() {
        assert!(is_expired(Some(&json!((NOW - 1.0) * 1000.0)), NOW), "ms, past");
        assert!(!is_expired(Some(&json!((NOW + 60.0) * 1000.0)), NOW), "ms, future");
        assert!(is_expired(Some(&json!(NOW - 1.0)), NOW), "seconds, past");
        assert!(!is_expired(Some(&json!(NOW + 60.0)), NOW), "seconds, future");
        assert!(!is_expired(None, NOW), "unknown expiry: let the API decide");
        assert!(!is_expired(Some(&json!("soon")), NOW));
    }

    #[test]
    fn claude_code_credentials_shape() {
        let ok = serde_json::to_vec(&json!({"claudeAiOauth": {"accessToken": "cc", "expiresAt": future_ms()}})).unwrap();
        assert_eq!(claude_code_access_token(&ok, NOW).as_deref(), Some("cc"));
        let old = serde_json::to_vec(&json!({"claudeAiOauth": {"accessToken": "cc", "expiresAt": past_ms()}})).unwrap();
        assert_eq!(claude_code_access_token(&old, NOW), None);
        // A jcode file is not mistaken for Claude Code credentials.
        let j = auth(json!([{"label": "a", "access": "tok-a"}]), Some("a"));
        assert_eq!(claude_code_access_token(&j, NOW), None);
    }

    #[test]
    fn claude_keeps_both_windows_with_resets() {
        let body = json!({
            "five_hour": {"utilization": 37.0, "resets_at": "2026-10-02T23:59:59.536032+00:00"},
            "seven_day": {"utilization": 79.0, "resets_at": "2026-10-05T03:59:59.536051+00:00"}
        });
        let p = claude_provider(&body).unwrap();
        assert_eq!(p.windows.iter().map(|w| w.label.as_str()).collect::<Vec<_>>(), ["Session", "Weekly"]);
        assert!((p.windows[0].fraction_left - 0.63).abs() < 1e-9);
        assert!((p.windows[1].fraction_left - 0.21).abs() < 1e-9);
        assert_eq!(p.windows[0].period, 18_000.0);
        assert_eq!(p.windows[1].period, 604_800.0);
        assert!(p.windows.iter().all(|w| w.resets_at.is_some()));
    }

    #[test]
    fn claude_clamps_and_skips_missing_windows() {
        let p = claude_provider(&json!({"five_hour": {"utilization": 130}, "seven_day": null})).unwrap();
        assert_eq!(p.windows.len(), 1);
        assert_eq!(p.windows[0].fraction_left, 0.0);
        assert_eq!(p.windows[0].resets_at, None);
        assert_eq!(claude_provider(&json!({"error": "x"})), None);
    }

    #[test]
    fn candidates_dedupe_and_skip_unreadable() {
        let dir = std::env::temp_dir().join(format!("ap-limits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cc = dir.join("cc.json");
        let jc = dir.join("auth.json");
        std::fs::write(&cc, serde_json::to_vec(&json!({"claudeAiOauth": {"accessToken": "same"}})).unwrap()).unwrap();
        std::fs::write(&jc, auth(json!([{"label": "a", "access": "same"}, {"label": "b", "access": "other"}]), Some("a"))).unwrap();
        let got = candidate_tokens(&[dir.join("missing.json"), cc, jc], NOW);
        assert_eq!(got, vec!["same".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn cx(extra: &str, tok: &str) -> String {
        format!(r#"{{"tokens":{{"access_token":"{tok}"{extra}}}}}"#)
    }

    fn win<'a>(l: &'a LimitProvider, label: &str) -> &'a LimitWindow {
        l.windows.iter().find(|w| w.label == label).unwrap()
    }

    #[test]
    fn codex_auth_valid() {
        let c = parse_codex_auth(&cx(r#","account_id":"acct-fake""#, "tok-fake")).unwrap();
        assert_eq!(c.token, "tok-fake");
        assert_eq!(c.account_id.as_deref(), Some("acct-fake"));
        let c = parse_codex_auth(&cx("", "tok-fake")).unwrap();
        assert_eq!(c.account_id, None);
    }

    #[test]
    fn codex_auth_invalid() {
        assert!(parse_codex_auth(r#"{"tokens":{}}"#).is_none());
        assert!(parse_codex_auth(r#"{"other":1}"#).is_none());
        assert!(parse_codex_auth(&cx("", "")).is_none());
        assert!(parse_codex_auth("{nope").is_none());
    }

    const H1: (&str, &str) = ("x-codex-primary-used-percent", "30");
    const H2: (&str, &str) = ("x-codex-secondary-used-percent", "60.5");

    #[test]
    fn codex_headers_only() {
        let l = parse_codex_usage(&[H1, H2], b"garbage", NOW).unwrap();
        assert_eq!(l.id, "codex");
        assert_eq!(l.display_name, "Codex");
        assert!((win(&l, "Session").fraction_left - 0.7).abs() < 1e-9);
        assert!((win(&l, "Weekly").fraction_left - 0.395).abs() < 1e-9);
        assert_eq!(win(&l, "Session").period, 18_000.0);
        assert_eq!(win(&l, "Weekly").period, 604_800.0);
        assert_eq!(win(&l, "Weekly").resets_at, None);
    }

    #[test]
    fn codex_body_only() {
        let b = br#"{"rate_limit":{"primary_window":{"used_percent":12,"resets_in_seconds":90},"secondary_window":{"used_percent":40}}}"#;
        let l = parse_codex_usage(&[], b, NOW).unwrap();
        assert!((win(&l, "Session").fraction_left - 0.88).abs() < 1e-9);
        assert_eq!(win(&l, "Session").resets_at.as_deref(), Some("1800000090"));
        assert!((win(&l, "Weekly").fraction_left - 0.6).abs() < 1e-9);
    }

    #[test]
    fn codex_headers_override_body() {
        let b = br#"{"rate_limit":{"primary_window":{"used_percent":99,"reset_after_seconds":60},"secondary_window":{"used_percent":98}}}"#;
        let l = parse_codex_usage(&[H1], b, NOW).unwrap();
        assert!((win(&l, "Session").fraction_left - 0.7).abs() < 1e-9);
        assert_eq!(win(&l, "Session").resets_at.as_deref(), Some("1800000060"));
        assert!((win(&l, "Weekly").fraction_left - 0.02).abs() < 1e-9);
    }

    fn primary(w: &str) -> Vec<u8> {
        format!(r#"{{"rate_limit":{{"primary_window":{{"used_percent":5,{w}}}}}}}"#).into_bytes()
    }

    #[test]
    fn codex_reset_at_wins_in_seconds_or_ms() {
        let b = primary(r#""reset_at":1800000600,"reset_after_seconds":60"#);
        let l = parse_codex_usage(&[], &b, NOW).unwrap();
        assert_eq!(l.windows[0].resets_at.as_deref(), Some("1800000600"));
        let b = primary(r#""reset_at":1800000600123"#);
        let l = parse_codex_usage(&[], &b, NOW).unwrap();
        assert_eq!(l.windows[0].resets_at.as_deref(), Some("1800000600"));
    }

    #[test]
    fn codex_reset_huge_or_negative_seconds() {
        for s in ["1e16", "1e300"] {
            let l = parse_codex_usage(&[], &primary(&format!(r#""reset_after_seconds":{s}"#)), NOW).unwrap();
            assert_eq!(l.windows[0].resets_at.as_deref(), Some("4102444800"));
        }
        let l = parse_codex_usage(&[], &primary(r#""reset_after_seconds":-30"#), NOW).unwrap();
        assert_eq!(l.windows[0].resets_at, None);
    }

    #[test]
    fn codex_one_window_none_and_clamp() {
        let l = parse_codex_usage(&[H2], b"", NOW).unwrap();
        assert_eq!(l.windows.len(), 1);
        assert_eq!(l.windows[0].label, "Weekly");
        assert!(parse_codex_usage(&[], b"{}", NOW).is_none());
        assert!(parse_codex_usage(&[("x-codex-primary-used-percent", "abc")], b"", NOW).is_none());
        let l = parse_codex_usage(
            &[("x-codex-primary-used-percent", "120"), ("x-codex-secondary-used-percent", "-5")],
            b"",
            NOW,
        )
        .unwrap();
        assert_eq!(win(&l, "Session").fraction_left, 0.0);
        assert_eq!(win(&l, "Weekly").fraction_left, 1.0);
    }
}
