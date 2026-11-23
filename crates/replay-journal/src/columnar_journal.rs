
use replay_recording::capture::{Capture, Stop};
use replay_columnar::leaf::Reader;
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    ops::Range,
    path::{Path, PathBuf},
};

pub const MAGIC: &[u8; 8] = b"H4JRNL02";

fn check_magic(bytes: &[u8]) -> io::Result<()> {
    if bytes.starts_with(MAGIC) {
        return Ok(());
    }
    if bytes.len() >= MAGIC.len() && bytes.starts_with(&MAGIC[..6]) && bytes[6..8].iter().all(u8::is_ascii_digit) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "A columnar journal of version {}{}; this build reads version {}{} only and has no upgrader for it",
                char::from(bytes[6]),
                char::from(bytes[7]),
                char::from(MAGIC[6]),
                char::from(MAGIC[7]),
            ),
        ));
    }
    Err(invalid("Not a columnar journal segment"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Witness {
    pub qpc: u64,
    pub load_signal: Option<i64>,
}

const WITNESS_BYTES: usize = 1 + 8 + 8;

fn write_witness(witness: Option<Witness>, out: &mut [u8; WITNESS_BYTES]) -> &[u8] {
    let Some(witness) = witness else {
        out[0] = 0;
        return &out[..1];
    };
    out[0] = 1 | (u8::from(witness.load_signal.is_some()) << 1);
    out[1..9].copy_from_slice(&witness.qpc.to_le_bytes());
    match witness.load_signal {
        Some(signal) => {
            out[9..17].copy_from_slice(&signal.to_le_bytes());
            &out[..17]
        }
        None => &out[..9],
    }
}

fn read_witness(reader: &mut Reader<'_>) -> io::Result<Option<Witness>> {
    let flags = reader.u8()?;
    if flags > 3 || flags == 2 {
        return Err(invalid("A journal hour's witness flags are malformed"));
    }
    if flags & 1 == 0 {
        return Ok(None);
    }
    let qpc = u64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes"));
    let load_signal = if flags & 2 != 0 {
        Some(i64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes")))
    } else {
        None
    };
    Ok(Some(Witness { qpc, load_signal }))
}

const BEGIN: u8 = 1;
const RUN: u8 = 2;
const HOUR: u8 = 3;
const CONTROLS: u8 = 4;
const NEXT: u8 = 5;
const FINISH: u8 = 6;

const MOST_RECORD_BYTES: usize = 1 << 30;

fn invalid(what: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, what)
    }
}

pub const SEGMENT_NAME_BYTES: usize = "columnar.4294967295.4294967295.h4j".len();

pub fn segment_name(capture: u32, segment: u32, out: &mut [u8; SEGMENT_NAME_BYTES]) -> &str {
    let mut cursor = io::Cursor::new(&mut out[..]);
    write!(cursor, "columnar.{capture}.{segment}.h4j").expect("the longest name fits");
    let length = cursor.position() as usize;
    std::str::from_utf8(&out[..length]).expect("ASCII")
}

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

#[derive(Debug, PartialEq, Eq)]
pub enum Record<'a> {
    Begin { identity: &'a [u8], capture: &'a [u8] },
    Run { branch: u64, epoch: u8, first_hour: i32 },
    Hour {
        witness: Option<Witness>,
        manifest: &'a [u8],
        sections: Sections<'a>,
    },
    Controls(Vec<&'a [u8]>),
    Next,
    Finish { stop: Option<Stop> },
}

pub struct Sections<'a>(Held<'a>);

enum Held<'a> {
    Bytes { bytes: &'a [u8], at: usize },
    File { file: &'a mut File, at: u64, end: u64, buffer: &'a mut Vec<u8> },
}

impl<'a> Sections<'a> {
    pub fn held(bytes: &'a [u8]) -> Self {
        Self(Held::Bytes { bytes, at: 0 })
    }

    pub fn left(&self) -> u64 {
        match &self.0 {
            Held::Bytes { bytes, at } => (bytes.len() - at) as u64,
            Held::File { at, end, .. } => end - at,
        }
    }

    pub fn next(&mut self, length: u64) -> io::Result<&[u8]> {
        if length > self.left() {
            return Err(invalid("An hour's sections end before its manifest's"));
        }
        let length = length as usize;
        match &mut self.0 {
            Held::Bytes { bytes, at } => {
                let start = *at;
                *at += length;
                Ok(&bytes[start..start + length])
            }
            Held::File { file, at, buffer, .. } => {
                let _label = journal_label();
                buffer.clear();
                engine_allocation::reserve(buffer, length)?;
                file.seek(SeekFrom::Start(*at))?;
                Read::by_ref(&mut **file).take(length as u64).read_to_end(buffer)?;
                if buffer.len() != length {
                    return Err(invalid("A journal segment ends inside a section"));
                }
                *at += length as u64;
                Ok(buffer)
            }
        }
    }

    pub fn read_all(&mut self, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        let left = self.left();
        engine_allocation::reserve(out, left as usize)?;
        let bytes = self.next(left)?;
        out.extend_from_slice(bytes);
        Ok(())
    }

    pub fn bytes(&self) -> Option<&'a [u8]> {
        match &self.0 {
            Held::Bytes { bytes, at } => Some(&bytes[*at..]),
            Held::File { .. } => None,
        }
    }
}

impl std::fmt::Debug for Sections<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Sections({} bytes)", self.left())
    }
}

impl PartialEq for Sections<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes().is_some() && self.bytes() == other.bytes()
    }
}

impl Eq for Sections<'_> {}

fn journal_label() -> engine_allocation::labels::Scope {
    use engine_allocation::labels;
    labels::enter(labels::owner("journal"), labels::Kind::Operator)
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

pub struct JournalWriter {
    directory: PathBuf,
    capture: u32,
    segment: u32,
    file: BufWriter<File>,
    used: bool,
}

impl JournalWriter {
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

    pub fn capture(&self) -> u32 {
        self.capture
    }

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

    pub fn hour(&mut self, witness: Option<Witness>, manifest: &[u8], sections: &[&[u8]]) -> io::Result<()> {
        let mut witnessed = [0; WITNESS_BYTES];
        let witnessed = write_witness(witness, &mut witnessed);
        let mut length = [0; 10];
        let length = varint(&mut length, manifest.len() as u64);
        let parts: [&[u8]; 3] = [witnessed, length, manifest];
        self.append_with_sections(HOUR, &parts, sections)
    }

    fn append_with_sections(&mut self, kind: u8, head: &[&[u8]; 3], sections: &[&[u8]]) -> io::Result<()> {
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

pub trait Source {
    fn start(&mut self, segment: u32) -> io::Result<()>;

    fn record(&mut self) -> io::Result<Option<Record<'_>>>;

    fn advance(&mut self);

    fn retire(&mut self) -> io::Result<()>;

    fn finish(&mut self) -> io::Result<()>;

    fn producing(&mut self) -> io::Result<bool>;

    fn damage(&mut self) -> io::Result<Option<(u32, u64)>> {
        Ok(None)
    }
}

fn retire(segment: &Path) -> io::Result<()> {
    if cfg!(feature = "keep-journal")
        && let (Some(directory), Some(name)) = (segment.parent(), segment.file_name())
    {
        let kept = directory.join("kept-journal");
        fs::create_dir_all(&kept)?;
        return fs::rename(segment, kept.join(name));
    }
    match fs::remove_file(segment) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

const READ_AHEAD: usize = 1 << 20;

enum Current {
    Held(u8, Range<usize>),
    Streamed {
        witness: Option<Witness>,
        manifest: Range<usize>,
        sections: Range<u64>,
        end: u64,
    },
}

pub struct Journal {
    directory: PathBuf,
    capture: u32,
    segment: u32,
    retired: u32,
    file: Option<File>,
    buffer: Vec<u8>,
    base: u64,
    at: usize,
    section: Vec<u8>,
    checked: bool,
    current: Option<Current>,
    finished: bool,
    keeps: bool,
}

impl Journal {
    pub fn open(directory: &Path, capture: u32) -> Self {
        Self {
            directory: directory.to_path_buf(),
            capture,
            segment: 0,
            retired: 0,
            file: None,
            buffer: Vec::new(),
            base: 0,
            at: 0,
            section: Vec::new(),
            checked: false,
            current: None,
            finished: false,
            keeps: false,
        }
    }

    pub fn open_read_only(directory: &Path, capture: u32) -> Self {
        Self {
            keeps: true,
            ..Self::open(directory, capture)
        }
    }

    pub fn captures(directory: &Path) -> io::Result<Vec<u32>> {
        let mut captures: Vec<u32> = segments(directory)?.into_iter().map(|(capture, _, _)| capture).collect();
        captures.dedup();
        Ok(captures)
    }

    pub fn path(&self, segment: u32) -> PathBuf {
        let mut name = [0; SEGMENT_NAME_BYTES];
        self.directory.join(segment_name(self.capture, segment, &mut name))
    }

    fn held(&self) -> io::Result<Vec<(u32, PathBuf)>> {
        Ok(segments(&self.directory)?
            .into_iter()
            .filter(|(capture, _, _)| *capture == self.capture)
            .map(|(_, segment, path)| (segment, path))
            .collect())
    }

    fn let_go(&self, segment: &Path) -> io::Result<()> {
        if self.keeps { Ok(()) } else { retire(segment) }
    }

    fn rewind(&mut self) {
        self.file = None;
        self.buffer.clear();
        self.buffer.shrink_to(2 * READ_AHEAD);
        self.section = Vec::new();
        self.base = 0;
        self.at = 0;
        self.checked = false;
        self.current = None;
    }

    fn fill(&mut self, need: usize) -> io::Result<bool> {
        if self.buffer.len() - self.at >= need {
            return Ok(true);
        }
        if self.at > 0 {
            self.buffer.drain(..self.at);
            self.base += self.at as u64;
            self.at = 0;
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(false);
        };
        let _label = journal_label();
        file.seek(SeekFrom::Start(self.base + self.buffer.len() as u64))?;
        let more = (need - self.buffer.len()).max(READ_AHEAD);
        Read::by_ref(file).take(more as u64).read_to_end(&mut self.buffer)?;
        Ok(self.buffer.len() >= need)
    }

    fn read(&mut self) -> io::Result<()> {
        if self.file.is_none() {
            match File::open(self.path(self.segment)) {
                Ok(file) => self.file = Some(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        if !self.checked {
            if !self.fill(MAGIC.len())? {
                return Ok(());
            }
            check_magic(&self.buffer[self.at..])?;
            self.at += MAGIC.len();
            self.checked = true;
        }
        if !self.fill(5)? {
            return Ok(());
        }
        let kind = self.buffer[self.at];
        let length = u32::from_le_bytes(self.buffer[self.at + 1..self.at + 5].try_into().expect("four bytes")) as usize;
        if length > MOST_RECORD_BYTES {
            return Ok(());
        }
        if kind == HOUR && length > READ_AHEAD {
            return self.read_long(length);
        }
        if !self.fill(9 + length)? {
            return Ok(());
        }
        let record = &self.buffer[self.at..self.at + 9 + length];
        let sum = u32::from_le_bytes(record[5 + length..].try_into().expect("four bytes"));
        if sum == crc32fast::hash(&record[..5 + length]) {
            self.current = Some(Current::Held(kind, self.at + 5..self.at + 5 + length));
        }
        Ok(())
    }

    fn read_long(&mut self, length: usize) -> io::Result<()> {
        let start = self.base + self.at as u64;
        let file = self.file.as_mut().expect("an open segment");
        if file.metadata()?.len() < start + 9 + length as u64 {
            return Ok(());
        }
        let _label = journal_label();
        let mut crc = crc32fast::Hasher::new();
        let mut chunk = Vec::new();
        engine_allocation::reserve(&mut chunk, READ_AHEAD)?;
        file.seek(SeekFrom::Start(start))?;
        let mut left = 5 + length as u64;
        while left > 0 {
            chunk.clear();
            let took = Read::by_ref(&mut *file).take(left.min(READ_AHEAD as u64)).read_to_end(&mut chunk)?;
            if took == 0 {
                return Ok(());
            }
            crc.update(&chunk);
            left -= took as u64;
        }
        chunk.clear();
        Read::by_ref(&mut *file).take(4).read_to_end(&mut chunk)?;
        if chunk.len() != 4 || u32::from_le_bytes(chunk[..4].try_into().expect("four bytes")) != crc.finalize() {
            return Ok(());
        }
        drop(chunk);
        let payload = start + 5;
        self.buffer.clear();
        self.base = payload;
        self.at = 0;
        if !self.fill(WITNESS_BYTES + 10)? {
            return Err(invalid("A journal hour ends inside its witness"));
        }
        let mut reader = Reader::new(&self.buffer);
        let witness = read_witness(&mut reader)?;
        let manifest_length = reader.count()?;
        let head = reader.at();
        if head + manifest_length > length {
            return Err(invalid("A journal hour's manifest passes its record"));
        }
        if !self.fill(head + manifest_length)? {
            return Err(invalid("A journal segment ends inside a manifest"));
        }
        self.current = Some(Current::Streamed {
            witness,
            manifest: head..head + manifest_length,
            sections: payload + (head + manifest_length) as u64..payload + length as u64,
            end: payload + length as u64 + 4,
        });
        Ok(())
    }
}

impl Source for Journal {
    fn start(&mut self, segment: u32) -> io::Result<()> {
        self.rewind();
        self.segment = segment;
        self.retired = segment;
        for (held, path) in self.held()? {
            if held < segment {
                self.let_go(&path)?;
            }
        }
        Ok(())
    }

    fn record(&mut self) -> io::Result<Option<Record<'_>>> {
        if self.current.is_none() && !self.finished {
            self.read()?;
        }
        match &self.current {
            Some(Current::Held(kind, payload)) => decode(*kind, &self.buffer[payload.clone()]).map(Some),
            Some(Current::Streamed { witness, manifest, sections, .. }) => Ok(Some(Record::Hour {
                witness: *witness,
                manifest: &self.buffer[manifest.clone()],
                sections: Sections(Held::File {
                    file: self.file.as_mut().expect("an open segment"),
                    at: sections.start,
                    end: sections.end,
                    buffer: &mut self.section,
                }),
            })),
            None => Ok(None),
        }
    }

    fn advance(&mut self) {
        match self.current.take() {
            None => {}
            Some(Current::Held(kind, payload)) => {
                self.at = payload.end + 4;
                if kind == NEXT || kind == FINISH {
                    self.rewind();
                    self.segment = self.segment.saturating_add(1);
                    self.finished = kind == FINISH;
                }
            }
            Some(Current::Streamed { end, .. }) => {
                self.buffer.clear();
                self.buffer.shrink_to(2 * READ_AHEAD);
                self.section = Vec::new();
                self.base = end;
                self.at = 0;
            }
        }
    }

    fn retire(&mut self) -> io::Result<()> {
        for segment in self.retired..self.segment {
            self.let_go(&self.path(segment))?;
        }
        self.retired = self.segment;
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.rewind();
        self.finished = true;
        for (_, path) in self.held()? {
            self.let_go(&path)?;
        }
        Ok(())
    }

    fn producing(&mut self) -> io::Result<bool> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
        if self.finished || self.keeps {
            return Ok(false);
        }
        self.file = None;
        let newest = |journal: &Self| journal.held().map(|held| held.into_iter().next_back());
        let mut found = newest(self)?;
        while let Some((segment, path)) = found {
            match fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&path) {
                Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => return Ok(true),
                Err(error) if error.kind() == io::ErrorKind::NotFound => found = newest(self)?,
                Err(error) => return Err(error),
                Ok(free) => {
                    found = newest(self)?.filter(|(now, _)| *now != segment);
                    drop(free);
                }
            }
        }
        Ok(false)
    }

    fn damage(&mut self) -> io::Result<Option<(u32, u64)>> {
        if self.finished || self.record()?.is_some() {
            return Ok(None);
        }
        let read = self.base + self.at as u64;
        let length = match fs::metadata(self.path(self.segment)) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        let later = self.held()?.into_iter().any(|(segment, _)| segment > self.segment);
        Ok((length > read || later).then_some((self.segment, read)))
    }
}

pub struct Tail {
    path: PathBuf,
    file: Option<File>,
    pending: Vec<u8>,
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

    pub fn release(&mut self) {
        self.file = None;
    }

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
            check_magic(&self.pending)?;
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

pub struct Segment {
    bytes: Vec<u8>,
}

impl Segment {
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        check_magic(&bytes)?;
        Ok(Self { bytes })
    }

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
            let witness = read_witness(&mut reader)?;
            let length = reader.count()?;
            let manifest = reader.raw(length)?;
            let sections = Sections::held(reader.rest());
            return Ok(Record::Hour {
                witness,
                manifest,
                sections,
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("hoi4-journal-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn capture() -> Capture {
        Capture {
            campaign_id: b"campaign".to_vec(),
            content: Vec::new(),
            capture_mode: 0,
            sessions: Vec::new(),
            run_sessions: Vec::new(),
            stop: None,
            format: 0,
            journaled: 0,
            controls: Vec::new(),
        }
    }

    #[test]
    fn an_hour_reads_back_with_its_witness() {
        let folder = Folder::new("witness");
        let mut writer = JournalWriter::create(&folder.0, 0, b"identity", &capture()).unwrap();
        let _ = writer.next_segment().unwrap();
        writer.run(7, 0, -3).unwrap();
        let witnesses = [
            Some(Witness {
                qpc: u64::MAX,
                load_signal: Some(i64::MIN),
            }),
            Some(Witness {
                qpc: 0,
                load_signal: None,
            }),
            None,
        ];
        for witness in witnesses {
            writer.hour(witness, b"manifest", &[b"one", b"", b"two"]).unwrap();
        }
        writer.finish(None).unwrap();
        let mut read = Vec::new();
        for (_, _, path) in segments(&folder.0).unwrap() {
            for record in Segment::read(&path).unwrap().records(0) {
                if let Record::Hour {
                    witness,
                    manifest,
                    sections,
                } = record.unwrap().0
                {
                    assert_eq!((manifest, sections.bytes()), (&b"manifest"[..], Some(&b"onetwo"[..])));
                    read.push(witness);
                }
            }
        }
        assert_eq!(read, witnesses);
    }

    #[test]
    fn a_segment_of_another_version_is_refused_by_name() {
        let folder = Folder::new("version");
        let older = folder.0.join("columnar.0.0.h4j");
        std::fs::write(&older, b"H4JRNL01\x05\0\0\0\0").unwrap();
        let refused = |error: io::Error| {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            error.to_string()
        };
        let whole = refused(Segment::read(&older).err().unwrap());
        assert!(whole.contains("version 01") && whole.contains("version 02 only"), "{whole}");
        let growing = refused(Tail::open(&older).unwrap().records(|_| Ok(true)).unwrap_err());
        assert_eq!(growing, whole);
        let other = folder.0.join("columnar.0.1.h4j");
        std::fs::write(&other, b"H4JRNLxx").unwrap();
        assert!(!refused(Segment::read(&other).err().unwrap()).contains("version"));
        std::fs::write(&other, b"H4J").unwrap();
        assert!(Segment::read(&other).is_err());
    }
}
