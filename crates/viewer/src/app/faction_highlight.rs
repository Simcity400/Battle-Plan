
use std::sync::{Arc, mpsc};
use viewer_runtime::retirement::Dropper;
pub(crate) use viewer_playback::highlight::{Over, Picture};

use viewer_foundation::perspective;
use viewer_world::scene::FactionHighlight;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Key {
    pub branch: replay_data::BranchId,
    pub hour: replay_data::Hour,
    pub perspective: perspective::Choice,
    pub borders: u64,
    pub regions: Arc<[u32]>,
    pub adjacency: bool,
    pub selection: Option<super::ground_click::Outline>,
}

pub(super) fn wanted(hovered: &[u32], selection: Option<&super::ground_click::Outline>, shown: bool) -> bool {
    !hovered.is_empty() || selection.is_some() || shown
}

pub(super) struct Prepared {
    pub plan: FactionHighlight,
    pub restore: Option<FactionHighlight>,
}

pub(super) struct Lane<T = FactionHighlight> {
    requested: Option<Key>,
    installed: bool,
    pending: Option<mpsc::Receiver<(Key, T)>>,
    restore: Option<FactionHighlight>,
}

impl<T> Default for Lane<T> {
    fn default() -> Self {
        Self { requested: None, installed: false, pending: None, restore: None }
    }
}

impl<T: Send + 'static> Lane<T> {
    pub fn requested(&self) -> Option<&Key> {
        self.requested.as_ref()
    }

    pub fn installed(&mut self) {
        self.installed = true;
    }

    pub fn set_restore(&mut self, restore: Option<FactionHighlight>) {
        self.restore = restore;
    }

    pub fn take_restore(&mut self) -> Option<FactionHighlight> {
        self.restore.take()
    }

    #[cfg(test)]
    pub fn update(
        &mut self,
        key: Key,
        submit: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
        build: impl FnOnce() -> T + Send + 'static,
        dropper: &Dropper,
    ) -> Option<T> {
        self.update_with(key, submit, || build, dropper)
    }

    pub fn update_with<B: FnOnce() -> T + Send + 'static>(
        &mut self,
        key: Key,
        submit: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
        prepare: impl FnOnce() -> B,
        dropper: &Dropper,
    ) -> Option<T> {
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
        if self.installed || (key.regions.is_empty() && key.selection.is_none()) {
            return None;
        }
        let build = prepare();
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
