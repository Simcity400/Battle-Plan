//! Row keys and field masks of a keyed block's later entries.
//!
//! Writer and reader follow one table's plain updates through the block and
//! keep the keys of its rows at the frame before ([`KeySet`]): the keyframe's
//! rows when the block stores its keyframe as a snapshot (none for a keyframe
//! stored as a change against an earlier one), then each later update's
//! changes. A later delta or residual entry names each of its rows, and each
//! deleted key, by how many of those rows it skips since the row before
//! ([`KeyWriter`]); a key they do not hold (an inserted row) is stored as its
//! distance past the key before it. A changing subset of rows with wide keys
//! (a division's handle, a country and an index) then costs a byte or two a
//! row instead of the varint of the key difference. The keys a delta or
//! residual entry stores as distances are exactly the rows it inserts, so a
//! reader follows those entries from what it decoded ([`KeyChanges`]) rather
//! than walking their plain updates again.
//!
//! Each way of writing a table's later entries keeps the masks it stored
//! most recently ([`MaskList`]); a mask is stored as its place among them, or
//! as itself when it is not among them, since the rows of a table mostly
//! change the same few fields. A modelled section stores a later delta's or
//! residual entry's rows and masks as decisions of its [`Model`] instead.
//! A chosen block keeps both pairs while writing and stores whichever whole
//! section compresses smaller, named by its leading masks-column byte;
//! deleted keys and entries without coded rows keep the same bytes in both.

use super::{Columns, DELTA, KEYS, MASKS, SNAPSHOT, TRUNCATED, UNCHANGED, modelled::Model};
use crate::{
    codec::{Decoder, invalid_static as invalid, put_varint},
    rows::{TableId, Token},
};
use std::io;

/// Masks a [`MaskList`] keeps.
pub(super) const MASKS_KEPT: usize = 64;

/// The keys of one table's rows at the frame before, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeySet {
    keys: Vec<u128>,
    /// Membership changes within this table's live block.
    changes: u64,
}

impl KeySet {
    pub(super) fn clear(&mut self) {
        if !self.keys.is_empty() {
            self.keys.clear();
            self.changes += 1;
        }
    }

    pub(super) fn changes(&self) -> u64 {
        self.changes
    }

    pub(super) fn keys(&self) -> &[u128] {
        &self.keys
    }

    #[cfg(test)]
    pub(super) fn of(keys: Vec<u128>) -> Self {
        Self { keys, changes: 0 }
    }

    /// Follow one plain update: a snapshot or truncation names every row it
    /// holds, a delta adds the rows it changes and removes the keys it
    /// deletes, an unchanged table keeps its rows and an unavailable one has
    /// none.
    pub(super) fn advance(&mut self, table: TableId, format: u16, plain: &[u8]) -> io::Result<()> {
        let status = plain.first().copied().unwrap_or(UNCHANGED);
        match status {
            UNCHANGED => return Ok(()),
            SNAPSHOT | TRUNCATED | DELTA => {}
            _ => {
                self.clear();
                return Ok(());
            }
        }
        let mut counts = 0;
        let mut named = Vec::new();
        let mut deleted = Vec::new();
        table.walk_update(plain, format, &mut |token, _| match token {
            Token::Count => counts += 1,
            Token::Key(key) if status == DELTA && counts == 2 => deleted.push(key),
            Token::Key(key) => named.push(key),
            _ => {}
        })?;
        if status != DELTA {
            // Plain updates hold their rows in increasing key order.
            if self.keys != named {
                self.keys = named;
                self.changes += 1;
            }
            return Ok(());
        }
        let mut changes = KeyChanges {
            inserted: named
                .into_iter()
                .filter(|key| self.keys.binary_search(key).is_err())
                .collect(),
            deleted,
        };
        self.apply(&mut changes);
        Ok(())
    }

    /// Insert and delete what a later entry changed, leaving `changes` empty:
    /// inserted keys the set does not hold and deleted keys, each in
    /// increasing order.
    pub(super) fn apply(&mut self, changes: &mut KeyChanges) {
        let inserted = !changes.inserted.is_empty();
        let before = self.keys.len();
        if inserted {
            let mut merged = Vec::with_capacity(self.keys.len() + changes.inserted.len());
            let mut held = self.keys.iter().copied().peekable();
            for &key in &changes.inserted {
                while let Some(before) = held.next_if(|held| *held < key) {
                    merged.push(before);
                }
                merged.push(key);
            }
            merged.extend(held);
            self.keys = merged;
            changes.inserted.clear();
        }
        if !changes.deleted.is_empty() {
            let deleted = &changes.deleted;
            self.keys.retain(|key| deleted.binary_search(key).is_err());
            changes.deleted.clear();
        }
        if inserted || self.keys.len() != before {
            self.changes += 1;
        }
    }
}

/// The rows a keyed later entry inserts (the keys it stores as distances)
/// and the keys it deletes, as its reader decodes them.
#[derive(Debug, Default)]
pub(super) struct KeyChanges {
    pub inserted: Vec<u128>,
    pub deleted: Vec<u128>,
}

/// Stores one run of increasing keys (an entry's changed rows, residual rows
/// or deleted keys): a key the frame before holds as one plus how many of its
/// rows it skips since the key before it, any other as zero and then its
/// distance past the key before it (from zero for the first).
pub(super) struct KeyWriter<'a> {
    keys: &'a [u128],
    /// How many of `keys` lie at or before the key before.
    at: usize,
    previous: Option<u128>,
}

impl<'a> KeyWriter<'a> {
    pub(super) fn new(keys: &'a KeySet) -> Self {
        Self {
            keys: keys.keys(),
            at: 0,
            previous: None,
        }
    }

    /// Append `key`, which follows the run's key before it.
    pub(super) fn put(&mut self, key: u128, out: &mut Vec<u8>) -> io::Result<()> {
        if self.previous.is_some_and(|previous| key <= previous) {
            return Err(invalid("grouped row keys did not advance"));
        }
        let rest = &self.keys[self.at..];
        match rest.binary_search(&key) {
            Ok(skipped) => {
                put_varint(out, skipped as u128 + 1);
                self.at += skipped + 1;
            }
            Err(skipped) => {
                put_varint(out, 0);
                // The key before is below `key`, so one past it does not overflow.
                put_varint(out, key - self.previous.map_or(0, |previous| previous + 1));
                self.at += skipped;
            }
        }
        self.previous = Some(key);
        Ok(())
    }
}

/// Reads one run of keys a [`KeyWriter`] stored.
pub(super) struct KeyReader<'a> {
    keys: &'a [u128],
    at: usize,
    previous: Option<u128>,
}

impl<'a> KeyReader<'a> {
    pub(super) fn new(keys: &'a KeySet) -> Self {
        Self {
            keys: keys.keys(),
            at: 0,
            previous: None,
        }
    }

    /// The run's next key, and whether it was stored as a distance (a key
    /// the frame before does not hold); a stored key it holds is refused.
    pub(super) fn take(
        &mut self,
        table: TableId,
        column: &mut Decoder<'_>,
    ) -> io::Result<(u128, bool)> {
        let code = column.varint()?;
        let key = if code == 0 {
            let distance = column.varint()?;
            let key = self
                .previous
                .map_or(Some(0), |previous| previous.checked_add(1))
                .and_then(|from| from.checked_add(distance))
                .ok_or_else(|| invalid("row key addition overflow"))?;
            let rest = &self.keys[self.at..];
            let skipped = rest.partition_point(|held| *held < key);
            if rest.get(skipped) == Some(&key) {
                return Err(invalid("noncanonical grouped row key"));
            }
            self.at += skipped;
            key
        } else {
            let skipped = usize::try_from(code - 1)
                .ok()
                .filter(|skipped| *skipped < self.keys.len() - self.at)
                .ok_or_else(|| invalid("grouped row key past the rows before"))?;
            self.at += skipped + 1;
            self.keys[self.at - 1]
        };
        table.check_key(key)?;
        self.previous = Some(key);
        Ok((key, code == 0))
    }
}

/// The masks one way of writing a table's later entries stored most
/// recently, the latest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MaskList(Vec<u64>);

impl MaskList {
    /// Append `mask`: one plus its place among the kept masks, or zero and
    /// its `width` little-endian bytes. It becomes the latest either way.
    pub(super) fn put(&mut self, mask: u64, width: usize, out: &mut Vec<u8>) {
        match self.0.iter().position(|kept| *kept == mask) {
            Some(at) => {
                put_varint(out, at as u128 + 1);
                self.0.remove(at);
            }
            None => {
                put_varint(out, 0);
                out.extend_from_slice(&mask.to_le_bytes()[..width]);
                self.0.truncate(MASKS_KEPT - 1);
            }
        }
        self.0.insert(0, mask);
    }

    /// Read a mask [`Self::put`] stored; a stored mask the list keeps is
    /// refused. The caller validates the mask's bits.
    pub(super) fn take(&mut self, width: usize, column: &mut Decoder<'_>) -> io::Result<u64> {
        let code = column.varint()?;
        let mask = if code == 0 {
            let mut bytes = [0; 8];
            bytes[..width].copy_from_slice(column.take(width)?);
            let mask = u64::from_le_bytes(bytes);
            if self.0.contains(&mask) {
                return Err(invalid("noncanonical grouped mask"));
            }
            self.0.truncate(MASKS_KEPT - 1);
            mask
        } else {
            let at = usize::try_from(code - 1)
                .ok()
                .filter(|at| *at < self.0.len())
                .ok_or_else(|| invalid("grouped mask past the kept masks"))?;
            self.0.remove(at)
        };
        self.0.insert(0, mask);
        Ok(mask)
    }
}

/// How a later entry stores its row keys and masks: against the table's
/// rows at the frame before and its stream's recent masks (keyed blocks), or
/// as its stream's model decisions (modelled sections, with `model`). A reader
/// also collects the rows the entry inserts and deletes.
pub(super) struct RowCoding<'a> {
    pub keys: &'a KeySet,
    pub masks: &'a mut MaskList,
    pub model: Option<&'a mut Model>,
    pub changes: Option<&'a mut KeyChanges>,
}

/// An entry's changed or residual rows being written: listed rows go to the
/// keys and masks columns as they come, modelled ones once all are known.
/// A chosen stream feeds its listed pair from the same row calls.
pub(super) enum RowRun<'a> {
    Listed(KeyWriter<'a>),
    Modelled {
        rows: Vec<(u128, u64)>,
        listed: KeyWriter<'a>,
    },
}

/// An entry's changed or residual rows being read: modelled ones were all
/// decoded when the entry's rows started.
pub(super) enum RowRead<'a> {
    Listed(KeyReader<'a>),
    Modelled {
        rows: Vec<(u128, u64, bool)>,
        at: usize,
    },
}

impl<'a> RowCoding<'a> {
    /// Whether rows are model decisions.
    pub(super) fn modelled(&self) -> bool {
        self.model.is_some()
    }

    /// Rows a modelled entry may store beyond its key bytes: the table's
    /// rows at the frame before, which can cost less than a byte each.
    pub(super) fn held_rows(&self) -> usize {
        if self.modelled() {
            self.keys.keys().len()
        } else {
            0
        }
    }

    /// Start writing an entry's changed or residual rows.
    pub(super) fn run(&self) -> RowRun<'a> {
        if self.modelled() {
            RowRun::Modelled {
                rows: Vec::new(),
                listed: KeyWriter::new(self.keys),
            }
        } else {
            RowRun::Listed(KeyWriter::new(self.keys))
        }
    }

    /// Store one row of `run`, after the row before it.
    pub(super) fn put_row(
        &mut self,
        run: &mut RowRun<'a>,
        key: u128,
        mask: u64,
        width: usize,
        columns: &mut Columns,
    ) -> io::Result<()> {
        match run {
            RowRun::Listed(writer) => {
                writer.put(key, &mut columns.0[KEYS])?;
                self.masks.put(mask, width, &mut columns.0[MASKS]);
            }
            RowRun::Modelled { rows, listed } => {
                rows.push((key, mask));
                let (keys, masks) = columns.1.split_at_mut(MASKS);
                if let (Some(keys), Some(masks)) = (&mut keys[KEYS], &mut masks[0]) {
                    listed.put(key, keys)?;
                    self.masks.put(mask, width, masks);
                }
            }
        }
        Ok(())
    }

    /// End `run`: a modelled run's rows become its model's decisions.
    pub(super) fn finish_run(&mut self, run: RowRun<'a>, columns: &mut Columns) -> io::Result<()> {
        if let (RowRun::Modelled { rows, .. }, Some(model)) = (run, self.model.as_deref_mut()) {
            let (keys, masks) = columns.0.split_at_mut(MASKS);
            model.put(self.keys, &rows, &mut keys[KEYS], &mut masks[0])?;
        }
        Ok(())
    }

    /// Start reading an entry's `count` changed or residual rows.
    pub(super) fn read(
        &mut self,
        table: TableId,
        count: usize,
        columns: &mut [Decoder<'_>],
    ) -> io::Result<RowRead<'a>> {
        Ok(match self.model.as_deref_mut() {
            Some(model) => {
                let (keys, masks) = columns.split_at_mut(MASKS);
                RowRead::Modelled {
                    rows: model.take(table, self.keys, count, &mut keys[KEYS], &mut masks[0])?,
                    at: 0,
                }
            }
            None => RowRead::Listed(KeyReader::new(self.keys)),
        })
    }

    /// The next row of `read` and its mask, noting an inserted row.
    pub(super) fn take_row(
        &mut self,
        read: &mut RowRead<'a>,
        table: TableId,
        width: usize,
        columns: &mut [Decoder<'_>],
    ) -> io::Result<(u128, u64)> {
        let (key, mask, inserted) = match read {
            RowRead::Listed(reader) => {
                let (key, inserted) = reader.take(table, &mut columns[KEYS])?;
                (key, self.masks.take(width, &mut columns[MASKS])?, inserted)
            }
            RowRead::Modelled { rows, at } => {
                let row = rows
                    .get(*at)
                    .copied()
                    .ok_or_else(|| invalid("modelled entry has fewer rows than its count"))?;
                *at += 1;
                if *at == rows.len() {
                    self.model
                        .as_deref_mut()
                        .expect("a modelled run has a model")
                        .recycle_run(std::mem::take(rows));
                }
                row
            }
        };
        self.note_changed(key, inserted);
        Ok((key, mask))
    }
}

impl RowCoding<'_> {
    /// Note a key a reader decoded, `inserted` when stored as a distance.
    pub(super) fn note_changed(&mut self, key: u128, inserted: bool) {
        if inserted && let Some(changes) = self.changes.as_deref_mut() {
            changes.inserted.push(key);
        }
    }

    /// Note a deleted key a reader decoded.
    pub(super) fn note_deleted(&mut self, key: u128) {
        if let Some(changes) = self.changes.as_deref_mut() {
            changes.deleted.push(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: TableId = TableId::Divisions;

    fn set(keys: &[u128]) -> KeySet {
        KeySet::of(keys.to_vec())
    }

    #[test]
    fn held_keys_cost_their_skipped_rows_and_others_their_distance() {
        let held = set(&[10 << 32, 11 << 32, 12 << 32, 40 << 32]);
        let mut out = Vec::new();
        let mut writer = KeyWriter::new(&held);
        for key in [11 << 32, 20 << 32, 40 << 32, (40 << 32) + 1] {
            writer.put(key, &mut out).unwrap();
        }
        // 11 skips one held row; 20 is inserted, stored as its distance past
        // 11's successor; 40 skips none of the rows after 20; 40 + 1 is
        // inserted right after 40.
        let mut expected = Vec::new();
        put_varint(&mut expected, 2);
        put_varint(&mut expected, 0);
        put_varint(&mut expected, (20 << 32) - ((11 << 32) + 1));
        put_varint(&mut expected, 1);
        put_varint(&mut expected, 0);
        put_varint(&mut expected, 0);
        assert_eq!(out, expected);
        let mut column = Decoder::new(&out);
        let mut reader = KeyReader::new(&held);
        let keys: Vec<(u128, bool)> = (0..4)
            .map(|_| reader.take(TABLE, &mut column).unwrap())
            .collect();
        // The keys stored as distances are the inserted ones.
        assert_eq!(
            keys,
            [
                (11 << 32, false),
                (20 << 32, true),
                (40 << 32, false),
                ((40 << 32) + 1, true)
            ]
        );
        column.finish().unwrap();
        assert!(writer.put(40 << 32, &mut out).is_err());
    }

    #[test]
    fn readers_refuse_restated_and_out_of_range_keys() {
        let held = set(&[5, 9]);
        let read = |bytes: &[u8]| {
            let mut column = Decoder::new(bytes);
            KeyReader::new(&held).take(TABLE, &mut column)
        };
        // A held key stored as a distance is noncanonical.
        assert!(
            read(&[0, 9])
                .unwrap_err()
                .to_string()
                .contains("noncanonical")
        );
        // Two held rows: skipping two of them names none.
        assert!(read(&[3]).is_err());
        assert_eq!(read(&[2]).unwrap(), (9, false));
        assert_eq!(read(&[0, 7]).unwrap(), (7, true));
    }

    #[test]
    fn key_sets_follow_snapshots_deltas_and_unavailability() {
        use crate::rows::DivisionRow;
        let format = crate::FORMAT_VERSION;
        let rows = |ids: &[u64]| {
            crate::rows::TableRows::Divisions(
                ids.iter()
                    .map(|&id| DivisionRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let mut snapshot = vec![SNAPSHOT];
        snapshot.extend_from_slice(&3_u32.to_le_bytes());
        rows(&[2, 4, 6]).encode_rows(&mut snapshot).unwrap();
        let mut keys = KeySet::default();
        keys.advance(TABLE, format, &snapshot).unwrap();
        assert_eq!(keys.keys(), [2, 4, 6]);
        // Against rows 2 and 6, rows 4 and 5 are inserted and 6 deleted; 4
        // is already held.
        let mut delta = vec![DELTA];
        delta.extend_from_slice(&9_i32.to_le_bytes());
        let mut changes = Vec::new();
        assert!(
            rows(&[2, 4, 5])
                .encode_changes(&rows(&[2, 6]), &mut changes, &mut Vec::new())
                .unwrap()
        );
        delta.extend_from_slice(&changes);
        let mut followed = keys.clone();
        keys.advance(TABLE, format, &delta).unwrap();
        assert_eq!(keys.keys(), [2, 4, 5]);
        // A reader applies the same change from the keys it decoded.
        let mut changes = KeyChanges {
            inserted: vec![5],
            deleted: vec![6],
        };
        followed.apply(&mut changes);
        assert_eq!(followed, keys);
        assert!(changes.inserted.is_empty() && changes.deleted.is_empty());
        let mut unchanged = vec![UNCHANGED];
        unchanged.extend_from_slice(&9_i32.to_le_bytes());
        keys.advance(TABLE, format, &unchanged).unwrap();
        assert_eq!(keys.keys(), [2, 4, 5]);
        keys.advance(
            TABLE,
            format,
            &[super::super::UNAVAILABLE, 1, 0, 0, 0, b'x'],
        )
        .unwrap();
        assert!(keys.keys().is_empty());
    }

    #[test]
    fn key_change_counter_tracks_membership_and_ignores_no_ops() {
        use crate::rows::{DivisionRow, TableRows};
        let snapshot = |ids: &[u64], status: u8| {
            let mut plain = vec![status];
            if status == TRUNCATED {
                plain.extend_from_slice(&1_u32.to_le_bytes());
            }
            plain.extend_from_slice(&(ids.len() as u32).to_le_bytes());
            TableRows::Divisions(
                ids.iter()
                    .map(|&id| DivisionRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            )
            .encode_rows(&mut plain)
            .unwrap();
            plain
        };
        let format = crate::FORMAT_VERSION;
        let mut keys = KeySet::default();
        keys.clear();
        keys.apply(&mut KeyChanges::default());
        assert_eq!(keys.changes(), 0);
        keys.advance(TABLE, format, &snapshot(&[4, 8], SNAPSHOT))
            .unwrap();
        assert_eq!(keys.changes(), 1);
        keys.advance(TABLE, format, &snapshot(&[4, 8], SNAPSHOT))
            .unwrap();
        keys.advance(TABLE, format, &snapshot(&[4, 8], TRUNCATED))
            .unwrap();
        keys.advance(TABLE, format, &[UNCHANGED]).unwrap();
        keys.apply(&mut KeyChanges {
            inserted: Vec::new(),
            deleted: vec![12],
        });
        assert_eq!(keys.changes(), 1);
        // Equal length does not imply equal membership.
        keys.apply(&mut KeyChanges {
            inserted: vec![12],
            deleted: vec![4],
        });
        assert_eq!(keys.keys(), [8, 12]);
        assert_eq!(keys.changes(), 2);
        keys.advance(TABLE, format, &snapshot(&[4, 8], TRUNCATED))
            .unwrap();
        assert_eq!(keys.changes(), 3);
        // An ordinary delta advances through the same membership operation.
        let mut delta = vec![DELTA];
        delta.extend_from_slice(&9_i32.to_le_bytes());
        delta.extend_from_slice(&0_u32.to_le_bytes());
        delta.extend_from_slice(&1_u32.to_le_bytes());
        put_varint(&mut delta, 4);
        keys.advance(TABLE, format, &delta).unwrap();
        assert_eq!(keys.keys(), [8]);
        assert_eq!(keys.changes(), 4);
        keys.clear();
        assert_eq!(keys.changes(), 5);
        keys.clear();
        assert_eq!(keys.changes(), 5);
        keys.advance(TABLE, format, &snapshot(&[4], SNAPSHOT))
            .unwrap();
        assert_eq!(keys.changes(), 6);
        keys.advance(
            TABLE,
            format,
            &[super::super::UNAVAILABLE, 1, 0, 0, 0, b'x'],
        )
        .unwrap();
        assert_eq!(keys.changes(), 7);
        keys.advance(
            TABLE,
            format,
            &[super::super::UNAVAILABLE, 1, 0, 0, 0, b'x'],
        )
        .unwrap();
        assert_eq!(keys.changes(), 7);
    }

    #[test]
    fn masks_are_their_place_among_recent_masks() {
        let mut list = MaskList::default();
        let mut out = Vec::new();
        for mask in [1 << 19, 1 << 19, 1 << 4, 1 << 19, 1 << 4] {
            list.put(mask, 4, &mut out);
        }
        assert_eq!(out, [0, 0, 0, 8, 0, 1, 0, 16, 0, 0, 0, 2, 2]);
        let mut column = Decoder::new(&out);
        let mut reader = MaskList::default();
        let masks: Vec<u64> = (0..5)
            .map(|_| reader.take(4, &mut column).unwrap())
            .collect();
        assert_eq!(masks, [1 << 19, 1 << 19, 1 << 4, 1 << 19, 1 << 4]);
        column.finish().unwrap();
        assert_eq!(reader, list);
        // A kept mask stored as itself is noncanonical.
        let mut column = Decoder::new(&[0, 16, 0, 0, 0]);
        assert!(reader.take(4, &mut column).is_err());
        // The list keeps the latest MASKS_KEPT masks.
        let mut list = MaskList::default();
        for mask in 1..=MASKS_KEPT as u64 + 1 {
            list.put(mask, 8, &mut Vec::new());
        }
        assert_eq!(list.0.len(), MASKS_KEPT);
        assert!(!list.0.contains(&1) && list.0[0] == MASKS_KEPT as u64 + 1);
    }
}
