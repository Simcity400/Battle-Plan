use serde_json::{json, Value};
use snapshot::container::{Container, ContainerError, ExpectedIdentity};
use snapshot::{
    pack, pack_borrowed, pack_borrowed_to_writer, pack_mixed_to_writer, pack_with,
    BorrowedPackEntry, FileBackedPackEntry, PackEntry, PackIdentity, PackOptions, Store,
};

const GAME: &str = "campaign";
const BUILD: &str = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120";
const MODSET: &str = "abc123def456";

fn fixture_entries() -> Vec<PackEntry> {
    let snap_a = format!(
        r#"{{"schema_version":"1","game_id":"{GAME}","date":"1940.9.10.13","hour_index":17000461,"provenance":{{"save":{{"captured":true}},"memory":null,"mod":{{"modset_id":"{MODSET}"}}}},"countries":{{"GER":{{"politics":{{"stability":0.0,"war_support":null}}}}}},"states":{{"1":{{"owner":"GER","buildings":{{"infrastructure":3}}}}}}}}"#
    );
    let snap_b = format!(
        r#"{{"schema_version":"1","game_id":"{GAME}","date":"1946.5.18.12","hour_index":17050260,"provenance":{{"save":{{"captured":true}},"memory":null}},"countries":{{"GER":{{}}}},"states":{{}}}}"#
    );
    let index = format!(
        r#"{{"game_id":"{GAME}","schema_version":"1","snapshots":[{{"hour_index":17000461,"date":"1940.9.10.13","file":"000017000461.snapshot.json"}},{{"hour_index":17050260,"date":"1946.5.18.12","file":"s0003.000017050260.snapshot.json"}}]}}"#
    );
    let timeline = format!(r#"{{"game_id":"{GAME}","schema_version":"1","events":[]}}"#);

    vec![
        PackEntry {
            path: format!("payload/{GAME}/index.json"),
            bytes: index.into_bytes(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{GAME}/000017000461.snapshot.json"),
            bytes: snap_a.into_bytes(),
            role: "snapshot",
            hour_index: Some(17000461),
        },
        PackEntry {
            path: format!("payload/{GAME}/s0003.000017050260.snapshot.json"),
            bytes: snap_b.into_bytes(),
            role: "snapshot",
            hour_index: Some(17050260),
        },
        PackEntry {
            path: format!("payload/{GAME}/timeline.json"),
            bytes: timeline.into_bytes(),
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{GAME}/collector_raw/collector_frames.jsonl"),
            bytes: br#"{"type":"domain_delta","sequence":1,"domain":"divisions"}
{"type":"gap","sequence":2,"domain":"land_combats"}
"#
            .to_vec(),
            role: "collector_raw_stream",
            hour_index: None,
        },
    ]
}

fn fixture_identity() -> PackIdentity {
    PackIdentity {
        schema_version: Some("1".into()),
        build_exe_sha256: Some(BUILD.into()),
        build_pinned: true,
        mod_identity: Some(json!({ "modset_id": MODSET, "load_order": ["test.mod"] })),
        created_utc: Some("2026-06-26T21:30:00Z".into()),
        content_manifest: None,
    }
}

fn build_container(deflate: bool) -> Vec<u8> {
    pack(GAME, &fixture_entries(), &fixture_identity(), deflate).expect("pack")
}

#[test]
fn borrowed_pack_matches_owned_pack_bytes() {
    let entries = fixture_entries();
    let borrowed = entries
        .iter()
        .map(|entry| BorrowedPackEntry {
            path: entry.path.clone(),
            bytes: &entry.bytes,
            role: entry.role,
            hour_index: entry.hour_index,
        })
        .collect::<Vec<_>>();
    let identity = fixture_identity();

    assert_eq!(
        pack(GAME, &entries, &identity, true).expect("owned pack"),
        pack_borrowed(GAME, &borrowed, &identity, true).expect("borrowed pack")
    );
}

#[test]
fn borrowed_writer_pack_matches_borrowed_pack_bytes() {
    let entries = fixture_entries();
    let borrowed = entries
        .iter()
        .map(|entry| BorrowedPackEntry {
            path: entry.path.clone(),
            bytes: &entry.bytes,
            role: entry.role,
            hour_index: entry.hour_index,
        })
        .collect::<Vec<_>>();
    let identity = fixture_identity();
    let mut streamed = Vec::new();
    pack_borrowed_to_writer(GAME, &borrowed, &identity, true, &mut streamed)
        .expect("streaming borrowed pack");

    assert_eq!(
        pack_borrowed(GAME, &borrowed, &identity, true).expect("borrowed pack"),
        streamed
    );
}

#[test]
fn mixed_pack_file_backed_entry_roundtrips_exact_bytes() {
    let entries = fixture_entries();
    let file_backed = entries
        .iter()
        .find(|entry| entry.path.ends_with("000017000461.snapshot.json"))
        .expect("file-backed fixture snapshot");
    let dir = std::env::temp_dir().join(format!("hoi4replay_mixed_pack_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create mixed pack temp dir");
    let file_path = dir.join("snapshot.json");
    std::fs::write(&file_path, &file_backed.bytes).expect("write file-backed snapshot");
    let borrowed = entries
        .iter()
        .filter(|entry| entry.path != file_backed.path)
        .map(|entry| BorrowedPackEntry {
            path: entry.path.clone(),
            bytes: &entry.bytes,
            role: entry.role,
            hour_index: entry.hour_index,
        })
        .collect::<Vec<_>>();
    let file_entries = vec![FileBackedPackEntry {
        path: file_backed.path.clone(),
        file_path: file_path.as_path(),
        role: file_backed.role,
        hour_index: file_backed.hour_index,
    }];
    let mut packed = Vec::new();
    pack_mixed_to_writer(
        GAME,
        &borrowed,
        &file_entries,
        &fixture_identity(),
        true,
        &mut packed,
    )
    .expect("mixed pack");

    let container = Container::from_bytes(packed, &ExpectedIdentity::default()).expect("open");
    container.verify_all().expect("verify");
    assert_eq!(
        container.read_entry_bytes(&file_backed.path).unwrap(),
        file_backed.bytes
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn roundtrip_byte_identical_snapshots() {
    for deflate in [false, true] {
        let bytes = build_container(deflate);
        let c = Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("open container");

        let originals = fixture_entries();
        for e in &originals {
            if e.role == "snapshot" {
                let file = e.path.rsplit('/').next().unwrap();
                let _ = file;
            }
        }

        let idx = c.index();
        let files: Vec<&str> = idx.snapshots.iter().map(|s| s.file.as_str()).collect();
        assert!(files.contains(&"000017000461.snapshot.json"));
        assert!(
            files.contains(&"s0003.000017050260.snapshot.json"),
            "rollback file kept"
        );

        let snap_b = c.load(17050260).expect("load rollback snapshot via index");
        assert_eq!(snap_b.hour_index, Some(17050260));

        let snap_a = c.load(17000461).expect("load snapshot a");
        assert_eq!(
            snap_a.country_path("GER", "politics.stability"),
            Some(&json!(0.0)),
            "measured zero preserved"
        );
        assert_eq!(
            snap_a.country_path("GER", "politics.war_support"),
            Some(&Value::Null),
            "captured-null preserved (NOT collapsed to absent)"
        );
        assert_eq!(
            snap_a.country_path("GER", "politics.command_power"),
            None,
            "never-captured stays absent (NOT a 0)"
        );
    }
}

#[test]
fn trichotomy_survives_at_domain_level_not_just_header() {
    let raw = format!(
        r#"{{"schema_version":"1","game_id":"{GAME}","hour_index":17000461,"provenance":{{"save":{{"captured":true}},"memory":null,"mod":{{"modset_id":"{MODSET}"}}}},"countries":{{"GER":{{"economy":{{"factories":{{"civ":0,"mil":null}}}}}}}},"states":{{}}}}"#
    );
    let raw_bytes = raw.clone().into_bytes();
    let entries = vec![
        PackEntry {
            path: format!("payload/{GAME}/index.json"),
            bytes: format!(
                r#"{{"game_id":"{GAME}","schema_version":"1","snapshots":[{{"hour_index":17000461,"date":null,"file":"000017000461.snapshot.json"}}]}}"#
            )
            .into_bytes(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{GAME}/000017000461.snapshot.json"),
            bytes: raw_bytes.clone(),
            role: "snapshot",
            hour_index: Some(17000461),
        },
    ];
    let bytes = pack(GAME, &entries, &fixture_identity(), false).expect("pack");
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("open");
    let snap = c.load(17000461).expect("load");

    assert_eq!(
        snap.country_path("GER", "economy.factories.civ"),
        Some(&json!(0))
    );
    assert_eq!(
        snap.country_path("GER", "economy.factories.mil"),
        Some(&Value::Null)
    );
    assert_eq!(snap.country_path("GER", "economy.factories.dockyard"), None);

    let want = snapshot::sha256::Sha256::hex(&raw_bytes);
    let got = c
        .manifest()
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|es| {
            es.iter().find(|e| {
                e.get("path").and_then(Value::as_str)
                    == Some(&format!("payload/{GAME}/000017000461.snapshot.json"))
            })
        })
        .and_then(|e| e.get("sha256").and_then(Value::as_str))
        .unwrap();
    assert_eq!(
        got, want,
        "served snapshot bytes are byte-identical to source"
    );
}

#[test]
fn byte_identical_via_store_interface() {
    let bytes = build_container(false);
    let path = write_temp(&bytes, "rt.battleplan");
    let store = Store::open(&path);
    assert!(store.container().is_some(), "opened as a container");

    assert_eq!(store.games().unwrap(), vec![GAME.to_string()]);
    let idx = store.index(GAME).expect("index via Store");
    assert_eq!(idx.snapshots.len(), 2);

    let snap = store
        .load_file(GAME, "s0003.000017050260.snapshot.json")
        .expect("load_file via Store");
    assert_eq!(snap.hour_index, Some(17050260));

    let snap_a = store.load(GAME, 17000461).expect("load via Store");
    assert_eq!(
        snap_a.country_path("GER", "politics.war_support"),
        Some(&Value::Null)
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn container_snapshot_duplicate_members_are_rejected_recursively() {
    for (case, snapshot_bytes, duplicate_key) in [
        (
            "duplicate_session",
            br#"{"game_id":"campaign","hour_index":17050260,"session":{"active_dlcs":["Wrong DLC"]},"session":{}}"#.as_slice(),
            "session",
        ),
        (
            "duplicate_nested_active_dlcs",
            br#"{"game_id":"campaign","hour_index":17050260,"session":{"active_dlcs":["Wrong DLC"],"active_dlcs":[]}}"#.as_slice(),
            "active_dlcs",
        ),
    ] {
        let mut entries = fixture_entries();
        entries
            .iter_mut()
            .find(|entry| entry.path.ends_with("s0003.000017050260.snapshot.json"))
            .expect("second snapshot fixture")
            .bytes = snapshot_bytes.to_vec();
        let bytes = pack(GAME, &entries, &fixture_identity(), false).expect("pack duplicate fixture");
        let container = Container::from_bytes(bytes, &ExpectedIdentity::default())
            .expect("the first representative snapshot remains valid");

        let error = container
            .load(17050260)
            .expect_err("duplicate snapshot members must be rejected")
            .to_string();
        assert!(error.contains("duplicate JSON member"), "{case}: {error}");
        assert!(error.contains(duplicate_key), "{case}: {error}");
    }
}

#[test]
fn raw_collector_stream_can_be_enumerated_and_read_verified() {
    let bytes = build_container(true);
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("open");
    let entries = c.entries_by_role("collector_raw_stream");

    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(
        entry.path,
        format!("payload/{GAME}/collector_raw/collector_frames.jsonl")
    );
    assert_eq!(entry.role.as_deref(), Some("collector_raw_stream"));
    assert_eq!(entry.format, None);

    let raw = c.read_entry_bytes(&entry.path).expect("read raw stream");
    assert_eq!(
        raw,
        br#"{"type":"domain_delta","sequence":1,"domain":"divisions"}
{"type":"gap","sequence":2,"domain":"land_combats"}
"#
    );
    assert_eq!(entry.bytes as usize, raw.len());
    assert_eq!(entry.sha256, snapshot::sha256::Sha256::hex(&raw));
    c.verify_all()
        .expect("raw stream participates in full verification");
}

#[test]
fn legacy_pre222_store_deflate_fixture_opens_and_stream_verifies() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/legacy/pre222_store_deflate.battleplan");
    let container =
        Container::open_strict_with_file_receipt_from_file(
            std::fs::File::open(&path).expect("open legacy fixture"),
            &ExpectedIdentity::default(),
        )
        .map(|(container, _, _)| container)
        .expect("open legacy fixture");
    assert_eq!(container.game_id(), "legacy");
    let snap = container
        .load(16_959_360)
        .expect("load legacy fixture snapshot");
    assert_eq!(snap.hour_index, Some(16_959_360));
    let stream_entry = container.entries_by_role("collector_compact_stream");
    assert_eq!(stream_entry.len(), 1);
    let stream = container
        .read_entry_bytes(&stream_entry[0].path)
        .expect("read legacy compact stream");
    assert!(stream.starts_with(b"H4CR"));
    container.verify_all().expect("legacy fixture verify_all");

    let report =
        snapshot::verify_replay_container_streaming(&path, "legacy", "collector_compact_stream")
            .expect("streaming verify legacy fixture");
    assert_eq!(report.entries_verified, 4);
    assert_eq!(report.stream_entry_bytes, stream.len() as u64);
    assert!(report.stream_entry_sha256.is_some());
}

#[test]
fn verify_all_passes_on_clean_container() {
    let c =
        Container::from_bytes(build_container(true), &ExpectedIdentity::default()).expect("open");
    c.verify_all().expect("all entries hash-match");
}

#[test]
fn rejects_tampered_first_snapshot_at_open() {
    let mut bytes = build_container(false);
    let needle = b"\"stability\":0.0";
    let pos = find(&bytes, needle).expect("snapshot data present in STORE zip");
    bytes[pos + needle.len() - 1] = b'9';
    reseal_store_crc(
        &mut bytes,
        &format!("payload/{GAME}/000017000461.snapshot.json"),
    );
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(c.as_ref().err(), Some(ContainerError::PayloadIntegrity(m)) if m.contains("sha256")),
        "tampered first snapshot must be rejected as payload integrity, got err={:?}",
        c.err()
    );
}

#[test]
fn rejects_tampered_later_snapshot_on_lazy_read() {
    let mut bytes = build_container(false);
    let needle = br#""countries":{"GER":{}}"#;
    let pos = find(&bytes, needle).expect("snap_b unique payload present");
    bytes[pos + 2] = b'x';
    reseal_store_crc(
        &mut bytes,
        &format!("payload/{GAME}/s0003.000017050260.snapshot.json"),
    );
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default())
        .expect("opens: first snapshot + index are intact");
    let err = c.load(17050260).unwrap_err();
    assert!(
        matches!(&err, ContainerError::PayloadIntegrity(m) if m.contains("sha256")),
        "tampered later snapshot must fail on lazy read as payload integrity, got {err}"
    );
    assert!(c.load(17000461).is_ok());
}

#[test]
fn rejects_tampered_manifest_identity() {
    let c = Container::from_bytes(
        build_container(false),
        &ExpectedIdentity {
            build_exe_sha256: Some("deadbeef".into()),
            ..Default::default()
        },
    );
    assert!(
        matches!(c, Err(ContainerError::Reject(m)) if m.contains("build.exe_sha256")),
        "wrong build must reject"
    );
}

#[test]
fn rejects_wrong_schema() {
    let c = Container::from_bytes(
        build_container(false),
        &ExpectedIdentity {
            schema_version: Some("2".into()),
            ..Default::default()
        },
    );
    assert!(matches!(c, Err(ContainerError::Reject(m)) if m.contains("schema_version")));
}

#[test]
fn rejects_wrong_modset() {
    let c = Container::from_bytes(
        build_container(false),
        &ExpectedIdentity {
            modset_id: Some("not-the-modset".into()),
            ..Default::default()
        },
    );
    assert!(matches!(c, Err(ContainerError::Reject(m)) if m.contains("modset_id")));
}

#[test]
fn accepts_matching_identity() {
    let c = Container::from_bytes(
        build_container(false),
        &ExpectedIdentity {
            build_exe_sha256: Some(BUILD.into()),
            schema_version: Some("1".into()),
            modset_id: Some(MODSET.into()),
        },
    );
    assert!(c.is_ok(), "matching identity should open: {:?}", c.err());
}

#[test]
fn rejects_manifest_digest_corruption() {
    let mut bytes = build_container(false);
    let needle = b"2026-06-26T21:30:00Z";
    let pos = find(&bytes, needle).expect("created_utc in manifest");
    bytes[pos] = b'1';
    reseal_store_crc(&mut bytes, "manifest.json");
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(c, Err(ContainerError::Reject(m)) if m.contains("manifest_digest")),
        "digest mismatch must reject"
    );
}

#[test]
fn rejects_no_manifest_recovery_only() {
    let opts = PackOptions {
        omit_manifest: true,
        ..Default::default()
    };
    let bytes = pack_with(GAME, &fixture_entries(), &fixture_identity(), false, &opts).unwrap();
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(c.as_ref().err(), Some(ContainerError::NoManifest)),
        "payload-only file must be NoManifest (recovery-only), got err={:?}",
        c.err()
    );

    let garbage = b"PK\x03\x04 not a real central directory".to_vec();
    assert!(Container::from_bytes(garbage, &ExpectedIdentity::default()).is_err());
}

#[test]
fn rejects_unknown_required_feature() {
    let opts = PackOptions {
        inject_required_feature: Some("future_delta_encoding_v2".into()),
        ..Default::default()
    };
    let bytes = pack_with(GAME, &fixture_entries(), &fixture_identity(), false, &opts).unwrap();
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(c.as_ref().err(), Some(ContainerError::Reject(m)) if m.contains("required feature")),
        "unknown required feature must reject, got err={:?}",
        c.err()
    );
}

#[test]
fn rejects_required_features_wrong_type() {
    let opts = PackOptions {
        required_features_raw: Some(json!("future_delta_encoding_v2")),
        ..Default::default()
    };
    let bytes = pack_with(GAME, &fixture_entries(), &fixture_identity(), false, &opts).unwrap();
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(c.as_ref().err(), Some(ContainerError::Reject(m)) if m.contains("required_features")),
        "non-array required_features must reject (fail-closed), got err={:?}",
        c.err()
    );

    let opts2 = PackOptions {
        required_features_raw: Some(json!([123])),
        ..Default::default()
    };
    let bytes2 = pack_with(GAME, &fixture_entries(), &fixture_identity(), false, &opts2).unwrap();
    let c2 = Container::from_bytes(bytes2, &ExpectedIdentity::default());
    assert!(
        matches!(c2.as_ref().err(), Some(ContainerError::Reject(m)) if m.contains("required_features")),
        "non-string required_features element must reject, got err={:?}",
        c2.err()
    );
}

#[test]
fn rejects_stowaway_zip_entry_not_in_manifest() {
    let opts = PackOptions {
        inject_stowaway: Some((
            format!("payload/{GAME}/evil.snapshot.json"),
            br#"{"injected":true}"#.to_vec(),
        )),
        ..Default::default()
    };
    let bytes = pack_with(GAME, &fixture_entries(), &fixture_identity(), false, &opts).unwrap();
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(c.as_ref().err(), Some(ContainerError::Reject(m)) if m.contains("not listed in the manifest")),
        "stowaway zip entry must reject, got err={:?}",
        c.err()
    );
}

#[test]
fn rejects_path_traversal_entry_name() {
    use snapshot::zip::validate_path;
    assert!(validate_path("../escape").is_err());
    assert!(validate_path("payload/../../etc").is_err());
}

#[test]
fn strict_open_rejects_duplicate_role_member_before_last_wins() {
    let bytes = pack_with(
        GAME,
        &fixture_entries(),
        &fixture_identity(),
        false,
        &PackOptions {
            inject_duplicate_role_member: true,
            ..Default::default()
        },
    )
    .unwrap();
    let error = Container::from_bytes_strict(bytes, &ExpectedIdentity::default())
        .err()
        .expect("duplicate role must reject")
        .to_string();
    assert!(error.contains("duplicate JSON member"), "{error}");
    assert!(error.contains("role"), "{error}");
}

#[test]
fn strict_open_rejects_present_non_string_role() {
    let bytes = pack_with(
        GAME,
        &fixture_entries(),
        &fixture_identity(),
        false,
        &PackOptions {
            first_role_raw: Some(json!(7)),
            ..Default::default()
        },
    )
    .unwrap();
    let error = Container::from_bytes_strict(bytes, &ExpectedIdentity::default())
        .err()
        .expect("non-string role must reject")
        .to_string();
    assert!(
        error.contains("role is present but is not a string"),
        "{error}"
    );
}

#[test]
fn strict_open_rejects_unroled_ascii_timeline_lookalike() {
    let mut entries: Vec<PackEntry> = fixture_entries()
        .into_iter()
        .filter(|entry| entry.path != format!("payload/{GAME}/timeline.json"))
        .collect();
    entries.push(PackEntry {
        path: format!("payload/{GAME}/Timeline.json"),
        bytes: br#"{"lookalike":true}"#.to_vec(),
        role: "artifact",
        hour_index: None,
    });
    let bytes = pack(GAME, &entries, &fixture_identity(), false).unwrap();
    let error = Container::from_bytes_strict(bytes, &ExpectedIdentity::default())
        .err()
        .expect("timeline lookalike must reject")
        .to_string();
    assert!(error.contains("ASCII-case lookalike"), "{error}");
    assert!(error.contains("Timeline.json"), "{error}");
}

fn named_zip_header(bytes: &[u8], name: &str, signature: [u8; 4], fixed_len: usize) -> usize {
    bytes
        .windows(name.len())
        .enumerate()
        .find_map(|(name_start, candidate)| {
            let header = name_start.checked_sub(fixed_len)?;
            (candidate == name.as_bytes()
                && bytes.get(header..header + 4) == Some(signature.as_slice()))
            .then_some(header)
        })
        .unwrap_or_else(|| panic!("missing ZIP header for {name}"))
}

fn reseal_store_crc(bytes: &mut [u8], name: &str) {
    let local = named_zip_header(bytes, name, [0x50, 0x4b, 0x03, 0x04], 30);
    let name_len = u16::from_le_bytes(bytes[local + 26..local + 28].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(bytes[local + 28..local + 30].try_into().unwrap()) as usize;
    let payload_len =
        u32::from_le_bytes(bytes[local + 18..local + 22].try_into().unwrap()) as usize;
    let payload_start = local + 30 + name_len + extra_len;
    let crc = crc32fast::hash(&bytes[payload_start..payload_start + payload_len]);
    bytes[local + 14..local + 18].copy_from_slice(&crc.to_le_bytes());

    let central = named_zip_header(bytes, name, [0x50, 0x4b, 0x01, 0x02], 46);
    bytes[central + 16..central + 20].copy_from_slice(&crc.to_le_bytes());
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn write_temp(bytes: &[u8], name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("hoi4replay_test_{}_{}", std::process::id(), name));
    std::fs::write(&p, bytes).expect("write temp container");
    p
}
