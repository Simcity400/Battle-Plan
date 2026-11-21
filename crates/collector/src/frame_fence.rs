use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(crate) struct Frames {
    started: AtomicU64,
    published: AtomicU64,
}

pub(crate) struct Capture<'a> {
    frames: &'a Frames,
    sequence: u64,
}

impl Frames {
    pub(crate) fn capture(&self) -> Capture<'_> {
        Capture {
            frames: self,
            sequence: self.started.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }

    pub(crate) fn snapshot(&self) -> u64 {
        self.started.load(Ordering::SeqCst)
    }

    pub(crate) fn pending<'a>(
        &self,
        fence: u64,
        owned: impl IntoIterator<Item = &'a AtomicU64>,
    ) -> bool {
        self.published.load(Ordering::Acquire) < fence
            || owned.into_iter().any(|owned| {
                let sequence = owned.load(Ordering::Acquire);
                sequence != 0 && sequence <= fence
            })
    }

    pub(crate) fn recycle(&self, owned: &AtomicU64) {
        owned.store(0, Ordering::Release);
    }
}

impl Capture<'_> {
    pub(crate) fn frame(&self, owned: &AtomicU64) {
        owned.store(self.sequence, Ordering::Release);
    }
}

impl Drop for Capture<'_> {
    fn drop(&mut self) {
        self.frames
            .published
            .store(self.sequence, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_includes_acquisition_and_decode_but_not_later_captures() {
        let frames = Frames::default();
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
        let frames = Frames::default();
        let capture = frames.capture();
        let fence = frames.snapshot();
        drop(capture);
        assert!(!frames.pending(fence, std::iter::empty()));
    }
}
