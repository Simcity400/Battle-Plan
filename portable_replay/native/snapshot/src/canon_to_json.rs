use std::sync::Arc;

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

/// [`to_value`] for a value sitting below `containers_above` enclosing
/// containers, so the nesting limit is applied where the value is placed.
pub fn to_value_at(value: &CanonValue, containers_above: usize) -> Result<Value, CanonToValueError> {
    convert(value, containers_above)
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

/// [`to_value`] with the root's members converted on `workers` threads. The
/// root-level checks are [`convert`]'s own: decodable, non-duplicate member
/// names.
pub fn materialize_parallel(value: &CanonValue, workers: usize) -> Result<Value, CanonToValueError> {
    let CanonValue::Object(members) = value else {
        return to_value(value);
    };
    let values: Vec<&CanonValue> = members.iter().map(|(_, member)| member).collect();
    let converted = convert_parallel(&values, 1, workers);
    let mut out = Map::new();
    for ((name, _), converted) in members.iter().zip(converted) {
        let key = decode_json_string_body(name).ok_or_else(|| {
            CanonToValueError::new("world JSON member name does not decode to text")
        })?;
        if out.insert(key, converted?).is_some() {
            return Err(CanonToValueError::new(
                "world JSON object has a duplicate member name",
            ));
        }
    }
    Ok(Value::Object(out))
}

/// The root's members as shared values. A member named in `reuse.changed`, or
/// every member when `reuse` carries no change set, is converted at depth one
/// on `workers` threads; every other member is the previous hour's `Arc`,
/// which the walk proved untouched. Member by member the same values as
/// [`materialize_parallel`], with the same root-level checks.
pub fn materialize_root_members(
    value: &CanonValue,
    workers: usize,
    reuse: crate::replay_cache::RootReuse<'_>,
) -> Result<crate::replay_cache::MaterializedRoot, CanonToValueError> {
    let CanonValue::Object(members) = value else {
        return Err(CanonToValueError::new("a world body is a JSON object"));
    };
    let mut out: crate::replay_cache::MaterializedRoot = Vec::with_capacity(members.len());
    let mut todo: Vec<usize> = Vec::new();
    for (index, (name, _)) in members.iter().enumerate() {
        let key = decode_json_string_body(name).ok_or_else(|| {
            CanonToValueError::new("world JSON member name does not decode to text")
        })?;
        if out.iter().any(|(seen, _)| *seen == key) {
            return Err(CanonToValueError::new(
                "world JSON object has a duplicate member name",
            ));
        }
        let kept = match (reuse.previous, reuse.changed) {
            (Some(previous), Some(changed)) if !changed.contains(name.as_str()) => previous
                .iter()
                .find(|(seen, _)| *seen == key)
                .map(|(_, member)| member.clone()),
            _ => None,
        };
        match kept {
            Some(member) => out.push((key, member)),
            None => {
                todo.push(index);
                out.push((key, Arc::new(Value::Null)));
            }
        }
    }
    let values: Vec<&CanonValue> = todo.iter().map(|&index| &members[index].1).collect();
    let converted = convert_parallel(&values, 1, workers);
    for (&index, value) in todo.iter().zip(converted) {
        out[index].1 = Arc::new(value?);
    }
    Ok(out)
}

/// [`convert`] of several values below `depth` containers on `workers`
/// threads, each wide container split one level down so one large domain does
/// not serialise the conversion on a single worker.
fn convert_parallel(
    values: &[&CanonValue],
    depth: usize,
    workers: usize,
) -> Vec<Result<Value, CanonToValueError>> {
    if values.is_empty() || workers <= 1 {
        return values.iter().map(|value| convert(value, depth)).collect();
    }
    const SPLIT_AT: usize = 32;
    enum Task<'a> {
        Whole(usize, &'a CanonValue),
        Child(usize, usize, &'a CanonValue, usize),
    }
    let mut tasks: Vec<Task<'_>> = Vec::new();
    let mut split: Vec<Option<Result<(), CanonToValueError>>> = Vec::with_capacity(values.len());
    let mut child_slots: Vec<Vec<std::sync::Mutex<Option<Result<Value, CanonToValueError>>>>> =
        Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let children: Option<Vec<&CanonValue>> = match value {
            CanonValue::Object(members) if members.len() >= SPLIT_AT => {
                Some(members.iter().map(|(_, member)| member).collect())
            }
            CanonValue::Array(items) if items.len() >= SPLIT_AT => Some(items.iter().collect()),
            _ => None,
        };
        match children {
            Some(children) => match enter(depth) {
                Ok(child_depth) => {
                    child_slots.push((0..children.len()).map(|_| std::sync::Mutex::new(None)).collect());
                    for (child, value) in children.into_iter().enumerate() {
                        tasks.push(Task::Child(index, child, value, child_depth));
                    }
                    split.push(Some(Ok(())));
                }
                Err(error) => {
                    child_slots.push(Vec::new());
                    split.push(Some(Err(error)));
                }
            },
            None => {
                child_slots.push(Vec::new());
                split.push(None);
                tasks.push(Task::Whole(index, value));
            }
        }
    }
    if tasks.is_empty() {
        return values.iter().map(|value| convert(value, depth)).collect();
    }
    let whole_slots: Vec<std::sync::Mutex<Option<Result<Value, CanonToValueError>>>> =
        (0..values.len()).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers.min(tasks.len()) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(task) = tasks.get(at) else { break };
                match task {
                    Task::Whole(index, value) => {
                        *whole_slots[*index].lock().expect("slot lock") = Some(convert(value, depth));
                    }
                    Task::Child(index, child, value, child_depth) => {
                        *child_slots[*index][*child].lock().expect("slot lock") =
                            Some(convert(value, *child_depth));
                    }
                }
            });
        }
    });
    let take = |slot: &std::sync::Mutex<Option<Result<Value, CanonToValueError>>>| {
        slot.lock().expect("slot lock").take().expect("every task ran")
    };
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            match &split[index] {
                None => return take(&whole_slots[index]),
                Some(Err(error)) => return Err(error.clone()),
                Some(Ok(())) => {}
            }
            match value {
                CanonValue::Object(members) => {
                    let mut out = Map::new();
                    for ((name, _), slot) in members.iter().zip(&child_slots[index]) {
                        let key = decode_json_string_body(name).ok_or_else(|| {
                            CanonToValueError::new("world JSON member name does not decode to text")
                        })?;
                        if out.insert(key, take(slot)?).is_some() {
                            return Err(CanonToValueError::new(
                                "world JSON object has a duplicate member name",
                            ));
                        }
                    }
                    Ok(Value::Object(out))
                }
                CanonValue::Array(_) => Ok(Value::Array(
                    child_slots[index]
                        .iter()
                        .map(take)
                        .collect::<Result<Vec<_>, _>>()?,
                )),
                _ => unreachable!("only containers are split"),
            }
        })
        .collect()
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

    /// The parallel routes produce exactly the `Value` the depth-first
    /// converter produces and refuse exactly what it refuses: wide members
    /// (split one level down), narrow ones, defects in a split child, and
    /// the nesting limit met inside a split member.
    #[test]
    fn the_parallel_routes_agree_with_the_depth_first_converter() {
        let wide_object = (0..40)
            .map(|index| format!(r#""m{index:03}":[{index},{{"k":"v{index}"}}]"#))
            .collect::<Vec<_>>()
            .join(",");
        let wide_array = (0..40).map(|index| index.to_string()).collect::<Vec<_>>().join(",");
        let mut nested = String::new();
        for _ in 0..(SERDE_JSON_RECURSION_LIMIT - 1) {
            nested.push('[');
        }
        for _ in 0..(SERDE_JSON_RECURSION_LIMIT - 1) {
            nested.push(']');
        }
        let documents = [
            r#"{}"#.to_string(),
            r#"{"a":[1,2],"bb":"xyz"}"#.to_string(),
            format!(r#"{{"narrow":1,"wide":{{{wide_object}}}}}"#),
            format!(r#"{{"array":[{wide_array}],"wide":{{{wide_object}}}}}"#),
            format!(r#"{{"array":[{wide_array},"\ud800"],"z":1}}"#),
            format!(r#"{{"wide":{{{wide_object},"zz":"\ud800"}}}}"#),
            format!(r#"{{"deep":{nested}}}"#),
            format!(r#"{{"deep":[{nested}]}}"#),
            format!(r#"{{"deep":[{wide_array},{nested}]}}"#),
            format!(r#"{{"v":"\ud800"}}"#),
        ];
        for document in &documents {
            let canon = CanonValue::parse(document.as_bytes()).expect("document parses");
            let direct = to_value(&canon);
            for workers in [0usize, 1, 3] {
                let parallel = materialize_parallel(&canon, workers);
                let members = materialize_root_members(
                    &canon,
                    workers,
                    crate::replay_cache::RootReuse {
                        previous: None,
                        changed: None,
                    },
                )
                .map(|members| {
                    Value::Object(
                        members
                            .into_iter()
                            .map(|(key, member)| (key, member.as_ref().clone()))
                            .collect(),
                    )
                });
                match (&direct, parallel, members) {
                    (Ok(want), Ok(parallel), Ok(members)) => {
                        assert_eq!(&parallel, want, "{document} materialises differently");
                        assert_eq!(&members, want, "{document} root members differ");
                    }
                    (Err(_), Err(_), Err(_)) => {}
                    (want, parallel, members) => panic!(
                        "{document}: acceptance disagrees: depth-first ok={} parallel ok={} members ok={}",
                        want.is_ok(),
                        parallel.is_ok(),
                        members.is_ok()
                    ),
                }
            }
        }
    }

    /// A member the walk did not touch is handed back by pointer; a touched
    /// one is converted again, and nothing else is.
    #[test]
    fn root_members_reuse_the_untouched_members_by_pointer() {
        let previous_world = CanonValue::parse(br#"{"a":[1,2],"b":{"k":1},"c":"x"}"#).unwrap();
        let previous = materialize_root_members(
            &previous_world,
            2,
            crate::replay_cache::RootReuse {
                previous: None,
                changed: None,
            },
        )
        .unwrap();
        let world = CanonValue::parse(br#"{"a":[1,2],"b":{"k":2},"c":"x"}"#).unwrap();
        let changed: std::collections::BTreeSet<String> = ["b".to_string()].into_iter().collect();
        let members = materialize_root_members(
            &world,
            2,
            crate::replay_cache::RootReuse {
                previous: Some(&previous),
                changed: Some(&changed),
            },
        )
        .unwrap();
        assert!(Arc::ptr_eq(&previous[0].1, &members[0].1), "a is shared");
        assert!(!Arc::ptr_eq(&previous[1].1, &members[1].1), "b is rebuilt");
        assert_eq!(*members[1].1, serde_json::json!({"k": 2}));
        assert!(Arc::ptr_eq(&previous[2].1, &members[2].1), "c is shared");
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
