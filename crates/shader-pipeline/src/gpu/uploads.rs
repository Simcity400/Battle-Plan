
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

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn writes(&self) -> impl ExactSizeIterator<Item = (&wgpu::Buffer, u64, &[u8])> {
        self.writes
            .iter()
            .map(|(buffer, offset, bytes)| (buffer, *offset, bytes.as_slice()))
    }

    pub fn execute(self, queue: &wgpu::Queue) {
        for (buffer, offset, bytes) in self.writes {
            queue.write_buffer(&buffer, offset, &bytes);
        }
    }
}
