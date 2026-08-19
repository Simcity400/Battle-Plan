//! The update-row contract update row: a change mask, new state bits for the changed fields
//! only, then their values.
//!
//! An insert carries a full record-state contract record; an update carries only what moved. The
//! reader chooses between them by looking the key up in the state it has already
//! reconstructed, so the distinction costs no discriminator byte and cannot
//! disagree between implementations.
//!
//! This is also the only place schema-kind contract's monotone differencing has any effect, since
//! it is defined against a reconstructed prior value that exists only here. derived-field contract's
//! derived rule is *not* one of those: it holds in every frame, so it lives in
//! `record::encode_plain_value` where baselines and inserts reach it too.

use crate::bitmap::Bitmap;
use crate::record::{decode_plain_value, encode_plain_value, packed_bits, Record};
use crate::schema::{Collection, Field, Kind};
use crate::value::{escapes, Value};
use crate::varint::{read_varint, read_zigzag, write_varint, write_zigzag, Result, WireError};

/// Per-field state as the change mask and its bit groups describe it.
#[derive(Clone, Copy)]
struct NewState {
    present: bool,
    null: bool,
    unknown: bool,
    escaped: bool,
}

impl NewState {
    fn carries_value(self) -> bool {
        self.present && !self.null && !self.unknown
    }
}

pub fn encode(
    collection: &Collection,
    prior: &Record,
    next: &Record,
    out: &mut Vec<u8>,
) -> Result<()> {
    let fields: Vec<&Field> = collection.value_fields().collect();
    // Both records in full, not just the changed fields. An update only routes
    // what moved through the value encoder, so a record-state contract violation sitting on an
    // unchanged field — a derived field holding its live variant, say — reached
    // the wire here while `Record::encode` rejected the identical record.
    prior.validate(collection)?;
    next.validate(collection)?;

    // delta-minimality contract: a mask bit is set if and only if that field's state or value differs.
    // A bit on an unchanged field is a second encoding of the same row.
    // Stored equality, not `==`: under `fixed(2)` a field moving 0.33 → 0.331 is
    // the same stored value, and a mask bit on it would encode a row the decoder
    // reconstructs as unchanged and rejects.
    let mut mask = Bitmap::with_len(fields.len());
    for (i, (p, n)) in prior.values.iter().zip(&next.values).enumerate() {
        mask.set(i, !crate::value::stored_eq(fields[i], *p, *n));
    }
    if mask.count_set() == 0 {
        return Err(WireError::Corrupt("update row with an all-zero change mask"));
    }
    mask.encode(out);

    let changed: Vec<(usize, &Field)> = mask.iter_set().map(|i| (i, fields[i])).collect();

    let mut presence_bits = Vec::new();
    let mut null_bits = Vec::new();
    let mut unknown_bits = Vec::new();
    let mut escape_bits = Vec::new();
    for (i, field) in &changed {
        let v = next.values[*i];
        if field.optional {
            presence_bits.push(v.is_present());
        } else if !v.is_present() {
            return Err(WireError::Corrupt("required field became absent"));
        }
        if field.nullable {
            null_bits.push(v.is_null());
        } else if v.is_null() {
            return Err(WireError::Corrupt("null on a non-nullable field"));
        }
        if field.unknown_capable {
            unknown_bits.push(v.is_unknown());
        } else if v.is_unknown() {
            return Err(WireError::Corrupt("unknown on a non-unknown-capable field"));
        }
        if field.quant.is_quantized() {
            let fires = match v {
                Value::Continuous(x) => v.carries_value() && escapes(x, field.quant),
                _ => false,
            };
            escape_bits.push(fires);
        }
    }
    Bitmap::from(presence_bits).encode(out);
    Bitmap::from(null_bits).encode(out);
    Bitmap::from(unknown_bits).encode(out);
    Bitmap::from(escape_bits).encode(out);

    let mut bits: Vec<bool> = Vec::new();
    let mut tail: Vec<u8> = Vec::new();
    for (i, field) in &changed {
        let v = next.values[*i];
        if !v.carries_value() {
            continue;
        }
        encode_delta_value(field, prior.values[*i], v, &mut bits, &mut tail)?;
    }
    Bitmap::from(bits).encode(out);
    out.extend_from_slice(&tail);
    Ok(())
}

pub fn decode(
    collection: &Collection,
    prior: &Record,
    buf: &[u8],
    pos: usize,
) -> Result<(Record, usize)> {
    let fields: Vec<&Field> = collection.value_fields().collect();
    // The prior comes from the caller's reconstructed state and every index below
    // reaches into it, so a short one panicked rather than erroring. Arity alone
    // is not enough either: the decoder carries every unchanged field straight
    // through, so an invalid prior — an out-of-range enum, say — would survive
    // into reconstructed state and into every later frame built from it.
    prior.validate(collection)?;
    let (mask, mut pos) = Bitmap::decode(buf, pos, fields.len())?;
    if mask.count_set() == 0 {
        return Err(WireError::Corrupt("update row with an all-zero change mask"));
    }
    let changed: Vec<(usize, &Field)> = mask.iter_set().map(|i| (i, fields[i])).collect();

    let count = |f: fn(&Field) -> bool| changed.iter().filter(|(_, fld)| f(fld)).count();
    let (presence, p) = Bitmap::decode(buf, pos, count(|f| f.optional))?;
    pos = p;
    let (nulls, p) = Bitmap::decode(buf, pos, count(|f| f.nullable))?;
    pos = p;
    let (unknowns, p) = Bitmap::decode(buf, pos, count(|f| f.unknown_capable))?;
    pos = p;
    let (escaped_map, p) = Bitmap::decode(buf, pos, count(|f| f.quant.is_quantized()))?;
    pos = p;

    // Resolve every changed field's new state before reading a single value byte.
    let mut states: Vec<NewState> = Vec::with_capacity(changed.len());
    let (mut pi, mut ni, mut ui, mut ei) = (0usize, 0usize, 0usize, 0usize);
    for (_, field) in &changed {
        let present = if field.optional {
            let v = presence.get(pi);
            pi += 1;
            v
        } else {
            true
        };
        let null = if field.nullable {
            let v = nulls.get(ni);
            ni += 1;
            v
        } else {
            false
        };
        let unknown = if field.unknown_capable {
            let v = unknowns.get(ui);
            ui += 1;
            v
        } else {
            false
        };
        let escaped = if field.quant.is_quantized() {
            let v = escaped_map.get(ei);
            ei += 1;
            v
        } else {
            false
        };
        if null && unknown {
            return Err(WireError::Corrupt("field sets both null and unknown"));
        }
        // update-row contract: "a changed field whose new presence bit is 0 has its null,
        // unknown and escape bits 0". Without this, `became absent` has four
        // encodings instead of one — the same defect as an unconstrained escape
        // bit, and it survives a round trip because the decoder never reads the
        // bits it accepted.
        if !present && (null || unknown || escaped) {
            return Err(WireError::Corrupt("state bits set on a field that became absent"));
        }
        let state = NewState { present, null, unknown, escaped };
        if escaped && !state.carries_value() {
            return Err(WireError::Corrupt("escape bit set on a field carrying no value"));
        }
        states.push(state);
    }

    let mut bit_width = 0usize;
    for ((_, field), state) in changed.iter().zip(&states) {
        if state.carries_value() {
            bit_width += packed_bits(field, state.escaped);
        }
    }
    let (packed, p) = Bitmap::decode(buf, pos, bit_width)?;
    pos = p;

    let mut values = prior.values.clone();
    let mut bit_cursor = 0usize;
    for ((index, field), state) in changed.iter().zip(&states) {
        values[*index] = if !state.present {
            Value::Absent
        } else if state.null {
            Value::Null
        } else if state.unknown {
            Value::Unknown
        } else {
            let (v, next) = decode_delta_value(
                field,
                prior.values[*index],
                state.escaped,
                &packed,
                &mut bit_cursor,
                buf,
                pos,
            )?;
            pos = next;
            v
        };
        // delta-minimality contract: a mask bit is set if and only if the field changed, so a bit on a
        // field that decodes back to the prior value is a second encoding of a
        // shorter row. The encoder cannot emit one; the decoder must not accept
        // one, or byte-identity between two writers is not enforceable.
        if crate::value::stored_eq(field, values[*index], prior.values[*index]) {
            return Err(WireError::Corrupt("change mask bit on a field that did not change"));
        }
    }
    Ok((Record::new(values), pos))
}

/// schema-kind contract: inside a `DELTA` a monotone counter is a `zigzag` difference against the
/// reconstructed prior value, and absolute when that prior has no concrete value.
///
/// derived-field contract's derived rule is frame-independent, so it lives in `encode_plain_value`;
/// the check here is only to keep a derived monotone field off the differencing
/// path, since derived-field contract overrides the kind-2 rule.
fn encode_delta_value(
    field: &Field,
    prior: Value,
    value: Value,
    bits: &mut Vec<bool>,
    tail: &mut Vec<u8>,
) -> Result<()> {
    if field.derived.is_none() && matches!(field.kind, Kind::Monotone) {
        return match (prior, value) {
            (Value::Unsigned(p), Value::Unsigned(v)) => {
                Ok(write_zigzag(tail, (v as i64).wrapping_sub(p as i64)))
            }
            (_, Value::Unsigned(v)) => Ok(write_varint(tail, v)),
            _ => Err(WireError::Corrupt("monotone field carries a non-unsigned value")),
        };
    }
    encode_plain_value(field, value, bits, tail)
}

fn decode_delta_value(
    field: &Field,
    prior: Value,
    escaped: bool,
    packed: &Bitmap,
    bit_cursor: &mut usize,
    buf: &[u8],
    pos: usize,
) -> Result<(Value, usize)> {
    if field.derived.is_none() && matches!(field.kind, Kind::Monotone) {
        return match prior {
            Value::Unsigned(p) => {
                let (d, next) = read_zigzag(buf, pos)?;
                Ok((Value::Unsigned((p as i64).wrapping_add(d) as u64), next))
            }
            _ => {
                let (v, next) = read_varint(buf, pos)?;
                Ok((Value::Unsigned(v), next))
            }
        };
    }
    decode_plain_value(field, escaped, packed, bit_cursor, buf, pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Field as F, Kind as K, Quant};

    fn collection() -> Collection {
        Collection {
            collection_id: 1,
            shape: crate::schema::Shape::KeyedSet,
            fields: vec![
                F::new(1, K::Unsigned).key(0),
                F::new(2, K::Unsigned),
                F::new(3, K::Unsigned).optional().nullable(),
                F::new(4, K::Monotone),
                F::new(5, K::Continuous).quant(Quant::Fixed(2)),
            ],
        }
    }

    fn rec(a: u64, b: Value, c: u64, d: f64) -> Record {
        Record::new(vec![Value::Unsigned(a), b, Value::Unsigned(c), Value::Continuous(d)])
    }

    #[test]
    fn an_update_carries_only_what_moved() {
        let c = collection();
        let prior = rec(1, Value::Unsigned(5), 100, 0.5);
        let next = rec(2, Value::Unsigned(5), 100, 0.5);

        let mut update = Vec::new();
        encode(&c, &prior, &next, &mut update).unwrap();
        let mut full = Vec::new();
        next.encode(&c, &mut full).unwrap();
        assert!(
            update.len() < full.len(),
            "update {} bytes vs full record {} bytes",
            update.len(),
            full.len()
        );

        let (back, pos) = decode(&c, &prior, &update, 0).unwrap();
        assert_eq!(pos, update.len());
        assert_eq!(back, next);
    }

    #[test]
    fn an_all_zero_change_mask_is_rejected() {
        let c = collection();
        let prior = rec(1, Value::Unsigned(5), 100, 0.5);
        let mut out = Vec::new();
        assert_eq!(
            encode(&c, &prior, &prior.clone(), &mut out),
            Err(WireError::Corrupt("update row with an all-zero change mask"))
        );
    }

    #[test]
    fn a_monotone_counter_encodes_as_a_difference() {
        let c = collection();
        let prior = rec(1, Value::Unsigned(5), 1_000_000, 0.5);
        let next = rec(1, Value::Unsigned(5), 1_000_001, 0.5);
        let mut out = Vec::new();
        encode(&c, &prior, &next, &mut out).unwrap();
        // The difference is 1, so the value costs one byte rather than the three
        // an absolute 1,000,001 would take.
        assert!(out.len() <= 4, "monotone delta should be tiny, got {out:?}");
        let (back, _) = decode(&c, &prior, &out, 0).unwrap();
        assert_eq!(back, next);
    }

    #[test]
    fn a_monotone_counter_may_legally_decrease() {
        // schema-kind contract: monotone is an encoding hint, never a validity constraint.
        let c = collection();
        let prior = rec(1, Value::Unsigned(5), 500, 0.5);
        let next = rec(1, Value::Unsigned(5), 400, 0.5);
        let mut out = Vec::new();
        encode(&c, &prior, &next, &mut out).unwrap();
        let (back, _) = decode(&c, &prior, &out, 0).unwrap();
        assert_eq!(back, next, "a negative delta is valid data");
    }

    #[test]
    fn a_value_becoming_null_is_expressible() {
        // The transition the extra state bits exist for: d56's repair target goes
        // non-null to null the moment a repair completes.
        let c = collection();
        let prior = rec(1, Value::Unsigned(9), 100, 0.5);
        let next = rec(1, Value::Null, 100, 0.5);
        let mut out = Vec::new();
        encode(&c, &prior, &next, &mut out).unwrap();
        let (back, _) = decode(&c, &prior, &out, 0).unwrap();
        assert_eq!(back.values[1], Value::Null);
    }

    #[test]
    fn a_value_becoming_absent_is_expressible() {
        let c = collection();
        let prior = rec(1, Value::Unsigned(9), 100, 0.5);
        let next = rec(1, Value::Absent, 100, 0.5);
        let mut out = Vec::new();
        encode(&c, &prior, &next, &mut out).unwrap();
        let (back, _) = decode(&c, &prior, &out, 0).unwrap();
        assert_eq!(back.values[1], Value::Absent);
    }
}
