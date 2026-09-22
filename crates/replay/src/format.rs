//! The recording contract accepted by this build.
//!
//! Replay files and spool segments use the current format only. A different
//! version is rejected before its payload is decoded. Each frame stores one
//! timing and one update for every table in [`crate::rows::TableId::ALL`], in
//! that order. Replay derives ETA from recorded movement operands.

use crate::{FORMAT_VERSION, codec::invalid};
use std::io;

/// Proof that a recording's version was accepted by this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatVersion {
    _private: (),
}

impl FormatVersion {
    pub const CURRENT: Self = Self { _private: () };

    pub fn read(version: u16) -> io::Result<Self> {
        if version == FORMAT_VERSION {
            Ok(Self::CURRENT)
        } else {
            Err(invalid(format!(
                "unsupported replay format version {version}; this build reads and writes {FORMAT_VERSION}"
            )))
        }
    }

    pub const fn version(self) -> u16 {
        FORMAT_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_version_is_accepted() {
        for version in 0..=u16::MAX {
            match FormatVersion::read(version) {
                Ok(format) => {
                    assert_eq!(version, FORMAT_VERSION);
                    assert_eq!(format, FormatVersion::CURRENT);
                    assert_eq!(format.version(), version);
                }
                Err(error) => {
                    assert_ne!(version, FORMAT_VERSION);
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                }
            }
        }
        assert_eq!(
            FormatVersion::read(FORMAT_VERSION - 1)
                .unwrap_err()
                .to_string(),
            format!(
                "unsupported replay format version {}; this build reads and writes {FORMAT_VERSION}",
                FORMAT_VERSION - 1
            )
        );
    }
}
