use crate::{
    codec::{Decode, Encode},
    invalid,
    types::{NONE, PageId},
};
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::Mutex,
};
use zstd::bulk::{Compressor, Decompressor};

pub const PAGE_BYTES: usize = 256 * 1024;
const HEADER: usize = 56;
const MAGIC: &[u8; 8] = b"H4SERIES";
const VERSION: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub(crate) enum Kind {
    Root = 1,
    Node = 2,
    Records = 3,
    Rows = 4,
    Heap = 5,
    List = 6,
}

pub trait PageRead {
    /// Equal identities name equal immutable bytes, across stores and snapshots.
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]>;
    fn len(&mut self, id: PageId) -> io::Result<u64>;
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()>;
}
pub trait PageWrite: PageRead {
    fn append(&mut self, bytes: &[u8]) -> io::Result<PageId>;
}

pub struct StoreRead<'a> {
    pub snapshot: &'a store::Snapshot,
    pub section: &'a str,
}
impl StoreRead<'_> {
    fn range(&self, id: PageId) -> io::Result<store::Range> {
        self.snapshot
            .read(self.section, id)?
            .ok_or_else(|| invalid("missing series range"))
    }
}
impl PageRead for StoreRead<'_> {
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]> {
        Ok(self.range(id)?.id().cache_key())
    }
    fn len(&mut self, id: PageId) -> io::Result<u64> {
        Ok(self.range(id)?.len())
    }
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()> {
        let mut range = self.range(id)?;
        range.seek(SeekFrom::Start(offset))?;
        range.read_exact(into)
    }
}
pub struct StoreWrite<'a> {
    pub writer: &'a mut store::Writer,
    pub section: &'a str,
}
impl StoreWrite<'_> {
    fn range(&mut self, id: PageId) -> io::Result<store::Range> {
        self.writer
            .read_at(self.section, id)?
            .ok_or_else(|| invalid("missing staged series range"))
    }
}
impl PageRead for StoreWrite<'_> {
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]> {
        Ok(self
            .writer
            .range_id(self.section, id)?
            .ok_or_else(|| invalid("missing staged series range"))?
            .cache_key())
    }
    fn len(&mut self, id: PageId) -> io::Result<u64> {
        Ok(self.range(id)?.len())
    }
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()> {
        let mut range = self.range(id)?;
        range.seek(SeekFrom::Start(offset))?;
        range.read_exact(into)
    }
}
impl PageWrite for StoreWrite<'_> {
    fn append(&mut self, bytes: &[u8]) -> io::Result<PageId> {
        self.writer.write(self.section, bytes)
    }
}

/// One page's checked, decompressed bytes.
pub(crate) struct Page {
    kind: Kind,
    raw: Vec<u8>,
}
impl Page {
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub(crate) fn held(&self) -> usize {
        self.raw.capacity() + std::mem::size_of::<Self>()
    }
}

struct Encoder {
    compressor: Compressor<'static>,
    buffer: Vec<u8>,
}
// Contexts are pooled: their count follows the number of concurrent sealers and readers.
static ENCODERS: Mutex<Vec<Encoder>> = Mutex::new(Vec::new());
static DECODERS: Mutex<Vec<Decompressor<'static>>> = Mutex::new(Vec::new());

fn pooled<T>(pool: &Mutex<Vec<T>>) -> Option<T> {
    pool.lock().unwrap_or_else(|e| e.into_inner()).pop()
}
fn restore<T>(pool: &Mutex<Vec<T>>, value: T) {
    pool.lock().unwrap_or_else(|e| e.into_inner()).push(value);
}

pub(crate) fn seal(io: &mut impl PageWrite, kind: Kind, raw: &[u8]) -> io::Result<PageId> {
    if raw.len() > PAGE_BYTES {
        return Err(invalid("split series page before sealing"));
    }
    let mut encoder = match pooled(&ENCODERS) {
        Some(encoder) => encoder,
        None => Encoder {
            compressor: Compressor::new(1)?,
            buffer: Vec::new(),
        },
    };
    let result = seal_with(io, kind, raw, &mut encoder);
    restore(&ENCODERS, encoder);
    result
}
fn seal_with(
    io: &mut impl PageWrite,
    kind: Kind,
    raw: &[u8],
    encoder: &mut Encoder,
) -> io::Result<PageId> {
    let bound = HEADER + zstd::zstd_safe::compress_bound(raw.len());
    if encoder.buffer.len() < bound {
        engine_allocation::resize(&mut encoder.buffer, bound, 0)?;
    }
    let compressed = encoder
        .compressor
        .compress_to_buffer(raw, &mut encoder.buffer[HEADER..bound])?;
    let codec = u8::from(compressed + 32 < raw.len());
    let stored = if codec == 1 {
        compressed
    } else {
        encoder.buffer[HEADER..HEADER + raw.len()].copy_from_slice(raw);
        raw.len()
    };
    let mut header = Encode::new(&mut encoder.buffer[..HEADER]);
    header.bytes(MAGIC)?;
    header.u16(VERSION)?;
    header.u16(kind as u16)?;
    header.u8(codec)?;
    header.bytes(&[0; 3])?;
    header.u32(raw.len() as u32)?;
    header.u32(stored as u32)?;
    header.bytes(blake3::hash(raw).as_bytes())?;
    let id = io.append(&encoder.buffer[..HEADER + stored])?;
    if id == NONE {
        return Err(invalid("series page ID exhausted"));
    }
    Ok(id)
}

pub(crate) fn open(io: &mut impl PageRead, id: PageId, kind: Kind) -> io::Result<Page> {
    if id == NONE {
        return Err(invalid("missing series page"));
    }
    let len = io.len(id)?;
    if len < HEADER as u64 || len > (HEADER + PAGE_BYTES) as u64 {
        return Err(invalid("series page stored length"));
    }
    let mut header = [0; HEADER];
    io.read(id, 0, &mut header)?;
    let mut d = Decode::new(&header);
    if d.bytes(8)? != MAGIC || d.u16()? != VERSION || d.u16()? != kind as u16 {
        return Err(invalid("series page kind/version"));
    }
    let codec = d.u8()?;
    if d.bytes(3)? != [0; 3] {
        return Err(invalid("series reserved header"));
    }
    let raw_len = d.u32()? as usize;
    let stored = d.u32()? as usize;
    let hash = d.array::<32>()?;
    if raw_len > PAGE_BYTES || stored as u64 + HEADER as u64 != len || codec > 1 {
        return Err(invalid("series page dimensions"));
    }
    let mut packed = engine_allocation::filled(stored, 0_u8)?;
    io.read(id, HEADER as u64, &mut packed)?;
    let raw = if codec == 0 {
        if stored != raw_len {
            return Err(invalid("raw series page length"));
        }
        packed
    } else {
        let mut raw = engine_allocation::vec(raw_len)?;
        let mut decoder = match pooled(&DECODERS) {
            Some(decoder) => decoder,
            None => Decompressor::new()?,
        };
        let result = decoder.decompress_to_buffer(&packed, &mut raw);
        restore(&DECODERS, decoder);
        if result? != raw_len || raw.len() != raw_len {
            return Err(invalid("series compressed frame extent"));
        }
        raw
    };
    if blake3::hash(&raw).as_bytes() != &hash {
        return Err(invalid("series page checksum"));
    }
    Ok(Page { kind, raw })
}
