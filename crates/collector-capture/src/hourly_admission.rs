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
    /// Only the attaching runtime may open the live gate, once the capture state is published and
    /// before the first admitted hour.
    pub unsafe fn open(&self) {
        self.closed.store(false, Ordering::SeqCst);
    }

    /// # Safety
    /// Enter the live gate only for an actual hourly callback or its beside worker. Drop the lease
    /// when that hour has completed and before detach waits for quiescence.
    pub unsafe fn enter(&self) -> Option<CaptureLease<'_>> {
        self.active.fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::SeqCst) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(CaptureLease(self))
        }
    }

    /// # Safety
    /// Only the detaching runtime may close the live gate, before waiting for every admitted hour
    /// to leave.
    pub unsafe fn close(&self) {
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
    // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
    // hourly exit.
    unsafe { HOURLY_ADMISSION.enter() }
}
