//! Finished bytes kept on disk beside the store until the range they belong to is written.

use super::{
    DELETE, FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_DELETE_ON_CLOSE, GENERIC_READ, GENERIC_WRITE,
    SHARING, invalid, read_at, write_all_at,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const BUFFER: usize = 1 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Piece {
    pub offset: u64,
    pub len: u64,
}

/// An append-only temporary file, made at its first write and deleted when it closes.
#[derive(Debug)]
pub struct Spill {
    directory: PathBuf,
    file: Option<File>,
    len: u64,
    buffer: Vec<u8>,
}

impl Spill {
    pub fn new(directory: &Path) -> Self {
        Self {
            directory: directory.to_path_buf(),
            file: None,
            len: 0,
            buffer: Vec::new(),
        }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn append(&mut self, bytes: &[u8]) -> io::Result<Piece> {
        let piece = Piece {
            offset: self.len,
            len: bytes.len() as u64,
        };
        let len = self
            .len
            .checked_add(piece.len)
            .ok_or_else(|| invalid("spill length overflow"))?;
        if self.buffer.len() + bytes.len() > BUFFER {
            self.flush()?;
        }
        if bytes.len() >= BUFFER {
            write_all_at(self.file()?, bytes, piece.offset)?;
        } else {
            self.buffer.extend_from_slice(bytes);
        }
        self.len = len;
        Ok(piece)
    }

    pub fn read<I>(&self, pieces: I) -> Pieces<'_, I::IntoIter>
    where
        I: IntoIterator<Item = Piece>,
    {
        Pieces {
            spill: self,
            pieces: pieces.into_iter(),
            at: Piece::default(),
        }
    }

    /// Writes what it buffers and lets the buffer go: an idle spill holds no memory.
    pub fn seal(&mut self) -> io::Result<()> {
        self.flush()?;
        self.buffer = Vec::new();
        Ok(())
    }

    /// A handle reading what it wrote, apart from it; none before its first write. Pieces still
    /// buffered are not in the file until [`Self::seal`].
    pub fn reader(&self) -> io::Result<Option<File>> {
        self.file.as_ref().map(File::try_clone).transpose()
    }

    /// Forgets every piece; the file is written over from its start.
    pub fn clear(&mut self) {
        self.len = 0;
        self.buffer = Vec::new();
    }

    fn file(&mut self) -> io::Result<&File> {
        if self.file.is_none() {
            self.file = Some(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                    .share_mode(SHARING)
                    .custom_flags(FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE)
                    .open(
                        self.directory
                            .join(format!("spill_{}.tmp", Uuid::new_v4().simple())),
                    )?,
            );
        }
        Ok(self.file.as_ref().expect("a spill file"))
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let offset = self.len - (self.buffer.len() as u64);
        self.file()?;
        let file = self.file.as_ref().expect("a spill file");
        write_all_at(file, &self.buffer, offset)?;
        self.buffer.clear();
        Ok(())
    }
}

pub struct Pieces<'a, I> {
    spill: &'a Spill,
    pieces: I,
    at: Piece,
}

impl<I: Iterator<Item = Piece>> Read for Pieces<'_, I> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.at.len == 0 {
            match self.pieces.next() {
                Some(piece) => self.at = piece,
                None => return Ok(0),
            }
        }
        let spill = self.spill;
        let count = out.len().min(usize::try_from(self.at.len).unwrap_or(usize::MAX));
        // What is still buffered was never written to the file.
        let written = spill.len - spill.buffer.len() as u64;
        let read = if self.at.offset >= written {
            let from = usize::try_from(self.at.offset - written).map_err(io::Error::other)?;
            let held = spill.buffer.get(from..).unwrap_or_default();
            let count = count.min(held.len());
            out[..count].copy_from_slice(&held[..count]);
            count
        } else {
            let file = spill
                .file
                .as_ref()
                .ok_or_else(|| invalid("a spill piece was never written"))?;
            let count = count.min(usize::try_from(written - self.at.offset).unwrap_or(usize::MAX));
            read_at(file, &mut out[..count], self.at.offset)?
        };
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.at.offset += read as u64;
        self.at.len -= read as u64;
        Ok(read)
    }
}
