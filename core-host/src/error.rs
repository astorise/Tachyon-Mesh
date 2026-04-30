#![allow(dead_code)]

use thiserror::Error;

// Shared error surface for incremental migration away from ad-hoc panics.
#[derive(Debug, Error)]
pub(crate) enum TachyonError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("wasm execution failed: {0}")]
    Wasm(String),
    #[error("missing expected header: {0}")]
    MissingHeader(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

pub(crate) type TachyonResult<T> = std::result::Result<T, TachyonError>;
