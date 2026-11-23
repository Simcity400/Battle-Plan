use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
    walk::{Buffer, Outcome},
};
use engine_native::ccountry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029e_f950);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029e_f980);
pub const SECONDARY: usize = 8;
pub const OWNER: usize = 0x10;
pub const REGION_MISSIONS: usize = 0x68;
pub const REGION_MISSIONS_STRIDE: usize = 0x18;
pub const REGISTERED_MISSIONS: usize = 0x110;
pub const REGIONAL_OPERANDS: usize = 0xc8;
pub const REGIONAL_CONVOYS: usize = 0x98;
pub const REGIONAL_CONVOYS_STRIDE: usize = 0x30;
pub const REGIONAL_CONVOY_EFFICIENCY: usize = 0x10;
pub const FULL_REGIONAL_CONVOY_EFFICIENCY: i64 = 100_000;

pub fn regional_convoys(
    memory: &impl Memory,
    base: usize,
    navy: usize,
) -> Result<Array, &'static str> {
    with_identity(memory, base, navy, || {
        let root = navy
            .checked_add(REGIONAL_CONVOYS)
            .ok_or("navy regional convoys root overflow")?;
        Array::read(memory, root, REGIONAL_CONVOYS_STRIDE).ok_or("navy regional convoys invalid")
    })
}

pub fn regional_convoy_efficiency(
    memory: &impl Memory,
    records: Array,
    region: u32,
) -> Result<i64, &'static str> {
    let Some(record) = records.element(region, REGIONAL_CONVOYS_STRIDE) else {
        return Ok(FULL_REGIONAL_CONVOY_EFFICIENCY);
    };
    record
        .checked_add(REGIONAL_CONVOY_EFFICIENCY)
        .and_then(|address| read::<i64>(memory, address))
        .ok_or("navy regional convoy efficiency unreadable")
}

pub fn registered_missions(memory: &impl Memory, navy: usize) -> Result<u32, &'static str> {
    navy.checked_add(REGISTERED_MISSIONS)
        .and_then(|at| Array::read(memory, at, 8))
        .map(|missions| missions.count)
        .ok_or("navy registered missions unreadable or invalid")
}

fn with_identity<T>(
    memory: &impl Memory,
    base: usize,
    navy: usize,
    consume: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let primary = engine_sites::at(base, VTABLE_RVA).ok_or("navy vtable overflow")? as u64;
    let secondary =
        engine_sites::at(base, SECONDARY_VTABLE_RVA).ok_or("navy vtable overflow")? as u64;
    let secondary_at = navy
        .checked_add(SECONDARY)
        .ok_or("navy identity overflow")?;
    if navy == 0
        || read::<u64>(memory, navy) != Some(primary)
        || read::<u64>(memory, secondary_at) != Some(secondary)
    {
        return Err("navy identity mismatch");
    }
    consume()
}

pub fn authenticate(memory: &impl Memory, base: usize, navy: usize) -> Result<(), &'static str> {
    with_identity(memory, base, navy, || Ok(()))
}

pub(crate) fn regional_operand(
    memory: &impl Memory,
    base: usize,
    navy: usize,
    region: u32,
) -> Result<i64, &'static str> {
    let array = regional_inputs(memory, base, navy)?;
    let at = array
        .element(region, 8)
        .ok_or("navy regional operand outside array")?;
    read::<i64>(memory, at).ok_or("navy regional operand unreadable")
}

pub(crate) fn regional_inputs(
    memory: &impl Memory,
    base: usize,
    navy: usize,
) -> Result<Array, &'static str> {
    with_identity(memory, base, navy, || regional_array(memory, navy))
}

pub fn regional_array(memory: &impl Memory, navy: usize) -> Result<Array, &'static str> {
    let root = navy
        .checked_add(REGIONAL_OPERANDS)
        .ok_or("navy regional root overflow")?;
    Array::read(memory, root, 8).ok_or("navy regional array invalid")
}

pub const ACCESS_MODES: usize = 0xb0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AccessRow {
    pub country: u32,
    pub region: u32,
    pub mode: u8,
}

const NAVY_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: SECONDARY,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: ACCESS_MODES,
        destination_offset: 16,
        len: 16,
    },
];
const NAVY_BYTES: usize = 32;
const NAVY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NAVY_FIELDS, NAVY_BYTES).expect("valid navy access layout");
const BATCH: usize = 64;
const MODE_CHUNK: usize = 512;
const MODE_SPANS: usize = 16;

#[derive(Clone, Copy, Default)]
struct ModeSpan {
    address: usize,
    len: usize,
    country: u32,
    region: u32,
}

struct ModeSpans {
    spans: [ModeSpan; MODE_SPANS],
    len: usize,
}

impl ModeSpans {
    fn flush(
        &mut self,
        memory: &impl Memory,
        modes: &mut [u8; MODE_SPANS * MODE_CHUNK],
        output: &mut Buffer<'_, AccessRow>,
        len: &mut usize,
        unread: &mut u32,
    ) -> Result<(), &'static str> {
        let pending = self.len;
        self.len = 0;
        if pending == 0 {
            return Ok(());
        }
        let spans = self.spans;
        let addresses: [usize; MODE_SPANS] = std::array::from_fn(|index| {
            if index < pending {
                spans[index].address
            } else {
                0
            }
        });
        let mut chunks = modes.chunks_exact_mut(MODE_CHUNK);
        let mut destinations: [&mut [u8]; MODE_SPANS] = std::array::from_fn(|index| {
            let chunk = chunks.next().unwrap();
            let take = if index < pending { spans[index].len } else { 0 };
            &mut chunk[..take]
        });
        if memory.copy_scattered(addresses, &mut destinations) < MODE_SPANS {
            return Err("navy access modes unreadable");
        }
        let (mut rows, mut missing) = (*len, *unread);
        for (span, bytes) in spans[..pending].iter().zip(modes.chunks_exact(MODE_CHUNK)) {
            for (word_index, word) in bytes[..span.len].chunks(8).enumerate() {
                if word.iter().all(|&mode| mode == 0) {
                    continue;
                }
                for (offset, &mode) in word.iter().enumerate() {
                    if mode == 0 {
                        continue;
                    }
                    if rows == output.len() {
                        missing = missing
                            .checked_add(1)
                            .ok_or("navy access unread row count overflow")?;
                        continue;
                    }
                    output[rows] = AccessRow {
                        country: span.country,
                        region: span.region + (word_index * 8 + offset) as u32,
                        mode,
                    };
                    rows += 1;
                }
            }
        }
        (*len, *unread) = (rows, missing);
        Ok(())
    }
}

pub fn count_access_modes(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    access_modes_buffer(memory, base, game_state, Buffer::Count)
}

#[cfg(any(test, feature = "test-support"))]
pub fn access_modes(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<AccessRow>,
) -> Outcome {
    access_modes_buffer(memory, base, game_state, Buffer::Growing(output))
}

pub fn access_modes_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, AccessRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let manager = game_state
            .checked_add(engine_native::cgame_state::STRATEGIC_NAVY_MANAGER)
            .and_then(|root| read::<u64>(memory, root))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("navy manager absent or unreadable")?;
        let expected = |rva: Rva| {
            engine_sites::at(base, rva)
                .map(|value| value as u64)
                .ok_or("module address overflow")
        };
        if read::<u64>(memory, manager)
            != Some(expected(crate::cstrategic_navy_manager::VTABLE_RVA)?)
        {
            return Err("navy manager identity mismatch");
        }
        let navies = manager
            .checked_add(crate::cstrategic_navy_manager::NAVIES)
            .and_then(|header| Array::read(memory, header, 8))
            .ok_or("navy manager array invalid")?;
        let (primary, secondary) = (expected(VTABLE_RVA)?, expected(SECONDARY_VTABLE_RVA)?);
        let slots = navies.count;
        let mut words = [0_u8; BATCH * 8];
        let mut pointers = [0_usize; BATCH];
        let mut navy_slots = [0_u32; BATCH];
        let mut records = [0_u8; BATCH * NAVY_BYTES];
        let mut modes = [0_u8; MODE_SPANS * MODE_CHUNK];
        let mut pieces = ModeSpans {
            spans: [ModeSpan::default(); MODE_SPANS],
            len: 0,
        };
        let (mut len, mut unread) = (0, 0_u32);
        let mut required_rows = 0_usize;
        let mut first = 1;
        while first < slots {
            let count = (slots - first).min(BATCH as u32) as usize;
            let address = navies
                .element(first, 8)
                .ok_or("navy manager array overflow")?;
            if !read_slice(memory, address, &mut words[..count * 8]) {
                return Err("navy pointers unreadable");
            }
            let mut present = 0;
            for (index, word) in words[..count * 8].chunks_exact(8).enumerate() {
                let navy = usize::try_from(u64::from_le_bytes(word.try_into().unwrap()))
                    .map_err(|_| "navy pointer overflow")?;
                if navy == 0 {
                    continue;
                }
                pointers[present] = navy;
                navy_slots[present] = first + index as u32;
                present += 1;
            }
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..present]),
                &NAVY_LAYOUT,
                &mut records[..present * NAVY_BYTES],
            ) < present * NAVY_FIELDS.len()
            {
                return Err("navy access fields unreadable");
            }
            for (record, &slot) in records[..present * NAVY_BYTES]
                .chunks_exact(NAVY_BYTES)
                .zip(&navy_slots[..present])
            {
                let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
                if word(0) != primary || word(8) != secondary {
                    return Err("navy identity mismatch");
                }
                let Some(country) = countries.at(slot)? else {
                    continue;
                };
                let array = Array::from_header(record[16..32].try_into().unwrap(), 1)
                    .ok_or("navy access modes invalid")?;
                required_rows = required_rows
                    .checked_add(array.count as usize)
                    .ok_or("navy access row demand overflow")?;
                output.ensure(required_rows)?;
                let mut start = 0;
                while start < array.count {
                    let take = (array.count - start).min(MODE_CHUNK as u32) as usize;
                    let at = array
                        .element(start, 1)
                        .ok_or("navy access modes overflow")?;
                    if pieces.len == MODE_SPANS {
                        pieces.flush(memory, &mut modes, &mut output, &mut len, &mut unread)?;
                    }
                    pieces.spans[pieces.len] = ModeSpan {
                        address: at,
                        len: take,
                        country: country.id,
                        region: start,
                    };
                    pieces.len += 1;
                    start += take as u32;
                }
            }
            first += count as u32;
        }
        pieces.flush(memory, &mut modes, &mut output, &mut len, &mut unread)?;
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use engine_native::cgame_state;
    use crate::cstrategic_navy_manager as manager;
    use engine_native_memory::memory::SliceMemory;

    const GS: usize = 0x1000;
    const COUNTRIES: [usize; 2] = [0x4000, 0x5000];
    const MANAGER: usize = 0x6000;
    const NAVIES: [usize; 2] = [0x7000, 0x7400];

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x10000];
        header(&mut bytes, GS + cgame_state::COUNTRIES, 0x3000, 4);
        p64(&mut bytes, 0x3000, u64::MAX);
        for (slot, country) in COUNTRIES.iter().enumerate() {
            p64(&mut bytes, 0x3008 + slot * 8, *country as u64);
            p64(&mut bytes, *country, ccountry::VTABLE_RVA.0 as u64);
            p32(&mut bytes, country + ccountry::ID, slot as u32 + 1);
        }
        header(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100, 4);
        for (index, value) in [0_u32, 1, 2, 3].into_iter().enumerate() {
            p32(&mut bytes, 0x3100 + index * 4, value);
        }
        p64(
            &mut bytes,
            GS + cgame_state::STRATEGIC_NAVY_MANAGER,
            MANAGER as u64,
        );
        p64(&mut bytes, MANAGER, manager::VTABLE_RVA.0 as u64);
        header(&mut bytes, MANAGER + manager::NAVIES, 0x6100, 4);
        for (slot, navy) in NAVIES.iter().enumerate() {
            p64(&mut bytes, 0x6108 + slot * 8, *navy as u64);
            p64(&mut bytes, *navy, VTABLE_RVA.0 as u64);
            p64(&mut bytes, navy + SECONDARY, SECONDARY_VTABLE_RVA.0 as u64);
            p32(&mut bytes, navy + OWNER, slot as u32 + 1);
        }
        header(&mut bytes, NAVIES[0] + ACCESS_MODES, 0x8000, 700);
        bytes[0x8000 + 18] = 1;
        bytes[0x8000 + 699] = 2;
        header(&mut bytes, NAVIES[1] + ACCESS_MODES, 0x8400, 0);
        bytes
    }

    fn run(bytes: &[u8], output: &mut Vec<AccessRow>) -> Outcome {
        access_modes(&SliceMemory { base: 0, bytes }, 0, GS, output)
    }

    #[test]
    fn only_nonzero_modes_become_rows_keyed_by_region_id() {
        let bytes = fixture();
        let mut rows = vec![AccessRow::default(); 4];
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows[..2],
            [
                AccessRow {
                    country: 1,
                    region: 18,
                    mode: 1
                },
                AccessRow {
                    country: 1,
                    region: 699,
                    mode: 2
                },
            ]
        );
    }

    #[test]
    fn a_navy_failing_its_identity_fails_instead_of_dropping_modes() {
        for (at, value) in [(NAVIES[1], 0_u64), (0x6118, 0x7800)] {
            let mut bytes = fixture();
            p64(&mut bytes, at, value);
            let mut rows = vec![AccessRow::default(); 4];
            assert_eq!(
                run(&bytes, &mut rows),
                Outcome::Unavailable {
                    reason: "navy identity mismatch"
                }
            );
        }
    }

    #[test]
    fn foreign_navy_is_recorded_as_its_slots_country() {
        let mut bytes = fixture();
        p32(&mut bytes, NAVIES[0] + OWNER, 2);
        p32(&mut bytes, NAVIES[1] + OWNER, 1);
        let mut rows = vec![AccessRow::default(); 4];
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(rows[..2].iter().map(|row| row.country).collect::<Vec<_>>(), [1, 1]);
    }
}

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod bench {
    use super::*;
    use engine_native::{
        cgame_state,
        crelation::status::bench::{Arena, Counting, countries, report},
    };
    use crate::cstrategic_navy_manager as manager;

    #[test]
    #[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
    fn bench_naval_region_access() {
        for (n, regions) in [(1_500, 300_usize), (5_000, 1_000)] {
            let mut arena = Arena::new(n * (0x80 + 0xd0 + regions + 32) + (n + 1) * 16 + 0x10_0000);
            let (game_state, objects) = countries(&mut arena, n, 0x20);
            let root = arena.alloc(0x20);
            arena.u64(
                game_state + cgame_state::STRATEGIC_NAVY_MANAGER,
                root as u64,
            );
            arena.u64(root, arena.rva(manager::VTABLE_RVA));
            let navies = arena.alloc((n + 1) * 8);
            arena.vector(root + manager::NAVIES, navies, n as u32 + 1);
            for slot in 1..=n {
                let navy = arena.alloc(0xd0);
                arena.u64(navies + slot * 8, navy as u64);
                arena.u64(navy, arena.rva(VTABLE_RVA));
                arena.u64(navy + SECONDARY, arena.rva(SECONDARY_VTABLE_RVA));
                arena.u32(navy + OWNER, slot as u32);
                let modes = arena.alloc(regions);
                arena.vector(navy + ACCESS_MODES, modes, regions as u32);
                for region in [
                    slot % regions,
                    (slot * 7 + 3) % regions,
                    (slot * 13 + 5) % regions,
                ] {
                    arena.u8(modes + region, 1 + (region % 2) as u8);
                }
            }
            let mut rows = vec![AccessRow::default(); 3 * n];
            for (label, memory) in [
                ("NavalRegionAccess", Counting::new()),
                (
                    "NavalRegionAccess (with country census)",
                    Counting::new().with_census(game_state, &objects),
                ),
            ] {
                report(label, n, &memory, || {
                    let outcome = access_modes(&memory, arena.base(), game_state, &mut rows);
                    assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
                    format!("{} rows over {regions} regions", outcome.len())
                });
            }
        }
    }
}
