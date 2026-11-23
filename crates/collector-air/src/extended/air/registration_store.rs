pub use collector_observers::air_base_carry_changes::registration::Counts;
use collector_raw::raw::Span;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SEGMENT: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, PartialEq, Eq)]
pub struct CarriedRaw {
    pub spans: Vec<Span>,
    pub bytes: Vec<u8>,
    pub dense_count: usize,
}

impl CarriedRaw {
    pub(crate) fn memory<'a, M: engine_native::Memory>(
        &'a self,
        fallback: &'a M,
    ) -> CarriedMemory<'a, M> {
        CarriedMemory {
            raw: self,
            fallback,
        }
    }
}

pub(crate) struct CarriedMemory<'a, M> {
    raw: &'a CarriedRaw,
    fallback: &'a M,
}

impl<M: engine_native::Memory> engine_native::Memory for CarriedMemory<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(output.len()) else {
            return false;
        };
        for span in self.raw.spans.iter().rev() {
            if span.ok
                && let Some(span_end) = span.addr.checked_add(span.len as usize)
                && address >= span.addr
                && end <= span_end
            {
                let offset = span.start as usize + (address - span.addr);
                if let Some(slice) = self.raw.bytes.get(offset..offset + output.len()) {
                    output.copy_from_slice(slice);
                    return true;
                }
            }
        }
        self.fallback.copy(address, output)
    }
}

fn dense_count_from_spans(pointer: usize, spans: &[Span], bytes: &[u8]) -> usize {
    let Some(target) = pointer.checked_add(engine_native::cair_base::REGISTERED_CONTAINERS) else {
        return 0;
    };
    let Some(target_end) = target.checked_add(16) else {
        return 0;
    };
    for span in spans.iter().rev() {
        if span.ok
            && let Some(span_end) = span.addr.checked_add(span.len as usize)
            && target >= span.addr
            && target_end <= span_end
        {
            let offset = span.start as usize + (target - span.addr);
            if let Some(slice) = bytes.get(offset..offset + 16) {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(slice);
                return engine_native::cpdx_array::Array::from_header(raw, 8)
                    .map_or(0, |array| array.count as usize);
            }
        }
    }
    0
}

#[derive(Debug)]
struct Held {
    pointer: usize,
    certificate: (u64, u64),
    planned: bool,
    raw: Arc<CarriedRaw>,
    links: Vec<(u32, u32)>,
    demand: [usize; 2],
    pass: u64,
    segment: u64,
}

#[derive(Debug, Default)]
pub struct RegistrationStore {
    held: HashMap<u64, Held>,
    pass: u64,
    counts: Counts,
    visited: usize,
    bases: usize,
    bytes: usize,
    forgotten: bool,
    pub(super) state_inputs: super::state_fuel_inputs::Store,
    pub(super) census: super::census_store::CensusStore,
}

pub struct Carried<'a> {
    pub raw: Arc<CarriedRaw>,
    pub links: &'a [(u32, u32)],
    pub demand: [usize; 2],
    pub segment: u64,
}

pub struct Fresh<'a> {
    pub spans: &'a [Span],
    pub bytes: &'a [u8],
    pub start: usize,
    pub links: &'a [(u32, u32)],
    pub demand: [usize; 2],
    pub link_table: std::ops::Range<usize>,
    pub containers: &'a [usize],
    pub wings: &'a [usize],
}

impl Fresh<'_> {
    fn held(&self) -> impl Iterator<Item = &Span> {
        self.spans
            .iter()
            .filter(|span| !(span.len == 4 && self.link_table.contains(&span.addr)))
            .filter(|span| span.ok || span.mask != 0)
    }

    fn consumed(&self, span: &Span, output: &mut Vec<u8>) {
        use engine_native_air::{
            cair_base::CONTAINER_SPAN,
            ccountry_air_container as container,
        };
        let start = span.start as usize - self.start;
        let at = output.len();
        output.extend_from_slice(&self.bytes[start..start + span.len as usize]);
        let kept = &mut output[at..];
        let names = |objects: &[usize], offset: usize| {
            objects
                .iter()
                .any(|&pointer| pointer.checked_add(offset) == Some(span.addr))
        };
        if span.len as usize == CONTAINER_SPAN && names(self.containers, container::COUNTRY) {
            let capacity = container::CAPACITY_RAW - container::COUNTRY;
            kept[4..capacity].fill(0);
            kept[capacity + 8..].fill(0);
        } else if span.len == 32 && names(self.wings, 0) {
            kept[8..engine_native_parts::cair_wing_layout::SECONDARY].fill(0);
        }
    }
}

impl RegistrationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn census_counts(&self) -> super::census_store::Counts {
        self.census.counts()
    }

    pub fn begin(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
            self.state_inputs.clear();
            self.census.clear();
        }
        self.pass += 1;
        self.counts = Counts::default();
        self.visited = 0;
        self.census.begin(self.pass);
    }

    pub fn visit(&mut self) {
        self.visited += 1;
    }

    pub fn carryable(
        &mut self,
        handle: u64,
        pointer: usize,
        certificate: Option<(u64, u64)>,
        planned: bool,
    ) -> Option<Carried<'_>> {
        let pass = self.pass;
        let held = self.held.get_mut(&handle)?;
        if held.pointer != pointer
            || Some(held.certificate) != certificate
            || held.planned != planned
        {
            return None;
        }
        held.pass = pass;
        Some(Carried {
            raw: Arc::clone(&held.raw),
            links: &held.links,
            demand: held.demand,
            segment: held.segment,
        })
    }

    pub fn segment(&self, handle: u64) -> Option<u64> {
        self.held
            .get(&handle)
            .filter(|held| held.pass == self.pass)
            .map(|held| held.segment)
    }

    pub fn carried(&mut self) {
        self.counts.carried += 1;
    }

    pub fn hold(
        &mut self,
        handle: u64,
        pointer: usize,
        certificate: Option<(u64, u64)>,
        planned: bool,
        fresh: &Fresh<'_>,
    ) {
        self.counts.read += 1;
        self.held.remove(&handle);
        let Some(certificate) =
            certificate.filter(|_| fresh.spans.iter().all(|span| span.ok || span.mask == 0))
        else {
            return;
        };
        let mut spans = Vec::new();
        let mut bytes = Vec::new();
        let mut links = Vec::new();
        let span_count = fresh.held().count();
        let byte_count = fresh.held().map(|span| span.len as usize).sum::<usize>();
        let stored = engine_allocation::advisory(|| {
            engine_allocation::reserve_map(&mut self.held, 1)?;
            engine_allocation::reserve(&mut spans, span_count)?;
            engine_allocation::reserve(&mut bytes, byte_count)?;
            engine_allocation::reserve(&mut links, fresh.links.len())
        });
        if stored.is_err() {
            return;
        }
        for span in fresh.held() {
            spans.push(Span {
                start: bytes.len() as u32,
                ..*span
            });
            fresh.consumed(span, &mut bytes);
        }
        let dense_count = dense_count_from_spans(pointer, &spans, &bytes);
        links.extend_from_slice(fresh.links);
        let held = Held {
            pointer,
            certificate,
            planned,
            raw: Arc::new(CarriedRaw {
                spans,
                bytes,
                dense_count,
            }),
            links,
            demand: fresh.demand,
            pass: self.pass,
            segment: NEXT_SEGMENT.fetch_add(1, Ordering::Relaxed),
        };
        self.held.insert(handle, held);
    }

    pub fn bases(&self) -> usize {
        self.bases
    }

    pub fn finish(&mut self, complete: bool) {
        if complete {
            let pass = self.pass;
            self.held.retain(|_, held| held.pass == pass);
            self.bases = self.visited;
        } else {
            self.held.clear();
            self.state_inputs.clear();
        }
        self.census.finish(complete);
        self.refresh_allocated_bytes();
    }

    pub(super) fn refresh_allocated_bytes(&mut self) {
        self.bytes = self.state_inputs.bytes()
            + self.census.bytes()
            + self.held.capacity() * size_of::<(u64, Held)>()
            + self
                .held
                .values()
                .map(|held| {
                    size_of::<CarriedRaw>()
                        + held.raw.spans.capacity() * size_of::<Span>()
                        + held.raw.bytes.capacity()
                        + held.links.capacity() * size_of::<(u32, u32)>()
                })
                .sum::<usize>();
    }
}
