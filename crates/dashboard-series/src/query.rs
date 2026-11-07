use crate::{
    Grow, add,
    codec::{Binary, Decode, Encode},
    format::{Definition, Entry, Meta, Node, NodeRef, Root},
    invalid,
    pages::{Kind, PAGE_BYTES, PageRead},
    read::{Reader, Upper},
    source::{Piece, Rows, RunRows, Source, Subject, partition_point},
    summary::Summary,
    types::*,
};
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub use crate::events::Cluster;

#[derive(Clone, Copy)]
pub struct Selection<'a> {
    pub key: KeyHandle,
    pub membership: Option<&'a [Span]>,
}
impl Selection<'_> {
    pub fn validate(self) -> io::Result<()> {
        if let Some(spans) = self.membership {
            if spans.iter().any(|s| s.first > s.last)
                || spans.windows(2).any(|w| w[0].last >= w[1].first)
            {
                return Err(invalid("invalid series membership"));
            }
        }
        Ok(())
    }
    pub(crate) fn includes(self, hour: i32) -> bool {
        self.membership.is_none_or(|s| {
            s.get(s.partition_point(|s| s.last < hour))
                .is_some_and(|s| s.contains(hour))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub(crate) binding: [u8; 32],
    pub(crate) position: u64,
    pub(crate) after: [u64; 3],
    pub(crate) auxiliary: u64,
    pub(crate) base: u64,
}
impl Binary for Cursor {
    const SIZE: usize = 80;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.bytes(&self.binding)?;
        e.u64(self.position)?;
        for v in self.after {
            e.u64(v)?;
        }
        e.u64(self.auxiliary)?;
        e.u64(self.base)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            binding: d.array()?,
            position: d.u64()?,
            after: [d.u64()?, d.u64()?, d.u64()?],
            auxiliary: d.u64()?,
            base: d.u64()?,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageResult {
    pub next: Option<Cursor>,
    pub returned: u64,
    pub total: Option<u64>,
    pub first_ordinal: u64,
}

#[derive(Clone, Copy)]
pub struct EventQuery<'a> {
    pub selection: &'a [Selection<'a>],
    pub native: Span,
    pub as_of: Option<i32>,
    pub order: EventOrder,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventView {
    Family(Event),
    Evidence {
        key: KeyHandle,
        sequence: u64,
        value: Evidence,
    },
}
impl EventView {
    pub fn key(self) -> u64 {
        match self {
            Self::Family(v) => v.key,
            Self::Evidence { key, .. } => key,
        }
    }
    pub fn sequence(self) -> u64 {
        match self {
            Self::Family(v) => v.sequence,
            Self::Evidence { sequence, .. } => sequence,
        }
    }
    pub fn native_hour(self) -> i32 {
        match self {
            Self::Family(v) => v.native_hour,
            Self::Evidence { value, .. } => value.span.first,
        }
    }
    pub fn read_hour(self) -> Option<i32> {
        match self {
            Self::Family(v) => v.read_hour,
            Self::Evidence { value, .. } => value.supplying_read,
        }
    }
}

pub(crate) struct Access<'a, P> {
    pub(crate) inner: P,
    cancelled: Option<&'a AtomicBool>,
}
impl<P: PageRead> PageRead for Access<'_, P> {
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]> {
        self.check()?;
        self.inner.identity(id)
    }
    fn len(&mut self, id: PageId) -> io::Result<u64> {
        self.check()?;
        self.inner.len(id)
    }
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()> {
        self.check()?;
        self.inner.read(id, offset, into)
    }
}
impl<P> Access<'_, P> {
    fn check(&self) -> io::Result<()> {
        cancelled(self.cancelled)
    }
}
fn cancelled(flag: Option<&AtomicBool>) -> io::Result<()> {
    if flag.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "series query cancelled",
        ))
    } else {
        Ok(())
    }
}

/// The part of a pinned publication every query of it walks, its root and interior nodes: held
/// by whoever holds the publication open, and shared by each snapshot opened from it.
#[derive(Clone)]
pub struct Pinned(Arc<Upper>);

/// One pinned publication. It holds every page, node header and key definition it reads until
/// it is dropped; decoded pages are shared with every reader holding them.
pub struct Snapshot<'a, P> {
    pub(crate) io: Access<'a, P>,
    pub(crate) reader: Reader,
    defs: HashMap<u64, Option<Definition>>,
    cancelled: Option<&'a AtomicBool>,
    pub(crate) event_walk: Option<crate::events::EventWalk>,
}
/// The identity of the publication rooted at `id`, before any is expected of it: what an
/// owner compares to learn whether it may resume the publication.
pub fn root_identity(io: &mut impl PageRead, id: PageId) -> io::Result<Identity> {
    Ok(Reader::open(io, id)?.root().identity)
}

impl<'a, P: PageRead> Snapshot<'a, P> {
    pub fn open(mut io: P, id: PageId, identity: Identity) -> io::Result<Self> {
        let reader = Reader::open(&mut io, id)?;
        if reader.root().identity != identity {
            return Err(invalid("series snapshot identity mismatch"));
        }
        Ok(Self {
            io: Access {
                inner: io,
                cancelled: None,
            },
            reader,
            defs: HashMap::new(),
            cancelled: None,
            event_walk: None,
        })
    }
    /// A snapshot of the publication `pinned` holds, read through `io`.
    pub fn open_pinned(io: P, pinned: &Pinned) -> Self {
        Self {
            io: Access {
                inner: io,
                cancelled: None,
            },
            reader: Reader::upper(Arc::clone(&pinned.0)),
            defs: HashMap::new(),
            cancelled: None,
            event_walk: None,
        }
    }
    /// Its root and interior nodes, read once, for snapshots of the same publication to start from.
    pub fn pin(&mut self) -> io::Result<Pinned> {
        self.check()?;
        Ok(Pinned(Arc::new(self.reader.pin(&mut self.io)?)))
    }
    pub fn with_cancellation(mut self, cancelled: &'a AtomicBool) -> Self {
        self.cancelled = Some(cancelled);
        self.io.cancelled = Some(cancelled);
        self
    }
    pub(crate) fn check(&self) -> io::Result<()> {
        cancelled(self.cancelled)
    }
    pub fn publication(&self) -> Publication {
        self.reader.root().state.publication
    }
    pub fn identity(&self) -> Identity {
        self.reader.root().identity
    }
    pub fn into_io(self) -> P {
        self.io.inner
    }
    pub(crate) fn accepted(&self) -> Span {
        self.reader.root().state.publication.accepted
    }
    pub(crate) fn source(&mut self) -> Source<'_, Access<'a, P>> {
        let accepted = self.accepted();
        Source {
            r: &mut self.reader,
            io: &mut self.io,
            defs: &mut self.defs,
            accepted,
        }
    }

    /// The prefix a joint gate accepts: the publication of the last segment ending by `joint_hour`.
    pub fn through(mut self, joint_hour: i32) -> io::Result<Option<Self>> {
        self.check()?;
        let accepted = self.accepted();
        if accepted.last <= joint_hour {
            return Ok(Some(self));
        }
        let Some(segment) = self.reader.segment_ref(&mut self.io, joint_hour)? else {
            return Ok(None);
        };
        let end = if segment.hours.last == joint_hour {
            segment
        } else {
            let Some(before) = segment.hours.first.checked_sub(1) else {
                return Ok(None);
            };
            match self.reader.segment_ref(&mut self.io, before)? {
                Some(before) => before,
                None => return Ok(None),
            }
        };
        let mut nodes = Vec::new();
        let frontier = self.reader.root().nodes.clone();
        collect(
            &mut self.reader,
            &mut self.io,
            &frontier,
            end.first,
            &mut nodes,
        )?;
        let node = self.reader.child(&mut self.io, end)?;
        let state = node
            .segment
            .ok_or_else(|| invalid("series segment header"))?
            .state;
        let root = Root {
            identity: self.reader.root().identity,
            state,
            nodes,
        };
        self.reader.rebase(root, end.page)?;
        self.defs.clear();
        self.event_walk = None;
        Ok(Some(self))
    }
    pub fn batch_through(&mut self, hour: i32) -> io::Result<Option<i32>> {
        self.check()?;
        if !self.accepted().contains(hour) {
            return Ok(None);
        }
        Ok(self
            .reader
            .segment_ref(&mut self.io, hour)?
            .map(|segment| segment.hours.last))
    }

    pub fn value_at(&mut self, key: u64, hour: i32) -> io::Result<Observation> {
        self.check()?;
        let accepted = self.accepted();
        if hour > accepted.last {
            return Ok(Observation::Pending);
        }
        if hour < accepted.first {
            return Ok(Observation::Unknown(None));
        }
        let mut s = self.source();
        let Some(definition) = s.def(key)? else {
            return Ok(Observation::Unknown(None));
        };
        observe(&mut s, key, &definition, hour, None)
    }
    pub fn values_at(
        &mut self,
        keys: &[KeyHandle],
        hour: i32,
        mut sink: impl FnMut(usize, io::Result<Observation>) -> io::Result<()>,
    ) -> io::Result<()> {
        for (at, &key) in keys.iter().enumerate() {
            self.check()?;
            let value = self.value_at(key, hour);
            sink(at, value)?;
        }
        Ok(())
    }
    /// Each key's samples whose amount differs from the sample before, each
    /// key's runs read once in hour order.
    pub fn changes_at(
        &mut self,
        keys: &[KeyHandle],
        hours: &[i32],
        mut sink: impl FnMut(usize, usize, Observation) -> io::Result<()>,
    ) -> io::Result<()> {
        if hours.windows(2).any(|w| w[0] >= w[1]) {
            return Err(invalid("sample hours must be distinct and ascending"));
        }
        let accepted = self.accepted();
        for (key_index, &key) in keys.iter().enumerate() {
            self.check()?;
            let mut s = self.source();
            let Some(definition) = s.def(key)? else {
                continue;
            };
            let mut runs = Held::default();
            let mut previous = None;
            for (sample, &hour) in hours.iter().enumerate() {
                let run = if !accepted.contains(hour) {
                    None
                } else if definition.mode == RunMode::Exact {
                    runs.at(&mut s, key, hour)?
                } else {
                    s.point_run(key, &definition, hour)?
                };
                let amount = run.map(|r| r.value);
                if previous != amount {
                    let value = if hour > accepted.last {
                        Observation::Pending
                    } else if hour < accepted.first {
                        Observation::Unknown(None)
                    } else {
                        observe(&mut s, key, &definition, hour, Some(run))?
                    };
                    sink(key_index, sample, value)?;
                    previous = amount;
                }
            }
        }
        Ok(())
    }
    pub fn observation_changes_at(
        &mut self,
        keys: &[KeyHandle],
        hours: &[i32],
        mut sink: impl FnMut(usize, usize, Observation) -> io::Result<()>,
    ) -> io::Result<()> {
        if hours.windows(2).any(|w| w[0] >= w[1]) {
            return Err(invalid("sample hours must be distinct and ascending"));
        }
        let accepted = self.accepted();
        for (key_index, &key) in keys.iter().enumerate() {
            self.check()?;
            let mut s = self.source();
            let definition = s.def(key)?;
            let mut runs = Held::default();
            let mut evidence = HeldEvidence::default();
            let mut previous = None;
            for (sample, &hour) in hours.iter().enumerate() {
                let value = if hour > accepted.last {
                    Observation::Pending
                } else if hour < accepted.first {
                    Observation::Unknown(None)
                } else if let Some(definition) = definition {
                    match evidence.at(&mut s, Subject::of(key, &definition), hour)? {
                        None => Observation::Unknown(None),
                        Some(e) if matches!(e.known, Knownness::Unknown(_)) => {
                            Observation::Unknown(Some(e))
                        }
                        Some(e) => {
                            let run = if definition.mode == RunMode::Exact {
                                runs.at(&mut s, key, hour)?
                            } else {
                                s.point_run(key, &definition, hour)?
                            };
                            match run {
                                Some(run) => Observation::Value { run, evidence: e },
                                None => Observation::KnownAbsent(e),
                            }
                        }
                    }
                } else {
                    Observation::Unknown(None)
                };
                if previous != Some(value) {
                    sink(key_index, sample, value)?;
                    previous = Some(value);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn binding(
        &self,
        kind: u8,
        selection: &[Selection<'_>],
        span: Span,
        extra: &[u8],
    ) -> io::Result<[u8; 32]> {
        let root = self.reader.root();
        let mut hash = blake3::Hasher::new();
        hash.update(&root.identity.source);
        hash.update(&root.identity.content);
        hash.update(&root.identity.branch);
        hash.update(&root.identity.schema.to_le_bytes());
        hash.update(&root.state.publication.generation.to_le_bytes());
        hash.update(&self.reader.id().to_le_bytes());
        hash.update(&[kind]);
        hash.update(&span.first.to_le_bytes());
        hash.update(&span.last.to_le_bytes());
        hash.update(&(selection.len() as u64).to_le_bytes());
        for s in selection {
            s.validate()?;
            hash.update(&s.key.to_le_bytes());
            hash.update(&[u8::from(s.membership.is_some())]);
            if let Some(spans) = s.membership {
                hash.update(&(spans.len() as u64).to_le_bytes());
                for span in spans {
                    hash.update(&span.first.to_le_bytes());
                    hash.update(&span.last.to_le_bytes());
                }
            }
        }
        hash.update(extra);
        Ok(*hash.finalize().as_bytes())
    }
    pub(crate) fn cursor(&self, binding: [u8; 32], cursor: Option<Cursor>) -> io::Result<Cursor> {
        if let Some(cursor) = cursor {
            if cursor.binding != binding {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "restart series cursor: snapshot or selection changed",
                ));
            }
            Ok(cursor)
        } else {
            Ok(Cursor {
                binding,
                position: 0,
                after: [0; 3],
                auxiliary: NONE,
                base: 0,
            })
        }
    }

    /// Key definitions by id from the cursor; a filter keeps one schema and measure.
    pub fn catalogue_page(
        &mut self,
        filter: Option<(u64, u64)>,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(KeyDefinition) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        self.check()?;
        let mut extra = [0; 17];
        extra[0] = u8::from(filter.is_some());
        if let Some((schema, measure)) = filter {
            extra[1..9].copy_from_slice(&schema.to_le_bytes());
            extra[9..].copy_from_slice(&measure.to_le_bytes());
        }
        let binding = self.binding(1, &[], self.accepted(), &extra)?;
        let mut cursor = self.cursor(binding, cursor)?;
        let allowance = byte_budget / KeyDefinition::SIZE;
        if allowance == 0 {
            return Err(invalid("catalogue page budget cannot hold one row"));
        }
        let total = self.reader.root().state.next_key;
        let mut returned = 0;
        let mut more = false;
        while cursor.position < total {
            self.check()?;
            let Some(definition) = self.source().def(cursor.position)? else {
                break;
            };
            if filter.is_some_and(|(schema, measure)| {
                definition.key.spec.schema != schema || definition.key.spec.measure != measure
            }) {
                cursor.position = add(cursor.position, 1)?;
                continue;
            }
            if returned == allowance {
                more = true;
                break;
            }
            sink(definition.key)?;
            returned += 1;
            cursor.position = add(cursor.position, 1)?;
        }
        Ok(PageResult {
            next: more.then_some(cursor),
            returned: returned as u64,
            total: filter.is_none().then_some(total),
            first_ordinal: 0,
        })
    }
    /// Family metadata by id, each id's latest value.
    pub fn metadata_page(
        &mut self,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(u64, Cell) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        let binding = self.binding(2, &[], self.accepted(), &[])?;
        let mut cursor = self.cursor(binding, cursor)?;
        let allowance = byte_budget / (Cell::SIZE + 8);
        if allowance == 0 {
            return Err(invalid("metadata page budget cannot hold one row"));
        }
        let first = cursor.position;
        let frontier = self.reader.root().nodes.clone();
        let mut tables = engine_allocation::vec(frontier.len())?;
        for reference in frontier {
            let node = self.reader.child(&mut self.io, reference)?;
            let table = node.metadata;
            let (mut low, mut high) = (0, table.count);
            while low < high {
                let middle = low + (high - low) / 2;
                if self.reader.record::<Meta>(&mut self.io, table, middle)?.id < cursor.position {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            tables.try_push((table, low))?;
        }
        let mut returned = 0;
        let mut more = false;
        loop {
            self.check()?;
            let mut next: Option<(Meta, usize)> = None;
            for (at, &(table, position)) in tables.iter().enumerate() {
                if position < table.count {
                    let meta = self.reader.record::<Meta>(&mut self.io, table, position)?;
                    if next.is_none_or(|(held, _)| meta.id <= held.id) {
                        next = Some((meta, at));
                    }
                }
            }
            let Some((meta, _)) = next else {
                break;
            };
            if returned == allowance {
                more = true;
                break;
            }
            for at in 0..tables.len() {
                let (table, position) = tables[at];
                if position < table.count
                    && self
                        .reader
                        .record::<Meta>(&mut self.io, table, position)?
                        .id
                        == meta.id
                {
                    tables[at].1 += 1;
                }
            }
            sink(meta.id, meta.value)?;
            returned += 1;
            cursor.position = add(meta.id, 1)?;
        }
        Ok(PageResult {
            next: more.then_some(cursor),
            returned: returned as u64,
            total: None,
            first_ordinal: first,
        })
    }
    pub fn checkpoint(&self) -> Blob {
        self.reader.root().state.checkpoint
    }
    /// Bytes of a variable-width cell, read from the segment heap that holds it.
    pub fn cell_bytes(&mut self, blob: Blob, offset: u64, into: &mut [u8]) -> io::Result<()> {
        self.check()?;
        let end = add(offset, into.len() as u64)?;
        if end > blob.len {
            return Err(invalid("blob read outside cell"));
        }
        if into.is_empty() {
            return Ok(());
        }
        let start = add(blob.at, offset)?;
        let segment = self.heap_segment(blob)?;
        let (first, list) = {
            let node = self.reader.child(&mut self.io, segment)?;
            let heap = node
                .segment
                .ok_or_else(|| invalid("series heap outside a segment"))?
                .heap;
            (node.heap.0, heap)
        };
        let mut copied = 0;
        while copied < into.len() {
            let at = start + copied as u64 - first;
            let page = self
                .reader
                .page(&mut self.io, list, at / PAGE_BYTES as u64, Kind::Heap)?;
            let within = (at % PAGE_BYTES as u64) as usize;
            let bytes = page
                .bytes()
                .get(within..)
                .ok_or_else(|| invalid("series heap page extent"))?;
            let count = bytes.len().min(into.len() - copied);
            if count == 0 {
                return Err(invalid("truncated variable-width series cell"));
            }
            into[copied..copied + count].copy_from_slice(&bytes[..count]);
            copied += count;
        }
        Ok(())
    }
    fn heap_segment(&mut self, blob: Blob) -> io::Result<NodeRef> {
        let end = add(blob.at, blob.len)?;
        let mut references = self.reader.root().nodes.clone();
        loop {
            let mut found = None;
            for reference in references {
                let node = self.reader.child(&mut self.io, reference)?;
                if node.heap.0 <= blob.at && end <= node.heap.1 {
                    found = Some((reference, node));
                    break;
                }
            }
            let (reference, node) = found.ok_or_else(|| invalid("series cell outside the heap"))?;
            if node.level == 0 {
                return Ok(reference);
            }
            references = node.children.clone();
        }
    }
    pub fn field(&mut self, fields: Blob, index: u64) -> io::Result<Cell> {
        let offset = index
            .checked_mul(Cell::SIZE as u64)
            .ok_or_else(|| invalid("field offset overflow"))?;
        let mut bytes = [0; Cell::SIZE];
        self.cell_bytes(fields, offset, &mut bytes)?;
        Cell::decode(&mut Decode::new(&bytes))
    }

    /// A key's runs over `span`, clipped to it, the accepted hours and the
    /// membership, and coalesced where equal amounts continue without a break.
    pub fn run_page(
        &mut self,
        selection: Selection<'_>,
        span: Span,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(Run) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        let binding = self.binding(3, &[selection], span, &[])?;
        let mut cursor = self.cursor(binding, cursor)?;
        let allowance = byte_budget / Run::SIZE;
        if allowance == 0 {
            return Err(invalid("run page budget cannot hold one row"));
        }
        let first = cursor.position;
        let accepted = self.accepted();
        let mut s = self.source();
        let Some(definition) = s.def(selection.key)? else {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: Some(0),
                first_ordinal: first,
            });
        };
        let Some(window) = span.intersect(accepted) else {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: None,
                first_ordinal: first,
            });
        };
        let mut pieces = Pieces::new(selection, definition, window, cursor)?;
        let mut returned = 0;
        while returned < allowance {
            let Some(mut run) = pieces.next(&mut s)? else {
                break;
            };
            while let Some(following) = pieces.peek(&mut s)? {
                if !following.break_before
                    && following.value == run.value
                    && i64::from(run.span.last) + 1 == i64::from(following.span.first)
                {
                    run.span.last = following.span.last;
                    pieces.next(&mut s)?;
                } else {
                    break;
                }
            }
            sink(run)?;
            returned += 1;
        }
        let more = pieces.peek(&mut s)?.is_some();
        cursor.position = add(cursor.position, returned as u64)?;
        pieces.save(&mut cursor);
        Ok(PageResult {
            next: more.then_some(cursor),
            returned: returned as u64,
            total: None,
            first_ordinal: first,
        })
    }
    pub fn runs_each(
        &mut self,
        selections: &[Selection<'_>],
        span: Span,
        byte_budget: usize,
        mut sink: impl FnMut(usize, io::Result<Run>) -> io::Result<()>,
    ) -> io::Result<()> {
        for (i, &selection) in selections.iter().enumerate() {
            let mut cursor = None;
            loop {
                let mut sink_failed = false;
                match self.run_page(selection, span, cursor, byte_budget, |run| {
                    let result = sink(i, Ok(run));
                    sink_failed = result.is_err();
                    result
                }) {
                    Ok(page) => {
                        cursor = page.next;
                        if cursor.is_none() {
                            break;
                        }
                    }
                    Err(error) => {
                        if sink_failed || error.kind() == io::ErrorKind::Interrupted {
                            return Err(error);
                        }
                        sink(i, Err(error))?;
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    /// A key's evidence rows over `span`, clipped to it and the accepted hours.
    pub fn evidence_page(
        &mut self,
        key: u64,
        span: Span,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(Evidence) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        let binding = self.binding(4, &[selection_of(key)], span, &[])?;
        let mut cursor = self.cursor(binding, cursor)?;
        let first = cursor.position;
        let allowance = byte_budget / Evidence::SIZE;
        if allowance == 0 {
            return Err(invalid("evidence page budget cannot hold one row"));
        }
        let accepted = self.accepted();
        let mut s = self.source();
        let Some(definition) = s.def(key)? else {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: Some(0),
                first_ordinal: first,
            });
        };
        let subject = Subject::of(key, &definition);
        let Some(window) = span.intersect(accepted) else {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: None,
                first_ordinal: first,
            });
        };
        let mut from = resume(cursor.auxiliary).unwrap_or(window.first);
        let mut returned = 0;
        let mut next = s.evidence_from(subject, from)?;
        while let Some(row) = next.filter(|row| row.span.first <= window.last) {
            if returned == allowance {
                break;
            }
            let part = row
                .span
                .intersect(window)
                .ok_or_else(|| invalid("series evidence outside its window"))?;
            sink(Evidence { span: part, ..row })?;
            returned += 1;
            match row.span.last.checked_add(1) {
                Some(after) => {
                    from = after;
                    next = s.evidence_from(subject, from)?;
                }
                None => next = None,
            }
        }
        let more = next.is_some_and(|row| row.span.first <= window.last);
        cursor.position = add(cursor.position, returned as u64)?;
        cursor.auxiliary = stamp(from);
        Ok(PageResult {
            next: more.then_some(cursor),
            returned: returned as u64,
            total: None,
            first_ordinal: first,
        })
    }
}

/// The run holding each ascending hour, read forward without reseeking within a run.
#[derive(Default)]
struct Held {
    run: Option<Run>,
    until: Option<i32>,
}
impl Held {
    fn at<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        key: u64,
        hour: i32,
    ) -> io::Result<Option<Run>> {
        if self.until.is_none_or(|until| hour > until) {
            self.run = s.run_from(key, hour)?;
            self.until = Some(match self.run {
                Some(run) if run.span.first > hour => run.span.first - 1,
                Some(run) => run.span.last,
                None => i32::MAX,
            });
        }
        Ok(self.run.filter(|run| run.span.contains(hour)))
    }
}
#[derive(Default)]
struct HeldEvidence {
    row: Option<Evidence>,
    until: Option<i32>,
}
impl HeldEvidence {
    fn at<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        subject: Subject,
        hour: i32,
    ) -> io::Result<Option<Evidence>> {
        if self.until.is_none_or(|until| hour > until) {
            self.row = s.evidence_from(subject, hour)?;
            self.until = Some(match self.row {
                Some(row) if row.span.first > hour => row.span.first - 1,
                Some(row) => row.span.last,
                None => i32::MAX,
            });
        }
        Ok(self.row.filter(|row| row.span.contains(hour)))
    }
}

/// An observation at an accepted hour; `run` passes the run already found there.
fn observe<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    definition: &Definition,
    hour: i32,
    run: Option<Option<Run>>,
) -> io::Result<Observation> {
    let Some(evidence) = s.evidence_at(Subject::of(key, definition), hour)? else {
        return Ok(Observation::Unknown(None));
    };
    if matches!(evidence.known, Knownness::Unknown(_)) {
        return Ok(Observation::Unknown(Some(evidence)));
    }
    let run = match run {
        Some(run) => run,
        None => s.point_run(key, definition, hour)?,
    };
    Ok(match run {
        Some(run) => Observation::Value { run, evidence },
        None => Observation::KnownAbsent(evidence),
    })
}

fn stamp(hour: i32) -> u64 {
    u64::from((hour as u32) ^ 0x8000_0000)
}
fn resume(value: u64) -> Option<i32> {
    (value != NONE).then(|| ((value as u32) ^ 0x8000_0000) as i32)
}

/// Canonical runs clipped to a window and membership, in hour order for exact
/// keys and in insertion order for ordered contributions.
struct Pieces<'m> {
    selection: Selection<'m>,
    definition: Definition,
    window: Span,
    // Ordered contributions: the next hour of the current canonical, none once it ends.
    from: Option<i32>,
    canonical: u64,
    started: bool,
    done: bool,
    peeked: Option<Run>,
    walk: Option<Contributions>,
}
impl<'m> Pieces<'m> {
    fn new(
        selection: Selection<'m>,
        definition: Definition,
        window: Span,
        cursor: Cursor,
    ) -> io::Result<Self> {
        selection.validate()?;
        Ok(Self {
            selection,
            definition,
            window,
            from: Some(resume(cursor.auxiliary).unwrap_or(window.first)),
            canonical: cursor.after[0],
            started: cursor.after[1] != 0,
            done: false,
            peeked: None,
            walk: None,
        })
    }
    fn save(&self, cursor: &mut Cursor) {
        match self.peeked {
            Some(run) => {
                cursor.auxiliary = stamp(run.span.first);
                cursor.after[0] = run.canonical;
                cursor.after[1] = u64::from(self.started && run.canonical == self.canonical);
            }
            None => {
                cursor.auxiliary = self.from.map_or(NONE, stamp);
                cursor.after[0] = self.canonical;
                cursor.after[1] = u64::from(self.started);
            }
        }
    }
    fn peek<P: PageRead>(&mut self, s: &mut Source<'_, P>) -> io::Result<Option<Run>> {
        if self.peeked.is_none() {
            self.peeked = self.find(s)?;
        }
        Ok(self.peeked)
    }
    fn next<P: PageRead>(&mut self, s: &mut Source<'_, P>) -> io::Result<Option<Run>> {
        let run = self.peek(s)?;
        self.peeked = None;
        if let Some(run) = run {
            self.from = run.span.last.checked_add(1);
            self.canonical = run.canonical;
            self.started = true;
        }
        Ok(run)
    }
    fn find<P: PageRead>(&mut self, s: &mut Source<'_, P>) -> io::Result<Option<Run>> {
        if self.definition.mode == RunMode::OrderedContributions {
            return self.contribution(s);
        }
        let key = self.selection.key;
        loop {
            let Some(from) = self.from.filter(|h| *h <= self.window.last) else {
                return Ok(None);
            };
            let Some(run) = s.run_from(key, from)? else {
                self.from = None;
                return Ok(None);
            };
            let start = from.max(run.span.first);
            let Some(member) = member_from(self.selection, start) else {
                self.from = None;
                return Ok(None);
            };
            let Some(part) = run
                .span
                .intersect(self.window)
                .and_then(|s| s.intersect(member))
                .and_then(|s| {
                    s.intersect(Span {
                        first: start,
                        last: i32::MAX,
                    })
                })
            else {
                self.from = if member.first > run.span.last {
                    Some(member.first)
                } else {
                    run.span.last.checked_add(1)
                };
                continue;
            };
            return Ok(Some(Run { span: part, ..run }));
        }
    }
    /// Ordered contributions keep the current reader's canonical order: pieces
    /// by canonical, then by hour within one canonical's membership parts.
    fn contribution<P: PageRead>(&mut self, s: &mut Source<'_, P>) -> io::Result<Option<Run>> {
        if self.done {
            return Ok(None);
        }
        let key = self.selection.key;
        if self.walk.is_none() {
            self.walk = Contributions::seek(s, key, self.window, self.canonical)?;
        }
        loop {
            let Some(walk) = self.walk.as_mut() else {
                break;
            };
            if walk.current.is_none() {
                walk.current = walk.next(s, key, self.window)?;
            }
            let Some(run) = walk.current else {
                self.walk = None;
                break;
            };
            if run.canonical >= self.canonical {
                let at = if run.canonical == self.canonical && self.started {
                    self.from.map(|after| after.max(run.span.first))
                } else {
                    Some(run.span.first)
                };
                let part = at.and_then(|at| first_part(self.selection, self.window, run, at));
                if let Some(part) = part {
                    return Ok(Some(part));
                }
            }
            walk.current = None;
        }
        self.done = true;
        Ok(None)
    }
}

/// A canonical run's first part from `at` within the window and membership.
fn first_part(selection: Selection<'_>, window: Span, run: Run, at: i32) -> Option<Run> {
    let span = run.span.intersect(window)?;
    let at = at.max(span.first);
    if at > span.last {
        return None;
    }
    let member = member_from(selection, at)?;
    let part = Span {
        first: at.max(member.first),
        last: member.last.min(span.last),
    };
    (part.first <= part.last).then_some(Run { span: part, ..run })
}

/// Ordered contributions in storage order from a canonical position: each
/// canonical whole, its continuations followed through node heads.
struct Contributions {
    segment: Arc<Node>,
    entry: Entry,
    ordinal: u64,
    held: RunRows,
    current: Option<Run>,
}
impl Contributions {
    /// The walk from the first row, in the segments meeting `window`, of the
    /// first canonical at or after `canonical`.
    fn seek<P: PageRead>(
        s: &mut Source<'_, P>,
        key: u64,
        window: Span,
        canonical: u64,
    ) -> io::Result<Option<Self>> {
        let Some((segment, entry)) = s.seek(Subject::key(key), window.first, true, &|e| {
            e.runs.count != 0 && e.canonical_end > canonical
        })?
        else {
            return Ok(None);
        };
        let ordinal = canonical.saturating_sub(entry.first_canonical()?);
        Ok(Some(Self {
            segment,
            entry,
            ordinal,
            held: RunRows::default(),
            current: None,
        }))
    }
    /// The next canonical run, none past the segments meeting `window`.
    fn next<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        key: u64,
        window: Span,
    ) -> io::Result<Option<Run>> {
        if self.ordinal == self.entry.runs.count {
            let Some(after) = self.segment.hours.last.checked_add(1) else {
                return Ok(None);
            };
            let Some((segment, entry)) =
                s.seek(Subject::key(key), after, true, &|e| e.runs.count != 0)?
            else {
                return Ok(None);
            };
            self.segment = segment;
            self.entry = entry;
            self.ordinal = 0;
            self.held.clear();
        }
        if self.segment.hours.first > window.last {
            return Ok(None);
        }
        let row = s.run_row(&self.segment, &self.entry, self.ordinal, &mut self.held)?;
        let detail = self
            .entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its detail"))?;
        let canonical = add(self.entry.first_canonical()?, self.ordinal)?;
        let head = self.ordinal == 0 && row.continues;
        let mut run = Run {
            span: row.span,
            value: row.value,
            break_before: if head {
                detail.origin.break_before
            } else {
                row.break_before
            },
            supplying_boundary: if head {
                detail.origin.supplying_boundary
            } else {
                row.supplying_boundary
            },
            canonical,
            sequence: if head {
                detail.origin.sequence
            } else {
                add(self.entry.row_start, self.ordinal)?
            },
        };
        self.ordinal += 1;
        if self.ordinal == self.entry.runs.count
            && row.span.last == self.segment.hours.last
            && row.span.last < window.last
        {
            let end = s.chase(
                Subject::key(key),
                Rows::Runs,
                Some(canonical),
                row.span.last,
            )?;
            if end > row.span.last {
                let segment = s
                    .segment(end)?
                    .ok_or_else(|| invalid("series hierarchy has a hole"))?;
                self.entry = s
                    .entry(&segment, Subject::key(key))?
                    .filter(|e| {
                        e.runs.head.continues && e.first_canonical().ok() == Some(canonical)
                    })
                    .ok_or_else(|| invalid("series run chain without its row"))?;
                self.segment = segment;
                self.ordinal = 1;
                self.held.clear();
                run.span.last = end;
            }
        }
        Ok(Some(run))
    }
}

/// The run the current reader's `value_at` selects at ascending hours, and until when.
#[derive(Default)]
struct Sweep {
    path: Option<Path>,
}
impl Sweep {
    fn select<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        key: u64,
        definition: &Definition,
        hour: i32,
    ) -> io::Result<(Option<Run>, i32)> {
        if definition.mode == RunMode::Exact {
            return Ok(match s.run_from(key, hour)? {
                Some(run) if run.span.first > hour => (None, run.span.first - 1),
                Some(run) => (Some(run), run.span.last),
                None => (None, i32::MAX),
            });
        }
        let accepted = s.accepted;
        if hour < accepted.first {
            return Ok((None, accepted.first - 1));
        }
        if hour > accepted.last {
            return Ok((None, i32::MAX));
        }
        if !self
            .path
            .as_ref()
            .is_some_and(|path| path.segment.hours.contains(hour))
        {
            self.path = None;
            let segment = s
                .segment(hour)?
                .ok_or_else(|| invalid("series hierarchy has a hole"))?;
            let Some(entry) = s
                .entry(&segment, Subject::key(key))?
                .filter(|e| e.runs.count != 0)
            else {
                return Ok((None, segment.hours.last));
            };
            self.path = Some(Path::new(s, segment, entry, hour)?);
        }
        let path = self
            .path
            .as_mut()
            .ok_or_else(|| invalid("series partition path"))?;
        let (index, holds) = path.at(s, hour)?;
        let until = holds.min(path.segment.hours.last);
        let Some(ordinal) = index.checked_sub(1) else {
            return Ok((None, until));
        };
        let row = s.run_row(&path.segment, &path.entry, ordinal, &mut path.held)?;
        if row.span.first > hour {
            return Ok((None, until.min(row.span.first - 1)));
        }
        if row.span.last < hour {
            return Ok((None, until));
        }
        let run = s.canonical(key, &path.segment, &path.entry, ordinal, row)?;
        Ok((Some(run), until.min(row.span.last)))
    }
}

/// One std partition-point step: the search state it probed and the probed row's start.
#[derive(Clone, Copy, Default)]
struct Probe {
    base: u64,
    size: u64,
    at: u64,
    first: i32,
}
/// A segment's partition point kept across ascending hours: steps are probed again
/// only below the first probed row that starts by the new hour.
struct Path {
    segment: Arc<Node>,
    entry: Entry,
    held: RunRows,
    probes: [Probe; 65],
    depth: usize,
    hour: i32,
}
impl Path {
    fn new<P: PageRead>(
        s: &mut Source<'_, P>,
        segment: Arc<Node>,
        entry: Entry,
        hour: i32,
    ) -> io::Result<Self> {
        let count = entry.runs.count;
        let mut path = Self {
            segment,
            entry,
            held: RunRows::default(),
            probes: [Probe::default(); 65],
            depth: 0,
            hour,
        };
        path.descend(s, 0, 0, count)?;
        Ok(path)
    }
    /// The search from step `depth`, in `<[T]>::partition_point`'s probe order.
    fn descend<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        mut depth: usize,
        mut base: u64,
        mut size: u64,
    ) -> io::Result<()> {
        while size != 0 {
            let at = base + size / 2;
            let first = s
                .run_row(&self.segment, &self.entry, at, &mut self.held)?
                .span
                .first;
            *self
                .probes
                .get_mut(depth)
                .ok_or_else(|| invalid("series partition depth"))? = Probe {
                base,
                size,
                at,
                first,
            };
            depth += 1;
            if size == 1 {
                break;
            }
            if first <= self.hour {
                base = at;
            }
            size -= size / 2;
        }
        self.depth = depth;
        Ok(())
    }
    /// The partition point at `hour`, no earlier than the last asked, and the last hour it holds.
    fn at<P: PageRead>(&mut self, s: &mut Source<'_, P>, hour: i32) -> io::Result<(u64, i32)> {
        let before = self.hour;
        self.hour = hour;
        let flipped = self.probes[..self.depth]
            .iter()
            .position(|probe| before < probe.first && probe.first <= hour);
        if let Some(step) = flipped {
            let probe = self.probes[step];
            if probe.size > 1 {
                self.descend(s, step + 1, probe.at, probe.size - probe.size / 2)?;
            }
        }
        let holds = self.probes[..self.depth]
            .iter()
            .filter(|probe| probe.first > hour)
            .map(|probe| probe.first - 1)
            .min()
            .unwrap_or(i32::MAX);
        let index = match self.probes[..self.depth].last() {
            Some(probe) => probe.base + u64::from(probe.first <= hour),
            None => 0,
        };
        Ok((index, holds))
    }
}

/// The membership span holding `hour`, else the next one; the whole axis without membership.
fn member_from(selection: Selection<'_>, hour: i32) -> Option<Span> {
    match selection.membership {
        None => Some(Span {
            first: i32::MIN,
            last: i32::MAX,
        }),
        Some(members) => members
            .get(members.partition_point(|s| s.last < hour))
            .copied(),
    }
}

fn collect(
    reader: &mut Reader,
    io: &mut impl PageRead,
    references: &[NodeRef],
    end: u64,
    out: &mut Vec<NodeRef>,
) -> io::Result<()> {
    for &reference in references {
        let last = reference.first + reference.segments - 1;
        if last <= end {
            engine_allocation::reserve(out, 1)?;
            out.push(reference);
        } else if reference.first <= end {
            let node = reader.child(io, reference)?;
            return collect(reader, io, &node.children.clone(), end, out);
        } else {
            return Ok(());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub from: f64,
    pub to: f64,
    pub width: u32,
}
#[derive(Clone, Copy)]
pub(crate) struct Grid {
    first: i64,
    last: i64,
    base: i64,
    size: i64,
    count: u64,
}
impl Grid {
    fn new(bounds: Span, view: Viewport) -> io::Result<Self> {
        if !view.from.is_finite() || !view.to.is_finite() || view.from > view.to || view.width == 0
        {
            return Err(invalid("invalid series viewport"));
        }
        let first = view.from.floor().max(f64::from(bounds.first));
        let last = view.to.floor().min(f64::from(bounds.last));
        if first > last {
            return Ok(Self {
                first: 0,
                last: -1,
                base: 0,
                size: 1,
                count: 0,
            });
        }
        let first = first as i64;
        let last = last as i64;
        let count = last - first + 1;
        let per_pixel = (count / i64::from(view.width)).max(1);
        let fine = per_pixel <= 16;
        let size = if fine {
            1_i64 << (63 - (per_pixel as u64).leading_zeros())
        } else {
            count.div_euclid(i64::from(view.width)) + i64::from(count % i64::from(view.width) != 0)
        };
        let base = if fine {
            i64::from(bounds.first) + (first - i64::from(bounds.first)) / size * size
        } else {
            first
        };
        Ok(Self {
            first,
            last,
            base,
            size,
            count: ((last - base) / size + 1) as u64,
        })
    }
    fn bin(self, index: u64) -> io::Result<Span> {
        if index >= self.count {
            return Err(invalid("chart bin outside viewport"));
        }
        let first = self.base + index as i64 * self.size;
        Span::new(
            first.max(self.first) as i32,
            (first + self.size - 1).min(self.last) as i32,
        )
    }
    fn index(self, hour: i32) -> u64 {
        ((i64::from(hour) - self.base) / self.size) as u64
    }
}

#[derive(Clone, Copy)]
pub struct TailRun {
    pub run: Run,
    pub evidence: Evidence,
}
#[derive(Clone, Copy)]
pub struct Tail<'a> {
    pub exact: &'a [TailRun],
    pub approximate: &'a [Approximation],
}
impl Tail<'_> {
    pub const EMPTY: Self = Self {
        exact: &[],
        approximate: &[],
    };
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bucket {
    pub span: Span,
    pub exact: Option<Summary>,
    pub pending: Option<Span>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileResult {
    pub page: PageResult,
    pub key_index: usize,
    pub first_bin: u64,
    pub bins: u64,
    pub hourly: bool,
}

impl<P: PageRead> Snapshot<'_, P> {
    /// A key's known spans over `span`, coalesced across rows and segments.
    pub fn coverage_page(
        &mut self,
        key: u64,
        span: Span,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(Span) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        let binding = self.binding(11, &[selection_of(key)], span, &[])?;
        let mut next = self.cursor(binding, cursor)?;
        let first = next.position;
        let limit = byte_budget / Span::SIZE;
        if limit == 0 {
            return Err(invalid("coverage page budget cannot hold one span"));
        }
        let accepted = self.accepted();
        let mut s = self.source();
        let Some(definition) = s.def(key)? else {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: Some(0),
                first_ordinal: first,
            });
        };
        let Some(window) = span.intersect(accepted) else {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: Some(0),
                first_ordinal: first,
            });
        };
        let subject = Subject::of(key, &definition);
        let mut from = resume(next.auxiliary).unwrap_or(window.first);
        let mut pending = s.coverage_from(subject, from)?;
        let mut returned = 0;
        while let Some(known) = pending.filter(|k| k.first <= window.last) {
            if returned == limit {
                break;
            }
            let part = known
                .intersect(window)
                .ok_or_else(|| invalid("coverage cursor extent mismatch"))?;
            sink(part)?;
            returned += 1;
            next.position = add(next.position, 1)?;
            match known.last.checked_add(1) {
                Some(after) => {
                    from = after;
                    pending = s.coverage_from(subject, from)?;
                }
                None => pending = None,
            }
        }
        next.auxiliary = stamp(from);
        Ok(PageResult {
            next: pending
                .is_some_and(|k| k.first <= window.last)
                .then_some(next),
            returned: returned as u64,
            total: None,
            first_ordinal: first,
        })
    }
    /// The original runs an hourly chart shows, as the current reader lists them: each grid
    /// hour's selection unless it starts where the last one did, clipped to the membership.
    pub fn hourly_runs(
        &mut self,
        selection: Selection<'_>,
        bounds: Span,
        view: Viewport,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(Run) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        selection.validate()?;
        let grid = Grid::new(bounds, view)?;
        if grid.size != 1 {
            return Err(invalid(
                "original supplying runs require an hourly viewport",
            ));
        }
        let mut extra = [0; 20];
        extra[..8].copy_from_slice(&view.from.to_bits().to_le_bytes());
        extra[8..16].copy_from_slice(&view.to.to_bits().to_le_bytes());
        extra[16..].copy_from_slice(&view.width.to_le_bytes());
        let binding = self.binding(12, &[selection], bounds, &extra)?;
        let mut next = self.cursor(binding, cursor)?;
        let first = next.position;
        let limit = byte_budget / Run::SIZE;
        if limit == 0 {
            return Err(invalid("hourly run page budget cannot hold one run"));
        }
        let mut s = self.source();
        let definition = match s.def(selection.key)? {
            Some(definition) if grid.count != 0 => definition,
            found => {
                return Ok(PageResult {
                    next: None,
                    returned: 0,
                    total: found.is_none().then_some(0),
                    first_ordinal: first,
                });
            }
        };
        // Cursor: the hour to select, its run's parts returned, and the previous selection's start.
        let index = next.after[1].saturating_sub(1);
        if index >= grid.count {
            return Err(invalid("hourly run cursor is outside its pinned grid"));
        }
        let mut hour = grid.bin(index)?.first;
        let mut skip = next.after[0];
        let mut previous = resume(next.auxiliary);
        let last = grid.bin(grid.count - 1)?.last;
        let whole = [Span {
            first: i32::MIN,
            last: i32::MAX,
        }];
        let members = selection.membership.unwrap_or(&whole);
        let mut sweep = Sweep::default();
        let mut returned = 0;
        let mut stop = None;
        loop {
            if returned == limit {
                stop = Some((hour, 0, previous));
                break;
            }
            let (run, until) = sweep.select(&mut s, selection.key, &definition, hour)?;
            if let Some(run) = run {
                if previous != Some(run.span.first) {
                    let from = members.partition_point(|m| m.last < run.span.first);
                    let parts = members[from..]
                        .iter()
                        .take_while(|m| m.first <= run.span.last)
                        .enumerate();
                    for (part, member) in parts.skip(usize::try_from(skip).unwrap_or(usize::MAX)) {
                        if returned == limit {
                            stop = Some((hour, part as u64, previous));
                            break;
                        }
                        let span = run
                            .span
                            .intersect(*member)
                            .ok_or_else(|| invalid("hourly run outside its membership"))?;
                        sink(Run {
                            span,
                            break_before: run.break_before || span.first != run.span.first,
                            ..run
                        })?;
                        returned += 1;
                    }
                    if stop.is_some() {
                        break;
                    }
                }
                previous = Some(run.span.first);
            }
            skip = 0;
            let mut after = i64::from(until) + 1;
            if definition.mode == RunMode::Exact {
                // Exact runs selected from here start here: only those reaching a member count.
                match member_from(selection, i32::try_from(after).unwrap_or(i32::MAX)) {
                    Some(member) if after <= i64::from(last) => {
                        after = after.max(i64::from(member.first)).min(i64::from(last));
                    }
                    _ => break,
                }
            }
            if after > i64::from(last) {
                break;
            }
            hour = after as i32;
        }
        next.position = add(next.position, returned as u64)?;
        if let Some((hour, part, previous)) = stop {
            next.after = [part, grid.index(hour) + 1, 0];
            next.auxiliary = previous.map_or(NONE, stamp);
        }
        Ok(PageResult {
            next: stop.map(|_| next),
            returned: returned as u64,
            total: None,
            first_ordinal: first,
        })
    }
    pub fn top_spans_with_tail(
        &mut self,
        selection: Selection<'_>,
        top: u64,
        span: Span,
        tail: Tail<'_>,
        mut sink: impl FnMut(Span) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut pending = None;
        self.top_spans(selection, top, span, |s| {
            join_span(&mut pending, s, &mut sink)
        })?;
        let mut previous = self.accepted().last;
        for value in tail.exact {
            if value.run.span.first <= previous
                || value.evidence.span.intersect(value.run.span) != Some(value.run.span)
                || value.evidence.known != Knownness::Known
            {
                return Err(invalid("unverified exact rank tail"));
            }
            previous = value.run.span.last;
            if value.run.value as f64 > top as f64 {
                continue;
            }
            if let Some(part) = value.run.span.intersect(span) {
                let whole = [part];
                for member in selection.membership.unwrap_or(&whole) {
                    if let Some(part) = part.intersect(*member) {
                        join_span(&mut pending, part, &mut sink)?;
                    }
                }
            }
        }
        if let Some(pending) = pending {
            sink(pending)?;
        }
        Ok(())
    }
    pub fn value_with_tail(
        &mut self,
        key: u64,
        hour: i32,
        tail: Tail<'_>,
    ) -> io::Result<Observation> {
        if hour <= self.accepted().last {
            return self.value_at(key, hour);
        }
        let mut previous = self.accepted().last;
        for value in tail.exact {
            if value.run.span.first <= previous
                || value.evidence.span.intersect(value.run.span) != Some(value.run.span)
                || value.evidence.known != Knownness::Known
            {
                return Err(invalid("unverified exact point tail"));
            }
            previous = value.run.span.last;
            if value.run.span.contains(hour) {
                return Ok(Observation::Value {
                    run: value.run,
                    evidence: value.evidence,
                });
            }
        }
        Ok(Observation::Pending)
    }
    pub fn summary_tiles(
        &mut self,
        selections: &[Selection<'_>],
        bounds: Span,
        view: Viewport,
        tail: Tail<'_>,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(u64, Bucket) -> io::Result<()>,
    ) -> io::Result<TileResult> {
        if selections.len() != 1 && (!tail.exact.is_empty() || !tail.approximate.is_empty()) {
            return Err(invalid("an unkeyed tail requires exactly one selected key"));
        }
        self.check()?;
        Span::new(bounds.first, bounds.last)?;
        let grid = Grid::new(bounds, view)?;
        let mut extra = [0; 20];
        extra[..8].copy_from_slice(&view.from.to_bits().to_le_bytes());
        extra[8..16].copy_from_slice(&view.to.to_bits().to_le_bytes());
        extra[16..].copy_from_slice(&view.width.to_le_bytes());
        let mut hash = blake3::Hasher::new();
        hash.update(&extra);
        for v in tail.exact {
            let mut bytes = [0; Run::SIZE + Evidence::SIZE];
            let mut e = Encode::new(&mut bytes);
            v.run.encode(&mut e)?;
            v.evidence.encode(&mut e)?;
            hash.update(&bytes);
        }
        for v in tail.approximate {
            hash.update(&v.span.first.to_le_bytes());
            hash.update(&v.span.last.to_le_bytes());
            hash.update(&v.value.to_bits().to_le_bytes());
        }
        let binding = self.binding(5, selections, bounds, hash.finalize().as_bytes())?;
        let mut cursor = self.cursor(binding, cursor)?;
        if cursor.auxiliary == NONE {
            cursor.auxiliary = 0;
        }
        let key_index =
            usize::try_from(cursor.position).map_err(|_| invalid("chart key cursor overflow"))?;
        let first_bin = cursor.auxiliary;
        if key_index >= selections.len() || grid.count == 0 {
            return Ok(TileResult {
                page: PageResult {
                    next: None,
                    returned: 0,
                    total: Some(grid.count),
                    first_ordinal: first_bin,
                },
                key_index,
                first_bin,
                bins: grid.count,
                hourly: grid.size == 1,
            });
        }
        if first_bin >= grid.count {
            return Err(invalid("chart cursor is outside its pinned grid"));
        }
        let allowance = byte_budget / std::mem::size_of::<Bucket>();
        if allowance == 0 {
            return Err(invalid("chart tile budget cannot hold one bucket"));
        }
        let count = usize::try_from((grid.count - first_bin).min(allowance as u64))
            .map_err(|_| invalid("chart tile size"))?;
        let mut buckets = engine_allocation::filled(count, None)?;
        let selection = selections[key_index];
        selection.validate()?;
        let tile = Span {
            first: grid.bin(first_bin)?.first,
            last: grid.bin(first_bin + count as u64 - 1)?.last,
        };
        let accepted = self.accepted();
        let mut s = self.source();
        if let Some(definition) = s.def(selection.key)? {
            numeric_tiles(
                &mut s,
                selection,
                &definition,
                grid,
                first_bin,
                &mut buckets,
            )?;
            let mut previous = accepted.last;
            for v in tail.exact {
                if v.run.span.first <= previous
                    || v.evidence.span.intersect(v.run.span) != Some(v.run.span)
                    || !matches!(v.evidence.known, Knownness::Known)
                {
                    return Err(invalid("tail is not separately authenticated and ordered"));
                }
                previous = v.run.span.last;
                if let Some(span) = v.run.span.intersect(tile) {
                    if let Some(membership) = selection.membership {
                        for member in membership {
                            if let Some(span) = span.intersect(*member) {
                                distribute(
                                    Summary::run(v.run, span, definition.decimals),
                                    grid,
                                    first_bin,
                                    &mut buckets,
                                    definition.decimals,
                                )?;
                            }
                        }
                    } else {
                        distribute(
                            Summary::run(v.run, span, definition.decimals),
                            grid,
                            first_bin,
                            &mut buckets,
                            definition.decimals,
                        )?;
                    }
                }
            }
        }
        for (i, exact) in buckets.iter_mut().enumerate() {
            let bin = grid.bin(first_bin + i as u64)?;
            if let Some(s) = exact {
                if selection.membership.is_none() {
                    s.has_break |= s.first_hour > bin.first || s.last_hour < bin.last;
                }
            }
            let pending = accepted.last.checked_add(1).and_then(|first| {
                bin.intersect(Span {
                    first,
                    last: bounds.last,
                })
            });
            sink(
                first_bin + i as u64,
                Bucket {
                    span: bin,
                    exact: *exact,
                    pending,
                },
            )?;
        }
        cursor.auxiliary = first_bin + count as u64;
        if cursor.auxiliary == grid.count {
            cursor.position = add(cursor.position, 1)?;
            cursor.auxiliary = 0;
        }
        Ok(TileResult {
            page: PageResult {
                next: ((cursor.position as usize) < selections.len()).then_some(cursor),
                returned: count as u64,
                total: Some(grid.count),
                first_ordinal: first_bin,
            },
            key_index,
            first_bin,
            bins: grid.count,
            hourly: grid.size == 1,
        })
    }
    pub fn approximate_spans(
        &self,
        tail: Tail<'_>,
        span: Span,
        mut sink: impl FnMut(Approximation) -> io::Result<()>,
    ) -> io::Result<()> {
        for &v in tail.approximate {
            if !v.value.is_finite() || v.span.first > v.span.last {
                return Err(invalid("invalid marked approximation"));
            }
            if let Some(span) = v.span.intersect(span) {
                sink(Approximation { span, ..v })?;
            }
        }
        Ok(())
    }
    pub fn rank_at(
        &mut self,
        ranks: &[(u64, KeyHandle)],
        hour: i32,
        mut sink: impl FnMut(u64, Observation) -> io::Result<()>,
    ) -> io::Result<()> {
        for &(actor_or_position, key) in ranks {
            sink(actor_or_position, self.value_at(key, hour)?)?;
        }
        Ok(())
    }
    /// The hours a rank key holds a value within `top`: wholly inside nodes and
    /// blocks are taken from their summaries, crossings read their rows.
    pub fn top_spans(
        &mut self,
        selection: Selection<'_>,
        top: u64,
        span: Span,
        mut sink: impl FnMut(Span) -> io::Result<()>,
    ) -> io::Result<()> {
        selection.validate()?;
        let accepted = self.accepted();
        let Some(span) = span.intersect(accepted) else {
            return Ok(());
        };
        let mut s = self.source();
        let Some(definition) = s.def(selection.key)? else {
            return Ok(());
        };
        let cut = top as f64;
        let mut pending: Option<Span> = None;
        s.fold(
            selection.key,
            definition.decimals,
            definition.mode == RunMode::Exact,
            span,
            &mut |piece: Piece| {
                let summary = piece.summary;
                let take = |part: Span,
                            pending: &mut Option<Span>,
                            sink: &mut dyn FnMut(Span) -> io::Result<()>|
                 -> io::Result<()> {
                    let Some(part) = part.intersect(span) else {
                        return Ok(());
                    };
                    let whole = [part];
                    for member in selection.membership.unwrap_or(&whole) {
                        if let Some(part) = part.intersect(*member) {
                            join_span(pending, part, sink)?;
                        }
                    }
                    Ok(())
                };
                if piece.row {
                    if summary.raw_max as f64 <= cut {
                        take(piece.extent, &mut pending, &mut sink)?;
                    }
                    return Ok(true);
                }
                if summary.raw_max as f64 <= cut
                    && !summary.has_break
                    && summary.covered == piece.extent.hours()
                {
                    take(piece.extent, &mut pending, &mut sink)?;
                    return Ok(true);
                }
                if summary.raw_min as f64 > cut {
                    return Ok(true);
                }
                Ok(false)
            },
        )?;
        if let Some(span) = pending {
            sink(span)?;
        }
        Ok(())
    }
    pub fn navigate(
        &mut self,
        selections: &[Selection<'_>],
        origin: Option<i32>,
        direction: Direction,
        bounds: Span,
    ) -> io::Result<Option<i32>> {
        Ok(self
            .navigate_at(selections, origin.map(f64::from), direction, bounds)?
            .map(|h| h as i32))
    }
    pub fn navigate_at(
        &mut self,
        selections: &[Selection<'_>],
        origin: Option<f64>,
        direction: Direction,
        bounds: Span,
    ) -> io::Result<Option<f64>> {
        self.numeric_navigation(
            selections,
            origin,
            direction,
            (f64::from(bounds.first), f64::from(bounds.last)),
            true,
        )
    }
    pub fn navigate_window(
        &mut self,
        selections: &[Selection<'_>],
        origin: Option<f64>,
        direction: Direction,
        bounds: (f64, f64),
    ) -> io::Result<Option<f64>> {
        self.numeric_navigation(selections, origin, direction, bounds, true)
    }
    fn numeric_navigation(
        &mut self,
        selections: &[Selection<'_>],
        origin: Option<f64>,
        direction: Direction,
        bounds: (f64, f64),
        fallback: bool,
    ) -> io::Result<Option<f64>> {
        self.check()?;
        if !bounds.0.is_finite() || !bounds.1.is_finite() || bounds.0 > bounds.1 {
            return Err(invalid("invalid navigation bounds"));
        }
        if origin.is_some_and(|h| !h.is_finite()) {
            return Err(invalid("invalid navigation hour"));
        }
        let forward = matches!(direction, Direction::Forward | Direction::Home);
        let at = match direction {
            Direction::Home => bounds.0 - 1.0,
            Direction::End => bounds.1 + 1.0,
            _ => origin.unwrap_or(if forward {
                bounds.0 - 1.0
            } else {
                bounds.1 + 1.0
            }),
        };
        let accepted = self.accepted();
        let mut result = None;
        for &selection in selections {
            selection.validate()?;
            let mut s = self.source();
            let Some(definition) = s.def(selection.key)? else {
                continue;
            };
            let whole = [Span {
                first: i32::MIN,
                last: i32::MAX,
            }];
            for member in selection.membership.unwrap_or(&whole) {
                let (first, last) = if selection.membership.is_some() {
                    (
                        bounds.0.max(f64::from(member.first)),
                        bounds.1.min(f64::from(member.last)),
                    )
                } else {
                    bounds
                };
                if first > last {
                    continue;
                }
                let within = if forward {
                    at.max(first - 1.0)
                } else {
                    at.min(last + 1.0)
                };
                let candidate = numeric_next(
                    &mut s,
                    selection.key,
                    &definition,
                    accepted,
                    within,
                    forward,
                )?
                .map(f64::from)
                .filter(|h| *h >= first && *h <= last);
                let carried =
                    numeric_next(&mut s, selection.key, &definition, accepted, first, false)?
                        .is_some();
                let edge = (carried && ((forward && at < first) || (!forward && at > first)))
                    .then_some(first);
                for hour in [candidate, edge].into_iter().flatten() {
                    result = Some(result.map_or(hour, |previous: f64| {
                        if forward {
                            previous.min(hour)
                        } else {
                            previous.max(hour)
                        }
                    }));
                }
            }
        }
        Ok(result.or_else(|| {
            if fallback {
                origin.map(|h| h.clamp(bounds.0, bounds.1))
            } else {
                None
            }
        }))
    }
    /// The last hour of `bounds` (which starts at `hour`) through which every key this snapshot
    /// has read keeps the value and knownness it has at `hour`: an answer read from those keys at
    /// `hour` is the same through it. `bounds.last` when none of them changes within `bounds`.
    pub fn held_through(&mut self, hour: i32, bounds: Span) -> io::Result<i32> {
        let keys: Vec<u64> = self
            .defs
            .iter()
            .filter(|(_, definition)| definition.is_some())
            .map(|(key, _)| *key)
            .collect();
        let mut through = bounds.last;
        for key in keys {
            if through <= hour {
                break;
            }
            let selection = selection_of(key);
            let bounds = Span {
                first: hour,
                last: through,
            };
            for next in [
                self.next_change(selection, hour, true, bounds)?,
                self.next_evidence_change(selection, hour, true, bounds)?,
            ]
            .into_iter()
            .flatten()
            {
                through = through.min(next.saturating_sub(1));
            }
        }
        Ok(through.max(hour))
    }
    pub fn next_change(
        &mut self,
        selection: Selection<'_>,
        hour: impl Into<f64>,
        forward: bool,
        bounds: Span,
    ) -> io::Result<Option<i32>> {
        Ok(self
            .numeric_navigation(
                &[selection],
                Some(hour.into()),
                if forward {
                    Direction::Forward
                } else {
                    Direction::Backward
                },
                (f64::from(bounds.first), f64::from(bounds.last)),
                false,
            )?
            .map(|h| h as i32))
    }
    /// The nearest evidence boundary (a row's start or the hour after its end) past `hour`.
    pub fn next_evidence_change(
        &mut self,
        selection: Selection<'_>,
        hour: impl Into<f64>,
        forward: bool,
        bounds: Span,
    ) -> io::Result<Option<i32>> {
        self.check()?;
        selection.validate()?;
        let hour = hour.into();
        if !hour.is_finite() {
            return Err(invalid("invalid evidence navigation hour"));
        }
        let mut s = self.source();
        let Some(definition) = s.def(selection.key)? else {
            return Ok(None);
        };
        let subject = Subject::of(selection.key, &definition);
        let accepts = |at: i32| {
            bounds.contains(at)
                && selection.includes(at)
                && if forward {
                    f64::from(at) > hour
                } else {
                    f64::from(at) < hour
                }
        };
        if forward {
            let start = hour.floor().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            let mut row = s.evidence_from(subject, start)?;
            while let Some(e) = row.filter(|e| e.span.first <= bounds.last) {
                for at in [Some(e.span.first), e.span.last.checked_add(1)]
                    .into_iter()
                    .flatten()
                {
                    if accepts(at) {
                        return Ok(Some(at));
                    }
                }
                row = match e.span.last.checked_add(1) {
                    Some(next) => s.evidence_from(subject, next)?,
                    None => None,
                };
            }
        } else {
            let start = hour.ceil().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            let mut row = s.evidence_before(subject, start)?;
            while let Some(e) = row {
                if i64::from(e.span.last) + 1 < i64::from(bounds.first) {
                    break;
                }
                for at in [e.span.last.checked_add(1), Some(e.span.first)]
                    .into_iter()
                    .flatten()
                {
                    if accepts(at) {
                        return Ok(Some(at));
                    }
                }
                row = match e.span.first.checked_sub(1) {
                    Some(previous) => s.evidence_before(subject, previous)?,
                    None => None,
                };
            }
        }
        Ok(None)
    }
}

/// The start after `hour` of an exact key's next run.
fn next_run_start<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<i32>> {
    let Some(after) = hour.checked_add(1) else {
        return Ok(None);
    };
    Ok(match s.run_from(key, after)? {
        Some(run) if run.span.first > hour => Some(run.span.first),
        Some(run) => match run.span.last.checked_add(1) {
            Some(next) => s.run_from(key, next)?.map(|run| run.span.first),
            None => None,
        },
        None => None,
    })
}

/// A navigation stop as the current reader's `next_change` finds it: forward,
/// the hour after the run at `hour` or the next run's start; backward, the end
/// of the last run starting before `hour`, or its start.
fn numeric_next<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    definition: &Definition,
    accepted: Span,
    hour: f64,
    forward: bool,
) -> io::Result<Option<i32>> {
    if forward {
        let floor = hour.floor();
        if floor >= f64::from(accepted.first) && floor <= f64::from(accepted.last) {
            if let Some(run) = s.point_run(key, definition, floor as i32)? {
                return Ok(run.span.last.min(accepted.last).checked_add(1));
            }
        }
        let next = floor + 1.0;
        if next > f64::from(accepted.last) {
            return Ok(None);
        }
        let first = next.max(f64::from(accepted.first)) as i32;
        if definition.mode == RunMode::OrderedContributions {
            return ordered_forward(s, key, hour, first);
        }
        if s.point_run(key, definition, first)?.is_some() {
            return Ok(Some(first));
        }
        return Ok(next_run_start(s, key, first)?.filter(|h| *h <= accepted.last));
    }
    if hour <= f64::from(accepted.first) {
        return Ok(None);
    }
    let probe = (hour.ceil() - 1.0).min(f64::from(accepted.last)) as i32;
    let previous = if definition.mode == RunMode::Exact {
        s.run_before(key, probe)?
    } else {
        ordered_backward(s, key, definition, hour, probe)?
    };
    let Some(run) = previous else {
        return Ok(None);
    };
    let end = run.span.last.min(accepted.last);
    let first = run.span.first.max(accepted.first);
    Ok(Some(
        end.checked_add(1)
            .filter(|h| f64::from(*h) < hour)
            .unwrap_or(first),
    ))
}

/// Forward from `hour` with no run at it: the row after the partition point in
/// the segment holding `first`, else the first run created after that segment.
fn ordered_forward<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    hour: f64,
    first: i32,
) -> io::Result<Option<i32>> {
    let Some(segment) = s.segment(first)? else {
        return Ok(None);
    };
    let mut held = RunRows::default();
    if let Some(entry) = s.entry(&segment, Subject::key(key))? {
        let index = partition_point(entry.runs.count, |ordinal| {
            Ok(f64::from(s.run_row(&segment, &entry, ordinal, &mut held)?.span.first) <= hour)
        })?;
        if index < entry.runs.count {
            return Ok(Some(
                s.run_row(&segment, &entry, index, &mut held)?.span.first,
            ));
        }
    }
    let Some(after) = segment.hours.last.checked_add(1) else {
        return Ok(None);
    };
    let Some((next, entry)) = s.seek(Subject::key(key), after, true, &|e| {
        e.canonical_end > e.canonical_start
    })?
    else {
        return Ok(None);
    };
    let ordinal = entry
        .canonical_start
        .checked_sub(entry.first_canonical()?)
        .ok_or_else(|| invalid("series canonical counters"))?;
    held.clear();
    Ok(Some(
        s.run_row(&next, &entry, ordinal, &mut held)?.span.first,
    ))
}

/// Backward from `hour`: the run at the end of the row before the partition
/// point in the segment holding `probe`, else at the end of the run carried
/// out of the segment before it.
fn ordered_backward<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    definition: &Definition,
    hour: f64,
    probe: i32,
) -> io::Result<Option<Run>> {
    let segment = s
        .segment(probe)?
        .ok_or_else(|| invalid("series hierarchy has a hole"))?;
    let mut held = RunRows::default();
    let mut last = None;
    if let Some(entry) = s.entry(&segment, Subject::key(key))? {
        let index = partition_point(entry.runs.count, |ordinal| {
            Ok(f64::from(s.run_row(&segment, &entry, ordinal, &mut held)?.span.first) < hour)
        })?;
        if let Some(ordinal) = index.checked_sub(1) {
            last = Some(s.run_row(&segment, &entry, ordinal, &mut held)?.span.last);
        }
    }
    let last = match last {
        Some(last) => last,
        None => {
            let Some(before) = segment.hours.first.checked_sub(1) else {
                return Ok(None);
            };
            let Some((carried, entry)) =
                s.seek(Subject::key(key), before, false, &|e| e.runs.count != 0)?
            else {
                return Ok(None);
            };
            held.clear();
            s.run_row(&carried, &entry, entry.runs.count - 1, &mut held)?
                .span
                .last
        }
    };
    s.point_run(key, definition, last)?
        .map(Some)
        .ok_or_else(|| invalid("missing preceding run"))
}

fn distribute(
    summary: Summary,
    grid: Grid,
    first_bin: u64,
    buckets: &mut [Option<Summary>],
    decimals: u8,
) -> io::Result<()> {
    let start = grid.index(summary.first_hour);
    let end = grid.index(summary.last_hour);
    if start > end {
        return Err(invalid("reversed summary cannot be distributed as a span"));
    }
    for index in start.max(first_bin)..=end.min(first_bin + buckets.len() as u64 - 1) {
        let contribution = if start == end {
            summary
        } else {
            summary.clip_constant(grid.bin(index)?, decimals)?
        };
        let target = &mut buckets[(index - first_bin) as usize];
        *target = Some(if let Some(previous) = *target {
            previous.merge(contribution)?
        } else {
            contribution
        });
    }
    Ok(())
}
fn join_span(
    pending: &mut Option<Span>,
    span: Span,
    sink: &mut dyn FnMut(Span) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(previous) = pending {
        if i64::from(previous.last) + 1 >= i64::from(span.first) {
            previous.last = previous.last.max(span.last);
            return Ok(());
        }
        sink(*previous)?;
    }
    *pending = Some(span);
    Ok(())
}

pub(crate) fn selection_of(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

fn merge_present(into: &mut Option<Summary>, value: Option<Summary>) -> io::Result<()> {
    if let Some(value) = value {
        *into = Some(match *into {
            Some(previous) => previous.merge(value)?,
            None => value,
        });
    }
    Ok(())
}
fn merge_missing(
    into: &mut Option<Summary>,
    value: Option<Summary>,
    initialized: &mut bool,
) -> io::Result<()> {
    if *initialized && (into.is_none() || value.is_none()) {
        merge_present(into, value)?;
        if let Some(summary) = into {
            summary.has_break = true;
        }
    } else {
        merge_present(into, value)?;
    }
    *initialized = true;
    Ok(())
}
fn mark_gap(value: &mut Option<Summary>, span: Span) {
    if let Some(value) = value {
        value.has_break |= value.first_hour > span.first || value.last_hour < span.last;
    }
}

fn numeric_tiles<P: PageRead>(
    s: &mut Source<'_, P>,
    selection: Selection<'_>,
    definition: &Definition,
    grid: Grid,
    first_bin: u64,
    buckets: &mut [Option<Summary>],
) -> io::Result<()> {
    let key = selection.key;
    let decimals = definition.decimals;
    let accepted = s.accepted;
    let tile = Span {
        first: grid.bin(first_bin)?.first,
        last: grid.bin(first_bin + buckets.len() as u64 - 1)?.last,
    };
    if grid.size == 1 {
        let mut runs = Held::default();
        for (index, bucket) in buckets.iter_mut().enumerate() {
            let bin = grid.bin(first_bin + index as u64)?;
            if !accepted.contains(bin.first) {
                continue;
            }
            let run = if definition.mode == RunMode::Exact {
                runs.at(s, key, bin.first)?
            } else {
                s.point_run(key, definition, bin.first)?
            };
            *bucket = run.map(|run| Summary::run(run, bin, decimals));
        }
    } else if let Some(published) = tile.intersect(accepted) {
        if definition.mode == RunMode::Exact {
            s.fold(key, decimals, true, published, &mut |piece: Piece| {
                if piece.row
                    || (published.first <= piece.extent.first
                        && piece.extent.last <= published.last
                        && grid.index(piece.extent.first) == grid.index(piece.extent.last))
                {
                    distribute(piece.summary, grid, first_bin, buckets, decimals)?;
                    return Ok(true);
                }
                if piece.summary.constant() {
                    if let Some(part) = piece.extent.intersect(published) {
                        distribute(
                            piece.summary.clip_constant(part, decimals)?,
                            grid,
                            first_bin,
                            buckets,
                            decimals,
                        )?;
                    }
                    return Ok(true);
                }
                Ok(false)
            })?;
        } else {
            for (index, bucket) in buckets.iter_mut().enumerate() {
                if let Some(span) = grid.bin(first_bin + index as u64)?.intersect(accepted) {
                    *bucket = legacy_range(s, key, definition, span)?;
                }
            }
        }
    }
    if let Some(members) = selection.membership {
        for bucket in buckets.iter_mut() {
            let Some(original) = *bucket else {
                continue;
            };
            let mut masked = None;
            for member in members
                .iter()
                .skip_while(|s| s.last < original.first_hour)
                .take_while(|s| s.first <= original.last_hour)
            {
                let first = original.first_hour.max(member.first);
                let last = original.last_hour.min(member.last);
                if first > last {
                    continue;
                }
                let next = numeric_range(s, key, definition, Span { first, last })?;
                if masked.is_some() {
                    let mut initialized = true;
                    merge_missing(&mut masked, next, &mut initialized)?;
                } else {
                    masked = next;
                }
            }
            *bucket = masked;
        }
    }
    Ok(())
}

pub(crate) fn numeric_range<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    definition: &Definition,
    span: Span,
) -> io::Result<Option<Summary>> {
    let Some(part) = span.intersect(s.accepted) else {
        return Ok(None);
    };
    if definition.mode == RunMode::OrderedContributions {
        let mut result = legacy_range(s, key, definition, part)?;
        mark_gap(&mut result, span);
        return Ok(result);
    }
    let decimals = definition.decimals;
    let mut result = None;
    s.fold(key, decimals, true, part, &mut |piece: Piece| {
        if piece.row || (part.first <= piece.extent.first && piece.extent.last <= part.last) {
            merge_present(&mut result, Some(piece.summary))?;
            return Ok(true);
        }
        if piece.summary.constant() {
            if let Some(clip) = piece.extent.intersect(part) {
                merge_present(
                    &mut result,
                    Some(piece.summary.clip_constant(clip, decimals)?),
                )?;
            }
            return Ok(true);
        }
        Ok(false)
    })?;
    mark_gap(&mut result, span);
    Ok(result)
}

fn repeated(mut summary: Summary, blocks: u64, size: i64) -> io::Result<Summary> {
    if blocks == 1 {
        return Ok(summary);
    }
    let shift = (blocks - 1) as i64 * size;
    let contiguous = i64::from(summary.last_hour) + 1 == i64::from(summary.first_hour) + size;
    // As `Summary::merge` folding the copies: only one unbroken stretch carries its value on.
    if summary.last_value_hour != summary.first_hour
        || !contiguous
        || summary.min_value != summary.max_value
        || summary.first_value != summary.last_value
    {
        summary.last_value_hour = i32::try_from(i64::from(summary.last_value_hour) + shift)
            .map_err(|_| invalid("repeated value hour overflow"))?;
    }
    summary.last_hour = i32::try_from(i64::from(summary.last_hour) + shift)
        .map_err(|_| invalid("repeated summary extent overflow"))?;
    summary.has_break |= !contiguous;
    summary.covered = summary
        .covered
        .checked_mul(blocks)
        .ok_or_else(|| invalid("summary coverage overflow"))?;
    summary.absolute_total = summary.absolute_total.checked_mul(blocks)?;
    Ok(summary)
}
/// One aligned block of ordered contributions, folded in insertion order, and
/// how many identical blocks follow it before any contribution changes.
fn contribution_blocks<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    decimals: u8,
    block: Span,
    size: i64,
    mut blocks: u64,
) -> io::Result<(Option<Summary>, u64)> {
    let aligned = i64::from(block.first).div_euclid(size);
    fn limit(span: Span, block: Span, size: i64, aligned: i64, blocks: &mut u64) {
        if span.first > block.last {
            *blocks = (*blocks).min((i64::from(span.first).div_euclid(size) - aligned) as u64);
        } else if span.last >= block.first {
            if span.first > block.first || span.last < block.last {
                *blocks = 1;
            } else {
                *blocks =
                    (*blocks).min(((i64::from(span.last) + 1).div_euclid(size) - aligned) as u64);
            }
        }
    }
    let mut result = None;
    s.contributions(key, block, &mut |row| {
        limit(row.span, block, size, aligned, &mut blocks);
        if let Some(part) = row.span.intersect(block) {
            merge_present(&mut result, Some(Summary::run(row, part, decimals)))?;
        }
        Ok(())
    })?;
    if let Some(after) = block.last.checked_add(1) {
        let mut at = after;
        while let Some((segment, entry)) = s.run_segment(key, at)? {
            let mut first: Option<i32> = None;
            s.segment_runs(&segment, &entry, &mut |row| {
                if row.span.first > block.last {
                    first = Some(first.map_or(row.span.first, |h| h.min(row.span.first)));
                }
                Ok(())
            })?;
            if let Some(first) = first {
                limit(
                    Span { first, last: first },
                    block,
                    size,
                    aligned,
                    &mut blocks,
                );
                break;
            }
            match segment.hours.last.checked_add(1) {
                Some(next) => at = next,
                None => break,
            }
        }
    }
    if blocks == 0 {
        return Err(invalid("contribution block sweep did not advance"));
    }
    Ok((
        result.map(|s| repeated(s, blocks, size)).transpose()?,
        blocks,
    ))
}
fn legacy_range<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    definition: &Definition,
    span: Span,
) -> io::Result<Option<Summary>> {
    let levels = [16_i64, 256, 4096, 65536];
    let mut hour = i64::from(span.first);
    let last = i64::from(span.last);
    let mut result = None;
    let mut initialized = false;
    while hour <= last {
        if let Some((level, &size)) = levels
            .iter()
            .enumerate()
            .rev()
            .find(|(_, size)| hour.rem_euclid(**size) == 0 && hour + **size - 1 <= last)
        {
            let mut count = ((last - hour + 1) / size) as u64;
            if let Some(&next) = levels.get(level + 1) {
                count = count.min(((next - hour.rem_euclid(next)) / size) as u64);
            }
            let block = Span {
                first: hour as i32,
                last: (hour + size - 1) as i32,
            };
            let (value, used) =
                contribution_blocks(s, key, definition.decimals, block, size, count)?;
            merge_missing(&mut result, value, &mut initialized)?;
            hour += used as i64 * size;
        } else {
            let end = last.min((hour.div_euclid(16) + 1) * 16 - 1);
            let mut partial = None;
            let mut seen = false;
            while hour <= end {
                let run = s.point_run(key, definition, hour as i32)?;
                let until = run.map_or(hour, |run| i64::from(run.span.last).min(end));
                let value = run.map(|run| {
                    Summary::run(
                        run,
                        Span {
                            first: hour as i32,
                            last: until as i32,
                        },
                        definition.decimals,
                    )
                });
                merge_missing(&mut partial, value, &mut seen)?;
                hour = until + 1;
            }
            merge_missing(&mut result, partial, &mut initialized)?;
        }
    }
    mark_gap(&mut result, span);
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alias {
    pub key: KeyHandle,
    pub span: Span,
    pub boundary: Option<bool>,
}

impl<P: PageRead> Snapshot<'_, P> {
    pub fn hourly_alias_runs(
        &mut self,
        aliases: &[Alias],
        bounds: Span,
        view: Viewport,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(Run) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        Span::new(bounds.first, bounds.last)?;
        if aliases.iter().any(|a| a.span.first > a.span.last)
            || aliases
                .windows(2)
                .any(|w| w[0].span.last >= w[1].span.first)
        {
            return Err(invalid("dated aliases must be ordered and disjoint"));
        }
        if Grid::new(bounds, view)?.size != 1 {
            return Err(invalid("original alias runs require an hourly viewport"));
        }
        let mut selections = engine_allocation::vec(aliases.len())?;
        let mut hash = blake3::Hasher::new();
        let mut extra = [0; 20];
        extra[..8].copy_from_slice(&view.from.to_bits().to_le_bytes());
        extra[8..16].copy_from_slice(&view.to.to_bits().to_le_bytes());
        extra[16..].copy_from_slice(&view.width.to_le_bytes());
        hash.update(&extra);
        for alias in aliases {
            selections.try_push(Selection {
                key: alias.key,
                membership: Some(std::slice::from_ref(&alias.span)),
            })?;
            hash.update(&[match alias.boundary {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            }]);
        }
        let binding = self.binding(13, &selections, bounds, hash.finalize().as_bytes())?;
        let mut next = self.cursor(binding, cursor)?;
        let first = next.base;
        let limit = byte_budget / Run::SIZE;
        if limit == 0 {
            return Err(invalid("alias run page budget cannot hold one run"));
        }
        let mut returned = 0;
        while next.position < aliases.len() as u64 && returned < limit {
            self.check()?;
            let alias = aliases[next.position as usize];
            if f64::from(alias.span.last) < view.from.floor()
                || f64::from(alias.span.first) > view.to.floor()
            {
                next.position = add(next.position, 1)?;
                continue;
            }
            let selected = Selection {
                key: alias.key,
                membership: Some(std::slice::from_ref(&alias.span)),
            };
            let child = if next.after[2] == 0 {
                None
            } else {
                let binding = self.binding(12, &[selected], bounds, &extra)?;
                let mut child = self.cursor(binding, None)?;
                child.auxiliary = next.auxiliary;
                child.after[..2].copy_from_slice(&next.after[..2]);
                Some(child)
            };
            let page = self.hourly_runs(
                selected,
                bounds,
                view,
                child,
                (limit - returned) * Run::SIZE,
                |mut run| {
                    if run.span.first == alias.span.first {
                        if let Some(boundary) = alias.boundary {
                            run.break_before = boundary;
                        }
                    }
                    sink(run)
                },
            )?;
            returned += page.returned as usize;
            next.base = add(next.base, page.returned)?;
            if let Some(child) = page.next {
                next.auxiliary = child.auxiliary;
                next.after = [child.after[0], child.after[1], 1];
                break;
            }
            next.position = add(next.position, 1)?;
            next.auxiliary = NONE;
            next.after = [0; 3];
        }
        Ok(PageResult {
            next: (next.position < aliases.len() as u64).then_some(next),
            returned: returned as u64,
            total: None,
            first_ordinal: first,
        })
    }
    pub fn summary_aliases(
        &mut self,
        aliases: &[Alias],
        bounds: Span,
        view: Viewport,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(u64, Bucket) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        Span::new(bounds.first, bounds.last)?;
        if aliases.iter().any(|a| a.span.first > a.span.last)
            || aliases
                .windows(2)
                .any(|w| w[0].span.last >= w[1].span.first)
        {
            return Err(invalid("dated aliases must be ordered and disjoint"));
        }
        let mut selections = engine_allocation::vec(aliases.len())?;
        for alias in aliases {
            selections.try_push(Selection {
                key: alias.key,
                membership: Some(std::slice::from_ref(&alias.span)),
            })?;
        }
        let mut extra = [0; 20];
        extra[..8].copy_from_slice(&view.from.to_bits().to_le_bytes());
        extra[8..16].copy_from_slice(&view.to.to_bits().to_le_bytes());
        extra[16..].copy_from_slice(&view.width.to_le_bytes());
        let mut hash = blake3::Hasher::new();
        hash.update(&extra);
        for alias in aliases {
            hash.update(&[match alias.boundary {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            }]);
        }
        let binding = self.binding(9, &selections, bounds, hash.finalize().as_bytes())?;
        let mut cursor = self.cursor(binding, cursor)?;
        let first = cursor.position;
        let grid = Grid::new(bounds, view)?;
        let limit = byte_budget / std::mem::size_of::<Bucket>();
        if limit == 0 {
            return Err(invalid("alias tile budget cannot hold one bin"));
        }
        if first >= grid.count {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: Some(grid.count),
                first_ordinal: first,
            });
        }
        let count = (grid.count - first).min(limit as u64) as usize;
        let mut buckets: Vec<Option<Summary>> = engine_allocation::filled(count, None)?;
        let extent = Span {
            first: grid.bin(first)?.first,
            last: grid.bin(first + count as u64 - 1)?.last,
        };
        let accepted = self.accepted();
        let mut decimal_scale = None;
        let mut part = Vec::new();
        for alias in aliases
            .iter()
            .skip_while(|a| a.span.last < extent.first)
            .take_while(|a| a.span.first <= extent.last)
        {
            self.check()?;
            let mut s = self.source();
            let Some(definition) = s.def(alias.key)? else {
                continue;
            };
            if decimal_scale.is_some_and(|scale| scale != definition.decimals) {
                return Err(invalid("dated aliases use different decimal scales"));
            }
            decimal_scale = Some(definition.decimals);
            let overlap = alias
                .span
                .intersect(extent)
                .ok_or_else(|| invalid("missing alias intersection"))?;
            let first_part = grid.index(overlap.first);
            let last_part = grid.index(overlap.last);
            let length = (last_part - first_part + 1) as usize;
            part.clear();
            engine_allocation::resize(&mut part, length, None)?;
            let selection = Selection {
                key: alias.key,
                membership: Some(std::slice::from_ref(&alias.span)),
            };
            numeric_tiles(&mut s, selection, &definition, grid, first_part, &mut part)?;
            for (index, value) in part.iter().enumerate() {
                if let Some(mut value) = *value {
                    if value.first_hour == alias.span.first {
                        if let Some(boundary) = alias.boundary {
                            value.first_break = boundary;
                        }
                    }
                    let target = &mut buckets[(first_part - first) as usize + index];
                    *target = Some(match *target {
                        Some(previous) => previous.merge(value)?,
                        None => value,
                    });
                }
            }
        }
        for (i, value) in buckets.iter_mut().enumerate() {
            let span = grid.bin(first + i as u64)?;
            let pending = accepted.last.checked_add(1).and_then(|first| {
                span.intersect(Span {
                    first,
                    last: bounds.last,
                })
            });
            sink(
                first + i as u64,
                Bucket {
                    span,
                    exact: *value,
                    pending,
                },
            )?;
        }
        cursor.position = add(first, count as u64)?;
        Ok(PageResult {
            next: (cursor.position < grid.count).then_some(cursor),
            returned: count as u64,
            total: Some(grid.count),
            first_ordinal: first,
        })
    }
}
