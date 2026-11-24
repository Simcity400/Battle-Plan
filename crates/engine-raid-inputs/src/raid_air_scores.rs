
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::{Buffer, Outcome},
};
use engine_native::cgame_state;
use engine_native_air::cstrategic_air;
use engine_native_parts::cstrategic_region;
use engine_sites::Rva;
use std::sync::Arc;

mod menu;
pub use menu::{Contribution, Menu};

const SCORES: usize = 0xe0;
const STRIDE: usize = 0xa0;
const IMPACT: usize = 0x18;
const FRIENDLY_MISSIONS: usize = 0x28;
const ENEMY_MISSIONS: usize = 0x2a;
const ENTRY: usize = ENEMY_MISSIONS + 2;
const AIR_SPOTTING_SCALE: usize = 0x80;
pub const COMBAT_DELAY_RVA: Rva = Rva(0x033e_0bbc);
const HOUR_EPOCH: i32 = 43_800_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub region: u32,
    pub slot: u32,
    pub friendly: i64,
    pub enemy: i64,
    pub impact: i64,
    pub friendly_missions: u16,
    pub enemy_missions: u16,
    pub air_spotting_scale: Option<i64>,
    pub menu: Option<Arc<Menu>>,
    menu_bytes: u32,
}

impl Row {
    fn assign(&mut self, values: &Row, menu: Option<&Arc<Menu>>, menu_bytes: u32) {
        self.country = values.country;
        self.region = values.region;
        self.slot = values.slot;
        self.friendly = values.friendly;
        self.enemy = values.enemy;
        self.impact = values.impact;
        self.friendly_missions = values.friendly_missions;
        self.enemy_missions = values.enemy_missions;
        self.air_spotting_scale = values.air_spotting_scale;
        self.menu_bytes = menu_bytes;
        match (self.menu.as_ref(), menu) {
            (Some(kept), Some(menu)) if Arc::ptr_eq(kept, menu) => {}
            (None, None) => {}
            _ => self.menu = menu.cloned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Region {
    id: u32,
    phase: i32,
    present: bool,
    provinces: bool,
}

impl Region {
    fn updated(self, slot: usize, phase: i32) -> bool {
        slot != 0 && self.provinces && self.phase == phase
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Country {
    canonical_slot: u32,
    id: u32,
    address: usize,
    data: usize,
    rows: usize,
}

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
    token: Option<u64>,
    zero: Option<Arc<Menu>>,
    menu_scratch: menu::Scratch,
    menu_bytes: usize,
}

impl Held {
    pub fn invalidate(&mut self) {
        self.hour = None;
    }

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
        crate::allocation::resize_default(&mut self.rows, len)?;
        Ok(())
    }

    pub fn allocated_bytes(&self) -> usize {
        (self.regions.capacity() + self.next_regions.capacity()) * size_of::<Region>()
            + (self.countries.capacity() + self.next_countries.capacity()) * size_of::<Country>()
            + self.slots.capacity() * size_of::<u32>()
            + self.rows.capacity() * size_of::<Row>()
            + self.menu_bytes
            + self.zero.as_ref().map_or(0, |menu| menu.allocated_bytes())
            + self.menu_scratch.allocated_bytes()
    }
}

pub struct Walk {
    pub outcome: Outcome,
    pub regions: usize,
    pub countries: usize,
    pub full: bool,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game: usize,
    hour: i32,
    held: &mut Held,
    output: &mut Vec<Row>,
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        regions: 0,
        countries: 0,
        full: true,
    };
    let result = read_table(memory, base, game, hour, held, output, &mut walk).and_then(|result| {
        Buffer::Growing(&mut held.rows).ensure(result.0)?;
        Ok(result)
    });
    walk.outcome = match result {
        Ok((len, period)) => {
            let mut menu_bytes = 0;
            for (kept, row) in held.rows[..len].iter_mut().zip(&output[..len]) {
                kept.assign(row, row.menu.as_ref(), row.menu_bytes);
                menu_bytes += row.menu_bytes as usize;
            }
            let stale = held.row_len.max(len);
            for row in &mut held.rows[len..stale] {
                row.menu = None;
            }
            held.menu_bytes = menu_bytes;
            held.row_len = len;
            std::mem::swap(&mut held.regions, &mut held.next_regions);
            std::mem::swap(&mut held.countries, &mut held.next_countries);
            held.region_len = walk.regions;
            held.country_len = walk.countries;
            held.period = period;
            held.hour = Some(hour);
            Outcome::Rows { len }
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
    output: &mut Vec<Row>,
    walk: &mut Walk,
) -> Result<(usize, i32), &'static str> {
    let delay = engine_sites::at(base, COMBAT_DELAY_RVA)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("air combat delay define unreadable")?;
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
    held.grow(walk.regions, held.next_countries.len(), held.rows.len())
        .map_err(|_| "capture buffer allocation failed")?;
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
    let zero = &*held.zero.get_or_insert_with(|| Arc::new(Menu::default()));
    let scratch = &mut held.menu_scratch;
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
            zero,
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
                scratch,
                output,
                &mut len,
            )?,
            None => read_all(memory, scores, &target, scratch, output, &mut len)?,
        }
        let counted = countries
            .checked_add(1)
            .ok_or("raid air country count overflow")?;
        Buffer::Growing(&mut *next).ensure(counted)?;
        next[countries] = Country {
            canonical_slot: root.canonical_slot,
            id: root.country,
            address: root.address,
            data: scores.data,
            rows: len - first,
        };
        countries = counted;
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

fn read_region_serial(memory: &impl Memory, pointer: usize, out: &mut [u8; 28]) -> bool {
    let (vtable, rest) = out.split_at_mut(8);
    let (provinces, rest) = rest.split_at_mut(4);
    let (identity, phase) = rest.split_at_mut(12);
    let (Some(p_provinces), Some(p_identity), Some(p_phase)) = (
        pointer.checked_add(cstrategic_region::PROVINCES_COUNT),
        pointer.checked_add(cstrategic_region::ID),
        pointer.checked_add(cstrategic_region::AIR_PHASE),
    ) else {
        return false;
    };
    memory.copy_scattered(
        [pointer, p_provinces, p_identity, p_phase],
        &mut [vtable, provinces, identity, phase],
    ) == 4
}

fn read_regions(
    memory: &impl Memory,
    base: usize,
    roster: Array,
    regions: &mut [Region],
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    let expected = engine_sites::at(base, cstrategic_region::VTABLE_RVA)
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
        let copied = if live == 0 {
            0
        } else {
            memory.copy_records(
                RecordSources::Pointers(&pointers[..live]),
                &REGION_LAYOUT,
                &mut bytes[..live * 28],
            )
        };
        let done = (copied / REGION_FIELDS.len()).min(live);
        for index in done..live {
            let pointer = pointers[index];
            let offset = index * 28;
            if let Some(previous) = pointers[..index].iter().position(|&seen| seen == pointer) {
                let src = previous * 28;
                bytes.copy_within(src..src + 28, offset);
            } else {
                let record: &mut [u8; 28] = (&mut bytes[offset..offset + 28]).try_into().unwrap();
                if !read_region_serial(memory, pointer, record) {
                    return Err("raid air region unreadable");
                }
            }
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
    zero: &'a Arc<Menu>,
}

impl Target<'_> {
    #[expect(
        clippy::too_many_arguments,
        reason = "the slot, pair, spotting scale, read and held row are separate facts of one score, and the output and length are caller-owned"
    )]
    fn push(
        &self,
        slot: usize,
        pair: &[u8],
        air_spotting_scale: Option<i64>,
        read: menu::Read,
        held: Option<&Row>,
        output: &mut Vec<Row>,
        len: &mut usize,
    ) -> Result<(), &'static str> {
        let changed;
        let (menu, menu_bytes) = match read {
            menu::Read::Unavailable => (None, 0),
            menu::Read::Zero => (Some(self.zero), 0),
            menu::Read::Same => match held {
                Some(held) => (held.menu.as_ref(), held.menu_bytes),
                None => return Err("raid air menu held row missing"),
            },
            menu::Read::Changed(menu) => {
                changed = menu;
                (
                    Some(&changed),
                    u32::try_from(changed.allocated_bytes()).unwrap_or(u32::MAX),
                )
            }
        };
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
            && menu.is_some_and(|menu| Arc::ptr_eq(menu, self.zero) || menu.is_zero())
            && slot != self.last
        {
            return Ok(());
        }
        push(
            output,
            len,
            &Row {
                country: self.country,
                region: self.regions[slot].id,
                slot: slot as u32,
                friendly,
                enemy,
                impact,
                friendly_missions,
                enemy_missions,
                air_spotting_scale,
                menu: None,
                menu_bytes: 0,
            },
            menu,
            menu_bytes,
        )
    }
}

fn push(
    output: &mut Vec<Row>,
    len: &mut usize,
    row: &Row,
    menu: Option<&Arc<Menu>>,
    menu_bytes: u32,
) -> Result<(), &'static str> {
    let next = len.checked_add(1).ok_or("raid air score count overflow")?;
    Buffer::Growing(&mut *output).ensure(next)?;
    output[*len].assign(row, menu, menu_bytes);
    *len = next;
    Ok(())
}

fn carry_row(output: &mut Vec<Row>, len: &mut usize, row: &Row) -> Result<(), &'static str> {
    push(output, len, row, row.menu.as_ref(), row.menu_bytes)
}

const MENU_REDUCTIONS: usize = 0x30;
const FULL_ENTRY: usize = AIR_SPOTTING_SCALE + 8;
const FULL_CONTIGUOUS: [RecordField; 1] = [field(0, 0, FULL_ENTRY)];
const FULL_CONTIGUOUS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FULL_CONTIGUOUS, FULL_ENTRY).expect("valid contiguous raid air score layout");
const FULL: [RecordField; 3] = [
    field(0, 0, ENTRY),
    field(MENU_REDUCTIONS, MENU_REDUCTIONS, 64),
    field(AIR_SPOTTING_SCALE, AIR_SPOTTING_SCALE, 8),
];
const FULL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FULL, FULL_ENTRY).expect("valid full raid air score layout");
const BATCH: usize = 128;

#[inline]
fn is_fast_zero(entry: &[u8]) -> bool {
    let u64_at = |o: usize| u64::from_le_bytes(entry[o..o + 8].try_into().unwrap());
    let missions = u32::from_le_bytes(
        entry[FRIENDLY_MISSIONS..FRIENDLY_MISSIONS + 4]
            .try_into()
            .unwrap(),
    );
    (u64_at(0)
        | u64_at(8)
        | u64_at(0x10)
        | u64_at(0x30)
        | u64_at(0x38)
        | u64_at(0x40)
        | u64_at(0x48)
        | u64_at(0x58)
        | u64_at(0x68)
        | u64_at(AIR_SPOTTING_SCALE)
        | u64::from(missions))
        == 0
        && usize::try_from(u64_at(0x50)).is_ok_and(|head| head != 0)
        && usize::try_from(u64_at(0x60)).is_ok_and(|head| head != 0)
}

fn read_entries<'a>(
    memory: &impl Memory,
    count: usize,
    mut sources: impl FnMut(usize, usize) -> Result<RecordSources<'a>, &'static str>,
    mut address_at: impl FnMut(usize) -> Result<usize, &'static str>,
    full: &mut [u8; BATCH * FULL_ENTRY],
    scales: &mut [Option<i64>; BATCH],
    menu_ok: &mut [bool; BATCH],
) -> Result<bool, &'static str> {
    let mut all_menus = true;
    let mut at = 0;
    while at < count {
        let fast = memory.copy_records(
            sources(at, count - at)?,
            &FULL_CONTIGUOUS_LAYOUT,
            &mut full[at * FULL_ENTRY..count * FULL_ENTRY],
        );
        for index in at..at + fast {
            let entry = &full[index * FULL_ENTRY..(index + 1) * FULL_ENTRY];
            scales[index] = Some(i64::from_le_bytes(
                entry[AIR_SPOTTING_SCALE..FULL_ENTRY].try_into().unwrap(),
            ));
            menu_ok[index] = true;
        }
        at += fast;
        if at == count {
            break;
        }
        let copied = memory.copy_records(
            sources(at, 1)?,
            &FULL_LAYOUT,
            &mut full[at * FULL_ENTRY..(at + 1) * FULL_ENTRY],
        );
        match copied {
            0 => return Err("raid air score pair unreadable"),
            1 => {
                menu_ok[at] = false;
                all_menus = false;
                scales[at] = address_at(at)?
                    .checked_add(AIR_SPOTTING_SCALE)
                    .and_then(|addr| read::<i64>(memory, addr));
            }
            2 => {
                menu_ok[at] = true;
                scales[at] = None;
            }
            _ => {
                let entry = &full[at * FULL_ENTRY..(at + 1) * FULL_ENTRY];
                scales[at] = Some(i64::from_le_bytes(
                    entry[AIR_SPOTTING_SCALE..FULL_ENTRY].try_into().unwrap(),
                ));
                menu_ok[at] = true;
            }
        }
        at += 1;
    }
    Ok(all_menus)
}

fn read_all(
    memory: &impl Memory,
    scores: Array,
    target: &Target<'_>,
    scratch: &mut menu::Scratch,
    output: &mut Vec<Row>,
    len: &mut usize,
) -> Result<(), &'static str> {
    let mut reads = scratch.take_reads();
    let result = (|| -> Result<(), &'static str> {
        for first in (0..scores.count as usize).step_by(BATCH) {
            let count = (scores.count as usize - first).min(BATCH);
            let buffers = scratch.buffers();
            let all_menus = read_entries(
                memory,
                count,
                |at, span| {
                    let base = scores
                        .element((first + at) as u32, STRIDE)
                        .ok_or("raid air score address overflow")?;
                    Ok(RecordSources::Strided {
                        base,
                        stride: STRIDE,
                        count: span,
                    })
                },
                |at| {
                    scores
                        .element((first + at) as u32, STRIDE)
                        .ok_or("raid air scale address overflow")
                },
                &mut buffers.full,
                &mut buffers.scales,
                &mut buffers.menu_ok,
            )?;
            let mut active_len = 0;
            for index in 0..count {
                let slot = first + index;
                if !target.regions[slot].present {
                    continue;
                }
                let entry = &buffers.full[index * FULL_ENTRY..(index + 1) * FULL_ENTRY];
                if (all_menus || buffers.menu_ok[index])
                    && buffers.scales[index] == Some(0)
                    && is_fast_zero(entry)
                {
                    continue;
                }
                buffers.active[active_len] = index;
                active_len += 1;
            }
            menu::read_selected_entries(
                memory,
                FULL_ENTRY,
                active_len,
                all_menus,
                &[],
                scratch,
                &mut reads,
            );
            let buffers = scratch.buffers();
            let mut cursor = 0;
            for index in 0..count {
                let slot = first + index;
                if !target.regions[slot].present {
                    continue;
                }
                let entry = &buffers.full[index * FULL_ENTRY..(index + 1) * FULL_ENTRY];
                if cursor < active_len && buffers.active[cursor] == index {
                    let read = std::mem::replace(&mut reads[cursor], menu::Read::Unavailable);
                    cursor += 1;
                    target.push(
                        slot,
                        &entry[..ENTRY],
                        buffers.scales[index],
                        read,
                        None,
                        output,
                        len,
                    )?;
                } else if slot == target.last {
                    target.push(
                        slot,
                        &entry[..ENTRY],
                        buffers.scales[index],
                        menu::Read::Zero,
                        None,
                        output,
                        len,
                    )?;
                }
            }
        }
        Ok(())
    })();
    scratch.return_reads(reads);
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "the scores, target, slots, held rows and phase come from the caller, and the scratch, output and length are caller-owned"
)]
fn carry(
    memory: &impl Memory,
    scores: Array,
    target: &Target<'_>,
    slots: &[u32],
    held: &[Row],
    phase: i32,
    scratch: &mut menu::Scratch,
    output: &mut Vec<Row>,
    len: &mut usize,
) -> Result<(), &'static str> {
    let updated = |row: &Row| target.regions[row.slot as usize].updated(row.slot as usize, phase);
    let mut next = 0;
    let mut reads = scratch.take_reads();
    let result = (|| -> Result<(), &'static str> {
        for batch in slots.chunks(BATCH) {
            let count = batch.len();
            let buffers = scratch.buffers();
            for (address, slot) in buffers.addresses[..count].iter_mut().zip(batch) {
                *address = scores
                    .element(*slot, STRIDE)
                    .ok_or("raid air score address overflow")?;
            }
            let slice = &buffers.addresses[..count];
            let all_menus = read_entries(
                memory,
                count,
                |at, span| Ok(RecordSources::Pointers(&slice[at..at + span])),
                |at| Ok(slice[at]),
                &mut buffers.full,
                &mut buffers.scales,
                &mut buffers.menu_ok,
            )?;
            let mut active_len = 0;
            let mut active_prev: [Option<&Row>; BATCH] = [None; BATCH];
            let mut active_menus: [Option<&Arc<Menu>>; BATCH] = [None; BATCH];
            let mut ahead = next;
            for (index, &slot) in batch.iter().enumerate() {
                let entry = &buffers.full[index * FULL_ENTRY..(index + 1) * FULL_ENTRY];
                if (all_menus || buffers.menu_ok[index])
                    && buffers.scales[index] == Some(0)
                    && is_fast_zero(entry)
                {
                    continue;
                }
                while held.get(ahead).is_some_and(|row| row.slot < slot) {
                    ahead += 1;
                }
                let prev = held.get(ahead).filter(|row| row.slot == slot);
                buffers.active[active_len] = index;
                active_prev[active_len] = prev;
                active_menus[active_len] = prev.and_then(|row| row.menu.as_ref());
                active_len += 1;
            }
            menu::read_selected_entries(
                memory,
                FULL_ENTRY,
                active_len,
                all_menus,
                &active_menus[..active_len],
                scratch,
                &mut reads,
            );
            let buffers = scratch.buffers();
            let mut cursor = 0;
            for (index, &slot) in batch.iter().enumerate() {
                while let Some(row) = held.get(next).filter(|row| row.slot <= slot) {
                    next += 1;
                    if row.slot < slot && !updated(row) {
                        carry_row(output, len, row)?;
                    }
                }
                let entry = &buffers.full[index * FULL_ENTRY..(index + 1) * FULL_ENTRY];
                if cursor < active_len && buffers.active[cursor] == index {
                    let read = std::mem::replace(&mut reads[cursor], menu::Read::Unavailable);
                    let prev = active_prev[cursor];
                    cursor += 1;
                    target.push(
                        slot as usize,
                        &entry[..ENTRY],
                        buffers.scales[index],
                        read,
                        prev,
                        output,
                        len,
                    )?;
                } else if slot as usize == target.last {
                    target.push(
                        slot as usize,
                        &entry[..ENTRY],
                        buffers.scales[index],
                        menu::Read::Zero,
                        None,
                        output,
                        len,
                    )?;
                }
            }
        }
        for row in &held[next..] {
            if !updated(row) {
                carry_row(output, len, row)?;
            }
        }
        Ok(())
    })();
    scratch.return_reads(reads);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const MEMORY: usize = 0x1000_0000;
    const BASE: usize = MEMORY + 0x100 - COMBAT_DELAY_RVA.0;
    const GAME: usize = MEMORY + 0x1000;
    const HOUR: i32 = 43_800_000 + 8 * 1_000 + 3;

    struct World {
        bytes: Vec<u8>,
        region_list: usize,
        region_objects: usize,
        airs: usize,
        stride: usize,
    }

    impl World {
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
                bytes: vec![0; airs + countries * stride + 32],
                region_list,
                region_objects,
                airs,
                stride,
            };
            world.put32(0x100, 4);
            world.header(0x1000 + cgame_state::COUNTRIES, countries_at, slots);
            world.header(0x1000 + cgame_state::COUNTRY_LINKS, links_at, slots);
            world.put(0x1000 + cgame_state::STRATEGIC_AIR_MANAGER, MEMORY + 0x2000);
            world.put(0x2000, BASE + engine_native_air::cstrategic_air_manager::VTABLE_RVA.0);
            world.header(
                0x2000 + engine_native_air::cstrategic_air_manager::COUNTRY_AIR,
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
                world.put(region, BASE + cstrategic_region::VTABLE_RVA.0);
                world.put32(region + cstrategic_region::PROVINCES_COUNT, 1);
                world.put32(region + cstrategic_region::ID, 1_000 + slot as u32);
                world.put32(region + cstrategic_region::RUNTIME_SLOT, slot as u32);
                world.put32(region + cstrategic_region::AIR_PHASE, slot as u32 % 8);
            }
            let sentinel = airs + countries * stride;
            world.put(sentinel, MEMORY + sentinel);
            world.put(sentinel + 8, MEMORY + sentinel);
            world.put(sentinel + 16, MEMORY + sentinel);
            world.bytes[sentinel + 25] = 1;
            for slot in 1..slots {
                let air = world.air(slot);
                world.put(countries_at + slot * 8, MEMORY + air - 0x20);
                world.put32(links_at + slot * 4, slot as u32);
                world.put(air - 0x20, BASE + engine_native::ccountry::VTABLE_RVA.0);
                world.put(air - 0x20 + engine_native::ccountry::ID, slot);
                world.put(air_at + slot * 8, MEMORY + air);
                world.put(air, BASE + cstrategic_air::VTABLE_RVA.0);
                world.put(air + cstrategic_air::COUNTRY, slot);
                world.header(air + SCORES, air + 0x1e0, regions);
                for region in 0..regions {
                    let entry = air + 0x1e0 + region * STRIDE;
                    world.put(entry + 0x50, MEMORY + sentinel);
                    world.put(entry + 0x60, MEMORY + sentinel);
                }
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

        fn entry(&self, country: usize, slot: usize) -> usize {
            let data = u64::from_le_bytes(
                self.bytes[self.air(country) + SCORES..][..8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            data - MEMORY + slot * STRIDE
        }

        fn coverage(&mut self, country: usize, slot: usize, value: i64) {
            let at = self.entry(country, slot) + 0x10;
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        fn contributions(
            &mut self,
            country: usize,
            slot: usize,
            side: usize,
            entries: &[(u32, i64)],
        ) {
            let head = self.bytes.len();
            let nodes = head + 0x30;
            self.bytes.resize(nodes + entries.len() * 0x30, 0);
            let root = self.subtree(entries, 0, entries.len(), nodes, head, head);
            let last = entries.len().saturating_sub(1);
            let (left, right) = if entries.is_empty() {
                (head, head)
            } else {
                (nodes, nodes + last * 0x30)
            };
            self.put(head, MEMORY + left);
            self.put(head + 8, MEMORY + root);
            self.put(head + 0x10, MEMORY + right);
            self.bytes[head + 0x19] = 1;
            let at = self.entry(country, slot) + 0x50 + side * 16;
            self.put(at, MEMORY + head);
            self.put(at + 8, entries.len());
        }

        fn subtree(
            &mut self,
            entries: &[(u32, i64)],
            first: usize,
            end: usize,
            nodes: usize,
            parent: usize,
            head: usize,
        ) -> usize {
            if first == end {
                return head;
            }
            let middle = (first + end) / 2;
            let at = nodes + middle * 0x30;
            let left = self.subtree(entries, first, middle, nodes, at, head);
            let right = self.subtree(entries, middle + 1, end, nodes, at, head);
            self.put(at, MEMORY + left);
            self.put(at + 8, MEMORY + parent);
            self.put(at + 0x10, MEMORY + right);
            self.put32(at + 0x20, entries[middle].0);
            self.bytes[at + 0x28..at + 0x30].copy_from_slice(&entries[middle].1.to_le_bytes());
            at
        }

        fn walk(&self, hour: i32, held: &mut Held, output: &mut Vec<Row>) -> Walk {
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
    fn menus_are_read_at_their_phase_and_an_unchanged_menu_keeps_its_allocation() {
        let mut world = World::new(1, 12);
        world.pair(1, 3, 5, 0);
        world.coverage(1, 3, 70);
        world.contributions(1, 3, 0, &[(2, 10), (5, 20), (9, 30)]);
        world.pair(1, 4, 1, 0);
        let empty = world.entry(1, 4) + 0x60;
        world.put(empty, MEMORY + 0x0fff_0000);
        let mut held = held(12, 1, 16);
        held.certify(Some(1));
        let mut rows = Vec::new();
        let menu = |walk: &Walk, rows: &[Row], slot: u32| {
            rows[..walk.outcome.len()]
                .iter()
                .find(|row| row.slot == slot)
                .and_then(|row| row.menu.clone())
        };
        let full = world.walk(HOUR - 1, &mut held, &mut rows);
        assert!(full.full);
        let read = menu(&full, &rows, 3).unwrap();
        assert_eq!(read.coverage, 70);
        assert_eq!(
            read.contributions[0]
                .iter()
                .map(|row| (row.country, row.amount))
                .collect::<Vec<_>>(),
            [(2, 10), (5, 20), (9, 30)]
        );
        assert!(read.contributions[1].is_empty());
        let zero = menu(&full, &rows, 4).unwrap();
        assert!(zero.is_zero());
        assert!(held.allocated_bytes() >= read.allocated_bytes());

        let same = world.walk(HOUR, &mut held, &mut rows);
        assert!(!same.full);
        assert!(Arc::ptr_eq(&menu(&same, &rows, 3).unwrap(), &read));
        assert!(Arc::ptr_eq(&menu(&same, &rows, 4).unwrap(), &zero));

        world.contributions(1, 3, 0, &[(2, 10), (5, 25)]);
        world.coverage(1, 4, 8);
        let next = world.walk(HOUR + 1, &mut held, &mut rows);
        assert!(Arc::ptr_eq(&menu(&next, &rows, 3).unwrap(), &read));
        assert_eq!(menu(&next, &rows, 4).unwrap().coverage, 8);
        for hour in HOUR + 2..HOUR + 8 {
            assert!(!world.walk(hour, &mut held, &mut rows).full);
        }
        let phase = world.walk(HOUR + 8, &mut held, &mut rows);
        let changed = menu(&phase, &rows, 3).unwrap();
        assert!(!Arc::ptr_eq(&changed, &read));
        assert_eq!(
            changed.contributions[0]
                .iter()
                .map(|row| (row.country, row.amount))
                .collect::<Vec<_>>(),
            [(2, 10), (5, 25)]
        );

        let at = world.entry(1, 3) + 0x50;
        let head =
            u64::from_le_bytes(world.bytes[at..at + 8].try_into().unwrap()) as usize - MEMORY;
        let root = u64::from_le_bytes(world.bytes[head + 8..head + 16].try_into().unwrap())
            as usize
            - MEMORY;
        world.put(root + 8, MEMORY + root);
        for hour in HOUR + 9..HOUR + 16 {
            world.walk(hour, &mut held, &mut rows);
        }
        let broken = world.walk(HOUR + 16, &mut held, &mut rows);
        let row = rows[..broken.outcome.len()]
            .iter()
            .find(|row| row.slot == 3)
            .unwrap();
        assert_eq!((row.friendly, row.menu.clone()), (5, None));
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
                if (at..at.saturating_add(to.len())).contains(&self.at) {
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
        assert_eq!(memory.attempts.get(), 2);
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
        assert_eq!(memory.attempts.get(), 2);

        world.spotting_scale(1, 5, 3);
        struct OneShot<'a> {
            memory: SliceMemory<'a>,
            at: usize,
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for OneShot<'_> {
            fn copy(&self, at: usize, to: &mut [u8]) -> bool {
                if (at..at.saturating_add(to.len())).contains(&self.at) {
                    self.attempts.set(self.attempts.get() + 1);
                    if self.attempts.get() == 1 {
                        return false;
                    }
                }
                self.memory.copy(at, to)
            }
        }
        let memory = OneShot {
            memory: SliceMemory {
                base: MEMORY,
                bytes: &world.bytes,
            },
            at: data + 5 * STRIDE + AIR_SPOTTING_SCALE,
            attempts: Default::default(),
        };
        let mut fresh = super::tests::held(17, 1, 17);
        fresh.certify(Some(1));
        let recovered = walk(&memory, BASE, GAME, HOUR, &mut fresh, &mut output);
        assert_eq!(
            output[..recovered.outcome.len()]
                .iter()
                .find(|r| r.slot == 5)
                .unwrap()
                .air_spotting_scale,
            Some(3)
        );
        assert_eq!(memory.attempts.get(), 2);

        let zero_scale_fault = Fault {
            memory: SliceMemory {
                base: MEMORY,
                bytes: &world.bytes,
            },
            at: data + 4 * STRIDE + AIR_SPOTTING_SCALE,
            attempts: Default::default(),
        };
        let mut zero_held = super::tests::held(17, 1, 17);
        zero_held.certify(Some(1));
        let full_zero = walk(&zero_scale_fault, BASE, GAME, HOUR, &mut zero_held, &mut output);
        assert!(matches!(full_zero.outcome, Outcome::Rows { .. }));
        let row = output[..full_zero.outcome.len()]
            .iter()
            .find(|r| r.slot == 4)
            .expect("zero non-final row with unreadable scale must be preserved");
        assert_eq!((row.friendly, row.enemy, row.air_spotting_scale), (0, 0, None));
        let carry_zero = walk(
            &zero_scale_fault,
            BASE,
            GAME,
            HOUR + 1,
            &mut zero_held,
            &mut output,
        );
        assert!(!carry_zero.full);
        let row = output[..carry_zero.outcome.len()]
            .iter()
            .find(|r| r.slot == 4)
            .expect("carried zero non-final row with unreadable scale must be preserved");
        assert_eq!((row.friendly, row.enemy, row.air_spotting_scale), (0, 0, None));
        assert_eq!(zero_scale_fault.attempts.get(), 4);

        world.spotting_scale(1, 7, 11);
        struct TwoOneShots<'a> {
            memory: SliceMemory<'a>,
            first: usize,
            second: usize,
            first_attempts: std::cell::Cell<usize>,
            second_attempts: std::cell::Cell<usize>,
        }
        impl Memory for TwoOneShots<'_> {
            fn copy(&self, at: usize, to: &mut [u8]) -> bool {
                let range = at..at.saturating_add(to.len());
                if range.contains(&self.first) {
                    let count = self.first_attempts.get() + 1;
                    self.first_attempts.set(count);
                    if count == 1 {
                        return false;
                    }
                }
                if range.contains(&self.second) {
                    let count = self.second_attempts.get() + 1;
                    self.second_attempts.set(count);
                    if count == 1 {
                        return false;
                    }
                }
                self.memory.copy(at, to)
            }
        }
        let two = TwoOneShots {
            memory: SliceMemory {
                base: MEMORY,
                bytes: &world.bytes,
            },
            first: data + 5 * STRIDE,
            second: data + 7 * STRIDE + AIR_SPOTTING_SCALE,
            first_attempts: Default::default(),
            second_attempts: Default::default(),
        };
        let mut two_held = super::tests::held(17, 1, 17);
        two_held.certify(Some(1));
        let both = walk(&two, BASE, GAME, HOUR, &mut two_held, &mut output);
        assert_eq!(
            output[..both.outcome.len()]
                .iter()
                .find(|r| r.slot == 7)
                .unwrap()
                .air_spotting_scale,
            Some(11)
        );
        assert_eq!(two.second_attempts.get(), 2);
    }

    #[test]
    fn region_and_score_pair_faults_retry_once_without_rereading_completed_rows() {
        struct CountedFault<'a> {
            memory: SliceMemory<'a>,
            fault: usize,
            once: bool,
            spent: std::cell::Cell<bool>,
            reads: std::cell::RefCell<std::collections::HashMap<usize, usize>>,
        }
        impl<'a> CountedFault<'a> {
            fn new(bytes: &'a [u8], fault: usize, once: bool) -> Self {
                Self {
                    memory: SliceMemory {
                        base: MEMORY,
                        bytes,
                    },
                    fault,
                    once,
                    spent: std::cell::Cell::new(false),
                    reads: std::cell::RefCell::new(std::collections::HashMap::new()),
                }
            }
            fn reads_at(&self, address: usize) -> usize {
                self.reads.borrow().get(&address).copied().unwrap_or(0)
            }
        }
        impl Memory for CountedFault<'_> {
            fn copy(&self, at: usize, to: &mut [u8]) -> bool {
                {
                    let mut reads = self.reads.borrow_mut();
                    for offset in 0..to.len() {
                        *reads.entry(at + offset).or_insert(0) += 1;
                    }
                }
                if at <= self.fault
                    && self.fault < at.saturating_add(to.len())
                    && (!self.once || !self.spent.get())
                {
                    if self.once {
                        self.spent.set(true);
                    }
                    to.fill(0xff);
                    return false;
                }
                self.memory.copy(at, to)
            }
        }

        let mut world = World::new(1, 20);
        world.pair(1, 3, 5, 1);
        world.pair(1, 5, 9, 2);
        world.pair(1, 11, 7, 3);

        let region_kept = MEMORY + world.region(1) + cstrategic_region::AIR_PHASE;
        let region_fault = MEMORY + world.region(3) + cstrategic_region::AIR_PHASE;
        for once in [true, false] {
            let memory = CountedFault::new(&world.bytes, region_fault, once);
            let mut held = held(20, 1, 20);
            held.certify(Some(1));
            let mut output = vec![Row::default(); 20];
            let walked = walk(&memory, BASE, GAME, HOUR, &mut held, &mut output);
            if once {
                assert!(matches!(walked.outcome, Outcome::Rows { .. }));
                assert!(pairs(&walked, &output).contains(&(1, 1_003, 5, 1)));
            } else {
                assert_eq!(
                    walked.outcome,
                    Outcome::Unavailable {
                        reason: "raid air region unreadable"
                    }
                );
            }
            assert_eq!(memory.reads_at(region_kept), 1);
            assert_eq!(memory.reads_at(region_fault), 2);
        }

        let bad_vtable = world.region(1);
        world.bytes[bad_vtable] ^= 1;
        let memory = CountedFault::new(&world.bytes, region_fault, false);
        let mut state = held(20, 1, 20);
        state.certify(Some(1));
        let mut output = vec![Row::default(); 20];
        let walked = walk(&memory, BASE, GAME, HOUR, &mut state, &mut output);
        assert_eq!(
            walked.outcome,
            Outcome::Unavailable {
                reason: "raid air region unreadable"
            }
        );
        assert_eq!(memory.reads_at(MEMORY + bad_vtable), 1);
        assert_eq!(memory.reads_at(region_fault), 2);
        world.bytes[bad_vtable] ^= 1;

        let pair_kept = MEMORY + world.entry(1, 3);
        let pair_fault = MEMORY + world.entry(1, 5);
        for once in [true, false] {
            let memory = CountedFault::new(&world.bytes, pair_fault, once);
            let mut held = held(20, 1, 20);
            held.certify(Some(1));
            let mut output = vec![Row::default(); 20];
            let walked = walk(&memory, BASE, GAME, HOUR, &mut held, &mut output);
            if once {
                assert!(walked.full);
                assert!(matches!(walked.outcome, Outcome::Rows { .. }));
                assert!(pairs(&walked, &output).contains(&(1, 1_005, 9, 2)));
            } else {
                assert_eq!(
                    walked.outcome,
                    Outcome::Unavailable {
                        reason: "raid air score pair unreadable"
                    }
                );
            }
            assert_eq!(memory.reads_at(pair_kept), 1);
            assert_eq!(memory.reads_at(pair_fault), 2);
        }

        let carry_kept = MEMORY + world.entry(1, 3);
        let carry_fault = MEMORY + world.entry(1, 11);
        for once in [true, false] {
            let mut held = held(20, 1, 20);
            held.certify(Some(1));
            let mut output = vec![Row::default(); 20];
            assert!(world.walk(HOUR - 1, &mut held, &mut output).full);
            world.pair(1, 3, 15, 4);
            world.pair(1, 11, 17, 6);
            let memory = CountedFault::new(&world.bytes, carry_fault, once);
            let walked = walk(&memory, BASE, GAME, HOUR, &mut held, &mut output);
            if once {
                assert!(!walked.full);
                assert!(matches!(walked.outcome, Outcome::Rows { .. }));
                let got = pairs(&walked, &output);
                assert!(got.contains(&(1, 1_003, 15, 4)));
                assert!(got.contains(&(1, 1_011, 17, 6)));
            } else {
                assert_eq!(
                    walked.outcome,
                    Outcome::Unavailable {
                        reason: "raid air score pair unreadable"
                    }
                );
            }
            assert_eq!(memory.reads_at(carry_kept), 1);
            assert_eq!(memory.reads_at(carry_fault), 2);
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
        let mut rows = Vec::new();
        let impacts = |walk: &Walk, rows: &[Row]| {
            rows[..walk.outcome.len()]
                .iter()
                .map(|row| (row.region, row.impact))
                .collect::<Vec<_>>()
        };
        let first = world.walk(HOUR - 1, &mut held, &mut rows);
        assert_eq!(impacts(&first, &rows), [(1_005, -40_000), (1_019, 0)]);
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
        let mut rows = Vec::new();
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
        assert!(world.walk(HOUR + 11, &mut held, &mut rows).full);
    }

    #[test]
    fn each_changed_premise_rereads_what_it_cannot_carry() {
        let mut world = World::new(2, 12);
        let mut held = held(12, 2, 16);
        let mut rows = Vec::new();
        assert!(world.walk(HOUR - 1, &mut held, &mut rows).full);
        assert!(!world.walk(HOUR, &mut held, &mut rows).full);
        world.put32(0x100, 3);
        assert!(world.walk(HOUR + 1, &mut held, &mut rows).full);
        world.put32(0x100, 4);
        assert!(world.walk(HOUR + 2, &mut held, &mut rows).full);
        let region = world.region(7);
        world.put32(region + cstrategic_region::AIR_PHASE, 6);
        assert!(world.walk(HOUR + 3, &mut held, &mut rows).full);
        assert!(!world.walk(HOUR + 4, &mut held, &mut rows).full);
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
    fn an_in_place_reset_or_a_new_raw_id_rereads_the_carried_pairs() {
        let mut world = World::new(2, 12);
        world.pair(1, 5, 9, 9);
        let mut held = held(12, 2, 16);
        let mut rows = Vec::new();
        held.certify(Some(1));
        assert!(world.walk(HOUR - 1, &mut held, &mut rows).full);
        world.pair(1, 5, 0, 0);
        held.certify(Some(1));
        let carried = world.walk(HOUR, &mut held, &mut rows);
        assert!(!carried.full);
        assert!(pairs(&carried, &rows).contains(&(1, 1_005, 9, 9)));
        held.certify(Some(2));
        let reread = world.walk(HOUR + 1, &mut held, &mut rows);
        assert!(reread.full);
        assert!(!pairs(&reread, &rows).contains(&(1, 1_005, 9, 9)));
        for (hour, token, full) in [
            (HOUR + 2, None, true),
            (HOUR + 3, None, true),
            (HOUR + 4, Some(2), true),
            (HOUR + 5, Some(2), false),
        ] {
            held.certify(token);
            assert_eq!(world.walk(hour, &mut held, &mut rows).full, full, "{hour}");
        }
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
    fn held_tables_smaller_than_the_game_grow_and_bad_rosters_leave_nothing_to_carry() {
        let mut world = World::new(2, 10);
        world.pair(1, 2, 1, 1);
        let mut rows = Vec::new();
        let mut held = held(9, 1, 0);
        let walked = world.walk(HOUR, &mut held, &mut rows);
        assert_eq!((walked.regions, walked.countries), (10, 2));
        assert_eq!(walked.outcome, Outcome::Rows { len: 3 });
        assert_eq!(
            pairs(&walked, &rows),
            [(1, 1_002, 1, 1), (1, 1_009, 0, 0), (2, 1_009, 0, 0)]
        );
        assert!(!world.walk(HOUR + 1, &mut held, &mut rows).full);
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

}
