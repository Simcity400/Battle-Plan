use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires MODVFS_RECORDING, BATTLE_PLAN_GAME_ROOT and BATTLE_PLAN_USER_DIR; run with the bounded whole-recording nextest profile"]
fn modvfs_whole_recording_admission_reads_all_winners_and_closes_hud() {
    let started = Instant::now();
    let bounded = || assert!(started.elapsed() < Duration::from_secs(540), "whole-recording proof exceeded 540 seconds");
    let required_path = |name| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    let recording = required_path("MODVFS_RECORDING");
    let paths = crate::install_paths::ProductPaths {
        game_install: required_path("BATTLE_PLAN_GAME_ROOT"),
        user_data: required_path("BATTLE_PLAN_USER_DIR"),
    };
    let timeline = crate::validate::open_timeline_segment_unvalidated(&recording, None)
        .unwrap_or_else(|error| panic!("{}", error.summary()));
    assert!(timeline.product_admission().is_some(), "proof requires a certified production recording");
    let hours = timeline.hours().len();
    assert!((1..=100_000).contains(&hours), "proof accepts complete captures of 1..=100000 hours");
    let mut visited = 0;
    let timing = crate::validate::validate_all_frame_payloads(&timeline, &mut |_, _| {
        bounded();
        visited += 1;
        Ok(())
    }).unwrap_or_else(|error| panic!("{}", error.summary()));
    assert_eq!(visited, hours);
    assert_eq!(timing.hours, hours);
    let manifest = timeline.content_manifest().expect("recorded content manifest").clone();
    let provider = crate::timeline::RecordingTimeline::new_with_policy(timeline, None);
    let (resolver, admission) = crate::content_compatibility::bind_provider_content(&provider, &paths).unwrap();
    let (resolver, owned_root) = resolver.isolated_content_snapshot().unwrap();
    bounded();
    let required = admission.required_paths().collect::<Vec<_>>();
    assert!((1..=100_000).contains(&required.len()), "proof accepts 1..=100000 complete content winners");
    let mut total_bytes = 0u64;
    for path in &required {
        bounded();
        let key = admission.require_used_content(path).unwrap();
        let bytes = resolver.read_admitted_used_content(key).unwrap();
        assert_eq!(bytes.len() as u64, key.payload_bytes, "{path}");
        total_bytes = total_bytes.checked_add(bytes.len() as u64).unwrap();
        assert!(total_bytes <= 64 * 1024 * 1024 * 1024, "proof content exceeds 64 GiB");
    }
    assert!(total_bytes > 0);
    let route = game_ui::production_renderer::admit_authenticated_product_hud_route(&admission)
        .unwrap().expect("recording must bind every always-on HUD surface");
    assert!(route.consumer_prefixes().count() >= 4);
    let language = portable_content_manifest::authored_ui::safe_language_directory(&manifest.localisation_language).unwrap();
    let index = game_ui::hud_role_index::emit_effective_vfs_hud_role_index(
        &resolver, &manifest.effective_vfs_manifest_sha256, &language,
    ).unwrap();
    index.validate().unwrap();
    assert_eq!(index.closures.len(), game_ui::engine_surface_roots::ENGINE_CONTROLLER_ROOTS.len());
    assert!(index.closures.iter().all(|closure| closure.complete_dependency_closure && closure.missing_roles.is_empty()));
    for content in &index.winning_content {
        assert_eq!(admission.require_used_content(&content.normalized_game_relative_path).unwrap(), content);
    }
    let assets = game_ui::assets::AssetResolver::from_content_admission(&resolver, &admission).unwrap();
    let renderer = game_ui::production_renderer::ProductionUiRenderer::new(assets, &admission).unwrap();
    bounded();
    let discovery = crate::content_compatibility::resolver_for_manifest(&paths, &manifest).unwrap();
    let unrecorded = discovery.audit_effective_dir_files_recursive("", None).unwrap()
        .into_iter().find(|path| {
            ["common/", "gfx/", "map/", "interface/", "localisation/", "history/", "music/", "sound/"]
                .iter().any(|prefix| path.starts_with(prefix)) && admission.used_content(path).is_none()
        }).expect("capture must leave an installed authored file outside its used-content manifest");
    drop(discovery);
    assert!(resolver.read_required_bytes(&unrecorded, "unrecorded proof").is_err());
    assert!(renderer.resolver().resolve_optional_file(&unrecorded).is_err(),
        "unrecorded installed content is not witnessed absent");
    bounded();
    let dependency = &index.closures[0].bindings[0].content.normalized_game_relative_path;
    let snapshot = resolver.resolve_required(dependency, "HUD dependency proof").unwrap();
    assert!(snapshot.starts_with(&owned_root));
    let mut permissions = std::fs::metadata(&snapshot).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&snapshot, permissions).unwrap();
    std::fs::remove_file(&snapshot).unwrap();
    let missing = game_ui::hud_role_index::emit_effective_vfs_hud_role_index(
        &resolver, &manifest.effective_vfs_manifest_sha256, &language,
    );
    assert!(!missing.is_ok_and(|index| index.closures.iter().all(|closure| closure.complete_dependency_closure)));
    bounded();
    drop(renderer); drop(resolver);
    assert!(!owned_root.exists());
    println!("{}", serde_json::json!({
        "hours": hours, "visited_hours": visited, "verified_winners": required.len(),
        "verified_bytes": total_bytes, "manifest_sha256": manifest.computed_digest().unwrap(),
        "refused_installed_path": unrecorded, "elapsed_seconds": started.elapsed().as_secs_f64(),
    }));
}
