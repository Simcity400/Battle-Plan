//! Stored regional air-superiority weights and active-mission counters used by raids and the Air map mode.
//! Copy these region-slot pairs rather than reconstructing them from wings.
//!
//! The game rewrites a region's pairs only in its phase hour. Its phase roster
//! lists slots from 1 whose region has provinces and whose
//! [`cstrategic_region::AIR_PHASE`] equals the signed 32-bit remainder of
//! `CURRENT_HOUR - 43,800,000` over twice
//! `NAir::HOURS_DELAY_AFTER_EACH_COMBAT`. At those slots alone the
//! strategic-air phase pass clears every active country's entry and the
//! regional update accumulates it and rebuilds the aircraft spotting scale;
//! setup zeroes new entries. The hook samples
//! at hourly-update entry, whose previous call advanced to the hour it reads
//! before updating air. After one complete read, each hour therefore re-reads
//! only its phase slots and carries every other pair.
//!
//! One other writer clears pairs off phase: country strategic-air reset, called
//! during dynamic country reuse, zeroes every region entry in place after
//! clearing its site lists through two clearing operations. The caller
//! certifies the carry with a token those two sites move ([`Held::certify`]),
//! and a country is carried only under the raw id it was read with.

use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    cstrategic_air, cstrategic_region,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::{Buffer, Outcome},
};

/// Strategic-air region-entry selector.
const SCORES: usize = 0xe0;
const STRIDE: usize = 0xa0;
/// The entry's movement impact, which the land movement speed scales by the enemy's
/// share of the pair; the phase pass rewrites it with the pair.
const IMPACT: usize = 0x18;
/// Active-mission counters cleared and rebuilt in the same regional phase as the weights.
const FRIENDLY_MISSIONS: usize = 0x28;
const ENEMY_MISSIONS: usize = 0x2a;
/// The weights, movement impact and active-mission counters in one guarded read.
const ENTRY: usize = ENEMY_MISSIONS + 2;
/// Raw fixed5 normalization applied to regional aircraft spotting.
const AIR_SPOTTING_SCALE: usize = 0x80;
/// Loaded signed i32 combat delay, doubled for the phase period.
pub const COMBAT_DELAY_RVA: usize = 0x033e_0bbc;
/// Epoch subtracted from the game hour before signed remainder.
const HOUR_EPOCH: i32 = 43_800_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub region: u32,
    /// Runtime slot, used only to merge carried pairs; not recorded.
    pub slot: u32,
    pub friendly: i64,
    pub enemy: i64,
    /// The entry's movement impact ([`IMPACT`]).
    pub impact: i64,
    pub friendly_missions: u16,
    pub enemy_missions: u16,
    pub air_spotting_scale: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Region {
    id: u32,
    phase: i32,
    present: bool,
    provinces: bool,
}

impl Region {
    /// Whether this region slot is reached in the current phase hour.
    fn updated(self, slot: usize, phase: i32) -> bool {
        slot != 0 && self.provinces && self.phase == phase
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Country {
    canonical_slot: u32,
    /// The raw country id the pairs were read under.
    id: u32,
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
    /// The token the held table was read under.
    token: Option<u64>,
}

impl Held {
    /// The next walk reads every pair.
    pub fn invalidate(&mut self) {
        self.hour = None;
    }

    /// The next walk carries off-phase pairs only under `token`, which the
    /// caller takes before it from the writers that clear pairs in place (both
    /// site-list clears called by strategic-air reset): an absent token, or one
    /// other than the held table's, reads every pair.
    pub fn certify(&mut self, token: Option<u64>) {
        if token.is_none() || token != self.token {
            self.hour = None;
        }
        self.token = token;
    }

    pub fn grow(&mut self, regions: usize, countries: usize, rows: usize) -> std::io::Result<()> {
        for list in [&mut self.regions, &mut self.next_regions] {
            let len = list.len().max(regions);
            crate::allocation::resize(list, len, Region::default())?;
        }
        for list in [&mut self.countries, &mut self.next_countries] {
            let len = list.len().max(countries);
            crate::allocation::resize(list, len, Country::default())?;
        }
        let len = self.slots.len().max(regions);
        crate::allocation::resize(&mut self.slots, len, 0)?;
        let len = self.rows.len().max(rows);
        crate::allocation::resize(&mut self.rows, len, Row::default())?;
        Ok(())
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

/// Entries with zero weights, active-mission counters and an observed zero
/// aircraft spotting scale are omitted, except at the roster's last region for every
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
    walk_buffer(memory, base, game, hour, held, Buffer::Fixed(output))
}

/// Worker-only growth follows the original roster, score headers and retained
/// nonzero pairs; no capacity failure restarts a native observation.
pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game: usize,
    hour: i32,
    held: &mut Held,
    output: &mut Vec<Row>,
) -> Walk {
    walk_buffer(memory, base, game, hour, held, Buffer::Growing(output))
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    hour: i32,
    held: &mut Held,
    mut output: Buffer<'_, Row>,
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        regions: 0,
        countries: 0,
        full: true,
    };
    let result =
        read_table(memory, base, game, hour, held, &mut output, &mut walk).and_then(|result| {
            if output.is_growing() {
                Buffer::Growing(&mut held.rows).ensure(result.0)?;
            }
            Ok(result)
        });
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
    output: &mut Buffer<'_, Row>,
    walk: &mut Walk,
) -> Result<(usize, i32), &'static str> {
    let delay = base
        .checked_add(COMBAT_DELAY_RVA)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("air combat delay define unreadable")?;
    // Signed division traps when no remainder exists.
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
    if output.is_growing() {
        // Setup sizes regional entries from the live strategic-region roster.
        held.grow(walk.regions, held.next_countries.len(), held.rows.len())
            .map_err(|_| "capture buffer allocation failed")?;
    }
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
    let mut countries = 0usize;
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
                && kept.id == root.country
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
        if output.is_growing() {
            Buffer::Growing(&mut *next).ensure(
                countries
                    .checked_add(1)
                    .ok_or("raid air country count overflow")?,
            )?;
        }
        if let Some(next) = next.get_mut(countries) {
            *next = Country {
                canonical_slot: root.canonical_slot,
                id: root.country,
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

/// Every positive region slot is dereferenced; slot zero may be null.
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
        air_spotting_scale: Option<i64>,
        output: &mut Buffer<'_, Row>,
        len: &mut usize,
    ) -> Result<(), &'static str> {
        let friendly = i64::from_le_bytes(pair[..8].try_into().unwrap());
        let enemy = i64::from_le_bytes(pair[8..16].try_into().unwrap());
        let impact = i64::from_le_bytes(pair[IMPACT..IMPACT + 8].try_into().unwrap());
        let friendly_missions = u16::from_le_bytes(
            pair[FRIENDLY_MISSIONS..FRIENDLY_MISSIONS + 2]
                .try_into()
                .unwrap(),
        );
        let enemy_missions = u16::from_le_bytes(pair[ENEMY_MISSIONS..ENTRY].try_into().unwrap());
        if friendly == 0
            && enemy == 0
            && friendly_missions == 0
            && enemy_missions == 0
            && air_spotting_scale == Some(0)
            && slot != self.last
        {
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
                impact,
                friendly_missions,
                enemy_missions,
                air_spotting_scale,
            },
        )
    }
}

fn push(output: &mut Buffer<'_, Row>, len: &mut usize, row: Row) -> Result<(), &'static str> {
    output.ensure(len.checked_add(1).ok_or("raid air score count overflow")?)?;
    if let Some(slot) = output.get_mut(*len) {
        *slot = row;
    }
    *len = len.checked_add(1).ok_or("raid air score count overflow")?;
    Ok(())
}

const PAIR: [RecordField; 1] = [field(0, 0, ENTRY)];
const PAIR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PAIR, ENTRY).expect("valid record layout");
const BATCH: usize = 128;
const SCALE_FIELD: [RecordField; 1] = [field(AIR_SPOTTING_SCALE, 0, 8)];
const SCALE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SCALE_FIELD, 8).expect("valid regional spotting scale layout");

fn read_scales(memory: &impl Memory, addresses: &[usize]) -> [Option<i64>; BATCH] {
    let mut bytes = [0_u8; BATCH * 8];
    let mut complete = [false; BATCH];
    crate::memory::gather_records(
        memory,
        addresses,
        &SCALE_LAYOUT,
        &mut bytes[..addresses.len() * 8],
        &mut complete[..addresses.len()],
    );
    std::array::from_fn(|index| {
        complete[index]
            .then(|| i64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap()))
    })
}

fn read_all(
    memory: &impl Memory,
    scores: Array,
    target: &Target<'_>,
    output: &mut Buffer<'_, Row>,
    len: &mut usize,
) -> Result<(), &'static str> {
    let mut bytes = [0; BATCH * ENTRY];
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
            &mut bytes[..count * ENTRY],
        );
        if copied != count {
            return Err("raid air score pair unreadable");
        }
        let mut addresses = [0_usize; BATCH];
        for (index, address) in addresses[..count].iter_mut().enumerate() {
            *address = scores
                .element((first + index) as u32, STRIDE)
                .ok_or("raid air scale address overflow")?;
        }
        let scales = read_scales(memory, &addresses[..count]);
        for (index, pair) in bytes[..count * ENTRY].chunks_exact(ENTRY).enumerate() {
            if target.regions[first + index].present {
                target.push(first + index, pair, scales[index], output, len)?;
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
    output: &mut Buffer<'_, Row>,
    len: &mut usize,
) -> Result<(), &'static str> {
    let mut held = held
        .iter()
        .filter(|row| !target.regions[row.slot as usize].updated(row.slot as usize, phase))
        .peekable();
    let mut addresses = [0_usize; BATCH];
    let mut bytes = [0; BATCH * ENTRY];
    for batch in slots.chunks(BATCH) {
        for (address, slot) in addresses.iter_mut().zip(batch) {
            *address = scores
                .element(*slot, STRIDE)
                .ok_or("raid air score address overflow")?;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..batch.len()]),
            &PAIR_LAYOUT,
            &mut bytes[..batch.len() * ENTRY],
        );
        if copied != batch.len() {
            return Err("raid air score pair unreadable");
        }
        let scales = read_scales(memory, &addresses[..batch.len()]);
        for (index, (slot, pair)) in batch
            .iter()
            .zip(bytes[..batch.len() * ENTRY].chunks_exact(ENTRY))
            .enumerate()
        {
            while let Some(row) = held.next_if(|row| row.slot < *slot) {
                push(output, len, *row)?;
            }
            target.push(*slot as usize, pair, scales[index], output, len)?;
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
    /// Synthetic phase-three hour with combat delay four and period eight.
    const HOUR: i32 = 43_800_000 + 8 * 1_000 + 3;

    struct World {
        bytes: Vec<u8>,
        region_list: usize,
        region_objects: usize,
        airs: usize,
        stride: usize,
    }

    impl World {
        /// Slot zero is null; fixture regions have provinces and phase equal to slot modulo eight.
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

        fn impact(&mut self, country: usize, slot: usize, impact: i64) {
            let data = u64::from_le_bytes(
                self.bytes[self.air(country) + SCORES..][..8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let at = data - MEMORY + slot * STRIDE + IMPACT;
            self.bytes[at..at + 8].copy_from_slice(&impact.to_le_bytes());
        }

        fn spotting_scale(&mut self, country: usize, slot: usize, value: i64) {
            let data = u64::from_le_bytes(
                self.bytes[self.air(country) + SCORES..][..8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let at = data - MEMORY + slot * STRIDE + AIR_SPOTTING_SCALE;
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        fn mission_counts(&mut self, country: usize, slot: usize, counts: [u16; 2]) {
            let data = u64::from_le_bytes(
                self.bytes[self.air(country) + SCORES..][..8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let at = data - MEMORY + slot * STRIDE + FRIENDLY_MISSIONS;
            self.bytes[at..at + 2].copy_from_slice(&counts[0].to_le_bytes());
            self.bytes[at + 2..at + 4].copy_from_slice(&counts[1].to_le_bytes());
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
        held.grow(regions, countries, rows).unwrap();
        held
    }

    fn pairs(walk: &Walk, rows: &[Row]) -> Vec<(u32, u32, i64, i64)> {
        rows[..walk.outcome.len()]
            .iter()
            .map(|row| (row.country, row.region, row.friendly, row.enemy))
            .collect()
    }

    #[test]
    fn zero_weight_mission_counters_are_kept_and_carried_until_their_region_phase() {
        let mut world = World::new(1, 17);
        let mut held = held(17, 1, 17);
        held.certify(Some(1));
        let mut output = vec![Row::default(); 17];
        world.mission_counts(1, 5, [65535, 3]);
        let first = world.walk(HOUR, &mut held, &mut output);
        assert!(first.full);
        let row = output[..first.outcome.len()]
            .iter()
            .find(|row| row.slot == 5)
            .unwrap();
        assert_eq!(
            (
                row.friendly,
                row.enemy,
                row.friendly_missions,
                row.enemy_missions
            ),
            (0, 0, 65535, 3)
        );
        world.mission_counts(1, 5, [7, 8]);
        let repeated = world.walk(HOUR + 1, &mut held, &mut output);
        assert!(!repeated.full);
        assert_eq!(
            output[..repeated.outcome.len()]
                .iter()
                .find(|r| r.slot == 5)
                .unwrap()
                .friendly_missions,
            65535
        );
        let next = world.walk(HOUR + 2, &mut held, &mut output);
        assert!(!next.full);
        let row = output[..next.outcome.len()]
            .iter()
            .find(|row| row.slot == 5)
            .unwrap();
        assert_eq!((row.friendly_missions, row.enemy_missions), (7, 8));
        world.mission_counts(1, 5, [0, 0]);
        let cleared = world.walk(HOUR + 10, &mut held, &mut output);
        assert!(!output[..cleared.outcome.len()].iter().any(|r| r.slot == 5));
    }

    #[test]
    fn scale_only_regions_are_sparse_and_carry_until_the_same_native_phase() {
        let mut world = World::new(1, 17);
        let mut held = held(17, 1, 17);
        held.certify(Some(1));
        let mut output = vec![Row::default(); 17];
        world.spotting_scale(1, 5, -7);
        let first = world.walk(HOUR, &mut held, &mut output);
        assert_eq!(
            output[..first.outcome.len()]
                .iter()
                .find(|r| r.slot == 5)
                .unwrap()
                .air_spotting_scale,
            Some(-7)
        );
        world.spotting_scale(1, 5, 100_000);
        let next = world.walk(HOUR + 1, &mut held, &mut output);
        assert!(!next.full);
        assert_eq!(
            output[..next.outcome.len()]
                .iter()
                .find(|r| r.slot == 5)
                .unwrap()
                .air_spotting_scale,
            Some(-7)
        );
        let changed = world.walk(HOUR + 2, &mut held, &mut output);
        assert_eq!(
            output[..changed.outcome.len()]
                .iter()
                .find(|r| r.slot == 5)
                .unwrap()
                .air_spotting_scale,
            Some(100_000)
        );
        world.spotting_scale(1, 5, 0);
        let cleared = world.walk(HOUR + 10, &mut held, &mut output);
        assert!(!output[..cleared.outcome.len()].iter().any(|r| r.slot == 5));
    }

    #[test]
    fn a_failed_optional_scale_preserves_scores_and_is_not_retried_off_phase() {
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            at: usize,
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, at: usize, to: &mut [u8]) -> bool {
                if at == self.at {
                    self.attempts.set(self.attempts.get() + 1);
                    return false;
                }
                self.memory.copy(at, to)
            }
        }
        let mut world = World::new(1, 17);
        world.pair(1, 5, 42, 7);
        let data = u64::from_le_bytes(
            world.bytes[world.air(1) + SCORES..][..8]
                .try_into()
                .unwrap(),
        ) as usize;
        let memory = Fault {
            memory: SliceMemory {
                base: MEMORY,
                bytes: &world.bytes,
            },
            at: data + 5 * STRIDE + AIR_SPOTTING_SCALE,
            attempts: Default::default(),
        };
        let mut held = held(17, 1, 17);
        held.certify(Some(1));
        let mut output = vec![Row::default(); 17];
        let first = walk(&memory, BASE, GAME, HOUR, &mut held, &mut output);
        assert!(matches!(first.outcome, Outcome::Rows { .. }));
        let row = output[..first.outcome.len()]
            .iter()
            .find(|r| r.slot == 5)
            .unwrap();
        assert_eq!(
            (row.friendly, row.enemy, row.air_spotting_scale),
            (42, 7, None)
        );
        assert_eq!(memory.attempts.get(), 1);
        let next = walk(&memory, BASE, GAME, HOUR + 1, &mut held, &mut output);
        assert!(!next.full);
        assert_eq!(
            output[..next.outcome.len()]
                .iter()
                .find(|r| r.slot == 5)
                .unwrap()
                .air_spotting_scale,
            None
        );
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn allocation_growth_preserves_the_previous_capacity_trace() {
        let mut held = Held::default();
        let mut regions = Vec::new();
        let mut countries = Vec::new();
        let mut slots = Vec::new();
        let mut rows = Vec::new();
        for (region_count, country_count, row_count) in
            [(1, 1, 1), (5, 5, 5), (3, 3, 3), (11, 9, 17)]
        {
            regions.resize(regions.len().max(region_count), Region::default());
            countries.resize(countries.len().max(country_count), Country::default());
            slots.resize(slots.len().max(region_count), 0_u32);
            rows.resize(rows.len().max(row_count), Row::default());
            held.grow(region_count, country_count, row_count).unwrap();
            assert_eq!(held.regions.capacity(), regions.capacity());
            assert_eq!(held.next_regions.capacity(), regions.capacity());
            assert_eq!(held.countries.capacity(), countries.capacity());
            assert_eq!(held.next_countries.capacity(), countries.capacity());
            let previous = regions.capacity() * 2 * size_of::<Region>()
                + countries.capacity() * 2 * size_of::<Country>()
                + slots.capacity() * size_of::<u32>()
                + rows.capacity() * size_of::<Row>();
            assert_eq!(held.allocated_bytes(), previous);
        }
    }

    #[test]
    fn failed_growth_reports_allocation_failure() {
        let mut held = Held::default();
        held.grow(2, 2, 2).unwrap();
        let pointer = held.regions.as_ptr();
        assert_eq!(
            held.grow(usize::MAX, 2, 2).unwrap_err().kind(),
            std::io::ErrorKind::OutOfMemory
        );
        assert_eq!(held.regions.as_ptr(), pointer);
    }

    #[test]
    fn the_movement_impact_is_read_and_carried_with_its_pair() {
        let mut world = World::new(1, 20);
        world.pair(1, 5, 5, 7);
        world.impact(1, 5, -40_000);
        let mut held = held(20, 1, 16);
        let mut rows = [Row::default(); 16];
        let impacts = |walk: &Walk, rows: &[Row]| {
            rows[..walk.outcome.len()]
                .iter()
                .map(|row| (row.region, row.impact))
                .collect::<Vec<_>>()
        };
        let first = world.walk(HOUR - 1, &mut held, &mut rows);
        assert_eq!(impacts(&first, &rows), [(1_005, -40_000), (1_019, 0)]);
        // Slot 5 is off HOUR's phase: its pair and impact are carried.
        world.impact(1, 5, -1);
        let next = world.walk(HOUR, &mut held, &mut rows);
        assert!(!next.full);
        assert_eq!(impacts(&next, &rows), [(1_005, -40_000), (1_019, 0)]);
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

    /// Strategic-air reset zeroes a country's pairs in place, off phase: a
    /// token its clears moved, an absent token or a new raw id at the same slot
    /// and address rereads the pairs a carry would keep.
    #[test]
    fn an_in_place_reset_or_a_new_raw_id_rereads_the_carried_pairs() {
        let mut world = World::new(2, 12);
        // Slot 5 is rewritten only at phase 5; HOUR is phase 3.
        world.pair(1, 5, 9, 9);
        let mut held = held(12, 2, 16);
        let mut rows = [Row::default(); 16];
        held.certify(Some(1));
        assert!(world.walk(HOUR - 1, &mut held, &mut rows).full);
        world.pair(1, 5, 0, 0);
        // Under the same token the pair is carried, as before the token.
        held.certify(Some(1));
        let carried = world.walk(HOUR, &mut held, &mut rows);
        assert!(!carried.full);
        assert!(pairs(&carried, &rows).contains(&(1, 1_005, 9, 9)));
        // The reset moved the token: every pair is read.
        held.certify(Some(2));
        let reread = world.walk(HOUR + 1, &mut held, &mut rows);
        assert!(reread.full);
        assert!(!pairs(&reread, &rows).contains(&(1, 1_005, 9, 9)));
        // Without a token nothing is carried, nor the hour after one returns.
        for (hour, token, full) in [
            (HOUR + 2, None, true),
            (HOUR + 3, None, true),
            (HOUR + 4, Some(2), true),
            (HOUR + 5, Some(2), false),
        ] {
            held.certify(token);
            assert_eq!(world.walk(hour, &mut held, &mut rows).full, full, "{hour}");
        }
        // A new raw id for slot one's object (an alias linked to slot one)
        // rereads that country alone.
        const LINKS: usize = 0x3000 + 3 * 8;
        let links = world.bytes.len();
        world.bytes.extend_from_within(LINKS..LINKS + 3 * 4);
        world.bytes.extend_from_slice(&1_u32.to_le_bytes());
        world.header(0x1000 + cgame_state::COUNTRY_LINKS, links, 4);
        world.pair(1, 6, 4, 4);
        world.pair(2, 6, 5, 5);
        let air = world.air(1);
        world.put(air + cstrategic_air::COUNTRY, 3);
        held.certify(Some(2));
        let walked = world.walk(HOUR + 6, &mut held, &mut rows);
        assert!(!walked.full);
        let got = pairs(&walked, &rows);
        assert!(got.contains(&(3, 1_006, 4, 4)) && !got.contains(&(2, 1_006, 5, 5)));
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
