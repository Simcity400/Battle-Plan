//! Machine-free content identity. Layer order is part of the identity.
//!
//! The types and their validation; replay-data's `identity` module writes and
//! reads them in the recording. Nothing here depends on the recording format,
//! so the content layer (installed-content) builds on this crate without the replay
//! chain.

use std::io;

use engine_allocation as allocation;

/// An invalid identity field. Collector error paths cannot allocate even for a
/// static diagnostic.
fn invalid(message: &'static str) -> io::Error {
    if allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}

pub const COVERAGE_POLICY_VERSION: u32 = 5;

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
    /// After the hooked full tick returns and the commands its batch runs behind it
    /// (straight after the tick where that batch is not observed), so commands of later
    /// batches in the same hour are absent from hour H.
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

    /// The recorded tag.
    pub const fn byte(self) -> u8 {
        match self {
            Self::AfterHourlyUpdate => 0,
            Self::EndOfHour => 1,
        }
    }

    /// The boundary a recorded tag names; unknown tags are refused.
    pub fn from_byte(byte: u8) -> io::Result<Self> {
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

/// Which tables a recording holds. replay-data's `scope` module lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(replay_spool::Spooled)]
pub enum CaptureMode {
    Full,
    Map,
}

impl CaptureMode {
    pub const ALL: [Self; 2] = [Self::Full, Self::Map];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Map => "map",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }

    pub const fn byte(self) -> u8 {
        match self {
            Self::Full => 0,
            Self::Map => 1,
        }
    }

    pub fn from_byte(byte: u8) -> io::Result<Self> {
        match byte {
            0 => Ok(Self::Full),
            1 => Ok(Self::Map),
            _ => Err(invalid("unknown capture mode")),
        }
    }
}

impl std::fmt::Display for CaptureMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStart {
    /// The session's id (replay-data's `SessionId`).
    pub id: u64,
    pub process_id: u32,
    /// Process creation FILETIME: 100 ns ticks since 1601-01-01 UTC.
    pub process_start_time: u64,
    /// QueryPerformanceFrequency ticks per second for this session's clock witnesses.
    pub qpc_frequency: u64,
    /// The explicitly recorded acquisition boundary.
    pub sample_boundary: SampleBoundary,
    /// The same for every session of a recording file.
    pub capture_mode: CaptureMode,
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

impl Roster {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            dlc: clone_strings(&self.dlc)?,
            mods: clone_strings(&self.mods)?,
        })
    }
    /// The recorded size.
    pub fn encoded_bytes(&self) -> io::Result<usize> {
        self.dlc
            .iter()
            .chain(&self.mods)
            .try_fold(8, |size, id| allocation::add(size, string_bytes(id)?))
    }
    pub fn validate(&self) -> io::Result<()> {
        validate_ids(self.dlc.iter().map(String::as_str))?;
        validate_ids(self.mods.iter().map(String::as_str))
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
    /// The recorded size.
    pub fn encoded_bytes(&self) -> io::Result<usize> {
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
}

impl SessionStart {
    pub fn encoded_bytes(&self) -> io::Result<usize> {
        allocation::add(30, self.identity.encoded_bytes()?)
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            id: self.id,
            process_id: self.process_id,
            process_start_time: self.process_start_time,
            qpc_frequency: self.qpc_frequency,
            sample_boundary: self.sample_boundary,
            capture_mode: self.capture_mode,
            identity: self.identity.try_clone()?,
        })
    }
    pub fn validate(&self) -> io::Result<()> {
        if self.qpc_frequency == 0 {
            return Err(invalid("session QPC frequency is zero"));
        }
        self.identity.validate()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures {
    use super::*;

    /// A valid identity with two DLC, a workshop and a local mod, and no
    /// verification.
    pub fn identity() -> Identity {
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
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::identity as fixture;
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
}
