use engine_native_parts::cair_wing_layout::GROUP_HANDLE;
use engine_native_parts::cair_wing_layout::{
    POOL,
    SECONDARY,
    SECONDARY_VTABLE_RVA,
    VTABLE_RVA,
};

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        RecordField,
        RecordLayout,
        RecordSources,
        packed_fields,
        packed_stride,
    },
    msvc_string,
    read,
    walk::{Buffer, Outcome},
};
use crate::{air_mission, cstrategic_air, cstrategic_air_manager};
use engine_native_parts::{cair_wing_pool, creference_registry, cstrategic_region};
use engine_native::{ccountry, cgame_state, corders_group};
use engine_sites::Rva;

pub mod fuel_inputs;
pub mod replacement;
pub mod transport_operands;

pub const ID: usize = 0x1c;
pub const PLANE_COUNT: usize = 0x7c;
pub const ACQUIRED_MANPOWER: usize = 0x80;
pub const AIR_ACCIDENTS: usize = 0x84;
pub const MISSION: usize = 0x90;
pub const EQUIPMENT_BANK: usize = 0x1f8;
pub const EQUIPMENT_POOL: usize = 0x1d8;
#[cfg(any(test, feature = "layout-fixtures"))]
pub const EQUIPMENT_BANK_CAPACITY: usize = 0x200;
pub const EQUIPMENT_BANK_COUNT: usize = 0x204;
pub const EXPERIENCE: usize = 0x218;
pub const MODIFIERS: usize = 0x238;
pub const NIGHT_OPERATIONS_STAT: usize = 0x4c;
pub const BASE_OVERRIDE: usize = 0x48;
pub const RELOCATION_PROGRESS: usize = 0x50;
pub const RELOCATION_DISTANCE: usize = 0x58;
pub const ELIGIBILITY_COUNTER: usize = 0x60;
pub const ELIGIBILITY_LIMIT: usize = 0x64;
pub const ELIGIBILITY_VETO: usize = 0x79;
pub const ACE_REFERENCE: usize = 0x2e8;
pub const BASE_RANGE: usize = 0x500;
pub const BASE_FUEL: usize = 0x5b8;
pub const COUNTRY_OVERRIDE: usize = 0x9c4;
pub const EXILE_COUNTRY: usize = 0x9c8;
pub const ATTACHED_ORDERS_GROUP: usize = 0x9b8;
pub const NAME: usize = 0x998;
pub const TACTICAL_MISSION: u32 = 1 << 1;

pub const DEFAULT_STATS: usize = 0x390;
pub const MISSION_STATS: usize = 0x968;
pub const MISSION_STAT_INDEXES: usize = 0x18;
pub const MISSION_STAT_STRIDE: usize = 0x270;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreLeaf {
    pub id: u64,
    pub planes: Result<i32, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameScoreWalk {
    pub shared: SharedWalk,
    pub score: Outcome,
    pub ended: bool,
    pub row_fallbacks: u32,
}

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
    pub id: u64,
    pub owner: u32,
    pub owner_pointer: usize,
    pub owner_slot: u32,
    pub pool_ordinal: u32,
    pub country_override: i32,
    pub air_base: u64,
    pub base_override: u64,
    pub planes: i32,
    pub acquired_manpower: Option<i32>,
    pub exile_country: Option<i32>,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
    pub region_pointer: usize,
    pub base_range_raw: i64,
    pub ace: u64,
    pub mission_active: u8,
    pub eligibility_veto: u8,
    pub eligibility_counter: i32,
    pub eligibility_limit: i32,
    pub attached_orders_group: Option<u64>,
    pub attached_group_pointer: u64,
    pub name: Option<(usize, usize)>,
    pub pool_subunit: Option<u32>,
    pub raid_reference: Option<transport_operands::RaidReference>,
    pub strategic_bombing_raw: Option<i64>,
    pub replacement_setting: Option<u8>,
    pub map_flags: Option<u32>,
    pub fuel_inputs: Option<fuel_inputs::Inputs>,
}

pub struct NameArena<'a> {
    pub bytes: &'a mut Vec<u8>,
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
            Buffer::Growing(&mut *self.bytes),
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

pub fn name_demand(memory: &impl Memory, base: usize, state: usize) -> Result<usize, &'static str> {
    let mut required = Ok(0usize);
    let walked = walk_shared(
        memory,
        base,
        state,
        Buffer::Count,
        Buffer::Count,
        |pointer, _, _, admitted| {
            if let Ok(bytes) = required {
                required = (|| {
                    use crate::raid_tooltip_operands::{NameReadError, StoredString};
                    let at = pointer
                        .checked_add(NAME)
                        .ok_or("air wing name address overflow")?;
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub map: bool,
    pub sources: bool,
    pub naval: bool,
    pub name_header: Option<[u8; msvc_string::STRIDE]>,
    pub crew: Option<[i32; 2]>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CumulativeRow {
    pub id: u64,
    pub accidents: i32,
    pub experience: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EquipmentRow {
    pub wing: u64,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
    pub variant_pointer: usize,
}

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

pub fn equipment_pool(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Array, &'static str> {
    let pool = pointer
        .checked_add(EQUIPMENT_POOL)
        .ok_or("air wing equipment pool address overflow")?;
    engine_native::cequipment_variant::pool::entries(memory, base, pool)
}

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
                .element(ordinal as u32, engine_native_parts::equipment_entry::STRIDE)
                .ok_or("air wing equipment entry address overflow")?;
            let entry = engine_native_parts::equipment_entry::read(memory, base, address, ordinal as u32)?;
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

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut Vec<Row>) -> Outcome {
    walk_with(memory, base, game_state, output, |_, _, _| {})
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Count,
        None,
        false,
        |_, _, _, _| {},
        |_, _| {},
        false,
        |_, _| {},
    )
    .shared
    .map
}

pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    mut visit: impl FnMut(usize, usize, &Row),
) -> Outcome {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Growing(output),
        None,
        false,
        |pointer, pool, row, _| visit(pointer, pool, row),
        |_, _| {},
        false,
        |_, _| {},
    )
    .shared
    .map
}

pub fn walk_shared(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: Buffer<'_, Row>,
    sources: Buffer<'_, Source>,
    mut visit: impl FnMut(usize, usize, &Row, Admitted),
) -> SharedWalk {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        output,
        Some(sources),
        true,
        |pointer, pool, row, admitted| visit(pointer, pool, row, admitted),
        |_, _| {},
        false,
        |_, _| {},
    )
    .shared
}

#[cfg(test)]
pub fn walk_with_names(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<Row>,
    names: &mut NameArena<'_>,
    mut visit: impl FnMut(usize, usize, &Row),
) -> Outcome {
    walk_projected(
        memory,
        base,
        state,
        EVERY_OWNER,
        Buffer::Growing(output),
        None,
        false,
        |pointer, pool, row, admitted| {
            row.name = names.capture(memory, pointer, admitted.name_header);
            visit(pointer, pool, row);
        },
        |_, _| {},
        false,
        |_, _| {},
    )
    .shared
    .map
}

#[cfg(test)]
pub fn walk_shared_names_with_operands(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<Row>,
    sources: &mut Vec<Source>,
    names: &mut NameArena<'_>,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
) -> SharedWalk {
    walk_projected(
        memory,
        base,
        state,
        EVERY_OWNER,
        Buffer::Growing(output),
        Some(Buffer::Growing(sources)),
        true,
        |pointer, pool, row, admitted| {
            if admitted.map {
                row.name = names.capture(memory, pointer, admitted.name_header);
            }
            visit(pointer, pool, row, admitted);
        },
        |_, _| {},
        false,
        |_, _| {},
    )
    .shared
}

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "the output, names and the row, score and owner callbacks are separate caller-owned parts of one walk"
)]
pub(crate) fn walk_game_score_with_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    mut names: Option<&mut NameArena<'_>>,
    scored: bool,
    mut visit: impl FnMut(usize, usize, &mut Row),
    score_visit: impl FnMut(usize, &ScoreLeaf),
    owners: impl FnMut(u32, Option<usize>),
) -> GameScoreWalk {
    walk_projected(
        memory,
        base,
        game_state,
        EVERY_OWNER,
        Buffer::Growing(output),
        None,
        false,
        |pointer, pool, row, admitted| {
            if let Some(names) = &mut names {
                row.name = names.capture(memory, pointer, admitted.name_header);
            }
            visit(pointer, pool, row);
        },
        owners,
        scored,
        score_visit,
    )
}

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "the output, sources, names and the row, score and owner callbacks are separate caller-owned parts of one walk"
)]
pub(crate) fn walk_shared_game_score_with_owners(memory: &impl Memory, base: usize, game_state: usize, output: &mut Vec<Row>, sources: &mut Vec<Source>, names: Option<&mut NameArena<'_>>, scored: bool, visit: impl FnMut(usize, usize, &mut Row, Admitted), score_visit: impl FnMut(usize, &ScoreLeaf), owners: impl FnMut(u32, Option<usize>)) -> GameScoreWalk {
    walk_shared_game_score_buffers_with_owners(memory, base, game_state, EVERY_OWNER, Buffer::Growing(output), Buffer::Growing(sources), names, scored, visit, score_visit, owners)
}

pub fn walk_shared_game_score_buffers_with_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    slots: std::ops::Range<u32>,
    output: Buffer<'_, Row>,
    sources: Buffer<'_, Source>,
    mut names: Option<&mut NameArena<'_>>,
    scored: bool,
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
        true,
        |pointer, pool, row, admitted| {
            if admitted.map
                && let Some(names) = &mut names
            {
                row.name = names.capture(memory, pointer, admitted.name_header);
            }
            visit(pointer, pool, row, admitted);
        },
        owners,
        scored,
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

pub const ROW_BATCH: usize = 64;
const STEP_BATCH: usize = 4 * ROW_BATCH;

const ROW_SPANS: [(usize, usize); 15] = [
    (0, 8),
    (SECONDARY, 16),
    (POOL, 8),
    (PLANE_COUNT, 8),
    (BASE_OVERRIDE, 8),
    (MISSION + air_mission::ENABLED_MASK, 4),
    (MISSION + air_mission::REGION, 8),
    (MISSION + air_mission::ACTIVE, 1),
    (ELIGIBILITY_VETO, 1),
    (ELIGIBILITY_COUNTER, 8),
    (COUNTRY_OVERRIDE, 8),
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
    static SERIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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

#[expect(
    clippy::too_many_arguments,
    reason = "the slots, output, sources and the row, owner and score callbacks are separate caller-owned parts of one walk"
)]
fn walk_projected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    slots: std::ops::Range<u32>,
    mut output: Buffer<'_, Row>,
    mut sources: Option<Buffer<'_, Source>>,
    naval: bool,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
    mut owner_visit: impl FnMut(u32, Option<usize>),
    scored: bool,
    mut score_visit: impl FnMut(usize, &ScoreLeaf),
) -> GameScoreWalk {
    let mut map = Projection::new(output.len());
    let mut source = sources.as_ref().map(|rows| Projection::new(rows.len()));
    let mut naval = naval.then(|| Projection::new(usize::MAX));
    let mut score = scored.then(|| Projection::new(usize::MAX));
    #[cfg(test)]
    if SERIAL.get() {
        return serial_reference::walk_projected(
            memory,
            base,
            game_state,
            slots,
            output,
            sources,
            naval.is_some(),
            visit,
            owner_visit,
            scored,
            score_visit,
        );
    }
    let fallbacks = std::cell::Cell::new(0_u32);
    let fallback = || {
        engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::AirWingCensus);
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
                                    crew: fields.crew,
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
            if manager == 0 {
                if slots.start == 0 {
                    for slot in 0..countries.count() {
                        queue!(Step::Owner { slot, entry: None });
                    }
                }
                return Err("strategic air manager absent");
            }
            let vtable =
                read::<u64>(memory, manager).ok_or("strategic air manager vtable unreadable")?;
            let expected = engine_sites::at(base, cstrategic_air_manager::VTABLE_RVA)
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
                        return Ok(false);
                    }
                    let air_base = air_base.unwrap_or(0);
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
    eligibility: [u8; 8],
    country_override: [u8; 4],
    base_range: [u8; 8],
    ace: [u8; 8],
    group: [u8; 8],
    name: Option<[u8; msvc_string::STRIDE]>,
    crew: Option<[i32; 2]>,
}

impl RowFields {
    fn gathered(record: &[u8]) -> Self {
        fn take<const N: usize>(record: &[u8], field: usize) -> [u8; N] {
            let at = ROW_FIELDS[field].destination_offset;
            record[at..at + N].try_into().unwrap()
        }
        let planes_and_manpower: [u8; 8] = take(record, 3);
        let country_and_exile: [u8; 8] = take(record, 10);
        Self {
            completed: ROW_FIELDS.len() - 1,
            primary: take(record, 0),
            secondary_and_handle: take(record, 1),
            pool: take(record, 2),
            planes: planes_and_manpower[..4].try_into().unwrap(),
            base_override: take(record, 4),
            mask: take(record, 5),
            region: take(record, 6),
            active: take(record, 7),
            veto: take(record, 8),
            eligibility: take(record, 9),
            country_override: country_and_exile[..4].try_into().unwrap(),
            base_range: take(record, 11),
            ace: take(record, 12),
            group: take(record, 13),
            name: Some(take(record, 14)),
            crew: Some([
                i32::from_le_bytes(planes_and_manpower[4..8].try_into().unwrap()),
                i32::from_le_bytes(country_and_exile[4..8].try_into().unwrap()),
            ]),
        }
    }
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    regions: RegionMemo,
}

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
    fn field<T: engine_native_memory::memory::Raw>(
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

    fn vtable(&self, pointer: usize, rva: Rva, reason: &'static str) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, rva).ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

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

    #[expect(
        clippy::too_many_arguments,
        reason = "the pointer, pool, owner, ordinal and air base identify one wing, and the score callback is caller-owned"
    )]
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
            crew: _,
        } = *fields;
        let leaf = (|| {
            let expected_primary =
                engine_sites::at(self.base, VTABLE_RVA).ok_or("module address overflow")?;
            let expected_secondary = engine_sites::at(self.base, SECONDARY_VTABLE_RVA)
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WingMembership {
    pub wing: u64,
    pub group: u64,
}

pub fn read_wing_membership(
    memory: &impl Memory,
    pointer: usize,
    wing: &Row,
) -> Result<WingMembership, &'static str> {
    let group = pointer
        .checked_add(GROUP_HANDLE)
        .and_then(|address| read::<u64>(memory, address))
        .ok_or("air wing theatre group reference unreadable")?;
    Ok(WingMembership {
        wing: wing.id,
        group,
    })
}
