//! Canonical product-blocker registry — the enumerated set of surfaces
//! `replay_app` refuses in the product path.
//!
//! Authority: `documents/DECISIONS.md` ("Viewing refuses unless the exact
//! recorded playback capability can be reconstructed"; declared gaps are
//! unavailable and never substitute a stale frame) and the per-law contracts
//! under `replay_interface/documents/`. This registry is the executable form of that
//! intent: `--list-blockers` exposes the enumeration, and the coverage test
//! below proves that the enumeration is complete.
//!
//! `replay_app` fails closed on any product path whose rendering/state law still
//! has FIT/UNKNOWN residuals, plus legacy opt-outs and debug knobs that must
//! never touch the final render surface. This module is the SINGLE SOURCE for
//! that list:
//!
//! - every fail-closed message is stored here as [`Blocker::reason`] and the
//!   `main.rs` gates reference it (pure indirection — no behavior change);
//! - `--list-blockers` prints [`render_list`] deterministically so audits (G3)
//!   and the definition-of-done can consume the enumeration mechanically;
//! - a coverage test asserts every `*_product_path_blocked` gate in the crate is
//!   represented here, so a new block cannot be added without a registry row.
//!
//! Two refinements keep the registry verified against the code rather than
//! parallel to it:
//!
//! - [`Scope`] distinguishes unconditional rows from debug/evidence CLI inputs
//!   that are legal in the bundle-based debug modes but rejected on any
//!   production-recording invocation (windowed or `--dump-frame`); the
//!   production-recording gates in `main.rs`, `headless.rs`, and `app.rs`
//!   return these rows' reasons verbatim.
//! - [`ENV_INPUTS`] enumerates the sanctioned (non-blocker) environment inputs,
//!   and source-scan tests prove (a) every `EnvOverride` row and every
//!   [`EnvInput`] has a live reader (no phantom rows) and (b) every literal
//!   environment read in the crate is registered.
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
    NoMapFurniture,
    // --- CLI debug/evidence inputs scoped to production recording playback ---
    RecordingBundle,
    RecordingCapture,
    RecordingDumpZoom,
    RecordingDumpCenter,
    RecordingDumpHdr,
    RecordingDayNightHour,
    // --- environment overrides ---
    DebugDumpUnderwater,
    DebugClearBeforeWater,
    DebugUnderwaterInputs,
    DebugConstantRefraction,
    SnowMudTextureOverride,
    SnowMudFadeOverride,
    SnowMudH2ReceiptDir,
    OccupationH2ReceiptDir,
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
            BlockerId::NoMapFurniture => "no_map_furniture",
            BlockerId::RecordingBundle => "recording_bundle",
            BlockerId::RecordingCapture => "recording_capture",
            BlockerId::RecordingDumpZoom => "recording_dump_zoom",
            BlockerId::RecordingDumpCenter => "recording_dump_center",
            BlockerId::RecordingDumpHdr => "recording_dump_hdr",
            BlockerId::RecordingDayNightHour => "recording_day_night_hour",
            BlockerId::DebugDumpUnderwater => "env_debug_dump_underwater",
            BlockerId::DebugClearBeforeWater => "env_debug_clear_before_water",
            BlockerId::DebugUnderwaterInputs => "env_debug_underwater_inputs",
            BlockerId::DebugConstantRefraction => "env_debug_constant_refraction",
            BlockerId::SnowMudTextureOverride => "env_snow_mud_texture_override",
            BlockerId::SnowMudFadeOverride => "env_snow_mud_fade_override",
            BlockerId::SnowMudH2ReceiptDir => "env_snow_mud_h2_receipt_dir",
            BlockerId::OccupationH2ReceiptDir => "env_occupation_h2_receipt_dir",
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

/// Where the block applies.
///
/// `Always` rows are unconditional: `parse_cli` rejects the switch and
/// `reject_env_overrides` rejects the variable on every invocation.
/// `ProductionRecording` rows are legal debug/evidence inputs for the
/// bundle-based debug modes but are rejected on any invocation that opens a
/// production recording (`--recording`, windowed or `--dump-frame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Rejected on every invocation.
    Always,
    /// Rejected whenever the invocation opens a production recording.
    ProductionRecording,
}

impl Scope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::Always => "always",
            Scope::ProductionRecording => "production-recording",
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
    /// A debug/validation knob (CLI or environment) barred from the product
    /// render path.
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
    /// Closure reference: the ledger row or law document whose evidence would
    /// earn the block off (or note that it is a design block).
    pub waits_for: &'static str,
    pub status: Status,
    /// Where the block applies ([`Scope::Always`] vs a production-recording-
    /// scoped debug/evidence input). Emitted so the `--list-blockers` audit
    /// cannot misread a conditional row as unconditional.
    pub scope: Scope,
    /// The `*_product_path_blocked` (or sibling) crate test that exercises this
    /// surface, if any. Drives the coverage gate (read from the test binary; in
    /// a bin crate that reads as dead in the non-test build).
    #[cfg_attr(not(test), allow(dead_code))]
    pub product_path_test: Option<&'static str>,
}

use BlockerId as B;
use BlockerKind::{DebugKnob, LawResidual, LegacyOptOut};
use Scope::{Always, ProductionRecording};
use Status::Blocked;
use Surface::{CliSwitch, EnvOverride};

/// The canonical, ordered blocker list. Declaration order is the emission order.
pub const REGISTRY: &[Blocker] = &[
    Blocker {
        id: B::DemoScrub,
        surface: CliSwitch("--demo-scrub"),
        kind: LawResidual,
        reason: "--demo-scrub is blocked in replay_app: synthetic scrub data is not replay/capture-derived",
        waits_for: "none: synthetic non-replay scrub state is intentionally unsupported in product paths",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_demo_scrub_product_path_blocked"),
    },
    Blocker {
        id: B::GbCpuFlag,
        surface: CliSwitch("--gb-cpu"),
        kind: LegacyOptOut,
        reason: "--gb-cpu is blocked: authenticated XMAP05 political and GradientBorder resources are unconditional and cannot be replaced by the legacy CPU diagnostic surface",
        waits_for: "none: the XMAP05 product handoff is promoted; this legacy replacement switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::SynthesizeTerrainChunks,
        surface: CliSwitch("--synthesize-terrain-chunks"),
        kind: LegacyOptOut,
        reason: "--synthesize-terrain-chunks is blocked: authenticated XMAP04 terrain/map-copy execution is unconditional and cannot be replaced by the legacy synthesis route",
        waits_for: "none: the XMAP04 product pass is promoted; this legacy replacement switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_synthesize_terrain_chunks_product_path_blocked"),
    },
    Blocker {
        id: B::ChunksFromFiles,
        surface: CliSwitch("--chunks-from-files"),
        kind: LegacyOptOut,
        reason: "--chunks-from-files is blocked: authenticated XMAP04 terrain/map-copy execution is unconditional and cannot be replaced by a files-only legacy route",
        waits_for: "none: the XMAP04 product pass is promoted; this legacy replacement switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_chunks_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoWaterFull,
        surface: CliSwitch("--no-water-full"),
        kind: LegacyOptOut,
        reason: "--no-water-full is blocked: authenticated XMAP07 water execution is unconditional and has no legacy opt-out",
        waits_for: "none: the XMAP07 product pass is promoted; this legacy opt-out remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::BordersFromFiles,
        surface: CliSwitch("--borders-from-files"),
        kind: LegacyOptOut,
        reason: "--borders-from-files is blocked: authenticated XMAP06 early/late thin-border passes are unconditional and cannot be replaced by a files-only legacy route",
        waits_for: "none: the XMAP06 product pass is promoted; this legacy replacement switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_borders_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoBordersFromFiles,
        surface: CliSwitch("--no-borders-from-files"),
        kind: LegacyOptOut,
        reason: "--no-borders-from-files is blocked: authenticated XMAP06 early/late thin-border passes are unconditional and have no legacy opt-out",
        waits_for: "none: the XMAP06 product pass is promoted; this legacy opt-out remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_borders_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoCaptureWater,
        surface: CliSwitch("--no-capture-water"),
        kind: LegacyOptOut,
        reason: "--no-capture-water is blocked: authenticated XMAP07 water is an unconditional product pass and capture tooling cannot omit it",
        waits_for: "none: XMAP07 reconstructs the product water pass; this capture-tool opt-out remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::NoCaptureRivers,
        surface: CliSwitch("--no-capture-rivers"),
        kind: LegacyOptOut,
        reason: "--no-capture-rivers is blocked: authenticated XMAP08 rivers/straits are an unconditional product pass and capture tooling cannot omit them",
        waits_for: "none: XMAP08 reconstructs the product rivers/straits pass; this capture-tool opt-out remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::NoCaptureTrees,
        surface: CliSwitch("--no-capture-trees"),
        kind: LegacyOptOut,
        reason: "--no-capture-trees is blocked: authenticated XMAP11 trees are an unconditional product pass and capture tooling cannot omit them",
        waits_for: "none: XMAP11 reconstructs the product tree pass; this capture-tool opt-out remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::NoPostfx,
        surface: CliSwitch("--no-postfx"),
        kind: LegacyOptOut,
        reason: "--no-postfx is blocked: raw captured render targets are not the game's final on-screen image",
        waits_for: "Design block (GRADING_LAW.md; --postfx is default-on, raw RTs are pre-tonemap)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::NoPostfxRestoreFull,
        surface: CliSwitch("--no-postfx-restore-full"),
        kind: LegacyOptOut,
        reason: "--no-postfx-restore-full is blocked: RestoreScene grading is part of the final game image",
        waits_for: "Design block (GRADING_LAW.md GRAD1; --postfx-restore-full default-on, fail-closed)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::TreeSeasonsByDate,
        surface: CliSwitch("--tree-seasons-by-date"),
        kind: LegacyOptOut,
        reason: "--tree-seasons-by-date is blocked: authenticated XMAP11 uses the captured exact-a729 zero season lanes and a date patch would mutate the product program",
        waits_for: "none: the XMAP11 product pass is promoted; this legacy override remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_tree_season_date_patch_product_path_blocked"),
    },
    Blocker {
        id: B::TreesFromFiles,
        surface: CliSwitch("--trees-from-files"),
        kind: LegacyOptOut,
        reason: "--trees-from-files is blocked: authenticated XMAP11 is an unconditional product pass derived from the replay hour, viewer camera/map copies, and admitted effective content",
        waits_for: "none: the XMAP11 product pass is promoted; this legacy replacement switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_trees_from_files_product_path_blocked"),
    },
    Blocker {
        id: B::NoTreesSecondSubdraw,
        surface: CliSwitch("--no-trees-second-subdraw"),
        kind: LegacyOptOut,
        reason: "--no-trees-second-subdraw is removed: the alleged second tree draw is the mutually exclusive LOW_END_GFX tree shader, and captured product sessions do not bind it",
        waits_for: "Design block (RE-refuted; the alleged second draw is the mutually exclusive LOW_END_GFX shader)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::BuildingsFromFiles,
        surface: CliSwitch("--buildings-from-files"),
        kind: LegacyOptOut,
        reason: "--buildings-from-files is blocked: authenticated XMAP12 cities/buildings/map-objects are unconditional and cannot be replaced by a files-only legacy route",
        waits_for: "none: the XMAP12 product pass is promoted; this legacy replacement switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_building_preview_product_path_blocked"),
    },
    Blocker {
        id: B::BuildingsPreviewAll,
        surface: CliSwitch("--buildings-preview-all"),
        kind: LegacyOptOut,
        reason: "--buildings-preview-all is blocked: authenticated XMAP12 is unconditional and preview-all would fabricate product membership",
        waits_for: "none: the XMAP12 product pass is promoted; this legacy preview switch remains intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_building_preview_product_path_blocked"),
    },
    Blocker {
        id: B::FreezeClock,
        surface: CliSwitch("--freeze-clock"),
        kind: LawResidual,
        reason: "--freeze-clock is blocked in replay_app: synthetic wall-clock map animation would overwrite captured engine constant-buffer time bytes",
        waits_for: "none: XMAP10 SceneCB0 timing is authoritative; synthetic clock overrides are intentionally unsupported",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_freeze_clock_product_path_blocked"),
    },
    Blocker {
        id: B::MapLabels,
        surface: CliSwitch("--map-labels"),
        kind: LegacyOptOut,
        reason: "--map-labels is blocked: country labels are an authenticated operation in the selected production static program, not an optional reconstructed layer; B10 controller placement and the reflected mapname CB1 semantic are integrated for exact admission/evidence",
        waits_for: "Design block (normal D80 playback executes authenticated mapname operations automatically; missing carriers are receipted as missing and legacy unstamped programs are receipted as omitted/data-blocked)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_map_labels_product_path_blocked"),
    },
    Blocker {
        id: B::NoMapLabels,
        surface: CliSwitch("--no-map-labels"),
        kind: LegacyOptOut,
        reason: "--no-map-labels is blocked: omitting authenticated mapname operations would mutate the selected production render program",
        waits_for: "Design block (production render operation order is immutable)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_map_labels_product_path_blocked"),
    },
    Blocker {
        id: B::MapLabelsFallback,
        surface: CliSwitch("--map-labels-fallback"),
        kind: LegacyOptOut,
        reason: "--map-labels-fallback is blocked: legacy country-label reconstruction is not engine-derived",
        waits_for: "Design block (legacy label reconstruction is an approximation, not engine-derived)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_map_labels_product_path_blocked"),
    },
    Blocker {
        id: B::NoMapFurniture,
        surface: CliSwitch("--no-map-furniture"),
        kind: LegacyOptOut,
        reason: "--no-map-furniture is blocked: XMAP21 map furniture is an unconditional production layer and cannot be silently omitted",
        waits_for: "Design block (mandatory production layer; no-op legacy opt-out rejected)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("cli_map_furniture_opt_out_product_path_blocked"),
    },
    Blocker {
        id: B::RecordingBundle,
        surface: CliSwitch("--bundle"),
        kind: DebugKnob,
        reason: "--bundle is blocked in production recording playback: the debug/evidence map bundle must never accompany or substitute for the authenticated production render (windowed or --dump-frame)",
        waits_for: "Design block (debug/evidence input; production recording playback renders only the authenticated carried program)",
        status: Blocked,
        scope: ProductionRecording,
        product_path_test: Some("cli_recording_debug_surfaces_product_path_blocked"),
    },
    Blocker {
        id: B::RecordingCapture,
        surface: CliSwitch("--capture"),
        kind: DebugKnob,
        reason: "--capture is blocked in production recording playback: the captured-session debug renderer and proxy overlay are not authenticated production surfaces",
        waits_for: "Design block (debug/evidence input; the proxy overlay must never draw over the authenticated production render)",
        status: Blocked,
        scope: ProductionRecording,
        product_path_test: Some("cli_recording_debug_surfaces_product_path_blocked"),
    },
    Blocker {
        id: B::RecordingDumpZoom,
        surface: CliSwitch("--dump-zoom"),
        kind: DebugKnob,
        reason: "--dump-zoom is blocked in production recording playback: the production dump renders the exact recorded camera, never a debug camera override",
        waits_for: "Design block (debug camera input; the recorded C03 camera is the only production camera)",
        status: Blocked,
        scope: ProductionRecording,
        product_path_test: Some("cli_recording_debug_surfaces_product_path_blocked"),
    },
    Blocker {
        id: B::RecordingDumpCenter,
        surface: CliSwitch("--dump-center"),
        kind: DebugKnob,
        reason: "--dump-center is blocked in production recording playback: the production dump renders the exact recorded camera, never a debug camera override",
        waits_for: "Design block (debug camera input; the recorded C03 camera is the only production camera)",
        status: Blocked,
        scope: ProductionRecording,
        product_path_test: Some("cli_recording_debug_surfaces_product_path_blocked"),
    },
    Blocker {
        id: B::RecordingDumpHdr,
        surface: CliSwitch("--dump-hdr"),
        kind: DebugKnob,
        reason: "--dump-hdr is blocked in production recording playback: the raw HDR target is debug evidence, not the product frame",
        waits_for: "Design block (debug evidence output; the product frame is the graded final image only)",
        status: Blocked,
        scope: ProductionRecording,
        product_path_test: Some("cli_recording_debug_surfaces_product_path_blocked"),
    },
    Blocker {
        id: B::RecordingDayNightHour,
        surface: CliSwitch("--hour"),
        kind: DebugKnob,
        reason: "--hour is blocked in production recording playback: day/night state is replay-derived, never a CLI override",
        waits_for: "Design block (debug day/night input; the recording's own time bytes drive day/night)",
        status: Blocked,
        scope: ProductionRecording,
        product_path_test: Some("cli_recording_debug_surfaces_product_path_blocked"),
    },
    Blocker {
        id: B::DebugDumpUnderwater,
        surface: EnvOverride("REPLAY_DEBUG_DUMP_UNDERWATER"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_DUMP_UNDERWATER is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugClearBeforeWater,
        surface: EnvOverride("REPLAY_DEBUG_CLEAR_BEFORE_WATER"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_CLEAR_BEFORE_WATER is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugUnderwaterInputs,
        surface: EnvOverride("REPLAY_DEBUG_UNDERWATER_INPUTS"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_UNDERWATER_INPUTS is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::DebugConstantRefraction,
        surface: EnvOverride("REPLAY_DEBUG_CONSTANT_REFRACTION"),
        kind: DebugKnob,
        reason: "REPLAY_DEBUG_CONSTANT_REFRACTION is blocked in replay_app: debug and validation environment knobs must not mutate or side-effect the final product render path",
        waits_for: "Design block (debug/validation knob barred from the product render path)",
        status: Blocked,
        scope: Always,
        product_path_test: None,
    },
    Blocker {
        id: B::SnowMudTextureOverride,
        surface: EnvOverride("REPLAY_SNOW_MUD_OVERRIDE"),
        kind: DebugKnob,
        reason: "REPLAY_SNOW_MUD_OVERRIDE is blocked in replay_app: SnowMudTexture must come from the measured replay carrier; an environment-selected texture is not engine-derived product input",
        waits_for: "Design block (SnowMudTexture is capture/replay data, never an environment override)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("snow_mud_env_overrides_are_product_path_blocked"),
    },
    Blocker {
        id: B::SnowMudFadeOverride,
        surface: EnvOverride("REPLAY_SNOW_MUD_FADE"),
        kind: DebugKnob,
        reason: "REPLAY_SNOW_MUD_FADE is blocked in replay_app: SnowMudFade must come from the measured replay carrier; an environment-selected fade is not engine-derived product input",
        waits_for: "Design block (SnowMudFade is capture/replay data, never an environment override)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("snow_mud_env_overrides_are_product_path_blocked"),
    },
    Blocker {
        id: B::SnowMudH2ReceiptDir,
        surface: EnvOverride("REPLAY_SNOW_MUD_H2_RECEIPT_DIR"),
        kind: DebugKnob,
        reason: "REPLAY_SNOW_MUD_H2_RECEIPT_DIR is blocked in replay_app: the snow/mud H2 receipt sink can fail a good product frame (empty value, receipt/readback disagreement, or a create-new collision with different bytes), so it is an evidence knob and never part of a product render",
        waits_for: "Design block (evidence sink; a product frame must never depend on an environment-selected output directory)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("h2_receipt_dir_env_overrides_are_product_path_blocked"),
    },
    Blocker {
        id: B::OccupationH2ReceiptDir,
        surface: EnvOverride("REPLAY_OCCUPATION_H2_RECEIPT_DIR"),
        kind: DebugKnob,
        reason: "REPLAY_OCCUPATION_H2_RECEIPT_DIR is blocked in replay_app: the occupation-secondary H2 receipt sink can fail a good product frame (empty value, receipt/readback disagreement, or a create-new collision with different bytes), so it is an evidence knob and never part of a product render",
        waits_for: "Design block (evidence sink; a product frame must never depend on an environment-selected output directory)",
        status: Blocked,
        scope: Always,
        product_path_test: Some("h2_receipt_dir_env_overrides_are_product_path_blocked"),
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
/// Test-only since the `main.rs` env gate became registry-iteration-driven
/// (`reject_env_overrides`); kept as the by-name lookup seam with its own test.
#[cfg_attr(not(test), allow(dead_code))]
pub fn env_reason_by_name(name: &str) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|b| matches!(b.surface, Surface::EnvOverride(n) if n == name))
        .map(|b| b.reason)
}

/// Sanctioned machine-configuration environment input — NOT a blocker. These
/// variables select machine-local configuration (profile path, game install)
/// and are documented in the usage text. They are enumerated here so the
/// registry is the complete accounting of every environment variable the
/// product binary reads: a variable absent from both this table and the
/// `EnvOverride` rows must not be read by non-test `replay_app` code (the
/// `every_crate_env_read_is_registered` scan enforces that against the source,
/// and `no_env_surface_row_is_a_phantom` proves every listed name has a live
/// reader — the registry is verified against the code, never parallel to it).
pub struct EnvInput {
    pub name: &'static str,
    /// What the variable selects and where it is read.
    pub role: &'static str,
}

pub const ENV_INPUTS: &[EnvInput] = &[
    EnvInput {
        name: "battle-plan_CONFIG",
        role: "selects the playback-profile path when --config is absent (install_paths::default_config_path)",
    },
    EnvInput {
        name: "BATTLE_PLAN_VANILLA",
        role: "fallback the game install directory when --game is absent (main::resolve_game_dir; playback, benchmark, and --validate-playback routes)",
    },
    EnvInput {
        name: "XDG_CONFIG_HOME",
        role: "non-Windows configuration-root fallback for the playback profile (install_paths::default_config_path)",
    },
    EnvInput {
        name: "HOME",
        role: "non-Windows configuration-root fallback for the playback profile (install_paths::default_config_path)",
    },
];

/// Environment variables read only by `#[cfg(test)]` code in this crate. They
/// are listed so the env-read completeness scan is two-way: every literal env
/// read found in `src/` must be registered as an [`EnvInput`], an
/// `EnvOverride` row, or listed here — and every name listed here must still
/// occur in the source, so a stale entry fails the scan instead of drifting.
#[cfg_attr(not(test), allow(dead_code))]
/// `BATTLE_PLAN_GAME_DIR`, `BATTLE_PLAN_GAME_ROOT`, `BATTLE_PLAN_REQUIRE_CAPTURE_GATES`, and
/// `BATTLE_PLAN_CAPTURES_ROOT` have no literal reader in replay-app source. The last
/// name belongs to a map-viewer evidence gate and must not pose as a live
/// replay-app test reader. `XMAP_NATIVE_EVIDENCE_REPLAY` is read only by the
/// gated native evidence test in `app.rs`.
pub const TEST_ONLY_ENV_READS: &[&str] = &["XMAP_NATIVE_EVIDENCE_REPLAY"];

/// Deterministic, test-consumable emission of the whole registry: one blocker
/// row per line as tab-separated fields
/// `slug<TAB>kind<TAB>surface-kind<TAB>surface<TAB>status<TAB>scope<TAB>waits-for`,
/// followed by the sanctioned environment-input section, one line per input as
/// `name<TAB>env-input<TAB>SANCTIONED<TAB>role`.
pub fn render_list() -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "replay_app product blockers: {}\n",
        REGISTRY.len()
    ));
    for b in REGISTRY {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            b.id.slug(),
            b.kind.as_str(),
            b.surface.kind_str(),
            b.surface.label(),
            b.status.as_str(),
            b.scope.as_str(),
            b.waits_for,
        ));
    }
    out.push_str(&format!(
        "replay_app sanctioned environment inputs: {}\n",
        ENV_INPUTS.len()
    ));
    for input in ENV_INPUTS {
        out.push_str(&format!(
            "{}\tenv-input\tSANCTIONED\t{}\n",
            input.name, input.role,
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
        // Guardrail: the canonical count is pinned. Bumping it is intentional
        // and forces `documents/DECISIONS.md` plus the `--list-blockers`
        // snapshot below to be updated alongside.
        assert_eq!(REGISTRY.len(), 36, "total blocker surfaces");
        let cli = REGISTRY
            .iter()
            .filter(|b| matches!(b.surface, Surface::CliSwitch(_)))
            .count();
        let env = REGISTRY
            .iter()
            .filter(|b| matches!(b.surface, Surface::EnvOverride(_)))
            .count();
        assert_eq!(cli, 28, "blocked CLI switches");
        assert_eq!(env, 8, "rejected env overrides");
        // Production-recording-scoped rows are exactly the six CLI
        // debug/evidence inputs; every env row is unconditional.
        let scoped: Vec<&Blocker> = REGISTRY
            .iter()
            .filter(|b| b.scope == Scope::ProductionRecording)
            .collect();
        assert_eq!(scoped.len(), 6, "production-recording-scoped rows");
        for b in &scoped {
            assert!(
                matches!(b.surface, Surface::CliSwitch(_)),
                "scoped row {:?} must be a CLI switch",
                b.id
            );
        }
        assert_eq!(ENV_INPUTS.len(), 4, "sanctioned environment inputs");
    }

    #[test]
    fn promoted_xmap06_xmap11_xmap12_legacy_switches_are_not_law_residuals() {
        for id in [
            BlockerId::BordersFromFiles,
            BlockerId::NoBordersFromFiles,
            BlockerId::TreesFromFiles,
            BlockerId::BuildingsFromFiles,
            BlockerId::BuildingsPreviewAll,
        ] {
            let row = get(id);
            assert_eq!(row.kind, BlockerKind::LegacyOptOut, "{id:?}");
            assert!(row.waits_for.starts_with("none:"), "{id:?}: {}", row.waits_for);
            assert!(row.reason.contains("authenticated XMAP"), "{id:?}: {}", row.reason);
        }
    }

    #[test]
    fn env_inputs_are_unique_and_disjoint_from_registry_surfaces() {
        let mut names = BTreeSet::new();
        for input in ENV_INPUTS {
            assert!(
                names.insert(input.name),
                "duplicate env input {}",
                input.name
            );
            assert!(
                env_reason_by_name(input.name).is_none(),
                "{} is both a sanctioned input and a rejected override",
                input.name
            );
        }
        for test_only in TEST_ONLY_ENV_READS {
            assert!(
                !names.contains(test_only) && env_reason_by_name(test_only).is_none(),
                "{test_only} is listed test-only but is also registered"
            );
        }
    }

    /// Collect every literal environment read — a `var_os("NAME")` /
    /// `var("NAME")` call — under the given source tree.
    ///
    /// Soundness measures: `//` line comments are
    /// stripped before matching, so a commented mention can no longer pose as
    /// a live reader; and the call patterns are matched WITHOUT requiring an
    /// `env::` path prefix (a `use std::env::var_os;` import call site is
    /// caught), while a preceding identifier character or `.` disqualifies
    /// the match (`some_var(` and method calls like `x.var(` are not
    /// counted).
    ///
    /// Known residual limitations, accepted and pinned here: `/* */` block
    /// comments and string literals containing the call pattern are not
    /// recognized as non-code, and a line whose string content contains `//`
    /// before the call is truncated early. None of these shapes occur in the
    /// scanned trees today; if one appears, the consuming assertions fail
    /// loudly (unregistered-name or stale-entry), never silently. The dynamic
    /// registry-driven read in `reject_env_overrides` has no literal name and
    /// is deliberately outside this scan.
    fn literal_env_reads_in_source(src: &str, out: &mut BTreeSet<String>) {
        for line in src.lines() {
            // Strip `//` line comments (also drops `///` doc lines).
            let code = match line.find("//") {
                Some(cut) => &line[..cut],
                None => line,
            };
            for pattern in ["var_os(\"", "var(\""] {
                for (idx, _) in code.match_indices(pattern) {
                    if idx > 0 {
                        let prev = code.as_bytes()[idx - 1];
                        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' {
                            continue;
                        }
                    }
                    let rest = &code[idx + pattern.len()..];
                    let Some(name) = rest.split('"').next() else {
                        continue;
                    };
                    if !name.is_empty() {
                        out.insert(name.to_string());
                    }
                }
            }
        }
    }

    fn literal_env_reads(root: &std::path::Path, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(root)
            .unwrap_or_else(|error| panic!("read env-scan tree {}: {error}", root.display()))
        {
            let path = entry.expect("env-scan directory entry").path();
            if path.is_dir() {
                literal_env_reads(&path, out);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                let src = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                    panic!("read env-scan source {}: {error}", path.display())
                });
                literal_env_reads_in_source(&src, &mut *out);
            }
        }
    }

    /// The collector's parsing rules themselves, proven on synthetic source:
    /// commented mentions are not readers, import-site bare calls ARE readers,
    /// and unrelated identifiers/method calls are not counted. This is what
    /// makes `no_env_surface_row_is_a_phantom` and
    /// `every_crate_env_read_is_registered` sound rather than pattern-lucky.
    #[test]
    fn env_read_collector_rules_are_sound() {
        // Written with escaped quotes so this test's own source text never
        // contains the bare call pattern — the real file scan of blockers.rs
        // must not pick these synthetic names up as crate reads.
        let src = "use std::env::var_os;\n\
            // env::var_os(\"COMMENTED_PHANTOM\") must not count\n\
            /// doc: env::var(\"DOC_PHANTOM\") must not count\n\
            let a = std::env::var_os(\"QUALIFIED_READ\");\n\
            let b = env::var(\"SEMI_QUALIFIED_READ\").ok();\n\
            let c = var_os(\"IMPORTED_READ\"); // env::var(\"TRAILING_PHANTOM\")\n\
            let d = some_var(\"NOT_ENV\");\n\
            let e = thing.var(\"METHOD_NOT_ENV\");\n\
            let f = my_var_os(\"IDENT_NOT_ENV\");\n";
        let mut reads = BTreeSet::new();
        literal_env_reads_in_source(src, &mut reads);
        let expected: BTreeSet<String> = ["QUALIFIED_READ", "SEMI_QUALIFIED_READ", "IMPORTED_READ"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(reads, expected);
    }

    /// No registered environment surface may lack a live reader.
    /// Every `EnvOverride` row's variable and every sanctioned [`EnvInput`]
    /// must be read somewhere in `replay_app` or `map_viewer` source (the
    /// debug-underwater knobs live at the `map_viewer` render seams). This is
    /// the regression gate for phantom rows such as `REPLAY_APP_GB_CPU` and
    /// `REPLAY_DUMP_LABELS`.
    #[test]
    fn no_env_surface_row_is_a_phantom() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut reads = BTreeSet::new();
        literal_env_reads(&manifest.join("src"), &mut reads);
        literal_env_reads(&manifest.join("../map_viewer/src"), &mut reads);
        for b in REGISTRY {
            if let Surface::EnvOverride(name) = b.surface {
                assert!(
                    reads.contains(name),
                    "EnvOverride row {name} has no live reader in replay_app or map_viewer source (phantom row)"
                );
            }
        }
        for input in ENV_INPUTS {
            assert!(
                reads.contains(input.name),
                "EnvInput {} has no live reader in replay_app or map_viewer source (phantom row)",
                input.name
            );
        }
    }

    /// The registry is verified against the code, not parallel to
    /// it. Every literal environment read in `replay_app`'s own source must be
    /// registered — as a sanctioned [`EnvInput`], as a rejected `EnvOverride`
    /// row, or in [`TEST_ONLY_ENV_READS`] — and every test-only entry must
    /// still occur in the source (two-way, so entries cannot go stale).
    #[test]
    fn every_crate_env_read_is_registered() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut reads = BTreeSet::new();
        literal_env_reads(&manifest.join("src"), &mut reads);
        let inputs: BTreeSet<&str> = ENV_INPUTS.iter().map(|input| input.name).collect();
        for name in &reads {
            assert!(
                inputs.contains(name.as_str())
                    || env_reason_by_name(name).is_some()
                    || TEST_ONLY_ENV_READS.contains(&name.as_str()),
                "environment read {name} is not registered (EnvInput / EnvOverride row / TEST_ONLY_ENV_READS)"
            );
        }
        for test_only in TEST_ONLY_ENV_READS {
            assert!(
                reads.contains(*test_only),
                "TEST_ONLY_ENV_READS entry {test_only} no longer occurs in src/ (stale)"
            );
        }
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
        // Snapshot: blocker header + one line per row, then the env-input
        // header + one line per sanctioned input, no more.
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), REGISTRY.len() + ENV_INPUTS.len() + 2);
        assert_eq!(lines[0], "replay_app product blockers: 36");
        // Spot-check exact rows (surface tokens + status + scope) as a golden
        // snapshot.
        assert_eq!(
            lines[1],
            "demo_scrub\tlaw-residual\tcli-switch\t--demo-scrub\tBLOCKED\talways\tnone: synthetic non-replay scrub state is intentionally unsupported in product paths"
        );
        assert_eq!(
            lines[23],
            "recording_bundle\tdebug-knob\tcli-switch\t--bundle\tBLOCKED\tproduction-recording\tDesign block (debug/evidence input; production recording playback renders only the authenticated carried program)"
        );
        assert_eq!(
            lines[29],
            "env_debug_dump_underwater\tdebug-knob\tenv-override\tREPLAY_DEBUG_DUMP_UNDERWATER\tBLOCKED\talways\tDesign block (debug/validation knob barred from the product render path)"
        );
        assert_eq!(
            lines[REGISTRY.len()],
            "env_occupation_h2_receipt_dir\tdebug-knob\tenv-override\tREPLAY_OCCUPATION_H2_RECEIPT_DIR\tBLOCKED\talways\tDesign block (evidence sink; a product frame must never depend on an environment-selected output directory)"
        );
        assert_eq!(
            lines[REGISTRY.len() + 1],
            "replay_app sanctioned environment inputs: 4"
        );
        assert_eq!(
            lines[REGISTRY.len() + 2],
            "battle-plan_CONFIG\tenv-input\tSANCTIONED\tselects the playback-profile path when --config is absent (install_paths::default_config_path)"
        );
        assert_eq!(
            *lines.last().unwrap(),
            "HOME\tenv-input\tSANCTIONED\tnon-Windows configuration-root fallback for the playback profile (install_paths::default_config_path)"
        );
        // Every surface has its row line (status column follows the surface).
        for b in REGISTRY {
            let needle = format!("\t{}\tBLOCKED\t{}\t", b.surface.label(), b.scope.as_str());
            assert!(
                list.contains(&needle),
                "missing row for {}",
                b.surface.label()
            );
        }
    }
}
