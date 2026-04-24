use std::path::PathBuf;

#[test]
fn parses_minimal_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    let cfg: zunel_config::Config = serde_json::from_str(&raw).unwrap();

    let custom = cfg
        .providers
        .custom
        .as_ref()
        .expect("custom provider present");
    assert_eq!(custom.api_key, "sk-test");
    assert_eq!(custom.api_base, "https://api.openai.com/v1");
    assert_eq!(
        custom
            .extra_headers
            .as_ref()
            .unwrap()
            .get("X-Demo")
            .unwrap(),
        "1"
    );

    let d = &cfg.agents.defaults;
    assert_eq!(d.provider.as_deref(), Some("custom"));
    assert_eq!(d.model, "gpt-4o-mini");
    assert_eq!(d.temperature, Some(0.2));
    assert_eq!(d.max_tokens, Some(1024));
}

#[test]
fn tools_section_defaults_when_absent() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    assert!(!cfg.tools.exec.enable, "exec must be opt-in");
    assert!(!cfg.tools.web.enable, "web must be opt-in");
    assert!(
        cfg.tools.web.search_provider.is_empty(),
        "search_provider defaults to unset"
    );
    assert!(
        !cfg.tools.approval_required,
        "approval_required defaults off"
    );
    assert!(cfg.tools.filesystem.media_dir.is_none());
}

#[test]
fn tools_section_round_trips() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } },
        "tools": {
            "approval_required": true,
            "approval_scope": "shell",
            "exec": {
                "enable": true,
                "default_timeout_secs": 30,
                "max_timeout_secs": 600
            },
            "web": {
                "enable": true,
                "search_provider": "brave",
                "brave_api_key": "k"
            },
            "filesystem": {
                "media_dir": "/tmp/media"
            }
        }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    assert!(cfg.tools.exec.enable);
    assert_eq!(cfg.tools.exec.default_timeout_secs, 30);
    assert_eq!(cfg.tools.exec.max_timeout_secs, 600);
    assert!(cfg.tools.web.enable);
    assert_eq!(cfg.tools.web.search_provider, "brave");
    assert_eq!(cfg.tools.web.brave_api_key.as_deref(), Some("k"));
    assert_eq!(
        cfg.tools.filesystem.media_dir.as_deref(),
        Some(std::path::Path::new("/tmp/media"))
    );
    assert!(cfg.tools.approval_required);
    assert_eq!(cfg.tools.approval_scope, "shell");
}

#[test]
fn mcp_servers_section_round_trips_python_shape() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } },
        "tools": {
            "mcpServers": {
                "self": {
                    "type": "stdio",
                    "command": "python",
                    "args": ["-m", "zunel.mcp.zunel_self"],
                    "env": { "A": "B" },
                    "headers": { "Authorization": "Bearer ${TOKEN}" },
                    "toolTimeout": 30,
                    "initTimeout": 10,
                    "enabledTools": ["sessions", "mcp_self_status"],
                    "oauth": {
                        "enabled": true,
                        "clientId": "client-1",
                        "clientSecret": "secret-1",
                        "authorizationUrl": "https://auth.example/authorize",
                        "tokenUrl": "https://auth.example/token",
                        "scope": "read write"
                    }
                }
            }
        }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    let server = cfg.tools.mcp_servers.get("self").unwrap();
    assert_eq!(server.transport_type.as_deref(), Some("stdio"));
    assert_eq!(server.command.as_deref(), Some("python"));
    assert_eq!(
        server.args.as_deref(),
        Some(&["-m".to_string(), "zunel.mcp.zunel_self".to_string()][..])
    );
    assert_eq!(server.env.as_ref().unwrap().get("A").unwrap(), "B");
    assert_eq!(
        server
            .headers
            .as_ref()
            .unwrap()
            .get("Authorization")
            .unwrap(),
        "Bearer ${TOKEN}"
    );
    assert_eq!(server.tool_timeout, Some(30));
    assert_eq!(server.init_timeout, Some(10));
    assert_eq!(
        server.enabled_tools.as_deref(),
        Some(&["sessions".to_string(), "mcp_self_status".to_string()][..])
    );
    let oauth = server.oauth.as_ref().unwrap();
    assert!(oauth.enabled);
    assert_eq!(oauth.client_id.as_deref(), Some("client-1"));
    assert_eq!(oauth.client_secret.as_deref(), Some("secret-1"));
    assert_eq!(
        oauth.authorization_url.as_deref(),
        Some("https://auth.example/authorize")
    );
    assert_eq!(
        oauth.token_url.as_deref(),
        Some("https://auth.example/token")
    );
    assert_eq!(oauth.scope.as_deref(), Some("read write"));

    let value = serde_json::to_value(&cfg).unwrap();
    assert!(value["tools"]["mcpServers"]["self"].is_object());
    assert_eq!(value["tools"]["mcpServers"]["self"]["toolTimeout"], 30);
    assert_eq!(value["tools"]["mcpServers"]["self"]["initTimeout"], 10);
}

#[test]
fn unknown_fields_ignored() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } },
        "extraTopLevelThing": { "nested": true }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.agents.defaults.model, "m");
}
