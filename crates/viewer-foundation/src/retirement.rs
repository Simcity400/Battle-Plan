//! Bounded background retirement shared by decode and presentation workers.

use crate::worker_priority::background_priority;
use std::{io, sync::mpsc, thread};

/// Builders for the processors this process may use
/// ([`crate::parallel::usable_processors`]), not the machine's.
pub fn builder_count() -> usize {
    crate::parallel::usable_processors()
        .saturating_sub(8)
        .max(2)
}

/// Frees what the window let go of on a thread of its own, so a frame that
/// swaps in a new hour does not pay for returning the last one's memory.
///
/// The destructor runs at the builders' priority, so saturated builders can
/// starve it. At most one retirement per builder waits behind the one being
/// freed; past that the caller frees its retirement itself, spending that
/// frame's time rather than holding an unbounded backlog of hours.
pub struct Dropper {
    bin: Option<mpsc::SyncSender<Box<dyn Send>>>,
}

/// Everything one frame lets go of, handed to the [`Dropper`] as a single
/// retirement so its bound counts frames, not the pieces of an hour.
#[derive(Default)]
pub struct Retired(Vec<Box<dyn Send>>);

impl Retired {
    pub fn push<T: Send + 'static>(&mut self, value: T) {
        self.0.push(Box::new(value));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Dropper {
    pub fn new() -> io::Result<Self> {
        Self::with_capacity(builder_count())
    }

    fn with_capacity(capacity: usize) -> io::Result<Self> {
        let (bin, inbox) = mpsc::sync_channel::<Box<dyn Send>>(capacity);
        thread::Builder::new()
            .name("hour dropper".into())
            .spawn(move || {
                let _cost = replay_frame::cpu::scope("display.retire", Some("other"));
                background_priority();
                while inbox.recv().is_ok() {}
            })?;
        Ok(Self { bin: Some(bin) })
    }

    /// Frees `value` off the frame path, or here, without waiting, when the
    /// queue is full.
    pub fn drop_later<T: Send + 'static>(&self, value: T) {
        if let Some(bin) = &self.bin {
            // A full or closed queue hands the value back; it drops here.
            let _ = bin.try_send(Box::new(value));
        }
    }
}

#[cfg(test)]
mod tests;
