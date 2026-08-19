
use crate::design_catalog::{
    AdmittedDesignCatalog, DesignKind,
};
use replay_core::world::ViewStatus;
use replay_core::{
    DesignRecord, PeaceConferenceObservationExact,
    PeaceConferencePresentationObservationExact, PeaceParticipantRoleExact,
    SelectedDesignerFamilyExact, SelectedDesignerIdentityExactView, SelectedDesignerOwnerExact,
    StableDesignIdentityExact, WorldFrame,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DesignerViewKind {
    Division,
    Equipment,
    Tank,
    Plane,
    Ship,
    VariantUpgrade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesignerChromeLaw {
    pub view: DesignerViewKind,
    pub gui_file: &'static str,
    pub root_widget: &'static str,
}

pub const DESIGNER_CHROME: [DesignerChromeLaw; 6] = [
    DesignerChromeLaw {
        view: DesignerViewKind::Division,
        gui_file: "interface/divisiondesignerview.gui",
        root_widget: "countrydivisiondesignerview",
    },
    DesignerChromeLaw {
        view: DesignerViewKind::Equipment,
        gui_file: "interface/equipmentdesignerview.gui",
        root_widget: "countryequipmentdesignerview",
    },
    DesignerChromeLaw {
        view: DesignerViewKind::Tank,
        gui_file: "interface/tank_designer_view.gui",
        root_widget: "tank_designer_view",
    },
    DesignerChromeLaw {
        view: DesignerViewKind::Plane,
        gui_file: "interface/plane_designer_view.gui",
        root_widget: "plane_designer_view",
    },
    DesignerChromeLaw {
        view: DesignerViewKind::Ship,
        gui_file: "interface/equipmentdesignerview.gui",
        root_widget: "countryequipmentdesignerview",
    },
    DesignerChromeLaw {
        view: DesignerViewKind::VariantUpgrade,
        gui_file: "interface/equipmentupgradedesignerwindow.gui",
        root_widget: "equipment_upgrade_designer_window",
    },
];

pub fn designer_chrome(view: DesignerViewKind) -> &'static DesignerChromeLaw {
    DESIGNER_CHROME
        .iter()
        .find(|law| law.view == view)
        .expect("all DesignerViewKind values have frozen chrome")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayAtom {
    pub source_record_id: String,
    pub raw_value: String,
    pub rendered_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignerModuleSlot {
    pub source_record_id: String,
    pub ordinal: u32,
    pub slot_id: String,
    pub module_token: Option<String>,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignerStatRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub section_id: String,
    pub stat_id: String,
    pub value: DisplayAtom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignerUpgradeRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub upgrade_id: String,
    pub level: i64,
    pub value: DisplayAtom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivisionSlotRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub column: u32,
    pub row: u32,
    pub subunit_token: String,
    pub support: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadOnlyDesignerSnapshot {
    pub source_record_id: String,
    pub view: DesignerViewKind,
    pub owner: String,
    pub ordered_design_ids: Vec<String>,
    pub selected_design_id: String,
    pub name: DisplayAtom,
    pub archetype: String,
    pub icon_token: Option<String>,
    pub obsolete: bool,
    pub assigned_mio_id: Option<String>,
    pub module_slots: Vec<DesignerModuleSlot>,
    pub stat_rows: Vec<DesignerStatRow>,
    pub upgrade_rows: Vec<DesignerUpgradeRow>,
    pub division_slots: Vec<DivisionSlotRow>,
    pub cost_rows: Vec<DisplayAtom>,
    pub controls_interactive: bool,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeaceParticipantRole {
    Victor,
    Beneficiary,
    Defeated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeaceActionGroup {
    Primary,
    Stackable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeaceActionControlLaw {
    pub ordinal: u32,
    pub action_kind: &'static str,
    pub widget_id: &'static str,
    pub controller_offset: usize,
    pub group: PeaceActionGroup,
}

pub const PEACE_ACTION_CONTROLS: [PeaceActionControlLaw; 9] = [
    PeaceActionControlLaw {
        ordinal: 0,
        action_kind: "take_states",
        widget_id: "take_state_action",
        controller_offset: 0x5190,
        group: PeaceActionGroup::Primary,
    },
    PeaceActionControlLaw {
        ordinal: 1,
        action_kind: "liberate",
        widget_id: "liberate_action",
        controller_offset: 0x5198,
        group: PeaceActionGroup::Primary,
    },
    PeaceActionControlLaw {
        ordinal: 2,
        action_kind: "puppet",
        widget_id: "puppet_action",
        controller_offset: 0x51a0,
        group: PeaceActionGroup::Primary,
    },
    PeaceActionControlLaw {
        ordinal: 3,
        action_kind: "force_government",
        widget_id: "force_government_action",
        controller_offset: 0x51a8,
        group: PeaceActionGroup::Primary,
    },
    PeaceActionControlLaw {
        ordinal: 4,
        action_kind: "take_navy",
        widget_id: "take_navy_action",
        controller_offset: 0x51b0,
        group: PeaceActionGroup::Primary,
    },
    PeaceActionControlLaw {
        ordinal: 5,
        action_kind: "demilitarize_zone",
        widget_id: "demilitarize_zone_action",
        controller_offset: 0x51b8,
        group: PeaceActionGroup::Stackable,
    },
    PeaceActionControlLaw {
        ordinal: 6,
        action_kind: "resource_rights",
        widget_id: "resource_rights_action",
        controller_offset: 0x51c8,
        group: PeaceActionGroup::Stackable,
    },
    PeaceActionControlLaw {
        ordinal: 7,
        action_kind: "war_reparation",
        widget_id: "war_reparation_action",
        controller_offset: 0x51c0,
        group: PeaceActionGroup::Stackable,
    },
    PeaceActionControlLaw {
        ordinal: 8,
        action_kind: "dismantle_industry",
        widget_id: "dismantle_industry_action",
        controller_offset: 0x51d0,
        group: PeaceActionGroup::Stackable,
    },
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeaceParticipantRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub participant_id: String,
    pub country_tag: String,
    pub role: PeaceParticipantRole,
    pub score: DisplayAtom,
    pub score_ratio: DisplayAtom,
    pub allocation_ids: Vec<String>,
    pub result_ids: Vec<String>,
    pub bid_count: DisplayAtom,
    pub contested_bid_count: DisplayAtom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeaceAllocationRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub allocation_id: String,
    pub turn: u32,
    pub action_kind: String,
    pub actor_participant_id: String,
    pub beneficiary_country_tag: Option<String>,
    pub target_country_tag: Option<String>,
    pub target_state_ids: Vec<u32>,
    pub target_ship_ids: Vec<String>,
    pub cost: DisplayAtom,
    pub bid: DisplayAtom,
    pub status: DisplayAtom,
    pub popup_bid_count: DisplayAtom,
    pub contested_by_participant_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeaceAllocationGroupRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub group_id: String,
    pub allocation_ids: Vec<String>,
    pub flag_texture_path: Option<String>,
    pub target_display: DisplayAtom,
    pub available_state_count: DisplayAtom,
    pub wargoal_count: DisplayAtom,
    pub contested_state_count: DisplayAtom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeaceContestedPopupRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub popup_row_id: String,
    pub participant_id: String,
    pub allocation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeaceContestedPopup {
    pub source_record_id: String,
    pub ordinal: u32,
    pub popup_id: String,
    pub selected_allocation_id: String,
    pub rows: Vec<PeaceContestedPopupRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeaceResultRow {
    pub source_record_id: String,
    pub ordinal: u32,
    pub result_id: String,
    pub allocation_id: String,
    pub action_kind: String,
    pub actor_country_tag: String,
    pub beneficiary_country_tag: Option<String>,
    pub target_country_tag: Option<String>,
    pub target_state_ids: Vec<u32>,
    pub target_ship_ids: Vec<String>,
    pub outcome: DisplayAtom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeaceConferenceReference {
    pub object_type: u32,
    pub object_id: u32,
}

impl PeaceConferenceReference {
    pub fn valid(self) -> bool {
        self.object_type != 0 || self.object_id != 0
    }

    pub fn portable_id(self) -> String {
        format!("hoi4-ref:{}:{}", self.object_type, self.object_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivePeaceConferenceSnapshot {
    pub source_record_id: String,
    pub conference_reference: PeaceConferenceReference,
    pub conference_id: String,
    pub phase: String,
    pub turn: u32,
    pub remaining_score_turns: u32,
    pub current_participant_id: String,
    pub current_score: DisplayAtom,
    pub score_this_turn: DisplayAtom,
    pub available_state_ids: Vec<u32>,
    pub contested_state_ids: Vec<u32>,
    pub participants: Vec<PeaceParticipantRow>,
    pub allocations: Vec<PeaceAllocationRow>,
    pub allocation_groups: Vec<PeaceAllocationGroupRow>,
    pub contested_popups: Vec<PeaceContestedPopup>,
    pub results: Vec<PeaceResultRow>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeaceConferenceSnapshot {
    NoConferenceInProgress { source_record_id: String },
    Inactive { source_record_id: String },
    Active(ActivePeaceConferenceSnapshot),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xui08Rejection {
    MissingProvenance,
    Incomplete,
    InteractiveControl,
    CatalogOrderMismatch,
    UnknownSelectedDesign,
    DesignerKindMismatch,
    CatalogBindingMismatch,
    InvalidDisplayAtom,
    InvalidOrderedRows,
    DuplicateDisplayIdentity,
    InvalidDivisionComposition,
    InvalidPeaceIdentity,
    InvalidParticipantSet,
    InvalidAllocation,
    InvalidResult,
    UnsupportedPeaceAction,
    UnknownReference,
    ExactSelectionUnavailable,
    ExactSelectionFamilyMismatch,
    ExactSelectionOrderMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedDesignerSnapshot {
    snapshot: ReadOnlyDesignerSnapshot,
}
impl AdmittedDesignerSnapshot {
    pub fn view(&self) -> DesignerViewKind {
        self.snapshot.view
    }
    pub fn selected_design_id(&self) -> &str {
        &self.snapshot.selected_design_id
    }
    pub fn render_model(&self) -> &ReadOnlyDesignerSnapshot {
        &self.snapshot
    }
    pub fn chrome(&self) -> &'static DesignerChromeLaw {
        designer_chrome(self.snapshot.view)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedSelectedDesignerIdentity {
    source_record_identity: String,
    transition_sequence: u64,
    view: DesignerViewKind,
    owner: SelectedDesignerOwnerExact,
    ordered: Vec<DesignRecord>,
    selected_index: usize,
}

impl AdmittedSelectedDesignerIdentity {
    pub fn source_record_identity(&self) -> &str {
        &self.source_record_identity
    }
    pub fn transition_sequence(&self) -> u64 {
        self.transition_sequence
    }
    pub fn view(&self) -> DesignerViewKind {
        self.view
    }
    pub fn owner(&self) -> SelectedDesignerOwnerExact {
        self.owner
    }
    pub fn ordered(&self) -> &[DesignRecord] {
        &self.ordered
    }
    pub fn selected(&self) -> &DesignRecord {
        &self.ordered[self.selected_index]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedPeaceConference {
    snapshot: PeaceConferenceSnapshot,
}
impl AdmittedPeaceConference {
    pub fn active(&self) -> bool {
        matches!(self.snapshot, PeaceConferenceSnapshot::Active(_))
    }
    pub fn conference_id(&self) -> Option<&str> {
        match &self.snapshot {
            PeaceConferenceSnapshot::NoConferenceInProgress { .. }
            | PeaceConferenceSnapshot::Inactive { .. } => None,
            PeaceConferenceSnapshot::Active(value) => Some(&value.conference_id),
        }
    }
    pub fn conference_reference(&self) -> Option<PeaceConferenceReference> {
        match &self.snapshot {
            PeaceConferenceSnapshot::NoConferenceInProgress { .. }
            | PeaceConferenceSnapshot::Inactive { .. } => None,
            PeaceConferenceSnapshot::Active(value) => Some(value.conference_reference),
        }
    }
    pub fn render_model(&self) -> &PeaceConferenceSnapshot {
        &self.snapshot
    }
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}
fn valid_atom(value: &DisplayAtom) -> bool {
    nonempty(&value.source_record_id)
        && nonempty(&value.raw_value)
        && nonempty(&value.rendered_text)
}
fn valid_peace_atom(value: &DisplayAtom) -> bool {
    nonempty(&value.source_record_id)
}
fn exact_ordinals(values: impl Iterator<Item = u32>) -> bool {
    values
        .enumerate()
        .all(|(index, ordinal)| ordinal as usize == index)
}
fn expected_kind(view: DesignerViewKind) -> Option<DesignKind> {
    match view {
        DesignerViewKind::Division => Some(DesignKind::DivisionTemplate),
        DesignerViewKind::Tank => Some(DesignKind::TankVariant),
        DesignerViewKind::Plane => Some(DesignKind::AircraftVariant),
        DesignerViewKind::Ship => Some(DesignKind::ShipVariant),
        DesignerViewKind::Equipment | DesignerViewKind::VariantUpgrade => {
            Some(DesignKind::EquipmentVariant)
        }
    }
}

fn exact_view_kind(family: SelectedDesignerFamilyExact) -> DesignerViewKind {
    match family {
        SelectedDesignerFamilyExact::Division => DesignerViewKind::Division,
        SelectedDesignerFamilyExact::Equipment => DesignerViewKind::Equipment,
        SelectedDesignerFamilyExact::Tank => DesignerViewKind::Tank,
        SelectedDesignerFamilyExact::Plane => DesignerViewKind::Plane,
        SelectedDesignerFamilyExact::Ship => DesignerViewKind::Ship,
        SelectedDesignerFamilyExact::VariantUpgrade => DesignerViewKind::VariantUpgrade,
    }
}

fn portable_design_id(identity: StableDesignIdentityExact) -> String {
    format!("{}:{}", identity.stable_id, identity.object_type)
}

fn exact_family_accepts_kind(family: SelectedDesignerFamilyExact, kind: DesignKind) -> bool {
    match family {
        SelectedDesignerFamilyExact::Division => kind == DesignKind::DivisionTemplate,
        SelectedDesignerFamilyExact::Equipment => kind == DesignKind::EquipmentVariant,
        SelectedDesignerFamilyExact::Tank => kind == DesignKind::TankVariant,
        SelectedDesignerFamilyExact::Plane => kind == DesignKind::AircraftVariant,
        SelectedDesignerFamilyExact::Ship => kind == DesignKind::ShipVariant,
        SelectedDesignerFamilyExact::VariantUpgrade => matches!(
            kind,
            DesignKind::EquipmentVariant
                | DesignKind::TankVariant
                | DesignKind::AircraftVariant
                | DesignKind::ShipVariant
        ),
    }
}

pub fn admit_exact_selected_designer_identity(
    catalog: &AdmittedDesignCatalog,
    exact: &SelectedDesignerIdentityExactView,
) -> Result<AdmittedSelectedDesignerIdentity, Xui08Rejection> {
    if exact.status != ViewStatus::Present {
        return Err(Xui08Rejection::ExactSelectionUnavailable);
    }
    let selection = exact
        .selection
        .as_ref()
        .ok_or(Xui08Rejection::ExactSelectionUnavailable)?;
    let view = exact_view_kind(selection.family);
    let mut ordered = Vec::with_capacity(selection.ordered_designs.len());
    let mut selected_index = None;
    for (index, identity) in selection.ordered_designs.iter().enumerate() {
        let design_id = portable_design_id(StableDesignIdentityExact {
            object_type: identity.object_type,
            stable_id: identity.stable_id,
        });
        let row = catalog
            .get(&design_id)
            .ok_or(Xui08Rejection::ExactSelectionOrderMismatch)?;
        if !exact_family_accepts_kind(selection.family, row.kind) {
            return Err(Xui08Rejection::ExactSelectionFamilyMismatch);
        }
        if identity.object_type == selection.selected_design.object_type
            && identity.stable_id == selection.selected_design.stable_id
        {
            selected_index = Some(index);
        }
        ordered.push(row.clone());
    }
    let selected_index = selected_index.ok_or(Xui08Rejection::ExactSelectionOrderMismatch)?;
    Ok(AdmittedSelectedDesignerIdentity {
        source_record_identity: selection.source_record_identity.clone(),
        transition_sequence: selection.transition_sequence,
        view,
        owner: selection.owner,
        ordered,
        selected_index,
    })
}

fn unique_nonempty(mut values: impl Iterator<Item = String>) -> bool {
    let mut seen = BTreeSet::new();
    values.all(|value| nonempty(&value) && seen.insert(value))
}

pub fn admit_readonly_designer(
    catalog: &AdmittedDesignCatalog,
    snapshot: &ReadOnlyDesignerSnapshot,
) -> Result<AdmittedDesignerSnapshot, Xui08Rejection> {
    if !nonempty(&snapshot.source_record_id) || !nonempty(&snapshot.owner) {
        return Err(Xui08Rejection::MissingProvenance);
    }
    if !snapshot.complete {
        return Err(Xui08Rejection::Incomplete);
    }
    if snapshot.controls_interactive {
        return Err(Xui08Rejection::InteractiveControl);
    }
    let catalog_ids = catalog
        .ordered()
        .iter()
        .map(|row| row.design_id.as_str())
        .collect::<Vec<_>>();
    if snapshot
        .ordered_design_ids
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != catalog_ids
    {
        return Err(Xui08Rejection::CatalogOrderMismatch);
    }
    let selected = catalog
        .get(&snapshot.selected_design_id)
        .ok_or(Xui08Rejection::UnknownSelectedDesign)?;
    if expected_kind(snapshot.view) != Some(selected.kind) {
        return Err(Xui08Rejection::DesignerKindMismatch);
    }
    let selected_obsolete = match selected.lifecycle_state {
        Some(crate::design_catalog::DesignLifecycleState::Active) => false,
        Some(crate::design_catalog::DesignLifecycleState::Obsolete) => true,
        _ => return Err(Xui08Rejection::CatalogBindingMismatch),
    };
    if selected.owner.as_deref() != Some(snapshot.owner.as_str())
        || selected.name.as_deref() != Some(snapshot.name.raw_value.as_str())
        || selected.archetype.as_deref() != Some(snapshot.archetype.as_str())
        || selected_obsolete != snapshot.obsolete
    {
        return Err(Xui08Rejection::CatalogBindingMismatch);
    }
    if !valid_atom(&snapshot.name)
        || !nonempty(&snapshot.archetype)
        || snapshot.cost_rows.iter().any(|row| !valid_atom(row))
    {
        return Err(Xui08Rejection::InvalidDisplayAtom);
    }
    if !exact_ordinals(snapshot.module_slots.iter().map(|row| row.ordinal))
        || !exact_ordinals(snapshot.stat_rows.iter().map(|row| row.ordinal))
        || !exact_ordinals(snapshot.upgrade_rows.iter().map(|row| row.ordinal))
        || !exact_ordinals(snapshot.division_slots.iter().map(|row| row.ordinal))
        || snapshot.module_slots.iter().any(|row| {
            !nonempty(&row.source_record_id)
                || !nonempty(&row.slot_id)
                || row.module_token.as_deref().is_some_and(|v| !nonempty(v))
        })
        || snapshot.stat_rows.iter().any(|row| {
            !nonempty(&row.source_record_id)
                || !nonempty(&row.section_id)
                || !nonempty(&row.stat_id)
                || !valid_atom(&row.value)
        })
        || snapshot.upgrade_rows.iter().any(|row| {
            !nonempty(&row.source_record_id)
                || !nonempty(&row.upgrade_id)
                || row.level < 0
                || !valid_atom(&row.value)
        })
    {
        return Err(Xui08Rejection::InvalidOrderedRows);
    }
    let module_ids = snapshot.module_slots.iter().map(|row| row.slot_id.clone());
    let stat_ids = snapshot
        .stat_rows
        .iter()
        .map(|row| format!("{}\0{}", row.section_id, row.stat_id));
    let upgrade_ids = snapshot
        .upgrade_rows
        .iter()
        .map(|row| row.upgrade_id.clone());
    if !unique_nonempty(module_ids) || !unique_nonempty(stat_ids) || !unique_nonempty(upgrade_ids) {
        return Err(Xui08Rejection::DuplicateDisplayIdentity);
    }
    let displayed_modules = snapshot
        .module_slots
        .iter()
        .filter_map(|row| row.module_token.as_ref().map(|token| (&row.slot_id, token)))
        .collect::<Vec<_>>();
    if displayed_modules.len() != selected.modules.len()
        || displayed_modules
            .iter()
            .zip(&selected.modules)
            .any(|((slot, token), catalog)| {
                slot.as_str() != catalog.slot_id || token.as_str() != catalog.module_token
            })
        || snapshot.upgrade_rows.len() != selected.upgrades.len()
        || snapshot
            .upgrade_rows
            .iter()
            .zip(&selected.upgrades)
            .any(|(row, catalog)| {
                row.upgrade_id != catalog.upgrade_id || row.level != catalog.level
            })
        || snapshot.stat_rows.len() != selected.stats.len()
        || snapshot
            .stat_rows
            .iter()
            .zip(&selected.stats)
            .any(|(row, catalog)| {
                row.stat_id != catalog.stat_id
                    || row.value.raw_value.parse::<f64>().ok() != Some(catalog.value)
            })
    {
        return Err(Xui08Rejection::CatalogBindingMismatch);
    }
    let has_division = !snapshot.division_slots.is_empty();
    if (snapshot.view == DesignerViewKind::Division) != has_division
        || snapshot
            .division_slots
            .iter()
            .any(|row| !nonempty(&row.source_record_id) || !nonempty(&row.subunit_token))
    {
        return Err(Xui08Rejection::InvalidDivisionComposition);
    }
    let mut division_positions = BTreeSet::new();
    if snapshot
        .division_slots
        .iter()
        .any(|row| !division_positions.insert((row.support, row.column, row.row)))
    {
        return Err(Xui08Rejection::DuplicateDisplayIdentity);
    }
    if snapshot
        .icon_token
        .as_deref()
        .is_some_and(|value| !nonempty(value))
        || snapshot
            .assigned_mio_id
            .as_deref()
            .is_some_and(|value| !nonempty(value))
    {
        return Err(Xui08Rejection::InvalidDisplayAtom);
    }
    Ok(AdmittedDesignerSnapshot {
        snapshot: snapshot.clone(),
    })
}

pub fn admit_peace_conference(
    snapshot: &PeaceConferenceSnapshot,
) -> Result<AdmittedPeaceConference, Xui08Rejection> {
    let active = match snapshot {
        PeaceConferenceSnapshot::NoConferenceInProgress { source_record_id }
        | PeaceConferenceSnapshot::Inactive { source_record_id } => {
            if !nonempty(source_record_id) {
                return Err(Xui08Rejection::MissingProvenance);
            }
            return Ok(AdmittedPeaceConference {
                snapshot: snapshot.clone(),
            });
        }
        PeaceConferenceSnapshot::Active(value) => value,
    };
    if !active.complete {
        return Err(Xui08Rejection::Incomplete);
    }
    if !nonempty(&active.source_record_id)
        || !active.conference_reference.valid()
        || active.conference_id != active.conference_reference.portable_id()
        || !nonempty(&active.conference_id)
        || !nonempty(&active.phase)
        || (active.phase != "completed" && !nonempty(&active.current_participant_id))
        || !valid_peace_atom(&active.current_score)
        || !valid_peace_atom(&active.score_this_turn)
    {
        return Err(Xui08Rejection::InvalidPeaceIdentity);
    }
    if !exact_ordinals(active.participants.iter().map(|row| row.ordinal))
        || active.participants.is_empty()
        || active.participants.iter().any(|row| {
            !nonempty(&row.source_record_id)
                || !nonempty(&row.participant_id)
                || !nonempty(&row.country_tag)
                || !valid_peace_atom(&row.score)
                || !valid_peace_atom(&row.score_ratio)
                || !valid_peace_atom(&row.bid_count)
                || !valid_peace_atom(&row.contested_bid_count)
                || !unique_nonempty(row.allocation_ids.iter().cloned())
                || !unique_nonempty(row.result_ids.iter().cloned())
        })
    {
        return Err(Xui08Rejection::InvalidParticipantSet);
    }
    let participant_ids = active
        .participants
        .iter()
        .map(|row| row.participant_id.as_str())
        .collect::<BTreeSet<_>>();
    let participant_countries = active
        .participants
        .iter()
        .map(|row| row.country_tag.as_str())
        .collect::<BTreeSet<_>>();
    let current = active
        .participants
        .iter()
        .find(|row| row.participant_id == active.current_participant_id);
    if participant_ids.len() != active.participants.len()
        || participant_countries.len() != active.participants.len()
        || (active.phase != "completed"
            && (!participant_ids.contains(active.current_participant_id.as_str())
                || current.is_none_or(|row| row.role != PeaceParticipantRole::Victor)))
    {
        return Err(Xui08Rejection::InvalidParticipantSet);
    }
    if !unique_nonempty(active.available_state_ids.iter().map(u32::to_string))
        || !unique_nonempty(active.contested_state_ids.iter().map(u32::to_string))
        || active.available_state_ids.contains(&0)
        || active.contested_state_ids.contains(&0)
        || active
            .available_state_ids
            .iter()
            .any(|id| active.contested_state_ids.contains(id))
    {
        return Err(Xui08Rejection::InvalidAllocation);
    }
    let state_set = active
        .available_state_ids
        .iter()
        .chain(active.contested_state_ids.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut allocation_ids = BTreeSet::new();
    if !exact_ordinals(active.allocations.iter().map(|row| row.ordinal)) {
        return Err(Xui08Rejection::InvalidAllocation);
    }
    for row in &active.allocations {
        if !PEACE_ACTION_CONTROLS
            .iter()
            .any(|law| law.action_kind == row.action_kind)
        {
            return Err(Xui08Rejection::UnsupportedPeaceAction);
        }
        let valid_state_ids = unique_nonempty(row.target_state_ids.iter().map(u32::to_string))
            && !row.target_state_ids.contains(&0);
        let valid_ship_ids = unique_nonempty(row.target_ship_ids.iter().cloned());
        let valid_contesters = unique_nonempty(row.contested_by_participant_ids.iter().cloned());
        if !nonempty(&row.source_record_id)
            || !nonempty(&row.allocation_id)
            || !nonempty(&row.action_kind)
            || !participant_ids.contains(row.actor_participant_id.as_str())
            || !valid_peace_atom(&row.cost)
            || !valid_peace_atom(&row.bid)
            || !valid_peace_atom(&row.status)
            || !valid_peace_atom(&row.popup_bid_count)
            || row.turn > active.turn
            || !valid_state_ids
            || !valid_ship_ids
            || !valid_contesters
            || row
                .beneficiary_country_tag
                .as_deref()
                .is_some_and(|v| !nonempty(v))
            || row
                .target_country_tag
                .as_deref()
                .is_some_and(|v| !nonempty(v))
            || (row.beneficiary_country_tag.is_none()
                && row.target_country_tag.is_none()
                && row.target_state_ids.is_empty()
                && row.target_ship_ids.is_empty())
            || row
                .target_state_ids
                .iter()
                .any(|id| !state_set.contains(id))
            || !allocation_ids.insert(row.allocation_id.as_str())
            || row
                .contested_by_participant_ids
                .iter()
                .any(|id| !participant_ids.contains(id.as_str()) || id == &row.actor_participant_id)
        {
            return Err(Xui08Rejection::InvalidAllocation);
        }
    }
    let mut group_ids = BTreeSet::new();
    let mut grouped_allocation_ids = BTreeSet::new();
    if !exact_ordinals(active.allocation_groups.iter().map(|row| row.ordinal))
        || active.allocation_groups.iter().any(|row| {
            !nonempty(&row.source_record_id)
                || !nonempty(&row.group_id)
                || !group_ids.insert(row.group_id.as_str())
                || row.allocation_ids.is_empty()
                || !unique_nonempty(row.allocation_ids.iter().cloned())
                || row.allocation_ids.iter().any(|id| {
                    !allocation_ids.contains(id.as_str())
                        || !grouped_allocation_ids.insert(id.as_str())
                })
                || row
                    .flag_texture_path
                    .as_deref()
                    .is_some_and(|path| !nonempty(path))
                || !valid_peace_atom(&row.target_display)
                || !valid_peace_atom(&row.available_state_count)
                || !valid_peace_atom(&row.wargoal_count)
                || !valid_peace_atom(&row.contested_state_count)
        })
        || grouped_allocation_ids != allocation_ids
    {
        return Err(Xui08Rejection::InvalidAllocation);
    }
    if !exact_ordinals(active.results.iter().map(|row| row.ordinal)) {
        return Err(Xui08Rejection::InvalidResult);
    }
    let mut result_ids = BTreeSet::new();
    for row in &active.results {
        let valid_state_ids = unique_nonempty(row.target_state_ids.iter().map(u32::to_string))
            && !row.target_state_ids.contains(&0);
        let valid_ship_ids = unique_nonempty(row.target_ship_ids.iter().cloned());
        if !nonempty(&row.source_record_id)
            || !nonempty(&row.result_id)
            || !nonempty(&row.action_kind)
            || !nonempty(&row.actor_country_tag)
            || !valid_peace_atom(&row.outcome)
            || !valid_state_ids
            || !valid_ship_ids
            || row
                .beneficiary_country_tag
                .as_deref()
                .is_some_and(|value| !nonempty(value))
            || row
                .target_country_tag
                .as_deref()
                .is_some_and(|value| !nonempty(value))
            || !result_ids.insert(row.result_id.as_str())
        {
            return Err(Xui08Rejection::InvalidResult);
        }
        if nonempty(&row.allocation_id) {
            let Some(allocation) = active
                .allocations
                .iter()
                .find(|allocation| allocation.allocation_id == row.allocation_id)
            else {
                return Err(Xui08Rejection::InvalidResult);
            };
            let actor_country = active
                .participants
                .iter()
                .find(|participant| participant.participant_id == allocation.actor_participant_id)
                .map(|participant| participant.country_tag.as_str());
            if actor_country != Some(row.actor_country_tag.as_str())
                || row.action_kind != allocation.action_kind
                || row.beneficiary_country_tag != allocation.beneficiary_country_tag
                || row.target_country_tag != allocation.target_country_tag
                || row.target_state_ids != allocation.target_state_ids
                || row.target_ship_ids != allocation.target_ship_ids
            {
                return Err(Xui08Rejection::InvalidResult);
            }
        }
    }
    let mut assigned_allocations = BTreeSet::new();
    let mut assigned_results = BTreeSet::new();
    let memberships_valid = active.participants.iter().all(|participant| {
        participant.allocation_ids.iter().all(|id| {
            active
                .allocations
                .iter()
                .find(|allocation| allocation.allocation_id == *id)
                .is_some_and(|allocation| {
                    allocation.actor_participant_id == participant.participant_id
                        && assigned_allocations.insert(id.as_str())
                })
        }) && participant.result_ids.iter().all(|id| {
            active
                .results
                .iter()
                .find(|result| result.result_id == *id)
                .is_some_and(|result| {
                    result.actor_country_tag == participant.country_tag
                        && assigned_results.insert(id.as_str())
                })
        })
    }) && assigned_allocations == allocation_ids
        && assigned_results == result_ids;
    if !memberships_valid {
        return Err(Xui08Rejection::InvalidParticipantSet);
    }
    let contested_ids = active
        .allocations
        .iter()
        .filter(|allocation| !allocation.contested_by_participant_ids.is_empty())
        .map(|allocation| allocation.allocation_id.as_str())
        .collect::<BTreeSet<_>>();
    let popup_selected_ids = active
        .contested_popups
        .iter()
        .map(|popup| popup.selected_allocation_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut popup_ids = BTreeSet::new();
    if !exact_ordinals(active.contested_popups.iter().map(|popup| popup.ordinal))
        || popup_selected_ids != contested_ids
        || active.contested_popups.iter().any(|popup| {
            let Some(selected) = active
                .allocations
                .iter()
                .find(|allocation| allocation.allocation_id == popup.selected_allocation_id)
            else {
                return true;
            };
            let expected_participants = std::iter::once(selected.actor_participant_id.as_str())
                .chain(
                    selected
                        .contested_by_participant_ids
                        .iter()
                        .map(String::as_str),
                )
                .collect::<Vec<_>>();
            let mut popup_allocation_ids = BTreeSet::new();
            !nonempty(&popup.source_record_id)
                || !nonempty(&popup.popup_id)
                || !popup_ids.insert(popup.popup_id.as_str())
                || !exact_ordinals(popup.rows.iter().map(|row| row.ordinal))
                || popup.rows.len() != expected_participants.len()
                || popup
                    .rows
                    .first()
                    .is_none_or(|row| row.allocation_id != popup.selected_allocation_id)
                || popup.rows.iter().enumerate().any(|(ordinal, row)| {
                    !nonempty(&row.source_record_id)
                        || !nonempty(&row.popup_row_id)
                        || row.participant_id != expected_participants[ordinal]
                        || !popup_allocation_ids.insert(row.allocation_id.as_str())
                        || active
                            .allocations
                            .iter()
                            .find(|allocation| allocation.allocation_id == row.allocation_id)
                            .is_none_or(|allocation| {
                                allocation.actor_participant_id != row.participant_id
                            })
                })
        })
    {
        return Err(Xui08Rejection::InvalidAllocation);
    }
    Ok(AdmittedPeaceConference {
        snapshot: snapshot.clone(),
    })
}

pub fn peace_conference_snapshot_from_world(
    frame: &WorldFrame,
) -> Result<PeaceConferenceSnapshot, Xui08Rejection> {
    if let Some(exact) = frame
        .peace_conference_presentation_exact()
        .presentation
        .as_ref()
    {
        let atom = |value: &replay_core::PeaceFinalTextExact| DisplayAtom {
            source_record_id: value.authored_identity.clone(),
            raw_value: value.raw_value.clone(),
            rendered_text: value.final_text.clone(),
        };
        return match &exact.observation {
            PeaceConferencePresentationObservationExact::NoConferenceInProgress {} => {
                Ok(PeaceConferenceSnapshot::NoConferenceInProgress {
                    source_record_id: format!(
                        "pcs3:{}:{}",
                        exact.record_sha256, exact.frame_hour
                    ),
                })
            }
            PeaceConferencePresentationObservationExact::Inactive { refresh_sequence } => {
                Ok(PeaceConferenceSnapshot::Inactive {
                    source_record_id: format!(
                        "pcs3:{}:{}:{}",
                        exact.record_sha256, exact.frame_hour, refresh_sequence
                    ),
                })
            }
            PeaceConferencePresentationObservationExact::Active {
                conference_reference,
                phase,
                turn,
                remaining_score_turns,
                current_participant_id,
                current_score,
                score_this_turn,
                available_state_ids,
                contested_state_ids,
                participants,
                allocations,
                allocation_groups,
                contested_popups,
                results,
                ..
            } => {
                let reference = PeaceConferenceReference {
                    object_type: conference_reference.object_type,
                    object_id: conference_reference.object_id,
                };
                let snapshot = PeaceConferenceSnapshot::Active(ActivePeaceConferenceSnapshot {
                    source_record_id: exact.record_sha256.clone(),
                    conference_reference: reference,
                    conference_id: reference.portable_id(),
                    phase: phase.token().into(),
                    turn: *turn,
                    remaining_score_turns: *remaining_score_turns,
                    current_participant_id: current_participant_id.clone().unwrap_or_default(),
                    current_score: atom(current_score),
                    score_this_turn: atom(score_this_turn),
                    available_state_ids: available_state_ids.clone(),
                    contested_state_ids: contested_state_ids.clone(),
                    participants: participants
                        .iter()
                        .map(|row| PeaceParticipantRow {
                            source_record_id: format!(
                                "{}:participant:{}",
                                exact.record_sha256, row.source_ordinal
                            ),
                            ordinal: row.source_ordinal,
                            participant_id: row.stable_participant_id.clone(),
                            country_tag: row.country_tag.clone(),
                            role: match row.role {
                                PeaceParticipantRoleExact::Victor => PeaceParticipantRole::Victor,
                                PeaceParticipantRoleExact::Beneficiary => {
                                    PeaceParticipantRole::Beneficiary
                                }
                                PeaceParticipantRoleExact::Defeated => {
                                    PeaceParticipantRole::Defeated
                                }
                            },
                            score: atom(&row.score),
                            score_ratio: atom(&row.score_ratio),
                            allocation_ids: row.allocation_ids.clone(),
                            result_ids: row.result_ids.clone(),
                            bid_count: atom(&row.bid_count_display),
                            contested_bid_count: atom(&row.contested_bid_count_display),
                        })
                        .collect(),
                    allocations: allocations
                        .iter()
                        .map(|row| PeaceAllocationRow {
                            source_record_id: format!(
                                "{}:allocation:{}",
                                exact.record_sha256, row.source_ordinal
                            ),
                            ordinal: row.source_ordinal,
                            allocation_id: row.stable_allocation_id.clone(),
                            turn: row.turn,
                            action_kind: row.action_kind.token().into(),
                            actor_participant_id: row.actor_participant_id.clone(),
                            beneficiary_country_tag: row.beneficiary_country_tag.clone(),
                            target_country_tag: row.target_country_tag.clone(),
                            target_state_ids: row.target_state_ids.clone(),
                            target_ship_ids: row.target_ship_ids.clone(),
                            cost: atom(&row.cost),
                            bid: atom(&row.bid),
                            status: atom(&row.status),
                            popup_bid_count: atom(&row.popup_bid_count_display),
                            contested_by_participant_ids: row.contested_by_participant_ids.clone(),
                        })
                        .collect(),
                    allocation_groups: allocation_groups
                        .iter()
                        .map(|row| PeaceAllocationGroupRow {
                            source_record_id: format!(
                                "{}:allocation-group:{}",
                                exact.record_sha256, row.source_ordinal
                            ),
                            ordinal: row.source_ordinal,
                            group_id: row.stable_group_id.clone(),
                            allocation_ids: row.allocation_ids.clone(),
                            flag_texture_path: row.flag_texture_path.clone(),
                            target_display: atom(&row.target_display),
                            available_state_count: atom(&row.available_state_count_display),
                            wargoal_count: atom(&row.wargoal_count_display),
                            contested_state_count: atom(&row.contested_state_count_display),
                        })
                        .collect(),
                    contested_popups: contested_popups
                        .iter()
                        .map(|popup| PeaceContestedPopup {
                            source_record_id: format!(
                                "{}:contested-popup:{}",
                                exact.record_sha256, popup.source_ordinal
                            ),
                            ordinal: popup.source_ordinal,
                            popup_id: popup.stable_popup_id.clone(),
                            selected_allocation_id: popup.selected_allocation_id.clone(),
                            rows: popup
                                .rows
                                .iter()
                                .map(|row| PeaceContestedPopupRow {
                                    source_record_id: format!(
                                        "{}:contested-popup:{}:row:{}",
                                        exact.record_sha256,
                                        popup.source_ordinal,
                                        row.source_ordinal
                                    ),
                                    ordinal: row.source_ordinal,
                                    popup_row_id: row.stable_popup_row_id.clone(),
                                    participant_id: row.participant_id.clone(),
                                    allocation_id: row.allocation_id.clone(),
                                })
                                .collect(),
                        })
                        .collect(),
                    results: results
                        .iter()
                        .map(|row| PeaceResultRow {
                            source_record_id: format!(
                                "{}:result:{}",
                                exact.record_sha256, row.source_ordinal
                            ),
                            ordinal: row.source_ordinal,
                            result_id: row.stable_result_id.clone(),
                            allocation_id: row.allocation_id.clone().unwrap_or_default(),
                            action_kind: row.action_kind.token().into(),
                            actor_country_tag: row.actor_country_tag.clone(),
                            beneficiary_country_tag: row.beneficiary_country_tag.clone(),
                            target_country_tag: row.target_country_tag.clone(),
                            target_state_ids: row.target_state_ids.clone(),
                            target_ship_ids: row.target_ship_ids.clone(),
                            outcome: atom(&row.outcome),
                        })
                        .collect(),
                    complete: true,
                });
                admit_peace_conference(&snapshot)?;
                Ok(snapshot)
            }
        };
    }
    let exact = frame
        .peace_conference_exact()
        .conference
        .as_ref()
        .ok_or(Xui08Rejection::MissingProvenance)?;
    match &exact.observation {
        PeaceConferenceObservationExact::NoConferenceInProgress {} => {
            Ok(PeaceConferenceSnapshot::NoConferenceInProgress {
                source_record_id: format!(
                    "pcs1:{}:{}",
                    exact.producer_manifest_sha256, exact.frame_hour
                ),
            })
        }
        PeaceConferenceObservationExact::Inactive { refresh_sequence } => {
            Ok(PeaceConferenceSnapshot::Inactive {
                source_record_id: format!(
                    "pcs1:{}:{}:{}",
                    exact.producer_manifest_sha256, exact.frame_hour, refresh_sequence
                ),
            })
        }
        PeaceConferenceObservationExact::Active { .. } => Err(Xui08Rejection::Incomplete),
    }
}
