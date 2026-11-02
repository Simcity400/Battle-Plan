use crate::{
    budget::{Budget, Buffer},
    invalid,
    paged::PagedFile,
    types::{Blob, KeyId},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub struct Dictionary {
    keys: PagedFile,
    index: PagedFile,
    offsets: PagedFile,
    end: u64,
    slots: u64,
    count: u64,
    scratch: Buffer,
    last: Buffer,
    last_id: Option<u64>,
}
impl Dictionary {
    pub fn create(path: &Path, allowance: u64, memory: &Budget, disk: &Budget) -> io::Result<Self> {
        Self::create_named(path, "keys", allowance, memory, disk)
    }
    pub fn create_named(
        path: &Path,
        name: &str,
        allowance: u64,
        memory: &Budget,
        disk: &Budget,
    ) -> io::Result<Self> {
        let pages = allowance
            .checked_sub(8192)
            .ok_or_else(|| invalid("dictionary allowance cannot hold key buffers"))?;
        let mut keys = PagedFile::create(&path.join(name), pages / 8 * 3, memory, disk)?;
        keys.write(0, b"H4KEYS02")?;
        let mut index =
            PagedFile::create(&path.join(format!("{name}-index")), pages / 2, memory, disk)?;
        index.grow(4096)?;
        Ok(Self {
            keys,
            index,
            offsets: PagedFile::create(
                &path.join(format!("{name}-offsets")),
                pages / 8,
                memory,
                disk,
            )?,
            end: 8,
            slots: 256,
            count: 0,
            scratch: Buffer::new(4096, memory)?,
            last: Buffer::new(4096, memory)?,
            last_id: None,
        })
    }
    fn entry(&mut self, slot: u64) -> io::Result<(u64, u64)> {
        let mut bytes = [0; 16];
        self.index.read(slot * 16, &mut bytes)?;
        Ok((
            u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..].try_into().unwrap()),
        ))
    }
    fn insert(&mut self, hash: u64, offset: u64) -> io::Result<()> {
        let mut slot = hash & (self.slots - 1);
        while self.entry(slot)?.1 != 0 {
            slot = (slot + 1) & (self.slots - 1);
        }
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&hash.to_le_bytes());
        bytes[8..].copy_from_slice(&offset.to_le_bytes());
        self.index.write(slot * 16, &bytes)
    }
    fn grow_index(&mut self) -> io::Result<()> {
        let slots = self
            .slots
            .checked_mul(2)
            .ok_or_else(|| invalid("dictionary index overflow"))?;
        let end = slots
            .checked_mul(16)
            .ok_or_else(|| invalid("dictionary index overflow"))?;
        self.index.discard()?;
        self.index.grow(end)?;
        self.slots = slots;
        let mut at = 8;
        for _ in 0..self.count {
            let mut header = [0; 40];
            self.keys.read(at, &mut header)?;
            let hash = u64::from_le_bytes(header[..8].try_into().unwrap());
            self.insert(hash, at)?;
            at += 40 + u64::from_le_bytes(header[16..24].try_into().unwrap());
        }
        Ok(())
    }
    pub fn intern(&mut self, key: &[u8]) -> io::Result<u64> {
        if let Some(id) = self.last_id {
            if &self.last[..] == key {
                return Ok(id);
            }
        }
        let id = self.intern_inner(key)?;
        self.last.clear();
        self.last_id = if key.len() <= self.last.capacity() {
            self.last.append(key)?;
            Some(id)
        } else {
            None
        };
        Ok(id)
    }
    fn intern_inner(&mut self, key: &[u8]) -> io::Result<u64> {
        let mut hash = 0xcbf29ce484222325_u64;
        for &b in key {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        let mut slot = hash & (self.slots - 1);
        loop {
            let (stored_hash, at) = self.entry(slot)?;
            if at == 0 {
                break;
            }
            if stored_hash == hash {
                let mut header = [0; 40];
                self.keys.read(at, &mut header)?;
                if u64::from_le_bytes(header[16..24].try_into().unwrap()) == key.len() as u64 {
                    let mut same = true;
                    let mut checked = 0;
                    while checked < key.len() {
                        let n = (key.len() - checked).min(self.scratch.capacity());
                        self.scratch.resize(n)?;
                        self.keys
                            .read(at + 40 + checked as u64, &mut self.scratch)?;
                        if self.scratch[..] != key[checked..checked + n] {
                            same = false;
                            break;
                        }
                        checked += n;
                    }
                    if same {
                        return Ok(u64::from_le_bytes(header[32..40].try_into().unwrap()));
                    }
                }
            }
            slot = (slot + 1) & (self.slots - 1);
        }
        if self.count + 1 > self.slots / 2 {
            self.grow_index()?;
        }
        let next = self
            .end
            .checked_add(40)
            .and_then(|n| n.checked_add(key.len() as u64))
            .ok_or_else(|| invalid("dictionary file address overflow"))?;
        let id = self.count;
        let count = id
            .checked_add(1)
            .ok_or_else(|| invalid("dictionary count overflow"))?;
        let mut header = [0; 40];
        header[..8].copy_from_slice(&hash.to_le_bytes());
        header[16..24].copy_from_slice(&(key.len() as u64).to_le_bytes());
        header[24..28].copy_from_slice(&crc32fast::hash(key).to_le_bytes());
        header[32..40].copy_from_slice(&id.to_le_bytes());
        self.keys.write(self.end, &header)?;
        self.keys.write(self.end + 40, key)?;
        self.offsets.write(
            id.checked_mul(8)
                .ok_or_else(|| invalid("dictionary id overflow"))?,
            &self.end.to_le_bytes(),
        )?;
        self.insert(hash, self.end)?;
        self.end = next;
        self.count = count;
        Ok(id)
    }
    pub fn flush(&mut self) -> io::Result<()> {
        self.keys.flush()?;
        self.offsets.flush()
    }
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        self.flush()?;
        self.index.discard()
    }
}

pub(crate) struct Arena {
    file: File,
    buffer: Buffer,
    end: u64,
    disk: Budget,
}
impl Arena {
    pub fn create(path: &Path, bytes: usize, memory: &Budget, disk: &Budget) -> io::Result<Self> {
        let buffer = Buffer::new(bytes, &memory.owner(crate::Owner::WriterBlocks))?;
        Ok(Self {
            file: OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path.join("arena"))?,
            buffer,
            end: 0,
            disk: disk.clone(),
        })
    }
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let count = bytes.len().min(self.buffer.capacity() - self.buffer.len());
            self.buffer.append(&bytes[..count])?;
            bytes = &bytes[count..];
            if self.buffer.len() == self.buffer.capacity() {
                self.flush()?;
            }
        }
        Ok(())
    }
    pub fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            self.file.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
    fn header(&mut self, len: u64, crc: u32) -> io::Result<Blob> {
        let size = len
            .checked_add(16)
            .ok_or_else(|| invalid("arena address overflow"))?;
        let next = self
            .end
            .checked_add(size)
            .ok_or_else(|| invalid("arena address overflow"))?;
        self.disk.grow_disk(size)?;
        let mut header = [0; 16];
        header[..8].copy_from_slice(&len.to_le_bytes());
        header[8..12].copy_from_slice(&crc.to_le_bytes());
        self.write(&header)?;
        let blob = Blob {
            offset: self.end + 16,
            len,
        };
        self.end = next;
        Ok(blob)
    }
    pub fn bytes(&mut self, data: &[u8]) -> io::Result<Blob> {
        let blob = self.header(data.len() as u64, crc32fast::hash(data))?;
        self.write(data)?;
        Ok(blob)
    }
    pub fn u32s(&mut self, data: &[u32]) -> io::Result<Blob> {
        self.numbers(data.len(), 4, |i| u64::from(data[i]))
    }
    pub fn u64s(&mut self, data: &[u64]) -> io::Result<Blob> {
        self.numbers(data.len(), 8, |i| data[i])
    }
    fn numbers(
        &mut self,
        count: usize,
        width: usize,
        at: impl Fn(usize) -> u64,
    ) -> io::Result<Blob> {
        let len = (count as u64)
            .checked_mul(width as u64)
            .ok_or_else(|| invalid("list length overflow"))?;
        let mut hash = crc32fast::Hasher::new();
        let mut chunk = [0; 4096];
        for start in (0..count).step_by(chunk.len() / width) {
            let n = (count - start).min(chunk.len() / width);
            for i in 0..n {
                chunk[i * width..(i + 1) * width]
                    .copy_from_slice(&at(start + i).to_le_bytes()[..width]);
            }
            hash.update(&chunk[..n * width]);
        }
        let blob = self.header(len, hash.finalize())?;
        for start in (0..count).step_by(chunk.len() / width) {
            let n = (count - start).min(chunk.len() / width);
            for i in 0..n {
                chunk[i * width..(i + 1) * width]
                    .copy_from_slice(&at(start + i).to_le_bytes()[..width]);
            }
            self.write(&chunk[..n * width])?;
        }
        Ok(blob)
    }
}

pub struct PayloadReader {
    keys: Pages,
    offsets: Pages,
    reasons: Pages,
    reason_offsets: Pages,
    arena: Pages,
}
pub struct PayloadBlock<'a> {
    reader: &'a mut PayloadReader,
    location: crate::BlockLocation,
}
impl PayloadBlock<'_> {
    pub fn key(
        &mut self,
        id: KeyId<'_>,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.reader
            .key(id, visit)
            .map_err(|error| crate::reader::damage(self.location, error))
    }
    pub fn reason(
        &mut self,
        id: u64,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.reader
            .reason(id, visit)
            .map_err(|e| crate::reader::damage(self.location, e))
    }
    pub fn key_number(
        &mut self,
        id: u64,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.reader
            .key_number(id, visit)
            .map_err(|e| crate::reader::damage(self.location, e))
    }
    pub fn blob(
        &mut self,
        blob: Blob,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        self.reader
            .blob(blob, visit)
            .map_err(|error| crate::reader::damage(self.location, error))
    }
}
impl PayloadReader {
    pub fn for_block(&mut self, location: crate::BlockLocation) -> PayloadBlock<'_> {
        PayloadBlock {
            reader: self,
            location,
        }
    }
    pub fn open(path: &Path, page_bytes: usize, memory: &Budget) -> io::Result<Self> {
        if page_bytes == 0 {
            return Err(invalid("payload page size must be nonzero"));
        }
        let memory = memory.owner(crate::Owner::PayloadPages);
        Ok(Self {
            keys: Pages::open(&path.join("keys"), page_bytes, &memory)?,
            offsets: Pages::open(&path.join("keys-offsets"), page_bytes, &memory)?,
            reasons: Pages::open(&path.join("reasons"), page_bytes, &memory)?,
            reason_offsets: Pages::open(&path.join("reasons-offsets"), page_bytes, &memory)?,
            arena: Pages::open(&path.join("arena"), page_bytes, &memory)?,
        })
    }
    fn key(
        &mut self,
        id: KeyId<'_>,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.key_number(id.index(), visit)
    }
    fn key_number(
        &mut self,
        id: u64,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        Self::dictionary(&mut self.keys, &mut self.offsets, id, visit)
    }
    fn reason(&mut self, id: u64, visit: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<u64> {
        Self::dictionary(&mut self.reasons, &mut self.reason_offsets, id, visit)
    }
    fn dictionary(
        keys: &mut Pages,
        offsets: &mut Pages,
        id: u64,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        let mut offset = [0; 8];
        offsets.copy(
            id.checked_mul(8)
                .ok_or_else(|| invalid("dictionary id overflow"))?,
            &mut offset,
        )?;
        let at = u64::from_le_bytes(offset);
        if at < 8 {
            return Err(invalid("invalid dictionary offset"));
        }
        let mut h = [0; 40];
        keys.copy(at, &mut h)?;
        if u64::from_le_bytes(h[32..40].try_into().unwrap()) != id {
            return Err(invalid("dictionary id mismatch"));
        }
        let len = u64::from_le_bytes(h[16..24].try_into().unwrap());
        keys.visit(
            at.checked_add(40)
                .ok_or_else(|| invalid("key address overflow"))?,
            len,
            u32::from_le_bytes(h[24..28].try_into().unwrap()),
            visit,
        )?;
        Ok(len)
    }
    fn blob(&mut self, blob: Blob, visit: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        let at = blob
            .offset
            .checked_sub(16)
            .ok_or_else(|| invalid("invalid arena offset"))?;
        let mut h = [0; 16];
        self.arena.copy(at, &mut h)?;
        if u64::from_le_bytes(h[..8].try_into().unwrap()) != blob.len {
            return Err(invalid("arena length mismatch"));
        }
        self.arena.visit(
            blob.offset,
            blob.len,
            u32::from_le_bytes(h[8..12].try_into().unwrap()),
            visit,
        )
    }
}
struct Pages {
    file: File,
    page: Buffer,
    start: u64,
    valid: bool,
}
impl Pages {
    fn open(path: &Path, bytes: usize, memory: &Budget) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            page: Buffer::new(bytes, memory)?,
            start: 0,
            valid: false,
        })
    }
    fn at(&mut self, offset: u64) -> io::Result<&[u8]> {
        if !self.valid || offset < self.start || offset - self.start >= self.page.len() as u64 {
            self.start = offset / self.page.capacity() as u64 * self.page.capacity() as u64;
            self.file.seek(SeekFrom::Start(self.start))?;
            self.page.resize(self.page.capacity())?;
            let mut n = 0;
            while n < self.page.len() {
                let read = self.file.read(&mut self.page[n..])?;
                if read == 0 {
                    break;
                }
                n += read;
            }
            self.page.resize(n)?;
            self.valid = true;
        }
        let index =
            usize::try_from(offset - self.start).map_err(|_| invalid("page address overflow"))?;
        if index >= self.page.len() {
            return Err(invalid("payload exceeds backing file"));
        }
        Ok(&self.page[index..])
    }
    fn copy(&mut self, mut offset: u64, mut out: &mut [u8]) -> io::Result<()> {
        while !out.is_empty() {
            let page = self.at(offset)?;
            let n = page.len().min(out.len());
            out[..n].copy_from_slice(&page[..n]);
            offset = offset
                .checked_add(n as u64)
                .ok_or_else(|| invalid("payload address overflow"))?;
            out = &mut out[n..];
        }
        Ok(())
    }
    fn chunks(
        &mut self,
        mut offset: u64,
        mut len: u64,
        mut visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        offset
            .checked_add(len)
            .ok_or_else(|| invalid("payload address overflow"))?;
        while len != 0 {
            let page = self.at(offset)?;
            let n = len.min(page.len() as u64) as usize;
            visit(&page[..n])?;
            offset += n as u64;
            len -= n as u64;
        }
        Ok(())
    }
    fn visit(
        &mut self,
        offset: u64,
        len: u64,
        crc: u32,
        visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut hash = crc32fast::Hasher::new();
        self.chunks(offset, len, |bytes| {
            hash.update(bytes);
            Ok(())
        })?;
        if hash.finalize() != crc {
            return Err(invalid("payload checksum failed"));
        }
        // Validate the complete value before lending any bytes to a consumer.
        self.chunks(offset, len, visit)
    }
}
