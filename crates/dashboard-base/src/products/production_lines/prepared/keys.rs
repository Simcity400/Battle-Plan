//! Each history's keyed values beyond those its fold holds: a handle's latest lifetime once
//! its line closed, a variant's last design read, an organisation's last version.
//! They lie in a copy-on-write B+ tree in `production-ledger`, one root per history, its
//! nodes shared by forks and by every later root. A history holds only the values read or
//! changed since the sweep before; a sweep writes the rest into its tree, in one batch.

use super::*;
use std::collections::HashMap;
use std::collections::btree_map::Entry as Slot;
use std::sync::mpsc;

pub(super) type Key = (u8, u64);
pub(super) const HANDLE: u8 = 0;
pub(super) const DESIGN: u8 = 1;
pub(super) const ORGANISATION: u8 = 2;

/// A node's most entries or children.
const FANOUT: usize = 64;
/// A leaf's most value bytes, beyond its first entry.
const LEAF_BYTES: usize = 8 * 1024;
/// The range a node names a child written in its own range by.
const SAME: u64 = u64::MAX;
/// Deeper than any tree of 2^64 keys.
const DEPTH: usize = 64;

/// Where a node lies in `production-ledger`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct NodeRef {
    pub range: u64,
    pub offset: u64,
    pub len: u64,
}

product_codec::product! { impl NodeRef {
    range: u64,
    offset: u64,
    len: u64,
} default {} }

product_codec::product! {
    enum Stored {
        0 => Leaf { entries: Vec<(Key, Vec<u8>)> },
        1 => Inner { children: Vec<(Key, (u64, u64, u64))> },
    }
}

/// A node as read, its children's ranges resolved.
enum Node {
    Leaf(Vec<(Key, Vec<u8>)>),
    Inner(Vec<(Key, NodeRef)>),
}

fn read_node(pages: &mut dyn Pages, section: &str, at: NodeRef) -> io::Result<Node> {
    let len = usize::try_from(at.len)
        .map_err(|_| crate::invalid("Production keyed node exceeds the address space"))?;
    if len == 0 {
        return Err(crate::invalid("Empty production keyed node"));
    }
    // Its end must lie in its range before anything is allocated for it.
    let end = at
        .offset
        .checked_add(at.len)
        .ok_or_else(|| crate::invalid("Production keyed node overflows its range"))?;
    pages.read_slice(section, at.range, end, &mut [])?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(io::Error::other)?;
    bytes.resize(len, 0);
    pages.read_slice(section, at.range, at.offset, &mut bytes)?;
    match index::unpack::<Stored>(&bytes)? {
        Stored::Leaf { entries } => {
            if entries.is_empty() || !entries.windows(2).all(|pair| pair[0].0 < pair[1].0) {
                return Err(crate::invalid("Invalid production keyed leaf"));
            }
            Ok(Node::Leaf(entries))
        }
        Stored::Inner { children } => {
            if children.is_empty() || !children.windows(2).all(|pair| pair[0].0 < pair[1].0) {
                return Err(crate::invalid("Invalid production keyed node"));
            }
            let children = children
                .into_iter()
                .map(|(key, (range, offset, len))| {
                    let child = NodeRef {
                        range: if range == SAME { at.range } else { range },
                        offset,
                        len,
                    };
                    // A child is written before its parent, so no path comes back to a node.
                    let earlier = child.range < at.range
                        || (child.range == at.range
                            && offset.checked_add(len).is_some_and(|end| end <= at.offset));
                    if earlier {
                        Ok((key, child))
                    } else {
                        Err(crate::invalid("Production keyed node names a later node"))
                    }
                })
                .collect::<io::Result<Vec<_>>>()?;
            Ok(Node::Inner(children))
        }
    }
}

/// The nodes decoded since the last sweep: those on the paths to the values the histories
/// read since, which they hold until that sweep. A sweep lets them go.
#[derive(Default)]
struct Nodes(HashMap<NodeRef, Arc<Node>>);

impl Nodes {
    fn get(&mut self, pages: &mut dyn Pages, section: &str, at: NodeRef) -> io::Result<Arc<Node>> {
        if let Some(node) = self.0.get(&at) {
            return Ok(Arc::clone(node));
        }
        let node = Arc::new(read_node(pages, section, at)?);
        self.0.insert(at, Arc::clone(&node));
        Ok(node)
    }
}

/// What every history's tree is read through: the stream its nodes lie in, the nodes read
/// lately, and the sweep generation a read marks its value with.
pub(super) struct Trees {
    pub section: String,
    /// Where the absence's unread spans are paged.
    pub absence: String,
    nodes: Nodes,
    generation: u64,
}

impl Trees {
    pub fn new(branch: u64) -> Self {
        Self {
            section: index::ledger_section(branch),
            absence: index::absence_section(branch),
            nodes: Nodes::default(),
            generation: 0,
        }
    }

    /// The sweep generation reads mark their values with.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Ends a sweep: the next reads begin a generation, and the nodes read before go.
    pub fn swept(&mut self) {
        self.generation += 1;
        self.nodes.0 = HashMap::new();
    }

    /// The nodes held decoded.
    #[cfg(test)]
    pub fn cached(&self) -> usize {
        self.nodes.0.len()
    }
}

/// Where a history's fold reads its tree's nodes: the fold's own pages, or, on a shard's
/// thread, the fold's thread that holds them.
enum Reader<'a> {
    Local {
        pages: &'a mut dyn Pages,
        trees: &'a mut Trees,
    },
    Remote(&'a Remote),
}

/// What a history's fold reads beyond its own state, and the chunk its events go into.
pub(super) struct Ctx<'a> {
    reader: Reader<'a>,
    /// The sweep generation a read or change marks its value with.
    pub generation: u64,
    pub chunk: u64,
}

impl<'a> Ctx<'a> {
    pub fn new(pages: &'a mut dyn Pages, trees: &'a mut Trees, chunk: u64) -> Self {
        Self {
            generation: trees.generation,
            reader: Reader::Local { pages, trees },
            chunk,
        }
    }

    /// A shard's context, reading nodes through `remote`.
    pub fn remote(remote: &'a Remote, generation: u64, chunk: u64) -> Self {
        Self {
            reader: Reader::Remote(remote),
            generation,
            chunk,
        }
    }

    /// The fold's own pages and trees, which a shard's thread never holds.
    pub fn local(&mut self) -> io::Result<(&mut (dyn Pages + 'a), &mut Trees)> {
        match &mut self.reader {
            Reader::Local { pages, trees } => Ok((&mut **pages, &mut **trees)),
            Reader::Remote(_) => Err(crate::invalid(
                "A production shard reads pages only through the fold's thread",
            )),
        }
    }

    /// The value at `key` of the tree at `root`.
    fn get(&mut self, root: Option<NodeRef>, key: Key) -> io::Result<Option<Vec<u8>>> {
        match &mut self.reader {
            Reader::Local { pages, trees } => get(&mut **pages, &mut **trees, root, key),
            Reader::Remote(remote) => find(|at| remote.node(at), root, key),
        }
    }
}

/// A node a shard asks the fold's thread to read, and where the answer goes.
pub(super) struct Request {
    at: NodeRef,
    reply: mpsc::Sender<io::Result<Arc<Node>>>,
}

impl Request {
    /// Reads the node asked for and answers it.
    pub fn serve(self, pages: &mut dyn Pages, trees: &mut Trees) {
        let node = trees.nodes.get(pages, &trees.section, self.at);
        // A shard that is gone asks nothing more.
        let _ = self.reply.send(node);
    }
}

/// A shard's way to the fold thread's pages.
pub(super) struct Remote {
    requests: mpsc::Sender<Request>,
}

impl Remote {
    pub fn new(requests: mpsc::Sender<Request>) -> Self {
        Self { requests }
    }

    fn node(&self, at: NodeRef) -> io::Result<Arc<Node>> {
        let gone = || crate::invalid("The production fold's thread stopped serving its shards");
        // Its own answer's way, so a request dropped unanswered ends the wait.
        let (reply, answer) = mpsc::channel();
        self.requests
            .send(Request { at, reply })
            .map_err(|_| gone())?;
        answer.recv().map_err(|_| gone())?
    }
}

/// The value at `key` of the tree at `root`.
fn get(
    pages: &mut dyn Pages,
    trees: &mut Trees,
    root: Option<NodeRef>,
    key: Key,
) -> io::Result<Option<Vec<u8>>> {
    find(|at| trees.nodes.get(pages, &trees.section, at), root, key)
}

/// The value at `key` of the tree at `root`, its nodes read by `read`.
fn find(
    mut read: impl FnMut(NodeRef) -> io::Result<Arc<Node>>,
    root: Option<NodeRef>,
    key: Key,
) -> io::Result<Option<Vec<u8>>> {
    let Some(mut at) = root else {
        return Ok(None);
    };
    for _ in 0..DEPTH {
        let node = read(at)?;
        match &*node {
            Node::Leaf(entries) => {
                return Ok(entries
                    .binary_search_by(|(found, _)| found.cmp(&key))
                    .ok()
                    .map(|index| entries[index].1.clone()));
            }
            Node::Inner(children) => {
                let index = children
                    .partition_point(|(first, _)| *first <= key)
                    .saturating_sub(1);
                at = children[index].1;
            }
        }
    }
    Err(crate::invalid("Production keyed tree is too deep"))
}

/// The nodes one sweep writes, laid out as one range of `production-ledger`.
#[derive(Default)]
pub(super) struct Builder {
    bytes: Vec<u8>,
}

impl Builder {
    fn put(&mut self, node: &Stored) -> io::Result<NodeRef> {
        let packed = index::pack(node)?;
        let at = NodeRef {
            range: SAME,
            offset: self.bytes.len() as u64,
            len: packed.len() as u64,
        };
        self.bytes.extend(packed);
        Ok(at)
    }

    /// Appends the nodes; the range they lie in, if there are any.
    pub fn write(self, pages: &mut dyn Pages, section: &str) -> io::Result<Option<u64>> {
        if self.bytes.is_empty() {
            return Ok(None);
        }
        pages.write(section, &self.bytes).map(Some)
    }
}

/// `at` once its builder's nodes lie in `range`.
pub(super) fn placed(at: Option<NodeRef>, range: u64) -> Option<NodeRef> {
    at.map(|at| {
        if at.range == SAME {
            NodeRef { range, ..at }
        } else {
            at
        }
    })
}

/// The tree at `root` with `batch` (sorted, unique) written over it; its new nodes go into
/// `out`, the rest are shared.
fn insert(
    pages: &mut dyn Pages,
    trees: &mut Trees,
    root: Option<NodeRef>,
    batch: &[(Key, Vec<u8>)],
    out: &mut Builder,
) -> io::Result<Option<NodeRef>> {
    if batch.is_empty() {
        return Ok(root);
    }
    let mut level = merge(pages, trees, root, batch, out)?;
    while level.len() > 1 {
        level = inners(level, out)?;
    }
    Ok(level.pop().map(|(_, at)| at))
}

/// The nodes that replace `at` once `batch` is written into it, each with its first key.
fn merge(
    pages: &mut dyn Pages,
    trees: &mut Trees,
    at: Option<NodeRef>,
    batch: &[(Key, Vec<u8>)],
    out: &mut Builder,
) -> io::Result<Vec<(Key, NodeRef)>> {
    let Some(at) = at else {
        return leaves(batch.to_vec(), out);
    };
    let node = trees.nodes.get(pages, &trees.section, at)?;
    match &*node {
        Node::Leaf(entries) => {
            let mut merged = Vec::with_capacity(entries.len() + batch.len());
            let (mut old, mut new) = (entries.iter().peekable(), batch.iter().peekable());
            loop {
                match (old.peek(), new.peek()) {
                    (Some(a), Some(b)) if a.0 < b.0 => merged.push(old.next().unwrap().clone()),
                    (Some(a), Some(b)) if a.0 == b.0 => {
                        old.next();
                        merged.push(new.next().unwrap().clone());
                    }
                    (_, Some(_)) => merged.push(new.next().unwrap().clone()),
                    (Some(_), None) => merged.push(old.next().unwrap().clone()),
                    (None, None) => break,
                }
            }
            leaves(merged, out)
        }
        Node::Inner(children) => {
            let mut next = Vec::with_capacity(children.len() + 1);
            let mut from = 0;
            for (index, &(key, child)) in children.iter().enumerate() {
                let until = match children.get(index + 1) {
                    Some((bound, _)) => {
                        from + batch[from..].partition_point(|(key, _)| key < bound)
                    }
                    None => batch.len(),
                };
                if from == until {
                    next.push((key, child));
                } else {
                    next.extend(merge(pages, trees, Some(child), &batch[from..until], out)?);
                }
                from = until;
            }
            inners(next, out)
        }
    }
}

fn leaves(entries: Vec<(Key, Vec<u8>)>, out: &mut Builder) -> io::Result<Vec<(Key, NodeRef)>> {
    let mut written = Vec::new();
    let mut leaf: Vec<(Key, Vec<u8>)> = Vec::new();
    let mut bytes = 0;
    for entry in entries {
        if !leaf.is_empty() && (leaf.len() == FANOUT || bytes + entry.1.len() > LEAF_BYTES) {
            let entries = std::mem::take(&mut leaf);
            written.push((entries[0].0, out.put(&Stored::Leaf { entries })?));
            bytes = 0;
        }
        bytes += entry.1.len();
        leaf.push(entry);
    }
    if !leaf.is_empty() {
        written.push((leaf[0].0, out.put(&Stored::Leaf { entries: leaf })?));
    }
    Ok(written)
}

fn inners(children: Vec<(Key, NodeRef)>, out: &mut Builder) -> io::Result<Vec<(Key, NodeRef)>> {
    if children.is_empty() {
        return Ok(children);
    }
    let size = children.len().div_ceil(children.len().div_ceil(FANOUT));
    children
        .chunks(size)
        .map(|group| {
            let at = out.put(&Stored::Inner {
                children: group
                    .iter()
                    .map(|(key, at)| (*key, (at.range, at.offset, at.len)))
                    .collect(),
            })?;
            Ok((group[0].0, at))
        })
        .collect()
}

#[derive(Clone)]
struct Entry<V> {
    value: Option<V>,
    /// Its tree holds it as it stands.
    stored: bool,
    /// The sweep generation that last read or changed it.
    seen: u64,
}

/// One kind of a history's keyed values, those it holds. None is a value its tree was
/// asked for and does not hold.
#[derive(Clone)]
pub(super) struct Resident<V> {
    kind: u8,
    entries: BTreeMap<u64, Entry<V>>,
}

impl<V: product_codec::Product + Clone> Resident<V> {
    fn new(kind: u8) -> Self {
        Self {
            kind,
            entries: BTreeMap::new(),
        }
    }

    /// The value at `id`, read from the tree at `root` where it is not held.
    pub fn get(
        &mut self,
        id: u64,
        root: Option<NodeRef>,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<Option<&V>> {
        let seen = ctx.generation;
        let entry = match self.entries.entry(id) {
            Slot::Occupied(entry) => entry.into_mut(),
            Slot::Vacant(vacant) => {
                let value = ctx
                    .get(root, (self.kind, id))?
                    .map(|bytes| product_codec::decode::<V>(&bytes))
                    .transpose()?;
                vacant.insert(Entry {
                    value,
                    stored: true,
                    seen,
                })
            }
        };
        entry.seen = seen;
        Ok(entry.value.as_ref())
    }

    pub fn put(&mut self, id: u64, value: V, seen: u64) {
        self.entries.insert(
            id,
            Entry {
                value: Some(value),
                stored: false,
                seen,
            },
        );
    }

    /// The value at `id` where it is held and its tree does not hold it.
    pub fn unstored(&self, id: u64) -> Option<&V> {
        self.entries
            .get(&id)
            .filter(|entry| !entry.stored)
            .and_then(|entry| entry.value.as_ref())
    }

    /// Every value held that its tree does not hold, by id.
    pub fn all_unstored(&self) -> impl Iterator<Item = (u64, &V)> {
        self.entries
            .iter()
            .filter(|(_, entry)| !entry.stored)
            .filter_map(|(id, entry)| Some((*id, entry.value.as_ref()?)))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Values a ledger record holds, which the tree it names may not.
    pub fn restore(&mut self, values: impl IntoIterator<Item = (u64, V)>) {
        for (id, value) in values {
            self.put(id, value, 0);
        }
    }

    /// Lets go of the values not read since sweep `generation` (every value when `all`),
    /// those the tree lacks into `batch`; whether any went.
    fn sweep(
        &mut self,
        generation: u64,
        all: bool,
        batch: &mut Vec<(Key, Vec<u8>)>,
    ) -> io::Result<bool> {
        let before = self.entries.len();
        let kind = self.kind;
        let mut failed = None;
        self.entries.retain(|&id, entry| {
            if !all && entry.seen >= generation {
                return true;
            }
            if !entry.stored
                && let Some(value) = &entry.value
                && failed.is_none()
            {
                match product_codec::encode(value) {
                    Ok(bytes) => batch.push(((kind, id), bytes)),
                    Err(error) => failed = Some(error),
                }
            }
            false
        });
        match failed {
            Some(error) => Err(error),
            None => Ok(self.entries.len() != before),
        }
    }
}

/// A history's keyed values: its tree, and those it holds.
#[derive(Clone)]
pub(super) struct Keys {
    pub root: Option<NodeRef>,
    pub handles: Resident<Handle>,
    /// Each variant's last design read, and the chunk it was pushed in.
    pub designs: Resident<(DesignRead, u64)>,
    /// Each organisation's last version, and the chunk it was pushed in.
    pub organisations: Resident<(RawOrganisation, u64)>,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            root: None,
            handles: Resident::new(HANDLE),
            designs: Resident::new(DESIGN),
            organisations: Resident::new(ORGANISATION),
        }
    }
}

impl Keys {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.handles.len() + self.designs.len() + self.organisations.len()
    }

    /// Writes the values not read since sweep `generation` (every value when `all`) into
    /// the tree, its new nodes into `out`, and lets them go; whether any went.
    pub fn sweep(
        &mut self,
        generation: u64,
        all: bool,
        pages: &mut dyn Pages,
        trees: &mut Trees,
        out: &mut Builder,
    ) -> io::Result<bool> {
        // Kinds in order, each by id: the batch is sorted.
        let mut batch = Vec::new();
        let mut went = self.handles.sweep(generation, all, &mut batch)?;
        went |= self.designs.sweep(generation, all, &mut batch)?;
        went |= self.organisations.sweep(generation, all, &mut batch)?;
        self.root = insert(pages, trees, self.root, &batch, out)?;
        Ok(went)
    }
}

/// The absence's unread spans in the order recorded: how many, and those after its last
/// full page in `production-absence`.
#[derive(Clone, Debug, Default)]
pub(super) struct Stream {
    pub count: u64,
    pub tail: Vec<HourSpan>,
}

impl Stream {
    /// The stream a ledger record holds, refused unless its tail is what follows whole pages.
    pub fn restored(count: u64, tail: Vec<HourSpan>) -> io::Result<Self> {
        if !index::spans_add_up(count, tail.len()) {
            return Err(crate::invalid("Invalid production absence spans"));
        }
        Ok(Self { count, tail })
    }

    pub fn push(&mut self, first_hour: i32, last_hour: i32) -> io::Result<()> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| crate::invalid("Production absence spans overflow"))?;
        self.tail.push(HourSpan {
            first_hour,
            last_hour,
        });
        Ok(())
    }

    /// Appends each full page, holding only the last, partial one.
    pub fn page(&mut self, pages: &mut dyn Pages, section: &str) -> io::Result<()> {
        while self.tail.len() >= index::SPAN_PAGE {
            let paged = self
                .count
                .checked_sub(self.tail.len() as u64)
                .ok_or_else(|| crate::invalid("Invalid production absence spans"))?;
            let page = paged / index::SPAN_PAGE as u64;
            // Its range is its number, whatever an earlier preparation left past it.
            pages.write_at(
                section,
                page,
                &index::encode_spans(&self.tail[..index::SPAN_PAGE]),
            )?;
            self.tail.drain(..index::SPAN_PAGE);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u64) -> Key {
        (DESIGN, id)
    }

    /// Every key of a tree grown in batches reads back its latest value, and an earlier
    /// root still reads what it held.
    #[test]
    fn batches_written_over_a_tree_keep_every_root_readable() {
        let mut pages = fixtures::FixturePages::default();
        let mut trees = Trees::new(1);
        let mut roots = Vec::new();
        let mut root = None;
        for batch in 0..40_u64 {
            let entries: Vec<(Key, Vec<u8>)> = (0..300_u64)
                .map(|at| at * 7 % 2_000 + batch % 3)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|id| (key(id), product_codec::encode(&(id, batch)).unwrap()))
                .collect();
            let mut out = Builder::default();
            let next = insert(&mut pages, &mut trees, root, &entries, &mut out).unwrap();
            let range = out
                .write(&mut pages, &trees.section.clone())
                .unwrap()
                .unwrap();
            root = placed(next, range);
            roots.push((root, entries));
        }
        for (root, entries) in &roots {
            for (key, value) in entries {
                assert_eq!(
                    get(&mut pages, &mut trees, *root, *key).unwrap().as_ref(),
                    Some(value)
                );
            }
        }
        assert!(
            get(&mut pages, &mut trees, root, (HANDLE, 5))
                .unwrap()
                .is_none()
        );
        trees.swept();
        assert_eq!(trees.cached(), 0);
    }

    #[test]
    fn a_node_naming_a_later_node_or_a_huge_length_is_refused() {
        let mut pages = fixtures::FixturePages::default();
        let mut trees = Trees::new(1);
        let section = trees.section.clone();
        let cyclic = index::pack(&Stored::Inner {
            children: vec![((DESIGN, 1), (SAME, 0, 64))],
        })
        .unwrap();
        pages.write(&section, &cyclic).unwrap();
        let at = NodeRef {
            range: 0,
            offset: 0,
            len: cyclic.len() as u64,
        };
        let error = get(&mut pages, &mut trees, Some(at), (DESIGN, 1)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let huge = NodeRef {
            range: 0,
            offset: 0,
            len: u64::MAX,
        };
        let error = get(&mut pages, &mut trees, Some(huge), (DESIGN, 1)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn absence_spans_page_in_order_and_only_a_partial_tail_restores() {
        let page = index::SPAN_PAGE;
        let mut pages = fixtures::FixturePages::default();
        let mut stream = Stream::default();
        for at in 0..(page as i32 * 3 + 7) {
            stream.push(at * 10, at * 10 + 3).unwrap();
            if at % 100 == 0 {
                stream.page(&mut pages, "absence").unwrap();
            }
        }
        stream.page(&mut pages, "absence").unwrap();
        assert_eq!(stream.tail.len(), 7);
        for number in 0..3 {
            let spans =
                index::decode_spans(&pages.read_at("absence", number).unwrap().unwrap()).unwrap();
            assert_eq!(spans[0].first_hour, number as i32 * page as i32 * 10);
        }
        assert!(Stream::restored(stream.count, stream.tail.clone()).is_ok());
        let span = |at: i32| HourSpan {
            first_hour: at,
            last_hour: at,
        };
        // A count below its tail, a tail of a whole page, or one not after whole pages.
        for (count, tail) in [
            (0, (0..page as i32).map(span).collect::<Vec<_>>()),
            (page as u64, (0..page as i32).map(span).collect()),
            (5, vec![span(1), span(2), span(3)]),
            (u64::MAX, vec![span(1)]),
        ] {
            let error = Stream::restored(count, tail).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{count}");
        }
        let mut full = Stream {
            count: u64::MAX,
            tail: Vec::new(),
        };
        assert_eq!(
            full.push(1, 1).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
