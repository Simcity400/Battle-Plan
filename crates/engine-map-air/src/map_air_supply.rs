use engine_native::allocation as engine_allocation;
use engine_native::{
    Memory, cair_base, ccountry, cdiplomacy_status,
    cpdx_array::Array,
    cprovince,
    creference_registry::ScopedResolver,
    cstrategic_region, csupply_system,
    memory::{Raw, RecordLayout, RecordSources, packed_fields},
    read,
};
use std::collections::HashMap;

const CONSUMERS: usize = 0x1b8;
const CONSUMER_STRIDE: usize = 16;
const PROVINCE: usize = 0x18;
const KIND: usize = 0x1c;
const ASKED: usize = 0x28;
const RATIO: usize = 0x30;
const AIR_DROPS: usize = 0x48;
const ORIGINS: usize = 0x50;
const ORIGIN_STRIDE: usize = 0x30;
const ORIGIN_SUPPLY: usize = 0x18;
const FROM_STATE: usize = 0x68;
const AIR_BASE: usize = 0xa8;
const AIR_COUNTRY: usize = 0xb0;
const DIVISION_COUNTRY: usize = 0x1d0;
const UNIT_COUNTRY: usize = 0x1c8;
const OTHER_COUNTRY: usize = 0xa8;
const ACCESS_COUNTRIES: usize = 0x1410;
const OFFERED_ACCESS: usize = 0x2c8;
const VERY_LOW_THRESHOLD: usize = 0x033e_34c0;
const LOW_THRESHOLD: usize = 0x033e_35c0;
const BATCH: usize = 64;
const CONSUMER_FIELDS: [engine_native::memory::RecordField; 3] =
    packed_fields([(PROVINCE, 4), (ASKED, 8), (FROM_STATE, 8)]);
const CONSUMER_BYTES: usize = 20;
const CONSUMER_LAYOUT: RecordLayout<'static> =
    match RecordLayout::new(&CONSUMER_FIELDS, CONSUMER_BYTES) {
        Some(layout) => layout,
        None => panic!("valid regional Air supply consumer layout"),
    };
const TOTAL_FIELDS: [engine_native::memory::RecordField; 2] =
    packed_fields([(RATIO, 8), (AIR_DROPS, 8)]);
const TOTAL_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&TOTAL_FIELDS, 16) {
    Some(layout) => layout,
    None => panic!("valid regional Air supply total layout"),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub country_pointer: usize,
    pub region_pointer: usize,
    pub country: u32,
    pub region: u32,
    pub severity: i32,
    pub needed: i64,
    pub supplied: i64,
    pub deficit: i64,
}

impl Summary {
    fn empty(country: u32, region: u32) -> Self {
        Self {
            country_pointer: 0,
            region_pointer: 0,
            country,
            region,
            severity: 2,
            needed: 0,
            supplied: 0,
            deficit: 0,
        }
    }
}

#[derive(Default)]
pub struct Capture {
    indices: HashMap<(u32, u32), usize>,
    pub rows: Vec<Summary>,
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("regional Air supply operand unreadable")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("regional Air supply array invalid")
}

fn origin_sum(memory: &impl Memory, consumer: usize) -> Result<i64, &'static str> {
    const FIELDS: [engine_native::memory::RecordField; 1] = packed_fields([(ORIGIN_SUPPLY, 8)]);
    const LAYOUT: RecordLayout<'static> = match RecordLayout::new(&FIELDS, 8) {
        Some(layout) => layout,
        None => panic!("valid supply origin layout"),
    };
    let origins = array(memory, consumer, ORIGINS, ORIGIN_STRIDE)?;
    let mut bytes = [0u8; BATCH * 8];
    let mut total = 0i64;
    let mut index = 0;
    while index < origins.count {
        let count = (origins.count - index).min(BATCH as u32) as usize;
        let pointer = origins
            .element(index, ORIGIN_STRIDE)
            .ok_or("regional Air supply origin extent invalid")?;
        if memory.copy_records(
            RecordSources::Strided {
                base: pointer,
                stride: ORIGIN_STRIDE,
                count,
            },
            &LAYOUT,
            &mut bytes[..count * 8],
        ) != count
        {
            return Err("regional Air supply origins unreadable");
        }
        for bytes in bytes[..count * 8].chunks_exact(8) {
            total = total.wrapping_add(i64::from_le_bytes(bytes.try_into().unwrap()));
        }
        index += count as u32;
    }
    if array(memory, consumer, ORIGINS, ORIGIN_STRIDE)? != origins {
        return Err("regional Air supply origins changed");
    }
    Ok(total)
}

fn owner<M: Memory>(
    memory: &M,
    base: usize,
    game: usize,
    countries: &ccountry::Reader<'_, M>,
    resolver: &mut ScopedResolver,
    consumer: usize,
) -> Result<u32, &'static str> {
    match field::<u8>(memory, consumer, KIND)? {
        0 => field(memory, consumer, DIVISION_COUNTRY),
        1 | 3 => field(memory, consumer, UNIT_COUNTRY),
        4 => field(memory, consumer, OTHER_COUNTRY),
        2 => {
            let country = field::<u32>(memory, consumer, AIR_COUNTRY)?;
            let handle = field::<u64>(memory, consumer, AIR_BASE)?;
            let Some(air_base) = resolver.resolve(memory, handle)? else {
                return Ok(country);
            };
            let host = field::<u64>(memory, air_base, cair_base::HOST_REFERENCE)?;
            if resolver.resolve(memory, host)?.is_some() {
                return Ok(country);
            }
            let mut province = field::<u64>(memory, air_base, cair_base::PROVINCE)? as usize;
            if province == 0 {
                let reference =
                    crate::map_air_base_reference::read_base(memory, base, game, handle)?
                        .ok_or("regional Air supply base reference changed")?;
                let id = reference
                    .province
                    .ok_or("regional Air supply land base location unavailable")?;
                province = array(memory, game, engine_native::cgame_state::PROVINCES, 8)?
                    .element(id, 8)
                    .and_then(|address| read::<u64>(memory, address))
                    .ok_or("regional Air supply base province unreadable")?
                    as usize;
            }
            let controller = field::<u32>(memory, province, cprovince::CONTROLLER)?;
            if (controller as i32) <= 0 {
                return Ok(country);
            }
            if ccountry::movement_friendly(memory, base, game, country, controller)? {
                return Ok(country);
            }
            let holder = countries.resolve(controller)?;
            let target = countries.resolve(country)?;
            let access = array(memory, holder.pointer, ACCESS_COUNTRIES, 4)?;
            for index in 0..access.count {
                let raw = access
                    .element(index, 4)
                    .and_then(|address| read::<u32>(memory, address))
                    .ok_or("regional Air supply access country unreadable")?;
                if raw == country
                    || (raw != 0 && countries.resolve(raw)?.canonical_slot == target.canonical_slot)
                {
                    return Ok(controller);
                }
            }
            let diplomacy =
                field::<u64>(memory, holder.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
            let relations = array(memory, diplomacy, cdiplomacy_status::RELATIONS, 8)?;
            let status = relations
                .element(target.canonical_slot, 8)
                .and_then(|address| read::<u64>(memory, address))
                .ok_or("regional Air supply relation unreadable")?
                as usize;
            if field::<u64>(memory, status, OFFERED_ACCESS)? != 0 {
                Ok(controller)
            } else {
                Ok(country)
            }
        }
        _ => Ok(0),
    }
}

impl Capture {
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<Summary>()
            + self.indices.capacity() * std::mem::size_of::<((u32, u32), usize)>()
    }

    pub fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
    ) -> Result<(), &'static str> {
        self.rows.clear();
        self.indices.clear();
        let result = self.read_inner(memory, base, game);
        if result.is_err() {
            self.rows.clear();
            self.indices.clear();
        }
        result
    }

    fn read_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
    ) -> Result<(), &'static str> {
        let systems = csupply_system::Reader::new(memory, base, game)?;
        let countries = ccountry::Reader::new(memory, base, game)?;
        let mut resolver = ScopedResolver::new(base);
        let provinces = array(memory, game, engine_native::cgame_state::PROVINCES, 8)?;
        let thresholds = [
            field::<i64>(memory, base, VERY_LOW_THRESHOLD)?,
            field::<i64>(memory, base, LOW_THRESHOLD)?,
        ];
        let mut handles = [0u8; BATCH * CONSUMER_STRIDE];
        let mut pointers = [0usize; BATCH];
        let mut consumers_bytes = [0u8; BATCH * CONSUMER_BYTES];
        let mut total_pointers = [0usize; BATCH];
        let mut total_rows = [(0usize, 0i64, 0usize); BATCH];
        let mut totals_bytes = [0u8; BATCH * 16];
        for slot in 1..systems.count() {
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            let start = self.rows.len();
            let acquired = (|| -> Result<(), &'static str> {
                engine_allocation::reserve(&mut self.rows, 1)
                    .map_err(|_| "regional Air supply country allocation failed")?;
                self.rows.push(Summary::empty(slot, u32::MAX));
                let consumers = array(memory, system.companion, CONSUMERS, CONSUMER_STRIDE)?;
                let mut index = 0;
                while index < consumers.count {
                    let count = (consumers.count - index).min(BATCH as u32) as usize;
                    let source = consumers
                        .element(index, CONSUMER_STRIDE)
                        .ok_or("regional Air supply membership extent invalid")?;
                    if !memory.copy(source, &mut handles[..count * CONSUMER_STRIDE]) {
                        return Err("regional Air supply membership unreadable");
                    }
                    let mut resolved = 0;
                    for record in handles[..count * CONSUMER_STRIDE].chunks_exact(CONSUMER_STRIDE) {
                        let handle = u64::from_le_bytes(record[..8].try_into().unwrap());
                        let Some(consumer) = resolver.resolve(memory, handle)? else {
                            continue;
                        };
                        pointers[resolved] = consumer;
                        resolved += 1;
                    }
                    if memory.copy_records(
                        RecordSources::Pointers(&pointers[..resolved]),
                        &CONSUMER_LAYOUT,
                        &mut consumers_bytes[..resolved * CONSUMER_BYTES],
                    ) != resolved * CONSUMER_FIELDS.len()
                    {
                        return Err("regional Air supply consumers unreadable");
                    }
                    let mut admitted = 0;
                    for (&consumer, bytes) in pointers[..resolved].iter().zip(
                        consumers_bytes[..resolved * CONSUMER_BYTES].chunks_exact(CONSUMER_BYTES),
                    ) {
                        let asked = i64::from_le_bytes(bytes[4..12].try_into().unwrap());
                        let from_state = i64::from_le_bytes(bytes[12..20].try_into().unwrap());
                        let needed = asked
                            .wrapping_sub(from_state)
                            .wrapping_sub(origin_sum(memory, consumer)?);
                        let province = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                        if needed == 0 || province == 0 {
                            continue;
                        }
                        let province = provinces
                            .element(province, 8)
                            .and_then(|address| read::<u64>(memory, address))
                            .ok_or("regional Air supply province unreadable")?
                            as usize;
                        let region =
                            field::<u64>(memory, province, cprovince::STRATEGIC_REGION)? as usize;
                        let region_pointer = region;
                        let Some(region) = cstrategic_region::id(memory, base, region)? else {
                            continue;
                        };
                        let owner = owner(memory, base, game, &countries, &mut resolver, consumer)?;
                        if owner != system.country.id
                            && (owner == 0
                                || countries.resolve(owner)?.canonical_slot
                                    != system.country.canonical_slot)
                        {
                            continue;
                        }
                        let key = (slot, region);
                        let at = if let Some(&at) = self.indices.get(&key) {
                            at
                        } else {
                            engine_allocation::reserve(&mut self.rows, 1)
                                .map_err(|_| "regional Air supply row allocation failed")?;
                            engine_allocation::reserve_map(&mut self.indices, 1)
                                .map_err(|_| "regional Air supply index allocation failed")?;
                            let at = self.rows.len();
                            self.rows.push(Summary::empty(slot, region));
                            self.indices.insert(key, at);
                            at
                        };
                        total_pointers[admitted] = consumer;
                        total_rows[admitted] = (at, needed, region_pointer);
                        admitted += 1;
                    }
                    if memory.copy_records(
                        RecordSources::Pointers(&total_pointers[..admitted]),
                        &TOTAL_LAYOUT,
                        &mut totals_bytes[..admitted * 16],
                    ) != admitted * TOTAL_FIELDS.len()
                    {
                        return Err("regional Air supply totals unreadable");
                    }
                    for (&(at, needed, region_pointer), bytes) in total_rows[..admitted]
                        .iter()
                        .zip(totals_bytes[..admitted * 16].chunks_exact(16))
                    {
                        let ratio = i64::from_le_bytes(bytes[..8].try_into().unwrap());
                        let supplied = i64::from_le_bytes(bytes[8..].try_into().unwrap());
                        let row = &mut self.rows[at];
                        row.country_pointer = system.country.pointer;
                        row.region_pointer = region_pointer;
                        row.needed = row.needed.wrapping_add(needed);
                        row.supplied = row.supplied.wrapping_add(supplied);
                        if ratio < 100_000 {
                            row.deficit = row.deficit.wrapping_add(needed.wrapping_sub(supplied));
                        }
                        let severity = if ratio < thresholds[0] {
                            0
                        } else if ratio < thresholds[1] {
                            1
                        } else {
                            2
                        };
                        row.severity = row.severity.min(severity);
                    }
                    index += count as u32;
                }
                if array(memory, system.companion, CONSUMERS, CONSUMER_STRIDE)? != consumers {
                    return Err("regional Air supply membership changed");
                }
                Ok(())
            })();
            if acquired.is_err() {
                self.rows.truncate(start);
                self.indices.retain(|(country, _), _| *country != slot);
            }
        }
        Ok(())
    }
}
