//! A recording's bytes as its readers read them: from its file or from memory, through the
//! footer of the revision opened ([`Open`]). The unit's reader primitives read their frames
//! through the same source ([`Extents`]).

use super::invalid;
use replay_columnar::{
    container::{self, Open, ReadAt},
    envelope::{self, Extent},
    unit::reader::Extents,
};
use std::{fmt, fs::File, io};

/// A recording none of whose revisions is saved yet: its hours wait to be saved or discarded,
/// and only its recorder opens it.
#[derive(Debug)]
pub struct NotSaved;

impl fmt::Display for NotSaved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "This recording has not been saved yet: its hours wait to be saved or discarded in Battle Plan Recorder.",
        )
    }
}

impl std::error::Error for NotSaved {}

/// Whether `error` refuses a recording that is not saved yet ([`NotSaved`]).
pub fn is_not_saved(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<NotSaved>())
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
            return Err(io::Error::other("The recording's file ends before its committed length"));
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
    fn read(&self, at: u64, out: &mut [u8]) -> io::Result<()> {
        match self {
            Self::File(file) => read_at(file, at, out),
            Self::Memory(bytes) => {
                let start = usize::try_from(at).map_err(|_| invalid("reads past the recording"))?;
                let held = bytes.get(start..start + out.len()).ok_or_else(|| invalid("reads past the recording"))?;
                out.copy_from_slice(held);
                Ok(())
            }
        }
    }
}

impl ReadAt for Held {
    fn len(&self) -> io::Result<u64> {
        match self {
            Self::File(file) => Ok(file.metadata()?.len()),
            Self::Memory(bytes) => Ok(bytes.len() as u64),
        }
    }

    fn read_at(&self, at: u64, out: &mut [u8]) -> io::Result<()> {
        self.read(at, out)
    }
}

/// A recording's bytes through the footer of the revision opened.
pub struct Source {
    held: Held,
    end: u64,
}

impl Source {
    /// What `held` holds through the footer of the revision `open` names, and that revision's
    /// root. A saved revision asked of a recording that holds none is refused as [`NotSaved`].
    pub(super) fn open(mut held: Held, open: Open) -> io::Result<(Self, Extent)> {
        let (root, end) = container::locate(&held, open)?.ok_or_else(|| io::Error::other(NotSaved))?;
        // What follows the revision, later ones or a torn tail, is no part of what is opened.
        if let Held::Memory(bytes) = &mut held {
            bytes.truncate(end as usize);
        }
        Ok((Self { held, end }, root))
    }

    pub fn len(&self) -> u64 {
        self.end
    }

    /// The bytes it holds in memory, through its revision's footer; none for a file.
    pub fn memory(&self) -> Option<&[u8]> {
        match &self.held {
            Held::Memory(bytes) => Some(bytes),
            Held::File(_) => None,
        }
    }

    /// `extent`'s stored length once its place is checked.
    fn place(&self, extent: &Extent) -> io::Result<usize> {
        let placed = extent.offset >= envelope::HEADER_BYTES as u64
            && extent.offset.checked_add(extent.stored).is_some_and(|end| end + envelope::FOOTER_BYTES as u64 <= self.end);
        if !placed {
            return Err(invalid("lies outside the recording's extents"));
        }
        usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())
    }

    /// `out.len()` stored bytes of `extent` from `at` within it, once its place is checked; the
    /// digest is not: the caller checks the extent whole once.
    pub(super) fn stored_at(&self, extent: &Extent, at: u64, out: &mut [u8]) -> io::Result<()> {
        let length = self.place(extent)? as u64;
        if at.checked_add(out.len() as u64).is_none_or(|end| end > length) {
            return Err(invalid("reads past its extent"));
        }
        self.held.read(extent.offset + at, out)
    }

    /// `extent`'s stored bytes, their place and digest checked.
    pub(super) fn stored(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        let mut stored = engine_allocation::filled(self.place(extent)?, 0)?;
        self.held.read(extent.offset, &mut stored)?;
        if envelope::digest(&stored) != extent.digest {
            return Err(invalid("differs from its digest"));
        }
        Ok(stored)
    }
}

impl Extents for Source {
    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
        if offset.checked_add(length as u64).is_none_or(|end| end > self.end) {
            return Err(invalid("A unit frame lies past the revision"));
        }
        out.clear();
        engine_allocation::resize(out, length, 0)?;
        self.held.read(offset, out)
    }
}
