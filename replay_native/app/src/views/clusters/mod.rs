//! BATCH 0 deliverable 6 — the PRE-CREATED per-cluster module slots (2a–2j).
//!
//! Each cluster owns its OWN file here. A cluster agent adds its views ENTIRELY inside
//! its file: define the `ViewInstance` impl(s), the `open` factory fn(s), and
//! `register_view!` / `register_template!` the descriptor(s). Because registration is
//! distributed through `inventory`, NO shared file (this `mod.rs`, the registry, the
//! menubar, `compose.rs`, `main.rs`) is ever edited when adding a view — two cluster
//! agents work fully concurrently.
//!
//! The module list below is created ONCE (now) and is the only shared seam; it is
//! complete for the 10 planned clusters and never needs another edit for views.

pub mod chrome_outliner_maps;
pub mod designers;
pub mod intel_diplomacy_trade;
pub mod leaders_officers_traits;
pub mod logistics_combat_units_events;
pub mod military_management;
pub mod politics_government;
pub mod production_construction_decisions;
pub mod research_tech_doctrine;
pub mod scripted_gui_overlays;
