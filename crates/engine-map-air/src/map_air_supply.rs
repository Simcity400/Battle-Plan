
use engine_native::allocation as engine_allocation;
use engine_native::{
    Memory,
    ccountry,
    cpdx_array::Array,
    cprovince,
    creference_registry::{Header, ScopedResolver},
    cstrategic_region,
    csupply_system,
    memory::{Raw, RecordField, RecordLayout, RecordSources, packed_fields},
    read,
};
use engine_native_politics::cdiplomacy_status;
use engine_native_air::cair_base;
use engine_sites::Rva;

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
const VERY_LOW_THRESHOLD: Rva = Rva(0x033e_34c0);
const LOW_THRESHOLD: Rva = Rva(0x033e_35c0);
const BATCH: usize = 64;

const SPAN_START: usize = PROVINCE;
const SPAN: usize = FROM_STATE + 8 - SPAN_START;
const SPAN_FIELDS: [RecordField; 1] = packed_fields([(SPAN_START, SPAN)]);
const SPAN_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&SPAN_FIELDS, SPAN) {
    Some(layout) => layout,
    None => panic!("valid regional Air supply consumer layout"),
};
const U64_FIELDS: [RecordField; 1] = packed_fields([(0, 8)]);
const U64_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&U64_FIELDS, 8) {
    Some(layout) => layout,
    None => panic!("valid regional Air supply word layout"),
};
const U32_FIELDS: [RecordField; 1] = packed_fields([(0, 4)]);
const U32_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&U32_FIELDS, 4) {
    Some(layout) => layout,
    None => panic!("valid regional Air supply owner layout"),
};
const BUCKET: usize = 24;
const BUCKET_FIELDS: [RecordField; 1] = packed_fields([(0, BUCKET)]);
const BUCKET_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&BUCKET_FIELDS, BUCKET) {
    Some(layout) => layout,
    None => panic!("valid regional Air supply registry bucket layout"),
};
const REGION_FIELDS: [RecordField; 1] = packed_fields([(cprovince::STRATEGIC_REGION, 8)]);
const REGION_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&REGION_FIELDS, 8) {
    Some(layout) => layout,
    None => panic!("valid regional Air supply region layout"),
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
struct Scratch {
    regions: Vec<(u32, usize)>,
    origins: Vec<usize>,
    origin_bytes: Vec<u8>,
    region_ids: Vec<(usize, Result<Option<u32>, &'static str>)>,
}

#[derive(Default)]
pub struct Capture {
    pub rows: Vec<Summary>,
    scratch: Scratch,
}

#[derive(Clone, Copy, Default)]
struct Consumer {
    pointer: usize,
    province: u32,
    kind: u8,
    needed: i64,
    ratio: i64,
    supplied: i64,
    region: u32,
    region_pointer: usize,
    owner: u32,
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("regional Air supply operand unreadable")
}

fn global<T: Raw>(memory: &impl Memory, base: usize, key: Rva) -> Result<T, &'static str> {
    engine_sites::at(base, key)
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

fn span<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    bytes[offset - SPAN_START..offset - SPAN_START + N]
        .try_into()
        .unwrap()
}

fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(span(bytes, offset))
}

fn gather(
    memory: &impl Memory,
    sources: &[usize],
    layout: &RecordLayout<'_>,
    bytes: &mut [u8],
    reason: &'static str,
) -> Result<(), &'static str> {
    let fields = layout.field_count();
    if memory.copy_records(
        RecordSources::Pointers(sources),
        layout,
        &mut bytes[..sources.len() * layout.stride()],
    ) != sources.len() * fields
    {
        return Err(reason);
    }
    Ok(())
}

fn air_base_owner<M: Memory>(
    memory: &M,
    base: usize,
    game: usize,
    countries: &ccountry::Reader<'_, M>,
    resolver: &mut ScopedResolver,
    consumer: usize,
) -> Result<u32, &'static str> {
    const _: () = assert!(AIR_COUNTRY == AIR_BASE + 8);
    let bytes = field::<[u8; 12]>(memory, consumer, AIR_BASE)?;
    let handle = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let country = u32::from_le_bytes(bytes[8..].try_into().unwrap());
    let Some(air_base) = resolver.resolve(memory, handle)? else {
        return Ok(country);
    };
    let host = field::<u64>(memory, air_base, cair_base::HOST_REFERENCE)?;
    if resolver.resolve(memory, host)?.is_some() {
        return Ok(country);
    }
    let mut province = field::<u64>(memory, air_base, cair_base::PROVINCE)? as usize;
    if province == 0 {
        let reference = crate::map_air_base_reference::read_base(memory, base, game, handle)?
            .ok_or("regional Air supply base reference changed")?;
        let id = reference
            .province
            .ok_or("regional Air supply land base location unavailable")?;
        province = array(memory, game, engine_native::cgame_state::PROVINCES, 8)?
            .element(id, 8)
            .and_then(|address| read::<u64>(memory, address))
            .ok_or("regional Air supply base province unreadable")? as usize;
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
    let diplomacy = field::<u64>(memory, holder.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
    let relations = array(memory, diplomacy, cdiplomacy_status::RELATIONS, 8)?;
    let status = relations
        .element(target.canonical_slot, 8)
        .and_then(|address| read::<u64>(memory, address))
        .ok_or("regional Air supply relation unreadable")? as usize;
    if field::<u64>(memory, status, OFFERED_ACCESS)? != 0 {
        Ok(controller)
    } else {
        Ok(country)
    }
}

struct Context<'a, 'm, M: Memory> {
    memory: &'m M,
    base: usize,
    game: usize,
    countries: &'a ccountry::Reader<'m, M>,
    provinces: Array,
    thresholds: [i64; 2],
}

impl Scratch {
    fn region_id(
        &mut self,
        memory: &impl Memory,
        base: usize,
        region: usize,
    ) -> Result<Option<u32>, &'static str> {
        match self
            .region_ids
            .binary_search_by_key(&region, |&(pointer, _)| pointer)
        {
            Ok(at) => self.region_ids[at].1,
            Err(at) => {
                let id = cstrategic_region::id(memory, base, region);
                engine_allocation::reserve(&mut self.region_ids, 1)
                    .map_err(|_| "regional Air supply region allocation failed")?;
                self.region_ids.insert(at, (region, id));
                id
            }
        }
    }

    fn country<M: Memory>(
        &mut self,
        context: &Context<'_, '_, M>,
        resolver: &mut ScopedResolver,
        slot: u32,
        system: &csupply_system::CountrySystem,
        rows: &mut Vec<Summary>,
    ) -> Result<(), &'static str> {
        let memory = context.memory;
        let start = rows.len();
        self.regions.clear();
        let consumers = array(memory, system.companion, CONSUMERS, CONSUMER_STRIDE)?;
        let mut handles = [0u8; BATCH * CONSUMER_STRIDE];
        let mut pointers = [0usize; BATCH];
        let mut index = 0;
        while index < consumers.count {
            let count = (consumers.count - index).min(BATCH as u32) as usize;
            let source = consumers
                .element(index, CONSUMER_STRIDE)
                .ok_or("regional Air supply membership extent invalid")?;
            if !memory.copy(source, &mut handles[..count * CONSUMER_STRIDE]) {
                return Err("regional Air supply membership unreadable");
            }
            let mut probes = [(0u64, None::<Header>); BATCH];
            let mut cells = [0usize; BATCH];
            let mut probed = 0;
            for record in handles[..count * CONSUMER_STRIDE].chunks_exact(CONSUMER_STRIDE) {
                let handle = u64::from_le_bytes(record[..8].try_into().unwrap());
                if let Some((header, cell)) = resolver.first_cell(memory, handle)? {
                    probes[probed] = (handle, Some(header));
                    cells[probed] = cell;
                    probed += 1;
                }
            }
            let mut buckets = [0u8; BATCH * BUCKET];
            gather(
                memory,
                &cells[..probed],
                &BUCKET_LAYOUT,
                &mut buckets,
                "reference bucket unreadable",
            )?;
            let mut resolved = 0;
            for (&(handle, header), first) in
                probes[..probed].iter().zip(buckets.chunks_exact(BUCKET))
            {
                let header = header.ok_or("reference header unread")?;
                if let Some(consumer) =
                    resolver.resolve_gathered(memory, header, handle, first.try_into().unwrap())?
                {
                    pointers[resolved] = consumer;
                    resolved += 1;
                }
            }
            self.batch(context, resolver, slot, system, &pointers[..resolved], rows)?;
            index += count as u32;
        }
        if array(memory, system.companion, CONSUMERS, CONSUMER_STRIDE)? != consumers {
            return Err("regional Air supply membership changed");
        }
        rows[start..].sort_unstable_by_key(|row| row.region);
        engine_allocation::reserve(rows, 1)
            .map_err(|_| "regional Air supply country allocation failed")?;
        rows.push(Summary::empty(slot, u32::MAX));
        Ok(())
    }

    fn batch<M: Memory>(
        &mut self,
        context: &Context<'_, '_, M>,
        resolver: &mut ScopedResolver,
        slot: u32,
        system: &csupply_system::CountrySystem,
        pointers: &[usize],
        rows: &mut Vec<Summary>,
    ) -> Result<(), &'static str> {
        let memory = context.memory;
        if pointers.is_empty() {
            return Ok(());
        }
        let mut spans = [0u8; BATCH * SPAN];
        gather(
            memory,
            pointers,
            &SPAN_LAYOUT,
            &mut spans,
            "regional Air supply consumers unreadable",
        )?;
        let mut consumers = [Consumer::default(); BATCH];
        let mut origin_counts = [0u32; BATCH];
        self.origins.clear();
        for (at, (&pointer, bytes)) in pointers.iter().zip(spans.chunks_exact(SPAN)).enumerate() {
            let origins = Array::from_header(span(bytes, ORIGINS), ORIGIN_STRIDE)
                .ok_or("regional Air supply array invalid")?;
            engine_allocation::reserve(&mut self.origins, origins.count as usize)
                .map_err(|_| "regional Air supply origin allocation failed")?;
            for origin in 0..origins.count {
                self.origins.push(
                    origins
                        .element(origin, ORIGIN_STRIDE)
                        .and_then(|entry| entry.checked_add(ORIGIN_SUPPLY))
                        .ok_or("regional Air supply origin extent invalid")?,
                );
            }
            origin_counts[at] = origins.count;
            consumers[at] = Consumer {
                pointer,
                province: u32::from_le_bytes(span(bytes, PROVINCE)),
                kind: span::<1>(bytes, KIND)[0],
                needed: i64_at(bytes, ASKED).wrapping_sub(i64_at(bytes, FROM_STATE)),
                ratio: i64_at(bytes, RATIO),
                supplied: i64_at(bytes, AIR_DROPS),
                ..Consumer::default()
            };
        }
        if !self.origins.is_empty() {
            self.origin_bytes.clear();
            engine_allocation::reserve(&mut self.origin_bytes, self.origins.len() * 8)
                .map_err(|_| "regional Air supply origin allocation failed")?;
            self.origin_bytes.resize(self.origins.len() * 8, 0);
            gather(
                memory,
                &self.origins,
                &U64_LAYOUT,
                &mut self.origin_bytes,
                "regional Air supply origins unreadable",
            )?;
            let mut values = self.origin_bytes.chunks_exact(8);
            for (consumer, &count) in consumers.iter_mut().zip(&origin_counts[..pointers.len()]) {
                for value in values.by_ref().take(count as usize) {
                    consumer.needed = consumer
                        .needed
                        .wrapping_sub(i64::from_le_bytes(value.try_into().unwrap()));
                }
            }
        }
        let mut candidates = [Consumer::default(); BATCH];
        let mut addresses = [0usize; BATCH];
        let mut count = 0;
        for consumer in &consumers[..pointers.len()] {
            if consumer.needed == 0 || consumer.province == 0 {
                continue;
            }
            addresses[count] = context
                .provinces
                .element(consumer.province, 8)
                .ok_or("regional Air supply province unreadable")?;
            candidates[count] = *consumer;
            count += 1;
        }
        if count == 0 {
            return Ok(());
        }
        let mut words = [0u8; BATCH * 8];
        gather(
            memory,
            &addresses[..count],
            &U64_LAYOUT,
            &mut words,
            "regional Air supply province unreadable",
        )?;
        for (address, word) in addresses[..count].iter_mut().zip(words.chunks_exact(8)) {
            *address = u64::from_le_bytes(word.try_into().unwrap()) as usize;
        }
        gather(
            memory,
            &addresses[..count],
            &REGION_LAYOUT,
            &mut words,
            "regional Air supply operand unreadable",
        )?;
        let mut kept = 0;
        for at in 0..count {
            let region = u64::from_le_bytes(words[at * 8..at * 8 + 8].try_into().unwrap()) as usize;
            let Some(id) = self.region_id(memory, context.base, region)? else {
                continue;
            };
            candidates[kept] = Consumer {
                region: id,
                region_pointer: region,
                ..candidates[at]
            };
            kept += 1;
        }
        let mut plain = [0usize; BATCH];
        let mut owners = [0usize; BATCH];
        let mut plains = 0;
        for (at, candidate) in candidates[..kept].iter_mut().enumerate() {
            let offset = match candidate.kind {
                0 => DIVISION_COUNTRY,
                1 | 3 => UNIT_COUNTRY,
                4 => OTHER_COUNTRY,
                2 => {
                    candidate.owner = air_base_owner(
                        memory,
                        context.base,
                        context.game,
                        context.countries,
                        resolver,
                        candidate.pointer,
                    )?;
                    continue;
                }
                _ => continue,
            };
            owners[plains] = candidate
                .pointer
                .checked_add(offset)
                .ok_or("regional Air supply operand unreadable")?;
            plain[plains] = at;
            plains += 1;
        }
        if plains != 0 {
            let mut bytes = [0u8; BATCH * 4];
            gather(
                memory,
                &owners[..plains],
                &U32_LAYOUT,
                &mut bytes,
                "regional Air supply operand unreadable",
            )?;
            for (&at, owner) in plain[..plains].iter().zip(bytes.chunks_exact(4)) {
                candidates[at].owner = u32::from_le_bytes(owner.try_into().unwrap());
            }
        }
        for candidate in &candidates[..kept] {
            let owner = candidate.owner;
            if owner != system.country.id
                && (owner == 0
                    || context.countries.resolve(owner)?.canonical_slot
                        != system.country.canonical_slot)
            {
                continue;
            }
            let at = match self
                .regions
                .binary_search_by_key(&candidate.region, |&(region, _)| region)
            {
                Ok(found) => self.regions[found].1,
                Err(insert) => {
                    engine_allocation::reserve(rows, 1)
                        .map_err(|_| "regional Air supply row allocation failed")?;
                    engine_allocation::reserve(&mut self.regions, 1)
                        .map_err(|_| "regional Air supply index allocation failed")?;
                    let at = rows.len();
                    rows.push(Summary::empty(slot, candidate.region));
                    self.regions.insert(insert, (candidate.region, at));
                    at
                }
            };
            let row = &mut rows[at];
            row.country_pointer = system.country.pointer;
            row.region_pointer = candidate.region_pointer;
            row.needed = row.needed.wrapping_add(candidate.needed);
            row.supplied = row.supplied.wrapping_add(candidate.supplied);
            if candidate.ratio < 100_000 {
                row.deficit = row
                    .deficit
                    .wrapping_add(candidate.needed.wrapping_sub(candidate.supplied));
            }
            let severity = if candidate.ratio < context.thresholds[0] {
                0
            } else if candidate.ratio < context.thresholds[1] {
                1
            } else {
                2
            };
            row.severity = row.severity.min(severity);
        }
        Ok(())
    }
}

impl Capture {
    pub fn allocated_bytes(&self) -> usize {
        let scratch = &self.scratch;
        self.rows.capacity() * std::mem::size_of::<Summary>()
            + scratch.regions.capacity() * std::mem::size_of::<(u32, usize)>()
            + scratch.origins.capacity() * std::mem::size_of::<usize>()
            + scratch.origin_bytes.capacity()
            + scratch.region_ids.capacity()
                * std::mem::size_of::<(usize, Result<Option<u32>, &'static str>)>()
    }

    pub fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
    ) -> Result<(), &'static str> {
        self.rows.clear();
        let result = self.read_inner(memory, base, game);
        if result.is_err() {
            self.rows.clear();
        }
        result
    }

    fn read_inner<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game: usize,
    ) -> Result<(), &'static str> {
        let systems = csupply_system::Reader::new(memory, base, game)?;
        let countries = ccountry::Reader::new(memory, base, game)?;
        let mut resolver = ScopedResolver::new(base);
        let context = Context {
            memory,
            base,
            game,
            countries: &countries,
            provinces: array(memory, game, engine_native::cgame_state::PROVINCES, 8)?,
            thresholds: [
                global::<i64>(memory, base, VERY_LOW_THRESHOLD)?,
                global::<i64>(memory, base, LOW_THRESHOLD)?,
            ],
        };
        let Self { rows, scratch } = self;
        scratch.region_ids.clear();
        for slot in 1..systems.count() {
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            let start = rows.len();
            if scratch
                .country(&context, &mut resolver, slot, &system, rows)
                .is_err()
            {
                rows.truncate(start);
            }
        }
        Ok(())
    }
}
