
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    batch_fallback::{Batch, Link},
    cpdx_array::Array,
    memory::{
        Raw,
        RecordField,
        RecordLayout,
        RecordSources,
        packed_fields,
    },
    read,
    walk::Outcome,
};
use engine_native::ccountry::{self, Ahead};

pub const VTABLE_RVA: Rva = Rva(0x029f_e450);
pub const RECRUITMENT: usize = 0x08;
pub const OWNER_COUNTRY: usize = 0x78;
pub const NAME: usize = 0x80;
pub const ICON: usize = 0xa0;
pub const CREATED: usize = 0xc0;
pub const UPGRADE_PROGRESS: usize = 0xc8;
pub const OPERATIVES: usize = 0xf0;
#[cfg(test)]
pub const OPERATIVE_COUNT: usize = 0xfc;
pub const UNKNOWN_F0: usize = 0x108;
pub const OPERATIVE_ADMISSION_CEILING: usize = 0x10c;
pub const CAPTURED_OPERATIVE_REFERENCES: usize = 0x120;
pub const CRYPTOLOGY: usize = 0x138;
pub const DEFENSE: usize = 0x140;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0105_bdb0);
pub const ADMISSION_OVERRIDE_RVA: Rva = Rva(0x033d_da9e);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub upgrade_progress_raw: i64,
    pub defense_raw: i64,
    pub active_count: i32,
    pub admission_ceiling: i32,
    pub unknown_f0: u32,
    pub created: u8,
    pub name: engine_native::crelation::subject::Name,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> engine_native::crelation::subject::NameWalk {
    walk_shared(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, |_, _| {})
}

pub struct UpgradeWalk {
    pub agencies: engine_native::crelation::subject::NameWalk,
    pub upgrades: crate::agency_upgrade_levels::Walk,
}

pub fn walk_with_upgrades(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
    upgrades: &mut Vec<crate::agency_upgrade_levels::Row>,
    catalog: &mut crate::agency_upgrade_levels::Catalog,
) -> UpgradeWalk {
    walk_with_upgrades_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
        engine_native_memory::walk::Buffer::Growing(upgrades),
        catalog,
    )
}

pub fn walk_with_upgrades_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
    upgrades: engine_native_memory::walk::Buffer<'_, crate::agency_upgrade_levels::Row>,
    catalog: &mut crate::agency_upgrade_levels::Catalog,
) -> UpgradeWalk {
    let mut capture = crate::agency_upgrade_levels::Capture::new_buffer(upgrades, catalog);
    let agencies = walk_shared(
        memory,
        base,
        game_state,
        output,
        names,
        |country, agency| {
            capture.agency(memory, base, country, agency);
        },
    );
    if let Outcome::Unavailable { reason } = agencies.outcome {
        capture.failed(reason);
    }
    UpgradeWalk {
        agencies,
        upgrades: capture.finish(memory, base),
    }
}

fn walk_shared(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    mut each: impl FnMut(u32, usize),
) -> engine_native::crelation::subject::NameWalk {
    use engine_native_memory::msvc_string::Literal;
    use engine_native::crelation::subject::{Name, NameWalk};
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (mut len, mut bytes_len, mut required_bytes) = (0_usize, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        let mut ahead = None;
        let mut block = [0_u8; BLOCK];
        for slot in 1..countries.count() {
            let (listed, link) = Ahead::take(&mut ahead, &countries, slot);
            let Some(country) = listed? else {
                continue;
            };
            let root = root_block(memory, base, country, link, &mut block)?;
            ahead = Ahead::read(
                memory,
                &countries,
                slot + 1,
                ccountry::INTELLIGENCE_AGENCY,
                |agency| memory.prefetch(agency, BLOCK),
            );
            let Some((pointer, whole)) = root else {
                continue;
            };
            each(country.id, pointer);
            let name = if whole {
                Literal::from_header(
                    at(pointer, NAME)?,
                    block[NAME..NAME + 32].try_into().unwrap(),
                )?
            } else {
                Literal::read(memory, at(pointer, NAME)?)?
            };
            Literal::validate(memory, [&name])?;
            required_bytes = required_bytes
                .checked_add(name.len)
                .ok_or("agency name size overflow")?;
            output.ensure(len.checked_add(1).ok_or("agency row count overflow")?)?;
            names.ensure(required_bytes)?;
            if len == output.len() || name.len > names.len() - bytes_len {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("agency row count overflow")?;
                continue;
            }
            let active = if whole {
                Array::from_header(block[OPERATIVES..OPERATIVES + 16].try_into().unwrap(), 8)
            } else {
                Array::read(memory, at(pointer, OPERATIVES)?, 8)
            }
            .ok_or("agency active roster unreadable or invalid")?;
            Literal::copy_all(memory, [&name], &mut names[bytes_len..bytes_len + name.len])?;
            let scalars = if whole {
                Scalars::from_block(&block)
            } else {
                scalars(memory, pointer)?
            };
            output[len] = Row {
                country: country.id,
                upgrade_progress_raw: scalars.upgrade_progress_raw,
                defense_raw: scalars.defense_raw,
                active_count: active.count as i32,
                admission_ceiling: scalars.admission_ceiling,
                unknown_f0: scalars.unknown_f0,
                created: scalars.created,
                name: Name {
                    start: bytes_len,
                    len: name.len,
                },
            };
            bytes_len += name.len;
            len += 1;
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(engine_native::crelation::subject::unavailable)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CapturedOperativeRow {
    pub country: u32,
    pub ordinal: u32,
    pub operative: u64,
    pub owner: Option<u32>,
}

pub fn captured_operative_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    match count_captured_operatives(memory, base, game_state) {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("captured operative row count overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

pub fn count_captured_operatives(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_captured_operatives_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
}

pub fn walk_captured_operatives(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CapturedOperativeRow>,
) -> Outcome {
    walk_captured_operatives_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_captured_operatives_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, CapturedOperativeRow>,
) -> Outcome {
    use crate::{ccaptured_operative_reference as captured, coperative_leader};
    use engine_native_parts::creference_registry;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (mut len, mut unread_rows) = (0_usize, 0_u32);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let Some(agency) = root(memory, base, country)? else {
                continue;
            };
            let array = Array::read(
                memory,
                at(agency, CAPTURED_OPERATIVE_REFERENCES)?,
                captured::STRIDE,
            )
            .ok_or("captured operative roster unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("captured operative row demand overflow")?,
            )?;
            for ordinal in 0..array.count {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(array.count - ordinal)
                        .ok_or("captured operative row count overflow")?;
                    break;
                }
                let pointer = array
                    .element(ordinal, captured::STRIDE)
                    .ok_or("captured operative entry address overflow")?;
                let header = read::<[u8; 0x14]>(memory, pointer)
                    .ok_or("captured operative entry unreadable")?;
                if u64::from_le_bytes(header[..8].try_into().unwrap())
                    != engine_sites::at(base, captured::VTABLE_RVA)
                        .ok_or("module address overflow")? as u64
                {
                    return Err("captured operative entry vtable mismatch");
                }
                let operative = u64::from_le_bytes(header[0xC..0x14].try_into().unwrap());
                let owner = match creference_registry::resolve(memory, base, operative)? {
                    None => None,
                    Some(leader) => {
                        let identity = read::<[u8; 16]>(memory, leader)
                            .ok_or("captured operative referent unreadable")?;
                        if u64::from_le_bytes(identity[..8].try_into().unwrap())
                            != engine_sites::at(base, engine_native_parts::coperative_leader_layout::VTABLE_RVA)
                                .ok_or("module address overflow")?
                                as u64
                            || u64::from_le_bytes(identity[8..].try_into().unwrap()) != operative
                        {
                            return Err("captured operative referent identity mismatch");
                        }
                        Some(
                            read::<u32>(memory, at(leader, coperative_leader::OWNER_COUNTRY)?)
                                .ok_or("captured operative owner unreadable")?,
                        )
                    }
                };
                output[len] = CapturedOperativeRow {
                    country: country.id,
                    ordinal,
                    operative,
                    owner,
                };
                len += 1;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

const BLOCK: usize = DEFENSE + 8;

fn root_block(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    link: Link,
    block: &mut [u8; BLOCK],
) -> Result<Option<(usize, bool)>, &'static str> {
    let pointer = agency_link(memory, country, link)?;
    if pointer == 0 {
        return Ok(None);
    }
    if !memory.copy(pointer, block) {
        engine_native_memory::batch_fallback::record(Batch::IntelligenceAgencies);
        return Ok(authenticate(memory, base, country, pointer)?.map(|pointer| (pointer, false)));
    }
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("agency vtable address overflow")?;
    if u64::from_le_bytes(block[..8].try_into().unwrap()) != expected as u64 {
        return Err("agency vtable mismatch");
    }
    let owner = &block[OWNER_COUNTRY..OWNER_COUNTRY + 8];
    if u64::from_le_bytes(owner.try_into().unwrap()) != country.pointer as u64 {
        return Err("agency owner pointer mismatch");
    }
    Ok(Some((pointer, true)))
}

fn agency_link(
    memory: &impl Memory,
    country: engine_native_memory::country_identity::Country,
    link: Link,
) -> Result<usize, &'static str> {
    let link = link
        .resolve(
            memory,
            country,
            ccountry::INTELLIGENCE_AGENCY,
            Batch::IntelligenceAgencies,
        )
        .ok_or("agency field unreadable")?;
    usize::try_from(link).map_err(|_| "agency pointer overflow")
}

pub fn root(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<Option<usize>, &'static str> {
    let pointer = agency_link(memory, country, Link::Unread)?;
    if pointer == 0 {
        return Ok(None);
    }
    authenticate(memory, base, country, pointer)
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    pointer: usize,
) -> Result<Option<usize>, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("agency vtable address overflow")?;
    const FIELDS: [RecordField; 2] = packed_fields([(0, 8), (OWNER_COUNTRY, 8)]);
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 16).unwrap();
    if pointer.checked_add(OWNER_COUNTRY + 8).is_none() {
        if read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("agency vtable mismatch");
        }
        if field::<u64>(memory, pointer, OWNER_COUNTRY)? != country.pointer as u64 {
            return Err("agency owner pointer mismatch");
        }
        return Ok(Some(pointer));
    }
    let mut bytes = [0_u8; 16];
    let copied = memory.copy_records(RecordSources::Pointers(&[pointer]), &LAYOUT, &mut bytes);
    if copied < 1 {
        return Err("agency vtable mismatch");
    }
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
        return Err("agency vtable mismatch");
    }
    if copied < 2 {
        return Err("agency field unreadable");
    }
    if u64::from_le_bytes(bytes[8..].try_into().unwrap()) != country.pointer as u64 {
        return Err("agency owner pointer mismatch");
    }
    Ok(Some(pointer))
}

struct Scalars {
    upgrade_progress_raw: i64,
    defense_raw: i64,
    admission_ceiling: i32,
    unknown_f0: u32,
    created: u8,
}

impl Scalars {
    fn from_block(block: &[u8; BLOCK]) -> Self {
        let word = |offset: usize| -> [u8; 8] { block[offset..offset + 8].try_into().unwrap() };
        let half = |offset: usize| -> [u8; 4] { block[offset..offset + 4].try_into().unwrap() };
        Self {
            created: block[CREATED],
            upgrade_progress_raw: i64::from_le_bytes(word(UPGRADE_PROGRESS)),
            unknown_f0: u32::from_le_bytes(half(UNKNOWN_F0)),
            admission_ceiling: i32::from_le_bytes(half(OPERATIVE_ADMISSION_CEILING)),
            defense_raw: i64::from_le_bytes(word(DEFENSE)),
        }
    }
}

fn scalars(memory: &impl Memory, pointer: usize) -> Result<Scalars, &'static str> {
    const FIELDS: [RecordField; 5] = packed_fields([
        (CREATED, 1),
        (UPGRADE_PROGRESS, 8),
        (UNKNOWN_F0, 4),
        (OPERATIVE_ADMISSION_CEILING, 4),
        (DEFENSE, 8),
    ]);
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 25).unwrap();
    if pointer.checked_add(DEFENSE + 8).is_none() {
        return Ok(Scalars {
            upgrade_progress_raw: field(memory, pointer, UPGRADE_PROGRESS)?,
            defense_raw: field(memory, pointer, DEFENSE)?,
            admission_ceiling: field(memory, pointer, OPERATIVE_ADMISSION_CEILING)?,
            unknown_f0: field(memory, pointer, UNKNOWN_F0)?,
            created: field(memory, pointer, CREATED)?,
        });
    }
    let mut bytes = [0_u8; 25];
    if memory.copy_records(RecordSources::Pointers(&[pointer]), &LAYOUT, &mut bytes) != 5 {
        return Err("agency field unreadable");
    }
    Ok(Scalars {
        created: bytes[0],
        upgrade_progress_raw: i64::from_le_bytes(bytes[1..9].try_into().unwrap()),
        unknown_f0: u32::from_le_bytes(bytes[9..13].try_into().unwrap()),
        admission_ceiling: i32::from_le_bytes(bytes[13..17].try_into().unwrap()),
        defense_raw: i64::from_le_bytes(bytes[17..25].try_into().unwrap()),
    })
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer.checked_add(offset).ok_or("agency address overflow")
}

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("agency field unreadable")
}

#[cfg(test)]
#[path = "cintelligence_agency_cost_baseline.rs"]
mod baseline;

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use engine_native::ccountry::synthetic::World;
    use crate::{ccountry_decryption_state as state, ccryptology};
    use engine_native_memory::msvc_string;

    pub(crate) trait Image {
        fn alloc(&mut self, len: usize) -> usize;
        fn u64(&mut self, address: usize, value: u64);
        fn u32(&mut self, address: usize, value: u32);
        fn array(&mut self, header: usize, data: usize, count: u32);
        fn vtable(&self, key: Rva) -> u64;
    }

    impl Image for World {
        fn alloc(&mut self, len: usize) -> usize {
            World::alloc(self, len)
        }
        fn u64(&mut self, address: usize, value: u64) {
            World::u64(self, address, value);
        }
        fn u32(&mut self, address: usize, value: u32) {
            World::u32(self, address, value);
        }
        fn array(&mut self, header: usize, data: usize, count: u32) {
            World::array(self, header, data, count);
        }
        fn vtable(&self, key: Rva) -> u64 {
            (self.module + key.0) as u64
        }
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    impl Image for engine_native::crelation::status::bench::Arena {
        fn alloc(&mut self, len: usize) -> usize {
            engine_native::crelation::status::bench::Arena::alloc(self, len)
        }
        fn u64(&mut self, address: usize, value: u64) {
            engine_native::crelation::status::bench::Arena::u64(self, address, value);
        }
        fn u32(&mut self, address: usize, value: u32) {
            engine_native::crelation::status::bench::Arena::u32(self, address, value);
        }
        fn array(&mut self, header: usize, data: usize, count: u32) {
            self.vector(header, data, count);
        }
        fn vtable(&self, key: Rva) -> u64 {
            self.rva(key)
        }
    }

    pub(crate) fn populate(image: &mut impl Image, objects: &[usize], states: u32) {
        let countries = objects.len() as u32 - 1;
        for (slot, &country) in objects.iter().enumerate().skip(1) {
            let slot = slot as u32;
            let agency = image.alloc(0x160);
            image.u64(agency, image.vtable(VTABLE_RVA));
            image.u64(agency + OWNER_COUNTRY, country as u64);
            image.u64(agency + NAME, u64::from_le_bytes(*b"AGENCY\0\0"));
            image.u64(agency + NAME + msvc_string::LENGTH, 6);
            image.u64(
                agency + NAME + msvc_string::CAPACITY,
                msvc_string::INLINE_CAPACITY,
            );
            image.u32(agency + CREATED, 1);
            image.u64(agency + UPGRADE_PROGRESS, u64::from(slot) * 1_000);
            image.u32(agency + UNKNOWN_F0, slot);
            image.u32(agency + OPERATIVE_ADMISSION_CEILING, 3);
            image.u64(agency + DEFENSE, u64::from(slot) * 7);
            image.u64(country + ccountry::INTELLIGENCE_AGENCY, agency as u64);
            let crypto = image.alloc(0x40);
            image.u64(crypto, image.vtable(ccryptology::VTABLE_RVA));
            image.u64(crypto + ccryptology::OWNER_COUNTRY, country as u64);
            let rows = image.alloc(states as usize * state::STRIDE);
            for index in 0..states {
                let at = rows + index as usize * state::STRIDE;
                image.u64(at, image.vtable(state::VTABLE_RVA));
                image.u32(at + 8, (slot + index) % countries + 1);
                image.u64(at + 24, u64::from(index) * 100);
            }
            image.array(crypto + ccryptology::DECRYPTION_STATES, rows, states);
            image.u64(agency + CRYPTOLOGY, crypto as u64);
        }
    }

    pub(crate) fn intel_world(countries: u32, states: u32) -> World {
        let mut w = World::new(0, countries, ccountry::INTELLIGENCE_AGENCY + 8);
        let objects: Vec<usize> = (0..=countries).map(|slot| w.object(slot)).collect();
        populate(&mut w, &objects, states);
        w
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    pub(crate) fn intel_arena(
        countries: usize,
        states: u32,
    ) -> (engine_native::crelation::status::bench::Arena, usize) {
        use engine_native::crelation::status::bench::{Arena, countries as census};
        let object = ccountry::INTELLIGENCE_AGENCY + 8;
        let per_country = 12 + object + 16 + 0x160 + 0x40 + states as usize * state::STRIDE + 48;
        let mut arena = Arena::new(0x10_0000 + (countries + 1) * per_country);
        let (game_state, objects) = census(&mut arena, countries, object);
        populate(&mut arena, &objects, states);
        (arena, game_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    fn agency(w: &engine_native::ccountry::synthetic::World, slot: u32) -> usize {
        read::<u64>(&w.memory(), w.object(slot) + ccountry::INTELLIGENCE_AGENCY).unwrap() as usize
    }

    fn both(
        memory: &impl Memory,
        serial_memory: &impl Memory,
        w: &engine_native::ccountry::synthetic::World,
        rows: usize,
        names: usize,
    ) -> [(engine_native::crelation::subject::NameWalk, Vec<Row>, Vec<u8>); 2] {
        let mut walks = [(); 2].map(|_| (vec![Row::default(); rows], vec![0; names]));
        let [(new_rows, new_names), (old_rows, old_names)] = &mut walks;
        let new = super::walk_shared(memory, 0, w.game_state(), engine_native_memory::walk::Buffer::Growing(new_rows), engine_native_memory::walk::Buffer::Growing(new_names), |_, _| {});
        let old = baseline::walk(serial_memory, 0, w.game_state(), old_rows, old_names);
        let [(new_rows, new_names), (old_rows, old_names)] = walks;
        [(new, new_rows, new_names), (old, old_rows, old_names)]
    }

    #[test]
    fn batched_agencies_match_the_serial_walk() {
        use engine_native_memory::memory::{CountedMemory, ReadCounts};
        const COUNT: u32 = 130;
        let w = test_support::intel_world(COUNT, 2);
        let memory = CountedMemory::new(w.memory());
        let prior = ReadCounts::take_current();
        let mut rows = vec![Row::default(); COUNT as usize];
        let mut names = vec![0; COUNT as usize * 6];
        let new = super::walk_shared(&memory, 0, w.game_state(), engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut names), |_, _| {});
        let new_reads = ReadCounts::take_current();
        let mut old_rows = vec![Row::default(); COUNT as usize];
        let mut old_names = vec![0; COUNT as usize * 6];
        let old = baseline::walk(&memory, 0, w.game_state(), &mut old_rows, &mut old_names);
        let old_reads = ReadCounts::take_current();
        prior.add_current();
        assert_eq!(
            new.outcome,
            Outcome::Rows {
                len: COUNT as usize
            }
        );
        assert_eq!((new, &rows, &names), (old, &old_rows, &old_names));
        let n = u64::from(COUNT);
        assert_eq!(new_reads.copy_calls + 8 * n, old_reads.copy_calls);
        assert_eq!(new_reads.record_calls, old_reads.record_calls);
        assert_eq!(new_reads.spans + 8 * n, old_reads.spans);
    }

    fn fault_sites(w: &engine_native::ccountry::synthetic::World) -> [std::ops::Range<usize>; 4] {
        let agency = agency(w, 3);
        let link = w.object(4) + ccountry::INTELLIGENCE_AGENCY;
        [
            agency + RECRUITMENT..agency + RECRUITMENT + 8,
            agency + NAME..agency + NAME + 32,
            link..link + 8,
            agency + DEFENSE..agency + DEFENSE + 8,
        ]
    }

    fn fallbacks() -> u64 {
        engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::IntelligenceAgencies)
    }

    #[test]
    fn a_persistent_agency_fault_gives_the_serial_result() {
        use engine_native_memory::batch_fallback::Faulty;
        let w = test_support::intel_world(6, 2);
        for (site, range) in fault_sites(&w).into_iter().enumerate() {
            let before = fallbacks();
            let memory = Faulty::new(w.memory(), vec![range], false);
            let [batched, serial] = both(&memory, &memory, &w, 6, 36);
            assert_eq!(batched, serial, "site {site}");
            assert_eq!(fallbacks() - before, 1, "site {site}");
            if site == 0 {
                assert_eq!(batched.0.outcome, Outcome::Rows { len: 6 });
            } else {
                assert!(
                    matches!(batched.0.outcome, Outcome::Unavailable { .. }),
                    "site {site}"
                );
            }
        }
    }

    #[test]
    fn a_one_shot_agency_fault_refuses_no_more_than_the_serial_walk() {
        use engine_native_memory::batch_fallback::Faulty;
        let w = test_support::intel_world(6, 2);
        let [healthy, _] = both(&w.memory(), &w.memory(), &w, 6, 36);
        for (site, range) in fault_sites(&w).into_iter().enumerate() {
            let once = || Faulty::new(w.memory(), vec![range.clone()], true);
            let before = fallbacks();
            let [batched, serial] = both(&once(), &once(), &w, 6, 36);
            assert_eq!(fallbacks() - before, 1, "site {site}");
            assert_eq!(batched, healthy, "site {site}");
            assert_eq!(
                matches!(serial.0.outcome, Outcome::Unavailable { .. }),
                site != 0,
                "site {site}"
            );
        }
    }

    fn edge_world() -> engine_native::ccountry::synthetic::World {
        use engine_native_memory::msvc_string;
        let mut w = test_support::intel_world(8, 2);
        let table = engine_native::ccountry::synthetic::START + 0x1000;
        let links = table + 9 * 8;
        w.u64(table + 3 * 8, 0);
        w.u64(table + 5 * 8, w.object(4) as u64);
        w.u32(links + 5 * 4, 4);
        for (slot, len) in [(1_u32, 40_usize), (6, 20)] {
            let heap = w.alloc(len + 8);
            w.put(heap, &vec![b'a' + slot as u8; len]);
            let name = agency(&w, slot) + NAME;
            w.u64(name, heap as u64);
            w.u64(name + msvc_string::LENGTH, len as u64);
            w.u64(name + msvc_string::CAPACITY, len as u64 + 7);
        }
        let name = agency(&w, 2) + NAME;
        w.put(name, &[0; 8]);
        w.u64(name + msvc_string::LENGTH, 0);
        w
    }

    #[test]
    fn batched_agencies_match_the_serial_walk_at_name_and_slot_edges() {
        let w = edge_world();
        for (rows, names) in [(6, 78), (6, 39), (6, 25), (6, 0), (3, 78), (0, 0)] {
            let [batched, serial] = both(&w.memory(), &w.memory(), &w, rows, names);
            assert_eq!(batched, serial, "rows {rows} names {names}");
        }
        let [full, _] = both(&w.memory(), &w.memory(), &w, 6, 78);
        assert_eq!(full.0.outcome, Outcome::Rows { len: 6 });
        assert_eq!(full.0.required_bytes, 78);
    }

    #[test]
    fn upgrade_visits_match_the_serial_walk_under_persistent_faults() {
        use engine_native_memory::batch_fallback::Faulty;
        use engine_native_memory::walk::Buffer;
        let w = edge_world();
        let agency_six = agency(&w, 6);
        let link = w.object(7) + ccountry::INTELLIGENCE_AGENCY;
        let sites = [
            None,
            Some(agency_six + NAME..agency_six + NAME + 32),
            Some(agency_six + DEFENSE..agency_six + DEFENSE + 8),
            Some(link..link + 8),
        ];
        for (site, range) in sites.into_iter().enumerate() {
            let memory = Faulty::new(w.memory(), range.into_iter().collect(), false);
            let run = |serial: bool| {
                let mut visits = Vec::new();
                let mut rows = vec![Row::default(); 6];
                let mut names = vec![0; 78];
                let (rows_buffer, names_buffer) =
                    (Buffer::Growing(&mut rows), Buffer::Growing(&mut names));
                let visit = |country: u32, agency: usize| visits.push((country, agency));
                let walk = if serial {
                    baseline::walk_shared(
                        &memory,
                        0,
                        w.game_state(),
                        rows_buffer,
                        names_buffer,
                        visit,
                    )
                } else {
                    walk_shared(&memory, 0, w.game_state(), rows_buffer, names_buffer, visit)
                };
                (walk, rows, names, visits)
            };
            let batched = run(false);
            assert_eq!(batched, run(true), "site {site}");
            assert_eq!(
                matches!(batched.0.outcome, Outcome::Rows { len: 6 }),
                site == 0,
                "site {site}"
            );
        }
    }

    #[test]
    #[ignore = "synthetic acquisition cost; release, affinity 0xF"]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn acquisition_cost() {
        use engine_native_memory::memory::{CountedMemory, LiveMemory, ReadCounts};
        let countries = std::env::var("BATTLEPLAN_INTEL_COST_COUNTRIES").or_else(|_| std::env::var("HOI4_INTEL_COST_COUNTRIES"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000_u32);
        let (arena, state) = test_support::intel_arena(countries as usize, 3);
        let offset = arena.base();
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        let mut rows = vec![Row::default(); countries as usize];
        let mut names = vec![0; countries as usize * 6];
        for (label, gathered) in [("serial", false), ("batched", true)] {
            let mut samples = Vec::new();
            let mut reads = ReadCounts::ZERO;
            for _ in 0..40 {
                ReadCounts::take_current();
                let start = std::time::Instant::now();
                let walk = if gathered {
                    super::walk_shared(&memory, offset, state, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut names), |_, _| {})
                } else {
                    baseline::walk(&memory, offset, state, &mut rows, &mut names)
                };
                samples.push(start.elapsed().as_nanos() as f64);
                reads = ReadCounts::take_current();
                assert_eq!(
                    walk.outcome,
                    Outcome::Rows {
                        len: countries as usize
                    }
                );
                std::hint::black_box((&rows, &names));
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "intelligence_agencies {label} countries={countries} median_ns_per_country={:.1} copy_calls={} record_calls={} spans={}",
                samples[20] / f64::from(countries),
                reads.copy_calls,
                reads.record_calls,
                reads.spans
            );
        }
    }

    fn walk(memory: &impl Memory, base: usize, state: usize, rows: &mut Vec<Row>) -> Outcome {
        super::walk_shared(memory, base, state, engine_native_memory::walk::Buffer::Growing(rows), engine_native_memory::walk::Buffer::Count, |_, _| {}).outcome
    }

    fn count(memory: &impl Memory, base: usize, state: usize) -> Outcome {
        super::walk_shared(memory, base, state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, |_, _| {}).outcome
    }

    fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRIES,
            0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRIES + 8,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRIES + 12,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRY_LINKS,
            0x2100_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRY_LINKS + 8,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRY_LINKS + 12,
            2_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x2008, 0x3000_u64.to_le_bytes());
        put(&mut bytes, 0x2104, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000,
            (ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x3008, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + ccountry::INTELLIGENCE_AGENCY,
            0x5000_u64.to_le_bytes(),
        );
        put(&mut bytes, 0x5000, (VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, 0x5000 + OWNER_COUNTRY, 0x3000_u64.to_le_bytes());
        put(&mut bytes, 0x5000 + NAME + 24, 15_u64.to_le_bytes());
        bytes
    }

    #[test]
    fn growing_agency_name_arena_tracks_later_names_without_country_growth() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut names = Vec::new();
        let mut upgrades = Vec::new();
        let mut catalog = crate::agency_upgrade_levels::Catalog::new(0);
        for count in [3_usize, 5_000] {
            const PAYLOAD: usize = 0x7000;
            bytes.resize(PAYLOAD + count + 1, 0);
            let header = 0x5000 + NAME;
            put(&mut bytes, header, (PAYLOAD as u64).to_le_bytes());
            put(&mut bytes, header + 16, (count as u64).to_le_bytes());
            put(
                &mut bytes,
                header + 24,
                (count.max(16) as u64).to_le_bytes(),
            );
            bytes[PAYLOAD..PAYLOAD + count].fill(0xfe);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut expected_rows = vec![Row::default(); 1];
            let mut expected_names = vec![0; count];
            let expected = super::walk_shared(&memory, 0, 0x100, engine_native_memory::walk::Buffer::Growing(&mut expected_rows), engine_native_memory::walk::Buffer::Growing(&mut expected_names), |_, _| {});
            assert_eq!(expected.outcome, Outcome::Rows { len: 1 });
            let actual = walk_with_upgrades(
                &memory,
                0,
                0x100,
                &mut rows,
                &mut names,
                &mut upgrades,
                &mut catalog,
            );
            assert_eq!(actual.agencies, expected);
            assert_eq!(&rows[..1], &expected_rows);
            assert_eq!(&names[..count], expected_names.as_slice());
        }
    }

    #[test]
    fn captured_roster_preserves_nulls_order_and_resolved_owner_without_active_membership() {
        use crate::{ccaptured_operative_reference as captured, coperative_leader};
        use engine_native_parts::{creference_registry as registry, reference_bucket};
        let handle = (17_u64 << 32) | 54;
        let mut bytes = fixture();
        bytes.resize(0x7000, 0);
        put(
            &mut bytes,
            0x5000 + CAPTURED_OPERATIVE_REFERENCES,
            0x6000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + CAPTURED_OPERATIVE_REFERENCES + 8,
            3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + CAPTURED_OPERATIVE_REFERENCES + 12,
            3_i32.to_le_bytes(),
        );
        for ordinal in 0..3 {
            put(
                &mut bytes,
                0x6000 + ordinal * captured::STRIDE,
                (captured::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            0x6000 + captured::STRIDE + captured::OPERATIVE_HANDLE,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x6000 + 2 * captured::STRIDE + captured::OPERATIVE_HANDLE,
            (handle + (1 << 32)).to_le_bytes(),
        );
        let mut memory = registry::test_support::Fixture::default();
        memory.put(0, &bytes);
        memory.put(registry::LOW_ROOTS_RVA.0 + 54 * 8, &0x8000_u64.to_le_bytes());
        memory.put(0x8000 + registry::ENTRIES, &0x9000_u64.to_le_bytes());
        memory.put(0x8000 + registry::COUNT, &1_u32.to_le_bytes());
        memory.put(0x8000 + registry::MASK, &7_u32.to_le_bytes());
        memory.put(0x8000 + registry::OVERFLOW_DISTANCE, &[5]);
        memory.put(0x9000, &[0; 14 * reference_bucket::STRIDE]);
        memory.put(
            0x9000 + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE,
            &[255],
        );
        let index = registry::test_support::Fixture::index(handle);
        let bucket = 0x9000 + index * reference_bucket::STRIDE;
        memory.put(bucket + reference_bucket::DISTANCE, &[1]);
        memory.put(bucket + reference_bucket::TYPE, &handle.to_le_bytes());
        memory.put(bucket + reference_bucket::OBJECT, &0xa000_u64.to_le_bytes());
        memory.put(
            0xa000,
            &(engine_native_parts::coperative_leader_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        memory.put(0xa008, &handle.to_le_bytes());
        memory.put(
            0xa000 + coperative_leader::OWNER_COUNTRY,
            &5000_u32.to_le_bytes(),
        );
        let mut rows = vec![CapturedOperativeRow::default(); 3];
        assert_eq!(
            walk_captured_operatives(&memory, 0, 0x100, &mut rows),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(rows.iter().map(|row| row.owner).collect::<Vec<_>>(), [None, Some(5000), None]);
        assert_eq!(rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(rows[1].operative, handle);
        assert_eq!(captured_operative_demand(&memory, 0, 0x100), Ok(3));
        memory.put(0xa008, &0_u64.to_le_bytes());
        assert_eq!(
            walk_captured_operatives(&memory, 0, 0x100, &mut rows),
            Outcome::Unavailable {
                reason: "captured operative referent identity mismatch"
            }
        );
    }

    #[test]
    fn signed_qwords_and_capacity_are_preserved() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x5000 + UPGRADE_PROGRESS,
            (i64::MAX - 7).to_le_bytes(),
        );
        put(&mut bytes, 0x5000 + DEFENSE, (i64::MIN + 9).to_le_bytes());
        put(
            &mut bytes,
            0x5000 + OPERATIVE_ADMISSION_CEILING,
            (-3_i32).to_le_bytes(),
        );
        put(&mut bytes, 0x5000 + UNKNOWN_F0, u32::MAX.to_le_bytes());
        put(&mut bytes, 0x5000 + CREATED, [0x7f]);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 1];
        assert_eq!(walk(&memory, 0, 0x100, &mut rows), Outcome::Rows { len: 1 });
        assert_eq!(
            rows[0],
            Row {
                country: 1,
                upgrade_progress_raw: i64::MAX - 7,
                defense_raw: i64::MIN + 9,
                active_count: 0,
                admission_ceiling: -3,
                unknown_f0: u32::MAX,
                created: 0x7f,
                name: Default::default(),
            }
        );
        assert_eq!(
            count(&memory, 0, 0x100),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn null_agency_is_empty_but_bad_owner_or_partial_scalar_is_unavailable() {
        let mut bytes = fixture();
        put(&mut bytes, 0x5000 + OWNER_COUNTRY, 0x3008_u64.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut vec![Row::default()]
            ),
            Outcome::Unavailable {
                reason: "agency owner pointer mismatch"
            }
        );
        put(&mut bytes, 0x5000 + OWNER_COUNTRY, 0x3000_u64.to_le_bytes());
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x5000 + DEFENSE + 7]
                },
                0,
                0x100,
                &mut vec![Row::default()]
            ),
            Outcome::Unavailable { .. }
        ));
        put(
            &mut bytes,
            0x3000 + ccountry::INTELLIGENCE_AGENCY,
            0_u64.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut vec![Row::default()]
            ),
            Outcome::Rows { len: 0 }
        );
    }
    #[test]
    fn heap_name_demand_capacity_and_terminator_are_exact() {
        let mut bytes = fixture();
        let text = vec![b'x'; 40_000];
        bytes.resize(0x6000 + text.len() + 1, 0);
        bytes[0x6000..0x6000 + text.len()].copy_from_slice(&text);
        put(&mut bytes, 0x5000 + NAME, 0x6000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x5000 + NAME + 16,
            (text.len() as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + NAME + 24,
            (text.len() as u64).to_le_bytes(),
        );
        let mut rows = vec![Row::default()];
        let mut names = vec![0; text.len()];
        let read = super::walk_shared(&SliceMemory {
                base: 0,
                bytes: &bytes,
            }, 0, 0x100, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut names), |_, _| {});
        assert_eq!(read.outcome, Outcome::Rows { len: 1 });
        assert_eq!(read.required_bytes, text.len());
        assert_eq!(names, text);
        bytes[0x6000 + text.len()] = 1;
        assert!(matches!(
            super::walk_shared(&SliceMemory {
                    base: 0,
                    bytes: &bytes
                }, 0, 0x100, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut names), |_, _| {})
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
