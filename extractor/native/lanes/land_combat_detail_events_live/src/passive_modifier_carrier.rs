use std::fmt;

pub const MODIFIER_IDENTITY: i64 = 100_000;

pub const UNIT_OFF_MODIFIER_DATA: usize = 0x170;
pub const UNIT_OFF_MODIFIER_CAPACITY: usize = 0x178;
pub const UNIT_OFF_MODIFIER_COUNT: usize = 0x17C;
pub const UNIT_OFF_FOLD_ATTACK: usize = 0x188;
pub const UNIT_OFF_FOLD_DEFENCE: usize = 0x190;
pub const UNIT_OFF_SCALER_ATTACK: usize = 0x198;
pub const UNIT_OFF_SCALER_DEFENCE: usize = 0x1A0;
pub const MODIFIER_ENTRY_STRIDE: usize = 0x18;
pub const MODIFIER_ENTRY_OFF_VALUE_A: usize = 0x08;
pub const MODIFIER_ENTRY_OFF_VALUE_B: usize = 0x10;

pub const SIDE_OFF_COMBAT_LOG: usize = 0x1B0;
pub const LOG_OFF_LEADER_DATA: usize = 0x20;
pub const LOG_OFF_LEADER_CAPACITY: usize = 0x28;
pub const LOG_OFF_LEADER_COUNT: usize = 0x2C;
pub const LEADER_ENTRY_STRIDE: usize = 0x0C;

pub const SIDE_OFF_LEADER_DATA: usize = SIDE_OFF_COMBAT_LOG + LOG_OFF_LEADER_DATA;
pub const SIDE_OFF_LEADER_CAPACITY: usize = SIDE_OFF_COMBAT_LOG + LOG_OFF_LEADER_CAPACITY;
pub const SIDE_OFF_LEADER_COUNT: usize = SIDE_OFF_COMBAT_LOG + LOG_OFF_LEADER_COUNT;

pub const SIDE_OFF_UNIT_DATA: usize = 0x20;
pub const SIDE_OFF_UNIT_COUNT: usize = 0x2C;

pub const MAX_SIDE_UNITS: u32 = 128;
pub const MAX_MODIFIER_ENTRIES: u32 = 512;
pub const MAX_LEADER_ENTRIES: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CarrierRefusal {
    SideNotHeapPointer,
    UnitVectorHeaderUnreadable,
    UnitVectorHeaderOutOfRange,
    UnitVectorDisagreesWithCensus,
    UnitVectorDataNotHeapPointer,
    UnitVectorBodyUnreadable,
    UnitVectorHeaderChangedDuringRead,
    UnitElementNotHeapPointer,
    UnitIdentityInvalid,
    ModifierHeaderUnreadable,
    ModifierHeaderOutOfRange,
    ModifierDataNotHeapPointer,
    ModifierBodyUnreadable,
    ModifierHeaderChangedDuringRead,
    FoldsUnreadable,
    LeaderHeaderUnreadable,
    LeaderHeaderOutOfRange,
    LeaderDataNotHeapPointer,
    LeaderBodyUnreadable,
    LeaderHeaderChangedDuringRead,
    LeaderEntryHandleInvalid,
    CarrierRowBudgetExceeded,
}

impl CarrierRefusal {
    pub fn wire_code(self) -> u32 {
        match self {
            CarrierRefusal::SideNotHeapPointer => 1,
            CarrierRefusal::UnitVectorHeaderUnreadable => 2,
            CarrierRefusal::UnitVectorHeaderOutOfRange => 3,
            CarrierRefusal::UnitVectorDataNotHeapPointer => 4,
            CarrierRefusal::UnitVectorBodyUnreadable => 5,
            CarrierRefusal::UnitVectorHeaderChangedDuringRead => 6,
            CarrierRefusal::UnitElementNotHeapPointer => 7,
            CarrierRefusal::UnitIdentityInvalid => 8,
            CarrierRefusal::ModifierHeaderUnreadable => 9,
            CarrierRefusal::ModifierHeaderOutOfRange => 10,
            CarrierRefusal::ModifierDataNotHeapPointer => 11,
            CarrierRefusal::ModifierBodyUnreadable => 12,
            CarrierRefusal::ModifierHeaderChangedDuringRead => 13,
            CarrierRefusal::FoldsUnreadable => 14,
            CarrierRefusal::LeaderHeaderUnreadable => 15,
            CarrierRefusal::LeaderHeaderOutOfRange => 16,
            CarrierRefusal::LeaderDataNotHeapPointer => 17,
            CarrierRefusal::LeaderBodyUnreadable => 18,
            CarrierRefusal::LeaderHeaderChangedDuringRead => 19,
            CarrierRefusal::LeaderEntryHandleInvalid => 20,
            CarrierRefusal::CarrierRowBudgetExceeded => 21,
            CarrierRefusal::UnitVectorDisagreesWithCensus => 22,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CarrierRefusal::SideNotHeapPointer => "side_not_heap_pointer",
            CarrierRefusal::UnitVectorHeaderUnreadable => "unit_vector_header_unreadable",
            CarrierRefusal::UnitVectorHeaderOutOfRange => "unit_vector_header_out_of_range",
            CarrierRefusal::UnitVectorDataNotHeapPointer => "unit_vector_data_not_heap_pointer",
            CarrierRefusal::UnitVectorBodyUnreadable => "unit_vector_body_unreadable",
            CarrierRefusal::UnitVectorHeaderChangedDuringRead => {
                "unit_vector_header_changed_during_read"
            }
            CarrierRefusal::UnitElementNotHeapPointer => "unit_element_not_heap_pointer",
            CarrierRefusal::UnitIdentityInvalid => "unit_identity_invalid",
            CarrierRefusal::ModifierHeaderUnreadable => "modifier_header_unreadable",
            CarrierRefusal::ModifierHeaderOutOfRange => "modifier_header_out_of_range",
            CarrierRefusal::ModifierDataNotHeapPointer => "modifier_data_not_heap_pointer",
            CarrierRefusal::ModifierBodyUnreadable => "modifier_body_unreadable",
            CarrierRefusal::ModifierHeaderChangedDuringRead => {
                "modifier_header_changed_during_read"
            }
            CarrierRefusal::FoldsUnreadable => "folds_unreadable",
            CarrierRefusal::LeaderHeaderUnreadable => "leader_header_unreadable",
            CarrierRefusal::LeaderHeaderOutOfRange => "leader_header_out_of_range",
            CarrierRefusal::LeaderDataNotHeapPointer => "leader_data_not_heap_pointer",
            CarrierRefusal::LeaderBodyUnreadable => "leader_body_unreadable",
            CarrierRefusal::LeaderHeaderChangedDuringRead => "leader_header_changed_during_read",
            CarrierRefusal::LeaderEntryHandleInvalid => "leader_entry_handle_invalid",
            CarrierRefusal::CarrierRowBudgetExceeded => "carrier_row_budget_exceeded",
            CarrierRefusal::UnitVectorDisagreesWithCensus => "unit_vector_disagrees_with_census",
        }
    }
}

impl fmt::Display for CarrierRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModifierTickWitness {
    NoTickPhaseWitness,
    ProvenQuiescent,
}

impl ModifierTickWitness {
    pub fn wire_code(self) -> u32 {
        match self {
            ModifierTickWitness::NoTickPhaseWitness => 1,
            ModifierTickWitness::ProvenQuiescent => 2,
        }
    }

    pub fn from_wire_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(ModifierTickWitness::NoTickPhaseWitness),
            2 => Some(ModifierTickWitness::ProvenQuiescent),
            _ => None,
        }
    }

    pub fn is_provisional(self) -> bool {
        matches!(self, ModifierTickWitness::NoTickPhaseWitness)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LeaderFreshness {
    LastNonNullNoFreshnessProof,
    ProvenCurrent,
}

impl LeaderFreshness {
    pub fn wire_code(self) -> u32 {
        match self {
            LeaderFreshness::LastNonNullNoFreshnessProof => 1,
            LeaderFreshness::ProvenCurrent => 2,
        }
    }

    pub fn from_wire_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(LeaderFreshness::LastNonNullNoFreshnessProof),
            2 => Some(LeaderFreshness::ProvenCurrent),
            _ => None,
        }
    }

    pub fn is_stale_permitted(self) -> bool {
        matches!(self, LeaderFreshness::LastNonNullNoFreshnessProof)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LeaderSelection {
    NeverRecorded,
    LastNonNull { handle: u64, tally: u32 },
}

impl LeaderSelection {
    pub fn handle(self) -> Option<u64> {
        match self {
            LeaderSelection::NeverRecorded => None,
            LeaderSelection::LastNonNull { handle, .. } => Some(handle),
        }
    }

    pub fn tally(self) -> Option<u32> {
        match self {
            LeaderSelection::NeverRecorded => None,
            LeaderSelection::LastNonNull { tally, .. } => Some(tally),
        }
    }

    pub fn wire_presence(self) -> u32 {
        match self {
            LeaderSelection::NeverRecorded => 0,
            LeaderSelection::LastNonNull { .. } => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CombatModifierEntry {
    pub id: u32,
    pub value_a_raw: i64,
    pub value_b_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitModifierRead {
    pub ordinal: u32,
    pub division_id: u32,
    pub entries: Vec<CombatModifierEntry>,
    pub fold_attack_raw: i64,
    pub fold_defence_raw: i64,
    pub scaler_attack_raw: i64,
    pub scaler_defence_raw: i64,
    pub witness: ModifierTickWitness,
}

impl UnitModifierRead {
    pub fn effective_attack_raw(&self) -> i64 {
        self.scaler_attack_raw.saturating_mul(self.fold_attack_raw) / MODIFIER_IDENTITY
    }

    pub fn effective_defence_raw(&self) -> i64 {
        self.scaler_defence_raw
            .saturating_mul(self.fold_defence_raw)
            / MODIFIER_IDENTITY
    }

    pub fn is_identity(&self) -> bool {
        self.entries.is_empty()
            && self.fold_attack_raw == MODIFIER_IDENTITY
            && self.fold_defence_raw == MODIFIER_IDENTITY
            && self.scaler_attack_raw == MODIFIER_IDENTITY
            && self.scaler_defence_raw == MODIFIER_IDENTITY
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaderCacheRead {
    pub selection: LeaderSelection,
    pub entry_count: u32,
    pub freshness: LeaderFreshness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideCarrierRead {
    pub units: Vec<UnitModifierRead>,
    pub leader: LeaderCacheRead,
}

pub trait PassiveCombatMemory {
    fn read_u32(&self, addr: usize) -> Option<u32>;
    fn read_u64(&self, addr: usize) -> Option<u64>;
    fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>>;
    fn is_heap_pointer(&self, addr: usize) -> bool;
    fn division_id(&self, wrapper_ptr: usize) -> Option<u32>;

    fn read_i64(&self, addr: usize) -> Option<i64> {
        self.read_u64(addr).map(|value| value as i64)
    }
}

fn read_i64_from(slice: &[u8], offset: usize) -> Option<i64> {
    let bytes = slice.get(offset..offset + 8)?;
    Some(i64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u64_from(slice: &[u8], offset: usize) -> Option<u64> {
    let bytes = slice.get(offset..offset + 8)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32_from(slice: &[u8], offset: usize) -> Option<u32> {
    let bytes = slice.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

pub fn read_unit_modifiers<M: PassiveCombatMemory + ?Sized>(
    memory: &M,
    unit_ptr: usize,
    ordinal: u32,
    division_id: u32,
) -> Result<UnitModifierRead, CarrierRefusal> {
    let read_header = || -> Option<(usize, u32, u32)> {
        let data = memory.read_u64(unit_ptr.checked_add(UNIT_OFF_MODIFIER_DATA)?)? as usize;
        let capacity = memory.read_u32(unit_ptr.checked_add(UNIT_OFF_MODIFIER_CAPACITY)?)?;
        let count = memory.read_u32(unit_ptr.checked_add(UNIT_OFF_MODIFIER_COUNT)?)?;
        Some((data, capacity, count))
    };
    let (data, capacity, count) = read_header().ok_or(CarrierRefusal::ModifierHeaderUnreadable)?;

    if count > capacity || count > MAX_MODIFIER_ENTRIES {
        return Err(CarrierRefusal::ModifierHeaderOutOfRange);
    }

    let entries = if count == 0 {
        Vec::new()
    } else {
        if !memory.is_heap_pointer(data) {
            return Err(CarrierRefusal::ModifierDataNotHeapPointer);
        }
        let byte_len = (count as usize)
            .checked_mul(MODIFIER_ENTRY_STRIDE)
            .ok_or(CarrierRefusal::ModifierHeaderOutOfRange)?;
        let raw = memory
            .read_bytes(data, byte_len)
            .filter(|raw| raw.len() == byte_len)
            .ok_or(CarrierRefusal::ModifierBodyUnreadable)?;
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            let base = index * MODIFIER_ENTRY_STRIDE;
            let id = read_u32_from(&raw, base).ok_or(CarrierRefusal::ModifierBodyUnreadable)?;
            let value_a_raw = read_i64_from(&raw, base + MODIFIER_ENTRY_OFF_VALUE_A)
                .ok_or(CarrierRefusal::ModifierBodyUnreadable)?;
            let value_b_raw = read_i64_from(&raw, base + MODIFIER_ENTRY_OFF_VALUE_B)
                .ok_or(CarrierRefusal::ModifierBodyUnreadable)?;
            entries.push(CombatModifierEntry {
                id,
                value_a_raw,
                value_b_raw,
            });
        }
        entries
    };

    let read_fold =
        |offset: usize| -> Option<i64> { memory.read_i64(unit_ptr.checked_add(offset)?) };
    let fold_attack_raw = read_fold(UNIT_OFF_FOLD_ATTACK).ok_or(CarrierRefusal::FoldsUnreadable)?;
    let fold_defence_raw =
        read_fold(UNIT_OFF_FOLD_DEFENCE).ok_or(CarrierRefusal::FoldsUnreadable)?;
    let scaler_attack_raw =
        read_fold(UNIT_OFF_SCALER_ATTACK).ok_or(CarrierRefusal::FoldsUnreadable)?;
    let scaler_defence_raw =
        read_fold(UNIT_OFF_SCALER_DEFENCE).ok_or(CarrierRefusal::FoldsUnreadable)?;

    if read_header() != Some((data, capacity, count)) {
        return Err(CarrierRefusal::ModifierHeaderChangedDuringRead);
    }

    Ok(UnitModifierRead {
        ordinal,
        division_id,
        entries,
        fold_attack_raw,
        fold_defence_raw,
        scaler_attack_raw,
        scaler_defence_raw,
        witness: ModifierTickWitness::NoTickPhaseWitness,
    })
}

pub fn read_side_leader<M: PassiveCombatMemory + ?Sized>(
    memory: &M,
    side_ptr: usize,
) -> Result<LeaderCacheRead, CarrierRefusal> {
    let read_header = || -> Option<(usize, u32, u32)> {
        let data = memory.read_u64(side_ptr.checked_add(SIDE_OFF_LEADER_DATA)?)? as usize;
        let capacity = memory.read_u32(side_ptr.checked_add(SIDE_OFF_LEADER_CAPACITY)?)?;
        let count = memory.read_u32(side_ptr.checked_add(SIDE_OFF_LEADER_COUNT)?)?;
        Some((data, capacity, count))
    };
    let (data, capacity, count) = read_header().ok_or(CarrierRefusal::LeaderHeaderUnreadable)?;
    if count > capacity || count > MAX_LEADER_ENTRIES {
        return Err(CarrierRefusal::LeaderHeaderOutOfRange);
    }

    let selection = if count == 0 {
        LeaderSelection::NeverRecorded
    } else {
        if !memory.is_heap_pointer(data) {
            return Err(CarrierRefusal::LeaderDataNotHeapPointer);
        }
        let raw = memory
            .read_bytes(data, LEADER_ENTRY_STRIDE)
            .filter(|raw| raw.len() == LEADER_ENTRY_STRIDE)
            .ok_or(CarrierRefusal::LeaderBodyUnreadable)?;
        let handle = read_u64_from(&raw, 0).ok_or(CarrierRefusal::LeaderBodyUnreadable)?;
        let tally = read_u32_from(&raw, 8).ok_or(CarrierRefusal::LeaderBodyUnreadable)?;
        if handle == 0 {
            return Err(CarrierRefusal::LeaderEntryHandleInvalid);
        }
        LeaderSelection::LastNonNull { handle, tally }
    };

    if read_header() != Some((data, capacity, count)) {
        return Err(CarrierRefusal::LeaderHeaderChangedDuringRead);
    }

    Ok(LeaderCacheRead {
        selection,
        entry_count: count,
        freshness: LeaderFreshness::LastNonNullNoFreshnessProof,
    })
}

pub fn read_side_modifiers<M: PassiveCombatMemory + ?Sized>(
    memory: &M,
    side_ptr: usize,
    census: &[u32],
) -> Result<SideCarrierRead, CarrierRefusal> {
    if !memory.is_heap_pointer(side_ptr) {
        return Err(CarrierRefusal::SideNotHeapPointer);
    }
    if census.len() > MAX_SIDE_UNITS as usize {
        return Err(CarrierRefusal::UnitVectorHeaderOutOfRange);
    }
    let read_header = || -> Option<(usize, u32)> {
        let data = memory.read_u64(side_ptr.checked_add(SIDE_OFF_UNIT_DATA)?)? as usize;
        let count = memory.read_u32(side_ptr.checked_add(SIDE_OFF_UNIT_COUNT)?)?;
        Some((data, count))
    };
    let (data, count) = read_header().ok_or(CarrierRefusal::UnitVectorHeaderUnreadable)?;
    if count as usize != census.len() {
        return Err(CarrierRefusal::UnitVectorDisagreesWithCensus);
    }

    let mut units = Vec::new();
    if count > 0 {
        if !memory.is_heap_pointer(data) {
            return Err(CarrierRefusal::UnitVectorDataNotHeapPointer);
        }
        let byte_len = (count as usize)
            .checked_mul(8)
            .ok_or(CarrierRefusal::UnitVectorHeaderOutOfRange)?;
        let raw = memory
            .read_bytes(data, byte_len)
            .filter(|raw| raw.len() == byte_len)
            .ok_or(CarrierRefusal::UnitVectorBodyUnreadable)?;
        for index in 0..count as usize {
            let unit_ptr = read_u64_from(&raw, index * 8)
                .ok_or(CarrierRefusal::UnitVectorBodyUnreadable)?
                as usize;
            if !memory.is_heap_pointer(unit_ptr) {
                return Err(CarrierRefusal::UnitElementNotHeapPointer);
            }
            let division_id = memory
                .division_id(unit_ptr)
                .ok_or(CarrierRefusal::UnitIdentityInvalid)?;
            if division_id != census[index] {
                return Err(CarrierRefusal::UnitVectorDisagreesWithCensus);
            }
            units.push(read_unit_modifiers(
                memory,
                unit_ptr,
                index as u32,
                division_id,
            )?);
        }
    }
    if read_header() != Some((data, count)) {
        return Err(CarrierRefusal::UnitVectorHeaderChangedDuringRead);
    }

    let leader = read_side_leader(memory, side_ptr)?;
    Ok(SideCarrierRead { units, leader })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitModifierRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side_wire_id: u32,
    pub ordinal: u32,
    pub division_id: u32,
    pub entry_count: u32,
    pub fold_attack_raw: i64,
    pub fold_defence_raw: i64,
    pub scaler_attack_raw: i64,
    pub scaler_defence_raw: i64,
    pub witness_code: u32,
}

pub const UNIT_MODIFIER_ROW_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierEntryRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side_wire_id: u32,
    pub unit_ordinal: u32,
    pub entry_index: u32,
    pub modifier_id: u32,
    pub value_a_raw: i64,
    pub value_b_raw: i64,
}

pub const MODIFIER_ENTRY_ROW_LEN: usize = 44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeaderCacheRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side_wire_id: u32,
    pub selection_present: u32,
    pub leader_handle: u64,
    pub tally: u32,
    pub entry_count: u32,
    pub freshness_code: u32,
}

pub const LEADER_CACHE_ROW_LEN: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarrierRefusalRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side_wire_id: u32,
    pub refusal_code: u32,
}

pub const CARRIER_REFUSAL_ROW_LEN: usize = 20;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarrierRows {
    pub unit_modifier_rows: Vec<UnitModifierRow>,
    pub modifier_entry_rows: Vec<ModifierEntryRow>,
    pub leader_cache_rows: Vec<LeaderCacheRow>,
    pub refusal_rows: Vec<CarrierRefusalRow>,
}

impl CarrierRows {
    pub fn is_empty(&self) -> bool {
        self.unit_modifier_rows.is_empty()
            && self.modifier_entry_rows.is_empty()
            && self.leader_cache_rows.is_empty()
            && self.refusal_rows.is_empty()
    }

    fn would_exceed_budget(&self, read: &SideCarrierRead) -> bool {
        let entries: usize = read.units.iter().map(|unit| unit.entries.len()).sum();
        self.unit_modifier_rows
            .len()
            .saturating_add(read.units.len())
            > MAX_UNIT_MODIFIER_ROWS
            || self.modifier_entry_rows.len().saturating_add(entries) > MAX_MODIFIER_ENTRY_ROWS
            || self.leader_cache_rows.len().saturating_add(1) > MAX_LEADER_CACHE_ROWS
    }

    pub fn push_side(
        &mut self,
        combat_id: u32,
        generation: u64,
        side_wire_id: u32,
        read: &SideCarrierRead,
    ) {
        if self.would_exceed_budget(read) {
            self.push_refusal(
                combat_id,
                generation,
                side_wire_id,
                CarrierRefusal::CarrierRowBudgetExceeded,
            );
            return;
        }
        for unit in &read.units {
            self.unit_modifier_rows.push(UnitModifierRow {
                combat_id,
                generation,
                side_wire_id,
                ordinal: unit.ordinal,
                division_id: unit.division_id,
                entry_count: unit.entries.len().min(u32::MAX as usize) as u32,
                fold_attack_raw: unit.fold_attack_raw,
                fold_defence_raw: unit.fold_defence_raw,
                scaler_attack_raw: unit.scaler_attack_raw,
                scaler_defence_raw: unit.scaler_defence_raw,
                witness_code: unit.witness.wire_code(),
            });
            for (entry_index, entry) in unit.entries.iter().enumerate() {
                self.modifier_entry_rows.push(ModifierEntryRow {
                    combat_id,
                    generation,
                    side_wire_id,
                    unit_ordinal: unit.ordinal,
                    entry_index: entry_index as u32,
                    modifier_id: entry.id,
                    value_a_raw: entry.value_a_raw,
                    value_b_raw: entry.value_b_raw,
                });
            }
        }
        self.leader_cache_rows.push(LeaderCacheRow {
            combat_id,
            generation,
            side_wire_id,
            selection_present: read.leader.selection.wire_presence(),
            leader_handle: read.leader.selection.handle().unwrap_or(0),
            tally: read.leader.selection.tally().unwrap_or(0),
            entry_count: read.leader.entry_count,
            freshness_code: read.leader.freshness.wire_code(),
        });
    }

    pub fn push_refusal(
        &mut self,
        combat_id: u32,
        generation: u64,
        side_wire_id: u32,
        refusal: CarrierRefusal,
    ) {
        if self.refusal_rows.len() >= MAX_CARRIER_REFUSAL_ROWS {
            return;
        }
        self.refusal_rows.push(CarrierRefusalRow {
            combat_id,
            generation,
            side_wire_id,
            refusal_code: refusal.wire_code(),
        });
    }
}

pub fn validate_unit_modifier_row(row: &UnitModifierRow) -> Result<(), String> {
    if row.combat_id == 0 || row.generation == 0 {
        return Err("unit modifier row has no combat identity".into());
    }
    if row.division_id == 0 {
        return Err("unit modifier row has no division identity".into());
    }
    if row.witness_code != ModifierTickWitness::NoTickPhaseWitness.wire_code() {
        return Err(format!(
            "unit modifier row carries no admissible tick-phase witness (code {})",
            row.witness_code
        ));
    }
    Ok(())
}

pub fn validate_leader_cache_row(row: &LeaderCacheRow) -> Result<(), String> {
    if row.combat_id == 0 || row.generation == 0 {
        return Err("leader cache row has no combat identity".into());
    }
    if row.freshness_code != LeaderFreshness::LastNonNullNoFreshnessProof.wire_code() {
        return Err(format!(
            "leader cache row carries no admissible freshness typing (code {})",
            row.freshness_code
        ));
    }
    if row.selection_present == 0 && (row.leader_handle != 0 || row.tally != 0) {
        return Err("leader cache row marks no selection but carries one".into());
    }
    if row.selection_present == 0 && row.entry_count != 0 {
        return Err("leader cache row marks no selection but counts entries".into());
    }
    if row.selection_present != 0 && row.entry_count == 0 {
        return Err("leader cache row carries a selection with no entries".into());
    }
    if row.selection_present != 0 && row.leader_handle == 0 {
        return Err("leader cache row presents a zero leader handle".into());
    }
    if row.selection_present > 1 {
        return Err("leader cache row presence flag out of range".into());
    }
    Ok(())
}

pub fn validate_carrier_refusal_row(row: &CarrierRefusalRow) -> Result<(), String> {
    if row.combat_id == 0 || row.generation == 0 {
        return Err("carrier refusal row has no combat identity".into());
    }
    if row.refusal_code == 0 {
        return Err("carrier refusal row has no typed reason".into());
    }
    Ok(())
}

pub fn append_unit_modifier_row(payload: &mut Vec<u8>, row: &UnitModifierRow) {
    validate_unit_modifier_row(row).expect("invalid unit modifier row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side_wire_id.to_le_bytes());
    payload.extend_from_slice(&row.ordinal.to_le_bytes());
    payload.extend_from_slice(&row.division_id.to_le_bytes());
    payload.extend_from_slice(&row.entry_count.to_le_bytes());
    payload.extend_from_slice(&row.fold_attack_raw.to_le_bytes());
    payload.extend_from_slice(&row.fold_defence_raw.to_le_bytes());
    payload.extend_from_slice(&row.scaler_attack_raw.to_le_bytes());
    payload.extend_from_slice(&row.scaler_defence_raw.to_le_bytes());
    payload.extend_from_slice(&row.witness_code.to_le_bytes());
}

pub fn append_modifier_entry_row(payload: &mut Vec<u8>, row: &ModifierEntryRow) {
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side_wire_id.to_le_bytes());
    payload.extend_from_slice(&row.unit_ordinal.to_le_bytes());
    payload.extend_from_slice(&row.entry_index.to_le_bytes());
    payload.extend_from_slice(&row.modifier_id.to_le_bytes());
    payload.extend_from_slice(&row.value_a_raw.to_le_bytes());
    payload.extend_from_slice(&row.value_b_raw.to_le_bytes());
}

pub fn append_leader_cache_row(payload: &mut Vec<u8>, row: &LeaderCacheRow) {
    validate_leader_cache_row(row).expect("invalid leader cache row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side_wire_id.to_le_bytes());
    payload.extend_from_slice(&row.selection_present.to_le_bytes());
    payload.extend_from_slice(&row.leader_handle.to_le_bytes());
    payload.extend_from_slice(&row.tally.to_le_bytes());
    payload.extend_from_slice(&row.entry_count.to_le_bytes());
    payload.extend_from_slice(&row.freshness_code.to_le_bytes());
}

pub fn append_carrier_refusal_row(payload: &mut Vec<u8>, row: &CarrierRefusalRow) {
    validate_carrier_refusal_row(row).expect("invalid carrier refusal row");
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side_wire_id.to_le_bytes());
    payload.extend_from_slice(&row.refusal_code.to_le_bytes());
}

fn slice_u32(payload: &[u8], offset: usize) -> u32 {
    read_u32_from(payload, offset).unwrap_or(0)
}

fn slice_u64(payload: &[u8], offset: usize) -> u64 {
    read_u64_from(payload, offset).unwrap_or(0)
}

fn slice_i64(payload: &[u8], offset: usize) -> i64 {
    read_i64_from(payload, offset).unwrap_or(0)
}

pub fn decode_unit_modifier_row(payload: &[u8], base: usize) -> UnitModifierRow {
    UnitModifierRow {
        combat_id: slice_u32(payload, base),
        generation: slice_u64(payload, base + 4),
        side_wire_id: slice_u32(payload, base + 12),
        ordinal: slice_u32(payload, base + 16),
        division_id: slice_u32(payload, base + 20),
        entry_count: slice_u32(payload, base + 24),
        fold_attack_raw: slice_i64(payload, base + 28),
        fold_defence_raw: slice_i64(payload, base + 36),
        scaler_attack_raw: slice_i64(payload, base + 44),
        scaler_defence_raw: slice_i64(payload, base + 52),
        witness_code: slice_u32(payload, base + 60),
    }
}

pub fn decode_modifier_entry_row(payload: &[u8], base: usize) -> ModifierEntryRow {
    ModifierEntryRow {
        combat_id: slice_u32(payload, base),
        generation: slice_u64(payload, base + 4),
        side_wire_id: slice_u32(payload, base + 12),
        unit_ordinal: slice_u32(payload, base + 16),
        entry_index: slice_u32(payload, base + 20),
        modifier_id: slice_u32(payload, base + 24),
        value_a_raw: slice_i64(payload, base + 28),
        value_b_raw: slice_i64(payload, base + 36),
    }
}

pub fn decode_leader_cache_row(payload: &[u8], base: usize) -> LeaderCacheRow {
    LeaderCacheRow {
        combat_id: slice_u32(payload, base),
        generation: slice_u64(payload, base + 4),
        side_wire_id: slice_u32(payload, base + 12),
        selection_present: slice_u32(payload, base + 16),
        leader_handle: slice_u64(payload, base + 20),
        tally: slice_u32(payload, base + 28),
        entry_count: slice_u32(payload, base + 32),
        freshness_code: slice_u32(payload, base + 36),
    }
}

pub fn decode_carrier_refusal_row(payload: &[u8], base: usize) -> CarrierRefusalRow {
    CarrierRefusalRow {
        combat_id: slice_u32(payload, base),
        generation: slice_u64(payload, base + 4),
        side_wire_id: slice_u32(payload, base + 12),
        refusal_code: slice_u32(payload, base + 16),
    }
}

pub const CARRIER_TRAILER_HEADER_LEN: usize = 16;

pub const MAX_UNIT_MODIFIER_ROWS: usize = 64_000;
pub const MAX_MODIFIER_ENTRY_ROWS: usize = 512_000;
pub const MAX_LEADER_CACHE_ROWS: usize = 16_000;
pub const MAX_CARRIER_REFUSAL_ROWS: usize = 16_000;

pub fn validate_carrier_rows(rows: &CarrierRows) -> Result<(), String> {
    if rows.unit_modifier_rows.len() > MAX_UNIT_MODIFIER_ROWS {
        return Err(format!(
            "carrier unit rows {} exceed maximum {MAX_UNIT_MODIFIER_ROWS}",
            rows.unit_modifier_rows.len()
        ));
    }
    if rows.modifier_entry_rows.len() > MAX_MODIFIER_ENTRY_ROWS {
        return Err(format!(
            "carrier entry rows {} exceed maximum {MAX_MODIFIER_ENTRY_ROWS}",
            rows.modifier_entry_rows.len()
        ));
    }
    if rows.leader_cache_rows.len() > MAX_LEADER_CACHE_ROWS {
        return Err(format!(
            "carrier leader rows {} exceed maximum {MAX_LEADER_CACHE_ROWS}",
            rows.leader_cache_rows.len()
        ));
    }
    if rows.refusal_rows.len() > MAX_CARRIER_REFUSAL_ROWS {
        return Err(format!(
            "carrier refusal rows {} exceed maximum {MAX_CARRIER_REFUSAL_ROWS}",
            rows.refusal_rows.len()
        ));
    }
    Ok(())
}

pub fn append_carrier_trailer(payload: &mut Vec<u8>, rows: &CarrierRows) {
    validate_carrier_rows(rows).expect("carrier rows exceed the v9 trailer budget");
    payload.reserve(carrier_trailer_len(rows));
    payload.extend_from_slice(
        &(rows.unit_modifier_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(rows.modifier_entry_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload.extend_from_slice(
        &(rows.leader_cache_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    payload
        .extend_from_slice(&(rows.refusal_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for row in &rows.unit_modifier_rows {
        append_unit_modifier_row(payload, row);
    }
    for row in &rows.modifier_entry_rows {
        append_modifier_entry_row(payload, row);
    }
    for row in &rows.leader_cache_rows {
        append_leader_cache_row(payload, row);
    }
    for row in &rows.refusal_rows {
        append_carrier_refusal_row(payload, row);
    }
}

pub fn carrier_trailer_len(rows: &CarrierRows) -> usize {
    CARRIER_TRAILER_HEADER_LEN
        + rows.unit_modifier_rows.len() * UNIT_MODIFIER_ROW_LEN
        + rows.modifier_entry_rows.len() * MODIFIER_ENTRY_ROW_LEN
        + rows.leader_cache_rows.len() * LEADER_CACHE_ROW_LEN
        + rows.refusal_rows.len() * CARRIER_REFUSAL_ROW_LEN
}

pub fn decode_carrier_trailer(
    payload: &[u8],
    cursor: usize,
) -> Result<(CarrierRows, usize), String> {
    if cursor.saturating_add(CARRIER_TRAILER_HEADER_LEN) > payload.len() {
        return Err("carrier trailer header overruns payload".into());
    }
    let unit_count = slice_u32(payload, cursor) as usize;
    let entry_count = slice_u32(payload, cursor + 4) as usize;
    let leader_count = slice_u32(payload, cursor + 8) as usize;
    let refusal_count = slice_u32(payload, cursor + 12) as usize;
    if unit_count > MAX_UNIT_MODIFIER_ROWS
        || entry_count > MAX_MODIFIER_ENTRY_ROWS
        || leader_count > MAX_LEADER_CACHE_ROWS
        || refusal_count > MAX_CARRIER_REFUSAL_ROWS
    {
        return Err("carrier trailer row count exceeds maximum".into());
    }
    let body_len = unit_count * UNIT_MODIFIER_ROW_LEN
        + entry_count * MODIFIER_ENTRY_ROW_LEN
        + leader_count * LEADER_CACHE_ROW_LEN
        + refusal_count * CARRIER_REFUSAL_ROW_LEN;
    let mut at = cursor + CARRIER_TRAILER_HEADER_LEN;
    if at.saturating_add(body_len) > payload.len() {
        return Err("carrier trailer body overruns payload".into());
    }

    let mut rows = CarrierRows::default();
    for _ in 0..unit_count {
        let row = decode_unit_modifier_row(payload, at);
        validate_unit_modifier_row(&row)?;
        rows.unit_modifier_rows.push(row);
        at += UNIT_MODIFIER_ROW_LEN;
    }
    for _ in 0..entry_count {
        rows.modifier_entry_rows
            .push(decode_modifier_entry_row(payload, at));
        at += MODIFIER_ENTRY_ROW_LEN;
    }
    for _ in 0..leader_count {
        let row = decode_leader_cache_row(payload, at);
        validate_leader_cache_row(&row)?;
        rows.leader_cache_rows.push(row);
        at += LEADER_CACHE_ROW_LEN;
    }
    for _ in 0..refusal_count {
        let row = decode_carrier_refusal_row(payload, at);
        validate_carrier_refusal_row(&row)?;
        rows.refusal_rows.push(row);
        at += CARRIER_REFUSAL_ROW_LEN;
    }
    Ok((rows, at))
}

#[cfg(test)]
mod tests;
