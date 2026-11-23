pub use collector_observers::air_base_carry_changes::registration::Census as Counts;
use collector_raw::raw::Span;
use engine_native_air::cair_base;
use engine_native::cpdx_array::Array;
use std::collections::HashMap;

pub const SLOT_ARRAYS: usize = 3;

#[derive(Debug, Default)]
struct Slots {
    certificate: Option<(u64, u64)>,
    header: (usize, u32),
    spans: Vec<Span>,
    bytes: Vec<u8>,
    pass: u64,
}

#[derive(Debug)]
struct Row {
    handle: u64,
    slot: (usize, u32),
    certificate: (u64, u64),
    row: cair_base::Row,
    spans: Vec<Span>,
    bytes: Vec<u8>,
    pass: u64,
}

pub struct CarriedRow<'a> {
    pub handle: u64,
    pub certificate: (u64, u64),
    pub row: cair_base::Row,
    pub spans: &'a [Span],
    pub bytes: &'a [u8],
}

pub struct Fresh<'a> {
    pub spans: &'a [Span],
    pub bytes: &'a [u8],
    pub start: usize,
}

impl Fresh<'_> {
    fn copy(&self, spans: &mut Vec<Span>, bytes: &mut Vec<u8>) -> bool {
        let stored = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut *spans, self.spans.len())?;
            engine_allocation::reserve(&mut *bytes, self.bytes.len())
        });
        if stored.is_err() || self.spans.iter().any(|span| !span.ok) {
            return false;
        }
        spans.extend(self.spans.iter().map(|span| Span {
            start: span.start - self.start as u32,
            ..*span
        }));
        bytes.extend_from_slice(self.bytes);
        true
    }
}

#[derive(Debug, Default)]
pub struct CensusStore {
    slots: [Slots; SLOT_ARRAYS],
    rows: HashMap<usize, Row>,
    pass: u64,
    counts: Counts,
}

impl CensusStore {
    pub fn clear(&mut self) {
        self.slots = Default::default();
        self.rows.clear();
    }

    pub fn begin(&mut self, pass: u64) {
        self.pass = pass;
        self.counts = Counts::default();
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn slots(
        &mut self,
        array: usize,
        header: Array,
        certificate: Option<(u64, u64)>,
    ) -> Option<(&[Span], &[u8])> {
        let held = &mut self.slots[array];
        if held.certificate.is_none()
            || held.certificate != certificate
            || held.header != (header.data, header.count)
        {
            return None;
        }
        held.pass = self.pass;
        self.counts.arrays[0] += 1;
        self.counts.spans[1] += held.spans.len() as u64;
        Some((held.spans.as_slice(), held.bytes.as_slice()))
    }

    pub fn hold_slots(
        &mut self,
        array: usize,
        header: Array,
        certificate: Option<(u64, u64)>,
        complete: bool,
        fresh: &Fresh<'_>,
    ) {
        self.counts.arrays[1] += 1;
        self.counts.spans[0] += fresh.spans.len() as u64;
        let held = &mut self.slots[array];
        held.certificate = None;
        held.spans.clear();
        held.bytes.clear();
        if certificate.is_none() || !complete || !fresh.copy(&mut held.spans, &mut held.bytes) {
            held.spans.clear();
            held.bytes.clear();
            return;
        }
        held.certificate = certificate;
        held.header = (header.data, header.count);
        held.pass = self.pass;
    }

    pub fn row(
        &mut self,
        pointer: usize,
        slot: (usize, u32),
        certificate: impl FnOnce(u64) -> Option<(u64, u64)>,
    ) -> Option<CarriedRow<'_>> {
        let held = self.rows.get_mut(&pointer)?;
        if held.slot != slot || certificate(held.handle) != Some(held.certificate) {
            return None;
        }
        held.pass = self.pass;
        self.counts.rows[0] += 1;
        self.counts.spans[1] += held.spans.len() as u64;
        Some(CarriedRow {
            handle: held.handle,
            certificate: held.certificate,
            row: held.row,
            spans: &held.spans,
            bytes: &held.bytes,
        })
    }

    pub fn hold_row(
        &mut self,
        pointer: usize,
        slot: (usize, u32),
        handle: u64,
        certificate: Option<(u64, u64)>,
        row: cair_base::Row,
        fresh: &Fresh<'_>,
    ) {
        self.counts.rows[1] += 1;
        self.counts.spans[0] += fresh.spans.len() as u64;
        self.rows.remove(&pointer);
        let Some(certificate) = certificate else {
            return;
        };
        let mut held = Row {
            handle,
            slot,
            certificate,
            row,
            spans: Vec::new(),
            bytes: Vec::new(),
            pass: self.pass,
        };
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.rows, 1))
            .is_err()
            || !fresh.copy(&mut held.spans, &mut held.bytes)
        {
            return;
        }
        self.rows.insert(pointer, held);
    }

    pub fn read(&mut self, spans: usize) {
        self.counts.spans[0] += spans as u64;
    }

    pub fn finish(&mut self, complete: bool) {
        if !complete {
            self.clear();
            return;
        }
        let pass = self.pass;
        self.rows.retain(|_, held| held.pass == pass);
        for held in &mut self.slots {
            if held.pass != pass {
                *held = Slots::default();
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.slots
            .iter()
            .map(|held| held.spans.capacity() * size_of::<Span>() + held.bytes.capacity())
            .sum::<usize>()
            + self.rows.capacity() * size_of::<(usize, Row)>()
            + self
                .rows
                .values()
                .map(|held| held.spans.capacity() * size_of::<Span>() + held.bytes.capacity())
                .sum::<usize>()
    }
}
