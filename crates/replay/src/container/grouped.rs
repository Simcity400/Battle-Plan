//! Grouped replay blocks with independently compressed, state-carrying later-hour chunks.

use super::{
    BlockIndex, CompressedBlock, MAX_BLOCK_RAW_BYTES, Parallel, Replay,
    check::Check,
    pack::{self, Cores},
    parallel_tables, patch, zstd_error,
};
use crate::{
    BranchId, Hour,
    codec::{Decoder, invalid, invalid_static, put_varint},
    format::FormatVersion,
    frame::{
        DecodedFrame, Frame, Metrics, READ_TIME_NOT_RECORDED, RecordedFrame, SharedView,
        TABLE_COUNT, TableUpdate, TableView, decode_recorded_update, decode_update,
    },
    rows::{
        Change, Kind, TableId, TableRows, Token,
        compatibility::{HISTORY, LastChange},
    },
    scope::{self, CaptureMode},
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{self, Read},
    sync::atomic::{AtomicUsize, Ordering},
};

pub(super) mod anchors;
#[path = "chunks.rs"]
pub(super) mod chunks;
pub(super) mod cursor;
#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
#[path = "hour_tests.rs"]
mod hour_tests;
#[cfg(test)]
mod reader_chunk_tests;
#[cfg(test)]
#[path = "reader_reference.rs"]
mod reader_reference;

#[cfg(test)]
#[path = "reader_tests.rs"]
pub(super) mod reader_tests;

#[cfg(test)]
mod allocation_tests;
#[cfg(test)]
mod cancellation_tests;
#[cfg(test)]
mod chunk_tests;
mod chunked;
#[cfg(test)]
#[path = "decoded_section_tests.rs"]
mod decoded_section_tests;
mod direct;
#[cfg(test)]
mod direct_tests;
#[cfg(test)]
mod event_feed_tests;
mod group;
mod keyed;
mod modelled;
mod predicted;
#[cfg(any(test, feature = "test-support"))]
mod prefix_fixture;
mod ramp;
mod schedule;
#[cfg(test)]
mod test_recording;
#[cfg(test)]
mod through_tests;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use prefix_fixture::trailing_control;

use crate::frame::recorded::Updates;
use std::sync::{Arc, OnceLock};

use crate::compression::Compression;
use keyed::{KeyChanges, KeyReader, KeySet, KeyWriter, MaskList, RowCoding, RowRun};
use modelled::Model;

pub(crate) const CHOSEN_MAGIC: &[u8; 4] = b"H4GD";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    Chosen,
}

impl Layout {
    fn of(block: &[u8]) -> Option<Self> {
        (block.get(..4)? == CHOSEN_MAGIC).then_some(Self::Chosen)
    }
    fn magic(self) -> &'static [u8; 4] {
        CHOSEN_MAGIC
    }
    fn predicts(self) -> bool {
        true
    }
    fn anchors_back(self) -> bool {
        true
    }
    fn keyed(self) -> bool {
        true
    }
    fn modelled(self) -> bool {
        true
    }
    fn chooses_rows(self) -> bool {
        true
    }
    fn accelerates(self) -> bool {
        false
    }
    fn ramped(self) -> bool {
        true
    }
    fn trims(self) -> bool {
        true
    }
    fn advances(self) -> bool {
        true
    }
}

pub const LATER_CHUNK_HOURS: usize = 168;

/// Whether a block's first bytes name a grouped layout.
pub(crate) fn is_grouped(block: &[u8]) -> bool {
    Layout::of(block).is_some()
}
/// Recordings from this format on write grouped blocks.
pub(crate) const GROUPED_FORMAT: u16 = 124;

const HEADER: usize = 0;
const DIAGNOSTICS: usize = 1;
const SECTIONS: usize = 2 + 2 * TABLE_COUNT;
pub(super) const fn keyframe_section(table: TableId) -> usize {
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
/// The plain update of a table the row frame leaves to the native capture.
const NOT_RECORDED: u8 = 5;
const ANCHOR_DELTA: u8 = 5;
const ANCHOR_UNCHANGED: u8 = 6;
/// A delta against the previous frame, stored against its predictions.
const RESIDUAL: u8 = 7;
/// The entry of a [`NOT_RECORDED`] plain update.
const NOT_RECORDED_ENTRY: u8 = 8;

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

/// Tables a format and capture mode store, in TableId order, and their positions.
struct Present {
    tables: Vec<TableId>,
    ordinal: [usize; TABLE_COUNT],
}

impl Present {
    fn new(format: u16, mode: CaptureMode) -> Self {
        let mut ordinal = [usize::MAX; TABLE_COUNT];
        let tables: Vec<TableId> = TableId::ALL
            .into_iter()
            .filter(|table| HISTORY[table.index()].present(format) && scope::records(mode, *table))
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

/// A diagnostics metric after its recorded zigzag change.
fn metric_after(before: u64, change: u128) -> io::Result<u64> {
    i128::from(before)
        .checked_add(unzigzag(change))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| invalid_static("grouped metric outside its range"))
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
    let mut key = 0;
    let mut counts = 0;
    let mut allocation_failure = None;
    let walked = table.walk_update(plain, format, &mut |token, bytes| match token {
        _ if allocation_failure.is_some() => {}
        Token::Count => counts += 1,
        Token::Key(value) => {
            key = value;
            if counts == 2 {
                shadow.remove(&value);
            }
        }
        Token::Field(field) => {
            if let Err(error) = observe_shadow(table, status, key, field, bytes, shadow) {
                allocation_failure = Some(error);
            }
        }
        _ => {}
    });
    // A failed diagnostic is not an invalid head. Keep that first allocation
    // failure even if the remaining wire framing also refuses the update.
    allocation_failure.map_or(walked, Err)
}

/// Observe one field of a plain update; a direct delta observes its fields
/// only after their entire row has been recovered against the old bases.
/// These are the writer's patch-base rules, including the movement head and
/// dropped-list dependency.
#[inline]
fn observe_shadow(
    table: TableId,
    status: u8,
    key: u128,
    field: usize,
    bytes: &[u8],
    shadow: &mut Shadow,
) -> io::Result<()> {
    let (head, list) = advance_pair(table).unzip();
    match field {
        field if Some(field) == head => {
            // A value the field cannot hold leaves no head, so writer and
            // reader alike store that row's next change without its advance.
            let fields = shadow.entry(key).or_default();
            match head_value(status, bytes, fields.get(&field)) {
                Ok(value) => fields.insert(field, value.to_le_bytes().to_vec()),
                Err(error) if engine_allocation::is_allocation_error(&error) => return Err(error),
                Err(_) => fields.remove(&field),
            };
        }
        field if table.byte_field(field) => {
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
    }
    Ok(())
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
    fn choose(&mut self, level: i32) -> io::Result<Vec<usize>> {
        let mut chosen = Vec::new();
        for (at, (column, differences)) in
            self.0.iter_mut().zip(&mut self.1).enumerate().skip(FIELDS)
        {
            if let Some(differences) = differences.take()
                && differences != *column
                && compress_at(&differences, level)?.len() < compress_at(column, level)?.len()
            {
                *column = differences;
                chosen.push(at);
            }
        }
        Ok(chosen)
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

/// A section's columns, its alias marks, and, for one chunk of a chunked
/// section, the inflated bytes of the chunks after it: at least what each
/// column holds there.
struct ColumnReader<'a>(Vec<Decoder<'a>>, Option<&'a [u8]>, usize);

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
        Ok(Self(columns, None, 0))
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
            0,
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
    fn exhausted(&self) -> bool {
        self.0.iter().all(|column| column.remaining() == 0)
    }
    fn finish(self) -> io::Result<()> {
        if self.exhausted() {
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
        (DELTA, Mode::Back { .. }) => ANCHOR_DELTA,
        (UNCHANGED, Mode::Back { .. }) => ANCHOR_UNCHANGED,
        (DELTA | UNCHANGED, Mode::Plain) => {
            return Err(invalid_static(
                "grouped keyframe entry depends on a baseline",
            ));
        }
        (NOT_RECORDED, _) => NOT_RECORDED_ENTRY,
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
/// bytes so far, and `ahead` at least what `column` holds in later chunks.
#[allow(clippy::too_many_arguments)]
fn take_byte_field(
    table: TableId,
    field: usize,
    column: &mut Decoder<'_>,
    ahead: usize,
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
            // At least the joined column's remainder: a chunk refused by this
            // bound alone falls back to the joined section.
            budget.check((used + base.len() + column.remaining()).saturating_add(ahead))?;
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
            SNAPSHOT | UNAVAILABLE | NOT_RECORDED_ENTRY | ANCHOR_DELTA | ANCHOR_UNCHANGED
        ),
        Some(_) => {
            matches!(
                entry,
                SNAPSHOT | DELTA | UNCHANGED | TRUNCATED | UNAVAILABLE | NOT_RECORDED_ENTRY
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
        NOT_RECORDED_ENTRY => NOT_RECORDED,
        status => status,
    };
    plain.push(status);
    let mut anchor = None;
    if let Some(previous) = previous.filter(|_| matches!(entry, DELTA | UNCHANGED)) {
        let baseline = i128::from(previous).checked_sub(unzigzag(control.varint()?));
        let baseline = hour_from(baseline.unwrap_or(i128::MAX))?;
        plain.extend_from_slice(&baseline.to_le_bytes());
    } else if matches!(entry, ANCHOR_DELTA | ANCHOR_UNCHANGED) {
        let offset = control.varint()?;
        if offset == 0 {
            return Err(invalid_static("keyframe anchor is not an earlier block"));
        }
        {
            // The plain baseline is not read for an anchored change.
            plain.extend_from_slice(&block.to_le_bytes());
            anchor = Some(AnchorRef::Back(usize::try_from(offset).map_err(|_| {
                invalid_static("keyframe anchor is not an earlier block")
            })?));
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
    take_body_checked(
        table,
        fields,
        status,
        input,
        bases,
        plain,
        budget,
        layout,
        coding,
        &mut Check::disabled(),
    )
}

#[allow(clippy::too_many_arguments)]
fn take_body_checked(
    table: TableId,
    fields: usize,
    status: u8,
    input: &mut ColumnReader<'_>,
    bases: &Shadow,
    plain: &mut Vec<u8>,
    budget: &Budget,
    layout: Layout,
    coding: Option<RowCoding<'_>>,
    check: &mut Check<'_>,
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
                check.run()?;
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
                check.run()?;
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
                        table, field, column, input.2, base, None, plain, used, budget, layout,
                    )?;
                }
                budget.check(plain.len() - start)?;
            }
            let mut deleted = coding.as_ref().map(|coding| KeyReader::new(coding.keys));
            let mut previous = 0_u128;
            for _ in 0..count(input, plain)? {
                check.run()?;
                match (&mut deleted, &mut coding) {
                    (Some(run), Some(coding)) => {
                        let (key, _) = run.take(table, &mut input.0[KEYS])?;
                        coding.note_deleted(key)?;
                        put_varint(plain, key - previous);
                        previous = key;
                    }
                    _ => {
                        key(input, plain)?;
                    }
                }
            }
        }
        UNCHANGED | NOT_RECORDED => {}
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
    /// Anchors between this keyframe and its snapshot, 0 for a snapshot.
    depth: usize,
    /// The raw bytes of the snapshot it reaches (its own for a snapshot).
    root: usize,
    /// What reading it costs beyond its snapshot ([`chain_charge`]): 0 for
    /// a snapshot.
    charge: usize,
}

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

/// An unreadable optional anchor is not a candidate; allocation failures are
/// terminal, including native resource/commit exhaustion reported by I/O.
fn anchor_candidate<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if engine_allocation::is_allocation_error(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

impl Anchors {
    /// No candidates yet; the next block is at `position` in the file.
    pub(crate) fn at(position: usize) -> Self {
        Self {
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
        let Some(block) = anchor_candidate(replay.block_bytes(index))? else {
            return Ok(anchors);
        };
        if !is_grouped(&block) {
            return Ok(anchors);
        }
        let Some(directory) = anchor_candidate(Directory::parse(&block, index))? else {
            return Ok(anchors);
        };
        let from = From {
            position: Some(replay.index.len() - 1),
            index,
            layout: directory.layout,
        };
        for table in TableId::ALL {
            if let Some(Some((snapshot, latest))) =
                anchor_candidate(seed_table(replay, &block, &directory, &from, table))?
            {
                anchors.snapshots[table.index()] = snapshot;
                anchors.latest[table.index()] = Some(latest);
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
            ..
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
    cuts: Vec<Vec<usize>>,
    alternate_cuts: Vec<Vec<usize>>,
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
            cuts: Vec::new(),
            alternate_cuts: Vec::new(),
            entries: 0,
            present: vec![true],
            masks: MaskList::default(),
            model: layout.modelled().then(|| Model::new(fields)),
        }
    }
    fn checkpoint(&mut self, slot: usize) {
        let mut cut: Vec<_> = self.columns.0.iter().map(Vec::len).collect();
        // Encoder pending bytes plus its four-byte lookahead equal the decoder cursor.
        cut[MASKS] += self.model.as_ref().map_or(0, Model::waiting);
        let alternate: Vec<_> = self
            .columns
            .1
            .iter()
            .zip(&cut)
            .map(|(column, &length)| column.as_ref().map_or(length, Vec::len))
            .collect();
        while self.cuts.len() <= slot {
            self.cuts.push(
                self.cuts
                    .last()
                    .cloned()
                    .unwrap_or_else(|| vec![0; cut.len()]),
            );
            self.alternate_cuts.push(
                self.alternate_cuts
                    .last()
                    .cloned()
                    .unwrap_or_else(|| vec![0; cut.len()]),
            );
        }
        self.cuts[slot] = cut;
        self.alternate_cuts[slot] = alternate;
    }
    fn raw_bytes(&self) -> usize {
        let bytes = self
            .columns
            .0
            .iter()
            .map(|column| column.len() + 5)
            .sum::<usize>()
            + 5
            + self.cuts.len() * (5 * self.columns.0.len() + 5);
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
    /// Select fields and row codings at the block's compression level.
    fn compress(&mut self, compression: Compression) -> io::Result<(usize, Option<Vec<u8>>)> {
        self.finish();
        for column in self.columns.choose(compression.selection_level())? {
            for (cut, alternate) in self.cuts.iter_mut().zip(&self.alternate_cuts) {
                cut[column] = alternate[column];
            }
        }
        let mut best = {
            let raw = self.columns.raw();
            (raw.len(), compress_later(&raw, self.entries, compression)?)
        };
        if let (Some(keys), Some(masks)) =
            (self.columns.1[KEYS].take(), self.columns.1[MASKS].take())
        {
            let modelled_keys = std::mem::replace(&mut self.columns.0[KEYS], keys);
            let modelled_masks = std::mem::replace(&mut self.columns.0[MASKS], masks);
            let raw = self.columns.raw();
            let compressed = compress_later(&raw, self.entries, compression)?;
            // Modelled wins ties, independently of worker scheduling.
            if compressed.as_ref().map_or(0, Vec::len) < best.1.as_ref().map_or(0, Vec::len) {
                best = (raw.len(), compressed);
                for (cut, alternate) in self.cuts.iter_mut().zip(&self.alternate_cuts) {
                    cut[KEYS] = alternate[KEYS];
                    cut[MASKS] = alternate[MASKS];
                }
            } else {
                self.columns.0[KEYS] = modelled_keys;
                self.columns.0[MASKS] = modelled_masks;
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
    /// Every frame's capture mode.
    mode: CaptureMode,
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
    /// The block's column encoding.
    layout: Layout,
    compression: Compression,
    /// Tests keep every predicting table's residual entries, whichever is
    /// smaller ([`FORCE_RESIDUAL`]).
    force_residual: bool,
}

pub const STORED_LEVEL: i32 = 3;
#[cfg(test)]
const LEVEL: i32 = STORED_LEVEL;

pub(crate) fn compress(raw: &[u8]) -> io::Result<Vec<u8>> {
    compress_at(raw, STORED_LEVEL)
}

/// The window readers accept (`window_log_max`), which every level is held
/// to.
const WINDOW_LOG: u32 = 26;

/// `raw` as one checksummed zstd frame at `level`, its window within
/// [`WINDOW_LOG`]. The compressor knows the size, so it sizes its tables to
/// it, but the frame does not record it: the directory does.
pub(crate) fn compress_at(raw: &[u8], level: i32) -> io::Result<Vec<u8>> {
    with_compressor(level, |compressor| {
        if crate::progress::Progress::current().is_none() {
            return compressor.compress(raw);
        }
        use zstd::zstd_safe::{InBuffer, OutBuffer, ResetDirective, zstd_sys::ZSTD_EndDirective};
        let error = |code| io::Error::other(zstd::zstd_safe::get_error_name(code));
        let context = compressor.context_mut();
        context.reset(ResetDirective::SessionOnly).map_err(error)?;
        context
            .set_pledged_src_size(Some(raw.len() as u64))
            .map_err(error)?;
        let mut compressed = Vec::new();
        engine_allocation::reserve(&mut compressed, zstd::zstd_safe::compress_bound(raw.len()))?;
        let mut scratch = [0; 64 * 1024];
        let mut at = 0;
        loop {
            let end = (at + 64 * 1024).min(raw.len());
            let last = end == raw.len();
            let mut input = InBuffer::around(&raw[at..end]);
            loop {
                let before = input.pos();
                let mut output = OutBuffer::around(&mut scratch[..]);
                let remaining = context
                    .compress_stream2(
                        &mut output,
                        &mut input,
                        if last {
                            ZSTD_EndDirective::ZSTD_e_end
                        } else {
                            ZSTD_EndDirective::ZSTD_e_continue
                        },
                    )
                    .map_err(error)?;
                let count = output.pos();
                compressed.extend_from_slice(&scratch[..count]);
                if input.pos() != before || count != 0 {
                    crate::progress::Progress::completed();
                }
                if input.pos() == end - at && (!last || remaining == 0) {
                    break;
                }
                if input.pos() == before && count == 0 {
                    return Err(io::Error::other("compression made no progress"));
                }
            }
            if last {
                break;
            }
            at = end;
        }
        Ok(compressed)
    })
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

/// Recompress the selected bytes at the requested stored level when smaller.
fn store(compressed: &[u8], raw: usize, compression: Compression) -> io::Result<Option<Vec<u8>>> {
    if compression.level() == compression.selection_level() {
        return Ok(None);
    }
    let mut plain = Vec::with_capacity(raw);
    crate::progress::Reader(zstd::stream::read::Decoder::with_buffer(compressed)?.single_frame())
        .read_to_end(&mut plain)?;
    if plain.len() != raw {
        return Err(invalid_static("chosen section raw length mismatch"));
    }
    let stored = compress_at(&plain, compression.level())?;
    Ok((stored.len() < compressed.len()).then_some(stored))
}

/// A table's later entries compressed, or none without entries.
fn compress_later(
    raw: &[u8],
    entries: usize,
    compression: Compression,
) -> io::Result<Option<Vec<u8>>> {
    if entries == 0 {
        return Ok(None);
    }
    compress_at(raw, compression.selection_level()).map(Some)
}

#[cfg(test)]
thread_local! {
    /// The layout this thread's recordings are written in.
    pub(crate) static WRITTEN_LAYOUT: std::cell::Cell<Layout> =
        const { std::cell::Cell::new(Layout::Chosen) };
    /// Whether this thread's blocks keep each predicting table's residual
    /// entries even where its plain entries compress smaller, so tests of
    /// residual coding do not depend on that choice.
    pub(crate) static FORCE_RESIDUAL: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

// Tests pick layouts per thread; writer threads take their caller's.
#[cfg(test)]
pub(crate) fn written_settings() -> (Layout, bool) {
    (WRITTEN_LAYOUT.get(), FORCE_RESIDUAL.get())
}

#[cfg(test)]
pub(crate) fn write_with((layout, residual): (Layout, bool)) {
    WRITTEN_LAYOUT.set(layout);
    FORCE_RESIDUAL.set(residual);
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
    compression: Compression,
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
    let snapshot_bytes = compress_at(&raw, compression.selection_level())?;
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
    let depth = usize::MAX;
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
            let mode = Mode::Back {
                blocks: position - anchor.block,
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
            let compressed = compress_at(&raw, compression.selection_level())?;
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
    pub(crate) fn work_bytes(&self) -> u64 {
        self.headers as u64 + self.plain.iter().map(|bytes| *bytes as u64).sum::<u64>()
    }

    pub(crate) fn new(frame: &Frame) -> io::Result<Self> {
        #[cfg(test)]
        let layout = WRITTEN_LAYOUT.get();
        #[cfg(not(test))]
        let layout = Layout::Chosen;
        Self::with_layout(frame, layout)
    }

    /// A block in `layout`.
    pub(crate) fn with_layout(frame: &Frame, layout: Layout) -> io::Result<Self> {
        let format = FormatVersion::CURRENT.version();
        let mode = frame.capture_mode()?;
        let present = Present::new(format, mode);
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
        Ok(Self {
            branch: frame.branch,
            first_hour: frame.hour,
            last_hour: frame.hour,
            mode,
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
            compression: pack::compression(),
            #[cfg(test)]
            force_residual: FORCE_RESIDUAL.get(),
            #[cfg(not(test))]
            force_residual: false,
        })
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
            || frame.capture_mode()? != self.mode
        {
            return Err(invalid(
                "block frame branch, keyframe, or hour ordering mismatch",
            ));
        }
        let layout = self.layout();
        let compression = self.compression;
        let format = FormatVersion::CURRENT.version();
        let previous = self.last_hour;
        let first_hour = self.first_hour;
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
                    writer,
                    plain,
                    view,
                    snapshot,
                    latest,
                    layout,
                    first_hour,
                    position,
                    column,
                    compression,
                )
            })?;
        } else {
            // Each table's entries read only its own writer and plain update.
            let tables: Vec<_> = self.tables.iter_mut().zip(plains).collect();
            pack::each(cores, tables, |(writer, plain)| -> io::Result<()> {
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
                let slot = usize::try_from(i64::from(frame.hour) - i64::from(first_hour) - 1)
                    .map_err(|_| invalid_static("later hour precedes its keyframe"))?
                    / LATER_CHUNK_HOURS;
                writer.plain.checkpoint(slot);
                if let Some(later) = &mut writer.predicted {
                    later.checkpoint(slot);
                }
                advance_shadow(table, format, plain, &mut writer.shadow)?;
                if layout.keyed() {
                    writer.keys.advance(table, format, plain)?;
                }
                Ok(())
            })?;
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
    pub(crate) fn compress(self, cores: &Cores) -> io::Result<CompressedBlock> {
        Ok(Compressing::new(self)
            .run(cores, &|| false)?
            .expect("a compression that never stops finishes"))
    }
}

/// A block's compression, which stops between tables and sections and resumes there.
pub(crate) struct Compressing {
    block: BlockWriter,
    chosen: Vec<Option<(usize, Option<Vec<u8>>)>>,
    sections: Vec<(usize, usize, Vec<u8>)>,
    /// Sections not yet tried at the stored level, largest first.
    unstored: Vec<usize>,
}

impl Compressing {
    pub(crate) fn work_bytes(&self) -> u64 {
        if self.sections.is_empty() {
            self.block
                .tables
                .iter()
                .zip(&self.chosen)
                .filter(|(_, chosen)| chosen.is_none())
                .map(|(writer, _)| self.block.plain[writer.table.index()] as u64)
                .sum()
        } else {
            self.unstored
                .iter()
                .map(|at| self.sections[*at].2.len() as u64)
                .sum()
        }
    }

    pub(crate) fn raw_bytes(&self) -> u64 {
        self.block.work_bytes()
    }

    pub(crate) fn new(block: BlockWriter) -> Self {
        let chosen = block.tables.iter().map(|_| None).collect();
        Self {
            block,
            chosen,
            sections: Vec::new(),
            unstored: Vec::new(),
        }
    }

    /// None where `stop` held before every table and section was done.
    pub(crate) fn run(
        &mut self,
        cores: &Cores,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> io::Result<Option<CompressedBlock>> {
        if self.sections.is_empty() {
            let force_residual = self.block.force_residual;
            let tables: Vec<_> = self
                .block
                .tables
                .iter_mut()
                .zip(&mut self.chosen)
                .filter(|(_, chosen)| chosen.is_none())
                .collect();
            pack::each(cores, tables, |(writer, chosen)| {
                if stop() {
                    return Ok(());
                }
                let mut best = writer.plain.compress(self.block.compression)?;
                let mut predicted = false;
                if let Some(later) = &mut writer.predicted {
                    let candidate = later.compress(self.block.compression)?;
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
                *chosen = Some(best);
                Ok(())
            })?;
            if self.chosen.iter().any(Option::is_none) {
                return Ok(None);
            }
            self.sections = self.sections()?;
            self.unstored = (0..self.sections.len())
                .filter(|&at| !self.sections[at].2.is_empty())
                .collect();
            self.unstored
                .sort_by_key(|&at| std::cmp::Reverse(self.sections[at].2.len()));
        }
        let sections = &self.sections;
        let stored = pack::each(cores, std::mem::take(&mut self.unstored), |at| {
            if stop() {
                return Ok((at, None, false));
            }
            let (_, raw, bytes) = &sections[at];
            Ok((at, store(bytes, *raw, self.block.compression)?, true))
        })?;
        for (at, bytes, done) in stored {
            match (bytes, done) {
                (Some(bytes), _) => self.sections[at].2 = bytes,
                (None, false) => self.unstored.push(at),
                (None, true) => {}
            }
        }
        if !self.unstored.is_empty() {
            return Ok(None);
        }
        self.assemble().map(Some)
    }

    /// The header, diagnostics and each table's chosen sections, each at the choosing level.
    fn sections(&mut self) -> io::Result<Vec<(usize, usize, Vec<u8>)>> {
        let block = &mut self.block;
        // Which tables have an entry in each frame after the keyframe, as
        // the change from the frame before.
        let width = block.present.width();
        let mut header = Vec::new();
        let mut previous = vec![0_u8; width];
        for ordinal in 0..block.tables.len() {
            set(&mut previous, ordinal);
        }
        for (frame, gap) in block.gaps.iter().enumerate() {
            put_varint(&mut header, *gap);
            let mut entries = vec![0_u8; width];
            for (ordinal, writer) in block.tables.iter().enumerate() {
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
        block.header = header;
        let mut sections: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        if !block.header.is_empty() {
            sections.push((
                HEADER,
                block.header.len(),
                compress_at(&block.header, block.compression.selection_level())?,
            ));
        }
        sections.push((
            DIAGNOSTICS,
            block.diagnostics.len(),
            compress_at(&block.diagnostics, block.compression.selection_level())?,
        ));
        // Each table's entries are released once its sections are taken.
        for (writer, chosen) in std::mem::take(&mut block.tables)
            .into_iter()
            .zip(&mut self.chosen)
        {
            sections.push((
                keyframe_section(writer.table),
                writer.keyframe_raw,
                writer.keyframe,
            ));
            if let Some((_, Some(_))) = chosen.take() {
                let later = writer.predicted.as_ref().unwrap_or(&writer.plain);
                sections.extend(chunked::split_at(
                    later_section(writer.table),
                    later,
                    block.compression.selection_level(),
                )?);
            }
        }
        Ok(sections)
    }

    fn assemble(&mut self) -> io::Result<CompressedBlock> {
        let block = &self.block;
        let sections = &self.sections;
        let mut directory = Vec::new();
        put_varint(&mut directory, u128::from(block.frames));
        let count = sections
            .iter()
            .enumerate()
            .filter(|(at, section)| *at == 0 || sections[*at - 1].0 != section.0)
            .count();
        put_varint(&mut directory, count as u128);
        let mut raw_length = 0_usize;
        let mut at = 0;
        while at < sections.len() {
            let id = sections[at].0;
            let end = at
                + sections[at..]
                    .iter()
                    .take_while(|section| section.0 == id)
                    .count();
            let group = &sections[at..end];
            let raw: usize = group.iter().map(|section| section.1).sum();
            let stored: usize = group.iter().map(|section| section.2.len()).sum();
            put_varint(&mut directory, id as u128);
            put_varint(&mut directory, raw as u128);
            put_varint(&mut directory, stored as u128);
            if id >= 3 && id % 2 == 1 {
                for (_, raw, bytes) in group {
                    put_varint(&mut directory, bytes.len() as u128);
                    put_varint(&mut directory, *raw as u128);
                }
            }
            raw_length = raw_length
                .checked_add(raw)
                .ok_or_else(|| invalid_static("block raw length overflow"))?;
            at = end;
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
        bytes.extend_from_slice(block.layout().magic());
        bytes.extend_from_slice(&crc32fast::hash(&directory).to_le_bytes());
        bytes.extend_from_slice(&directory);
        for (_, _, section) in sections {
            bytes.extend_from_slice(section);
        }
        Ok(CompressedBlock {
            branch: block.branch,
            first_hour: block.first_hour,
            last_hour: block.last_hour,
            raw_length: raw_length as u64,
            bytes,
        })
    }
}

pub(super) fn source_digest(
    replay: &Replay,
    index: &BlockIndex,
    meter: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    check: &mut Check<'_>,
) -> io::Result<blake3::Hash> {
    use super::reader::source::Evidence;
    let mut evidence = Evidence::new(SECTIONS + 1, std::sync::Arc::clone(meter))?;
    let directory = Directory::read_admitted(
        replay,
        index,
        Directory::prefix(replay, index)?,
        &mut |offset, bytes| evidence.record(0, offset, bytes, check),
    )?;
    for (id, section) in directory.sections.iter().enumerate() {
        if let Some((offset, length, _)) = *section {
            evidence.read(id + 1, replay, index, offset, length, check)?;
        }
    }
    Ok(evidence.digest())
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

pub(super) struct Directory {
    frames: usize,
    layout: Layout,
    /// By section id: offset in the block, compressed length, raw length.
    sections: Vec<Option<(usize, usize, usize)>>,
    chunks: Vec<Vec<(usize, usize, usize)>>,
    read_bytes: usize,
}

// The wire decoder admits unsigned u128 varints, at most 19 bytes each.
const MAX_VARINT_BYTES: usize = 19;

impl Directory {
    fn parse(block: &[u8], index: &BlockIndex) -> io::Result<Self> {
        Self::parse_extent(block, block.len(), index)
    }

    pub(super) fn read(replay: &Replay, index: &BlockIndex) -> io::Result<Self> {
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
        bytes: Cow<'a, [u8]>,
    ) -> io::Result<Self> {
        Self::read_admitted(replay, index, bytes, &mut |_, _| Ok(()))
    }

    fn read_admitted<'a>(
        replay: &'a Replay,
        index: &BlockIndex,
        mut bytes: Cow<'a, [u8]>,
        admit: &mut dyn FnMut(usize, &[u8]) -> io::Result<()>,
    ) -> io::Result<Self> {
        let length = super::to_usize(index.length)?;
        let read_bytes = if replay.is_file_backed() {
            let mut input = Decoder::new(&bytes);
            let (_, _, _, count) = Self::counts(&mut input, index)?;
            let mut position = bytes.len() - input.remaining();
            let slots = chunked::slots(index)?;
            let bytes = bytes.to_mut();
            for _ in 0..count {
                let id = Self::read_varint(replay, index, bytes, &mut position)?;
                Self::read_varint(replay, index, bytes, &mut position)?;
                Self::read_varint(replay, index, bytes, &mut position)?;
                if id >= 3 && id % 2 == 1 {
                    if slots > length.saturating_sub(position) / 2 {
                        return Err(invalid_static("invalid grouped chunk index extent"));
                    }
                    for _ in 0..slots {
                        Self::read_varint(replay, index, bytes, &mut position)?;
                        Self::read_varint(replay, index, bytes, &mut position)?;
                    }
                }
            }
            bytes.len()
        } else {
            bytes = replay.block_bytes(index)?;
            0
        };
        let mut directory = Self::parse_extent(&bytes, length, index)?;
        directory.read_bytes = read_bytes;
        let end = directory
            .sections
            .iter()
            .flatten()
            .map(|section| section.0)
            .min()
            .unwrap_or(length);
        admit(0, &bytes[..end])?;
        Ok(directory)
    }

    fn read_varint(
        replay: &Replay,
        index: &BlockIndex,
        bytes: &mut Vec<u8>,
        position: &mut usize,
    ) -> io::Result<usize> {
        let start = *position;
        for _ in 0..MAX_VARINT_BYTES {
            if *position == bytes.len() {
                let remaining = super::to_usize(index.length)?.saturating_sub(bytes.len());
                if remaining == 0 {
                    return Err(invalid_static("truncated grouped directory"));
                }
                let more = remaining.min(4096);
                engine_allocation::reserve(bytes, more)?;
                let offset = index
                    .offset
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| invalid_static("block offset overflow"))?;
                let end = bytes.len() + more;
                bytes.resize(end, 0);
                replay.read_at(offset, &mut bytes[end - more..])?;
            }
            let byte = bytes[*position];
            *position += 1;
            if byte & 0x80 == 0 {
                return varint_usize(&mut Decoder::new(&bytes[start..*position]));
            }
        }
        Err(invalid_static("varint exceeds u128"))
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
        if frames == 0 || !i64::try_from(frames).is_ok_and(|frames| frames <= span) {
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
        let slots = chunked::slots(index)?;
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
            let mut chunks = Vec::new();
            if id >= 3 && id % 2 == 1 {
                if slots == 0 || slots > input.remaining() / 2 {
                    return Err(invalid_static("invalid grouped chunk index extent"));
                }
                let (mut stored_total, mut raw_total) = (0_usize, 0_usize);
                for _ in 0..slots {
                    let stored = varint_usize(&mut input)?;
                    let plain = varint_usize(&mut input)?;
                    if (stored == 0) != (plain == 0) {
                        return Err(invalid_static("empty chunk index entry has nonzero length"));
                    }
                    stored_total = stored_total
                        .checked_add(stored)
                        .ok_or_else(|| invalid_static("chunk index stored length overflow"))?;
                    raw_total = raw_total
                        .checked_add(plain)
                        .ok_or_else(|| invalid_static("chunk index raw length overflow"))?;
                    if raw_total > MAX_BLOCK_RAW_BYTES {
                        return Err(invalid_static("chunk index exceeds raw size limit"));
                    }
                    chunks.push((stored, plain));
                }
                if (stored_total, raw_total) != (length, raw) {
                    return Err(invalid_static("chunk index lengths disagree with section"));
                }
            }
            entries.push((id, raw, length, chunks));
        }
        let end = bytes.len() - input.remaining();
        if crc32fast::hash(&bytes[8..end]) != crc {
            return Err(invalid_static("grouped block directory CRC mismatch"));
        }
        let mut sections = vec![None; SECTIONS];
        let mut offset = end;
        let mut raw_total = 0_usize;
        let mut chunks = vec![Vec::new(); SECTIONS];
        for (id, raw, length, sizes) in entries {
            let next = offset
                .checked_add(length)
                .filter(|next| *next <= block_length)
                .ok_or_else(|| invalid_static("grouped section outside its block"))?;
            raw_total = raw_total
                .checked_add(raw)
                .ok_or_else(|| invalid_static("block raw length mismatch"))?;
            sections[id] = Some((offset, length, raw));
            let mut chunk_offset = offset;
            for (stored, plain) in sizes {
                chunks[id].push((chunk_offset, stored, plain));
                chunk_offset += stored;
            }
            offset = next;
        }
        if offset != block_length {
            return Err(invalid_static(
                "grouped block has trailing or concatenated bytes",
            ));
        }
        if raw_total > MAX_BLOCK_RAW_BYTES || raw_total as u64 != index.raw_length {
            return Err(invalid_static("block raw length mismatch"));
        }
        Ok(Self {
            frames,
            layout,
            sections,
            chunks,
            read_bytes: 0,
        })
    }

    pub(super) fn chunk_extents(&self, section: Section) -> Vec<(usize, usize, usize)> {
        let id = match section {
            Section::Header => HEADER,
            Section::Diagnostics => DIAGNOSTICS,
            Section::Keyframe(table) => keyframe_section(table),
            Section::Later(table) => later_section(table),
        };
        if self.chunks[id].is_empty() {
            self.sections[id].into_iter().collect()
        } else {
            self.chunks[id].clone()
        }
    }

    /// Whether section `id` is stored, in chunks.
    fn chunked(&self, id: usize) -> bool {
        self.sections[id].is_some() && !self.chunks[id].is_empty()
    }

    fn chunks<'a>(&'a self, block: &'a [u8], id: usize) -> chunked::Chunks<'a> {
        chunked::Chunks::new(block, &self.chunks[id])
    }

    /// [`Self::raw`]'s inflation and checks of section `id`, holding one
    /// chunk of a chunked section at a time.
    fn verify_raw(&self, block: &[u8], id: usize, check: &mut Check<'_>) -> io::Result<()> {
        let Some((offset, length, raw)) = self.sections[id] else {
            return Ok(());
        };
        if !self.chunks[id].is_empty() {
            let mut chunks = self.chunks(block, id);
            chunks.first_checked(check)?;
            while chunks.next_checked(check)?.is_some() {}
            return Ok(());
        }
        // Checked through the scratch buffer alone: the inflated bytes are not kept.
        Self::inflate_checked(
            &block[offset..offset + length],
            raw,
            &mut Vec::new(),
            false,
            check,
        )
    }

    fn raw(&self, block: &[u8], id: usize) -> io::Result<Option<Vec<u8>>> {
        let Some((offset, length, raw)) = self.sections[id] else {
            return Ok(None);
        };
        if !self.chunks[id].is_empty() {
            return chunked::join(self.chunks[id].iter().map(|&(offset, length, raw)| {
                if length == 0 {
                    Ok(Vec::new())
                } else {
                    Self::inflate(&block[offset..offset + length], raw)
                }
            }))
            .map(Some);
        }
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
        if !self.chunks[id].is_empty() {
            return chunked::join(self.chunks[id].iter().map(|&(offset, length, raw)| {
                if length == 0 {
                    Ok(Vec::new())
                } else {
                    Self::inflate(&replay.block_range(index, offset, length)?, raw)
                }
            }))
            .map(Some);
        }
        Self::inflate(&replay.block_range(index, offset, length)?, raw).map(Some)
    }

    fn inflate(compressed: &[u8], raw: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        Self::inflate_into(compressed, raw, &mut bytes)?;
        Ok(bytes)
    }

    pub(super) fn inflate_into(
        compressed: &[u8],
        raw: usize,
        bytes: &mut Vec<u8>,
    ) -> io::Result<()> {
        Self::inflate_checked(compressed, raw, bytes, true, &mut Check::disabled())
    }

    pub(super) fn inflate_checked(
        compressed: &[u8],
        raw: usize,
        bytes: &mut Vec<u8>,
        keep: bool,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        bytes.clear();
        if raw > MAX_BLOCK_RAW_BYTES {
            return Err(invalid_static("chunk exceeds raw size limit"));
        }
        let _cpu = crate::cpu::scope("inflate", None);
        check.run()?;
        if compressed.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd]) {
            return Err(invalid_static("grouped section is not a zstd frame"));
        }
        if compressed
            .get(4)
            .is_none_or(|descriptor| descriptor & 0x04 == 0)
        {
            return Err(invalid_static(
                "grouped section is missing its zstd checksum",
            ));
        }
        let frame_size = zstd::zstd_safe::find_frame_compressed_size(compressed)
            .map_err(|_| invalid_static("invalid zstd frame"))?;
        if frame_size != compressed.len() {
            return Err(invalid_static(
                "grouped section contains trailing or concatenated zstd frames",
            ));
        }
        let mut decoder = zstd::stream::read::Decoder::with_buffer(compressed)
            .map_err(zstd_error)?
            .single_frame();
        decoder.window_log_max(26).map_err(zstd_error)?;
        if keep {
            engine_allocation::reserve(bytes, raw)?;
        }
        let mut scratch = [0; 64 * 1024];
        let mut total = 0usize;
        loop {
            check.run()?;
            let count = decoder.read(&mut scratch).map_err(zstd_error)?;
            if count == 0 {
                break;
            }
            crate::progress::Progress::completed();
            total = total
                .checked_add(count)
                .filter(|&n| n <= raw)
                .ok_or_else(|| invalid_static("grouped section raw length mismatch"))?;
            if keep {
                bytes.extend_from_slice(&scratch[..count]);
            }
        }
        if total != raw {
            return Err(invalid_static("grouped section raw length mismatch"));
        }
        check.run()
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
    verify_with(block, index, &mut Check::disabled())
}

pub(super) fn verify_with(
    block: &[u8],
    index: &BlockIndex,
    check: &mut Check<'_>,
) -> io::Result<()> {
    let _cpu = crate::cpu::scope("decode", None);
    let directory = Directory::parse(block, index)?;
    for id in 0..SECTIONS {
        #[cfg(test)]
        let _step = super::check::Scope::enter(super::check::Step::Verify(id));
        check.run()?;
        directory.verify_raw(block, id, check)?;
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
        let section = if id % 2 == 0 {
            Section::Keyframe(TableId::ALL[(id - 2) / 2])
        } else {
            Section::Later(TableId::ALL[(id - 3) / 2])
        };
        let mut columns: Vec<ColumnSize> = Vec::new();
        for (offset, stored, raw) in directory.chunk_extents(section) {
            if stored == 0 {
                continue;
            }
            let raw = Directory::inflate(&block[offset..offset + stored], raw)?;
            let slices = chunked::columns(&raw)?;
            if columns.is_empty() {
                columns.resize(slices.len(), ColumnSize { raw: 0, alone: 0 });
            }
            if columns.len() != slices.len() {
                return Err(invalid_static("chunk column counts disagree"));
            }
            for (size, column) in columns.iter_mut().zip(slices) {
                size.raw += column.len() as u64;
                if !column.is_empty() {
                    size.alone += compress(column)?.len() as u64;
                }
            }
        }
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
    for (_id, size) in ids.zip(sizes) {
        let mut section = SectionLevels {
            section: size.section,
            compressed: size.compressed,
            levels: Vec::with_capacity(levels.len()),
            elapsed: Vec::with_capacity(levels.len()),
        };
        for &level in levels {
            let started = std::time::Instant::now();
            let mut stored = 0_u64;
            for (offset, length, raw) in directory.chunk_extents(size.section) {
                if length == 0 {
                    continue;
                }
                let raw = Directory::inflate(&block[offset..offset + length], raw)?;
                stored += compress_at(&raw, level)?.len() as u64;
            }
            section.levels.push(stored);
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
    /// The read's alias group, resolved by the first member read.
    group: &'a OnceLock<group::Group>,
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
        let _consumer = self.section_block_reader(
            self.block_position(index).unwrap_or(0),
            Some(tables),
            "read",
        );
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
        let present = Present::new(self.format.version(), self.capture_mode);
        check_header_size(directory.frames, &present)?;
        let header = directory.read_raw(self, index, HEADER)?.unwrap_or_default();
        read_header(&header, index, directory.frames, &present)?;
        let diagnostics = directory
            .read_raw(self, index, DIAGNOSTICS)?
            .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
        let metrics = read_diagnostics_prefix(&diagnostics, directory.frames, &present, 1)?
            .pop()
            .expect("one keyframe's metrics");
        let from = From {
            position: self.block_position(index),
            index,
            layout: directory.layout,
        };
        let memo = engine_allocation::collect_vec(TableId::ALL.iter().map(|_| OnceLock::new()))?;
        let updates = engine_allocation::collect_vec(TableId::ALL.iter().map(|&table| {
            if let Some(update) = keyframe_refusal(
                &directory,
                self.format,
                self.capture_mode,
                Some(tables),
                table,
            )? {
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

    /// Shared selected updates with cooperative cancellation inside table resolution.
    pub fn read_block_recorded_selected_with(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<RecordedFrame>> {
        self.read_block_recorded_inner_checked(
            index,
            tables,
            index.last_hour,
            None,
            &mut Check::new(&mut check),
        )
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
        self.read_block_recorded_inner_checked(
            index,
            tables,
            through,
            parallel,
            &mut Check::disabled(),
        )
    }

    fn read_block_recorded_inner_checked(
        &self,
        index: &BlockIndex,
        tables: &[TableId],
        through: Hour,
        parallel: Option<&Parallel<'_>>,
        check: &mut Check<'_>,
    ) -> io::Result<Vec<RecordedFrame>> {
        check.run()?;
        let _consumer = self.section_block_reader(
            self.block_position(index).unwrap_or(0),
            Some(tables),
            "read",
        );
        let block = self.grouped_block(index)?;
        let frames = read_recorded_through_checked(
            self,
            index,
            &block,
            Some(tables),
            parallel,
            Some(through),
            check,
        )?;
        let mut order = super::BlockOrder::new(index);
        for frame in &frames {
            check.run()?;
            order.next(frame)?;
        }
        if frames
            .last()
            .is_some_and(|frame| frame.hour == index.last_hour)
        {
            order.finish()?;
            if !self.is_verified(index) {
                verify_with(&block, index, check)?;
            }
            check.run()?;
            self.mark_verified(index);
        }
        Ok(frames)
    }

    /// Read selected tables through `through`, including an equal hour.
    /// Grouped reads validate the complete directory, header and diagnostics,
    /// every inflated section's framing and checksum, selected keyframes and
    /// anchors, and the decoded prefix's ordinary wire grammar and budgets.
    /// Later frames' column bytes have not been decoded or grammar-verified;
    /// an early read does not mark the block verified. A whole-block read
    /// retains ordinary full verification. With `parallel`, tables decode on
    /// the caller's workers, in table order.
    pub fn read_block_selected_through(
        &self,
        index: &BlockIndex,
        tables: Option<&[TableId]>,
        through: Hour,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<Frame>> {
        self.read_block_selected_through_checked(
            index,
            tables,
            through,
            parallel,
            &mut Check::disabled(),
        )
    }

    /// Selected prefix read with cooperative cancellation inside table resolution.
    pub fn read_block_selected_through_with(
        &self,
        index: &BlockIndex,
        tables: Option<&[TableId]>,
        through: Hour,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<Frame>> {
        self.read_block_selected_through_checked(
            index,
            tables,
            through,
            None,
            &mut Check::new(&mut check),
        )
    }

    pub(super) fn read_block_selected_through_checked(
        &self,
        index: &BlockIndex,
        tables: Option<&[TableId]>,
        through: Hour,
        parallel: Option<&Parallel<'_>>,
        check: &mut Check<'_>,
    ) -> io::Result<Vec<Frame>> {
        check.run()?;
        let block = self.grouped_block(index)?;
        let frames = read_recorded_through_checked(
            self,
            index,
            &block,
            tables,
            parallel,
            Some(through),
            check,
        )?;
        let frames = frames
            .into_iter()
            .map(|frame| {
                check.run()?;
                frame.into_owned_frame()
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut order = super::BlockOrder::new(index);
        for frame in &frames {
            check.run()?;
            order.next(frame)?;
        }
        if frames
            .last()
            .is_some_and(|frame| frame.hour == index.last_hour)
        {
            order.finish()?;
            if tables.is_some() && !self.is_verified(index) {
                verify_with(&block, index, check)?;
            }
            check.run()?;
            self.mark_verified(index);
        }
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
            #[cfg(any(test, feature = "test-support"))]
            replay.decoded_sections.wire_decode(
                from.position,
                table,
                1,
                super::decoded_sections::WireRoute::Keyframe,
            );
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
                &mut Check::disabled(),
            )?;
            super::aliases::private::resolve(
                table,
                &[from.index.first_hour],
                vec![keyframe.update],
                keyframe.marks.as_deref(),
                None,
                true,
                |source| {
                    if let Some(update) = keyframe_refusal(
                        directory,
                        replay.format,
                        replay.capture_mode,
                        None,
                        source,
                    )? {
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
    read_recorded_through_checked(
        replay,
        index,
        block,
        selected,
        parallel,
        through,
        &mut Check::disabled(),
    )
}

fn read_recorded_through_checked(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: Option<&[TableId]>,
    parallel: Option<&Parallel<'_>>,
    through: Option<Hour>,
    check: &mut Check<'_>,
) -> io::Result<Vec<RecordedFrame>> {
    let _consumer =
        replay.section_block_reader(replay.block_position(index).unwrap_or(0), selected, "read");
    let _cpu = crate::cpu::scope("decode", None);
    let format = replay.format;
    if format.version() < GROUPED_FORMAT {
        return Err(invalid(format!(
            "grouped block in a format {} recording",
            format.version()
        )));
    }
    let directory = Directory::parse(block, index)?;
    let present = Present::new(format.version(), replay.capture_mode);
    let frames = directory.frames;
    check_header_size(frames, &present)?;
    let header = directory.raw(block, HEADER)?.unwrap_or_default();
    let (hours, entries) = read_header_with(&header, index, frames, &present, check)?;
    let keep = through.map_or(frames, |through| {
        hours.partition_point(|hour| *hour <= through)
    });
    let diagnostics = directory
        .raw(block, DIAGNOSTICS)?
        .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
    let metrics = read_diagnostics_prefix_with(&diagnostics, frames, &present, keep, check)?;
    if keep == 0 {
        return Ok(Vec::new());
    }
    let hours = &hours[..keep];
    let memo = engine_allocation::collect_vec(TableId::ALL.iter().map(|_| OnceLock::new()))?;
    let group = OnceLock::new();
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
        group: &group,
    };
    let decode = |table: &TableId| table_updates(&context, *table, &mut Check::disabled());
    let decoded = match parallel {
        // Start alias sources before their readers, then the heaviest remaining tables.
        Some(parallel) => {
            let order = replay
                .decoded_sections
                .with_largest(schedule::table_order)?;
            parallel_tables(&TableId::ALL, &order, parallel, decode)?
        }
        None => {
            let mut decoded = engine_allocation::vec(TableId::ALL.len())?;
            for table in TableId::ALL {
                check.run()?;
                let result = match table_updates(&context, table, check) {
                    Err(error) if check.failed() => return Err(error),
                    result => result,
                };
                decoded.push(result);
            }
            decoded
        }
    };
    let columns = decoded.into_iter().collect::<io::Result<Vec<_>>>()?;
    let slots = Arc::new(std::array::from_fn(Some));
    let unselected = Arc::new(TableUpdate::Unavailable {
        reason: String::new(),
    });
    let mut out = engine_allocation::vec(keep)?;
    for (position, (hour, metrics)) in hours.iter().zip(metrics).enumerate() {
        check.run()?;
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
/// where [`read`] fails. `check` stops header, table and alias-hour work on its first error.
pub(crate) fn read_selected_updates(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: &[TableId],
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<SelectedUpdates> {
    read_selected_updates_inner(replay, index, block, selected, None, &mut Check::new(check))
}

pub(crate) fn read_selected_updates_coarse(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: &[TableId],
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<SelectedUpdates> {
    read_selected_updates_inner(
        replay,
        index,
        block,
        selected,
        Some(check),
        &mut Check::disabled(),
    )
}

fn read_selected_updates_inner(
    replay: &Replay,
    index: &BlockIndex,
    block: &[u8],
    selected: &[TableId],
    mut table_check: Option<&mut dyn FnMut() -> io::Result<()>>,
    check: &mut Check<'_>,
) -> io::Result<SelectedUpdates> {
    let _consumer = replay.section_block_reader(
        replay.block_position(index).unwrap_or(0),
        Some(selected),
        "read",
    );
    let _cpu = crate::cpu::scope("decode", None);
    let format = replay.format;
    if format.version() < GROUPED_FORMAT {
        return Err(invalid(format!(
            "grouped block in a format {} recording",
            format.version()
        )));
    }
    let directory = Directory::parse(block, index)?;
    let present = Present::new(format.version(), replay.capture_mode);
    let frames = directory.frames;
    check_header_size(frames, &present)?;
    let header = directory.raw(block, HEADER)?.unwrap_or_default();
    let (hours, entries) = read_header_with(&header, index, frames, &present, check)?;
    let diagnostics = directory
        .raw(block, DIAGNOSTICS)?
        .ok_or_else(|| invalid_static("grouped block has no diagnostics"))?;
    let metrics = read_diagnostics_prefix_with(&diagnostics, frames, &present, frames, check)?;
    let memo = engine_allocation::collect_vec(TableId::ALL.iter().map(|_| OnceLock::new()))?;
    let group = OnceLock::new();
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
        group: &group,
    };
    let mut updates: Vec<Option<Updates>> =
        engine_allocation::collect_vec(selected.iter().map(|_| None))?;
    let mut dependent_keyframe = false;
    for table in TableId::ALL {
        if let Some(table_check) = &mut table_check {
            table_check()?;
        }
        check.run()?;
        let wanted = selected.iter().position(|selected| *selected == table);
        // One selection-aware admission per table. A present layout gap
        // still reaches the ordinary decoder even when not selected.
        let refusal = keyframe_refusal(
            &directory,
            format,
            replay.capture_mode,
            Some(selected),
            table,
        )?;
        let decoded = match (wanted, refusal) {
            (None, Some(_)) => continue,
            (Some(_), Some(update)) => Updates::repeat(update, frames)?,
            (_, None) => table_updates_admitted(&context, table, check)?,
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
    let bytes = frames
        .checked_mul(4 + present.plain_header_bytes())
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or_else(|| invalid_static("block exceeds raw size limit"))?;
    Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES)).spend(bytes)
}

fn read_header(
    header: &[u8],
    index: &BlockIndex,
    frames: usize,
    present: &Present,
) -> io::Result<(Vec<Hour>, Vec<u8>)> {
    read_header_with(header, index, frames, present, &mut Check::disabled())
}

fn read_header_with(
    header: &[u8],
    index: &BlockIndex,
    frames: usize,
    present: &Present,
    check: &mut Check<'_>,
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
        check.run()?;
        let gap = i128::try_from(input.varint()?).unwrap_or(i128::MAX);
        let hour = (i128::from(hours[frame - 1]) + 1).checked_add(gap);
        hours.push(hour_from(hour.unwrap_or(i128::MAX))?);
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
    keep: usize,
) -> io::Result<Vec<Metrics>> {
    read_diagnostics_prefix_with(raw, frames, present, keep, &mut Check::disabled())
}

fn read_diagnostics_prefix_with(
    raw: &[u8],
    frames: usize,
    present: &Present,
    keep: usize,
    check: &mut Check<'_>,
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
        check.run()?;
        let total_ns = u64_from(i128::try_from(input.varint()?).unwrap_or(-1))?;
        pool = metric_after(pool, input.varint()?)?;
        dropped = metric_after(dropped, input.varint()?)?;
        let bits = input.take(width)?;
        for (byte, change) in reads.iter_mut().zip(bits) {
            *byte ^= change;
        }
        if tail != 0 && reads[width - 1] >> tail != 0 {
            return Err(invalid_static("grouped read bits name an absent table"));
        }
        let mut table_ns = [0; TABLE_COUNT];
        if frame < keep {
            for (ordinal, table) in present.tables.iter().enumerate() {
                if get(&reads, ordinal) {
                    table_ns[table.index()] = READ_TIME_NOT_RECORDED;
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
    mode: CaptureMode,
    selected: Option<&[TableId]>,
    table: TableId,
) -> io::Result<Option<TableUpdate>> {
    #[cfg(test)]
    decoded_section_tests::admitted(table);
    let version = format.version();
    let change = &HISTORY[table.index()];
    let key_id = keyframe_section(table);
    let later_id = later_section(table);
    let stored = directory.sections[key_id].is_some() || directory.sections[later_id].is_some();
    if change.present(version) && !scope::records(mode, table) {
        if stored {
            return Err(invalid_static(
                "grouped block stores a table its capture mode leaves out",
            ));
        }
        return Ok(Some(TableUpdate::NotRecorded { mode }));
    }
    if !change.present(version) {
        if stored {
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
    check: &mut Check<'_>,
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
    let mode = replay.capture_mode;
    let decode = |plain: &[u8], keyframe: bool| -> io::Result<TableUpdate> {
        let _cpu = crate::cpu::scope("decode_wire", None);
        let mut input = Decoder::new(plain);
        let update = decode_recorded_update(table, change, format, mode, keyframe, &mut input)?;
        input.finish()?;
        Ok(update)
    };
    let head = take_head(&mut key_input, None, first, &mut plain, layout)?;
    let update = if let Some(anchor) = head.anchor {
        let own = raw.len();
        let anchor = anchored_keyframe_with(
            replay, from, table, fields, discarded, anchor, own, format, false, check,
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

fn table_updates(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> io::Result<Updates> {
    #[cfg(test)]
    let _step = super::check::Scope::enter(super::check::Step::Table(table));
    check.run()?;
    // Unselected tables keep their own compatibility and framing checks.
    if let Some(update) = keyframe_refusal(
        context.directory,
        context.format,
        context.replay.capture_mode,
        context.selected,
        table,
    )? {
        return Updates::repeat(update, context.hours.len());
    }
    table_updates_admitted(context, table, check)
}

// The cached decoder receives only admitted sections. No selection-dependent
// value can enter the cache; selected tables and unselected layout gaps use
// this identical decoder. Selection below only attributes CPU measurements.
fn dependency_updates(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> io::Result<Updates> {
    if let Some(update) = keyframe_refusal(
        context.directory,
        context.format,
        context.replay.capture_mode,
        None,
        table,
    )? {
        return Updates::repeat(update, context.hours.len());
    }
    table_updates_admitted(context, table, check)
}

fn table_updates_admitted(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> io::Result<Updates> {
    check.run()?;
    if let Some(updates) = context.memo[table.index()].get() {
        return Ok(updates.clone());
    }
    let key = super::decoded_sections::Key::new(context.replay, context.index, table);
    let cache = &context.replay.decoded_sections;
    let updates = if group::resolves(context, table) {
        cache.read_updates(key, context.hours.len(), false, || {
            let _cpu = crate::cpu::scope("decode_alias_group", None);
            let started = crate::cpu::thread_cycles();
            let column = (|| {
                let column = group_column(context, table, check)?;
                check.run()?;
                Ok(column)
            })();
            if crate::cpu::enabled() {
                let label = format!("{table:?}");
                crate::cpu::note_named("alias_group_inclusive", &label, started);
            }
            column
        })?
    } else {
        cache.read(key, context.hours.len(), false, || {
            measured(context, table, || {
                let updates = table_updates_inner(context, table, check)?;
                check.run()?;
                Ok(updates)
            })
        })?
    };
    let _ = context.memo[table.index()].set(updates.clone());
    Ok(updates)
}

/// Keep existing per-table CPU attribution when decoding a shared column.
fn measured<T>(context: &Context<'_>, table: TableId, decode: impl FnOnce() -> T) -> T {
    let _cpu = crate::cpu::scope("decode", None);
    let started = crate::cpu::thread_cycles();
    let decoded = decode();
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
}

/// A cached member missing from the group still resolves alone.
fn group_column(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> io::Result<Updates> {
    let Some(column) = group::column(context, table, check) else {
        return Updates::new(table_updates_inner(context, table, check)?);
    };
    #[cfg(test)]
    super::watched::note_decoded();
    #[cfg(any(test, feature = "test-support"))]
    super::test_work::decoded(context.index, table, context.hours.len());
    column
}

fn table_updates_inner(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> io::Result<Vec<TableUpdate>> {
    #[cfg(test)]
    super::watched::note_decoded();
    #[cfg(any(test, feature = "test-support"))]
    super::test_work::decoded(context.index, table, context.hours.len());
    direct::table_updates(context, table, check)
}

fn table_updates_body(
    context: &Context<'_>,
    table: TableId,
    use_direct: bool,
    check: &mut Check<'_>,
) -> io::Result<Vec<TableUpdate>> {
    let RawColumn {
        updates,
        key_marks,
        later,
    } = raw_stage(context, table, use_direct, check)?;
    super::aliases::private::resolve_with(
        table,
        context.hours,
        updates,
        key_marks.as_deref(),
        later.marks(),
        context.complete,
        |source, check| dependency_updates(context, source, check),
        check,
    )
}

struct RawColumn {
    updates: Vec<TableUpdate>,
    /// The keyframe entry's alias marks.
    key_marks: Option<Vec<u8>>,
    later: LaterSection,
}

/// Alias marks occupy the end of the retained later section.
struct LaterSection {
    raw: Option<Vec<u8>>,
    marks: Option<usize>,
}

impl LaterSection {
    fn marks(&self) -> Option<&[u8]> {
        let (raw, length) = (self.raw.as_deref()?, self.marks?);
        Some(&raw[raw.len() - length..])
    }
}

fn raw_stage(
    context: &Context<'_>,
    table: TableId,
    use_direct: bool,
    check: &mut Check<'_>,
) -> io::Result<RawColumn> {
    let _cpu = crate::cpu::reader("reader_raw_grammar", table);
    #[cfg(test)]
    let _step = super::check::Scope::enter(super::check::Step::Raw(table, 0));
    check.run()?;
    #[cfg(any(test, feature = "test-support"))]
    context.replay.decoded_sections.wire_decode(
        context.position,
        table,
        context.hours.len(),
        if use_direct {
            super::decoded_sections::WireRoute::Direct
        } else {
            super::decoded_sections::WireRoute::Original
        },
    );
    #[cfg(test)]
    super::aliases::private::hooks::fail_raw(table)?;
    #[cfg(test)]
    super::aliases::private::hooks::gate_raw(table);
    #[cfg(test)]
    let joined = JOINED.get();
    #[cfg(not(test))]
    let joined = false;
    if !joined && context.complete && context.directory.chunked(later_section(table)) {
        match raw_column(context, table, use_direct, true, check) {
            Err(error) if !check.failed() && !engine_allocation::is_allocation_error(&error) => {
                #[cfg(test)]
                CHUNKED_READS.with(|reads| reads.set([reads.get()[0], reads.get()[1] + 1]));
            }
            read => {
                #[cfg(test)]
                if read.is_ok() {
                    CHUNKED_READS.with(|reads| reads.set([reads.get()[0] + 1, reads.get()[1]]));
                }
                return read;
            }
        }
    }
    raw_column(context, table, use_direct, false, check)
}

#[cfg(test)]
thread_local! {
    /// This thread's reads take every later section joined.
    pub(super) static JOINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// This thread's later sections read a chunk at a time, and those that
    /// fell back to their joined section.
    pub(super) static CHUNKED_READS: std::cell::Cell<[usize; 2]> =
        const { std::cell::Cell::new([0; 2]) };
    /// This thread's tables' plain-byte budget, in place of a block's.
    pub(super) static RAW_BUDGET: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    /// This thread's chunks bound their columns by their own bytes alone.
    pub(super) static CHUNK_LOCAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A table's budget of plain bytes in one block.
fn raw_budget() -> usize {
    #[cfg(test)]
    if let Some(budget) = RAW_BUDGET.get() {
        return budget;
    }
    MAX_BLOCK_RAW_BYTES
}

/// [`raw_stage`]'s column. A `chunked` read of a complete block holds one
/// chunk of the later section at a time and keeps only its alias marks;
/// whatever the joined section reads differently it refuses, for the joined
/// read to answer.
fn raw_column(
    context: &Context<'_>,
    table: TableId,
    use_direct: bool,
    chunked: bool,
    check: &mut Check<'_>,
) -> io::Result<RawColumn> {
    let format = context.format;
    let version = format.version();
    let change: &LastChange = &HISTORY[table.index()];
    let frames = context.hours.len();
    let key_id = keyframe_section(table);
    let later_id = later_section(table);
    let gap = change.table_gap(table, version);
    let fields = change.fields(table, version);
    let ordinal = context.present.ordinal[table.index()];
    let width = context.present.width();
    let key_raw = context
        .directory
        .raw(context.block, key_id)?
        .ok_or_else(|| invalid(format!("grouped block has no keyframe entry for {table:?}")))?;
    let mut chunks = chunked.then(|| context.directory.chunks(context.block, later_id));
    let mut later_raw = match &mut chunks {
        Some(chunks) => Some(chunks.first()?),
        None => context.directory.raw(context.block, later_id)?,
    };
    let mut later_input = match &later_raw {
        Some(raw) => ColumnReader::stored(raw, fields, table, format.version())?,
        None => ColumnReader::empty(fields),
    };
    // A chunked read's marks: the joined section's last column.
    let mut marks_read = Vec::new();
    let marked = later_input.1.is_some();
    if let Some(chunks) = &chunks {
        later_input.2 = chunks.ahead();
        keep_marks(&mut marks_read, later_input.1)?;
    }
    let mode = context.replay.capture_mode;
    let decode = |plain: &[u8], keyframe: bool| -> io::Result<TableUpdate> {
        let _cpu = crate::cpu::scope("decode_wire", None);
        let mut input = Decoder::new(plain);
        let update = decode_recorded_update(table, change, format, mode, keyframe, &mut input)?;
        input.finish()?;
        Ok(update)
    };
    let layout = context.directory.layout;
    let budget = Budget(AtomicUsize::new(raw_budget()));
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
    } = keyframe_update(context.replay, &from, table, &key_raw, &budget, check)?;
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
        #[cfg(test)]
        let _step = super::check::Scope::enter(super::check::Step::Raw(table, frame));
        check.run()?;
        if let Some(chunks) = &mut chunks {
            // The joined section goes on with its next chunk's columns only
            // once every column of this one is read; this one goes before the
            // next is inflated.
            while later_input.exhausted() && chunks.holds_more() {
                later_input = ColumnReader::empty(fields);
                later_raw = None;
                let Some(raw) = chunks.next()? else {
                    break;
                };
                later_raw = Some(raw);
                later_input = ColumnReader::stored(
                    later_raw.as_deref().expect("a chunk"),
                    fields,
                    table,
                    format.version(),
                )?;
                later_input.2 = chunks.ahead();
                keep_marks(&mut marks_read, later_input.1)?;
            }
        }
        let previous = context.hours[frame - 1];
        if !get(
            &context.entries[frame * width..(frame + 1) * width],
            ordinal,
        ) {
            if predictor.predicts() {
                // Exactly the predicted changes.
                if use_direct && gap.is_none() {
                    updates.push(predicted::take_residual_direct(
                        table,
                        fields,
                        version,
                        previous,
                        None,
                        &mut predictor,
                        &mut shadow,
                        &budget,
                        layout,
                        None,
                    )?);
                    continue;
                }
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
        if use_direct && gap.is_none() && head.status == DELTA {
            let update = if head.residual {
                predicted::take_residual_direct(
                    table,
                    fields,
                    version,
                    previous,
                    Some(&mut later_input),
                    &mut predictor,
                    &mut shadow,
                    &budget,
                    layout,
                    coding,
                )?
            } else {
                predictor.clear();
                direct::take_delta(
                    table,
                    fields,
                    version,
                    plain_baseline(&plain).expect("a delta header has a baseline"),
                    &mut later_input,
                    &mut shadow,
                    &budget,
                    layout,
                    coding,
                )?
            };
            updates.push(update);
            if layout.keyed() {
                keys.apply(&mut changes)?;
            }
            continue;
        }
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
                keys.apply(&mut changes)?;
            } else {
                keys.advance(table, version, &plain)?;
            }
        }
    }
    // The marks are the section's last column, so they end where it does.
    let later_marks = later_input.1.map(<[u8]>::len);
    if context.complete {
        later_input.finish()?;
    } else {
        drop(later_input);
    }
    let later = match &mut chunks {
        Some(chunks) => {
            drop(later_raw);
            if chunks.next()?.is_some() {
                return Err(invalid_static("grouped section has unread column bytes"));
            }
            LaterSection {
                marks: marked.then_some(marks_read.len()),
                raw: Some(marks_read),
            }
        }
        None => LaterSection {
            raw: later_raw,
            marks: later_marks,
        },
    };
    Ok(RawColumn {
        updates,
        key_marks: marks,
        later,
    })
}

fn keep_marks(marks: &mut Vec<u8>, chunk: Option<&[u8]>) -> io::Result<()> {
    let chunk = chunk.unwrap_or_default();
    engine_allocation::reserve(marks, chunk.len())?;
    marks.extend_from_slice(chunk);
    Ok(())
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
    depth: usize,
}

#[derive(Default)]
struct CandidateHistory {
    version: Option<u16>,
    steps: Vec<CandidateStep>,
}

struct CandidateStep {
    index: BlockIndex,
    ordinal: usize,
    snapshot: Option<usize>,
    latest: Option<usize>,
}

pub(super) struct CandidateCache {
    tables: [std::sync::Mutex<CandidateHistory>; TABLE_COUNT],
}

impl Default for CandidateCache {
    fn default() -> Self {
        Self {
            tables: std::array::from_fn(|_| std::sync::Mutex::default()),
        }
    }
}

#[cfg(test)]
fn anchor_candidates(
    replay: &Replay,
    before: usize,
    table: TableId,
    fields: usize,
    version: u16,
) -> io::Result<(Option<usize>, Option<usize>)> {
    anchor_candidates_with(
        replay,
        before,
        table,
        fields,
        version,
        &mut Check::disabled(),
    )
}

fn anchor_candidates_with(
    replay: &Replay,
    before: usize,
    table: TableId,
    fields: usize,
    version: u16,
    check: &mut Check<'_>,
) -> io::Result<(Option<usize>, Option<usize>)> {
    let mut history = replay.anchor_candidates.tables[table.index()]
        .lock()
        .map_err(|_| io::Error::other("keyframe anchor candidate cache poisoned"))?;
    if history.version != Some(version) {
        history.steps.clear();
        history.version = Some(version);
    }
    // Public index edits must not reuse candidate positions from a different prefix.
    let shared = history.steps.len().min(before);
    let prefix = history.steps[..shared]
        .iter()
        .enumerate()
        .position(|(at, step)| {
            replay.index.get(at) != Some(&step.index)
                || replay.ordinals.get(at) != Some(&step.ordinal)
        })
        .unwrap_or(shared);
    if prefix < shared {
        history.steps.truncate(prefix);
    }
    while history.steps.len() < before {
        check.run()?;
        let at = history.steps.len();
        let ordinal = replay.ordinals[at];
        let previous = history.steps.last();
        let (mut snapshot, mut latest) =
            if previous.is_some_and(|step| step.ordinal.checked_add(1) == Some(ordinal)) {
                let previous = previous.unwrap();
                (previous.snapshot, previous.latest)
            } else {
                (None, None)
            };
        let index = &replay.index[at];
        let head = (|| -> io::Result<Option<Head>> {
            let directory = Directory::read(replay, index)?;
            let Some(raw) = directory.read_raw(replay, index, keyframe_section(table))? else {
                if keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)?
                    .is_none()
                {
                    return Err(invalid(format!(
                        "grouped block has no keyframe entry for {table:?}"
                    )));
                }
                return Ok(None);
            };
            let mut input = ColumnReader::stored(&raw, fields, table, version)?;
            take_head(
                &mut input,
                None,
                index.first_hour,
                &mut Vec::new(),
                directory.layout,
            )
            .map(Some)
        })();
        match head {
            Ok(Some(head)) if head.anchor.is_some() || head.status == SNAPSHOT => {
                latest = Some(at);
                if head.anchor.is_none() {
                    snapshot = Some(at);
                }
            }
            Ok(_) => {}
            // Damage invalidates only candidates before it, not later known keyframes.
            Err(error) if super::recording_damage(&error) => {
                snapshot = None;
                latest = None;
            }
            Err(error) => return Err(error),
        }
        engine_allocation::reserve(&mut history.steps, 1)?;
        history.steps.push(CandidateStep {
            index: index.clone(),
            ordinal,
            snapshot,
            latest,
        });
    }
    let Some(last) = before.checked_sub(1).and_then(|at| history.steps.get(at)) else {
        return Ok((None, None));
    };
    if last.ordinal.checked_add(1) != replay.ordinals.get(before).copied() {
        return Ok((None, None));
    }
    Ok((last.snapshot, last.latest))
}

// Eligibility reads only the demanded reference interval, never a historical candidate prepass.
#[allow(clippy::too_many_arguments)]
fn anchor_eligible_bounded(
    replay: &Replay,
    before: usize,
    target: usize,
    snapshot: bool,
    table: TableId,
    fields: usize,
    version: u16,
    check: &mut Check<'_>,
    reads: &mut AnchorReads,
) -> io::Result<bool> {
    let mut next = replay.ordinals[before];
    for at in (target + 1..before).rev() {
        check.run()?;
        if replay.ordinals[at].checked_add(1) != Some(next) {
            return Ok(false);
        }
        next = replay.ordinals[at];
        let index = &replay.index[at];
        let head = (|| -> io::Result<Option<Head>> {
            let directory = Directory::read_admitted(
                replay,
                index,
                Directory::prefix(replay, index)?,
                &mut |_, _| Ok(()),
            )?;
            reads.stored_bytes = engine_allocation::add(reads.stored_bytes, directory.read_bytes)?;
            let Some(chunk) = replay
                .section_chunks_from(index, Section::Keyframe(table), &directory)?
                .next()
            else {
                if keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)?
                    .is_none()
                {
                    return Err(invalid_static("keyframe anchor has no entry for its table"));
                }
                return Ok(None);
            };
            reads.sections += 1;
            reads.stored_bytes =
                engine_allocation::add(reads.stored_bytes, chunk.stored_length as usize)?;
            reads.raw_bytes = engine_allocation::add(reads.raw_bytes, chunk.raw_length as usize)?;
            let mut raw = Vec::new();
            chunk.inflate_admitted(&mut raw, true, &mut |_, _, _| Ok(()), check)?;
            let mut input = ColumnReader::stored(&raw, fields, table, version)?;
            take_head(
                &mut input,
                None,
                index.first_hour,
                &mut Vec::new(),
                directory.layout,
            )
            .map(Some)
        })();
        match head {
            Ok(Some(head)) if head.anchor.is_none() && head.status == SNAPSHOT => return Ok(false),
            Ok(Some(head)) if head.anchor.is_some() && !snapshot => return Ok(false),
            Ok(_) => {}
            Err(error) if check.failed() || !super::recording_damage(&error) => return Err(error),
            Err(_) => return Ok(false),
        }
    }
    Ok(replay.ordinals[target].checked_add(1) == Some(next))
}

/// The keyframe of `table` that `from`'s keyframe entry anchors to, an
/// earlier block's, following its own anchors to their snapshot. An anchored
/// block's chain may cross branches and, with `own`, stays within
/// [`chain_charge`] of its snapshot. `keep_root` also returns the snapshot's state.
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
    anchored_keyframe_with(
        replay,
        from,
        table,
        fields,
        discarded,
        anchor,
        own,
        format,
        keep_root,
        &mut Check::disabled(),
    )
}

#[allow(clippy::too_many_arguments)]
fn anchored_keyframe_with(
    replay: &Replay,
    from: &From<'_>,
    table: TableId,
    fields: usize,
    discarded: bool,
    anchor: AnchorRef,
    own: usize,
    format: FormatVersion,
    keep_root: bool,
    check: &mut Check<'_>,
) -> io::Result<Resolved> {
    anchored_keyframe_read(
        replay,
        from,
        table,
        fields,
        discarded,
        anchor,
        own,
        format,
        keep_root,
        check,
        &mut AnchorReads::default(),
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn anchored_keyframe_counted(
    replay: &Replay,
    from: &From<'_>,
    table: TableId,
    fields: usize,
    discarded: bool,
    anchor: AnchorRef,
    own: usize,
    format: FormatVersion,
    keep_root: bool,
    check: &mut Check<'_>,
    reads: &mut AnchorReads,
) -> io::Result<Resolved> {
    let started = anchor_cpu_cycles();
    let previous_reads = reads.sections;
    let result = anchored_keyframe_read(
        replay, from, table, fields, discarded, anchor, own, format, keep_root, check, reads, true,
    );
    reads.cycles = started.zip(anchor_cpu_cycles()).and_then(|(start, end)| {
        if previous_reads == 0 {
            Some(end.saturating_sub(start))
        } else {
            reads
                .cycles
                .map(|prior| prior.saturating_add(end.saturating_sub(start)))
        }
    });
    result
}

#[derive(Default)]
pub(crate) struct AnchorReads {
    pub sections: usize,
    pub stored_bytes: usize,
    pub raw_bytes: usize,
    pub cycles: Option<u64>,
}

fn anchor_cpu_cycles() -> Option<u64> {
    #[cfg(windows)]
    {
        crate::cpu::enabled().then(crate::cpu::thread_cycles)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn anchored_keyframe_read(
    replay: &Replay,
    from: &From<'_>,
    table: TableId,
    fields: usize,
    discarded: bool,
    anchor: AnchorRef,
    own: usize,
    format: FormatVersion,
    keep_root: bool,
    check: &mut Check<'_>,
    reads: &mut AnchorReads,
    bounded: bool,
) -> io::Result<Resolved> {
    let _table = replay_data::measurement::table(table);
    let _cpu = crate::cpu::scope("anchors", None);
    let version = format.version();
    let change: &LastChange = &HISTORY[table.index()];
    let decode = |plain: &[u8], keyframe: bool| -> io::Result<TableUpdate> {
        let _cpu = crate::cpu::scope("decode_wire", None);
        let mut input = Decoder::new(plain);
        let update = decode_update(table, change, format, keyframe, &mut input)?;
        input.finish()?;
        Ok(update)
    };
    let bound = || invalid_static("keyframe anchor chain exceeds its size bound");
    // The chain's keyframe sections, newest first, ending at its snapshot,
    // and the raw bytes of its anchored entries with the naming entry's.
    let mut chain: Vec<Link> = Vec::new();
    let mut anchored = own;
    let (mut position, mut anchor) = (from.position, anchor);
    loop {
        check.run()?;
        // Each link is an earlier block of the file than the one naming it.
        let (at, block_index) = match anchor {
            // Counted among the file's blocks, so a block whose index entry
            // opening withheld is never replaced by the one before it.
            AnchorRef::Back(blocks) => position
                .and_then(|position| replay.ordinals[position].checked_sub(blocks))
                .and_then(|ordinal| replay.ordinals.binary_search(&ordinal).ok())
                .map(|at| (at, &replay.index[at])),
        }
        .ok_or_else(|| invalid_static("keyframe anchor names no earlier block of its file"))?;
        let before = position
            .ok_or_else(|| invalid_static("keyframe anchor has no indexed source block"))?;
        if !bounded {
            let (snapshot, latest) =
                anchor_candidates_with(replay, before, table, fields, version, check)?;
            if Some(at) != snapshot && Some(at) != latest {
                return Err(invalid_static(
                    "keyframe anchor is not the latest snapshot or latest keyframe",
                ));
            }
        }
        let directory = Directory::read_admitted(
            replay,
            block_index,
            Directory::prefix(replay, block_index)?,
            &mut |_, _| Ok(()),
        )?;
        reads.stored_bytes = engine_allocation::add(reads.stored_bytes, directory.read_bytes)?;
        let (offset, length, raw_length) = directory.sections[keyframe_section(table)]
            .ok_or_else(|| invalid_static("keyframe anchor has no entry for its table"))?;
        reads.sections += 1;
        reads.stored_bytes += length;
        reads.raw_bytes += raw_length;
        check.run()?;
        let raw = chunks::inflate(
            &replay.block_range(block_index, offset, length)?,
            raw_length,
            true,
            check,
        )?;
        let mut input = ColumnReader::stored(&raw, fields, table, version)?;
        let head = take_head(
            &mut input,
            None,
            block_index.first_hour,
            &mut Vec::new(),
            directory.layout,
        )?;
        if bounded
            && !anchor_eligible_bounded(
                replay,
                before,
                at,
                head.anchor.is_none(),
                table,
                fields,
                version,
                check,
                reads,
            )?
        {
            return Err(invalid_static(
                "keyframe anchor is not the latest snapshot or latest keyframe",
            ));
        }
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
            Some(next) => (position, anchor) = (Some(at), next),
            None => break,
        }
    }
    let snapshot = chain.last().expect("a chain ends at its snapshot");
    let root = (snapshot.position, snapshot.raw.len());
    // Every anchored entry, the naming one included, also counts its share
    // of rebuilt rows.
    let rebuilds = chain
        .len()
        .checked_mul(root.1 / REBUILD_SHARE)
        .ok_or_else(bound)?;
    let charge = anchored.checked_add(rebuilds).ok_or_else(bound)?;
    if from.layout.anchors_back() && charge > root.1 {
        return Err(bound());
    }
    let mut state: Option<AnchorState> = None;
    let mut root_state = None;
    for link in chain.iter().rev() {
        check.run()?;
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
        take_body_checked(
            table,
            fields,
            head.status,
            &mut input,
            &shadow,
            &mut plain,
            &budget,
            link.layout,
            None,
            check,
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
            if replay_data::measurement::enabled() {
                replay_data::measurement::count(replay_data::measurement::Counter::AnchorCopies, 1);
                replay_data::measurement::count(
                    replay_data::measurement::Counter::AnchorRows,
                    rows.as_ref().map_or(0, TableRows::len),
                );
                replay_data::measurement::count(
                    replay_data::measurement::Counter::AnchorShadowBytes,
                    shadow
                        .values()
                        .flat_map(|fields| fields.values())
                        .map(Vec::len)
                        .sum(),
                );
            }
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
        depth: chain.len() - 1,
    })
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn replace_sections(
    replay: &Replay,
    original: &BlockIndex,
    overrides: &[(usize, Vec<u8>)],
) -> io::Result<(BlockIndex, Vec<u8>)> {
    chunked::replace_sections(replay, original, overrides)
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
        assert_eq!(advance(&bases, Layout::Chosen, 5, 0), Some(30));
        assert_eq!(advance(&bases, Layout::Chosen, 5, 1), None);
        assert_eq!(advance(&bases, Layout::Chosen, 6, 0), None);
        bases.get_mut(&5).unwrap().remove(&DROPPED);
        assert_eq!(advance(&bases, Layout::Chosen, 5, 0), None);
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
                0,
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
        assert_eq!(take(path, &base, Layout::Chosen).unwrap(), list(&[8, 9]));
        assert_eq!(take(path, &list(&[7]), Layout::Chosen).unwrap(), list(&[]));
        // An empty list has no head.
        assert!(take(path, &list(&[]), Layout::Chosen).is_err());
        assert!(take(source, &[0; 4], Layout::Chosen).is_err());
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
                    0,
                    Some(&base),
                    None,
                    &mut values,
                    0,
                    &budget,
                    Layout::Chosen,
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
        let head = |entry, offset, previous| head_in(Layout::Chosen, entry, offset, previous);
        assert_eq!(
            head(ANCHOR_UNCHANGED, Some(3), None).unwrap(),
            Some(AnchorRef::Back(3))
        );
        assert!(head(ANCHOR_UNCHANGED, Some(0), None).is_err());
        assert!(head(DELTA, Some(0), None).is_err());
        assert!(head(ANCHOR_DELTA, Some(1), Some(9)).is_err());
        let highest_assigned = [
            SNAPSHOT,
            DELTA,
            UNCHANGED,
            TRUNCATED,
            UNAVAILABLE,
            ANCHOR_DELTA,
            ANCHOR_UNCHANGED,
            RESIDUAL,
            NOT_RECORDED_ENTRY,
        ]
        .into_iter()
        .max()
        .unwrap();
        assert!(head(highest_assigned + 1, None, Some(9)).is_err());
        for previous in [None, Some(9)] {
            assert!(head(NOT_RECORDED_ENTRY, None, previous).unwrap().is_none());
        }
        // A residual entry belongs to a predicted block's later entries.
        assert!(head(RESIDUAL, None, Some(9)).unwrap().is_none());
        assert!(head(RESIDUAL, None, None).is_err());
    }

    #[test]
    fn recorded_steps_past_any_range_are_invalid_data_never_an_overflow() {
        let present = Present::new(FormatVersion::CURRENT.version(), CaptureMode::Full);
        let refused = |error: Option<io::Error>| {
            error.is_some_and(|error| error.kind() == io::ErrorKind::InvalidData)
        };
        // A block of hours 100 and 101: its second frame's gap from the first.
        let index = BlockIndex {
            branch: 1,
            first_hour: 100,
            last_hour: 101,
            offset: 0,
            length: 0,
            raw_length: 0,
        };
        let header = |gap: u128| {
            let mut header = Vec::new();
            put_varint(&mut header, gap);
            header.resize(header.len() + present.width(), 0);
            read_header(&header, &index, 2, &present).map(|(hours, _)| hours)
        };
        assert_eq!(header(0).unwrap(), [100, 101]);
        for gap in [i128::MAX as u128 - 101, i128::MAX as u128, u128::MAX] {
            assert!(refused(header(gap).err()), "gap {gap}");
        }
        // A later entry's baseline, told back from the frame before at hour 10.
        let baseline = |code: u128| {
            let mut columns = Columns::new(1);
            put_varint(&mut columns.0[CONTROL], u128::from(DELTA));
            put_varint(&mut columns.0[CONTROL], code);
            let raw = columns.raw();
            let mut input = ColumnReader::open(&raw, 1).unwrap();
            let mut plain = Vec::new();
            take_head(&mut input, Some(10), 10, &mut plain, Layout::Chosen)
                .map(|_| plain_baseline(&plain))
        };
        assert_eq!(baseline(zigzag(1)).unwrap(), Some(9));
        for code in [zigzag(i128::MAX), zigzag(i128::MIN)] {
            assert!(refused(baseline(code).err()), "code {code}");
        }
        // Two frames' metrics: the first adds 1 to the pool and the drops, the second `change`.
        let diagnostics = |change: u128| {
            let mut raw = Vec::new();
            for step in [zigzag(1), change] {
                put_varint(&mut raw, 5);
                put_varint(&mut raw, step);
                put_varint(&mut raw, step);
                raw.resize(raw.len() + present.width(), 0);
            }
            raw
        };
        let metrics = read_diagnostics_prefix(&diagnostics(zigzag(2)), 2, &present, 2).unwrap();
        assert_eq!((metrics[1].pool_bytes, metrics[1].dropped_frames), (3, 3));
        for change in [zigzag(-2), zigzag(i128::MAX)] {
            let raw = diagnostics(change);
            let prefix = read_diagnostics_prefix(&raw, 2, &present, 2);
            let streamed = cursor::DiagnosticsCursor::new(raw, &present, 2, &mut Check::disabled());
            assert!(refused(prefix.err()), "change {change}");
            assert!(refused(streamed.err()), "change {change}");
        }
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
                Layout::Chosen,
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
