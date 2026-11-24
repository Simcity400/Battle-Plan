use crate::availability::{Verified, Withheld};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, io::{self, Read}};

pub const FIXED_MODIFIER_GAP: u32 = 0x29a;

#[derive(Debug, Clone, Copy)]
pub struct Build {
    pub version: &'static str,
    pub exe_size: u64,
    pub exe_sha256: [u8; 32],
}

pub const PINNED_BUILD: Build = Build {
    version: "1.19.5.0",
    exe_size: 56_535_160,
    exe_sha256: [
        0x70, 0x9f, 0x91, 0x6a, 0xfb, 0xeb, 0xcf, 0xe6, 0x4a, 0x26, 0x67, 0x87, 0x02, 0x69, 0x1a,
        0xeb, 0x40, 0x94, 0x5d, 0x20, 0x60, 0x74, 0xc2, 0xa8, 0x0d, 0xe8, 0xe9, 0xd6, 0x01, 0xfc,
        0xe1, 0x48,
    ],
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableIdentity {
    pub size: u64,
    pub sha256: [u8; 32],
}

impl Build {
    pub fn matches(&self, identity: &ExecutableIdentity) -> bool {
        identity.size == self.exe_size && identity.sha256 == self.exe_sha256
    }
}

impl ExecutableIdentity {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
        }
    }

    pub fn recognised(&self) -> bool {
        PINNED_BUILD.matches(self)
    }

    pub fn warning(&self, version: Option<&str>) -> Option<String> {
        if self.recognised() {
            return None;
        }
        let found = match version {
            Some(version) => format!("{version} ({})", describe(self.size, &self.sha256)),
            None => describe(self.size, &self.sha256),
        };
        Some(format!(
            "Unrecognised game build: this release was made for version {} ({}), but the game is {found}. \
             Parts that fail their own checks on this build are withheld.",
            PINNED_BUILD.version,
            describe(PINNED_BUILD.exe_size, &PINNED_BUILD.exe_sha256),
        ))
    }
}

pub struct RecordedBuild<'a> {
    pub version: &'a str,
    pub identity: ExecutableIdentity,
}

impl RecordedBuild<'_> {
    pub fn warning(&self, installed: (&ExecutableIdentity, Option<&str>)) -> Option<String> {
        let (identity, version) = installed;
        if *identity == self.identity {
            return None;
        }
        let installed = match version {
            Some(version) => format!("{version} ({})", describe(identity.size, &identity.sha256)),
            None => describe(identity.size, &identity.sha256),
        };
        Some(format!(
            "This recording was made with game {} ({}), but the installed game is {installed}.",
            self.version,
            describe(self.identity.size, &self.identity.sha256),
        ))
    }
}

fn describe(size: u64, sha256: &[u8; 32]) -> String {
    let mut text = format!("hoi4.exe of {size} bytes, SHA-256 ");
    for byte in &sha256[..6] {
        let _ = write!(text, "{byte:02x}");
    }
    text.push('…');
    text
}

pub fn version_banner(read_only_data: &[&[u8]]) -> Verified<String> {
    let mut found = None;
    for data in read_only_data {
        for at in memchr::memmem::find_iter(data, b" v") {
            let start = data[..at].iter().rposition(|&byte| byte == 0).map_or(0, |nul| nul + 1);
            let Some(length) = data[start..].iter().position(|&byte| byte == 0) else {
                continue;
            };
            let text = &data[start..start + length];
            if is_banner(text, at - start) {
                if found.replace(text).is_some_and(|previous| previous != text) {
                    return Err(Withheld::new(
                        "game version text",
                        "the executable holds more than one version-shaped string",
                    ));
                }
            }
        }
    }
    let text = found.ok_or_else(|| {
        Withheld::new(
            "game version text",
            "the executable holds no version-shaped string",
        )
    })?;
    Ok(String::from_utf8(text.to_vec()).expect("banner shape is ASCII"))
}

fn is_banner(text: &[u8], separator: usize) -> bool {
    let (title, version) = (&text[..separator], &text[separator + 2..]);
    if title.len() > 64
        || !title.first().is_some_and(u8::is_ascii_uppercase)
        || !title.iter().all(|&byte| byte.is_ascii_alphabetic() || byte == b' ')
    {
        return false;
    }
    let parts: Vec<_> = version.split(|&byte| byte == b'.').collect();
    let Some((build, numbers)) = parts.split_last() else {
        return false;
    };
    numbers.len() >= 3
        && numbers
            .iter()
            .all(|part| !part.is_empty() && part.len() <= 4 && part.iter().all(u8::is_ascii_digit))
        && (1..=8).contains(&build.len())
        && build.iter().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
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
    fn byte_change_and_length_change_both_change_the_identity() {
        let original = hash_executable(&b"engine"[..]).unwrap();
        let build = Build {
            version: "test",
            exe_size: original.size,
            exe_sha256: original.sha256,
        };
        assert!(build.matches(&original));
        assert_eq!(original, ExecutableIdentity::of(b"engine"));
        assert!(!build.matches(&hash_executable(&b"Engine"[..]).unwrap()));
        assert!(!build.matches(&ExecutableIdentity {
            size: original.size + 1,
            ..original
        }));
    }

    #[test]
    fn version_text_is_the_one_banner_shaped_string() {
        let data: &[u8] = b"\0x v1\0Synthetic Label v2.31.4.0.9af3\0lower v1.2.3.ab\0Two v1.2\0";
        assert_eq!(
            version_banner(&[data]).unwrap(),
            "Synthetic Label v2.31.4.0.9af3"
        );
        assert_eq!(
            version_banner(&[data, b"\0Other Label v2.31.4.0.9af4\0"])
                .unwrap_err()
                .reason,
            "the executable holds more than one version-shaped string"
        );
        assert!(version_banner(&[b"\0Title v1.2.zz\0"]).is_err());
    }

}
