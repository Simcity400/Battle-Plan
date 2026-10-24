//! Persistent raid instance operands. Identity, phase, outcome, preparation
//! and source use guarded readers; layout alone establishes neither hourly
//! consistency nor a new observation boundary.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0298_3a78;
/// CRaidInstance u32 reference type.
pub const REFERENCE_TYPE: usize = 0x08;
/// Raid-instance reference type identifier.
pub const REFERENCE_TYPE_VALUE: u32 = 84;
/// CRaidInstance u32 engine reference id.
pub const ID: usize = 0x0c;
/// Phase enum, i32.
pub const PHASE: usize = 0x38;
/// Outcome enum, i32.
pub const OUTCOME: usize = 0x3c;
/// Preparation hours, i32, incremented and queried in phase 2 and subtracted
/// from remaining hours. Starting preparation clears it; serialization emits
/// `prep_time` only when nonzero.
pub const PREPARATION_HOURS: usize = 0x44;
/// Distance travelled, signed i64 fixed5, increased by raid speed each
/// phase-4 hour. Elapsed hours divide it by that speed; serialization emits
/// `distance` only when nonzero.
pub const DISTANCE: usize = 0x50;
/// Owner-object pointer; country handle is in the owner object, class unresolved.
pub const OWNER_OBJECT: usize = 0x58;
/// Embedded variables object.
pub const VARIABLES: usize = 0x60;
/// Raid-type definition pointer serialized by database name.
pub const RAID_TYPE: usize = 0x98;
/// Embedded target scope.
pub const TARGET: usize = 0xa0;
/// Embedded raid unit.
pub const UNIT: usize = 0xc8;
/// Inline raid source.
pub const SOURCE: usize = 0xd8;
/// Embedded equipment object; inner layout unresolved.
pub const EQUIPMENT: usize = 0x110;
/// Nuke count, i32.
pub const NUKES: usize = 0x150;
/// Detected boolean byte, serialized only when nonzero.
pub const DETECTED: usize = 0x190;
/// Show-for country-handle array pointer.
pub const SHOW_FOR_COUNTRIES: usize = 0x1b8;
/// Show-for country-handle stride.
pub const SHOW_FOR_COUNTRY_STRIDE: usize = 4;
/// CRaidInstance phase 1 is ASSEMBLING.
pub const PHASE_ASSEMBLING: i32 = 1;
/// CRaidInstance phase 2 is PREPARING.
pub const PHASE_PREPARING: i32 = 2;
/// CRaidInstance phase 3 is PREPARED.
pub const PHASE_PREPARED: i32 = 3;
/// CRaidInstance phase 4 is IN_PROGRESS.
pub const PHASE_IN_PROGRESS: i32 = 4;
/// CRaidInstance phase 5 is ENDED.
pub const PHASE_ENDED: i32 = 5;
/// Outcome zero is absent and omitted from serialization.
pub const OUTCOME_NONE: i32 = 0;
/// CRaidInstance outcome 3 is SUCCESS.
pub const OUTCOME_SUCCESS: i32 = 3;
/// CRaidInstance outcome 4 is CRITICAL_SUCCESS.
pub const OUTCOME_CRITICAL_SUCCESS: i32 = 4;
/// CRaidInstance outcome 5 is CANCELED.
pub const OUTCOME_CANCELED: i32 = 5;

/// Phase description and progress text select these executable localisation
/// keys, falling back to the absent phase outside the enum.
pub const PHASE_KEYS: [&str; 6] = [
    "RAID_INST_PHASE_NONE",
    "RAID_INST_PHASE_ASSEMBLING",
    "RAID_INST_PHASE_PREPARING",
    "RAID_INST_PHASE_PREPARED",
    "RAID_INST_PHASE_IN_PROGRESS",
    "RAID_INST_PHASE_ENDED",
];

pub const fn phase_key(phase: i32) -> &'static str {
    match phase {
        0..=PHASE_ENDED => PHASE_KEYS[phase as usize],
        _ => PHASE_KEYS[0],
    }
}

/// Feedback tooltip and outcome text select these executable localisation
/// keys, falling back to absent outcome outside the enum.
pub const OUTCOME_KEYS: [&str; 6] = [
    "RAID_INST_OUTCOME_NONE",
    "RAID_INST_OUTCOME_FAILURE",
    "RAID_INST_OUTCOME_LIMITED_SUCCESS",
    "RAID_INST_OUTCOME_SUCCESS",
    "RAID_INST_OUTCOME_CRITICAL_SUCCESS",
    "RAID_INST_OUTCOME_CANCELED",
];

pub const fn outcome_key(outcome: i32) -> &'static str {
    match outcome {
        OUTCOME_NONE..=OUTCOME_CANCELED => OUTCOME_KEYS[outcome as usize],
        _ => OUTCOME_KEYS[0],
    }
}

use crate::{
    Memory, ccountry, ccountry_raid_status, cpdx_array::Array, craid_source, craid_target,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub reference_type: u32,
    pub id: u32,
    pub parent_slot: u32,
    pub ordinal: u32,
    pub phase: i32,
    pub outcome: i32,
    pub source: craid_source::Row,
    /// [`UNIT`] holds assigned army and air-wing references. Assignment writes
    /// the units' reciprocal raid references and ending clears both, so these
    /// identify units held by a live raid.
    pub unit: u64,
    pub air_wing: u64,
    /// What the raid is aimed at, from the scope at [`TARGET`].
    pub target: craid_target::Row,
    /// Raid-type definition lexer token, zero for a null definition.
    /// Serialization writes its string as `type`; `LexicalTokens` names it.
    pub raid_type: u32,
    /// [`DETECTED`] controls the incoming-raid detection icon.
    pub detected: u8,
    /// [`SHOW_FOR_COUNTRIES`] in the caller's handle arena.
    pub show_for_start: usize,
    pub show_for_len: usize,
    /// [`DISTANCE`], fixed5, read in every phase.
    pub travelled: i64,
    /// [`PREPARATION_HOURS`].
    pub preparation_hours: i32,
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.reference_type) << 32) | u64::from(self.id)
    }
}

/// One instance and its `show_for` array, which the caller copies.
pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    parent_slot: u32,
    ordinal: u32,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<(Row, Array), &'static str> {
    let mut identity = [0_u8; 16];
    // Phase, outcome, risk and preparation hours.
    let mut state = [0_u8; 16];
    let mut definition = [0_u8; 8];
    let mut detected = [0_u8; 1];
    let mut show_for = [0_u8; 16];
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
            field(SHOW_FOR_COUNTRIES)?,
            field(UNIT)?,
            field(DISTANCE)?,
        ],
        &mut [
            &mut identity,
            &mut state,
            &mut definition,
            &mut detected,
            &mut show_for,
            &mut unit,
            &mut travelled,
        ],
    ) != 7
    {
        return Err("raid instance unreadable");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
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
    if detected[0] > 1 {
        return Err("raid detected flag is invalid");
    }
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
            detected: detected[0],
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
        },
        show_for,
    ))
}
