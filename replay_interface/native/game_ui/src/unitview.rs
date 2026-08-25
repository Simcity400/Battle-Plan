
use crate::gui::{GuiElement, LayoutSet};
use replay_core::{
    DivisionUnitRow, EquipmentAmount, FleetUnitRow, Leaf, ShipCompositionRow, TaskForceKey,
    ViewStatus, WingUnitRow, WorldFrame,
};
use std::collections::BTreeSet;


pub const UNITVIEW_GUI: &str = "interface/unitview.gui";

pub const LAND_DETAIL_WINDOW: &str = "army_division_stats_view";
pub const NAVAL_DETAIL_WINDOW: &str = "ship_stats_window";
pub const AIR_WING_ENTRY_WINDOW: &str = "ship_stats_air_wing_entry";

pub const LAND_FIELD_SLOTS: &[&str] = &[
    "unit_title",
    "experience",
    "combat_width_text",
    "support_header_label",
];

pub const NAVAL_FIELD_SLOTS: &[&str] = &[
    "ship_title",
    "ship_name",
    "experience",
    "current_manpower",
    "dominance_value",
    "support_value",
    "fuel_consumption",
];

pub const AIR_FIELD_SLOTS: &[&str] = &["name", "count"];



#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelLayout {
    pub window: String,
    pub found: bool,
    pub descendant_count: usize,
    pub field_slots_present: Vec<String>,
    pub field_slots_missing: Vec<String>,
    pub runtime_grids: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitViewLayoutReport {
    pub land: PanelLayout,
    pub naval: PanelLayout,
    pub air: PanelLayout,
}

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

pub fn inspect_unitview_layout(layouts: &LayoutSet) -> UnitViewLayoutReport {
    UnitViewLayoutReport {
        land: inspect_panel(layouts, LAND_DETAIL_WINDOW, LAND_FIELD_SLOTS),
        naval: inspect_panel(layouts, NAVAL_DETAIL_WINDOW, NAVAL_FIELD_SLOTS),
        air: inspect_panel(layouts, AIR_WING_ENTRY_WINDOW, AIR_FIELD_SLOTS),
    }
}


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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LandUnitView {
    pub uid: String,
    pub owner: Option<String>,
    pub display_name: Option<String>,
    pub template_name: Option<String>,
    pub template_role: Option<String>,
    pub template_id: Option<i64>,
    pub strength: Option<f64>,
    pub organisation: Option<f64>,
    pub experience: Option<f64>,
    pub location_province: Option<u32>,
    pub moving: Option<bool>,
    pub in_combat: Option<bool>,
    pub equipment_available: bool,
    pub equipment: Vec<UnitEquipmentLine>,
    pub available: bool,
}

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
        experience: opt_f64(&row.experience),
        location_province: opt_u32(&row.location_province),
        moving: opt_bool(&row.moving),
        in_combat: opt_bool(&row.in_combat),
        equipment_available,
        equipment,
        available,
    }
}

pub fn land_unit_view_from_frame(frame: &WorldFrame, div_uid: &str) -> Option<LandUnitView> {
    let available = frame.division_units_status() == ViewStatus::Present;
    frame
        .division_units()
        .iter()
        .find(|d| d.div_uid == div_uid)
        .map(|row| land_unit_view_from_row(row, available))
}

pub fn land_unit_view_for_pov(
    frame: &WorldFrame,
    div_uid: &str,
    pov_tag: &str,
) -> Option<LandUnitView> {
    let row = frame
        .division_units()
        .iter()
        .find(|division| division.div_uid == div_uid)?;
    match frame.division_pov_exact_details(pov_tag, row)? {
        true => Some(land_unit_view_from_row(
            row,
            frame.division_units_status() == ViewStatus::Present,
        )),
        false => Some(LandUnitView {
            uid: row.div_uid.clone(),
            available: false,
            ..LandUnitView::default()
        }),
    }
}

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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavalUnitView {
    pub uid: String,
    pub fleet_uid: Option<String>,
    pub owner: Option<String>,
    pub display_name: Option<String>,
    pub mission: Option<String>,
    pub mission_state: Option<i64>,
    pub detached_activity: Option<String>,
    pub activity_key: Option<String>,
    pub active_combat_epochs_available: bool,
    pub active_combat_epochs: Vec<String>,
    pub combat_membership_authority: Option<String>,
    pub movement_path_available: bool,
    pub movement_path: Vec<u32>,
    pub movement_progress: Option<f64>,
    pub save_repair_mode: Option<i64>,
    pub save_repair_split: Option<bool>,
    pub save_repair_target_province: Option<u32>,
    pub save_repair_parent_uid: Option<String>,
    pub save_repair_child_uid: Option<String>,
    pub save_repair_last_mission: Option<i64>,
    pub save_detached_activity: Option<String>,
    pub save_retreat: Option<bool>,
    pub ship_count: Option<i64>,
    pub dominant_hull_class: Option<String>,
    pub location_province: Option<u32>,
    pub ships_available: bool,
    pub ships: Vec<NavalShipLine>,
    pub available: bool,
}

pub fn naval_unit_view_from_row(row: &FleetUnitRow, available: bool) -> NavalUnitView {
    let (ships_available, ships) = ship_lines(&row.ships);
    let (active_combat_epochs_available, active_combat_epochs) = match &row.active_combat_epochs {
        Leaf::Measured(epochs) => (true, epochs.clone()),
        Leaf::Absent | Leaf::Unobtained => (false, Vec::new()),
    };
    let (movement_path_available, movement_path) = match &row.movement_path {
        Leaf::Measured(path) => (true, path.clone()),
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
        movement_path_available,
        movement_path,
        movement_progress: opt_f64(&row.movement_progress),
        save_repair_mode: opt_i64(&row.save_repair_mode),
        save_repair_split: opt_bool(&row.save_repair_split),
        save_repair_target_province: opt_u32(&row.save_repair_target_province),
        save_repair_parent_uid: opt_string(&row.save_repair_parent_uid),
        save_repair_child_uid: opt_string(&row.save_repair_child_uid),
        save_repair_last_mission: opt_i64(&row.save_repair_last_mission),
        save_detached_activity: opt_string(&row.save_detached_activity),
        save_retreat: opt_bool(&row.save_retreat),
        ship_count: opt_i64(&row.ship_count),
        dominant_hull_class: opt_string(&row.dominant_hull_class),
        location_province: opt_u32(&row.location_province),
        ships_available,
        ships,
        available,
    }
}

/// The task force with this whole identity, if the frame has it.
///
/// THE ENTRY POINT ANY CALLER WITH A KEY SHOULD USE. Two clausewitz registries
/// mint task forces and their numbers collide, so the number alone can name two
/// different fleets; the key names one (landing bounce #2).
pub fn naval_unit_view_from_frame_by_key(
    frame: &WorldFrame,
    key: TaskForceKey,
) -> Option<NavalUnitView> {
    let available = frame.fleets_status() == ViewStatus::Present;
    frame
        .fleets()
        .iter()
        .find(|f| f.task_force_key() == Some(key))
        .map(|row| naval_unit_view_from_row(row, available))
}

/// The task force the game would call by this number.
///
/// **FAILS CLOSED ON EITHER KIND OF DOUBT.** `task_force_uid` is the GAME's
/// number -- what the outliner and the unit panel render -- and resolving a
/// panel from it can go wrong two ways, so both refuse:
///
/// * *Ambiguous.* Both clausewitz registries can mint one number, so a frame
///   may hold two rows answering to one string. Returning the first would open
///   whichever the census happened to build first, which is a silently wrong
///   panel.
/// * *Keyless.* A row whose `task_force_key()` is `None` has no proven number.
///   The world-frame builder never makes one -- `build_fleets` skips a row
///   whose `id` or `id_space` is unusable -- but `FleetUnitRow` is public and
///   constructible, so the state is representable and a uid match on such a row
///   proves nothing.
///
/// A caller holding an identity should call
/// [`naval_unit_view_from_frame_by_key`], which has neither problem.
pub fn naval_unit_view_from_frame(
    frame: &WorldFrame,
    task_force_uid: &str,
) -> Option<NavalUnitView> {
    let available = frame.fleets_status() == ViewStatus::Present;
    let mut matches = frame
        .fleets()
        .iter()
        .filter(|f| f.task_force_uid == task_force_uid);
    let row = matches.next()?;
    if matches.next().is_some() || row.task_force_key().is_none() {
        return None;
    }
    Some(naval_unit_view_from_row(row, available))
}

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
    pub plane_definition: Option<String>,
    pub experience: Option<f64>,
    pub manpower: Option<i64>,
    pub available: bool,
}

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
            raw_division_id: 9,
            raw_division_type: Leaf::Measured(1),
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
            max_organisation: Leaf::Absent,
            experience: Leaf::Absent,
            current_supply: Leaf::Absent,
            max_supply: Leaf::Absent,
            counter_supply_status: Leaf::Absent,
            supply_ratio: Leaf::Absent,
            dig_in: Leaf::Absent,
            dig_in_cap: Leaf::Absent,
            counter_training_visible: Leaf::Absent,
            counter_dig_in_frame: Leaf::Absent,
            counter_invalid_orders_visible: Leaf::Absent,
            army_id: Leaf::Absent,
            primary_order: Leaf::Absent,
            moving: Leaf::Absent,
            move_target_province: Leaf::Absent,
            move_progress: Leaf::Absent,
            in_combat: Leaf::Absent,
            attrition: Leaf::Absent,
            equipment: Leaf::Absent,
        };
        let view = land_unit_view_from_row(&row, true);
        assert_eq!(view.uid, "9:1");
        assert_eq!(view.owner, None);
        assert_eq!(view.strength, None);
        assert_eq!(view.organisation, None);
        assert_eq!(view.experience, None);
        assert_eq!(view.template_name, None);
        assert_eq!(view.display_name, None);
        assert!(!view.equipment_available);
        assert!(view.equipment.is_empty());
    }

    #[test]
    fn measured_land_row_carries_raw_values() {
        let row = DivisionUnitRow {
            raw_division_id: 101,
            raw_division_type: Leaf::Measured(1),
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
            max_organisation: Leaf::Absent,
            experience: Leaf::Measured(0.42),
            current_supply: Leaf::Absent,
            max_supply: Leaf::Absent,
            counter_supply_status: Leaf::Absent,
            supply_ratio: Leaf::Absent,
            dig_in: Leaf::Absent,
            dig_in_cap: Leaf::Absent,
            counter_training_visible: Leaf::Absent,
            counter_dig_in_frame: Leaf::Absent,
            counter_invalid_orders_visible: Leaf::Absent,
            army_id: Leaf::Absent,
            primary_order: Leaf::Absent,
            moving: Leaf::Measured(true),
            move_target_province: Leaf::Measured(2000),
            move_progress: Leaf::Measured(0.5),
            in_combat: Leaf::Measured(false),
            attrition: Leaf::Absent,
            equipment: Leaf::Measured(Vec::new()),
        };
        let view = land_unit_view_from_row(&row, true);
        assert_eq!(view.owner.as_deref(), Some("GER"));
        assert_eq!(view.template_name.as_deref(), Some("Fixture Infantry"));
        assert_eq!(view.template_role.as_deref(), Some("infantry"));
        assert_eq!(view.strength, Some(0.95));
        assert_eq!(view.organisation, Some(42.0));
        assert_eq!(view.experience, Some(0.42));
        assert_eq!(view.location_province, Some(1000));
        assert_eq!(view.moving, Some(true));
        assert_eq!(view.in_combat, Some(false));
        assert!(view.equipment_available);
        assert!(view.equipment.is_empty());
    }

    #[test]
    fn naval_activity_keeps_detached_state_separate_from_member_epoch_branch() {
        let row = FleetUnitRow {
            task_force_uid: "201".to_string(),
            fleet_uid: Leaf::Measured("200".to_string()),
            raw_task_force_id: Leaf::Measured(201),
            raw_task_force_id_space: Leaf::Measured(61),
            raw_fleet_id: Leaf::Measured(200),
            owner: Leaf::Measured("GER".to_string()),
            location_province: Leaf::Measured(6434),
            movement_path: Leaf::Measured(vec![6434, 5088]),
            movement_progress: Leaf::Measured(0.5),
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
            save_repair_mode: Leaf::Measured(3),
            save_repair_split: Leaf::Measured(false),
            save_repair_target_province: Leaf::Measured(5088),
            save_repair_parent_uid: Leaf::Measured("202:61".to_string()),
            save_repair_child_uid: Leaf::Absent,
            save_repair_last_mission: Leaf::Measured(2),
            save_detached_activity: Leaf::Measured("refitting".to_string()),
            save_retreat: Leaf::Measured(true),
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
        assert!(view.movement_path_available);
        assert_eq!(view.movement_path, vec![6434, 5088]);
        assert_eq!(view.movement_progress, Some(0.5));
        assert_eq!(view.save_repair_mode, Some(3));
        assert_eq!(view.save_repair_target_province, Some(5088));
        assert_eq!(view.save_retreat, Some(true));
    }

    #[test]
    fn air_recording_absent_fields_are_none() {
        let row = WingUnitRow {
            wing_uid: "800:1".to_string(),
            raw_wing_id: Leaf::Measured(800),
            owner: Leaf::Measured("ENG".to_string()),
            name: Leaf::Measured("Fighter Wing".to_string()),
            plane_count: Leaf::Measured(100),
            experience: Leaf::Absent,
            manpower: Leaf::Absent,
            mission_kind: Leaf::Measured("air_superiority".to_string()),
            mission_mask: Leaf::Measured(1),
            strategic_region_id: Leaf::Measured(301),
            air_base: Leaf::Measured("3000:1".to_string()),
            air_base_province: Leaf::Measured(2500),
            state_id: Leaf::Measured(30),
            plane_definition: Leaf::Absent,
            effective_planes: Leaf::Measured(90),
            equipment: Leaf::Absent,
            exact_equipment: Leaf::Absent,
        };
        let view = air_wing_view_from_row(&row, true);
        assert_eq!(view.display_name.as_deref(), Some("Fighter Wing"));
        assert_eq!(view.plane_count, Some(100));
        assert_eq!(view.effective_planes, Some(90));
        assert_eq!(view.mission_kind.as_deref(), Some("air_superiority"));
        assert_eq!(view.experience, None);
        assert_eq!(view.manpower, None);
        assert_eq!(view.plane_definition, None);
    }
}
