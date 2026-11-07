//! Where a family's items wait for its emit: a file of the run's folder for each family and
//! segment, `<folder>/<lane>-<family>/<segment>`, its walk writing each item as the operator
//! pushes it and its emit reading them back in order once the walk filed the segment, then
//! removing it. Parked operators spill to files of their own there.
//!
//! A file is a sequence of frames, each its raw and stored lengths and its bytes, compressed
//! or, where compressing would not shrink them, as they are. Their raw bytes are a sequence of
//! records, each its kind, its length and its bytes: an item, or the shared allocations its
//! writer forgot after it ([`replay_spool::Shared::released`]). A walk's writer keeps its
//! shared allocations across the segments its operator goes on into, its lineage, and the
//! emit's reader keeps them for the lineage alike.

use engine_allocation::zstd::{self, Compressor, Decompressor, Holder};
use replay_spool::{Reader, Restored, Shared, Spooled, Writer};
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const ITEM: u8 = 0;
const RELEASED: u8 = 1;
/// What a walk buffers of a file before writing it out as a frame, and an emit reads at once.
const BUFFER: usize = 64 << 10;
/// An item's bytes past which their buffer is let go once written or read, not kept for the
/// next: such items are rare, and one walk's or emit's buffer would hold the largest.
const KEPT: usize = 16 * BUFFER;
/// A record's header at most: its kind and its length.
const RECORD_HEADER: usize = 11;
/// A frame's header at most: its raw and stored lengths.
const FRAME_HEADER: usize = 20;
/// Frames' zstd level, measured on a long campaign's spool.
const LEVEL: i32 = 1;

/// `options` for a file read back soon and removed: one the system keeps from the disk while it
/// has the memory.
pub(crate) fn temporary(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_ATTRIBUTE_TEMPORARY.
        options.attributes(0x100);
    }
    options
}

/// The run's spool folder, emptied of what an earlier run left as it opens and removed once
/// dropped.
pub(super) struct Folder {
    root: PathBuf,
    lanes: Vec<PathBuf>,
    next: AtomicU64,
    lineages: AtomicU64,
    counts: Arc<Counts>,
}

/// What a spool holds on disk, as stored and as its bytes were before compressing.
#[derive(Default)]
struct Counts {
    stored: Held,
    raw: Held,
}

/// Bytes held now, at most, and written in all.
#[derive(Default)]
struct Held {
    now: AtomicU64,
    peak: AtomicU64,
    written: AtomicU64,
}

impl Held {
    fn wrote(&self, bytes: u64) {
        self.written.fetch_add(bytes, Ordering::Relaxed);
        let now = self.now.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak.fetch_max(now, Ordering::Relaxed);
    }

    fn removed(&self, bytes: u64) {
        self.now.fetch_sub(bytes, Ordering::Relaxed);
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

    /// Counts into `stats` the most it held on disk at once and what it wrote in all, stored
    /// and raw.
    pub(super) fn count(&self, stats: &mut super::Stats) {
        let Counts { stored, raw } = &*self.counts;
        stats.spool_bytes = stored.peak.load(Ordering::Relaxed);
        stats.spool_raw_bytes = raw.peak.load(Ordering::Relaxed);
        stats.spooled = stored.written.load(Ordering::Relaxed);
        stats.spooled_raw = raw.written.load(Ordering::Relaxed);
    }

    /// Writes `bytes`, a parked operator, to a file of its own, removed once dropped.
    pub(super) fn spill(&self, bytes: &[u8]) -> io::Result<Spilled> {
        let path = self.root.join(format!(
            "carry-{}",
            self.next.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = temporary(OpenOptions::new().write(true).create_new(true)).open(&path)?;
        let mut spilled = Spilled {
            counts: Arc::clone(&self.counts),
            path,
            length: bytes.len(),
            stored: 0,
        };
        self.counts.raw.wrote(bytes.len() as u64);
        for raw in bytes.chunks(BUFFER) {
            let stored = frame(&mut file, raw)?;
            self.counts.stored.wrote(stored);
            spilled.stored += stored;
        }
        Ok(spilled)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // Best effort: a later run empties it as it opens.
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A parked operator in a [`Folder`]: its bytes, and those stored.
pub(super) struct Spilled {
    counts: Arc<Counts>,
    path: PathBuf,
    length: usize,
    stored: u64,
}

impl Spilled {
    pub(super) fn read(&self, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        engine_allocation::reserve_exact(out, self.length)?;
        out.resize(self.length, 0);
        let mut input = File::open(&self.path)?;
        let mut frames = Frames::default();
        frames.start()?;
        let mut at = 0_usize;
        while let Some((raw, stored)) = frames.next(&mut input)? {
            let end = at
                .checked_add(raw)
                .filter(|&end| end <= self.length)
                .ok_or_else(|| super::invalid("A spill reads back at another length"))?;
            unpack(stored, &mut out[at..end])?;
            at = end;
        }
        if at != self.length {
            return Err(super::invalid("A spill reads back at another length"));
        }
        Ok(())
    }
}

impl Drop for Spilled {
    fn drop(&mut self) {
        self.counts.raw.removed(self.length as u64);
        self.counts.stored.removed(self.stored);
        let _ = fs::remove_file(&self.path);
    }
}

static FRAMES: Holder = Holder::new("zstd: spool frames");
static READS: Holder = Holder::new("zstd: spool reads");

thread_local! {
    // One of each per worker, never per family, segment or lineage.
    static COMPRESSOR: RefCell<Option<Compressor>> = const { RefCell::new(None) };
    static DECOMPRESSOR: RefCell<Option<Decompressor>> = const { RefCell::new(None) };
    static PACKED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Writes `raw` to `file` as a frame, compressed by the thread's context into its buffer: the
/// bytes it stored.
fn frame(file: &mut File, raw: &[u8]) -> io::Result<u64> {
    if raw.is_empty() {
        return Ok(0);
    }
    PACKED.with_borrow_mut(|packed| packed_frame(packed, file, raw))
}

fn packed_frame(packed: &mut Vec<u8>, file: &mut File, raw: &[u8]) -> io::Result<u64> {
    let bound = FRAME_HEADER + zstd::bound(raw.len());
    if packed.len() < bound {
        engine_allocation::resize(packed, bound, 0)?;
    }
    let body = &mut packed[FRAME_HEADER..];
    let mut stored = COMPRESSOR.with_borrow_mut(|context| {
        if context.is_none() {
            *context = Some(Compressor::new(&FRAMES, LEVEL)?);
        }
        let context = context.as_mut().expect("the thread's context is made");
        context
            .compress(&mut *body, raw)
            .map_err(zstd::Error::invalid)
    })?;
    if stored >= raw.len() {
        body[..raw.len()].copy_from_slice(raw);
        stored = raw.len();
    }
    let mut head = [0; FRAME_HEADER];
    let used = varint(&mut head, 0, raw.len() as u64);
    let used = varint(&mut head, used, stored as u64);
    let from = FRAME_HEADER - used;
    packed[from..FRAME_HEADER].copy_from_slice(&head[..used]);
    file.write_all(&packed[from..FRAME_HEADER + stored])?;
    if packed.len() > KEPT {
        *packed = Vec::new();
    }
    Ok((used + stored) as u64)
}

/// A frame's `stored` bytes as the `out` they hold, by the thread's context: as they are when
/// as long.
fn unpack(stored: &[u8], out: &mut [u8]) -> io::Result<()> {
    if stored.len() == out.len() {
        out.copy_from_slice(stored);
        return Ok(());
    }
    let written = DECOMPRESSOR.with_borrow_mut(|context| {
        if context.is_none() {
            *context = Some(Decompressor::new(&READS)?);
        }
        let context = context.as_mut().expect("the thread's context is made");
        context
            .decompress(out, stored)
            .map_err(zstd::Error::invalid)
    })?;
    if written != out.len() {
        return Err(super::invalid("A spooled frame holds another length"));
    }
    Ok(())
}

/// A file's frames as they are read back, a buffer's bytes at once.
#[derive(Default)]
struct Frames {
    stored: Vec<u8>,
    at: usize,
    held: usize,
}

impl Frames {
    /// Ready to read a file from its start.
    fn start(&mut self) -> io::Result<()> {
        (self.at, self.held) = (0, 0);
        if self.stored.len() < BUFFER + FRAME_HEADER {
            engine_allocation::resize(&mut self.stored, BUFFER + FRAME_HEADER, 0)?;
        }
        Ok(())
    }

    /// Holds at least `count` bytes from `at`, but where `input` ends first: the bytes it
    /// holds from there.
    fn hold(&mut self, input: &mut File, count: usize) -> io::Result<usize> {
        if self.held - self.at < count {
            self.stored.copy_within(self.at..self.held, 0);
            self.held -= self.at;
            self.at = 0;
            if self.stored.len() < count {
                engine_allocation::resize(&mut self.stored, count, 0)?;
            }
            while self.held < count {
                let read = input.read(&mut self.stored[self.held..])?;
                if read == 0 {
                    break;
                }
                self.held += read;
            }
        }
        Ok(self.held - self.at)
    }

    /// The next frame of `input`: its raw length and stored bytes, or none where it ends.
    fn next(&mut self, input: &mut File) -> io::Result<Option<(usize, &[u8])>> {
        if self.hold(input, FRAME_HEADER)? == 0 {
            return Ok(None);
        }
        let ends = || super::invalid("A spooled file ends inside a frame");
        let bytes = &self.stored[self.at..self.held];
        let (raw, first) = take_varint(bytes)?.ok_or_else(ends)?;
        let (stored, second) = take_varint(&bytes[first..])?.ok_or_else(ends)?;
        let (raw, stored) = (length(raw)?, length(stored)?);
        let head = first + second;
        let whole = head.checked_add(stored).ok_or_else(passes)?;
        if self.hold(input, whole)? < whole {
            return Err(ends());
        }
        let from = self.at + head;
        self.at += whole;
        Ok(Some((raw, &self.stored[from..from + stored])))
    }

    /// Lets go of a buffer a rare large frame grew.
    fn shrink(&mut self) {
        if self.stored.len() > KEPT {
            self.stored.truncate(BUFFER + FRAME_HEADER);
            self.stored.shrink_to(BUFFER + FRAME_HEADER);
        }
    }
}

fn passes() -> io::Error {
    super::invalid("A spool record passes the address space")
}

fn length(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| passes())
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
    /// The segment's items, and their bytes, of which the folder's counts do not hold the last
    /// `unreported`: shared by every walk, they are told as the buffer goes out.
    items: u64,
    length: u64,
    unreported: u64,
    /// The bytes the segment's file stored, all told.
    stored: u64,
}

struct Open {
    file: File,
    path: PathBuf,
    buffer: Vec<u8>,
}

impl Open {
    /// Writes out what it buffered as a frame: the bytes it stored.
    fn out(&mut self) -> io::Result<u64> {
        let stored = frame(&mut self.file, &self.buffer)?;
        self.buffer.clear();
        Ok(stored)
    }
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
    stored: u64,
    lineage: u64,
    /// It begins its lineage.
    pub(super) first: bool,
    /// Its lineage goes on into a later segment's file.
    pub(super) goes_on: bool,
}

impl Drop for Filed {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            self.counts.raw.removed(self.length);
            self.counts.stored.removed(self.stored);
            let _ = fs::remove_file(path);
        }
    }
}

/// Writes `value` into `out` from `at`: where it ends.
fn varint(out: &mut [u8], mut at: usize, mut value: u64) -> usize {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out[at] = byte;
            return at + 1;
        }
        out[at] = byte | 0x80;
        at += 1;
    }
}

/// The varint at the start of `bytes`, and its bytes; none while `bytes` holds less than all
/// of it.
fn take_varint(bytes: &[u8]) -> io::Result<Option<(u64, usize)>> {
    let mut value = 0_u64;
    for (at, &byte) in bytes.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (7 * at);
        if byte & 0x80 == 0 {
            return Ok(Some((value, at + 1)));
        }
    }
    if bytes.len() >= 10 {
        return Err(super::invalid("A spool varint runs too long"));
    }
    Ok(None)
}

fn record_header(out: &mut Vec<u8>, kind: u8, length: usize) {
    let mut header = [kind; RECORD_HEADER];
    let used = varint(&mut header, 1, length as u64);
    out.extend_from_slice(&header[..used]);
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
            unreported: 0,
            stored: 0,
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
        self.shared.sweep_recent()?;
        self.released.clear();
        for &code in self.shared.released().iter() {
            put_varint(&mut self.released, code)?;
        }
        self.shared.released().clear();
        if self.open.is_none() {
            let path = folder.lanes[lane].join(at.to_string());
            let file =
                temporary(OpenOptions::new().write(true).create(true).truncate(true)).open(&path)?;
            self.open = Some(Open {
                file,
                path,
                buffer: engine_allocation::vec(BUFFER)?,
            });
        }
        let Self {
            scratch,
            released,
            open,
            stored,
            ..
        } = self;
        let open = open.as_mut().expect("the file is open");
        let mut counted = |bytes: u64| {
            *stored += bytes;
            folder.counts.stored.wrote(bytes);
        };
        let mut written = 0;
        let mut out = false;
        for (kind, bytes) in [(ITEM, &*scratch), (RELEASED, &*released)] {
            if kind == RELEASED && bytes.is_empty() {
                continue;
            }
            if open.buffer.len() + bytes.len() + RECORD_HEADER > BUFFER {
                counted(open.out()?);
                out = true;
            }
            let before = open.buffer.len();
            record_header(&mut open.buffer, kind, bytes.len());
            written += (open.buffer.len() - before + bytes.len()) as u64;
            if bytes.len() > BUFFER / 2 {
                counted(open.out()?);
                counted(frame(&mut open.file, bytes)?);
                out = true;
            } else {
                open.buffer.extend_from_slice(bytes);
            }
        }
        self.items += 1;
        self.length += written;
        self.unreported += written;
        if out {
            folder.counts.raw.wrote(std::mem::take(&mut self.unreported));
        }
        if self.scratch.capacity() > KEPT {
            self.scratch = Vec::new();
        }
        Ok(())
    }

    /// The bytes spooled of the segment so far, raw.
    pub(super) fn length(&self) -> u64 {
        self.length
    }

    /// Writes out what it buffered of the segment.
    pub(super) fn flush(&mut self, folder: &Folder) -> io::Result<()> {
        if let Some(open) = &mut self.open
            && !open.buffer.is_empty()
        {
            let stored = open.out()?;
            self.stored += stored;
            folder.counts.stored.wrote(stored);
        }
        folder.counts.raw.wrote(std::mem::take(&mut self.unreported));
        Ok(())
    }

    /// Files what it spooled of the segment, [`Self::flush`]ed, for its emit; `goes_on` when
    /// its operator goes on into the segment after, writing on in this lineage.
    pub(super) fn file(&mut self, folder: &Folder, goes_on: bool) -> Filed {
        folder.counts.raw.wrote(std::mem::take(&mut self.unreported));
        let path = self.open.take().map(|open| open.path);
        let filed = Filed {
            counts: Arc::clone(&folder.counts),
            path,
            resident: self.shared.take_resident(),
            items: std::mem::take(&mut self.items),
            length: std::mem::take(&mut self.length),
            stored: std::mem::take(&mut self.stored),
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
        let reported = std::mem::take(&mut self.length) - std::mem::take(&mut self.unreported);
        folder.counts.raw.removed(reported);
        folder.counts.stored.removed(std::mem::take(&mut self.stored));
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

fn put_varint(out: &mut Vec<u8>, value: u64) -> io::Result<()> {
    engine_allocation::reserve(out, 10)?;
    let mut bytes = [0; 10];
    let used = varint(&mut bytes, 0, value);
    out.extend_from_slice(&bytes[..used]);
    Ok(())
}

/// A record's header at the start of `bytes`: its kind, its length and the header's bytes; None
/// while `bytes` holds less than all of it.
fn header(bytes: &[u8]) -> io::Result<Option<(u8, usize, usize)>> {
    let Some((&kind, rest)) = bytes.split_first() else {
        return Ok(None);
    };
    let Some((length, used)) = take_varint(rest)? else {
        return Ok(None);
    };
    Ok(Some((kind, self::length(length)?, used + 1)))
}

/// A family's emit's readers: each lineage's shared allocations, the raw bytes held of the file
/// being read and the frames it reads them from, and the codes a record released.
#[derive(Default)]
pub(super) struct Readers {
    lineages: std::collections::HashMap<u64, Restored>,
    buffer: Vec<u8>,
    frames: Frames,
    codes: Vec<u64>,
}

/// A filed segment's items being read back: its file's raw bytes held from `start` to `end` of
/// the readers' buffer, among them the item [`Unspooling::next`] read, decoded where it lies.
pub(super) struct Unspooling<'u> {
    readers: &'u mut Readers,
    input: Option<File>,
    start: usize,
    end: usize,
    item: (usize, usize),
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
            Some(path) => Some(File::open(path)?),
            None => None,
        };
        if input.is_some() {
            // A frame's raw bytes, after what is left of a record's header.
            if self.buffer.len() < BUFFER + RECORD_HEADER {
                engine_allocation::resize(&mut self.buffer, BUFFER + RECORD_HEADER, 0)?;
            }
            self.frames.start()?;
        }
        Ok(Unspooling {
            readers: self,
            input,
            start: 0,
            end: 0,
            item: (0, 0),
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
    /// Holds at least `count` raw bytes from `start`, but where the file ends first: the bytes
    /// it holds from there.
    fn hold(&mut self, count: usize) -> io::Result<usize> {
        while self.end - self.start < count {
            let Some(input) = self.input.as_mut() else {
                break;
            };
            let Readers { buffer, frames, .. } = &mut *self.readers;
            let Some((raw, stored)) = frames.next(input)? else {
                break;
            };
            // What is held moves to the front, and the buffer grows for a frame past it.
            buffer.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
            let end = self.end.checked_add(raw).ok_or_else(passes)?;
            if buffer.len() < end {
                engine_allocation::resize(buffer, end, 0)?;
            }
            unpack(stored, &mut buffer[self.end..end])?;
            self.end = end;
        }
        Ok(self.end - self.start)
    }

    /// Reads up to the next item, taking what its lineage released before it.
    pub(super) fn next(&mut self) -> io::Result<Next> {
        loop {
            if self.input.is_none() {
                return Ok(Next::End);
            }
            let held = self.hold(RECORD_HEADER)?;
            if held == 0 {
                if self.items != 0 {
                    return Err(super::invalid("A spooled segment ends short of its items"));
                }
                self.input = None;
                return Ok(Next::End);
            }
            let Some((kind, length, head)) = header(&self.readers.buffer[self.start..self.end])?
            else {
                return Err(super::invalid("A spooled segment ends inside a record"));
            };
            let whole = head.checked_add(length).ok_or_else(passes)?;
            if self.hold(whole)? < whole {
                return Err(super::invalid("A spooled segment ends inside a record"));
            }
            let payload = (self.start + head, self.start + whole);
            self.start += whole;
            match kind {
                ITEM => {
                    if self.items == 0 {
                        return Err(super::invalid(
                            "A spooled segment holds more than its items",
                        ));
                    }
                    self.items -= 1;
                    self.item = payload;
                    return Ok(Next::Item);
                }
                RELEASED => {
                    let Readers {
                        lineages,
                        buffer,
                        codes,
                        ..
                    } = &mut *self.readers;
                    codes.clear();
                    let mut bytes = &buffer[payload.0..payload.1];
                    while !bytes.is_empty() {
                        let (code, used) = take_varint(bytes)?.ok_or_else(|| {
                            super::invalid("A released allocation's code runs on")
                        })?;
                        engine_allocation::reserve(codes, 1)?;
                        codes.push(code);
                        bytes = &bytes[used..];
                    }
                    lineages
                        .get_mut(&self.lineage)
                        .ok_or_else(|| super::invalid("A spooled segment's lineage is gone"))?
                        .release(codes)?;
                }
                _ => return Err(super::invalid("A spool record of a kind it does not know")),
            }
        }
    }

    /// The item whose bytes [`Self::next`] read, by its family's code.
    pub(super) fn decode<T: Spooled>(&mut self) -> io::Result<T> {
        let Readers {
            lineages, buffer, ..
        } = &mut *self.readers;
        let restored = lineages
            .get_mut(&self.lineage)
            .ok_or_else(|| super::invalid("A spooled segment's lineage is gone"))?;
        let mut reader = Reader::new(&buffer[self.item.0..self.item.1], restored);
        let item = T::unspool(&mut reader)?;
        reader.finish()?;
        Ok(item)
    }

    /// Whether every item was read.
    pub(super) fn done(&self) -> bool {
        self.items == 0
    }

    /// The bytes of the item [`Self::next`] read.
    pub(super) fn read(&self) -> usize {
        self.item.1 - self.item.0
    }
}

impl Drop for Unspooling<'_> {
    fn drop(&mut self) {
        // Buffers a rare large record grew are let go.
        let buffer = &mut self.readers.buffer;
        if buffer.len() > KEPT {
            buffer.truncate(BUFFER + RECORD_HEADER);
            buffer.shrink_to(BUFFER + RECORD_HEADER);
        }
        self.readers.frames.shrink();
        // A lineage that goes on no further is forgotten with its last segment.
        if !self.goes_on {
            self.readers.lineages.remove(&self.lineage);
        }
    }
}
