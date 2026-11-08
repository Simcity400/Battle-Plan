//! A capture's columnar journal: what the collector hands the recorder of its columnar
//! recording, as it records it, uncompressed, for the recorder to write each block of once in
//! final form ([`crate::columnar_blocks`], [`crate::columnar_file`]). The collector does no
//! more for it than append what its frame writer made: no compression, no restart, no index.
//!
//! A journal is a sequence of segment files, `columnar.<capture>.<segment>.h4j` in the
//! campaign's session directory. Each holds the hours of at most one block of one run (a new
//! run, or the block after a full one, starts the next segment), and ends with
//! [`Record::Next`] once the collector moved on, or [`Record::Finish`] once the capture ended.
//! The recorder removes a segment once its file holds what the segment does. A segment's last
//! record may be torn by a crash: what precedes it stands.
//!
//! A segment: [`MAGIC`], then records, each its kind (u8), its payload's length (u32 LE), the
//! payload and the CRC-32 of kind, length and payload.

use crate::columnar_file::{Capture, Stop};
use replay_columnar::leaf::Reader;
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub const MAGIC: &[u8; 8] = b"H4JRNL01";

const BEGIN: u8 = 1;
const RUN: u8 = 2;
const HOUR: u8 = 3;
const CONTROLS: u8 = 4;
const NEXT: u8 = 5;
const FINISH: u8 = 6;

/// The most bytes one record's payload takes: an hour's manifest and sections.
const MOST_RECORD_BYTES: usize = 1 << 30;

fn invalid(what: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, what)
    }
}

/// The longest name [`segment_name`] writes.
pub const SEGMENT_NAME_BYTES: usize = "columnar.4294967295.4294967295.h4j".len();

/// A segment's file name, `columnar.<capture>.<segment>.h4j`, written into `out`, so it is
/// named without allocating.
pub fn segment_name(capture: u32, segment: u32, out: &mut [u8; SEGMENT_NAME_BYTES]) -> &str {
    let mut cursor = io::Cursor::new(&mut out[..]);
    write!(cursor, "columnar.{capture}.{segment}.h4j").expect("the longest name fits");
    let length = cursor.position() as usize;
    std::str::from_utf8(&out[..length]).expect("ASCII")
}

/// The capture and segment a segment file's name names.
pub fn segment_of(name: &OsStr) -> Option<(u32, u32)> {
    let name = name.to_str()?;
    let rest = name.strip_prefix("columnar.")?.strip_suffix(".h4j")?;
    let (capture, segment) = rest.split_once('.')?;
    let counted = |number: &str| {
        (number.bytes().all(|byte| byte.is_ascii_digit()) && (number == "0" || !number.starts_with('0')))
            .then(|| number.parse().ok())
            .flatten()
    };
    Some((counted(capture)?, counted(segment)?))
}

/// Every journal segment in `directory`, ascending by capture and segment.
pub fn segments(directory: &Path) -> io::Result<Vec<(u32, u32, PathBuf)>> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        entries => entries?,
    };
    for entry in entries {
        let entry = entry?;
        if let Some((capture, segment)) = segment_of(&entry.file_name()) {
            found.push((capture, segment, entry.path()));
        }
    }
    found.sort_unstable_by_key(|&(capture, segment, _)| (capture, segment));
    Ok(found)
}

/// One record of a journal.
#[derive(Debug, PartialEq, Eq)]
pub enum Record<'a> {
    /// The first record of a capture's first segment: its file identity
    /// ([`replay_columnar::envelope::Identity::chain_bytes`]) and its capture record, which
    /// holds no control records.
    Begin { identity: &'a [u8], capture: &'a [u8] },
    /// A run starts with the next hour.
    Run { branch: u64, epoch: u8, first_hour: i32 },
    /// The run's next hour: its manifest, then its sections in the manifest's order.
    Hour { manifest: &'a [u8], sections: &'a [u8] },
    /// Control records the capture holds from now on, each as the rows encode it.
    Controls(Vec<&'a [u8]>),
    /// The segment ends; the next holds what follows.
    Next,
    /// The capture ends: where writing stopped short of the rows, when it did.
    Finish { stop: Option<Stop> },
}

fn varint(out: &mut [u8; 10], mut value: u64) -> &[u8] {
    let mut at = 0;
    while value >= 0x80 {
        out[at] = value as u8 | 0x80;
        value >>= 7;
        at += 1;
    }
    out[at] = value as u8;
    &out[..=at]
}

/// Appends a capture's journal, a segment at a time. Every record reaches the file as it is
/// written; an ended segment goes back to the caller to make durable, so the next one opens
/// without waiting on the disk.
pub struct JournalWriter {
    directory: PathBuf,
    capture: u32,
    segment: u32,
    file: BufWriter<File>,
    /// The segment holds a record past its magic.
    used: bool,
}

impl JournalWriter {
    /// Starts capture `capture`'s journal in `directory` with its file identity and capture.
    pub fn create(directory: &Path, capture: u32, identity: &[u8], record: &Capture) -> io::Result<Self> {
        let mut writer = Self {
            directory: directory.to_path_buf(),
            capture,
            segment: 0,
            file: Self::open(directory, capture, 0)?,
            used: false,
        };
        let capture = record.write_with(&[])?;
        let mut length = [0; 10];
        let length = varint(&mut length, identity.len() as u64);
        writer.append(BEGIN, &[length, identity, &capture])?;
        Ok(writer)
    }

    fn open(directory: &Path, capture: u32, segment: u32) -> io::Result<BufWriter<File>> {
        let mut name = [0; SEGMENT_NAME_BYTES];
        let mut path = directory.to_path_buf();
        path.push(segment_name(capture, segment, &mut name));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut file = BufWriter::with_capacity(1 << 16, file);
        file.write_all(MAGIC)?;
        Ok(file)
    }

    /// One record of `parts`, to the file.
    fn append(&mut self, kind: u8, parts: &[&[u8]]) -> io::Result<()> {
        let length: usize = parts.iter().map(|part| part.len()).sum();
        if length > MOST_RECORD_BYTES {
            return Err(invalid("A journal record passes the largest record"));
        }
        let length = (length as u32).to_le_bytes();
        let mut crc = crc32fast::Hasher::new();
        crc.update(&[kind]);
        crc.update(&length);
        self.file.write_all(&[kind])?;
        self.file.write_all(&length)?;
        for part in parts {
            crc.update(part);
            self.file.write_all(part)?;
        }
        self.file.write_all(&crc.finalize().to_le_bytes())?;
        self.file.flush()?;
        self.used = true;
        Ok(())
    }

    /// The capture this journal records.
    pub fn capture(&self) -> u32 {
        self.capture
    }

    /// Ends the segment and starts the next; nothing when the segment holds nothing past its
    /// magic. The ended segment's file is the caller's to make durable (`sync_data`).
    #[must_use = "an ended segment is durable only once its file is synced"]
    pub fn next_segment(&mut self) -> io::Result<Option<File>> {
        if !self.used {
            return Ok(None);
        }
        self.append(NEXT, &[])?;
        self.segment = self
            .segment
            .checked_add(1)
            .ok_or_else(|| invalid("A journal's segments pass 2^32"))?;
        let next = Self::open(&self.directory, self.capture, self.segment)?;
        let ended = std::mem::replace(&mut self.file, next);
        self.used = false;
        ended.into_inner().map(Some).map_err(io::IntoInnerError::into_error)
    }

    /// A run starting with the next hour, in a segment of its own: after [`Self::next_segment`],
    /// so the segment it ended is the caller's to sync even when this append fails.
    pub fn run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
        if self.used {
            return Err(invalid("A run starts in a segment of its own"));
        }
        let mut branch_bytes = [0; 10];
        let mut hour_bytes = [0; 10];
        let branch = varint(&mut branch_bytes, branch);
        let hour = varint(&mut hour_bytes, u64::from(((first_hour << 1) ^ (first_hour >> 31)) as u32));
        self.append(RUN, &[branch, &[epoch], hour])
    }

    /// The run's next hour: its manifest and its sections in the manifest's order.
    pub fn hour(&mut self, manifest: &[u8], sections: &[&[u8]]) -> io::Result<()> {
        let mut length = [0; 10];
        let length = varint(&mut length, manifest.len() as u64);
        let mut parts: [&[u8]; 2] = [length, manifest];
        self.append_with_sections(HOUR, &mut parts, sections)
    }

    fn append_with_sections(&mut self, kind: u8, head: &mut [&[u8]; 2], sections: &[&[u8]]) -> io::Result<()> {
        let length: usize = head.iter().chain(sections).map(|part| part.len()).sum();
        if length > MOST_RECORD_BYTES {
            return Err(invalid("A journal record passes the largest record"));
        }
        let length = (length as u32).to_le_bytes();
        let mut crc = crc32fast::Hasher::new();
        crc.update(&[kind]);
        crc.update(&length);
        self.file.write_all(&[kind])?;
        self.file.write_all(&length)?;
        for part in head.iter().chain(sections) {
            crc.update(part);
            self.file.write_all(part)?;
        }
        self.file.write_all(&crc.finalize().to_le_bytes())?;
        self.file.flush()?;
        self.used = true;
        Ok(())
    }

    /// Control records the capture holds from now on.
    pub fn controls(&mut self, records: &[Vec<u8>]) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut lengths = Vec::new();
        engine_allocation::reserve(&mut lengths, records.len())?;
        for record in records {
            let mut length = [0; 10];
            let written = varint(&mut length, record.len() as u64).len();
            lengths.push((length, written));
        }
        let mut count = [0; 10];
        let count = varint(&mut count, records.len() as u64);
        let mut parts: Vec<&[u8]> = Vec::new();
        engine_allocation::reserve(&mut parts, 1 + 2 * records.len())?;
        parts.push(count);
        for ((length, written), record) in lengths.iter().zip(records) {
            parts.push(&length[..*written]);
            parts.push(record);
        }
        self.append(CONTROLS, &parts)
    }

    /// Ends the capture, durable.
    pub fn finish(mut self, stop: Option<&Stop>) -> io::Result<()> {
        let mut bytes = Vec::new();
        let mut scratch = [0; 10];
        match stop {
            None => bytes.push(0),
            Some(stop) => {
                bytes.push(1);
                bytes.extend_from_slice(varint(&mut scratch, stop.branch));
                bytes.extend_from_slice(varint(
                    &mut scratch,
                    stop.last_hour.map_or(0, |hour| u64::from(hour as u32) + 1),
                ));
                bytes.extend_from_slice(varint(&mut scratch, stop.reason.len() as u64));
                bytes.extend_from_slice(stop.reason.as_bytes());
            }
        }
        self.append(FINISH, &[&bytes])?;
        self.file.get_ref().sync_data()
    }
}

/// A segment as it grows: what its writer appended since it was last read, a record at a time.
pub struct Tail {
    path: PathBuf,
    /// None while released ([`Self::release`]): the next read opens it again.
    file: Option<File>,
    /// Bytes read and not yet taken as records: the start of a record still being written.
    pending: Vec<u8>,
    /// Bytes of the file read so far.
    read: u64,
    checked: bool,
}

impl Tail {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(File::open(path)?),
            pending: Vec::new(),
            read: 0,
            checked: false,
        })
    }

    /// Closes the segment until the next read, which takes up where this one left off: so a
    /// check whether its collector still holds it sees only the collector.
    pub fn release(&mut self) {
        self.file = None;
    }

    /// Hands `visit` each complete record appended since the last call, in order, until it
    /// returns false; whether the segment ended ([`Record::Next`] or [`Record::Finish`]).
    pub fn records(&mut self, mut visit: impl FnMut(Record<'_>) -> io::Result<bool>) -> io::Result<bool> {
        use std::io::Seek;
        let file = match &mut self.file {
            Some(file) => file,
            None => self.file.insert(File::open(&self.path)?),
        };
        file.seek(io::SeekFrom::Start(self.read))?;
        let added = file.read_to_end(&mut self.pending)?;
        self.read += added as u64;
        let mut at = 0;
        if !self.checked {
            if self.pending.len() < MAGIC.len() {
                return Ok(false);
            }
            if !self.pending.starts_with(MAGIC) {
                return Err(invalid("Not a columnar journal segment"));
            }
            self.checked = true;
            at = MAGIC.len();
        }
        let mut ended = false;
        let mut records = Records {
            bytes: &self.pending,
            at,
        };
        let mut stopped = false;
        while let Some(record) = records.next() {
            let (record, _) = record?;
            ended |= matches!(record, Record::Next | Record::Finish { .. });
            if !visit(record)? || ended {
                stopped = true;
                break;
            }
        }
        let used = records.at;
        self.pending.drain(..used);
        let _ = stopped;
        Ok(ended)
    }
}

/// A segment read whole, through its last complete record.
pub struct Segment {
    bytes: Vec<u8>,
}

impl Segment {
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        if !bytes.starts_with(MAGIC) {
            return Err(invalid("Not a columnar journal segment"));
        }
        Ok(Self { bytes })
    }

    /// Its records from byte `from` (the first past [`MAGIC`] when earlier), each with the
    /// offset past it, through the last complete one.
    pub fn records(&self, from: usize) -> Records<'_> {
        Records {
            bytes: &self.bytes,
            at: from.max(MAGIC.len()),
        }
    }
}

pub struct Records<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Records<'a> {
    type Item = io::Result<(Record<'a>, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = &self.bytes[self.at..];
        if rest.len() < 9 {
            return None;
        }
        let kind = rest[0];
        let length = u32::from_le_bytes(rest[1..5].try_into().expect("four bytes")) as usize;
        if length > MOST_RECORD_BYTES || rest.len() < 9 + length {
            return None;
        }
        let mut crc = crc32fast::Hasher::new();
        crc.update(&rest[..5 + length]);
        let sum = u32::from_le_bytes(rest[5 + length..9 + length].try_into().expect("four bytes"));
        if sum != crc.finalize() {
            // A torn or damaged tail ends the segment's readable records.
            return None;
        }
        self.at += 9 + length;
        Some(decode(kind, &rest[5..5 + length]).map(|record| (record, self.at)))
    }
}

fn decode(kind: u8, payload: &[u8]) -> io::Result<Record<'_>> {
    let mut reader = Reader::new(payload);
    let record = match kind {
        BEGIN => {
            let length = reader.count()?;
            let identity = reader.raw(length)?;
            let capture = reader.rest();
            return Ok(Record::Begin { identity, capture });
        }
        RUN => {
            let branch = reader.varint()?;
            let epoch = reader.u8()?;
            let hour = reader.varint_u32()?;
            Record::Run {
                branch,
                epoch,
                first_hour: ((hour >> 1) as i32) ^ -((hour & 1) as i32),
            }
        }
        HOUR => {
            let length = reader.count()?;
            let manifest = reader.raw(length)?;
            let sections = reader.rest();
            return Ok(Record::Hour { manifest, sections });
        }
        CONTROLS => {
            let count = reader.count()?;
            if count > payload.len() {
                return Err(invalid("A journal's controls count passes its record"));
            }
            let mut records = Vec::with_capacity(count);
            for _ in 0..count {
                let length = reader.count()?;
                records.push(reader.raw(length)?);
            }
            Record::Controls(records)
        }
        NEXT => Record::Next,
        FINISH => Record::Finish {
            stop: match reader.u8()? {
                0 => None,
                1 => {
                    let branch = reader.varint()?;
                    let last_hour = match reader.varint()? {
                        0 => None,
                        hour => Some(
                            u32::try_from(hour - 1)
                                .map_err(|_| invalid("A stop's hour passes 2^32"))? as i32,
                        ),
                    };
                    let length = reader.count()?;
                    let reason = String::from_utf8(reader.raw(length)?.to_vec())
                        .map_err(|_| invalid("A stop's reason is not UTF-8"))?;
                    Some(Stop {
                        branch,
                        last_hour,
                        reason,
                    })
                }
                _ => return Err(invalid("A journal's stop marker is malformed")),
            },
        },
        _ => return Err(invalid("Unknown journal record")),
    };
    reader.finish()?;
    Ok(record)
}
