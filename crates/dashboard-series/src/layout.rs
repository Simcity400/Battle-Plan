use crate::{
    codec::{Binary, Decode, Encode, Sink},
    format::{Directory, Entry, List, Table},
    invalid,
    pages::{self, Kind, PAGE_BYTES, PageWrite},
    types::PageId,
};
use std::{io, marker::PhantomData};

const IDS: usize = PAGE_BYTES / 8;

/// One page's first key, so a directory lookup reads one fence and one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fence(pub u64);
impl Binary for Fence {
    const SIZE: usize = 8;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        e.u64(self.0)
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self(d.u64()?))
    }
}

pub(crate) fn per_page<T: Binary>() -> usize {
    PAGE_BYTES / T::SIZE
}

/// Page ids indexed as they are written: each level holds at most one open
/// index page of ids, so a list's scratch does not grow with its length.
pub(crate) struct Pages {
    levels: Vec<Vec<PageId>>,
    count: u64,
    bytes: Sink,
}
impl Pages {
    pub fn new() -> Self {
        Self {
            levels: Vec::new(),
            count: 0,
            bytes: Sink::default(),
        }
    }
    pub fn push(&mut self, io: &mut impl PageWrite, page: PageId) -> io::Result<()> {
        self.count += 1;
        self.add(io, 0, page)
    }
    /// A full index page is sealed only when a further id arrives, so the
    /// finished list has the shape the reader descends.
    fn add(&mut self, io: &mut impl PageWrite, level: usize, page: PageId) -> io::Result<()> {
        if self.levels.len() == level {
            engine_allocation::reserve(&mut self.levels, 1)?;
            self.levels.push(Vec::new());
        }
        if self.levels[level].len() == IDS {
            let index = self.seal(io, level)?;
            self.add(io, level + 1, index)?;
        }
        engine_allocation::reserve(&mut self.levels[level], 1)?;
        self.levels[level].push(page);
        Ok(())
    }
    fn seal(&mut self, io: &mut impl PageWrite, level: usize) -> io::Result<PageId> {
        self.bytes.clear();
        for id in &self.levels[level] {
            self.bytes.put(&id.to_le_bytes())?;
        }
        let page = pages::seal(io, Kind::List, self.bytes.as_slice())?;
        self.levels[level].clear();
        Ok(page)
    }
    pub fn finish(mut self, io: &mut impl PageWrite) -> io::Result<List> {
        match self.count {
            0 => return Ok(List::EMPTY),
            1 => {
                return Ok(List {
                    root: self.levels[0][0],
                    levels: 0,
                    pages: 1,
                });
            }
            _ => {}
        }
        let mut level = 0;
        loop {
            let above = self
                .levels
                .get(level + 1)
                .is_some_and(|ids| !ids.is_empty());
            if level > 0 && !above && self.levels[level].len() == 1 {
                return Ok(List {
                    root: self.levels[level][0],
                    levels: u8::try_from(level).map_err(|_| invalid("series page list"))?,
                    pages: self.count,
                });
            }
            if !self.levels[level].is_empty() {
                let index = self.seal(io, level)?;
                self.add(io, level + 1, index)?;
            }
            level += 1;
        }
    }
}

/// Fixed-width records written page by page.
pub(crate) struct Records<T> {
    page: Sink,
    pages: Pages,
    count: u64,
    _record: PhantomData<T>,
}
impl<T: Binary> Records<T> {
    pub fn new() -> Self {
        Self {
            page: Sink::default(),
            pages: Pages::new(),
            count: 0,
            _record: PhantomData,
        }
    }
    pub fn count(&self) -> u64 {
        self.count
    }
    pub fn push(&mut self, io: &mut impl PageWrite, record: &T) -> io::Result<()> {
        self.page.record(record)?;
        self.count += 1;
        if self.page.len() + T::SIZE > PAGE_BYTES {
            self.flush(io)?;
        }
        Ok(())
    }
    fn flush(&mut self, io: &mut impl PageWrite) -> io::Result<()> {
        if self.page.len() != 0 {
            let page = pages::seal(io, Kind::Records, self.page.as_slice())?;
            self.pages.push(io, page)?;
            self.page.clear();
        }
        Ok(())
    }
    pub fn finish(mut self, io: &mut impl PageWrite) -> io::Result<Table> {
        self.flush(io)?;
        Ok(Table {
            list: self.pages.finish(io)?,
            count: self.count,
        })
    }
}

/// Variable-width entries by ascending key, with a fence per page.
pub(crate) struct Entries {
    keys: Vec<u64>,
    offsets: Vec<u32>,
    body: Sink,
    entry: Sink,
    page: Sink,
    pages: Pages,
    fences: Records<Fence>,
    count: u64,
}
impl Entries {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            offsets: Vec::new(),
            body: Sink::default(),
            entry: Sink::default(),
            page: Sink::default(),
            pages: Pages::new(),
            fences: Records::new(),
            count: 0,
        }
    }
    fn size(&self, extra: usize) -> usize {
        4 + (self.keys.len() + 1) * 12 + self.body.len() + extra
    }
    pub fn push(&mut self, io: &mut impl PageWrite, entry: &Entry) -> io::Result<()> {
        if self.keys.last().is_some_and(|key| *key >= entry.key) {
            return Err(invalid("series directory keys must ascend"));
        }
        self.entry.clear();
        entry.put(&mut self.entry)?;
        if self.size(self.entry.len()) + 4 > PAGE_BYTES {
            if self.keys.is_empty() {
                return Err(invalid("series directory entry exceeds a page"));
            }
            self.flush(io)?;
        }
        engine_allocation::reserve(&mut self.keys, 1)?;
        engine_allocation::reserve(&mut self.offsets, 1)?;
        self.keys.push(entry.key);
        self.offsets.push(self.body.len() as u32);
        self.body.put(self.entry.as_slice())?;
        self.count += 1;
        Ok(())
    }
    fn flush(&mut self, io: &mut impl PageWrite) -> io::Result<()> {
        if self.keys.is_empty() {
            return Ok(());
        }
        self.page.clear();
        self.page.put(&(self.keys.len() as u32).to_le_bytes())?;
        for key in &self.keys {
            self.page.put(&key.to_le_bytes())?;
        }
        for offset in &self.offsets {
            self.page.put(&offset.to_le_bytes())?;
        }
        self.page.put(&(self.body.len() as u32).to_le_bytes())?;
        self.page.put(self.body.as_slice())?;
        self.fences.push(io, &Fence(self.keys[0]))?;
        let page = pages::seal(io, Kind::Records, self.page.as_slice())?;
        self.pages.push(io, page)?;
        self.keys.clear();
        self.offsets.clear();
        self.body.clear();
        Ok(())
    }
    pub fn finish(mut self, io: &mut impl PageWrite) -> io::Result<Directory> {
        self.flush(io)?;
        let count = self.count;
        let list = self.pages.finish(io)?;
        Ok(Directory {
            list,
            fences: self.fences.finish(io)?,
            count,
        })
    }
}

/// Encoded blocks packed into data pages; a block never spans two pages.
pub(crate) struct Data {
    page: Sink,
    pages: Pages,
}
impl Data {
    pub fn new() -> Self {
        Self {
            page: Sink::default(),
            pages: Pages::new(),
        }
    }
    /// The page ordinal and offset where `block` lands.
    pub fn block(&mut self, io: &mut impl PageWrite, block: &[u8]) -> io::Result<(u64, u32)> {
        if block.len() > PAGE_BYTES {
            return Err(invalid("series block exceeds a page"));
        }
        if self.page.len() + block.len() > PAGE_BYTES {
            self.flush(io)?;
        }
        let at = (self.pages.count, self.page.len() as u32);
        self.page.put(block)?;
        Ok(at)
    }
    fn flush(&mut self, io: &mut impl PageWrite) -> io::Result<()> {
        if self.page.len() != 0 {
            let page = pages::seal(io, Kind::Rows, self.page.as_slice())?;
            self.pages.push(io, page)?;
            self.page.clear();
        }
        Ok(())
    }
    pub fn finish(mut self, io: &mut impl PageWrite) -> io::Result<List> {
        self.flush(io)?;
        self.pages.finish(io)
    }
}

/// Heap bytes in whole pages, so an offset names its page by division: `heap` comes a page a
/// piece, its last piece shorter.
pub(crate) fn write_heap<'a>(
    io: &mut impl PageWrite,
    heap: impl Iterator<Item = &'a [u8]>,
) -> io::Result<List> {
    let mut list = Pages::new();
    for chunk in heap {
        let page = pages::seal(io, Kind::Heap, chunk)?;
        list.push(io, page)?;
    }
    list.finish(io)
}
