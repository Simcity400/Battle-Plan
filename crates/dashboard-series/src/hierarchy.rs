use crate::{
    Grow, add,
    codec::{Binary, Decode, Sink},
    format::{Chain, Entry, Events, FANOUT, Held, Late, List, Meta, Node, NodeRef, Root, Track},
    invalid,
    layout::{Entries, Records, per_page},
    pages::{self, Kind, Page, PageRead, PageWrite},
    read::{Reader, entry_count, entry_in},
    summary::Summary,
    types::*,
};
use std::{io, sync::Arc};

pub(crate) fn write_root(io: &mut impl PageWrite, root: &Root) -> io::Result<PageId> {
    let mut bytes = Sink::default();
    root.encode(&mut bytes)?;
    pages::seal(io, Kind::Root, bytes.as_slice())
}

/// What merging a frontier node reads, kept by the writer that wrote it until it is merged: its
/// header, the pages of its directories, late and metadata tables, and its held runs.
pub(crate) struct Built {
    pub node: Arc<Node>,
    pub entries: Vec<Arc<Page>>,
    pub domains: Vec<Arc<Page>>,
    pub late: Vec<Arc<Page>>,
    pub metadata: Vec<Arc<Page>>,
    /// The run each held entry with a run continues, by key; None for a node a resumed writer
    /// reads back, whose held runs are read from the segments that began them.
    pub held: Option<Vec<HeldRun>>,
}

/// The value and scale of the run a held entry continues.
#[derive(Clone, Copy)]
pub(crate) struct HeldRun {
    pub key: u64,
    pub value: i128,
    pub decimals: u8,
}

impl Built {
    /// A node of a resumed hierarchy, which no writer kept, read back.
    fn load(io: &mut impl PageRead, reader: &mut Reader, reference: NodeRef) -> io::Result<Self> {
        let node = reader.child(io, reference)?;
        Ok(Self {
            entries: listed(io, reader, node.entries.list)?,
            domains: listed(io, reader, node.domains.list)?,
            late: listed(io, reader, node.late.list)?,
            metadata: listed(io, reader, node.metadata.list)?,
            node,
            held: None,
        })
    }
}

fn listed(io: &mut impl PageRead, reader: &mut Reader, list: List) -> io::Result<Vec<Arc<Page>>> {
    let mut pages = Vec::new();
    for ordinal in 0..list.pages {
        pages.try_push(reader.page(io, list, ordinal, Kind::Records)?)?;
    }
    Ok(pages)
}

fn record<T: Binary>(pages: &[Arc<Page>], index: u64) -> io::Result<T> {
    let per = per_page::<T>() as u64;
    let page = usize::try_from(index / per)
        .ok()
        .and_then(|at| pages.get(at))
        .ok_or_else(|| invalid("series record outside its table"))?;
    let at = (index % per) as usize * T::SIZE;
    let bytes = page
        .bytes()
        .get(at..at + T::SIZE)
        .ok_or_else(|| invalid("series record outside its page"))?;
    T::decode(&mut Decode::new(bytes))
}

/// The children one merge replaced, and what was kept of them, to put back.
pub(crate) type Replaced = (Vec<NodeRef>, Vec<Option<Built>>);

/// Merges every run of 16 consecutive nodes of one level into a node a level up, noting in
/// `undo` the children each merge replaced.
pub(crate) fn settle(
    io: &mut impl PageWrite,
    nodes: &mut Vec<NodeRef>,
    built: &mut Vec<Option<Built>>,
    undo: &mut Vec<Replaced>,
) -> io::Result<()> {
    // Read only for nodes of a resumed hierarchy; merging adds no hours, so it reads the merged
    // ones too.
    let mut reader = None;
    loop {
        let Some(at) = nodes.len().checked_sub(FANOUT) else {
            return Ok(());
        };
        let level = nodes[at].level;
        if nodes[at..].iter().any(|node| node.level != level) {
            return Ok(());
        }
        for index in at..nodes.len() {
            if built[index].is_none() {
                if reader.is_none() {
                    reader = Some(Reader::frontier(nodes)?);
                }
                let reader = reader
                    .as_mut()
                    .ok_or_else(|| invalid("series frontier reader"))?;
                built[index] = Some(Built::load(io, reader, nodes[index])?);
            }
        }
        let (node, merged) = merge(io, &mut reader, &nodes[at..], &built[at..])?;
        engine_allocation::reserve(undo, 1)?;
        let mut children = engine_allocation::vec(FANOUT)?;
        let mut kept = engine_allocation::vec(FANOUT)?;
        children.extend(nodes.drain(at..));
        kept.extend(built.drain(at..));
        undo.push((children, kept));
        nodes.push(node);
        built.push(Some(merged));
    }
}

/// Entries of one directory, read page by page in key order.
struct Stream<'b> {
    pages: &'b [Arc<Page>],
    ordinal: usize,
    count: usize,
    at: usize,
    head: Option<u64>,
}
impl<'b> Stream<'b> {
    fn new(pages: &'b [Arc<Page>]) -> Self {
        Self {
            pages,
            ordinal: 0,
            count: 0,
            at: 0,
            head: None,
        }
    }
    fn page(&self) -> io::Result<&'b Page> {
        self.ordinal
            .checked_sub(1)
            .and_then(|at| self.pages.get(at))
            .map(|page| &**page)
            .ok_or_else(|| invalid("series directory stream"))
    }
    /// The next entry's key, decoded once until it is taken.
    fn peek(&mut self) -> io::Result<Option<u64>> {
        if self.head.is_some() {
            return Ok(self.head);
        }
        while self.ordinal == 0 || self.at == self.count {
            let Some(page) = self.pages.get(self.ordinal) else {
                return Ok(None);
            };
            self.count = entry_count(page.bytes())?;
            self.ordinal += 1;
            self.at = 0;
        }
        self.head = Some(Decode::at(self.page()?.bytes(), 4 + self.at * 8).u64()?);
        Ok(self.head)
    }
    fn take(&mut self) -> io::Result<Entry> {
        if self.head.is_none() {
            return Err(invalid("series directory stream"));
        }
        let entry = entry_in(self.page()?.bytes(), self.count, self.at)?;
        self.at += 1;
        self.head = None;
        Ok(entry)
    }
}
/// The smallest next key among `streams`, each stream's head decoded once.
fn least(streams: &mut [Stream<'_>]) -> io::Result<Option<u64>> {
    let mut key = None;
    for stream in streams {
        if let Some(next) = stream.peek()? {
            key = Some(key.map_or(next, |k: u64| k.min(next)));
        }
    }
    Ok(key)
}

fn merge_track(parts: &[Option<Track>]) -> io::Result<Track> {
    let mut out = Track::default();
    for track in parts.iter().flatten().filter(|t| t.count != 0) {
        if out.count == 0 {
            out.first = track.first;
            out.last = track.last;
        }
        out.count = add(out.count, track.count)?;
        out.first = out.first.min(track.first);
        out.last = out.last.max(track.last);
    }
    let head = match parts.first().copied().flatten() {
        Some(first) if first.count != 0 && first.head.continues => {
            let mut end = first.head.end;
            let mut through = first.head.through;
            let mut at = 1;
            while through && at < parts.len() {
                match parts[at] {
                    Some(t) if t.count != 0 && t.head.continues => {
                        end = t.head.end;
                        through = t.head.through;
                        at += 1;
                    }
                    _ => through = false,
                }
            }
            Chain {
                continues: true,
                end,
                through,
            }
        }
        _ => Chain::default(),
    };
    Ok(Track { head, ..out })
}

/// The run `child` holds of `key`: from what its writer kept, else from the segment that began it.
fn held_run(
    io: &mut impl PageRead,
    reader: &mut Option<Reader>,
    child: &Built,
    cursor: &mut usize,
    key: u64,
    held: Held,
) -> io::Result<HeldRun> {
    if let Some(runs) = &child.held {
        while runs.get(*cursor).is_some_and(|run| run.key < key) {
            *cursor += 1;
        }
        return runs
            .get(*cursor)
            .filter(|run| run.key == key)
            .copied()
            .ok_or_else(|| invalid("series held run without its run"));
    }
    let reader = reader
        .as_mut()
        .ok_or_else(|| invalid("series frontier reader"))?;
    let only = Held {
        evidence: false,
        coverage: false,
        ..held
    };
    let run = reader
        .continued(io, key, only)?
        .0
        .ok_or_else(|| invalid("series held run without its run"))?;
    let decimals = reader
        .definition(io, key)?
        .ok_or_else(|| invalid("series held run of an undefined key"))?
        .decimals;
    Ok(HeldRun {
        key,
        value: run.value,
        decimals,
    })
}

/// One key's entry over the children: held where every child holds the same rows, else merged
/// from the rows each holds or continues; with the run it holds, where it holds one.
fn merge_key(
    io: &mut impl PageRead,
    reader: &mut Option<Reader>,
    children: &[&Built],
    cursors: &mut [usize],
    key: u64,
    parts: &mut [Option<Entry>],
) -> io::Result<(Entry, Option<HeldRun>)> {
    let first = parts.first().copied().flatten();
    if let Some((first, held)) = first.and_then(|entry| Some((entry, entry.held?)))
        && parts
            .iter()
            .all(|part| part.is_some_and(|entry| entry.held == Some(held)))
    {
        let mut rows = first.row_start;
        for (at, part) in parts.iter().enumerate() {
            let entry = part.ok_or_else(|| invalid("series held merge"))?;
            if entry.row_start != rows || entry.canonical_start != first.canonical_start {
                return Err(invalid("series run counters do not continue across nodes"));
            }
            if held.runs {
                rows = add(rows, children[at].node.segments)?;
            }
        }
        let run = if held.runs {
            Some(held_run(io, reader, children[0], &mut cursors[0], key, held)?)
        } else {
            None
        };
        return Ok((
            Entry::holding(key, held, first.row_start, first.canonical_start),
            run,
        ));
    }
    let mut resolved: Option<(i32, HeldRun)> = None;
    for (at, part) in parts.iter_mut().enumerate() {
        let Some((entry, held)) = part.and_then(|entry| Some((entry, entry.held?))) else {
            continue;
        };
        let node = &children[at].node;
        let mut shaped = entry.shaped(node.hours, node.segments);
        if held.runs {
            let run = match resolved.filter(|(origin, _)| *origin == held.origin) {
                Some((_, run)) => run,
                None => held_run(io, reader, children[at], &mut cursors[at], key, held)?,
            };
            resolved = Some((held.origin, run));
            shaped.summary = Some(Summary::held(run.value, node.hours, run.decimals));
        }
        *part = Some(shaped);
    }
    Ok((merge_entries(key, parts)?, None))
}

fn merge_entries(key: u64, parts: &[Option<Entry>]) -> io::Result<Entry> {
    let mut out = Entry::empty(key);
    let mut runs = [None; FANOUT];
    let mut evidence = [None; FANOUT];
    let mut coverage = [None; FANOUT];
    let mut started = false;
    let mut rows = 0;
    let mut summary: Option<Summary> = None;
    for (at, part) in parts.iter().enumerate() {
        let Some(entry) = part else {
            continue;
        };
        runs[at] = Some(entry.runs);
        evidence[at] = Some(entry.evidence);
        coverage[at] = Some(entry.coverage);
        if !started {
            out.row_start = entry.row_start;
            out.canonical_start = entry.canonical_start;
            started = true;
        } else if entry.canonical_start != out.canonical_end || entry.row_start != rows {
            return Err(invalid("series run counters do not continue across nodes"));
        }
        out.canonical_end = entry.canonical_end;
        if entry.last_row.is_some() {
            out.last_row = entry.last_row;
        }
        rows = add(entry.row_start, entry.runs.count)?;
        if let Some(part) = entry.summary {
            summary = Some(match summary {
                Some(previous) => previous.merge(part)?,
                None => part,
            });
        }
        out.events = out.events.merge(entry.events)?;
    }
    out.runs = merge_track(&runs[..parts.len()])?;
    out.evidence = merge_track(&evidence[..parts.len()])?;
    out.coverage = merge_track(&coverage[..parts.len()])?;
    out.summary = summary;
    Ok(out)
}

/// The next late event by (key, native, sequence) among the children's late tables.
struct LateStream<'b> {
    pages: &'b [Arc<Page>],
    count: u64,
    at: u64,
    head: Option<Late>,
}
impl LateStream<'_> {
    fn fill(&mut self) -> io::Result<()> {
        if self.head.is_none() && self.at < self.count {
            self.head = Some(record::<Late>(self.pages, self.at)?);
            self.at += 1;
        }
        Ok(())
    }
}
fn late_order(late: &Late) -> (u64, i32, u64) {
    (late.0.key(), late.native(), late.0.sequence())
}

/// The node merging `children`, read from what was kept of them, and what to keep of it.
fn merge(
    io: &mut impl PageWrite,
    reader: &mut Option<Reader>,
    children: &[NodeRef],
    built: &[Option<Built>],
) -> io::Result<(NodeRef, Built)> {
    let mut nodes: Vec<&Built> = engine_allocation::vec(children.len())?;
    for (reference, child) in children.iter().zip(built) {
        let child = child
            .as_ref()
            .ok_or_else(|| invalid("series frontier node unread"))?;
        let node = &child.node;
        if node.level != reference.level
            || node.first != reference.first
            || node.segments != reference.segments
            || node.hours != reference.hours
        {
            return Err(invalid("series node does not match its reference"));
        }
        nodes.push(child);
    }
    let level = children[0]
        .level
        .checked_add(1)
        .ok_or_else(|| invalid("series hierarchy height"))?;
    let segments = children.iter().try_fold(0, |n, c| add(n, c.segments))?;
    let hours = Span::new(
        children[0].hours.first,
        children[children.len() - 1].hours.last,
    )?;

    let mut late = Records::<Late>::keeping();
    let mut lates: Vec<LateStream<'_>> = engine_allocation::vec(nodes.len())?;
    for child in &nodes {
        lates.push(LateStream {
            pages: &child.late,
            count: child.node.late.count,
            at: 0,
            head: None,
        });
    }
    let mut entries = Entries::keeping();
    let mut held = Vec::new();
    let mut cursors = [0_usize; FANOUT];
    let mut streams: Vec<Stream<'_>> = engine_allocation::vec(nodes.len())?;
    for child in &nodes {
        streams.push(Stream::new(&child.entries));
    }
    while let Some(key) = least(&mut streams)? {
        let mut parts = [None; FANOUT];
        for (at, stream) in streams.iter_mut().enumerate() {
            if stream.head == Some(key) {
                parts[at] = Some(stream.take()?);
            }
        }
        let (mut entry, run) = merge_key(
            io,
            reader,
            &nodes,
            &mut cursors[..nodes.len()],
            key,
            &mut parts[..nodes.len()],
        )?;
        if let Some(run) = run {
            held.try_push(run)?;
        }
        let mut stats = Events::default();
        let first = late.count();
        loop {
            let mut best: Option<(usize, (u64, i32, u64))> = None;
            for (at, stream) in lates.iter_mut().enumerate() {
                stream.fill()?;
                if let Some(head) = &stream.head {
                    let order = late_order(head);
                    if order.0 == key && best.is_none_or(|(_, b)| order < b) {
                        best = Some((at, order));
                    }
                }
            }
            let Some((at, _)) = best else {
                break;
            };
            let event = lates[at]
                .head
                .take()
                .ok_or_else(|| invalid("series late stream"))?;
            stats.add(event.native(), event.read())?;
            late.push(io, &event)?;
        }
        if stats.count != 0 {
            entry.late = stats;
            entry.late_at = first;
        }
        entries.push(io, &entry)?;
    }
    if lates
        .iter_mut()
        .any(|stream| stream.head.is_some() || stream.at < stream.count)
    {
        return Err(invalid("series late event without an entry"));
    }

    let mut domains = Entries::keeping();
    let mut streams: Vec<Stream<'_>> = engine_allocation::vec(nodes.len())?;
    for child in &nodes {
        streams.push(Stream::new(&child.domains));
    }
    while let Some(key) = least(&mut streams)? {
        let mut parts = [None; FANOUT];
        for (at, stream) in streams.iter_mut().enumerate() {
            if stream.head == Some(key) {
                parts[at] = Some(stream.take()?);
            }
        }
        if parts.iter().flatten().any(|entry| entry.held.is_some()) {
            return Err(invalid("series shared evidence domain held"));
        }
        domains.push(io, &merge_entries(key, &parts[..nodes.len()])?)?;
    }

    let mut metadata = Records::<Meta>::keeping();
    let mut positions = [0_u64; FANOUT];
    let mut heads: [Option<Meta>; FANOUT] = [None; FANOUT];
    loop {
        let mut next: Option<(u64, usize)> = None;
        for (at, child) in nodes.iter().enumerate() {
            if heads[at].is_none() && positions[at] < child.node.metadata.count {
                heads[at] = Some(record::<Meta>(&child.metadata, positions[at])?);
            }
            if let Some(meta) = heads[at] {
                if next.is_none_or(|(id, _)| meta.id <= id) {
                    next = Some((meta.id, at));
                }
            }
        }
        let Some((id, latest)) = next else {
            break;
        };
        let value = heads[latest].ok_or_else(|| invalid("series metadata merge"))?;
        for at in 0..nodes.len() {
            if heads[at].is_some_and(|meta| meta.id == id) {
                heads[at] = None;
                positions[at] += 1;
            }
        }
        metadata.push(io, &value)?;
    }

    let (entries, entry_pages) = entries.finish_kept(io)?;
    let (domains, domain_pages) = domains.finish_kept(io)?;
    let (late, late_pages) = late.finish_kept(io)?;
    let (metadata, metadata_pages) = metadata.finish_kept(io)?;
    let (first, last) = (&nodes[0].node, &nodes[nodes.len() - 1].node);
    let node = Node {
        level,
        first: children[0].first,
        segments,
        hours,
        keys: (first.keys.0, last.keys.1),
        heap: (first.heap.0, last.heap.1),
        sequences: (first.sequences.0, last.sequences.1),
        children: engine_allocation::copy_vec(children)?,
        entries,
        domains,
        late,
        metadata,
        segment: None,
    };
    let mut bytes = Sink::default();
    node.encode(&mut bytes)?;
    let page = pages::seal(io, Kind::Node, bytes.as_slice())?;
    Ok((
        NodeRef {
            page,
            level,
            first: children[0].first,
            segments,
            hours,
        },
        Built {
            node: Arc::new(node),
            entries: entry_pages,
            domains: domain_pages,
            late: late_pages,
            metadata: metadata_pages,
            held: Some(held),
        },
    ))
}
