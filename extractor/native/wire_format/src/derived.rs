//! derived-field contract derived fields: the clock-linear transform between a field's live value
//! and the value the format stores.
//!
//! ```text
//!   stored = v + c × (frame_tick >> u)
//! ```
//!
//! A field that counts down with the clock has a *constant* stored value, so a
//! whole run of ticks collapses to one `UNCHANGED` frame. That is the entire
//! point of the declaration, and it only works if the transform is applied.
//!
//! **A `Record` holds stored values, not live ones.** The conversion happens
//! here, at the two boundaries — capture converts live to stored before building
//! a record, replay converts back after decoding one. Everything between those
//! boundaries is then tick-free: `diff`, `check_minimal`, the change mask and the
//! encoders all compare stored forms, which is exactly what delta-minimality contract's "differs as
//! stored" asks of them, and none of them has to carry a frame tick to do it.
//!
//! Putting the transform inside the codec instead would mean every comparison
//! needed both records' ticks — a `KeyedState` would have to carry a tick per
//! row — and the tick-invariance that makes `UNCHANGED` work would become an
//! emergent property of the comparison rather than a property of the stored
//! value. It is stated here as a contract rather than left to the caller because
//! an undocumented pre-transformation cannot enforce derived-field contract's overflow rule, and
//! cannot supply the inverse a reader needs.

use crate::schema::Field;
use crate::value::Value;
use crate::varint::{Result, WireError};

/// `c × (frame_tick >> u)`, the clock term. derived-field contract computes it in `i64` and makes
/// overflow a hard error rather than a wrapped value, because a wrapped clock
/// term silently reconstructs the wrong live value forever after.
fn clock_term(field: &Field, frame_tick: u64) -> Result<i64> {
    let Some(d) = field.derived else {
        return Ok(0);
    };
    // The conversion is part of the arithmetic, so it is checked too: `as i64`
    // turns `u64::MAX` into `-1`, and the `checked_mul` below would then never
    // see the overflow derived-field contract calls a hard error.
    let ticks = i64::try_from(frame_tick >> d.u)
        .map_err(|_| WireError::Corrupt("derived clock term overflows i64"))?;
    d.c.checked_mul(ticks)
        .ok_or(WireError::Corrupt("derived clock term overflows i64"))
}

/// The one live representation of each derived-capable kind — the exact set
/// `from_stored` produces, so `to_stored` accepts nothing it cannot invert.
fn matches_live_kind(kind: crate::schema::Kind, live: Value) -> bool {
    use crate::schema::Kind;
    match kind {
        Kind::Signed => matches!(live, Value::Signed(_)),
        Kind::Unsigned | Kind::Monotone => matches!(live, Value::Unsigned(_)),
        _ => false,
    }
}

/// Live value at `frame_tick` → the value a record stores, always `Signed`.
///
/// The stored domain is all of `i64` whatever the field's live kind, because derived-field contract's
/// transform "can carry an otherwise unsigned quantity negative" — a count-up
/// field with `c = -1` stores `-5` for a live `5` at tick 10, which is legal and
/// has no overflow anywhere. Forcing the result back into the live variant made
/// that whole class unrepresentable.
pub fn to_stored(field: &Field, live: Value, frame_tick: u64) -> Result<Value> {
    if field.derived.is_none() || !live.carries_value() {
        return Ok(live);
    }
    // The live variant must be exactly the one `from_stored` produces for this
    // kind, so the boundary is a bijection.
    //
    // `matches_kind` is too generous here: it blesses `Value::Signed` for
    // `Kind::Monotone` — correct for a *stored* monotone value, which schema-kind contract may
    // difference negative — but a monotone field's *live* value is unsigned
    // everywhere else in the crate. Gating on it let capture accept
    // `Signed(-5)` on a derived monotone field, encode it, and then have replay
    // refuse the crate's own bytes.
    if !matches_live_kind(field.kind, live) {
        return Err(WireError::Corrupt("derived live value does not match the field's kind"));
    }
    let term = clock_term(field, frame_tick)?;
    let base = match live {
        Value::Unsigned(v) => {
            i64::try_from(v).map_err(|_| WireError::Corrupt("derived live value exceeds i64"))?
        }
        Value::Signed(v) => v,
        _ => return Err(WireError::Corrupt("derived field carries a non-integer value")),
    };
    let stored = base
        .checked_add(term)
        .ok_or(WireError::Corrupt("derived stored value overflows i64"))?;
    Ok(Value::Signed(stored))
}

/// Stored value → the live value at `frame_tick`, in the field's declared kind.
///
/// This is where a negative result is genuinely an error for an unsigned field:
/// the *live* value is unsigned by declaration, even though the stored one need
/// not be.
pub fn from_stored(field: &Field, stored: Value, frame_tick: u64) -> Result<Value> {
    if field.derived.is_none() || !stored.carries_value() {
        return Ok(stored);
    }
    let Value::Signed(s) = stored else {
        return Err(WireError::Corrupt("derived stored value must be Signed"));
    };
    let term = clock_term(field, frame_tick)?;
    let live = s
        .checked_sub(term)
        .ok_or(WireError::Corrupt("derived live value overflows i64"))?;
    Ok(match field.kind {
        crate::schema::Kind::Signed => Value::Signed(live),
        _ => Value::Unsigned(
            u64::try_from(live)
                .map_err(|_| WireError::Corrupt("derived live value is negative on an unsigned field"))?,
        ),
    })
}

/// Convert a whole row of live values into the values a record stores, leaving
/// every non-derived field untouched.
pub fn row_to_stored(
    fields: &[&Field],
    live: &[Value],
    frame_tick: u64,
) -> Result<Vec<Value>> {
    if fields.len() != live.len() {
        return Err(WireError::Corrupt("row arity does not match schema"));
    }
    fields
        .iter()
        .zip(live)
        .map(|(f, v)| to_stored(f, *v, frame_tick))
        .collect()
}

/// The inverse: a decoded record's stored values back to live values at `frame_tick`.
pub fn row_from_stored(
    fields: &[&Field],
    stored: &[Value],
    frame_tick: u64,
) -> Result<Vec<Value>> {
    if fields.len() != stored.len() {
        return Err(WireError::Corrupt("row arity does not match schema"));
    }
    fields
        .iter()
        .zip(stored)
        .map(|(f, v)| from_stored(f, *v, frame_tick))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Derived, Kind};

    fn countdown(c: i64, u: u8) -> Field {
        let mut f = Field::new(1, Kind::Unsigned);
        f.derived = Some(Derived { c, u });
        f
    }

    #[test]
    fn a_countdown_has_a_constant_stored_value() {
        // The whole reason the declaration exists: a value falling by one per
        // tick stores the same number at every tick, so the run is UNCHANGED.
        let f = countdown(1, 0);
        let stored: Vec<Value> = (0..10u64)
            .map(|t| to_stored(&f, Value::Unsigned(100 - t), t).unwrap())
            .collect();
        assert!(
            stored.iter().all(|v| *v == Value::Signed(100)),
            "expected a constant stored value, got {stored:?}"
        );
    }

    #[test]
    fn the_shift_lets_a_value_change_once_per_hour_on_a_finer_grid() {
        // u is the log2 of ticks per unit of change: with 8 ticks to the hour, a
        // once-an-hour countdown declares u = 3.
        let f = countdown(1, 3);
        for tick in 0..8u64 {
            assert_eq!(to_stored(&f, Value::Unsigned(50), tick).unwrap(), Value::Signed(50));
        }
        assert_eq!(to_stored(&f, Value::Unsigned(49), 8).unwrap(), Value::Signed(50));
    }

    #[test]
    fn the_transform_inverts_exactly() {
        let f = countdown(-3, 2);
        for tick in [0u64, 1, 7, 8, 4096, 1 << 40] {
            for v in [0u64, 1, 1_000_000] {
                let live = Value::Unsigned(v);
                let Ok(stored) = to_stored(&f, live, tick) else { continue };
                assert_eq!(from_stored(&f, stored, tick).unwrap(), live, "tick {tick}, v {v}");
            }
        }
    }

    #[test]
    fn overflow_is_a_hard_error_not_a_wrapped_value() {
        // derived-field contract: "overflow is a hard writer error, not a wrapped value."
        let f = countdown(i64::MAX, 0);
        assert_eq!(
            to_stored(&f, Value::Unsigned(1), 2),
            Err(WireError::Corrupt("derived clock term overflows i64"))
        );
        let f = countdown(1, 0);
        assert_eq!(
            to_stored(&f, Value::Unsigned(u64::MAX), 0),
            Err(WireError::Corrupt("derived live value exceeds i64"))
        );
        // A NEGATIVE stored value is legal — derived-field contract zigzags precisely because the
        // transform can carry an otherwise unsigned quantity negative.
        assert_eq!(to_stored(&countdown(-1, 0), Value::Unsigned(5), 10), Ok(Value::Signed(-5)));
        // What is not legal is a negative LIVE value on an unsigned field.
        assert_eq!(
            from_stored(&f, Value::Signed(0), 5),
            Err(WireError::Corrupt("derived live value is negative on an unsigned field"))
        );
    }

    #[test]
    fn the_four_states_pass_through_untouched() {
        let f = countdown(1, 0);
        for state in [Value::Absent, Value::Null, Value::Unknown] {
            assert_eq!(to_stored(&f, state, 99).unwrap(), state);
            assert_eq!(from_stored(&f, state, 99).unwrap(), state);
        }
    }

    #[test]
    fn a_non_derived_field_is_untouched() {
        let f = Field::new(1, Kind::Unsigned);
        assert_eq!(to_stored(&f, Value::Unsigned(7), 12345).unwrap(), Value::Unsigned(7));
    }
}
