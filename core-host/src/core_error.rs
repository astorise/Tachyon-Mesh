use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum CoreError {
    #[error("shared state lock `{name}` is poisoned")]
    PoisonedLock { name: &'static str },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

pub(crate) type CoreResult<T> = std::result::Result<T, CoreError>;

pub(crate) fn poisoned_lock(name: &'static str) -> CoreError {
    CoreError::PoisonedLock { name }
}
