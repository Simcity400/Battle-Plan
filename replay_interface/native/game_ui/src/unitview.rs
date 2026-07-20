//! U1 — selected-unit view panel (`interface/unitview.gui`) layout gate +
//! binding (REPLAY_PRODUCTION_PLAN §6a / UI_COVERAGE_MATRIX §2 row
//! "Selected-unit panel").
//!
//! This module lays the layout for the selected-unit detail panels the game
//! shows when a division / task-force / air-wing is selected, and binds
//! `replay_core`'s READ-ONLY unit views onto their displayed field slots. It is
//! the sibling of the D3 outliner seam in [`crate::binding`]: it parses no
//! recording bytes — every value comes through a typed `WorldFrame` view
//! (Constraint 2) — and fabricates nothing: a leaf that is `Absent`/`Unobtained`
//! (or a view whose status is not `Present`) renders the fail-closed absent form
//! (`None` / empty list), never a default like `0` (Constraint 3).
//!
//! ## Two label classes
//!
//! * **Layout slots are FILE.** The panel windows and their text/icon slot
//!   *names* are cited from `interface/unitview.gui` (the `LAND_DETAIL_WINDOW` /
//!   `NAVAL_DETAIL_WINDOW` / `AIR_WING_ENTRY_WINDOW` consts and the
//!   `*_FIELD_SLOTS` lists) and gated by [`inspect_unitview_layout`].
//! * **Field *formatting* is UNKNOWN → rendered raw.** the game's unit-panel number
//!   formatting (strength as a percent bar, org rounding, thousands grouping) is
//!   engine-computed and not cited from any shipped file in this decoupled
//!   context. Per Constraint 3 every scalar is carried as its raw measured value
//!   (`Option<f64>` / `Option<i64>`), never a fabricated percent/grouping. The
//!   caller renders it raw until a shipped-file citation for a field's format is
//!   found. See `LANE_REPORT.md` for the per-field FILE/UNKNOWN/VIEW-GAP table.
//!
//! ## VIEW GAPS (data the read-only view does not surface)
//!
//! The panel shows a per-unit *custom display name* (division / task-force
//! name) and the land panel shows an `experience` slot, but `DivisionUnitRow`
//! and `FleetUnitRow` expose no such field, so `display_name` (land + naval) and
//! land experience are fail-closed `None` with a filed view gap. This lane is
//! READ-ONLY on `replay_core`; closing these is a `replay_core` change, not a
//! binding change. See `LANE_REPORT.md`.

use crate::gui::{GuiElement, LayoutSet};
use replay_core::{
    DivisionUnitRow, EquipmentAmount, FleetUnitRow, Leaf, ShipCompositionRow, ViewStatus,
    WingUnitRow, WorldFrame,
};
use std::collections::BTreeSet;

// ===========================================================================
// FILE citations — panel windows + field slots (interface/unitview.gui)
// ===========================================================================

/// The shipped source file every window/slot name below is cited from.
pub const UNITVIEW_GUI: &str = "interface/unitview.gui";

/// Selected land-division detail window (FILE: `unitview.gui`).
pub const LAND_DETAIL_WINDOW: &str = "army_division_stats_view";
/// Selected ship / task-force detail window (FILE: `unitview.gui`).
pub const NAVAL_DETAIL_WINDOW: &str = "ship_stats_window";
/// Selected / listed air-wing row window (FILE: `unitview.gui`).
pub const AIR_WING_ENTRY_WINDOW: &str = "ship_stats_air_wing_entry";

/// FILE text-slot element names inside [`LAND_DETAIL_WINDOW`] the land binding
/// targets. `unit_title`/`experience` are present in the layout but map to a
/// VIEW GAP (see module header) — they are gated for layout presence here and
/// documented as unbound in `LANE_REPORT.md`.
pub const LAND_FIELD_SLOTS: &[&str] = &[
    "unit_title",
    "experience",
    "combat_width_text",
    "support_header_label",
];

/// FILE text-slot element names inside [`NAVAL_DETAIL_WINDOW`] the naval binding
/// targets or documents.
pub const NAVAL_FIELD_SLOTS: &[&str] = &[
    "ship_title",
    "ship_name",
    "experience",
    "current_manpower",
    "dominance_value",
    "support_value",
    "fuel_consumption",
];

/// FILE text-slot element names inside [`AIR_WING_ENTRY_WINDOW`] the air binding
/// targets.
pub const AIR_FIELD_SLOTS: &[&str] = &["name", "count"];

// Runtime-populated list/grid surfaces (documented skips) share the single
// runtime-populated-surface law in [`crate::major_views::is_runtime_populated`];
// their contents are filled per-frame by the binding, not laid out statically.

// ===========================================================================
// Layout gate — parse unitview.gui and verify each detail panel's slots
// ===========================================================================

/// The parsed layout of one selected-unit detail panel: whether its window was
/// found, which FILE field slots are present/missing, and the runtime-populated
/// grid/list surfaces inside it (documented skips).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelLayout {
    /// The window element name (one of the `*_DETAIL_WINDOW` / entry consts).
    pub window: String,
    /// `true` when the window element was found in the parsed `unitview.gui`.
    pub found: bool,
    /// Total descendant element count under the window (inclusive of nested).
    pub descendant_count: usize,
    /// FILE field slots (the `*_FIELD_SLOTS` list) present in the subtree.
    pub field_slots_present: Vec<String>,
    /// FILE field slots expected but absent (should be empty for a solid gate).
    pub field_slots_missing: Vec<String>,
    /// Runtime-populated grid/list surfaces under the window (documented skips).
    pub runtime_grids: Vec<String>,
}

/// The full selected-unit layout report: land + naval + air detail panels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitViewLayoutReport {
    pub land: PanelLayout,
    pub naval: PanelLayout,
    pub air: PanelLayout,
}

/// Recursively collect every descendant element name and count under `element`
/// (inclusive of `element` itself), plus the names of runtime grid surfaces.
fn walk_subtree(
    element: &GuiElement,
    names: &mut BTreeSet<String>,
    runtime_grids: &mut BTreeSet<String>,
    count: &mut usize,
) {
    *count += 1;
    names.insert(element.name.clone());
    if crate::major_views::is_runtime_populated(&element.element_type) {
        runtime_grids.insert(element.name.clone());
    }
    for child in &element.children {
        walk_subtree(child, names, runtime_grids, count);
    }
}

/// Gate one panel window: locate it in `layouts`, walk its subtree, and classify
/// the expected FILE field slots as present/missing. Fail-closed: a window not
/// found yields `found: false` with every slot listed missing.
fn inspect_panel(layouts: &LayoutSet, window: &str, expected_slots: &[&str]) -> PanelLayout {
    let Some(element) = layouts.find_by_name(window).into_iter().next() else {
        return PanelLayout {
            window: window.to_string(),
            found: false,
            descendant_count: 0,
            field_slots_present: Vec::new(),
            field_slots_missing: expected_slots.iter().map(|s| s.to_string()).collect(),
            runtime_grids: Vec::new(),
        };
    };
    let mut names = BTreeSet::new();
    let mut runtime_grids = BTreeSet::new();
    let mut count = 0usize;
    walk_subtree(element, &mut names, &mut runtime_grids, &mut count);

    let mut present = Vec::new();
    let mut missing = Vec::new();
    for &slot in expected_slots {
        if names.contains(slot) {
            present.push(slot.to_string());
        } else {
            missing.push(slot.to_string());
        }
    }
    PanelLayout {
        window: window.to_string(),
        found: true,
        descendant_count: count,
        field_slots_present: present,
        field_slots_missing: missing,
        runtime_grids: runtime_grids.into_iter().collect(),
    }
}

/// Parse `interface/unitview.gui` (already loaded into `layouts`) and gate the
/// three selected-unit detail panels' layouts. The caller loads the file via
/// [`LayoutSet::parse_file`] using the `GameFileResolver`/mod_resolver root.
pub fn inspect_unitview_layout(layouts: &LayoutSet) -> UnitViewLayoutReport {
    UnitViewLayoutReport {
        land: inspect_panel(layouts, LAND_DETAIL_WINDOW, LAND_FIELD_SLOTS),
        naval: inspect_panel(layouts, NAVAL_DETAIL_WINDOW, NAVAL_FIELD_SLOTS),
        air: inspect_panel(layouts, AIR_WING_ENTRY_WINDOW, AIR_FIELD_SLOTS),
    }
}

// ===========================================================================
// Fail-closed leaf → Option helpers (typed; never a fabricated default)
// ===========================================================================

fn opt_string(leaf: &Leaf<String>) -> Option<String> {
    leaf.measured().cloned()
}
fn opt_f64(leaf: &Leaf<f64>) -> Option<f64> {
    leaf.measured().copied()
}
fn opt_i64(leaf: &Leaf<i64>) -> Option<i64> {
    leaf.measured().copied()
}
fn opt_u32(leaf: &Leaf<u32>) -> Option<u32> {
    leaf.measured().copied()
}
fn opt_bool(leaf: &Leaf<bool>) -> Option<bool> {
    leaf.measured().copied()
}

// ===========================================================================
// Binding — replay_core unit views → panel field bindings
// ===========================================================================

/// One equipment-composition line (division / air-wing), fail-closed per field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitEquipmentLine {
    pub archetype: Option<String>,
    pub amount: Option<f64>,
}

fn equipment_lines(leaf: &Leaf<Vec<EquipmentAmount>>) -> (bool, Vec<UnitEquipmentLine>) {
    match leaf {
        Leaf::Measured(items) => (
            true,
            items
                .iter()
                .map(|e| UnitEquipmentLine {
                    archetype: opt_string(&e.archetype),
                    amount: opt_f64(&e.amount),
                })
                .collect(),
        ),
        Leaf::Absent | Leaf::Unobtained => (false, Vec::new()),
    }
}

/// Selected land-division panel binding (from [`DivisionUnitRow`]).
///
/// `display_name` is a VIEW GAP (`DivisionUnitRow` surfaces no custom name) and
/// stays `None`; `uid` is the raw stable identity `div_uid` (UNKNOWN formatting,
/// carried raw, same precedent as the D3 outliner row text).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LandUnitView {
    pub uid: String,
    pub owner: Option<String>,
    /// VIEW GAP: no division custom-name field in the read-only view → `None`.
    pub display_name: Option<String>,
    pub template_name: Option<String>,
    pub template_role: Option<String>,
    pub template_id: Option<i64>,
    pub strength: Option<f64>,
    pub organisation: Option<f64>,
    pub location_province: Option<u32>,
    pub moving: Option<bool>,
    pub in_combat: Option<bool>,
    /// `true` when the `equipment` leaf was `Measured` (even if empty).
    pub equipment_available: bool,
    pub equipment: Vec<UnitEquipmentLine>,
    /// `true` when the source `division_units` view status was `Present`.
    pub available: bool,
}

/// Bind one [`DivisionUnitRow`] to a [`LandUnitView`], fail-closed per field.
pub fn land_unit_view_from_row(row: &DivisionUnitRow, available: bool) -> LandUnitView {
    let (equipment_available, equipment) = equipment_lines(&row.equipment);
    LandUnitView {
        uid: row.div_uid.clone(),
        owner: opt_string(&row.owner),
        display_name: None,
        template_name: opt_string(&row.template_name),
        template_role: opt_string(&row.template_role),
        template_id: opt_i64(&row.template_id),
        strength: opt_f64(&row.strength),
        organisation: opt_f64(&row.organisation),
        location_province: opt_u32(&row.location_province),
        moving: opt_bool(&row.moving),
        in_combat: opt_bool(&row.in_combat),
        equipment_available,
        equipment,
        available,
    }
}

/// Select the land division `div_uid` at one world frame and bind its panel.
/// Fail-closed: returns `None` when no division with that UID is present this
/// frame (never a fabricated empty unit).
pub fn land_unit_view_from_frame(frame: &WorldFrame, div_uid: &str) -> Option<LandUnitView> {
    let available = frame.division_units_status() == ViewStatus::Present;
    frame
        .division_units()
        .iter()
        .find(|d| d.div_uid == div_uid)
        .map(|row| land_unit_view_from_row(row, available))
}

/// One ship line inside a selected task force (from [`ShipCompositionRow`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavalShipLine {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub definition: Option<String>,
    pub strength: Option<f64>,
    pub organisation: Option<f64>,
}

fn ship_lines(leaf: &Leaf<Vec<ShipCompositionRow>>) -> (bool, Vec<NavalShipLine>) {
    match leaf {
        Leaf::Measured(items) => (
            true,
            items
                .iter()
                .map(|s| NavalShipLine {
                    uid: opt_string(&s.ship_uid),
                    name: opt_string(&s.name),
                    definition: opt_string(&s.definition),
                    strength: opt_f64(&s.strength),
                    organisation: opt_f64(&s.organisation),
                })
                .collect(),
        ),
        Leaf::Absent | Leaf::Unobtained => (false, Vec::new()),
    }
}

/// Selected naval task-force panel binding (from [`FleetUnitRow`]).
///
/// `display_name` is a VIEW GAP (`FleetUnitRow` surfaces no task-force name) and
/// stays `None`; `uid` is the raw `task_force_uid`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavalUnitView {
    pub uid: String,
    pub fleet_uid: Option<String>,
    pub owner: Option<String>,
    /// VIEW GAP: no task-force custom-name field in the read-only view → `None`.
    pub display_name: Option<String>,
    pub mission: Option<String>,
    pub mission_state: Option<i64>,
    /// Exact persisted D56 state name; intentionally separate from the final
    /// engine activity label.
    pub detached_activity: Option<String>,
    /// Exact localization key for a mechanically closed activity branch.
    pub activity_key: Option<String>,
    /// `true` only when same-hour D49/D50 member-epoch authority was present.
    pub active_combat_epochs_available: bool,
    pub active_combat_epochs: Vec<String>,
    pub combat_membership_authority: Option<String>,
    pub ship_count: Option<i64>,
    pub dominant_hull_class: Option<String>,
    pub location_province: Option<u32>,
    /// `true` when the `ships` composition leaf was `Measured`.
    pub ships_available: bool,
    pub ships: Vec<NavalShipLine>,
    /// `true` when the source `fleets` view status was `Present`.
    pub available: bool,
}

/// Bind one [`FleetUnitRow`] to a [`NavalUnitView`], fail-closed per field.
pub fn naval_unit_view_from_row(row: &FleetUnitRow, available: bool) -> NavalUnitView {
    let (ships_available, ships) = ship_lines(&row.ships);
    let (active_combat_epochs_available, active_combat_epochs) = match &row.active_combat_epochs {
        Leaf::Measured(epochs) => (true, epochs.clone()),
        Leaf::Absent | Leaf::Unobtained => (false, Vec::new()),
    };
    NavalUnitView {
        uid: row.task_force_uid.clone(),
        fleet_uid: opt_string(&row.fleet_uid),
        owner: opt_string(&row.owner),
        display_name: None,
        mission: opt_string(&row.mission),
        mission_state: opt_i64(&row.mission_state),
        detached_activity: opt_string(&row.detached_activity),
        activity_key: opt_string(&row.activity_key),
        active_combat_epochs_available,
        active_combat_epochs,
        combat_membership_authority: opt_string(&row.combat_membership_authority),
        ship_count: opt_i64(&row.ship_count),
        dominant_hull_class: opt_string(&row.dominant_hull_class),
        location_province: opt_u32(&row.location_province),
        ships_available,
        ships,
        available,
    }
}

/// Select the task force `task_force_uid` at one world frame and bind its panel.
/// Fail-closed `None` when no task force with that UID is present this frame.
pub fn naval_unit_view_from_frame(
    frame: &WorldFrame,
    task_force_uid: &str,
) -> Option<NavalUnitView> {
    let available = frame.fleets_status() == ViewStatus::Present;
    frame
        .fleets()
        .iter()
        .find(|f| f.task_force_uid == task_force_uid)
        .map(|row| naval_unit_view_from_row(row, available))
}

/// Selected air-wing panel binding (from [`WingUnitRow`]).
///
/// Unlike land/naval, the air view DOES surface a `name`, so `display_name` is
/// bound. `plane_definition` / `experience` / `manpower` are commonly
/// recording-Absent → fail-closed `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AirWingView {
    pub uid: String,
    pub owner: Option<String>,
    pub display_name: Option<String>,
    pub plane_count: Option<i64>,
    pub effective_planes: Option<i64>,
    pub mission_kind: Option<String>,
    pub strategic_region_id: Option<u32>,
    pub air_base: Option<String>,
    pub state_id: Option<u32>,
    /// Recording-Absent in the fixtures → `None`.
    pub plane_definition: Option<String>,
    /// Recording-Absent in the fixtures → `None`.
    pub experience: Option<f64>,
    /// Recording-Absent in the fixtures → `None`.
    pub manpower: Option<i64>,
    /// `true` when the source `wings` view status was `Present`.
    pub available: bool,
}

/// Bind one [`WingUnitRow`] to an [`AirWingView`], fail-closed per field.
pub fn air_wing_view_from_row(row: &WingUnitRow, available: bool) -> AirWingView {
    AirWingView {
        uid: row.wing_uid.clone(),
        owner: opt_string(&row.owner),
        display_name: opt_string(&row.name),
        plane_count: opt_i64(&row.plane_count),
        effective_planes: opt_i64(&row.effective_planes),
        mission_kind: opt_string(&row.mission_kind),
        strategic_region_id: opt_u32(&row.strategic_region_id),
        air_base: opt_string(&row.air_base),
        state_id: opt_u32(&row.state_id),
        plane_definition: opt_string(&row.plane_definition),
        experience: opt_f64(&row.experience),
        manpower: opt_i64(&row.manpower),
        available,
    }
}

/// Select the air wing `wing_uid` at one world frame and bind its panel.
/// Fail-closed `None` when no wing with that UID is present this frame.
pub fn air_wing_view_from_frame(frame: &WorldFrame, wing_uid: &str) -> Option<AirWingView> {
    let available = frame.wings_status() == ViewStatus::Present;
    frame
        .wings()
        .iter()
        .find(|w| w.wing_uid == wing_uid)
        .map(|row| air_wing_view_from_row(row, available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_land_row_binds_fail_closed_none() {
        let row = DivisionUnitRow {
            div_uid: "9:1".to_string(),
            owner: Leaf::Absent,
            location_province: Leaf::Absent,
            state_id: Leaf::Absent,
            template_id: Leaf::Absent,
            template_type: Leaf::Absent,
            template_name: Leaf::Absent,
            template_role: Leaf::Absent,
            strength: Leaf::Absent,
            organisation: Leaf::Absent,
            moving: Leaf::Absent,
            move_target_province: Leaf::Absent,
            move_progress: Leaf::Absent,
            in_combat: Leaf::Absent,
            equipment: Leaf::Absent,
        };
        let view = land_unit_view_from_row(&row, true);
        assert_eq!(view.uid, "9:1");
        assert_eq!(view.owner, None);
        assert_eq!(view.strength, None);
        assert_eq!(view.organisation, None);
        assert_eq!(view.template_name, None);
        assert_eq!(view.display_name, None); // VIEW GAP, always None
        assert!(!view.equipment_available);
        assert!(view.equipment.is_empty());
    }

    #[test]
    fn measured_land_row_carries_raw_values() {
        let row = DivisionUnitRow {
            div_uid: "101:1".to_string(),
            owner: Leaf::Measured("GER".to_string()),
            location_province: Leaf::Measured(1000),
            state_id: Leaf::Measured(10),
            template_id: Leaf::Measured(1),
            template_type: Leaf::Measured(1),
            template_name: Leaf::Measured("Fixture Infantry".to_string()),
            template_role: Leaf::Measured("infantry".to_string()),
            strength: Leaf::Measured(0.95),
            organisation: Leaf::Measured(42.0),
            moving: Leaf::Measured(true),
            move_target_province: Leaf::Measured(2000),
            move_progress: Leaf::Measured(0.5),
            in_combat: Leaf::Measured(false),
            equipment: Leaf::Measured(Vec::new()),
        };
        let view = land_unit_view_from_row(&row, true);
        assert_eq!(view.owner.as_deref(), Some("GER"));
        assert_eq!(view.template_name.as_deref(), Some("Fixture Infantry"));
        assert_eq!(view.template_role.as_deref(), Some("infantry"));
        assert_eq!(view.strength, Some(0.95));
        assert_eq!(view.organisation, Some(42.0));
        assert_eq!(view.location_province, Some(1000));
        assert_eq!(view.moving, Some(true));
        assert_eq!(view.in_combat, Some(false));
        // equipment leaf is Measured-but-empty → available, zero lines.
        assert!(view.equipment_available);
        assert!(view.equipment.is_empty());
    }

    #[test]
    fn naval_activity_keeps_detached_state_separate_from_member_epoch_branch() {
        let row = FleetUnitRow {
            task_force_uid: "201".to_string(),
            fleet_uid: Leaf::Measured("200".to_string()),
            owner: Leaf::Measured("GER".to_string()),
            location_province: Leaf::Measured(6434),
            mission: Leaf::Measured("STRIKE_FORCE".to_string()),
            mission_enum: Leaf::Measured(2),
            mission_state: Leaf::Measured(3),
            detached_activity: Leaf::Measured("refitting".to_string()),
            activity_key: Leaf::Measured(
                "TASK_FORCE_ACTIVITY_DESC_IN_COMBAT".to_string(),
            ),
            active_combat_epochs: Leaf::Measured(vec!["240580:7:attacker".to_string()]),
            combat_membership_authority: Leaf::Measured(
                "d49_d50_member_epoch".to_string(),
            ),
            home_base_province: Leaf::Measured(4021),
            repair_base_province: Leaf::Measured(5088),
            sunk_convoys: Leaf::Measured(0),
            ship_count: Leaf::Measured(12),
            dominant_hull_class: Leaf::Measured("ship_hull_light_1".to_string()),
            ships: Leaf::Measured(Vec::new()),
        };
        let view = naval_unit_view_from_row(&row, true);
        assert_eq!(view.mission_state, Some(3));
        assert_eq!(view.detached_activity.as_deref(), Some("refitting"));
        assert_eq!(
            view.activity_key.as_deref(),
            Some("TASK_FORCE_ACTIVITY_DESC_IN_COMBAT")
        );
        assert!(view.active_combat_epochs_available);
        assert_eq!(
            view.active_combat_epochs,
            vec!["240580:7:attacker".to_string()]
        );
        assert_eq!(
            view.combat_membership_authority.as_deref(),
            Some("d49_d50_member_epoch")
        );
    }

    #[test]
    fn air_recording_absent_fields_are_none() {
        let row = WingUnitRow {
            wing_uid: "800:1".to_string(),
            owner: Leaf::Measured("ENG".to_string()),
            name: Leaf::Measured("Fighter Wing".to_string()),
            plane_count: Leaf::Measured(100),
            experience: Leaf::Absent,
            manpower: Leaf::Absent,
            mission_kind: Leaf::Measured("air_superiority".to_string()),
            strategic_region_id: Leaf::Measured(301),
            air_base: Leaf::Measured("3000:1".to_string()),
            state_id: Leaf::Measured(30),
            plane_definition: Leaf::Absent,
            effective_planes: Leaf::Measured(90),
            equipment: Leaf::Absent,
        };
        let view = air_wing_view_from_row(&row, true);
        assert_eq!(view.display_name.as_deref(), Some("Fighter Wing"));
        assert_eq!(view.plane_count, Some(100));
        assert_eq!(view.effective_planes, Some(90));
        assert_eq!(view.mission_kind.as_deref(), Some("air_superiority"));
        // Recording-Absent fields fail closed.
        assert_eq!(view.experience, None);
        assert_eq!(view.manpower, None);
        assert_eq!(view.plane_definition, None);
    }
}
