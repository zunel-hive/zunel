#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("provider error: {0}")]
    Provider(#[from] zunel_providers::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
