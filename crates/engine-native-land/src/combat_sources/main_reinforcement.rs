use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{carmy, ccountry, cgame_state, cprovince, cunit};

pub const MODIFIER_LIMIT: usize = 96;
pub const SUPPLY_TERM_LIMIT: usize = 64;
pub const ACCLIMATIZATION_LIMIT: usize = 64;

pub use crate::land_reinforcement_inputs::{
    AcclimatizationTerm, ContextRow, ModifierTerm, SupplyTerm, TransportTerm,
};

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    p: usize,
    o: usize,
) -> Result<T, &'static str> {
    p.checked_add(o)
        .and_then(|at| read(memory, at))
        .ok_or("reinforcement input unreadable")
}

fn copy_input_spans<const N: usize>(
    memory: &impl Memory,
    sources: [(usize, usize); N],
    destinations: &mut [&mut [u8]; N],
) -> Result<(), &'static str> {
    let addresses = sources.map(|(pointer, offset)| pointer.checked_add(offset));
    if addresses.iter().any(Option::is_none) {
        for (address, destination) in addresses.into_iter().zip(destinations) {
            let address = address.ok_or("reinforcement input unreadable")?;
            if !engine_native_memory::read_slice(memory, address, destination) {
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
            (base, 0x033e_3678),
            (base, 0x033e_3844),
            (base, 0x033e_3990),
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
    Array::from_header(header, stride).ok_or("reinforcement array invalid")
}

fn element(array: Array, index: u32, stride: usize) -> Result<usize, &'static str> {
    array
        .element(index, stride)
        .ok_or("reinforcement array index invalid")
}

fn auth(memory: &impl Memory, base: usize, p: usize, key: Rva) -> Result<(), &'static str> {
    if p == 0 || engine_sites::at(base, key).map(|n| n as u64) != Some(field(memory, p, 0)?) {
        return Err("reinforcement dependency identity invalid");
    }
    Ok(())
}

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
            if !engine_native_memory::read_slice(
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
    use engine_native_parts::modifier_definition as metadata;
    let table = pointer(memory, base, metadata::TABLE_RVA.0)?;
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

fn state_accumulator(
    memory: &impl Memory,
    base: usize,
    game: usize,
    state: usize,
    owner: u32,
) -> Result<usize, &'static str> {
    use engine_native::cstate;
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
        auth(memory, base, unit, engine_native_parts::csub_unit_definition::VTABLE_RVA)?;
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
    use engine_native_parts::{cbuilding, cbuilding_database, cbuilding_template};
    use engine_native::{cprovince::buildings as cbuilding_status, cstate};
    if state == 0 {
        return Ok(());
    }
    auth(memory, base, state, cstate::VTABLE_RVA)?;
    row.flags[9] = 1;
    row.ids[3] = field(memory, state, cstate::ID)?;
    let database = pointer(memory, base, cbuilding_database::SINGLETON_RVA.0)?;
    let definition = pointer(memory, database, 0x3b0)?;
    if database == 0 || definition == 0 {
        return Err("reinforcement infrastructure definition null");
    }
    row.scalar_values[20] = i64::from(field::<i32>(memory, definition, 0x2bc)?);
    if row.scalar_values[20] < 1 {
        row.scalar_values[21] = i64::from(field::<i32>(memory, definition, 0x2b8)?);
        if row.scalar_values[21] < 1 {
            row.scalar_values[22] = i64::from(field::<i32>(memory, base, 0x033e_5274)?);
        }
    }
    row.scalar_values[23] = field(memory, base, 0x033e_5080)?;
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
        row.scalar_values[17] = field(memory, base, 0x033e_1850)?;
        if row.scalar_values[16] != 0 {
            row.scalar_values[18] = field(memory, base, 0x033e_18b8)?;
            row.scalar_values[19] = field(memory, base, 0x033e_1830)?;
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
    target_country: engine_native_memory::country_identity::Country,
) -> Result<i64, &'static str> {
    use engine_native_intelligence::{ccountry_intel, country_intel_row};
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

#[expect(
    clippy::too_many_arguments,
    reason = "the army and location locate one context, and the modifier and transport arenas are caller-owned"
)]
fn transport_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    army: usize,
    location: usize,
    modifiers: &mut [ModifierTerm],
    terms: &mut Vec<TransportTerm>,
    start: usize,
) -> Result<ContextRow, &'static str> {
    use engine_native_intelligence::ccountry_intel;
    use engine_native_politics::cdiplomacy_status;
    use engine_native::cweather_manager::{self, region_weather as sweather_per_region};
    use engine_native_naval::cstrategic_navy_manager;
    use engine_native_parts::{
        cstatic_modifier,
        cstrategic_region,
        modifier_definition as metadata,
    };
    let mut row = ContextRow::default();
    row.flags[11] = 2;
    row.ids[0] = field(memory, location, cprovince::ID)?;
    row.ids[5] = field(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA.0)?;
    let callback = pointer(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA.0)?;
    row.ids[6] = if callback == 0 {
        0
    } else if engine_sites::at(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA) == Some(callback) {
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
        let definitions = pointer(memory, base, 0x033d_e590)?;
        if definitions == 0 {
            return Err("reinforcement transport static modifier set null");
        }
        let definition = pointer(memory, definitions, 0xf8)?;
        if definition != 0 {
            auth(memory, base, definition, cstatic_modifier::VTABLE_RVA)?;
            cstrategic_navy_manager::authenticate(memory, base, game, owner)?;
            let owner_intel = pointer(memory, owner.pointer, ccountry::INTEL_TABLE)?;
            auth(memory, base, owner_intel, ccountry_intel::VTABLE_RVA)?;
            let diplomacy = pointer(memory, owner.pointer, ccountry::DIPLOMACY_STATUS)?;
            auth(memory, base, diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
            if pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != owner.pointer {
                return Err("reinforcement transport diplomacy backlink mismatch");
            }
            let enemies = array(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES, 4)?;
            for index in 0..enemies.count {
                let raw = field::<u32>(memory, element(enemies, index, 4)?, 0)?;
                let enemy = countries.resolve(raw)?;
                if field::<i32>(memory, enemy.pointer, ccountry::FOG_NETWORK_TABLE_COUNT)? <= 0 {
                    continue;
                }
                let enemy_diplomacy = pointer(memory, enemy.pointer, ccountry::DIPLOMACY_STATUS)?;
                auth(memory, base, enemy_diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
                if pointer(memory, enemy_diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != enemy.pointer {
                    return Err("reinforcement transport enemy backlink mismatch");
                }
                if field::<u8>(memory, enemy_diplomacy, cdiplomacy_status::CAPITULATED)? != 0 {
                    continue;
                }
                let enemy_intel = pointer(memory, enemy.pointer, ccountry::INTEL_TABLE)?;
                auth(memory, base, enemy_intel, ccountry_intel::VTABLE_RVA)?;
                let term = TransportTerm {
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
                let slot = start
                    .checked_add(row.transport_term_count)
                    .ok_or("reinforcement transport term index overflow")?;
                if terms.len() <= slot {
                    terms.resize(slot + 1, TransportTerm::default());
                }
                terms[slot] = term;
                row.transport_term_count += 1;
            }
            for (index, rva) in [
                (25, 0x033e_1d68),
                (26, 0x033e_1000),
                (27, 0x033e_1088),
                (28, 0x033e_2200),
            ] {
                row.scalar_values[index] = field(memory, base, rva)?;
            }
            append_modifiers(
                memory,
                base,
                owner.pointer + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
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
            let metadata_table = pointer(memory, base, metadata::TABLE_RVA.0)?;
            row.scalar_values[29] = i64::from(field::<u32>(
                memory,
                metadata_table,
                0x29 * metadata::STRIDE + metadata::CATEGORY_MASK,
            )?);
        }
    }
    Ok(row)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the army and expected handle identify one context, and each term arena is caller-owned"
)]
pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    army: usize,
    expected_handle: u64,
    modifiers: &mut [ModifierTerm],
    supply: &mut [SupplyTerm],
    acclimatization_terms: &mut [AcclimatizationTerm],
    transport_terms: &mut Vec<TransportTerm>,
    transport_start: usize,
) -> Result<ContextRow, &'static str> {
    auth(memory, base, army, carmy::VTABLE_RVA)?;
    if expected_handle == 0 || field::<u64>(memory, army, cunit::HANDLE)? != expected_handle {
        return Err("reinforcement army handle mismatch");
    }
    let transfer = pointer(memory, army, carmy::NAVAL_TRANSFER_MEMBERSHIP)?;
    let stats = pointer(memory, army, cunit::STAT_BLOCK)?;
    auth(memory, base, stats, engine_native_parts::csub_unit_definition::VTABLE_RVA)?;
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
            transport_start,
        );
    }
    let template = pointer(memory, army, carmy::STANDARD_TEMPLATE)?;
    auth(
        memory,
        base,
        template,
        engine_native::creferenced_division_template::VTABLE_RVA,
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
    let leader = engine_native::carmy_leader::movement_accumulator(memory, base, army)?;
    let country_accumulator = owner
        .pointer
        .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
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
    use engine_native_parts::modifier_definition as metadata;
    let callback = pointer(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA.0)?;
    row.ids[6] = if callback == 0 {
        0
    } else if engine_sites::at(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA) == Some(callback) {
        1
    } else {
        return Err("reinforcement modifier callback unsupported");
    };
    row.ids[5] = field(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA.0)?;
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
        row.ids[2] = field(memory, actual_state, engine_native::cstate::ID)?;
        let controller = field(memory, actual_state, engine_native::cstate::CONTROLLER)?;
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
        engine_native::cweather_manager::movement_modifiers(memory, base, game, location)?;
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
                    0x033e_3ad0
                } else {
                    0x033e_3a20
                },
            )?;
        }
    }
    if row.flags[0] != 0 {
        row.scalar_values[11] = field(memory, base, 0x033e_3c20)?;
    } else if row.flags[1] != 0 {
        row.scalar_values[12] = field(memory, base, 0x033e_3cb8)?;
    }
    let infrastructure_state = if next == 0 {
        actual_state
    } else {
        pointer(memory, next, cprovince::STATE)?
    };
    infrastructure(memory, base, infrastructure_state, &mut row)?;
    Ok(row)
}
