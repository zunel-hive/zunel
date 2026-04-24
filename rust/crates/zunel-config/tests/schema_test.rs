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
fn unknown_fields_ignored() {
    let json = r#"{
        "providers": {},
        "agents": { "defaults": { "model": "m" } },
        "extraTopLevelThing": { "nested": true }
    }"#;
    let cfg: zunel_config::Config = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.agents.defaults.model, "m");
}
