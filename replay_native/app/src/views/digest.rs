//! RENDER-PLAN DIGEST + the byte-identical migration proof (Codex (f)).
//!
//! The spec requires the 5 migrated surfaces to render AND scrub byte-identical after
//! the registry/host migration. A golden PNG needs a GPU + a game install; this digest
//! proves the SAME thing headlessly and everywhere: it serialises the ORDERED render
//! plan a view's `bind` produces (every node's name + kind + rect + text, in draw
//! order) and asserts:
//!   1. the MIGRATED view's plan equals the OLD direct-bind plan (no silent change), and
//!   2. scrub A→B→A reproduces A's plan byte-identically (the immutable-LRU rule, now
//!      proven through the host's per-view `bind`).
//!
//! Digesting the render PLAN (pre-GPU) is stronger than a PNG diff (Codex (f)): it
//! catches a reordered root / changed rect / dropped text that a lossy raster might hide.

use gui_runtime::scene::{ElementKind, Node};

use crate::host::view::BoundView;

/// Serialise a laid-out scene-graph into a stable, comparable string: one line per node
/// in DRAW order (depth-first, children in declaration order), carrying name + kind +
/// rect + any text. Two plans are byte-identical iff this string is equal.
pub fn nodes_digest(roots: &[Node]) -> String {
    let mut s = String::new();
    for root in roots {
        walk(root, 0, &mut s);
    }
    s
}

fn walk(n: &Node, depth: usize, s: &mut String) {
    let kind = match &n.kind {
        ElementKind::Container { background } => format!("Container(bg={background:?})"),
        ElementKind::Background { sprite } => format!("Background({sprite})"),
        ElementKind::Icon { sprite, frame } => format!("Icon({sprite},{frame})"),
        ElementKind::Button {
            sprite,
            frame,
            text,
        } => {
            format!(
                "Button({sprite:?},{frame},text={:?})",
                text.as_ref().map(|t| t.text.clone())
            )
        }
        ElementKind::Text(t) => format!("Text({:?},align={:?})", t.text, t.align),
        ElementKind::NineSlice { sprite } => format!("NineSlice({sprite})"),
        ElementKind::ProgressBar { sprite, fraction } => {
            format!("ProgressBar({sprite},{fraction})")
        }
    };
    s.push_str(&format!(
        "{:indent$}{name}|{kind}|{x:.2},{y:.2},{w:.2},{h:.2}\n",
        "",
        indent = depth * 2,
        name = n.name,
        kind = kind,
        x = n.rect.x,
        y = n.rect.y,
        w = n.rect.w,
        h = n.rect.h,
    ));
    for c in &n.children {
        walk(c, depth + 1, s);
    }
}

/// The full render-plan digest of a bound view: the hit-model roots (the laid-out scene
/// graph) + the fidelity tally + the `Drawable::draw_digest` (the EXTRA pixel inputs a
/// custom draw carries that the node graph does not — the map LUT-bytes hash + camera,
/// the topbar flag identity, the state-panel flags/gridbox/occupation, the combat bubble
/// positions + open panel, the focus placed-node positions + camera). Together these
/// cover EVERY input that determines the surface's pixels, so two bound views render the
/// same iff this digest is equal — the headless byte-identity proof for ALL 5 surfaces,
/// including the GPU-side overlays whose node roots are empty (Codex (c) mitigation).
pub fn bound_digest(bv: &BoundView) -> String {
    let tooltip = bv
        .tooltip_plan()
        .map(|p| p.digest())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "VIEW {}\nFID e={} d={} u={}\nNODES\n{}DRAW {}\nTOOLTIP {}",
        bv.view_id,
        bv.fidelity.exact,
        bv.fidelity.derived,
        bv.fidelity.unavailable,
        nodes_digest(&bv.hit.roots),
        bv.drawable.draw_digest(),
        tooltip,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{BindContext, Stage};
    use crate::host::view::{RebindContext, ViewInstance};
    use crate::views::state_panel::StatePanelView;
    use crate::views::topbar::TopbarView;
    use gui_runtime::{GuiFile, Loc, Rect};
    use std::collections::HashMap;

    // A tiny topbar `.gui` + province_state `.gui` so the migrated views bind against a
    // real scene-graph WITHOUT a game install. The element NAMES match the binding spec
    // so `apply_topbar` / `bind_state` substitute into them.
    fn topbar_gui() -> GuiFile {
        GuiFile::parse(
            r#"{"view":"t","windows":[
              {"type":"containerWindowType","name":"topbar","position":{"x":0,"y":0},
               "size":{"width":"100%%","height":"100%%"},
               "children":[
                 {"type":"instantTextBoxType","name":"pol_power","position":{"x":10,"y":4},
                  "orientation":"UPPER_LEFT","text":{"text":"PP","font":"hoi_16mbs"}},
                 {"type":"instantTextBoxType","name":"stability","position":{"x":80,"y":4},
                  "orientation":"UPPER_LEFT","text":{"text":"ST","font":"hoi_16mbs"}},
                 {"type":"instantTextBoxType","name":"experience_value","position":{"x":160,"y":4},
                  "orientation":"UPPER_LEFT","text":{"text":"XP","font":"hoi_16mbs"}}
               ]}
            ]}"#,
        )
        .unwrap()
    }

    fn panel_gui() -> GuiFile {
        GuiFile::parse(
            r#"{"view":"p","windows":[
              {"type":"containerWindowType","name":"countrystateview","position":{"x":0,"y":0},
               "size":{"width":436,"height":496},
               "children":[
                 {"type":"instantTextBoxType","name":"state_name","position":{"x":10,"y":4},
                  "orientation":"UPPER_LEFT","text":{"text":"NAME","font":"hoi_16mbs"}},
                 {"type":"instantTextBoxType","name":"victory_points","position":{"x":10,"y":30},
                  "orientation":"UPPER_LEFT","text":{"text":"VP","font":"hoi_16mbs"}}
               ]}
            ]}"#,
        )
        .unwrap()
    }

    fn stage() -> Stage {
        let mut gui_files = HashMap::new();
        gui_files.insert("topbar.json".to_string(), topbar_gui());
        gui_files.insert("province_state.json".to_string(), panel_gui());
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: HashMap::new(),
            },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn snap_a() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12", "hour_index": 100,
            "countries": { "GER": { "politics": { "political_power": 10.0, "stability": 0.5, "ruling_party": "fascism" } } },
            "states": { "64": { "owner": "GER", "controller": "GER", "victory_points_total": 5,
                "category": "city", "manpower_pool": {"total": 1000}, "buildings": {"infrastructure": 3},
                "occupation": null, "occupation_law": null, "resources": null } }
        }))
        .unwrap()
    }

    fn snap_b() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12", "hour_index": 200,
            "countries": { "GER": { "politics": { "political_power": 99.0, "stability": 0.9, "ruling_party": "democratic" } } },
            "states": { "64": { "owner": "GER", "controller": "SOV", "victory_points_total": 5,
                "category": "city", "manpower_pool": {"total": 900}, "buildings": {"infrastructure": 3, "arms_factory": 2},
                "occupation": {"resistance": 10.0, "compliance": 50.0}, "occupation_law": null, "resources": null } }
        }))
        .unwrap()
    }

    fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot, date: &'a str) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country: "GER",
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date,
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    /// The migrated TOPBAR view's render plan must equal a direct `apply_topbar` plan
    /// (no silent change in the migration).
    #[test]
    fn topbar_migration_matches_direct_bind() {
        let stage = stage();
        let snap = snap_a();
        // direct (old) path: build the scene + apply_topbar + layout.
        let direct = {
            use bind::{apply_topbar, TopbarSpec};
            use gui_runtime::{build_scene, layout, ManifestContent};
            let mut roots = build_scene(
                stage.gui_file("topbar.json").unwrap(),
                &stage.manifest,
                &stage.loc,
            );
            let spec = TopbarSpec::topbar();
            let _ = apply_topbar(&mut roots, &spec, &snap, "GER");
            let cm = ManifestContent {
                manifest: &stage.manifest,
                measure: |_t, _f| None,
            };
            for r in roots.iter_mut() {
                layout(
                    r,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 1600.0,
                        h: 900.0,
                    },
                    None,
                    &cm,
                );
            }
            nodes_digest(&roots)
        };
        // migrated path: the view's bind.
        let view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap, "1940.1.1.12"));
        let migrated = nodes_digest(&bv.hit.roots);
        assert_eq!(direct, migrated, "topbar migration changed the render plan");
        // the blocked field (experience_value) badged not-captured "—", never a 0.
        assert!(
            migrated.contains('—'),
            "the unavailable topbar field must render the not-captured glyph"
        );
    }

    /// The migrated STATE PANEL view's plan is stable + the scrub A→B→A is byte-
    /// identical (the immutable-LRU rule, now through the host's per-view bind).
    #[test]
    fn state_panel_scrub_a_b_a_byte_identical() {
        let stage = stage();
        let (a, b) = (snap_a(), snap_b());
        let view = StatePanelView::new(64, "Brandenburg");

        let da1 = bound_digest(&view.bind(&ctx(&stage, &a, "1940.1.1.12")));
        let db = bound_digest(&view.bind(&ctx(&stage, &b, "1941.1.1.12")));
        assert_ne!(
            da1, db,
            "A and B must differ, else the scrub test proves nothing"
        );
        let da2 = bound_digest(&view.bind(&ctx(&stage, &a, "1940.1.1.12")));
        assert_eq!(
            da1, da2,
            "scrub A→B→A: the state panel carried a value from the B visit"
        );
    }

    /// The topbar scrub A→B→A is byte-identical (the flag ideology flips fascism→
    /// democratic between A and B — the Codex §4 cache-key trap — yet A re-binds the same).
    #[test]
    fn topbar_scrub_a_b_a_byte_identical() {
        let stage = stage();
        let (a, b) = (snap_a(), snap_b());
        let view = TopbarView::new("GER");
        let da1 = bound_digest(&view.bind(&ctx(&stage, &a, "1940.1.1.12")));
        let db = bound_digest(&view.bind(&ctx(&stage, &b, "1941.1.1.12")));
        assert_ne!(da1, db);
        let da2 = bound_digest(&view.bind(&ctx(&stage, &a, "1940.1.1.12")));
        assert_eq!(
            da1, da2,
            "scrub A→B→A: the topbar carried a value from the B visit"
        );
    }

    /// FABRICATION GUARD (Codex (a)): no CLUSTER view module may turn a snapshot value
    /// into a fabricated `0`/`""` default. The audited raw `snapshot()` seam is used
    /// only by the migrated Core surfaces (which drive the already-no-fabrication
    /// `bind::apply_*` layer); a cluster file that reaches for `.unwrap_or(0)` /
    /// `.unwrap_or_default()` on a snapshot value is the exact hole this guard closes.
    /// (Source-level audit so the guarantee is enforced, not merely conventional.)
    #[test]
    fn no_fabricated_default_in_cluster_views() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/views/clusters");
        let mut offenders = Vec::new();
        // RECURSE (nested cluster submodules count too) over every .rs under clusters/.
        let files = rs_files_recursive(&dir);
        for path in files {
            let src = std::fs::read_to_string(&path).unwrap();
            for (lineno, raw) in src.lines().enumerate() {
                // strip line comments + whitespace so the worked-example PROSE in
                // politics_government (which mentions the patterns) is not a false hit,
                // and whitespace variants (`unwrap_or( 0 )`) are still caught.
                let code: String = raw
                    .split("//")
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect();
                // (1) fabricated numeric/string defaults from a snapshot value.
                for pat in [
                    "unwrap_or(0)",
                    "unwrap_or(0.0)",
                    "unwrap_or(0i",
                    "unwrap_or(0u",
                    "unwrap_or_default()",
                    "unwrap_or(\"\")",
                    "unwrap_or(String::new())",
                    "map_or(0,",
                    "map_or(0.0,",
                    "unwrap_or_else(||0)",
                    "unwrap_or_else(||0.0)",
                ] {
                    if code.contains(pat) {
                        offenders.push(format!(
                            "{}:{} fabricated-default `{}`",
                            path.display(),
                            lineno + 1,
                            pat
                        ));
                    }
                }
                // (2) the RAW snapshot seam is Core-only: a cluster view must read fields
                // via ctx.country_field/state_field (-> Captured), NEVER ctx.snapshot()
                // or a raw Snapshot, which would bypass the no-fabrication trichotomy.
                for pat in [
                    "ctx.snapshot(",
                    ".snapshot(",
                    "ctx.snapshot.",
                    "BindContext::snapshot",
                ] {
                    if code.contains(pat) {
                        offenders.push(format!(
                            "{}:{} raw-snapshot-seam `{}` (use country_field/state_field)",
                            path.display(),
                            lineno + 1,
                            pat
                        ));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "cluster view(s) fabricate a default or bypass the no-fabrication seam: {offenders:#?}"
        );
    }

    /// Recursively collect every `.rs` file under `dir`.
    fn rs_files_recursive(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    out.extend(rs_files_recursive(&p));
                } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
        out
    }

    /// END-TO-END inventory wiring: `ViewRegistry::from_inventory()` collects the 5
    /// migrated Core views + the state-building template, registered distributed across
    /// their own files (the deliverable-6 conflict-free seam, proven through the real
    /// build — not just an isolated probe).
    #[test]
    fn inventory_collects_the_five_core_views() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        for id in [
            "core.map",
            "core.topbar",
            "core.state_panel",
            "core.focus_tree",
            "core.land_combat",
        ] {
            assert!(
                reg.view(crate::host::ViewId(id)).is_some(),
                "Core view {id} must be collected from the inventory at startup"
            );
        }
        // the state-panel building-entry TEMPLATE is registered too (deliverable 4).
        assert!(reg
            .template(crate::host::ViewId("core.state_building_entry"))
            .is_some());
        assert!(
            reg.len() >= 5,
            "registry has the 5 Core views (+ any cluster views)"
        );
    }

    /// CORE-RESERVED-LETTER guard (Codex claim-2): the binary reserves C/F/M for the
    /// combat/focus/map-mode Core glue, so a cluster view that binds one as a global
    /// `Shortcut` would silently NOT toggle. Assert no DESCRIPTOR in the inventory binds
    /// a reserved letter (so the limitation can't bite a cluster agent unnoticed).
    #[test]
    fn no_cluster_shortcut_uses_a_core_reserved_letter() {
        use crate::host::registry::Shortcut;
        let reg = crate::host::ViewRegistry::from_inventory();
        for d in reg.iter_views() {
            if let Some(Shortcut::Letter(c)) = d.shortcut {
                let up = c.to_ascii_uppercase();
                assert!(
                    up != 'C' && up != 'F' && up != 'M',
                    "view {} binds Core-reserved shortcut '{}' (C/F/M are reserved for combat/focus/map-mode)",
                    d.id, c
                );
            }
        }
    }

    /// CROSS-CLUSTER REGISTRY INTEGRITY (the inter-cluster collision a per-cluster review
    /// structurally CANNOT see). Registration is distributed via `inventory`; the registry
    /// keys on a `HashMap<ViewId, _>`, so two clusters submitting the SAME `id` (or the
    /// same template id) is silently last-wins: the shadowed descriptor's `open`/`route`/
    /// `gui_root` is overwritten, the view count drops below the submission count, and yet
    /// EVERY per-cluster `reg.view(id).is_some()` test still passes (the id still resolves —
    /// to the WRONG cluster's descriptor). Only a GLOBAL raw-vs-deduped check catches it.
    ///
    /// This guard asserts `inventory submissions == unique ids` for BOTH views and templates
    /// (no magic count — that would churn on every legitimate add/remove; the invariant is
    /// uniqueness, not an absolute number), printing the exact duplicate ids on failure.
    /// Codex-confirmed: templates are even MORE silently swallowed (no `order` tracking).
    #[test]
    fn no_duplicate_view_or_template_id_across_clusters() {
        use crate::host::registry::{TemplateDescriptor, ViewDescriptor};
        use std::collections::BTreeMap;
        let reg = crate::host::ViewRegistry::from_inventory();

        let mut view_ids: BTreeMap<&str, usize> = BTreeMap::new();
        let mut raw_views = 0usize;
        for d in inventory::iter::<ViewDescriptor> {
            *view_ids.entry(d.id.as_str()).or_insert(0) += 1;
            raw_views += 1;
        }
        let view_dups: Vec<(&str, usize)> = view_ids
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(k, c)| (*k, *c))
            .collect();

        let mut tmpl_ids: BTreeMap<&str, usize> = BTreeMap::new();
        let mut raw_tmpls = 0usize;
        for t in inventory::iter::<TemplateDescriptor> {
            *tmpl_ids.entry(t.id.as_str()).or_insert(0) += 1;
            raw_tmpls += 1;
        }
        let tmpl_dups: Vec<(&str, usize)> = tmpl_ids
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(k, c)| (*k, *c))
            .collect();

        assert!(
            view_dups.is_empty(),
            "duplicate ViewId across clusters — the colliding cluster silently SHADOWS the \
             other (last-wins HashMap), and the shadowed cluster's own register test still \
             passes. Dups (id, times submitted): {view_dups:#?}"
        );
        assert!(
            tmpl_dups.is_empty(),
            "duplicate template id across clusters (silently overwritten in the template \
             HashMap): {tmpl_dups:#?}"
        );
        // raw submissions must equal the deduped registry length (the no-silent-shadow
        // invariant, independent of any absolute count).
        assert_eq!(
            raw_views,
            reg.len(),
            "inventory submitted {raw_views} ViewDescriptors but the registry holds only \
             {} — a duplicate id was silently swallowed",
            reg.len()
        );
        assert_eq!(
            raw_tmpls,
            reg.template_count(),
            "inventory submitted {raw_tmpls} TemplateDescriptors but the registry holds only \
             {} — a duplicate template id was silently swallowed",
            reg.template_count()
        );
    }

    /// `rebind` is a no-op for plain windows, so binding before/after a rebind to the
    /// SAME hour is identical (the host calls rebind every tick; it must be idempotent
    /// for the hour — the deliverable-7 invariant for plain windows).
    #[test]
    fn plain_window_rebind_is_idempotent() {
        let stage = stage();
        let a = snap_a();
        let mut view = TopbarView::new("GER");
        let before = bound_digest(&view.bind(&ctx(&stage, &a, "1940.1.1.12")));
        view.rebind(&RebindContext {
            snapshot: &a,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        });
        let after = bound_digest(&view.bind(&ctx(&stage, &a, "1940.1.1.12")));
        assert_eq!(
            before, after,
            "a rebind to the same hour must not change the plan"
        );
    }
}
