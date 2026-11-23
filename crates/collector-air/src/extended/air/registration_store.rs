pub use collector_observers::air_base_carry_changes::registration::Counts;
use collector_raw::raw::Span;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SEGMENT: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct Held {
    pointer: usize,
    certificate: (u64, u64),
    planned: bool,
    spans: Vec<Span>,
    bytes: Vec<u8>,
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
    pub spans: &'a [Span],
    pub bytes: &'a [u8],
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
            kept[8..engine_native_air::cair_wing::SECONDARY].fill(0);
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
            spans: &held.spans,
            bytes: &held.bytes,
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
        let mut held = Held {
            pointer,
            certificate,
            planned,
            spans: Vec::new(),
            bytes: Vec::new(),
            links: Vec::new(),
            demand: fresh.demand,
            pass: self.pass,
            segment: NEXT_SEGMENT.fetch_add(1, Ordering::Relaxed),
        };
        let spans = fresh.held().count();
        let bytes = fresh.held().map(|span| span.len as usize).sum::<usize>();
        let stored = engine_allocation::advisory(|| {
            engine_allocation::reserve_map(&mut self.held, 1)?;
            engine_allocation::reserve(&mut held.spans, spans)?;
            engine_allocation::reserve(&mut held.bytes, bytes)?;
            engine_allocation::reserve(&mut held.links, fresh.links.len())
        });
        if stored.is_err() {
            return;
        }
        for span in fresh.held() {
            held.spans.push(Span {
                start: held.bytes.len() as u32,
                ..*span
            });
            fresh.consumed(span, &mut held.bytes);
        }
        held.links.extend_from_slice(fresh.links);
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
                    held.spans.capacity() * size_of::<Span>()
                        + held.bytes.capacity()
                        + held.links.capacity() * size_of::<(u32, u32)>()
                })
                .sum::<usize>();
    }
}
