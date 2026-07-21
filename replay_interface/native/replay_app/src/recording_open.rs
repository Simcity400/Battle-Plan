//! Single shipped recording-open policy.
//!
//! Every product surface enters through the explicit bounded decoder
//! capability. Snapshot-native recordings pass through without invoking it;
//! compact-only containers are authenticated into the product-owned spool and
//! folded by the extractor's one production `WorldFold`. There is deliberately
//! no ordinary-open or raw-stream fallback in this module.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use replay_core::{
    CertifiedPrefoldSpoolDecoder, CertifiedPrefoldSpoolRequest, ContentManifest,
    PackagedDecisionCatalog, PackagedG2EngineCapture, PortableDesignCatalog, PrefoldDecodeError,
    PrefoldSourceReceipt, PrefoldStreamIdentityMode, PrefoldedSpool, PrefoldedSpoolEntry,
    RawFrame, RecordedCampaignIdentity, ReplaySource, SourceError,
};

static PRODUCTION_DECODER: ProductionPrefoldWorker = ProductionPrefoldWorker;

const PREFOLD_WORKER_DECODER_ID: &str =
    "extractor.replay_decoder.production_world_fold.worker.v1";

fn prefold_identity_mode_label(mode: PrefoldStreamIdentityMode) -> &'static str {
    match mode {
        PrefoldStreamIdentityMode::Legacy => "legacy",
        PrefoldStreamIdentityMode::IdentityV1 => "identity-v1",
    }
}

/// Process boundary for the extractor-owned production WorldFold. Keeping the
/// worker as a sibling executable preserves replay_app's zero-extractor Cargo
/// dependency while still making compact-only packages playable in product.
struct ProductionPrefoldWorker;

impl CertifiedPrefoldSpoolDecoder for ProductionPrefoldWorker {
    fn decoder_id(&self) -> &'static str {
        PREFOLD_WORKER_DECODER_ID
    }

    fn decode_certified_spool(
        &self,
        request: CertifiedPrefoldSpoolRequest,
    ) -> Result<PrefoldedSpool, PrefoldDecodeError> {
        let identity_mode = request.identity_mode();
        let (game_id, raw_path, snapshot_dir, source_len, source_sha256) =
            request.into_parts();
        let worker = prefold_worker_path()?;
        let output = Command::new(&worker)
            .arg("--game-id")
            .arg(&game_id)
            .arg("--source")
            .arg(&raw_path)
            .arg("--snapshot-dir")
            .arg(&snapshot_dir)
            .arg("--source-len")
            .arg(source_len.to_string())
            .arg("--source-sha256")
            .arg(&source_sha256)
            .arg("--identity-mode")
            .arg(prefold_identity_mode_label(identity_mode))
            .output()
            .map_err(|error| {
                PrefoldDecodeError::new(format!(
                    "launch production prefold worker {}: {error}",
                    worker.display()
                ))
            })?;
        if !output.status.success() {
            return Err(PrefoldDecodeError::new(format!(
                "production prefold worker exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            PrefoldDecodeError::new(format!(
                "production prefold worker returned malformed JSON: {error}"
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            PrefoldDecodeError::new("production prefold worker receipt is not an object")
        })?;
        if object.get("schema").and_then(serde_json::Value::as_str)
            != Some("hoi4_prefold_worker_receipt_v2")
            || object.get("decoder_id").and_then(serde_json::Value::as_str)
                != Some(PREFOLD_WORKER_DECODER_ID)
            || object.get("game_id").and_then(serde_json::Value::as_str)
                != Some(game_id.as_str())
            || object
                .get("identity_mode")
                .and_then(serde_json::Value::as_str)
                != Some(prefold_identity_mode_label(identity_mode))
        {
            return Err(PrefoldDecodeError::new(
                "production prefold worker receipt identity mismatch",
            ));
        }
        let schema_version = object.get("schema_version").cloned().ok_or_else(|| {
            PrefoldDecodeError::new("production prefold worker omitted schema_version")
        })?;
        let rows = object
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                PrefoldDecodeError::new("production prefold worker omitted entries array")
            })?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let row = row.as_object().ok_or_else(|| {
                PrefoldDecodeError::new("production prefold worker entry is not an object")
            })?;
            let hour_index = row
                .get("hour_index")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted hour_index"))?;
            let date = row
                .get("date")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted date"))?;
            let file_name = row
                .get("file_name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted file_name"))?;
            let byte_len = row
                .get("byte_len")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted byte_len"))?;
            let sha256 = row
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted sha256"))?;
            entries.push(PrefoldedSpoolEntry::try_new(
                hour_index, date, file_name, byte_len, sha256,
            )?);
        }
        PrefoldedSpool::try_new(game_id, schema_version, entries)
    }
}

fn prefold_worker_path() -> Result<PathBuf, PrefoldDecodeError> {
    let current = std::env::current_exe().map_err(|error| {
        PrefoldDecodeError::new(format!("resolve replay_app executable path: {error}"))
    })?;
    let parent = current.parent().ok_or_else(|| {
        PrefoldDecodeError::new("replay_app executable has no parent directory")
    })?;
    let worker = parent.join(if cfg!(windows) {
        "replay_prefold_worker.exe"
    } else {
        "replay_prefold_worker"
    });
    let meta = fs::symlink_metadata(&worker).map_err(|error| {
        PrefoldDecodeError::new(format!(
            "production prefold worker is not installed beside replay_app at {}: {error}",
            worker.display()
        ))
    })?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() {
        return Err(PrefoldDecodeError::new(format!(
            "production prefold worker is not a regular non-symlink file: {}",
            worker.display()
        )));
    }
    Ok(worker)
}

/// Open one recording under the shipped activation policy.
pub(crate) fn open_product_recording(
    path: &Path,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_product_recording_segment(path, None)
}

/// Open either one ordinary portable recording or one explicitly selected
/// member of an ordered CID1 rollover package set.
pub(crate) fn open_product_recording_segment(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    Ok(open_product_recording_segment_with_authority(path, logical_stream_id)?.source)
}

pub(crate) struct ProductRecordingOpen {
    pub source: Box<dyn ReplaySource>,
    /// Whole-file SHA-256 from the exact container handle moved into `source`.
    /// Directory stores have no single-file authority and return `None`.
    pub authority_sha256: Option<String>,
}

pub(crate) fn open_product_recording_segment_with_authority(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<ProductRecordingOpen, SourceError> {
    // Resolve the stable machine-local root before authenticating a package-set
    // member, so the normal container open follows the retained-handle join
    // immediately with no intervening filesystem setup.
    let root = product_spool_root()?;
    let is_rollover = path.file_name().and_then(|value| value.to_str())
        == Some(replay_core::ROLLOVER_PACKAGE_SET_FILE)
        || (path.is_dir() && path.join(replay_core::ROLLOVER_PACKAGE_SET_FILE).is_file());
    let (source, authority_sha256) = if is_rollover {
        let selected = replay_core::authenticate_rollover_segment(path, logical_stream_id)
            .map_err(SourceError::Unsupported)?;
        let sha256 = selected.package_sha256().to_string();
        let source = replay_core::open_container_with_spooled_decoder(
            selected.into_container(),
            root.path(),
            &PRODUCTION_DECODER,
        )?;
        (source, Some(sha256))
    } else {
        if let Some(selected) = logical_stream_id {
            return Err(SourceError::Unsupported(format!(
                "--logical-stream {selected} requires a {} descriptor or a directory containing one",
                replay_core::ROLLOVER_PACKAGE_SET_FILE
            )));
        }
        if path.is_file() {
            let (container, _bytes, sha256) = snapshot::Container::open_strict_with_file_receipt(
                path,
                &snapshot::ExpectedIdentity::default(),
            )
            .map_err(|error| {
                let message = format!("open retained replay container: {error}");
                // The retained-handle product path opens the snapshot container
                // directly, bypassing replay_core's Store adapter. Preserve the
                // same typed modset-mismatch signal here instead of flattening it
                // into a generic container/schema error at the UX boundary.
                match error {
                    snapshot::ContainerError::ModsetMismatch { .. } => {
                        SourceError::ModsetMismatch(message)
                    }
                    _ => SourceError::Open(message),
                }
            })?;
            let source = replay_core::open_container_with_spooled_decoder(
                container,
                root.path(),
                &PRODUCTION_DECODER,
            )?;
            (source, Some(sha256))
        } else {
            (
                replay_core::open_recording_with_spooled_decoder(
                    path,
                    root.path(),
                    &PRODUCTION_DECODER,
                )?,
                None,
            )
        }
    };
    Ok(ProductRecordingOpen {
        source: Box::new(ProductReplaySource {
            inner: source,
            _spool_root: root,
        }),
        authority_sha256,
    })
}

fn open_product_recording_at(
    path: &Path,
    spool_root: ProductSpoolRoot,
    decoder: &dyn CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let source = replay_core::open_recording_with_spooled_decoder(
        path,
        spool_root.path(),
        decoder,
    )?;
    Ok(Box::new(ProductReplaySource {
        inner: source,
        _spool_root: spool_root,
    }))
}

/// Resolve the machine-local parent for transient replay data. Nothing inside
/// a portable recording selects or influences this path.
fn product_spool_root() -> Result<ProductSpoolRoot, SourceError> {
    #[cfg(test)]
    let base = std::env::temp_dir();
    #[cfg(all(windows, not(test)))]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| SourceError::Open("LOCALAPPDATA is unavailable for the product replay spool".to_string()))?;
    #[cfg(all(not(windows), not(test)))]
    let base = std::env::temp_dir();

    let product = ensure_owned_child(&base, "battle-plan")?;
    let spool = ensure_owned_child(&product, "replay_spool")?;
    Ok(ProductSpoolRoot { canonical: spool })
}

fn ensure_owned_child(base: &Path, name: &str) -> Result<PathBuf, SourceError> {
    let base_meta = fs::symlink_metadata(base).map_err(|error| SourceError::Open(format!(
        "inspect product spool parent {}: {error}", base.display()
    )))?;
    if !base_meta.file_type().is_dir() || base_meta.file_type().is_symlink() {
        return Err(SourceError::Open(format!(
            "product spool parent is not a regular non-symlink directory: {}",
            base.display()
        )));
    }
    let canonical_base = fs::canonicalize(base).map_err(|error| SourceError::Open(format!(
        "canonicalize product spool parent {}: {error}", base.display()
    )))?;
    let child = canonical_base.join(name);
    match fs::create_dir(&child) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(SourceError::Open(format!(
                "create product spool component {}: {error}", child.display()
            )))
        }
    }
    let child_meta = fs::symlink_metadata(&child).map_err(|error| SourceError::Open(format!(
        "inspect product spool component {}: {error}", child.display()
    )))?;
    if !child_meta.file_type().is_dir() || child_meta.file_type().is_symlink() {
        return Err(SourceError::Open(format!(
            "product spool component is not a regular non-symlink directory: {}",
            child.display()
        )));
    }
    let canonical_child = fs::canonicalize(&child).map_err(|error| SourceError::Open(format!(
        "canonicalize product spool component {}: {error}", child.display()
    )))?;
    if canonical_child.parent() != Some(canonical_base.as_path()) {
        return Err(SourceError::Open(format!(
            "product spool component escaped its canonical parent: {}",
            child.display()
        )));
    }
    Ok(canonical_child)
}

/// The stable product-owned parent. Per-recording children and their recursive
/// cleanup remain owned by replay_core's identity-marked spool guard. The
/// parent deliberately persists: deleting a shared empty parent can race a
/// concurrent source between its root check and private-child creation.
struct ProductSpoolRoot {
    canonical: PathBuf,
}

impl ProductSpoolRoot {
    #[cfg(test)]
    fn ensure(path: PathBuf) -> Result<Self, SourceError> {
        fs::create_dir_all(&path).map_err(|error| SourceError::Open(format!(
            "create product replay spool {}: {error}",
            path.display()
        )))?;
        let meta = fs::symlink_metadata(&path).map_err(|error| SourceError::Open(format!(
            "inspect product replay spool {}: {error}",
            path.display()
        )))?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(SourceError::Open(format!(
                "product replay spool is not a regular non-symlink directory: {}",
                path.display()
            )));
        }
        let canonical = fs::canonicalize(&path).map_err(|error| SourceError::Open(format!(
            "canonicalize product replay spool {}: {error}",
            path.display()
        )))?;
        Ok(Self { canonical })
    }

    fn path(&self) -> &Path {
        &self.canonical
    }
}

/// Retains the canonical product-root identity alongside the replay source.
/// The inner replay_core owner deletes only its own authenticated child.
struct ProductReplaySource {
    inner: Box<dyn ReplaySource>,
    _spool_root: ProductSpoolRoot,
}

impl ReplaySource for ProductReplaySource {
    fn game_id(&self) -> &str { self.inner.game_id() }
    fn hours(&self) -> &[i64] { self.inner.hours() }
    fn warnings(&self) -> &[String] { self.inner.warnings() }
    fn content_manifest(&self) -> Option<&ContentManifest> { self.inner.content_manifest() }
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.inner.campaign_identity()
    }
    fn g2_engine_capture(&self) -> Option<&PackagedG2EngineCapture> {
        self.inner.g2_engine_capture()
    }
    fn design_catalog(&self) -> Option<&Arc<PortableDesignCatalog>> {
        self.inner.design_catalog()
    }
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.inner.decision_catalog()
    }
    fn prefold_receipt(&self) -> Option<&PrefoldSourceReceipt> {
        self.inner.prefold_receipt()
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        self.inner.frame(hour_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        CertifiedPrefoldSpoolRequest, PrefoldDecodeError, PrefoldedSpool,
        PrefoldedSpoolEntry,
    };
    use snapshot::{pack, PackEntry, PackIdentity};

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "replay_app_product_spool_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn worker_identity_mode_wire_labels_are_closed_and_stable() {
        assert_eq!(
            prefold_identity_mode_label(PrefoldStreamIdentityMode::Legacy),
            "legacy"
        );
        assert_eq!(
            prefold_identity_mode_label(PrefoldStreamIdentityMode::IdentityV1),
            "identity-v1"
        );
    }

    #[test]
    fn product_parent_is_stable_and_never_recursively_cleaned() {
        let parent = temp_root("parent_cleanup");
        let root_path = parent.join("spool");
        let root = ProductSpoolRoot::ensure(root_path.clone()).unwrap();
        fs::write(root_path.join("unexpected-residue"), b"preserve").unwrap();
        drop(root);
        assert!(root_path.exists(), "stable product root must be preserved");
        fs::remove_file(root_path.join("unexpected-residue")).unwrap();
        fs::remove_dir(&root_path).unwrap();
        fs::remove_dir(&parent).unwrap();
    }

    struct FixtureSpoolDecoder;

    struct CampaignIdentitySource {
        identity: RecordedCampaignIdentity,
    }

    impl ReplaySource for CampaignIdentitySource {
        fn game_id(&self) -> &str {
            "campaign-identity"
        }

        fn hours(&self) -> &[i64] {
            &[]
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
            Some(&self.identity)
        }

        fn frame(&self, _hour_index: i64) -> Result<RawFrame, SourceError> {
            Err(SourceError::Open("fixture has no frames".into()))
        }
    }

    #[test]
    fn product_source_preserves_validated_campaign_metadata() {
        let parent = temp_root("campaign_identity_forwarding");
        let spool_path = parent.join("spool");
        let source = ProductReplaySource {
            inner: Box::new(CampaignIdentitySource {
                identity: RecordedCampaignIdentity {
                    state: "unavailable".into(),
                    campaign_id: None,
                    game_unique_id: None,
                    game_unique_seed: None,
                    source: None,
                    source_build_profile: Some("hoi4-a729".into()),
                    observed_game_hour_raw: None,
                    stream_header_sha256: None,
                    resolution_note: Some("fixture unavailable".into()),
                    witnesses: replay_core::RecordedCampaignIdentityWitnesses {
                        start_date_raw: Some(1),
                        session: Some(2),
                        scenario: None,
                        initial_human_country_tags: None,
                        modset_identity: None,
                    },
                },
            }),
            _spool_root: ProductSpoolRoot::ensure(spool_path.clone()).unwrap(),
        };
        let identity = source
            .campaign_identity()
            .expect("product wrapper must delegate campaign metadata");
        assert_eq!(identity.resolution_note.as_deref(), Some("fixture unavailable"));
        assert_eq!(identity.witnesses.session, Some(2));
        drop(source);
        fs::remove_dir(&spool_path).unwrap();
        fs::remove_dir(&parent).unwrap();
    }

    impl CertifiedPrefoldSpoolDecoder for FixtureSpoolDecoder {
        fn decoder_id(&self) -> &'static str {
            "test.replay-app.bounded-spool"
        }

        fn decode_certified_spool(
            &self,
            request: CertifiedPrefoldSpoolRequest,
        ) -> Result<PrefoldedSpool, PrefoldDecodeError> {
            let (game_id, raw_path, snapshot_dir, source_len, source_sha256) =
                request.into_parts();
            let raw = fs::read(raw_path).map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            if raw != b"H4CRfixture-stream"
                || raw.len() as u64 != source_len
                || snapshot::sha256::to_hex(
                    &snapshot::sha256::Sha256::digest(&raw),
                ) != source_sha256
            {
                return Err(PrefoldDecodeError::new("fixture source receipt mismatch"));
            }
            let bytes = serde_json::to_vec(&serde_json::json!({
                "game_id": game_id,
                "schema_version": "1",
                "hour_index": 7,
                "date": "1936.1.1.7"
            }))
            .unwrap();
            let file_name = "000000000007.snapshot.json";
            fs::write(snapshot_dir.join(file_name), &bytes)
                .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            let entry = PrefoldedSpoolEntry::try_new(
                7,
                "1936.1.1.7",
                file_name,
                bytes.len() as u64,
                snapshot::sha256::to_hex(
                    &snapshot::sha256::Sha256::digest(&bytes),
                ),
            )?;
            PrefoldedSpool::try_new(
                game_id,
                serde_json::Value::String("1".to_string()),
                vec![entry],
            )
        }
    }

    #[test]
    fn shipped_policy_activates_only_the_explicit_bounded_capability() {
        let parent = temp_root("bounded_activation");
        let game = "bounded-product-game";
        let container_path = parent.join("compact.battleplan");
        let entries = vec![
            PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&serde_json::json!({
                    "game_id": game,
                    "schema_version": "1",
                    "snapshots": []
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/collector_raw/collector_stream.bin"),
                bytes: b"H4CRfixture-stream".to_vec(),
                role: "collector_compact_stream",
                hour_index: None,
            },
        ];
        let identity = PackIdentity {
            schema_version: Some("1".to_string()),
            ..Default::default()
        };
        fs::write(
            &container_path,
            pack(game, &entries, &identity, true).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            replay_core::open_recording(&container_path),
            Err(SourceError::Unsupported(_))
        ));

        let spool_path = parent.join("product-spool");
        let root = ProductSpoolRoot::ensure(spool_path.clone()).unwrap();
        let source = open_product_recording_at(&container_path, root, &FixtureSpoolDecoder)
            .expect("explicit bounded product activation");
        assert_eq!(source.hours(), &[7]);
        let receipt = source.prefold_receipt().expect("source receipt exposed");
        assert_eq!(receipt.decoder_id(), "test.replay-app.bounded-spool");
        assert_eq!(receipt.source_len(), 18);
        assert_eq!(source.frame(7).unwrap().date.as_deref(), Some("1936.1.1.7"));
        drop(source);
        assert!(spool_path.is_dir(), "stable product spool parent persists");
        assert_eq!(
            fs::read_dir(&spool_path).unwrap().count(),
            0,
            "per-source authenticated child must be cleaned"
        );
        fs::remove_dir(&spool_path).unwrap();
        fs::remove_dir_all(parent).unwrap();
    }
}
