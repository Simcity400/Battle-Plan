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
use engine_allocation as allocation;
use replay_data::measurement::{self, Counter};
use std::io;

/// Masks a [`MaskList`] keeps.
pub(super) const MASKS_KEPT: usize = 64;

/// The keys of one table's rows at the frame before, in order.
#[derive(Clone, Debug, Default)]
pub(super) struct KeySet {
    keys: Vec<u128>,
    /// Membership changes within this table's live block.
    changes: u64,
    /// Malformed plain updates can leave keys unordered.
    unordered: bool,
    /// The last membership change, by place.
    edit: Edit,
}

/// Equality ignores the placement cache, not keys or membership-change counts.
impl PartialEq for KeySet {
    fn eq(&self, other: &Self) -> bool {
        self.keys == other.keys && self.changes == other.changes
    }
}

impl Eq for KeySet {}

/// Removal positions refer to the old keys; insertion positions refer to the final keys.
#[derive(Clone, Debug, Default)]
pub(super) struct Edit {
    /// Only the latest recorded membership change can be followed by place.
    from: Option<u64>,
    removed: Vec<usize>,
    added: Vec<usize>,
}

impl Edit {
    pub(super) fn removed(&self) -> &[usize] {
        &self.removed
    }

    pub(super) fn added(&self) -> &[usize] {
        &self.added
    }
}

/// Positions must be increasing; move each intervening run once.
pub(super) fn remove_at<T: Copy>(values: &mut Vec<T>, removed: &[usize]) {
    let Some(&first) = removed.first() else {
        return;
    };
    let mut write = first;
    for (index, &at) in removed.iter().enumerate() {
        let end = removed.get(index + 1).copied().unwrap_or(values.len());
        measurement::count(
            if std::mem::size_of::<T>() == 16 {
                Counter::KeyCopyBytes
            } else {
                Counter::ModelCopyBytes
            },
            (end - at - 1) * std::mem::size_of::<T>(),
        );
        values.copy_within(at + 1..end, write);
        write += end - at - 1;
    }
    values.truncate(write);
}

/// Insertion positions are increasing and refer to the final vector.
pub(super) fn insert_at<T: Copy + Default>(
    values: &mut Vec<T>,
    added: &[usize],
    mut value: impl FnMut(usize) -> T,
) -> io::Result<()> {
    let old = values.len();
    allocation::reserve(values, added.len())?;
    values.resize(old + added.len(), T::default());
    let mut end = old;
    for (index, &place) in added.iter().enumerate().rev() {
        let at = place - index;
        measurement::count(
            if std::mem::size_of::<T>() == 16 {
                Counter::KeyCopyBytes
            } else {
                Counter::ModelCopyBytes
            },
            (end - at) * std::mem::size_of::<T>(),
        );
        values.copy_within(at..end, place + 1);
        values[place] = value(index);
        end = at;
    }
    Ok(())
}

fn increasing(keys: &[u128]) -> bool {
    keys.windows(2).all(|pair| pair[0] < pair[1])
}

impl KeySet {
    pub(super) fn clear(&mut self) {
        if !self.keys.is_empty() {
            self.keys.clear();
            self.changes += 1;
            self.unordered = false;
            self.edit.from = None;
        }
    }

    pub(super) fn changes(&self) -> u64 {
        self.changes
    }

    pub(super) fn keys(&self) -> &[u128] {
        &self.keys
    }

    pub(super) fn edit_from(&self, changes: Option<u64>) -> Option<&Edit> {
        let changes = changes?;
        (self.edit.from == Some(changes) && changes + 1 == self.changes).then_some(&self.edit)
    }

    #[cfg(test)]
    pub(super) fn of(keys: Vec<u128>) -> Self {
        Self {
            unordered: !increasing(&keys),
            keys,
            changes: 0,
            edit: Edit::default(),
        }
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
        let mut failure = None;
        let walked = table.walk_update(plain, format, &mut |token, _| {
            let (keys, key) = match token {
                _ if failure.is_some() => return,
                Token::Count => {
                    counts += 1;
                    return;
                }
                Token::Key(key) if status == DELTA && counts == 2 => (&mut deleted, key),
                Token::Key(key) => (&mut named, key),
                _ => return,
            };
            match allocation::reserve(keys, 1) {
                Ok(()) => keys.push(key),
                Err(error) => failure = Some(error),
            }
        });
        // An allocation failure is not a framing refusal: it comes first.
        failure.map_or(walked, Err)?;
        if status != DELTA {
            // Plain updates hold their rows in increasing key order.
            if self.keys != named {
                self.unordered = !increasing(&named);
                self.keys = named;
                self.changes += 1;
                self.edit.from = None;
            }
            return Ok(());
        }
        let mut changes = KeyChanges {
            inserted: allocation::collect_vec(
                named
                    .into_iter()
                    .filter(|key| self.keys.binary_search(key).is_err()),
            )?,
            deleted,
        };
        self.apply(&mut changes)
    }

    /// Unordered keys or changes retain the original merge semantics.
    pub(super) fn apply(&mut self, changes: &mut KeyChanges) -> io::Result<()> {
        let inserted = !changes.inserted.is_empty();
        if !inserted && changes.deleted.is_empty() {
            return Ok(());
        }
        let (before, from) = (self.keys.len(), self.changes);
        let placed = !self.unordered
            && increasing(&changes.inserted)
            && increasing(&changes.deleted)
            && self.place(changes)?;
        if placed {
            remove_at(&mut self.keys, &self.edit.removed);
            let added = &changes.inserted;
            insert_at(&mut self.keys, &self.edit.added, |index| added[index])?;
        } else {
            self.merge(changes)?;
        }
        changes.inserted.clear();
        changes.deleted.clear();
        if inserted || self.keys.len() != before {
            self.changes += 1;
        }
        self.edit.from = (placed && self.changes != from).then_some(from);
        Ok(())
    }

    /// A key already held requires the original merge, not placed insertion.
    fn place(&mut self, changes: &mut KeyChanges) -> io::Result<bool> {
        let (keys, edit) = (&self.keys, &mut self.edit);
        edit.removed.clear();
        edit.added.clear();
        for key in &changes.deleted {
            if let Ok(at) = keys.binary_search(key) {
                allocation::reserve(&mut edit.removed, 1)?;
                edit.removed.push(at);
            }
        }
        // A key both inserted and deleted ends deleted.
        let (deleted, mut next) = (&changes.deleted, 0);
        changes.inserted.retain(|key| {
            while deleted.get(next).is_some_and(|deleted| deleted < key) {
                next += 1;
            }
            deleted.get(next) != Some(key)
        });
        allocation::reserve(&mut edit.added, changes.inserted.len())?;
        for (index, key) in changes.inserted.iter().enumerate() {
            let Err(at) = keys.binary_search(key) else {
                return Ok(false);
            };
            let kept = at - edit.removed.partition_point(|&removed| removed < at);
            edit.added.push(kept + index);
        }
        allocation::reserve(&mut self.keys, changes.inserted.len())?;
        Ok(true)
    }

    fn merge(&mut self, changes: &KeyChanges) -> io::Result<()> {
        if !changes.inserted.is_empty() {
            let mut merged =
                allocation::vec(allocation::add(self.keys.len(), changes.inserted.len())?)?;
            let mut held = self.keys.iter().copied().peekable();
            for &key in &changes.inserted {
                while let Some(before) = held.next_if(|held| *held < key) {
                    merged.push(before);
                }
                merged.push(key);
            }
            merged.extend(held);
            measurement::count(
                Counter::KeyCopyBytes,
                merged.len() * std::mem::size_of::<u128>(),
            );
            self.keys = merged;
        }
        if !changes.deleted.is_empty() {
            let deleted = &changes.deleted;
            if measurement::enabled() {
                let (mut removed, mut copied) = (false, 0);
                self.keys.retain(|key| {
                    let keep = deleted.binary_search(key).is_err();
                    copied += usize::from(keep && removed);
                    removed |= !keep;
                    keep
                });
                measurement::count(Counter::KeyCopyBytes, copied * std::mem::size_of::<u128>());
            } else {
                self.keys.retain(|key| deleted.binary_search(key).is_err());
            }
        }
        self.unordered = !increasing(&self.keys);
        Ok(())
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
        self.note_changed(key, inserted)?;
        Ok((key, mask))
    }
}

impl RowCoding<'_> {
    /// Note a key a reader decoded, `inserted` when stored as a distance.
    pub(super) fn note_changed(&mut self, key: u128, inserted: bool) -> io::Result<()> {
        if inserted && let Some(changes) = self.changes.as_deref_mut() {
            allocation::reserve(&mut changes.inserted, 1)?;
            changes.inserted.push(key);
        }
        Ok(())
    }

    /// Note a deleted key a reader decoded.
    pub(super) fn note_deleted(&mut self, key: u128) -> io::Result<()> {
        if let Some(changes) = self.changes.as_deref_mut() {
            allocation::reserve(&mut changes.deleted, 1)?;
            changes.deleted.push(key);
        }
        Ok(())
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
        followed.apply(&mut changes).unwrap();
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
        keys.apply(&mut KeyChanges::default()).unwrap();
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
        })
        .unwrap();
        assert_eq!(keys.changes(), 1);
        // Equal length does not imply equal membership.
        keys.apply(&mut KeyChanges {
            inserted: vec![12],
            deleted: vec![4],
        })
        .unwrap();
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

    /// Independent writer reference from before checked growth and placed edits.
    fn unchecked_advance(keys: &mut Vec<u128>, changes: &mut u64, format: u16, plain: &[u8]) {
        let status = plain.first().copied().unwrap_or(UNCHANGED);
        match status {
            UNCHANGED => return,
            SNAPSHOT | TRUNCATED | DELTA => {}
            _ => {
                if !keys.is_empty() {
                    keys.clear();
                    *changes += 1;
                }
                return;
            }
        }
        let mut counts = 0;
        let mut named = Vec::new();
        let mut deleted = Vec::new();
        TABLE
            .walk_update(plain, format, &mut |token, _| match token {
                Token::Count => counts += 1,
                Token::Key(key) if status == DELTA && counts == 2 => deleted.push(key),
                Token::Key(key) => named.push(key),
                _ => {}
            })
            .unwrap();
        if status != DELTA {
            if *keys != named {
                *keys = named;
                *changes += 1;
            }
            return;
        }
        let inserted: Vec<u128> = named
            .into_iter()
            .filter(|key| keys.binary_search(key).is_err())
            .collect();
        let before = keys.len();
        if !inserted.is_empty() {
            let mut merged = Vec::with_capacity(keys.len() + inserted.len());
            let mut held = keys.iter().copied().peekable();
            for &key in &inserted {
                while let Some(before) = held.next_if(|held| *held < key) {
                    merged.push(before);
                }
                merged.push(key);
            }
            merged.extend(held);
            *keys = merged;
        }
        if !deleted.is_empty() {
            keys.retain(|key| deleted.binary_search(key).is_err());
        }
        if !inserted.is_empty() || keys.len() != before {
            *changes += 1;
        }
    }

    /// Compare writer and reader keys and change counts with the independent unchecked algorithm.
    #[test]
    fn checked_key_sets_follow_a_multi_hour_keyed_table_exactly_as_before() {
        use crate::rows::{DivisionRow, TableRows};
        let format = crate::FORMAT_VERSION;
        let rows = |ids: &[u64], hour: i64| {
            TableRows::Divisions(
                ids.iter()
                    .map(|&id| DivisionRow {
                        id,
                        organisation: (id as i64 * 7 + hour / 3) * 10_000,
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let whole = |status: u8, ids: &[u64], hour: i64| {
            let mut plain = vec![status];
            if status == TRUNCATED {
                plain.extend_from_slice(&1_u32.to_le_bytes());
            }
            plain.extend_from_slice(&(ids.len() as u32).to_le_bytes());
            rows(ids, hour).encode_rows(&mut plain).unwrap();
            plain
        };
        let unchanged = |hour: i64| {
            let mut plain = vec![UNCHANGED];
            plain.extend_from_slice(&((hour - 1) as i32).to_le_bytes());
            plain
        };
        let mut seed = 0x6b65_7973_u64;
        let mut next = |below: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % below
        };
        let mut writer = KeySet::default();
        let mut reader = KeySet::default();
        let (mut reference, mut reference_changes) = (Vec::new(), 0_u64);
        let mut ids: Vec<u64> = (1..=40).collect();
        let mut complete = false;
        let mut kinds = [0_usize; 5];
        for hour in 0..400_i64 {
            let kind = match next(20) {
                _ if !complete => 0,
                0 => 2,
                1 => 3,
                2 => 4,
                _ => 1,
            };
            let before = ids.clone();
            let plain = match kind {
                // A snapshot: the first hour, and every hour after a break.
                0 => {
                    complete = true;
                    whole(SNAPSHOT, &ids, hour)
                }
                // A delta: delete, insert and keep rows, changing some.
                1 => {
                    ids.retain(|_| next(12) != 0);
                    for _ in 0..next(4) {
                        let id = 1 + next(300);
                        if let Err(at) = ids.binary_search(&id) {
                            ids.insert(at, id);
                        }
                    }
                    let mut body = Vec::new();
                    if rows(&ids, hour)
                        .encode_changes(&rows(&before, hour - 1), &mut body, &mut Vec::new())
                        .unwrap()
                    {
                        let mut plain = vec![DELTA];
                        plain.extend_from_slice(&((hour - 1) as i32).to_le_bytes());
                        plain.extend_from_slice(&body);
                        plain
                    } else {
                        unchanged(hour)
                    }
                }
                2 => unchanged(hour),
                3 => {
                    complete = false;
                    whole(TRUNCATED, &ids, hour)
                }
                _ => {
                    complete = false;
                    vec![super::super::UNAVAILABLE, 1, 0, 0, 0, b'x']
                }
            };
            kinds[kind] += 1;
            writer.advance(TABLE, format, &plain).unwrap();
            unchecked_advance(&mut reference, &mut reference_changes, format, &plain);
            assert_eq!(writer.keys(), reference, "hour {hour}");
            assert_eq!(writer.changes(), reference_changes, "hour {hour}");
            // Reader membership comes from decoded inserts and deletes, not the writer's key set.
            if plain[0] == DELTA {
                let held: Vec<u128> = before.iter().map(|&id| u128::from(id)).collect();
                let now: Vec<u128> = ids.iter().map(|&id| u128::from(id)).collect();
                let mut changes = KeyChanges {
                    inserted: now
                        .iter()
                        .copied()
                        .filter(|key| held.binary_search(key).is_err())
                        .collect(),
                    deleted: held
                        .iter()
                        .copied()
                        .filter(|key| now.binary_search(key).is_err())
                        .collect(),
                };
                reader.apply(&mut changes).unwrap();
            } else {
                reader.advance(TABLE, format, &plain).unwrap();
            }
            assert_eq!(reader, writer, "hour {hour}");
        }
        // Every kind of hour occurred.
        assert!(kinds.iter().all(|&count| count > 0), "{kinds:?}");
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

    /// Independent pre-placement merge: insert first, then delete.
    fn merged(keys: &[u128], inserted: &[u128], deleted: &[u128]) -> Vec<u128> {
        let mut merged = Vec::new();
        let mut held = keys.iter().copied().peekable();
        for &key in inserted {
            while let Some(before) = held.next_if(|held| *held < key) {
                merged.push(before);
            }
            merged.push(key);
        }
        merged.extend(held);
        merged.retain(|key| deleted.binary_search(key).is_err());
        merged
    }

    fn draw(seed: &mut u64, below: u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (*seed >> 33) % below
    }

    /// Include unsorted and repeated keys as well as strictly increasing changes.
    fn drawn(seed: &mut u64, span: u64, most: u64) -> Vec<u128> {
        let count = draw(seed, most + 1);
        let mut keys: Vec<u128> = (0..count).map(|_| u128::from(draw(seed, span))).collect();
        if draw(seed, 8) != 0 {
            keys.sort_unstable();
            keys.dedup();
        }
        keys
    }

    /// Compare placed edits with the original merge, including malformed ordering and overlaps.
    #[test]
    fn placed_changes_hold_the_keys_the_merge_held() {
        let mut seed = 0x706c_6163_u64;
        let (mut placed, mut merges) = (0, 0);
        for case in 0..3_000 {
            let span = 1 + draw(&mut seed, 80);
            let held = drawn(&mut seed, span, 40);
            let deleted = drawn(&mut seed, span, 8);
            let mut inserted = drawn(&mut seed, span, 8);
            // Mostly keys not held, as a reader's inserted keys are.
            if draw(&mut seed, 4) != 0 {
                inserted.retain(|key| !held.contains(key));
            }
            let mut set = KeySet::of(held.clone());
            let mut changes = KeyChanges {
                inserted: inserted.clone(),
                deleted: deleted.clone(),
            };
            set.apply(&mut changes).unwrap();
            assert!(changes.inserted.is_empty() && changes.deleted.is_empty());
            let expected = merged(&held, &inserted, &deleted);
            assert_eq!(set.keys(), expected, "case {case}");
            let changed = !inserted.is_empty() || expected.len() != held.len();
            assert_eq!(set.changes(), u64::from(changed), "case {case}");
            assert_eq!(set.unordered, !increasing(&expected), "case {case}");
            match set.edit_from(Some(0)) {
                Some(edit) => {
                    placed += 1;
                    let mut rebuilt = held.clone();
                    remove_at(&mut rebuilt, edit.removed());
                    let added = edit.added();
                    insert_at(&mut rebuilt, added, |index| expected[added[index]]).unwrap();
                    assert_eq!(rebuilt, expected, "case {case}");
                }
                None if changed => merges += 1,
                None => {}
            }
        }
        assert!(
            placed > 1_000 && merges > 100,
            "{placed} placed, {merges} merged"
        );
    }
}
