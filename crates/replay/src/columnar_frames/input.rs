//! A columnar recording's bytes as its walkers read them: from its file or from memory,
//! through its last footer whose root verifies, a whole extent at a time, under one budget
//! for every walker's buffers.

use super::invalid;
use replay_columnar::envelope::{self, Extent, Kind};
use std::{
    collections::VecDeque,
    fs::File,
    io,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};
use zstd::zstd_safe::{self, DCtx};

/// The blocks a footer is searched for and a file's root hashed in.
const BLOCK: usize = 64 << 10;
/// The most bytes a zstd frame's header takes, its content size among them.
const FRAME_HEADER: usize = 18;

/// What a recording's walkers may hold.
#[derive(Clone)]
pub struct Limits {
    /// Every walker's buffers at once, shared by the recordings handed the same budget.
    pub budget: Arc<Budget>,
    /// The strings a walker's dictionary holds before it drops those no live cell names.
    pub dictionary_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            budget: Arc::new(Budget::new(2048 << 20)),
            dictionary_bytes: 32 << 20,
        }
    }
}

/// What walkers' buffers hold at once, admitted in arrival order: a walker waits until those
/// before it are admitted and its need fits, or no walker holds any; one that never waits
/// takes its share then or not at all.
pub struct Budget {
    limit: usize,
    queue: Mutex<Queue>,
    changed: Condvar,
}

#[derive(Default)]
struct Queue {
    held: usize,
    walkers: usize,
    /// The tickets of the walkers waiting, oldest first.
    waiting: VecDeque<u64>,
    next: u64,
}

/// A walker's share of a [`Budget`], given back when dropped.
pub(super) struct Permit {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            queue: Mutex::default(),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `bytes` of the budget, once every walker that asked before has its share and they
    /// fit; refused as interrupted once `stop` is set and [`Self::wake`] called.
    pub(super) fn enter(self: &Arc<Self>, bytes: usize, stop: &AtomicBool) -> io::Result<Permit> {
        let mut queue = self.lock();
        let ticket = queue.next;
        queue.next += 1;
        queue
            .waiting
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        queue.waiting.push_back(ticket);
        self.changed.notify_all();
        loop {
            if stop.load(Ordering::Relaxed) {
                queue.waiting.retain(|&waiting| waiting != ticket);
                self.changed.notify_all();
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "The walk stopped waiting for its buffers",
                ));
            }
            let fits = queue.walkers == 0
                || queue
                    .held
                    .checked_add(bytes)
                    .is_some_and(|held| held <= self.limit);
            if fits && queue.waiting.front() == Some(&ticket) {
                queue.waiting.pop_front();
                queue.held += bytes;
                queue.walkers += 1;
                self.changed.notify_all();
                return Ok(Permit {
                    budget: self.clone(),
                    bytes,
                });
            }
            queue = self
                .changed
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// `bytes` of the budget now, when no walker waits for its share and they fit, or no
    /// walker holds any; none otherwise.
    pub(super) fn try_enter(self: &Arc<Self>, bytes: usize) -> Option<Permit> {
        let mut queue = self.lock();
        let fits = queue.walkers == 0
            || queue
                .held
                .checked_add(bytes)
                .is_some_and(|held| held <= self.limit);
        if !fits || !queue.waiting.is_empty() {
            return None;
        }
        queue.held += bytes;
        queue.walkers += 1;
        Some(Permit {
            budget: self.clone(),
            bytes,
        })
    }
}

#[cfg(test)]
impl Budget {
    /// Wakes the walkers waiting, for those whose walk stopped to leave.
    pub(super) fn wake(&self) {
        drop(self.lock());
        self.changed.notify_all();
    }

    /// Waits until `count` walkers wait for their share.
    pub(crate) fn wait_for_waiters(&self, count: usize) {
        let mut queue = self.lock();
        while queue.waiting.len() < count {
            queue = self
                .changed
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The walkers holding a share.
    pub(crate) fn walkers(&self) -> usize {
        self.lock().walkers
    }
}

impl Permit {
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Grows its share to `bytes` now, when no walker waits for one and it fits beside the
    /// others', or no other walker holds any; false otherwise.
    pub(super) fn try_grow(&mut self, bytes: usize) -> bool {
        let mut queue = self.budget.lock();
        let others = queue.held - self.bytes;
        let fits = queue.walkers == 1
            || others
                .checked_add(bytes)
                .is_some_and(|held| held <= self.budget.limit);
        if !fits || !queue.waiting.is_empty() {
            return false;
        }
        queue.held = others + bytes;
        self.bytes = bytes;
        true
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut queue = self.budget.lock();
        queue.held -= self.bytes;
        queue.walkers -= 1;
        drop(queue);
        self.budget.changed.notify_all();
    }
}

/// Fills `bytes` from `file` at `at`.
fn read_at(file: &File, mut at: u64, mut bytes: &mut [u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        #[cfg(windows)]
        let count = {
            use std::os::windows::fs::FileExt;
            file.seek_read(bytes, at)?
        };
        #[cfg(unix)]
        let count = {
            use std::os::unix::fs::FileExt;
            file.read_at(bytes, at)?
        };
        if count == 0 {
            // The file changed under the reader: no damage of the recording it opened.
            return Err(io::Error::other(
                "The recording's file ends before its committed length",
            ));
        }
        at += count as u64;
        bytes = &mut bytes[count..];
    }
    Ok(())
}

/// Where a recording's bytes are.
pub(super) enum Held {
    File(File),
    Memory(Vec<u8>),
}

impl Held {
    fn len(&self) -> io::Result<u64> {
        match self {
            Self::File(file) => Ok(file.metadata()?.len()),
            Self::Memory(bytes) => Ok(bytes.len() as u64),
        }
    }

    /// Fills `out` from `at`, within the bytes held.
    fn read(&self, at: u64, out: &mut [u8]) -> io::Result<()> {
        match self {
            Self::File(file) => read_at(file, at, out),
            Self::Memory(bytes) => {
                out.copy_from_slice(&bytes[at as usize..][..out.len()]);
                Ok(())
            }
        }
    }

    /// The digest of `length` bytes from `at`, within the bytes held, as
    /// [`envelope::digest`] makes it; a file's are read through `block`.
    fn digest(&self, at: u64, length: u64, block: &mut [u8]) -> io::Result<[u8; 16]> {
        match self {
            Self::Memory(bytes) => Ok(envelope::digest(&bytes[at as usize..][..length as usize])),
            Self::File(file) => {
                let mut hasher = blake3::Hasher::new();
                let mut done = 0;
                while done < length {
                    let count = (length - done).min(block.len() as u64) as usize;
                    read_at(file, at + done, &mut block[..count])?;
                    hasher.update(&block[..count]);
                    done += count as u64;
                }
                Ok(hasher.finalize().as_bytes()[..16]
                    .try_into()
                    .expect("sixteen bytes"))
            }
        }
    }
}

/// The last footer whose root verifies, as [`envelope::last_root`] finds it, read a block at
/// a time from the end: its root's envelope, and the recording's length through the footer.
fn last_root(held: &Held) -> io::Result<(Extent, u64)> {
    let length = held.len()?;
    let header = envelope::HEADER_BYTES as u64;
    let footer = envelope::FOOTER_BYTES as u64;
    if length < header + footer {
        return Err(invalid("Not a version 3 H4COLUMN recording"));
    }
    let mut start = [0; envelope::HEADER_BYTES];
    held.read(0, &mut start)?;
    if &start[..8] != envelope::MAGIC || start[8..] != envelope::VERSION.to_le_bytes() {
        return Err(invalid("Not a version 3 H4COLUMN recording"));
    }
    let magic = envelope::FOOTER_MAGIC.len();
    let mut block = engine_allocation::filled(BLOCK, 0_u8)?;
    let mut hashed = engine_allocation::filled(BLOCK, 0_u8)?;
    // A root sits right before its footer. All candidates' roots together are hashed at most
    // twice the file's length: a tail of footer-shaped bytes cannot cost more.
    let (mut work, mut stop) = (0_u64, length);
    while stop > header {
        let first = stop.saturating_sub(BLOCK as u64).max(header);
        let count = (stop - first) as usize;
        held.read(first, &mut block[..count])?;
        for at in (0..(count + 1).saturating_sub(magic)).rev() {
            if block[at..at + magic] != *envelope::FOOTER_MAGIC {
                continue;
            }
            let end = first + (at + magic) as u64;
            if end < header + footer {
                continue;
            }
            let mut bytes = [0; envelope::FOOTER_BYTES];
            held.read(end - footer, &mut bytes)?;
            let root = Extent {
                kind: Kind::Root,
                run: 0,
                stream: 0,
                first: 0,
                hours: 0,
                offset: u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")),
                stored: u64::from_le_bytes(bytes[8..16].try_into().expect("eight bytes")),
                raw: 0,
                digest: bytes[16..32].try_into().expect("sixteen bytes"),
            };
            if root.offset < header || root.offset.checked_add(root.stored) != Some(end - footer) {
                continue;
            }
            work += root.stored;
            if work > 2 * length {
                return Err(invalid(
                    "Recovering the recording's footer exceeds its work bound",
                ));
            }
            if held.digest(root.offset, root.stored, &mut hashed)? == root.digest {
                return Ok((root, end));
            }
        }
        if first == header {
            break;
        }
        // Blocks overlap by a magic less one byte, so one across their boundary is seen.
        stop = first + magic as u64 - 1;
    }
    Err(invalid("The recording has no footer whose root verifies"))
}

/// A recording's bytes through its last footer whose root verifies.
pub(super) struct Source {
    held: Held,
    end: u64,
}

impl Source {
    /// What `held` holds through its last footer whose root verifies, and that root.
    pub(super) fn open(mut held: Held) -> io::Result<(Self, Extent)> {
        let (root, end) = last_root(&held)?;
        // A torn tail after the last valid footer is no part of the recording.
        if let Held::Memory(bytes) = &mut held {
            bytes.truncate(end as usize);
        }
        Ok((Self { held, end }, root))
    }

    pub(super) fn len(&self) -> u64 {
        self.end
    }

    /// Whether extents are read from a file rather than memory.
    pub(super) fn reads_file(&self) -> bool {
        matches!(self.held, Held::File(_))
    }

    /// `extent`'s stored length once its place is checked.
    fn place(&self, extent: &Extent) -> io::Result<usize> {
        let placed = extent.offset >= envelope::HEADER_BYTES as u64
            && extent
                .offset
                .checked_add(extent.stored)
                .is_some_and(|end| end <= self.end - envelope::FOOTER_BYTES as u64);
        if !placed {
            return Err(invalid("lies outside the recording's extents"));
        }
        usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())
    }

    /// `extent`'s raw length once its place is checked and its frame's header declares it:
    /// what buffers are sized by before its stored bytes are read.
    pub(super) fn raw_length(&self, extent: &Extent) -> io::Result<usize> {
        let mut header = [0; FRAME_HEADER];
        let header = &mut header[..self.place(extent)?.min(FRAME_HEADER)];
        self.held.read(extent.offset, header)?;
        frame_length(extent, header)
    }

    /// `extent`'s stored bytes once their place is checked; a file's are read into `buffer`.
    fn stored<'a>(&'a self, extent: &Extent, buffer: &'a mut Vec<u8>) -> io::Result<&'a [u8]> {
        let length = self.place(extent)?;
        match &self.held {
            Held::Memory(bytes) => Ok(&bytes[extent.offset as usize..][..length]),
            Held::File(file) => {
                if buffer.len() < length {
                    engine_allocation::resize(buffer, length, 0)?;
                }
                read_at(file, extent.offset, &mut buffer[..length])?;
                Ok(&buffer[..length])
            }
        }
    }
}

/// The raw length `frame`'s header declares, refused unless `extent`, a root aside, declares
/// it too.
fn frame_length(extent: &Extent, frame: &[u8]) -> io::Result<usize> {
    match zstd_safe::get_frame_content_size(frame) {
        Ok(Some(raw)) if extent.kind == Kind::Root || raw == extent.raw => {
            usize::try_from(raw).map_err(|_| engine_allocation::oom())
        }
        _ => Err(invalid("differs from its frame's length")),
    }
}

/// A zstd context, and the buffer a file's stored extents are read into, reused extent after
/// extent.
pub(super) struct Decoder {
    context: DCtx<'static>,
    stored: Vec<u8>,
}

impl Decoder {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            context: DCtx::try_create().ok_or_else(engine_allocation::oom)?,
            stored: Vec::new(),
        })
    }

    /// The bytes it allocated.
    pub(super) fn held(&self) -> usize {
        self.context() + self.stored.capacity()
    }

    /// The bytes its zstd context allocated.
    pub(super) fn context(&self) -> usize {
        self.context.sizeof()
    }

    /// The stored bytes it reads an extent into without growing.
    pub(super) fn stored(&self) -> usize {
        self.stored.capacity()
    }

    pub(super) fn reserve(&mut self, stored: usize) -> io::Result<()> {
        let more = stored.saturating_sub(self.stored.len());
        engine_allocation::reserve_exact(&mut self.stored, more)
    }

    /// Frees the buffer it reads into.
    pub(super) fn release(&mut self) {
        self.stored = Vec::new();
    }

    /// Appends `extent`'s raw bytes to `out`, its place, digest and frame's length checked
    /// before it is decoded in one pass.
    pub(super) fn append(
        &mut self,
        source: &Source,
        extent: &Extent,
        out: &mut Vec<u8>,
    ) -> io::Result<()> {
        let stored = source.stored(extent, &mut self.stored)?;
        if envelope::digest(stored) != extent.digest {
            return Err(invalid("differs from its digest"));
        }
        let raw = frame_length(extent, stored)?;
        engine_allocation::reserve(out, raw)?;
        let start = out.len();
        let mut into = std::io::Cursor::new(&mut *out);
        into.set_position(start as u64);
        let written = self
            .context
            .decompress(&mut into, stored)
            .map_err(|code| invalid(zstd_safe::get_error_name(code)))?;
        if written != raw {
            return Err(invalid("decompresses to another length"));
        }
        Ok(())
    }
}
