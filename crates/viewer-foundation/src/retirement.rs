//! Bounded background retirement shared by decode and presentation workers.

use job_priority::SpawnCarried as _;
use crate::worker_priority::normal_priority;
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
/// Handing a retirement over never waits and never frees here: a full queue
/// would make the frame thread free an hour, which is what this exists to
/// avoid. The destructor runs at normal priority, above every maker, so the
/// queue is drained whatever the builders are doing.
pub struct Dropper {
    bin: Option<mpsc::Sender<Box<dyn Send>>>,
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
        let (bin, inbox) = mpsc::channel::<Box<dyn Send>>();
        thread::Builder::new()
            .name("map retirement".into())
            .spawn_carried(move || {
                let _cost = replay_frame::cpu::scope("display.retire", Some("other"));
                normal_priority();
                while inbox.recv().is_ok() {}
            })?;
        Ok(Self { bin: Some(bin) })
    }

    /// Frees `value` off the frame path, never here and never waiting.
    pub fn drop_later<T: Send + 'static>(&self, value: T) {
        if let Some(bin) = &self.bin {
            // Only a dropper whose thread has stopped hands the value back; it drops here.
            let _ = bin.send(Box::new(value));
        }
    }
}

#[cfg(test)]
mod tests;
