//! The portable projection every recorder manifest passes through before it is
//! sealed into a `.battleplan` container.
//!
//! The run directory's `native_session_manifest.json` keeps absolute paths, for
//! recovery and audit on the machine that recorded. The independently shareable
//! container must never carry them (container spec §0, "No personal paths"), so
//! the recorder redacts every absolute path string — and every absolute path
//! object KEY — down to its final component before packing, then refuses to
//! seal if anything absolute survived.
//!
//! This lives in the library, not in the recorder binary, because it is now
//! read from two sides: the recorder applies it when sealing, and
//! `compact_replay_verification_gates` applies it when binding a run
//! directory's mutable manifest back to the sealed authority inside the
//! container. A second, independently written copy of these rules in the gate
//! would drift silently the first time either side learned a new path shape,
//! and the gate's refusal message would blame the recording.

use serde_json::Value;

/// Reduce a possibly-personal filesystem path to a portable basename. The
/// container spec forbids personal paths in emitted metadata (§0 "No personal
/// paths"); the domain-59 catalog embeds absolute install/mod roots, so we keep
/// only the final component (mirrors `redact_path` in the Python gates).
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
    // Static-artifact provenance is itself portable index data. Reuse the
    // checked recursive pass so absolute paths in nested object keys are
    // redacted too, and fail closed instead of silently overwriting colliding
    // basenames.
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

/// The reserved top-level ledger naming everything the projection had to
/// remove to make the document portable. ABSENT on a clean document — its
/// presence is exactly the statement "this manifest is portability-degraded".
pub const PORTABILITY_EXCLUSIONS_FIELD: &str = "portability_exclusions";

/// The ledger's own vocabulary, so a reader that does not understand this
/// version refuses instead of guessing at the rows.
pub const PORTABILITY_EXCLUSIONS_SCHEMA: &str = "hoi4_portability_exclusions_v1";

/// The reserved stand-in for a string value the projection could not reduce.
/// Deliberately unmistakable: it is not a plausible filename, hash, count or
/// language token, so a consumer that reads one has read a hole, not a value.
pub const PORTABILITY_EXCLUDED_VALUE: &str = "__portability_excluded__";

/// Whether a value is the reserved stand-in. A verifier must never certify a
/// claim whose sealed value is this.
pub fn is_portability_excluded_value(value: &Value) -> bool {
    value.as_str() == Some(PORTABILITY_EXCLUDED_VALUE)
}

/// Can this string cross the boundary once the whole-string redaction has had
/// its turn? The decision is taken with the SEALER's own two functions, so the
/// exclusion pass can never disagree with the rejector that follows it.
fn survives_redaction(text: &str) -> bool {
    reject_absolute_paths_in_json(&Value::String(redact_absolute_path_string(text)), "$").is_ok()
}

/// Is this a well-formed ledger written by this module?
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

/// One thing the projection removed, and where it was.
///
/// The location is a COMPONENT ARRAY, never a JSON Pointer: a pointer is spelled
/// `/modset/errors/0`, and `contains_unix_absolute_fragment` classifies a
/// leading `/` as an absolute unix path — the accounting record would poison the
/// document it is accounting for.
///
/// An excluded object KEY is located by its ORDINAL within its parent, never by
/// its text. The text is the thing being removed for carrying a personal path;
/// repeating it in the ledger would defeat the removal.
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

    /// The operator-facing line. One per exclusion, named, never silent —
    /// the same idiom refused static artifacts already use.
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

/// Remove what cannot be made portable, and say so — instead of refusing to
/// seal a recording over it.
///
/// WHY THIS EXISTS. The projection below reduces a string that IS an absolute
/// path to its basename, but `reject_absolute_paths_in_json` is stricter than
/// that: it also refuses a path EMBEDDED in a longer string. Every diagnostic
/// the recorder composes with a path in it therefore survived the projection
/// and then killed the seal. Capture t55 (2026-08-15) lost a 91-game-hour
/// recording to exactly one such string — `$.modset.errors[0]`, a modset
/// resolution failure that had spelled out the absolute path of `settings.txt`.
/// That is backwards: a diagnostic string is the least load-bearing thing in
/// the document, and the owner's standing rule is that missing or unprovable
/// data is excluded and ACCOUNTED, never that it destroys the recording.
///
/// WHAT IT CAN REACH. Only what the whole-string redaction could not reduce.
/// Every field a verifier binds as authority — digests, counts, names, and
/// bare paths like `$.dll` or `$.static_bundle.<label>.path` — is a whole-string
/// value that the redaction reduces on its own and this pass never sees. What
/// is left is composite text: prose with a path in it. So this is not a licence
/// to blank out authority; structurally it cannot reach it. The verifier
/// nevertheless refuses to certify a claim whose sealed value is the reserved
/// stand-in, because "structurally cannot" is an argument and the sentinel
/// check is a proof.
///
/// IDEMPOTENT. A second pass finds nothing to exclude (the stand-in carries no
/// path) and finds its own ledger already present, so it changes nothing. That
/// is what lets a verifier apply this to the run-directory copy and compare the
/// result with the sealed copy.
pub fn apply_portability_exclusions(value: &Value) -> (Value, Vec<PortabilityExclusion>) {
    let mut exclusions = Vec::new();
    // WHICH STAND-INS DID A PREVIOUS PASS WRITE? The question is per-LOCATION,
    // not per-document. A stand-in and an authored string that equals one are
    // the same text; what separates them is whether this document's own ledger
    // already accounts for the value AT THIS PATH. Asking it of the document --
    // "does any ledger exist" -- let one inherited row wave through every
    // stand-in anywhere in it, including one added afterwards, and the
    // verifier took the same shortcut and agreed with the sealer while replay
    // refused the value neither of them had accounted for.
    let inherited = inherited_ledger_rows(value);
    // `Value` is not `Ord`, so this is a plain list; a ledger holds a handful
    // of rows, and a manifest that produced enough of them for the linear scan
    // to matter has bigger problems than this lookup.
    let accounted: Vec<Value> = inherited
        .iter()
        .filter(|row| row.get("kind").and_then(Value::as_str) == Some("value"))
        .filter_map(|row| row.get("location").cloned())
        .collect();
    let mut projected = exclude_in_value(value, &mut Vec::new(), &mut exclusions, &accounted);
    if exclusions.is_empty() {
        // A clean document is returned untouched, ledger and all: adding an
        // empty ledger to every manifest would change the shape of every
        // recording ever sealed to say nothing. A document that arrived with a
        // ledger keeps exactly the one it arrived with.
        return (projected, exclusions);
    }
    if let Some(object) = projected.as_object_mut() {
        // INHERITED ROWS FIRST, then what this pass found. A pass over an
        // already-degraded document must not drop the accounting it was handed:
        // the ledger's promise is that it names every value missing from the
        // document, not merely the ones the latest pass happened to remove.
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

/// The rows of a valid ledger the document arrived carrying, or none.
///
/// The ledger is found the way the TRAVERSAL finds it -- by the name its key
/// redacts onto -- because the traversal also preserves a valid ledger hidden
/// under an absolute key like `C:\\x\\portability_exclusions`. Looking only under
/// the literal name meant those rows were not inherited, and the moment the
/// pass found any new exclusion its ledger insertion overwrote the preserved
/// one: the document lost the accounting for values that are still missing from
/// it, and since the verifier recomputes the same incomplete ledger, the two
/// agreed and the container bound on an incomplete account.
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
            // THE STAND-IN IS RESERVED AS A VALUE, exactly as the ledger's name
            // is reserved as a key. A reader that meets it must be able to
            // conclude "this was removed" without consulting anything else, so
            // an authored string that happens to equal it -- a modded DLC may
            // legally be named anything -- is excluded and accounted too.
            // Leaving it untouched made a clean container indistinguishable
            // from a degraded one, and every consumer refused it as a hole
            // while no ledger row explained why.
            if text == PORTABILITY_EXCLUDED_VALUE
                && !accounted.iter().any(|row| row == &Value::Array(location.clone()))
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
                // KEYS FIRST, and the whole member goes when a key cannot
                // travel. Recursing into a member whose key is being removed
                // would produce ledger locations naming a key that is not in
                // the document.
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
                    // Two absolute keys reducing to one basename. The
                    // redaction below cannot represent both and used to refuse
                    // the seal; the later member is dropped and accounted
                    // instead. Ordering is serde_json's map order, which is
                    // sorted and therefore the same on both sides of the
                    // boundary.
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
                // The name the key will HAVE after redaction, not the one it
                // has now: an absolute key like `C:\x\portability_exclusions`
                // reduces onto the reserved name, and comparing the original
                // spelling let it through here and then collide with the ledger
                // on the next pass -- breaking the idempotence the verifier
                // depends on.
                if location.is_empty() && is_reserved_ledger {
                    // The ledger's namespace is reserved. A document that
                    // already carries a VALID ledger is one this pass has
                    // already run over, and it is preserved untouched so the
                    // projection stays idempotent. Anything else under that
                    // name is dropped, so the ledger a reader finds is always
                    // the one this pass wrote.
                    if is_portability_exclusion_ledger(item) {
                        // Under the REDACTED name, which for the ordinary
                        // ledger is the key it already had. It matters for the
                        // disguised spelling: an absolute key reducing onto the
                        // reserved name kept its original spelling here, and
                        // this pass alone then produced no ledger under the
                        // reserved name at all. The verifier runs only this
                        // pass and looks the ledger up by that name, so it
                        // computed no expected ledger and refused an honestly
                        // sealed container.
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

/// The whole-document portable projection, without serializing it.
///
/// This is the transform the sealed copy of a recorder manifest has already
/// been through. It is IDEMPOTENT — a redacted basename has no absolute prefix
/// left to strip, and the exclusion pass ahead of it leaves an already-projected
/// document alone — which is what lets a verifier apply it to both copies and
/// compare, instead of having to know which copy came first.
///
/// It is also many-to-one: two different absolute paths sharing a basename
/// project to the same value. That is not a weakness of the projection, it is
/// the point. The absolute prefix is deliberately not carried into the
/// container, so it is not part of what a container can attest, and no verifier
/// may claim to have bound it.
pub fn portable_manifest_projection(value: &Value) -> Result<Value, String> {
    let (excluded, _) = apply_portability_exclusions(value);
    redact_json_strings_checked(&excluded, "$")
}

/// Build the recorder-manifest payload that is allowed to cross the portable
/// file boundary. The local run manifest retains absolute paths for recovery
/// and audit, but the independently shareable `.battleplan` never does.
pub fn portable_session_manifest_bytes(value: &Value) -> Result<Vec<u8>, String> {
    portable_session_manifest_bytes_accounted(value).map(|(bytes, _)| bytes)
}

/// The same seal-time payload, plus what had to be removed to produce it.
///
/// The caller prints one warning line per exclusion. The ledger travels inside
/// the container so a reader who never saw this machine's stderr still learns
/// the document is degraded; the stderr line is for the operator watching the
/// capture.
pub fn portable_session_manifest_bytes_accounted(
    value: &Value,
) -> Result<(Vec<u8>, Vec<PortabilityExclusion>), String> {
    let (excluded, exclusions) = apply_portability_exclusions(value);
    let redacted = redact_json_strings_checked(&excluded, "$")?;
    // The rejector still runs, and still refuses. It is now a claim about this
    // module rather than about the recording: the exclusion pass decided what
    // to remove with these exact functions, so anything it left behind is a
    // defect here, not a capture that deserves to die.
    reject_absolute_paths_in_json(&redacted, "$")?;
    let bytes = serde_json::to_vec_pretty(&redacted)
        .map_err(|error| format!("serialize portable recorder manifest failed: {error}"))?;
    Ok((bytes, exclusions))
}

/// The single authority on "this JSON still carries an absolute path", used
/// from BOTH ends of the container boundary:
///
///  * the recorder runs it at seal time, after the projection, and refuses to
///    write a container whose recorder manifest still holds one;
///  * `compact_replay_verification_gates` runs it on the SEALED copy it reads
///    back out of a container, because `verify_all()` proves the container is
///    internally consistent, not that the recorder's seal-time gate ever ran.
///
/// One implementation, deliberately. Two independently written path detectors
/// would agree until the first time either end learned a new path shape, and
/// the gate would then admit exactly the containers the sealer refuses to
/// write (or refuse the ones it does).
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
                    // The key is named, not just its parent. A refusal that
                    // located an offending KEY only by the object containing it
                    // is unactionable in a 17 MB manifest — and the value arm
                    // below already names its own full location.
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

    /// The projection a verifier applies to the run-dir copy must be a no-op on
    /// a manifest that has ALREADY been through it — otherwise applying it to
    /// both copies would not be a valid comparison, and every sealed container
    /// would have to be re-derived to know which form it holds.
    ///
    /// Production change that would make this fail: a redaction rule that
    /// rewrites a value which is already a bare basename (for example stripping
    /// a leading segment unconditionally instead of only for rooted paths).
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
        assert_eq!(
            once["modset"]["pdx_user_dir"],
            json!("the game")
        );
        assert_eq!(once["relative"], json!("recorder/native_session_manifest.json"));
        assert_eq!(once["unix"], json!("collector_stream.bin"));
        assert_eq!(once["nested"][0]["path"], json!("a.json"));
        assert_eq!(once["nested"][1]["path"], json!("b.json"));
    }

    /// The projection is the only thing standing between a personal absolute
    /// path and a shareable container, so a shape it fails to reduce must never
    /// be packed — but it must cost the FIELD, not the recording.
    ///
    /// This is capture t55's defect, as a unit: one modset diagnostic that had
    /// spelled out an absolute `settings.txt` path refused to seal a recording
    /// that had already captured 91 game hours.
    ///
    /// Production change that would make this fail: dropping the exclusion pass
    /// from the projection (the seal would refuse again), or dropping the
    /// `reject_absolute_paths_in_json` pass from
    /// `portable_session_manifest_bytes` (the path would ship).
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
        // The accounting must not reintroduce what it is accounting for: a
        // JSON Pointer location (`/modset/errors/0`) would itself be read as a
        // unix-absolute path by the rejector.
        reject_absolute_paths_in_json(&sealed, "$").expect("the ledger is itself portable");
        assert!(exclusions[0].warning_line().contains("modset.errors.0"));
    }

    /// A clean document must be sealed exactly as it always was. If the
    /// exclusion pass volunteered an empty ledger, every recording ever made
    /// would change shape to say nothing, and every fixture pinned to the old
    /// shape would have to be regenerated for no gain.
    ///
    /// Production change that would make this fail: writing the ledger
    /// unconditionally instead of only when something was excluded.
    #[test]
    fn a_clean_document_gains_no_ledger() {
        let clean = json!({"dll": r"C:\x\collector.dll", "count": 3});
        let projected = portable_manifest_projection(&clean).unwrap();
        assert_eq!(projected, json!({"dll": "collector.dll", "count": 3}));
        assert!(projected.get(PORTABILITY_EXCLUSIONS_FIELD).is_none());
    }

    /// The verifier applies this projection to the run-directory copy and
    /// compares the result with the sealed copy, so a degraded document must
    /// project to ITSELF. A pass that re-excluded its own ledger, or appended a
    /// second one, would make every degraded container unverifiable.
    ///
    /// Production change that would make this fail: dropping the valid-ledger
    /// arm of the reserved-key handling in `exclude_in_value`, or including the
    /// ledger in the walk that computes exclusions.
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

    /// A document already carrying a VALID ledger under a disguised absolute
    /// key must come out of the EXCLUSION PASS ALONE with that ledger under the
    /// reserved name.
    ///
    /// The verifier runs only the exclusion pass and then looks the ledger up
    /// by the reserved name. Preserving the ledger under its original spelling
    /// meant the sealer -- which also runs the redaction -- produced a
    /// canonical ledger while the verifier found none, and an honestly sealed
    /// container refused.
    ///
    /// Production change that would make this fail: re-inserting a preserved
    /// ledger under `key` rather than under its redacted name.
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

        // What the VERIFIER computes: the exclusion pass on its own.
        let (excluded, _) = apply_portability_exclusions(&disguised);
        assert_eq!(excluded[PORTABILITY_EXCLUSIONS_FIELD], ledger);
        // What the SEALER writes: the whole projection. The two must agree, or
        // the verifier refuses a container the sealer just produced.
        let sealed = portable_manifest_projection(&disguised).unwrap();
        assert_eq!(sealed[PORTABILITY_EXCLUSIONS_FIELD], ledger);
        assert_eq!(excluded[PORTABILITY_EXCLUSIONS_FIELD], sealed[PORTABILITY_EXCLUSIONS_FIELD]);
        assert_eq!(sealed, portable_manifest_projection(&sealed).unwrap());
    }

    /// An authored string that happens to EQUAL the reserved stand-in is
    /// excluded and accounted, not passed through.
    ///
    /// Consumers refuse the stand-in wherever they would read it as authority
    /// -- a DLC display name equal to it is a hole, not a name. A modded DLC
    /// may legally carry any nonblank name, so leaving the literal untouched
    /// made a clean container refuse with no ledger row to explain it.
    ///
    /// Production change that would make this fail: passing a string equal to
    /// `PORTABILITY_EXCLUDED_VALUE` through the exclusion pass unaccounted.
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
        let ledger = &portable_manifest_projection(&authored).unwrap()
            [PORTABILITY_EXCLUSIONS_FIELD];
        assert_eq!(ledger["status"], json!("degraded"));
        assert_eq!(
            ledger["excluded"][0]["reason"],
            json!("authored_value_equals_reserved_stand_in")
        );
        // And the pass stays idempotent over its own output: the stand-in it
        // just wrote is accounted for already and must not be re-accounted.
        let once = portable_manifest_projection(&authored).unwrap();
        assert_eq!(once, portable_manifest_projection(&once).unwrap());
    }

    /// A ledger hidden under an absolute key keeps its rows when the SAME pass
    /// finds something new to exclude.
    ///
    /// The traversal preserves a valid ledger whose key redacts onto the
    /// reserved name, but the inherited-rows lookup used the literal name only.
    /// With no new exclusion nothing showed: the preserved ledger survived
    /// untouched. Add one, and the final insertion replaced the preserved
    /// ledger with just the new row -- the document then carried an account of
    /// one missing value and silence about the others, and the verifier, which
    /// recomputes the same thing, agreed with it.
    ///
    /// Production change that would make this fail: looking the inherited
    /// ledger up by `PORTABILITY_EXCLUSIONS_FIELD` instead of by the name each
    /// key redacts onto.
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
            // ...and something this pass has to remove for the first time.
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
            rows.iter()
                .any(|row| row["location"] == json!(["detail"])),
            "the new exclusion must be beside it: {rows:?}"
        );
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(projected["detail"], json!(PORTABILITY_EXCLUDED_VALUE));
        // The verifier runs the exclusion pass alone and must reach the same
        // ledger, or it refuses a container the sealer just wrote.
        let (excluded, _) = apply_portability_exclusions(&disguised);
        assert_eq!(
            excluded[PORTABILITY_EXCLUSIONS_FIELD],
            projected[PORTABILITY_EXCLUSIONS_FIELD]
        );
        assert_eq!(projected, portable_manifest_projection(&projected).unwrap());
    }

    /// A stand-in the ledger does NOT account for is accounted, even when the
    /// document already carries a ledger for something else.
    ///
    /// "Has this been through the pass" is a question about the LOCATION, not
    /// about the document. Asking it of the document let one inherited row wave
    /// through every stand-in anywhere in it: project a document with one path
    /// exclusion, add an authored DLC name equal to the stand-in, project
    /// again, and the new value travelled with no row and no warning -- while
    /// replay refuses that name on sight, so the container could not open and
    /// nothing said why.
    ///
    /// Production change that would make this fail: deciding
    /// `already projected` from the presence of a ledger rather than from
    /// whether the ledger names this path.
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

        // Now a DLC whose authored name is the stand-in itself.
        let mut authored = degraded.clone();
        authored["dlc_display_names"]["dlc/b.dlc"] = json!(PORTABILITY_EXCLUDED_VALUE);
        let (_, exclusions) = apply_portability_exclusions(&authored);
        assert_eq!(exclusions.len(), 1, "{exclusions:?}");
        let projected = portable_manifest_projection(&authored).unwrap();
        let rows = projected[PORTABILITY_EXCLUSIONS_FIELD]["excluded"]
            .as_array()
            .unwrap();
        // The inherited row is kept -- the ledger names every value missing
        // from the document, not only the ones the latest pass removed -- and
        // the new one is beside it.
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().any(|row| {
            row["location"] == json!(["dlc_display_names", "dlc/b.dlc"])
                && row["reason"] == json!("authored_value_equals_reserved_stand_in")
        }));
        // And the accounting is stable: the stand-in it just accounted for is
        // not re-accounted on the next pass.
        assert_eq!(projected, portable_manifest_projection(&projected).unwrap());
    }

    /// An absolute key whose FINAL COMPONENT is the reserved ledger name must
    /// be recognised before the redaction renames it onto that name — otherwise
    /// the first pass lets it through, the redaction turns it into the ledger,
    /// and a second pass sees a foreign ledger and replaces it. The verifier
    /// applies this projection to the run-directory copy and compares the
    /// result with the sealed copy, so a projection that is not idempotent is a
    /// verifier that refuses honest containers.
    ///
    /// Production change that would make this fail: deciding the reserved-name
    /// match on the key as written instead of on the name it will have.
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

    /// The ledger's name is reserved. A document carrying something else under
    /// it — a stale ledger, or a field a future recorder happened to name the
    /// same — must not be able to pass itself off as the accounting, and must
    /// not kill the seal either.
    ///
    /// Production change that would make this fail: keeping a pre-existing
    /// `portability_exclusions` value without validating it as a ledger.
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

    /// An object KEY that cannot travel takes its whole member with it, and the
    /// ledger locates it by ORDINAL. Recording the key's text would republish
    /// the personal path that got the member removed in the first place.
    ///
    /// Production change that would make this fail: recording the key text in
    /// the location, or recursing into a member whose key is being dropped.
    #[test]
    fn a_non_portable_key_is_dropped_by_ordinal_without_republishing_it() {
        let poisoned = json!({
            "a": 1,
            r"loaded from C:\Users\Example\x": {"inner": "value"},
        });
        let (projected, exclusions) = apply_portability_exclusions(&poisoned);
        assert_eq!(projected, json!({"a": 1, PORTABILITY_EXCLUSIONS_FIELD: {
            "schema": PORTABILITY_EXCLUSIONS_SCHEMA,
            "status": "degraded",
            "excluded": [{"location": [1], "kind": "key", "reason": "absolute_path_fragment"}],
        }}));
        assert_eq!(exclusions.len(), 1);
        let rendered = serde_json::to_string(&projected).unwrap();
        assert!(!rendered.contains("someone"), "{rendered}");
    }

    /// Two absolute keys reducing to one basename used to refuse the seal
    /// outright (`redact_json_strings_checked` returns `Err`). It costs the
    /// later member now, accounted.
    ///
    /// Production change that would make this fail: removing the collision arm
    /// from `exclude_in_value`, which puts the unsatisfiable `Err` back on the
    /// seal path.
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
