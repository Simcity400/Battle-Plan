//! Column-major encoding of a keyed-set baseline for layout measurement.
//!
//! The format uses row-major only if it wins on the reference domain. This is
//! the other arm: identical information, grouped by field instead of by row, so
//! like values sit together and the compressor sees them as one run.
//!
//! Nothing here is a commitment. It exists to be measured against `body.rs`, and
//! the loser is deleted rather than kept as an option.

use crate::bitmap::Bitmap;
use crate::body::{Key, KeyedState};
use crate::value::escapes;
use crate::schema::{Collection, Field, Kind, Shape};
use crate::varint::{write_varint, write_zigzag, Result};

fn write_key_component(out: &mut Vec<u8>, kind: Kind, value: i128) {
    match kind {
        Kind::Signed => write_zigzag(out, value as i64),
        _ => write_varint(out, value as u64),
    }
}

/// Column-major baseline: keys first, then one contiguous column per field.
///
/// The row-major form interleaves a division's province id with its organisation
/// and its supply, so the compressor sees province, org, supply, province, org,
/// supply. Columnar puts all 3,051 province ids together, then all 3,051
/// organisations. Whether that pays after zstd is exactly the open question.
pub fn encode_baseline_columnar(
    collection: &Collection,
    state: &KeyedState,
    out: &mut Vec<u8>,
) -> Result<()> {
    // collection-shape contract: this arm implements the keyed-set grammar only, exactly as the
    // row-major one does. Without the gate it panicked on an occurrence list —
    // which has no key fields, so the key reader indexes an empty list — and
    // silently encoded an ordered keyed list with the sorted-delta key grammar,
    // whose components key-encoding contract requires to be absolute and in source order.
    // This is a deliberate measurement-arm boundary: no columnar
    // delta or ordered-collection grammar was specified, so extending it would
    // invent a new wire format rather than measure the closed row/column question.
    if !matches!(collection.shape, Shape::KeyedSet) {
        return Err(crate::varint::WireError::Corrupt(
            "ordered collections are not representable as a keyed state",
        ));
    }
    write_varint(out, state.len() as u64);

    // Key column, using the same sorted-difference scheme as row-major (key-encoding contract).
    let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
    if kinds.is_empty() {
        return Err(crate::varint::WireError::Corrupt("keyed collection has no key fields"));
    }
    let keys: Vec<&Key> = state.keys().collect();

    // The same record-state contract and key-encoding contract checks the row-major path makes, before any bytes are
    // written. Sharing only the per-value rules was not enough: key arity is
    // checked here because the component loop indexes `kinds` directly, and
    // record validity because a row this arm cannot represent must be an error
    // rather than a short column no reader can realign.
    for key in &keys {
        crate::body::check_key(&kinds, key)?;
    }
    for key in &keys {
        state[*key].validate(collection)?;
    }

    let mut previous_first: Option<i128> = None;
    for key in &keys {
        match previous_first {
            None => write_key_component(out, kinds[0], key.first()),
            Some(prev) => write_varint(out, (key.first() - prev) as u64),
        }
        previous_first = Some(key.first());
        for (i, component) in key.0.iter().enumerate().skip(1) {
            write_key_component(out, kinds[i], *component);
        }
    }

    let fields: Vec<&Field> = collection.value_fields().collect();
    let rows: Vec<&crate::record::Record> = keys.iter().map(|k| &state[*k]).collect();

    // Presence: one column of `row_count` bits per optional field, rather than a
    // per-row bitmap. A field absent on every row becomes a run of zeroes.
    for (field_index, field) in fields.iter().enumerate() {
        if !field.optional {
            continue;
        }
        let mut column = Bitmap::with_len(rows.len());
        for (i, row) in rows.iter().enumerate() {
            column.set(i, row.values[field_index].is_present());
        }
        column.encode(out);
    }

    // Then one value column per field, carrying only the rows that have a value.
    for (field_index, field) in fields.iter().enumerate() {
        let mut bits: Vec<bool> = Vec::new();
        #[allow(unused_mut)]
        let mut tail: Vec<u8> = Vec::new();

        // Null / unknown / escape columns, each over the rows where the field is
        // present, so a field that is never null costs nothing.
        if field.nullable {
            let mut column = Vec::new();
            for row in &rows {
                if row.values[field_index].is_present() {
                    column.push(row.values[field_index].is_null());
                }
            }
            Bitmap::from(column).encode(out);
        }
        if field.unknown_capable {
            let mut column = Vec::new();
            for row in &rows {
                if row.values[field_index].is_present() {
                    column.push(row.values[field_index].is_unknown());
                }
            }
            Bitmap::from(column).encode(out);
        }
        if field.quant.is_quantized() {
            let mut column = Vec::new();
            for row in &rows {
                let v = row.values[field_index];
                if v.is_present() {
                    column.push(match v {
                        crate::value::Value::Continuous(x) => {
                            v.carries_value() && escapes(x, field.quant)
                        }
                        _ => false,
                    });
                }
            }
            Bitmap::from(column).encode(out);
        }

        for row in &rows {
            let value = row.values[field_index];
            if !value.carries_value() {
                continue;
            }
            encode_columnar_value(field, value, &mut bits, &mut tail)?;
        }
        Bitmap::from(bits).encode(out);
        out.extend_from_slice(&tail);
    }
    Ok(())
}

/// The columnar arm rearranges *where* values go, not what they are, so it
/// delegates the value rules to the row-major encoder rather than keeping a copy.
///
/// The copy is how this arm came to accept an out-of-range enum code that the
/// row-major path rejected, and to refuse the `Signed` stored value a derived
/// field legitimately carries. A measurement arm that validates differently is
/// measuring a different format.
fn encode_columnar_value(
    field: &Field,
    value: crate::value::Value,
    bits: &mut Vec<bool>,
    tail: &mut Vec<u8>,
) -> Result<()> {
    crate::record::encode_plain_value(field, value, bits, tail)
}
