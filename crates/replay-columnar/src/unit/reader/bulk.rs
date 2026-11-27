
use super::numbers;
use crate::{
    invalid,
    state::Kind,
    store::{Flags, NumberCells, Numbers, PAGE, TableState, Value},
    unit::{get_wide, unzig, unzig_wide},
};
use std::io;

#[inline(always)]
fn each_live(live: &Flags, from: u32, to: u32, mut each: impl FnMut(u32)) {
    let words = live.words();
    let last = (to as usize).div_ceil(64).min(words.len());
    for index in from as usize / 64..last {
        let base = index as u32 * 64;
        let mut rest = words[index];
        if base < from {
            rest &= u64::MAX << (from - base);
        }
        if to - base < 64 {
            rest &= (1_u64 << (to - base)) - 1;
        }
        while rest != 0 {
            each(base + rest.trailing_zeros());
            rest &= rest - 1;
        }
    }
}

fn live_count(live: &Flags, from: u32, to: u32) -> usize {
    let mut count = 0;
    each_live(live, from, to, |_| count += 1);
    count
}

#[inline(always)]
fn wide(bytes: &[u8], at: &mut usize) -> io::Result<u128> {
    let mut damaged = false;
    if bytes.len() >= *at + 9 {
        let mut probe = *at;
        let value = numbers::varint(bytes, &mut probe, &mut damaged);
        if !damaged && probe - *at < 10 {
            *at = probe;
            return Ok(u128::from(value));
        }
    }
    get_wide(bytes, at)
}

fn bit(bits: &[u8], offset: u32) -> bool {
    bits[(offset / 8) as usize] >> (offset % 8) & 1 == 1
}

pub(crate) fn rows(bytes: &[u8], from: u32, to: u32, table: &mut TableState, keys: &mut Vec<u128>) -> io::Result<()> {
    let count = to.checked_sub(from).ok_or_else(|| invalid("A restore's rows chunk ends before it starts"))?;
    let bits = bytes.get(..count.div_ceil(8) as usize).ok_or_else(|| invalid("A restore's rows end inside their bits"))?;
    let mut at = bits.len();
    let mut previous = 0_u128;
    keys.clear();
    for _ in 0..bits.iter().map(|byte| byte.count_ones()).sum::<u32>() {
        previous = previous.wrapping_add(unzig_wide(wide(bytes, &mut at)?) as u128);
        keys.push(previous);
    }
    if at != bytes.len() {
        return Err(invalid("A restore's rows hold bytes past their keys"));
    }
    table.put_rows(from, to, bits, keys)
}

pub(crate) fn changed_rows(bytes: &[u8], from: u32, to: u32, whole: &[u8], whole_to: u32, table: &mut TableState, keys: &mut Vec<u128>) -> io::Result<()> {
    let count = to.checked_sub(from).ok_or_else(|| invalid("A restore's rows chunk ends before it starts"))?;
    let bits = bytes.get(..count.div_ceil(8) as usize).ok_or_else(|| invalid("A restore's rows end inside their bits"))?;
    let mut at = bits.len();
    let (mut last, mut previous) = (from, 0_u128);
    let mut changed = |at: &mut usize| -> io::Result<Option<(u32, u128)>> {
        if *at >= bytes.len() {
            return Ok(None);
        }
        last = u32::try_from(u64::from(last) + crate::unit::get_varint(bytes, at)?).map_err(|_| invalid("A restore's row names a slot past 32 bits"))?;
        previous = previous.wrapping_add(unzig_wide(get_wide(bytes, at)?) as u128);
        Ok(Some((last, previous)))
    };
    let mut next = changed(&mut at)?;
    let held = whole_to.saturating_sub(from);
    let whole_bits = whole.get(..held.div_ceil(8) as usize).ok_or_else(|| invalid("A whole restore's rows end inside their bits"))?;
    let (mut whole_at, mut whole_key) = (whole_bits.len(), 0_u128);
    keys.clear();
    for offset in 0..count.max(held) {
        let was = if offset < held && bit(whole_bits, offset) {
            whole_key = whole_key.wrapping_add(unzig_wide(get_wide(whole, &mut whole_at)?) as u128);
            Some(whole_key)
        } else {
            None
        };
        if offset >= count || !bit(bits, offset) {
            continue;
        }
        let key = match next {
            Some((named, key)) if named == from + offset => {
                next = changed(&mut at)?;
                key
            }
            _ => was.ok_or_else(|| invalid("A restore's row is named neither in its chunk nor in its whole one"))?,
        };
        keys.push(key);
    }
    if whole_at != whole.len() {
        return Err(invalid("A whole restore's rows hold bytes past their keys"));
    }
    if next.is_some() || at != bytes.len() {
        return Err(invalid("A restore's rows name a slot that holds no row"));
    }
    table.put_rows(from, to, bits, keys)
}

fn scatter<const W: usize>(cells: &mut [u8], live: &Flags, from: u32, to: u32, values: &[u64]) {
    let mut values = values.iter();
    each_live(live, from, to, |slot| {
        let value = *values.next().expect("a value a live slot");
        let at = slot as usize * W;
        cells[at..at + W].copy_from_slice(&value.to_le_bytes()[..W]);
    });
}

pub(crate) fn selected_cells(
    bytes: &[u8], end: usize, range: (u32, u32), column: usize, live: &Flags,
    compact: &mut TableState, slots: &[(u32, u32)],
) -> io::Result<()> {
    let kind = compact.column(column).expect("a selected column").kind();
    let list = matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque);
    let optional = compact.numbers(column).is_some_and(|cells| cells.present.is_some());
    let mut selected = slots[slots.partition_point(|&(slot, _)| slot < range.0)..].iter().peekable();
    let (mut at, mut damaged, mut result) = (0, false, Ok(()));
    each_live(live, range.0, range.1, |slot| {
        if result.is_err() || damaged { return; }
        let value = if list {
            let head = numbers::varint(&bytes[..end], &mut at, &mut damaged);
            let start = at;
            at = at.saturating_add((head >> 1).min(u64::from(u32::MAX)) as usize);
            if damaged || at > end {
                result = Err(invalid("A restore's list cell passes its chunk"));
                return;
            }
            if head & 1 == 0 { Value::List(&bytes[start..at]) } else { Value::Unrecorded }
        } else {
            let (half, odd) = numbers::code(&bytes[..bytes.len().min(end + 8)], &mut at, &mut damaged);
            match (odd, half, kind) {
                (0, _, Kind::Bytes) => Value::Text(unzig(half) as u32),
                (0, _, _) => Value::Number(unzig(half) as u64),
                (_, 0, _) if optional => Value::Missing,
                (_, 0, _) => {
                    result = Err(invalid(format!("A restore leaves a cell missing in a {kind:?} column, whose cells are never missing")));
                    return;
                }
                (_, 1, _) => Value::Unrecorded,
                _ => { damaged = true; return; }
            }
        };
        if let Some(&&(old, new)) = selected.peek() && old == slot {
            selected.next();
            result = compact.put_snapshot_cell(new, column, value);
        }
    });
    result?;
    if damaged || at != end { return Err(invalid("A restore's column holds bytes past its cells")); }
    Ok(())
}

pub(crate) fn numbers(bytes: &[u8], end: usize, from: u32, to: u32, column: usize, table: &mut TableState, values: &mut Vec<u64>) -> io::Result<()> {
    let bytes = &bytes[..bytes.len().min(end + 8)];
    let kind = table.column(column).map(|held| held.kind()).ok_or_else(|| invalid("A restore names a column past its table"))?;
    let Numbers { live, slots, cells, mut present, unrecorded } = table.numbers(column).ok_or_else(|| invalid("A restore's number chunk for a column of lists"))?;
    let text = kind == Kind::Bytes;
    let signed = matches!(&cells, NumberCells::Narrow { cells, .. } if cells.signed());
    if let Some(present) = present.as_deref_mut() {
        present.include(live, from, to);
    }
    let (mut damaged, mut refused) = (false, false);
    let mut at = 0;
    values.clear();
    values.reserve(live_count(live, from, to));
    each_live(live, from, to, |slot| {
        let (half, odd) = numbers::code(bytes, &mut at, &mut damaged);
        if odd == 0 {
            let value = unzig(half) as u64;
            values.push(if text { value as u32 as u64 } else { value });
            return;
        }
        values.push(0);
        match half {
            0 => match present.as_deref_mut() {
                Some(present) => present.clear(slot),
                None => refused = true,
            },
            1 => {
                if let Some(present) = present.as_deref_mut() {
                    present.clear(slot);
                }
                unrecorded.get_or_insert_with(Flags::default).set(slot);
            }
            _ => damaged = true,
        }
    });
    if refused {
        return Err(invalid(format!("A restore leaves a cell missing in a {kind:?} column, whose cells are never missing")));
    }
    if damaged || at != end {
        return Err(invalid("A restore's column holds bytes past its cells"));
    }
    match cells {
        NumberCells::Bool(flags) => {
            let mut values = values.iter();
            each_live(live, from, to, |slot| {
                if *values.next().expect("a value a live slot") != 0 {
                    flags.set(slot);
                }
            });
        }
        NumberCells::Narrow { cells, .. } => {
            if signed {
                let least = values.iter().map(|&value| value as i64).min().unwrap_or(0);
                let most = values.iter().map(|&value| value as i64).max().unwrap_or(0);
                cells.hold(least as u64, slots);
                cells.hold(most as u64, slots);
            } else {
                cells.hold(values.iter().fold(0, |held, &value| held | value), slots);
            }
            let width = cells.width();
            let held = cells.cells_mut(slots);
            match width {
                0 => {}
                1 => scatter::<1>(held, live, from, to, values),
                2 => scatter::<2>(held, live, from, to, values),
                4 => scatter::<4>(held, live, from, to, values),
                _ => scatter::<8>(held, live, from, to, values),
            }
        }
    }
    Ok(())
}

pub(crate) fn lists(bytes: &[u8], end: usize, from: u32, to: u32, column: usize, table: &mut TableState, cells: &mut Vec<(u16, u32, u32)>) -> io::Result<()> {
    let bytes = &bytes[..end];
    let mut lists = table.lists(column).ok_or_else(|| invalid("A restore's list chunk for a column of numbers"))?;
    let live = lists.live;
    let mut damaged = false;
    let mut at = 0;
    let mut page = None;
    let mut unrecorded = Vec::new();
    let mut failed = None;
    cells.clear();
    each_live(live, from, to, |slot| {
        if failed.is_some() {
            return;
        }
        let head = numbers::varint(bytes, &mut at, &mut damaged);
        let start = at as u32;
        at = at.saturating_add((head >> 1).min(u64::from(u32::MAX)) as usize);
        if damaged || at > end {
            failed = Some(invalid("A restore's list cell passes its chunk"));
            return;
        }
        let held = slot as usize / PAGE;
        if page != Some(held) {
            if let Some(page) = page
                && let Err(error) = lists.fill(page, cells, bytes)
            {
                failed = Some(error);
                return;
            }
            cells.clear();
            page = Some(held);
        }
        cells.push(((slot as usize % PAGE) as u16, start, (head >> 1) as u32));
        if head & 1 == 1 {
            unrecorded.push(slot);
        }
    });
    if let Some(error) = failed {
        return Err(error);
    }
    if let Some(page) = page {
        lists.fill(page, cells, bytes)?;
    }
    if at != end {
        return Err(invalid("A restore's column holds bytes past its cells"));
    }
    for slot in unrecorded {
        lists.record(slot, false);
    }
    Ok(())
}
