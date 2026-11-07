//! A columnar recording's bytes as its walkers read them: from its file or from memory,
//! through its last footer whose root verifies, a whole extent at a time.

use super::invalid;
use replay_columnar::envelope::{self, Extent, Kind};
use std::{fs::File, io};
use zstd::zstd_safe::{self, DCtx};

/// The blocks a footer is searched for and a file's root hashed in.
const BLOCK: usize = 64 << 10;
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
    let mut start = [0; envelope::HEADER_BYTES];
    held.read(0, &mut start[..length.min(header) as usize])?;
    envelope::check_header(&start[..length.min(header) as usize])?;
    if length < header + footer {
        return Err(invalid("An H4COLUMN recording holds no footer"));
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

    /// `extent`'s stored bytes once their place and digest are checked; a file's are read into
    /// `buffer`.
    pub(super) fn stored_checked<'a>(&'a self, extent: &Extent, buffer: &'a mut Vec<u8>) -> io::Result<&'a [u8]> {
        let stored = self.stored(extent, buffer)?;
        check(extent, stored)?;
        Ok(stored)
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
fn check(extent: &Extent, stored: &[u8]) -> io::Result<()> {
    if envelope::digest(stored) != extent.digest {
        return Err(invalid("differs from its digest"));
    }
    Ok(())
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

    /// The raw length of `extent`, gathered from `start`: as [`Source::raw_length`] reads it.
    pub(super) fn gathered_length(&self, extent: &Extent, start: usize) -> io::Result<usize> {
        let stored = extent.stored as usize;
        frame_length(extent, &self.stored[start..][..stored.min(FRAME_HEADER)])
    }

    /// Appends the raw bytes of `extent`, gathered from `start`, to `out`: as
    /// [`Self::append_with`] decodes it.
    pub(super) fn append_gathered(
        &mut self,
        extent: &Extent,
        start: usize,
        base: Option<&[u8]>,
        out: &mut Vec<u8>,
    ) -> io::Result<()> {
        let stored = &self.stored[start..][..extent.stored as usize];
        decode(&mut self.context, extent, stored, base, out)
    }
}

/// Appends `stored`, `extent`'s bytes, decoded to `out` with `base` as its prefix when it was
/// compressed with one, its digest and frame's length checked first.
fn decode(
    context: &mut DCtx<'static>,
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
    let start = out.len();
    let mut into = std::io::Cursor::new(&mut *out);
    into.set_position(start as u64);
    let written = match base {
        None => context.decompress(&mut into, stored),
        // A prefix serves the one frame it is given for, on a context of its own.
        Some(base) => {
            let mut context = DCtx::try_create().ok_or_else(engine_allocation::oom)?;
            context
                .ref_prefix(base)
                .and_then(|_| context.decompress(&mut into, stored))
        }
    }
    .map_err(|code| invalid(zstd_safe::get_error_name(code)))?;
    if written != raw {
        return Err(invalid("decompresses to another length"));
    }
    Ok(())
}
