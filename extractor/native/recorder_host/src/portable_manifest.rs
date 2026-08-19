use serde_json::Value;

pub fn redact_root_basename(root: &str) -> String {
    let trimmed = root.trim_end_matches(['/', '\\']);
    let tail = trimmed
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(trimmed);
    tail.to_string()
}

pub fn is_drive_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

pub fn is_unc_absolute_path(value: &str) -> bool {
    value.starts_with(r"\\") || value.starts_with("//")
}

pub fn is_unix_rooted_path(value: &str) -> bool {
    value.starts_with('/') && !value.starts_with("//")
}

pub fn redact_absolute_path_string(value: &str) -> String {
    if is_drive_absolute_path(value) || is_unc_absolute_path(value) || is_unix_rooted_path(value) {
        redact_root_basename(value)
    } else {
        value.to_string()
    }
}

pub fn redact_json_strings(value: &Value) -> Result<Value, String> {
    redact_json_strings_checked(value, "$static_artifact")
}

pub fn redact_json_strings_checked(value: &Value, location: &str) -> Result<Value, String> {
    match value {
        Value::String(text) => Ok(Value::String(redact_absolute_path_string(text))),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                redact_json_strings_checked(value, &format!("{location}[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                let redacted_key = redact_absolute_path_string(key);
                if out.contains_key(&redacted_key) {
                    return Err(format!(
                        "portable recorder manifest key collision after path redaction at {location}"
                    ));
                }
                out.insert(
                    redacted_key,
                    redact_json_strings_checked(value, &format!("{location}.{key}"))?,
                );
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

pub const PORTABILITY_EXCLUSIONS_FIELD: &str = "portability_exclusions";

pub const PORTABILITY_EXCLUSIONS_SCHEMA: &str = "hoi4_portability_exclusions_v1";

pub const PORTABILITY_EXCLUDED_VALUE: &str = "__portability_excluded__";

pub fn is_portability_excluded_value(value: &Value) -> bool {
    value.as_str() == Some(PORTABILITY_EXCLUDED_VALUE)
}

fn survives_redaction(text: &str) -> bool {
    reject_absolute_paths_in_json(&Value::String(redact_absolute_path_string(text)), "$").is_ok()
}

fn is_portability_exclusion_ledger(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 3 {
        return false;
    }
    if object.get("schema").and_then(Value::as_str) != Some(PORTABILITY_EXCLUSIONS_SCHEMA) {
        return false;
    }
    if object.get("status").and_then(Value::as_str) != Some("degraded") {
        return false;
    }
    let Some(rows) = object.get("excluded").and_then(Value::as_array) else {
        return false;
    };
    !rows.is_empty() && rows.iter().all(is_portability_exclusion_row)
}

fn is_portability_exclusion_row(row: &Value) -> bool {
    let Some(object) = row.as_object() else {
        return false;
    };
    if object.len() != 3 {
        return false;
    }
    if !matches!(
        object.get("kind").and_then(Value::as_str),
        Some("value" | "key" | "key_collision" | "reserved_key_conflict")
    ) {
        return false;
    }
    if object.get("reason").and_then(Value::as_str).is_none() {
        return false;
    }
    object
        .get("location")
        .and_then(Value::as_array)
        .is_some_and(|components| {
            components
                .iter()
                .all(|component| component.is_string() || component.is_u64())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortabilityExclusion {
    location: Vec<Value>,
    kind: &'static str,
    reason: &'static str,
}

impl PortabilityExclusion {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "location": Value::Array(self.location.clone()),
            "kind": self.kind,
            "reason": self.reason,
        })
    }

    pub fn warning_line(&self) -> String {
        let location = self
            .location
            .iter()
            .map(|component| match component {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".");
        format!(
            "recorder warning: portable manifest {} at $.{location} was EXCLUDED ({}); \
             the recording still seals and the container's portability_exclusions ledger \
             names it",
            self.kind, self.reason
        )
    }
}

pub fn apply_portability_exclusions(value: &Value) -> (Value, Vec<PortabilityExclusion>) {
    let mut exclusions = Vec::new();
    let inherited = inherited_ledger_rows(value);
    let accounted: Vec<Value> = inherited
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("value"))
        .filter_map(|row| row.get("location").cloned())
        .collect();
    let mut projected = exclude_in_value(value, &mut Vec::new(), &mut exclusions, &accounted);
    if exclusions.is_empty() {
        return (projected, exclusions);
    }
    if let Some(object) = projected.as_object_mut() {
        let mut rows = inherited;
        for exclusion in &exclusions {
            let row = exclusion.to_json();
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
        object.insert(
            PORTABILITY_EXCLUSIONS_FIELD.to_string(),
            serde_json::json!({
                "schema": PORTABILITY_EXCLUSIONS_SCHEMA,
                "status": "degraded",
                "excluded": rows,
            }),
        );
    }
    (projected, exclusions)
}

fn inherited_ledger_rows(value: &Value) -> Vec<Value> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .find(|(key, item)| {
            redact_absolute_path_string(key) == PORTABILITY_EXCLUSIONS_FIELD
                && is_portability_exclusion_ledger(item)
        })
        .and_then(|(_, ledger)| ledger.get("excluded"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn exclude_in_value(
    value: &Value,
    location: &mut Vec<Value>,
    exclusions: &mut Vec<PortabilityExclusion>,
    accounted: &[Value],
) -> Value {
    match value {
        Value::String(text) => {
            if text == PORTABILITY_EXCLUDED_VALUE
                && !accounted
                    .iter()
                    .any(|row| row == &Value::Array(location.clone()))
            {
                exclusions.push(PortabilityExclusion {
                    location: location.clone(),
                    kind: "value",
                    reason: "authored_value_equals_reserved_stand_in",
                });
                Value::String(PORTABILITY_EXCLUDED_VALUE.to_string())
            } else if survives_redaction(text) {
                value.clone()
            } else {
                exclusions.push(PortabilityExclusion {
                    location: location.clone(),
                    kind: "value",
                    reason: "absolute_path_fragment",
                });
                Value::String(PORTABILITY_EXCLUDED_VALUE.to_string())
            }
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    location.push(Value::from(index as u64));
                    let projected = exclude_in_value(item, location, exclusions, accounted);
                    location.pop();
                    projected
                })
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            let mut claimed: Vec<String> = Vec::new();
            for (ordinal, (key, item)) in map.iter().enumerate() {
                if !survives_redaction(key) {
                    location.push(Value::from(ordinal as u64));
                    exclusions.push(PortabilityExclusion {
                        location: location.clone(),
                        kind: "key",
                        reason: "absolute_path_fragment",
                    });
                    location.pop();
                    continue;
                }
                let redacted_key = redact_absolute_path_string(key);
                if claimed.contains(&redacted_key) {
                    location.push(Value::from(ordinal as u64));
                    exclusions.push(PortabilityExclusion {
                        location: location.clone(),
                        kind: "key_collision",
                        reason: "basename_collision_after_redaction",
                    });
                    location.pop();
                    continue;
                }
                let is_reserved_ledger = redacted_key == PORTABILITY_EXCLUSIONS_FIELD;
                claimed.push(redacted_key.clone());
                if location.is_empty() && is_reserved_ledger {
                    if is_portability_exclusion_ledger(item) {
                        out.insert(redacted_key, item.clone());
                    } else {
                        exclusions.push(PortabilityExclusion {
                            location: vec![Value::from(PORTABILITY_EXCLUSIONS_FIELD)],
                            kind: "reserved_key_conflict",
                            reason: "reserved_ledger_field_occupied",
                        });
                    }
                    continue;
                }
                location.push(Value::from(key.as_str()));
                let projected = exclude_in_value(item, location, exclusions, accounted);
                location.pop();
                out.insert(key.clone(), projected);
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub fn portable_manifest_projection(value: &Value) -> Result<Value, String> {
    let (excluded, _) = apply_portability_exclusions(value);
    redact_json_strings_checked(&excluded, "$")
}

pub fn portable_session_manifest_bytes(value: &Value) -> Result<Vec<u8>, String> {
    portable_session_manifest_bytes_accounted(value).map(|(bytes, _)| bytes)
}

pub fn portable_session_manifest_bytes_accounted(
    value: &Value,
) -> Result<(Vec<u8>, Vec<PortabilityExclusion>), String> {
    let (excluded, exclusions) = apply_portability_exclusions(value);
    let redacted = redact_json_strings_checked(&excluded, "$")?;
    reject_absolute_paths_in_json(&redacted, "$")?;
    let bytes = serde_json::to_vec_pretty(&redacted)
        .map_err(|error| format!("serialize portable recorder manifest failed: {error}"))?;
    Ok((bytes, exclusions))
}

pub fn reject_absolute_paths_in_json(value: &Value, location: &str) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if is_drive_absolute_path(text)
                || is_unc_absolute_path(text)
                || is_unix_rooted_path(text)
                || contains_drive_absolute_fragment(text)
                || contains_unc_absolute_fragment(text)
                || contains_unix_absolute_fragment(text)
            {
                return Err(format!(
                    "portable recorder manifest still contains an absolute path at {location}"
                ));
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                reject_absolute_paths_in_json(value, &format!("{location}[{index}]"))?;
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if is_drive_absolute_path(key)
                    || is_unc_absolute_path(key)
                    || is_unix_rooted_path(key)
                    || contains_drive_absolute_fragment(key)
                    || contains_unc_absolute_fragment(key)
                    || contains_unix_absolute_fragment(key)
                {
                    return Err(format!(
                        "portable recorder manifest still contains an absolute path key {key:?} at {location}"
                    ));
                }
                reject_absolute_paths_in_json(value, &format!("{location}.{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn contains_drive_absolute_fragment(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(3).any(|window| {
        window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && (window[2] == b'\\' || window[2] == b'/')
    })
}

pub fn contains_unc_absolute_fragment(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(2).any(|window| window == b"\\\\")
        || bytes.windows(2).enumerate().any(|(index, window)| {
            window == b"//"
                && index > 0
                && matches!(
                    bytes[index - 1],
                    b' ' | b'\t' | b'=' | b'(' | b'[' | b'{' | b'\'' | b'"'
                )
        })
}

pub fn contains_unix_absolute_fragment(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'/'
            && index > 0
            && bytes.get(index + 1).is_some_and(|next| *next != b'/')
            && matches!(
                bytes[index - 1],
                b' ' | b'\t' | b'=' | b':' | b'(' | b'[' | b'{' | b'\'' | b'"'
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_portable_projection_is_idempotent() {
        let value = json!({
            "dll": r"\\?\.cargo-target\release\collector.dll",
            "modset": {
                "game": {"exe_path": r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"},
                "pdx_user_dir": r"\\?\C:\Users\Example\Documents\Paradox Interactive\Hearts of Iron IV",
            },
            "relative": "recorder/native_session_manifest.json",
            "unix": "/var/lib/hoi4/collector_stream.bin",
            "nested": [{"path": "C:/tmp/a.json"}, {"path": "b.json"}],
        });
        let once = portable_manifest_projection(&value).unwrap();
        let twice = portable_manifest_projection(&once).unwrap();
        assert_eq!(once, twice);
        assert_eq!(once["dll"], json!("collector.dll"));
        assert_eq!(once["modset"]["game"]["exe_path"], json!("hoi4.exe"));
        assert_eq!(once["modset"]["pdx_user_dir"], json!("the game"));
        assert_eq!(
            once["relative"],
            json!("recorder/native_session_manifest.json")
        );
        assert_eq!(once["unix"], json!("collector_stream.bin"));
        assert_eq!(once["nested"][0]["path"], json!("a.json"));
        assert_eq!(once["nested"][1]["path"], json!("b.json"));
    }

    #[test]
    fn an_embedded_absolute_fragment_costs_the_field_not_the_seal() {
        let (bytes, exclusions) = portable_session_manifest_bytes_accounted(&json!({
            "modset": {"errors": [
                r"settings.txt launch settings unresolved: \\?\C:\Users\Example\Documents\settings.txt mtime is at/after game process start",
            ]},
            "session_id": "kept",
        }))
        .expect("a poisoned diagnostic must not destroy the recording");
        let sealed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(sealed["session_id"], json!("kept"));
        assert_eq!(
            sealed["modset"]["errors"][0],
            json!(PORTABILITY_EXCLUDED_VALUE)
        );
        assert_eq!(exclusions.len(), 1);
        assert_eq!(
            sealed[PORTABILITY_EXCLUSIONS_FIELD]["excluded"][0]["location"],
            json!(["modset", "errors", 0])
        );
        assert_eq!(
            sealed[PORTABILITY_EXCLUSIONS_FIELD]["excluded"][0]["kind"],
            json!("value")
        );
        assert_eq!(
            sealed[PORTABILITY_EXCLUSIONS_FIELD]["status"],
            json!("degraded")
        );
        reject_absolute_paths_in_json(&sealed, "$").expect("the ledger is itself portable");
        assert!(exclusions[0].warning_line().contains("modset.errors.0"));
    }

    /// shape would have to be regenerated for no gain.
    #[test]
    fn a_clean_document_gains_no_ledger() {
        let clean = json!({"dll": r"C:\x\collector.dll", "count": 3});
        let projected = portable_manifest_projection(&clean).unwrap();
        assert_eq!(projected, json!({"dll": "collector.dll", "count": 3}));
        assert!(projected.get(PORTABILITY_EXCLUSIONS_FIELD).is_none());
    }

    #[test]
    fn the_projection_of_a_degraded_document_is_idempotent() {
        let poisoned = json!({
            "errors": [r"read failed at C:\Users\Example\dlc_load.json"],
            "dll": r"C:\x\collector.dll",
        });
        let once = portable_manifest_projection(&poisoned).unwrap();
        let twice = portable_manifest_projection(&once).unwrap();
        assert_eq!(once, twice);
        assert_eq!(
            once[PORTABILITY_EXCLUSIONS_FIELD]["excluded"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_valid_ledger_under_a_disguised_key_lands_under_the_reserved_name() {
        let ledger = json!({
            "schema": PORTABILITY_EXCLUSIONS_SCHEMA,
            "status": "degraded",
            "excluded": [{
                "location": ["detail"],
                "kind": "value",
                "reason": "absolute_path_fragment",
            }],
        });
        assert!(is_portability_exclusion_ledger(&ledger));
        let disguised = json!({
            r"C:\evidence\portability_exclusions": ledger.clone(),
            "kept": 1,
        });

        let (excluded, _) = apply_portability_exclusions(&disguised);
        assert_eq!(excluded[PORTABILITY_EXCLUSIONS_FIELD], ledger);
        let sealed = portable_manifest_projection(&disguised).unwrap();
        assert_eq!(sealed[PORTABILITY_EXCLUSIONS_FIELD], ledger);
        assert_eq!(
            excluded[PORTABILITY_EXCLUSIONS_FIELD],
            sealed[PORTABILITY_EXCLUSIONS_FIELD]
        );
        assert_eq!(sealed, portable_manifest_projection(&sealed).unwrap());
    }

    #[test]
    fn an_authored_value_equal_to_the_stand_in_is_itself_excluded() {
        let authored = json!({
            "dlc_display_names": {"dlc/x.dlc": PORTABILITY_EXCLUDED_VALUE},
        });
        let (excluded, exclusions) = apply_portability_exclusions(&authored);
        assert_eq!(exclusions.len(), 1, "{exclusions:?}");
        assert_eq!(
            excluded["dlc_display_names"]["dlc/x.dlc"],
            json!(PORTABILITY_EXCLUDED_VALUE)
        );
        let ledger =
            &portable_manifest_projection(&authored).unwrap()[PORTABILITY_EXCLUSIONS_FIELD];
        assert_eq!(ledger["status"], json!("degraded"));
        assert_eq!(
            ledger["excluded"][0]["reason"],
            json!("authored_value_equals_reserved_stand_in")
        );
        let once = portable_manifest_projection(&authored).unwrap();
        assert_eq!(once, portable_manifest_projection(&once).unwrap());
    }

    #[test]
    fn a_disguised_ledger_keeps_its_rows_when_the_pass_finds_something_new() {
        let inherited_row = json!({
            "location": [3],
            "kind": "key",
            "reason": "absolute_path_fragment",
        });
        let disguised = json!({
            r"C:\\evidence\\portability_exclusions": {
                "schema": PORTABILITY_EXCLUSIONS_SCHEMA,
                "status": "degraded",
                "excluded": [inherited_row.clone()],
            },
            "detail": r"read failed at C:\\Users\\Example\\notes.json",
        });

        let projected = portable_manifest_projection(&disguised).unwrap();
        let rows = projected[PORTABILITY_EXCLUSIONS_FIELD]["excluded"]
            .as_array()
            .unwrap();
        assert!(
            rows.contains(&inherited_row),
            "the inherited row must survive: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row["location"] == json!(["detail"])),
            "the new exclusion must be beside it: {rows:?}"
        );
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(projected["detail"], json!(PORTABILITY_EXCLUDED_VALUE));
        let (excluded, _) = apply_portability_exclusions(&disguised);
        assert_eq!(
            excluded[PORTABILITY_EXCLUSIONS_FIELD],
            projected[PORTABILITY_EXCLUSIONS_FIELD]
        );
        assert_eq!(projected, portable_manifest_projection(&projected).unwrap());
    }

    #[test]
    fn a_stand_in_the_ledger_does_not_account_for_is_still_accounted() {
        let degraded = portable_manifest_projection(&json!({
            "errors": [r"read failed at C:\\Users\\Example\\dlc_load.json"],
            "dlc_display_names": {"dlc/a.dlc": "DLC A"},
        }))
        .unwrap();
        assert_eq!(
            degraded[PORTABILITY_EXCLUSIONS_FIELD]["excluded"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let mut authored = degraded.clone();
        authored["dlc_display_names"]["dlc/b.dlc"] = json!(PORTABILITY_EXCLUDED_VALUE);
        let (_, exclusions) = apply_portability_exclusions(&authored);
        assert_eq!(exclusions.len(), 1, "{exclusions:?}");
        let projected = portable_manifest_projection(&authored).unwrap();
        let rows = projected[PORTABILITY_EXCLUSIONS_FIELD]["excluded"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().any(|row| {
            row["location"] == json!(["dlc_display_names", "dlc/b.dlc"])
                && row["reason"] == json!("authored_value_equals_reserved_stand_in")
        }));
        assert_eq!(projected, portable_manifest_projection(&projected).unwrap());
    }

    #[test]
    fn an_absolute_key_reducing_onto_the_ledger_name_cannot_break_idempotence() {
        let disguised = json!({
            r"C:\evidence\portability_exclusions": {"schema": "not a ledger"},
            "kept": 1,
        });
        let once = portable_manifest_projection(&disguised).unwrap();
        let twice = portable_manifest_projection(&once).unwrap();
        assert_eq!(once, twice);
        assert!(is_portability_exclusion_ledger(
            &once[PORTABILITY_EXCLUSIONS_FIELD]
        ));
        assert_eq!(once["kept"], json!(1));
    }

    #[test]
    fn a_spoofed_ledger_is_replaced_and_accounted() {
        let spoofed = json!({
            PORTABILITY_EXCLUSIONS_FIELD: {"schema": "something_else", "excluded": []},
        });
        let (projected, exclusions) = apply_portability_exclusions(&spoofed);
        assert_eq!(exclusions.len(), 1);
        assert_eq!(
            projected[PORTABILITY_EXCLUSIONS_FIELD]["excluded"][0]["kind"],
            json!("reserved_key_conflict")
        );
        let again = apply_portability_exclusions(&projected).0;
        assert_eq!(again, projected);
    }

    #[test]
    fn a_non_portable_key_is_dropped_by_ordinal_without_republishing_it() {
        let poisoned = json!({
            "a": 1,
            r"loaded from C:\Users\Example\x": {"inner": "value"},
        });
        let (projected, exclusions) = apply_portability_exclusions(&poisoned);
        assert_eq!(
            projected,
            json!({"a": 1, PORTABILITY_EXCLUSIONS_FIELD: {
                "schema": PORTABILITY_EXCLUSIONS_SCHEMA,
                "status": "degraded",
                "excluded": [{"location": [1], "kind": "key", "reason": "absolute_path_fragment"}],
            }})
        );
        assert_eq!(exclusions.len(), 1);
        let rendered = serde_json::to_string(&projected).unwrap();
        assert!(!rendered.contains("someone"), "{rendered}");
    }

    #[test]
    fn colliding_absolute_keys_cost_a_member_not_the_seal() {
        let colliding = json!({
            r"C:\one\descriptor.mod": 1,
            r"C:\two\descriptor.mod": 2,
        });
        let (bytes, exclusions) = portable_session_manifest_bytes_accounted(&colliding)
            .expect("a key collision must not destroy the recording");
        let sealed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(sealed["descriptor.mod"], json!(1));
        assert_eq!(exclusions.len(), 1);
        assert_eq!(
            sealed[PORTABILITY_EXCLUSIONS_FIELD]["excluded"][0]["kind"],
            json!("key_collision")
        );
    }
}
