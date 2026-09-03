
use crate::assets::{normalize_rel_path, AssetResolver};
use crate::compose::{ComposeOptions, ComposeOutput, Compositor, UiAssetCache};
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_gui_document, GuiElement, Measure, MeasurePair};
use crate::loc::Localisation;
use crate::paradox_text::{parse_file_refusing_decode_damage, Document, Value};
use crate::{GameUiError, Result as UiResult};
use replay_core::NamedCountry;
use replay_core::{CommandAbilityTimer, CommandAbilityTimerKind};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct AbilityDefinition {
    pub ability_id: String,
    pub target_kind: String,
    pub name_key: String,
    pub description_key: String,
    pub icon: String,
    pub active_icon: String,
    pub base_cost_fraction: f64,
    pub authored_duration_hours: i64,
    pub authored_cooldown_hours: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedAbilityDefinition {
    pub authored_order: u32,
    pub ability_id: String,
    pub target_kind: String,
    pub name_key: String,
    pub description_key: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub active_icon: String,
    pub base_cost_fraction: f64,
    pub authored_duration_hours: i64,
    pub authored_cooldown_hours: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedAbilityCatalog {
    pub abilities: Vec<AdmittedAbilityDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAbilityRejection(pub String);

pub const ABILITY_LIST_GUI: &str = "interface/abilitylist.gui";
pub const ABILITY_LIST_ROOT: &str = "abilitylist_window";
const ABILITY_BUTTON_TEMPLATE: &str = "abilitylist_button_container";
const ABILITY_MID_TEMPLATE: &str = "abilitylist_bg_mid";
pub const XUI23_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const XUI23_FIXED_SCALE: i64 = 100_000;
const XUI23_MAX_ROWS: usize = 65_536;
pub const XUI23_PRODUCER_AUTHORITY_FIELD: &str =
    "_r2_xui23_passive_command_abilities_authority";
pub const XUI23_PRODUCER_AUTHORITY_SCHEMA: &str =
    "replay.xui23-passive-command-abilities-authority.v1";
pub const XUI23_PASSIVE_CARRIER_UNAVAILABLE_REASON: &str =
    "EIR-NE-009: canonical UI-open-independent all-country command-ability carrier is absent";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverAbilityAuthority {
    pub executable_sha256: String,
    pub frame_hour: i64,
    pub country_roster_complete: bool,
    pub country_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverAbilityFramePayload {
    pub executable_sha256: String,
    pub producer_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub completeness_record_sha256: String,
    pub source_record_id: String,
    pub authority: ObserverAbilityAuthority,
    pub snapshots: Vec<ObserverAbilityRuntimeSnapshot>,
}

impl ObserverAbilityFramePayload {
    pub fn receipt_sha256(&self) -> String {
        observer_ability_payload_receipt_sha256(self)
    }

    pub fn replay_authority_record(&self) -> serde_json::Value {
        serde_json::json!({
            "schema": XUI23_PRODUCER_AUTHORITY_SCHEMA,
            "payload": self,
            "payload_receipt_sha256": self.receipt_sha256(),
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecodedObserverAbilityAuthority {
    schema: String,
    payload: ObserverAbilityFramePayload,
    payload_receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverAbilityScopeKind {
    ArmyLeader,
    Army,
    ArmyGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverAbilityScope {
    pub kind: ObserverAbilityScopeKind,
    pub stable_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverAbilityContext {
    pub caster: Option<ObserverAbilityScope>,
    pub group_target: Option<ObserverAbilityScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverAbilityLifecycle {
    Dormant,
    Active,
    Cooldown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverAbilityRuntimeRow {
    pub ability_id: String,
    pub caster: Option<ObserverAbilityScope>,
    pub group_target: Option<ObserverAbilityScope>,
    pub definition_order: u32,
    pub target_kind_code: i32,
    pub available: bool,
    pub lifecycle: ObserverAbilityLifecycle,
    pub activation_hour: Option<i64>,
    pub authored_duration_hours: i32,
    pub authored_cooldown_hours: Option<i32>,
    pub duration_remaining_hours: Option<i32>,
    pub cooldown_remaining_hours: Option<i32>,
    pub effective_command_power_cost_fixed: i64,
    pub effective_command_power_cost_scale: i64,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverAbilityRuntimeSnapshot {
    pub country_tag: String,
    pub complete: bool,
    pub context_count: u32,
    pub contexts: Vec<ObserverAbilityContext>,
    pub row_count: usize,
    pub rows: Vec<ObserverAbilityRuntimeRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObserverAbilityPresentation {
    pub ability_id: String,
    pub caster: Option<ObserverAbilityScope>,
    pub group_target: Option<ObserverAbilityScope>,
    pub definition_order: u32,
    pub target_kind_code: i32,
    pub target_kind: String,
    pub available: bool,
    pub lifecycle: ObserverAbilityLifecycle,
    pub activation_hour: Option<i64>,
    pub duration_total: CommandAbilityTimer,
    pub duration_remaining: CommandAbilityTimer,
    pub cooldown_total: CommandAbilityTimer,
    pub cooldown_remaining: CommandAbilityTimer,
    pub cost_command_power_fixed: i64,
    pub cost_command_power_scale: i64,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObserverAbilitySurface {
    pub country_tag: String,
    pub selected_context: ObserverAbilityContext,
    pub rows: Vec<ObserverAbilityPresentation>,
    pub authored_root: &'static str,
    pub pixels: ComposeOutput,
    pub input_region: ObserverAbilityInputRegion,
    pub button_regions: Vec<ObserverAbilityButtonRegion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObserverAbilityInputRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ObserverAbilityInputRegion {
    pub fn contains(self, x: i32, y: i32) -> bool {
        self.width > 0
            && self.height > 0
            && x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverAbilityButtonRegion {
    pub ability_id: String,
    pub caster: Option<ObserverAbilityScope>,
    pub group_target: Option<ObserverAbilityScope>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ObserverAbilityButtonRegion {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.width > 0
            && self.height > 0
            && x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObserverAbilityViewerState {
    pub open: bool,
    pub selected_context: Option<ObserverAbilityContext>,
    pub animation_elapsed_micros: u64,
}

impl ObserverAbilityViewerState {
    pub fn select_context(
        &mut self,
        context: ObserverAbilityContext,
        snapshot: &ObserverAbilityRuntimeSnapshot,
    ) -> std::result::Result<(), ActiveAbilityRejection> {
        if !snapshot.contexts.contains(&context) {
            return Err(ActiveAbilityRejection(
                "viewer-selected ability context is absent from the complete roster".into(),
            ));
        }
        self.selected_context = Some(context);
        Ok(())
    }

    pub fn advance_animation(&mut self, elapsed_micros: u64) {
        self.animation_elapsed_micros = self
            .animation_elapsed_micros
            .saturating_add(elapsed_micros);
    }

    pub fn reset_for_country_switch(&mut self) {
        self.open = false;
        self.selected_context = None;
        self.animation_elapsed_micros = 0;
    }

    pub fn reset_for_seek(&mut self) {
        self.open = false;
        self.selected_context = None;
        self.animation_elapsed_micros = 0;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedObserverAbilityFrame {
    pub source_record_id: String,
    pub country_tag: String,
    pub snapshot: ObserverAbilityRuntimeSnapshot,
    pub rows: Vec<ObserverAbilityPresentation>,
}

pub fn load_admitted_ability_catalog(
    resolver: &AssetResolver,
    sprites: &SpriteRegistry,
    localisation: &Localisation,
) -> UiResult<AdmittedAbilityCatalog> {
    let definitions = load_ordered_active_ability_definitions(resolver, sprites)?;
    admit_ability_catalog(definitions, localisation)
}

fn load_ordered_active_ability_definitions(
    resolver: &AssetResolver,
    sprites: &SpriteRegistry,
) -> UiResult<Vec<AbilityDefinition>> {
    let mut ability_files = resolver.list_files_recursive("common/abilities", Some("txt"))?;
    ability_files.retain(|file| is_direct_ability_file(&file.rel_path));
    ability_files.sort_by(|left, right| {
        source_order_key(&left.rel_path).cmp(&source_order_key(&right.rel_path))
    });
    let mut documents = Vec::with_capacity(ability_files.len());
    for file in &ability_files {
        let document = parse_file_refusing_decode_damage(&file.path, &file.rel_path)?;
        if !document.recoveries.is_empty() {
            return Err(GameUiError::parse(format!(
                "active ability definition recoveries in {}: {:?}",
                file.rel_path, document.recoveries
            )));
        }
        documents.push((file.rel_path.clone(), document));
    }
    ordered_definitions_from_documents(&documents, sprites)
}

#[cfg(test)]
fn definitions_from_documents(
    documents: &[(String, Document)],
    sprites: &SpriteRegistry,
) -> UiResult<BTreeMap<String, AbilityDefinition>> {
    Ok(first_definitions_by_id(
        ordered_definitions_from_documents(documents, sprites)?,
    ))
}

fn first_definitions_by_id(
    definitions: Vec<AbilityDefinition>,
) -> BTreeMap<String, AbilityDefinition> {
    let mut by_id = BTreeMap::new();
    for definition in definitions {
        by_id
            .entry(definition.ability_id.clone())
            .or_insert(definition);
    }
    by_id
}

fn source_order_key(source: &str) -> String {
    normalize_rel_path(source)
}

fn is_direct_ability_file(source: &str) -> bool {
    normalize_rel_path(source)
        .rsplit_once('/')
        .is_some_and(|(parent, _)| parent.eq_ignore_ascii_case("common/abilities"))
}

fn ordered_definitions_from_documents(
    documents: &[(String, Document)],
    sprites: &SpriteRegistry,
) -> UiResult<Vec<AbilityDefinition>> {
    let mut ordered_documents = documents.iter().collect::<Vec<_>>();
    ordered_documents.sort_by(|left, right| {
        source_order_key(&left.0).cmp(&source_order_key(&right.0))
    });
    let mut definitions = Vec::new();
    for (source, document) in ordered_documents {
        if !document.recoveries.is_empty() {
            return Err(GameUiError::parse(format!(
                "active ability definition recoveries in {source}: {:?}",
                document.recoveries
            )));
        }
        for top in document.root.fields() {
            if !top
                .key
                .as_deref()
                .is_some_and(|key| key.eq_ignore_ascii_case("ability"))
            {
                return Err(GameUiError::parse(format!(
                    "unknown active ability top-level construct {:?} in {source}",
                    top.key
                )));
            }
            let block = top.value.as_object().ok_or_else(|| {
                GameUiError::parse(format!("ability root is not an object in {source}"))
            })?;
            for entry in block {
                let ability_id = entry.key.as_deref().ok_or_else(|| {
                    GameUiError::parse(format!("anonymous active ability in {source}"))
                })?;
                let body = entry.value.as_object().ok_or_else(|| {
                    GameUiError::parse(format!(
                        "active ability {ability_id} is not an object in {source}"
                    ))
                })?;
                let value = Value::Object(body.to_vec());
                let required = |key: &str| {
                    value.first_scalar_ci(key).ok_or_else(|| {
                        GameUiError::parse(format!(
                            "active ability {ability_id} lacks {key} in {source}"
                        ))
                    })
                };
                let name_key = required("name")?.to_string();
                let description_key = required("desc")?.to_string();
                let target_kind = required("type")?.to_string();
                let base_cost_fraction = required("cost")?.parse::<f64>().map_err(|_| {
                    GameUiError::parse(format!(
                        "active ability {ability_id} has invalid cost in {source}"
                    ))
                })?;
                let authored_duration_hours =
                    required("duration")?.parse::<i64>().map_err(|_| {
                        GameUiError::parse(format!(
                            "active ability {ability_id} has invalid duration in {source}"
                        ))
                    })?;
                let authored_cooldown_hours = value
                    .first_scalar_ci("cooldown")
                    .map(|raw| {
                        raw.parse::<i64>().map_err(|_| {
                            GameUiError::parse(format!(
                                "active ability {ability_id} has invalid cooldown in {source}"
                            ))
                        })
                    })
                    .transpose()?;
                if !base_cost_fraction.is_finite()
                    || base_cost_fraction < 0.0
                    || authored_duration_hours < -1
                    || authored_cooldown_hours.is_some_and(|hours| hours <= 0)
                {
                    return Err(GameUiError::parse(format!(
                        "active ability {ability_id} has invalid authored bounds in {source}"
                    )));
                }
                let icon = value
                    .first_scalar_ci("icon")
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("GFX_ability_{ability_id}"));
                if sprites.get(&icon).is_none() {
                    return Err(GameUiError::parse(format!(
                        "active ability {ability_id} icon {icon} is unresolved in {source}"
                    )));
                }
                let active_icon = format!("{icon}_animated");
                if sprites.get(&active_icon).is_none() {
                    return Err(GameUiError::parse(format!(
                        "active ability {ability_id} animated icon {active_icon} is unresolved in {source}"
                    )));
                }
                let definition = AbilityDefinition {
                    ability_id: ability_id.to_string(),
                    target_kind,
                    name_key,
                    description_key,
                    icon,
                    active_icon,
                    base_cost_fraction,
                    authored_duration_hours,
                    authored_cooldown_hours,
                };
                definitions.push(definition);
            }
        }
    }
    if definitions.is_empty() {
        return Err(GameUiError::parse(
            "effective active ability catalog is empty",
        ));
    }
    Ok(definitions)
}

fn admit_ability_catalog(
    definitions: Vec<AbilityDefinition>,
    localisation: &Localisation,
) -> UiResult<AdmittedAbilityCatalog> {
    if definitions.is_empty() {
        return Err(GameUiError::parse(
            "effective active ability catalog is empty",
        ));
    }
    let mut abilities = Vec::with_capacity(definitions.len());
    for (authored_order, definition) in definitions.into_iter().enumerate() {
        let authored_order = u32::try_from(authored_order).map_err(|_| {
            GameUiError::parse("effective active ability catalog exceeds u32 authored order")
        })?;
        let name = localisation.get(&definition.name_key).ok_or_else(|| {
            GameUiError::parse(format!(
                "active ability {} name localisation {} is missing",
                definition.ability_id, definition.name_key
            ))
        })?;
        let description = localisation
            .get(&definition.description_key)
            .ok_or_else(|| {
                GameUiError::parse(format!(
                    "active ability {} description localisation {} is missing",
                    definition.ability_id, definition.description_key
                ))
            })?;
        abilities.push(AdmittedAbilityDefinition {
            authored_order,
            ability_id: definition.ability_id,
            target_kind: definition.target_kind,
            name_key: definition.name_key,
            description_key: definition.description_key,
            name: name.to_string(),
            description: description.to_string(),
            icon: definition.icon,
            active_icon: definition.active_icon,
            base_cost_fraction: definition.base_cost_fraction,
            authored_duration_hours: definition.authored_duration_hours,
            authored_cooldown_hours: definition.authored_cooldown_hours,
        });
    }
    Ok(AdmittedAbilityCatalog { abilities })
}

fn timer_hours(
    timer: &CommandAbilityTimer,
    field: &str,
) -> std::result::Result<Option<u32>, ActiveAbilityRejection> {
    match (timer.kind, timer.hours) {
        (CommandAbilityTimerKind::NotStarted | CommandAbilityTimerKind::Indefinite, None) => {
            Ok(None)
        }
        (CommandAbilityTimerKind::Hours, Some(hours)) => Ok(Some(hours)),
        _ => Err(ActiveAbilityRejection(format!(
            "{field} has a kind/value mismatch"
        ))),
    }
}

fn valid_country_tag(tag: &str) -> bool {
    tag.len() == 3
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn valid_observer_scope(scope: &ObserverAbilityScope) -> bool {
    scope.stable_id != 0
}

fn observer_runtime_timer(total: Option<i32>, remaining: Option<i32>) -> CommandAbilityTimer {
    match remaining.or(total) {
        None => CommandAbilityTimer {
            kind: CommandAbilityTimerKind::NotStarted,
            hours: None,
        },
        Some(-1) => CommandAbilityTimer {
            kind: CommandAbilityTimerKind::Indefinite,
            hours: None,
        },
        Some(hours) => CommandAbilityTimer {
            kind: CommandAbilityTimerKind::Hours,
            hours: u32::try_from(hours).ok(),
        },
    }
}

pub fn admit_observer_ability_snapshot(
    authority: &ObserverAbilityAuthority,
    snapshot: &ObserverAbilityRuntimeSnapshot,
    catalog: &AdmittedAbilityCatalog,
    expected_country: &str,
    expected_hour: i64,
) -> std::result::Result<Vec<ObserverAbilityPresentation>, ActiveAbilityRejection> {
    if authority.executable_sha256 != XUI23_EXECUTABLE_SHA256
        || authority.frame_hour != expected_hour
        || !authority.country_roster_complete
        || authority.country_count == 0
    {
        return Err(ActiveAbilityRejection(
            "observer ability authority is not exact-a729 all-country data".into(),
        ));
    }
    if !snapshot.complete
        || snapshot.country_tag != expected_country
        || !valid_country_tag(&snapshot.country_tag)
        || snapshot.context_count as usize != snapshot.contexts.len()
        || snapshot.contexts.is_empty()
        || snapshot.row_count != snapshot.rows.len()
        || snapshot.rows.len() > XUI23_MAX_ROWS
    {
        return Err(ActiveAbilityRejection(
            "observer ability snapshot identity/completeness is invalid".into(),
        ));
    }

    let country_context = ObserverAbilityContext {
        caster: None,
        group_target: None,
    };
    let mut contexts = BTreeSet::new();
    let mut prior_context: Option<&ObserverAbilityContext> = None;
    for context in &snapshot.contexts {
        if [&context.caster, &context.group_target]
            .into_iter()
            .flatten()
            .any(|scope| !valid_observer_scope(scope))
            || prior_context.is_some_and(|prior| prior >= context)
            || !contexts.insert(context)
        {
            return Err(ActiveAbilityRejection(
                "observer ability context roster is not canonical and complete".into(),
            ));
        }
        prior_context = Some(context);
    }
    if !contexts.contains(&country_context) {
        return Err(ActiveAbilityRejection(
            "observer ability context roster lacks the country context".into(),
        ));
    }

    let mut first_definitions = BTreeMap::new();
    for definition in &catalog.abilities {
        first_definitions
            .entry(definition.ability_id.as_str())
            .or_insert(definition);
    }
    if first_definitions.is_empty() {
        return Err(ActiveAbilityRejection(
            "effective observer ability FILE catalog is empty".into(),
        ));
    }

    let mut presentations = Vec::with_capacity(snapshot.rows.len());
    let mut prior_key = None;
    let mut keys = BTreeSet::new();
    let mut context_orders = BTreeSet::new();
    for (index, row) in snapshot.rows.iter().enumerate() {
        let prefix = format!("observer_active_abilities.rows[{index}]");
        let definition = first_definitions
            .get(row.ability_id.as_str())
            .copied()
            .ok_or_else(|| {
                ActiveAbilityRejection(format!(
                    "{prefix}.ability_id has no first-match effective FILE definition"
                ))
            })?;
        let context = ObserverAbilityContext {
            caster: row.caster.clone(),
            group_target: row.group_target.clone(),
        };
        if !contexts.contains(&context)
            || [&row.caster, &row.group_target]
                .into_iter()
                .flatten()
                .any(|scope| !valid_observer_scope(scope))
        {
            return Err(ActiveAbilityRejection(format!(
                "{prefix} names a scope outside the complete context roster"
            )));
        }
        let key = (
            row.ability_id.as_str(),
            row.caster.as_ref(),
            row.group_target.as_ref(),
        );
        if prior_key.is_some_and(|prior| prior >= key) || !keys.insert(key) {
            return Err(ActiveAbilityRejection(format!(
                "{prefix} duplicates or reorders its stable tuple identity"
            )));
        }
        prior_key = Some(key);
        if !context_orders.insert((
            row.caster.as_ref(),
            row.group_target.as_ref(),
            row.definition_order,
        )) {
            return Err(ActiveAbilityRejection(format!(
                "{prefix} duplicates a definition within one evaluator context"
            )));
        }
        let file_duration = i32::try_from(definition.authored_duration_hours).map_err(|_| {
            ActiveAbilityRejection(format!("{prefix} FILE duration exceeds i32"))
        })?;
        let file_cooldown = definition
            .authored_cooldown_hours
            .map(i32::try_from)
            .transpose()
            .map_err(|_| ActiveAbilityRejection(format!("{prefix} FILE cooldown exceeds i32")))?;
        if row.definition_order != definition.authored_order
            || row.authored_duration_hours != file_duration
            || row.authored_cooldown_hours != file_cooldown
            || row.effective_command_power_cost_scale != XUI23_FIXED_SCALE
            || row.disabled_reason.is_some()
            || row.authored_duration_hours < -1
            || row.authored_cooldown_hours.is_some_and(|hours| hours <= 0)
        {
            return Err(ActiveAbilityRejection(format!(
                "{prefix} disagrees with exact FILE/runtime authority"
            )));
        }
        let timer_valid = match row.lifecycle {
            ObserverAbilityLifecycle::Dormant => {
                row.activation_hour.is_none()
                    && row.duration_remaining_hours.is_none()
                    && row.cooldown_remaining_hours.is_none()
            }
            ObserverAbilityLifecycle::Active => row.activation_hour.is_some_and(|activation| {
                expected_hour
                    .checked_sub(activation)
                    .filter(|_| activation >= 0 && activation <= expected_hour)
                    .is_some_and(|elapsed| match row.authored_duration_hours {
                        -1 => {
                            row.duration_remaining_hours.is_none()
                                && row.cooldown_remaining_hours.is_none()
                        }
                        total => i64::from(total)
                            .checked_sub(elapsed)
                            .and_then(|remaining| i32::try_from(remaining).ok())
                            .is_some_and(|remaining| {
                                remaining > 0
                                    && row.duration_remaining_hours == Some(remaining)
                                    && row.cooldown_remaining_hours.is_none()
                            }),
                    })
            }),
            ObserverAbilityLifecycle::Cooldown => {
                row.activation_hour.is_some_and(|activation| {
                    expected_hour
                        .checked_sub(activation)
                        .filter(|_| activation >= 0 && activation <= expected_hour)
                        .is_some_and(|elapsed| {
                            row.authored_duration_hours >= 0
                                && row.duration_remaining_hours.is_none()
                                && row
                                    .authored_cooldown_hours
                                    .zip(row.cooldown_remaining_hours)
                                    .is_some_and(|(total, remaining)| {
                                        i64::from(row.authored_duration_hours)
                                            .checked_add(i64::from(total))
                                            .and_then(|end| end.checked_sub(elapsed))
                                            .and_then(|expected| i32::try_from(expected).ok())
                                            .is_some_and(|expected| {
                                                remaining == expected
                                                    && remaining > 0
                                                    && remaining <= total
                                            })
                                    })
                        })
                })
            }
        };
        if !timer_valid {
            return Err(ActiveAbilityRejection(format!(
                "{prefix} has an invalid lifecycle/timer interval"
            )));
        }

        let duration_total = observer_runtime_timer(Some(row.authored_duration_hours), None);
        let duration_remaining =
            observer_runtime_timer(None, row.duration_remaining_hours);
        let cooldown_total = observer_runtime_timer(row.authored_cooldown_hours, None);
        let cooldown_remaining =
            observer_runtime_timer(None, row.cooldown_remaining_hours);
        presentations.push(ObserverAbilityPresentation {
            ability_id: row.ability_id.clone(),
            caster: row.caster.clone(),
            group_target: row.group_target.clone(),
            definition_order: row.definition_order,
            target_kind_code: row.target_kind_code,
            target_kind: definition.target_kind.clone(),
            available: row.available,
            lifecycle: row.lifecycle,
            activation_hour: row.activation_hour,
            duration_total,
            duration_remaining,
            cooldown_total,
            cooldown_remaining,
            cost_command_power_fixed: row.effective_command_power_cost_fixed,
            cost_command_power_scale: row.effective_command_power_cost_scale,
            name: definition.name.clone(),
            description: definition.description.clone(),
            icon: if row.lifecycle == ObserverAbilityLifecycle::Active {
                definition.active_icon.clone()
            } else {
                definition.icon.clone()
            },
            disabled_reason: None,
        });
    }
    Ok(presentations)
}

pub fn admit_observer_ability_frame(
    frame: &replay_core::WorldFrame,
    admitted_manifest_sha256: &str,
    catalog: &AdmittedAbilityCatalog,
    expected_country: NamedCountry<'_>,
) -> std::result::Result<AdmittedObserverAbilityFrame, ActiveAbilityRejection> {
    let decoded = frame
        .domain(XUI23_PRODUCER_AUTHORITY_FIELD)
        .cloned()
        .ok_or_else(|| ActiveAbilityRejection(XUI23_PASSIVE_CARRIER_UNAVAILABLE_REASON.into()))
        .and_then(|value| {
            serde_json::from_value::<DecodedObserverAbilityAuthority>(value).map_err(|_| {
                ActiveAbilityRejection(
                    "XUI23 passive carrier authority record is malformed".into(),
                )
            })
        })?;
    // The caller says which country this frame is for. A key two ids reach is
    // not one, and every snapshot below is attributed to it.
    if frame.country_key_is_contended(expected_country.tag()) {
        return Err(ActiveAbilityRejection(format!(
            "XUI23 expected country {expected_country:?} is a contended key this hour, so it names no country"
        )));
    }
    let payload = &decoded.payload;
    let frame_identity = observer_ability_replay_frame_identity_sha256(frame)?;
    if decoded.schema != XUI23_PRODUCER_AUTHORITY_SCHEMA
        || payload.authority.frame_hour != frame.hour_index()
        || payload.executable_sha256 != XUI23_EXECUTABLE_SHA256
        || payload.authority.executable_sha256 != XUI23_EXECUTABLE_SHA256
        || payload.producer_manifest_sha256 != admitted_manifest_sha256
        || payload.replay_frame_identity_sha256 != frame_identity
        || !crate::is_lowercase_sha256(&payload.producer_manifest_sha256)
        || !crate::is_lowercase_sha256(&payload.replay_frame_identity_sha256)
        || !crate::is_lowercase_sha256(&payload.completeness_record_sha256)
        || !crate::is_lowercase_sha256(&decoded.payload_receipt_sha256)
        || payload.source_record_id.trim().is_empty()
        || !payload.authority.country_roster_complete
        || payload.authority.country_count == 0
        || payload.snapshots.len() != payload.authority.country_count
        || payload.receipt_sha256() != decoded.payload_receipt_sha256
    {
        return Err(ActiveAbilityRejection(
            "XUI23 passive carrier authority/receipt does not authenticate this frame".into(),
        ));
    }

    // COMPLETE, not partial: the payload's snapshots are compared as an
    // ORDERED EQUALITY against the whole roster and its `country_count` is
    // asserted equal to the roster size. Both are claims about the entire
    // country set, so a roster short by a country the frame carries cannot back
    // them; the refusal names the countries.
    let d18 = frame.d18_country_roster();
    let d18_entries = match d18.complete() {
        Ok(entries) => entries,
        Err(refusal) => {
            return Err(ActiveAbilityRejection(format!(
                "XUI23 passive carrier requires the complete current-hour D18 roster: {refusal}"
            )))
        }
    };
    if d18_entries.len() != payload.authority.country_count
        || d18_entries
            .iter()
            .map(|entry| entry.tag.as_str())
            .ne(payload.snapshots.iter().map(|snapshot| snapshot.country_tag.as_str()))
    {
        return Err(ActiveAbilityRejection(
            "XUI23 passive carrier country roster does not exactly equal the ordered D18 census"
                .into(),
        ));
    }

    let mut prior_country: Option<&str> = None;
    let mut countries = BTreeSet::new();
    let mut selected = None;
    for snapshot in &payload.snapshots {
        if prior_country.is_some_and(|prior| prior >= snapshot.country_tag.as_str())
            || !countries.insert(snapshot.country_tag.as_str())
        {
            return Err(ActiveAbilityRejection(
                "XUI23 passive carrier country roster is duplicated or unordered".into(),
            ));
        }
        prior_country = Some(&snapshot.country_tag);
        let rows = admit_observer_ability_snapshot(
            &payload.authority,
            snapshot,
            catalog,
            &snapshot.country_tag,
            frame.hour_index(),
        )?;
        if snapshot.country_tag == expected_country.tag() {
            selected = Some((snapshot.clone(), rows));
        }
    }
    let (snapshot, rows) = selected.ok_or_else(|| {
        ActiveAbilityRejection(
            "XUI23 passive carrier complete country roster lacks the viewer-selected country"
                .into(),
        )
    })?;
    Ok(AdmittedObserverAbilityFrame {
        source_record_id: payload.source_record_id.clone(),
        country_tag: expected_country.to_string(),
        snapshot,
        rows,
    })
}

pub fn observer_ability_replay_frame_identity_sha256(
    frame: &replay_core::WorldFrame,
) -> std::result::Result<String, ActiveAbilityRejection> {
    frame
        .base_frame_identity_sha256(snapshot::replay_frame_identity::XUI23_BASE_FRAME_IDENTITY_DOMAIN)
        .map_err(|error| ActiveAbilityRejection(error.reason()))
}

fn digest_observer_ability_field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

fn digest_observer_ability_scope(bytes: &mut Vec<u8>, scope: &Option<ObserverAbilityScope>) {
    let Some(scope) = scope else {
        bytes.push(0);
        return;
    };
    bytes.push(match scope.kind {
        ObserverAbilityScopeKind::ArmyLeader => 1,
        ObserverAbilityScopeKind::Army => 2,
        ObserverAbilityScopeKind::ArmyGroup => 3,
    });
    bytes.extend_from_slice(&scope.stable_id.to_le_bytes());
}

fn observer_ability_payload_receipt_sha256(payload: &ObserverAbilityFramePayload) -> String {
    let mut bytes = b"hoi4.xui23.passive-command-abilities.v1\0".to_vec();
    for value in [
        &payload.executable_sha256,
        &payload.producer_manifest_sha256,
        &payload.replay_frame_identity_sha256,
        &payload.completeness_record_sha256,
        &payload.source_record_id,
    ] {
        digest_observer_ability_field(&mut bytes, value.as_bytes());
    }
    bytes.extend_from_slice(&payload.authority.frame_hour.to_le_bytes());
    bytes.push(u8::from(payload.authority.country_roster_complete));
    bytes.extend_from_slice(&(payload.authority.country_count as u64).to_le_bytes());
    bytes.extend_from_slice(&(payload.snapshots.len() as u64).to_le_bytes());
    for snapshot in &payload.snapshots {
        digest_observer_ability_field(&mut bytes, snapshot.country_tag.as_bytes());
        bytes.push(u8::from(snapshot.complete));
        bytes.extend_from_slice(&snapshot.context_count.to_le_bytes());
        bytes.extend_from_slice(&(snapshot.contexts.len() as u64).to_le_bytes());
        for context in &snapshot.contexts {
            digest_observer_ability_scope(&mut bytes, &context.caster);
            digest_observer_ability_scope(&mut bytes, &context.group_target);
        }
        bytes.extend_from_slice(&(snapshot.row_count as u64).to_le_bytes());
        bytes.extend_from_slice(&(snapshot.rows.len() as u64).to_le_bytes());
        for row in &snapshot.rows {
            digest_observer_ability_field(&mut bytes, row.ability_id.as_bytes());
            digest_observer_ability_scope(&mut bytes, &row.caster);
            digest_observer_ability_scope(&mut bytes, &row.group_target);
            bytes.extend_from_slice(&row.definition_order.to_le_bytes());
            bytes.extend_from_slice(&row.target_kind_code.to_le_bytes());
            bytes.push(u8::from(row.available));
            bytes.push(match row.lifecycle {
                ObserverAbilityLifecycle::Dormant => 0,
                ObserverAbilityLifecycle::Active => 1,
                ObserverAbilityLifecycle::Cooldown => 2,
            });
            for value in [
                row.activation_hour,
                row.duration_remaining_hours.map(i64::from),
                row.cooldown_remaining_hours.map(i64::from),
            ] {
                bytes.push(u8::from(value.is_some()));
                bytes.extend_from_slice(&value.unwrap_or_default().to_le_bytes());
            }
            bytes.extend_from_slice(&row.authored_duration_hours.to_le_bytes());
            bytes.push(u8::from(row.authored_cooldown_hours.is_some()));
            bytes.extend_from_slice(&row.authored_cooldown_hours.unwrap_or_default().to_le_bytes());
            bytes.extend_from_slice(&row.effective_command_power_cost_fixed.to_le_bytes());
            bytes.extend_from_slice(&row.effective_command_power_cost_scale.to_le_bytes());
            bytes.push(u8::from(row.disabled_reason.is_some()));
            if let Some(reason) = &row.disabled_reason {
                digest_observer_ability_field(&mut bytes, reason.as_bytes());
            }
        }
    }
    Sha256::hex(&bytes)
}

fn presentation_matches_context(
    row: &ObserverAbilityPresentation,
    context: &ObserverAbilityContext,
) -> bool {
    row.caster == context.caster && row.group_target == context.group_target
}

pub fn compose_observer_ability_surface(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    cache: &mut UiAssetCache,
    country_tag: &str,
    selected_context: &ObserverAbilityContext,
    rows: &[ObserverAbilityPresentation],
    animation_elapsed_micros: u64,
    viewport_width: usize,
    viewport_height: usize,
) -> UiResult<Option<ObserverAbilitySurface>> {
    let mut rows = rows
        .iter()
        .filter(|row| presentation_matches_context(row, selected_context))
        .cloned()
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.definition_order);
    if rows.is_empty() {
        return Ok(None);
    }

    let resolved = resolver.resolve_file(ABILITY_LIST_GUI)?;
    let document = parse_file_refusing_decode_damage(&resolved.path, &resolved.rel_path)?;
    if !document.recoveries.is_empty() {
        return Err(GameUiError::parse(format!(
            "ability-list GUI recoveries in {}: {:?}",
            resolved.rel_path, document.recoveries
        )));
    }
    let roots = parse_gui_document(&document);
    let mut window = unique_ability_element(&roots, ABILITY_LIST_ROOT)?.clone();
    let button_template = unique_ability_element(&roots, ABILITY_BUTTON_TEMPLATE)?.clone();
    let mid_template = unique_ability_element(&roots, ABILITY_MID_TEMPLATE)?.clone();
    if !window
        .element_type
        .eq_ignore_ascii_case("containerWindowType")
        || !button_template
            .element_type
            .eq_ignore_ascii_case("containerWindowType")
    {
        return Err(GameUiError::parse(
            "ability-list authored roots have unexpected element kinds",
        ));
    }
    let supported_window_shape = window.children.len() == 2
        && window.children[0].name.eq_ignore_ascii_case("bg_begin")
        && window.children[1].name.eq_ignore_ascii_case("bg_end");
    if !supported_window_shape {
        return Err(GameUiError::parse(format!(
            "ability-list effective window has unsupported authored child shape: {:?}",
            window
                .children
                .iter()
                .map(|child| child.name.as_str())
                .collect::<Vec<_>>()
        )));
    }
    let begin = unique_ability_element(&window.children, "bg_begin")?.clone();
    let mut end = unique_ability_element(&window.children, "bg_end")?.clone();
    let base_width = authored_axis(
        window
            .size
            .as_ref()
            .ok_or_else(|| GameUiError::parse("abilitylist_window has no authored size"))?,
        true,
        "abilitylist_window.width",
    )?;
    let begin_width = ability_sprite_width(resolver, registry, &begin)?;
    let mid_width = ability_sprite_width(resolver, registry, &mid_template)?;
    let button_width = authored_axis(
        button_template.size.as_ref().ok_or_else(|| {
            GameUiError::parse("abilitylist_button_container has no authored size")
        })?,
        true,
        "abilitylist_button_container.width",
    )?;
    if begin_width <= 0 || mid_width <= 0 || button_width <= 0 {
        return Err(GameUiError::parse(
            "ability-list authored clone extents are not positive",
        ));
    }
    let button_x = authored_element_axis(&button_template, true, "ability button x")?;
    let button_y = authored_element_axis(&button_template, false, "ability button y")?;
    let middle_count = rows.len().saturating_sub(4);
    let middle_count_i32 = i32::try_from(middle_count)
        .map_err(|_| GameUiError::parse("ability-list middle clone count exceeds i32"))?;
    let growth = mid_width
        .checked_mul(middle_count_i32)
        .ok_or_else(|| GameUiError::parse("ability-list authored width overflow"))?;
    set_authored_axis(
        window.size.as_mut().expect("window size checked"),
        true,
        base_width
            .checked_add(growth)
            .ok_or_else(|| GameUiError::parse("ability-list window width overflow"))?,
    );
    let end_y = authored_element_axis(&end, false, "ability end-cap y")?;
    set_element_position(
        &mut end,
        begin_width
            .checked_add(growth)
            .ok_or_else(|| GameUiError::parse("ability-list end-cap position overflow"))?,
        end_y,
    );

    let mut children = vec![begin];
    for ordinal in 0..middle_count {
        let mut middle = mid_template.clone();
        rename_ability_tree(&mut middle, &format!("__xui23_mid_{ordinal}"));
        let ordinal = i32::try_from(ordinal)
            .map_err(|_| GameUiError::parse("ability-list middle ordinal exceeds i32"))?;
        set_element_position(
            &mut middle,
            begin_width
                .checked_add(
                    mid_width
                        .checked_mul(ordinal)
                        .ok_or_else(|| GameUiError::parse("ability middle position overflow"))?,
                )
                .ok_or_else(|| GameUiError::parse("ability middle position overflow"))?,
            authored_element_axis(&mid_template, false, "ability middle y")?,
        );
        children.push(middle);
    }
    children.push(end);

    let mut options = ComposeOptions::new(viewport_width, viewport_height)
        .with_root_size(viewport_width, viewport_height);
    options.animation_elapsed_micros = animation_elapsed_micros;
    options.xui23_require_scoped_progress = true;
    for (ordinal, row) in rows.iter().enumerate() {
        let mut button = button_template.clone();
        let suffix = format!("__xui23_row_{ordinal}");
        rename_ability_tree(&mut button, &suffix);
        let ordinal = i32::try_from(ordinal)
            .map_err(|_| GameUiError::parse("ability-list row ordinal exceeds i32"))?;
        set_element_position(
            &mut button,
            button_x
                .checked_add(
                    button_width
                        .checked_mul(ordinal)
                        .ok_or_else(|| GameUiError::parse("ability button position overflow"))?,
                )
                .ok_or_else(|| GameUiError::parse("ability button position overflow"))?,
            button_y,
        );
        let icon_name = format!("abilitylist_button{suffix}");
        let active_name = format!("abilitylist_active_bg{suffix}");
        let cooldown_name = format!("abilitylist_cooldown_bg{suffix}");
        options
            .sprite_name_overrides
            .insert(icon_name.clone(), row.icon.clone());
        options
            .xui23_buttonstate_elements
            .insert(icon_name.clone());
        options.xui23_progress_elements.insert(active_name.clone());
        options
            .xui23_progress_elements
            .insert(cooldown_name.clone());
        if !row.available {
            options.disabled_button_elements.insert(icon_name);
        }
        match (
            row.lifecycle,
            runtime_timer_fraction(&row.duration_total, &row.duration_remaining)?,
            runtime_timer_fraction(&row.cooldown_total, &row.cooldown_remaining)?,
        ) {
            (ObserverAbilityLifecycle::Active, Some(progress), _) => {
                options.progress_overrides.insert(active_name, 1.0 - progress);
                options.hidden_elements.insert(cooldown_name);
            }
            (ObserverAbilityLifecycle::Cooldown, _, Some(progress)) => {
                options.progress_overrides.insert(cooldown_name, progress);
                options.hidden_elements.insert(active_name);
            }
            _ => {
                options.hidden_elements.insert(active_name);
                options.hidden_elements.insert(cooldown_name);
            }
        }
        children.push(button);
    }
    window.children = children;
    let pixels = Compositor::with_cache(resolver, registry, cache).compose(&window, &options);
    if !pixels.report.missing_sprites.is_empty()
        || !pixels.report.undecodable_sprites.is_empty()
    {
        return Err(GameUiError::parse(format!(
            "ability-list composition rejected unresolved authored assets: missing_sprites={:?}; undecodable_sprites={:?}",
            pixels.report.missing_sprites, pixels.report.undecodable_sprites
        )));
    }
    let root_matches = pixels
        .debug_rects
        .iter()
        .filter(|rect| rect.name == ABILITY_LIST_ROOT)
        .collect::<Vec<_>>();
    let [root_rect] = root_matches.as_slice() else {
        return Err(GameUiError::parse(
            "ability-list composed root input region is missing or duplicated",
        ));
    };
    if root_rect.w <= 0 || root_rect.h <= 0 {
        return Err(GameUiError::parse(
            "ability-list composed root input region is not positive",
        ));
    }
    let input_region = ObserverAbilityInputRegion {
        x: root_rect.x,
        y: root_rect.y,
        width: root_rect.w,
        height: root_rect.h,
    };
    let mut button_regions = Vec::with_capacity(rows.len());
    for (ordinal, row) in rows.iter().enumerate() {
        let name = format!("abilitylist_button__xui23_row_{ordinal}");
        let matches = pixels
            .debug_rects
            .iter()
            .filter(|rect| rect.name == name)
            .collect::<Vec<_>>();
        let [rect] = matches.as_slice() else {
            return Err(GameUiError::parse(format!(
                "ability-list composed button region {name} is missing or duplicated"
            )));
        };
        if rect.w <= 0 || rect.h <= 0 {
            return Err(GameUiError::parse(format!(
                "ability-list composed button region {name} is not positive"
            )));
        }
        button_regions.push(ObserverAbilityButtonRegion {
            ability_id: row.ability_id.clone(),
            caster: row.caster.clone(),
            group_target: row.group_target.clone(),
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
        });
    }
    Ok(Some(ObserverAbilitySurface {
        country_tag: country_tag.to_string(),
        selected_context: selected_context.clone(),
        rows,
        authored_root: ABILITY_LIST_ROOT,
        pixels,
        input_region,
        button_regions,
    }))
}

fn runtime_timer_fraction(
    total: &CommandAbilityTimer,
    remaining: &CommandAbilityTimer,
) -> UiResult<Option<f32>> {
    match (
        timer_hours(total, "ability timer total"),
        timer_hours(remaining, "ability timer remaining"),
    ) {
        (Ok(Some(total)), Ok(Some(remaining))) if total != 0 && remaining <= total => {
            Ok(Some(remaining as f32 / total as f32))
        }
        (Ok(_), Ok(_)) => Ok(None),
        (Err(error), _) | (_, Err(error)) => Err(GameUiError::parse(error.0)),
    }
}

fn unique_ability_element<'a>(elements: &'a [GuiElement], name: &str) -> UiResult<&'a GuiElement> {
    let matches = elements
        .iter()
        .filter(|element| element.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [element] => Ok(*element),
        _ => Err(GameUiError::parse(format!(
            "ability-list requires exactly one element named {name}, found {}",
            matches.len()
        ))),
    }
}

fn ability_sprite_width(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    element: &GuiElement,
) -> UiResult<i32> {
    let sprite_name = element
        .sprite_refs
        .first()
        .ok_or_else(|| GameUiError::parse(format!("{} has no authored sprite", element.name)))?;
    let sprite = registry.get(sprite_name).ok_or_else(|| {
        GameUiError::missing(format!("ability-list sprite {sprite_name} is unresolved"))
    })?;
    let decoded = resolver.decode_sprite(sprite)?;
    i32::try_from(
        decoded
            .frames
            .first()
            .ok_or_else(|| GameUiError::parse(format!("{sprite_name} has no decoded frame")))?
            .width,
    )
    .map_err(|_| GameUiError::parse(format!("{sprite_name} width exceeds i32")))
}

fn authored_axis(pair: &MeasurePair, x: bool, field: &str) -> UiResult<i32> {
    let measure = if x { &pair.x } else { &pair.y };
    if measure.is_percent {
        return Err(GameUiError::parse(format!(
            "{field} is not a fixed authored pixel measure"
        )));
    }
    measure
        .raw
        .parse::<i32>()
        .map_err(|_| GameUiError::parse(format!("{field} is not an i32 pixel measure")))
}

fn authored_element_axis(element: &GuiElement, x: bool, field: &str) -> UiResult<i32> {
    authored_axis(
        element
            .position
            .as_ref()
            .ok_or_else(|| GameUiError::parse(format!("{field} has no authored position")))?,
        x,
        field,
    )
}

fn fixed_measure(value: i32) -> Measure {
    Measure {
        raw: value.to_string(),
        value: Some(value.to_string()),
        is_percent: false,
    }
}

fn set_authored_axis(pair: &mut MeasurePair, x: bool, value: i32) {
    if x {
        pair.x = fixed_measure(value);
    } else {
        pair.y = fixed_measure(value);
    }
}

fn set_element_position(element: &mut GuiElement, x: i32, y: i32) {
    element.position = Some(MeasurePair {
        x: fixed_measure(x),
        y: fixed_measure(y),
    });
}

fn rename_ability_tree(element: &mut GuiElement, suffix: &str) {
    element.name.push_str(suffix);
    for child in &mut element.children {
        rename_ability_tree(child, suffix);
    }
}

#[cfg(test)]
mod tests {
    /// A TAG this frame's census admits. A test that means to name a country says
    /// so here; a test about a contended key asks `named_country` and gets `None`.
    fn named<'tag>(
        frame: &'tag replay_core::WorldFrame,
        tag: &'tag str,
    ) -> replay_core::NamedCountry<'tag> {
        frame.named_country(tag).expect("census-admitted tag")
    }

    use super::*;
    use crate::paradox_text::parse_str;
    use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
    use serde_json::Map;
    fn definition() -> AbilityDefinition {
        AbilityDefinition {
            ability_id: "force_attack".into(),
            target_kind: "army_leader".into(),
            name_key: "ABILITY_FORCE_ATTACK".into(),
            description_key: "ABILITY_FORCE_ATTACK_DESC".into(),
            icon: "GFX_ability_force_attack".into(),
            active_icon: "GFX_ability_force_attack_animated".into(),
            base_cost_fraction: 0.22,
            authored_duration_hours: 168,
            authored_cooldown_hours: None,
        }
    }

    fn ability_document(entries: &str) -> Document {
        parse_str(&format!("ability = {{ {entries} }}"))
    }

    fn resolved_sprites(ability_ids: &[&str], include_animated: bool) -> SpriteRegistry {
        let mut definitions = String::from("spriteTypes = {");
        for ability_id in ability_ids {
            definitions.push_str(&format!(
                "spriteType = {{ name = GFX_ability_{ability_id} texturefile = \"gfx/{ability_id}.dds\" }}"
            ));
            if include_animated {
                definitions.push_str(&format!(
                    "frameAnimatedSpriteType = {{ name = GFX_ability_{ability_id}_animated texturefile = \"gfx/{ability_id}_strip.dds\" }}"
                ));
            }
        }
        definitions.push('}');
        let mut sprites = SpriteRegistry::new();
        sprites.add_document("interface/test_abilities.gfx", &parse_str(&definitions));
        sprites
    }

    fn localisation(entries: &[(&str, &str)]) -> Localisation {
        let mut localisation = Localisation::default();
        localisation.entries.extend(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );
        localisation
    }

    fn definition_text(ability_id: &str, name_key: &str, description_key: &str) -> String {
        format!(
            "{ability_id} = {{ name = {name_key} desc = {description_key} type = army_leader cost = 0.1 duration = 24 }}"
        )
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        path.push(format!("game_ui_active_abilities_{label}_{nonce}"));
        path
    }

    struct Xui23FrameSource {
        hours: [i64; 1],
        rest: Map<String, serde_json::Value>,
    }

    impl ReplaySource for Xui23FrameSource {
        fn game_id(&self) -> &str {
            "xui23-passive-authority"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: {
                    let mut rest = self.rest.clone();
                    if replay_core::COUNTRY_COMPOSED_MEMBERS
                        .iter()
                        .any(|member| rest.contains_key(*member))
                    {
                        rest.entry("tag_db_id_index").or_insert_with(
                            || serde_json::json!({"country_composition_version": 2}),
                        );
                    }
                    rest
                },
            })
        }
    }

    fn decoded_xui23_frame(rest: Map<String, serde_json::Value>) -> std::sync::Arc<replay_core::WorldFrame> {
        WorldTimeline::open(Box::new(Xui23FrameSource {
            hours: [1_000],
            rest,
        }))
        .unwrap()
        .state_at(1_000)
        .unwrap()
    }

    fn authenticated_observer_payload() -> (
        ObserverAbilityFramePayload,
        std::sync::Arc<replay_core::WorldFrame>,
    ) {
        let mut base = Map::new();
        base.insert(
            "countries".into(),
            serde_json::json!({"GER": {"_r2_tag_db_id": 73}}),
        );
        let empty_frame = decoded_xui23_frame(base.clone());
        let frame_identity =
            observer_ability_replay_frame_identity_sha256(empty_frame.as_ref()).unwrap();
        let payload = ObserverAbilityFramePayload {
            executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
            producer_manifest_sha256: "cd".repeat(32),
            replay_frame_identity_sha256: frame_identity,
            completeness_record_sha256: "ef".repeat(32),
            source_record_id: "xui23:test:1000:all-countries".into(),
            authority: ObserverAbilityAuthority {
                executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
                frame_hour: 1_000,
                country_roster_complete: true,
                country_count: 1,
            },
            snapshots: vec![observer_snapshot()],
        };
        base.insert(
            XUI23_PRODUCER_AUTHORITY_FIELD.into(),
            payload.replay_authority_record(),
        );
        (payload, decoded_xui23_frame(base))
    }

    fn solid_tga(width: u16, height: u16, rgba: [u8; 4]) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        data[2] = 2;
        data[12..14].copy_from_slice(&width.to_le_bytes());
        data[14..16].copy_from_slice(&height.to_le_bytes());
        data[16] = 32;
        data[17] = 0x28;
        for _ in 0..usize::from(width) * usize::from(height) {
            data.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        data
    }

    #[must_use = "the guards must be held for the length of the test"]
    fn write_synthetic_ability_surface(root: &std::path::Path) -> Vec<std::fs::File> {
        std::fs::create_dir_all(root.join("interface")).unwrap();
        std::fs::create_dir_all(root.join("gfx/xui23")).unwrap();
        std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
        for (name, source) in [
            (
                "buttonstate.shader",
                include_str!("../../replay_app/tests/fixtures/exact_a729/buttonstate.shader"),
            ),
            (
                "buttonstate.fxh",
                include_str!("../../replay_app/tests/fixtures/exact_a729/buttonstate.fxh"),
            ),
            (
                "sprite_animation.fxh",
                include_str!("../../replay_app/tests/fixtures/exact_a729/sprite_animation.fxh"),
            ),
            (
                "progress.shader",
                include_str!("../../replay_app/tests/fixtures/exact_a729/progress.shader"),
            ),
        ] {
            std::fs::write(root.join("gfx/FX").join(name), source.replace("\r\n", "\n"))
                .unwrap();
        }
        std::fs::write(
            root.join("interface/abilitylist.gui"),
            r#"guiTypes = {
                iconType = { position = { x = 12 y = 4 } name = abilitylist_bg_mid spriteType = GFX_ability_list_mid }
                containerWindowType = {
                    name = abilitylist_button_container position = { x = 8 y = 2 } size = { width = 12 height = 16 }
                    iconType = { position = { x = 0 y = 0 } name = abilitylist_active_bg spriteType = GFX_ability_active_bg }
                    iconType = { position = { x = 0 y = 0 } name = abilitylist_cooldown_bg spriteType = GFX_ability_cooldown_bg }
                    buttonType = { position = { x = 0 y = 0 } name = abilitylist_button spriteType = GFX_ability_icon_default }
                }
                containerWindowType = {
                    name = abilitylist_window position = { x = 0 y = 0 } size = { width = 40 height = 16 }
                    iconType = { position = { x = 0 y = 0 } name = bg_begin spriteType = GFX_ability_list_begin }
                    iconType = { position = { x = 24 y = 0 } name = bg_end spriteType = GFX_ability_list_end }
                }
            }"#,
        )
        .unwrap();
        let sprites = [
            ("GFX_ability_list_begin", "begin.tga"),
            ("GFX_ability_list_mid", "mid.tga"),
            ("GFX_ability_list_end", "end.tga"),
            ("GFX_ability_icon_default", "default.tga"),
            ("GFX_ability_force_attack", "ability.tga"),
        ];
        let mut gfx = String::from("spriteTypes = {");
        for (name, file) in sprites {
            gfx.push_str(&format!(
                " spriteType = {{ name = {name} texturefile = \"gfx/xui23/{file}\" }}"
            ));
            std::fs::write(root.join("gfx/xui23").join(file), solid_tga(8, 8, [80, 120, 180, 255])).unwrap();
        }
        for (file, rgba) in [
            ("active.tga", [220, 40, 20, 255]),
            ("active_bg.tga", [20, 40, 80, 255]),
            ("cooldown.tga", [40, 200, 60, 255]),
            ("cooldown_bg.tga", [30, 30, 30, 255]),
        ] {
            std::fs::write(root.join("gfx/xui23").join(file), solid_tga(8, 8, rgba)).unwrap();
        }
        gfx.push_str(
            r#" progressbarType = {
                name = GFX_ability_active_bg
                textureFile1 = "gfx/xui23/active.tga"
                textureFile2 = "gfx/xui23/active_bg.tga"
                effectFile = "gfx/FX/progress.lua"
                horizontal = no
            }
            progressbarType = {
                name = GFX_ability_cooldown_bg
                textureFile1 = "gfx/xui23/cooldown.tga"
                textureFile2 = "gfx/xui23/cooldown_bg.tga"
                effectFile = "gfx/FX/progress.lua"
                horizontal = no
            }"#,
        );
        std::fs::write(
            root.join("gfx/xui23/ability_active.tga"),
            solid_tga(8, 8, [80, 120, 180, 255]),
        )
        .unwrap();
        gfx.push_str(
            r#" spriteType = {
                name = GFX_ability_force_attack_animated
                texturefile = "gfx/xui23/ability_active.tga"
                effectFile = "gfx/FX/buttonstate.lua"
                animation = {
                    animationmaskfile = "gfx/xui23/ability_active.tga"
                    animationtexturefile = "gfx/xui23/ability_active.tga"
                    animationrotation = 0.0
                    animationlooping = yes
                    animationtime = 26.0
                    animationdelay = 0.0
                    animationblendmode = "add"
                    animationtype = "scrolling"
                    animationrotationoffset = { x = 0.0 y = 0.0 }
                    animationtexturescale = { x = 1.0 y = 1.0 }
                }
            }"#,
        );
        gfx.push_str(" }");
        std::fs::write(root.join("interface/abilitylist.gfx"), gfx).unwrap();
        scanner_guards(root)
    }

    fn scanner_guards(root: &std::path::Path) -> Vec<std::fs::File> {
        fn walk(directory: &std::path::Path, guards: &mut Vec<std::fs::File>) {
            let Ok(entries) = std::fs::read_dir(directory) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => walk(&path, guards),
                    Ok(kind) if kind.is_file() => {
                        if let Ok(file) = portable_test_fs_retry::open(&path) {
                            guards.push(file);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut guards = Vec::new();
        walk(root, &mut guards);
        guards
    }

    #[test]
    fn effective_definition_catalog_parses_and_resolves_icons() {
        let ability_doc = parse_str(
            r#"ability = {
                force_attack = {
                    name = ABILITY_FORCE_ATTACK
                    desc = ABILITY_FORCE_ATTACK_DESC
                    type = army_leader
                    cost = 0.22
                    duration = 168
                }
            }"#,
        );
        let gfx_doc = parse_str(
            r#"spriteTypes = {
                spriteType = {
                    name = GFX_ability_force_attack
                    texturefile = "gfx/interface/abilitylist/ability_force_attack.dds"
                }
                frameAnimatedSpriteType = {
                    name = GFX_ability_force_attack_animated
                    texturefile = "gfx/interface/abilitylist/ability_force_attack_strip.dds"
                }
            }"#,
        );
        let mut sprites = SpriteRegistry::new();
        sprites.add_document("interface/abilitylist.gfx", &gfx_doc);
        let catalog = definitions_from_documents(
            &[("common/abilities/generic.txt".into(), ability_doc)],
            &sprites,
        )
        .expect("effective ability catalog");
        let row = catalog.get("force_attack").expect("force attack");
        assert_eq!(row.target_kind, "army_leader");
        assert_eq!(row.icon, "GFX_ability_force_attack");
        assert_eq!(row.active_icon, "GFX_ability_force_attack_animated");
        assert_eq!(row.base_cost_fraction, 0.22);
        assert_eq!(row.authored_duration_hours, 168);
    }

    #[test]
    fn admitted_catalog_preserves_authored_order_and_reloads_stably() {
        let documents = vec![
            (
                "common\\abilities\\B.txt".into(),
                ability_document(&definition_text("third", "THIRD", "THIRD_DESC")),
            ),
            (
                "./common/abilities/a.txt".into(),
                ability_document(&format!(
                    "{} {}",
                    definition_text("first", "FIRST", "FIRST_DESC"),
                    definition_text("second", "SECOND", "SECOND_DESC")
                )),
            ),
        ];
        let sprites = resolved_sprites(&["first", "second", "third"], true);
        let localisation = localisation(&[
            ("FIRST", "First"),
            ("FIRST_DESC", "First description"),
            ("SECOND", "Second"),
            ("SECOND_DESC", "Second description"),
            ("THIRD", "Third"),
            ("THIRD_DESC", "Third description"),
        ]);
        let load = || {
            admit_ability_catalog(
                ordered_definitions_from_documents(&documents, &sprites)
                    .expect("ordered definitions"),
                &localisation,
            )
            .expect("admitted catalog")
        };
        let first = load();
        let second = load();
        assert_eq!(first, second);
        assert_eq!(
            first
                .abilities
                .iter()
                .map(|ability| (ability.authored_order, ability.ability_id.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "third"), (1, "first"), (2, "second")]
        );
    }

    #[test]
    fn admitted_catalog_uses_localised_text_not_localisation_keys() {
        let definitions = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                ability_document(&definition_text("x", "X_NAME_KEY", "X_DESC_KEY")),
            )],
            &resolved_sprites(&["x"], true),
        )
        .expect("definition");
        let catalog = admit_ability_catalog(
            definitions,
            &localisation(&[
                ("X_NAME_KEY", "Viewer-localised name"),
                ("X_DESC_KEY", "Viewer-localised description"),
            ]),
        )
        .expect("catalog");
        assert_eq!(catalog.abilities[0].name, "Viewer-localised name");
        assert_eq!(
            catalog.abilities[0].description,
            "Viewer-localised description"
        );
        assert_ne!(catalog.abilities[0].name, catalog.abilities[0].name_key);
        assert_ne!(
            catalog.abilities[0].description,
            catalog.abilities[0].description_key
        );
    }

    #[test]
    fn repeated_ability_ids_follow_engine_vector_and_first_lookup_laws() {
        let documents = [
            (
                "common/abilities/a.txt".into(),
                ability_document(&definition_text("x", "FIRST", "FIRST_DESC")),
            ),
            (
                "common/abilities/b.txt".into(),
                ability_document(&definition_text("x", "SECOND", "SECOND_DESC")),
            ),
        ];
        let definitions = ordered_definitions_from_documents(
            &documents,
            &resolved_sprites(&["x"], true),
        )
        .expect("the engine appends both repeated tokens");
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].name_key, "FIRST");
        assert_eq!(definitions[1].name_key, "SECOND");
        let keyed = first_definitions_by_id(definitions.clone());
        assert_eq!(keyed.len(), 1);
        assert_eq!(keyed["x"].name_key, "FIRST");
        let catalog = admit_ability_catalog(
            definitions,
            &localisation(&[
                ("FIRST", "First"),
                ("FIRST_DESC", "First description"),
                ("SECOND", "Second"),
                ("SECOND_DESC", "Second description"),
            ]),
        )
        .expect("ordered catalog preserves both engine definitions");
        assert_eq!(
            catalog
                .abilities
                .iter()
                .map(|ability| (ability.authored_order, ability.name.as_str()))
                .collect::<Vec<_>>(),
            [(0, "First"), (1, "Second")]
        );
    }

    #[test]
    fn effective_vfs_same_path_override_and_mod_addition_feed_catalog() {
        let root = temp_root("mod_override");
        let mod_root = root.join("mod");
        let vanilla_root = root.join("vanilla");
        std::fs::create_dir_all(mod_root.join("common/abilities/nested"))
            .expect("create mod ability directories");
        std::fs::create_dir_all(vanilla_root.join("common/abilities"))
            .expect("create vanilla ability directory");

        std::fs::write(
            vanilla_root.join("common/abilities/shared.txt"),
            format!(
                "ability = {{ {} }}",
                definition_text("shadowed_definition", "SHADOWED", "SHADOWED_DESC")
            ),
        )
        .expect("write lower-precedence same-path definition");
        std::fs::write(
            vanilla_root.join("common/abilities/retained.txt"),
            format!(
                "ability = {{ {} }}",
                definition_text("vanilla_retained", "RETAINED", "RETAINED_DESC")
            ),
        )
        .expect("write retained lower-precedence definition");
        std::fs::write(
            mod_root.join("common/abilities/shared.txt"),
            format!(
                "ability = {{ {} }}",
                definition_text("same_path_winner", "WINNER", "WINNER_DESC")
            ),
        )
        .expect("write higher-precedence same-path definition");
        std::fs::write(
            mod_root.join("common/abilities/added.txt"),
            format!(
                "ability = {{ {} }}",
                definition_text("mod_added", "ADDED", "ADDED_DESC")
            ),
        )
        .expect("write mod-added definition");
        std::fs::write(
            mod_root.join("common/abilities/nested/ignored.txt"),
            format!(
                "ability = {{ {} }}",
                definition_text("nested_definition", "NESTED", "NESTED_DESC")
            ),
        )
        .expect("write nested definition");

        let resolver = AssetResolver::new(vec![mod_root, vanilla_root]);
        let sprites = resolved_sprites(
            &[
                "shadowed_definition",
                "vanilla_retained",
                "same_path_winner",
                "mod_added",
                "nested_definition",
            ],
            true,
        );
        let localisation = localisation(&[
            ("SHADOWED", "Shadowed"),
            ("SHADOWED_DESC", "Shadowed description"),
            ("RETAINED", "Retained"),
            ("RETAINED_DESC", "Retained description"),
            ("WINNER", "Winner"),
            ("WINNER_DESC", "Winner description"),
            ("ADDED", "Added"),
            ("ADDED_DESC", "Added description"),
            ("NESTED", "Nested"),
            ("NESTED_DESC", "Nested description"),
        ]);
        let catalog = load_admitted_ability_catalog(&resolver, &sprites, &localisation)
            .expect("layered effective-VFS catalog");
        assert_eq!(
            catalog
                .abilities
                .iter()
                .map(|ability| ability.ability_id.as_str())
                .collect::<Vec<_>>(),
            ["mod_added", "vanilla_retained", "same_path_winner"]
        );
        assert_eq!(catalog.abilities[2].name, "Winner");
        assert!(!catalog
            .abilities
            .iter()
            .any(|ability| ability.ability_id == "shadowed_definition"));
        assert!(!catalog
            .abilities
            .iter()
            .any(|ability| ability.ability_id == "nested_definition"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR"]
    fn installed_effective_vfs_admits_the_file_derived_ability_catalog() {
        let game_dir = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR"))
                .expect("BATTLE_PLAN_GAME_DIR must be set for the installed ability catalog gate"),
        );
        let user_dir = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR"))
                .expect("BATTLE_PLAN_USER_DIR must be set for the installed ability catalog gate"),
        );
        let discovered = map_viewer::GameFileResolver::from_dlc_load(&game_dir, user_dir)
            .expect("construct launcher-equivalent installed effective VFS");
        let mut required_paths = std::collections::BTreeSet::new();
        for (directory, extension) in [
            ("common/abilities", "txt"),
            ("interface", "gfx"),
            ("localisation/english", "yml"),
        ] {
            required_paths.extend(
                discovered
                    .resolve_dir_files_recursive(directory, Some(extension))
                    .expect("discover installed ability-catalog dependencies")
                    .into_iter()
                    .map(|(logical_path, _)| logical_path),
            );
        }
        let required = required_paths
            .into_iter()
            .map(|logical_path| {
                let resolved = discovered
                    .resolve_required_with_provenance(
                        &logical_path,
                        "installed ability-catalog dependency",
                    )
                    .expect("hash an installed ability-catalog dependency");
                map_viewer::RequiredGameContent {
                    logical_path: resolved.logical_path,
                    sha256: resolved.sha256,
                    byte_len: resolved.byte_len,
                }
            })
            .collect::<Vec<_>>();
        let restricted = discovered
            .restrict_to(required)
            .expect("restrict installed effective VFS to catalog dependencies");
        let resolver = AssetResolver::from_restricted_game_files(&restricted)
            .expect("adapt installed effective VFS for ability catalog");
        let mut sprites = SpriteRegistry::new();
        for file in resolver
            .list_files_recursive("interface", Some("gfx"))
            .expect("discover effective interface sprite definitions")
        {
            sprites
                .parse_file(&file.path, &file.rel_path)
                .expect("parse an effective interface sprite definition");
        }
        let localisation =
            Localisation::load_english(&resolver).expect("load effective English localisation");
        let catalog = load_admitted_ability_catalog(&resolver, &sprites, &localisation)
            .expect("the installed effective ability catalog must admit");
        assert!(!catalog.abilities.is_empty());
        assert_eq!(
            catalog
                .abilities
                .iter()
                .map(|ability| ability.authored_order)
                .collect::<Vec<_>>(),
            (0..u32::try_from(catalog.abilities.len()).expect("catalog length fits u32"))
                .collect::<Vec<_>>()
        );
        for ability in &catalog.abilities {
            assert_ne!(ability.name, ability.name_key);
            assert_ne!(ability.description, ability.description_key);
        }

        let mut independently_discovered = resolver
            .list_files_recursive("common/abilities", Some("txt"))
            .expect("independently discover effective ability files");
        independently_discovered.retain(|file| is_direct_ability_file(&file.rel_path));
        independently_discovered.sort_by(|left, right| {
            source_order_key(&left.rel_path).cmp(&source_order_key(&right.rel_path))
        });
        let mut independent_ids = Vec::new();
        for file in independently_discovered {
            let document = parse_file_refusing_decode_damage(&file.path, &file.rel_path)
                .expect("independently parse effective ability file");
            assert!(
                document.recoveries.is_empty(),
                "independent ability walk recovered {}",
                file.rel_path
            );
            for top in document.root.fields() {
                if top
                    .key
                    .as_deref()
                    .is_some_and(|key| key.eq_ignore_ascii_case("ability"))
                {
                    let block = top
                        .value
                        .as_object()
                        .expect("independent ability root is an object");
                    independent_ids.extend(block.iter().map(|entry| {
                        entry
                            .key
                            .clone()
                            .expect("independent ability entry has an identity")
                    }));
                }
            }
        }
        assert_eq!(
            catalog
                .abilities
                .iter()
                .map(|ability| ability.ability_id.clone())
                .collect::<Vec<_>>(),
            independent_ids
        );
    }

    #[test]
    fn unresolved_normal_icon_rejects_the_catalog() {
        let error = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                ability_document(&definition_text("x", "X", "X_DESC")),
            )],
            &SpriteRegistry::new(),
        )
        .expect_err("missing normal icon must reject");
        assert!(error
            .to_string()
            .contains("icon GFX_ability_x is unresolved"));
    }

    #[test]
    fn unresolved_active_icon_rejects_the_catalog() {
        let error = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                ability_document(&definition_text("x", "X", "X_DESC")),
            )],
            &resolved_sprites(&["x"], false),
        )
        .expect_err("missing active icon must reject");
        assert!(error
            .to_string()
            .contains("animated icon GFX_ability_x_animated is unresolved"));
    }

    #[test]
    fn missing_name_localisation_rejects_the_catalog() {
        let definitions = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                ability_document(&definition_text("x", "X", "X_DESC")),
            )],
            &resolved_sprites(&["x"], true),
        )
        .expect("definition");
        let error = admit_ability_catalog(definitions, &localisation(&[("X_DESC", "Description")]))
            .expect_err("missing name localisation must reject");
        assert!(error
            .to_string()
            .contains("name localisation X is missing"));
    }

    #[test]
    fn missing_description_localisation_rejects_the_catalog() {
        let definitions = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                ability_document(&definition_text("x", "X", "X_DESC")),
            )],
            &resolved_sprites(&["x"], true),
        )
        .expect("definition");
        let error = admit_ability_catalog(definitions, &localisation(&[("X", "Name")]))
            .expect_err("missing description localisation must reject");
        assert!(error
            .to_string()
            .contains("description localisation X_DESC is missing"));
    }

    #[test]
    fn zero_ability_catalog_rejects() {
        let error = admit_ability_catalog(Vec::new(), &Localisation::default())
            .expect_err("empty catalog must reject");
        assert!(error
            .to_string()
            .contains("effective active ability catalog is empty"));
    }

    #[test]
    fn parser_recovery_rejects_the_catalog() {
        let damaged = parse_str(
            "ability = { x = { name = X desc = X_DESC type = army_leader cost = 0.1 duration = 24",
        );
        assert!(!damaged.recoveries.is_empty(), "fixture must require recovery");
        let error = ordered_definitions_from_documents(
            &[("common/abilities/damaged.txt".into(), damaged)],
            &resolved_sprites(&["x"], true),
        )
        .expect_err("parser recovery must reject");
        assert!(error
            .to_string()
            .contains("active ability definition recoveries"));
    }

    #[test]
    fn unknown_top_level_construct_rejects_the_catalog() {
        let error = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                parse_str("not_an_ability = { x = yes }"),
            )],
            &resolved_sprites(&["x"], true),
        )
        .expect_err("unknown top-level construct must reject");
        assert!(error
            .to_string()
            .contains("unknown active ability top-level construct"));
    }

    #[test]
    fn missing_required_field_rejects_the_catalog() {
        let error = ordered_definitions_from_documents(
            &[(
                "common/abilities/a.txt".into(),
                ability_document(
                    "x = { name = X desc = X_DESC type = army_leader cost = 0.1 }",
                ),
            )],
            &resolved_sprites(&["x"], true),
        )
        .expect_err("missing duration must reject");
        assert!(error
            .to_string()
            .contains("active ability x lacks duration"));
    }

    fn observer_catalog() -> AdmittedAbilityCatalog {
        AdmittedAbilityCatalog {
            abilities: vec![AdmittedAbilityDefinition {
                authored_order: 0,
                ability_id: "force_attack".into(),
                target_kind: "army_leader".into(),
                name_key: "ABILITY_FORCE_ATTACK".into(),
                description_key: "ABILITY_FORCE_ATTACK_DESC".into(),
                name: "Force Attack".into(),
                description: "Attack without regard for losses".into(),
                icon: "GFX_ability_force_attack".into(),
                active_icon: "GFX_ability_force_attack_animated".into(),
                base_cost_fraction: 0.22,
                authored_duration_hours: 168,
                authored_cooldown_hours: Some(72),
            }],
        }
    }

    fn observer_authority() -> ObserverAbilityAuthority {
        ObserverAbilityAuthority {
            executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
            frame_hour: 1_000,
            country_roster_complete: true,
            country_count: 3,
        }
    }

    fn country_context() -> ObserverAbilityContext {
        ObserverAbilityContext {
            caster: None,
            group_target: None,
        }
    }

    fn leader_context() -> ObserverAbilityContext {
        ObserverAbilityContext {
            caster: Some(ObserverAbilityScope {
                kind: ObserverAbilityScopeKind::ArmyLeader,
                stable_id: 73,
            }),
            group_target: None,
        }
    }

    fn observer_snapshot() -> ObserverAbilityRuntimeSnapshot {
        ObserverAbilityRuntimeSnapshot {
            country_tag: "GER".into(),
            complete: true,
            context_count: 2,
            contexts: vec![country_context(), leader_context()],
            row_count: 1,
            rows: vec![ObserverAbilityRuntimeRow {
                ability_id: "force_attack".into(),
                caster: leader_context().caster,
                group_target: None,
                definition_order: 0,
                target_kind_code: 1,
                available: false,
                lifecycle: ObserverAbilityLifecycle::Active,
                activation_hour: Some(900),
                authored_duration_hours: 168,
                authored_cooldown_hours: Some(72),
                duration_remaining_hours: Some(68),
                cooldown_remaining_hours: None,
                effective_command_power_cost_fixed: -2_200_000,
                effective_command_power_cost_scale: 100_000,
                disabled_reason: None,
            }],
        }
    }

    #[test]
    fn observer_ingress_preserves_exact_runtime_and_localised_file_join() {
        let rows = admit_observer_ability_snapshot(
            &observer_authority(),
            &observer_snapshot(),
            &observer_catalog(),
            "GER",
            1_000,
        )
        .expect("exact passive observer snapshot");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "Force Attack");
        assert_eq!(row.description, "Attack without regard for losses");
        assert_eq!(row.icon, "GFX_ability_force_attack_animated");
        assert_eq!(row.duration_remaining.hours, Some(68));
        assert_eq!(row.cost_command_power_fixed, -2_200_000);
        assert_eq!(row.target_kind_code, 1);
        assert_eq!(row.disabled_reason, None);
    }

    #[test]
    fn shipping_ingress_requires_frame_owned_authority_and_complete_receipt() {
        let (payload, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .expect("frame-owned XUI23 authority");
        assert_eq!(admitted.country_tag, "GER");
        assert_eq!(admitted.source_record_id, "xui23:test:1000:all-countries");
        assert_eq!(admitted.rows.len(), 1);

        let absent = decoded_xui23_frame(Map::new());
        let error = admit_observer_ability_frame(
            absent.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&absent.as_ref(), "GER"),
        )
        .expect_err("ordinary frame must expose the exact external residual");
        assert_eq!(error.0, XUI23_PASSIVE_CARRIER_UNAVAILABLE_REASON);

        let mut tampered = payload.replay_authority_record();
        tampered["payload"]["snapshots"][0]["rows"][0]["available"] =
            serde_json::Value::Bool(true);
        let mut rest = frame.raw_for_consumer_tests().clone();
        rest.insert(
            XUI23_PRODUCER_AUTHORITY_FIELD.into(),
            tampered,
        );
        let tampered_frame = decoded_xui23_frame(rest);
        assert!(admit_observer_ability_frame(
            tampered_frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&tampered_frame.as_ref(), "GER"),
        )
        .is_err());
    }

    #[test]
    fn shipping_ingress_requires_exact_ordered_d18_country_census() {
        let (mut payload, frame) = authenticated_observer_payload();
        let mut rest = frame.raw_for_consumer_tests().clone();
        rest.remove(XUI23_PRODUCER_AUTHORITY_FIELD);
        rest.insert(
            "countries".into(),
            serde_json::json!({
                "ENG": {"_r2_tag_db_id": 44},
                "GER": {"_r2_tag_db_id": 73}
            }),
        );
        let unsigned = decoded_xui23_frame(rest.clone());
        payload.replay_frame_identity_sha256 =
            observer_ability_replay_frame_identity_sha256(unsigned.as_ref()).unwrap();
        rest.insert(
            XUI23_PRODUCER_AUTHORITY_FIELD.into(),
            payload.replay_authority_record(),
        );
        let mismatched = decoded_xui23_frame(rest);
        let error = admit_observer_ability_frame(
            mismatched.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&mismatched.as_ref(), "GER"),
        )
        .expect_err("producer self-attestation cannot replace the D18 census");
        assert!(error.0.contains("ordered D18 census"));
    }

    #[test]
    fn synthetic_canonical_carrier_reaches_authored_abilitylist_pixels_and_hit_regions() {
        let root = temp_root("shipping_surface");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver
            .resolve_file("interface/abilitylist.gfx")
            .expect("synthetic ability GFX");
        registry
            .parse_file(&gfx.path, &gfx.rel_path)
            .expect("parse synthetic ability GFX");
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .expect("admitted synthetic canonical carrier");
        let mut cache = UiAssetCache::default();
        let surface = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut cache,
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect("shipping XUI23 compositor")
        .expect("nonempty selected context");
        assert_eq!(surface.authored_root, ABILITY_LIST_ROOT);
        assert_eq!(surface.rows.len(), 1);
        assert!(surface.input_region.contains(
            surface.button_regions[0].x,
            surface.button_regions[0].y
        ));
        assert_eq!(surface.button_regions.len(), 1);
        assert!(surface.button_regions[0].contains(
            surface.button_regions[0].x,
            surface.button_regions[0].y
        ));
        assert!(surface.pixels.report.drawn_elements > 0);
        assert!(surface.pixels.report.missing_sprites.is_empty());
        assert!(
            surface.pixels.report.undecodable_sprites.is_empty(),
            "undecodable sprites: {:?}",
            surface.pixels.report.undecodable_sprites
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_refuses_modified_effective_buttonstate_shader_bytes() {
        let root = temp_root("modified_buttonstate");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let shader = root.join("gfx/FX/buttonstate.shader");
        let mut bytes = portable_test_fs_retry::read(&shader).unwrap();
        bytes.push(b' ');
        std::fs::write(&shader, bytes).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&shader)
            .expect("hold the rewritten asset across the scanner's window");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
        registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .unwrap();
        let error = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut UiAssetCache::default(),
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect_err("modified effective shader bytes must reject XUI23 composition");
        assert!(error.to_string().contains("BuiltinShaderLawHashMismatch"));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_authenticates_the_default_shader_for_unanimated_enabled_icons() {
        let root = temp_root("unanimated_shader_authentication");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let gfx_path = root.join("interface/abilitylist.gfx");
        let gfx = portable_test_fs_retry::read_to_string(&gfx_path)
            .unwrap()
            .replace(
                "                effectFile = \"gfx/FX/buttonstate.lua\"\n                animation = {",
                "                unused_animation = {",
            );
        std::fs::write(&gfx_path, gfx).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&gfx_path)
            .expect("hold the rewritten asset across the scanner's window");
        let shader = root.join("gfx/FX/buttonstate.shader");
        let mut bytes = portable_test_fs_retry::read(&shader).unwrap();
        bytes.push(b' ');
        std::fs::write(&shader, bytes).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&shader)
            .expect("hold the rewritten asset across the scanner's window");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
        registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .unwrap();
        let error = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut UiAssetCache::default(),
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect_err("unanimated enabled XUI23 icons must authenticate their effective shader");
        assert!(
            error.to_string().contains("BuiltinShaderLawHashMismatch"),
            "unexpected refusal: {error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_executes_two_authored_animations_in_source_order() {
        let root = temp_root("multiple_animations");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let gfx_path = root.join("interface/abilitylist.gfx");
        let gfx = portable_test_fs_retry::read_to_string(&gfx_path).unwrap().replace(
            "                animation = {",
            r#"                animation = {
                    animationmaskfile = "gfx/xui23/ability_active.tga"
                    animationtexturefile = "gfx/xui23/ability_active.tga"
                    animationrotation = 0.0
                    animationlooping = yes
                    animationtime = 26.0
                    animationdelay = 0.0
                    animationblendmode = "add"
                    animationtype = "scrolling"
                    animationrotationoffset = { x = 0.0 y = 0.0 }
                    animationtexturescale = { x = 1.0 y = 1.0 }
                }
                animation = {"#,
        );
        std::fs::write(&gfx_path, gfx).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&gfx_path)
            .expect("hold the rewritten asset across the scanner's window");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
        registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .unwrap();
        let surface = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut UiAssetCache::default(),
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect("NUM_ANIMATIONS_2 must compose")
        .expect("selected context must be visible");
        assert!(surface.pixels.report.undecodable_sprites.is_empty());
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_accepts_authored_animationframes_for_the_selected_sprite_frame() {
        let root = temp_root("animationframes");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let gfx_path = root.join("interface/abilitylist.gfx");
        let gfx = portable_test_fs_retry::read_to_string(&gfx_path).unwrap().replace(
            "                animation = {",
            "                animation = {\n                    animationframes = { 1 2 3 }",
        );
        std::fs::write(&gfx_path, gfx).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&gfx_path)
            .expect("hold the rewritten asset across the scanner's window");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
        registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .unwrap();
        let surface = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut UiAssetCache::default(),
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect("authored frame-one animation must compose")
        .expect("selected context must be visible");
        assert!(surface.pixels.report.undecodable_sprites.is_empty());
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_progress_law_rejects_missing_custom_and_modified_effects() {
        for (case, expected) in [
            ("missing", "UnsupportedEffectiveShader"),
            ("custom", "UnsupportedEffectiveShader"),
            ("modified", "BuiltinShaderLawHashMismatch"),
        ] {
            let root = temp_root(&format!("progress_effect_{case}"));
            let _surface_guards = write_synthetic_ability_surface(&root);
            let gfx_path = root.join("interface/abilitylist.gfx");
            let rewritten = match case {
                "missing" => {
                    let gfx = portable_test_fs_retry::read_to_string(&gfx_path).unwrap().replace(
                        "                effectFile = \"gfx/FX/progress.lua\"
",
                        "",
                    );
                    std::fs::write(&gfx_path, gfx).unwrap();
                    gfx_path.clone()
                }
                "custom" => {
                    let gfx = portable_test_fs_retry::read_to_string(&gfx_path)
                        .unwrap()
                        .replace("gfx/FX/progress.lua", "gfx/FX/custom_progress.lua");
                    std::fs::write(&gfx_path, gfx).unwrap();
                    gfx_path.clone()
                }
                "modified" => {
                    let shader = root.join("gfx/FX/progress.shader");
                    let mut bytes = portable_test_fs_retry::read(&shader).unwrap();
                    bytes.push(b' ');
                    std::fs::write(&shader, bytes).unwrap();
                    shader
                }
                _ => unreachable!(),
            };
            let _scanner_guard = portable_test_fs_retry::open(&rewritten)
                .expect("hold the rewritten asset across the scanner's window");
            let resolver = AssetResolver::new(vec![root.clone()]);
            let mut registry = SpriteRegistry::new();
            let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
            registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
            let (_, frame) = authenticated_observer_payload();
            let admitted = admit_observer_ability_frame(
                frame.as_ref(),
                &"cd".repeat(32),
                &observer_catalog(),
                named(&frame.as_ref(), "GER"),
            )
            .unwrap();
            let error = compose_observer_ability_surface(
                &resolver,
                &registry,
                &mut UiAssetCache::default(),
                "GER",
                &leader_context(),
                &admitted.rows,
                250_000,
                320,
                180,
            )
            .expect_err("unauthenticated XUI23 progress effects must fail closed");
            assert!(
                error.to_string().contains(expected),
                "unexpected {case} progress-effect refusal: {error}"
            );
            portable_test_fs_retry::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn xui23_refuses_mod_added_unscoped_progress_children() {
        let root = temp_root("unscoped_progress_child");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let gui_path = root.join("interface/abilitylist.gui");
        let gui = portable_test_fs_retry::read_to_string(&gui_path).unwrap().replace(
            "                    buttonType = { position = { x = 0 y = 0 } name = abilitylist_button spriteType = GFX_ability_icon_default }",
            "                    iconType = { position = { x = 0 y = 0 } name = mod_progress spriteType = GFX_ability_active_bg }\n                    buttonType = { position = { x = 0 y = 0 } name = abilitylist_button spriteType = GFX_ability_icon_default }",
        );
        std::fs::write(&gui_path, gui).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&gui_path)
            .expect("hold the rewritten asset across the scanner's window");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
        registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .unwrap();
        let error = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut UiAssetCache::default(),
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect_err("mod-added XUI23 progress children must fail closed");
        assert!(
            error.to_string().contains("unscoped progress element"),
            "unexpected refusal: {error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_refuses_unmodelled_effective_ability_window_children() {
        let root = temp_root("extra_window_child");
        let _surface_guards = write_synthetic_ability_surface(&root);
        let gui_path = root.join("interface/abilitylist.gui");
        let source = portable_test_fs_retry::read_to_string(&gui_path).unwrap();
        let source = source.replace(
            "iconType = { position = { x = 24 y = 0 } name = bg_end spriteType = GFX_ability_list_end }",
            "iconType = { position = { x = 12 y = 0 } name = mod_extra spriteType = GFX_ability_list_mid }\n                    iconType = { position = { x = 24 y = 0 } name = bg_end spriteType = GFX_ability_list_end }",
        );
        std::fs::write(&gui_path, source).unwrap();
        let _scanner_guard = portable_test_fs_retry::open(&gui_path)
            .expect("hold the rewritten asset across the scanner's window");
        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut registry = SpriteRegistry::new();
        let gfx = resolver.resolve_file("interface/abilitylist.gfx").unwrap();
        registry.parse_file(&gfx.path, &gfx.rel_path).unwrap();
        let (_, frame) = authenticated_observer_payload();
        let admitted = admit_observer_ability_frame(
            frame.as_ref(),
            &"cd".repeat(32),
            &observer_catalog(),
            named(&frame.as_ref(), "GER"),
        )
        .unwrap();
        let error = compose_observer_ability_surface(
            &resolver,
            &registry,
            &mut UiAssetCache::default(),
            "GER",
            &leader_context(),
            &admitted.rows,
            250_000,
            320,
            180,
        )
        .expect_err("unmodelled effective window children must reject XUI23 composition");
        assert!(error
            .to_string()
            .contains("unsupported authored child shape"));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xui23_country_and_seek_resets_close_stale_open_contexts() {
        let mut viewer = ObserverAbilityViewerState {
            open: true,
            selected_context: Some(leader_context()),
            animation_elapsed_micros: 123,
        };
        viewer.reset_for_country_switch();
        assert!(!viewer.open);
        assert_eq!(viewer.selected_context, None);
        assert_eq!(viewer.animation_elapsed_micros, 0);

        viewer.open = true;
        viewer.selected_context = Some(leader_context());
        viewer.animation_elapsed_micros = 456;
        viewer.reset_for_seek();
        assert!(!viewer.open);
        assert_eq!(viewer.selected_context, None);
        assert_eq!(viewer.animation_elapsed_micros, 0);
    }

    #[test]
    fn observer_ingress_requires_complete_zero_row_context_roster() {
        let mut snapshot = observer_snapshot();
        let empty_context = ObserverAbilityContext {
            caster: Some(ObserverAbilityScope {
                kind: ObserverAbilityScopeKind::Army,
                stable_id: 91,
            }),
            group_target: None,
        };
        snapshot.contexts.push(empty_context.clone());
        snapshot.context_count = 3;
        let rows = admit_observer_ability_snapshot(
            &observer_authority(),
            &snapshot,
            &observer_catalog(),
            "GER",
            1_000,
        )
        .expect("zero-row context remains explicitly selectable");
        assert_eq!(rows.len(), 1);

        let mut viewer = ObserverAbilityViewerState::default();
        viewer
            .select_context(empty_context.clone(), &snapshot)
            .expect("complete zero-row context");
        assert_eq!(viewer.selected_context, Some(empty_context));
        assert!(viewer
            .select_context(
                ObserverAbilityContext {
                    caster: None,
                    group_target: Some(ObserverAbilityScope {
                        kind: ObserverAbilityScopeKind::ArmyGroup,
                        stable_id: 999,
                    }),
                },
                &snapshot,
            )
            .is_err());
    }

    #[test]
    fn observer_ingress_fails_closed_on_authority_identity_and_semantic_drift() {
        let catalog = observer_catalog();
        let authority = observer_authority();

        let mut snapshot = observer_snapshot();
        snapshot.complete = false;
        assert!(admit_observer_ability_snapshot(
            &authority,
            &snapshot,
            &catalog,
            "GER",
            1_000
        )
        .is_err());

        let mut snapshot = observer_snapshot();
        snapshot.rows[0].disabled_reason = Some("guessed cause".into());
        assert!(admit_observer_ability_snapshot(
            &authority,
            &snapshot,
            &catalog,
            "GER",
            1_000
        )
        .is_err());

        let mut snapshot = observer_snapshot();
        snapshot.rows[0].duration_remaining_hours = Some(67);
        assert!(admit_observer_ability_snapshot(
            &authority,
            &snapshot,
            &catalog,
            "GER",
            1_000
        )
        .is_err());

        let mut authority = authority;
        authority.executable_sha256 = "not-exact-a729".into();
        assert!(admit_observer_ability_snapshot(
            &authority,
            &observer_snapshot(),
            &catalog,
            "GER",
            1_000
        )
        .is_err());
    }

    #[test]
    fn observer_timer_fraction_distinguishes_absent_indefinite_and_hours() {
        let hours = |value| CommandAbilityTimer {
            kind: CommandAbilityTimerKind::Hours,
            hours: Some(value),
        };
        assert_eq!(
            runtime_timer_fraction(&hours(20), &hours(5)).expect("bounded hours"),
            Some(0.25)
        );
        assert_eq!(
            runtime_timer_fraction(
                &CommandAbilityTimer {
                    kind: CommandAbilityTimerKind::Indefinite,
                    hours: None,
                },
                &CommandAbilityTimer {
                    kind: CommandAbilityTimerKind::NotStarted,
                    hours: None,
                },
            )
            .expect("indefinite is not a guessed fill"),
            None
        );
    }
}
