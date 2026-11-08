//! A columnar recording's bytes as its walkers read them: from its file or from memory,
//! through its last footer whose root verifies, a whole extent at a time.

use super::invalid;
use engine_allocation::zstd::{Decompressor, Error, Holder};
use replay_columnar::{
    container::{Footers, ReadAt},
    envelope::{self, Extent, Kind},
};
use std::{fs::File, io, mem::MaybeUninit, ops::Range};
use zstd::zstd_safe;

/// The most bytes a zstd frame's header takes, its content size among them.
const FRAME_HEADER: usize = 18;

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
}

impl ReadAt for Held {
    fn len(&self) -> io::Result<u64> {
        Held::len(self)
    }

    fn read_at(&self, at: u64, out: &mut [u8]) -> io::Result<()> {
        self.read(at, out)
    }
}

/// The last footer whose root verifies ([`Footers`]): its root's envelope, and the recording's
/// length through the footer.
fn last_root(held: &Held) -> io::Result<(Extent, u64)> {
    Footers::new(held)?
        .next()?
        .ok_or_else(|| invalid("The recording has no footer whose root verifies"))
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

    /// The bytes it holds in memory, through its last valid footer; none for a file.
    pub(super) fn memory(&self) -> Option<&[u8]> {
        match &self.held {
            Held::Memory(bytes) => Some(bytes),
            Held::File(_) => None,
        }
    }

    /// Whether extents are read from a file rather than memory.
    pub(super) fn reads_file(&self) -> bool {
        matches!(self.held, Held::File(_))
    }

    /// `extent`'s stored length once its place is checked.
    pub(super) fn place(&self, extent: &Extent) -> io::Result<usize> {
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

    /// A page unit's raw length once its place is checked and each of its `chunks`' frame headers
    /// declares that chunk's: what buffers are sized by before it decodes. The headers are read
    /// from `stored`, its stored bytes, when given.
    pub(super) fn unit_length(&self, extent: &Extent, chunks: &[u32], stored: Option<&[u8]>) -> io::Result<usize> {
        self.place(extent)?;
        let mut at = 0_u64;
        for (index, &length) in (0_u64..).zip(chunks) {
            let mut header = [0; FRAME_HEADER];
            let header = &mut header[..(length as usize).min(FRAME_HEADER)];
            match stored {
                Some(stored) => header.copy_from_slice(&stored[at as usize..][..header.len()]),
                None => self.held.read(extent.offset + at, header)?,
            }
            chunk_header(header, envelope::chunk_raw(extent.raw, index))?;
            at += u64::from(length);
        }
        usize::try_from(extent.raw).map_err(|_| engine_allocation::oom())
    }

    /// `out.len()` stored bytes of `extent` from `at` within it, once its place is checked; the
    /// digest is not: the caller checks the extent whole once.
    pub(super) fn stored_at(&self, extent: &Extent, at: u64, out: &mut [u8]) -> io::Result<()> {
        let length = self.place(extent)? as u64;
        let end = at.checked_add(out.len() as u64).filter(|&end| end <= length);
        if end.is_none() {
            return Err(invalid("reads past its extent"));
        }
        self.held.read(extent.offset + at, out)
    }

    /// `extent`'s stored bytes once their place and digest are checked; a file's are read into
    /// `buffer`.
    pub(super) fn stored_checked<'a>(&'a self, extent: &Extent, buffer: &'a mut Vec<u8>) -> io::Result<&'a [u8]> {
        let stored = self.stored(extent, buffer)?;
        check(extent, stored)?;
        Ok(stored)
    }

    /// `extent`'s stored bytes in memory once their place is checked; a file's are not read.
    pub(super) fn memory_stored(&self, extent: &Extent) -> io::Result<&[u8]> {
        let length = self.place(extent)?;
        match &self.held {
            Held::Memory(bytes) => Ok(&bytes[extent.offset as usize..][..length]),
            Held::File(_) => Err(io::Error::other("A unit of a file was not gathered")),
        }
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

/// Why an extent whose stored bytes go on past its one frame is refused.
pub(super) const TRAILING: &str = "holds bytes past its frame";

/// Refuses `stored`, `extent`'s bytes, unless they hold its digest.
pub(super) fn check(extent: &Extent, stored: &[u8]) -> io::Result<()> {
    if envelope::digest(stored) != extent.digest {
        return Err(invalid("differs from its digest"));
    }
    Ok(())
}

static EXTENTS: Holder = Holder::new("zstd: walker extents");

/// A zstd context, and the buffer a file's stored extents are read into, reused extent after
/// extent.
pub(super) struct Decoder {
    context: Decompressor,
    stored: Vec<u8>,
}

impl Decoder {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            context: Decompressor::new(&EXTENTS)?,
            stored: Vec::new(),
        })
    }

    /// The bytes its zstd context allocated.
    pub(super) fn context(&self) -> usize {
        self.context.size()
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
        self.append_with(source, extent, None, out)
    }

    /// [`Self::append`] of an extent compressed with `base`'s raw bytes as its prefix, when it
    /// was.
    pub(super) fn append_with(
        &mut self,
        source: &Source,
        extent: &Extent,
        base: Option<&[u8]>,
        out: &mut Vec<u8>,
    ) -> io::Result<()> {
        let stored = source.stored(extent, &mut self.stored)?;
        decode(&mut self.context, extent, stored, base, out)
    }

    /// Reads into its buffer the stored bytes of `spans`, extents placed in a recording read
    /// from a file as (offset, stored length, slot), one read per run of them lying end to
    /// end: where each one's bytes begin there, at its slot of `starts`.
    pub(super) fn gather(
        &mut self,
        source: &Source,
        spans: &mut [(u64, usize, usize)],
        starts: &mut [usize],
    ) -> io::Result<()> {
        let Held::File(file) = &source.held else {
            return Err(io::Error::other("A recording held in memory is not read"));
        };
        let total = spans
            .iter()
            .try_fold(0_usize, |total, &(_, length, _)| total.checked_add(length))
            .ok_or_else(engine_allocation::oom)?;
        if self.stored.len() < total {
            engine_allocation::resize(&mut self.stored, total, 0)?;
        }
        spans.sort_unstable();
        let mut at = 0;
        for run in spans.chunk_by(|a, b| a.0 + a.1 as u64 == b.0) {
            let length = run.iter().map(|&(_, length, _)| length).sum::<usize>();
            read_at(file, run[0].0, &mut self.stored[at..][..length])?;
            for &(_, length, slot) in run {
                starts[slot] = at;
                at += length;
            }
        }
        Ok(())
    }

    /// The stored bytes of `extent`, gathered from `start`.
    pub(super) fn gathered(&self, extent: &Extent, start: usize) -> &[u8] {
        &self.stored[start..][..extent.stored as usize]
    }

    /// Appends page unit `extent`'s raw bytes, stored as `chunks`, to `out`: its place and digest
    /// checked, then each chunk decoded in turn.
    pub(super) fn append_unit(
        &mut self,
        source: &Source,
        extent: &Extent,
        chunks: &[u32],
        out: &mut Vec<u8>,
    ) -> io::Result<()> {
        let stored = source.stored(extent, &mut self.stored)?;
        decode_unit(&mut self.context, extent, stored, chunks, out)
    }

    /// [`Self::append_unit`] of a unit gathered from `start`.
    pub(super) fn append_unit_gathered(
        &mut self,
        extent: &Extent,
        chunks: &[u32],
        start: usize,
        out: &mut Vec<u8>,
    ) -> io::Result<()> {
        let stored = &self.stored[start..][..extent.stored as usize];
        decode_unit(&mut self.context, extent, stored, chunks, out)
    }

    /// Decodes one chunk, `stored`, into `out`, which its frame fills exactly.
    pub(super) fn decode_chunk(&mut self, stored: &[u8], out: &mut [MaybeUninit<u8>]) -> io::Result<()> {
        decode_chunk(&mut self.context, stored, out)
    }
}

/// Each chunk of a unit of `raw` bytes stored as `chunks`: where its stored bytes and its raw
/// bytes sit within the unit's.
pub(super) fn chunk_places(raw: u64, chunks: &[u32]) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
    let mut stored = 0;
    (0_u64..).zip(chunks).map(move |(at, &length)| {
        let from = stored;
        stored += length as usize;
        let start = (at * envelope::CHUNK_BYTES) as usize;
        (from..stored, start..start + envelope::chunk_raw(raw, at) as usize)
    })
}

/// Appends `stored`, unit `extent`'s bytes as `chunks`, decoded to `out`, its digest checked
/// first.
fn decode_unit(
    context: &mut Decompressor,
    extent: &Extent,
    stored: &[u8],
    chunks: &[u32],
    out: &mut Vec<u8>,
) -> io::Result<()> {
    check(extent, stored)?;
    let raw = usize::try_from(extent.raw).map_err(|_| engine_allocation::oom())?;
    let held = chunks.iter().map(|&chunk| chunk as usize).sum::<usize>();
    if chunks.len() as u64 != envelope::chunk_count(extent.raw) || held != stored.len() {
        return Err(invalid("differs from its chunks"));
    }
    for (stored_at, raw_at) in chunk_places(extent.raw, chunks) {
        chunk_header(&stored[stored_at], raw_at.len() as u64)?;
    }
    engine_allocation::reserve(out, raw)?;
    let spare = &mut out.spare_capacity_mut()[..raw];
    for (stored_at, raw_at) in chunk_places(extent.raw, chunks) {
        decode_chunk(context, &stored[stored_at], &mut spare[raw_at])?;
    }
    // SAFETY: the chunks cover the unit's raw bytes, and each decoded fills its own.
    unsafe { out.set_len(out.len() + raw) };
    Ok(())
}

/// Refuses `header`, the start of one chunk's frame, unless it declares `raw` bytes.
fn chunk_header(header: &[u8], raw: u64) -> io::Result<()> {
    match zstd_safe::get_frame_content_size(header) {
        Ok(Some(declared)) if declared == raw => Ok(()),
        _ => Err(invalid("differs from its chunk's frame length")),
    }
}

/// Decodes `stored`, one chunk's frame, into `out`, which it fills exactly.
fn decode_chunk(context: &mut Decompressor, stored: &[u8], out: &mut [MaybeUninit<u8>]) -> io::Result<()> {
    chunk_header(stored, out.len() as u64)?;
    if zstd_safe::find_frame_compressed_size(stored).ok() != Some(stored.len()) {
        return Err(invalid(TRAILING));
    }
    let written = context.decompress_into(out, stored).map_err(Error::invalid)?;
    if written != out.len() {
        return Err(invalid("decompresses to another length"));
    }
    Ok(())
}

/// Appends `stored`, `extent`'s bytes, decoded to `out` with `base` as its prefix when it was
/// compressed with one, its digest and frame's length checked first.
fn decode(
    context: &mut Decompressor,
    extent: &Extent,
    stored: &[u8],
    base: Option<&[u8]>,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    check(extent, stored)?;
    let raw = frame_length(extent, stored)?;
    if zstd_safe::find_frame_compressed_size(stored).ok() != Some(stored.len()) {
        return Err(invalid(TRAILING));
    }
    engine_allocation::reserve(out, raw)?;
    let written = match base {
        None => context.decompress_append(stored, out),
        Some(base) => context.decompress_append_prefixed(base, stored, out),
    }
    .map_err(Error::invalid)?;
    if written != raw {
        return Err(invalid("decompresses to another length"));
    }
    Ok(())
}
