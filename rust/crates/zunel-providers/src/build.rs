use std::sync::Arc;

use zunel_config::Config;

use crate::base::LLMProvider;
use crate::error::{Error, Result};
use crate::openai_compat::OpenAICompatProvider;

/// Build the concrete provider selected by `agents.defaults.provider`.
pub fn build_provider(config: &Config) -> Result<Arc<dyn LLMProvider>> {
    let name = config
        .agents
        .defaults
        .provider
        .as_deref()
        .unwrap_or("custom")
        .to_ascii_lowercase();

    match name.as_str() {
        "custom" | "openai" | "openai_compat" => {
            let custom = config.providers.custom.as_ref().ok_or_else(|| {
                Error::Config(
                    "providers.custom is required when agents.defaults.provider = custom".into(),
                )
            })?;
            let headers = custom.extra_headers.clone().unwrap_or_default();
            let provider = OpenAICompatProvider::new(
                custom.api_key.clone(),
                custom.api_base.clone(),
                headers,
            )?;
            Ok(Arc::new(provider))
        }
        "codex" => Err(Error::Config(
            "codex provider lands in slice 4; use 'custom' for slice 1".into(),
        )),
        other => Err(Error::Config(format!("unknown provider '{other}'"))),
    }
}
