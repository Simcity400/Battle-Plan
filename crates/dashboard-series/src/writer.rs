use crate::{
    add,
    budget::{Budget, Slab},
    cache::Cache,
    codec::{Binary, Encode},
    format::{Definition, Meta, NodeRef, Root, State},
    hierarchy, invalid,
    pages::{PageRead, PageWrite},
    query::EventView,
    read::Reader,
    seal::{Pending, Sealed},
    types::*,
};
use std::{collections::HashMap, io, io::Read};

const NO_ROW: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SpecKey {
    schema: u64,
    actor: u64,
    measure: u64,
    len: u64,
    hash: [u8; 32],
}

/// One key's carried state: what its next append continues or must follow.
#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub mode: RunMode,
    pub decimals: u8,
    pub domain: u64,
    pub defined: u64,
    pub canonicals: u64,
    pub rows: u64,
    pub last: Option<Run>,
    pub evidence: Option<Evidence>,
    pub known: Option<i32>,
    row: u32,
    touched: bool,
}
impl Edge {
    fn new(mode: RunMode, decimals: u8, domain: u64, defined: u64) -> Self {
        Self {
            mode,
            decimals,
            domain,
            defined,
            canonicals: 0,
            rows: 0,
            last: None,
            evidence: None,
            known: None,
            row: NO_ROW,
            touched: false,
        }
    }
}
/// A shared evidence domain's carried state.
#[derive(Clone, Copy)]
pub(crate) struct DomainEdge {
    pub evidence: Option<Evidence>,
    pub known: Option<i32>,
    last: Option<(DomainRow, Evidence)>,
    touched: bool,
}

/// A shared evidence domain row as [`Writer::domain`] appended it, for presenting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainRow {
    domain: u64,
    segment: u64,
    row: u32,
}

/// A key's (or domain's) state when the segment first touched it, and at the cut.
#[derive(Clone, Copy)]
pub(crate) struct Touch<T> {
    pub id: u64,
    pub start: T,
    pub end: T,
}

#[derive(Clone, Copy)]
pub(crate) struct RunRow {
    pub key: u64,
    pub span: Span,
    pub value: i128,
    pub break_before: bool,
    pub supplying_boundary: bool,
    pub continues: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct EvidenceRow {
    pub subject: u64,
    pub value: Evidence,
}

/// The rows one segment appended, in append order.
pub(crate) struct Columns<'a> {
    pub runs: Slab<'a, RunRow>,
    pub evidence: Slab<'a, EvidenceRow>,
    pub domain_rows: Slab<'a, EvidenceRow>,
    pub events: Slab<'a, EventView>,
    pub keys: Slab<'a, Touch<Edge>>,
    pub domains: Slab<'a, Touch<DomainEdge>>,
    pub definitions: Slab<'a, Definition>,
    pub metadata: Slab<'a, Meta>,
    pub heap: Slab<'a, u8>,
    pub last: i32,
}
impl<'a> Columns<'a> {
    fn new(budget: &'a Budget) -> io::Result<Self> {
        Ok(Self {
            runs: Slab::new(budget, 0)?,
            evidence: Slab::new(budget, 0)?,
            domain_rows: Slab::new(budget, 0)?,
            events: Slab::new(budget, 0)?,
            keys: Slab::new(budget, 0)?,
            domains: Slab::new(budget, 0)?,
            definitions: Slab::new(budget, 0)?,
            metadata: Slab::new(budget, 0)?,
            heap: Slab::new(budget, 0)?,
            last: i32::MIN,
        })
    }
}

/// A family's serial emit. Appends go to in-memory columns: no pages, trees or
/// hashing. [`Writer::cut`] hands the finished segment to a sealing job that can
/// run on any thread; [`Writer::publish`] adds sealed segments, in order, to the
/// published hierarchy.
pub struct Writer<'a> {
    budget: &'a Budget,
    identity: Identity,
    first: i32,
    state: State,
    published: Option<Publication>,
    segment: u64,
    start: Option<i32>,
    sequences: u64,
    heap: u64,
    keys: Vec<Edge>,
    specs: HashMap<SpecKey, u64>,
    domain_ids: HashMap<u64, u32>,
    domains: Vec<DomainEdge>,
    columns: Columns<'a>,
    nodes: Vec<NodeRef>,
    sealed: u64,
}

impl<'a> Writer<'a> {
    /// The bytes its appended rows hold of its budget until it is cut.
    pub fn held_bytes(&self) -> usize {
        let columns = &self.columns;
        [
            columns.runs.held(),
            columns.evidence.held(),
            columns.domain_rows.held(),
            columns.events.held(),
            columns.keys.held(),
            columns.domains.held(),
            columns.definitions.held(),
            columns.metadata.held(),
            columns.heap.held(),
        ]
        .into_iter()
        .fold(0, usize::saturating_add)
    }

    pub fn new(budget: &'a Budget, identity: Identity, first: i32) -> io::Result<Self> {
        Ok(Self {
            budget,
            identity,
            first,
            state: State {
                publication: Publication {
                    generation: 0,
                    segment: 0,
                    accepted: Span { first, last: first },
                    reached: first,
                    refusal: None,
                },
                next_key: 0,
                next_event: 0,
                heap: 0,
                checkpoint: Blob::EMPTY,
            },
            published: None,
            segment: 0,
            start: Some(first),
            sequences: 0,
            heap: 0,
            keys: Vec::new(),
            specs: HashMap::new(),
            domain_ids: HashMap::new(),
            domains: Vec::new(),
            columns: Columns::new(budget)?,
            nodes: Vec::new(),
            sealed: 0,
        })
    }

    /// Continues the publication rooted at `root`, read through `cache`: keys, counters and edges
    /// come from its hierarchy, and its rows draw on `budget`.
    pub fn resume(
        io: &mut impl PageRead,
        cache: &Cache<'_>,
        budget: &'a Budget,
        root: PageId,
        identity: Identity,
    ) -> io::Result<Self> {
        let mut reader = Reader::open(io, cache, root)?;
        if reader.root().identity != identity {
            return Err(invalid("series source/content/branch/schema changed"));
        }
        let root: Root = reader.root().clone();
        let mut writer = Self::new(
            budget,
            identity,
            root.state.publication.accepted.first,
        )?;
        writer.state = root.state;
        writer.published = Some(root.state.publication);
        writer.start = root.state.publication.accepted.last.checked_add(1);
        writer.sequences = root.state.next_event;
        writer.heap = root.state.heap;
        writer.segment = root
            .nodes
            .iter()
            .try_fold(0, |n, node| add(n, node.segments))?;
        writer.sealed = writer.segment;
        writer.nodes = root.nodes;
        reader.definitions(io, &mut |definition| writer.restore(definition))?;
        if writer.keys.len() as u64 != root.state.next_key {
            return Err(invalid("series key count"));
        }
        let end = root.state.publication.accepted.last;
        let segments = writer.segment;
        let keys = &mut writer.keys;
        reader.latest_entries(io, false, &mut |entry, last| {
            let edge = usize::try_from(entry.key)
                .ok()
                .and_then(|key| keys.get_mut(key))
                .ok_or_else(|| invalid("series entry for an undefined key"))?;
            edge.canonicals = entry.canonical_end;
            edge.rows = add(entry.row_start, entry.runs.count)?;
            if last {
                if let Some(detail) = entry.detail {
                    edge.last = detail.tail;
                    edge.evidence = detail.evidence_tail;
                }
                edge.known =
                    (entry.coverage.count != 0 && entry.coverage.last == end).then_some(end);
            }
            Ok(())
        })?;
        reader.latest_entries(io, true, &mut |entry, last| {
            let index = writer.domain_index(entry.key)?;
            let domain = &mut writer.domains[index];
            if let Some(row) = entry.last_row {
                if row.segment >= segments || row.value.span.last > end {
                    return Err(invalid("series domain row outside its publication"));
                }
                domain.last = Some((
                    DomainRow {
                        domain: entry.key,
                        segment: row.segment,
                        row: row.row,
                    },
                    row.value,
                ));
            }
            if last {
                domain.evidence = entry.detail.and_then(|detail| detail.evidence_tail);
                domain.known =
                    (entry.coverage.count != 0 && entry.coverage.last == end).then_some(end);
            }
            Ok(())
        })?;
        Ok(writer)
    }
    /// Takes back one published key definition, in key order.
    fn restore(&mut self, definition: Definition) -> io::Result<()> {
        if definition.key.id != self.keys.len() as u64 {
            return Err(invalid("series key definitions are not dense"));
        }
        self.specs.insert(
            SpecKey {
                schema: definition.key.spec.schema,
                actor: definition.key.spec.actor,
                measure: definition.key.spec.measure,
                len: definition.key.spec.dimensions.len,
                hash: definition.dimensions,
            },
            definition.key.id,
        );
        engine_allocation::reserve(&mut self.keys, 1)?;
        self.keys.push(Edge::new(
            definition.mode,
            definition.decimals,
            definition.domain,
            NONE,
        ));
        if definition.domain != NONE {
            self.domain_index(definition.domain)?;
        }
        Ok(())
    }

    pub fn sequence(&mut self) -> io::Result<u64> {
        let sequence = self.state.next_event;
        self.state.next_event = add(sequence, 1)?;
        Ok(sequence)
    }

    fn start(&self) -> io::Result<i32> {
        self.start
            .ok_or_else(|| invalid("series publication already reaches the last hour"))
    }
    // Appends check, then admit every row they add, then commit: a refusal changes nothing.
    fn placed(&self, span: Span) -> io::Result<()> {
        Span::new(span.first, span.last)?;
        if span.first < self.start()? {
            return Err(invalid("series row precedes its segment"));
        }
        Ok(())
    }
    fn key(&self, key: KeyHandle) -> io::Result<usize> {
        usize::try_from(key)
            .ok()
            .filter(|index| *index < self.keys.len())
            .ok_or_else(|| invalid("unknown series key"))
    }
    fn admit_touch(&mut self, index: usize) -> io::Result<()> {
        if self.keys[index].touched {
            return Ok(());
        }
        self.columns.keys.reserve(1)
    }
    /// Records the key's state before the segment's first append to it.
    fn touch(&mut self, index: usize) {
        if !self.keys[index].touched {
            self.keys[index].touched = true;
            let edge = self.keys[index];
            self.columns.keys.put(Touch {
                id: index as u64,
                start: edge,
                end: edge,
            });
        }
    }
    fn domain_slot(&self, id: u64) -> io::Result<Option<usize>> {
        if id == NONE {
            return Err(invalid("shared evidence domain identifier is reserved"));
        }
        Ok(self.domain_ids.get(&id).map(|&index| index as usize))
    }
    fn admit_domain(&mut self) -> io::Result<()> {
        u32::try_from(self.domains.len()).map_err(|_| invalid("series domain count"))?;
        engine_allocation::reserve(&mut self.domains, 1)?;
        engine_allocation::reserve_map(&mut self.domain_ids, 1)
    }
    fn insert_domain(&mut self, id: u64) -> usize {
        let index = self.domains.len();
        self.domains.push(DomainEdge {
            evidence: None,
            known: None,
            last: None,
            touched: false,
        });
        self.domain_ids.insert(id, index as u32);
        index
    }
    fn domain_index(&mut self, id: u64) -> io::Result<usize> {
        if let Some(index) = self.domain_slot(id)? {
            return Ok(index);
        }
        self.admit_domain()?;
        Ok(self.insert_domain(id))
    }
    fn touch_domain(&mut self, index: usize, id: u64) {
        if !self.domains[index].touched {
            self.domains[index].touched = true;
            let edge = self.domains[index];
            self.columns.domains.put(Touch {
                id,
                start: edge,
                end: edge,
            });
        }
    }
    fn reserved(&self, sequence: u64) -> io::Result<()> {
        if sequence < self.sequences || sequence >= self.state.next_event {
            return Err(invalid("unreserved publication sequence"));
        }
        Ok(())
    }

    /// Interns a structured key in first-seen order. Its dimensions must be
    /// written in the current segment, so their identity is their content.
    pub fn intern(
        &mut self,
        spec: KeySpec,
        label: Blob,
        mode: RunMode,
        decimals: u8,
    ) -> io::Result<KeyHandle> {
        let hash = if spec.dimensions.len == 0 {
            [0; 32]
        } else {
            *blake3::hash(self.held(spec.dimensions)?).as_bytes()
        };
        let lookup = SpecKey {
            schema: spec.schema,
            actor: spec.actor,
            measure: spec.measure,
            len: spec.dimensions.len,
            hash,
        };
        if let Some(&key) = self.specs.get(&lookup) {
            let edge = self.keys[key as usize];
            if edge.mode != mode || edge.decimals != decimals {
                return Err(invalid("key mode/decimal scale changed"));
            }
            return Ok(key);
        }
        let id = self.state.next_key;
        if id == NONE {
            return Err(invalid("series key IDs exhausted"));
        }
        let next = add(id, 1)?;
        self.columns.definitions.reserve(1)?;
        engine_allocation::reserve(&mut self.keys, 1)?;
        engine_allocation::reserve_map(&mut self.specs, 1)?;
        self.columns.definitions.put(Definition {
            key: KeyDefinition {
                id,
                spec,
                label,
                first_seen: id,
            },
            dimensions: hash,
            mode,
            decimals,
            domain: NONE,
        });
        self.keys
            .push(Edge::new(mode, decimals, NONE, self.segment));
        self.specs.insert(lookup, id);
        self.state.next_key = next;
        Ok(id)
    }
    /// Binds a key defined in this segment, before any evidence, to a shared domain.
    pub fn bind_domain(&mut self, key: KeyHandle, domain: u64) -> io::Result<()> {
        let index = self.key(key)?;
        let edge = self.keys[index];
        if edge.domain != NONE && edge.domain != domain {
            return Err(invalid("a key cannot reinterpret its evidence domain"));
        }
        if edge.defined != self.segment {
            return Err(invalid(
                "bind a key's evidence domain in the segment that defines it",
            ));
        }
        if edge.evidence.is_some() {
            return Err(invalid("key already has independent evidence"));
        }
        self.domain_index(domain)?;
        let first = self.columns.definitions[0].key.id;
        self.columns.definitions[(key - first) as usize].domain = domain;
        self.keys[index].domain = domain;
        Ok(())
    }

    pub fn run(
        &mut self,
        key: KeyHandle,
        span: Span,
        value: i128,
        break_before: bool,
        supplying_boundary: bool,
    ) -> io::Result<()> {
        let index = self.key(key)?;
        let edge = self.keys[index];
        if edge.mode == RunMode::Exact && edge.last.is_some_and(|p| p.span.last >= span.first) {
            return Err(invalid("ordinary exact runs must be ordered and disjoint"));
        }
        self.placed(span)?;
        let tail = edge.last.filter(|p| {
            !break_before
                && !supplying_boundary
                && p.value == value
                && i64::from(p.span.last) + 1 == i64::from(span.first)
        });
        if let Some(mut tail) = tail.filter(|_| edge.row != NO_ROW) {
            tail.span.last = span.last;
            self.columns.last = self.columns.last.max(span.last);
            self.columns.runs[edge.row as usize].span.last = span.last;
            self.keys[index].last = Some(tail);
            return Ok(());
        }
        let rows = add(edge.rows, 1)?;
        let (last, canonicals) = match tail {
            Some(mut tail) => {
                tail.span.last = span.last;
                (tail, edge.canonicals)
            }
            None => (
                Run {
                    span,
                    value,
                    break_before,
                    supplying_boundary,
                    canonical: edge.canonicals,
                    sequence: edge.rows,
                },
                add(edge.canonicals, 1)?,
            ),
        };
        let row = u32::try_from(self.columns.runs.len())
            .ok()
            .filter(|row| *row != NO_ROW)
            .ok_or_else(|| invalid("series segment rows"))?;
        self.admit_touch(index)?;
        self.columns.runs.reserve(1)?;
        self.touch(index);
        self.columns.last = self.columns.last.max(span.last);
        let continues = tail.is_some();
        self.columns.runs.put(RunRow {
            key,
            span,
            value,
            break_before: break_before && !continues,
            supplying_boundary: supplying_boundary && !continues,
            continues,
        });
        let edge = &mut self.keys[index];
        edge.last = Some(last);
        edge.canonicals = canonicals;
        edge.rows = rows;
        edge.row = row;
        Ok(())
    }

    pub fn evidence(
        &mut self,
        key: KeyHandle,
        value: Evidence,
        presentation: Option<u64>,
    ) -> io::Result<()> {
        let index = self.key(key)?;
        let edge = self.keys[index];
        if edge.domain != NONE {
            return Err(invalid("domain-bound key has independent evidence"));
        }
        if edge
            .evidence
            .is_some_and(|p| p.span.last >= value.span.first)
        {
            return Err(invalid("evidence must be ordered and disjoint"));
        }
        if let Some(sequence) = presentation {
            self.reserved(sequence)?;
        }
        self.placed(value.span)?;
        self.admit_touch(index)?;
        self.columns.evidence.reserve(1)?;
        if presentation.is_some() {
            self.columns.events.reserve(1)?;
        }
        self.touch(index);
        self.columns.last = self.columns.last.max(value.span.last);
        let edge = &mut self.keys[index];
        edge.evidence = Some(follow(edge.evidence, value));
        if value.known == Knownness::Known {
            edge.known = Some(value.span.last);
        }
        self.columns.evidence.put(EvidenceRow {
            subject: key,
            value,
        });
        if let Some(sequence) = presentation {
            self.columns.events.put(EventView::Evidence {
                key,
                sequence,
                value,
            });
        }
        Ok(())
    }

    /// Appends a row of a shared evidence domain.
    pub fn domain(&mut self, id: u64, value: Evidence) -> io::Result<DomainRow> {
        let index = self.domain_slot(id)?;
        if index.is_some_and(|index| {
            self.domains[index]
                .evidence
                .is_some_and(|p| p.span.last >= value.span.first)
        }) {
            return Err(invalid("overlapping evidence extents"));
        }
        self.placed(value.span)?;
        let row = DomainRow {
            domain: id,
            segment: self.segment,
            row: u32::try_from(self.columns.domain_rows.len())
                .map_err(|_| invalid("series segment rows"))?,
        };
        if index.is_none() {
            self.admit_domain()?;
        }
        if !index.is_some_and(|index| self.domains[index].touched) {
            self.columns.domains.reserve(1)?;
        }
        self.columns.domain_rows.reserve(1)?;
        let index = match index {
            Some(index) => index,
            None => self.insert_domain(id),
        };
        self.touch_domain(index, id);
        self.columns.last = self.columns.last.max(value.span.last);
        self.columns
            .domain_rows
            .put(EvidenceRow { subject: id, value });
        let edge = &mut self.domains[index];
        edge.evidence = Some(follow(edge.evidence, value));
        if value.known == Knownness::Known {
            edge.known = Some(value.span.last);
        }
        edge.last = Some((row, value));
        Ok(row)
    }

    /// Presents a domain row appended in this segment, or the domain's last row.
    pub fn present_domain(
        &mut self,
        key: KeyHandle,
        row: DomainRow,
        sequence: u64,
    ) -> io::Result<()> {
        let index = self.key(key)?;
        if self.keys[index].domain != row.domain {
            return Err(invalid(
                "a presentation must read the key's own evidence domain",
            ));
        }
        self.reserved(sequence)?;
        let value = if row.segment == self.segment {
            self.columns
                .domain_rows
                .get(row.row as usize)
                .filter(|held| held.subject == row.domain)
                .map(|held| held.value)
        } else {
            self.domain_ids
                .get(&row.domain)
                .and_then(|&at| self.domains[at as usize].last)
                .filter(|(last, _)| *last == row)
                .map(|(_, value)| value)
        }
        .ok_or_else(|| invalid("missing shared read presentation"))?;
        self.admit_touch(index)?;
        self.columns.events.reserve(1)?;
        self.touch(index);
        self.columns.events.put(EventView::Evidence {
            key,
            sequence,
            value,
        });
        Ok(())
    }

    pub fn event(&mut self, value: Event) -> io::Result<()> {
        if value.fields.len % Cell::SIZE as u64 != 0 {
            return Err(invalid("event key/schema fields"));
        }
        self.reserved(value.sequence)?;
        let index = self.key(value.key)?;
        self.admit_touch(index)?;
        self.columns.events.reserve(1)?;
        self.touch(index);
        self.columns.events.put(EventView::Family(value));
        Ok(())
    }

    pub fn catalogue(&mut self, id: u64, value: Cell) -> io::Result<()> {
        self.columns.metadata.push(Meta { id, value })
    }

    fn held(&self, blob: Blob) -> io::Result<&[u8]> {
        let start = blob
            .at
            .checked_sub(self.heap)
            .and_then(|at| usize::try_from(at).ok())
            .ok_or_else(|| invalid("key dimensions must be written in the current segment"))?;
        let end = usize::try_from(blob.len)
            .ok()
            .and_then(|len| start.checked_add(len))
            .filter(|end| *end <= self.columns.heap.len())
            .ok_or_else(|| invalid("key dimensions must be written in the current segment"))?;
        Ok(&self.columns.heap[start..end])
    }
    pub fn blob(&mut self, input: &mut impl Read, len: u64) -> io::Result<Blob> {
        if len == 0 {
            return Ok(Blob::EMPTY);
        }
        let held = self.columns.heap.len();
        let at = add(self.heap, held as u64)?;
        add(at, len)?;
        self.columns
            .heap
            .reserve(usize::try_from(len).map_err(|_| engine_allocation::oom())?)?;
        let mut chunk = [0; 4096];
        let mut left = len;
        while left > 0 {
            let count = left.min(chunk.len() as u64) as usize;
            let read = input
                .read_exact(&mut chunk[..count])
                .and_then(|()| self.columns.heap.extend(&chunk[..count]));
            if let Err(error) = read {
                self.columns.heap.truncate(held);
                return Err(error);
            }
            left -= count as u64;
        }
        Ok(Blob { at, len })
    }
    pub fn text(&mut self, text: &str) -> io::Result<Blob> {
        self.blob(&mut text.as_bytes(), text.len() as u64)
    }
    pub fn cells(&mut self, input: impl IntoIterator<Item = Cell>) -> io::Result<Blob> {
        let held = self.columns.heap.len();
        let at = add(self.heap, held as u64)?;
        let mut len = 0_u64;
        for cell in input {
            let mut bytes = [0; Cell::SIZE];
            let encoded = cell.encode(&mut Encode::new(&mut bytes));
            let appended = encoded
                .and_then(|()| self.columns.heap.extend(&bytes))
                .and_then(|()| add(len, Cell::SIZE as u64));
            match appended {
                Ok(next) => len = next,
                Err(error) => {
                    self.columns.heap.truncate(held);
                    return Err(error);
                }
            }
        }
        Ok(if len == 0 {
            Blob::EMPTY
        } else {
            Blob { at, len }
        })
    }

    /// Closes the segment: its rows must lie within the publication's accepted hours.
    /// The returned job seals pages on any thread; the emit continues at once.
    pub fn cut(&mut self, publication: Publication, checkpoint: Blob) -> io::Result<Pending<'a>> {
        let start = self.start()?;
        Span::new(publication.accepted.first, publication.accepted.last)?;
        if publication.reached < publication.accepted.last
            || publication.accepted.first != self.first
            || publication.accepted.last < start
            || self.published.is_some_and(|p| {
                publication.generation <= p.generation || publication.segment <= p.segment
            })
        {
            return Err(invalid("invalid series publication successor"));
        }
        let natives = self.columns.events.iter().map(|e| e.native_hour()).max();
        if self.columns.last > publication.accepted.last
            || natives.is_some_and(|hour| hour > publication.accepted.last)
        {
            return Err(invalid("series rows outside their published segment"));
        }
        let held = add(self.heap, self.columns.heap.len() as u64)?;
        if checkpoint.len != 0
            && checkpoint
                .at
                .checked_add(checkpoint.len)
                .is_none_or(|end| end > held)
        {
            return Err(invalid("series checkpoint outside the heap"));
        }
        let segment = add(self.segment, 1)?;
        let fresh = Columns::new(self.budget)?;
        for touch in self.columns.keys.iter_mut() {
            let edge = &mut self.keys[touch.id as usize];
            edge.row = NO_ROW;
            edge.touched = false;
            touch.end = *edge;
        }
        for touch in self.columns.domains.iter_mut() {
            let index = self.domain_ids[&touch.id] as usize;
            self.domains[index].touched = false;
            touch.end = self.domains[index];
        }
        let state = State {
            publication,
            heap: held,
            checkpoint,
            ..self.state
        };
        let pending = Pending {
            budget: self.budget,
            index: self.segment,
            hours: Span {
                first: start,
                last: publication.accepted.last,
            },
            state,
            first_key: self
                .columns
                .definitions
                .first()
                .map_or(state.next_key, |d| d.key.id),
            heap_base: self.heap,
            sequences: (self.sequences, state.next_event),
            columns: std::mem::replace(&mut self.columns, fresh),
        };
        self.state = state;
        self.published = Some(publication);
        self.start = publication.accepted.last.checked_add(1);
        self.segment = segment;
        self.sequences = state.next_event;
        self.heap = held;
        Ok(pending)
    }

    /// Adds the next sealed segment to the hierarchy, merging any 16 complete
    /// siblings, and writes the publication's root.
    pub fn publish(
        &mut self,
        io: &mut impl PageWrite,
        cache: &Cache<'_>,
        sealed: Sealed,
    ) -> io::Result<PageId> {
        if sealed.node.first != self.sealed {
            return Err(invalid("series segments must be published in order"));
        }
        let next = add(self.sealed, 1)?;
        let mut nodes = self.nodes.clone();
        nodes.push(sealed.node);
        hierarchy::settle(io, cache, &mut nodes)?;
        let root = hierarchy::write_root(
            io,
            &Root {
                identity: self.identity,
                state: sealed.state,
                nodes: nodes.clone(),
            },
        )?;
        self.nodes = nodes;
        self.sealed = next;
        Ok(root)
    }
}

/// The appended row as carried: a row equal to the last and adjacent continues its span.
fn follow(previous: Option<Evidence>, value: Evidence) -> Evidence {
    match previous {
        Some(p)
            if same_evidence(p, value)
                && i64::from(p.span.last) + 1 == i64::from(value.span.first) =>
        {
            Evidence {
                span: Span {
                    first: p.span.first,
                    last: value.span.last,
                },
                ..value
            }
        }
        _ => value,
    }
}
pub(crate) fn same_evidence(a: Evidence, b: Evidence) -> bool {
    a.known == b.known
        && a.supplying_read == b.supplying_read
        && a.holder == b.holder
        && a.identity_first == b.identity_first
}
