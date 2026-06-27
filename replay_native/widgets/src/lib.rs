//! `widgets` — BATCH 1: the 8 SHARED RENDER PRIMITIVES the BATCH-2 view clusters bind
//! against. FROZEN public interfaces (documented + usage-exampled) so ~110 cluster
//! views build against a stable surface.
//!
//! ## The 8 primitives
//!   * [`node_graph`] (N1) — generic placed-node + edge-line + pan/zoom graph. HIGHEST
//!     LEVERAGE (tech/doctrine/research/power-balance/focus). Backs N3.
//!   * [`tabs`] (N2) — a tab/sub-window manager (politics/mil-mgmt/intel/find/designers).
//!   * [`trait_tree`] (N3) — a trait grid with prereq/XOR lines, a thin N1 specialization.
//!   * [`designer`] (N4) — a designer module-slot grid + per-slot stat contribution.
//!   * [`event_card`] (N5) — a portrait + wrapped title/desc/options popup card.
//!   * [`naval_combat`] (N6) — a side-roster combat panel + sea-zone bubble.
//!   * [`minimap`] (N7) — a downscaled-map viewport + camera rect.
//!   * [`map_overlay`] (N8) — a province-centroid (or world/screen) icon layer.
//!
//! ## Design contract (every primitive)
//!   * a PURE data→layout struct + a `plan_*` fn (NO GPU — headlessly testable) that
//!     emits a [`plan::Plan`], and a `draw_*` fn that EXECUTES the plan against
//!     `render::Renderer`. The plan's [`plan::Plan::digest`] is the scrub byte-identity
//!     key.
//!   * NO-FABRICATION at the boundary: visible labels/titles/stats are [`common::Atom`]
//!     (a blocked field draws `—`, never a fabricated 0); asset slots are
//!     [`common::AssetRef`] (no-asset / missing / blocked stay distinct).
//!   * empty / degenerate / non-finite / out-of-range inputs never panic (each
//!     primitive states + tests its policy).
//!   * SUBSTRATE-LIGHT: depends only on `render` / `gui_runtime` / `assets`. It does NOT
//!     depend on `focus_tree` / `combat_panel` (their placed-graph + centroid-bubble
//!     PATTERNS are re-implemented here, not imported — the additive build guard).

pub mod common;
pub mod designer;
pub mod event_card;
pub mod map_overlay;
pub mod minimap;
pub mod naval_combat;
pub mod node_graph;
pub mod plan;
pub mod tabs;
pub mod trait_tree;

pub use common::{
    Atom, AssetRef, Canvas, DrawAssets, Fidelity, Reason, CANVAS_MAX_SCALE, CANVAS_MIN_SCALE,
};
pub use plan::{Cmd, Plan};
