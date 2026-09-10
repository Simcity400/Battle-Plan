//! Deterministic guarded-read census used by pure-function cost proxies.
//!
//! A walker's game-thread cost is dominated by how many times it crosses the
//! guarded copy boundary and how many bytes it pulls out of the foreign heap.
//! Wrapping any `Memory` in this counter makes both quantities exact and
//! reproducible in ordinary `cargo test` assertions, with no timing, no
//! sampling and no framework. It records observations only; every returned
//! value, completed-prefix count and failure position comes from the source.

use std::cell::Cell;

use crate::{Memory, memory::RecordField};

/// Counts boundary crossings, copied spans and copied bytes for one walk.
pub struct ReadCensus<'a, M> {
    source: &'a M,
    calls: Cell<usize>,
    spans: Cell<usize>,
    bytes: Cell<usize>,
}

impl<'a, M: Memory> ReadCensus<'a, M> {
    pub fn new(source: &'a M) -> Self {
        Self {
            source,
            calls: Cell::new(0),
            spans: Cell::new(0),
            bytes: Cell::new(0),
        }
    }

    /// Guarded boundary crossings: one per `copy`, `copy_scattered` or
    /// `copy_records` invocation that reaches the source.
    pub fn calls(&self) -> usize {
        self.calls.get()
    }

    /// Individual nonempty spans copied out of the source.
    pub fn spans(&self) -> usize {
        self.spans.get()
    }

    /// Bytes copied out of the source, counting only completed spans.
    pub fn bytes(&self) -> usize {
        self.bytes.get()
    }

    pub fn reset(&self) {
        self.calls.set(0);
        self.spans.set(0);
        self.bytes.set(0);
    }

    fn observe(&self, spans: usize, bytes: usize) {
        self.calls.set(self.calls.get() + 1);
        self.spans.set(self.spans.get() + spans);
        self.bytes.set(self.bytes.get() + bytes);
    }
}

impl<M: Memory> Memory for ReadCensus<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let len = destination.len();
        let copied = self.source.copy(address, destination);
        self.observe(
            usize::from(copied && len != 0),
            if copied { len } else { 0 },
        );
        copied
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let lengths = destinations.each_ref().map(|destination| destination.len());
        let copied = self.source.copy_scattered(addresses, destinations);
        let completed = &lengths[..copied.min(N)];
        self.observe(
            completed.iter().filter(|&&len| len != 0).count(),
            completed.iter().sum(),
        );
        copied
    }

    fn copy_records(
        &self,
        pointers: &[usize],
        fields: &[RecordField],
        row_stride: usize,
        destination: &mut [u8],
    ) -> usize {
        let completed = self
            .source
            .copy_records(pointers, fields, row_stride, destination);
        let mut spans = 0;
        let mut bytes = 0;
        for index in 0..completed {
            let len = fields[index % fields.len()].len;
            spans += usize::from(len != 0);
            bytes += len;
        }
        self.observe(spans, bytes);
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    fn source() -> SliceMemory<'static> {
        static BYTES: [u8; 64] = [7; 64];
        SliceMemory {
            base: 0x1000,
            bytes: &BYTES,
        }
    }

    #[test]
    fn every_shape_counts_only_completed_spans_and_their_exact_bytes() {
        let source = source();
        let census = ReadCensus::new(&source);
        let mut single = [0; 8];
        assert!(census.copy(0x1000, &mut single));
        assert!(!census.copy(0x2000, &mut single));
        assert_eq!((census.calls(), census.spans(), census.bytes()), (2, 1, 8));
        census.reset();
        assert_eq!((census.calls(), census.spans(), census.bytes()), (0, 0, 0));

        let mut empty = [];
        let mut first = [0; 4];
        let mut failed = [0; 4];
        assert_eq!(
            census.copy_scattered(
                [0x1000, 0x1004, 0x9000],
                &mut [&mut empty, &mut first, &mut failed],
            ),
            2
        );
        assert_eq!((census.calls(), census.spans(), census.bytes()), (1, 1, 4));

        census.reset();
        let fields = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 4,
                len: 0,
            },
            RecordField {
                source_offset: 16,
                destination_offset: 4,
                len: 2,
            },
        ];
        let mut rows = [0; 16];
        assert_eq!(
            census.copy_records(&[0x1000, 0x9000], &fields, 8, &mut rows),
            3
        );
        assert_eq!((census.calls(), census.spans(), census.bytes()), (1, 2, 6));
    }
}
