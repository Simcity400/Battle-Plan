//! The VIEW MODULES — the 5 migrated "Core" surfaces + the 10 pre-created BATCH-2
//! cluster slots. Each surface OWNS its bind + draw + (where needed) its persistent
//! input state, and registers itself with the host via `inventory`. The host iterates
//! them generically; `compose.rs`/`main.rs` carry ZERO per-view branches.
//!
//! Registration is via the `inventory` crate's link-section collection: every
//! `register_view!` / `register_template!` in these modules is gathered by
//! `ViewRegistry::from_inventory()` at startup. Because the `app` library is a single
//! compilation unit linked into the binary, no "force-link" shim is needed and adding
//! a view to any module requires editing ONLY that module.

pub mod clusters;
pub mod digest;
pub mod interaction_contract;

// The 5 migrated Core surfaces (BATCH 0 deliverable: migrate existing 5 onto the
// registry; must render + scrub byte-identical).
pub mod combat;
pub mod focus;
pub mod map;
pub mod state_panel;
pub mod topbar;
