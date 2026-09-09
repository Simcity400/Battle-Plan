use sha2::{Digest, Sha256};
use std::io::{self, Read};

/// Module-static version banner candidate; old session_header DOMAIN68 reads ASCII at RVA0x2701108, hash remains the build authority.
pub const VERSION_BANNER_RVA: usize = 0x0270_1108;

#[derive(Debug, Clone, Copy)]
pub struct Build {
    pub version: &'static str,
    pub exe_size: u64,
    pub exe_sha256: [u8; 32],
}

/// Installed hoi4.exe and the Ghidra project agree by SHA-256, checked 2026-09-09.
pub const PINNED_BUILD: Build = Build {
    version: "Operation Postern v1.19.2.0.a729",
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
            version: "test",
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
}
