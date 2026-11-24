use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_ff58);
pub const REFERENCE_TYPE: usize = 0x08;
pub const REFERENCE_TYPE_VALUE: u32 = 84;
pub const ID: usize = 0x0c;
pub const PHASE: usize = 0x38;
pub const OUTCOME: usize = 0x3c;
pub const PREPARATION_HOURS: usize = 0x44;
pub const DISTANCE: usize = 0x50;
pub const OWNER_OBJECT: usize = 0x58;
pub const VARIABLES: usize = 0x60;
pub const RAID_TYPE: usize = 0x98;
pub const TARGET: usize = 0xa0;
pub const UNIT: usize = 0xc8;
pub const SOURCE: usize = 0xd8;
pub const EQUIPMENT: usize = 0x110;
pub const NUKES: usize = 0x150;
pub const DETECTED: usize = 0x190;
pub const END_DATE: usize = 0x198;
pub const END_HOUR: usize = 0x1a0;
pub const END_PERSISTENT_VTABLE: usize = 0x1a8;
pub const END_PRESENT: usize = 0x1b0;
pub const SHOW_FOR_COUNTRIES: usize = 0x1b8;
pub const SHOW_FOR_COUNTRY_STRIDE: usize = 4;
pub const COMPLETION_COUNTRY: usize = 0x1d0;
pub const PHASE_ASSEMBLING: i32 = 1;
pub const PHASE_PREPARING: i32 = 2;
pub const PHASE_PREPARED: i32 = 3;
pub const PHASE_IN_PROGRESS: i32 = 4;
pub const PHASE_ENDED: i32 = 5;
pub const OUTCOME_NONE: i32 = 0;
pub const OUTCOME_FAILURE: i32 = 1;
pub const OUTCOME_LIMITED_SUCCESS: i32 = 2;
pub const OUTCOME_SUCCESS: i32 = 3;
pub const OUTCOME_CRITICAL_SUCCESS: i32 = 4;
pub const OUTCOME_CANCELED: i32 = 5;

pub const END_DATE_VTABLE_WITHHELD: &str = "raid end date vtable withheld";
pub const END_DATE_VTABLE_MISMATCH: &str = "raid end date vtable mismatch";
pub const END_DATE_PRESENCE_INVALID: &str = "raid end date presence invalid";
pub const COMPLETION_COUNTRY_NEGATIVE: &str = "raid completion country handle negative";
pub const COMPLETION_COUNTRY_UNLINKED: &str = "raid completion country handle unlinked";
pub const COMPLETION_COUNTRY_OUT_OF_RANGE: &str = "raid completion country handle outside country table";

pub const PHASE_KEYS: [&str; 6] = [
    "RAID_INST_PHASE_NONE",
    "RAID_INST_PHASE_ASSEMBLING",
    "RAID_INST_PHASE_PREPARING",
    "RAID_INST_PHASE_PREPARED",
    "RAID_INST_PHASE_IN_PROGRESS",
    "RAID_INST_PHASE_ENDED",
];

pub const fn known_phase_key(phase: i32) -> Option<&'static str> {
    match phase {
        0..=PHASE_ENDED => Some(PHASE_KEYS[phase as usize]),
        _ => None,
    }
}

pub const fn phase_key(phase: i32) -> &'static str {
    match known_phase_key(phase) {
        Some(key) => key,
        None => PHASE_KEYS[0],
    }
}

pub const OUTCOME_KEYS: [&str; 6] = [
    "RAID_INST_OUTCOME_NONE",
    "RAID_INST_OUTCOME_FAILURE",
    "RAID_INST_OUTCOME_LIMITED_SUCCESS",
    "RAID_INST_OUTCOME_SUCCESS",
    "RAID_INST_OUTCOME_CRITICAL_SUCCESS",
    "RAID_INST_OUTCOME_CANCELED",
];

pub const fn known_outcome_key(outcome: i32) -> Option<&'static str> {
    match outcome {
        OUTCOME_NONE..=OUTCOME_CANCELED => Some(OUTCOME_KEYS[outcome as usize]),
        _ => None,
    }
}

pub const fn outcome_key(outcome: i32) -> &'static str {
    match known_outcome_key(outcome) {
        Some(key) => key,
        None => OUTCOME_KEYS[0],
    }
}

use engine_native_memory::{Memory, cgame_date, cpdx_array::Array};
use engine_native::ccountry;
use crate::{ccountry_raid_status, craid_source, craid_target};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub reference_type: u32,
    pub id: u32,
    pub parent_slot: u32,
    pub ordinal: u32,
    pub phase: i32,
    pub outcome: i32,
    pub source: craid_source::Row,
    pub unit: u64,
    pub air_wing: u64,
    pub target: craid_target::Row,
    pub raid_type: u32,
    pub detected: u8,
    pub show_for_start: usize,
    pub show_for_len: usize,
    pub travelled: i64,
    pub preparation_hours: i32,
    pub end_hour: Option<Option<i32>>,
    pub end_hour_refusal: Option<&'static str>,
    pub completion_country: Option<Option<u32>>,
    pub completion_country_refusal: Option<&'static str>,
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.reference_type) << 32) | u64::from(self.id)
    }
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    parent_slot: u32,
    ordinal: u32,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<(Row, Array), &'static str> {
    let mut identity = [0_u8; 16];
    let mut state = [0_u8; 16];
    let mut definition = [0_u8; 8];
    let mut tail = [0_u8; COMPLETION_COUNTRY + 4 - DETECTED];
    let mut unit = [0_u8; 16];
    let mut travelled = [0_u8; 8];
    let field = |offset| {
        address
            .checked_add(offset)
            .ok_or("raid instance address overflow")
    };
    if memory.copy_scattered(
        [
            address,
            field(PHASE)?,
            field(RAID_TYPE)?,
            field(DETECTED)?,
            field(UNIT)?,
            field(DISTANCE)?,
        ],
        &mut [
            &mut identity,
            &mut state,
            &mut definition,
            &mut tail,
            &mut unit,
            &mut travelled,
        ],
    ) != 6
    {
        return Err("raid instance unreadable");
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("raid instance vtable mismatch");
    }
    let reference_type = u32::from_le_bytes(identity[8..12].try_into().unwrap());
    if reference_type != REFERENCE_TYPE_VALUE {
        return Err("raid instance reference type mismatch");
    }
    let id = u32::from_le_bytes(identity[12..].try_into().unwrap());
    if id == 0 {
        return Err("raid instance reference id is zero");
    }
    let phase = i32::from_le_bytes(state[..4].try_into().unwrap());
    let outcome = i32::from_le_bytes(state[4..8].try_into().unwrap());
    let preparation_hours =
        i32::from_le_bytes(state[PREPARATION_HOURS - PHASE..][..4].try_into().unwrap());
    if !(PHASE_ASSEMBLING..=PHASE_ENDED).contains(&phase) {
        return Err("raid instance phase is invalid");
    }
    if !(OUTCOME_NONE..=OUTCOME_CANCELED).contains(&outcome) {
        return Err("raid instance outcome is invalid");
    }
    let detected = tail[0];
    if detected > 1 {
        return Err("raid detected flag is invalid");
    }
    let show_for: [u8; 16] = tail[SHOW_FOR_COUNTRIES - DETECTED..][..16]
        .try_into()
        .unwrap();
    let (end_hour, end_hour_refusal) = match (
        engine_sites::at(base, cgame_date::VTABLE_RVA),
        engine_sites::at(base, cgame_date::PERSISTENT_VTABLE_RVA),
    ) {
        (Some(primary), Some(persistent)) => match tail[END_PRESENT - DETECTED] {
            0 => (Some(None), None),
            1 => {
                let got_primary =
                    u64::from_le_bytes(tail[END_DATE - DETECTED..][..8].try_into().unwrap());
                let got_persistent = u64::from_le_bytes(
                    tail[END_PERSISTENT_VTABLE - DETECTED..][..8]
                        .try_into()
                        .unwrap(),
                );
                if got_primary == primary as u64 && got_persistent == persistent as u64 {
                    let hour =
                        i32::from_le_bytes(tail[END_HOUR - DETECTED..][..4].try_into().unwrap());
                    (Some(Some(hour)), None)
                } else {
                    (None, Some(END_DATE_VTABLE_MISMATCH))
                }
            }
            _ => (None, Some(END_DATE_PRESENCE_INVALID)),
        },
        _ => (None, Some(END_DATE_VTABLE_WITHHELD)),
    };
    let raw_victim = i32::from_le_bytes(
        tail[COMPLETION_COUNTRY - DETECTED..][..4]
            .try_into()
            .unwrap(),
    );
    let (completion_country, completion_country_refusal) = if raw_victim == 0 {
        (Some(None), None)
    } else if raw_victim < 0 {
        (None, Some(COMPLETION_COUNTRY_NEGATIVE))
    } else {
        match countries.link(raw_victim as u32) {
            Ok(Some(_)) => (Some(Some(raw_victim as u32)), None),
            Ok(None) => (None, Some(COMPLETION_COUNTRY_UNLINKED)),
            Err(_) => (None, Some(COMPLETION_COUNTRY_OUT_OF_RANGE)),
        }
    };
    let definition = usize::try_from(u64::from_le_bytes(definition))
        .map_err(|_| "raid type definition address overflow")?;
    let raid_type = if definition == 0 {
        0
    } else {
        ccountry_raid_status::type_token(memory, base, definition)?
    };
    let show_for = Array::from_header(show_for, SHOW_FOR_COUNTRY_STRIDE)
        .ok_or("raid show-for array invalid")?;
    Ok((
        Row {
            target: craid_target::read_row(memory, base, field(TARGET)?)?,
            raid_type,
            detected,
            unit: u64::from_le_bytes(unit[..8].try_into().unwrap()),
            air_wing: u64::from_le_bytes(unit[8..].try_into().unwrap()),
            reference_type,
            id,
            parent_slot,
            ordinal,
            phase,
            outcome,
            source: craid_source::read_row(memory, base, field(SOURCE)?, countries)?,
            show_for_start: 0,
            show_for_len: show_for.count as usize,
            travelled: i64::from_le_bytes(travelled),
            preparation_hours,
            end_hour,
            end_hour_refusal,
            completion_country,
            completion_country_refusal,
        },
        show_for,
    ))
}
