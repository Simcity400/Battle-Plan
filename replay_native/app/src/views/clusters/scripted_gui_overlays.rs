//! CLUSTER 2j — scripted_gui_overlays (38 views).
//!
//! The mod + vanilla in-game **scripted-GUI overlay** surface. Per
//! `replay_ui/BINDING_MAP_scripted_gui_overlays.md` §4 (the authoritative 38-overlay
//! accounting), each overlay gets exactly one verdict:
//!
//!   * **(ii) PARTIALLY BINDABLE (3):** `topbarresources`, `topbarvp`, `vp_scripted_gui`.
//!   * **(i) CHROME-ONLY (32):** `naval_scripted_gui` + 8 mod propaganda boards + 23
//!     vanilla nation-system boards. Their backing datum is a **country-scoped** script
//!     variable, which the snapshot does NOT record (SCHEMA has no per-country arbitrary
//!     variable bag — only GLOBAL `world.variables`/`world.flags`). We build the full
//!     CHROME now and BADGE every script-var-driven value slot `unavailable` ("—").
//!   * **(iii) STATIC DECORATION (1):** `SWOA_focus_background`.
//!   * **(iv) DEV-TOOL / INERT (2):** `horsttool` (mutation panel — 4 bound readouts,
//!     every button inert), `game_check` (game-rules dev/debug panel).
//!
//! ## Synthetic render plan (NOT a faithful `.gui` reconstruction)
//!
//! These 38 overlay `.gui` files are **not** in PRIORITY_GUI, so they are not pre-parsed
//! into `game_assets/gui/*.json`, and the binary's `Stage.gui_files` holds only
//! `topbar.json` + `province_state.json`. Because this cluster module may not edit any
//! shared file (`main.rs`, the registry, …), it cannot add a `.gui`-parse step at
//! startup. Instead each overlay's chrome is built as a **SYNTHETIC RENDER PLAN**:
//! `gui_runtime::scene::Node` trees constructed directly from the authoritative
//! element→variable map captured by the DEF parser in `game_defs/out/scripted_guis.json`
//! (embedded below as [`OverlaySpec`] static data, the single source of truth for
//! element accounting). This is honest by construction (Codex round, §"Codex"):
//!
//!   * Synthesizing a **label / background frame / placeholder value slot** is chrome —
//!     approximate geometry, fine.
//!   * Synthesizing a **value** is forbidden. Every value slot is either a real
//!     [`Captured`] bind (the 3 partial-bind amounts + horsttool occupation readouts) or
//!     a [`DisplayAtom::unavailable`] badge — NEVER a fabricated `0`/`""`/guessed number.
//!
//! When the live fold adds a per-country `script_variables` bag (the OPEN gap), each
//! badged slot becomes a one-line `Captured` bind via [`OverlaySlot::bound_var`] — the
//! spec already names the backing variable, so the transition is mechanical.
//!
//! THIS FILE IS YOURS, cluster agent — edit ONLY here. Registration is distributed
//! through `inventory`, so adding/binding a view edits NO shared file.

use gui_runtime::scene::{Anchor, ElementKind, Node, Rect, TextAlign, TextSpec};

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::registry::{GuiSymbolRef, MenuCluster, MenuRoute, ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, HitModel, HostPolicy, InstanceId, NodeDrawable, ViewCommand, ViewInstance, ZBand,
};
use crate::register_view;
use bind::Fidelity;

// ===========================================================================
// The overlay spec model — the embedded element→variable map (no DEF dep).
// ===========================================================================

/// The four binding-map verdicts (§0 taxonomy). Drives the per-overlay badge + how the
/// generic builder treats each value slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// (i) backing var is country-scoped / absent from the snapshot → chrome + badge.
    ChromeOnly,
    /// (ii) at least one element binds to a real snapshot path.
    PartiallyBindable,
    /// (iii) pure cosmetic decoration, no datum even in-game.
    StaticDecoration,
    /// (iv) dev/debug or a mutation panel whose buttons are inert in a read-only replay.
    DevToolInert,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::ChromeOnly => "scripted content not captured",
            Verdict::PartiallyBindable => "partially bound",
            Verdict::StaticDecoration => "static decoration",
            Verdict::DevToolInert => "dev tool — inert (read-only replay)",
        }
    }
}

/// How ONE value/dynamic-frame slot in an overlay resolves. The whole no-fabrication
/// contract is concentrated here: a slot is EITHER a real captured bind OR a badge.
#[derive(Debug, Clone, Copy)]
pub enum SlotSource {
    /// A script-var-driven slot whose backing token is NOT a country script variable —
    /// a transient country temp var, a `[Get*]`/`[GET_*]` GUI script function, a computed
    /// colour, or a `THIS.`/`@`-scoped flag lookup. None of those are recorded in the
    /// per-country `script_variables` bag, so this renders the not-captured glyph, badged
    /// `Blocked` — never a fabricated value. `var` names the backing token (from
    /// `scripted_guis.json`) for documentary/accounting purposes.
    Badged { var: &'static str },
    /// A bindable slot whose backing datum IS a genuine per-country script variable now
    /// recorded in the snapshot at `countries.<observed>.script_variables.<var>`
    /// (variables_live fold; the `var` is the DEF `frame` token, provenance-confirmed a
    /// stored variable — see the `provenance_confirms_country_var_slots` test). Resolves
    /// via the no-fabrication [`BindContext::country_field`] seam: a MEASURED value renders
    /// (with the given fidelity), an ABSENT var (the country never set it) or a snapshot
    /// without the bag stays badged "—". Never a fabricated `0`. Array vars use the `name^N`
    /// key form, which the variables_live reader emits verbatim (so it round-trips).
    CountryVar {
        var: &'static str,
        fidelity: Fidelity,
    },
    /// A bindable slot reading a dotted path under `countries.<TAG>` (Captured).
    CountryPath {
        path: &'static str,
        fidelity: Fidelity,
    },
    /// A bindable slot reading a dotted path under `states.<sel>` (Captured) — the
    /// horsttool occupation readouts. `sel` (selected state) is the open arg.
    StatePath {
        path: &'static str,
        fidelity: Fidelity,
    },
    /// A GLOBAL `world.variables[var]` score slot. The snapshot's `world` domain is
    /// schema-present (§18) but UNFED in the demo (the DEMO caveat), and the cluster's
    /// no-fabrication audit forbids the raw-snapshot seam, so this renders the badge now
    /// and lights up the moment a `world_field` accessor lands. NEVER a fabricated score.
    WorldVar { var: &'static str },
}

/// One value/icon slot inside an overlay window: the `.gui` element name + a fixed label
/// shown beside it + its [`SlotSource`].
#[derive(Debug, Clone, Copy)]
pub struct OverlaySlot {
    /// The `.gui` element `name` (from `scripted_guis.json` `elements`, or the binding
    /// map's enumerated readouts).
    pub element: &'static str,
    /// A fixed caption drawn to the left of the value (chrome — never a datum).
    pub label: &'static str,
    pub source: SlotSource,
}

impl OverlaySlot {
    const fn badged(element: &'static str, label: &'static str, var: &'static str) -> OverlaySlot {
        OverlaySlot {
            element,
            label,
            source: SlotSource::Badged { var },
        }
    }
    /// A slot bound to a genuine per-country script variable (`script_variables.<var>`).
    /// The icon-frame / paranoia-portrait / rearmament-frame readouts are FRAME indices —
    /// integer-valued script vars — so they bind at `Derived` fidelity (the EXACT in-game
    /// surface is the sprite at that frame; we surface the measured frame *index*).
    const fn country_var(
        element: &'static str,
        label: &'static str,
        var: &'static str,
    ) -> OverlaySlot {
        OverlaySlot {
            element,
            label,
            source: SlotSource::CountryVar {
                var,
                fidelity: Fidelity::Derived,
            },
        }
    }
    const fn country(
        element: &'static str,
        label: &'static str,
        path: &'static str,
        fidelity: Fidelity,
    ) -> OverlaySlot {
        OverlaySlot {
            element,
            label,
            source: SlotSource::CountryPath { path, fidelity },
        }
    }
    const fn state(
        element: &'static str,
        label: &'static str,
        path: &'static str,
        fidelity: Fidelity,
    ) -> OverlaySlot {
        OverlaySlot {
            element,
            label,
            source: SlotSource::StatePath { path, fidelity },
        }
    }
    const fn world(element: &'static str, label: &'static str, var: &'static str) -> OverlaySlot {
        OverlaySlot {
            element,
            label,
            source: SlotSource::WorldVar { var },
        }
    }
}

/// The full spec for one overlay (one registered view). `elements` is the authoritative
/// element list from `scripted_guis.json` — element accounting asserts the synthesized
/// scene carries exactly one node per element (plus fixed chrome), so nothing is dropped.
pub struct OverlaySpec {
    /// The registered view id (`overlay.<short>`).
    pub id: ViewId,
    /// The menubar label.
    pub label: &'static str,
    /// The overlay window's `.gui` ROOT name (from `scripted_guis.json` `window_name`).
    pub window: &'static str,
    /// The source `.gui` file (for the descriptor's `GuiSymbolRef`, documentary).
    pub gui_file: &'static str,
    pub verdict: Verdict,
    /// The element→value slots (every script-var-driven element + enumerated readouts).
    pub slots: &'static [OverlaySlot],
    /// Sort order within the cluster menu.
    pub order: i32,
}

// ===========================================================================
// The 38 overlay specs (BINDING_MAP §4 + scripted_guis.json element map).
// ===========================================================================
//
// Fidelity::Derived for topbarresources: the EXACT in-game number is the country
// script var `resource@<res>` (net-of-import availability); we approximate it with the
// produced/exported ledger `economy.resources.available_derived.*` (binding map §1.2).

const ALU: &str = "economy.resources.available_derived.aluminium";

/// M2 `topbarresources.gui` — (ii) PARTIALLY BINDABLE — 6 resource amounts.
const TOPBARRESOURCES: &[OverlaySlot] = &[
    OverlaySlot::country("aluminium_value", "Aluminium", ALU, Fidelity::Derived),
    OverlaySlot::country(
        "rubber_value",
        "Rubber",
        "economy.resources.available_derived.rubber",
        Fidelity::Derived,
    ),
    OverlaySlot::country(
        "tungsten_value",
        "Tungsten",
        "economy.resources.available_derived.tungsten",
        Fidelity::Derived,
    ),
    OverlaySlot::country(
        "steel_value",
        "Steel",
        "economy.resources.available_derived.steel",
        Fidelity::Derived,
    ),
    OverlaySlot::country(
        "chromium_value",
        "Chromium",
        "economy.resources.available_derived.chromium",
        Fidelity::Derived,
    ),
    OverlaySlot::country(
        "coal_value",
        "Coal",
        "economy.resources.available_derived.coal",
        Fidelity::Derived,
    ),
];

/// M3 `topbarvp.gui` — (ii) PARTIALLY BINDABLE — GLOBAL score banners.
const TOPBARVP: &[OverlaySlot] = &[
    OverlaySlot::world("vp_value_allies", "Allies score", "allies_score"),
    OverlaySlot::world("vp_value_axis", "Axis score", "axis_score"),
];

/// M4 `vp_scripted_gui.gui` — (ii) PARTIALLY BINDABLE — score + per-city control tint.
/// The two score banners bind GLOBAL `world.variables`; the ~16 per-city held-by tints
/// are `states.<id>.controller` / `map.vp_by_state` (badged here until the per-city→
/// state lookup is wired; their static label text is fixed loc, the dynamic tint derived).
const VP_EXPLAINER: &[OverlaySlot] = &[
    OverlaySlot::world("vp_overview_allies", "Allies score", "allies_score"),
    OverlaySlot::world("vp_overview_axis", "Axis score", "axis_score"),
    OverlaySlot::badged("vp_paris", "Paris", "controller@paris_vp_state"),
    OverlaySlot::badged("vp_london", "London", "controller@london_vp_state"),
    OverlaySlot::badged("vp_moscow", "Moscow", "controller@moscow_vp_state"),
    OverlaySlot::badged("vp_berlin", "Berlin", "controller@berlin_vp_state"),
    OverlaySlot::badged("vp_rome", "Rome", "controller@rome_vp_state"),
    OverlaySlot::badged("vp_cairo", "Cairo", "controller@cairo_vp_state"),
];

/// M1 `horsttool.gui` — (iv) DEV-TOOL / INERT (mutation console). The binding map §1.1
/// 4-row readout table is the ONLY bound data; every other element is a mutation button
/// (inert in a read-only replay). The two occupation readouts are EXACT (when a state is
/// selected & occupied); the owner readout is DERIVED (the owner TAG is the exact captured
/// `states.<sel>.owner`; the static loc name-lookup is omitted, hence derived). The
/// selection-echo `toogle_mark` frame is UI interaction state with no replay meaning →
/// badged. All bound through the Captured no-fabrication path — never a fabricated value.
const HORSTTOOL: &[OverlaySlot] = &[
    OverlaySlot::state(
        "compliance_number",
        "Compliance",
        "occupation.compliance",
        Fidelity::Exact,
    ),
    OverlaySlot::state(
        "resistance_number",
        "Resistance",
        "occupation.resistance",
        Fidelity::Exact,
    ),
    OverlaySlot::state("owner_title", "Owner", "owner", Fidelity::Derived),
    // selection-echo FRAME token — `ROOT.mark_status` is a genuine per-country script var
    // (the host-tool reads it; provenance-confirmed a `frame` var). It IS in the recorded
    // `script_variables` bag, so bind it: a country that set it shows the measured frame,
    // a country that never set it badges "—" (no fabrication, no fake 0).
    OverlaySlot::country_var("toogle_mark", "Marked", "mark_status"),
];

/// M5 `naval_scripted_gui.gui` — (i) CHROME-ONLY — per-admiral command-limit readout.
/// The `[?*_count]` hull counts are transient country temp vars + `[Get*]` GUI script
/// functions; neither is in the snapshot (binding map §1.4) → all badged.
const NAVAL_SCRIPTED: &[OverlaySlot] = &[
    OverlaySlot::badged("admiral_name", "Admiral", "GetAdmiralNameTitle"),
    OverlaySlot::badged("ship_count", "Ships", "GetShipCount"),
    OverlaySlot::badged("command_limit", "Command limit", "GetCommandLimit"),
    OverlaySlot::badged("screen_count", "Screens", "GetScreenCount"),
    OverlaySlot::badged("destroyer_count", "Destroyers", "destroyer_count"),
    OverlaySlot::badged(
        "light_cruiser_count",
        "Light cruisers",
        "light_cruiser_count",
    ),
    OverlaySlot::badged(
        "heavy_cruiser_count",
        "Heavy cruisers",
        "heavy_cruiser_count",
    ),
    OverlaySlot::badged("battleship_count", "Battleships", "battleship_count"),
    OverlaySlot::badged(
        "battlecruiser_count",
        "Battlecruisers",
        "battlecruiser_count",
    ),
    OverlaySlot::badged("carrier_count", "Carriers", "carrier_count"),
    OverlaySlot::badged("submarine_count", "Submarines", "submarine_count"),
    OverlaySlot::badged("capital_count", "Capitals", "capital_count"),
];

/// M6 `SWOA_focus_background.gui` — (iii) STATIC DECORATION — one decorative backdrop.
const SWOA_BG: &[OverlaySlot] = &[OverlaySlot::badged(
    "SWOA_focus_background_image",
    "",
    "THIS.GetFocusBG",
)];

/// M7 `game_check.gui` — (iv) DEV-TOOL / INERT — game-rules dev/debug echo (no data).
const GAME_CHECK: &[OverlaySlot] = &[];

// ---- propaganda poster boards (mod ×8) — (i) CHROME-ONLY (active frame country-scoped).

const PROP_GER: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "ger_poster_slot_left",
        "Poster (left)",
        "GER_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "ger_poster_slot_center",
        "Poster (center)",
        "GER_poster_center_icon_frame",
    ),
];
const PROP_ENG: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "eng_poster_slot_left",
        "Poster (left)",
        "ENG_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "eng_poster_slot_center",
        "Poster (center)",
        "ENG_poster_center_icon_frame",
    ),
];
const PROP_FRA: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "fra_poster_slot_left",
        "Poster (left)",
        "FRA_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "fra_poster_slot_center",
        "Poster (center)",
        "FRA_poster_center_icon_frame",
    ),
];
const PROP_ITA: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "ita_poster_slot_left",
        "Poster (left)",
        "ITA_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "ita_poster_slot_center",
        "Poster (center)",
        "ITA_poster_center_icon_frame",
    ),
];
const PROP_JAP: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "jap_poster_slot_left",
        "Poster (left)",
        "JAP_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "jap_poster_slot_center",
        "Poster (center)",
        "JAP_poster_center_icon_frame",
    ),
];
const PROP_USA: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "usa_poster_slot_left",
        "Poster (left)",
        "USA_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "usa_poster_slot_center",
        "Poster (center)",
        "USA_poster_center_icon_frame",
    ),
];
const PROP_COM: &[OverlaySlot] = &[OverlaySlot::country_var(
    "com_poster_slot",
    "Poster",
    "COM_poster_center_icon_frame",
)];
/// `non_major_propaganda_campaigns_scripted_gui.gui` bundles the 4 ideology variants
/// (axis/allies/man/mon windows) — each poster-slot FRAME is a per-country script var
/// (`*_poster_center_icon_frame`), now recorded in the snapshot → bound (badge "—" when
/// the observed country never set it).
const PROP_NON_MAJOR: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "allies_poster_slot",
        "Allies poster",
        "ALLIES_poster_center_icon_frame",
    ),
    OverlaySlot::country_var(
        "axis_poster_slot",
        "Axis poster",
        "AXIS_poster_center_icon_frame",
    ),
    OverlaySlot::country_var(
        "man_poster_slot",
        "Manchukuo poster",
        "MAN_poster_center_icon_frame",
    ),
    OverlaySlot::country_var(
        "mon_poster_slot",
        "Mongolia poster",
        "MON_poster_center_icon_frame",
    ),
];

// ---- vanilla nation-system boards (23) — (i) CHROME-ONLY (country-scoped vars).

const V_GER_INNER: &[OverlaySlot] = &[
    OverlaySlot::badged(
        "inner_circle_influence",
        "Inner-circle influence",
        "GER_inner_circle_influence",
    ),
    OverlaySlot::badged(
        "inner_circle_loyalty",
        "Inner-circle loyalty",
        "GER_inner_circle_loyalty",
    ),
];
const V_GER_INNER_HIDDEN: &[OverlaySlot] = &[OverlaySlot::badged(
    "inner_circle_hidden",
    "Inner circle (collapsed)",
    "GER_inner_circle_hidden",
)];
const V_GER_RK: &[OverlaySlot] = &[OverlaySlot::badged(
    "rk_status",
    "Reichskommissariat status",
    "GER_rk_status",
)];
const V_GER_MONROE: &[OverlaySlot] = &[
    OverlaySlot::badged(
        "monroe_in_doctrine",
        "In doctrine",
        "GER_monroe_in_doctrine_count",
    ),
    OverlaySlot::badged(
        "monroe_in_support",
        "In support",
        "GER_monroe_in_support_count",
    ),
    OverlaySlot::badged(
        "monroe_in_opposition",
        "In opposition",
        "GER_monroe_in_opposition_count",
    ),
];
const V_JAP_INFLUENCE: &[OverlaySlot] = &[OverlaySlot::badged(
    "imperial_influence",
    "Imperial influence",
    "JAP_imperial_influence",
)];
/// V6 `sov_paranoia_system_scripted_gui` — meter + 17 paranoia-target portrait frames
/// (5 named + 12 generic array) — all SOV-scoped, all badged. Full 19-element coverage
/// matching `scripted_guis.json` (no element dropped).
const V_SOV_PARANOIA: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "paranoia_main_paranoiameter",
        "Paranoia meter",
        "SOV_paranoia_main_paranoiameter",
    ),
    OverlaySlot::country_var(
        "paranoia_main_needle",
        "Paranoia needle",
        "SOV_paranoia_main_paranoiameter",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_zinovyev",
        "Zinovyev",
        "SOV_paranoia_photo_zinovyev",
    ),
    OverlaySlot::country_var("paranoia_photo_radek", "Radek", "SOV_paranoia_photo_radek"),
    OverlaySlot::country_var(
        "paranoia_photo_bukharin",
        "Bukharin",
        "SOV_paranoia_photo_bukharin",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_trotsky",
        "Trotsky",
        "SOV_paranoia_photo_trotsky",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_tukhachevsky",
        "Tukhachevsky",
        "SOV_paranoia_photo_tukhachevsky",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_1",
        "Target 1",
        "SOV_paranoia_generic_portraits_array^1",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_2",
        "Target 2",
        "SOV_paranoia_generic_portraits_array^2",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_3",
        "Target 3",
        "SOV_paranoia_generic_portraits_array^3",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_4",
        "Target 4",
        "SOV_paranoia_generic_portraits_array^4",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_5",
        "Target 5",
        "SOV_paranoia_generic_portraits_array^5",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_6",
        "Target 6",
        "SOV_paranoia_generic_portraits_array^6",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_7",
        "Target 7",
        "SOV_paranoia_generic_portraits_array^7",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_8",
        "Target 8",
        "SOV_paranoia_generic_portraits_array^8",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_9",
        "Target 9",
        "SOV_paranoia_generic_portraits_array^9",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_10",
        "Target 10",
        "SOV_paranoia_generic_portraits_array^10",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_11",
        "Target 11",
        "SOV_paranoia_generic_portraits_array^11",
    ),
    OverlaySlot::country_var(
        "paranoia_photo_12",
        "Target 12",
        "SOV_paranoia_generic_portraits_array^12",
    ),
];
const V_SOV_PROP: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "propaganda_poster_left",
        "Poster (left)",
        "SOV_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "propaganda_poster_center",
        "Poster (center)",
        "SOV_poster_center_icon_frame",
    ),
    OverlaySlot::country_var(
        "propaganda_poster_right",
        "Poster (right)",
        "SOV_poster_right_icon_frame",
    ),
];
const V_USA_CONGRESS: &[OverlaySlot] = &[
    OverlaySlot::country_var("senate_icon", "Senate support", "current_senate_icon_frame"),
    OverlaySlot::country_var("house_icon", "House support", "current_house_icon_frame"),
];
const V_SWE_REARM: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "swe_navy_icon",
        "Navy rearmament",
        "SWE_main_rearnament_frame",
    ),
    OverlaySlot::country_var(
        "swe_army_icon",
        "Army rearmament",
        "SWE_main_rearnament_frame",
    ),
    OverlaySlot::country_var(
        "swe_air_icon",
        "Air rearmament",
        "SWE_main_rearnament_frame",
    ),
    OverlaySlot::country_var(
        "swe_folkhemmet_icon",
        "Folkhemmet",
        "SWE_main_rearnament_frame",
    ),
];
const V_SWE_SHELTERED: &[OverlaySlot] = &[OverlaySlot::badged(
    "train_the_sheltered",
    "Train the sheltered",
    "SWE_train_the_sheltered",
)];
const V_AST_TRUST: &[OverlaySlot] = &[
    OverlaySlot::badged(
        "AST_cabinet_trust_state_1_logo",
        "Minister 1 trust",
        "GET_CABINET_TRUST_STATE_1",
    ),
    OverlaySlot::badged(
        "AST_cabinet_trust_state_2_logo",
        "Minister 2 trust",
        "GET_CABINET_TRUST_STATE_2",
    ),
    OverlaySlot::badged(
        "AST_cabinet_trust_state_3_logo",
        "Minister 3 trust",
        "GET_CABINET_TRUST_STATE_3",
    ),
    OverlaySlot::badged(
        "AST_cabinet_trust_state_4_logo",
        "Minister 4 trust",
        "GET_CABINET_TRUST_STATE_4",
    ),
    OverlaySlot::badged(
        "AST_cabinet_trust_state_5_logo",
        "Minister 5 trust",
        "GET_CABINET_TRUST_STATE_5",
    ),
];
const V_AST_RVL: &[OverlaySlot] = &[OverlaySlot::badged(
    "right_vs_left_track",
    "Right-vs-left campaign",
    "AST_right_vs_left_campaign",
)];
const V_AST_RVL_EMPTY: &[OverlaySlot] = &[OverlaySlot::badged(
    "right_vs_left_empty",
    "Campaign (inactive)",
    "AST_right_vs_left_empty",
)];
const V_AUS_ANTISCHLUSS: &[OverlaySlot] = &[OverlaySlot::badged(
    "antischluss_risk",
    "Anti-Anschluss risk",
    "AUS_antischluss_risk",
)];
const V_BUL_FACTIONS: &[OverlaySlot] = &[OverlaySlot::badged(
    "internal_factions_balance",
    "Internal factions",
    "BUL_internal_factions",
)];
const V_GRE_FACTIONS: &[OverlaySlot] = &[OverlaySlot::badged(
    "internal_factions_balance",
    "Internal factions",
    "GRE_internal_factions",
)];
const V_CZE_INDUSTRY: &[OverlaySlot] = &[OverlaySlot::badged(
    "industry_focus_allocation",
    "Industry focus",
    "CZE_industry_focus",
)];
const V_SIA_KHANA: &[OverlaySlot] = &[OverlaySlot::badged(
    "khana_ratsadon_influence",
    "Khana Ratsadon influence",
    "SIA_khana_ratsadon",
)];
const V_SIA_MOVIE: &[OverlaySlot] = &[
    OverlaySlot::country_var(
        "propaganda_poster_left",
        "Poster (left)",
        "SIA_poster_left_icon_frame",
    ),
    OverlaySlot::country_var(
        "propaganda_poster_center",
        "Poster (center)",
        "SIA_poster_center_icon_frame",
    ),
    OverlaySlot::country_var(
        "propaganda_poster_right",
        "Poster (right)",
        "SIA_poster_right_icon_frame",
    ),
];
const V_RAJ_FAMINE: &[OverlaySlot] = &[
    OverlaySlot::badged(
        "famine_timeframe",
        "Famine timeframe",
        "RAJ_famine_timeframe",
    ),
    OverlaySlot::badged(
        "famine_spread_risk",
        "Spread risk",
        "RAJ_famine_spread_risk",
    ),
];
const V_RAJ_TAX: &[OverlaySlot] = &[OverlaySlot::badged(
    "eic_tax_fraud",
    "EIC tax-fraud investigation",
    "RAJ_eic_tax_fraud",
)];
const V_RAJ_INVEST: &[OverlaySlot] = &[OverlaySlot::badged(
    "local_leaders_investments",
    "Local-leader investments",
    "RAJ_investments",
)];
const V_WAR_ESCALATION: &[OverlaySlot] = &[
    OverlaySlot::badged(
        "lon_we_1_icon",
        "Escalation 1",
        "GET_WAR_ESCALATION_LEVEL_1_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_2_icon",
        "Escalation 2",
        "GET_WAR_ESCALATION_LEVEL_2_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_3_icon",
        "Escalation 3",
        "GET_WAR_ESCALATION_LEVEL_3_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_4_icon",
        "Escalation 4",
        "GET_WAR_ESCALATION_LEVEL_4_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_5_icon",
        "Escalation 5",
        "GET_WAR_ESCALATION_LEVEL_5_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_6_icon",
        "Escalation 6",
        "GET_WAR_ESCALATION_LEVEL_6_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_7_icon",
        "Escalation 7",
        "GET_WAR_ESCALATION_LEVEL_7_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_8_icon",
        "Escalation 8",
        "GET_WAR_ESCALATION_LEVEL_8_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_9_icon",
        "Escalation 9",
        "GET_WAR_ESCALATION_LEVEL_9_COLOR",
    ),
    OverlaySlot::badged(
        "lon_we_10_icon",
        "Escalation 10",
        "GET_WAR_ESCALATION_LEVEL_10_COLOR",
    ),
];

/// THE 38 OVERLAY SPECS (BINDING_MAP §4: 15 mod + 23 vanilla = 38).
pub static OVERLAYS: &[OverlaySpec] = &[
    // ---- mod overlays (M1..M15) ----
    OverlaySpec {
        id: ViewId("overlay.horsttool"),
        label: "Host Tool",
        window: "horsttool_container",
        gui_file: "horsttool.gui",
        verdict: Verdict::DevToolInert,
        slots: HORSTTOOL,
        order: 1,
    },
    OverlaySpec {
        id: ViewId("overlay.topbarresources"),
        label: "Topbar Resources",
        window: "topbar_resource_container",
        gui_file: "topbarresources.gui",
        verdict: Verdict::PartiallyBindable,
        slots: TOPBARRESOURCES,
        order: 2,
    },
    OverlaySpec {
        id: ViewId("overlay.topbarvp"),
        label: "Topbar VP",
        window: "topbar_vp_container",
        gui_file: "topbarvp.gui",
        verdict: Verdict::PartiallyBindable,
        slots: TOPBARVP,
        order: 3,
    },
    OverlaySpec {
        id: ViewId("overlay.vp_scripted_gui"),
        label: "VP Explainer",
        window: "vp_explainer_window",
        gui_file: "vp_scripted_gui.gui",
        verdict: Verdict::PartiallyBindable,
        slots: VP_EXPLAINER,
        order: 4,
    },
    OverlaySpec {
        id: ViewId("overlay.naval_scripted_gui"),
        label: "Naval Command Limit",
        window: "admirals_leading_battlefleets_list",
        gui_file: "naval_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: NAVAL_SCRIPTED,
        order: 5,
    },
    OverlaySpec {
        id: ViewId("overlay.SWOA_focus_background"),
        label: "Focus Background",
        window: "SWOA_focus_background_container",
        gui_file: "SWOA_focus_background.gui",
        verdict: Verdict::StaticDecoration,
        slots: SWOA_BG,
        order: 6,
    },
    OverlaySpec {
        id: ViewId("overlay.game_check"),
        label: "Game-Rules Check",
        window: "check_gamerules_decision_ui_window",
        gui_file: "game_check.gui",
        verdict: Verdict::DevToolInert,
        slots: GAME_CHECK,
        order: 7,
    },
    OverlaySpec {
        id: ViewId("overlay.com_propaganda"),
        label: "Comintern Propaganda",
        window: "com_propaganda_campaigns_ui_window",
        gui_file: "com_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_COM,
        order: 8,
    },
    OverlaySpec {
        id: ViewId("overlay.eng_propaganda"),
        label: "British Propaganda",
        window: "eng_propaganda_campaigns_ui_window",
        gui_file: "eng_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_ENG,
        order: 9,
    },
    OverlaySpec {
        id: ViewId("overlay.fra_propaganda"),
        label: "French Propaganda",
        window: "fra_propaganda_campaigns_ui_window",
        gui_file: "fra_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_FRA,
        order: 10,
    },
    OverlaySpec {
        id: ViewId("overlay.ger_propaganda"),
        label: "German Propaganda",
        window: "ger_propaganda_campaigns_ui_window",
        gui_file: "ger_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_GER,
        order: 11,
    },
    OverlaySpec {
        id: ViewId("overlay.ita_propaganda"),
        label: "Italian Propaganda",
        window: "ita_propaganda_campaigns_ui_window",
        gui_file: "ita_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_ITA,
        order: 12,
    },
    OverlaySpec {
        id: ViewId("overlay.jap_propaganda"),
        label: "Japanese Propaganda",
        window: "jap_propaganda_campaigns_ui_window",
        gui_file: "jap_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_JAP,
        order: 13,
    },
    OverlaySpec {
        id: ViewId("overlay.usa_propaganda"),
        label: "American Propaganda",
        window: "usa_propaganda_campaigns_ui_window",
        gui_file: "usa_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_USA,
        order: 14,
    },
    OverlaySpec {
        id: ViewId("overlay.non_major_propaganda"),
        label: "Non-Major Propaganda",
        window: "non_major_propaganda_campaigns_ui_window",
        gui_file: "non_major_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: PROP_NON_MAJOR,
        order: 15,
    },
    // ---- vanilla overlays (V1..V23) ----
    OverlaySpec {
        id: ViewId("overlay.ger_inner_circle"),
        label: "GER Inner Circle",
        window: "GER_inner_circle_scripted_gui_window",
        gui_file: "GER_inner_circle_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_GER_INNER,
        order: 16,
    },
    OverlaySpec {
        id: ViewId("overlay.ger_inner_circle_hidden"),
        label: "GER Inner Circle (hidden)",
        window: "GER_inner_circle_hidden_scripted_gui_window",
        gui_file: "GER_inner_circle_hidden_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_GER_INNER_HIDDEN,
        order: 17,
    },
    OverlaySpec {
        id: ViewId("overlay.ger_rk"),
        label: "GER Reichskommissariat",
        window: "ger_rk_ui_window",
        gui_file: "GER_rk_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_GER_RK,
        order: 18,
    },
    OverlaySpec {
        id: ViewId("overlay.ger_monroe"),
        label: "GER Monroe Doctrine",
        window: "ger_monroe_doctrine_decision_ui_window",
        gui_file: "ger_monroe_doctrine_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_GER_MONROE,
        order: 19,
    },
    OverlaySpec {
        id: ViewId("overlay.jap_imperial_influence"),
        label: "JAP Imperial Influence",
        window: "JAP_imperial_influence_scripted_gui_window",
        gui_file: "JAP_imperial_influence_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_JAP_INFLUENCE,
        order: 20,
    },
    OverlaySpec {
        id: ViewId("overlay.sov_paranoia"),
        label: "SOV Paranoia",
        window: "sov_paranoia_system_ui_window",
        gui_file: "sov_paranoia_system_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_SOV_PARANOIA,
        order: 21,
    },
    OverlaySpec {
        id: ViewId("overlay.sov_propaganda"),
        label: "SOV Propaganda",
        window: "sov_propaganda_campaigns_ui_window",
        gui_file: "sov_propaganda_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_SOV_PROP,
        order: 22,
    },
    OverlaySpec {
        id: ViewId("overlay.usa_congress"),
        label: "USA Congress",
        window: "usa_congress_decision_ui_window",
        gui_file: "usa_congress_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_USA_CONGRESS,
        order: 23,
    },
    OverlaySpec {
        id: ViewId("overlay.swe_rearmament"),
        label: "SWE Rearmament",
        window: "swe_rearnament_system_ui_window",
        gui_file: "swe_rearnament_system_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_SWE_REARM,
        order: 24,
    },
    OverlaySpec {
        id: ViewId("overlay.swe_sheltered"),
        label: "SWE Train the Sheltered",
        window: "swe_train_the_sheltered_ui_window",
        gui_file: "swe_train_the_sheltered_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_SWE_SHELTERED,
        order: 25,
    },
    OverlaySpec {
        id: ViewId("overlay.ast_cabinet_trust"),
        label: "AST Cabinet Trust",
        window: "AST_cabinet_trust_ui_window",
        gui_file: "AST_cabinet_trust_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_AST_TRUST,
        order: 26,
    },
    OverlaySpec {
        id: ViewId("overlay.ast_right_vs_left"),
        label: "AST Right vs Left",
        window: "ast_right_vs_left_campaign_scripted_gui_window",
        gui_file: "ast_right_vs_left_campaign_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_AST_RVL,
        order: 27,
    },
    OverlaySpec {
        id: ViewId("overlay.ast_right_vs_left_empty"),
        label: "AST Right vs Left (empty)",
        window: "ast_right_vs_left_campaign_empty_scripted_gui_window",
        gui_file: "ast_right_vs_left_campaign_empty_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_AST_RVL_EMPTY,
        order: 28,
    },
    OverlaySpec {
        id: ViewId("overlay.aus_antischluss"),
        label: "AUS Anti-Anschluss",
        window: "aus_antischluss_measures_decision_ui_window",
        gui_file: "aus_antischluss_measures_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_AUS_ANTISCHLUSS,
        order: 29,
    },
    OverlaySpec {
        id: ViewId("overlay.bul_factions"),
        label: "BUL Internal Factions",
        window: "bul_internal_factions_decision_ui_window",
        gui_file: "bul_internal_factions_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_BUL_FACTIONS,
        order: 30,
    },
    OverlaySpec {
        id: ViewId("overlay.gre_factions"),
        label: "GRE Internal Factions",
        window: "gre_internal_factions_decision_ui_window",
        gui_file: "gre_internal_factions_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_GRE_FACTIONS,
        order: 31,
    },
    OverlaySpec {
        id: ViewId("overlay.cze_industry"),
        label: "CZE Industry Focus",
        window: "CZE_industry_focus_scripted_gui_window",
        gui_file: "CZE_industry_focus_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_CZE_INDUSTRY,
        order: 32,
    },
    OverlaySpec {
        id: ViewId("overlay.sia_khana_ratsadon"),
        label: "SIA Khana Ratsadon",
        window: "SIA_khana_ratsadon_scripted_gui_window",
        gui_file: "SIA_khana_ratsadon_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_SIA_KHANA,
        order: 33,
    },
    OverlaySpec {
        id: ViewId("overlay.sia_movie_theater"),
        label: "SIA Movie Theater",
        window: "sia_propaganda_campaigns_ui_window",
        gui_file: "sia_movie_theater_campaigns_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_SIA_MOVIE,
        order: 34,
    },
    OverlaySpec {
        id: ViewId("overlay.raj_famine"),
        label: "RAJ Famine",
        window: "RAJ_famine_ui_window",
        gui_file: "RAJ_famine.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_RAJ_FAMINE,
        order: 35,
    },
    OverlaySpec {
        id: ViewId("overlay.raj_eic_tax_fraud"),
        label: "RAJ EIC Tax Fraud",
        window: "RAJ_eic_taxes_ui_window",
        gui_file: "RAJ_eic_tax_fraud.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_RAJ_TAX,
        order: 36,
    },
    OverlaySpec {
        id: ViewId("overlay.raj_local_leaders"),
        label: "RAJ Local-Leader Investments",
        window: "raj_local_leaders_investments_decision_ui_window",
        gui_file: "raj_local_leaders_investments_decision_ui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_RAJ_INVEST,
        order: 37,
    },
    OverlaySpec {
        id: ViewId("overlay.war_escalation"),
        label: "War Escalation",
        window: "war_escalation_decision_ui_window",
        gui_file: "war_escalation_scripted_gui.gui",
        verdict: Verdict::ChromeOnly,
        slots: V_WAR_ESCALATION,
        order: 38,
    },
];

// ===========================================================================
// The generic overlay view instance (one impl drives all 38 from the spec).
// ===========================================================================

const OVERLAY_X: f32 = 360.0;
const OVERLAY_Y: f32 = 140.0;
const OVERLAY_W: f32 = 420.0;
const ROW_H: f32 = 26.0;
const HEADER_H: f32 = 64.0;

/// One scripted-GUI overlay instance. Holds the spec + the open arg (a selected state id
/// for horsttool's occupation readouts, an observed-country override otherwise). `bind`
/// is pure — it synthesizes a fresh scene-graph each tick from the immutable snapshot.
pub struct ScriptedOverlayView {
    spec: &'static OverlaySpec,
    /// horsttool: the SELECTED state id (occupation readouts read `states.<sel>.*`).
    selected_state: Option<i64>,
}

impl ScriptedOverlayView {
    pub fn new(spec: &'static OverlaySpec, arg: Option<i64>) -> ScriptedOverlayView {
        ScriptedOverlayView {
            spec,
            selected_state: arg,
        }
    }
}

impl ViewInstance for ScriptedOverlayView {
    fn view_id(&self) -> ViewId {
        self.spec.id
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }

    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let root = self.build_root(ctx, &mut fidelity);
        let roots = vec![root];
        BoundView {
            instance: InstanceId(0),
            view_id: self.spec.id,
            policy: self.policy(),
            // a panel-style window: its bounds block the map (Codex §3 guard) so a click
            // on the overlay's empty interior is consumed, not a province re-pick.
            hit: HitModel::from_roots(roots.clone()).with_block_rect(self.block_rect()),
            drawable: Box::new(NodeDrawable { roots }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

impl ScriptedOverlayView {
    fn block_rect(&self) -> Rect {
        let h = HEADER_H + self.spec.slots.len() as f32 * ROW_H + 12.0;
        Rect {
            x: OVERLAY_X,
            y: OVERLAY_Y,
            w: OVERLAY_W,
            h,
        }
    }

    /// Build the laid-out synthetic scene-graph for this overlay: a root window
    /// (background) + a title + a verdict-badge line + one row per slot (label + value),
    /// then `gui_runtime::layout`. Every value cell is a [`DisplayAtom`] (real Captured or
    /// "—"), tallied into `fidelity`.
    fn build_root(&self, ctx: &BindContext, fidelity: &mut FidelityReport) -> Node {
        let rect = self.block_rect();
        let mut children: Vec<Node> = Vec::new();

        // header: title + verdict badge.
        children.push(text_node(
            "overlay_title",
            self.spec.label,
            10.0,
            8.0,
            OVERLAY_W - 20.0,
            TextAlign::Left,
            "hoi_18mbs",
        ));
        children.push(text_node(
            "overlay_verdict",
            self.spec.verdict.label(),
            10.0,
            34.0,
            OVERLAY_W - 20.0,
            TextAlign::Left,
            "hoi_16mbs",
        ));

        // one row per slot: a fixed label + the value cell (the element node).
        for (i, slot) in self.spec.slots.iter().enumerate() {
            let y = HEADER_H + i as f32 * ROW_H;
            if !slot.label.is_empty() {
                children.push(text_node(
                    &format!("{}_label", slot.element),
                    slot.label,
                    10.0,
                    y,
                    200.0,
                    TextAlign::Left,
                    "hoi_16mbs",
                ));
            }
            let atom = self.resolve_slot(ctx, slot);
            fidelity.record(&atom);
            // the VALUE cell carries the element's real `.gui` name so a future binder /
            // accounting walk can find it by name.
            children.push(text_node(
                slot.element,
                atom.as_str(),
                216.0,
                y,
                OVERLAY_W - 226.0,
                TextAlign::Right,
                "hoi_16mbs",
            ));
        }

        // the root container with a window background (a real chrome sprite).
        let mut root = container_node(
            "overlay_root_bg",
            rect,
            Some("GFX_tile_window_bg".to_string()),
        );
        // place children relative to the root's top-left.
        for c in children.iter_mut() {
            c.pos = (rect.x + c.pos.0, rect.y + c.pos.1);
        }
        root.children = children;

        // lay out the whole tree at screen (inline, matching the topbar/state-panel
        // pattern — `ManifestContent` is generic over its measure closure).
        let cm = gui_runtime::ManifestContent {
            manifest: &ctx.stage.manifest,
            measure: |_t, _f| None,
        };
        gui_runtime::layout(&mut root, ctx.screen, None, &cm);
        root
    }

    /// Resolve ONE slot to a [`DisplayAtom`] — the no-fabrication seam. A badged slot ALWAYS
    /// renders the not-captured glyph; a country/state path binds via [`Captured`]; a world
    /// var badges (unfed in the demo + the cluster's raw-snapshot seam is audited-closed).
    fn resolve_slot(&self, ctx: &BindContext, slot: &OverlaySlot) -> DisplayAtom {
        match slot.source {
            SlotSource::Badged { .. } => {
                DisplayAtom::unavailable_because(UnavailableReason::Blocked)
            }
            SlotSource::CountryVar { var, fidelity } => {
                // bind the genuine per-country script variable now recorded in the snapshot
                // at `countries.<observed>.script_variables.<var>`. The dotted-path walk is
                // safe for array vars (`name^N` carries no '.') and the no-fabrication seam
                // ([`country_field`]) returns Absent when the country never set the var OR the
                // snapshot has no `script_variables` bag → badge "—", never a fabricated 0.
                let path = format!("script_variables.{var}");
                let cap = ctx.country_field(ctx.country, &path);
                country_atom(cap, fidelity)
            }
            SlotSource::WorldVar { .. } => {
                // GLOBAL world.variables — schema-present (§18) but unfed in the demo
                // snapshot AND not reachable through the cluster's audited accessor set.
                // Badge now; this lights up when a `world_field` Captured accessor lands.
                DisplayAtom::unavailable_because(UnavailableReason::Blocked)
            }
            SlotSource::CountryPath { path, fidelity } => {
                // bind under the observed country (player_context overlays resolve to it).
                let cap = ctx.country_field(ctx.country, path);
                country_atom(cap, fidelity)
            }
            SlotSource::StatePath { path, fidelity } => match self.selected_state {
                Some(sid) => {
                    let cap = ctx.state_field(&sid.to_string(), path);
                    country_atom(cap, fidelity)
                }
                // no selected state in a replay open → the readout is unavailable, not 0.
                None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
            },
        }
    }
}

/// Turn a `Captured<&Value>` into a display atom with the given measured-fidelity. A
/// number renders compactly; a string renders verbatim; Absent/Null → "—".
fn country_atom(cap: Captured<&serde_json::Value>, fidelity: Fidelity) -> DisplayAtom {
    // map &Value to an owned String only on the Measured arm (never fabricates).
    let owned: Captured<String> = cap.map(value_to_string);
    owned.display_with_fidelity(fidelity, |s| s)
}

/// Display a JSON value: integral floats lose the `.0`, strings are verbatim. (Only ever
/// called on a Measured value — there is no fabrication path here.)
fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 {
                    format!("{}", f as i64)
                } else {
                    format!("{f:.1}")
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// scene-graph construction helpers (synthetic chrome).
// ---------------------------------------------------------------------------

fn container_node(name: &str, rect: Rect, background: Option<String>) -> Node {
    Node {
        name: name.to_string(),
        raw_kind: "containerWindowType".into(),
        kind: ElementKind::Container { background },
        pos: (rect.x, rect.y),
        size_w: Some(gui_runtime::model::Scalar::Int(rect.w as i64)),
        size_h: Some(gui_runtime::model::Scalar::Int(rect.h as i64)),
        orientation: Anchor::UpperLeft,
        origo: Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect: Rect::default(),
        clip: None,
        children: vec![],
    }
}

fn text_node(
    name: &str,
    text: &str,
    x: f32,
    y: f32,
    max_w: f32,
    align: TextAlign,
    font: &str,
) -> Node {
    Node {
        name: name.to_string(),
        raw_kind: "instantTextBoxType".into(),
        kind: ElementKind::Text(TextSpec {
            text: text.to_string(),
            raw: text.to_string(),
            font: font.to_string(),
            align,
            max_width: Some(max_w),
            max_height: Some(ROW_H),
        }),
        pos: (x, y),
        size_w: None,
        size_h: None,
        orientation: Anchor::UpperLeft,
        origo: Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect: Rect::default(),
        clip: None,
        children: vec![],
    }
}

// ===========================================================================
// inventory registration — distributed, no shared-file edit. One descriptor per
// overlay, all generated from the OVERLAYS table via a macro that hands each a
// dedicated `open` factory fn (the factory must be a `fn` pointer, not a closure).
// ===========================================================================

/// Generate, for one overlay index, its `open` factory + the `register_view!` block.
macro_rules! register_overlay {
    ($fn_name:ident, $idx:expr) => {
        fn $fn_name(arg: Option<i64>) -> Box<dyn ViewInstance> {
            Box::new(ScriptedOverlayView::new(&OVERLAYS[$idx], arg))
        }
        register_view!(ViewDescriptor {
            id: OVERLAYS[$idx].id,
            gui_root: Some(GuiSymbolRef::root(
                OVERLAYS[$idx].gui_file,
                OVERLAYS[$idx].window,
            )),
            required_domains: REQUIRED_DOMAINS[$idx],
            route: Some(MenuRoute {
                cluster: MenuCluster::ScriptedGuiOverlays,
                label: OVERLAYS[$idx].label,
                order: OVERLAYS[$idx].order,
            }),
            shortcut: None,
            open: $fn_name,
            policy: HostPolicy::window(ZBand::Panel),
        });
    };
}

/// Per-overlay required snapshot domains (index-aligned with `OVERLAYS`). The 3 partial-
/// bind + horsttool name their real domains; an overlay whose slots bind a genuine
/// per-country script variable (the propaganda / paranoia / congress / rearmament boards)
/// requires `countries` (the `script_variables` bag lives under `countries.<TAG>`). The
/// remaining chrome-only / dev-tool / static overlays — whose backing token is a GUI
/// script function, computed colour, or an unconfirmed loc-text board — require none.
static REQUIRED_DOMAINS: &[&[&str]] = &[
    &["states", "countries"], // 0  horsttool (occupation under states + mark_status var)
    &["countries"],           // 1  topbarresources
    &["world"],               // 2  topbarvp
    &["world", "map", "states"], // 3 vp_scripted_gui
    &[],                      // 4  naval_scripted_gui (GUI script fns — badged)
    &[],                      // 5  SWOA_focus_background (GetFocusBG fn — badged)
    &[],                      // 6  game_check (dev tool)
    &["countries"],           // 7  com_propaganda     (poster-frame script var)
    &["countries"],           // 8  eng_propaganda
    &["countries"],           // 9  fra_propaganda
    &["countries"],           // 10 ger_propaganda
    &["countries"],           // 11 ita_propaganda
    &["countries"],           // 12 jap_propaganda
    &["countries"],           // 13 usa_propaganda
    &["countries"],           // 14 non_major_propaganda
    &[],                      // 15 ger_inner_circle      (loc-text board — unconfirmed, badged)
    &[],                      // 16 ger_inner_circle_hidden
    &[],                      // 17 ger_rk
    &[],                      // 18 ger_monroe
    &[],                      // 19 jap_imperial_influence
    &["countries"],           // 20 sov_paranoia          (paranoia portrait/meter script vars)
    &["countries"],           // 21 sov_propaganda        (poster-frame script var)
    &["countries"],           // 22 usa_congress          (senate/house frame script var)
    &["countries"],           // 23 swe_rearmament        (rearmament frame script var)
    &[],                      // 24 swe_sheltered          (loc-text board — unconfirmed, badged)
    &[],                      // 25 ast_cabinet_trust       (GET_CABINET_TRUST_* fn — badged)
    &[],                      // 26 ast_right_vs_left
    &[],                      // 27 ast_right_vs_left_empty
    &[],                      // 28 aus_antischluss
    &[],                      // 29 bul_factions
    &[],                      // 30 gre_factions
    &[],                      // 31 cze_industry
    &[],                      // 32 sia_khana_ratsadon
    &["countries"],           // 33 sia_movie_theater       (poster-frame script var)
    &[],                      // 34 raj_famine               (loc-text board — unconfirmed, badged)
    &[],                      // 35 raj_eic_tax_fraud
    &[],                      // 36 raj_local_leaders
    &[], // 37 war_escalation           (GET_WAR_ESCALATION_*_COLOR fn — badged)
];

register_overlay!(open_0, 0);
register_overlay!(open_1, 1);
register_overlay!(open_2, 2);
register_overlay!(open_3, 3);
register_overlay!(open_4, 4);
register_overlay!(open_5, 5);
register_overlay!(open_6, 6);
register_overlay!(open_7, 7);
register_overlay!(open_8, 8);
register_overlay!(open_9, 9);
register_overlay!(open_10, 10);
register_overlay!(open_11, 11);
register_overlay!(open_12, 12);
register_overlay!(open_13, 13);
register_overlay!(open_14, 14);
register_overlay!(open_15, 15);
register_overlay!(open_16, 16);
register_overlay!(open_17, 17);
register_overlay!(open_18, 18);
register_overlay!(open_19, 19);
register_overlay!(open_20, 20);
register_overlay!(open_21, 21);
register_overlay!(open_22, 22);
register_overlay!(open_23, 23);
register_overlay!(open_24, 24);
register_overlay!(open_25, 25);
register_overlay!(open_26, 26);
register_overlay!(open_27, 27);
register_overlay!(open_28, 28);
register_overlay!(open_29, 29);
register_overlay!(open_30, 30);
register_overlay!(open_31, 31);
register_overlay!(open_32, 32);
register_overlay!(open_33, 33);
register_overlay!(open_34, 34);
register_overlay!(open_35, 35);
register_overlay!(open_36, 36);
register_overlay!(open_37, 37);

/// Force this module to link (the binary calls cluster `link()`s; not required for
/// `inventory`, which auto-collects, but kept consistent with the Core surfaces).
pub fn link() {}

// ===========================================================================
// Tests — element accounting + render-plan digest + no-fabrication.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::Stage;
    use crate::views::digest::{bound_digest, nodes_digest};
    use gui_runtime::Loc;
    use std::collections::HashMap;

    fn stage() -> Stage {
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: HashMap::new(),
            },
            loc: Loc::new(),
            gui_files: HashMap::new(),
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    /// A snapshot with the topbarresources amounts fed + an occupied selected state, so
    /// the 3 partial-bind + horsttool readouts bind REAL data; `world` left absent (the
    /// DEMO caveat) so the VP banners badge.
    fn snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "countries": { "GER": { "economy": { "resources": { "available_derived": {
                "aluminium": 201.0, "rubber": 16.0, "tungsten": 12.0,
                "steel": 363.0, "chromium": 40.0, "coal": 250.0
            }}}}},
            "states": { "64": { "owner": "SOV",
                "occupation": { "compliance": 50.0, "resistance": 10.0 } } }
        }))
        .unwrap()
    }

    fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot) -> BindContext<'a> {
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
            date: "1940.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    // -----------------------------------------------------------------------
    // (a) ALL 38 overlays accounted for, none dropped, verdict tally exact.
    // -----------------------------------------------------------------------
    #[test]
    fn exactly_38_overlays_with_correct_tally() {
        assert_eq!(
            OVERLAYS.len(),
            38,
            "binding map §4: 15 mod + 23 vanilla = 38"
        );
        assert_eq!(
            REQUIRED_DOMAINS.len(),
            38,
            "domains table index-aligned with overlays"
        );
        // ids are unique.
        let mut ids: Vec<&str> = OVERLAYS.iter().map(|o| o.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 38, "every overlay id is unique");
        // the verdict tally matches §4: 32 chrome-only, 3 partial, 1 static, 2 dev-tool.
        let count = |v: Verdict| OVERLAYS.iter().filter(|o| o.verdict == v).count();
        assert_eq!(count(Verdict::ChromeOnly), 32, "32 chrome-only");
        assert_eq!(count(Verdict::PartiallyBindable), 3, "3 partially bindable");
        assert_eq!(count(Verdict::StaticDecoration), 1, "1 static decoration");
        assert_eq!(count(Verdict::DevToolInert), 2, "2 dev-tool / inert");
    }

    /// All 38 overlays register into the inventory (reachable from the menubar under the
    /// ScriptedGuiOverlays cluster) — proven through the REAL registry build.
    #[test]
    fn all_38_register_in_inventory() {
        use crate::host::ViewRegistry;
        let reg = ViewRegistry::from_inventory();
        for o in OVERLAYS {
            assert!(
                reg.view(o.id).is_some(),
                "overlay {} not collected by inventory",
                o.id
            );
        }
        let bar = reg.menubar();
        let group = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::ScriptedGuiOverlays)
            .expect("ScriptedGuiOverlays cluster present in menubar");
        assert_eq!(group.items.len(), 38, "all 38 overlays are menu-reachable");
    }

    // -----------------------------------------------------------------------
    // ELEMENT ACCOUNTING: the synthesized scene carries exactly one node per
    // scripted_guis.json element (by name), plus the fixed chrome — nothing dropped.
    // -----------------------------------------------------------------------
    #[test]
    fn element_accounting_scene_walk_matches_spec() {
        let stage = stage();
        let snap = snap();
        for o in OVERLAYS {
            let view = ScriptedOverlayView::new(o, Some(64));
            let bv = view.bind(&ctx(&stage, &snap));
            // every spec element must appear as a named node in the scene.
            for slot in o.slots {
                assert!(
                    find_named(&bv.hit.roots, slot.element),
                    "overlay {}: element `{}` missing from synthesized scene",
                    o.id,
                    slot.element
                );
            }
            // the value-node count equals the slot count (one value cell per element) —
            // no element silently dropped, none fabricated extra.
            let value_nodes = o
                .slots
                .iter()
                .filter(|s| find_named(&bv.hit.roots, s.element))
                .count();
            assert_eq!(
                value_nodes,
                o.slots.len(),
                "overlay {} element count mismatch",
                o.id
            );
        }
    }

    fn find_named(roots: &[Node], name: &str) -> bool {
        fn walk(n: &Node, name: &str) -> bool {
            n.name == name || n.children.iter().any(|c| walk(c, name))
        }
        roots.iter().any(|r| walk(r, name))
    }

    /// NON-TAUTOLOGICAL element accounting (Codex round 1, point a): assert coverage
    /// against the AUTHORITATIVE element names captured by the DEF parser in
    /// `game_defs/out/scripted_guis.json`, not a self-count. The frozen expectations
    /// below are the exact element sets that file emits for the overlays in our 38 whose
    /// def carries `elements` — if the DEF parser ever emits a NEW element for one of
    /// these windows, this test fails until the spec adds it (so nothing is silently
    /// dropped). Overlays whose def has zero `elements` (the loc-text-only vanilla
    /// boards) are covered by representative badged readouts and are not pinned here.
    #[test]
    fn element_accounting_against_def_source_of_truth() {
        let expected: &[(&str, &[&str])] = &[
            (
                "overlay.topbarresources",
                &[
                    "aluminium_value",
                    "rubber_value",
                    "tungsten_value",
                    "steel_value",
                    "chromium_value",
                    "coal_value",
                ],
            ),
            (
                "overlay.ger_propaganda",
                &["ger_poster_slot_left", "ger_poster_slot_center"],
            ),
            (
                "overlay.eng_propaganda",
                &["eng_poster_slot_left", "eng_poster_slot_center"],
            ),
            (
                "overlay.fra_propaganda",
                &["fra_poster_slot_left", "fra_poster_slot_center"],
            ),
            (
                "overlay.ita_propaganda",
                &["ita_poster_slot_left", "ita_poster_slot_center"],
            ),
            (
                "overlay.jap_propaganda",
                &["jap_poster_slot_left", "jap_poster_slot_center"],
            ),
            (
                "overlay.usa_propaganda",
                &["usa_poster_slot_left", "usa_poster_slot_center"],
            ),
            ("overlay.com_propaganda", &["com_poster_slot"]),
            (
                "overlay.non_major_propaganda",
                &[
                    "allies_poster_slot",
                    "axis_poster_slot",
                    "man_poster_slot",
                    "mon_poster_slot",
                ],
            ),
            (
                "overlay.sov_propaganda",
                &[
                    "propaganda_poster_left",
                    "propaganda_poster_center",
                    "propaganda_poster_right",
                ],
            ),
            (
                "overlay.sia_movie_theater",
                &[
                    "propaganda_poster_left",
                    "propaganda_poster_center",
                    "propaganda_poster_right",
                ],
            ),
            ("overlay.usa_congress", &["senate_icon", "house_icon"]),
            (
                "overlay.swe_rearmament",
                &[
                    "swe_navy_icon",
                    "swe_army_icon",
                    "swe_air_icon",
                    "swe_folkhemmet_icon",
                ],
            ),
            (
                "overlay.ast_cabinet_trust",
                &[
                    "AST_cabinet_trust_state_1_logo",
                    "AST_cabinet_trust_state_2_logo",
                    "AST_cabinet_trust_state_3_logo",
                    "AST_cabinet_trust_state_4_logo",
                    "AST_cabinet_trust_state_5_logo",
                ],
            ),
            (
                "overlay.war_escalation",
                &[
                    "lon_we_1_icon",
                    "lon_we_2_icon",
                    "lon_we_3_icon",
                    "lon_we_4_icon",
                    "lon_we_5_icon",
                    "lon_we_6_icon",
                    "lon_we_7_icon",
                    "lon_we_8_icon",
                    "lon_we_9_icon",
                    "lon_we_10_icon",
                ],
            ),
            (
                "overlay.sov_paranoia",
                &[
                    "paranoia_main_paranoiameter",
                    "paranoia_main_needle",
                    "paranoia_photo_zinovyev",
                    "paranoia_photo_radek",
                    "paranoia_photo_bukharin",
                    "paranoia_photo_trotsky",
                    "paranoia_photo_tukhachevsky",
                    "paranoia_photo_1",
                    "paranoia_photo_2",
                    "paranoia_photo_3",
                    "paranoia_photo_4",
                    "paranoia_photo_5",
                    "paranoia_photo_6",
                    "paranoia_photo_7",
                    "paranoia_photo_8",
                    "paranoia_photo_9",
                    "paranoia_photo_10",
                    "paranoia_photo_11",
                    "paranoia_photo_12",
                ],
            ),
            (
                "overlay.SWOA_focus_background",
                &["SWOA_focus_background_image"],
            ),
        ];
        for (id, want) in expected {
            let spec = OVERLAYS.iter().find(|o| o.id.as_str() == *id).unwrap();
            let have: Vec<&str> = spec.slots.iter().map(|s| s.element).collect();
            for w in *want {
                assert!(
                    have.contains(w),
                    "overlay {id}: DEF element `{w}` is in scripted_guis.json but missing from the spec (silently dropped)"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // (b) every script-var field is BADGED, never fabricated.
    // -----------------------------------------------------------------------
    #[test]
    fn every_badged_slot_renders_not_captured_never_a_value() {
        let stage = stage();
        let snap = snap();
        for o in OVERLAYS {
            let view = ScriptedOverlayView::new(o, Some(64));
            let cx = ctx(&stage, &snap);
            for slot in o.slots {
                let atom = view.resolve_slot(&cx, slot);
                match slot.source {
                    SlotSource::Badged { .. } | SlotSource::WorldVar { .. } => {
                        assert!(
                            atom.is_unavailable() && atom.as_str() == "—",
                            "overlay {}: badged slot `{}` must render '—', got {:?}",
                            o.id,
                            slot.element,
                            atom.as_str()
                        );
                        assert_ne!(atom.as_str(), "0", "never a fabricated 0");
                    }
                    // a CountryVar slot binds the script_variables bag — under this
                    // `snap()` (no bag) it is ABSENT, so it ALSO badges "—" (no-fab); the
                    // Measured path is covered by `country_var_slot_binds_when_bag_present`.
                    SlotSource::CountryVar { .. } => {
                        assert!(
                            atom.is_unavailable() && atom.as_str() == "—",
                            "overlay {}: country-var slot `{}` with no bag must badge '—', got {:?}",
                            o.id, slot.element, atom.as_str()
                        );
                        assert_ne!(atom.as_str(), "0", "never a fabricated 0");
                    }
                    SlotSource::CountryPath { .. } | SlotSource::StatePath { .. } => {}
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // (c) the 3 partial-bind actually bind REAL data.
    // -----------------------------------------------------------------------
    #[test]
    fn topbarresources_binds_real_amounts() {
        let stage = stage();
        let snap = snap();
        let spec = OVERLAYS
            .iter()
            .find(|o| o.id == ViewId("overlay.topbarresources"))
            .unwrap();
        let view = ScriptedOverlayView::new(spec, None);
        let cx = ctx(&stage, &snap);
        // aluminium=201, steel=363 are fed → measured, badged Derived, NOT "—".
        let alu = view.resolve_slot(&cx, &spec.slots[0]);
        assert_eq!(
            alu.as_str(),
            "201",
            "aluminium amount bound from economy.resources"
        );
        assert_eq!(alu.fidelity(), Fidelity::Derived);
        let steel = view.resolve_slot(&cx, &spec.slots[3]);
        assert_eq!(steel.as_str(), "363");
        assert!(!alu.is_unavailable());
    }

    #[test]
    fn horsttool_occupation_readouts_bind_real_state_data() {
        let stage = stage();
        let snap = snap();
        let spec = OVERLAYS
            .iter()
            .find(|o| o.id == ViewId("overlay.horsttool"))
            .unwrap();
        // opened on a selected, occupied state (64) → compliance/resistance are EXACT.
        let view = ScriptedOverlayView::new(spec, Some(64));
        let cx = ctx(&stage, &snap);
        let compl = view.resolve_slot(&cx, &spec.slots[0]);
        assert_eq!(
            compl.as_str(),
            "50",
            "compliance bound from states.64.occupation"
        );
        assert_eq!(compl.fidelity(), Fidelity::Exact);
        let resist = view.resolve_slot(&cx, &spec.slots[1]);
        assert_eq!(resist.as_str(), "10");
        // the binding-map §1.1 owner readout binds the REAL `states.64.owner` TAG (derived).
        let owner = view.resolve_slot(&cx, &spec.slots[2]);
        assert_eq!(
            owner.as_str(),
            "SOV",
            "owner readout binds states.<sel>.owner TAG"
        );
        assert_eq!(owner.fidelity(), Fidelity::Derived);
        // with NO selected state, the same readout is unavailable (not a fabricated 0).
        let view_none = ScriptedOverlayView::new(spec, None);
        let none_atom = view_none.resolve_slot(&cx, &spec.slots[0]);
        assert!(none_atom.is_unavailable());
        assert_ne!(none_atom.as_str(), "0");
        // the owner readout is likewise unavailable with no selection (never a fake tag).
        assert!(view_none.resolve_slot(&cx, &spec.slots[2]).is_unavailable());
    }

    #[test]
    fn vp_score_banners_badge_when_world_absent() {
        let stage = stage();
        let snap = snap(); // no `world` domain
        let spec = OVERLAYS
            .iter()
            .find(|o| o.id == ViewId("overlay.topbarvp"))
            .unwrap();
        let view = ScriptedOverlayView::new(spec, None);
        let cx = ctx(&stage, &snap);
        for slot in spec.slots {
            let atom = view.resolve_slot(&cx, slot);
            assert!(
                atom.is_unavailable(),
                "VP banner badges while world.variables unfed"
            );
            assert_eq!(atom.reason(), Some(UnavailableReason::Blocked));
        }
    }

    // -----------------------------------------------------------------------
    // RENDER-PLAN DIGEST: every overlay renders chrome; scrub A→B→A byte-identical.
    // -----------------------------------------------------------------------
    #[test]
    fn every_overlay_renders_chrome_under_scrubber() {
        let stage = stage();
        let snap = snap();
        for o in OVERLAYS {
            let view = ScriptedOverlayView::new(o, Some(64));
            let bv = view.bind(&ctx(&stage, &snap));
            let digest = nodes_digest(&bv.hit.roots);
            // chrome at minimum: the root + title node always present.
            assert!(
                !digest.is_empty(),
                "overlay {} produced an empty render plan",
                o.id
            );
            assert!(
                digest.contains("overlay_title"),
                "overlay {} missing title chrome",
                o.id
            );
        }
    }

    #[test]
    fn scrub_a_b_a_byte_identical_for_partial_bind() {
        let stage = stage();
        let a = snap();
        // B: different resource amounts → A and B digests differ; A re-binds identical.
        let b: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.1.1.12",
            "countries": { "GER": { "economy": { "resources": { "available_derived": {
                "aluminium": 999.0, "steel": 111.0
            }}}}},
            "states": { "64": { "owner": "SOV", "occupation": { "compliance": 5.0, "resistance": 90.0 } } }
        }))
        .unwrap();
        let spec = OVERLAYS
            .iter()
            .find(|o| o.id == ViewId("overlay.topbarresources"))
            .unwrap();
        let view = ScriptedOverlayView::new(spec, None);
        let da1 = bound_digest(&view.bind(&ctx(&stage, &a)));
        let db = bound_digest(&view.bind(&ctx(&stage, &b)));
        assert_ne!(
            da1, db,
            "A and B must differ, else the scrub test proves nothing"
        );
        let da2 = bound_digest(&view.bind(&ctx(&stage, &a)));
        assert_eq!(
            da1, da2,
            "scrub A→B→A: overlay carried a value from the B visit"
        );
    }

    #[test]
    fn chrome_only_overlays_badge_every_slot_when_var_bag_absent() {
        // No-fabrication contract: under a snapshot WITHOUT the per-country
        // `script_variables` bag (`snap()` omits it), EVERY chrome-only overlay slot —
        // whether a GUI-script-function `Badged` slot or a now-bindable `CountryVar` slot
        // whose backing var is simply not present — badges "—", never measures a value and
        // never fabricates a 0. (The Measured arm for `CountryVar` is covered by
        // `country_var_slot_binds_when_bag_present`; with the bag PRESENT a chrome-only
        // overlay legitimately binds — that is the whole point of the re-wire.)
        let stage = stage();
        let snap = snap();
        for o in OVERLAYS.iter().filter(|o| o.verdict == Verdict::ChromeOnly) {
            let view = ScriptedOverlayView::new(o, Some(64));
            let bv = view.bind(&ctx(&stage, &snap));
            assert_eq!(
                bv.fidelity.exact, 0,
                "chrome-only {} bound an EXACT value (no bag)",
                o.id
            );
            assert_eq!(
                bv.fidelity.derived, 0,
                "chrome-only {} bound a DERIVED value (no bag)",
                o.id
            );
            assert_eq!(
                bv.fidelity.unavailable as usize,
                o.slots.len(),
                "chrome-only {} must badge every slot when the var bag is absent",
                o.id
            );
        }
    }

    // -----------------------------------------------------------------------
    // RE-WIRE: the now-recorded per-country `script_variables` bag lights up the
    // CountryVar slots — Measured shows the value, Absent still badges (no-fab).
    // -----------------------------------------------------------------------

    /// A snapshot carrying the per-country `script_variables` bag (the variables_live fold).
    /// USA holds the congress senate/house frame vars + a SOV-style paranoia array var,
    /// proving the `name^N` array key round-trips through the dotted-path bind.
    fn snap_with_vars() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "countries": { "USA": {
                "script_variables": {
                    "current_senate_icon_frame": 11.0,
                    "current_house_icon_frame": 7.0,
                    // a measured-0 frame is a REAL value, must render "0", not "—".
                    "SOV_paranoia_main_paranoiameter": 0.0,
                    "SOV_paranoia_generic_portraits_array^1": 3.0
                }
            }, "GER": {
                // GER measured an EMPTY bag: a real read of an empty container. Every
                // var lookup is Absent → badge, never a fabricated 0.
                "script_variables": {}
            } }
        }))
        .unwrap()
    }

    fn ctx_for<'a>(
        stage: &'a Stage,
        snap: &'a snapshot::Snapshot,
        country: &'a str,
    ) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country,
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1940.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    /// A CountryVar slot binds the MEASURED script variable when the bag carries it (the
    /// re-wire's whole point): the USA congress senate/house frames show their measured
    /// values at Derived fidelity — NOT "—".
    #[test]
    fn country_var_slot_binds_when_bag_present() {
        let stage = stage();
        let snap = snap_with_vars();
        let cx = ctx_for(&stage, &snap, "USA");
        let spec = OVERLAYS
            .iter()
            .find(|o| o.id == ViewId("overlay.usa_congress"))
            .unwrap();
        let view = ScriptedOverlayView::new(spec, None);
        let senate = view.resolve_slot(&cx, &spec.slots[0]); // current_senate_icon_frame=11
        assert_eq!(
            senate.as_str(),
            "11",
            "senate frame bound from script_variables"
        );
        assert_eq!(senate.fidelity(), Fidelity::Derived);
        assert!(!senate.is_unavailable());
        let house = view.resolve_slot(&cx, &spec.slots[1]); // current_house_icon_frame=7
        assert_eq!(house.as_str(), "7");
    }

    /// A MEASURED 0 frame is a real value: it renders "0", never the not-captured glyph
    /// (the inverse of fabrication — a real 0 must not be hidden).
    #[test]
    fn country_var_measured_zero_renders_zero_not_dash() {
        let stage = stage();
        let snap = snap_with_vars();
        let cx = ctx_for(&stage, &snap, "USA");
        let spec = OVERLAYS
            .iter()
            .find(|o| o.id == ViewId("overlay.sov_paranoia"))
            .unwrap();
        let view = ScriptedOverlayView::new(spec, None);
        // slots[0] = paranoia_main_paranoiameter = SOV_paranoia_main_paranoiameter = 0.0
        let meter = view.resolve_slot(&cx, &spec.slots[0]);
        assert_eq!(
            meter.as_str(),
            "0",
            "a measured-0 script var renders 0, not —"
        );
        assert!(!meter.is_unavailable());
        // the array var `^1` round-trips the dotted-path bind.
        let arr = view.resolve_slot(&cx, &spec.slots[7]); // paranoia_photo_1 = ...array^1 = 3
        assert_eq!(
            arr.as_str(),
            "3",
            "name^N array var binds via the dotted path"
        );
    }

    /// A CountryVar slot still BADGES when the var is not in the country's bag: GER measured
    /// an empty bag, so every USA-paranoia/congress var is Absent → "—", never a fake 0.
    #[test]
    fn country_var_slot_badges_when_var_absent_from_bag() {
        let stage = stage();
        let snap = snap_with_vars();
        let cx = ctx_for(&stage, &snap, "GER"); // empty bag
        for id in [
            "overlay.usa_congress",
            "overlay.sov_paranoia",
            "overlay.ger_propaganda",
        ] {
            let spec = OVERLAYS.iter().find(|o| o.id.as_str() == id).unwrap();
            let view = ScriptedOverlayView::new(spec, None);
            for slot in spec.slots {
                if let SlotSource::CountryVar { .. } = slot.source {
                    let atom = view.resolve_slot(&cx, slot);
                    assert!(
                        atom.is_unavailable(),
                        "{id}/{} must badge (var absent)",
                        slot.element
                    );
                    assert_eq!(atom.as_str(), "—");
                    assert_ne!(atom.as_str(), "0", "never a fabricated 0");
                }
            }
        }
    }

    /// PROVENANCE GUARD (Codex round): a slot is converted to `CountryVar` ONLY when its
    /// backing token is provenance-confirmed a genuine per-country script variable — the
    /// `frame` token of a DEF element in `game_defs/out/scripted_guis.json` (a stored
    /// variable), NOT a `[Get*]`/`[GET_*]` GUI script function, a computed `_COLOR`, or a
    /// `THIS.`/`@`-scoped flag lookup (which are never in the `script_variables` bag). This
    /// frozen allowlist is the exact set of provenance-confirmed `frame` var tokens; any
    /// CountryVar var outside it is a mis-classification (would present unrelated data).
    #[test]
    fn provenance_confirms_country_var_slots() {
        // The 45 provenance-confirmed `frame` var tokens (DEF element `frame`, non-numeric).
        const CONFIRMED: &[&str] = &[
            "ALLIES_poster_center_icon_frame",
            "AXIS_poster_center_icon_frame",
            "COM_poster_center_icon_frame",
            "ENG_poster_center_icon_frame",
            "ENG_poster_left_icon_frame",
            "FRA_poster_center_icon_frame",
            "FRA_poster_left_icon_frame",
            "GER_poster_center_icon_frame",
            "GER_poster_left_icon_frame",
            "ITA_poster_center_icon_frame",
            "ITA_poster_left_icon_frame",
            "JAP_poster_center_icon_frame",
            "JAP_poster_left_icon_frame",
            "MAN_poster_center_icon_frame",
            "MON_poster_center_icon_frame",
            "SIA_poster_center_icon_frame",
            "SIA_poster_left_icon_frame",
            "SIA_poster_right_icon_frame",
            "SOV_paranoia_generic_portraits_array^1",
            "SOV_paranoia_generic_portraits_array^2",
            "SOV_paranoia_generic_portraits_array^3",
            "SOV_paranoia_generic_portraits_array^4",
            "SOV_paranoia_generic_portraits_array^5",
            "SOV_paranoia_generic_portraits_array^6",
            "SOV_paranoia_generic_portraits_array^7",
            "SOV_paranoia_generic_portraits_array^8",
            "SOV_paranoia_generic_portraits_array^9",
            "SOV_paranoia_generic_portraits_array^10",
            "SOV_paranoia_generic_portraits_array^11",
            "SOV_paranoia_generic_portraits_array^12",
            "SOV_paranoia_main_paranoiameter",
            "SOV_paranoia_photo_bukharin",
            "SOV_paranoia_photo_radek",
            "SOV_paranoia_photo_trotsky",
            "SOV_paranoia_photo_tukhachevsky",
            "SOV_paranoia_photo_zinovyev",
            "SOV_poster_center_icon_frame",
            "SOV_poster_left_icon_frame",
            "SOV_poster_right_icon_frame",
            "SWE_main_rearnament_frame",
            "USA_poster_center_icon_frame",
            "USA_poster_left_icon_frame",
            "current_house_icon_frame",
            "current_senate_icon_frame",
            "mark_status",
        ];
        // every CountryVar in every overlay must be in the provenance allowlist.
        for o in OVERLAYS {
            for slot in o.slots {
                if let SlotSource::CountryVar { var, .. } = slot.source {
                    assert!(
                        CONFIRMED.contains(&var),
                        "overlay {}: CountryVar `{}` (var `{}`) is NOT provenance-confirmed \
                         a script variable — keep it Badged or confirm via scripted_guis.json",
                        o.id,
                        slot.element,
                        var
                    );
                }
            }
        }
        // and the converse: at least the 45 confirmed bind (the re-wire actually happened).
        let bound: std::collections::HashSet<&str> = OVERLAYS
            .iter()
            .flat_map(|o| o.slots.iter())
            .filter_map(|s| match s.source {
                SlotSource::CountryVar { var, .. } => Some(var),
                _ => None,
            })
            .collect();
        // SOV_paranoia_main_paranoiameter backs 2 elements (meter+needle); SWE one var, 4
        // elements; posters share none — so the DISTINCT bound var set is 43 (the 45 tokens
        // minus the 2 that are aliases of an already-counted var). Assert the big ones landed.
        for must in [
            "GER_poster_left_icon_frame",
            "SOV_paranoia_generic_portraits_array^12",
            "current_senate_icon_frame",
            "mark_status",
            "SWE_main_rearnament_frame",
        ] {
            assert!(
                bound.contains(must),
                "re-wire missed confirmed var `{must}`"
            );
        }
    }
}
