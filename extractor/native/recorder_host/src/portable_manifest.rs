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

/// The whole-document portable projection, without serializing it.
///
/// This is the transform the sealed copy of a recorder manifest has already
/// been through. It is IDEMPOTENT — a redacted basename has no absolute prefix
/// left to strip — which is what lets a verifier apply it to both copies and
/// compare, instead of having to know which copy came first.
///
/// It is also many-to-one: two different absolute paths sharing a basename
/// project to the same value. That is not a weakness of the projection, it is
/// the point. The absolute prefix is deliberately not carried into the
/// container, so it is not part of what a container can attest, and no verifier
/// may claim to have bound it.
pub fn portable_manifest_projection(value: &Value) -> Result<Value, String> {
    redact_json_strings_checked(value, "$")
}

/// Build the recorder-manifest payload that is allowed to cross the portable
/// file boundary. The local run manifest retains absolute paths for recovery
/// and audit, but the independently shareable `.battleplan` never does.
pub fn portable_session_manifest_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let redacted = portable_manifest_projection(value)?;
    reject_absolute_paths_in_json(&redacted, "$")?;
    serde_json::to_vec_pretty(&redacted)
        .map_err(|error| format!("serialize portable recorder manifest failed: {error}"))
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
                        "portable recorder manifest still contains an absolute path key at {location}"
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
    /// path and a shareable container, so a shape it fails to reduce must still
    /// be refused at seal time rather than silently packed.
    ///
    /// Production change that would make this fail: dropping the
    /// `reject_absolute_paths_in_json` pass from
    /// `portable_session_manifest_bytes`.
    #[test]
    fn an_embedded_absolute_fragment_still_refuses_to_seal() {
        let error = portable_session_manifest_bytes(&json!({
            "detail": r"loaded from C:\Users\Example\mod.zip"
        }))
        .expect_err("an embedded absolute fragment must not seal");
        assert!(error.contains("still contains an absolute path"), "{error}");
    }
}
