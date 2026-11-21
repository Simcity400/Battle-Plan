
use super::SlotRef;
use crate::{envelope::digest, invalid};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const MAGIC: &[u8; 8] = b"H4STATE_";
pub const VERSION: u32 = 1;
const SLOTS: u64 = 16;
const SLOT_BYTES: u64 = 64;
const PAYLOADS: u64 = SLOTS + 2 * SLOT_BYTES;
const HEADER_FIELDS: usize = 8 + 8 + 8 + 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
    sequence: u64,
    offset: u64,
    length: u64,
    digest: [u8; 16],
}

impl Header {
    fn write(&self) -> [u8; SLOT_BYTES as usize] {
        let mut bytes = [0; SLOT_BYTES as usize];
        bytes[..8].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.offset.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.length.to_le_bytes());
        bytes[24..40].copy_from_slice(&self.digest);
        let sealed = digest(&bytes[..HEADER_FIELDS]);
        bytes[40..56].copy_from_slice(&sealed);
        bytes
    }

    fn read(bytes: &[u8; SLOT_BYTES as usize]) -> Option<Self> {
        if digest(&bytes[..HEADER_FIELDS]) != bytes[40..56] {
            return None;
        }
        let header = Self {
            sequence: u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")),
            offset: u64::from_le_bytes(bytes[8..16].try_into().expect("eight bytes")),
            length: u64::from_le_bytes(bytes[16..24].try_into().expect("eight bytes")),
            digest: bytes[24..40].try_into().expect("sixteen bytes"),
        };
        let placed = header.sequence > 0
            && header.offset >= PAYLOADS
            && header.offset.checked_add(header.length).is_some();
        placed.then_some(header)
    }

    fn end(&self) -> u64 {
        self.offset + self.length
    }
}

pub struct StateFile {
    path: PathBuf,
    file: File,
    slots: [Option<Header>; 2],
}

impl StateFile {
    pub fn path_for(recording: &Path) -> PathBuf {
        let mut name = recording.as_os_str().to_os_string();
        name.push(".state");
        PathBuf::from(name)
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let length = file.seek(SeekFrom::End(0))?;
        if length < PAYLOADS {
            let mut start = [0; SLOTS as usize];
            start[..8].copy_from_slice(MAGIC);
            start[8..12].copy_from_slice(&VERSION.to_le_bytes());
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&start)?;
            file.write_all(&[0; 2 * SLOT_BYTES as usize])?;
            file.sync_data()?;
            return Ok(Self {
                path: path.to_path_buf(),
                file,
                slots: [None, None],
            });
        }
        let mut start = [0; PAYLOADS as usize];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut start)?;
        if &start[..8] != MAGIC {
            return Err(invalid(format!("{} is no recording's resume state", path.display())));
        }
        let version = u32::from_le_bytes(start[8..12].try_into().expect("four bytes"));
        if version != VERSION {
            return Err(invalid(format!(
                "{} is resume state version {version}; this build reads version {VERSION} only",
                path.display()
            )));
        }
        let header = |at: u64| -> Option<Header> {
            let at = at as usize;
            Header::read(start[at..at + SLOT_BYTES as usize].try_into().expect("a slot"))
        };
        Ok(Self {
            path: path.to_path_buf(),
            file,
            slots: [header(SLOTS), header(SLOTS + SLOT_BYTES)],
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn newest(&self) -> Option<u64> {
        self.slots.iter().flatten().map(|header| header.sequence).max()
    }

    pub fn slots(&self) -> impl Iterator<Item = u64> + '_ {
        let mut held: [Option<u64>; 2] = [None, None];
        for (at, header) in self.slots.iter().enumerate() {
            held[at] = header.map(|header| header.sequence);
        }
        if held[0] < held[1] {
            held.swap(0, 1);
        }
        held.into_iter().flatten()
    }

    pub fn read(&self, slot: SlotRef) -> io::Result<Option<Vec<u8>>> {
        let Some(header) = self
            .slots
            .iter()
            .flatten()
            .find(|header| header.sequence == slot.sequence && header.digest == slot.digest)
        else {
            return Ok(None);
        };
        let length = self.file.metadata()?.len();
        if header.end() > length {
            return Ok(None);
        }
        let mut payload = engine_allocation::filled(usize::try_from(header.length).map_err(|_| engine_allocation::oom())?, 0)?;
        crate::container::ReadAt::read_at(&self.file, header.offset, &mut payload)?;
        Ok((digest(&payload) == header.digest).then_some(payload))
    }

    pub fn write(&mut self, sequence: u64, payload: &[u8], keep: Option<SlotRef>) -> io::Result<SlotRef> {
        let kept = self.slots.iter().position(|header| {
            header.is_some_and(|header| keep.is_some_and(|keep| keep.sequence == header.sequence && keep.digest == header.digest))
        });
        let at = match kept {
            Some(kept) => 1 - kept,
            None => match self.slots {
                [Some(first), Some(second)] if first.sequence > second.sequence => 1,
                [Some(_), None] => 1,
                _ => 0,
            },
        };
        let length = payload.len() as u64;
        let offset = match kept.and_then(|kept| self.slots[kept]) {
            Some(other) if PAYLOADS + length > other.offset => other.end(),
            _ => PAYLOADS,
        };
        let header = Header {
            sequence,
            offset,
            length,
            digest: digest(payload),
        };
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(payload)?;
        self.file.sync_data()?;
        self.file.seek(SeekFrom::Start(SLOTS + at as u64 * SLOT_BYTES))?;
        self.file.write_all(&header.write())?;
        self.file.sync_data()?;
        self.slots[at] = Some(header);
        let end = self.slots.iter().flatten().map(Header::end).max().unwrap_or(PAYLOADS);
        if self.file.metadata()?.len() > end {
            self.file.set_len(end)?;
        }
        Ok(SlotRef {
            sequence,
            digest: header.digest,
        })
    }

    pub fn remove(self) -> io::Result<()> {
        let Self { path, file, .. } = self;
        drop(file);
        match fs::remove_file(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

pub fn remove_at(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
