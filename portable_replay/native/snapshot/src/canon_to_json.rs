use serde_json::{Map, Number, Value};

use crate::canon_value::{decode_json_string_body, CanonValue};

pub const SERDE_JSON_RECURSION_LIMIT: usize = 127;

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

pub fn to_value(value: &CanonValue) -> Result<Value, CanonToValueError> {
    convert(value, 0)
}

fn convert(value: &CanonValue, depth: usize) -> Result<Value, CanonToValueError> {
    match value {
        CanonValue::Null => Ok(Value::Null),
        CanonValue::Bool(flag) => Ok(Value::Bool(*flag)),
        CanonValue::Number(token) => number_from_token(token).map(Value::Number),
        CanonValue::String(body) => {
            decode_json_string_body(body)
                .map(Value::String)
                .ok_or_else(|| {
                    CanonToValueError::new(
                        "world JSON string body does not decode to text (lone surrogate escape)",
                    )
                })
        }
        CanonValue::Array(items) => {
            let depth = enter(depth)?;
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(convert(item, depth)?);
            }
            Ok(Value::Array(out))
        }
        CanonValue::Object(members) => {
            let depth = enter(depth)?;
            let mut out = Map::new();
            for (name, member) in members.iter() {
                let key = decode_json_string_body(name).ok_or_else(|| {
                    CanonToValueError::new("world JSON member name does not decode to text")
                })?;
                let converted = convert(member, depth)?;
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

fn number_from_token(token: &str) -> Result<Number, CanonToValueError> {
    if !token.contains(['.', 'e', 'E']) {
        if token.starts_with('-') {
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
        CanonToValueError::new(format!(
            "world JSON number token {token} is not a number: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(
                direct, through_text,
                "document {document} converted differently"
            );
        }
    }

    #[test]
    fn a_body_serde_json_refuses_is_refused_here_too() {
        let canon = CanonValue::parse(br#"{"v":"\ud800"}"#).expect("the cache stores it");
        assert!(to_value(&canon).is_err(), "direct route must refuse");
        assert!(
            crate::parse_json_strict(&canon.to_json_bytes()).is_err(),
            "text route must refuse"
        );
    }

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
