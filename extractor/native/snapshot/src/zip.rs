//! A minimal, hardened, READ-ONLY ZIP reader for the `.battleplan` container.
//!
//! Scope on purpose: just enough to enumerate the central directory and read one
//! entry's decompressed bytes (STORE, DEFLATE, or Zstandard method 93). It does
//! NOT extract to disk, so zip-slip/symlink/device-name *extraction* attacks do
//! not apply to this path, but it still validates every path (spec section 4.2)
//! so a caller that does materialize files is handed only safe names.
//!
//! Hardening: rejects duplicate entry names, validates paths, supports Zip64
//! metadata explicitly, cross-checks every local file header against the central
//! directory, provides whole-archive payload CRC validation, enforces the
//! declared uncompressed size per entry, and keeps the in-RAM reader's 2 GiB
//! materialization cap separate from the streaming verifier's larger multi-GiB
//! cap.

use flate2::read::DeflateDecoder;
use std::collections::HashSet;
use std::io::Read;

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const DATA_DESCRIPTOR_SIG: u32 = 0x0807_4b50;
const ZIP64_EXTRA_ID: u16 = 0x0001;

pub const ZIP_METHOD_STORE: u16 = 0;
pub const ZIP_METHOD_DEFLATE: u16 = 8;
/// Zstandard compression method registered in APPNOTE.TXT as method 93.
pub const ZIP_METHOD_ZSTD: u16 = 93;
/// Writer/reader zstd window. `2^27` = 128 MiB.
pub const ZSTD_WINDOW_LOG: u32 = 27;

/// ZIP32 scalar-field maximum that is not the Zip64 sentinel.
pub const ZIP32_MAX_FIELD_VALUE: u64 = (u32::MAX as u64) - 1;
/// ZIP32 EOCD entry-count maximum that is not the Zip64 sentinel.
pub const ZIP32_MAX_ENTRY_COUNT: usize = (u16::MAX as usize) - 1;
/// Per-entry cap for the in-RAM container reader. It materializes entries.
pub const MAX_CONTAINER_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Per-entry cap for streaming verification/writing paths. These paths never
/// materialize the entry and exist specifically for multi-GiB compact streams.
pub const STREAMING_MAX_CONTAINER_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
/// Hard central-directory entry-count cap enforced before allocating entry vectors.
/// Ten million entries is far above any plausible replay and prevents hostile
/// Zip64 EOCD counts from driving unbounded reserves before CD validation.
pub const MAX_READER_ENTRIES: u64 = 10_000_000;
/// Initial reserve cap for parsed entries; the parser grows if needed after the
/// central directory proves real instead of trusting an EOCD-controlled count.
pub const INITIAL_ENTRY_RESERVE: usize = 64 * 1024;

/// A single central-directory record we care about.
#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub version_needed: u16,
    pub flags: u16,
    pub method: u16,
    pub mod_time: u16,
    pub mod_date: u16,
    pub crc32: u32,
    pub comp_size: u64,
    pub uncomp_size: u64,
    /// offset of the local file header within the archive
    pub local_header_off: u64,
}

#[derive(Debug)]
pub enum ZipError {
    Truncated(&'static str),
    BadSignature(&'static str),
    Unsupported(String),
    DuplicateName(String),
    BadPath(String),
    SizeMismatch {
        name: String,
        declared: u64,
        got: u64,
    },
    Decompress(String),
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipError::Truncated(w) => write!(f, "zip truncated: {w}"),
            ZipError::BadSignature(w) => write!(f, "zip bad signature: {w}"),
            ZipError::Unsupported(w) => write!(f, "zip unsupported: {w}"),
            ZipError::DuplicateName(n) => write!(f, "zip duplicate entry name: {n}"),
            ZipError::BadPath(n) => write!(f, "zip unsafe path: {n}"),
            ZipError::SizeMismatch {
                name,
                declared,
                got,
            } => write!(
                f,
                "zip entry {name}: decompressed {got} bytes != declared {declared}"
            ),
            ZipError::Decompress(e) => write!(f, "zip decompress: {e}"),
        }
    }
}
impl std::error::Error for ZipError {}

fn rd_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(off)?, *b.get(off + 1)?]))
}
fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(off)?,
        *b.get(off + 1)?,
        *b.get(off + 2)?,
        *b.get(off + 3)?,
    ]))
}
fn rd_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes([
        *b.get(off)?,
        *b.get(off + 1)?,
        *b.get(off + 2)?,
        *b.get(off + 3)?,
        *b.get(off + 4)?,
        *b.get(off + 5)?,
        *b.get(off + 6)?,
        *b.get(off + 7)?,
    ]))
}

/// A ZIP archive held entirely in memory (the per-snapshot caller still reads
/// one entry at a time).
pub struct ZipArchive {
    data: Vec<u8>,
    entries: Vec<ZipEntry>,
    central_directory_off: usize,
}

struct LocalFileHeader<'a> {
    version_needed: u16,
    flags: u16,
    method: u16,
    mod_time: u16,
    mod_date: u16,
    crc32: u32,
    comp_size32: u32,
    uncomp_size32: u32,
    name: &'a [u8],
    extra: &'a [u8],
    data_start: usize,
}

#[derive(Clone, Copy)]
struct LocalEntryRegion {
    header_start: usize,
    data_start: usize,
    data_end: usize,
    region_end: usize,
}

impl ZipArchive {
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Parse the central directory of an in-memory ZIP, validating + de-duping
    /// every name. Supports Zip64 metadata; `read_entry` still enforces the
    /// in-RAM materialization cap.
    pub fn parse(data: Vec<u8>) -> Result<ZipArchive, ZipError> {
        let eocd = find_eocd(&data).ok_or(ZipError::Truncated("no EOCD record"))?;
        let disk_no = rd_u16(&data, eocd + 4).ok_or(ZipError::Truncated("eocd"))?;
        let cd_disk = rd_u16(&data, eocd + 6).ok_or(ZipError::Truncated("eocd"))?;
        if disk_no != 0 || cd_disk != 0 {
            return Err(ZipError::Unsupported(
                "multi-disk ZIP archives are not supported".into(),
            ));
        }
        let entries_on_disk16 = rd_u16(&data, eocd + 8).ok_or(ZipError::Truncated("eocd"))?;
        let total16 = rd_u16(&data, eocd + 10).ok_or(ZipError::Truncated("eocd"))?;
        let cd_size32 = rd_u32(&data, eocd + 12).ok_or(ZipError::Truncated("eocd"))?;
        let cd_off32 = rd_u32(&data, eocd + 16).ok_or(ZipError::Truncated("eocd"))?;
        let locator = find_zip64_locator(&data, eocd);
        let zip64_in_effect = entries_on_disk16 == u16::MAX
            || total16 == u16::MAX
            || cd_size32 == u32::MAX
            || cd_off32 == u32::MAX
            || locator.is_some();
        let (total, cd_size_u64, cd_off_u64, expected_cd_end) = if zip64_in_effect {
            let locator =
                locator.ok_or_else(|| ZipError::Unsupported("Zip64 locator missing".into()))?;
            let (zip64_total, zip64_cd_size, zip64_cd_off, zip64_eocd_off) =
                parse_zip64_eocd(&data, locator)?;
            validate_ordinary_zip64_field(
                "entries-on-disk count",
                entries_on_disk16 as u64,
                u16::MAX as u64,
                zip64_total,
            )?;
            validate_ordinary_zip64_field(
                "total-entry count",
                total16 as u64,
                u16::MAX as u64,
                zip64_total,
            )?;
            validate_ordinary_zip64_field(
                "central-directory size",
                cd_size32 as u64,
                u32::MAX as u64,
                zip64_cd_size,
            )?;
            validate_ordinary_zip64_field(
                "central-directory offset",
                cd_off32 as u64,
                u32::MAX as u64,
                zip64_cd_off,
            )?;
            (zip64_total, zip64_cd_size, zip64_cd_off, zip64_eocd_off)
        } else {
            if entries_on_disk16 != total16 {
                return Err(ZipError::Unsupported(
                    "split ZIP central directories are not supported".into(),
                ));
            }
            (
                total16 as u64,
                cd_size32 as u64,
                cd_off32 as u64,
                eocd as u64,
            )
        };
        let actual_cd_end = cd_off_u64
            .checked_add(cd_size_u64)
            .ok_or_else(|| ZipError::Unsupported("central directory range overflows".into()))?;
        if actual_cd_end != expected_cd_end {
            return Err(ZipError::Unsupported(format!(
                "central directory does not end at its end record: {actual_cd_end} != {expected_cd_end}"
            )));
        }
        if total > MAX_READER_ENTRIES {
            return Err(ZipError::Unsupported(format!(
                "entry count {total} exceeds reader cap {MAX_READER_ENTRIES}"
            )));
        }
        let total = usize::try_from(total)
            .map_err(|_| ZipError::Unsupported("entry count does not fit usize".into()))?;
        let cd_size = usize::try_from(cd_size_u64)
            .map_err(|_| ZipError::Unsupported("central directory too large".into()))?;
        let cd_off = usize::try_from(cd_off_u64)
            .map_err(|_| ZipError::Unsupported("central directory offset too large".into()))?;
        if cd_off.saturating_add(cd_size) > data.len() {
            return Err(ZipError::Truncated("central directory out of range"));
        }

        let mut entries = Vec::with_capacity(total.min(INITIAL_ENTRY_RESERVE));
        let mut seen: HashSet<String> = HashSet::new();
        let mut seen_ci: HashSet<String> = HashSet::new();
        let mut p = cd_off;
        let cd_end = cd_off + cd_size;
        while p + 46 <= cd_end {
            let sig = rd_u32(&data, p).ok_or(ZipError::Truncated("cdh"))?;
            if sig != CDH_SIG {
                break;
            }
            let version_needed = rd_u16(&data, p + 6).ok_or(ZipError::Truncated("cdh"))?;
            let flags = rd_u16(&data, p + 8).ok_or(ZipError::Truncated("cdh"))?;
            if flags & !0x0008 != 0 {
                return Err(ZipError::Unsupported(format!(
                    "entry has unsupported zip general-purpose flags 0x{flags:04x}"
                )));
            }
            let method = rd_u16(&data, p + 10).ok_or(ZipError::Truncated("cdh"))?;
            let mod_time = rd_u16(&data, p + 12).ok_or(ZipError::Truncated("cdh"))?;
            let mod_date = rd_u16(&data, p + 14).ok_or(ZipError::Truncated("cdh"))?;
            let crc32 = rd_u32(&data, p + 16).ok_or(ZipError::Truncated("cdh"))?;
            let comp_size32 = rd_u32(&data, p + 20).ok_or(ZipError::Truncated("cdh"))?;
            let uncomp_size32 = rd_u32(&data, p + 24).ok_or(ZipError::Truncated("cdh"))?;
            let name_len = rd_u16(&data, p + 28).ok_or(ZipError::Truncated("cdh"))? as usize;
            let extra_len = rd_u16(&data, p + 30).ok_or(ZipError::Truncated("cdh"))? as usize;
            let comment_len = rd_u16(&data, p + 32).ok_or(ZipError::Truncated("cdh"))? as usize;
            let disk_start = rd_u16(&data, p + 34).ok_or(ZipError::Truncated("cdh"))?;
            if disk_start != 0 {
                return Err(ZipError::Unsupported(format!(
                    "entry at central-directory offset {p} starts on disk {disk_start}; multi-disk ZIP archives are not supported"
                )));
            }
            let lho32 = rd_u32(&data, p + 42).ok_or(ZipError::Truncated("cdh"))?;

            let name_start = p + 46;
            let name_end = name_start + name_len;
            if name_end > cd_end {
                return Err(ZipError::Truncated("cdh name"));
            }
            let raw_name = &data[name_start..name_end];
            let name = std::str::from_utf8(raw_name)
                .map_err(|_| ZipError::BadPath("<non-utf8>".into()))?
                .to_string();
            let extra_start = name_end;
            let extra_end = extra_start
                .checked_add(extra_len)
                .ok_or(ZipError::Truncated("cdh extra overflow"))?;
            if extra_end > cd_end {
                return Err(ZipError::Truncated("cdh extra past central dir"));
            }
            let extra = &data[extra_start..extra_end];
            let need_uncomp = uncomp_size32 == u32::MAX;
            let need_comp = comp_size32 == u32::MAX;
            let need_lho = lho32 == u32::MAX;
            let zip64 = parse_zip64_extra(extra, need_uncomp, need_comp, need_lho)?;
            let uncomp_size = if need_uncomp {
                zip64.uncomp_size.ok_or_else(|| {
                    ZipError::Unsupported(format!("Zip64 uncompressed size missing for {name}"))
                })?
            } else {
                uncomp_size32 as u64
            };
            let comp_size = if need_comp {
                zip64.comp_size.ok_or_else(|| {
                    ZipError::Unsupported(format!("Zip64 compressed size missing for {name}"))
                })?
            } else {
                comp_size32 as u64
            };
            let lho = if need_lho {
                zip64.local_header_off.ok_or_else(|| {
                    ZipError::Unsupported(format!("Zip64 local header offset missing for {name}"))
                })?
            } else {
                lho32 as u64
            };

            validate_path(&name)?;
            if !seen.insert(name.clone()) {
                return Err(ZipError::DuplicateName(name));
            }
            // case-insensitive collision (would clobber on Windows/macOS unpack)
            if !seen_ci.insert(name.to_ascii_lowercase()) {
                return Err(ZipError::DuplicateName(format!(
                    "{name} (case-insensitive)"
                )));
            }

            entries.push(ZipEntry {
                name,
                version_needed,
                flags,
                method,
                mod_time,
                mod_date,
                crc32,
                comp_size,
                uncomp_size,
                local_header_off: lho,
            });
            let next = name_end
                .checked_add(extra_len)
                .and_then(|v| v.checked_add(comment_len))
                .ok_or(ZipError::Truncated("cdh advance overflow"))?;
            if next > cd_end {
                return Err(ZipError::Truncated("cdh extra/comment past central dir"));
            }
            p = next;
        }
        if entries.len() != total {
            return Err(ZipError::Truncated(
                "central directory entry count mismatch",
            ));
        }
        if p != cd_end {
            return Err(ZipError::Truncated("central directory trailing bytes"));
        }
        Ok(ZipArchive {
            data,
            entries,
            central_directory_off: cd_off,
        })
    }

    pub fn by_name(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Cross-check every local file header against its central-directory record
    /// before callers trust any payload. Central-directory order must also be
    /// physical local-header order, and complete entry regions may not overlap or
    /// extend into the central directory.
    pub fn validate_local_headers(&self) -> Result<(), ZipError> {
        let mut previous: Option<(&ZipEntry, LocalEntryRegion)> = None;
        for entry in &self.entries {
            let current = self.validate_local_entry(entry)?;
            if let Some((previous_entry, previous_region)) = previous {
                if previous_region.header_start >= current.header_start {
                    return Err(ZipError::Unsupported(format!(
                        "local header order disagrees with central directory: {} at {} then {} at {}",
                        previous_entry.name,
                        previous_region.header_start,
                        entry.name,
                        current.header_start
                    )));
                }
                if previous_region.region_end > current.header_start {
                    return Err(ZipError::Unsupported(format!(
                        "entry regions overlap: {} ends at {}, {} starts at {}",
                        previous_entry.name,
                        previous_region.region_end,
                        entry.name,
                        current.header_start
                    )));
                }
            }
            previous = Some((entry, current));
        }
        Ok(())
    }

    /// Re-derive the uncompressed length and CRC-32 for every stored payload.
    /// Compressed entries are decoded through a declared-size-bounded reader, so
    /// callers can gate archive open without materializing every entry.
    pub fn validate_payloads(&self) -> Result<(), ZipError> {
        for entry in &self.entries {
            let region = self.validate_local_entry(entry)?;
            self.validate_payload_crc(entry, region)?;
        }
        Ok(())
    }

    fn read_local_header<'a>(&'a self, entry: &ZipEntry) -> Result<LocalFileHeader<'a>, ZipError> {
        let header_start = usize::try_from(entry.local_header_off).map_err(|_| {
            ZipError::Unsupported(format!(
                "entry {}: local header offset {} is too large",
                entry.name, entry.local_header_off
            ))
        })?;
        let header_end = header_start.checked_add(30).ok_or_else(|| {
            ZipError::Unsupported(format!(
                "entry {}: local header range overflows",
                entry.name
            ))
        })?;
        if header_end > self.data.len() {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local header at {} is truncated",
                entry.name, header_start
            )));
        }
        if rd_u32(&self.data, header_start) != Some(LFH_SIG) {
            return Err(ZipError::Unsupported(format!(
                "entry {}: bad local file header signature at {}",
                entry.name, header_start
            )));
        }
        let name_len =
            rd_u16(&self.data, header_start + 26).ok_or(ZipError::Truncated("lfh"))? as usize;
        let extra_len =
            rd_u16(&self.data, header_start + 28).ok_or(ZipError::Truncated("lfh"))? as usize;
        let name_end = header_end.checked_add(name_len).ok_or_else(|| {
            ZipError::Unsupported(format!(
                "entry {}: local header name range overflows",
                entry.name
            ))
        })?;
        let extra_end = name_end.checked_add(extra_len).ok_or_else(|| {
            ZipError::Unsupported(format!(
                "entry {}: local header extra range overflows",
                entry.name
            ))
        })?;
        if extra_end > self.data.len() {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local header name/extra is truncated",
                entry.name
            )));
        }
        Ok(LocalFileHeader {
            version_needed: rd_u16(&self.data, header_start + 4)
                .ok_or(ZipError::Truncated("lfh"))?,
            flags: rd_u16(&self.data, header_start + 6).ok_or(ZipError::Truncated("lfh"))?,
            method: rd_u16(&self.data, header_start + 8).ok_or(ZipError::Truncated("lfh"))?,
            mod_time: rd_u16(&self.data, header_start + 10).ok_or(ZipError::Truncated("lfh"))?,
            mod_date: rd_u16(&self.data, header_start + 12).ok_or(ZipError::Truncated("lfh"))?,
            crc32: rd_u32(&self.data, header_start + 14).ok_or(ZipError::Truncated("lfh"))?,
            comp_size32: rd_u32(&self.data, header_start + 18).ok_or(ZipError::Truncated("lfh"))?,
            uncomp_size32: rd_u32(&self.data, header_start + 22)
                .ok_or(ZipError::Truncated("lfh"))?,
            name: &self.data[header_end..name_end],
            extra: &self.data[name_end..extra_end],
            data_start: extra_end,
        })
    }

    fn validate_local_entry(&self, entry: &ZipEntry) -> Result<LocalEntryRegion, ZipError> {
        let local = self.read_local_header(entry)?;
        if local.name != entry.name.as_bytes() {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local/central name mismatch (local {:?})",
                entry.name,
                String::from_utf8_lossy(local.name)
            )));
        }
        if local.version_needed != entry.version_needed {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local/central version-needed mismatch (local {}, central {})",
                entry.name, local.version_needed, entry.version_needed
            )));
        }
        if local.flags != entry.flags {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local/central flags mismatch (local 0x{:04x}, central 0x{:04x})",
                entry.name, local.flags, entry.flags
            )));
        }
        if local.method != entry.method {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local/central method mismatch (local {}, central {})",
                entry.name, local.method, entry.method
            )));
        }
        if local.mod_time != entry.mod_time || local.mod_date != entry.mod_date {
            return Err(ZipError::Unsupported(format!(
                "entry {}: local/central timestamp mismatch",
                entry.name
            )));
        }
        if entry.method == ZIP_METHOD_STORE && entry.comp_size != entry.uncomp_size {
            return Err(ZipError::Unsupported(format!(
                "entry {}: STORE compressed size {} != uncompressed size {}",
                entry.name, entry.comp_size, entry.uncomp_size
            )));
        }

        if entry.flags & 0x0008 == 0 {
            let need_comp = local.comp_size32 == u32::MAX;
            let need_uncomp = local.uncomp_size32 == u32::MAX;
            let zip64 =
                parse_zip64_extra(local.extra, need_uncomp, need_comp, false).map_err(|error| {
                    ZipError::Unsupported(format!(
                        "entry {}: invalid local Zip64 sizes: {error}",
                        entry.name
                    ))
                })?;
            let local_comp_size = if need_comp {
                zip64.comp_size.ok_or_else(|| {
                    ZipError::Unsupported(format!(
                        "entry {}: local Zip64 compressed size missing",
                        entry.name
                    ))
                })?
            } else {
                local.comp_size32 as u64
            };
            let local_uncomp_size = if need_uncomp {
                zip64.uncomp_size.ok_or_else(|| {
                    ZipError::Unsupported(format!(
                        "entry {}: local Zip64 uncompressed size missing",
                        entry.name
                    ))
                })?
            } else {
                local.uncomp_size32 as u64
            };
            if local.crc32 != entry.crc32
                || local_comp_size != entry.comp_size
                || local_uncomp_size != entry.uncomp_size
            {
                return Err(ZipError::Unsupported(format!(
                    "entry {}: local/central crc or size mismatch",
                    entry.name
                )));
            }
        } else if local.crc32 != 0 || local.comp_size32 != 0 || local.uncomp_size32 != 0 {
            return Err(ZipError::Unsupported(format!(
                "entry {}: data-descriptor local crc/size fields must be zero",
                entry.name
            )));
        }

        let data_end = local
            .data_start
            .checked_add(usize::try_from(entry.comp_size).map_err(|_| {
                ZipError::Unsupported(format!(
                    "entry {}: compressed size {} is too large",
                    entry.name, entry.comp_size
                ))
            })?)
            .ok_or_else(|| {
                ZipError::Unsupported(format!("entry {}: data range overflows", entry.name))
            })?;
        if data_end > self.data.len() {
            return Err(ZipError::Unsupported(format!(
                "entry {}: data range ends past archive",
                entry.name
            )));
        }
        let region_end = if entry.flags & 0x0008 != 0 {
            self.data_descriptor_end(entry, data_end)?
        } else {
            data_end
        };
        let header_start = usize::try_from(entry.local_header_off).map_err(|_| {
            ZipError::Unsupported(format!(
                "entry {}: local header offset {} is too large",
                entry.name, entry.local_header_off
            ))
        })?;
        if header_start >= self.central_directory_off || region_end > self.central_directory_off {
            return Err(ZipError::Unsupported(format!(
                "entry {} region {}..{} overlaps central directory at {}",
                entry.name, header_start, region_end, self.central_directory_off
            )));
        }
        Ok(LocalEntryRegion {
            header_start,
            data_start: local.data_start,
            data_end,
            region_end,
        })
    }

    fn validate_payload_crc(
        &self,
        entry: &ZipEntry,
        region: LocalEntryRegion,
    ) -> Result<(), ZipError> {
        let comp = &self.data[region.data_start..region.data_end];
        let (crc32, uncompressed_len) = match entry.method {
            ZIP_METHOD_STORE => (crc32fast::hash(comp), comp.len() as u64),
            ZIP_METHOD_DEFLATE => crc32_and_len(
                DeflateDecoder::new(comp),
                entry.uncomp_size.saturating_add(1),
            )?,
            ZIP_METHOD_ZSTD => {
                let mut dec = zstd::stream::read::Decoder::new(comp)
                    .map_err(|err| ZipError::Decompress(err.to_string()))?;
                dec.window_log_max(ZSTD_WINDOW_LOG)
                    .map_err(|err| ZipError::Decompress(err.to_string()))?;
                crc32_and_len(dec, entry.uncomp_size.saturating_add(1))?
            }
            method => {
                return Err(ZipError::Unsupported(format!(
                    "compression method {method}"
                )));
            }
        };
        if uncompressed_len != entry.uncomp_size {
            return Err(ZipError::SizeMismatch {
                name: entry.name.clone(),
                declared: entry.uncomp_size,
                got: uncompressed_len,
            });
        }
        if crc32 != entry.crc32 {
            return Err(ZipError::Unsupported(format!(
                "entry {}: payload crc32 mismatch (derived {crc32:08x}, declared {:08x})",
                entry.name, entry.crc32
            )));
        }
        Ok(())
    }

    fn data_descriptor_end(&self, entry: &ZipEntry, start: usize) -> Result<usize, ZipError> {
        let signed = data_descriptor_candidate(&self.data, entry, start, true);
        let unsigned = data_descriptor_candidate(&self.data, entry, start, false);
        match (signed, unsigned) {
            (Some(end), None) | (None, Some(end)) => Ok(end),
            (Some(_), Some(_)) => Err(ZipError::Unsupported(format!(
                "entry {}: ambiguous signed/unsigned data descriptor",
                entry.name
            ))),
            (None, None) => Err(ZipError::Unsupported(format!(
                "entry {}: data descriptor missing, truncated, or inconsistent with central directory",
                entry.name
            ))),
        }
    }

    /// Read + decompress one entry's bytes. Enforces the declared uncompressed
    /// size exactly and keeps the normal container reader capped at 2 GiB.
    pub fn read_entry(&self, e: &ZipEntry) -> Result<Vec<u8>, ZipError> {
        if e.uncomp_size > MAX_CONTAINER_ENTRY_BYTES {
            return Err(ZipError::Unsupported(format!(
                "entry {} exceeds in-RAM reader cap ({} > {})",
                e.name, e.uncomp_size, MAX_CONTAINER_ENTRY_BYTES
            )));
        }
        let local = self.validate_local_entry(e)?;
        let comp = &self.data[local.data_start..local.data_end];

        let out = match e.method {
            ZIP_METHOD_STORE => comp.to_vec(),
            ZIP_METHOD_DEFLATE => {
                let mut out = Vec::with_capacity(e.uncomp_size.min(1 << 20) as usize);
                let cap = e.uncomp_size.saturating_add(1);
                let mut dec = DeflateDecoder::new(comp).take(cap);
                dec.read_to_end(&mut out)
                    .map_err(|err| ZipError::Decompress(err.to_string()))?;
                out
            }
            ZIP_METHOD_ZSTD => {
                let mut out = Vec::with_capacity(e.uncomp_size.min(1 << 20) as usize);
                let cap = e.uncomp_size.saturating_add(1);
                let mut dec = zstd::stream::read::Decoder::new(comp)
                    .map_err(|err| ZipError::Decompress(err.to_string()))?;
                dec.window_log_max(ZSTD_WINDOW_LOG)
                    .map_err(|err| ZipError::Decompress(err.to_string()))?;
                dec.take(cap)
                    .read_to_end(&mut out)
                    .map_err(|err| ZipError::Decompress(err.to_string()))?;
                out
            }
            m => return Err(ZipError::Unsupported(format!("compression method {m}"))),
        };

        if out.len() as u64 != e.uncomp_size {
            return Err(ZipError::SizeMismatch {
                name: e.name.clone(),
                declared: e.uncomp_size,
                got: out.len() as u64,
            });
        }
        Ok(out)
    }
}

fn crc32_and_len<R: Read>(reader: R, limit: u64) -> Result<(u32, u64), ZipError> {
    let mut reader = reader.take(limit);
    let mut crc32 = crc32fast::Hasher::new();
    let mut len = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|err| ZipError::Decompress(err.to_string()))?;
        if read == 0 {
            break;
        }
        crc32.update(&buf[..read]);
        len += read as u64;
    }
    Ok((crc32.finalize(), len))
}

fn data_descriptor_candidate(
    data: &[u8],
    entry: &ZipEntry,
    start: usize,
    signed: bool,
) -> Option<usize> {
    let mut p = start;
    if signed {
        if rd_u32(data, p)? != DATA_DESCRIPTOR_SIG {
            return None;
        }
        p = p.checked_add(4)?;
    }
    let crc32 = rd_u32(data, p)?;
    p = p.checked_add(4)?;
    let zip64 =
        entry.comp_size > ZIP32_MAX_FIELD_VALUE || entry.uncomp_size > ZIP32_MAX_FIELD_VALUE;
    let (comp_size, uncomp_size) = if zip64 {
        let comp_size = rd_u64(data, p)?;
        p = p.checked_add(8)?;
        let uncomp_size = rd_u64(data, p)?;
        p = p.checked_add(8)?;
        (comp_size, uncomp_size)
    } else {
        let comp_size = rd_u32(data, p)? as u64;
        p = p.checked_add(4)?;
        let uncomp_size = rd_u32(data, p)? as u64;
        p = p.checked_add(4)?;
        (comp_size, uncomp_size)
    };
    (crc32 == entry.crc32 && comp_size == entry.comp_size && uncomp_size == entry.uncomp_size)
        .then_some(p)
}

#[derive(Default)]
struct Zip64Extra {
    uncomp_size: Option<u64>,
    comp_size: Option<u64>,
    local_header_off: Option<u64>,
}

fn validate_ordinary_zip64_field(
    name: &str,
    ordinary: u64,
    sentinel: u64,
    zip64: u64,
) -> Result<(), ZipError> {
    if ordinary != sentinel && ordinary != zip64 {
        return Err(ZipError::Unsupported(format!(
            "ordinary EOCD {name} {ordinary} disagrees with Zip64 value {zip64}"
        )));
    }
    Ok(())
}

fn find_zip64_locator(data: &[u8], eocd: usize) -> Option<usize> {
    if eocd < 20 {
        return None;
    }
    let off = eocd - 20;
    (rd_u32(data, off) == Some(ZIP64_LOCATOR_SIG)).then_some(off)
}

fn parse_zip64_eocd(data: &[u8], locator: usize) -> Result<(u64, u64, u64, u64), ZipError> {
    if rd_u32(data, locator + 4).ok_or(ZipError::Truncated("zip64 locator"))? != 0 {
        return Err(ZipError::Unsupported(
            "multi-disk Zip64 archives are not supported".into(),
        ));
    }
    let zip64_eocd_off = rd_u64(data, locator + 8).ok_or(ZipError::Truncated("zip64 locator"))?;
    let total_disks = rd_u32(data, locator + 16).ok_or(ZipError::Truncated("zip64 locator"))?;
    if total_disks != 1 {
        return Err(ZipError::Unsupported(
            "multi-disk Zip64 archives are not supported".into(),
        ));
    }
    let off = usize::try_from(zip64_eocd_off)
        .map_err(|_| ZipError::Unsupported("Zip64 EOCD offset too large".into()))?;
    if off.checked_add(56).map_or(true, |end| end > data.len()) {
        return Err(ZipError::Truncated("zip64 eocd"));
    }
    if rd_u32(data, off) != Some(ZIP64_EOCD_SIG) {
        return Err(ZipError::BadSignature("zip64 eocd"));
    }
    let record_size = rd_u64(data, off + 4).ok_or(ZipError::Truncated("zip64 eocd"))?;
    if record_size < 44 {
        return Err(ZipError::Truncated("zip64 eocd size"));
    }
    let disk_no = rd_u32(data, off + 16).ok_or(ZipError::Truncated("zip64 eocd"))?;
    let cd_disk = rd_u32(data, off + 20).ok_or(ZipError::Truncated("zip64 eocd"))?;
    if disk_no != 0 || cd_disk != 0 {
        return Err(ZipError::Unsupported(
            "multi-disk Zip64 archives are not supported".into(),
        ));
    }
    let entries_on_disk = rd_u64(data, off + 24).ok_or(ZipError::Truncated("zip64 eocd"))?;
    let total_entries = rd_u64(data, off + 32).ok_or(ZipError::Truncated("zip64 eocd"))?;
    if entries_on_disk != total_entries {
        return Err(ZipError::Unsupported(
            "split Zip64 central directories are not supported".into(),
        ));
    }
    let cd_size = rd_u64(data, off + 40).ok_or(ZipError::Truncated("zip64 eocd"))?;
    let cd_off = rd_u64(data, off + 48).ok_or(ZipError::Truncated("zip64 eocd"))?;
    Ok((total_entries, cd_size, cd_off, zip64_eocd_off))
}

fn parse_zip64_extra(
    extra: &[u8],
    need_uncomp: bool,
    need_comp: bool,
    need_lho: bool,
) -> Result<Zip64Extra, ZipError> {
    let mut p = 0usize;
    let mut zip64 = None;
    while p < extra.len() {
        if extra.len() - p < 4 {
            return Err(ZipError::Truncated("extra field header"));
        }
        let id = rd_u16(extra, p).ok_or(ZipError::Truncated("extra"))?;
        let len = rd_u16(extra, p + 2).ok_or(ZipError::Truncated("extra"))? as usize;
        let start = p + 4;
        let end = start
            .checked_add(len)
            .ok_or(ZipError::Truncated("extra overflow"))?;
        if end > extra.len() {
            return Err(ZipError::Truncated("extra past end"));
        }
        if id == ZIP64_EXTRA_ID {
            if zip64.is_some() {
                return Err(ZipError::Unsupported("duplicate Zip64 extra field".into()));
            }
            let field = &extra[start..end];
            let mut q = 0usize;
            let mut z = Zip64Extra::default();
            if need_uncomp {
                z.uncomp_size = Some(rd_u64(field, q).ok_or(ZipError::Truncated("zip64 extra"))?);
                q += 8;
            }
            if need_comp {
                z.comp_size = Some(rd_u64(field, q).ok_or(ZipError::Truncated("zip64 extra"))?);
                q += 8;
            }
            if need_lho {
                z.local_header_off =
                    Some(rd_u64(field, q).ok_or(ZipError::Truncated("zip64 extra"))?);
            }
            zip64 = Some(z);
        }
        p = end;
    }
    if zip64.is_none() && (need_uncomp || need_comp || need_lho) {
        return Err(ZipError::Unsupported("Zip64 extra field missing".into()));
    }
    Ok(zip64.unwrap_or_default())
}

/// Scan backwards for the End-Of-Central-Directory signature (handles a ZIP
/// comment up to 64 KiB). Returns the EOCD record offset.
fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let max_back = 22 + 0xFFFF;
    let start = data.len().saturating_sub(max_back);
    let mut i = data.len() - 22;
    loop {
        if rd_u32(data, i) == Some(EOCD_SIG) && eocd_comment_ends_at_eof(data, i) {
            return Some(i);
        }
        if i == start {
            return None;
        }
        i -= 1;
    }
}

fn eocd_comment_ends_at_eof(data: &[u8], off: usize) -> bool {
    let comment_len = match rd_u16(data, off + 20) {
        Some(len) => len as usize,
        None => return false,
    };
    off.checked_add(22)
        .and_then(|end| end.checked_add(comment_len))
        == Some(data.len())
}

/// Path rules. Reject anything that could escape a payload root or collide on a
/// real filesystem.
pub fn validate_path(name: &str) -> Result<(), ZipError> {
    let bad = |n: &str| Err(ZipError::BadPath(n.to_string()));
    if name.is_empty() {
        return bad(name);
    }
    if name.contains('\\') {
        return bad(name); // ambiguous separator
    }
    if name.starts_with('/') {
        return bad(name); // absolute
    }
    if name.contains("//") {
        return bad(name); // UNC-ish / empty component
    }
    // drive letter like "C:" anywhere
    if name.contains(':') {
        return bad(name);
    }
    for comp in name.split('/') {
        if comp == "." || comp == ".." {
            return bad(name); // traversal
        }
        // Windows reserved device names (case-insensitive, with or without ext)
        let stem = comp.split('.').next().unwrap_or(comp).to_ascii_uppercase();
        const RESERVED: &[&str] = &[
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if RESERVED.contains(&stem.as_str()) {
            return bad(name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip64_empty_with_entry_count(total_entries: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let zip64_eocd_off = out.len() as u64;
        out.extend_from_slice(&ZIP64_EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&44u64.to_le_bytes()); // size after this field
        out.extend_from_slice(&45u16.to_le_bytes()); // version made by
        out.extend_from_slice(&45u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u32.to_le_bytes()); // disk
        out.extend_from_slice(&0u32.to_le_bytes()); // central dir disk
        out.extend_from_slice(&total_entries.to_le_bytes());
        out.extend_from_slice(&total_entries.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // central dir size
        out.extend_from_slice(&0u64.to_le_bytes()); // central dir offset

        out.extend_from_slice(&ZIP64_LOCATOR_SIG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&zip64_eocd_off.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());

        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn add_comment_with_fake_eocd(mut data: Vec<u8>) -> Vec<u8> {
        let eocd = find_eocd(&data).expect("base archive eocd");
        assert_eq!(rd_u16(&data, eocd + 20), Some(0));
        let mut comment = vec![b'x'; 40];
        comment[8..12].copy_from_slice(&EOCD_SIG.to_le_bytes());
        comment[28..30].copy_from_slice(&0u16.to_le_bytes());
        data[eocd + 20..eocd + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        data.extend_from_slice(&comment);
        data
    }

    #[test]
    fn rejects_traversal_and_absolute() {
        assert!(validate_path("../etc/passwd").is_err());
        assert!(validate_path("/abs").is_err());
        assert!(validate_path("a\\b").is_err());
        assert!(validate_path("C:/x").is_err());
        assert!(validate_path("payload//x").is_err());
        assert!(validate_path("payload/CON").is_err());
        assert!(validate_path("payload/nul.json").is_err());
        assert!(validate_path("payload/g/index.json").is_ok());
        assert!(validate_path("manifest.json").is_ok());
    }

    #[test]
    fn hostile_bytes_error_never_panic() {
        // empty, too-short, garbage, and fake EOCD variants must all return Err
        // cleanly.
        for input in [
            vec![],
            b"PK".to_vec(),
            vec![0u8; 21],
            b"PK\x05\x06 not enough".to_vec(),
            {
                // a valid-looking EOCD that claims a CD far out of range
                let mut v = vec![0u8; 22];
                v[0..4].copy_from_slice(&EOCD_SIG.to_le_bytes());
                v[16..20].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
                v[12..16].copy_from_slice(&0x10u32.to_le_bytes());
                v
            },
            {
                // Zip64 sentinel without the required locator.
                let mut v = vec![0u8; 22];
                v[0..4].copy_from_slice(&EOCD_SIG.to_le_bytes());
                v[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
                v
            },
        ] {
            let _ = ZipArchive::parse(input);
        }
    }

    #[test]
    fn rejects_zip64_entry_count_above_reader_cap_before_cd_parse() {
        let err = match ZipArchive::parse(zip64_empty_with_entry_count(MAX_READER_ENTRIES + 1)) {
            Ok(_) => panic!("entry-count cap should reject hostile Zip64 EOCD"),
            Err(err) => err,
        };
        assert!(
            matches!(&err, ZipError::Unsupported(msg) if msg.contains("entry count") && msg.contains("reader cap")),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn eocd_collision_in_comment_is_ignored_unless_it_reaches_eof() {
        let entries = vec![crate::pack::BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
            role: "index",
            hour_index: None,
        }];
        let base =
            crate::pack::pack_borrowed("g", &entries, &crate::pack::PackIdentity::default(), false)
                .expect("pack base archive");
        let eocd = find_eocd(&base).expect("base eocd");
        let with_collision = add_comment_with_fake_eocd(base);
        assert_eq!(find_eocd(&with_collision), Some(eocd));

        let archive = ZipArchive::parse(with_collision).expect("valid archive opens");
        assert_eq!(archive.entries().len(), 2);
    }
}
