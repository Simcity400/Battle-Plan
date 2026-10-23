//! Source-load bounds. Only the new wider optimization may retry a prefix.
use replay_data::Hour;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Load {
    RequestedPrefix,
    AcceptedBlock,
    AcceptedBlockOrPrefix,
}

impl Load {
    pub(super) fn required(self, hour: Hour, accepted_end: Hour) -> Hour {
        if self == Self::AcceptedBlock {
            accepted_end
        } else {
            hour.min(accepted_end)
        }
    }

    pub(super) fn read<T>(
        self,
        required: Hour,
        accepted_end: Hour,
        mut read: impl FnMut(Hour) -> io::Result<T>,
    ) -> io::Result<(Hour, T)> {
        if self == Self::AcceptedBlockOrPrefix
            && required < accepted_end
            && let Ok(value) = read(accepted_end)
        {
            return Ok((accepted_end, value));
        }
        // Only speculative loads retry; the original prefix owns its refusal precedence.
        read(required).map(|value| (required, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_reuse_retries_every_wider_error_and_returns_original_precedence() {
        for kind in [
            io::ErrorKind::InvalidData,
            io::ErrorKind::OutOfMemory,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::Interrupted,
        ] {
            let mut calls = Vec::new();
            let (through, value) = Load::AcceptedBlockOrPrefix
                .read(24, 719, |through| {
                    calls.push(through);
                    if through == 719 {
                        Err(io::Error::new(kind, "future suffix"))
                    } else {
                        Ok(vec![24])
                    }
                })
                .unwrap();
            assert_eq!((through, value, calls), (24, vec![24], vec![719, 24]));
            let error = Load::AcceptedBlockOrPrefix
                .read::<()>(24, 719, |through| {
                    if through == 719 {
                        Err(io::Error::new(kind, "future suffix"))
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "original prefix",
                        ))
                    }
                })
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), "original prefix");
            // A failed attempt is not retained by the policy.
            assert_eq!(
                Load::AcceptedBlockOrPrefix.read(24, 719, Ok).unwrap(),
                (719, 719)
            );
        }
    }

    #[test]
    fn accepted_reuse_keeps_cold_and_consecutive_error_policies_and_equal_bounds() {
        for (load, required, calls) in [
            (Load::RequestedPrefix, 24, vec![24]),
            (Load::AcceptedBlock, 719, vec![719]),
            (Load::AcceptedBlockOrPrefix, 719, vec![719]),
        ] {
            let mut actual = Vec::new();
            let error = load
                .read::<()>(required, 719, |through| {
                    actual.push(through);
                    Err(io::Error::new(io::ErrorKind::InvalidData, "first"))
                })
                .unwrap_err();
            assert_eq!(error.to_string(), "first");
            assert_eq!(actual, calls);
        }
        assert_eq!(Load::RequestedPrefix.required(24, 719), 24);
        assert_eq!(Load::AcceptedBlock.required(24, 719), 719);
        assert_eq!(Load::AcceptedBlockOrPrefix.required(24, 719), 24);
        assert_eq!(Load::AcceptedBlockOrPrefix.required(800, 719), 719);
    }
}
