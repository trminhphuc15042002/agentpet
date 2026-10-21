//! Claude and Codex subscription usage limits (session and weekly windows).
//!
//! Credential sources read (read-only, never written, refreshed, copied or cached):
//! - `~/.claude/.credentials.json` (`claudeAiOauth.accessToken`, `claudeAiOauth.expiresAt`)
//! - `~/.codex/auth.json`, else `~/.config/codex/auth.json`
//!   (`tokens.access_token`, optional `tokens.account_id`)
//!
//! Each token is only sent over HTTPS to its fixed provider usage endpoint
//! (api.anthropic.com, chatgpt.com). Probes are independent and run concurrently;
//! every failure silently omits only that provider.

use serde::Serialize;
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const MAX_CRED_BYTES: u64 = 1024 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub label: String,
    pub used_pct: f64,
    pub resets_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLimit {
    pub id: String,
    pub name: String,
    pub windows: Vec<LimitWindow>,
}

fn epoch_to_ms(n: f64) -> i64 {
    if n > 1e11 {
        n as i64
    } else {
        (n * 1000.0) as i64
    }
}

pub fn parse_claude_credentials(text: &str, now_ms: i64) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    let oauth = v.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?;
    if token.is_empty() {
        return None;
    }
    if let Some(exp) = oauth.get("expiresAt").and_then(Value::as_f64) {
        if epoch_to_ms(exp) <= now_ms {
            return None;
        }
    }
    Some(token.to_string())
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || !s.is_ascii() {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    if b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b't' || b[10] == b' ')
        || b[13] != b':' || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac_ms = 0i64;
    if b[i] == b'.' {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits: String = s[start..i].chars().chain("000".chars()).take(3).collect();
        frac_ms = digits.parse().ok()?;
    }
    let offset_s = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let oh = num(i + 1, i + 3)?;
            let om = num(i + 4, i + 6)?;
            let t = oh * 3600 + om * 60;
            if *sign == b'+' {
                t
            } else {
                -t
            }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 86400 + h * 3600 + mi * 60 + sec - offset_s) * 1000) + frac_ms)
}

fn parse_window(v: &Value, label: &str) -> Option<LimitWindow> {
    let obj = v.as_object()?;
    let used = obj.get("utilization")?.as_f64()?;
    if !used.is_finite() {
        return None;
    }
    let resets_at_ms = match obj.get("resets_at") {
        Some(Value::Number(n)) => n.as_f64().map(epoch_to_ms),
        Some(Value::String(s)) => parse_rfc3339_ms(s),
        _ => None,
    };
    Some(LimitWindow {
        label: label.to_string(),
        used_pct: used.clamp(0.0, 100.0),
        resets_at_ms,
    })
}

pub fn parse_claude_usage(body: &[u8]) -> Option<ProviderLimit> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let windows: Vec<LimitWindow> = [("five_hour", "session"), ("seven_day", "weekly")]
        .iter()
        .filter_map(|(key, label)| parse_window(v.get(*key)?, label))
        .collect();
    if windows.is_empty() {
        return None;
    }
    Some(ProviderLimit {
        id: "claude".to_string(),
        name: "Claude".to_string(),
        windows,
    })
}

/// Never Debug/Serialize: the token and account id must not be printable.
pub struct CodexCreds {
    token: String,
    account_id: Option<String>,
}

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
    Some(CodexCreds {
        token: token.to_string(),
        account_id,
    })
}

fn codex_window(
    headers: &[(&str, &str)],
    header: &str,
    body: Option<&Value>,
    label: &str,
    now_ms: i64,
) -> Option<LimitWindow> {
    let from_header = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(header))
        .and_then(|(_, v)| v.trim().parse::<f64>().ok());
    let used = from_header
        .or_else(|| body?.get("used_percent")?.as_f64())
        .filter(|u| u.is_finite())?;
    let secs = body.and_then(|w| {
        ["reset_after_seconds", "resets_in_seconds"]
            .iter()
            .find_map(|k| w.get(*k)?.as_f64())
    });
    let resets_at_ms = body
        .and_then(|w| w.get("reset_at")?.as_f64())
        .map(epoch_to_ms)
        .or_else(|| {
            secs.filter(|s| s.is_finite() && *s >= 0.0)
                .map(|s| now_ms.saturating_add((s * 1000.0) as i64))
        });
    Some(LimitWindow {
        label: label.to_string(),
        used_pct: used.clamp(0.0, 100.0),
        resets_at_ms,
    })
}

pub fn parse_codex_usage(headers: &[(&str, &str)], body: &[u8], now_ms: i64) -> Option<ProviderLimit> {
    let v: Option<Value> = serde_json::from_slice(body).ok();
    let rate = v.as_ref().and_then(|v| v.get("rate_limit"));
    let windows: Vec<LimitWindow> = [
        ("x-codex-primary-used-percent", "primary_window", "session"),
        ("x-codex-secondary-used-percent", "secondary_window", "weekly"),
    ]
    .iter()
    .filter_map(|(h, key, label)| codex_window(headers, h, rate.and_then(|r| r.get(*key)), label, now_ms))
    .collect();
    if windows.is_empty() {
        return None;
    }
    Some(ProviderLimit {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        windows,
    })
}

fn now_ms() -> Option<i64> {
    Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as i64,
    )
}

fn read_capped(path: PathBuf) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_CRED_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_CRED_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

fn read_claude_token() -> Option<String> {
    let text = read_capped(dirs::home_dir()?.join(".claude").join(".credentials.json"))?;
    parse_claude_credentials(&text, now_ms()?)
}

fn read_codex_creds() -> Option<CodexCreds> {
    let home = dirs::home_dir()?;
    let text = read_capped(home.join(".codex").join("auth.json"))
        .or_else(|| read_capped(home.join(".config").join("codex").join("auth.json")))?;
    parse_codex_auth(&text)
}

type Fetched = (Vec<(String, String)>, Vec<u8>);

async fn fetch_once(req: reqwest::RequestBuilder) -> Option<Option<Fetched>> {
    // Outer None: retryable failure. Some(None): final failure. Some(Some(r)): response.
    let mut resp = match req.send().await {
        Ok(r) => r,
        Err(_) => return None,
    };
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

fn build_client() -> Option<reqwest::Client> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .user_agent(concat!("AgentPet/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()
}

async fn fetch_with_retry(
    client: &reqwest::Client,
    build: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
) -> Option<Fetched> {
    let mut result = fetch_once(build(client)).await;
    if result.is_none() {
        let _ = tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(Duration::from_secs(1))
        })
        .await;
        result = fetch_once(build(client)).await;
    }
    result?
}

async fn fetch_claude(token: &str) -> Option<ProviderLimit> {
    let client = build_client()?;
    let (_, body) = fetch_with_retry(&client, |c| {
        c.get(USAGE_URL)
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
    })
    .await?;
    parse_claude_usage(&body)
}

async fn fetch_codex(creds: &CodexCreds) -> Option<ProviderLimit> {
    let client = build_client()?;
    let (headers, body) = fetch_with_retry(&client, |c| {
        let mut r = c
            .get(CODEX_USAGE_URL)
            .bearer_auth(&creds.token)
            .header("Accept", "application/json");
        if let Some(id) = &creds.account_id {
            r = match reqwest::header::HeaderValue::from_str(id) {
                Ok(mut v) => {
                    v.set_sensitive(true);
                    r.header("ChatGPT-Account-Id", v)
                }
                Err(_) => r.header("ChatGPT-Account-Id", id),
            };
        }
        r
    })
    .await?;
    let pairs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    parse_codex_usage(&pairs, &body, now_ms()?)
}

async fn claude_probe() -> Option<ProviderLimit> {
    let token = tauri::async_runtime::spawn_blocking(read_claude_token).await.ok()??;
    fetch_claude(&token).await
}

async fn codex_probe() -> Option<ProviderLimit> {
    let creds = tauri::async_runtime::spawn_blocking(read_codex_creds).await.ok()??;
    fetch_codex(&creds).await
}

#[tauri::command]
pub async fn usage_limits() -> Vec<ProviderLimit> {
    let codex = tauri::async_runtime::spawn(codex_probe());
    let claude = claude_probe().await;
    let codex = codex.await.ok().flatten();
    claude.into_iter().chain(codex).collect()
}

#[cfg(test)]
mod limits_tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn creds(extra: &str, tok: &str) -> String {
        format!(r#"{{"claudeAiOauth":{{"accessToken":"{tok}"{extra}}}}}"#)
    }

    #[test]
    fn creds_valid() {
        let t = creds(r#","expiresAt":1900000000000"#, "sk-test-fake");
        assert_eq!(parse_claude_credentials(&t, NOW).as_deref(), Some("sk-test-fake"));
    }

    #[test]
    fn creds_expired_seconds() {
        let t = creds(r#","expiresAt":1700000000"#, "sk-test-fake");
        assert_eq!(parse_claude_credentials(&t, NOW), None);
    }

    #[test]
    fn creds_expired_ms() {
        let t = creds(r#","expiresAt":1700000000000"#, "sk-test-fake");
        assert_eq!(parse_claude_credentials(&t, NOW), None);
    }

    #[test]
    fn creds_missing_expiry() {
        let t = creds("", "sk-test-fake");
        assert!(parse_claude_credentials(&t, NOW).is_some());
    }

    #[test]
    fn creds_empty_token() {
        assert_eq!(parse_claude_credentials(&creds("", ""), NOW), None);
    }

    #[test]
    fn creds_malformed() {
        assert_eq!(parse_claude_credentials("{not json", NOW), None);
    }

    fn win<'a>(l: &'a ProviderLimit, label: &str) -> &'a LimitWindow {
        l.windows.iter().find(|w| w.label == label).unwrap()
    }

    #[test]
    fn usage_both() {
        let b = br#"{"five_hour":{"utilization":42.5,"resets_at":1800000000},"seven_day":{"utilization":10,"resets_at":null}}"#;
        let l = parse_claude_usage(b).unwrap();
        assert_eq!(l.id, "claude");
        assert_eq!(l.windows.len(), 2);
        assert_eq!(win(&l, "session").used_pct, 42.5);
        assert_eq!(win(&l, "weekly").used_pct, 10.0);
        assert_eq!(win(&l, "weekly").resets_at_ms, None);
    }

    #[test]
    fn usage_only_weekly() {
        let l = parse_claude_usage(br#"{"seven_day":{"utilization":3}}"#).unwrap();
        assert_eq!(l.windows.len(), 1);
        assert_eq!(l.windows[0].label, "weekly");
        assert_eq!(l.windows[0].resets_at_ms, None);
    }

    #[test]
    fn usage_neither() {
        assert!(parse_claude_usage(br#"{"other":{}}"#).is_none());
        assert!(parse_claude_usage(b"garbage").is_none());
    }

    #[test]
    fn usage_clamped() {
        let l = parse_claude_usage(
            br#"{"five_hour":{"utilization":120},"seven_day":{"utilization":-5}}"#,
        )
        .unwrap();
        assert_eq!(win(&l, "session").used_pct, 100.0);
        assert_eq!(win(&l, "weekly").used_pct, 0.0);
    }

    #[test]
    fn usage_resets_epoch() {
        let l = parse_claude_usage(br#"{"five_hour":{"utilization":1,"resets_at":1800000000}}"#)
            .unwrap();
        assert_eq!(l.windows[0].resets_at_ms, Some(1_800_000_000_000));
        let l = parse_claude_usage(br#"{"five_hour":{"utilization":1,"resets_at":1800000000123}}"#)
            .unwrap();
        assert_eq!(l.windows[0].resets_at_ms, Some(1_800_000_000_123));
    }

    fn cx(extra: &str, tok: &str) -> String {
        format!(r#"{{"tokens":{{"access_token":"{tok}"{extra}}}}}"#)
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
        assert_eq!(l.name, "Codex");
        assert_eq!(win(&l, "session").used_pct, 30.0);
        assert_eq!(win(&l, "weekly").used_pct, 60.5);
        assert_eq!(win(&l, "weekly").resets_at_ms, None);
    }

    #[test]
    fn codex_body_only() {
        let b = br#"{"rate_limit":{"primary_window":{"used_percent":12,"resets_in_seconds":90},"secondary_window":{"used_percent":40}}}"#;
        let l = parse_codex_usage(&[], b, NOW).unwrap();
        assert_eq!(win(&l, "session").used_pct, 12.0);
        assert_eq!(win(&l, "session").resets_at_ms, Some(NOW + 90_000));
        assert_eq!(win(&l, "weekly").used_pct, 40.0);
    }

    #[test]
    fn codex_headers_override_body() {
        let b = br#"{"rate_limit":{"primary_window":{"used_percent":99,"reset_after_seconds":60},"secondary_window":{"used_percent":98}}}"#;
        let l = parse_codex_usage(&[H1], b, NOW).unwrap();
        assert_eq!(win(&l, "session").used_pct, 30.0);
        assert_eq!(win(&l, "session").resets_at_ms, Some(NOW + 60_000));
        assert_eq!(win(&l, "weekly").used_pct, 98.0);
    }

    fn primary(w: &str) -> Vec<u8> {
        format!(r#"{{"rate_limit":{{"primary_window":{{"used_percent":5,{w}}}}}}}"#).into_bytes()
    }

    #[test]
    fn codex_reset_at_wins() {
        let b = primary(r#""reset_at":1800000600,"reset_after_seconds":60"#);
        let l = parse_codex_usage(&[], &b, NOW).unwrap();
        assert_eq!(l.windows[0].resets_at_ms, Some(1_800_000_600_000));
    }

    #[test]
    fn codex_reset_at_ms_and_relative_only() {
        let b = primary(r#""reset_at":1800000600123"#);
        let l = parse_codex_usage(&[], &b, NOW).unwrap();
        assert_eq!(l.windows[0].resets_at_ms, Some(1_800_000_600_123));
        let b = primary(r#""reset_after_seconds":120"#);
        let l = parse_codex_usage(&[], &b, NOW).unwrap();
        assert_eq!(l.windows[0].resets_at_ms, Some(NOW + 120_000));
    }

    #[test]
    fn codex_reset_huge_or_negative_seconds() {
        for s in ["1e16", "1e300", "-30"] {
            let b = primary(&format!(r#""reset_after_seconds":{s}"#));
            let l = parse_codex_usage(&[], &b, NOW).unwrap();
            match l.windows[0].resets_at_ms {
                Some(ms) => assert!(ms >= NOW),
                None => assert_eq!(s, "-30"),
            }
        }
        let l = parse_codex_usage(&[], &primary(r#""reset_after_seconds":1e300"#), i64::MAX).unwrap();
        assert_eq!(l.windows[0].resets_at_ms, Some(i64::MAX));
    }

    #[test]
    fn codex_one_window_none_and_clamp() {
        let l = parse_codex_usage(&[H2], b"", NOW).unwrap();
        assert_eq!(l.windows.len(), 1);
        assert_eq!(l.windows[0].label, "weekly");
        assert!(parse_codex_usage(&[], b"{}", NOW).is_none());
        assert!(parse_codex_usage(&[("x-codex-primary-used-percent", "abc")], b"", NOW).is_none());
        let l = parse_codex_usage(
            &[("x-codex-primary-used-percent", "120"), ("x-codex-secondary-used-percent", "-5")],
            b"",
            NOW,
        )
        .unwrap();
        assert_eq!(win(&l, "session").used_pct, 100.0);
        assert_eq!(win(&l, "weekly").used_pct, 0.0);
    }

    fn reset_of(s: &str) -> Option<i64> {
        let b = format!(r#"{{"five_hour":{{"utilization":1,"resets_at":"{s}"}}}}"#);
        parse_claude_usage(b.as_bytes()).unwrap().windows[0].resets_at_ms
    }

    #[test]
    fn usage_resets_iso() {
        // 2027-01-15T08:00:00Z == 1800000000
        assert_eq!(reset_of("2027-01-15T08:00:00Z"), Some(1_800_000_000_000));
        assert_eq!(reset_of("2027-01-15T10:00:00+02:00"), Some(1_800_000_000_000));
        assert_eq!(reset_of("2027-01-15T03:30:00-04:30"), Some(1_800_000_000_000));
        assert_eq!(reset_of("2027-01-15T08:00:00.500Z"), Some(1_800_000_000_500));
        assert_eq!(
            reset_of("2027-01-15T08:00:00.123456+00:00"),
            Some(1_800_000_000_123)
        );
        assert_eq!(reset_of("garbage"), None);
    }

    #[test]
    fn usage_integer_utilization() {
        let l = parse_claude_usage(br#"{"five_hour":{"utilization":7}}"#).unwrap();
        assert_eq!(l.windows[0].used_pct, 7.0);
    }
}
