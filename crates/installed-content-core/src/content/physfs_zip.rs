//! An archive layer using the PhysFS 3.0 ZIP archiver's rules (`physfs_archiver_zip.cpp`),
//! mounting and reading the central directory's own records. Members stay keyed by their
//! name bytes; decoding names would merge members whose bytes differ.
//!
//! Mounting starts with a backward scan for the end-of-central-directory record.
//! Parsing first tries the Zip64 locator 20 bytes before it, then enters
//! every central record in the directory tree, creating missing ancestors and using
//! the record's name bytes. Any failure fails the whole mount; there is no partial archive.
//!
//! Opening ([`Mounted::lookup`] then [`Archive::read`]) finds the node by exactly
//! the requested bytes. Only when none exists, an archive holding an encrypted record
//! splits a request at its last `$`, treating the suffix as a password.
//! The node must not be a directory; opening checks its local header against its
//! central record. Traditional PKWARE keys decrypt an encrypted record's 12-byte header to
//! the check byte; a plain record opens only without a password.
//! An open that fails is no error: lookup goes on to the next search-path entry.
//!
//! Reading leaves stored data as it stands; every other method uses raw deflate
//! with window bits -15 and synchronous flushing, up to the central record's
//! uncompressed size. The CRC is not checked.
//!
//! Seeks past the end succeed; a read past it fails. Reads return all requested bytes or fail.

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

/// One parsed central-directory record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    /// The key its tree node has: the record's name bytes as they stand (no code page, and not
    /// the Info-ZIP Unicode path extra field), a trailing `/` removed, ending at the first NUL
    /// (the tree stores and compares a C string), `\` made `/` only when the version-made-by
    /// host byte is 0 (FAT). Empty for the root.
    pub name: LayerPath,
    /// Whether its tree node is a directory: a name that ended in `/`, or a node the tree made
    /// first as an ancestor of an earlier record, which a later file record of
    /// that name does not change. Opening a directory fails with a not-a-file error.
    pub directory: bool,
    /// A host that makes symbolic links (any but 0, 1, 2, 4, 6, 11, 13,
    /// 14, 15, 18), a nonzero uncompressed size and a Unix link mode in the external
    /// attributes' high half. Never set for a directory name.
    pub symlink: bool,
    /// The uncompressed size, which is what a read returns.
    pub size: u64,
    /// The DOS date and time, whose time's high byte checks an encrypted header when the record
    /// has a data descriptor (flag 8).
    time: u32,
    /// The local header's offset, the data before the archive (`data_start`) added.
    local: u64,
    version_needed: u16,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u64,
}

/// An open archive file.
pub(super) struct Archive {
    io: Cursor,
}

impl Archive {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
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

    /// The archive as the game mounts it; an error names why the game's mount fails.
    pub(super) fn mount(&mut self) -> io::Result<Mounted> {
        let directory = self.end_of_central_directory()?;
        let mut tree = Tree::default();
        let mut entries = Vec::new();
        self.io.seek(directory.offset);
        // Read one record after another, however many the end record counts;
        // each read moves forward, so a count the file does not hold ends at its end.
        let mut index = 0_u64;
        while index < directory.count {
            entries.push(self.load_entry(directory.zip64, directory.data_start, &mut tree)?);
            index += 1;
        }
        // Mark the archive as holding encryption when any record's
        // flag 1 is set.
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

    /// Native open admission: local header and password, with no payload read.
    pub(super) fn opens(&mut self, entry: &Entry, password: Option<&[u8]>) -> io::Result<bool> {
        self.open_entry(entry, password)
            .map(|opened| opened.is_some())
    }

    /// The mounted lookup and its opening headers, without reading any payload.
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

    fn opening(&mut self, entry: &Entry) -> io::Result<Option<Opening>> {
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
            check: if entry.flags & 8 != 0 {
                (entry.time >> 8) as u8
            } else {
                (entry.crc >> 24) as u8
            },
        }))
    }

    /// Opens and reads a file record of this archive's [`Mounted`] as the game does, with the
    /// password [`Mounted::lookup`] split off the request: `None` when the game's open fails
    /// (its search goes on to the next layer); an error for a read that comes up short, which
    /// the replay refuses.
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
    ) -> ObservedResult<Vec<u8>> {
        self.io.evidence = traced.then(ReadEvidence::default);
        let result = self.stored(entry);
        let evidence = self.io.evidence.take();
        result.map_err(|error| ReadFailure {
            error,
            payload: evidence
                .filter(|evidence| evidence.length != 0)
                .map(ReadEvidence::finish),
        })
    }

    /// Where the record's data starts, or `None` when its local header does not agree
    /// with its central record or the file ends inside it. Such a broken record fails to open.
    fn local_data(&mut self, entry: &Entry) -> io::Result<Option<u64>> {
        let io = &mut self.io;
        if entry.local.checked_add(30).is_none_or(|end| end > io.len) {
            return Ok(None);
        }
        io.seek(entry.local);
        let header = io.bytes(30)?;
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
            return Ok(None);
        }
        Ok(Some(entry.local.wrapping_add(
            30 + u64::from(u16_at(26)) + u64::from(u16_at(28)),
        )))
    }

    /// The bytes a record occupies as stored: its local header, name, extra field and
    /// compressed data (the lengths its local header and central record give), as far as the
    /// file holds them. With [`Entry::opening`], what names a record the game does not open
    /// without a password.
    pub(super) fn stored(&mut self, entry: &Entry) -> io::Result<Vec<u8>> {
        let len = self.io.len;
        let start = entry.local.min(len);
        let end = if entry.local.checked_add(30).is_some_and(|end| end <= len) {
            self.io.seek(entry.local);
            let header = self.io.bytes(30)?;
            let lengths = u64::from(u16::from_le_bytes([header[26], header[27]]))
                + u64::from(u16::from_le_bytes([header[28], header[29]]));
            entry
                .local
                .saturating_add(30 + lengths)
                .saturating_add(entry.compressed)
                .min(len)
        } else {
            len
        };
        self.io.seek(start);
        self.io.bytes(end - start)
    }

    /// Read the end record and locate the central directory, including any prefix bytes.
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

    /// Read the locator 20 bytes before the end record: `None` when no locator is there
    /// (not Zip64).
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

    /// Find the Zip64 end record at the offset the locator names, else 56 or 84
    /// bytes before the locator, else the last signature in the (at most 256 KiB) before it.
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

    /// Read the record at the cursor and leave it at the next record.
    fn load_entry(&mut self, zip64: bool, data_start: u64, tree: &mut Tree) -> io::Result<Entry> {
        let io = &mut self.io;
        if io.u32()? != CENTRAL_SIGNATURE {
            return Err(unmounted("a central record lacks its signature"));
        }
        let made_by = io.u16()?;
        let version_needed = io.u16()?;
        let flags = io.u16()?;
        let method = io.u16()?;
        // The DOS time `mktime` makes the node's modification time: never 0 (a DOS date is
        // 1980 or later), which is how a node already entered is told (below).
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
        name.truncate(crate::parser::c_string(&name).len());
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
            // The Zip64 extra field (id 1) holds, in this order, each of the three that is
            // `0xFFFFFFFF`, and nothing else; the remaining length is a `u16` that wraps.
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
    /// The central record's fields that decide whether and how the game opens and decodes the
    /// record, beyond its stored bytes: the version needed, method, CRC and both sizes, which
    /// opening checks the local header against (the method also choosing
    /// inflation, the sizes bounding it); the flags (1 encryption, 8 which byte checks a
    /// password); the DOS time, whose high byte checks a password under flag 8 and which
    /// no local field is compared with.
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

/// A mounted archive: its records and its directory tree.
#[derive(Clone, Debug)]
pub(super) struct Mounted {
    /// Every central-directory record, in its order.
    pub entries: Vec<Entry>,
    /// Every tree node: `None` a directory (the root, a directory record, an ancestor the tree
    /// made), else the file record it is.
    nodes: BTreeMap<LayerPath, Option<usize>>,
    /// Whether any record is encrypted (the archive's `has_crypto`).
    crypto: bool,
}

#[derive(Clone, Debug)]
struct Opening {
    data: u64,
    header: Option<[u8; 12]>,
    check: u8,
}

impl Opening {
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

/// Native open admission from the inventory, with no filesystem or payload reads.
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
                    .is_some_and(|opening| opening.keys(password).is_some())
            })
    }
}

impl Mounted {
    /// Look up a sanitized request, returning its file record and password.
    /// Return `None` when it names no node or a directory; the `$` split is tried only when no node
    /// has the request's name.
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

/// The traditional PKWARE keys.
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

    /// One byte decrypted, the keys updated with the plain byte.
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

/// One byte of the reflected CRC-32 (polynomial `0xEDB88320`), no inversion.
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

/// Where the central directory is.
struct Directory {
    zip64: bool,
    /// Bytes before the archive proper (a self-extractor's stub), added to every offset.
    data_start: u64,
    offset: u64,
    count: u64,
}

/// The mount's directory tree, by node name (the root is the empty name, a directory).
struct Tree {
    /// Whether each node is a directory, and whether a record has entered it.
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
    /// Add a record's name and check for duplicates, returning whether the node is a directory. A new node's missing ancestors are made
    /// directories, each the name up to its last `/`; an existing ancestor that is a file,
    /// or a node a record has entered already (its modification time is set),
    /// fails the mount as corrupt.
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

/// The archive file, read as PhysFS's native I/O reads it.
struct Cursor {
    file: BufReader<File>,
    len: u64,
    /// Where the next read starts.
    pos: u64,
    /// Where `file` stands.
    at: u64,
    evidence: Option<ReadEvidence>,
}

impl Cursor {
    fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// `n` bytes from the cursor, all or a failure.
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

    /// Find the end record's signature, scanning back from the end through
    /// 256-byte windows that overlap by 4, as far as 65557 bytes, with its quirks kept: a
    /// window's first byte is never a match (`i > 0`), and a window clamped to the file's start
    /// is completed by the previous window's first 4 bytes.
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

    /// Decode a member with raw deflate over at most `compressed` bytes of input,
    /// read 16 KiB at a time, until
    /// `size` bytes are out or the stream stops.
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

/// A read past the file's end, which fails the game's mount or open.
fn past_end() -> io::Error {
    invalid("the game's read runs past the archive's end, which fails its mount or open")
}

/// The game's mount of the archive fails.
fn unmounted(why: impl std::fmt::Display) -> io::Error {
    invalid(format!("the game does not mount this archive: {why}"))
}

/// The game's read of the member returns fewer bytes than its size.
fn short(why: &str) -> io::Error {
    invalid(format!("the game reads this archive member short ({why})"))
}
