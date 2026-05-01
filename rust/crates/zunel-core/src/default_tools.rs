//! Slice 3 default tool seeding.
//!
//! Builds a `ToolRegistry` populated with the standard zunel toolset
//! based on a [`Config`]. Read-only filesystem and search tools are
//! always seeded; `exec` and the web tools are gated behind their
//! respective `enable` flags to match Python's parity behavior.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;
use zunel_config::{Config, McpServerConfig, WebToolsConfig};
use zunel_mcp::{
    oauth as mcp_oauth_refresh, McpToolWrapper, RemoteMcpClient, RemoteTransport, StdioMcpClient,
};
use zunel_tools::{
    cron::CronTool,
    fs::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool},
    path_policy::PathPolicy,
    search::{GlobTool, GrepTool},
    shell::ExecTool,
    web::{WebFetchTool, WebSearchTool},
    BraveProvider, DuckDuckGoProvider, StubProvider, ToolRegistry, WebSearchProvider,
};

pub fn build_default_registry(cfg: &Config, workspace: &Path) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    let mut policy = PathPolicy::restricted(workspace);
    if let Some(media_dir) = cfg.tools.filesystem.media_dir.as_deref() {
        policy = policy.with_media_dir(media_dir);
    }
    registry.register(Arc::new(ReadFileTool::new(policy.clone())));
    registry.register(Arc::new(WriteFileTool::new(policy.clone())));
    registry.register(Arc::new(EditFileTool::new(policy.clone())));
    registry.register(Arc::new(ListDirTool::new(policy.clone())));
    registry.register(Arc::new(GlobTool::new(policy.clone())));
    registry.register(Arc::new(GrepTool::new(policy)));

    if cfg.tools.exec.enable {
        registry.register(Arc::new(ExecTool::new_default()));
    }
    if cfg.tools.web.enable {
        registry.register(Arc::new(WebFetchTool::new()));
        let provider = build_search_provider(&cfg.tools.web);
        registry.register(Arc::new(WebSearchTool::new(provider)));
    }
    registry.register(Arc::new(CronTool::new(
        workspace.join("cron").join("jobs.json"),
        "UTC",
    )));
    registry
}

pub async fn build_default_registry_async(cfg: &Config, workspace: &Path) -> ToolRegistry {
    let mut registry = build_default_registry(cfg, workspace);
    register_mcp_tools(&mut registry, cfg).await;
    registry
}

async fn register_mcp_tools(registry: &mut ToolRegistry, cfg: &Config) {
    for (server_name, server) in &cfg.tools.mcp_servers {
        let init_timeout = server.init_timeout.unwrap_or(10);
        let tool_timeout = server.tool_timeout.unwrap_or(30);
        let mut client: Box<dyn zunel_mcp::McpClient> = match mcp_transport(server) {
            McpTransport::Stdio => {
                let Some(command_raw) = server.command.as_deref() else {
                    tracing::warn!(server = %server_name, "skipping stdio MCP server without command");
                    continue;
                };
                let resolved_command = match resolve_stdio_command(command_raw) {
                    Ok(cmd) => cmd,
                    Err(err) => {
                        tracing::warn!(
                            server = %server_name,
                            command = command_raw,
                            error = %err,
                            "failed to resolve stdio MCP command"
                        );
                        continue;
                    }
                };
                let args = server.args.clone().unwrap_or_default();
                let env = server.env.clone().unwrap_or_default();
                match StdioMcpClient::connect(&resolved_command, &args, env, init_timeout).await {
                    Ok(client) => Box::new(client),
                    Err(err) => {
                        tracing::warn!(server = %server_name, error = %err, "failed to initialize MCP server");
                        continue;
                    }
                }
            }
            McpTransport::Remote(transport) => {
                let Some(url) = server.url.as_deref() else {
                    tracing::warn!(server = %server_name, "skipping remote MCP server without url");
                    continue;
                };
                let headers = remote_headers_with_cached_oauth(server_name, server).await;
                match RemoteMcpClient::connect(url, headers, transport, init_timeout).await {
                    Ok(client) => Box::new(client),
                    Err(err) => {
                        tracing::warn!(server = %server_name, error = %err, "failed to initialize MCP server");
                        continue;
                    }
                }
            }
            McpTransport::Unsupported(transport) => {
                tracing::warn!(server = %server_name, transport, "skipping unsupported MCP transport");
                continue;
            }
        };
        let tools = match client.list_tools(tool_timeout).await {
            Ok(tools) => tools,
            Err(err) => {
                tracing::warn!(server = %server_name, error = %err, "failed to list MCP tools");
                continue;
            }
        };
        let client = Arc::new(Mutex::new(client));
        for tool in tools {
            let wrapped_name = format!("mcp_{server_name}_{}", tool.name);
            if !valid_tool_name(&wrapped_name) {
                tracing::warn!(
                    server = %server_name,
                    tool = %tool.name,
                    wrapped = %wrapped_name,
                    "skipping MCP tool with invalid provider function name"
                );
                continue;
            }
            if !tool_enabled(server, &tool.name, &wrapped_name) {
                continue;
            }
            registry.register(Arc::new(McpToolWrapper::new(
                server_name,
                tool,
                Arc::clone(&client),
                tool_timeout,
            )));
        }
    }
}

/// Resolve the `command` field of a stdio MCP server entry.
///
/// Most values are passed through verbatim and ultimately fed to
/// `Command::new`, which performs PATH lookup for bare names and
/// uses the literal argument for absolute/relative paths.
///
/// The single sentinel value `"self"` is special-cased: it expands
/// to the absolute path of the *currently running* zunel binary
/// (via [`std::env::current_exe`]). This lets users wire the
/// built-in `zunel mcp serve --server slack|self` MCP servers into
/// their config without hardcoding a Homebrew/cargo/install prefix:
///
/// ```json
/// {
///   "tools": {
///     "mcpServers": {
///       "slack_me": {
///         "type": "stdio",
///         "command": "self",
///         "args": ["mcp", "serve", "--server", "slack"]
///       }
///     }
///   }
/// }
/// ```
///
/// The motivating environment is `brew services start zunel`
/// (macOS launchd): brew's mxcl plist does not propagate
/// `/opt/homebrew/bin` to the gateway's `PATH`, so a bare
/// `"command": "zunel"` would fail to spawn. Resolving via
/// `current_exe()` is prefix-agnostic and works for cargo
/// installs, .deb installs, and direct binary drops as well.
fn resolve_stdio_command(command: &str) -> std::io::Result<String> {
    if command != "self" {
        return Ok(command.to_string());
    }
    let exe = std::env::current_exe()?;
    exe.to_str().map(ToOwned::to_owned).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("current_exe path is not valid UTF-8: {}", exe.display()),
        )
    })
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

enum McpTransport {
    Stdio,
    Remote(RemoteTransport),
    Unsupported(String),
}

fn mcp_transport(server: &McpServerConfig) -> McpTransport {
    let raw = server.transport_type.as_deref().unwrap_or_else(|| {
        if server.url.is_some() {
            "streamableHttp"
        } else {
            "stdio"
        }
    });
    match raw.to_ascii_lowercase().as_str() {
        "stdio" => McpTransport::Stdio,
        "streamablehttp" | "http" => McpTransport::Remote(RemoteTransport::StreamableHttp),
        "sse" => McpTransport::Remote(RemoteTransport::Sse),
        other => McpTransport::Unsupported(other.to_string()),
    }
}

fn tool_enabled(server: &McpServerConfig, raw_name: &str, wrapped_name: &str) -> bool {
    let Some(enabled) = &server.enabled_tools else {
        return true;
    };
    enabled
        .iter()
        .any(|name| name == "*" || name == raw_name || name == wrapped_name)
}

async fn remote_headers_with_cached_oauth(
    server_name: &str,
    server: &McpServerConfig,
) -> BTreeMap<String, String> {
    let raw = server.headers.clone().unwrap_or_default();
    let mut headers = expand_header_envs(server_name, raw, &|name| std::env::var(name).ok());
    if headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("authorization"))
    {
        // Operator-supplied `Authorization` header wins; don't
        // even read the cached OAuth token (and definitely don't
        // try to refresh it).
        return headers;
    }
    let Ok(home) = zunel_config::zunel_home() else {
        return headers;
    };
    log_oauth_refresh_outcome(
        server_name,
        mcp_oauth_refresh::refresh_if_needed(&home, server_name).await,
    );
    apply_cached_oauth_header(&mut headers, &home, server_name);
    headers
}

/// Surface the [`mcp_oauth_refresh::Outcome`] in `tracing` so the
/// nightly "the Atlassian token went stale and nothing's wired up
/// to Jira anymore" failure mode is visible at info/warn instead
/// of being inferred from a downstream 401 buried in
/// `gateway.err.log`.
fn log_oauth_refresh_outcome(server_name: &str, outcome: mcp_oauth_refresh::Outcome) {
    use mcp_oauth_refresh::Outcome::*;
    match outcome {
        NotCached | NoExpiry => {}
        StillFresh { secs_remaining } => {
            tracing::debug!(
                server = server_name,
                secs_remaining,
                "MCP OAuth token still fresh"
            );
        }
        Refreshed { new_expires_in } => {
            tracing::info!(
                server = server_name,
                new_expires_in,
                "refreshed MCP OAuth access token via refresh_token grant"
            );
        }
        NoRefreshToken => {
            tracing::warn!(
                server = server_name,
                "MCP OAuth access token expired and no refresh_token is cached; \
                 run `zunel mcp login {server_name} --force` to re-authenticate",
                server_name = server_name,
            );
        }
        NoTokenUrl => {
            tracing::warn!(
                server = server_name,
                "MCP OAuth token cache is missing the tokenUrl needed to refresh; \
                 run `zunel mcp login {server_name} --force`",
                server_name = server_name,
            );
        }
        RefreshFailed(err) => {
            tracing::warn!(
                server = server_name,
                error = %err,
                "MCP OAuth refresh attempt failed; continuing with cached (likely-stale) token",
            );
        }
    }
}

/// Walk the configured `headers` map and substitute `${VAR}` /
/// `${VAR:-default}` placeholders against `lookup` (which is just
/// `std::env::var` in production but stubbable from tests). Any
/// header whose value references an unset variable with no default
/// is dropped so we never put the literal `${...}` token onto the
/// wire — operators rely on this so they can keep secrets out of
/// `config.json` and the dotenv-style `${X:-fallback}` form lets
/// them ship safe defaults for non-secret values.
fn expand_header_envs(
    server: &str,
    headers: BTreeMap<String, String>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .filter_map(|(key, value)| {
            expand_env_placeholders(server, &key, &value, lookup).map(|expanded| (key, expanded))
        })
        .collect()
}

/// Expand a single header value. Returns `None` (after logging at
/// `warn`) when an unset variable was referenced without a default,
/// when a `${` block was unterminated, or when the variable name was
/// not a valid POSIX-style identifier. Returning `None` instructs
/// the caller to drop the header entirely.
fn expand_env_placeholders(
    server: &str,
    header: &str,
    raw: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            // Fast-path: copy the run of non-`$` bytes in one go.
            // Safe to slice on a byte boundary because `$` is ASCII
            // and never falls inside a UTF-8 multibyte sequence.
            let next = bytes[i..]
                .iter()
                .position(|&b| b == b'$')
                .map(|p| i + p)
                .unwrap_or(bytes.len());
            out.push_str(&raw[i..next]);
            i = next;
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'$') => {
                // `$$` escapes a literal `$` so users can write a real
                // dollar sign without a placeholder being inferred.
                out.push('$');
                i += 2;
            }
            Some(b'{') => {
                let Some(close_rel) = bytes[i + 2..].iter().position(|&b| b == b'}') else {
                    tracing::warn!(
                        server,
                        header,
                        "unterminated `${{` in header value; dropping header"
                    );
                    return None;
                };
                let close = i + 2 + close_rel;
                let inside = &raw[i + 2..close];
                let (var_name, default) = match inside.split_once(":-") {
                    Some((name, default)) => (name.trim(), Some(default)),
                    None => (inside.trim(), None),
                };
                if !valid_env_var_name(var_name) {
                    tracing::warn!(
                        server,
                        header,
                        env_var = var_name,
                        "invalid env var name in header value; dropping header"
                    );
                    return None;
                }
                match lookup(var_name) {
                    Some(value) if !value.is_empty() => out.push_str(&value),
                    _ => match default {
                        Some(default) => out.push_str(default),
                        None => {
                            tracing::warn!(
                                server,
                                header,
                                env_var = var_name,
                                "environment variable referenced in header value is unset; \
                                 dropping header. Use ${{VAR:-default}} to provide a fallback."
                            );
                            return None;
                        }
                    },
                }
                i = close + 1;
            }
            _ => {
                // A bare `$` not followed by `{` or `$` is left as-is
                // for forward compatibility (e.g. JWTs that contain
                // `$argon2id$` literals).
                out.push('$');
                i += 1;
            }
        }
    }
    Some(out)
}

fn valid_env_var_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn apply_cached_oauth_header(
    headers: &mut BTreeMap<String, String>,
    home: &Path,
    server_name: &str,
) {
    match zunel_config::mcp_oauth::load_token(home, server_name) {
        Ok(Some(token)) => {
            headers.insert("Authorization".to_string(), token.authorization_header());
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                server = server_name,
                error = %err,
                "ignoring invalid MCP OAuth token cache"
            );
        }
    }
}

fn build_search_provider(cfg: &WebToolsConfig) -> Box<dyn WebSearchProvider> {
    match cfg.search_provider.as_str() {
        "brave" => {
            let key = cfg.brave_api_key.clone().unwrap_or_default();
            Box::new(BraveProvider::new(key))
        }
        "duckduckgo" | "ddg" => Box::new(DuckDuckGoProvider::new()),
        // Empty string + anything unknown collapses to a stub provider
        // that returns a clear "unimplemented" error at call time.
        _ => Box::new(StubProvider {
            provider_name: "stub",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zunel_config::mcp_oauth::{save_token, CachedMcpOAuthToken};

    fn full_token(access_token: &str, token_type: Option<&str>) -> CachedMcpOAuthToken {
        CachedMcpOAuthToken {
            access_token: access_token.into(),
            token_type: token_type.map(ToOwned::to_owned),
            refresh_token: None,
            expires_in: None,
            scope: None,
            obtained_at: 0,
            client_id: "cid".into(),
            client_secret: None,
            authorization_url: "https://example.test/authorize".into(),
            token_url: "https://example.test/token".into(),
        }
    }

    #[test]
    fn resolve_stdio_command_passes_through_non_sentinel_values() {
        assert_eq!(resolve_stdio_command("zunel").unwrap(), "zunel".to_string());
        assert_eq!(
            resolve_stdio_command("/opt/homebrew/bin/zunel").unwrap(),
            "/opt/homebrew/bin/zunel".to_string()
        );
        // Empty string is not the sentinel and is returned as-is so the
        // downstream warning surfaces the real misconfiguration instead
        // of getting swallowed by current_exe() success.
        assert_eq!(resolve_stdio_command("").unwrap(), "".to_string());
    }

    #[test]
    fn resolve_stdio_command_self_sentinel_expands_to_current_exe() {
        let resolved = resolve_stdio_command("self").expect("current_exe must succeed in tests");
        let expected = std::env::current_exe()
            .expect("current_exe in tests")
            .to_string_lossy()
            .into_owned();
        assert_eq!(resolved, expected);
        assert!(
            std::path::Path::new(&resolved).is_absolute(),
            "expected absolute path from current_exe, got {resolved}"
        );
    }

    #[test]
    fn cached_mcp_oauth_token_adds_authorization_header() {
        let home = tempfile::tempdir().unwrap();
        save_token(
            home.path(),
            "remote",
            &full_token("token-1", Some("bearer")),
        )
        .unwrap();

        let mut headers = BTreeMap::new();
        apply_cached_oauth_header(&mut headers, home.path(), "remote");

        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer token-1")
        );
    }

    #[test]
    fn missing_token_cache_leaves_headers_untouched() {
        let home = tempfile::tempdir().unwrap();
        let mut headers = BTreeMap::new();
        apply_cached_oauth_header(&mut headers, home.path(), "never-logged-in");
        assert!(headers.is_empty());
    }

    fn lookup_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn expand_substitutes_simple_var() {
        let lookup = lookup_from(&[("API_KEY", "supersecret")]);
        assert_eq!(
            expand_env_placeholders("self", "Authorization", "Bearer ${API_KEY}", &lookup),
            Some("Bearer supersecret".to_string())
        );
    }

    #[test]
    fn expand_chains_multiple_vars_and_literals() {
        let lookup = lookup_from(&[("USER", "ada"), ("ORG", "tunnel")]);
        assert_eq!(
            expand_env_placeholders(
                "self",
                "X-Trace",
                "user=${USER};org=${ORG};static=ok",
                &lookup
            ),
            Some("user=ada;org=tunnel;static=ok".to_string())
        );
    }

    #[test]
    fn expand_drops_header_when_var_missing_and_no_default() {
        let lookup = lookup_from(&[]);
        assert_eq!(
            expand_env_placeholders("self", "Authorization", "Bearer ${MISSING}", &lookup),
            None,
        );
    }

    #[test]
    fn expand_uses_default_when_var_missing() {
        let lookup = lookup_from(&[]);
        assert_eq!(
            expand_env_placeholders(
                "self",
                "Authorization",
                "Bearer ${MISSING:-fallback-token}",
                &lookup,
            ),
            Some("Bearer fallback-token".to_string())
        );
    }

    #[test]
    fn expand_uses_default_when_var_empty() {
        let lookup = lookup_from(&[("EMPTY", "")]);
        assert_eq!(
            expand_env_placeholders(
                "self",
                "Authorization",
                "Bearer ${EMPTY:-fallback}",
                &lookup,
            ),
            Some("Bearer fallback".to_string())
        );
    }

    #[test]
    fn expand_treats_double_dollar_as_literal() {
        let lookup = lookup_from(&[("X", "should-not-appear")]);
        assert_eq!(
            expand_env_placeholders("self", "X-Hash", "$$X $${X} $$$$", &lookup),
            Some("$X ${X} $$".to_string())
        );
    }

    #[test]
    fn expand_passes_bare_dollar_through() {
        // Argon2/PHC strings begin with `$` and must survive untouched.
        let lookup = lookup_from(&[]);
        assert_eq!(
            expand_env_placeholders(
                "self",
                "X-Hash",
                "$argon2id$v=19$m=65536,t=3,p=4$abc$def",
                &lookup
            ),
            Some("$argon2id$v=19$m=65536,t=3,p=4$abc$def".to_string())
        );
    }

    #[test]
    fn expand_drops_header_on_unterminated_brace() {
        let lookup = lookup_from(&[("API_KEY", "x")]);
        assert_eq!(
            expand_env_placeholders("self", "Authorization", "Bearer ${API_KEY", &lookup),
            None,
        );
    }

    #[test]
    fn expand_rejects_invalid_var_name() {
        let lookup = lookup_from(&[("9NOPE", "x")]);
        // Identifiers can't start with a digit.
        assert_eq!(
            expand_env_placeholders("self", "Authorization", "Bearer ${9NOPE}", &lookup),
            None,
        );
    }

    #[test]
    fn expand_default_value_is_taken_literally_including_dollars() {
        let lookup = lookup_from(&[]);
        assert_eq!(
            expand_env_placeholders(
                "self",
                "X-Default",
                "${MISSING:-some$weird:-value}",
                &lookup
            ),
            Some("some$weird:-value".to_string())
        );
    }

    #[test]
    fn expand_header_envs_drops_only_failing_entries() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "Bearer ${API_KEY}".into());
        headers.insert("X-Static".into(), "fixed".into());
        headers.insert("X-Missing".into(), "v=${NOPE}".into());

        let lookup = lookup_from(&[("API_KEY", "abc")]);
        let expanded = expand_header_envs("self", headers, &lookup);

        assert_eq!(expanded.len(), 2);
        assert_eq!(
            expanded.get("Authorization").map(String::as_str),
            Some("Bearer abc")
        );
        assert_eq!(expanded.get("X-Static").map(String::as_str), Some("fixed"));
        assert!(!expanded.contains_key("X-Missing"));
    }
}
