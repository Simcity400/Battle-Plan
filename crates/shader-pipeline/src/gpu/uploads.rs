//! Owned writes from one serial arena checkpoint. Recording consumes their exact bytes without
//! borrowing the arena that prepared them.

/// Writes reached at one arena flush boundary, in their original order.
///
/// Source bytes and destination handles are owned, but destination bytes are not versioned:
/// the arena must still not reset/reuse buffers before dependent commands are submitted.
/// Executing a checkpoint writes bytes only; it does not submit commands.
#[derive(Debug)]
#[must_use = "reached checkpoint writes must execute even if the main frame fails"]
pub struct FrozenBufferWrites {
    frame: u64,
    writes: Vec<(wgpu::Buffer, u64, Vec<u8>)>,
}

impl FrozenBufferWrites {
    pub(super) fn new(frame: u64, writes: Vec<(wgpu::Buffer, u64, Vec<u8>)>) -> Self {
        Self { frame, writes }
    }

    /// The source arena's frame sequence at the checkpoint, not a physical staging-slot ID.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Exact ordered destinations, offsets and bytes, with no arena or preparation borrow.
    pub fn writes(&self) -> impl ExactSizeIterator<Item = (&wgpu::Buffer, u64, &[u8])> {
        self.writes
            .iter()
            .map(|(buffer, offset, bytes)| (buffer, *offset, bytes.as_slice()))
    }

    /// Replay this boundary once, before submitting its dependent commands.
    pub fn execute(self, queue: &wgpu::Queue) {
        for (buffer, offset, bytes) in self.writes {
            queue.write_buffer(&buffer, offset, &bytes);
        }
    }
}
