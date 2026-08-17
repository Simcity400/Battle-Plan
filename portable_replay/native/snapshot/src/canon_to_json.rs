//! Build the `serde_json::Value` a consumer sees DIRECTLY from a
//! [`CanonValue`], without spelling the world to text and parsing it back.
//!
//! WHY THIS EXISTS. The certified replay cache reconstructs a game hour as a
//! [`CanonValue`] — a JSON tree that keeps every scalar's source spelling — and
//! the viewer wants a `serde_json::Value`. Until this module, the only route
//! between them was through TEXT: `CanonValue::to_json_bytes` spelled the whole
//! world out (31.6 MB per hour on capture t53), SHA-256 hashed those bytes for
//! the per-hour parity check, `parse_json_strict` parsed them back into a
//! `Value`, and `serde_json::from_value` then re-buffered the entire tree
//! through serde's `Content` intermediate because `Snapshot::rest` is
//! `#[serde(flatten)]`. Four whole-world passes and three whole-world
//! allocations to hand the viewer a tree the reader was already holding.
//!
//! WHAT IT MUST GUARANTEE. Exactly one thing, and it is not "a reasonable
//! `Value`": for every `CanonValue` the cache can reconstruct,
//!
//! ```text
//!     to_value(&canon)  ==  parse_json_strict(&canon.to_json_bytes())
//! ```
//!
//! including the places where `serde_json` is LOSSY. Its default float parser
//! is deliberately not correctly-rounded, so the token `189352.52932739258`
//! becomes a `f64` one ULP low; a census of capture t26 found 510 such tokens
//! in 2,990,242 (see [`crate::canon_value`]). A conversion that used Rust's own
//! correctly-rounded `str::parse::<f64>` would be *more* accurate and therefore
//! WRONG here — it would hand the viewer a different number than the shipped
//! path does, silently, on 510 tokens per 16 GB. So every token that is not
//! plainly an integer is handed to `serde_json`'s own parser, and the integers
//! that are taken by a fast path are proved equivalent in
//! `every_number_token_matches_the_text_round_trip`.
//!
//! The same applies to the two behaviours that are not about numbers at all:
//! `serde_json` refuses a document nested past its own recursion limit, and
//! refuses a string body containing a lone surrogate. Both are mirrored here
//! ([`SERDE_JSON_RECURSION_LIMIT`], and `decode_json_string_body` returning
//! `None`) so the direct route refuses exactly the documents the text route
//! refuses, rather than serving an hour its predecessor would have declined.

use serde_json::{Map, Number, Value};

use crate::canon_value::{decode_json_string_body, CanonValue};

/// The DEEPEST nesting `serde_json`'s `Deserializer` accepts, and therefore the
/// deepest this conversion accepts.
///
/// One less than `serde_json`'s own `RECURSION_LIMIT` of 128, because its guard
/// spends a level on the document itself; the number that matters is the one a
/// document is measured against, so that is the one named here.
///
/// It is `serde_json`'s number, not ours, and
/// `the_recursion_limit_is_the_one_serde_json_enforces` re-derives it from
/// `parse_json_strict` itself rather than trusting this constant — so if a
/// future `serde_json` moves its limit, the test fails and the two routes are
/// brought back together instead of quietly disagreeing about which hours a
/// recording can serve.
pub const SERDE_JSON_RECURSION_LIMIT: usize = 127;

/// A `CanonValue` that has no `serde_json::Value` counterpart.
///
/// Every variant corresponds to a document `parse_json_strict` also refuses;
/// see the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonToValueError {
    message: String,
}

impl CanonToValueError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CanonToValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CanonToValueError {}

/// The `serde_json::Value` `parse_json_strict(&value.to_json_bytes())` would
/// produce, built directly from the tree.
pub fn to_value(value: &CanonValue) -> Result<Value, CanonToValueError> {
    convert(value, 0)
}

fn convert(value: &CanonValue, depth: usize) -> Result<Value, CanonToValueError> {
    match value {
        CanonValue::Null => Ok(Value::Null),
        CanonValue::Bool(flag) => Ok(Value::Bool(*flag)),
        CanonValue::Number(token) => number_from_token(token).map(Value::Number),
        CanonValue::String(body) => decode_json_string_body(body)
            .map(Value::String)
            .ok_or_else(|| {
                CanonToValueError::new(
                    "world JSON string body does not decode to text (lone surrogate escape)",
                )
            }),
        CanonValue::Array(items) => {
            let depth = enter(depth)?;
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(convert(item, depth)?);
            }
            Ok(Value::Array(out))
        }
        CanonValue::Object(members) => {
            let depth = enter(depth)?;
            let mut out = Map::new();
            for (name, member) in members {
                let key = decode_json_string_body(name).ok_or_else(|| {
                    CanonToValueError::new("world JSON member name does not decode to text")
                })?;
                let converted = convert(member, depth)?;
                // A `CanonValue` object holds STRICTLY INCREASING decoded names
                // by construction, so this can only fire if a patch produced a
                // tree the value model forbids. `parse_json_strict` refuses a
                // duplicate member too; refusing here keeps the two routes
                // agreeing about that as well, instead of silently keeping the
                // last one the way a plain `insert` would.
                if out.insert(key, converted).is_some() {
                    return Err(CanonToValueError::new(
                        "world JSON object has a duplicate member name",
                    ));
                }
            }
            Ok(Value::Object(out))
        }
    }
}

fn enter(depth: usize) -> Result<usize, CanonToValueError> {
    let depth = depth + 1;
    if depth > SERDE_JSON_RECURSION_LIMIT {
        return Err(CanonToValueError::new(
            "world JSON exceeds the JSON reader's nesting limit",
        ));
    }
    Ok(depth)
}

/// The `serde_json::Number` `serde_json`'s own parser makes of this token.
///
/// The fast path is taken only for tokens with no fraction and no exponent,
/// where `serde_json` accumulates digits into a `u64` (or negates into an
/// `i64`) and the answer is exact — so the fast path and the parser cannot
/// disagree. Everything else, including an integer too large for those types,
/// goes to `serde_json` itself, because its float conversion is the thing that
/// must be reproduced and not improved on.
fn number_from_token(token: &str) -> Result<Number, CanonToValueError> {
    if !token.contains(['.', 'e', 'E']) {
        if token.starts_with('-') {
            // STRICTLY negative, and that is not pedantry: `serde_json` negates
            // its accumulated magnitude and takes the float branch whenever the
            // result is not negative — which is exactly `-0`, whose `Value` is
            // `Number(-0.0)` and not `Number(0)`. A fast path that accepted
            // every `i64` would hand the viewer an integer zero where the
            // shipped path hands it a negative float zero.
            if let Ok(value) = token.parse::<i64>() {
                if value < 0 {
                    return Ok(Number::from(value));
                }
            }
        } else if let Ok(value) = token.parse::<u64>() {
            return Ok(Number::from(value));
        }
    }
    serde_json::from_str::<Number>(token).map_err(|error| {
        CanonToValueError::new(format!("world JSON number token {token} is not a number: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole contract, on every token shape the value model documents as
    /// surviving only as a spelling, plus the shapes that stress the fast path
    /// boundary. Each one is converted directly and parsed through text, and
    /// the two `Value`s must be equal — which for `f64` means the SAME bits,
    /// since `serde_json::Number`'s `PartialEq` compares the stored number.
    ///
    /// kills: replacing `serde_json::from_str::<Number>` with
    /// `token.parse::<f64>()` in `number_from_token`. Rust's parser is
    /// correctly rounded and `serde_json`'s default one is not, so the first
    /// six tokens here — all real capture t26 spellings — come back one ULP
    /// apart and this fails. Widening the integer fast path to accept a
    /// fraction or an exponent fails for the same reason.
    #[test]
    fn every_number_token_matches_the_text_round_trip() {
        let mut tokens: Vec<String> = crate::canon_value::NON_IDENTITY_TOKENS
            .iter()
            .map(|token| (*token).to_string())
            .collect();
        for extra in [
            "0",
            "-0",
            "1",
            "-1",
            "9223372036854775807",
            "-9223372036854775808",
            "9223372036854775808",
            "18446744073709551615",
            "18446744073709551616",
            "-9223372036854775809",
            "1.0",
            "-1.0",
            "0.1",
            "1e-3",
            "-0.0",
            "1.7976931348623157e308",
            "5e-324",
        ] {
            tokens.push(extra.to_string());
        }
        for token in tokens {
            let document = format!(r#"{{"v":{token}}}"#);
            let canon = CanonValue::parse(document.as_bytes()).expect("token parses");
            let direct = to_value(&canon).expect("token converts");
            let through_text =
                crate::parse_json_strict(&canon.to_json_bytes()).expect("token round trips");
            assert_eq!(direct, through_text, "token {token} converted differently");
        }
    }

    /// Strings, member names and containers: the direct route and the text
    /// route must agree on the decoded text of every escape the format allows,
    /// on member ORDER (`serde_json` writes and reads members in decoded-name
    /// order, which is not the order of their escaped spellings), and on empty
    /// containers.
    ///
    /// kills: dropping the `decode_json_string_body` call and using the stored
    /// body verbatim — `"\n"` would reach the viewer as a two-character
    /// backslash-n instead of a newline, and every escaped member name would
    /// become a different key.
    #[test]
    fn strings_names_and_containers_match_the_text_round_trip() {
        for document in [
            r#"{}"#,
            r#"[]"#,
            r#"null"#,
            r#"{"a":[1,2],"bb":"xyz"}"#,
            r#"{"a\"b":1,"aXb":2}"#,
            r#"{"v":"quote \" backslash \\ slash \/ \b\f\n\r\t"}"#,
            r#"{"v":"café 中文 😀"}"#,
            r#"{"a":{"b":{"c":[true,false,null,-0.0,1e3]}},"z":""}"#,
            r#"[{"k":"A\\"},{"k":"plain"}]"#,
        ] {
            let canon = CanonValue::parse(document.as_bytes()).expect("document parses");
            let direct = to_value(&canon).expect("document converts");
            let through_text =
                crate::parse_json_strict(&canon.to_json_bytes()).expect("document round trips");
            assert_eq!(direct, through_text, "document {document} converted differently");
        }
    }

    /// A body `serde_json` cannot decode must be refused here too, not carried
    /// through as bytes. The cache is happy to STORE a lone surrogate escape
    /// verbatim — that is what `CanonValue` is for — but the text route hands
    /// it to `serde_json`, which refuses, so the direct route must refuse.
    ///
    /// kills: replacing the `ok_or_else` on `decode_json_string_body` with a
    /// fallback that keeps the raw body.
    #[test]
    fn a_body_serde_json_refuses_is_refused_here_too() {
        let canon = CanonValue::parse(br#"{"v":"\ud800"}"#).expect("the cache stores it");
        assert!(to_value(&canon).is_err(), "direct route must refuse");
        assert!(
            crate::parse_json_strict(&canon.to_json_bytes()).is_err(),
            "text route must refuse"
        );
    }

    /// [`SERDE_JSON_RECURSION_LIMIT`] is not a guess: this finds the depth at
    /// which `parse_json_strict` starts refusing and requires the direct route
    /// to refuse at exactly the same depth.
    ///
    /// kills: raising or removing the depth check in `enter`. A recording whose
    /// document nests past `serde_json`'s limit is refused by the shipped path
    /// today; a direct route without the check would start SERVING those hours,
    /// which is a behaviour change nobody measured, and would hand the viewer a
    /// `Value` whose recursive `Drop` is the thing the limit exists to bound.
    #[test]
    fn the_recursion_limit_is_the_one_serde_json_enforces() {
        let nested = |depth: usize| {
            let mut document = String::new();
            for _ in 0..depth {
                document.push('[');
            }
            for _ in 0..depth {
                document.push(']');
            }
            document
        };
        let accepts = |depth: usize| {
            let document = nested(depth);
            let canon = CanonValue::parse(document.as_bytes()).expect("the value model parses it");
            (
                crate::parse_json_strict(document.as_bytes()).is_ok(),
                to_value(&canon).is_ok(),
            )
        };
        // Find the deepest document `serde_json` still accepts, rather than
        // asserting a number at it.
        let mut deepest_accepted = 0;
        for depth in 1..=(SERDE_JSON_RECURSION_LIMIT + 8) {
            if accepts(depth).0 {
                deepest_accepted = depth;
            } else {
                break;
            }
        }
        assert_eq!(
            deepest_accepted, SERDE_JSON_RECURSION_LIMIT,
            "serde_json's nesting limit moved; SERDE_JSON_RECURSION_LIMIT must follow it"
        );
        assert_eq!(
            accepts(deepest_accepted),
            (true, true),
            "the two routes disagree at the deepest accepted document"
        );
        assert_eq!(
            accepts(deepest_accepted + 1),
            (false, false),
            "the two routes disagree one past the deepest accepted document"
        );
    }
}
