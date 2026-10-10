//! CAirWing; field offsets are relative to the primary object.
//! The secondary subobject follows the primary header. Read widths alone do
//! not establish the full width of fixed-point storage.

use crate::{
    Memory, air_mission, cair_wing_pool, ccountry, cgame_state, corders_group,
    cpdx_array::Array,
    creference_registry, cstrategic_air, cstrategic_air_manager, cstrategic_region, read,
    walk::{Buffer, Outcome},
};

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
/// CAirWing CAirWingPool* at primary+38; translated from body+28 and independently used by fuel walkers.
pub const POOL: usize = 0x38;
/// Air-group paired-u32 handle.
pub const GROUP_HANDLE: usize = 0x40;
/// Plane count, i32.
pub const PLANE_COUNT: usize = 0x7c;
/// Cumulative signed i32 accident count, incremented after aircraft removal.
pub const AIR_ACCIDENTS: usize = 0x84;
/// Embedded mission object, with layout in air_mission.
pub const MISSION: usize = 0x90;
/// Inline equipment-bank vector.
pub const EQUIPMENT_BANK: usize = 0x1f8;
/// Embedded CEquipmentVariantPool initialized by wing construction.
pub const EQUIPMENT_POOL: usize = 0x1d8;
/// Equipment-bank capacity, u32.
pub const EQUIPMENT_BANK_CAPACITY: usize = 0x200;
/// Equipment-bank count, u32.
pub const EQUIPMENT_BANK_COUNT: usize = 0x204;
/// Experience, signed i64; updates read, add, clamp and write the complete value.
pub const EXPERIENCE: usize = 0x218;
/// Assigned ace id, u32.
pub const ASSIGNED_ACE_ID: usize = 0x2ec;
/// Base override reference resolved before falling back to the pool base.
pub const BASE_OVERRIDE: usize = 0x48;
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
/// Attached orders-group pointer matched against combat armies in tactical target scans.
pub const ATTACHED_ORDERS_GROUP: usize = 0x9b8;
/// Stored wing name string.
pub const NAME: usize = 0x998;
/// The tactical (CAS) mission bit, the only reader of the attached group here.
pub const TACTICAL_MISSION: u32 = 1 << 1;
/// Modified fuel getter returning i64 fixed5.
pub const FUEL_RVA: usize = 0x00f4_a700;
/// Capacity divisor selecting pool definition and, when needed, base capacity.
pub const CAPACITY_DIVISOR_RVA: usize = 0x00f4_9e80;
/// Air-base accessor.
pub const AIR_BASE_RVA: usize = 0x00f4_a6b0;
/// Training tick candidate updating the experience field.
pub const TRAINING_TICK_RVA: usize = 0x00f4_8450;
/// Air-base admission gate using pool, country override and access.
pub const AIR_BASE_ADMISSION_RVA: usize = 0x00f4_7d60;

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
}

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
    /// Full native reference at primary+18; no collector generation or pointer.
    pub id: u64,
    /// Raw country reference from the owning CStrategicAir roster.
    pub owner: u32,
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
    pub mission_mask: u32,
    /// None means a null native region pointer, never a failed region read.
    pub mission_region: Option<u32>,
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
    /// Span in the caller's name arena; absent after a field-local refusal.
    pub name: Option<(usize, usize)>,
    /// Authenticated containing pool's recorded-content database index.
    /// Null subtype is Some(0); optional field read failure is None.
    pub pool_subunit: Option<u32>,
    /// Complete stored raid handle plus same-pass registry liveness for
    /// admitted idle transport wings; absent after optional field refusal.
    pub raid_reference: Option<transport_operands::RaidReference>,
    /// Selected tactical mission stat, None after an optional-read refusal.
    pub strategic_bombing_raw: Option<i64>,
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
    pub fn capture(&mut self, memory: &impl Memory, pointer: usize) -> Option<(usize, usize)> {
        match capture_name(
            memory,
            pointer,
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
    bytes: &mut Vec<u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), NameFailure> {
    capture_name(memory, pointer, Buffer::Growing(bytes), used, required)
}

fn capture_name(
    memory: &impl Memory,
    pointer: usize,
    mut bytes: Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<(usize, usize), NameFailure> {
    let at = pointer.checked_add(NAME).ok_or(NameFailure::Overflow)?;
    let name = crate::raid_tooltip_operands::StoredString::read(memory, at)
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
        |pointer, _, _, _| {
            if let Ok(bytes) = required {
                required = (|| {
                    let at = pointer
                        .checked_add(NAME)
                        .ok_or("air wing name address overflow")?;
                    let name = crate::raid_tooltip_operands::StoredString::read(memory, at)
                        .map_err(crate::raid_tooltip_operands::NameReadError::reason)?;
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
        Buffer::Fixed(output),
        None,
        None,
        |pointer, pool, row, _| {
            row.name = names.capture(memory, pointer);
            visit(pointer, pool, row);
        },
        |_, _| {},
        None,
        |_, _| {},
    )
    .shared
    .map
}

#[allow(clippy::too_many_arguments)]
pub fn walk_shared_names(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [Row],
    sources: &mut [Source],
    naval_limit: usize,
    names: &mut NameArena<'_>,
    mut visit: impl FnMut(usize, usize, &Row, Admitted),
) -> SharedWalk {
    walk_shared_names_with_operands(
        memory,
        base,
        state,
        output,
        sources,
        naval_limit,
        names,
        |pointer, pool, row, admitted| visit(pointer, pool, row, admitted),
    )
}

/// The named wing census plus field-local political transport inputs.
/// Optional reads run only for independently admitted map rows, not naval
/// or equipment-only leaves. Scratch contains no cross-pass observations.
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
        Buffer::Fixed(output),
        Some(Buffer::Fixed(sources)),
        Some(naval_limit),
        |pointer, pool, row, admitted| {
            if admitted.map {
                row.name = names.capture(memory, pointer);
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
        Buffer::Fixed(output),
        None,
        None,
        |pointer, pool, row, _| {
            if let Some(names) = &mut names {
                row.name = names.capture(memory, pointer);
            }
            visit(pointer, pool, row);
        },
        owners,
        score_limit,
        score_visit,
    )
}

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
#[allow(clippy::too_many_arguments)]
pub fn walk_shared_game_score_buffers_with_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
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
        output,
        Some(sources),
        Some(naval_limit),
        |pointer, pool, row, admitted| {
            if admitted.map
                && let Some(names) = &mut names
            {
                row.name = names.capture(memory, pointer);
            }
            visit(pointer, pool, row, admitted);
        },
        owners,
        score_limit,
        score_visit,
    )
}

pub fn walk_with_names_and_transport(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [Row],
    names: &mut NameArena<'_>,
    scratch: &mut transport_operands::Scratch,
    mut visit: impl FnMut(usize, usize, &Row),
) -> Outcome {
    let mut transport = transport_operands::Pass::new(base, output.len(), scratch);
    walk_projected(
        memory,
        base,
        state,
        Buffer::Fixed(output),
        None,
        None,
        |pointer, pool, row, _| {
            row.name = names.capture(memory, pointer);
            transport.capture(memory, pointer, pool, row);
            visit(pointer, pool, row);
        },
        |_, _| {},
        None,
        |_, _| {},
    )
    .shared
    .map
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

#[allow(clippy::too_many_arguments)]
fn walk_projected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, Row>,
    mut sources: Option<Buffer<'_, Source>>,
    naval_limit: Option<usize>,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
    mut owner_visit: impl FnMut(u32, Option<usize>),
    score_limit: Option<usize>,
    mut score_visit: impl FnMut(usize, &ScoreLeaf),
) -> GameScoreWalk {
    let growing_sources = matches!(&sources, Some(Buffer::Growing(_)));
    let mut map = Projection::new(output.len());
    let mut source = sources.as_ref().map(|rows| Projection::new(rows.len()));
    let mut naval = naval_limit.map(Projection::new);
    let mut score = score_limit.map(Projection::new);
    let result = (|| {
        let reader = Reader { memory, base };
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let manager = reader.pointer(game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
        // A null country-air manager means known zero for score. Preserve the
        // independent AirWings refusal.
        if manager == 0 {
            for slot in 0..countries.count() {
                owner_visit(slot, None);
            }
            return Err("strategic air manager absent");
        }
        // The published air-manager complete object uses its own primary vtable,
        // separately from the embedded base subobject.
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
            for slot in 0..countries.count() {
                owner_visit(slot, None);
            }
            return Err("strategic air manager has no country slots");
        }
        for slot in 0..owners.count {
            let owner_pointer = reader.element(owners, slot)?;
            if owner_pointer == 0 {
                owner_visit(slot, None);
                continue;
            }
            reader.vtable(
                owner_pointer,
                cstrategic_air::VTABLE_RVA,
                "strategic air country vtable mismatch",
            )?;
            let owner = reader.field::<u32>(owner_pointer, cstrategic_air::COUNTRY)?;
            let owner_slot = if slot == 0 && score.is_some() {
                let country =
                    crate::game_score_inputs::read_slot_zero(memory, base, countries.arrays()[0])?
                        .ok_or("strategic air slot zero owner has no catalogue object")?;
                if country.id != owner {
                    return Err("strategic air slot zero owner differs from catalogue object");
                }
                country.canonical_slot
            } else {
                countries.resolve(owner)?.canonical_slot
            };
            if owner_slot != slot {
                return Err("strategic air owner differs from canonical country slot");
            }
            owner_visit(slot, Some(owner_pointer));
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
                // The acquired pool header selects the complete wing range. Workers
                // grow storage before any leaf read; fixed callback projections retain
                // their independent limits.
                let required = map
                    .len
                    .checked_add(wings.count as usize)
                    .ok_or("air wing roster demand overflow")?;
                output.ensure(required)?;
                map.capacity = output.len();
                if let (Some(source), Some(sources)) = (&mut source, &mut sources) {
                    let required = source
                        .len
                        .checked_add(wings.count as usize)
                        .ok_or("air wing source demand overflow")?;
                    sources.ensure(required)?;
                    source.capacity = sources.len();
                }
                if growing_sources && let Some(naval) = &mut naval {
                    naval.capacity = naval
                        .len
                        .checked_add(wings.count as usize)
                        .ok_or("air wing naval source demand overflow")?;
                }
                let air_base = match reader.field::<u64>(pool, cair_wing_pool::AIR_BASE_REFERENCE) {
                    Ok(value) => value,
                    Err(reason) => {
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
                        0
                    }
                };
                for wing_index in 0..wings.count {
                    let wing = reader.element(wings, wing_index)?;
                    if wing == 0 {
                        return Err("air wing roster contains null wing");
                    }
                    let ordinal = pool_ordinal;
                    pool_ordinal = pool_ordinal
                        .checked_add(1)
                        .ok_or("air wing pool ordinal overflow")?;
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
                        match reader.row(wing, pool, owner, ordinal, air_base, details, |leaf| {
                            if let Some(score) = score.as_mut().filter(|_| score_needs) {
                                match leaf {
                                    Ok(leaf) => {
                                        score_visit(wing, &leaf);
                                        score.len += 1;
                                    }
                                    Err(reason) => score.fail(reason),
                                }
                            }
                        }) {
                            Ok(Some(mut row)) => {
                                visit(
                                    wing,
                                    pool,
                                    &mut row,
                                    Admitted {
                                        map: map_needs,
                                        sources: source_needs,
                                        naval: naval_needs,
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
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(reason) = result {
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
    }
    GameScoreWalk {
        shared: SharedWalk {
            map: map.outcome(),
            sources: source.map_or(Outcome::Rows { len: 0 }, |source| source.outcome()),
            naval: naval.map_or(Outcome::Rows { len: 0 }, |naval| naval.outcome()),
        },
        score: score.map_or(Outcome::Rows { len: 0 }, |score| score.outcome()),
    }
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
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

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        pointer: usize,
        pool: usize,
        owner: u32,
        pool_ordinal: u32,
        air_base: u64,
        details: bool,
        mut score_visit: impl FnMut(Result<ScoreLeaf, &'static str>),
    ) -> Result<Option<Row>, &'static str> {
        let mut primary = [0; 8];
        let mut secondary_and_handle = [0; 16];
        let mut pool_bytes = [0; 8];
        let mut override_bytes = [0; 8];
        let mut planes = [0; 4];
        let mut mask = [0; 4];
        let mut region = [0; 8];
        let mut active = [0; 1];
        let mut veto = [0; 1];
        let mut counter = [0; 4];
        let mut limit = [0; 4];
        let mut country_override = [0; 4];
        let mut base_range = [0; 8];
        let mut ace = [0; 8];
        let mut group = [0; 8];
        // Exact fields only: one guarded scatter, without copying intervening
        // bytes. Authentication precedes following the mission-region pointer.
        let completed = self.memory.copy_scattered(
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
                ELIGIBILITY_LIMIT,
                COUNTRY_OVERRIDE,
                BASE_RANGE,
                ACE_REFERENCE,
                ATTACHED_ORDERS_GROUP,
            ]
            .map(|offset| pointer.checked_add(offset).unwrap_or(0)),
            &mut [
                &mut primary,
                &mut secondary_and_handle,
                &mut pool_bytes,
                &mut planes,
                &mut override_bytes[..if details { 8 } else { 0 }],
                &mut mask[..if details { 4 } else { 0 }],
                &mut region[..if details { 8 } else { 0 }],
                &mut active[..usize::from(details)],
                &mut veto[..usize::from(details)],
                &mut counter[..if details { 4 } else { 0 }],
                &mut limit[..if details { 4 } else { 0 }],
                &mut country_override[..if details { 4 } else { 0 }],
                &mut base_range[..if details { 8 } else { 0 }],
                &mut ace[..if details { 8 } else { 0 }],
                &mut group[..if details { 8 } else { 0 }],
            ],
        );
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
        if completed != 15 {
            return Err("air wing row field unreadable or address overflow");
        }
        let region = usize::try_from(u64::from_le_bytes(region))
            .map_err(|_| "air wing region address overflow")?;
        let mission_region = if region == 0 {
            None
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
            Some(self.field::<u32>(region, cstrategic_region::ID)?)
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
            pool_ordinal,
            country_override: i32::from_le_bytes(country_override),
            air_base,
            base_override: u64::from_le_bytes(override_bytes),
            planes: i32::from_le_bytes(planes),
            mission_mask: u32::from_le_bytes(mask),
            mission_region,
            base_range_raw: i64::from_le_bytes(base_range),
            ace: u64::from_le_bytes(ace),
            mission_active: active[0],
            eligibility_veto: veto[0],
            eligibility_counter: i32::from_le_bytes(counter),
            eligibility_limit: i32::from_le_bytes(limit),
            attached_orders_group,
            name: None,
            pool_subunit: None,
            raid_reference: None,
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
mod tests;
