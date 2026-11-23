use super::*;
#[cfg(windows)]
use crate::test_support::{change_directory_with_new_stamp, rewrite_preserving_modified_time};

type Answer = Result<Vec<u8>, (io::ErrorKind, String)>;
fn answer(result: io::Result<Bytes>) -> Answer {
    result
        .map(|bytes| bytes.to_vec())
        .map_err(|error| (error.kind(), error.to_string()))
}

#[test]
fn warm_memo_keeps_fixed_bytes_errors_loads_reports_and_changed_stack_checks() {
    let root = std::env::temp_dir().join(format!("read-memo-exact-{}", std::process::id()));
    let base = root.join("base");
    let overlay = root.join("overlay");
    fs::create_dir_all(base.join("map")).unwrap();
    fs::create_dir_all(base.join("gfx/world")).unwrap();
    fs::create_dir_all(overlay.join("gfx/world")).unwrap();
    fs::write(base.join("map/covered.txt"), b"covered").unwrap();
    fs::write(base.join("gfx/world/fallback.mesh"), b"fallback").unwrap();
    fs::write(base.join("gfx/world/Case.mesh"), b"base case").unwrap();
    fs::write(overlay.join("gfx/world/Case.mesh"), b"higher case").unwrap();
    let archive = root.join("synthetic.zip");
    let bytes = zip_records(
        &[],
        &[
            Record {
                name: b"map/covered-zip.txt",
                host: 3,
                flags: 0,
                deflate: false,
                data: b"verified archive",
            },
            Record {
                name: b"gfx/world/stored.mesh",
                host: 3,
                flags: 0,
                deflate: false,
                data: b"stored",
            },
            Record {
                name: b"gfx/world/deflate.mesh",
                host: 3,
                flags: 0,
                deflate: true,
                data: &[7; 12000],
            },
            Record {
                name: b"gfx/world/raw-\xff.mesh",
                host: 3,
                flags: 0,
                deflate: false,
                data: b"raw name",
            },
        ],
    );
    fs::write(&archive, &bytes).unwrap();
    let encrypted = root.join("encrypted.zip");
    fs::write(
        &encrypted,
        include_bytes!("../testdata/zipcrypto_stored.zip"),
    )
    .unwrap();
    let make = || {
        let mut layers = ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
        layers
            .layers
            .push(archive_layer("synthetic", &archive, &[b"gfx/hidden"]));
        layers
            .layers
            .push(archive_layer("encrypted", &encrypted, &[]));
        layers.finish().unwrap();
        layers
    };
    let new = {
        let mut layers = make();
        let reports = layers.report_uncovered_reads();
        let first = LOADS.with(std::cell::Cell::get);
        let mut out = Vec::new();
        let mut loads = Vec::new();
        for _ in 0..3 {
            for path in [
                b"map/covered.txt".as_slice(),
                b"map/covered-zip.txt",
                b"gfx/world/fallback.mesh",
                b"gfx/world/Case.mesh",
                b"gfx/world/CASE.mesh",
                b"gfx/world/stored.mesh",
                b"gfx/world/deflate.mesh",
                b"gfx/world/raw-\xff.mesh",
                b"gfx/world/missing.mesh",
                b"gfx/hidden/blocked.mesh",
                b"gfx/interface/e.dds$secret",
                b"gfx/interface/e.dds$wrong",
                b"gfx//world/Case.mesh",
                b"gfx\\world\\Case.mesh",
                b"../unsafe",
                b"gfx/world/.",
            ] {
                out.push(answer(layers.read_uncovered_path(path)));
                loads.push(LOADS.with(std::cell::Cell::get) - first);
            }
        }
        fs::remove_file(base.join("gfx/world/fallback.mesh")).unwrap();
        out.push(answer(layers.read_uncovered("gfx/world/fallback.mesh")));
        fs::write(base.join("gfx/world/fallback.mesh"), b"fallback").unwrap();
        fs::write(base.join("map/covered.txt"), b"changed").unwrap();
        out.push(answer(layers.read("map/covered.txt")));
        fs::write(base.join("map/covered.txt"), b"covered").unwrap();
        let higher = layers.layers.remove(1);
        out.push(answer(layers.read_uncovered("gfx/world/Case.mesh")));
        layers.layers.insert(1, higher);
        out.push(answer(layers.read_uncovered("gfx/world/Case.mesh")));
        fs::write(&archive, raw_zip(&[(b"map/changed.txt", 0, b"changed")])).unwrap();
        out.push(answer(layers.read_uncovered("gfx/world/stored.mesh")));
        fs::write(&archive, &bytes).unwrap();
        (
            out,
            loads,
            LOADS.with(std::cell::Cell::get) - first,
            reports.warning(),
            reports.overlay_warning(),
            reports.overlay_refusals(),
        )
    };
    assert_eq!(new.0.len(), 53);
    for round in 1..3 {
        assert_eq!(&new.0[round * 16..(round + 1) * 16], &new.0[..16]);
    }
    assert_eq!(new.0[0], Ok(b"covered".to_vec()));
    assert_eq!(new.0[1], Ok(b"verified archive".to_vec()));
    assert_eq!(new.0[2], Ok(b"fallback".to_vec()));
    assert_eq!(new.0[5], Ok(b"stored".to_vec()));
    assert_eq!(new.0[7], Ok(b"raw name".to_vec()));
    assert!(
        new.0[8]
            .as_ref()
            .is_err_and(|(kind, _)| *kind == io::ErrorKind::NotFound)
    );
    assert_eq!(new.0[50], Ok(b"base case".to_vec()));
    assert_eq!(new.0[51], Ok(b"higher case".to_vec()));
    assert_eq!(new.0[3], Ok(b"higher case".to_vec()));
    assert_eq!(new.0[6], Ok(vec![7; 12000]));
    assert!(new.0.iter().any(|r| {
        r.as_ref()
            .is_err_and(|(kind, _)| *kind == io::ErrorKind::NotFound)
    }));
    assert!(new.0.iter().any(|r| {
        r.as_ref()
            .is_err_and(|(_, text)| text.contains("changed since verification"))
    }));
    assert!(new.2 > 0);
    fs::remove_dir_all(root).unwrap();
}


#[test]
fn retained_products_follow_the_effective_stack_context() {
    let _generation = Generation::enter();
    let mut recorded = ResolvedLayers::default();
    recorded.finish().unwrap();
    let read = |layers: &ResolvedLayers| {
        *layers.retained_product("context-dependent defines", 1, &[], |layers| {
            Ok(layers.defines_visible)
        }).unwrap()
    };
    assert!(!read(&recorded));
    let mut graphics = recorded.clone();
    graphics.defines_visible = true;
    graphics.finish().unwrap();
    assert!(read(&graphics));
    assert!(!read(&recorded));
}

#[test]
fn unavailable_configured_roots_refresh_when_they_recover() {
    let root = std::env::temp_dir().join(format!("content-root-recovery-{}", std::process::id()));
    let witnesses = generation::Witnesses::default();
    assert_eq!(witnesses.root(&root), None);
    witnesses.validate().unwrap();
    fs::create_dir_all(&root).unwrap();
    assert!(witnesses.validate().is_err());
    let fresh = generation::Witnesses::default();
    assert_eq!(fresh.root(&root), Some(root.canonicalize().unwrap()));
    fresh.validate().unwrap();
    fs::write(root.join("launcher.sqlite-journal"), b"journal").unwrap();
    fresh.validate().unwrap();
    fs::create_dir(root.join("logs")).unwrap();
    fs::write(root.join("logs/game.log"), b"log").unwrap();
    fresh.validate().unwrap();
    fs::remove_file(root.join("launcher.sqlite-journal")).unwrap();
    fresh.validate().unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(fresh.validate().is_err());
}

#[test]
fn retained_readers_retry_errors_except_the_explicit_error_catalogue() {
    let _generation = Generation::enter();
    let layers = ResolvedLayers::default();
    assert!(
        layers
            .retained_product("synthetic transient", 1, &[], |_| {
                Err::<u32, _>(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "transient lock",
                ))
            })
            .is_err()
    );
    assert_eq!(
        *layers
            .retained_product("synthetic transient", 1, &[], |_| Ok(7_u32))
            .unwrap(),
        7
    );
    assert!(
        layers
            .retained_product_with_errors("synthetic catalogue", 1, &[], true, |_| {
                Err::<u32, _>(io::Error::other("catalogue refusal"))
            })
            .is_err()
    );
    assert!(
        layers
            .retained_product_with_errors(
                "synthetic catalogue",
                1,
                &[],
                true,
                |_| -> io::Result<u32> {
                    panic!("intentional catalogue refusal remains cached");
                }
            )
            .is_err()
    );
}

#[cfg(windows)]
#[test]
fn preserved_mtime_edits_and_added_art_invalidate_a_live_generation() {
    let root = std::env::temp_dir().join(format!("generation-edits-{}", std::process::id()));
    fs::create_dir_all(root.join("gfx/world")).unwrap();
    let file = root.join("gfx/world/asset.mesh");
    fs::write(&file, b"old").unwrap();
    let _generation = Generation::enter();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert_eq!(
        layers.read_uncovered("gfx/world/asset.mesh").unwrap(),
        b"old"
    );
    rewrite_preserving_modified_time(&file, b"new");
    assert!(layers.validate_generation().is_err());
    let _fresh = Generation::enter();
    let fresh = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert_eq!(fresh.descriptor_paths(b"gfx", &[".mesh"]).unwrap().len(), 1);
    let directory = root.join("gfx/world");
    change_directory_with_new_stamp(&directory, || {
        fs::write(directory.join("added.mesh"), b"added").unwrap();
    });
    assert!(fresh.validate_generation().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn pinned_generations_refresh_changed_settings_snapshots() {
    let root = std::env::temp_dir().join(format!("settings-generation-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let file = root.join("settings.txt");
    fs::write(&file, b"old").unwrap();
    let layers = ResolvedLayers::default();
    let _generation = layers.enter_generation();
    let first = layers.with_file_witness(&file, || Generation::parse_snapshot(&file, "synthetic settings", 1, &[], |path| fs::read(path), |bytes| {
        Ok(bytes.to_vec())
    }))
    .unwrap();
    rewrite_preserving_modified_time(&file, b"new");
    let second = Generation::parse_snapshot(&file, "synthetic settings", 1, &[], |path| fs::read(path), |bytes| {
        Ok(bytes.to_vec())
    })
    .unwrap();
    assert_eq!(first.as_slice(), b"old");
    assert_eq!(second.as_slice(), b"new");
    assert!(layers.validate_generation().is_err());
    let _fresh = Generation::enter();
    let absent = ResolvedLayers::default();
    let later = root.join("later-settings.txt");
    absent.with_file_witness(&later, || {
        assert!(!later.exists());
        Ok(())
    }).unwrap();
    absent.validate_generation().unwrap();
    fs::write(&later, b"created").unwrap();
    assert!(absent.validate_generation().is_err());
    fs::remove_dir_all(root).unwrap();
}


#[test]
fn standalone_parse_file_refuses_a_file_that_appeared_within_the_same_generation() {
    let root = std::env::temp_dir().join(format!(
        "parse-file-appeared-{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("gfx/world")).unwrap();
    let file = root.join("gfx/world/late.mesh");
    let _generation = Generation::enter();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    for _ in 0..2 {
        let missing = Generation::parse_file(&file, "synthetic late", 1, &[], |bytes| {
            Ok(bytes.to_vec())
        })
        .unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }
    fs::write(&file, b"appeared").unwrap();
    assert_eq!(
        layers.read_uncovered("gfx/world/late.mesh").unwrap(),
        b"appeared"
    );
    let appeared = Generation::parse_file(&file, "synthetic late", 1, &[], |bytes| {
        Ok(bytes.to_vec())
    })
    .unwrap_err();
    assert_eq!(appeared.kind(), io::ErrorKind::InvalidData);
    assert!(
        appeared.to_string().contains("content generation changed"),
        "{appeared}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn deferred_watch_arming_forces_sweep_and_catches_pre_scope_changes() {
    let root = std::env::temp_dir().join(format!(
        "deferred-watch-arming-{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layer_root = root.join("layer");
    fs::create_dir_all(layer_root.join("gfx/world")).unwrap();
    let extra = root.join("extra");
    fs::create_dir_all(&extra).unwrap();
    let file = layer_root.join("gfx/world/a.mesh");
    fs::write(&file, b"old").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&layer_root, &[])]).unwrap();
    assert_eq!(layers.read_uncovered("gfx/world/a.mesh").unwrap(), b"old");
    layers.admit_generation().unwrap();
    rewrite_preserving_modified_time(&file, b"new");
    {
        let _scope = layers.enter_generation();
        assert!(layers.admit_generation().is_err());
    }
    let fresh = ResolvedLayers::from_folders(&[(&layer_root, &[])]).unwrap();
    assert_eq!(fresh.read_uncovered("gfx/world/a.mesh").unwrap(), b"new");
    {
        let _scope = fresh.enter_generation();
        fresh.admit_generation().unwrap();
        let validations = fresh
            .generation
            .changes
            .validations
            .load(std::sync::atomic::Ordering::Relaxed);
        fresh.admit_generation().unwrap();
        assert_eq!(
            fresh
                .generation
                .changes
                .validations
                .load(std::sync::atomic::Ordering::Relaxed),
            validations
        );
    }
    let mid = ResolvedLayers::from_folders(&[(&layer_root, &[])]).unwrap();
    assert_eq!(mid.read_uncovered("gfx/world/a.mesh").unwrap(), b"new");
    assert!(mid.generation.has_unwatched_roots());
    mid.admit_generation().unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (checked_tx, checked_rx) = std::sync::mpsc::channel();
    let registered = std::sync::Mutex::new(Vec::new());
    let mid_ref = &mid;
    let registered_ref = &registered;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut paused = false;
            mid_ref.generation.arm_pending_watches_with(|path, subtree| {
                if !paused {
                    paused = true;
                    entered_tx.send(()).unwrap();
                    checked_rx.recv().unwrap();
                }
                registered_ref.lock().unwrap().push(path.to_path_buf());
                mid_ref.generation.changes.watch_root(path, subtree);
            });
            drop(entered_tx);
        });
        entered_rx.recv().expect("arming entered watch registration");
        assert!(mid.generation.has_unwatched_roots());
        let before = mid
            .generation
            .changes
            .validations
            .load(std::sync::atomic::Ordering::Relaxed);
        mid.admit_generation().unwrap();
        assert_eq!(
            mid.generation
                .changes
                .validations
                .load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );
        mid.generation.watch_root(&extra, true);
        checked_tx.send(()).unwrap();
    });
    assert!(!mid.generation.has_unwatched_roots());
    assert!(registered.lock().unwrap().iter().any(|path| path == &extra));
    let after_arm = mid
        .generation
        .changes
        .validations
        .load(std::sync::atomic::Ordering::Relaxed);
    mid.admit_generation().unwrap();
    assert_eq!(
        mid.generation
            .changes
            .validations
            .load(std::sync::atomic::Ordering::Relaxed),
        after_arm + 1
    );
    mid.admit_generation().unwrap();
    assert_eq!(
        mid.generation
            .changes
            .validations
            .load(std::sync::atomic::Ordering::Relaxed),
        after_arm + 1
    );
    fs::remove_dir_all(root).unwrap();
}
