//! WHAT BUILD PRODUCED THIS CAPTURE — written onto the session manifest, and
//! read back by anything that measures a stream.
//!
//! # The defect this closes
//!
//! Runs `t34` and `t35` (2026-08-07) were captured with a collector compiled
//! `--features range-tracer`. The tracer records every guarded read, and its
//! own contract says a traced frame's timings are not comparable with an
//! untraced one's. Nothing in either manifest said the feature was compiled in.
//! Their frame timings were compared against an untraced run's, a 12.7 ms shift
//! was opened as a production regression, and the feature build was only
//! noticed a day later by eye, from the collector DLL's sha256 differing.
//!
//! A sha names a build exactly — to somebody who already holds that build. It
//! cannot answer "was this compiled with the tracer", which is the question
//! that decides whether the numbers are evidence. This module carries the
//! collector's own compile-time answer from the authenticated capture-ready
//! payload onto the manifest, and gives the audit tools one way to read it.
//!
//! # Absent is absent
//!
//! Three states, never two:
//!
//! * **no declaration reached the recorder** — no capture-ready handshake on
//!   this path. `collector_build` is `null`.
//! * **the collector declared nothing** — it predates
//!   [`CollectorFeatureBits::DECLARED`]. `declared: false`, and
//!   `diagnostic_build` is `null`, NOT `false`.
//! * **the collector declared** — `features` is the exact cargo feature list,
//!   possibly empty, and `diagnostic_build` is a real boolean.
//!
//! Every reader here preserves that split. A consumer that renders an absent
//! declaration as "clean production build" has recreated the original defect
//! with more ceremony.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use compact_contract::control_transport::payloads::CollectorFeatureBits;
use serde_json::{json, Value};

/// The session manifest's leaf name. A capture writes it into the same
/// directory as its stream, which is the whole basis of [`for_stream`].
pub const SESSION_MANIFEST_FILE_NAME: &str = "native_session_manifest.json";

/// The manifest key carrying the block this module writes.
pub const COLLECTOR_BUILD_KEY: &str = "collector_build";

/// The manifest's top-level prominence copy of the one fact a measurement has
/// to check first.
pub const DIAGNOSTIC_BUILD_KEY: &str = "diagnostic_build";

/// The `collector_build` manifest block.
///
/// `None` means no capture-ready payload reached this recorder on this path, so
/// there is nothing to report — written as JSON `null` rather than as an empty
/// object, because an empty object reads like a declaration that said nothing.
pub fn collector_build_json(features: Option<CollectorFeatureBits>) -> Value {
    let Some(features) = features else {
        return Value::Null;
    };
    json!({
        "declared": features.declared(),
        "feature_bits": features.bits(),
        // `null` when undeclared: the producer did not say, which is not the
        // same fact as an empty feature list.
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

/// The manifest's top-level `diagnostic_build`: `null` unless the collector
/// actually declared, so a gate reading only this key cannot mistake silence
/// for a clean build.
pub fn diagnostic_build_json(features: Option<CollectorFeatureBits>) -> Value {
    features
        .and_then(CollectorFeatureBits::diagnostic_build)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

/// What a measuring tool learned about the build behind a stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildDeclaration {
    /// No manifest beside the stream. Ordinary for a hand-copied or
    /// synthesized stream; never an error.
    ManifestAbsent { looked_in: PathBuf },
    /// A manifest exists but could not be read or parsed. Reported, never
    /// fatal: the audit's own findings do not depend on it.
    ManifestUnreadable { path: PathBuf, reason: String },
    /// A manifest exists and carries no declaration — it predates the schema,
    /// or the capture had no capture-ready handshake.
    Undeclared { path: PathBuf },
    /// The collector declared its build.
    Declared {
        path: PathBuf,
        diagnostic: bool,
        features: Vec<String>,
    },
}

impl BuildDeclaration {
    /// `Some(true)` only when a manifest declared a diagnostic build.
    /// `None` whenever nothing was declared — absence is not a clean build.
    pub fn diagnostic(&self) -> Option<bool> {
        match self {
            Self::Declared { diagnostic, .. } => Some(*diagnostic),
            _ => None,
        }
    }
}

/// Read the build declaration for `stream_path` from the session manifest in
/// the same directory.
///
/// Total: every failure mode is one of the variants, and none of them is an
/// error the caller has to handle. An audit that cannot find a manifest still
/// audits.
pub fn for_stream(stream_path: &Path) -> BuildDeclaration {
    let manifest_path = stream_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SESSION_MANIFEST_FILE_NAME);
    from_manifest_path(&manifest_path)
}

/// [`for_stream`] against an explicit manifest path.
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

/// The parse itself, split out so it is testable without a file.
///
/// # `feature_bits` is the only authority
///
/// Everything else in the block — `declared`, `features`, `diagnostic_build`,
/// and the manifest's top-level `diagnostic_build` copy — is a RENDERING of
/// that one word, produced by [`collector_build_json`] from the collector's own
/// authenticated declaration. So the reader decodes the word and DERIVES the
/// answer from it, then requires every rendering to agree.
///
/// Trusting the rendered `diagnostic_build` instead would let a block that
/// names `range-tracer` in `features` and `false` in `diagnostic_build` print
/// "no diagnostic features" — a hand-edited or half-written manifest rendering
/// a traced capture as clean, which is the exact claim this module exists to
/// make impossible. A disagreement is not resolved in either direction: it
/// means the block cannot be trusted, so it degrades to UNREADABLE, which
/// reports as unknown and never as clean.
pub fn from_manifest_value(manifest_path: &Path, document: &Value) -> BuildDeclaration {
    let path = manifest_path.to_path_buf();
    let unreadable = |reason: String| BuildDeclaration::ManifestUnreadable {
        path: manifest_path.to_path_buf(),
        reason,
    };
    // A missing key, an explicit null, and a non-object are the same fact: this
    // manifest predates the declaration. Refusing here would make every capture
    // recorded before today unauditable.
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
    // The rendered fields must be exactly what these bits mean.
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
    // The manifest's top-level prominence copy, when it is present, is the
    // field a gate is most likely to read on its own. It gets the same
    // treatment: agree with the authority or the manifest is untrustworthy.
    // ABSENT is tolerated — an older writer produced the block without it.
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

/// The same finding as [`banner`], for a tool's machine-readable report.
///
/// `state` is the discriminant by name, so a consumer sees WHY the build is
/// unknown rather than only that it is; `diagnostic_build` stays `null` in
/// every unknown state.
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

/// The banner a measuring tool prints before its own output.
///
/// Loud and multi-line for a diagnostic build, because the finding it qualifies
/// is every number below it. One quiet line otherwise — the tool still says
/// what it looked for and what it found, so "no warning" is distinguishable
/// from "did not check".
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
            let _ = writeln!(out, "!!! DIAGNOSTIC BUILD — TIMINGS BELOW ARE NOT PRODUCTION EVIDENCE");
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
        // Every capture recorded before this schema. The tool must still run,
        // and must not claim the build was clean.
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

        // An explicit null is the same fact as a missing key.
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

    /// A block whose rendered fields contradict its own `feature_bits` is
    /// UNTRUSTWORTHY, and untrustworthy reports as unknown — never as clean.
    ///
    /// Every one of these is a traced capture that would otherwise have printed
    /// "no diagnostic features", which is the single outcome this module exists
    /// to prevent. (Codex 019fdbba, finding 1.)
    #[test]
    fn a_block_that_contradicts_its_own_feature_bits_is_never_read_as_clean() {
        let traced = CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER;
        let contradictions = [
            // Bits say traced; the rendered boolean says clean.
            json!({
                "declared": true,
                "feature_bits": traced,
                "features": ["range-tracer"],
                "diagnostic_build": false,
            }),
            // Bits say traced; the rendered feature list is empty.
            json!({
                "declared": true,
                "feature_bits": traced,
                "features": [],
                "diagnostic_build": true,
            }),
            // Bits say traced; the block claims it never declared.
            json!({
                "declared": false,
                "feature_bits": traced,
                "features": ["range-tracer"],
                "diagnostic_build": true,
            }),
            // A feature list of the right length but the wrong content, and
            // non-string entries that a filter_map would have silently dropped.
            json!({
                "declared": true,
                "feature_bits": traced,
                "features": [7],
                "diagnostic_build": true,
            }),
            // No authority at all.
            json!({
                "declared": true,
                "features": ["range-tracer"],
                "diagnostic_build": true,
            }),
            // An authority this build cannot decode.
            json!({
                "declared": true,
                "feature_bits": 1u32 << 31,
                "features": [],
                "diagnostic_build": false,
            }),
            // Feature bits without the declaration bit.
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

    /// The manifest's top-level prominence copy is held to the same authority,
    /// and its ABSENCE is tolerated.
    #[test]
    fn the_top_level_diagnostic_copy_must_agree_with_the_authority() {
        let features = CollectorFeatureBits::from_bits(
            CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER,
        )
        .unwrap();
        let block = collector_build_json(Some(features));

        // Agreeing: the pair the recorder actually writes.
        let agreeing = json!({
            COLLECTOR_BUILD_KEY: block.clone(),
            DIAGNOSTIC_BUILD_KEY: diagnostic_build_json(Some(features)),
        });
        assert_eq!(
            from_manifest_value(Path::new("m.json"), &agreeing).diagnostic(),
            Some(true)
        );

        // Disagreeing: a gate reading only the top-level key would have been
        // told the opposite of the truth.
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

        // Absent: tolerated, because a writer that predates the prominence copy
        // still produced a trustworthy block.
        let absent = json!({ COLLECTOR_BUILD_KEY: block });
        assert_eq!(
            from_manifest_value(Path::new("m.json"), &absent).diagnostic(),
            Some(true)
        );
    }

    /// Whatever the recorder writes, this reader reads back — for every state
    /// the recorder can be in.
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
