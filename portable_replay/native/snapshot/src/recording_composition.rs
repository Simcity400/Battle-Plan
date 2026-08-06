//! What a recording is made of, stated by the recorder and read back strictly.
//!
//! A recording is composed of three things: the delta cache (`hour_index.h4ix`
//! plus `checkpoints.h4cp`) which is what replay actually consumes, the raw
//! collector wire stream (`collector_raw/collector_stream.bin`, role
//! [`crate::COMPACT_STREAM_ROLE`]), and fixed overhead (the recorder manifest,
//! static catalogs, index/timeline).
//!
//! The raw stream is roughly 40% of a campaign artifact and replay does not
//! read its CONTENT — the packaged-decoder route uses only its length and
//! sha256 as a source binding, and reconstructs hours from the cache. So the
//! recorder can seal a recording without it. That is a composition decision,
//! not a defect, and this module is the single vocabulary both sides use to
//! state it and to read it back.
//!
//! Two laws hold here:
//!
//! * **Self-describing.** Every sealed recording carries an explicit
//!   composition field. A reader never infers the composition from the mere
//!   absence of an entry.
//! * **Fail-closed at the decoder.** An unknown spelling, a wrong JSON type, or
//!   a composition that disagrees with the entries actually present is a hard
//!   error. There is no default and no "unassigned" value: a recording that
//!   does not say what it is, is rejected.

use serde_json::{Map, Value};

/// Field name carrying the composition, in the recorder session manifest, the
/// container `index.json`, and `timeline.json`.
pub const RAW_STREAM_PACKAGING_FIELD: &str = "raw_stream_packaging";

/// Wire spelling for a recording that carries the raw collector stream.
pub const RAW_STREAM_PACKAGING_PACKAGED: &str = "packaged";

/// Wire spelling for a recording that deliberately omits the raw collector
/// stream and ships the delta cache alone.
pub const RAW_STREAM_PACKAGING_OMITTED: &str = "omitted";

/// The reason a cache-only recording states for the stream's absence. This is
/// the NAMED cause a degraded consumer reports; it never surfaces as a bare
/// "not found".
pub const RAW_STREAM_OMITTED_REASON: &str =
    "recorded without --package-raw-stream: this recording ships the delta cache only, \
     so the raw collector wire stream was never packaged into the container";

/// Whether a sealed recording carries the raw collector stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStreamPackaging {
    /// The container carries the raw stream at the canonical path.
    Packaged,
    /// The container ships the delta cache alone, by recorder decision.
    Omitted,
}

impl RawStreamPackaging {
    /// The wire spelling written into the manifest, index and timeline.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Packaged => RAW_STREAM_PACKAGING_PACKAGED,
            Self::Omitted => RAW_STREAM_PACKAGING_OMITTED,
        }
    }

    /// Whether the raw stream entry must be present for this composition.
    pub fn requires_stream_entry(self) -> bool {
        matches!(self, Self::Packaged)
    }

    /// Parse one wire spelling. Every value other than the two assigned
    /// spellings is rejected — an unrecognized composition is never treated as
    /// a default, because a reader that guessed here would silently accept a
    /// recording it cannot describe.
    pub fn parse(value: &str) -> Result<Self, CompositionError> {
        match value {
            RAW_STREAM_PACKAGING_PACKAGED => Ok(Self::Packaged),
            RAW_STREAM_PACKAGING_OMITTED => Ok(Self::Omitted),
            other => Err(CompositionError::UnassignedSpelling(other.to_string())),
        }
    }
}

/// Why a composition could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionError {
    /// The document carries no composition field at all.
    Missing,
    /// The field is present but is not a JSON string.
    NotAString,
    /// The field is a string, but not one of the assigned spellings.
    UnassignedSpelling(String),
}

impl std::fmt::Display for CompositionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "{RAW_STREAM_PACKAGING_FIELD:?} is missing: the recording does not state \
                 whether it carries the raw collector stream, and a composition is never assumed"
            ),
            Self::NotAString => write!(
                f,
                "{RAW_STREAM_PACKAGING_FIELD:?} is not a JSON string"
            ),
            Self::UnassignedSpelling(value) => write!(
                f,
                "{RAW_STREAM_PACKAGING_FIELD:?} has unassigned value {value:?}; \
                 expected {RAW_STREAM_PACKAGING_PACKAGED:?} or {RAW_STREAM_PACKAGING_OMITTED:?}"
            ),
        }
    }
}

impl std::error::Error for CompositionError {}

/// Read the composition out of a JSON object (a recorder manifest, an
/// `index.json`, or a `timeline.json`), rejecting every unassigned value.
pub fn read_raw_stream_packaging(object: &Map<String, Value>) -> Result<RawStreamPackaging, CompositionError> {
    match object.get(RAW_STREAM_PACKAGING_FIELD) {
        None => Err(CompositionError::Missing),
        Some(Value::String(value)) => RawStreamPackaging::parse(value),
        Some(_) => Err(CompositionError::NotAString),
    }
}

/// Read the composition where the field may legitimately be absent because the
/// recording predates it.
///
/// Absence means "written before recordings stated their composition", and
/// those recordings all carry the stream — that was the only composition that
/// existed. A PRESENT but unassigned or mistyped value is still a hard error:
/// this relaxes the missing case only, never the strictness of a value that
/// is actually there.
pub fn read_raw_stream_packaging_with_legacy_default(
    object: &Map<String, Value>,
) -> Result<RawStreamPackaging, CompositionError> {
    match read_raw_stream_packaging(object) {
        Err(CompositionError::Missing) => Ok(RawStreamPackaging::Packaged),
        other => other,
    }
}

/// Read a sealed container's declared composition from its recorder manifest.
///
/// This is the one implementation every consumer of the raw stream uses to
/// explain an absent stream entry, so a cache-only recording produces the same
/// named cause wherever it is met instead of a per-tool "not found".
pub fn container_raw_stream_packaging(
    container: &crate::container::Container,
) -> Result<RawStreamPackaging, String> {
    let entries = container.entries_by_role(RECORDER_MANIFEST_ROLE);
    let entry = match entries.as_slice() {
        [entry] => entry,
        // Without a recorder authority there is nothing to declare a
        // composition, which is the pre-composition shape: those all carried
        // the stream.
        [] => return Ok(RawStreamPackaging::Packaged),
        entries => {
            return Err(format!(
                "container has {} entries with role {RECORDER_MANIFEST_ROLE:?}; expected exactly one",
                entries.len()
            ))
        }
    };
    let bytes = container
        .read_entry_bytes(&entry.path)
        .map_err(|error| format!("read recorder manifest {:?}: {error}", entry.path))?;
    let value: Value = crate::strict_json::from_slice(&bytes)
        .map_err(|error| format!("strict parse recorder manifest {:?}: {error}", entry.path))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("recorder manifest {:?} is not a JSON object", entry.path))?;
    read_raw_stream_packaging_with_legacy_default(object)
        .map_err(|error| format!("recorder manifest {:?}: {error}", entry.path))
}

/// Container role of the recorder session manifest.
const RECORDER_MANIFEST_ROLE: &str = "recorder_manifest";

/// The message a consumer that genuinely needs the raw wire records reports
/// when it meets a cache-only recording.
///
/// `what_it_needs_the_stream_for` completes the sentence "… cannot X": it names
/// the work that is impossible, so the degraded outcome states both the cause
/// and the consequence.
pub fn raw_stream_unavailable_message(what_it_needs_the_stream_for: &str) -> String {
    format!(
        "cannot {what_it_needs_the_stream_for}: {RAW_STREAM_OMITTED_REASON}. \
         Re-record with --package-raw-stream to produce a container this can read."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("test object").clone()
    }

    #[test]
    fn assigned_spellings_round_trip() {
        for packaging in [RawStreamPackaging::Packaged, RawStreamPackaging::Omitted] {
            assert_eq!(
                RawStreamPackaging::parse(packaging.as_str()),
                Ok(packaging),
                "{packaging:?} must survive its own wire spelling"
            );
        }
    }

    #[test]
    fn only_packaged_requires_the_stream_entry() {
        assert!(RawStreamPackaging::Packaged.requires_stream_entry());
        assert!(!RawStreamPackaging::Omitted.requires_stream_entry());
    }

    /// An unassigned spelling must reject at the decoder rather than fall back
    /// to a default. Production change that would make this fail: giving
    /// `RawStreamPackaging::parse` a catch-all arm.
    #[test]
    fn unassigned_spelling_rejects() {
        for value in ["", "PACKAGED", "none", "true", "omitted "] {
            let error = RawStreamPackaging::parse(value)
                .expect_err("unassigned composition spelling must reject");
            assert_eq!(error, CompositionError::UnassignedSpelling(value.to_string()));
            assert!(
                error.to_string().contains("unassigned"),
                "the rejection must name the cause, got {error}"
            );
        }
    }

    /// Production change that would make this fail: reading the field with
    /// `as_str()` and treating a non-string as absent.
    #[test]
    fn wrong_json_type_rejects() {
        for value in [json!({"raw_stream_packaging": true}), json!({"raw_stream_packaging": 1})] {
            assert_eq!(
                read_raw_stream_packaging(&object(value)),
                Err(CompositionError::NotAString)
            );
        }
    }

    /// Production change that would make this fail: defaulting a missing field
    /// in the strict reader.
    #[test]
    fn missing_field_rejects_in_the_strict_reader() {
        let error = read_raw_stream_packaging(&object(json!({}))).expect_err("missing must reject");
        assert_eq!(error, CompositionError::Missing);
        assert!(error.to_string().contains("never assumed"));
    }

    /// The legacy relaxation covers ABSENCE only. Production change that would
    /// make this fail: widening the legacy default to swallow a bad value.
    #[test]
    fn legacy_default_relaxes_absence_but_not_a_bad_value() {
        assert_eq!(
            read_raw_stream_packaging_with_legacy_default(&object(json!({}))),
            Ok(RawStreamPackaging::Packaged)
        );
        assert_eq!(
            read_raw_stream_packaging_with_legacy_default(&object(
                json!({"raw_stream_packaging": "sometimes"})
            )),
            Err(CompositionError::UnassignedSpelling("sometimes".to_string()))
        );
        assert_eq!(
            read_raw_stream_packaging_with_legacy_default(&object(
                json!({"raw_stream_packaging": 7})
            )),
            Err(CompositionError::NotAString)
        );
    }

    #[test]
    fn the_omission_reason_names_the_flag_that_causes_it() {
        assert!(RAW_STREAM_OMITTED_REASON.contains("--package-raw-stream"));
        assert!(RAW_STREAM_OMITTED_REASON.contains("delta cache"));
    }
}
