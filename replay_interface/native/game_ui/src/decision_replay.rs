//! XUI19 production seam for the decisions and missions panel.
//!
//! Static definition facts come from the extractor's effective-load-order
//! `decisions.json`. Runtime facts are accepted only through the complete,
//! ordered [`DecisionPanelSnapshot`] carrier. The module never evaluates
//! scripted triggers, invents lifecycle results, derives display order, or
//! formats costs/timers from authored widget names.

use crate::{GameUiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const DECISION_ALERT_TIMEOUT_DAYS: i32 = 30;
pub const MISSION_REMOVE_FROM_INTERFACE_DEFAULT: i32 = 13;
pub const MAX_DECISIONS_IN_DECISION_ALERT_TOOLTIP: u32 = 5;

pub const DECISION_ITEM_BG: &str = "GFX_decision_item_bg";
pub const DECISION_PROGRESS_GOOD: &str = "GFX_decision_item_progress_good";
pub const DECISION_PROGRESS_BAD: &str = "GFX_decision_item_progress_bad";
pub const DECISION_TIMER_ICON: &str = "GFX_decision_timer_icon_strip";
pub const DECISION_SELECT_ICON: &str = "GFX_decision_select_icon_strip";
pub const DECISION_UNKNOWN_ICON: &str = "GFX_decision_unknown";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTargetScope {
    Country,
    State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionStaticEntry {
    pub decision_id: String,
    pub category_id: String,
    pub loc_key: String,
    pub icon_token: Option<String>,
    pub source_layer: String,
    pub source_file: String,
    pub target_scope: Option<DecisionTargetScope>,
    pub has_visible_clause: bool,
    pub has_available_clause: bool,
    pub has_allowed_clause: bool,
    pub has_activation_clause: bool,
    pub has_cost: bool,
    pub has_custom_cost: bool,
    pub has_days_re_enable: bool,
    pub has_days_remove: bool,
    pub has_days_mission_timeout: bool,
    pub complete_effect_present: bool,
    pub timeout_effect_present: bool,
    pub remove_effect_present: bool,
    pub do_not_show_alert: bool,
    pub is_good: Option<bool>,
    pub authored_sort_order: Option<String>,
    pub authored_priority: Option<String>,
    pub template: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionStaticCatalog {
    pub entries: BTreeMap<String, DecisionStaticEntry>,
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| GameUiError::parse(format!("{context} must be an object")))
}

fn string_field<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| GameUiError::parse(format!("decision catalog field {key} missing")))
}

fn nested<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &str,
) -> &'a serde_json::Map<String, Value> {
    obj.get(key)
        .and_then(Value::as_object)
        .unwrap_or_else(|| empty_object())
}

fn empty_object() -> &'static serde_json::Map<String, Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(serde_json::Map::new)
}

fn boolish(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(value) if value.eq_ignore_ascii_case("yes") || value == "1" => Some(true),
        Value::String(value) if value.eq_ignore_ascii_case("no") || value == "0" => Some(false),
        _ => None,
    }
}

fn present_non_null(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key).is_some_and(|value| !value.is_null())
}

impl DecisionStaticCatalog {
    /// Admit the extractor's effective `decisions.json` without interpreting
    /// arbitrary Clausewitz trigger/effect bodies.
    pub fn from_extractor_json(bytes: &[u8]) -> Result<Self> {
        let root: Value = serde_json::from_slice(bytes)
            .map_err(|error| GameUiError::parse(format!("decisions.json: {error}")))?;
        let root = object(&root, "decisions.json")?;
        let decisions = object(
            root.get("decisions")
                .ok_or_else(|| GameUiError::parse("decisions.json lacks decisions"))?,
            "decisions",
        )?;
        let mut entries = BTreeMap::new();
        for (map_id, row) in decisions {
            let row = object(row, map_id)?;
            let decision_id = string_field(row, "id")?.to_string();
            if map_id != &decision_id {
                return Err(GameUiError::parse(format!(
                    "decision map key {map_id:?} disagrees with id {decision_id:?}"
                )));
            }
            let targets = nested(row, "targets");
            let cooldowns = nested(row, "cooldowns");
            let visibility = nested(row, "visibility");
            let cost = nested(row, "cost");
            let targeted_by_definition = boolish(targets.get("targeted_decision")) == Some(true)
                || targets
                    .get("has_target_trigger")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                || targets
                    .get("has_target_root_trigger")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            let target_scope = match targets.get("target_scope").and_then(Value::as_str) {
                Some("country") => Some(DecisionTargetScope::Country),
                Some("state") => Some(DecisionTargetScope::State),
                Some(other) => {
                    return Err(GameUiError::parse(format!(
                        "decision {decision_id} has unknown target_scope {other:?}"
                    )))
                }
                // A12 proves the runtime union directly: numeric rows are
                // classified by an explicit state_target definition, while
                // targeted definitions without that declaration use the
                // string/country member. The pinned corpus has no province
                // target declaration.
                None if targeted_by_definition => Some(DecisionTargetScope::Country),
                None => None,
            };
            let has_activation_clause = present_non_null(cooldowns, "activation");
            let has_days_remove = present_non_null(cooldowns, "days_remove");
            let has_days_mission_timeout = present_non_null(cooldowns, "days_mission_timeout");
            let template = if target_scope.is_some() {
                "targeted_decision_item"
            } else if has_days_remove || has_days_mission_timeout {
                "timed_decision_item"
            } else {
                "decision_item"
            };
            entries.insert(
                decision_id.clone(),
                DecisionStaticEntry {
                    decision_id,
                    category_id: string_field(row, "category")?.to_string(),
                    loc_key: string_field(row, "loc_key")?.to_string(),
                    icon_token: row.get("icon").and_then(Value::as_str).map(str::to_string),
                    source_layer: string_field(row, "source_layer")?.to_string(),
                    source_file: string_field(row, "source_file")?.to_string(),
                    target_scope,
                    has_visible_clause: visibility
                        .get("has_visible")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    has_available_clause: visibility
                        .get("has_available")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    has_allowed_clause: visibility
                        .get("has_allowed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    has_activation_clause,
                    has_cost: present_non_null(cost, "political_power"),
                    has_custom_cost: cost
                        .get("has_custom_cost_trigger")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        || present_non_null(cost, "custom_cost_text"),
                    has_days_re_enable: present_non_null(cooldowns, "days_re_enable"),
                    has_days_remove,
                    has_days_mission_timeout,
                    complete_effect_present: cooldowns
                        .get("complete_effect_present")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    timeout_effect_present: cooldowns
                        .get("timeout_effect_present")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    remove_effect_present: cooldowns
                        .get("remove_effect_present")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    do_not_show_alert: boolish(row.get("do_not_show_alert")) == Some(true),
                    is_good: boolish(row.get("is_good")),
                    authored_sort_order: row
                        .get("sort_order")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    authored_priority: row
                        .get("priority")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    template: template.to_string(),
                },
            );
        }
        Ok(Self { entries })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAvailability {
    Available,
    Unavailable,
    Active,
    OnCooldown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionLifecycle {
    Pending,
    Running,
    Completed,
    Failed,
    Removed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionTarget {
    Country { tag: String },
    State { state_id: u32 },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecisionCostState {
    pub political_power: Option<i64>,
    pub custom_cost_available: Option<bool>,
    /// Exact producer-selected display text. Never synthesized from the widget
    /// key `cost_and_timer_text`, which UI13 proved can contain row names.
    pub formatted_text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecisionPanelRow {
    pub decision_id: String,
    pub category_id: String,
    pub presentation_index: u32,
    pub availability: DecisionAvailability,
    pub lifecycle: DecisionLifecycle,
    pub target: Option<DecisionTarget>,
    pub cost: Option<DecisionCostState>,
    pub remaining_timer_days: Option<i32>,
    pub cooldown_remaining_days: Option<i32>,
    pub name_text: String,
    pub timer_text: Option<String>,
    pub alert_to_take: bool,
    pub alert_timeout: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecisionPanelSnapshot {
    pub producer_complete: bool,
    pub frame_hour: i64,
    pub country_tag: String,
    pub available_alert_count: u32,
    pub timeout_alert_count: u32,
    pub rows: Vec<DecisionPanelRow>,
}

/// Convert replay-core's fail-closed d87 view into the only runtime carrier
/// accepted by the decision binder. A missing or unavailable typed view never
/// falls back to authored definitions or local script evaluation.
pub fn decision_panel_snapshot_from_world(
    view: &replay_core::DecisionPanelExactView,
) -> Result<DecisionPanelSnapshot> {
    if view.status != replay_core::ViewStatus::Present {
        return Err(GameUiError::missing(
            "exact decision-panel runtime view is absent or unavailable",
        ));
    }
    let panel = view.panel.as_ref().ok_or_else(|| {
        GameUiError::missing("present decision-panel view omitted its exact payload")
    })?;
    let frame_hour = i64::try_from(panel.frame_hour)
        .map_err(|_| GameUiError::parse("decision-panel frame hour exceeds i64"))?;
    let rows = panel
        .rows
        .iter()
        .map(|row| DecisionPanelRow {
            decision_id: row.decision_id.clone(),
            category_id: row.category_id.clone(),
            presentation_index: row.presentation_index,
            availability: match row.availability {
                replay_core::DecisionAvailabilityExact::Available => DecisionAvailability::Available,
                replay_core::DecisionAvailabilityExact::Unavailable => {
                    DecisionAvailability::Unavailable
                }
                replay_core::DecisionAvailabilityExact::Active => DecisionAvailability::Active,
                replay_core::DecisionAvailabilityExact::OnCooldown => {
                    DecisionAvailability::OnCooldown
                }
            },
            lifecycle: match row.lifecycle {
                replay_core::DecisionLifecycleExact::Pending => DecisionLifecycle::Pending,
                replay_core::DecisionLifecycleExact::Running => DecisionLifecycle::Running,
                replay_core::DecisionLifecycleExact::Completed => DecisionLifecycle::Completed,
                replay_core::DecisionLifecycleExact::Failed => DecisionLifecycle::Failed,
                replay_core::DecisionLifecycleExact::Removed => DecisionLifecycle::Removed,
            },
            target: row.target.as_ref().map(|target| match target {
                replay_core::DecisionTargetExact::Country { tag } => {
                    DecisionTarget::Country { tag: tag.clone() }
                }
                replay_core::DecisionTargetExact::State { state_id } => {
                    DecisionTarget::State { state_id: *state_id }
                }
            }),
            cost: row.cost.as_ref().map(|cost| DecisionCostState {
                political_power: cost.political_power,
                custom_cost_available: cost.custom_cost_available,
                formatted_text: cost.formatted_text.clone(),
            }),
            remaining_timer_days: row.remaining_timer_days,
            cooldown_remaining_days: row.cooldown_remaining_days,
            name_text: row.name_text.clone(),
            timer_text: row.timer_text.clone(),
            alert_to_take: row.alert_to_take,
            alert_timeout: row.alert_timeout,
        })
        .collect();
    Ok(DecisionPanelSnapshot {
        producer_complete: panel.producer_complete,
        frame_hour,
        country_tag: panel.country_tag.clone(),
        available_alert_count: panel.available_alert_count,
        timeout_alert_count: panel.timeout_alert_count,
        rows,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionCard {
    pub decision_id: String,
    pub category_id: String,
    pub presentation_index: u32,
    pub template: String,
    pub icon_sprite: String,
    pub background_sprite: &'static str,
    pub progress_sprite: &'static str,
    pub timer_icon_sprite: Option<&'static str>,
    pub select_icon_sprite: &'static str,
    pub availability: DecisionAvailability,
    pub lifecycle: DecisionLifecycle,
    pub target: Option<DecisionTarget>,
    pub cost: Option<DecisionCostState>,
    pub remaining_timer_days: Option<i32>,
    pub cooldown_remaining_days: Option<i32>,
    pub name_text: String,
    pub timer_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPanelBinding {
    pub frame_hour: i64,
    pub country_tag: String,
    pub available_alert_count: u32,
    pub timeout_alert_count: u32,
    pub alert_tooltip_row_limit: u32,
    pub cards: Vec<DecisionCard>,
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

fn icon_sprite(token: Option<&str>) -> String {
    match token {
        Some(token) if token.starts_with("GFX_") => token.to_string(),
        Some(token) => format!("GFX_decision_{token}"),
        None => DECISION_UNKNOWN_ICON.to_string(),
    }
}

/// Validate and bind a complete exact producer snapshot. Any missing,
/// inconsistent, inferred, or unordered input rejects the entire panel.
pub fn bind_decision_panel(
    catalog: &DecisionStaticCatalog,
    snapshot: &DecisionPanelSnapshot,
) -> Result<DecisionPanelBinding> {
    if !snapshot.producer_complete {
        return Err(GameUiError::missing(
            "decision panel producer did not attest complete ordered state",
        ));
    }
    if !valid_token(&snapshot.country_tag) {
        return Err(GameUiError::parse("decision panel country_tag is invalid"));
    }
    let mut identities = BTreeSet::new();
    let mut available_alert_count = 0u32;
    let mut timeout_alert_count = 0u32;
    let mut cards = Vec::with_capacity(snapshot.rows.len());
    for (ordinal, row) in snapshot.rows.iter().enumerate() {
        if row.presentation_index as usize != ordinal {
            return Err(GameUiError::parse(format!(
                "decision row {} presentation index {} is not contiguous",
                row.decision_id, row.presentation_index
            )));
        }
        if !valid_token(&row.decision_id) || !valid_token(&row.category_id) {
            return Err(GameUiError::parse("decision row identity is invalid"));
        }
        let def = catalog.entries.get(&row.decision_id).ok_or_else(|| {
            GameUiError::missing(format!(
                "decision {} absent from effective catalog",
                row.decision_id
            ))
        })?;
        if def.category_id != row.category_id {
            return Err(GameUiError::parse(format!(
                "decision {} category mismatch: runtime {} static {}",
                row.decision_id, row.category_id, def.category_id
            )));
        }
        if !identities.insert((row.decision_id.clone(), row.target.clone())) {
            return Err(GameUiError::parse(format!(
                "duplicate decision/target identity {}",
                row.decision_id
            )));
        }
        match (&def.target_scope, &row.target) {
            (Some(DecisionTargetScope::Country), Some(DecisionTarget::Country { tag }))
                if valid_token(tag) => {}
            (Some(DecisionTargetScope::State), Some(DecisionTarget::State { state_id }))
                if *state_id > 0 => {}
            (Some(_), _) => {
                return Err(GameUiError::parse(format!(
                    "decision {} target is absent or disagrees with static scope",
                    row.decision_id
                )))
            }
            (None, Some(_)) => {
                return Err(GameUiError::parse(format!(
                    "untargeted decision {} supplied a target",
                    row.decision_id
                )))
            }
            (None, None) => {}
        }
        if row.name_text.is_empty() {
            return Err(GameUiError::missing(format!(
                "decision {} lacks exact name_text",
                row.decision_id
            )));
        }
        if row.remaining_timer_days.is_some_and(|days| days < 0)
            || row.cooldown_remaining_days.is_some_and(|days| days < 0)
        {
            return Err(GameUiError::parse("decision timer/cooldown is negative"));
        }
        let has_timer = row.remaining_timer_days.is_some() || row.cooldown_remaining_days.is_some();
        if has_timer != row.timer_text.is_some() {
            return Err(GameUiError::missing(format!(
                "decision {} timer value/text completeness mismatch",
                row.decision_id
            )));
        }
        if row.availability == DecisionAvailability::Active
            && row.lifecycle != DecisionLifecycle::Running
        {
            return Err(GameUiError::parse("active decision is not running"));
        }
        if row.availability == DecisionAvailability::OnCooldown
            && !row.cooldown_remaining_days.is_some_and(|days| days > 0)
        {
            return Err(GameUiError::parse(
                "on_cooldown decision lacks positive cooldown_remaining_days",
            ));
        }
        if row.alert_to_take {
            if row.availability != DecisionAvailability::Available || def.do_not_show_alert {
                return Err(GameUiError::parse("invalid decision-to-take alert row"));
            }
            available_alert_count += 1;
        }
        if row.alert_timeout {
            let days = row
                .remaining_timer_days
                .ok_or_else(|| GameUiError::parse("timeout alert lacks remaining_timer_days"))?;
            if days > DECISION_ALERT_TIMEOUT_DAYS || def.do_not_show_alert {
                return Err(GameUiError::parse("invalid decision timeout alert row"));
            }
            timeout_alert_count += 1;
        }
        let progress_sprite = if def.is_good == Some(false) {
            DECISION_PROGRESS_BAD
        } else {
            DECISION_PROGRESS_GOOD
        };
        cards.push(DecisionCard {
            decision_id: row.decision_id.clone(),
            category_id: row.category_id.clone(),
            presentation_index: row.presentation_index,
            template: def.template.clone(),
            icon_sprite: icon_sprite(def.icon_token.as_deref()),
            background_sprite: DECISION_ITEM_BG,
            progress_sprite,
            timer_icon_sprite: has_timer.then_some(DECISION_TIMER_ICON),
            select_icon_sprite: DECISION_SELECT_ICON,
            availability: row.availability,
            lifecycle: row.lifecycle,
            target: row.target.clone(),
            cost: row.cost.clone(),
            remaining_timer_days: row.remaining_timer_days,
            cooldown_remaining_days: row.cooldown_remaining_days,
            name_text: row.name_text.clone(),
            timer_text: row.timer_text.clone(),
        });
    }
    if available_alert_count != snapshot.available_alert_count
        || timeout_alert_count != snapshot.timeout_alert_count
    {
        return Err(GameUiError::parse(
            "decision panel alert counts disagree with ordered rows",
        ));
    }
    Ok(DecisionPanelBinding {
        frame_hour: snapshot.frame_hour,
        country_tag: snapshot.country_tag.clone(),
        available_alert_count,
        timeout_alert_count,
        alert_tooltip_row_limit: MAX_DECISIONS_IN_DECISION_ALERT_TOOLTIP,
        cards,
    })
}
