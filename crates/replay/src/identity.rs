//! Machine-free content identity. Layer order is part of the identity.

use std::{collections::BTreeSet, io};

use crate::{
    SessionId,
    codec::{Decoder, invalid, put_string},
};

pub const COVERAGE_POLICY_VERSION: u32 = 1;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStart {
    pub id: SessionId,
    pub process_id: u32,
    /// Process creation FILETIME: 100 ns ticks since 1601-01-01 UTC.
    pub process_start_time: u64,
    /// QueryPerformanceFrequency ticks per second for this session's clock witnesses.
    pub qpc_frequency: u64,
    pub identity: Identity,
}

fn nonempty(value: &str, field: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(invalid(format!("{field} is empty or contains NUL")));
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

fn validate_ids<'a>(ids: impl Iterator<Item = &'a str>) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        logical_id(id)?;
        if !seen.insert(id) {
            return Err(invalid(format!("duplicate layer id: {id}")));
        }
    }
    Ok(())
}

fn put_count(out: &mut Vec<u8>, count: usize) -> io::Result<()> {
    let count = u32::try_from(count).map_err(|_| invalid("identity list exceeds u32"))?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn hash(decoder: &mut Decoder<'_>) -> io::Result<[u8; 32]> {
    Ok(decoder.take(32)?.try_into().unwrap())
}

impl Roster {
    pub fn validate(&self) -> io::Result<()> {
        validate_ids(self.dlc.iter().map(String::as_str))?;
        validate_ids(self.mods.iter().map(String::as_str))
    }

    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
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
            (0..count).map(|_| decoder.string()).collect()
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
                if launcher != memory || *launcher != self.roster() {
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
        let mut dlc = Vec::with_capacity(count);
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
        let mut mods = Vec::with_capacity(count);
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
    pub fn validate(&self) -> io::Result<()> {
        if self.qpc_frequency == 0 {
            return Err(invalid("session QPC frequency is zero"));
        }
        self.identity.validate()
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.process_id.to_le_bytes());
        out.extend_from_slice(&self.process_start_time.to_le_bytes());
        out.extend_from_slice(&self.qpc_frequency.to_le_bytes());
        self.identity.encode(out)
    }

    pub fn decode(decoder: &mut Decoder<'_>) -> io::Result<Self> {
        let session = Self {
            id: decoder.u64()?,
            process_id: decoder.u32()?,
            process_start_time: decoder.u64()?,
            qpc_frequency: decoder.u64()?,
            identity: Identity::decode(decoder)?,
        };
        session.validate()?;
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Identity {
        Identity {
            game_version: "Operation Postern v1.19.2.0.a729".into(),
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
            identity,
        };
        let mut bytes = Vec::new();
        start.encode(&mut bytes).unwrap();
        assert_eq!(&bytes[..8], &(u64::MAX - 1).to_le_bytes());
        assert_eq!(&bytes[8..12], &u32::MAX.to_le_bytes());
        assert_eq!(&bytes[12..20], &(u64::MAX - 2).to_le_bytes());
        assert_eq!(&bytes[20..28], &(u64::MAX - 3).to_le_bytes());
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
