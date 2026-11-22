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

    #[cfg(any(test, feature = "test-support"))]
    pub fn enter(&self) -> Option<CaptureLease<'_>> {
        assert!(!std::ptr::eq(self, &HOURLY_ADMISSION));
        self.enter_capture()
    }

    fn enter_capture(&self) -> Option<CaptureLease<'_>> {
        self.active.fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::SeqCst) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(CaptureLease(self))
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn quiet_inner(&self) -> bool {
        self.active.load(Ordering::SeqCst) == 0
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn quiet(&self) -> bool {
        self.quiet_inner()
    }
}

pub struct CaptureLease<'a>(&'a CaptureAdmission);
impl Drop for CaptureLease<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

static HOURLY_ADMISSION: CaptureAdmission = CaptureAdmission::closed();

pub struct RuntimeAdmission<'a> {
    _owner: &'a collector_workers::Owner,
}

pub fn runtime_admission(owner: &collector_workers::Owner) -> RuntimeAdmission<'_> {
    RuntimeAdmission { _owner: owner }
}

impl RuntimeAdmission<'_> {
    /// # Safety
    /// Capture state must already be published and teardown must not have started.
    pub unsafe fn open(&self) {
        HOURLY_ADMISSION.closed.store(false, Ordering::SeqCst);
    }

    pub fn close(&self) {
        HOURLY_ADMISSION.closed.store(true, Ordering::SeqCst);
    }

    pub fn quiet(&self) -> bool {
        HOURLY_ADMISSION.quiet_inner()
    }
}

pub(crate) fn admit_hourly_capture() -> Option<CaptureLease<'static>> {
    HOURLY_ADMISSION.enter_capture()
}
