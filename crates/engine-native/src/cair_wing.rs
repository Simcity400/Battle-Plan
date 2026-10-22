//! CAirWing; field offsets are relative to the primary object.
//! The secondary subobject follows the primary header. Read widths alone do
//! not establish the full width of fixed-point storage.

use crate::{
    Memory, air_mission, cair_wing_pool, ccountry, cgame_state, corders_group,
    cpdx_array::Array,
    creference_registry, cstrategic_air, cstrategic_air_manager, cstrategic_region,
    memory::{RecordField, RecordLayout, RecordSources, packed_fields, packed_stride},
    msvc_string, read,
    walk::{Buffer, Outcome},
};

pub mod fuel_inputs;
pub mod replacement;
pub mod transport_operands;

/// Primary wing vtable.
pub const VTABLE_RVA: usize = 0x0296_4178;
/// Secondary wing vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0296_41b0;
/// Secondary subobject displacement.
pub const SECONDARY: usize = 0x10;
/// Registry type value.
pub const TYPE: u32 = 69;
/// Registry type field, u32.
pub const REFERENCE_TYPE: usize = 0x18;
/// Instance id, u32.
pub const ID: usize = 0x1c;
/// CAirWing CAirWingPool* (`POOL`); translated from the secondary body's copy and independently
/// used by fuel walkers.
pub const POOL: usize = 0x38;
/// Air-group paired-u32 handle.
pub const GROUP_HANDLE: usize = 0x40;
/// Plane count, i32.
pub const PLANE_COUNT: usize = 0x7c;
/// Air-wing signed i32 acquired manpower: aircraft destruction subtracts the lost crews and clamps
/// at zero.
pub const ACQUIRED_MANPOWER: usize = 0x80;
/// CAirWing cumulative signed i32 accident count; the accident update increments this word after
/// aircraft removal.
pub const AIR_ACCIDENTS: usize = 0x84;
/// Embedded mission object, with layout in air_mission.
pub const MISSION: usize = 0x90;
/// Inline equipment-bank vector.
pub const EQUIPMENT_BANK: usize = 0x1f8;
/// Embedded CEquipmentVariantPool initialized by wing construction.
pub const EQUIPMENT_POOL: usize = 0x1d8;
/// Equipment-bank capacity, u32.
#[cfg(any(test, feature = "test-support"))]
pub const EQUIPMENT_BANK_CAPACITY: usize = 0x200;
/// Equipment-bank count, u32.
pub const EQUIPMENT_BANK_COUNT: usize = 0x204;
/// Experience, signed i64; updates read, add, clamp and write the complete value.
pub const EXPERIENCE: usize = 0x218;
/// Inline wing modifier accumulator.
pub const MODIFIERS: usize = 0x238;
/// Night-operations statistic in a cached mission/default stat record.
pub const NIGHT_OPERATIONS_STAT: usize = 0x4c;
/// Base override reference resolved before falling back to the pool base.
pub const BASE_OVERRIDE: usize = 0x48;
/// Signed fixed5 distance already travelled during relocation.
pub const RELOCATION_PROGRESS: usize = 0x50;
/// Signed fixed5 total relocation distance, assigned with the destination.
pub const RELOCATION_DISTANCE: usize = 0x58;
/// Mission-admission counter, i32; admission requires it not to exceed the limit.
pub const ELIGIBILITY_COUNTER: usize = 0x60;
/// Mission-admission limit, i32.
pub const ELIGIBILITY_LIMIT: usize = 0x64;
/// Byte veto rejecting every mission while nonzero.
pub const ELIGIBILITY_VETO: usize = 0x79;
/// Assigned ace paired-u32 reference; the high word is the ace id.
pub const ACE_REFERENCE: usize = 0x2e8;
/// Base range, i64; scaled to map units before applying range modifiers.
pub const BASE_RANGE: usize = 0x500;
/// Base fuel, i64, read before applying fixed-point modifiers.
pub const BASE_FUEL: usize = 0x5b8;
/// Signed country-tag override; a positive value wins over the pool owner.
pub const COUNTRY_OVERRIDE: usize = 0x9c4;
pub const EXILE_COUNTRY: usize = 0x9c8;
/// Attached orders-group pointer matched against combat armies in tactical target scans.
pub const ATTACHED_ORDERS_GROUP: usize = 0x9b8;
/// Stored wing name string.
pub const NAME: usize = 0x998;
/// The tactical (CAS) mission bit, the only reader of the attached group here.
pub const TACTICAL_MISSION: u32 = 1 << 1;

/// Inline i64 default stat block.
pub const DEFAULT_STATS: usize = 0x390;
/// Cached mission-stat array.
pub const MISSION_STATS: usize = 0x968;
/// Signed mission-bit index map relative to MISSION_STATS.
pub const MISSION_STAT_INDEXES: usize = 0x18;
/// Per-mission stat-record size.
pub const MISSION_STAT_STRIDE: usize = 0x270;

/// Authenticated wing identity and shared score count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreLeaf {
    pub id: u64,
    pub planes: Result<i32, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameScoreWalk {
    pub shared: SharedWalk,
    pub score: Outcome,
    /// The walk stopped before the end of its owner slots: a mandatory
    /// fault, or every projection failed. The serial walk stops there too,
    /// so no later part of a census split by owner range belongs to it.
    pub ended: bool,
    /// This walk's record gathers that did not complete and pointer runs
    /// that did not read, after which the serial reads ran ([`ROW_BATCH`]).
    /// Each is also counted for status where it starts
    /// ([`crate::batch_fallback`]), whichever walk wrapper ran it.
    pub row_fallbacks: u32,
}

/// Every owner slot: the serial walk, one part covering the whole census.
pub const EVERY_OWNER: std::ops::Range<u32> = 0..u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionStatError {
    MultipleMissionBits,
    IndexUnreadable,
    AddressOverflow,
    HeaderInvalid,
    IndexNegative,
    IndexOutsideArray,
}

impl MissionStatError {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::MultipleMissionBits => "air wing mission stat requires one mission bit",
            Self::IndexUnreadable => "air wing mission stat index unreadable",
            Self::AddressOverflow => "air wing mission stat address overflow",
            Self::HeaderInvalid => "air wing mission stat header invalid",
            Self::IndexNegative => "air wing mission stat index negative",
            Self::IndexOutsideArray => "air wing mission stat index outside array",
        }
    }
}

/// The selector admits NONE or one mission bit. Only -1 selects the
/// default record; other invalid indices stay unknown.
pub fn mission_stat_index(
    memory: &impl Memory,
    wing: usize,
    mission: u32,
) -> Result<i8, MissionStatError> {
    if mission == 0 {
        return Ok(-1);
    }
    if !mission.is_power_of_two() {
        return Err(MissionStatError::MultipleMissionBits);
    }
    wing.checked_add(MISSION_STATS + MISSION_STAT_INDEXES)
        .and_then(|at| at.checked_add(mission.trailing_zeros() as usize))
        .and_then(|at| read(memory, at))
        .ok_or(MissionStatError::IndexUnreadable)
}

/// Select the cached record without recalculating it.
pub fn mission_stat_record(
    memory: &impl Memory,
    wing: usize,
    index: i8,
) -> Result<usize, MissionStatError> {
    if index == -1 {
        return wing
            .checked_add(DEFAULT_STATS)
            .ok_or(MissionStatError::AddressOverflow);
    }
    let roster = wing
        .checked_add(MISSION_STATS)
        .and_then(|at| Array::read(memory, at, MISSION_STAT_STRIDE))
        .ok_or(MissionStatError::HeaderInvalid)?;
    let index = u32::try_from(index).map_err(|_| MissionStatError::IndexNegative)?;
    roster
        .element(index, MISSION_STAT_STRIDE)
        .ok_or(MissionStatError::IndexOutsideArray)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native reference; no collector generation or pointer.
    pub id: u64,
    /// Raw country reference from the owning CStrategicAir roster.
    pub owner: u32,
    /// Already authenticated census context for regional map consumers.
    /// These pointers and the canonical slot never enter replay rows.
    pub owner_pointer: usize,
    pub owner_slot: u32,
    /// Position in the owner's roster: pools in order, then wings in each pool,
    /// starting at zero for each owner. Every wing counts even when a bounded
    /// consumer omits it. This walk already supplies the order without extra
    /// reads. Training takes its first fifty region-matching candidates in that
    /// order before drawing randomly; serialization retains the same wing order.
    pub pool_ordinal: u32,
    /// Signed country override; positive values select it, otherwise the pool owner.
    pub country_override: i32,
    /// Assigned pool base, distinct from the wing's override reference.
    pub air_base: u64,
    /// Try the base override first, then the pool base if resolution fails.
    pub base_override: u64,
    /// Signed integer counts without an imposed range restriction.
    pub planes: i32,
    /// Collector-owned certified acquired-crew observation.
    pub acquired_manpower: Option<i32>,
    pub exile_country: Option<i32>,
    pub mission_mask: u32,
    /// None means a null native region pointer, never a failed region read.
    pub mission_region: Option<u32>,
    pub region_pointer: usize,
    /// [`BASE_RANGE`], the raid range limit's wing operand.
    pub base_range_raw: i64,
    /// [`ACE_REFERENCE`], unresolved.
    pub ace: u64,
    /// Mission-admission operands: active byte, veto, counter and limit.
    pub mission_active: u8,
    pub eligibility_veto: u8,
    pub eligibility_counter: i32,
    pub eligibility_limit: i32,
    /// For a tactical-mission wing, the attached orders group's full handle
    /// authenticated in the reference registry, `Some(0)` when null. `None`
    /// for other wings, or when the group does not authenticate.
    pub attached_orders_group: Option<u64>,
    /// Already-gathered attachment pointer, joined to the same pass's group
    /// census during conversion. Acquisition context only; never serialized.
    pub attached_group_pointer: u64,
    /// Span in the caller's name arena; absent after a field-local refusal.
    pub name: Option<(usize, usize)>,
    /// Authenticated containing pool's recorded-content database index.
    /// Null subtype is Some(0); optional field read failure is None.
    pub pool_subunit: Option<u32>,
    /// Complete stored raid handle plus authenticated registry liveness. The
    /// transport pass or certified worker observation supplies this field.
    pub raid_reference: Option<transport_operands::RaidReference>,
    /// Selected tactical mission stat, None after an optional-read refusal.
    pub strategic_bombing_raw: Option<i64>,
    /// Writer-certified raw replacement preference, populated by the owning worker.
    pub replacement_setting: Option<u8>,
    /// Worker-only writer-certified Air grid flags; the synchronous fallback
    /// leaves them unknown without reading these fields.
    pub map_flags: Option<u32>,
    /// Independent simulation fuel operands, acquired only by the existing worker.
    pub fuel_inputs: Option<fuel_inputs::Inputs>,
}

/// Allocation-free name acquisition into caller-owned live-sized storage.
pub struct NameArena<'a> {
    pub bytes: &'a mut [u8],
    pub used: usize,
    pub required: usize,
    pub failures: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFailure {
    Native(crate::raid_tooltip_operands::NameReadError),
    Capacity,
    Allocation,
    Overflow,
}

impl NameFailure {
    pub const REASONS: [&'static str; 8] = [
        "air wing name native string header invalid",
        "air wing name native string data pointer null",
        "air wing name native string terminator invalid",
        "air wing name native string address or length overflow",
        "air wing name native string header or bytes unreadable",
        "air wing name arena capacity refused",
        "air wing name arena allocation failed",
        "air wing name arena demand or address overflow",
    ];
    pub const fn index(self) -> usize {
        use crate::raid_tooltip_operands::NameReadError;
        match self {
            Self::Native(NameReadError::Header) => 0,
            Self::Native(NameReadError::NullPointer) => 1,
            Self::Native(NameReadError::Terminator) => 2,
            Self::Native(NameReadError::Overflow) => 3,
            Self::Native(NameReadError::Unreadable) => 4,
            Self::Capacity => 5,
            Self::Allocation => 6,
            Self::Overflow => 7,
        }
    }
}

impl NameArena<'_> {
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        header: Option<[u8; msvc_string::STRIDE]>,
    ) -> Option<(usize, usize)> {
        match capture_name(
            memory,
            pointer,
            header,
            Buffer::Fixed(self.bytes),
            &mut self.used,
            &mut self.required,
        ) {
            Ok(span) => Some(span),
            Err(_) => {
                self.failures = self.failures.saturating_add(1);
                None
            }
        }
    }
}

/// Worker name growth uses the original retained string header and copies
/// its bytes once.
pub fn capture_name_growing(
    memory: &impl Memory,
    pointer: usize,
    header: Option<[u8; msvc_string::STRIDE]>,
    bytes: &mut Vec<u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), NameFailure> {
    capture_name(
        memory,
        pointer,
        header,
        Buffer::Growing(bytes),
        used,
        required,
    )
}

/// `header` is the census's gathered copy of the 32-byte header
/// ([`Admitted::name_header`]), the bytes the whole-header read would take;
/// without it the header is read here.
fn capture_name(
    memory: &impl Memory,
    pointer: usize,
    header: Option<[u8; msvc_string::STRIDE]>,
    mut bytes: Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), NameFailure> {
    use crate::raid_tooltip_operands::StoredString;
    let at = pointer.checked_add(NAME).ok_or(NameFailure::Overflow)?;
    let name = match header {
        Some(header) => StoredString::from_whole_header(memory, at, header),
        None => StoredString::read_whole(memory, at),
    }
    .map_err(NameFailure::Native)?;
    *required = required
        .checked_add(name.len)
        .ok_or(NameFailure::Overflow)?;
    let start = *used;
    let end = start.checked_add(name.len).ok_or(NameFailure::Overflow)?;
    bytes.ensure(end).map_err(|_| NameFailure::Allocation)?;
    let output = bytes.get_mut(start..end).ok_or(NameFailure::Capacity)?;
    name.copy(memory, output).map_err(NameFailure::Native)?;
    *used = end;
    Ok((start, name.len))
}

/// Presizing only: the shared authenticated census reads each header, no text
/// payload, using its unbounded visitor admission and no row storage.
pub fn name_demand(memory: &impl Memory, base: usize, state: usize) -> Result<usize, &'static str> {
    let mut required = Ok(0usize);
    let walked = walk_shared(
        memory,
        base,
        state,
        &mut [],
        &mut [],
        usize::MAX,
        |pointer, _, _, admitted| {
            if let Ok(bytes) = required {
                required = (|| {
                    use crate::raid_tooltip_operands::{NameReadError, StoredString};
                    let at = pointer
                        .checked_add(NAME)
                        .ok_or("air wing name address overflow")?;
                    // The header gathered with the row holds the fields the
                    // field reads would take.
                    let name = match admitted.name_header {
                        Some(header) => StoredString::from_whole_header(memory, at, header),
                        None => StoredString::read(memory, at),
                    }
                    .map_err(NameReadError::reason)?;
                    bytes
                        .checked_add(name.len)
                        .ok_or("air wing name arena demand overflow")
                })();
            }
        },
    );
    match walked.naval {
        Outcome::Rows { .. } => required,
        Outcome::Unavailable { reason } => Err(reason),
        Outcome::Truncated { .. } => Err("air wing name demand census truncated"),
    }
}

/// Owned copied row plus callback-local discovery context. Pointer values are
/// opaque after the callback and must never be dereferenced or serialized then.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub pointer: usize,
    pub pool: usize,
    pub row: Row,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedWalk {
    pub map: Outcome,
    pub sources: Outcome,
    pub naval: Outcome,
}

/// Which independently bounded consumers admitted this authenticated row.
/// One visitor invocation can populate shared extension inputs without rereads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub map: bool,
    pub sources: bool,
    pub naval: bool,
    /// The stored name's header, when the census gathered it with the row;
    /// [`NameArena::capture`] and [`capture_name_growing`] take it instead
    /// of reading it.
    pub name_header: Option<[u8; msvc_string::STRIDE]>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CumulativeRow {
    pub id: u64,
    /// Signed counter incremented after removing accident losses.
    pub accidents: i32,
    /// Signed fixed-point value, read and stored as a complete i64.
    pub experience: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EquipmentRow {
    pub wing: u64,
    /// Native vector position; repeated variants retain separate positions.
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
    /// Callback-local discovery context for shared variant definitions only.
    /// This is never replay identity and must not be serialized or cached.
    pub variant_pointer: usize,
}

/// Extends an already authenticated row from `walk_with`. The primary pointer
/// and row must be those passed to the same visitor invocation. Only the two
/// extra native fields are copied; a failure belongs to the extension table.
pub fn read_cumulative(
    memory: &impl Memory,
    pointer: usize,
    wing: &Row,
) -> Result<CumulativeRow, &'static str> {
    let mut accidents = [0; 4];
    let mut experience = [0; 8];
    if memory.copy_scattered(
        [AIR_ACCIDENTS, EXPERIENCE].map(|offset| pointer.checked_add(offset).unwrap_or(0)),
        &mut [&mut accidents, &mut experience],
    ) != 2
    {
        return Err("air wing cumulative field unreadable or address overflow");
    }
    Ok(CumulativeRow {
        id: wing.id,
        accidents: i32::from_le_bytes(accidents),
        experience: i64::from_le_bytes(experience),
    })
}

/// Wing construction initializes the embedded CEquipmentVariantPool.
/// Use the shared entry reader inside the wing visitor, retaining wing
/// identity and source ordinal on every variant entry.
pub fn equipment_pool(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Array, &'static str> {
    let pool = pointer
        .checked_add(EQUIPMENT_POOL)
        .ok_or("air wing equipment pool address overflow")?;
    crate::cequipment_variant_pool::entries(memory, base, pool)
}

/// Reads one already authenticated visitor wing's equipment into caller-owned
/// storage. Keep this outcome independent of AirWings. A truncated/unavailable
/// AirWings census still invalidates a seemingly complete visited prefix.
/// Entries are not filtered by amount, merged by variant, sorted or allocated.
pub fn read_equipment(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    wing: &Row,
    output: &mut [EquipmentRow],
) -> Outcome {
    let result = (|| {
        let entries = equipment_pool(memory, base, pointer)?;
        let len = output.len().min(entries.count as usize);
        for (ordinal, destination) in output[..len].iter_mut().enumerate() {
            let address = entries
                .element(ordinal as u32, crate::equipment_entry::STRIDE)
                .ok_or("air wing equipment entry address overflow")?;
            let entry = crate::equipment_entry::read(memory, base, address, ordinal as u32)?;
            *destination = EquipmentRow {
                wing: wing.id,
                ordinal: entry.source_ordinal,
                variant: entry.variant_handle,
                amount_raw: entry.amount_raw,
                variant_pointer: entry.variant_pointer,
            };
        }
        let unread_rows = entries.count - len as u32;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Read the manager's canonical-country roster, then each pool and wing once.
/// Country slots are indexed by the resolved raw country reference. Null
/// manager slots contain no roster; dense pool and wing vectors cannot
/// silently lose null entries. Duplicate handles are checked when the spool
/// sorts rows. A full buffer counts omitted pointer slots without reading
/// their wing fields, allocating, retrying or calling the engine.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_with(memory, base, game_state, output, |_, _, _| {})
}

/// Shares the authenticated wing walk with independent state extensions. The
/// visitor receives transient primary/pool pointers and the exact copied row;
/// pointers must never become replay identity. It runs only for emitted rows,
/// and cannot turn a failed extension into a failed AirWings table. Extensions
/// must withhold complete baselines when this parent census is incomplete.
/// Caller code must remain allocation-free and account for its own measured cost.
pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    mut visit: impl FnMut(usize, usize, &Row),
) -> Outcome {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Fixed(output),
        None,
        None,
        |pointer, pool, row, _| visit(pointer, pool, row),
        |_, _| {},
        None,
        |_, _| {},
    )
    .shared
    .map
}

/// Acquires independently bounded map, full-source and naval-source censuses
/// in one traversal. Naval rows go directly to the visitor instead of another
/// buffer. The visitor runs once for each row admitted to any consumer, with
/// explicit admission flags, including prefixes before later census failures.
/// Its private extension failures do not change these census outcomes.
///
/// A consumer that is full still validates mandatory roster pointers/headers,
/// but does not require omitted wing fields. Consequently a source-only leaf
/// failure cannot invalidate an already full map, or vice versa. Each outcome
/// preserves the first failure that its independent bounded walk would see.
/// Call independently for each required consistency pass; nothing is retained.
pub fn walk_shared(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    sources: &mut [Source],
    naval_limit: usize,
    mut visit: impl FnMut(usize, usize, &Row, Admitted),
) -> SharedWalk {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Fixed(output),
        Some(Buffer::Fixed(sources)),
        Some(naval_limit),
        |pointer, pool, row, admitted| visit(pointer, pool, row, admitted),
        |_, _| {},
        None,
        |_, _| {},
    )
    .shared
}

#[cfg(test)]
pub fn walk_with_names(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [Row],
    names: &mut NameArena<'_>,
    mut visit: impl FnMut(usize, usize, &Row),
) -> Outcome {
    walk_projected(
        memory,
        base,
        state,
        EVERY_OWNER,
        Buffer::Fixed(output),
        None,
        None,
        |pointer, pool, row, admitted| {
            row.name = names.capture(memory, pointer, admitted.name_header);
            visit(pointer, pool, row);
        },
        |_, _| {},
        None,
        |_, _| {},
    )
    .shared
    .map
}

/// The named wing census plus field-local political transport inputs.
/// Optional reads run only for independently admitted map rows, not naval
/// or equipment-only leaves. Scratch contains no cross-pass observations.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn walk_shared_names_with_operands(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [Row],
    sources: &mut [Source],
    naval_limit: usize,
    names: &mut NameArena<'_>,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
) -> SharedWalk {
    walk_projected(
        memory,
        base,
        state,
        EVERY_OWNER,
        Buffer::Fixed(output),
        Some(Buffer::Fixed(sources)),
        Some(naval_limit),
        |pointer, pool, row, admitted| {
            if admitted.map {
                row.name = names.capture(memory, pointer, admitted.name_header);
            }
            visit(pointer, pool, row, admitted);
        },
        |_, _| {},
        None,
        |_, _| {},
    )
    .shared
}

/// Independently captures minimal score leaves in the same authenticated walk.
/// None disables score reads; a full score arena still counts remaining demand.
#[allow(clippy::too_many_arguments)]
pub fn walk_game_score_with_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    mut names: Option<&mut NameArena<'_>>,
    score_limit: Option<usize>,
    mut visit: impl FnMut(usize, usize, &mut Row),
    score_visit: impl FnMut(usize, &ScoreLeaf),
    owners: impl FnMut(u32, Option<usize>),
) -> GameScoreWalk {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Fixed(output),
        None,
        None,
        |pointer, pool, row, admitted| {
            if let Some(names) = &mut names {
                row.name = names.capture(memory, pointer, admitted.name_header);
            }
            visit(pointer, pool, row);
        },
        owners,
        score_limit,
        score_visit,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn walk_shared_game_score_with_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    sources: &mut [Source],
    naval_limit: usize,
    names: Option<&mut NameArena<'_>>,
    score_limit: Option<usize>,
    visit: impl FnMut(usize, usize, &mut Row, Admitted),
    score_visit: impl FnMut(usize, &ScoreLeaf),
    owners: impl FnMut(u32, Option<usize>),
) -> GameScoreWalk {
    walk_shared_game_score_buffers_with_owners(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Fixed(output),
        Buffer::Fixed(sources),
        naval_limit,
        names,
        score_limit,
        visit,
        score_visit,
        owners,
    )
}

/// Allocating worker projections grow from each original pool header while
/// fixed projections preserve the existing callback refusal semantics.
///
/// `slots` are the owner (country) slots walked after the shared manager and
/// roster headers, which every part reads. Parts over consecutive slot ranges,
/// joined in slot order up to and including the first that [`GameScoreWalk::ended`],
/// are the serial walk when every projection grows: each admits every wing
/// until it fails, so no part's admissions depend on an earlier part's rows.
#[allow(clippy::too_many_arguments)]
pub fn walk_shared_game_score_buffers_with_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    slots: std::ops::Range<u32>,
    output: Buffer<'_, Row>,
    sources: Buffer<'_, Source>,
    naval_limit: usize,
    mut names: Option<&mut NameArena<'_>>,
    score_limit: Option<usize>,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
    score_visit: impl FnMut(usize, &ScoreLeaf),
    owners: impl FnMut(u32, Option<usize>),
) -> GameScoreWalk {
    walk_projected(
        memory,
        base,
        game_state,
        slots,
        output,
        Some(sources),
        Some(naval_limit),
        |pointer, pool, row, admitted| {
            if admitted.map
                && let Some(names) = &mut names
            {
                row.name = names.capture(memory, pointer, admitted.name_header);
            }
            visit(pointer, pool, row, admitted);
        },
        owners,
        score_limit,
        score_visit,
    )
}

struct Projection {
    capacity: usize,
    len: usize,
    unread_rows: u32,
    failure: Option<&'static str>,
}

impl Projection {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            len: 0,
            unread_rows: 0,
            failure: None,
        }
    }

    fn needs_row(&self) -> bool {
        self.failure.is_none() && self.len < self.capacity
    }

    fn omit(&mut self) {
        if self.failure.is_none() {
            match self.unread_rows.checked_add(1) {
                Some(unread_rows) => self.unread_rows = unread_rows,
                None => self.fail("air wing unread row count overflow"),
            }
        }
    }

    fn fail(&mut self, reason: &'static str) {
        self.failure.get_or_insert(reason);
    }

    fn outcome(&self) -> Outcome {
        match self.failure {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.unread_rows == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread_rows,
            },
        }
    }
}

/// Wings whose census rows and stored-name headers one record gather reads.
pub const ROW_BATCH: usize = 64;
/// Census steps queued between replays: up to [`ROW_BATCH`] wings and the
/// owner and pool steps among them.
const STEP_BATCH: usize = 4 * ROW_BATCH;

/// A wing's census-row spans in [`Reader::row`]'s order, then the 32-byte
/// stored-name header [`capture_name`] reads (`NAME`).
const ROW_SPANS: [(usize, usize); 15] = [
    (0, 8),
    (SECONDARY, 16),
    (POOL, 8),
    (PLANE_COUNT, 4),
    (BASE_OVERRIDE, 8),
    (MISSION + air_mission::ENABLED_MASK, 4),
    (MISSION + air_mission::REGION, 8),
    (MISSION + air_mission::ACTIVE, 1),
    (ELIGIBILITY_VETO, 1),
    (ELIGIBILITY_COUNTER, 8),
    (COUNTRY_OVERRIDE, 4),
    (BASE_RANGE, 8),
    (ACE_REFERENCE, 8),
    (ATTACHED_ORDERS_GROUP, 8),
    (NAME, msvc_string::STRIDE),
];
const ROW_FIELDS: [RecordField; 15] = packed_fields(ROW_SPANS);
const ROW_STRIDE: usize = packed_stride(ROW_SPANS);
const ROW_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&ROW_FIELDS, ROW_STRIDE) {
    Some(layout) => layout,
    None => panic!("air wing row layout"),
};

#[cfg(test)]
thread_local! {
    /// The tests' per-object reference: the census as it stood before its
    /// rows were gathered ([`serial_reference`]).
    static SERIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One step of the census in its serial order, queued by the roster walk
/// and replayed by [`walk_projected`]: an owner witness, a pool's storage
/// demand and base, or a wing.
#[derive(Clone, Copy)]
enum Step {
    Owner {
        slot: u32,
        entry: Option<usize>,
    },
    Pool {
        wings: u32,
        air_base: Result<u64, &'static str>,
    },
    Wing {
        wing: usize,
        pool: usize,
        owner: u32,
        owner_pointer: usize,
        owner_slot: u32,
        ordinal: u32,
        air_base: u64,
    },
}

struct Queue {
    steps: [Step; STEP_BATCH],
    len: usize,
    wings: usize,
}

impl Queue {
    /// Queues `step`; true when the queue must be replayed before another.
    fn push(&mut self, step: Step) -> bool {
        self.steps[self.len] = step;
        self.len += 1;
        self.wings += usize::from(matches!(step, Step::Wing { .. }));
        self.len == STEP_BATCH || self.wings == ROW_BATCH
    }

    fn take(&mut self) -> usize {
        let len = self.len;
        self.len = 0;
        self.wings = 0;
        len
    }
}

/// The census in two halves, reading what the serial walk read.
///
/// The roster half reads the manager, owners, pools and each pool's wing
/// pointers exactly as the serial walk did, a run of pointers in one copy,
/// and queues the census's steps. The replay half then visits the queued
/// steps in their serial order, so storage growth, owner witnesses, wing
/// admission and every visitor run as before; the first wing that needs its
/// row gathers the rows and stored-name headers of every queued wing from it
/// on in one record copy ([`ROW_BATCH`] a copy). Rows and headers are read
/// ahead of the earlier wings' visitors, and roster reads ahead of earlier
/// replays; a value is used only where the serial walk read it.
///
/// A failed gather keeps the rows it completed; the wing it failed at and
/// the later queued wings read their rows serially, as does a pointer run
/// that does not read; each such fallback is counted
/// ([`crate::batch_fallback`], [`GameScoreWalk::row_fallbacks`]). So a
/// persistent fault refuses what the serial walk refused, at the same wing,
/// and a one-shot fault met by a gather refuses nothing the serial walk
/// would not have. A roster fault or full projections end the census where
/// the serial walk ended it.
#[allow(clippy::too_many_arguments)]
fn walk_projected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    slots: std::ops::Range<u32>,
    mut output: Buffer<'_, Row>,
    mut sources: Option<Buffer<'_, Source>>,
    naval_limit: Option<usize>,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
    mut owner_visit: impl FnMut(u32, Option<usize>),
    score_limit: Option<usize>,
    mut score_visit: impl FnMut(usize, &ScoreLeaf),
) -> GameScoreWalk {
    let growing_sources = matches!(&sources, Some(Buffer::Growing(_)));
    let scored = score_limit.is_some();
    let mut map = Projection::new(output.len());
    let mut source = sources.as_ref().map(|rows| Projection::new(rows.len()));
    let mut naval = naval_limit.map(Projection::new);
    let mut score = score_limit.map(Projection::new);
    #[cfg(test)]
    if SERIAL.get() {
        return serial_reference::walk_projected(
            memory,
            base,
            game_state,
            slots,
            output,
            sources,
            naval_limit,
            visit,
            owner_visit,
            score_limit,
            score_visit,
        );
    }
    let fallbacks = std::cell::Cell::new(0_u32);
    let fallback = || {
        crate::batch_fallback::record(crate::batch_fallback::Batch::AirWingCensus);
        fallbacks.set(fallbacks.get().saturating_add(1));
    };
    let reader = Reader {
        memory,
        base,
        regions: RegionMemo::new(),
    };
    let mut queue = Queue {
        steps: [Step::Owner {
            slot: 0,
            entry: None,
        }; STEP_BATCH],
        len: 0,
        wings: 0,
    };
    let result = {
        // Replays queued steps in order; true when every projection failed.
        let mut replay = |steps: &[Step]| -> Result<bool, &'static str> {
            let mut pointers = [0_usize; ROW_BATCH];
            let mut wings = 0;
            for step in steps {
                if let Step::Wing { wing, .. } = *step {
                    pointers[wings] = wing;
                    wings += 1;
                }
            }
            let mut records = [0_u8; ROW_BATCH * ROW_STRIDE];
            // The gathered rows: queued wings `first..first + rows`.
            let mut gathered: Option<(usize, usize)> = None;
            let mut index = 0;
            for step in steps {
                let (wing, pool, owner, owner_pointer, owner_slot, ordinal, air_base) = match *step
                {
                    Step::Owner { slot, entry } => {
                        owner_visit(slot, entry);
                        continue;
                    }
                    Step::Pool {
                        wings: count,
                        air_base,
                    } => {
                        // The wing roster walk snapshots the pool's wing vector and count as its
                        // entire wing range. Worker storage follows that acquired range before any
                        // leaf; fixed callback projections retain their independent limits.
                        let required = map
                            .len
                            .checked_add(count as usize)
                            .ok_or("air wing roster demand overflow")?;
                        output.ensure(required)?;
                        map.capacity = output.len();
                        if let (Some(source), Some(sources)) = (&mut source, &mut sources) {
                            let required = source
                                .len
                                .checked_add(count as usize)
                                .ok_or("air wing source demand overflow")?;
                            sources.ensure(required)?;
                            source.capacity = sources.len();
                        }
                        if growing_sources && let Some(naval) = &mut naval {
                            naval.capacity = naval
                                .len
                                .checked_add(count as usize)
                                .ok_or("air wing naval source demand overflow")?;
                        }
                        if let Err(reason) = air_base {
                            map.fail(reason);
                            if let Some(source) = &mut source {
                                source.fail(reason);
                            }
                            if let Some(naval) = &mut naval {
                                naval.fail(reason);
                            }
                            if score.is_none() {
                                return Err(reason);
                            }
                        }
                        continue;
                    }
                    Step::Wing {
                        wing,
                        pool,
                        owner,
                        owner_pointer,
                        owner_slot,
                        ordinal,
                        air_base,
                    } => (
                        wing,
                        pool,
                        owner,
                        owner_pointer,
                        owner_slot,
                        ordinal,
                        air_base,
                    ),
                };
                let queued = index;
                index += 1;
                let map_needs = map.needs_row();
                let source_needs = source.as_ref().is_some_and(Projection::needs_row);
                let naval_needs = naval.as_ref().is_some_and(Projection::needs_row);
                let score_needs = score.as_ref().is_some_and(Projection::needs_row);
                if !map_needs {
                    map.omit();
                }
                if let Some(source) = &mut source
                    && !source_needs
                {
                    source.omit();
                }
                if let Some(naval) = &mut naval
                    && !naval_needs
                {
                    naval.omit();
                }
                if let Some(score) = &mut score
                    && !score_needs
                {
                    score.omit();
                }
                let details = map_needs || source_needs || naval_needs;
                if details || score_needs {
                    let (first, rows) = *gathered.get_or_insert_with(|| {
                        let wanted = wings - queued;
                        let completed = memory.copy_records(
                            RecordSources::Pointers(&pointers[queued..wings]),
                            &ROW_LAYOUT,
                            &mut records[..wanted * ROW_STRIDE],
                        );
                        let rows = completed / ROW_FIELDS.len();
                        if rows < wanted {
                            fallback();
                        }
                        (queued, rows)
                    });
                    let fields = if queued - first < rows {
                        let at = (queued - first) * ROW_STRIDE;
                        RowFields::gathered(&records[at..at + ROW_STRIDE])
                    } else {
                        hint_wing(memory, wing);
                        reader.row_fields(wing, details)
                    };
                    match reader.row(
                        &fields,
                        wing,
                        pool,
                        owner,
                        ordinal,
                        air_base,
                        details,
                        |leaf| {
                            if let Some(score) = score.as_mut().filter(|_| score_needs) {
                                match leaf {
                                    Ok(leaf) => {
                                        score_visit(wing, &leaf);
                                        score.len += 1;
                                    }
                                    Err(reason) => score.fail(reason),
                                }
                            }
                        },
                    ) {
                        Ok(Some(mut row)) => {
                            row.owner_pointer = owner_pointer;
                            row.owner_slot = owner_slot;
                            visit(
                                wing,
                                pool,
                                &mut row,
                                Admitted {
                                    map: map_needs,
                                    sources: source_needs,
                                    naval: naval_needs,
                                    name_header: fields.name,
                                },
                            );
                            if map_needs {
                                output[map.len] = row;
                                map.len += 1;
                            }
                            if let Some(source) = source.as_mut().filter(|_| source_needs) {
                                if let Some(destination) = sources
                                    .as_deref_mut()
                                    .and_then(|rows| rows.get_mut(source.len))
                                {
                                    *destination = Source {
                                        pointer: wing,
                                        pool,
                                        row,
                                    };
                                    source.len += 1;
                                } else {
                                    source.fail("air wing source projection storage missing");
                                }
                            }
                            if let Some(naval) = naval.as_mut().filter(|_| naval_needs) {
                                naval.len += 1;
                            }
                        }
                        Ok(None) => {}
                        Err(reason) => {
                            if map_needs {
                                map.fail(reason);
                            }
                            if let Some(source) = source.as_mut().filter(|_| source_needs) {
                                source.fail(reason);
                            }
                            if let Some(naval) = naval.as_mut().filter(|_| naval_needs) {
                                naval.fail(reason);
                            }
                        }
                    }
                }
                if map.failure.is_some()
                    && source
                        .as_ref()
                        .is_none_or(|source| source.failure.is_some())
                    && naval.as_ref().is_none_or(|naval| naval.failure.is_some())
                    && score.as_ref().is_none_or(|score| score.failure.is_some())
                {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let roster = (|| -> Result<bool, &'static str> {
            // Queues a step; a full queue is replayed, which may end the census.
            macro_rules! queue {
                ($step:expr) => {
                    if queue.push($step) {
                        let len = queue.take();
                        if replay(&queue.steps[..len])? {
                            return Ok(true);
                        }
                    }
                };
            }
            let countries = ccountry::Reader::new(memory, base, game_state)?;
            let manager = reader.pointer(game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
            // Country lookup returns null for a null manager. Preserve
            // AirWings' own refusal while witnessing score's independent zero.
            // The part that starts the census witnesses every slot, as the
            // serial walk does; the refusal ends it, so no later part joins.
            if manager == 0 {
                if slots.start == 0 {
                    for slot in 0..countries.count() {
                        queue!(Step::Owner { slot, entry: None });
                    }
                }
                return Err("strategic air manager absent");
            }
            // Game initialization publishes the complete strategic-air manager in the game state.
            // Its primary vtable is distinct from its secondary base's.
            let vtable =
                read::<u64>(memory, manager).ok_or("strategic air manager vtable unreadable")?;
            let expected = base
                .checked_add(cstrategic_air_manager::VTABLE_RVA)
                .ok_or("module address overflow")?;
            if vtable != expected as u64 {
                return Err("strategic air manager vtable mismatch");
            }
            let owners = reader.array(manager, cstrategic_air_manager::COUNTRY_AIR)?;
            if owners.count == 0 {
                if slots.start == 0 {
                    for slot in 0..countries.count() {
                        queue!(Step::Owner { slot, entry: None });
                    }
                }
                return Err("strategic air manager has no country slots");
            }
            for slot in slots.start..slots.end.min(owners.count) {
                let owner_pointer = reader.element(owners, slot)?;
                if owner_pointer == 0 {
                    queue!(Step::Owner { slot, entry: None });
                    continue;
                }
                reader.vtable(
                    owner_pointer,
                    cstrategic_air::VTABLE_RVA,
                    "strategic air country vtable mismatch",
                )?;
                let owner = reader.field::<u32>(owner_pointer, cstrategic_air::COUNTRY)?;
                let owner_country = if slot == 0 && scored {
                    let country = crate::game_score_inputs::read_slot_zero(
                        memory,
                        base,
                        countries.arrays()[0],
                    )?
                    .ok_or("strategic air slot zero owner has no catalogue object")?;
                    if country.id != owner {
                        return Err("strategic air slot zero owner differs from catalogue object");
                    }
                    country
                } else {
                    countries.resolve(owner)?
                };
                let owner_slot = owner_country.canonical_slot;
                if owner_slot != slot {
                    return Err("strategic air owner differs from canonical country slot");
                }
                queue!(Step::Owner {
                    slot,
                    entry: Some(owner_pointer),
                });
                let pools = reader.array(owner_pointer, cstrategic_air::POOLS)?;
                let mut pool_ordinal = 0_u32;
                for pool_index in 0..pools.count {
                    let pool = reader.element(pools, pool_index)?;
                    reader.vtable(
                        pool,
                        cair_wing_pool::VTABLE_RVA,
                        "air wing pool vtable mismatch",
                    )?;
                    if reader.pointer(pool, cair_wing_pool::OWNER)? != owner_pointer {
                        return Err("air wing pool owner differs from roster");
                    }
                    let wings = reader.array(pool, cair_wing_pool::WINGS)?;
                    let air_base = reader.field::<u64>(pool, cair_wing_pool::AIR_BASE_REFERENCE);
                    queue!(Step::Pool {
                        wings: wings.count,
                        air_base,
                    });
                    if air_base.is_err() && !scored {
                        // The replay refuses the census at this pool.
                        return Ok(false);
                    }
                    let air_base = air_base.unwrap_or(0);
                    // The pool's wing pointers, a run in one copy.
                    memory.prefetch(wings.data, (wings.count as usize).saturating_mul(8));
                    let mut next = 0_u32;
                    while next < wings.count {
                        let run = (wings.count - next).min(ROW_BATCH as u32);
                        let mut words = [0_u8; ROW_BATCH * 8];
                        let words = &mut words[..run as usize * 8];
                        let copied = wings
                            .element(next, 8)
                            .is_some_and(|at| memory.copy(at, words));
                        if !copied {
                            fallback();
                        }
                        for offset in 0..run {
                            let wing = if copied {
                                let at = offset as usize * 8;
                                usize::try_from(u64::from_le_bytes(
                                    words[at..at + 8].try_into().unwrap(),
                                ))
                                .map_err(
                                    |_| "air wing roster pointer unreadable or address overflow",
                                )?
                            } else {
                                reader.element(wings, next + offset)?
                            };
                            if wing == 0 {
                                return Err("air wing roster contains null wing");
                            }
                            let ordinal = pool_ordinal;
                            pool_ordinal = pool_ordinal
                                .checked_add(1)
                                .ok_or("air wing pool ordinal overflow")?;
                            queue!(Step::Wing {
                                wing,
                                pool,
                                owner,
                                owner_pointer: owner_country.pointer,
                                owner_slot,
                                ordinal,
                                air_base,
                            });
                        }
                        next += run;
                    }
                }
            }
            Ok(false)
        })();
        // The steps queued before the roster finished or refused replay
        // first: a replay refusal or the end of the census comes before the
        // roster's own refusal, as in the serial walk.
        match roster {
            Ok(true) => Ok(true),
            roster => {
                let len = queue.take();
                match replay(&queue.steps[..len]) {
                    Ok(true) => Ok(true),
                    Ok(false) => roster,
                    Err(reason) => Err(reason),
                }
            }
        }
    };
    let ended = result.unwrap_or_else(|reason| {
        map.fail(reason);
        if let Some(source) = &mut source {
            source.fail(reason);
        }
        if let Some(naval) = &mut naval {
            naval.fail(reason);
        }
        if let Some(score) = &mut score {
            score.fail(reason);
        }
        true
    });
    GameScoreWalk {
        shared: SharedWalk {
            map: map.outcome(),
            sources: source.map_or(Outcome::Rows { len: 0 }, |source| source.outcome()),
            naval: naval.map_or(Outcome::Rows { len: 0 }, |naval| naval.outcome()),
        },
        score: score.map_or(Outcome::Rows { len: 0 }, |score| score.outcome()),
        ended,
        row_fallbacks: fallbacks.get(),
    }
}

/// Hints the lines of one wing that its census row and stored name read: identity, pool, planes,
/// eligibility and the embedded mission through its region, the ace reference, the base range, and
/// the name, attached group and country override. Issued right after the wing's pointer is read, so
/// its misses overlap rather than serialise in the guarded scatter that follows. A hint reads
/// nothing and cannot fault ([`crate::prefetch`]): every read keeps its place and order.
fn hint_wing(memory: &impl Memory, wing: usize) {
    for (offset, len) in [
        (0, MISSION + air_mission::REGION + 8),
        (ACE_REFERENCE, 8),
        (BASE_RANGE, 8),
        (NAME, COUNTRY_OVERRIDE + 4 - NAME),
    ] {
        memory.prefetch(wing.wrapping_add(offset), len);
    }
}

/// One wing's census-row fields: one guarded scatter's ([`Reader::row_fields`])
/// with its completed prefix, or a completed gathered record's, which also
/// holds the stored-name header.
#[derive(Default)]
struct RowFields {
    completed: usize,
    primary: [u8; 8],
    secondary_and_handle: [u8; 16],
    pool: [u8; 8],
    planes: [u8; 4],
    base_override: [u8; 8],
    mask: [u8; 4],
    region: [u8; 8],
    active: [u8; 1],
    veto: [u8; 1],
    // ELIGIBILITY_COUNTER and ELIGIBILITY_LIMIT, adjacent: one span.
    eligibility: [u8; 8],
    country_override: [u8; 4],
    base_range: [u8; 8],
    ace: [u8; 8],
    group: [u8; 8],
    name: Option<[u8; msvc_string::STRIDE]>,
}

impl RowFields {
    /// A gathered [`ROW_LAYOUT`] record, every field read.
    fn gathered(record: &[u8]) -> Self {
        fn take<const N: usize>(record: &[u8], field: usize) -> [u8; N] {
            let at = ROW_FIELDS[field].destination_offset;
            record[at..at + N].try_into().unwrap()
        }
        Self {
            completed: ROW_FIELDS.len() - 1,
            primary: take(record, 0),
            secondary_and_handle: take(record, 1),
            pool: take(record, 2),
            planes: take(record, 3),
            base_override: take(record, 4),
            mask: take(record, 5),
            region: take(record, 6),
            active: take(record, 7),
            veto: take(record, 8),
            eligibility: take(record, 9),
            country_override: take(record, 10),
            base_range: take(record, 11),
            ace: take(record, 12),
            group: take(record, 13),
            name: Some(take(record, 14)),
        }
    }
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    regions: RegionMemo,
}

/// Strategic regions this pass authenticated, by address. A region's two
/// vtables and its id are read once a pass rather than once for every wing
/// whose mission names it: the pass reads the hour's world while the game
/// thread is parked at the hourly boundary, so a region read again in it
/// reads the same bytes. Only successes are held; a slot another region
/// took is read again, so the memo never changes a row or a failure.
struct RegionMemo {
    cells: [std::cell::Cell<(usize, u32)>; RegionMemo::CELLS],
}

impl RegionMemo {
    const CELLS: usize = 512;

    fn new() -> Self {
        Self {
            cells: std::array::from_fn(|_| std::cell::Cell::new((0, 0))),
        }
    }

    fn cell(&self, region: usize) -> &std::cell::Cell<(usize, u32)> {
        &self.cells[((region >> 4) ^ (region >> 13)) % Self::CELLS]
    }

    fn get(&self, region: usize) -> Option<u32> {
        let (held, id) = self.cell(region).get();
        (held == region && region != 0).then_some(id)
    }

    fn set(&self, region: usize, id: u32) {
        self.cell(region).set((region, id));
    }
}

impl<M: Memory> Reader<'_, M> {
    fn field<T: crate::memory::Raw>(
        &self,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|p| read(self.memory, p))
            .ok_or("air wing field unreadable or address overflow")
    }

    fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "air wing pointer address overflow")
    }

    fn array(&self, pointer: usize, offset: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|p| Array::read(self.memory, p, 8))
            .ok_or("air wing roster unreadable or invalid")
    }

    fn element(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("air wing roster pointer unreadable or address overflow")
    }

    fn vtable(&self, pointer: usize, rva: usize, reason: &'static str) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    /// The row's exact fields in one guarded scatter, without copying
    /// intervening bytes; without `details` only the leaf's four.
    fn row_fields(&self, pointer: usize, details: bool) -> RowFields {
        let mut fields = RowFields::default();
        let detail = |len: usize| if details { len } else { 0 };
        fields.completed = self.memory.copy_scattered(
            [
                0,
                SECONDARY,
                POOL,
                PLANE_COUNT,
                BASE_OVERRIDE,
                MISSION + air_mission::ENABLED_MASK,
                MISSION + air_mission::REGION,
                MISSION + air_mission::ACTIVE,
                ELIGIBILITY_VETO,
                ELIGIBILITY_COUNTER,
                COUNTRY_OVERRIDE,
                BASE_RANGE,
                ACE_REFERENCE,
                ATTACHED_ORDERS_GROUP,
            ]
            .map(|offset| pointer.checked_add(offset).unwrap_or(0)),
            &mut [
                &mut fields.primary,
                &mut fields.secondary_and_handle,
                &mut fields.pool,
                &mut fields.planes,
                &mut fields.base_override[..detail(8)],
                &mut fields.mask[..detail(4)],
                &mut fields.region[..detail(8)],
                &mut fields.active[..detail(1)],
                &mut fields.veto[..detail(1)],
                &mut fields.eligibility[..detail(8)],
                &mut fields.country_override[..detail(4)],
                &mut fields.base_range[..detail(8)],
                &mut fields.ace[..detail(8)],
                &mut fields.group[..detail(8)],
            ],
        );
        fields
    }

    /// Authentication precedes following the mission-region pointer.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        fields: &RowFields,
        pointer: usize,
        pool: usize,
        owner: u32,
        pool_ordinal: u32,
        air_base: u64,
        details: bool,
        mut score_visit: impl FnMut(Result<ScoreLeaf, &'static str>),
    ) -> Result<Option<Row>, &'static str> {
        let RowFields {
            completed,
            primary,
            secondary_and_handle,
            pool: pool_bytes,
            planes,
            base_override: override_bytes,
            mask,
            region,
            active,
            veto,
            eligibility,
            country_override,
            base_range,
            ace,
            group,
            name: _,
        } = *fields;
        let leaf = (|| {
            let expected_primary = self
                .base
                .checked_add(VTABLE_RVA)
                .ok_or("module address overflow")?;
            let expected_secondary = self
                .base
                .checked_add(SECONDARY_VTABLE_RVA)
                .ok_or("module address overflow")?;
            if completed == 0 || u64::from_le_bytes(primary) != expected_primary as u64 {
                return Err("air wing primary vtable mismatch");
            }
            if completed < 2
                || u64::from_le_bytes(secondary_and_handle[..8].try_into().unwrap())
                    != expected_secondary as u64
            {
                return Err("air wing secondary vtable mismatch or handle unreadable");
            }
            let id = u64::from_le_bytes(secondary_and_handle[8..].try_into().unwrap());
            if id == 0 {
                return Err("non-null air wing has null handle");
            }
            if completed < 3 {
                return Err("air wing pool field unreadable");
            }
            if u64::from_le_bytes(pool_bytes) != pool as u64 {
                return Err("air wing pool differs from roster");
            }
            Ok(ScoreLeaf {
                id,
                planes: if completed >= 4 {
                    Ok(i32::from_le_bytes(planes))
                } else {
                    Err("game score air-wing plane count unreadable")
                },
            })
        })();
        score_visit(leaf);
        let leaf = leaf?;
        if !details {
            return Ok(None);
        }
        if completed != ROW_FIELDS.len() - 1 {
            return Err("air wing row field unreadable or address overflow");
        }
        let region = usize::try_from(u64::from_le_bytes(region))
            .map_err(|_| "air wing region address overflow")?;
        let mission_region = if region == 0 {
            None
        } else if let Some(id) = self.regions.get(region) {
            Some(id)
        } else {
            self.vtable(
                region,
                cstrategic_region::VTABLE_RVA,
                "air wing mission region vtable mismatch",
            )?;
            self.vtable(
                region
                    .checked_add(cstrategic_region::SECONDARY)
                    .ok_or("air wing region address overflow")?,
                cstrategic_region::SECONDARY_VTABLE_RVA,
                "air wing mission region secondary vtable mismatch",
            )?;
            let id = self.field::<u32>(region, cstrategic_region::ID)?;
            self.regions.set(region, id);
            Some(id)
        };
        let mission_mask = u32::from_le_bytes(mask);
        // Only the tactical scan reads the group, and a group that does not
        // authenticate leaves that one field unknown, not the census.
        let attached_orders_group = if mission_mask & TACTICAL_MISSION == 0 {
            None
        } else {
            match usize::try_from(u64::from_le_bytes(group)) {
                Ok(0) => Some(0),
                Ok(group) => corders_group::plan_pointer_handle(self.memory, self.base, group)
                    .ok()
                    .filter(|&handle| {
                        creference_registry::resolve(self.memory, self.base, handle)
                            == Ok(Some(group))
                    }),
                Err(_) => None,
            }
        };
        Ok(Some(Row {
            id: leaf.id,
            owner,
            owner_pointer: 0,
            owner_slot: 0,
            pool_ordinal,
            country_override: i32::from_le_bytes(country_override),
            air_base,
            base_override: u64::from_le_bytes(override_bytes),
            planes: i32::from_le_bytes(planes),
            acquired_manpower: None,
            exile_country: None,
            mission_mask: u32::from_le_bytes(mask),
            mission_region,
            region_pointer: region,
            base_range_raw: i64::from_le_bytes(base_range),
            ace: u64::from_le_bytes(ace),
            mission_active: active[0],
            eligibility_veto: veto[0],
            eligibility_counter: i32::from_le_bytes(eligibility[..4].try_into().unwrap()),
            eligibility_limit: i32::from_le_bytes(eligibility[4..].try_into().unwrap()),
            attached_orders_group,
            attached_group_pointer: u64::from_le_bytes(group),
            name: None,
            pool_subunit: None,
            raid_reference: None,
            replacement_setting: None,
            map_flags: None,
            fuel_inputs: None,
            strategic_bombing_raw: (mission_mask & 8 != 0 && mission_mask & 0x202 == 0)
                .then(|| strategic_bombing_stat(self.memory, pointer))
                .and_then(Result::ok),
        }))
    }
}

/// The signed mission index selects a cached record; -1 selects the
/// default word. An invalid index or unreadable selected word is unknown.
/// Never substitute zero or default after a failed mission-record read.
pub fn strategic_bombing_stat(memory: &impl Memory, wing: usize) -> Result<i64, &'static str> {
    let collection = wing
        .checked_add(0x968)
        .ok_or("strategic bombing stat address overflow")?;
    let index: i8 = collection
        .checked_add(0x18 + 3)
        .and_then(|at| read(memory, at))
        .ok_or("strategic bombing mission index unreadable")?;
    let at = if index == -1 {
        wing.checked_add(0x390 + 0x32 * 8)
    } else {
        let header = Array::read(memory, collection, 0x270)
            .ok_or("strategic bombing mission-stat vector invalid")?;
        if index < 0 || index as u32 >= header.count {
            return Err("strategic bombing mission index outside vector");
        }
        header
            .element(index as u32, 0x270)
            .and_then(|at| at.checked_add(0x32 * 8))
    }
    .ok_or("strategic bombing selected stat address overflow")?;
    read(memory, at).ok_or("strategic bombing selected stat unreadable")
}

#[cfg(test)]
mod serial_reference;
#[cfg(test)]
mod tests;
