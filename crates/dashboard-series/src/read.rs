use crate::{
    codec::{Binary, Decode},
    format::{
        BLOCK, Block, Definition, Detail, Directory, Entry, Held, List, Node, NodeRef, Origin,
        Root, Table,
    },
    invalid,
    layout::{Fence, per_page},
    pages::{self, Kind, PAGE_BYTES, Page, PageRead},
    query::EventView,
    types::*,
};
use std::{collections::HashMap, io, sync::Arc};

const IDS: u64 = (PAGE_BYTES / 8) as u64;

#[cfg(test)]
thread_local! {
    /// Run rows decoded on this thread, for work bounds that do not depend on timing.
    pub(crate) static DECODED_RUNS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// On-time event rows and late records decoded on this thread.
    pub(crate) static DECODED_EVENTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One run row as stored: a fragment of its canonical run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fragment {
    pub span: Span,
    pub value: i128,
    pub break_before: bool,
    pub supplying_boundary: bool,
    pub continues: bool,
}

/// The part of a pinned publication every query of it walks: its root and interior nodes.
pub(crate) struct Upper {
    root: Root,
    id: PageId,
    nodes: HashMap<PageId, Arc<Node>>,
}

/// Reads one publication's hierarchy. It holds every node and page it visits until it is
/// dropped, so a query decodes each once; pages are shared with every reader holding them.
pub(crate) struct Reader {
    root: Root,
    id: PageId,
    upper: Option<Arc<Upper>>,
    nodes: HashMap<PageId, Arc<Node>>,
    pages: HashMap<PageId, Arc<Page>>,
}

impl Reader {
    pub fn open(io: &mut impl PageRead, id: PageId) -> io::Result<Self> {
        let page = pages::shared(io, id, Kind::Root)?;
        let root = Root::decode(page.bytes())?;
        Self::with(root, id)
    }
    /// A reader starting from `upper`, which it shares.
    pub fn upper(upper: Arc<Upper>) -> Self {
        Self {
            root: upper.root.clone(),
            id: upper.id,
            upper: Some(upper),
            nodes: HashMap::new(),
            pages: HashMap::new(),
        }
    }
    /// Its root and every interior node under it, read once.
    pub fn pin(&mut self, io: &mut impl PageRead) -> io::Result<Upper> {
        let mut nodes = HashMap::new();
        let mut stack = engine_allocation::vec(self.root.nodes.len())?;
        stack.extend(self.root.nodes.iter().filter(|node| node.level > 0).copied());
        while let Some(reference) = stack.pop() {
            let node = self.child(io, reference)?;
            engine_allocation::reserve(&mut stack, node.children.len())?;
            stack.extend(node.children.iter().filter(|child| child.level > 0).copied());
            engine_allocation::reserve_map(&mut nodes, 1)?;
            nodes.insert(reference.page, node);
        }
        Ok(Upper {
            root: self.root.clone(),
            id: self.id,
            nodes,
        })
    }
    /// The reader of `root`, a prefix of its publication, keeping what it visited.
    pub fn rebase(&mut self, root: Root, id: PageId) -> io::Result<()> {
        let rebased = Self::with(root, id)?;
        self.root = rebased.root;
        self.id = rebased.id;
        Ok(())
    }
    pub fn with(root: Root, id: PageId) -> io::Result<Self> {
        let mut segment = 0_u64;
        let mut hour = i64::from(root.state.publication.accepted.first);
        for node in &root.nodes {
            if node.first != segment || i64::from(node.hours.first) != hour {
                return Err(invalid("series hierarchy is not contiguous"));
            }
            segment = crate::add(segment, node.segments)?;
            hour = i64::from(node.hours.last) + 1;
        }
        if !root.nodes.is_empty() && hour - 1 != i64::from(root.state.publication.accepted.last) {
            return Err(invalid("series hierarchy does not reach the accepted hour"));
        }
        Ok(Self {
            root,
            id,
            upper: None,
            nodes: HashMap::new(),
            pages: HashMap::new(),
        })
    }
    /// A reader of nodes alone, for merging them before any root names them.
    pub fn detached() -> Self {
        Self {
            root: Root {
                identity: Identity {
                    source: [0; 32],
                    content: [0; 32],
                    branch: [0; 32],
                    schema: 0,
                },
                state: crate::format::State {
                    publication: Publication {
                        generation: 0,
                        segment: 0,
                        accepted: Span::default(),
                        reached: 0,
                        refusal: None,
                    },
                    next_key: 0,
                    next_event: 0,
                    heap: 0,
                    checkpoint: Blob::EMPTY,
                },
                nodes: Vec::new(),
            },
            id: NONE,
            upper: None,
            nodes: HashMap::new(),
            pages: HashMap::new(),
        }
    }
    /// A reader of the hierarchy `nodes` before any root names it, for what its merges continue.
    pub fn frontier(nodes: &[NodeRef]) -> io::Result<Self> {
        let mut reader = Self::detached();
        let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else {
            return Ok(reader);
        };
        let state = &mut reader.root.state;
        state.publication.accepted = Span::new(first.hours.first, last.hours.last)?;
        state.publication.reached = last.hours.last;
        state.next_key = NONE;
        reader.root.nodes = engine_allocation::copy_vec(nodes)?;
        Self::with(reader.root, NONE)
    }
    pub fn root(&self) -> &Root {
        &self.root
    }
    pub fn id(&self) -> PageId {
        self.id
    }
    pub fn node(&mut self, io: &mut impl PageRead, page: PageId) -> io::Result<Arc<Node>> {
        if let Some(node) = self.nodes.get(&page) {
            return Ok(Arc::clone(node));
        }
        if let Some(node) = self.upper.as_ref().and_then(|upper| upper.nodes.get(&page)) {
            return Ok(Arc::clone(node));
        }
        let bytes = pages::shared(io, page, Kind::Node)?;
        let node = Arc::new(Node::decode(bytes.bytes())?);
        engine_allocation::reserve_map(&mut self.nodes, 1)?;
        self.nodes.insert(page, Arc::clone(&node));
        Ok(node)
    }
    /// Page `id`, held from its first visit until the reader is dropped.
    fn visit(&mut self, io: &mut impl PageRead, id: PageId, kind: Kind) -> io::Result<Arc<Page>> {
        if let Some(page) = self.pages.get(&id) {
            if page.kind() != kind {
                return Err(invalid("series page kind"));
            }
            return Ok(Arc::clone(page));
        }
        let page = pages::shared(io, id, kind)?;
        engine_allocation::reserve_map(&mut self.pages, 1)?;
        self.pages.insert(id, Arc::clone(&page));
        Ok(page)
    }
    pub fn child(&mut self, io: &mut impl PageRead, reference: NodeRef) -> io::Result<Arc<Node>> {
        let node = self.node(io, reference.page)?;
        if node.level != reference.level
            || node.first != reference.first
            || node.segments != reference.segments
            || node.hours != reference.hours
        {
            return Err(invalid("series node does not match its reference"));
        }
        Ok(node)
    }

    pub fn list_page(
        &mut self,
        io: &mut impl PageRead,
        list: List,
        ordinal: u64,
    ) -> io::Result<PageId> {
        if ordinal >= list.pages {
            return Err(invalid("series page ordinal outside its list"));
        }
        if list.levels == 0 {
            return Ok(list.root);
        }
        let mut span = 1_u64;
        for _ in 1..list.levels {
            span = span.saturating_mul(IDS);
        }
        let mut page = list.root;
        let mut left = ordinal;
        for _ in 0..list.levels {
            let index = self.visit(io, page, Kind::List)?;
            let at = usize::try_from(left / span)
                .ok()
                .and_then(|at| at.checked_mul(8))
                .ok_or_else(|| invalid("series list"))?;
            page = Decode::at(index.bytes(), at).u64()?;
            left %= span;
            span = (span / IDS).max(1);
        }
        Ok(page)
    }
    pub fn page(
        &mut self,
        io: &mut impl PageRead,
        list: List,
        ordinal: u64,
        kind: Kind,
    ) -> io::Result<Arc<Page>> {
        let id = self.list_page(io, list, ordinal)?;
        self.visit(io, id, kind)
    }
    /// [`Self::page`] for a pass over a table: this reader does not keep the page.
    pub fn page_once(
        &mut self,
        io: &mut impl PageRead,
        list: List,
        ordinal: u64,
        kind: Kind,
    ) -> io::Result<Arc<Page>> {
        let id = self.list_page(io, list, ordinal)?;
        match self.pages.get(&id) {
            Some(page) => Ok(Arc::clone(page)),
            None => pages::shared(io, id, kind),
        }
    }
    pub fn record<T: Binary>(
        &mut self,
        io: &mut impl PageRead,
        table: Table,
        index: u64,
    ) -> io::Result<T> {
        if index >= table.count {
            return Err(invalid("series record outside its table"));
        }
        let per = per_page::<T>() as u64;
        let page = self.page(io, table.list, index / per, Kind::Records)?;
        let at = (index % per) as usize * T::SIZE;
        let bytes = page
            .bytes()
            .get(at..at + T::SIZE)
            .ok_or_else(|| invalid("series record outside its page"))?;
        T::decode(&mut Decode::new(bytes))
    }

    /// The entry for `key`, by fence then binary search in one page.
    pub fn entry(
        &mut self,
        io: &mut impl PageRead,
        dir: Directory,
        key: u64,
    ) -> io::Result<Option<Entry>> {
        if dir.count == 0 {
            return Ok(None);
        }
        let (mut low, mut high) = (0, dir.fences.count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.record::<Fence>(io, dir.fences, middle)?.0 <= key {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let Some(page) = low.checked_sub(1) else {
            return Ok(None);
        };
        let page = self.page(io, dir.list, page, Kind::Records)?;
        let bytes = page.bytes();
        let count = entry_count(bytes)?;
        let keys = |i: usize| Decode::at(bytes, 4 + i * 8).u64();
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if keys(middle)? < key {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low == count || keys(low)? != key {
            return Ok(None);
        }
        entry_in(bytes, count, low).map(Some)
    }
    pub fn each_entry(
        &mut self,
        io: &mut impl PageRead,
        dir: Directory,
        visit: &mut dyn FnMut(Entry) -> io::Result<()>,
    ) -> io::Result<()> {
        for ordinal in 0..dir.list.pages {
            let page = self.page(io, dir.list, ordinal, Kind::Records)?;
            let bytes = page.bytes();
            let count = entry_count(bytes)?;
            for at in 0..count {
                visit(entry_in(bytes, count, at)?)?;
            }
        }
        Ok(())
    }

    /// The segment containing `hour`, found from the frontier down.
    pub fn segment_at(
        &mut self,
        io: &mut impl PageRead,
        hour: i32,
    ) -> io::Result<Option<Arc<Node>>> {
        match self.segment_ref(io, hour)? {
            Some(reference) => self.child(io, reference).map(Some),
            None => Ok(None),
        }
    }
    pub fn segment_ref(
        &mut self,
        io: &mut impl PageRead,
        hour: i32,
    ) -> io::Result<Option<NodeRef>> {
        let at = self.root.nodes.partition_point(|n| n.hours.last < hour);
        let Some(&reference) = self.root.nodes.get(at).filter(|n| n.hours.first <= hour) else {
            return Ok(None);
        };
        let mut reference = reference;
        while reference.level > 0 {
            let node = self.child(io, reference)?;
            let at = node.children.partition_point(|n| n.hours.last < hour);
            reference = *node
                .children
                .get(at)
                .ok_or_else(|| invalid("series node children do not cover its hours"))?;
        }
        Ok(Some(reference))
    }
    /// The largest node starting exactly at `hour`, if a node starts there.
    pub fn node_at(&mut self, io: &mut impl PageRead, hour: i32) -> io::Result<Option<Arc<Node>>> {
        let at = self.root.nodes.partition_point(|n| n.hours.last < hour);
        let Some(&reference) = self.root.nodes.get(at).filter(|n| n.hours.first <= hour) else {
            return Ok(None);
        };
        let mut node = self.child(io, reference)?;
        while node.hours.first != hour {
            if node.level == 0 {
                return Ok(None);
            }
            let at = node.children.partition_point(|n| n.hours.last < hour);
            let reference = *node
                .children
                .get(at)
                .ok_or_else(|| invalid("series node children do not cover its hours"))?;
            node = self.child(io, reference)?;
        }
        Ok(Some(node))
    }
    /// The largest node ending exactly at `hour`, if a node ends there.
    pub fn node_ending(
        &mut self,
        io: &mut impl PageRead,
        hour: i32,
    ) -> io::Result<Option<Arc<Node>>> {
        let at = self.root.nodes.partition_point(|n| n.hours.last < hour);
        let Some(&reference) = self.root.nodes.get(at).filter(|n| n.hours.first <= hour) else {
            return Ok(None);
        };
        let mut node = self.child(io, reference)?;
        while node.hours.last != hour {
            if node.level == 0 {
                return Ok(None);
            }
            let at = node.children.partition_point(|n| n.hours.last < hour);
            let reference = *node
                .children
                .get(at)
                .ok_or_else(|| invalid("series node children do not cover its hours"))?;
            node = self.child(io, reference)?;
        }
        Ok(Some(node))
    }
    /// The segment holding publication sequence `sequence`.
    pub fn segment_of_sequence(
        &mut self,
        io: &mut impl PageRead,
        sequence: u64,
    ) -> io::Result<Option<Arc<Node>>> {
        let mut found = None;
        for reference in self.root.nodes.clone() {
            let node = self.child(io, reference)?;
            if sequence < node.sequences.1 {
                found = Some(node);
                break;
            }
        }
        let Some(mut node) = found else {
            return Ok(None);
        };
        while node.level > 0 {
            let mut next = None;
            for reference in node.children.clone() {
                let child = self.child(io, reference)?;
                if sequence < child.sequences.1 {
                    next = Some(child);
                    break;
                }
            }
            node = next.ok_or_else(|| invalid("series sequence outside its node's children"))?;
        }
        Ok(Some(node))
    }
    /// The last segment of the publication.
    pub fn last_segment(&mut self, io: &mut impl PageRead) -> io::Result<Option<Arc<Node>>> {
        let last = self.root.state.publication.accepted.last;
        if self.root.nodes.is_empty() {
            return Ok(None);
        }
        self.segment_at(io, last)
    }

    /// Every segment in hour order.
    pub fn each_segment(
        &mut self,
        io: &mut impl PageRead,
        visit: &mut dyn FnMut(&mut Self, &mut dyn PageRead, Arc<Node>) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut stack = engine_allocation::vec(self.root.nodes.len())?;
        for node in self.root.nodes.iter().rev() {
            stack.push(*node);
        }
        while let Some(reference) = stack.pop() {
            let node = self.child(io, reference)?;
            if node.level == 0 {
                visit(self, io, node)?;
            } else {
                engine_allocation::reserve(&mut stack, node.children.len())?;
                for child in node.children.iter().rev() {
                    stack.push(*child);
                }
            }
        }
        Ok(())
    }

    pub fn definitions(
        &mut self,
        io: &mut impl PageRead,
        visit: &mut dyn FnMut(Definition) -> io::Result<()>,
    ) -> io::Result<()> {
        self.each_segment(io, &mut |reader, io, node| {
            let segment = node
                .segment
                .ok_or_else(|| invalid("series segment header"))?;
            let mut io = Dyn(io);
            for index in 0..segment.dictionary.count {
                visit(reader.record::<Definition>(&mut io, segment.dictionary, index)?)?;
            }
            Ok(())
        })
    }
    /// The definition of `key`, from the segment that created it.
    pub fn definition(
        &mut self,
        io: &mut impl PageRead,
        key: u64,
    ) -> io::Result<Option<Definition>> {
        if key >= self.root.state.next_key {
            return Ok(None);
        }
        let mut found = None;
        for &reference in &self.root.nodes.clone() {
            let node = self.child(io, reference)?;
            if node.keys.0 <= key && key < node.keys.1 {
                found = Some(node);
                break;
            }
        }
        let Some(mut node) = found else {
            return Err(invalid("series key outside every definition range"));
        };
        while node.level > 0 {
            let mut next = None;
            for &reference in &node.children {
                let child = self.child(io, reference)?;
                if child.keys.0 <= key && key < child.keys.1 {
                    next = Some(child);
                    break;
                }
            }
            node = next.ok_or_else(|| invalid("series key outside its node's children"))?;
        }
        let segment = node
            .segment
            .ok_or_else(|| invalid("series segment header"))?;
        let definition = self.record::<Definition>(io, segment.dictionary, key - node.keys.0)?;
        if definition.key.id != key {
            return Err(invalid("series dictionary order"));
        }
        Ok(Some(definition))
    }

    /// The run and evidence a held entry of `key` continues, as the segments that began them
    /// carried them out of their last hours.
    pub fn continued(
        &mut self,
        io: &mut impl PageRead,
        key: u64,
        held: Held,
    ) -> io::Result<(Option<Run>, Option<Evidence>)> {
        let mut origin = None;
        let run = if held.runs {
            let entry = self.began(io, key, held.origin, &mut origin)?;
            Some(
                entry
                    .detail
                    .and_then(|detail| detail.tail)
                    .filter(|run| run.span.first == held.origin)
                    .ok_or_else(|| invalid("series held run its origin did not carry"))?,
            )
        } else {
            None
        };
        let evidence = if held.evidence {
            let entry = self.began(io, key, held.evidence_origin, &mut origin)?;
            Some(
                entry
                    .detail
                    .and_then(|detail| detail.evidence_tail)
                    .filter(|evidence| evidence.span.first == held.evidence_origin)
                    .ok_or_else(|| invalid("series held evidence its origin did not carry"))?,
            )
        } else {
            None
        };
        Ok((run, evidence))
    }

    /// The entry of `key` in the segment holding `hour`, which began rows a later one holds.
    fn began(
        &mut self,
        io: &mut impl PageRead,
        key: u64,
        hour: i32,
        last: &mut Option<(Span, Entry)>,
    ) -> io::Result<Entry> {
        if let Some((_, entry)) = last.filter(|(hours, _)| hours.contains(hour)) {
            return Ok(entry);
        }
        let segment = self
            .segment_at(io, hour)?
            .ok_or_else(|| invalid("series held row without the segment that began it"))?;
        let entry = self
            .entry(io, segment.entries, key)?
            .filter(|entry| entry.held.is_none())
            .ok_or_else(|| invalid("series held row without the rows that began it"))?;
        *last = Some((segment.hours, entry));
        Ok(entry)
    }

    /// A held entry of `node` as the rows it continues read: its tracks over the node and, in a
    /// segment, its detail. Its summary is the caller's, which knows the key's scale.
    pub fn held(
        &mut self,
        io: &mut impl PageRead,
        node: &Node,
        entry: Entry,
    ) -> io::Result<(Entry, Option<Run>)> {
        let held = entry
            .held
            .ok_or_else(|| invalid("series entry holds rows of its own"))?;
        let (run, evidence) = self.continued(io, entry.key, held)?;
        if run.is_some_and(|run| crate::add(run.canonical, 1).ok() != Some(entry.canonical_start)) {
            return Err(invalid("series held run names another canonical run"));
        }
        let mut entry = entry.shaped(node.hours, node.segments);
        if node.level == 0 {
            let last = node.hours.last;
            entry.detail = Some(Detail {
                blocks: [
                    (0, u64::from(held.runs)),
                    (0, u64::from(held.evidence)),
                    (0, u64::from(held.coverage)),
                    (0, 0),
                ],
                origin: run.map_or(Origin::default(), |run| Origin {
                    first: run.span.first,
                    break_before: run.break_before,
                    supplying_boundary: run.supplying_boundary,
                    sequence: run.sequence,
                }),
                evidence_origin: evidence.map_or(0, |evidence| evidence.span.first),
                tail: run.map(|run| Run {
                    span: Span {
                        first: run.span.first,
                        last,
                    },
                    ..run
                }),
                evidence_tail: evidence.map(|evidence| Evidence {
                    span: Span {
                        first: evidence.span.first,
                        last,
                    },
                    ..evidence
                }),
            });
        }
        Ok((entry, run))
    }

    /// Each frontier node's entries, oldest node first, then the last segment's, held ones read
    /// as the rows they continue.
    pub fn latest_entries(
        &mut self,
        io: &mut impl PageRead,
        domain: bool,
        visit: &mut dyn FnMut(Entry, bool) -> io::Result<()>,
    ) -> io::Result<()> {
        for reference in self.root.nodes.clone() {
            let node = self.child(io, reference)?;
            let dir = if domain { node.domains } else { node.entries };
            self.each_entry(io, dir, &mut |entry| {
                visit(entry.shaped(node.hours, node.segments), false)
            })?;
        }
        if let Some(node) = self.last_segment(io)? {
            let dir = if domain { node.domains } else { node.entries };
            for ordinal in 0..dir.list.pages {
                let page = self.page(io, dir.list, ordinal, Kind::Records)?;
                let count = entry_count(page.bytes())?;
                for at in 0..count {
                    let entry = entry_in(page.bytes(), count, at)?;
                    let entry = if entry.held.is_some() {
                        self.held(io, &node, entry)?.0
                    } else {
                        entry
                    };
                    visit(entry, true)?;
                }
            }
        }
        Ok(())
    }

    /// The data page holding `block`, and its offset.
    pub fn block(
        &mut self,
        io: &mut impl PageRead,
        node: &Node,
        block: Block,
    ) -> io::Result<(Arc<Page>, usize)> {
        let segment = node
            .segment
            .ok_or_else(|| invalid("series blocks outside a segment"))?;
        let page = self.page(io, segment.data, block.page, Kind::Rows)?;
        if block.offset as usize > page.bytes().len() {
            return Err(invalid("series block outside its page"));
        }
        Ok((page, block.offset as usize))
    }
}

/// Lets a `dyn PageRead` stand where a sized reader is expected.
pub(crate) struct Dyn<'r>(pub &'r mut dyn PageRead);
impl PageRead for Dyn<'_> {
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]> {
        self.0.identity(id)
    }
    fn len(&mut self, id: PageId) -> io::Result<u64> {
        self.0.len(id)
    }
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()> {
        self.0.read(id, offset, into)
    }
}

/// A directory page's entry count, refused unless the page holds its keys and offsets.
pub(crate) fn entry_count(bytes: &[u8]) -> io::Result<usize> {
    let count = Decode::new(bytes).u32()? as usize;
    if count > bytes.len().saturating_sub(8) / 12 {
        return Err(invalid("series directory page count"));
    }
    Ok(count)
}

pub(crate) fn entry_in(bytes: &[u8], count: usize, at: usize) -> io::Result<Entry> {
    let key = Decode::at(bytes, 4 + at * 8).u64()?;
    let offsets = 4 + count * 8;
    let start = Decode::at(bytes, offsets + at * 4).u32()? as usize;
    let end = Decode::at(bytes, offsets + (at + 1) * 4).u32()? as usize;
    let body = offsets + (count + 1) * 4;
    let entry = bytes
        .get(body + start..body + end)
        .filter(|_| start <= end)
        .ok_or_else(|| invalid("series entry outside its page"))?;
    let mut d = Decode::new(entry);
    let value = Entry::get(key, &mut d)?;
    d.end()?;
    Ok(value)
}

fn checked_hour(value: Option<i64>) -> io::Result<i32> {
    value
        .and_then(|hour| i32::try_from(hour).ok())
        .ok_or_else(|| invalid("series hour width"))
}

pub(crate) fn runs(
    bytes: &[u8],
    at: usize,
    block: Block,
    out: &mut [Fragment; BLOCK],
) -> io::Result<usize> {
    let mut d = Decode::at(bytes, at);
    let mut previous = i64::from(block.first);
    let mut value = 0_i128;
    let rows = block.rows as usize;
    #[cfg(test)]
    DECODED_RUNS.with(|decoded| decoded.set(decoded.get() + rows as u64));
    for row in out.iter_mut().take(rows) {
        let first = checked_hour(previous.checked_add(d.zig()?))?;
        let length = i64::try_from(d.var()?).map_err(|_| invalid("series run length"))?;
        let last = checked_hour(i64::from(first).checked_add(length))?;
        let flags = d.u8()?;
        if flags > 7 {
            return Err(invalid("invalid run flags"));
        }
        value = value.wrapping_add(d.zig128()?);
        if first < block.first || last > block.last {
            return Err(invalid("series run outside its block"));
        }
        *row = Fragment {
            span: Span { first, last },
            value,
            break_before: flags & 1 != 0,
            supplying_boundary: flags & 2 != 0,
            continues: flags & 4 != 0,
        };
        previous = i64::from(last) + 1;
    }
    Ok(rows)
}

fn evidence_tail(
    d: &mut Decode<'_>,
    first: i32,
    last: i32,
    flags: u8,
    read: Option<i32>,
) -> io::Result<Evidence> {
    let known = if flags & 1 != 0 {
        Knownness::Known
    } else {
        Knownness::Unknown(d.var()?)
    };
    let holder = if flags & 4 != 0 { Some(d.var()?) } else { None };
    let identity_first = if flags & 8 != 0 {
        Some(checked_hour(i64::from(first).checked_sub(d.zig()?))?)
    } else {
        None
    };
    Ok(Evidence {
        span: Span::new(first, last)?,
        known,
        supplying_read: read,
        holder,
        identity_first,
    })
}

pub(crate) fn evidence(
    bytes: &[u8],
    at: usize,
    block: Block,
    out: &mut [(Evidence, bool); BLOCK],
) -> io::Result<usize> {
    let mut d = Decode::at(bytes, at);
    let mut previous = i64::from(block.first);
    let rows = block.rows as usize;
    for row in out.iter_mut().take(rows) {
        let first = checked_hour(previous.checked_add(d.zig()?))?;
        let length = i64::try_from(d.var()?).map_err(|_| invalid("series evidence length"))?;
        let last = checked_hour(i64::from(first).checked_add(length))?;
        let flags = d.u8()?;
        if flags > 31 {
            return Err(invalid("evidence flags"));
        }
        let mut value = evidence_tail(&mut d, first, last, flags, None)?;
        if flags & 2 != 0 {
            value.supplying_read = Some(checked_hour(i64::from(first).checked_add(d.zig()?))?);
        }
        if first < block.first || last > block.last {
            return Err(invalid("series evidence outside its block"));
        }
        *row = (value, flags & 16 != 0);
        previous = i64::from(last) + 1;
    }
    Ok(rows)
}

pub(crate) fn coverage(
    bytes: &[u8],
    at: usize,
    block: Block,
    out: &mut [Span; BLOCK],
) -> io::Result<usize> {
    let mut d = Decode::at(bytes, at);
    let mut previous = i64::from(block.first);
    let rows = block.rows as usize;
    for row in out.iter_mut().take(rows) {
        let first = checked_hour(previous.checked_add(d.zig()?))?;
        let length = i64::try_from(d.var()?).map_err(|_| invalid("series coverage length"))?;
        let last = checked_hour(i64::from(first).checked_add(length))?;
        if first < block.first || last > block.last {
            return Err(invalid("series coverage outside its block"));
        }
        *row = Span { first, last };
        previous = i64::from(last) + 1;
    }
    Ok(rows)
}

pub(crate) fn events(
    bytes: &[u8],
    at: usize,
    key: u64,
    block: Block,
    out: &mut [EventView; BLOCK],
) -> io::Result<usize> {
    let mut d = Decode::at(bytes, at);
    let mut native = i64::from(block.first);
    let mut sequence = 0_u64;
    let rows = block.rows as usize;
    for row in out.iter_mut().take(rows) {
        *row = event(&mut d, key, block, &mut native, &mut sequence)?;
    }
    Ok(rows)
}

/// One block's events, decoded one at a time so a long merge holds no buffer.
pub(crate) struct EventDecoder {
    page: Arc<Page>,
    at: usize,
    key: u64,
    block: Block,
    native: i64,
    sequence: u64,
    left: u32,
}
impl EventDecoder {
    pub fn new(page: Arc<Page>, at: usize, key: u64, block: Block) -> Self {
        Self {
            page,
            at,
            key,
            block,
            native: i64::from(block.first),
            sequence: 0,
            left: block.rows,
        }
    }
    pub fn next(&mut self) -> io::Result<Option<EventView>> {
        if self.left == 0 {
            return Ok(None);
        }
        let mut d = Decode::at(self.page.bytes(), self.at);
        let value = event(
            &mut d,
            self.key,
            self.block,
            &mut self.native,
            &mut self.sequence,
        )?;
        self.at = d.position();
        self.left -= 1;
        Ok(Some(value))
    }
}

fn event(
    d: &mut Decode<'_>,
    key: u64,
    block: Block,
    native: &mut i64,
    sequence: &mut u64,
) -> io::Result<EventView> {
    #[cfg(test)]
    DECODED_EVENTS.with(|decoded| decoded.set(decoded.get() + 1));
    let hour = checked_hour(native.checked_add(d.zig()?))?;
    *sequence = sequence.wrapping_add(d.zig()? as u64);
    let sequence = *sequence;
    let flags = d.u8()?;
    if flags > 3 {
        return Err(invalid("series event flags"));
    }
    let read = if flags & 2 != 0 {
        Some(checked_hour(i64::from(hour).checked_add(d.zig()?))?)
    } else {
        None
    };
    let value = if flags & 1 == 0 {
        let kind = d.var()?;
        let len = d.var()?;
        let fields = if len == 0 {
            Blob::EMPTY
        } else {
            Blob { at: d.var()?, len }
        };
        if len % Cell::SIZE as u64 != 0 || fields.at.checked_add(len).is_none() {
            return Err(invalid("event field table length"));
        }
        EventView::Family(Event {
            key,
            native_hour: hour,
            read_hour: read,
            sequence,
            kind,
            fields,
        })
    } else {
        let length = i64::try_from(d.var()?).map_err(|_| invalid("series evidence length"))?;
        let last = checked_hour(i64::from(hour).checked_add(length))?;
        let flags = d.u8()?;
        if flags > 15 {
            return Err(invalid("evidence flags"));
        }
        EventView::Evidence {
            key,
            sequence,
            value: evidence_tail(d, hour, last, flags, read)?,
        }
    };
    if hour < block.first || hour > block.last {
        return Err(invalid("series event outside its block"));
    }
    *native = i64::from(hour);
    Ok(value)
}
