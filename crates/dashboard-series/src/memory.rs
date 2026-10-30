//! Pages held in memory: a publication built and read in one process, as tests and tools
//! build one.

use crate::{
    cache::Cache,
    invalid,
    pages::{PageRead, PageWrite},
    types::{Blob, PageId, Publication},
    writer::Writer,
};
use std::io;

#[derive(Clone, Debug, Default)]
pub struct Memory {
    pages: Vec<Vec<u8>>,
}

impl PageRead for Memory {
    fn identity(&mut self, id: PageId) -> io::Result<[u64; 3]> {
        let bytes = self.page(id)?;
        let hash = blake3::hash(bytes);
        let words = hash.as_bytes();
        Ok([
            u64::from_le_bytes(words[..8].try_into().expect("eight bytes")),
            u64::from_le_bytes(words[8..16].try_into().expect("eight bytes")),
            bytes.len() as u64,
        ])
    }
    fn len(&mut self, id: PageId) -> io::Result<u64> {
        Ok(self.page(id)?.len() as u64)
    }
    fn read(&mut self, id: PageId, offset: u64, into: &mut [u8]) -> io::Result<()> {
        let bytes = self.page(id)?;
        let at = usize::try_from(offset).map_err(|_| invalid("memory page offset"))?;
        into.copy_from_slice(
            bytes
                .get(at..at + into.len())
                .ok_or_else(|| invalid("short memory page"))?,
        );
        Ok(())
    }
}

impl PageWrite for Memory {
    fn append(&mut self, bytes: &[u8]) -> io::Result<PageId> {
        engine_allocation::reserve(&mut self.pages, 1)?;
        self.pages.push(bytes.to_vec());
        Ok(self.pages.len() as u64 - 1)
    }
}

impl Memory {
    fn page(&self, id: PageId) -> io::Result<&[u8]> {
        usize::try_from(id)
            .ok()
            .and_then(|at| self.pages.get(at))
            .map(Vec::as_slice)
            .ok_or_else(|| invalid("missing memory page"))
    }
}

/// Cuts `writer`'s segment as `publication`, seals it and publishes it into `io`: the root.
pub fn publish(
    writer: &mut Writer<'_>,
    io: &mut impl PageWrite,
    cache: &Cache<'_>,
    publication: Publication,
    checkpoint: Blob,
) -> io::Result<PageId> {
    let pending = writer.cut(publication, checkpoint)?;
    let sealed = pending.seal(io)?;
    writer.publish(io, cache, sealed)
}
