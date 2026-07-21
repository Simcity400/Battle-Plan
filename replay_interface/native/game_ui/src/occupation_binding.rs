//! XUI16 production binding for occupation, resistance/compliance and garrisons.
//!
//! The binding consumes only typed replay leaves plus authenticated XUI03/XUI06
//! producer inputs. Its production entry point constructs alert/history rows
//! from exact typed carriers; callers still own resolved strings and collection
//! geometry/order, but cannot provide semantic alert or loss rows.

use crate::compose::ComposeOptions;
use crate::dynamic_collection::DynamicCollectionTrace;
use replay_core::{Leaf, OccupationRow, ViewStatus, WorldFrame};
use std::collections::{BTreeMap, BTreeSet};

pub const COUNTRY_OCCUPATION_GUI: &str = "interface/countryoccupationview.gui";
pub const GARRISON_LOG_GUI: &str = "interface/garrisonlogview.gui";
pub const COUNTRY_STATE_GUI: &str = "interface/countrystateview.gui";
pub const OCCUPATION_LAWS_SOURCE: &str = "common/occupation_laws/occupation_laws.txt";
pub const OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT: u32 = 0x3f;
pub const OCCUPATION_GARRISON_EQUIPMENT_FIXED_POINT_DIVISOR: i64 = 100_000;
const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_992.0;

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
    pub controller_tag: String,
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
    /// Exact history events contributing to this equipment-tab aggregate.
    pub contributing_activity_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GarrisonShortageEquipmentRow {
    /// Stable `CEquipmentArchetype+0x418` identity from the exact alert
    /// producer. This is deliberately not a save equipment-variant identity.
    pub archetype_id: u32,
    /// Positive whole-unit deficit after the producer's exact /100000 law.
    pub whole_amount: i64,
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
    pub missing_equipment: Vec<GarrisonShortageEquipmentRow>,
    pub resolved_text: String,
}

#[derive(Clone, Copy)]
pub struct OccupationProductionSurfaceRequest<'a> {
    pub source_record_id: &'a str,
    pub frame: &'a WorldFrame,
    pub visible_text_by_state: &'a BTreeMap<u32, OccupationVisibleText>,
    pub law_definitions_by_hash: &'a BTreeMap<u32, OccupationLawDefinition>,
    pub garrison_templates: &'a BTreeMap<u32, GarrisonTemplateDefinition>,
    pub equipment_by_state: &'a BTreeMap<u32, Vec<GarrisonEquipmentRow>>,
    pub equipment_complete: bool,
    pub occupation_collection: &'a DynamicCollectionTrace,
    pub activity_collection: &'a DynamicCollectionTrace,
    pub equipment_loss_collection: &'a DynamicCollectionTrace,
    /// A25/A28-resolved location text keyed by exact history stable_event_id.
    pub activity_location_text: &'a BTreeMap<String, String>,
    /// A25-resolved display names keyed by save equipment variant identity.
    pub equipment_display_names: &'a BTreeMap<(u32, u32), String>,
    /// A25-resolved final alert text keyed by controller TAG.
    pub alert_resolved_text: &'a BTreeMap<String, String>,
    /// A28-authenticated global alert-manager order keyed by controller TAG.
    pub alert_visible_order: &'a BTreeMap<String, u32>,
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

/// Production XUI16 entry point. History rows and alert rows are constructed
/// only from the typed replay carriers; callers may supply resolved strings
/// and authenticated collection traces, but cannot fabricate semantic rows.
pub fn admit_occupation_surface_from_world(
    request: OccupationProductionSurfaceRequest<'_>,
) -> Result<AdmittedOccupationSurface, OccupationRejection> {
    let frame_hour = u64::try_from(request.frame.hour_index())
        .map_err(|_| OccupationRejection::InvalidCollectionTrace)?;
    if [
        request.occupation_collection,
        request.activity_collection,
        request.equipment_loss_collection,
    ]
    .iter()
    .any(|trace| trace.frame_hour != frame_hour)
    {
        return Err(OccupationRejection::InvalidCollectionTrace);
    }
    let (activities, equipment_losses) = production_garrison_history_rows(
        request.frame,
        request.activity_location_text,
        request.equipment_display_names,
    )?;
    let alerts = production_garrison_alert_rows(
        request.frame,
        request.alert_resolved_text,
        request.alert_visible_order,
    )?;
    admit_occupation_surface(OccupationSurfaceRequest {
        source_record_id: request.source_record_id,
        occupation_rows: request.frame.occupation_by_state(),
        visible_text_by_state: request.visible_text_by_state,
        law_definitions_by_hash: request.law_definitions_by_hash,
        garrison_templates: request.garrison_templates,
        equipment_by_state: request.equipment_by_state,
        equipment_complete: request.equipment_complete,
        occupation_collection: request.occupation_collection,
        activity_collection: request.activity_collection,
        equipment_loss_collection: request.equipment_loss_collection,
        activities: &activities,
        activities_complete: true,
        equipment_losses: &equipment_losses,
        equipment_losses_complete: true,
        alerts: &alerts,
        alerts_complete: true,
    })
}

fn production_garrison_history_rows(
    frame: &WorldFrame,
    activity_location_text: &BTreeMap<String, String>,
    equipment_display_names: &BTreeMap<(u32, u32), String>,
) -> Result<(Vec<GarrisonActivityRow>, Vec<GarrisonEquipmentLossRow>), OccupationRejection> {
    let Leaf::Measured(true) = frame.occupation_history_complete() else {
        return Err(OccupationRejection::IncompleteProducer);
    };
    let Leaf::Measured(history) = frame.occupation_history() else {
        return Err(OccupationRejection::IncompleteProducer);
    };
    let mut activities = Vec::with_capacity(history.len());
    let mut losses = BTreeMap::<(u32, u32), GarrisonEquipmentLossRow>::new();
    let mut stable_ids = BTreeSet::new();
    for event in history {
        let stable_id = nonempty_leaf(&event.stable_event_id)
            .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
            .to_owned();
        if !stable_ids.insert(stable_id.clone()) {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
        let location_text = activity_location_text
            .get(&stable_id)
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or(OccupationRejection::MissingVisibleText)?;
        let Leaf::Measured(true) = &event.equipment_loss_children_complete else {
            return Err(OccupationRejection::IncompleteProducer);
        };
        let Leaf::Measured(equipment) = &event.equipment_loss_rows else {
            return Err(OccupationRejection::IncompleteProducer);
        };
        let mut lost_equipment = 0i64;
        for (expected_ordinal, row) in equipment.iter().enumerate() {
            let ordinal = measured_i64(&row.source_ordinal)
                .filter(|value| *value >= 0)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?;
            if usize::try_from(ordinal).ok() != Some(expected_ordinal) {
                return Err(OccupationRejection::InvalidGarrisonLog);
            }
            let equipment_id =
                measured_u32(&row.equipment_id).ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let equipment_type =
                measured_u32(&row.equipment_type).ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let equipment_token = nonempty_leaf(&row.equipment_token)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned();
            let amount = measured_f64(&row.amount)
                .filter(|value| {
                    value.is_finite()
                        && *value >= 0.0
                        && *value <= MAX_EXACT_F64_INTEGER
                        && value.fract() == 0.0
                })
                .ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let amount_i64 = amount as i64;
            if amount_i64 as f64 != amount {
                return Err(OccupationRejection::InvalidGarrisonLog);
            }
            lost_equipment = lost_equipment
                .checked_add(amount_i64)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let display_name = equipment_display_names
                .get(&(equipment_id, equipment_type))
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .ok_or(OccupationRejection::MissingVisibleText)?;
            let key = (equipment_id, equipment_type);
            match losses.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(GarrisonEquipmentLossRow {
                        stable_id: format!("equipment:{equipment_id}:{equipment_type}"),
                        equipment_id,
                        equipment_type,
                        equipment_token,
                        display_name,
                        amount_lost: amount,
                        contributing_activity_ids: vec![stable_id.clone()],
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let aggregate = entry.get_mut();
                    if aggregate.equipment_token != equipment_token
                        || aggregate.display_name != display_name
                    {
                        return Err(OccupationRejection::InvalidGarrisonLog);
                    }
                    let aggregate_i64 = aggregate.amount_lost as i64;
                    if aggregate_i64 as f64 != aggregate.amount_lost {
                        return Err(OccupationRejection::InvalidGarrisonLog);
                    }
                    let aggregate_i64 = aggregate_i64
                        .checked_add(amount_i64)
                        .ok_or(OccupationRejection::InvalidGarrisonLog)?;
                    let aggregate_f64 = aggregate_i64 as f64;
                    if aggregate_f64 as i64 != aggregate_i64 {
                        return Err(OccupationRejection::InvalidGarrisonLog);
                    }
                    aggregate.amount_lost = aggregate_f64;
                    if aggregate.contributing_activity_ids.last() != Some(&stable_id) {
                        aggregate.contributing_activity_ids.push(stable_id.clone());
                    }
                }
            }
        }
        activities.push(GarrisonActivityRow {
            stable_id,
            controller_tag: nonempty_leaf(&event.controller_tag)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned(),
            date_text: nonempty_leaf(&event.event_hour_or_engine_date)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned(),
            location_text,
            state_id: measured_u32(&event.state_id)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?,
            occupied_country_tag: nonempty_leaf(&event.occupied_country_tag)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned(),
            attack_blocked: match &event.attack_blocked {
                Leaf::Measured(value) => *value,
                _ => return Err(OccupationRejection::InvalidGarrisonLog),
            },
            lost_manpower: measured_i64(&event.lost_manpower)
                .filter(|value| *value >= 0)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?,
            lost_equipment,
        });
    }
    Ok((activities, losses.into_values().collect()))
}

fn production_garrison_alert_rows(
    frame: &WorldFrame,
    resolved_text: &BTreeMap<String, String>,
    visible_order: &BTreeMap<String, u32>,
) -> Result<Vec<OccupationAlertRow>, OccupationRejection> {
    let view = frame.occupation_garrison_alert_exact();
    if view.status != ViewStatus::Present {
        return Err(OccupationRejection::IncompleteProducer);
    }
    let alert = view
        .alert
        .as_ref()
        .ok_or(OccupationRejection::IncompleteProducer)?;
    if alert.predicate_returned {
        return Ok(Vec::new());
    }
    let text = resolved_text
        .get(&alert.controller_tag)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or(OccupationRejection::MissingVisibleText)?;
    let order = visible_order
        .get(&alert.controller_tag)
        .copied()
        .ok_or(OccupationRejection::InvalidAlertRows)?;
    Ok(vec![OccupationAlertRow {
        stable_id: format!(
            "alert_not_enough_garrison:{}:{}",
            alert.game_hour_raw, alert.controller_tag_db_id
        ),
        controller_tag: alert.controller_tag.clone(),
        kind: "alert_not_enough_garrison".into(),
        severity: "HIGH".into(),
        registry_slot: OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT,
        predicate_returned: alert.predicate_returned,
        equipment_fixed_point_divisor: i64::from(alert.equipment_fixed_point_divisor),
        visible_order: order,
        missing_manpower: alert.missing_manpower,
        missing_equipment: alert
            .missing_equipment
            .iter()
            .map(|row| GarrisonShortageEquipmentRow {
                archetype_id: row.archetype_id,
                whole_amount: row.whole_units,
            })
            .collect(),
        resolved_text: text,
    }])
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
    validate_equipment_losses(&equipment_losses, &activities)?;

    let mut alerts = request.alerts.to_vec();
    alerts.sort_by_key(|row| row.visible_order);
    validate_alerts(&alerts, request.occupation_rows)?;

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
        let Some(occupation_row) = occupation.get(&row.state_id) else {
            return Err(OccupationRejection::InvalidGarrisonLog);
        };
        let controller_matches = occupation_row.occupier_tag.measured().is_some_and(|tag| {
            !tag.trim().is_empty() && tag == &row.controller_tag
        });
        let occupied_matches = occupation_row
            .occupied_country_tag
            .measured()
            .is_some_and(|tag| !tag.trim().is_empty() && tag == &row.occupied_country_tag);
        if row.stable_id.trim().is_empty()
            || row.controller_tag.trim().is_empty()
            || row.date_text.trim().is_empty()
            || row.location_text.trim().is_empty()
            || row.occupied_country_tag.trim().is_empty()
            || row.lost_manpower < 0
            || row.lost_equipment < 0
            || !controller_matches
            || !occupied_matches
        {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
    }
    Ok(())
}

fn validate_equipment_losses(
    rows: &[GarrisonEquipmentLossRow],
    activities: &[GarrisonActivityRow],
) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    let activity_by_id = activities
        .iter()
        .map(|row| (row.stable_id.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let expected_total = activities.iter().try_fold(0i64, |total, row| {
        total.checked_add(row.lost_equipment)
            .ok_or(OccupationRejection::InvalidGarrisonLog)
    })?;
    let mut aggregate_total = 0i64;
    for row in rows {
        let exact_stable_id = format!("equipment:{}:{}", row.equipment_id, row.equipment_type);
        let integral_amount = row.amount_lost.is_finite()
            && row.amount_lost >= 0.0
            && row.amount_lost.fract() == 0.0
            && row.amount_lost <= MAX_EXACT_F64_INTEGER;
        if row.stable_id.trim().is_empty()
            || row.stable_id != exact_stable_id
            || row.equipment_id == 0
            || row.equipment_type == 0
            || row.equipment_token.trim().is_empty()
            || row.display_name.trim().is_empty()
            || !integral_amount
            || !ids.insert(row.stable_id.as_str())
            || row.contributing_activity_ids.is_empty()
            || row.contributing_activity_ids.iter().any(|activity_id| {
                !activity_by_id.contains_key(activity_id.as_str())
            })
            || row
                .contributing_activity_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                .len()
                != row.contributing_activity_ids.len()
        {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
        let amount = row.amount_lost as i64;
        let Some(total) = aggregate_total.checked_add(amount)
        else {
            return Err(OccupationRejection::InvalidGarrisonLog);
        };
        aggregate_total = total;
    }
    if aggregate_total != expected_total {
        return Err(OccupationRejection::InvalidGarrisonLog);
    }
    Ok(())
}

fn validate_alerts(
    rows: &[OccupationAlertRow],
    occupation: &BTreeMap<u32, OccupationRow>,
) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    for (ordinal, row) in rows.iter().enumerate() {
        let controller_owns_occupation = occupation.values().any(|occupation_row| {
            occupation_row
                .occupier_tag
                .measured()
                .is_some_and(|tag| !tag.trim().is_empty() && tag == &row.controller_tag)
        });
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
            || !controller_owns_occupation
            || !ids.insert(row.stable_id.as_str())
        {
            return Err(OccupationRejection::InvalidAlertRows);
        }
        let mut archetype_ids = BTreeSet::new();
        for equipment in &row.missing_equipment {
            if equipment.archetype_id == 0
                || equipment.whole_amount <= 0
                || !archetype_ids.insert(equipment.archetype_id)
            {
                return Err(OccupationRejection::InvalidAlertRows);
            }
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
