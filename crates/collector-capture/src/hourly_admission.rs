use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct CaptureAdmission {
    closed: AtomicBool,
    active: AtomicUsize,
}

impl CaptureAdmission {
    #[cfg(any(test, feature = "test-support"))]
    pub const fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            active: AtomicUsize::new(0),
        }
    }

    const fn closed() -> Self {
        Self {
            closed: AtomicBool::new(true),
            active: AtomicUsize::new(0),
        }
    }

    /// # Safety
    /// Only the runtime owner may open hourly admission, after publishing the capture state
    /// and before starting teardown. No previous runtime may still be using the gate.
    pub unsafe fn open(&self) {
        self.closed.store(false, Ordering::SeqCst);
    }

    pub fn enter(&self) -> Option<CaptureLease<'_>> {
        self.active.fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::SeqCst) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(CaptureLease(self))
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub fn quiet(&self) -> bool {
        self.active.load(Ordering::SeqCst) == 0
    }
}

pub struct CaptureLease<'a>(&'a CaptureAdmission);
impl Drop for CaptureLease<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub static HOURLY_ADMISSION: CaptureAdmission = CaptureAdmission::closed();

pub(crate) fn admit_hourly_capture() -> Option<CaptureLease<'static>> {
    HOURLY_ADMISSION.enter()
}
