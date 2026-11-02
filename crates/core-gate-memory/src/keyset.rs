use crate::manifest::invalid;
use std::{
    cell::RefCell,
    collections::hash_map::RandomState,
    fs::{File, OpenOptions},
    hash::BuildHasher,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    rc::Rc,
};

const PAGE: usize = 4096;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct CacheSize {
    pub owner: String,
    pub allowance_bytes: usize,
    pub peak_bytes: usize,
}

pub struct Cache {
    pool: Rc<RefCell<Pool>>,
    size: CacheSize,
}

impl Cache {
    pub fn reserved(owner: &str, allowance_bytes: usize) -> io::Result<Self> {
        let pool = Pool::new(allowance_bytes)?;
        let peak_bytes = pool.bytes();
        Ok(Self {
            pool: Rc::new(RefCell::new(pool)),
            size: CacheSize {
                owner: owner.into(),
                allowance_bytes,
                peak_bytes,
            },
        })
    }

    pub fn size(&self) -> CacheSize {
        self.size.clone()
    }
}
const NODE: usize = 64;
pub type Key = (u128, u128);

#[derive(Clone, Copy, Default)]
struct Node {
    key: Key,
    value: u64,
    children: [u64; 2],
    bit: u16,
}

struct Page {
    key: Option<(u64, u64)>,
    file: Option<Rc<RefCell<File>>>,
    dirty: bool,
    recent: bool,
    bytes: [u8; PAGE],
}

type Slot = Option<((u64, u64), usize)>;

struct Slots {
    entries: Vec<Slot>,
    hash: RandomState,
}

impl Slots {
    fn count(pages: usize) -> usize {
        (pages * 2).next_power_of_two()
    }

    fn new(pages: usize) -> io::Result<Self> {
        let count = Self::count(pages);
        let mut entries = Vec::new();
        entries.try_reserve_exact(count).map_err(io::Error::other)?;
        entries.resize(count, None);
        Ok(Self {
            entries,
            hash: RandomState::new(),
        })
    }

    fn home(&self, key: (u64, u64)) -> usize {
        self.hash.hash_one(key) as usize & (self.entries.len() - 1)
    }

    fn get(&self, key: &(u64, u64)) -> Option<usize> {
        let mut at = self.home(*key);
        while let Some((found, value)) = self.entries[at] {
            if found == *key {
                return Some(value);
            }
            at = (at + 1) & (self.entries.len() - 1);
        }
        None
    }

    fn insert(&mut self, key: (u64, u64), value: usize) {
        let mut at = self.home(key);
        while let Some((found, _)) = self.entries[at] {
            if found == key {
                break;
            }
            at = (at + 1) & (self.entries.len() - 1);
        }
        self.entries[at] = Some((key, value));
    }

    fn remove(&mut self, key: &(u64, u64)) {
        let mask = self.entries.len() - 1;
        let mut hole = self.home(*key);
        while let Some((found, _)) = self.entries[hole] {
            if found == *key {
                break;
            }
            hole = (hole + 1) & mask;
        }
        if self.entries[hole].is_none() {
            return;
        }
        let mut next = (hole + 1) & mask;
        while let Some((key, value)) = self.entries[next] {
            let home = self.home(key);
            if (next.wrapping_sub(home) & mask) >= (next.wrapping_sub(hole) & mask) {
                self.entries[hole] = Some((key, value));
                hole = next;
            }
            next = (next + 1) & mask;
        }
        self.entries[hole] = None;
    }
}

struct Pool {
    pages: Vec<Page>,
    slots: Slots,
    hand: usize,
    next_owner: u64,
}

impl Pool {
    fn fixed_bytes() -> usize {
        std::mem::size_of::<RefCell<Pool>>() + 2 * std::mem::size_of::<usize>()
    }

    fn new(budget: usize) -> io::Result<Self> {
        let available = budget.saturating_sub(Self::fixed_bytes());
        let mut low = 0;
        let mut high = available / std::mem::size_of::<Page>();
        while low < high {
            let count = low + (high - low).div_ceil(2);
            let bytes = count * std::mem::size_of::<Page>()
                + Slots::count(count) * std::mem::size_of::<Slot>();
            if bytes <= available {
                low = count;
            } else {
                high = count - 1;
            }
        }
        let count = low;
        if count == 0 {
            return Err(invalid("Index page budget cannot hold one page"));
        }
        let slots = Slots::new(count)?;
        let mut pages = Vec::new();
        pages.try_reserve_exact(count).map_err(io::Error::other)?;
        if Self::fixed_bytes()
            + slots.entries.capacity() * std::mem::size_of::<Slot>()
            + pages.capacity() * std::mem::size_of::<Page>()
            > budget
        {
            return Err(invalid(
                "Index cache allocation exceeds its reserved budget",
            ));
        }
        pages.extend((0..count).map(|_| Page {
            key: None,
            file: None,
            dirty: false,
            recent: false,
            bytes: [0; PAGE],
        }));
        Ok(Self {
            pages,
            slots,
            hand: 0,
            next_owner: 0,
        })
    }

    fn bytes(&self) -> usize {
        self.pages.capacity() * std::mem::size_of::<Page>()
            + self.slots.entries.capacity() * std::mem::size_of::<Slot>()
            + Self::fixed_bytes()
    }

    fn page(&mut self, owner: u64, number: u64, file: &Rc<RefCell<File>>) -> io::Result<&mut Page> {
        let key = (owner, number);
        let slot = if let Some(slot) = self.slots.get(&key) {
            slot
        } else {
            let slot = loop {
                let slot = self.hand;
                self.hand = (self.hand + 1) % self.pages.len();
                let page = &mut self.pages[slot];
                if !page.recent {
                    break slot;
                }
                page.recent = false;
            };
            let page = &mut self.pages[slot];
            if let Some(old) = page.key {
                if page.dirty {
                    let mut old_file = page.file.as_ref().unwrap().borrow_mut();
                    old_file.seek(SeekFrom::Start(old.1 * PAGE as u64))?;
                    old_file.write_all(&page.bytes)?;
                }
                self.slots.remove(&old);
            }
            page.bytes.fill(0);
            let mut source = file.borrow_mut();
            source.seek(SeekFrom::Start(number * PAGE as u64))?;
            let mut filled = 0;
            while filled < PAGE {
                let read = source.read(&mut page.bytes[filled..])?;
                if read == 0 {
                    break;
                }
                filled += read;
            }
            page.key = Some(key);
            page.file = Some(Rc::clone(file));
            page.dirty = false;
            self.slots.insert(key, slot);
            slot
        };
        self.pages[slot].recent = true;
        Ok(&mut self.pages[slot])
    }
}

// All indexes on this reader share a budgeted, fully associative clock cache.
pub struct Index {
    file: Rc<RefCell<File>>,
    pool: Rc<RefCell<Pool>>,
    owner: u64,
    root: u64,
    allocated: u64,
    free: u64,
}

fn difference(a: Key, b: Key) -> u16 {
    if a.0 != b.0 {
        (a.0 ^ b.0).leading_zeros() as u16
    } else {
        128 + (a.1 ^ b.1).leading_zeros() as u16
    }
}

fn branch(key: Key, bit: u16) -> usize {
    if bit < 128 {
        ((key.0 >> (127 - bit)) & 1) as usize
    } else {
        ((key.1 >> (255 - bit)) & 1) as usize
    }
}

pub fn after(key: Key) -> Option<Key> {
    if let Some(next) = key.1.checked_add(1) {
        Some((key.0, next))
    } else {
        key.0.checked_add(1).map(|next| (next, 0))
    }
}

impl Index {
    pub fn new(path: &Path, cache: &Cache) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        let pool = Rc::clone(&cache.pool);
        let owner = {
            let mut pool = pool.borrow_mut();
            pool.next_owner = pool
                .next_owner
                .checked_add(1)
                .ok_or_else(|| invalid("Index owner overflow"))?;
            pool.next_owner
        };
        Ok(Self {
            file: Rc::new(RefCell::new(file)),
            pool,
            owner,
            root: 0,
            allocated: 0,
            free: 0,
        })
    }

    fn read(&self, at: u64) -> io::Result<Node> {
        let offset = (at - 1)
            .checked_mul(NODE as u64)
            .ok_or_else(|| invalid("Index offset overflow"))?;
        let mut pool = self.pool.borrow_mut();
        let page = pool.page(self.owner, offset / PAGE as u64, &self.file)?;
        let bytes = &page.bytes[offset as usize % PAGE..][..NODE];
        Ok(Node {
            key: (
                u128::from_le_bytes(bytes[0..16].try_into().unwrap()),
                u128::from_le_bytes(bytes[16..32].try_into().unwrap()),
            ),
            value: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            children: [
                u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
                u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            ],
            bit: u16::from_le_bytes(bytes[56..58].try_into().unwrap()),
        })
    }

    fn write(&self, at: u64, node: Node) -> io::Result<()> {
        let offset = (at - 1)
            .checked_mul(NODE as u64)
            .ok_or_else(|| invalid("Index offset overflow"))?;
        let mut pool = self.pool.borrow_mut();
        let page = pool.page(self.owner, offset / PAGE as u64, &self.file)?;
        let bytes = &mut page.bytes[offset as usize % PAGE..][..NODE];
        bytes[0..16].copy_from_slice(&node.key.0.to_le_bytes());
        bytes[16..32].copy_from_slice(&node.key.1.to_le_bytes());
        bytes[32..40].copy_from_slice(&node.value.to_le_bytes());
        bytes[40..48].copy_from_slice(&node.children[0].to_le_bytes());
        bytes[48..56].copy_from_slice(&node.children[1].to_le_bytes());
        bytes[56..58].copy_from_slice(&node.bit.to_le_bytes());
        page.dirty = true;
        Ok(())
    }

    fn allocate(&mut self, node: Node) -> io::Result<u64> {
        let at = if self.free != 0 {
            let at = self.free;
            self.free = self.read(at)?.children[0];
            at
        } else {
            self.allocated = self
                .allocated
                .checked_add(1)
                .ok_or_else(|| invalid("Index size overflow"))?;
            self.allocated
        };
        self.write(at, node)?;
        Ok(at)
    }

    fn release(&mut self, at: u64) -> io::Result<()> {
        self.write(
            at,
            Node {
                children: [self.free, 0],
                ..Node::default()
            },
        )?;
        self.free = at;
        Ok(())
    }

    pub fn clear(&mut self) -> io::Result<()> {
        self.root = 0;
        self.allocated = 0;
        self.free = 0;
        Ok(())
    }

    pub fn get(&self, key: Key) -> io::Result<Option<u64>> {
        let mut at = self.root;
        while at != 0 {
            let node = self.read(at)?;
            if node.bit == 256 {
                return Ok((node.key == key).then_some(node.value));
            }
            at = node.children[branch(key, node.bit)];
        }
        Ok(None)
    }

    pub fn put(&mut self, key: Key, value: u64) -> io::Result<()> {
        if self.root == 0 {
            self.root = self.allocate(Node {
                key,
                value,
                bit: 256,
                ..Node::default()
            })?;
            return Ok(());
        }
        let mut at = self.root;
        let leaf = loop {
            let node = self.read(at)?;
            if node.bit == 256 {
                break node;
            }
            at = node.children[branch(key, node.bit)];
        };
        let bit = difference(key, leaf.key);
        if bit == 256 {
            return self.write(at, Node { value, ..leaf });
        }
        let (mut at, mut parent) = (self.root, None);
        loop {
            let node = self.read(at)?;
            if node.bit >= bit {
                break;
            }
            let side = branch(key, node.bit);
            parent = Some((at, node, side));
            at = node.children[side];
        }
        let new = self.allocate(Node {
            key,
            value,
            bit: 256,
            ..Node::default()
        })?;
        let side = branch(key, bit);
        let mut children = [at; 2];
        children[side] = new;
        let split = self.allocate(Node {
            key,
            bit,
            children,
            value: 0,
        })?;
        if let Some((at, mut node, side)) = parent {
            node.children[side] = split;
            self.write(at, node)?;
        } else {
            self.root = split;
        }
        Ok(())
    }

    pub fn remove(&mut self, key: Key) -> io::Result<()> {
        let mut at = self.root;
        let mut parent: Option<(u64, Node, usize)> = None;
        let mut grand: Option<(u64, Node, usize)> = None;
        while at != 0 {
            let node = self.read(at)?;
            if node.bit == 256 {
                if node.key != key {
                    return Ok(());
                }
                if let Some((parent_at, parent_node, side)) = parent {
                    let sibling = parent_node.children[1 - side];
                    if let Some((grand_at, mut grand_node, grand_side)) = grand {
                        grand_node.children[grand_side] = sibling;
                        self.write(grand_at, grand_node)?;
                    } else {
                        self.root = sibling;
                    }
                    self.release(parent_at)?;
                } else {
                    self.root = 0;
                }
                return self.release(at);
            }
            let side = branch(key, node.bit);
            grand = parent;
            parent = Some((at, node, side));
            at = node.children[side];
        }
        Ok(())
    }

    fn minimum(&self, mut at: u64) -> io::Result<(Key, u64)> {
        loop {
            let node = self.read(at)?;
            if node.bit == 256 {
                return Ok((node.key, node.value));
            }
            at = node.children[0];
        }
    }

    fn lower(&self, at: u64, key: Key) -> io::Result<Option<(Key, u64)>> {
        if at == 0 {
            return Ok(None);
        }
        let node = self.read(at)?;
        if difference(key, node.key) < node.bit {
            return if node.key > key {
                self.minimum(at).map(Some)
            } else {
                Ok(None)
            };
        }
        if node.bit == 256 {
            return Ok(Some((node.key, node.value)));
        }
        let side = branch(key, node.bit);
        if let Some(found) = self.lower(node.children[side], key)? {
            return Ok(Some(found));
        }
        if side == 0 {
            self.minimum(node.children[1]).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn first(&self, key: Key) -> io::Result<Option<(Key, u64)>> {
        self.lower(self.root, key)
    }

    fn maximum(&self, mut at: u64) -> io::Result<(Key, u64)> {
        loop {
            let node = self.read(at)?;
            if node.bit == 256 {
                return Ok((node.key, node.value));
            }
            at = node.children[1];
        }
    }

    fn upper(&self, at: u64, key: Key) -> io::Result<Option<(Key, u64)>> {
        if at == 0 {
            return Ok(None);
        }
        let node = self.read(at)?;
        if difference(key, node.key) < node.bit {
            return if node.key < key {
                self.maximum(at).map(Some)
            } else {
                Ok(None)
            };
        }
        if node.bit == 256 {
            return Ok(Some((node.key, node.value)));
        }
        let side = branch(key, node.bit);
        if let Some(found) = self.upper(node.children[side], key)? {
            return Ok(Some(found));
        }
        if side == 1 {
            self.maximum(node.children[0]).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn last(&self, key: Key) -> io::Result<Option<(Key, u64)>> {
        self.upper(self.root, key)
    }
}

pub struct Keys {
    counts: Index,
    before: Index,
    retained: bool,
}

impl Keys {
    pub fn new(path: &Path, retained: bool, cache: &Cache) -> io::Result<Self> {
        let file = path.file_name().unwrap().to_string_lossy();
        Ok(Self {
            counts: Index::new(&path.with_file_name(format!("{file}.counts")), cache)?,
            before: Index::new(&path.with_file_name(format!("{file}.before")), cache)?,
            retained,
        })
    }

    pub fn begin(&mut self) -> io::Result<()> {
        self.before.clear()
    }
    pub fn contains(&self, key: u128) -> io::Result<bool> {
        Ok(self.counts.get((key, 0))?.unwrap_or(0) != 0)
    }
    pub fn contained(&self, key: u128) -> io::Result<bool> {
        match self.before.get((key, 0))? {
            Some(count) => Ok(count != 0),
            None => self.contains(key),
        }
    }

    pub fn adjust(&mut self, key: u128, add: bool) -> io::Result<()> {
        let old = self.counts.get((key, 0))?.unwrap_or(0);
        let count = if self.retained {
            if add { 1 } else { old }
        } else if add {
            old.checked_add(1)
                .ok_or_else(|| invalid("Reference count overflow"))?
        } else {
            old.checked_sub(1)
                .ok_or_else(|| invalid("Reference count underflow"))?
        };
        if old == count {
            return Ok(());
        }
        if self.before.get((key, 0))?.is_none() {
            self.before.put((key, 0), old)?;
        }
        if count == 0 {
            self.counts.remove((key, 0))
        } else {
            self.counts.put((key, 0), count)
        }
    }

    pub fn next_change(&self, start: u128) -> io::Result<Option<(u128, bool, bool)>> {
        let mut next = Some((start, 0));
        while let Some(key) = next {
            let Some((key, old)) = self.before.first(key)? else {
                break;
            };
            let now = self.contains(key.0)?;
            if now != (old != 0) {
                return Ok(Some((key.0, old != 0, now)));
            }
            next = after(key);
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radix_index_spills_updates_and_orders_full_width_keys() {
        let path = crate::synthetic_tests::folder().join("radix-index");
        let cache = Cache::reserved("synthetic index", 32 * 1024).unwrap();
        let mut index = Index::new(&path, &cache).unwrap();
        assert!(cache.size().peak_bytes <= cache.size().allowance_bytes);
        let mut expected = std::collections::BTreeMap::new();
        for i in 0..6000_u128 {
            let key = (i.wrapping_mul(0x9e3779b97f4a7c15), u128::MAX - i);
            index.put(key, i as u64).unwrap();
            expected.insert(key, i as u64);
        }
        index.put((u128::MAX, u128::MAX), 9).unwrap();
        expected.insert((u128::MAX, u128::MAX), 9);
        for (i, (&key, &value)) in expected.iter().enumerate() {
            assert_eq!(index.get(key).unwrap(), Some(value));
            if i % 3 == 0 {
                index.remove(key).unwrap();
            }
        }
        expected = expected
            .into_iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 0)
            .map(|(_, row)| row)
            .collect();
        let mut next = Some((0, 0));
        for (&key, &value) in &expected {
            assert_eq!(index.first(next.unwrap()).unwrap(), Some((key, value)));
            next = after(key);
        }
        index.clear().unwrap();
        assert_eq!(index.first((0, 0)).unwrap(), None);
        drop(index);
    }
}
