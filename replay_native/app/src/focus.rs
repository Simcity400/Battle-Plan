//! STAGE 3 app-shell integration of the focus-tree adapter.
//!
//! [`FocusShell`] owns the parsed focus CATALOG (loaded once from the resolver),
//! and — when the overlay is OPEN — the placed+bound tree for the observed
//! country at the current hour, plus the pan/zoom camera + hover state. It is
//! re-bound on every scrub tick (a FRESH placed tree per hour; never mutated in
//! place — the no-fabrication immutability rule). All input here is READ-ONLY
//! navigation (pan/zoom/hover); nothing mutates the snapshot or the game.

use assets::resolver::GameResolver;
use focus_tree::{
    eval_context, load_catalog, place, Camera, FocusCatalog, FocusDomain, PlacedTree,
};
use snapshot::Snapshot;

/// The focus-tree overlay's per-frame inputs. The placed tree + bound domain are
/// rebuilt by the shell on each scrub tick; the camera is the shell's persistent input
/// state. The `white` texture is a process-owned solid pixel. (Lives here now that the
/// view module — not `compose` — consumes it.)
pub struct FocusOverlay<'a> {
    pub tree: &'a focus_tree::PlacedTree,
    pub domain: &'a focus_tree::FocusDomain,
    pub camera: focus_tree::Camera,
    pub white: &'a render::Texture,
    pub tag: String,
    pub date: String,
    /// optional id under the cursor (for the hover tooltip line).
    pub hover: Option<String>,
}

/// Where the overlay's graph viewport sits (below the header, above the scrubber).
const HEADER_H: f32 = 56.0;
const FOOTER_H: f32 = 72.0; // leave room for the scrubber + tooltip line

pub struct FocusShell {
    catalog: Option<FocusCatalog>,
    /// open/closed visibility (toggled by the F key).
    pub open: bool,
    /// the placed tree for the current country+hour (None until first bind).
    placed: Option<PlacedTree>,
    domain: FocusDomain,
    camera: Option<Camera>,
    /// the country whose tree we last bound (re-bind on country change).
    bound_tag: Option<String>,
    bound_tree_id: Option<String>,
    date: String,
    hover: Option<String>,
}

impl FocusShell {
    /// Load the catalog once. A failure leaves the shell present-but-empty (the
    /// overlay just won't open) — never a panic, never fabricated data.
    pub fn load(resolver: &GameResolver) -> FocusShell {
        let catalog = match load_catalog(resolver) {
            Ok(c) => {
                eprintln!(
                    "[focus] catalog: {} trees, {} shared focuses",
                    c.trees.len(),
                    c.shared.len()
                );
                Some(c)
            }
            Err(e) => {
                eprintln!("[focus] WARNING: focus catalog not loaded ({e}); overlay disabled.");
                None
            }
        };
        FocusShell {
            catalog,
            open: false,
            placed: None,
            domain: FocusDomain::default(),
            camera: None,
            bound_tag: None,
            bound_tree_id: None,
            date: String::new(),
            hover: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn has_catalog(&self) -> bool {
        self.catalog.is_some()
    }

    /// Toggle the overlay. On OPEN, (re)bind against the current snapshot/country
    /// and fit the camera to the viewport.
    pub fn toggle(&mut self, snap: &Snapshot, tag: &str, screen: (u32, u32)) {
        self.open = !self.open;
        if self.open {
            self.rebind(snap, tag, screen, true);
        } else {
            self.hover = None;
        }
    }

    fn viewport(screen: (u32, u32)) -> [f32; 4] {
        [
            0.0,
            HEADER_H,
            screen.0 as f32,
            (screen.1 as f32 - HEADER_H - FOOTER_H).max(1.0),
        ]
    }

    /// Rebuild the placed tree + bound domain for `tag` at the snapshot's hour.
    /// Called on toggle-open and on every scrub tick while open. `refit` re-fits
    /// the camera (true on open / country change; false on a same-tree scrub so
    /// the user's pan/zoom is preserved).
    pub fn rebind(&mut self, snap: &Snapshot, tag: &str, screen: (u32, u32), refit: bool) {
        let catalog = match &self.catalog {
            Some(c) => c,
            None => return,
        };
        self.domain = snap
            .country(tag)
            .map(FocusDomain::from_country)
            .unwrap_or_default();
        self.date = snap.date.clone().unwrap_or_default();

        // pick the tree the country's focus domain names; fall back to a
        // tag-derived guess only as a last resort (never fabricates STATE — an
        // unknown tree just renders nothing).
        let tree_id = self
            .domain
            .focus_tree
            .clone()
            .or_else(|| guess_tree_id(catalog, tag));
        let tree_id = match tree_id {
            Some(id) => id,
            None => {
                eprintln!("[focus] no focus tree for {tag}; overlay empty.");
                self.placed = None;
                return;
            }
        };

        let tree = match catalog.tree(&tree_id) {
            Some(t) => t,
            None => {
                eprintln!("[focus] tree `{tree_id}` not in catalog; overlay empty.");
                self.placed = None;
                return;
            }
        };
        let ctx = eval_context(tag, &self.domain, None);
        let placed = place(&tree, &ctx);
        if placed.approximate_count > 0 {
            eprintln!(
                "[focus] {} of {} nodes have APPROXIMATE positions (unsupported offset triggers, badged).",
                placed.approximate_count, placed.nodes.len()
            );
        }

        let viewport = Self::viewport(screen);
        let changed_tree = self.bound_tree_id.as_deref() != Some(tree_id.as_str())
            || self.bound_tag.as_deref() != Some(tag);
        if refit || changed_tree || self.camera.is_none() {
            self.camera = Some(Camera::fit(&placed, viewport, 24.0));
        } else if let Some(cam) = &mut self.camera {
            cam.viewport = viewport; // keep pan/zoom, just track resize
        }
        self.placed = Some(placed);
        self.bound_tag = Some(tag.to_string());
        self.bound_tree_id = Some(tree_id);
    }

    /// Update the viewport on a window resize (keeps pan/zoom).
    pub fn resize(&mut self, screen: (u32, u32)) {
        if let Some(cam) = &mut self.camera {
            cam.viewport = Self::viewport(screen);
        }
    }

    /// Whether the cursor is inside the overlay's interactive region (so the app
    /// routes pan/zoom here instead of to the map). Only true when open.
    pub fn captures(&self, sx: f32, sy: f32) -> bool {
        if !self.open {
            return false;
        }
        match &self.camera {
            Some(c) => {
                let v = c.viewport;
                sx >= v[0] && sx <= v[0] + v[2] && sy >= v[1] && sy <= v[1] + v[3]
            }
            None => self.open, // empty tree still captures (modal)
        }
    }

    /// Pan by a screen-space delta (left-drag inside the overlay). READ-ONLY.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        if let Some(cam) = &mut self.camera {
            cam.pan(dx, dy);
        }
    }

    /// Zoom around the cursor (wheel inside the overlay). READ-ONLY.
    pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32) {
        if let Some(cam) = &mut self.camera {
            cam.zoom_at(sx, sy, factor);
        }
    }

    /// Update the hover-highlighted node from the cursor.
    pub fn hover(&mut self, sx: f32, sy: f32) {
        self.hover = match (&self.camera, &self.placed) {
            (Some(cam), Some(tree)) => focus_tree::pick(cam, tree, sx, sy),
            _ => None,
        };
    }

    /// Build the per-frame overlay descriptor for the compose pass (when open +
    /// bound). `white` is the app-owned solid texture.
    pub fn overlay<'a>(&'a self, white: &'a render::Texture) -> Option<FocusOverlay<'a>> {
        if !self.open {
            return None;
        }
        let (tree, camera) = match (&self.placed, &self.camera) {
            (Some(t), Some(c)) => (t, *c),
            _ => return None,
        };
        Some(FocusOverlay {
            tree,
            domain: &self.domain,
            camera,
            white,
            tag: self.bound_tag.clone().unwrap_or_default(),
            date: self.date.clone(),
            hover: self.hover.clone(),
        })
    }
}

/// Best-effort tree-id guess from a tag when the snapshot doesn't carry
/// `focus.focus_tree` (e.g. an early frame). Returns a catalog tree whose id
/// starts with the lowercased tag, else None. This only picks WHICH tree to
/// show, never the node STATE (which stays empty/available without a domain).
fn guess_tree_id(catalog: &FocusCatalog, tag: &str) -> Option<String> {
    let lt = tag.to_lowercase();
    catalog
        .trees
        .iter()
        .find(|t| t.id.starts_with(&lt))
        .map(|t| t.id.clone())
}
