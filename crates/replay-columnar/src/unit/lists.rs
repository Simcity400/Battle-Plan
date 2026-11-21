
use super::{get_varint, put_varint, unzig, zig};
use crate::invalid;
use std::io;

pub const LITERAL: u8 = 0;
pub const PATCH: u8 = 1;
pub const EDIT: u8 = 2;
pub const GONE: u8 = 3;
pub const SAME: u8 = 4;
pub const POOLED: u8 = 5;
pub const PIECES: u8 = 6;

#[derive(Default)]
pub struct Matcher {
    alt: Vec<u8>,
    table: Vec<u32>,
    pieces: Vec<(bool, u32, u32, u32, u32)>,
    patches: Vec<(u32, u8, i64)>,
    runs: Vec<u8>,
}

fn difference(new: &[u8], old: &[u8], wide: usize) -> i64 {
    let (mut x, mut y) = (0_u64, 0_u64);
    for step in 0..wide {
        x |= u64::from(new[step]) << (8 * step);
        y |= u64::from(old[step]) << (8 * step);
    }
    let mut difference = x.wrapping_sub(y);
    if wide < 8 {
        let mask = (1_u64 << (8 * wide)) - 1;
        difference &= mask;
        if difference >> (8 * wide - 1) != 0 {
            difference |= !mask;
        }
    }
    difference as i64
}

fn pieces(new: &[u8], old: &[u8], matcher: &mut Matcher) {
    let Matcher { alt, table, pieces, patches, .. } = matcher;
    patches.clear();
    let bits = (usize::BITS - (old.len() / 4 + 1).leading_zeros() + 1).max(6);
    table.clear();
    table.resize(1 << bits, u32::MAX);
    let key = |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"));
    let place = |key: u64| (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - bits)) as usize;
    let mut at = 0;
    while at + 8 <= old.len() {
        table[place(key(old, at))] = at as u32;
        at += 4;
    }
    pieces.clear();
    let (mut at, mut kept) = (0_usize, 0_usize);
    while at + 8 <= new.len() {
        let candidate = table[place(key(new, at))];
        if candidate != u32::MAX {
            let from = candidate as usize;
            let first_patch = patches.len();
            let (mut length, mut differing) = (0, 0);
            loop {
                while at + length < new.len() && from + length < old.len() && new[at + length] == old[from + length] {
                    length += 1;
                }
                let (a, o) = (at + length, from + length);
                if a >= new.len() || o >= old.len() {
                    break;
                }
                let mut wide = 1;
                while wide < 8 && a + wide < new.len() && o + wide < old.len() && new[a + wide] != old[o + wide] {
                    wide += 1;
                }
                if a + wide + 8 > new.len() || o + wide + 8 > old.len() || new[a + wide..a + wide + 8] != old[o + wide..o + wide + 8] {
                    break;
                }
                patches.push((length as u32, wide as u8, difference(&new[a..], &old[o..], wide)));
                length += wide;
                differing += wide;
            }
            let mut back = 0;
            while back < at - kept && back < from && new[at - 1 - back] == old[from - 1 - back] {
                back += 1;
            }
            if length + back - differing >= 16 {
                if at - back > kept {
                    pieces.push((true, kept as u32, (at - back - kept) as u32, 0, 0));
                }
                for patch in &mut patches[first_patch..] {
                    patch.0 += back as u32;
                }
                pieces.push((false, (from - back) as u32, (length + back) as u32, first_patch as u32, (patches.len() - first_patch) as u32));
                at += length;
                kept = at;
                continue;
            }
            patches.truncate(first_patch);
        }
        at += 1;
    }
    if new.len() > kept {
        pieces.push((true, kept as u32, (new.len() - kept) as u32, 0, 0));
    }
    alt.clear();
    alt.push(PIECES);
    put_varint(alt, pieces.len() as u64);
    let mut expected = 0_i64;
    for &(inserted, start, length, first_patch, count) in pieces.iter() {
        if inserted {
            put_varint(alt, u64::from(length) << 1 | 1);
            alt.extend_from_slice(&new[start as usize..(start + length) as usize]);
        } else {
            put_varint(alt, u64::from(length) << 1);
            put_varint(alt, zig(i64::from(start) - expected));
            expected = i64::from(start + length);
            put_varint(alt, u64::from(count));
            let mut last = 0_u32;
            for &(place, wide, difference) in &patches[first_patch as usize..(first_patch + count) as usize] {
                put_varint(alt, u64::from(place - last) << 3 | (u64::from(wide) - 1));
                put_varint(alt, zig(difference));
                last = place + u32::from(wide);
            }
        }
    }
}

pub fn against(new: &[u8], old: &[u8], out: &mut Vec<u8>, matcher: &mut Matcher) -> bool {
    out.clear();
    if new.len() == old.len() {
        out.push(PATCH);
        let runs = &mut matcher.runs;
        runs.clear();
        let (mut count, mut last, mut at) = (0_u64, 0_usize, 0_usize);
        while at < new.len() {
            if new[at] == old[at] {
                at += 1;
                continue;
            }
            let mut end = at + 1;
            while end < new.len() && end - at < 8 && new[end] != old[end] {
                end += 1;
            }
            let wide = end - at;
            put_varint(runs, ((at - last) as u64) << 3 | (wide as u64 - 1));
            put_varint(runs, zig(difference(&new[at..], &old[at..], wide)));
            count += 1;
            last = end;
            at = end;
        }
        put_varint(out, count);
        out.extend_from_slice(runs);
    } else {
        let limit = new.len().min(old.len());
        let mut front = 0;
        while front < limit && new[front] == old[front] {
            front += 1;
        }
        let mut back = 0;
        while back < limit - front && new[new.len() - 1 - back] == old[old.len() - 1 - back] {
            back += 1;
        }
        out.push(EDIT);
        put_varint(out, front as u64);
        put_varint(out, back as u64);
        put_varint(out, (new.len() - front - back) as u64);
        out.extend_from_slice(&new[front..new.len() - back]);
    }
    if out.len() > 48 && old.len() >= 32 && new.len() >= 32 {
        pieces(new, old, matcher);
        if matcher.alt.len() < out.len() {
            std::mem::swap(out, &mut matcher.alt);
        }
    }
    out.len() < new.len() + 2
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, length: usize) -> io::Result<&'a [u8]> {
    let end = at.checked_add(length).filter(|&end| end <= bytes.len()).ok_or_else(|| invalid("A list event ends inside its bytes"))?;
    let taken = &bytes[*at..end];
    *at = end;
    Ok(taken)
}

fn patch_bytes(cell: &mut [u8], place: usize, wide: usize, difference: u64) -> io::Result<()> {
    let target = cell.get_mut(place..place + wide).ok_or_else(|| invalid("A list patch passes its cell"))?;
    let mut held = 0_u64;
    for (step, byte) in target.iter().enumerate() {
        held |= u64::from(*byte) << (8 * step);
    }
    let new = held.wrapping_add(difference);
    for (step, byte) in target.iter_mut().enumerate() {
        *byte = (new >> (8 * step)) as u8;
    }
    Ok(())
}

pub fn patch(bytes: &[u8], mut at: usize, cell: &mut [u8]) -> io::Result<usize> {
    let count = get_varint(bytes, &mut at)?;
    let mut place = 0_usize;
    for _ in 0..count {
        let head = get_varint(bytes, &mut at)?;
        let wide = (head & 7) as usize + 1;
        place += (head >> 3) as usize;
        let difference = unzig(get_varint(bytes, &mut at)?) as u64;
        patch_bytes(cell, place, wide, difference)?;
        place += wide;
    }
    Ok(at)
}

pub fn rebuild(form: u8, bytes: &[u8], mut at: usize, old: &[u8], out: &mut Vec<u8>) -> io::Result<usize> {
    out.clear();
    match form {
        PIECES => {
            let count = get_varint(bytes, &mut at)?;
            let mut expected = 0_i64;
            for _ in 0..count {
                let head = get_varint(bytes, &mut at)?;
                let length = (head >> 1) as usize;
                if head & 1 == 1 {
                    out.extend_from_slice(take(bytes, &mut at, length)?);
                } else {
                    let from = usize::try_from(expected + unzig(get_varint(bytes, &mut at)?)).map_err(|_| invalid("A copied piece starts before its cell"))?;
                    let base = out.len();
                    out.extend_from_slice(old.get(from..from + length).ok_or_else(|| invalid("A copied piece passes its cell"))?);
                    expected = (from + length) as i64;
                    let mut place = base;
                    for _ in 0..get_varint(bytes, &mut at)? {
                        let head = get_varint(bytes, &mut at)?;
                        let wide = (head & 7) as usize + 1;
                        place += (head >> 3) as usize;
                        let difference = unzig(get_varint(bytes, &mut at)?) as u64;
                        patch_bytes(out, place, wide, difference)?;
                        place += wide;
                    }
                }
            }
        }
        EDIT => {
            let front = get_varint(bytes, &mut at)? as usize;
            let back = get_varint(bytes, &mut at)? as usize;
            let middle = get_varint(bytes, &mut at)? as usize;
            if front + back > old.len() {
                return Err(invalid("An edit keeps more than its cell"));
            }
            out.extend_from_slice(&old[..front]);
            out.extend_from_slice(take(bytes, &mut at, middle)?);
            out.extend_from_slice(&old[old.len() - back..]);
        }
        other => return Err(invalid(format!("List form {other} rebuilds no cell"))),
    }
    Ok(at)
}

pub fn skip(bytes: &[u8], at: &mut usize, literal_is_reference: bool) -> io::Result<()> {
    let form = *bytes.get(*at).ok_or_else(|| invalid("A list event ends before its form"))?;
    *at += 1;
    match form {
        LITERAL if literal_is_reference => {
            get_varint(bytes, at)?;
        }
        LITERAL => {
            let length = get_varint(bytes, at)? as usize;
            take(bytes, at, length)?;
        }
        POOLED => {
            get_varint(bytes, at)?;
        }
        PATCH => {
            for _ in 0..get_varint(bytes, at)? {
                get_varint(bytes, at)?;
                get_varint(bytes, at)?;
            }
        }
        EDIT => {
            get_varint(bytes, at)?;
            get_varint(bytes, at)?;
            let middle = get_varint(bytes, at)? as usize;
            take(bytes, at, middle)?;
        }
        PIECES => {
            for _ in 0..get_varint(bytes, at)? {
                let head = get_varint(bytes, at)?;
                if head & 1 == 1 {
                    take(bytes, at, (head >> 1) as usize)?;
                } else {
                    get_varint(bytes, at)?;
                    for _ in 0..get_varint(bytes, at)? {
                        get_varint(bytes, at)?;
                        get_varint(bytes, at)?;
                    }
                }
            }
        }
        GONE | SAME => {}
        other => return Err(invalid(format!("List form {other}"))),
    }
    Ok(())
}
