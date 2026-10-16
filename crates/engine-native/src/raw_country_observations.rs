//! Callback-local authority for the exact signed raw country operand.
//! Nonzero raw words index live tags; zero uses the database. Arithmetic
//! country lookup is independent of tag spelling. Original attempts,
//! including failures and policy-limited string prefixes, survive family
//! output refusal. No pointer or live control is replay data.

use crate::{Memory, ccountry, cpdx_array::Array, msvc_string, read, walk::Outcome};
use std::mem::size_of;

pub type Field<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub occurrences: usize,
    pub links: usize,
    pub demands: usize,
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
    /// A bounded Text policy rejection is completable; native faults are final.
    pub policy_limited: bool,
    pub failure: Option<&'static str>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapping {
    Unreached,
    Refused(&'static str),
    Observed {
        canonical_slot: u32,
        country_id: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Unreached,
    Refused(&'static str),
    Bytes(Span),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Demand {
    pub raw: i32,
    pub mapping: Mapping,
    pub tag: Tag,
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
    pub base: usize,
    pub state: usize,
    pub controls: Option<Field<[Array; 2]>>,
    words: &'a [Option<Field<u32>>],
}
impl LinkObservations<'_> {
    pub fn word(&self, raw: u32) -> Option<Field<u32>> {
        self.words.get(raw as usize).copied().flatten()
    }
}
pub struct MapLoan<'a> {
    pub links: LinkObservations<'a>,
    results: &'a mut Vec<Demand>,
    required: &'a mut Capacity,
    failed: &'a mut Option<&'static str>,
    growing: bool,
}
impl MapLoan<'_> {
    pub fn admit(&mut self, raw: u32) -> Field<()> {
        if self.results.iter().any(|entry| entry.raw == raw as i32) {
            return Ok(());
        }
        self.required.demands = self.required.demands.saturating_add(1);
        if self.growing && crate::allocation::reserve_push_batch(self.results, 1).is_err() {
            self.failed
                .get_or_insert("raw country demand allocation failed");
            return Err("raw country demand allocation failed");
        }
        if self.results.len() == self.results.capacity() {
            self.failed
                .get_or_insert("raw country demand arena exhausted");
            return Err("raw country demand arena exhausted");
        }
        self.results.push(Demand {
            raw: raw as i32,
            mapping: Mapping::Unreached,
            tag: Tag::Unreached,
        });
        Ok(())
    }
    pub fn observe(&mut self, raw: u32, result: Field<ccountry::Country>) {
        let raw = raw as i32;
        let index = match self.results.iter().position(|entry| entry.raw == raw) {
            Some(index) => index,
            None => {
                self.required.demands = self.required.demands.saturating_add(1);
                if self.growing && crate::allocation::reserve_push_batch(self.results, 1).is_err() {
                    self.failed
                        .get_or_insert("raw country demand allocation failed");
                    return;
                }
                if self.results.len() == self.results.capacity() {
                    self.failed
                        .get_or_insert("raw country demand arena exhausted");
                    return;
                }
                self.results.push(Demand {
                    raw,
                    mapping: Mapping::Unreached,
                    tag: Tag::Unreached,
                });
                self.results.len() - 1
            }
        };
        if matches!(self.results[index].mapping, Mapping::Unreached) {
            self.results[index].mapping = match result {
                Ok(country) => Mapping::Observed {
                    canonical_slot: country.canonical_slot,
                    country_id: country.id,
                },
                Err(reason) => Mapping::Refused(reason),
            };
        }
    }
}

pub struct Journal {
    base: usize,
    state: usize,
    hour: i32,
    occurrences: Vec<Occurrence>,
    links: Vec<Option<Field<u32>>>,
    controls: Option<Field<[Array; 2]>>,
    tags: Option<Field<Array>>,
    demands: Vec<Demand>,
    bytes: Vec<u8>,
    used: usize,
    required: Capacity,
    failed: Option<&'static str>,
    sealed: bool,
    growing: bool,
}
impl Journal {
    pub fn vacant() -> Self {
        Self {
            base: 0,
            state: 0,
            hour: 0,
            occurrences: Vec::new(),
            links: Vec::new(),
            controls: None,
            tags: None,
            demands: Vec::new(),
            bytes: Vec::new(),
            used: 0,
            required: Capacity::default(),
            failed: None,
            sealed: false,
            growing: false,
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
            crate::allocation::reserve_exact(&mut self.occurrences, additional)?;
        }
        if self.demands.capacity() < capacity.demands {
            let additional = capacity.demands.saturating_sub(self.demands.len());
            crate::allocation::reserve_exact(&mut self.demands, additional)?;
        }
        if self.links.len() < capacity.links {
            crate::allocation::resize(&mut self.links, capacity.links, None)?;
        }
        if self.bytes.len() < capacity.bytes {
            crate::allocation::resize(&mut self.bytes, capacity.bytes, 0)?;
        }
        Ok(())
    }
    /// Worker-only acquisitions retain every reached observation and original
    /// tag and country-link operand before publication. Reset keeps the mode;
    /// fixed callback journals never opt in.
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn reset(&mut self) {
        self.occurrences.clear();
        self.links.fill(None);
        self.controls = None;
        self.tags = None;
        self.demands.clear();
        self.used = 0;
        self.required = Capacity::default();
        self.failed = None;
        self.sealed = false;
    }
    pub fn begin(&mut self, base: usize, state: usize, hour: i32) {
        self.reset();
        self.base = base;
        self.state = state;
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
            + self.demands.capacity() * size_of::<Demand>()
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
    pub fn entries(&self) -> &[Demand] {
        &self.demands
    }
    pub fn bytes(&self, span: Span) -> Option<&[u8]> {
        self.bytes
            .get(span.start..span.start.checked_add(span.len)?)
            .filter(|_| span.start + span.len <= self.used)
    }
    pub fn finish_native(&mut self) {
        self.occurrences.clear();
        self.links.fill(None);
        self.controls = None;
        self.tags = None;
        self.base = 0;
        self.state = 0;
    }
    fn occurrence(&mut self, value: Occurrence) -> Option<usize> {
        self.required.occurrences = self.required.occurrences.saturating_add(1);
        if self.growing && crate::allocation::reserve_push_batch(&mut self.occurrences, 1).is_err()
        {
            self.refuse("raw country observation allocation failed");
            return None;
        }
        if self.occurrences.len() == self.occurrences.capacity() {
            self.refuse("raw country observation arena exhausted");
            return None;
        }
        self.occurrences.push(value);
        Some(self.occurrences.len() - 1)
    }
    pub fn table(&mut self, source: Source, outcome: Outcome) {
        self.occurrence(Occurrence::Table { source, outcome });
        if source == Source::Countries
            && self.controls.is_none()
            && let Outcome::Unavailable { reason } = outcome
        {
            self.controls = Some(Err(reason));
        }
    }
    pub fn link_controls(&mut self, result: Field<[Array; 2]>) {
        self.occurrence(Occurrence::LinkControls { result });
        if self.controls.is_none() {
            self.controls = Some(result);
        }
        if let Ok([_, links]) = result {
            self.required.links = self.required.links.max(links.count as usize);
            self.grow_links(links.count as usize);
        }
    }
    pub fn tag_control(&mut self, result: Field<Array>) {
        self.occurrence(Occurrence::TagControl { result });
        if self.tags.is_none() {
            self.tags = Some(result);
        }
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
        self.required.links = self.required.links.max((raw as usize).saturating_add(1));
        self.grow_links((raw as usize).saturating_add(1));
        match self.links.get_mut(raw as usize) {
            Some(slot) => {
                if slot.is_none() {
                    *slot = Some(result);
                }
            }
            None => self.refuse("raw country link observation arena exhausted"),
        }
    }
    fn grow_links(&mut self, required: usize) {
        if self.growing
            && required > self.links.len()
            && crate::allocation::resize(&mut self.links, required, None).is_err()
        {
            self.refuse("raw country link allocation failed");
        }
    }
    /// Admit storage BEFORE the original native attempt; failure poisons later
    /// publication but never suppresses or repeats the established reader.
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
    fn demand_index(&mut self, raw: i32) -> Field<usize> {
        if let Some(index) = self.demands.iter().position(|entry| entry.raw == raw) {
            return Ok(index);
        }
        self.required.demands = self.required.demands.saturating_add(1);
        if self.growing && crate::allocation::reserve_push_batch(&mut self.demands, 1).is_err() {
            self.refuse("raw country demand allocation failed");
            return Err("raw country demand allocation failed");
        }
        if self.demands.len() == self.demands.capacity() {
            self.refuse("raw country demand arena exhausted");
            return Err("raw country demand arena exhausted");
        }
        self.demands.push(Demand {
            raw,
            mapping: Mapping::Unreached,
            tag: Tag::Unreached,
        });
        Ok(self.demands.len() - 1)
    }
    fn grow_bytes(&mut self, required: usize) -> Field<()> {
        let capacity = required.max(self.bytes.len().saturating_mul(2));
        if self.growing
            && required > self.bytes.len()
            && crate::allocation::resize(&mut self.bytes, capacity, 0).is_err()
        {
            self.refuse("raw country byte allocation failed");
            return Err("raw country byte allocation failed");
        }
        Ok(())
    }
    pub fn demand_tag(&mut self, memory: &impl Memory, raw: i32) {
        let Ok(index) = self.demand_index(raw) else {
            return;
        };
        if !matches!(self.demands[index].tag, Tag::Unreached) {
            return;
        }
        let result = self.acquire_tag(memory, raw);
        self.demands[index].tag = match result {
            Ok(span) => Tag::Bytes(span),
            Err(reason) => Tag::Refused(reason),
        };
    }
    fn acquire_tag(&mut self, memory: &impl Memory, raw: i32) -> Field<Span> {
        // Never reinterpret a negative raw word as a canonical country or zero.
        if raw < 0 {
            return Err("negative raw country tag is unsupported");
        }
        // A selecting header fault for this already reached subject precedes
        // any later original success and cannot authorize a fresh attempt.
        for occurrence in &self.occurrences {
            match occurrence {
                Occurrence::TagSelection {
                    raw: subject,
                    result: Err(reason),
                    ..
                } if *subject == raw => return Err(*reason),
                Occurrence::Text { raw: subject, .. } if *subject == raw => break,
                _ => {}
            }
        }
        if let Some((index, mut attempted)) =
            self.occurrences
                .iter()
                .enumerate()
                .find_map(|(index, value)| match value {
                    Occurrence::Text {
                        raw: subject,
                        attempted,
                        ..
                    } if *subject == raw => Some((index, *attempted)),
                    _ => None,
                })
        {
            if let Attempt::Completed(result) = attempted.payload {
                return result;
            }
            if let Some(reason) = attempted.failure {
                return Err(reason);
            }
            // A bounded original may stop after len/cap. Later original
            // readers can already have reached the remaining native stages;
            // consume their owned result or compatible controls once. A
            // later fault or changed control must never be retried here.
            {
                for occurrence in &self.occurrences[index + 1..] {
                    match occurrence {
                        Occurrence::TagSelection {
                            raw: subject,
                            result: Err(reason),
                            ..
                        } if *subject == raw => return Err(*reason),
                        Occurrence::Text {
                            raw: subject,
                            attempted: later,
                            ..
                        } if *subject == raw => {
                            if let Some(reason) = later.failure {
                                return Err(reason);
                            }
                            if later.address != attempted.address
                                || later.length != attempted.length
                                || later.capacity != attempted.capacity
                            {
                                return Err("raw country original tag controls changed");
                            }
                            if let Attempt::Completed(result) = later.header {
                                result?;
                                if matches!(attempted.header, Attempt::Completed(_))
                                    && attempted.header != later.header
                                {
                                    return Err("raw country original tag controls changed");
                                }
                                attempted.header = later.header;
                            }
                            if let Attempt::Completed(result) = later.source {
                                result?;
                                if matches!(attempted.source, Attempt::Completed(_))
                                    && attempted.source != later.source
                                {
                                    return Err("raw country original tag controls changed");
                                }
                                attempted.source = later.source;
                            }
                            if let Attempt::Completed(result) = later.terminator {
                                result?;
                                if matches!(attempted.terminator, Attempt::Completed(_))
                                    && attempted.terminator != later.terminator
                                {
                                    return Err("raw country original tag controls changed");
                                }
                                attempted.terminator = later.terminator;
                            }
                            if let Attempt::Completed(result) = later.payload {
                                return result;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if self.failed.is_some() {
                return Err("raw country original observation incomplete");
            }
            let result = self.complete_text(memory, &mut attempted);
            attempted.payload = Attempt::Completed(result);
            attempted.failure = result.err();
            self.finish_text(Some(index), attempted, None);
            return result;
        }
        if self.failed.is_some() {
            return Err("raw country original observation incomplete");
        }
        if raw == 0 {
            return Err("raw zero tag original observation unreached");
        }
        let tags = self
            .tags
            .ok_or("raw country selecting tag array unobserved")??;
        let address = tags
            .element(raw as u32, msvc_string::STRIDE)
            .ok_or("raw country tag outside live array")?;
        let admitted = self.begin_text(raw, Source::TemplateFallback, address);
        if admitted.is_none() {
            return Err("raw country observation arena exhausted");
        }
        let mut attempt = TextAttempt {
            address,
            ..Default::default()
        };
        let result = (|| {
            let length = read::<u64>(
                memory,
                address
                    .checked_add(msvc_string::LENGTH)
                    .ok_or("raw country tag address overflow")?,
            )
            .ok_or("raw country tag length unreadable");
            attempt.length = Attempt::Completed(length);
            length?;
            let capacity = read::<u64>(
                memory,
                address
                    .checked_add(msvc_string::CAPACITY)
                    .ok_or("raw country tag address overflow")?,
            )
            .ok_or("raw country tag capacity unreadable");
            attempt.capacity = Attempt::Completed(capacity);
            capacity?;
            self.complete_text(memory, &mut attempt)
        })();
        attempt.payload = Attempt::Completed(result);
        attempt.failure = result.err();
        self.finish_text(admitted, attempt, None);
        result
    }
    fn complete_text(&mut self, memory: &impl Memory, attempt: &mut TextAttempt) -> Field<Span> {
        let length = match attempt.length {
            Attempt::Completed(value) => value?,
            Attempt::Unreached => return Err("raw country tag length unreached"),
        };
        let capacity = match attempt.capacity {
            Attempt::Completed(value) => value?,
            Attempt::Unreached => return Err("raw country tag capacity unreached"),
        };
        if length > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("raw country tag layout invalid");
        }
        let len = usize::try_from(length).map_err(|_| "raw country tag length overflow")?;
        self.required.bytes = self.required.bytes.saturating_add(len);
        let end = self
            .used
            .checked_add(len)
            .ok_or("raw country byte demand overflow")?;
        // No new native controls or payload are attempted without admitted bytes.
        self.grow_bytes(end)?;
        if end > self.bytes.len() {
            return Err("raw country byte arena exhausted");
        }
        let source = match attempt.source {
            Attempt::Completed(value) => value?,
            Attempt::Unreached if capacity == msvc_string::INLINE_CAPACITY => attempt.address,
            Attempt::Unreached => {
                let value = read::<u64>(memory, attempt.address)
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value != 0)
                    .ok_or("raw country tag pointer unreadable or invalid");
                attempt.source = Attempt::Completed(value);
                value?
            }
        };
        attempt.source = Attempt::Completed(Ok(source));
        let terminator = match attempt.terminator {
            Attempt::Completed(value) => value?,
            Attempt::Unreached => {
                let value = read::<u8>(
                    memory,
                    source
                        .checked_add(len)
                        .ok_or("raw country tag payload address overflow")?,
                )
                .ok_or("raw country tag terminator unreadable");
                attempt.terminator = Attempt::Completed(value);
                value?
            }
        };
        if terminator != 0 {
            return Err("raw country tag terminator invalid");
        }
        if capacity == msvc_string::INLINE_CAPACITY
            && let Attempt::Completed(Ok(header)) = attempt.header
        {
            self.bytes[self.used..end].copy_from_slice(&header[..len]);
        } else if !crate::read_slice(memory, source, &mut self.bytes[self.used..end]) {
            return Err("raw country tag payload unreadable");
        }
        let span = Span {
            start: self.used,
            len,
        };
        self.used = end;
        Ok(span)
    }
    /// Immutable loan of the first original selecting controls and link attempts.
    pub fn link_observations(&self) -> LinkObservations<'_> {
        LinkObservations {
            base: self.base,
            state: self.state,
            controls: self.controls,
            words: &self.links,
        }
    }

    pub fn map_loan(&mut self) -> MapLoan<'_> {
        MapLoan {
            links: LinkObservations {
                base: self.base,
                state: self.state,
                controls: self.controls,
                words: &self.links,
            },
            results: &mut self.demands,
            required: &mut self.required,
            failed: &mut self.failed,
            growing: self.growing,
        }
    }
    pub fn sort(&mut self) {
        self.demands.sort_unstable_by_key(|entry| entry.raw);
    }
}

#[cfg(test)]
mod tests;
