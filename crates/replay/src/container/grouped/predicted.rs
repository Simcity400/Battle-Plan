//! Field prediction for later entries of predicted grouped blocks.
//!
//! Writer and reader follow the plain updates of one table in one block and
//! keep, for every field that changed in the previous frame's continuous
//! delta (its baseline is the previous frame), how it moved ([`Motion`]):
//!
//! - A numeric field changed by the same amount in two consecutive frames is
//!   predicted to change by it again. A different nonzero change keeps that
//!   prediction, so a one-off jump (a wrapping clock, a reset countdown, a
//!   rounded step of a fractional rate) costs one correction; a field that
//!   stops, or becomes None, is no longer predicted. In a keyed block, a
//!   field whose change grew by the same amount over three consecutive
//!   frames is predicted to grow by it again: a rate that ramps up an hour
//!   at a time (a division's supply grace refilling ever faster) costs its
//!   first three changes and one correction when it levels off.
//! - In a ramped block a numeric field that changes in two hours in a row by
//!   amounts the repeated change does not predict also follows the rate its
//!   changes allow ([`super::ramp`]), and takes whichever prediction was
//!   right in more of its last eight hours (the repeated change on a tie),
//!   so a value moving a fractional number of display steps an hour costs
//!   nothing once learned and an irregular one costs no more than before. A
//!   rate counts unchanged hours, so the field's row is visited every hour
//!   while it has one, changed or not.
//! - A byte field (string, list or nested record) whose encoding kept its
//!   length and changed by the same little-endian difference in two
//!   consecutive frames is predicted to change by it again, which carries
//!   every nested counter that moves at a constant rate.
//!
//! A continuous delta is stored as a residual entry: its deleted keys, then
//! only the rows with a field that did not move as predicted, each with
//! those fields' corrections. A frame with no entry is exactly its
//! predictions (no prediction is the unchanged table). Any other update, and
//! the block's keyframe, starts the state again, so a block decodes from its
//! keyframe alone.

use super::{
    Budget, CONTROL, ColumnReader, Columns, DELTA, FIELDS, KEYS, KeyReader, KeyWriter, Layout,
    MASKS, RESIDUAL, RowCoding, Shadow, list_width, put_byte_field, ramp::Ramp, take_byte_field,
    unzigzag, varint_usize, zigzag,
};
use crate::{
    Hour,
    codec::{Decoder, invalid_static as invalid, put_varint},
    rows::{Change, Kind, TableId, Token},
};
use std::io;

/// How a field moved in the previous frame's continuous delta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Motion {
    /// A numeric field's plain change (its difference, or an inserted row's
    /// value), in a keyed block its change in the frame before when it
    /// changed then too, and the change predicted for the next frame.
    Number {
        delta: i128,
        earlier: Option<i128>,
        prediction: Option<i128>,
    },
    /// A byte field's change from an encoding of the same length, as the
    /// wrapping little-endian difference of the two encodings, and whether
    /// it repeated the previous frame's change.
    Bytes { delta: Vec<u8>, predicted: bool },
    /// A ramped block's numeric field ([`Rated`]).
    Rated(Rated),
}

/// A numeric field's last change, in an accelerating layout its change in
/// the frame before when it changed then too, and the change predicted next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Repeat {
    delta: i128,
    earlier: Option<i128>,
    prediction: Option<i128>,
}

impl Repeat {
    /// After the field took `step`; None when it did not change.
    fn after(step: Step, before: Option<Self>, accelerates: bool) -> Option<Self> {
        let Step::Number(delta) = step else {
            return None;
        };
        if delta == 0 {
            return None;
        }
        let (last, earlier, prediction) = before.map_or((None, None, None), |before| {
            (Some(before.delta), before.earlier, before.prediction)
        });
        // Changes fit 64 bits, so their differences and sums fit i128.
        let prediction = if last == Some(delta) {
            Some(delta)
        } else if let (Some(last), Some(earlier)) = (last, earlier)
            && delta - last == last - earlier
        {
            // A ramp that reaches zero stops: no field changes by zero.
            Some(delta + (delta - last)).filter(|next| *next != 0)
        } else {
            prediction
        };
        Some(Self {
            delta,
            earlier: last.filter(|_| accelerates),
            prediction,
        })
    }
}

/// A ramped block's numeric field: the keyed layout's [`Repeat`], its
/// [`Ramp`], and in which of the last eight hours each was right (bit 0 the
/// latest). The ramp is boxed: few fields have one, and every motion of the
/// table is as large as its largest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Rated {
    repeat: Option<Repeat>,
    ramp: Option<Box<Ramp>>,
    repeat_hits: u8,
    ramp_hits: u8,
}

impl Rated {
    /// The change predicted next, if any.
    fn prediction(&self) -> Option<i128> {
        let repeat = self.repeat.and_then(|repeat| repeat.prediction);
        let ramp = self
            .ramp
            .as_ref()
            .map(|ramp| ramp.next())
            .filter(|next| *next != 0);
        if self.ramp_hits.count_ones() > self.repeat_hits.count_ones() {
            ramp
        } else {
            repeat
        }
    }

    /// After the field took `step`; None when neither predictor follows it.
    fn after(step: Step, before: Option<Self>) -> Option<Self> {
        let change = match step {
            Step::Null => return None,
            Step::Same => 0,
            Step::Number(change) => change,
        };
        let before = before.unwrap_or(Self {
            repeat: None,
            ramp: None,
            repeat_hits: 0,
            ramp_hits: 0,
        });
        let hit = |prediction: Option<i128>| u8::from(prediction.unwrap_or(0) == change);
        let repeat_hits =
            before.repeat_hits << 1 | hit(before.repeat.and_then(|repeat| repeat.prediction));
        let ramp_hits = before.ramp_hits << 1 | hit(before.ramp.as_ref().map(|ramp| ramp.next()));
        let rated = Self {
            repeat: Repeat::after(step, before.repeat, true),
            ramp: match (before.ramp, before.repeat) {
                (Some(mut ramp), _) => ramp.push(change).map(|next| {
                    *ramp = next;
                    ramp
                }),
                // A field changing every hour by amounts its repeated change
                // does not predict: its last two changes start a ramp.
                (None, Some(last)) if change != 0 && last.prediction != Some(change) => {
                    Ramp::start(last.delta)
                        .and_then(|ramp| ramp.push(change))
                        .map(Box::new)
                }
                (None, _) => None,
            },
            repeat_hits,
            ramp_hits,
        };
        (rated.repeat.is_some() || rated.ramp.is_some()).then_some(rated)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Moving {
    pub key: u128,
    pub field: usize,
    pub motion: Motion,
}

impl Moving {
    fn predicts(&self) -> bool {
        match &self.motion {
            Motion::Number { prediction, .. } => prediction.is_some(),
            Motion::Bytes { predicted, .. } => *predicted,
            Motion::Rated(rated) => rated.prediction().is_some(),
        }
    }
}

/// One table's moving fields, in key then field order.
#[derive(Default)]
pub(super) struct Predictor {
    moving: Vec<Moving>,
}

impl Predictor {
    pub(super) fn clear(&mut self) {
        self.moving.clear();
    }
    /// Whether any field is predicted to change in the next frame.
    pub(super) fn predicts(&self) -> bool {
        self.moving.iter().any(Moving::predicts)
    }
    #[cfg(test)]
    pub(super) fn moving(&self) -> &[Moving] {
        &self.moving
    }
}

/// A numeric field's change in one plain delta row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Not in the row's mask.
    Same,
    /// An optional field became None.
    Null,
    Number(i128),
}

fn optional(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64
    )
}

fn step_of(kind: Kind, span: &[u8]) -> io::Result<Step> {
    let mut input = Decoder::new(span);
    let step = match kind.decode_change(&mut input)? {
        Change::Number(value) | Change::Optional(Some(value)) => Step::Number(value),
        Change::Optional(None) => Step::Null,
        _ => return Err(invalid("numeric field has a byte change")),
    };
    input.finish()?;
    Ok(step)
}

/// A stored numeric field: 0 is no change, then (optional fields) 1 is None,
/// then the zigzag correction to `predicted`, the predicted change or,
/// without one, the field's advance ([`super::advance_change`]).
fn residual_code(kind: Kind, predicted: Option<i128>, step: Step) -> io::Result<u128> {
    let shift = if optional(kind) { 2 } else { 1 };
    Ok(match step {
        Step::Same => 0,
        Step::Null => 1,
        Step::Number(value) => value
            .checked_sub(predicted.unwrap_or(0))
            .map(zigzag)
            .and_then(|code| code.checked_add(shift))
            .ok_or_else(|| invalid("predicted change outside its range"))?,
    })
}

/// A stored numeric field's step: `code` against `zero`, the predicted
/// change or the field's advance ([`residual_code`]); a stored field differs
/// from `predicted`, the predicted change alone.
fn step_from_code(
    kind: Kind,
    predicted: Option<i128>,
    zero: Option<i128>,
    code: u128,
) -> io::Result<Step> {
    let shift = if optional(kind) { 2 } else { 1 };
    let step = match code {
        0 => Step::Same,
        1 if shift == 2 => Step::Null,
        code => Step::Number(
            zero.unwrap_or(0)
                .checked_add(unzigzag(code - shift))
                .ok_or_else(|| invalid("predicted change outside its range"))?,
        ),
    };
    // A stored field differs from its prediction, and a predicted field
    // belongs to a row that already holds a value, which cannot change by zero.
    if step == predicted.map_or(Step::Same, Step::Number)
        || (predicted.is_some() && step == Step::Number(0))
    {
        return Err(invalid("noncanonical predicted field correction"));
    }
    Ok(step)
}

fn put_step(kind: Kind, step: Step, plain: &mut Vec<u8>) {
    match step {
        Step::Same => unreachable!("unchanged fields have no plain change"),
        Step::Null => plain.push(0),
        Step::Number(value) => {
            if optional(kind) {
                plain.push(1);
            }
            put_varint(plain, zigzag(value));
        }
    }
}

/// The change predicted for a numeric field with the motion `before`.
fn number_prediction(before: Option<&Moving>) -> Option<i128> {
    match before.map(|before| &before.motion) {
        Some(Motion::Number { prediction, .. }) => *prediction,
        Some(Motion::Rated(rated)) => rated.prediction(),
        _ => None,
    }
}

/// The motion of a numeric field that took `step`, given its previous
/// motion. In a keyed block a steadily growing change is predicted too; a
/// ramped one also follows the field's rate ([`Rated`]).
fn number_motion(step: Step, before: Option<&Moving>, layout: Layout) -> Option<Motion> {
    let before = before.map(|before| &before.motion);
    if layout.ramped() {
        let before = match before {
            Some(Motion::Rated(rated)) => Some(rated.clone()),
            _ => None,
        };
        return Rated::after(step, before).map(Motion::Rated);
    }
    let before = match before {
        Some(Motion::Number {
            delta,
            earlier,
            prediction,
        }) => Some(Repeat {
            delta: *delta,
            earlier: *earlier,
            prediction: *prediction,
        }),
        _ => None,
    };
    Repeat::after(step, before, layout.accelerates()).map(|repeat| Motion::Number {
        delta: repeat.delta,
        earlier: repeat.earlier,
        prediction: repeat.prediction,
    })
}

/// `a - b` as little-endian integers of one length, wrapping.
fn byte_difference(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut borrow = false;
    a.iter()
        .zip(b)
        .map(|(a, b)| {
            let (value, first) = a.overflowing_sub(*b);
            let (value, second) = value.overflowing_sub(u8::from(borrow));
            borrow = first || second;
            value
        })
        .collect()
}

/// `a + b` as little-endian integers of one length, wrapping.
fn byte_sum(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut carry = false;
    a.iter()
        .zip(b)
        .map(|(a, b)| {
            let (value, first) = a.overflowing_add(*b);
            let (value, second) = value.overflowing_add(u8::from(carry));
            carry = first || second;
            value
        })
        .collect()
}

/// The motion of a byte field whose encoding changed from `base` to
/// `value`, given its previous motion.
fn byte_motion(base: Option<&[u8]>, value: &[u8], before: Option<&Moving>) -> Option<Motion> {
    let base = base.filter(|base| base.len() == value.len())?;
    let delta = byte_difference(value, base);
    let predicted = matches!(
        before.map(|before| &before.motion),
        Some(Motion::Bytes { delta: last, .. }) if *last == delta
    );
    Some(Motion::Bytes { delta, predicted })
}

/// The predicted encoding of a byte field whose encoding is `base`.
fn byte_prediction(base: Option<&[u8]>, before: Option<&Moving>) -> Option<Vec<u8>> {
    match before.map(|before| &before.motion) {
        Some(Motion::Bytes {
            delta,
            predicted: true,
        }) => base
            .filter(|base| base.len() == delta.len())
            .map(|base| byte_sum(base, delta)),
        _ => None,
    }
}

/// Walks the previous frame's moving fields in key then field order.
struct Cursor<'a> {
    moving: &'a [Moving],
}

impl<'a> Cursor<'a> {
    fn find(&mut self, key: u128, field: usize) -> Option<&'a Moving> {
        while let Some((first, rest)) = self.moving.split_first() {
            if (first.key, first.field) >= (key, field) {
                return (first.key == key && first.field == field).then_some(first);
            }
            self.moving = rest;
        }
        None
    }
}

/// Keys with a predicted field or a rate that moves on through an unchanged
/// hour, in order, skipping deleted keys.
fn predicted_keys<'a>(
    moving: &'a [Moving],
    deleted: &'a [u128],
) -> impl Iterator<Item = u128> + 'a {
    let mut deleted = deleted.iter().copied().peekable();
    let mut last = None;
    moving
        .iter()
        .filter(|moving| {
            moving.predicts()
                || matches!(&moving.motion, Motion::Rated(rated) if rated.ramp.is_some())
        })
        .filter_map(move |moving| {
            if last == Some(moving.key) {
                return None;
            }
            last = Some(moving.key);
            while deleted.next_if(|key| *key < moving.key).is_some() {}
            (deleted.peek() != Some(&moving.key)).then_some(moving.key)
        })
}

fn base(bases: &Shadow, key: u128, field: usize) -> Option<&[u8]> {
    bases
        .get(&key)
        .and_then(|fields| fields.get(&field))
        .map(Vec::as_slice)
}

/// One changed row of a plain delta and where its fields start.
struct PlainRow {
    key: u128,
    fields: usize,
}

/// Append a continuous plain delta as a residual entry, unless every change
/// is predicted and nothing is deleted; returns whether an entry was written.
/// The predictor moves on to this frame's changes either way; `bases` are
/// the byte fields' encodings before it. With `coding` (a keyed block), row
/// keys and masks are stored against the rows and masks before them, or as
/// model decisions (a modelled block).
#[allow(clippy::too_many_arguments)]
pub(super) fn put_residual(
    table: TableId,
    format: u16,
    fields: usize,
    plain: &[u8],
    predictor: &mut Predictor,
    bases: &Shadow,
    out: &mut Columns,
    layout: Layout,
    mut coding: Option<RowCoding<'_>>,
) -> io::Result<bool> {
    if plain.first() != Some(&DELTA) {
        return Err(invalid("a residual entry needs a plain delta"));
    }
    let mut rows: Vec<PlainRow> = Vec::new();
    let mut spans: Vec<(usize, &[u8])> = Vec::new();
    let mut deleted = Vec::new();
    let mut counts = 0;
    table.walk_update(plain, format, &mut |token, bytes| match token {
        Token::Count => counts += 1,
        Token::Key(key) if counts == 1 => rows.push(PlainRow {
            key,
            fields: spans.len(),
        }),
        Token::Key(key) => deleted.push(key),
        Token::Field(field) => spans.push((field, bytes)),
        _ => {}
    })?;
    let lengths: Vec<usize> = out.0.iter().map(Vec::len).collect();
    let alternates: Vec<Option<usize>> = out
        .1
        .iter()
        .map(|column| column.as_ref().map(Vec::len))
        .collect();
    put_varint(&mut out.0[CONTROL], u128::from(RESIDUAL));
    put_varint(&mut out.0[CONTROL], deleted.len() as u128);
    let mut previous_key = 0;
    let mut run = coding.as_ref().map(|coding| KeyWriter::new(coding.keys));
    for &key in &deleted {
        match &mut run {
            Some(run) => out.put_key(run, key)?,
            None => put_varint(&mut out.0[KEYS], key - previous_key),
        }
        previous_key = key;
    }
    let mut run = coding.as_ref().map(RowCoding::run);
    let schema = table.schema();
    let width = fields.div_ceil(8);
    let mut cursor = Cursor {
        moving: &predictor.moving,
    };
    let mut next = Vec::with_capacity(predictor.moving.len());
    let mut residuals = 0_usize;
    let mut previous_key = 0;
    let mut plain_rows = rows.iter().enumerate().peekable();
    let mut predicted = predicted_keys(&predictor.moving, &deleted).peekable();
    loop {
        let key = match (plain_rows.peek(), predicted.peek()) {
            (None, None) => break,
            (Some((_, row)), None) => row.key,
            (None, Some(&key)) => key,
            (Some((_, row)), Some(&key)) => row.key.min(key),
        };
        let row = plain_rows.next_if(|(_, row)| row.key == key);
        predicted.next_if(|predicted| *predicted == key);
        let fields_of = row.map_or(0..0, |(index, row)| {
            row.fields..rows.get(index + 1).map_or(spans.len(), |next| next.fields)
        });
        let mut row_spans = spans[fields_of].iter().peekable();
        let mut mask = 0_u64;
        for (field, &kind) in schema[..fields].iter().enumerate() {
            let span = row_spans
                .next_if(|(at, _)| *at == field)
                .map(|(_, span)| *span);
            let before = cursor.find(key, field);
            let motion = if table.byte_field(field) {
                let base = base(bases, key, field);
                let prediction = byte_prediction(base, before);
                if span != prediction.as_deref() {
                    mask |= 1 << field;
                    let trim = list_width(table, field).filter(|_| layout.trims());
                    put_byte_field(span, base, prediction.as_deref(), out, FIELDS + field, trim)?;
                }
                span.and_then(|value| byte_motion(base, value, before))
            } else {
                let prediction = number_prediction(before);
                let step = span.map_or(Ok(Step::Same), |span| step_of(kind, span))?;
                if step != prediction.map_or(Step::Same, Step::Number) {
                    mask |= 1 << field;
                    put_varint(
                        &mut out.0[FIELDS + field],
                        residual_code(
                            kind,
                            prediction.or_else(|| {
                                super::advance_change(table, layout, bases, key, field)
                            }),
                            step,
                        )?,
                    );
                }
                number_motion(step, before, layout)
            };
            if let Some(motion) = motion {
                next.push(Moving { key, field, motion });
            }
        }
        if mask != 0 {
            match (&mut run, &mut coding) {
                (Some(run), Some(coding)) => coding.put_row(run, key, mask, width, out)?,
                _ => {
                    put_varint(&mut out.0[KEYS], key - previous_key);
                    out.0[MASKS].extend_from_slice(&mask.to_le_bytes()[..width]);
                }
            }
            previous_key = key;
            residuals += 1;
        }
    }
    drop(predicted);
    predictor.moving = next;
    if residuals == 0 && deleted.is_empty() {
        for (column, length) in out.0.iter_mut().zip(lengths) {
            column.truncate(length);
        }
        for (column, length) in out.1.iter_mut().zip(alternates) {
            if let (Some(column), Some(length)) = (column, length) {
                column.truncate(length);
            }
        }
        return Ok(false);
    }
    // A modelled entry's rows are stored once the entry is kept.
    if let (Some(run), Some(coding)) = (run, &mut coding) {
        coding.finish_run(run, out)?;
    }
    put_varint(&mut out.0[CONTROL], residuals as u128);
    Ok(true)
}

/// Write the plain delta of a residual entry, or with no `input` of a frame
/// with no entry, and move the predictor on to its changes; `bases` are the
/// byte fields' encodings before it. With `coding` (a keyed block), row keys
/// and masks are stored against the rows and masks before them, or as model
/// decisions (a modelled block).
#[allow(clippy::too_many_arguments)]
pub(super) fn take_residual(
    table: TableId,
    fields: usize,
    previous: Hour,
    mut input: Option<&mut ColumnReader<'_>>,
    predictor: &mut Predictor,
    bases: &Shadow,
    plain: &mut Vec<u8>,
    budget: &Budget,
    layout: Layout,
    mut coding: Option<RowCoding<'_>>,
) -> io::Result<()> {
    let start = plain.len();
    plain.push(DELTA);
    plain.extend_from_slice(&previous.to_le_bytes());
    let count_at = plain.len();
    plain.extend_from_slice(&0_u32.to_le_bytes());
    // Every stored key has a key byte, so counts are bounded by the keys
    // column; a modelled row can cost less, so the rows held at the frame
    // before count too.
    let count = |input: &mut ColumnReader<'_>, held: usize| -> io::Result<usize> {
        let count = varint_usize(&mut input.0[CONTROL])?;
        if count > input.0[KEYS].remaining().saturating_add(held) {
            return Err(invalid("grouped row count exceeds its keys"));
        }
        Ok(count)
    };
    let mut deleted = Vec::new();
    let mut residuals = 0;
    if let Some(input) = input.as_deref_mut() {
        let mut key = 0_u128;
        let mut run = coding.as_ref().map(|coding| KeyReader::new(coding.keys));
        for index in 0..count(input, 0)? {
            if let (Some(run), Some(coding)) = (&mut run, &mut coding) {
                let (key, _) = run.take(table, &mut input.0[KEYS])?;
                coding.note_deleted(key);
                deleted.push(key);
                continue;
            }
            let difference = input.0[KEYS].varint()?;
            if index != 0 && difference == 0 {
                return Err(invalid("deleted keys did not advance"));
            }
            key = key
                .checked_add(difference)
                .ok_or_else(|| invalid("row key addition overflow"))?;
            table.check_key(key)?;
            deleted.push(key);
        }
        residuals = count(input, coding.as_ref().map_or(0, RowCoding::held_rows))?;
    }
    let mut run = match (&mut coding, input.as_deref_mut()) {
        (Some(coding), Some(input)) => Some(coding.read(table, residuals, &mut input.0)?),
        _ => None,
    };
    let schema = table.schema();
    let width = fields.div_ceil(8);
    let mut cursor = Cursor {
        moving: &predictor.moving,
    };
    let mut next = Vec::with_capacity(predictor.moving.len());
    let mut predicted = predicted_keys(&predictor.moving, &deleted).peekable();
    let mut residual_key = 0_u128;
    let mut read = 0_usize;
    let mut stored: Option<(u128, u64)> = None;
    let mut rows = 0_u32;
    let mut previous_key = 0_u128;
    let mut values = Vec::new();
    loop {
        if stored.is_none() && read < residuals {
            let input = input.as_deref_mut().expect("residual rows have an entry");
            let mask = match (&mut run, &mut coding) {
                (Some(run), Some(coding)) => {
                    let (key, mask) = coding.take_row(run, table, width, &mut input.0)?;
                    residual_key = key;
                    mask
                }
                _ => {
                    let difference = input.0[KEYS].varint()?;
                    if read != 0 && difference == 0 {
                        return Err(invalid("residual row keys did not advance"));
                    }
                    residual_key = residual_key
                        .checked_add(difference)
                        .ok_or_else(|| invalid("row key addition overflow"))?;
                    table.check_key(residual_key)?;
                    let mut bytes = [0; 8];
                    bytes[..width].copy_from_slice(input.0[MASKS].take(width)?);
                    u64::from_le_bytes(bytes)
                }
            };
            read += 1;
            if mask == 0 || (fields < 64 && mask >> fields != 0) {
                return Err(invalid("invalid residual field mask"));
            }
            stored = Some((residual_key, mask));
        }
        let key = match (stored, predicted.peek()) {
            (None, None) => break,
            (Some((key, _)), None) => key,
            (None, Some(&key)) => key,
            (Some((key, _)), Some(&predicted)) => key.min(predicted),
        };
        let mask = match stored {
            Some((stored_key, mask)) if stored_key == key => {
                stored = None;
                mask
            }
            _ => 0,
        };
        predicted.next_if(|predicted| *predicted == key);
        if mask != 0 && deleted.binary_search(&key).is_ok() {
            return Err(invalid("residual row is also deleted"));
        }
        values.clear();
        let mut plain_mask = 0_u64;
        for (field, &kind) in schema[..fields].iter().enumerate() {
            let stored_field = mask & (1 << field) != 0;
            let before = cursor.find(key, field);
            let motion = if table.byte_field(field) {
                let base = base(bases, key, field);
                let prediction = byte_prediction(base, before);
                let at = values.len();
                let changed = if stored_field {
                    let input = input.as_deref_mut().expect("stored fields have an entry");
                    let used = plain.len() - start + at;
                    take_byte_field(
                        table,
                        field,
                        &mut input.0[FIELDS + field],
                        base,
                        prediction.as_deref(),
                        &mut values,
                        used,
                        budget,
                        layout,
                    )?
                } else if let Some(prediction) = &prediction {
                    values.extend_from_slice(prediction);
                    true
                } else {
                    false
                };
                if !changed {
                    continue;
                }
                plain_mask |= 1 << field;
                byte_motion(base, &values[at..], before)
            } else {
                let prediction = number_prediction(before);
                let step = if stored_field {
                    let input = input.as_deref_mut().expect("stored fields have an entry");
                    let zero = prediction
                        .or_else(|| super::advance_change(table, layout, bases, key, field));
                    step_from_code(kind, prediction, zero, input.0[FIELDS + field].varint()?)?
                } else {
                    prediction.map_or(Step::Same, Step::Number)
                };
                if step != Step::Same {
                    plain_mask |= 1 << field;
                    put_step(kind, step, &mut values);
                }
                number_motion(step, before, layout)
            };
            if let Some(motion) = motion {
                next.push(Moving { key, field, motion });
            }
        }
        if plain_mask != 0 {
            put_varint(plain, key - previous_key);
            previous_key = key;
            plain.extend_from_slice(&plain_mask.to_le_bytes()[..width]);
            plain.extend_from_slice(&values);
            rows = rows
                .checked_add(1)
                .ok_or_else(|| invalid("too many residual rows"))?;
            budget.check(plain.len() - start)?;
        }
    }
    drop(predicted);
    if input.is_some() && rows == 0 && deleted.is_empty() {
        return Err(invalid("residual entry changes nothing"));
    }
    plain[count_at..count_at + 4].copy_from_slice(&rows.to_le_bytes());
    plain.extend_from_slice(&(deleted.len() as u32).to_le_bytes());
    let mut previous_key = 0;
    for &key in &deleted {
        put_varint(plain, key - previous_key);
        previous_key = key;
    }
    predictor.moving = next;
    budget.spend(plain.len() - start)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        format::FormatVersion,
        frame::{TableUpdate, decode_update},
        rows::compatibility::HISTORY,
    };
    use std::sync::atomic::AtomicUsize;

    const TABLE: TableId = TableId::Divisions;
    /// DivisionRow organisation.
    const FIELD: usize = 6;

    fn fields() -> usize {
        TABLE.field_count()
    }

    /// A plain continuous delta changing division 3's organisation by `delta`.
    fn plain(delta: i128) -> Vec<u8> {
        changes(&[(3, delta)])
    }

    /// A plain continuous delta changing each division's organisation, by
    /// key, by its nonzero amount.
    fn changes(rows: &[(u128, i128)]) -> Vec<u8> {
        let rows: Vec<_> = rows.iter().filter(|(_, delta)| *delta != 0).collect();
        let mut plain = vec![DELTA];
        plain.extend_from_slice(&9_i32.to_le_bytes());
        plain.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        let mut previous = 0;
        for (key, delta) in rows {
            put_varint(&mut plain, key - previous);
            previous = *key;
            plain.extend_from_slice(&(1_u64 << FIELD).to_le_bytes()[..fields().div_ceil(8)]);
            put_varint(&mut plain, zigzag(*delta));
        }
        plain.extend_from_slice(&0_u32.to_le_bytes());
        plain
    }

    /// Division 3 changing by `delta` beside division 5 rising steadily, so
    /// an hour without division 3's change still has a delta.
    fn busy(delta: i128) -> Vec<u8> {
        changes(&[(3, delta), (5, 1)])
    }

    fn take(columns: Option<&Columns>, predictor: &mut Predictor) -> Result<Vec<u8>, String> {
        take_in(Layout::Predicted, columns, predictor)
    }

    fn take_in(
        layout: Layout,
        columns: Option<&Columns>,
        predictor: &mut Predictor,
    ) -> Result<Vec<u8>, String> {
        let raw = columns.map(Columns::raw);
        let mut input = raw
            .as_ref()
            .map(|raw| ColumnReader::open(raw, fields()).unwrap());
        let budget = Budget(AtomicUsize::new(1 << 20));
        let mut out = Vec::new();
        take_residual(
            TABLE,
            fields(),
            9,
            input.as_mut(),
            predictor,
            &Shadow::new(),
            &mut out,
            &budget,
            layout,
            None,
        )
        .map_err(|error| error.to_string())?;
        if let Some(input) = input {
            input.finish().unwrap();
        }
        Ok(out)
    }

    #[test]
    fn residual_entries_rebuild_their_plain_deltas_and_keep_one_off_predictions() {
        let mut writer = Predictor::default();
        let mut reader = Predictor::default();
        // A countdown, a one-off reset, the countdown again, then a stop.
        for (hour, delta) in [-1, -1, -1, 40, -1, -1].into_iter().enumerate() {
            let plain = plain(delta);
            let mut columns = Columns::new(fields());
            let stored = put_residual(
                TABLE,
                FormatVersion::CURRENT.version(),
                fields(),
                &plain,
                &mut writer,
                &Shadow::new(),
                &mut columns,
                Layout::Predicted,
                None,
            )
            .unwrap();
            // The first two changes and the reset are stored, never the
            // change after the reset.
            assert_eq!(stored, matches!(hour, 0 | 1 | 3), "hour {hour}");
            let mut control = Decoder::new(&columns.0[CONTROL]);
            if stored {
                assert_eq!(control.varint().unwrap(), u128::from(RESIDUAL));
                columns.0[CONTROL].remove(0);
            }
            assert_eq!(
                take(stored.then_some(&columns), &mut reader).unwrap(),
                plain
            );
            assert_eq!(reader.moving(), writer.moving());
        }
        assert_eq!(
            writer.moving()[0].motion,
            Motion::Number {
                delta: -1,
                earlier: None,
                prediction: Some(-1)
            }
        );
        // A frame with no entry is the predicted change.
        let mut predicted = Predictor {
            moving: writer.moving().to_vec(),
        };
        let rebuilt = take(None, &mut predicted).unwrap();
        assert_eq!(rebuilt, plain(-1));
        let mut input = Decoder::new(&rebuilt);
        let update = decode_update(
            TABLE,
            &HISTORY[TABLE.index()],
            FormatVersion::CURRENT,
            false,
            &mut input,
        )
        .unwrap();
        assert!(matches!(update, TableUpdate::Delta { changed, .. } if changed[0].key == 3));
    }

    #[test]
    fn a_wrapping_clock_costs_one_correction_per_wrap() {
        let mut writer = Predictor::default();
        let mut stored = Vec::new();
        // Three days of a local hour: +1 an hour and -23 at midnight.
        for hour in 1..=72 {
            let delta = if hour % 24 == 0 { -23 } else { 1 };
            let mut columns = Columns::new(fields());
            if put_residual(
                TABLE,
                FormatVersion::CURRENT.version(),
                fields(),
                &plain(delta),
                &mut writer,
                &Shadow::new(),
                &mut columns,
                Layout::Predicted,
                None,
            )
            .unwrap()
            {
                stored.push(hour);
            }
        }
        // The first two changes, then each wrap and nothing after it.
        assert_eq!(stored, [1, 2, 24, 48, 72]);
    }

    /// The hours whose change `layout` stores, of a field changing by `deltas`.
    fn stored_hours(layout: Layout, deltas: &[i128]) -> Vec<usize> {
        stored_hours_of(layout, deltas, plain)
    }

    /// The hours with an entry of the plain deltas `delta` builds.
    fn stored_hours_of(
        layout: Layout,
        deltas: &[i128],
        delta: impl Fn(i128) -> Vec<u8>,
    ) -> Vec<usize> {
        let mut writer = Predictor::default();
        let mut reader = Predictor::default();
        let mut stored = Vec::new();
        for (hour, &change) in deltas.iter().enumerate() {
            let plain = delta(change);
            let mut columns = Columns::new(fields());
            let entry = put_residual(
                TABLE,
                FormatVersion::CURRENT.version(),
                fields(),
                &plain,
                &mut writer,
                &Shadow::new(),
                &mut columns,
                layout,
                None,
            )
            .unwrap();
            if entry {
                stored.push(hour + 1);
                columns.0[CONTROL].remove(0);
            }
            assert_eq!(
                take_in(layout, entry.then_some(&columns), &mut reader).unwrap(),
                plain,
                "hour {}",
                hour + 1
            );
            assert_eq!(reader.moving(), writer.moving());
        }
        stored
    }

    #[test]
    fn keyed_blocks_predict_a_ramping_change_and_earlier_layouts_do_not() {
        // A supply grace refilling 0.01 hour faster each hour up to 0.15 an
        // hour, then steadily.
        let deltas: Vec<i128> = (1..=15).chain([15; 10]).collect();
        // Its first three changes and the correction where it levels off.
        assert_eq!(stored_hours(Layout::Keyed, &deltas), [1, 2, 3, 16]);
        // Earlier layouts store every change until two repeat.
        assert_eq!(
            stored_hours(Layout::Predicted, &deltas),
            (1..=16).collect::<Vec<_>>()
        );
        // A wrapping clock costs the keyed layout no more than before.
        let clock: Vec<i128> = (1..=72)
            .map(|hour| if hour % 24 == 0 { -23 } else { 1 })
            .collect();
        assert_eq!(stored_hours(Layout::Keyed, &clock), [1, 2, 24, 48, 72]);
    }

    #[test]
    fn ramped_blocks_predict_fractional_rates_through_unchanged_hours() {
        let stored_hours = |layout, deltas: &[i128]| stored_hours_of(layout, deltas, busy);
        // An acclimatization factor falling 7.5 an hour.
        let alternating: Vec<i128> = (0..40).map(|t| if t % 2 == 0 { -8 } else { -7 }).collect();
        assert_eq!(stored_hours(Layout::Ramped, &alternating), [1, 2, 3]);
        assert_eq!(stored_hours(Layout::Keyed, &alternating).len(), 40);
        // Organisation regained 0.127 an hour, stored in tenths.
        let organisation: Vec<i128> = (1..=200)
            .map(|t: i128| (30 + 127 * t) / 100 - (30 + 127 * (t - 1)) / 100)
            .collect();
        let ramped = stored_hours(Layout::Ramped, &organisation).len();
        let keyed = stored_hours(Layout::Keyed, &organisation).len();
        assert!(ramped * 3 < keyed * 2, "ramped {ramped}, keyed {keyed}");
        // A change every fifth hour starts no rate and costs what it did.
        let sparse: Vec<i128> = (0..60).map(|t| if t % 5 == 4 { -72 } else { 0 }).collect();
        assert_eq!(
            stored_hours(Layout::Ramped, &sparse),
            stored_hours(Layout::Keyed, &sparse)
        );
        // What the keyed layout predicts costs the ramped one no more.
        let clock: Vec<i128> = (1..=72)
            .map(|hour| if hour % 24 == 0 { -23 } else { 1 })
            .collect();
        assert_eq!(stored_hours(Layout::Ramped, &clock), [1, 2, 24, 48, 72]);
        assert_eq!(stored_hours(Layout::Ramped, &[5; 20]), [1, 2]);
    }

    #[test]
    fn residual_entries_refuse_restated_predictions_and_empty_changes() {
        let moving = || Predictor {
            moving: vec![Moving {
                key: 3,
                field: FIELD,
                motion: Motion::Number {
                    delta: 5,
                    earlier: None,
                    prediction: Some(5),
                },
            }],
        };
        let entry = |code: u128| {
            let mut columns = Columns::new(fields());
            put_varint(&mut columns.0[CONTROL], 0);
            put_varint(&mut columns.0[CONTROL], 1);
            put_varint(&mut columns.0[KEYS], 3);
            columns.0[MASKS]
                .extend_from_slice(&(1_u64 << FIELD).to_le_bytes()[..fields().div_ceil(8)]);
            put_varint(&mut columns.0[FIELDS + FIELD], code);
            take(Some(&columns), &mut moving())
        };
        // Code 1 is a zero correction: the prediction itself.
        assert!(entry(1).unwrap_err().contains("noncanonical"));
        // Code 0 stops the field, and a stop alone changes nothing.
        assert!(entry(0).unwrap_err().contains("changes nothing"));
        // A correction of 2 is a change of 7.
        assert_eq!(entry(zigzag(2) + 1).unwrap(), plain(7));
    }
}

#[cfg(test)]
#[path = "ramp_census.rs"]
mod ramp_census;
