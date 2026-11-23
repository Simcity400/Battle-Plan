
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};
use crate::{cair_base, cair_wing, cstrategic_air, cstrategic_air_manager};
use engine_native_parts::cair_wing_pool;
use engine_native::{ccountry, cfleet};
use engine_sites::Rva;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirState {
    Unknown,
    Absent,
    Present,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryRow {
    pub country: u32,
    pub canonical_slot: u32,
    pub army_count: Result<i32, &'static str>,
    pub navy_count: Result<i32, &'static str>,
    pub land_count: Result<i32, &'static str>,
    pub fleet_start: usize,
    pub fleet_len: usize,
}

impl Default for CountryRow {
    fn default() -> Self {
        Self {
            country: 0,
            canonical_slot: 0,
            army_count: Err("game score country not read"),
            navy_count: Err("game score country not read"),
            land_count: Err("game score country not read"),
            fleet_start: 0,
            fleet_len: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FleetCount {
    pub fleet: u64,
    pub task_forces: Result<i32, &'static str>,
}

impl Default for FleetCount {
    fn default() -> Self {
        Self {
            fleet: 0,
            task_forces: Err("game score fleet not read"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryCapture {
    pub outcome: Outcome,
    pub country_len: usize,
    pub fleet_len: usize,
    pub fleet_required: usize,
    pub slot_zero_null: bool,
    pub slot_zero_refusal: Option<&'static str>,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    error: &'static str,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or(error)
}

fn vtable(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    rva: Rva,
    error: &'static str,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, rva).ok_or(error)?;
    if pointer == 0 || read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err(error);
    }
    Ok(())
}

pub fn walk_country_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<CountryRow>,
    fleets: &mut Vec<FleetCount>,
) -> CountryCapture {
    walk_country_inputs_buffer(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(fleets),
    )
}

fn walk_country_inputs_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, CountryRow>,
    mut fleets: engine_native_memory::walk::Buffer<'_, FleetCount>,
) -> CountryCapture {
    let mut country_len = 0_usize;
    let mut fleet_len = 0_usize;
    let mut fleet_required = 0_usize;
    let mut slot_zero_refusal = None;
    let mut slot_zero_null = false;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        output.ensure(countries.count() as usize)?;
        let mut unread_rows = 0_u32;
        for slot in 0..countries.count() {
            let country = if slot == 0 {
                match read_slot_zero(memory, base, countries.arrays()[0]) {
                    Ok(Some(country)) => country,
                    Ok(None) => {
                        slot_zero_null = true;
                        continue;
                    }
                    Err(reason) => {
                        slot_zero_refusal = Some(reason);
                        continue;
                    }
                }
            } else {
                let Some(country) = countries.at(slot)? else {
                    continue;
                };
                country
            };
            let Some(destination) = output.get_mut(country_len) else {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("game score country demand overflow")?;
                continue;
            };
            let army_count = field(
                memory,
                country.pointer,
                ccountry::DIVISIONS_COUNT,
                "game score army roster count unreadable",
            );
            let land_count = field(
                memory,
                country.pointer,
                ccountry::CONTROLLED_PROVINCES_COUNT,
                "game score controlled province count unreadable",
            );
            let start = fleet_len;
            let navy_count = (|| {
                let roster = country
                    .pointer
                    .checked_add(ccountry::FLEETS)
                    .and_then(|at| Array::read(memory, at, 8))
                    .ok_or("game score fleet roster unreadable or invalid")?;
                fleet_required = fleet_required
                    .checked_add(roster.count as usize)
                    .ok_or("game score fleet demand overflow")?;
                if !fleets.ensure(
                    fleet_len
                        .checked_add(roster.count as usize)
                        .ok_or("game score fleet demand overflow")?,
                )? {
                    return Err("game score fleet witness arena exhausted");
                }
                let mut sum = 0_i32;
                let mut failure = None;
                let mut first = 0;
                while first < roster.count {
                    let count = ((roster.count - first) as usize).min(FLEET_BATCH);
                    let rows = &mut fleets[fleet_len..fleet_len + count];
                    read_fleet_batch(memory, base, roster, first, rows);
                    for row in rows.iter() {
                        match row.task_forces {
                            Ok(count) => sum = sum.wrapping_add(count),
                            Err(error) => {
                                failure.get_or_insert(error);
                            }
                        }
                    }
                    fleet_len += count;
                    first += count as u32;
                }
                failure.map_or(Ok(sum), Err)
            })();
            *destination = CountryRow {
                country: country.id,
                canonical_slot: country.canonical_slot,
                army_count,
                navy_count,
                land_count,
                fleet_start: start,
                fleet_len: fleet_len - start,
            };
            country_len += 1;
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len: country_len }
        } else {
            Outcome::Truncated {
                len: country_len,
                unread_rows,
            }
        })
    })();
    CountryCapture {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        country_len,
        fleet_len,
        fleet_required,
        slot_zero_null,
        slot_zero_refusal,
    }
}

pub fn read_slot_zero(
    memory: &impl Memory,
    base: usize,
    countries: Array,
) -> Result<Option<ccountry::Country>, &'static str> {
    let pointer = countries
        .element(0, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("game score country slot zero pointer unreadable")?;
    if pointer == 0 {
        return Ok(None);
    }
    vtable(
        memory,
        base,
        pointer,
        ccountry::VTABLE_RVA,
        "game score country slot zero vtable mismatch",
    )?;
    let id = field(
        memory,
        pointer,
        ccountry::ID,
        "game score country slot zero identity unreadable",
    )?;
    Ok(Some(ccountry::Country {
        pointer,
        canonical_slot: 0,
        id,
    }))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArenaDemand {
    pub countries: usize,
    pub fleets: usize,
    pub sites: usize,
    pub wings: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ArenaSizing {
    pub demand: ArenaDemand,
    pub failures: Vec<(&'static str, u64)>,
}
impl ArenaSizing {
    pub fn unavailable(reason: &'static str) -> std::io::Result<Self> {
        let mut sizing = Self::default();
        sizing.refuse(reason)?;
        Ok(sizing)
    }
    fn refuse(&mut self, reason: &'static str) -> std::io::Result<()> {
        match self
            .failures
            .binary_search_by_key(&reason, |(reason, _)| *reason)
        {
            Ok(index) => self.failures[index].1 = self.failures[index].1.saturating_add(1),
            Err(index) => {
                engine_allocation::reserve(&mut self.failures, 1)?;
                self.failures.insert(index, (reason, 1));
            }
        }
        Ok(())
    }
}

fn sizing_array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    reason: &'static str,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or(reason)
}
fn add_demand(target: &mut usize, count: u32, reason: &'static str) -> Result<(), &'static str> {
    *target = target.checked_add(count as usize).ok_or(reason)?;
    Ok(())
}

pub fn arena_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> std::io::Result<ArenaSizing> {
    let mut sizing = ArenaSizing::default();
    let countries = match ccountry::Reader::new(memory, base, state) {
        Ok(countries) => Some(countries),
        Err(reason) => {
            sizing.refuse(reason)?;
            None
        }
    };
    if let Some(countries) = countries {
        sizing.demand.countries = countries.count() as usize;
        for slot in 0..countries.count() {
            let country = if slot == 0 {
                read_slot_zero(memory, base, countries.arrays()[0])
            } else {
                countries.at(slot)
            };
            let country = match country {
                Ok(Some(country)) => country,
                Ok(None) => continue,
                Err(reason) => {
                    sizing.refuse(reason)?;
                    continue;
                }
            };
            let result = sizing_array(
                memory,
                country.pointer,
                ccountry::FLEETS,
                "game score sizing fleet roster unreadable or invalid",
            )
            .and_then(|fleets| {
                add_demand(
                    &mut sizing.demand.fleets,
                    fleets.count,
                    "game score sizing fleet demand overflow",
                )
            });
            if let Err(reason) = result {
                sizing.refuse(reason)?;
            }
        }
    }
    let manager = field::<u64>(
        memory,
        state,
        engine_native::cgame_state::STRATEGIC_AIR_MANAGER,
        "game score sizing strategic-air manager unreadable",
    )
    .and_then(|manager| {
        usize::try_from(manager).map_err(|_| "game score sizing manager address overflow")
    });
    let manager = match manager {
        Ok(0) => return Ok(sizing),
        Ok(manager) => manager,
        Err(reason) => {
            sizing.refuse(reason)?;
            return Ok(sizing);
        }
    };
    if let Err(reason) = vtable(
        memory,
        base,
        manager,
        cstrategic_air_manager::VTABLE_RVA,
        "game score sizing strategic-air manager vtable mismatch",
    ) {
        sizing.refuse(reason)?;
        return Ok(sizing);
    }
    let owners = match sizing_array(
        memory,
        manager,
        cstrategic_air_manager::COUNTRY_AIR,
        "game score sizing air-country roster unreadable or invalid",
    ) {
        Ok(owners) => owners,
        Err(reason) => {
            sizing.refuse(reason)?;
            return Ok(sizing);
        }
    };
    for slot in 0..owners.count {
        let entry = owners
            .element(slot, 8)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok());
        let entry = match entry {
            Some(0) => continue,
            Some(entry) => entry,
            None => {
                sizing.refuse("game score sizing air-country pointer unreadable")?;
                continue;
            }
        };
        if let Err(reason) = vtable(
            memory,
            base,
            entry,
            cstrategic_air::VTABLE_RVA,
            "game score sizing air-country vtable mismatch",
        ) {
            sizing.refuse(reason)?;
            continue;
        }
        let result = sizing_array(
            memory,
            entry,
            cstrategic_air::FIRST_AIR_SITES,
            "game score sizing first air-site roster unreadable or invalid",
        )
        .and_then(|sites| {
            add_demand(
                &mut sizing.demand.sites,
                sites.count,
                "game score sizing first air-site demand overflow",
            )
        });
        if let Err(reason) = result {
            sizing.refuse(reason)?;
        }
        let pools = match sizing_array(
            memory,
            entry,
            cstrategic_air::POOLS,
            "game score sizing air-pool roster unreadable or invalid",
        ) {
            Ok(pools) => pools,
            Err(reason) => {
                sizing.refuse(reason)?;
                continue;
            }
        };
        for index in 0..pools.count {
            let pool = pools
                .element(index, 8)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|pointer| usize::try_from(pointer).ok());
            let pool = match pool {
                Some(pool) => pool,
                None => {
                    sizing.refuse("game score sizing air-pool pointer unreadable")?;
                    continue;
                }
            };
            if let Err(reason) = vtable(
                memory,
                base,
                pool,
                cair_wing_pool::VTABLE_RVA,
                "game score sizing air-pool vtable mismatch",
            ) {
                sizing.refuse(reason)?;
                continue;
            }
            let result = sizing_array(
                memory,
                pool,
                cair_wing_pool::WINGS,
                "game score sizing wing roster unreadable or invalid",
            )
            .and_then(|wings| {
                add_demand(
                    &mut sizing.demand.wings,
                    wings.count,
                    "game score sizing wing demand overflow",
                )
            });
            if let Err(reason) = result {
                sizing.refuse(reason)?;
            }
        }
    }
    Ok(sizing)
}

const FLEET_BATCH: usize = 64;
const FLEET_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 16,
    },
    RecordField {
        source_offset: cfleet::TASK_FORCE_COUNT,
        destination_offset: 16,
        len: 4,
    },
];
const FLEET_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FLEET_FIELDS, 20).expect("valid fleet layout");

fn read_fleet_batch(
    memory: &impl Memory,
    base: usize,
    roster: Array,
    first: u32,
    output: &mut [FleetCount],
) {
    let failed = |error| FleetCount {
        fleet: 0,
        task_forces: Err(error),
    };
    let count = output.len();
    let mut raw = [0_u8; FLEET_BATCH * 8];
    let pointers_read = roster
        .element(first, 8)
        .is_some_and(|at| engine_native_memory::memory::read_slice(memory, at, &mut raw[..count * 8]));
    if !pointers_read {
        for (index, row) in output.iter_mut().enumerate() {
            *row = roster
                .element(first + index as u32, 8)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("game score fleet pointer unreadable")
                .and_then(|pointer| read_fleet_count(memory, base, pointer))
                .unwrap_or_else(failed);
        }
        return;
    }
    let mut pointers = [0_usize; FLEET_BATCH];
    for (pointer, word) in pointers.iter_mut().zip(raw[..count * 8].chunks_exact(8)) {
        *pointer = u64::from_le_bytes(word.try_into().unwrap()) as usize;
    }
    let mut records = [0_u8; FLEET_BATCH * 20];
    let mut gathered = [false; FLEET_BATCH];
    let mut start = 0;
    while start < count {
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[start..count]),
            &FLEET_LAYOUT,
            &mut records[start * 20..count * 20],
        );
        let whole = copied / FLEET_FIELDS.len();
        gathered[start..start + whole].fill(true);
        start += whole + 1;
    }
    let expected = engine_sites::at(base, cfleet::VTABLE_RVA);
    for (index, row) in output.iter_mut().enumerate() {
        if !gathered[index] || pointers[index] == 0 {
            *row = read_fleet_count(memory, base, pointers[index]).unwrap_or_else(failed);
            continue;
        }
        let record = &records[index * 20..index * 20 + 20];
        if expected != Some(u64::from_le_bytes(record[..8].try_into().unwrap()) as usize) {
            *row = failed("game score fleet vtable mismatch");
            continue;
        }
        let fleet = u64::from_le_bytes(record[8..16].try_into().unwrap());
        if fleet == 0 {
            *row = failed("game score fleet handle is null");
            continue;
        }
        *row = FleetCount {
            fleet,
            task_forces: Ok(i32::from_le_bytes(record[16..20].try_into().unwrap())),
        };
    }
}

pub fn read_fleet_count(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<FleetCount, &'static str> {
    if pointer == 0 {
        return Err("game score fleet pointer is null");
    }
    let identity =
        read::<[u8; 16]>(memory, pointer).ok_or("game score fleet identity unreadable")?;
    if engine_sites::at(base, cfleet::VTABLE_RVA)
        != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()) as usize)
    {
        return Err("game score fleet vtable mismatch");
    }
    let fleet = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if fleet == 0 {
        return Err("game score fleet handle is null");
    }
    Ok(FleetCount {
        fleet,
        task_forces: field(
            memory,
            pointer,
            cfleet::TASK_FORCE_COUNT,
            "game score task force count unreadable",
        ),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirSite {
    pub base: u64,
    pub access: Result<bool, &'static str>,
}

impl Default for AirSite {
    fn default() -> Self {
        Self {
            base: 0,
            access: Err("game score air site not read"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirRead {
    pub state: AirState,
    pub error: &'static str,
    pub len: usize,
    pub required: usize,
}

pub fn read_first_sites_buffer(
    memory: &impl Memory,
    base: usize,
    canonical_slot: u32,
    entry: Option<usize>,
    mut output: engine_native_memory::walk::Buffer<'_, AirSite>,
) -> AirRead {
    let Some(entry) = entry else {
        return AirRead {
            state: AirState::Absent,
            error: "",
            len: 0,
            required: 0,
        };
    };
    let mut required = 0;
    let result = (|| {
        let sites = entry
            .checked_add(cstrategic_air::FIRST_AIR_SITES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("game score first air-site roster unreadable or invalid")?;
        required = sites.count as usize;
        output.ensure(required)?;
        if required > output.len() {
            return Err("game score first air-site arena exhausted");
        }
        let mut first = 0;
        while first < sites.count {
            let count = ((sites.count - first) as usize).min(SITE_BATCH);
            read_site_batch(
                memory,
                base,
                canonical_slot,
                sites,
                first,
                &mut output[first as usize..first as usize + count],
            );
            first += count as u32;
        }
        Ok(required)
    })();
    match result {
        Ok(len) => AirRead {
            state: AirState::Present,
            error: "",
            len,
            required,
        },
        Err(error) => AirRead {
            state: AirState::Unknown,
            error,
            len: 0,
            required,
        },
    }
}

const SITE_BATCH: usize = 64;
const SITE_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 16,
    },
    RecordField {
        source_offset: cair_base::COUNTRY_CONTAINERS,
        destination_offset: 16,
        len: 16,
    },
];
const SITE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SITE_FIELDS, 32).expect("valid air-site layout");
const ACCESS_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const ACCESS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ACCESS_FIELD, 8).expect("valid air-site access layout");

fn read_site_batch(
    memory: &impl Memory,
    base: usize,
    canonical_slot: u32,
    sites: Array,
    first: u32,
    output: &mut [AirSite],
) {
    let alone = |index: u32| {
        sites
            .element(index, 8)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("game score air-site pointer unreadable")
            .and_then(|pointer| read_air_site(memory, base, canonical_slot, pointer))
            .unwrap_or_else(|error| AirSite {
                base: 0,
                access: Err(error),
            })
    };
    let count = output.len();
    let mut raw = [0_u8; SITE_BATCH * 8];
    let pointers_read = sites
        .element(first, 8)
        .is_some_and(|at| engine_native_memory::memory::read_slice(memory, at, &mut raw[..count * 8]));
    if !pointers_read {
        for (index, site) in output.iter_mut().enumerate() {
            *site = alone(first + index as u32);
        }
        return;
    }
    let mut pointers = [0_usize; SITE_BATCH];
    for (pointer, word) in pointers.iter_mut().zip(raw[..count * 8].chunks_exact(8)) {
        *pointer = u64::from_le_bytes(word.try_into().unwrap()) as usize;
    }
    let mut records = [0_u8; SITE_BATCH * 32];
    let mut gathered = [false; SITE_BATCH];
    let mut start = 0;
    while start < count {
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[start..count]),
            &SITE_LAYOUT,
            &mut records[start * 32..count * 32],
        );
        let whole = copied / SITE_FIELDS.len();
        gathered[start..start + whole].fill(true);
        start += whole + 1;
    }
    let expected = engine_sites::at(base, cair_base::VTABLE_RVA);
    let mut access_rows = [0_usize; SITE_BATCH];
    let mut access_at = [0_usize; SITE_BATCH];
    let mut accesses = 0;
    for (index, site) in output.iter_mut().enumerate() {
        let pointer = pointers[index];
        if !gathered[index] || pointer == 0 {
            *site = read_air_site(memory, base, canonical_slot, pointer).unwrap_or_else(|error| {
                AirSite {
                    base: 0,
                    access: Err(error),
                }
            });
            continue;
        }
        let record = &records[index * 32..index * 32 + 32];
        let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
        if expected != Some(word(0) as usize) {
            *site = AirSite {
                base: 0,
                access: Err("game score air-site vtable mismatch"),
            };
            continue;
        }
        let handle = word(8);
        if handle == 0 {
            *site = AirSite {
                base: 0,
                access: Err("game score air-site handle is null"),
            };
            continue;
        }
        let bound = i32::from_le_bytes(record[28..32].try_into().unwrap());
        let access = (|| {
            let slot = i32::try_from(canonical_slot)
                .map_err(|_| "game score air-site country index invalid")?;
            if slot >= bound {
                return Ok(None);
            }
            let data = usize::try_from(word(16))
                .map_err(|_| "game score air-site container address overflow")?;
            if data == 0 {
                return Err("game score air-site container pointer is null");
            }
            data.checked_add(canonical_slot as usize * 8)
                .map(Some)
                .ok_or("game score air-site container address overflow")
        })();
        *site = AirSite {
            base: handle,
            access: match access {
                Ok(None) => Ok(false),
                Ok(Some(at)) => {
                    access_rows[accesses] = index;
                    access_at[accesses] = at;
                    accesses += 1;
                    Err("game score air-site access unreadable")
                }
                Err(error) => Err(error),
            },
        };
    }
    let mut words = [0_u8; SITE_BATCH * 8];
    let mut read_words = [false; SITE_BATCH];
    let mut start = 0;
    while start < accesses {
        let copied = memory.copy_records(
            RecordSources::Pointers(&access_at[start..accesses]),
            &ACCESS_LAYOUT,
            &mut words[start * 8..accesses * 8],
        );
        read_words[start..start + copied].fill(true);
        start += copied + 1;
    }
    for (entry, &index) in access_rows[..accesses].iter().enumerate() {
        let value = if read_words[entry] {
            Some(u64::from_le_bytes(
                words[entry * 8..entry * 8 + 8].try_into().unwrap(),
            ))
        } else {
            read::<u64>(memory, access_at[entry])
        };
        output[index].access = value
            .map(|value| value != 0)
            .ok_or("game score air-site access unreadable");
    }
}

pub fn read_air_site(
    memory: &impl Memory,
    base: usize,
    canonical_slot: u32,
    pointer: usize,
) -> Result<AirSite, &'static str> {
    if pointer == 0 {
        return Err("game score air-site pointer is null");
    }
    let identity =
        read::<[u8; 16]>(memory, pointer).ok_or("game score air-site identity unreadable")?;
    if engine_sites::at(base, cair_base::VTABLE_RVA)
        != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()) as usize)
    {
        return Err("game score air-site vtable mismatch");
    }
    let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if handle == 0 {
        return Err("game score air-site handle is null");
    }
    let access = (|| {
        let count = field::<i32>(
            memory,
            pointer,
            cair_base::COUNTRY_CONTAINERS_COUNT,
            "game score air-site access bounds unreadable",
        )?;
        let slot = i32::try_from(canonical_slot)
            .map_err(|_| "game score air-site country index invalid")?;
        if slot >= count {
            return Ok(false);
        }
        let data = field::<u64>(
            memory,
            pointer,
            cair_base::COUNTRY_CONTAINERS,
            "game score air-site container pointer unreadable",
        )?;
        let data =
            usize::try_from(data).map_err(|_| "game score air-site container address overflow")?;
        if data == 0 {
            return Err("game score air-site container pointer is null");
        }
        let at = data
            .checked_add(canonical_slot as usize * 8)
            .ok_or("game score air-site container address overflow")?;
        Ok(read::<u64>(memory, at).ok_or("game score air-site access unreadable")? != 0)
    })();
    Ok(AirSite {
        base: handle,
        access,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WingRow {
    pub id: u64,
    pub planes: Result<i32, &'static str>,
    pub stat_raw: Result<i64, &'static str>,
    pub mission_index: Result<i32, &'static str>,
}

impl Default for WingRow {
    fn default() -> Self {
        Self {
            id: 0,
            planes: Err("game score wing not read"),
            stat_raw: Err("game score wing not read"),
            mission_index: Err("game score wing not read"),
        }
    }
}

pub fn read_wing(memory: &impl Memory, pointer: usize, row: &cair_wing::ScoreLeaf) -> WingRow {
    let index = cair_wing::mission_stat_index(memory, pointer, 1);
    let stat_raw = index
        .and_then(|index| cair_wing::mission_stat_record(memory, pointer, index))
        .map_err(cair_wing::MissionStatError::reason)
        .and_then(|record| {
            field(
                memory,
                record,
                0x33 * 8,
                "game score air superiority stat unreadable",
            )
        });
    WingRow {
        id: row.id,
        planes: row.planes,
        stat_raw,
        mission_index: index
            .map(i32::from)
            .map_err(cair_wing::MissionStatError::reason),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Multipliers {
    pub total_raw: Result<i64, &'static str>,
    pub army_raw: Result<i64, &'static str>,
    pub navy_raw: Result<i64, &'static str>,
    pub air_raw: Result<i64, &'static str>,
    pub industry_raw: Result<i64, &'static str>,
    pub land_raw: Result<i64, &'static str>,
}

pub const COUNTRY_SCORE_MULTIPLIER_RVA: Rva = Rva(0x033e_1f00);
pub const ARMY_SCORE_MULTIPLIER_RVA: Rva = Rva(0x033e_1f98);
pub const NAVY_SCORE_MULTIPLIER_RVA: Rva = Rva(0x033e_2038);
pub const AIR_SCORE_MULTIPLIER_RVA: Rva = Rva(0x033e_20c8);
pub const INDUSTRY_SCORE_MULTIPLIER_RVA: Rva = Rva(0x033e_2158);
pub const PROVINCE_SCORE_MULTIPLIER_RVA: Rva = Rva(0x033e_21f8);

fn global(
    memory: &impl Memory,
    base: usize,
    key: Rva,
    error: &'static str,
) -> Result<i64, &'static str> {
    engine_sites::at(base, key)
        .and_then(|at| read(memory, at))
        .ok_or(error)
}

pub fn read_multipliers(memory: &impl Memory, base: usize) -> Multipliers {
    Multipliers {
        total_raw: global(
            memory,
            base,
            COUNTRY_SCORE_MULTIPLIER_RVA,
            "NDefines.NCountry.COUNTRY_SCORE_MULTIPLIER unreadable",
        ),
        army_raw: global(
            memory,
            base,
            ARMY_SCORE_MULTIPLIER_RVA,
            "NDefines.NCountry.ARMY_SCORE_MULTIPLIER unreadable",
        ),
        navy_raw: global(
            memory,
            base,
            NAVY_SCORE_MULTIPLIER_RVA,
            "NDefines.NCountry.NAVY_SCORE_MULTIPLIER unreadable",
        ),
        air_raw: global(
            memory,
            base,
            AIR_SCORE_MULTIPLIER_RVA,
            "NDefines.NCountry.AIR_SCORE_MULTIPLIER unreadable",
        ),
        industry_raw: global(
            memory,
            base,
            INDUSTRY_SCORE_MULTIPLIER_RVA,
            "NDefines.NCountry.INDUSTRY_SCORE_MULTIPLIER unreadable",
        ),
        land_raw: global(
            memory,
            base,
            PROVINCE_SCORE_MULTIPLIER_RVA,
            "NDefines.NCountry.PROVINCE_SCORE_MULTIPLIER unreadable",
        ),
    }
}

#[cfg(test)]
mod tests;
