//! Mutex access that survives poisoning.

use std::sync::{Mutex, MutexGuard};

/// Locks `mutex`, taking the guard even when another thread panicked while holding it.
/// The protected data is still consistent for the callers in this crate: each critical
/// section only mutates it in whole steps.
pub fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
