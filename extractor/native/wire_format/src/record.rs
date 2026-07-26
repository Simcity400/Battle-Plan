//! Record encode and decode, per `WIRE_FORMAT.md` §3.
//!
//! Four schema-conditional bitmaps, then a bit-packed region, then value bytes.
//! Key fields never appear here — a keyed row is its key (§7.4) followed by a
//! record of the non-key fields only.

use crate::bitmap::Bitmap;
use crate::schema::{Collection, Field, Kind, Quant};
use crate::value::{canonical_f64_bits, dequantize, escapes, quantize, Value};
use crate::varint::{
    read_varint, read_zigzag, write_varint, write_zigzag, Result, WireError,
};

/// Which of the three conditional bitmaps a field qualifies for.
fn qualifies_null(f: &Field) -> bool {
    f.nullable
}
fn qualifies_unknown(f: &Field) -> bool {
    f.unknown_capable
}
fn qualifies_escape(f: &Field) -> bool {
    f.quant.is_quantized()
}

/// §3: a conditional bitmap's width counts the qualifying fields **present in
/// this row** — every required qualifying field, which has no presence bit and is
/// always present, plus every optional qualifying field the presence bitmap marks
/// present. Excluding required fields would leave a required nullable field with
/// two encodings.
fn present_qualifying<'a>(
    collection: &'a Collection,
    presence: &'a Bitmap,
    qualifies: fn(&Field) -> bool,
) -> Vec<&'a Field> {
    let mut optional_index = 0usize;
    let mut out = Vec::new();
    for field in collection.value_fields() {
        let present = if field.optional {
            let p = presence.get(optional_index);
            optional_index += 1;
            p
        } else {
            true
        };
        if present && qualifies(field) {
            out.push(field);
        }
    }
    out
}

fn schema_declares(collection: &Collection, qualifies: fn(&Field) -> bool) -> bool {
    collection.value_fields().any(qualifies)
}

/// One row's non-key values, in schema order.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub values: Vec<Value>,
}

impl Record {
    pub fn new(values: Vec<Value>) -> Self {
        Self { values }
    }

    /// Whether two records are the same **as stored** under `collection` — the
    /// equality §7.1 means by "differs". See `value::stored_eq`: for anything
    /// unquantized this is `==`, and for a quantized field it compares what the
    /// bytes will actually hold.
    ///
    /// A record of the wrong arity is never equal to anything; encoding will
    /// reject it, and silently reporting it equal would suppress the row that
    /// carries the error.
    pub fn stored_eq(&self, collection: &Collection, other: &Record) -> bool {
        let fields: Vec<&Field> = collection.value_fields().collect();
        if self.values.len() != fields.len() || other.values.len() != fields.len() {
            return false;
        }
        fields
            .iter()
            .enumerate()
            .all(|(i, f)| crate::value::stored_eq(f, self.values[i], other.values[i]))
    }

    /// §3 record validity, independent of layout.
    ///
    /// This is shared rather than inlined because the column-major arm needs the
    /// identical rules: when it carried its own copy it silently accepted a
    /// `Null` on a non-nullable field — writing no null column and no value, so
    /// the column ended up with fewer values than rows and nothing marking which
    /// — and panicked on a record of the wrong arity.
    pub fn validate(&self, collection: &Collection) -> Result<()> {
        let fields: Vec<&Field> = collection.value_fields().collect();
        if self.values.len() != fields.len() {
            return Err(WireError::Corrupt("record arity does not match schema"));
        }
        for (field, value) in fields.iter().zip(&self.values) {
            if !value.matches_field(field) {
                return Err(WireError::Corrupt("value does not match field kind"));
            }
            if !field.optional && !value.is_present() {
                return Err(WireError::Corrupt("required field is absent"));
            }
            if value.is_null() && !field.nullable {
                return Err(WireError::Corrupt("null on a non-nullable field"));
            }
            if value.is_unknown() && !field.unknown_capable {
                return Err(WireError::Corrupt(
                    "unknown on a field the schema does not declare unknown-capable",
                ));
            }
            // §4: "a value the writer cannot map into 0..n-1 is a failed sample
            // and a typed gap, never a coerced code." The check has to be here
            // and not only in the value encoder, because an update touches
            // nothing but its changed fields — an out-of-range code sitting on
            // an unchanged field would otherwise persist in reconstructed state.
            if let Value::Enum(code) = value {
                if *code >= field.enum_card.unwrap_or(0) {
                    return Err(WireError::Corrupt("enum code >= declared cardinality"));
                }
            }
        }
        Ok(())
    }

    pub fn encode(&self, collection: &Collection, out: &mut Vec<u8>) -> Result<()> {
        self.validate(collection)?;
        let fields: Vec<&Field> = collection.value_fields().collect();

        // Presence bitmap: always ceil(O/8), fixed by the schema alone.
        let mut presence = Bitmap::with_len(collection.optional_count());
        let mut optional_index = 0usize;
        for (field, value) in fields.iter().zip(&self.values) {
            if field.optional {
                presence.set(optional_index, value.is_present());
                optional_index += 1;
            }
        }
        presence.encode(out);

        // The three conditional bitmaps, each over its present qualifying fields.
        let null_fields = present_qualifying(collection, &presence, qualifies_null);
        let unknown_fields = present_qualifying(collection, &presence, qualifies_unknown);
        let escape_fields = present_qualifying(collection, &presence, qualifies_escape);

        let value_of = |target: &Field| -> Value {
            let index = fields
                .iter()
                .position(|f| f.field_id == target.field_id)
                .expect("field belongs to this collection");
            self.values[index]
        };

        if schema_declares(collection, qualifies_null) {
            let mut map = Bitmap::with_len(null_fields.len());
            for (i, f) in null_fields.iter().enumerate() {
                map.set(i, value_of(f).is_null());
            }
            map.encode(out);
        }
        if schema_declares(collection, qualifies_unknown) {
            let mut map = Bitmap::with_len(unknown_fields.len());
            for (i, f) in unknown_fields.iter().enumerate() {
                map.set(i, value_of(f).is_unknown());
            }
            map.encode(out);
        }
        if schema_declares(collection, qualifies_escape) {
            let mut map = Bitmap::with_len(escape_fields.len());
            for (i, f) in escape_fields.iter().enumerate() {
                let v = value_of(f);
                let fires = match v {
                    Value::Continuous(x) => v.carries_value() && escapes(x, f.quant),
                    _ => false,
                };
                map.set(i, fires);
            }
            map.encode(out);
        }

        // §3 validity: at most one of null and unknown, escape only with a value.
        for (field, value) in fields.iter().zip(&self.values) {
            if value.is_null() && value.is_unknown() {
                return Err(WireError::Corrupt("field sets both null and unknown"));
            }
            // A state the schema does not declare has nowhere to live: its bitmap
            // is not written, so encoding it would drop the state silently and
            // leave the reader expecting value bytes that were never emitted.
            if value.is_null() && !field.nullable {
                return Err(WireError::Corrupt("null on a field the schema does not declare nullable"));
            }
            if value.is_unknown() && !field.unknown_capable {
                return Err(WireError::Corrupt(
                    "unknown on a field the schema does not declare unknown-capable",
                ));
            }
            if let Value::Enum(code) = value {
                let n = field.enum_card.unwrap_or(0);
                if *code >= n {
                    return Err(WireError::Corrupt("enum code >= declared cardinality"));
                }
            }
        }

        // Bit-packed region, then value bytes.
        let mut bits: Vec<bool> = Vec::new();
        let mut tail: Vec<u8> = Vec::new();
        for (field, value) in fields.iter().zip(&self.values) {
            if !value.carries_value() {
                continue;
            }
            encode_plain_value(field, *value, &mut bits, &mut tail)?;
        }
        Bitmap::from(bits).encode(out);
        out.extend_from_slice(&tail);
        Ok(())
    }

    pub fn decode(collection: &Collection, buf: &[u8], pos: usize) -> Result<(Self, usize)> {
        let fields: Vec<&Field> = collection.value_fields().collect();
        let (presence, mut pos) = Bitmap::decode(buf, pos, collection.optional_count())?;

        let null_fields = present_qualifying(collection, &presence, qualifies_null);
        let unknown_fields = present_qualifying(collection, &presence, qualifies_unknown);
        let escape_fields = present_qualifying(collection, &presence, qualifies_escape);

        let mut null_map = Bitmap::default();
        if schema_declares(collection, qualifies_null) {
            let (m, p) = Bitmap::decode(buf, pos, null_fields.len())?;
            null_map = m;
            pos = p;
        }
        let mut unknown_map = Bitmap::default();
        if schema_declares(collection, qualifies_unknown) {
            let (m, p) = Bitmap::decode(buf, pos, unknown_fields.len())?;
            unknown_map = m;
            pos = p;
        }
        let mut escape_map = Bitmap::default();
        if schema_declares(collection, qualifies_escape) {
            let (m, p) = Bitmap::decode(buf, pos, escape_fields.len())?;
            escape_map = m;
            pos = p;
        }

        let index_in = |list: &[&Field], id: u32| list.iter().position(|f| f.field_id == id);

        // Resolve each field's state before reading any value bytes.
        let mut states: Vec<Value> = Vec::with_capacity(fields.len());
        let mut optional_index = 0usize;
        for field in &fields {
            let present = if field.optional {
                let p = presence.get(optional_index);
                optional_index += 1;
                p
            } else {
                true
            };
            if !present {
                states.push(Value::Absent);
                continue;
            }
            let is_null = index_in(&null_fields, field.field_id)
                .map_or(false, |i| null_map.get(i));
            let is_unknown = index_in(&unknown_fields, field.field_id)
                .map_or(false, |i| unknown_map.get(i));
            if is_null && is_unknown {
                return Err(WireError::Corrupt("field sets both null and unknown"));
            }
            if is_null || is_unknown {
                // §3: the escape bit is legal only alongside a value. A set bit
                // here is a second encoding of null/unknown, so the frame is
                // corrupt rather than leniently accepted.
                let escaped = index_in(&escape_fields, field.field_id)
                    .map_or(false, |i| escape_map.get(i));
                if escaped {
                    return Err(WireError::Corrupt(
                        "escape bit set on a field carrying no value",
                    ));
                }
            }
            states.push(if is_null {
                Value::Null
            } else if is_unknown {
                Value::Unknown
            } else {
                // Placeholder; replaced once the value regions are read.
                Value::Unsigned(0)
            });
        }

        // Bit-packed region: width is determined by the fields that carry values.
        let mut bit_width = 0usize;
        for (field, state) in fields.iter().zip(&states) {
            if !state.carries_value() {
                continue;
            }
            let escaped = index_in(&escape_fields, field.field_id)
                .map_or(false, |i| escape_map.get(i));
            bit_width += packed_bits(field, escaped);
        }
        let (packed, mut pos) = Bitmap::decode(buf, pos, bit_width)?;

        let mut bit_cursor = 0usize;
        let mut out_values = Vec::with_capacity(fields.len());
        for (field, state) in fields.iter().zip(&states) {
            if !state.carries_value() {
                out_values.push(*state);
                continue;
            }
            let escaped = index_in(&escape_fields, field.field_id)
                .map_or(false, |i| escape_map.get(i));
            let (value, next) =
                decode_plain_value(field, escaped, &packed, &mut bit_cursor, buf, pos)?;
            pos = next;
            out_values.push(value);
        }

        Ok((Self::new(out_values), pos))
    }
}

pub(crate) fn packed_bits(field: &Field, escaped: bool) -> usize {
    // A derived field's value lives in the tail as an absolute zigzag whatever
    // its kind, so it reserves no bit-packed region. Schema validation restricts
    // `derived` to the integer kinds, which reserve none anyway; the guard is
    // here so the three functions agree by construction rather than by luck.
    if field.derived.is_some() {
        return 0;
    }
    match field.kind {
        Kind::Boolean => 1,
        Kind::Enum => {
            let n = field.enum_card.unwrap_or(2).max(2);
            (u32::BITS - (n - 1).leading_zeros()) as usize
        }
        Kind::Continuous => match field.quant {
            Quant::Ratio(d) if !escaped => d as usize,
            _ => 0,
        },
        _ => 0,
    }
}

pub(crate) fn encode_plain_value(
    field: &Field,
    value: Value,
    bits: &mut Vec<bool>,
    tail: &mut Vec<u8>,
) -> Result<()> {
    // §6: a derived field's stored value "is always encoded as an absolute
    // zigzag, whatever its kind and whatever the frame". Baselines and inserts
    // reach this function too, so the rule belongs here and not only in the
    // update path — otherwise the same field encodes two different ways
    // depending on which frame carries it.
    if field.derived.is_some() {
        return match value {
            // One canonical stored representation, whatever the live kind.
            Value::Signed(v) => Ok(write_zigzag(tail, v)),
            _ => Err(WireError::Corrupt("derived stored value must be Signed")),
        };
    }
    match (field.kind, value) {
        (Kind::Boolean, Value::Bool(b)) => bits.push(b),
        (Kind::Enum, Value::Enum(code)) => {
            // §4: "a value the writer cannot map into 0..n-1 is a failed sample,
            // never a coerced code." The check lives here rather than only in
            // `Record::encode` because the update path writes values through this
            // function too, and there the truncation to the field's bit width was
            // silent — `Enum(4)` under `n = 3` encoded and decoded as `Enum(0)`.
            let n = field.enum_card.unwrap_or(0);
            if code >= n {
                return Err(WireError::Corrupt("enum code >= declared cardinality"));
            }
            let width = packed_bits(field, false);
            for i in 0..width {
                bits.push(code & (1 << i) != 0);
            }
        }
        (Kind::Unsigned, Value::Unsigned(v)) => write_varint(tail, v),
        (Kind::ContentId, Value::ContentId(v)) => write_varint(tail, v),
        (Kind::Monotone, Value::Unsigned(v)) => write_varint(tail, v),
        (Kind::Signed, Value::Signed(v)) => write_zigzag(tail, v),
        (Kind::Continuous, Value::Continuous(x)) => match field.quant {
            Quant::Ratio(d) if !escapes(x, field.quant) => {
                let raw = quantize(x, field.quant).unwrap_or(0) as u64;
                for i in 0..d as usize {
                    bits.push(raw & (1 << i) != 0);
                }
            }
            Quant::Fixed(_) if !escapes(x, field.quant) => {
                write_zigzag(tail, quantize(x, field.quant).unwrap_or(0));
            }
            // `exact`, or a quantized field whose escape bit fired: raw f64.
            _ => tail.extend_from_slice(&canonical_f64_bits(x).to_le_bytes()),
        },
        _ => return Err(WireError::Corrupt("value does not match field kind")),
    }
    Ok(())
}

pub(crate) fn decode_plain_value(
    field: &Field,
    escaped: bool,
    packed: &Bitmap,
    bit_cursor: &mut usize,
    buf: &[u8],
    pos: usize,
) -> Result<(Value, usize)> {
    if field.derived.is_some() {
        let (v, next) = read_zigzag(buf, pos)?;
        return Ok((Value::Signed(v), next));
    }
    match field.kind {
        Kind::Boolean => {
            let b = packed.get(*bit_cursor);
            *bit_cursor += 1;
            Ok((Value::Bool(b), pos))
        }
        Kind::Enum => {
            let width = packed_bits(field, escaped);
            let mut code = 0u32;
            for i in 0..width {
                if packed.get(*bit_cursor + i) {
                    code |= 1 << i;
                }
            }
            *bit_cursor += width;
            let n = field.enum_card.unwrap_or(0);
            if code >= n {
                return Err(WireError::Corrupt("enum code >= declared cardinality"));
            }
            Ok((Value::Enum(code), pos))
        }
        Kind::Unsigned => {
            let (v, next) = read_varint(buf, pos)?;
            Ok((Value::Unsigned(v), next))
        }
        Kind::ContentId => {
            let (v, next) = read_varint(buf, pos)?;
            Ok((Value::ContentId(v), next))
        }
        Kind::Monotone => {
            let (v, next) = read_varint(buf, pos)?;
            Ok((Value::Unsigned(v), next))
        }
        Kind::Signed => {
            let (v, next) = read_zigzag(buf, pos)?;
            Ok((Value::Signed(v), next))
        }
        Kind::Continuous => match field.quant {
            Quant::Ratio(d) if !escaped => {
                let mut raw = 0u64;
                for i in 0..d as usize {
                    if packed.get(*bit_cursor + i) {
                        raw |= 1 << i;
                    }
                }
                *bit_cursor += d as usize;
                Ok((Value::Continuous(dequantize(raw as i64, field.quant)), pos))
            }
            Quant::Fixed(_) if !escaped => {
                let (raw, next) = read_zigzag(buf, pos)?;
                let x = dequantize(raw, field.quant);
                // The dual of the check below, and needed for the same reason.
                // §6's fourth condition forces an unstable integer to escape, so
                // no legal writer emits one unescaped — and two unstable integers
                // can share a reconstruction, which would give one value two
                // reader-accepted encodings.
                if crate::value::quantize(x, field.quant) != Some(raw) {
                    return Err(WireError::Corrupt(
                        "stored integer is not its own reconstruction's quantization",
                    ));
                }
                Ok((Value::Continuous(x), next))
            }
            _ => {
                if pos + 8 > buf.len() {
                    return Err(WireError::Truncated);
                }
                let bits = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
                let x = f64::from_bits(bits);
                // §0: NaN is normalised, so a non-canonical payload is a second
                // encoding of the same value.
                if x.is_nan() && bits != crate::value::CANONICAL_NAN_BITS {
                    return Err(WireError::Corrupt("NaN payload is not the canonical pattern"));
                }
                // §6: the escape fires on exactly the listed conditions. A value
                // escaped that did not need to be is likewise a second encoding.
                if escaped && field.quant.is_quantized() && !escapes(x, field.quant) {
                    return Err(WireError::Corrupt(
                        "escape bit set on a value that does not satisfy an escape condition",
                    ));
                }
                Ok((Value::Continuous(x), pos + 8))
            }
        },
    }
}
