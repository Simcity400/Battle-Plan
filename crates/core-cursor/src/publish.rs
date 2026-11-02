use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

// Readers pin before loading; only publishers destroy retired snapshots, after all pins leave.
pub(crate) struct Published<T> {
    current: AtomicPtr<T>,
    readers: AtomicUsize,
    retired: Mutex<Option<Box<T>>>,
}

impl<T> Published<T> {
    pub fn new() -> Self {
        Self {
            current: AtomicPtr::new(std::ptr::null_mut()),
            readers: AtomicUsize::new(0),
            retired: Mutex::new(None),
        }
    }
    pub fn read(&self) -> Read<'_, T> {
        self.readers.fetch_add(1, Ordering::SeqCst);
        let pointer = self.current.load(Ordering::SeqCst);
        Read {
            publication: self,
            pointer,
        }
    }
    pub fn publish(&self, next: Box<T>) -> Result<Option<Box<T>>, Box<T>> {
        let mut retired = self.retired.lock().unwrap();
        if retired.is_some() {
            if self.readers.load(Ordering::SeqCst) != 0 {
                return Err(next);
            }
            *retired = None;
        }
        let previous = self.current.swap(Box::into_raw(next), Ordering::SeqCst);
        let mut recycled = None;
        if !previous.is_null() {
            let previous = unsafe { Box::from_raw(previous) };
            if self.readers.load(Ordering::SeqCst) == 0 {
                recycled = Some(previous);
            } else {
                *retired = Some(previous);
            }
        }
        Ok(recycled)
    }
    pub fn collect(&self) -> Option<Box<T>> {
        let mut retired = self.retired.lock().unwrap();
        if self.readers.load(Ordering::SeqCst) == 0 {
            retired.take()
        } else {
            None
        }
    }
    pub fn clear(&self) -> bool {
        let mut retired = self.retired.lock().unwrap();
        if retired.is_some() && self.readers.load(Ordering::SeqCst) != 0 {
            return false;
        }
        *retired = None;
        let previous = self.current.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !previous.is_null() {
            let previous = unsafe { Box::from_raw(previous) };
            if self.readers.load(Ordering::SeqCst) == 0 {
                drop(previous);
            } else {
                *retired = Some(previous);
            }
        }
        true
    }
}

unsafe impl<T: Send + Sync> Send for Published<T> {}
unsafe impl<T: Send + Sync> Sync for Published<T> {}

pub(crate) struct Read<'a, T> {
    publication: &'a Published<T>,
    pointer: *const T,
}
impl<T> Read<'_, T> {
    pub fn get(&self) -> Option<&T> {
        unsafe { self.pointer.as_ref() }
    }
}
impl<T> Drop for Read<'_, T> {
    fn drop(&mut self) {
        self.publication.readers.fetch_sub(1, Ordering::SeqCst);
    }
}
impl<T> Drop for Published<T> {
    fn drop(&mut self) {
        let pointer = *self.current.get_mut();
        if !pointer.is_null() {
            unsafe {
                drop(Box::from_raw(pointer));
            }
        }
    }
}

// A successful claim owns the cell until its release; the frame makes one attempt and never waits.
pub(crate) struct ErrorMailbox {
    state: std::sync::atomic::AtomicU8,
    error: std::cell::UnsafeCell<Option<std::io::Error>>,
}
impl ErrorMailbox {
    pub fn new() -> Self {
        Self {
            state: std::sync::atomic::AtomicU8::new(0),
            error: std::cell::UnsafeCell::new(None),
        }
    }
    pub fn post(&self, error: std::io::Error) {
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            unsafe {
                *self.error.get() = Some(error);
            }
            self.state.store(2, Ordering::Release);
        }
    }
    pub fn take(&self) -> Option<std::io::Error> {
        if self
            .state
            .compare_exchange(2, 3, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        let error = unsafe { (&mut *self.error.get()).take() };
        self.state.store(0, Ordering::Release);
        error
    }
}
unsafe impl Send for ErrorMailbox {}
unsafe impl Sync for ErrorMailbox {}
