use super::fixtures::{raw_zip, archive_layer, folder_layer, folder_of, overlaid_stack};
use super::*;

mod memo_tests;

pub(super) use crate::test_support::{Record, zip_records};
#[cfg(windows)]
use crate::test_support::{file_timestamps, set_file_timestamps};

#[test]
fn prepared_metadata_inputs_keep_mount_order_and_archive_files_as_single_inputs() {
    let root = std::env::temp_dir().join(format!("prepared-inputs-{}", std::process::id()));
    let base = root.join("base");
    let mod_folder = root.join("mod");
    fs::create_dir_all(base.join("common")).unwrap();
    fs::create_dir_all(mod_folder.join("common")).unwrap();
    fs::write(base.join("common/a.txt"), b"synthetic base").unwrap();
    fs::write(mod_folder.join("common/b.txt"), b"synthetic mod").unwrap();
    let mut layers = ResolvedLayers::from_folders(&[(&base, &[]), (&mod_folder, &[])]).unwrap();
    let expected = vec![
        fs::canonicalize(base.join("common/a.txt")).unwrap(),
        fs::canonicalize(mod_folder.join("common/b.txt")).unwrap(),
    ];
    assert_eq!(layers.recorded_source_files().unwrap(), expected);
    let descriptor = root.join("synthetic.mod");
    let archive = root.join("synthetic.zip");
    layers.layers[1].source = LayerSource::Archive(archive.clone());
    layers.layers[1].descriptor = Some(descriptor.clone());
    assert_eq!(
        layers.recorded_source_files().unwrap(),
        vec![
            fs::canonicalize(base.join("common/a.txt")).unwrap(),
            descriptor,
            archive
        ]
    );
    assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verified_prefix_retains_only_the_header_and_checks_the_remaining_payload() {
    let root = std::env::temp_dir().join(format!("verified-prefix-{}", std::process::id()));
    fs::create_dir_all(root.join("map")).unwrap();
    let mut bytes = vec![5; 2 * 1024 * 1024];
    bytes[..6].copy_from_slice(b"header");
    let path = root.join("map/synthetic.bin");
    fs::write(&path, &bytes).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert_eq!(
        layers.read_prefix("map/synthetic.bin", 6).unwrap(),
        (Bytes::from(b"header".as_slice()), bytes.len() as u64)
    );
    *bytes.last_mut().unwrap() = 7;
    fs::write(path, bytes).unwrap();
    assert!(
        layers
            .read_prefix("map/synthetic.bin", 6)
            .unwrap_err()
            .to_string()
            .contains("changed since verification")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_uncovered_reads_report_hits_and_missing_candidates() {
    let root = std::env::temp_dir().join(format!("explicit-uncovered-{}", std::process::id()));
    fs::create_dir_all(root.join("gfx/custom")).unwrap();
    fs::write(root.join("gfx/custom/mod-icon.dds"), b"synthetic").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let reads = layers.report_uncovered_reads();
    for _ in 0..2 {
        assert_eq!(
            layers
                .read_uncovered_path(b"gfx/custom/mod-icon.dds")
                .unwrap(),
            b"synthetic"
        );
        assert_eq!(
            layers
                .read_uncovered_path(b"gfx/custom/missing.dds")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }
    let warning = reads.warning().unwrap();
    assert!(warning.contains("2 file(s)"), "{warning}");
    assert!(warning.contains("gfx/custom/mod-icon.dds"));
    assert!(warning.contains("gfx/custom/missing.dds"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
pub(super) fn file_usn(file: &File) -> i64 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::{
        IO::DeviceIoControl,
        Ioctl::{FSCTL_READ_FILE_USN_DATA, USN_RECORD_V2},
    };
    let versions = [2_u16, 2_u16];
    let mut buffer = [0_u64; 512];
    let mut returned = 0;
    assert_ne!(
        unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_READ_FILE_USN_DATA,
                versions.as_ptr().cast(),
                std::mem::size_of_val(&versions) as u32,
                buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&buffer) as u32,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        },
        0,
        "synthetic fixture requires NTFS file change records"
    );
    assert!(returned >= std::mem::size_of::<USN_RECORD_V2>() as u32);
    let record = unsafe { &*buffer.as_ptr().cast::<USN_RECORD_V2>() };
    assert_eq!(record.MajorVersion, 2);
    assert!(record.RecordLength <= returned);
    record.Usn
}

#[cfg(windows)]
pub(super) fn overwrite_preserving_timestamps(path: &Path, offset: u64, bytes: &[u8]) {
    use std::io::{Seek, SeekFrom, Write};
    let mut writer = File::options().write(true).open(path).unwrap();
    set_file_timestamps(&writer, -1, -1);
    writer.seek(SeekFrom::Start(offset)).unwrap();
    writer.write_all(bytes).unwrap();
    writer.sync_all().unwrap();
}

#[cfg(windows)]
#[test]
fn held_reader_central_directory_edit_is_detected_on_the_next_open() {
    let root = std::env::temp_dir().join(format!("mount-central-edit-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("synthetic.zip");
    let bytes = raw_zip(&[(b"common/one.txt", 0, b"unchanged payload")]);
    let central = bytes
        .windows(4)
        .position(|bytes| bytes == 0x0201_4B50_u32.to_le_bytes())
        .unwrap();
    fs::write(&path, &bytes).unwrap();
    let reader = File::open(&path).unwrap();
    let stamp = Stamp::read(&path).unwrap();
    let timestamps = file_timestamps(&reader);
    overwrite_preserving_timestamps(&path, (central + 46) as u64, b"common/one.txt");
    let layer = archive_layer("synthetic", &path, &[]);
    assert_eq!(
        layer
            .archive_open(b"common/one.txt", &mut ReadObservation::default())
            .unwrap()
            .unwrap(),
        b"unchanged payload"
    );
    let usn = file_usn(&reader);
    overwrite_preserving_timestamps(&path, (central + 46) as u64, b"common/two.txt");
    assert_eq!(Stamp::read(&path).unwrap(), stamp);
    assert_eq!(file_timestamps(&reader), timestamps);
    assert_eq!(file_usn(&reader), usn);
    let edited = fs::read(&path).unwrap();
    assert_eq!(&edited[..central], &bytes[..central]);
    for result in [
        layer
            .archive_open(b"common/one.txt", &mut ReadObservation::default())
            .map(|_| ()),
        layer.archive_exists(b"common/two.txt").map(|_| ()),
        layer
            .archive_open(b"outside/absent", &mut ReadObservation::default())
            .map(|_| ()),
    ] {
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("changed since verification") && error.contains("common/two.txt"),
            "{error}"
        );
    }
    drop(reader);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn derived_caches_share_a_resolved_stack_and_reset_for_another_roster() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let mut layers = ResolvedLayers::default();
    let first = layers.derived::<AtomicUsize>();
    first.store(7, Ordering::Relaxed);
    assert!(Arc::ptr_eq(
        &first,
        &layers.clone().derived::<AtomicUsize>()
    ));
    assert_eq!(
        layers
            .clone()
            .derived::<AtomicUsize>()
            .load(Ordering::Relaxed),
        7
    );
    assert!(!Arc::ptr_eq(
        &first,
        &ResolvedLayers::default().derived::<AtomicUsize>()
    ));
    let root = std::env::temp_dir().join(format!("derived-roster-{}", std::process::id()));
    layers
        .layers
        .push(folder_layer("additional-mod", &root, Vec::new()));
    let changed = layers.derived::<AtomicUsize>();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.load(Ordering::Relaxed), 0);
}

#[test]
fn tracing_scopes_and_named_readers_isolate_lazy_caches_while_retained_clones_share_them() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let layers = ResolvedLayers::default();
    let original = layers.derived::<AtomicUsize>();
    original.store(7, Ordering::Relaxed);
    let traced = layers.with_read_trace(Arc::new(|_| {}));
    assert!(!Arc::ptr_eq(&original, &traced.derived::<AtomicUsize>()));
    assert!(!Arc::ptr_eq(
        &layers.derived::<RetainedProducts>(),
        &traced.derived::<RetainedProducts>()
    ));
    let a = traced.with_read_purpose("a");
    let b = traced.with_read_purpose("b");
    let cache = a.derived::<AtomicUsize>();
    cache.store(11, Ordering::Relaxed);
    let a_clone = a.clone();
    assert!(Arc::ptr_eq(&cache, &a_clone.derived::<AtomicUsize>()));
    assert!(!Arc::ptr_eq(&cache, &b.derived::<AtomicUsize>()));
    assert!(!Arc::ptr_eq(&a.derived::<RetainedProducts>(), &b.derived::<RetainedProducts>()));
    assert_eq!(b.derived::<AtomicUsize>().load(Ordering::Relaxed), 0);
    assert_eq!(original.load(Ordering::Relaxed), 7);
}

#[test]
fn traced_archive_verification_failures_keep_the_read_payload_and_original_error() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "traced-archive-verification-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("synthetic.zip");
    let mut payloads = Vec::new();
    for bytes in [b"invalid1", b"invalid2"] {
        fs::write(&path, raw_zip(&[(b"common/one.txt", 0, bytes)])).unwrap();
        let mut layer = archive_layer("synthetic", &path, &[]);
        Arc::make_mut(&mut layer.hashes).insert(b"common/one.txt".to_vec(), [0; 32]);
        let layers = ResolvedLayers::from_test_layers(vec![layer]).unwrap();
        let original = layers.read("common/one.txt").unwrap_err();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let log = reads.clone();
        let traced = layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)));
        let error = traced.read_prefix("common/one.txt", 2).unwrap_err();
        assert_eq!(error.kind(), original.kind());
        assert_eq!(error.to_string(), original.to_string());
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "content file changed since verification: common/one.txt"
        );
        let reads = reads.lock().unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].path, b"common/one.txt");
        assert_eq!(reads[0].layer, Some(0));
        match &reads[0].outcome {
            ReadOutcome::Error {
                kind,
                message,
                payload: Some(payload),
            } => {
                assert_eq!(kind, "InvalidData");
                assert_eq!(message, &error.to_string());
                assert_eq!(payload.length, bytes.len() as u64);
                assert_eq!(payload.digest, <[u8; 32]>::from(Sha256::digest(bytes)));
                payloads.push(payload.clone());
            }
            outcome => panic!("expected rejected byte evidence, got {outcome:?}"),
        }
    }
    assert_eq!(payloads[0].length, payloads[1].length);
    assert_ne!(payloads[0].digest, payloads[1].digest);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_database_directory_lists_its_own_files_by_byte_name_without_recursion() {
    let listed = [
        "gfx/train_gfx_database/zz_mod.txt",
        "gfx/train_gfx_database/sub/a.txt",
        "gfx/train_gfx_database/B.txt",
        "gfx/train_gfx_database/a.txt",
        "gfx/train_gfx_database/upper.TXT",
        "gfx/train_gfx_database_other/a.txt",
    ]
    .map(ScriptPath::from)
    .to_vec();
    assert_eq!(
        directory_order(b"gfx/train_gfx_database", ".txt", listed),
        [
            "gfx/train_gfx_database/B.txt",
            "gfx/train_gfx_database/a.txt",
            "gfx/train_gfx_database/zz_mod.txt",
        ]
    );
}

#[test]
fn a_database_directory_filters_by_its_loaders_raw_suffix() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_raw_suffix_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let directory = root.join("gfx/train_gfx_database");
    fs::create_dir_all(directory.join("sub")).unwrap();
    for name in ["a.txt", "footxt", "upper.TXT", "b.yml", "sub/c.txt"] {
        fs::write(directory.join(name), b"synthetic").unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let names = |suffix: &str| {
        layers
            .directory_files("gfx/train_gfx_database", suffix)
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names("txt"),
        ["gfx/train_gfx_database/a.txt", "gfx/train_gfx_database/footxt"]
    );
    assert_eq!(names(".txt"), ["gfx/train_gfx_database/a.txt"]);
    drop(layers);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn coverage_is_closed_and_paths_cannot_escape_the_layer() {
    for path in [
        "map/provinces.bmp",
        "common/countries/colors.txt",
        "events/nested/outcomes.txt",
        "gfx/interface/x.dds",
        "gfx/flags/GER.tga",
        "GFX/Texticons/nested/x.dds",
        "gfx/leaders/nested/x.tga",
        "gfx/fonts/x.fnt",
        "gfx/FX/x.shader",
        "GFX/fx/nested/x.fxh",
        "gfx/posteffect_volumes.txt",
        "gfx/entities/buildings.asset",
        "GFX/Models/Buildings/supply_station.GFX",
        "localisation/english/x.yml",
    ] {
        assert!(covered(path));
    }
    for path in [
        "music/x.ogg",
        "gfx/models/x.mesh",
        "gfx/models/x.dds",
        "gfx/posteffect/x.shader",
        "gfx/posteffect_volumes.txt.bak",
        "gfx/fonts_backup/x.fnt",
        "gfx/fx_backup/x.shader",
        "events_backup/x.txt",
        "gfx/texticons_backup/x.dds",
        "gfx/leaders_backup/x.tga",
        "gfx/sound/x.asset",
        "gfx/loadingscreens/x.dds",
        "map_backup/x.bmp",
        "launcher/logs/x.txt",
    ] {
        assert!(!covered(path));
    }
    assert_eq!(normalize_relative("map\\x.bmp").unwrap(), "map/x.bmp");
    for path in [
        "",
        "../x",
        "map/../x",
        "map//x",
        "/x",
        "C:/x",
        "map/x:stream",
        "map/./x",
        "map/x\0",
    ] {
        assert!(normalize_relative(path).is_err(), "{path}");
    }
}

#[test]
fn coverage_policy_four_hashes_and_verifies_event_tooltip_font_effect_and_volume_roots() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_coverage_four_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let (game, user, cache) = (root.join("game"), root.join("user"), root.join("cache"));
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(game.join("dlc")).unwrap();
    fs::write(game.join("hoi4.exe"), b"synthetic executable").unwrap();
    fs::write(user.join("settings.txt"), b"language=l_english").unwrap();
    fs::write(
        user.join("dlc_load.json"),
        br#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    )
    .unwrap();
    let paths = [
        "events/nested/outcomes.txt",
        "gfx/texticons/nested/icon.dds",
        "gfx/leaders/nested/portrait.tga",
        "gfx/fonts/nested/font.fnt",
        "gfx/FX/nested/effect.shader",
        "gfx/posteffect_volumes.txt",
    ];
    for path in paths {
        let file = game.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"original").unwrap();
    }
    let gathered = gather(&GatherOptions {
        game_dir: game.clone(),
        user_dir: user.clone(),
        cache_dir: cache.clone(),
        dlc_order: None,
    })
    .unwrap();
    let identity = gathered
        .identity(
            "synthetic build".into(),
            Sha256::digest(b"synthetic executable").into(),
            20,
            "test".into(),
            [0; 32],
        )
        .unwrap();
    assert_eq!(identity.coverage_policy, 5);
    let options = ResolveOptions {
        game_dir: game.clone(),
        user_dir: user,
        cache_dir: cache,
    };
    let resolved = resolve(&identity, &options).unwrap();
    for path in paths {
        assert_eq!(
            gathered.layers.layers[0].hashes[path.as_bytes()],
            <[u8; 32]>::from(Sha256::digest(b"original")),
            "{path}"
        );
        assert_eq!(resolved.layers.read(path).unwrap(), b"original");
        assert_eq!(resolved.layers.read_uncovered(path).unwrap(), b"original");
        fs::write(game.join(path), b"changed content").unwrap();
        assert_eq!(
            resolved.layers.read(path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            resolved.layers.read_uncovered(path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let changed = resolve(&identity, &options).unwrap();
        assert_eq!(changed.layers.read(path).unwrap(), b"changed content");
        assert_eq!(changed.changed.len(), 1);
        let layer = &changed.changed[0];
        assert_eq!((layer.kind, layer.id.as_str()), ("Game", "vanilla"));
        assert_eq!(layer.recorded, identity.vanilla_hash);
        assert_ne!(layer.installed, layer.recorded);
        let error = changed.exact().unwrap_err().to_string();
        assert!(
            error.contains("Game vanilla: content hash mismatch"),
            "{path}: {error}"
        );
        fs::write(game.join(path), b"original").unwrap();
    }
    for policy in [1, COVERAGE_POLICY_VERSION + 1] {
        let mut old = identity.clone();
        old.coverage_policy = policy;
        assert_eq!(
            resolve(&old, &options).unwrap_err().to_string(),
            format!(
                "unsupported recorded content coverage policy {policy} (supported 2 through {COVERAGE_POLICY_VERSION})"
            )
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn older_coverage_policies_verify_their_own_roots_and_leave_newer_roots_uncovered() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_older_coverage_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let (game, user, cache) = (root.join("game"), root.join("user"), root.join("cache"));
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("hoi4.exe"), b"synthetic executable").unwrap();
    let policy_two = ["map/root.txt", "gfx/entities/root.asset"];
    let policy_three = [
        "events/nested/outcomes.txt",
        "gfx/texticons/nested/icon.dds",
        "gfx/leaders/nested/portrait.tga",
    ];
    let policy_four = [
        "gfx/fonts/nested/font.fnt",
        "gfx/FX/nested/effect.shader",
        "gfx/posteffect_volumes.txt",
    ];
    for path in policy_two
        .into_iter()
        .chain(policy_three)
        .chain(policy_four)
    {
        let file = game.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"original").unwrap();
    }
    let options = ResolveOptions {
        game_dir: game.clone(),
        user_dir: user,
        cache_dir: cache,
    };
    for policy in [2, 3] {
        let kept: Vec<_> = policy_two
            .into_iter()
            .chain(policy_three.into_iter().filter(|_| policy == 3))
            .collect();
        let hashes = kept
            .iter()
            .map(|path| {
                (
                    path.as_bytes().to_vec(),
                    <[u8; 32]>::from(Sha256::digest(b"original")),
                )
            })
            .collect();
        let identity = Identity {
            game_version: "synthetic build".into(),
            exe_hash: Sha256::digest(b"synthetic executable").into(),
            exe_size: 20,
            vanilla_hash: layer_hash(&hashes).unwrap(),
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: policy,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [0; 32],
            verification: Verification::Unchecked,
        };
        let resolved = resolve(&identity, &options).unwrap();
        assert_eq!(resolved.layers.paths().unwrap().count(), kept.len());
        for path in &kept {
            assert!(resolved.layers.covered(path), "policy {policy}: {path}");
            assert_eq!(resolved.layers.read(path).unwrap(), b"original");
            fs::write(game.join(path), b"changed covered bytes").unwrap();
            assert_eq!(
                resolved.layers.read_uncovered(path).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            let changed = resolve(&identity, &options).unwrap();
            assert_eq!(changed.layers.read(path).unwrap(), b"changed covered bytes");
            let error = changed.exact().unwrap_err().to_string();
            assert!(
                error.contains("Game vanilla: content hash mismatch"),
                "{error}"
            );
            fs::write(game.join(path), b"original").unwrap();
        }
        let uncovered = policy_three
            .into_iter()
            .filter(|_| policy == 2)
            .chain(policy_four);
        for path in uncovered {
            assert!(!resolved.layers.covered(path), "policy {policy}: {path}");
            assert!(covered(path), "current policy: {path}");
            assert_eq!(resolved.layers.read_uncovered(path).unwrap(), b"original");
            fs::write(game.join(path), b"changed uncovered bytes").unwrap();
            assert_eq!(
                resolved.layers.read_uncovered(path).unwrap(),
                b"changed uncovered bytes"
            );
            let fresh = resolve(&identity, &options).unwrap();
            assert!(!fresh.layers.covered(path));
            assert_eq!(
                fresh.layers.read_uncovered(path).unwrap(),
                b"changed uncovered bytes"
            );
            fs::write(game.join(path), b"original").unwrap();
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_coverage_reads_newer_folder_and_archive_files_with_counted_notes() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_coverage_reads_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let folder = root.join("folder");
    let archive = root.join("content.zip");
    let files: [(&str, &[u8]); 8] = [
        ("common/check.txt", b"covered"),
        ("gfx/models/check.txt", b"model"),
        ("events/check.txt", b"event"),
        ("gfx/leaders/check.tga", b"portrait"),
        ("gfx/texticons/check.dds", b"icon"),
        ("gfx/fonts/check.fnt", b"font"),
        ("gfx/FX/check.shader", b"effect"),
        ("gfx/posteffect_volumes.txt", b"posteffect_volumes = {}"),
    ];
    for (path, bytes) in files {
        let path = folder.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let members: Vec<_> = files
        .iter()
        .map(|(path, bytes)| (path.as_bytes(), 0, *bytes))
        .collect();
    fs::write(&archive, raw_zip(&members)).unwrap();
    for source in [&folder, &archive] {
        for policy in [2, 3, COVERAGE_POLICY_VERSION] {
            let layers =
                ResolvedLayers::from_folders_with_coverage_policy(&[(source, &[])], policy)
                    .unwrap();
            let error = layers.read("gfx/models/check.txt").unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("this reader does not report uncovered reads")
            );
            let reads = layers.report_uncovered_reads();
            assert!(layers.has_file("gfx/posteffect_volumes.txt").unwrap());
            assert!(layers.read("gfx/fonts/missing.fnt").is_err());
            assert_eq!(layers.read("common/check.txt").unwrap(), b"covered");
            assert_eq!(reads.warning(), None);
            for (path, bytes) in files {
                assert_eq!(layers.read(path).unwrap(), bytes);
                assert_eq!(layers.clone().read_path(path.as_bytes()).unwrap(), bytes);
            }
            let mut uncovered: Vec<_> = files
                .iter()
                .filter_map(|(path, _)| (!layers.covered(path)).then_some(*path))
                .collect();
            uncovered.sort();
            let expected = (!uncovered.is_empty()).then(|| {
                format!(
                    "Content: {} file(s) read outside the recorded content coverage policy {policy}; these bytes are not hash-verified: {}.",
                    uncovered.len(),
                    uncovered.join(", ")
                )
            });
            assert_eq!(reads.warning(), expected);
            if source == &folder {
                fs::write(folder.join("common/check.txt"), b"changed").unwrap();
                assert_eq!(
                    layers.read("common/check.txt").unwrap_err().kind(),
                    io::ErrorKind::InvalidData
                );
                assert_eq!(reads.warning(), expected);
                fs::write(folder.join("common/check.txt"), b"covered").unwrap();
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reported_uncovered_reads_refresh_diagnostics_after_lazy_reads() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_lazy_coverage_reads_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(root.join("gfx/models")).unwrap();
    fs::write(root.join("gfx/models/first.txt"), b"first").unwrap();
    fs::write(root.join("gfx/models/late.txt"), b"late").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let reads = layers.report_uncovered_reads();
    let mut warnings = vec!["Another diagnostic".into()];
    reads.append_to(&mut warnings);
    assert_eq!(warnings.len(), 1);
    let other = layers.clone();
    other.read("gfx/models/first.txt").unwrap();
    reads.append_to(&mut warnings);
    assert!(warnings[1].starts_with("Content: 1 file(s)"));
    layers.read("gfx/models/late.txt").unwrap();
    reads.append_to(&mut warnings);
    reads.append_to(&mut warnings);
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0], "Another diagnostic");
    assert!(warnings[1].starts_with("Content: 2 file(s)"));
    assert!(warnings[1].contains("gfx/models/first.txt, gfx/models/late.txt"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn older_coverage_policy_roots_apply_to_builtin_dlc_and_mod_layers() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_layer_older_coverage_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let (game, user) = (root.join("game"), root.join("user"));
    let write = |path: &Path, bytes: &[u8]| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    };
    let integrated = b"name=Integrated";
    let dlc = b"name=Synthetic path=dlc/synthetic";
    let mod_descriptor = b"name=Synthetic archive=layers/synthetic.zip";
    write(&game.join("hoi4.exe"), b"synthetic executable");
    write(
        &game.join("integrated_dlc/synthetic/content.dlc"),
        integrated,
    );
    write(&game.join("dlc/synthetic/content.dlc"), dlc);
    write(&user.join("mod/synthetic.mod"), mod_descriptor);
    let folder_pairs = [
        ("", "base"),
        ("integrated_dlc/synthetic/", "integrated"),
        ("dlc/synthetic/", "dlc"),
        ("tweakergui_assets/", "tweaker"),
    ];
    for (prefix, name) in folder_pairs {
        write(
            &game.join(format!("{prefix}events/{name}.txt")),
            b"original",
        );
        write(
            &game.join(format!("{prefix}gfx/fonts/{name}.fnt")),
            b"original",
        );
    }
    let archive = user.join("layers/synthetic.zip");
    write(
        &archive,
        &raw_zip(&[
            (b"events/mod.txt", 0, b"original"),
            (b"gfx/fonts/mod.fnt", 0, b"original"),
        ]),
    );
    let hash = |files: &[(&str, &[u8])]| {
        layer_hash(
            &files
                .iter()
                .map(|(path, bytes)| {
                    (
                        path.as_bytes().to_vec(),
                        <[u8; 32]>::from(Sha256::digest(bytes)),
                    )
                })
                .collect(),
        )
        .unwrap()
    };
    let identity = Identity {
        game_version: "synthetic build".into(),
        exe_hash: Sha256::digest(b"synthetic executable").into(),
        exe_size: 20,
        vanilla_hash: hash(&[
            ("events/base.txt", b"original"),
            ("integrated_dlc/synthetic/content.dlc", integrated),
            (
                "integrated_dlc/synthetic/events/integrated.txt",
                b"original",
            ),
            ("tweakergui_assets/events/tweaker.txt", b"original"),
        ]),
        dlc: vec![Dlc {
            id: "synthetic".into(),
            hash: hash(&[("content.dlc", dlc), ("events/dlc.txt", b"original")]),
            declared_checksum: None,
        }],
        mods: vec![Mod {
            id: "synthetic.mod".into(),
            display_name: "Synthetic".into(),
            declared_version: String::new(),
            hash: hash(&[
                ("synthetic.mod", mod_descriptor),
                ("events/mod.txt", b"original"),
            ]),
        }],
        coverage_policy: 3,
        language: "l_english".into(),
        recorder_version: "test".into(),
        collector_hash: [0; 32],
        verification: Verification::Unchecked,
    };
    let options = ResolveOptions {
        game_dir: game.clone(),
        user_dir: user,
        cache_dir: root.join("cache"),
    };
    let layers = resolve(&identity, &options).unwrap().layers;
    assert_eq!(layers.layers.len(), 5);
    assert!(!layers.base_only().unwrap().covered("gfx/fonts/base.fnt"));
    for name in ["base", "integrated", "dlc", "tweaker", "mod"] {
        assert_eq!(
            layers.read(&format!("events/{name}.txt")).unwrap(),
            b"original"
        );
        assert!(!layers.covered(&format!("gfx/fonts/{name}.fnt")));
        assert_eq!(
            layers
                .read_uncovered(&format!("gfx/fonts/{name}.fnt"))
                .unwrap(),
            b"original"
        );
    }
    for (prefix, name) in folder_pairs {
        write(
            &game.join(format!("{prefix}gfx/fonts/{name}.fnt")),
            b"newer uncovered bytes",
        );
    }
    write(
        &archive,
        &raw_zip(&[
            (b"events/mod.txt", 0, b"original"),
            (b"gfx/fonts/mod.fnt", 0, b"newer uncovered bytes"),
        ]),
    );
    let fresh = resolve(&identity, &options).unwrap().layers;
    for name in ["base", "integrated", "dlc", "tweaker", "mod"] {
        assert_eq!(
            fresh
                .read_uncovered(&format!("gfx/fonts/{name}.fnt"))
                .unwrap(),
            b"newer uncovered bytes"
        );
    }
    for (relative, named) in [
        ("events/base.txt", "Game vanilla"),
        (
            "integrated_dlc/synthetic/events/integrated.txt",
            "Game vanilla",
        ),
        ("tweakergui_assets/events/tweaker.txt", "Game vanilla"),
        ("dlc/synthetic/events/dlc.txt", "DLC synthetic"),
    ] {
        write(&game.join(relative), b"changed covered bytes");
        let changed = resolve(&identity, &options).unwrap();
        let names: Vec<_> = changed
            .changed
            .iter()
            .map(|layer| format!("{} {}", layer.kind, layer.id))
            .collect();
        assert_eq!(names, [named]);
        let error = changed.exact().unwrap_err().to_string();
        assert!(
            error.contains(&format!("{named}: content hash mismatch")),
            "{error}"
        );
        write(&game.join(relative), b"original");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn older_coverage_policies_inventory_and_verify_archive_roots() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_archive_older_coverage_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("synthetic.zip");
    let paths: &[&[u8]] = &[
        b"map/root.txt",
        b"events/root.txt",
        b"gfx/fonts/root.fnt",
        b"gfx/FX/root.shader",
        b"gfx/posteffect_volumes.txt",
    ];
    fs::write(
        &archive,
        raw_zip(
            &paths
                .iter()
                .map(|path| (*path, 0, &b"original"[..]))
                .collect::<Vec<_>>(),
        ),
    )
    .unwrap();
    for policy in [2, 3] {
        let layers =
            ResolvedLayers::from_folders_with_coverage_policy(&[(&archive, &[])], policy).unwrap();
        let expected = if policy == 2 {
            &paths[..1]
        } else {
            &paths[..2]
        };
        assert_eq!(
            layers.layers[0]
                .hashes
                .keys()
                .map(Vec::as_slice)
                .collect::<Vec<_>>(),
            expected
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        for path in expected {
            assert_eq!(layers.read_path(path).unwrap(), b"original");
        }
        for path in &paths[expected.len()..] {
            assert!(!layers.covered_path(path));
            assert_eq!(layers.read_uncovered_path(path).unwrap(), b"original");
        }
        let members: Vec<_> = paths
            .iter()
            .map(|path| {
                (
                    *path,
                    0,
                    if *path == b"map/root.txt" {
                        &b"changed"[..]
                    } else {
                        &b"original"[..]
                    },
                )
            })
            .collect();
        fs::write(&archive, raw_zip(&members)).unwrap();
        assert_eq!(
            layers
                .read_uncovered_path(b"gfx/fonts/root.fnt")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(
            &archive,
            raw_zip(
                &paths
                    .iter()
                    .map(|path| (*path, 0, &b"original"[..]))
                    .collect::<Vec<_>>(),
            ),
        )
        .unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn coverage_policy_four_verifies_archive_event_tooltip_font_effect_and_volume_roots() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_archive_coverage_four_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("synthetic.zip");
    for path in [
        "events/nested/outcomes.txt",
        "gfx/texticons/nested/icon.dds",
        "gfx/leaders/nested/portrait.tga",
        "gfx/fonts/nested/font.fnt",
        "gfx/FX/nested/effect.shader",
        "gfx/posteffect_volumes.txt",
    ] {
        fs::write(&archive, raw_zip(&[(path.as_bytes(), 0, b"original")])).unwrap();
        let layer = archive_layer("synthetic", &archive, &[]);
        assert_eq!(
            layer.hashes[path.as_bytes()],
            <[u8; 32]>::from(Sha256::digest(b"original"))
        );
        let original_hash = layer_hash(&layer.hashes).unwrap();
        let mut layers = ResolvedLayers {
            layers: vec![layer],
            ..Default::default()
        };
        layers.finish().unwrap();
        assert_eq!(layers.read(path).unwrap(), b"original");
        assert_eq!(layers.read_uncovered(path).unwrap(), b"original");
        fs::write(
            &archive,
            raw_zip(&[(path.as_bytes(), 0, b"changed content")]),
        )
        .unwrap();
        assert_eq!(
            layers.read(path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            layers.read_uncovered(path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let changed = archive_layer("synthetic", &archive, &[]);
        assert_ne!(
            original_hash,
            layer_hash(&changed.hashes).unwrap(),
            "{path}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlay_replacement_removes_subdirectories_without_prefix_collisions() {
    let mut effective = BTreeMap::new();
    let base = ["map/a", "map/sub/b", "maps/c", "common/a"].map(|path| path.as_bytes().to_vec());
    apply_overlay(&mut effective, 0, false, base.iter(), &[], None).unwrap();
    let additions = [b"map/new".to_vec(), b"common/a".to_vec()];
    apply_overlay(
        &mut effective,
        1,
        false,
        additions.iter(),
        &[b"MAP".to_vec()],
        None,
    )
    .unwrap();
    assert_eq!(
        effective
            .into_iter()
            .map(|(path, file)| (path, file.layer))
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::from([
            (b"map/new".to_vec(), 1),
            (b"maps/c".to_vec(), 0),
            (b"common/a".to_vec(), 1)
        ])
    );
}

#[test]
fn overlay_refuses_to_guess_folder_and_archive_case_precedence() {
    let mut effective = BTreeMap::new();
    apply_overlay(
        &mut effective,
        0,
        false,
        [b"map/file.txt".to_vec()].iter(),
        &[],
        None,
    )
    .unwrap();
    apply_overlay(
        &mut effective,
        1,
        false,
        [b"map/FILE.txt".to_vec()].iter(),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[b"map/file.txt".as_slice()].layer, 1);
    assert_eq!(effective[b"map/file.txt".as_slice()].path, b"map/FILE.txt");
    assert!(
        apply_overlay(
            &mut effective,
            2,
            true,
            [b"map/file.txt".to_vec()].iter(),
            &[],
            None,
        )
        .is_err()
    );
    apply_overlay(
        &mut effective,
        2,
        true,
        [b"map/file.txt".to_vec()].iter(),
        &[],
        Some(&|_: &[u8]| true),
    )
    .unwrap();
    assert_eq!(effective[b"map/file.txt".as_slice()].layer, 2);
}

#[test]
fn cache_key_requires_path_size_and_full_timestamp() {
    let stamp = Stamp {
        size: 42,
        modified_seconds: 5,
        modified_nanos: 99,
    };
    let mut cache = HashCache::default();
    cache.insert("a".into(), stamp.clone(), [7; 32]);
    assert_eq!(cache.get("a", &stamp), Some([7; 32]));
    assert_eq!(cache.get("b", &stamp), None);
    assert_eq!(
        cache.get(
            "a",
            &Stamp {
                size: 43,
                ..stamp
            }
        ),
        None
    );
    assert_eq!(
        cache.get(
            "a",
            &Stamp {
                modified_nanos: 98,
                ..stamp
            }
        ),
        None
    );
}

#[test]
fn recorded_selection_preserves_order_and_ignores_unrequested_layers() {
    let selected = select_recorded_paths(
        "DLC",
        ["b", "a"].into_iter(),
        vec![
            ("a".into(), PathBuf::from("dlc/a/a.dlc")),
            ("extra".into(), PathBuf::from("dlc/extra/first.dlc")),
            ("b".into(), PathBuf::from("dlc/b/b.dlc")),
            ("extra".into(), PathBuf::from("dlc/extra/second.dlc")),
        ],
    )
    .unwrap();
    assert_eq!(
        selected,
        [PathBuf::from("dlc/b/b.dlc"), PathBuf::from("dlc/a/a.dlc")]
    );
    let missing = select_recorded_paths("DLC", ["missing"].into_iter(), Vec::new()).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    assert!(missing.to_string().contains("DLC missing"));
    let ambiguous = select_recorded_paths(
        "Mod",
        ["123"].into_iter(),
        vec![
            ("123".into(), PathBuf::from("mod/one.mod")),
            ("123".into(), PathBuf::from("mod/two.mod")),
        ],
    )
    .unwrap_err();
    assert!(ambiguous.to_string().contains("Mod 123"));
    assert!(ambiguous.to_string().contains("multiple"));
}

#[test]
fn remote_mod_ids_take_precedence_over_local_descriptor_names() {
    let remote = parser::parse(b"remote_file_id=123 name=Example").unwrap();
    let empty = parser::parse(b"remote_file_id=\"\" name=Example").unwrap();
    let local = parser::parse(b"name=Example").unwrap();
    let descriptor = Path::new("mod/example.mod");
    assert_eq!(mod_id(&remote, descriptor).unwrap(), "123");
    assert_eq!(mod_id(&empty, descriptor).unwrap(), "example.mod");
    assert_eq!(mod_id(&local, descriptor).unwrap(), "example.mod");
    assert!(
        select_recorded_paths(
            "Mod",
            ["example.mod"].into_iter(),
            vec![(mod_id(&remote, descriptor).unwrap(), descriptor.to_owned())],
        )
        .is_err()
    );
}

#[test]
fn listed_paths_keep_case_variants_and_honour_replace_path() {
    let layer = |id: &str, replace: &[&str], files: &[&str]| {
        let mut layer = folder_layer(
            id,
            Path::new(id),
            replace.iter().map(|p| p.to_string()).collect(),
        );
        for path in files {
            std::sync::Arc::make_mut(&mut layer.files).insert(
                path.as_bytes().to_vec(),
                Member {
                    archive_entry: None,
                    size: 0,
                    stamp: None,
                },
            );
        }
        layer
    };
    let layers = ResolvedLayers {
        layers: vec![
            layer(
                "base",
                &[],
                &["common/defines/00_defines.lua", "common/defines/sub/a.lua"],
            ),
            layer(
                "mod",
                &["common/defines/sub"],
                &[
                    "common/defines/00_Defines.lua",
                    "common/defines/00_defines.lua",
                ],
            ),
        ],
        ..ResolvedLayers::default()
    };
    assert_eq!(
        layers
            .listed_paths("common/defines/")
            .into_iter()
            .collect::<Vec<_>>(),
        [
            "common/defines/00_Defines.lua",
            "common/defines/00_defines.lua"
        ]
    );
}

#[test]
fn uncovered_reads_follow_load_order_replacement_and_folder_case() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_uncovered_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, over) = (root.join("base"), root.join("over"));
    fs::create_dir_all(base.join("gfx/World")).unwrap();
    fs::create_dir_all(over.join("gfx/world")).unwrap();
    fs::write(base.join("gfx/World/Hoi.fnt"), b"base").unwrap();
    fs::write(over.join("gfx/world/hoi.fnt"), b"over").unwrap();

    assert_eq!(
        folded_path(&base, "gfx/world/hoi.fnt").unwrap(),
        Some(base.join("gfx").join("World").join("Hoi.fnt"))
    );
    assert_eq!(folded_path(&base, "gfx/world/absent.fnt").unwrap(), None);
    assert_eq!(folded_path(&base, "no/such/dir.fnt").unwrap(), None);

    let mut layers = ResolvedLayers {
        layers: vec![
            folder_layer("base", &base, Vec::new()),
            folder_layer("over", &over, Vec::new()),
        ],
        coverage: CoveragePolicy::default(),
        effective: Arc::new(BTreeMap::new()),
        uncovered_reads: Default::default(),
        context: [0; 32],
        recorded_projection: Default::default(),
        generation: Default::default(),
        derived: Default::default(),
        read_trace: None,
        above_mods: 0,
        presentation: None,
        presentation_defines: None,
        defines_visible: false,
        overlay_reads: Default::default(),
    };
    assert_eq!(layers.read_uncovered("gfx/world/hoi.fnt").unwrap(), b"over");
    assert!(!layers.uncovered_from_base("gfx/world/hoi.fnt").unwrap());
    layers.layers.pop();
    assert_eq!(layers.read_uncovered("gfx/world/hoi.fnt").unwrap(), b"base");
    assert!(layers.uncovered_from_base("gfx/world/hoi.fnt").unwrap());

    let replacing = ResolvedLayers {
        layers: vec![
            folder_layer("base", &base, Vec::new()),
            folder_layer("over", &root.join("empty"), vec!["gfx/world".into()]),
        ],
        coverage: CoveragePolicy::default(),
        effective: Arc::new(BTreeMap::new()),
        uncovered_reads: Default::default(),
        context: [0; 32],
        recorded_projection: Default::default(),
        generation: Default::default(),
        derived: Default::default(),
        read_trace: None,
        above_mods: 0,
        presentation: None,
        presentation_defines: None,
        defines_visible: false,
        overlay_reads: Default::default(),
    };
    assert_eq!(
        replacing
            .read_uncovered("gfx/world/hoi.fnt")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(!replacing.uncovered_from_base("gfx/world/hoi.fnt").unwrap());

    assert_eq!(
        layers
            .read_uncovered("map/provinces.bmp")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(layers.read_uncovered("../escape").is_err());
    assert!(layers.read_uncovered("gfx/world/absent.fnt").is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn descriptor_paths_list_nested_files_in_layer_order_and_honour_replace_path() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_descriptors_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, over, replacing) = (root.join("base"), root.join("over"), root.join("rep"));
    for (dir, files) in [
        (
            &base,
            &[
                "gfx/Particles/a.asset",
                "gfx/particles/deep/b.ASSET",
                "gfx/particles/c.txt",
            ][..],
        ),
        (
            &over,
            &["gfx/particles/A.asset", "gfx/particles/z.asset"][..],
        ),
        (
            &replacing,
            &["gfx/particles/new/n.asset", "interface/w.gfx"][..],
        ),
    ] {
        for file in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
    }
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_layer("base", &base, Vec::new()),
            folder_layer("over", &over, Vec::new()),
        ],
        ..Default::default()
    };
    assert_eq!(
        layers
            .descriptor_paths("gfx/particles", &[".asset"])
            .unwrap(),
        [
            "gfx/Particles/deep/b.ASSET",
            "gfx/Particles/a.asset",
            "gfx/particles/A.asset",
            "gfx/particles/z.asset"
        ]
    );
    layers.layers.push(folder_layer(
        "rep",
        &replacing,
        vec!["gfx/particles".into()],
    ));
    assert_eq!(
        layers
            .descriptor_paths("gfx/particles", &[".asset", ".txt"])
            .unwrap(),
        ["gfx/particles/new/n.asset"]
    );
    assert!(covered("interface/w.gfx"));
    assert_eq!(
        layers.descriptor_paths("interface", &[".gfx"]).unwrap(),
        ["interface/w.gfx"]
    );
    assert!(layers.descriptor_paths("gfx", &["asset"]).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(feature = "test-support")]
#[test]
fn folder_fixtures_replace_directories_as_descriptors_do() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_from_folders_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, over) = (root.join("base"), root.join("over"));
    for (dir, file) in [
        (&base, "common/units/a.txt"),
        (&base, "common/units/deep/b.txt"),
        (&base, "common/other.txt"),
        (&over, "common/units/c.txt"),
    ] {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, file).unwrap();
    }
    for replace in ["common/units", "common/units/", "common\\units\\"] {
        let layers =
            ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[replace][..])]).unwrap();
        assert_eq!(
            layers.paths().unwrap().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                b"common/other.txt".as_slice(),
                b"common/units/c.txt".as_slice()
            ]),
            "{replace}"
        );
        assert_eq!(
            layers.read("common/units/c.txt").unwrap(),
            b"common/units/c.txt"
        );
        assert_eq!(
            layers.layers[1].replace_paths,
            [b"common/units"],
            "{replace}"
        );
    }
    assert!(ResolvedLayers::from_folders(&[(&base, &["../common"][..])]).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn archive_members_keep_their_raw_name_bytes() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_raw_zip_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("base/gfx/interface")).unwrap();
    fs::write(root.join("base/gfx/interface/kept.dds"), b"base kept").unwrap();
    fs::write(root.join("base/gfx/interface/hidden.dds"), b"base hidden").unwrap();
    let low = root.join("low.zip");
    let high = root.join("high.zip");
    fs::write(
        &low,
        raw_zip(&[
            (b"gfx/\x80.dds", 0, b"low raw"),
            (b"gfx/interface/\x81.dds", 0, b"low covered"),
            (b"gfx\\world\\a.fnt", 0, b"fat"),
            (b"gfx\\world\\b.fnt", 3, b"unix"),
        ]),
    )
    .unwrap();
    fs::write(&high, raw_zip(&[(b"gfx/\x80.dds", 0, b"high raw")])).unwrap();
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_layer("base", &root.join("base"), Vec::new()),
            archive_layer("low", &low, &[]),
        ],
        ..ResolvedLayers::default()
    };
    layers.layers[0].inventory().unwrap();
    layers.layers[0]
        .hash(&mut HashCache::default(), &mut HashStats::default())
        .unwrap();
    layers.finish().unwrap();
    assert_eq!(
        layers.read_uncovered_path(b"gfx/\x80.dds").unwrap(),
        b"low raw"
    );
    assert_eq!(
        layers.read_uncovered_path(b"gfx/\x80.dds").unwrap(),
        b"low raw"
    );
    assert_eq!(
        layers.read_path(b"gfx/interface/\x81.dds").unwrap(),
        b"low covered"
    );
    for lossy in ["gfx/\u{C7}.dds", "gfx/\u{FFFD}.dds"] {
        let error = layers.read_uncovered(lossy).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{lossy}");
    }
    assert_eq!(layers.read_uncovered("gfx/world/a.fnt").unwrap(), b"fat");
    let error = layers.read_uncovered("gfx/world/b.fnt").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(
        layers
            .effective
            .contains_key(b"gfx/interface/\x81.dds".as_slice())
    );
    assert!(
        layers
            .paths()
            .unwrap()
            .any(|path| path == b"gfx/interface/\x81.dds".as_slice())
    );
    layers
        .layers
        .push(archive_layer("high", &high, &[b"gfx\\interface\\"]));
    layers.finish().unwrap();
    assert_eq!(layers.layers[2].replace_paths, [b"gfx/interface"]);
    assert_eq!(
        layers.read_uncovered_path(b"gfx/\x80.dds").unwrap(),
        b"high raw"
    );
    assert_eq!(
        layers
            .read_path(b"gfx/interface/kept.dds")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn records_whose_names_decode_alike_stay_apart() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_collide_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let record = |name, flags, data| Record {
        name,
        host: 0,
        flags,
        deflate: false,
        data,
    };
    let cp437: &[u8] = b"gfx/interface/\x80.dds";
    let utf8: &[u8] = b"gfx/interface/\xC3\x87.dds";
    let (cp437_font, utf8_font): (&[u8], &[u8]) =
        (b"gfx/world/\x80.fnt", b"gfx/world/\xC3\x87.fnt");
    for (order, flipped) in [("first", false), ("second", true)] {
        let mut records = vec![
            record(cp437, 0, b"cp437"),
            record(utf8, 0x800, b"utf8"),
            record(cp437_font, 0, b"cp437 font"),
            record(utf8_font, 0x800, b"utf8 font"),
        ];
        if flipped {
            records.reverse();
        }
        let archive = root.join(format!("{order}.zip"));
        fs::write(&archive, zip_records(&[], &records)).unwrap();
        let layers = ResolvedLayers {
            layers: vec![archive_layer(order, &archive, &[])],
            ..ResolvedLayers::default()
        };
        let layer = &layers.layers[0];
        assert_eq!(
            layer.files.keys().collect::<Vec<_>>(),
            [&cp437.to_vec(), &utf8.to_vec()],
            "{order}"
        );
        for (name, data) in [(cp437, &b"cp437"[..]), (utf8, b"utf8")] {
            let hash: [u8; 32] = Sha256::digest(data).into();
            assert_eq!(layer.hashes.get(name), Some(&hash), "{order}");
        }
        let mut layers = layers;
        layers.finish().unwrap();
        assert_eq!(layers.read_path(cp437).unwrap(), b"cp437", "{order}");
        assert_eq!(layers.read_path(utf8).unwrap(), b"utf8", "{order}");
        assert_eq!(
            layers.listed_layer_paths(b"gfx/interface/").unwrap(),
            BTreeSet::from([cp437.to_vec(), utf8.to_vec()]),
            "{order}"
        );
        assert_eq!(
            layers.read_uncovered_path(cp437_font).unwrap(),
            b"cp437 font",
            "{order}"
        );
        assert_eq!(
            layers.read_uncovered_path(utf8_font).unwrap(),
            b"utf8 font",
            "{order}"
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn archive_tree_preserves_registration_rules() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_tree_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let record = |name, deflate, data| Record {
        name,
        host: 3,
        flags: 0,
        deflate,
        data,
    };
    let mount = |name: &str, prefix: &[u8], records: &[Record]| {
        let path = root.join(name);
        fs::write(&path, zip_records(prefix, records)).unwrap();
        physfs_zip::Archive::open(&path)
            .unwrap()
            .mount()
            .map(|mounted| mounted.entries)
    };
    let twice = mount(
        "twice.zip",
        &[],
        &[
            record(b"gfx/interface/a.dds", false, b"one"),
            record(b"gfx/interface/a.dds", false, b"two"),
        ],
    );
    assert!(twice.unwrap_err().to_string().contains("two records name"));
    let both = mount(
        "both.zip",
        &[],
        &[
            record(b"gfx/interface/", false, b""),
            record(b"gfx/interface", false, b"file"),
        ],
    );
    assert!(both.unwrap_err().to_string().contains("two records name"));
    let below = mount(
        "below.zip",
        &[],
        &[
            record(b"gfx/a", false, b"file"),
            record(b"gfx/a/b.dds", false, b"below"),
        ],
    );
    assert!(below.unwrap_err().to_string().contains("a file"));
    let text = b"spriteTypes = { spriteType = { name = \"GFX_a\" } } ".repeat(40);
    let entries = mount(
        "stub.zip",
        b"MZ stub",
        &[
            record(b"gfx/a/b.dds", false, b"below"),
            record(b"gfx/a", false, b"file"),
            record(b"interface/a.gfx", true, &text),
        ],
    )
    .unwrap();
    let directories: Vec<_> = entries.iter().map(|entry| entry.directory).collect();
    assert_eq!(directories, [false, true, false]);
    let mut archive = physfs_zip::Archive::open(&root.join("stub.zip")).unwrap();
    assert_eq!(archive.read(&entries[0], None).unwrap().unwrap(), b"below");
    assert_eq!(archive.read(&entries[2], None).unwrap().unwrap(), text);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn records_the_game_does_not_open_pass_to_the_layers_below() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_unopened_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let record = |name, data| Record {
        name,
        host: 0,
        flags: 0,
        deflate: false,
        data,
    };
    let (covered, uncovered): (&[u8], &[u8]) = (b"gfx/interface/a.dds", b"gfx/world/f.fnt");
    let mut bytes = zip_records(
        &[],
        &[record(covered, b"mod a"), record(uncovered, b"mod f")],
    );
    for local in [0, 30 + covered.len() + 5] {
        bytes[local + 8] = 8;
    }
    let archive = root.join("broken.zip");
    fs::write(&archive, bytes).unwrap();
    let encrypted = root.join("encrypted.zip");
    fs::write(&encrypted, include_bytes!("testdata/zipcrypto_stored.zip")).unwrap();
    let base = folder_of(
        "base",
        &root.join("base"),
        &[
            ("gfx/interface/a.dds", b"base a"),
            ("gfx/world/f.fnt", b"base f"),
            ("gfx/interface/e.dds", b"base e"),
        ],
    );
    let mut layers = ResolvedLayers {
        layers: vec![
            base,
            archive_layer("broken", &archive, &[]),
            archive_layer("encrypted", &encrypted, &[]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert_eq!(layers.read_path(covered).unwrap(), b"base a");
    assert_eq!(layers.read_uncovered_path(uncovered).unwrap(), b"base f");
    assert_eq!(layers.read_path(b"gfx/interface/e.dds").unwrap(), b"base e");
    let listed = layers.listed_layer_paths(b"gfx/interface/").unwrap();
    assert!(listed.contains(covered) && listed.contains(b"gfx/interface/e.dds".as_slice()));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_backslash_is_part_of_a_record_name_and_fails_a_request() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_backslash_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let record = |name, host, data| Record {
        name,
        host,
        flags: 0,
        deflate: false,
        data,
    };
    let archive = root.join("unix.zip");
    fs::write(
        &archive,
        zip_records(
            &[],
            &[
                record(b"gfx\\interface\\b.dds", 3, b"unix b"),
                record(b"gfx\\world\\c.fnt", 3, b"unix c"),
                record(b"gfx\\interface\\fat.dds", 0, b"fat"),
            ],
        ),
    )
    .unwrap();
    let base = folder_of(
        "base",
        &root.join("base"),
        &[
            ("gfx/interface/b.dds", b"base b"),
            ("gfx/world/c.fnt", b"base c"),
        ],
    );
    let mut layers = ResolvedLayers {
        layers: vec![base, archive_layer("unix", &archive, &[])],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert_eq!(
        layers.descriptor_paths("gfx/world", &[".fnt"]).unwrap(),
        ["gfx/world/c.fnt"]
    );
    assert_eq!(layers.read_path(b"gfx/interface/b.dds").unwrap(), b"base b");
    assert_eq!(
        layers.read_uncovered_path(b"gfx/world/c.fnt").unwrap(),
        b"base c"
    );
    assert_eq!(layers.read_path(b"gfx/interface/fat.dds").unwrap(), b"fat");
    for request in [
        &b"gfx\\interface\\b.dds"[..],
        b"gfx\\world\\c.fnt",
        b"gfx:x.dds",
    ] {
        let error = layers.read_uncovered_path(request).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
    assert_eq!(
        layers.listed_layer_paths(b"gfx/interface/").unwrap(),
        BTreeSet::from([
            b"gfx/interface/b.dds".to_vec(),
            b"gfx/interface/fat.dds".to_vec()
        ])
    );
    assert_eq!(
        layers.read_path(b"//gfx//interface/fat.dds").unwrap(),
        b"fat"
    );
    fs::remove_dir_all(&root).unwrap();
}

fn uncovered_members(vector: &[u8]) -> Vec<u8> {
    let mut bytes = vector.to_vec();
    for at in 0..bytes.len() {
        if bytes[at..].starts_with(b"gfx/fonts/") {
            bytes[at..at + 10].copy_from_slice(b"gfx/world/");
        }
    }
    bytes
}

#[test]
fn a_dollar_opens_an_encrypted_record_with_its_password() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_password_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let (stored, deflated) = (root.join("stored.zip"), root.join("deflated.zip"));
    fs::write(&stored, include_bytes!("testdata/zipcrypto_stored.zip")).unwrap();
    fs::write(
        &deflated,
        uncovered_members(include_bytes!("testdata/zipcrypto_deflated.zip")),
    )
    .unwrap();
    let base = folder_of(
        "base",
        &root.join("base"),
        &[("gfx/interface/e.dds$wrong", b"base wrong")],
    );
    let mut layers = ResolvedLayers {
        layers: vec![
            base,
            archive_layer("stored", &stored, &[]),
            archive_layer("deflated", &deflated, &[]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert_eq!(
        layers.read_path(b"gfx/interface/e.dds$secret").unwrap(),
        b"encrypted stored texture"
    );
    assert_eq!(
        layers
            .read_uncovered_path(b"gfx/world/x.fnt$secret")
            .unwrap(),
        b"info face=\"x\" size=12\n".repeat(20)
    );
    assert_eq!(
        layers.read_path(b"gfx/interface/e.dds$wrong").unwrap(),
        b"base wrong"
    );
    assert_eq!(
        layers
            .read_uncovered_path(b"gfx/world/x.fnt$wrong")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn central_password_metadata_is_part_of_an_encrypted_records_identity() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_central_time_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let original = include_bytes!("testdata/zipcrypto_stored.zip");
    let central = original
        .windows(4)
        .position(|word| word == b"PK\x01\x02")
        .unwrap();
    let variant = |name: &str, off: u8| {
        let mut bytes = original.to_vec();
        bytes[central + 8] |= 8;
        bytes[central + 13] = bytes[central + 19] ^ off;
        let path = root.join(name);
        fs::write(&path, bytes).unwrap();
        archive_layer(name, &path, &[])
    };
    let (opens, refuses) = (variant("opens.zip", 0), variant("refuses.zip", 1));
    let name: &[u8] = b"gfx/interface/e.dds";
    assert_ne!(opens.hashes.get(name), refuses.hashes.get(name));
    let request: &[u8] = b"gfx/interface/e.dds$secret";
    for (layer, expected) in [
        (opens, &b"encrypted stored texture"[..]),
        (refuses, b"base"),
    ] {
        let base = folder_of(
            "base",
            &root.join("base"),
            &[("gfx/interface/e.dds$secret", b"base")],
        );
        let mut layers = ResolvedLayers {
            layers: vec![base, layer],
            ..ResolvedLayers::default()
        };
        layers.finish().unwrap();
        assert_eq!(layers.read_path(request).unwrap(), expected);
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_password_for_a_plain_record_passes_to_the_layers_below() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_mixed_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let mixed = root.join("mixed.zip");
    fs::write(
        &mixed,
        uncovered_members(include_bytes!("testdata/zipcrypto_mixed.zip")),
    )
    .unwrap();
    let base = folder_of(
        "base",
        &root.join("base"),
        &[
            ("gfx/interface/plain.dds$secret", b"base plain"),
            ("gfx/world/p.fnt$secret", b"base font"),
        ],
    );
    let mut layers = ResolvedLayers {
        layers: vec![base, archive_layer("mixed", &mixed, &[])],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert_eq!(
        layers.read_path(b"gfx/interface/plain.dds$secret").unwrap(),
        b"base plain"
    );
    assert_eq!(
        layers
            .read_uncovered_path(b"gfx/world/p.fnt$secret")
            .unwrap(),
        b"base font"
    );
    assert_eq!(
        layers.read_path(b"gfx/interface/plain.dds").unwrap(),
        b"plain covered texture"
    );
    assert_eq!(
        layers.read_path(b"gfx/interface/e.dds$secret").unwrap(),
        b"encrypted stored texture"
    );
    fs::remove_dir_all(&root).unwrap();
}

fn flagged_encrypted(opens: bool) -> Vec<u8> {
    let mut bytes = include_bytes!("testdata/zipcrypto_stored.zip").to_vec();
    let central = bytes
        .windows(4)
        .position(|word| word == b"PK\x01\x02")
        .unwrap();
    bytes[central + 8] |= 8;
    bytes[central + 13] = bytes[central + 19] ^ u8::from(!opens);
    bytes
}

#[test]
fn a_password_that_stops_opening_after_verification_is_a_change() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_changed_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("encrypted.zip");
    fs::write(&archive, flagged_encrypted(true)).unwrap();
    let base = folder_of(
        "base",
        &root.join("base"),
        &[("gfx/interface/e.dds$secret", b"base")],
    );
    let mut layers = ResolvedLayers {
        layers: vec![base, archive_layer("encrypted", &archive, &[])],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    let request: &[u8] = b"gfx/interface/e.dds$secret";
    assert_eq!(
        layers.read_path(request).unwrap(),
        b"encrypted stored texture"
    );
    fs::write(&archive, flagged_encrypted(false)).unwrap();
    let error = layers.read_path(request).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error.to_string().contains("changed since verification"),
        "{error}"
    );
    layers.layers[1].archive_stamp = Some(Stamp::read(&archive).unwrap());
    let error = layers.read_path(request).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("content file changed since verification"),
        "{error}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_cache_written_by_an_earlier_digest_scheme_is_not_served() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_cache_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("encrypted.zip");
    fs::write(&path, flagged_encrypted(true)).unwrap();
    let layer = |cache: &mut HashCache| {
        let mut layer = ResolvedLayer {
            content_digest: [0; 32],
            inventory_witnesses: Default::default(),
            inventory_paths: Default::default(),
            coverage: CoveragePolicy::default(),
            role: LayerRole::Mod,
            facts: None,
            id: "encrypted".into(),
            source: LayerSource::Archive(path.clone()),
            descriptor: None,
            replace_paths: Vec::new(),
            files: Default::default(),
            lookup: Default::default(),
            hashes: Default::default(),
            archive_stamp: None,
            descriptor_hash: None,
            overlay: None,
            display_name: None,
            art_unmounted: None,
        };
        layer.inventory().unwrap();
        layer.hash(cache, &mut HashStats::default()).unwrap()
    };
    let fresh = layer(&mut HashCache::default());
    let mut archive = physfs_zip::Archive::open(&path).unwrap();
    let entry = archive.mount().unwrap().entries.remove(0);
    let mut earlier = Sha256::new();
    earlier.update(b"\0unopened archive record\0");
    earlier.update(archive.stored(&entry).unwrap());
    let mut cache = HashCache::default();
    cache.insert(
        format!("{}\0gfx/interface/e.dds", text_path(&path).unwrap()),
        Stamp::read(&path).unwrap(),
        earlier.finalize().into(),
    );
    assert_eq!(layer(&mut cache), fresh);
    assert_eq!(layer(&mut cache), fresh);
    fs::remove_dir_all(&root).unwrap();
}

fn covered_encrypted_descriptor() -> Vec<u8> {
    let original = include_bytes!("testdata/zipcrypto_stored.zip");
    let (from, to): (&[u8], &[u8]) = (b"gfx/interface/e.dds", b"gfx/descriptor1.gfx");
    let mut bytes = original.to_vec();
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(from.len()).position(|w| w == from) {
        bytes[at + found..at + found + from.len()].copy_from_slice(to);
        at += found + from.len();
    }
    bytes
}

fn password_stack(root: &Path, path: &Path) -> ResolvedLayers {
    let base = folder_of(
        "base",
        &root.join("base"),
        &[
            ("gfx/descriptor1.gfx$secret", b"base"),
            ("gfx/descriptor1.gfx$wrong", b"base wrong"),
        ],
    );
    let mut layers = ResolvedLayers {
        layers: vec![base, archive_layer("encrypted", path, &[])],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    layers
}

#[test]
fn a_memoized_password_read_of_a_changed_payload_is_a_change() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_memo_changed_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("encrypted.zip");
    fs::write(&path, covered_encrypted_descriptor()).unwrap();
    let mut layers = password_stack(&root, &path);
    let request: &[u8] = b"gfx/descriptor1.gfx$secret";
    assert!(!covered_path(request) && covered_path(b"gfx/descriptor1.gfx"));
    for _ in 0..2 {
        assert_eq!(
            layers.read_uncovered_path(request).unwrap(),
            b"encrypted stored texture"
        );
    }
    let mut bytes = covered_encrypted_descriptor();
    let central = bytes
        .windows(4)
        .position(|word| word == b"PK\x01\x02")
        .unwrap();
    bytes[central - 1] ^= 0x55;
    fs::write(&path, bytes).unwrap();
    let error = layers.read_uncovered_path(request).unwrap_err();
    assert!(
        error.to_string().contains("changed since verification"),
        "{error}"
    );
    layers.layers[1].archive_stamp = Some(Stamp::read(&path).unwrap());
    let error = layers.read_uncovered_path(request).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("content file changed since verification"),
        "{error}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn verified_encrypted_archive_reads_verify_and_decrypt_in_a_single_read() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_single_read_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let stored = root.join("stored.zip");
    let deflated = root.join("deflated.zip");
    let mixed = root.join("mixed.zip");
    fs::write(&stored, covered_encrypted_descriptor()).unwrap();
    fs::write(&deflated, include_bytes!("testdata/zipcrypto_deflated.zip")).unwrap();
    fs::write(&mixed, include_bytes!("testdata/zipcrypto_mixed.zip")).unwrap();
    let base = folder_of(
        "base",
        &root.join("base"),
        &[("gfx/interface/plain.dds$secret", b"lower plain")],
    );
    let mut layers = ResolvedLayers {
        layers: vec![
            base,
            archive_layer("stored", &stored, &[]),
            archive_layer("deflated", &deflated, &[]),
            archive_layer("mixed", &mixed, &[]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert!(covered_path(b"gfx/descriptor1.gfx"));
    assert!(covered_path(b"gfx/fonts/x.fnt"));
    assert!(covered_path(b"gfx/interface/plain.dds"));
    LOADS.with(|loads| loads.set(0));
    assert_eq!(
        layers
            .read_uncovered_path(b"gfx/descriptor1.gfx$secret")
            .unwrap(),
        b"encrypted stored texture"
    );
    assert_eq!(LOADS.with(|loads| loads.get()), 1);
    LOADS.with(|loads| loads.set(0));
    assert_eq!(
        layers.read_uncovered_path(b"gfx/fonts/x.fnt$secret").unwrap(),
        b"info face=\"x\" size=12\n".repeat(20)
    );
    assert_eq!(LOADS.with(|loads| loads.get()), 1);
    LOADS.with(|loads| loads.set(0));
    assert_eq!(
        layers.layers[3]
            .archive_open(b"gfx/interface/plain.dds$secret", &mut ReadObservation::default())
            .unwrap(),
        None
    );
    assert_eq!(LOADS.with(|loads| loads.get()), 1);
    assert_eq!(
        layers.read_path(b"gfx/interface/plain.dds$secret").unwrap(),
        b"lower plain"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn uncovered_password_probe_refuses_a_changed_covered_archive_opening() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_probe_opening_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("encrypted.zip");
    fs::write(&path, covered_encrypted_descriptor()).unwrap();
    let mut layers = password_stack(&root, &path);
    LOADS.with(|loads| loads.set(0));
    assert!(layers.has_file("gfx/descriptor1.gfx$secret").unwrap());
    assert_eq!(LOADS.with(|loads| loads.get()), 0);
    let mut header_tampered = covered_encrypted_descriptor();
    let data_offset = 30 + b"gfx/descriptor1.gfx".len();
    header_tampered[data_offset] ^= 0xFF;
    fs::write(&path, &header_tampered).unwrap();
    layers.layers[1].archive_stamp = Some(Stamp::read(&path).unwrap());
    let error = layers
        .has_file("gfx/descriptor1.gfx$secret")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("content file changed since verification"),
        "{error}"
    );
    assert_eq!(LOADS.with(|loads| loads.get()), 0);
    let mut sig_tampered = covered_encrypted_descriptor();
    sig_tampered[0] ^= 0xFF;
    fs::write(&path, &sig_tampered).unwrap();
    layers.layers[1].archive_stamp = Some(Stamp::read(&path).unwrap());
    let error = layers
        .has_file("gfx/descriptor1.gfx$secret")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("content file changed since verification"),
        "{error}"
    );
    assert_eq!(LOADS.with(|loads| loads.get()), 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_password_read_of_a_removed_record_is_a_change() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_zip_removed_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("encrypted.zip");
    fs::write(&path, covered_encrypted_descriptor()).unwrap();
    let mut layers = password_stack(&root, &path);
    let missing: &[u8] = b"gfx/descriptor1.gfx$other";
    let lower_hit: &[u8] = b"gfx/descriptor1.gfx$wrong";
    for _ in 0..2 {
        let error = layers.read_uncovered_path(missing).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            layers.read_uncovered_path(lower_hit).unwrap(),
            b"base wrong"
        );
    }
    let other = Record {
        name: b"gfx/world/other.fnt",
        host: 0,
        flags: 0,
        deflate: false,
        data: b"other",
    };
    fs::write(&path, zip_records(&[], &[other])).unwrap();
    for request in [&b"gfx/descriptor1.gfx$secret"[..], missing, lower_hit] {
        let error = layers.read_uncovered_path(request).unwrap_err();
        assert!(
            error.to_string().contains("changed since verification"),
            "{error}"
        );
    }
    layers.layers[1].archive_stamp = Some(Stamp::read(&path).unwrap());
    for request in [&b"gfx/descriptor1.gfx$secret"[..], missing, lower_hit] {
        let error = layers.read_uncovered_path(request).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("content file changed since verification: gfx/descriptor1.gfx"),
            "{error}"
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_covered_folder_file_gone_or_appearing_after_verification_is_a_change() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_folder_changed_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let (base, top) = (root.join("base"), root.join("top"));
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_of("base", &base, &[("common/a.txt", b"base a")]),
            folder_of("top", &top, &[("common/b.txt", b"top b")]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    let assert_changed = |error: io::Error, path: &str| {
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        assert!(
            error
                .to_string()
                .contains(&format!("content file changed since verification: {path}")),
            "{error}"
        );
    };
    assert_eq!(layers.read("common/a.txt").unwrap(), b"base a");
    fs::write(top.join("common/a.txt"), b"top a").unwrap();
    assert_changed(layers.read("common/a.txt").unwrap_err(), "common/a.txt");
    fs::remove_file(top.join("common/a.txt")).unwrap();
    assert_eq!(layers.read("common/a.txt").unwrap(), b"base a");
    let error = layers.read("common/c.txt").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    fs::write(base.join("common/c.txt"), b"base c").unwrap();
    assert_changed(layers.read("common/c.txt").unwrap_err(), "common/c.txt");
    fs::remove_file(base.join("common/a.txt")).unwrap();
    assert_changed(layers.read("common/a.txt").unwrap_err(), "common/a.txt");
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(windows)]
pub(super) fn junction(link: &Path, target: &Path) {
    let native = |path: &Path| path.components().collect::<PathBuf>();
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(native(link))
        .arg(native(target))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J {} {}: {}{}",
        link.display(),
        target.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_tree(root: &Path, files: &[(&str, &[u8])]) {
    for (path, data) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
}

#[test]
fn a_visual_mods_symbolic_links_are_listed_as_files_and_never_entered() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_symbolic_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    write_tree(
        &root,
        &[
            ("gfx/interface/a.dds", b"a"),
            ("gfx/interface/to_file.dds", b"file"),
            ("gfx/interface/to_folder/inner.dds", b"inner"),
        ],
    );
    let root = fs::canonicalize(&root).unwrap();
    let kind = |entry: &fs::DirEntry| match entry.file_name().to_str() {
        Some("to_file.dds" | "to_folder") => Ok(Listed::LinkedFile),
        _ => entry_kind(entry),
    };
    let walk = |overlay| -> io::Result<Vec<String>> {
        let mut listed = Vec::new();
        walk_folder(
            &root,
            &root.join("gfx"),
            overlay,
            &kind,
            &mut |relative, _| {
                listed.push(String::from_utf8(relative).unwrap());
                Ok(())
            },
        )?;
        Ok(listed)
    };
    assert_eq!(
        walk(true).unwrap(),
        [
            "gfx/interface/a.dds",
            "gfx/interface/to_file.dds",
            "gfx/interface/to_folder"
        ]
    );
    let error = walk(false).unwrap_err().to_string();
    assert!(
        error.contains("content layer contains a symbolic link"),
        "{error}"
    );
    let mount = |links: &[&str]| -> io::Result<Vec<String>> {
        let kind = |entry: &fs::DirEntry| match entry.file_name().to_str() {
            Some(name) if links.contains(&name) => Ok(Listed::LinkedFile),
            _ => entry_kind(entry),
        };
        let mut files = BTreeMap::new();
        let gfx = root.join("gfx");
        collect_folder_with(
            &root,
            &gfx,
            &mut files,
            CoveragePolicy::default(),
            true,
            &kind,
            &mut BTreeSet::new(),
        )?;
        Ok(files
            .into_keys()
            .map(|path| String::from_utf8(path).unwrap())
            .collect())
    };
    assert_eq!(
        mount(&["to_file.dds"]).unwrap(),
        [
            "gfx/interface/a.dds",
            "gfx/interface/to_file.dds",
            "gfx/interface/to_folder/inner.dds"
        ]
    );
    let error = mount(&["to_folder"]).unwrap_err().to_string();
    assert!(error.contains("names a folder"), "{error}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_unverified_visual_mods_link_to_no_file_passes_to_the_layers_below() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_link_passes_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, art) = (root.join("base"), root.join("art"));
    write_tree(&art, &[("gfx/interface/x.dds/inner.dds", b"inner")]);
    let base = folder_of("base", &base, &[("gfx/interface/x.dds", b"base")]);
    let mut overlay = folder_layer("art", &art, Vec::new());
    overlay.overlay = Some(Overlay {
        name: "Art".into(),
        verified: false,
    });
    let LayerSource::Folder(art) = overlay.source.clone() else {
        unreachable!()
    };
    let kind = |entry: &fs::DirEntry| match entry.file_name().to_str() {
        Some("x.dds") => Ok(Listed::LinkedFile),
        _ => entry_kind(entry),
    };
    let files = std::sync::Arc::make_mut(&mut overlay.files);
    walk_folder(&art, &art.join("gfx"), true, &kind, &mut |relative, _| {
        files.insert(
            relative,
            Member {
                archive_entry: None,
                size: 0,
                stamp: None,
            },
        );
        Ok(())
    })
    .unwrap();
    overlay.finish_inventory().unwrap();
    let mut layers = ResolvedLayers {
        layers: vec![base, overlay],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert!(
        layers
            .listed_paths("gfx/interface")
            .contains("gfx/interface/x.dds")
    );
    assert_eq!(layers.read("gfx/interface/x.dds").unwrap(), b"base");
    assert!(!layers.supplied_by_visual_mod("gfx/interface/x.dds"));
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(windows)]
#[test]
fn a_recorded_folder_layer_refuses_a_junction() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_recorded_junction_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (layer, outside) = (root.join("layer"), root.join("outside"));
    write_tree(&layer, &[("common/a.txt", b"a")]);
    write_tree(&outside, &[("b.txt", b"b")]);
    junction(&layer.join("common").join("linked"), &outside);
    let error = ResolvedLayers::from_folders(&[(layer.as_path(), &[][..])])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("content layer contains a symbolic link"),
        "{error}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_folder_aliases_verify_the_opened_file_including_memoized_hits() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_folder_alias_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let (base, top) = (root.join("base"), root.join("top"));
    let name = "gfx/Élementlongname.gfx";
    let unopened = "gfx/Unopenedlongname.gfx";
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_of(
                "base",
                &base,
                &[
                    (name, b"verified"),
                    (unopened, b"never opened"),
                    ("gfx/älternate.gfx", b"base"),
                ],
            ),
            folder_of("top", &top, &[("gfx/Älternate.gfx", b"top")]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert_eq!(layers.read("gfx/älternate.gfx").unwrap(), b"top");
    let short_name_of = |file_path: PathBuf| -> String {
        let wide: Vec<_> = file_path
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect();
        let mut short = vec![0; 32768];
        let length =
            unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), short.len() as u32) };
        assert!(length > 0 && (length as usize) < short.len());
        let short = PathBuf::from(std::ffi::OsString::from_wide(&short[..length as usize]));
        short.file_name().unwrap().to_str().unwrap().to_owned()
    };
    let mut aliases = vec![
        format!("{name}."),
        format!("{name} "),
        "gfx/éLEMENTLONGNAME.GFX".into(),
    ];
    assert!(!covered(&aliases[0]) && !covered(&aliases[1]));
    let short_name = short_name_of(base.join(name));
    if !short_name.eq_ignore_ascii_case("Élementlongname.gfx") {
        aliases.push(format!("gfx/{short_name}"));
    }
    let unopened_short = short_name_of(base.join(unopened));
    fs::remove_file(base.join(unopened)).unwrap();
    if !unopened_short.eq_ignore_ascii_case("Unopenedlongname.gfx") {
        let short_alias = format!("gfx/{unopened_short}");
        for error in [
            layers.read_uncovered(&short_alias).unwrap_err(),
            layers.read(&short_alias).unwrap_err(),
        ] {
            assert!(error.to_string().contains(unopened), "{short_alias}: {error}");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }
    for alias in &aliases {
        for _ in 0..2 {
            assert_eq!(
                layers.read_uncovered(alias).unwrap(),
                b"verified",
                "{alias}"
            );
            assert_eq!(layers.read(alias).unwrap(), b"verified", "{alias}");
        }
    }
    fs::write(base.join(name), b"modified").unwrap();
    for alias in &aliases {
        for error in [
            layers.read_uncovered(alias).unwrap_err(),
            layers.read(alias).unwrap_err(),
        ] {
            assert!(error.to_string().contains(name), "{alias}: {error}");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }
    fs::remove_file(base.join(name)).unwrap();
    for alias in &aliases {
        for error in [
            layers.read_uncovered(alias).unwrap_err(),
            layers.read(alias).unwrap_err(),
        ] {
            assert!(error.to_string().contains(name), "{alias}: {error}");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }
    fs::write(base.join(name), b"verified").unwrap();
    fs::write(top.join(name), b"appeared").unwrap();
    assert_eq!(
        layers.read_uncovered(&aliases[0]).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    fs::write(top.join("gfx/uncovered.fnt"), b"outside").unwrap();
    assert_eq!(
        layers.read_uncovered("gfx/uncovered.fnt").unwrap(),
        b"outside"
    );
    assert!(
        layers
            .read("gfx/uncovered.fnt")
            .unwrap_err()
            .to_string()
            .contains("this reader does not report uncovered reads")
    );
    let reads = layers.report_uncovered_reads();
    assert_eq!(layers.read("gfx/uncovered.fnt").unwrap(), b"outside");
    assert_eq!(
        reads.warning().unwrap(),
        format!(
            "Content: 1 file(s) read outside the recorded content coverage policy {COVERAGE_POLICY_VERSION}; these bytes are not hash-verified: gfx/uncovered.fnt."
        )
    );
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(windows)]
#[test]
fn short_name_enumeration_propagates_unexpected_win32_errors() {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES,
    };
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_short_enum_err_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let missing = root.join("missing");
    check_find_error(
        &missing,
        io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND as i32),
        ERROR_FILE_NOT_FOUND,
    )
    .unwrap();
    check_find_error(
        &missing,
        io::Error::from_raw_os_error(ERROR_NO_MORE_FILES as i32),
        ERROR_NO_MORE_FILES,
    )
    .unwrap();
    let denied = check_find_error(
        &missing,
        io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32),
        ERROR_NO_MORE_FILES,
    )
    .unwrap_err();
    assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        denied.to_string().contains(&missing.display().to_string()),
        "{denied}"
    );
    let mut files = BTreeMap::new();
    files.insert(
        b"sub/file.gfx".to_vec(),
        Member {
            archive_entry: None,
            size: 0,
            stamp: None,
        },
    );
    let error = collect_folder_short_names(&missing, &files).unwrap_err();
    assert!(
        error.to_string().contains(&missing.display().to_string()),
        "{error}"
    );
}

fn mod_order(mods: &[(&str, &str)]) -> (io::Result<Vec<String>>, Vec<String>) {
    let mut notes = Vec::new();
    let selected = mods
        .iter()
        .map(|(path, text)| {
            (
                path.to_string(),
                (),
                parser::parse(text.as_bytes()).unwrap(),
            )
        })
        .collect();
    let order = native_mod_order(selected, &mut notes)
        .map(|mods| mods.into_iter().map(|(path, ..)| path).collect());
    (order, notes)
}

#[test]
fn mods_mount_in_the_engines_dependency_then_path_order() {
    let (order, notes) = mod_order(&[
        ("mod/b.mod", r#"name="B""#),
        ("mod/a.mod", r#"name="A" dependencies={}"#),
        ("mod/B.mod", r#"name="C""#),
    ]);
    assert_eq!(order.unwrap(), ["mod/B.mod", "mod/a.mod", "mod/b.mod"]);
    assert!(notes.is_empty());
    let (order, _) = mod_order(&[
        (
            "mod/a.mod",
            r#"name="Top" dependencies={"Absent"} dependencies={"Left" "Right"}"#,
        ),
        ("mod/b.mod", r#"name="Left" dependencies={"Base"}"#),
        ("mod/c.mod", r#"name="Right" dependencies={"Base"}"#),
        ("mod/d.mod", r#"name="Base""#),
        ("mod/0.mod", r#"name="Alone""#),
    ]);
    assert_eq!(
        order.unwrap(),
        [
            "mod/d.mod",
            "mod/b.mod",
            "mod/c.mod",
            "mod/0.mod",
            "mod/a.mod"
        ]
    );
}

#[test]
fn a_new_recording_takes_the_engine_mod_order_and_a_replay_keeps_it() {
    use synthetic_game::{Dlc, Installation, Mod};
    let installed = Installation::new("mod-order")
        .file("common/file.txt", "vanilla")
        .dlc(Dlc::new("dlc_synthetic", "Synthetic DLC").file("common/file.txt", "DLC"))
        .with_mod(
            Mod::new("a_top", "Top")
                .remote_id(42)
                .depends_on("Base")
                .file("common/file.txt", "top"),
        )
        .with_mod(Mod::new("b_base", "Base").file("common/file.txt", "base"))
        .build();
    let write = |path: &str, text: &str| fs::write(installed.user_dir.join(path), text).unwrap();
    let (game_dir, user_dir, cache_dir) = (
        installed.game_dir.clone(),
        installed.user_dir.clone(),
        installed.cache_dir.clone(),
    );
    let gathered = gather(&GatherOptions {
        game_dir: game_dir.clone(),
        user_dir: user_dir.clone(),
        cache_dir: cache_dir.clone(),
        dlc_order: None,
    })
    .unwrap();
    assert_eq!(gathered.order_note, None);
    assert_eq!(gathered.mod_descriptors, ["b_base.mod", "a_top.mod"]);
    assert_eq!(gathered.layers.read("common/file.txt").unwrap(), b"top");
    let identity = gathered
        .identity(
            "build".into(),
            Sha256::digest(b"synthetic executable").into(),
            20,
            "test".into(),
            [0; 32],
        )
        .unwrap();
    let ids = identity.mods.iter().map(|layer| layer.id.as_str());
    assert_eq!(ids.collect::<Vec<_>>(), ["b_base.mod", "42"]);
    write("dlc_load.json", "not JSON");
    let options = ResolveOptions {
        game_dir,
        user_dir,
        cache_dir,
    };
    let resolved = resolve(&identity, &options).unwrap();
    let layers = resolved.layers.layers.iter().map(|layer| layer.id.as_str());
    assert!(layers.collect::<Vec<_>>().ends_with(&["b_base.mod", "42"]));
    assert_eq!(resolved.layers.read("common/file.txt").unwrap(), b"top");
    write("layers/b_base/common/file.txt", "changed");
    let changed = resolve(&identity, &options).unwrap();
    assert_eq!(changed.layers.read("common/file.txt").unwrap(), b"top");
    let names: Vec<_> = changed
        .changed
        .iter()
        .map(|layer| layer.id.as_str())
        .collect();
    assert_eq!(names, ["b_base.mod"]);
    assert_eq!(changed.changed[0].kind, "Mod");
    let mut executable = identity.clone();
    executable.exe_hash[0] ^= 1;
    let both = resolve(&executable, &options).unwrap();
    let names: Vec<_> = both
        .changed
        .iter()
        .map(|layer| format!("{} {}", layer.kind, layer.id))
        .collect();
    assert_eq!(names, ["Game hoi4.exe", "Mod b_base.mod"]);
    let error = changed.exact().unwrap_err().to_string();
    assert!(
        error.contains("Mod b_base.mod: content hash mismatch"),
        "{error}"
    );
}

#[test]
fn a_mod_installed_elsewhere_keeps_its_recorded_hash_and_a_changed_replace_path_does_not() {
    let root = std::env::temp_dir().join(format!("hoi4_mod_moved_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("game/hoi4.exe", "synthetic executable");
    write("game/common/file.txt", "vanilla");
    fs::create_dir_all(root.join("game/dlc")).unwrap();
    write("user/settings.txt", "language=l_english");
    write(
        "user/dlc_load.json",
        r#"{"enabled_mods":["mod/ugc_42.mod"],"disabled_dlcs":[]}"#,
    );
    write(
        "user/mod/ugc_42.mod",
        r#"name="Moved" remote_file_id=42 path="first/42" replace_path="common/ideas" dependencies={"Base"}"#,
    );
    write("user/first/42/common/file.txt", "mod");
    let (game_dir, user_dir, cache_dir) =
        (root.join("game"), root.join("user"), root.join("cache"));
    let identity = gather(&GatherOptions {
        game_dir: game_dir.clone(),
        user_dir: user_dir.clone(),
        cache_dir: cache_dir.clone(),
        dlc_order: None,
    })
    .unwrap()
    .identity(
        "build".into(),
        Sha256::digest(b"synthetic executable").into(),
        20,
        "test".into(),
        [0; 32],
    )
    .unwrap();
    let options = ResolveOptions {
        game_dir,
        user_dir,
        cache_dir,
    };
    fs::rename(root.join("user/first"), root.join("user/second")).unwrap();
    fs::remove_file(root.join("user/mod/ugc_42.mod")).unwrap();
    let moved = root.join("user/second/42");
    let moved = moved.to_str().unwrap().replace('\\', "/");
    write(
        "user/mod/42.mod",
        &format!(
            "remote_file_id=\"42\"\npath=\"{moved}\"\nname=\"Moved\"\ndependencies={{ \"Base\" }}\nreplace_path=\"common/ideas/\"\n"
        ),
    );
    let resolved = resolve(&identity, &options).unwrap();
    assert!(resolved.changed.is_empty(), "{:?}", resolved.changed);
    assert_eq!(resolved.layers.read("common/file.txt").unwrap(), b"mod");
    write(
        "user/mod/42.mod",
        &format!(
            "remote_file_id=\"42\"\npath=\"{moved}\"\nname=\"Moved\"\ndependencies={{ \"Base\" }}\nreplace_path=\"common/national_focus\"\n"
        ),
    );
    let changed = resolve(&identity, &options).unwrap();
    let names: Vec<_> = changed
        .changed
        .iter()
        .map(|layer| format!("{} {}", layer.kind, layer.id))
        .collect();
    assert_eq!(names, ["Mod 42"]);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn nested_dlc_resolves_where_it_was_gathered_and_nested_mods_are_left_out() {
    let root = std::env::temp_dir().join(format!("hoi4_nested_layers_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("game/hoi4.exe", "synthetic executable");
    write("game/common/file.txt", "vanilla");
    write(
        "game/dlc/outer/inner/content.dlc",
        r#"name="Nested DLC" path="dlc/outer/inner""#,
    );
    write("game/dlc/outer/inner/common/file.txt", "DLC");
    write("user/settings.txt", "language=l_english");
    write(
        "user/dlc_load.json",
        r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    );
    let (game_dir, user_dir, cache_dir) =
        (root.join("game"), root.join("user"), root.join("cache"));
    let gather_options = GatherOptions {
        game_dir: game_dir.clone(),
        user_dir: user_dir.clone(),
        cache_dir: cache_dir.clone(),
        dlc_order: None,
    };
    let identity = gather(&gather_options)
        .unwrap()
        .identity(
            "build".into(),
            Sha256::digest(b"synthetic executable").into(),
            20,
            "test".into(),
            [0; 32],
        )
        .unwrap();
    let ids = identity.dlc.iter().map(|dlc| dlc.id.as_str());
    assert_eq!(ids.collect::<Vec<_>>(), ["inner"]);
    let resolved = resolve(
        &identity,
        &ResolveOptions {
            game_dir,
            user_dir,
            cache_dir,
        },
    )
    .unwrap();
    assert_eq!(resolved.layers.read("common/file.txt").unwrap(), b"DLC");
    write("user/mod/nested/a.mod", r#"name="Nested" path="layers/a""#);
    write("user/layers/a/common/file.txt", "mod");
    write(
        "user/dlc_load.json",
        r#"{"enabled_mods":["mod/nested/a.mod"],"disabled_dlcs":[]}"#,
    );
    let gathered = gather(&gather_options).unwrap();
    assert!(gathered.mods.is_empty());
    assert!(gathered.mod_descriptors.is_empty());
    assert_eq!(gathered.layers.read("common/file.txt").unwrap(), b"DLC");
    let note = gathered.order_note.unwrap_or_default();
    assert!(
        note.contains("enabled mod descriptor mod/nested/a.mod is not directly in mod/"),
        "{note}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_mod_descriptor_is_listed_by_its_exact_suffix() {
    let root = std::env::temp_dir().join(format!("hoi4_mod_suffix_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write("game/hoi4.exe", "synthetic executable");
    write("game/common/file.txt", "vanilla");
    fs::create_dir_all(root.join("game/dlc")).unwrap();
    write("user/settings.txt", "language=l_english");
    write(
        "user/dlc_load.json",
        r#"{"enabled_mods":["mod/upper.MOD"],"disabled_dlcs":[]}"#,
    );
    write(
        "user/mod/upper.MOD",
        r#"name="Upper" remote_file_id=7 path="layers/upper""#,
    );
    write("user/layers/upper/common/file.txt", "mod");
    let gathered = gather(&GatherOptions {
        game_dir: root.join("game"),
        user_dir: root.join("user"),
        cache_dir: root.join("cache"),
        dlc_order: None,
    })
    .unwrap();
    assert!(gathered.mods.is_empty());
    assert_eq!(gathered.layers.read("common/file.txt").unwrap(), b"vanilla");
    let note = gathered.order_note.unwrap_or_default();
    assert!(
        note.contains("enabled mod descriptor mod/upper.MOD does not end in .mod"),
        "{note}"
    );
    let recorded = |id: &str| Mod {
        id: id.into(),
        display_name: String::new(),
        declared_version: String::new(),
        hash: [0; 32],
    };
    let user_dir = fs::canonicalize(root.join("user")).unwrap();
    let missing = recorded_mod_paths(&user_dir, &[recorded("7")]).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    write(
        "user/mod/lower.mod",
        r#"name="Lower" remote_file_id=8 path="layers/upper""#,
    );
    assert_eq!(
        recorded_mod_paths(&user_dir, &[recorded("8")]).unwrap(),
        ["mod/lower.mod"]
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn mod_forms_the_engine_cannot_order_are_named() {
    let (_, notes) = mod_order(&[
        ("mod/a.mod", r#"name="Same""#),
        ("mod/b.mod", r#"name="Same""#),
    ]);
    assert_eq!(
        notes,
        [
            r#"enabled mods mod/a.mod, mod/b.mod share the display name "Same"; the engine mounts only one"#
        ]
    );
    for (mods, refusal) in [
        (
            &[
                ("mod/a.mod", r#"name="A" dependencies={"B"}"#),
                ("mod/b.mod", r#"name="B" dependencies={"A"}"#),
            ][..],
            "dependency cycle",
        ),
        (
            &[("mod/a.mod", r#"name="A" dependencies={"A"}"#)],
            "Mod mod/a.mod: dependency cycle",
        ),
        (
            &[("mod/a.mod", r#"name="A" dependencies="B""#)],
            "Mod mod/a.mod: dependencies is not a list",
        ),
        (
            &[("mod/a.mod", r#"name="A" dependencies={x=y}"#)],
            "other than a name",
        ),
        (
            &[("mod/a.mod", r#"dependencies={}"#)],
            "Mod mod/a.mod: descriptor lacks name",
        ),
    ] {
        let error = mod_order(mods).0.unwrap_err().to_string();
        assert!(error.contains(refusal), "{error}");
    }
}

#[cfg(feature = "test-support")]
#[test]
fn has_file_sees_covered_and_uncovered_files_the_loader_reads() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_has_file_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, mod_root) = (root.join("base"), root.join("mod"));
    for (dir, file) in [
        (&base, "common/other.txt"),
        (&mod_root, "gfx/designs/my_tank.dds"),
    ] {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, file).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&base, &[][..]), (&mod_root, &[][..])]).unwrap();
    assert!(!covered("gfx/designs/my_tank.dds"));
    assert!(layers.has_file("gfx/designs/my_tank.dds").unwrap());
    assert!(
        layers.has_file("GFX/Designs/My_Tank.dds").unwrap(),
        "matched case-insensitively"
    );
    assert!(layers.has_file("common/other.txt").unwrap());
    assert!(!layers.has_file("gfx/designs/absent.dds").unwrap());
    assert!(!layers.has_file("common/absent.txt").unwrap());
    assert!(!layers.has_file("../escape.dds").unwrap());
    let zip_path = root.join("assets.zip");
    fs::write(
        &zip_path,
        raw_zip(&[
            (b"gfx/interface/MyTank.dds", 0, b"tank"),
            (b"gfx/designs/Other.dds", 0, b"other"),
        ]),
    )
    .unwrap();
    let zipped = ResolvedLayers::from_folders(&[(&base, &[][..]), (&zip_path, &[][..])]).unwrap();
    assert!(covered("gfx/interface/MyTank.dds"));
    LOADS.with(|loads| loads.set(0));
    for _ in 0..2 {
        assert!(layers.has_file("COMMON/Other.txt").unwrap());
        assert!(!layers.has_file("common/absent.txt").unwrap());
        assert!(zipped.has_file("gfx/interface/MyTank.dds").unwrap());
        assert!(!zipped.has_file("gfx/interface/mytank.dds").unwrap());
        assert!(layers.has_file("GFX/Designs/My_Tank.dds").unwrap());
        assert!(zipped.has_file("gfx/designs/Other.dds").unwrap());
        assert!(!zipped.has_file("gfx/designs/other.dds").unwrap());
    }
    assert_eq!(LOADS.with(|loads| loads.get()), 0, "no bytes loaded");
    assert!(layers.read("common/other.txt").is_ok());
    assert_eq!(
        LOADS.with(|loads| loads.get()),
        1,
        "the hook detects a covered payload read"
    );
    for (path, found) in [
        ("gfx/interface/MyTank.dds", true),
        ("gfx/interface/mytank.dds", false),
        ("gfx/designs/Other.dds", true),
        ("gfx/designs/other.dds", false),
    ] {
        assert_eq!(zipped.has_file(path).unwrap(), found, "{path}");
        assert_eq!(
            zipped.read_uncovered(path).is_ok(),
            found,
            "{path} as read_uncovered loads it"
        );
    }
    fs::write(base.join("common/other.txt"), b"changed").unwrap();
    assert!(layers.has_file("common/other.txt").unwrap());
    assert!(zipped.has_file("common/other.txt").unwrap());
    assert!(
        layers
            .read("common/other.txt")
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert!(
        zipped
            .read("common/other.txt")
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn has_file_dollar_lookup_precedes_the_effective_file_without_payload_reads() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_has_file_dollar_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("encrypted.zip");
    fs::write(&archive, include_bytes!("testdata/zipcrypto_stored.zip")).unwrap();
    let request = b"gfx/interface/e.dds$secret";
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_of(
                "base",
                &root.join("base"),
                &[("gfx/interface/e.dds$secret", b"base")],
            ),
            archive_layer("encrypted", &archive, &[]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    assert_eq!(layers.effective.get(request.as_slice()).unwrap().layer, 0);
    LOADS.with(|loads| loads.set(0));
    for _ in 0..2 {
        assert!(layers.has_file("gfx/interface/e.dds$secret").unwrap());
        assert!(!layers.has_file("gfx/interface/e.dds$wrong").unwrap());
        let Some(hit) = layers.lookup_path(request).unwrap() else {
            panic!("encrypted record must open");
        };
        assert_eq!(hit.layer, 1);
        assert_eq!(hit.path, request);
    }
    assert_eq!(LOADS.with(|loads| loads.get()), 0, "no bytes loaded");
    assert_eq!(
        layers.read_path(request).unwrap(),
        b"encrypted stored texture"
    );
    layers.layers.remove(0);
    layers.finish().unwrap();
    LOADS.with(|loads| loads.set(0));
    assert!(layers.has_file("gfx/interface/e.dds$secret").unwrap());
    assert_eq!(LOADS.with(|loads| loads.get()), 0, "no bytes loaded");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn has_file_case_missing_archive_member_falls_through_to_folder_without_payload_reads() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_has_file_case_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("assets.zip");
    fs::write(
        &archive,
        raw_zip(&[
            (b"gfx/interface/Asset.dds", 0, b"archive"),
            (b"gfx/world/font.fnt", 0, b"font"),
        ]),
    )
    .unwrap();
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_of(
                "base",
                &root.join("base"),
                &[("gfx/interface/Asset.dds", b"folder")],
            ),
            archive_layer("assets", &archive, &[]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    let request = b"GFX/Interface/ASSET.dds";
    LOADS.with(|loads| loads.set(0));
    for _ in 0..2 {
        assert!(layers.has_file("GFX/Interface/ASSET.dds").unwrap());
        assert!(layers.has_file("gfx/interface/Asset.dds").unwrap());
        let Some(hit) = layers.lookup_path(request).unwrap() else {
            panic!("folder file must open");
        };
        assert_eq!(hit.layer, 0);
        assert_eq!(hit.path, b"gfx/interface/Asset.dds");
    }
    assert_eq!(LOADS.with(|loads| loads.get()), 0, "no bytes loaded");
    assert_eq!(layers.read_path(request).unwrap(), b"folder");
    assert_eq!(layers.read("gfx/interface/Asset.dds").unwrap(), b"archive");
    let loads = LOADS.with(|loads| loads.get());
    let error = layers.read_path(b"gfx/world/font.fnt").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .to_string()
            .contains("this reader does not report uncovered reads")
    );
    assert_eq!(LOADS.with(|loads| loads.get()), loads);
    let reads = layers.report_uncovered_reads();
    assert_eq!(reads.warning(), None);
    assert_eq!(layers.read_path(b"gfx/world/font.fnt").unwrap(), b"font");
    assert!(reads.warning().unwrap().contains("gfx/world/font.fnt"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn has_file_replace_path_stops_fallthrough_without_payload_reads() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_has_file_replace_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("assets.zip");
    fs::write(
        &archive,
        raw_zip(&[(b"gfx/interface/Asset.dds", 0, b"archive")]),
    )
    .unwrap();
    let mut layers = ResolvedLayers {
        layers: vec![
            folder_of(
                "base",
                &root.join("base"),
                &[
                    ("gfx/interface/asset.dds", b"folder"),
                    ("gfx/interface/hidden.dds", b"hidden"),
                ],
            ),
            archive_layer("assets", &archive, &[b"GFX/Interface"]),
        ],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();
    LOADS.with(|loads| loads.set(0));
    for _ in 0..2 {
        assert!(!layers.has_file("gfx/interface/asset.dds").unwrap());
        assert!(!layers.has_file("gfx/interface/hidden.dds").unwrap());
        assert!(layers.has_file("gfx/interface/Asset.dds").unwrap());
    }
    assert_eq!(LOADS.with(|loads| loads.get()), 0, "no bytes loaded");
    for path in ["gfx/interface/asset.dds", "gfx/interface/hidden.dds"] {
        assert_eq!(
            layers.read(path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
    assert_eq!(layers.read("gfx/interface/Asset.dds").unwrap(), b"archive");
    fs::remove_dir_all(&root).unwrap();
}

thread_local! {
    pub(super) static LOADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static FOLDER_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ARCHIVE_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ARCHIVE_MOUNTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

thread_local! {
    pub(super) static PAYLOAD_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct PartialFailure {
    bytes: &'static [u8],
    calls: usize,
}
impl Read for PartialFailure {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.bytes.is_empty() {
            return Err(io::Error::from_raw_os_error(12345));
        }
        let count = out.len().min(self.bytes.len());
        out[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}

#[test]
fn finish_read_formats_failures_only_when_traced_and_returns_the_original_error() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct CountedError(Arc<AtomicUsize>);
    impl std::fmt::Display for CountedError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fetch_add(1, Ordering::Relaxed);
            formatter.write_str("synthetic read failure")
        }
    }
    impl std::error::Error for CountedError {}

    for traced in [false, true] {
        let layers = ResolvedLayers::default();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let log = reads.clone();
        let layers = if traced {
            layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)))
        } else {
            layers
        };
        let displays = Arc::new(AtomicUsize::new(0));
        let payload = ReadPayload {
            length: 13,
            digest: Sha256::digest(b"actual prefix").into(),
        };
        let mut observed = layers.observation();
        observed.at(0);
        let error = layers
            .finish_read::<Vec<u8>>(
                b"map/read.txt",
                observed,
                Err(ReadFailure {
                    error: io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        CountedError(displays.clone()),
                    ),
                    payload: Some(payload.clone()),
                }),
            )
            .unwrap_err();
        assert_eq!(displays.load(Ordering::Relaxed), usize::from(traced));
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let original = error
            .get_ref()
            .unwrap()
            .downcast_ref::<CountedError>()
            .unwrap();
        assert!(Arc::ptr_eq(&original.0, &displays));
        let reads = reads.lock().unwrap();
        assert_eq!(reads.len(), usize::from(traced));
        if traced {
            assert_eq!(reads[0].path, b"map/read.txt");
            assert_eq!(reads[0].purpose, "");
            assert_eq!(reads[0].layer, Some(0));
            assert_eq!(
                reads[0].outcome,
                ReadOutcome::Error {
                    kind: "PermissionDenied".to_owned(),
                    message: "synthetic read failure".to_owned(),
                    payload: Some(payload),
                }
            );
        }
    }
}

#[test]
fn partial_folder_failures_hash_only_when_traced_and_success_reuses_the_covered_hash() {
    let mut layer = folder_layer("synthetic", Path::new("unused"), Vec::new());
    std::sync::Arc::make_mut(&mut layer.hashes)
        .insert(b"map/read.txt".to_vec(), Sha256::digest(b"complete").into());
    for traced in [false, true] {
        let observed = ReadObservation {
            enabled: traced,
            layer: Some(0),
        };
        let mut reader = PartialFailure {
            bytes: b"actual prefix",
            calls: 0,
        };
        PAYLOAD_HASHES.set(0);
        let failure = layer
            .folder_read_from(b"map/read.txt", &mut reader, &observed)
            .unwrap_err();
        assert_eq!(failure.error.raw_os_error(), Some(12345));
        assert_eq!(reader.calls, 2);
        assert_eq!(PAYLOAD_HASHES.get(), usize::from(traced));
        assert_eq!(
            failure.payload,
            traced.then(|| ReadPayload {
                length: 13,
                digest: Sha256::digest(b"actual prefix").into(),
            })
        );
        PAYLOAD_HASHES.set(0);
        let value = layer
            .folder_read_from(b"map/read.txt", &b"complete"[..], &observed)
            .unwrap();
        assert_eq!(PAYLOAD_HASHES.get(), 1);
        match value {
            ReadSuccess::Traced { value, payload } => {
                assert!(traced);
                assert_eq!(value, b"complete");
                assert_eq!(payload.length, 8);
                assert_eq!(
                    payload.digest,
                    <[u8; 32]>::from(Sha256::digest(b"complete"))
                );
            }
            ReadSuccess::Untraced(value) => {
                assert!(!traced);
                assert_eq!(value, b"complete");
            }
        }
    }
}

#[test]
fn archive_input_evidence_keeps_a_partial_read_and_the_original_error_without_retrying() {
    let mut original = PartialFailure {
        bytes: b"archive prefix",
        calls: 0,
    };
    let mut bytes = [0xA5; 32];
    let error = original.read_exact(&mut bytes).unwrap_err();
    let mut traced = PartialFailure {
        bytes: b"archive prefix",
        calls: 0,
    };
    let mut evidence = ReadEvidence::default();
    let traced_error = EvidenceReader {
        reader: &mut traced,
        evidence: &mut evidence,
    }
    .read_exact(&mut bytes)
    .unwrap_err();
    assert_eq!(traced.calls, original.calls);
    assert_eq!(traced.calls, 2);
    assert_eq!(traced_error.raw_os_error(), error.raw_os_error());
    assert_eq!(traced_error.kind(), error.kind());
    assert_eq!(traced_error.to_string(), error.to_string());
    assert_eq!(
        evidence.finish(),
        ReadPayload {
            length: 14,
            digest: Sha256::digest(b"archive prefix").into(),
        }
    );
}

fn stored_deflate_fixture(name: &[u8]) -> Vec<u8> {
    let mut bytes = raw_zip(&[(
        name,
        0,
        &[1, 6, 0, 249, 255, b'v', b'a', b'l', b'i', b'd', b'!'],
    )]);
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    bytes[8..10].copy_from_slice(&8u16.to_le_bytes());
    bytes[22..26].copy_from_slice(&6u32.to_le_bytes());
    bytes[central + 10..central + 12].copy_from_slice(&8u16.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&6u32.to_le_bytes());
    bytes
}

#[test]
fn malformed_deflate_reads_keep_distinct_archive_input_evidence_and_identical_errors() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "traced-deflate-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("synthetic.zip");
    let name = b"common/one.txt";
    let valid = stored_deflate_fixture(name);
    fs::write(&path, &valid).unwrap();
    let layers =
        ResolvedLayers::from_test_layers(vec![archive_layer("synthetic", &path, &[])]).unwrap();
    assert_eq!(layers.read("common/one.txt").unwrap(), b"valid!");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut payloads = Vec::new();
    let mut errors = Vec::new();
    for byte in [b'A', b'B'] {
        let mut damaged = valid.clone();
        let body = [1, 1, 0, 254, 255, byte, 0, 0, 0, 0, 0];
        damaged[30 + name.len()..30 + name.len() + body.len()].copy_from_slice(&body);
        fs::write(&path, &damaged).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let original = layers.read("common/one.txt").unwrap_err();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let log = reads.clone();
        let traced = layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)));
        for mode in 0..3 {
            let error = match mode {
                0 => traced.read("common/one.txt"),
                1 => traced
                    .read_prefix("common/one.txt", 1)
                    .map(|(bytes, _)| bytes),
                _ => traced.read_uncovered("common/one.txt"),
            }
            .unwrap_err();
            assert_eq!(error.kind(), original.kind());
            assert_eq!(error.to_string(), original.to_string());
        }
        let expected = [&valid[..30], &body].concat();
        let reads = reads.lock().unwrap();
        assert_eq!(reads.len(), 3);
        for read in reads.iter() {
            assert_eq!(read.layer, Some(0));
            assert_eq!(
                read.outcome,
                ReadOutcome::Error {
                    kind: "InvalidData".into(),
                    message: original.to_string(),
                    payload: Some(ReadPayload {
                        length: expected.len() as u64,
                        digest: Sha256::digest(&expected).into()
                    }),
                }
            );
        }
        payloads.push(reads[0].outcome.clone());
        errors.push(original.to_string());
    }
    assert_eq!(errors[0], errors[1]);
    assert!(errors[0].contains("it inflates to fewer bytes than its uncompressed size"));
    assert_ne!(payloads[0], payloads[1]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn password_decode_failure_records_its_own_input_not_the_prior_verification_read() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "traced-password-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("synthetic.zip");
    let mut bytes = include_bytes!("testdata/zipcrypto_deflated.zip").to_vec();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    let size = u32::from_le_bytes(bytes[central + 24..central + 28].try_into().unwrap()) + 1;
    bytes[22..26].copy_from_slice(&size.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let layers =
        ResolvedLayers::from_test_layers(vec![archive_layer("synthetic", &path, &[])]).unwrap();
    let original = layers.read("gfx/fonts/x.fnt$secret").unwrap_err();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let traced = layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)));
    let error = traced.read_prefix("gfx/fonts/x.fnt$secret", 2).unwrap_err();
    assert_eq!(error.kind(), original.kind());
    assert_eq!(error.to_string(), original.to_string());
    assert!(
        error
            .to_string()
            .contains("it inflates to fewer bytes than its uncompressed size")
    );
    let data = 30
        + u16::from_le_bytes(bytes[26..28].try_into().unwrap()) as usize
        + u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as usize;
    let compressed =
        u32::from_le_bytes(bytes[central + 20..central + 24].try_into().unwrap()) as usize;
    let expected = [&bytes[..30], &bytes[data..data + 12 + compressed]].concat();
    let reads = reads.lock().unwrap();
    assert_eq!(reads.len(), 1);
    assert_eq!(
        reads[0].outcome,
        ReadOutcome::Error {
            kind: "InvalidData".into(),
            message: error.to_string(),
            payload: Some(ReadPayload {
                length: expected.len() as u64,
                digest: Sha256::digest(&expected).into()
            }),
        }
    );
    drop(reads);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verified_encrypted_read_refuses_bytes_outside_its_record_digest() {
    let root = std::env::temp_dir().join(format!(
        "verified-encrypted-suffix-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    for (name, raw, request) in [
        (
            "stored.zip",
            include_bytes!("testdata/zipcrypto_stored.zip").as_slice(),
            "gfx/interface/e.dds$secret",
        ),
        (
            "deflated.zip",
            include_bytes!("testdata/zipcrypto_deflated.zip").as_slice(),
            "gfx/fonts/x.fnt$secret",
        ),
    ] {
        let mut bytes = raw.to_vec();
        let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
        let compressed =
            u32::from_le_bytes(bytes[central + 20..central + 24].try_into().unwrap()) - 4;
        bytes[18..22].copy_from_slice(&compressed.to_le_bytes());
        bytes[central + 20..central + 24].copy_from_slice(&compressed.to_le_bytes());
        let path = root.join(name);
        fs::write(&path, &bytes).unwrap();
        let layers =
            ResolvedLayers::from_test_layers(vec![archive_layer(name, &path, &[])]).unwrap();
        let error = layers.read(request).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{name}: {error}");
        assert!(
            error
                .to_string()
                .contains("reads past its verified stored range"),
            "{name}: {error}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn truncated_encrypted_stored_member_preserves_short_read_error_and_header_evidence() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "traced-truncated-stored-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("truncated.zip");
    let mut bytes = include_bytes!("testdata/zipcrypto_stored.zip").to_vec();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    let compressed = 4_u32;
    let size = bytes.len() as u32 + 16;
    bytes[18..22].copy_from_slice(&compressed.to_le_bytes());
    bytes[central + 20..central + 24].copy_from_slice(&compressed.to_le_bytes());
    bytes[22..26].copy_from_slice(&size.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let layers =
        ResolvedLayers::from_test_layers(vec![archive_layer("truncated", &path, &[])]).unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let traced = layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)));
    let error = traced.read("gfx/interface/e.dds$secret").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    assert!(
        error
            .to_string()
            .contains("its stored data ends before its uncompressed size"),
        "{error}"
    );
    let data = 30
        + u16::from_le_bytes(bytes[26..28].try_into().unwrap()) as usize
        + u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as usize;
    let expected = [&bytes[..30], &bytes[data..data + 12]].concat();
    let reads = reads.lock().unwrap();
    assert_eq!(reads.len(), 1);
    assert_eq!(
        reads[0].outcome,
        ReadOutcome::Error {
            kind: "InvalidData".into(),
            message: error.to_string(),
            payload: Some(ReadPayload {
                length: expected.len() as u64,
                digest: Sha256::digest(&expected).into(),
            }),
        }
    );
    drop(reads);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn multi_chunk_encrypted_deflate_matches_direct_reader_under_verification() {
    let root = std::env::temp_dir().join(format!(
        "verified-encrypted-multichunk-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("large.zip");
    let plain = vec![b'A'; 65_536 + 10];
    assert!(plain.len() > 0x10000);
    let deflated = miniz_oxide::deflate::compress_to_vec(&plain, 6);
    let mut stream = Vec::with_capacity(12 + deflated.len());
    stream.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0]);
    stream.extend_from_slice(&deflated);
    let encrypted = physfs_zip::encrypt_zipcrypto(b"secret", &stream);
    let mut bytes = raw_zip(&[(b"gfx/fonts/large.fnt", 0, &encrypted)]);
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    let size = plain.len() as u32;
    bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&8u16.to_le_bytes());
    bytes[14..18].copy_from_slice(&0u32.to_le_bytes());
    bytes[22..26].copy_from_slice(&size.to_le_bytes());
    bytes[central + 8..central + 10].copy_from_slice(&1u16.to_le_bytes());
    bytes[central + 10..central + 12].copy_from_slice(&8u16.to_le_bytes());
    bytes[central + 16..central + 20].copy_from_slice(&0u32.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let mut archive = physfs_zip::Archive::open(&path).unwrap();
    let mounted = archive.mount().unwrap();
    let (entry, password) = mounted.lookup(b"gfx/fonts/large.fnt$secret").unwrap();
    let direct = archive.read(entry, password).unwrap().unwrap();
    assert_eq!(direct, plain);
    let layers =
        ResolvedLayers::from_test_layers(vec![archive_layer("large", &path, &[])]).unwrap();
    let verified = layers.read("gfx/fonts/large.fnt$secret").unwrap();
    assert_eq!(verified.as_ref(), direct.as_slice());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn successful_traced_reads_own_full_evidence_through_zero_prefix_and_memoized_paths() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "traced-success-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(root.join("map")).unwrap();
    fs::create_dir_all(root.join("gfx/custom")).unwrap();
    for path in ["map/read.txt", "gfx/custom/read.bin"] {
        fs::write(root.join(path), b"full evidence").unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let traced = layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)));
    assert_eq!(traced.read("map/read.txt").unwrap(), b"full evidence");
    assert_eq!(
        traced.read_prefix("map/read.txt", 0).unwrap(),
        (Bytes::default(), 13)
    );
    for _ in 0..2 {
        assert_eq!(
            traced.read_uncovered("gfx/custom/read.bin").unwrap(),
            b"full evidence"
        );
    }
    let reads = reads.lock().unwrap();
    assert_eq!(reads.len(), 4);
    for read in reads.iter() {
        assert_eq!(read.layer, Some(0));
        assert_eq!(
            read.outcome,
            ReadOutcome::Bytes {
                length: 13,
                digest: Sha256::digest(b"full evidence").into()
            }
        );
    }
    drop(reads);
    let archive_path = root.join("synthetic.zip");
    fs::write(&archive_path, stored_deflate_fixture(b"common/one.txt")).unwrap();
    let archive_layers =
        ResolvedLayers::from_test_layers(vec![archive_layer("synthetic", &archive_path, &[])])
            .unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let traced =
        archive_layers.with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)));
    assert_eq!(
        traced.read_prefix("common/one.txt", 0).unwrap(),
        (Bytes::default(), 6)
    );
    assert_eq!(
        reads.lock().unwrap()[0].outcome,
        ReadOutcome::Bytes {
            length: 6,
            digest: Sha256::digest(b"valid!").into()
        }
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_below_the_visual_mods_keep_the_trace_purpose_and_presentation_layers() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_fallback_trace_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let presentation = overlaid_stack(
        &root,
        &[("gfx/entities/a.asset", b"base a")],
        &[("gfx/entities/a.asset", b"art a")],
        &[("gfx/entities/b.asset", b"above b")],
    );
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let traced = presentation
        .with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)))
        .with_read_purpose("artwork");
    assert_eq!(
        traced.read_uncovered("gfx/entities/a.asset").unwrap(),
        b"art a"
    );
    assert_eq!(
        traced.read_uncovered("gfx/entities/b.asset").unwrap(),
        b"above b"
    );
    let recorded = traced.without_overlays().unwrap();
    assert_eq!(
        recorded.read_uncovered("gfx/entities/a.asset").unwrap(),
        b"base a"
    );
    assert_eq!(
        recorded.read_uncovered("gfx/entities/b.asset").unwrap(),
        b"above b"
    );
    assert!(recorded.read_uncovered("gfx/entities/c.asset").is_err());
    let bytes = |data: &[u8]| ReadOutcome::Bytes {
        length: data.len() as u64,
        digest: Sha256::digest(data).into(),
    };
    let reads = reads.lock().unwrap();
    let observed: Vec<_> = reads
        .iter()
        .map(|read| (read.path.as_slice(), read.purpose.as_str(), read.layer))
        .collect();
    assert_eq!(
        observed,
        [
            (b"gfx/entities/a.asset".as_slice(), "artwork", Some(1)),
            (b"gfx/entities/b.asset".as_slice(), "artwork", Some(2)),
            (b"gfx/entities/a.asset".as_slice(), "artwork", Some(0)),
            (b"gfx/entities/b.asset".as_slice(), "artwork", Some(2)),
            (b"gfx/entities/c.asset".as_slice(), "artwork", None),
        ]
    );
    assert_eq!(reads[2].outcome, bytes(b"base a"));
    assert_eq!(reads[3].outcome, reads[1].outcome);
    assert!(matches!(&reads[4].outcome, ReadOutcome::Error { kind, .. } if kind == "NotFound"));
    drop(reads);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stamped_reads_open_listed_covered_files() {
    let root = std::env::temp_dir().join(format!("stamped-reads-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("map/music")).unwrap();
    fs::write(root.join("map/continent.txt"), b"continents={a}").unwrap();
    fs::write(root.join("map/music/continent.txt"), b"continents={b}").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert!(layers.stamped("map/continent.txt").unwrap());
    assert!(!layers.stamped("map/music/continent.txt").unwrap());
    assert!(!layers.stamped("map/missing.txt").unwrap());
    let listed = layers.recorded_source_files().unwrap();
    assert!(listed.iter().any(|path| path.ends_with("map/continent.txt")));
    assert!(!listed.iter().any(|path| path.ends_with("map/music/continent.txt")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_password_request_is_stamped_by_its_records_name() {
    let root = std::env::temp_dir().join(format!(
        "stamped-password-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let stamped = |file: &str, name: &[u8], request: &str| {
        let mut bytes = include_bytes!("testdata/zipcrypto_stored.zip").to_vec();
        for at in 0..bytes.len() {
            if bytes[at..].starts_with(b"gfx/interface/e.dds") {
                bytes[at..at + name.len()].copy_from_slice(name);
            }
        }
        let archive = root.join(file);
        fs::write(&archive, bytes).unwrap();
        let mut layers = ResolvedLayers {
            layers: vec![archive_layer("mod", &archive, &[])],
            ..ResolvedLayers::default()
        };
        layers.finish().unwrap();
        layers.stamped(request).unwrap()
    };
    assert!(stamped(
        "covered.zip",
        b"gfx/interface/e.dds",
        "gfx/interface/e.dds$secret"
    ));
    assert!(!stamped(
        "skipped.zip",
        b"gfx/interface/music",
        "gfx/interface/music$secret"
    ));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn playback_and_visual_share_workshop_mod_discovery_with_empty_user_dir() {
    let root = std::env::temp_dir().join(format!(
        "shared-workshop-discovery-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let game_dir = root.join("steamapps/common/TestGame");
    let workshop_item = root.join("steamapps/workshop/content/394360/2777392649");
    let empty_user = root.join("empty_user");
    let cache_dir = root.join("cache");
    fs::create_dir_all(game_dir.join("gfx")).unwrap();
    fs::create_dir_all(workshop_item.join("gfx/flags")).unwrap();
    fs::create_dir_all(&empty_user).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(
        workshop_item.join("descriptor.mod"),
        "name=\"Synthetic Workshop Mod\"\nremote_file_id=\"2777392649\"\nreplace_path=\"gfx/replaced\"\n",
    )
    .unwrap();
    fs::write(workshop_item.join("gfx/flags/SOV_communism.tga"), b"mod-flag").unwrap();

    let options = ResolveOptions {
        game_dir: game_dir.clone(),
        user_dir: empty_user.clone(),
        cache_dir,
    };
    let installed = visual::installed(&options, &AtomicBool::new(false)).unwrap();
    assert!(
        installed
            .iter()
            .any(|m| m.recorded_id == "2777392649" && m.name == "Synthetic Workshop Mod"),
        "{installed:?}"
    );

    let descriptors = Descriptors::installed(Some(&game_dir), Some(&empty_user));
    let (source, descriptor) = descriptors.find_mod("2777392649").unwrap();
    let expected_folder = fs::canonicalize(&workshop_item).unwrap();
    match source {
        LayerSource::Folder(path) => assert_eq!(path, expected_folder),
        other => panic!("expected folder source, got {other:?}"),
    }
    assert_eq!(
        descriptor,
        Some(expected_folder.join("descriptor.mod"))
    );

    fs::create_dir_all(empty_user.join("mod")).unwrap();
    fs::write(
        empty_user.join("mod/ugc_2777392649.mod"),
        format!(
            "name=\"Synthetic Workshop Mod\"\npath=\"{}\"\nremote_file_id=\"2777392649\"\n",
            workshop_item.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let deduped = Descriptors::installed(Some(&game_dir), Some(&empty_user));
    assert!(deduped.find_mod("2777392649").is_ok());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn uncovered_art_lookup_scans_each_layer_once() {
    let root = std::env::temp_dir().join(format!(
        "uncovered-art-lookup-counts-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let base = folder_of(
        "base",
        &root.join("base"),
        &[
            ("gfx/models/base_only.dds", b"base-model"),
            ("gfx/interface/Base_Icon.dds", b"base-icon"),
            ("gfx/models/shared.dds", b"base-shared"),
        ],
    );
    let zip_path = root.join("dlc.zip");
    fs::write(
        &zip_path,
        raw_zip(&[
            (b"gfx/models/from_dlc.dds", 0, b"dlc-model"),
            (b"gfx/models/shared.dds", 0, b"dlc-shared"),
        ]),
    )
    .unwrap();
    let upper = folder_of(
        "mod",
        &root.join("mod"),
        &[("gfx/models/mod_only.dds", b"mod-model")],
    );
    let _generation = Generation::enter();
    let _loading = Generation::loading();
    let mut layers = ResolvedLayers {
        layers: vec![base, archive_layer("dlc", &zip_path, &[]), upper],
        ..ResolvedLayers::default()
    };
    layers.finish().unwrap();

    let reset = || {
        FOLDER_OPENS.with(|c| c.set(0));
        ARCHIVE_OPENS.with(|c| c.set(0));
        ARCHIVE_MOUNTS.with(|c| c.set(0));
    };
    let counts = || {
        (
            FOLDER_OPENS.with(|c| c.get()),
            ARCHIVE_OPENS.with(|c| c.get()),
            ARCHIVE_MOUNTS.with(|c| c.get()),
        )
    };

    reset();
    assert_eq!(
        layers.read_uncovered("gfx/models/base_only.dds").unwrap(),
        b"base-model"
    );
    let base_counts = counts();

    reset();
    assert_eq!(
        layers.read_uncovered("gfx/models/from_dlc.dds").unwrap(),
        b"dlc-model"
    );
    let dlc_counts = counts();

    reset();
    assert_eq!(
        layers
            .read_uncovered("gfx/models/absent.dds")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let missing_counts = counts();

    reset();
    let mounted = layers.read_art_file(b"gfx/interface/base_icon.dds").unwrap();
    let ArtFileRead::Mounted { layer, path, bytes } = mounted else {
        panic!("expected mounted, got {mounted:?}");
    };
    assert_eq!(layer, 0);
    assert_eq!(path, b"gfx/interface/Base_Icon.dds");
    assert_eq!(bytes, b"base-icon");
    let covered_art_counts = counts();

    reset();
    let repeat = layers.read_art_file(b"gfx/interface/base_icon.dds").unwrap();
    let ArtFileRead::Mounted { bytes, .. } = repeat else {
        panic!("expected cached mounted, got {repeat:?}");
    };
    assert_eq!(bytes, b"base-icon");
    let cached_counts = counts();

    reset();
    let dlc_art = layers.read_art_file(b"gfx/models/shared.dds").unwrap();
    let ArtFileRead::Mounted { layer, bytes, .. } = dlc_art else {
        panic!("expected dlc mounted, got {dlc_art:?}");
    };
    assert_eq!(layer, 1);
    assert_eq!(bytes, b"dlc-shared");
    let dlc_art_counts = counts();

    let mut unverified_layers = ResolvedLayers {
        layers: vec![
            folder_of("base", &root.join("base"), &[]),
            {
                let mut dlc = archive_layer("dlc", &zip_path, &[]);
                dlc.overlay = Some(Overlay {
                    name: "unverified-dlc".into(),
                    verified: false,
                });
                dlc
            },
            folder_of("mod", &root.join("mod"), &[]),
        ],
        ..ResolvedLayers::default()
    };
    unverified_layers.finish().unwrap();

    reset();
    let unverified_base = unverified_layers
        .read_art_file(b"gfx/models/base_only.dds")
        .unwrap();
    let ArtFileRead::Mounted { bytes, .. } = unverified_base else {
        panic!("expected mounted, got {unverified_base:?}");
    };
    assert_eq!(bytes, b"base-model");
    let unverified_miss_counts = counts();

    reset();
    let unverified_dlc = unverified_layers
        .read_art_file(b"gfx/models/from_dlc.dds")
        .unwrap();
    let ArtFileRead::Mounted { bytes, .. } = unverified_dlc else {
        panic!("expected mounted, got {unverified_dlc:?}");
    };
    assert_eq!(bytes, b"dlc-model");
    let unverified_hit_counts = counts();

    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        (
            base_counts,
            dlc_counts,
            missing_counts,
            covered_art_counts,
            cached_counts,
            dlc_art_counts,
            unverified_miss_counts,
            unverified_hit_counts,
        ),
        (
            (2, 1, 1),
            (1, 1, 1),
            (2, 1, 1),
            (2, 1, 1),
            (0, 0, 0),
            (1, 1, 1),
            (2, 1, 1),
            (1, 1, 1),
        ),
        "after counts (folder_opens, archive_opens, archive_mounts)"
    );
}
