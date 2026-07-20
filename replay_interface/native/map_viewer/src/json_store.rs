use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) enum JsonStoreError {
    Read {
        role: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Corrupt {
        role: &'static str,
        path: PathBuf,
        source: serde_json::Error,
    },
    Invalid {
        role: &'static str,
        path: PathBuf,
        detail: String,
    },
}

impl JsonStoreError {
    pub(crate) fn invalid(
        path: impl Into<PathBuf>,
        role: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self::Invalid {
            role,
            path: path.into(),
            detail: detail.into(),
        }
    }

    pub(crate) fn corrupt(
        path: impl Into<PathBuf>,
        role: &'static str,
        source: serde_json::Error,
    ) -> Self {
        Self::Corrupt {
            role,
            path: path.into(),
            source,
        }
    }
}

impl std::fmt::Display for JsonStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { role, path, source } => write!(
                formatter,
                "replay JSON store {role} {} is present but unreadable: {source}",
                path.display()
            ),
            Self::Corrupt { role, path, source } => write!(
                formatter,
                "replay JSON store {role} {} is corrupt: {source}",
                path.display()
            ),
            Self::Invalid { role, path, detail } => write!(
                formatter,
                "replay JSON store {role} {} is invalid: {detail}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for JsonStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Corrupt { source, .. } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}

pub(crate) fn read_optional_json<T: DeserializeOwned>(
    path: &Path,
    role: &'static str,
) -> std::result::Result<Option<T>, JsonStoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(JsonStoreError::Read {
                role,
                path: path.to_path_buf(),
                source,
            })
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| JsonStoreError::corrupt(path, role, source))
}

pub(crate) fn read_required_json<T: DeserializeOwned>(
    path: &Path,
    role: &'static str,
) -> std::result::Result<T, JsonStoreError> {
    match read_optional_json(path, role)? {
        Some(value) => Ok(value),
        None => Err(JsonStoreError::invalid(path, role, "file is absent")),
    }
}

pub(crate) fn decode_json_value<T: DeserializeOwned>(
    value: serde_json::Value,
    path: &Path,
    role: &'static str,
) -> std::result::Result<T, JsonStoreError> {
    serde_json::from_value(value).map_err(|source| JsonStoreError::corrupt(path, role, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct FixtureRow {
        value: u32,
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "map-viewer-json-store-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn optional_json_distinguishes_absent_from_corrupt() {
        let path = temp_file("absent-corrupt");
        assert_eq!(
            read_optional_json::<FixtureRow>(&path, "fixture").expect("absence is valid"),
            None
        );
        fs::write(&path, b"{not-json").expect("write corrupt fixture");
        let error = read_optional_json::<FixtureRow>(&path, "fixture")
            .expect_err("present corrupt JSON must not degrade to absent");
        assert!(matches!(error, JsonStoreError::Corrupt { .. }));
        assert!(error.to_string().contains("is corrupt"));
        let _ = fs::remove_file(path);
    }
}
