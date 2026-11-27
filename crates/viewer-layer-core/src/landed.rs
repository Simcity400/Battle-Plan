use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc,
};

static LANDED: AtomicU64 = AtomicU64::new(0);

pub fn count() -> u64 {
    LANDED.load(Ordering::Acquire)
}

pub fn moved() {
    LANDED.fetch_add(1, Ordering::AcqRel);
}

pub struct Sender<T>(mpsc::Sender<T>);

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        self.0.send(value)?;
        moved();
        Ok(())
    }
}

pub fn channel<T>() -> (Sender<T>, mpsc::Receiver<T>) {
    let (send, receive) = mpsc::channel();
    (Sender(send), receive)
}
