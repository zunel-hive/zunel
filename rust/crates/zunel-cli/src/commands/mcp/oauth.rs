//! `zunel mcp login`: PKCE-based OAuth flow for remote MCP servers.
//!
//! High-level shape of [`login`]:
//!
//! 1. Resolve `redirect_uri` and any preconfigured client credentials.
//! 2. Discover authorization + token endpoints (RFC 9728 → `WWW-Authenticate`
//!    `resource_metadata` → `.well-known/oauth-authorization-server`).
//! 3. Dynamic-client-register if no `client_id` is configured.
//! 4. Spawn the local callback server (or fall back to manual paste) and open
//!    the authorize URL.
//! 5. Exchange the authorization code for an access token, then persist it to
//!    `~/.zunel/mcp-oauth/<server>/token.json`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::Engine;
use reqwest::header::WWW_AUTHENTICATE;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use zunel_config::mcp_oauth::{
    mcp_oauth_token_path as shared_token_path, save_token, CachedMcpOAuthToken,
};
use zunel_config::{McpOAuthConfig, McpServerConfig};

use crate::cli::McpLoginArgs;
use crate::oauth_callback::{bind_callback_server, open_browser};

pub(super) async fn login(args: McpLoginArgs, config_path: Option<&Path>) -> Result<()> {
    let cfg = zunel_config::load_config(config_path).context("loading config")?;
    let server = cfg
        .tools
        .mcp_servers
        .get(&args.server)
        .with_context(|| format!("unknown MCP server '{}'", args.server))?;
    if server.url.is_none() {
        bail!("MCP server '{}' is not a remote server", args.server);
    }

    let token_path = mcp_oauth_token_path(&args.server)?;
    if token_path.exists() && !args.force {
        println!(
            "MCP server '{}' already has a cached token at {}. Use --force to re-authenticate.",
            args.server,
            token_path.display()
        );
        println!(
            "Note: zunel will auto-refresh expired access tokens via the refresh_token grant; \
             --force is only needed if you've actually been signed out at the IdP."
        );
        return Ok(());
    }

    let client = reqwest::Client::new();
    let oauth = server.normalized_oauth().unwrap_or_default();
    if matches!(server.normalized_oauth(), Some(ref oauth) if !oauth.enabled) {
        bail!("MCP server '{}' has OAuth disabled", args.server);
    }
    let metadata = discover_oauth_metadata(&client, server, &oauth).await?;
    let redirect_uri = redirect_uri(&oauth);
    let mut client_credentials = client_credentials(&oauth);
    if client_credentials.client_id.is_empty() {
        client_credentials = register_client(&client, &metadata, &redirect_uri)
            .await
            .context("registering OAuth client")?;
    }

    let state = args.state.unwrap_or_else(generate_oauth_token);
    let verifier = generate_oauth_token();
    let challenge = pkce_challenge(&verifier);
    let callback_server = if args.url_in.is_none() {
        bind_callback_server(&redirect_uri).await?
    } else {
        None
    };
    let authorization_url = authorization_url(
        &metadata,
        server,
        &oauth,
        &client_credentials.client_id,
        &redirect_uri,
        &state,
        &challenge,
    )?;

    println!(
        "Open this URL in your browser to authenticate '{}':",
        args.server
    );
    println!("{authorization_url}");

    let callback_url = match args.url_in {
        Some(url) => url,
        None if callback_server.is_some() => {
            let server = callback_server.expect("checked is_some");
            open_browser(&authorization_url);
            println!("Waiting for OAuth callback on {redirect_uri}...");
            server.wait_for_callback().await?
        }
        None => {
            println!("Paste the full callback URL here:");
            read_stdin_line().await?
        }
    };
    let code = authorization_code_from_callback(&callback_url, &state)?;
    let token = exchange_code_for_token(
        &client,
        &metadata,
        server,
        &client_credentials,
        &redirect_uri,
        &verifier,
        &code,
    )
    .await
    .context("exchanging authorization code for token")?;

    let home = zunel_config::zunel_home().context("resolving zunel home directory")?;
    save_token(&home, &args.server, &token)
        .with_context(|| format!("writing {}", token_path.display()))?;
    println!(
        "Cached OAuth token for '{}' at {}.",
        args.server,
        token_path.display()
    );
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ResourceMetadata {
    #[serde(alias = "authorizationServers")]
    authorization_servers: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AuthorizationMetadata {
    #[serde(alias = "authorizationUrl")]
    authorization_endpoint: String,
    #[serde(alias = "tokenUrl")]
    token_endpoint: String,
    registration_endpoint: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ClientCredentials {
    client_id: String,
    client_secret: Option<String>,
}

async fn discover_oauth_metadata(
    client: &reqwest::Client,
    server: &McpServerConfig,
    oauth: &McpOAuthConfig,
) -> Result<AuthorizationMetadata> {
    if let (Some(authorization_endpoint), Some(token_endpoint)) =
        (&oauth.authorization_url, &oauth.token_url)
    {
        return Ok(AuthorizationMetadata {
            authorization_endpoint: authorization_endpoint.clone(),
            token_endpoint: token_endpoint.clone(),
            registration_endpoint: None,
        });
    }

    if let Some(resource_metadata_url) = protected_resource_metadata_url(client, server).await? {
        let resource_metadata: ResourceMetadata = client
            .get(&resource_metadata_url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .with_context(|| format!("reading resource metadata from {resource_metadata_url}"))?;
        if let Some(issuer) = resource_metadata.authorization_servers.first() {
            return fetch_authorization_metadata(client, issuer).await;
        }
    }

    let server_url = server
        .url
        .as_deref()
        .context("remote MCP server missing url")?;
    let url = reqwest::Url::parse(server_url).context("parsing MCP server URL")?;
    let issuer = format!(
        "{}://{}",
        url.scheme(),
        url.host_str().context("MCP server URL missing host")?
    );
    fetch_authorization_metadata(client, &issuer).await
}

async fn protected_resource_metadata_url(
    client: &reqwest::Client,
    server: &McpServerConfig,
) -> Result<Option<String>> {
    let Some(server_url) = server.url.as_deref() else {
        return Ok(None);
    };
    let response = client.get(server_url).send().await?;
    let Some(value) = response.headers().get(WWW_AUTHENTICATE) else {
        return Ok(None);
    };
    let value = value.to_str().context("invalid WWW-Authenticate header")?;
    Ok(www_authenticate_param(value, "resource_metadata"))
}

async fn fetch_authorization_metadata(
    client: &reqwest::Client,
    issuer: &str,
) -> Result<AuthorizationMetadata> {
    let metadata_url = authorization_metadata_url(issuer)?;
    client
        .get(metadata_url.as_str())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("reading OAuth metadata from {metadata_url}"))
}

async fn register_client(
    client: &reqwest::Client,
    metadata: &AuthorizationMetadata,
    redirect_uri: &str,
) -> Result<ClientCredentials> {
    let endpoint = metadata.registration_endpoint.as_deref().context(
        "OAuth metadata did not include a registration_endpoint and no clientId is configured",
    )?;
    let response: Value = client
        .post(endpoint)
        .json(&json!({
            "client_name": "zunel",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let client_id = response
        .get("client_id")
        .and_then(Value::as_str)
        .context("dynamic client registration response missing client_id")?
        .to_string();
    let client_secret = response
        .get("client_secret")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Ok(ClientCredentials {
        client_id,
        client_secret,
    })
}

fn client_credentials(oauth: &McpOAuthConfig) -> ClientCredentials {
    ClientCredentials {
        client_id: oauth.client_id.clone().unwrap_or_default(),
        client_secret: oauth.client_secret.clone(),
    }
}

fn redirect_uri(oauth: &McpOAuthConfig) -> String {
    if let Some(uri) = &oauth.redirect_uri {
        return uri.clone();
    }
    let host = oauth
        .callback_host
        .clone()
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = oauth.callback_port.unwrap_or(33419);
    format!("http://{host}:{port}/callback")
}

fn authorization_url(
    metadata: &AuthorizationMetadata,
    server: &McpServerConfig,
    oauth: &McpOAuthConfig,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String> {
    let mut url =
        reqwest::Url::parse(&metadata.authorization_endpoint).context("parsing authorize URL")?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(scope) = &oauth.scope {
            query.append_pair("scope", scope);
        }
        if let Some(resource) = &server.url {
            query.append_pair("resource", resource);
        }
    }
    Ok(url.into())
}

async fn exchange_code_for_token(
    client: &reqwest::Client,
    metadata: &AuthorizationMetadata,
    server: &McpServerConfig,
    credentials: &ClientCredentials,
    redirect_uri: &str,
    verifier: &str,
    code: &str,
) -> Result<CachedMcpOAuthToken> {
    let mut form = BTreeMap::from([
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", credentials.client_id.clone()),
        ("code_verifier", verifier.to_string()),
    ]);
    if let Some(secret) = &credentials.client_secret {
        form.insert("client_secret", secret.clone());
    }
    if let Some(resource) = &server.url {
        form.insert("resource", resource.clone());
    }

    let response: Value = client
        .post(&metadata.token_endpoint)
        .form(&form)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let access_token = response
        .get("access_token")
        .and_then(Value::as_str)
        .context("token response missing access_token")?
        .to_string();
    Ok(CachedMcpOAuthToken {
        access_token,
        token_type: response
            .get("token_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        refresh_token: response
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        expires_in: response.get("expires_in").and_then(Value::as_u64),
        scope: response
            .get("scope")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        obtained_at: unix_timestamp(),
        client_id: credentials.client_id.clone(),
        client_secret: credentials.client_secret.clone(),
        authorization_url: metadata.authorization_endpoint.clone(),
        token_url: metadata.token_endpoint.clone(),
    })
}

fn authorization_code_from_callback(callback_url: &str, expected_state: &str) -> Result<String> {
    let url = reqwest::Url::parse(callback_url).context("parsing callback URL")?;
    if let Some(error) = url
        .query_pairs()
        .find_map(|(name, value)| (name == "error").then(|| value.into_owned()))
    {
        bail!("OAuth callback returned error: {error}");
    }
    let state = url
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .context("callback URL missing state")?;
    if state != expected_state {
        bail!("OAuth state mismatch");
    }
    url.query_pairs()
        .find_map(|(name, value)| (name == "code").then(|| value.into_owned()))
        .context("callback URL missing code")
}

async fn read_stdin_line() -> Result<String> {
    let mut line = String::new();
    let mut reader = BufReader::new(tokio::io::stdin());
    reader.read_line(&mut line).await?;
    Ok(line.trim().to_string())
}

fn mcp_oauth_token_path(server_name: &str) -> Result<std::path::PathBuf> {
    let home = zunel_config::zunel_home().context("resolving zunel home directory")?;
    Ok(shared_token_path(&home, server_name))
}

fn www_authenticate_param(header: &str, name: &str) -> Option<String> {
    for part in header.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        let key = key.trim().trim_start_matches("Bearer ").trim();
        if key == name {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn authorization_metadata_url(issuer: &str) -> Result<reqwest::Url> {
    let issuer = reqwest::Url::parse(issuer).context("parsing OAuth issuer URL")?;
    let path = issuer.path().trim_start_matches('/');
    let mut url = issuer.clone();
    let metadata_path = if path.is_empty() {
        "/.well-known/oauth-authorization-server".to_string()
    } else {
        format!("/.well-known/oauth-authorization-server/{path}")
    };
    url.set_path(&metadata_path);
    url.set_query(None);
    Ok(url)
}

fn generate_oauth_token() -> String {
    let mut bytes = [0_u8; 32];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        if file.read_exact(&mut bytes).is_ok() {
            return base64_url(&bytes);
        }
    }
    let fallback = format!("{}:{}", unix_timestamp(), std::process::id());
    base64_url(fallback.as_bytes())
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64_url(&digest)
}

fn base64_url(input: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
