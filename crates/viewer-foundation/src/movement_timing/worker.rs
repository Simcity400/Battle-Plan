use super::Etas;
use std::sync::{Arc, Mutex, OnceLock, TryLockError};

/// Success-only ETA sharing for one ticket's immutable overlay and movers inputs.
#[derive(Default)]
pub struct WorkerEtas {
    ready: OnceLock<Arc<Etas>>,
    initializing: Mutex<()>,
    #[cfg(test)]
    waiting: OnceLock<std::sync::mpsc::SyncSender<()>>,
}

impl WorkerEtas {
    /// The synchronous initializer must not wait for siblings or locks they retain.
    pub fn get_or_try_init<E>(
        &self,
        build: impl FnOnce() -> Result<Arc<Etas>, E>,
    ) -> Result<Arc<Etas>, E> {
        if let Some(ready) = self.ready.get() {
            return Ok(Arc::clone(ready));
        }
        let _guard = match self.initializing.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => {
                #[cfg(test)]
                if let Some(waiting) = self.waiting.get() {
                    let _ = waiting.try_send(());
                }
                self.initializing
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            }
        };
        if let Some(ready) = self.ready.get() {
            return Ok(Arc::clone(ready));
        }
        // Errors publish nothing; panics unwind the guard and later callers retry.
        let value = build()?;
        Ok(Arc::clone(self.ready.get_or_init(|| value)))
    }
}

#[cfg(test)]
mod tests;
