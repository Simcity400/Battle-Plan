use crate::{
    add,
    chunked::Chunked,
    codec::Binary,
    format::{Definition, Held, Meta, NodeRef, Root, State},
    hierarchy::{self, Built, Replaced},
    invalid,
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
    /// What the next cut continues unchanged: 1 its run, 2 its evidence.
    hold: u8,
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
            hold: 0,
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
/// A key the segment holds nothing of but what it continues unchanged.
#[derive(Clone, Copy)]
pub(crate) struct HeldRow {
    pub key: u64,
    pub held: Held,
    pub row_start: u64,
    pub canonicals: u64,
    pub domain: u64,
    /// The held run's value and the key's scale, which merging the segment reads.
    pub value: i128,
    pub decimals: u8,
}
#[derive(Clone, Copy)]
pub(crate) struct EvidenceRow {
    pub subject: u64,
    pub value: Evidence,
}

/// The rows one segment appended, in append order.
#[derive(Default)]
pub(crate) struct Columns {
    pub runs: Chunked<RunRow>,
    pub evidence: Chunked<EvidenceRow>,
    pub domain_rows: Chunked<EvidenceRow>,
    pub events: Chunked<EventView>,
    pub keys: Chunked<Touch<Edge>>,
    /// Keys whose run or evidence the cut continues, in the order asked.
    pub holds: Chunked<u64>,
    /// The last hour every hold goes on through: the last its family's cut covers.
    pub held_through: Option<i32>,
    pub held: Chunked<HeldRow>,
    pub domains: Chunked<Touch<DomainEdge>>,
    pub definitions: Chunked<Definition>,
    pub metadata: Chunked<Meta>,
    pub heap: Chunked<u8>,
    pub last: i32,
}
impl Columns {
    fn new() -> Self {
        Self {
            last: i32::MIN,
            ..Self::default()
        }
    }
}

/// The published hierarchy of a writer's sealed segments, which [`Tree::publish`] adds each next
/// one to. A writer lends it out so the segment it cut seals and publishes on another thread
/// while it appends the next ([`Writer::lend_tree`]).
pub struct Tree {
    identity: Identity,
    nodes: Vec<NodeRef>,
    /// What merging each of `nodes` reads: its header and held runs kept from when it was
    /// written until it is merged, its pages read back then; None for a node of a resumed
    /// hierarchy, read back whole when it merges.
    built: Vec<Option<Built>>,
    sealed: u64,
}

impl Tree {
    fn new(identity: Identity, nodes: Vec<NodeRef>, sealed: u64) -> io::Result<Self> {
        let mut built = engine_allocation::vec(nodes.len())?;
        built.resize_with(nodes.len(), || None);
        Ok(Self {
            identity,
            nodes,
            built,
            sealed,
        })
    }

    /// Adds `sealed` to the hierarchy, any 16 complete siblings merged: what it merged, to undo.
    fn add(&mut self, io: &mut impl PageWrite, sealed: Sealed) -> io::Result<Vec<Replaced>> {
        if sealed.node.first != self.sealed {
            return Err(invalid("series segments must be published in order"));
        }
        engine_allocation::reserve(&mut self.nodes, 1)?;
        engine_allocation::reserve(&mut self.built, 1)?;
        self.nodes.push(sealed.node);
        self.built.push(Some(sealed.built));
        let mut undo = Vec::new();
        match hierarchy::settle(io, &mut self.nodes, &mut self.built, &mut undo) {
            Ok(()) => {
                // Written pages are read back when they merge, every 16 segments a level.
                for built in self.built.iter_mut().flatten() {
                    built.let_go();
                }
                Ok(undo)
            }
            Err(error) => {
                self.undo(undo);
                Err(error)
            }
        }
    }

    /// Takes back the segment [`Self::add`] added and what it merged.
    fn undo(&mut self, undo: Vec<Replaced>) {
        for (nodes, built) in undo.into_iter().rev() {
            self.nodes.pop();
            self.built.pop();
            self.nodes.extend(nodes);
            self.built.extend(built);
        }
        self.nodes.pop();
        self.built.pop();
    }

    /// Lets go of what merging its committed nodes reads: each is read back when it merges, as
    /// a resumed hierarchy's is.
    pub fn release_built(&mut self) {
        self.built.fill_with(|| None);
    }

    /// Adds the next sealed segment to the hierarchy without a root of its own: the root of a
    /// later segment names it ([`Self::publish`]).
    pub fn append(&mut self, io: &mut impl PageWrite, sealed: Sealed) -> io::Result<()> {
        let next = add(self.sealed, 1)?;
        self.add(io, sealed)?;
        self.sealed = next;
        Ok(())
    }

    /// Adds the next sealed segment to the hierarchy, merging any 16 complete
    /// siblings, and writes the publication's root.
    pub fn publish(&mut self, io: &mut impl PageWrite, sealed: Sealed) -> io::Result<PageId> {
        let next = add(self.sealed, 1)?;
        let state = sealed.state;
        let undo = self.add(io, sealed)?;
        let root = engine_allocation::copy_vec(&self.nodes).and_then(|nodes| {
            hierarchy::write_root(
                io,
                &Root {
                    identity: self.identity,
                    state,
                    nodes,
                },
            )
        });
        match root {
            Ok(root) => {
                self.sealed = next;
                Ok(root)
            }
            Err(error) => {
                self.undo(undo);
                Err(error)
            }
        }
    }
}

/// A family's serial emit. Appends go to in-memory columns: no pages, trees or
/// hashing. [`Writer::cut`] hands the finished segment to a sealing job that can
/// run on any thread; [`Writer::publish`] adds sealed segments, in order, to the
/// published hierarchy.
pub struct Writer {
    identity: Identity,
    first: i32,
    state: State,
    published: Option<Publication>,
    segment: u64,
    start: Option<i32>,
    sequences: u64,
    heap: u64,
    keys: Chunked<Edge>,
    specs: HashMap<SpecKey, u64>,
    domain_ids: HashMap<u64, u32>,
    domains: Vec<DomainEdge>,
    columns: Columns,
    /// None while lent out.
    tree: Option<Tree>,
    /// The cells [`Self::cells`] encodes before it appends them at once.
    encoded: Vec<u8>,
}

/// What a writer holds is its family's publication's.
#[inline]
fn publishing() -> engine_allocation::labels::Scope {
    engine_allocation::labels::kind(engine_allocation::labels::Kind::Publish)
}

/// The state it keeps of every key it defined, which grows with the publication's history.
#[inline]
fn keying() -> engine_allocation::labels::Scope {
    engine_allocation::labels::kind(engine_allocation::labels::Kind::Keys)
}

impl Writer {
    /// Whether its open segment holds anything appended since it was last cut.
    pub fn holds(&self) -> bool {
        let c = &self.columns;
        [
            c.runs.len(),
            c.evidence.len(),
            c.domain_rows.len(),
            c.events.len(),
            c.keys.len(),
            c.holds.len(),
            c.domains.len(),
            c.definitions.len(),
            c.metadata.len(),
            c.heap.len(),
        ]
        .into_iter()
        .any(|len| len != 0)
    }

    /// The publication it cut last; None before its first.
    pub fn published(&self) -> Option<Publication> {
        self.published
    }

    /// The bytes its appended rows hold until it is cut.
    pub fn held_bytes(&self) -> usize {
        let columns = &self.columns;
        [
            columns.runs.held(),
            columns.evidence.held(),
            columns.domain_rows.held(),
            columns.events.held(),
            columns.keys.held(),
            columns.holds.held(),
            columns.held.held(),
            columns.domains.held(),
            columns.definitions.held(),
            columns.metadata.held(),
            columns.heap.held(),
        ]
        .into_iter()
        .fold(0, usize::saturating_add)
    }

    pub fn new(identity: Identity, first: i32) -> io::Result<Self> {
        let _memory = publishing();
        Ok(Self {
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
            keys: Chunked::default(),
            specs: HashMap::new(),
            domain_ids: HashMap::new(),
            domains: Vec::new(),
            columns: Columns::new(),
            tree: Some(Tree::new(identity, Vec::new(), 0)?),
            encoded: Vec::new(),
        })
    }

    /// Continues the publication rooted at `root`: keys, counters and edges come from its
    /// hierarchy.
    pub fn resume(io: &mut impl PageRead, root: PageId, identity: Identity) -> io::Result<Self> {
        let _memory = publishing();
        let mut reader = Reader::open(io, root)?;
        if reader.root().identity != identity {
            return Err(invalid("series source/content/branch/schema changed"));
        }
        let root: Root = reader.root().clone();
        let mut writer = Self::new(identity, root.state.publication.accepted.first)?;
        writer.state = root.state;
        writer.published = Some(root.state.publication);
        writer.start = root.state.publication.accepted.last.checked_add(1);
        writer.sequences = root.state.next_event;
        writer.heap = root.state.heap;
        writer.segment = root
            .nodes
            .iter()
            .try_fold(0, |n, node| add(n, node.segments))?;
        writer.tree = Some(Tree::new(identity, root.nodes, writer.segment)?);
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
        let _memory = keying();
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
        self.keys.push(Edge::new(
            definition.mode,
            definition.decimals,
            definition.domain,
            NONE,
        ))?;
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
        let _memory = keying();
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
        let _memory = publishing();
        let hash = if spec.dimensions.len == 0 {
            [0; 32]
        } else {
            let (start, end) = self.held(spec.dimensions)?;
            let mut hasher = blake3::Hasher::new();
            for piece in self.columns.heap.pieces(start, end) {
                hasher.update(piece);
            }
            *hasher.finalize().as_bytes()
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
        {
            let _memory = keying();
            self.keys.reserve(1)?;
            engine_allocation::reserve_map(&mut self.specs, 1)?;
        }
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
            .put(Edge::new(mode, decimals, NONE, self.segment));
        self.specs.insert(lookup, id);
        self.state.next_key = next;
        Ok(id)
    }
    /// Binds a key defined in this segment, before any evidence, to a shared domain.
    pub fn bind_domain(&mut self, key: KeyHandle, domain: u64) -> io::Result<()> {
        let _memory = publishing();
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
        let _memory = publishing();
        let index = self.key(key)?;
        let edge = self.keys[index];
        if edge.hold & 1 != 0 {
            return Err(invalid(
                "a held run takes no rows in the segment holding it",
            ));
        }
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

    /// Continues `key`'s run, carried out of the segment before, unchanged through `through`,
    /// the last hour its family's cut covers. A segment holding nothing else of the key, and
    /// held through its last hour, refers to the run where it began instead of holding a piece.
    pub fn hold(&mut self, key: KeyHandle, through: i32) -> io::Result<()> {
        let index = self.key(key)?;
        let start = self.start()?;
        if !self.keys[index]
            .last
            .is_some_and(|run| i64::from(run.span.last) + 1 == i64::from(start))
        {
            return Err(invalid(
                "a held run must be carried out of the segment before",
            ));
        }
        self.mark_held(index, 1, start, through)
    }

    /// Continues `key`'s evidence, carried out of the segment before, unchanged through
    /// `through`, as [`Self::hold`] does its run.
    pub fn hold_evidence(&mut self, key: KeyHandle, through: i32) -> io::Result<()> {
        let index = self.key(key)?;
        let start = self.start()?;
        let edge = self.keys[index];
        if edge.domain != NONE
            || !edge
                .evidence
                .is_some_and(|evidence| i64::from(evidence.span.last) + 1 == i64::from(start))
        {
            return Err(invalid(
                "held evidence must be the key's own, carried out of the segment before",
            ));
        }
        self.mark_held(index, 2, start, through)
    }

    /// Appends `value`, the piece of `key`'s open evidence through the last hour its family's
    /// cut covers: a piece only continuing the evidence carried out of the segment before, from
    /// its first hour, is held ([`Self::hold_evidence`]).
    pub fn continue_evidence(&mut self, key: KeyHandle, value: Evidence) -> io::Result<()> {
        let index = self.key(key)?;
        let edge = self.keys[index];
        if edge.domain == NONE
            && edge.hold & 2 == 0
            && value.span.first == self.start()?
            && edge.evidence.is_some_and(|carried| {
                same_evidence(carried, value)
                    && i64::from(carried.span.last) + 1 == i64::from(value.span.first)
            })
        {
            return self.hold_evidence(key, value.span.last);
        }
        self.evidence(key, value, None)
    }

    fn mark_held(&mut self, index: usize, part: u8, start: i32, through: i32) -> io::Result<()> {
        let _memory = publishing();
        Span::new(start, through)?;
        if self.columns.held_through.is_some_and(|held| held != through) {
            return Err(invalid("a segment's holds end at different hours"));
        }
        if self.keys[index].hold == 0 {
            self.columns.holds.push(index as u64)?;
        }
        self.columns.held_through = Some(through);
        self.keys[index].hold |= part;
        Ok(())
    }

    /// Continues every held run and evidence through the hour they are held through: by
    /// reference where that is `last`, the segment's, and the segment holds nothing else of
    /// its key, else as the piece the segment holds.
    fn continue_holds(&mut self, start: i32, last: i32) -> io::Result<()> {
        let Some(through) = self.columns.held_through else {
            return Ok(());
        };
        if through > last {
            return Err(invalid("series rows outside their published segment"));
        }
        let span = Span::new(start, through)?;
        self.columns.held.reserve(self.columns.holds.len())?;
        for at in 0..self.columns.holds.len() {
            let index = self.columns.holds[at] as usize;
            let key = index as u64;
            let hold = std::mem::take(&mut self.keys[index].hold);
            let edge = self.keys[index];
            let run = edge.last.filter(|_| hold & 1 != 0);
            let evidence = edge.evidence.filter(|_| hold & 2 != 0);
            // A family whose walk ended short of the segment's last hour holds nothing past it.
            if edge.touched || through < last {
                if let Some(run) = run {
                    self.run(key, span, run.value, false, false)?;
                }
                if let Some(evidence) = evidence {
                    self.evidence(key, Evidence { span, ..evidence }, None)?;
                }
                continue;
            }
            let coverage = evidence.is_some_and(|e| e.known == Knownness::Known)
                && edge
                    .known
                    .is_some_and(|known| i64::from(known) + 1 == i64::from(start));
            if run.is_some() && edge.domain == NONE && !coverage {
                return Err(invalid("series run outside known coverage"));
            }
            let rows = if run.is_some() {
                add(edge.rows, 1)?
            } else {
                edge.rows
            };
            self.columns.held.put(HeldRow {
                key,
                held: Held {
                    runs: run.is_some(),
                    evidence: evidence.is_some(),
                    coverage,
                    origin: run.map_or(0, |run| run.span.first),
                    evidence_origin: evidence.map_or(0, |evidence| evidence.span.first),
                },
                row_start: edge.rows,
                canonicals: edge.canonicals,
                domain: edge.domain,
                value: run.map_or(0, |run| run.value),
                decimals: edge.decimals,
            });
            self.columns.last = self.columns.last.max(last);
            let edge = &mut self.keys[index];
            edge.rows = rows;
            if let Some(run) = edge.last.as_mut().filter(|_| hold & 1 != 0) {
                run.span.last = last;
            }
            if let Some(held) = edge.evidence.as_mut().filter(|_| hold & 2 != 0) {
                held.span.last = last;
                if held.known == Knownness::Known {
                    edge.known = Some(last);
                }
            }
        }
        Ok(())
    }

    pub fn evidence(
        &mut self,
        key: KeyHandle,
        value: Evidence,
        presentation: Option<u64>,
    ) -> io::Result<()> {
        let _memory = publishing();
        let index = self.key(key)?;
        let edge = self.keys[index];
        if edge.domain != NONE {
            return Err(invalid("domain-bound key has independent evidence"));
        }
        if edge.hold & 2 != 0 {
            return Err(invalid(
                "held evidence takes no rows in the segment holding it",
            ));
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
        let _memory = publishing();
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
        let _memory = publishing();
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
        let _memory = publishing();
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
        let _memory = publishing();
        self.columns.metadata.push(Meta { id, value })
    }

    /// Where `blob` lies in the segment's heap.
    fn held(&self, blob: Blob) -> io::Result<(usize, usize)> {
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
        Ok((start, end))
    }
    pub fn blob(&mut self, input: &mut impl Read, len: u64) -> io::Result<Blob> {
        let _memory = publishing();
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
        self.bytes(text.as_bytes())
    }
    /// `bytes` as a blob, copied once.
    pub fn bytes(&mut self, bytes: &[u8]) -> io::Result<Blob> {
        let _memory = publishing();
        if bytes.is_empty() {
            return Ok(Blob::EMPTY);
        }
        let at = add(self.heap, self.columns.heap.len() as u64)?;
        add(at, bytes.len() as u64)?;
        self.columns.heap.extend(bytes)?;
        Ok(Blob {
            at,
            len: bytes.len() as u64,
        })
    }
    /// `input` as a list blob: every cell encoded first, then appended at once, so a refused
    /// cell or a refused admission appends nothing.
    pub fn cells(&mut self, input: impl IntoIterator<Item = Cell>) -> io::Result<Blob> {
        let _memory = publishing();
        let mut encoded = std::mem::take(&mut self.encoded);
        encoded.clear();
        let appended = (|| {
            for cell in input {
                let bytes = cell.bytes()?;
                engine_allocation::reserve(&mut encoded, bytes.len())?;
                encoded.extend_from_slice(&bytes);
            }
            if encoded.is_empty() {
                return Ok(Blob::EMPTY);
            }
            let at = add(self.heap, self.columns.heap.len() as u64)?;
            let len = encoded.len() as u64;
            add(at, len)?;
            self.columns.heap.extend(&encoded)?;
            Ok(Blob { at, len })
        })();
        self.encoded = encoded;
        appended
    }

    /// Closes the segment: its rows must lie within the publication's accepted hours.
    /// The returned job seals pages on any thread; the emit continues at once.
    pub fn cut(&mut self, publication: Publication, checkpoint: Blob) -> io::Result<Pending> {
        let _memory = publishing();
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
        self.continue_holds(start, publication.accepted.last)?;
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
        let fresh = Columns::new();
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

    /// Closes the segment at hour `last` as the next publication, with no checkpoint: no root
    /// names it until a later segment's does, and a reader pins it only as a prefix of that.
    pub fn cut_through(&mut self, last: i32) -> io::Result<Pending> {
        let (generation, segment) = match self.published {
            Some(previous) => (add(previous.generation, 1)?, add(previous.segment, 1)?),
            None => (1, 1),
        };
        let publication = Publication {
            generation,
            segment,
            accepted: Span::new(self.first, last)?,
            reached: last,
            refusal: None,
        };
        self.cut(publication, Blob::EMPTY)
    }

    /// Adds the next sealed segment to the hierarchy, merging any 16 complete siblings, and
    /// writes the publication's root.
    pub fn publish(&mut self, io: &mut impl PageWrite, sealed: Sealed) -> io::Result<PageId> {
        let _memory = publishing();
        self.tree
            .as_mut()
            .ok_or_else(|| invalid("series hierarchy lent out"))?
            .publish(io, sealed)
    }

    /// Lends out its hierarchy: the segments it cuts publish into it elsewhere, in order, until
    /// [`Self::return_tree`] takes it back.
    pub fn lend_tree(&mut self) -> io::Result<Tree> {
        let _memory = publishing();
        self.tree
            .take()
            .ok_or_else(|| invalid("series hierarchy lent out"))
    }

    /// Takes back the hierarchy [`Self::lend_tree`] lent, with what was published into it.
    pub fn return_tree(&mut self, tree: Tree) -> io::Result<()> {
        let _memory = publishing();
        if self.tree.is_some() || tree.identity != self.identity {
            return Err(invalid("series hierarchy returned to another writer"));
        }
        self.tree = Some(tree);
        Ok(())
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
