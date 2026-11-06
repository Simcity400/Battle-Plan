use super::*;
use engine_native::{
    carmy, carmy_combat_inputs as inputs, cbuilding_database, ccombat_tactic, ccountry,
    cgame_state, cland_combatant, cnaval_combat, cprovince,
    creferenced_division_template as template, cstate, cstrategic_region, csub_unit_definition,
    cterrain_type, cunit, cweather_manager, modifier_definition, province_definition,
    sweather_per_region,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

const PAGE: usize = 4096;
const GAME: usize = 0x4000_0000;

#[derive(Default)]
struct World {
    pages: HashMap<usize, Box<[u8; PAGE]>>,
    next: usize,
    /// Guarded calls, as the recorder's live reader counts them.
    calls: AtomicUsize,
    /// Byte ranges whose reads fail.
    poisoned: Vec<(usize, usize)>,
    /// Every guarded span, when logging.
    log: std::sync::Mutex<Option<Vec<(usize, usize)>>>,
    /// Every guarded call's first address, when logging calls.
    call_log: std::sync::Mutex<Option<Vec<usize>>>,
}

impl World {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            let address = at + index;
            self.pages
                .entry(address / PAGE)
                .or_insert_with(|| Box::new([0; PAGE]))[address % PAGE] = *byte;
        }
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn alloc(&mut self, size: usize) -> usize {
        if self.next == 0 {
            self.next = 0x1_0000_0000;
        }
        let at = self.next;
        self.next += size.next_multiple_of(64);
        at
    }
    /// A CPdxArray header: data, capacity, count.
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn object(&mut self, size: usize, vtable: usize) -> usize {
        let at = self.alloc(size);
        self.u64(at, vtable as u64);
        at
    }
    /// An inline MSVC string.
    fn text(&mut self, at: usize, text: &[u8]) {
        assert!(text.len() <= 15);
        self.put(at, text);
        self.u64(at + 16, text.len() as u64);
        self.u64(at + 24, 15);
    }
    fn accumulator(&mut self, at: usize, entries: &[(u32, i64)]) {
        let data = self.alloc(entries.len().max(1) * 16);
        for (index, (key, value)) in entries.iter().enumerate() {
            self.u32(data + index * 16, *key);
            self.u64(data + index * 16 + 8, *value as u64);
        }
        self.array(at, data, entries.len() as u32);
        self.text(at + 0x48, b"");
    }
    #[cfg(test)]
    fn poison(&mut self, at: usize, len: usize) {
        self.poisoned.push((at, len));
    }
    fn get_u64(&self, at: usize) -> u64 {
        let mut bytes = [0; 8];
        assert!(self.fill(at, &mut bytes));
        u64::from_le_bytes(bytes)
    }
    fn fill(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some(log) = self.log.lock().unwrap().as_mut() {
            log.push((address, output.len()));
        }
        // A process's first pages fault: a null object's field fails.
        if address < 0x1_0000
            || self
                .poisoned
                .iter()
                .any(|&(at, len)| address < at + len && at < address + output.len())
        {
            return false;
        }
        let mut done = 0;
        while done < output.len() {
            let at = address + done;
            let offset = at % PAGE;
            let chunk = (PAGE - offset).min(output.len() - done);
            match self.pages.get(&(at / PAGE)) {
                Some(page) => {
                    output[done..done + chunk].copy_from_slice(&page[offset..offset + chunk])
                }
                None => output[done..done + chunk].fill(0),
            }
            done += chunk;
        }
        true
    }
}

impl World {
    fn call(&self, address: usize) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(log) = self.call_log.lock().unwrap().as_mut() {
            log.push(address);
        }
    }
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.call(address);
        self.fill(address, output)
    }
    fn copy_scattered<const M: usize>(
        &self,
        addresses: [usize; M],
        destinations: &mut [&mut [u8]; M],
    ) -> usize {
        self.call(addresses.first().copied().unwrap_or(0));
        for (ordinal, (address, destination)) in addresses.into_iter().zip(destinations).enumerate()
        {
            if !destination.is_empty() && !self.fill(address, destination) {
                return ordinal;
            }
        }
        M
    }
    /// One guarded call, as the recorder's live gather makes one.
    fn copy_records(
        &self,
        sources: engine_native::memory::RecordSources<'_>,
        layout: &engine_native::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.call(sources.pointer(0).unwrap_or(0));
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.fill(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
}

/// The Ordinary job's country roster over `world`: an authenticated
/// census, as the callback prepares it.
#[cfg(test)]
fn roster(world: &World) -> engine_native::country_memory::Cache {
    let mut cache = engine_native::country_memory::Cache::try_with_capacity(
        engine_native::country_memory::Cache::cells_for_countries(8192, 1024),
    )
    .unwrap();
    cache.prepare(world, 0, GAME);
    world.calls.store(0, Ordering::Relaxed);
    cache
}

/// The fixture behind the recorder's read accounting.
#[cfg(test)]
struct Shared<'a>(&'a World);

#[cfg(test)]
impl Memory for Shared<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }
    fn copy_scattered<const M: usize>(
        &self,
        addresses: [usize; M],
        destinations: &mut [&mut [u8]; M],
    ) -> usize {
        self.0.copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: engine_native::memory::RecordSources<'_>,
        layout: &engine_native::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.0.copy_records(sources, layout, destination)
    }
}

#[cfg(test)]
type Rostered<'a> = engine_native::country_memory::CountryMemory<
    'a,
    engine_native::memory::CountedMemory<Shared<'a>>,
    false,
>;

/// Guarded calls per table, attributed as the recorder attributes them,
/// over the authenticated roster.
#[cfg(test)]
fn per_table(
    world: &World,
    cache: &engine_native::country_memory::Cache,
    work: impl FnOnce(&Rostered<'_>),
) -> [u64; N] {
    let counted = engine_native::memory::CountedMemory::new(Shared(world));
    let capture = crate::read_accounting::Capture::new();
    work(&cache.roster_view(&counted));
    let counts = capture.finish();
    TABLES.map(|table| {
        let c = counts[table.index()];
        c.copy_calls + c.scattered_calls + c.record_calls + c.plan_calls
    })
}

struct Shape {
    countries: u32,
    combats: usize,
    /// Divisions per combat, both sides.
    divisions: usize,
    owners: usize,
    templates: usize,
    states: usize,
    provinces: usize,
    relations: usize,
    transports: usize,
    tactics: u32,
    /// Army leaders; every fourth division, from the second, has one.
    leaders: usize,
}

impl Shape {
    fn small() -> Self {
        Self {
            countries: 24,
            combats: 7,
            divisions: 5,
            owners: 5,
            templates: 7,
            states: 5,
            provinces: 11,
            relations: 6,
            transports: 3,
            tactics: 2,
            leaders: 3,
        }
    }
    #[cfg(test)]
    fn scale(divisions: usize) -> Self {
        Self {
            countries: 1_500,
            combats: divisions.div_ceil(20),
            divisions: 20,
            owners: (divisions / 25).clamp(6, 150),
            templates: (divisions / 6).max(20),
            states: (divisions / 8).max(10),
            provinces: (divisions / 5).max(20),
            relations: 20,
            transports: divisions / 50,
            tactics: 40,
            leaders: (divisions / 20).clamp(1, 48),
        }
    }
}

/// A built world and what the variants break.
struct Built {
    world: World,
    /// The CReferencedDivisionTemplate objects, registered by handle.
    #[cfg(test)]
    templates: Vec<usize>,
    #[cfg(test)]
    combats: Vec<usize>,
    /// Army pointers per combat, attackers first.
    #[cfg(test)]
    armies: Vec<Vec<usize>>,
    /// Attacker and defender CLandCombatant per combat.
    #[cfg(test)]
    sides: Vec<[usize; 2]>,
}

fn world(shape: &Shape) -> Built {
    let mut w = World::default();
    let slots = shape.countries as usize + 1;
    // Module data: definitions, defines and singletons.
    let table = w.alloc(0x78 * 0x300);
    w.u64(modifier_definition::TABLE_RVA.0, table as u64);
    for key in [0x2b_usize, 0x5e, 0x17f, 0x1e9, 0xfb, 0x10f] {
        w.u32(
            table + key * modifier_definition::STRIDE + modifier_definition::FLAGS,
            3,
        );
    }
    let database = w.alloc(0x400);
    w.u64(cbuilding_database::SINGLETON_RVA.0, database as u64);
    let definition = w.alloc(0x400);
    w.u64(database + 0x3b0, definition as u64);
    // The tactic database: its null entry and `tactics` definitions.
    let tactics = w.alloc(0x100);
    w.u64(ccombat_tactic::DATABASE_RVA.0, tactics as u64);
    let entries = w.alloc((shape.tactics as usize + 1) * 8);
    w.array(tactics + 0x10, entries, shape.tactics + 1);
    for id in 1..=shape.tactics {
        let tactic = w.object(0x200, ccombat_tactic::VTABLE_RVA.0);
        w.u32(tactic + ccombat_tactic::ID, id);
        w.text(
            tactic + ccombat_tactic::TOKEN,
            format!("tactic_{id}").as_bytes(),
        );
        w.u32(tactic + 0xc0, 10 * id);
        w.u64(entries + id as usize * 8, tactic as u64);
    }
    let doctrine = w.alloc(0x40);
    w.u64(
        GAME + engine_land_inputs::country_tactic_unlocks::DOCTRINE_SYSTEM,
        doctrine as u64,
    );
    // Countries.
    let countries = w.alloc(slots * 8);
    let links = w.alloc(slots * 4);
    w.array(GAME + cgame_state::COUNTRIES, countries, slots as u32);
    w.array(GAME + cgame_state::COUNTRY_LINKS, links, slots as u32);
    let relation = w.alloc(0x40);
    w.u32(relation + 8, 0x3801);
    let air_manager = w.alloc(0x100);
    w.u64(
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        air_manager as u64,
    );
    let air_countries = w.alloc(slots * 8);
    w.array(air_manager + 0x30, air_countries, slots as u32);
    for slot in 1..slots {
        w.u32(links + slot * 4, slot as u32);
    }
    let mut owners = Vec::new();
    let mut diplomacies = Vec::new();
    for owner in 0..shape.owners {
        let slot = 1 + owner * (slots - 1) / shape.owners.max(1);
        let country = w.object(0x1000, ccountry::VTABLE_RVA.0);
        w.u32(country + ccountry::ID, slot as u32);
        w.u64(countries + slot * 8, country as u64);
        w.accumulator(
            country + ccountry::MODIFIER_ACCUMULATOR,
            &[
                (0x2b, 5_000),
                (0x5e, -2_000),
                (0xfb, 7),
                (0x17f, 11),
                (0x1e9, 13),
            ],
        );
        let diplomacy = w.object(0x300, engine_native::cdiplomacy_status::VTABLE_RVA.0);
        w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        w.u64(
            diplomacy + engine_native::cdiplomacy_status::COUNTRY,
            country as u64,
        );
        diplomacies.push(diplomacy);
        // Every third owner is at war with the next owner: its enemy list's
        // count supplies the air test's at-war answer.
        let next = 1 + ((owner + 1) % shape.owners) * (slots - 1) / shape.owners;
        let tags = w.alloc(8);
        w.u32(tags, slot as u32);
        w.u32(tags + 4, next as u32);
        w.array(diplomacy + inputs::OCCUPIER_SET, tags, 2);
        let list = w.alloc(4);
        let at_war = owner % 3 == 0;
        if at_war {
            let enemy = (owner + 1) % shape.owners;
            w.u32(list, (1 + enemy * (slots - 1) / shape.owners) as u32);
        }
        w.array(
            diplomacy + engine_native::cdiplomacy_status::ENEMY_COUNTRIES,
            list,
            u32::from(at_war),
        );
        let list = w.alloc(shape.relations.max(1) * 8);
        for index in 0..shape.relations {
            w.u64(list + index * 8, relation as u64);
        }
        w.array(diplomacy + 0x20, list, shape.relations as u32);
        let air = w.alloc(0x100);
        w.u64(air_countries + slot * 8, air as u64);
        let regions = w.alloc(0xa0 * 4);
        w.array(air + 0xe0, regions, 4);
        for index in 0..4 {
            w.u64(regions + index * 0xa0, 100 + index as u64);
            w.u64(regions + index * 0xa0 + 8, index as u64);
            w.u64(regions + index * 0xa0 + 0x18, 900 + index as u64);
        }
        owners.push(slot as u32);
    }
    for (from, &diplomacy) in diplomacies.iter().enumerate() {
        let row = w.alloc(slots * 8);
        w.array(
            diplomacy + engine_native::cdiplomacy_status::RELATIONS,
            row,
            slots as u32,
        );
        for (to, &target) in owners.iter().enumerate() {
            if to == from {
                continue;
            }
            let tuple = w.object(0x300, engine_native::crelation_status::VTABLE_RVA.0);
            w.u64(
                tuple + engine_native::crelation_status::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            w.u32(
                tuple + engine_native::crelation_status::TARGET_COUNTRY,
                target,
            );
            w.u64(tuple + 0x2a0, u64::from((from + to) % 2 == 0));
            w.u64(row + target as usize * 8, tuple as u64);
        }
    }
    // Weather: two strategic regions with records.
    let weather = w.object(0x100, cweather_manager::VTABLE_RVA.0);
    w.u64(GAME + cgame_state::WEATHER_MANAGER, weather as u64);
    let records = w.alloc(sweather_per_region::STRIDE * 2);
    w.array(weather + cweather_manager::REGIONS, records, 2);
    let mut regions = Vec::new();
    for id in 0..2_u32 {
        let record = records + id as usize * sweather_per_region::STRIDE;
        w.u64(record, sweather_per_region::VTABLE_RVA.0 as u64);
        w.u32(record + sweather_per_region::ID, id);
        w.accumulator(record + sweather_per_region::MODIFIERS, &[(0x29, 3)]);
        let region = w.object(0x100, cstrategic_region::VTABLE_RVA.0);
        w.u32(region + cstrategic_region::ID, id);
        w.u32(region + 0x60, id + 1);
        regions.push(region);
    }
    // Sub-units and templates.
    let subunits: Vec<usize> = (0..6)
        .map(|index| {
            let unit = w.object(0x600, csub_unit_definition::VTABLE_RVA.0);
            w.u32(unit + 8, 40 + index);
            w.u64(unit + 0x58, 200_000 + u64::from(index) * 50_000);
            unit
        })
        .collect();
    let templates: Vec<usize> = (0..shape.templates)
        .map(|index| {
            let t = w.object(0x600, template::VTABLE_RVA.0);
            w.u64(
                t + 8,
                ((index as u64 + 1) << 32) | u64::from(template::REFERENCE_TYPE),
            );
            let terms = w.alloc(16 * 4);
            let count = 2 + index % 3;
            for term in 0..count {
                w.u64(terms + term * 16, subunits[(index + term) % 6] as u64);
                w.u32(terms + term * 16 + 8, 1 + term as u32);
            }
            w.array(t + 0xc0, terms, count as u32);
            w.u64(t + template::PRIORITY_UNIT, subunits[index % 6] as u64);
            // Every other template is an exile template of an owner.
            if index % 2 == 0 {
                w.u32(t + inputs::TEMPLATE_ORIGIN_TYPE, 2);
                w.u32(t + inputs::TEMPLATE_COUNTRY, owners[index % owners.len()]);
            }
            t
        })
        .collect();
    register_templates(&mut w, &templates);
    // Three naming groups the templates share, so several divisions'
    // generated names store one group key between them.
    let name_groups: Vec<usize> = (0..3)
        .map(|index| {
            let group = w.object(0x40, engine_native::cname_group_member::GROUP_VTABLE_RVA.0);
            w.text(
                group + engine_native::cname_group_member::GROUP_KEY,
                format!("GROUP_{index}").as_bytes(),
            );
            group
        })
        .collect();
    for (index, &t) in templates.iter().enumerate() {
        w.u64(
            t + engine_native::cname_group_member::TEMPLATE_NAME_GROUP,
            name_groups[index % name_groups.len()] as u64,
        );
    }
    // States, terrain and provinces, registered by id.
    let states: Vec<usize> = (0..shape.states)
        .map(|index| {
            let state = w.object(0x1000, cstate::VTABLE_RVA.0);
            w.u32(state + cstate::ID, 500 + index as u32);
            w.u32(state + cstate::CONTROLLER, owners[index % owners.len()]);
            let cores = w.alloc(4);
            w.u32(cores, owners[(index + 1) % owners.len()]);
            w.array(state + cstate::CORES, cores, 1);
            let buckets = w.alloc(0x40);
            w.u64(state + cstate::COUNTRY_MODIFIERS, buckets as u64);
            w.accumulator(
                state + cstate::RESOURCE_MODIFIER + 0x10,
                &[(0x2b, 4), (0x1e7, 9)],
            );
            state
        })
        .collect();
    let terrains: Vec<usize> = (0..2)
        .map(|index| {
            let terrain = w.object(0x100, cterrain_type::VTABLE_RVA.0);
            w.put(terrain + cterrain_type::VALID, &[1]);
            w.text(
                terrain + cterrain_type::NAME,
                [&b"plains"[..], &b"forest"[..]][index],
            );
            w.u64(
                terrain + cterrain_type::COMBAT_WIDTH,
                9_000_000 + index as u64,
            );
            terrain
        })
        .collect();
    let first_province = 1000;
    let registered = w.alloc((first_province + shape.provinces) * 8);
    w.array(
        GAME + cgame_state::PROVINCES,
        registered,
        (first_province + shape.provinces) as u32,
    );
    let provinces: Vec<usize> = (0..shape.provinces)
        .map(|index| {
            let province = w.object(0x200, cprovince::VTABLE_RVA.0);
            let definition = w.alloc(0x100);
            let id = first_province + index;
            w.u32(province + cprovince::ID, id as u32);
            w.u32(definition + province_definition::ID, id as u32);
            w.u64(province + cprovince::DEFINITION, definition as u64);
            w.u64(
                province + cprovince::STATE,
                states[index % states.len()] as u64,
            );
            if index % 4 != 3 {
                w.u64(
                    province + cprovince::STRATEGIC_REGION,
                    regions[index % 2] as u64,
                );
            }
            w.u64(registered + id * 8, province as u64);
            province
        })
        .collect();
    // Army leaders with their orders groups, resolved by reference.
    let groups = leaders(&mut w, shape.leaders, owners[0]);
    // Divisions.
    let total = shape.combats * shape.divisions;
    let mut armies = Vec::new();
    for index in 0..total {
        let army = w.object(0x800, carmy::VTABLE_RVA.0);
        w.u64(
            army + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        let handle = ((index as u64 + 1) << 32) | 3;
        w.u64(army + cunit::HANDLE, handle);
        let stats = w.object(0x600, csub_unit_definition::VTABLE_RVA.0);
        w.u64(army + cunit::STAT_BLOCK, stats as u64);
        let province = provinces[(index * 7) % provinces.len()];
        w.u64(army + cunit::LOCATION, province as u64);
        w.u64(
            army + carmy::STANDARD_TEMPLATE,
            templates[(index * 5) % templates.len()] as u64,
        );
        let owner = owners[index % owners.len()];
        w.u32(army + cunit::ROSTER_OWNER, owner);
        w.u32(army + cunit::LOGICAL_OWNER, owner);
        // Three naming shapes: a type-zero name from the template's shared
        // group, a player's custom name, and a generated name of nonzero
        // type (which reads nothing further).
        let name = w.alloc(0x100);
        w.u64(army + inputs::NAME_GROUP_MEMBER, name as u64);
        w.u64(name, engine_native::cname_group_member::VTABLE_RVA.0 as u64);
        w.u32(
            name + engine_native::cname_group_member::ORDER,
            index as u32,
        );
        let template = templates[(index * 5) % templates.len()];
        match index % 3 {
            0 => {
                w.text(name + engine_native::cname_group_member::OVERRIDE_NAME, b"");
                w.u64(
                    name + engine_native::cname_group_member::TEMPLATE_REFERENCE,
                    w.get_u64(template + 8),
                );
            }
            1 => w.text(
                name + engine_native::cname_group_member::OVERRIDE_NAME,
                format!("Guard {}", index % 1000).as_bytes(),
            ),
            _ => {
                w.u32(name + engine_native::cname_group_member::TYPE, 1);
                w.put(name + engine_native::cname_group_member::FLAG_B, &[1]);
            }
        }
        w.put(army + 0x2e, &[1]);
        if index < shape.transports {
            w.u64(army + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
            w.u64(army + cunit::LOCATION, provinces[0] as u64);
        }
        let modifiers = index % 3;
        let data = w.alloc(modifiers.max(1) * 24);
        for entry in 0..modifiers {
            w.u32(data + entry * 24, 10 + entry as u32);
            w.u64(data + entry * 24 + 8, (index * 100 + entry) as u64);
            w.u64(data + entry * 24 + 16, (entry as i64 - 1) as u64);
        }
        w.array(army + cunit::COMBAT_MODIFIER_DATA, data, modifiers as u32);
        if !groups.is_empty() && index % 4 == 1 {
            w.u64(
                army + carmy::ORDERS_GROUP,
                groups[(index / 4) % groups.len()] as u64,
            );
        }
        armies.push(army);
    }
    // Combats: a naval combat first, which the land census skips.
    let registry = w.alloc((shape.combats + 1) * 8);
    w.array(
        GAME + cgame_state::COMBATS + 8,
        registry,
        shape.combats as u32 + 1,
    );
    let naval = w.object(0x100, cnaval_combat::VTABLE_RVA.0);
    w.u64(registry, naval as u64);
    let mut combats = Vec::new();
    let mut members = Vec::new();
    let mut all_sides = Vec::new();
    for combat in 0..shape.combats {
        let object = w.object(0x600, cland_combat::VTABLE_RVA.0);
        w.u64(
            object + cland_combat::BODY,
            cland_combat::BODY_VTABLE_RVA.0 as u64,
        );
        w.u64(
            object + cland_combat::HANDLE,
            ((combat as u64 + 1) << 32) | u64::from(cland_combat::TYPE_WORD),
        );
        w.u64(
            object + cland_combat::PROVINCE,
            provinces[(combat * 3) % provinces.len()] as u64,
        );
        w.u64(
            object + cland_combat::STORED_TERRAIN,
            terrains[combat % 2] as u64,
        );
        w.put(object + cland_combat::INDICATOR_FRAME_HIGH, &[combat as u8]);
        w.u64(object + cland_combat::WIDTH_FACTOR, (combat as u64) * 1_000);
        let own = armies[combat * shape.divisions..(combat + 1) * shape.divisions].to_vec();
        let attackers = (shape.divisions / 2 + combat % 2).clamp(1, shape.divisions - 1);
        let mut pair = [0; 2];
        for (index, (units, is_attacker)) in [(&own[..attackers], 1_u8), (&own[attackers..], 0)]
            .into_iter()
            .enumerate()
        {
            let side = w.object(0x600, cland_combatant::VTABLE_RVA.0);
            w.u64(side + cland_combatant::COMBAT, object as u64);
            w.put(side + cland_combatant::IS_ATTACKER, &[is_attacker]);
            w.u32(side + cland_combatant::FLAGS, (combat * 2 + index) as u32);
            let list = w.alloc(units.len() * 8);
            for (ordinal, &unit) in units.iter().enumerate() {
                w.u64(list + ordinal * 8, unit as u64);
            }
            w.array(side + cland_combatant::UNITS, list, units.len() as u32);
            // Committed first, then one reserve, the rest unassigned.
            let committed = units.len().div_ceil(2);
            let reserves = usize::from(units.len() > committed);
            w.array(side + cland_combatant::COMMITTED, list, committed as u32);
            w.array(
                side + cland_combatant::RESERVES,
                list + committed * 8,
                reserves as u32,
            );
            pair[index] = side;
        }
        w.u64(object + cland_combat::ATTACKER, pair[0] as u64);
        w.u64(object + cland_combat::DEFENDER, pair[1] as u64);
        w.u64(registry + (combat + 1) * 8, object as u64);
        combats.push(object);
        members.push(own);
        all_sides.push(pair);
    }
    Built {
        world: w,
        #[cfg(test)]
        templates,
        #[cfg(test)]
        combats,
        #[cfg(test)]
        armies: members,
        #[cfg(test)]
        sides: all_sides,
    }
}

/// `count` category-2 army leaders (no formation), each in its own orders
/// group, resolvable through the high reference root. Returns the groups.
/// Registers the templates by their handle in the reference registry's
/// root for their type, one bucket each at its home slot.
fn register_templates(w: &mut World, templates: &[usize]) {
    use engine_native::{creference_registry as registry, reference_bucket as bucket};
    // At least twice the templates in buckets, a power of two: the
    // header's overflow distance is the exponent plus two.
    let buckets = (templates.len() * 2).next_power_of_two().max(8);
    let (mask, overflow) = (buckets as u32 - 1, buckets.trailing_zeros() as u8 + 2);
    let table = w.alloc(0x40);
    let entries = w.alloc((mask as usize + 1 + overflow as usize + 1) * bucket::STRIDE);
    w.u64(
        registry::LOW_ROOTS_RVA.0 + template::REFERENCE_TYPE as usize * 8,
        table as u64,
    );
    w.u64(table + registry::ENTRIES, entries as u64);
    w.u32(table + registry::COUNT, templates.len() as u32);
    w.u32(table + registry::MASK, mask);
    w.put(table + registry::OVERFLOW_DISTANCE, &[overflow]);
    w.put(
        entries + (mask as usize + 1 + overflow as usize) * bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
    // Robin Hood insertion, as the registry keeps its buckets: a probe
    // stops at a bucket whose distance is below its own, so an entry
    // displaced from home takes the slot of any closer-to-home entry.
    let mut buckets: Vec<Option<(u64, usize, u32)>> =
        vec![None; mask as usize + 1 + overflow as usize];
    for &t in templates {
        let mut entry = (w.get_u64(t + 8), t, 1_u32);
        let hash = entry
            .0
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let mut index = (((hash >> 32) as u32 ^ hash as u32) & mask) as usize;
        loop {
            assert!(
                entry.2 <= u32::from(overflow) + 1,
                "template buckets overflow"
            );
            match &mut buckets[index] {
                slot @ None => {
                    *slot = Some(entry);
                    break;
                }
                Some(resident) if resident.2 < entry.2 => std::mem::swap(resident, &mut entry),
                Some(_) => {}
            }
            index += 1;
            entry.2 += 1;
        }
    }
    for (index, filled) in buckets.iter().enumerate() {
        if let Some((handle, t, distance)) = *filled {
            let cell = entries + index * bucket::STRIDE;
            w.put(cell + bucket::DISTANCE, &[distance as u8]);
            w.u64(cell + 8, handle);
            w.u64(cell + 16, t as u64);
        }
    }
}

/// Divisions sharing group keys store each key once, as the reference
/// walk does: with a name arena too small for every name, both publish
/// the same rows, outcomes and demand, and after growth both keep all.
#[test]
#[cfg(test)]
fn shared_naming_keys_and_custom_names_match_the_reference_under_pressure() {
    let shape = Shape::small();
    let built = world(&shape);
    for names in [0, 8, 24, 64] {
        assert_equivalent_with(
            &built.world,
            Capacities {
                division_names: names,
                ..roomy()
            },
        );
    }
    // A naming group whose key cannot be read fails the same division in
    // both walks.
    let mut built = world(&shape);
    let group = built
        .world
        .get_u64(built.templates[0] + engine_native::cname_group_member::TEMPLATE_NAME_GROUP)
        as usize;
    built
        .world
        .poison(group + engine_native::cname_group_member::GROUP_KEY + 16, 8);
    assert_equivalent(&built.world);
}

fn leaders(w: &mut World, count: usize, country: u32) -> Vec<usize> {
    use engine_native::{
        carmy_leader, cnull_combat_tactic, corders_group, creference_registry as registry,
        cunit_leader, cunit_leader_skill, reference_bucket as bucket,
    };
    if count == 0 {
        return Vec::new();
    }
    let skill = w.object(0x200, cunit_leader_skill::VTABLE_RVA.0);
    w.u32(skill + cunit_leader_skill::LEVEL, 3);
    w.u32(skill + cunit_leader_skill::CATEGORY, 2);
    let skills = w.alloc(0x200);
    w.u64(
        engine_native::naval_commander_inputs::SKILL_DATABASE_RVA.0,
        skills as u64,
    );
    let levels = w.alloc(8 * 5);
    for level in 0..5 {
        w.u64(levels + level * 8, skill as u64);
    }
    w.array(skills + 0xe8 + 2 * 0x18, levels, 5);
    let preferred = w.object(0x100, cnull_combat_tactic::VTABLE_RVA.0);
    // One bucket per leader in a 64-slot table, without collisions.
    let (mask, overflow) = (63_u32, 8_u8);
    let table = w.alloc(0x40);
    let entries = w.alloc((mask as usize + 1 + overflow as usize + 1) * bucket::STRIDE);
    w.u64(registry::HIGH_ROOT_RVA.0, table as u64);
    w.u64(table + registry::ENTRIES, entries as u64);
    w.u32(table + registry::COUNT, count as u32);
    w.u32(table + registry::MASK, mask);
    w.put(table + registry::OVERFLOW_DISTANCE, &[overflow]);
    w.put(
        entries + (mask as usize + 1 + overflow as usize) * bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
    let mut used = Vec::new();
    let mut groups = Vec::new();
    let mut id = 0_u64;
    while groups.len() < count {
        id += 1;
        let handle = (id << 32) | u64::from(cunit_leader::REFERENCE_TYPE);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = ((hash >> 32) as u32 ^ hash as u32) & mask;
        if used.contains(&slot) {
            continue;
        }
        used.push(slot);
        let leader = w.object(0x1100, carmy_leader::VTABLE_RVA.0);
        w.u64(leader + cunit_leader::TYPE, handle);
        w.u64(leader + cunit_leader::SKILL, skill as u64);
        w.u32(leader + cunit_leader::CATEGORY, 2);
        w.u64(leader + cunit_leader::EXPERIENCE, 10 * id);
        w.text(
            leader + cunit_leader::NAME,
            format!("leader {id}").as_bytes(),
        );
        // The raw saved name the detail and division commander read.
        w.text(
            leader + cunit_leader::TOKEN,
            format!("LEADER_{id}").as_bytes(),
        );
        // An empty inline description token.
        w.u64(leader + cunit_leader::DESCRIPTION_TOKEN + 24, 15);
        for offset in [
            carmy_leader::MOVEMENT_MODIFIER,
            carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER,
        ] {
            w.accumulator(leader + offset + 0x10, &[]);
        }
        for offset in [0xc0, 0xe0, 0x100] {
            w.text(leader + offset, b"fallback");
        }
        w.u32(leader + 0x120, country);
        w.u64(
            leader + land_tactic_inputs::LEADER_PREFERRED_TACTIC,
            preferred as u64,
        );
        let cell = entries + slot as usize * bucket::STRIDE;
        w.put(cell + bucket::DISTANCE, &[1]);
        w.u64(cell + 8, handle);
        w.u64(cell + 16, leader as u64);
        let group = w.object(corders_group::SIZE, corders_group::VTABLE_RVA.0);
        w.u64(
            group + corders_group::HANDLE,
            (id << 32) | u64::from(corders_group::REFERENCE_TYPE),
        );
        w.u64(group + corders_group::LEADER, leader as u64);
        groups.push(group);
    }
    groups
}

/// Large enough for every fixture here, so nothing truncates.
fn roomy() -> Capacities {
    Capacities {
        divisions: 8192,
        details: 1024,
        sides: 2048,
        ..Capacities::default()
    }
}

#[test]
#[cfg(test)]
fn live_detail_demand_presizes_attach_and_later_combat_arena_growth() {
    let mut capacities = Capacities::default();
    for (combats, divisions) in [(66, 5), (71, 17), (1500, 3), (5000, 3)] {
        let built = world(&Shape {
            #[cfg(test)]
            combats,
            divisions,
            ..Shape::small()
        });
        let demand = engine_native::cland_combat::detail_demand(&built.world, 0, GAME).unwrap();
        assert_eq!(demand.rows, combats);
        assert_eq!(demand.census_participants, combats * divisions);
        let old = capacities;
        capacities.raise_for_live_details(demand);
        assert!(capacities.details >= combats);
        assert!(capacities.census_participants >= demand.census_participants);
        assert!(capacities.rosters >= demand.arenas.participants);
        assert!(capacities.air_counts >= demand.arenas.air);
        assert!(capacities.detail_text >= demand.arenas.text);
        assert!(capacities.commander_traits >= demand.arenas.traits);
        assert!(capacities.timed_traits >= demand.arenas.timed);
        assert!(capacities.learning_traits >= demand.arenas.learning);
        assert!(capacities.details >= old.details);
        let mut captured = Captured::with_capacities(&old);
        captured.try_grow_hint(&capacities).unwrap();
        pass(&mut captured, &built.world);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: combats });
        assert_eq!(captured.detail_text.required, demand.arenas.text);
        assert_eq!(captured.air_counts.required, demand.arenas.air);
        assert_eq!(captured.rosters.required, demand.arenas.participants);
        let retained = capacities;
        capacities.raise_for_live_details(Default::default());
        assert!(capacities == retained);
    }
}

#[test]
#[cfg(test)]
fn combat_details_hint_reserve_failure_keeps_existing_lengths() {
    let capacities = Capacities::default();
    let mut captured = Captured::with_capacities(&capacities);
    let mut hint = capacities;
    hint.combats += 1;
    hint.details = usize::MAX;
    assert!(captured.try_grow_hint(&hint).is_err());
    assert_eq!(captured.census.len(), capacities.combats);
    assert_eq!(captured.details.rows.len(), capacities.details);
    captured.try_grow_hint(&capacities).unwrap();
}

pub(super) fn allocation_failure_exercise(arm: impl FnOnce(), disarm: impl FnOnce()) {
    let built = world(&Shape::small());
    let mut capacities = roomy();
    arm();
    let scope = crate::allocation::enter(crate::allocation::Phase::Extended);
    let mut captured = Captured::with_capacities(&capacities);
    for _ in 0..3 {
        if crate::allocation::failed() {
            break;
        }
        captured.reset();
        captured.capture(&built.world, 0, GAME);
        if crate::allocation::failed() {
            break;
        }
        std::hint::black_box(captured.tables());
        if crate::allocation::failed() {
            break;
        }
        captured.grow(&mut capacities);
    }
    drop(scope);
    disarm();
}

/// Everything a walk publishes: tables, outcomes, and the demand of every
/// arena and census.
#[derive(Debug, PartialEq)]
#[cfg(test)]
struct Observed {
    tables: [TableView; N],
    outcomes: [Outcome; N],
    demand: Vec<usize>,
}

#[cfg(test)]
fn demand(captured: &Captured) -> Vec<usize> {
    vec![
        captured.rosters.required,
        captured.members.required,
        captured.modifiers.required,
        captured.width_terms.required,
        captured.exile_tags.required,
        captured.division_names.required,
        captured.country_tags.required,
        captured.air_counts.required,
        captured.detail_text.required,
        captured.commander_traits.required,
        captured.timed_traits.required,
        captured.learning_traits.required,
        captured.required_census,
        captured.required_census_participants,
    ]
}

#[cfg(test)]
fn observe(captured: &Captured) -> Observed {
    Observed {
        tables: captured.tables(),
        outcomes: captured.outcomes(),
        demand: demand(captured),
    }
}

#[cfg(test)]
fn pass(captured: &mut Captured, world: &impl Memory) {
    captured.reset();
    captured.capture(world, 0, GAME);
}

#[cfg(test)]
fn run(world: &impl Memory, c: Capacities) -> Observed {
    let mut captured = Captured::with_capacities(&c);
    pass(&mut captured, world);
    observe(&captured)
}

/// One walk as main's readers read: nothing shared (see `Destination`).
#[cfg(test)]
fn run_reference(world: &impl Memory, c: Capacities) -> Observed {
    let mut captured = Captured::with_capacities(&c);
    captured.reference = true;
    pass(&mut captured, world);
    observe(&captured)
}

#[test]
fn deferred_reserve_arithmetic_preserves_rows_refusals_and_native_read_order() {
    let mut built = world(&Shape::small());
    for fault in [None, Some((reserve_speed::SLOWEST_SPEED_RVA.0, 8))] {
        built.world.poisoned = fault.into_iter().collect();
        *built.world.log.lock().unwrap() = Some(Vec::new());
        let expected = run(&built.world, roomy());
        let reads = built.world.log.lock().unwrap().take().unwrap();
        *built.world.log.lock().unwrap() = Some(Vec::new());
        let mut deferred = Captured::with_capacities(&roomy());
        deferred.reset();
        deferred.capture_deferred(&built.world, 0, GAME);
        assert_eq!(built.world.log.lock().unwrap().as_ref().unwrap(), &reads);
        built.world.poisoned = vec![(0, usize::MAX)];
        let bytes = deferred.allocated_bytes() as u64;
        let tail_bytes = deferred.owned_tail_allocated_bytes() as u64;
        deferred.settle_owned();
        assert_eq!(
            (deferred.allocated_bytes() as u64).wrapping_sub(bytes),
            (deferred.owned_tail_allocated_bytes() as u64).wrapping_sub(tail_bytes)
        );
        assert_eq!(built.world.log.lock().unwrap().as_ref().unwrap(), &reads);
        assert_eq!(observe(&deferred), expected);
        deferred.settle_owned();
        assert_eq!(observe(&deferred), expected);
    }
}

/// The walk and the reference walk publish the same, with and without
/// the authenticated roster: every table, outcome, failure and demand.
#[cfg(test)]
fn assert_equivalent_with(world: &World, c: Capacities) -> Observed {
    let cache = roster(world);
    let rostered = cache.roster_view(world);
    let one = run(world, c);
    assert_eq!(run_reference(world, c), one, "reference walk");
    let with_roster = run(&rostered, c);
    assert_eq!(
        run_reference(&rostered, c),
        with_roster,
        "reference walk, roster"
    );
    if one
        .outcomes
        .iter()
        .all(|outcome| !matches!(outcome, Outcome::Unavailable { .. }))
    {
        assert_eq!(with_roster, one, "authenticated roster");
    }
    one
}

#[cfg(test)]
fn assert_equivalent(world: &World) -> Observed {
    assert_equivalent_with(world, roomy())
}

/// Every failure is the reference walk's: a read that fails at a chosen
/// ordinal of the reference walk publishes the tables, outcomes and
/// demand the reference walk publishes.
#[test]
#[cfg(test)]
fn walk_fails_as_the_reference_walk() {
    let shape = Shape::small();
    let mut built = world(&shape);
    *built.world.log.lock().unwrap() = Some(Vec::new());
    run_reference(&built.world, roomy());
    let log = built.world.log.lock().unwrap().take().unwrap();
    let mut seen = std::collections::HashSet::new();
    let spans: Vec<_> = log.into_iter().filter(|span| seen.insert(*span)).collect();
    let step = (spans.len() / 150).max(1);
    let mut failed = 0;
    for &span in spans.iter().step_by(step) {
        built.world.poisoned = vec![span];
        let one = assert_equivalent(&built.world);
        failed += usize::from(
            one.outcomes
                .iter()
                .any(|outcome| matches!(outcome, Outcome::Unavailable { .. })),
        );
    }
    assert!(failed > 0);
}

#[cfg(test)]
fn rows(view: &TableView) -> usize {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
        _ => 0,
    }
}

/// Every table complete and identical to the reference walk's, with
/// shared owners, templates, states, provinces and regions, leaders,
/// transport divisions, a naval combat the census skips and a tactic
/// registry.
#[test]
#[cfg(test)]
fn family_walk_publishes_the_reference_tables() {
    let shape = Shape::small();
    let built = world(&shape);
    let one = assert_equivalent(&built.world);
    for (table, view) in one.tables.iter().enumerate() {
        assert!(
            matches!(view, TableView::Complete(_)),
            "table {table}: {view:?}"
        );
    }
    assert_eq!(rows(&one.tables[0]), shape.combats);
    assert_eq!(rows(&one.tables[1]), shape.combats * shape.divisions);
    assert_eq!(rows(&one.tables[2]), shape.combats * 2);
    // LandCombatDivisionModifiers holds a row a member.
    assert_eq!(rows(&one.tables[4]), shape.combats * shape.divisions);
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDivisions(divisions)) =
        &one.tables[1]
    else {
        unreachable!()
    };
    // Commanded divisions keep their commander handle; the retired
    // commander hover inputs and reinforcement operands are tombstones, and
    // only a reserve carries a speed share, composed or refused.
    let commanded = divisions.iter().filter(|row| row.commander != 0).count();
    assert!(divisions.iter().all(|row| {
        row.commander_inputs == Retired
            && row.reinforcement == Retired
            && (row.role == 2 || row.reinforcement_speed == Recorded::Value(None))
            && matches!(row.reinforcement_speed, Recorded::Value(_))
    }));
    assert_eq!(
        commanded,
        (0..shape.combats * shape.divisions)
            .filter(|index| index % 4 == 1)
            .count()
    );
    assert!(divisions.iter().any(|row| !row.width_terms.is_empty()));
    // Each reserve's country prints its reinforcement-rate tree (here a
    // childless root of 7), read once however many reserves it has; a
    // commander row exists only for a nonzero root and keeps its name.
    let TableView::Complete(replay_data::rows::TableRows::ReserveModifierTrees(trees)) =
        &one.tables[5]
    else {
        unreachable!()
    };
    assert!(divisions.iter().any(|row| row.role == 2));
    let countries: Vec<_> = trees.iter().filter(|row| row.kind == 0).collect();
    assert!(!countries.is_empty());
    for row in &countries {
        assert!(row.name.is_empty());
        assert_eq!(
            row.nodes.as_deref(),
            Some(
                &[replay_data::rows::ModifierBreakdownNode {
                    children: 0,
                    value: 7,
                    name: Vec::new(),
                }][..]
            )
        );
    }
    assert!(trees.windows(2).all(|pair| pair[0].key() < pair[1].key()));
    assert!(
        trees
            .iter()
            .filter(|row| row.kind != 0)
            .all(|row| row.nodes.as_ref().is_some_and(|nodes| nodes[0].value != 0))
    );
    // Committed divisions on exile templates carry their exile operands;
    // every division carries its generated name.
    assert!(divisions.iter().any(|row| row.exile.is_some()));
    assert!(divisions.iter().any(|row| row.exile.is_none()));
    // Group, custom and generated names all occur.
    assert!(divisions.iter().any(|row| !row.name_group.is_empty()));
    assert!(divisions.iter().any(|row| !row.name_custom.is_empty()));
    assert!(divisions.iter().any(|row| row.name_programmatic != 0));
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(details)) =
        &one.tables[0]
    else {
        unreachable!()
    };
    assert!(
        details
            .iter()
            .any(|row| row.sides.iter().any(|side| side.commander != 0))
    );
    let TableView::Complete(replay_data::rows::TableRows::LandCombatModifiers(sides)) =
        &one.tables[2]
    else {
        unreachable!()
    };
    assert!(sides.iter().all(|side| side.divisions == Retired));
    let TableView::Complete(replay_data::rows::TableRows::LandCombatDivisionModifiers(members)) =
        &one.tables[4]
    else {
        unreachable!()
    };
    assert!(members.iter().any(|member| !member.entries.is_empty()));
    // One row a member, keyed as its LandCombatDivisions row.
    assert_eq!(
        members
            .iter()
            .map(|member| (member.combat, member.side, member.ordinal, member.division))
            .collect::<Vec<_>>(),
        divisions
            .iter()
            .map(|row| (row.combat, row.side, row.ordinal, row.division))
            .collect::<Vec<_>>()
    );
}

#[cfg(test)]
fn reason(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Unavailable { reason } => reason,
        other => panic!("expected a failure, got {other:?}"),
    }
}

/// Guarded division failures keep the earliest combat's reason.
#[test]
#[cfg(test)]
fn earliest_guarded_failure_wins() {
    let shape = Shape::small();
    let stat_block = |built: &mut Built, combat: usize| {
        let army = built.armies[combat][0];
        built.world.poison(army + cunit::STAT_BLOCK, 8);
    };
    let stat_vtable = |built: &mut Built, combat: usize| {
        let army = built.armies[combat][1];
        let stats = built.world.get_u64(army + cunit::STAT_BLOCK) as usize;
        built.world.u64(stats, 0x1234);
    };
    // Each failure alone, to show the reasons differ.
    let mut built = world(&shape);
    stat_block(&mut built, 3);
    let first = reason(assert_equivalent(&built.world).outcomes[1]);
    let mut built = world(&shape);
    stat_vtable(&mut built, 4);
    let second = reason(assert_equivalent(&built.world).outcomes[1]);
    assert_ne!(first, second);
    for (early, late, expected) in [(3, 4, first), (2, 5, second)] {
        let mut built = world(&shape);
        if expected == first {
            stat_block(&mut built, early);
            stat_vtable(&mut built, late);
        } else {
            stat_vtable(&mut built, early);
            stat_block(&mut built, late);
        }
        let observed = assert_equivalent(&built.world);
        assert_eq!(reason(observed.outcomes[1]), expected);
        assert!(matches!(observed.tables[0], TableView::Complete(_)));
    }
}

/// A failure that replaces earlier ones (a combat's unit list) keeps the
/// latest combat's reason, also over an earlier or later guarded failure.
#[test]
#[cfg(test)]
fn latest_overwriting_failure_wins() {
    let shape = Shape::small();
    let duplicate = |built: &mut Built, combat: usize| {
        let side = built.sides[combat][0];
        let list = built.world.get_u64(side + cland_combatant::COMMITTED) as usize;
        let first = built.world.get_u64(list);
        built.world.u64(list + 8, first);
    };
    let overlap = |built: &mut Built, combat: usize| {
        let side = built.sides[combat][0];
        let list = built.world.get_u64(side + cland_combatant::COMMITTED);
        built.world.u64(side + cland_combatant::RESERVES, list);
    };
    for (duplicated, overlapped, expected) in [
        (1, 4, "land role subsets overlap"),
        (5, 2, "land role subset has duplicate unit"),
        (0, 6, "land role subsets overlap"),
    ] {
        let mut built = world(&shape);
        duplicate(&mut built, duplicated);
        overlap(&mut built, overlapped);
        // A guarded division failure between them never replaces one.
        let army = built.armies[3][0];
        built.world.poison(army + cunit::STAT_BLOCK, 8);
        let observed = assert_equivalent(&built.world);
        assert_eq!(reason(observed.outcomes[1]), expected);
        assert_eq!(reason(observed.outcomes[2]), expected);
    }
}

/// A division read failure fails the division table alone and a detail
/// failure the detail table alone, whichever combat each is in, as the
/// reference walk publishes them.
#[test]
#[cfg(test)]
fn division_and_detail_failures_stay_in_their_tables() {
    let shape = Shape::small();
    // The name member is in the division's army image.
    let division = |built: &mut Built, combat: usize| {
        let army = built.armies[combat][0];
        built.world.poison(army + inputs::NAME_GROUP_MEMBER, 8);
    };
    // The stored terrain is read only by detail(): point it at the combat.
    let detail = |built: &mut Built, combat: usize| {
        let object = built.combats[combat];
        built
            .world
            .u64(object + cland_combat::STORED_TERRAIN, object as u64);
    };
    for (divided, detailed) in [(1, 2), (4, 1), (3, 3)] {
        let mut built = world(&shape);
        division(&mut built, divided);
        detail(&mut built, detailed);
        let observed = assert_equivalent(&built.world);
        let [details, divisions] = [0, 1].map(|table| reason(observed.outcomes[table]));
        assert_ne!(details, divisions);
        let mut built = world(&shape);
        detail(&mut built, detailed);
        assert_eq!(reason(assert_equivalent(&built.world).outcomes[0]), details);
    }
}

/// A census that fails at a later combat stops the walk there; an
/// earlier division failure still names the division table.
#[test]
#[cfg(test)]
fn census_failure_stops_the_walk_at_its_combat() {
    let shape = Shape::small();
    let mut built = world(&shape);
    let army = built.armies[5][0];
    built.world.poison(army + carmy::SECONDARY_BASE, 16);
    let census = reason(assert_equivalent(&built.world).outcomes[0]);
    let army = built.armies[2][0];
    built.world.poison(army + cunit::STAT_BLOCK, 8);
    let observed = assert_equivalent(&built.world);
    assert_eq!(reason(observed.outcomes[0]), census);
    assert_eq!(reason(observed.outcomes[2]), census);
    assert_ne!(reason(observed.outcomes[1]), census);
}

/// Small initial row storage grows before the first publication and preserves
/// the unshared reference walk's rows, faults and demand.
#[test]
#[cfg(test)]
fn worker_rows_match_the_reference_without_a_truncated_first_hour() {
    let shape = Shape {
        combats: 5,
        ..Shape::small()
    };
    let built = world(&shape);
    let mut c = Capacities {
        details: 4,
        sides: 8,
        ..Capacities::default()
    };
    let truncated = assert_equivalent_with(&built.world, c);
    assert_eq!(truncated.outcomes[0], Outcome::Rows { len: 5 });
    assert_eq!(truncated.outcomes[2], Outcome::Rows { len: 10 });
    let mut captured = Captured::with_capacities(&c);
    pass(&mut captured, &built.world);
    captured.grow(&mut c);
    pass(&mut captured, &built.world);
    let grown = observe(&captured);
    assert_eq!(grown, run_reference(&built.world, c));
    assert!(
        grown
            .tables
            .iter()
            .all(|view| matches!(view, TableView::Complete(_)))
    );
}

/// Every leader a combat's army points to.
#[cfg(test)]
fn army_leaders(built: &Built) -> Vec<usize> {
    let mut leaders = Vec::new();
    for &army in built.armies.iter().flatten() {
        let group = built.world.get_u64(army + carmy::ORDERS_GROUP) as usize;
        if group == 0 {
            continue;
        }
        let leader = built
            .world
            .get_u64(group + engine_native::corders_group::LEADER) as usize;
        if !leaders.contains(&leader) {
            leaders.push(leader);
        }
    }
    leaders
}

#[cfg(test)]
fn detail_rows(view: &TableView) -> &[LandCombatDetailsRow] {
    match view {
        TableView::Complete(replay_data::rows::TableRows::LandCombatDetails(rows)) => rows,
        other => panic!("expected complete details, got {other:?}"),
    }
}

#[cfg(test)]
fn border_war_hooks_after_neighbouring_text(reference: bool) {
    use engine_native::{
        cland_border_war_combat as combat, cland_border_war_combatant as side,
        cunit_leader as leader, cunit_leader_trait as trait_, msvc_string,
    };
    let shape = Shape {
        countries: 5_000,
        combats: 3,
        ..Shape::small()
    };
    let mut built = world(&shape);
    let heap_text = |world: &mut World, at: usize, value: &[u8]| {
        assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
        let heap = world.alloc(value.len() + 1);
        world.put(heap, value);
        world.put(heap + value.len(), &[0]);
        world.u64(at + msvc_string::DATA, heap as u64);
        world.u64(at + msvc_string::LENGTH, value.len() as u64);
        world.u64(at + msvc_string::CAPACITY, value.len() as u64);
    };
    let name = b"synthetic_commander_name_token";
    let description = b"synthetic_commander_description";
    let trait_token = b"synthetic_commander_trait_token";
    for pointer in army_leaders(&built) {
        let w = &mut built.world;
        heap_text(w, pointer + leader::TOKEN, name);
        heap_text(w, pointer + leader::DESCRIPTION_TOKEN, description);
        let trait_pointer = w.object(0xb00, trait_::VTABLE_RVA.0);
        w.u32(trait_pointer + trait_::TOKEN_ID, 80);
        w.u32(trait_pointer + trait_::ORDINAL, 1);
        heap_text(w, trait_pointer + trait_::TOKEN, trait_token);
        let assigned = w.alloc(8);
        w.u64(assigned, trait_pointer as u64);
        w.array(pointer + leader::TRAIT_DATA, assigned, 1);
        let learning = w.alloc(16);
        w.u64(learning, trait_pointer as u64);
        w.array(pointer + leader::TRAIT_EXPERIENCE_DATA, learning, 1);
    }
    let win = b"tst_border_incidents_events.1";
    let lose = b"tst_border_incidents_events.2";
    for index in 1..shape.combats {
        let pointer = built.combats[index];
        let w = &mut built.world;
        w.u64(pointer, combat::VTABLE_RVA.0 as u64);
        w.u64(pointer + combat::BODY, combat::BODY_VTABLE_RVA.0 as u64);
        for (ordinal, pointer) in built.sides[index].into_iter().enumerate() {
            w.u64(pointer, side::VTABLE_RVA.0 as u64);
            let state = w.object(0x100, cstate::VTABLE_RVA.0);
            w.u32(state + cstate::ID, ordinal as u32 + 1);
            w.u64(pointer + side::STATE, state as u64);
            w.u32(pointer + side::COUNTRY, ordinal as u32 + 1);
            for (offset, value) in
                [side::WIN_EVENT, side::LOSE_EVENT]
                    .into_iter()
                    .zip(if ordinal == 0 {
                        [win, lose]
                    } else {
                        [lose, win]
                    })
            {
                if index == 1 {
                    heap_text(w, pointer + offset, value);
                } else {
                    w.text(pointer + offset, b"");
                }
            }
        }
    }
    let mut captured = Captured::with_capacities(&roomy());
    captured.reference = reference;
    pass(&mut captured, &built.world);
    let tables = captured.tables();
    let rows = detail_rows(&tables[0]);
    assert_eq!(rows.len(), shape.combats);
    let recorded = |row: usize| rows[row].border_war.require("border_war").unwrap().as_ref();
    assert!(recorded(0).is_none());
    let border = recorded(1).unwrap();
    for (side, expected) in border.sides.iter().zip([[win, lose], [lose, win]]) {
        assert_eq!(side.win_event, expected[0]);
        assert_eq!(side.lose_event, expected[1]);
    }
    assert!(
        recorded(2)
            .unwrap()
            .sides
            .iter()
            .all(|side| side.win_event.is_empty() && side.lose_event.is_empty())
    );
    let commanded: Vec<_> = rows
        .iter()
        .flat_map(|row| &row.sides)
        .filter(|side| side.commander != 0)
        .collect();
    assert!(!commanded.is_empty());
    for side in commanded {
        assert_eq!(side.commander_name_token, name);
        assert_eq!(side.traits[0].token, trait_token);
    }
}

#[test]
#[cfg(test)]
fn border_war_hooks_relocate_after_neighbouring_heap_text() {
    border_war_hooks_after_neighbouring_text(false);
}

#[test]
#[cfg(test)]
fn border_war_hooks_reference_relocates_after_neighbouring_heap_text() {
    border_war_hooks_after_neighbouring_text(true);
}

/// A selected commander's stored name and token use ordinary MSVC strings
/// of any length, empty included. A long name grows the land-detail text arena
/// during its original read and publishes whole, as the reference walk does.
#[test]
#[cfg(test)]
fn selected_commander_texts_keep_native_length_and_grow() {
    use engine_native::{cunit_leader::DESCRIPTION_TOKEN, cunit_leader::TOKEN};
    let shape = Shape::small();
    let mut built = world(&shape);
    let leaders = army_leaders(&built);
    assert!(!leaders.is_empty());
    let long = vec![b'k'; 3000];
    let heap = built.world.alloc(long.len() + 1);
    built.world.put(heap, &long);
    let empty = built.world.alloc(32);
    for &leader in &leaders {
        built.world.u64(leader + TOKEN, heap as u64);
        built.world.u64(leader + TOKEN + 16, long.len() as u64);
        built.world.u64(leader + TOKEN + 24, long.len() as u64);
        // An empty heap description.
        built.world.u64(leader + DESCRIPTION_TOKEN, empty as u64);
        built.world.u64(leader + DESCRIPTION_TOKEN + 16, 0);
        built.world.u64(leader + DESCRIPTION_TOKEN + 24, 31);
    }
    let mut c = Capacities {
        detail_text: 64,
        ..roomy()
    };
    let truncated = assert_equivalent_with(&built.world, c);
    assert!(
        matches!(truncated.outcomes[0], Outcome::Rows { .. }),
        "{:?}",
        truncated.outcomes[0]
    );
    let mut captured = Captured::with_capacities(&c);
    pass(&mut captured, &built.world);
    captured.grow(&mut c);
    pass(&mut captured, &built.world);
    let grown = observe(&captured);
    assert_eq!(grown, run_reference(&built.world, c));
    assert!(
        grown
            .tables
            .iter()
            .all(|view| matches!(view, TableView::Complete(_)))
    );
    let commanded: Vec<_> = detail_rows(&grown.tables[0])
        .iter()
        .flat_map(|row| row.sides.iter())
        .filter(|side| side.commander != 0)
        .collect();
    assert!(!commanded.is_empty());
    assert!(
        commanded
            .iter()
            .all(|side| side.commander_name_token == long)
    );

    // An empty name is a valid text, not a failure.
    for &leader in &leaders {
        built.world.u64(leader + TOKEN, 0);
        built.world.text(leader + TOKEN, b"");
    }
    let one = assert_equivalent(&built.world);
    let commanded: Vec<_> = detail_rows(&one.tables[0])
        .iter()
        .flat_map(|row| row.sides.iter())
        .filter(|side| side.commander != 0)
        .collect();
    assert!(!commanded.is_empty());
    assert!(
        commanded
            .iter()
            .all(|side| side.commander_name_token.is_empty())
    );
}

/// The live rate on the run3 save: 44,741 guarded calls in 0.96 ms.
#[cfg(test)]
const LIVE_NS_PER_CALL: f64 = 21.5;

/// Guarded calls per pass for the whole land combat family (census,
/// details, divisions and modifiers) over the authenticated roster, as
/// the Ordinary job reads, at 153, 1,000, 3,000, 5,000, 10,000 and 50,000 divisions in
/// combat: the reference walk (main's reads, nothing shared) and the
/// walk. A gather (`copy_records`) or scatter is one guarded call, as
/// live. The estimate uses run3's 21.5 ns per guarded call; the
/// fixture's own time is its slow sparse memory, not the game's. Run in
/// release with `--ignored --nocapture`.
#[test]
#[ignore]
#[cfg(test)]
fn family_cost_at_scale() {
    const PASSES: usize = 5;
    let live = |calls: usize| calls as f64 * LIVE_NS_PER_CALL / 1e6;
    println!(
        "| divisions | combats | reference calls | walk calls | walk live ms | calls per division | fixture ms |"
    );
    println!("|---|---|---|---|---|---|---|");
    let mut attribution = Vec::new();
    let mut kinds = Vec::new();
    let mut by_stage = Vec::new();
    for divisions in [153, 1_000, 3_000, 5_000, 10_000, 50_000] {
        let shape = Shape::scale(divisions);
        let built = world(&shape);
        let w = &built.world;
        let cache = roster(w);
        let view = &cache.roster_view(w);
        let mut capacities = Capacities::default();
        let mut walk = Captured::with_capacities(&capacities);
        // Grow to the census and warm storage, as a running recorder has.
        for _ in 0..3 {
            pass(&mut walk, view);
            walk.grow(&mut capacities);
        }
        pass(&mut walk, view);
        let published = observe(&walk);
        assert!(
            published
                .tables
                .iter()
                .all(|view| matches!(view, TableView::Complete(_)))
        );
        let mut reference = Captured::with_capacities(&capacities);
        reference.reference = true;
        pass(&mut reference, view);
        w.calls.store(0, Ordering::Relaxed);
        pass(&mut reference, view);
        let reference_calls = w.calls.swap(0, Ordering::Relaxed);
        assert_eq!(observe(&reference), published);
        let (mut fixture_ms, mut calls) = (Vec::new(), 0);
        for _ in 0..PASSES {
            walk.reset();
            w.calls.store(0, Ordering::Relaxed);
            let started = std::time::Instant::now();
            walk.capture(view, 0, GAME);
            fixture_ms.push(started.elapsed().as_secs_f64() * 1e3);
            calls = w.calls.swap(0, Ordering::Relaxed);
        }
        fixture_ms.sort_by(f64::total_cmp);
        assert_eq!(observe(&walk), published);
        let tables = per_table(w, &cache, |memory| {
            walk.reset();
            walk.capture(memory, 0, GAME);
        });
        kinds.push((divisions, calls_by_object(&built, view, &mut walk)));
        let total = shape.combats * shape.divisions;
        let stages = stages(w, view, tables);
        assert_eq!(
            stages.iter().map(|(_, calls)| calls).sum::<u64>(),
            calls as u64,
            "stages cover the walk"
        );
        by_stage.push((total, stages));
        attribution.push(format!(
            "| {total} | {} | {} | {} |",
            tables[0], tables[1], tables[2]
        ));
        println!(
            "| {total} | {} | {reference_calls} | {calls} | {:.2} | {:.1} | {:.2} |",
            shape.combats,
            live(calls),
            calls as f64 / total as f64,
            fixture_ms[PASSES / 2],
        );
    }
    println!();
    println!("Guarded calls by table, as the recorder attributes them:");
    println!("| divisions | details | divisions table | modifiers |");
    println!("|---|---|---|---|");
    for row in attribution {
        println!("{row}");
    }
    println!();
    println!("Guarded calls by stage (live ms at 21.5 ns per call):");
    for (divisions, stages) in by_stage {
        let row: Vec<String> = stages
            .iter()
            .map(|(stage, calls)| format!("{stage} {calls} ({:.2} ms)", live(*calls as usize)))
            .collect();
        println!("{divisions} divisions: {}", row.join("; "));
    }
    println!();
    println!("The walk's guarded calls by the object each call reads first:");
    for (divisions, kinds) in kinds {
        println!("{divisions} divisions: {kinds:?}");
    }
}

/// One walk's guarded calls by stage over `memory` (the roster view of
/// `w`). Each stage runs in the walk's order with the walk's shares and
/// counts as the difference from the stages before it; the division
/// readers (with their commanders) and the modifier members are what the
/// divisions and modifiers tables (`tables`) count beyond those stages.
#[cfg(test)]
fn stages(w: &World, memory: &impl Memory, tables: [u64; N]) -> Vec<(&'static str, u64)> {
    use engine_native::land_tactic_inputs as tactic;
    let through = |stage: u8| {
        let mut air = vec![0; 1 << 12];
        let mut text = vec![0; 1 << 16];
        let mut traits = vec![Default::default(); 1 << 12];
        let mut timed = vec![Default::default(); 1 << 12];
        let mut learning = vec![Default::default(); 1 << 12];
        let mut census = vec![cland_combat::Row::default(); 4096];
        let mut participants = vec![0; 1 << 17];
        let mut pointers = vec![0; 1 << 17];
        let mut roles = vec![0; 1 << 12];
        let mut images = engine_native::combat_sources::Images::default();
        let mut rosters = vec![0; 1 << 12];
        w.calls.store(0, Ordering::Relaxed);
        let walk = cland_combat::walk_members(
            memory,
            0,
            GAME,
            &mut census,
            &mut participants,
            &mut pointers,
            |source, members| {
                let mut commanders = cland_combat::Commanders::default();
                if stage >= 1 {
                    let mut arenas = cland_combat::DetailArenas::new(
                        &mut rosters,
                        &mut air,
                        &mut text,
                        &mut traits,
                        &mut timed,
                        &mut learning,
                    );
                    cland_combat::read_details(
                        memory,
                        0,
                        source,
                        &members,
                        &mut commanders,
                        &mut arenas,
                    )
                    .unwrap();
                    tactic::capture_context(memory, 0, GAME, source, &members, &mut commanders);
                }
                if stage >= 2 {
                    cland_combat::read_modifiers_context(memory, 0, source).unwrap();
                }
                if stage >= 3 {
                    images.capture(memory, members.pointers).unwrap();
                }
                if stage >= 4 {
                    cland_combat::visit_units(
                        memory,
                        0,
                        source,
                        &members,
                        &mut roles,
                        |_, _, _, _, _, _| {},
                    )
                    .unwrap();
                }
            },
        );
        assert!(matches!(walk.outcome, Outcome::Rows { .. }));
        w.calls.swap(0, Ordering::Relaxed) as u64
    };
    let counts: Vec<u64> = (0..=4).map(through).collect();
    let stage = |index: usize| counts[index] - counts[index - 1];
    vec![
        ("census", counts[0]),
        ("details and tactic context", stage(1)),
        ("unit visit", stage(4)),
        ("division images", stage(3)),
        ("division readers and commanders", tables[1] - stage(3)),
        ("modifier context", stage(2)),
        ("modifier members", tables[2] - stage(2)),
        ("country sets", tables[3]),
    ]
}

/// One walk's guarded calls by the object each call reads first: the
/// divisions' own armies and stat blocks, the combats and their sides,
/// module and game-state words, and every other object (countries,
/// provinces, states, templates, leaders, participant lists).
#[cfg(test)]
fn calls_by_object(
    built: &Built,
    memory: &impl Memory,
    captured: &mut Captured,
) -> Vec<(&'static str, usize)> {
    let w = &built.world;
    let armies: Vec<usize> = built.armies.iter().flatten().copied().collect();
    let mut stats: Vec<usize> = armies
        .iter()
        .map(|&army| w.get_u64(army + cunit::STAT_BLOCK) as usize)
        .collect();
    stats.sort_unstable();
    let mut sorted = armies.clone();
    sorted.sort_unstable();
    let mut sides: Vec<usize> = built.sides.iter().flatten().copied().collect();
    sides.sort_unstable();
    let mut combats = built.combats.clone();
    combats.sort_unstable();
    *w.call_log.lock().unwrap() = Some(Vec::new());
    captured.reset();
    captured.capture(memory, 0, GAME);
    let log = w.call_log.lock().unwrap().take().unwrap();
    let within = |objects: &[usize], size: usize, address: usize| {
        let at = objects.partition_point(|&start| start <= address);
        at > 0 && address < objects[at - 1] + size
    };
    let mut counts = [
        ("division army", 0),
        ("division stat block", 0),
        ("combat", 0),
        ("combat side", 0),
        ("module or game state", 0),
        ("other", 0),
    ];
    for address in log {
        let kind = if within(&sorted, 0x800, address) {
            0
        } else if within(&stats, 0x600, address) {
            1
        } else if within(&combats, 0x600, address) {
            2
        } else if within(&sides, 0x600, address) {
            3
        } else if address < 0x1_0000_0000 {
            4
        } else {
            5
        };
        counts[kind].1 += 1;
    }
    counts.to_vec()
}
