
use super::{GROUP, NEVER, Tracks};
use crate::{
    invalid,
    store::{Flags, NumberCells, Numbers},
    unit::{BLOCK, DIFFERENCES, RAW, RUNS, SECOND, unzig},
};
use std::io;

#[inline(never)]
fn long(bytes: &[u8], at: &mut usize, damaged: &mut bool, width: u32) -> u128 {
    let (mut value, mut shift) = (0_u128, 0_u32);
    loop {
        let Some(&byte) = bytes.get(*at) else {
            *damaged = true;
            return value;
        };
        *at += 1;
        if shift < width && u32::from(byte & 0x7f) >> (width - shift).min(7) == 0 {
            value |= u128::from(byte & 0x7f) << shift;
        } else {
            *damaged = true;
        }
        if byte < 0x80 {
            return value;
        }
        shift += 7;
    }
}

#[inline(always)]
fn window(word: [u8; 8]) -> Option<(u64, usize)> {
    let word = u64::from_le_bytes(word);
    let stops = !word & 0x8080_8080_8080_8080;
    if stops == 0 {
        return None;
    }
    let length = (stops.trailing_zeros() / 8 + 1) as usize;
    let mut value = 0_u64;
    for byte in 0..length {
        value |= (word >> (8 * byte) & 0x7f) << (7 * byte);
    }
    Some((value, length))
}

#[inline(always)]
pub(super) fn varint(bytes: &[u8], at: &mut usize, damaged: &mut bool) -> u64 {
    if let Some(word) = bytes.get(*at..*at + 8) {
        if word[0] < 0x80 {
            *at += 1;
            return u64::from(word[0]);
        }
        if word[1] < 0x80 {
            *at += 2;
            return u64::from(word[0] & 0x7f) | u64::from(word[1]) << 7;
        }
        if let Some((value, length)) = window(word.try_into().expect("eight bytes")) {
            *at += length;
            return value;
        }
    }
    long(bytes, at, damaged, 64) as u64
}

pub(super) fn skip(bytes: &[u8], mut at: usize, end: usize, mut count: usize) -> Option<usize> {
    let end = end.min(bytes.len());
    const HIGH: u64 = 0x8080_8080_8080_8080;
    while at + 64 <= end {
        let held: usize = bytes[at..at + 64].chunks_exact(8).map(|word| (!u64::from_le_bytes(word.try_into().expect("eight bytes")) & HIGH).count_ones() as usize).sum();
        if held >= count {
            break;
        }
        count -= held;
        at += 64;
    }
    while count > 0 && at + 8 <= end {
        let word = u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"));
        let mut stops = !word & 0x8080_8080_8080_8080;
        let held = stops.count_ones() as usize;
        if held < count {
            count -= held;
            at += 8;
            continue;
        }
        for _ in 1..count {
            stops &= stops - 1;
        }
        return Some(at + (stops.trailing_zeros() / 8 + 1) as usize);
    }
    while count > 0 {
        let &byte = bytes.get(at).filter(|_| at < end)?;
        at += 1;
        count -= usize::from(byte < 0x80);
    }
    Some(at)
}

#[inline(always)]
pub(super) fn hours(bytes: &[u8], at: &mut usize, damaged: &mut bool) -> u32 {
    let gap = varint(bytes, at, damaged);
    *damaged |= gap > u64::from(BLOCK);
    gap.min(u64::from(BLOCK) + 1) as u32
}

#[inline(always)]
pub(super) fn code(bytes: &[u8], at: &mut usize, damaged: &mut bool) -> (u64, u64) {
    if let Some(word) = bytes.get(*at..*at + 8) {
        if word[0] < 0x80 {
            *at += 1;
            return (u64::from(word[0] >> 1), u64::from(word[0] & 1));
        }
        if word[1] < 0x80 {
            *at += 2;
            let value = u64::from(word[0] & 0x7f) | u64::from(word[1]) << 7;
            return (value >> 1, value & 1);
        }
        if let Some((value, length)) = window(word.try_into().expect("eight bytes")) {
            *at += length;
            return (value >> 1, value & 1);
        }
    }
    let value = long(bytes, at, damaged, 65);
    ((value >> 1) as u64, (value & 1) as u64)
}

pub(super) trait Cells {
    fn get(&self, slot: usize) -> u64;
    fn put(&mut self, slot: usize, value: u64) -> bool;
}

struct Zero;

impl Cells for Zero {
    #[inline(always)]
    fn get(&self, _: usize) -> u64 {
        0
    }

    #[inline(always)]
    fn put(&mut self, _: usize, value: u64) -> bool {
        value == 0
    }
}

macro_rules! width {
    ($name:ident, $unsigned:ty, $signed:ty, $bytes:literal) => {
        struct $name<'a, const SIGNED: bool>(&'a mut [u8]);

        impl<const SIGNED: bool> Cells for $name<'_, SIGNED> {
            #[inline(always)]
            fn get(&self, slot: usize) -> u64 {
                let held = <$unsigned>::from_le_bytes(self.0[slot * $bytes..slot * $bytes + $bytes].try_into().expect("a cell"));
                if SIGNED { held as $signed as i64 as u64 } else { held as u64 }
            }

            #[inline(always)]
            fn put(&mut self, slot: usize, value: u64) -> bool {
                let fits = if SIGNED { value as i64 == value as $signed as i64 } else { value == value as $unsigned as u64 };
                if fits {
                    self.0[slot * $bytes..slot * $bytes + $bytes].copy_from_slice(&(value as $unsigned).to_le_bytes());
                }
                fits
            }
        }
    };
}

width!(One, u8, i8, 1);
width!(Two, u16, i16, 2);
width!(Four, u32, i32, 4);
width!(Eight, u64, i64, 8);

struct Truth<'a>(&'a mut Flags);

impl Cells for Truth<'_> {
    #[inline(always)]
    fn get(&self, slot: usize) -> u64 {
        u64::from(self.0.get(slot as u32))
    }

    #[inline(always)]
    fn put(&mut self, slot: usize, value: u64) -> bool {
        self.0.put(slot as u32, value != 0);
        true
    }
}

struct Text<C>(C);

impl<C: Cells> Cells for Text<C> {
    #[inline(always)]
    fn get(&self, slot: usize) -> u64 {
        self.0.get(slot)
    }

    #[inline(always)]
    fn put(&mut self, slot: usize, value: u64) -> bool {
        self.0.put(slot, value as u32 as u64)
    }
}

pub(super) struct Pass<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) from: u16,
    pub(super) to: u16,
    pub(super) inserted: bool,
    pub(super) born: &'a [u16],
}

#[derive(Default)]
pub(super) struct Count {
    pub(super) events: u64,
    pub(super) bytes: u64,
    pub(super) scanned: u64,
    pub(super) visited: u64,
    pub(super) damaged: bool,
}

enum Stop {
    Done,
    Widen(usize, u64),
    Bits(usize),
    Missing,
}

struct Bits<'a> {
    present: Option<&'a mut Flags>,
    unrecorded: &'a mut Option<Flags>,
}

struct Run<'a, 'p> {
    pass: &'p Pass<'a>,
    live: &'p Flags,
    slots: usize,
}

pub(super) fn advance(model: u8, tracks: &mut Tracks, born_group: bool, pass: &Pass<'_>, numbers: Numbers<'_>, written: &mut Vec<u32>, count: &mut Count) -> io::Result<()> {
    if tracks.slot.is_empty() || tracks.due > pass.to {
        return Ok(());
    }
    let Numbers { live, slots, cells, present, unrecorded } = numbers;
    if !born_group && tracks.max_slot as usize >= slots {
        return Err(invalid("A unit track names a slot past its table"));
    }
    let mut bits = Bits { present, unrecorded };
    let mut full = born_group || bits.present.is_some() || bits.unrecorded.is_some();
    let mut run = Run { pass, live, slots };
    let mut start = 0;
    match cells {
        NumberCells::Bool(flags) => loop {
            match models(model, full, born_group, &mut Truth(&mut *flags), tracks, start, &mut run, &mut bits, written, count) {
                Stop::Done => break,
                Stop::Bits(at) => (full, start) = (true, at),
                Stop::Widen(..) | Stop::Missing => return Err(invalid("A bool cell left missing")),
            }
        },
        NumberCells::Narrow { cells: narrow, text } => loop {
            let (width, signed) = (narrow.width(), narrow.signed());
            let held = narrow.cells_mut(slots);
            let stop = match (width, signed, text) {
                (0, _, false) => models(model, full, born_group, &mut Zero, tracks, start, &mut run, &mut bits, written, count),
                (1, false, false) => models(model, full, born_group, &mut One::<false>(held), tracks, start, &mut run, &mut bits, written, count),
                (1, true, false) => models(model, full, born_group, &mut One::<true>(held), tracks, start, &mut run, &mut bits, written, count),
                (2, false, false) => models(model, full, born_group, &mut Two::<false>(held), tracks, start, &mut run, &mut bits, written, count),
                (2, true, false) => models(model, full, born_group, &mut Two::<true>(held), tracks, start, &mut run, &mut bits, written, count),
                (4, false, false) => models(model, full, born_group, &mut Four::<false>(held), tracks, start, &mut run, &mut bits, written, count),
                (4, true, false) => models(model, full, born_group, &mut Four::<true>(held), tracks, start, &mut run, &mut bits, written, count),
                (_, _, false) => models(model, full, born_group, &mut Eight::<false>(held), tracks, start, &mut run, &mut bits, written, count),
                (0, _, true) => models(model, full, born_group, &mut Text(Zero), tracks, start, &mut run, &mut bits, written, count),
                (1, _, true) => models(model, full, born_group, &mut Text(One::<false>(held)), tracks, start, &mut run, &mut bits, written, count),
                (2, _, true) => models(model, full, born_group, &mut Text(Two::<false>(held)), tracks, start, &mut run, &mut bits, written, count),
                (4, _, true) => models(model, full, born_group, &mut Text(Four::<false>(held)), tracks, start, &mut run, &mut bits, written, count),
                (_, _, true) => models(model, full, born_group, &mut Text(Eight::<false>(held)), tracks, start, &mut run, &mut bits, written, count),
            };
            match stop {
                Stop::Done => break,
                Stop::Widen(at, value) => {
                    narrow.hold(if text { value as u32 as u64 } else { value }, slots);
                    start = at;
                }
                Stop::Bits(at) => (full, start) = (true, at),
                Stop::Missing => return Err(invalid("A unit track leaves a cell missing in a column whose cells are never missing")),
            }
        },
    }
    tracks.due = tracks.mins.iter().copied().min().unwrap_or(NEVER);
    Ok(())
}

#[inline(always)]
fn models<C: Cells>(model: u8, full: bool, born: bool, cells: &mut C, tracks: &mut Tracks, start: usize, run: &mut Run<'_, '_>, bits: &mut Bits<'_>, written: &mut Vec<u32>, count: &mut Count) -> Stop {
    match (model, full, born) {
        (RUNS, false, _) => runs::<C, false, false>(cells, tracks, start, run, bits, written, count),
        (RUNS, true, false) => runs::<C, true, false>(cells, tracks, start, run, bits, written, count),
        (RUNS, true, true) => runs::<C, true, true>(cells, tracks, start, run, bits, written, count),
        (RAW, false, _) => plain::<RAW, C, false, false>(cells, tracks, start, run, bits, written, count),
        (RAW, true, false) => plain::<RAW, C, true, false>(cells, tracks, start, run, bits, written, count),
        (RAW, true, true) => plain::<RAW, C, true, true>(cells, tracks, start, run, bits, written, count),
        (SECOND, false, _) => plain::<SECOND, C, false, false>(cells, tracks, start, run, bits, written, count),
        (SECOND, true, false) => plain::<SECOND, C, true, false>(cells, tracks, start, run, bits, written, count),
        (SECOND, true, true) => plain::<SECOND, C, true, true>(cells, tracks, start, run, bits, written, count),
        (_, false, _) => plain::<DIFFERENCES, C, false, false>(cells, tracks, start, run, bits, written, count),
        (_, true, false) => plain::<DIFFERENCES, C, true, false>(cells, tracks, start, run, bits, written, count),
        (_, true, true) => plain::<DIFFERENCES, C, true, true>(cells, tracks, start, run, bits, written, count),
    }
}

#[inline(always)]
fn held<C: Cells, const BITS: bool>(cells: &C, bits: &Bits<'_>, slot: usize) -> (u64, u8) {
    if !BITS {
        return (cells.get(slot), 0);
    }
    if bits.unrecorded.as_ref().is_some_and(|held| held.get(slot as u32)) {
        (0, 2)
    } else if bits.present.as_ref().is_some_and(|held| !held.get(slot as u32)) {
        (0, 1)
    } else {
        (cells.get(slot), 0)
    }
}

#[inline(always)]
fn commit<C: Cells, const BITS: bool>(cells: &mut C, bits: &mut Bits<'_>, slot: usize, value: u64, state: u8, index: usize) -> Option<Stop> {
    if !BITS {
        return (!cells.put(slot, value)).then_some(Stop::Widen(index, value));
    }
    match state {
        0 => {
            if !cells.put(slot, value) {
                return Some(Stop::Widen(index, value));
            }
            if let Some(present) = bits.present.as_deref_mut() {
                present.set(slot as u32);
            }
            if let Some(unrecorded) = bits.unrecorded.as_mut() {
                unrecorded.clear(slot as u32);
            }
        }
        1 => {
            let Some(present) = bits.present.as_deref_mut() else { return Some(Stop::Missing) };
            cells.put(slot, 0);
            present.clear(slot as u32);
            if let Some(unrecorded) = bits.unrecorded.as_mut() {
                unrecorded.clear(slot as u32);
            }
        }
        _ => bits.unrecorded.get_or_insert_with(Flags::default).set(slot as u32),
    }
    None
}

#[inline(always)]
fn born_track(tracks: &Tracks, index: usize, run: &Run<'_, '_>, damaged: &mut bool) -> Option<bool> {
    let born = tracks.born[index];
    let now = born >= run.pass.from;
    if now != run.pass.inserted {
        return None;
    }
    let slot = tracks.slot[index];
    if slot as usize >= run.slots {
        *damaged = true;
        return Some(true);
    }
    Some(now && (!run.live.get(slot) || run.pass.born.get(slot as usize).copied() != Some(born)))
}

macro_rules! leave {
    ($count:expr, $damaged:expr, $events:expr, $bytes:expr, $scanned:expr, $visited:expr, $stop:expr) => {{
        $count.damaged |= $damaged;
        $count.events += $events;
        $count.bytes += $bytes;
        $count.scanned += $scanned;
        $count.visited += $visited;
        return $stop;
    }};
}

#[inline(never)]
fn plain<const MODEL: u8, C: Cells, const BITS: bool, const BORN: bool>(
    cells: &mut C,
    tracks: &mut Tracks,
    start: usize,
    run: &mut Run<'_, '_>,
    bits: &mut Bits<'_>,
    written: &mut Vec<u32>,
    count: &mut Count,
) -> Stop {
    let bytes = run.pass.bytes;
    let to = u32::from(run.pass.to);
    let (mut damaged, mut all_events, mut took, mut scanned, mut visited) = (false, 0_u64, 0_u64, 0_u64, 0_u64);
    let mut group = start / GROUP;
    while group * GROUP < tracks.slot.len() {
        let begin = group * GROUP;
        let until = (begin + GROUP).min(tracks.slot.len());
        if u32::from(tracks.mins[group]) > to {
            group += 1;
            continue;
        }
        scanned += (until - begin) as u64;
        let mut low = NEVER;
        for index in begin..until {
            let next = tracks.next[index];
            if u32::from(next) > to {
                low = low.min(next);
                continue;
            }
            if BORN {
                match born_track(tracks, index, run, &mut damaged) {
                    None => {
                        low = low.min(next);
                        continue;
                    }
                    Some(true) => {
                        tracks.at[index] = tracks.end[index];
                        tracks.next[index] = NEVER;
                        continue;
                    }
                    Some(false) => {}
                }
            }
            let slot = tracks.slot[index] as usize;
            let (mut value, mut state) = held::<C, BITS>(cells, bits, slot);
            let (mut at, end) = (tracks.at[index] as usize, tracks.end[index] as usize);
            let mut last = if MODEL == SECOND { tracks.last[index] } else { 0 };
            let mut hour = u32::from(next);
            let mut events = 0;
            loop {
                let (half, odd) = code(bytes, &mut at, &mut damaged);
                if odd == 0 {
                    let number = unzig(half) as u64;
                    let old = if BITS && state != 0 { 0 } else { value };
                    value = match MODEL {
                        RAW => number,
                        SECOND => {
                            last = last.wrapping_add(number);
                            old.wrapping_add(last)
                        }
                        _ => old.wrapping_add(number),
                    };
                    if BITS {
                        state = 0;
                    }
                } else {
                    if !BITS {
                        leave!(count, damaged, all_events, took, scanned, visited, Stop::Bits(index));
                    }
                    damaged |= half > 1;
                    value = 0;
                    last = 0;
                    state = 1 + (half & 1) as u8;
                }
                events += 1;
                if at >= end {
                    damaged |= at != end;
                    hour = u32::from(NEVER);
                    break;
                }
                hour += hours(bytes, &mut at, &mut damaged);
                if hour > to {
                    break;
                }
            }
            if let Some(stop) = commit::<C, BITS>(cells, bits, slot, value, state, index) {
                leave!(count, damaged, all_events, took, scanned, visited, stop);
            }
            written.push(slot as u32);
            all_events += events;
            visited += 1;
            took += u64::from(at as u32 - tracks.at[index]);
            tracks.at[index] = at as u32;
            tracks.next[index] = hour as u16;
            if MODEL == SECOND {
                tracks.last[index] = last;
            }
            low = low.min(hour as u16);
        }
        tracks.mins[group] = low;
        group += 1;
    }
    leave!(count, damaged, all_events, took, scanned, visited, Stop::Done)
}

#[inline(never)]
fn runs<C: Cells, const BITS: bool, const BORN: bool>(
    cells: &mut C,
    tracks: &mut Tracks,
    start: usize,
    run: &mut Run<'_, '_>,
    bits: &mut Bits<'_>,
    written: &mut Vec<u32>,
    count: &mut Count,
) -> Stop {
    let bytes = run.pass.bytes;
    let to = u32::from(run.pass.to);
    let (mut damaged, mut all_events, mut took, mut scanned, mut visited) = (false, 0_u64, 0_u64, 0_u64, 0_u64);
    let mut group = start / GROUP;
    while group * GROUP < tracks.slot.len() {
        let begin = group * GROUP;
        let until = (begin + GROUP).min(tracks.slot.len());
        if u32::from(tracks.mins[group]) > to {
            group += 1;
            continue;
        }
        scanned += (until - begin) as u64;
        let mut low = NEVER;
        for index in begin..until {
            let next = tracks.next[index];
            if u32::from(next) > to {
                low = low.min(next);
                continue;
            }
            if BORN {
                match born_track(tracks, index, run, &mut damaged) {
                    None => {
                        low = low.min(next);
                        continue;
                    }
                    Some(true) => {
                        tracks.at[index] = tracks.end[index];
                        tracks.next[index] = NEVER;
                        continue;
                    }
                    Some(false) => {}
                }
            }
            let slot = tracks.slot[index] as usize;
            let (mut value, mut state) = held::<C, BITS>(cells, bits, slot);
            let (mut at, end) = (tracks.at[index] as usize, tracks.end[index] as usize);
            let (mut left, mut gap, mut payload) = (tracks.left[index], u32::from(tracks.gap[index]), tracks.payload[index] as usize);
            let mut hour = u32::from(next);
            let mut events = 0;
            loop {
                let (half, odd) = if left == 0 {
                    payload = at;
                    let code = code(bytes, &mut at, &mut damaged);
                    let repeats = varint(bytes, &mut at, &mut damaged);
                    damaged |= repeats > u64::from(BLOCK);
                    left = repeats.min(u64::from(BLOCK)) as u32;
                    code
                } else {
                    left -= 1;
                    let mut again = payload;
                    code(bytes, &mut again, &mut damaged)
                };
                if odd == 0 {
                    let old = if BITS && state != 0 { 0 } else { value };
                    value = old.wrapping_add(unzig(half) as u64);
                    if BITS {
                        state = 0;
                    }
                } else {
                    if !BITS {
                        leave!(count, damaged, all_events, took, scanned, visited, Stop::Bits(index));
                    }
                    damaged |= half > 1;
                    value = 0;
                    state = 1 + (half & 1) as u8;
                }
                events += 1;
                if left != 0 {
                    let due = if gap == 0 { left } else { ((to - hour) / gap).min(left) };
                    if odd == 0 {
                        value = value.wrapping_add((unzig(half) as u64).wrapping_mul(u64::from(due)));
                    }
                    left -= due;
                    events += u64::from(due);
                    hour += gap * due;
                    if left != 0 {
                        hour += gap;
                        break;
                    }
                }
                if at >= end {
                    damaged |= at != end;
                    hour = u32::from(NEVER);
                    break;
                }
                gap = hours(bytes, &mut at, &mut damaged);
                hour += gap;
                if hour > to {
                    break;
                }
            }
            if let Some(stop) = commit::<C, BITS>(cells, bits, slot, value, state, index) {
                leave!(count, damaged, all_events, took, scanned, visited, stop);
            }
            written.push(slot as u32);
            all_events += events;
            visited += 1;
            took += u64::from(at as u32 - tracks.at[index]);
            tracks.at[index] = at as u32;
            tracks.next[index] = hour.min(u32::from(NEVER)) as u16;
            tracks.left[index] = left;
            tracks.gap[index] = gap as u16;
            tracks.payload[index] = payload as u32;
            low = low.min(hour.min(u32::from(NEVER)) as u16);
        }
        tracks.mins[group] = low;
        group += 1;
    }
    leave!(count, damaged, all_events, took, scanned, visited, Stop::Done)
}

pub(super) fn track(model: u8, bytes: &[u8], mut at: usize, end: usize, mut value: u64, mut state: u8) -> Option<(u64, u8)> {
    let mut damaged = false;
    let mut last = 0_u64;
    loop {
        let (half, odd) = code(bytes, &mut at, &mut damaged);
        let repeats = if model == RUNS { varint(bytes, &mut at, &mut damaged).min(u64::from(BLOCK)) } else { 0 };
        for _ in 0..=repeats {
            if odd == 0 {
                let number = unzig(half) as u64;
                let old = if state != 0 { 0 } else { value };
                value = match model {
                    RAW => number,
                    SECOND => {
                        last = last.wrapping_add(number);
                        old.wrapping_add(last)
                    }
                    _ => old.wrapping_add(number),
                };
                state = 0;
            } else {
                damaged |= half > 1;
                (value, last, state) = (0, 0, 1 + (half & 1) as u8);
            }
        }
        if at >= end {
            return (!damaged && at == end).then_some((value, state));
        }
        hours(bytes, &mut at, &mut damaged);
    }
}

pub(super) fn inserted(bytes: &[u8], at: &mut usize, end: usize, numbers: Numbers<'_>, inserted: &[u32]) -> io::Result<()> {
    let Numbers { slots, mut cells, mut present, unrecorded, .. } = numbers;
    let mut damaged = false;
    for &slot in inserted {
        if *at >= end {
            return Err(invalid("A part inserts more rows than its column holds values"));
        }
        if slot == super::SKIP {
            code(bytes, at, &mut damaged);
            continue;
        }
        if slot as usize >= slots {
            return Err(invalid("An inserted value for a slot past its table"));
        }
        let (half, odd) = code(bytes, at, &mut damaged);
        if odd == 1 && half == 1 {
            unrecorded.get_or_insert_with(Flags::default).set(slot);
            continue;
        }
        let value = if odd == 1 {
            let Some(present) = present.as_deref_mut() else {
                return Err(invalid("An inserted cell left missing in a column whose cells are never missing"));
            };
            damaged |= half != 0;
            present.clear(slot);
            0
        } else {
            if let Some(present) = present.as_deref_mut() {
                present.set(slot);
            }
            unzig(half) as u64
        };
        match &mut cells {
            NumberCells::Narrow { cells, text } => cells.set(slot, if *text { value as u32 as u64 } else { value }, slots),
            NumberCells::Bool(flags) => flags.put(slot, value != 0),
        }
        if let Some(unrecorded) = unrecorded.as_mut() {
            unrecorded.clear(slot);
        }
    }
    if damaged || *at > end {
        return Err(invalid("A unit stream's inserted values are damaged"));
    }
    Ok(())
}
