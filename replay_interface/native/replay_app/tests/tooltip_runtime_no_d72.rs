
use std::path::PathBuf;

/// use selected-Present history as geometry, ownership, content, or placement
#[test]
fn runtime_adapter_has_only_observer_geometry_and_effective_vfs_dependencies() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let module = std::fs::read_to_string(manifest_dir.join("src/tooltip_runtime.rs"))
        .expect("replay_app/src/tooltip_runtime.rs is readable");

    let (production, _tests) = module
        .split_once("#[cfg(test)]")
        .expect("tooltip_runtime.rs retains an explicit production/test boundary");
    assert!(
        !production.is_empty(),
        "tooltip_runtime.rs production half is empty"
    );

    for forbidden in [
        "replay_core",
        "selected_present",
        "AuthenticatedUiPass",
        "UiPassOperation",
        "recorded_ui_owner",
        "d72",
        "pass:",
        "pass.",
        "pass,",
        "ResolvedTooltipExactView",
        "binding_from_exact_view(",
        "tooltip_binding_from_frame(",
        "pov_tag",
        "pov_country_or_global",
        "final_resolved_text",
        "screen_rect",
    ] {
        assert!(
            !production.contains(forbidden),
            "viewer composition adapter reintroduced forbidden dependency {forbidden:?}"
        );
    }

    assert!(production.contains("pub fn xui05_product_route_status()"));
    assert!(production.contains("available: true"));
    for required in [
        "authenticated XUI02/XUI04 hovered authored node",
        "unscoped pdx_tooltip ordinary disabled and delayed declarations",
        "viewer-clock dwell hide and country-or-seek reset",
        "effective admitted localisation font chrome and clamp assets",
        "custom provider without a packet-owned exact semantic payload",
        "scoped or runtime-substituted localisation without evaluation context",
    ] {
        assert!(
            production.contains(required),
            "blocked status omitted external contract {required:?}"
        );
    }
    for reopened in [
        "pub fn prepare_viewer_tooltip(",
        "pub struct ObserverAuthoredWidgetGeometry",
        "pub struct ObserverWidgetGeometryFrame",
        "prepare_composed_tooltip(",
        "discover_authored_unscoped_tooltip(",
    ] {
        assert!(
            !production.contains(reopened),
            "replay_app reopened sealed XUI05 authority {reopened:?}"
        );
    }
}

#[test]
fn product_runtime_composes_authenticated_tooltip_last_without_d72() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime = std::fs::read_to_string(manifest_dir.join("src/ui_runtime.rs"))
        .expect("replay_app/src/ui_runtime.rs is readable")
        .replace("\r\n", "\n");
    let execute = runtime
        .split_once("impl ProductObserverUiRuntime {")
        .expect("product runtime implementation")
        .1
        .split_once("pub fn invalidate(&mut self)")
        .expect("product execute boundary")
        .0;
    let compose = execute
        .find("compose_authenticated_xui05_tooltip(")
        .expect("authenticated XUI05 composition");
    let upload = execute
        .find(".image\n            .set_rgba(gfx, &framebuffer)")
        .expect("single framebuffer upload");
    assert!(compose < upload, "XUI05 must be blended before the one upload");
    assert!(execute[compose..upload].contains("manager-overlay-composited-last"));
    // Scanned over the whole shipping half of the module, not the execute slice:
    // a helper declared after `invalidate` and called from execute under a new
    // name would otherwise carry the carrier in unseen. The last three tokens are
    // every entry point that reaches `frame.resolved_tooltip_exact()`; each reads
    // it without naming it at the call site.
    let production = runtime
        .split_once("\n#[cfg(test)]\n")
        .expect("ui_runtime.rs has a top-level test module")
        .0;
    for forbidden in [
        "resolved_tooltip_exact",
        "selected_present",
        "_r2_resolved_tooltip_instances_live",
        "tooltip_binding_from_frame(",
        "tooltip_binding_from_exact_view(",
        "admit_xui05_tooltip_route(",
    ] {
        assert!(
            !production.contains(forbidden),
            "product runtime reached the d72 carrier through {forbidden:?}"
        );
    }
}

#[test]
fn composed_route_measures_and_draws_from_one_engine_text_plan() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let hud = std::fs::read_to_string(manifest_dir.join("../game_ui/src/hud.rs"))
        .expect("game_ui/src/hud.rs is readable")
        .replace("\r\n", "\n");
    let prepare_start = hud
        .find("pub(crate) fn prepare_composed_tooltip(")
        .expect("composed tooltip preparation");
    let prepare_end = hud[prepare_start..]
        .find("\nfn resolve_tooltip_inline_icons(")
        .map(|offset| prepare_start + offset)
        .expect("inline-icon resolver follows preparation");
    let prepare = &hud[prepare_start..prepare_end];
    assert!(prepare.contains("layout_engine_text_with_icons("));
    assert!(prepare.contains("let shell_width = text_layout.total_width;"));
    assert!(prepare.contains("let shell_height = text_layout.total_height;"));
    assert!(!prepare.contains("refuse_rows_requiring_wrap_from_authored("));

    let compose_start = hud
        .find("pub(crate) fn compose_prepared_tooltip_layer(")
        .expect("composed tooltip raster path");
    let compose_end = hud[compose_start..]
        .find("\npub fn compose_hud(")
        .map(|offset| compose_start + offset)
        .expect("composed raster path boundary");
    let compose = &hud[compose_start..compose_end];
    assert!(compose.contains("for glyph in &prepared.text_layout.glyphs"));
    assert!(compose.contains(".text_layout\n        .inline_icons"));
    assert!(!compose.contains("layout_engine_text"));
}
