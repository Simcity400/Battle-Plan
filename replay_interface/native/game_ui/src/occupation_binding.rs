//! XUI16 production binding for occupation, resistance/compliance and garrisons.
//!
//! The binding consumes only typed replay leaves plus authenticated XUI03/XUI06
//! producer inputs. It does not reconstruct controller ordering, localized
//! strings, equipment completeness, alert rows, or garrison history.

use crate::compose::ComposeOptions;
use crate::dynamic_collection::DynamicCollectionTrace;
use replay_core::{Leaf, OccupationRow};
use std::collections::{BTreeMap, BTreeSet};

pub const COUNTRY_OCCUPATION_GUI: &str = "interface/countryoccupationview.gui";
pub const GARRISON_LOG_GUI: &str = "interface/garrisonlogview.gui";
pub const COUNTRY_STATE_GUI: &str = "interface/countrystateview.gui";
pub const OCCUPATION_LAWS_SOURCE: &str = "common/occupation_laws/occupation_laws.txt";
pub const OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT: u32 = 0x3f;
pub const OCCUPATION_GARRISON_EQUIPMENT_FIXED_POINT_DIVISOR: i64 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationLawDefinition {
    pub token: String,
    pub display_name: String,
    /// One-based frame in `GFX_occupation_policy_icon_strip`.
    pub icon_frame: usize,
    pub gui_order: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GarrisonTemplateDefinition {
    pub template_id: u32,
    pub display_name: String,
    pub icon_sprite: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationVisibleText {
    pub state_name: String,
    pub occupied_country_name: String,
    pub resistance_text: String,
    pub resistance_speed_text: String,
    pub compliance_text: String,
    pub compliance_speed_text: String,
    pub garrison_manpower_need_text: String,
    pub garrison_required_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GarrisonEquipmentRow {
    pub equipment_id: u32,
    pub equipment_type: u32,
    pub equipment_token: String,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GarrisonActivityRow {
    pub stable_id: String,
    pub date_text: String,
    pub location_text: String,
    pub state_id: u32,
    pub occupied_country_tag: String,
    pub attack_blocked: bool,
    pub lost_manpower: i64,
    pub lost_equipment: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GarrisonEquipmentLossRow {
    pub stable_id: String,
    pub equipment_id: u32,
    pub equipment_type: u32,
    pub equipment_token: String,
    pub display_name: String,
    pub amount_lost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationAlertRow {
    pub stable_id: String,
    pub controller_tag: String,
    pub kind: String,
    pub severity: String,
    /// Slot registered by `hoi4.exe` function `0x140aeeaa0`.
    pub registry_slot: u32,
    /// `0x140afe8a0` returns false when a shortage activates this alert.
    pub predicate_returned: bool,
    /// Exact signed fixed-point divisor used for equipment shortage children.
    pub equipment_fixed_point_divisor: i64,
    pub visible_order: u32,
    pub missing_manpower: i64,
    pub missing_equipment: Vec<GarrisonEquipmentRow>,
    pub resolved_text: String,
}

pub struct OccupationSurfaceRequest<'a> {
    pub source_record_id: &'a str,
    pub occupation_rows: &'a BTreeMap<u32, OccupationRow>,
    pub visible_text_by_state: &'a BTreeMap<u32, OccupationVisibleText>,
    pub law_definitions_by_hash: &'a BTreeMap<u32, OccupationLawDefinition>,
    pub garrison_templates: &'a BTreeMap<u32, GarrisonTemplateDefinition>,
    pub equipment_by_state: &'a BTreeMap<u32, Vec<GarrisonEquipmentRow>>,
    pub equipment_complete: bool,
    pub occupation_collection: &'a DynamicCollectionTrace,
    pub activity_collection: &'a DynamicCollectionTrace,
    pub equipment_loss_collection: &'a DynamicCollectionTrace,
    pub activities: &'a [GarrisonActivityRow],
    pub activities_complete: bool,
    pub equipment_losses: &'a [GarrisonEquipmentLossRow],
    pub equipment_losses_complete: bool,
    pub alerts: &'a [OccupationAlertRow],
    pub alerts_complete: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationStatePresentation {
    pub state_id: u32,
    pub occupier_tag: String,
    pub occupied_country_tag: String,
    pub text: OccupationVisibleText,
    pub resistance: f64,
    pub resistance_speed: f64,
    pub compliance: f64,
    pub compliance_speed: f64,
    pub law: OccupationLawDefinition,
    pub garrison_template: GarrisonTemplateDefinition,
    pub garrison_army_manpower_need: i64,
    pub garrison_strength_ratio: f64,
    pub garrison_required: f64,
    pub garrison_manpower_source_tag_db_id: Option<u32>,
    pub equipment: Vec<GarrisonEquipmentRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedOccupationSurface {
    source_record_id: String,
    states: Vec<OccupationStatePresentation>,
    activities: Vec<GarrisonActivityRow>,
    equipment_losses: Vec<GarrisonEquipmentLossRow>,
    alerts: Vec<OccupationAlertRow>,
}

impl AdmittedOccupationSurface {
    pub fn source_record_id(&self) -> &str {
        &self.source_record_id
    }
    pub fn states(&self) -> &[OccupationStatePresentation] {
        &self.states
    }
    pub fn activities(&self) -> &[GarrisonActivityRow] {
        &self.activities
    }
    pub fn equipment_losses(&self) -> &[GarrisonEquipmentLossRow] {
        &self.equipment_losses
    }
    pub fn alerts(&self) -> &[OccupationAlertRow] {
        &self.alerts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccupationRejection {
    MissingProvenance,
    IncompleteProducer,
    InvalidCollectionTrace,
    CollectionPopulationMismatch,
    MissingVisibleText,
    MissingOrInvalidOccupationValue,
    MissingLawDefinition,
    LawHashMismatch,
    MissingGarrisonTemplate,
    InvalidGarrisonEquipment,
    InvalidGarrisonLog,
    InvalidAlertRows,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationMapModeStateInput {
    pub state_id: u32,
    pub resistance: Option<f32>,
    pub compliance: Option<f32>,
}

/// Shared state inputs for the resistance/compliance map modes. Unmeasured
/// values remain `None`, so map composition can fail closed per state.
pub fn occupation_map_mode_inputs(
    rows: &BTreeMap<u32, OccupationRow>,
) -> Vec<OccupationMapModeStateInput> {
    rows.iter()
        .map(|(&state_id, row)| OccupationMapModeStateInput {
            state_id,
            resistance: measured_f64(&row.resistance)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|v| v as f32),
            compliance: measured_f64(&row.compliance)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|v| v as f32),
        })
        .collect()
}

pub fn admit_occupation_surface(
    request: OccupationSurfaceRequest<'_>,
) -> Result<AdmittedOccupationSurface, OccupationRejection> {
    if request.source_record_id.trim().is_empty() {
        return Err(OccupationRejection::MissingProvenance);
    }
    if !request.activities_complete
        || !request.equipment_losses_complete
        || !request.alerts_complete
    {
        return Err(OccupationRejection::IncompleteProducer);
    }
    for trace in [
        request.occupation_collection,
        request.activity_collection,
        request.equipment_loss_collection,
    ] {
        trace
            .validate()
            .map_err(|_| OccupationRejection::InvalidCollectionTrace)?;
    }

    let state_ids = request
        .occupation_rows
        .keys()
        .map(|id| format!("state:{id}"))
        .collect::<BTreeSet<_>>();
    if request
        .occupation_collection
        .ordered_item_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        != state_ids
    {
        return Err(OccupationRejection::CollectionPopulationMismatch);
    }

    let mut states = Vec::with_capacity(request.occupation_rows.len());
    for item_id in &request.occupation_collection.ordered_item_ids {
        let state_id = parse_prefixed_u32(item_id, "state:")
            .ok_or(OccupationRejection::CollectionPopulationMismatch)?;
        let row = request
            .occupation_rows
            .get(&state_id)
            .ok_or(OccupationRejection::CollectionPopulationMismatch)?;
        states.push(admit_state(&request, state_id, row)?);
    }

    let activities = order_rows(request.activities, request.activity_collection, |row| {
        row.stable_id.as_str()
    })
    .ok_or(OccupationRejection::InvalidGarrisonLog)?;
    let equipment_losses = order_rows(
        request.equipment_losses,
        request.equipment_loss_collection,
        |row| row.stable_id.as_str(),
    )
    .ok_or(OccupationRejection::InvalidGarrisonLog)?;
    validate_activity_rows(&activities, request.occupation_rows)?;
    validate_equipment_losses(&equipment_losses)?;

    let mut alerts = request.alerts.to_vec();
    alerts.sort_by_key(|row| row.visible_order);
    validate_alerts(&alerts)?;

    Ok(AdmittedOccupationSurface {
        source_record_id: request.source_record_id.to_owned(),
        states,
        activities,
        equipment_losses,
        alerts,
    })
}

fn admit_state(
    request: &OccupationSurfaceRequest<'_>,
    state_id: u32,
    row: &OccupationRow,
) -> Result<OccupationStatePresentation, OccupationRejection> {
    let resistance = percent(&row.resistance)?;
    let compliance = percent(&row.compliance)?;
    let resistance_speed = finite(&row.resistance_speed)?;
    let compliance_speed = finite(&row.compliance_speed)?;
    let occupier_tag = nonempty_leaf(&row.occupier_tag)?;
    let occupied_country_tag = nonempty_leaf(&row.occupied_country_tag)?;
    let text = request
        .visible_text_by_state
        .get(&state_id)
        .filter(|text| visible_text_complete(text))
        .cloned()
        .ok_or(OccupationRejection::MissingVisibleText)?;

    if matches!(row.occupation_law_status, Leaf::Measured(_)) {
        return Err(OccupationRejection::MissingLawDefinition);
    }
    let law_hash = measured_u32(&row.occupation_law_token_id)
        .ok_or(OccupationRejection::MissingLawDefinition)?;
    let law_encoding = nonempty_leaf(&row.occupation_law_token_encoding)
        .map_err(|_| OccupationRejection::MissingLawDefinition)?;
    if !matches!(law_encoding, "fnv1a32_of_law_token" | "fnv1a32_name_hash") {
        return Err(OccupationRejection::MissingLawDefinition);
    }
    let law = request
        .law_definitions_by_hash
        .get(&law_hash)
        .filter(|law| {
            !law.token.trim().is_empty()
                && !law.display_name.trim().is_empty()
                && law.icon_frame > 0
        })
        .cloned()
        .ok_or(OccupationRejection::MissingLawDefinition)?;
    if fnv1a32(&law.token) != law_hash {
        return Err(OccupationRejection::LawHashMismatch);
    }

    let template_id = measured_u32(&row.garrison_template_id)
        .ok_or(OccupationRejection::MissingGarrisonTemplate)?;
    let garrison_template = request
        .garrison_templates
        .get(&template_id)
        .filter(|definition| {
            definition.template_id == template_id
                && !definition.display_name.trim().is_empty()
                && !definition.icon_sprite.trim().is_empty()
        })
        .cloned()
        .ok_or(OccupationRejection::MissingGarrisonTemplate)?;
    if matches!(row.garrison_signals_status, Leaf::Measured(_)) {
        return Err(OccupationRejection::MissingOrInvalidOccupationValue);
    }
    let garrison_army_manpower_need = measured_i64(&row.garrison_army_manpower_need)
        .filter(|value| *value >= 0)
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)?;
    let garrison_strength_ratio = unit_interval(&row.garrison_strength_ratio)?;
    let garrison_required = nonnegative(&row.garrison_required)?;

    let equipment = typed_garrison_equipment(row)?.unwrap_or_else(|| {
        if !request.equipment_complete {
            return Vec::new();
        }
        request
            .equipment_by_state
            .get(&state_id)
            .cloned()
            .unwrap_or_default()
    });
    if equipment.is_empty()
        && !matches!(row.garrison_equipment_complete, Leaf::Measured(true))
        && (!request.equipment_complete || !request.equipment_by_state.contains_key(&state_id))
    {
        return Err(OccupationRejection::InvalidGarrisonEquipment);
    }
    validate_equipment(&equipment)?;

    Ok(OccupationStatePresentation {
        state_id,
        occupier_tag: occupier_tag.to_owned(),
        occupied_country_tag: occupied_country_tag.to_owned(),
        text,
        resistance,
        resistance_speed,
        compliance,
        compliance_speed,
        law,
        garrison_template,
        garrison_army_manpower_need,
        garrison_strength_ratio,
        garrison_required,
        garrison_manpower_source_tag_db_id: measured_u32(&row.garrison_manpower_source_tag_db_id),
        equipment,
    })
}

/// Convert the save-backed typed carrier directly into XUI16 equipment rows.
/// `None` means the carrier predates this field and the authenticated live
/// producer may be used; measured false or malformed children fail closed.
fn typed_garrison_equipment(
    row: &OccupationRow,
) -> Result<Option<Vec<GarrisonEquipmentRow>>, OccupationRejection> {
    match (&row.garrison_equipment_complete, &row.garrison_equipment) {
        (Leaf::Measured(true), Leaf::Measured(rows)) => rows
            .iter()
            .map(|row| {
                Ok(GarrisonEquipmentRow {
                    equipment_id: measured_u32(&row.equipment_id)
                        .ok_or(OccupationRejection::InvalidGarrisonEquipment)?,
                    equipment_type: measured_u32(&row.equipment_type)
                        .ok_or(OccupationRejection::InvalidGarrisonEquipment)?,
                    equipment_token: nonempty_leaf(&row.equipment_token)
                        .map_err(|_| OccupationRejection::InvalidGarrisonEquipment)?
                        .to_owned(),
                    amount: measured_f64(&row.amount)
                        .ok_or(OccupationRejection::InvalidGarrisonEquipment)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        (Leaf::Measured(true), _) | (Leaf::Measured(false), _) => {
            Err(OccupationRejection::InvalidGarrisonEquipment)
        }
        _ => Ok(None),
    }
}

/// Bind the shipped single-state/country entry names. Dynamic row placement is
/// owned by the authenticated XUI03 materialized slots.
pub fn apply_occupation_state_to_compose_options(
    state: &OccupationStatePresentation,
    options: &mut ComposeOptions,
) {
    options
        .text_overrides
        .insert("state_name".into(), state.text.state_name.clone());
    options
        .text_overrides
        .insert("selected_law".into(), state.law.display_name.clone());
    options.text_overrides.insert(
        "template_need_text".into(),
        state.text.garrison_required_text.clone(),
    );
    options
        .frame_overrides
        .insert("select_law_icon".into(), state.law.icon_frame);
    options
        .progress_overrides
        .insert("bar_str".into(), state.garrison_strength_ratio as f32);
}

fn validate_equipment(rows: &[GarrisonEquipmentRow]) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    for row in rows {
        if row.equipment_id == 0
            || row.equipment_type == 0
            || row.equipment_token.trim().is_empty()
            || !row.amount.is_finite()
            || row.amount < 0.0
            || !ids.insert((row.equipment_id, row.equipment_type))
        {
            return Err(OccupationRejection::InvalidGarrisonEquipment);
        }
    }
    Ok(())
}

fn validate_activity_rows(
    rows: &[GarrisonActivityRow],
    occupation: &BTreeMap<u32, OccupationRow>,
) -> Result<(), OccupationRejection> {
    for row in rows {
        if row.stable_id.trim().is_empty()
            || row.date_text.trim().is_empty()
            || row.location_text.trim().is_empty()
            || row.occupied_country_tag.trim().is_empty()
            || row.lost_manpower < 0
            || row.lost_equipment < 0
            || !occupation.contains_key(&row.state_id)
        {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
    }
    Ok(())
}

fn validate_equipment_losses(rows: &[GarrisonEquipmentLossRow]) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    for row in rows {
        if row.stable_id.trim().is_empty()
            || row.equipment_id == 0
            || row.equipment_type == 0
            || row.equipment_token.trim().is_empty()
            || row.display_name.trim().is_empty()
            || !row.amount_lost.is_finite()
            || row.amount_lost < 0.0
            || !ids.insert(row.stable_id.as_str())
        {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
    }
    Ok(())
}

fn validate_alerts(rows: &[OccupationAlertRow]) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    for (ordinal, row) in rows.iter().enumerate() {
        if row.stable_id.trim().is_empty()
            || row.controller_tag.trim().is_empty()
            || row.kind != "alert_not_enough_garrison"
            || row.severity != "HIGH"
            || row.registry_slot != OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT
            || row.predicate_returned
            || row.equipment_fixed_point_divisor
                != OCCUPATION_GARRISON_EQUIPMENT_FIXED_POINT_DIVISOR
            || row.resolved_text.trim().is_empty()
            || row.visible_order as usize != ordinal
            || row.missing_manpower < 0
            || !ids.insert(row.stable_id.as_str())
        {
            return Err(OccupationRejection::InvalidAlertRows);
        }
        validate_equipment(&row.missing_equipment)
            .map_err(|_| OccupationRejection::InvalidAlertRows)?;
        if row
            .missing_equipment
            .iter()
            .any(|equipment| equipment.amount <= 0.0 || equipment.amount.fract() != 0.0)
        {
            return Err(OccupationRejection::InvalidAlertRows);
        }
        if row.missing_manpower == 0 && row.missing_equipment.is_empty() {
            return Err(OccupationRejection::InvalidAlertRows);
        }
    }
    Ok(())
}

fn order_rows<T: Clone>(
    rows: &[T],
    trace: &DynamicCollectionTrace,
    id: impl Fn(&T) -> &str,
) -> Option<Vec<T>> {
    let by_id = rows
        .iter()
        .map(|row| (id(row), row))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != rows.len() || by_id.len() != trace.ordered_item_ids.len() {
        return None;
    }
    trace
        .ordered_item_ids
        .iter()
        .map(|item| by_id.get(item.as_str()).map(|row| (*row).clone()))
        .collect()
}

fn visible_text_complete(text: &OccupationVisibleText) -> bool {
    [
        &text.state_name,
        &text.occupied_country_name,
        &text.resistance_text,
        &text.resistance_speed_text,
        &text.compliance_text,
        &text.compliance_speed_text,
        &text.garrison_manpower_need_text,
        &text.garrison_required_text,
    ]
    .into_iter()
    .all(|value| !value.trim().is_empty())
}

fn parse_prefixed_u32(value: &str, prefix: &str) -> Option<u32> {
    value.strip_prefix(prefix)?.parse().ok()
}

fn measured_f64(value: &Leaf<f64>) -> Option<f64> {
    value.measured().copied()
}
fn measured_i64(value: &Leaf<i64>) -> Option<i64> {
    value.measured().copied()
}
fn measured_u32(value: &Leaf<u32>) -> Option<u32> {
    value.measured().copied()
}
fn nonempty_leaf(value: &Leaf<String>) -> Result<&str, OccupationRejection> {
    value
        .measured()
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn percent(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn finite(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite())
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn nonnegative(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn unit_interval(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}

pub fn fnv1a32(text: &str) -> u32 {
    let mut value = 0x811C_9DC5u32;
    for byte in text.as_bytes() {
        value ^= u32::from(*byte);
        value = value.wrapping_mul(0x0100_0193);
    }
    if value == 0 {
        1
    } else {
        value
    }
}
