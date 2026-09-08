//! The recorded native query has its own ordered roster and full-key payloads.
//! It never obtains membership from a playback directory enumeration.
use super::*;
use portable_content_manifest::native_graphics_query::NativeGraphicsQueryWitness;
pub use portable_content_manifest::native_graphics_query::{
    NativeGraphicsQueryEntry as AdmittedNativeGraphicsEntry, NativeGraphicsQuerySpec,
    NativeGraphicsQueryUnavailable,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedNativeGraphicsQuery {
    lease: ContentAdmissionLease,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedNativeGraphicsInput {
    original_query_spelling: String,
    content: UsedContentKey,
    bytes: Vec<u8>,
}

impl ResolvedNativeGraphicsInput {
    pub fn original_query_spelling(&self) -> &str {
        &self.original_query_spelling
    }
    pub fn content(&self) -> &UsedContentKey {
        &self.content
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl ContentAdmissionLease {
    pub fn native_graphics_query(
        &self,
    ) -> std::result::Result<AdmittedNativeGraphicsQuery, NativeGraphicsQueryUnavailable> {
        let witness = &self.0.native_graphics_query;
        if let Some(cause) = witness.unavailable_cause() {
            return Err(cause.clone());
        }
        let complete = witness
            .complete()
            .ok_or(NativeGraphicsQueryUnavailable::NotObserved)?;
        if complete
            .entries()
            .iter()
            .any(|entry| !self.0.native_graphics_entries.contains_key(entry.content()))
        {
            return Err(NativeGraphicsQueryUnavailable::InputUnavailable);
        }
        Ok(AdmittedNativeGraphicsQuery {
            lease: self.clone(),
        })
    }
}

impl AdmittedNativeGraphicsQuery {
    pub fn lease(&self) -> &ContentAdmissionLease {
        &self.lease
    }

    pub fn specification(&self) -> &NativeGraphicsQuerySpec {
        self.lease
            .0
            .native_graphics_query
            .complete()
            .expect("query is minted only from an immutable Complete witness")
            .specification()
    }

    pub fn entries(&self) -> &[AdmittedNativeGraphicsEntry] {
        self.lease
            .0
            .native_graphics_query
            .complete()
            .expect("query is minted only from an immutable Complete witness")
            .entries()
    }

    pub fn read_entry(
        &self,
        index: usize,
    ) -> std::result::Result<ResolvedNativeGraphicsInput, NativeGraphicsQueryUnavailable> {
        let entry = self
            .entries()
            .get(index)
            .ok_or(NativeGraphicsQueryUnavailable::BoundsExceeded)?;
        let admitted = self
            .lease
            .0
            .native_graphics_entries
            .get(entry.content())
            .ok_or(NativeGraphicsQueryUnavailable::InputUnavailable)?;
        let bytes =
            read_verified_native_graphics_payload(&admitted.snapshot_path, entry.content())?;
        Ok(ResolvedNativeGraphicsInput {
            original_query_spelling: entry.original_query_spelling().to_string(),
            content: entry.content().clone(),
            bytes,
        })
    }
}

fn read_native_graphics_payload(
    path: &Path,
) -> std::result::Result<Vec<u8>, NativeGraphicsQueryUnavailable> {
    let metadata =
        fs::metadata(path).map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)?;
    if metadata.len() > MAX_CONTENT_PAYLOAD_BYTES {
        return Err(NativeGraphicsQueryUnavailable::BoundsExceeded);
    }
    read_bounded_file(path, "native graphics query payload")
        .map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)
}

pub(super) fn read_verified_native_graphics_payload(
    path: &Path,
    key: &UsedContentKey,
) -> std::result::Result<Vec<u8>, NativeGraphicsQueryUnavailable> {
    let bytes = read_native_graphics_payload(path)?;
    if bytes.len() as u64 != key.payload_bytes || Sha256::hex(&bytes) != key.payload_sha256 {
        return Err(NativeGraphicsQueryUnavailable::ChangedDuringObservation);
    }
    Ok(bytes)
}

impl GameFileResolver {
    /// Query completeness includes mounts that emitted no payload. A caller's
    /// inventory cannot substitute for the actual selection, even when empty.
    pub(super) fn validate_native_graphics_mounts(
        &self,
        admission: &ValidatedContentAdmission,
    ) -> std::result::Result<(), NativeGraphicsQueryUnavailable> {
        let recorded = admission.ordered_layers();
        if recorded.len() != self.resolver.layers.len() {
            return Err(NativeGraphicsQueryUnavailable::InputUnavailable);
        }
        for (expected, actual) in recorded.iter().zip(&self.resolver.layers) {
            let source = &actual.source;
            if source.ordinal as u32 != expected.ordinal
                || known_content_layer_kind(&source.kind) != Some(expected.layer_kind)
                || source.source_id != expected.layer_id
                || source.descriptor_sha256 != expected.descriptor_sha256
                || source.archive_sha256 != expected.source_container_sha256
                || source.replace_path != expected.replace_paths
                || (source.archive_sha256.is_some())
                    != (expected.storage_kind == LayerStorageKind::ZipArchive)
            {
                return Err(NativeGraphicsQueryUnavailable::InputUnavailable);
            }
            checked_directory_root(&actual.root)
                .map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)?;
        }
        Ok(())
    }

    fn read_native_graphics_source(
        &self,
        original_query: &str,
        expected: &UsedContentKey,
    ) -> std::result::Result<(ResolvedGameFile, Vec<u8>), NativeGraphicsQueryUnavailable> {
        expected
            .validate()
            .map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)?;
        let layer = self
            .resolver
            .layers
            .iter()
            .find(|layer| layer.source.ordinal as u32 == expected.winning_layer_ordinal)
            .ok_or(NativeGraphicsQueryUnavailable::InputUnavailable)?;
        let (stored, path) = self
            .resolver
            .scan
            .find_file_in_layer(&layer.root, &expected.stored_game_relative_path)
            .map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)?
            .ok_or(NativeGraphicsQueryUnavailable::InputUnavailable)?;
        if stored != expected.stored_game_relative_path {
            return Err(NativeGraphicsQueryUnavailable::ChangedDuringObservation);
        }
        let bytes = read_verified_native_graphics_payload(&path, expected)?;
        let actual = ResolvedGameFile {
            logical_path: original_query.to_string(),
            source: layer.source.clone(),
            source_relative_path: stored,
            byte_len: bytes.len() as u64,
            sha256: Sha256::hex(&bytes),
            local_path: path,
        };
        validate_used_content_match(&actual, expected)
            .map_err(|_| NativeGraphicsQueryUnavailable::ChangedDuringObservation)?;
        Ok((actual, bytes))
    }

    fn native_graphics_vfs(
        &self,
        authority: &str,
    ) -> std::result::Result<
        portable_content_manifest::effective_vfs::EffectiveVfs,
        NativeGraphicsQueryUnavailable,
    > {
        use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
        let mounts = self
            .resolver
            .layers
            .iter()
            .map(|layer| {
                let source = &layer.source;
                let (storage_kind, local_source) = if source.archive_sha256.is_some() {
                    (
                        LayerStorageKind::ZipArchive,
                        layer
                            .original_archive
                            .clone()
                            .ok_or(NativeGraphicsQueryUnavailable::InputUnavailable)?,
                    )
                } else {
                    (LayerStorageKind::Directory, layer.root.clone())
                };
                Ok(EffectiveVfsLayerMount {
                    ordinal: source.ordinal as u32,
                    layer_kind: known_content_layer_kind(&source.kind)
                        .ok_or(NativeGraphicsQueryUnavailable::InputUnavailable)?,
                    layer_id: source.source_id.clone(),
                    storage_kind,
                    local_source,
                    source_container_sha256: source.archive_sha256.clone(),
                    replace_paths: source.replace_path.clone(),
                })
            })
            .collect::<std::result::Result<Vec<_>, NativeGraphicsQueryUnavailable>>()?;
        EffectiveVfs::new(authority, mounts)
            .map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)
    }

    /// Re-observation proves that the local query scope still agrees with the
    /// recording. Its rows never fill an omitted or unavailable recorded query.
    pub(super) fn revalidate_native_graphics_scope(
        &self,
        witness: &NativeGraphicsQueryWitness,
        authority: &str,
    ) -> std::result::Result<(), NativeGraphicsQueryUnavailable> {
        let Some(complete) = witness.complete() else {
            return Ok(());
        };
        let vfs = self.native_graphics_vfs(authority)?;
        let observed = vfs
            .observe_native_graphics_query(complete.specification().executable_sha256())
            .map_err(|_| NativeGraphicsQueryUnavailable::InputUnavailable)?;
        if let Some(cause) = observed.witness().unavailable_cause() {
            return Err(*cause);
        }
        if observed.witness() != witness {
            return Err(NativeGraphicsQueryUnavailable::ChangedDuringObservation);
        }
        Ok(())
    }

    /// Every original spelling is resolved, even when several rows share a key.
    /// An inventory supplied by a caller cannot stand in for these local reads.
    pub(super) fn revalidate_native_graphics_query(
        &self,
        witness: &NativeGraphicsQueryWitness,
        authority: &str,
    ) -> Result<BTreeMap<UsedContentKey, ResolvedGameFile>> {
        self.revalidate_native_graphics_scope(witness, authority)?;
        let mut required = BTreeMap::new();
        if let Some(complete) = witness.complete() {
            for entry in complete.entries() {
                let (actual, _) = self.read_native_graphics_source(
                    entry.original_query_spelling(),
                    entry.content(),
                )?;
                required.insert(entry.content().clone(), actual);
            }
        }
        Ok(required)
    }
}

/// Query-only payloads use a separate directory and a full-key address, so
/// equal normalized paths or payload hashes cannot merge distinct identities.
pub(super) fn native_graphics_snapshot_path(root: &Path, key: &UsedContentKey) -> PathBuf {
    root.join("native-graphics-query").join(Sha256::hex(
        &serde_json::to_vec(key).expect("UsedContentKey contains only serializable scalar fields"),
    ))
}

pub(super) fn snapshot_native_graphics_entries(
    root: &Path,
    winners: &BTreeMap<String, AdmittedContent>,
    required: &BTreeMap<UsedContentKey, ResolvedGameFile>,
) -> Result<BTreeMap<UsedContentKey, AdmittedContent>> {
    let mut entries = BTreeMap::new();
    for (key, actual) in required {
        let snapshot_path = if let Some(winner) = winners
            .get(&key.normalized_game_relative_path)
            .filter(|winner| &winner.key == key)
        {
            read_verified_native_graphics_payload(&winner.snapshot_path, key)?;
            winner.snapshot_path.clone()
        } else {
            let bytes = read_verified_native_graphics_payload(&actual.local_path, key)?;
            let path = native_graphics_snapshot_path(root, key);
            fs::create_dir_all(path.parent().ok_or("query snapshot has no parent")?)?;
            fs::write(&path, bytes)?;
            let mut permissions = fs::metadata(&path)?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions)?;
            path
        };
        entries.insert(
            key.clone(),
            AdmittedContent {
                key: key.clone(),
                resolved: actual.clone(),
                snapshot_path,
            },
        );
    }
    Ok(entries)
}

pub(super) fn rebase_native_graphics_entries(
    root: &Path,
    winners: &BTreeMap<String, AdmittedContent>,
    entries: BTreeMap<UsedContentKey, AdmittedContent>,
) -> BTreeMap<UsedContentKey, AdmittedContent> {
    entries
        .into_iter()
        .map(|(key, entry)| {
            let snapshot_path = winners
                .get(&key.normalized_game_relative_path)
                .filter(|winner| winner.key == key)
                .map(|winner| winner.snapshot_path.clone())
                .unwrap_or_else(|| native_graphics_snapshot_path(root, &key));
            (
                key,
                AdmittedContent {
                    snapshot_path,
                    ..entry
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_test_dir, write_modvfs_fixture as write};
    use portable_content_manifest::native_graphics_query::NATIVE_GRAPHICS_EXECUTABLE_SHA256;

    fn environment(manifest: &ContentManifest) -> PlaybackEnvironment {
        PlaybackEnvironment {
            source_game: manifest.source_game.clone(),
            source_profile: manifest.source_profile.clone(),
            dlc_selection: manifest.dlc_selection.clone(),
            localisation_language: manifest.localisation_language.clone(),
            settings: manifest.required_settings.clone(),
        }
    }

    fn recorded_query(raw: &GameFileResolver, label: &str) -> ContentManifest {
        let authority = Sha256::hex(format!("{}:{label}", std::process::id()).as_bytes());
        let vfs = raw.native_graphics_vfs(&authority).unwrap();
        let witness = vfs
            .observe_native_graphics_query(NATIVE_GRAPHICS_EXECUTABLE_SHA256)
            .unwrap()
            .seal(&vfs)
            .unwrap()
            .into_witness();
        assert!(
            witness.complete().is_some(),
            "{label}: {:?}",
            witness.unavailable_cause()
        );
        let mut manifest = ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
            product_modset_sha256: "99".repeat(32),
            source_game: SourceGameIdentity {
                version: "1.19.2.0".into(),
                build: "a729".into(),
                executable_sha256: NATIVE_GRAPHICS_EXECUTABLE_SHA256.into(),
                build_profile: label.into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: label.into(),
                playset_id: label.into(),
                selection_sha256: "00".repeat(32),
            },
            effective_vfs_manifest_sha256: authority,
            ordered_layers: raw
                .active_sources()
                .into_iter()
                .map(|source| replay_core::RecordedContentLayer {
                    ordinal: source.ordinal as u32,
                    layer_kind: content_layer_kind(&source.kind),
                    layer_id: source.source_id,
                    display_name: label.into(),
                    descriptor_sha256: source.descriptor_sha256,
                    storage_kind: if source.archive_sha256.is_some() {
                        LayerStorageKind::ZipArchive
                    } else {
                        LayerStorageKind::Directory
                    },
                    source_container_sha256: source.archive_sha256,
                    replace_paths: source.replace_path,
                    required_for_playback: true,
                })
                .collect(),
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content: Vec::new(),
            witnessed_absent_content: Vec::new(),
            witnessed_content_namespaces: Vec::new(),
            native_graphics_query: witness,
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    fn admission(raw: &GameFileResolver, manifest: &ContentManifest) -> ValidatedContentAdmission {
        let inventory = raw
            .playback_inventory(manifest, environment(manifest))
            .unwrap();
        manifest.admit_playback(&inventory).unwrap()
    }

    fn directory_fixture(
        label: &str,
        files: &[(&str, &[u8])],
    ) -> (
        PathBuf,
        GameFileResolver,
        ContentManifest,
        ValidatedContentAdmission,
    ) {
        let root = temp_test_dir(label);
        fs::create_dir_all(&root).unwrap();
        for (path, bytes) in files {
            write(&root, path, bytes);
        }
        let raw = GameFileResolver::vanilla(root.clone()).unwrap();
        let manifest = recorded_query(&raw, label);
        let admitted = admission(&raw, &manifest);
        (root, raw, manifest, admitted)
    }

    fn cache_root(raw: &GameFileResolver) -> PathBuf {
        raw.admission
            .as_ref()
            .unwrap()
            .snapshot_root
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn admitted_query_keeps_native_file_before_recursion_order_and_exact_suffix() {
        let (root, raw, manifest, proof) = directory_fixture(
            "native-query-order",
            &[
                ("interface/z.gfx", b"root"),
                ("interface/a/first.gfx", b"first"),
                ("interface/b/second.gfx", b"second"),
                ("interface/ignored.gui", b"gui"),
                ("interface/ignored.GFX", b"uppercase suffix"),
            ],
        );
        let admitted = raw.with_content_admission(&proof).unwrap();
        let cache = cache_root(&admitted);
        let lease = admitted.content_admission_lease().unwrap();
        let query = lease.native_graphics_query().unwrap();
        assert_eq!(query.lease(), &lease);
        assert!(Arc::ptr_eq(&query.lease().0, &lease.0));
        assert_eq!(
            query.lease().manifest_digest_sha256(),
            manifest.manifest_digest_sha256.as_deref()
        );
        assert_eq!(query.specification().directory(), "interface");
        assert_eq!(query.specification().suffix(), ".gfx");
        assert_eq!(query.specification().contains(), None);
        assert_eq!(query.specification().depth(), -1);
        assert_eq!(
            query
                .entries()
                .iter()
                .map(|e| e.original_query_spelling())
                .collect::<Vec<_>>(),
            [
                "interface/z.gfx",
                "interface/a/first.gfx",
                "interface/b/second.gfx"
            ]
        );
        for (index, expected) in [b"root".as_slice(), b"first", b"second"]
            .into_iter()
            .enumerate()
        {
            let read = query.read_entry(index).unwrap();
            assert_eq!(read.content(), query.entries()[index].content());
            assert_eq!(
                read.original_query_spelling(),
                query.entries()[index].original_query_spelling()
            );
            assert_eq!(read.bytes(), expected);
        }
        assert_eq!(
            query.read_entry(3).unwrap_err(),
            NativeGraphicsQueryUnavailable::BoundsExceeded
        );
        drop((query, lease, admitted));
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn query_only_content_rejects_conflicting_witnessed_absence_on_every_admission_route() {
        for (label, present, absent) in [
            (
                "native-query-only-absence",
                "interface/a.gfx",
                "interface/a.gfx",
            ),
            (
                "native-query-only-absence-alias",
                "interface/i.gfx",
                "interface/\u{0131}.gfx",
            ),
        ] {
            let (root, raw, mut manifest, _) = directory_fixture(label, &[(present, b"present")]);
            assert!(manifest.required_content.is_empty());
            let entries = manifest.native_graphics_query.complete().unwrap().entries();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].content().normalized_game_relative_path, present);
            assert_eq!(
                windows_collision_key(present).unwrap(),
                windows_collision_key(absent).unwrap()
            );
            assert_eq!(normalize_game_relative_path(absent).unwrap(), absent);
            let inventory = raw
                .playback_inventory(&manifest, environment(&manifest))
                .unwrap();
            assert!(manifest.admit_playback(&inventory).is_ok());
            manifest.witnessed_absent_content = vec![absent.into()];
            manifest.manifest_digest_sha256 = None;
            let expected =
                format!("witnessed absent content conflicts with another content path: {absent}");
            assert_eq!(
                manifest.clone().with_computed_digest().unwrap_err(),
                expected
            );
            let wire = serde_json::to_vec(&manifest).unwrap();
            assert_eq!(ContentManifest::decode_json(&wire).unwrap_err(), expected);
            let report = manifest.admit_playback(&inventory).unwrap_err();
            assert!(!report.compatible);
            assert_eq!(report.issues.len(), 1);
            assert_eq!(
                report.issues[0].code,
                portable_content_manifest::ContentIssueCode::InvalidManifest
            );
            assert_eq!(report.issues[0].message, expected);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn unavailable_query_never_mints_empty_or_winner_fallback() {
        let (root, raw, mut manifest, _) = directory_fixture(
            "native-query-unavailable",
            &[("interface/a.gfx", b"authored")],
        );
        for cause in [
            NativeGraphicsQueryUnavailable::NotObserved,
            NativeGraphicsQueryUnavailable::UnsupportedBuild,
            NativeGraphicsQueryUnavailable::InputUnavailable,
        ] {
            manifest.native_graphics_query = NativeGraphicsQueryWitness::unavailable(cause);
            manifest = manifest.with_computed_digest().unwrap();
            let proof = admission(&raw, &manifest);
            let admitted = raw.clone().with_content_admission(&proof).unwrap();
            assert_eq!(
                admitted
                    .content_admission_lease()
                    .unwrap()
                    .native_graphics_query()
                    .unwrap_err(),
                cause
            );
            drop(admitted);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn complete_empty_query_checks_all_mounts_and_replace_scope() {
        let root = temp_test_dir("native-query-empty-mounts");
        let base = root.join("base");
        let first = root.join("first");
        let second = root.join("second");
        for path in [&base, &first, &second] {
            fs::create_dir_all(path).unwrap();
        }
        let raw =
            GameFileResolver::with_mod_paths(base.clone(), vec![first.clone(), second.clone()])
                .unwrap();
        let manifest = recorded_query(&raw, "native-query-empty-mounts");
        let proof = admission(&raw, &manifest);
        let admitted = raw.clone().with_content_admission(&proof).unwrap();
        assert!(admitted
            .content_admission_lease()
            .unwrap()
            .native_graphics_query()
            .unwrap()
            .entries()
            .is_empty());
        let missing = GameFileResolver::with_mod_paths(base.clone(), vec![first.clone()]).unwrap();
        assert!(missing.with_content_admission(&proof).is_err());
        let reordered = GameFileResolver::with_mod_paths(base, vec![second, first]).unwrap();
        assert!(reordered.with_content_admission(&proof).is_err());
        let mut changed_replace = raw.clone();
        changed_replace.resolver.layers[1]
            .source
            .replace_path
            .push("interface".into());
        changed_replace.resolver.layers[1]
            .replace_paths
            .insert("interface".into());
        assert_eq!(
            changed_replace
                .validate_native_graphics_mounts(&proof)
                .unwrap_err(),
            NativeGraphicsQueryUnavailable::InputUnavailable
        );
        fs::remove_dir_all(&raw.resolver.layers[2].root).unwrap();
        assert_eq!(
            raw.validate_native_graphics_mounts(&proof).unwrap_err(),
            NativeGraphicsQueryUnavailable::InputUnavailable
        );
        drop(admitted);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lower_spelling_change_refuses_even_with_unchanged_upper_winner_and_cache() {
        let root = temp_test_dir("native-query-lower-spelling");
        let base = root.join("base");
        let upper = root.join("upper");
        write(&base, "interface/a.gfx", b"lower");
        write(&upper, "interface/a.gfx", b"winner");
        let raw = GameFileResolver::with_mod_paths(base.clone(), vec![upper.clone()]).unwrap();
        let manifest = recorded_query(&raw, "native-query-lower-spelling");
        let proof = admission(&raw, &manifest);
        let admitted = raw.clone().with_content_admission(&proof).unwrap();
        let cache = cache_root(&admitted);
        assert_eq!(
            admitted
                .content_admission_lease()
                .unwrap()
                .native_graphics_query()
                .unwrap()
                .entries()
                .len(),
            1
        );
        fs::rename(
            base.join("interface/a.gfx"),
            base.join("interface/renaming"),
        )
        .unwrap();
        fs::rename(
            base.join("interface/renaming"),
            base.join("interface/A.gfx"),
        )
        .unwrap();
        let changed = GameFileResolver::with_mod_paths(base, vec![upper]).unwrap();
        assert_eq!(
            changed
                .read_required_bytes("interface/a.gfx", "same winner")
                .unwrap(),
            b"winner"
        );
        assert_eq!(
            changed
                .revalidate_native_graphics_scope(
                    &manifest.native_graphics_query,
                    &manifest.effective_vfs_manifest_sha256
                )
                .unwrap_err(),
            NativeGraphicsQueryUnavailable::ChangedDuringObservation
        );
        assert!(changed
            .playback_inventory(&manifest, environment(&manifest))
            .is_err());
        assert!(changed.with_content_admission(&proof).is_err());
        drop(admitted);
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_root_case_alias_admits_original_query_and_replacement_stays_unavailable() {
        let (root, mut raw, manifest, proof) = directory_fixture(
            "native-query-root-alias",
            &[("Interface/a.gfx", b"authored")],
        );
        let admitted = raw.clone().with_content_admission(&proof).unwrap();
        let cache = cache_root(&admitted);
        let query = admitted
            .content_admission_lease()
            .unwrap()
            .native_graphics_query()
            .unwrap();
        assert_eq!(query.entries().len(), 1);
        assert_eq!(
            query.entries()[0].original_query_spelling(),
            "interface/a.gfx"
        );
        assert_eq!(
            query.entries()[0].content().stored_game_relative_path,
            "Interface/a.gfx"
        );
        assert_eq!(query.read_entry(0).unwrap().bytes(), b"authored");
        raw.resolver.layers[0]
            .source
            .replace_path
            .push("interface".into());
        assert_eq!(
            raw.revalidate_native_graphics_scope(
                &manifest.native_graphics_query,
                &manifest.effective_vfs_manifest_sha256
            )
            .unwrap_err(),
            NativeGraphicsQueryUnavailable::UnprovenReplacement
        );
        raw.resolver.layers[0].source.replace_path.clear();
        fs::rename(root.join("Interface"), root.join("renaming")).unwrap();
        fs::rename(root.join("renaming"), root.join("interface")).unwrap();
        assert_eq!(
            raw.revalidate_native_graphics_scope(
                &manifest.native_graphics_query,
                &manifest.effective_vfs_manifest_sha256
            )
            .unwrap_err(),
            NativeGraphicsQueryUnavailable::ChangedDuringObservation
        );
        drop((query, admitted));
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn query_retains_exact_lease_and_immutable_snapshot_until_last_owner() {
        let (root, raw, _, proof) = directory_fixture(
            "native-query-retention",
            &[("interface/a.gfx", b"original")],
        );
        let admitted = raw.clone().with_content_admission(&proof).unwrap();
        let cache = cache_root(&admitted);
        let second = raw.with_content_admission(&proof).unwrap();
        let (isolated, content_root) = admitted.isolated_content_snapshot().unwrap();
        let outer = content_root.parent().unwrap().to_path_buf();
        let lease = isolated.content_admission_lease().unwrap();
        let query = lease.native_graphics_query().unwrap();
        assert!(Arc::ptr_eq(&query.lease().0, &lease.0));
        assert_ne!(query.lease(), &second.content_admission_lease().unwrap());
        assert_eq!(
            query.lease().manifest_digest_sha256(),
            second
                .content_admission_lease()
                .unwrap()
                .manifest_digest_sha256()
        );
        write(&root, "interface/a.gfx", b"live source changed");
        drop((isolated, lease, second));
        assert!(outer.is_dir());
        assert_eq!(query.read_entry(0).unwrap().bytes(), b"original");
        drop(query);
        assert!(!outer.exists());
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn query_cache_reuses_verified_bytes_and_refuses_tampered_payload_or_manifest() {
        let (root, raw, _, proof) =
            directory_fixture("native-query-cache", &[("interface/a.gfx", b"original")]);
        let first = raw.clone().with_content_admission(&proof).unwrap();
        let cache = cache_root(&first);
        let second = raw.clone().with_content_admission(&proof).unwrap();
        assert_eq!(cache_root(&second), cache);
        let key = proof
            .native_graphics_query()
            .required_content()
            .pop()
            .unwrap();
        let snapshot = native_graphics_snapshot_path(&cache, &key);
        crate::test_support::overwrite_readonly(&snapshot, b"tampered");
        assert_eq!(
            first
                .content_admission_lease()
                .unwrap()
                .native_graphics_query()
                .unwrap()
                .read_entry(0)
                .unwrap_err(),
            NativeGraphicsQueryUnavailable::ChangedDuringObservation
        );
        let fresh = raw.clone().with_content_admission(&proof).unwrap();
        assert!(!fresh.admission.as_ref().unwrap().retain_snapshot_root);
        let staging = fresh
            .admission
            .as_ref()
            .unwrap()
            .snapshot_cleanup_root
            .clone()
            .unwrap();
        assert_eq!(
            fresh
                .content_admission_lease()
                .unwrap()
                .native_graphics_query()
                .unwrap()
                .read_entry(0)
                .unwrap()
                .bytes(),
            b"original"
        );
        drop(fresh);
        assert!(!staging.exists());
        crate::test_support::overwrite_readonly(&snapshot, b"original");
        let manifest_path = cache.join(ADMISSION_SNAPSHOT_MANIFEST);
        let mut wire: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        wire["native_graphics_content"][0]["stored_game_relative_path"] = "interface/A.gfx".into();
        fs::write(&manifest_path, serde_json::to_vec(&wire).unwrap()).unwrap();
        let rebuilt = raw.with_content_admission(&proof).unwrap();
        assert!(!rebuilt.admission.as_ref().unwrap().retain_snapshot_root);
        assert_eq!(
            rebuilt
                .content_admission_lease()
                .unwrap()
                .native_graphics_query()
                .unwrap()
                .read_entry(0)
                .unwrap()
                .bytes(),
            b"original"
        );
        drop((rebuilt, first, second));
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }

    fn include_query_winner(
        mut manifest: ContentManifest,
        winner: &UsedContentKey,
    ) -> ContentManifest {
        manifest
            .required_content
            .push(replay_core::RequiredContent {
                content_key: "native-query-winner".into(),
                kind: replay_core::RequiredContentKind::Definition,
                content: winner.clone(),
                observation_count: 1,
                consumers: vec!["native-query-test".into()],
            });
        manifest.with_computed_digest().unwrap()
    }

    #[test]
    fn admitted_directory_and_zip_payloads_with_same_folded_path_remain_distinct() {
        use std::io::Write as _;
        let root = temp_test_dir("native-query-directory-zip");
        let base = root.join("base");
        let profile = root.join("profile");
        write(&base, "interface/a.gfx", b"lower directory");
        write(
            &profile,
            "mod/upper.mod",
            b"name=\"Upper\"\narchive=\"mod/upper.zip\"\n",
        );
        write(
            &profile,
            "dlc_load.json",
            br#"{"enabled_mods":["mod/upper.mod"],"disabled_dlcs":[]}"#,
        );
        let mut zip = zip::ZipWriter::new(File::create(profile.join("mod/upper.zip")).unwrap());
        zip.start_file("interface/A.gfx", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"upper archive").unwrap();
        zip.finish().unwrap();
        let raw =
            GameFileResolver::from_recorded_content_order(base, profile, &["mod/upper.mod".into()])
                .unwrap();
        let manifest = recorded_query(&raw, "native-query-directory-zip");
        let entries = manifest.native_graphics_query.complete().unwrap().entries();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.original_query_spelling())
                .collect::<Vec<_>>(),
            ["interface/A.gfx", "interface/a.gfx"]
        );
        let upper = entries[0].content().clone();
        let lower = entries[1].content().clone();
        assert_eq!(upper.winning_layer_ordinal, 1);
        assert_eq!(lower.winning_layer_ordinal, 0);
        assert_eq!(
            lower.normalized_game_relative_path,
            upper.normalized_game_relative_path
        );
        assert_ne!(lower.payload_sha256, upper.payload_sha256);
        let manifest = include_query_winner(manifest, &upper);
        let proof = admission(&raw, &manifest);
        let admitted = raw.clone().with_content_admission(&proof).unwrap();
        let cache = cache_root(&admitted);
        let second = raw.with_content_admission(&proof).unwrap();
        assert_eq!(cache_root(&second), cache);
        let lease = admitted.content_admission_lease().unwrap();
        let query = lease.native_graphics_query().unwrap();
        assert_eq!(lease.0.native_graphics_entries.len(), 2);
        assert_eq!(query.read_entry(0).unwrap().content(), &upper);
        assert_eq!(query.read_entry(0).unwrap().bytes(), b"upper archive");
        assert_eq!(query.read_entry(1).unwrap().content(), &lower);
        assert_eq!(query.read_entry(1).unwrap().bytes(), b"lower directory");
        assert_eq!(
            lease.0.native_graphics_entries[&upper].snapshot_path,
            lease.0.entries[&upper.normalized_game_relative_path].snapshot_path
        );
        assert_ne!(
            lease.0.native_graphics_entries[&upper].snapshot_path,
            lease.0.native_graphics_entries[&lower].snapshot_path
        );
        assert!(
            lease.read_used_content(&lower).is_err(),
            "winner namespace must keep its full-key check"
        );
        let (isolated, _) = admitted.isolated_content_snapshot().unwrap();
        let isolated_query = isolated
            .content_admission_lease()
            .unwrap()
            .native_graphics_query()
            .unwrap();
        assert_eq!(
            isolated_query.read_entry(0).unwrap().bytes(),
            b"upper archive"
        );
        assert_eq!(
            isolated_query.read_entry(1).unwrap().bytes(),
            b"lower directory"
        );
        drop((isolated_query, isolated, query, lease, second));
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn admitted_multiple_spellings_keep_multiplicity_when_payload_identity_is_shared() {
        let root = temp_test_dir("native-query-multiplicity");
        let base = root.join("base");
        let upper = root.join("upper");
        write(&base, "interface/a.gfx", b"lower");
        write(&upper, "interface/A.gfx", b"shared payload");
        let raw = GameFileResolver::with_mod_paths(base, vec![upper]).unwrap();
        let manifest = recorded_query(&raw, "native-query-multiplicity");
        let entries = manifest.native_graphics_query.complete().unwrap().entries();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.original_query_spelling())
                .collect::<Vec<_>>(),
            ["interface/A.gfx", "interface/a.gfx"]
        );
        assert_eq!(entries[0].content(), entries[1].content());
        let key = entries[0].content().clone();
        assert_eq!(key.winning_layer_ordinal, 1);
        let manifest = include_query_winner(manifest, &key);
        let proof = admission(&raw, &manifest);
        let admitted = raw.with_content_admission(&proof).unwrap();
        let cache = cache_root(&admitted);
        let lease = admitted.content_admission_lease().unwrap();
        let query = lease.native_graphics_query().unwrap();
        assert_eq!(lease.0.native_graphics_entries.len(), 1);
        assert_eq!(query.entries().len(), 2);
        assert_eq!(
            query.entries()[0].original_query_spelling(),
            "interface/A.gfx"
        );
        assert_eq!(
            query.entries()[1].original_query_spelling(),
            "interface/a.gfx"
        );
        assert_eq!(
            query.read_entry(0).unwrap().content(),
            query.read_entry(1).unwrap().content()
        );
        assert_eq!(query.read_entry(0).unwrap().bytes(), b"shared payload");
        assert_eq!(query.read_entry(1).unwrap().bytes(), b"shared payload");
        drop((query, lease, admitted));
        remove_tree_allow_readonly(&cache);
        fs::remove_dir_all(root).unwrap();
    }
}
