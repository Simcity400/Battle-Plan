use std::collections::HashMap;

const HEADER: usize = 12;
const BLOCK: usize = 16;
const LITERAL: u32 = 1 << 31;

fn fingerprint(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5, |hash, &byte| (hash ^ u32::from(byte)).wrapping_mul(0x01000193))
}

fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn literal(out: &mut Vec<u8>, bytes: &[u8]) {
    if !bytes.is_empty() {
        word(out, bytes.len() as u32 | LITERAL);
        out.extend_from_slice(bytes);
    }
}

pub(super) fn encode(base: &[u8], next: &[u8]) -> Option<Vec<u8>> {
    if base.len() < BLOCK || next.len() <= HEADER + 8 || next.len() > crate::lead::BYTES / 2 || base.len() > crate::lead::BYTES / 2 {
        return None;
    }
    let mut blocks = HashMap::with_capacity(base.len() / BLOCK);
    for (i, chunk) in base.chunks_exact(BLOCK).enumerate() {
        blocks.entry(chunk).or_insert(i * BLOCK);
    }
    let mut out = vec![0; HEADER];
    let (mut cursor, mut start, mut aligned) = (0, 0, 0);
    while let Some(chunk) = next.get(cursor..cursor + BLOCK) {
        let matched = if base.get(aligned..aligned + BLOCK) == Some(chunk) { Some(aligned) } else { blocks.get(chunk).copied() };
        let Some(at) = matched else {
            cursor += 1;
            aligned += 1;
            continue;
        };
        let run = BLOCK + base[at + BLOCK..].iter().zip(&next[cursor + BLOCK..]).take_while(|(a, b)| a == b).count();
        literal(&mut out, &next[start..cursor]);
        word(&mut out, run as u32);
        word(&mut out, at as u32);
        if out.len().next_multiple_of(8) >= next.len().next_multiple_of(8) { return None; }
        cursor += run;
        start = cursor;
        aligned = at + run;
    }
    literal(&mut out, &next[start..]);
    if out.len().next_multiple_of(8) >= next.len().next_multiple_of(8) { return None; }
    out[0..4].copy_from_slice(&fingerprint(base).to_le_bytes());
    out[4..8].copy_from_slice(&(next.len() as u32).to_le_bytes());
    out[8..12].copy_from_slice(&fingerprint(next).to_le_bytes());
    Some(out)
}

fn take_word(bytes: &mut &[u8]) -> Option<u32> {
    let (word, rest) = bytes.split_first_chunk::<4>()?;
    *bytes = rest;
    Some(u32::from_le_bytes(*word))
}

pub(super) fn apply(base: &[u8], mut patch: &[u8]) -> Option<Vec<u8>> {
    let based_on = take_word(&mut patch)?;
    let length = take_word(&mut patch)? as usize;
    let target = take_word(&mut patch)?;
    if length > crate::lead::BYTES / 2 || fingerprint(base) != based_on { return None; }
    let mut out = Vec::with_capacity(length);
    while !patch.is_empty() {
        let code = take_word(&mut patch)?;
        let count = (code & !LITERAL) as usize;
        if count == 0 || count > length - out.len() { return None; }
        let bytes = if code & LITERAL != 0 {
            let (bytes, rest) = patch.split_at_checked(count)?;
            patch = rest;
            bytes
        } else {
            let offset = take_word(&mut patch)? as usize;
            base.get(offset..offset.checked_add(count)?)?
        };
        out.extend_from_slice(bytes);
    }
    (out.len() == length && fingerprint(&out) == target).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::{apply, encode};

    #[test]
    fn rebuilds_a_shifted_page_with_a_changed_row_and_unchanged_art() {
        let first = b"first archived row \x91".repeat(22);
        let middle = b"rows retained across the step \x81".repeat(23);
        let art = b"unchanged artwork names and URLs \x92".repeat(31);
        let before = [&first[..], &middle[..], b"cost 155", &art[..]].concat();
        let after = [b"new".as_slice(), &middle[..], b"cost 5999", &art[..]].concat();
        let patch = encode(&before, &after).expect("the repeated regions fit in copy spans");
        assert!(patch.len() < after.len() / 4);
        assert_eq!(apply(&before, &patch).as_deref(), Some(after.as_slice()));
        assert!(apply(&after, &patch).is_none(), "a different base must be refused");
        for end in [0, 11, patch.len() - 1] {
            assert!(apply(&before, &patch[..end]).is_none(), "truncated at {end}");
        }
        let mut damaged = patch;
        damaged[8] ^= 1;
        assert!(apply(&before, &damaged).is_none(), "the reconstructed body must be verified");
    }

    #[test]
    fn sends_a_full_value_if_it_would_be_smaller() {
        assert!(encode(&[7; 50], &[8; 50]).is_none());
    }
}
