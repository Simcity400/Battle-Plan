use std::cmp::Ordering;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::path::Path;

const PAGE: usize = 4096;
const HEADER: usize = 24;
const INLINE: usize = PAGE / 4;
const NONE: u64 = u64::MAX;
pub(crate) const TRANSIENT: usize = 1024 * 1024;
const WAYS: usize = 4;

pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Blob {
    file: usize,
    offset: u64,
    pub len: u64,
}

impl Blob {
    pub(crate) fn slice(self, offset: u64, len: u64) -> Self {
        Self {
            offset: self.offset + offset,
            len,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Memory {
    pub allowance: usize,
    pub cache_bytes: usize,
    pub metadata_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StorageStats {
    pub metadata_bytes: usize,
    pub resident_bytes: usize,
    pub peak_resident_bytes: usize,
    pub backing_bytes: u64,
    pub peak_backing_bytes: u64,
    pub live_bytes: u64,
    pub peak_live_bytes: u64,
    pub page_reads: u64,
    pub page_writes: u64,
    pub tree_operations: u64,
    pub page_decodes: u64,
    pub compactions: u64,
    pub page_allocations: u64,
    pub page_reuses: u64,
    pub reader_lookups: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Bytes<'a> {
    Borrowed(&'a [u8]),
    Stored(Blob),
    Parts(&'a [Bytes<'a>]),
}

impl Bytes<'_> {
    pub(crate) fn len(self) -> u64 {
        match self {
            Self::Borrowed(bytes) => bytes.len() as u64,
            Self::Stored(blob) => blob.len,
            Self::Parts(parts) => parts
                .iter()
                .fold(0u64, |n, part| n.saturating_add(part.len())),
        }
    }
}

pub(crate) const EMPTY: Bytes<'static> = Bytes::Borrowed(&[]);

#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub a: Blob,
    pub b: Blob,
    pub value: Blob,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tree(pub usize);

#[derive(Clone, Copy, Default)]
pub(crate) struct TreeStats {
    pub entries: u64,
    pub payload_bytes: u64,
    pub page_bytes: u64,
    pub overflow_bytes: u64,
    record_bytes: u64,
}

#[derive(Clone, Copy)]
struct Root {
    page: u64,
    stats: TreeStats,
}

struct Frame {
    file: usize,
    page: u64,
    used: u64,
    dirty: bool,
    node: Node,
}

struct Node {
    bytes: [u8; PAGE],
}

impl Node {
    fn new(leaf: bool, left: u64, next: u64) -> Self {
        let mut node = Self { bytes: [0; PAGE] };
        node.bytes[0] = u8::from(leaf);
        node.bytes[4..6].copy_from_slice(&(HEADER as u16).to_le_bytes());
        node.bytes[8..16].copy_from_slice(&left.to_le_bytes());
        node.bytes[16..24].copy_from_slice(&next.to_le_bytes());
        node
    }

    fn leaf(&self) -> bool {
        self.bytes[0] != 0
    }
    fn len(&self) -> usize {
        u16::from_le_bytes(self.bytes[2..4].try_into().unwrap()) as usize
    }
    fn used(&self) -> usize {
        u16::from_le_bytes(self.bytes[4..6].try_into().unwrap()) as usize
    }
    fn next(&self) -> u64 {
        u64::from_le_bytes(self.bytes[16..24].try_into().unwrap())
    }
    fn start(&self, at: usize) -> usize {
        if at == self.len() {
            return self.used();
        }
        let end = PAGE - at * 2;
        u16::from_le_bytes(self.bytes[end - 2..end].try_into().unwrap()) as usize
    }
    fn record(&self, at: usize) -> &[u8] {
        &self.bytes[self.start(at)..self.start(at + 1)]
    }
    fn child(&self, at: usize) -> u64 {
        let bytes = if at == 0 {
            &self.bytes[8..16]
        } else {
            &self.record(at - 1)[..8]
        };
        u64::from_le_bytes(bytes.try_into().unwrap())
    }
    fn push(&mut self, record: &[u8]) -> io::Result<()> {
        let at = self.used();
        let count = self.len();
        if at + record.len() + (count + 1) * 2 > PAGE {
            return Err(invalid("index page overflow"));
        }
        self.bytes[at..at + record.len()].copy_from_slice(record);
        self.bytes[PAGE - (count + 1) * 2..PAGE - count * 2]
            .copy_from_slice(&(at as u16).to_le_bytes());
        self.bytes[2..4].copy_from_slice(&((count + 1) as u16).to_le_bytes());
        self.bytes[4..6].copy_from_slice(&((at + record.len()) as u16).to_le_bytes());
        Ok(())
    }

    fn splice(&mut self, at: usize, remove: bool, record: &[u8]) {
        let count = self.len();
        let start = self.start(at);
        let end = self.start(at + usize::from(remove));
        let used = self.used();
        let delta = record.len() as isize - (end - start) as isize;
        self.bytes.copy_within(end..used, start + record.len());
        let new_count = count + usize::from(!record.is_empty()) - usize::from(remove);
        if !remove {
            self.bytes
                .copy_within(PAGE - count * 2..PAGE - at * 2, PAGE - (count + 1) * 2);
        } else if record.is_empty() {
            self.bytes.copy_within(
                PAGE - count * 2..PAGE - (at + 1) * 2,
                PAGE - (count - 1) * 2,
            );
        }
        let inserted = usize::from(!record.is_empty());
        for i in at + inserted..new_count {
            let pos = PAGE - (i + 1) * 2;
            let offset = u16::from_le_bytes(self.bytes[pos..pos + 2].try_into().unwrap());
            self.bytes[pos..pos + 2]
                .copy_from_slice(&((offset as isize + delta) as u16).to_le_bytes());
        }
        if inserted != 0 {
            self.bytes[start..start + record.len()].copy_from_slice(record);
            let pos = PAGE - (at + 1) * 2;
            self.bytes[pos..pos + 2].copy_from_slice(&(start as u16).to_le_bytes());
        }
        self.bytes[2..4].copy_from_slice(&(new_count as u16).to_le_bytes());
        self.bytes[4..6].copy_from_slice(&((used as isize + delta) as u16).to_le_bytes());
    }
}

pub(crate) struct Cursor {
    file: usize,
    page: u64,
    at: usize,
}

pub(crate) struct Store {
    files: [File; 2],
    lengths: [u64; 2],
    capacities: [u64; 2],
    active: usize,
    frames: Vec<Frame>,
    roots: Vec<Root>,
    clock: u64,
    garbage: u64,
    free: u64,
    // Pages held in the single-page pool and the extent tree; they don't coalesce, so compaction counts them.
    free_pages: u64,
    extents: Tree,
    stats: StorageStats,
}

impl Store {
    pub fn new(path: &Path, memory: Memory, metadata: usize, trees: usize) -> io::Result<Self> {
        let extents = Tree(trees);
        let trees = trees
            .checked_add(1)
            .ok_or_else(|| invalid("metadata overflow"))?;
        let metadata = metadata
            .checked_add(
                trees
                    .checked_mul(size_of::<Root>())
                    .ok_or_else(|| invalid("metadata overflow"))?,
            )
            .ok_or_else(|| invalid("metadata overflow"))?;
        let count = memory.cache_bytes / size_of::<Frame>();
        let reserved = count
            .checked_mul(size_of::<Frame>())
            .and_then(|bytes| bytes.checked_add(memory.metadata_bytes))
            .and_then(|bytes| bytes.checked_add(TRANSIENT))
            .ok_or_else(|| invalid("memory reservation overflow"))?;
        if count == 0 || reserved > memory.allowance || metadata > memory.metadata_bytes {
            return Err(invalid("index exceeds its admitted working set"));
        }
        std::fs::create_dir_all(path)?;
        let open = |name| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path.join(name))
        };
        let files = [open("indexes.0")?, open("indexes.1")?];
        let mut frames = Vec::new();
        frames.try_reserve_exact(count).map_err(io::Error::other)?;
        frames.resize_with(count, || Frame {
            file: 0,
            page: NONE,
            used: 0,
            dirty: false,
            node: Node { bytes: [0; PAGE] },
        });
        let mut roots = Vec::new();
        roots.try_reserve_exact(trees).map_err(io::Error::other)?;
        roots.resize(
            trees,
            Root {
                page: NONE,
                stats: TreeStats::default(),
            },
        );
        Ok(Self {
            files,
            lengths: [0; 2],
            capacities: [0; 2],
            active: 0,
            frames,
            roots,
            clock: 0,
            garbage: 0,
            free: NONE,
            free_pages: 0,
            extents,
            stats: StorageStats {
                metadata_bytes: metadata,
                resident_bytes: reserved,
                peak_resident_bytes: reserved,
                ..StorageStats::default()
            },
        })
    }

    fn frame(&mut self, file: usize, page: u64) -> io::Result<&mut Frame> {
        self.load_frame(file, page, true)
    }

    fn load_frame(&mut self, file: usize, page: u64, read: bool) -> io::Result<&mut Frame> {
        let sets = self.frames.len().div_ceil(WAYS);
        let set = ((page.wrapping_mul(0x9e3779b97f4a7c15) ^ file as u64) % sets as u64) as usize;
        let first = set * WAYS;
        let end = (first + WAYS).min(self.frames.len());
        self.clock += 1;
        let slot = (first..end)
            .find(|&i| self.frames[i].file == file && self.frames[i].page == page)
            .unwrap_or_else(|| (first..end).min_by_key(|&i| self.frames[i].used).unwrap());
        if self.frames[slot].file != file || self.frames[slot].page != page {
            self.flush_frame(slot)?;
            let frame = &mut self.frames[slot];
            if read {
                self.files[file].seek(SeekFrom::Start(page * PAGE as u64))?;
                self.files[file].read_exact(&mut frame.node.bytes)?;
                self.stats.page_reads += 1;
            }
            frame.file = file;
            frame.page = page;
            frame.dirty = false;
        }
        self.frames[slot].used = self.clock;
        Ok(&mut self.frames[slot])
    }

    fn flush_frame(&mut self, slot: usize) -> io::Result<()> {
        let frame = &mut self.frames[slot];
        if frame.dirty {
            self.files[frame.file].seek(SeekFrom::Start(frame.page * PAGE as u64))?;
            self.files[frame.file].write_all(&frame.node.bytes)?;
            self.stats.page_writes += 1;
            frame.dirty = false;
        }
        Ok(())
    }

    fn allocate(&mut self, bytes: u64) -> io::Result<u64> {
        let pages = bytes.div_ceil(PAGE as u64);
        if pages == 1 && self.free != NONE {
            let page = self.free;
            let frame = self.frame(self.active, page)?;
            let next = u64::from_le_bytes(frame.node.bytes[..8].try_into().unwrap());
            self.free = next;
            self.free_pages -= 1;
            self.stats.page_reuses += 1;
            return Ok(page * PAGE as u64);
        }
        if pages > 1 && self.roots[self.extents.0].stats.entries != 0 {
            let mut cursor = self.cursor(
                self.extents,
                Bytes::Borrowed(&pages.to_be_bytes()),
                EMPTY,
                true,
            )?;
            if let Some(entry) = self.advance(&mut cursor)? {
                let mut count = [0; 8];
                let mut page = [0; 8];
                self.read(entry.a, 0, &mut count)?;
                self.read(entry.b, 0, &mut page)?;
                self.remove(
                    self.extents,
                    Bytes::Borrowed(&count),
                    Bytes::Borrowed(&page),
                )?;
                let count = u64::from_be_bytes(count);
                let page = u64::from_be_bytes(page);
                self.free_pages -= count;
                if count > pages {
                    self.recycle(page + pages, count - pages)?;
                }
                self.stats.page_reuses += pages;
                return Ok(page * PAGE as u64);
            }
        }
        let start = self.lengths[self.active];
        let end = pages
            .checked_mul(PAGE as u64)
            .and_then(|bytes| start.checked_add(bytes))
            .ok_or_else(|| invalid("index backing exhausted"))?;
        if end > self.capacities[self.active] {
            let capacity = end
                .checked_add(TRANSIENT as u64 - 1)
                .ok_or_else(|| invalid("index backing exhausted"))?
                / TRANSIENT as u64
                * TRANSIENT as u64;
            self.files[self.active].set_len(capacity)?;
            self.capacities[self.active] = capacity;
        }
        self.lengths[self.active] = end;
        self.stats.page_allocations += pages;
        self.stats.peak_backing_bytes = self
            .stats
            .peak_backing_bytes
            .max(self.capacities.iter().sum());
        Ok(start)
    }

    fn recycle(&mut self, page: u64, pages: u64) -> io::Result<()> {
        self.free_pages += pages;
        if pages == 1 {
            let next = self.free.to_le_bytes();
            let frame = self.load_frame(self.active, page, false)?;
            frame.node.bytes[..8].copy_from_slice(&next);
            frame.dirty = true;
            self.free = page;
            Ok(())
        } else {
            // Fixed-width extent records use the admitted pager, never overflow allocation.
            self.put(
                self.extents,
                Bytes::Borrowed(&pages.to_be_bytes()),
                Bytes::Borrowed(&page.to_be_bytes()),
                EMPTY,
            )
        }
    }

    pub fn read(&mut self, blob: Blob, offset: u64, output: &mut [u8]) -> io::Result<()> {
        if offset
            .checked_add(output.len() as u64)
            .is_none_or(|end| end > blob.len)
        {
            return Err(invalid("index blob range"));
        }
        let mut done = 0;
        while done < output.len() {
            let at = blob.offset + offset + done as u64;
            let start = (at % PAGE as u64) as usize;
            let count = (PAGE - start).min(output.len() - done);
            output[done..done + count].copy_from_slice(
                &self.frame(blob.file, at / PAGE as u64)?.node.bytes[start..start + count],
            );
            done += count;
        }
        Ok(())
    }

    pub fn copy(&mut self, bytes: Bytes<'_>, out: &mut impl Write) -> io::Result<()> {
        if let Bytes::Borrowed(bytes) = bytes {
            return out.write_all(bytes);
        }
        let mut buffer = [0; PAGE];
        let mut at = 0;
        while at < bytes.len() {
            let count = (bytes.len() - at).min(PAGE as u64) as usize;
            self.chunk(bytes, at, &mut buffer[..count])?;
            out.write_all(&buffer[..count])?;
            at += count as u64;
        }
        Ok(())
    }

    pub(crate) fn chunk(&mut self, bytes: Bytes<'_>, at: u64, out: &mut [u8]) -> io::Result<()> {
        match bytes {
            Bytes::Borrowed(bytes) => {
                out.copy_from_slice(&bytes[at as usize..at as usize + out.len()]);
                Ok(())
            }
            Bytes::Stored(blob) => self.read(blob, at, out),
            Bytes::Parts(parts) => {
                let mut skip = at;
                let mut done = 0;
                for part in parts {
                    if skip >= part.len() {
                        skip -= part.len();
                        continue;
                    }
                    let count = (part.len() - skip).min((out.len() - done) as u64) as usize;
                    self.chunk(*part, skip, &mut out[done..done + count])?;
                    done += count;
                    skip = 0;
                    if done == out.len() {
                        return Ok(());
                    }
                }
                if done == out.len() {
                    Ok(())
                } else {
                    Err(invalid("segmented byte range"))
                }
            }
        }
    }

    pub fn cmp(&mut self, a: Bytes<'_>, b: Bytes<'_>) -> io::Result<Ordering> {
        if let (Bytes::Borrowed(a), Bytes::Borrowed(b)) = (a, b) {
            return Ok(a.cmp(b));
        }
        let mut left = [0; 256];
        let mut right = [0; 256];
        let mut at = 0;
        let len = a.len().min(b.len());
        while at < len {
            let count = (len - at).min(256) as usize;
            self.chunk(a, at, &mut left[..count])?;
            self.chunk(b, at, &mut right[..count])?;
            match left[..count].cmp(&right[..count]) {
                Ordering::Equal => at += count as u64,
                result => return Ok(result),
            }
        }
        Ok(a.len().cmp(&b.len()))
    }

    fn node(&mut self, file: usize, page: u64) -> io::Result<&Node> {
        self.stats.page_decodes += 1;
        Ok(&self.frame(file, page)?.node)
    }

    fn write_node(&mut self, page: u64, node: &Node) -> io::Result<()> {
        let frame = self.load_frame(self.active, page, false)?;
        frame.node.bytes.copy_from_slice(&node.bytes);
        frame.dirty = true;
        Ok(())
    }

    fn new_node(&mut self, root: &mut Root, node: &Node) -> io::Result<u64> {
        let page = self.allocate(PAGE as u64)? / PAGE as u64;
        root.stats.page_bytes += PAGE as u64;
        self.write_node(page, node)?;
        Ok(page)
    }

    fn entry(file: usize, page: u64, node: &Node, at: usize) -> Entry {
        let mut offset = node.start(at) + if node.leaf() { 0 } else { 8 };
        let overflow = node.bytes[offset] != 0;
        offset += 1;
        let a = take_varint(&node.bytes, &mut offset);
        let b = take_varint(&node.bytes, &mut offset);
        let value = take_varint(&node.bytes, &mut offset);
        let offset = if overflow {
            u64::from_le_bytes(node.bytes[offset..offset + 8].try_into().unwrap())
        } else {
            page * PAGE as u64 + offset as u64
        };
        Entry {
            a: Blob {
                file,
                offset,
                len: a,
            },
            b: Blob {
                file,
                offset: offset + a,
                len: b,
            },
            value: Blob {
                file,
                offset: offset + a + b,
                len: value,
            },
        }
    }

    fn compare(&mut self, entry: Entry, a: Bytes<'_>, b: Bytes<'_>) -> io::Result<Ordering> {
        let order = self.cmp(Bytes::Stored(entry.a), a)?;
        if order == Ordering::Equal {
            self.cmp(Bytes::Stored(entry.b), b)
        } else {
            Ok(order)
        }
    }

    fn compare_at(
        &mut self,
        file: usize,
        page: u64,
        at: usize,
        a: Bytes<'_>,
        b: Bytes<'_>,
    ) -> io::Result<Ordering> {
        let node = &self.frame(file, page)?.node;
        let record = node.record(at);
        let mut offset = if node.leaf() { 0 } else { 8 };
        if record[offset] == 0
            && let (Bytes::Borrowed(a), Bytes::Borrowed(b)) = (a, b)
        {
            offset += 1;
            let la = take_varint(record, &mut offset) as usize;
            let lb = take_varint(record, &mut offset) as usize;
            let _ = take_varint(record, &mut offset);
            return Ok(record[offset..offset + la]
                .cmp(a)
                .then_with(|| record[offset + la..offset + la + lb].cmp(b)));
        }
        let entry = Self::entry(file, page, node, at);
        self.compare(entry, a, b)
    }

    fn position(
        &mut self,
        file: usize,
        page: u64,
        a: Bytes<'_>,
        b: Bytes<'_>,
        upper: bool,
    ) -> io::Result<usize> {
        let mut low = 0;
        let mut high = self.frame(file, page)?.node.len();
        while low < high {
            let mid = (low + high) / 2;
            let order = self.compare_at(file, page, mid, a, b)?;
            if order == Ordering::Less || (upper && order == Ordering::Equal) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    pub fn get(&mut self, tree: Tree, a: Bytes<'_>, b: Bytes<'_>) -> io::Result<Option<Entry>> {
        self.stats.tree_operations += 1;
        let file = self.active;
        let mut page = self.roots[tree.0].page;
        while page != NONE {
            let leaf = self.node(self.active, page)?.leaf();
            let at = self.position(self.active, page, a, b, !leaf)?;
            let node = &self.frame(self.active, page)?.node;
            if leaf {
                if at == node.len() {
                    return Ok(None);
                }
                let entry = Self::entry(file, page, node, at);
                return if self.compare_at(self.active, page, at, a, b)? == Ordering::Equal {
                    Ok(Some(entry))
                } else {
                    Ok(None)
                };
            }
            page = node.child(at);
        }
        Ok(None)
    }

    pub fn cursor(
        &mut self,
        tree: Tree,
        a: Bytes<'_>,
        b: Bytes<'_>,
        inclusive: bool,
    ) -> io::Result<Cursor> {
        self.stats.tree_operations += 1;
        self.cursor_root(self.roots[tree.0], self.active, a, b, inclusive)
    }

    fn cursor_root(
        &mut self,
        root: Root,
        file: usize,
        a: Bytes<'_>,
        b: Bytes<'_>,
        inclusive: bool,
    ) -> io::Result<Cursor> {
        let mut page = root.page;
        while page != NONE {
            let leaf = self.node(file, page)?.leaf();
            let at = self.position(file, page, a, b, !leaf || !inclusive)?;
            if leaf {
                return Ok(Cursor { file, page, at });
            }
            page = self.frame(file, page)?.node.child(at);
        }
        Ok(Cursor {
            file,
            page: NONE,
            at: 0,
        })
    }

    pub fn advance(&mut self, cursor: &mut Cursor) -> io::Result<Option<Entry>> {
        while cursor.page != NONE {
            let node = if cursor.at == 0 {
                self.node(cursor.file, cursor.page)?
            } else {
                &self.frame(cursor.file, cursor.page)?.node
            };
            if cursor.at < node.len() {
                let entry = Self::entry(cursor.file, cursor.page, node, cursor.at);
                cursor.at += 1;
                return Ok(Some(entry));
            }
            cursor.page = node.next();
            cursor.at = 0;
        }
        Ok(None)
    }

    pub fn next(
        &mut self,
        tree: Tree,
        a: Bytes<'_>,
        b: Bytes<'_>,
        inclusive: bool,
    ) -> io::Result<Option<Entry>> {
        let mut cursor = self.cursor(tree, a, b, inclusive)?;
        self.advance(&mut cursor)
    }

    fn encode(
        &mut self,
        a: Bytes<'_>,
        b: Bytes<'_>,
        value: Bytes<'_>,
        out: &mut [u8; INLINE + 48],
    ) -> io::Result<(usize, u64)> {
        let len = a
            .len()
            .checked_add(b.len())
            .and_then(|n| n.checked_add(value.len()))
            .ok_or_else(|| invalid("edge size overflow"))?;
        let overflow = len > INLINE as u64;
        out[0] = u8::from(overflow);
        let mut at = 1;
        for size in [a.len(), b.len(), value.len()] {
            put_varint(out, &mut at, size);
        }
        if overflow {
            let start = self.allocate(len)?;
            out[at..at + 8].copy_from_slice(&start.to_le_bytes());
            at += 8;
            let mut offset = start;
            let mut buffer = [0; PAGE];
            for bytes in [a, b, value] {
                let mut done = 0;
                while done < bytes.len() {
                    let count = (bytes.len() - done).min(PAGE as u64) as usize;
                    self.chunk(bytes, done, &mut buffer[..count])?;
                    let mut written = 0;
                    while written < count {
                        let pos = ((offset + written as u64) % PAGE as u64) as usize;
                        let n = (PAGE - pos).min(count - written);
                        let frame = self.load_frame(
                            self.active,
                            (offset + written as u64) / PAGE as u64,
                            pos != 0,
                        )?;
                        frame.node.bytes[pos..pos + n]
                            .copy_from_slice(&buffer[written..written + n]);
                        frame.dirty = true;
                        written += n;
                    }
                    offset += count as u64;
                    done += count as u64;
                }
            }
            Ok((at, len.div_ceil(PAGE as u64) * PAGE as u64))
        } else {
            for bytes in [a, b, value] {
                let size = bytes.len() as usize;
                self.chunk(bytes, 0, &mut out[at..at + size])?;
                at += size;
            }
            Ok((at, 0))
        }
    }

    pub fn put(
        &mut self,
        tree: Tree,
        a: Bytes<'_>,
        b: Bytes<'_>,
        value: Bytes<'_>,
    ) -> io::Result<()> {
        self.stats.tree_operations += 1;
        let mut root = self.roots[tree.0];
        self.insert(&mut root, a, b, value)?;
        self.replace_root(tree, root);
        Ok(())
    }

    fn replace_root(&mut self, tree: Tree, root: Root) {
        let previous = self.roots[tree.0].stats;
        self.stats.live_bytes =
            self.stats.live_bytes - previous.page_bytes - previous.overflow_bytes
                + root.stats.page_bytes
                + root.stats.overflow_bytes;
        self.stats.peak_live_bytes = self.stats.peak_live_bytes.max(self.stats.live_bytes);
        self.roots[tree.0] = root;
    }

    fn insert(
        &mut self,
        root: &mut Root,
        a: Bytes<'_>,
        b: Bytes<'_>,
        value: Bytes<'_>,
    ) -> io::Result<()> {
        let mut path = [(NONE, 0usize); 64];
        let mut depth = 0;
        if root.page == NONE {
            root.page = self.new_node(root, &Node::new(true, NONE, NONE))?;
        }
        let mut page = root.page;
        let mut at = loop {
            let leaf = self.node(self.active, page)?.leaf();
            let at = self.position(self.active, page, a, b, !leaf)?;
            if leaf {
                break at;
            }
            path[depth] = (page, at);
            depth += 1;
            page = self.frame(self.active, page)?.node.child(at);
        };
        let node = &self.frame(self.active, page)?.node;
        let replacing =
            at < node.len() && self.compare_at(self.active, page, at, a, b)? == Ordering::Equal;
        if replacing {
            let old = Self::entry(self.active, page, &self.frame(self.active, page)?.node, at);
            let payload = old.a.len + old.b.len + old.value.len;
            root.stats.payload_bytes -= payload;
            let overflow = if payload > INLINE as u64 {
                payload.div_ceil(PAGE as u64) * PAGE as u64
            } else {
                0
            };
            root.stats.overflow_bytes -= overflow;
            self.garbage += overflow;
            root.stats.record_bytes -=
                self.frame(self.active, page)?.node.record(at).len() as u64 + 2;
        } else {
            root.stats.entries += 1;
        }
        let mut encoded = [0; INLINE + 48];
        let (mut size, overflow) = self.encode(a, b, value, &mut encoded)?;
        root.stats.payload_bytes += a.len() + b.len() + value.len();
        root.stats.overflow_bytes += overflow;
        root.stats.record_bytes += size as u64 + 2;
        let mut replacing = replacing;
        loop {
            let node = &self.frame(self.active, page)?.node;
            let count = node.len() + usize::from(!replacing);
            let total = node.used() - HEADER + size + count * 2
                - if replacing { node.record(at).len() } else { 0 };
            if total + HEADER <= PAGE {
                let frame = self.frame(self.active, page)?;
                frame.node.splice(at, replacing, &encoded[..size]);
                frame.dirty = true;
                return Ok(());
            }
            let node = Node {
                bytes: self.frame(self.active, page)?.node.bytes,
            };
            let record = |i: usize| -> &[u8] {
                if i == at {
                    &encoded[..size]
                } else {
                    node.record(if i < at {
                        i
                    } else {
                        i - usize::from(!replacing)
                    })
                }
            };
            let mut mid = 0;
            let mut used = 0;
            while mid + 1 < count && used + record(mid).len() + 2 <= total / 2 {
                used += record(mid).len() + 2;
                mid += 1;
            }
            mid = mid.max(1);
            let mut separator = [0; INLINE + 48];
            let sep = record(mid);
            let skip = if node.leaf() { 0 } else { 8 };
            let mut pos = skip + 1;
            let la = take_varint(sep, &mut pos);
            let lb = take_varint(sep, &mut pos);
            let _ = take_varint(sep, &mut pos);
            separator[8] = sep[skip];
            let mut sep_len = 9;
            put_varint(&mut separator, &mut sep_len, la);
            put_varint(&mut separator, &mut sep_len, lb);
            put_varint(&mut separator, &mut sep_len, 0);
            let key_len = if sep[skip] != 0 {
                8
            } else {
                (la + lb) as usize
            };
            separator[sep_len..sep_len + key_len].copy_from_slice(&sep[pos..pos + key_len]);
            sep_len += key_len;
            let right_left = if node.leaf() {
                NONE
            } else {
                u64::from_le_bytes(sep[..8].try_into().unwrap())
            };
            let mut right = Node::new(node.leaf(), right_left, node.next());
            for i in mid + usize::from(!node.leaf())..count {
                right.push(record(i))?;
            }
            let right_page = self.new_node(root, &right)?;
            let mut left = Node::new(
                node.leaf(),
                node.child(0),
                if node.leaf() { right_page } else { NONE },
            );
            for i in 0..mid {
                left.push(record(i))?;
            }
            self.write_node(page, &left)?;
            separator[..8].copy_from_slice(&right_page.to_le_bytes());
            if depth == 0 {
                let mut parent = Node::new(false, page, NONE);
                parent.push(&separator[..sep_len])?;
                root.page = self.new_node(root, &parent)?;
                return Ok(());
            }
            depth -= 1;
            (page, at) = path[depth];
            encoded = separator;
            size = sep_len;
            replacing = false;
        }
    }

    pub fn remove(&mut self, tree: Tree, a: Bytes<'_>, b: Bytes<'_>) -> io::Result<()> {
        self.stats.tree_operations += 1;
        let previous = self.roots[tree.0];
        let result = self.remove_inner(tree, a, b);
        let root = self.roots[tree.0];
        self.roots[tree.0] = previous;
        self.replace_root(tree, root);
        result
    }

    fn remove_inner(&mut self, tree: Tree, a: Bytes<'_>, b: Bytes<'_>) -> io::Result<()> {
        let file = self.active;
        let mut page = self.roots[tree.0].page;
        let mut path = [(NONE, 0usize); 64];
        let mut depth = 0;
        while page != NONE {
            let leaf = self.node(self.active, page)?.leaf();
            let at = self.position(self.active, page, a, b, !leaf)?;
            let node = &self.frame(self.active, page)?.node;
            if !leaf {
                path[depth] = (page, at);
                depth += 1;
                page = node.child(at);
                continue;
            }
            if at == node.len() {
                return Ok(());
            }
            let entry = Self::entry(file, page, node, at);
            if self.compare_at(self.active, page, at, a, b)? != Ordering::Equal {
                return Ok(());
            }
            let payload = entry.a.len + entry.b.len + entry.value.len;
            let overflow = if payload > INLINE as u64 {
                payload.div_ceil(PAGE as u64) * PAGE as u64
            } else {
                0
            };
            let record_bytes = self.frame(self.active, page)?.node.record(at).len() as u64 + 2;
            let stats = &mut self.roots[tree.0].stats;
            stats.entries -= 1;
            stats.payload_bytes -= payload;
            stats.overflow_bytes -= overflow;
            stats.record_bytes -= record_bytes;
            // Separators can still reference a removed overflow key until compaction.
            self.garbage += overflow;
            let frame = self.frame(self.active, page)?;
            frame.node.splice(at, true, &[]);
            frame.dirty = true;
            if frame.node.len() != 0 {
                return Ok(());
            }
            let next = frame.node.next();
            for &(ancestor, child) in path[..depth].iter().rev() {
                if child == 0 {
                    continue;
                }
                let mut previous = self.frame(self.active, ancestor)?.node.child(child - 1);
                loop {
                    let node = self.node(self.active, previous)?;
                    if node.leaf() {
                        break;
                    }
                    previous = node.child(node.len());
                }
                let frame = self.frame(self.active, previous)?;
                frame.node.bytes[16..24].copy_from_slice(&next.to_le_bytes());
                frame.dirty = true;
                break;
            }
            loop {
                self.recycle(page, 1)?;
                self.roots[tree.0].stats.page_bytes -= PAGE as u64;
                if depth == 0 {
                    self.roots[tree.0].page = NONE;
                    return Ok(());
                }
                depth -= 1;
                let (parent, child) = path[depth];
                let frame = self.frame(self.active, parent)?;
                if frame.node.len() == 0 {
                    page = parent;
                    continue;
                }
                if child == 0 {
                    let left = frame.node.child(1);
                    frame.node.bytes[8..16].copy_from_slice(&left.to_le_bytes());
                }
                frame.node.splice(child.saturating_sub(1), true, &[]);
                frame.dirty = true;
                if depth == 0 && frame.node.len() == 0 {
                    let left = frame.node.child(0);
                    self.roots[tree.0].page = left;
                    self.roots[tree.0].stats.page_bytes -= PAGE as u64;
                    self.recycle(parent, 1)?;
                }
                return Ok(());
            }
        }
        Ok(())
    }

    pub fn compact_if_needed(&mut self) -> io::Result<bool> {
        let occupied: u64 = self
            .roots
            .iter()
            .map(|r| r.stats.page_bytes + r.stats.overflow_bytes)
            .sum();
        let packed: u64 = self
            .roots
            .iter()
            .map(|r| {
                r.stats.record_bytes.div_ceil((PAGE - HEADER) as u64) * PAGE as u64
                    + r.stats.overflow_bytes
            })
            .sum();
        if occupied > packed.saturating_mul(2)
            || self.garbage > occupied.max(PAGE as u64)
            || self.free_pages.saturating_mul(PAGE as u64) > occupied.max(TRANSIENT as u64)
        {
            self.compact()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn clear_page(&mut self, page: u64) -> io::Result<()> {
        let leaf = self.node(self.active, page)?.leaf();
        let count = self.frame(self.active, page)?.node.len();
        if leaf {
            for at in 0..count {
                let entry =
                    Self::entry(self.active, page, &self.frame(self.active, page)?.node, at);
                let len = entry.a.len + entry.b.len + entry.value.len;
                if len > INLINE as u64 {
                    self.recycle(entry.a.offset / PAGE as u64, len.div_ceil(PAGE as u64))?;
                }
            }
        } else {
            for at in 0..=count {
                let child = self.frame(self.active, page)?.node.child(at);
                self.clear_page(child)?;
            }
        }
        self.recycle(page, 1)
    }

    pub fn clear(&mut self, tree: Tree) -> io::Result<()> {
        let old = self.roots[tree.0];
        if old.page != NONE {
            self.clear_page(old.page)?;
        }
        self.replace_root(
            tree,
            Root {
                page: NONE,
                stats: TreeStats::default(),
            },
        );
        Ok(())
    }

    fn separator(
        &mut self,
        file: usize,
        mut child: u64,
        right: u64,
        out: &mut [u8; INLINE + 48],
    ) -> io::Result<usize> {
        loop {
            let node = self.node(file, child)?;
            if !node.leaf() {
                child = node.child(0);
                continue;
            }
            let record = node.record(0);
            let mut at = 1;
            let la = take_varint(record, &mut at);
            let lb = take_varint(record, &mut at);
            let _ = take_varint(record, &mut at);
            out[..8].copy_from_slice(&right.to_le_bytes());
            out[8] = record[0];
            let mut len = 9;
            put_varint(out, &mut len, la);
            put_varint(out, &mut len, lb);
            put_varint(out, &mut len, 0);
            let bytes = if record[0] == 0 {
                (la + lb) as usize
            } else {
                8
            };
            out[len..len + bytes].copy_from_slice(&record[at..at + bytes]);
            return Ok(len + bytes);
        }
    }

    fn bulk_load(&mut self, old: Root, source: usize) -> io::Result<Root> {
        let mut root = Root {
            page: NONE,
            stats: TreeStats::default(),
        };
        if old.stats.entries == 0 {
            return Ok(root);
        }
        self.stats.tree_operations += 1;
        let mut cursor = self.cursor_root(old, source, EMPTY, EMPTY, true)?;
        let mut node = Node::new(true, NONE, NONE);
        let mut page = self.new_node(&mut root, &node)?;
        let mut first = page;
        let mut count = 1u64;
        let mut encoded = [0; INLINE + 48];
        while let Some(entry) = self.advance(&mut cursor)? {
            let (size, overflow) = self.encode(
                Bytes::Stored(entry.a),
                Bytes::Stored(entry.b),
                Bytes::Stored(entry.value),
                &mut encoded,
            )?;
            if node.used() + size + (node.len() + 1) * 2 > PAGE {
                let next = self.new_node(&mut root, &Node::new(true, NONE, NONE))?;
                node.bytes[16..24].copy_from_slice(&next.to_le_bytes());
                self.write_node(page, &node)?;
                page = next;
                node = Node::new(true, NONE, NONE);
                count += 1;
            }
            node.push(&encoded[..size])?;
            root.stats.entries += 1;
            root.stats.payload_bytes += entry.a.len + entry.b.len + entry.value.len;
            root.stats.overflow_bytes += overflow;
            root.stats.record_bytes += size as u64 + 2;
        }
        self.write_node(page, &node)?;
        while count > 1 {
            let mut child = first;
            let mut parent = Node::new(false, child, NONE);
            let mut parent_page = self.new_node(&mut root, &parent)?;
            let parent_first = parent_page;
            let mut parents = 1;
            child = self.frame(self.active, child)?.node.next();
            while child != NONE {
                let next = self.frame(self.active, child)?.node.next();
                let size = self.separator(self.active, child, child, &mut encoded)?;
                if parent.used() + size + (parent.len() + 1) * 2 > PAGE {
                    let new = Node::new(false, child, NONE);
                    let next_parent = self.new_node(&mut root, &new)?;
                    parent.bytes[16..24].copy_from_slice(&next_parent.to_le_bytes());
                    self.write_node(parent_page, &parent)?;
                    parent = new;
                    parent_page = next_parent;
                    parents += 1;
                } else {
                    parent.push(&encoded[..size])?;
                }
                child = next;
            }
            self.write_node(parent_page, &parent)?;
            first = parent_first;
            count = parents;
        }
        root.page = first;
        Ok(root)
    }

    pub fn compact(&mut self) -> io::Result<()> {
        self.clear(self.extents)?;
        let source = self.active;
        self.active = 1 - source;
        self.free = NONE;
        self.free_pages = 0;
        for i in 0..self.roots.len() {
            let root = self.bulk_load(self.roots[i], source)?;
            self.replace_root(Tree(i), root);
        }
        for frame in &mut self.frames {
            if frame.file == source {
                frame.page = NONE;
                frame.used = 0;
                frame.dirty = false;
            }
        }
        self.files[source].set_len(0)?;
        self.lengths[source] = 0;
        self.capacities[source] = 0;
        self.files[self.active].set_len(self.lengths[self.active])?;
        self.capacities[self.active] = self.lengths[self.active];
        self.garbage = 0;
        self.stats.compactions += 1;
        Ok(())
    }

    pub fn tree_stats(&self, tree: Tree) -> TreeStats {
        self.roots[tree.0].stats
    }
    pub fn reader_lookup(&mut self) {
        self.stats.reader_lookups += 1;
    }
    pub fn stats(&self) -> StorageStats {
        StorageStats {
            backing_bytes: self.capacities.iter().sum(),
            ..self.stats
        }
    }
}

pub(crate) fn put_varint(out: &mut [u8], at: &mut usize, mut value: u64) {
    while value >= 128 {
        out[*at] = (value as u8 & 127) | 128;
        *at += 1;
        value >>= 7;
    }
    out[*at] = value as u8;
    *at += 1;
}

pub(crate) fn take_varint(input: &[u8], at: &mut usize) -> u64 {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = input[*at];
        *at += 1;
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return value;
        }
        shift += 7;
    }
}
