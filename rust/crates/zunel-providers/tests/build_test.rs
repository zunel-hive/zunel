use zunel_config::{AgentDefaults, AgentsConfig, Config, CustomProvider, ProvidersConfig};
use zunel_providers::build_provider;

fn config_with_custom() -> Config {
    Config {
        providers: ProvidersConfig {
            custom: Some(CustomProvider {
                api_key: "sk".into(),
                api_base: "https://x.test".into(),
                extra_headers: None,
            }),
            codex: None,
        },
        agents: AgentsConfig {
            defaults: AgentDefaults {
                provider: Some("custom".into()),
                model: "gpt-x".into(),
                temperature: None,
                max_tokens: None,
                reasoning_effort: None,
                workspace: None,
            },
        },
        tools: Default::default(),
    }
}

#[test]
fn builds_custom_provider_from_config() {
    let cfg = config_with_custom();
    let _provider = build_provider(&cfg).expect("builds");
}

#[test]
fn errors_when_codex_requested_in_slice_1() {
    let mut cfg = config_with_custom();
    cfg.agents.defaults.provider = Some("codex".into());
    cfg.providers.custom = None;
    let err = build_provider(&cfg).err().expect("expected Err");
    assert!(
        matches!(err, zunel_providers::Error::Config(ref m) if m.contains("codex")),
        "got {err:?}"
    );
}

#[test]
fn errors_when_no_provider_configured() {
    let cfg = Config::default();
    let err = build_provider(&cfg).err().expect("expected Err");
    assert!(matches!(err, zunel_providers::Error::Config(_)));
}
