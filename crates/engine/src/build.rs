use sha2::{Digest, Sha256};
use std::io::{self, BufRead, Read, Seek, SeekFrom};

/// Image-relative location of the executable's NUL-terminated version text.
pub const VERSION_BANNER_RVA: usize = 0x0270_1108;

#[derive(Debug, Clone, Copy)]
pub struct Build {
    pub version_banner_rva: usize,
    pub exe_size: u64,
    pub exe_sha256: [u8; 32],
}

/// Supported executable identity and version-text location.
pub const PINNED_BUILD: Build = Build {
    version_banner_rva: VERSION_BANNER_RVA,
    exe_size: 55_680_632,
    exe_sha256: [
        0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75,
        0xc4, 0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c,
        0x65, 0xad,
    ],
};

#[derive(Debug, PartialEq, Eq)]
pub struct ExecutableIdentity {
    pub size: u64,
    pub sha256: [u8; 32],
}

impl Build {
    pub fn matches(&self, identity: &ExecutableIdentity) -> bool {
        identity.size == self.exe_size && identity.sha256 == self.exe_sha256
    }

    /// Load owned version text from the supported executable, outside acquisition.
    /// The same source is hash-verified before its image-relative location is mapped
    /// to file bytes. Section geometry bounds the string; missing text is refused.
    pub fn read_version_banner(&self, mut source: impl Read + Seek) -> io::Result<String> {
        source.seek(SeekFrom::Start(0))?;
        if !self.matches(&hash_executable(&mut source)?) {
            return Err(invalid_image(
                "version text requires the supported executable",
            ));
        }
        let (offset, available) =
            file_span_for_rva(&mut source, self.version_banner_rva as u64, self.exe_size)?;
        source.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        io::BufReader::new(source.take(available)).read_until(0, &mut bytes)?;
        if bytes.pop() != Some(0) || bytes.is_empty() {
            return Err(invalid_image(
                "executable version text is empty or unterminated",
            ));
        }
        String::from_utf8(bytes).map_err(|_| invalid_image("executable version text is not UTF-8"))
    }
}

fn invalid_image(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn file_span_for_rva(
    source: &mut (impl Read + Seek),
    rva: u64,
    file_size: u64,
) -> io::Result<(u64, u64)> {
    source.seek(SeekFrom::Start(0))?;
    let mut dos = [0_u8; 64];
    source.read_exact(&mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err(invalid_image("executable DOS header is invalid"));
    }
    let pe_offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    source.seek(SeekFrom::Start(pe_offset))?;
    let mut header = [0_u8; 24];
    source.read_exact(&mut header)?;
    if &header[..4] != b"PE\0\0" {
        return Err(invalid_image("executable PE header is invalid"));
    }
    let sections = u16::from_le_bytes(header[6..8].try_into().unwrap());
    let optional_size = u16::from_le_bytes(header[20..22].try_into().unwrap());
    if optional_size < 64 {
        return Err(invalid_image("executable optional header is truncated"));
    }
    let mut optional = [0_u8; 64];
    source.read_exact(&mut optional)?;
    if u16::from_le_bytes(optional[..2].try_into().unwrap()) != 0x20b {
        return Err(invalid_image("executable optional header is unsupported"));
    }
    let header_size = u32::from_le_bytes(optional[60..64].try_into().unwrap()) as u64;
    if rva < header_size {
        if header_size > file_size {
            return Err(invalid_image("executable headers exceed the file"));
        }
        return Ok((rva, header_size - rva));
    }
    let section_offset = pe_offset + 24 + u64::from(optional_size);
    source.seek(SeekFrom::Start(section_offset))?;
    for _ in 0..sections {
        let mut section = [0_u8; 40];
        source.read_exact(&mut section)?;
        let start = u32::from_le_bytes(section[12..16].try_into().unwrap()) as u64;
        let raw_size = u32::from_le_bytes(section[16..20].try_into().unwrap()) as u64;
        let raw_offset = u32::from_le_bytes(section[20..24].try_into().unwrap()) as u64;
        let Some(displacement) = rva.checked_sub(start) else {
            continue;
        };
        if displacement >= raw_size {
            continue;
        }
        if raw_offset + raw_size > file_size {
            return Err(invalid_image("executable section exceeds the file"));
        }
        return Ok((raw_offset + displacement, raw_size - displacement));
    }
    Err(invalid_image(
        "executable version text has no file-backed section",
    ))
}

pub fn hash_executable(mut source: impl Read) -> io::Result<ExecutableIdentity> {
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = match source.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(ExecutableIdentity {
                size,
                sha256: hash.finalize().into(),
            });
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("executable too large"))?;
        hash.update(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_change_and_length_change_both_refuse_the_build() {
        let original = hash_executable(&b"engine"[..]).unwrap();
        let build = Build {
            version_banner_rva: VERSION_BANNER_RVA,
            exe_size: original.size,
            exe_sha256: original.sha256,
        };
        assert!(build.matches(&original));
        assert!(!build.matches(&hash_executable(&b"Engine"[..]).unwrap()));
        assert!(!build.matches(&ExecutableIdentity {
            size: original.size + 1,
            ..original
        }));
    }

    fn version_image(text: &[u8]) -> (Build, Vec<u8>) {
        let mut image = vec![0_u8; 0x280];
        image[..2].copy_from_slice(b"MZ");
        image[60..64].copy_from_slice(&0x80_u32.to_le_bytes());
        image[0x80..0x84].copy_from_slice(b"PE\0\0");
        image[0x86..0x88].copy_from_slice(&1_u16.to_le_bytes());
        image[0x94..0x96].copy_from_slice(&0x70_u16.to_le_bytes());
        image[0x98..0x9a].copy_from_slice(&0x20b_u16.to_le_bytes());
        image[0xd4..0xd8].copy_from_slice(&0x200_u32.to_le_bytes());
        image[0x114..0x118].copy_from_slice(&0x2000_u32.to_le_bytes());
        image[0x118..0x11c].copy_from_slice(&0x80_u32.to_le_bytes());
        image[0x11c..0x120].copy_from_slice(&0x200_u32.to_le_bytes());
        image[0x208..0x208 + text.len()].copy_from_slice(text);
        let identity = hash_executable(image.as_slice()).unwrap();
        (
            Build {
                version_banner_rva: 0x2008,
                exe_size: identity.size,
                exe_sha256: identity.sha256,
            },
            image,
        )
    }

    #[test]
    fn installed_version_text_uses_section_file_mapping() {
        let (build, image) = version_image(b"Synthetic build label\0");
        assert_eq!(
            build.read_version_banner(io::Cursor::new(image)).unwrap(),
            "Synthetic build label"
        );
    }

    #[test]
    fn installed_version_text_requires_matching_file_identity() {
        let (build, mut image) = version_image(b"Synthetic build label\0");
        image[0x208] ^= 1;
        assert_eq!(
            build
                .read_version_banner(io::Cursor::new(image))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn installed_version_text_cannot_cross_its_file_backed_section() {
        let (build, image) = version_image(&[b'x'; 0x78]);
        assert_eq!(
            build
                .read_version_banner(io::Cursor::new(image))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
