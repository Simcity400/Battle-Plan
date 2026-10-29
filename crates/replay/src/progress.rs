use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::Arc};

thread_local! {
    static CURRENT: RefCell<Option<Progress>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct Progress(Arc<dyn Fn() + Send + Sync>);

pub struct Guard {
    previous: Option<Progress>,
    _thread: PhantomData<Rc<()>>,
}

impl Progress {
    pub fn new(completed: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(completed))
    }

    pub fn install(self) -> Guard {
        Guard {
            previous: CURRENT.with_borrow_mut(|slot| slot.replace(self)),
            _thread: PhantomData,
        }
    }

    pub fn current() -> Option<Self> {
        CURRENT
            .with_borrow(Clone::clone)
            .or_else(crate::container::pack::progress)
    }

    /// Each 64 KiB of compression or reading: a packing thread waits out the game's tick
    /// here, before a callback that may lock.
    pub fn completed() {
        crate::container::pack::wait_here();
        if let Some(progress) = Self::current() {
            (progress.0)();
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|slot| *slot = self.previous.take());
    }
}

pub(crate) struct Reader<T>(pub T);

impl<T: std::io::Read> std::io::Read for Reader<T> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(64 * 1024);
        let count = self.0.read(&mut output[..length])?;
        if count != 0 {
            Progress::completed();
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Cursor, Read, Write},
        sync::atomic::{AtomicU64, Ordering},
    };

    #[test]
    fn completed_chunks_are_scoped_and_eof_or_idle_work_does_not_advance_them() {
        let ticks = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&ticks);
        let progress = Progress::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        })
        .install();
        let mut source = crate::container::Budgeted::new(Cursor::new(vec![1; 192 * 1024]));
        let mut bytes = vec![0; 192 * 1024];
        source.read_exact(&mut bytes).unwrap();
        assert_eq!(ticks.load(Ordering::Relaxed), 3);
        assert_eq!(source.read(&mut bytes).unwrap(), 0);
        assert_eq!(ticks.load(Ordering::Relaxed), 3);
        let mut destination = crate::container::Budgeted::new(Vec::new());
        destination.write_all(&bytes).unwrap();
        assert_eq!(ticks.load(Ordering::Relaxed), 6);
        drop(progress);
        Progress::completed();
        assert_eq!(ticks.load(Ordering::Relaxed), 6);
    }
}
