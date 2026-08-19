use serde_json::{Map, Value};

pub const RAW_STREAM_PACKAGING_FIELD: &str = "raw_stream_packaging";

pub const RAW_STREAM_PACKAGING_PACKAGED: &str = "packaged";

pub const RAW_STREAM_PACKAGING_OMITTED: &str = "omitted";

pub const RAW_STREAM_OMITTED_REASON: &str =
    "recorded without --package-raw-stream: this recording ships the delta cache only, \
     so the raw collector wire stream was never packaged into the container";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStreamPackaging {
    Packaged,
    Omitted,
}

impl RawStreamPackaging {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Packaged => RAW_STREAM_PACKAGING_PACKAGED,
            Self::Omitted => RAW_STREAM_PACKAGING_OMITTED,
        }
    }

    pub fn requires_stream_entry(self) -> bool {
        matches!(self, Self::Packaged)
    }

    pub fn parse(value: &str) -> Result<Self, CompositionError> {
        match value {
            RAW_STREAM_PACKAGING_PACKAGED => Ok(Self::Packaged),
            RAW_STREAM_PACKAGING_OMITTED => Ok(Self::Omitted),
            other => Err(CompositionError::UnassignedSpelling(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionError {
    Missing,
    NotAString,
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
            Self::NotAString => write!(f, "{RAW_STREAM_PACKAGING_FIELD:?} is not a JSON string"),
            Self::UnassignedSpelling(value) => write!(
                f,
                "{RAW_STREAM_PACKAGING_FIELD:?} has unassigned value {value:?}; \
                 expected {RAW_STREAM_PACKAGING_PACKAGED:?} or {RAW_STREAM_PACKAGING_OMITTED:?}"
            ),
        }
    }
}

impl std::error::Error for CompositionError {}

pub fn read_raw_stream_packaging(
    object: &Map<String, Value>,
) -> Result<RawStreamPackaging, CompositionError> {
    match object.get(RAW_STREAM_PACKAGING_FIELD) {
        None => Err(CompositionError::Missing),
        Some(Value::String(value)) => RawStreamPackaging::parse(value),
        Some(_) => Err(CompositionError::NotAString),
    }
}

pub fn read_raw_stream_packaging_with_legacy_default(
    object: &Map<String, Value>,
) -> Result<RawStreamPackaging, CompositionError> {
    match read_raw_stream_packaging(object) {
        Err(CompositionError::Missing) => Ok(RawStreamPackaging::Packaged),
        other => other,
    }
}

pub fn container_raw_stream_packaging(
    container: &crate::container::Container,
) -> Result<RawStreamPackaging, String> {
    let entries = container.entries_by_role(RECORDER_MANIFEST_ROLE);
    let entry = match entries.as_slice() {
        [entry] => entry,
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

const RECORDER_MANIFEST_ROLE: &str = "recorder_manifest";

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

    #[test]
    fn unassigned_spelling_rejects() {
        for value in ["", "PACKAGED", "none", "true", "omitted "] {
            let error = RawStreamPackaging::parse(value)
                .expect_err("unassigned composition spelling must reject");
            assert_eq!(
                error,
                CompositionError::UnassignedSpelling(value.to_string())
            );
            assert!(
                error.to_string().contains("unassigned"),
                "the rejection must name the cause, got {error}"
            );
        }
    }

    #[test]
    fn wrong_json_type_rejects() {
        for value in [
            json!({"raw_stream_packaging": true}),
            json!({"raw_stream_packaging": 1}),
        ] {
            assert_eq!(
                read_raw_stream_packaging(&object(value)),
                Err(CompositionError::NotAString)
            );
        }
    }

    #[test]
    fn missing_field_rejects_in_the_strict_reader() {
        let error = read_raw_stream_packaging(&object(json!({}))).expect_err("missing must reject");
        assert_eq!(error, CompositionError::Missing);
        assert!(error.to_string().contains("never assumed"));
    }

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
            Err(CompositionError::UnassignedSpelling(
                "sometimes".to_string()
            ))
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
