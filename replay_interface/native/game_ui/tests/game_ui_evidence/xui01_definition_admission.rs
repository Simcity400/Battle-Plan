use game_ui::definition_admission::{
    frozen_playset_files, load_frozen_definition_catalog, FROZEN_FILE_COUNT, FROZEN_WINDOW_COUNT,
};
use std::path::PathBuf;

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> PathBuf {
    std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT))
}

#[test]
fn frozen_representative_corpus_has_nonzero_exact_admission() {
    let root = game_root();
    assert!(
        root.is_dir(),
        "XUI01 requires the retained the game install at {}",
        root.display()
    );
    let rows = frozen_playset_files(&root).expect("frozen playset rows");
    assert_eq!(rows.len(), FROZEN_FILE_COUNT);
    let catalog = load_frozen_definition_catalog(rows).expect("production definition admission");

    assert_eq!(catalog.files.len(), FROZEN_FILE_COUNT);
    assert_eq!(catalog.total_window_count(), FROZEN_WINDOW_COUNT);
    assert_eq!(catalog.admitted_file_count(), 454);
    assert_eq!(catalog.rejected_file_count(), 0);
    assert_eq!(catalog.admitted_windows.len(), 9_155);
    assert_eq!(catalog.rejected_windows.len(), 0);
    println!(
        "XUI01 corpus: admitted_files={} rejected_files={} admitted_windows={} rejected_windows={}",
        catalog.admitted_file_count(),
        catalog.rejected_file_count(),
        catalog.admitted_windows.len(),
        catalog.rejected_windows.len()
    );
    let dependency_windows = catalog
        .admitted_windows
        .iter()
        .filter(|row| !row.downstream_runtime_dependencies.is_empty())
        .collect::<Vec<_>>();
    let dependency_files = dependency_windows
        .iter()
        .map(|row| row.source_file.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(dependency_windows.len(), 190);
    assert_eq!(dependency_files.len(), 77);
    let provisional_zero_admitted_files = catalog
        .files
        .iter()
        .filter_map(|file| {
            let dependency_count = dependency_windows
                .iter()
                .filter(|row| row.source_file == file.source_file)
                .count();
            (file.admitted_windows == dependency_count).then_some((file, dependency_count))
        })
        .collect::<Vec<_>>();
    assert_eq!(provisional_zero_admitted_files.len(), 6);
    for (file, dependency_count) in provisional_zero_admitted_files {
        let windows = dependency_windows
            .iter()
            .filter(|row| row.source_file == file.source_file)
            .collect::<Vec<_>>();
        let owners = windows
            .iter()
            .flat_map(|row| row.downstream_runtime_dependencies.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        println!(
            "XUI01 corrected provisional zero-admission file: {} windows={} definition-only-dependency-windows={} owners={:?}",
            file.source_file,
            file.admitted_windows,
            dependency_count,
            owners
        );
    }

    for representative in [
        "interface/topbar.gui",
        "interface/countrypoliticsview.gui",
        "interface/mtg_music.gui",
    ] {
        assert!(
            catalog
                .admitted_windows
                .iter()
                .any(|row| row.source_file == representative),
            "representative GUI file had no admitted window: {representative}"
        );
    }
}

#[test]
fn frozen_namespace_drift_is_rejected_before_parse() {
    let root = game_root();
    let mut rows = frozen_playset_files(&root).expect("frozen playset rows");
    rows.pop();
    let error = load_frozen_definition_catalog(rows).expect_err("missing file must reject");
    assert!(error.to_string().contains("namespace drift"), "{error}");
}

#[test]
fn case_colliding_effective_path_is_rejected() {
    let root = game_root();
    let mut rows = frozen_playset_files(&root).expect("frozen playset rows");
    let mut duplicate = rows[0].clone();
    duplicate.rel_path = duplicate.rel_path.to_ascii_uppercase();
    rows.push(duplicate);
    let error = load_frozen_definition_catalog(rows).expect_err("case collision must reject");
    assert!(error.to_string().contains("case-colliding"), "{error}");
}
