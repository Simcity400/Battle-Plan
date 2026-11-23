
use crate::{
    Memory,
    batch_fallback::Batch,
    ccountry::{self, Ahead}, ccountry_decryption_state as state, cintelligence_agency as agency,
    cpdx_array::Array,
    memory::{Raw, RecordField, RecordLayout, RecordSources, packed_fields},
    read,
    walk::Outcome,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a3_7ef0);
pub const DECRYPTION_STATES: usize = 0x28;
#[cfg(test)]
pub const DECRYPTION_STATE_COUNT: usize = 0x34;
#[cfg(test)]
pub const DECRYPTION_STATE_CAPACITY: usize = 0x30;
pub const OWNER_COUNTRY: usize = 0x08;
#[cfg(test)]
pub const SAVE_RVA: usize = 0x0149_4e50;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner_country: u32,
    pub target_country: u32,
    pub amount_raw: i64,
    pub bonus_days: i32,
    pub bonus_active: u8,
    pub decrypting: u8,
    pub ordinal: u32,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_inner(memory, base, game_state, output)
        .unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut found_country = false;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    let mut seen = crate::slot_set::SlotSet::new();
    let mut ahead = None;
    let mut block = [0_u8; STATE_CHUNK * state::STRIDE];
    for slot in 1..countries.count() {
        let (listed, link) = Ahead::take(&mut ahead, &countries, slot);
        let Some(country) = listed? else {
            continue;
        };
        found_country = true;
        let agency_pointer = link
            .resolve(
                memory,
                country,
                ccountry::INTELLIGENCE_AGENCY,
                Batch::Cryptology,
            )
            .ok_or("cryptology field unreadable or address overflow")
            .and_then(|link| usize::try_from(link).map_err(|_| "cryptology pointer overflow"))?;
        ahead = Ahead::read(
            memory,
            &countries,
            slot + 1,
            ccountry::INTELLIGENCE_AGENCY,
            |object| memory.prefetch(object, agency::CRYPTOLOGY + 8),
        );
        if agency_pointer == 0 {
            continue;
        }
        let crypto = cryptology(memory, base, agency_pointer, country.pointer)?;
        if crypto == 0 {
            continue;
        }
        let states = decryption_states(memory, base, crypto, country.pointer)?;
        output.ensure(
            len.checked_add(states.count as usize)
                .ok_or("cryptology row demand overflow")?,
        )?;
        let owner_start = len;
        let (mut first, mut chunk, mut whole) = (0, 0, false);
        for index in 0..states.count {
            if len == output.len() {
                unread_rows = unread_rows
                    .checked_add(states.count - index)
                    .ok_or("cryptology unread row count overflow")?;
                break;
            }
            if index == first + chunk {
                first = index;
                chunk = (states.count - index)
                    .min(STATE_CHUNK as u32)
                    .min(u32::try_from(output.len() - len).unwrap_or(u32::MAX));
                whole = state_block(memory, states, first, chunk, &mut block);
                if !whole {
                    crate::batch_fallback::record(Batch::Cryptology);
                }
            }
            let mut row = if whole {
                let start = (index - first) as usize * state::STRIDE;
                decode_state(base, block[start..start + 32].try_into().unwrap())?
            } else {
                let at = states
                    .element(index, state::STRIDE)
                    .ok_or("cryptology row address overflow")?;
                read_state(memory, base, at)?
            };
            let target = countries.resolve(row.target_country)?;
            let slot = target.canonical_slot;
            let earlier = &output[owner_start..len];
            if seen.insert(slot, || {
                Ok::<_, &'static str>(earlier.iter().any(|p| p.owner_country == slot))
            })? {
                return Err("duplicate cryptology target");
            }
            row.owner_country = target.canonical_slot;
            row.ordinal = index;
            output[len] = row;
            len += 1;
        }
        seen.clear();
        for row in &mut output[owner_start..len] {
            row.owner_country = country.id;
        }
    }
    if !found_country {
        Err("country table has no readable countries")
    } else if unread_rows != 0 {
        Ok(Outcome::Truncated { len, unread_rows })
    } else {
        Ok(Outcome::Rows { len })
    }
}

fn cryptology(
    memory: &impl Memory,
    base: usize,
    agency_pointer: usize,
    country: usize,
) -> Result<usize, &'static str> {
    const FIELDS: [RecordField; 3] =
        packed_fields([(0, 8), (agency::OWNER_COUNTRY, 8), (agency::CRYPTOLOGY, 8)]);
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 24).unwrap();
    if agency_pointer.checked_add(agency::CRYPTOLOGY + 8).is_none() {
        authenticate(memory, base, agency_pointer, agency::VTABLE_RVA)?;
        if pointer(memory, agency_pointer, agency::OWNER_COUNTRY)? != country {
            return Err("cryptology agency owner mismatch");
        }
        return pointer(memory, agency_pointer, agency::CRYPTOLOGY);
    }
    let expected = engine_sites::at(base, agency::VTABLE_RVA).ok_or("module address overflow")?;
    let mut bytes = [0_u8; 24];
    let copied = memory.copy_records(
        RecordSources::Pointers(&[agency_pointer]),
        &LAYOUT,
        &mut bytes,
    );
    let word =
        |index: usize| u64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap());
    if copied < 1 || word(0) != expected as u64 {
        return Err("cryptology object vtable unreadable or mismatched");
    }
    if copied < 2 {
        return Err("cryptology field unreadable or address overflow");
    }
    if usize::try_from(word(1)).map_err(|_| "cryptology pointer overflow")? != country {
        return Err("cryptology agency owner mismatch");
    }
    if copied < 3 {
        return Err("cryptology field unreadable or address overflow");
    }
    usize::try_from(word(2)).map_err(|_| "cryptology pointer overflow")
}

fn decryption_states(
    memory: &impl Memory,
    base: usize,
    crypto: usize,
    country: usize,
) -> Result<Array, &'static str> {
    const FIELDS: [RecordField; 3] =
        packed_fields([(0, 8), (OWNER_COUNTRY, 8), (DECRYPTION_STATES, 16)]);
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 32).unwrap();
    if crypto.checked_add(DECRYPTION_STATES + 16).is_none() {
        authenticate(memory, base, crypto, VTABLE_RVA)?;
        if pointer(memory, crypto, OWNER_COUNTRY)? != country {
            return Err("cryptology owner mismatch");
        }
        return crypto
            .checked_add(DECRYPTION_STATES)
            .and_then(|address| Array::read(memory, address, state::STRIDE))
            .ok_or("cryptology state array unreadable or invalid");
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let mut bytes = [0_u8; 32];
    let copied = memory.copy_records(RecordSources::Pointers(&[crypto]), &LAYOUT, &mut bytes);
    let word =
        |index: usize| u64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap());
    if copied < 1 || word(0) != expected as u64 {
        return Err("cryptology object vtable unreadable or mismatched");
    }
    if copied < 2 {
        return Err("cryptology field unreadable or address overflow");
    }
    if usize::try_from(word(1)).map_err(|_| "cryptology pointer overflow")? != country {
        return Err("cryptology owner mismatch");
    }
    if copied < 3 {
        return Err("cryptology state array unreadable or invalid");
    }
    Array::from_header(bytes[16..32].try_into().unwrap(), state::STRIDE)
        .ok_or("cryptology state array unreadable or invalid")
}

const STATE_CHUNK: usize = 64;

fn state_block(
    memory: &impl Memory,
    states: Array,
    first: u32,
    chunk: u32,
    block: &mut [u8; STATE_CHUNK * state::STRIDE],
) -> bool {
    let len = (chunk as usize - 1) * state::STRIDE + 32;
    match (
        states.element(first, state::STRIDE),
        states.element(first + chunk - 1, state::STRIDE),
    ) {
        (Some(start), Some(_)) => memory.copy(start, &mut block[..len]),
        _ => false,
    }
}

fn read_state(memory: &impl Memory, base: usize, at: usize) -> Result<Row, &'static str> {
    let bytes = read::<[u8; 32]>(memory, at).ok_or("cryptology state unreadable")?;
    decode_state(base, bytes)
}

fn decode_state(base: usize, bytes: [u8; 32]) -> Result<Row, &'static str> {
    let expected = engine_sites::at(base, state::VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(bytes[0..8].try_into().unwrap()) != expected as u64 {
        return Err("cryptology state vtable mismatch");
    }
    Ok(Row {
        owner_country: 0,
        target_country: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        bonus_days: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        bonus_active: bytes[state::BONUS_ACTIVE],
        decrypting: bytes[state::DECRYPTING],
        amount_raw: i64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        ordinal: 0,
    })
}

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("cryptology field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "cryptology pointer overflow")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    rva: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, rva).ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) == Some(expected as u64) {
        Ok(())
    } else {
        Err("cryptology object vtable unreadable or mismatched")
    }
}

#[cfg(test)]
#[path = "ccryptology_cost_baseline.rs"]
mod baseline;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    fn both(
        memory: &impl Memory,
        serial_memory: &impl Memory,
        w: &crate::ccountry::synthetic::World,
        rows: usize,
    ) -> [(Outcome, Vec<Row>); 2] {
        let mut batched = vec![Row::default(); rows];
        let mut serial = batched.clone();
        let new = super::walk(memory, 0, w.game_state(), &mut batched);
        let old = baseline::walk(serial_memory, 0, w.game_state(), &mut serial);
        [(new, batched), (old, serial)]
    }

    #[test]
    fn batched_states_match_the_serial_walk() {
        use crate::memory::{CountedMemory, ReadCounts};
        const COUNT: u32 = 130;
        const STATES: u32 = 3;
        let w = crate::cintelligence_agency::test_support::intel_world(COUNT, STATES);
        let memory = CountedMemory::new(w.memory());
        let rows = (COUNT * STATES) as usize;
        let prior = ReadCounts::take_current();
        let mut batched = vec![Row::default(); rows];
        let new = super::walk(&memory, 0, w.game_state(), &mut batched);
        let new_reads = ReadCounts::take_current();
        let mut serial = vec![Row::default(); rows];
        let old = baseline::walk(&memory, 0, w.game_state(), &mut serial);
        let old_reads = ReadCounts::take_current();
        prior.add_current();
        assert_eq!(new, Outcome::Rows { len: rows });
        assert_eq!((new, &batched), (old, &serial));
        let n = u64::from(COUNT);
        assert_eq!(new_reads.copy_calls + 8 * n, old_reads.copy_calls);
        assert_eq!(new_reads.record_calls, old_reads.record_calls + 2 * n);
        assert_eq!(new_reads.spans + 2 * n, old_reads.spans);
    }

    fn state_at(w: &crate::ccountry::synthetic::World, slot: u32, index: usize) -> usize {
        let memory = w.memory();
        let agency =
            read::<u64>(&memory, w.object(slot) + ccountry::INTELLIGENCE_AGENCY).unwrap() as usize;
        let crypto = read::<u64>(&memory, agency + agency::CRYPTOLOGY).unwrap() as usize;
        let rows = read::<u64>(&memory, crypto + DECRYPTION_STATES).unwrap() as usize;
        rows + index * state::STRIDE
    }

    fn fault_sites(w: &crate::ccountry::synthetic::World) -> [std::ops::Range<usize>; 3] {
        let tail = state_at(w, 3, 0) + 32;
        let prefix = state_at(w, 3, 1) + 24;
        let link = w.object(4) + ccountry::INTELLIGENCE_AGENCY;
        [tail..tail + 8, prefix..prefix + 8, link..link + 8]
    }

    fn fallbacks() -> u64 {
        crate::batch_fallback::count(crate::batch_fallback::Batch::Cryptology)
    }

    #[test]
    fn a_persistent_state_fault_gives_the_serial_result() {
        use crate::batch_fallback::Faulty;
        let w = crate::cintelligence_agency::test_support::intel_world(6, 3);
        for (site, range) in fault_sites(&w).into_iter().enumerate() {
            let before = fallbacks();
            let memory = Faulty::new(w.memory(), vec![range], false);
            let [batched, serial] = both(&memory, &memory, &w, 18);
            assert_eq!(batched, serial, "site {site}");
            assert_eq!(fallbacks() - before, 1, "site {site}");
            if site == 0 {
                assert_eq!(batched.0, Outcome::Rows { len: 18 });
            } else {
                assert!(
                    matches!(batched.0, Outcome::Unavailable { .. }),
                    "site {site}"
                );
            }
        }
    }

    #[test]
    fn a_one_shot_state_fault_refuses_no_more_than_the_serial_walk() {
        use crate::batch_fallback::Faulty;
        let w = crate::cintelligence_agency::test_support::intel_world(6, 3);
        let [healthy, _] = both(&w.memory(), &w.memory(), &w, 18);
        for (site, range) in fault_sites(&w).into_iter().enumerate() {
            let once = || Faulty::new(w.memory(), vec![range.clone()], true);
            let before = fallbacks();
            let [batched, serial] = both(&once(), &once(), &w, 18);
            assert_eq!(fallbacks() - before, 1, "site {site}");
            assert_eq!(batched, healthy, "site {site}");
            assert_eq!(
                matches!(serial.0, Outcome::Unavailable { .. }),
                site != 0,
                "site {site}"
            );
        }
    }

    #[test]
    fn batched_states_cross_the_block_edge_as_the_serial_walk() {
        use crate::batch_fallback::Faulty;
        const STATES: u32 = 66;
        let w = crate::cintelligence_agency::test_support::intel_world(70, STATES);
        let all = 70 * STATES as usize;
        for rows in [all, 64, 65, 66, 66 + 64, 66 + 65, 0] {
            let [batched, serial] = both(&w.memory(), &w.memory(), &w, rows);
            assert_eq!(batched, serial, "rows {rows}");
        }
        let range = |index: usize, offset: usize, len: usize| {
            let at = state_at(&w, 3, index) + offset;
            at..at + len
        };
        for (site, (fault, refuses, counted)) in [
            (range(63, 32, 24), false, 0),
            (range(62, 32, 24), false, 1),
            (range(64, 24, 8), true, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let before = fallbacks();
            let memory = Faulty::new(w.memory(), vec![fault.clone()], false);
            let [batched, serial] = both(&memory, &memory, &w, all);
            assert_eq!(batched, serial, "site {site}");
            assert_eq!(
                matches!(batched.0, Outcome::Unavailable { .. }),
                refuses,
                "site {site}"
            );
            assert_eq!(fallbacks() - before, counted, "site {site}");
            let before = fallbacks();
            let once = || Faulty::new(w.memory(), vec![fault.clone()], true);
            let [batched, serial] = both(&once(), &once(), &w, all);
            assert_eq!(batched.0, Outcome::Rows { len: all }, "site {site}");
            assert_eq!(
                matches!(serial.0, Outcome::Unavailable { .. }),
                refuses,
                "site {site}"
            );
            assert_eq!(fallbacks() - before, counted, "site {site}");
        }
    }

    #[test]
    #[ignore = "synthetic acquisition cost; release, affinity 0xF"]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn acquisition_cost() {
        use crate::memory::{CountedMemory, LiveMemory, ReadCounts};
        let countries = std::env::var("BATTLEPLAN_INTEL_COST_COUNTRIES").or_else(|_| std::env::var("HOI4_INTEL_COST_COUNTRIES"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000_u32);
        let (arena, state) =
            crate::cintelligence_agency::test_support::intel_arena(countries as usize, 3);
        let offset = arena.base();
        // SAFETY: `intel_arena` finished building the arena every read points into, which stays
        // allocated and immutable through both cases.
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        let mut rows = vec![Row::default(); countries as usize * 3];
        for (label, gathered) in [("serial", false), ("batched", true)] {
            let mut samples = Vec::new();
            let mut reads = ReadCounts::ZERO;
            for _ in 0..40 {
                ReadCounts::take_current();
                let start = std::time::Instant::now();
                let walk = if gathered {
                    super::walk(&memory, offset, state, &mut rows)
                } else {
                    baseline::walk(&memory, offset, state, &mut rows)
                };
                samples.push(start.elapsed().as_nanos() as f64);
                reads = ReadCounts::take_current();
                assert_eq!(
                    walk,
                    Outcome::Rows {
                        len: countries as usize * 3
                    }
                );
                std::hint::black_box(&rows);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "cryptology {label} countries={countries} median_ns_per_country={:.1} copy_calls={} record_calls={} spans={}",
                samples[20] / f64::from(countries),
                reads.copy_calls,
                reads.record_calls,
                reads.spans
            );
        }
    }

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x3000;
    const LINKS: usize = 0x3100;
    const COUNTRY: [usize; 3] = [0x4000, 0x6000, 0x8000];
    const AGENCY: usize = 0xa000;
    const CRYPTO: usize = 0xb000;
    const STATES: usize = 0xc000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, capacity: u32, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, capacity);
        p32(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xd000];
        header(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 5, 5);
        header(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 5, 5);
        for (index, pointer) in [usize::MAX, COUNTRY[0], COUNTRY[1], COUNTRY[2], COUNTRY[0]]
            .into_iter()
            .enumerate()
        {
            p64(&mut bytes, COUNTRIES + index * 8, pointer as u64);
        }
        for (index, slot) in [0, 1, 2, 3, 1].into_iter().enumerate() {
            p32(&mut bytes, LINKS + index * 4, slot);
        }
        for (index, country) in COUNTRY.into_iter().enumerate() {
            p64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            p32(&mut bytes, country + ccountry::ID, [4, 2, 3][index]);
        }
        p64(
            &mut bytes,
            COUNTRY[0] + ccountry::INTELLIGENCE_AGENCY,
            AGENCY as u64,
        );
        p64(&mut bytes, AGENCY, agency::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            AGENCY + agency::OWNER_COUNTRY,
            COUNTRY[0] as u64,
        );
        p64(&mut bytes, AGENCY + agency::CRYPTOLOGY, CRYPTO as u64);
        p64(&mut bytes, CRYPTO, VTABLE_RVA.0 as u64);
        p64(&mut bytes, CRYPTO + OWNER_COUNTRY, COUNTRY[0] as u64);
        header(&mut bytes, CRYPTO + DECRYPTION_STATES, STATES, 3, 2);
        for (index, target) in [2, 3].into_iter().enumerate() {
            let at = STATES + index * state::STRIDE;
            p64(&mut bytes, at, state::VTABLE_RVA.0 as u64);
            p32(&mut bytes, at + state::TARGET_COUNTRY, target);
            p32(&mut bytes, at + state::BONUS_DAYS, (-1_i32) as u32);
            bytes[at + state::BONUS_ACTIVE] = 7;
            bytes[at + state::DECRYPTING] = 255;
        }
        p64(
            &mut bytes,
            STATES + state::STRIDE + state::AMOUNT,
            0x1_2345_6789,
        );
        bytes
    }

    fn run(bytes: &[u8], output: &mut Vec<Row>) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, output)
    }

    #[test]
    fn native_values_alias_owner_zero_progress_and_live_count_are_preserved() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 3];
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows[0],
            Row {
                owner_country: 4,
                target_country: 2,
                amount_raw: 0,
                bonus_days: -1,
                bonus_active: 7,
                decrypting: 255,
                ordinal: 0,
            }
        );
        assert_eq!(rows[1].owner_country, 4);
        assert_eq!(rows[1].target_country, 3);
        assert_eq!(rows[1].amount_raw, 0x1_2345_6789);
    }

    #[test]
    fn absent_manager_and_empty_array_are_measured_empty() {
        let original = fixture();
        let mut rows = vec![Row::default(); 2];
        let mut bytes = original.clone();
        p64(&mut bytes, AGENCY + agency::CRYPTOLOGY, 0);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 0 });
        let mut bytes = original.clone();
        p64(&mut bytes, COUNTRY[0] + ccountry::INTELLIGENCE_AGENCY, 0);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 0 });
        let mut bytes = original;
        header(&mut bytes, CRYPTO + DECRYPTION_STATES, 0, 0, 0);
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 0 });
    }

    #[test]
    fn corrupt_headers_objects_and_country_links_are_unavailable() {
        let original = fixture();
        let mut rows = vec![Row::default(); 3];
        for (at, value) in [
            (CRYPTO + DECRYPTION_STATE_COUNT, 4),
            (CRYPTO + DECRYPTION_STATE_COUNT, u32::MAX),
            (CRYPTO + DECRYPTION_STATE_CAPACITY, u32::MAX),
            (STATES + state::TARGET_COUNTRY, 0),
            (STATES + state::TARGET_COUNTRY, 8),
        ] {
            let mut bytes = original.clone();
            p32(&mut bytes, at, value);
            assert!(matches!(
                run(&bytes, &mut rows),
                Outcome::Unavailable { .. }
            ));
        }
        for (at, value) in [
            (AGENCY, 1),
            (CRYPTO, 1),
            (STATES, 1),
            (AGENCY + agency::OWNER_COUNTRY, COUNTRY[1] as u64),
            (CRYPTO + OWNER_COUNTRY, COUNTRY[1] as u64),
            (CRYPTO + DECRYPTION_STATES, u64::MAX - 4),
        ] {
            let mut bytes = original.clone();
            p64(&mut bytes, at, value);
            assert!(matches!(
                run(&bytes, &mut rows),
                Outcome::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn repeated_target_aliases_fail_the_table() {
        let original = fixture();
        let mut rows = vec![Row::default(); 2];
        let mut bytes = original.clone();
        p32(&mut bytes, STATES + state::TARGET_COUNTRY, 1);
        p32(
            &mut bytes,
            STATES + state::STRIDE + state::TARGET_COUNTRY,
            4,
        );
        assert_eq!(
            run(&bytes, &mut rows),
            Outcome::Unavailable {
                reason: "duplicate cryptology target"
            }
        );
    }

    #[test]
    fn unreadable_live_row_is_not_silently_omitted() {
        let bytes = fixture();
        assert!(matches!(
            run(
                &bytes[..STATES + state::STRIDE + 31],
                &mut vec![Row::default(); 2]
            ),
            Outcome::Unavailable {
                reason: "cryptology state unreadable"
            }
        ));
    }
}
