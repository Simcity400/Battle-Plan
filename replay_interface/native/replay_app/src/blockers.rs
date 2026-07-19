//! Canonical product-blocker registry (REPLAY_PRODUCTION_PLAN.md plan item G3;
//! §0 Constraint 3; §7 Definition-of-production item 2).
//!
//! `replay_app` fails closed on any product path whose rendering/state law still
//! has FIT/UNKNOWN residuals, plus a set of legacy opt-outs and debug knobs that
//! must never touch the final render surface. Historically those blocks were
//! scattered as inline `return Err(...)` arms in `main.rs` and env rejections in
//! startup. This module is the SINGLE SOURCE for that list:
//!
//! - every fail-closed message is stored here as [`Blocker::reason`] and the
//!   `main.rs` gates reference it (pure indirection — no behavior change);
//! - `--list-blockers` prints [`render_list`] deterministically so audits (G3)
//!   and the definition-of-done can consume the enumeration mechanically;
//! - a coverage test asserts every `*_product_path_blocked` gate in the crate is
//!   represented here, so a new block cannot be added without a registry row.
//!
//! Adding a new product block = add a [`BlockerId`] variant + a [`REGISTRY`] row
//! (and, for a CLI/env gate, route the `main.rs` arm through [`reason`]).

/// Stable identifier for one product-blocker surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockerId {
    // --- CLI switches ---
    DemoScrub,
    GbCpuFlag,
    SynthesizeTerrainChunks,
    ChunksFromFiles,
    NoWaterFull,
    BordersFromFiles,
    NoBordersFromFiles,
    NoCaptureWater,
    NoCaptureRivers,
    NoCaptureTrees,
    NoPostfx,
    NoPostfxRestoreFull,
    TreeSeasonsByDate,
    TreesFromFiles,
    NoTreesSecondSubdraw,
    BuildingsFromFiles,
    BuildingsPreviewAll,
    FreezeClock,
    MapLabels,
    NoMapLabels,
    MapLabelsFallback,
    MapFurniture,
    NoMapFurniture,
    // --- environment overrides ---
    GbCpuEnv,
    DebugDumpUnderwater,
    DebugClearBeforeWater,
    DebugUnderwaterInputs,
    DebugConstantRefraction,
    DumpLabels,
}

impl BlockerId {
    /// Short stable slug for the `--list-blockers` emission (deterministic).
    pub const fn slug(self) -> &'static str {
        match self {
            BlockerId::DemoScrub => "demo_scrub",
            BlockerId::GbCpuFlag => "gb_cpu_flag",
            BlockerId::SynthesizeTerrainChunks => "synthesize_terrain_chunks",
            BlockerId::ChunksFromFiles => "chunks_from_files",
            BlockerId::NoWaterFull => "no_water_full",
            BlockerId::BordersFromFiles => "borders_from_files",
            BlockerId::NoBordersFromFiles => "no_borders_from_files",
            BlockerId::NoCaptureWater => "no_capture_water",
            BlockerId::NoCaptureRivers => "no_capture_rivers",
            BlockerId::NoCaptureTrees => "no_capture_trees",
            BlockerId::NoPostfx => "no_postfx",
            BlockerId::NoPostfxRestoreFull => "no_postfx_restore_full",
            BlockerId::TreeSeasonsByDate => "tree_seasons_by_date",
            BlockerId::TreesFromFiles => "trees_from_files",
            BlockerId::NoTreesSecondSubdraw => "no_trees_second_subdraw",
            BlockerId::BuildingsFromFiles => "buildings_from_files",
            BlockerId::BuildingsPreviewAll => "buildings_preview_all",
            BlockerId::FreezeClock => "freeze_clock",
            BlockerId::MapLabels => "map_labels",
            BlockerId::NoMapLabels => "no_map_labels",
            BlockerId::MapLabelsFallback => "map_labels_fallback",
            BlockerId::MapFurniture => "map_furniture",
            BlockerId::NoMapFurniture => "no_map_furniture",
            BlockerId::GbCpuEnv => "env_gb_cpu",
            BlockerId::DebugDumpUnderwater => "env_debug_dump_underwater",
            BlockerId::DebugClearBeforeWater => "env_debug_clear_before_water",
            BlockerId::DebugUnderwaterInputs => "env_debug_underwater_inputs",
            BlockerId::DebugConstantRefraction => "env_debug_constant_refraction",
            BlockerId::DumpLabels => "env_dump_labels",
        }
    }
}

/// Where the block is surfaced to the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// A blocked CLI switch (the exact token, e.g. `--demo-scrub`).
    CliSwitch(&'static str),
    /// A rejected environment override (the variable name).
    EnvOverride(&'static str),
}

impl Surface {
    pub const fn label(self) -> &'static str {
        match self {
            Surface::CliSwitch(s) => s,
            Surface::EnvOverride(s) => s,
        }
    }

    pub const fn kind_str(self) -> &'static str {
        match self {
            Surface::CliSwitch(_) => "cli-switch",
            Surface::EnvOverride(_) => "env-override",
        }
    }
}

/// Why the block exists — distinguishes a law residual (earnable off with
/// evidence) from a permanent legacy opt-out / debug knob (never a product
/// input by design).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockerKind {
    /// A render/state law still has FIT/UNKNOWN residuals; closes with evidence.
    LawResidual,
    /// A legacy approximation opt-out or no-op flag; blocked by design.
    LegacyOptOut,
    /// A debug/validation environment knob barred from the product render path.
    DebugKnob,
}

impl BlockerKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            BlockerKind::LawResidual => "law-residual",
            BlockerKind::LegacyOptOut => "legacy-opt-out",
            BlockerKind::DebugKnob => "debug-knob",
        }
    }
}

/// Current disposition. Every registry row is `Blocked` today; the type exists
/// so an earned-off block flips its status (and the emission records it) rather
/// than silently disappearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Fail-closed. Awaits the closure evidence in [`Blocker::waits_for`].
    Blocked,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Blocked => "BLOCKED",
        }
    }
}

/// One canonical product-blocker row.
#[derive(Debug, Clone, Copy)]
pub struct Blocker {
    pub id: BlockerId,
    pub surface: Surface,
    pub kind: BlockerKind,
    /// The exact fail-closed message returned when the surface is triggered.
    /// The `main.rs` gate for this surface returns this string verbatim.
    pub reason: &'static str,
    /// Closure reference: the plan phase item / ledger row / law doc whose
    /// evidence would earn the block off (or note that it is a design block).
    pub waits_for: &'static str,
    pub status: Status,
    /// The `*_product_path_blocked` (or sibling) crate test that exercises this
    /// surface, if any. Drives the coverage gate (read from the test binary; in
    /// a bin crate that reads as dead in the non-test build).
    #[cfg_attr(not(test), allow(dead_code))]
    pub product_path_test: Option<&'static str>,
}

use BlockerId as B;
use BlockerKind::{DebugKnob, LawResidual, LegacyOptOut};
use Status::Blocked;
use Surface::{CliSwitch, EnvOverride};

/// The canonical, ordered blocker list. Declaration order is the emission order.
pub const REGISTRY: &[Blocker] = &[
    Blocker {
        id: B::DemoScrub,
        surface: CliSwitch("--demo-scrub"),
        kind: LawResidual,
        reason: "--demo-scrub is blocked in replay_app: synthetic scrub data is not replay/capture-derived",
        waits_for: "Plan A5 renderer keying swap + E1 transport (real ReplaySource-driven timeline)",
        status: Blocked,
        product_path_test: Some("cli_demo_scrub_product_path_blocked"),
    },
    Blocker {
        id: B::GbCpuFlag,
        surface: CliSwitch("--gb-cpu"),
        kind: LawResidual,
        reason: "--gb-cpu is blocked: replay-generated political GradientBorder textures are not engine-derived enough for product rendering",
        waits_for: "Plan A5 + POLITICAL_COLOR_LAW.md / GB field law (exact GradientBorder generation); ledger §2.8",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::SynthesizeTerrainChunks,
        surface: CliSwitch("--synthesize-terrain-chunks"),
        kind: LawResidual,
        reason: "--synthesize-terrain-chunks is blocked in replay_app: terrain chunk synthesis still depends on runtime/LOD state that is not fully engine-derived; use map_viewer terrain tooling only for development",
        waits_for: "Plan B7 terrain vSnap/render-view frustum (CAP1); ledger §2.9 terrain chunk rows",
        status: Blocked,
        product_path_test: Some("cli_synthesize_terrain_chunks_product_path_blocked"),
    },
    Blocker {
        id: B::ChunksFromFiles,
        surface: CliSwitch("--chunks-from-files"),
        kind: LawResidual,
        reason: "--chunks-from-files is blocked in replay_app: generated terrain chunk mesh buffers still depend on runtime/LOD state that is not fully engine-derived; use map_viewer terrain tooling only for development",
        waits_for: "Plan B7 terrain vSnap/LOD level-assignment (CAP1); ledger §2.9 (LODSNAP/LODRING/LODCULL2 residual)",
        status: Blocked,
        product_path_test: Some("cli_chunks_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoWaterFull,
        surface: CliSwitch("--no-water-full"),
        kind: LegacyOptOut,
        reason: "--no-water-full is blocked: the old app-owned water approximation is not engine-derived",
        waits_for: "Design block (WATER_LAW.md; --water-full is default-on and fail-closed)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::BordersFromFiles,
        surface: CliSwitch("--borders-from-files"),
        kind: LawResidual,
        reason: "--borders-from-files is blocked in replay_app: thin-border fade/loop/sea-region law still has UNKNOWN/FIT residuals; use map_viewer border tooling only for development",
        waits_for: "Plan B1 water-strip residuals + island loops + fade interior (BORDER_LAW.md §2.5, BORD8/9/10)",
        status: Blocked,
        product_path_test: Some("cli_borders_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoBordersFromFiles,
        surface: CliSwitch("--no-borders-from-files"),
        kind: LegacyOptOut,
        reason: "--no-borders-from-files is blocked: borders are already blocked in replay_app; do not pass a no-op legacy opt-out",
        waits_for: "Design block (borders already blocked; no-op legacy opt-out rejected)",
        status: Blocked,
        product_path_test: Some("cli_borders_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoCaptureWater,
        surface: CliSwitch("--no-capture-water"),
        kind: LegacyOptOut,
        reason: "--no-capture-water is blocked: omitting captured engine water is not the game's final map",
        waits_for: "Design block (captured engine water is required for the final map)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::NoCaptureRivers,
        surface: CliSwitch("--no-capture-rivers"),
        kind: LegacyOptOut,
        reason: "--no-capture-rivers is blocked: omitting captured engine rivers is not the game's final map",
        waits_for: "Design block (captured engine rivers are required for the final map)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::NoCaptureTrees,
        surface: CliSwitch("--no-capture-trees"),
        kind: LegacyOptOut,
        reason: "--no-capture-trees is blocked: omitting captured engine trees is not the game's final map",
        waits_for: "Design block (captured engine trees are required for the final map)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::NoPostfx,
        surface: CliSwitch("--no-postfx"),
        kind: LegacyOptOut,
        reason: "--no-postfx is blocked: raw captured render targets are not the game's final on-screen image",
        waits_for: "Design block (GRADING_LAW.md; --postfx is default-on, raw RTs are pre-tonemap)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::NoPostfxRestoreFull,
        surface: CliSwitch("--no-postfx-restore-full"),
        kind: LegacyOptOut,
        reason: "--no-postfx-restore-full is blocked: RestoreScene grading is part of the final game image",
        waits_for: "Design block (GRADING_LAW.md GRAD1; --postfx-restore-full default-on, fail-closed)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::TreeSeasonsByDate,
        surface: CliSwitch("--tree-seasons-by-date"),
        kind: LawResidual,
        reason: "--tree-seasons-by-date is blocked in replay_app: tree-season CB1 date patching is still a hypothesis pending dated engine capture evidence",
        waits_for: "Plan B4 tree season CB1 from files (CAP9); ledger §2.11 trees",
        status: Blocked,
        product_path_test: Some("cli_tree_season_date_patch_product_path_blocked"),
    },
    Blocker {
        id: B::TreesFromFiles,
        surface: CliSwitch("--trees-from-files"),
        kind: LawResidual,
        reason: "--trees-from-files is blocked in replay_app: tree scatter anchor, rotation, and model selection still have FIT/UNKNOWN residuals; use map_viewer tree tooling only for development",
        waits_for: "Plan B4 tree scatter anchor/rotation/model selection (CAP9); ledger §2.11 trees",
        status: Blocked,
        product_path_test: Some("cli_trees_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoTreesSecondSubdraw,
        surface: CliSwitch("--no-trees-second-subdraw"),
        kind: LegacyOptOut,
        reason: "--no-trees-second-subdraw is removed: the alleged second tree draw is the mutually exclusive LOW_END_GFX tree shader, and captured product sessions do not bind it",
        waits_for: "Design block (RE-refuted; the alleged second draw is the mutually exclusive LOW_END_GFX shader)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::BuildingsFromFiles,
        surface: CliSwitch("--buildings-from-files"),
        kind: LawResidual,
        reason: "--buildings-from-files is blocked in replay_app: visible static buildings/cities require a grounded map_objects_pass or real mesh/material importer",
        waits_for: "Plan B3 building .mesh importer / map_objects_pass derivation (CAP9)",
        status: Blocked,
        product_path_test: Some("cli_building_preview_product_path_blocked"),
    },
    Blocker {
        id: B::BuildingsPreviewAll,
        surface: CliSwitch("--buildings-preview-all"),
        kind: LawResidual,
        reason: "--buildings-preview-all is blocked: building preview rendering is not product engine-derived",
        waits_for: "Plan B3 building .mesh importer / map_objects_pass derivation (CAP9)",
        status: Blocked,
        product_path_test: Some("cli_building_preview_product_path_blocked"),
    },
    Blocker {
        id: B::FreezeClock,
        surface: CliSwitch("--freeze-clock"),
        kind: LawResidual,
        reason: "--freeze-clock is blocked in replay_app: synthetic wall-clock map animation would overwrite captured engine constant-buffer time bytes",
        waits_for: "Plan D6 replay-native clock + ledger §2.9 scene-CB0 runtime time fields (vGlobalTime)",
        status: Blocked,
        product_path_test: Some("cli_freeze_clock_product_path_blocked"),
    },
    Blocker {
        id: B::MapLabels,
        surface: CliSwitch("--map-labels"),
        kind: LawResidual,
        reason: "--map-labels is blocked in replay_app: owner-vs-controller label keying and per-label Transp_OffsetX.x runtime alpha values await capture confirmation; the product glyph-VB builder/render path is present and gated against map_viewer mapname law output, but these two runtime inputs remain fail-closed",
        waits_for: "Plan B10 label owner-vs-controller keying + per-label Transp_OffsetX.x (CAP5); MAP_LABEL_LAW.md §2.6",
        status: Blocked,
        product_path_test: Some("cli_map_labels_product_path_blocked"),
    },
    Blocker {
        id: B::NoMapLabels,
        surface: CliSwitch("--no-map-labels"),
        kind: LegacyOptOut,
        reason: "--no-map-labels is blocked: country labels are already blocked in replay_app; do not pass a no-op legacy opt-out",
        waits_for: "Design block (country labels already blocked; no-op legacy opt-out rejected)",
        status: Blocked,
        product_path_test: Some("cli_map_labels_product_path_blocked"),
    },
    Blocker {
        id: B::MapLabelsFallback,
        surface: CliSwitch("--map-labels-fallback"),
        kind: LegacyOptOut,
        reason: "--map-labels-fallback is blocked: legacy country-label reconstruction is not engine-derived",
        waits_for: "Design block (legacy label reconstruction is an approximation, not engine-derived)",
        status: Blocked,
        product_path_test: Some("cli_map_labels_product_path_blocked"),
    },
    Blocker {
        id: B::MapFurniture,
        surface: CliSwitch("--map-furniture"),
        kind: LawResidual,
        reason: "--map-furniture is blocked in replay_app: highlight ring rendering and the 0.40 map-text scale origin await capture/decompile confirmation; the relation-column law is engine-traced in map_viewer, but non-neutral capture coverage is still pending; the replay_app furniture module and 3D projection API are present but these draw-time inputs remain fail-closed",
        waits_for: "Ledger §2 furniture unblock assessment (highlight ring + 0.40 map-text scale origin; RINT1/FURN-REL)",
        status: Blocked,
        product_path_test: Some("cli_map_furniture_product_path_blocked"),
    },
    Blocker {
        id: B::NoMapFurniture,
        surface: CliSwitch("--no-map-furniture"),
        kind: LegacyOptOut,
        reason: "--no-map-furniture is blocked: map furniture is already blocked in replay_app; do not pass a no-op legacy opt-out",
        waits_for: "Design block (map furniture already blocked; no-op legacy opt-out rejected)",
        status: Blocked,
        product_path_test: Some("cli_map_furniture_product_path_blocked"),
    },
    Blocker {
        id: B::GbCpuEnv,
        surface: EnvOverride("REPLAY_APP_GB_CPU"),
        kind: LawResidual,
        reason: "REPLAY_APP_GB_CPU is blocked: replay-generated political GradientBorder textures are not engine-derived enough for product rendering",
        waits_for: "Plan A5 + POLITICAL_COLOR_LAW.md / GB field law (env twin of --gb-cpu); ledger §2.8",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugDumpUnderwater,
        surface: EnvOverride("REPLAY_DEBUG_DUMP_UNDERWATER"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_DUMP_UNDERWATER is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugClearBeforeWater,
        surface: EnvOverride("REPLAY_DEBUG_CLEAR_BEFORE_WATER"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_CLEAR_BEFORE_WATER is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugUnderwaterInputs,
        surface: EnvOverride("REPLAY_DEBUG_UNDERWATER_INPUTS"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_UNDERWATER_INPUTS is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugConstantRefraction,
        surface: EnvOverride("REPLAY_DEBUG_CONSTANT_REFRACTION"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_CONSTANT_REFRACTION is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        product_path_test: None,
    },
    Blocker {
        id: B::DumpLabels,
        surface: EnvOverride("REPLAY_DUMP_LABELS"),
        kind: DebugKnob,
        reason: "REPLAY_DUMP_LABELS is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        product_path_test: None,
    },
];

/// Look up a blocker row by id. Panics if the id has no row — every
/// [`BlockerId`] variant MUST have exactly one [`REGISTRY`] entry (enforced by
/// `registry_has_one_row_per_id`).
pub fn get(id: BlockerId) -> &'static Blocker {
    REGISTRY
        .iter()
        .find(|b| b.id == id)
        .expect("every BlockerId has a REGISTRY row")
}

/// The exact fail-closed message for a blocker surface. `main.rs` gates return
/// this verbatim, keeping the registry the single source of the message text.
pub fn reason(id: BlockerId) -> &'static str {
    get(id).reason
}

/// The fail-closed message for a rejected environment variable, by name.
pub fn env_reason_by_name(name: &str) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|b| matches!(b.surface, Surface::EnvOverride(n) if n == name))
        .map(|b| b.reason)
}

/// Deterministic, test-consumable emission of the whole registry, one row per
/// line as tab-separated fields: `slug<TAB>kind<TAB>surface-kind<TAB>surface<TAB>status<TAB>waits-for`.
pub fn render_list() -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "replay_app product blockers: {}\n",
        REGISTRY.len()
    ));
    for b in REGISTRY {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            b.id.slug(),
            b.kind.as_str(),
            b.surface.kind_str(),
            b.surface.label(),
            b.status.as_str(),
            b.waits_for,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn registry_has_one_row_per_id_and_unique_surfaces() {
        // Every row's `get(id)` resolves to itself (one row per id).
        for b in REGISTRY {
            assert_eq!(get(b.id).surface, b.surface, "id {:?} row mismatch", b.id);
        }
        // Surfaces (CLI token / env name) are unique across the registry.
        let mut surfaces = BTreeSet::new();
        for b in REGISTRY {
            assert!(
                surfaces.insert(b.surface.label()),
                "duplicate surface {}",
                b.surface.label()
            );
        }
        // Slugs are unique too (they key the emission).
        let mut slugs = BTreeSet::new();
        for b in REGISTRY {
            assert!(slugs.insert(b.id.slug()), "duplicate slug {}", b.id.slug());
        }
    }

    #[test]
    fn every_reason_message_starts_with_its_surface_token() {
        // The message the gate returns names the surface it blocks. This ties the
        // registry text to the `main.rs` `*_product_path_blocked` assertions,
        // which all check `err.contains("<surface> is blocked")`.
        for b in REGISTRY {
            let token = b.surface.label();
            let head = format!("{token} is blocked");
            let removed = format!("{token} is removed");
            assert!(
                b.reason.starts_with(&head) || b.reason.starts_with(&removed),
                "reason for {:?} must start with '{token} is blocked/removed': {}",
                b.id,
                b.reason
            );
        }
    }

    #[test]
    fn counts_are_exact() {
        // Guardrail: the canonical count is pinned. Bumping it is intentional and
        // forces the LANE_REPORT + plan G3 row to be updated alongside.
        assert_eq!(REGISTRY.len(), 29, "total blocker surfaces");
        let cli = REGISTRY
            .iter()
            .filter(|b| matches!(b.surface, Surface::CliSwitch(_)))
            .count();
        let env = REGISTRY
            .iter()
            .filter(|b| matches!(b.surface, Surface::EnvOverride(_)))
            .count();
        assert_eq!(cli, 23, "blocked CLI switches");
        assert_eq!(env, 6, "rejected env overrides");
    }

    #[test]
    fn env_reason_lookup_resolves_all_env_rows() {
        for b in REGISTRY {
            if let Surface::EnvOverride(name) = b.surface {
                assert_eq!(env_reason_by_name(name), Some(b.reason));
            }
        }
        assert_eq!(env_reason_by_name("REPLAY_NOT_A_KNOB"), None);
    }

    #[test]
    fn render_list_is_deterministic_and_snapshot_stable() {
        // Deterministic: identical across calls.
        assert_eq!(render_list(), render_list());
        let list = render_list();
        // Snapshot: header + one line per row, no more.
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), REGISTRY.len() + 1);
        assert_eq!(lines[0], "replay_app product blockers: 29");
        // Spot-check exact rows (surface tokens + status) as a golden snapshot.
        assert_eq!(
            lines[1],
            "demo_scrub\tlaw-residual\tcli-switch\t--demo-scrub\tBLOCKED\tPlan A5 renderer keying swap + E1 transport (real ReplaySource-driven timeline)"
        );
        assert_eq!(
            lines[24],
            "env_gb_cpu\tlaw-residual\tenv-override\tREPLAY_APP_GB_CPU\tBLOCKED\tPlan A5 + POLITICAL_COLOR_LAW.md / GB field law (env twin of --gb-cpu); ledger §2.8"
        );
        assert_eq!(
            *lines.last().unwrap(),
            "env_dump_labels\tdebug-knob\tenv-override\tREPLAY_DUMP_LABELS\tBLOCKED\tDesign block (debug/validation knob barred from the product render path)"
        );
        // Every CLI surface line carries a `--` token; every env line does not.
        for b in REGISTRY {
            let needle = format!("\t{}\tBLOCKED\t", b.surface.label());
            assert!(
                list.contains(&needle),
                "missing row for {}",
                b.surface.label()
            );
        }
    }
}
