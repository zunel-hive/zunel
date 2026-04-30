//! Refresh-token rotation for cached MCP OAuth credentials.
//!
//! `zunel mcp login` does the initial **authorization-code** flow
//! and writes `~/.zunel/mcp-oauth/<server>/token.json`. That access
//! token is short-lived (Atlassian = ~8h) and historically zunel
//! never refreshed it: the gateway would happily attach the dead
//! `Authorization: Bearer …` header at every connect, the server
//! would 401, and the MCP entry would be silently dropped from the
//! tool registry until the operator re-ran `mcp login --force`.
//!
//! This module closes that loop. Before the agent loop wires up
//! remote MCP servers it calls [`refresh_if_needed`], which
//! consults the cached `obtainedAt + expiresIn`, and — when the
//! token is past expiry minus a small skew — POSTs
//! `grant_type=refresh_token` to the saved `tokenUrl` and rewrites
//! the cache atomically. On unrecoverable failures (refresh token
//! revoked, missing `token_url`, etc.) the helper returns a
//! descriptive [`Outcome`] variant so the caller can decide what
//! to do (typically: log and let the dead bearer header still
//! attempt the connect, mirroring the pre-refresh behavior so we
//! never *regress* an env that was already broken).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use zunel_config::{
    mcp_oauth::{load_token, save_token, unix_timestamp_now, DEFAULT_REFRESH_SKEW_SECS},
    CachedMcpOAuthToken,
};

/// Result of a [`refresh_if_needed`] call. Variants are exhaustive
/// so `register_mcp_tools` can pattern-match and emit appropriately
/// scoped log lines (info vs warn).
#[derive(Debug)]
pub enum Outcome {
    /// No `token.json` on disk yet; nothing to refresh.
    NotCached,
    /// Cached token is still within its TTL (minus skew). Includes
    /// remaining seconds for log breadcrumbs.
    StillFresh { secs_remaining: u64 },
    /// Token had no `expires_in` value; we treat it as long-lived
    /// and skip refresh. Caller may still get a 401 at connect
    /// time, in which case the cleanup path is `mcp login --force`.
    NoExpiry,
    /// Token was expired but no `refresh_token` is on file (e.g.
    /// the IdP didn't issue one). Caller should fall through to
    /// the existing dead-bearer code path; only `mcp login` can
    /// recover.
    NoRefreshToken,
    /// Token was expired and the cached metadata didn't include a
    /// `tokenUrl`. Same recovery as `NoRefreshToken`.
    NoTokenUrl,
    /// Refresh succeeded; the on-disk cache has been rewritten.
    Refreshed { new_expires_in: Option<u64> },
    /// Refresh attempt failed (network, 4xx from the IdP, malformed
    /// response). The on-disk cache is untouched.
    RefreshFailed(String),
}

impl Outcome {
    /// `true` when the caller can safely attach the cached
    /// `Authorization` header and expect it to work. False for
    /// states where the header is known to be stale (the caller
    /// will probably still try, but won't be surprised by a 401).
    pub fn header_likely_valid(&self) -> bool {
        matches!(
            self,
            Outcome::StillFresh { .. } | Outcome::Refreshed { .. } | Outcome::NoExpiry
        )
    }
}

/// Refresh the cached OAuth token for `server_name` if it's past
/// its expiry minus [`DEFAULT_REFRESH_SKEW_SECS`].
///
/// On success the on-disk cache is rewritten in place via the
/// atomic-rename path provided by [`zunel_config::mcp_oauth`]. The
/// function never panics on bad input; every failure mode maps to
/// an [`Outcome`] variant the caller can log.
pub async fn refresh_if_needed(home: &Path, server_name: &str) -> Outcome {
    refresh_if_needed_with(
        home,
        server_name,
        DEFAULT_REFRESH_SKEW_SECS,
        unix_timestamp_now(),
        &reqwest::Client::new(),
    )
    .await
}

/// Test-friendly variant of [`refresh_if_needed`] that lets tests
/// inject a deterministic clock and reqwest client (e.g. one
/// pointed at a `wiremock::MockServer`).
pub async fn refresh_if_needed_with(
    home: &Path,
    server_name: &str,
    skew_secs: u64,
    now_unix: u64,
    client: &reqwest::Client,
) -> Outcome {
    let mut token = match load_token(home, server_name) {
        Ok(Some(token)) => token,
        Ok(None) => return Outcome::NotCached,
        Err(err) => {
            tracing::warn!(
                server = server_name,
                error = %err,
                "ignoring invalid MCP OAuth token cache"
            );
            return Outcome::NotCached;
        }
    };

    let secs_remaining = match token.expires_at() {
        Some(exp) => exp.saturating_sub(now_unix),
        None => return Outcome::NoExpiry,
    };

    if !token.is_expired(now_unix, skew_secs) {
        return Outcome::StillFresh { secs_remaining };
    }

    let Some(refresh_token) = token.refresh_token.clone() else {
        return Outcome::NoRefreshToken;
    };
    if token.token_url.is_empty() {
        return Outcome::NoTokenUrl;
    }

    match exchange_refresh_token(client, &token, &refresh_token).await {
        Ok(refreshed) => {
            apply_refresh(&mut token, refreshed, now_unix);
            if let Err(err) = save_token(home, server_name, &token) {
                return Outcome::RefreshFailed(format!("persisting refreshed token: {err}"));
            }
            Outcome::Refreshed {
                new_expires_in: token.expires_in,
            }
        }
        Err(err) => Outcome::RefreshFailed(err),
    }
}

#[derive(Debug)]
struct RefreshedFields {
    access_token: String,
    token_type: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
}

async fn exchange_refresh_token(
    client: &reqwest::Client,
    token: &CachedMcpOAuthToken,
    refresh_token: &str,
) -> Result<RefreshedFields, String> {
    let mut form: BTreeMap<&str, String> = BTreeMap::new();
    form.insert("grant_type", "refresh_token".into());
    form.insert("refresh_token", refresh_token.to_string());
    form.insert("client_id", token.client_id.clone());
    if let Some(secret) = &token.client_secret {
        form.insert("client_secret", secret.clone());
    }

    let response = client
        .post(&token.token_url)
        .form(&form)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|err| format!("token endpoint request failed: {err}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("reading token endpoint body: {err}"))?;
    if !status.is_success() {
        // Truncate the body so a verbose IdP error page doesn't
        // dominate the log line; 256 chars is enough to spot
        // `invalid_grant` / `invalid_client` style markers.
        let snippet = body.chars().take(256).collect::<String>();
        return Err(format!(
            "token endpoint returned HTTP {status}: {}",
            snippet.trim()
        ));
    }
    let json: Value = serde_json::from_str(&body)
        .map_err(|err| format!("token endpoint returned non-JSON body: {err}"))?;
    let access_token = json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "token endpoint response missing access_token".to_string())?
        .to_string();
    Ok(RefreshedFields {
        access_token,
        token_type: json
            .get("token_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        refresh_token: json
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        expires_in: json.get("expires_in").and_then(Value::as_u64),
        scope: json
            .get("scope")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn apply_refresh(token: &mut CachedMcpOAuthToken, refreshed: RefreshedFields, now_unix: u64) {
    token.access_token = refreshed.access_token;
    if let Some(token_type) = refreshed.token_type {
        token.token_type = Some(token_type);
    }
    // Some IdPs rotate refresh tokens (RFC 6749 §10.4). Persist the
    // new value when present, but don't *erase* the existing one if
    // the response omits it.
    if let Some(rt) = refreshed.refresh_token {
        token.refresh_token = Some(rt);
    }
    if let Some(expires_in) = refreshed.expires_in {
        token.expires_in = Some(expires_in);
    }
    if let Some(scope) = refreshed.scope {
        token.scope = Some(scope);
    }
    token.obtained_at = now_unix;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zunel_config::mcp_oauth::mcp_oauth_token_path;

    fn cached(token_url: &str) -> CachedMcpOAuthToken {
        CachedMcpOAuthToken {
            access_token: "old-access".into(),
            token_type: Some("Bearer".into()),
            refresh_token: Some("old-refresh".into()),
            expires_in: Some(3600),
            scope: Some("read".into()),
            obtained_at: 1_000,
            client_id: "client-id".into(),
            client_secret: None,
            authorization_url: "https://example.test/authorize".into(),
            token_url: token_url.into(),
        }
    }

    #[tokio::test]
    async fn skips_refresh_when_token_is_fresh() {
        let dir = TempDir::new().unwrap();
        let token = cached("https://nope.example.test/token");
        save_token(dir.path(), "srv", &token).unwrap();

        let outcome = refresh_if_needed_with(
            dir.path(),
            "srv",
            60,
            token.obtained_at + 100,
            &reqwest::Client::new(),
        )
        .await;
        match outcome {
            Outcome::StillFresh { secs_remaining } => assert_eq!(secs_remaining, 3500),
            other => panic!("expected StillFresh, got {other:?}"),
        }

        // Cache file is untouched.
        let after = load_token(dir.path(), "srv").unwrap().unwrap();
        assert_eq!(after.access_token, "old-access");
    }

    #[tokio::test]
    async fn refreshes_when_expired_and_persists_rotated_refresh_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "new-access",
                "token_type": "Bearer",
                "refresh_token": "rotated-refresh",
                "expires_in": 7200,
                "scope": "read write"
            })))
            .mount(&server)
            .await;

        let dir = TempDir::new().unwrap();
        let token = cached(&format!("{}/token", server.uri()));
        save_token(dir.path(), "srv", &token).unwrap();

        let now = token.obtained_at + token.expires_in.unwrap() + 1;
        let outcome =
            refresh_if_needed_with(dir.path(), "srv", 60, now, &reqwest::Client::new()).await;
        match outcome {
            Outcome::Refreshed { new_expires_in } => assert_eq!(new_expires_in, Some(7200)),
            other => panic!("expected Refreshed, got {other:?}"),
        }

        let after = load_token(dir.path(), "srv").unwrap().unwrap();
        assert_eq!(after.access_token, "new-access");
        assert_eq!(after.refresh_token.as_deref(), Some("rotated-refresh"));
        assert_eq!(after.expires_in, Some(7200));
        assert_eq!(after.obtained_at, now);
        assert_eq!(after.scope.as_deref(), Some("read write"));
    }

    #[tokio::test]
    async fn keeps_existing_refresh_token_when_response_omits_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "new-access",
                "token_type": "Bearer",
                "expires_in": 1800
            })))
            .mount(&server)
            .await;

        let dir = TempDir::new().unwrap();
        let token = cached(&format!("{}/token", server.uri()));
        save_token(dir.path(), "srv", &token).unwrap();

        let now = token.obtained_at + token.expires_in.unwrap() + 1;
        let _ = refresh_if_needed_with(dir.path(), "srv", 60, now, &reqwest::Client::new()).await;

        let after = load_token(dir.path(), "srv").unwrap().unwrap();
        assert_eq!(after.access_token, "new-access");
        assert_eq!(after.refresh_token.as_deref(), Some("old-refresh"));
    }

    #[tokio::test]
    async fn idp_400_leaves_cache_intact_and_reports_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "Refresh token is invalid"
            })))
            .mount(&server)
            .await;

        let dir = TempDir::new().unwrap();
        let token = cached(&format!("{}/token", server.uri()));
        save_token(dir.path(), "srv", &token).unwrap();

        let now = token.obtained_at + token.expires_in.unwrap() + 1;
        let outcome =
            refresh_if_needed_with(dir.path(), "srv", 60, now, &reqwest::Client::new()).await;
        let msg = match outcome {
            Outcome::RefreshFailed(msg) => msg,
            other => panic!("expected RefreshFailed, got {other:?}"),
        };
        assert!(msg.contains("HTTP 400"), "got: {msg}");
        assert!(msg.contains("invalid_grant"), "got: {msg}");

        let after = load_token(dir.path(), "srv").unwrap().unwrap();
        assert_eq!(after.access_token, "old-access");
        assert_eq!(after.refresh_token.as_deref(), Some("old-refresh"));
    }

    #[tokio::test]
    async fn no_refresh_token_short_circuits() {
        let dir = TempDir::new().unwrap();
        let mut token = cached("https://example.test/token");
        token.refresh_token = None;
        save_token(dir.path(), "srv", &token).unwrap();

        let now = token.obtained_at + token.expires_in.unwrap() + 1;
        let outcome =
            refresh_if_needed_with(dir.path(), "srv", 60, now, &reqwest::Client::new()).await;
        assert!(matches!(outcome, Outcome::NoRefreshToken));
    }

    #[tokio::test]
    async fn no_cached_token_short_circuits() {
        let dir = TempDir::new().unwrap();
        let outcome = refresh_if_needed_with(
            dir.path(),
            "never-logged-in",
            60,
            42,
            &reqwest::Client::new(),
        )
        .await;
        assert!(matches!(outcome, Outcome::NotCached));
        // Sanity: nothing was created on disk.
        assert!(!mcp_oauth_token_path(dir.path(), "never-logged-in").exists());
    }
}
