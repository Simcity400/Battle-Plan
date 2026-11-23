use engine_native_memory::Memory;

pub struct Counted<M> {
    pub inner: M,
    pub guards: std::cell::Cell<u64>,
    pub bytes: std::cell::Cell<u64>,
}

impl<M: Memory> Counted<M> {
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            guards: std::cell::Cell::new(0),
            bytes: std::cell::Cell::new(0),
        }
    }
    fn count(&self, bytes: usize) {
        self.guards.set(self.guards.get() + 1);
        self.bytes.set(self.bytes.get() + bytes as u64);
    }
    pub fn reset(&self) {
        self.guards.set(0);
        self.bytes.set(0);
    }
}

impl<M: Memory> Memory for Counted<M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.count(destination.len());
        self.inner.copy(address, destination)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        output: &mut [&mut [u8]; N],
    ) -> usize {
        self.count(output.iter().map(|bytes| bytes.len()).sum());
        self.inner.copy_scattered(addresses, output)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        output: &mut [&mut [u8]; N],
    ) -> usize {
        self.count(output.iter().map(|bytes| bytes.len()).sum());
        self.inner.copy_scattered_prefetched(addresses, output)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        output: &mut [u8],
    ) -> usize {
        let completed = self.inner.copy_records(sources, layout, output);
        self.count(layout.completed_bytes(completed));
        completed
    }
}
