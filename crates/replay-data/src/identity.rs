//! The recorded wire form of the content identity ([`replay_identity`]):
//! validated on write and on read.

use std::io;

use crate::codec::{Decoder, invalid_static as invalid, put_string};
use engine_allocation as allocation;
pub use replay_identity::*;

/// The identity types as the recording writes and reads them.
pub trait IdentityCodec: Sized {
    /// Validates and appends the recorded form.
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()>;
    /// Reads and validates the recorded form.
    fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self>;
}

fn put_count(out: &mut Vec<u8>, count: usize) -> io::Result<()> {
    let count = u32::try_from(count).map_err(|_| invalid("identity list exceeds u32"))?;
    allocation::reserve(out, 4)?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn hash(decoder: &mut Decoder<'_>) -> io::Result<[u8; 32]> {
    Ok(decoder.take(32)?.try_into().unwrap())
}

fn encode_roster(roster: &Roster, out: &mut Vec<u8>) -> io::Result<()> {
    allocation::reserve(out, roster.encoded_bytes()?)?;
    for list in [&roster.dlc, &roster.mods] {
        put_count(out, list.len())?;
        for id in list {
            put_string(out, id)?;
        }
    }
    Ok(())
}

fn decode_roster(decoder: &mut Decoder<'_>) -> io::Result<Roster> {
    fn ids(decoder: &mut Decoder<'_>) -> io::Result<Vec<String>> {
        let count = decoder.count(4)?;
        let mut ids = allocation::vec(count)?;
        for _ in 0..count {
            ids.push(decoder.string()?);
        }
        Ok(ids)
    }
    let roster = Roster {
        dlc: ids(decoder)?,
        mods: ids(decoder)?,
    };
    roster.validate()?;
    Ok(roster)
}

impl IdentityCodec for Identity {
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        allocation::reserve(out, self.encoded_bytes()?)?;
        put_string(out, &self.game_version)?;
        out.extend_from_slice(&self.exe_hash);
        out.extend_from_slice(&self.exe_size.to_le_bytes());
        out.extend_from_slice(&self.vanilla_hash);
        put_count(out, self.dlc.len())?;
        for layer in &self.dlc {
            put_string(out, &layer.id)?;
            out.extend_from_slice(&layer.hash);
            out.push(u8::from(layer.declared_checksum.is_some()));
            if let Some(checksum) = &layer.declared_checksum {
                put_string(out, checksum)?;
            }
        }
        put_count(out, self.mods.len())?;
        for layer in &self.mods {
            put_string(out, &layer.id)?;
            put_string(out, &layer.display_name)?;
            put_string(out, &layer.declared_version)?;
            out.extend_from_slice(&layer.hash);
        }
        out.extend_from_slice(&self.coverage_policy.to_le_bytes());
        put_string(out, &self.language)?;
        put_string(out, &self.recorder_version)?;
        out.extend_from_slice(&self.collector_hash);
        match &self.verification {
            Verification::Unchecked => out.push(0),
            Verification::Verified { launcher, memory } => {
                out.push(1);
                encode_roster(launcher, out)?;
                encode_roster(memory, out)?;
            }
            Verification::Unverified {
                reason,
                launcher,
                memory,
            } => {
                out.push(2);
                put_string(out, reason)?;
                encode_roster(launcher, out)?;
                encode_roster(memory, out)?;
            }
        }
        Ok(())
    }

    fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self> {
        let game_version = decoder.string()?;
        let exe_hash = hash(decoder)?;
        let exe_size = decoder.u64()?;
        let vanilla_hash = hash(decoder)?;
        let count = decoder.count(37)?;
        let mut dlc = allocation::vec(count)?;
        for _ in 0..count {
            dlc.push(Dlc {
                id: decoder.string()?,
                hash: hash(decoder)?,
                declared_checksum: if decoder.bool()? {
                    Some(decoder.string()?)
                } else {
                    None
                },
            });
        }
        let count = decoder.count(44)?;
        let mut mods = allocation::vec(count)?;
        for _ in 0..count {
            mods.push(Mod {
                id: decoder.string()?,
                display_name: decoder.string()?,
                declared_version: decoder.string()?,
                hash: hash(decoder)?,
            });
        }
        let coverage_policy = decoder.u32()?;
        let language = decoder.string()?;
        let recorder_version = decoder.string()?;
        let collector_hash = hash(decoder)?;
        let verification = match decoder.u8()? {
            0 => Verification::Unchecked,
            1 => Verification::Verified {
                launcher: decode_roster(decoder)?,
                memory: decode_roster(decoder)?,
            },
            2 => Verification::Unverified {
                reason: decoder.string()?,
                launcher: decode_roster(decoder)?,
                memory: decode_roster(decoder)?,
            },
            _ => return Err(invalid("unknown identity verification tag")),
        };
        let identity = Self {
            game_version,
            exe_hash,
            exe_size,
            vanilla_hash,
            dlc,
            mods,
            coverage_policy,
            language,
            recorder_version,
            collector_hash,
            verification,
        };
        identity.validate()?;
        Ok(identity)
    }
}

impl IdentityCodec for SessionStart {
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        allocation::reserve(out, self.encoded_bytes()?)?;
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.process_id.to_le_bytes());
        out.extend_from_slice(&self.process_start_time.to_le_bytes());
        out.extend_from_slice(&self.qpc_frequency.to_le_bytes());
        out.push(self.sample_boundary.byte());
        self.identity.encode(out)
    }

    fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self> {
        let session = Self {
            id: decoder.u64()?,
            process_id: decoder.u32()?,
            process_start_time: decoder.u64()?,
            qpc_frequency: decoder.u64()?,
            sample_boundary: SampleBoundary::from_byte(decoder.u8()?)?,
            identity: Identity::decode(decoder)?,
        };
        session.validate()?;
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_identity::test_fixtures::identity as fixture;

    #[test]
    fn coverage_policy_four_and_old_policies_three_and_two_round_trip_without_translation() {
        assert_eq!(COVERAGE_POLICY_VERSION, 4);
        let mut lengths = Vec::new();
        for policy in [COVERAGE_POLICY_VERSION, 3, 2] {
            let mut identity = fixture();
            identity.coverage_policy = policy;
            let mut bytes = Vec::new();
            identity.encode(&mut bytes).unwrap();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(Identity::decode(&mut decoder).unwrap(), identity);
            decoder.finish().unwrap();
            lengths.push(bytes.len());
        }
        assert!(lengths.iter().all(|length| *length == lengths[0]));
    }

    #[test]
    fn session_fields_keep_all_bits_and_hash_bytes() {
        let mut identity = fixture();
        identity.exe_size = u64::MAX;
        // Unknown policies are preserved for a viewer to name and refuse.
        identity.coverage_policy = u32::MAX;
        let start = SessionStart {
            id: u64::MAX - 1,
            process_id: u32::MAX,
            process_start_time: u64::MAX - 2,
            qpc_frequency: u64::MAX - 3,
            sample_boundary: SampleBoundary::EndOfHour,
            identity,
        };
        let mut bytes = Vec::new();
        start.encode(&mut bytes).unwrap();
        assert_eq!(&bytes[..8], &(u64::MAX - 1).to_le_bytes());
        assert_eq!(&bytes[8..12], &u32::MAX.to_le_bytes());
        assert_eq!(&bytes[12..20], &(u64::MAX - 2).to_le_bytes());
        assert_eq!(&bytes[20..28], &(u64::MAX - 3).to_le_bytes());
        assert_eq!(bytes[28], 1);
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(SessionStart::decode(&mut decoder).unwrap(), start);
        decoder.finish().unwrap();
        for length in 0..bytes.len() {
            assert!(SessionStart::decode(&mut Decoder::new(&bytes[..length])).is_err());
        }
    }

    #[test]
    fn session_refuses_zero_qpc_frequency_on_write_and_read() {
        let mut session = SessionStart {
            id: 1,
            process_id: 77_760,
            process_start_time: 134_000_000_000_000_000,
            qpc_frequency: 24_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: fixture(),
        };
        let mut bytes = Vec::new();
        session.encode(&mut bytes).unwrap();
        bytes[20..28].fill(0);
        assert!(SessionStart::decode(&mut Decoder::new(&bytes)).is_err());
        session.qpc_frequency = 0;
        let mut output = vec![123];
        assert!(session.encode(&mut output).is_err());
        assert_eq!(output, [123]);
    }

    #[test]
    fn sample_boundary_is_explicit_and_rejects_unknown_tags() {
        let mut session = SessionStart {
            id: 7,
            process_id: 1,
            process_start_time: 2,
            qpc_frequency: 3,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: fixture(),
        };
        let mut current = Vec::new();
        session.encode(&mut current).unwrap();
        assert_eq!(current[28], 1);
        let decoded = SessionStart::decode(&mut Decoder::new(&current)).unwrap();
        assert_eq!(decoded.sample_boundary, SampleBoundary::EndOfHour);
        let mut unknown = current.clone();
        unknown[28] = 2;
        assert!(SessionStart::decode(&mut Decoder::new(&unknown)).is_err());
        session.sample_boundary = SampleBoundary::AfterHourlyUpdate;
        let mut bytes = Vec::new();
        session.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), current.len());
        let mut decoder = Decoder::new(&bytes);
        let decoded = SessionStart::decode(&mut decoder).unwrap();
        decoder.finish().unwrap();
        assert_eq!(decoded, session);
        assert_eq!(decoded.sample_boundary.name(), "after_hourly_update");
        assert_eq!(SampleBoundary::EndOfHour.to_string(), "end_of_hour");
    }

    #[test]
    fn verified_requires_both_rosters_and_identity_order() {
        let mut identity = fixture();
        let roster = identity.roster();
        identity.verification = Verification::Verified {
            launcher: roster.clone(),
            memory: roster.clone(),
        };
        let mut bytes = Vec::new();
        identity.encode(&mut bytes).unwrap();
        assert_eq!(
            Identity::decode(&mut Decoder::new(&bytes)).unwrap(),
            identity
        );

        let mut reversed = roster.clone();
        reversed.dlc.reverse();
        identity.verification = Verification::Verified {
            launcher: roster,
            memory: reversed.clone(),
        };
        assert!(identity.validate().is_err());
        identity.verification = Verification::Verified {
            launcher: reversed.clone(),
            memory: reversed,
        };
        assert!(identity.validate().is_err());

        identity.verification = Verification::Unchecked;
        bytes.clear();
        identity.encode(&mut bytes).unwrap();
        bytes.pop();
        bytes.push(1);
        encode_roster(&Roster::default(), &mut bytes).unwrap();
        encode_roster(&Roster::default(), &mut bytes).unwrap();
        assert!(Identity::decode(&mut Decoder::new(&bytes)).is_err());
    }

    #[test]
    fn unverified_retains_both_lists_and_the_reason() {
        let mut identity = fixture();
        let launcher = identity.roster();
        let mut memory = launcher.clone();
        memory.mods.reverse();
        identity.verification = Verification::Unverified {
            reason: "in-memory mod order differs from launcher".into(),
            launcher,
            memory,
        };
        let mut bytes = Vec::new();
        identity.encode(&mut bytes).unwrap();
        assert_eq!(
            Identity::decode(&mut Decoder::new(&bytes)).unwrap(),
            identity
        );
    }

    #[test]
    fn identifiers_refuse_paths_and_duplicates_on_write_and_read() {
        for id in [
            "",
            " ",
            ".",
            "..",
            "../mod",
            "mods/local.mod",
            "C:\\mods\\local.mod",
            "C:local.mod",
            "/tmp/local.mod",
            "\\\\host\\mod",
            "line\nbreak",
        ] {
            let mut identity = fixture();
            identity.mods[0].id = id.into();
            assert!(identity.encode(&mut Vec::new()).is_err(), "accepted {id:?}");
        }
        let mut identity = fixture();
        identity.dlc.push(identity.dlc[0].clone());
        assert!(identity.validate().is_err());

        let mut bytes = Vec::new();
        fixture().encode(&mut bytes).unwrap();
        let at = bytes.windows(9).position(|s| s == b"dlc_alpha").unwrap();
        bytes[at..at + 9].copy_from_slice(b"../escape");
        assert!(Identity::decode(&mut Decoder::new(&bytes)).is_err());
    }

    #[test]
    fn unknown_verification_tags_and_missing_required_metadata_are_refused() {
        let mut bytes = Vec::new();
        fixture().encode(&mut bytes).unwrap();
        *bytes.last_mut().unwrap() = 3;
        assert!(Identity::decode(&mut Decoder::new(&bytes)).is_err());
        let mut identity = fixture();
        identity.language.clear();
        assert!(identity.validate().is_err());
        identity = fixture();
        identity.exe_size = 0;
        assert!(identity.validate().is_err());
    }
}
