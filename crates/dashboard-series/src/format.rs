use crate::{
    add,
    codec::{Binary, Decode, Encode, Sink},
    invalid,
    query::EventView,
    summary::Summary,
    types::*,
};
use std::io;

/// Rows per block: blocks are the unit of seeks, summaries and decoding.
pub(crate) const BLOCK: usize = 32;
/// Children per hierarchy node.
pub(crate) const FANOUT: usize = 16;

fn put_span(s: &mut Sink, first: i32, last: i32) -> io::Result<()> {
    s.zig(i64::from(first))?;
    s.var((i64::from(last) - i64::from(first)) as u64)
}
fn get_span(d: &mut Decode<'_>) -> io::Result<(i32, i32)> {
    let first = i32::try_from(d.zig()?).map_err(|_| invalid("series hour width"))?;
    let last = i64::from(first)
        .checked_add(i64::try_from(d.var()?).map_err(|_| invalid("series extent"))?)
        .and_then(|h| i32::try_from(h).ok())
        .ok_or_else(|| invalid("series extent"))?;
    Ok((first, last))
}
pub(crate) fn hour(d: &mut Decode<'_>) -> io::Result<i32> {
    i32::try_from(d.zig()?).map_err(|_| invalid("series hour width"))
}

/// Page ids of one paged object: level 0 means `root` is its only page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct List {
    pub root: PageId,
    pub levels: u8,
    pub pages: u64,
}
impl List {
    pub const EMPTY: Self = Self {
        root: NONE,
        levels: 0,
        pages: 0,
    };
}
impl Binary for List {
    const SIZE: usize = 17;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.root)?;
        e.u8(self.levels)?;
        e.u64(self.pages)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let v = Self {
            root: d.u64()?,
            levels: d.u8()?,
            pages: d.u64()?,
        };
        if (v.pages == 0) != (v.root == NONE) || (v.pages > 1 && v.levels == 0) || v.levels > 8 {
            return Err(invalid("series page list"));
        }
        Ok(v)
    }
}

/// Fixed-width records, `PAGE_BYTES / T::SIZE` per page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Table {
    pub list: List,
    pub count: u64,
}
impl Table {
    pub const EMPTY: Self = Self {
        list: List::EMPTY,
        count: 0,
    };
}
impl Binary for Table {
    const SIZE: usize = List::SIZE + 8;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.list.encode(e)?;
        e.u64(self.count)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            list: List::decode(d)?,
            count: d.u64()?,
        })
    }
}

/// Variable-width keyed entries: each page is `n`, `n` sorted keys, `n + 1`
/// offsets and the encoded entries; `fences` holds each page's first key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Directory {
    pub list: List,
    pub fences: Table,
    pub count: u64,
}
impl Binary for Directory {
    const SIZE: usize = List::SIZE + Table::SIZE + 8;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.list.encode(e)?;
        self.fences.encode(e)?;
        e.u64(self.count)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            list: List::decode(d)?,
            fences: Table::decode(d)?,
            count: d.u64()?,
        })
    }
}

/// Whether a node's first row continues a row from before the node, where that
/// chain ends inside it, and whether it runs through the node's last hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Chain {
    pub continues: bool,
    pub end: i32,
    pub through: bool,
}
/// One row table's rows of a key in a node: count, extent and head chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Track {
    pub count: u64,
    pub first: i32,
    pub last: i32,
    pub head: Chain,
}
impl Track {
    fn put(&self, s: &mut Sink) -> io::Result<()> {
        s.var(self.count)?;
        if self.count == 0 {
            return Ok(());
        }
        put_span(s, self.first, self.last)?;
        s.u8(u8::from(self.head.continues) | u8::from(self.head.through) << 1)?;
        if self.head.continues {
            s.var((i64::from(self.head.end) - i64::from(self.first)) as u64)?;
        }
        Ok(())
    }
    fn get(d: &mut Decode<'_>) -> io::Result<Self> {
        let count = d.var()?;
        if count == 0 {
            return Ok(Self::default());
        }
        let (first, last) = get_span(d)?;
        let flags = d.u8()?;
        if flags > 3 || (flags == 2) {
            return Err(invalid("series chain flags"));
        }
        let continues = flags & 1 != 0;
        let end = if continues {
            i64::from(first)
                .checked_add(i64::try_from(d.var()?).map_err(|_| invalid("series chain"))?)
                .and_then(|h| i32::try_from(h).ok())
                .filter(|h| *h <= last)
                .ok_or_else(|| invalid("series chain end"))?
        } else {
            first
        };
        Ok(Self {
            count,
            first,
            last,
            head: Chain {
                continues,
                end,
                through: flags & 2 != 0,
            },
        })
    }
}

/// Event counts and the native and read extents that decide visibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Events {
    pub count: u64,
    pub first: i32,
    pub last: i32,
    pub read_min: i32,
    pub read_max: i32,
    pub unknown: bool,
}
impl Events {
    pub fn add(&mut self, native: i32, read: Option<i32>) -> io::Result<()> {
        if self.count == 0 {
            *self = Self {
                count: 0,
                first: native,
                last: native,
                read_min: i32::MAX,
                read_max: i32::MIN,
                unknown: false,
            };
        }
        self.count = add(self.count, 1)?;
        self.first = self.first.min(native);
        self.last = self.last.max(native);
        match read {
            Some(read) => {
                self.read_min = self.read_min.min(read);
                self.read_max = self.read_max.max(read);
            }
            None => self.unknown = true,
        }
        Ok(())
    }
    pub fn merge(self, b: Self) -> io::Result<Self> {
        if self.count == 0 {
            return Ok(b);
        }
        if b.count == 0 {
            return Ok(self);
        }
        Ok(Self {
            count: add(self.count, b.count)?,
            first: self.first.min(b.first),
            last: self.last.max(b.last),
            read_min: self.read_min.min(b.read_min),
            read_max: self.read_max.max(b.read_max),
            unknown: self.unknown || b.unknown,
        })
    }
    fn put(&self, s: &mut Sink) -> io::Result<()> {
        s.var(self.count)?;
        if self.count == 0 {
            return Ok(());
        }
        put_span(s, self.first, self.last)?;
        s.u8(u8::from(self.unknown) | u8::from(self.read_min <= self.read_max) << 1)?;
        if self.read_min <= self.read_max {
            put_span(s, self.read_min, self.read_max)?;
        }
        Ok(())
    }
    fn get(d: &mut Decode<'_>) -> io::Result<Self> {
        let count = d.var()?;
        if count == 0 {
            return Ok(Self::default());
        }
        let (first, last) = get_span(d)?;
        let flags = d.u8()?;
        if flags > 3 || flags == 0 {
            return Err(invalid("series event visibility flags"));
        }
        let (read_min, read_max) = if flags & 2 != 0 {
            get_span(d)?
        } else {
            (i32::MAX, i32::MIN)
        };
        Ok(Self {
            count,
            first,
            last,
            read_min,
            read_max,
            unknown: flags & 1 != 0,
        })
    }
}

/// Where a segment's continuing head run began.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Origin {
    pub first: i32,
    pub break_before: bool,
    pub supplying_boundary: bool,
    pub sequence: u64,
}

/// One key's (or shared evidence domain's) aggregate over a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Entry {
    pub key: u64,
    pub runs: Track,
    pub row_start: u64,
    pub canonical_start: u64,
    pub canonical_end: u64,
    pub summary: Option<Summary>,
    pub evidence: Track,
    pub coverage: Track,
    pub events: Events,
    pub late: Events,
    pub late_at: u64,
    pub detail: Option<Detail>,
    pub last_row: Option<LastRow>,
}
/// A shared domain's last appended row, as the writer's handle names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LastRow {
    pub segment: u64,
    pub row: u32,
    pub value: Evidence,
}
/// What only a segment's entry holds: its blocks, head origins and tail edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Detail {
    pub blocks: [(u64, u64); 4],
    pub origin: Origin,
    pub evidence_origin: i32,
    pub tail: Option<Run>,
    pub evidence_tail: Option<Evidence>,
}
pub(crate) const RUNS: usize = 0;
pub(crate) const EVIDENCE: usize = 1;
pub(crate) const COVERAGE: usize = 2;
pub(crate) const EVENTS: usize = 3;

impl Entry {
    pub fn empty(key: u64) -> Self {
        Self {
            key,
            runs: Track::default(),
            row_start: 0,
            canonical_start: 0,
            canonical_end: 0,
            summary: None,
            evidence: Track::default(),
            coverage: Track::default(),
            events: Events::default(),
            late: Events::default(),
            late_at: 0,
            detail: None,
            last_row: None,
        }
    }
    pub fn put(&self, s: &mut Sink) -> io::Result<()> {
        self.runs.put(s)?;
        s.var(self.row_start)?;
        s.var(self.canonical_start)?;
        s.var(
            self.canonical_end
                .checked_sub(self.canonical_start)
                .ok_or_else(|| invalid("series canonical counters"))?,
        )?;
        match self.summary {
            Some(summary) => {
                s.u8(1)?;
                s.record(&summary)?;
            }
            None => s.u8(0)?,
        }
        self.evidence.put(s)?;
        self.coverage.put(s)?;
        self.events.put(s)?;
        self.late.put(s)?;
        if self.late.count != 0 {
            s.var(self.late_at)?;
        }
        s.u8(u8::from(self.detail.is_some()) | u8::from(self.last_row.is_some()) << 1)?;
        if let Some(detail) = &self.detail {
            for (first, count) in detail.blocks {
                s.var(count)?;
                if count != 0 {
                    s.var(first)?;
                }
            }
            if self.runs.head.continues {
                s.zig(i64::from(detail.origin.first))?;
                s.u8(u8::from(detail.origin.break_before)
                    | u8::from(detail.origin.supplying_boundary) << 1)?;
                s.var(detail.origin.sequence)?;
            }
            if self.evidence.head.continues {
                s.zig(i64::from(detail.evidence_origin))?;
            }
            match detail.tail {
                Some(run) => {
                    s.u8(1)?;
                    s.record(&run)?;
                }
                None => s.u8(0)?,
            }
            match detail.evidence_tail {
                Some(evidence) => {
                    s.u8(1)?;
                    s.record(&evidence)?;
                }
                None => s.u8(0)?,
            }
        }
        if let Some(row) = self.last_row {
            s.var(row.segment)?;
            s.var(u64::from(row.row))?;
            s.record(&row.value)?;
        }
        Ok(())
    }
    pub fn get(key: u64, d: &mut Decode<'_>) -> io::Result<Self> {
        let runs = Track::get(d)?;
        let row_start = d.var()?;
        let canonical_start = d.var()?;
        let canonical_end = add(canonical_start, d.var()?)?;
        let summary = if d.flag()? {
            Some(Summary::decode(d)?)
        } else {
            None
        };
        let evidence = Track::get(d)?;
        let coverage = Track::get(d)?;
        let events = Events::get(d)?;
        let late = Events::get(d)?;
        let late_at = if late.count != 0 { d.var()? } else { 0 };
        let parts = d.u8()?;
        if parts > 3 {
            return Err(invalid("series entry parts"));
        }
        let detail = if parts & 1 != 0 {
            let mut blocks = [(0, 0); 4];
            for block in &mut blocks {
                let count = d.var()?;
                *block = (if count != 0 { d.var()? } else { 0 }, count);
            }
            let origin = if runs.head.continues {
                let first = hour(d)?;
                let flags = d.u8()?;
                if flags > 3 {
                    return Err(invalid("series origin flags"));
                }
                Origin {
                    first,
                    break_before: flags & 1 != 0,
                    supplying_boundary: flags & 2 != 0,
                    sequence: d.var()?,
                }
            } else {
                Origin::default()
            };
            let evidence_origin = if evidence.head.continues { hour(d)? } else { 0 };
            let tail = if d.flag()? {
                Some(Run::decode(d)?)
            } else {
                None
            };
            let evidence_tail = if d.flag()? {
                Some(Evidence::decode(d)?)
            } else {
                None
            };
            Some(Detail {
                blocks,
                origin,
                evidence_origin,
                tail,
                evidence_tail,
            })
        } else {
            None
        };
        let last_row = if parts & 2 != 0 {
            Some(LastRow {
                segment: d.var()?,
                row: u32::try_from(d.var()?).map_err(|_| invalid("series domain row"))?,
                value: Evidence::decode(d)?,
            })
        } else {
            None
        };
        if canonical_end - canonical_start > runs.count || summary.is_some() != (runs.count != 0) {
            return Err(invalid("series entry counters"));
        }
        Ok(Self {
            key,
            runs,
            row_start,
            canonical_start,
            canonical_end,
            summary,
            evidence,
            coverage,
            events,
            late,
            late_at,
            detail,
            last_row,
        })
    }
    /// The canonical id of this node's first run row.
    pub fn first_canonical(&self) -> io::Result<u64> {
        if self.runs.head.continues {
            self.canonical_start
                .checked_sub(1)
                .ok_or_else(|| invalid("series continuation without a canonical run"))
        } else {
            Ok(self.canonical_start)
        }
    }
}

/// One block of a key's rows: data page ordinal, byte offset, row count and extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    pub page: u64,
    pub offset: u32,
    pub rows: u32,
    pub first: i32,
    pub last: i32,
}
impl Binary for Block {
    const SIZE: usize = 24;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.page)?;
        e.u32(self.offset)?;
        e.u32(self.rows)?;
        e.i32(self.first)?;
        e.i32(self.last)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let v = Self {
            page: d.u64()?,
            offset: d.u32()?,
            rows: d.u32()?,
            first: d.i32()?,
            last: d.i32()?,
        };
        if v.rows == 0 || v.rows as usize > BLOCK {
            return Err(invalid("series block rows"));
        }
        Ok(v)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RunBlock {
    pub block: Block,
    pub summary: Summary,
}
impl Binary for RunBlock {
    const SIZE: usize = Block::SIZE + Summary::SIZE;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.block.encode(e)?;
        self.summary.encode(e)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            block: Block::decode(d)?,
            summary: Summary::decode(d)?,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EventBlock {
    pub block: Block,
    pub read_min: i32,
    pub read_max: i32,
    pub unknown: bool,
}
impl Binary for EventBlock {
    const SIZE: usize = Block::SIZE + 9;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.block.encode(e)?;
        e.i32(self.read_min)?;
        e.i32(self.read_max)?;
        e.u8(u8::from(self.unknown))
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            block: Block::decode(d)?,
            read_min: d.i32()?,
            read_max: d.i32()?,
            unknown: d.flag()?,
        })
    }
}

/// A late event: its key, then the event as a family event or evidence presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Late(pub EventView);
impl Late {
    pub fn native(&self) -> i32 {
        self.0.native_hour()
    }
    pub fn read(&self) -> Option<i32> {
        self.0.read_hour()
    }
}
impl Binary for Late {
    const SIZE: usize = 1 + 8 + 8 + 33;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        match self.0 {
            EventView::Family(event) => {
                e.u8(0)?;
                e.u64(event.key)?;
                e.u64(event.sequence)?;
                e.i32(event.native_hour)?;
                e.u8(u8::from(event.read_hour.is_some()))?;
                e.i32(event.read_hour.unwrap_or(0))?;
                e.u64(event.kind)?;
                event.fields.encode(e)
            }
            EventView::Evidence {
                key,
                sequence,
                value,
            } => {
                e.u8(1)?;
                e.u64(key)?;
                e.u64(sequence)?;
                value.encode(e)
            }
        }
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        #[cfg(test)]
        crate::read::DECODED_EVENTS.with(|decoded| decoded.set(decoded.get() + 1));
        let tag = d.u8()?;
        let key = d.u64()?;
        let sequence = d.u64()?;
        Ok(Self(match tag {
            0 => {
                let native_hour = d.i32()?;
                let flag = d.flag()?;
                let read = d.i32()?;
                let kind = d.u64()?;
                let fields = Blob::decode(d)?;
                if fields.len % Cell::SIZE as u64 != 0 {
                    return Err(invalid("event field table length"));
                }
                EventView::Family(Event {
                    key,
                    native_hour,
                    read_hour: flag.then_some(read),
                    sequence,
                    kind,
                    fields,
                })
            }
            1 => EventView::Evidence {
                key,
                sequence,
                value: Evidence::decode(d)?,
            },
            _ => return Err(invalid("series late event tag")),
        }))
    }
}

/// A key's definition with what readers need to fold and find its evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Definition {
    pub key: KeyDefinition,
    pub dimensions: [u8; 32],
    pub mode: RunMode,
    pub decimals: u8,
    pub domain: u64,
}
impl Binary for Definition {
    const SIZE: usize = KeyDefinition::SIZE + 32 + 1 + 1 + 8;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.key.encode(e)?;
        e.bytes(&self.dimensions)?;
        e.u8(match self.mode {
            RunMode::Exact => 0,
            RunMode::OrderedContributions => 1,
        })?;
        e.u8(self.decimals)?;
        e.u64(self.domain)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            key: KeyDefinition::decode(d)?,
            dimensions: d.array()?,
            mode: match d.u8()? {
                0 => RunMode::Exact,
                1 => RunMode::OrderedContributions,
                _ => return Err(invalid("unknown run mode")),
            },
            decimals: d.u8()?,
            domain: d.u64()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Meta {
    pub id: u64,
    pub value: Cell,
}
impl Binary for Meta {
    const SIZE: usize = 8 + Cell::SIZE;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.id)?;
        self.value.encode(e)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self {
            id: d.u64()?,
            value: Cell::decode(d)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeRef {
    pub page: PageId,
    pub level: u8,
    pub first: u64,
    pub segments: u64,
    pub hours: Span,
}
impl Binary for NodeRef {
    const SIZE: usize = 8 + 1 + 8 + 8 + 8;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.page)?;
        e.u8(self.level)?;
        e.u64(self.first)?;
        e.u64(self.segments)?;
        self.hours.encode(e)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let v = Self {
            page: d.u64()?,
            level: d.u8()?,
            first: d.u64()?,
            segments: d.u64()?,
            hours: Span::decode(d)?,
        };
        if v.page == NONE
            || v.segments == 0
            || v.level > 15
            || v.first.checked_add(v.segments).is_none()
        {
            return Err(invalid("series node reference"));
        }
        Ok(v)
    }
}

/// What a publication fixes besides its nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub publication: Publication,
    pub next_key: u64,
    pub next_event: u64,
    pub heap: u64,
    pub checkpoint: Blob,
}
impl Binary for State {
    const SIZE: usize = 8 + 8 + 8 + 4 + 1 + 8 + 8 + 8 + 8 + Blob::SIZE;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        let p = self.publication;
        e.u64(p.generation)?;
        e.u64(p.segment)?;
        p.accepted.encode(e)?;
        e.i32(p.reached)?;
        e.u8(u8::from(p.refusal.is_some()))?;
        e.u64(p.refusal.unwrap_or(0))?;
        e.u64(self.next_key)?;
        e.u64(self.next_event)?;
        e.u64(self.heap)?;
        self.checkpoint.encode(e)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let generation = d.u64()?;
        let segment = d.u64()?;
        let accepted = Span::decode(d)?;
        let reached = d.i32()?;
        let flag = d.flag()?;
        let refusal = d.u64()?;
        if reached < accepted.last {
            return Err(invalid("published coverage ahead of reached hour"));
        }
        Ok(Self {
            publication: Publication {
                generation,
                segment,
                accepted,
                reached,
                refusal: flag.then_some(refusal),
            },
            next_key: d.u64()?,
            next_event: d.u64()?,
            heap: d.u64()?,
            checkpoint: Blob::decode(d)?,
        })
    }
}

/// A hierarchy node: a sealed segment (level 0) or the merge of 16 nodes a level below.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    pub level: u8,
    pub first: u64,
    pub segments: u64,
    pub hours: Span,
    pub keys: (u64, u64),
    pub heap: (u64, u64),
    pub sequences: (u64, u64),
    pub children: Vec<NodeRef>,
    pub entries: Directory,
    pub domains: Directory,
    pub late: Table,
    pub metadata: Table,
    pub segment: Option<Segment>,
}
/// What only a sealed segment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub state: State,
    pub dictionary: Table,
    pub blocks: [Table; 4],
    pub data: List,
    pub heap: List,
}
impl Node {
    pub fn encode(&self, s: &mut Sink) -> io::Result<()> {
        s.u8(self.level)?;
        s.var(self.first)?;
        s.var(self.segments)?;
        s.record(&self.hours)?;
        for v in [
            self.keys.0,
            self.keys.1,
            self.heap.0,
            self.heap.1,
            self.sequences.0,
            self.sequences.1,
        ] {
            s.var(v)?;
        }
        s.var(self.children.len() as u64)?;
        for child in &self.children {
            s.record(child)?;
        }
        s.record(&self.entries)?;
        s.record(&self.domains)?;
        s.record(&self.late)?;
        s.record(&self.metadata)?;
        match &self.segment {
            None => s.u8(0),
            Some(segment) => {
                s.u8(1)?;
                s.record(&segment.state)?;
                s.record(&segment.dictionary)?;
                for table in &segment.blocks {
                    s.record(table)?;
                }
                s.record(&segment.data)?;
                s.record(&segment.heap)
            }
        }
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut d = Decode::new(bytes);
        let level = d.u8()?;
        let first = d.var()?;
        let segments = d.var()?;
        let hours = Span::decode(&mut d)?;
        let keys = (d.var()?, d.var()?);
        let heap = (d.var()?, d.var()?);
        let sequences = (d.var()?, d.var()?);
        let count = d.var()?;
        if count as usize > FANOUT
            || (level == 0) != (count == 0)
            || keys.0 > keys.1
            || heap.0 > heap.1
            || sequences.0 > sequences.1
        {
            return Err(invalid("series node shape"));
        }
        let mut children = engine_allocation::vec(count as usize)?;
        for _ in 0..count {
            children.push(NodeRef::decode(&mut d)?);
        }
        let entries = Directory::decode(&mut d)?;
        let domains = Directory::decode(&mut d)?;
        let late = Table::decode(&mut d)?;
        let metadata = Table::decode(&mut d)?;
        let segment = if d.flag()? {
            let state = State::decode(&mut d)?;
            let dictionary = Table::decode(&mut d)?;
            let mut blocks = [Table::EMPTY; 4];
            for table in &mut blocks {
                *table = Table::decode(&mut d)?;
            }
            Some(Segment {
                state,
                dictionary,
                blocks,
                data: List::decode(&mut d)?,
                heap: List::decode(&mut d)?,
            })
        } else {
            None
        };
        d.end()?;
        if (level == 0) != segment.is_some() {
            return Err(invalid("series node kind"));
        }
        Ok(Self {
            level,
            first,
            segments,
            hours,
            keys,
            heap,
            sequences,
            children,
            entries,
            domains,
            late,
            metadata,
            segment,
        })
    }
}

/// A family publication: its identity, state and the hierarchy's unmerged nodes in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Root {
    pub identity: Identity,
    pub state: State,
    pub nodes: Vec<NodeRef>,
}
impl Root {
    pub fn encode(&self, s: &mut Sink) -> io::Result<()> {
        s.put(&self.identity.source)?;
        s.put(&self.identity.content)?;
        s.put(&self.identity.branch)?;
        s.put(&self.identity.schema.to_le_bytes())?;
        s.record(&self.state)?;
        s.var(self.nodes.len() as u64)?;
        for node in &self.nodes {
            s.record(node)?;
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut d = Decode::new(bytes);
        let identity = Identity {
            source: d.array()?,
            content: d.array()?,
            branch: d.array()?,
            schema: d.u64()?,
        };
        let state = State::decode(&mut d)?;
        let count = d.var()?;
        let mut nodes = engine_allocation::vec(
            usize::try_from(count)
                .map_err(|_| invalid("series root"))?
                .min(1024),
        )?;
        for _ in 0..count {
            nodes.push(NodeRef::decode(&mut d)?);
        }
        d.end()?;
        Ok(Self {
            identity,
            state,
            nodes,
        })
    }
}
