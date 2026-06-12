use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("io error: {0}")]
    IoBare(#[from] std::io::Error),

    #[error("json error at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("json error: {0}")]
    JsonBare(#[from] serde_json::Error),

    #[error("corrupt event log at {path}: {reason}")]
    CorruptEventLog { path: PathBuf, reason: String },

    #[error("invalid schema_version {found} (supported: {supported:?}) at {path}")]
    UnsupportedSchemaVersion {
        path: PathBuf,
        found: u32,
        supported: Vec<u32>,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }
}
