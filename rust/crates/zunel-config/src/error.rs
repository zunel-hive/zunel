use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config file not found: {0}")]
    NotFound(PathBuf),

    #[error("failed to read config at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("no provider configured for model {0}")]
    MissingProvider(String),

    #[error("provider {0} missing apiKey")]
    MissingApiKey(String),
}

pub type Result<T> = std::result::Result<T, Error>;
