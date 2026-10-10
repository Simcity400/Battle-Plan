//! Machine-free content identity. Layer order is part of the identity.

use std::io;

use crate::{
    SessionId,
    codec::{Decoder, invalid_static as invalid, put_string},
};
use engine::allocation;

pub const COVERAGE_POLICY_VERSION: u32 = 4;

/// Whether a campaign identity can be used unchanged as a UUID file name.
pub fn is_campaign_uuid(bytes: &[u8]) -> bool {
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, byte)| {
            if [8, 13, 18, 23].contains(&i) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dlc {
    pub id: String,
    pub hash: [u8; 32],
    /// The descriptor's claim, retained separately from the installed-byte hash.
    pub declared_checksum: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mod {
    /// Workshop remote_file_id, or a local descriptor's file name.
    pub id: String,
    pub display_name: String,
    /// Empty when the descriptor declares no version.
    pub declared_version: String,
    pub hash: [u8; 32],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    pub dlc: Vec<String>,
    pub mods: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Verification {
    #[default]
    Unchecked,
    Verified {
        launcher: Roster,
        memory: Roster,
    },
    Unverified {
        reason: String,
        launcher: Roster,
        memory: Roster,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub game_version: String,
    pub exe_hash: [u8; 32],
    pub exe_size: u64,
    pub vanilla_hash: [u8; 32],
    pub dlc: Vec<Dlc>,
    pub mods: Vec<Mod>,
    /// Readers must understand this policy before claiming byte-equal content.
    pub coverage_policy: u32,
    pub language: String,
    pub recorder_version: String,
    pub collector_hash: [u8; 32],
    pub verification: Verification,
}

/// Which instant of the engine hour a session's frames sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleBoundary {
    /// After the hooked full tick returns, so commands drained later in the
    /// same hour are absent from hour H.
    AfterHourlyUpdate,
    /// At the next full tick's entry, before the date increment, so hour H
    /// includes every command of hour H.
    EndOfHour,
}

impl SampleBoundary {
    pub const fn name(self) -> &'static str {
        match self {
            Self::AfterHourlyUpdate => "after_hourly_update",
            Self::EndOfHour => "end_of_hour",
        }
    }

    const fn byte(self) -> u8 {
        match self {
            Self::AfterHourlyUpdate => 0,
            Self::EndOfHour => 1,
        }
    }

    fn from_byte(byte: u8) -> io::Result<Self> {
        match byte {
            0 => Ok(Self::AfterHourlyUpdate),
            1 => Ok(Self::EndOfHour),
            _ => Err(invalid("unknown session sample boundary")),
        }
    }
}

impl std::fmt::Display for SampleBoundary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStart {
    pub id: SessionId,
    pub process_id: u32,
    /// Process creation FILETIME: 100 ns ticks since 1601-01-01 UTC.
    pub process_start_time: u64,
    /// QueryPerformanceFrequency ticks per second for this session's clock witnesses.
    pub qpc_frequency: u64,
    /// The explicitly recorded acquisition boundary.
    pub sample_boundary: SampleBoundary,
    pub identity: Identity,
}

fn nonempty(value: &str, _field: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(invalid("identity field is empty or contains NUL"));
    }
    Ok(())
}

fn logical_id(value: &str) -> io::Result<()> {
    nonempty(value, "layer id")?;
    if value == "."
        || value == ".."
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return Err(invalid(
            "layer id must be one logical identifier, without a path",
        ));
    }
    Ok(())
}

fn validate_ids<'a>(ids: impl ExactSizeIterator<Item = &'a str>) -> io::Result<()> {
    let mut seen = allocation::vec(ids.len())?;
    for id in ids {
        logical_id(id)?;
        seen.push(id);
    }
    seen.sort_unstable();
    if seen.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid("duplicate content layer id"));
    }
    Ok(())
}

fn string_bytes(value: &str) -> io::Result<usize> {
    allocation::add(4, value.len())
}
fn clone_strings(values: &[String]) -> io::Result<Vec<String>> {
    let mut owned = allocation::vec(values.len())?;
    for value in values {
        owned.push(allocation::string(value)?);
    }
    Ok(owned)
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

impl Roster {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            dlc: clone_strings(&self.dlc)?,
            mods: clone_strings(&self.mods)?,
        })
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.dlc
            .iter()
            .chain(&self.mods)
            .try_fold(8, |size, id| allocation::add(size, string_bytes(id)?))
    }
    pub fn validate(&self) -> io::Result<()> {
        validate_ids(self.dlc.iter().map(String::as_str))?;
        validate_ids(self.mods.iter().map(String::as_str))
    }

    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(out, self.encoded_bytes()?)?;
        for list in [&self.dlc, &self.mods] {
            put_count(out, list.len())?;
            for id in list {
                put_string(out, id)?;
            }
        }
        Ok(())
    }

    fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self> {
        fn ids(decoder: &mut Decoder<'_>) -> io::Result<Vec<String>> {
            let count = decoder.count(4)?;
            let mut ids = allocation::vec(count)?;
            for _ in 0..count {
                ids.push(decoder.string()?);
            }
            Ok(ids)
        }
        let roster = Self {
            dlc: ids(decoder)?,
            mods: ids(decoder)?,
        };
        roster.validate()?;
        Ok(roster)
    }
}

impl Identity {
    pub fn try_roster(&self) -> io::Result<Roster> {
        let mut dlc = allocation::vec(self.dlc.len())?;
        for layer in &self.dlc {
            dlc.push(allocation::string(&layer.id)?);
        }
        let mut mods = allocation::vec(self.mods.len())?;
        for layer in &self.mods {
            mods.push(allocation::string(&layer.id)?);
        }
        Ok(Roster { dlc, mods })
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        let mut dlc = allocation::vec(self.dlc.len())?;
        for layer in &self.dlc {
            dlc.push(Dlc {
                id: allocation::string(&layer.id)?,
                hash: layer.hash,
                declared_checksum: layer
                    .declared_checksum
                    .as_deref()
                    .map(allocation::string)
                    .transpose()?,
            });
        }
        let mut mods = allocation::vec(self.mods.len())?;
        for layer in &self.mods {
            mods.push(Mod {
                id: allocation::string(&layer.id)?,
                display_name: allocation::string(&layer.display_name)?,
                declared_version: allocation::string(&layer.declared_version)?,
                hash: layer.hash,
            });
        }
        let verification = match &self.verification {
            Verification::Unchecked => Verification::Unchecked,
            Verification::Verified { launcher, memory } => Verification::Verified {
                launcher: launcher.try_clone()?,
                memory: memory.try_clone()?,
            },
            Verification::Unverified {
                reason,
                launcher,
                memory,
            } => Verification::Unverified {
                reason: allocation::string(reason)?,
                launcher: launcher.try_clone()?,
                memory: memory.try_clone()?,
            },
        };
        Ok(Self {
            game_version: allocation::string(&self.game_version)?,
            exe_hash: self.exe_hash,
            exe_size: self.exe_size,
            vanilla_hash: self.vanilla_hash,
            dlc,
            mods,
            coverage_policy: self.coverage_policy,
            language: allocation::string(&self.language)?,
            recorder_version: allocation::string(&self.recorder_version)?,
            collector_hash: self.collector_hash,
            verification,
        })
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        let mut bytes = 32 + 8 + 32 + 4 + 4 + 4 + 32 + 1;
        for value in [&self.game_version, &self.language, &self.recorder_version] {
            bytes = allocation::add(bytes, string_bytes(value)?)?;
        }
        for layer in &self.dlc {
            bytes = allocation::add(bytes, allocation::add(33, string_bytes(&layer.id)?)?)?;
            if let Some(value) = &layer.declared_checksum {
                bytes = allocation::add(bytes, string_bytes(value)?)?;
            }
        }
        for layer in &self.mods {
            bytes = allocation::add(bytes, 32)?;
            for value in [&layer.id, &layer.display_name, &layer.declared_version] {
                bytes = allocation::add(bytes, string_bytes(value)?)?;
            }
        }
        match &self.verification {
            Verification::Unchecked => (),
            Verification::Verified { launcher, memory } => {
                bytes = allocation::add(
                    bytes,
                    allocation::add(launcher.encoded_bytes()?, memory.encoded_bytes()?)?,
                )?;
            }
            Verification::Unverified {
                reason,
                launcher,
                memory,
            } => {
                bytes = allocation::add(bytes, string_bytes(reason)?)?;
                bytes = allocation::add(
                    bytes,
                    allocation::add(launcher.encoded_bytes()?, memory.encoded_bytes()?)?,
                )?;
            }
        }
        Ok(bytes)
    }
    pub fn roster(&self) -> Roster {
        Roster {
            dlc: self.dlc.iter().map(|layer| layer.id.clone()).collect(),
            mods: self.mods.iter().map(|layer| layer.id.clone()).collect(),
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        nonempty(&self.game_version, "game version")?;
        nonempty(&self.language, "language")?;
        nonempty(&self.recorder_version, "recorder version")?;
        if self.exe_size == 0 {
            return Err(invalid("game executable size is zero"));
        }
        validate_ids(self.dlc.iter().map(|layer| layer.id.as_str()))?;
        validate_ids(self.mods.iter().map(|layer| layer.id.as_str()))?;
        for layer in &self.mods {
            nonempty(&layer.display_name, "mod display name")?;
        }
        match &self.verification {
            Verification::Unchecked => {}
            Verification::Verified { launcher, memory } => {
                launcher.validate()?;
                memory.validate()?;
                if launcher != memory
                    || launcher.dlc.len() != self.dlc.len()
                    || launcher.mods.len() != self.mods.len()
                    || launcher
                        .dlc
                        .iter()
                        .zip(&self.dlc)
                        .any(|(id, layer)| *id != layer.id)
                    || launcher
                        .mods
                        .iter()
                        .zip(&self.mods)
                        .any(|(id, layer)| *id != layer.id)
                {
                    return Err(invalid(
                        "verified rosters do not match the ordered content layers",
                    ));
                }
            }
            Verification::Unverified {
                reason,
                launcher,
                memory,
            } => {
                nonempty(reason, "unverified reason")?;
                launcher.validate()?;
                memory.validate()?;
            }
        }
        Ok(())
    }

    /// Tool builds and verification outcomes belong to each session. They do not
    /// change the content needed to render sessions together.
    pub fn same_content(&self, other: &Self) -> bool {
        self.game_version == other.game_version
            && self.exe_hash == other.exe_hash
            && self.exe_size == other.exe_size
            && self.vanilla_hash == other.vanilla_hash
            && self.dlc == other.dlc
            && self.mods == other.mods
            && self.coverage_policy == other.coverage_policy
            && self.language == other.language
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
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
                launcher.encode(out)?;
                memory.encode(out)?;
            }
            Verification::Unverified {
                reason,
                launcher,
                memory,
            } => {
                out.push(2);
                put_string(out, reason)?;
                launcher.encode(out)?;
                memory.encode(out)?;
            }
        }
        Ok(())
    }

    pub fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self> {
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
                launcher: Roster::decode(decoder)?,
                memory: Roster::decode(decoder)?,
            },
            2 => Verification::Unverified {
                reason: decoder.string()?,
                launcher: Roster::decode(decoder)?,
                memory: Roster::decode(decoder)?,
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

impl SessionStart {
    pub(crate) fn encoded_bytes(&self) -> io::Result<usize> {
        allocation::add(29, self.identity.encoded_bytes()?)
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            id: self.id,
            process_id: self.process_id,
            process_start_time: self.process_start_time,
            qpc_frequency: self.qpc_frequency,
            sample_boundary: self.sample_boundary,
            identity: self.identity.try_clone()?,
        })
    }
    pub fn validate(&self) -> io::Result<()> {
        if self.qpc_frequency == 0 {
            return Err(invalid("session QPC frequency is zero"));
        }
        self.identity.validate()
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        allocation::reserve(out, self.encoded_bytes()?)?;
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.process_id.to_le_bytes());
        out.extend_from_slice(&self.process_start_time.to_le_bytes());
        out.extend_from_slice(&self.qpc_frequency.to_le_bytes());
        out.push(self.sample_boundary.byte());
        self.identity.encode(out)
    }

    pub fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self> {
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

    #[test]
    fn campaign_uuid_accepts_original_case_and_refuses_unsafe_paths() {
        for (id, accepted) in [
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6".as_slice(), true),
            (b"21B7CC84-3028-4F65-8E0D-ADF2F37CA1A6", true),
            (b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6", true),
            (b"00000000-0000-0000-0000-000000000000", true),
            (b"", false),
            (b"../campaign", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a66", false),
            (b"21b7cc843028-4f65-8e0d-adf2f37ca1a6-", false),
            (b"21b7cc84_3028-4f65-8e0d-adf2f37ca1a6", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1ag", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a/", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a\\", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a\0", false),
            (b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a\xff", false),
        ] {
            assert_eq!(is_campaign_uuid(id), accepted, "{id:?}");
        }
    }

    fn fixture() -> Identity {
        Identity {
            game_version: "Synthetic version 1.2.3".into(),
            exe_hash: std::array::from_fn(|i| i as u8),
            exe_size: 55_680_632,
            vanilla_hash: [41; 32],
            dlc: vec![
                Dlc {
                    id: "dlc_alpha".into(),
                    hash: [42; 32],
                    declared_checksum: Some("declared only".into()),
                },
                Dlc {
                    id: "dlc_beta".into(),
                    hash: [43; 32],
                    declared_checksum: None,
                },
            ],
            mods: vec![
                Mod {
                    id: "2819901234".into(),
                    display_name: "Workshop mod".into(),
                    declared_version: "1.2".into(),
                    hash: [44; 32],
                },
                Mod {
                    id: "Local mod.mod".into(),
                    display_name: "Local mod".into(),
                    declared_version: String::new(),
                    hash: [45; 32],
                },
            ],
            coverage_policy: COVERAGE_POLICY_VERSION,
            language: "l_english".into(),
            recorder_version: "0.1.0".into(),
            collector_hash: [46; 32],
            verification: Verification::Unchecked,
        }
    }

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
        Roster::default().encode(&mut bytes).unwrap();
        Roster::default().encode(&mut bytes).unwrap();
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
    fn content_compatibility_excludes_tools_but_preserves_layer_order() {
        let original = fixture();
        let mut changed = original.clone();
        changed.recorder_version = "new recorder".into();
        changed.collector_hash = [99; 32];
        let roster = changed.roster();
        changed.verification = Verification::Verified {
            launcher: roster.clone(),
            memory: roster,
        };
        assert!(original.same_content(&changed));
        changed.mods.reverse();
        assert!(!original.same_content(&changed));
        changed = original.clone();
        changed.dlc[0].hash[0] ^= 1;
        assert!(!original.same_content(&changed));
        changed = original.clone();
        changed.coverage_policy += 1;
        assert!(!original.same_content(&changed));
        changed = original.clone();
        changed.language = "l_french".into();
        assert!(!original.same_content(&changed));
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
