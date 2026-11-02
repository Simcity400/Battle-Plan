use crate::{
    Budget,
    budget::{Buffer, Reservation},
    invalid,
};
use std::{
    fs::OpenOptions,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub const PAGE: usize = 4096;
const NONE: usize = usize::MAX;
struct Slot {
    tag: u64,
    dirty: bool,
    dirty_previous: usize,
    dirty_next: usize,
    previous: usize,
    next: usize,
}
pub struct PagedFile {
    file: std::fs::File,
    data: Vec<Buffer>,
    slots: Vec<Slot>,
    _held: Reservation,
    lookup: Buffer,
    capacity: usize,
    newest: usize,
    oldest: usize,
    last: usize,
    dirty_head: usize,
    len: u64,
    physical: u64,
    memory: Budget,
    disk: Budget,
}
impl PagedFile {
    pub fn create(path: &Path, allowance: u64, memory: &Budget, disk: &Budget) -> io::Result<Self> {
        let allowance =
            usize::try_from(allowance).map_err(|_| invalid("page allowance overflow"))?;
        let mut capacity = allowance / (PAGE + size_of::<Slot>() + size_of::<Buffer>() + 32);
        while capacity != 0 {
            let entries = capacity
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(|| invalid("page directory overflow"))?;
            let bytes = capacity
                .checked_mul(PAGE + size_of::<Slot>() + size_of::<Buffer>())
                .and_then(|n| entries.checked_mul(8).and_then(|m| n.checked_add(m)))
                .ok_or_else(|| invalid("page allowance overflow"))?;
            if bytes <= allowance {
                break;
            }
            capacity -= 1;
        }
        if capacity == 0 {
            return Err(invalid(
                "paged store allowance cannot hold one page and its directory",
            ));
        }
        let held = memory.reserve((capacity * (size_of::<Slot>() + size_of::<Buffer>())) as u64)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(io::Error::other)?;
        if slots.capacity() != capacity {
            return Err(invalid("allocator exceeded page metadata reservation"));
        }
        let mut data = Vec::new();
        data.try_reserve_exact(capacity).map_err(io::Error::other)?;
        if data.capacity() != capacity {
            return Err(invalid("allocator exceeded page metadata reservation"));
        }
        let mut lookup = Buffer::new((capacity * 2).next_power_of_two() * 8, memory)?;
        lookup.resize(lookup.capacity())?;
        Ok(Self {
            file: OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(path)?,
            data,
            slots,
            _held: held,
            lookup,
            capacity,
            newest: NONE,
            oldest: NONE,
            last: NONE,
            dirty_head: NONE,
            len: 0,
            physical: 0,
            memory: memory.clone(),
            disk: disk.clone(),
        })
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(crate) fn discard(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.disk.release_disk(self.len);
        self.len = 0;
        self.physical = 0;
        self.slots.clear();
        self.lookup.fill(0);
        self.newest = NONE;
        self.oldest = NONE;
        self.last = NONE;
        self.dirty_head = NONE;
        Ok(())
    }
    pub fn grow(&mut self, len: u64) -> io::Result<()> {
        if len > self.len {
            self.disk.grow_disk(len - self.len)?;
            self.len = len;
        }
        Ok(())
    }
    fn hash(&self, tag: u64) -> usize {
        let mut hash = tag.wrapping_add(0x9e3779b97f4a7c15);
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        (hash ^ (hash >> 31)) as usize & (self.lookup.len() / 8 - 1)
    }
    fn entry(&self, at: usize) -> usize {
        u64::from_le_bytes(self.lookup[at * 8..at * 8 + 8].try_into().unwrap()) as usize
    }
    fn set_entry(&mut self, at: usize, value: usize) {
        self.lookup[at * 8..at * 8 + 8].copy_from_slice(&(value as u64).to_le_bytes());
    }
    fn find(&self, tag: u64) -> usize {
        let mut at = self.hash(tag);
        while self.entry(at) != 0 && self.slots[self.entry(at) - 1].tag != tag {
            at = (at + 1) & (self.lookup.len() / 8 - 1);
        }
        at
    }
    fn remove(&mut self, tag: u64) {
        let mask = self.lookup.len() / 8 - 1;
        let mut hole = self.find(tag);
        let mut at = (hole + 1) & mask;
        while self.entry(at) != 0 {
            let value = self.entry(at);
            let home = self.hash(self.slots[value - 1].tag);
            if (at.wrapping_sub(home) & mask) >= (hole.wrapping_sub(home) & mask) {
                self.set_entry(hole, value);
                hole = at;
            }
            at = (at + 1) & mask;
        }
        self.set_entry(hole, 0);
    }
    fn touch(&mut self, slot: usize) {
        self.last = slot;
        if self.newest == slot {
            return;
        }
        let previous = self.slots[slot].previous;
        let next = self.slots[slot].next;
        if previous != NONE {
            self.slots[previous].next = next;
        } else if self.oldest == slot {
            self.oldest = next;
        }
        if next != NONE {
            self.slots[next].previous = previous;
        }
        self.slots[slot].previous = self.newest;
        self.slots[slot].next = NONE;
        if self.newest != NONE {
            self.slots[self.newest].next = slot;
        } else {
            self.oldest = slot;
        }
        self.newest = slot;
    }
    fn mark_dirty(&mut self, slot: usize) {
        if self.slots[slot].dirty {
            return;
        }
        self.slots[slot].dirty = true;
        self.slots[slot].dirty_previous = NONE;
        self.slots[slot].dirty_next = self.dirty_head;
        if self.dirty_head != NONE {
            self.slots[self.dirty_head].dirty_previous = slot;
        }
        self.dirty_head = slot;
    }
    fn flush_slot(&mut self, slot: usize, spill: bool) -> io::Result<()> {
        if self.slots[slot].dirty {
            let start = self.slots[slot].tag * PAGE as u64;
            let count = (self.len - start).min(PAGE as u64) as usize;
            self.file.seek(SeekFrom::Start(start))?;
            self.file.write_all(&self.data[slot][..count])?;
            self.physical = self.physical.max(start + count as u64);
            let previous = self.slots[slot].dirty_previous;
            let next = self.slots[slot].dirty_next;
            if previous == NONE {
                self.dirty_head = next;
            } else {
                self.slots[previous].dirty_next = next;
            }
            if next != NONE {
                self.slots[next].dirty_previous = previous;
            }
            self.slots[slot].dirty = false;
            if spill {
                self.memory.spill(count as u64);
            }
        }
        Ok(())
    }
    fn page(&mut self, offset: u64) -> io::Result<usize> {
        let tag = offset / PAGE as u64;
        let slot = if self.last != NONE && self.slots[self.last].tag == tag {
            self.last
        } else {
            let at = self.find(tag);
            if self.entry(at) != 0 {
                self.entry(at) - 1
            } else {
                let slot = if self.slots.len() < self.capacity {
                    let slot = self.slots.len();
                    if slot == self.data.len() {
                        let mut page = Buffer::new(PAGE, &self.memory)?;
                        page.resize(PAGE)?;
                        self.data.push(page);
                    }
                    self.slots.push(Slot {
                        tag,
                        dirty: false,
                        dirty_previous: NONE,
                        dirty_next: NONE,
                        previous: NONE,
                        next: NONE,
                    });
                    slot
                } else {
                    let slot = self.oldest;
                    self.flush_slot(slot, true)?;
                    self.remove(self.slots[slot].tag);
                    slot
                };
                let start = tag * PAGE as u64;
                let count = self.physical.saturating_sub(start).min(PAGE as u64) as usize;
                self.memory.page_miss(count != 0);
                let data = &mut self.data[slot];
                data.fill(0);
                if count != 0 {
                    self.file.seek(SeekFrom::Start(start))?;
                    self.file.read_exact(&mut data[..count])?;
                }
                self.slots[slot].tag = tag;
                let at = self.find(tag);
                self.set_entry(at, slot + 1);
                slot
            }
        };
        self.touch(slot);
        Ok(slot * PAGE + (offset % PAGE as u64) as usize)
    }
    pub fn read(&mut self, mut offset: u64, mut out: &mut [u8]) -> io::Result<()> {
        if offset
            .checked_add(out.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(invalid("paged read exceeds backing store"));
        }
        while !out.is_empty() {
            let at = self.page(offset)?;
            let n = out.len().min(PAGE - at % PAGE);
            out[..n].copy_from_slice(&self.data[at / PAGE][at % PAGE..at % PAGE + n]);
            out = &mut out[n..];
            offset += n as u64;
        }
        Ok(())
    }
    pub fn write(&mut self, mut offset: u64, mut bytes: &[u8]) -> io::Result<()> {
        self.grow(
            offset
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("paged address overflow"))?,
        )?;
        while !bytes.is_empty() {
            let at = self.page(offset)?;
            let n = bytes.len().min(PAGE - at % PAGE);
            self.data[at / PAGE][at % PAGE..at % PAGE + n].copy_from_slice(&bytes[..n]);
            self.mark_dirty(at / PAGE);
            bytes = &bytes[n..];
            offset += n as u64;
        }
        Ok(())
    }
    pub fn flush(&mut self) -> io::Result<()> {
        while self.dirty_head != NONE {
            self.flush_slot(self.dirty_head, false)?;
        }
        if self.physical != self.len {
            self.file.set_len(self.len)?;
            self.physical = self.len;
        }
        Ok(())
    }
}

// Consumer scratch pins a dense prefix; only its overflow needs associative paging.
pub struct FlatFile {
    pages: Vec<Buffer>,
    _held: Reservation,
    capacity: usize,
    overflow: PagedFile,
    memory: Budget,
    len: u64,
}
impl FlatFile {
    pub fn create(path: &Path, allowance: u64, memory: &Budget, disk: &Budget) -> io::Result<Self> {
        let prefix =
            usize::try_from(allowance / 4 * 3).map_err(|_| invalid("flat allowance overflow"))?;
        let capacity = prefix / (PAGE + size_of::<Buffer>());
        let bytes = capacity * (PAGE + size_of::<Buffer>());
        let held = memory.reserve((capacity * size_of::<Buffer>()) as u64)?;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(capacity)
            .map_err(io::Error::other)?;
        if pages.capacity() != capacity {
            return Err(invalid("allocator exceeded flat directory reservation"));
        }
        Ok(Self {
            pages,
            _held: held,
            capacity,
            overflow: PagedFile::create(path, allowance - bytes as u64, memory, disk)?,
            memory: memory.clone(),
            len: 0,
        })
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[inline]
    pub fn grow(&mut self, len: u64) -> io::Result<()> {
        self.overflow
            .grow(len.saturating_sub((self.capacity * PAGE) as u64))?;
        self.len = self.len.max(len);
        Ok(())
    }
    #[inline]
    fn prefix(&mut self, offset: u64) -> io::Result<(usize, usize)> {
        let page = offset as usize / PAGE;
        while self.pages.len() <= page {
            let mut data = Buffer::new(PAGE, &self.memory)?;
            data.resize(PAGE)?;
            self.pages.push(data);
            self.memory.page_miss(false);
        }
        Ok((page, offset as usize % PAGE))
    }
    #[inline]
    pub fn read(&mut self, mut offset: u64, mut out: &mut [u8]) -> io::Result<()> {
        if offset
            .checked_add(out.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(invalid("flat read exceeds store"));
        }
        let bound = (self.capacity * PAGE) as u64;
        while !out.is_empty() && offset < bound {
            let (page, at) = self.prefix(offset)?;
            let n = out.len().min(PAGE - at);
            out[..n].copy_from_slice(&self.pages[page][at..at + n]);
            offset += n as u64;
            out = &mut out[n..];
        }
        if !out.is_empty() {
            self.overflow.read(offset - bound, out)?;
        }
        Ok(())
    }
    #[inline]
    pub fn write(&mut self, mut offset: u64, mut input: &[u8]) -> io::Result<()> {
        self.grow(
            offset
                .checked_add(input.len() as u64)
                .ok_or_else(|| invalid("flat address overflow"))?,
        )?;
        let bound = (self.capacity * PAGE) as u64;
        while !input.is_empty() && offset < bound {
            let (page, at) = self.prefix(offset)?;
            let n = input.len().min(PAGE - at);
            self.pages[page][at..at + n].copy_from_slice(&input[..n]);
            offset += n as u64;
            input = &input[n..];
        }
        if !input.is_empty() {
            self.overflow.write(offset - bound, input)?;
        }
        Ok(())
    }
}
