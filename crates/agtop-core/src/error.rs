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

    #[error("no pricing known for model '{model}' (client: {client})")]
    UnknownPricing { client: String, model: String },

    #[error("no token usage data found for session '{0}'")]
    NoUsage(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn unknown_pricing_error_uses_client_vocabulary() {
        let err = Error::UnknownPricing {
            client: "claude".into(),
            model: "claude-sonnet".into(),
        };

        assert_eq!(
            err.to_string(),
            "no pricing known for model 'claude-sonnet' (client: claude)"
        );
    }
}
