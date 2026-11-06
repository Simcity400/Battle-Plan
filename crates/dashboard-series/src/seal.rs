use crate::{
    Grow,
    codec::Sink,
    format::{
        BLOCK, Block, COVERAGE, Chain, Definition, Detail, EVENTS, EVIDENCE, Entry, EventBlock,
        Events, LastRow, Late, Meta, Node, NodeRef, Origin, RUNS, RunBlock, Segment, State, Track,
    },
    hierarchy::{Built, HeldRun},
    invalid,
    layout::{Data, Entries, Records, write_heap},
    pages::{self, Kind, PageWrite},
    query::EventView,
    summary::Summary,
    types::*,
    writer::{Columns, Edge, EvidenceRow, HeldRow, RunRow, same_evidence},
};
use std::{io, sync::Arc};

/// A cut segment waiting to be sealed. Sealing reads nothing but this segment,
/// so it runs on any thread and can be retried after a failed write.
pub struct Pending {
    pub(crate) index: u64,
    pub(crate) hours: Span,
    pub(crate) state: State,
    pub(crate) first_key: u64,
    pub(crate) heap_base: u64,
    pub(crate) sequences: (u64, u64),
    pub(crate) columns: Columns,
}
/// A sealed segment, ready for [`crate::Writer::publish`], with what merging it reads.
pub struct Sealed {
    pub(crate) node: NodeRef,
    pub(crate) state: State,
    pub(crate) built: Built,
}

/// What one seal wrote, for bounds that do not depend on timing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub pages: u64,
    pub bytes: u64,
}

struct Writers {
    data: Data,
    runs: Records<RunBlock>,
    evidence: Records<Block>,
    coverage: Records<Block>,
    events: Records<EventBlock>,
    late: Records<Late>,
    block: Sink,
}

/// Stable grouping of row indices by their subject's position in `order`.
fn group(
    order: &[u64],
    subjects: impl Iterator<Item = u64> + Clone,
) -> io::Result<(Vec<u32>, Vec<u32>)> {
    let mut starts = engine_allocation::filled(order.len() + 1, 0_u32)?;
    let slot = |subject: u64| -> io::Result<usize> {
        order
            .binary_search(&subject)
            .map_err(|_| invalid("series row of an untouched subject"))
    };
    let mut rows = 0_usize;
    for subject in subjects.clone() {
        starts[slot(subject)? + 1] += 1;
        rows += 1;
    }
    for at in 1..starts.len() {
        starts[at] += starts[at - 1];
    }
    let mut next = engine_allocation::copy_vec(&starts[..order.len()])?;
    let mut grouped = engine_allocation::filled(rows, 0_u32)?;
    for (row, subject) in subjects.enumerate() {
        let at = slot(subject)?;
        grouped[next[at] as usize] =
            u32::try_from(row).map_err(|_| invalid("series segment rows"))?;
        next[at] += 1;
    }
    Ok((starts, grouped))
}

/// A head chain; `carried` says the head is still the row carried out of the segment.
fn chain(continues: bool, first_end: i32, carried: bool, hours: Span) -> Chain {
    Chain {
        continues,
        end: first_end,
        through: continues && carried && first_end == hours.last,
    }
}

impl Pending {
    /// The publication it was cut as.
    pub fn publication(&self) -> Publication {
        self.state.publication
    }

    pub fn seal(&self, io: &mut impl PageWrite) -> io::Result<Sealed> {
        self.seal_counted(io).map(|(sealed, _)| sealed)
    }
    pub fn seal_counted(&self, io: &mut impl PageWrite) -> io::Result<(Sealed, Work)> {
        let mut counted = Counted {
            io,
            work: Work::default(),
        };
        let sealed = self.seal_into(&mut counted)?;
        Ok((sealed, counted.work))
    }
    fn seal_into(&self, io: &mut impl PageWrite) -> io::Result<Sealed> {
        let c = &self.columns;
        let mut sequences = engine_allocation::vec(c.events.len())?;
        for event in c.events.iter() {
            sequences.try_push(event.sequence())?;
        }
        sequences.sort_unstable();
        if sequences.windows(2).any(|w| w[0] == w[1])
            || sequences.first().is_some_and(|s| *s < self.sequences.0)
            || sequences.last().is_some_and(|s| *s >= self.sequences.1)
        {
            return Err(invalid("unreserved/duplicate publication sequence"));
        }
        drop(sequences);
        let mut keys = engine_allocation::vec(c.keys.len())?;
        for touch in c.keys.iter() {
            keys.try_push(*touch)?;
        }
        keys.sort_unstable_by_key(|touch| touch.id);
        let mut key_ids = engine_allocation::vec(keys.len())?;
        for touch in keys.iter() {
            key_ids.try_push(touch.id)?;
        }
        let mut domains = engine_allocation::vec(c.domains.len())?;
        for touch in c.domains.iter() {
            domains.try_push(*touch)?;
        }
        domains.sort_unstable_by_key(|touch| touch.id);
        let mut domain_ids = engine_allocation::vec(domains.len())?;
        for touch in domains.iter() {
            domain_ids.try_push(touch.id)?;
        }
        let (run_starts, run_rows) = group(&key_ids, c.runs.iter().map(|r| r.key))?;
        let (evidence_starts, evidence_rows) =
            group(&key_ids, c.evidence.iter().map(|r| r.subject))?;
        let (event_starts, event_rows) = group(&key_ids, c.events.iter().map(|e| e.key()))?;
        let (domain_starts, domain_rows) =
            group(&domain_ids, c.domain_rows.iter().map(|r| r.subject))?;

        let mut w = Writers {
            data: Data::new(),
            runs: Records::new(),
            evidence: Records::new(),
            coverage: Records::new(),
            events: Records::new(),
            late: Records::keeping(),
            block: Sink::default(),
        };
        let mut merged = Vec::new();
        let mut spans = Vec::new();
        let mut domain_entries = Entries::keeping();
        let mut domain_spans: Vec<(u64, Vec<Span>)> = engine_allocation::vec(domains.len())?;
        for (slot, touch) in domains.iter().enumerate() {
            let rows = &domain_rows[domain_starts[slot] as usize..domain_starts[slot + 1] as usize];
            let mut entry = Entry::empty(touch.id);
            let mut detail = Detail {
                blocks: [(0, 0); 4],
                origin: Origin::default(),
                evidence_origin: 0,
                tail: None,
                evidence_tail: None,
            };
            self.evidence(
                io,
                &mut w,
                rows.iter().map(|&i| c.domain_rows[i as usize]),
                touch.start.evidence,
                touch.start.known,
                touch.end.evidence,
                &mut merged,
                &mut spans,
                &mut entry,
                &mut detail,
            )?;
            entry.detail = Some(detail);
            entry.last_row = rows.last().map(|&row| LastRow {
                segment: self.index,
                row,
                value: c.domain_rows[row as usize].value,
            });
            domain_entries.push(io, &entry)?;
            domain_spans.try_push((touch.id, engine_allocation::copy_vec(&spans)?))?;
        }

        let mut held = engine_allocation::vec(c.held.len())?;
        for row in c.held.iter() {
            held.try_push(*row)?;
        }
        held.sort_unstable_by_key(|row| row.key);
        let mut held_runs = engine_allocation::vec(held.len())?;
        let mut holding = held.iter().peekable();
        let mut entries = Entries::keeping();
        for (slot, touch) in keys.iter().enumerate() {
            while let Some(row) = holding.next_if(|row| row.key <= touch.id) {
                if row.key == touch.id {
                    return Err(invalid("series key both held and holding rows"));
                }
                self.held(io, row, &domain_spans, &mut entries, &mut held_runs)?;
            }
            let start: Edge = touch.start;
            let end: Edge = touch.end;
            let mut entry = Entry::empty(touch.id);
            let mut detail = Detail {
                blocks: [(0, 0); 4],
                origin: Origin::default(),
                evidence_origin: 0,
                tail: None,
                evidence_tail: None,
            };
            let evidence =
                &evidence_rows[evidence_starts[slot] as usize..evidence_starts[slot + 1] as usize];
            // A key defined in this segment may be bound after it was first touched.
            let domain = end.domain;
            if start.domain != NONE && start.domain != domain {
                return Err(invalid("a key cannot reinterpret its evidence domain"));
            }
            if domain == NONE {
                self.evidence(
                    io,
                    &mut w,
                    evidence.iter().map(|&i| c.evidence[i as usize]),
                    start.evidence,
                    start.known,
                    end.evidence,
                    &mut merged,
                    &mut spans,
                    &mut entry,
                    &mut detail,
                )?;
            } else if !evidence.is_empty() {
                return Err(invalid("domain-bound key has independent evidence"));
            }
            let known: &[Span] = if domain == NONE {
                &spans
            } else {
                domain_spans
                    .binary_search_by_key(&domain, |(id, _)| *id)
                    .map_or(&[][..], |at| &domain_spans[at].1[..])
            };
            let rows = &run_rows[run_starts[slot] as usize..run_starts[slot + 1] as usize];
            self.runs(io, &mut w, rows, start, end, known, &mut entry, &mut detail)?;
            let events = &event_rows[event_starts[slot] as usize..event_starts[slot + 1] as usize];
            self.events(io, &mut w, events, &mut entry, &mut detail)?;
            entry.detail = Some(detail);
            entries.push(io, &entry)?;
        }
        for row in holding {
            self.held(io, row, &domain_spans, &mut entries, &mut held_runs)?;
        }

        let mut dictionary = Records::<Definition>::new();
        for (at, definition) in c.definitions.iter().enumerate() {
            if definition.key.id != self.first_key + at as u64 {
                return Err(invalid("series key definitions are not dense"));
            }
            dictionary.push(io, definition)?;
        }
        let mut metadata = engine_allocation::vec(c.metadata.len())?;
        for (at, meta) in c.metadata.iter().enumerate() {
            metadata.try_push((meta.id, at))?;
        }
        metadata.sort_unstable();
        let mut written = Records::<Meta>::keeping();
        for (at, &(id, index)) in metadata.iter().enumerate() {
            if metadata.get(at + 1).is_none_or(|next| next.0 != id) {
                written.push(io, &c.metadata[index])?;
            }
        }
        let blocks = [
            w.runs.finish(io)?,
            w.evidence.finish(io)?,
            w.coverage.finish(io)?,
            w.events.finish(io)?,
        ];
        let (entries, entry_pages) = entries.finish_kept(io)?;
        let (domains, domain_pages) = domain_entries.finish_kept(io)?;
        let (late, late_pages) = w.late.finish_kept(io)?;
        let (metadata, metadata_pages) = written.finish_kept(io)?;
        let node = Node {
            level: 0,
            first: self.index,
            segments: 1,
            hours: self.hours,
            keys: (self.first_key, self.state.next_key),
            heap: (self.heap_base, self.state.heap),
            sequences: self.sequences,
            children: Vec::new(),
            entries,
            domains,
            late,
            metadata,
            segment: Some(Segment {
                state: self.state,
                dictionary: dictionary.finish(io)?,
                blocks,
                data: w.data.finish(io)?,
                heap: write_heap(io, c.heap.whole())?,
            }),
        };
        let mut header = Sink::default();
        node.encode(&mut header)?;
        let page = pages::seal(io, Kind::Node, header.as_slice())?;
        Ok(Sealed {
            node: NodeRef {
                page,
                level: 0,
                first: self.index,
                segments: 1,
                hours: self.hours,
            },
            state: self.state,
            built: Built {
                node: Arc::new(node),
                entries: entry_pages,
                domains: domain_pages,
                late: late_pages,
                metadata: metadata_pages,
                held: Some(held_runs),
            },
        })
    }

    /// The entry of a key the segment only continues: a held run of a key bound to a shared
    /// domain must lie within the domain's known hours.
    fn held(
        &self,
        io: &mut impl PageWrite,
        row: &HeldRow,
        domains: &[(u64, Vec<Span>)],
        entries: &mut Entries,
        runs: &mut Vec<HeldRun>,
    ) -> io::Result<()> {
        if row.held.runs && row.domain != NONE {
            let known = domains
                .binary_search_by_key(&row.domain, |(id, _)| *id)
                .map_or(&[][..], |at| &domains[at].1[..]);
            let at = known.partition_point(|k| k.first <= self.hours.first);
            if at == 0 || known[at - 1].last < self.hours.last {
                return Err(invalid("series run outside known coverage"));
            }
        }
        if row.held.runs {
            runs.try_push(HeldRun {
                key: row.key,
                value: row.value,
                decimals: row.decimals,
            })?;
        }
        entries.push(
            io,
            &Entry::holding(row.key, row.held, row.row_start, row.canonicals),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evidence(
        &self,
        io: &mut impl PageWrite,
        w: &mut Writers,
        rows: impl Iterator<Item = EvidenceRow>,
        before: Option<Evidence>,
        known_before: Option<i32>,
        after: Option<Evidence>,
        merged: &mut Vec<Evidence>,
        spans: &mut Vec<Span>,
        entry: &mut Entry,
        detail: &mut Detail,
    ) -> io::Result<()> {
        merged.clear();
        spans.clear();
        for row in rows {
            let value = row.value;
            match merged.last_mut() {
                Some(last)
                    if same_evidence(*last, value)
                        && i64::from(last.span.last) + 1 == i64::from(value.span.first) =>
                {
                    last.span.last = value.span.last;
                }
                _ => merged.try_push(value)?,
            }
            if value.known == Knownness::Known {
                match spans.last_mut() {
                    Some(last) if i64::from(last.last) + 1 == i64::from(value.span.first) => {
                        last.last = value.span.last;
                    }
                    _ => spans.try_push(value.span)?,
                }
            }
        }
        let hours = self.hours;
        if let Some(first) = merged.first() {
            let continues = before.is_some_and(|p| {
                same_evidence(p, *first)
                    && i64::from(p.span.last) + 1 == i64::from(first.span.first)
            });
            if continues {
                detail.evidence_origin = before.map_or(first.span.first, |p| p.span.first);
            }
            entry.evidence = Track {
                count: merged.len() as u64,
                first: first.span.first,
                last: merged[merged.len() - 1].span.last,
                head: chain(continues, first.span.last, merged.len() == 1, hours),
            };
            let first_block = w.evidence.count();
            for (at, chunk) in merged.chunks(BLOCK).enumerate() {
                w.block.clear();
                let mut previous = i64::from(chunk[0].span.first);
                for (i, row) in chunk.iter().enumerate() {
                    w.block.zig(i64::from(row.span.first) - previous)?;
                    w.block
                        .var((i64::from(row.span.last) - i64::from(row.span.first)) as u64)?;
                    put_evidence_flags(&mut w.block, row, continues && at == 0 && i == 0)?;
                    if let Some(read) = row.supplying_read {
                        w.block.zig(i64::from(read) - i64::from(row.span.first))?;
                    }
                    previous = i64::from(row.span.last) + 1;
                }
                let (page, offset) = w.data.block(io, w.block.as_slice())?;
                w.evidence.push(
                    io,
                    &Block {
                        page,
                        offset,
                        rows: chunk.len() as u32,
                        first: chunk[0].span.first,
                        last: chunk[chunk.len() - 1].span.last,
                    },
                )?;
            }
            detail.blocks[EVIDENCE] = (first_block, w.evidence.count() - first_block);
        }
        detail.evidence_tail = after.filter(|e| e.span.last == hours.last && !merged.is_empty());
        if let Some(first) = spans.first() {
            let continues = first.first == hours.first
                && known_before.is_some_and(|k| i64::from(k) + 1 == i64::from(hours.first));
            entry.coverage = Track {
                count: spans.len() as u64,
                first: first.first,
                last: spans[spans.len() - 1].last,
                head: chain(continues, first.last, spans.len() == 1, hours),
            };
            let first_block = w.coverage.count();
            for chunk in spans.chunks(BLOCK) {
                w.block.clear();
                let mut previous = i64::from(chunk[0].first);
                for span in chunk {
                    w.block.zig(i64::from(span.first) - previous)?;
                    w.block
                        .var((i64::from(span.last) - i64::from(span.first)) as u64)?;
                    previous = i64::from(span.last) + 1;
                }
                let (page, offset) = w.data.block(io, w.block.as_slice())?;
                w.coverage.push(
                    io,
                    &Block {
                        page,
                        offset,
                        rows: chunk.len() as u32,
                        first: chunk[0].first,
                        last: chunk[chunk.len() - 1].last,
                    },
                )?;
            }
            detail.blocks[COVERAGE] = (first_block, w.coverage.count() - first_block);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn runs(
        &self,
        io: &mut impl PageWrite,
        w: &mut Writers,
        rows: &[u32],
        start: Edge,
        end: Edge,
        known: &[Span],
        entry: &mut Entry,
        detail: &mut Detail,
    ) -> io::Result<()> {
        let c = &self.columns;
        entry.row_start = start.rows;
        entry.canonical_start = start.canonicals;
        entry.canonical_end = end.canonicals;
        if end.rows.checked_sub(start.rows) != Some(rows.len() as u64) {
            return Err(invalid("series run counters"));
        }
        let Some(&head) = rows.first() else {
            return Ok(());
        };
        let head: RunRow = c.runs[head as usize];
        for &row in rows {
            let span = c.runs[row as usize].span;
            let at = known.partition_point(|k| k.first <= span.first);
            if at == 0 || known[at - 1].last < span.last {
                return Err(invalid("series run outside known coverage"));
            }
        }
        if head.continues {
            let origin = start
                .last
                .ok_or_else(|| invalid("series continuation without a carried run"))?;
            detail.origin = Origin {
                first: origin.span.first,
                break_before: origin.break_before,
                supplying_boundary: origin.supplying_boundary,
                sequence: origin.sequence,
            };
        }
        detail.tail = end.last.filter(|run| run.span.last == self.hours.last);
        let first_block = w.runs.count();
        let mut summary: Option<Summary> = None;
        let (mut first, mut last) = (i32::MAX, i32::MIN);
        for chunk in rows.chunks(BLOCK) {
            w.block.clear();
            let low = chunk
                .iter()
                .map(|&i| c.runs[i as usize].span.first)
                .min()
                .ok_or_else(|| invalid("empty series block"))?;
            let high = chunk
                .iter()
                .map(|&i| c.runs[i as usize].span.last)
                .max()
                .ok_or_else(|| invalid("empty series block"))?;
            let mut previous = i64::from(low);
            let mut value = 0_i128;
            let mut block: Option<Summary> = None;
            for &i in chunk {
                let row = c.runs[i as usize];
                w.block.zig(i64::from(row.span.first) - previous)?;
                w.block
                    .var((i64::from(row.span.last) - i64::from(row.span.first)) as u64)?;
                w.block.u8(u8::from(row.break_before)
                    | u8::from(row.supplying_boundary) << 1
                    | u8::from(row.continues) << 2)?;
                w.block.zig128(row.value.wrapping_sub(value))?;
                previous = i64::from(row.span.last) + 1;
                value = row.value;
                let piece = Summary::run(
                    Run {
                        span: row.span,
                        value: row.value,
                        break_before: row.break_before,
                        supplying_boundary: row.supplying_boundary,
                        canonical: 0,
                        sequence: 0,
                    },
                    row.span,
                    start.decimals,
                );
                block = Some(match block {
                    Some(previous) => previous.merge(piece)?,
                    None => piece,
                });
            }
            let block = block.ok_or_else(|| invalid("empty series block"))?;
            let (page, offset) = w.data.block(io, w.block.as_slice())?;
            w.runs.push(
                io,
                &RunBlock {
                    block: Block {
                        page,
                        offset,
                        rows: chunk.len() as u32,
                        first: low,
                        last: high,
                    },
                    summary: block,
                },
            )?;
            summary = Some(match summary {
                Some(previous) => previous.merge(block)?,
                None => block,
            });
            first = first.min(low);
            last = last.max(high);
        }
        detail.blocks[RUNS] = (first_block, w.runs.count() - first_block);
        entry.summary = summary;
        entry.runs = Track {
            count: rows.len() as u64,
            first,
            last,
            head: chain(
                head.continues,
                head.span.last,
                end.canonicals == start.canonicals,
                self.hours,
            ),
        };
        Ok(())
    }

    fn events(
        &self,
        io: &mut impl PageWrite,
        w: &mut Writers,
        rows: &[u32],
        entry: &mut Entry,
        detail: &mut Detail,
    ) -> io::Result<()> {
        let c = &self.columns;
        let mut sorted = engine_allocation::vec(rows.len())?;
        for &i in rows {
            let event = c.events[i as usize];
            sorted.try_push((event.native_hour(), event.sequence(), i))?;
        }
        sorted.sort_unstable();
        let late = sorted.partition_point(|(native, _, _)| *native < self.hours.first);
        if late != 0 {
            entry.late_at = w.late.count();
        }
        for &(_, _, i) in &sorted[..late] {
            let event = c.events[i as usize];
            entry.late.add(event.native_hour(), event.read_hour())?;
            w.late.push(io, &Late(event))?;
        }
        let first_block = w.events.count();
        for chunk in sorted[late..].chunks(BLOCK) {
            w.block.clear();
            let mut stats = Events::default();
            let mut native = i64::from(chunk[0].0);
            let mut sequence = 0_u64;
            for &(_, _, i) in chunk {
                let event = c.events[i as usize];
                stats.add(event.native_hour(), event.read_hour())?;
                entry.events.add(event.native_hour(), event.read_hour())?;
                put_event(&mut w.block, event, native, sequence)?;
                native = i64::from(event.native_hour());
                sequence = event.sequence();
            }
            let (page, offset) = w.data.block(io, w.block.as_slice())?;
            w.events.push(
                io,
                &EventBlock {
                    block: Block {
                        page,
                        offset,
                        rows: chunk.len() as u32,
                        first: stats.first,
                        last: stats.last,
                    },
                    read_min: stats.read_min,
                    read_max: stats.read_max,
                    unknown: stats.unknown,
                },
            )?;
        }
        detail.blocks[EVENTS] = (first_block, w.events.count() - first_block);
        Ok(())
    }
}

fn put_evidence_flags(s: &mut Sink, row: &Evidence, continues: bool) -> io::Result<()> {
    s.u8(u8::from(row.known == Knownness::Known)
        | u8::from(row.supplying_read.is_some()) << 1
        | u8::from(row.holder.is_some()) << 2
        | u8::from(row.identity_first.is_some()) << 3
        | u8::from(continues) << 4)?;
    if let Knownness::Unknown(reason) = row.known {
        s.var(reason)?;
    }
    if let Some(holder) = row.holder {
        s.var(holder)?;
    }
    if let Some(first) = row.identity_first {
        s.zig(i64::from(row.span.first) - i64::from(first))?;
    }
    Ok(())
}

fn put_event(s: &mut Sink, event: EventView, native: i64, sequence: u64) -> io::Result<()> {
    s.zig(i64::from(event.native_hour()) - native)?;
    s.zig(event.sequence().wrapping_sub(sequence) as i64)?;
    let read = event.read_hour();
    match event {
        EventView::Family(e) => {
            s.u8(u8::from(read.is_some()) << 1)?;
            if let Some(read) = read {
                s.zig(i64::from(read) - i64::from(e.native_hour))?;
            }
            s.var(e.kind)?;
            s.var(e.fields.len)?;
            if e.fields.len != 0 {
                s.var(e.fields.at)?;
            }
        }
        EventView::Evidence { value, .. } => {
            s.u8(1 | u8::from(read.is_some()) << 1)?;
            if let Some(read) = read {
                s.zig(i64::from(read) - i64::from(value.span.first))?;
            }
            s.var((i64::from(value.span.last) - i64::from(value.span.first)) as u64)?;
            put_evidence_flags(s, &value, false)?;
        }
    }
    Ok(())
}

/// Counts pages and bytes a seal writes.
struct Counted<'w, P> {
    io: &'w mut P,
    work: Work,
}
impl<P: PageWrite> crate::pages::PageRead for Counted<'_, P> {
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]> {
        self.io.identity(id)
    }
    fn len(&mut self, id: PageId) -> io::Result<u64> {
        self.io.len(id)
    }
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()> {
        self.io.read(id, offset, into)
    }
}
impl<P: PageWrite> PageWrite for Counted<'_, P> {
    fn append(&mut self, bytes: &[u8]) -> io::Result<PageId> {
        self.work.pages += 1;
        self.work.bytes += bytes.len() as u64;
        self.io.append(bytes)
    }
}
