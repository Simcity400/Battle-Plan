use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use compact_contract::control_transport::payloads::CollectorFeatureBits;
use serde_json::{json, Value};

pub const SESSION_MANIFEST_FILE_NAME: &str = "native_session_manifest.json";

pub const COLLECTOR_BUILD_KEY: &str = "collector_build";

pub const DIAGNOSTIC_BUILD_KEY: &str = "diagnostic_build";

pub fn collector_build_json(features: Option<CollectorFeatureBits>) -> Value {
    let Some(features) = features else {
        return Value::Null;
    };
    json!({
        "declared": features.declared(),
        "feature_bits": features.bits(),
        "features": features
            .feature_names()
            .map(|names| Value::from(names.iter().map(|name| Value::from(*name)).collect::<Vec<_>>()))
            .unwrap_or(Value::Null),
        "diagnostic_build": features
            .diagnostic_build()
            .map(Value::from)
            .unwrap_or(Value::Null),
        "source": "capture_ready_v2.collector_features",
    })
}

pub fn diagnostic_build_json(features: Option<CollectorFeatureBits>) -> Value {
    features
        .and_then(CollectorFeatureBits::diagnostic_build)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildDeclaration {
    ManifestAbsent {
        looked_in: PathBuf,
    },
    ManifestUnreadable {
        path: PathBuf,
        reason: String,
    },
    Undeclared {
        path: PathBuf,
    },
    Declared {
        path: PathBuf,
        diagnostic: bool,
        features: Vec<String>,
    },
}

impl BuildDeclaration {
    pub fn diagnostic(&self) -> Option<bool> {
        match self {
            Self::Declared { diagnostic, .. } => Some(*diagnostic),
            _ => None,
        }
    }
}

pub fn for_stream(stream_path: &Path) -> BuildDeclaration {
    let manifest_path = stream_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SESSION_MANIFEST_FILE_NAME);
    from_manifest_path(&manifest_path)
}

pub fn from_manifest_path(manifest_path: &Path) -> BuildDeclaration {
    let text = match std::fs::read_to_string(manifest_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return BuildDeclaration::ManifestAbsent {
                looked_in: manifest_path.to_path_buf(),
            }
        }
        Err(error) => {
            return BuildDeclaration::ManifestUnreadable {
                path: manifest_path.to_path_buf(),
                reason: error.to_string(),
            }
        }
    };
    let document: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            return BuildDeclaration::ManifestUnreadable {
                path: manifest_path.to_path_buf(),
                reason: error.to_string(),
            }
        }
    };
    from_manifest_value(manifest_path, &document)
}

pub fn from_manifest_value(manifest_path: &Path, document: &Value) -> BuildDeclaration {
    let path = manifest_path.to_path_buf();
    let unreadable = |reason: String| BuildDeclaration::ManifestUnreadable {
        path: manifest_path.to_path_buf(),
        reason,
    };
    let Some(block) = document.get(COLLECTOR_BUILD_KEY).and_then(Value::as_object) else {
        return BuildDeclaration::Undeclared { path };
    };
    let Some(raw_bits) = block
        .get("feature_bits")
        .and_then(Value::as_u64)
        .and_then(|bits| u32::try_from(bits).ok())
    else {
        return unreadable(format!(
            "{COLLECTOR_BUILD_KEY}.feature_bits is missing or is not a u32"
        ));
    };
    let features = match CollectorFeatureBits::from_bits(raw_bits) {
        Ok(features) => features,
        Err(error) => {
            return unreadable(format!(
                "{COLLECTOR_BUILD_KEY}.feature_bits {raw_bits:#x} is not a valid declaration: \
                 {error}"
            ))
        }
    };
    if block.get("declared").and_then(Value::as_bool) != Some(features.declared()) {
        return unreadable(format!(
            "{COLLECTOR_BUILD_KEY}.declared disagrees with feature_bits {raw_bits:#x}"
        ));
    }
    let rendered_diagnostic = block.get("diagnostic_build").unwrap_or(&Value::Null);
    let expected_diagnostic = features
        .diagnostic_build()
        .map(Value::from)
        .unwrap_or(Value::Null);
    if *rendered_diagnostic != expected_diagnostic {
        return unreadable(format!(
            "{COLLECTOR_BUILD_KEY}.diagnostic_build disagrees with feature_bits {raw_bits:#x}"
        ));
    }
    let rendered_features = block.get("features").unwrap_or(&Value::Null);
    let expected_features = features
        .feature_names()
        .map(|names| {
            Value::from(
                names
                    .iter()
                    .map(|name| Value::from(*name))
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or(Value::Null);
    if *rendered_features != expected_features {
        return unreadable(format!(
            "{COLLECTOR_BUILD_KEY}.features disagrees with feature_bits {raw_bits:#x}"
        ));
    }
    if let Some(top_level) = document.get(DIAGNOSTIC_BUILD_KEY) {
        if *top_level != expected_diagnostic {
            return unreadable(format!(
                "top-level {DIAGNOSTIC_BUILD_KEY} disagrees with \
                 {COLLECTOR_BUILD_KEY}.feature_bits {raw_bits:#x}"
            ));
        }
    }
    let Some(diagnostic) = features.diagnostic_build() else {
        return BuildDeclaration::Undeclared { path };
    };
    BuildDeclaration::Declared {
        path,
        diagnostic,
        features: features
            .feature_names()
            .unwrap_or_default()
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

pub fn summary_json(declaration: &BuildDeclaration) -> Value {
    match declaration {
        BuildDeclaration::Declared {
            path,
            diagnostic,
            features,
        } => json!({
            "state": "declared",
            "manifest": path.display().to_string(),
            "diagnostic_build": diagnostic,
            "features": features,
        }),
        BuildDeclaration::Undeclared { path } => json!({
            "state": "undeclared",
            "manifest": path.display().to_string(),
            "diagnostic_build": Value::Null,
            "features": Value::Null,
        }),
        BuildDeclaration::ManifestAbsent { looked_in } => json!({
            "state": "manifest_absent",
            "manifest": looked_in.display().to_string(),
            "diagnostic_build": Value::Null,
            "features": Value::Null,
        }),
        BuildDeclaration::ManifestUnreadable { path, reason } => json!({
            "state": "manifest_unreadable",
            "manifest": path.display().to_string(),
            "reason": reason,
            "diagnostic_build": Value::Null,
            "features": Value::Null,
        }),
    }
}

pub fn banner(declaration: &BuildDeclaration) -> String {
    let mut out = String::new();
    match declaration {
        BuildDeclaration::Declared {
            diagnostic: true,
            features,
            path,
        } => {
            let names = if features.is_empty() {
                "(none named)".to_string()
            } else {
                features.join(", ")
            };
            let _ = writeln!(
                out,
                "!!! ============================================================"
            );
            let _ = writeln!(
                out,
                "!!! DIAGNOSTIC BUILD — TIMINGS BELOW ARE NOT PRODUCTION EVIDENCE"
            );
            let _ = writeln!(out, "!!! collector features: {names}");
            let _ = writeln!(out, "!!! declared by: {}", path.display());
            let _ = writeln!(
                out,
                "!!! A diagnostic collector changes what a capture costs. Compare"
            );
            let _ = writeln!(
                out,
                "!!! these numbers only against another capture from the SAME build."
            );
            let _ = writeln!(
                out,
                "!!! ============================================================"
            );
        }
        BuildDeclaration::Declared {
            diagnostic: false,
            features,
            ..
        } => {
            let names = if features.is_empty() {
                "none".to_string()
            } else {
                features.join(", ")
            };
            let _ = writeln!(
                out,
                "collector build: declared, no diagnostic features (features: {names})"
            );
        }
        BuildDeclaration::Undeclared { path } => {
            let _ = writeln!(
                out,
                "collector build: NOT DECLARED by {} — this capture predates the \
                 declaration, so whether it carried a diagnostic feature is UNKNOWN",
                path.display()
            );
        }
        BuildDeclaration::ManifestAbsent { looked_in } => {
            let _ = writeln!(
                out,
                "collector build: UNKNOWN — no session manifest at {}",
                looked_in.display()
            );
        }
        BuildDeclaration::ManifestUnreadable { path, reason } => {
            let _ = writeln!(
                out,
                "collector build: UNKNOWN — session manifest {} could not be read ({reason})",
                path.display()
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(block: Value) -> Value {
        json!({ "schema": "native_collector_session_v1", COLLECTOR_BUILD_KEY: block })
    }

    #[test]
    fn a_declared_diagnostic_build_produces_the_loud_banner() {
        let features = CollectorFeatureBits::from_bits(
            CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER,
        )
        .unwrap();
        let block = collector_build_json(Some(features));
        assert_eq!(block["declared"], json!(true));
        assert_eq!(block["diagnostic_build"], json!(true));
        assert_eq!(block["features"], json!(["range-tracer"]));
        assert_eq!(diagnostic_build_json(Some(features)), json!(true));

        let declaration = from_manifest_value(Path::new("run/manifest.json"), &manifest(block));
        assert_eq!(declaration.diagnostic(), Some(true));
        let banner = banner(&declaration);
        assert!(banner.contains("DIAGNOSTIC BUILD"), "{banner}");
        assert!(banner.contains("range-tracer"), "{banner}");
    }

    #[test]
    fn a_declared_clean_build_says_so_without_shouting() {
        let features = CollectorFeatureBits::from_bits(CollectorFeatureBits::DECLARED).unwrap();
        let block = collector_build_json(Some(features));
        assert_eq!(block["features"], json!([]));
        assert_eq!(block["diagnostic_build"], json!(false));
        let declaration = from_manifest_value(Path::new("run/manifest.json"), &manifest(block));
        assert_eq!(declaration.diagnostic(), Some(false));
        let banner = banner(&declaration);
        assert!(!banner.contains("DIAGNOSTIC BUILD"), "{banner}");
        assert!(banner.contains("no diagnostic features"), "{banner}");
    }

    #[test]
    fn an_undeclaring_collector_is_absent_and_never_false() {
        let features = CollectorFeatureBits::undeclared();
        let block = collector_build_json(Some(features));
        assert_eq!(block["declared"], json!(false));
        assert_eq!(block["features"], Value::Null);
        assert_eq!(block["diagnostic_build"], Value::Null);
        assert_eq!(diagnostic_build_json(Some(features)), Value::Null);
        let declaration = from_manifest_value(Path::new("run/manifest.json"), &manifest(block));
        assert_eq!(
            declaration,
            BuildDeclaration::Undeclared {
                path: PathBuf::from("run/manifest.json")
            }
        );
        assert_eq!(declaration.diagnostic(), None);
        assert!(banner(&declaration).contains("UNKNOWN"));
    }

    #[test]
    fn no_handshake_writes_null_rather_than_an_empty_declaration() {
        assert_eq!(collector_build_json(None), Value::Null);
        assert_eq!(diagnostic_build_json(None), Value::Null);
    }

    #[test]
    fn an_old_manifest_without_the_key_audits_without_erroring() {
        let old = json!({
            "schema": "native_collector_session_v1",
            "pid": 1234,
            "dll": "C:\\\\build\\\\collector.dll",
        });
        let declaration = from_manifest_value(Path::new("t35/native_session_manifest.json"), &old);
        assert!(matches!(declaration, BuildDeclaration::Undeclared { .. }));
        assert_eq!(declaration.diagnostic(), None);
        let banner = banner(&declaration);
        assert!(banner.contains("NOT DECLARED"), "{banner}");
        assert!(banner.contains("UNKNOWN"), "{banner}");
        assert!(!banner.contains("DIAGNOSTIC BUILD"), "{banner}");

        let nulled = json!({ COLLECTOR_BUILD_KEY: Value::Null });
        assert!(matches!(
            from_manifest_value(Path::new("m.json"), &nulled),
            BuildDeclaration::Undeclared { .. }
        ));
    }

    #[test]
    fn a_missing_manifest_is_reported_not_fatal() {
        let declaration = for_stream(Path::new(
            "this-directory-does-not-exist-eh/collector_stream.bin",
        ));
        match &declaration {
            BuildDeclaration::ManifestAbsent { looked_in } => {
                assert!(looked_in.ends_with(SESSION_MANIFEST_FILE_NAME));
            }
            other => panic!("expected ManifestAbsent, got {other:?}"),
        }
        assert_eq!(declaration.diagnostic(), None);
        assert!(banner(&declaration).contains("no session manifest"));
    }

    #[test]
    fn a_declared_block_without_a_diagnostic_boolean_is_malformed_not_clean() {
        let block = json!({ "declared": true, "features": ["range-tracer"] });
        let declaration = from_manifest_value(Path::new("m.json"), &manifest(block));
        assert!(matches!(
            declaration,
            BuildDeclaration::ManifestUnreadable { .. }
        ));
        assert_eq!(declaration.diagnostic(), None);
    }

    #[test]
    fn a_block_that_contradicts_its_own_feature_bits_is_never_read_as_clean() {
        let traced = CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER;
        let contradictions = [
            json!({
                "declared": true,
                "feature_bits": traced,
                "features": ["range-tracer"],
                "diagnostic_build": false,
            }),
            json!({
                "declared": true,
                "feature_bits": traced,
                "features": [],
                "diagnostic_build": true,
            }),
            json!({
                "declared": false,
                "feature_bits": traced,
                "features": ["range-tracer"],
                "diagnostic_build": true,
            }),
            json!({
                "declared": true,
                "feature_bits": traced,
                "features": [7],
                "diagnostic_build": true,
            }),
            json!({
                "declared": true,
                "features": ["range-tracer"],
                "diagnostic_build": true,
            }),
            json!({
                "declared": true,
                "feature_bits": 1u32 << 31,
                "features": [],
                "diagnostic_build": false,
            }),
            json!({
                "declared": true,
                "feature_bits": CollectorFeatureBits::RANGE_TRACER,
                "features": ["range-tracer"],
                "diagnostic_build": true,
            }),
        ];
        for block in contradictions {
            let declaration = from_manifest_value(Path::new("m.json"), &manifest(block.clone()));
            assert!(
                matches!(declaration, BuildDeclaration::ManifestUnreadable { .. }),
                "{block} was not refused: {declaration:?}"
            );
            assert_eq!(declaration.diagnostic(), None, "{block}");
            let banner = banner(&declaration);
            assert!(banner.contains("UNKNOWN"), "{banner}");
            assert!(!banner.contains("no diagnostic features"), "{banner}");
        }
    }

    #[test]
    fn the_top_level_diagnostic_copy_must_agree_with_the_authority() {
        let features = CollectorFeatureBits::from_bits(
            CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER,
        )
        .unwrap();
        let block = collector_build_json(Some(features));

        let agreeing = json!({
            COLLECTOR_BUILD_KEY: block.clone(),
            DIAGNOSTIC_BUILD_KEY: diagnostic_build_json(Some(features)),
        });
        assert_eq!(
            from_manifest_value(Path::new("m.json"), &agreeing).diagnostic(),
            Some(true)
        );

        let disagreeing = json!({
            COLLECTOR_BUILD_KEY: block.clone(),
            DIAGNOSTIC_BUILD_KEY: false,
        });
        let declaration = from_manifest_value(Path::new("m.json"), &disagreeing);
        assert!(matches!(
            declaration,
            BuildDeclaration::ManifestUnreadable { .. }
        ));
        assert_eq!(declaration.diagnostic(), None);

        let absent = json!({ COLLECTOR_BUILD_KEY: block });
        assert_eq!(
            from_manifest_value(Path::new("m.json"), &absent).diagnostic(),
            Some(true)
        );
    }

    #[test]
    fn every_block_the_recorder_writes_round_trips_through_the_reader() {
        for bits in [
            None,
            Some(CollectorFeatureBits::undeclared()),
            Some(CollectorFeatureBits::from_bits(CollectorFeatureBits::DECLARED).unwrap()),
            Some(
                CollectorFeatureBits::from_bits(
                    CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER,
                )
                .unwrap(),
            ),
            Some(CollectorFeatureBits::from_bits(CollectorFeatureBits::KNOWN_MASK).unwrap()),
        ] {
            let document = json!({
                COLLECTOR_BUILD_KEY: collector_build_json(bits),
                DIAGNOSTIC_BUILD_KEY: diagnostic_build_json(bits),
            });
            let declaration = from_manifest_value(Path::new("m.json"), &document);
            assert_eq!(
                declaration.diagnostic(),
                bits.and_then(CollectorFeatureBits::diagnostic_build),
                "round trip lost the answer for {bits:?}"
            );
            if let BuildDeclaration::Declared { features, .. } = &declaration {
                assert_eq!(
                    features.iter().map(String::as_str).collect::<Vec<_>>(),
                    bits.and_then(CollectorFeatureBits::feature_names)
                        .unwrap_or_default()
                );
            }
        }
    }
}
