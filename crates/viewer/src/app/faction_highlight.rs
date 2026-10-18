//! Bounded faction-pin preparation on the existing hour builders.

use std::sync::{Arc, mpsc};
use viewer_foundation::retirement::Dropper;

use {viewer_foundation::map::perspective, viewer_world::map::scene::FactionHighlight};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Key {
    pub epoch: u64,
    pub branch: replay_data::BranchId,
    pub hour: replay_data::Hour,
    pub perspective: perspective::Choice,
    pub borders: u64,
    pub regions: Arc<[u32]>,
}

#[derive(Default)]
pub(super) struct Lane {
    requested: Option<Key>,
    installed: bool,
    pending: Option<mpsc::Receiver<(Key, FactionHighlight)>>,
}

impl Lane {
    pub fn requested(&self) -> Option<&Key> {
        self.requested.as_ref()
    }

    pub fn installed(&mut self) {
        self.installed = true;
    }

    /// Coalesce hover changes behind at most one submitted job. A refused
    /// admission retries on the next frame; stale results retire off frames.
    pub fn update(
        &mut self,
        key: Key,
        submit: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
        build: impl FnOnce() -> FactionHighlight + Send + 'static,
        dropper: &Dropper,
    ) -> Option<FactionHighlight> {
        if self.requested.as_ref() != Some(&key) {
            self.requested = Some(key.clone());
            self.installed = false;
        }
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok((built_for, plan)) => {
                    self.pending = None;
                    if built_for == key {
                        return Some(plan);
                    }
                    dropper.drop_later(plan);
                }
                Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
                Err(mpsc::TryRecvError::Empty) => return None,
            }
        }
        if self.installed || key.regions.is_empty() {
            return None;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        if submit(Box::new(move || {
            let _ = sender.send((key, build()));
        })) {
            self.pending = Some(receiver);
        }
        None
    }
}

#[cfg(test)]
mod tests;
