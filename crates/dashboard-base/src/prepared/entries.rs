//! A keyed catalogue kept in one section's ranges as a B-tree of compressed
//! pages. Range 0 is the root; a branch lists each child's first key and
//! range, a leaf its entries in key order. A flush rewrites only the leaves
//! holding changed entries and the branches above them, and a query reads
//! only the pages on its way to the keys it asks for, so neither grows with
//! the entries the catalogue already holds. The writer holds only the changes
//! since its last flush.
use super::pages::Pages;
use super::series::{compress, decompress};
use crate::prepared_chart::{take, var};
use serde::{Serialize, de::DeserializeOwned};
use crate::metadata::check_cancel;
use std::{collections::BTreeMap, io, io::Read, sync::atomic::AtomicBool};

const LEAF: u8 = b'L';
const BRANCH: u8 = b'B';
/// A page splits once its keys and values pass this many bytes.
const PAGE_BYTES: usize = 64 * 1024;
const LEVEL: i32 = 3;

type Entry = (Vec<u8>, Vec<u8>);
type Child = (Vec<u8>, u64);

enum Node {
    Leaf(Vec<Entry>),
    Branch(Vec<Child>),
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn length(value: u128) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid("Prepared entry page length differs"))
}

fn bytes<'a>(rest: &mut &'a [u8]) -> io::Result<&'a [u8]> {
    let len = length(take(rest)?)?;
    if rest.len() < len {
        return Err(invalid("Prepared entry page is truncated"));
    }
    let (value, tail) = rest.split_at(len);
    *rest = tail;
    Ok(value)
}

impl Node {
    fn encode(&self) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        let kind = match self {
            Node::Leaf(entries) => {
                var(&mut body, entries.len() as u128);
                for (key, value) in entries {
                    var(&mut body, key.len() as u128);
                    body.extend_from_slice(key);
                    var(&mut body, value.len() as u128);
                    body.extend_from_slice(value);
                }
                LEAF
            }
            Node::Branch(children) => {
                var(&mut body, children.len() as u128);
                for (key, range) in children {
                    var(&mut body, key.len() as u128);
                    body.extend_from_slice(key);
                    var(&mut body, u128::from(*range));
                }
                BRANCH
            }
        };
        let mut page = vec![kind];
        page.extend(compress(&body, LEVEL)?);
        Ok(page)
    }

    fn decode(page: &[u8]) -> io::Result<Self> {
        let Some((&kind, compressed)) = page.split_first() else {
            return Err(invalid("Prepared entry page is empty"));
        };
        let body = decompress(compressed)?;
        let mut rest = body.as_slice();
        let count = length(take(&mut rest)?)?;
        let node = match kind {
            LEAF => {
                let mut entries = Vec::with_capacity(count.min(rest.len()));
                for _ in 0..count {
                    let key = bytes(&mut rest)?.to_vec();
                    entries.push((key, bytes(&mut rest)?.to_vec()));
                }
                Node::Leaf(entries)
            }
            BRANCH => {
                let mut children = Vec::with_capacity(count.min(rest.len()));
                for _ in 0..count {
                    let key = bytes(&mut rest)?.to_vec();
                    let range = u64::try_from(take(&mut rest)?)
                        .map_err(|_| invalid("Prepared entry child range differs"))?;
                    children.push((key, range));
                }
                if children.is_empty() {
                    return Err(invalid("Prepared entry branch has no children"));
                }
                Node::Branch(children)
            }
            _ => return Err(invalid("Prepared entry page kind differs")),
        };
        if !rest.is_empty() {
            return Err(invalid("Prepared entry page has trailing bytes"));
        }
        Ok(node)
    }
}

fn entry_bytes((key, value): &Entry) -> usize {
    key.len() + value.len() + 8
}

fn child_bytes((key, _): &Child) -> usize {
    key.len() + 12
}

/// `items` in consecutive runs of at most [`PAGE_BYTES`], each holding at
/// least one item.
fn pages<T>(items: Vec<T>, size: impl Fn(&T) -> usize) -> Vec<Vec<T>> {
    let mut pages = vec![Vec::new()];
    let mut held = 0;
    for item in items {
        let bytes = size(&item);
        let page = pages.last_mut().expect("a page");
        if !page.is_empty() && held + bytes > PAGE_BYTES {
            pages.push(Vec::new());
            held = 0;
        }
        held += bytes;
        pages.last_mut().expect("a page").push(item);
    }
    pages
}

fn read_node<W: Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    range: u64,
) -> io::Result<Option<Node>> {
    writer
        .read_at(section, range)?
        .filter(|page| !page.is_empty())
        .map(|page| Node::decode(&page))
        .transpose()
}

/// Write `nodes`, the first at `range` and the rest appended, as the
/// children that replace the node at `range`. The first keeps `lower`.
fn write_nodes<W: Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    range: u64,
    lower: &[u8],
    nodes: Vec<Node>,
) -> io::Result<Vec<Child>> {
    let mut children = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.into_iter().enumerate() {
        let first = match (&node, index) {
            (_, 0) => lower.to_vec(),
            (Node::Leaf(entries), _) => entries[0].0.clone(),
            (Node::Branch(nodes), _) => nodes[0].0.clone(),
        };
        let page = node.encode()?;
        let at = if index == 0 {
            writer.write_at(section, range, &page)?;
            range
        } else {
            writer.write(section, &page)?
        };
        children.push((first, at));
    }
    Ok(children)
}

/// Apply `changes` (in key order, all at or after `lower`) to the node at
/// `range`; the children replacing it, or None when it is unchanged.
fn update<W: Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    range: u64,
    lower: &[u8],
    node: Node,
    changes: &[(Vec<u8>, Option<Vec<u8>>)],
) -> io::Result<Option<Vec<Child>>> {
    match node {
        Node::Leaf(entries) => {
            let mut merged = Vec::with_capacity(entries.len() + changes.len());
            let mut old = entries.into_iter().peekable();
            let mut changed = false;
            for (key, value) in changes {
                while let Some(entry) = old.next_if(|entry| entry.0 < *key) {
                    merged.push(entry);
                }
                let previous = old.next_if(|entry| entry.0 == *key);
                changed |= previous.as_ref().map(|entry| &entry.1) != value.as_ref();
                if let Some(value) = value {
                    merged.push((key.clone(), value.clone()));
                }
            }
            merged.extend(old);
            if !changed {
                return Ok(None);
            }
            let nodes = pages(merged, entry_bytes)
                .into_iter()
                .map(Node::Leaf)
                .collect();
            write_nodes(writer, section, range, lower, nodes).map(Some)
        }
        Node::Branch(children) => {
            let mut replaced = Vec::with_capacity(children.len());
            let mut changed = false;
            let mut rest = changes;
            for (index, (first, child)) in children.iter().enumerate() {
                let end = children.get(index + 1).map_or(rest.len(), |next| {
                    rest.partition_point(|change| change.0 < next.0)
                });
                let (mine, after) = rest.split_at(end);
                rest = after;
                if mine.is_empty() {
                    replaced.push((first.clone(), *child));
                    continue;
                }
                let node = read_node(writer, section, *child)?
                    .ok_or_else(|| invalid("Prepared entry child page is missing"))?;
                match update(writer, section, *child, first, node, mine)? {
                    Some(parts) => {
                        changed = true;
                        replaced.extend(parts);
                    }
                    None => replaced.push((first.clone(), *child)),
                }
            }
            if !changed {
                return Ok(None);
            }
            let nodes = pages(replaced, child_bytes)
                .into_iter()
                .map(Node::Branch)
                .collect();
            write_nodes(writer, section, range, lower, nodes).map(Some)
        }
    }
}

/// The changes to a catalogue since its last flush ([`Self::flush`]).
#[derive(Default)]
pub struct Entries {
    changes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    bytes: usize,
}

impl Entries {
    /// Set `key` to `value`'s JSON.
    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: &impl Serialize) -> io::Result<()> {
        self.put_bytes(key, serde_json::to_vec(value)?);
        Ok(())
    }

    /// Set `key` to `value`.
    pub fn put_bytes(&mut self, key: impl Into<Vec<u8>>, value: Vec<u8>) {
        let key = key.into();
        self.bytes += key.len() + value.len();
        self.changes.insert(key, Some(value));
    }

    /// Remove `key`.
    pub fn remove(&mut self, key: impl Into<Vec<u8>>) {
        let key = key.into();
        self.bytes += key.len();
        self.changes.insert(key, None);
    }

    /// What the unflushed changes hold, about.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Write the changes into `section`'s tree and hold none.
    pub fn flush<W: Pages + ?Sized>(&mut self, writer: &mut W, section: &str) -> io::Result<()> {
        if self.changes.is_empty() {
            return Ok(());
        }
        let changes: Vec<_> = std::mem::take(&mut self.changes).into_iter().collect();
        self.bytes = 0;
        let root = read_node(writer, section, 0)?.unwrap_or(Node::Leaf(Vec::new()));
        let Some(mut children) = update(writer, section, 0, &[], root, &changes)? else {
            return Ok(());
        };
        // The root stays at range 0: a split root moves its first part out
        // and a branch above the parts takes its place.
        while children.len() > 1 {
            if children[0].1 == 0 {
                let page = writer
                    .read_at(section, 0)?
                    .ok_or_else(|| invalid("Prepared entry root is missing"))?;
                children[0].1 = writer.write(section, &page)?;
            }
            let mut nodes = pages(children, child_bytes);
            if nodes.len() == 1 {
                let root = Node::Branch(nodes.pop().expect("a page"));
                writer.write_at(section, 0, &root.encode()?)?;
                return Ok(());
            }
            children = Vec::with_capacity(nodes.len());
            for node in nodes {
                let first = node[0].0.clone();
                let range = writer.write(section, &Node::Branch(node).encode()?)?;
                children.push((first, range));
            }
        }
        Ok(())
    }
}

/// Where a [`Reader`] takes its pages from.
pub trait PageSource {
    /// `range`'s bytes, or None when it holds none.
    fn page(&mut self, range: u64) -> io::Result<Option<Vec<u8>>>;
    /// After a read failed because a flush overtook this source's
    /// publication, moves to the store's newer one; false when there is none.
    fn renew(&mut self) -> io::Result<bool>;
}

/// One publication of a catalogue in a store. A flush rewrites pages in
/// place, so everything one reply reads comes from one publication.
pub struct Published<'a> {
    store: &'a store::Store,
    key: &'a store::SourceKey,
    session: store::Session,
    snapshot: store::Snapshot,
    section: &'a str,
    cancelled: Option<&'a AtomicBool>,
    /// The range whose pack this publication lost.
    lost: Option<u64>,
    /// The store's session has moved past this publication.
    moved: bool,
}

fn overtaken() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "A flush overtook the prepared entries' publication",
    )
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OVERTAKE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `between` when this thread's next [`compose`] reads its first page,
/// then reads that composition again over the store's publication after it:
/// a flush overtaking a query.
#[cfg(any(test, feature = "test-support"))]
pub fn overtake_next_composition(between: impl FnOnce() + 'static) {
    OVERTAKE.with(|hook| *hook.borrow_mut() = Some(Box::new(between)));
}

impl PageSource for Published<'_> {
    fn page(&mut self, range: u64) -> io::Result<Option<Vec<u8>>> {
        if let Some(cancelled) = self.cancelled {
            check_cancel(cancelled)?;
            #[cfg(any(test, feature = "test-support"))]
            if let Some(between) = OVERTAKE.with(|hook| hook.borrow_mut().take()) {
                between();
                self.moved = true;
                return Err(overtaken());
            }
        }
        let page = self.snapshot.read(self.section, range).and_then(|found| {
            found
                .map(|mut found| {
                    let mut bytes = Vec::with_capacity(usize::try_from(found.len()).unwrap_or(0));
                    found.read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
                .transpose()
        });
        if page
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            self.lost = Some(range);
        }
        page
    }

    fn renew(&mut self) -> io::Result<bool> {
        let lost = self.lost.take();
        if lost.is_none() && !std::mem::take(&mut self.moved) {
            return Ok(false);
        }
        if let Some(range) = lost {
            // Reading a removed pack's range moves the session, which a query
            // shares with its other readers, to the store's current publication.
            self.session.read(self.section, range)?;
        }
        let Some(session) = self.store.session(self.key)? else {
            return Ok(false);
        };
        let snapshot = session.snapshot();
        if let Some(range) = lost
            && let Err(error) = snapshot.read(self.section, range)
        {
            return match error.kind() {
                io::ErrorKind::NotFound => Ok(false),
                _ => Err(error),
            };
        }
        self.session = session;
        self.snapshot = snapshot;
        Ok(true)
    }
}

/// A catalogue as a writer holds it, its unflushed changes aside.
pub struct Written<'a, W: ?Sized> {
    writer: &'a mut W,
    section: &'a str,
}

impl<W: Pages + ?Sized> PageSource for Written<'_, W> {
    fn page(&mut self, range: u64) -> io::Result<Option<Vec<u8>>> {
        self.writer.read_at(self.section, range)
    }

    fn renew(&mut self) -> io::Result<bool> {
        Ok(false)
    }
}

/// Reads a catalogue's entries from its pages.
pub struct Reader<S> {
    source: S,
}

/// A catalogue `section` as a store's latest publication for `key` holds it.
/// A read that composes a reply from several reads uses [`compose`].
pub fn read<'a>(
    store: &'a store::Store,
    key: &'a store::SourceKey,
    section: &'a str,
) -> io::Result<Option<Reader<Published<'a>>>> {
    published(store, key, section, None)
}

fn published<'a>(
    store: &'a store::Store,
    key: &'a store::SourceKey,
    section: &'a str,
    cancelled: Option<&'a AtomicBool>,
) -> io::Result<Option<Reader<Published<'a>>>> {
    let Some(session) = store.session(key)? else {
        return Ok(None);
    };
    Ok(Some(Reader {
        source: Published {
            store,
            key,
            snapshot: session.snapshot(),
            session,
            section,
            cancelled,
            lost: None,
            moved: false,
        },
    }))
}

/// `read`'s result over one publication of `section`, read again whole from
/// the store's newer publication when a flush overtakes the one it read:
/// removes a page it needs, or appends to a section it also reads
/// ([`Reader::agrees`]). None before any publication.
pub fn compose<'a, T>(
    store: &'a store::Store,
    key: &'a store::SourceKey,
    section: &'a str,
    cancelled: &'a AtomicBool,
    read: impl FnMut(&mut Reader<Published<'a>>) -> io::Result<T>,
) -> io::Result<Option<T>> {
    let Some(mut reader) = published(store, key, section, Some(cancelled))? else {
        return Ok(None);
    };
    reader.whole(cancelled, read).map(Some)
}

/// A catalogue `section` as `writer` holds it, its unflushed changes aside.
pub fn read_written<'a, W: Pages + ?Sized>(
    writer: &'a mut W,
    section: &'a str,
) -> Reader<Written<'a, W>> {
    Reader {
        source: Written { writer, section },
    }
}

impl Reader<Published<'_>> {
    /// Fails, so that [`compose`] reads again over the store's current
    /// publication, when `section`, which the composition also reads through
    /// the store, has ranges this reader's publication lacks. A flush only
    /// appends to such a section.
    pub fn agrees(&mut self, section: &str) -> io::Result<()> {
        let current = self
            .source
            .store
            .session(self.source.key)?
            .and_then(|session| session.range_count(section));
        if current == self.source.snapshot.range_count(section) {
            return Ok(());
        }
        self.source.moved = true;
        Err(overtaken())
    }
}

impl<S: PageSource> Reader<S> {
    fn node(&mut self, range: u64) -> io::Result<Option<Node>> {
        self.source
            .page(range)?
            .filter(|page| !page.is_empty())
            .map(|page| Node::decode(&page))
            .transpose()
    }

    /// `read` over this reader's publication, again from the start over a
    /// newer one when a flush removed a page it needed.
    fn whole<T>(
        &mut self,
        cancelled: &AtomicBool,
        mut read: impl FnMut(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        loop {
            check_cancel(cancelled)?;
            match read(self) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if !self.source.renew()? {
                        return Err(error);
                    }
                }
                result => return result,
            }
        }
    }

    /// `key`'s value.
    pub fn get(&mut self, key: &[u8]) -> io::Result<Option<Vec<u8>>> {
        let mut range = 0;
        loop {
            match self.node(range)? {
                None if range == 0 => return Ok(None),
                None => return Err(invalid("Prepared entry child page is missing")),
                Some(Node::Leaf(entries)) => {
                    return Ok(entries
                        .binary_search_by(|entry| entry.0.as_slice().cmp(key))
                        .ok()
                        .map(|index| entries[index].1.clone()));
                }
                Some(Node::Branch(children)) => {
                    let index = children
                        .partition_point(|child| child.0.as_slice() <= key)
                        .saturating_sub(1);
                    range = children[index].1;
                }
            }
        }
    }

    /// `key`'s value, read as JSON.
    pub fn get_json<T: DeserializeOwned>(&mut self, key: &[u8]) -> io::Result<Option<T>> {
        self.get(key)?
            .map(|value| serde_json::from_slice(&value).map_err(io::Error::from))
            .transpose()
    }

    /// The entries whose keys are at least `from` and, with `to`, before it,
    /// in key order.
    pub fn range(&mut self, from: &[u8], to: Option<&[u8]>) -> io::Result<Vec<Entry>> {
        let mut found = Vec::new();
        self.collect(0, from, to, &mut found)?;
        Ok(found)
    }

    /// The entries whose keys start with `prefix`, in key order.
    pub fn prefix(&mut self, prefix: &[u8]) -> io::Result<Vec<Entry>> {
        let end = successor(prefix);
        self.range(prefix, end.as_deref())
    }

    fn collect(
        &mut self,
        range: u64,
        from: &[u8],
        to: Option<&[u8]>,
        found: &mut Vec<Entry>,
    ) -> io::Result<()> {
        match self.node(range)? {
            None if range == 0 => Ok(()),
            None => Err(invalid("Prepared entry child page is missing")),
            Some(Node::Leaf(entries)) => {
                found.extend(entries.into_iter().filter(|(key, _)| {
                    key.as_slice() >= from && to.is_none_or(|to| key.as_slice() < to)
                }));
                Ok(())
            }
            Some(Node::Branch(children)) => {
                let start = children
                    .partition_point(|child| child.0.as_slice() <= from)
                    .saturating_sub(1);
                for child in &children[start..] {
                    if to.is_some_and(|to| child.0.as_slice() >= to) {
                        break;
                    }
                    self.collect(child.1, from, to, found)?;
                }
                Ok(())
            }
        }
    }
}

/// The least key after every key starting with `prefix`; None when there is
/// none.
fn successor(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < u8::MAX {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

#[cfg(test)]
mod tests;
