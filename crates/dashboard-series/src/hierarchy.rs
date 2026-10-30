use crate::{
    add,
    cache::Cache,
    codec::{Decode, Sink},
    format::{Chain, Directory, Entry, Events, FANOUT, Late, Meta, Node, NodeRef, Root, Track},
    invalid,
    layout::{Entries, Records},
    pages::{self, Kind, Page, PageRead, PageWrite},
    read::Reader,
    summary::Summary,
    types::*,
};
use std::{io, sync::Arc};

pub(crate) fn write_root(io: &mut impl PageWrite, root: &Root) -> io::Result<PageId> {
    let mut bytes = Sink::default();
    root.encode(&mut bytes)?;
    pages::seal(io, Kind::Root, bytes.as_slice())
}

/// Merges every run of 16 consecutive nodes of one level into a node a level up.
pub(crate) fn settle(
    io: &mut impl PageWrite,
    cache: &Cache<'_>,
    nodes: &mut Vec<NodeRef>,
) -> io::Result<()> {
    loop {
        let Some(at) = nodes.len().checked_sub(FANOUT) else {
            return Ok(());
        };
        let level = nodes[at].level;
        if nodes[at..].iter().any(|node| node.level != level) {
            return Ok(());
        }
        let merged = merge(io, cache, &nodes[at..])?;
        nodes.truncate(at);
        nodes.push(merged);
    }
}

/// Entries of one directory, read page by page in key order.
struct Stream {
    dir: Directory,
    ordinal: u64,
    page: Option<Arc<Page>>,
    count: usize,
    at: usize,
    head: Option<u64>,
}
impl Stream {
    fn new(dir: Directory) -> Self {
        Self {
            dir,
            ordinal: 0,
            page: None,
            count: 0,
            at: 0,
            head: None,
        }
    }
    /// The next entry's key, decoded once until it is taken.
    fn peek(&mut self, reader: &mut Reader<'_>, io: &mut impl PageRead) -> io::Result<Option<u64>> {
        if self.head.is_some() {
            return Ok(self.head);
        }
        while self.page.is_none() || self.at == self.count {
            if self.ordinal == self.dir.list.pages {
                return Ok(None);
            }
            let page = reader.page(io, self.dir.list, self.ordinal, Kind::Records)?;
            self.count = crate::read::entry_count(page.bytes())?;
            self.page = Some(page);
            self.ordinal += 1;
            self.at = 0;
        }
        let page = self
            .page
            .as_ref()
            .ok_or_else(|| invalid("series directory stream"))?;
        self.head = Some(Decode::at(page.bytes(), 4 + self.at * 8).u64()?);
        Ok(self.head)
    }
    fn take(&mut self) -> io::Result<Entry> {
        let page = self
            .page
            .as_ref()
            .filter(|_| self.head.is_some())
            .ok_or_else(|| invalid("series directory stream"))?;
        let entry = crate::read::entry_in(page.bytes(), self.count, self.at)?;
        self.at += 1;
        self.head = None;
        Ok(entry)
    }
}
/// The smallest next key among `streams`, each stream's head decoded once.
fn least(
    streams: &mut [Stream],
    reader: &mut Reader<'_>,
    io: &mut impl PageRead,
) -> io::Result<Option<u64>> {
    let mut key = None;
    for stream in streams {
        if let Some(next) = stream.peek(reader, io)? {
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
struct LateStream {
    table: crate::format::Table,
    at: u64,
    head: Option<Late>,
}
impl LateStream {
    fn fill(&mut self, reader: &mut Reader<'_>, io: &mut impl PageRead) -> io::Result<()> {
        if self.head.is_none() && self.at < self.table.count {
            self.head = Some(reader.record::<Late>(io, self.table, self.at)?);
            self.at += 1;
        }
        Ok(())
    }
}
fn late_order(late: &Late) -> (u64, i32, u64) {
    (late.0.key(), late.native(), late.0.sequence())
}

fn merge(io: &mut impl PageWrite, cache: &Cache<'_>, children: &[NodeRef]) -> io::Result<NodeRef> {
    let mut reader = Reader::detached(cache);
    let mut nodes = engine_allocation::vec(children.len())?;
    for &reference in children {
        nodes.push(reader.child(io, reference)?);
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

    let mut late = Records::<Late>::new();
    let mut lates: Vec<LateStream> = engine_allocation::vec(nodes.len())?;
    for node in &nodes {
        lates.push(LateStream {
            table: node.late,
            at: 0,
            head: None,
        });
    }
    let mut entries = Entries::new();
    let mut streams: Vec<Stream> = engine_allocation::vec(nodes.len())?;
    for node in &nodes {
        streams.push(Stream::new(node.entries));
    }
    while let Some(key) = least(&mut streams, &mut reader, io)? {
        let mut parts = [None; FANOUT];
        for (at, stream) in streams.iter_mut().enumerate() {
            if stream.head == Some(key) {
                parts[at] = Some(stream.take()?);
            }
        }
        let mut entry = merge_entries(key, &parts[..nodes.len()])?;
        let mut stats = Events::default();
        let first = late.count();
        loop {
            let mut best: Option<(usize, (u64, i32, u64))> = None;
            for (at, stream) in lates.iter_mut().enumerate() {
                stream.fill(&mut reader, io)?;
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
        .any(|stream| stream.head.is_some() || stream.at < stream.table.count)
    {
        return Err(invalid("series late event without an entry"));
    }

    let mut domains = Entries::new();
    let mut streams: Vec<Stream> = engine_allocation::vec(nodes.len())?;
    for node in &nodes {
        streams.push(Stream::new(node.domains));
    }
    while let Some(key) = least(&mut streams, &mut reader, io)? {
        let mut parts = [None; FANOUT];
        for (at, stream) in streams.iter_mut().enumerate() {
            if stream.head == Some(key) {
                parts[at] = Some(stream.take()?);
            }
        }
        domains.push(io, &merge_entries(key, &parts[..nodes.len()])?)?;
    }

    let mut metadata = Records::<Meta>::new();
    let mut positions = [0_u64; FANOUT];
    let mut heads: [Option<Meta>; FANOUT] = [None; FANOUT];
    loop {
        let mut next: Option<(u64, usize)> = None;
        for (at, node) in nodes.iter().enumerate() {
            if heads[at].is_none() && positions[at] < node.metadata.count {
                heads[at] = Some(reader.record::<Meta>(io, node.metadata, positions[at])?);
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

    let node = Node {
        level,
        first: children[0].first,
        segments,
        hours,
        keys: (nodes[0].keys.0, nodes[nodes.len() - 1].keys.1),
        heap: (nodes[0].heap.0, nodes[nodes.len() - 1].heap.1),
        sequences: (nodes[0].sequences.0, nodes[nodes.len() - 1].sequences.1),
        children: engine_allocation::copy_vec(children)?,
        entries: entries.finish(io)?,
        domains: domains.finish(io)?,
        late: late.finish(io)?,
        metadata: metadata.finish(io)?,
        segment: None,
    };
    let mut bytes = Sink::default();
    node.encode(&mut bytes)?;
    let page = pages::seal(io, Kind::Node, bytes.as_slice())?;
    Ok(NodeRef {
        page,
        level,
        first: children[0].first,
        segments,
        hours,
    })
}
