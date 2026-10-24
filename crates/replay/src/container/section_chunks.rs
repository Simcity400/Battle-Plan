use super::{
    BlockIndex, Replay, Section,
    grouped::{Directory, LATER_CHUNK_HOURS},
};
use crate::{Hour, codec::invalid_static};
use std::{io, vec};

/// Later chunks continue column decoder state; they are not standalone table updates.
pub struct SectionChunks<'a> {
    chunks: vec::IntoIter<SectionChunk<'a>>,
}

pub struct SectionChunk<'a> {
    pub first_hour: Hour,
    pub last_hour: Hour,
    pub stored_length: u64,
    pub raw_length: u64,
    replay: &'a Replay,
    index: BlockIndex,
    offset: usize,
}

impl Replay {
    pub fn section_chunks(
        &self,
        index: &BlockIndex,
        section: Section,
    ) -> io::Result<SectionChunks<'_>> {
        let directory = Directory::read(self, index)?;
        let mut chunks = Vec::new();
        for (slot, (offset, stored, raw)) in
            directory.chunk_extents(section).into_iter().enumerate()
        {
            let (first_hour, last_hour) = match section {
                Section::Later(_) => {
                    let first = i64::from(index.first_hour) + 1 + (slot * LATER_CHUNK_HOURS) as i64;
                    let last =
                        (first + LATER_CHUNK_HOURS as i64 - 1).min(i64::from(index.last_hour));
                    (
                        Hour::try_from(first).map_err(|_| invalid_static("chunk hour overflow"))?,
                        last as Hour,
                    )
                }
                Section::Keyframe(_) => (index.first_hour, index.first_hour),
                _ => (index.first_hour, index.last_hour),
            };
            chunks.push(SectionChunk {
                first_hour,
                last_hour,
                stored_length: stored as u64,
                raw_length: raw as u64,
                replay: self,
                index: index.clone(),
                offset,
            });
        }
        Ok(SectionChunks {
            chunks: chunks.into_iter(),
        })
    }
}

impl<'a> Iterator for SectionChunks<'a> {
    type Item = SectionChunk<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}
impl ExactSizeIterator for SectionChunks<'_> {}

impl SectionChunk<'_> {
    pub fn inflate_into(&self, buffer: &mut Vec<u8>) -> io::Result<()> {
        buffer.clear();
        if self.stored_length == 0 {
            return Ok(());
        }
        let bytes =
            self.replay
                .block_range(&self.index, self.offset, self.stored_length as usize)?;
        Directory::inflate_into(&bytes, self.raw_length as usize, buffer)
    }
}
