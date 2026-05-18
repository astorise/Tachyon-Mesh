#[cfg(feature = "experimental")]
use thiserror::Error;

// Future error infrastructure — kept as scaffolding for the v1.2 typed-error
// migration. Gated behind `experimental` so the default profile remains free
// of placebo annotations.

#[cfg(feature = "experimental")]
#[derive(Debug, Error)]
pub(crate) enum CoreError {
    #[error("shared state lock `{name}` is poisoned")]
    PoisonedLock { name: &'static str },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

#[cfg(feature = "experimental")]
pub(crate) type CoreResult<T> = std::result::Result<T, CoreError>;

#[cfg(feature = "experimental")]
pub(crate) fn poisoned_lock(name: &'static str) -> CoreError {
    CoreError::PoisonedLock { name }
}
