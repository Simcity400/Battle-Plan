//! Simulation inputs for class-13 railway-gun map counters.
//!
//! Every value is a simulation field of the gun, its province or its orders
//! group, read the same for any viewing country or camera. The viewer rebuilds
//! the counter's settled route point, fog admission, stack kind and every
//! counter child from them; no CUnit map entity is read here.

use crate::{
    Memory, cbuilding, ccountry, cgame_state, corders_group,
    cpdx_array::Array,
    cprovince, cprovince_railway_info as rail_info, crailway_gun, crailway_manager,
    creference_registry, csub_unit_definition, cunit, province_definition, read,
    walk::{Buffer, Outcome},
};

/// Stored unit string in the primary object, accepting arbitrary supplied bytes. Serialization uses
/// the secondary receiver and emits its name only when the stored length is nonzero.
pub const NAME: usize = 0x258;

/// First-edge ETA operands from passive speed inputs, separate from the
/// client counter timing cache and durations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EtaOperands {
    pub numerator: i64,
    pub current_speed: i64,
}

mod eta;
mod fresh;

/// Current-hour operands for a freshly searched route. Native transfer skips
/// both land operands; each otherwise retains its own guarded refusal.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FreshEta {
    pub sample_hour: i32,
    pub maximum_speed: Option<i64>,
    pub land_supply_factor: Option<i64>,
    /// Genuine orders-group member link (`carmy::ORDERS_GROUP`), not the rail assignee (`crailway_gun::ASSIGNEE`).
    pub orders_group: Option<u64>,
}

/// One acquisition's attempted common getters, including failures. Prepared
/// per admitted row and cleared before the private roots leave this walk.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FreshEtaObservations {
    pub(super) maximum_speed: Option<Result<i64, &'static str>>,
    pub(super) land_supply_factor: Option<Result<i64, &'static str>>,
}

/// Caller-owned scratch for appended tooltip inputs. Pointers last only for this
/// walk: the whole established counter census is read before these inputs.
pub struct TooltipScratch<'a> {
    pub guns: Buffer<'a, (usize, usize)>,
    pub fresh_observations: Buffer<'a, FreshEtaObservations>,
    /// The accepted callback's simulation hour; absent for legacy-only readers.
    pub sample_hour: Option<i32>,
    pub named: crate::railway_gun_speed::NamedBonuses<'a>,
    /// A completed fixed-layout report is final for the attachment. An
    /// interrupted check is not retained or repeated within this walk.
    pub layout: &'a mut Option<crate::fixed_layout::Report>,
    /// Text scratch prepared from immutable names before capture begins.
    pub layout_buffer: &'a mut [u8],
    pub supply_origins: Buffer<'a, crate::cunit_supply_inputs::Origin>,
    pub supply_origins_len: usize,
    pub required_supply_origins: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub gun: u64,
    /// Raw CUnit owner tags. The viewer resolves aliases through CountryLinks.
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
    pub logical_owner: u32,
    /// Authenticated location; zero for a gun its roster holds unlocated.
    pub province: u32,
    /// A naval-transfer association exists (stack kinds 19/20, actions 7-9).
    pub naval_transfer: bool,
    /// A nonzero transfer word selects action 9 when moving by sea.
    pub transfer_action_status: bool,
    /// Raw exile state, which clears the location warning.
    pub exiled: bool,
    pub retreat: bool,
    pub strategic_redeployment: bool,
    /// Stored count (`crailway_gun::REFERENCE_ENTRY_COUNT`) of its combat references, serialized as
    /// the combat list. A positive count selects action 12 instead of idle 1 when both the ordinary
    /// route and combat vector are empty.
    pub reference_entry_count: i32,
    /// Authenticated full COrdersGroup handle behind `crailway_gun::ASSIGNEE`, or zero for null.
    pub assignee_group: u64,
    /// The stat definition's lexical token; the viewer resolves its name and
    /// counter icon from matching content.
    pub subunit_token: u32,
    /// Raw fixed5 numerator and the per-gun rebuilt maximum of the strength bar.
    pub current_strength: i64,
    pub maximum_strength: i64,
    /// Raw CSupplyConsumer ratio for the gun's low-supply strip.
    pub received_supply_ratio: i64,
    /// Signed level of the railway building at the gun's province, the input
    /// of the location warning. `None` means the rail info or its building
    /// pointer is null, which the warning treats like a nonpositive level.
    pub railway_building_level: Option<i16>,
    /// Authenticated movement target; zero for none.
    pub target: u32,
    /// First combat side for the gun's logical owner: 0 without combat,
    /// 1 attacking, 2 defending.
    pub combat_role: u8,
    /// Province of the first combat; zero without one.
    pub combat_province: u32,
    /// Stored count of the gun's CUnit combat vector, not the saved combat reference array behind
    /// `reference_entry_count`.
    pub combat_count: u32,
    /// The logical owner attacks in any of the gun's combats.
    pub attacking: bool,
    pub ordinary_start: usize,
    pub ordinary_len: usize,
    pub alternate_start: usize,
    pub alternate_len: usize,
    /// Exact bytes in the caller's arena; absent after a field-local refusal.
    pub name: Option<(usize, usize)>,
    /// Present for a positive ordinary route; None outside that demand or
    /// after an isolated acquisition failure.
    pub eta: Option<EtaOperands>,
    /// Exact shared supply-footer pack; origin failures preserve its scalars.
    pub supply: Option<crate::cunit_supply_inputs::Inputs>,
    pub supply_origins_start: usize,
    pub supply_failure: Option<crate::cunit_supply_inputs::Failure>,
    /// Saved CUnit `withdraw` byte; read in the final optional suffix, using
    /// this walk's private roots. Refusal preserves all established fields.
    pub withdrawal: Option<u8>,
    pub fresh: Option<FreshEta>,
    pub fresh_failure: Option<&'static str>,
    /// Maximum-speed, supply-factor and genuine orders-group failures.
    pub fresh_failures: [Option<&'static str>; 3],
    pub combat_references: Option<(usize, usize)>,
    pub combat_references_failure: Option<&'static str>,
}

/// Prepared full-reference output from the existing gun motion visit. Spans
/// publish only for retained rows and complete vectors; discarded/failed spans
/// remain reusable. Pressure includes every header this visit attempted.
pub struct CombatReferencesArena<'a> {
    pub entries: Buffer<'a, cunit::MapCombatReference>,
    pub used: usize,
    pub required: usize,
}

/// Stored text using the native SSO/heap descriptor and caller storage mode.
pub struct NameArena<'a> {
    pub bytes: Buffer<'a, u8>,
    pub used: usize,
    pub required: usize,
    pub failures: usize,
}

impl NameArena<'_> {
    fn capture(&mut self, memory: &impl Memory, gun: usize, keep: bool) -> Option<(usize, usize)> {
        let result = (|| {
            let name = crate::raid_tooltip_operands::StoredString::read(memory, at(gun, NAME)?)
                .map_err(crate::raid_tooltip_operands::NameReadError::reason)?;
            self.required = self
                .required
                .checked_add(name.len)
                .ok_or("railway gun name demand overflow")?;
            if !keep {
                return Ok(None);
            }
            let start = self.used;
            let end = start
                .checked_add(name.len)
                .ok_or("railway gun name arena overflow")?;
            self.bytes.ensure(end)?;
            let output = self
                .bytes
                .get_mut(start..end)
                .ok_or("railway gun name arena too short")?;
            name.copy(memory, output)
                .map_err(crate::raid_tooltip_operands::NameReadError::reason)?;
            self.used = end;
            Ok::<_, &'static str>(Some((start, name.len)))
        })();
        match result {
            Ok(span) => span,
            Err(_) => {
                self.failures = self.failures.saturating_add(1);
                None
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NameDemand {
    pub rows: usize,
    pub bytes: usize,
    pub paths: usize,
    /// Maximum raw named-bonus handle count of any authenticated gun variant.
    /// Optional header faults provide no sizing hint; clocked capture retains
    /// its independent guarded failure and exact arena growth demand.
    pub named_bonuses: usize,
    /// Total origin entries across authenticated guns; optional header faults
    /// provide no hint and never erase the independent name or speed demand.
    pub supply_origins: usize,
    /// Same existing hint census; header-only, no additional vector walk.
    pub combat_references: usize,
}

/// Off-thread presizing: every canonical country's authenticated gun roster,
/// reading string and optional named-bonus/supply-origin headers only. No
/// simulation, route, text, bonus or origin payload is read.
pub fn name_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<NameDemand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut registry = creference_registry::HeaderResolver::default();
    let mut demand = NameDemand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let roster = Array::read(memory, at(country.pointer, ccountry::RAILWAY_UNITS)?, 8)
            .ok_or("railway gun country roster invalid")?;
        for index in 0..roster.count {
            let gun = roster
                .element(index, 8)
                .and_then(|p| pointer_at(memory, p))
                .ok_or("railway gun roster pointer unreadable")?;
            gun_identity(memory, base, gun, &mut registry)?;
            let owner = read::<u32>(memory, at(gun, cunit::ROSTER_OWNER)?)
                .ok_or("railway gun roster owner unreadable")?;
            if countries.resolve(owner)?.canonical_slot != country.canonical_slot {
                return Err("railway gun owner differs from country roster");
            }
            let name = crate::raid_tooltip_operands::StoredString::read(memory, at(gun, NAME)?)
                .map_err(crate::raid_tooltip_operands::NameReadError::reason)?;
            demand.rows = demand
                .rows
                .checked_add(1)
                .ok_or("railway gun row demand overflow")?;
            demand.bytes = demand
                .bytes
                .checked_add(name.len)
                .ok_or("railway gun name demand overflow")?;
            // Unit serialization writes both route vectors.
            for offset in [cunit::ORDINARY_ROUTE_DATA, cunit::ALTERNATE_ROUTE_DATA] {
                if let Some(route) = gun
                    .checked_add(offset)
                    .and_then(|at| Array::read(memory, at, 4))
                {
                    demand.paths = demand
                        .paths
                        .checked_add(route.count as usize)
                        .ok_or("railway gun route hint demand overflow")?;
                }
            }
            if let Ok(count) = eta::named_demand(memory, base, gun, &mut registry) {
                demand.named_bonuses = demand.named_bonuses.max(count);
            }
            if let Some(count) = gun
                .checked_add(cunit::COMBAT_REFERENCE_DATA)
                .and_then(|at| Array::read(memory, at, 8))
                .map(|array| array.count as usize)
            {
                demand.combat_references = demand
                    .combat_references
                    .checked_add(count)
                    .ok_or("railway gun combat-reference hint demand overflow")?;
            }
            if let Ok(count) = crate::cunit_supply_inputs::origin_demand(memory, gun) {
                demand.supply_origins = demand
                    .supply_origins
                    .checked_add(count)
                    .ok_or("railway gun supply origin demand overflow")?;
            }
        }
    }
    Ok(demand)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub paths_len: usize,
    /// Path cells needed to publish every gun this hour, including unread ones.
    pub required_paths: usize,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("railway gun address overflow")
}

fn pointer_at(memory: &impl Memory, address: usize) -> Option<usize> {
    read::<u64>(memory, address).and_then(|p| usize::try_from(p).ok())
}

fn gun_identity(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    registry: &mut creference_registry::HeaderResolver,
) -> Result<u64, &'static str> {
    if gun == 0 {
        return Err("railway gun roster has a null counted slot");
    }
    let (handle, class) = cunit::counter_identity(memory, base, gun, registry)?;
    let secondary = base
        .checked_add(crailway_gun::SECONDARY_VTABLE_RVA)
        .ok_or("railway gun module overflow")? as u64;
    if class != 13 || read::<u64>(memory, at(gun, 0x10)?) != Some(secondary) {
        return Err("railway gun class or secondary identity mismatch");
    }
    Ok(handle)
}

/// The rail-info table the rail manager keeps beside the province roster.
/// Read once per walk, and only when a located gun needs it.
fn rail_slots(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: Array,
) -> Result<Array, &'static str> {
    let manager = pointer_at(memory, at(game_state, cgame_state::RAIL_MANAGER)?)
        .filter(|p| *p != 0)
        .ok_or("railway gun rail manager unreadable or null")?;
    let expected = base
        .checked_add(crailway_manager::VTABLE_RVA)
        .ok_or("railway gun rail manager module overflow")? as u64;
    if read::<u64>(memory, manager) != Some(expected) {
        return Err("railway gun rail manager vtable mismatch");
    }
    Array::read(
        memory,
        at(manager, crailway_manager::PROVINCE_INFO_TABLE)?,
        crailway_manager::PROVINCE_INFO_STRIDE,
    )
    .filter(|rows| rows.count == provinces.count)
    .ok_or("railway gun rail-info table extent mismatch")
}

/// The railway building level through the province's rail information.
/// The rail information and building must both point back at this
/// province's own building status.
fn railway_building_level(
    memory: &impl Memory,
    base: usize,
    slots: Array,
    province: u32,
    province_pointer: usize,
) -> Result<Option<i16>, &'static str> {
    let info = slots
        .element(province, crailway_manager::PROVINCE_INFO_STRIDE)
        .and_then(|p| pointer_at(memory, p))
        .ok_or("railway gun rail-info slot unreadable")?;
    if info == 0 {
        return Ok(None);
    }
    let expected = base
        .checked_add(rail_info::VTABLE_RVA)
        .ok_or("railway gun rail-info module overflow")? as u64;
    if read::<u64>(memory, info) != Some(expected) {
        return Err("railway gun rail-info vtable mismatch");
    }
    let definition = pointer_at(memory, at(province_pointer, cprovince::DEFINITION)?)
        .filter(|p| *p != 0)
        .ok_or("railway gun province definition unreadable or null")?;
    let status = at(province_pointer, cprovince::BUILDING_STATUS)?;
    if read::<u32>(memory, at(definition, province_definition::ID)?) != Some(province)
        || read::<u64>(memory, at(info, rail_info::PROVINCE_RECORD)?) != Some(definition as u64)
        || read::<u64>(memory, at(info, rail_info::BUILDING_STATUS)?) != Some(status as u64)
    {
        return Err("railway gun rail-info province backlink mismatch");
    }
    let building = pointer_at(memory, at(info, rail_info::BUILDING)?)
        .ok_or("railway gun rail building pointer unreadable")?;
    if building == 0 {
        return Ok(None);
    }
    let expected = base
        .checked_add(cbuilding::VTABLE_RVA)
        .ok_or("railway gun rail building module overflow")? as u64;
    if read::<u64>(memory, building) != Some(expected)
        || read::<u64>(memory, at(building, cbuilding::BUILDING_STATUS)?) != Some(status as u64)
    {
        return Err("railway gun rail building identity mismatch");
    }
    read::<i16>(memory, at(building, cbuilding::LEVEL)?)
        .map(Some)
        .ok_or("railway gun rail building level unreadable")
}

fn assignee_group(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    registry: &mut creference_registry::HeaderResolver,
) -> Result<u64, &'static str> {
    let pointer = pointer_at(memory, at(gun, crailway_gun::ASSIGNEE)?)
        .ok_or("railway gun assignee pointer unreadable")?;
    if pointer == 0 {
        return Ok(0);
    }
    let group = pointer
        .checked_sub(crailway_gun::ASSIGNEE_BASE)
        .ok_or("railway gun assignee address underflow")?;
    let expected_secondary = base
        .checked_add(crailway_gun::ORDERS_GROUP_ASSIGNEE_VTABLE_RVA)
        .ok_or("railway gun assignee module overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected_secondary) {
        return Err("railway gun assignee secondary vtable mismatch");
    }
    let handle = corders_group::plan_pointer_handle(memory, base, group)?;
    if handle >> 32 == 0 || registry.resolve(memory, base, handle)? != Some(group) {
        return Err("railway gun assignee registry identity mismatch");
    }
    Ok(handle)
}

fn subunit_stats(
    memory: &impl Memory,
    base: usize,
    gun: usize,
) -> Result<(usize, u32, i64), &'static str> {
    let pointer = pointer_at(memory, at(gun, cunit::STAT_BLOCK)?)
        .filter(|pointer| *pointer != 0)
        .ok_or("railway gun stat block pointer unreadable or null")?;
    let expected = base
        .checked_add(csub_unit_definition::VTABLE_RVA)
        .ok_or("railway gun stat block module overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected) {
        return Err("railway gun stat block vtable mismatch");
    }
    let token = read::<u32>(memory, at(pointer, csub_unit_definition::TOKEN)?)
        .ok_or("railway gun subunit token unreadable")?;
    let maximum = read::<i64>(memory, at(pointer, csub_unit_definition::MAXIMUM_STRENGTH)?)
        .ok_or("railway gun maximum strength unreadable")?;
    Ok((pointer, token, maximum))
}

fn naval_transfer(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    registry: &mut creference_registry::HeaderResolver,
) -> Result<(bool, bool), &'static str> {
    let pointer = pointer_at(memory, at(gun, cunit::NAVAL_TRANSFER_MEMBERSHIP)?)
        .ok_or("railway gun naval transfer unreadable")?;
    let observed = crate::cnaval_unit_transfer::transfer(memory, base, Ok(pointer), |handle| {
        registry.resolve(memory, base, handle)
    })
    .map_err(|reason| match reason {
        "selected transport transfer module overflow" => {
            "railway gun naval transfer module overflow"
        }
        "selected transport transfer vtable mismatch" => {
            "railway gun naval transfer vtable mismatch"
        }
        "selected transport transfer full handle invalid" => {
            "railway gun naval transfer handle invalid"
        }
        "selected transport transfer registry mismatch" => {
            "railway gun naval transfer registry mismatch"
        }
        _ => reason,
    })?;
    match observed {
        crate::cnaval_unit_transfer::Transfer::Absent => Ok((false, false)),
        crate::cnaval_unit_transfer::Transfer::Present { status, .. } => Ok((
            true,
            status.map_err(|_| "railway gun naval transfer action unreadable")? != 0,
        )),
    }
}

fn flag(memory: &impl Memory, address: usize, reason: &'static str) -> Result<bool, &'static str> {
    read::<u8>(memory, address)
        .map(|byte| byte != 0)
        .ok_or(reason)
}

/// Walks each canonical country's native railway-gun roster.
///
/// A null counted roster slot refuses the census. Every unit is checked
/// against its exact class and vtables, the reference registry, its roster
/// country and the province roster. Routes are copied whole and in native
/// order into `paths`. Running out of rows or path cells truncates the table
/// with the exact demand, so the next capture can grow; a gun is never
/// published stationary or with a partial route. The kept rows are not always a
/// prefix of the census: once a long route misses the path arena, a later gun
/// whose route still fits is kept, so a truncated hour holds whole guns in
/// native order with gaps, never a partial one. Row and path capacity are the
/// caller's, sized from observed demand; nothing here caps the census.
///
/// `combats` is the capture's first-combat memo, cleared by the caller for each
/// capture and reserved off the game thread.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    paths: &mut [u32],
    combats: &mut cunit::FirstCombatMemo,
) -> Walk {
    walk_impl(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        Buffer::Fixed(paths),
        combats,
        None,
        None,
        None,
    )
}

#[cfg(test)]
pub fn walk_with_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    paths: &mut [u32],
    combats: &mut cunit::FirstCombatMemo,
    names: &mut NameArena<'_>,
) -> Walk {
    walk_impl(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        Buffer::Fixed(paths),
        combats,
        Some(names),
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn walk_with_tooltip_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    paths: &mut [u32],
    combats: &mut cunit::FirstCombatMemo,
    names: &mut NameArena<'_>,
    tooltip: &mut TooltipScratch<'_>,
) -> Walk {
    walk_impl(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        Buffer::Fixed(paths),
        combats,
        Some(names),
        Some(tooltip),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn walk_with_tooltip_and_combat_references(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    paths: &mut [u32],
    combats: &mut cunit::FirstCombatMemo,
    names: &mut NameArena<'_>,
    tooltip: &mut TooltipScratch<'_>,
    references: &mut CombatReferencesArena<'_>,
) -> Walk {
    walk_impl(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        Buffer::Fixed(paths),
        combats,
        Some(names),
        Some(tooltip),
        Some(references),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_with_tooltip_and_combat_references_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    paths: &mut Vec<u32>,
    combats: &mut cunit::FirstCombatMemo,
    names: &mut NameArena<'_>,
    tooltip: &mut TooltipScratch<'_>,
    references: &mut CombatReferencesArena<'_>,
) -> Walk {
    walk_impl(
        memory,
        base,
        game_state,
        Buffer::Growing(output),
        Buffer::Growing(paths),
        combats,
        Some(names),
        Some(tooltip),
        Some(references),
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_impl(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, Row>,
    mut paths: Buffer<'_, u32>,
    combats: &mut cunit::FirstCombatMemo,
    mut names: Option<&mut NameArena<'_>>,
    mut tooltip: Option<&mut TooltipScratch<'_>>,
    mut references: Option<&mut CombatReferencesArena<'_>>,
) -> Walk {
    let mut len = 0_usize;
    let mut paths_len = 0_usize;
    let mut required_paths = 0_usize;
    let mut missing = 0_u32;
    let result = (|| -> Result<Outcome, &'static str> {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let provinces = game_state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("railway gun province roster invalid")?;
        let mut registry = creference_registry::HeaderResolver::default();
        let mut rail = None;
        for country_slot in 1..countries.count() {
            let Some(country) = countries.at(country_slot)? else {
                continue;
            };
            let roster = Array::read(memory, at(country.pointer, ccountry::RAILWAY_UNITS)?, 8)
                .ok_or("railway gun country roster invalid")?;
            // Size the complete roster from this original acquired header.
            let required = len
                .checked_add(roster.count as usize)
                .ok_or("railway gun row demand overflow")?;
            output.ensure(required)?;
            if output.is_growing() {
                combats
                    .reserve(required)
                    .map_err(|_| "railway gun combat memo allocation failed")?;
            }
            if let Some(scratch) = &mut tooltip {
                scratch.guns.ensure(required)?;
                scratch.fresh_observations.ensure(required)?;
            }
            for index in 0..roster.count {
                let gun = roster
                    .element(index, 8)
                    .and_then(|p| pointer_at(memory, p))
                    .ok_or("railway gun roster pointer unreadable")?;
                let handle = gun_identity(memory, base, gun, &mut registry)?;
                let owners = read::<[u8; 12]>(memory, at(gun, cunit::ROSTER_OWNER)?)
                    .ok_or("railway gun owner fields unreadable")?;
                let word = |at: usize| u32::from_le_bytes(owners[at..at + 4].try_into().unwrap());
                let (roster_owner, expeditionary_owner, logical_owner) =
                    (word(0), word(4), word(8));
                if countries.resolve(roster_owner)?.canonical_slot != country.canonical_slot {
                    return Err("railway gun owner differs from country roster");
                }
                // A roster can hold an unlocated gun; the province's mixed
                // occupant vector is the map source, so zero is preserved.
                let location = pointer_at(memory, at(gun, cunit::LOCATION)?)
                    .ok_or("railway gun location unreadable")?;
                let province = cunit::registered_province(memory, base, provinces, location)?;
                let mut reference_span = None;
                let mut reference_failure = None;
                let motion = if tooltip
                    .as_ref()
                    .is_some_and(|scratch| scratch.sample_hour.is_some())
                {
                    if let Some(arena) = references.as_mut() {
                        let start = arena.used;
                        let invalid_start = start > arena.entries.len();
                        let mut empty = [];
                        let entries = if invalid_start {
                            Buffer::Fixed(&mut empty)
                        } else {
                            arena.entries.suffix(start)
                        };
                        let mut capture = cunit::CombatReferencesCapture::new_buffer(entries);
                        let result = cunit::read_map_motion_observed(
                            memory,
                            base,
                            game_state,
                            provinces,
                            gun,
                            logical_owner,
                            combats,
                            &mut registry,
                            &mut capture,
                        );
                        match arena.required.checked_add(capture.required) {
                            Some(required) => arena.required = required,
                            None => {
                                arena.required = usize::MAX;
                                capture.refuse("unit combat-reference demand overflow");
                            }
                        }
                        if invalid_start {
                            capture.refuse("unit combat-reference arena offset invalid");
                        }
                        match capture.result() {
                            Ok(len) => reference_span = Some((start, len)),
                            Err(reason) => reference_failure = Some(reason),
                        }
                        result?
                    } else {
                        cunit::read_map_motion(
                            memory,
                            base,
                            game_state,
                            provinces,
                            gun,
                            logical_owner,
                            combats,
                        )?
                    }
                } else {
                    cunit::read_map_motion(
                        memory,
                        base,
                        game_state,
                        provinces,
                        gun,
                        logical_owner,
                        combats,
                    )?
                };
                let (naval_transfer, transfer_action_status) =
                    naval_transfer(memory, base, gun, &mut registry)?;
                let railway_building_level = if province == 0 {
                    None
                } else {
                    let slots = match rail {
                        Some(slots) => slots,
                        None => *rail.insert(rail_slots(memory, base, game_state, provinces)?),
                    };
                    railway_building_level(memory, base, slots, province, location)?
                };
                let exiled = flag(
                    memory,
                    at(gun, cunit::EXILED)?,
                    "railway gun exile byte unreadable",
                )?;
                let retreat = flag(
                    memory,
                    at(gun, cunit::RETREAT)?,
                    "railway gun retreat byte unreadable",
                )?;
                let strategic_redeployment = flag(
                    memory,
                    at(gun, cunit::STRATEGIC_REDEPLOYMENT)?,
                    "railway gun strategic redeployment unreadable",
                )?;
                let reference_entry_count =
                    read::<i32>(memory, at(gun, crailway_gun::REFERENCE_ENTRY_COUNT)?)
                        .ok_or("railway gun reference entry count unreadable")?;
                let assignee_group = assignee_group(memory, base, gun, &mut registry)?;
                let (_, subunit_token, maximum_strength) = subunit_stats(memory, base, gun)?;
                let current_strength =
                    read::<i64>(memory, at(gun, crailway_gun::CURRENT_STRENGTH)?)
                        .ok_or("railway gun current strength unreadable")?;
                let received_supply_ratio =
                    read::<i64>(memory, at(gun, cunit::RECEIVED_SUPPLY_RATIO)?)
                        .ok_or("railway gun received supply ratio unreadable")?;
                let needed = motion.required_paths();
                required_paths = required_paths
                    .checked_add(needed)
                    .ok_or("railway gun path arena overflow")?;
                // Both original movement headers have already been acquired.
                paths.ensure(
                    paths_len
                        .checked_add(needed)
                        .ok_or("railway gun path arena overflow")?,
                )?;
                let keep = len < output.len() && needed <= paths.len().saturating_sub(paths_len);
                if !keep {
                    if let Some(names) = &mut names {
                        let _ = names.capture(memory, gun, false);
                    }
                    missing = missing
                        .checked_add(1)
                        .ok_or("railway gun missing count overflow")?;
                    continue;
                }
                motion.copy_paths(memory, &mut paths[paths_len..paths_len + needed])?;
                // Every established scalar and route copy finishes first. Name
                // failures never change their outcome or withhold this row.
                let name = names
                    .as_mut()
                    .and_then(|names| names.capture(memory, gun, true));
                // Only a retained, established row commits the optional arena.
                if let Some((start, count)) = reference_span
                    && let Some(arena) = references.as_mut()
                {
                    arena.used = start + count;
                }
                output[len] = Row {
                    combat_references: reference_span,
                    combat_references_failure: reference_failure,
                    gun: handle,
                    roster_owner,
                    expeditionary_owner,
                    logical_owner,
                    province,
                    naval_transfer,
                    transfer_action_status,
                    exiled,
                    retreat,
                    strategic_redeployment,
                    reference_entry_count,
                    assignee_group,
                    subunit_token,
                    current_strength,
                    maximum_strength,
                    received_supply_ratio,
                    railway_building_level,
                    target: motion.target,
                    combat_role: motion.combat_role,
                    combat_province: motion.combat_province,
                    combat_count: motion.combat_count,
                    attacking: motion.attacking,
                    ordinary_start: paths_len,
                    ordinary_len: motion.ordinary.count as usize,
                    alternate_start: paths_len + motion.ordinary.count as usize,
                    alternate_len: motion.alternate.count as usize,
                    name,
                    eta: None,
                    supply: None,
                    supply_origins_start: 0,
                    supply_failure: None,
                    withdrawal: None,
                    fresh: None,
                    fresh_failure: None,
                    fresh_failures: [None; 3],
                };
                if let Some(scratch) = &mut tooltip
                    && let Some(pointer) = scratch.guns.get_mut(len)
                {
                    *pointer = (gun, location);
                }
                paths_len += needed;
                len += 1;
            }
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    let outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    // Optional ETA reads follow every established row/route/name. Their
    // failures preserve the complete prefix and the independent naming suffix.
    if let Some(scratch) = tooltip {
        scratch
            .fresh_observations
            .fill(FreshEtaObservations::default());
        let mut registry = creference_registry::HeaderResolver::default();
        let layout = scratch
            .layout
            .map(Ok)
            .map_or_else(std::cell::OnceCell::new, std::cell::OnceCell::from);
        for (index, (row, &source)) in output[..outcome.len()]
            .iter_mut()
            .zip(scratch.guns.iter())
            .enumerate()
        {
            if row.ordinary_len != 0 {
                let mut unretained = FreshEtaObservations::default();
                let observations = scratch
                    .fresh_observations
                    .get_mut(index)
                    .unwrap_or(&mut unretained);
                row.eta = eta::capture(
                    memory,
                    base,
                    game_state,
                    source,
                    row,
                    paths[row.ordinary_start],
                    &mut scratch.named,
                    &mut registry,
                    &layout,
                    scratch.layout_buffer,
                    observations,
                )
                .ok();
            }
        }
        // The supply footer is independent of the counter prefix, stored name
        // and ETA. Reuse the same authenticated roots after those reads finish.
        // A refused origin vector leaves exact scalar footer lines available.
        scratch.supply_origins_len = 0;
        scratch.required_supply_origins = 0;
        for (row, &(gun, _)) in output[..outcome.len()].iter_mut().zip(scratch.guns.iter()) {
            row.supply_origins_start = scratch.supply_origins_len;
            match crate::cunit_supply_inputs::capture_buffer(
                memory,
                gun,
                scratch.supply_origins.suffix(scratch.supply_origins_len),
            ) {
                Ok(inputs) => {
                    let demand = match inputs.origins_len {
                        Ok(len) => {
                            scratch.supply_origins_len += len;
                            len
                        }
                        Err(crate::cunit_supply_inputs::Failure::OriginsCapacity { required }) => {
                            required
                        }
                        Err(_) => 0,
                    };
                    scratch.required_supply_origins =
                        scratch.required_supply_origins.saturating_add(demand);
                    row.supply_failure = inputs.origins_len.err();
                    row.supply = Some(inputs);
                }
                Err(reason) => row.supply_failure = Some(reason),
            }
        }
        // Withdrawal is independent of every counter, name, ETA and supply
        // operand. Read it last, before discarding the private current roots.
        for (row, &(gun, _)) in output[..outcome.len()].iter_mut().zip(scratch.guns.iter()) {
            row.withdrawal = cunit::read_withdrawal(memory, gun);
        }
        // Fresh route operands are independent of first-edge ETA and the
        // supply footer. Reuse every attempted common getter, including Err.
        for (index, (row, &(gun, _))) in output[..outcome.len()]
            .iter_mut()
            .zip(scratch.guns.iter())
            .enumerate()
        {
            let Some(hour) = scratch.sample_hour else {
                row.fresh_failure = Some("railway gun fresh ETA callback hour unavailable");
                continue;
            };
            let Some(observations) = scratch.fresh_observations.get_mut(index) else {
                row.fresh_failure = Some("railway gun fresh ETA observation arena too short");
                continue;
            };
            let (inputs, failures) = fresh::capture(
                memory,
                base,
                game_state,
                gun,
                row,
                hour,
                &mut scratch.named,
                &mut registry,
                &layout,
                scratch.layout_buffer,
                observations,
            );
            row.fresh = Some(inputs);
            row.fresh_failures = failures;
        }
        scratch.guns.fill((0, 0));
        scratch
            .fresh_observations
            .fill(FreshEtaObservations::default());
        scratch.named.pointers.fill(0);
        *scratch.layout = layout.into_inner().and_then(Result::ok);
    }
    Walk {
        outcome,
        paths_len,
        required_paths,
    }
}

#[cfg(test)]
mod tests;
