
use map_viewer::validate_terrain_d3d11_bundle;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use terrain_fixture::{
    write_map_frame_bundle_fixture, BindingSite, ConstantBufferNames,
    TERRAIN_DRAW_CB1_LAYOUT_SOURCE, TERRAIN_SCENE_CB0_SYMBOL,
};

fn temp_root(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "map_viewer_terrain_cb_contract_{}_{}_{}",
        std::process::id(),
        name,
        nonce
    ));
    fs::create_dir_all(&path).expect("create temp fixture root");
    path
}

fn has_constant_buffer_naming_complaint(message: &str) -> bool {
    message.contains("expected reflected constant buffer")
        || message.contains("has no named TerrainSceneCB0 PS constant-buffer capture")
        || message.contains("constant-buffer slot") && message.contains("is captured as")
}

#[test]
fn stale_constant_buffer_names_are_rejected_by_the_production_chain() {
    let root = temp_root("stale");
    let bundle = write_map_frame_bundle_fixture(&root, ConstantBufferNames::Stale);

    let error = validate_terrain_d3d11_bundle(&bundle)
        .expect_err("a bundle whose constant buffers carry raw dx11_cbN names must not load")
        .to_string();
    assert!(
        has_constant_buffer_naming_complaint(&error),
        "the stale bundle was rejected, but not for its constant-buffer names, so this \
         fixture no longer reaches the naming checks: {error}"
    );
    assert!(
        error.contains(TERRAIN_SCENE_CB0_SYMBOL) || error.contains(TERRAIN_DRAW_CB1_LAYOUT_SOURCE),
        "the naming complaint names neither constant buffer the renderer requires: {error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn canonical_constant_buffer_names_clear_the_production_naming_checks() {
    let root = temp_root("canonical");
    let bundle = write_map_frame_bundle_fixture(&root, ConstantBufferNames::Canonical);

    if let Err(error) = validate_terrain_d3d11_bundle(&bundle) {
        let message = error.to_string();
        assert!(
            !has_constant_buffer_naming_complaint(&message),
            "the canonically-named bundle still trips the renderer's constant-buffer \
             naming contract, so the names the producer is pinned to are not the names \
             the renderer accepts: {message}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn every_governed_binding_is_checked_individually() {
    for site in BindingSite::ALL {
        let label = format!("draw{}_{}_{}", site.draw_index, site.stage, site.slot);
        let root = temp_root(&label);
        let bundle =
            write_map_frame_bundle_fixture(&root, ConstantBufferNames::CanonicalExcept(site));

        let error = validate_terrain_d3d11_bundle(&bundle)
            .unwrap_err_or_empty(&format!("bundle stale only at {label} must be rejected"));
        assert!(
            has_constant_buffer_naming_complaint(&error),
            "staling only {label} was not caught by the naming contract: {error}"
        );
        let expected = format!(
            "draw {} {} constant-buffer slot {} is captured as {:?}, expected reflected \
             constant buffer {}",
            site.draw_index,
            site.stage,
            site.slot,
            Some(site.stale_name()),
            site.canonical_name()
        );
        assert!(
            error.contains(&expected),
            "the complaint does not name the one binding that is wrong.\nexpected to \
             contain: {expected}\ngot: {error}"
        );

        let _ = fs::remove_dir_all(root);
    }
}

trait UnwrapErrOrEmpty {
    fn unwrap_err_or_empty(self, context: &str) -> String;
}

impl<E: std::fmt::Display> UnwrapErrOrEmpty for Result<(), E> {
    fn unwrap_err_or_empty(self, context: &str) -> String {
        match self {
            Ok(()) => panic!("{context}, but the bundle loaded clean"),
            Err(error) => error.to_string(),
        }
    }
}

#[test]
fn the_two_fixtures_differ_only_in_the_names_under_test() {
    let stale_root = temp_root("diff_stale");
    let canonical_root = temp_root("diff_canonical");
    let stale: Value = serde_json::from_slice(
        &fs::read(write_map_frame_bundle_fixture(
            &stale_root,
            ConstantBufferNames::Stale,
        ))
        .expect("read stale bundle"),
    )
    .expect("parse stale bundle");
    let canonical: Value = serde_json::from_slice(
        &fs::read(write_map_frame_bundle_fixture(
            &canonical_root,
            ConstantBufferNames::Canonical,
        ))
        .expect("read canonical bundle"),
    )
    .expect("parse canonical bundle");

    assert_ne!(stale, canonical, "the two variants must not be identical");
    assert_eq!(
        scrub_constant_buffer_names(&stale),
        scrub_constant_buffer_names(&canonical),
        "the stale and canonical bundles differ somewhere other than the constant-buffer \
         names, so neither test above is evidence about naming"
    );

    let _ = fs::remove_dir_all(stale_root);
    let _ = fs::remove_dir_all(canonical_root);
}

fn scrub_constant_buffer_names(bundle: &Value) -> Value {
    let mut scrubbed = bundle.clone();
    let object = scrubbed.as_object_mut().expect("bundle object");
    let mut draws: Vec<&mut Value> = Vec::new();
    for (key, value) in object.iter_mut() {
        match key.as_str() {
            "active_draw" => draws.push(value),
            "draws" => draws.extend(value.as_array_mut().expect("draws array").iter_mut()),
            _ => {}
        }
    }
    assert!(!draws.is_empty(), "the fixture must carry draws");
    for draw in draws {
        for stage in ["shader", "vertex_shader"] {
            draw[stage]["constant_buffers"] = Value::Null;
        }
        for binding in draw["constant_buffers"]
            .as_array_mut()
            .expect("draw constant_buffers array")
        {
            binding["declared_name"] = Value::Null;
        }
    }
    scrubbed
}
