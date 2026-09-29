//! Stored regional air-superiority pairs, read by raid target intel
//! (`140FD0EA0`) and success (`14157BC30`) through
//! `GetStrategicAirRegionEntry@140C456F0`: `CStrategicAir+E0`, stride A0, at
//! the region's runtime slot. Copy the pairs; never reconstruct them from wings.
//!
//! The game rewrites a region's pairs only in its phase hour. `14194ED70`
//! lists the roster slots from 1 whose region has provinces and whose
//! [`cstrategic_region::AIR_PHASE`] equals `(CURRENT_HOUR - 43,800,000) %
//! (2 * NAir::HOURS_DELAY_AFTER_EACH_COMBAT)` in signed 32-bit arithmetic
//! (`140E0FC30`). At those slots alone `140C368F0` (serially `140C57F80`)
//! clears every active country's entry and `140C56A30`, from `140C3B330`,
//! accumulates it; setup (`140C49FB0`) zeroes new entries. The hook samples at
//! the entry of `CGameState::HourlyUpdate`, whose previous call advanced to the
//! hour it reads before updating air. So after one complete read, each hour
//! re-reads only its phase slots and carries every other pair.

use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    cstrategic_air, cstrategic_region,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};

/// `GetStrategicAirRegionEntry@140C456F0`.
const SCORES: usize = 0xe0;
const STRIDE: usize = 0xa0;
/// `NAir::HOURS_DELAY_AFTER_EACH_COMBAT`, i32; `140E0FC30` doubles it.
pub const COMBAT_DELAY_RVA: usize = 0x0331_d048;
/// `140E0FC30` subtracts this from `CGameState+468` before the remainder.
const HOUR_EPOCH: i32 = 43_800_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub region: u32,
    /// Runtime slot, used only to merge carried pairs; not recorded.
    pub slot: u32,
    pub friendly: i64,
    pub enemy: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Region {
    id: u32,
    phase: i32,
    present: bool,
    provinces: bool,
}

impl Region {
    /// Whether `14194ED70` lists this slot in the hour with this phase.
    fn updated(self, slot: usize, phase: i32) -> bool {
        slot != 0 && self.provinces && self.phase == phase
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Country {
    canonical_slot: u32,
    address: usize,
    data: usize,
    rows: usize,
}

/// The last complete table and the roster it was read under, kept by the
/// caller across hours. [`grow`](Self::grow) sizes it off the game thread.
#[derive(Default)]
pub struct Held {
    hour: Option<i32>,
    period: i32,
    regions: Vec<Region>,
    region_len: usize,
    countries: Vec<Country>,
    country_len: usize,
    rows: Vec<Row>,
    row_len: usize,
    next_regions: Vec<Region>,
    next_countries: Vec<Country>,
    slots: Vec<u32>,
}

impl Held {
    /// The next walk reads every pair.
    pub fn invalidate(&mut self) {
        self.hour = None;
    }

    pub fn grow(&mut self, regions: usize, countries: usize, rows: usize) {
        for list in [&mut self.regions, &mut self.next_regions] {
            list.resize(list.len().max(regions), Region::default());
        }
        for list in [&mut self.countries, &mut self.next_countries] {
            list.resize(list.len().max(countries), Country::default());
        }
        self.slots.resize(self.slots.len().max(regions), 0);
        self.rows.resize(self.rows.len().max(rows), Row::default());
    }

    pub fn allocated_bytes(&self) -> usize {
        (self.regions.capacity() + self.next_regions.capacity()) * size_of::<Region>()
            + (self.countries.capacity() + self.next_countries.capacity()) * size_of::<Country>()
            + self.slots.capacity() * size_of::<u32>()
            + self.rows.capacity() * size_of::<Row>()
    }
}

pub struct Walk {
    pub outcome: Outcome,
    /// Live roster and strategic-air country counts, for sizing [`Held`].
    pub regions: usize,
    pub countries: usize,
    /// Whether every pair was read, rather than this hour's phase slots.
    pub full: bool,
}

/// Zero pairs are omitted, except at the roster's last region for every
/// country: a complete table so proves the country's strategic-air object,
/// and any other absent pair of that country is zero.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game: usize,
    hour: i32,
    held: &mut Held,
    output: &mut [Row],
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        regions: 0,
        countries: 0,
        full: true,
    };
    let result = read_table(memory, base, game, hour, held, output, &mut walk);
    walk.outcome = match result {
        Ok((len, period)) if len <= output.len() => {
            if len <= held.rows.len() && walk.countries <= held.next_countries.len() {
                held.rows[..len].copy_from_slice(&output[..len]);
                held.row_len = len;
                std::mem::swap(&mut held.regions, &mut held.next_regions);
                std::mem::swap(&mut held.countries, &mut held.next_countries);
                held.region_len = walk.regions;
                held.country_len = walk.countries;
                held.period = period;
                held.hour = Some(hour);
            } else {
                held.hour = None;
            }
            Outcome::Rows { len }
        }
        Ok((len, _)) => {
            held.hour = None;
            match u32::try_from(len - output.len()) {
                Ok(unread_rows) => Outcome::Truncated {
                    len: output.len(),
                    unread_rows,
                },
                Err(_) => Outcome::Unavailable {
                    reason: "raid air score count overflow",
                },
            }
        }
        Err(reason) => {
            held.hour = None;
            Outcome::Unavailable { reason }
        }
    };
    walk
}

fn read_table(
    memory: &impl Memory,
    base: usize,
    game: usize,
    hour: i32,
    held: &mut Held,
    output: &mut [Row],
    walk: &mut Walk,
) -> Result<(usize, i32), &'static str> {
    let delay = base
        .checked_add(COMBAT_DELAY_RVA)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("air combat delay define unreadable")?;
    // `140E0FC30`'s IDIV faults where this has no remainder.
    let period = delay.wrapping_add(delay);
    let phase = hour
        .wrapping_sub(HOUR_EPOCH)
        .checked_rem(period)
        .ok_or("air combat delay define has no phase period")?;
    let roster = game
        .checked_add(cgame_state::STRATEGIC_REGIONS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("raid air region roster unreadable")?;
    walk.regions = roster.count as usize;
    if walk.regions > held.next_regions.len() || walk.regions > held.slots.len() {
        return Err("raid air region scratch too small");
    }
    read_regions(memory, base, roster, &mut held.next_regions[..walk.regions])?;
    let regions = &held.next_regions[..walk.regions];
    let last = regions
        .iter()
        .rposition(|region| region.present)
        .ok_or("raid air region roster empty")?;
    let mut slot_len = 0;
    for (slot, region) in regions.iter().enumerate() {
        if region.updated(slot, phase) {
            held.slots[slot_len] = slot as u32;
            slot_len += 1;
        }
    }
    let slots = &held.slots[..slot_len];
    let carried = held.hour == Some(hour.wrapping_sub(1))
        && held.period == period
        && held.regions[..held.region_len] == *regions;
    walk.full = !carried;
    let previous = &held.countries[..if carried { held.country_len } else { 0 }];
    let previous_rows = &held.rows[..held.row_len];
    let next = &mut held.next_countries;
    let (mut len, mut country, mut row) = (0, 0, 0);
    let mut countries = 0;
    cstrategic_air::visit(memory, base, game, |root| {
        let scores = root
            .address
            .checked_add(SCORES)
            .and_then(|at| Array::read(memory, at, STRIDE))
            .ok_or("raid air score array unreadable")?;
        if scores.count as usize != regions.len() {
            return Err("raid air score count differs from region roster");
        }
        while previous
            .get(country)
            .is_some_and(|held| held.canonical_slot < root.canonical_slot)
        {
            row += previous[country].rows;
            country += 1;
        }
        let first = len;
        let target = Target {
            country: root.country,
            last,
            regions,
        };
        match previous.get(country).filter(|kept| {
            kept.canonical_slot == root.canonical_slot
                && kept.address == root.address
                && kept.data == scores.data
        }) {
            Some(kept) => carry(
                memory,
                scores,
                &target,
                slots,
                previous_rows
                    .get(row..row + kept.rows)
                    .ok_or("held raid air scores inconsistent")?,
                phase,
                output,
                &mut len,
            )?,
            None => read_all(memory, scores, &target, output, &mut len)?,
        }
        if let Some(next) = next.get_mut(countries) {
            *next = Country {
                canonical_slot: root.canonical_slot,
                address: root.address,
                data: scores.data,
                rows: len - first,
            };
        }
        countries += 1;
        Ok(())
    })?;
    walk.countries = countries;
    Ok((len, period))
}

const REGION_FIELDS: [RecordField; 4] = [
    field(0, 0, 8),
    field(cstrategic_region::PROVINCES_COUNT, 8, 4),
    field(
        cstrategic_region::ID,
        12,
        cstrategic_region::RUNTIME_SLOT + 4 - cstrategic_region::ID,
    ),
    field(cstrategic_region::AIR_PHASE, 24, 4),
];
const REGION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&REGION_FIELDS, 28).expect("valid region layout");

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

/// `14194ED70` dereferences every slot from 1; slot 0 may be null.
fn read_regions(
    memory: &impl Memory,
    base: usize,
    roster: Array,
    regions: &mut [Region],
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    let expected = base
        .checked_add(cstrategic_region::VTABLE_RVA)
        .ok_or("raid air region vtable address overflow")? as u64;
    let mut raw = [0_u8; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut slots = [0_usize; BATCH];
    let mut bytes = [0_u8; BATCH * 28];
    for first in (0..regions.len()).step_by(BATCH) {
        let count = (regions.len() - first).min(BATCH);
        let source = roster
            .element(first as u32, 8)
            .ok_or("raid air region roster address overflow")?;
        if !memory.copy(source, &mut raw[..count * 8]) {
            return Err("raid air region pointers unreadable");
        }
        let mut live = 0;
        for (index, pointer) in raw[..count * 8].chunks_exact(8).enumerate() {
            let slot = first + index;
            regions[slot] = Region::default();
            let pointer = usize::try_from(u64::from_le_bytes(pointer.try_into().unwrap()))
                .map_err(|_| "raid air region pointer overflow")?;
            if pointer == 0 {
                if slot != 0 {
                    return Err("raid air region pointer null");
                }
                continue;
            }
            pointers[live] = pointer;
            slots[live] = slot;
            live += 1;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..live]),
            &REGION_LAYOUT,
            &mut bytes[..live * 28],
        );
        if copied != live * REGION_FIELDS.len() {
            return Err("raid air region unreadable");
        }
        for (record, &slot) in bytes[..live * 28].chunks_exact(28).zip(&slots[..live]) {
            let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != expected {
                return Err("raid air region vtable mismatch");
            }
            if word(20) as usize != slot {
                return Err("raid air region runtime slot differs from roster");
            }
            regions[slot] = Region {
                id: word(12),
                phase: word(24) as i32,
                present: true,
                provinces: word(8) != 0,
            };
        }
    }
    Ok(())
}

struct Target<'a> {
    country: u32,
    last: usize,
    regions: &'a [Region],
}

impl Target<'_> {
    fn push(
        &self,
        slot: usize,
        pair: &[u8],
        output: &mut [Row],
        len: &mut usize,
    ) -> Result<(), &'static str> {
        let friendly = i64::from_le_bytes(pair[..8].try_into().unwrap());
        let enemy = i64::from_le_bytes(pair[8..16].try_into().unwrap());
        if friendly == 0 && enemy == 0 && slot != self.last {
            return Ok(());
        }
        push(
            output,
            len,
            Row {
                country: self.country,
                region: self.regions[slot].id,
                slot: slot as u32,
                friendly,
                enemy,
            },
        )
    }
}

fn push(output: &mut [Row], len: &mut usize, row: Row) -> Result<(), &'static str> {
    if let Some(slot) = output.get_mut(*len) {
        *slot = row;
    }
    *len = len.checked_add(1).ok_or("raid air score count overflow")?;
    Ok(())
}

const PAIR: [RecordField; 1] = [field(0, 0, 16)];
const PAIR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PAIR, 16).expect("valid record layout");
const BATCH: usize = 128;

fn read_all(
    memory: &impl Memory,
    scores: Array,
    target: &Target<'_>,
    output: &mut [Row],
    len: &mut usize,
) -> Result<(), &'static str> {
    let mut bytes = [0; BATCH * 16];
    for first in (0..scores.count as usize).step_by(BATCH) {
        let count = (scores.count as usize - first).min(BATCH);
        let source = scores
            .element(first as u32, STRIDE)
            .ok_or("raid air score address overflow")?;
        let copied = memory.copy_records(
            RecordSources::Strided {
                base: source,
                stride: STRIDE,
                count,
            },
            &PAIR_LAYOUT,
            &mut bytes[..count * 16],
        );
        if copied != count {
            return Err("raid air score pair unreadable");
        }
        for (index, pair) in bytes[..count * 16].chunks_exact(16).enumerate() {
            if target.regions[first + index].present {
                target.push(first + index, pair, output, len)?;
            }
        }
    }
    Ok(())
}

/// Reads this hour's phase `slots` and keeps every other held pair.
#[allow(clippy::too_many_arguments)]
fn carry(
    memory: &impl Memory,
    scores: Array,
    target: &Target<'_>,
    slots: &[u32],
    held: &[Row],
    phase: i32,
    output: &mut [Row],
    len: &mut usize,
) -> Result<(), &'static str> {
    let mut held = held
        .iter()
        .filter(|row| !target.regions[row.slot as usize].updated(row.slot as usize, phase))
        .peekable();
    let mut addresses = [0_usize; BATCH];
    let mut bytes = [0; BATCH * 16];
    for batch in slots.chunks(BATCH) {
        for (address, slot) in addresses.iter_mut().zip(batch) {
            *address = scores
                .element(*slot, STRIDE)
                .ok_or("raid air score address overflow")?;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..batch.len()]),
            &PAIR_LAYOUT,
            &mut bytes[..batch.len() * 16],
        );
        if copied != batch.len() {
            return Err("raid air score pair unreadable");
        }
        for (slot, pair) in batch.iter().zip(bytes[..batch.len() * 16].chunks_exact(16)) {
            while let Some(row) = held.next_if(|row| row.slot < *slot) {
                push(output, len, *row)?;
            }
            target.push(*slot as usize, pair, output, len)?;
        }
    }
    for row in held {
        push(output, len, *row)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const MEMORY: usize = 0x1000_0000;
    const BASE: usize = MEMORY + 0x100 - COMBAT_DELAY_RVA;
    const GAME: usize = MEMORY + 0x1000;
    /// `(HOUR - 43,800,000) % 8 == 3` with the vanilla delay of 4.
    const HOUR: i32 = 43_800_000 + 8 * 1_000 + 3;

    struct World {
        bytes: Vec<u8>,
        region_list: usize,
        region_objects: usize,
        airs: usize,
        stride: usize,
    }

    impl World {
        /// Slot 0 of the roster is null; every region has provinces and the
        /// phase `AddProvince` stores, `slot % 8`.
        fn new(countries: usize, regions: usize) -> Self {
            let slots = countries + 1;
            let countries_at = 0x3000;
            let links_at = countries_at + slots * 8;
            let air_at = links_at + slots * 4;
            let region_list = air_at + slots * 8;
            let region_objects = region_list + regions * 8;
            let airs = region_objects + regions * 0x200;
            let stride = 0x200 + regions * STRIDE;
            let mut world = Self {
                bytes: vec![0; airs + countries * stride],
                region_list,
                region_objects,
                airs,
                stride,
            };
            world.put32(0x100, 4);
            world.header(0x1000 + cgame_state::COUNTRIES, countries_at, slots);
            world.header(0x1000 + cgame_state::COUNTRY_LINKS, links_at, slots);
            world.put(0x1000 + cgame_state::STRATEGIC_AIR_MANAGER, MEMORY + 0x2000);
            world.put(0x2000, BASE + crate::cstrategic_air_manager::VTABLE_RVA);
            world.header(
                0x2000 + crate::cstrategic_air_manager::COUNTRY_AIR,
                air_at,
                slots,
            );
            world.header(
                0x1000 + cgame_state::STRATEGIC_REGIONS,
                region_list,
                regions,
            );
            for slot in 1..regions {
                let region = world.region(slot);
                world.put(region_list + slot * 8, MEMORY + region);
                world.put(region, BASE + cstrategic_region::VTABLE_RVA);
                world.put32(region + cstrategic_region::PROVINCES_COUNT, 1);
                world.put32(region + cstrategic_region::ID, 1_000 + slot as u32);
                world.put32(region + cstrategic_region::RUNTIME_SLOT, slot as u32);
                world.put32(region + cstrategic_region::AIR_PHASE, slot as u32 % 8);
            }
            for slot in 1..slots {
                let air = world.air(slot);
                world.put(countries_at + slot * 8, MEMORY + air - 0x20);
                world.put32(links_at + slot * 4, slot as u32);
                world.put(air - 0x20, BASE + crate::ccountry::VTABLE_RVA);
                world.put(air - 0x20 + crate::ccountry::ID, slot);
                world.put(air_at + slot * 8, MEMORY + air);
                world.put(air, BASE + cstrategic_air::VTABLE_RVA);
                world.put(air + cstrategic_air::COUNTRY, slot);
                world.header(air + SCORES, air + 0x1e0, regions);
            }
            world
        }

        fn put(&mut self, at: usize, value: usize) {
            self.bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }

        fn put32(&mut self, at: usize, value: u32) {
            self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn header(&mut self, at: usize, data: usize, count: usize) {
            self.put(at, MEMORY + data);
            self.put(at + 8, count << 32 | count);
        }

        fn air(&self, country: usize) -> usize {
            self.airs + (country - 1) * self.stride + 0x20
        }

        fn region(&self, slot: usize) -> usize {
            self.region_objects + slot * 0x200
        }

        fn pair(&mut self, country: usize, slot: usize, friendly: i64, enemy: i64) {
            let data = u64::from_le_bytes(
                self.bytes[self.air(country) + SCORES..][..8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let at = data - MEMORY + slot * STRIDE;
            self.bytes[at..at + 8].copy_from_slice(&friendly.to_le_bytes());
            self.bytes[at + 8..at + 16].copy_from_slice(&enemy.to_le_bytes());
        }

        fn walk(&self, hour: i32, held: &mut Held, output: &mut [Row]) -> Walk {
            let memory = SliceMemory {
                base: MEMORY,
                bytes: &self.bytes,
            };
            walk(&memory, BASE, GAME, hour, held, output)
        }
    }

    fn held(regions: usize, countries: usize, rows: usize) -> Held {
        let mut held = Held::default();
        held.grow(regions, countries, rows);
        held
    }

    fn pairs(walk: &Walk, rows: &[Row]) -> Vec<(u32, u32, i64, i64)> {
        rows[..walk.outcome.len()]
            .iter()
            .map(|row| (row.country, row.region, row.friendly, row.enemy))
            .collect()
    }

    #[test]
    fn phase_hours_reread_only_the_slots_the_game_rewrote() {
        let mut world = World::new(3, 20);
        world.pair(1, 3, 5, 0);
        world.pair(2, 12, 0, -7);
        let mut held = held(20, 3, 16);
        let mut rows = [Row::default(); 16];
        let first = world.walk(HOUR - 1, &mut held, &mut rows);
        assert!(first.full);
        assert_eq!((first.regions, first.countries), (20, 3));
        assert_eq!(
            pairs(&first, &rows),
            [
                (1, 1_003, 5, 0),
                (1, 1_019, 0, 0),
                (2, 1_012, 0, -7),
                (2, 1_019, 0, 0),
                (3, 1_019, 0, 0),
            ]
        );
        // HOUR rewrites slots 3, 11 and 19. A change anywhere else cannot
        // come from the game; reading it would prove a full read.
        world.pair(1, 3, 6, 0);
        world.pair(1, 5, 9, 9);
        world.pair(2, 11, 1, 2);
        world.pair(3, 19, i64::MIN, i64::MAX);
        let next = world.walk(HOUR, &mut held, &mut rows);
        assert!(!next.full);
        assert_eq!(
            pairs(&next, &rows),
            [
                (1, 1_003, 6, 0),
                (1, 1_019, 0, 0),
                (2, 1_011, 1, 2),
                (2, 1_012, 0, -7),
                (2, 1_019, 0, 0),
                (3, 1_019, i64::MIN, i64::MAX),
            ]
        );
        // Each slot is reread in its own phase hour; a rewritten zero pair
        // then leaves the table, and the witness stays.
        world.pair(1, 3, 0, 0);
        world.pair(3, 19, 0, 0);
        world.pair(1, 4, 1, 1);
        let walked = world.walk(HOUR + 1, &mut held, &mut rows);
        let got = pairs(&walked, &rows);
        assert!(got.contains(&(1, 1_004, 1, 1)) && got.contains(&(1, 1_003, 6, 0)));
        assert!(!got.contains(&(1, 1_005, 9, 9)) && got.contains(&(3, 1_019, i64::MIN, i64::MAX)));
        for hour in HOUR + 2..=HOUR + 8 {
            assert!(!world.walk(hour, &mut held, &mut rows).full);
        }
        let walked = world.walk(HOUR + 9, &mut held, &mut rows);
        assert!(!walked.full);
        assert_eq!(
            pairs(&walked, &rows),
            [
                (1, 1_004, 1, 1),
                (1, 1_005, 9, 9),
                (1, 1_019, 0, 0),
                (2, 1_011, 1, 2),
                (2, 1_012, 0, -7),
                (2, 1_019, 0, 0),
                (3, 1_019, 0, 0),
            ]
        );
        // A gap in the hours reads everything again.
        assert!(world.walk(HOUR + 11, &mut held, &mut rows).full);
    }

    #[test]
    fn each_changed_premise_rereads_what_it_cannot_carry() {
        let mut world = World::new(2, 12);
        let mut held = held(12, 2, 16);
        let mut rows = [Row::default(); 16];
        assert!(world.walk(HOUR - 1, &mut held, &mut rows).full);
        assert!(!world.walk(HOUR, &mut held, &mut rows).full);
        // The define is read live: a different period rereads everything.
        world.put32(0x100, 3);
        assert!(world.walk(HOUR + 1, &mut held, &mut rows).full);
        world.put32(0x100, 4);
        assert!(world.walk(HOUR + 2, &mut held, &mut rows).full);
        let region = world.region(7);
        world.put32(region + cstrategic_region::AIR_PHASE, 6);
        assert!(world.walk(HOUR + 3, &mut held, &mut rows).full);
        assert!(!world.walk(HOUR + 4, &mut held, &mut rows).full);
        // A moved score array rereads its own country alone.
        world.pair(1, 6, 3, 0);
        world.pair(2, 6, 4, 0);
        let air = world.air(1);
        let old = air + 0x1e0;
        let moved = world.bytes.len();
        world.bytes.extend_from_within(old..old + 12 * STRIDE);
        world.header(air + SCORES, moved, 12);
        let walked = world.walk(HOUR + 5, &mut held, &mut rows);
        assert!(!walked.full);
        let got = pairs(&walked, &rows);
        assert!(got.contains(&(1, 1_006, 3, 0)) && !got.contains(&(2, 1_006, 4, 0)));
    }

    #[test]
    fn capacity_failures_and_bad_rosters_leave_nothing_to_carry() {
        let mut world = World::new(2, 10);
        world.pair(1, 2, 1, 1);
        let mut rows = [Row::default(); 8];
        let walked = world.walk(HOUR, &mut held(9, 2, 8), &mut rows);
        assert_eq!(walked.regions, 10);
        assert_eq!(
            walked.outcome,
            Outcome::Unavailable {
                reason: "raid air region scratch too small"
            }
        );
        let mut held = held(10, 2, 8);
        assert_eq!(
            world.walk(HOUR, &mut held, &mut rows[..2]).outcome,
            Outcome::Truncated {
                len: 2,
                unread_rows: 1
            }
        );
        assert!(world.walk(HOUR + 1, &mut held, &mut rows).full);
        // Rows the caller cannot hold are published; the next hour is full.
        let mut unheld = self::held(10, 1, 8);
        assert_eq!(
            world.walk(HOUR, &mut unheld, &mut rows).outcome,
            Outcome::Rows { len: 3 }
        );
        assert!(world.walk(HOUR + 1, &mut unheld, &mut rows).full);
        let region = world.region(4);
        world.put32(region + cstrategic_region::RUNTIME_SLOT, 5);
        assert_eq!(
            world.walk(HOUR + 2, &mut held, &mut rows).outcome,
            Outcome::Unavailable {
                reason: "raid air region runtime slot differs from roster"
            }
        );
        world.put32(region + cstrategic_region::RUNTIME_SLOT, 4);
        world.bytes[region] ^= 1;
        assert_eq!(
            world.walk(HOUR + 3, &mut held, &mut rows).outcome,
            Outcome::Unavailable {
                reason: "raid air region vtable mismatch"
            }
        );
        world.bytes[region] ^= 1;
        let pointer = world.region_list + 4 * 8;
        world.put(pointer, 0);
        assert_eq!(
            world.walk(HOUR + 4, &mut held, &mut rows).outcome,
            Outcome::Unavailable {
                reason: "raid air region pointer null"
            }
        );
        world.put(pointer, MEMORY + region);
        world.put32(0x100, 0);
        assert_eq!(
            world.walk(HOUR + 5, &mut held, &mut rows).outcome,
            Outcome::Unavailable {
                reason: "air combat delay define has no phase period"
            }
        );
        world.put32(0x100, 4);
        assert!(world.walk(HOUR + 6, &mut held, &mut rows).full);
        assert!(!world.walk(HOUR + 7, &mut held, &mut rows).full);
    }

    #[test]
    #[ignore = "synthetic acquisition cost; run release and under affinity 0xF"]
    fn acquisition_cost() {
        let variable = |name, default| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let countries = variable("BATTLE_PLAN_RAID_SCORE_COUNTRIES", 1_500);
        let regions = variable("BATTLE_PLAN_RAID_SCORE_REGIONS", 305);
        let mut world = World::new(countries, regions);
        // A sparse war: every twentieth country holds ten nonzero pairs.
        for country in (1..=countries).step_by(20) {
            for slot in (1..regions).step_by(regions / 10) {
                world.pair(country, slot, country as i64, slot as i64);
            }
        }
        let capacity = countries * 12;
        let mut held = held(regions, countries, capacity);
        let mut rows = vec![Row::default(); capacity];
        let (mut full, mut steady) = (Vec::new(), Vec::new());
        for sample in 0..40 {
            let hour = HOUR + 16 * sample;
            held.invalidate();
            for (hour, times) in [(hour, &mut full), (hour + 1, &mut steady)] {
                let begin = std::time::Instant::now();
                let walked = world.walk(hour, &mut held, &mut rows);
                times.push(begin.elapsed().as_secs_f64() * 1000.0);
                assert!(matches!(walked.outcome, Outcome::Rows { .. }));
                std::hint::black_box(&rows);
            }
        }
        full.sort_by(f64::total_cmp);
        steady.sort_by(f64::total_cmp);
        println!(
            "raid air scores countries={countries} regions={regions} full_median_ms={:.3} full_max_ms={:.3} steady_median_ms={:.3} steady_max_ms={:.3} held_bytes={}",
            full[20],
            full[39],
            steady[20],
            steady[39],
            held.allocated_bytes()
        );
    }
}
