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

    #[error("Invalid profile name {0:?}: must not contain whitespace, path separators, or '..'.")]
    InvalidProfileName(String),

    /// The configured workspace is one of a small set of paths that
    /// would let zunel's filesystem tools mutate state outside the
    /// "zunel sandbox" — `/`, `$HOME`, an ancestor of `$ZUNEL_HOME`,
    /// or `$ZUNEL_HOME` itself. Operators who genuinely need this
    /// (e.g. a one-off script targeting `/tmp` that happens to be a
    /// home subdir) can opt out with `ZUNEL_ALLOW_UNSAFE_WORKSPACE=1`
    /// or the `zunel --i-know-what-im-doing` global flag.
    #[error(
        "refusing to start with workspace {path:?}: {reason}. \
         Set ZUNEL_ALLOW_UNSAFE_WORKSPACE=1 or pass `zunel \
         --i-know-what-im-doing` if you really mean it."
    )]
    UnsafeWorkspace { path: PathBuf, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;
