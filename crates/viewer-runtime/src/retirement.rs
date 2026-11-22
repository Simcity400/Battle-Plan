
use job_priority::SpawnCarried as _;
use crate::worker_priority::normal_priority;
use std::{io, sync::mpsc, thread};

pub fn builder_count() -> usize {
    crate::parallel::usable_processors()
        .saturating_sub(8)
        .max(2)
}

pub struct Dropper {
    bin: Option<mpsc::Sender<Box<dyn Send>>>,
}

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

    pub fn drop_later<T: Send + 'static>(&self, value: T) {
        if let Some(bin) = &self.bin {
            let _ = bin.send(Box::new(value));
        }
    }
}

#[cfg(test)]
mod tests;
