use crate::{
    format::{
        BLOCK, Block, COVERAGE, Definition, EVENTS, EVIDENCE, Entry, EventBlock, Events, Late,
        Node, RUNS, RunBlock, Track,
    },
    invalid,
    pages::PageRead,
    query::{EventView, Selection},
    read::{self, Fragment, Reader},
    summary::Summary,
    types::*,
};
use std::{collections::HashMap, io, sync::Arc};

const NO_EVIDENCE: Evidence = Evidence {
    span: Span { first: 0, last: 0 },
    known: Knownness::Known,
    supplying_read: None,
    holder: None,
    identity_first: None,
};
const NO_EVENT: EventView = EventView::Family(Event {
    key: 0,
    native_hour: 0,
    read_hour: None,
    sequence: 0,
    kind: 0,
    fields: Blob::EMPTY,
});

/// Which rows of a subject a walk follows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rows {
    Runs,
    Evidence,
    Coverage,
}
impl Rows {
    fn track(self, entry: &Entry) -> Track {
        match self {
            Self::Runs => entry.runs,
            Self::Evidence => entry.evidence,
            Self::Coverage => entry.coverage,
        }
    }
    fn table(self) -> usize {
        match self {
            Self::Runs => RUNS,
            Self::Evidence => EVIDENCE,
            Self::Coverage => COVERAGE,
        }
    }
}

/// A key's evidence lives with the key or with its shared domain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Subject {
    pub domain: bool,
    pub id: u64,
}
impl Subject {
    pub fn of(key: u64, definition: &Definition) -> Self {
        if definition.domain == NONE {
            Self {
                domain: false,
                id: key,
            }
        } else {
            Self {
                domain: true,
                id: definition.domain,
            }
        }
    }
    pub fn key(key: u64) -> Self {
        Self {
            domain: false,
            id: key,
        }
    }
}

/// Read and visibility state shared by event walks.
#[derive(Clone, Copy)]
pub(crate) struct Visibility {
    pub hour: i32,
    pub unknown: bool,
}
impl Visibility {
    pub fn none(self, stats: &Events) -> bool {
        (stats.read_max < stats.read_min || stats.read_min > self.hour)
            && (!self.unknown || !stats.unknown)
    }
    pub fn all(self, stats: &Events) -> bool {
        stats.read_max <= self.hour && (self.unknown || !stats.unknown)
    }
    pub fn sees(self, read: Option<i32>) -> bool {
        read.map_or(self.unknown, |read| read <= self.hour)
    }
}

/// A summary a fold offers: of a node or block over its run extent, or of one clipped row.
pub(crate) struct Piece {
    pub summary: Summary,
    pub extent: Span,
    pub row: bool,
}

/// What an event walk offers its visitor: a counted aggregate or one event.
pub(crate) enum Item {
    Many { count: u64, first: i32, last: i32 },
    One(EventView),
}

/// One decoded block of a segment entry's run rows, kept while ordinals stay in it.
pub(crate) struct RunRows {
    rows: [Fragment; BLOCK],
    block: u64,
    count: usize,
}
impl Default for RunRows {
    fn default() -> Self {
        Self {
            rows: [Fragment::default(); BLOCK],
            block: NONE,
            count: 0,
        }
    }
}
impl RunRows {
    /// Forgets the decoded block, for rows of another segment entry.
    pub fn clear(&mut self) {
        self.block = NONE;
    }
}

/// `<[T]>::partition_point` over `len` rows read by ordinal, probing in its
/// order, so rows out of hour order select what the current reader selects.
pub(crate) fn partition_point(
    len: u64,
    mut before: impl FnMut(u64) -> io::Result<bool>,
) -> io::Result<u64> {
    if len == 0 {
        return Ok(0);
    }
    let (mut base, mut size) = (0, len);
    while size > 1 {
        let half = size / 2;
        let middle = base + half;
        if before(middle)? {
            base = middle;
        }
        size -= half;
    }
    Ok(base + u64::from(before(base)?))
}

pub(crate) struct Source<'s, P> {
    pub r: &'s mut Reader,
    pub io: &'s mut P,
    pub defs: &'s mut HashMap<u64, Option<Definition>>,
    pub accepted: Span,
}

impl<P: PageRead> Source<'_, P> {
    pub fn def(&mut self, key: u64) -> io::Result<Option<Definition>> {
        if let Some(definition) = self.defs.get(&key) {
            return Ok(*definition);
        }
        let definition = self.r.definition(self.io, key)?;
        self.defs.insert(key, definition);
        Ok(definition)
    }
    /// The entry of `key` in `node` for its events: a held one holds none.
    pub fn event_entry(&mut self, node: &Node, key: u64) -> io::Result<Option<Entry>> {
        Ok(self
            .r
            .entry(self.io, node.entries, key)?
            .filter(|entry| entry.held.is_none()))
    }
    /// The entry of `subject` in `node`; a held one as the rows it continues.
    pub fn entry(&mut self, node: &Node, subject: Subject) -> io::Result<Option<Entry>> {
        match self.shaped(node, subject)? {
            Some(entry) => self.resolved(node, subject, entry).map(Some),
            None => Ok(None),
        }
    }
    /// The entry of `subject` in `node` with a held one's tracks and counters alone: what a seek
    /// or chase tests.
    fn shaped(&mut self, node: &Node, subject: Subject) -> io::Result<Option<Entry>> {
        let dir = if subject.domain {
            node.domains
        } else {
            node.entries
        };
        Ok(self
            .r
            .entry(self.io, dir, subject.id)?
            .map(|entry| entry.shaped(node.hours, node.segments)))
    }
    /// A shaped entry of `node`, a held one read as the rows it continues.
    fn resolved(&mut self, node: &Node, subject: Subject, entry: Entry) -> io::Result<Entry> {
        if entry.held.is_none() {
            return Ok(entry);
        }
        let (mut entry, run) = self.r.held(self.io, node, entry)?;
        if let Some(run) = run {
            let decimals = self
                .def(subject.id)?
                .ok_or_else(|| invalid("series held run of an undefined key"))?
                .decimals;
            entry.summary = Some(Summary::held(run.value, node.hours, decimals));
        }
        Ok(entry)
    }
    pub fn segment(&mut self, hour: i32) -> io::Result<Option<Arc<Node>>> {
        if !self.accepted.contains(hour) {
            return Ok(None);
        }
        self.r.segment_at(self.io, hour)
    }

    /// The first segment from `from` onward (or back) holding rows of `subject`
    /// for which `has` holds, skipping whole nodes that hold none.
    pub fn seek(
        &mut self,
        subject: Subject,
        from: i32,
        forward: bool,
        has: &dyn Fn(&Entry) -> bool,
    ) -> io::Result<Option<(Arc<Node>, Entry)>> {
        let from = if forward {
            from.max(self.accepted.first)
        } else {
            from.min(self.accepted.last)
        };
        if !self.accepted.contains(from) {
            return Ok(None);
        }
        let Some(segment) = self.r.segment_at(self.io, from)? else {
            return Ok(None);
        };
        if let Some(entry) = self.shaped(&segment, subject)?.filter(|e| has(e)) {
            let entry = self.resolved(&segment, subject, entry)?;
            return Ok(Some((segment, entry)));
        }
        let mut edge = if forward {
            segment.hours.last.checked_add(1)
        } else {
            segment.hours.first.checked_sub(1)
        };
        while let Some(at) = edge.filter(|h| self.accepted.contains(*h)) {
            let node = if forward {
                self.r.node_at(self.io, at)?
            } else {
                self.r.node_ending(self.io, at)?
            };
            let node = node.ok_or_else(|| invalid("series hierarchy has a hole"))?;
            if let Some(found) = self.descend(&node, subject, forward, has)? {
                return Ok(Some(found));
            }
            edge = if forward {
                node.hours.last.checked_add(1)
            } else {
                node.hours.first.checked_sub(1)
            };
        }
        Ok(None)
    }
    fn descend(
        &mut self,
        node: &Arc<Node>,
        subject: Subject,
        forward: bool,
        has: &dyn Fn(&Entry) -> bool,
    ) -> io::Result<Option<(Arc<Node>, Entry)>> {
        let Some(entry) = self.shaped(node, subject)?.filter(|e| has(e)) else {
            return Ok(None);
        };
        if node.level == 0 {
            let entry = self.resolved(node, subject, entry)?;
            return Ok(Some((Arc::clone(node), entry)));
        }
        let count = node.children.len();
        for at in 0..count {
            let reference = node.children[if forward { at } else { count - 1 - at }];
            let child = self.r.child(self.io, reference)?;
            if let Some(found) = self.descend(&child, subject, forward, has)? {
                return Ok(Some(found));
            }
        }
        Err(invalid("series node entry without a child holding it"))
    }

    /// Extends a chain of rows ending at `end` through every later node whose
    /// head continues it; a run chain names its canonical, which each node must carry.
    pub fn chase(
        &mut self,
        subject: Subject,
        rows: Rows,
        canonical: Option<u64>,
        mut end: i32,
    ) -> io::Result<i32> {
        loop {
            let Some(next) = end.checked_add(1).filter(|h| *h <= self.accepted.last) else {
                return Ok(end);
            };
            let Some(node) = self.r.node_at(self.io, next)? else {
                return Ok(end);
            };
            let Some(entry) = self.shaped(&node, subject)? else {
                return Ok(end);
            };
            let track = rows.track(&entry);
            if track.count == 0 || !track.head.continues {
                return Ok(end);
            }
            if canonical.is_some_and(|canonical| {
                entry.first_canonical().ok() != Some(canonical)
                    || (track.head.through && entry.canonical_end != entry.canonical_start)
            }) {
                return Err(invalid("series run chain continues another canonical run"));
            }
            end = track.head.end;
            if !track.head.through {
                return Ok(end);
            }
        }
    }

    /// Row `ordinal` of a segment entry's runs, its block decoded once.
    pub fn run_row(
        &mut self,
        segment: &Node,
        entry: &Entry,
        ordinal: u64,
        held: &mut RunRows,
    ) -> io::Result<Fragment> {
        let block = ordinal / BLOCK as u64;
        if held.block != block {
            held.block = NONE;
            let found = self.run_block(segment, entry, block)?.block;
            held.count = self.fragments(segment, entry, found, &mut held.rows)?;
            held.block = block;
        }
        held.rows[..held.count]
            .get((ordinal % BLOCK as u64) as usize)
            .copied()
            .ok_or_else(|| invalid("series run ordinal outside its block"))
    }

    fn blocks(&self, entry: &Entry, rows: Rows) -> io::Result<(u64, u64)> {
        let detail = entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its blocks"))?;
        Ok(detail.blocks[rows.table()])
    }
    pub fn run_block(&mut self, segment: &Node, entry: &Entry, index: u64) -> io::Result<RunBlock> {
        let (first, count) = self.blocks(entry, Rows::Runs)?;
        if index >= count {
            return Err(invalid("series run block outside its key"));
        }
        if entry.held.is_some() {
            return Ok(RunBlock {
                block: held_block(segment),
                summary: entry
                    .summary
                    .ok_or_else(|| invalid("series held run without its summary"))?,
            });
        }
        let table = segment
            .segment
            .ok_or_else(|| invalid("series blocks outside a segment"))?
            .blocks[RUNS];
        self.r
            .record::<RunBlock>(self.io, table, crate::add(first, index)?)
    }
    pub fn plain_block(
        &mut self,
        segment: &Node,
        entry: &Entry,
        rows: Rows,
        index: u64,
    ) -> io::Result<Block> {
        let (first, count) = self.blocks(entry, rows)?;
        if index >= count {
            return Err(invalid("series block outside its subject"));
        }
        if entry.held.is_some() {
            return Ok(held_block(segment));
        }
        let table = segment
            .segment
            .ok_or_else(|| invalid("series blocks outside a segment"))?
            .blocks[rows.table()];
        self.r
            .record::<Block>(self.io, table, crate::add(first, index)?)
    }
    pub fn event_block(
        &mut self,
        segment: &Node,
        entry: &Entry,
        index: u64,
    ) -> io::Result<EventBlock> {
        let detail = entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its blocks"))?;
        let (first, count) = detail.blocks[EVENTS];
        if index >= count {
            return Err(invalid("series event block outside its key"));
        }
        let table = segment
            .segment
            .ok_or_else(|| invalid("series blocks outside a segment"))?
            .blocks[EVENTS];
        self.r
            .record::<EventBlock>(self.io, table, crate::add(first, index)?)
    }
    /// A block of `entry`'s run rows; a held entry's one row continues its run through `segment`.
    pub fn fragments(
        &mut self,
        segment: &Node,
        entry: &Entry,
        block: Block,
        out: &mut [Fragment; BLOCK],
    ) -> io::Result<usize> {
        if entry.held.is_some() {
            let tail = entry
                .detail
                .and_then(|detail| detail.tail)
                .ok_or_else(|| invalid("series held run without its run"))?;
            out[0] = Fragment {
                span: segment.hours,
                value: tail.value,
                break_before: false,
                supplying_boundary: false,
                continues: true,
            };
            return Ok(1);
        }
        let (page, at) = self.r.block(self.io, segment, block)?;
        read::runs(page.bytes(), at, block, out)
    }
    pub fn evidence_rows(
        &mut self,
        segment: &Node,
        entry: &Entry,
        block: Block,
        out: &mut [(Evidence, bool); BLOCK],
    ) -> io::Result<usize> {
        if entry.held.is_some() {
            let tail = entry
                .detail
                .and_then(|detail| detail.evidence_tail)
                .ok_or_else(|| invalid("series held evidence without its evidence"))?;
            out[0] = (
                Evidence {
                    span: segment.hours,
                    ..tail
                },
                true,
            );
            return Ok(1);
        }
        let (page, at) = self.r.block(self.io, segment, block)?;
        read::evidence(page.bytes(), at, block, out)
    }
    pub fn coverage_rows(
        &mut self,
        segment: &Node,
        entry: &Entry,
        block: Block,
        out: &mut [Span; BLOCK],
    ) -> io::Result<usize> {
        if entry.held.is_some() {
            out[0] = segment.hours;
            return Ok(1);
        }
        let (page, at) = self.r.block(self.io, segment, block)?;
        read::coverage(page.bytes(), at, block, out)
    }
    pub fn event_rows(
        &mut self,
        segment: &Node,
        key: u64,
        block: Block,
        out: &mut [EventView; BLOCK],
    ) -> io::Result<usize> {
        let (page, at) = self.r.block(self.io, segment, block)?;
        read::events(page.bytes(), at, key, block, out)
    }
    /// The (native hour, sequence) of a block's first event, its only row decoded.
    pub fn event_head(&mut self, segment: &Node, key: u64, block: Block) -> io::Result<(i32, u64)> {
        let (page, at) = self.r.block(self.io, segment, block)?;
        let event = read::EventDecoder::new(page, at, key, block)
            .next()?
            .ok_or_else(|| invalid("series event block without rows"))?;
        Ok((event.native_hour(), event.sequence()))
    }

    /// The canonical run of row `row` of `entry` in `segment`.
    pub fn canonical(
        &mut self,
        key: u64,
        segment: &Node,
        entry: &Entry,
        row: u64,
        fragment: Fragment,
    ) -> io::Result<Run> {
        let detail = entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its detail"))?;
        let (first, break_before, supplying_boundary, sequence) = if row == 0 && fragment.continues
        {
            (
                detail.origin.first,
                detail.origin.break_before,
                detail.origin.supplying_boundary,
                detail.origin.sequence,
            )
        } else {
            (
                fragment.span.first,
                fragment.break_before,
                fragment.supplying_boundary,
                crate::add(entry.row_start, row)?,
            )
        };
        let canonical = crate::add(entry.first_canonical()?, row)?;
        // Only the segment's last row is carried into the next segment.
        let last = if crate::add(row, 1)? == entry.runs.count
            && fragment.span.last == segment.hours.last
        {
            self.chase(
                Subject::key(key),
                Rows::Runs,
                Some(canonical),
                fragment.span.last,
            )?
        } else {
            fragment.span.last
        };
        Ok(Run {
            span: Span::new(first, last)?,
            value: fragment.value,
            break_before,
            supplying_boundary,
            canonical,
            sequence,
        })
    }

    /// The run row a key holds at `hour`, chosen as the current reader's
    /// partition point chooses it, with its canonical bounds.
    pub fn point_run(
        &mut self,
        key: u64,
        definition: &Definition,
        hour: i32,
    ) -> io::Result<Option<Run>> {
        let Some(segment) = self.segment(hour)? else {
            return Ok(None);
        };
        let Some(entry) = self.entry(&segment, Subject::key(key))? else {
            return Ok(None);
        };
        if entry.runs.count == 0 {
            return Ok(None);
        }
        let found = if definition.mode == RunMode::Exact {
            let mut rows = [Fragment::default(); BLOCK];
            let count = self.blocks(&entry, Rows::Runs)?.1;
            let (mut low, mut high) = (0, count);
            while low < high {
                let middle = low + (high - low) / 2;
                if self.run_block(&segment, &entry, middle)?.block.first <= hour {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            let Some(index) = low.checked_sub(1) else {
                return Ok(None);
            };
            let block = self.run_block(&segment, &entry, index)?.block;
            let n = self.fragments(&segment, &entry, block, &mut rows)?;
            match rows[..n].iter().position(|row| row.span.contains(hour)) {
                Some(at) => Some((row_ordinal(index, at)?, rows[at])),
                None => None,
            }
        } else {
            let mut held = RunRows::default();
            let index = partition_point(entry.runs.count, |ordinal| {
                Ok(self
                    .run_row(&segment, &entry, ordinal, &mut held)?
                    .span
                    .first
                    <= hour)
            })?;
            match index.checked_sub(1) {
                None => None,
                Some(ordinal) => {
                    let row = self.run_row(&segment, &entry, ordinal, &mut held)?;
                    row.span.contains(hour).then_some((ordinal, row))
                }
            }
        };
        match found {
            Some((ordinal, fragment)) => self
                .canonical(key, &segment, &entry, ordinal, fragment)
                .map(Some),
            None => Ok(None),
        }
    }

    /// The canonical run of an exact key holding `hour`, else the next one after it.
    pub fn run_from(&mut self, key: u64, hour: i32) -> io::Result<Option<Run>> {
        let subject = Subject::key(key);
        let Some((segment, entry)) = self.seek(subject, hour, true, &|e| {
            e.runs.count != 0 && e.runs.last >= hour
        })?
        else {
            return Ok(None);
        };
        let count = self.blocks(&entry, Rows::Runs)?.1;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.run_block(&segment, &entry, middle)?.block.last < hour {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut rows = [Fragment::default(); BLOCK];
        for index in low..count {
            let block = self.run_block(&segment, &entry, index)?.block;
            let n = self.fragments(&segment, &entry, block, &mut rows)?;
            if let Some(at) = rows[..n].iter().position(|row| row.span.last >= hour) {
                return self
                    .canonical(key, &segment, &entry, row_ordinal(index, at)?, rows[at])
                    .map(Some);
            }
        }
        Err(invalid("series run extent does not match its rows"))
    }
    /// The canonical run of an exact key holding `hour`, else the last one before it.
    pub fn run_before(&mut self, key: u64, hour: i32) -> io::Result<Option<Run>> {
        let subject = Subject::key(key);
        let Some((segment, entry)) = self.seek(subject, hour, false, &|e| {
            e.runs.count != 0 && e.runs.first <= hour
        })?
        else {
            return Ok(None);
        };
        let count = self.blocks(&entry, Rows::Runs)?.1;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.run_block(&segment, &entry, middle)?.block.first <= hour {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let index = low
            .checked_sub(1)
            .ok_or_else(|| invalid("series run extent does not match its rows"))?;
        let block = self.run_block(&segment, &entry, index)?.block;
        let mut rows = [Fragment::default(); BLOCK];
        let n = self.fragments(&segment, &entry, block, &mut rows)?;
        let at = rows[..n]
            .iter()
            .rposition(|row| row.span.first <= hour)
            .ok_or_else(|| invalid("series run extent does not match its rows"))?;
        self.canonical(key, &segment, &entry, row_ordinal(index, at)?, rows[at])
            .map(Some)
    }

    /// The first segment from `from` onward holding runs of `key` that reach `from`.
    pub fn run_segment(&mut self, key: u64, from: i32) -> io::Result<Option<(Arc<Node>, Entry)>> {
        self.seek(Subject::key(key), from, true, &|e| {
            e.runs.count != 0 && e.runs.last >= from
        })
    }
    /// A segment's run rows of `key` in storage order, each with its canonical identity.
    pub fn segment_runs(
        &mut self,
        segment: &Node,
        entry: &Entry,
        visit: &mut dyn FnMut(Run) -> io::Result<()>,
    ) -> io::Result<()> {
        let detail = entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its detail"))?;
        let first_canonical = entry.first_canonical()?;
        let count = self.blocks(entry, Rows::Runs)?.1;
        let mut rows = [Fragment::default(); BLOCK];
        for index in 0..count {
            let block = self.run_block(segment, entry, index)?.block;
            let n = self.fragments(segment, entry, block, &mut rows)?;
            for (at, row) in rows[..n].iter().enumerate() {
                let ordinal = row_ordinal(index, at)?;
                let head = ordinal == 0 && row.continues;
                visit(Run {
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
                    canonical: crate::add(first_canonical, ordinal)?,
                    sequence: if head {
                        detail.origin.sequence
                    } else {
                        crate::add(entry.row_start, ordinal)?
                    },
                })?;
            }
        }
        Ok(())
    }
    /// Every run row of `key` in segments meeting `span`, in storage order.
    pub fn contributions(
        &mut self,
        key: u64,
        span: Span,
        visit: &mut dyn FnMut(Run) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut at = span.first;
        loop {
            let Some((segment, entry)) = self.run_segment(key, at)? else {
                return Ok(());
            };
            if segment.hours.first > span.last {
                return Ok(());
            }
            self.segment_runs(&segment, &entry, visit)?;
            match segment.hours.last.checked_add(1) {
                Some(next) if next <= span.last => at = next,
                _ => return Ok(()),
            }
        }
    }

    /// Offers a key's runs within `span` in storage order: each node's and
    /// block's summary over its run extent first, then, unless the visitor
    /// consumed it, its parts; rows arrive clipped to `span` and must be consumed.
    pub fn fold(
        &mut self,
        key: u64,
        decimals: u8,
        sorted: bool,
        span: Span,
        visit: &mut dyn FnMut(Piece) -> io::Result<bool>,
    ) -> io::Result<()> {
        let Some(span) = span.intersect(self.accepted) else {
            return Ok(());
        };
        let frontier = self.r.root().nodes.clone();
        for reference in frontier {
            if reference.hours.intersect(span).is_none() {
                continue;
            }
            let node = self.r.child(self.io, reference)?;
            self.fold_node(&node, key, decimals, sorted, span, visit)?;
        }
        Ok(())
    }
    fn fold_node(
        &mut self,
        node: &Arc<Node>,
        key: u64,
        decimals: u8,
        sorted: bool,
        span: Span,
        visit: &mut dyn FnMut(Piece) -> io::Result<bool>,
    ) -> io::Result<()> {
        let Some(entry) = self.entry(node, Subject::key(key))? else {
            return Ok(());
        };
        let Some(summary) = entry.summary else {
            return Ok(());
        };
        let extent = Span::new(entry.runs.first, entry.runs.last)?;
        if extent.intersect(span).is_none()
            || visit(Piece {
                summary,
                extent,
                row: false,
            })?
        {
            return Ok(());
        }
        if node.level > 0 {
            for reference in node.children.clone() {
                if reference.hours.intersect(span).is_none() {
                    continue;
                }
                let child = self.r.child(self.io, reference)?;
                self.fold_node(&child, key, decimals, sorted, span, visit)?;
            }
            return Ok(());
        }
        let count = self.blocks(&entry, Rows::Runs)?.1;
        let mut low = 0;
        if sorted {
            let mut high = count;
            while low < high {
                let middle = low + (high - low) / 2;
                if self.run_block(node, &entry, middle)?.block.last < span.first {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
        }
        let mut rows = [Fragment::default(); BLOCK];
        for index in low..count {
            let block = self.run_block(node, &entry, index)?;
            if sorted && block.block.first > span.last {
                break;
            }
            let extent = Span::new(block.block.first, block.block.last)?;
            if extent.intersect(span).is_none()
                || visit(Piece {
                    summary: block.summary,
                    extent,
                    row: false,
                })?
            {
                continue;
            }
            let n = self.fragments(node, &entry, block.block, &mut rows)?;
            for row in &rows[..n] {
                if let Some(part) = row.span.intersect(span) {
                    visit(Piece {
                        summary: Summary::run(as_run(*row), part, decimals),
                        extent: part,
                        row: true,
                    })?;
                }
            }
        }
        Ok(())
    }

    /// The evidence row (coalesced across segments) holding `hour`, else the next one.
    pub fn evidence_from(&mut self, subject: Subject, hour: i32) -> io::Result<Option<Evidence>> {
        let Some((segment, entry)) = self.seek(subject, hour, true, &|e| {
            e.evidence.count != 0 && e.evidence.last >= hour
        })?
        else {
            return Ok(None);
        };
        let count = self.blocks(&entry, Rows::Evidence)?.1;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self
                .plain_block(&segment, &entry, Rows::Evidence, middle)?
                .last
                < hour
            {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut rows = [(NO_EVIDENCE, false); BLOCK];
        for index in low..count {
            let block = self.plain_block(&segment, &entry, Rows::Evidence, index)?;
            let n = self.evidence_rows(&segment, &entry, block, &mut rows)?;
            if let Some(at) = rows[..n].iter().position(|(row, _)| row.span.last >= hour) {
                let (mut row, continues) = rows[at];
                if continues && index == 0 && at == 0 {
                    row.span.first = entry
                        .detail
                        .ok_or_else(|| invalid("series segment entry without its detail"))?
                        .evidence_origin;
                }
                if row.span.last == segment.hours.last {
                    row.span.last = self.chase(subject, Rows::Evidence, None, row.span.last)?;
                }
                return Ok(Some(row));
            }
        }
        Err(invalid("series evidence extent does not match its rows"))
    }
    /// The evidence row holding `hour`, else the last one before it.
    pub fn evidence_before(&mut self, subject: Subject, hour: i32) -> io::Result<Option<Evidence>> {
        let Some((segment, entry)) = self.seek(subject, hour, false, &|e| {
            e.evidence.count != 0 && e.evidence.first <= hour
        })?
        else {
            return Ok(None);
        };
        let count = self.blocks(&entry, Rows::Evidence)?.1;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self
                .plain_block(&segment, &entry, Rows::Evidence, middle)?
                .first
                <= hour
            {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let index = low
            .checked_sub(1)
            .ok_or_else(|| invalid("series evidence extent does not match its rows"))?;
        let block = self.plain_block(&segment, &entry, Rows::Evidence, index)?;
        let mut rows = [(NO_EVIDENCE, false); BLOCK];
        let n = self.evidence_rows(&segment, &entry, block, &mut rows)?;
        let at = rows[..n]
            .iter()
            .rposition(|(row, _)| row.span.first <= hour)
            .ok_or_else(|| invalid("series evidence extent does not match its rows"))?;
        let (mut row, continues) = rows[at];
        if continues && index == 0 && at == 0 {
            row.span.first = entry
                .detail
                .ok_or_else(|| invalid("series segment entry without its detail"))?
                .evidence_origin;
        }
        if row.span.last == segment.hours.last {
            row.span.last = self.chase(subject, Rows::Evidence, None, row.span.last)?;
        }
        Ok(Some(row))
    }
    pub fn evidence_at(&mut self, subject: Subject, hour: i32) -> io::Result<Option<Evidence>> {
        Ok(self
            .evidence_from(subject, hour)?
            .filter(|row| row.span.contains(hour)))
    }

    /// The known span (coalesced across segments) holding `hour`, else the next one.
    pub fn coverage_from(&mut self, subject: Subject, hour: i32) -> io::Result<Option<Span>> {
        let Some((segment, entry)) = self.seek(subject, hour, true, &|e| {
            e.coverage.count != 0 && e.coverage.last >= hour
        })?
        else {
            return Ok(None);
        };
        let count = self.blocks(&entry, Rows::Coverage)?.1;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self
                .plain_block(&segment, &entry, Rows::Coverage, middle)?
                .last
                < hour
            {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut rows = [Span::default(); BLOCK];
        for index in low..count {
            let block = self.plain_block(&segment, &entry, Rows::Coverage, index)?;
            let n = self.coverage_rows(&segment, &entry, block, &mut rows)?;
            if let Some(at) = rows[..n].iter().position(|row| row.last >= hour) {
                let mut row = rows[at];
                if row.last == segment.hours.last {
                    row.last = self.chase(subject, Rows::Coverage, None, row.last)?;
                }
                return Ok(Some(row));
            }
        }
        Err(invalid("series coverage extent does not match its rows"))
    }

    /// Visits a key's visible on-time and late events with native hour in
    /// `span`: aggregates when `whole` accepts them, else single events.
    pub fn walk_events(
        &mut self,
        key: u64,
        selection: Selection<'_>,
        span: Span,
        visible: Visibility,
        whole: &mut dyn FnMut(&mut Self, Item) -> io::Result<bool>,
    ) -> io::Result<()> {
        let frontier = self.r.root().nodes.clone();
        for reference in &frontier {
            if reference.hours.intersect(span).is_none() {
                continue;
            }
            let node = self.r.child(self.io, *reference)?;
            self.walk_node(&node, key, selection, span, visible, whole)?;
        }
        for reference in frontier {
            let node = self.r.child(self.io, reference)?;
            let Some(entry) = self.event_entry(&node, key)? else {
                continue;
            };
            if entry.late.count == 0 || entry.late.first > span.last || entry.late.last < span.first
            {
                continue;
            }
            if eligible(&entry.late, selection, span, visible)
                && whole(
                    self,
                    Item::Many {
                        count: entry.late.count,
                        first: entry.late.first,
                        last: entry.late.last,
                    },
                )?
            {
                continue;
            }
            let (low, high) = self.late_range(&node, &entry, span)?;
            for index in low..high {
                let event = self.r.record::<Late>(self.io, node.late, index)?.0;
                if event.key() != key {
                    return Err(invalid("series late event of another key"));
                }
                if visible.sees(event.read_hour()) && selection.includes(event.native_hour()) {
                    whole(self, Item::One(event))?;
                }
            }
        }
        Ok(())
    }
    /// The late records of `entry` in `node` whose native hour lies in `span`.
    pub fn late_range(&mut self, node: &Node, entry: &Entry, span: Span) -> io::Result<(u64, u64)> {
        self.late_from(node, entry, (span.first, 0), span.last)
    }
    /// The late records of `entry` in `node` from position `low` through native hour `last`.
    pub fn late_from(
        &mut self,
        node: &Node,
        entry: &Entry,
        low: (i32, u64),
        last: i32,
    ) -> io::Result<(u64, u64)> {
        let first = entry.late_at;
        let end = crate::add(first, entry.late.count)?;
        let bound = |source: &mut Self, at: (i64, u64)| -> io::Result<u64> {
            let (mut low, mut high) = (first, end);
            while low < high {
                let middle = low + (high - low) / 2;
                let late = source.r.record::<Late>(source.io, node.late, middle)?;
                if (i64::from(late.native()), late.0.sequence()) < at {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            Ok(low)
        };
        let from = bound(self, (i64::from(low.0), low.1))?;
        let to = bound(self, (i64::from(last) + 1, 0))?;
        Ok((from, to))
    }
    fn walk_node(
        &mut self,
        node: &Arc<Node>,
        key: u64,
        selection: Selection<'_>,
        span: Span,
        visible: Visibility,
        whole: &mut dyn FnMut(&mut Self, Item) -> io::Result<bool>,
    ) -> io::Result<()> {
        let Some(entry) = self.event_entry(node, key)? else {
            return Ok(());
        };
        let stats = entry.events;
        if stats.count == 0
            || stats.first > span.last
            || stats.last < span.first
            || visible.none(&stats)
            || !intersects(
                selection,
                Span {
                    first: stats.first,
                    last: stats.last,
                },
            )
        {
            return Ok(());
        }
        if eligible(&stats, selection, span, visible)
            && whole(
                self,
                Item::Many {
                    count: stats.count,
                    first: stats.first,
                    last: stats.last,
                },
            )?
        {
            return Ok(());
        }
        if node.level > 0 {
            for reference in node.children.clone() {
                if reference.hours.intersect(span).is_none() {
                    continue;
                }
                let child = self.r.child(self.io, reference)?;
                self.walk_node(&child, key, selection, span, visible, whole)?;
            }
            return Ok(());
        }
        let count = entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its blocks"))?
            .blocks[EVENTS]
            .1;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.event_block(node, &entry, middle)?.block.last < span.first {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut rows = [NO_EVENT; BLOCK];
        for index in low..count {
            let block = self.event_block(node, &entry, index)?;
            if block.block.first > span.last {
                break;
            }
            let stats = Events {
                count: u64::from(block.block.rows),
                first: block.block.first,
                last: block.block.last,
                read_min: block.read_min,
                read_max: block.read_max,
                unknown: block.unknown,
            };
            if stats.first > span.last || stats.last < span.first || visible.none(&stats) {
                continue;
            }
            if eligible(&stats, selection, span, visible)
                && whole(
                    self,
                    Item::Many {
                        count: stats.count,
                        first: stats.first,
                        last: stats.last,
                    },
                )?
            {
                continue;
            }
            let n = self.event_rows(node, key, block.block, &mut rows)?;
            for event in &rows[..n] {
                let hour = event.native_hour();
                if span.contains(hour)
                    && visible.sees(event.read_hour())
                    && selection.includes(hour)
                {
                    whole(self, Item::One(*event))?;
                }
            }
        }
        Ok(())
    }
}

fn eligible(stats: &Events, selection: Selection<'_>, span: Span, visible: Visibility) -> bool {
    let extent = Span {
        first: stats.first,
        last: stats.last,
    };
    span.first <= extent.first
        && extent.last <= span.last
        && visible.all(stats)
        && covers(selection, extent)
}
pub(crate) fn covers(selection: Selection<'_>, span: Span) -> bool {
    selection.membership.is_none_or(|members| {
        members
            .get(members.partition_point(|s| s.last < span.first))
            .is_some_and(|s| s.first <= span.first && s.last >= span.last)
    })
}
pub(crate) fn intersects(selection: Selection<'_>, span: Span) -> bool {
    selection.membership.is_none_or(|members| {
        members
            .get(members.partition_point(|s| s.last < span.first))
            .is_some_and(|s| s.first <= span.last)
    })
}

/// The one block of a held entry's table: its row spans the segment.
fn held_block(segment: &Node) -> Block {
    Block {
        page: NONE,
        offset: 0,
        rows: 1,
        first: segment.hours.first,
        last: segment.hours.last,
    }
}

/// The ordinal of row `at` of block `index`.
fn row_ordinal(index: u64, at: usize) -> io::Result<u64> {
    index
        .checked_mul(BLOCK as u64)
        .and_then(|first| first.checked_add(at as u64))
        .ok_or_else(|| invalid("series row ordinal"))
}

pub(crate) fn as_run(row: Fragment) -> Run {
    Run {
        span: row.span,
        value: row.value,
        break_before: row.break_before,
        supplying_boundary: row.supplying_boundary,
        canonical: 0,
        sequence: 0,
    }
}
