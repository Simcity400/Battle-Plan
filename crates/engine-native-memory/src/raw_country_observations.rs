
use crate::{cpdx_array::Array, msvc_string, walk::Outcome};
use std::mem::size_of;

pub type Field<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub occurrences: usize,
    pub links: usize,
    pub bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Countries,
    AppearanceOriginal,
    ReservedTag,
    ReservedOriginal,
    TemplateFallback,
    AttackOwner,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attempt<T> {
    #[default]
    Unreached,
    Completed(Field<T>),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextAttempt {
    pub address: usize,
    pub header: Attempt<[u8; msvc_string::STRIDE]>,
    pub length: Attempt<u64>,
    pub capacity: Attempt<u64>,
    pub source: Attempt<usize>,
    pub terminator: Attempt<u8>,
    pub payload: Attempt<Span>,
    pub policy_limited: bool,
    pub failure: Option<&'static str>,
}
#[derive(Clone, Copy, Debug)]
pub enum Occurrence {
    Text {
        raw: i32,
        source: Source,
        attempted: TextAttempt,
    },
    TagSelection {
        raw: i32,
        source: Source,
        result: Field<Array>,
    },
    ReservedDatabase {
        result: Field<usize>,
    },
    OriginalReference {
        country: u32,
        result: Field<i32>,
    },
    LinkControls {
        result: Field<[Array; 2]>,
    },
    TagControl {
        result: Field<Array>,
    },
    Link {
        raw: u32,
        result: Field<u32>,
    },
    Table {
        source: Source,
        outcome: Outcome,
    },
}

#[derive(Clone, Copy)]
pub struct LinkObservations<'a> {
    words: &'a [Option<Field<u32>>],
}
impl LinkObservations<'_> {
    pub fn word(&self, raw: u32) -> Option<Field<u32>> {
        self.words.get(raw as usize).copied().flatten()
    }
}
#[cfg(any(test, feature = "test-support"))]
pub struct MapLoan<'a> {
    pub links: LinkObservations<'a>,
}

pub struct Journal {
    hour: i32,
    occurrences: Vec<Occurrence>,
    links: Vec<Option<Field<u32>>>,
    link_bound: u32,
    bytes: Vec<u8>,
    used: usize,
    required: Capacity,
    failed: Option<&'static str>,
    sealed: bool,
}
impl Journal {
    pub fn vacant() -> Self {
        Self {
            hour: 0,
            occurrences: Vec::new(),
            links: Vec::new(),
            link_bound: 0,
            bytes: Vec::new(),
            used: 0,
            required: Capacity::default(),
            failed: None,
            sealed: false,
        }
    }
    pub fn try_new(capacity: Capacity) -> std::io::Result<Self> {
        let mut value = Self::vacant();
        value.reserve(capacity)?;
        Ok(value)
    }
    pub fn reserve(&mut self, capacity: Capacity) -> std::io::Result<()> {
        if self.occurrences.capacity() < capacity.occurrences {
            let additional = capacity.occurrences.saturating_sub(self.occurrences.len());
            engine_allocation::reserve_exact(&mut self.occurrences, additional)?;
        }
        if self.links.len() < capacity.links {
            engine_allocation::resize(&mut self.links, capacity.links, None)?;
        }
        if self.bytes.len() < capacity.bytes {
            engine_allocation::resize(&mut self.bytes, capacity.bytes, 0)?;
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        self.occurrences.clear();
        self.links.fill(None);
        self.link_bound = 0;
        self.used = 0;
        self.required = Capacity::default();
        self.failed = None;
        self.sealed = false;
    }
    pub fn begin(&mut self, hour: i32) {
        self.reset();
        self.hour = hour;
    }
    pub fn sample_hour(&self) -> i32 {
        self.hour
    }
    pub fn required(&self) -> Capacity {
        self.required
    }
    pub fn allocated_bytes(&self) -> usize {
        self.occurrences.capacity() * size_of::<Occurrence>()
            + self.links.capacity() * size_of::<Option<Field<u32>>>()
            + self.bytes.capacity()
    }
    pub fn refuse(&mut self, reason: &'static str) {
        self.failed.get_or_insert(reason);
    }
    pub fn seal(&mut self) {
        self.sealed = true;
    }
    pub fn complete(&self) -> bool {
        self.sealed && self.failed.is_none()
    }
    pub fn bytes(&self, span: Span) -> Option<&[u8]> {
        self.bytes
            .get(span.start..span.start.checked_add(span.len)?)
            .filter(|_| span.start + span.len <= self.used)
    }
    pub fn finish_native(&mut self) {
        self.occurrences.clear();
        self.links.fill(None);
    }
    fn occurrence(&mut self, value: Occurrence) -> Option<usize> {
        self.required.occurrences = self.required.occurrences.saturating_add(1);
        if engine_allocation::reserve_push_batch(&mut self.occurrences, 1).is_err() {
            self.refuse("raw country observation allocation failed");
            return None;
        }
        self.occurrences.push(value);
        Some(self.occurrences.len() - 1)
    }
    pub fn table(&mut self, source: Source, outcome: Outcome) {
        self.occurrence(Occurrence::Table { source, outcome });
    }
    pub fn link_controls(&mut self, result: Field<[Array; 2]>) {
        self.occurrence(Occurrence::LinkControls { result });
        if let Ok([_, links]) = result {
            self.link_bound = links.count;
            self.required.links = self.required.links.max(links.count as usize);
            self.grow_links(links.count as usize);
        }
    }
    pub fn tag_control(&mut self, result: Field<Array>) {
        self.occurrence(Occurrence::TagControl { result });
    }
    pub fn tag_selection(&mut self, raw: i32, source: Source, result: Field<Array>) {
        self.occurrence(Occurrence::TagSelection {
            raw,
            source,
            result,
        });
    }
    pub fn reserved_database(&mut self, result: Field<usize>) {
        self.occurrence(Occurrence::ReservedDatabase { result });
    }
    pub fn original_reference(&mut self, country: u32, result: Field<i32>) {
        self.occurrence(Occurrence::OriginalReference { country, result });
    }
    pub fn link(&mut self, raw: u32, result: Field<u32>) {
        self.occurrence(Occurrence::Link { raw, result });
        if raw >= self.link_bound {
            self.refuse("raw country link outside link table");
            return;
        }
        if let Some(slot) = self.links.get_mut(raw as usize)
            && slot.is_none()
        {
            *slot = Some(result);
        }
    }
    fn grow_links(&mut self, required: usize) {
        if required > self.links.len()
            && engine_allocation::resize(&mut self.links, required, None).is_err()
        {
            self.refuse("raw country link allocation failed");
        }
    }
    pub fn begin_text(&mut self, raw: i32, source: Source, address: usize) -> Option<usize> {
        self.occurrence(Occurrence::Text {
            raw,
            source,
            attempted: TextAttempt {
                address,
                ..Default::default()
            },
        })
    }
    pub fn finish_text(
        &mut self,
        index: Option<usize>,
        mut attempted: TextAttempt,
        payload: Option<&[u8]>,
    ) {
        if let Some(payload) = payload {
            attempted.payload = Attempt::Completed(self.store(payload));
            if let Attempt::Completed(Err(reason)) = attempted.payload {
                attempted.failure = Some(reason);
            }
        }
        if let Some(index) = index
            && let Some(Occurrence::Text {
                attempted: target, ..
            }) = self.occurrences.get_mut(index)
        {
            *target = attempted;
        }
    }
    pub fn text_attempt(&self, index: Option<usize>) -> Option<TextAttempt> {
        match self.occurrences.get(index?)? {
            Occurrence::Text { attempted, .. } => Some(*attempted),
            _ => None,
        }
    }
    fn store(&mut self, payload: &[u8]) -> Field<Span> {
        self.required.bytes = self.required.bytes.saturating_add(payload.len());
        let end = self
            .used
            .checked_add(payload.len())
            .ok_or("raw country byte demand overflow")?;
        self.grow_bytes(end)?;
        let output = self
            .bytes
            .get_mut(self.used..end)
            .ok_or("raw country byte arena exhausted")?;
        output.copy_from_slice(payload);
        let span = Span {
            start: self.used,
            len: payload.len(),
        };
        self.used = end;
        Ok(span)
    }
    fn grow_bytes(&mut self, required: usize) -> Field<()> {
        let capacity = required.max(self.bytes.len().saturating_mul(2));
        if required > self.bytes.len()
            && engine_allocation::resize(&mut self.bytes, capacity, 0).is_err()
        {
            self.refuse("raw country byte allocation failed");
            return Err("raw country byte allocation failed");
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn map_loan(&mut self) -> MapLoan<'_> {
        MapLoan {
            links: LinkObservations {
                words: &self.links,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_outside_the_game_link_table_are_refused_without_growth() {
        let count = 6;
        let mut journal = Journal::try_new(Capacity {
            occurrences: 8,
            ..Default::default()
        })
        .unwrap();
        journal.begin(1);
        journal.link_controls(Ok([
            Array {
                data: 0x2000,
                count: 4,
            },
            Array {
                data: 0x1000,
                count,
            },
        ]));
        let retained = journal.allocated_bytes();
        let growths = engine_allocation::growths();
        journal.link(count - 1, Ok(3));
        assert_eq!(journal.failed, None);
        for raw in [count, u32::MAX - 1] {
            journal.link(raw, Err("country link unreadable or outside link table"));
            assert_eq!(engine_allocation::growths(), growths);
            assert_eq!(journal.allocated_bytes(), retained);
        }
        assert_eq!(journal.required().links, count as usize);
        assert_eq!(journal.failed, Some("raw country link outside link table"));
        journal.seal();
        assert!(!journal.complete());
        let links = journal.map_loan().links;
        assert_eq!(links.word(count - 1), Some(Ok(3)));
        assert_eq!(links.word(count), None);
    }
}
