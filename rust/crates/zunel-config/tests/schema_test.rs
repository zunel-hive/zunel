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
fn unknown_fields_ignored() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } },
        "extraTopLevelThing": { "nested": true }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.agents.defaults.model, "m");
}
