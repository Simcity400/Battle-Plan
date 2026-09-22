//! Raw dependencies of reinforcement speed140C76E90 ->140C70670.
//!
//! The caller passes (ignore_terrain=1, ignore_combat=1); effective route speed
//! and its organisation/fuel factors are NOT this getter. No displayed speed,
//! chance, modifier sum, acclimatization fraction or supply ratio is composed.

use crate::{Memory, carmy, ccountry, cgame_state, cpdx_array::Array, cprovince, cunit, read};

pub const MODIFIER_LIMIT: usize = 96;
pub const SUPPLY_TERM_LIMIT: usize = 64;
pub const ACCLIMATIZATION_LIMIT: usize = 64;
pub const TRANSPORT_TERM_LIMIT: usize = 64;
const ARRAY_LIMIT: u32 = 4096;

/// Scopes:0 country;1 selected contextual leader;2 location-state;
///3 province weather;4 region weather;5 priority subunit;6 category;
///7 transport weather;8 transport static source;9 transport country reduction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierTerm {
    pub scope: u8,
    pub key: u32,
    pub raw: i64,
    pub present: u8,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SupplyTerm {
    pub subunit: u32,
    pub count: i32,
    pub special: u8,
}

/// `active` is a reference join: this entry's definition equals the currently
/// selected acclimatization definition. Native order and ties are retained.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AcclimatizationTerm {
    pub weight: i64,
    pub active: u8,
}

/// Each eligible enemy in native diplomacy order; all three signed values
/// are copied before any difference, product, total or mean is evaluated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportTerm {
    pub country: u32,
    pub regional_value: i64,
    pub own_intel: i64,
    pub enemy_intel: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    ///0 acclimatization maximum;1 active-definition coefficient;
    ///2 supply+600;3 supply+610;4 supply+40;5 supply-days+498;
    ///6 supply-hours define33191C0;7 penalty-days define3319394;
    ///8 supply-speed define33194E0;9 selected noncore define;
    ///10 air-supply+58;11 retreat define3319758;12 shattered define3319808;
    ///13/14 friendly/hostile air;15 impact;16 AA;17 air scale331DDD8;
    ///18/19 AA denominator/reduction;20/21 infrastructure definition limits;
    ///22 infrastructure fallback limit;23 per-level;24 healthy level;
    ///25 transport maximum331E100;26 minimum intel331D5B0;27 maximum intel331D620;
    ///28 intel reduction331E550;29 static modifier category mask;
    ///30..31 reserved, always zero. Absent branch operands are zero and their
    ///presence is given by flags; zero does not assert that native storage is zero.
    pub scalar_values: [i64; 32],
    ///0 raw retreat;1 raw shattered+24D;2 raw supply-bypass+2AE;
    ///3 raw core-bypass+2E;4 active-acclimatization reference present;
    ///5 route-attack branch;6 controller branch(0 neither,1 friendly,2 enemy);
    ///7 air-cache branch;8 noncore branch;9 infrastructure branch;
    ///10 raw infrastructure lookup byte;11 normal supply branch=1, naval transfer=2.
    pub flags: [u8; 12],
    ///0 location province;1 next route province(0 null);2 location state;
    ///3 infrastructure state;4 selected subunit lexer id;5 feature flags;
    ///6 eligibility callback(0 null,1 authenticated getter);7 region index.
    pub ids: [u32; 8],
    pub modifier_count: usize,
    pub supply_term_count: usize,
    pub acclimatization_count: usize,
    pub transport_term_count: usize,
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    p: usize,
    o: usize,
) -> Result<T, &'static str> {
    p.checked_add(o)
        .and_then(|at| read(memory, at))
        .ok_or("reinforcement input unreadable")
}

/// Preserve the original field order, including a late checked-address failure.
/// A failed field may be partial; no later span is read and nothing is retried.
fn copy_input_spans<const N: usize>(
    memory: &impl Memory,
    sources: [(usize, usize); N],
    destinations: &mut [&mut [u8]; N],
) -> Result<(), &'static str> {
    let addresses = sources.map(|(pointer, offset)| pointer.checked_add(offset));
    if addresses.iter().any(Option::is_none) {
        // No read has started. Preserve the scalar prefix instead of turning
        // an arithmetic failure into a request at a sentinel address.
        for (address, destination) in addresses.into_iter().zip(destinations) {
            let address = address.ok_or("reinforcement input unreadable")?;
            if !crate::read_slice(memory, address, destination) {
                return Err("reinforcement input unreadable");
            }
        }
        return Ok(());
    }
    if memory.copy_scattered(addresses.map(Option::unwrap), destinations) == N {
        Ok(())
    } else {
        Err("reinforcement input unreadable")
    }
}

fn formation_flags(memory: &impl Memory, army: usize) -> Result<[u8; 4], &'static str> {
    let mut values = [[0; 1]; 4];
    copy_input_spans(
        memory,
        [carmy::RETREAT, 0x24d, 0x2ae, 0x2e].map(|offset| (army, offset)),
        &mut values.each_mut().map(|value| value.as_mut_slice()),
    )?;
    Ok(values.map(|value| value[0]))
}

fn supply_scalars(
    memory: &impl Memory,
    base: usize,
    army: usize,
) -> Result<[i64; 8], &'static str> {
    let mut values = [[0; 8]; 8];
    let mut fields = values.iter_mut();
    copy_input_spans(
        memory,
        [
            (army, 0x600),
            (army, 0x610),
            (army, 0x40),
            (army, 0x58),
            (army, 0x498),
            (base, 0x0331_91c0),
            (base, 0x0331_9394),
            (base, 0x0331_94e0),
        ],
        &mut std::array::from_fn(|index| {
            &mut fields.next().unwrap()[..if (4..7).contains(&index) { 4 } else { 8 }]
        }),
    )?;
    Ok(std::array::from_fn(|index| {
        if (4..7).contains(&index) {
            i64::from(i32::from_le_bytes(values[index][..4].try_into().unwrap()))
        } else {
            i64::from_le_bytes(values[index])
        }
    }))
}

fn pointer(memory: &impl Memory, p: usize, o: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, p, o).map(|p| p as usize)
}

fn array(memory: &impl Memory, p: usize, o: usize, stride: usize) -> Result<Array, &'static str> {
    let header = field(memory, p, o)?;
    let array = Array::from_header(header, stride).ok_or("reinforcement array invalid")?;
    if array.count > ARRAY_LIMIT {
        return Err("reinforcement array bound exceeded");
    }
    Ok(array)
}

fn element(array: Array, index: u32, stride: usize) -> Result<usize, &'static str> {
    array
        .element(index, stride)
        .ok_or("reinforcement array index invalid")
}

fn auth(memory: &impl Memory, base: usize, p: usize, rva: usize) -> Result<(), &'static str> {
    if p == 0 || base.checked_add(rva).map(|n| n as u64) != Some(field(memory, p, 0)?) {
        return Err("reinforcement dependency identity invalid");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_modifiers(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    scope: u8,
    keys: &[u32],
    output: &mut [ModifierTerm],
    len: &mut usize,
) -> Result<(), &'static str> {
    if accumulator == 0 {
        return Ok(());
    }
    if keys.len() > MODIFIER_LIMIT.saturating_sub(*len)
        || keys.len() > output.len().saturating_sub(*len)
    {
        return Err("reinforcement modifier arena full");
    }
    let start = *len;
    for key in keys {
        if *key == 0 {
            return Err("reinforcement modifier key invalid");
        }
        output[*len] = ModifierTerm {
            scope,
            key: *key,
            ..ModifierTerm::default()
        };
        *len += 1;
    }
    // The owning pass supplies this authenticated complete raw country snapshot.
    // Never memoize a derived effective value.
    let mut shared = true;
    for term in &mut output[start..*len] {
        if let Some(value) = memory.modifier_accumulator_value(accumulator, term.key) {
            let value = value?;
            term.raw = value.raw;
            term.present = u8::from(value.present);
        } else {
            shared = false;
        }
    }
    if !shared {
        let entries = array(memory, accumulator, 0, 16)?;
        for term in &mut output[start..*len] {
            term.raw = 0;
            term.present = 0;
        }
        let mut batch = [0_u8; 64 * 16];
        let mut previous = 0;
        for first in (0..entries.count as usize).step_by(64) {
            let count = (entries.count as usize - first).min(64);
            if !crate::read_slice(
                memory,
                element(entries, first as u32, 16)?,
                &mut batch[..count * 16],
            ) {
                return Err("reinforcement modifier entries unreadable");
            }
            for entry in batch[..count * 16].chunks_exact(16) {
                let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
                if key <= previous {
                    return Err("reinforcement modifier order invalid");
                }
                previous = key;
                for term in &mut output[start..*len] {
                    if term.key == key {
                        term.present = 1;
                        term.raw = i64::from_le_bytes(entry[8..16].try_into().unwrap());
                    }
                }
            }
        }
    }
    use crate::modifier_definition as metadata;
    let table = pointer(memory, base, metadata::TABLE_RVA)?;
    for term in &mut output[start..*len] {
        if term.present != 0 {
            let row = table
                .checked_add(term.key as usize * metadata::STRIDE)
                .filter(|_| table != 0)
                .ok_or("reinforcement modifier metadata null")?;
            let bytes = field::<[u8; 12]>(memory, row, metadata::FLAGS)?;
            term.metadata_flags = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            term.feature_selector = u32::from_le_bytes(bytes[8..].try_into().unwrap());
        }
    }
    Ok(())
}

/// Raw accumulator selection for1409CC140; its hash selects a source rather
/// than calculating a displayed numeric result. All four possible speed keys
/// are kept so controller/attacking choices cannot lose an operand.
fn state_accumulator(
    memory: &impl Memory,
    base: usize,
    game: usize,
    state: usize,
    owner: u32,
) -> Result<usize, &'static str> {
    use crate::cstate;
    auth(memory, base, state, cstate::VTABLE_RVA)?;
    let links = array(memory, game, cgame_state::COUNTRY_LINKS, 4)?;
    let canonical = |raw: u32| field::<u32>(memory, element(links, raw, 4)?, 0);
    let hash = canonical(owner)?;
    let header = field::<[u8; 17]>(memory, state, cstate::COUNTRY_MODIFIERS)?;
    let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let buckets = mask
        .checked_add(1)
        .filter(|n| n.is_power_of_two())
        .ok_or("reinforcement state hash invalid")?;
    let extent = buckets
        .checked_add(u32::from(header[16]))
        .ok_or("reinforcement state hash overflow")?;
    let mut selected = None;
    for probe in 1..=255_u32 {
        let index = (hash & mask)
            .checked_add(probe - 1)
            .filter(|n| *n < extent)
            .ok_or("reinforcement state hash extent invalid")?;
        let bucket = data
            .checked_add(index as usize * cstate::COUNTRY_MODIFIER_STRIDE)
            .filter(|_| data != 0)
            .ok_or("reinforcement state hash null")?;
        let distance = field::<u8>(memory, bucket, 4)?;
        if distance == 0 || probe > u32::from(distance) {
            break;
        }
        let raw = field::<u32>(memory, bucket, 8)?;
        if raw == owner || (raw != 0 && owner != 0 && canonical(raw)? == hash) {
            selected = Some(bucket);
            break;
        }
        if probe == 255 {
            return Err("reinforcement state hash bound exceeded");
        }
    }
    selected
        .map_or_else(
            || state.checked_add(cstate::RESOURCE_MODIFIER + 0x10),
            |p| p.checked_add(0x20),
        )
        .ok_or("reinforcement state accumulator overflow")
}

fn acclimatization(
    memory: &impl Memory,
    army: usize,
    row: &mut ContextRow,
    output: &mut [AcclimatizationTerm],
) -> Result<(), &'static str> {
    let object = army
        .checked_add(carmy::ACCLIMATIZATION)
        .ok_or("reinforcement acclimatization overflow")?;
    let active = pointer(memory, object, 0x20)?;
    row.flags[4] = u8::from(active != 0);
    if active == 0 {
        return Ok(());
    }
    let entries = array(memory, object, 8, 16)?;
    let count = entries.count as usize;
    if count > ACCLIMATIZATION_LIMIT {
        return Err("reinforcement acclimatization bound exceeded");
    }
    if count > output.len() {
        return Err("reinforcement acclimatization arena full");
    }
    for (index, term) in output[..count].iter_mut().enumerate() {
        let entry = element(entries, index as u32, 16)?;
        *term = AcclimatizationTerm {
            weight: field(memory, entry, 8)?,
            active: u8::from(pointer(memory, entry, 0)? == active),
        };
    }
    row.scalar_values[0] = field(memory, object, 0x38)?;
    row.scalar_values[1] = field(memory, active, 0x30)?;
    row.acclimatization_count = count;
    Ok(())
}

fn supply_terms(
    memory: &impl Memory,
    base: usize,
    template: usize,
    row: &mut ContextRow,
    output: &mut [SupplyTerm],
) -> Result<(), &'static str> {
    let entries = array(memory, template, 0xc0, 16)?;
    let count = entries.count as usize;
    if count > SUPPLY_TERM_LIMIT {
        return Err("reinforcement supply term bound exceeded");
    }
    if count > output.len() {
        return Err("reinforcement supply term arena full");
    }
    for (index, term) in output[..count].iter_mut().enumerate() {
        let entry = element(entries, index as u32, 16)?;
        let unit = pointer(memory, entry, 0)?;
        auth(memory, base, unit, crate::csub_unit_definition::VTABLE_RVA)?;
        *term = SupplyTerm {
            subunit: field(memory, unit, 8)?,
            count: field(memory, entry, 8)?,
            special: field(memory, unit, 0x5b7)?,
        };
    }
    row.supply_term_count = count;
    Ok(())
}

fn infrastructure(
    memory: &impl Memory,
    base: usize,
    state: usize,
    row: &mut ContextRow,
) -> Result<(), &'static str> {
    use crate::{cbuilding, cbuilding_database, cbuilding_status, cbuilding_template, cstate};
    if state == 0 {
        return Ok(());
    }
    auth(memory, base, state, cstate::VTABLE_RVA)?;
    row.flags[9] = 1;
    row.ids[3] = field(memory, state, cstate::ID)?;
    let database = pointer(memory, base, cbuilding_database::SINGLETON_RVA)?;
    let definition = pointer(memory, database, 0x3b0)?;
    if database == 0 || definition == 0 {
        return Err("reinforcement infrastructure definition null");
    }
    row.scalar_values[20] = i64::from(field::<i32>(memory, definition, 0x2bc)?);
    if row.scalar_values[20] < 1 {
        row.scalar_values[21] = i64::from(field::<i32>(memory, definition, 0x2b8)?);
        if row.scalar_values[21] < 1 {
            row.scalar_values[22] = i64::from(field::<i32>(memory, base, 0x0331_b5b4)?);
        }
    }
    row.scalar_values[23] = field(memory, base, 0x0331_aac0)?;
    row.flags[10] = field(
        memory,
        definition,
        cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
    )?;
    let status = state
        .checked_add(cstate::BUILDING_STATUS)
        .ok_or("reinforcement building status overflow")?;
    if row.flags[10] != 0
        && field::<i32>(memory, status, cbuilding_status::BUILDING_INDEX_COUNT)? != 0
    {
        let indices = array(memory, status, cbuilding_status::BUILDING_INDICES, 4)?;
        let ordinal = field::<u32>(memory, definition, cbuilding_template::DATABASE_ORDINAL)?;
        let index = field::<i32>(memory, element(indices, ordinal, 4)?, 0)?;
        if index != cbuilding_status::NO_BUILDING_INDEX {
            let buildings = array(memory, status, cbuilding_status::BUILDINGS, 8)?;
            let building = pointer(
                memory,
                element(
                    buildings,
                    u32::try_from(index)
                        .map_err(|_| "reinforcement infrastructure index invalid")?,
                    8,
                )?,
                0,
            )?;
            if building != 0 {
                if pointer(memory, building, cbuilding::TEMPLATE)? != definition {
                    return Err("reinforcement infrastructure template mismatch");
                }
                row.scalar_values[24] =
                    i64::from(field::<i16>(memory, building, cbuilding::HEALTHY_LEVEL)?);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn air(
    memory: &impl Memory,
    base: usize,
    game: usize,
    army: usize,
    stats: usize,
    location: usize,
    row: &mut ContextRow,
) -> Result<(), &'static str> {
    let owner = ccountry::Reader::new(memory, base, game)?.resolve(field(
        memory,
        army,
        cunit::LOGICAL_OWNER,
    )?)?;
    let diplomacy = pointer(memory, owner.pointer, 0xf88)?;
    let mut active = field::<i32>(memory, diplomacy, 0xa4)? != 0;
    if !active {
        let relations = array(memory, diplomacy, 0x20, 8)?;
        for i in 0..relations.count {
            let relation = pointer(memory, element(relations, i, 8)?, 0)?;
            if field::<u32>(memory, relation, 8)? == 0x380a {
                active = true;
                break;
            }
        }
    }
    let region = pointer(memory, location, cprovince::STRATEGIC_REGION)?;
    if !active || region == 0 {
        return Ok(());
    }
    let manager = pointer(memory, game, cgame_state::STRATEGIC_AIR_MANAGER)?;
    let countries = array(memory, manager, 0x30, 8)?;
    let country = pointer(memory, element(countries, owner.canonical_slot, 8)?, 0)?;
    let index = field::<i32>(memory, region, 0x60)?;
    let regions = array(memory, country, 0xe0, 0xa0)?;
    let index = u32::try_from(index).map_err(|_| "reinforcement air region negative")?;
    let entry = element(regions, index, 0xa0)?;
    row.flags[7] = 1;
    row.ids[7] = index;
    row.scalar_values[13] = field(memory, entry, 0)?;
    row.scalar_values[14] = field(memory, entry, 8)?;
    if row.scalar_values[14] > 0 {
        row.scalar_values[15] = field(memory, entry, 0x18)?;
        row.scalar_values[16] = field(memory, stats, 0x220)?;
        row.scalar_values[17] = field(memory, base, 0x0331_ddd8)?;
        if row.scalar_values[16] != 0 {
            row.scalar_values[18] = field(memory, base, 0x0331_de18)?;
            row.scalar_values[19] = field(memory, base, 0x0331_dd98)?;
        }
    }
    Ok(())
}

fn transport_intel_lane(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    source: usize,
    target: usize,
    target_country: ccountry::Country,
) -> Result<i64, &'static str> {
    use crate::{ccountry_intel, country_intel_row};
    let raw = field::<i32>(memory, target, ccountry_intel::OWNER)?;
    if raw <= 0 {
        return Ok(0);
    }
    let resolved = countries.resolve(raw as u32)?;
    if resolved.pointer != target_country.pointer {
        return Err("reinforcement transport intel owner mismatch");
    }
    auth(memory, base, source, ccountry_intel::VTABLE_RVA)?;
    let rows = array(
        memory,
        source,
        ccountry_intel::DATA,
        country_intel_row::STRIDE,
    )?;
    let value = match rows.element(resolved.canonical_slot, country_intel_row::STRIDE) {
        Some(row) => field(memory, row, country_intel_row::NAVY)?,
        None => 0,
    };
    Ok(value)
}

fn transport_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    army: usize,
    location: usize,
    modifiers: &mut [ModifierTerm],
    terms: &mut [TransportTerm],
) -> Result<ContextRow, &'static str> {
    use crate::{
        ccountry_intel, cdiplomacy_status, cstatic_modifier, cstrategic_navy_manager,
        cstrategic_region, cweather_manager, modifier_definition as metadata, sweather_per_region,
    };
    let mut row = ContextRow::default();
    row.flags[11] = 2;
    row.ids[0] = field(memory, location, cprovince::ID)?;
    row.ids[5] = field(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA)?;
    let callback = pointer(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA)?;
    row.ids[6] = if callback == 0 {
        0
    } else if base.checked_add(metadata::ELIGIBILITY_FLAGS_GETTER_RVA) == Some(callback) {
        1
    } else {
        return Err("reinforcement modifier callback unsupported");
    };
    let region = pointer(memory, location, cprovince::STRATEGIC_REGION)?;
    auth(memory, base, region, cstrategic_region::VTABLE_RVA)?;
    let region_id = field::<u32>(memory, region, cstrategic_region::ID)?;
    row.ids[7] = region_id;
    let manager = pointer(memory, game, cgame_state::WEATHER_MANAGER)?;
    auth(memory, base, manager, cweather_manager::VTABLE_RVA)?;
    let weather = array(
        memory,
        manager,
        cweather_manager::REGIONS,
        sweather_per_region::STRIDE,
    )?;
    if let Some(record) = weather.element(region_id, sweather_per_region::STRIDE) {
        auth(memory, base, record, sweather_per_region::VTABLE_RVA)?;
        if field::<u32>(memory, record, sweather_per_region::ID)? != region_id {
            return Err("reinforcement transport weather identity mismatch");
        }
        append_modifiers(
            memory,
            base,
            record + sweather_per_region::MODIFIERS,
            7,
            &[0x29],
            modifiers,
            &mut row.modifier_count,
        )?;
    }
    let owner_raw = field::<u32>(memory, army, cunit::LOGICAL_OWNER)?;
    let countries = ccountry::Reader::new(memory, base, game)?;
    let owner = countries.resolve(owner_raw)?;
    if row.ids[5] & (1 << 4) != 0 {
        let definitions = pointer(memory, base, 0x0331_7cc8)?;
        if definitions == 0 {
            return Err("reinforcement transport static modifier set null");
        }
        let definition = pointer(memory, definitions, 0xf8)?;
        if definition != 0 {
            auth(memory, base, definition, cstatic_modifier::VTABLE_RVA)?;
            cstrategic_navy_manager::authenticate_owner(memory, base, game, owner)?;
            let owner_intel = pointer(memory, owner.pointer, ccountry::INTEL_TABLE)?;
            auth(memory, base, owner_intel, ccountry_intel::VTABLE_RVA)?;
            let diplomacy = pointer(memory, owner.pointer, ccountry::DIPLOMACY_STATUS)?;
            auth(memory, base, diplomacy, cdiplomacy_status::VTABLE_RVA)?;
            if pointer(memory, diplomacy, cdiplomacy_status::COUNTRY)? != owner.pointer {
                return Err("reinforcement transport diplomacy backlink mismatch");
            }
            let enemies = array(memory, diplomacy, cdiplomacy_status::ENEMY_COUNTRIES, 4)?;
            if enemies.count as usize > TRANSPORT_TERM_LIMIT {
                return Err("reinforcement transport enemy bound exceeded");
            }
            for index in 0..enemies.count {
                let raw = field::<u32>(memory, element(enemies, index, 4)?, 0)?;
                let enemy = countries.resolve(raw)?;
                if field::<i32>(memory, enemy.pointer, ccountry::FOG_NETWORK_TABLE_COUNT)? <= 0 {
                    continue;
                }
                let enemy_diplomacy = pointer(memory, enemy.pointer, ccountry::DIPLOMACY_STATUS)?;
                auth(memory, base, enemy_diplomacy, cdiplomacy_status::VTABLE_RVA)?;
                if pointer(memory, enemy_diplomacy, cdiplomacy_status::COUNTRY)? != enemy.pointer {
                    return Err("reinforcement transport enemy backlink mismatch");
                }
                if field::<u8>(memory, enemy_diplomacy, cdiplomacy_status::CAPITULATED)? != 0 {
                    continue;
                }
                let enemy_intel = pointer(memory, enemy.pointer, ccountry::INTEL_TABLE)?;
                auth(memory, base, enemy_intel, ccountry_intel::VTABLE_RVA)?;
                let output = terms
                    .get_mut(row.transport_term_count)
                    .ok_or("reinforcement transport term arena full")?;
                *output = TransportTerm {
                    country: raw,
                    regional_value: cstrategic_navy_manager::regional_operand(
                        memory, base, game, enemy, region_id,
                    )?,
                    own_intel: transport_intel_lane(
                        memory,
                        base,
                        &countries,
                        owner_intel,
                        enemy_intel,
                        enemy,
                    )?,
                    enemy_intel: transport_intel_lane(
                        memory,
                        base,
                        &countries,
                        enemy_intel,
                        owner_intel,
                        owner,
                    )?,
                };
                row.transport_term_count += 1;
            }
            for (index, rva) in [
                (25, 0x0331_e100),
                (26, 0x0331_d5b0),
                (27, 0x0331_d620),
                (28, 0x0331_e550),
            ] {
                row.scalar_values[index] = field(memory, base, rva)?;
            }
            append_modifiers(
                memory,
                base,
                owner.pointer + ccountry::MODIFIER_ACCUMULATOR,
                9,
                &[0x1a7],
                modifiers,
                &mut row.modifier_count,
            )?;
            append_modifiers(
                memory,
                base,
                definition + cstatic_modifier::ACCUMULATOR,
                8,
                &[0x29],
                modifiers,
                &mut row.modifier_count,
            )?;
            let metadata_table = pointer(memory, base, metadata::TABLE_RVA)?;
            row.scalar_values[29] = i64::from(field::<u32>(
                memory,
                metadata_table,
                0x29 * metadata::STRIDE + metadata::CATEGORY_MASK,
            )?);
        }
    }
    Ok(row)
}

/// Authenticated pointer-free, bounded raw speed graph. Source selectors follow
/// native predicates; numeric operands retain their native integer bits. The
/// naval-transfer branch owns the separate regional weather/enemy/intel graph.
#[allow(clippy::too_many_arguments)]
pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    army: usize,
    expected_handle: u64,
    modifiers: &mut [ModifierTerm],
    supply: &mut [SupplyTerm],
    acclimatization_terms: &mut [AcclimatizationTerm],
    transport_terms: &mut [TransportTerm],
) -> Result<ContextRow, &'static str> {
    auth(memory, base, army, carmy::VTABLE_RVA)?;
    if expected_handle == 0 || field::<u64>(memory, army, cunit::HANDLE)? != expected_handle {
        return Err("reinforcement army handle mismatch");
    }
    let transfer = pointer(memory, army, carmy::NAVAL_TRANSFER_MEMBERSHIP)?;
    let stats = pointer(memory, army, cunit::STAT_BLOCK)?;
    auth(memory, base, stats, crate::csub_unit_definition::VTABLE_RVA)?;
    let location = pointer(memory, army, cunit::LOCATION)?;
    carmy::movement_definition(memory, base, location)?;
    if transfer != 0 {
        return transport_context(
            memory,
            base,
            game,
            army,
            location,
            modifiers,
            transport_terms,
        );
    }
    let template = pointer(memory, army, carmy::STANDARD_TEMPLATE)?;
    auth(
        memory,
        base,
        template,
        crate::creferenced_division_template::VTABLE_RVA,
    )?;
    let owners = field::<[u8; 8]>(memory, army, cunit::ROSTER_OWNER)?;
    let roster = u32::from_le_bytes(owners[..4].try_into().unwrap());
    let controller = u32::from_le_bytes(owners[4..].try_into().unwrap());
    let owner_raw = if controller as i32 > 0 {
        controller
    } else {
        roster
    };
    let countries = ccountry::Reader::new(memory, base, game)?;
    let owner = countries.resolve(owner_raw)?;
    let leader = crate::carmy_leader::movement_accumulator(memory, base, army)?;
    let country_accumulator = owner
        .pointer
        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
        .ok_or("reinforcement country overflow")?;
    let actual_state = pointer(memory, location, cprovince::STATE)?;
    let mut row = ContextRow::default();
    row.ids[0] = field(memory, location, cprovince::ID)?;
    let route = array(memory, army, cunit::ORDINARY_ROUTE_DATA, 4)?;
    let next = if route.count == 0 {
        0
    } else {
        carmy::movement_route_province(memory, base, game, route, 0)?
    };
    if next != 0 {
        row.ids[1] = field(memory, next, cprovince::ID)?;
    }
    row.flags[..4].copy_from_slice(&formation_flags(memory, army)?);
    row.flags[5] = u8::from(carmy::movement_attacking(memory, base, game, army)?);
    use crate::modifier_definition as metadata;
    let callback = pointer(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA)?;
    row.ids[6] = if callback == 0 {
        0
    } else if base.checked_add(metadata::ELIGIBILITY_FLAGS_GETTER_RVA) == Some(callback) {
        1
    } else {
        return Err("reinforcement modifier callback unsupported");
    };
    row.ids[5] = field(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA)?;
    let country_keys = [0x2b, 0x1e9, 0x5e, 0x17f, 0x286];
    append_modifiers(
        memory,
        base,
        country_accumulator,
        0,
        &country_keys,
        modifiers,
        &mut row.modifier_count,
    )?;
    if let Some(leader) = leader {
        append_modifiers(
            memory,
            base,
            leader,
            1,
            &[0x2b, 0x5e, 0x17f],
            modifiers,
            &mut row.modifier_count,
        )?;
    }
    if actual_state != 0 {
        let accumulator = state_accumulator(memory, base, game, actual_state, roster)?;
        row.ids[2] = field(memory, actual_state, crate::cstate::ID)?;
        let controller = field(memory, actual_state, crate::cstate::CONTROLLER)?;
        row.flags[6] = if ccountry::movement_friendly(memory, base, game, owner_raw, controller)? {
            1
        } else if ccountry::movement_enemy(memory, base, game, owner_raw, controller)? {
            2
        } else {
            0
        };
        append_modifiers(
            memory,
            base,
            accumulator,
            2,
            &[0x2b, 0x1e9, 0x1e7, 0x1e8],
            modifiers,
            &mut row.modifier_count,
        )?;
    }
    acclimatization(memory, army, &mut row, acclimatization_terms)?;
    let (province_weather, region_weather) =
        crate::cweather_manager::movement_modifiers(memory, base, game, location)?;
    for (accumulator, scope) in [(province_weather, 3), (region_weather, 4)] {
        append_modifiers(
            memory,
            base,
            accumulator,
            scope,
            &[0x2b, 0x1e9],
            modifiers,
            &mut row.modifier_count,
        )?;
    }
    let selected = carmy::movement_priority_unit(memory, base, template)?;
    row.ids[4] = field(memory, selected, 8)?;
    let token = field::<u32>(memory, selected, 0x5d8)?;
    if token != 0 {
        append_modifiers(
            memory,
            base,
            country_accumulator,
            5,
            &[token],
            modifiers,
            &mut row.modifier_count,
        )?;
    }
    let categories = array(memory, selected, 0x508, 8)?;
    if categories.count as usize > MODIFIER_LIMIT.saturating_sub(row.modifier_count) {
        return Err("reinforcement category bound exceeded");
    }
    for index in 0..categories.count {
        let category = pointer(memory, element(categories, index, 8)?, 0)?;
        let token = field::<u32>(memory, category, 0x60)?;
        if token != 0 {
            append_modifiers(
                memory,
                base,
                country_accumulator,
                6,
                &[token],
                modifiers,
                &mut row.modifier_count,
            )?;
        }
    }
    air(memory, base, game, army, stats, location, &mut row)?;
    if row.flags[2] == 0 {
        row.flags[11] = 1;
        for (index, value) in [2, 3, 4, 10, 5, 6, 7, 8]
            .into_iter()
            .zip(supply_scalars(memory, base, army)?)
        {
            row.scalar_values[index] = value;
        }
        supply_terms(memory, base, template, &mut row, supply)?;
        if row.flags[3] == 0
            && !ccountry::has_core_supply_access(
                memory,
                base,
                game,
                owner_raw,
                pointer(memory, location, cprovince::CONTROLLER_AREA)?,
            )?
        {
            row.flags[8] = 1;
            row.scalar_values[9] = field(
                memory,
                base,
                if row.scalar_values[10] > 0 {
                    0x0331_9618
                } else {
                    0x0331_9568
                },
            )?;
        }
    }
    if row.flags[0] != 0 {
        row.scalar_values[11] = field(memory, base, 0x0331_9758)?;
    } else if row.flags[1] != 0 {
        row.scalar_values[12] = field(memory, base, 0x0331_9808)?;
    }
    let infrastructure_state = if next == 0 {
        actual_state
    } else {
        pointer(memory, next, cprovince::STATE)?
    };
    infrastructure(memory, base, infrastructure_state, &mut row)?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::{Cell, RefCell};

    struct PacketMemory {
        fields: Vec<(usize, Vec<u8>)>,
        reads: RefCell<Vec<(usize, usize)>>,
        scatters: Cell<usize>,
        fault: Option<usize>,
    }

    impl PacketMemory {
        fn new(fields: &[(usize, Vec<u8>)], fault: Option<usize>) -> Self {
            Self {
                fields: fields.to_vec(),
                reads: RefCell::default(),
                scatters: Cell::new(0),
                fault,
            }
        }
    }

    impl Memory for PacketMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            if self.fault == Some(ordinal) {
                let partial = output.len().div_ceil(2);
                output[..partial].fill(0xed);
                return false;
            }
            let Some((_, value)) = self
                .fields
                .iter()
                .find(|(source, value)| *source == address && value.len() == output.len())
            else {
                return false;
            };
            output.copy_from_slice(value);
            true
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.scatters.set(self.scatters.get() + 1);
            for (ordinal, (address, destination)) in
                addresses.into_iter().zip(destinations).enumerate()
            {
                if !self.copy(address, destination) {
                    return ordinal;
                }
            }
            N
        }
    }

    #[test]
    fn packets_preserve_native_values_exact_spans_and_every_fault_prefix() {
        let fields = [
            (0x124c, vec![0xff]),
            (0x124d, vec![0x80]),
            (0x12ae, vec![0]),
            (0x102e, vec![0x7f]),
            (0x1600, i64::MIN.to_le_bytes().to_vec()),
            (0x1610, i64::MAX.to_le_bytes().to_vec()),
            (0x1040, (-1_i64).to_le_bytes().to_vec()),
            (0x1058, 0_i64.to_le_bytes().to_vec()),
            (0x1498, i32::MIN.to_le_bytes().to_vec()),
            (0x0331_91c0, i32::MAX.to_le_bytes().to_vec()),
            (0x0331_9394, (-1_i32).to_le_bytes().to_vec()),
            (0x0331_94e0, (-987_654_i64).to_le_bytes().to_vec()),
        ];
        let values = [
            0xff,
            0x80,
            0,
            0x7f,
            i64::MIN,
            i64::MAX,
            -1,
            0,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
            -1,
            -987_654,
        ];
        for supply in [false, true] {
            let range = if supply { 4..12 } else { 0..4 };
            let fields = &fields[range.clone()];
            let trace: Vec<_> = fields
                .iter()
                .map(|(at, bytes)| (*at, bytes.len()))
                .collect();
            for fault in std::iter::once(None).chain((0..fields.len()).map(Some)) {
                let memory = PacketMemory::new(fields, fault);
                let result = if supply {
                    supply_scalars(&memory, 0, 0x1000).map(Vec::from)
                } else {
                    formation_flags(&memory, 0x1000)
                        .map(|flags| flags.into_iter().map(i64::from).collect())
                };
                let expected = fault.map_or_else(
                    || Ok(values[range.clone()].to_vec()),
                    |_| Err("reinforcement input unreadable"),
                );
                assert_eq!(result, expected);
                assert_eq!(memory.scatters.get(), 1);
                assert_eq!(
                    *memory.reads.borrow(),
                    trace[..fault.map_or(fields.len(), |n| n + 1)]
                );
            }
        }
    }

    #[test]
    fn packet_failure_keeps_partial_field_and_later_destinations_untouched() {
        let fields = [
            (0, vec![0xcc; 4]),
            (0x1000, vec![1, 2, 3, 4]),
            (0x1010, vec![5, 6, 7, 8]),
            (0x1020, vec![9, 10, 11, 12]),
        ];
        let memory = PacketMemory::new(&fields, Some(1));
        let mut values = [[0xa5; 4]; 3];
        assert_eq!(
            copy_input_spans(
                &memory,
                [(0x1000, 0), (0x1000, 0x10), (0x1000, 0x20)],
                &mut values.each_mut().map(|value| value.as_mut_slice()),
            ),
            Err("reinforcement input unreadable")
        );
        assert_eq!(values, [[1, 2, 3, 4], [0xed, 0xed, 0xa5, 0xa5], [0xa5; 4]]);
        assert_eq!(*memory.reads.borrow(), [(0x1000, 4), (0x1010, 4)]);
        let mut zero = [0; 4];
        assert!(PacketMemory::new(&fields, None).copy(0, &mut zero));
        assert_eq!(zero, [0xcc; 4]);
        let memory = PacketMemory::new(&fields, None);
        let mut values = [[0xa5; 4]; 3];
        assert_eq!(
            copy_input_spans(
                &memory,
                [(0x1000, 0), (usize::MAX, 1), (0x1000, 0x20)],
                &mut values.each_mut().map(|value| value.as_mut_slice()),
            ),
            Err("reinforcement input unreadable")
        );
        assert_eq!(values, [[1, 2, 3, 4], [0xa5; 4], [0xa5; 4]]);
        assert_eq!(*memory.reads.borrow(), [(0x1000, 4)]);
        assert_eq!(memory.scatters.get(), 0);
    }

    struct Counted<T> {
        inner: T,
        copies: Cell<usize>,
        bytes: Cell<usize>,
    }
    impl<T: Memory> Memory for Counted<T> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.bytes.set(self.bytes.get() + output.len());
            self.inner.copy(address, output)
        }
    }

    #[test]
    fn raw_modifier_preserves_negative_values_absence_and_order() {
        let mut bytes = vec![0_u8; 0x300];
        bytes[0x10..0x18].copy_from_slice(&0x100_u64.to_le_bytes());
        bytes[0x18..0x1c].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x1c..0x20].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x100..0x104].copy_from_slice(&5_u32.to_le_bytes());
        bytes[0x108..0x110].copy_from_slice(&(-170_005_i64).to_le_bytes());
        struct WithMetadata<'a>(&'a [u8]);
        impl Memory for WithMetadata<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == crate::modifier_definition::TABLE_RVA {
                    output.copy_from_slice(&0x10000_u64.to_le_bytes());
                    return true;
                }
                if address
                    == 0x10000
                        + 5 * crate::modifier_definition::STRIDE
                        + crate::modifier_definition::FLAGS
                {
                    output.fill(0);
                    output[..4].copy_from_slice(&7_u32.to_le_bytes());
                    return true;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
        let mut output = [ModifierTerm::default(); 3];
        let mut len = 0;
        let counted = Counted {
            inner: WithMetadata(&bytes),
            copies: Cell::new(0),
            bytes: Cell::new(0),
        };
        append_modifiers(&counted, 0, 0x10, 4, &[6, 5, 7], &mut output, &mut len).unwrap();
        assert_eq!(len, 3);
        assert_eq!(output[0].present, 0);
        assert_eq!(output[1].raw, -170_005);
        assert_eq!(output[1].metadata_flags, 7);
        assert_eq!(output[1].scope, 4);
        assert_eq!(output[2].key, 7);
        assert_eq!((counted.copies.get(), counted.bytes.get()), (4, 52));
        assert_eq!(
            append_modifiers(
                &WithMetadata(&bytes),
                0,
                0x10,
                0,
                &[1],
                &mut output,
                &mut len
            ),
            Err("reinforcement modifier arena full")
        );
    }

    #[test]
    fn raw_shared_modifier_snapshot_avoids_native_recensus() {
        struct Shared {
            copies: Cell<usize>,
        }
        impl Memory for Shared {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.copies.set(self.copies.get() + 1);
                if address == crate::modifier_definition::TABLE_RVA {
                    output.copy_from_slice(&0x10000_u64.to_le_bytes());
                    return true;
                }
                if address
                    == 0x10000
                        + 5 * crate::modifier_definition::STRIDE
                        + crate::modifier_definition::FLAGS
                {
                    output.fill(0);
                    return true;
                }
                false
            }
            fn modifier_accumulator_value(
                &self,
                address: usize,
                key: u32,
            ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
                assert_eq!(address, 0x99999);
                Some(Ok(crate::modifier_accumulator::Value {
                    raw: if key == 5 { -654321 } else { 0 },
                    present: key == 5,
                }))
            }
        }
        let memory = Shared {
            copies: Cell::new(0),
        };
        let mut output = [ModifierTerm::default(); 2];
        let mut len = 0;
        append_modifiers(&memory, 0, 0x99999, 1, &[4, 5], &mut output, &mut len).unwrap();
        assert_eq!(output[1].raw, -654321);
        assert_eq!(output[0].present, 0);
        assert_eq!(memory.copies.get(), 2);
    }

    #[test]
    fn transport_without_static_feature_preserves_branch_without_normal_dependencies() {
        let mut bytes = vec![0_u8; 0x9000];
        let mut put =
            |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        put(0x1000, carmy::VTABLE_RVA as u64);
        put(0x1000 + cunit::HANDLE, 0x1234_0000_0033);
        put(0x1000 + cunit::STAT_BLOCK, 0x2000);
        put(0x2000, crate::csub_unit_definition::VTABLE_RVA as u64);
        put(0x1000 + cunit::LOCATION, 0x3000);
        put(0x1000 + carmy::NAVAL_TRANSFER_MEMBERSHIP, 42);
        put(0x1000 + cunit::LOGICAL_OWNER, 1);
        put(0x3000, cprovince::VTABLE_RVA as u64);
        put(0x3000 + cprovince::DEFINITION, 0x3800);
        put(0x3000 + cprovince::ID, 14);
        put(0x3800 + crate::province_definition::ID, 14);
        put(0x3000 + cprovince::STRATEGIC_REGION, 0x4000);
        put(0x4000, crate::cstrategic_region::VTABLE_RVA as u64);
        put(0x4000 + crate::cstrategic_region::ID, 22);
        put(0x7000 + cgame_state::WEATHER_MANAGER, 0x5000);
        put(0x5000, crate::cweather_manager::VTABLE_RVA as u64);
        put(0x7000 + cgame_state::COUNTRIES, 0x8000);
        put(0x7000 + cgame_state::COUNTRIES + 8, 0x2_0000_0002);
        put(0x8008, 0x6000);
        put(0x7000 + cgame_state::COUNTRY_LINKS, 0x8100);
        put(0x7000 + cgame_state::COUNTRY_LINKS + 8, 0x2_0000_0002);
        put(0x8100, 0x1_0000_0000);
        put(0x6000, ccountry::VTABLE_RVA as u64);
        put(0x6000 + ccountry::ID, 1);
        struct Source<'a>(&'a [u8]);
        impl Memory for Source<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert_ne!(address, 0x1000 + carmy::STANDARD_TEMPLATE);
                assert!(
                    ![
                        0x124c, 0x124d, 0x12ae, 0x102e, 0x1600, 0x1610, 0x1040, 0x1058, 0x1498
                    ]
                    .contains(&address),
                    "naval transfer reached a normal land packet"
                );
                if [
                    crate::modifier_definition::ELIGIBILITY_CALLBACK_RVA,
                    crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
                ]
                .contains(&address)
                {
                    output.fill(0);
                    return true;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
        let row = capture_context(
            &Source(&bytes),
            0,
            0x7000,
            0x1000,
            0x1234_0000_0033,
            &mut [],
            &mut [],
            &mut [],
            &mut [],
        )
        .unwrap();
        assert_eq!(row.flags[11], 2);
        assert_eq!(row.ids[0], 14);
        assert_eq!(row.ids[7], 22);
        assert_eq!((row.modifier_count, row.transport_term_count), (0, 0));
    }

    #[test]
    fn acclimatization_retains_native_ties_and_active_reference_join() {
        let mut bytes = vec![0_u8; 0x1000];
        let object = 0x100 + carmy::ACCLIMATIZATION;
        bytes[object + 8..object + 16].copy_from_slice(&0x800_u64.to_le_bytes());
        bytes[object + 16..object + 20].copy_from_slice(&2_u32.to_le_bytes());
        bytes[object + 20..object + 24].copy_from_slice(&2_u32.to_le_bytes());
        bytes[object + 0x20..object + 0x28].copy_from_slice(&0x900_u64.to_le_bytes());
        bytes[object + 0x38..object + 0x40].copy_from_slice(&123_i64.to_le_bytes());
        for (index, pointer) in [0x900_u64, 0x920].into_iter().enumerate() {
            bytes[0x800 + index * 16..0x808 + index * 16].copy_from_slice(&pointer.to_le_bytes());
            bytes[0x808 + index * 16..0x810 + index * 16].copy_from_slice(&99_i64.to_le_bytes());
        }
        bytes[0x930..0x938].copy_from_slice(&(-500_i64).to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut row = ContextRow::default();
        let mut terms = [AcclimatizationTerm::default(); 2];
        acclimatization(&memory, 0x100, &mut row, &mut terms).unwrap();
        assert_eq!(
            terms,
            [
                AcclimatizationTerm {
                    weight: 99,
                    active: 1
                },
                AcclimatizationTerm {
                    weight: 99,
                    active: 0
                }
            ]
        );
        assert_eq!(row.scalar_values[0], 123);
        assert_eq!(row.scalar_values[1], -500);
        assert_eq!(
            acclimatization(&memory, 0x100, &mut row, &mut terms[..1]),
            Err("reinforcement acclimatization arena full")
        );
    }
}
