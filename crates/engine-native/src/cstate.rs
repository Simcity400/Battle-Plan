use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    memory::{FieldBatch, RecordField, RecordLayout, RecordSources},
    read,
    walk::{self, Outcome},
};

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x029b_4d38;
/// State ID, u32, copied from the template and used as its roster slot.
pub const ID: usize = 0x58;

/// Owning country raw reference, u32.
pub const OWNER: usize = 0xc8;
/// Controlling country raw reference, u32, independent of owner and province controllers.
pub const CONTROLLER: usize = 0xcc;

/// A province's state as the land movement speed reads it: the state's id and
/// raw controller. `state` is a nonnull state pointer a province holds.
pub fn id_and_controller(memory: &impl Memory, state: usize) -> Result<(u32, u32), &'static str> {
    let field = |offset: usize| {
        state
            .checked_add(offset)
            .and_then(|address| read::<u32>(memory, address))
            .ok_or("movement state field unreadable")
    };
    Ok((field(ID)?, field(CONTROLLER)?))
}

/// CState province-pointer CPdxArray.
pub const PROVINCES: usize = 0x18;
/// CState province-list signed i32 count.
pub const PROVINCE_COUNT: usize = 0x24;
/// State-template pointer from the second constructor argument.
pub const TEMPLATE: usize = 0x30;
/// Primary display-name string selected when nonempty. Scripted naming
/// localises it; serialization emits nonempty text as `name`.
pub const PRIMARY_DISPLAY_NAME: usize = 0x38;
/// Secondary display-name string selected before template fallback. Ownership
/// updates localise the owner tag/template key when available and otherwise
/// clear it. This cache is not serialized.
pub const SECONDARY_DISPLAY_NAME: usize = 0xe8;
/// CState claim-country CPdxArray of u32 links.
pub const CLAIMS: usize = 0x98;
/// CState core-country CPdxArray of u32 links.
pub const CORES: usize = 0xb0;
/// Cached controller-group count; the scripted layer tests exactly one.
pub const CONTROLLER_GROUP_COUNT: usize = 0x1a4;
/// CState strategic-location CPdxArray of 8-byte (location token, anchor
/// province id) pairs in insertion order. The state effect appends one
/// pair and refuses a token already listed; loading reads the list.
pub const STRATEGIC_LOCATIONS: usize = 0x800;
/// The size of one [`STRATEGIC_LOCATIONS`] pair.
pub const STRATEGIC_LOCATION_STRIDE: usize = 8;
/// Embedded building status with its building vector.
pub const BUILDING_STATUS: usize = 0x120;
/// Embedded resistance object.
pub const RESISTANCE: usize = 0x268;
/// Flag-walk address. Container ownership is unresolved; construction only
/// establishes the pointer before the resource modifier.
pub const FLAGS: usize = 0x540;
/// Embedded resource modifier and accumulator.
pub const RESOURCE_MODIFIER: usize = 0x548;
/// Base resource-modifier accumulator used by base and country-specific queries.
pub const BASE_MODIFIERS: usize = 0x558;
/// Additive resource-modifier gate byte refreshed before resource calculation.
pub const RESOURCE_MODIFIER_GATE: usize = 0x1b4;
/// Added-resource pool serialized as `resources`.
pub const RESOURCE_POOL: usize = 0x1b8;
/// Dynamic-modifier container used by state-scope triggers and serialized.
pub const DYNAMIC_MODIFIERS: usize = 0x788;
/// Variable-walk address with unresolved container ownership.
pub const VARIABLES: usize = 0x7f8;
/// Demilitarized boolean byte set by the effect and serialized independently
/// of its adjacent byte.
pub const DEMILITARIZED: usize = 0x864;
/// Active border-war byte serialized independently.
#[cfg(test)]
pub const BORDER_CONFLICT_ACTIVE: usize = 0x865;
/// Pointer-ordered border-combat set size.
pub const BORDER_CONFLICT_SIZE: usize = 0x8a8;
mod border_conflicts;
pub mod infrastructure_map;
/// Embedded state manpower object.
pub const MANPOWER: usize = 0x838;
/// Stored signed available manpower summed across pools and serialized.
pub const MANPOWER_AVAILABLE: usize = MANPOWER + 0x10;
/// Stored signed population locked outside the recruitable pool.
pub const MANPOWER_LOCKED: usize = MANPOWER + 0x14;
/// The state's population, signed i32, serialized as the manpower pool's
/// `total`: the value the `state_population` triggers compare and the
/// manpower map mode colours each state by (over its define
/// `MAP_MODE_MANPOWER_RANGE_MAX`). Available manpower is a different value.
pub const POPULATION: usize = MANPOWER + 0x18;
/// Cached best victory-point index; negative selects the first province.
pub const BEST_VP_INDEX: usize = 0x85c;
/// Bytes from [`MANPOWER_AVAILABLE`] through [`POPULATION`], read as one span.
const MANPOWER_SPAN: usize = POPULATION + 4 - MANPOWER_AVAILABLE;
/// The state's category definition pointer: loading and the category effect
/// replace it, and the save writes its definition's name as `state_category`.
pub const CATEGORY: usize = 0x898;
/// State category definition vtable.
pub const CATEGORY_VTABLE_RVA: usize = 0x02a3_5078;
/// The category definition's lexer token id, u32, beside its vtable.
pub const CATEGORY_TOKEN: usize = 0x08;
/// State constructor establishing template and ID.
pub const CONSTRUCTOR_RVA: usize = 0x00a1_a830;
/// Owner setter updating the raw country reference before ownership notifications.
pub const SET_OWNER_RVA: usize = 0x00a2_9220;
/// Sorted conflict-tree head storing border combats. Predicates inspect
/// side countries and intersect tree values.
pub const BORDER_CONFLICT_TREE: usize = 0x8a0;
/// Country-specific modifier hash data pointer.
pub const COUNTRY_MODIFIERS: usize = 0x8d0;
/// Country-modifier bucket stride.
pub const COUNTRY_MODIFIER_STRIDE: usize = 0xd0;

/// Batch IDs behind an ordered state-pointer array. Null memberships remain
/// zero. Claims and country rosters use the same stored state ID.
pub(crate) fn copy_ids(
    memory: &impl Memory,
    source: Array,
    output: &mut [u32],
    pointer_error: &'static str,
    id_error: &'static str,
) -> Result<(), &'static str> {
    if output.len() != source.count as usize {
        return Err("state id destination does not match header");
    }
    const BATCH: usize = 64;
    let mut pointers = [[0; 8]; BATCH];
    let mut ids = [[0; 4]; BATCH];
    for (batch, output) in output.chunks_mut(BATCH).enumerate() {
        let count = output.len();
        let first = batch * BATCH;
        let addresses: [usize; BATCH] =
            std::array::from_fn(|index| source.element((first + index) as u32, 8).unwrap_or(0));
        let mut fields = pointers.iter_mut().enumerate();
        let copied = memory
            .copy_scattered(
                addresses,
                &mut std::array::from_fn(|_| {
                    let (index, bytes) = fields.next().unwrap();
                    &mut bytes[..if index < count { 8 } else { 0 }]
                }),
            )
            .min(count);
        let addresses: [usize; BATCH] = std::array::from_fn(|index| {
            let pointer = u64::from_le_bytes(pointers[index]) as usize;
            if pointer == 0 {
                0
            } else {
                pointer.checked_add(ID).unwrap_or(0)
            }
        });
        // The states are scattered objects: hint every id's line first so
        // their misses overlap. Hints read nothing.
        let mut fields = ids.iter_mut().enumerate();
        let read_ids = memory
            .copy_scattered_prefetched(
                addresses,
                &mut std::array::from_fn(|_| {
                    let (index, bytes) = fields.next().unwrap();
                    let nonnull = pointers[index] != [0; 8];
                    &mut bytes[..if index < copied && nonnull { 4 } else { 0 }]
                }),
            )
            .min(copied);
        for index in 0..read_ids {
            output[index] = if pointers[index] == [0; 8] {
                0
            } else {
                u32::from_le_bytes(ids[index])
            };
        }
        if read_ids < copied {
            return Err(id_error);
        }
        if copied < count {
            return Err(pointer_error);
        }
    }
    Ok(())
}

fn movement_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("state movement input unreadable")
}

fn movement_state(memory: &impl Memory, base: usize, state: usize) -> Result<(), &'static str> {
    movement_state_vtable(base, state, read::<u64>(memory, state))
}

/// The identity check over an already acquired vtable read.
fn movement_state_vtable(
    base: usize,
    state: usize,
    vtable: Option<u64>,
) -> Result<(), &'static str> {
    if state == 0 || vtable != base.checked_add(VTABLE_RVA).map(|address| address as u64) {
        return Err("state movement identity invalid");
    }
    Ok(())
}

/// A gathered field, consulted where `movement_field` would read it.
fn movement_batched<T: crate::memory::Raw, const N: usize>(
    memory: &impl Memory,
    fields: &FieldBatch<N>,
    index: usize,
) -> Result<T, &'static str> {
    fields
        .get(memory, index)
        .ok_or("state movement input unreadable")
}

/// Fields of `pointer` in one guarded call; an overflowing one fails there.
fn movement_fields<const N: usize>(
    memory: &impl Memory,
    fields: [(usize, usize, usize); N],
) -> FieldBatch<N> {
    FieldBatch::gather(
        memory,
        fields.map(|(pointer, offset, len)| (pointer.checked_add(offset).unwrap_or(0), len)),
    )
}

/// Country-specific numeric modifier uses its override or the state base.
/// The selector flag only changes optional tooltip formatting; passive reads
/// need no game call or mutation.
pub fn movement_modifier(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: usize,
    owner_raw: u32,
    key: u32,
) -> Result<i64, &'static str> {
    movement_modifier_with(
        memory,
        base,
        game_state,
        state,
        owner_raw,
        key,
        |root, key| {
            Ok(
                crate::modifier_accumulator::lookup_effective_at(memory, base, root, key)?
                    .effective,
            )
        },
    )
}

/// Same selecting controls as movement_modifier, with caller-owned numeric
/// observations/metadata. The callback cannot change native selection.
#[allow(clippy::too_many_arguments)]
pub fn movement_modifier_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: usize,
    owner_raw: u32,
    key: u32,
    mut effective: impl FnMut(usize, u32) -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    let linked = game_state != 0 && owner_raw != 0;
    // The state's identity, its hash header and, for a linked owner, the
    // country links header are one guarded call.
    let heads = FieldBatch::gather(
        memory,
        [
            (state, 8),
            (state.checked_add(COUNTRY_MODIFIERS).unwrap_or(0), 17),
            (
                game_state
                    .checked_add(cgame_state::COUNTRY_LINKS)
                    .filter(|_| linked)
                    .unwrap_or(0),
                16,
            ),
        ],
    );
    movement_state_vtable(base, state, heads.get(memory, 0))?;
    let links = if linked {
        Some(
            game_state
                .checked_add(cgame_state::COUNTRY_LINKS)
                .and_then(|_| heads.get(memory, 2))
                .and_then(|header| Array::from_header(header, 4))
                .ok_or("state movement country links invalid")?,
        )
    } else {
        None
    };
    state
        .checked_add(COUNTRY_MODIFIERS)
        .ok_or("state movement hash address overflow")?;
    let header = heads
        .get::<[u8; 17]>(memory, 1)
        .ok_or("state movement hash header unreadable")?;
    let accumulator = probe_country_modifiers(memory, state, owner_raw, links, header)?;
    effective(accumulator, key)
}

/// Authenticated state accumulator for the country: selected bucket
/// accumulator or base accumulator.
pub fn country_modifier_accumulator(
    memory: &impl Memory,
    game_state: usize,
    state: usize,
    owner_raw: u32,
) -> Result<usize, &'static str> {
    let links = if game_state == 0 || owner_raw == 0 {
        None
    } else {
        Some(
            game_state
                .checked_add(cgame_state::COUNTRY_LINKS)
                .and_then(|address| Array::read(memory, address, 4))
                .ok_or("state movement country links invalid")?,
        )
    };
    let header_address = state
        .checked_add(COUNTRY_MODIFIERS)
        .ok_or("state movement hash address overflow")?;
    let header =
        read::<[u8; 17]>(memory, header_address).ok_or("state movement hash header unreadable")?;
    probe_country_modifiers(memory, state, owner_raw, links, header)
}

/// Country-modifier probe uses hash data, mask, maximum distance and linked
/// country identity.
fn probe_country_modifiers(
    memory: &impl Memory,
    state: usize,
    owner_raw: u32,
    links: Option<Array>,
    header: [u8; 17],
) -> Result<usize, &'static str> {
    let canonical = |raw: u32| -> Result<u32, &'static str> {
        match links {
            Some(links) => links
                .element(raw, 4)
                .and_then(|address| read(memory, address))
                .ok_or("state movement country link unreadable"),
            None => Ok(raw),
        }
    };
    country_modifier_accumulator_with_canonical(memory, state, owner_raw, header, canonical)
}

/// Probe using an already copied state hash header and authenticated country
/// links. Worker censuses can reuse their country mapping and state identity.
pub fn country_modifier_accumulator_with_canonical(
    memory: &impl Memory,
    state: usize,
    owner_raw: u32,
    header: [u8; 17],
    mut canonical: impl FnMut(u32) -> Result<u32, &'static str>,
) -> Result<usize, &'static str> {
    let hash = canonical(owner_raw)?;
    let data = u64::from_le_bytes(header[..8].try_into().expect("eight bytes")) as usize;
    let mask = u32::from_le_bytes(header[12..16].try_into().expect("four bytes"));
    let buckets = mask
        .checked_add(1)
        .filter(|count| count.is_power_of_two() && *count <= i32::MAX as u32)
        .ok_or("state movement hash mask invalid")?;
    let sentinel = buckets
        .checked_add(u32::from(header[16]))
        .ok_or("state movement hash extent invalid")?;
    let bucket_address = |index: u32| {
        if data == 0 || index >= sentinel {
            return Err("state movement hash probe outside allocation");
        }
        data.checked_add(index as usize * COUNTRY_MODIFIER_STRIDE)
            .ok_or("state movement hash bucket address overflow")
    };
    let mut index = hash & mask;
    let mut probe = 1_u16;
    let mut selected = None;
    loop {
        let bucket = bucket_address(index)?;
        let distance = movement_field::<u8>(memory, bucket, 4)?;
        if distance == 0 || probe > u16::from(distance) {
            break;
        }
        let raw = movement_field::<u32>(memory, bucket, 8)?;
        if raw == owner_raw || (raw != 0 && owner_raw != 0 && canonical(raw)? == hash) {
            selected = Some(bucket);
            break;
        }
        index = index
            .checked_add(1)
            .ok_or("state movement hash probe overflow")?;
        probe += 1;
        if probe > u16::from(u8::MAX) {
            return Err("state movement hash probe distance invalid");
        }
    }
    match selected {
        Some(bucket) => bucket.checked_add(0x20),
        None => state.checked_add(BASE_MODIFIERS),
    }
    .ok_or("state movement modifier address overflow")
}

/// Resource infrastructure modifier retains signed widths, wrapping multiplies
/// and -0.95 floor. Healthy infrastructure levels contribute rather than total levels.
pub fn movement_infrastructure(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<i64, &'static str> {
    movement_infrastructure_shared(memory, base, state, &mut None)
}

/// Successful global operands from this infrastructure pass only. The ordinal
/// remains absent until a state actually needs the native index lookup.
#[derive(Clone, Copy)]
struct InfrastructureInputs {
    definition: usize,
    maximum: i32,
    lookup_enabled: u8,
    ordinal: Option<u32>,
    per_level: i64,
}

fn movement_infrastructure_shared(
    memory: &impl Memory,
    base: usize,
    state: usize,
    shared: &mut Option<InfrastructureInputs>,
) -> Result<i64, &'static str> {
    use crate::{cbuilding, cbuilding_database, cbuilding_status, cbuilding_template};
    // A shared pass knows the lookup flag; a first pass gathered it and reads
    // it at the same point either way.
    enum Lookup {
        Known(u8),
        Gathered(FieldBatch<2>),
    }
    let (definition, maximum, lookup) = if let Some(inputs) = shared {
        movement_state(memory, base, state)?;
        (
            inputs.definition,
            inputs.maximum,
            Lookup::Known(inputs.lookup_enabled),
        )
    } else {
        // The state's identity and the database root are one guarded call.
        let heads = movement_fields(
            memory,
            [(state, 0, 8), (base, cbuilding_database::SINGLETON_RVA, 8)],
        );
        movement_state_vtable(base, state, heads.get(memory, 0))?;
        let database = movement_batched::<u64, 2>(memory, &heads, 1)? as usize;
        if database == 0 {
            return Err("state movement building database null");
        }
        // Select the infrastructure definition from its database field.
        let definition = movement_field::<u64>(memory, database, 0x3b0)? as usize;
        if definition == 0 {
            return Err("state movement infrastructure definition null");
        }
        // The maximum and the lookup flag are one guarded call.
        let fields = movement_fields(
            memory,
            [
                (definition, cbuilding_template::SCOPE, 4),
                (definition, cbuilding_template::LOOKUP_ENABLED_CANDIDATE, 1),
            ],
        );
        // Select the first local level scale, then the second, then the loaded default.
        let mut maximum = movement_batched::<i32, 2>(memory, &fields, 0)?;
        if maximum < 1 {
            maximum = movement_field::<i32>(memory, definition, 0x2b8)?;
            if maximum < 1 {
                maximum = movement_field::<i32>(memory, base, 0x033e_5274)?;
            }
        }
        (definition, maximum, Lookup::Gathered(fields))
    };
    let status = state
        .checked_add(BUILDING_STATUS)
        .ok_or("state movement building status overflow")?;
    let mut level = 0_i32;
    let lookup_enabled = match lookup {
        Lookup::Known(flag) => flag,
        Lookup::Gathered(fields) => movement_batched::<u8, 2>(memory, &fields, 1)?,
    };
    let mut ordinal = shared.as_ref().and_then(|inputs| inputs.ordinal);
    if lookup_enabled != 0
        && movement_field::<i32>(memory, status, cbuilding_status::BUILDING_INDEX_COUNT)? != 0
    {
        // The index header and, on a first lookup, the ordinal: one call.
        // A known ordinal's address is null, so the call ends before it.
        let ordinal_at = match ordinal {
            Some(_) => 0,
            None => definition
                .checked_add(cbuilding_template::DATABASE_ORDINAL)
                .unwrap_or(0),
        };
        let fields = FieldBatch::gather(
            memory,
            [
                (
                    status
                        .checked_add(cbuilding_status::BUILDING_INDICES)
                        .unwrap_or(0),
                    16,
                ),
                (ordinal_at, 4),
            ],
        );
        let indices = status
            .checked_add(cbuilding_status::BUILDING_INDICES)
            .and_then(|_| fields.get(memory, 0))
            .and_then(|header| Array::from_header(header, 4))
            .ok_or("state movement building indices invalid")?;
        let selected = match ordinal {
            Some(ordinal) => ordinal,
            None => movement_batched::<u32, 2>(memory, &fields, 1)?,
        };
        ordinal = Some(selected);
        let index = indices
            .element(selected, 4)
            .and_then(|address| read::<i32>(memory, address))
            .ok_or("state movement building ordinal outside indices")?;
        if index != cbuilding_status::NO_BUILDING_INDEX {
            let buildings = status
                .checked_add(cbuilding_status::BUILDINGS)
                .and_then(|address| Array::read(memory, address, 8))
                .ok_or("state movement buildings invalid")?;
            let building = u32::try_from(index)
                .ok()
                .and_then(|index| buildings.element(index, 8))
                .and_then(|address| read::<u64>(memory, address))
                .ok_or("state movement building index outside array")?
                as usize;
            if building != 0 {
                let fields = movement_fields(
                    memory,
                    [
                        (building, cbuilding::TEMPLATE, 8),
                        (building, cbuilding::HEALTHY_LEVEL, 2),
                    ],
                );
                if movement_batched::<u64, 2>(memory, &fields, 0)? != definition as u64 {
                    return Err("state movement infrastructure template mismatch");
                }
                level = i32::from(movement_batched::<i16, 2>(memory, &fields, 1)?);
            }
        }
    }
    let per_level = match shared {
        Some(inputs) => inputs.per_level,
        None => movement_field::<i64>(memory, base, 0x033e_5080)?,
    };
    // A failed state read never installs a partial set of shared operands.
    *shared = Some(InfrastructureInputs {
        definition,
        maximum,
        lookup_enabled,
        ordinal,
        per_level,
    });
    Ok(infrastructure_value(maximum, level, per_level))
}

fn infrastructure_value(maximum: i32, level: i32, per_level: i64) -> i64 {
    (i64::from(maximum.wrapping_sub(level))
        .wrapping_mul(per_level)
        .wrapping_mul(100_000)
        / 100_000)
        .max(-95_000)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub infrastructure: Option<infrastructure_map::Infrastructure>,
    pub id: u32,
    pub owner: u32,
    pub controller: u32,
    /// Current hourly fixed5 infrastructure term. Staggered StateBuildings
    /// cannot supply this movement operand.
    pub movement_infrastructure: i64,
    /// Stored available state manpower.
    pub manpower_available: i32,
    pub manpower_locked: i32,
    /// Stored state population ([`POPULATION`]).
    pub population: i32,
    pub best_vp_index: Option<i32>,
    /// The token of the state's category definition ([`CATEGORY`]); None is a
    /// failed independent capture.
    pub category: Option<u32>,
    /// The exact bytes of [`PRIMARY_DISPLAY_NAME`] and
    /// [`SECONDARY_DISPLAY_NAME`] in the caller's name arena ([`read_names`]).
    pub custom_name: NameSpan,
    pub owner_name: NameSpan,
    /// Cached controller-group count equals one; None is an independent refusal.
    pub single_controller: Option<bool>,
    /// Retained state-template continent token, zero for the native null object; None is a failed
    /// independent capture, never a guessed content value.
    pub continent: Option<u32>,
}

/// A string's bytes in a caller-owned arena; an empty string is valid.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NameSpan {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    /// Bytes copied for the returned rows.
    pub bytes_len: usize,
    /// Both strings' native lengths over every census row, including rows
    /// omitted for the arena's capacity, so the spool thread can grow it.
    pub required_bytes: usize,
}

/// Authenticated callback-scoped state identity, never a replay field.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub pointer: usize,
    pub id: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FlagRow {
    pub state: u32,
    pub demilitarized: u8,
    pub border_conflict_active: u8,
    pub border_conflict_countries: Vec<[i32; 2]>,
}

/// Read the ID-indexed state array. Country references remain raw, including
/// zero and aliases.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_with(memory, base, game_state, output, |_| {})
}

/// Both country tags follow the identity gates. Keep their separate spans so
/// a failed controller never changes the owner-first failure or copied prefix.
pub fn read_country_tags(memory: &impl Memory, pointer: usize) -> Result<[u32; 2], &'static str> {
    let owner = pointer.checked_add(OWNER).ok_or("state address overflow")?;
    let Some(controller) = pointer.checked_add(CONTROLLER) else {
        // Preserve the scalar prefix before a later address overflow. Never
        // submit an invented null source or retry a failed field.
        read::<u32>(memory, owner).ok_or("state owner unreadable")?;
        return Err("state address overflow");
    };
    let mut tags = [[0; 4]; 2];
    let copied = memory.copy_scattered(
        [owner, controller],
        &mut tags.each_mut().map(|tag| tag.as_mut_slice()),
    );
    if copied != 2 {
        return Err(if copied == 0 {
            "state owner unreadable"
        } else {
            "state controller unreadable"
        });
    }
    Ok(tags.map(u32::from_le_bytes))
}

/// The visitor runs only for complete authenticated rows, inside this census.
/// Its sources may be consumed only before this hourly callback returns.
pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    visit: impl FnMut(Source),
) -> Outcome {
    walk_with_buffer(memory, base, game_state, walk::Buffer::Fixed(output), visit)
}

pub fn walk_with_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    visit: impl FnMut(Source),
) -> Outcome {
    walk_with_buffer(
        memory,
        base,
        game_state,
        walk::Buffer::Growing(output),
        visit,
    )
}

fn walk_with_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: walk::Buffer<'_, Row>,
    mut visit: impl FnMut(Source),
) -> Outcome {
    let Some(array) = game_state
        .checked_add(cgame_state::STATES)
        .and_then(|p| Array::read(memory, p, 8))
    else {
        return Outcome::Unavailable {
            reason: "state array unreadable or invalid",
        };
    };
    if array.count < 2 {
        return Outcome::Unavailable {
            reason: "state table has no state slots",
        };
    }
    // World reset sizes slots from the live state database.
    let outcome = walk::pointers_buffer(memory, array, output, |index, pointer| {
        let at = |offset| pointer.checked_add(offset).ok_or("state address overflow");
        let vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(memory, pointer) != Some(vtable as u64) {
            return Err("state vtable mismatch");
        }
        let id = read::<u32>(memory, at(ID)?).ok_or("state id unreadable")?;
        if id != index {
            return Err("state id differs from slot");
        }
        let [owner, controller] = read_country_tags(memory, pointer)?;
        visit(Source { pointer, id });
        let manpower = read::<[u8; MANPOWER_SPAN]>(memory, at(MANPOWER_AVAILABLE)?)
            .ok_or("state manpower unreadable")?;
        let word = |offset: usize| {
            let at = offset - MANPOWER_AVAILABLE;
            i32::from_le_bytes(manpower[at..at + 4].try_into().unwrap())
        };
        Ok(Row {
            id,
            owner,
            controller,
            manpower_available: word(MANPOWER_AVAILABLE),
            manpower_locked: word(MANPOWER_LOCKED),
            population: word(POPULATION),
            category: read::<u64>(memory, at(CATEGORY)?)
                .filter(|&category| category != 0)
                .and_then(|category| category_token(memory, base, category).ok()),
            ..Row::default()
        })
    });
    if outcome == (Outcome::Rows { len: 0 }) {
        Outcome::Unavailable {
            reason: "state table has no readable states",
        }
    } else {
        outcome
    }
}

/// Retained template continent tokens read independently of census/name
/// failures. Unassignable passes and province edits preserve the cache.
/// Authenticate the plain continent through its live database ordinal without
/// chosen count limits.
pub fn read_continents(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
) {
    read_continents_selected(memory, base, game_state, census, rows, |_| true);
}

/// [`read_continents`] for the census rows `selected` admits by index; the
/// others keep their values.
pub fn read_continents_selected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
    selected: impl Fn(usize) -> bool,
) {
    let database = (|| {
        // State database singleton and constructor layout.
        let database = base
            .checked_add(0x033d_d4b8)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())?;
        if database == 0
            || read::<u64>(memory, database)
                != base.checked_add(0x029c_0958).map(|address| address as u64)
        {
            return None;
        }
        let continents = database
            .checked_add(0x58)
            .and_then(|address| Array::read(memory, address, 8))?;
        let null = continents
            .element(0, 8)
            .and_then(|address| read::<u64>(memory, address))?;
        let states = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|address| Array::read(memory, address, 8))?;
        (null != 0).then_some((states, continents, null))
    })();
    for (index, row) in rows.iter_mut().take(census.len()).enumerate() {
        if !selected(index) {
            continue;
        }
        row.continent = (|| {
            let (states, continents, null) = database?;
            let state = states
                .element(row.id, 8)
                .and_then(|address| read::<u64>(memory, address))? as usize;
            let template = state
                .checked_add(TEMPLATE)
                .and_then(|address| read::<u64>(memory, address))?
                as usize;
            if state == 0
                || template == 0
                || read::<u64>(memory, template)
                    != base
                        .checked_add(crate::cstate_template::VTABLE_RVA)
                        .map(|address| address as u64)
            {
                return None;
            }
            let pointer = template
                .checked_add(crate::cstate_template::CONTINENT)
                .and_then(|address| read::<u64>(memory, address))?;
            if pointer == null {
                return Some(0);
            }
            if pointer == 0 {
                return None;
            }
            let bytes = (pointer as usize)
                .checked_add(0x28)
                .and_then(|address| read::<[u8; 12]>(memory, address))?;
            let token = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
            let ordinal = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
            if bytes[0] == 0
                || token == 0
                || ordinal == 0
                || continents
                    .element(ordinal, 8)
                    .and_then(|address| read::<u64>(memory, address))
                    != Some(pointer)
            {
                return None;
            }
            Some(token)
        })();
    }
}

/// Fills the hourly movement infrastructure term over an already-authenticated
/// state census. It is a second pass because it chases the building database
/// rather than the state's own fields. A fault denies the States table; the
/// independently captured StateFlags table keeps the authenticated census.
pub fn read_movement_infrastructure(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
) -> Outcome {
    read_movement_infrastructure_selected(memory, base, game_state, census, rows, |_| true)
}

/// [`read_movement_infrastructure`] for the census rows `selected` admits by
/// index; the others keep their values.
pub fn read_movement_infrastructure_selected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
    selected: impl Fn(usize) -> bool,
) -> Outcome {
    let result = (|| {
        if let Outcome::Unavailable { reason } = census {
            return Err(reason);
        }
        let rows = rows
            .get_mut(..census.len())
            .ok_or("state row count exceeds buffer")?;
        let array = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|address| Array::read(memory, address, 8))
            .filter(|array| array.count >= 2)
            .ok_or("state movement root invalid")?;
        let mut infrastructure = None;
        for (index, row) in rows.iter_mut().enumerate() {
            if !selected(index) {
                continue;
            }
            let pointer = array
                .element(row.id, 8)
                .and_then(|address| read::<u64>(memory, address))
                .ok_or("state movement slot outside census")? as usize;
            row.movement_infrastructure =
                movement_infrastructure_shared(memory, base, pointer, &mut infrastructure)?;
        }
        Ok(census)
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// The two display-name strings at [`PRIMARY_DISPLAY_NAME`] and [`SECONDARY_DISPLAY_NAME`] over an
/// already-authenticated census, copied exactly into preallocated storage without an engine call or
/// allocation. One read covers both headers (0xD0 bytes from `PRIMARY_DISPLAY_NAME`); a heap string
/// adds its terminator and payload reads. A row whose strings do not fit ends the returned prefix
/// (the rest are omitted rows); `required_bytes` counts every row so the arena grows off the game
/// thread. A fault denies the table.
pub fn read_names(
    memory: &impl Memory,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
    bytes: &mut [u8],
) -> NameWalk {
    read_names_carried(
        memory,
        game_state,
        census,
        rows,
        walk::Buffer::Fixed(bytes),
        |_| None,
    )
}

#[cfg(test)]
pub fn read_names_growing(
    memory: &impl Memory,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
    bytes: &mut Vec<u8>,
) -> NameWalk {
    read_names_carried(
        memory,
        game_state,
        census,
        rows,
        walk::Buffer::Growing(bytes),
        |_| None,
    )
}

/// [`read_names`] where `carried` supplies, by row index, the custom and
/// owner name bytes of a row the caller proves current: they are copied into
/// the arena at the row's place without reading the state, so the arena and
/// every span equal a fresh read's.
pub fn read_names_carried<'h>(
    memory: &impl Memory,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
    bytes: walk::Buffer<'_, u8>,
    carried: impl FnMut(usize) -> Option<([&'h [u8]; 2], Option<bool>, Option<i32>)>,
) -> NameWalk {
    read_names_carried_scripted(memory, game_state, census, rows, bytes, carried, |_| {
        (false, false)
    })
}

pub fn read_names_carried_scripted<'h>(
    memory: &impl Memory,
    game_state: usize,
    census: Outcome,
    rows: &mut [Row],
    mut bytes: walk::Buffer<'_, u8>,
    mut carried: impl FnMut(usize) -> Option<([&'h [u8]; 2], Option<bool>, Option<i32>)>,
    scripted: impl Fn(u32) -> (bool, bool),
) -> NameWalk {
    use crate::msvc_string::{Descriptor, STRIDE};
    const SPAN: usize = SECONDARY_DISPLAY_NAME - PRIMARY_DISPLAY_NAME + STRIDE;
    let mut bytes_len = 0_usize;
    let mut required_bytes = 0_usize;
    let result = (|| {
        let (len, unread) = match census {
            Outcome::Unavailable { reason } => return Err(reason),
            Outcome::Rows { len } => (len, 0),
            Outcome::Truncated { len, unread_rows } => (len, unread_rows),
        };
        let rows = rows
            .get_mut(..len)
            .ok_or("state row count exceeds buffer")?;
        let array = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|address| Array::read(memory, address, 8))
            .filter(|array| array.count >= 2)
            .ok_or("state name root invalid")?;
        let mut kept = len;
        for (index, row) in rows.iter_mut().enumerate() {
            if let Some(([custom, owner], single_controller, best_vp_index)) = carried(index) {
                row.best_vp_index = best_vp_index;
                row.single_controller = single_controller;
                let wanted = custom
                    .len()
                    .checked_add(owner.len())
                    .ok_or("state name size overflow")?;
                required_bytes = required_bytes
                    .checked_add(wanted)
                    .ok_or("state name size overflow")?;
                bytes.ensure(required_bytes)?;
                if kept < len || wanted > bytes.len() - bytes_len {
                    kept = kept.min(index);
                    continue;
                }
                for (source, span) in [(custom, &mut row.custom_name), (owner, &mut row.owner_name)]
                {
                    bytes[bytes_len..bytes_len + source.len()].copy_from_slice(source);
                    *span = NameSpan {
                        start: bytes_len,
                        len: source.len(),
                    };
                    bytes_len += source.len();
                }
                continue;
            }
            let pointer = array
                .element(row.id, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|pointer| *pointer != 0)
                .ok_or("state name slot outside census")?;
            let at = pointer
                .checked_add(PRIMARY_DISPLAY_NAME)
                .ok_or("state name address overflow")?;
            let headers = read::<[u8; SPAN]>(memory, at).ok_or("state name headers unreadable")?;
            let header = |offset: usize| -> [u8; STRIDE] {
                headers[offset..offset + STRIDE].try_into().unwrap()
            };
            let custom = Descriptor::from_header(memory, at, header(0))?;
            let owner = Descriptor::from_header(
                memory,
                at + SECONDARY_DISPLAY_NAME - PRIMARY_DISPLAY_NAME,
                header(SECONDARY_DISPLAY_NAME - PRIMARY_DISPLAY_NAME),
            )?;
            let wanted = custom
                .len
                .checked_add(owner.len)
                .ok_or("state name size overflow")?;
            required_bytes = required_bytes
                .checked_add(wanted)
                .ok_or("state name size overflow")?;
            // Display naming selects these retained strings.
            bytes.ensure(required_bytes)?;
            if kept < len || wanted > bytes.len() - bytes_len {
                kept = kept.min(index);
                continue;
            }
            let mut span = |descriptor: &Descriptor| -> Result<NameSpan, &'static str> {
                let start = bytes_len;
                descriptor.copy(memory, &mut bytes[start..])?;
                bytes_len += descriptor.len;
                Ok(NameSpan {
                    start,
                    len: descriptor.len,
                })
            };
            row.custom_name = span(&custom)?;
            row.owner_name = span(&owner)?;
            let (victory_point, controllers) = scripted(row.id);
            row.best_vp_index = if victory_point {
                pointer
                    .checked_add(BEST_VP_INDEX)
                    .and_then(|at| read::<i32>(memory, at))
            } else {
                None
            };
            row.single_controller = if controllers {
                pointer
                    .checked_add(CONTROLLER_GROUP_COUNT)
                    .and_then(|at| read::<i32>(memory, at))
                    .map(|count| count == 1)
            } else {
                None
            };
        }
        let omitted = u32::try_from(len - kept)
            .ok()
            .and_then(|n| n.checked_add(unread))
            .ok_or("state name omitted count overflow")?;
        Ok(if omitted == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len: kept,
                unread_rows: omitted,
            }
        })
    })();
    match result {
        Ok(outcome) => NameWalk {
            outcome,
            bytes_len,
            required_bytes,
        },
        Err(reason) => NameWalk {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes,
        },
    }
}

/// Advisory storage demand from retained display strings, without copying text.
pub fn name_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let array = game_state
        .checked_add(cgame_state::STATES)
        .and_then(|address| Array::read(memory, address, 8))
        .filter(|array| array.count >= 2)
        .ok_or("state name demand root invalid")?;
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut required = 0_usize;
    crate::cpdx_array::pointers(
        memory,
        array
            .slice(1, array.count - 1, 8)
            .map_err(|_| "state name demand slots overflow")?,
        "state name demand pointer unreadable",
        |slot, pointer| {
            if pointer == 0 {
                return Ok(());
            }
            if read::<u64>(memory, pointer) != Some(vtable)
                || pointer
                    .checked_add(ID)
                    .and_then(|address| read::<u32>(memory, address))
                    != Some(slot + 1)
            {
                return Err("state name demand identity mismatch");
            }
            // Display naming selects these retained descriptors.
            for offset in [PRIMARY_DISPLAY_NAME, SECONDARY_DISPLAY_NAME] {
                let address = pointer
                    .checked_add(offset)
                    .ok_or("state name demand address overflow")?;
                let descriptor = crate::msvc_string::Descriptor::read(memory, address)?;
                required = required
                    .checked_add(descriptor.len)
                    .ok_or("state name demand size overflow")?;
            }
            Ok(())
        },
    )?;
    Ok(required)
}

/// Acquire the byte flags from this callback's shared census. Source faults and
/// omitted rows remain faults/truncation; no partial census becomes complete.
pub fn read_flags(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[Source],
    parent: Outcome,
    output: &mut [FlagRow],
) -> Outcome {
    let result = (|| {
        let missing = match parent {
            Outcome::Unavailable { reason } => return Err(reason),
            Outcome::Truncated { unread_rows: 0, .. } => {
                return Err("state source truncation has no omitted rows");
            }
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            Outcome::Rows { len: 0 } => return Err("state source has no readable states"),
            Outcome::Rows { .. } => 0,
        };
        let sources = sources
            .get(..parent.len())
            .ok_or("state source count exceeds buffer")?;
        let root = game_state
            .checked_add(cgame_state::STATES)
            .ok_or("state source root overflow")?;
        let array = Array::read(memory, root, 8)
            .filter(|a| a.count >= 2)
            .ok_or("state source root invalid")?;
        let vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut previous = 0;
        let len = sources.len().min(output.len());
        for (source, row) in sources.iter().take(len).zip(output.iter_mut()) {
            if source.id <= previous || source.pointer == 0 {
                return Err("state source identity invalid or duplicate");
            }
            previous = source.id;
            let slot = array
                .element(source.id, 8)
                .ok_or("state source slot outside root")?;
            let id = source
                .pointer
                .checked_add(ID)
                .ok_or("state source identity overflow")?;
            let flag = source
                .pointer
                .checked_add(DEMILITARIZED)
                .ok_or("state flag address overflow")?;
            if read::<u64>(memory, slot) != Some(source.pointer as u64)
                || read::<u64>(memory, source.pointer) != Some(vtable)
                || read::<u32>(memory, id) != Some(source.id)
            {
                return Err("state source identity changed");
            }
            let flags = read::<[u8; 2]>(memory, flag).ok_or("state flag bytes unreadable")?;
            row.state = source.id;
            row.demilitarized = flags[0];
            row.border_conflict_active = flags[1];
            row.border_conflict_countries.clear();
            if flags[1] != 0 {
                border_conflicts::capture(
                    memory,
                    base,
                    source.pointer,
                    &mut row.border_conflict_countries,
                )?;
            }
        }
        let omitted = u32::try_from(sources.len() - len)
            .ok()
            .and_then(|n| n.checked_add(missing))
            .ok_or("state flag omitted count overflow")?;
        Ok(if omitted == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: omitted,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Bytes of [`Globals`].
const GLOBAL_BYTES: usize = 8 + 8 + 4 + 1 + 4 + 4 + 4 + 16 + 8 + 4 + 8 + 16;

/// The operands every state's movement and railway infrastructure terms and continent share, as raw
/// bytes: the building database and its infrastructure definition (held by the database; its type,
/// lookup flag, both maxima and ordinal), the database's template array, the infrastructure
/// movement-penalty and maximum-level defines, and the state database with its continent array.
/// Only loaders write them; a caller holding rows computed from them compares this every hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Globals([u8; GLOBAL_BYTES]);

/// This hour's [`Globals`]; `None` when an infrastructure operand does not
/// read or a pointer is null, which a fresh read refuses. The continent part
/// is optional to a fresh read, so an unreadable one is kept as zero bytes.
pub fn globals(memory: &impl Memory, base: usize) -> Option<Globals> {
    use crate::{cbuilding_database as database, cbuilding_template as template};
    let mut bytes = [0_u8; GLOBAL_BYTES];
    let mut at = 0;
    let mut put = |value: &[u8]| {
        bytes[at..at + value.len()].copy_from_slice(value);
        at += value.len();
    };
    let field = |pointer: usize, offset: usize| pointer.checked_add(offset);
    let building_database = read::<u64>(memory, field(base, database::SINGLETON_RVA)?)
        .filter(|&pointer| pointer != 0)?;
    put(&building_database.to_le_bytes());
    let building_database = usize::try_from(building_database).ok()?;
    let definition =
        read::<u64>(memory, field(building_database, 0x3b0)?).filter(|&pointer| pointer != 0)?;
    put(&definition.to_le_bytes());
    let definition = usize::try_from(definition).ok()?;
    put(&read::<[u8; 4]>(
        memory,
        field(definition, template::TYPE_ID)?,
    )?);
    put(&read::<[u8; 1]>(
        memory,
        field(definition, template::LOOKUP_ENABLED_CANDIDATE)?,
    )?);
    put(&read::<[u8; 4]>(memory, field(definition, 0x2b8)?)?);
    put(&read::<[u8; 4]>(
        memory,
        field(definition, template::SCOPE)?,
    )?);
    put(&read::<[u8; 4]>(
        memory,
        field(definition, template::DATABASE_ORDINAL)?,
    )?);
    put(&read::<[u8; 16]>(
        memory,
        field(building_database, database::TEMPLATES)?,
    )?);
    put(&read::<[u8; 8]>(memory, field(base, 0x033e_5080)?)?);
    put(&read::<[u8; 4]>(memory, field(base, 0x033e_5274)?)?);
    // The state database singleton, as read_continents reads it.
    let state_database = field(base, 0x033d_d4b8)
        .and_then(|address| read::<u64>(memory, address))
        .unwrap_or(0);
    put(&state_database.to_le_bytes());
    put(&usize::try_from(state_database)
        .ok()
        .filter(|&pointer| pointer != 0)
        .and_then(|pointer| field(pointer, 0x58))
        .and_then(|address| read::<[u8; 16]>(memory, address))
        .unwrap_or([0; 16]));
    debug_assert_eq!(at, GLOBAL_BYTES);
    Some(Globals(bytes))
}

/// The raw header of the state-id-indexed array, [`walk`]'s root.
pub fn header(memory: &impl Memory, game_state: usize) -> Option<[u8; 16]> {
    game_state
        .checked_add(cgame_state::STATES)
        .and_then(|address| read(memory, address))
}

const SLOT_LAYOUT: RecordLayout<'static> = RecordLayout::new(
    &[RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }],
    8,
)
.expect("valid slot layout");

/// The pointer of every state slot after the reserved slot zero, in slot
/// order ([`walk`] reads the same slots), in guarded record copies of 64;
/// `output[index]` is slot `index + 1`.
pub fn read_slots(
    memory: &impl Memory,
    array: Array,
    output: &mut [usize],
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    if array.count == 0 || output.len() != array.count as usize - 1 {
        return Err("state slot destination does not match header");
    }
    let mut bytes = [0_u8; BATCH * 8];
    for (batch, output) in output.chunks_mut(BATCH).enumerate() {
        let first = u32::try_from(batch * BATCH + 1).map_err(|_| "table address overflow")?;
        let base = array.element(first, 8).ok_or("table address overflow")?;
        let count = output.len();
        if memory.copy_records(
            RecordSources::Strided {
                base,
                stride: 8,
                count,
            },
            &SLOT_LAYOUT,
            &mut bytes[..count * 8],
        ) != count
        {
            return Err("table pointer unreadable");
        }
        for (pointer, bytes) in output.iter_mut().zip(bytes.chunks_exact(8)) {
            *pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
        }
    }
    Ok(())
}

const MANPOWER_LAYOUT: RecordLayout<'static> = RecordLayout::new(
    &[
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: ID,
            destination_offset: 8,
            len: 4,
        },
        RecordField {
            source_offset: MANPOWER_AVAILABLE,
            destination_offset: 12,
            len: 12,
        },
        RecordField {
            source_offset: CATEGORY,
            destination_offset: 24,
            len: 8,
        },
    ],
    MANPOWER_RECORD,
)
.expect("valid manpower layout");
const MANPOWER_RECORD: usize = 32;

/// A state's stored manpower words and category pointer ([`read_manpower`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Manpower {
    pub available: i32,
    pub locked: i32,
    pub population: i32,
    /// The category definition pointer ([`CATEGORY`]), for [`category_token`].
    pub category: u64,
}

/// A category definition's token: its vtable must be the category vtable.
/// Several states share a definition, so callers resolve each pointer once.
pub fn category_token(
    memory: &impl Memory,
    base: usize,
    category: u64,
) -> Result<u32, &'static str> {
    let vtable = base
        .checked_add(CATEGORY_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let bytes = read::<[u8; CATEGORY_TOKEN + 4]>(memory, category as usize)
        .ok_or("state category unreadable")?;
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != vtable {
        return Err("state category vtable mismatch");
    }
    Ok(u32::from_le_bytes(
        bytes[CATEGORY_TOKEN..CATEGORY_TOKEN + 4]
            .try_into()
            .unwrap(),
    ))
}

/// Each listed state's identity (vtable and id, as [`walk`] checks them),
/// available manpower, population and category pointer, in guarded record
/// copies of 64. A failed or mismatching field fails the whole read.
pub fn read_manpower(
    memory: &impl Memory,
    base: usize,
    pointers: &[usize],
    ids: &[u32],
    output: &mut [Manpower],
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    if pointers.len() != ids.len() || pointers.len() != output.len() {
        return Err("state manpower destination does not match sources");
    }
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut bytes = [0_u8; BATCH * MANPOWER_RECORD];
    for ((pointers, ids), output) in pointers
        .chunks(BATCH)
        .zip(ids.chunks(BATCH))
        .zip(output.chunks_mut(BATCH))
    {
        let count = pointers.len();
        if memory.copy_records(
            RecordSources::Pointers(pointers),
            &MANPOWER_LAYOUT,
            &mut bytes[..count * MANPOWER_RECORD],
        ) != count * 4
        {
            return Err("state manpower unreadable");
        }
        for ((record, &id), output) in bytes
            .chunks_exact(MANPOWER_RECORD)
            .zip(ids)
            .zip(output.iter_mut())
        {
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable {
                return Err("state vtable mismatch");
            }
            if u32::from_le_bytes(record[8..12].try_into().unwrap()) != id {
                return Err("state id differs from slot");
            }
            *output = Manpower {
                available: i32::from_le_bytes(record[12..16].try_into().unwrap()),
                locked: i32::from_le_bytes(record[16..20].try_into().unwrap()),
                population: i32::from_le_bytes(record[20..24].try_into().unwrap()),
                category: u64::from_le_bytes(record[24..32].try_into().unwrap()),
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
