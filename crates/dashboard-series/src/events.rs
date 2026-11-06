use crate::{
    add,
    Grow,
    codec::Binary,
    format::{BLOCK, EVENTS, Entry, Late, Node},
    invalid,
    pages::PageRead,
    query::{
        Cursor, EventQuery, EventView, PageResult, Selection, Snapshot, Viewport, selection_of,
    },
    read::EventDecoder,
    source::{Item, Source, Subject, Visibility, intersects},
    types::*,
};
use std::{io, sync::Arc};

const NO_EVENT: EventView = EventView::Family(Event {
    key: 0,
    native_hour: 0,
    read_hour: None,
    sequence: 0,
    kind: 0,
    fields: Blob::EMPTY,
});

#[derive(Clone, Copy)]
struct EventKey {
    key: u64,
    first: usize,
    len: usize,
    all: bool,
}
/// Selected keys in key order, each once, with their memberships unioned.
pub(crate) struct EventSet {
    keys: Vec<EventKey>,
    spans: Vec<Span>,
}
impl EventSet {
    fn new(selected: &[Selection<'_>]) -> io::Result<Self> {
        let capacity = selected.iter().try_fold(0_usize, |count, s| {
            count
                .checked_add(s.membership.map_or(0, |s| s.len()))
                .ok_or_else(engine_allocation::oom)
        })?;
        let mut order = engine_allocation::vec(selected.len())?;
        for (index, s) in selected.iter().enumerate() {
            order.try_push((s.key, index))?;
        }
        order.sort_unstable();
        let mut keys = engine_allocation::vec(selected.len())?;
        let mut spans = engine_allocation::vec(capacity)?;
        let mut at = 0;
        while at < order.len() {
            let key = order[at].0;
            let first = spans.len();
            let mut all = false;
            while at < order.len() && order[at].0 == key {
                if let Some(members) = selected[order[at].1].membership {
                    for &span in members {
                        Span::new(span.first, span.last)?;
                        spans.try_push(span)?;
                    }
                } else {
                    all = true;
                }
                at += 1;
            }
            if all {
                spans.truncate(first);
            } else {
                spans[first..].sort_unstable_by_key(|s| (s.first, s.last));
                let mut end = first;
                for index in first..spans.len() {
                    let span = spans[index];
                    if end > first && i64::from(spans[end - 1].last) + 1 >= i64::from(span.first) {
                        spans[end - 1].last = spans[end - 1].last.max(span.last);
                    } else {
                        spans[end] = span;
                        end += 1;
                    }
                }
                spans.truncate(end);
            }
            keys.try_push(EventKey {
                key,
                first,
                len: spans.len() - first,
                all,
            })?;
        }
        Ok(Self { keys, spans })
    }
    fn selection(&self, index: usize) -> Selection<'_> {
        let key = self.keys[index];
        Selection {
            key: key.key,
            membership: (!key.all).then_some(&self.spans[key.first..key.first + key.len]),
        }
    }
}

/// Order of (native hour, key, sequence), the current reader's dated order.
type Order = (i32, u64, u64);
fn order(event: &EventView) -> Order {
    (event.native_hour(), event.key(), event.sequence())
}

struct OnTime {
    segment: Arc<Node>,
    entry: Entry,
    block: u64,
    count: u64,
    decoder: Option<EventDecoder>,
}
struct LateCursor {
    node: Arc<Node>,
    index: u64,
    end: u64,
    head: Option<EventView>,
}
/// One key's visible events in native order from a strict lower bound.
struct Stream {
    selection: usize,
    low: (i32, u64),
    high: i32,
    on: Option<OnTime>,
    on_head: Option<EventView>,
    seek: Option<i32>,
    late: Vec<LateCursor>,
}
impl Stream {
    fn new<P: PageRead>(
        s: &mut Source<'_, P>,
        key: u64,
        selection: usize,
        low: (i32, u64),
        high: i32,
    ) -> io::Result<Self> {
        let mut late = Vec::new();
        for reference in s.r.root().nodes.clone() {
            let node = s.r.child(s.io, reference)?;
            let Some(entry) = s.event_entry(&node, key)? else {
                continue;
            };
            if entry.late.count == 0 || entry.late.last < low.0 || entry.late.first > high {
                continue;
            }
            let (index, end) = s.late_from(&node, &entry, low, high)?;
            if index < end {
                engine_allocation::reserve(&mut late, 1)?;
                late.push(LateCursor {
                    node,
                    index,
                    end,
                    head: None,
                });
            }
        }
        Ok(Self {
            selection,
            low,
            high,
            on: None,
            on_head: None,
            seek: (low.0 <= high).then_some(low.0),
            late,
        })
    }
    fn wanted(&self, event: &EventView, selection: Selection<'_>, visible: Visibility) -> bool {
        let hour = event.native_hour();
        (hour, event.sequence()) >= self.low
            && hour <= self.high
            && visible.sees(event.read_hour())
            && selection.includes(hour)
    }
    fn fill<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        key: u64,
        selection: Selection<'_>,
        visible: Visibility,
    ) -> io::Result<()> {
        while self.on_head.is_none() {
            if let Some(on) = self.on.as_mut() {
                if let Some(decoder) = on.decoder.as_mut() {
                    match decoder.next()? {
                        Some(event) => {
                            if event.native_hour() > self.high {
                                self.on = None;
                                self.seek = None;
                            } else if self.wanted(&event, selection, visible) {
                                self.on_head = Some(event);
                            }
                            continue;
                        }
                        None => on.decoder = None,
                    }
                }
                if on.block < on.count {
                    let block = s.event_block(&on.segment, &on.entry, on.block)?;
                    on.block += 1;
                    let stats = crate::format::Events {
                        count: u64::from(block.block.rows),
                        first: block.block.first,
                        last: block.block.last,
                        read_min: block.read_min,
                        read_max: block.read_max,
                        unknown: block.unknown,
                    };
                    if block.block.first > self.high {
                        self.on = None;
                        self.seek = None;
                        continue;
                    }
                    if block.block.last < self.low.0
                        || visible.none(&stats)
                        || !intersects(
                            selection,
                            Span {
                                first: block.block.first,
                                last: block.block.last,
                            },
                        )
                    {
                        continue;
                    }
                    let (page, at) = s.r.block(s.io, &on.segment, block.block)?;
                    on.decoder = Some(EventDecoder::new(page, at, key, block.block));
                    continue;
                }
                self.seek = on
                    .segment
                    .hours
                    .last
                    .checked_add(1)
                    .filter(|h| *h <= self.high);
                self.on = None;
                continue;
            }
            let Some(from) = self.seek else {
                return Ok(());
            };
            let low = self.low;
            let found = s.seek(Subject::key(key), from, true, &|e| {
                e.events.count != 0 && e.events.last >= low.0
            })?;
            let Some((segment, entry)) =
                found.filter(|(segment, _)| segment.hours.first <= self.high)
            else {
                self.seek = None;
                return Ok(());
            };
            let count = entry
                .detail
                .ok_or_else(|| invalid("series segment entry without its blocks"))?
                .blocks[EVENTS]
                .1;
            // Blocks starting by the full position, so duplicate dates are not rescanned.
            let (mut first, mut last) = (0, count);
            while first < last {
                let middle = first + (last - first) / 2;
                let block = s.event_block(&segment, &entry, middle)?.block;
                if block.first < low.0
                    || (block.first == low.0 && s.event_head(&segment, key, block)? <= low)
                {
                    first = middle + 1;
                } else {
                    last = middle;
                }
            }
            self.on = Some(OnTime {
                segment,
                entry,
                block: first.saturating_sub(1),
                count,
                decoder: None,
            });
        }
        Ok(())
    }
    fn fill_late<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        selection: Selection<'_>,
        visible: Visibility,
    ) -> io::Result<()> {
        for at in 0..self.late.len() {
            while self.late[at].head.is_none() && self.late[at].index < self.late[at].end {
                let cursor = &mut self.late[at];
                let event = s.r.record::<Late>(s.io, cursor.node.late, cursor.index)?.0;
                cursor.index += 1;
                let hour = event.native_hour();
                if (hour, event.sequence()) >= self.low
                    && hour <= self.high
                    && visible.sees(event.read_hour())
                    && selection.includes(hour)
                {
                    self.late[at].head = Some(event);
                }
            }
        }
        Ok(())
    }
    /// The stream's next event without taking it.
    fn peek<P: PageRead>(
        &mut self,
        s: &mut Source<'_, P>,
        key: u64,
        selection: Selection<'_>,
        visible: Visibility,
    ) -> io::Result<Option<EventView>> {
        self.fill(s, key, selection, visible)?;
        self.fill_late(s, selection, visible)?;
        let mut best = self.on_head;
        for cursor in &self.late {
            if let Some(event) = cursor.head {
                if best.is_none_or(|b| order(&event) < order(&b)) {
                    best = Some(event);
                }
            }
        }
        Ok(best)
    }
    fn take(&mut self, event: &EventView) {
        if self.on_head.is_some_and(|head| head == *event) {
            self.on_head = None;
            return;
        }
        for cursor in &mut self.late {
            if cursor.head.is_some_and(|head| head == *event) {
                cursor.head = None;
                return;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct HeapItem {
    sort: Order,
    stream: usize,
}
fn heap_push(heap: &mut Vec<HeapItem>, item: HeapItem) -> io::Result<()> {
    heap.try_push(item)?;
    let mut at = heap.len() - 1;
    while at > 0 {
        let parent = (at - 1) / 2;
        if heap[parent].sort <= heap[at].sort {
            break;
        }
        heap.swap(parent, at);
        at = parent;
    }
    Ok(())
}
fn heap_pop(heap: &mut Vec<HeapItem>) -> Option<HeapItem> {
    let last = heap.pop()?;
    if heap.is_empty() {
        return Some(last);
    }
    let result = std::mem::replace(&mut heap[0], last);
    let mut at = 0;
    loop {
        let left = at * 2 + 1;
        if left >= heap.len() {
            break;
        }
        let right = left + 1;
        let child = if right < heap.len() && heap[right].sort < heap[left].sort {
            right
        } else {
            left
        };
        if heap[at].sort <= heap[child].sort {
            break;
        }
        heap.swap(at, child);
        at = child;
    }
    Some(result)
}

enum Walk {
    Native {
        streams: Vec<Stream>,
        heap: Vec<HeapItem>,
    },
    Publication {
        segment: Option<i32>,
        buffer: Vec<EventView>,
        at: usize,
        after: Option<u64>,
    },
}
/// An event page's merge state, kept by the snapshot for the next page of the same reply.
pub(crate) struct EventWalk {
    binding: [u8; 32],
    next: Cursor,
    selected: EventSet,
    walk: Walk,
    total: u64,
    base: u64,
}

impl<'a, P: PageRead> Snapshot<'a, P> {
    fn event_visibility(&self, as_of: Option<i32>) -> Visibility {
        let last = self.accepted().last;
        Visibility {
            hour: as_of.unwrap_or(last).min(last),
            unknown: as_of.is_none(),
        }
    }
    fn event_binding(&self, query: EventQuery<'_>, kind: u8) -> io::Result<[u8; 32]> {
        Span::new(query.native.first, query.native.last)?;
        let selected = EventSet::new(query.selection)?;
        let mut bytes = [0; 6];
        bytes[0] = u8::from(query.as_of.is_some());
        bytes[1..5].copy_from_slice(&query.as_of.unwrap_or(0).to_le_bytes());
        bytes[5] = u8::from(query.order == EventOrder::Publication);
        let mut hash = blake3::Hasher::new();
        hash.update(&self.binding(kind, &[], query.native, &bytes)?);
        for index in 0..selected.keys.len() {
            let s = selected.selection(index);
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
        Ok(*hash.finalize().as_bytes())
    }
    /// Visible events of one key within `span` and its membership.
    fn count_key(
        &mut self,
        selection: Selection<'_>,
        span: Span,
        visible: Visibility,
    ) -> io::Result<u64> {
        let mut count = 0_u64;
        let mut s = self.source();
        if s.def(selection.key)?.is_none() {
            return Ok(0);
        }
        s.walk_events(selection.key, selection, span, visible, &mut |_, item| {
            count = add(
                count,
                match item {
                    Item::Many { count, .. } => count,
                    Item::One(_) => 1,
                },
            )?;
            Ok(true)
        })?;
        Ok(count)
    }
    pub fn event_count(&mut self, query: EventQuery<'_>) -> io::Result<u64> {
        self.event_binding(query, 6)?;
        let visible = self.event_visibility(query.as_of);
        let selected = EventSet::new(query.selection)?;
        let mut count = 0;
        for index in 0..selected.keys.len() {
            self.check()?;
            count = add(
                count,
                self.count_key(selected.selection(index), query.native, visible)?,
            )?;
        }
        Ok(count)
    }
    pub fn event_page(
        &mut self,
        query: EventQuery<'_>,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(EventView) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        self.check()?;
        let binding = self.event_binding(query, 7)?;
        let mut next = self.cursor(binding, cursor)?;
        let first = next.position;
        let limit = byte_budget / (Event::SIZE + 1);
        if limit == 0 {
            return Err(invalid("event page budget cannot hold one row"));
        }
        let visible = self.event_visibility(query.as_of);
        let resumed = self
            .event_walk
            .take()
            .filter(|walk| walk.binding == binding && cursor == Some(walk.next));
        let mut walk = match resumed {
            Some(walk) => walk,
            None => self.start_walk(query, cursor, next, binding, visible)?,
        };
        next.auxiliary = walk.total;
        next.base = walk.base;
        let mut returned = 0;
        let mut more = false;
        loop {
            self.check()?;
            let Some(event) = self.walk_next(&mut walk, query, visible)? else {
                break;
            };
            if returned == limit {
                more = true;
                break;
            }
            self.walk_take(&mut walk, query, visible)?;
            sink(event)?;
            returned += 1;
            next.position = add(next.position, 1)?;
            next.after = match query.order {
                EventOrder::Native => [
                    u64::from((event.native_hour() as u32) ^ 0x8000_0000),
                    event.key(),
                    event.sequence(),
                ],
                EventOrder::Publication => [event.sequence(), 0, 0],
            };
        }
        let page = PageResult {
            next: more.then_some(next),
            returned: returned as u64,
            total: Some(walk.total),
            first_ordinal: add(walk.base, first)?,
        };
        if page.next.is_some() {
            walk.next = next;
            self.event_walk = Some(walk);
        }
        Ok(page)
    }
    fn start_walk(
        &mut self,
        query: EventQuery<'_>,
        cursor: Option<Cursor>,
        next: Cursor,
        binding: [u8; 32],
        visible: Visibility,
    ) -> io::Result<EventWalk> {
        let selected = EventSet::new(query.selection)?;
        let total = if cursor.is_some() {
            next.auxiliary
        } else {
            self.event_count(query)?
        };
        let mut base = if cursor.is_some() { next.base } else { 0 };
        if cursor.is_none()
            && query.order == EventOrder::Native
            && selected.keys.iter().all(|key| key.all)
        {
            if let Some(last) = query.native.first.checked_sub(1) {
                for index in 0..selected.keys.len() {
                    base = add(
                        base,
                        self.count_key(
                            selection_of(selected.keys[index].key),
                            Span {
                                first: i32::MIN,
                                last,
                            },
                            visible,
                        )?,
                    )?;
                }
            }
        }
        let walk = match query.order {
            EventOrder::Native => {
                let after = cursor.map(|c| c.after);
                let mut streams = Vec::new();
                engine_allocation::reserve_exact(&mut streams, selected.keys.len())?;
                let mut heap = engine_allocation::vec(selected.keys.len())?;
                for index in 0..selected.keys.len() {
                    let key = selected.keys[index].key;
                    let Some(low) = lower_bound(query.native.first, key, after) else {
                        continue;
                    };
                    let mut s = self.source();
                    if s.def(key)?.is_none() {
                        continue;
                    }
                    let mut stream = Stream::new(&mut s, key, index, low, query.native.last)?;
                    let selection = selected.selection(index);
                    if let Some(event) = stream.peek(&mut s, key, selection, visible)? {
                        heap_push(
                            &mut heap,
                            HeapItem {
                                sort: order(&event),
                                stream: streams.len(),
                            },
                        )?;
                    }
                    streams.push(stream);
                }
                Walk::Native { streams, heap }
            }
            EventOrder::Publication => Walk::Publication {
                segment: None,
                buffer: Vec::new(),
                at: 0,
                after: cursor.map(|c| c.after[0]),
            },
        };
        Ok(EventWalk {
            binding,
            next,
            selected,
            walk,
            total,
            base,
        })
    }
    fn walk_next(
        &mut self,
        walk: &mut EventWalk,
        query: EventQuery<'_>,
        visible: Visibility,
    ) -> io::Result<Option<EventView>> {
        match &mut walk.walk {
            Walk::Native { streams, heap } => {
                let Some(item) = heap.first().copied() else {
                    return Ok(None);
                };
                let index = streams[item.stream].selection;
                let key = walk.selected.keys[index].key;
                let selection = walk.selected.selection(index);
                let mut s = self.source();
                streams[item.stream].peek(&mut s, key, selection, visible)
            }
            Walk::Publication {
                segment,
                buffer,
                at,
                after,
            } => {
                while *at == buffer.len() {
                    let from = match *segment {
                        Some(last) => match last.checked_add(1) {
                            Some(next) => next,
                            None => return Ok(None),
                        },
                        None => match *after {
                            None => self.accepted().first,
                            Some(sequence) => {
                                let Some(next) = sequence.checked_add(1) else {
                                    return Ok(None);
                                };
                                match self.reader.segment_of_sequence(&mut self.io, next)? {
                                    Some(node) => node.hours.first,
                                    None => return Ok(None),
                                }
                            }
                        },
                    };
                    if !self.accepted().contains(from) {
                        return Ok(None);
                    }
                    let mut s = self.source();
                    let node =
                        s.r.segment_at(s.io, from)?
                            .ok_or_else(|| invalid("series hierarchy has a hole"))?;
                    *segment = Some(node.hours.last);
                    buffer.clear();
                    *at = 0;
                    for index in 0..walk.selected.keys.len() {
                        let key = walk.selected.keys[index].key;
                        let selection = walk.selected.selection(index);
                        let Some(entry) = s.event_entry(&node, key)? else {
                            continue;
                        };
                        let mut keep = |event: EventView| -> io::Result<()> {
                            let hour = event.native_hour();
                            if query.native.contains(hour)
                                && visible.sees(event.read_hour())
                                && selection.includes(hour)
                                && after.is_none_or(|a| event.sequence() > a)
                            {
                                buffer.try_push(event)?;
                            }
                            Ok(())
                        };
                        segment_events(&mut s, &node, &entry, key, &mut keep)?;
                    }
                    buffer.sort_unstable_by_key(|event| event.sequence());
                }
                Ok(Some(buffer[*at]))
            }
        }
    }
    fn walk_take(
        &mut self,
        walk: &mut EventWalk,
        query: EventQuery<'_>,
        visible: Visibility,
    ) -> io::Result<()> {
        let _ = query;
        match &mut walk.walk {
            Walk::Native { streams, heap } => {
                let item = heap_pop(heap).ok_or_else(|| invalid("series event merge is empty"))?;
                let stream = &mut streams[item.stream];
                let index = stream.selection;
                let key = walk.selected.keys[index].key;
                let selection = walk.selected.selection(index);
                let mut s = self.source();
                let head = stream
                    .peek(&mut s, key, selection, visible)?
                    .ok_or_else(|| invalid("series event stream lost its head"))?;
                stream.take(&head);
                if let Some(event) = stream.peek(&mut s, key, selection, visible)? {
                    heap_push(
                        heap,
                        HeapItem {
                            sort: order(&event),
                            stream: item.stream,
                        },
                    )?;
                }
                Ok(())
            }
            Walk::Publication {
                buffer, at, after, ..
            } => {
                *after = Some(buffer[*at].sequence());
                *at += 1;
                Ok(())
            }
        }
    }

    /// The native hour of one key's ordinal-th visible event, and how many come before that hour.
    fn event_hour(
        &mut self,
        selection: Selection<'_>,
        visible: Visibility,
        ordinal: u64,
    ) -> io::Result<Option<(i32, u64)>> {
        selection.validate()?;
        let all = Span {
            first: i32::MIN,
            last: i32::MAX,
        };
        if self.count_key(selection, all, visible)? <= ordinal {
            return Ok(None);
        }
        let (mut low, mut high) = (i64::from(i32::MIN), i64::from(i32::MAX));
        while low < high {
            let middle = low + (high - low) / 2;
            let through = Span {
                first: i32::MIN,
                last: middle as i32,
            };
            if self.count_key(selection, through, visible)? > ordinal {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        let hour = low as i32;
        let before = match hour.checked_sub(1) {
            Some(last) => self.count_key(
                selection,
                Span {
                    first: i32::MIN,
                    last,
                },
                visible,
            )?,
            None => 0,
        };
        Ok(Some((hour, before)))
    }
    pub fn event_date(
        &mut self,
        selection: Selection<'_>,
        as_of: Option<i32>,
        ordinal: u64,
    ) -> io::Result<Option<i32>> {
        let visible = self.event_visibility(as_of);
        Ok(self
            .event_hour(selection, visible, ordinal)?
            .map(|(hour, _)| hour))
    }
    /// The ordinal-th visible event of one key in native order: its hour by
    /// counting, then its place among that hour's events by a merge in sequence order.
    pub fn event_raw(
        &mut self,
        selection: Selection<'_>,
        as_of: Option<i32>,
        ordinal: u64,
    ) -> io::Result<Option<EventView>> {
        let visible = self.event_visibility(as_of);
        let Some((hour, before)) = self.event_hour(selection, visible, ordinal)? else {
            return Ok(None);
        };
        let mut s = self.source();
        let mut stream = Stream::new(&mut s, selection.key, 0, (hour, 0), hour)?;
        for _ in before..ordinal {
            let event = stream
                .peek(&mut s, selection.key, selection, visible)?
                .ok_or_else(|| invalid("series event counts disagree with their rows"))?;
            stream.take(&event);
        }
        stream
            .peek(&mut s, selection.key, selection, visible)?
            .map(Some)
            .ok_or_else(|| invalid("series event counts disagree with their rows"))
    }
    pub fn latest_event_rows(
        &mut self,
        selections: &[Selection<'_>],
        through: i32,
        as_of: Option<i32>,
        mut sink: impl FnMut(usize, Option<EventView>) -> io::Result<()>,
    ) -> io::Result<()> {
        let visible = self.event_visibility(as_of);
        for (at, &selection) in selections.iter().enumerate() {
            selection.validate()?;
            self.check()?;
            let mut s = self.source();
            if s.def(selection.key)?.is_none() {
                sink(at, None)?;
                continue;
            }
            let event = latest(&mut s, selection, through, visible)?;
            sink(at, event)?;
        }
        Ok(())
    }
    pub fn latest_event(
        &mut self,
        selection: Selection<'_>,
        through: i32,
        as_of: Option<i32>,
    ) -> io::Result<Option<EventView>> {
        let mut result = None;
        self.latest_event_rows(&[selection], through, as_of, |_, event| {
            result = event;
            Ok(())
        })?;
        Ok(result)
    }
    pub fn event_count_through(
        &mut self,
        key: u64,
        through: i32,
        as_of: Option<i32>,
    ) -> io::Result<u64> {
        let selections = [selection_of(key)];
        self.event_count(EventQuery {
            selection: &selections,
            native: Span {
                first: i32::MIN,
                last: through,
            },
            as_of,
            order: EventOrder::Native,
        })
    }
    pub fn each_event(
        &mut self,
        mut query: EventQuery<'_>,
        byte_budget: usize,
        mut sink: impl FnMut(EventView) -> io::Result<()>,
    ) -> io::Result<()> {
        query.order = EventOrder::Publication;
        let mut cursor = None;
        loop {
            let page = self.event_page(query, cursor, byte_budget, &mut sink)?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// Clusters of the selected events in the viewport's 18-pixel cells. Each
    /// cluster's first ordinal is its first event's place among all selected
    /// keys' visible events, membership aside: one pass per key counts the
    /// events below each cluster's first, aggregates taken whole between firsts.
    pub fn event_clusters(
        &mut self,
        query: EventQuery<'_>,
        view: Viewport,
        cursor: Option<Cursor>,
        byte_budget: usize,
        mut sink: impl FnMut(Cluster) -> io::Result<()>,
    ) -> io::Result<PageResult> {
        self.check()?;
        let grid = MarkGrid::new(query.native, view)?;
        let mut extra = [0; 52];
        extra[..32].copy_from_slice(&self.event_binding(query, 8)?);
        extra[32..40].copy_from_slice(&view.from.to_bits().to_le_bytes());
        extra[40..48].copy_from_slice(&view.to.to_bits().to_le_bytes());
        extra[48..].copy_from_slice(&view.width.to_le_bytes());
        let binding = self.binding(8, &[], query.native, &extra)?;
        let mut cursor = self.cursor(binding, cursor)?;
        let first = cursor.position;
        let allowance = byte_budget / (std::mem::size_of::<Cluster>() + 24);
        if allowance == 0 {
            return Err(invalid("cluster page budget cannot hold one bin"));
        }
        if first >= grid.cells {
            return Ok(PageResult {
                next: None,
                returned: 0,
                total: None,
                first_ordinal: first,
            });
        }
        let count = (grid.cells - first).min(allowance as u64) as usize;
        let mut clusters = engine_allocation::vec(count)?;
        let mut firsts: Vec<Option<Order>> = engine_allocation::vec(count)?;
        for i in 0..count {
            clusters.try_push(grid.bin(first + i as u64).map(|span| Cluster {
                span,
                first_hour: None,
                last_hour: None,
                first_ordinal: 0,
                count: 0,
            }))?;
            firsts.try_push(None)?;
        }
        let extent = clusters
            .iter()
            .flatten()
            .fold(None, |extent: Option<Span>, c| {
                Some(extent.map_or(c.span, |s| Span {
                    first: s.first,
                    last: c.span.last,
                }))
            });
        let visible = self.event_visibility(query.as_of);
        let selected = EventSet::new(query.selection)?;
        let Some(extent) = extent else {
            cursor.position = add(first, count as u64)?;
            return Ok(PageResult {
                next: (cursor.position < grid.cells).then_some(cursor),
                returned: 0,
                total: None,
                first_ordinal: first,
            });
        };
        for index in 0..selected.keys.len() {
            self.check()?;
            let selection = selected.selection(index);
            let key = selection.key;
            let mut s = self.source();
            if s.def(key)?.is_none() {
                continue;
            }
            s.walk_events(key, selection, extent, visible, &mut |s, item| {
                let (count_here, low, high, single) = match item {
                    Item::Many { count, first, last } => (count, first, last, None),
                    Item::One(event) => (1, event.native_hour(), event.native_hour(), Some(event)),
                };
                let cell = grid.index(low);
                if cell.is_none() || cell != grid.index(high) {
                    return Ok(false);
                }
                let cell = cell.ok_or_else(|| invalid("series cluster cell"))?;
                if cell < first || cell - first >= count as u64 {
                    return Err(invalid("cluster node outside tile"));
                }
                let at = (cell - first) as usize;
                let cluster = clusters[at]
                    .as_mut()
                    .ok_or_else(|| invalid("event in an empty mark cell"))?;
                cluster.count = add(cluster.count, count_here)?;
                cluster.first_hour = Some(cluster.first_hour.map_or(low, |h| h.min(low)));
                cluster.last_hour = Some(cluster.last_hour.map_or(high, |h| h.max(high)));
                let start = match single {
                    Some(event) => order(&event),
                    None => (low, key, first_sequence(s, key, selection, low, visible)?),
                };
                firsts[at] = Some(firsts[at].map_or(start, |old: Order| old.min(start)));
                Ok(true)
            })?;
        }
        let mut thresholds = engine_allocation::vec(count)?;
        for (at, start) in firsts.iter().enumerate() {
            if let Some(start) = start {
                thresholds.try_push((*start, at))?;
            }
        }
        let mut below = engine_allocation::vec(thresholds.len() + 1)?;
        for _ in 0..=thresholds.len() {
            below.try_push(0_u64)?;
        }
        for index in 0..selected.keys.len() {
            self.check()?;
            let key = selected.keys[index].key;
            let mut s = self.source();
            if s.def(key)?.is_none() {
                continue;
            }
            let Some(last) = thresholds.last().map(|(start, _)| start.0) else {
                break;
            };
            let unmasked = selection_of(key);
            s.walk_events(
                key,
                unmasked,
                Span {
                    first: i32::MIN,
                    last,
                },
                visible,
                &mut |_, item| {
                    let (count_here, low, high, single) = match item {
                        Item::Many { count, first, last } => (count, first, last, None),
                        Item::One(event) => {
                            (1, event.native_hour(), event.native_hour(), Some(event))
                        }
                    };
                    // The first threshold this item sorts below, for its least and greatest event.
                    let rank = |at: Order| thresholds.partition_point(|(t, _)| *t <= at);
                    let at = match single {
                        Some(event) => rank(order(&event)),
                        None => {
                            let a = rank((low, key, 0));
                            if a != rank((high, key, u64::MAX)) {
                                return Ok(false);
                            }
                            a
                        }
                    };
                    below[at] = add(below[at], count_here)?;
                    Ok(true)
                },
            )?;
        }
        let mut running = 0_u64;
        let mut returned = 0;
        for (index, &(_, at)) in thresholds.iter().enumerate() {
            running = add(running, below[index])?;
            let cluster = clusters[at]
                .as_mut()
                .ok_or_else(|| invalid("missing first cluster event"))?;
            cluster.first_ordinal = running;
            sink(*cluster)?;
            returned += 1;
        }
        cursor.position = add(first, count as u64)?;
        Ok(PageResult {
            next: (cursor.position < grid.cells).then_some(cursor),
            returned,
            total: None,
            first_ordinal: first,
        })
    }
}

/// The lower bound of a key's events after a native-order resume position.
fn lower_bound(first: i32, key: u64, after: Option<[u64; 3]>) -> Option<(i32, u64)> {
    let Some(after) = after else {
        return Some((first, 0));
    };
    let date = ((after[0] as u32) ^ 0x8000_0000) as i32;
    let low = if key < after[1] {
        (date.checked_add(1)?, 0)
    } else if key > after[1] {
        (date, 0)
    } else if after[2] == u64::MAX {
        (date.checked_add(1)?, 0)
    } else {
        (date, after[2] + 1)
    };
    Some(if low < (first, 0) { (first, 0) } else { low })
}

/// The least sequence among a key's visible member events at native `hour`.
fn first_sequence<P: PageRead>(
    s: &mut Source<'_, P>,
    key: u64,
    selection: Selection<'_>,
    hour: i32,
    visible: Visibility,
) -> io::Result<u64> {
    let mut least: Option<u64> = None;
    s.walk_events(
        key,
        selection,
        Span {
            first: hour,
            last: hour,
        },
        visible,
        &mut |_, item| match item {
            Item::Many { .. } => Ok(false),
            Item::One(event) => {
                least = Some(least.map_or(event.sequence(), |l| l.min(event.sequence())));
                Ok(true)
            }
        },
    )?;
    least.ok_or_else(|| invalid("series event aggregate without its first event"))
}

/// A segment's events of one key: its on-time blocks, then its own late events.
fn segment_events<P: PageRead>(
    s: &mut Source<'_, P>,
    node: &Node,
    entry: &Entry,
    key: u64,
    keep: &mut dyn FnMut(EventView) -> io::Result<()>,
) -> io::Result<()> {
    let count = entry
        .detail
        .ok_or_else(|| invalid("series segment entry without its blocks"))?
        .blocks[EVENTS]
        .1;
    let mut rows = [NO_EVENT; BLOCK];
    for index in 0..count {
        let block = s.event_block(node, entry, index)?;
        let n = s.event_rows(node, key, block.block, &mut rows)?;
        for event in &rows[..n] {
            keep(*event)?;
        }
    }
    for index in entry.late_at..add(entry.late_at, entry.late.count)? {
        keep(s.r.record::<Late>(s.io, node.late, index)?.0)?;
    }
    Ok(())
}

/// A key's last visible member event dated by `through`.
fn latest<P: PageRead>(
    s: &mut Source<'_, P>,
    selection: Selection<'_>,
    through: i32,
    visible: Visibility,
) -> io::Result<Option<EventView>> {
    let key = selection.key;
    let mut best: Option<EventView> = None;
    let consider = |event: EventView, best: &mut Option<EventView>| {
        if best.is_none_or(|b| order(&event) > order(&b)) {
            *best = Some(event);
        }
    };
    let mut from = through;
    let mut rows = [NO_EVENT; BLOCK];
    'segments: while let Some((segment, entry)) = s.seek(Subject::key(key), from, false, &|e| {
        e.events.count != 0 && e.events.first <= through
    })? {
        let count = entry
            .detail
            .ok_or_else(|| invalid("series segment entry without its blocks"))?
            .blocks[EVENTS]
            .1;
        for index in (0..count).rev() {
            let block = s.event_block(&segment, &entry, index)?;
            let stats = crate::format::Events {
                count: u64::from(block.block.rows),
                first: block.block.first,
                last: block.block.last,
                read_min: block.read_min,
                read_max: block.read_max,
                unknown: block.unknown,
            };
            if block.block.first > through
                || visible.none(&stats)
                || !intersects(
                    selection,
                    Span {
                        first: block.block.first,
                        last: block.block.last,
                    },
                )
            {
                continue;
            }
            let n = s.event_rows(&segment, key, block.block, &mut rows)?;
            if let Some(event) = rows[..n].iter().rev().find(|event| {
                event.native_hour() <= through
                    && visible.sees(event.read_hour())
                    && selection.includes(event.native_hour())
            }) {
                consider(*event, &mut best);
                break 'segments;
            }
        }
        match segment.hours.first.checked_sub(1) {
            Some(before) => from = before,
            None => break,
        }
    }
    for reference in s.r.root().nodes.clone() {
        let node = s.r.child(s.io, reference)?;
        let Some(entry) = s.event_entry(&node, key)? else {
            continue;
        };
        if entry.late.count == 0 || entry.late.first > through {
            continue;
        }
        let (low, high) = s.late_range(
            &node,
            &entry,
            Span {
                first: i32::MIN,
                last: through,
            },
        )?;
        for index in (low..high).rev() {
            let event = s.r.record::<Late>(s.io, node.late, index)?.0;
            if visible.sees(event.read_hour()) && selection.includes(event.native_hour()) {
                consider(event, &mut best);
                break;
            }
        }
    }
    Ok(best)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cluster {
    pub span: Span,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub first_ordinal: u64,
    pub count: u64,
}
#[derive(Clone, Copy)]
struct MarkGrid {
    bounds: Span,
    view: Viewport,
    cells: u64,
}
impl MarkGrid {
    fn new(bounds: Span, view: Viewport) -> io::Result<Self> {
        if !view.from.is_finite() || !view.to.is_finite() || view.from > view.to || view.width == 0
        {
            return Err(invalid("invalid event cluster viewport"));
        }
        Ok(Self {
            bounds,
            view,
            cells: u64::from(view.width.div_ceil(18)),
        })
    }
    fn raw(self, cell: u64) -> (i32, i32) {
        let start =
            self.view.from + (self.view.to - self.view.from) * cell as f64 / self.cells as f64;
        let end = self.view.from
            + (self.view.to - self.view.from) * (cell + 1) as f64 / self.cells as f64;
        (
            start.ceil() as i32,
            if cell + 1 == self.cells {
                end.floor()
            } else {
                end.ceil() - 1.0
            } as i32,
        )
    }
    fn bin(self, cell: u64) -> Option<Span> {
        let (first, last) = self.raw(cell);
        (first <= last)
            .then_some(Span { first, last })?
            .intersect(self.bounds)
    }
    fn index(self, hour: i32) -> Option<u64> {
        let (mut low, mut high) = (0, self.cells);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.raw(middle).1 < hour {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        (low < self.cells && self.bin(low).is_some_and(|span| span.contains(hour))).then_some(low)
    }
}
