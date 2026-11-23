use content_reader_layers::content::visual::{
    apply,
    fixtures::{Machine, never, pick},
};
use content_reader_layers::{
    ResolvedLayers,
    content::{Generation, LayerPath, ReadOutcome, fixtures::overlay_folders},
};
use sha2::{Digest, Sha256};
use std::{fs, io, sync::Arc};

#[test]
fn warm_recorded_projection_replays_reads_to_new_traced_readers_and_parent_products() {
    let root = std::env::temp_dir().join(format!(
        "warm-projection-trace-{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let base = root.join("base");
    let visual = root.join("visual");
    fs::create_dir_all(base.join("common/defines")).unwrap();
    fs::create_dir_all(visual.join("gfx/world")).unwrap();
    fs::write(
        base.join("common/defines/00_base.lua"),
        b"NDefines = { Synthetic = { Value = 1 } }",
    )
    .unwrap();
    fs::write(visual.join("gfx/world/visual.mesh"), b"visual").unwrap();
    let _generation = Generation::enter();
    let source = overlay_folders(&base, &visual, "Art", false);
    let traced = |purpose: &str| {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = Arc::clone(&log);
        let layers = source
            .with_read_trace(Arc::new(move |read| kept.lock().unwrap().push(read)))
            .with_read_purpose(purpose);
        (layers, log)
    };
    let (p1, log1) = traced("p1");
    let (p2, log2) = traced("p2");
    assert_eq!(
        crate::defines::read(&p1)
            .unwrap()
            .number("Synthetic", "Value")
            .unwrap(),
        1.0
    );
    let first_len = log1.lock().unwrap().len();
    assert!(log1.lock().unwrap().iter().any(|read| {
        read.path == b"common/defines/00_base.lua"
            && read.purpose == "p1"
            && matches!(read.outcome, ReadOutcome::Bytes { .. })
    }));
    assert_eq!(
        crate::defines::read(&p1)
            .unwrap()
            .number("Synthetic", "Value")
            .unwrap(),
        1.0
    );
    assert_eq!(log1.lock().unwrap().len(), first_len);
    assert_eq!(
        crate::defines::read(&p2)
            .unwrap()
            .number("Synthetic", "Value")
            .unwrap(),
        1.0
    );
    assert!(log2.lock().unwrap().iter().any(|read| {
        read.path == b"common/defines/00_base.lua"
            && read.purpose == "p2"
            && matches!(read.outcome, ReadOutcome::Bytes { .. })
    }));
    let parent_value = p1
        .product("parent over defines", 1, &[], |reader| {
            let defines = crate::defines::read(reader)?;
            Ok(defines.number("Synthetic", "Value").unwrap() as u32)
        })
        .unwrap();
    assert_eq!(*parent_value, 1);
    let (p3, log3) = traced("p3");
    let cached_parent = p3
        .product("parent over defines", 1, &[], |_| -> io::Result<u32> {
            panic!("parent product must be reused");
        })
        .unwrap();
    assert_eq!(*cached_parent, 1);
    assert!(log3.lock().unwrap().iter().any(|read| {
        read.path == b"common/defines/00_base.lua"
            && read.purpose == "p3"
            && matches!(read.outcome, ReadOutcome::Bytes { .. })
    }));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn unscoped_visual_defines_rebuild_when_an_unverified_script_changes() {
    let root = std::env::temp_dir().join(format!(
        "unscoped-visual-defines-{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let base = root.join("base");
    let visual = root.join("visual");
    fs::create_dir_all(base.join("common/defines")).unwrap();
    fs::create_dir_all(visual.join("common/defines")).unwrap();
    fs::write(
        base.join("common/defines/00_base.lua"),
        b"NDefines = { NGraphics = { X = 1 } }",
    )
    .unwrap();
    let visual_script = visual.join("common/defines/zz_visual.lua");
    fs::write(&visual_script, b"NDefines.NGraphics.X = 2").unwrap();
    let layers = overlay_folders(&base, &visual, "Visual", true);
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = Arc::clone(&log);
    let traced = layers.with_read_trace(Arc::new(move |read| kept.lock().unwrap().push(read)));
    assert_eq!(
        crate::defines::read(&traced)
            .unwrap()
            .number("NGraphics", "X")
            .unwrap(),
        2.0
    );
    let updated = b"NDefines.NGraphics.X = 3";
    fs::write(&visual_script, updated).unwrap();
    log.lock().unwrap().clear();
    assert_eq!(
        crate::defines::read(&traced)
            .unwrap()
            .number("NGraphics", "X")
            .unwrap(),
        3.0
    );
    assert!(log.lock().unwrap().iter().any(|read| {
        read.path == b"common/defines/zz_visual.lua"
            && matches!(
                read.outcome,
                ReadOutcome::Bytes { digest, .. } if digest == <[u8; 32]>::from(Sha256::digest(updated))
            )
    }));
    let parent_script = b"NDefines.NGraphics.X = 4";
    fs::write(&visual_script, parent_script).unwrap();
    log.lock().unwrap().clear();
    let parent_value = traced
        .product("parent over unscoped visual defines", 1, &[], |reader| {
            let defines = crate::defines::read(reader)?;
            Ok(defines.number("NGraphics", "X").unwrap() as u32)
        })
        .unwrap();
    assert_eq!(*parent_value, 4);
    let visual_reads: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .filter(|read| read.path == b"common/defines/zz_visual.lua")
        .cloned()
        .collect();
    assert!(!visual_reads.is_empty());
    assert!(visual_reads.iter().all(|read| {
        matches!(
            read.outcome,
            ReadOutcome::Bytes { digest, .. } if digest == <[u8; 32]>::from(Sha256::digest(parent_script))
        )
    }));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn concurrent_calls_on_same_traced_reader_never_duplicate_reads() {
    let root = std::env::temp_dir().join(format!(
        "concurrent-traced-credit-{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let base = root.join("base");
    fs::create_dir_all(base.join("common/defines")).unwrap();
    fs::write(
        base.join("common/defines/00_base.lua"),
        b"NDefines = { Synthetic = { Value = 1 } }",
    )
    .unwrap();
    let _generation = Generation::enter();
    let source = ResolvedLayers::from_folders(&[(&base, &[])]).unwrap();
    for round in 0..4 {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = Arc::clone(&log);
        let traced = source
            .with_read_trace(Arc::new(move |read| kept.lock().unwrap().push(read)))
            .with_read_purpose(format!("round-{round}"));
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let traced = &traced;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    assert_eq!(
                        crate::defines::read(traced)
                            .unwrap()
                            .number("Synthetic", "Value")
                            .unwrap(),
                        1.0
                    );
                });
            }
        });
        let define_reads = log
            .lock()
            .unwrap()
            .iter()
            .filter(|read| read.path == b"common/defines/00_base.lua")
            .count();
        assert_eq!(define_reads, 1, "round {round}");
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn unscoped_defines_match_scoped_error_and_trace_order() {
    let root = std::env::temp_dir().join(format!(
        "defines-error-order-{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let base = root.join("base");
    let visual = root.join("visual");
    fs::create_dir_all(base.join("common/defines")).unwrap();
    fs::create_dir_all(visual.join("common/defines")).unwrap();
    let base_first = base.join("common/defines/00_first.lua");
    let base_second = base.join("common/defines/01_second.lua");
    fs::write(&base_first, b"NDefines = { NGraphics = { X = 1 } }").unwrap();
    fs::write(&base_second, b"NDefines.NGraphics.X = 2").unwrap();
    let visual_first = visual.join("common/defines/00_first.lua");
    let visual_second = visual.join("common/defines/01_second.lua");
    fs::write(&visual_first, b"NDefines = { NGraphics = { X = 10 } }").unwrap();
    fs::write(&visual_second, b"NDefines.NGraphics.X = 20").unwrap();
    let build_stack = || {
        let layers = overlay_folders(&base, &visual, "Visual", true);
        layers
    };
    let read_traced = |layers: &ResolvedLayers| {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = Arc::clone(&log);
        let traced = layers.with_read_trace(Arc::new(move |read| kept.lock().unwrap().push(read)));
        let err = crate::defines::read(&traced).unwrap_err().to_string();
        let paths: Vec<LayerPath> = log
            .lock()
            .unwrap()
            .iter()
            .map(|read| read.path.clone())
            .collect();
        (err, paths)
    };
    let warm_log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept_warm = Arc::clone(&warm_log);
    let warm_traced =
        build_stack().with_read_trace(Arc::new(move |read| kept_warm.lock().unwrap().push(read)));
    assert_eq!(
        crate::defines::read(&warm_traced)
            .unwrap()
            .number("NGraphics", "X")
            .unwrap(),
        20.0
    );
    let cold_unscoped = build_stack();
    let (scoped_err, scoped_paths) = {
        let _generation = Generation::enter();
        let scoped = build_stack();
        fs::write(&visual_first, b"NDefines = {").unwrap();
        fs::remove_file(&visual_second).unwrap();
        fs::remove_file(&base_second).unwrap();
        read_traced(&scoped)
    };
    let (cold_err, cold_paths) = read_traced(&cold_unscoped);
    warm_log.lock().unwrap().clear();
    let warm_err = crate::defines::read(&warm_traced).unwrap_err().to_string();
    let warm_paths: Vec<LayerPath> = warm_log
        .lock()
        .unwrap()
        .iter()
        .map(|read| read.path.clone())
        .collect();
    assert_eq!(cold_err, scoped_err);
    assert_eq!(warm_err, scoped_err);
    assert!(
        scoped_err.contains("common/defines/00_first.lua"),
        "{scoped_err}"
    );
    assert_eq!(scoped_paths, vec![b"common/defines/00_first.lua".to_vec()]);
    assert_eq!(cold_paths, scoped_paths);
    assert_eq!(warm_paths, scoped_paths);
    fs::write(&visual_second, b"NDefines.NGraphics.X = 20").unwrap();
    fs::write(&base_second, b"NDefines.NGraphics.X = 2").unwrap();
    let (present_cold_err, present_cold_paths) = read_traced(&cold_unscoped);
    warm_log.lock().unwrap().clear();
    let present_warm_err = crate::defines::read(&warm_traced).unwrap_err().to_string();
    let present_warm_paths: Vec<LayerPath> = warm_log
        .lock()
        .unwrap()
        .iter()
        .map(|read| read.path.clone())
        .collect();
    assert_eq!(present_cold_err, scoped_err);
    assert_eq!(present_warm_err, scoped_err);
    assert_eq!(present_cold_paths, scoped_paths);
    assert_eq!(present_warm_paths, scoped_paths);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn presentation_defines_merge_the_mods_scripts_natively() {
    let machine = Machine::new("defines");
    machine.write(
        "base/common/defines/00_defines.lua",
        b"NDefines={NGraphics={TREES=1},NMilitary={WIDTH=2}}",
    );
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write(
        "art/common/defines/00_defines.lua",
        b"NDefines={NGraphics={TREES=5},NMilitary={WIDTH=9}}",
    );
    machine.write(
        "art/common/defines/01_art.lua",
        b"NDefines.NGraphics.TREES = NDefines.NGraphics.TREES * 2",
    );
    let mut layers = machine.recorded(&["base"]);
    assert!(layers.presentation_defines().is_none());
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let recorded = crate::defines::read(&layers).unwrap();
    let shown = crate::defines::read(layers.presentation()).unwrap();
    let art = crate::defines::read(layers.presentation_defines().unwrap()).unwrap();
    assert_eq!(recorded.number("NGraphics", "TREES").unwrap(), 1.0);
    assert_eq!(shown.number("NGraphics", "TREES").unwrap(), 1.0);
    assert_eq!(art.number("NGraphics", "TREES").unwrap(), 10.0);
    assert_eq!(art.number("NMilitary", "WIDTH").unwrap(), 9.0);
    assert_eq!(recorded.number("NMilitary", "WIDTH").unwrap(), 2.0);
}
