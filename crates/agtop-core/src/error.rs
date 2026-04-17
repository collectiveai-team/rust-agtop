use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("walkdir error: {0}")]
    Walk(#[from] walkdir::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("no pricing known for model '{model}' (provider: {provider})")]
    UnknownPricing { provider: String, model: String },

    #[error("no token usage data found for session '{0}'")]
    NoUsage(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
