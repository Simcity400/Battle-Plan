
use crate::modifier_keys as keys;
use crate::{
    Memory, carmy_group, ccountry, cgame_state, corders_group, cpdx_array::Array, cprovince,
    creferenced_division_template, cunit, read, walk::Outcome,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_6d98);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029d_6f78);
pub const SECONDARY_BASE: usize = 0x10;
pub const COUNTER_SUPPLY_STATUS: usize = 0x40;
pub const INVALID_ORDERS: usize = 0x24e;
pub const STRENGTH_FACTOR: usize = 0x468;
pub const ORDERS_GROUP: usize = 0xc0;
#[cfg(test)]
pub const PRIMARY_ORDER: usize = 0xd8;
pub const MOVEMENT_TARGET: usize = 0x1e8;
pub const MOVEMENT_SOURCE: usize = 0x1f8;
pub const MOVEMENT_NUMERATOR: usize = 0x240;
pub const RETREAT: usize = 0x24c;
pub const STRATEGIC_REDEPLOYMENT: usize = 0x2af;
pub const EXILED: usize = 0x2ae;
pub const NAVAL_TRANSFER_MEMBERSHIP: usize = 0x2f8;
pub const NONCORE_SUPPLY_BYPASS: usize = 0x2e;
pub const AIR_SUPPLY: usize = 0x58;
pub const OUT_OF_SUPPLY_DAYS: usize = 0x498;
pub const DEPLOYMENT_TYPE: usize = 0x32c;
pub const EQUIPMENT_POOL: usize = 0x348;
pub const EQUIPMENT_DATA: usize = 0x368;
pub const STANDARD_TEMPLATE: usize = 0x3b8;
pub const PREVIOUS_TEMPLATE: usize = 0x3c0;
pub const FAKE_INTEL_TEMPLATE: usize = 0x3c8;
pub const MANPOWER_VALUES: usize = 0x3d8;
pub const MANPOWER_DATA: usize = MANPOWER_VALUES + crate::carmy_manpower_values::DATA;
pub const STRENGTH: usize = 0x420;
pub const ORGANISATION: usize = 0x428;
pub const EXPERIENCE: usize = 0x430;
pub const ATTRITION_TOTAL: usize = 0x448;
pub const ENTRENCHMENT: usize = 0x458;
pub const ENTRENCHMENT_CAP: usize = 0x460;
pub const COUNTER_ATTRITION: usize = 0x438;
pub const COUNTER_ATTRITION_THRESHOLD_RVA: Rva = Rva(0x033e_0008);
pub const EXERCISE_MAXIMUM_LEVEL_RVA: Rva = Rva(0x033e_3ff4);
pub const EXERCISE_MINIMUM_STRENGTH_RVA: Rva = Rva(0x033e_40b0);
pub const EXPERIENCE_THRESHOLDS_RVA: Rva = Rva(0x033e_6ec0);
pub const REQUESTS: usize = 0x478;
pub const RETAINED_VISUAL_VARIANT: usize = 0x4b0;
pub const ACCLIMATIZATION: usize = 0x4b8;
pub const MAXIMUM_SUPPLY_GRACE: usize = 0x600;
pub const REMAINING_SUPPLY_GRACE: usize = 0x610;
pub const FUEL_CANDIDATE: usize = 0x620;
pub const SEPARATE_COUNTER: usize = 0x630;

pub const CONSTRUCTOR_RVA: Rva = Rva(0x00cc_a1b0);
pub const HOURLY_UPDATE_RVA: Rva = Rva(0x00ce_4fc0);
pub const UPDATE_ACTIVE_STATS_RVA: Rva = Rva(0x00ce_a3c0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
    pub hit_points: i64,
    pub organisation: i64,
    pub orders_group: u64,
    pub reveal_skip: u8,
    pub theatre: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IconInputsRow {
    pub division: u64,
    pub separate_counter: u8,
    pub alternate_template: u64,
}

pub fn read_icon_inputs(
    memory: &impl Memory,
    base: usize,
    source: crate::cequipment_variant::pool::DivisionSource,
    seed: Option<&DivisionSeed>,
) -> Result<IconInputsRow, &'static str> {
    let at = |offset| {
        source
            .pointer
            .checked_add(offset)
            .ok_or("army icon address overflow")
    };
    let primary = engine_sites::at(base, VTABLE_RVA).ok_or("army icon module address overflow")?;
    let secondary =
        engine_sites::at(base, SECONDARY_VTABLE_RVA).ok_or("army icon module address overflow")?;
    let identity = || {
        let matches = if source.pointer == 0 || source.division == 0 {
            false
        } else if let Some(seed) = seed {
            seed.pointer == source.pointer
                && seed.division == source.division
                && seed.primary_vtable == Some(primary as u64)
                && seed.secondary_vtable == Some(secondary as u64)
                && seed.handle == Some(source.division)
                && seed.roster_owner == Some(source.country)
        } else {
            read::<u64>(memory, source.pointer) == Some(primary as u64)
                && read::<u64>(memory, at(SECONDARY_BASE)?) == Some(secondary as u64)
                && read::<u64>(memory, at(cunit::HANDLE)?) == Some(source.division)
                && read::<u32>(memory, at(cunit::ROSTER_OWNER)?) == Some(source.country)
        };
        if !matches {
            return Err("army icon source identity changed");
        }
        Ok(())
    };
    identity()?;
    let separate_counter = match seed {
        Some(seed) => seed.separate_counter,
        None => read::<u8>(memory, at(SEPARATE_COUNTER)?),
    }
    .ok_or("army separate-counter byte unreadable")?;
    let pointer = match seed {
        Some(seed) => seed.alternate_template,
        None => read::<u64>(memory, at(FAKE_INTEL_TEMPLATE)?)
            .and_then(|pointer| usize::try_from(pointer).ok()),
    }
    .ok_or("army alternate template pointer unreadable")?;
    let alternate_template =
        if pointer != 0 && seed.is_some_and(|seed| seed.standard_template == Some(pointer)) {
            if source.template == 0 {
                return Err("non-null division template has null handle");
            }
            source.template
        } else {
            template_handle(memory, base, pointer)?
        };
    if seed.is_none() {
        identity()?;
    }
    Ok(IconInputsRow {
        division: source.division,
        separate_counter,
        alternate_template,
    })
}

pub fn pending_template(
    memory: &impl Memory,
    base: usize,
    army: usize,
) -> Result<u64, &'static str> {
    let pointer = army
        .checked_add(PREVIOUS_TEMPLATE)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("division previous template pointer unreadable")?;
    template_handle(memory, base, pointer)
}

fn template_handle(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let expected = engine_sites::at(base, creferenced_division_template::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("division template vtable mismatch");
    }
    let handle = pointer
        .checked_add(creferenced_division_template::TYPE)
        .and_then(|address| read::<u64>(memory, address))
        .ok_or("division template handle unreadable")?;
    if handle == 0 {
        Err("non-null division template has null handle")
    } else {
        Ok(handle)
    }
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_with(memory, base, game_state, output, |_, _| {})
}

pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    mut visit: impl FnMut(usize, &Row),
) -> Outcome {
    let result = (|| {
        let array = |offset, stride, reason| {
            game_state
                .checked_add(offset)
                .and_then(|p| Array::read(memory, p, stride))
                .ok_or(reason)
        };
        let reader = Reader {
            memory,
            base,
            countries: array(
                cgame_state::COUNTRIES,
                8,
                "country array unreadable or invalid",
            )?,
            links: array(
                cgame_state::COUNTRY_LINKS,
                4,
                "country links unreadable or invalid",
            )?,
            provinces: array(
                cgame_state::PROVINCES,
                8,
                "province array unreadable or invalid",
            )?,
        };
        if reader.countries.count < 2 {
            return Err("country table has no country slots");
        }
        if reader.links.count == 0 {
            return Err("country links have no sentinel slot");
        }
        if reader.provinces.count < 2 {
            return Err("province table has no province slots");
        }
        let mut len = 0;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        let mut theatres = crate::ctheatre::UnitTheatres::default();
        for slot in 1..reader.countries.count {
            let country = reader.country_pointer(slot)?;
            if country == 0 {
                continue;
            }
            reader.vtable(country, ccountry::VTABLE_RVA, "country vtable mismatch")?;
            let id = country
                .checked_add(ccountry::ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("country id unreadable")?;
            let canonical = reader.canonical(id)?;
            if reader.canonical(slot)? != canonical {
                return Err("country slot and object resolve to different countries");
            }
            if reader.country_pointer(canonical)? != country {
                return Err("canonical country slot points to a different object");
            }
            if slot != canonical {
                continue;
            }
            found_country = true;
            let roster = country
                .checked_add(ccountry::DIVISIONS)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("division roster unreadable or invalid")?;
            for index in 0..roster.count {
                let pointer = roster
                    .element(index, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .ok_or("division roster pointer unreadable")?;
                if pointer == 0 {
                    continue;
                }
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("division unread row count overflow")?;
                } else {
                    output[len] = reader.row(pointer, canonical, country, &mut theatres)?;
                    visit(pointer, &output[len]);
                    len += 1;
                }
            }
        }
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows != 0 {
            Ok(Outcome::Truncated { len, unread_rows })
        } else {
            Ok(Outcome::Rows { len })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionSeed {
    pub pointer: usize,
    pub division: u64,
    pub country_ordinal: Option<u32>,
    pub primary_vtable: Option<u64>,
    pub secondary_vtable: Option<u64>,
    pub handle: Option<u64>,
    pub roster_owner: Option<u32>,
    pub logical_owner: Option<u32>,
    pub expeditionary_owner: Option<u32>,
    pub location: Option<usize>,
    pub standard_template: Option<usize>,
    pub strength: Option<i64>,
    pub organisation: Option<i64>,
    pub orders_group: Option<usize>,
    pub class: Option<u32>,
    pub stat_block: Option<usize>,
    pub ordinary_route: Option<[u8; 16]>,
    pub alternate_route: Option<[u8; 16]>,
    pub combat: Option<[u8; 16]>,
    pub movement_numerator: Option<i64>,
    pub movement_target: Option<usize>,
    pub movement_source: Option<usize>,
    pub naval_transfer_membership: Option<usize>,
    pub retreat: Option<u8>,
    pub strategic_redeployment: Option<u8>,
    pub counter_attrition: Option<i64>,
    pub entrenchment: Option<i64>,
    pub entrenchment_cap: Option<i64>,
    pub counter_supply_status: Option<i64>,
    pub invalid_orders: Option<u8>,
    pub strength_factor: Option<i64>,
    pub model_number: Option<i32>,
    pub acclimatization: Option<[u8; 0x40]>,
    pub map_entity: Option<usize>,
    pub exiled: Option<u8>,
    pub name_member: Option<usize>,
    pub supply_grace_maximum: Option<i64>,
    pub supply_grace_remaining: Option<i64>,
    pub manpower_values: Option<[u8; 24]>,
    pub movement_priority: Option<i32>,
    pub withdrawal: Option<u8>,
    pub alternate_template: Option<usize>,
    pub separate_counter: Option<u8>,
    pub noncore_supply_bypass: Option<u8>,
    pub out_of_supply_days: Option<i32>,
    pub fuel: Option<i64>,
    pub air_supply: Option<i64>,
    pub route_head: Option<Option<u32>>,
    pub location_id: Option<Option<u32>>,
    pub selected: Option<crate::carmy_leader::Selected>,
    pub theatre: Option<usize>,
    pub queued_actions: Option<i32>,
}

impl DivisionSeed {
    pub const PRIMARY_VTABLE: u64 = 1 << 0;
    pub const SECONDARY_VTABLE: u64 = 1 << 1;
    pub const HANDLE: u64 = 1 << 2;
    pub const ROSTER_OWNER: u64 = 1 << 3;
    pub const LOGICAL_OWNER: u64 = 1 << 4;
    pub const EXPEDITIONARY_OWNER: u64 = 1 << 5;
    pub const LOCATION: u64 = 1 << 6;
    pub const STANDARD_TEMPLATE: u64 = 1 << 7;
    pub const STRENGTH: u64 = 1 << 8;
    pub const ORGANISATION: u64 = 1 << 9;
    pub const ORDERS_GROUP: u64 = 1 << 10;
    pub const CLASS: u64 = 1 << 11;
    pub const STAT_BLOCK: u64 = 1 << 12;
    pub const ORDINARY_ROUTE: u64 = 1 << 13;
    pub const ALTERNATE_ROUTE: u64 = 1 << 14;
    pub const COMBAT: u64 = 1 << 15;
    pub const MOVEMENT_NUMERATOR: u64 = 1 << 16;
    pub const MOVEMENT_TARGET: u64 = 1 << 17;
    pub const MOVEMENT_SOURCE: u64 = 1 << 18;
    pub const NAVAL_TRANSFER_MEMBERSHIP: u64 = 1 << 19;
    pub const RETREAT: u64 = 1 << 20;
    pub const STRATEGIC_REDEPLOYMENT: u64 = 1 << 21;
    pub const COUNTER_ATTRITION: u64 = 1 << 22;
    pub const ENTRENCHMENT: u64 = 1 << 23;
    pub const ENTRENCHMENT_CAP: u64 = 1 << 24;
    pub const COUNTER_SUPPLY_STATUS: u64 = 1 << 25;
    pub const INVALID_ORDERS: u64 = 1 << 26;
    pub const STRENGTH_FACTOR: u64 = 1 << 27;
    pub const MODEL_NUMBER: u64 = 1 << 28;
    pub const ACCLIMATIZATION: u64 = 1 << 29;
    pub const MAP_ENTITY: u64 = 1 << 30;
    pub const EXILED: u64 = 1 << 31;
    pub const NAME_MEMBER: u64 = 1 << 32;
    pub const SUPPLY_GRACE_MAXIMUM: u64 = 1 << 33;
    pub const SUPPLY_GRACE_REMAINING: u64 = 1 << 34;
    pub const MANPOWER_VALUES: u64 = 1 << 35;
    pub const MOVEMENT_PRIORITY: u64 = 1 << 36;
    pub const WITHDRAWAL: u64 = 1 << 37;
    pub const ALTERNATE_TEMPLATE: u64 = 1 << 38;
    pub const SEPARATE_COUNTER: u64 = 1 << 39;
    pub const NONCORE_SUPPLY_BYPASS: u64 = 1 << 40;
    pub const OUT_OF_SUPPLY_DAYS: u64 = 1 << 41;
    pub const FUEL: u64 = 1 << 42;
    pub const AIR_SUPPLY: u64 = 1 << 43;
    pub const THEATRE: u64 = 1 << 44;
    pub const QUEUED_ACTIONS: u64 = 1 << 45;
}

const DIVISION_SEED_SPANS: [(usize, usize); 46] = [
    (0, 8),
    (SECONDARY_BASE, 8),
    (cunit::HANDLE, 8),
    (cunit::ROSTER_OWNER, 4),
    (cunit::LOGICAL_OWNER, 4),
    (cunit::EXPEDITIONARY_OWNER, 4),
    (cunit::LOCATION, 8),
    (STANDARD_TEMPLATE, 8),
    (STRENGTH, 8),
    (ORGANISATION, 8),
    (ORDERS_GROUP, 8),
    (cunit::CLASS, 4),
    (cunit::STAT_BLOCK, 8),
    (cunit::ORDINARY_ROUTE_DATA, 16),
    (cunit::ALTERNATE_ROUTE_DATA, 16),
    (cunit::COMBAT_REFERENCE_DATA, 16),
    (MOVEMENT_NUMERATOR, 8),
    (MOVEMENT_TARGET, 8),
    (MOVEMENT_SOURCE, 8),
    (NAVAL_TRANSFER_MEMBERSHIP, 8),
    (RETREAT, 1),
    (STRATEGIC_REDEPLOYMENT, 1),
    (COUNTER_ATTRITION, 8),
    (ENTRENCHMENT, 8),
    (ENTRENCHMENT_CAP, 8),
    (COUNTER_SUPPLY_STATUS, 8),
    (INVALID_ORDERS, 1),
    (STRENGTH_FACTOR, 8),
    (RETAINED_VISUAL_VARIANT, 4),
    (ACCLIMATIZATION, 0x40),
    (cunit::MAP_ENTITY, 8),
    (EXILED, 1),
    (crate::carmy_combat_inputs::NAME_GROUP_MEMBER, 8),
    (MAXIMUM_SUPPLY_GRACE, 8),
    (REMAINING_SUPPLY_GRACE, 8),
    (MANPOWER_VALUES, 24),
    (cunit::MOVEMENT_PRIORITY, 4),
    (cunit::WITHDRAWAL, 1),
    (FAKE_INTEL_TEMPLATE, 8),
    (SEPARATE_COUNTER, 1),
    (NONCORE_SUPPLY_BYPASS, 1),
    (OUT_OF_SUPPLY_DAYS, 4),
    (FUEL_CANDIDATE, 8),
    (AIR_SUPPLY, 8),
    (cunit::THEATRE, 8),
    (cunit::QUEUED_ACTION_COUNT, 4),
];
const DIVISION_SEED_FIELDS: [crate::memory::RecordField; 46] =
    crate::memory::packed_fields(DIVISION_SEED_SPANS);
const DIVISION_SEED_STRIDE: usize = crate::memory::packed_stride(DIVISION_SEED_SPANS);

pub fn capture_division_seeds(memory: &impl Memory, seeds: &mut [DivisionSeed]) -> u64 {
    const BATCH: usize = 64;
    let mut bytes = [0u8; BATCH * DIVISION_SEED_STRIDE];
    let mut pointers = [0usize; BATCH];
    let mut total = 0u64;
    for first in (0..seeds.len()).step_by(BATCH) {
        let next = (first + BATCH).min(seeds.len());
        let chunk = &mut seeds[first..next];
        for (pointer, seed) in pointers.iter_mut().zip(chunk.iter()) {
            *pointer = seed.pointer;
        }
        const LAYOUT: crate::memory::RecordLayout<'static> =
            crate::memory::RecordLayout::new(&DIVISION_SEED_FIELDS, DIVISION_SEED_STRIDE)
                .expect("valid division seed layout");
        let copied = memory.copy_records(
            crate::memory::RecordSources::Pointers(&pointers[..chunk.len()]),
            &LAYOUT,
            &mut bytes,
        );
        for (row, seed) in chunk.iter_mut().enumerate() {
            let mut valid = 0u64;
            let record = &mut bytes[row * DIVISION_SEED_STRIDE..(row + 1) * DIVISION_SEED_STRIDE];
            for (field, layout) in DIVISION_SEED_FIELDS.iter().enumerate() {
                let ordinal = row * DIVISION_SEED_FIELDS.len() + field;
                let success = ordinal < copied
                    || (ordinal > copied
                        && seed
                            .pointer
                            .checked_add(layout.source_offset)
                            .is_some_and(|at| {
                                memory.copy(
                                    at,
                                    &mut record[layout.destination_offset
                                        ..layout.destination_offset + layout.len],
                                )
                            }));
                if success {
                    valid |= 1 << field;
                    total += layout.len as u64;
                }
            }
            seed.primary_vtable = (valid & DivisionSeed::PRIMARY_VTABLE != 0)
                .then(|| u64::from_le_bytes(record[0..8].try_into().unwrap()));
            seed.secondary_vtable = (valid & DivisionSeed::SECONDARY_VTABLE != 0)
                .then(|| u64::from_le_bytes(record[8..16].try_into().unwrap()));
            seed.handle = (valid & DivisionSeed::HANDLE != 0)
                .then(|| u64::from_le_bytes(record[16..24].try_into().unwrap()));
            seed.roster_owner = (valid & DivisionSeed::ROSTER_OWNER != 0)
                .then(|| u32::from_le_bytes(record[24..28].try_into().unwrap()));
            seed.logical_owner = (valid & DivisionSeed::LOGICAL_OWNER != 0)
                .then(|| u32::from_le_bytes(record[28..32].try_into().unwrap()));
            seed.expeditionary_owner = (valid & DivisionSeed::EXPEDITIONARY_OWNER != 0)
                .then(|| u32::from_le_bytes(record[32..36].try_into().unwrap()));
            seed.location = (valid & DivisionSeed::LOCATION != 0)
                .then(|| u64::from_le_bytes(record[36..44].try_into().unwrap()) as usize);
            seed.standard_template = (valid & DivisionSeed::STANDARD_TEMPLATE != 0)
                .then(|| u64::from_le_bytes(record[44..52].try_into().unwrap()) as usize);
            seed.strength = (valid & DivisionSeed::STRENGTH != 0)
                .then(|| i64::from_le_bytes(record[52..60].try_into().unwrap()));
            seed.organisation = (valid & DivisionSeed::ORGANISATION != 0)
                .then(|| i64::from_le_bytes(record[60..68].try_into().unwrap()));
            seed.orders_group = (valid & DivisionSeed::ORDERS_GROUP != 0)
                .then(|| u64::from_le_bytes(record[68..76].try_into().unwrap()) as usize);
            seed.class = (valid & DivisionSeed::CLASS != 0)
                .then(|| u32::from_le_bytes(record[76..80].try_into().unwrap()));
            seed.stat_block = (valid & DivisionSeed::STAT_BLOCK != 0)
                .then(|| u64::from_le_bytes(record[80..88].try_into().unwrap()) as usize);
            seed.ordinary_route = (valid & DivisionSeed::ORDINARY_ROUTE != 0)
                .then(|| record[88..104].try_into().unwrap());
            seed.alternate_route = (valid & DivisionSeed::ALTERNATE_ROUTE != 0)
                .then(|| record[104..120].try_into().unwrap());
            seed.combat =
                (valid & DivisionSeed::COMBAT != 0).then(|| record[120..136].try_into().unwrap());
            seed.movement_numerator = (valid & DivisionSeed::MOVEMENT_NUMERATOR != 0)
                .then(|| i64::from_le_bytes(record[136..144].try_into().unwrap()));
            seed.movement_target = (valid & DivisionSeed::MOVEMENT_TARGET != 0)
                .then(|| u64::from_le_bytes(record[144..152].try_into().unwrap()) as usize);
            seed.movement_source = (valid & DivisionSeed::MOVEMENT_SOURCE != 0)
                .then(|| u64::from_le_bytes(record[152..160].try_into().unwrap()) as usize);
            seed.naval_transfer_membership = (valid & DivisionSeed::NAVAL_TRANSFER_MEMBERSHIP != 0)
                .then(|| u64::from_le_bytes(record[160..168].try_into().unwrap()) as usize);
            seed.retreat = (valid & DivisionSeed::RETREAT != 0).then(|| record[168]);
            seed.strategic_redeployment =
                (valid & DivisionSeed::STRATEGIC_REDEPLOYMENT != 0).then(|| record[169]);
            seed.counter_attrition = (valid & DivisionSeed::COUNTER_ATTRITION != 0)
                .then(|| i64::from_le_bytes(record[170..178].try_into().unwrap()));
            seed.entrenchment = (valid & DivisionSeed::ENTRENCHMENT != 0)
                .then(|| i64::from_le_bytes(record[178..186].try_into().unwrap()));
            seed.entrenchment_cap = (valid & DivisionSeed::ENTRENCHMENT_CAP != 0)
                .then(|| i64::from_le_bytes(record[186..194].try_into().unwrap()));
            seed.counter_supply_status = (valid & DivisionSeed::COUNTER_SUPPLY_STATUS != 0)
                .then(|| i64::from_le_bytes(record[194..202].try_into().unwrap()));
            seed.invalid_orders = (valid & DivisionSeed::INVALID_ORDERS != 0).then(|| record[202]);
            seed.strength_factor = (valid & DivisionSeed::STRENGTH_FACTOR != 0)
                .then(|| i64::from_le_bytes(record[203..211].try_into().unwrap()));
            seed.model_number = (valid & DivisionSeed::MODEL_NUMBER != 0)
                .then(|| i32::from_le_bytes(record[211..215].try_into().unwrap()));
            seed.acclimatization = (valid & DivisionSeed::ACCLIMATIZATION != 0)
                .then(|| record[215..279].try_into().unwrap());
            seed.map_entity = (valid & DivisionSeed::MAP_ENTITY != 0)
                .then(|| u64::from_le_bytes(record[279..287].try_into().unwrap()) as usize);
            seed.exiled = (valid & DivisionSeed::EXILED != 0).then(|| record[287]);
            seed.name_member = (valid & DivisionSeed::NAME_MEMBER != 0)
                .then(|| u64::from_le_bytes(record[288..296].try_into().unwrap()) as usize);
            seed.supply_grace_maximum = (valid & DivisionSeed::SUPPLY_GRACE_MAXIMUM != 0)
                .then(|| i64::from_le_bytes(record[296..304].try_into().unwrap()));
            seed.supply_grace_remaining = (valid & DivisionSeed::SUPPLY_GRACE_REMAINING != 0)
                .then(|| i64::from_le_bytes(record[304..312].try_into().unwrap()));
            seed.manpower_values = (valid & DivisionSeed::MANPOWER_VALUES != 0)
                .then(|| record[312..336].try_into().unwrap());
            seed.movement_priority = (valid & DivisionSeed::MOVEMENT_PRIORITY != 0)
                .then(|| i32::from_le_bytes(record[336..340].try_into().unwrap()));
            seed.withdrawal = (valid & DivisionSeed::WITHDRAWAL != 0).then(|| record[340]);
            seed.alternate_template = (valid & DivisionSeed::ALTERNATE_TEMPLATE != 0)
                .then(|| u64::from_le_bytes(record[341..349].try_into().unwrap()) as usize);
            seed.separate_counter =
                (valid & DivisionSeed::SEPARATE_COUNTER != 0).then(|| record[349]);
            seed.noncore_supply_bypass =
                (valid & DivisionSeed::NONCORE_SUPPLY_BYPASS != 0).then(|| record[350]);
            seed.out_of_supply_days = (valid & DivisionSeed::OUT_OF_SUPPLY_DAYS != 0)
                .then(|| i32::from_le_bytes(record[351..355].try_into().unwrap()));
            seed.fuel = (valid & DivisionSeed::FUEL != 0)
                .then(|| i64::from_le_bytes(record[355..363].try_into().unwrap()));
            seed.air_supply = (valid & DivisionSeed::AIR_SUPPLY != 0)
                .then(|| i64::from_le_bytes(record[363..371].try_into().unwrap()));
            seed.theatre = (valid & DivisionSeed::THEATRE != 0)
                .then(|| u64::from_le_bytes(record[371..379].try_into().unwrap()) as usize);
            seed.queued_actions = (valid & DivisionSeed::QUEUED_ACTIONS != 0)
                .then(|| i32::from_le_bytes(record[379..383].try_into().unwrap()));
        }
    }
    total
}

fn gather_u32s(
    memory: &impl Memory,
    pointers: &[usize],
    offset: usize,
    output: &mut [Option<u32>],
) -> u64 {
    const BATCH: usize = 64;
    let fields = [crate::memory::RecordField {
        source_offset: offset,
        destination_offset: 0,
        len: 4,
    }];
    let layout = crate::memory::RecordLayout::new(&fields, 4).expect("valid four-byte layout");
    let mut bytes = [0_u8; BATCH * 4];
    let mut complete = [false; BATCH];
    let mut total = 0;
    for (pointers, output) in pointers.chunks(BATCH).zip(output.chunks_mut(BATCH)) {
        crate::memory::gather_records(
            memory,
            pointers,
            &layout,
            &mut bytes[..pointers.len() * 4],
            &mut complete[..pointers.len()],
        );
        for (row, value) in output.iter_mut().enumerate() {
            *value = complete[row]
                .then(|| u32::from_le_bytes(bytes[row * 4..row * 4 + 4].try_into().unwrap()));
            total += u64::from(complete[row]) * 4;
        }
    }
    total
}

pub fn capture_division_movers(memory: &impl Memory, seeds: &mut [DivisionSeed]) -> u64 {
    const BATCH: usize = 64;
    let mut movers = [0_usize; BATCH];
    let mut routes = [0_usize; BATCH];
    let mut locations = [0_usize; BATCH];
    let mut heads = [None::<u32>; BATCH];
    let mut ids = [None::<u32>; BATCH];
    let mut total = 0;
    let mut next = 0;
    while next < seeds.len() {
        let mut count = 0;
        while next < seeds.len() && count < BATCH {
            let seed = &seeds[next];
            if let Some(route) = seed
                .ordinary_route
                .and_then(|header| Array::from_header(header, 4))
                .filter(|route| route.count != 0)
            {
                movers[count] = next;
                routes[count] = route.data;
                locations[count] = seed.location.unwrap_or(0);
                count += 1;
            }
            next += 1;
        }
        total += gather_u32s(memory, &routes[..count], 0, &mut heads[..count]);
        total += gather_u32s(
            memory,
            &locations[..count],
            cprovince::ID,
            &mut ids[..count],
        );
        for row in 0..count {
            let seed = &mut seeds[movers[row]];
            seed.route_head = Some(heads[row]);
            seed.location_id = Some(ids[row]);
        }
    }
    total
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MovementRow {
    pub division: u64,
    pub source: u32,
    pub target: u32,
    pub numerator: i64,
    pub retreat: u8,
    pub strategic_redeployment: u8,
    pub strategic_action: StrategicAction,
    pub naval_transfer: u8,
    pub combat_count: u32,
    pub combat_role: u8,
    pub combat_province: u32,
    pub map_entity: u8,
    pub path_start: usize,
    pub path_len: usize,
    pub path_total: u32,
    pub alternate_start: usize,
    pub alternate_len: usize,
    pub alternate_total: u32,
    pub withdrawal: Option<u8>,
    pub movement_priority: Option<i32>,
    pub combat_references: Option<(usize, usize)>,
    pub combat_references_failure: Option<&'static str>,
    pub base_speed: Option<i64>,
    pub fuel: Option<i64>,
    pub transport_speed: Option<i64>,
    pub queued_actions: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementWalk {
    pub outcome: Outcome,
    pub paths_len: usize,
    pub required_paths: usize,
}

#[derive(Clone, Copy, Default)]
struct PresentationInput {
    bytes: [u8; 16],
}

pub(crate) struct PresentationInputs {
    entries: std::collections::HashMap<(usize, usize), PresentationInput>,
    refused: bool,
}

impl PresentationInputs {
    pub(crate) fn new(capacity: usize) -> std::io::Result<Self> {
        let mut entries = std::collections::HashMap::new();
        crate::allocation::reserve_map(&mut entries, capacity)?;
        Ok(Self {
            entries,
            refused: false,
        })
    }
    pub(crate) fn vacant() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            refused: false,
        }
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<((usize, usize), PresentationInput)>()
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.refused = false;
    }
    pub(crate) fn refused(&self) -> bool {
        self.refused
    }
    pub(crate) fn read<T: crate::memory::Raw>(
        &mut self,
        memory: &impl Memory,
        address: usize,
    ) -> std::io::Result<Option<T>> {
        let len = std::mem::size_of::<T>();
        if address == 0 || len > 16 || len == 0 {
            return Ok(read(memory, address));
        }
        if let Some(entry) = self.entries.get(&(address, len)) {
            return Ok(Some(unsafe { entry.bytes.as_ptr().cast::<T>().read_unaligned() }));
        }
        let mut entry = PresentationInput::default();
        if !crate::read_slice(memory, address, &mut entry.bytes[..len]) {
            return Ok(None);
        }
        let value = unsafe { entry.bytes.as_ptr().cast::<T>().read_unaligned() };
        if let Err(error) = crate::allocation::reserve_map(&mut self.entries, 1) {
            self.refused = true;
            return Err(error);
        }
        self.entries.insert((address, len), entry);
        Ok(Some(value))
    }
}

#[cfg(test)]
pub struct MovementInputs {
    shared: std::cell::RefCell<PresentationInputs>,
}
#[cfg(test)]
impl MovementInputs {
    pub fn new(capacity: usize) -> Self {
        Self {
            shared: std::cell::RefCell::new(PresentationInputs::new(capacity).unwrap()),
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.shared.borrow().allocated_bytes()
    }
}

#[cfg(test)]
fn shared_presentation_read<T: crate::memory::Raw>(
    memory: &impl Memory,
    inputs: Option<&MovementInputs>,
    address: usize,
) -> Option<T> {
    match inputs {
        Some(inputs) => inputs.shared.borrow_mut().read(memory, address).ok().flatten(),
        None => read(memory, address),
    }
}

fn presentation_global<T: crate::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    engine_sites::at(base, key)
        .and_then(|p| read(memory, p))
        .ok_or("division presentation field unreadable")
}

fn presentation_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("division presentation field unreadable")
}

fn presentation_array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| Array::read(memory, p, stride))
        .ok_or("division presentation array invalid")
}

fn presentation_batch<const N: usize>(
    memory: &impl Memory,
    fields: [(usize, usize, usize); N],
) -> crate::memory::FieldBatch<N> {
    crate::memory::FieldBatch::gather(
        memory,
        fields.map(|(object, offset, len)| (object.checked_add(offset).unwrap_or(0), len)),
    )
}

fn presentation_batched<T: crate::memory::Raw, const N: usize>(
    memory: &impl Memory,
    fields: &crate::memory::FieldBatch<N>,
    index: usize,
) -> Result<T, &'static str> {
    fields
        .get(memory, index)
        .ok_or("division presentation field unreadable")
}

fn movement_pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<usize, &'static str> {
    presentation_field::<u64>(memory, object, offset).map(|p| p as usize)
}

fn movement_address(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("movement address overflow")
}

pub fn movement_definition(
    memory: &impl Memory,
    base: usize,
    province: usize,
) -> Result<usize, &'static str> {
    if province == 0
        || read::<u64>(memory, province)
            != engine_sites::at(base, cprovince::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("movement province identity mismatch");
    }
    let definition = movement_pointer(memory, province, cprovince::DEFINITION)?;
    if definition == 0
        || presentation_field::<u32>(memory, definition, crate::cprovince::definition::ID)?
            != presentation_field::<u32>(memory, province, cprovince::ID)?
    {
        return Err("movement province definition mismatch");
    }
    Ok(definition)
}

pub fn movement_route_province(
    memory: &impl Memory,
    base: usize,
    state: usize,
    route: Array,
    index: u32,
) -> Result<usize, &'static str> {
    let id = read::<u32>(
        memory,
        route
            .element(index, 4)
            .ok_or("movement route index invalid")?,
    )
    .ok_or("movement route id unreadable")?;
    movement_route_destination(memory, base, movement_province_array(memory, state)?, id)
}

pub fn movement_province_array(memory: &impl Memory, state: usize) -> Result<Array, &'static str> {
    presentation_array(memory, state, cgame_state::PROVINCES, 8)
}

pub fn movement_route_destination(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    id: u32,
) -> Result<usize, &'static str> {
    let province = read::<u64>(
        memory,
        provinces
            .element(id, 8)
            .ok_or("movement route province outside roster")?,
    )
    .ok_or("movement route province unreadable")? as usize;
    if province != 0 {
        movement_definition(memory, base, province)?;
        if presentation_field::<u32>(memory, province, cprovince::ID)? != id {
            return Err("movement route province identity mismatch");
        }
    }
    Ok(province)
}

pub fn movement_destination_controller(
    memory: &impl Memory,
    province: usize,
) -> Result<i32, &'static str> {
    presentation_field(memory, province, cprovince::CONTROLLER)
}

pub fn movement_attacking(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
) -> Result<bool, &'static str> {
    let route = presentation_array(memory, army, cunit::ORDINARY_ROUTE_DATA, 4)?;
    if route.count == 0 {
        return Ok(false);
    }
    let destination = movement_route_province(memory, base, state, route, route.count - 1)?;
    if destination == 0 {
        return Ok(false);
    }
    let controller = movement_destination_controller(memory, destination)?;
    if controller <= 0 {
        return Ok(false);
    }
    ccountry::movement_enemy(
        memory,
        base,
        state,
        controller as u32,
        presentation_field(memory, army, cunit::LOGICAL_OWNER)?,
    )
}

pub fn movement_priority_unit(
    memory: &impl Memory,
    base: usize,
    template: usize,
) -> Result<usize, &'static str> {
    let fields = presentation_batch(
        memory,
        [
            (template, 0, 8),
            (template, creferenced_division_template::PRIORITY_UNIT, 8),
        ],
    );
    if template == 0
        || fields.get::<u64>(memory, 0)
            != engine_sites::at(base, creferenced_division_template::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("movement template identity mismatch");
    }
    let mut selected = presentation_batched::<u64, 2>(memory, &fields, 1)? as usize;
    if selected == 0 {
        let grid = presentation_array(
            memory,
            template,
            creferenced_division_template::REGIMENT_DATA,
            8,
        )?;
        let mut count = 0_u16;
        for i in 0..grid.count {
            let p = movement_pointer(
                memory,
                grid.element(i, 8)
                    .ok_or("movement regiment index invalid")?,
                0,
            )?;
            if p != 0 {
                count = count.wrapping_add(u16::from(
                    presentation_field::<u8>(memory, p, crate::csub_unit_definition::IS_REAL)? != 0,
                ));
            }
        }
        selected = if count != 0 {
            movement_pointer(
                memory,
                grid.element(0, 8).ok_or("movement first regiment absent")?,
                0,
            )?
        } else {
            presentation_global::<u64>(memory, base, crate::csub_unit_definition::NULL_OBJECT_RVA)?
                as usize
        };
    }
    let vtable = read::<u64>(memory, selected);
    if selected == 0
        || (vtable
            != engine_sites::at(base, crate::csub_unit_definition::VTABLE_RVA).map(|p| p as u64)
            && vtable
                != engine_sites::at(base, crate::csub_unit_definition::NULL_VTABLE_RVA)
                    .map(|p| p as u64))
    {
        return Err("movement selected unit identity mismatch");
    }
    Ok(selected)
}

pub fn movement_speed_keys(
    memory: &impl Memory,
    base: usize,
    template: usize,
    keys: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let start = keys.len();
    let result = movement_speed_key_demands(memory, base, template, keys);
    if result.is_err() {
        keys.truncate(start);
    }
    result
}

pub fn movement_speed_key_demands(
    memory: &impl Memory,
    base: usize,
    template: usize,
    keys: &mut Vec<u32>,
) -> Result<(), &'static str> {
    use crate::csub_unit_definition as unit;

    let selected = movement_priority_unit(memory, base, template)?;
    let token = presentation_field::<u32>(memory, selected, unit::SPEED_MODIFIER_KEY)?;
    if token != 0 {
        crate::allocation::reserve(keys, 1).map_err(|_| "movement speed key allocation failed")?;
        keys.push(token);
    }
    let categories = presentation_array(memory, selected, unit::CATEGORIES, 8)?;
    crate::allocation::reserve(keys, categories.count as usize)
        .map_err(|_| "movement speed key allocation failed")?;
    for index in 0..categories.count {
        let category = movement_pointer(
            memory,
            categories
                .element(index, 8)
                .ok_or("movement category index invalid")?,
            0,
        )?;
        let token = presentation_field::<u32>(memory, category, unit::CATEGORY_MODIFIER_KEY)?;
        if token != 0 {
            keys.push(token);
        }
    }
    Ok(())
}

mod strategic_action;
pub use strategic_action::read_inputs as strategic_redeployment_inputs;
pub use strategic_action::{StrategicAction, StrategicActionFailure, StrategicRedeploymentInputs};

mod movement;
pub mod organisation_movement;
pub use movement::{
    MovementPacket, capture_movement_inputs, capture_movement_inputs_with_combat_references,
    capture_movement_inputs_with_extra, project_movement,
};

#[cfg(test)]
use movement::movement_route_prefixes;
#[cfg(test)]
pub use movement::{walk_movement, walk_movement_with_inputs};

#[cfg(test)]
pub(crate) use cunit::FirstCombatKey as MovementCombatKey;
pub(crate) use cunit::FirstCombatMemo as MovementCombats;

fn movement_seed_value<T>(value: Option<T>) -> Result<T, &'static str> {
    value.ok_or("division presentation field unreadable")
}

#[cfg(test)]
fn movement_scalar_tail(memory: &impl Memory, army: usize) -> Result<(u8, u8), &'static str> {
    use crate::memory::RecordField;
    const FIELDS: [RecordField; 2] = [
        RecordField {
            source_offset: RETREAT,
            destination_offset: 0,
            len: 1,
        },
        RecordField {
            source_offset: STRATEGIC_REDEPLOYMENT,
            destination_offset: 1,
            len: 1,
        },
    ];
    let mut bytes = [0; 2];
    const LAYOUT: crate::memory::RecordLayout<'static> =
        crate::memory::RecordLayout::new(&FIELDS, 2).expect("valid movement scalar layout");
    if memory.copy_records(
        crate::memory::RecordSources::Pointers(&[army]),
        &LAYOUT,
        &mut bytes,
    ) != FIELDS.len()
    {
        return Err("division presentation field unreadable");
    }
    Ok((bytes[0], bytes[1]))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusRow {
    pub division: u64,
    pub maximum_hit_points: i64,
    pub maximum_organisation: i64,
    pub movement_organisation: Option<Option<[i64; 2]>>,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub supply_ratio: i64,
    pub supply_grace_maximum: i64,
    pub supply_grace_remaining: i64,
    pub attrition_value: i64,
    pub attrition_threshold: i64,
    pub attrition: u8,
    pub exercising: u8,
    pub has_order: u8,
    pub invalid_orders: u8,
    pub invalid_orders_reason: Option<i32>,
    pub strength_factor: i64,
    pub model_number: i32,
    pub camouflage: Option<Camouflage>,
    pub weather_factor: Option<i64>,
    pub move_flags: u8,
    pub out_of_supply_days: Option<i32>,
    pub border_combat_stats: Option<[i64; 3]>,
    pub naming: Option<crate::cname_group_member::Inputs>,
    pub country_ordinal: Option<u32>,
    pub supply: Option<crate::cunit_supply_inputs::Inputs>,
    pub supply_origins_start: usize,
    pub supply_failure: Option<crate::cunit_supply_inputs::Failure>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Camouflage {
    pub category: Option<u32>,
}

impl StatusRow {
    pub const MOVE_EXILED: u8 = 1;
    pub const MOVE_CAN_EXFILTRATE_FROM_COAST: u8 = 2;
    pub const MOVE_NONCORE_SUPPLY_BYPASS: u8 = 4;
    pub const MOVE_NONCORE_AIR: u8 = 8;
    pub const MOVE_NONCORE_AIR_REFUSED: u8 = 64;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetreatBanRow {
    pub division: u64,
    pub owner: u32,
    pub bans: u8,
}

use crate::corders_group_member::{MembershipCapacity, MembershipPacket, MembershipQuery};

mod model_inputs;
mod status_membership;
mod status_parts;
pub use model_inputs::{ModelInputsReport, ModelInputsRow};
#[cfg(feature = "order_membership_oracle")]
pub use status_membership::oracle as membership_oracle;
pub use status_parts::StatusPart;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StatusCapacities {
    pub units: usize,
    pub supply_origins: usize,
    pub manpower: usize,
    pub acclimatization_categories: usize,
    pub thresholds: usize,
    pub modifiers: crate::modifier_accumulator::SnapshotCapacity,
    pub membership: MembershipCapacity,
}

impl StatusCapacities {
    pub fn new(units: usize) -> Self {
        Self {
            units,
            supply_origins: 0,
            manpower: 0,
            acclimatization_categories: 0,
            thresholds: 0,
            modifiers: crate::modifier_accumulator::SnapshotCapacity::default(),
            membership: MembershipCapacity {
                queries: units,
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StatusLevelOperands {
    manpower_start: usize,
    manpower_len: usize,
    experience: Option<i64>,
    thresholds_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StatusExerciseOperands {
    maximum_level: i32,
    autonomous: bool,
    level: Option<StatusLevelOperands>,
    deployment_present: Option<bool>,
    strength: Option<i64>,
    minimum_strength: Option<i64>,
    stop_at_maximum: Option<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StatusUnitInputs {
    movement_organisation: Option<Option<[i64; 2]>>,
    division: u64,
    maxima: usize,
    threshold: usize,
    group: usize,
    membership: usize,
    attrition_value: i64,
    entrenchment: i64,
    entrenchment_cap: i64,
    supply_ratio: i64,
    supply_grace_maximum: i64,
    supply_grace_remaining: i64,
    training: u8,
    invalid_orders: u8,
    invalid_orders_reason: Option<i32>,
    strength_factor: i64,
    exiled: u8,
    noncore_supply_bypass: u8,
    air_supply: Option<i64>,
    out_of_supply_days: Option<i32>,
    model_number: i32,
    camouflage: Option<Camouflage>,
    weather_factor: Option<i64>,
    exercise: Option<StatusExerciseOperands>,
    has_order: Option<Result<bool, &'static str>>,
    exercising: Option<Result<u8, &'static str>>,
    failure: Option<&'static str>,
    border_combat_stats: Option<[i64; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusCountryInputs {
    pointer: usize,
    modifier: crate::modifier_accumulator::SnapshotObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusGroupInputs {
    pointer: usize,
    training: Result<u8, &'static str>,
    stop_at_maximum: Option<Result<u8, &'static str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusAttritionMetadata {
    feature_selector: u32,
    flags: Option<u32>,
}

pub struct StatusPacket {
    capacities: StatusCapacities,
    required: StatusCapacities,
    units: Vec<StatusUnitInputs>,
    prepared_experience: Vec<Option<i64>>,
    supply_requested: Vec<bool>,
    organisation_memo: organisation_movement::OrganisationMemo,
    supply: Vec<crate::cunit_supply_inputs::UnitCapture>,
    supply_len: usize,
    supply_origins: Vec<crate::cunit_supply_inputs::Origin>,
    supply_origins_len: usize,
    border_index: Vec<(u64, usize)>,
    border_stats: Vec<Option<[i64; 3]>>,
    maxima: Vec<crate::csub_unit_definition::DivisionMaxima>,
    maxima_index: Vec<usize>,
    #[cfg(test)]
    maxima_probes: std::sync::atomic::AtomicUsize,
    acclimatization: Option<Box<AcclimatizationBatch>>,
    countries: Vec<StatusCountryInputs>,
    country_index: Vec<usize>,
    groups: Vec<StatusGroupInputs>,
    group_index: Vec<usize>,
    commander_groups: Vec<usize>,
    commander_leaders: Vec<Option<Result<usize, &'static str>>>,
    commander_bans: Vec<Result<Option<crate::carmy_leader::RetreatBans>, &'static str>>,
    commander_index: Vec<usize>,
    commander_divisions: Vec<(u64, usize)>,
    commander_order: Vec<(u64, u32)>,
    commander_memo: std::sync::Mutex<crate::modifier_definition::DefinitionMemo>,
    modifiers: crate::modifier_accumulator::SnapshotArena,
    membership: MembershipPacket,
    manpower: Vec<i32>,
    exercise_control: Option<cgame_state::AiControl>,
    exercise_autonomy: std::collections::HashMap<u32, bool>,
    categories: crate::acclimatization_category::Categories,
    categories_loaded: Option<Result<(), &'static str>>,
    camouflage_show_at: Option<Result<i64, &'static str>>,
    category_factors: CategoryFactors,
    thresholds_header: Option<Result<[u8; 16], &'static str>>,
    thresholds: Vec<Result<i64, &'static str>>,
    attrition_metadata: Option<Result<StatusAttritionMetadata, &'static str>>,
    attrition_define: Option<Result<i64, &'static str>>,
    maximum_level: Option<Result<i32, &'static str>>,
    minimum_strength: Option<Result<i64, &'static str>>,
    outcome: Outcome,
}

pub type StagedDivisionMaxima<'a> =
    dyn Fn(usize) -> Option<Result<crate::csub_unit_definition::DivisionMaxima, &'static str>> + 'a;

#[derive(Clone, Copy)]
struct SelectedMaxima {
    slot: usize,
    row: usize,
}

impl Default for SelectedMaxima {
    fn default() -> Self {
        Self {
            slot: usize::MAX,
            row: usize::MAX,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct StagedMaxima {
    pointer: usize,
    slot: usize,
    row: usize,
}

fn status_pointer_slot(slots: &[usize], pointer: usize, key: impl Fn(usize) -> usize) -> usize {
    let hash = (pointer as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut at = (hash ^ (hash >> 32)) as usize % slots.len();
    while slots[at] != 0 && key(slots[at] - 1) != pointer {
        at = (at + 1) % slots.len();
    }
    at
}

impl StatusPacket {
    pub fn new(units: usize) -> std::io::Result<Self> {
        Self::with_capacities(StatusCapacities::new(units))
    }

    pub fn vacant() -> Self {
        let capacities = StatusCapacities {
            units: 0,
            supply_origins: 0,
            manpower: 0,
            acclimatization_categories: 0,
            thresholds: 0,
            modifiers: crate::modifier_accumulator::SnapshotCapacity::default(),
            membership: MembershipCapacity::default(),
        };
        Self {
            capacities,
            required: capacities,
            units: Vec::new(),
            prepared_experience: Vec::new(),
            supply_requested: Vec::new(),
            organisation_memo: organisation_movement::OrganisationMemo::vacant(),
            supply: Vec::new(),
            supply_len: 0,
            supply_origins: Vec::new(),
            supply_origins_len: 0,
            border_index: Vec::new(),
            border_stats: Vec::new(),
            maxima: Vec::new(),
            maxima_index: Vec::new(),
            #[cfg(test)]
            maxima_probes: std::sync::atomic::AtomicUsize::new(0),
            acclimatization: None,
            countries: Vec::new(),
            country_index: Vec::new(),
            groups: Vec::new(),
            group_index: Vec::new(),
            commander_groups: Vec::new(),
            commander_leaders: Vec::new(),
            commander_bans: Vec::new(),
            commander_index: Vec::new(),
            commander_divisions: Vec::new(),
            commander_order: Vec::new(),
            commander_memo: std::sync::Mutex::new(
                crate::modifier_definition::DefinitionMemo::default(),
            ),
            modifiers: crate::modifier_accumulator::SnapshotArena::vacant(),
            membership: MembershipPacket::vacant(),
            manpower: Vec::new(),
            exercise_control: None,
            exercise_autonomy: std::collections::HashMap::new(),
            categories: crate::acclimatization_category::Categories::default(),
            categories_loaded: None,
            camouflage_show_at: None,
            category_factors: CategoryFactors::default(),
            thresholds_header: None,
            thresholds: Vec::new(),
            attrition_metadata: None,
            attrition_define: None,
            maximum_level: None,
            minimum_strength: None,
            outcome: Outcome::Rows { len: 0 },
        }
    }

    pub fn with_capacities(capacities: StatusCapacities) -> std::io::Result<Self> {
        let index_capacity = capacities
            .units
            .checked_mul(2)
            .ok_or_else(crate::allocation::oom)?;
        let mut exercise_autonomy = std::collections::HashMap::new();
        crate::allocation::reserve_map(&mut exercise_autonomy, capacities.units)?;
        Ok(Self {
            capacities,
            required: capacities,
            units: crate::allocation::vec(capacities.units)?,
            prepared_experience: Vec::new(),
            supply_requested: Vec::new(),
            organisation_memo: organisation_movement::OrganisationMemo::new(capacities.units)
                .unwrap_or_else(|_| organisation_movement::OrganisationMemo::vacant()),
            supply: crate::allocation::filled(
                capacities.units,
                crate::cunit_supply_inputs::UnitCapture::default(),
            )?,
            supply_len: 0,
            supply_origins: crate::allocation::filled(
                capacities.supply_origins,
                crate::cunit_supply_inputs::Origin::default(),
            )?,
            supply_origins_len: 0,
            border_index: crate::allocation::vec(capacities.units)?,
            border_stats: crate::allocation::filled(capacities.units, None)?,
            maxima: crate::allocation::vec(capacities.units)?,
            maxima_index: crate::allocation::filled(index_capacity, 0)?,
            #[cfg(test)]
            maxima_probes: std::sync::atomic::AtomicUsize::new(0),
            acclimatization: if capacities.units == 0 {
                None
            } else {
                Some(crate::allocation::boxed(AcclimatizationBatch::new())?)
            },
            countries: crate::allocation::vec(capacities.units)?,
            country_index: crate::allocation::filled(index_capacity, 0)?,
            groups: crate::allocation::vec(capacities.units)?,
            group_index: crate::allocation::filled(index_capacity, 0)?,
            commander_groups: crate::allocation::vec(capacities.units)?,
            commander_leaders: crate::allocation::vec(capacities.units)?,
            commander_bans: crate::allocation::vec(capacities.units)?,
            commander_index: crate::allocation::filled(index_capacity, 0)?,
            commander_divisions: crate::allocation::vec(capacities.units)?,
            commander_order: crate::allocation::vec(capacities.units)?,
            commander_memo: std::sync::Mutex::new(
                crate::modifier_definition::DefinitionMemo::try_with_capacity(
                    if capacities.units == 0 { 0 } else { 2 },
                )?,
            ),
            modifiers: crate::modifier_accumulator::SnapshotArena::try_new(capacities.modifiers)?,
            membership: MembershipPacket::try_with_capacity(
                capacities.membership.queries,
                capacities.membership.groups,
                capacities.membership.roots,
                capacities.membership.chunks,
            )?,
            manpower: crate::allocation::vec(capacities.manpower)?,
            exercise_control: None,
            exercise_autonomy,
            categories: crate::acclimatization_category::Categories::try_with_capacity(
                capacities.acclimatization_categories,
            )?,
            categories_loaded: None,
            camouflage_show_at: None,
            category_factors: CategoryFactors::default(),
            thresholds_header: None,
            thresholds: crate::allocation::vec(capacities.thresholds)?,
            attrition_metadata: None,
            attrition_define: None,
            maximum_level: None,
            minimum_strength: None,
            outcome: Outcome::Rows { len: 0 },
        })
    }

    pub fn clear(&mut self) {
        self.units.clear();
        self.prepared_experience.clear();
        self.supply_len = 0;
        self.supply_origins_len = 0;
        self.border_index.clear();
        self.border_stats.fill(None);
        self.maxima.clear();
        self.maxima_index.fill(0);
        #[cfg(test)]
        self.maxima_probes
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.countries.clear();
        self.country_index.fill(0);
        self.groups.clear();
        self.group_index.fill(0);
        self.commander_groups.clear();
        self.commander_leaders.clear();
        self.commander_bans.clear();
        self.commander_index.fill(0);
        self.commander_divisions.clear();
        self.commander_order.clear();
        self.commander_memo
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        self.modifiers.clear();
        self.membership.clear();
        self.manpower.clear();
        self.exercise_control = None;
        self.exercise_autonomy.clear();
        self.categories.clear();
        self.categories_loaded = None;
        self.camouflage_show_at = None;
        self.category_factors = CategoryFactors::default();
        self.thresholds_header = None;
        self.thresholds.clear();
        self.attrition_metadata = None;
        self.attrition_define = None;
        self.maximum_level = None;
        self.minimum_strength = None;
        self.required = self.capacities;
        self.outcome = Outcome::Rows { len: 0 };
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn project(&self, output: &mut [StatusRow]) -> Outcome {
        project_status(self, output)
    }

    pub fn supply_origins(&self, row: &StatusRow) -> Option<&[crate::cunit_supply_inputs::Origin]> {
        let len = row.supply?.origins_len.ok()?;
        let end = row.supply_origins_start.checked_add(len)?;
        self.supply_origins
            .get(..self.supply_origins_len)?
            .get(row.supply_origins_start..end)
    }

    pub fn select_supply(&mut self, requested: &[bool]) -> Result<(), &'static str> {
        self.supply_requested.clear();
        crate::allocation::reserve(&mut self.supply_requested, requested.len())
            .map_err(|_| "division supply selection allocation failed")?;
        self.supply_requested.extend_from_slice(requested);
        Ok(())
    }
    pub fn supply_at(
        &self,
        index: usize,
    ) -> (
        Option<crate::cunit_supply_inputs::Inputs>,
        Option<crate::cunit_supply_inputs::Failure>,
        Option<&[crate::cunit_supply_inputs::Origin]>,
    ) {
        let unit = self
            .supply
            .get(index)
            .copied()
            .filter(|_| index < self.supply_len)
            .unwrap_or_default();
        let inputs = unit.inputs.ok();
        let failure = unit
            .inputs
            .err()
            .or_else(|| inputs.and_then(|v| v.origins_len.err()));
        let origins = inputs.and_then(|v| v.origins_len.ok()).and_then(|len| {
            self.supply_origins
                .get(..self.supply_origins_len)?
                .get(unit.origins_start..unit.origins_start.checked_add(len)?)
        });
        (inputs, failure, origins)
    }

    pub fn grow(&mut self, units: usize, membership: MembershipCapacity) -> std::io::Result<()> {
        self.capacities.membership = self.membership.capacity();
        let mut next = self.required_capacities();
        next.units = next.units.max(units).max(self.capacities.units);
        next.supply_origins = next.supply_origins.max(self.capacities.supply_origins);
        next.manpower = next.manpower.max(self.capacities.manpower);
        next.acclimatization_categories = next
            .acclimatization_categories
            .max(self.capacities.acclimatization_categories);
        next.thresholds = next.thresholds.max(self.capacities.thresholds);
        next.modifiers.nodes = next
            .modifiers
            .nodes
            .max(self.capacities.modifiers.nodes)
            .max(next.units);
        next.modifiers.entries = next
            .modifiers
            .entries
            .max(self.capacities.modifiers.entries);
        next.membership = self.capacities.membership.grown(next.membership);
        next.membership.queries = next
            .membership
            .queries
            .max(membership.queries)
            .max(next.units);
        next.membership.groups = next.membership.groups.max(membership.groups);
        next.membership.roots = next.membership.roots.max(membership.roots);
        next.membership.chunks = next.membership.chunks.max(membership.chunks);
        if next != self.capacities {
            let mut packet = Self::with_capacities(next)?;
            std::mem::swap(&mut packet.supply_requested, &mut self.supply_requested);
            *self = packet;
        }
        Ok(())
    }

    pub fn required_capacities(&self) -> StatusCapacities {
        let mut required = self.required;
        required.modifiers = self.modifiers.required_capacity();
        required.membership = self.membership.required_capacity();
        required
    }

    pub fn membership_capacity(&self) -> MembershipCapacity {
        self.membership.capacity()
    }

    pub fn raise_supply_origins(&mut self, required: usize) {
        self.required.supply_origins = self.required.supply_origins.max(required);
    }

    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.units.capacity() * size_of::<StatusUnitInputs>()
            + self.prepared_experience.capacity() * size_of::<Option<i64>>()
            + self.supply_requested.capacity() * size_of::<bool>()
            + self.organisation_memo.allocated_bytes()
            + self.supply.capacity() * size_of::<crate::cunit_supply_inputs::UnitCapture>()
            + self.supply_origins.capacity() * size_of::<crate::cunit_supply_inputs::Origin>()
            + self.border_index.capacity() * size_of::<(u64, usize)>()
            + self.border_stats.capacity() * size_of::<Option<[i64; 3]>>()
            + self.maxima.capacity() * size_of::<crate::csub_unit_definition::DivisionMaxima>()
            + self.countries.capacity() * size_of::<StatusCountryInputs>()
            + self.groups.capacity() * size_of::<StatusGroupInputs>()
            + self.commander_groups.capacity() * size_of::<usize>()
            + self.commander_leaders.capacity() * size_of::<Option<Result<usize, &'static str>>>()
            + self.commander_bans.capacity()
                * size_of::<Result<Option<crate::carmy_leader::RetreatBans>, &'static str>>()
            + self.commander_divisions.capacity() * size_of::<(u64, usize)>()
            + self.commander_order.capacity() * size_of::<(u64, u32)>()
            + self
                .commander_memo
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .allocated_bytes()
            + (self.maxima_index.capacity()
                + self.country_index.capacity()
                + self.group_index.capacity()
                + self.commander_index.capacity())
                * size_of::<usize>()
            + self.modifiers.allocated_bytes()
            + self.membership.allocated_bytes()
            + self.manpower.capacity() * size_of::<i32>()
            + self.exercise_autonomy.capacity() * (size_of::<(u32, bool)>() + 1)
            + self.categories.allocated_bytes()
            + self.thresholds.capacity() * size_of::<Result<i64, &'static str>>()
            + self
                .acclimatization
                .as_ref()
                .map_or(0, |_| size_of::<AcclimatizationBatch>())
    }

    fn maxima_slot(&self, pointer: usize, key: impl Fn(usize) -> usize) -> usize {
        #[cfg(test)]
        self.maxima_probes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        status_pointer_slot(&self.maxima_index, pointer, key)
    }

    fn stage_maxima(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        staged_reads: impl Fn(usize) -> bool,
        batch: &mut crate::csub_unit_definition::MaximaBatch,
    ) -> [SelectedMaxima; 64] {
        let mut selected = [SelectedMaxima::default(); 64];
        let mut staged = [StagedMaxima::default(); 64];
        let mut staged_len = 0;
        let mut pointers = [0_usize; 64];
        let mut len = 0;
        let identity = engine_sites::at(base, VTABLE_RVA).map(|at| at as u64);
        for (index, seed) in seeds.iter().enumerate() {
            if seed.primary_vtable != identity
                || seed.handle != Some(seed.division)
                || seed.division == 0
            {
                continue;
            }
            let Some(pointer) = seed.stat_block else {
                continue;
            };
            let slot = self.maxima_slot(pointer, |i| {
                if i < self.maxima.len() {
                    self.maxima[i].pointer
                } else {
                    staged[usize::MAX - 1 - i].pointer
                }
            });
            let resident = self.maxima_index[slot];
            let row = if resident == 0 {
                let carried = held
                    .get(index)
                    .copied()
                    .flatten()
                    .is_some_and(|value| value.pointer == pointer);
                let row = if carried || pointer == 0 || staged_reads(index) {
                    usize::MAX
                } else {
                    pointers[len] = pointer;
                    len += 1;
                    len - 1
                };
                self.maxima_index[slot] = usize::MAX - staged_len;
                staged[staged_len] = StagedMaxima { pointer, slot, row };
                staged_len += 1;
                row
            } else if resident - 1 < self.maxima.len() {
                usize::MAX
            } else {
                staged[usize::MAX - resident].row
            };
            selected[index] = SelectedMaxima { slot, row };
        }
        for pending in &staged[..staged_len] {
            self.maxima_index[pending.slot] = 0;
        }
        batch.capture(memory, &pointers[..len]);
        selected
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the seed's carried, staged and batched reads"
    )]
    fn maxima(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        selected: SelectedMaxima,
        held: Option<crate::csub_unit_definition::DivisionMaxima>,
        staged: Option<Result<crate::csub_unit_definition::DivisionMaxima, &'static str>>,
        batch: &crate::csub_unit_definition::MaximaBatch,
    ) -> Result<usize, &'static str> {
        let slot = selected.slot;
        debug_assert_ne!(slot, usize::MAX);
        if self.maxima_index[slot] != 0 {
            return Ok(self.maxima_index[slot] - 1);
        }
        let value = match held.filter(|value| value.pointer == pointer) {
            Some(value) => value,
            None => match staged {
                Some(staged) => staged?,
                None => batch.read(memory, base, pointer, selected.row)?,
            },
        };
        let index = self.maxima.len();
        self.maxima.push(value);
        self.maxima_index[slot] = index + 1;
        Ok(index)
    }

    pub fn unit_maxima(&self, index: usize) -> Option<crate::csub_unit_definition::DivisionMaxima> {
        let unit = self
            .units
            .get(index)
            .filter(|unit| unit.failure.is_none())?;
        self.maxima.get(unit.maxima).copied()
    }

    fn country_threshold(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<usize, &'static str> {
        let slot = status_pointer_slot(&self.country_index, pointer, |i| self.countries[i].pointer);
        if self.country_index[slot] != 0 {
            return Ok(self.country_index[slot] - 1);
        }
        let modifier =
            self.modifiers
                .capture(memory, pointer, keys::UNIT_UPKEEP_ATTRITION_FACTOR)?;
        if self.modifiers.value(modifier)?.present {
            if self.attrition_metadata.is_none() {
                self.attrition_metadata = Some((|| {
                    use crate::modifier_definition as metadata;
                    let table =
                        presentation_global::<u64>(memory, base, metadata::TABLE_RVA)? as usize;
                    let entry = table
                        .checked_add(keys::UNIT_UPKEEP_ATTRITION_FACTOR as usize * metadata::STRIDE)
                        .filter(|_| table != 0)
                        .ok_or("attrition metadata null or overflow")?;
                    let feature_selector =
                        presentation_field::<u32>(memory, entry, metadata::FEATURE_SELECTOR)?;
                    let flags = if feature_selector == 0 {
                        Some(presentation_field::<u32>(memory, entry, metadata::FLAGS)?)
                    } else {
                        None
                    };
                    Ok(StatusAttritionMetadata {
                        feature_selector,
                        flags,
                    })
                })());
            }
            if self.attrition_metadata.unwrap()?.feature_selector != 0 {
                return Err("attrition modifier needs unproved feature callback");
            }
        }
        if self.attrition_define.is_none() {
            self.attrition_define = Some(presentation_global::<i64>(
                memory,
                base,
                COUNTER_ATTRITION_THRESHOLD_RVA,
            ));
        }
        self.attrition_define.unwrap()?;
        let index = self.countries.len();
        self.countries
            .push(StatusCountryInputs { pointer, modifier });
        self.country_index[slot] = index + 1;
        Ok(index)
    }

    fn training(&mut self, memory: &impl Memory, pointer: usize) -> Result<u8, &'static str> {
        self.training_known(memory, pointer, None)
    }

    fn training_known(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        known: Option<Result<u8, &'static str>>,
    ) -> Result<u8, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let slot = status_pointer_slot(&self.group_index, pointer, |i| self.groups[i].pointer);
        if self.group_index[slot] != 0 {
            return self.groups[self.group_index[slot] - 1].training;
        }
        let training =
            known.unwrap_or_else(|| presentation_field(memory, pointer, corders_group::TRAINING));
        self.groups.push(StatusGroupInputs {
            pointer,
            training,
            stop_at_maximum: None,
        });
        self.group_index[slot] = self.groups.len();
        training
    }

    fn commander(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<u8, &'static str> {
        let group = seed
            .orders_group
            .ok_or("division presentation field unreadable")?;
        if group == 0 {
            return Ok(0);
        }
        let slot = status_pointer_slot(&self.commander_index, group, |i| self.commander_groups[i]);
        let bans = if self.commander_index[slot] != 0 {
            self.commander_bans[self.commander_index[slot] - 1]?
        } else {
            if self.commander_groups.len() == self.commander_groups.capacity() {
                return Err("division status commander packet capacity exceeded");
            }
            let bans = crate::carmy_leader::retreat_bans(memory, base, group);
            self.commander_groups.push(group);
            self.commander_leaders.push(None);
            self.commander_bans.push(bans);
            self.commander_index[slot] = self.commander_groups.len();
            bans?
        };
        let Some(bans) = bans else {
            return Ok(0);
        };
        if bans.ordinary == bans.alternate {
            return Ok(bans.ordinary);
        }
        let accumulator = match seed.selected {
            Some(selected) => selected
                .block
                .map(|block| block.map(|(accumulator, _)| accumulator)),
            None => crate::carmy_leader::movement_accumulator_seeded(memory, base, seed),
        }?
        .ok_or("retreat ban leader vanished")?;
        bans.select(accumulator)
    }

    fn prepare_commanders(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        wanted: &dyn Fn(&DivisionSeed) -> bool,
        held: &mut dyn FnMut(usize) -> Option<crate::carmy_leader::RetreatBans>,
    ) {
        let first = self.commander_groups.len();
        for seed in seeds {
            let Some(group) = seed
                .orders_group
                .filter(|&group| group != 0 && wanted(seed))
            else {
                continue;
            };
            let slot =
                status_pointer_slot(&self.commander_index, group, |i| self.commander_groups[i]);
            if self.commander_index[slot] != 0 {
                continue;
            }
            if self.commander_groups.len() == self.commander_groups.capacity() {
                break;
            }
            self.commander_groups.push(group);
            self.commander_leaders
                .push(seed.selected.and_then(|selected| selected.leader));
            self.commander_bans.push(Ok(None));
            self.commander_index[slot] = self.commander_groups.len();
        }
        crate::carmy_leader::retreat_bans_many(
            memory,
            base,
            &self.commander_groups[first..],
            &self.commander_leaders[first..],
            &mut self.commander_bans[first..],
            self.commander_memo
                .get_mut()
                .unwrap_or_else(|error| error.into_inner()),
            held,
        );
    }

    pub fn commander_movement(
        &self,
        division: u64,
        block: u8,
    ) -> Option<crate::carmy_leader::MovementEntries> {
        let at = self
            .commander_order
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()?;
        self.commander_movement_of(self.commander_order[at].1 as usize, block)
    }

    pub fn commander_movement_at(
        &self,
        position: usize,
        division: u64,
        block: u8,
    ) -> Option<crate::carmy_leader::MovementEntries> {
        match self.commander_divisions.get(position) {
            Some(&(at, _)) if at == division => self.commander_movement_of(position, block),
            _ => self.commander_movement(division, block),
        }
    }

    fn commander_movement_of(
        &self,
        at: usize,
        block: u8,
    ) -> Option<crate::carmy_leader::MovementEntries> {
        let absent = [None; crate::carmy_leader::MOVEMENT_MODIFIER_KEYS.len()];
        let bans = match self.commander_divisions.get(at)?.1 {
            usize::MAX => None,
            index => (*self.commander_bans.get(index)?).ok()?,
        };
        match (bans, block) {
            (_, COMMANDER_BLOCK_NONE) => Some(absent),
            (Some(bans), COMMANDER_BLOCK_ORDINARY) => Some(bans.movement[0]),
            (Some(bans), COMMANDER_BLOCK_SEPARATE_HQ) => Some(bans.movement[1]),
            _ => None,
        }
    }

    pub fn commander_bans(
        &self,
    ) -> impl Iterator<Item = Result<Option<crate::carmy_leader::RetreatBans>, &'static str>> + '_
    {
        self.commander_bans.iter().copied()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn retreat_bans(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        source: Outcome,
        owner: &dyn Fn(u32) -> Result<Option<u32>, &'static str>,
        output: &mut [RetreatBanRow],
        held: &mut dyn FnMut(usize) -> Option<crate::carmy_leader::RetreatBans>,
    ) -> Outcome {
        let result = (|| {
            if matches!(source, Outcome::Unavailable { .. }) {
                return Err("division retreat ban source unavailable");
            }
            let seeds = seeds
                .get(..source.len())
                .ok_or("division source count invalid")?;
            let owned = |seed: &DivisionSeed| {
                seed.roster_owner
                    .ok_or("division roster owner unreadable")
                    .and_then(owner)
            };
            let unresolved = seeds.iter().filter(|seed| owned(seed).is_err()).count();
            if unresolved != 0 {
                return Ok(Outcome::Truncated {
                    len: 0,
                    unread_rows: u32::try_from(unresolved).unwrap_or(u32::MAX),
                });
            }
            self.commander_groups.clear();
            self.commander_leaders.clear();
            self.commander_bans.clear();
            self.commander_index.fill(0);
            self.commander_divisions.clear();
            self.commander_order.clear();
            self.commander_memo
                .get_mut()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            self.prepare_commanders(
                memory,
                base,
                seeds,
                &|seed| matches!(owned(seed), Ok(Some(_))),
                held,
            );
            let (mut len, mut unread) = (0, 0_u32);
            for seed in seeds {
                let Ok(Some(owner)) = owned(seed) else {
                    continue;
                };
                let bans = self.commander(memory, base, seed)?;
                if self.commander_divisions.len() < self.commander_divisions.capacity() {
                    let index = match seed.orders_group {
                        Some(group) if group != 0 => {
                            let slot = status_pointer_slot(&self.commander_index, group, |i| {
                                self.commander_groups[i]
                            });
                            self.commander_index[slot].wrapping_sub(1)
                        }
                        _ => usize::MAX,
                    };
                    self.commander_divisions.push((seed.division, index));
                }
                if bans == 0 {
                    continue;
                }
                match output.get_mut(len) {
                    Some(row) => {
                        *row = RetreatBanRow {
                            division: seed.division,
                            owner,
                            bans,
                        };
                        len += 1;
                    }
                    None => unread += 1,
                }
            }
            self.commander_order.extend(
                self.commander_divisions
                    .iter()
                    .enumerate()
                    .map(|(at, entry)| (entry.0, at as u32)),
            );
            self.commander_order.sort_unstable_by_key(|entry| entry.0);
            Ok(if unread == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: unread,
                }
            })
        })();
        result.unwrap_or_else(|reason| {
            self.commander_divisions.clear();
            self.commander_order.clear();
            Outcome::Unavailable { reason }
        })
    }

    fn stop_at_maximum(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
    ) -> Result<u8, &'static str> {
        let slot = status_pointer_slot(&self.group_index, pointer, |i| self.groups[i].pointer);
        let group = self
            .groups
            .get_mut(self.group_index[slot].wrapping_sub(1))
            .ok_or("division training group inputs missing")?;
        *group.stop_at_maximum.get_or_insert_with(|| {
            presentation_field(memory, pointer, corders_group::STOP_AT_MAXIMUM)
        })
    }

    fn capture_level(
        &mut self,
        memory: &impl Memory,
        base: usize,
        army: usize,
        held_experience: Option<i64>,
    ) -> Result<StatusLevelOperands, &'static str> {
        let address = army
            .checked_add(MANPOWER_DATA)
            .ok_or("division presentation array invalid")?;
        let header =
            read::<[u8; 16]>(memory, address).ok_or("division presentation array invalid")?;
        let values = Array::from_header(header, 8).ok_or("division presentation array invalid")?;
        let start = self.manpower.len();
        let required = start
            .checked_add(values.count as usize)
            .ok_or("exercise manpower count overflow")?;
        self.required.manpower = self.required.manpower.max(required);
        crate::allocation::reserve(&mut self.manpower, required - start)
            .map_err(|_| "status manpower allocation failed")?;
        self.capacities.manpower = self.capacities.manpower.max(required);
        const BATCH: usize = 64;
        const FIELDS: [crate::memory::RecordField; 1] = [crate::memory::RecordField {
            source_offset: 4,
            destination_offset: 0,
            len: 4,
        }];
        const LAYOUT: crate::memory::RecordLayout<'static> =
            crate::memory::RecordLayout::new(&FIELDS, 4).expect("valid status manpower layout");
        let mut bytes = [0; BATCH * 4];
        let mut manpower = 0_i32;
        for first in (0..values.count).step_by(BATCH) {
            let count = ((values.count - first) as usize).min(BATCH);
            let entry = values
                .element(first, 8)
                .ok_or("exercise manpower entry overflow")?;
            let copied = memory.copy_records(
                crate::memory::RecordSources::Strided {
                    base: entry,
                    stride: 8,
                    count,
                },
                &LAYOUT,
                &mut bytes[..count * 4],
            );
            for word in bytes[..copied * 4].chunks_exact(4) {
                let value = i32::from_le_bytes(word.try_into().unwrap());
                self.manpower.push(value);
                manpower = manpower.wrapping_add(value);
            }
            if copied != count {
                return Err("division presentation field unreadable");
            }
        }
        let experience = if manpower < 1 {
            None
        } else {
            Some(match held_experience {
                Some(value) => value,
                None => presentation_field::<i64>(memory, army, EXPERIENCE)?,
            })
        };
        let fraction = status_experience_fraction(manpower, experience.unwrap_or(0));
        if self.thresholds_header.is_none() {
            self.thresholds_header = Some(
                engine_sites::at(base, EXPERIENCE_THRESHOLDS_RVA)
                    .and_then(|p| read::<[u8; 16]>(memory, p))
                    .ok_or("exercise thresholds invalid"),
            );
        }
        let thresholds = Array::from_header(self.thresholds_header.unwrap()?, 8)
            .ok_or("exercise thresholds invalid")?;
        let additional = (thresholds.count as usize).saturating_sub(self.thresholds.len());
        crate::allocation::reserve(&mut self.thresholds, additional)
            .map_err(|_| "status threshold allocation failed")?;
        self.capacities.thresholds = self.capacities.thresholds.max(thresholds.count as usize);
        if thresholds.count == 0 {
            return Err("exercise thresholds empty");
        }
        let mut thresholds_len = 0;
        for index in 0..thresholds.count as usize {
            if index == self.thresholds.len() {
                self.required.thresholds = self.required.thresholds.max(index + 1);
                let value = thresholds
                    .element(index as u32, 8)
                    .and_then(|p| read::<i64>(memory, p))
                    .ok_or("exercise threshold unreadable");
                self.thresholds.push(value);
            }
            let value = self.thresholds[index]?;
            thresholds_len = index + 1;
            if value > fraction {
                break;
            }
        }
        Ok(StatusLevelOperands {
            manpower_start: start,
            manpower_len: values.count as usize,
            experience,
            thresholds_len,
        })
    }

    fn exercise_autonomous<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        countries: &ccountry::Reader<'_, M>,
        raw: u32,
    ) -> Result<bool, &'static str> {
        if let Some(&value) = self.exercise_autonomy.get(&raw) {
            return Ok(value);
        }
        let canonical = if raw == 0 {
            0
        } else {
            countries.resolve(raw)?.canonical_slot
        };
        if self.exercise_control.is_none() {
            self.exercise_control = Some(cgame_state::AiControl::read(memory, state)?);
        }
        let value = self
            .exercise_control
            .as_mut()
            .unwrap()
            .autonomous(memory, base, countries, raw, canonical)?;
        if self.exercise_autonomy.len() < self.exercise_autonomy.capacity() {
            self.exercise_autonomy.insert(raw, value);
        }
        Ok(value)
    }

    fn capture_exercise(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        unit: usize,
        held_experience: Option<i64>,
        autonomous: impl FnOnce(&mut Self, u32) -> Result<bool, &'static str>,
        maximum_hp: impl FnOnce(&mut Self) -> Result<i64, &'static str>,
    ) -> Result<StatusExerciseOperands, &'static str> {
        let mut inputs = StatusExerciseOperands::default();

        let owner = seed
            .roster_owner
            .ok_or("division presentation field unreadable")?;
        inputs.maximum_level = (*self
            .maximum_level
            .get_or_insert_with(|| presentation_global(memory, base, EXERCISE_MAXIMUM_LEVEL_RVA)))?;
        inputs.autonomous = autonomous(self, owner)?;
        if inputs.autonomous {
            let level = self.capture_level(memory, base, seed.pointer, held_experience)?;
            inputs.level = Some(level);
            if self.level(level)? >= inputs.maximum_level {
                return Ok(inputs);
            }
        }
        let deployment = presentation_field::<u64>(memory, seed.pointer, DEPLOYMENT_TYPE)?;
        inputs.deployment_present =
            Some(crate::creference_registry::resolve(memory, base, deployment)?.is_some());
        if inputs.deployment_present == Some(true) {
            return Ok(inputs);
        }

        let strength = seed
            .strength
            .ok_or("division presentation field unreadable")?;
        inputs.strength = Some(strength);
        let fraction = status_strength_fraction(strength, maximum_hp(self)?);
        let minimum = (*self.minimum_strength.get_or_insert_with(|| {
            presentation_global(memory, base, EXERCISE_MINIMUM_STRENGTH_RVA)
        }))?;
        inputs.minimum_strength = Some(minimum);
        if fraction <= minimum {
            return Ok(inputs);
        }
        let stop = self.stop_at_maximum(memory, self.units[unit].group)?;
        inputs.stop_at_maximum = Some(stop);
        if stop != 0 && inputs.level.is_none() {
            inputs.level = Some(self.capture_level(memory, base, seed.pointer, held_experience)?);
        }
        Ok(inputs)
    }

    fn level(&self, inputs: StatusLevelOperands) -> Result<i32, &'static str> {
        let words = self
            .manpower
            .get(inputs.manpower_start..inputs.manpower_start + inputs.manpower_len)
            .ok_or("division exercise manpower inputs invalid")?;
        let manpower = words
            .iter()
            .fold(0_i32, |sum, word| sum.wrapping_add(*word));
        let fraction = status_experience_fraction(manpower, inputs.experience.unwrap_or(0));
        let thresholds = self
            .thresholds
            .get(..inputs.thresholds_len)
            .ok_or("division exercise threshold inputs invalid")?;
        let mut level = 0;
        for &threshold in thresholds {
            if threshold? > fraction {
                break;
            }
            level += 1;
        }
        Ok(level)
    }

    fn exercise(
        &self,
        inputs: StatusExerciseOperands,
        maximum_hp: Option<i64>,
    ) -> Result<bool, &'static str> {
        if inputs.autonomous
            && self.level(inputs.level.ok_or("division exercise level missing")?)?
                >= inputs.maximum_level
        {
            return Ok(false);
        }
        if inputs
            .deployment_present
            .ok_or("division exercise deployment input missing")?
        {
            return Ok(false);
        }
        let fraction = status_strength_fraction(
            inputs
                .strength
                .ok_or("division exercise strength missing")?,
            maximum_hp.ok_or("division exercise maxima missing")?,
        );
        if fraction
            <= inputs
                .minimum_strength
                .ok_or("division exercise strength define missing")?
        {
            return Ok(false);
        }
        Ok(inputs
            .stop_at_maximum
            .ok_or("division exercise stop flag missing")?
            == 0
            || self.level(inputs.level.ok_or("division exercise level missing")?)?
                < inputs.maximum_level)
    }

    pub fn exercising_at(&self, index: usize) -> Result<u8, &'static str> {
        let unit = self
            .units
            .get(index)
            .ok_or("division exercise index invalid")?;
        if let Some(decision) = unit.exercising {
            return decision;
        }
        let Some(inputs) = unit.exercise else {
            return Ok(0);
        };
        let maximum_hp = self
            .maxima
            .get(unit.maxima)
            .map(|maxima| maxima.effective().0);
        self.exercise(inputs, maximum_hp).map(u8::from)
    }

    pub fn status_index_at(&self, position: usize, division: u64) -> Option<usize> {
        if self.border_index.len() == self.units.len()
            && self
                .units
                .get(position)
                .is_some_and(|unit| unit.division == division)
        {
            return Some(position);
        }
        self.status_index(division)
    }

    pub fn status_index(&self, division: u64) -> Option<usize> {
        let position = self
            .border_index
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()?;
        Some(self.border_index[position].1)
    }

    fn capture_camouflage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        unit: usize,
        first: Option<&[u8]>,
    ) {
        let (camouflage, weather_factor) = match acclimatization_winner(memory, base, seed, first) {
            Ok((object, winner)) => (
                self.read_camouflage(memory, base, &object, winner),
                weather_factor(memory, &object, winner, &mut self.category_factors),
            ),
            Err(reason) => (Err(reason), Err(reason)),
        };
        self.units[unit].camouflage = camouflage.ok();
        self.units[unit].weather_factor = weather_factor.ok();
    }

    fn read_camouflage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        object: &[u8; 0x40],
        winner: Option<(u64, i64)>,
    ) -> Result<Camouflage, &'static str> {
        use crate::acclimatization_data as data;
        let Some((category, weight)) = winner.filter(|(category, _)| *category != 0) else {
            return Ok(Camouflage { category: None });
        };
        let show_at = *self.camouflage_show_at.get_or_insert_with(|| {
            presentation_global::<i64>(memory, base, data::CAMO_SHOW_AT_RVA)
        });
        let maximum =
            i64::from_le_bytes(object[data::MAXIMUM..data::MAXIMUM + 8].try_into().unwrap());
        if !data::shows(weight, maximum, show_at?) {
            return Ok(Camouflage { category: None });
        }
        let categories = *self.categories_loaded.get_or_insert_with(|| {
            self.categories
                .load(memory, base, &mut self.required.acclimatization_categories)
        });
        categories?;
        let token = usize::try_from(category)
            .ok()
            .and_then(|pointer| self.categories.token(pointer))
            .ok_or("division acclimatization category is not registered")?;
        Ok(Camouflage {
            category: Some(token),
        })
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
    ) -> Outcome {
        self.capture_holding(memory, base, state, seeds, source_outcome, &[], &[])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_holding(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        experience: &[Option<i64>],
    ) -> Outcome {
        let prepared = self.prepare_holding_inner(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            held,
            &|_| None,
            false,
        );
        self.finish_holding(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            experience,
            prepared,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_holding_inner<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        staged: &StagedDivisionMaxima<'_>,
        defer_membership: bool,
    ) -> Result<(usize, ccountry::Reader<'a, M>), &'static str> {
        self.clear();
        let prepared = (|| {
            if let Outcome::Unavailable { reason } = source_outcome {
                return Err(reason);
            }
            if source_outcome.len() != seeds.len() {
                return Err("division status source length invalid");
            }
            self.grow(seeds.len(), self.membership.capacity())
                .map_err(|_| "status seed allocation failed")?;
            let countries = ccountry::Reader::new(memory, base, state)?;
            self.required.units = self.required.units.max(seeds.len());
            let count = seeds.len();
            let mut batch = crate::csub_unit_definition::MaximaBatch::new();
            let mut selected = [SelectedMaxima::default(); 64];
            let identity = engine_sites::at(base, VTABLE_RVA).map(|at| at as u64);
            for (index, seed) in seeds[..count].iter().enumerate() {
                if index.is_multiple_of(64) {
                    let end = (index + 64).min(count);
                    selected = self.stage_maxima(
                        memory,
                        base,
                        &seeds[index..end],
                        held.get(index..held.len().min(end)).unwrap_or(&[]),
                        |offset| staged(index + offset).is_some(),
                        &mut batch,
                    );
                }
                let unit = self.units.len();
                self.units.push(StatusUnitInputs {
                    division: seed.division,
                    ..StatusUnitInputs::default()
                });

                let result = (|| -> Result<(), &'static str> {
                    if seed.primary_vtable != identity
                        || seed
                            .handle
                            .ok_or("division presentation field unreadable")?
                            != seed.division
                        || seed.division == 0
                    {
                        return Err("division status source identity changed");
                    }

                    let stats = seed
                        .stat_block
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].maxima = self.maxima(
                        memory,
                        base,
                        stats,
                        selected[index % 64],
                        held.get(index).copied().flatten(),
                        staged(index),
                        &batch,
                    )?;

                    let expeditionary = seed
                        .expeditionary_owner
                        .ok_or("division presentation field unreadable")?
                        as i32;
                    let raw_country = if expeditionary > 0 {
                        expeditionary as u32
                    } else {
                        seed.roster_owner
                            .ok_or("division presentation field unreadable")?
                    };
                    let country = countries.resolve(raw_country)?;
                    self.units[unit].threshold =
                        self.country_threshold(memory, base, country.pointer)?;

                    let group = seed
                        .orders_group
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].group = group;
                    let member = seed
                        .pointer
                        .checked_add(cunit::SCHEDULED_MEMBER)
                        .ok_or("division membership address overflow")?;
                    if status_parts::census_authenticated(seed) {
                        let _ = self.membership.admit_group(group);
                    }
                    self.units[unit].membership = self.membership.push_query(MembershipQuery {
                        member,
                        own_group: group,
                    })?;
                    Ok(())
                })();
                if let Err(reason) = result {
                    self.units[unit].failure = Some(reason);
                    break;
                }
            }
            Ok((count, countries))
        })();
        if !defer_membership {
            self.membership.acquire(memory, base);
        }
        prepared
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_holding(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        experience: &[Option<i64>],
    ) -> Outcome {
        self.prepare_holding_staged(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            held,
            experience,
            &|_| None,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "prepare_holding's operands and the staged reads"
    )]
    pub fn prepare_holding_staged(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        held: &[Option<crate::csub_unit_definition::DivisionMaxima>],
        experience: &[Option<i64>],
        staged: &StagedDivisionMaxima<'_>,
    ) -> Outcome {
        let prepared = self.prepare_holding_inner(
            memory,
            base,
            state,
            seeds,
            source_outcome,
            held,
            staged,
            true,
        );
        self.outcome = match prepared {
            Ok((count, _)) => {
                if crate::allocation::reserve(&mut self.prepared_experience, experience.len())
                    .is_err()
                {
                    Outcome::Unavailable {
                        reason: "status experience allocation failed",
                    }
                } else {
                    self.prepared_experience.extend_from_slice(experience);
                    match source_outcome {
                        Outcome::Truncated { unread_rows, .. } => Outcome::Truncated {
                            len: count,
                            unread_rows,
                        },
                        _ => Outcome::Rows { len: count },
                    }
                }
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        self.outcome
    }

    pub fn prepared_units(&self) -> usize {
        self.units.len()
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_holding<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        experience: &[Option<i64>],
        prepared: Result<(usize, ccountry::Reader<'_, M>), &'static str>,
        defer_children: bool,
    ) -> Outcome {
        let mut entries = self.acclimatization.take();
        let result = prepared.and_then(|(count, countries)| {
            let admitted = &seeds[..self.units.len()];
            if !admitted.is_empty() {
                let entries = entries
                    .as_deref_mut()
                    .expect("admitted packet has entry scratch");
                for (unit, seed) in admitted.iter().enumerate() {
                    if !defer_children && unit.is_multiple_of(64) {
                        entries.capture(
                            memory,
                            base,
                            &admitted[unit..(unit + 64).min(admitted.len())],
                        );
                    }
                    if let Some(reason) = self.units[unit].failure {
                        return Err(reason);
                    }
                    self.units[unit]
                        .has_order
                        .unwrap_or_else(|| self.membership.outcome(self.units[unit].membership))?;

                    self.units[unit].attrition_value = seed
                        .counter_attrition
                        .ok_or("division presentation field unreadable")?;

                    self.units[unit].entrenchment = seed
                        .entrenchment
                        .ok_or("division presentation field unreadable")?;

                    self.units[unit].entrenchment_cap = seed
                        .entrenchment_cap
                        .ok_or("division presentation field unreadable")?;

                    self.units[unit].supply_ratio = seed
                        .counter_supply_status
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].supply_grace_maximum = seed
                        .supply_grace_maximum
                        .ok_or("division maximum supply grace unreadable")?;
                    self.units[unit].supply_grace_remaining = seed
                        .supply_grace_remaining
                        .ok_or("division remaining supply grace unreadable")?;
                    self.units[unit].invalid_orders = seed
                        .invalid_orders
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].strength_factor = seed
                        .strength_factor
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].exiled = seed
                        .exiled
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].noncore_supply_bypass = seed
                        .noncore_supply_bypass
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].air_supply = seed.air_supply;
                    self.units[unit].out_of_supply_days = seed.out_of_supply_days;
                    self.units[unit].model_number = seed
                        .model_number
                        .ok_or("division presentation field unreadable")?;
                    if !defer_children {
                        self.capture_camouflage(memory, base, seed, unit, entries.first(unit % 64));
                    }
                    self.units[unit].training = self.training_known(
                        memory,
                        self.units[unit].group,
                        seed.selected.and_then(|selected| selected.training),
                    )?;
                    if self.units[unit].training != 0 {
                        self.units[unit].exercise = Some(self.capture_exercise(
                            memory,
                            base,
                            seed,
                            unit,
                            experience.get(unit).copied().flatten(),
                            |packet, raw| {
                                packet.exercise_autonomous(memory, base, state, &countries, raw)
                            },
                            |packet| Ok(packet.maxima[packet.units[unit].maxima].effective().0),
                        )?);
                    }
                }
            }
            let missing = match source_outcome {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            };
            Ok(if missing == 0 {
                Outcome::Rows { len: count }
            } else {
                Outcome::Truncated {
                    len: count,
                    unread_rows: missing,
                }
            })
        });
        self.acclimatization = entries;
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        if defer_children {
            return self.outcome;
        }
        if self.units.len() > self.border_index.capacity() {
            if matches!(self.outcome, Outcome::Rows { .. }) {
                self.outcome = Outcome::Unavailable {
                    reason: "border division index packet capacity exceeded",
                };
            }
            return self.outcome;
        }
        self.border_index.extend(
            self.units
                .iter()
                .enumerate()
                .map(|(index, unit)| (unit.division, index)),
        );
        self.border_index.sort_unstable_by_key(|entry| entry.0);
        self.capture_status_children(memory, seeds);
        self.outcome
    }

    fn capture_status_children(&mut self, memory: &impl Memory, seeds: &[DivisionSeed]) {
        for (unit, seed) in self.units.iter_mut().zip(seeds).take(self.outcome.len()) {
            if unit.invalid_orders != 0 {
                unit.invalid_orders_reason = seed
                    .pointer
                    .checked_add(cunit::INVALID_ORDERS_REASON)
                    .and_then(|address| read(memory, address));
            }
        }
        self.supply_len = self.outcome.len().min(self.supply.len());
        let used = crate::cunit_supply_inputs::capture_selected_units(
            memory,
            |index| seeds[index].pointer,
            |index| self.supply_requested.get(index).copied().unwrap_or(true),
            &mut self.supply[..self.supply_len],
            &mut self.supply_origins,
        );
        self.supply_origins_len = used.len;
        self.required.supply_origins = self.required.supply_origins.max(used.required);
    }

    pub fn capture_border_stats(
        &mut self,
        memory: &impl Memory,
        base: usize,
        demanded: impl IntoIterator<Item = u64>,
    ) -> Outcome {
        let result = (|| {
            if !matches!(self.outcome, Outcome::Rows { .. }) {
                return Ok(self.outcome);
            }
            if self
                .border_index
                .windows(2)
                .any(|entries| entries[0].0 == entries[1].0)
            {
                return Err("border division status census duplicates a handle");
            }
            for handle in demanded {
                let position = self
                    .border_index
                    .binary_search_by_key(&handle, |entry| entry.0)
                    .map_err(|_| "demanded border army absent from division census")?;
                let unit = self.border_index[position].1;
                let cache = self.units[unit].maxima;
                let stats = self
                    .border_stats
                    .get_mut(cache)
                    .ok_or("border combat stat memo outside packet")?;
                let value = match *stats {
                    Some(value) => value,
                    None => {
                        let pointer = self
                            .maxima
                            .get(cache)
                            .ok_or("border combat stat source outside packet")?
                            .pointer;
                        let value = crate::csub_unit_definition::capture_division_combat_stats(
                            memory, base, pointer,
                        )?;
                        *stats = Some(value);
                        value
                    }
                };
                self.units[unit].border_combat_stats = Some(value);
            }
            Ok(self.outcome)
        })();
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        self.outcome
    }

    pub fn fail_border_stats(&mut self, reason: &'static str) {
        if matches!(self.outcome, Outcome::Rows { .. }) {
            self.outcome = Outcome::Unavailable { reason };
        }
    }
}

fn status_experience_fraction(manpower: i32, experience: i64) -> i64 {
    if manpower < 1 {
        0
    } else if (experience as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        0xffff_ffff
    } else {
        experience.wrapping_mul(100_000) / (i64::from(manpower) * 100_000)
    }
}

fn status_strength_fraction(strength: i64, maximum_hp: i64) -> i64 {
    if (strength as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        100_000
    } else {
        (strength.wrapping_mul(100_000) / maximum_hp).clamp(0, 100_000)
    }
}

pub fn project_status(packet: &StatusPacket, output: &mut [StatusRow]) -> Outcome {
    let result = (|| -> Result<Outcome, &'static str> {
        if let Outcome::Unavailable { reason } = packet.outcome {
            return Err(reason);
        }
        if packet.outcome.len() > packet.units.len() || packet.outcome.len() > output.len() {
            return Err("division status projection capacity invalid");
        }
        for (index, (unit, row)) in packet
            .units
            .iter()
            .zip(output.iter_mut())
            .take(packet.outcome.len())
            .enumerate()
        {
            let maxima = packet
                .maxima
                .get(unit.maxima)
                .ok_or("division status maxima index invalid")?;
            let supply = packet
                .supply
                .get(..packet.supply_len)
                .and_then(|supply| supply.get(index))
                .copied()
                .unwrap_or_default();
            let (maximum_hit_points, maximum_organisation) = maxima.effective();
            let move_flags = (u8::from(unit.exiled != 0) * StatusRow::MOVE_EXILED)
                | (u8::from(maxima.can_exfiltrate_from_coast != 0)
                    * StatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST)
                | (u8::from(unit.noncore_supply_bypass != 0)
                    * StatusRow::MOVE_NONCORE_SUPPLY_BYPASS)
                | (u8::from(unit.air_supply.is_some_and(|value| value > 0))
                    * StatusRow::MOVE_NONCORE_AIR)
                | (u8::from(unit.air_supply.is_none()) * StatusRow::MOVE_NONCORE_AIR_REFUSED);
            let country = packet
                .countries
                .get(unit.threshold)
                .ok_or("division status country index invalid")?;
            let modifier = packet.modifiers.value(country.modifier)?;
            let mut value = modifier.raw;
            if modifier.present {
                let metadata = packet
                    .attrition_metadata
                    .ok_or("division status metadata missing")??;
                if metadata
                    .flags
                    .ok_or("division status metadata flags missing")?
                    & crate::modifier_definition::CLAMP_MAXIMUM
                    != 0
                {
                    value = value.clamp(0, 100_000);
                }
            }
            let attrition_threshold = value.wrapping_add(100_000).max(0).wrapping_mul(
                packet
                    .attrition_define
                    .ok_or("division status attrition define missing")??,
            ) / 100_000;
            let has_order = unit
                .has_order
                .unwrap_or_else(|| packet.membership.outcome(unit.membership))?;
            *row = StatusRow {
                division: unit.division,
                maximum_hit_points,
                maximum_organisation,
                movement_organisation: unit.movement_organisation,
                entrenchment: unit.entrenchment,
                entrenchment_cap: unit.entrenchment_cap,
                supply_ratio: unit.supply_ratio,
                supply_grace_maximum: unit.supply_grace_maximum,
                supply_grace_remaining: unit.supply_grace_remaining,
                attrition_value: unit.attrition_value,
                attrition_threshold,
                attrition: u8::from(attrition_threshold < unit.attrition_value),
                exercising: match unit.exercising {
                    Some(decision) => decision?,
                    None => u8::from(match unit.exercise {
                        Some(inputs) => packet.exercise(inputs, Some(maximum_hit_points))?,
                        None => false,
                    }),
                },
                has_order: u8::from(has_order),
                invalid_orders: unit.invalid_orders,
                invalid_orders_reason: unit.invalid_orders_reason,
                strength_factor: unit.strength_factor,
                move_flags,
                out_of_supply_days: unit.out_of_supply_days,
                model_number: unit.model_number,
                camouflage: unit.camouflage,
                weather_factor: unit.weather_factor,
                border_combat_stats: unit.border_combat_stats,
                naming: None,
                country_ordinal: None,
                supply: supply.inputs.ok(),
                supply_origins_start: supply.origins_start,
                supply_failure: match supply.inputs {
                    Ok(inputs) => inputs.origins_len.err(),
                    Err(reason) => Some(reason),
                },
            };
        }
        Ok(packet.outcome)
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

type AcclimatizationWinner = ([u8; 0x40], Option<(u64, i64)>);

struct AcclimatizationBatch {
    bytes: [[u8; 32 * crate::acclimatization_entry::STRIDE]; 64],
    lengths: [usize; 64],
    copied: usize,
}

impl AcclimatizationBatch {
    fn new() -> Self {
        Self {
            bytes: [[0; 32 * crate::acclimatization_entry::STRIDE]; 64],
            lengths: [0; 64],
            copied: 0,
        }
    }

    fn capture(&mut self, memory: &impl Memory, base: usize, seeds: &[DivisionSeed]) {
        use crate::{acclimatization_data as data, acclimatization_entry as entry};
        self.lengths.fill(0);
        let mut addresses = [0_usize; 64];
        for (index, seed) in seeds.iter().enumerate() {
            let Some(object) = seed.acclimatization else {
                continue;
            };
            if Some(u64::from_le_bytes(object[..8].try_into().unwrap()))
                != engine_sites::at(base, data::VTABLE_RVA).map(|at| at as u64)
            {
                continue;
            }
            if let Some(array) = Array::from_header(
                object[data::DATA..data::DATA + 16].try_into().unwrap(),
                entry::STRIDE,
            ) && array.count != 0
            {
                addresses[index] = array.data;
                self.lengths[index] = (array.count as usize).min(32) * entry::STRIDE;
            }
        }
        if self.lengths.iter().all(|&len| len == 0) {
            self.copied = 0;
            return;
        }
        let mut rows = self.bytes.iter_mut();
        let mut destinations: [&mut [u8]; 64] =
            std::array::from_fn(|index| &mut rows.next().unwrap()[..self.lengths[index]]);
        self.copied = memory.copy_scattered_prefetched(addresses, &mut destinations);
        if self.copied < 64 {
            crate::batch_fallback::record(crate::batch_fallback::Batch::DivisionAcclimatization);
        }
    }

    fn first(&self, index: usize) -> Option<&[u8]> {
        (index < self.copied && self.lengths[index] != 0)
            .then(|| &self.bytes[index][..self.lengths[index]])
    }
}

fn acclimatization_winner(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    gathered_first: Option<&[u8]>,
) -> Result<AcclimatizationWinner, &'static str> {
    use crate::{acclimatization_data as data, acclimatization_entry as entry};
    let object = seed
        .acclimatization
        .ok_or("division presentation field unreadable")?;
    if Some(u64::from_le_bytes(object[..8].try_into().unwrap()))
        != engine_sites::at(base, data::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("division acclimatization identity mismatch");
    }
    let entries = Array::from_header(
        object[data::DATA..data::DATA + 16].try_into().unwrap(),
        entry::STRIDE,
    )
    .ok_or("division acclimatization entries invalid")?;
    let count = entries.count as usize;
    const SPAN: usize = 32;
    let mut bytes = [std::mem::MaybeUninit::uninit(); SPAN * entry::STRIDE];
    let mut strongest = data::Strongest::new();
    for first in (0..count).step_by(SPAN) {
        let len = (count - first).min(SPAN);
        let at = entries
            .element(first as u32, entry::STRIDE)
            .ok_or("division acclimatization entries invalid")?;
        let raw_entries = match gathered_first.filter(|_| first == 0) {
            Some(gathered) => gathered,
            None => crate::memory::read_slice_uninit(memory, at, &mut bytes[..len * entry::STRIDE])
                .ok_or("division acclimatization entries unreadable")?,
        };
        for raw in raw_entries.chunks_exact(entry::STRIDE) {
            strongest.offer(
                u64::from_le_bytes(
                    raw[entry::CATEGORY..entry::CATEGORY + 8]
                        .try_into()
                        .unwrap(),
                ),
                i64::from_le_bytes(raw[entry::WEIGHT..entry::WEIGHT + 8].try_into().unwrap()),
            );
        }
    }
    Ok((object, strongest.winner()))
}

#[derive(Default)]
struct CategoryFactors {
    entries: [(u64, i64); 8],
    len: usize,
}

impl CategoryFactors {
    fn factor(&mut self, memory: &impl Memory, category: u64) -> Result<i64, &'static str> {
        if let Some(&(_, factor)) = self.entries[..self.len]
            .iter()
            .find(|(held, _)| *held == category)
        {
            return Ok(factor);
        }
        let factor = usize::try_from(category)
            .ok()
            .and_then(|at| at.checked_add(crate::acclimatization_data::CATEGORY_FACTOR))
            .and_then(|address| crate::read::<i64>(memory, address))
            .ok_or("division acclimatization category factor unreadable")?;
        if let Some(entry) = self.entries.get_mut(self.len) {
            *entry = (category, factor);
            self.len += 1;
        }
        Ok(factor)
    }
}

fn weather_factor(
    memory: &impl Memory,
    object: &[u8; 0x40],
    winner: Option<(u64, i64)>,
    factors: &mut CategoryFactors,
) -> Result<i64, &'static str> {
    use crate::acclimatization_data as data;
    let active = u64::from_le_bytes(object[data::ACTIVE..data::ACTIVE + 8].try_into().unwrap());
    let Some((_, weight)) = winner.filter(|(category, _)| active != 0 && *category == active)
    else {
        return Ok(100_000);
    };
    let maximum = i64::from_le_bytes(object[data::MAXIMUM..data::MAXIMUM + 8].try_into().unwrap());
    Ok(data::movement_factor(
        weight,
        maximum,
        factors.factor(memory, active)?,
    ))
}

pub fn counter_dig_in_factor(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
) -> Result<i64, &'static str> {
    let Some(root) = crate::carmy_leader::movement_accumulator_seeded(memory, base, seed)? else {
        return Ok(100_000);
    };
    crate::modifier_accumulator::requested_effective_at(
        memory,
        base,
        root,
        crate::modifier_keys::COMBAT_ENTRENCHMENT,
    )
    .map(|value| 100_000_i64.wrapping_add(value))
}

pub struct DigInMemo {
    leaders: crate::carmy_leader::LeaderMemo,
    leaders_stale: bool,
    values: crate::carmy_leader::PassTable<Result<i64, &'static str>>,
}

impl DigInMemo {
    pub fn try_with_capacity(divisions: usize) -> std::io::Result<Self> {
        Ok(Self {
            leaders: crate::carmy_leader::LeaderMemo::try_with_capacity(divisions)?,
            leaders_stale: false,
            values: crate::carmy_leader::PassTable::try_with_capacity(divisions)?,
        })
    }

    pub fn vacant() -> Self {
        Self {
            leaders: crate::carmy_leader::LeaderMemo::vacant(),
            leaders_stale: false,
            values: crate::carmy_leader::PassTable::vacant(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.leaders.capacity().min(self.values.capacity())
    }

    pub fn allocated_bytes(&self) -> usize {
        self.leaders.allocated_bytes() + self.values.allocated_bytes()
    }

    pub fn clear(&mut self) {
        self.leaders_stale = true;
        self.values.clear();
    }
}

pub fn counter_dig_in_factor_memoized(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    memo: &mut DigInMemo,
) -> Result<i64, &'static str> {
    counter_dig_in_selection_memoized(memory, base, seed, memo).map(|(factor, _)| factor)
}

pub const COMMANDER_BLOCK_NONE: u8 = 0;
pub const COMMANDER_BLOCK_ORDINARY: u8 = 1;
pub const COMMANDER_BLOCK_SEPARATE_HQ: u8 = 2;

pub fn counter_dig_in_selection_memoized(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    memo: &mut DigInMemo,
) -> Result<(i64, u8), &'static str> {
    counter_dig_in_selection_with(memory, base, seed, memo, |root| {
        commander_entrenchment(memory, base, root)
    })
}

pub fn counter_dig_in_selection_with(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    memo: &mut DigInMemo,
    entrenchment: impl FnOnce(usize) -> Result<i64, &'static str>,
) -> Result<(i64, u8), &'static str> {
    let block = match seed.selected {
        Some(selected) => selected.block,
        None => {
            if std::mem::take(&mut memo.leaders_stale) {
                memo.leaders.clear();
            }
            crate::carmy_leader::movement_block_memoized(memory, base, seed, &mut memo.leaders)
        }
    };
    let Some((root, separate)) = block? else {
        return Ok((100_000, COMMANDER_BLOCK_NONE));
    };
    let block = if separate {
        COMMANDER_BLOCK_SEPARATE_HQ
    } else {
        COMMANDER_BLOCK_ORDINARY
    };
    memo.values
        .get_or(root, || entrenchment(root))
        .map(|value| (100_000_i64.wrapping_add(value), block))
}

pub fn commander_entrenchment(
    memory: &impl Memory,
    base: usize,
    root: usize,
) -> Result<i64, &'static str> {
    crate::modifier_accumulator::requested_effective_at(
        memory,
        base,
        root,
        crate::modifier_keys::COMBAT_ENTRENCHMENT,
    )
}

#[cfg(test)]
fn status_counter_scalars(
    memory: &impl Memory,
    army: usize,
) -> Result<(i64, i64, i64, i64), &'static str> {
    use crate::memory::RecordField;
    const FIELDS: [RecordField; 4] = [
        RecordField {
            source_offset: COUNTER_ATTRITION,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: ENTRENCHMENT,
            destination_offset: 8,
            len: 8,
        },
        RecordField {
            source_offset: ENTRENCHMENT_CAP,
            destination_offset: 16,
            len: 8,
        },
        RecordField {
            source_offset: COUNTER_SUPPLY_STATUS,
            destination_offset: 24,
            len: 8,
        },
    ];
    let mut bytes = [0; 32];
    if memory.copy_records(
        crate::memory::RecordSources::Pointers(&[army]),
        &crate::memory::RecordLayout::new(&FIELDS, bytes.len()).unwrap(),
        &mut bytes,
    ) != FIELDS.len()
    {
        return Err("division presentation field unreadable");
    }
    Ok((
        i64::from_le_bytes(bytes[..8].try_into().unwrap()),
        i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        i64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        i64::from_le_bytes(bytes[24..].try_into().unwrap()),
    ))
}

#[cfg(test)]
fn exercise_level(memory: &impl Memory, base: usize, army: usize) -> Result<i32, &'static str> {
    let mut packet = StatusPacket::new(1).unwrap();
    let inputs = packet.capture_level(memory, base, army, None)?;
    packet.level(inputs)
}

#[cfg(test)]
fn exercising(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    group: usize,
    maximum_hp: i64,
) -> Result<bool, &'static str> {
    let mut packet = StatusPacket::new(1).unwrap();
    if packet.training(memory, group)? == 0 {
        return Ok(false);
    }
    let mut seeds = [DivisionSeed {
        pointer: army,
        ..DivisionSeed::default()
    }];
    capture_division_seeds(memory, &mut seeds);
    packet.units.push(StatusUnitInputs {
        group,
        ..StatusUnitInputs::default()
    });
    packet
        .maxima
        .push(crate::csub_unit_definition::DivisionMaxima {
            strength: maximum_hp,
            ..crate::csub_unit_definition::DivisionMaxima::default()
        });
    let inputs = packet.capture_exercise(
        memory,
        base,
        &seeds[0],
        0,
        None,
        |_, raw| cgame_state::autonomous_country(memory, base, state, raw),
        |_| Ok(maximum_hp),
    )?;
    packet.exercise(inputs, Some(maximum_hp))
}

#[cfg(test)]
pub fn walk_status(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[crate::cequipment_variant::pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [StatusRow],
) -> Outcome {
    let mut packet = StatusPacket::new(sources.len()).unwrap();
    status_test_capture(
        memory,
        base,
        state,
        sources,
        source_outcome,
        output,
        &mut packet,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn status_test_capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[crate::cequipment_variant::pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [StatusRow],
    packet: &mut StatusPacket,
) -> Outcome {
    let mut seeds: Vec<_> = sources
        .iter()
        .map(|source| DivisionSeed {
            pointer: source.pointer,
            division: source.division,
            ..DivisionSeed::default()
        })
        .collect();
    let count = seeds.len().min(output.len());
    if !matches!(source_outcome, Outcome::Unavailable { .. }) {
        capture_division_seeds(memory, &mut seeds[..count]);
    }
    let source_outcome = match source_outcome {
        Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
        Outcome::Rows { len } if len == count => Outcome::Rows { len },
        Outcome::Rows { len } => Outcome::Truncated {
            len: count,
            unread_rows: (len - count) as u32,
        },
        Outcome::Truncated { len, unread_rows } => Outcome::Truncated {
            len: count,
            unread_rows: unread_rows + (len - count) as u32,
        },
    };
    packet.capture(
        memory,
        base,
        state,
        &seeds[..count],
        source_outcome,
    );
    packet.project(output)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn walk_status_with_membership(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[crate::cequipment_variant::pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [StatusRow],
    membership: &mut MembershipPacket,
) -> Outcome {
    let mut packet = StatusPacket::new(sources.len()).unwrap();
    std::mem::swap(&mut packet.membership, membership);
    let outcome = status_test_capture(
        memory,
        base,
        state,
        sources,
        source_outcome,
        output,
        &mut packet,
    );
    std::mem::swap(&mut packet.membership, membership);
    outcome
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    countries: Array,
    links: Array,
    provinces: Array,
}

impl<M: Memory> Reader<'_, M> {
    fn country_pointer(&self, slot: u32) -> Result<usize, &'static str> {
        self.countries
            .element(slot, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("country pointer unreadable")
    }

    fn canonical(&self, id: u32) -> Result<u32, &'static str> {
        if id == 0 {
            return Err("country id is zero");
        }
        self.links
            .element(id, 4)
            .and_then(|p| read::<u32>(self.memory, p))
            .filter(|&slot| slot > 0 && slot < self.countries.count)
            .ok_or("country link unreadable or outside country table")
    }

    fn vtable(&self, pointer: usize, rva: Rva, reason: &'static str) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, rva).ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    fn row(
        &self,
        pointer: usize,
        canonical: u32,
        country: usize,
        theatres: &mut crate::ctheatre::UnitTheatres,
    ) -> Result<Row, &'static str> {
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("division address overflow")
        };
        self.vtable(pointer, VTABLE_RVA, "division primary vtable mismatch")?;
        self.vtable(
            at(SECONDARY_BASE)?,
            SECONDARY_VTABLE_RVA,
            "division secondary vtable mismatch",
        )?;
        let id =
            read::<u64>(self.memory, at(cunit::HANDLE)?).ok_or("division handle unreadable")?;
        if id == 0 {
            return Err("non-null division has null handle");
        }
        let roster_owner = read::<u32>(self.memory, at(cunit::ROSTER_OWNER)?)
            .ok_or("division roster owner unreadable")?;
        if self.canonical(roster_owner)? != canonical {
            return Err("division owner differs from roster country");
        }
        let logical_owner = read::<u32>(self.memory, at(cunit::LOGICAL_OWNER)?)
            .ok_or("division logical owner unreadable")?;
        let expeditionary_owner = read::<u32>(self.memory, at(cunit::EXPEDITIONARY_OWNER)?)
            .ok_or("division expeditionary owner unreadable")?;
        let location = read::<u64>(self.memory, at(cunit::LOCATION)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("division location pointer unreadable")?;
        let province = self.province(location)?;
        let template_pointer = read::<u64>(self.memory, at(STANDARD_TEMPLATE)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("division template pointer unreadable")?;
        let template = self.template(template_pointer)?;
        let reveal_skip = if template_pointer == 0 {
            0
        } else {
            template_pointer
                .checked_add(creferenced_division_template::REVEAL_SKIP)
                .and_then(|address| read::<u8>(self.memory, address))
                .ok_or("division template reveal skip unreadable")?
        };
        let hit_points =
            read::<i64>(self.memory, at(STRENGTH)?).ok_or("division hit points unreadable")?;
        let organisation = read::<i64>(self.memory, at(ORGANISATION)?)
            .ok_or("division organisation unreadable")?;
        let group = read::<u64>(self.memory, at(ORDERS_GROUP)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("division orders group pointer unreadable")?;
        let orders_group = self.orders_group(group)?;
        let theatre = read::<u64>(self.memory, at(cunit::THEATRE)?)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("division theatre pointer unreadable")
            .and_then(|pointer| theatres.handle(self.memory, self.base, country, pointer))
            .ok();
        Ok(Row {
            id,
            roster_owner,
            logical_owner,
            expeditionary_owner,
            province,
            template,
            hit_points,
            organisation,
            orders_group,
            reveal_skip,
            theatre,
        })
    }

    fn orders_group(&self, pointer: usize) -> Result<u64, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let identity = read::<[u8; 16]>(self.memory, pointer)
            .ok_or("division orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = engine_sites::at(self.base, corders_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let group = engine_sites::at(self.base, carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if vtable != normal && vtable != group {
            return Err("division orders group vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != corders_group::REFERENCE_TYPE {
            return Err("division orders group reference type mismatch");
        }
        Ok(handle)
    }

    fn province(&self, pointer: usize) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let expected =
            engine_sites::at(self.base, cprovince::VTABLE_RVA).ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) != Some(expected as u64) {
            return Err("division province vtable mismatch");
        }
        let id = pointer
            .checked_add(cprovince::ID)
            .and_then(|p| read::<u32>(self.memory, p))
            .ok_or("division province id unreadable")?;
        if id == 0 {
            return Err("non-null division province has zero id");
        }
        let registered = self
            .provinces
            .element(id, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .ok_or("division province slot unreadable or out of range")?;
        if registered != pointer as u64 {
            return Err("division province differs from registered slot");
        }
        Ok(id)
    }

    #[cfg(test)]
    fn province_cached(
        &self,
        pointer: usize,
        inputs: Option<&MovementInputs>,
    ) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        let expected =
            engine_sites::at(self.base, cprovince::VTABLE_RVA).ok_or("module address overflow")?;
        if shared_presentation_read::<u64>(self.memory, inputs, pointer) != Some(expected as u64) {
            return Err("division province vtable mismatch");
        }
        let id = pointer
            .checked_add(cprovince::ID)
            .and_then(|p| shared_presentation_read::<u32>(self.memory, inputs, p))
            .ok_or("division province id unreadable")?;
        if id == 0 {
            return Err("non-null division province has zero id");
        }
        let registered = self
            .provinces
            .element(id, 8)
            .and_then(|p| shared_presentation_read::<u64>(self.memory, inputs, p))
            .ok_or("division province slot unreadable or out of range")?;
        if registered != pointer as u64 {
            return Err("division province differs from registered slot");
        }
        Ok(id)
    }

    fn template(&self, pointer: usize) -> Result<u64, &'static str> {
        template_handle(self.memory, self.base, pointer)
    }
}

mod presentation_demand;
pub use presentation_demand::{PresentationDemand, presentation_demand};

#[cfg(test)]
#[path = "carmy/border/tests.rs"]
mod border_tests;
#[cfg(test)]
mod tests;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod bench;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod status_bench;

pub use game_model::movement::army::{MovementSpeedInputs, NormalEdgeOperands, NormalSpeedOperands, PreparedNormalSpeed};

use game_model::movement::army::MOVEMENT_ONE;
