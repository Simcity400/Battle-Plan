//! `focus_tree` — STAGE 3 the FOCUS-TREE ADAPTER (plan §B.5 + §E).
//!
//! Pipeline:
//!   1. [`pdx`]   — a small Clausewitz tokenizer/parser → generic AST.
//!   2. [`graph`] — `focus_tree { focus { … } }` → node+edge GRAPH (base x/y,
//!                  relative_position_id, offsets, prerequisite/mutually_exclusive).
//!   3. [`eval`]  — the deliberately-small OFFSET-TRIGGER EVALUATOR (supported
//!                  subset; UNSUPPORTED → badge + fall back, never fabricate).
//!   4. [`layout`]— resolve relative chains + apply offsets → ABSOLUTE pixels.
//!   5. [`bind`]  — bind node STATE from the snapshot `focus` domain.
//!
//! The rendered output (node prototype + link lines into the `nationalfocusview`
//! drag_scroll canvas) lives in the demo bin + the `app` shell, reusing
//! `gui_runtime` + `render`.

pub mod bind;
pub mod eval;
pub mod graph;
pub mod layout;
pub mod pdx;
pub mod view;

pub use bind::{bind_node, eval_context, FocusDomain, NodeBinding, NodeState};
pub use eval::{eval_trigger, Eval, EvalContext};
pub use graph::{FocusNode, FocusTree, Offset, PrereqGroup};
pub use layout::{place, PlacedNode, PlacedTree, PosFidelity, GRID_X, GRID_Y};
pub use view::{draw_tree, pick, tooltip, Camera, DrawAssets, FocusDrawStats};

use std::path::{Path, PathBuf};

use assets::resolver::GameResolver;

/// A catalog of every focus tree (+ shared focuses) found under
/// `common/national_focus/`, mod-resolved.
#[derive(Debug, Clone)]
pub struct FocusCatalog {
    pub trees: Vec<FocusTree>,
    pub shared: Vec<FocusNode>,
}

impl FocusCatalog {
    /// Find a tree by its `id` (e.g. `"german_focus"`), splicing in any
    /// `shared_focus` nodes the tree references so cross-file anchors resolve.
    pub fn tree(&self, id: &str) -> Option<FocusTree> {
        let base = self.trees.iter().find(|t| t.id == id)?.clone();
        // Splice shared focuses whose ids are referenced (as anchors or prereqs)
        // but not already present, so relative_position_id chains can resolve.
        let mut nodes = base.nodes.clone();
        let present: std::collections::HashSet<&str> =
            nodes.iter().map(|n| n.id.as_str()).collect();
        let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
        for n in &nodes {
            if let Some(a) = &n.relative_to {
                referenced.insert(a.clone());
            }
            for g in &n.prerequisites {
                for p in &g.any_of {
                    referenced.insert(p.clone());
                }
            }
        }
        let need: Vec<&str> = referenced
            .iter()
            .map(String::as_str)
            .filter(|r| !present.contains(*r))
            .collect();
        for sh in &self.shared {
            if need.contains(&sh.id.as_str()) {
                nodes.push(sh.clone());
            }
        }
        Some(FocusTree { id: base.id, nodes })
    }
}

/// Locate `common/national_focus` via the resolver (mod overlay merged with
/// vanilla). Files present in BOTH are taken from the mod (override-by-filename),
/// matching the engine's load model; mod-only and vanilla-only files both load.
pub fn focus_dirs(resolver: &GameResolver) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(m) = &resolver.mod_root {
        let d = m.join("common").join("national_focus");
        if d.is_dir() {
            dirs.push(d);
        }
    }
    let v = resolver.vanilla.join("common").join("national_focus");
    if v.is_dir() {
        dirs.push(v);
    }
    dirs
}

/// Read every `*.txt` under the focus dirs (mod over vanilla by filename) and
/// parse into a [`FocusCatalog`].
pub fn load_catalog(resolver: &GameResolver) -> std::io::Result<FocusCatalog> {
    use std::collections::BTreeMap;
    // filename -> full path, mod winning (insert mod first, skip if present)
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    for dir in focus_dirs(resolver) {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                files.entry(name.to_string()).or_insert(path);
            }
        }
    }
    let mut trees = Vec::new();
    let mut shared = Vec::new();
    for path in files.values() {
        let src = match read_text(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let top = pdx::parse_file(&src);
        trees.extend(graph::trees_from_entries(&top));
        shared.extend(graph::shared_focuses(&top));
    }
    Ok(FocusCatalog { trees, shared })
}

/// Read a focus file as UTF-8, lossily (the game files are UTF-8/ASCII with the
/// occasional stray byte — never panic on one).
fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_tree_splices_shared() {
        // simulate a catalog with a tree that anchors to a shared focus
        let base = FocusTree {
            id: "t".into(),
            nodes: vec![FocusNode {
                id: "A".into(),
                icon: None,
                base_x: 0,
                base_y: 1,
                relative_to: Some("SHARED1".into()),
                cost: None,
                offsets: vec![],
                prerequisites: vec![],
                mutually_exclusive: vec![],
            }],
        };
        let shared = vec![FocusNode {
            id: "SHARED1".into(),
            icon: None,
            base_x: 5,
            base_y: 5,
            relative_to: None,
            cost: None,
            offsets: vec![],
            prerequisites: vec![],
            mutually_exclusive: vec![],
        }];
        let cat = FocusCatalog {
            trees: vec![base],
            shared,
        };
        let t = cat.tree("t").unwrap();
        assert_eq!(t.nodes.len(), 2);
        let placed = place(&t, &EvalContext::default());
        // A is at SHARED1(5,5) + (0,1) = (5,6)
        let a = placed.node("A").unwrap();
        assert_eq!((a.gx, a.gy), (5, 6));
    }
}
