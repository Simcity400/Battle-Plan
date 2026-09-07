mod publication_index {
    use super::*;

    #[derive(Clone, Copy)]
    enum WriterKind {
        Borrowed,
        Streaming { package_raw: bool },
    }

    #[derive(Clone, Copy)]
    enum Mutation {
        Missing,
        Replaced,
    }

    fn rewrite_index(file: &mut fs::File, game_id: &str, mutation: Mutation) {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut original = Vec::new();
        file.read_to_end(&mut original).unwrap();
        let archive = snapshot::zip::ZipArchive::parse(original).unwrap();
        let mut payloads = BTreeMap::new();
        for entry in archive.entries() {
            payloads.insert(entry.name.clone(), archive.read_entry(entry).unwrap());
        }
        let mut manifest: Value =
            serde_json::from_slice(&payloads.remove("manifest.json").unwrap()).unwrap();
        let path = format!("payload/{game_id}/index.json");
        assert!(
            payloads.contains_key(&path),
            "the real writer must have authored an index"
        );
        match mutation {
            Mutation::Missing => {
                payloads.remove(&path).unwrap();
                manifest["entries"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|entry| entry["path"] != path);
            }
            Mutation::Replaced => {
                let replacement =
                    br#"{"game_id":"another-campaign","schema_version":"1","snapshots":[]}"#
                        .to_vec();
                let entry = manifest["entries"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|entry| entry["path"] == path)
                    .unwrap();
                entry["bytes"] = json!(replacement.len());
                entry["sha256"] = json!(to_hex(&Sha256::digest(&replacement)));
                payloads.insert(path, replacement);
            }
        }
        manifest["manifest_digest"] = json!(to_hex(&Sha256::digest(
            &snapshot::canon::manifest_canonical_bytes(&manifest),
        )));
        // Rebuild ZIP lengths and CRCs as well as the manifest hash. This is
        // an internally authenticated artifact, not incidental byte corruption.
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (path, bytes) in payloads {
            writer.start_file(path, options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let error = Container::from_bytes(bytes.clone(), &ExpectedIdentity::default())
            .err()
            .expect("the altered index must make the container reader refuse")
            .to_string();
        match mutation {
            Mutation::Missing => assert!(error.contains("index.json"), "{error}"),
            Mutation::Replaced => assert!(error.contains("index.game_id"), "{error}"),
        }
        file.set_len(0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();
        file.sync_all().unwrap();
    }

    fn publish<F>(
        fixture: &CertifiedPackagingFixture,
        kind: WriterKind,
        raw: &[u8],
        snapshots: &[(i64, String, Vec<u8>)],
        hook: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&Path, &mut fs::File) -> Result<(), String>,
    {
        match kind {
            WriterKind::Borrowed => write_native_replay_container_with_pre_verify_hook(
                &fixture.cfg,
                raw,
                snapshots,
                &fixture.manifest,
                &fixture.manifest_path,
                &fixture.replay_path,
                hook,
            ),
            WriterKind::Streaming { .. } => {
                write_native_replay_container_streaming_with_pre_verify_hook(
                    &fixture.cfg,
                    &fixture.raw_path,
                    &fixture.capture,
                    &[],
                    Some(&fixture.artifacts),
                    Some(&fixture.fold_stats),
                    compact_contract::stream_validator::StreamIdentityMode::Legacy,
                    None,
                    &fixture.manifest,
                    &fixture.manifest_path,
                    &fixture.replay_path,
                    hook,
                )
                .map(|_| ())
            }
        }
    }

    fn refusal_preserves_prior_publication(kind: WriterKind, mutation: Mutation, tag: &str) {
        let mut fixture = certified_artifact_packaging_fixture(tag);
        let package_raw = match kind {
            WriterKind::Borrowed => true,
            WriterKind::Streaming { package_raw } => package_raw,
        };
        fixture.cfg.package_raw_stream = package_raw;
        fixture.manifest[snapshot::RAW_STREAM_PACKAGING_FIELD] =
            json!(if package_raw { "packaged" } else { "omitted" });
        fs::write(
            &fixture.manifest_path,
            serde_json::to_vec_pretty(&fixture.manifest).unwrap(),
        )
        .unwrap();
        let raw_before = portable_test_fs_retry::read(&fixture.raw_path).unwrap();
        let manifest_before = portable_test_fs_retry::read(&fixture.manifest_path).unwrap();
        let (raw, snapshots) = valid_stream_and_snapshots(&fixture.cfg.session_id);
        publish(&fixture, kind, &raw, &snapshots, |_, _| Ok(())).unwrap();
        let prior = portable_test_fs_retry::read(&fixture.replay_path).unwrap();
        open_packaged_container(&fixture.replay_path)
            .unwrap()
            .verify_all()
            .unwrap();

        let mut hook_reached = false;
        let error = publish(&fixture, kind, &raw, &snapshots, |_, file| {
            hook_reached = true;
            rewrite_index(file, &fixture.game_id, mutation);
            // The low-level helper deliberately verifies payloads. It must
            // still accept this rehashed artifact; publication owns index binding.
            snapshot::verify_replay_container_streaming_file_at_path_with_policy(
                file,
                &fixture.game_id,
                "collector_compact_stream",
                "collector_raw/collector_stream.bin",
                if package_raw {
                    snapshot::StreamEntryPolicy::Required
                } else {
                    snapshot::StreamEntryPolicy::Optional
                },
            )
            .expect("all remaining payload hashes and lengths still verify");
            Ok(())
        })
        .expect_err("the installed writer must not publish an index the viewer refuses");
        assert!(
            hook_reached,
            "the regression must reach post-pack verification"
        );
        assert!(error.contains("replay index"), "{error}");
        assert!(
            error.contains(&format!("payload/{}/index.json", fixture.game_id)),
            "{error}"
        );
        assert_eq!(
            portable_test_fs_retry::read(&fixture.replay_path).unwrap(),
            prior,
            "a rejected replacement must preserve the previous valid replay"
        );
        assert_eq!(
            portable_test_fs_retry::read(&fixture.raw_path).unwrap(),
            raw_before
        );
        assert_eq!(
            portable_test_fs_retry::read(&fixture.manifest_path).unwrap(),
            manifest_before
        );
        publish(&fixture, kind, &raw, &snapshots, |_, _| Ok(())).unwrap();
        open_packaged_container(&fixture.replay_path)
            .unwrap()
            .verify_all()
            .unwrap();
    }

    macro_rules! publication_case {
        ($name:ident, $kind:expr, $mutation:expr) => {
            #[test]
            fn $name() {
                refusal_preserves_prior_publication($kind, $mutation, stringify!($name));
            }
        };
    }

    publication_case!(
        borrowed_missing_index,
        WriterKind::Borrowed,
        Mutation::Missing
    );
    publication_case!(
        borrowed_replaced_index,
        WriterKind::Borrowed,
        Mutation::Replaced
    );
    publication_case!(
        streaming_packaged_missing_index,
        WriterKind::Streaming { package_raw: true },
        Mutation::Missing
    );
    publication_case!(
        streaming_packaged_replaced_index,
        WriterKind::Streaming { package_raw: true },
        Mutation::Replaced
    );
    publication_case!(
        streaming_omitted_missing_index,
        WriterKind::Streaming { package_raw: false },
        Mutation::Missing
    );
    publication_case!(
        streaming_omitted_replaced_index,
        WriterKind::Streaming { package_raw: false },
        Mutation::Replaced
    );
}
