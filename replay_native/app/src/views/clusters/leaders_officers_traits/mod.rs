//! CLUSTER 2f — leaders_officers_traits (8 views) [N3 trait-tree].
//!
//! The country OFFICER-CORPS / LEADER / COMMANDER-TRAIT surface, bound per
//! `replay_ui/BINDING_MAP_leaders_officers_traits.md` against the REAL parsed game `.gui`
//! files (`officer_corps.json`, `unit_leader.json`, `navy_leader.json`,
//! `leader_groups.json`, `aces.json`, `army_trait_tree.json`, `navy_trait_tree.json`,
//! `trait_tree_common.json`) fetched via the `.gui`-set loader (`Stage::gui_files`, with a
//! lenient per-cluster fallback) — NOT synthetic chrome:
//!
//!   1. **officer_corps** — `countryofficercorpview` ROOT: the per-branch chief/theorist
//!      advisor grids (`*_chief_entry` / `theorist_entry` row factories) + the
//!      officer-corps SPIRIT tiles (`*_spirit_window_item_grid`) DERIVED from the
//!      `politics.ideas_other[]` active-ideas set + the captured-generals / doctrine-track
//!      readouts badged.
//!   2. **unit_leader** — `armyleaderwindow` ROOT + the SIX `*entry` row-factory templates
//!      (army / captured / navy / operative / division / faction): the high-command roster
//!      bound from `commanders.{generals,field_marshals,admirals}` / the OOB join.
//!   3. **navy_leader** — `navy_leader_window` ROOT: the per-fleet admiral portrait strip
//!      (`fleets_box` factory) bound from `order_of_battle.fleets[].leader`.
//!   4. **leader_groups** — `leader_groups_portrait_window` + `army_group_portrait_window`
//!      ROOTs: the map command portraits + divisions_count + the RUNTIME-cached plan bars
//!      (badged PROXY).
//!   5. **aces** — `acesview` ROOT + `acesview_entry` row factory: the flying-aces grid
//!      bound from `extra.aces[]`.
//!   6. **army_trait_tree** — `unitleader_trait_tree_window` ROOT: the selected-general
//!      HEADER (portrait / skill / 4 stats / XP) + the **N3 trait_tree**
//!      (`widgets::trait_tree`) of the selected leader's `traits[]`.
//!   7. **navy_trait_tree** — `navyleader_trait_tree_window` ROOT: the same for admirals
//!      (navy stats: maneuvering / coordination).
//!   8. **trait_tree_common** — the shared PROTOTYPE LIBRARY (`unit_leader_trait`,
//!      `picked_trait`, `pickable_trait`, `disabled_trait`, `active_trait`, `advisor_trait`,
//!      `exile_trait` + the connection-line prototypes). NO standalone window — registered
//!      as TEMPLATES only and accounted by the element-accounting test.
//!
//! THE DEMO-SNAPSHOT ARTIFACT. The `commanders` (§3.9), `extra` (§3.11), and
//! `order_of_battle` (§12) domains are SCHEMA-PRESENT but PARSER-UNFED in the save-only demo
//! snapshot — so every leader / advisor / ace / assignment field binds as
//! **FOLD-IN/RECONCILE**: it renders the not-captured glyph "—" NOW and becomes exact the
//! moment a full live snapshot folds those domains (no schema change). The officer-corps
//! SPIRIT tiles are the ONE thing derivable now (`politics.ideas_other[]` is folded). A
//! row factory over an unfed domain renders ZERO rows + a section badge — it NEVER
//! synthesizes an "empty leader" with skill 0 (Codex no-fab #3).
//!
//! THE NO-FABRICATION CONTRACT. This is a CLUSTER file, so the source-scanning guard
//! (`views::digest::no_fabricated_default_in_cluster_views`) forbids `unwrap_or(0)` /
//! `unwrap_or_default()` / the raw `ctx.snapshot()` seam. Every field is read through
//! [`BindContext::country_field`] / a top-level domain accessor → [`Captured`] and rendered
//! through [`DisplayAtom`], so a blocked / absent / null field renders "—" — NEVER a
//! fabricated 0. Trait-node OWNERSHIP/availability with no selected leader is `Unknown`,
//! never a fabricated `Available` (Codex no-fab #4).
//!
//! THE ROW MODEL. A `gridBoxType` is a row factory: the ROOT scene tree (static chrome) is
//! fixed; per-element rows are instantiated from a TEMPLATE window and CUSTOM-DRAWN. So the
//! element-accounting target is the FILE's static element set (proven ==
//! `stats.element_type_counts`); per-row data lives in the drawable digest + fidelity
//! report, not as extra ROOT nodes. Each FILE is treated as a BUNDLE MANIFEST: every
//! declared top-level root/template is classified exactly once (window-root / row-template /
//! prototype / gap) so the accounting sums across ALL roots, not one (Codex fix #2).
//!
//! THIS FILE IS YOURS, cluster agent — edit ONLY here (+ the cluster subdir). Registration
//! is distributed through `inventory`, so adding/binding a view edits NO shared file.

/// The bundle-manifest classification consumed by the element-accounting test (test-only
/// DATA — every declared `.gui` root classified exactly once). Gated `#[cfg(test)]` so the
/// non-test build carries no dead data.
#[cfg(test)]
mod accounting;
mod aces;
mod helpers;
mod leader_groups;
mod navy_leader;
mod officer_corps;
mod trait_trees;
mod unit_leader;

#[cfg(test)]
mod tests;

pub use trait_trees::{
    trait_tree_viewport_from_roots, ArmyTraitTreeView, NavyTraitTreeView, ARMY_TRAIT_TREE_VIEW_ID,
    NAVY_TRAIT_TREE_VIEW_ID,
};

// Re-export the shared helpers for the per-view submodules.
pub(crate) use helpers::*;
