use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Frames {
    active: AtomicBool,
    started: AtomicU64,
    published: AtomicU64,
}

pub struct Capture<'a> {
    frames: &'a Frames,
    sequence: u64,
}

#[derive(Default)]
pub struct Snapshot(Option<u64>);

impl Snapshot {
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_started(&self) -> bool {
        self.0.is_some()
    }
}

impl Frames {
    pub fn new(_owner: &crate::Owner) -> Self {
        Self::empty()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_test() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        Self {
            active: AtomicBool::new(false),
            started: AtomicU64::new(0),
            published: AtomicU64::new(0),
        }
    }

    pub fn capture(&self) -> Capture<'_> {
        assert!(
            !self.active.swap(true, Ordering::Acquire),
            "a capture already owns this fence"
        );
        Capture {
            frames: self,
            sequence: self.started.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }

    pub fn snapshot(&self) -> u64 {
        self.started.load(Ordering::SeqCst)
    }

    pub fn pending_snapshot<'a>(
        &self,
        fence: &mut Snapshot,
        owned: impl IntoIterator<Item = &'a AtomicU64>,
    ) -> bool {
        self.pending(*fence.0.get_or_insert_with(|| self.snapshot()), owned)
    }

    pub fn pending<'a>(&self, fence: u64, owned: impl IntoIterator<Item = &'a AtomicU64>) -> bool {
        self.published.load(Ordering::Acquire) < fence
            || owned.into_iter().any(|owned| {
                let sequence = owned.load(Ordering::Acquire);
                sequence != 0 && sequence <= fence
            })
    }

    pub fn recycle(&self, owned: &AtomicU64) {
        owned.store(0, Ordering::Release);
    }
}

impl Capture<'_> {
    pub fn frame(&self, owned: &AtomicU64) {
        owned.store(self.sequence, Ordering::Release);
    }
}

impl Drop for Capture<'_> {
    fn drop(&mut self) {
        self.frames
            .published
            .store(self.sequence, Ordering::Release);
        self.frames.active.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_includes_acquisition_and_decode_but_not_later_captures() {
        let frames = Frames::new_for_test();
        let owned = [AtomicU64::new(0), AtomicU64::new(0)];
        let acquiring = frames.capture();
        let fence = frames.snapshot();
        assert!(frames.pending(fence, &owned));
        acquiring.frame(&owned[0]);
        drop(acquiring);
        assert!(frames.pending(fence, &owned));
        let later = frames.capture();
        later.frame(&owned[1]);
        frames.recycle(&owned[0]);
        assert!(!frames.pending(fence, &owned));
        let next = frames.snapshot();
        assert!(frames.pending(next, &owned));
        frames.recycle(&owned[1]);
        assert!(frames.pending(next, &owned));
        drop(later);
        assert!(!frames.pending(next, &owned));
    }

    #[test]
    fn a_capture_without_a_frame_does_not_leave_a_pending_fence() {
        let frames = Frames::new_for_test();
        let capture = frames.capture();
        let fence = frames.snapshot();
        drop(capture);
        assert!(!frames.pending(fence, std::iter::empty()));
    }
}
