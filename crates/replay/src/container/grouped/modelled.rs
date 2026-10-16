//! Row keys and masks of a modelled later section.
//!
//! Each way of writing a table's later entries keeps one [`Model`]: which of
//! the table's rows its last two entries stored and with which masks, each
//! row's last stored mask and the masks it stored most recently, and the
//! adaptive probabilities of its decisions. An entry's changed or residual
//! rows are coded as binary decisions with those probabilities by an
//! LZMA-style range coder ([`Encoder`], [`RangeDecoder`]) into the masks
//! column; integer arithmetic only, so writer and reader agree exactly.
//!
//! An entry's rows are, against the table's rows at the frame before
//! ([`KeySet`]): how many it inserts, each inserted key in the keys column
//! as its distance past the inserted key before it; then for each row the
//! last two entries stored, in key order, whether it changed, until every
//! changed held row is found; then the other changed held rows, each as how
//! many of the rows not stored by those two entries it skips since the one
//! before. A row's mask is its place in a list of the row's last mask and
//! then the most recently stored masks, as a stop decision at each of the
//! first [`RANKS`] places, then whether it is further along the list (and
//! where) or stored bit by bit, each bit decided in the context of the
//! same field in the row's last three masks. A table whose rows mostly
//! change the same few fields, or the same rows hour after hour, then costs
//! a fraction of a bit for most rows.
//!
//! Modelled layouts always use these decisions. A chosen layout keeps them
//! beside listed skips and mask places, and stores whichever whole section
//! compresses smaller; its leading masks-column byte names the coding. A
//! reader builds a model only for a section that chose model decisions.

use super::keyed::KeySet;
use crate::{
    codec::{Decoder, invalid_static as invalid, put_varint},
    rows::TableId,
};
use std::{collections::HashMap, io};

/// Probabilities are of a zero decision, in units of `1 / 2^PROBABILITY_BITS`.
const PROBABILITY_BITS: u32 = 12;
const ONE: u32 = 1 << PROBABILITY_BITS;
const EVEN: u16 = (ONE / 2) as u16;
/// A probability moves this fraction (a power of two) of the way to the
/// decision it saw.
const ADAPT_SHIFT: u32 = 4;
/// The range is renormalized below this.
const TOP: u32 = 1 << 24;
/// Places in the mask list each with their own stop decision.
pub(super) const RANKS: usize = 4;
/// Recently stored masks a model keeps.
pub(super) const KEPT: usize = 64;
/// Adaptive prefix decisions of a Golomb code, the last shared by longer
/// prefixes.
const PREFIX: usize = 32;
/// Contexts of a gap: none, or the gap before it was zero, below 4, below
/// 32, or longer.
const GAP_BUCKETS: usize = 5;
/// The longest Golomb prefix, for values up to `u64::MAX`.
const LONGEST: usize = 64;
/// Contexts of a stored mask bit: the same field in the row's last stored
/// mask and in its masks of the last two entries, and how many bits (0, 1,
/// 2 or more) of the mask are set before it.
const FIELD_CONTEXTS: usize = 24;
/// Contexts of a mask place stop decision: the place, whether the row has
/// a last mask, whether the last entry stored it, and at the first place
/// whether that entry stored the row's last mask.
const RANK_CONTEXTS: usize = RANKS * 2 * 2 * 3;

fn adapt(probability: &mut u16, bit: bool) {
    let p = u32::from(*probability);
    *probability = if bit {
        p - (p >> ADAPT_SHIFT)
    } else {
        p + ((ONE - p) >> ADAPT_SHIFT)
    } as u16;
}

/// Writes decisions as range-coded bytes (LZMA's carry-propagating
/// encoder): bytes are only appended, a pending byte and the run of 0xFF
/// bytes after it wait for a carry.
#[derive(Debug)]
struct Encoder {
    low: u64,
    range: u32,
    cache: u8,
    /// The pending byte and the 0xFF bytes after it.
    pending: u64,
    used: bool,
}

impl Default for Encoder {
    fn default() -> Self {
        Self {
            low: 0,
            range: u32::MAX,
            cache: 0,
            pending: 1,
            used: false,
        }
    }
}

impl Encoder {
    fn shift_low(&mut self, out: &mut Vec<u8>) {
        if (self.low as u32) < 0xFF00_0000 || self.low >> 32 != 0 {
            let carry = (self.low >> 32) as u8;
            let mut byte = self.cache;
            loop {
                out.push(byte.wrapping_add(carry));
                byte = 0xFF;
                self.pending -= 1;
                if self.pending == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.pending += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    fn normalize(&mut self, out: &mut Vec<u8>) {
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low(out);
        }
    }

    fn bit(&mut self, probability: &mut u16, bit: bool, out: &mut Vec<u8>) {
        self.used = true;
        let bound = (self.range >> PROBABILITY_BITS) * u32::from(*probability);
        if bit {
            self.low += u64::from(bound);
            self.range -= bound;
        } else {
            self.range = bound;
        }
        adapt(probability, bit);
        self.normalize(out);
    }

    /// A decision as likely either way, with no probability to adapt.
    fn direct(&mut self, bit: bool, out: &mut Vec<u8>) {
        self.used = true;
        self.range >>= 1;
        if bit {
            self.low += u64::from(self.range);
        }
        self.normalize(out);
    }

    /// The bytes still to write once no decision follows.
    fn waiting(&self) -> usize {
        if self.used {
            usize::try_from(self.pending)
                .unwrap_or(usize::MAX)
                .saturating_add(4)
        } else {
            0
        }
    }

    fn finish(&mut self, out: &mut Vec<u8>) {
        if std::mem::take(&mut self.used) {
            for _ in 0..5 {
                self.shift_low(out);
            }
        }
        *self = Self::default();
    }
}

/// Reads decisions an [`Encoder`] wrote, from the masks column; it starts
/// at its first decision, and reads exactly the bytes the encoder wrote.
#[derive(Debug, Default)]
struct RangeDecoder {
    range: u32,
    code: u32,
    started: bool,
}

impl RangeDecoder {
    fn start(&mut self, column: &mut Decoder<'_>) -> io::Result<()> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        // The encoder's first byte is the carry into nothing before it.
        if column.u8()? != 0 {
            return Err(invalid("modelled row stream has a nonzero first byte"));
        }
        self.code = 0;
        for _ in 0..4 {
            self.code = (self.code << 8) | u32::from(column.u8()?);
        }
        self.range = u32::MAX;
        // The code stays below the range: a decision then never reads past it.
        if self.code == u32::MAX {
            return Err(invalid("modelled row stream starts past its range"));
        }
        Ok(())
    }

    fn normalize(&mut self, column: &mut Decoder<'_>) -> io::Result<()> {
        while self.range < TOP {
            self.range <<= 8;
            self.code = (self.code << 8) | u32::from(column.u8()?);
        }
        Ok(())
    }

    fn bit(&mut self, probability: &mut u16, column: &mut Decoder<'_>) -> io::Result<bool> {
        self.start(column)?;
        let bound = (self.range >> PROBABILITY_BITS) * u32::from(*probability);
        let bit = self.code >= bound;
        if bit {
            self.code -= bound;
            self.range -= bound;
        } else {
            self.range = bound;
        }
        adapt(probability, bit);
        self.normalize(column)?;
        Ok(bit)
    }

    fn direct(&mut self, column: &mut Decoder<'_>) -> io::Result<bool> {
        self.start(column)?;
        self.range >>= 1;
        let bit = self.code >= self.range;
        if bit {
            self.code -= self.range;
        }
        // An odd range leaves one value no encoder writes.
        if self.code >= self.range {
            return Err(invalid("modelled row stream is past its range"));
        }
        self.normalize(column)?;
        Ok(bit)
    }
}

/// An adaptive Exp-Golomb code: as many one decisions as the value plus one
/// has bits after its top one, a zero, its next bit with a probability by
/// that length, and the rest even.
#[derive(Debug)]
struct Golomb {
    prefix: Vec<u16>,
    top: Vec<u16>,
}

impl Golomb {
    fn new(buckets: usize) -> Self {
        Self {
            prefix: vec![EVEN; buckets * PREFIX],
            top: vec![EVEN; buckets * (LONGEST + 1)],
        }
    }

    fn put(&mut self, encoder: &mut Encoder, value: u64, bucket: usize, out: &mut Vec<u8>) {
        let value = u128::from(value) + 1;
        let length = (127 - value.leading_zeros()) as usize;
        let prefix = &mut self.prefix[bucket * PREFIX..][..PREFIX];
        for at in 0..length {
            encoder.bit(&mut prefix[at.min(PREFIX - 1)], true, out);
        }
        encoder.bit(&mut prefix[length.min(PREFIX - 1)], false, out);
        for at in (0..length).rev() {
            let bit = (value >> at) & 1 != 0;
            if at == length - 1 {
                encoder.bit(&mut self.top[bucket * (LONGEST + 1) + length], bit, out);
            } else {
                encoder.direct(bit, out);
            }
        }
    }

    /// A value below `limit`.
    fn take(
        &mut self,
        decoder: &mut RangeDecoder,
        bucket: usize,
        limit: u64,
        column: &mut Decoder<'_>,
    ) -> io::Result<u64> {
        let prefix = &mut self.prefix[bucket * PREFIX..][..PREFIX];
        let mut length = 0;
        while decoder.bit(&mut prefix[length.min(PREFIX - 1)], column)? {
            length += 1;
            if length > LONGEST {
                return Err(invalid("modelled number is too long"));
            }
        }
        let mut value = 1_u128;
        for at in (0..length).rev() {
            let bit = if at == length - 1 {
                decoder.bit(&mut self.top[bucket * (LONGEST + 1) + length], column)?
            } else {
                decoder.direct(column)?
            };
            value = (value << 1) | u128::from(bit);
        }
        let value = value - 1;
        if value >= u128::from(limit) {
            return Err(invalid("modelled number is past its bound"));
        }
        Ok(value as u64)
    }
}

/// A row one of the last two entries stored, with its masks there (zero
/// where it was not stored).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hot {
    key: u128,
    last: u64,
    before: u64,
}

/// One way of writing a table's later entries: what its entries stored and
/// the probabilities of their decisions, followed by writer and reader
/// alike.
#[derive(Debug)]
pub(super) struct Model {
    fields: usize,
    /// In key order.
    hot: Vec<Hot>,
    /// Each stored row's last mask.
    last: HashMap<u128, u64>,
    /// The latest first.
    recent: Vec<u64>,
    /// By whether the row was stored by the last entry, by the one before,
    /// and whether the row before it in this entry changed.
    changed: [u16; 8],
    gaps: Golomb,
    inserted: Golomb,
    further: Golomb,
    rank: [u16; RANK_CONTEXTS],
    /// Whether a mask past the stop places is stored bit by bit, by whether
    /// the row has a last mask.
    literal: [u16; 2],
    field: Vec<u16>,
    encoder: Encoder,
    decoder: RangeDecoder,
    spare_hot: Vec<Hot>,
    hot_places: Vec<(Hot, usize)>,
    inserted_keys: Vec<u128>,
    places: Vec<usize>,
    stored_rows: Vec<(u128, u64)>,
    run: Vec<(u128, u64, bool)>,
    held_masks: Vec<u64>,
    held_changes: Option<u64>,
}

/// Advance an ordered merge cursor, using exponential skips for cold spans.
fn held_at_or_after(held: &[u128], mut at: usize, key: u128) -> usize {
    if held.get(at).is_none_or(|held| *held >= key) {
        return at;
    }
    let mut span = 1;
    while span < held.len() - at && held[at + span] < key {
        at += span;
        span = span.saturating_mul(2);
    }
    let end = at + span.min(held.len() - at);
    at + held[at..end].partition_point(|held| *held < key)
}

fn gap_bucket(previous: Option<u64>) -> usize {
    match previous {
        None => 0,
        Some(0) => 1,
        Some(1..4) => 2,
        Some(4..32) => 3,
        Some(_) => 4,
    }
}

fn changed_context(hot: &Hot, before: bool) -> usize {
    usize::from(hot.last != 0) | usize::from(hot.before != 0) << 1 | usize::from(before) << 2
}

/// A mask's row contexts: its masks of the last two entries and its last.
#[derive(Clone, Copy)]
struct Seen {
    last: u64,
    before: u64,
    stored: u64,
}

fn rank_context(place: usize, seen: Seen) -> usize {
    let third = if place == 0 {
        usize::from(seen.last == seen.stored)
    } else {
        2
    };
    ((place * 2 + usize::from(seen.stored != 0)) * 2 + usize::from(seen.last != 0)) * 3 + third
}

fn field_context(field: usize, seen: Seen, set: usize) -> usize {
    let bit = |mask: u64| ((mask >> field) & 1) as usize;
    field * FIELD_CONTEXTS
        + ((bit(seen.last) * 2 + bit(seen.before)) * 2 + bit(seen.stored)) * 3
        + set
}

impl Model {
    pub(super) fn new(fields: usize) -> Self {
        Self {
            fields,
            hot: Vec::new(),
            last: HashMap::new(),
            recent: Vec::new(),
            changed: [EVEN; 8],
            gaps: Golomb::new(GAP_BUCKETS),
            inserted: Golomb::new(1),
            further: Golomb::new(1),
            rank: [EVEN; RANK_CONTEXTS],
            literal: [EVEN; 2],
            field: vec![EVEN; fields * FIELD_CONTEXTS],
            encoder: Encoder::default(),
            decoder: RangeDecoder::default(),
            spare_hot: Vec::new(),
            hot_places: Vec::new(),
            inserted_keys: Vec::new(),
            places: Vec::new(),
            stored_rows: Vec::new(),
            run: Vec::new(),
            held_masks: Vec::new(),
            held_changes: None,
        }
    }

    /// The hot rows `held` holds, with their places among its rows.
    fn hot_in(&self, held: &[u128]) -> Vec<(Hot, usize)> {
        self.hot
            .iter()
            .filter_map(|hot| held.binary_search(&hot.key).ok().map(|at| (*hot, at)))
            .collect()
    }

    /// Return a consumed reader run's allocation for the next entry.
    pub(super) fn recycle_run(&mut self, mut rows: Vec<(u128, u64, bool)>) {
        rows.clear();
        self.run = rows;
    }

    fn index_held(&mut self, held: &KeySet) {
        if self.held_changes != Some(held.changes()) {
            self.held_masks.clear();
            self.held_masks.extend(
                held.keys()
                    .iter()
                    .map(|key| self.last.get(key).copied().unwrap_or(0)),
            );
            self.held_changes = Some(held.changes());
        }
    }

    /// Merge both ordered sets. Galloping over a long cold span avoids
    /// scanning a large static inventory for a small set of hot rows.
    fn hot_into(&self, held: &[u128], out: &mut Vec<(Hot, usize)>) {
        out.clear();
        let mut at = 0;
        for &hot in &self.hot {
            at = held_at_or_after(held, at, hot.key);
            if held.get(at) == Some(&hot.key) {
                out.push((hot, at));
                at += 1;
            }
        }
    }

    fn seen(&self, key: u128) -> Seen {
        let (last, before) = self
            .hot
            .binary_search_by_key(&key, |hot| hot.key)
            .map_or((0, 0), |at| (self.hot[at].last, self.hot[at].before));
        Seen {
            last,
            before,
            stored: self.last.get(&key).copied().unwrap_or(0),
        }
    }

    /// The mask list of a row whose last mask is `stored` (zero for none):
    /// that mask, then the recent masks but it. Its length, and the place of
    /// `mask` in it.
    fn place(&self, stored: u64, mask: u64) -> (usize, Option<usize>) {
        let mut length = usize::from(stored != 0);
        let mut place = (stored != 0 && mask == stored).then_some(0);
        // A stored mask is never zero, so no row without one skips any.
        for &kept in self.recent.iter().filter(|kept| **kept != stored) {
            if place.is_none() && kept == mask {
                place = Some(length);
            }
            length += 1;
        }
        (length, place)
    }

    /// The mask at `place` in that list, which is shorter than it.
    fn listed(&self, stored: u64, place: usize) -> u64 {
        if stored != 0 && place == 0 {
            return stored;
        }
        let skip = place - usize::from(stored != 0);
        self.recent
            .iter()
            .copied()
            .filter(|kept| *kept != stored)
            .nth(skip)
            .expect("a place within the mask list")
    }

    /// Make `mask` the latest stored mask and `key`'s last.
    fn note(&mut self, key: u128, mask: u64) {
        if let Some(at) = self.recent.iter().position(|kept| *kept == mask) {
            self.recent.remove(at);
        }
        self.recent.insert(0, mask);
        self.recent.truncate(KEPT);
        self.last.insert(key, mask);
    }

    /// Follow an entry that stored `rows`, in key order.
    fn age(&mut self, rows: &[(u128, u64)]) {
        let mut next = std::mem::take(&mut self.spare_hot);
        next.clear();
        let (mut row, mut hot) = (0, 0);
        loop {
            match (rows.get(row), self.hot.get(hot)) {
                (None, None) => break,
                (Some(&(key, mask)), old) if old.is_none_or(|old| key <= old.key) => {
                    let before = match old {
                        Some(old) if old.key == key => {
                            hot += 1;
                            old.last
                        }
                        _ => 0,
                    };
                    next.push(Hot {
                        key,
                        last: mask,
                        before,
                    });
                    row += 1;
                }
                (_, Some(old)) => {
                    if old.last != 0 {
                        next.push(Hot {
                            key: old.key,
                            last: 0,
                            before: old.last,
                        });
                    }
                    hot += 1;
                }
                (Some(_), None) => unreachable!("a row with no hot row after it is stored first"),
            }
        }
        std::mem::swap(&mut self.hot, &mut next);
        next.clear();
        self.spare_hot = next;
    }

    /// Bytes the writer still owes the masks column.
    pub(super) fn waiting(&self) -> usize {
        self.encoder.waiting()
    }

    /// End the stream: the masks column gets the bytes the encoder still
    /// owes.
    pub(super) fn finish(&mut self, coded: &mut Vec<u8>) {
        self.encoder.finish(coded);
    }

    /// Store one entry's rows, in increasing key order, against the rows
    /// the table held at the frame before: inserted keys in `keys`, every
    /// decision in `coded`.
    pub(super) fn put(
        &mut self,
        held: &KeySet,
        rows: &[(u128, u64)],
        keys: &mut Vec<u8>,
        coded: &mut Vec<u8>,
    ) -> io::Result<()> {
        if rows.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(invalid("modelled rows did not advance"));
        }
        if rows
            .iter()
            .any(|(_, mask)| *mask == 0 || (self.fields < 64 && *mask >> self.fields != 0))
        {
            return Err(invalid("invalid modelled row mask"));
        }
        if rows.is_empty() {
            self.age(rows);
            return Ok(());
        }
        let held = held.keys();
        let mut changed: Vec<(u128, usize)> = Vec::with_capacity(rows.len());
        let mut inserted = Vec::new();
        for &(key, _) in rows {
            match held.binary_search(&key) {
                Ok(at) => changed.push((key, at)),
                Err(_) => inserted.push(key),
            }
        }
        self.inserted
            .put(&mut self.encoder, inserted.len() as u64, 0, coded);
        let mut previous: Option<u128> = None;
        for &key in &inserted {
            put_varint(keys, previous.map_or(key, |previous| key - previous - 1));
            previous = Some(key);
        }
        let hot = self.hot_in(held);
        let (mut found, mut next, mut before) = (0, 0, false);
        for (row, _) in &hot {
            if found == changed.len() {
                break;
            }
            while changed.get(next).is_some_and(|(key, _)| *key < row.key) {
                next += 1;
            }
            let bit = changed.get(next).is_some_and(|(key, _)| *key == row.key);
            let context = changed_context(row, before);
            self.encoder.bit(&mut self.changed[context], bit, coded);
            before = bit;
            found += usize::from(bit);
        }
        // The other changed rows by how many unstored rows each skips.
        let (mut hot_at, mut cold_next, mut gap_before) = (0, 0, None);
        for &(_, at) in &changed {
            while hot.get(hot_at).is_some_and(|(_, place)| *place < at) {
                hot_at += 1;
            }
            if hot.get(hot_at).is_some_and(|(_, place)| *place == at) {
                continue;
            }
            let cold = at - hot_at;
            let gap = (cold - cold_next) as u64;
            self.gaps
                .put(&mut self.encoder, gap, gap_bucket(gap_before), coded);
            gap_before = Some(gap);
            cold_next = cold + 1;
        }
        for &(key, mask) in rows {
            self.put_mask(key, mask, coded);
        }
        self.age(rows);
        Ok(())
    }

    fn put_mask(&mut self, key: u128, mask: u64, coded: &mut Vec<u8>) {
        let seen = self.seen(key);
        let (length, place) = self.place(seen.stored, mask);
        for at in 0..length.min(RANKS) {
            let stop = place == Some(at);
            self.encoder
                .bit(&mut self.rank[rank_context(at, seen)], stop, coded);
            if stop {
                self.note(key, mask);
                return;
            }
        }
        if length > RANKS {
            let literal = place.is_none();
            self.encoder.bit(
                &mut self.literal[usize::from(seen.stored != 0)],
                literal,
                coded,
            );
            if let Some(place) = place {
                self.further
                    .put(&mut self.encoder, (place - RANKS) as u64, 0, coded);
                self.note(key, mask);
                return;
            }
        }
        let mut set = 0;
        for field in 0..self.fields {
            let bit = (mask >> field) & 1 != 0;
            // A mask has a field: the last one when no other is set.
            if !(field == self.fields - 1 && set == 0) {
                self.encoder
                    .bit(&mut self.field[field_context(field, seen, set)], bit, coded);
            }
            set = (set + usize::from(bit)).min(2);
        }
        self.note(key, mask);
    }

    /// Read one entry's `count` rows a [`Self::put`] stored, in key order,
    /// each with whether the table did not hold it (an inserted row).
    pub(super) fn take(
        &mut self,
        table: TableId,
        held: &KeySet,
        count: usize,
        keys: &mut Decoder<'_>,
        coded: &mut Decoder<'_>,
    ) -> io::Result<Vec<(u128, u64, bool)>> {
        if count == 0 {
            self.age(&[]);
            return Ok(Vec::new());
        }
        self.index_held(held);
        let held = held.keys();
        let inserted_count =
            self.inserted
                .take(&mut self.decoder, 0, count as u64 + 1, coded)? as usize;
        // Every inserted key has a key byte.
        if inserted_count > keys.remaining() {
            return Err(invalid("modelled inserted rows exceed their keys"));
        }
        let mut inserted = std::mem::take(&mut self.inserted_keys);
        inserted.clear();
        let mut previous: Option<u128> = None;
        for _ in 0..inserted_count {
            let distance = keys.varint()?;
            let key = match previous {
                None => Some(distance),
                Some(previous) => previous
                    .checked_add(1)
                    .and_then(|key| key.checked_add(distance)),
            }
            .ok_or_else(|| invalid("row key addition overflow"))?;
            if held.binary_search(&key).is_ok() {
                return Err(invalid("noncanonical modelled row key"));
            }
            table.check_key(key)?;
            inserted.push(key);
            previous = Some(key);
        }
        let need = count - inserted_count;
        let mut hot = std::mem::take(&mut self.hot_places);
        self.hot_into(held, &mut hot);
        let mut places = std::mem::take(&mut self.places);
        places.clear();
        let (mut found, mut before) = (0, false);
        for (row, at) in &hot {
            if found == need {
                break;
            }
            let context = changed_context(row, before);
            let bit = self.decoder.bit(&mut self.changed[context], coded)?;
            if bit {
                places.push(*at);
                found += 1;
            }
            before = bit;
        }
        let cold_count = held.len() - hot.len();
        if need - found > cold_count {
            return Err(invalid("modelled rows exceed the rows before"));
        }
        let (mut hot_at, mut cold_next, mut gap_before) = (0, 0, None);
        for _ in found..need {
            let gap = self.gaps.take(
                &mut self.decoder,
                gap_bucket(gap_before),
                (cold_count - cold_next) as u64,
                coded,
            )?;
            let cold = cold_next + gap as usize;
            // The cold row's place: past every hot row at or before it.
            while hot
                .get(hot_at)
                .is_some_and(|(_, place)| *place <= cold + hot_at)
            {
                hot_at += 1;
            }
            places.push(cold + hot_at);
            gap_before = Some(gap);
            cold_next = cold + 1;
        }
        places.sort_unstable();
        let mut out = std::mem::take(&mut self.run);
        out.clear();
        let mut stored = std::mem::take(&mut self.stored_rows);
        stored.clear();
        let (mut kept_at, mut new_at, mut hot_at) = (0, 0, 0);
        while kept_at < places.len() || new_at < inserted.len() {
            let kept = places.get(kept_at).map(|&at| (held[at], at));
            let new = inserted.get(new_at).copied();
            let (key, position) = match (kept, new) {
                (Some((key, at)), new) if new.is_none_or(|new| key < new) => {
                    kept_at += 1;
                    (key, Some(at))
                }
                (_, Some(key)) => {
                    new_at += 1;
                    (key, None)
                }
                _ => unreachable!("one ordered row remains"),
            };
            while hot.get(hot_at).is_some_and(|(row, _)| row.key < key) {
                hot_at += 1;
            }
            let seen = match position {
                Some(at) => {
                    let row = hot.get(hot_at).filter(|(row, _)| row.key == key);
                    Seen {
                        last: row.map_or(0, |(row, _)| row.last),
                        before: row.map_or(0, |(row, _)| row.before),
                        stored: self.held_masks[at],
                    }
                }
                // A reinserted key retains its historical last mask, and
                // can still belong to either of the preceding two entries.
                None => self.seen(key),
            };
            let mask = self.take_mask(key, seen, coded)?;
            if let Some(at) = position {
                self.held_masks[at] = mask;
            }
            stored.push((key, mask));
            out.push((key, mask, position.is_none()));
        }
        self.age(&stored);
        self.hot_places = hot;
        self.places = places;
        self.inserted_keys = inserted;
        self.stored_rows = stored;
        Ok(out)
    }

    fn take_mask(&mut self, key: u128, seen: Seen, coded: &mut Decoder<'_>) -> io::Result<u64> {
        // No mask is zero: only the list's length.
        let (length, _) = self.place(seen.stored, 0);
        for at in 0..length.min(RANKS) {
            if self
                .decoder
                .bit(&mut self.rank[rank_context(at, seen)], coded)?
            {
                let mask = self.listed(seen.stored, at);
                self.note(key, mask);
                return Ok(mask);
            }
        }
        if length > RANKS
            && !self
                .decoder
                .bit(&mut self.literal[usize::from(seen.stored != 0)], coded)?
        {
            let place = RANKS
                + self
                    .further
                    .take(&mut self.decoder, 0, (length - RANKS) as u64, coded)?
                    as usize;
            let mask = self.listed(seen.stored, place);
            self.note(key, mask);
            return Ok(mask);
        }
        let (mut mask, mut set) = (0_u64, 0);
        for field in 0..self.fields {
            let bit = if field == self.fields - 1 && set == 0 {
                true
            } else {
                self.decoder
                    .bit(&mut self.field[field_context(field, seen, set)], coded)?
            };
            mask |= u64::from(bit) << field;
            set = (set + usize::from(bit)).min(2);
        }
        if self.place(seen.stored, mask).1.is_some() {
            return Err(invalid("noncanonical modelled mask"));
        }
        self.note(key, mask);
        Ok(mask)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn modelled_reader_reuses_runs_and_indexes_each_held_mask() {
        let mut held = KeySet::of(vec![4, 8]);
        let entries = [
            vec![(4, 1), (8, 2)],
            vec![(4, 4), (8, 16)],
            vec![(4, 1), (8, 2)],
        ];
        let mut writer = Model::new(FIELDS);
        let (mut keys, mut coded) = (Vec::new(), Vec::new());
        for rows in &entries {
            writer.put(&held, rows, &mut keys, &mut coded).unwrap();
        }
        writer.finish(&mut coded);
        let (mut keys, mut coded) = (Decoder::new(&keys), Decoder::new(&coded));
        let mut reader = Model::new(FIELDS);
        for expected in &entries {
            let rows = reader
                .take(TableId::Divisions, &held, 2, &mut keys, &mut coded)
                .unwrap();
            assert_eq!(
                rows,
                expected
                    .iter()
                    .map(|&(key, mask)| (key, mask, false))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                reader.held_masks,
                expected.iter().map(|&(_, mask)| mask).collect::<Vec<_>>()
            );
            let capacity = rows.capacity();
            reader.recycle_run(rows);
            assert!(reader.run.is_empty() && reader.run.capacity() >= capacity);
            assert!(reader.places.capacity() >= 2 && reader.stored_rows.capacity() >= 2);
        }
        keys.finish().unwrap();
        coded.finish().unwrap();
        // Replacing membership with an equal-sized table automatically gives
        // every position its own key's history.
        held.apply(&mut super::super::keyed::KeyChanges {
            inserted: vec![12],
            deleted: vec![4],
        });
        reader.index_held(&held);
        assert_eq!(reader.held_masks, [2, 0]);
        assert_eq!(reader.held_changes, Some(held.changes()));
        // A clear and reinsertion preserve historical masks by key without
        // any manual notification to the model.
        held.clear();
        held.apply(&mut super::super::keyed::KeyChanges {
            inserted: vec![4, 8],
            deleted: Vec::new(),
        });
        reader.index_held(&held);
        assert_eq!(reader.held_masks, [1, 2]);
    }

    #[test]
    fn ordered_hot_merge_skips_large_cold_spans_exactly() {
        let held: Vec<u128> = (0..5_000).map(|n| n * 3).collect();
        let mut model = Model::new(FIELDS);
        model.hot = [0, 1, 12, 1_501, 14_997, 15_001]
            .map(|key| Hot {
                key,
                last: 1,
                before: 2,
            })
            .to_vec();
        let mut merged = Vec::new();
        model.hot_into(&held, &mut merged);
        let expected: Vec<_> = model
            .hot
            .iter()
            .filter_map(|hot| held.binary_search(&hot.key).ok().map(|at| (*hot, at)))
            .collect();
        assert_eq!(merged, expected);
        for key in 0..15_005 {
            assert_eq!(
                held_at_or_after(&held, 0, key),
                held.partition_point(|held| *held < key)
            );
        }
    }

    use super::*;

    const FIELDS: usize = 5;

    /// The reference's linear congruential sequence.
    struct Sequence(u64);

    impl Sequence {
        fn next(&mut self) -> u64 {
            self.0 = (self.0 * 1_103_515_245 + 12_345) % (1 << 31);
            self.0
        }
    }

    #[test]
    fn range_coded_decisions_match_the_reference_and_read_every_byte() {
        let decisions = || {
            let mut sequence = Sequence(12_345);
            (0..200)
                .map(|at| {
                    let x = sequence.next();
                    let slot = (x % 3) as usize;
                    let bit = (x >> 7).is_multiple_of([4, 2, 7][slot]);
                    // Every 17th decision is even.
                    ((at % 17 != 0).then_some(slot), bit)
                })
                .collect::<Vec<_>>()
        };
        let mut encoder = Encoder::default();
        let mut probabilities = [EVEN; 3];
        let mut out = Vec::new();
        for (slot, bit) in decisions() {
            match slot {
                Some(slot) => encoder.bit(&mut probabilities[slot], bit, &mut out),
                None => encoder.direct(bit, &mut out),
            }
        }
        let waiting = encoder.waiting();
        let before = out.len();
        encoder.finish(&mut out);
        assert_eq!(out.len() - before, waiting);
        // The Python reference's bytes for the same decisions.
        assert_eq!(
            out,
            [
                0, 162, 169, 102, 32, 185, 247, 133, 35, 164, 119, 159, 208, 53, 181, 187, 66, 130,
                91, 103, 116, 86, 114, 126, 0
            ]
        );
        let mut column = Decoder::new(&out);
        let mut decoder = RangeDecoder::default();
        let mut probabilities = [EVEN; 3];
        for (slot, bit) in decisions() {
            let read = match slot {
                Some(slot) => decoder.bit(&mut probabilities[slot], &mut column).unwrap(),
                None => decoder.direct(&mut column).unwrap(),
            };
            assert_eq!(read, bit);
        }
        column.finish().unwrap();
        // A stream starts with the carry byte, which is zero.
        let mut bad = out.clone();
        bad[0] = 1;
        let mut probability = EVEN;
        assert!(
            RangeDecoder::default()
                .bit(&mut probability, &mut Decoder::new(&bad))
                .is_err()
        );
        // No decision, no bytes.
        let mut empty = Vec::new();
        Encoder::default().finish(&mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn golomb_values_round_trip_below_their_bound() {
        let values = [0, 1, 2, 3, 7, 8, 1_000, 1 << 40, u64::MAX - 1];
        let mut encoder = Encoder::default();
        let mut code = Golomb::new(2);
        let mut out = Vec::new();
        for (at, &value) in values.iter().enumerate() {
            code.put(&mut encoder, value, at % 2, &mut out);
        }
        encoder.finish(&mut out);
        let mut column = Decoder::new(&out);
        let mut decoder = RangeDecoder::default();
        let mut code = Golomb::new(2);
        for (at, &value) in values.iter().enumerate() {
            let read = code
                .take(&mut decoder, at % 2, value + 1, &mut column)
                .unwrap();
            assert_eq!(read, value);
        }
        column.finish().unwrap();
        // A value at its bound is refused.
        let mut encoder = Encoder::default();
        let mut out = Vec::new();
        Golomb::new(1).put(&mut encoder, 5, 0, &mut out);
        encoder.finish(&mut out);
        let error = Golomb::new(1)
            .take(&mut RangeDecoder::default(), 0, 5, &mut Decoder::new(&out))
            .unwrap_err();
        assert!(error.to_string().contains("past its bound"), "{error}");
    }

    type Entry = (Vec<(u128, u64)>, Vec<u128>);

    /// Each entry's rows held at the frame before, from `held` and the rows
    /// the entries before inserted and deleted.
    fn followed(held: &[u128], entries: &[Entry]) -> Vec<KeySet> {
        let mut keys = held.to_vec();
        entries
            .iter()
            .map(|(rows, deleted)| {
                let set = KeySet::of(keys.clone());
                keys.extend(rows.iter().map(|(key, _)| *key));
                keys.sort_unstable();
                keys.dedup();
                keys.retain(|key| !deleted.contains(key));
                set
            })
            .collect()
    }

    fn written(helds: &[KeySet], entries: &[Entry], fields: usize) -> (Vec<u8>, Vec<u8>) {
        let mut model = Model::new(fields);
        let (mut keys, mut coded) = (Vec::new(), Vec::new());
        for (held, (rows, _)) in helds.iter().zip(entries) {
            model.put(held, rows, &mut keys, &mut coded).unwrap();
        }
        model.finish(&mut coded);
        (keys, coded)
    }

    fn read(
        helds: &[KeySet],
        counts: &[usize],
        fields: usize,
        keys: &[u8],
        coded: &[u8],
    ) -> io::Result<Vec<Vec<(u128, u64, bool)>>> {
        let mut model = Model::new(fields);
        let (mut keys, mut coded) = (Decoder::new(keys), Decoder::new(coded));
        let mut live = KeySet::default();
        let rows = helds
            .iter()
            .zip(counts)
            .map(|(held, &count)| {
                if live.keys() != held.keys() {
                    live.clear();
                    live.apply(&mut super::super::keyed::KeyChanges {
                        inserted: held.keys().to_vec(),
                        deleted: Vec::new(),
                    });
                }
                model.take(TableId::Divisions, &live, count, &mut keys, &mut coded)
            })
            .collect::<io::Result<Vec<_>>>()?;
        keys.finish()?;
        coded.finish()?;
        Ok(rows)
    }

    /// The rows an entry stores as a reader returns them.
    fn as_read(entry: &Entry, held: &KeySet) -> Vec<(u128, u64, bool)> {
        entry
            .0
            .iter()
            .map(|&(key, mask)| (key, mask, held.keys().binary_search(&key).is_err()))
            .collect()
    }

    #[test]
    fn modelled_rows_match_the_reference_through_inserts_deletes_and_empty_entries() {
        let held = [10, 20, 30, 40, 50, 60, 70, 80];
        let entries: Vec<Entry> = vec![
            (vec![(20, 0b00011), (50, 0b00001)], vec![]),
            (vec![(20, 0b00011), (35, 0b10000), (60, 0b00001)], vec![]),
            (vec![], vec![70]),
            (
                vec![(10, 0b00100), (20, 0b00011), (35, 0b10000), (80, 0b11111)],
                vec![],
            ),
            (vec![(5, 1), (90, 2)], vec![]),
            (
                vec![(5, 1), (10, 0b00100), (20, 0b00011), (30, 0b01000), (90, 2)],
                vec![],
            ),
        ];
        let helds = followed(&held, &entries);
        let (keys, coded) = written(&helds, &entries, FIELDS);
        // The Python reference's bytes: inserted keys 35, then 5 and 90.
        assert_eq!(keys, [35, 5, 84]);
        assert_eq!(
            coded,
            [
                0, 75, 131, 223, 8, 198, 75, 151, 72, 64, 215, 182, 84, 57, 170, 70, 0
            ]
        );
        let counts: Vec<usize> = entries.iter().map(|(rows, _)| rows.len()).collect();
        let rows = read(&helds, &counts, FIELDS, &keys, &coded).unwrap();
        for ((rows, entry), held) in rows.iter().zip(&entries).zip(&helds) {
            assert_eq!(*rows, as_read(entry, held));
        }
        assert!(rows[1][1].2 && rows[4].iter().all(|row| row.2));
        // One row more than stored reads past the stream.
        let mut more = counts.clone();
        more[5] += 1;
        assert!(read(&helds, &more, FIELDS, &keys, &coded).is_err());
    }

    #[test]
    fn modelled_rows_round_trip_a_long_changing_table() {
        // 3,000 rows, a changing quarter of them each hour, a few inserted
        // and deleted, masks mostly a few common ones.
        let mut sequence = Sequence(7);
        let mut keys: Vec<u128> = (0..3_000_u128).map(|n| (n * 977) << 32 | 0x33).collect();
        let common = [
            1_u64 << 3,
            1 << 3 | 1 << 29,
            1 << 19,
            1 << 22 | 1 << 19,
            1 << 29,
        ];
        let mut entries: Vec<Entry> = Vec::new();
        let mut helds = Vec::new();
        for hour in 0..200_u128 {
            helds.push(KeySet::of(keys.clone()));
            let mut rows: Vec<(u128, u64)> = Vec::new();
            for &key in &keys {
                if !sequence.next().is_multiple_of(4) {
                    continue;
                }
                let x = sequence.next();
                let mask = if x.is_multiple_of(10) {
                    (x >> 5) & ((1 << 30) - 1) | 1
                } else {
                    common[(x % 5) as usize]
                };
                rows.push((key, mask));
            }
            if hour % 7 == 3 {
                rows.push(((hour * 977 + 500) << 32 | 0x33, 1 << 4));
                rows.sort_unstable();
            }
            let deleted: Vec<u128> = if hour % 11 == 5 {
                keys.iter().step_by(301).copied().collect()
            } else {
                Vec::new()
            };
            for (key, _) in &rows {
                if let Err(at) = keys.binary_search(key) {
                    keys.insert(at, *key);
                }
            }
            keys.retain(|key| !deleted.contains(key));
            entries.push((rows, deleted));
        }
        let (stored, coded) = written(&helds, &entries, 30);
        let counts: Vec<usize> = entries.iter().map(|(rows, _)| rows.len()).collect();
        let rows = read(&helds, &counts, 30, &stored, &coded).unwrap();
        for ((rows, entry), held) in rows.iter().zip(&entries).zip(&helds) {
            assert_eq!(*rows, as_read(entry, held));
        }
        // The Python reference's sizes: under a third of a byte a row.
        assert_eq!(counts.iter().sum::<usize>(), 194_947);
        assert_eq!((stored.len(), coded.len()), (213, 60_631));
    }

    #[test]
    fn modelled_readers_refuse_held_inserted_keys_and_rows_past_the_stream() {
        let held = KeySet::of(vec![4, 8]);
        // An inserted key the table holds is noncanonical.
        let mut model = Model::new(FIELDS);
        let (mut keys, mut coded) = (Vec::new(), Vec::new());
        model
            .put(&KeySet::of(vec![4]), &[(8, 1)], &mut keys, &mut coded)
            .unwrap();
        model.finish(&mut coded);
        let error = Model::new(FIELDS)
            .take(
                TableId::Divisions,
                &held,
                1,
                &mut Decoder::new(&keys),
                &mut Decoder::new(&coded),
            )
            .unwrap_err();
        assert!(error.to_string().contains("noncanonical"), "{error}");
        // Rows given out of order or with no field or one past the table's
        // are refused.
        let mut model = Model::new(FIELDS);
        let refused: [Vec<(u128, u64)>; 3] =
            [vec![(8, 1), (4, 1)], vec![(4, 0)], vec![(4, 1 << FIELDS)]];
        for rows in &refused {
            assert!(
                model
                    .put(&held, rows, &mut Vec::new(), &mut Vec::new())
                    .is_err()
            );
        }
        // More rows than the table held and the entry inserted.
        let mut model = Model::new(FIELDS);
        let (mut keys, mut coded) = (Vec::new(), Vec::new());
        model.put(&held, &[(4, 1)], &mut keys, &mut coded).unwrap();
        model.finish(&mut coded);
        assert!(
            Model::new(FIELDS)
                .take(
                    TableId::Divisions,
                    &held,
                    3,
                    &mut Decoder::new(&keys),
                    &mut Decoder::new(&coded),
                )
                .is_err()
        );
    }
}

#[cfg(test)]
#[path = "layout_census.rs"]
mod layout_census;
