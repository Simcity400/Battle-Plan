//! BUNDLE-MANIFEST element accounting (Codex fix #2).
//!
//! "Render real `.gui` ROOT" is not enough for a file with MANY window roots
//! (`officer_corps.json` has 31, `unit_leader.json` 15). So each file is treated as a
//! BUNDLE MANIFEST: every declared top-level root/template is classified EXACTLY ONCE as
//!   * [`RootRole::WindowRoot`]  — a routed window the view's `bind` renders + lays out,
//!   * [`RootRole::RowTemplate`] — a repeated row instantiated by the row factory,
//!   * [`RootRole::Prototype`]   — a node/line prototype the trait-tree adapter clones,
//!   * [`RootRole::Gap`]         — a TRUE-GAP surface (no schema home) we badge unavailable.
//!
//! The test (`tests.rs::every_declared_root_classified`) asserts the manifest's root set
//! equals the FILE's actual declared root set (`covered_root_ids == all_declared_root_ids`)
//! AND that the summed element_type_counts over the whole file equals the raw walk — so no
//! root is orphaned and every element is accounted, across ALL roots, not one.

/// How one declared `.gui` window root / template is used by the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootRole {
    /// A routed window an actual `ViewInstance::bind` renders (`build_root`). MUST appear in
    /// [`ROUTED_ROOTS`] (enforced by `window_roots_are_rendered`), so a `WindowRoot` can
    /// never be dead, unrendered UI surface (Codex (a)).
    WindowRoot,
    /// A repeated row template instantiated per array element by the row factory.
    RowTemplate,
    /// A node/line prototype the trait-tree adapter clones (no standalone render).
    Prototype,
    /// An ALTERNATE window root reachable in-game but NOT yet bound by a routed view (a
    /// captured/operative/division/faction picker, a portrait sub-window, the spirit /
    /// HQ-template sub-window). Honestly listed as not-rendered so it is never counted as
    /// covered (Codex (a): demote unrendered window roots, don't pass them off as bound).
    AltSurface,
    /// A TRUE-GAP surface (no schema home today) — badged unavailable, listed for honesty.
    Gap,
}

/// One entry in a file's bundle manifest: the declared root name + its classification.
#[derive(Debug, Clone, Copy)]
pub struct RootEntry {
    pub name: &'static str,
    pub role: RootRole,
}

const fn root(name: &'static str) -> RootEntry {
    RootEntry {
        name,
        role: RootRole::WindowRoot,
    }
}
const fn tmpl(name: &'static str) -> RootEntry {
    RootEntry {
        name,
        role: RootRole::RowTemplate,
    }
}
const fn proto(name: &'static str) -> RootEntry {
    RootEntry {
        name,
        role: RootRole::Prototype,
    }
}
const fn alt(name: &'static str) -> RootEntry {
    RootEntry {
        name,
        role: RootRole::AltSurface,
    }
}
const fn gap(name: &'static str) -> RootEntry {
    RootEntry {
        name,
        role: RootRole::Gap,
    }
}

/// The window roots an actual `ViewInstance::bind` renders (one per routed view). The
/// `window_roots_are_rendered` invariant asserts every [`RootRole::WindowRoot`] across all
/// manifests is in this set — so no `WindowRoot` is unrendered dead surface (Codex (a)).
pub const ROUTED_ROOTS: &[&str] = &[
    "countryofficercorpview",
    "armyleaderwindow",
    "navy_leader_window",
    "leader_groups_portrait_window",
    "acesview",
    "unitleader_trait_tree_window",
    "navyleader_trait_tree_window",
];

/// The bundle manifest for `officer_corps.json` (31 declared roots). The view renders
/// `countryofficercorpview` + `armyhqtemplatewindow` + `spirit_window`; the advisor /
/// spirit-grid / spirit-pip / track-progress windows are row templates; the named-HQ-
/// template + faction-intel item are TRUE GAPs.
pub const OFFICER_CORPS: &[RootEntry] = &[
    root("countryofficercorpview"),
    alt("spirit_window"),
    alt("armyhqtemplatewindow"),
    tmpl("command_view_button_entry"),
    tmpl("country_view_advisor_entry"),
    tmpl("high_command_entry"),
    tmpl("army_chief_entry"),
    tmpl("navy_chief_entry"),
    tmpl("air_chief_entry"),
    tmpl("theorist_entry"),
    tmpl("track_progress_item"),
    tmpl("division_command_spirit_entry"),
    tmpl("army_spirit_entry"),
    tmpl("academy_spirit_entry"),
    tmpl("naval_command_spirit_entry"),
    tmpl("navy_spirit_entry"),
    tmpl("naval_academy_spirit_entry"),
    tmpl("air_force_command_spirit_entry"),
    tmpl("air_force_spirit_entry"),
    tmpl("air_force_academy_spirit_entry"),
    tmpl("academy_spirit_window_item_grid"),
    tmpl("army_spirit_window_item_grid"),
    tmpl("division_command_spirit_window_item_grid"),
    tmpl("naval_academy_spirit_window_item_grid"),
    tmpl("navy_spirit_window_item_grid"),
    tmpl("naval_command_spirit_window_item_grid"),
    tmpl("air_force_academy_spirit_window_item_grid"),
    tmpl("air_force_spirit_window_item_grid"),
    tmpl("air_force_command_spirit_window_item_grid"),
    gap("faction_intel_agency_char_view_item"),
    gap("named_hq_template_entry"),
];

/// The bundle manifest for `unit_leader.json` (15 declared roots). The six `*window` roots
/// are routed windows; the six `*entry` roots + the stray trait/medal/supreme templates are
/// row templates.
pub const UNIT_LEADER: &[RootEntry] = &[
    root("armyleaderwindow"),
    alt("capturedarmyleaderwindow"),
    alt("navyleaderwindow"),
    alt("operativeleaderwindow"),
    alt("divisioncommanderwindow"),
    alt("factioncommanderwindow"),
    tmpl("armyleaderentry"),
    tmpl("capturedarmyleaderentry"),
    tmpl("navyleaderentry"),
    tmpl("operativeleaderentry"),
    tmpl("divisionleaderentry"),
    tmpl("unit_leader_trait_entry"),
    tmpl("unit_leader_exile_trait_entry"),
    tmpl("division_medal_entry"),
    tmpl("supreme_commander_type_item"),
];

/// The bundle manifest for `navy_leader.json` (3 declared roots).
pub const NAVY_LEADER: &[RootEntry] = &[
    root("navy_leader_window"),
    alt("navy_leader_portrait_window"),
    alt("navy_reserve_portrait_window"),
];

/// The bundle manifest for `leader_groups.json` (7 declared roots). The portrait windows +
/// army-group window are routed; the attached-unit boxes are row templates.
pub const LEADER_GROUPS: &[RootEntry] = &[
    alt("leader_groups_window"),
    alt("leader_groups_portrait_window_bg"),
    root("leader_groups_portrait_window"),
    alt("army_group_window"),
    alt("army_group_portrait_window"),
    tmpl("group_wing_type_box"),
    tmpl("group_railway_guns_box"),
];

/// The bundle manifest for `aces.json` (2 declared roots).
pub const ACES: &[RootEntry] = &[root("acesview"), tmpl("acesview_entry")];

/// The bundle manifest for `army_trait_tree.json` (1 declared root).
pub const ARMY_TRAIT_TREE: &[RootEntry] = &[root("unitleader_trait_tree_window")];

/// The bundle manifest for `navy_trait_tree.json` (1 declared root).
pub const NAVY_TRAIT_TREE: &[RootEntry] = &[root("navyleader_trait_tree_window")];

/// The bundle manifest for `trait_tree_common.json` (24 declared roots — ALL prototypes
/// except the one `skill_tree_empty_window_for_line` line-spacer, also a prototype).
pub const TRAIT_TREE_COMMON: &[RootEntry] = &[
    proto("skill_tree_empty_window_for_line"),
    proto("skill_tree_exclusive_horizontal"),
    proto("skill_tree_exclusive_vertical"),
    proto("skill_tree_line_mid"),
    proto("unit_leader_trait"),
    proto("unit_leader_trait_tree_exclusive_vertical"),
    proto("unit_leader_trait_tree_connection_offset_orientation_horizontal"),
    proto("disabled_trait"),
    proto("pickable_trait"),
    proto("advisor_trait"),
    proto("picked_trait"),
    proto("trait_tree_line_left_dotted"),
    proto("trait_tree_line_right_dotted"),
    proto("trait_tree_line_top_dotted"),
    proto("trait_tree_line_bottom_dotted"),
    proto("trait_tree_line_left"),
    proto("trait_tree_line_right"),
    proto("trait_tree_line_top"),
    proto("trait_tree_line_bottom"),
    proto("trait_tree_line_mid"),
    proto("trait_tree_exclusive"),
    proto("active_advisor_trait"),
    proto("active_trait"),
    proto("exile_trait"),
];

/// Every (file, manifest) the cluster owns — the input to the accounting test.
pub const ALL_MANIFESTS: &[(&str, &[RootEntry])] = &[
    ("officer_corps.json", OFFICER_CORPS),
    ("unit_leader.json", UNIT_LEADER),
    ("navy_leader.json", NAVY_LEADER),
    ("leader_groups.json", LEADER_GROUPS),
    ("aces.json", ACES),
    ("army_trait_tree.json", ARMY_TRAIT_TREE),
    ("navy_trait_tree.json", NAVY_TRAIT_TREE),
    ("trait_tree_common.json", TRAIT_TREE_COMMON),
];
