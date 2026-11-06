use crate::availability::{Verified, Withheld};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, io::{self, Read}};

/// Default modifier row separating fixed registrations from dynamic definitions.
pub const FIXED_MODIFIER_GAP: u32 = 0x29a;

/// An executable identity this release was made against.
#[derive(Debug, Clone, Copy)]
pub struct Build {
    /// The release number players see, for messages.
    pub version: &'static str,
    pub exe_size: u64,
    pub exe_sha256: [u8; 32],
}

/// The build this release recognises. Any other build is read part by part, each part
/// verifying itself.
pub const PINNED_BUILD: Build = Build {
    version: "1.19.4.2",
    exe_size: 56_535_160,
    exe_sha256: [
        0xbc, 0xcc, 0x62, 0x90, 0x9b, 0x6a, 0x1e, 0xc2, 0xc1, 0xd5, 0xa4, 0x6e, 0xf1, 0x86, 0x08,
        0xe4, 0xd0, 0xc7, 0xcd, 0xc8, 0x9a, 0xd3, 0x41, 0xb7, 0x40, 0x2a, 0x23, 0xc4, 0x0f, 0x7a,
        0x80, 0x37,
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

    /// Plain-language warning for a build this release does not recognise.
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

/// The game build a recording names, as its recorder saw it.
pub struct RecordedBuild<'a> {
    pub version: &'a str,
    pub identity: ExecutableIdentity,
}

impl RecordedBuild<'_> {
    /// A plain-language warning when the installed game (its identity and version text)
    /// is another build than the recorded one.
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
            "This recording was made with game {} ({}), but the installed game is {installed}. \
             Installed content and tables are read from the installed game.",
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

/// The executable's own version text, found by its shape: the one NUL-delimited string in
/// read-only data of the form `<Title> v<n>.<n>.<n>[.<n>…].<hex>`. None or several withhold it.
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

    #[test]
    fn unrecognised_identity_warns_with_both_builds() {
        let identity = ExecutableIdentity::of(b"other");
        let warning = identity.warning(Some("Label v9.9.9.9.abcd")).unwrap();
        assert!(warning.contains(PINNED_BUILD.version));
        assert!(warning.contains("Label v9.9.9.9.abcd (hoi4.exe of 5 bytes"));
    }
}
