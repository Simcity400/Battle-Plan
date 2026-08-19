//! Field values, the four states, and quantization arithmetic.
//!
//! Absent, null, zero and unknown are four different things and stay four
//! different things through encode and decode. Collapsing any pair loses the
//! difference between "we know there is nothing", "we do not carry this" and
//! "we did not capture it".

use crate::schema::{Kind, Quant};

#[derive(Clone, Copy, Debug)]
pub enum Value {
    /// Not applicable, or unsupported by this schema. Presence bit `0`.
    Absent,
    /// Applicable; the relation genuinely points at nothing. No value bytes.
    Null,
    /// Applicable; not captured. No value bytes.
    Unknown,
    Unsigned(u64),
    Signed(i64),
    /// A `0..n-1` code; `>= n` is corrupt (schema-kind contract).
    Enum(u32),
    Bool(bool),
    ContentId(u64),
    Continuous(f64),
}

/// Equality is **canonical-byte** equality, not IEEE equality.
///
/// This is not a convenience: the only equality this format has is "would encode
/// to the same bytes", because that is what every rule is written against. The
/// differ emits a row iff its record changed, `check_minimal` verifies that, and
/// the update mask sets a bit iff a field changed — all three ask the same
/// question, and IEEE answers it wrong in both directions.
///
/// * `NaN != NaN` under IEEE, but canonical primitive contract normalises every NaN to one bit pattern, so
///   two NaNs are the same value. Left as IEEE, a field holding NaN re-encodes
///   on every tick forever and `check_minimal` rejects the differ's own output.
/// * `-0.0 == 0.0` under IEEE, but they encode differently — canonical primitive contract keeps the sign of
///   zero, and the escape rule (`escapes`) fires on `-0.0` alone. Left as IEEE,
///   a transition between them is silently dropped and reconstructed state
///   diverges from captured state.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Continuous(a), Value::Continuous(b)) => {
                canonical_f64_bits(*a) == canonical_f64_bits(*b)
            }
            (Value::Absent, Value::Absent)
            | (Value::Null, Value::Null)
            | (Value::Unknown, Value::Unknown) => true,
            (Value::Unsigned(a), Value::Unsigned(b)) => a == b,
            (Value::Signed(a), Value::Signed(b)) => a == b,
            (Value::Enum(a), Value::Enum(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::ContentId(a), Value::ContentId(b)) => a == b,
            _ => false,
        }
    }
}

impl Value {
    pub fn is_present(self) -> bool {
        !matches!(self, Value::Absent)
    }

    pub fn is_null(self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_unknown(self) -> bool {
        matches!(self, Value::Unknown)
    }

    /// A field carries value bytes only when present, non-null and non-unknown.
    pub fn carries_value(self) -> bool {
        !matches!(self, Value::Absent | Value::Null | Value::Unknown)
    }

    /// Whether this value is a legal *stored* value for `field`.
    ///
    /// A derived field is the one case where the stored value's type is not the
    /// field's kind: derived-field contract's transform "can carry an otherwise unsigned quantity
    /// negative", so the stored domain is all of `i64` whatever the live kind,
    /// and `Value::Signed` is its one canonical representation. Letting the live
    /// variant through here instead is what made a derived monotone field encode
    /// as `Signed` and decode as `Unsigned` — an encoder emitting frames its own
    /// decoder rejected.
    pub fn matches_field(self, field: &crate::schema::Field) -> bool {
        if field.derived.is_some() {
            return matches!(self, Value::Absent | Value::Null | Value::Unknown | Value::Signed(_));
        }
        self.matches_kind(field.kind)
    }

    pub fn matches_kind(self, kind: Kind) -> bool {
        match self {
            Value::Absent | Value::Null | Value::Unknown => true,
            Value::Unsigned(_) => matches!(kind, Kind::Unsigned | Kind::Monotone),
            // NOT `Kind::Monotone`. schema-kind contract gives a monotone counter an unsigned
            // absolute form; the zigzag difference is a delta *encoding*, not a
            // value a record ever holds. Granting `Signed` here let a monotone
            // field carry one past `validate` on an unchanged field, where no
            // encoder arm would ever look at it.
            Value::Signed(_) => matches!(kind, Kind::Signed),
            Value::Enum(_) => matches!(kind, Kind::Enum),
            Value::Bool(_) => matches!(kind, Kind::Boolean),
            Value::ContentId(_) => matches!(kind, Kind::ContentId),
            Value::Continuous(_) => matches!(kind, Kind::Continuous),
        }
    }
}

/// derived-field contract: `scale(v, m)` is binary64 multiplication with round-to-nearest-even, then
/// the result rounded to an integer half-to-even.
///
/// Both steps are named in the specification because they disagree: `0.15` under
/// `fixed(1)` yields `2` here and `1` under exact-real arithmetic. Every
/// implementation does the binary64 multiplication.
pub fn scale(v: f64, m: f64) -> f64 {
    round_half_even(v * m)
}

fn round_half_even(x: f64) -> f64 {
    // Rust's `round_ties_even` is exactly the specified rounding.
    x.round_ties_even()
}

/// `10^k` for `k <= 22`, where every power of ten is exactly representable in
/// binary64. derived-field contract bounds `k` at 22 precisely so this table is unambiguous — past
/// it, repeated multiplication and decimal conversion land on adjacent doubles.
pub fn pow10(k: u8) -> f64 {
    // `Schema::validate` rejects k > 22, but this is public and reachable from a
    // record encoded against an unvalidated schema, where indexing the table
    // panicked. Saturating keeps it total; the schema check is what enforces the
    // bound, and a value that reaches here past it is already outside the format.
    let k = k.min(22);
    const TABLE: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15,
        1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    TABLE[k as usize]
}

/// Whether the escape bit fires for `v` under `quant`, per derived-field contract. The rule is total:
/// every `f64` takes exactly one branch, with no declared range to disagree about.
/// Ordinary rounding loss is **not** an escape condition — quantization exists to
/// round, and escaping on it would defeat the declaration entirely.
pub fn escapes(v: f64, quant: Quant) -> bool {
    if !v.is_finite() || (v == 0.0 && v.is_sign_negative()) {
        return true;
    }
    match quant {
        Quant::Ratio(_) => !(0.0..=1.0).contains(&v) || !is_stable(v, quant),
        Quant::Fixed(k) => {
            let scaled = scale(v, pow10(k));
            // Representable as i64: the round-trip through i64 must be exact.
            if !(scaled >= -(2f64.powi(63)) && scaled < 2f64.powi(63)) {
                return true;
            }
            !is_stable(v, quant)
        }
        Quant::None | Quant::Exact => false,
    }
}

/// The quantization arithmetic on its own, with no escape rule applied.
fn raw_quantized(v: f64, quant: Quant) -> Option<i64> {
    match quant {
        Quant::Fixed(k) => Some(scale(v, pow10(k)) as i64),
        Quant::Ratio(d) => {
            let max = ((1u64 << d) - 1) as f64;
            Some(scale(v, max) as i64)
        }
        Quant::None | Quant::Exact => None,
    }
}

/// Whether the stored integer survives the reader's reconstruction unchanged —
/// that is, whether `quantize(dequantize(q)) == q`.
///
/// **This is an escape condition, and it has to be**, because delta-minimality contract obliges a
/// writer to compare against the *reconstructed* prior. Where the property fails,
/// an unchanged source value differs from its own reconstruction, the writer
/// emits a delta, and the reader rejects it as a mask bit on a field that did not
/// change — an unrecoverable frame produced by legal input.
///
/// It fails once the stored integer leaves the range where `f64` represents
/// integers exactly: at `fixed(11)`, `90071.99254740993` scales to `2^53` by
/// rounding up, dequantizes to a float that scales back to `2^53 - 1`. Rather
/// than bound `k` or the value range against that boundary — which is a claim
/// about float round-trips that would need proving — the condition is checked
/// directly. Every operation in it is pinned binary64 (canonical primitive contract, derived-field contract), so two
/// implementations reach the same answer.
fn is_stable(v: f64, quant: Quant) -> bool {
    let Some(raw) = raw_quantized(v, quant) else {
        return true;
    };
    raw_quantized(dequantize(raw, quant), quant) == Some(raw)
}

/// The quantized integer for a non-escaping value.
pub fn quantize(v: f64, quant: Quant) -> Option<i64> {
    if escapes(v, quant) {
        return None;
    }
    raw_quantized(v, quant)
}

/// canonical primitive contract: a NaN is normalised to this pattern. Its payload and sign are not game
/// state, and leaving them unpinned lets two writers emit different bytes for the
/// same value.
pub const CANONICAL_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// The bits written for any raw `f64`, per canonical primitive contract.
pub fn canonical_f64_bits(x: f64) -> u64 {
    if x.is_nan() {
        CANONICAL_NAN_BITS
    } else {
        x.to_bits()
    }
}

/// Whether two values are the same **as stored** under `field`.
///
/// This, not `==`, is the equality delta-minimality contract is written against. `==` compares what a
/// value *is*; a quantized field stores something coarser, and minimality is a
/// statement about the bytes. Under `fixed(2)`, `0.33` and `0.331` are different
/// values and the same stored value — comparing them with `==` makes the encoder
/// emit an update whose own decoder then rejects it as unchanged, because the
/// decoder only ever sees the quantized side.
///
/// Escaping is part of the stored form: an escaped value keeps its raw `f64`, so
/// an escaping and a non-escaping value are never the same stored value even if
/// they would quantize alike.
pub fn stored_eq(field: &crate::schema::Field, a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Continuous(x), Value::Continuous(y)) if field.quant.is_quantized() => {
            match (escapes(x, field.quant), escapes(y, field.quant)) {
                (false, false) => quantize(x, field.quant) == quantize(y, field.quant),
                (true, true) => canonical_f64_bits(x) == canonical_f64_bits(y),
                _ => false,
            }
        }
        _ => a == b,
    }
}

pub fn dequantize(raw: i64, quant: Quant) -> f64 {
    match quant {
        Quant::Fixed(k) => raw as f64 / pow10(k),
        Quant::Ratio(d) => {
            let max = ((1u64 << d) - 1) as f64;
            raw as f64 / max
        }
        Quant::None | Quant::Exact => raw as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binary64_rule_is_what_the_spec_says_it_is() {
        // derived-field contract names this exact case: 0.15 under fixed(1) is 2 under binary64
        // multiplication and 1 under exact-real arithmetic. If this ever returns
        // 1, the implementation has silently switched arithmetic.
        assert_eq!(scale(0.15, pow10(1)), 2.0);
        // The reason: 0.15 is stored slightly BELOW 0.15, but 0.15*10 rounds to
        // exactly 1.5 in binary64, and 1.5 ties-to-even upward to 2.
        assert_eq!(0.15f64 * 10.0, 1.5);
    }

    #[test]
    fn rounding_is_half_to_even_not_half_away_from_zero() {
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(-0.5), -0.0);
        assert_eq!(round_half_even(-1.5), -2.0);
    }

    #[test]
    fn every_power_of_ten_in_range_is_exact() {
        // The bound at 22 exists because 10^23 is NOT exactly representable.
        for k in 0..=22u8 {
            let from_table = pow10(k);
            let from_string: f64 = format!("1e{k}").parse().unwrap();
            assert_eq!(from_table, from_string, "10^{k} must be unambiguous");
        }
    }

    #[test]
    fn escape_fires_on_non_finite_and_negative_zero() {
        for q in [Quant::Fixed(2), Quant::Ratio(8)] {
            assert!(escapes(f64::NAN, q));
            assert!(escapes(f64::INFINITY, q));
            assert!(escapes(f64::NEG_INFINITY, q));
            assert!(escapes(-0.0, q), "negative zero is a distinct value");
            assert!(!escapes(0.0, q), "positive zero is ordinary");
        }
    }

    #[test]
    fn escape_fires_outside_the_ratio_range_only() {
        let q = Quant::Ratio(8);
        assert!(!escapes(0.0, q));
        assert!(!escapes(1.0, q));
        assert!(!escapes(0.5, q));
        assert!(escapes(-0.001, q));
        assert!(escapes(1.001, q));
    }

    #[test]
    fn ordinary_rounding_loss_is_not_an_escape() {
        // The rule that keeps quantization useful: 0.333 under fixed(2) rounds,
        // and rounding is the point.
        assert!(!escapes(0.333, Quant::Fixed(2)));
        assert_eq!(quantize(0.333, Quant::Fixed(2)), Some(33));
    }

    #[test]
    fn ratio_maps_one_to_all_ones() {
        for d in [1u8, 8, 16, 32] {
            let max = (1u64 << d) - 1;
            assert_eq!(quantize(1.0, Quant::Ratio(d)), Some(max as i64));
            assert_eq!(quantize(0.0, Quant::Ratio(d)), Some(0));
        }
    }

    #[test]
    fn fixed_roundtrips_within_its_declared_precision() {
        let q = Quant::Fixed(2);
        for v in [0.0, 1.0, 0.25, 12.34, -7.5] {
            let raw = quantize(v, q).expect("in range");
            let back = dequantize(raw, q);
            assert!((back - v).abs() <= 0.005, "{v} -> {raw} -> {back}");
        }
    }

    #[test]
    fn the_four_states_are_distinguishable() {
        assert!(!Value::Absent.is_present());
        assert!(Value::Null.is_present());
        assert!(Value::Unknown.is_present());
        assert!(Value::Unsigned(0).is_present());

        assert!(!Value::Null.carries_value());
        assert!(!Value::Unknown.carries_value());
        assert!(Value::Unsigned(0).carries_value(), "zero carries a value");

        // The distinction the fidelity rule exists for.
        assert_ne!(Value::Absent, Value::Null);
        assert_ne!(Value::Null, Value::Unsigned(0));
        assert_ne!(Value::Unknown, Value::Absent);
    }

    #[test]
    fn equality_is_canonical_bytes_not_ieee() {
        // Both directions matter, and IEEE gets both wrong.
        assert_eq!(
            Value::Continuous(f64::NAN),
            Value::Continuous(f64::NAN),
            "canonical primitive contract normalises every NaN to one pattern, so two NaNs are one value"
        );
        assert_eq!(
            Value::Continuous(f64::NAN),
            Value::Continuous(f64::from_bits(0x7FF8_0000_DEAD_BEEF)),
            "payload is not game state"
        );
        assert_ne!(
            Value::Continuous(0.0),
            Value::Continuous(-0.0),
            "canonical primitive contract keeps the sign of zero and they encode differently"
        );
        // The invariant both cases are really about.
        for (a, b) in [(f64::NAN, f64::NAN), (0.0, -0.0), (1.5, 1.5), (1.5, 2.5)] {
            let same_bytes = canonical_f64_bits(a) == canonical_f64_bits(b);
            assert_eq!(
                Value::Continuous(a) == Value::Continuous(b),
                same_bytes,
                "equality must agree with what gets written for {a} vs {b}"
            );
        }
    }
}
