use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Frames {
    started: AtomicU64,
    published: AtomicU64,
}

pub struct Capture<'a> {
    frames: &'a Frames,
    sequence: u64,
}

impl Frames {
    /// # Safety
    /// Start a fence only in the frame pool's serialized admitted capture. Publish in sequence
    /// order and keep the pool alive through the capture guard's drop.
    pub unsafe fn capture(&self) -> Capture<'_> {
        Capture {
            frames: self,
            sequence: self.started.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }

    pub fn snapshot(&self) -> u64 {
        self.started.load(Ordering::SeqCst)
    }

    /// # Safety
    /// Use a fence sampled from this session and supply every occupied slot from the same pool,
    /// including slots published before the fence completed.
    pub unsafe fn pending<'a>(
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

    /// # Safety
    /// Only the owner of this pool slot may retire its fence, once the spool has settled the frame
    /// or the capture has been refused.
    pub unsafe fn recycle(&self, owned: &AtomicU64) {
        owned.store(0, Ordering::Release);
    }
}

impl Capture<'_> {
    /// # Safety
    /// Publish only the installed slot owned by this capture. Keep the slot in the pool until it is
    /// recycled by its next owner.
    pub unsafe fn frame(&self, owned: &AtomicU64) {
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
        let acquiring =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        let fence = frames.snapshot();
        assert!(
            // SAFETY: The originating session supplies its own frame slots and fence.
            unsafe { frames.pending(fence, &owned) }
        );
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { acquiring.frame(&owned[0]) };
        drop(acquiring);
        assert!(
            // SAFETY: The originating session supplies its own frame slots and fence.
            unsafe { frames.pending(fence, &owned) }
        );
        let later =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { later.frame(&owned[1]) };
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { frames.recycle(&owned[0]) };
        assert!(!
        // SAFETY: The originating session supplies its own frame slots and fence.
        unsafe { frames.pending(fence, &owned) });
        let next = frames.snapshot();
        assert!(
            // SAFETY: The originating session supplies its own frame slots and fence.
            unsafe { frames.pending(next, &owned) }
        );
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { frames.recycle(&owned[1]) };
        assert!(
            // SAFETY: The originating session supplies its own frame slots and fence.
            unsafe { frames.pending(next, &owned) }
        );
        drop(later);
        assert!(!
        // SAFETY: The originating session supplies its own frame slots and fence.
        unsafe { frames.pending(next, &owned) });
    }

    #[test]
    fn a_capture_without_a_frame_does_not_leave_a_pending_fence() {
        let frames = Frames::default();
        let capture =
        // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
        unsafe { frames.capture() };
        let fence = frames.snapshot();
        drop(capture);
        assert!(!
        // SAFETY: The originating session supplies its own frame slots and fence.
        unsafe { frames.pending(fence, std::iter::empty()) });
    }
}
