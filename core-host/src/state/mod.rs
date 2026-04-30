#![allow(dead_code)]

use crate::core_error::{poisoned_lock, CoreResult};
use std::sync::{Mutex, MutexGuard};

pub(crate) fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    name: &'static str,
) -> CoreResult<MutexGuard<'a, T>> {
    mutex.lock().map_err(|_| poisoned_lock(name))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeGenerationState {
    Active,
    Draining,
}
