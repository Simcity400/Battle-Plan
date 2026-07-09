//! A minimal, hardened, READ-ONLY ZIP reader for the `.battleplan` container.
//!
//! Scope on purpose: just enough to enumerate the central directory and read one
//! entry's decompressed bytes (STORE or DEFLATE). It does NOT extract to disk, so
//! zip-slip/symlink/device-name *extraction* attacks do not apply to this path —
//! but it still validates every path (§4.2 of the spec) so a caller that does
//! materialize files is handed only safe names.
//!
//! Hardening (Codex b): rejects duplicate entry names, validates paths, enforces
//! the declared uncompressed size per entry (zip-bomb guard), reads only the
//! central-directory entry it serves (never trusts a local-header name).

use flate2::read::DeflateDecoder;
use std::collections::HashSet;
use std::io::Read;

const EOCD_SIG: u32 = 0x0605_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;

/// ZIP32 scalar-field maximum. The writer rejects larger values until the
/// planned Zip64 container upgrade lands.
pub const ZIP32_MAX_FIELD_VALUE: u64 = u32::MAX as u64;
/// ZIP32 EOCD entry-count maximum. 0xFFFF itself is excluded: readers (here and
/// stream_verify) treat an EOCD total of 0xFFFF as the Zip64 sentinel and reject
/// the archive, so the writer must stop one below it.
pub const ZIP32_MAX_ENTRY_COUNT: usize = (u16::MAX as usize) - 1;
/// Shared per-entry cap enforced by both the standard reader and streaming writer.
pub const MAX_CONTAINER_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A single central-directory record we care about.
#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub method: u16,
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

/// A ZIP archive held entirely in memory (the container files are modest; the
/// per-snapshot caller still reads one entry at a time).
pub struct ZipArchive {
    data: Vec<u8>,
    entries: Vec<ZipEntry>,
}

impl ZipArchive {
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Parse the central directory of an in-memory ZIP, validating + de-duping
    /// every name. Rejects Zip64 (the container is small; if we ever need it,
    /// we add it explicitly rather than mis-handling the 0xFFFF.. sentinels).
    pub fn parse(data: Vec<u8>) -> Result<ZipArchive, ZipError> {
        let eocd = find_eocd(&data).ok_or(ZipError::Truncated("no EOCD record"))?;
        let total = rd_u16(&data, eocd + 10).ok_or(ZipError::Truncated("eocd"))? as usize;
        let cd_size = rd_u32(&data, eocd + 12).ok_or(ZipError::Truncated("eocd"))? as usize;
        let cd_off = rd_u32(&data, eocd + 16).ok_or(ZipError::Truncated("eocd"))? as usize;
        if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF || total == 0xFFFF {
            return Err(ZipError::Unsupported("Zip64 not supported".into()));
        }
        if cd_off.saturating_add(cd_size) > data.len() {
            return Err(ZipError::Truncated("central directory out of range"));
        }

        let mut entries = Vec::with_capacity(total);
        let mut seen: HashSet<String> = HashSet::new();
        let mut seen_ci: HashSet<String> = HashSet::new();
        let mut p = cd_off;
        let cd_end = cd_off + cd_size;
        while p + 46 <= cd_end {
            let sig = rd_u32(&data, p).ok_or(ZipError::Truncated("cdh"))?;
            if sig != CDH_SIG {
                break;
            }
            let method = rd_u16(&data, p + 10).ok_or(ZipError::Truncated("cdh"))?;
            let comp_size = rd_u32(&data, p + 20).ok_or(ZipError::Truncated("cdh"))? as u64;
            let uncomp_size = rd_u32(&data, p + 24).ok_or(ZipError::Truncated("cdh"))? as u64;
            let name_len = rd_u16(&data, p + 28).ok_or(ZipError::Truncated("cdh"))? as usize;
            let extra_len = rd_u16(&data, p + 30).ok_or(ZipError::Truncated("cdh"))? as usize;
            let comment_len = rd_u16(&data, p + 32).ok_or(ZipError::Truncated("cdh"))? as usize;
            let lho = rd_u32(&data, p + 42).ok_or(ZipError::Truncated("cdh"))? as u64;

            let name_start = p + 46;
            let name_end = name_start + name_len;
            if name_end > cd_end {
                return Err(ZipError::Truncated("cdh name"));
            }
            let raw_name = &data[name_start..name_end];
            let name = std::str::from_utf8(raw_name)
                .map_err(|_| ZipError::BadPath("<non-utf8>".into()))?
                .to_string();

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
                method,
                comp_size,
                uncomp_size,
                local_header_off: lho,
            });
            // Advance with checked adds and refuse a CDH whose extra/comment would
            // run past the central directory (Codex hardening: a hostile
            // extra/comment length must error, not silently over-run).
            let next = name_end
                .checked_add(extra_len)
                .and_then(|v| v.checked_add(comment_len))
                .ok_or(ZipError::Truncated("cdh advance overflow"))?;
            if next > cd_end {
                return Err(ZipError::Truncated("cdh extra/comment past central dir"));
            }
            p = next;
        }
        Ok(ZipArchive { data, entries })
    }

    pub fn by_name(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Read + decompress one entry's bytes. Enforces the declared uncompressed
    /// size exactly (a zip-bomb that lies about its size is caught here, and a
    /// truncated/over-long stream is rejected). Reads the data at the LOCAL header
    /// offset but never trusts the local header's name.
    pub fn read_entry(&self, e: &ZipEntry) -> Result<Vec<u8>, ZipError> {
        let lh = e.local_header_off as usize;
        // checked header-end (Codex hardening: on 32-bit a 0xffff_ffff offset must
        // error here, not panic on `lh + 30`).
        let header_end = lh
            .checked_add(30)
            .ok_or(ZipError::Truncated("local header range overflow"))?;
        if header_end > self.data.len() {
            return Err(ZipError::Truncated("local header"));
        }
        let sig = rd_u32(&self.data, lh).ok_or(ZipError::Truncated("lfh"))?;
        if sig != LFH_SIG {
            return Err(ZipError::BadSignature("local file header"));
        }
        let lh_method = rd_u16(&self.data, lh + 8).ok_or(ZipError::Truncated("lfh"))?;
        let lh_name_len = rd_u16(&self.data, lh + 26).ok_or(ZipError::Truncated("lfh"))? as usize;
        let lh_extra_len = rd_u16(&self.data, lh + 28).ok_or(ZipError::Truncated("lfh"))? as usize;
        // central-directory method is authoritative; a mismatch is suspicious.
        if lh_method != e.method {
            return Err(ZipError::Unsupported(format!(
                "local/central method mismatch for {}",
                e.name
            )));
        }
        // fully-checked data-start arithmetic (Codex hardening).
        let data_start = header_end
            .checked_add(lh_name_len)
            .and_then(|v| v.checked_add(lh_extra_len))
            .ok_or(ZipError::Truncated("entry data start overflow"))?;
        let data_end = data_start
            .checked_add(e.comp_size as usize)
            .ok_or(ZipError::Truncated("entry data range overflow"))?;
        if data_end > self.data.len() {
            return Err(ZipError::Truncated("entry data out of range"));
        }
        let comp = &self.data[data_start..data_end];

        let out = match e.method {
            0 => comp.to_vec(), // STORE
            8 => {
                // DEFLATE — decode with a hard cap at the declared size + 1 so a
                // bomb that inflates beyond its declaration is stopped early.
                let mut out = Vec::with_capacity(e.uncomp_size.min(1 << 20) as usize);
                let cap = e.uncomp_size.saturating_add(1);
                let mut dec = DeflateDecoder::new(comp).take(cap);
                dec.read_to_end(&mut out)
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

/// Scan backwards for the End-Of-Central-Directory signature (handles a ZIP
/// comment up to 64 KiB). Returns the EOCD record offset.
fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let max_back = 22 + 0xFFFF;
    let start = data.len().saturating_sub(max_back);
    // search the last window for the signature, scanning from the end.
    let mut i = data.len() - 22;
    loop {
        if rd_u32(data, i) == Some(EOCD_SIG) {
            return Some(i);
        }
        if i == start {
            return None;
        }
        i -= 1;
    }
}

/// §4.2 path rules. Reject anything that could escape a payload root or collide
/// on a real filesystem.
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
        // empty, too-short, garbage, and a fake EOCD with absurd offsets must all
        // return Err cleanly (Codex panic audit).
        for input in [
            vec![],
            b"PK".to_vec(),
            vec![0u8; 21],
            b"PK\x05\x06 not enough".to_vec(),
            {
                // a valid-looking EOCD that claims a CD far out of range
                let mut v = vec![0u8; 22];
                v[0..4].copy_from_slice(&EOCD_SIG.to_le_bytes());
                v[16..20].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes()); // cd_off
                v[12..16].copy_from_slice(&0x10u32.to_le_bytes()); // cd_size
                v
            },
        ] {
            // must not panic; result is irrelevant, only that it returns.
            let _ = ZipArchive::parse(input);
        }
    }
}
