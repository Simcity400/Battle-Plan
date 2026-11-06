//! Where a family's items wait for its emit: a file of the run's folder for each family and
//! segment, `<folder>/<lane>-<family>/<segment>`, its walk writing each item as the operator
//! pushes it and its emit reading them back in order once the walk filed the segment, then
//! removing it. Parked operators spill to files of their own there.
//!
//! A file is a sequence of records, each its kind, its length and its bytes: an item, or the
//! shared allocations its writer forgot after it ([`replay_spool::Shared::released`]). A
//! walk's writer keeps its shared allocations across the segments its operator goes on into,
//! its lineage, and the emit's reader keeps them for the lineage alike.

use replay_spool::{Reader, Restored, Shared, Spooled, Writer};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const ITEM: u8 = 0;
const RELEASED: u8 = 1;
/// What a walk buffers of a file before writing it out, and an emit reads at once.
const BUFFER: usize = 64 << 10;
/// An item's bytes past which their buffer is let go once written or read, not kept for the
/// next: such items are rare, and one walk's or emit's buffer would hold the largest.
const KEPT: usize = 16 * BUFFER;

/// The run's spool folder, emptied of what an earlier run left as it opens and removed once
/// dropped.
pub(super) struct Folder {
    root: PathBuf,
    lanes: Vec<PathBuf>,
    next: AtomicU64,
    lineages: AtomicU64,
    counts: Arc<Counts>,
}

/// Bytes a spool holds on disk now, at most, and wrote in all.
#[derive(Default)]
struct Counts {
    held: AtomicU64,
    peak: AtomicU64,
    written: AtomicU64,
}

impl Counts {
    fn wrote(&self, bytes: u64) {
        self.written.fetch_add(bytes, Ordering::Relaxed);
        let held = self.held.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak.fetch_max(held, Ordering::Relaxed);
    }

    fn removed(&self, bytes: u64) {
        self.held.fetch_sub(bytes, Ordering::Relaxed);
    }
}

/// A family's name as a folder's.
fn folder_name(lane: usize, name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    format!("{lane}-{name}")
}

impl Folder {
    /// The spool at `root` for families named `names`, by lane.
    pub(super) fn open(root: &Path, names: &[&str]) -> io::Result<Self> {
        // What an interrupted run left holds nothing a later run reads.
        match fs::remove_dir_all(root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir_all(root)?;
        let mut lanes = engine_allocation::vec(names.len())?;
        for (lane, name) in names.iter().enumerate() {
            let path = root.join(folder_name(lane, name));
            fs::create_dir(&path)?;
            lanes.push(path);
        }
        Ok(Self {
            root: root.to_path_buf(),
            lanes,
            next: AtomicU64::new(0),
            lineages: AtomicU64::new(0),
            counts: Arc::default(),
        })
    }

    /// A new lineage of a family's walks: an operator made anew.
    pub(super) fn lineage(&self) -> u64 {
        self.lineages.fetch_add(1, Ordering::Relaxed)
    }

    /// The most it held on disk at once, and what it wrote in all.
    pub(super) fn spooled(&self) -> (u64, u64) {
        (
            self.counts.peak.load(Ordering::Relaxed),
            self.counts.written.load(Ordering::Relaxed),
        )
    }

    /// Writes `bytes`, a parked operator, to a file of its own, removed once dropped.
    pub(super) fn spill(&self, bytes: &[u8]) -> io::Result<Spilled> {
        let path = self.root.join(format!(
            "carry-{}",
            self.next.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let spilled = Spilled {
            counts: Arc::clone(&self.counts),
            path,
            length: bytes.len(),
        };
        self.counts.wrote(bytes.len() as u64);
        file.write_all(bytes)?;
        Ok(spilled)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // Best effort: a later run empties it as it opens.
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A parked operator in a [`Folder`].
pub(super) struct Spilled {
    counts: Arc<Counts>,
    path: PathBuf,
    length: usize,
}

impl Spilled {
    pub(super) fn read(&self, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        engine_allocation::reserve_exact(out, self.length)?;
        File::open(&self.path)?
            .take(self.length as u64 + 1)
            .read_to_end(out)?;
        if out.len() != self.length {
            return Err(super::invalid("A spill reads back at another length"));
        }
        Ok(())
    }
}

impl Drop for Spilled {
    fn drop(&mut self) {
        self.counts.removed(self.length as u64);
        let _ = fs::remove_file(&self.path);
    }
}

/// What a family's walks write of their items: the lineage's shared allocations, and the
/// segment's file once it holds an item.
pub(super) struct Spooling {
    lineage: u64,
    /// Files its lineage filed before the segment's.
    parts: u64,
    shared: Shared,
    scratch: Vec<u8>,
    released: Vec<u8>,
    open: Option<Open>,
    /// The segment's items, and their bytes.
    items: u64,
    length: u64,
}

struct Open {
    file: File,
    path: PathBuf,
    buffer: Vec<u8>,
}

/// A segment's items as its walk filed them, for its family's emit: its file is removed once
/// dropped.
pub(super) struct Filed {
    counts: Arc<Counts>,
    path: Option<PathBuf>,
    /// The resident allocations its items name that its lineage had not handed on before.
    resident: Vec<Arc<dyn std::any::Any + Send + Sync>>,
    pub(super) items: u64,
    length: u64,
    lineage: u64,
    /// It begins its lineage.
    pub(super) first: bool,
    /// Its lineage goes on into a later segment's file.
    pub(super) goes_on: bool,
}

impl Drop for Filed {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            self.counts.removed(self.length);
            let _ = fs::remove_file(path);
        }
    }
}

fn record_header(out: &mut Vec<u8>, kind: u8, length: usize) {
    out.push(kind);
    let mut length = length as u64;
    loop {
        let byte = (length & 0x7f) as u8;
        length >>= 7;
        if length == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

impl Spooling {
    pub(super) fn lineage(&self) -> u64 {
        self.lineage
    }

    pub(super) fn new(lineage: u64) -> Self {
        Self {
            lineage,
            parts: 0,
            shared: Shared::default(),
            scratch: Vec::new(),
            released: Vec::new(),
            open: None,
            items: 0,
            length: 0,
        }
    }

    /// Writes `item` into `item`'s bytes, by its family's code: [`Self::put`] spools them.
    pub(super) fn encode<T: Spooled>(&mut self, item: &T) -> io::Result<()> {
        self.scratch.clear();
        item.spool(&mut Writer::new(&mut self.scratch, &mut self.shared))
    }

    /// Spools the item [`Self::encode`] wrote, once it dropped, into segment `at`'s file of
    /// family `lane`, with what its lineage forgot since.
    pub(super) fn put(&mut self, folder: &Folder, lane: usize, at: usize) -> io::Result<()> {
        self.shared.sweep()?;
        self.released.clear();
        for &code in self.shared.released().iter() {
            put_varint(&mut self.released, code)?;
        }
        self.shared.released().clear();
        if self.open.is_none() {
            let path = folder.lanes[lane].join(at.to_string());
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            self.open = Some(Open {
                file,
                path,
                buffer: engine_allocation::vec(BUFFER)?,
            });
        }
        let open = self.open.as_mut().expect("the file is open");
        let mut written = 0;
        for (kind, bytes) in [(ITEM, &self.scratch), (RELEASED, &self.released)] {
            if kind == RELEASED && bytes.is_empty() {
                continue;
            }
            if open.buffer.len() + bytes.len() + 11 > BUFFER {
                open.file.write_all(&open.buffer)?;
                open.buffer.clear();
            }
            let before = open.buffer.len();
            record_header(&mut open.buffer, kind, bytes.len());
            written += (open.buffer.len() - before + bytes.len()) as u64;
            if bytes.len() > BUFFER / 2 {
                open.file.write_all(&open.buffer)?;
                open.buffer.clear();
                open.file.write_all(bytes)?;
            } else {
                open.buffer.extend_from_slice(bytes);
            }
        }
        folder.counts.wrote(written);
        self.items += 1;
        self.length += written;
        if self.scratch.capacity() > KEPT {
            self.scratch = Vec::new();
        }
        Ok(())
    }

    /// The bytes spooled of the segment so far.
    pub(super) fn length(&self) -> u64 {
        self.length
    }

    /// Writes out what it buffered of the segment.
    pub(super) fn flush(&mut self) -> io::Result<()> {
        if let Some(open) = &mut self.open
            && !open.buffer.is_empty()
        {
            open.file.write_all(&open.buffer)?;
            open.buffer.clear();
        }
        Ok(())
    }

    /// Files what it spooled of the segment, [`Self::flush`]ed, for its emit; `goes_on` when
    /// its operator goes on into the segment after, writing on in this lineage.
    pub(super) fn file(&mut self, folder: &Folder, goes_on: bool) -> Filed {
        let path = self.open.take().map(|open| open.path);
        let filed = Filed {
            counts: Arc::clone(&folder.counts),
            path,
            resident: self.shared.take_resident(),
            items: std::mem::take(&mut self.items),
            length: std::mem::take(&mut self.length),
            lineage: self.lineage,
            first: self.parts == 0,
            goes_on,
        };
        self.parts += 1;
        filed
    }

    /// Drops what it spooled of the segment and has not filed.
    pub(super) fn discard(&mut self, folder: &Folder) {
        if let Some(open) = self.open.take() {
            drop(open.file);
            let _ = fs::remove_file(&open.path);
        }
        folder.counts.removed(std::mem::take(&mut self.length));
        self.items = 0;
    }
}

impl Drop for Spooling {
    fn drop(&mut self) {
        // A walk dropped without filing or discarding, as a run stops: its folder goes too.
        if let Some(open) = self.open.take() {
            drop(open.file);
            let _ = fs::remove_file(&open.path);
        }
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) -> io::Result<()> {
    engine_allocation::reserve(out, 10)?;
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return Ok(());
        }
        out.push(byte | 0x80);
    }
}

fn varint(input: &mut impl Read) -> io::Result<u64> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let mut byte = [0_u8];
        input.read_exact(&mut byte)?;
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(super::invalid("A spool record's length runs too long"))
}

/// A family's emit's readers: each lineage's shared allocations, and the bytes of the item
/// being read.
#[derive(Default)]
pub(super) struct Readers {
    lineages: std::collections::HashMap<u64, Restored>,
    payload: Vec<u8>,
}

/// A filed segment's items being read back.
pub(super) struct Unspooling<'u> {
    readers: &'u mut Readers,
    input: Option<BufReader<File>>,
    lineage: u64,
    items: u64,
    goes_on: bool,
}

impl Readers {
    /// Opens `filed` to read its items back, in its lineage.
    pub(super) fn open<'u>(&'u mut self, filed: &mut Filed) -> io::Result<Unspooling<'u>> {
        // An operator read back from disk begins its lineage again: what it held is let go.
        if filed.first {
            self.lineages.insert(filed.lineage, Restored::default());
        }
        let restored = self.lineages.get_mut(&filed.lineage).ok_or_else(|| {
            super::invalid("A spooled segment goes on from a lineage its emit never read")
        })?;
        restored.give_resident(std::mem::take(&mut filed.resident));
        let input = match &filed.path {
            Some(path) => Some(BufReader::with_capacity(BUFFER, File::open(path)?)),
            None => None,
        };
        Ok(Unspooling {
            readers: self,
            input,
            lineage: filed.lineage,
            items: filed.items,
            goes_on: filed.goes_on,
        })
    }
}

/// What [`Unspooling::next`] read: an item's bytes, which [`Unspooling::decode`] reads back.
pub(super) enum Next {
    Item,
    End,
}

impl Unspooling<'_> {
    /// Reads up to the next item, taking what its lineage released before it.
    pub(super) fn next(&mut self) -> io::Result<Next> {
        loop {
            let Some(input) = self.input.as_mut() else {
                return Ok(Next::End);
            };
            let mut kind = [0_u8];
            match input.read(&mut kind)? {
                0 => {
                    if self.items != 0 {
                        return Err(super::invalid("A spooled segment ends short of its items"));
                    }
                    self.input = None;
                    return Ok(Next::End);
                }
                _ => {}
            }
            let length = usize::try_from(varint(input)?)
                .map_err(|_| super::invalid("A spool record passes the address space"))?;
            let payload = &mut self.readers.payload;
            payload.clear();
            engine_allocation::reserve(payload, length)?;
            payload.resize(length, 0);
            input.read_exact(payload)?;
            match kind[0] {
                ITEM => {
                    if self.items == 0 {
                        return Err(super::invalid(
                            "A spooled segment holds more than its items",
                        ));
                    }
                    self.items -= 1;
                    return Ok(Next::Item);
                }
                RELEASED => {
                    let mut codes = &payload[..];
                    let mut released = Vec::new();
                    while !codes.is_empty() {
                        engine_allocation::reserve(&mut released, 1)?;
                        released.push(varint(&mut codes)?);
                    }
                    self.restored()?.release(&released)?;
                }
                _ => return Err(super::invalid("A spool record of a kind it does not know")),
            }
        }
    }

    fn restored(&mut self) -> io::Result<&mut Restored> {
        self.readers
            .lineages
            .get_mut(&self.lineage)
            .ok_or_else(|| super::invalid("A spooled segment's lineage is gone"))
    }

    /// The item whose bytes [`Self::next`] read, by its family's code.
    pub(super) fn decode<T: Spooled>(&mut self) -> io::Result<T> {
        let Readers {
            lineages, payload, ..
        } = &mut *self.readers;
        let restored = lineages
            .get_mut(&self.lineage)
            .ok_or_else(|| super::invalid("A spooled segment's lineage is gone"))?;
        let mut reader = Reader::new(payload, restored);
        let item = T::unspool(&mut reader)?;
        reader.finish()?;
        if payload.capacity() > KEPT {
            *payload = Vec::new();
        }
        Ok(item)
    }

    /// Whether every item was read.
    pub(super) fn done(&self) -> bool {
        self.items == 0
    }

    /// The bytes of the item [`Self::next`] read.
    pub(super) fn read(&self) -> usize {
        self.readers.payload.len()
    }
}

impl Drop for Unspooling<'_> {
    fn drop(&mut self) {
        // A lineage that goes on no further is forgotten with its last segment.
        if !self.goes_on {
            self.readers.lineages.remove(&self.lineage);
        }
    }
}
