
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

use miniz_oxide::{
    DataFormat, MZFlush, MZStatus,
    inflate::stream::{InflateState, inflate},
};

use super::{EvidenceReader, LayerPath, ObservedResult, ReadEvidence, ReadFailure, invalid};

const LOCAL_SIGNATURE: u32 = 0x0403_4B50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;
const END_SIGNATURE: u32 = 0x0605_4B50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4B50;
const ZIP64_END_SIGNATURE: u32 = 0x0606_4B50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub name: LayerPath,
    pub directory: bool,
    pub symlink: bool,
    pub size: u64,
    time: u32,
    local: u64,
    version_needed: u16,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u64,
}

pub(super) struct Archive {
    io: Cursor,
}

impl Archive {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        #[cfg(test)]
        super::tests::ARCHIVE_OPENS.with(|count| count.set(count.get() + 1));
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            io: Cursor {
                file: BufReader::new(file),
                len,
                pos: 0,
                at: 0,
                evidence: None,
            },
        })
    }

    pub(super) fn file(&self) -> &File {
        self.io.file.get_ref()
    }

    pub(super) fn mount(&mut self) -> io::Result<Mounted> {
        #[cfg(test)]
        super::tests::ARCHIVE_MOUNTS.with(|count| count.set(count.get() + 1));
        let directory = self.end_of_central_directory()?;
        let mut tree = Tree::default();
        let mut entries = Vec::new();
        self.io.seek(directory.offset);
        let mut index = 0_u64;
        while index < directory.count {
            entries.push(self.load_entry(directory.zip64, directory.data_start, &mut tree)?);
            index += 1;
        }
        let crypto = entries.iter().any(|entry| entry.flags & 1 != 0);
        let mut nodes: BTreeMap<LayerPath, Option<usize>> =
            tree.nodes.into_keys().map(|name| (name, None)).collect();
        for (index, entry) in entries.iter().enumerate() {
            if !entry.directory {
                nodes.insert(entry.name.clone(), Some(index));
            }
        }
        Ok(Mounted {
            entries,
            nodes,
            crypto,
        })
    }

    pub(super) fn index(&mut self, mounted: Mounted) -> io::Result<Index> {
        let mut openings = BTreeMap::new();
        for entry in &mounted.entries {
            if !entry.directory
                && let Some(opening) = self.opening(entry)?
            {
                openings.insert(entry.name.clone(), opening);
            }
        }
        Ok(Index { mounted, openings })
    }

    fn open_entry(
        &mut self,
        entry: &Entry,
        password: Option<&[u8]>,
    ) -> io::Result<Option<Option<Keys>>> {
        let Some(opening) = self.opening(entry)? else {
            return Ok(None);
        };
        self.io
            .seek(opening.data + u64::from(opening.header.is_some()) * 12);
        Ok(opening.keys(password))
    }

    pub(super) fn opening(&mut self, entry: &Entry) -> io::Result<Option<Opening>> {
        let Some(data) = self.local_data(entry)? else {
            return Ok(None);
        };
        let io = &mut self.io;
        io.seek(data);
        let header = if entry.flags & 1 != 0 {
            if data.checked_add(12).is_none_or(|end| end > io.len) {
                return Ok(None);
            }
            Some(io.bytes(12)?.try_into().unwrap())
        } else {
            None
        };
        Ok(Some(Opening {
            data,
            header,
            check: entry.check_byte(),
        }))
    }

    pub(super) fn read(
        &mut self,
        entry: &Entry,
        password: Option<&[u8]>,
    ) -> io::Result<Option<Vec<u8>>> {
        let Some(mut keys) = self.open_entry(entry, password)? else {
            return Ok(None);
        };
        let io = &mut self.io;
        let bytes = if entry.method == 0 {
            if io
                .pos
                .checked_add(entry.size)
                .is_none_or(|end| end > io.len)
            {
                return Err(short("its stored data ends before its uncompressed size"));
            }
            let mut bytes = io.bytes(entry.size)?;
            if let Some(keys) = &mut keys {
                keys.decrypt_all(&mut bytes);
            }
            bytes
        } else {
            io.inflate(entry.compressed, entry.size, keys.as_mut())?
        };
        Ok(Some(bytes))
    }

    pub(super) fn read_observed(
        &mut self,
        entry: &Entry,
        password: Option<&[u8]>,
        traced: bool,
    ) -> ObservedResult<Option<Vec<u8>>> {
        self.io.evidence = traced.then(ReadEvidence::default);
        let result = self.read(entry, password);
        let evidence = self.io.evidence.take();
        result.map_err(|error| ReadFailure {
            error,
            payload: evidence
                .filter(|evidence| evidence.length != 0)
                .map(ReadEvidence::finish),
        })
    }

    pub(super) fn stored_observed(
        &mut self,
        entry: &Entry,
        traced: bool,
    ) -> ObservedResult<Stored> {
        self.io.evidence = traced.then(ReadEvidence::default);
        let result = self.stored_record(entry);
        let evidence = self.io.evidence.take();
        result.map_err(|error| ReadFailure {
            error,
            payload: evidence
                .filter(|evidence| evidence.length != 0)
                .map(ReadEvidence::finish),
        })
    }

    fn local_data(&mut self, entry: &Entry) -> io::Result<Option<u64>> {
        let io = &mut self.io;
        if entry.local.checked_add(30).is_none_or(|end| end > io.len) {
            return Ok(None);
        }
        io.seek(entry.local);
        let header = io.bytes(30)?;
        Ok(parse_local_header(entry, &header).map(|offset| entry.local.wrapping_add(offset)))
    }

    #[cfg(test)]
    pub(super) fn stored(&mut self, entry: &Entry) -> io::Result<Vec<u8>> {
        let mut stored = self.stored_record(entry)?;
        stored.bytes.truncate(stored.digest_len);
        Ok(stored.bytes)
    }

    fn stored_record(&mut self, entry: &Entry) -> io::Result<Stored> {
        let len = self.io.len;
        let start = entry.local.min(len);
        let (digest_end, decode_end) = if entry.local.checked_add(30).is_some_and(|end| end <= len)
        {
            self.io.seek(entry.local);
            let header = self.io.bytes(30)?;
            let lengths = u64::from(u16::from_le_bytes([header[26], header[27]]))
                + u64::from(u16::from_le_bytes([header[28], header[29]]));
            let data = entry.local.saturating_add(30 + lengths);
            let digest_end = data.saturating_add(entry.compressed).min(len);
            let extra = u64::from(entry.encrypted()) * 12;
            let header_end = data.saturating_add(extra).min(len);
            let decode_end = if entry.method == 0 {
                header_end.max(digest_end)
            } else {
                data.saturating_add(extra)
                    .saturating_add(entry.compressed)
                    .max(digest_end)
                    .min(len)
            };
            (digest_end, decode_end)
        } else {
            (len, len)
        };
        self.io.seek(start);
        let bytes = self.io.bytes(decode_end - start)?;
        Ok(Stored {
            bytes,
            digest_len: (digest_end - start) as usize,
            file_len: len,
        })
    }

    fn end_of_central_directory(&mut self) -> io::Result<Directory> {
        let len = self.io.len;
        let Some(pos) = self.io.find_end()? else {
            return Err(unmounted(
                "no end-of-central-directory signature in the backward scan",
            ));
        };
        let io = &mut self.io;
        io.seek(pos);
        if io.u32()? != END_SIGNATURE {
            return Err(unmounted("its end record lacks its signature"));
        }
        if let Some(directory) = self.zip64(pos.checked_sub(20))? {
            return Ok(directory);
        }
        let io = &mut self.io;
        io.seek(pos + 4);
        if io.u16()? != 0 || io.u16()? != 0 {
            return Err(unmounted("its end record names another disk"));
        }
        let count = io.u16()?;
        if io.u16()? != count {
            return Err(unmounted("its end record's two entry counts differ"));
        }
        let size = u64::from(io.u32()?);
        let offset = u64::from(io.u32()?);
        if offset + size > pos {
            return Err(unmounted("its central directory runs past its end record"));
        }
        let data_start = pos - (offset + size);
        let comment = u64::from(io.u16()?);
        if pos + 22 + comment != len {
            return Err(unmounted(
                "its end record's comment does not end at the file's end",
            ));
        }
        Ok(Directory {
            zip64: false,
            data_start,
            offset: offset + data_start,
            count: u64::from(count),
        })
    }

    fn zip64(&mut self, locator: Option<u64>) -> io::Result<Option<Directory>> {
        let Some(locator) = locator else {
            return Err(unmounted(
                "its end record starts within 20 bytes of the file's start, leaving no room for \
                 a Zip64 locator",
            ));
        };
        let io = &mut self.io;
        io.seek(locator);
        if io.u32()? != ZIP64_LOCATOR_SIGNATURE {
            return Ok(None);
        }
        if io.u32()? != 0 {
            return Err(unmounted("its Zip64 locator names another disk"));
        }
        let offset = io.u64()?;
        if io.u32()? != 1 {
            return Err(unmounted("its Zip64 locator counts other than one disk"));
        }
        let found = self.find_zip64_end(locator, offset)?;
        if found < offset {
            return Err(unmounted(
                "its Zip64 end record lies before the offset its locator names",
            ));
        }
        let data_start = found - offset;
        let io = &mut self.io;
        io.seek(found);
        if io.u32()? != ZIP64_END_SIGNATURE {
            return Err(unmounted("its Zip64 end record lacks its signature"));
        }
        io.u64()?;
        io.u16()?;
        io.u16()?;
        if io.u32()? != 0 || io.u32()? != 0 {
            return Err(unmounted("its Zip64 end record names another disk"));
        }
        let count = io.u64()?;
        if io.u64()? != count {
            return Err(unmounted("its Zip64 end record's two entry counts differ"));
        }
        io.u64()?;
        let offset = io.u64()?.wrapping_add(data_start);
        Ok(Some(Directory {
            zip64: true,
            data_start,
            offset,
            count,
        }))
    }

    fn find_zip64_end(&mut self, locator: u64, offset: u64) -> io::Result<u64> {
        if locator == 0 {
            return Err(unmounted(
                "its Zip64 locator is at the file's start, leaving no preceding end record",
            ));
        }
        let io = &mut self.io;
        io.seek(offset);
        if io.u32()? == ZIP64_END_SIGNATURE {
            return Ok(offset);
        }
        for back in [56, 84] {
            if locator > back {
                io.seek(locator - back);
                if io.u32()? == ZIP64_END_SIGNATURE {
                    return Ok(locator - back);
                }
            }
        }
        if offset < locator && locator > 4 {
            let len = (locator - offset).min(256 * 1024);
            io.seek(locator - len);
            let buffer = io.bytes(len)?;
            if let Some(at) = buffer
                .windows(4)
                .rposition(|word| word == ZIP64_END_SIGNATURE.to_le_bytes())
            {
                return Ok(locator - len + at as u64);
            }
        }
        Err(unmounted(
            "no Zip64 end record at the locator offset or in the preceding scan",
        ))
    }

    fn load_entry(&mut self, zip64: bool, data_start: u64, tree: &mut Tree) -> io::Result<Entry> {
        let io = &mut self.io;
        if io.u32()? != CENTRAL_SIGNATURE {
            return Err(unmounted("a central record lacks its signature"));
        }
        let made_by = io.u16()?;
        let version_needed = io.u16()?;
        let flags = io.u16()?;
        let method = io.u16()?;
        let time = io.u32()?;
        let crc = io.u32()?;
        let mut compressed = u64::from(io.u32()?);
        let mut size = u64::from(io.u32()?);
        let name_length = io.u16()?;
        let extra_length = io.u16()?;
        let comment_length = io.u16()?;
        let disk = io.u16()?;
        io.u16()?;
        let external = io.u32()?;
        let mut offset = u64::from(io.u32()?);
        if name_length == 0 {
            return Err(invalid(
                "an archive record with an empty name: the ZIP loader tests the byte before its \
                 name buffer for `/` (and may write there), which the file does not determine",
            ));
        }
        let mut name = io.bytes(u64::from(name_length))?;
        let slash = name.last() == Some(&b'/');
        if slash {
            name.pop();
        }
        name.truncate(content_reader_parser::parser::c_string(&name).len());
        let host = (made_by >> 8) as u8;
        if host == 0 {
            for byte in &mut name {
                if *byte == b'\\' {
                    *byte = b'/';
                }
            }
        }
        let directory = tree.add(&name, slash)?;
        let symlink = !slash
            && !matches!(host, 0 | 1 | 2 | 4 | 6 | 11 | 13 | 14 | 15 | 18)
            && size != 0
            && (external >> 16) & 0xF000 == 0xA000;
        let mut pos = io.pos;
        let mut remaining = extra_length;
        if zip64 && (offset == 0xFFFF_FFFF || compressed == 0xFFFF_FFFF || size == 0xFFFF_FFFF) {
            loop {
                if remaining <= 4 {
                    return Err(unmounted(format!(
                        "record {} lacks the Zip64 field its sizes call for",
                        super::display(&name)
                    )));
                }
                let id = io.u16()?;
                let mut length = io.u16()?;
                pos += 4 + u64::from(length);
                remaining = remaining.wrapping_sub(4).wrapping_sub(length);
                if id == 1 {
                    for value in [&mut size, &mut compressed, &mut offset] {
                        if *value == 0xFFFF_FFFF {
                            if length < 8 {
                                return Err(unmounted(format!(
                                    "record {}'s Zip64 field is too short",
                                    super::display(&name)
                                )));
                            }
                            *value = io.u64()?;
                            length -= 8;
                        }
                    }
                    if length != 0 {
                        return Err(unmounted(format!(
                            "record {}'s Zip64 field holds more than its sizes",
                            super::display(&name)
                        )));
                    }
                    break;
                }
                io.seek(pos);
            }
        }
        if disk != 0 {
            return Err(unmounted(format!(
                "record {} starts on another disk",
                super::display(&name)
            )));
        }
        io.seek(pos + u64::from(remaining) + u64::from(comment_length));
        Ok(Entry {
            name,
            directory,
            symlink,
            size,
            local: offset.wrapping_add(data_start),
            version_needed,
            flags,
            method,
            crc,
            compressed,
            time,
        })
    }
}

impl Entry {
    pub(super) fn encrypted(&self) -> bool {
        self.flags & 1 != 0
    }

    fn check_byte(&self) -> u8 {
        if self.flags & 8 != 0 {
            (self.time >> 8) as u8
        } else {
            (self.crc >> 24) as u8
        }
    }

    pub(super) fn opening(&self) -> Vec<u8> {
        [
            &self.version_needed.to_le_bytes()[..],
            &self.flags.to_le_bytes(),
            &self.method.to_le_bytes(),
            &self.crc.to_le_bytes(),
            &self.compressed.to_le_bytes(),
            &self.size.to_le_bytes(),
            &self.time.to_le_bytes(),
        ]
        .concat()
    }
}

#[derive(Clone, Debug)]
pub(super) struct Mounted {
    pub entries: Vec<Entry>,
    nodes: BTreeMap<LayerPath, Option<usize>>,
    crypto: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Opening {
    data: u64,
    header: Option<[u8; 12]>,
    check: u8,
}

impl Opening {
    pub(super) fn opens(&self, password: Option<&[u8]>) -> bool {
        self.keys(password).is_some()
    }

    fn keys(&self, password: Option<&[u8]>) -> Option<Option<Keys>> {
        match (&self.header, password) {
            (None, None) => Some(None),
            (Some(header), Some(password)) => {
                let mut keys = Keys::new(password);
                let check = header.iter().fold(0, |_, byte| keys.decrypt(*byte));
                (check == self.check).then_some(Some(keys))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Index {
    pub mounted: Mounted,
    openings: BTreeMap<LayerPath, Opening>,
}

impl Index {
    pub(super) fn opens(&self, request: &[u8]) -> bool {
        self.mounted
            .lookup(request)
            .is_some_and(|(entry, password)| {
                self.openings
                    .get(&entry.name)
                    .is_some_and(|opening| opening.opens(password))
            })
    }

    pub(super) fn opening(&self, name: &[u8]) -> Option<&Opening> {
        self.openings.get(name)
    }
}

fn parse_local_header(entry: &Entry, header: &[u8]) -> Option<u64> {
    let u16_at = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let agrees = |local: u32, central: u64| {
        local == 0 || local == u32::MAX || u64::from(local) == central
    };
    if u32_at(0) != LOCAL_SIGNATURE
        || u16_at(4) != entry.version_needed
        || u16_at(8) != entry.method
        || (u32_at(14) != 0 && u32_at(14) != entry.crc)
        || !agrees(u32_at(18), entry.compressed)
        || !agrees(u32_at(22), entry.size)
    {
        return None;
    }
    Some(30 + u64::from(u16_at(26)) + u64::from(u16_at(28)))
}

pub(super) struct Stored {
    bytes: Vec<u8>,
    digest_len: usize,
    file_len: u64,
}

impl Stored {
    pub(super) fn digest_bytes(&self) -> &[u8] {
        &self.bytes[..self.digest_len]
    }
}

pub(super) fn decode_stored(
    entry: &Entry,
    stored: &Stored,
    password: Option<&[u8]>,
    traced: bool,
) -> ObservedResult<Option<Vec<u8>>> {
    let raw = &stored.bytes;
    let mut evidence = traced.then(ReadEvidence::default);
    if raw.len() < 30 {
        return Ok(None);
    }
    let header = &raw[..30];
    if let Some(evidence) = &mut evidence {
        evidence.update(header);
    }
    let Some(data_rel) = parse_local_header(entry, header) else {
        return Ok(None);
    };
    let Ok(data_rel) = usize::try_from(data_rel) else {
        return Ok(None);
    };
    let (mut keys, pos) = if entry.flags & 1 != 0 {
        let Some(end) = data_rel.checked_add(12) else {
            return Ok(None);
        };
        if end > raw.len() {
            return Ok(None);
        }
        let header_bytes: [u8; 12] = raw[data_rel..end].try_into().unwrap();
        if let Some(evidence) = &mut evidence {
            evidence.update(&header_bytes);
        }
        let opening = Opening {
            data: entry.local.wrapping_add(data_rel as u64),
            header: Some(header_bytes),
            check: entry.check_byte(),
        };
        let Some(keys) = opening.keys(password) else {
            return Ok(None);
        };
        (keys, end)
    } else {
        if password.is_some() {
            return Ok(None);
        }
        (None, data_rel)
    };
    let verified_payload = if pos <= stored.digest_len {
        Some(&raw[pos..stored.digest_len])
    } else {
        None
    };
    let result = if entry.method == 0 {
        if entry
            .local
            .checked_add(pos as u64)
            .and_then(|start| start.checked_add(entry.size))
            .is_none_or(|end| end > stored.file_len)
        {
            Err(short("its stored data ends before its uncompressed size"))
        } else if let Some(verified) = verified_payload
            && usize::try_from(entry.size).is_ok_and(|size| size <= verified.len())
        {
            let size = entry.size as usize;
            let mut bytes = verified[..size].to_vec();
            if let Some(evidence) = &mut evidence {
                evidence.update(&bytes);
            }
            if let Some(keys) = &mut keys {
                keys.decrypt_all(&mut bytes);
            }
            Ok(bytes)
        } else {
            Err(invalid(
                "the archive member reads past its verified stored range",
            ))
        }
    } else {
        let verified = verified_payload.and_then(|slice| {
            let mut verified_keys = keys.clone();
            inflate_slice(
                slice,
                entry.compressed.min(slice.len() as u64),
                entry.size,
                verified_keys.as_mut(),
                None,
            )
            .ok()
        });
        if let Some(bytes) = verified {
            Ok(bytes)
        } else {
            match inflate_slice(
                &raw[pos..],
                entry.compressed,
                entry.size,
                keys.as_mut(),
                evidence.as_mut(),
            ) {
                Ok(_) => Err(invalid(
                    "the archive member reads past its verified stored range",
                )),
                Err(error) => Err(error),
            }
        }
    };
    result.map(Some).map_err(|error| ReadFailure {
        error,
        payload: evidence
            .filter(|evidence| evidence.length != 0)
            .map(ReadEvidence::finish),
    })
}

fn inflate_slice(
    payload: &[u8],
    compressed: u64,
    size: u64,
    mut keys: Option<&mut Keys>,
    mut evidence: Option<&mut ReadEvidence>,
) -> io::Result<Vec<u8>> {
    let mut state = InflateState::new_boxed(DataFormat::Raw);
    let mut out = Vec::new();
    let (mut input, mut used, mut offset, mut left) = (Vec::new(), 0, 0usize, compressed);
    let mut chunk = vec![0; 0x10000];
    while (out.len() as u64) < size {
        if used == input.len() && left > 0 {
            let avail = payload.len().saturating_sub(offset) as u64;
            let n = left.min(0x4000).min(avail) as usize;
            if n == 0 {
                break;
            }
            let slice = &payload[offset..offset + n];
            if let Some(evidence) = evidence.as_deref_mut() {
                evidence.update(slice);
            }
            input = slice.to_vec();
            if let Some(keys) = keys.as_deref_mut() {
                keys.decrypt_all(&mut input);
            }
            offset += n;
            used = 0;
            left -= n as u64;
        }
        let want = (size - out.len() as u64).min(chunk.len() as u64) as usize;
        let result = inflate(
            &mut state,
            &input[used..],
            &mut chunk[..want],
            MZFlush::None,
        );
        used += result.bytes_consumed;
        out.extend_from_slice(&chunk[..result.bytes_written]);
        if result.status != Ok(MZStatus::Ok)
            || (result.bytes_consumed == 0 && result.bytes_written == 0 && left == 0)
        {
            break;
        }
    }
    if (out.len() as u64) < size {
        return Err(short(
            "it inflates to fewer bytes than its uncompressed size",
        ));
    }
    Ok(out)
}

impl Mounted {
    pub(super) fn lookup<'a>(&'a self, request: &'a [u8]) -> Option<(&'a Entry, Option<&'a [u8]>)> {
        let (name, password) = match self.nodes.get(request) {
            Some(_) => (request, None),
            None if self.crypto => {
                let dollar = request.iter().rposition(|byte| *byte == b'$')?;
                (&request[..dollar], Some(&request[dollar + 1..]))
            }
            None => return None,
        };
        let index = (*self.nodes.get(name)?)?;
        Some((&self.entries[index], password))
    }
}

#[cfg(test)]
pub(super) fn encrypt_zipcrypto(password: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut keys = Keys::new(password);
    let mut out = Vec::with_capacity(plain.len());
    for &byte in plain {
        let key = (keys.0[2] & 0xFFFF) | 2;
        out.push(byte ^ ((key * (key ^ 1)) >> 8) as u8);
        keys.update(byte);
    }
    out
}

#[derive(Clone)]
struct Keys([u32; 3]);

impl Keys {
    fn new(password: &[u8]) -> Self {
        let mut keys = Self([0x1234_5678, 0x2345_6789, 0x3456_7890]);
        for byte in password {
            keys.update(*byte);
        }
        keys
    }

    fn update(&mut self, byte: u8) {
        self.0[0] = crc_step(self.0[0], byte);
        self.0[1] = self.0[1]
            .wrapping_add(self.0[0] & 0xFF)
            .wrapping_mul(0x0808_8405)
            .wrapping_add(1);
        self.0[2] = crc_step(self.0[2], (self.0[1] >> 24) as u8);
    }

    fn decrypt(&mut self, byte: u8) -> u8 {
        let key = (self.0[2] & 0xFFFF) | 2;
        let plain = byte ^ ((key * (key ^ 1)) >> 8) as u8;
        self.update(plain);
        plain
    }

    fn decrypt_all(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            *byte = self.decrypt(*byte);
        }
    }
}

fn crc_step(crc: u32, byte: u8) -> u32 {
    let mut value = (crc ^ u32::from(byte)) & 0xFF;
    for _ in 0..8 {
        value = if value & 1 != 0 {
            (value >> 1) ^ 0xEDB8_8320
        } else {
            value >> 1
        };
    }
    value ^ (crc >> 8)
}

struct Directory {
    zip64: bool,
    data_start: u64,
    offset: u64,
    count: u64,
}

struct Tree {
    nodes: BTreeMap<LayerPath, (bool, bool)>,
}

impl Default for Tree {
    fn default() -> Self {
        Self {
            nodes: BTreeMap::from([(Vec::new(), (true, false))]),
        }
    }
}

impl Tree {
    fn add(&mut self, name: &[u8], directory: bool) -> io::Result<bool> {
        if !self.nodes.contains_key(name) {
            let mut missing = vec![(name.to_vec(), directory)];
            let mut child = name;
            while let Some(slash) = child.iter().rposition(|byte| *byte == b'/') {
                let parent = &child[..slash];
                match self.nodes.get(parent) {
                    Some((true, _)) => break,
                    Some((false, _)) => {
                        return Err(unmounted(format!(
                            "record {} lies below {}, a file",
                            super::display(name),
                            super::display(parent)
                        )));
                    }
                    None => missing.push((parent.to_vec(), true)),
                }
                child = parent;
            }
            for (path, directory) in missing {
                self.nodes.insert(path, (directory, false));
            }
        }
        let node = self
            .nodes
            .get_mut(name)
            .expect("the node was just found or made");
        if node.1 {
            return Err(unmounted(format!(
                "two records name {}",
                super::display(name)
            )));
        }
        node.1 = true;
        Ok(node.0)
    }
}

struct Cursor {
    file: BufReader<File>,
    len: u64,
    pos: u64,
    at: u64,
    evidence: Option<ReadEvidence>,
}

impl Cursor {
    fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    fn bytes(&mut self, n: u64) -> io::Result<Vec<u8>> {
        if self.pos.checked_add(n).is_none_or(|end| end > self.len) {
            return Err(past_end());
        }
        let mut buffer = vec![0; n as usize];
        self.read(&mut buffer)?;
        Ok(buffer)
    }

    fn read(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        let n = buffer.len() as u64;
        if self.pos.checked_add(n).is_none_or(|end| end > self.len) {
            return Err(past_end());
        }
        if self.pos != self.at {
            match self.pos.checked_sub(self.at).map(i64::try_from) {
                Some(Ok(forward)) => self.file.seek_relative(forward)?,
                _ => {
                    self.file.seek(SeekFrom::Start(self.pos))?;
                }
            }
        }
        if let Some(evidence) = &mut self.evidence {
            EvidenceReader {
                reader: &mut self.file,
                evidence,
            }
            .read_exact(buffer)?;
        } else {
            self.file.read_exact(buffer)?;
        }
        self.pos += n;
        self.at = self.pos;
        Ok(())
    }

    fn u16(&mut self) -> io::Result<u16> {
        let mut bytes = [0; 2];
        self.read(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> io::Result<u32> {
        let mut bytes = [0; 4];
        self.read(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> io::Result<u64> {
        let mut bytes = [0; 8];
        self.read(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn find_end(&mut self) -> io::Result<Option<u64>> {
        let len = self.len as i64;
        if len < 1 {
            return Ok(None);
        }
        let (mut start, window) = if len > 256 {
            (len - 256, 256)
        } else {
            (0, len)
        };
        let mut buffer = [0_u8; 256];
        let mut carried = [0_u8; 4];
        let mut total = 0_i64;
        while total < len && total < 65557 {
            self.seek(start as u64);
            if total != 0 {
                let fresh = (window - 4) as usize;
                self.read(&mut buffer[..fresh])?;
                buffer[fresh..fresh + 4].copy_from_slice(&carried);
                total += window - 4;
            } else {
                self.read(&mut buffer[..window as usize])?;
                total = window;
            }
            carried.copy_from_slice(&buffer[..4]);
            let mut i = window - 4;
            while i > 0 {
                let at = i as usize;
                if buffer[at..at + 4] == END_SIGNATURE.to_le_bytes() {
                    return Ok(Some((start + i) as u64));
                }
                i -= 1;
            }
            start = (start - (window - 4)).max(0);
        }
        Ok(None)
    }

    fn inflate(
        &mut self,
        compressed: u64,
        size: u64,
        mut keys: Option<&mut Keys>,
    ) -> io::Result<Vec<u8>> {
        let mut state = InflateState::new_boxed(DataFormat::Raw);
        let mut out = Vec::new();
        let (mut input, mut used, mut left) = (Vec::new(), 0, compressed);
        let mut chunk = vec![0; 0x10000];
        while (out.len() as u64) < size {
            if used == input.len() && left > 0 {
                let n = left.min(0x4000).min(self.len.saturating_sub(self.pos));
                if n == 0 {
                    break;
                }
                input = self.bytes(n)?;
                if let Some(keys) = keys.as_deref_mut() {
                    keys.decrypt_all(&mut input);
                }
                used = 0;
                left -= n;
            }
            let want = (size - out.len() as u64).min(chunk.len() as u64) as usize;
            let result = inflate(
                &mut state,
                &input[used..],
                &mut chunk[..want],
                MZFlush::None,
            );
            used += result.bytes_consumed;
            out.extend_from_slice(&chunk[..result.bytes_written]);
            if result.status != Ok(MZStatus::Ok)
                || (result.bytes_consumed == 0 && result.bytes_written == 0 && left == 0)
            {
                break;
            }
        }
        if (out.len() as u64) < size {
            return Err(short(
                "it inflates to fewer bytes than its uncompressed size",
            ));
        }
        Ok(out)
    }
}

fn past_end() -> io::Error {
    invalid("the game's read runs past the archive's end, which fails its mount or open")
}

fn unmounted(why: impl std::fmt::Display) -> io::Error {
    invalid(format!("the game does not mount this archive: {why}"))
}

fn short(why: &str) -> io::Error {
    invalid(format!("the game reads this archive member short ({why})"))
}
