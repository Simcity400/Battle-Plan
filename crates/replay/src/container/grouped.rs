//! Grouped replay blocks.
//!
//! A grouped block holds one branch's consecutive frames as independently
//! compressed sections: a header (frame hours and which tables have an entry
//! in each frame), diagnostics (callback total, pool bytes, dropped-frame
//! counter and which tables were read) and, for each table, its keyframe entry and its
//! later entries. Table sections are split into columns (control, keys,
//! masks, then one per field) so like values compress together, and a reader
//! that needs few tables decompresses only theirs.
//!
//! Entries rearrange the plain table update wire ([`decode_update`]) without
//! changing what it says: a baseline equal to the previous frame's hour is
//! implied, an unchanged table with that baseline has no entry, counts are
//! varints, and a changed byte field (string, list or nested record) may be a
//! [`patch`] against the same field's previous plain encoding in this block.
//! In a predicted block ([`MAGIC`], [`CHOSEN_MAGIC`], [`RAMPED_MAGIC`], [`KEYED_MAGIC`],
//! [`ANCHORED_MAGIC`], [`PREDICTED_MAGIC`]) a delta against the previous
//! frame may be a residual entry that stores only what did not move as
//! [`predicted`], and a frame whose changes are all predicted has no entry.
//! Its writer keeps, per table, residual or plain entries, and per byte
//! field column, patches or patches whose in-place bytes are differences,
//! whichever compresses smaller. In a keyed block ([`RAMPED_MAGIC`],
//! [`KEYED_MAGIC`]) later deltas and residual entries name each row by how
//! many of the table's rows at the frame before it skips, and store each
//! mask by its place among the masks stored before it ([`keyed`]); a
//! modelled block ([`MAGIC`]) stores those rows and masks as decisions of a
//! context model instead, by which rows the entries before stored and with
//! which masks ([`modelled`]). A chosen block ([`CHOSEN_MAGIC`]) keeps, per
//! table's later section, model decisions or listed skips and mask places,
//! whichever compresses smaller. Its masks column starts with 0 for modelled
//! or 1 for listed; the control, deleted-key run and field columns agree.
//! A ramped block ([`MAGIC`], [`CHOSEN_MAGIC`], [`RAMPED_MAGIC`])
//! predicts each numeric field from the rate its last changes allow,
//! unchanged hours included ([`ramp`]); a keyed one follows a steadily
//! growing change.
//!
//! A keyframe entry may instead be a change against an earlier keyframe of
//! the same table anywhere earlier in the file (its anchor), named by how
//! many blocks back it is, so loaded branches and appended sessions start
//! from what earlier ones stored: a table that does not change, a static one
//! included, is stored once per file. The anchor is the table's latest
//! snapshot, or its latest keyframe while the chain stays within
//! [`chain_charge`], so a seek inflates at most twice a table's snapshot
//! section and rebuilds its rows at most [`REBUILD_SHARE`] times however
//! many blocks back the snapshot is. Earlier layouts anchor within their
//! branch by hour: a predicted block through at most [`MAX_ANCHOR_DEPTH`]
//! anchors, earlier ones only to snapshots.

use super::{
    BlockIndex, CompressedBlock, MAX_BLOCK_RAW_BYTES, Parallel, Replay,
    pack::{self, Cores},
    parallel_map, patch, zstd_error,
};
use crate::{
    BranchId, Hour,
    codec::{Decoder, invalid, invalid_static, put_varint},
    format::FormatVersion,
    frame::{
        DecodedFrame, Frame, Metrics, READ_TIME_NOT_RECORDED, RecordedFrame, SharedView,
        TABLE_COUNT, TableUpdate, TableView, decode_update,
    },
    rows::{
        Change, Kind, TableId, TableRows, Token,
        compatibility::{HISTORY, LastChange},
    },
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{self, Read},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(test)]
#[path = "decoded_section_tests.rs"]
mod decoded_section_tests;
#[cfg(test)]
mod event_feed_tests;
mod keyed;
mod modelled;
mod predicted;
mod ramp;
#[cfg(test)]
mod test_recording;
#[cfg(test)]
mod through_tests;

use crate::frame::recorded::Updates;
use std::sync::{Arc, OnceLock};

use keyed::{KeyChanges, KeyReader, KeySet, KeyWriter, MaskList, RowCoding, RowRun};
use modelled::Model;

/// Grouped blocks whose later entries store row keys and masks as context
/// model decisions ([`modelled`]), predict each numeric field from the rate
/// its last changes allow ([`ramp`]), store a list field that is its patch
/// base without its first element as one tag ([`DROP_FIRST`]) and store a
/// row's advance against the head its list last dropped ([`advance_pair`]).
pub(crate) const MAGIC: &[u8; 4] = b"H4G9";
/// Grouped blocks whose later sections choose model decisions or listed
/// skips and mask places, with the same predictions and patches as advanced
/// blocks. The layout writers emit.
pub(crate) const CHOSEN_MAGIC: &[u8; 4] = b"H4GC";
/// Earlier format-102 trimmed blocks, which store [`DROP_FIRST`] but no
/// advance. Read, never written.
pub(crate) const TRIMMED_MAGIC: &[u8; 4] = b"H4G8";
/// Earlier format-102 modelled blocks, without the [`DROP_FIRST`] tag. Read,
/// never written.
pub(crate) const MODELLED_MAGIC: &[u8; 4] = b"H4G7";
/// Earlier format-102 ramped blocks, whose later entries store row keys and
/// masks against the table's rows and masks before them ([`keyed`]). Read,
/// never written.
pub(crate) const RAMPED_MAGIC: &[u8; 4] = b"H4G6";
/// Earlier format-102 keyed blocks, which predict a numeric field only from
/// its last two or three changes, while it changes every hour. Read, never
/// written.
pub(crate) const KEYED_MAGIC: &[u8; 4] = b"H4G5";
/// Earlier format-102 grouped blocks whose keyframes anchor to any earlier
/// block of the file, with later entries' row keys stored as key
/// differences, their masks as they are, and only constant changes
/// predicted. Read, never written.
pub(crate) const ANCHORED_MAGIC: &[u8; 4] = b"H4G4";
/// Earlier format-102 predicted grouped blocks, whose keyframes anchor only
/// within their branch through at most [`MAX_ANCHOR_DEPTH`] anchors. Read,
/// never written.
pub(crate) const PREDICTED_MAGIC: &[u8; 4] = b"H4G3";
/// Earlier format-102 grouped blocks without field prediction. Read, never
/// written.
pub(crate) const UNPREDICTED_MAGIC: &[u8; 4] = b"H4G2";
/// Earlier format-102 grouped blocks without field prediction, whose
/// diagnostics also hold each read table's time in microseconds. Read, never
/// written.
pub(crate) const TIMED_MAGIC: &[u8; 4] = b"H4GB";

/// The grouped layouts, from their first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    Timed,
    Unpredicted,
    Predicted,
    Anchored,
    Keyed,
    Ramped,
    Modelled,
    Trimmed,
    Advanced,
    Chosen,
}

impl Layout {
    fn of(block: &[u8]) -> Option<Self> {
        match block.get(..4)? {
            magic if magic == CHOSEN_MAGIC => Some(Self::Chosen),
            magic if magic == MAGIC => Some(Self::Advanced),
            magic if magic == TRIMMED_MAGIC => Some(Self::Trimmed),
            magic if magic == MODELLED_MAGIC => Some(Self::Modelled),
            magic if magic == RAMPED_MAGIC => Some(Self::Ramped),
            magic if magic == KEYED_MAGIC => Some(Self::Keyed),
            magic if magic == ANCHORED_MAGIC => Some(Self::Anchored),
            magic if magic == PREDICTED_MAGIC => Some(Self::Predicted),
            magic if magic == UNPREDICTED_MAGIC => Some(Self::Unpredicted),
            magic if magic == TIMED_MAGIC => Some(Self::Timed),
            _ => None,
        }
    }
    fn magic(self) -> &'static [u8; 4] {
        match self {
            Self::Timed => TIMED_MAGIC,
            Self::Unpredicted => UNPREDICTED_MAGIC,
            Self::Predicted => PREDICTED_MAGIC,
            Self::Anchored => ANCHORED_MAGIC,
            Self::Keyed => KEYED_MAGIC,
            Self::Ramped => RAMPED_MAGIC,
            Self::Modelled => MODELLED_MAGIC,
            Self::Trimmed => TRIMMED_MAGIC,
            Self::Advanced => MAGIC,
            Self::Chosen => CHOSEN_MAGIC,
        }
    }
    /// Residual entries and difference patches.
    fn predicts(self) -> bool {
        matches!(
            self,
            Self::Predicted
                | Self::Anchored
                | Self::Keyed
                | Self::Ramped
                | Self::Modelled
                | Self::Trimmed
                | Self::Advanced
                | Self::Chosen
        )
    }
    /// Keyframes anchor to any earlier block of the file, named by how many
    /// blocks back it is, within [`chain_charge`].
    fn anchors_back(self) -> bool {
        matches!(
            self,
            Self::Anchored
                | Self::Keyed
                | Self::Ramped
                | Self::Modelled
                | Self::Trimmed
                | Self::Advanced
                | Self::Chosen
        )
    }
    /// Later entries store row keys and masks against the rows and masks
    /// before them ([`keyed`]).
    fn keyed(self) -> bool {
        matches!(
            self,
            Self::Keyed
                | Self::Ramped
                | Self::Modelled
                | Self::Trimmed
                | Self::Advanced
                | Self::Chosen
        )
    }
    /// Later deltas and residual entries can store their rows and masks as
    /// context model decisions ([`modelled`]); chosen sections can list them.
    fn modelled(self) -> bool {
        matches!(
            self,
            Self::Modelled | Self::Trimmed | Self::Advanced | Self::Chosen
        )
    }
    /// Each later section keeps model decisions or listed skips and mask
    /// places, whichever compresses smaller.
    fn chooses_rows(self) -> bool {
        self == Self::Chosen
    }
    /// A numeric field that changed by a steadily changing amount is
    /// predicted ([`predicted`]).
    fn accelerates(self) -> bool {
        self == Self::Keyed
    }
    /// A numeric field is predicted from the rate its last changes allow,
    /// unchanged hours included ([`ramp`]).
    fn ramped(self) -> bool {
        matches!(
            self,
            Self::Ramped | Self::Modelled | Self::Trimmed | Self::Advanced | Self::Chosen
        )
    }
    /// A list field that is its patch base without its first element is
    /// stored as [`DROP_FIRST`].
    fn trims(self) -> bool {
        matches!(self, Self::Trimmed | Self::Advanced | Self::Chosen)
    }
    /// A numeric field of an [`advance_pair`] without a predicted change is
    /// stored against the change that takes it to its list's head.
    fn advances(self) -> bool {
        matches!(self, Self::Advanced | Self::Chosen)
    }
}

/// Whether a block's first bytes name a grouped layout.
pub(crate) fn is_grouped(block: &[u8]) -> bool {
    Layout::of(block).is_some()
}
/// Recordings from this format on write grouped blocks.
pub(crate) const GROUPED_FORMAT: u16 = 102;

const HEADER: usize = 0;
const DIAGNOSTICS: usize = 1;
const SECTIONS: usize = 2 + 2 * TABLE_COUNT;
const fn keyframe_section(table: TableId) -> usize {
    2 + 2 * table.index()
}
const fn later_section(table: TableId) -> usize {
    3 + 2 * table.index()
}

const CONTROL: usize = 0;
const KEYS: usize = 1;
const MASKS: usize = 2;
const FIELDS: usize = 3;

/// The leading masks-column byte of a chosen later section.
const MODELLED_ROWS: u8 = 0;
const LISTED_ROWS: u8 = 1;

const SNAPSHOT: u8 = 0;
const DELTA: u8 = 1;
const UNCHANGED: u8 = 2;
const TRUNCATED: u8 = 3;
const UNAVAILABLE: u8 = 4;
const ANCHOR_DELTA: u8 = 5;
const ANCHOR_UNCHANGED: u8 = 6;
/// A delta against the previous frame, stored against its predictions.
const RESIDUAL: u8 = 7;

/// Plain encodings of byte fields by row key and field: patch bases. A row
/// of a table with an [`advance_pair`] also keeps its head field's value and,
/// under [`DROPPED`], the element its list last dropped in the block, each as
/// four little-endian bytes.
type Shadow = BTreeMap<u128, BTreeMap<usize, Vec<u8>>>;

/// The [`Shadow`] slot of the element an [`advance_pair`] list last dropped.
const DROPPED: usize = usize::MAX;

/// A U32 field and a U32List field of the same row, where the field takes
/// the element the list dropped before the latest: DivisionMovement's source
/// (field 0) and remaining path (field 8). A moving division that reaches
/// the next province enters its path's head, which the path drops
/// ([`DROP_FIRST`]), and its source becomes the province it leaves, the
/// head the path dropped before (Divisions.province; in the 2d1ede901c 1943
/// burn-in 46.8 of 50.2 source changes an hour).
fn advance_pair(table: TableId) -> Option<(usize, usize)> {
    (table == TableId::DivisionMovement).then_some((0, 8))
}

/// The change that takes `key`'s head field to the element its list last
/// dropped in the block, as the row held them before this frame, in an
/// advancing block ([`Layout::advances`]); None for any other field, or
/// before the list has dropped an element.
fn advance_change(
    table: TableId,
    layout: Layout,
    bases: &Shadow,
    key: u128,
    field: usize,
) -> Option<i128> {
    let (head, _) = advance_pair(table).filter(|(head, _)| layout.advances() && *head == field)?;
    let fields = bases.get(&key)?;
    let word = |slot| {
        fields
            .get(&slot)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes.as_slice()).ok())
            .map(u32::from_le_bytes)
    };
    Some(i128::from(word(DROPPED)?) - i128::from(word(head)?))
}

/// Present tables of a format, in TableId order, and their positions.
struct Present {
    tables: Vec<TableId>,
    ordinal: [usize; TABLE_COUNT],
}

impl Present {
    fn new(format: u16) -> Self {
        let mut ordinal = [usize::MAX; TABLE_COUNT];
        let tables: Vec<TableId> = TableId::ALL
            .into_iter()
            .filter(|table| HISTORY[table.index()].present(format))
            .collect();
        for (position, table) in tables.iter().enumerate() {
            ordinal[table.index()] = position;
        }
        Self { tables, ordinal }
    }
    fn width(&self) -> usize {
        self.tables.len().div_ceil(8)
    }
    /// A plain frame's header bytes in this format.
    fn plain_header_bytes(&self) -> usize {
        37 + 8 * self.tables.len()
    }
}

fn has_byte_fields(table: TableId, fields: usize) -> bool {
    (0..fields).any(|field| table.byte_field(field))
}

fn zigzag(value: i128) -> u128 {
    ((value as u128) << 1) ^ ((value >> 127) as u128)
}

fn unzigzag(value: u128) -> i128 {
    ((value >> 1) as i128) ^ -((value & 1) as i128)
}

/// The baseline of a plain delta or unchanged update.
fn plain_baseline(plain: &[u8]) -> Option<Hour> {
    if !matches!(plain.first(), Some(&(DELTA | UNCHANGED))) {
        return None;
    }
    plain
        .get(1..5)
        .map(|bytes| Hour::from_le_bytes(bytes.try_into().unwrap()))
}

fn varint_usize(input: &mut Decoder<'_>) -> io::Result<usize> {
    usize::try_from(input.varint()?)
        .map_err(|_| invalid_static("grouped length exceeds address space"))
}

fn varint_u32(input: &mut Decoder<'_>) -> io::Result<u32> {
    u32::try_from(input.varint()?).map_err(|_| invalid_static("grouped count exceeds u32"))
}

fn hour_from(value: i128) -> io::Result<Hour> {
    Hour::try_from(value).map_err(|_| invalid_static("grouped hour outside its range"))
}

/// Advance the patch bases over one plain update; writer and reader both
/// follow exactly the plain updates, so they agree on every base.
fn advance_shadow(
    table: TableId,
    format: u16,
    plain: &[u8],
    shadow: &mut Shadow,
) -> io::Result<()> {
    let _cpu = crate::cpu::scope("decode_shadow", None);
    let status = plain.first().copied().unwrap_or(UNAVAILABLE);
    match status {
        UNCHANGED => return Ok(()),
        SNAPSHOT => shadow.clear(),
        DELTA => {}
        _ => {
            shadow.clear();
            return Ok(());
        }
    }
    if !has_byte_fields(table, table.last_change().fields(table, format)) {
        return Ok(());
    }
    let (head, list) = advance_pair(table).unzip();
    let mut key = 0;
    let mut counts = 0;
    table.walk_update(plain, format, &mut |token, bytes| match token {
        Token::Count => counts += 1,
        Token::Key(value) => {
            key = value;
            if counts == 2 {
                shadow.remove(&value);
            }
        }
        Token::Field(field) if Some(field) == head => {
            // A value the field cannot hold leaves no head, so writer and
            // reader alike store that row's next change without its advance.
            let fields = shadow.entry(key).or_default();
            match head_value(status, bytes, fields.get(&field)) {
                Ok(value) => fields.insert(field, value.to_le_bytes().to_vec()),
                Err(_) => fields.remove(&field),
            };
        }
        Token::Field(field) if table.byte_field(field) => {
            let fields = shadow.entry(key).or_default();
            // A delta's list that dropped its first element: keep the element.
            if Some(field) == list
                && status == DELTA
                && let Some(before) = fields.get(&field)
                && without_first(before, 4).as_deref() == Some(bytes)
            {
                let element = before[4..8].to_vec();
                fields.insert(DROPPED, element);
            }
            // The base before is replaced in its own storage when that is
            // not much larger, rather than freed and allocated again.
            match fields.get_mut(&field) {
                Some(base) if base.capacity() <= 2 * bytes.len().max(8) => {
                    base.clear();
                    base.extend_from_slice(bytes);
                }
                _ => {
                    fields.insert(field, bytes.to_vec());
                }
            }
        }
        _ => {}
    })
}

/// Start a block's advance state from its keyframe's rows, on writer and
/// reader alike: every row's head field value from `rows`, and no dropped
/// element, whichever entry stored the keyframe (a snapshot, or a change
/// against an anchor whose bases the two sides reach differently) and
/// whichever hour a seek starts from. Without complete rows (an unavailable
/// or truncated keyframe) no row has a head. Later entries then follow the
/// same plain updates on both sides ([`advance_shadow`]).
fn settle_advance(table: TableId, layout: Layout, shadow: &mut Shadow, rows: Option<&TableRows>) {
    let Some((head, _)) = advance_pair(table).filter(|_| layout.advances()) else {
        return;
    };
    for fields in shadow.values_mut() {
        fields.remove(&head);
        fields.remove(&DROPPED);
    }
    if let Some(TableRows::DivisionMovement(rows)) = rows {
        for row in rows {
            shadow
                .entry(u128::from(row.division))
                .or_default()
                .insert(head, row.source.to_le_bytes().to_vec());
        }
    }
}

/// An [`advance_pair`] head field's value after a plain update: a
/// snapshot's value, or a delta's difference added to the value before (0
/// for an inserted row).
fn head_value(status: u8, bytes: &[u8], before: Option<&Vec<u8>>) -> io::Result<u32> {
    if status == SNAPSHOT {
        return bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| invalid_static("advance head is not a u32"));
    }
    let before = before.map_or(Ok(0), |before| {
        before
            .as_slice()
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| invalid_static("advance head is not a u32"))
    })?;
    let mut input = Decoder::new(bytes);
    let Change::Number(difference) = Kind::U32.decode_change(&mut input)? else {
        return Err(invalid_static("advance head is not a number"));
    };
    input.finish()?;
    u32::try_from(i128::from(before) + difference)
        .map_err(|_| invalid_static("advance head leaves its native range"))
}

/// Section columns, and for a predicted block's later entries each byte
/// field column with its patches' in-place bytes as differences instead
/// ([`put_byte_field`]). A chosen later section also keeps listed keys and
/// masks in the second pair: only those columns differ between row codings.
struct Columns(Vec<Vec<u8>>, Vec<Option<Vec<u8>>>);

impl Columns {
    fn new(fields: usize) -> Self {
        Self(
            vec![Vec::new(); FIELDS + fields],
            vec![None; FIELDS + fields],
        )
    }
    /// Columns that also keep each byte field with difference patches.
    fn with_differences(table: TableId, fields: usize) -> Self {
        let mut columns = Self::new(fields);
        for field in (0..fields).filter(|field| table.byte_field(*field)) {
            columns.1[FIELDS + field] = Some(Vec::new());
        }
        columns
    }
    /// Append bytes that read the same either way.
    fn extend(&mut self, index: usize, bytes: &[u8]) {
        self.0[index].extend_from_slice(bytes);
        if let Some(differences) = &mut self.1[index] {
            differences.extend_from_slice(bytes);
        }
    }
    /// A deleted key has the same listed coding in both pairs.
    fn put_key(&mut self, writer: &mut KeyWriter<'_>, key: u128) -> io::Result<()> {
        let before = self.0[KEYS].len();
        writer.put(key, &mut self.0[KEYS])?;
        if let Some(listed) = &mut self.1[KEYS] {
            listed.extend_from_slice(&self.0[KEYS][before..]);
        }
        Ok(())
    }
    /// Keep each byte field column's smaller form, compressed alone.
    fn choose(&mut self) -> io::Result<()> {
        for (column, differences) in self.0.iter_mut().zip(&mut self.1).skip(FIELDS) {
            if let Some(differences) = differences.take()
                && differences != *column
                && compress(&differences)?.len() < compress(column)?.len()
            {
                *column = differences;
            }
        }
        Ok(())
    }
    fn marks(&mut self, table: TableId, marks: &[u8]) -> io::Result<()> {
        if super::aliases::private::stored(table, FormatVersion::CURRENT.version()) {
            self.0.push(engine_allocation::copy_vec(marks)?);
            self.1.push(None);
        }
        Ok(())
    }
    fn raw(&self) -> Vec<u8> {
        let mut raw =
            Vec::with_capacity(self.0.iter().map(Vec::len).sum::<usize>() + 4 * self.0.len());
        put_varint(&mut raw, self.0.len() as u128);
        for column in &self.0 {
            put_varint(&mut raw, column.len() as u128);
        }
        for column in &self.0 {
            raw.extend_from_slice(column);
        }
        raw
    }
}

struct ColumnReader<'a>(Vec<Decoder<'a>>, Option<&'a [u8]>);

impl<'a> ColumnReader<'a> {
    fn open(raw: &'a [u8], fields: usize) -> io::Result<Self> {
        let mut input = Decoder::new(raw);
        if varint_usize(&mut input)? != FIELDS + fields {
            return Err(invalid_static("grouped section has the wrong column count"));
        }
        let mut lengths = Vec::with_capacity(FIELDS + fields);
        for _ in 0..FIELDS + fields {
            lengths.push(varint_usize(&mut input)?);
        }
        let mut columns = Vec::with_capacity(FIELDS + fields);
        for length in lengths {
            columns.push(Decoder::new(input.take(length)?));
        }
        input.finish()?;
        Ok(Self(columns, None))
    }
    fn stored(raw: &'a [u8], fields: usize, table: TableId, version: u16) -> io::Result<Self> {
        let marked = super::aliases::private::stored(table, version);
        let mut columns = Self::open(raw, fields + usize::from(marked))?;
        if marked {
            columns.1 = Some(columns.0.pop().unwrap().rest());
        }
        Ok(columns)
    }
    fn empty(fields: usize) -> Self {
        Self(
            (0..FIELDS + fields).map(|_| Decoder::new(&[])).collect(),
            None,
        )
    }
    /// Consume a chosen later section's row-coding byte before its rows.
    fn modelled_rows(&mut self, table: TableId, layout: Layout) -> io::Result<bool> {
        if !layout.chooses_rows() {
            return Ok(layout.modelled());
        }
        let masks = &mut self.0[MASKS];
        if masks.remaining() == 0 {
            return Err(invalid(format!(
                "missing grouped row coding byte for {table:?}"
            )));
        }
        match masks.u8()? {
            MODELLED_ROWS => Ok(true),
            LISTED_ROWS => Ok(false),
            code => Err(invalid(format!(
                "unknown grouped row coding byte {code} for {table:?}"
            ))),
        }
    }
    fn finish(self) -> io::Result<()> {
        if self.0.iter().all(|column| column.remaining() == 0) {
            Ok(())
        } else {
            Err(invalid_static("grouped section has unread column bytes"))
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    /// Snapshots, truncations and unavailability, which have no baseline.
    Plain,
    /// A later entry: baselines are told relative to the previous frame.
    Later { previous: Hour },
    /// A keyframe change against the keyframe of the branch's block that
    /// starts at the baseline hour, told as hours before `block`'s.
    Anchor { block: Hour },
    /// A keyframe change against the keyframe of the block `blocks` before
    /// this one in the file, of any branch.
    Back { blocks: usize },
}

/// Append one plain update as an entry. With `coding`, a later delta stores
/// its row keys and masks against the table's rows and the stream's masks
/// before it ([`keyed`]), or as its stream's model decisions ([`modelled`]).
fn put_entry(
    table: TableId,
    format: u16,
    plain: &[u8],
    mode: Mode,
    bases: &Shadow,
    out: &mut Columns,
    coding: Option<RowCoding<'_>>,
) -> io::Result<()> {
    let status = plain[0];
    let mut coding = coding.filter(|_| status == DELTA && matches!(mode, Mode::Later { .. }));
    let width = table.last_change().fields(table, format).div_ceil(8);
    let mut run: Option<RowRun<'_>> = None;
    // The changed rows' run, then the deleted keys'.
    let mut runs = 0;
    let entry = match (status, mode) {
        (DELTA, Mode::Anchor { .. } | Mode::Back { .. }) => ANCHOR_DELTA,
        (UNCHANGED, Mode::Anchor { .. } | Mode::Back { .. }) => ANCHOR_UNCHANGED,
        (DELTA | UNCHANGED, Mode::Plain) => {
            return Err(invalid_static(
                "grouped keyframe entry depends on a baseline",
            ));
        }
        (status, _) => status,
    };
    put_varint(&mut out.0[CONTROL], u128::from(entry));
    let mut key = 0;
    let mut failure = None;
    table.walk_update(plain, format, &mut |token, bytes| {
        if failure.is_some() {
            return;
        }
        match token {
            Token::Status => {}
            Token::Baseline => {
                let baseline = i32::from_le_bytes(bytes.try_into().unwrap());
                let control = &mut out.0[CONTROL];
                match mode {
                    Mode::Later { previous } => {
                        put_varint(control, zigzag(i128::from(previous) - i128::from(baseline)));
                    }
                    Mode::Anchor { block } => {
                        put_varint(control, (i128::from(block) - i128::from(baseline)) as u128);
                    }
                    // The anchor's block, not its hour, names it.
                    Mode::Back { blocks } => put_varint(control, blocks as u128),
                    Mode::Plain => unreachable!("plain entries have no baseline"),
                }
            }
            Token::Count | Token::Missing => {
                put_varint(
                    &mut out.0[CONTROL],
                    u128::from(u32::from_le_bytes(bytes.try_into().unwrap())),
                );
                // Changed rows and deleted keys are each a run of keys; the
                // changed rows' ends where the deleted keys' starts.
                if token == Token::Count {
                    runs += 1;
                    if let (Some(coding), Some(changed)) = (&mut coding, run.take())
                        && let Err(error) = coding.finish_run(changed, out)
                    {
                        failure = Some(error);
                        return;
                    }
                    run = coding.as_ref().map(|coding| {
                        if runs == 1 {
                            coding.run()
                        } else {
                            RowRun::Listed(KeyWriter::new(coding.keys))
                        }
                    });
                }
            }
            Token::Reason => {
                let control = &mut out.0[CONTROL];
                put_varint(control, (bytes.len() - 4) as u128);
                control.extend_from_slice(&bytes[4..]);
            }
            Token::Key(value) => {
                key = value;
                match &mut run {
                    Some(RowRun::Listed(deleted)) if runs == 2 => {
                        if let Err(error) = out.put_key(deleted, value) {
                            failure = Some(error);
                        }
                    }
                    // A changed row is stored with its mask.
                    Some(_) => {}
                    None => out.extend(KEYS, bytes),
                }
            }
            Token::Mask => match (&mut coding, &mut run) {
                (Some(coding), Some(changed)) => {
                    let mut mask = [0; 8];
                    mask[..width].copy_from_slice(bytes);
                    let mask = u64::from_le_bytes(mask);
                    if let Err(error) = coding.put_row(changed, key, mask, width, out) {
                        failure = Some(error);
                    }
                }
                _ => out.extend(MASKS, bytes),
            },
            Token::Field(field) => {
                if status != DELTA || !table.byte_field(field) {
                    out.extend(FIELDS + field, bytes);
                    return;
                }
                let base = bases
                    .get(&key)
                    .and_then(|fields| fields.get(&field))
                    .map(Vec::as_slice);
                if let Err(error) =
                    put_byte_field(Some(bytes), base, None, out, FIELDS + field, None)
                {
                    failure = Some(error);
                }
            }
        }
    })?;
    failure.map_or(Ok(()), Err)
}

/// A stored byte field's tags: its plain encoding, a patch against its
/// previous encoding in the block (the patch base) or, in residual entries,
/// against its predicted encoding, or that a predicted field did not change.
/// In a predicted block either patch may store byte differences instead.
const PLAIN: u8 = 0;
const PATCH: u8 = 1;
const PATCH_PREDICTION: u8 = 2;
const STOPPED: u8 = 3;
/// A tag plus this is the same patch with byte differences.
const DIFFERENCES: u8 = 3;
/// In a trimmed block ([`Layout::trims`]), a list field (`U32List`,
/// `U64List`) that is its patch base without its first element: one tag, in
/// either form of the column. A moving division's remaining path
/// (DivisionMovement.path) loses its head each time the division reaches
/// the next province.
const DROP_FIRST: u8 = 7;

/// The element width of `table`'s list field `field`, whose plain encoding
/// is a u32 count then the elements; None for any other kind.
fn list_width(table: TableId, field: usize) -> Option<usize> {
    match table.schema().get(field)? {
        Kind::U32List => Some(4),
        Kind::U64List => Some(8),
        _ => None,
    }
}

/// `base`, a list field's plain encoding with elements `width` bytes wide,
/// without its first element; None when it has none.
fn without_first(base: &[u8], width: usize) -> Option<Vec<u8>> {
    let count = u32::from_le_bytes(base.get(..4)?.try_into().ok()?).checked_sub(1)?;
    let rest = base.get(4 + width..)?;
    let mut out = Vec::with_capacity(4 + rest.len());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(rest);
    Some(out)
}

/// Append a changed byte field to column `index`, or a predicted one that
/// did not move as predicted: its plain encoding or a patch against its
/// previous or predicted encoding, whichever is smallest, or that it
/// stopped. Columns that keep difference patches get the same patch with its
/// in-place bytes as their differences from the bytes they replace: a nested
/// counter that moves by a little is then a few small numbers. With `trim`,
/// a list field's element width in a trimmed block, a value that is its base
/// without its first element is [`DROP_FIRST`].
fn put_byte_field(
    value: Option<&[u8]>,
    base: Option<&[u8]>,
    prediction: Option<&[u8]>,
    out: &mut Columns,
    index: usize,
    trim: Option<usize>,
) -> io::Result<()> {
    let Some(value) = value else {
        out.extend(index, &[STOPPED]);
        return Ok(());
    };
    if let (Some(width), Some(base)) = (trim, base)
        && without_first(base, width).as_deref() == Some(value)
    {
        out.extend(index, &[DROP_FIRST]);
        return Ok(());
    }
    let mut best = (PLAIN, Vec::new(), None);
    for (tag, candidate) in [(PATCH, base), (PATCH_PREDICTION, prediction)] {
        if let Some(candidate) = candidate.filter(|candidate| *candidate != value) {
            let mut changed = Vec::new();
            patch::diff_with(candidate, value, &mut changed, false)?;
            let smallest = if best.0 == PLAIN {
                value.len()
            } else {
                best.1.len()
            };
            if changed.len() < smallest {
                best = (tag, changed, Some(candidate));
            }
        }
    }
    let (tag, changed, candidate) = best;
    let Some(candidate) = candidate else {
        out.extend(index, &[PLAIN]);
        out.extend(index, value);
        return Ok(());
    };
    out.0[index].push(tag);
    out.0[index].extend_from_slice(&changed);
    if let Some(differences) = &mut out.1[index] {
        differences.push(tag + DIFFERENCES);
        patch::diff_with(candidate, value, differences, true)?;
    }
    Ok(())
}

/// Read a stored byte field ([`put_byte_field`]), appending its plain
/// encoding to `values`; false when it stopped. `used` is the entry's plain
/// bytes so far.
#[allow(clippy::too_many_arguments)]
fn take_byte_field(
    table: TableId,
    field: usize,
    column: &mut Decoder<'_>,
    base: Option<&[u8]>,
    prediction: Option<&[u8]>,
    values: &mut Vec<u8>,
    used: usize,
    budget: &Budget,
    layout: Layout,
) -> io::Result<bool> {
    let at = values.len();
    let tag = column.u8()?;
    if tag == DROP_FIRST && layout.trims() {
        let width = list_width(table, field)
            .ok_or_else(|| invalid_static("grouped first-element drop of a non-list field"))?;
        let base = base.ok_or_else(|| invalid_static("grouped first-element drop has no base"))?;
        // The rebuilt field is the base less its head, budgeted before it
        // is allocated.
        let length = base
            .len()
            .checked_sub(width)
            .filter(|length| *length >= 4 && base[..4] != [0; 4])
            .ok_or_else(|| invalid_static("grouped first-element drop of an empty list"))?;
        budget.check(used + length)?;
        let value = without_first(base, width)
            .ok_or_else(|| invalid_static("grouped first-element drop of an empty list"))?;
        if prediction == Some(value.as_slice()) {
            return Err(invalid_static("noncanonical predicted field correction"));
        }
        values.extend_from_slice(&value);
        return Ok(true);
    }
    let differences = tag > STOPPED && layout.predicts();
    match if differences { tag - DIFFERENCES } else { tag } {
        PLAIN if !differences => values.extend_from_slice(table.field_span(field, column)?),
        tag @ (PATCH | PATCH_PREDICTION) => {
            let base = if tag == PATCH { base } else { prediction }
                .ok_or_else(|| invalid_static("grouped patch has no base"))?;
            budget.check(used + base.len() + column.remaining())?;
            patch::apply_with(base, column, values, differences)?;
            // The patched value is one complete plain field encoding.
            let mut value = Decoder::new(&values[at..]);
            table.field_span(field, &mut value)?;
            value.finish()?;
        }
        STOPPED if !differences && prediction.is_some() => return Ok(false),
        _ => return Err(invalid_static("invalid grouped field tag")),
    }
    if prediction == Some(&values[at..]) {
        return Err(invalid_static("noncanonical predicted field correction"));
    }
    Ok(true)
}

/// A table's remaining budget of plain bytes, or the frame headers': a block
/// never decodes either to more than the bound its writer kept.
struct Budget(AtomicUsize);

impl Budget {
    fn check(&self, bytes: usize) -> io::Result<()> {
        if bytes > self.0.load(Ordering::Relaxed) {
            return Err(invalid_static("block exceeds raw size limit"));
        }
        Ok(())
    }
    fn spend(&self, bytes: usize) -> io::Result<()> {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(bytes)
            })
            .map(|_| ())
            .map_err(|_| invalid_static("block exceeds raw size limit"))
    }
}

/// The earlier keyframe an anchored keyframe entry changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnchorRef {
    /// Earlier layouts: the keyframe of the same branch's block starting at
    /// this hour.
    Hour(Hour),
    /// The keyframe of the block this many blocks earlier in the file.
    Back(usize),
}

struct Head {
    status: u8,
    anchor: Option<AnchorRef>,
    /// A residual entry, whose plain update [`predicted::take_residual`] writes.
    residual: bool,
}

/// Read an entry's status and baseline, writing the plain status and
/// baseline. Keyframe entries cannot depend on the previous frame, and in a
/// predicted block a delta against the previous frame is a residual entry.
fn take_head(
    input: &mut ColumnReader<'_>,
    previous: Option<Hour>,
    block: Hour,
    plain: &mut Vec<u8>,
    layout: Layout,
) -> io::Result<Head> {
    let control = &mut input.0[CONTROL];
    let entry =
        u8::try_from(control.varint()?).map_err(|_| invalid_static("unknown grouped entry"))?;
    let allowed = match previous {
        None => matches!(
            entry,
            SNAPSHOT | UNAVAILABLE | ANCHOR_DELTA | ANCHOR_UNCHANGED
        ),
        Some(_) => {
            matches!(
                entry,
                SNAPSHOT | DELTA | UNCHANGED | TRUNCATED | UNAVAILABLE
            ) || (entry == RESIDUAL && layout.predicts())
        }
    };
    if !allowed {
        return Err(invalid_static(
            "grouped entry status is invalid in its section",
        ));
    }
    if entry == RESIDUAL {
        return Ok(Head {
            status: DELTA,
            anchor: None,
            residual: true,
        });
    }
    let status = match entry {
        ANCHOR_DELTA => DELTA,
        ANCHOR_UNCHANGED => UNCHANGED,
        status => status,
    };
    plain.push(status);
    let mut anchor = None;
    if let Some(previous) = previous.filter(|_| matches!(entry, DELTA | UNCHANGED)) {
        let baseline = hour_from(i128::from(previous) - unzigzag(control.varint()?))?;
        plain.extend_from_slice(&baseline.to_le_bytes());
    } else if matches!(entry, ANCHOR_DELTA | ANCHOR_UNCHANGED) {
        let offset = control.varint()?;
        if offset == 0 {
            return Err(invalid_static("keyframe anchor is not an earlier block"));
        }
        if layout.anchors_back() {
            // An anchored change means the rows it changes, whatever their
            // hour; the plain baseline is not read.
            plain.extend_from_slice(&block.to_le_bytes());
            anchor = Some(AnchorRef::Back(usize::try_from(offset).map_err(|_| {
                invalid_static("keyframe anchor is not an earlier block")
            })?));
        } else {
            let hour = hour_from(i128::from(block) - i128::try_from(offset).unwrap_or(i128::MAX))?;
            plain.extend_from_slice(&hour.to_le_bytes());
            anchor = Some(AnchorRef::Hour(hour));
        }
    }
    Ok(Head {
        status,
        anchor,
        residual: false,
    })
}

/// Read an entry's body after its head, writing the plain update body. With
/// `coding`, a delta's row keys and masks are stored against the table's
/// rows and the stream's masks before it ([`keyed`]), or as its stream's
/// model decisions ([`modelled`]).
#[allow(clippy::too_many_arguments)]
fn take_body(
    table: TableId,
    fields: usize,
    status: u8,
    input: &mut ColumnReader<'_>,
    bases: &Shadow,
    plain: &mut Vec<u8>,
    budget: &Budget,
    layout: Layout,
    coding: Option<RowCoding<'_>>,
) -> io::Result<()> {
    let start = plain.len();
    let width = fields.div_ceil(8);
    let count = |input: &mut ColumnReader<'_>, plain: &mut Vec<u8>| -> io::Result<u32> {
        let count = varint_u32(&mut input.0[CONTROL])?;
        // Every row has a key byte, so a count is bounded by its keys column.
        if count as usize > input.0[KEYS].remaining() {
            return Err(invalid_static("grouped row count exceeds its keys"));
        }
        plain.extend_from_slice(&count.to_le_bytes());
        Ok(count)
    };
    let key = |input: &mut ColumnReader<'_>, plain: &mut Vec<u8>| -> io::Result<u128> {
        let keys = &mut input.0[KEYS];
        let before = keys.rest();
        let difference = keys.varint()?;
        plain.extend_from_slice(&before[..before.len() - keys.remaining()]);
        Ok(difference)
    };
    match status {
        SNAPSHOT | TRUNCATED => {
            if status == TRUNCATED {
                plain.extend_from_slice(&varint_u32(&mut input.0[CONTROL])?.to_le_bytes());
            }
            for _ in 0..count(input, plain)? {
                key(input, plain)?;
                for field in 0..fields {
                    plain.extend_from_slice(table.field_span(field, &mut input.0[FIELDS + field])?);
                }
                budget.check(plain.len() - start)?;
            }
        }
        DELTA => {
            let mut coding = coding;
            let mut row = 0_u128;
            let changed = match &coding {
                // A modelled row can cost less than a key byte, so the rows
                // held at the frame before count too.
                Some(coding) if coding.modelled() => {
                    let changed = varint_u32(&mut input.0[CONTROL])?;
                    if changed as usize
                        > input.0[KEYS].remaining().saturating_add(coding.held_rows())
                    {
                        return Err(invalid_static("grouped row count exceeds its rows"));
                    }
                    plain.extend_from_slice(&changed.to_le_bytes());
                    changed
                }
                _ => count(input, plain)?,
            };
            let mut run = match &mut coding {
                Some(coding) => Some(coding.read(table, changed as usize, &mut input.0)?),
                None => None,
            };
            for _ in 0..changed {
                let mask = match (&mut coding, &mut run) {
                    (Some(coding), Some(run)) => {
                        let (key, mask) = coding.take_row(run, table, width, &mut input.0)?;
                        put_varint(plain, key - row);
                        row = key;
                        plain.extend_from_slice(&mask.to_le_bytes()[..width]);
                        mask
                    }
                    _ => {
                        row = row
                            .checked_add(key(input, plain)?)
                            .ok_or_else(|| invalid_static("row key addition overflow"))?;
                        let bytes = input.0[MASKS].take(width)?;
                        plain.extend_from_slice(bytes);
                        let mut mask = [0; 8];
                        mask[..width].copy_from_slice(bytes);
                        u64::from_le_bytes(mask)
                    }
                };
                if mask == 0 || (fields < 64 && mask >> fields != 0) {
                    return Err(invalid_static("invalid recorded delta mask"));
                }
                for field in (0..fields).filter(|field| mask & (1 << field) != 0) {
                    let column = &mut input.0[FIELDS + field];
                    if !table.byte_field(field) {
                        plain.extend_from_slice(table.change_span(field, column)?);
                        continue;
                    }
                    let used = plain.len() - start;
                    let base = bases
                        .get(&row)
                        .and_then(|fields| fields.get(&field))
                        .map(Vec::as_slice);
                    take_byte_field(
                        table, field, column, base, None, plain, used, budget, layout,
                    )?;
                }
                budget.check(plain.len() - start)?;
            }
            let mut deleted = coding.as_ref().map(|coding| KeyReader::new(coding.keys));
            let mut previous = 0_u128;
            for _ in 0..count(input, plain)? {
                match (&mut deleted, &mut coding) {
                    (Some(run), Some(coding)) => {
                        let (key, _) = run.take(table, &mut input.0[KEYS])?;
                        coding.note_deleted(key);
                        put_varint(plain, key - previous);
                        previous = key;
                    }
                    _ => {
                        key(input, plain)?;
                    }
                }
            }
        }
        UNCHANGED => {}
        UNAVAILABLE => {
            let control = &mut input.0[CONTROL];
            let length = varint_usize(control)?;
            let reason = control.take(length)?;
            plain.extend_from_slice(
                &u32::try_from(length)
                    .map_err(|_| invalid_static("reason exceeds u32"))?
                    .to_le_bytes(),
            );
            plain.extend_from_slice(reason);
        }
        _ => return Err(invalid_static("unknown grouped entry")),
    }
    budget.spend(plain.len() - start)
}

/// Each table's keyframe anchor candidates in the file being written: its
/// latest keyframe stored as a snapshot, and its latest keyframe. Earlier
/// layouts keep them per branch.
pub(crate) struct Anchors {
    /// The branch earlier layouts anchor within.
    branch: Option<BranchId>,
    /// The position in the file's block index of the next block.
    position: usize,
    snapshots: Vec<Option<Anchor>>,
    latest: Vec<Option<Anchor>>,
}

#[derive(Clone)]
struct Anchor {
    hour: Hour,
    /// Its block's position in the file's block index.
    block: usize,
    rows: SharedView,
    shadow: Shadow,
    /// Earlier layouts: anchors between this keyframe and its snapshot, 0
    /// for a snapshot.
    depth: usize,
    /// The raw bytes of the snapshot it reaches (its own for a snapshot).
    root: usize,
    /// What reading it costs beyond its snapshot ([`chain_charge`]): 0 for
    /// a snapshot.
    charge: usize,
}

/// A predicted block's keyframe reaches its snapshot through at most this
/// many anchors, so a seek decodes at most this many other blocks'
/// keyframe sections per table. Earlier layouts anchor only to snapshots.
const MAX_ANCHOR_DEPTH: usize = 4;

/// Each anchored keyframe a reader rebuilds a table's rows for counts this
/// share of its snapshot's raw bytes besides its own ([`chain_charge`]).
const REBUILD_SHARE: usize = 16;

/// What reading an anchored keyframe costs beyond its snapshot: its raw
/// section bytes and a [`REBUILD_SHARE`] of the snapshot's `root` raw bytes,
/// added to its anchor's charge. Keyframes of the layouts that anchor back
/// ([`Layout::anchors_back`]) keep it at most `root`, so a seek inflates at
/// most twice a table's snapshot section and rebuilds its rows at most
/// [`REBUILD_SHARE`] times, however many blocks back the snapshot is.
fn chain_charge(anchor: usize, raw: usize, root: usize) -> Option<usize> {
    anchor
        .checked_add(raw)?
        .checked_add(root / REBUILD_SHARE)
        .filter(|charge| *charge <= root)
}

impl Default for Anchors {
    fn default() -> Self {
        Self::at(0)
    }
}

impl Anchors {
    /// No candidates yet; the next block is at `position` in the file.
    pub(crate) fn at(position: usize) -> Self {
        Self {
            branch: None,
            position,
            snapshots: (0..TABLE_COUNT).map(|_| None).collect(),
            latest: (0..TABLE_COUNT).map(|_| None).collect(),
        }
    }

    /// The candidates of blocks appended to `replay`: each table's keyframe
    /// in its last block and the snapshot that keyframe reaches, so
    /// appended sessions anchor to the file's earlier ones. A table whose
    /// keyframe this reader cannot rebuild has none; only allocation failure
    /// is an error.
    pub(crate) fn seed(replay: &Replay) -> io::Result<Self> {
        let mut anchors = Self::at(replay.index.len());
        let Some(index) = replay.index.last() else {
            return Ok(anchors);
        };
        let Ok(block) = replay.block_bytes(index) else {
            return Ok(anchors);
        };
        let Some(Ok(directory)) = is_grouped(&block).then(|| Directory::parse(&block, index))
        else {
            return Ok(anchors);
        };
        let from = From {
            position: Some(replay.index.len() - 1),
            index,
            layout: directory.layout,
        };
        for table in TableId::ALL {
            match seed_table(replay, &block, &directory, &from, table) {
                Ok(Some((snapshot, latest))) => {
                    anchors.snapshots[table.index()] = snapshot;
                    anchors.latest[table.index()] = Some(latest);
                }
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::OutOfMemory => return Err(error),
                Err(_) => {}
            }
        }
        Ok(anchors)
    }
}

/// A table's keyframe in the block `from` names and the snapshot it
/// reaches, as anchor candidates: none for a table without a keyframe.
fn seed_table(
    replay: &Replay,
    block: &[u8],
    directory: &Directory,
    from: &From<'_>,
    table: TableId,
) -> io::Result<Option<(Option<Anchor>, Anchor)>> {
    let format = replay.format;
    let version = format.version();
    let change: &LastChange = &HISTORY[table.index()];
    if !change.present(version) || change.table_gap(table, version).is_some() {
        return Ok(None);
    }
    let fields = change.fields(table, version);
    let Some(raw) = directory.raw(block, keyframe_section(table))? else {
        return Ok(None);
    };
    let mut input = ColumnReader::stored(&raw, fields, table, version)?;
    let mut plain = Vec::new();
    let head = take_head(
        &mut input,
        None,
        from.index.first_hour,
        &mut plain,
        from.layout,
    )?;
    let resolved = match head.anchor {
        Some(anchor) => Some(anchored_keyframe(
            replay,
            from,
            table,
            fields,
            false,
            anchor,
            raw.len(),
            format,
            true,
        )?),
        None if head.status == SNAPSHOT => None,
        // An unavailable table has no keyframe to anchor to.
        None => return Ok(None),
    };
    let (base, shadow, root) = match resolved {
        Some(Resolved {
            state,
            root: (at, root, Some(snapshot)),
            charge,
        }) => (
            Some((state.rows, charge)),
            state.shadow,
            Some((at, root, snapshot)),
        ),
        Some(_) => return Ok(None),
        None => (None, Shadow::new(), None),
    };
    take_body(
        table,
        fields,
        head.status,
        &mut input,
        &shadow,
        &mut plain,
        &Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES)),
        from.layout,
        None,
    )?;
    input.finish()?;
    let mut shadow = shadow;
    advance_shadow(table, version, &plain, &mut shadow)?;
    let mut decoder = Decoder::new(&plain);
    let update = decode_update(table, change, format, base.is_none(), &mut decoder)?;
    decoder.finish()?;
    let position = from.position.expect("the file's last block");
    let keyframe = |rows: TableRows, shadow: Shadow, block: usize, depth, root, charge| Anchor {
        hour: replay.index[block].first_hour,
        block,
        rows: TableView::Complete(rows).into(),
        shadow,
        depth,
        root,
        charge,
    };
    Ok(match (base, update, root) {
        (None, TableUpdate::Snapshot(rows), None) => {
            let snapshot = keyframe(rows, shadow, position, 0, raw.len(), 0);
            Some((Some(snapshot.clone()), snapshot))
        }
        (Some((Some(base), charge)), update, Some((at, root, snapshot))) => {
            let rows = apply_anchor_change(base, update)?;
            let charge = charge
                .saturating_add(raw.len())
                .saturating_add(root / REBUILD_SHARE);
            let snapshot = snapshot
                .rows
                .map(|rows| keyframe(rows, snapshot.shadow, at, 0, root, 0));
            Some((snapshot, keyframe(rows, shadow, position, 1, root, charge)))
        }
        _ => None,
    })
}

/// One way of writing a table's later entries, and which frames have one.
struct Later {
    columns: Columns,
    entries: usize,
    /// By frame, the keyframe first.
    present: Vec<bool>,
    /// The listed coding's recent masks ([`keyed`]), including the second
    /// keys/masks pair in a chosen block.
    masks: MaskList,
    /// In a modelled block, its entries' rows and masks ([`modelled`]).
    model: Option<Model>,
}

impl Later {
    fn new(table: TableId, fields: usize, layout: Layout) -> Self {
        let mut columns = if layout.predicts() {
            Columns::with_differences(table, fields)
        } else {
            Columns::new(fields)
        };
        if super::aliases::private::stored(table, FormatVersion::CURRENT.version()) {
            columns.0.push(Vec::new());
            columns.1.push(None);
        }
        if layout.chooses_rows() {
            columns.0[MASKS].push(MODELLED_ROWS);
            columns.1[KEYS] = Some(Vec::new());
            columns.1[MASKS] = Some(vec![LISTED_ROWS]);
        }
        Self {
            columns,
            entries: 0,
            present: vec![true],
            masks: MaskList::default(),
            model: layout.modelled().then(|| Model::new(fields)),
        }
    }
    fn raw_bytes(&self) -> usize {
        let bytes = self
            .columns
            .0
            .iter()
            .map(|column| column.len() + 5)
            .sum::<usize>()
            + 5;
        let modelled = bytes + self.model.as_ref().map_or(0, Model::waiting);
        match (&self.columns.1[KEYS], &self.columns.1[MASKS]) {
            (Some(keys), Some(masks)) => modelled.max(
                bytes - self.columns.0[KEYS].len() - self.columns.0[MASKS].len()
                    + keys.len()
                    + masks.len(),
            ),
            _ => modelled,
        }
    }
    /// The masks column gets the bytes its model still owes it.
    fn finish(&mut self) {
        if let Some(model) = &mut self.model {
            model.finish(&mut self.columns.0[MASKS]);
        }
    }
    /// Choose field variants once, then compare the two whole row codings
    /// at LEVEL before the table chooses residual or plain entries.
    fn compress(&mut self) -> io::Result<(usize, Option<Vec<u8>>)> {
        self.finish();
        self.columns.choose()?;
        let mut best = {
            let raw = self.columns.raw();
            (raw.len(), compress_later(&raw, self.entries)?)
        };
        if let (Some(keys), Some(masks)) =
            (self.columns.1[KEYS].take(), self.columns.1[MASKS].take())
        {
            self.columns.0[KEYS] = keys;
            self.columns.0[MASKS] = masks;
            let raw = self.columns.raw();
            let compressed = compress_later(&raw, self.entries)?;
            // Modelled wins ties, independently of worker scheduling.
            if compressed.as_ref().map_or(0, Vec::len) < best.1.as_ref().map_or(0, Vec::len) {
                best = (raw.len(), compressed);
            }
        }
        Ok(best)
    }
}

struct TableWriter {
    table: TableId,
    fields: usize,
    keyframe: Vec<u8>,
    keyframe_raw: usize,
    /// Plain entries, and in a predicted block residual entries: the block
    /// keeps whichever compresses smaller. Residual entries leave out what
    /// is predicted, but a changing subset of rows can cost more keys than
    /// every row of a regular delta.
    plain: Later,
    predicted: Option<Later>,
    shadow: Shadow,
    predictor: predicted::Predictor,
    /// The table's row keys at the frame before, which a keyed block's later
    /// entries name rows among ([`keyed`]).
    keys: KeySet,
}

/// One grouped block being written.
pub(crate) struct BlockWriter {
    pub(crate) branch: BranchId,
    pub(crate) first_hour: Hour,
    pub(crate) last_hour: Hour,
    pub(crate) frames: u32,
    /// Plain frame headers as an interleaved block would frame them: the
    /// frame count, then each frame's length and header.
    headers: usize,
    /// Each table's plain update bytes, by `TableId::index`.
    plain: Vec<usize>,
    /// The previous frame's plain bytes.
    last_payload: usize,
    present: Present,
    header: Vec<u8>,
    diagnostics: Vec<u8>,
    reads: Vec<u8>,
    pool: u64,
    dropped: u64,
    tables: Vec<TableWriter>,
    /// Each later frame's hours after the frame before it, less one.
    gaps: Vec<u128>,
    /// Anchored; tests also write the earlier layouts.
    layout: Layout,
    /// Tests keep every predicting table's residual entries, whichever is
    /// smaller ([`FORCE_RESIDUAL`]).
    force_residual: bool,
}

/// The zstd level the writer chooses between encodings with: level 9 stores
/// about 7% fewer bytes than level 3 for a few milliseconds of finalization
/// per recorded hour.
const LEVEL: i32 = 9;

/// The zstd level every chosen section is stored at ([`store_sections`]):
/// level 19 stores about 11% fewer bytes than level 9 (the 1936 and 1943
/// in-run recordings, `recorder sizes --levels`) for about 25 ms of
/// single-threaded finalization per recorded hour of a 1943 game; level 22
/// saves another 0.2% at four times the time. Readers inflate it as fast and
/// its window (at most 2^23) stays within their `window_log_max`.
pub const STORED_LEVEL: i32 = 19;

pub(crate) fn compress(raw: &[u8]) -> io::Result<Vec<u8>> {
    compress_at(raw, LEVEL)
}

/// The window readers accept (`window_log_max`), which every level is held
/// to.
const WINDOW_LOG: u32 = 26;

/// `raw` as one checksummed zstd frame at `level`, its window within
/// [`WINDOW_LOG`]. The compressor knows the size, so it sizes its tables to
/// it, but the frame does not record it: the directory does.
pub(crate) fn compress_at(raw: &[u8], level: i32) -> io::Result<Vec<u8>> {
    with_compressor(level, |compressor| compressor.compress(raw))
}

/// Runs `compress` with this thread's compressor for `level`. Every frame
/// starts a new session with the same parameters, so a reused context writes
/// the same bytes as a fresh one; a failed call discards its context.
fn with_compressor<T>(
    level: i32,
    compress: impl FnOnce(&mut zstd::bulk::Compressor<'static>) -> io::Result<T>,
) -> io::Result<T> {
    thread_local! {
        static COMPRESSORS: std::cell::RefCell<BTreeMap<i32, zstd::bulk::Compressor<'static>>> =
            const { std::cell::RefCell::new(BTreeMap::new()) };
    }
    COMPRESSORS.with(|compressors| {
        let mut compressors = compressors.borrow_mut();
        let compressor = match compressors.entry(level) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(compressor(level)?),
        };
        let result = compress(compressor);
        if result.is_err() {
            compressors.remove(&level);
        }
        result
    })
}

fn compressor(level: i32) -> io::Result<zstd::bulk::Compressor<'static>> {
    let mut compressor = zstd::bulk::Compressor::new(level)?;
    compressor.set_parameter(zstd::zstd_safe::CParameter::ChecksumFlag(true))?;
    compressor.set_parameter(zstd::zstd_safe::CParameter::ContentSizeFlag(false))?;
    if level > 19 {
        compressor.set_parameter(zstd::zstd_safe::CParameter::WindowLog(WINDOW_LOG))?;
    }
    Ok(compressor)
}

/// A fresh context for `level`, and a reused one interrupted mid-frame by an
/// output buffer that cannot hold the frame.
#[cfg(test)]
pub(crate) fn fresh_and_failed_reuse(raw: &[u8], level: i32) -> io::Result<(Vec<u8>, bool)> {
    let fresh = compressor(level)?.compress(raw)?;
    let failed = with_compressor(level, |compressor| {
        let mut short = vec![0; 1];
        compressor.compress_to_buffer(raw, &mut short)
    })
    .is_err();
    Ok((fresh, failed))
}

/// Each chosen section (its id, plain length and [`compress`]ed bytes)
/// compressed again at [`STORED_LEVEL`] when that is smaller, the largest
/// first, on this thread and the threads `cores` has free; each holds one
/// level-19 compression context. Each section's bytes depend only on it.
fn store_sections(sections: &mut [(usize, usize, Vec<u8>)], cores: &Cores) -> io::Result<()> {
    let mut order: Vec<usize> = (0..sections.len()).collect();
    order.sort_by_key(|&at| std::cmp::Reverse(sections[at].2.len()));
    let chosen = &*sections;
    let stored = pack::each(cores, order, |at| {
        let (_, raw, bytes) = &chosen[at];
        Ok((at, store(bytes, *raw)?))
    })?;
    for (at, bytes) in stored {
        if let Some(bytes) = bytes {
            sections[at].2 = bytes;
        }
    }
    Ok(())
}

/// `compressed`, a frame of `raw` plain bytes, at [`STORED_LEVEL`]; none when
/// that is not smaller.
fn store(compressed: &[u8], raw: usize) -> io::Result<Option<Vec<u8>>> {
    let mut plain = Vec::with_capacity(raw);
    zstd::stream::read::Decoder::with_buffer(compressed)?
        .single_frame()
        .read_to_end(&mut plain)?;
    if plain.len() != raw {
        return Err(invalid_static("chosen section raw length mismatch"));
    }
    let stored = compress_at(&plain, STORED_LEVEL)?;
    Ok((stored.len() < compressed.len()).then_some(stored))
}

/// A table's later entries compressed, or none without entries.
fn compress_later(raw: &[u8], entries: usize) -> io::Result<Option<Vec<u8>>> {
    if entries == 0 {
        return Ok(None);
    }
    compress(raw).map(Some)
}

#[cfg(test)]
thread_local! {
    /// The layout this thread's recordings are written in: tests write
    /// earlier layouts' anchored recordings with it.
    pub(crate) static WRITTEN_LAYOUT: std::cell::Cell<Layout> =
        const { std::cell::Cell::new(Layout::Chosen) };
    /// Whether this thread's blocks keep each predicting table's residual
    /// entries even where its plain entries compress smaller, so tests of
    /// residual coding do not depend on that choice.
    pub(crate) static FORCE_RESIDUAL: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

fn set(bits: &mut [u8], position: usize) {
    bits[position / 8] |= 1 << (position % 8);
}

fn get(bits: &[u8], position: usize) -> bool {
    bits[position / 8] & (1 << (position % 8)) != 0
}

/// A table's keyframe entry in a block at `position` in the file: its
/// snapshot, or a change against one of its anchor candidates (`snapshot`,
/// `latest`; [`Anchors`]) that compresses smaller, which become this
/// keyframe's. It reads only the table's own plain update, decoded rows and
/// candidates.
#[allow(clippy::too_many_arguments)]
fn put_keyframe(
    writer: &mut TableWriter,
    plain: &[u8],
    view: &SharedView,
    snapshot: &mut Option<Anchor>,
    latest: &mut Option<Anchor>,
    layout: Layout,
    first_hour: Hour,
    position: usize,
    marks: &[u8],
) -> io::Result<()> {
    let table = writer.table;
    let format = FormatVersion::CURRENT.version();
    let mut whole = Columns::new(writer.fields);
    put_entry(
        table,
        format,
        plain,
        Mode::Plain,
        &Shadow::new(),
        &mut whole,
        None,
    )?;
    whole.marks(table, marks)?;
    let raw = whole.raw();
    let snapshot_bytes = compress(&raw)?;
    // A change against an older snapshot is kept while it
    // compresses to at most half the snapshot; in an anchored
    // block, one against the table's latest keyframe whenever it
    // is smaller, its chain bounded by chain_charge.
    let half = snapshot_bytes.len() / 2;
    let latest_limit = if layout.anchors_back() {
        snapshot_bytes.len()
    } else {
        half
    };
    let latest_block = latest.as_ref().map(|latest| latest.block);
    let mut chosen = (raw.len(), snapshot_bytes);
    // Its chain: depth for earlier layouts, then the snapshot's
    // raw bytes and the charge.
    let mut anchored: Option<(usize, usize, usize)> = None;
    // The latest snapshot, or the latest keyframe while its chain
    // to a snapshot stays cheap to read.
    let depth = match layout {
        Layout::Anchored
        | Layout::Keyed
        | Layout::Ramped
        | Layout::Modelled
        | Layout::Trimmed
        | Layout::Advanced
        | Layout::Chosen => usize::MAX,
        Layout::Predicted => MAX_ANCHOR_DEPTH,
        Layout::Unpredicted | Layout::Timed => 1,
    };
    let candidates = [
        snapshot.as_ref(),
        latest
            .as_ref()
            .filter(|latest| latest.depth != 0 && latest.depth < depth),
    ];
    if plain[0] == SNAPSHOT
        && let TableView::Complete(rows) = &**view
    {
        for anchor in candidates.into_iter().flatten() {
            let TableView::Complete(anchor_rows) = &*anchor.rows else {
                continue;
            };
            let mut change = vec![DELTA];
            change.extend_from_slice(&anchor.hour.to_le_bytes());
            if !rows.encode_changes(anchor_rows, &mut change, &mut Vec::new())? {
                change.truncate(1);
                change[0] = UNCHANGED;
                change.extend_from_slice(&anchor.hour.to_le_bytes());
            }
            let mode = if layout.anchors_back() {
                Mode::Back {
                    blocks: position - anchor.block,
                }
            } else {
                Mode::Anchor { block: first_hour }
            };
            let mut candidate = Columns::new(writer.fields);
            put_entry(
                table,
                format,
                &change,
                mode,
                &anchor.shadow,
                &mut candidate,
                None,
            )?;
            candidate.marks(table, marks)?;
            let raw = candidate.raw();
            let charge = chain_charge(anchor.charge, raw.len(), anchor.root);
            if layout.anchors_back() && charge.is_none() {
                continue;
            }
            let compressed = compress(&raw)?;
            // Keep the smallest; its plain bytes never exceed the
            // snapshot's, so readers keep the writer's plain bound.
            let limit = if Some(anchor.block) == latest_block {
                latest_limit
            } else {
                half
            };
            if compressed.len() <= limit
                && compressed.len() < chosen.1.len()
                && change.len() <= plain.len()
            {
                chosen = (raw.len(), compressed);
                anchored = Some((anchor.depth + 1, anchor.root, charge.unwrap_or(usize::MAX)));
            }
        }
    }
    (writer.keyframe_raw, writer.keyframe) = chosen;
    advance_shadow(table, format, plain, &mut writer.shadow)?;
    let rows = match &**view {
        TableView::Complete(rows) => Some(rows),
        _ => None,
    };
    settle_advance(table, layout, &mut writer.shadow, rows);
    // Later entries name rows among the keyframe's when the block
    // stores it whole, and among none after an anchored change,
    // whose rows a reader discarding the table does not rebuild.
    writer.keys.clear();
    if layout.keyed() && anchored.is_none() {
        writer.keys.advance(table, format, plain)?;
    }
    if plain[0] == SNAPSHOT {
        let (depth, root, charge) = anchored.unwrap_or((0, raw.len(), 0));
        let keyframe = Anchor {
            hour: first_hour,
            block: position,
            rows: view.clone(),
            shadow: writer.shadow.clone(),
            depth,
            root,
            charge,
        };
        if anchored.is_none() {
            *snapshot = Some(keyframe.clone());
        }
        *latest = Some(keyframe);
    }
    Ok(())
}

impl BlockWriter {
    pub(crate) fn new(frame: &Frame) -> Self {
        #[cfg(test)]
        let layout = WRITTEN_LAYOUT.get();
        #[cfg(not(test))]
        let layout = Layout::Chosen;
        Self::with_layout(frame, layout)
    }

    /// A block in `layout`: only tests write the earlier ones.
    pub(crate) fn with_layout(frame: &Frame, layout: Layout) -> Self {
        let format = FormatVersion::CURRENT.version();
        let present = Present::new(format);
        let tables = present
            .tables
            .iter()
            .map(|&table| {
                let fields = table.last_change().fields(table, format);
                TableWriter {
                    table,
                    fields,
                    keyframe: Vec::new(),
                    keyframe_raw: 0,
                    plain: Later::new(table, fields, layout),
                    predicted: layout.predicts().then(|| Later::new(table, fields, layout)),
                    shadow: Shadow::new(),
                    predictor: predicted::Predictor::default(),
                    keys: KeySet::default(),
                }
            })
            .collect();
        let width = present.width();
        Self {
            branch: frame.branch,
            first_hour: frame.hour,
            last_hour: frame.hour,
            frames: 0,
            headers: 4,
            plain: vec![0; TABLE_COUNT],
            last_payload: 0,
            present,
            header: Vec::new(),
            diagnostics: Vec::new(),
            reads: vec![0; width],
            pool: 0,
            dropped: 0,
            tables,
            gaps: Vec::new(),
            layout,
            #[cfg(test)]
            force_residual: FORCE_RESIDUAL.get(),
            #[cfg(not(test))]
            force_residual: false,
        }
    }

    fn layout(&self) -> Layout {
        self.layout
    }

    /// Whether another plain frame of this size, with these per-table plain
    /// sizes, keeps each table's plain updates and the plain frame headers
    /// within `limit`, and the block's stored sections within
    /// [`MAX_BLOCK_RAW_BYTES`]. A later frame's entries take at most ten
    /// times its own and the previous frame's plain bytes (a residual row's
    /// key and mask against a one-byte plain change).
    pub(crate) fn fits(
        &self,
        payload_bytes: usize,
        sizes: &[usize; TABLE_COUNT],
        limit: usize,
    ) -> bool {
        let within = |used: usize, more: usize, limit: usize| {
            used.checked_add(more).is_some_and(|size| size <= limit)
        };
        let Some(header) = payload_bytes.checked_sub(sizes.iter().sum()) else {
            return false;
        };
        within(self.headers, header.saturating_add(4), limit)
            && self
                .plain
                .iter()
                .zip(sizes)
                .all(|(used, more)| within(*used, *more, limit))
            && (self.frames == 0
                || payload_bytes
                    .checked_add(self.last_payload)
                    .and_then(|bytes| bytes.checked_mul(10))
                    .and_then(|bytes| bytes.checked_add(4096))
                    .is_some_and(|more| within(self.raw_bytes(), more, MAX_BLOCK_RAW_BYTES)))
    }

    /// The block's stored section bytes so far, before compression.
    fn raw_bytes(&self) -> usize {
        // The header holds each later frame's hour gap and entry bits.
        self.gaps.len() * (19 + self.present.width())
            + self.diagnostics.len()
            + self
                .tables
                .iter()
                .map(|writer| {
                    writer.keyframe_raw
                        + writer
                            .predicted
                            .as_ref()
                            .map_or(0, Later::raw_bytes)
                            .max(writer.plain.raw_bytes())
                })
                .sum::<usize>()
    }

    /// Append a frame, given its plain encoding and per-table plain sizes and
    /// the decoded hour it produced. A keyframe's tables are written on this
    /// thread and the threads `cores` has free.
    #[allow(clippy::too_many_arguments)]
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn push(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        payload: &[u8],
        sizes: &[usize; TABLE_COUNT],
        anchors: &mut Anchors,
        limit: usize,
        cores: &Cores,
    ) -> io::Result<()> {
        self.push_marked(
            frame,
            decoded,
            payload,
            sizes,
            anchors,
            limit,
            cores,
            &Default::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_marked(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        payload: &[u8],
        sizes: &[usize; TABLE_COUNT],
        anchors: &mut Anchors,
        limit: usize,
        cores: &Cores,
        marks: &super::aliases::private::Prepared,
    ) -> io::Result<()> {
        let mut stored_sizes = *sizes;
        let stored_len = engine_allocation::add(payload.len(), marks.charge(&mut stored_sizes)?)?;
        if !self.fits(stored_len, &stored_sizes, limit) {
            return Err(invalid("single replay frame exceeds raw block size limit"));
        }
        let keyframe = self.frames == 0;
        if keyframe != frame.keyframe
            || frame.branch != self.branch
            || (!keyframe && frame.hour <= self.last_hour)
        {
            return Err(invalid(
                "block frame branch, keyframe, or hour ordering mismatch",
            ));
        }
        let layout = self.layout();
        // Earlier layouts anchor only within a branch.
        if keyframe && !layout.anchors_back() && anchors.branch != Some(self.branch) {
            *anchors = Anchors {
                branch: Some(self.branch),
                ..Anchors::at(anchors.position)
            };
        }
        let format = FormatVersion::CURRENT.version();
        let previous = self.last_hour;
        let position = anchors.position;
        // Each table's plain update.
        let mut offset = payload.len() - sizes.iter().sum::<usize>();
        let plains: Vec<&[u8]> = self
            .tables
            .iter()
            .map(|writer| {
                let plain = &payload[offset..offset + sizes[writer.table.index()]];
                offset += plain.len();
                plain
            })
            .collect();
        if keyframe {
            // Tables choose their keyframe entries independently, each
            // against its own anchor candidates.
            let mut candidates: Vec<_> = anchors
                .snapshots
                .iter_mut()
                .zip(&mut anchors.latest)
                .map(Some)
                .collect();
            let first_hour = self.first_hour;
            let tables: Vec<_> = self
                .tables
                .iter_mut()
                .zip(plains)
                .map(|(writer, plain)| {
                    let (snapshot, latest) = candidates[writer.table.index()]
                        .take()
                        .expect("one writer per table");
                    (writer, plain, snapshot, latest)
                })
                .collect();
            pack::each(cores, tables, |(writer, plain, snapshot, latest)| {
                let view = &decoded.tables[writer.table.index()];
                let column = marks.column(writer.table, true);
                put_keyframe(
                    writer, plain, view, snapshot, latest, layout, first_hour, position, column,
                )
            })?;
        } else {
            for (writer, plain) in self.tables.iter_mut().zip(plains) {
                let table = writer.table;
                let continuous = plain_baseline(plain) == Some(previous);
                // An unchanged table with the previous frame as its baseline
                // has no plain entry.
                let implied = plain[0] == UNCHANGED && continuous;
                let entry = !implied;
                if entry {
                    let coding = layout.keyed().then_some(RowCoding {
                        keys: &writer.keys,
                        masks: &mut writer.plain.masks,
                        model: writer.plain.model.as_mut(),
                        changes: None,
                    });
                    put_entry(
                        table,
                        format,
                        plain,
                        Mode::Later { previous },
                        &writer.shadow,
                        &mut writer.plain.columns,
                        coding,
                    )?;
                    writer.plain.entries += 1;
                }
                writer.plain.present.push(entry);
                if super::aliases::private::stored(table, format) {
                    writer.plain.columns.0[FIELDS + writer.fields]
                        .extend_from_slice(marks.column(table, false));
                    writer.plain.entries += usize::from(marks.column(table, false) != [0]);
                }
                if let Some(later) = &mut writer.predicted {
                    let coding = layout.keyed().then_some(RowCoding {
                        keys: &writer.keys,
                        masks: &mut later.masks,
                        model: later.model.as_mut(),
                        changes: None,
                    });
                    // A plain delta changing nothing (never written by the
                    // frame encoder) keeps its plain entry.
                    const EMPTY_DELTA: usize = 13;
                    let entry = if continuous && plain[0] == DELTA && plain.len() > EMPTY_DELTA {
                        // A delta whose changes are all predicted has no entry.
                        predicted::put_residual(
                            table,
                            format,
                            writer.fields,
                            plain,
                            &mut writer.predictor,
                            &writer.shadow,
                            &mut later.columns,
                            layout,
                            coding,
                        )?
                    } else {
                        // No entry is the unchanged table only while nothing
                        // is predicted.
                        let entry = !implied || writer.predictor.predicts();
                        writer.predictor.clear();
                        if entry {
                            put_entry(
                                table,
                                format,
                                plain,
                                Mode::Later { previous },
                                &writer.shadow,
                                &mut later.columns,
                                coding,
                            )?;
                        }
                        entry
                    };
                    later.entries += usize::from(entry);
                    later.present.push(entry);
                    if super::aliases::private::stored(table, format) {
                        later.columns.0[FIELDS + writer.fields]
                            .extend_from_slice(marks.column(table, false));
                        later.entries += usize::from(marks.column(table, false) != [0]);
                    }
                }
                advance_shadow(table, format, plain, &mut writer.shadow)?;
                if layout.keyed() {
                    writer.keys.advance(table, format, plain)?;
                }
            }
        }
        if keyframe {
            anchors.position += 1;
        } else {
            self.gaps
                .push((i128::from(frame.hour) - i128::from(previous) - 1) as u128);
        }
        let metrics = &frame.metrics;
        let diagnostics = &mut self.diagnostics;
        put_varint(diagnostics, u128::from(metrics.total_ns));
        put_varint(
            diagnostics,
            zigzag(i128::from(metrics.pool_bytes) - i128::from(self.pool)),
        );
        put_varint(
            diagnostics,
            zigzag(i128::from(metrics.dropped_frames) - i128::from(self.dropped)),
        );
        let mut reads = vec![0; self.present.width()];
        for (ordinal, table) in self.present.tables.iter().enumerate() {
            if metrics.table_ns[table.index()] != 0 {
                set(&mut reads, ordinal);
            }
        }
        // Which tables were read, not how long they took: the spool keeps
        // read times for the finalization summary.
        for (byte, old) in reads.iter().zip(&self.reads) {
            diagnostics.push(byte ^ old);
        }
        #[cfg(test)]
        if self.layout == Layout::Timed {
            for table in &self.present.tables {
                let ns = metrics.table_ns[table.index()];
                if ns != 0 {
                    let micros =
                        (ns / 1000 + u64::from(ns % 1000 >= 500)).clamp(1, u64::MAX / 1000);
                    put_varint(diagnostics, u128::from(micros - 1));
                }
            }
        }
        self.reads = reads;
        self.pool = metrics.pool_bytes;
        self.dropped = metrics.dropped_frames;
        self.frames = self
            .frames
            .checked_add(1)
            .ok_or_else(|| invalid("too many frames"))?;
        self.headers += 4 + payload.len() - sizes.iter().sum::<usize>();
        for (used, size) in self.plain.iter_mut().zip(&stored_sizes) {
            *used += size;
        }
        self.last_payload = stored_len;
        self.last_hour = frame.hour;
        Ok(())
    }

    /// Bound both stored updates and their literal materialization, so aliases
    /// cannot expand a table beyond the existing plain decoder allowance.
    pub(crate) fn charge_source(
        &mut self,
        bytes: usize,
        source: &[usize; TABLE_COUNT],
        stored: &[usize; TABLE_COUNT],
    ) {
        for ((used, source), stored) in self.plain.iter_mut().zip(source).zip(stored) {
            *used += source.saturating_sub(*stored);
        }
        self.last_payload = self.last_payload.max(bytes);
    }

    /// The finished block as [`compress`](Self::compress) reads it, without
    /// what only writing its frames reads (patch bases, predictions, row keys
    /// and mask lists), so a block waiting to be compressed holds little more
    /// than its sections' bytes and its context models.
    pub(crate) fn written(mut self) -> Self {
        for writer in &mut self.tables {
            writer.shadow = Shadow::new();
            writer.predictor = predicted::Predictor::default();
            writer.keys = KeySet::default();
            writer.plain.masks = MaskList::default();
            if let Some(later) = &mut writer.predicted {
                later.masks = MaskList::default();
            }
        }
        self
    }

    /// The block's sections, on this thread and the threads `cores` has
    /// free: each table's choices depend only on its own entries.
    pub(crate) fn compress(mut self, cores: &Cores) -> io::Result<CompressedBlock> {
        let magic = self.layout().magic();
        // Each table's later entries as it stores them, compressed.
        let force_residual = self.force_residual;
        let tables: Vec<_> = self.tables.iter_mut().collect();
        let chosen = pack::each(cores, tables, |writer: &mut TableWriter| {
            let mut best = writer.plain.compress()?;
            let mut predicted = false;
            if let Some(later) = &mut writer.predicted {
                let candidate = later.compress()?;
                if force_residual
                    || candidate.1.as_ref().map_or(0, Vec::len)
                        < best.1.as_ref().map_or(0, Vec::len)
                {
                    best = candidate;
                    predicted = true;
                }
            }
            if !predicted {
                writer.predicted = None;
            }
            Ok(best)
        })?;
        // Which tables have an entry in each frame after the keyframe, as
        // the change from the frame before.
        let width = self.present.width();
        let mut header = Vec::new();
        let mut previous = vec![0_u8; width];
        for ordinal in 0..self.tables.len() {
            set(&mut previous, ordinal);
        }
        for (frame, gap) in self.gaps.iter().enumerate() {
            put_varint(&mut header, *gap);
            let mut entries = vec![0_u8; width];
            for (ordinal, writer) in self.tables.iter().enumerate() {
                let later = writer.predicted.as_ref().unwrap_or(&writer.plain);
                if later.present[frame + 1] {
                    set(&mut entries, ordinal);
                }
            }
            header.extend(
                entries
                    .iter()
                    .zip(&previous)
                    .map(|(entry, old)| entry ^ old),
            );
            previous = entries;
        }
        self.header = header;
        let mut sections: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        if !self.header.is_empty() {
            sections.push((HEADER, self.header.len(), compress(&self.header)?));
        }
        sections.push((
            DIAGNOSTICS,
            self.diagnostics.len(),
            compress(&self.diagnostics)?,
        ));
        for (writer, (raw, later)) in self.tables.into_iter().zip(chosen) {
            sections.push((
                keyframe_section(writer.table),
                writer.keyframe_raw,
                writer.keyframe,
            ));
            if let Some(later) = later {
                sections.push((later_section(writer.table), raw, later));
            }
        }
        store_sections(&mut sections, cores)?;
        let mut directory = Vec::new();
        put_varint(&mut directory, u128::from(self.frames));
        put_varint(&mut directory, sections.len() as u128);
        let mut raw_length = 0_usize;
        for (id, raw, bytes) in &sections {
            put_varint(&mut directory, *id as u128);
            put_varint(&mut directory, *raw as u128);
            put_varint(&mut directory, bytes.len() as u128);
            raw_length += raw;
        }
        if raw_length > MAX_BLOCK_RAW_BYTES {
            return Err(invalid("grouped block exceeds raw size limit"));
        }
        let mut bytes = Vec::with_capacity(
            8 + directory.len()
                + sections
                    .iter()
                    .map(|section| section.2.len())
                    .sum::<usize>(),
        );
        bytes.extend_from_slice(magic);
        bytes.extend_from_slice(&crc32fast::hash(&directory).to_le_bytes());
        bytes.extend_from_slice(&directory);
        for (_, _, section) in &sections {
            bytes.extend_from_slice(section);
        }
        Ok(CompressedBlock {
            branch: self.branch,
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            raw_length: raw_length as u64,
            bytes,
        })
    }
}

/// One stored section of a grouped block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Header,
    Diagnostics,
    Keyframe(TableId),
    Later(TableId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionSize {
    pub section: Section,
    pub raw: u64,
    pub compressed: u64,
}

struct Directory {
    frames: usize,
    layout: Layout,
    /// By section id: offset in the block, compressed length, raw length.
    sections: Vec<Option<(usize, usize, usize)>>,
}

// The wire decoder admits unsigned u128 varints, at most 19 bytes each.
const MAX_VARINT_BYTES: usize = 19;

impl Directory {
    fn parse(block: &[u8], index: &BlockIndex) -> io::Result<Self> {
        Self::parse_extent(block, block.len(), index)
    }

    fn read(replay: &Replay, index: &BlockIndex) -> io::Result<Self> {
        if !replay.is_file_backed() {
            return Self::parse(&replay.block_bytes(index)?, index);
        }
        Self::read_from_prefix(replay, index, Self::prefix(replay, index)?)
    }

    fn prefix<'a>(replay: &'a Replay, index: &BlockIndex) -> io::Result<Cow<'a, [u8]>> {
        replay.block_range(
            index,
            0,
            super::to_usize(index.length)?.min(8 + 2 * MAX_VARINT_BYTES),
        )
    }

    fn read_from_prefix<'a>(
        replay: &'a Replay,
        index: &BlockIndex,
        mut bytes: Cow<'a, [u8]>,
    ) -> io::Result<Self> {
        let length = super::to_usize(index.length)?;
        let mut input = Decoder::new(&bytes);
        let (_, _, _, count) = Self::counts(&mut input, index)?;
        let prefix_end = bytes.len() - input.remaining();
        let end = (prefix_end + count * 3 * MAX_VARINT_BYTES).min(length);
        if end > bytes.len() {
            if replay.is_file_backed() {
                let bytes = bytes.to_mut();
                let start = bytes.len();
                engine_allocation::reserve(bytes, end - start)?;
                bytes.resize(end, 0);
                let offset = index
                    .offset
                    .checked_add(start as u64)
                    .ok_or_else(|| invalid_static("block offset overflow"))?;
                replay.read_at(offset, &mut bytes[start..])?;
            } else {
                bytes = replay.block_range(index, 0, end)?;
            }
        }
        Self::parse_extent(&bytes, length, index)
    }

    fn counts(
        input: &mut Decoder<'_>,
        index: &BlockIndex,
    ) -> io::Result<(Layout, u32, usize, usize)> {
        if input.remaining() < 8 {
            return Err(invalid_static("block is not a grouped block"));
        }
        let prefix = input.take(8)?;
        let Some(layout) = Layout::of(prefix) else {
            return Err(invalid_static("block is not a grouped block"));
        };
        let crc = u32::from_le_bytes(prefix[4..8].try_into().unwrap());
        let frames = varint_usize(input)?;
        let span = i64::from(index.last_hour) - i64::from(index.first_hour) + 1;
        if frames == 0 || frames as i64 > span {
            return Err(invalid_static(
                "grouped frame count disagrees with its hour range",
            ));
        }
        let count = varint_usize(input)?;
        if count > SECTIONS {
            return Err(invalid_static("grouped block has too many sections"));
        }
        Ok((layout, crc, frames, count))
    }

    fn parse_extent(bytes: &[u8], block_length: usize, index: &BlockIndex) -> io::Result<Self> {
        let mut input = Decoder::new(bytes);
        let (layout, crc, frames, count) = Self::counts(&mut input, index)?;
        let mut entries = Vec::with_capacity(count);
        let mut previous = None;
        for _ in 0..count {
            let id = varint_usize(&mut input)?;
            if id >= SECTIONS || previous.is_some_and(|previous| id <= previous) {
                return Err(invalid_static(
                    "grouped sections are unknown or out of order",
                ));
            }
            previous = Some(id);
            let raw = varint_usize(&mut input)?;
            let length = varint_usize(&mut input)?;
            if length == 0 {
                return Err(invalid_static("grouped section is empty"));
            }
            entries.push((id, raw, length));
        }
        let end = bytes.len() - input.remaining();
        if crc32fast::hash(&bytes[8..end]) != crc {
            return Err(invalid_static("grouped block directory CRC mismatch"));
        }
        let mut sections = vec![None; SECTIONS];
        let mut offset = end;
        let mut raw_total = 0_usize;
        for (id, raw, length) in entries {
            let next = offset
                .checked_add(length)
                .filter(|next| *next <= block_length)
                .ok_or_else(|| invalid_static("grouped section outside its block"))?;
            raw_total = raw_total
                .checked_add(raw)
                .ok_or_else(|| invalid_static("block raw length mismatch"))?;
            sections[id] = Some((offset, length, raw));
            offset = next;
        }
        if offset != block_length {
            return Err(invalid_static(
                "grouped block has trailing or concatenated bytes",
            ));
        }
        if raw_total as u64 != index.raw_length {
            return Err(invalid_static("block raw length mismatch"));
        }
        Ok(Self {
            frames,
            layout,
            sections,
        })
    }

    fn raw(&self, block: &[u8], id: usize) -> io::Result<Option<Vec<u8>>> {
        let Some((offset, length, raw)) = self.sections[id] else {
            return Ok(None);
        };
        Self::inflate(&block[offset..offset + length], raw).map(Some)
    }

    fn read_raw(
        &self,
        replay: &Replay,
        index: &BlockIndex,
        id: usize,
    ) -> io::Result<Option<Vec<u8>>> {
        let Some((offset, length, raw)) = self.sections[id] else {
            return Ok(None);
        };
        Self::inflate(&replay.block_range(index, offset, length)?, raw).map(Some)
    }

    fn inflate(compressed: &[u8], raw: usize) -> io::Result<Vec<u8>> {
        let _cpu = crate::cpu::scope("inflate", None);
        if compressed.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd]) {
            return Err(invalid_static("grouped section is not a zstd frame"));
        }
        let frame_size = zstd::zstd_safe::find_frame_compressed_size(compressed)
            .map_err(|_| invalid_static("invalid zstd frame"))?;
        if frame_size != compressed.len() {
            return Err(invalid_static(
                "grouped section contains trailing or concatenated zstd frames",
            ));
        }
        // Damage is invalid data, as for an interleaved block.
        let mut decompressor = zstd::stream::read::Decoder::with_buffer(compressed)
            .map_err(zstd_error)?
            .single_frame();
        decompressor.window_log_max(26).map_err(zstd_error)?;
        let mut bytes = Vec::with_capacity(raw);
        decompressor
            .take(raw as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(zstd_error)?;
        if bytes.len() != raw {
            return Err(invalid_static("grouped section raw length mismatch"));
        }
        Ok(bytes)
    }
}

/// The stored sections of a grouped block, in order.
pub(crate) fn sections(block: &[u8], index: &BlockIndex) -> io::Result<Vec<SectionSize>> {
    let directory = Directory::parse(block, index)?;
    Ok(directory
        .sections
        .iter()
        .enumerate()
        .filter_map(|(id, section)| {
            section.map(|(_, length, raw)| SectionSize {
                section: match id {
                    HEADER => Section::Header,
                    DIAGNOSTICS => Section::Diagnostics,
                    id if id % 2 == 0 => Section::Keyframe(TableId::ALL[(id - 2) / 2]),
                    id => Section::Later(TableId::ALL[(id - 3) / 2]),
                },
                raw: raw as u64,
                compressed: length as u64,
            })
        })
        .collect())
}

/// Inflate every section of a grouped block, checking the directory's CRC and
/// each section's zstd checksum, without decoding any table.
pub(crate) fn verify(block: &[u8], index: &BlockIndex) -> io::Result<()> {
    let _cpu = crate::cpu::scope("decode", None);
    let directory = Directory::parse(block, index)?;
    for id in 0..SECTIONS {
        directory.raw(block, id)?;
    }
    Ok(())
}

/// The stored table sections of a grouped block split into their columns,
/// in section order: size attribution reads which column holds the bytes.
pub(crate) fn section_columns(block: &[u8], index: &BlockIndex) -> io::Result<Vec<ColumnSizes>> {
    let directory = Directory::parse(block, index)?;
    let mut sections = Vec::new();
    for id in 2..SECTIONS {
        let Some((_, length, _)) = directory.sections[id] else {
            continue;
        };
        let raw = directory
            .raw(block, id)?
            .ok_or_else(|| invalid_static("grouped section vanished"))?;
        let mut input = Decoder::new(&raw);
        let count = varint_usize(&mut input)?;
        if !(FIELDS..=FIELDS + 64).contains(&count) {
            return Err(invalid_static("grouped section has the wrong column count"));
        }
        let lengths = (0..count)
            .map(|_| varint_usize(&mut input))
            .collect::<io::Result<Vec<_>>>()?;
        let mut columns = Vec::with_capacity(count);
        for length in lengths {
            let column = input.take(length)?;
            let alone = if column.is_empty() {
                0
            } else {
                compress(column)?.len() as u64
            };
            columns.push(ColumnSize {
                raw: length as u64,
                alone,
            });
        }
        input.finish()?;
        let table = TableId::ALL[(id - 2) / 2];
        sections.push(ColumnSizes {
            section: if id % 2 == 0 {
                Section::Keyframe(table)
            } else {
                Section::Later(table)
            },
            compressed: length as u64,
            columns,
        });
    }
    Ok(sections)
}

/// Every stored section of a grouped block, in section order, with its
/// stored compressed bytes and its plain bytes compressed again at each of
/// `levels`: size attribution compares compression levels without writing
/// the recording again.
pub(crate) fn section_levels(
    block: &[u8],
    index: &BlockIndex,
    levels: &[i32],
) -> io::Result<Vec<SectionLevels>> {
    let directory = Directory::parse(block, index)?;
    let sizes = sections(block, index)?;
    let mut out = Vec::with_capacity(sizes.len());
    let ids = (0..SECTIONS).filter(|&id| directory.sections[id].is_some());
    for (id, size) in ids.zip(sizes) {
        let raw = directory
            .raw(block, id)?
            .ok_or_else(|| invalid_static("grouped section vanished"))?;
        let mut section = SectionLevels {
            section: size.section,
            compressed: size.compressed,
            levels: Vec::with_capacity(levels.len()),
            elapsed: Vec::with_capacity(levels.len()),
        };
        for &level in levels {
            let started = std::time::Instant::now();
            section.levels.push(compress_at(&raw, level)?.len() as u64);
            section.elapsed.push(started.elapsed());
        }
        out.push(section);
    }
    Ok(out)
}

/// A stored section's compressed bytes, and its plain bytes compressed again
/// at each requested level, with the time each took on one thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionLevels {
    pub section: Section,
    pub compressed: u64,
    pub levels: Vec<u64>,
    pub elapsed: Vec<std::time::Duration>,
}

/// One column of a stored table section: its raw bytes and its bytes
/// compressed alone at the sections' level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnSize {
    pub raw: u64,
    pub alone: u64,
}

/// A stored table section's compressed bytes and its columns: control,
/// keys, masks, then one per field ([`TableId::field_name`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnSizes {
    pub section: Section,
    pub compressed: u64,
    pub columns: Vec<ColumnSize>,
}

struct Context<'a> {
    replay: &'a Replay,
    index: &'a BlockIndex,
    /// The block's position in the file's block index, if it is there.
    position: Option<usize>,
    block: &'a [u8],
    directory: &'a Directory,
    format: FormatVersion,
    present: &'a Present,
    hours: &'a [Hour],
    entries: &'a [u8],
    selected: Option<&'a [TableId]>,
    complete: bool,
    memo: &'a [OnceLock<Updates>],
}

impl Replay {
    /// Read the selected tables of a block's first frame. Grouped reads validate
    /// the directory, header, diagnostics, selected keyframes and their anchors;
    /// later and unrelated table sections are not admitted or marked verified.
    /// Ordinary table compatibility refusals retain their recorded metadata.
    pub fn read_keyframe_selected(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
    ) -> io::Result<Frame> {
        self.read_keyframe_recorded(index, tables)?
            .into_owned_frame()
    }

    /// The selected keyframe with immutable shared update cells.
    pub fn read_keyframe_recorded(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
    ) -> io::Result<RecordedFrame> {
        let _consumer = self.section_consumer(self.block_position(index).unwrap_or(0));
        if index.raw_length > MAX_BLOCK_RAW_BYTES as u64 {
            return Err(invalid("block exceeds raw size limit"));
        }
        let prefix = Directory::prefix(self, index)?;
        if !is_grouped(&prefix) {
            return self
                .read_block_selected(index, tables)?
                .into_iter()
                .next()
                .map(RecordedFrame::from)
                .ok_or_else(|| invalid("empty block"));
        }
        if self.format.version() < GROUPED_FORMAT {
            return Err(invalid(format!(
                "grouped block in a format {} recording",
                self.format.version()
            )));
        }
        let directory = Directory::read_from_prefix(self, index, prefix)?;
        let present = Present::new(self.format.version());
        check_header_size(directory.frames, &present)?;
        let header = directory.read_raw(self, index, HEADER)?.unwrap_or_default();
        read_header(&header, index, directory.frames, &present)?;
        let diagnostics = directory
            .read_raw(self, index, DIAGNOSTICS)?
            .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
        let metrics = read_diagnostics_prefix(
            &diagnostics,
            directory.frames,
            &present,
            directory.layout == Layout::Timed,
            1,
        )?
        .pop()
        .expect("one keyframe's metrics");
        let from = From {
            position: self.block_position(index),
            index,
            layout: directory.layout,
        };
        let memo = engine_allocation::collect_vec(TableId::ALL.iter().map(|_| OnceLock::new()))?;
        let updates = engine_allocation::collect_vec(TableId::ALL.iter().map(|&table| {
            if let Some(update) = keyframe_refusal(&directory, self.format, Some(tables), table)? {
                return Updates::new(vec![update]);
            }
            keyframe_admitted(self, &from, &directory, table, &memo)
        }))?;
        let columns = updates.into_iter().collect::<io::Result<Vec<_>>>()?;
        RecordedFrame::from_columns(
            index.branch,
            index.first_hour,
            true,
            metrics,
            Arc::new(std::array::from_fn(Some)),
            &columns,
            0,
            Arc::new(TableUpdate::Unavailable {
                reason: String::new(),
            }),
        )
    }

    /// Selected recorded updates shared without materializing owned wire frames.
    pub fn read_block_recorded_selected(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
    ) -> io::Result<Vec<RecordedFrame>> {
        self.read_block_recorded_through(index, tables, index.last_hour)
    }

    /// The selected prefix as immutable updates. A prefix never admits later grammar.
    pub fn read_block_recorded_through(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        through: Hour,
    ) -> io::Result<Vec<RecordedFrame>> {
        self.read_block_recorded_inner(index, tables, through, None)
    }
    /// Shared updates decoded on caller-owned spare workers.
    pub fn read_block_recorded_selected_parallel(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        parallel: &Parallel<'_>,
    ) -> io::Result<Vec<RecordedFrame>> {
        self.read_block_recorded_inner(index, tables, index.last_hour, Some(parallel))
    }
    fn read_block_recorded_inner(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        through: Hour,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<RecordedFrame>> {
        let _consumer = self.section_consumer(self.block_position(index).unwrap_or(0));
        if let Some(block) = self.grouped_block(index)? {
            let frames =
                read_recorded_through(self, index, &block, Some(tables), parallel, Some(through))?;
            let mut order = super::BlockOrder::new(index);
            for frame in &frames {
                order.next(frame)?;
            }
            if frames
                .last()
                .is_some_and(|frame| frame.hour == index.last_hour)
            {
                order.finish()?;
                if !self.is_verified(index) {
                    verify(&block, index)?;
                }
                self.mark_verified(index);
            }
            return Ok(frames);
        }
        self.read_block_selected_through(index, Some(tables), through, None)?
            .into_iter()
            .map(|frame| Ok(frame.into()))
            .collect()
    }

    /// Read selected tables through `through`, including an equal hour.
    /// Grouped reads validate the complete directory, header and diagnostics,
    /// every inflated section's framing and checksum, selected keyframes and
    /// anchors, and the decoded prefix's ordinary wire grammar and budgets.
    /// Later frames' column bytes have not been decoded or grammar-verified;
    /// an early read does not mark the block verified. A whole-block read
    /// retains ordinary full verification. Interleaved blocks use `visit_block`
    /// and its ordinary verification and stopping rules. With `parallel`, a
    /// grouped block's tables decode on the caller's workers, in table order.
    pub fn read_block_selected_through(
        &self,
        index: &BlockIndex,
        tables: Option<&[TableId]>,
        through: Hour,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<Frame>> {
        if let Some(block) = self.grouped_block(index)? {
            let frames = read_through(self, index, &block, tables, parallel, Some(through))?;
            let mut order = super::BlockOrder::new(index);
            for frame in &frames {
                order.next(frame)?;
            }
            if frames
                .last()
                .is_some_and(|frame| frame.hour == index.last_hour)
            {
                order.finish()?;
                if tables.is_some() && !self.is_verified(index) {
                    verify(&block, index)?;
                }
                self.mark_verified(index);
            }
            return Ok(frames);
        }
        let mut frames = Vec::new();
        self.visit_block(index, tables, |frame| {
            if frame.hour > through {
                return Ok(false);
            }
            let keep_reading = frame.hour < through;
            engine_allocation::reserve(&mut frames, 1)?;
            frames.push(frame);
            Ok(keep_reading)
        })?;
        Ok(frames)
    }
}

fn keyframe_admitted(
    replay: &Replay,
    from: &From<'_>,
    directory: &Directory,
    table: TableId,
    memo: &[OnceLock<Updates>],
) -> io::Result<Updates> {
    if let Some(updates) = memo[table.index()].get() {
        return Ok(updates.clone());
    }
    let updates = replay.decoded_sections.read(
        super::decoded_sections::Key::new(replay, from.index, table),
        1,
        true,
        || {
            let raw = directory
                .read_raw(replay, from.index, keyframe_section(table))?
                .ok_or_else(|| {
                    invalid(format!("grouped block has no keyframe entry for {table:?}"))
                })?;
            #[cfg(any(test, feature = "test-support"))]
            super::test_work::decoded(from.index, table, 1);
            let keyframe = keyframe_update(
                replay,
                from,
                table,
                &raw,
                &Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES)),
            )?;
            super::aliases::private::resolve(
                table,
                &[from.index.first_hour],
                vec![keyframe.update],
                keyframe.marks.as_deref(),
                None,
                true,
                |source| {
                    if let Some(update) = keyframe_refusal(directory, replay.format, None, source)?
                    {
                        return Updates::new(vec![update]);
                    }
                    keyframe_admitted(replay, from, directory, source, memo)
                },
            )
        },
    )?;
    let _ = memo[table.index()].set(updates.clone());
    Ok(updates)
}

/// Decode a grouped block's frames.
pub(crate) fn read(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: Option<&[TableId]>,
    parallel: Option<&Parallel<'_>>,
) -> io::Result<Vec<Frame>> {
    read_through(replay, index, block, selected, parallel, None)
}

/// The same column reader, optionally stopping at a recorded hour. Parallel
/// work uses the caller's existing table workers and preserves table order.
fn read_recorded_through(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: Option<&[TableId]>,
    parallel: Option<&Parallel<'_>>,
    through: Option<Hour>,
) -> io::Result<Vec<RecordedFrame>> {
    let _consumer = replay.section_consumer(replay.block_position(index).unwrap_or(0));
    let _cpu = crate::cpu::scope("decode", None);
    let format = replay.format;
    if format.version() < GROUPED_FORMAT {
        return Err(invalid(format!(
            "grouped block in a format {} recording",
            format.version()
        )));
    }
    let directory = Directory::parse(block, index)?;
    let present = Present::new(format.version());
    let frames = directory.frames;
    check_header_size(frames, &present)?;
    let header = directory.raw(block, HEADER)?.unwrap_or_default();
    let (hours, entries) = read_header(&header, index, frames, &present)?;
    let keep = through.map_or(frames, |through| {
        hours.partition_point(|hour| *hour <= through)
    });
    let diagnostics = directory
        .raw(block, DIAGNOSTICS)?
        .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
    let timed = directory.layout == Layout::Timed;
    let metrics = read_diagnostics_prefix(&diagnostics, frames, &present, timed, keep)?;
    if keep == 0 {
        return Ok(Vec::new());
    }
    let hours = &hours[..keep];
    let memo = engine_allocation::collect_vec(TableId::ALL.iter().map(|_| OnceLock::new()))?;
    let context = Context {
        replay,
        index,
        position: replay.index.iter().position(|block| block == index),
        block,
        directory: &directory,
        format,
        present: &present,
        hours,
        entries: &entries,
        selected,
        complete: keep == frames,
        memo: &memo,
    };
    let decode = |table: &TableId| table_updates(&context, *table);
    let decoded = match parallel {
        Some(parallel) => parallel_map(&TableId::ALL, parallel, decode)?,
        None => engine_allocation::collect_vec(TableId::ALL.iter().map(decode))?,
    };
    let columns = decoded.into_iter().collect::<io::Result<Vec<_>>>()?;
    let slots = Arc::new(std::array::from_fn(Some));
    let unselected = Arc::new(TableUpdate::Unavailable {
        reason: String::new(),
    });
    let mut out = engine_allocation::vec(keep)?;
    for (position, (hour, metrics)) in hours.iter().zip(metrics).enumerate() {
        out.push(RecordedFrame::from_columns(
            index.branch,
            *hour,
            position == 0,
            metrics,
            Arc::clone(&slots),
            &columns,
            position,
            Arc::clone(&unselected),
        )?);
    }
    Ok(out)
}

fn read_through(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: Option<&[TableId]>,
    parallel: Option<&Parallel<'_>>,
    through: Option<Hour>,
) -> io::Result<Vec<Frame>> {
    read_recorded_through(replay, index, block, selected, parallel, through)?
        .into_iter()
        .map(RecordedFrame::into_owned_frame)
        .collect()
}

/// A grouped block's hours, metrics and its selected tables' updates at
/// every hour, decoded and admitted exactly as [`read`] decodes them for the
/// same selection, without assembling frames: no unselected table's update is
/// repeated at every hour.
pub(crate) struct SelectedUpdates {
    pub(crate) hours: Vec<Hour>,
    pub(crate) metrics: Vec<Metrics>,
    /// Each selected table's update at every hour, in the selection's order.
    pub(crate) updates: Vec<Updates>,
    /// An unselected table [`read`] still decodes (a catalogued layout gap)
    /// has a first update no keyframe may hold.
    pub(crate) dependent_keyframe: bool,
}

/// [`read`] of a whole block for `selected` (each table once), keeping each
/// selected table's updates by table rather than by frame. The same sections
/// are inflated and decoded, in the same table order, so it fails exactly
/// where [`read`] fails. `check` is called before each table is decoded; its
/// error ends the read.
pub(crate) fn read_selected_updates(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: &[TableId],
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<SelectedUpdates> {
    let _consumer = replay.section_consumer(replay.block_position(index).unwrap_or(0));
    let _cpu = crate::cpu::scope("decode", None);
    let format = replay.format;
    if format.version() < GROUPED_FORMAT {
        return Err(invalid(format!(
            "grouped block in a format {} recording",
            format.version()
        )));
    }
    let directory = Directory::parse(block, index)?;
    let present = Present::new(format.version());
    let frames = directory.frames;
    check_header_size(frames, &present)?;
    let header = directory.raw(block, HEADER)?.unwrap_or_default();
    let (hours, entries) = read_header(&header, index, frames, &present)?;
    let diagnostics = directory
        .raw(block, DIAGNOSTICS)?
        .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
    let timed = directory.layout == Layout::Timed;
    let metrics = read_diagnostics_prefix(&diagnostics, frames, &present, timed, frames)?;
    let memo = engine_allocation::collect_vec(TableId::ALL.iter().map(|_| OnceLock::new()))?;
    let context = Context {
        replay,
        index,
        position: replay.index.iter().position(|block| block == index),
        block,
        directory: &directory,
        format,
        present: &present,
        hours: &hours,
        entries: &entries,
        selected: Some(selected),
        complete: true,
        memo: &memo,
    };
    let mut updates: Vec<Option<Updates>> =
        engine_allocation::collect_vec(selected.iter().map(|_| None))?;
    let mut dependent_keyframe = false;
    for table in TableId::ALL {
        check()?;
        let wanted = selected.iter().position(|selected| *selected == table);
        // One selection-aware admission per table. A present layout gap
        // still reaches the ordinary decoder even when not selected.
        let refusal = keyframe_refusal(&directory, format, Some(selected), table)?;
        let decoded = match (wanted, refusal) {
            (None, Some(_)) => continue,
            (Some(_), Some(update)) => Updates::repeat(update, frames)?,
            (_, None) => table_updates_admitted(&context, table)?,
        };
        match wanted {
            Some(position) => updates[position] = Some(decoded),
            None => {
                dependent_keyframe |= decoded.first().is_some_and(|update| {
                    !matches!(
                        update,
                        TableUpdate::Snapshot(_)
                            | TableUpdate::Unavailable { .. }
                            | TableUpdate::Incompatible { .. }
                    )
                });
            }
        }
    }
    let updates = updates
        .into_iter()
        .map(|updates| updates.ok_or_else(|| invalid_static("table selected twice")))
        .collect::<io::Result<Vec<_>>>()?;
    Ok(SelectedUpdates {
        hours,
        metrics,
        updates,
        dependent_keyframe,
    })
}

fn check_header_size(frames: usize, present: &Present) -> io::Result<()> {
    // A plain block counts its frames, then frames each prefixed by their
    // length; each table's plain updates have their own budget.
    Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES))
        .spend(4 + frames * (4 + present.plain_header_bytes()))
}

fn read_header(
    header: &[u8],
    index: &BlockIndex,
    frames: usize,
    present: &Present,
) -> io::Result<(Vec<Hour>, Vec<u8>)> {
    let width = present.width();
    let mut input = Decoder::new(header);
    let mut hours = Vec::with_capacity(frames);
    hours.push(index.first_hour);
    let mut entries = vec![0xff_u8; width * frames];
    let tail = present.tables.len() % 8;
    if tail != 0 {
        entries[width - 1] = (1 << tail) - 1;
    }
    for frame in 1..frames {
        let gap = i128::try_from(input.varint()?).unwrap_or(i128::MAX);
        hours.push(hour_from(i128::from(hours[frame - 1]) + 1 + gap)?);
        let bits = input.take(width)?;
        for byte in 0..width {
            entries[frame * width + byte] = entries[(frame - 1) * width + byte] ^ bits[byte];
        }
        if tail != 0 && entries[frame * width + width - 1] >> tail != 0 {
            return Err(invalid_static("grouped entry bits name an absent table"));
        }
    }
    input.finish()?;
    if *hours.last().unwrap() != index.last_hour {
        return Err(invalid("block index hour range mismatch"));
    }
    Ok((hours, entries))
}

fn read_diagnostics_prefix(
    raw: &[u8],
    frames: usize,
    present: &Present,
    timed: bool,
    keep: usize,
) -> io::Result<Vec<Metrics>> {
    let mut input = Decoder::new(raw);
    let width = present.width();
    let mut reads = vec![0; width];
    let (mut pool, mut dropped) = (0_u64, 0_u64);
    let tail = present.tables.len() % 8;
    let mut out = Vec::with_capacity(keep);
    let u64_from = |value: i128| {
        u64::try_from(value).map_err(|_| invalid_static("grouped metric outside its range"))
    };
    for frame in 0..frames {
        let total_ns = u64_from(i128::try_from(input.varint()?).unwrap_or(-1))?;
        pool = u64_from(i128::from(pool) + unzigzag(input.varint()?))?;
        dropped = u64_from(i128::from(dropped) + unzigzag(input.varint()?))?;
        let bits = input.take(width)?;
        for (byte, change) in reads.iter_mut().zip(bits) {
            *byte ^= change;
        }
        if tail != 0 && reads[width - 1] >> tail != 0 {
            return Err(invalid_static("grouped read bits name an absent table"));
        }
        let mut table_ns = [0; TABLE_COUNT];
        // Untimed layouts have no per-table values left to validate. Only
        // materialize sample flags for the metrics the caller retains.
        if frame < keep || timed {
            for (ordinal, table) in present.tables.iter().enumerate() {
                if get(&reads, ordinal) && !timed {
                    table_ns[table.index()] = READ_TIME_NOT_RECORDED;
                } else if get(&reads, ordinal) {
                    let micros = u64::try_from(input.varint()?)
                        .ok()
                        .and_then(|micros| micros.checked_add(1))
                        .filter(|micros| *micros <= u64::MAX / 1000)
                        .ok_or_else(|| invalid_static("grouped table time outside its range"))?;
                    table_ns[table.index()] = micros * 1000;
                }
            }
        }
        if frame < keep {
            out.push(Metrics {
                total_ns,
                table_ns,
                pool_bytes: pool,
                dropped_frames: dropped,
            });
        }
    }
    input.finish()?;
    Ok(out)
}

fn keyframe_refusal(
    directory: &Directory,
    format: FormatVersion,
    selected: Option<&[TableId]>,
    table: TableId,
) -> io::Result<Option<TableUpdate>> {
    #[cfg(test)]
    decoded_section_tests::admitted(table);
    let version = format.version();
    let change = &HISTORY[table.index()];
    let key_id = keyframe_section(table);
    let later_id = later_section(table);
    if !change.present(version) {
        if directory.sections[key_id].is_some() || directory.sections[later_id].is_some() {
            return Err(invalid_static("grouped block stores an absent table"));
        }
        let reason = change
            .table_gap(table, version)
            .ok_or_else(|| invalid_static("absent table has no catalogued lifetime gap"))?;
        return Ok(Some(TableUpdate::Unavailable { reason }));
    }
    let gap = change.table_gap(table, version);
    if gap.is_none() && selected.is_some_and(|selected| !selected.contains(&table)) {
        return Ok(Some(TableUpdate::Unavailable {
            reason: "Table not selected for reconstruction".into(),
        }));
    }
    Ok(None)
}

struct KeyframeUpdate {
    marks: Option<Vec<u8>>,
    update: TableUpdate,
    plain: Vec<u8>,
    shadow: Shadow,
    anchored: bool,
}

fn keyframe_update(
    replay: &Replay,
    from: &From<'_>,
    table: TableId,
    raw: &[u8],
    budget: &Budget,
) -> io::Result<KeyframeUpdate> {
    let _cpu = crate::cpu::scope("decode_keyframe", None);
    let format = replay.format;
    let change = &HISTORY[table.index()];
    let fields = change.fields(table, format.version());
    let discarded = change.table_gap(table, format.version()).is_some();
    let layout = from.layout;
    let first = from.index.first_hour;
    let mut key_input = ColumnReader::stored(raw, fields, table, format.version())?;
    let marks = key_input.1.map(engine_allocation::copy_vec).transpose()?;
    let mut plain = Vec::new();
    let mut shadow = Shadow::new();
    let decode = |plain: &[u8], keyframe: bool| -> io::Result<TableUpdate> {
        let _cpu = crate::cpu::scope("decode_wire", None);
        let mut input = Decoder::new(plain);
        let update = decode_update(table, change, format, keyframe, &mut input)?;
        input.finish()?;
        Ok(update)
    };
    let head = take_head(&mut key_input, None, first, &mut plain, layout)?;
    let update = if let Some(anchor) = head.anchor {
        let own = raw.len();
        let anchor = anchored_keyframe(
            replay, from, table, fields, discarded, anchor, own, format, false,
        )?
        .state;
        shadow = anchor.shadow;
        take_body(
            table,
            fields,
            head.status,
            &mut key_input,
            &shadow,
            &mut plain,
            budget,
            layout,
            None,
        )?;
        match anchor.rows {
            None => decode(&plain, false)?,
            Some(rows) => TableUpdate::Snapshot(apply_anchor_change(rows, decode(&plain, false)?)?),
        }
    } else {
        take_body(
            table,
            fields,
            head.status,
            &mut key_input,
            &shadow,
            &mut plain,
            budget,
            layout,
            None,
        )?;
        decode(&plain, true)?
    };
    key_input.finish()?;
    Ok(KeyframeUpdate {
        marks,
        update,
        plain,
        shadow,
        anchored: head.anchor.is_some(),
    })
}

fn table_updates(context: &Context<'_>, table: TableId) -> io::Result<Updates> {
    // Unselected tables keep their own compatibility and framing checks.
    if let Some(update) =
        keyframe_refusal(context.directory, context.format, context.selected, table)?
    {
        return Updates::repeat(update, context.hours.len());
    }
    table_updates_admitted(context, table)
}

// The cached decoder receives only admitted sections. No selection-dependent
// value can enter the cache; selected tables and unselected layout gaps use
// this identical decoder. Selection below only attributes CPU measurements.
fn dependency_updates(context: &Context<'_>, table: TableId) -> io::Result<Updates> {
    if let Some(update) = keyframe_refusal(context.directory, context.format, None, table)? {
        return Updates::repeat(update, context.hours.len());
    }
    table_updates_admitted(context, table)
}

fn table_updates_admitted(context: &Context<'_>, table: TableId) -> io::Result<Updates> {
    if let Some(updates) = context.memo[table.index()].get() {
        return Ok(updates.clone());
    }
    let updates = context.replay.decoded_sections.read(
        super::decoded_sections::Key::new(context.replay, context.index, table),
        context.hours.len(),
        false,
        || {
            let _cpu = crate::cpu::scope("decode", None);
            let started = crate::cpu::thread_cycles();
            let decoded = table_updates_inner(context, table);
            if crate::cpu::enabled()
                && context
                    .selected
                    .is_none_or(|selected| selected.contains(&table))
            {
                crate::cpu::decoded(
                    context.index.branch,
                    context.index.first_hour,
                    table,
                    crate::cpu::thread_cycles().saturating_sub(started),
                );
            }
            decoded
        },
    )?;
    let _ = context.memo[table.index()].set(updates.clone());
    Ok(updates)
}

fn table_updates_inner(context: &Context<'_>, table: TableId) -> io::Result<Vec<TableUpdate>> {
    let format = context.format;
    let version = format.version();
    let change: &LastChange = &HISTORY[table.index()];
    let frames = context.hours.len();
    let key_id = keyframe_section(table);
    let later_id = later_section(table);
    #[cfg(test)]
    super::watched::note_decoded();
    #[cfg(any(test, feature = "test-support"))]
    super::test_work::decoded(context.index, table, context.hours.len());
    let gap = change.table_gap(table, version);
    let fields = change.fields(table, version);
    let ordinal = context.present.ordinal[table.index()];
    let width = context.present.width();
    let key_raw = context
        .directory
        .raw(context.block, key_id)?
        .ok_or_else(|| invalid(format!("grouped block has no keyframe entry for {table:?}")))?;
    let later_raw = context.directory.raw(context.block, later_id)?;
    let mut later_input = match &later_raw {
        Some(raw) => ColumnReader::stored(raw, fields, table, format.version())?,
        None => ColumnReader::empty(fields),
    };
    let decode = |plain: &[u8], keyframe: bool| -> io::Result<TableUpdate> {
        let _cpu = crate::cpu::scope("decode_wire", None);
        let mut input = Decoder::new(plain);
        let update = decode_update(table, change, format, keyframe, &mut input)?;
        input.finish()?;
        Ok(update)
    };
    let layout = context.directory.layout;
    let budget = Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES));
    let mut predictor = predicted::Predictor::default();
    // A keyed block's later entries name rows among the table's rows at the
    // frame before and store masks against the masks before them.
    let mut keys = KeySet::default();
    let mut masks = MaskList::default();
    let modelled = later_raw.is_some() && later_input.modelled_rows(table, layout)?;
    let mut model = modelled.then(|| Model::new(fields));
    let mut changes = KeyChanges::default();
    let mut updates = Vec::with_capacity(frames);
    let from = From {
        position: context.position,
        index: context.index,
        layout,
    };
    let KeyframeUpdate {
        marks,
        update,
        mut plain,
        mut shadow,
        anchored,
    } = keyframe_update(context.replay, &from, table, &key_raw, &budget)?;
    // Whole keyframes seed later row names; anchored changes leave none.
    if !anchored && layout.keyed() {
        keys.advance(table, version, &plain)?;
    }
    updates.push(update);
    advance_shadow(table, version, &plain, &mut shadow)?;
    let rows = match updates.last() {
        Some(TableUpdate::Snapshot(rows)) => Some(rows),
        _ => None,
    };
    settle_advance(table, layout, &mut shadow, rows);
    let first = context.hours[0];
    for frame in 1..frames {
        let previous = context.hours[frame - 1];
        if !get(
            &context.entries[frame * width..(frame + 1) * width],
            ordinal,
        ) {
            if predictor.predicts() {
                // Exactly the predicted changes.
                plain.clear();
                predicted::take_residual(
                    table,
                    fields,
                    previous,
                    None,
                    &mut predictor,
                    &shadow,
                    &mut plain,
                    &budget,
                    layout,
                    None,
                )?;
                updates.push(decode(&plain, false)?);
                advance_shadow(table, version, &plain, &mut shadow)?;
                // Predicted rows changed in the frame before: none is
                // inserted or deleted.
                continue;
            }
            predictor.clear();
            budget.spend(5)?;
            updates.push(match &gap {
                Some(reason) => TableUpdate::Incompatible {
                    reason: reason.clone(),
                    source_format: format,
                    recorded_bytes: 5,
                },
                None => TableUpdate::Unchanged { baseline: previous },
            });
            continue;
        }
        plain.clear();
        let head = take_head(&mut later_input, Some(previous), first, &mut plain, layout)?;
        let coding = layout.keyed().then_some(RowCoding {
            keys: &keys,
            masks: &mut masks,
            model: model.as_mut(),
            changes: Some(&mut changes),
        });
        if head.residual {
            predicted::take_residual(
                table,
                fields,
                previous,
                Some(&mut later_input),
                &mut predictor,
                &shadow,
                &mut plain,
                &budget,
                layout,
                coding,
            )?;
        } else {
            predictor.clear();
            take_body(
                table,
                fields,
                head.status,
                &mut later_input,
                &shadow,
                &mut plain,
                &budget,
                layout,
                coding,
            )?;
        }
        updates.push(decode(&plain, false)?);
        advance_shadow(table, version, &plain, &mut shadow)?;
        if layout.keyed() {
            // A delta or residual entry inserted the rows it stored as
            // distances; any other update names its rows itself.
            if head.status == DELTA {
                keys.apply(&mut changes);
            } else {
                keys.advance(table, version, &plain)?;
            }
        }
    }
    let later_marks = later_input.1;
    if context.complete {
        later_input.finish()?;
    }
    super::aliases::private::resolve(
        table,
        context.hours,
        updates,
        marks.as_deref(),
        later_marks,
        context.complete,
        |source| dependency_updates(context, source),
    )
}

/// A keyframe as a later keyframe's anchor: its rows (none for a table this
/// reader discards) and its byte fields' encodings, the anchored change's
/// patch bases.
struct AnchorState {
    rows: Option<TableRows>,
    shadow: Shadow,
}

fn apply_anchor_change(rows: TableRows, change: TableUpdate) -> io::Result<TableRows> {
    match change {
        TableUpdate::Delta {
            changed, deleted, ..
        } => rows.apply_delta(&changed, &deleted),
        TableUpdate::Unchanged { .. } => Ok(rows),
        _ => Err(invalid_static("keyframe anchor change is not a change")),
    }
}

/// The block whose keyframe entry names an anchor.
struct From<'a> {
    /// Its position in the file's block index, if it is there.
    position: Option<usize>,
    index: &'a BlockIndex,
    layout: Layout,
}

/// One keyframe section of an anchor chain.
struct Link {
    raw: Vec<u8>,
    hour: Hour,
    layout: Layout,
    position: usize,
}

/// A resolved anchor: the keyframe's state, its chain's snapshot, and what
/// reading the keyframe costs beyond that snapshot ([`chain_charge`]).
struct Resolved {
    state: AnchorState,
    /// The snapshot's block position and raw bytes, and its state when asked.
    root: (usize, usize, Option<AnchorState>),
    /// 0 when the anchor is the snapshot itself.
    charge: usize,
}

/// The keyframe of `table` that `from`'s keyframe entry anchors to, an
/// earlier block's, following its own anchors to their snapshot. An anchored
/// block's chain may cross branches and earlier layouts, and with `own`, the
/// raw bytes of the entry naming `anchor`, stays within [`chain_charge`] of
/// its snapshot; an earlier layout's reaches its snapshot within its branch,
/// through at most [`MAX_ANCHOR_DEPTH`] blocks from a predicted block and
/// only a snapshot otherwise. `keep_root` also returns the snapshot's state.
#[allow(clippy::too_many_arguments)]
fn anchored_keyframe(
    replay: &Replay,
    from: &From<'_>,
    table: TableId,
    fields: usize,
    discarded: bool,
    anchor: AnchorRef,
    own: usize,
    format: FormatVersion,
    keep_root: bool,
) -> io::Result<Resolved> {
    let version = format.version();
    let change: &LastChange = &HISTORY[table.index()];
    let decode = |plain: &[u8], keyframe: bool| -> io::Result<TableUpdate> {
        let _cpu = crate::cpu::scope("decode_wire", None);
        let mut input = Decoder::new(plain);
        let update = decode_update(table, change, format, keyframe, &mut input)?;
        input.finish()?;
        Ok(update)
    };
    let depth = match from.layout {
        Layout::Anchored
        | Layout::Keyed
        | Layout::Ramped
        | Layout::Modelled
        | Layout::Trimmed
        | Layout::Advanced
        | Layout::Chosen => usize::MAX,
        Layout::Predicted => MAX_ANCHOR_DEPTH,
        Layout::Unpredicted | Layout::Timed => 1,
    };
    let bound = || invalid_static("keyframe anchor chain exceeds its size bound");
    // The chain's keyframe sections, newest first, ending at its snapshot,
    // and the raw bytes of its anchored entries with the naming entry's.
    let mut chain: Vec<Link> = Vec::new();
    let mut anchored = own;
    let (mut position, mut index, mut anchor) = (from.position, from.index, anchor);
    loop {
        if chain.len() == depth {
            return Err(invalid_static("keyframe anchor is not a snapshot"));
        }
        // Each link is an earlier block of the file than the one naming it.
        let (at, block_index) = match anchor {
            // Counted among the file's blocks, so a block whose index entry
            // opening withheld is never replaced by the one before it.
            AnchorRef::Back(blocks) => position
                .and_then(|position| replay.ordinals[position].checked_sub(blocks))
                .and_then(|ordinal| replay.ordinals.binary_search(&ordinal).ok())
                .map(|at| (at, &replay.index[at])),
            AnchorRef::Hour(hour) => replay
                .index
                .iter()
                .enumerate()
                .find(|(_, block)| block.branch == index.branch && block.first_hour == hour)
                .filter(|(at, _)| {
                    hour < index.first_hour && position.is_none_or(|position| *at < position)
                }),
        }
        .ok_or_else(|| invalid_static("keyframe anchor names no earlier block of its file"))?;
        let directory = Directory::read(replay, block_index)?;
        let raw = directory
            .read_raw(replay, block_index, keyframe_section(table))?
            .ok_or_else(|| invalid_static("keyframe anchor has no entry for its table"))?;
        let mut input = ColumnReader::stored(&raw, fields, table, version)?;
        let head = take_head(
            &mut input,
            None,
            block_index.first_hour,
            &mut Vec::new(),
            directory.layout,
        )?;
        if head.anchor.is_none() && head.status != SNAPSHOT {
            return Err(invalid_static("keyframe anchor is not a snapshot"));
        }
        if head.anchor.is_some() {
            anchored = anchored
                .checked_add(raw.len())
                .filter(|bytes| *bytes <= MAX_BLOCK_RAW_BYTES)
                .ok_or_else(bound)?;
        }
        chain.push(Link {
            raw,
            hour: block_index.first_hour,
            layout: directory.layout,
            position: at,
        });
        match head.anchor {
            Some(next) => (position, index, anchor) = (Some(at), block_index, next),
            None => break,
        }
    }
    let snapshot = chain.last().expect("a chain ends at its snapshot");
    let root = (snapshot.position, snapshot.raw.len());
    // Every anchored entry, the naming one included, also counts its share
    // of rebuilt rows.
    let rebuilds = chain.len() * (root.1 / REBUILD_SHARE);
    let charge = anchored.checked_add(rebuilds).ok_or_else(bound)?;
    if from.layout.anchors_back() && charge > root.1 {
        return Err(bound());
    }
    let mut state: Option<AnchorState> = None;
    let mut root_state = None;
    for link in chain.iter().rev() {
        let mut input = ColumnReader::stored(&link.raw, fields, table, version)?;
        let mut plain = Vec::new();
        let head = take_head(&mut input, None, link.hour, &mut plain, link.layout)?;
        // Each keyframe's rows were bounded by its own block.
        let budget = Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES));
        // The base's patch bases become this keyframe's, never copied.
        let base = state.take();
        let snapshot = base.is_none();
        let (rows, mut shadow) =
            base.map_or((None, Shadow::new()), |base| (base.rows, base.shadow));
        take_body(
            table,
            fields,
            head.status,
            &mut input,
            &shadow,
            &mut plain,
            &budget,
            link.layout,
            None,
        )?;
        input.finish()?;
        advance_shadow(table, version, &plain, &mut shadow)?;
        let rows = match rows {
            _ if discarded => None,
            None if snapshot => match decode(&plain, true)? {
                TableUpdate::Snapshot(rows) => Some(rows),
                _ => return Err(invalid_static("keyframe anchor is not a snapshot")),
            },
            None => None,
            Some(rows) => Some(apply_anchor_change(rows, decode(&plain, false)?)?),
        };
        if snapshot && keep_root {
            root_state = Some(AnchorState {
                rows: rows.as_ref().map(TableRows::try_clone).transpose()?,
                shadow: shadow.clone(),
            });
        }
        state = Some(AnchorState { rows, shadow });
    }
    Ok(Resolved {
        state: state.expect("a chain ends at its snapshot"),
        root: (root.0, root.1, root_state),
        // The anchor's own charge, without the naming entry's.
        charge: charge - own - root.1 / REBUILD_SHARE,
    })
}

/// A section of the block at `position` as stored, and its raw bytes.
#[cfg(test)]
pub(crate) fn stored_section(
    replay: &Replay,
    position: usize,
    section: Section,
) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let index = &replay.index[position];
    let block = replay.block_bytes(index)?;
    let directory = Directory::parse(&block, index)?;
    let id = match section {
        Section::Header => HEADER,
        Section::Diagnostics => DIAGNOSTICS,
        Section::Keyframe(table) => keyframe_section(table),
        Section::Later(table) => later_section(table),
    };
    let (offset, length, _) = directory.sections[id].ok_or_else(|| invalid_static("no section"))?;
    let raw = directory
        .raw(&block, id)?
        .ok_or_else(|| invalid_static("no section"))?;
    Ok((block[offset..offset + length].to_vec(), raw))
}

/// The keyframe sections a seek into the block at `position` reads for
/// `table`: each one's block position and raw bytes, its own first and its
/// snapshot last.
#[cfg(test)]
pub(crate) fn keyframe_chain(
    replay: &Replay,
    position: usize,
    table: TableId,
) -> io::Result<Vec<(usize, usize)>> {
    let version = replay.format.version();
    let fields = table.last_change().fields(table, version);
    let mut chain = Vec::new();
    let mut at = position;
    loop {
        let index = &replay.index[at];
        let block = replay.block_bytes(index)?;
        let directory = Directory::parse(&block, index)?;
        let raw = directory
            .raw(&block, keyframe_section(table))?
            .ok_or_else(|| invalid_static("no keyframe entry"))?;
        let mut input = ColumnReader::stored(&raw, fields, table, version)?;
        let head = take_head(
            &mut input,
            None,
            index.first_hour,
            &mut Vec::new(),
            directory.layout,
        )?;
        chain.push((at, raw.len()));
        at = match head.anchor {
            Some(AnchorRef::Back(blocks)) => replay
                .ordinals
                .binary_search(&(replay.ordinals[at] - blocks))
                .map_err(|_| invalid_static("no anchor block"))?,
            Some(AnchorRef::Hour(hour)) => replay
                .index
                .iter()
                .position(|block| block.branch == index.branch && block.first_hour == hour)
                .ok_or_else(|| invalid_static("no anchor block"))?,
            None => return Ok(chain),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_row_coding_bytes_are_required_and_refused_by_table() {
        let table = TableId::SliceReads;
        let read = |masks: &[u8], layout| {
            let mut columns = Columns::new(1);
            columns.0[MASKS].extend_from_slice(masks);
            let raw = columns.raw();
            ColumnReader::open(&raw, 1)
                .unwrap()
                .modelled_rows(table, layout)
        };
        for code in [2, 255] {
            let error = read(&[code, 0, 1], Layout::Chosen).unwrap_err().to_string();
            assert!(error.contains("unknown grouped row coding byte"), "{error}");
            assert!(error.contains("SliceReads"), "{error}");
        }
        let error = read(&[], Layout::Chosen).unwrap_err().to_string();
        assert!(error.contains("missing grouped row coding byte"), "{error}");
        assert!(error.contains("SliceReads"), "{error}");
        assert!(read(&[MODELLED_ROWS], Layout::Chosen).unwrap());
        assert!(!read(&[LISTED_ROWS], Layout::Chosen).unwrap());
    }

    #[test]
    fn chosen_pairs_keep_listed_insertions_deletions_and_plain_entries_exact() {
        use crate::rows::SliceReadRow;
        let table = TableId::SliceReads;
        let format = crate::FORMAT_VERSION;
        let rows = |owners: &[u32], hour| {
            TableRows::SliceReads(
                owners
                    .iter()
                    .map(|&owner| SliceReadRow {
                        table: TableId::ResearchTechnologies.index() as u32,
                        owner,
                        sample_hour: hour,
                    })
                    .collect(),
            )
        };
        for residual in [false, true] {
            let mut chosen = Later::new(table, 1, Layout::Chosen);
            let mut advanced = Later::new(table, 1, Layout::Advanced);
            let mut listed = Later::new(table, 1, Layout::Ramped);
            let mut predictors =
                std::array::from_fn::<_, 3, _>(|_| predicted::Predictor::default());
            let mut keys = KeySet::default();
            let mut previous = rows(&[1, 3, 5], 0);
            let mut baseline = vec![SNAPSHOT];
            baseline.extend_from_slice(&3_u32.to_le_bytes());
            previous.encode_rows(&mut baseline).unwrap();
            keys.advance(table, format, &baseline).unwrap();
            // Both orderings of deleted keys and changed rows, and a later
            // snapshot whose uncoded keys must also reach the listed pair.
            for (hour, owners, snapshot) in [
                (1_i32, &[1, 2, 5][..], false),
                (2, &[7, 9][..], true),
                (3, &[8, 9][..], false),
            ] {
                let next = rows(owners, hour);
                let mut plain = vec![if snapshot { SNAPSHOT } else { DELTA }];
                if snapshot {
                    plain.extend_from_slice(&(owners.len() as u32).to_le_bytes());
                    next.encode_rows(&mut plain).unwrap();
                } else {
                    plain.extend_from_slice(&(hour - 1).to_le_bytes());
                    assert!(
                        next.encode_changes(&previous, &mut plain, &mut Vec::new())
                            .unwrap()
                    );
                }
                for ((layout, later), predictor) in [
                    (Layout::Chosen, &mut chosen),
                    (Layout::Advanced, &mut advanced),
                    (Layout::Ramped, &mut listed),
                ]
                .into_iter()
                .zip(&mut predictors)
                {
                    let coding = Some(RowCoding {
                        keys: &keys,
                        masks: &mut later.masks,
                        model: later.model.as_mut(),
                        changes: None,
                    });
                    if residual && !snapshot {
                        predicted::put_residual(
                            table,
                            format,
                            1,
                            &plain,
                            predictor,
                            &Shadow::new(),
                            &mut later.columns,
                            layout,
                            coding,
                        )
                        .unwrap();
                    } else {
                        predictor.clear();
                        put_entry(
                            table,
                            format,
                            &plain,
                            Mode::Later { previous: hour - 1 },
                            &Shadow::new(),
                            &mut later.columns,
                            coding,
                        )
                        .unwrap();
                    }
                }
                keys.advance(table, format, &plain).unwrap();
                previous = next;
            }
            chosen.finish();
            advanced.finish();
            assert_eq!(chosen.columns.0[KEYS], advanced.columns.0[KEYS]);
            assert_eq!(&chosen.columns.0[MASKS][1..], advanced.columns.0[MASKS]);
            assert_eq!(
                chosen.columns.1[KEYS].as_ref().unwrap(),
                &listed.columns.0[KEYS]
            );
            assert_eq!(
                &chosen.columns.1[MASKS].as_ref().unwrap()[1..],
                listed.columns.0[MASKS]
            );
            for column in [CONTROL, FIELDS] {
                assert_eq!(chosen.columns.0[column], advanced.columns.0[column]);
                assert_eq!(chosen.columns.0[column], listed.columns.0[column]);
            }
        }
    }

    #[test]
    fn a_source_advances_to_the_head_its_path_dropped_before() {
        let table = TableId::DivisionMovement;
        assert_eq!(advance_pair(table), Some((0, 8)));
        assert_eq!(table.field_name(0), Some("source"));
        assert_eq!(table.field_name(8), Some("path"));
        assert!(matches!(table.schema()[0], Kind::U32));
        assert!(matches!(table.schema()[8], Kind::U32List));
        assert_eq!(advance_pair(TableId::Divisions), None);
        // A snapshot's value, then a delta's difference added to it, or to 0
        // for an inserted row; a value outside u32 is no head.
        assert_eq!(head_value(SNAPSHOT, &7_u32.to_le_bytes(), None).unwrap(), 7);
        let mut change = Vec::new();
        put_varint(&mut change, zigzag(-3));
        let before = 10_u32.to_le_bytes().to_vec();
        assert_eq!(head_value(DELTA, &change, Some(&before)).unwrap(), 7);
        put_varint(&mut change, 0);
        assert!(head_value(DELTA, &change, Some(&before)).is_err());
        let mut inserted = Vec::new();
        put_varint(&mut inserted, zigzag(42));
        assert_eq!(head_value(DELTA, &inserted, None).unwrap(), 42);
        let mut below = Vec::new();
        put_varint(&mut below, zigzag(-11));
        assert!(head_value(DELTA, &below, Some(&before)).is_err());
        // The advance takes the source to the element the path last
        // dropped, only in an advancing block, only for the source, and only
        // once the path has dropped one.
        let mut bases = Shadow::new();
        bases.insert(
            5,
            BTreeMap::from([
                (0, 100_u32.to_le_bytes().to_vec()),
                (DROPPED, 130_u32.to_le_bytes().to_vec()),
            ]),
        );
        let advance =
            |bases: &Shadow, layout, key, field| advance_change(table, layout, bases, key, field);
        assert_eq!(advance(&bases, Layout::Advanced, 5, 0), Some(30));
        assert_eq!(advance(&bases, Layout::Trimmed, 5, 0), None);
        assert_eq!(advance(&bases, Layout::Advanced, 5, 1), None);
        assert_eq!(advance(&bases, Layout::Advanced, 6, 0), None);
        bases.get_mut(&5).unwrap().remove(&DROPPED);
        assert_eq!(advance(&bases, Layout::Advanced, 5, 0), None);
    }

    #[test]
    fn a_first_element_drop_needs_a_trimmed_block_a_list_and_a_head() {
        // DivisionMovement's source (field 0) and path (field 8).
        let table = TableId::DivisionMovement;
        let (source, path) = (0, 8);
        assert_eq!(list_width(table, path), Some(4));
        assert_eq!(list_width(table, source), None);
        let list = |values: &[u32]| {
            let mut bytes = (values.len() as u32).to_le_bytes().to_vec();
            for value in values {
                bytes.extend(value.to_le_bytes());
            }
            bytes
        };
        let budget = Budget(AtomicUsize::new(usize::MAX));
        let take = |field, base: &[u8], layout| {
            let mut values = Vec::new();
            let tag = [DROP_FIRST];
            take_byte_field(
                table,
                field,
                &mut Decoder::new(&tag),
                Some(base),
                None,
                &mut values,
                0,
                &budget,
                layout,
            )
            .map(|_| values)
        };
        let base = list(&[7, 8, 9]);
        assert_eq!(take(path, &base, Layout::Trimmed).unwrap(), list(&[8, 9]));
        assert_eq!(take(path, &list(&[7]), Layout::Trimmed).unwrap(), list(&[]));
        // An empty list has no head; earlier layouts and other kinds refuse it.
        assert!(take(path, &list(&[]), Layout::Trimmed).is_err());
        assert!(take(path, &base, Layout::Modelled).is_err());
        assert!(take(source, &[0; 4], Layout::Trimmed).is_err());
        // The budget counts the rebuilt field, the base less its head: a
        // U32List of three and a U64List of one fit exactly.
        let wings = TableId::ALL
            .into_iter()
            .find_map(|table| {
                let field = table
                    .schema()
                    .iter()
                    .position(|kind| matches!(kind, Kind::U64List))?;
                Some((table, field))
            })
            .unwrap();
        let mut one_wing = 1_u32.to_le_bytes().to_vec();
        one_wing.extend(u64::MAX.to_le_bytes());
        for ((table, field), base, length) in
            [((table, path), base.clone(), 12), (wings, one_wing, 4)]
        {
            for (limit, fits) in [(length, true), (length - 1, false)] {
                let budget = Budget(AtomicUsize::new(limit));
                let mut values = Vec::new();
                let taken = take_byte_field(
                    table,
                    field,
                    &mut Decoder::new(&[DROP_FIRST]),
                    Some(&base),
                    None,
                    &mut values,
                    0,
                    &budget,
                    Layout::Trimmed,
                );
                assert_eq!(taken.is_ok(), fits, "{table:?} at {limit}");
                if fits {
                    assert_eq!(values.len(), length);
                }
            }
        }
        // The writer stores the drop as the tag alone, and only with a width.
        let mut out = Columns::new(table.schema().len());
        let dropped = list(&[8, 9]);
        put_byte_field(
            Some(&dropped),
            Some(&base),
            None,
            &mut out,
            FIELDS + path,
            Some(4),
        )
        .unwrap();
        assert_eq!(out.0[FIELDS + path], [DROP_FIRST]);
        let mut plain = Columns::new(table.schema().len());
        put_byte_field(
            Some(&dropped),
            Some(&base),
            None,
            &mut plain,
            FIELDS + path,
            None,
        )
        .unwrap();
        assert_ne!(plain.0[FIELDS + path], [DROP_FIRST]);
    }

    #[test]
    fn entry_heads_refuse_dependent_keyframes_and_later_anchors() {
        let head_in = |layout, entry: u8, offset: Option<u128>, previous: Option<Hour>| {
            let mut columns = Columns::new(1);
            put_varint(&mut columns.0[CONTROL], u128::from(entry));
            if let Some(offset) = offset {
                put_varint(&mut columns.0[CONTROL], offset);
            }
            let raw = columns.raw();
            let mut input = ColumnReader::open(&raw, 1).unwrap();
            take_head(&mut input, previous, 10, &mut Vec::new(), layout).map(|head| head.anchor)
        };
        let head = |entry, offset, previous| head_in(Layout::Anchored, entry, offset, previous);
        // Earlier layouts name the anchor by its hour in the branch, this
        // one by its block in the file.
        assert_eq!(
            head_in(Layout::Predicted, ANCHOR_UNCHANGED, Some(3), None).unwrap(),
            Some(AnchorRef::Hour(7))
        );
        assert_eq!(
            head(ANCHOR_UNCHANGED, Some(3), None).unwrap(),
            Some(AnchorRef::Back(3))
        );
        assert!(head(ANCHOR_UNCHANGED, Some(0), None).is_err());
        assert!(head_in(Layout::Predicted, ANCHOR_UNCHANGED, Some(0), None).is_err());
        assert!(head(DELTA, Some(0), None).is_err());
        assert!(head(ANCHOR_DELTA, Some(1), Some(9)).is_err());
        assert!(head(8, None, Some(9)).is_err());
        // A residual entry belongs to a predicted block's later entries.
        assert!(head(RESIDUAL, None, Some(9)).unwrap().is_none());
        assert!(head(RESIDUAL, None, None).is_err());
    }

    #[test]
    fn patches_need_their_base_and_must_rebuild_one_field() {
        let table = TableId::LexicalTokens;
        let body = |patch: &[u8], bases: &Shadow| {
            let mut columns = Columns::new(1);
            put_varint(&mut columns.0[CONTROL], 1);
            put_varint(&mut columns.0[CONTROL], 0);
            put_varint(&mut columns.0[KEYS], 3);
            columns.0[MASKS].push(1);
            columns.0[FIELDS].push(1);
            columns.0[FIELDS].extend_from_slice(patch);
            let raw = columns.raw();
            let mut input = ColumnReader::open(&raw, 1).unwrap();
            let budget = Budget(AtomicUsize::new(1 << 20));
            let mut plain = Vec::new();
            take_body(
                table,
                1,
                DELTA,
                &mut input,
                bases,
                &mut plain,
                &budget,
                Layout::Unpredicted,
                None,
            )
            .map(|()| plain)
            .map_err(|error| error.to_string())
        };
        // One hunk replacing the first literal byte.
        let patch = [1, 4, 1, 1, b'y'];
        assert!(
            body(&patch, &Shadow::new())
                .unwrap_err()
                .contains("no base")
        );
        let mut bases = Shadow::new();
        bases
            .entry(3)
            .or_default()
            .insert(0, vec![2, 0, 0, 0, b'x', b'z']);
        let plain = body(&patch, &bases).unwrap();
        assert!(
            plain
                .windows(6)
                .any(|field| field == [2, 0, 0, 0, b'y', b'z']),
            "{plain:?}"
        );
        // A patch that breaks the field's own length framing is refused.
        assert!(body(&[1, 0, 1, 1, 9], &bases).is_err());
    }
}

#[cfg(test)]
#[path = "anchor_read_tests.rs"]
mod anchor_read_tests;
