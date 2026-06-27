//! `gui_runtime` — a generic, retained-mode interpreter of the Clausewitz `.gui`
//! format (parsed to JSON offline). It turns a parsed `.gui` tree into a typed
//! SCENE-GRAPH, resolves the LAYOUT MODEL (position / size + percent / -1 auto /
//! orientation + origo 9-anchor box model / clipping), and provides HIT-TESTING
//! (alwaystransparent pass-through, z = declaration order). M2 uses it to render
//! the real the game topbar statically via the `render` crate.
//!
//! Modules:
//!   * [`model`]   — lenient serde mirror of the parsed `.gui` JSON.
//!   * [`scene`]   — typed scene-graph nodes + element kinds + the 9-anchor enum.
//!   * [`layout`]  — the layout model (the load-bearing, Codex-hardened math).
//!   * [`hittest`] — front-to-back hit-test with alwaystransparent + clipping.
//!   * [`loc`]     — minimal the game localisation (.yml) layer.
//!   * [`fonts`]   — core.gfx font-name -> BMFont registry (Latin/English set).
//!   * [`draw`]    — bridge that draws a laid-out scene-graph via `render`.
//!
//! Element kinds implemented (the topbar's passive set + the generic primitives):
//! containerWindowType/windowType/positionType, iconType, instantTextBoxType/
//! textBoxType, buttonType/guiButtonType, background, corneredTileSpriteType
//! (9-slice, promoted from the manifest), progressbartype. Unknown kinds degrade
//! to a coordinate-frame container so any view still lays out + recurses.

pub mod draw;
pub mod fonts;
pub mod gridbox;
pub mod hittest;
pub mod layout;
pub mod loc;
pub mod markup;
pub mod model;
pub mod scene;

pub use gridbox::{instantiate, GridFlow, GridInstances, GridSpec, ScrollBar};
pub use hittest::{hit_test, hit_test_stack, tooltip_at, tooltip_at_stack, Hit};
pub use layout::{layout, ContentSize, ManifestContent};
pub use loc::Loc;
pub use markup::{expand, wrap, Expanded, Span, Wrapped, WrappedLine};
pub use model::GuiFile;
pub use scene::{Anchor, ElementKind, Node, Rect, TextAlign, TextSpec};

use assets::SpriteManifest;

/// Build a retained scene-graph (one root [`Node`] per top-level window) from a
/// parsed `.gui` file, resolving text loc keys and promoting 9-slice/progressbar
/// kinds against the sprite manifest. Layout is NOT yet applied — call
/// [`layout::layout`] on each root with the screen rect.
pub fn build_scene(file: &GuiFile, manifest: &SpriteManifest, loc: &Loc) -> Vec<Node> {
    file.windows
        .iter()
        .map(|w| Node::from_raw(w, manifest, loc))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Coord, RawNode};

    fn unresolved_position_tokens(file: &GuiFile) -> (usize, Vec<String>) {
        fn walk(node: &RawNode, unresolved_anchors: &mut usize, unknown_raw: &mut Vec<String>) {
            if let Some(pos) = node.position.as_ref() {
                for coord in [&pos.x, &pos.y] {
                    match coord {
                        Coord::Anchor { resolved: None, .. } => *unresolved_anchors += 1,
                        Coord::UnknownRaw(raw) => unknown_raw.push(raw.clone()),
                        _ => {}
                    }
                }
            }
            for child in &node.children {
                walk(child, unresolved_anchors, unknown_raw);
            }
        }

        let mut unresolved_anchors = 0;
        let mut unknown_raw = Vec::new();
        for window in &file.windows {
            walk(window, &mut unresolved_anchors, &mut unknown_raw);
        }
        unknown_raw.sort();
        (unresolved_anchors, unknown_raw)
    }

    #[test]
    fn build_and_layout_a_tiny_view() {
        let j = r#"{"view":"t","windows":[
          {"type":"containerWindowType","name":"root","position":{"x":-4,"y":-1},
           "size":{"width":"100%%","height":"100%%"},
           "children":[
             {"type":"iconType","name":"bg","position":{"x":0,"y":0},
              "orientation":"UPPER_LEFT","sprite":"GFX_x"},
             {"type":"instantTextBoxType","name":"label","position":{"x":10,"y":4},
              "orientation":"UPPER_LEFT",
              "text":{"text":"LOBBY","font":"hoi_16mbs","format":"centre","maxWidth":57,"maxHeight":24}}
           ]}
        ]}"#;
        let file = GuiFile::parse(j).unwrap();
        let manifest = SpriteManifest {
            sprites: std::collections::HashMap::new(),
        };
        let mut loc = Loc::new();
        loc.add_yml(" LOBBY: \"Lobby\"\n");
        let mut scene = build_scene(&file, &manifest, &loc);
        assert_eq!(scene.len(), 1);
        assert_eq!(scene[0].count(), 3);
        // the label resolved its loc key
        let label = &scene[0].children[1];
        if let ElementKind::Text(t) = &label.kind {
            assert_eq!(t.text, "Lobby");
            assert_eq!(t.align, TextAlign::Center);
        } else {
            panic!("label not text");
        }
        // layout the root at a 1920x1080 screen
        let screen = Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let content = ManifestContent {
            manifest: &manifest,
            measure: |_t: &str, _f: &str| None,
        };
        layout(&mut scene[0], screen, None, &content);
        // root right edge lands on the screen edge (percent + negative-pos rule)
        let r = scene[0].rect;
        assert_eq!(r.x + r.w, 1920.0);
        // hit-test the label area returns the label (topmost over bg-less point)
        let hit = hit_test(&scene[0], 12.0, 6.0).unwrap();
        assert_eq!(hit.leaf(), Some("label"));
    }

    /// FIX 2 — the 4 real `.gui` files the shared parser PREVIOUSLY REJECTED
    /// (`politics.json` carries `position.y = "1s"` + `text.maxHeight = "100%%"`; the
    /// 3 designer files carry `@named` position anchors) now parse cleanly through the
    /// SHARED `GuiFile::parse` with NO cluster-local sanitizer. Skipped (not failed)
    /// when the gitignored game assets are not extracted.
    #[test]
    fn previously_rejected_gui_files_parse_via_shared_parser() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui");
        if !dir.join("politics.json").exists() {
            eprintln!("[gui_runtime] skipped: game_assets/gui not extracted");
            return;
        }
        let manifest = SpriteManifest {
            sprites: std::collections::HashMap::new(),
        };
        let loc = Loc::new();
        for f in [
            "politics.json",
            "tank_designer.json",
            "plane_designer.json",
            "equipment_designer.json",
        ] {
            let path = dir.join(f);
            let g = GuiFile::load(&path)
                .unwrap_or_else(|e| panic!("{f} must parse via the shared parser now: {e}"));
            assert!(
                !g.windows.is_empty(),
                "{f} parsed to a populated window set"
            );
            // and build a scene-graph from each (the form classification flows all
            // the way through `from_raw` without panicking).
            let scene = build_scene(&g, &manifest, &loc);
            assert!(!scene.is_empty(), "{f} built a scene-graph");
        }

        // STRONGER: the SHARED parser must now swallow the ENTIRE extracted `.gui`
        // corpus (every view's real `.gui`) with no local fallback — no value form
        // rejects the file. Scan all *.json (except the `_index.json` manifest).
        let mut parsed = 0usize;
        for entry in std::fs::read_dir(&dir).expect("read gui dir") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let fname = path.file_name().unwrap().to_string_lossy().to_string();
            if fname == "_index.json" {
                continue;
            }
            let g = GuiFile::load(&path)
                .unwrap_or_else(|e| panic!("shared parser rejected {fname}: {e}"));
            let (unresolved_anchors, unknown_raw) = unresolved_position_tokens(&g);
            assert_eq!(
                unresolved_anchors, 0,
                "{fname} has unresolved @position anchors; rerun game_assets/extract.py so its defines table is regenerated"
            );
            let allowed_unknown_raw = if fname == "tech_tree.json" {
                vec!["275k".to_string()]
            } else {
                Vec::new()
            };
            assert_eq!(
                unknown_raw, allowed_unknown_raw,
                "{fname} has unexpected raw position tokens; investigate before they fall back to chrome 0.0 geometry"
            );
            let _ = build_scene(&g, &manifest, &loc);
            parsed += 1;
        }
        assert!(
            parsed > 50,
            "expected the full gui corpus, parsed only {parsed}"
        );
    }
}
