
use super::invalid;
use replay_columnar::{
    container::{self, Open, ReadAt},
    envelope::{self, Extent},
    unit::reader::{Extents, SpareUser},
};
use std::{fmt, fs::File, io, sync::Arc};

#[derive(Debug)]
pub struct NotSaved;

impl fmt::Display for NotSaved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "This recording has not been saved yet: its hours wait to be saved or discarded in BattlePlan Recorder.",
        )
    }
}

impl std::error::Error for NotSaved {}

pub fn is_not_saved(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<NotSaved>())
}

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
            return Err(io::Error::other("The recording's file ends before its committed length"));
        }
        at += count as u64;
        bytes = &mut bytes[count..];
    }
    Ok(())
}

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

pub struct Source {
    held: Held,
    end: u64,
    spare: Arc<SpareUser>,
}

impl Source {
    pub(super) fn open(mut held: Held, open: Open) -> io::Result<(Self, Extent)> {
        let (root, end) = container::locate(&held, open)?.ok_or_else(|| io::Error::other(NotSaved))?;
        if let Held::Memory(bytes) = &mut held {
            bytes.truncate(end as usize);
        }
        Ok((Self { held, end, spare: SpareUser::acquire() }, root))
    }

    pub fn len(&self) -> u64 {
        self.end
    }

    pub fn memory(&self) -> Option<&[u8]> {
        match &self.held {
            Held::Memory(bytes) => Some(bytes),
            Held::File(_) => None,
        }
    }

    fn place(&self, extent: &Extent) -> io::Result<usize> {
        let placed = extent.offset >= envelope::HEADER_BYTES as u64
            && extent.offset.checked_add(extent.stored).is_some_and(|end| end + envelope::FOOTER_BYTES as u64 <= self.end);
        if !placed {
            return Err(invalid("lies outside the recording's extents"));
        }
        usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())
    }

    pub(super) fn stored_at(&self, extent: &Extent, at: u64, out: &mut [u8]) -> io::Result<()> {
        let length = self.place(extent)? as u64;
        if at.checked_add(out.len() as u64).is_none_or(|end| end > length) {
            return Err(invalid("reads past its extent"));
        }
        self.held.read(extent.offset + at, out)
    }

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
    fn spare_user(&self) -> Arc<SpareUser> { Arc::clone(&self.spare) }

    fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
        if offset.checked_add(length as u64).is_none_or(|end| end > self.end) {
            return Err(invalid("A unit frame lies past the revision"));
        }
        out.clear();
        engine_allocation::resize(out, length, 0)?;
        self.held.read(offset, out)
    }
}
