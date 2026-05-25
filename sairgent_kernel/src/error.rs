use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Encryption/Decryption error: {0}")]
    VaultError(String),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Agent cap exceeded: {0}")]
    AgentCapExceeded(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization/Deserialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Overloaded: {0}")]
    Overloaded(String),
}

pub type Result<T> = std::result::Result<T, KernelError>;
