//! FORMATTERS — turn a measured snapshot scalar into the exact display string the
//! the game topbar shows. Each formatter is total: it takes a `&serde_json::Value`
//! that has already been classified as MEASURED (see [`crate::Fidelity`] /
//! [`crate::value_state`]) and returns a `String`. The no-fabrication boundary is
//! enforced in the apply layer, NOT here — a formatter is only ever called on a
//! measured value, so it never needs to invent a "0" or "—".
//!
//! Correctness notes (verified against the real `topbar.json` placeholders +
//! empirical GER 1941 snapshot, Codex-reviewed 2026-06-26):
//!   * DATE — the topbar `DateText` placeholder is `"7:00 29, September 1939"`,
//!     i.e. `"<hour>:00 <day>, <MonthName> <year>"`. Hour is NOT zero-padded;
//!     minutes are the literal `00`; the DAY precedes the month name.
//!   * PERCENT — a 0..1 float ×100, rounded to a whole number (`0.75 -> "75%"`);
//!     the placeholder `"100%"` carries no decimals.
//!   * UNSIGNED-INT (political/command power) — the game shows the TRUNCATED whole
//!     stock (`247.3735 -> "247"`), never signed/rounded-up.
//!   * MANPOWER — the game's compact people format: `<1000` shown raw, then `K`
//!     (thousands) / `M` (millions) with a FIXED TWO decimals, trailing zeros
//!     KEPT (`525000 -> "525.00K"`, `838600 -> "838.60K"`). (Codex-corrected from
//!     a trimmed `"525K"`; vanilla manpower tooltips show `308.56K`/`9.10M`.)
//!   * FUEL — compact AMOUNT with a FIXED ONE decimal (`849000 -> "849.0K"`).
//!     (Codex: vanilla fuel readouts show one decimal, `428.8K`/`50.0K`. The
//!     vanilla `fuel_value` SLOT actually shows a days/years DURATION — which
//!     needs a consumption rate we do not store — so binding the stored amount
//!     here is a replay SUBSTITUTE, badged `Derived`, not `Exact`.)
//!   * FACTORIES — `civ/mil/dockyard`, integers, slash-separated (`"110/131/16"`).

/// The NOT-CAPTURED glyph. Rendered for any element whose bound field is absent,
/// null, or schema-present-but-unfed (`unavailable`). NEVER a `0`. An em dash is
/// HOI4-idiomatic for "no data" and is present in the game's BMFont atlas.
pub const NOT_CAPTURED: &str = "—";

use serde_json::Value;

/// A measured JSON scalar as `f64`, if it is a number. (Bools/strings are not
/// numeric for our formatters.)
fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// PERCENT: a 0..1 float -> `"NN%"` (×100, rounded to nearest whole percent).
pub fn percent(v: &Value) -> String {
    match as_f64(v) {
        Some(f) => format!("{}%", (f * 100.0).round() as i64),
        None => NOT_CAPTURED.to_string(),
    }
}

/// PERCENT_100: an ALREADY-0..100 value (state resistance/compliance) -> `"NN%"`,
/// rounded to a whole percent. Distinct from [`percent`] (which ×100s a 0..1
/// fraction) — resistance/compliance are stored as 0..100 in the snapshot.
pub fn percent_100(v: f64) -> String {
    format!("{}%", v.round() as i64)
}

/// CATEGORY: a state category id (`"megalopolis"`) -> a display string. The id is
/// already a measured string; we present it verbatim (the in-game loc lookup is a
/// later-phase concern, and the raw id is honest, never fabricated). A non-string
/// value is not-captured.
pub fn category(v: &Value) -> String {
    match v {
        Value::String(s) if !s.is_empty() => s.clone(),
        _ => NOT_CAPTURED.to_string(),
    }
}

/// UNSIGNED-INT: a stored stock value -> its TRUNCATED whole part. the game displays
/// political/command power as the floor of the stock (the fractional part is the
/// sub-tick accrual, not shown). Negative input is clamped to its truncation too
/// (these stocks are non-negative in practice).
pub fn unsigned_int(v: &Value) -> String {
    match as_f64(v) {
        // `trunc()` then cast: 247.3735 -> 247, 182.36 -> 182.
        Some(f) => format!("{}", f.trunc() as i64),
        None => NOT_CAPTURED.to_string(),
    }
}

/// INT: a plain integer value (already whole) -> its string. Used for the
/// factory triple components and any exact integer count.
pub fn int(v: &Value) -> String {
    match v {
        Value::Number(n) if n.is_i64() => n.as_i64().unwrap().to_string(),
        Value::Number(n) if n.is_u64() => n.as_u64().unwrap().to_string(),
        Value::Number(_) => match as_f64(v) {
            Some(f) => format!("{}", f.round() as i64),
            None => NOT_CAPTURED.to_string(),
        },
        _ => NOT_CAPTURED.to_string(),
    }
}

/// Shared compact "big number" core: values <1000 shown as a whole number, then
/// scaled into `K`/`M` with a FIXED number of decimals (`decimals`), trailing
/// zeros KEPT (the game keeps them, e.g. `525.00K`, `9.10M`, `849.0K`).
fn compact_fixed(value: f64, decimals: usize) -> String {
    let neg = value < 0.0;
    let a = value.abs();
    let (scaled, suffix) = if a >= 1_000_000.0 {
        (a / 1_000_000.0, "M")
    } else if a >= 1_000.0 {
        (a / 1_000.0, "K")
    } else {
        // Sub-thousand: show the whole number (people / amount are integral here).
        let s = format!("{}", a.round() as i64);
        return if neg { format!("-{s}") } else { s };
    };
    let body = format!("{scaled:.*}", decimals);
    let out = format!("{body}{suffix}");
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

/// MANPOWER: deployable people -> compact with TWO decimals (`525000 ->
/// "525.00K"`). Trailing zeros are kept to match the game's manpower readout.
pub fn manpower(v: &Value) -> String {
    match as_f64(v) {
        Some(f) => compact_fixed(f, 2),
        None => NOT_CAPTURED.to_string(),
    }
}

/// FUEL: stored fuel AMOUNT -> compact with ONE decimal (`849000.0 -> "849.0K"`).
/// The vanilla `fuel_value` slot is a DURATION (days/years remaining) needing a
/// consumption rate we do not store; binding the stored amount is a replay
/// SUBSTITUTE (badged `Derived`), formatted in the game's one-decimal fuel style.
pub fn fuel(v: &Value) -> String {
    match as_f64(v) {
        Some(f) => compact_fixed(f, 1),
        None => NOT_CAPTURED.to_string(),
    }
}

/// FACTORIES (int-triple): civ/mil/dockyard -> `"110/131/16"`. Each component is
/// read independently; an absent/null component renders the not-captured glyph in
/// its slot (so a partial measure never fabricates a 0 factory).
pub fn factory_triple(
    civ: Option<&Value>,
    mil: Option<&Value>,
    dockyard: Option<&Value>,
) -> String {
    let cell = |o: Option<&Value>| match o {
        Some(v) if v.is_number() => int(v),
        _ => NOT_CAPTURED.to_string(),
    };
    format!("{}/{}/{}", cell(civ), cell(mil), cell(dockyard))
}

/// The 12 English month names, indexed 1..=12 (Clausewitz month is 1-based).
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// CLOCK: a Clausewitz `"YEAR.MONTH.DAY.HOUR"` date -> the topbar's displayed
/// `"<hour>:00 <day>, <MonthName> <year>"` (e.g. `1941.4.13.11 -> "11:00 13,
/// April 1941"`). Format taken verbatim from the `DateText` placeholder
/// `"7:00 29, September 1939"`. Hour is not zero-padded; minutes literal `00`;
/// day precedes the month name.
pub fn clock(date: &str) -> String {
    let parts: Vec<&str> = date.split('.').collect();
    let pi = |i: usize| parts.get(i).and_then(|s| s.parse::<i64>().ok());
    match (pi(0), pi(1), pi(2), pi(3)) {
        (Some(year), Some(month), Some(day), Some(hour)) if (1..=12).contains(&month) => {
            let mon = MONTHS[(month - 1) as usize];
            format!("{hour}:00 {day}, {mon} {year}")
        }
        // A malformed date string is NOT a measured value -> not captured.
        _ => NOT_CAPTURED.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn percent_rounds_to_whole() {
        assert_eq!(percent(&json!(0.75)), "75%");
        assert_eq!(percent(&json!(0.85)), "85%");
        assert_eq!(percent(&json!(1.0)), "100%");
        assert_eq!(percent(&json!(0.0)), "0%"); // measured zero is NOT not-captured
        assert_eq!(percent(&json!(0.834)), "83%");
    }

    #[test]
    fn unsigned_int_truncates_stock() {
        assert_eq!(unsigned_int(&json!(247.3735)), "247");
        assert_eq!(unsigned_int(&json!(182.36)), "182");
        assert_eq!(unsigned_int(&json!(999.9)), "999"); // truncated, not rounded up
        assert_eq!(unsigned_int(&json!(0)), "0");
    }

    #[test]
    fn manpower_two_decimals_kept() {
        assert_eq!(manpower(&json!(525000)), "525.00K");
        assert_eq!(manpower(&json!(838600)), "838.60K");
        assert_eq!(manpower(&json!(525250)), "525.25K");
        assert_eq!(manpower(&json!(1_500_000)), "1.50M");
        assert_eq!(manpower(&json!(9_100_000)), "9.10M");
        assert_eq!(manpower(&json!(999)), "999"); // sub-thousand verbatim
        assert_eq!(manpower(&json!(0)), "0"); // measured zero
    }

    #[test]
    fn fuel_one_decimal_kept() {
        assert_eq!(fuel(&json!(849000.0)), "849.0K");
        assert_eq!(fuel(&json!(428800.0)), "428.8K");
        assert_eq!(fuel(&json!(50000.0)), "50.0K");
        assert_eq!(fuel(&json!(0.0)), "0");
    }

    #[test]
    fn factory_triple_and_partial() {
        assert_eq!(
            factory_triple(Some(&json!(110)), Some(&json!(131)), Some(&json!(16))),
            "110/131/16"
        );
        // a missing dockyard renders not-captured in its slot, NOT 0.
        assert_eq!(
            factory_triple(Some(&json!(110)), Some(&json!(131)), None),
            "110/131/—"
        );
        // a measured-zero factory IS shown as 0.
        assert_eq!(
            factory_triple(Some(&json!(0)), Some(&json!(0)), Some(&json!(0))),
            "0/0/0"
        );
    }

    #[test]
    fn clock_matches_placeholder_shape() {
        assert_eq!(clock("1941.4.13.11"), "11:00 13, April 1941");
        assert_eq!(clock("1939.9.29.7"), "7:00 29, September 1939"); // == placeholder
        assert_eq!(clock("1936.1.1.12"), "12:00 1, January 1936");
        // malformed -> not captured (never a fabricated date)
        assert_eq!(clock("garbage"), NOT_CAPTURED);
        assert_eq!(clock("1941.13.1.0"), NOT_CAPTURED); // month out of range
    }

    #[test]
    fn not_captured_glyph_is_em_dash_not_zero() {
        assert_eq!(NOT_CAPTURED, "—");
        assert_ne!(NOT_CAPTURED, "0");
        // a non-number (null arrives here only via the apply layer's guard) -> "—"
        assert_eq!(percent(&Value::Null), NOT_CAPTURED);
        assert_eq!(unsigned_int(&Value::Null), NOT_CAPTURED);
    }
}
