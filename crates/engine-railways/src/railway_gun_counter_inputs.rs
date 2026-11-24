
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::{Buffer, Outcome}};
use engine_native_parts::{
    cbuilding,
    crailway_gun,
    creference_registry,
    csub_unit_definition,
};
use engine_native::{
    ccountry,
    cgame_state,
    corders_group,
    cprovince::{self, definition as province_definition},
    cunit,
};
use crate::{cprovince_railway_info as rail_info, crailway_manager};

pub const NAME: usize = 0x258;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EtaOperands {
    pub numerator: i64,
    pub current_speed: i64,
}

mod eta;
mod fresh;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FreshEta {
    pub sample_hour: i32,
    pub maximum_speed: Option<i64>,
    pub land_supply_factor: Option<i64>,
    pub orders_group: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FreshEtaObservations {
    pub(super) maximum_speed: Option<Result<i64, &'static str>>,
    pub(super) land_supply_factor: Option<Result<i64, &'static str>>,
}

pub struct TooltipScratch<'a> {
    pub guns: Buffer<'a, (usize, usize)>,
    pub fresh_observations: Buffer<'a, FreshEtaObservations>,
    pub sample_hour: Option<i32>,
    pub named: crate::railway_gun_speed::NamedBonuses<'a>,
    pub layout: &'a mut Option<crate::fixed_layout::Report>,
    pub layout_buffer: &'a mut [u8],
    pub supply_origins: Buffer<'a, engine_native_parts::cunit_supply_inputs::Origin>,
    pub supply_origins_len: usize,
    pub required_supply_origins: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub gun: u64,
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
    pub logical_owner: u32,
    pub province: u32,
    pub naval_transfer: bool,
    pub transfer_action_status: bool,
    pub exiled: bool,
    pub retreat: bool,
    pub strategic_redeployment: bool,
    pub reference_entry_count: i32,
    pub assignee_group: u64,
    pub subunit_token: u32,
    pub current_strength: i64,
    pub maximum_strength: i64,
    pub received_supply_ratio: i64,
    pub railway_building_level: Option<i16>,
    pub target: u32,
    pub combat_role: u8,
    pub combat_province: u32,
    pub combat_count: u32,
    pub attacking: bool,
    pub ordinary_start: usize,
    pub ordinary_len: usize,
    pub alternate_start: usize,
    pub alternate_len: usize,
    pub name: Option<(usize, usize)>,
    pub eta: Option<EtaOperands>,
    pub supply: Option<engine_native_parts::cunit_supply_inputs::Inputs>,
    pub supply_origins_start: usize,
    pub supply_failure: Option<engine_native_parts::cunit_supply_inputs::Failure>,
    pub withdrawal: Option<u8>,
    pub fresh: Option<FreshEta>,
    pub fresh_failure: Option<&'static str>,
    pub fresh_failures: [Option<&'static str>; 3],
    pub combat_references: Option<(usize, usize)>,
    pub combat_references_failure: Option<&'static str>,
    pub theatre: Option<u64>,
    pub queued_actions: Option<i32>,
}

pub struct CombatReferencesArena<'a> {
    pub entries: Buffer<'a, cunit::MapCombatReference>,
    pub used: usize,
    pub required: usize,
}

pub struct NameArena<'a> {
    pub bytes: Buffer<'a, u8>,
    pub used: usize,
    pub required: usize,
    pub failures: usize,
}

impl NameArena<'_> {
    fn capture(&mut self, memory: &impl Memory, gun: usize, keep: bool) -> Option<(usize, usize)> {
        let result = (|| {
            let name = engine_native_air::raid_tooltip_operands::StoredString::read(memory, at(gun, NAME)?)
                .map_err(engine_native_air::raid_tooltip_operands::NameReadError::reason)?;
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
                .map_err(engine_native_air::raid_tooltip_operands::NameReadError::reason)?;
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
    pub named_bonuses: usize,
    pub supply_origins: usize,
    pub combat_references: usize,
}

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
            let name = engine_native_air::raid_tooltip_operands::StoredString::read(memory, at(gun, NAME)?)
                .map_err(engine_native_air::raid_tooltip_operands::NameReadError::reason)?;
            demand.rows = demand
                .rows
                .checked_add(1)
                .ok_or("railway gun row demand overflow")?;
            demand.bytes = demand
                .bytes
                .checked_add(name.len)
                .ok_or("railway gun name demand overflow")?;
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
            if let Ok(count) = engine_native_parts::cunit_supply_inputs::origin_demand(memory, gun) {
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
    pub required_paths: usize,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("railway gun address overflow")
}

fn key(base: usize, key: engine_sites::Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("railway gun address overflow")
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
    let secondary = engine_sites::at(base, crailway_gun::SECONDARY_VTABLE_RVA)
        .ok_or("railway gun module overflow")? as u64;
    if class != 13 || read::<u64>(memory, at(gun, 0x10)?) != Some(secondary) {
        return Err("railway gun class or secondary identity mismatch");
    }
    Ok(handle)
}

fn rail_slots(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    provinces: Array,
) -> Result<Array, &'static str> {
    let manager = pointer_at(memory, at(game_state, cgame_state::RAIL_MANAGER)?)
        .filter(|p| *p != 0)
        .ok_or("railway gun rail manager unreadable or null")?;
    let expected = engine_sites::at(base, crailway_manager::VTABLE_RVA)
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
    let expected = engine_sites::at(base, rail_info::VTABLE_RVA)
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
    let expected = engine_sites::at(base, cbuilding::VTABLE_RVA)
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
    let expected_secondary = engine_sites::at(base, crailway_gun::ORDERS_GROUP_ASSIGNEE_VTABLE_RVA)
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
    let expected = engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
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

#[expect(
    clippy::too_many_arguments,
    reason = "the output, paths, combat memo, names, tooltip scratch and references are separate caller-owned parts of one walk"
)]
pub fn walk_with_tooltip_and_combat_references(
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

#[expect(
    clippy::too_many_arguments,
    reason = "the output, paths, combat memo, names, tooltip scratch and references are separate caller-owned parts of one walk"
)]
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
                        let entries = if invalid_start {
                            Buffer::Count
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
                let theatre = pointer_at(memory, at(gun, cunit::THEATRE)?)
                    .ok_or("railway gun theatre pointer unreadable")
                    .and_then(|pointer| {
                        engine_native::ctheatre::unit_theatre(memory, base, country.pointer, pointer)
                    })
                    .ok();
                let queued_actions = read::<i32>(memory, at(gun, cunit::QUEUED_ACTION_COUNT)?);
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
                let name = names
                    .as_mut()
                    .and_then(|names| names.capture(memory, gun, true));
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
                    theatre,
                    queued_actions,
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
        scratch.supply_origins_len = 0;
        scratch.required_supply_origins = 0;
        for (row, &(gun, _)) in output[..outcome.len()].iter_mut().zip(scratch.guns.iter()) {
            row.supply_origins_start = scratch.supply_origins_len;
            match engine_native_parts::cunit_supply_inputs::capture_buffer(
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
                        Err(engine_native_parts::cunit_supply_inputs::Failure::OriginsCapacity { required }) => {
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
        for (row, &(gun, _)) in output[..outcome.len()].iter_mut().zip(scratch.guns.iter()) {
            row.withdrawal = cunit::read_withdrawal(memory, gun);
        }
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
