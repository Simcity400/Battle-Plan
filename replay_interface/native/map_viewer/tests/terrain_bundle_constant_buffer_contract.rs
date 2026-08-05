//! The REAL `validate_terrain_d3d11_bundle` chain, run over a producer-shaped
//! map-frame bundle.
//!
//! # What this restores
//!
//! `extractor/native/terrain_capture` used to dev-depend on `map_viewer` so its
//! test `map_viewer_accepts_shared_constant_buffer_naming_contract` could call
//! `map_viewer::validate_terrain_d3d11_bundle` over a bundle it had just
//! written. That dev-dependency was a replay/extractor boundary violation and
//! was removed; the replacement on the producer side
//! (`constant_buffer_repair_establishes_the_names_the_renderer_requires`)
//! restates the consumer's naming rules in ITS OWN test code, so it can no
//! longer notice when the consumer's rules change. Nothing ran the production
//! validation chain over a bundle any more.
//!
//! This is that coverage, on the consumer side, over a fixture that belongs to
//! neither workspace (`portable_replay/native/terrain_fixture`).
//!
//! # What production change would make this fail?
//!
//! - Deleting or weakening `map_frame_reflected_constant_buffer_binding_blockers`
//!   in `map_viewer/src/frame_bundle.rs`, or unhooking it from
//!   `NativeD3d11TerrainBundle::load` — the stale bundle would stop being
//!   rejected for its names.
//! - Stopping the check at ONE draw, ONE shader stage, or ONE slot.
//!   `every_governed_binding_is_checked_individually` stales exactly one
//!   binding at a time and requires each one to be caught on its own, which an
//!   all-stale fixture cannot prove: with every site wrong at once, a validator
//!   that inspected only one of them would still reject.
//! - Changing which constant-buffer name the renderer demands at PS slot 0 or
//!   VS slot 1 without the producer's pinned literals moving with it — the
//!   canonical bundle would start being rejected for its names.
//!
//! Not claimed: `active_draw`. The fixture carries it (a copy of draw 1) because
//! a real bundle does, but `NativeD3d11TerrainBundle::load` walks `draws` only,
//! so no assertion here would notice `active_draw` going unchecked. Saying
//! otherwise would be the kind of coverage claim that is never tested.

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

/// The predicate the deleted producer-side test used, verbatim, so "the same
/// defects" means the same defects and not a nearby set.
///
/// Kept as a substring match rather than an equality: the chain reports the
/// FIRST blocker it hits, and this fixture is deliberately minimal, so which
/// non-naming complaint it would reach afterwards is not the subject here.
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
    // The complaint has to be about a slot the renderer actually needs, not
    // some incidental third binding.
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

/// Each governed binding, staled on its own, must be caught on its own.
///
/// `stale_constant_buffer_names_are_rejected_by_the_production_chain` above uses
/// a bundle where every binding is wrong at once, which is what a real
/// uncanonicalised capture looks like — but it cannot distinguish a validator
/// that checks all six sites from one that checks a single site and stops. This
/// can: the bundle is canonical everywhere except one binding, and the complaint
/// must name that binding's stage and slot.
///
/// # What production change would make this fail?
///
/// Restricting `map_frame_reflected_constant_buffer_binding_blockers` (or the
/// loop in `NativeD3d11TerrainBundle::load` that calls it) to one stage, one
/// slot, or the first draw. Each of those leaves five of the six cases below
/// loading clean.
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

/// `validate_terrain_d3d11_bundle` returns `Result<(), Box<dyn Error>>`; this
/// keeps the per-site loop above readable without swallowing the Ok case.
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

/// The negative control for the pair above.
///
/// Both tests read as evidence only if the stale and canonical bundles really
/// are the same bundle under two spellings. If a future edit made the canonical
/// variant differ in some other way, `canonical_...` could pass for a reason
/// that has nothing to do with names, and this file would quietly stop being a
/// test of the naming contract.
///
/// What production change would make this fail? Any edit to the neutral fixture
/// that makes the variants differ outside `declared_name` and the two shader
/// reflection maps.
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

/// Erases every constant-buffer name from a bundle: the `declared_name` of each
/// captured binding, and the whole reflected `constant_buffers` map of each
/// shader. What is left is everything the naming contract does not govern.
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
