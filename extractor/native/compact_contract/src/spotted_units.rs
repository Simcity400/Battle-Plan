//! Pointer-free exact-a729 current-POV map-spotted unit carrier (d95).

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 95;
pub const DOMAIN_NAME: &str = "pov_spotted_units_live";
pub const CARRIER_MAGIC: [u8; 4] = *b"SPU1";
pub const CARRIER_VERSION: u16 = 1;
pub const MAX_POV_COUNTRY_ID: u32 = 2_048;
pub const MAX_ROWS_PER_SURFACE: usize = 131_072;
pub const FLAG_LAND_FRAME: u16 = 1 << 0;
pub const FLAG_TASKFORCE_FRAME: u16 = 1 << 1;
const KNOWN_FLAGS: u16 = FLAG_LAND_FRAME | FLAG_TASKFORCE_FRAME;
const HEADER_LEN: usize = 44;
const ROW_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpottedUnitHandle {
    pub type_id: u32,
    pub stable_id: u32,
    pub raw_intel: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpottedUnitsSnapshot {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    pub land_frame_sequence: Option<u64>,
    pub taskforce_frame_sequence: Option<u64>,
    pub land: Vec<SpottedUnitHandle>,
    pub taskforces: Vec<SpottedUnitHandle>,
}

impl SpottedUnitsSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.capture_sequence == 0 {
            return Err("spotted-units capture sequence is zero".into());
        }
        if self.pov_country_id == 0 || self.pov_country_id > MAX_POV_COUNTRY_ID {
            return Err("spotted-units POV country ID is outside the admitted range".into());
        }
        validate_surface("land", self.land_frame_sequence, &self.land, 250)?;
        validate_surface(
            "taskforce",
            self.taskforce_frame_sequence,
            &self.taskforces,
            150,
        )?;
        if self.land_frame_sequence.is_none() && self.taskforce_frame_sequence.is_none() {
            return Err("spotted-units snapshot has no complete producer frame".into());
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut flags = 0u16;
        if self.land_frame_sequence.is_some() {
            flags |= FLAG_LAND_FRAME;
        }
        if self.taskforce_frame_sequence.is_some() {
            flags |= FLAG_TASKFORCE_FRAME;
        }
        let land_count = u32::try_from(self.land.len()).map_err(|_| "land row count overflow")?;
        let taskforce_count =
            u32::try_from(self.taskforces.len()).map_err(|_| "taskforce row count overflow")?;
        let mut out = Vec::with_capacity(
            HEADER_LEN + ROW_LEN.saturating_mul(self.land.len() + self.taskforces.len()),
        );
        out.extend_from_slice(&CARRIER_MAGIC);
        out.extend_from_slice(&CARRIER_VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&self.capture_sequence.to_le_bytes());
        out.extend_from_slice(&self.pov_country_id.to_le_bytes());
        out.extend_from_slice(&land_count.to_le_bytes());
        out.extend_from_slice(&taskforce_count.to_le_bytes());
        out.extend_from_slice(&self.land_frame_sequence.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&self.taskforce_frame_sequence.unwrap_or(0).to_le_bytes());
        for row in self.land.iter().chain(self.taskforces.iter()) {
            out.extend_from_slice(&row.type_id.to_le_bytes());
            out.extend_from_slice(&row.stable_id.to_le_bytes());
            out.push(row.raw_intel);
            out.extend_from_slice(&[0u8; 3]);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || bytes[..4] != CARRIER_MAGIC {
            return Err("spotted-units carrier magic/length mismatch".into());
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        if version != CARRIER_VERSION || flags & !KNOWN_FLAGS != 0 {
            return Err("spotted-units carrier version/flags mismatch".into());
        }
        let capture_sequence = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let pov_country_id = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let land_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        let taskforce_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
        let land_sequence = u64::from_le_bytes(bytes[28..36].try_into().unwrap());
        let taskforce_sequence = u64::from_le_bytes(bytes[36..44].try_into().unwrap());
        let row_count = land_count
            .checked_add(taskforce_count)
            .ok_or("spotted-units row count overflow")?;
        let expected = 44usize
            .checked_add(ROW_LEN.checked_mul(row_count).ok_or("spotted-units length overflow")?)
            .ok_or("spotted-units length overflow")?;
        if bytes.len() != expected {
            return Err("spotted-units carrier is truncated or has trailing bytes".into());
        }
        let mut rows = Vec::with_capacity(row_count);
        let mut cursor = 44;
        for _ in 0..row_count {
            let type_id = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            let stable_id = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap());
            let raw_intel = bytes[cursor + 8];
            if bytes[cursor + 9..cursor + 12] != [0u8; 3] {
                return Err("spotted-units row reserved bytes are nonzero".into());
            }
            rows.push(SpottedUnitHandle { type_id, stable_id, raw_intel });
            cursor += ROW_LEN;
        }
        let taskforces = rows.split_off(land_count);
        let snapshot = Self {
            capture_sequence,
            pov_country_id,
            land_frame_sequence: (flags & FLAG_LAND_FRAME != 0).then_some(land_sequence),
            taskforce_frame_sequence: (flags & FLAG_TASKFORCE_FRAME != 0)
                .then_some(taskforce_sequence),
            land: rows,
            taskforces,
        };
        if (flags & FLAG_LAND_FRAME == 0 && land_sequence != 0)
            || (flags & FLAG_TASKFORCE_FRAME == 0 && taskforce_sequence != 0)
        {
            return Err("spotted-units absent frame has a nonzero sequence".into());
        }
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn validate_surface(
    name: &str,
    sequence: Option<u64>,
    rows: &[SpottedUnitHandle],
    minimum_intel: u8,
) -> Result<(), String> {
    if rows.len() > MAX_ROWS_PER_SURFACE {
        return Err(format!("spotted-units {name} row count exceeds the admitted ceiling"));
    }
    match sequence {
        None if !rows.is_empty() => {
            return Err(format!("spotted-units {name} rows lack a complete frame"));
        }
        Some(0) => return Err(format!("spotted-units {name} frame sequence is zero")),
        _ => {}
    }
    let mut identities = BTreeSet::new();
    let mut prior = None;
    for row in rows {
        if row.type_id == 0 || row.stable_id == 0 || row.raw_intel < minimum_intel {
            return Err(format!("spotted-units {name} row is outside the admitted range"));
        }
        let key = (row.type_id, row.stable_id);
        if prior.is_some_and(|value| value >= key) || !identities.insert(key) {
            return Err(format!("spotted-units {name} rows are not unique and sorted"));
        }
        prior = Some(key);
    }
    Ok(())
}
