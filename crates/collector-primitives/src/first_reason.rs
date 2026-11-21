use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

#[derive(Default)]
pub struct FirstReason {
    code: AtomicPtr<u8>,
    length: AtomicUsize,
}
impl FirstReason {
    pub const fn new() -> Self {
        Self {
            code: AtomicPtr::new(std::ptr::null_mut()),
            length: AtomicUsize::new(0),
        }
    }
    pub fn record(&self, reason: &'static str) {
        if self
            .code
            .compare_exchange(
                std::ptr::null_mut(),
                reason.as_ptr().cast_mut(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            self.length.store(reason.len() + 1, Ordering::Release);
        }
    }
    pub fn get(&self) -> Option<&'static str> {
        let length = self.length.load(Ordering::Acquire).checked_sub(1)?;
        let code = self.code.load(Ordering::Relaxed);
        Some(unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(code, length)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_publication_keeps_one_complete_static_reason_without_allocating() {
        let reason = FirstReason::new();
        std::thread::scope(|scope| {
            for text in ["first", "second reason", ""] {
                let reason = &reason;
                scope.spawn(move || reason.record(text));
            }
        });
        assert!(["first", "second reason", ""].contains(&reason.get().unwrap()));
        let original = reason.get();
        collector_test_support::with_allocations_denied(|| reason.record("later"));
        assert_eq!(reason.get(), original);
    }
    #[test]
    fn an_unfinished_publication_never_waits_or_reads_partial_text() {
        let reason = FirstReason::new();
        reason
            .code
            .store("pending".as_ptr().cast_mut(), Ordering::Relaxed);
        reason.record("other");
        assert_eq!(reason.get(), None);
        reason.length.store("pending".len() + 1, Ordering::Release);
        assert_eq!(reason.get(), Some("pending"));
    }
}
