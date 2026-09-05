
use crate::WorldRoot;
use crate::world::ViewStatus;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrdDepthLeaf<T> {
    Absent,
    Unobtained,
    Measured(T),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrdFocusDepthEvent {
    pub kind: u32,
    pub focus_kind: u32,
    pub previous_completed: u32,
    pub current_completed: u32,
    pub previous_progress: i32,
    pub current_progress: i32,
    pub completed_delta: u32,
    pub from_token: String,
    pub to_token: String,
    pub completed_token: String,
    pub completed_token_exact: bool,
    pub completed_token_inferred: bool,
    pub paused: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrdResearchLiveRow {
    pub slot_index: u32,
    pub current_handle: u32,
    pub current_def: u32,
    pub previous_handle: u32,
    pub previous_def: u32,
    pub previous_progress_raw: i64,
    pub current_progress_raw: i64,
    pub delta_raw: i64,
    pub speed_raw: i64,
    pub base_raw: Option<i64>,
    pub eta_hours: Option<i64>,
    pub hour_delta: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrdDecisionStatus {
    pub active_timed_count: u32,
    pub targeted_count: u32,
    pub targeted_timed_count: u32,
    pub decisions_taken_bucket_count: u32,
    pub re_enable_count: u32,
    pub timed_live_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrdDecisionEvent {
    pub kind: u32,
    pub previous_days: i32,
    pub current_days: i32,
    pub previous_state: i32,
    pub current_state: i32,
    pub decision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrdDecisionTargetKind {
    Country,
    Numeric,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrdDecisionTargetWire {
    decision: String,
    target_kind: String,
    target: Option<String>,
    target_tag_db_id: Option<u32>,
    target_id: Option<u32>,
    flags: u32,
    source_kind: u32,
    source: String,
    lifecycle_state: Option<i32>,
    lifecycle_state_name: Option<String>,
    ignore: Option<bool>,
    days: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrdDecisionTarget {
    pub decision: String,
    pub target_kind: FrdDecisionTargetKind,
    pub target_country: Option<String>,
    pub target_id: u32,
    pub flags: u32,
    pub source_kind: u32,
    pub source: String,
    pub lifecycle_state: Option<i32>,
    pub lifecycle_state_name: Option<String>,
    pub ignore: Option<bool>,
    pub days: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrdDepthBadge {
    pub kind: u32,
    pub flags: u32,
    pub source_domain_id: u32,
    pub affected_count: u32,
    pub evidence_code: u32,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrdCountryDepth {
    pub focus_current: FrdDepthLeaf<String>,
    pub focus_current_continuous: FrdDepthLeaf<String>,
    pub focus_completed_count: FrdDepthLeaf<u32>,
    pub focus_completed: FrdDepthLeaf<Vec<String>>,
    pub focus_events: FrdDepthLeaf<Vec<FrdFocusDepthEvent>>,
    pub research_in_progress_live: FrdDepthLeaf<Vec<FrdResearchLiveRow>>,
    pub decision_status: FrdDepthLeaf<FrdDecisionStatus>,
    pub decision_targets: FrdDepthLeaf<Vec<FrdDecisionTarget>>,
    pub decision_events: FrdDepthLeaf<Vec<FrdDecisionEvent>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrdDepthAnnotationsView {
    pub status: ViewStatus,
    pub countries: BTreeMap<String, FrdCountryDepth>,
    pub badges: FrdDepthLeaf<Vec<FrdDepthBadge>>,
}

impl FrdDepthAnnotationsView {
    fn absent() -> Self {
        Self {
            status: ViewStatus::DomainAbsent,
            countries: BTreeMap::new(),
            badges: FrdDepthLeaf::Absent,
        }
    }

    fn unavailable() -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            countries: BTreeMap::new(),
            badges: FrdDepthLeaf::Unobtained,
        }
    }

    pub(crate) fn mark_unavailable(&mut self) {
        *self = Self::unavailable();
    }
}

fn leaf<T: DeserializeOwned>(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<(bool, FrdDepthLeaf<T>), String> {
    match object.get(key) {
        None => Ok((false, FrdDepthLeaf::Absent)),
        Some(Value::Null) => Ok((true, FrdDepthLeaf::Unobtained)),
        Some(value) => serde_json::from_value(value.clone())
            .map(|value| (true, FrdDepthLeaf::Measured(value)))
            .map_err(|error| format!("{path}.{key}: {error}")),
    }
}

fn require_nonblank(value: &str, path: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{path}: blank string"))
    } else {
        Ok(())
    }
}

fn validate_focus_events(
    leaf: &FrdDepthLeaf<Vec<FrdFocusDepthEvent>>,
    path: &str,
) -> Result<(), String> {
    let FrdDepthLeaf::Measured(rows) = leaf else {
        return Ok(());
    };
    for (index, row) in rows.iter().enumerate() {
        let row_path = format!("{path}[{index}]");
        if !(1..=7).contains(&row.kind) {
            return Err(format!("{row_path}.kind: unsupported value {}", row.kind));
        }
        if row.focus_kind > 2 {
            return Err(format!(
                "{row_path}.focus_kind: unsupported value {}",
                row.focus_kind
            ));
        }
        if (row.kind == 7) != row.paused.is_some() {
            return Err(format!(
                "{row_path}.paused: must be measured only for state snapshots"
            ));
        }
        if row.completed_token_exact && row.completed_token_inferred {
            return Err(format!(
                "{row_path}: completed token cannot be both exact and inferred"
            ));
        }
        if (row.completed_token_exact || row.completed_token_inferred)
            && row.completed_token.is_empty()
        {
            return Err(format!(
                "{row_path}: completed-token authority has no token"
            ));
        }
        if row.kind == 7 && (row.completed_token_exact || row.completed_token_inferred) {
            return Err(format!(
                "{row_path}: state snapshot carries completed-token authority"
            ));
        }
        if row.kind == 1 {
            if row.completed_delta == 0 {
                return Err(format!("{row_path}.completed_delta: zero increase"));
            }
            if row.current_completed != u32::MAX
                && row.previous_completed != u32::MAX
                && row.current_completed <= row.previous_completed
            {
                return Err(format!("{row_path}: completed count did not increase"));
            }
        }
        if row.kind == 5 && row.current_progress >= row.previous_progress {
            return Err(format!(
                "{row_path}: progress-regression event did not regress"
            ));
        }
        let unresolved_completion =
            row.kind == 1 && !row.completed_token_exact && !row.completed_token_inferred;
        if row.from_token.is_empty()
            && row.to_token.is_empty()
            && row.completed_token.is_empty()
            && !matches!(row.kind, 6 | 7)
            && !unresolved_completion
        {
            return Err(format!("{row_path}: event has no token context"));
        }
    }
    Ok(())
}

fn validate_research_rows(
    leaf: &FrdDepthLeaf<Vec<FrdResearchLiveRow>>,
    path: &str,
) -> Result<(), String> {
    let FrdDepthLeaf::Measured(rows) = leaf else {
        return Ok(());
    };
    let mut slots = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_path = format!("{path}[{index}]");
        if !slots.insert(row.slot_index) {
            return Err(format!(
                "{row_path}.slot_index: duplicate slot {}",
                row.slot_index
            ));
        }
        if row.slot_index > 32 {
            return Err(format!(
                "{row_path}.slot_index: unsupported slot {}",
                row.slot_index
            ));
        }
        if row.flags & !0x3f != 0 {
            return Err(format!("{row_path}.flags: unknown bits {}", row.flags));
        }
        if row.hour_delta == 0 {
            return Err(format!("{row_path}.hour_delta: zero interval"));
        }
        let same_tech = row.flags & 1 != 0;
        if same_tech
            && (row.previous_handle != row.current_handle || row.previous_def != row.current_def)
        {
            return Err(format!("{row_path}: same-tech IDs disagree"));
        }
        if !same_tech && row.flags & (1 << 3) == 0 {
            return Err(format!("{row_path}: changed tech without rollover flag"));
        }
        if row.delta_raw
            != row
                .current_progress_raw
                .saturating_sub(row.previous_progress_raw)
        {
            return Err(format!("{row_path}.delta_raw: does not match progress"));
        }
        if row.flags & (1 << 1) != 0 && row.speed_raw <= 0 {
            return Err(format!("{row_path}.speed_raw: positive flag without value"));
        }
        if row.flags & (1 << 4) != 0 && row.delta_raw >= 0 {
            return Err(format!("{row_path}.delta_raw: negative flag without value"));
        }
        if row.base_raw.is_some() != (row.flags & (1 << 5) != 0) {
            return Err(format!("{row_path}.base_raw: flags disagree with presence"));
        }
        if row.base_raw.is_some_and(|base| base <= 0) {
            return Err(format!("{row_path}.base_raw: non-positive base"));
        }
        if row.eta_hours.is_some() != (row.flags & (1 << 2) != 0) {
            return Err(format!(
                "{row_path}.eta_hours: flags disagree with presence"
            ));
        }
        if row.eta_hours.is_some_and(|eta| eta < 0) {
            return Err(format!("{row_path}.eta_hours: negative ETA"));
        }
    }
    Ok(())
}

fn required_target_keys(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "country" => Some(&[
            "decision",
            "target_kind",
            "target",
            "target_tag_db_id",
            "flags",
            "source_kind",
            "source",
            "lifecycle_state",
            "lifecycle_state_name",
            "ignore",
            "days",
        ]),
        "numeric" => Some(&[
            "decision",
            "target_kind",
            "target_id",
            "flags",
            "source_kind",
            "source",
            "lifecycle_state",
            "lifecycle_state_name",
            "ignore",
            "days",
        ]),
        _ => None,
    }
}

fn parse_targets(
    value: &Value,
    path: &str,
    contended_keys: &impl crate::world::ContendedKeys,
) -> Result<Vec<FrdDecisionTarget>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{path}: expected array"))?;
    let mut out = Vec::with_capacity(rows.len());
    let mut identities = BTreeSet::new();
    for (index, value) in rows.iter().enumerate() {
        let row_path = format!("{path}[{index}]");
        let object = value
            .as_object()
            .ok_or_else(|| format!("{row_path}: expected object"))?;
        let kind = object
            .get("target_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{row_path}.target_kind: expected string"))?;
        let required = required_target_keys(kind)
            .ok_or_else(|| format!("{row_path}.target_kind: unsupported value {kind:?}"))?;
        let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
        let required_keys: BTreeSet<&str> = required.iter().copied().collect();
        if keys != required_keys {
            return Err(format!(
                "{row_path}: key set does not match exact {kind} target schema"
            ));
        }
        let wire: FrdDecisionTargetWire = serde_json::from_value(value.clone())
            .map_err(|error| format!("{row_path}: {error}"))?;
        require_nonblank(&wire.decision, &format!("{row_path}.decision"))?;
        if !matches!(wire.source_kind, 1 | 2) {
            return Err(format!(
                "{row_path}.source_kind: unsupported value {}",
                wire.source_kind
            ));
        }
        let source_offset = if wire.source_kind == 2 {
            "0x100"
        } else {
            "0xE8"
        };
        let (target_kind, target_country, target_id, expected_flags, target_offset) = match kind {
            "country" => (
                FrdDecisionTargetKind::Country,
                wire.target,
                wire.target_tag_db_id
                    .ok_or_else(|| format!("{row_path}.target_tag_db_id: absent"))?,
                3,
                "0x20",
            ),
            "numeric" => (
                FrdDecisionTargetKind::Numeric,
                None,
                wire.target_id
                    .ok_or_else(|| format!("{row_path}.target_id: absent"))?,
                12,
                "0x24",
            ),
            _ => unreachable!(),
        };
        if wire.flags != expected_flags {
            return Err(format!(
                "{row_path}.flags: expected {expected_flags}, found {}",
                wire.flags
            ));
        }
        let expected_source = format!("CDecisionStatus+{source_offset} record+{target_offset}");
        if wire.source != expected_source {
            return Err(format!(
                "{row_path}.source: expected {expected_source:?}, found {:?}",
                wire.source
            ));
        }
        let max_target_id = if target_kind == FrdDecisionTargetKind::Country {
            2_048
        } else {
            1_000_000
        };
        if target_id == 0 || target_id > max_target_id {
            return Err(format!(
                "{row_path}.target_id: identity {target_id} outside 1..={max_target_id}"
            ));
        }
        if target_kind == FrdDecisionTargetKind::Country {
            let target = target_country
                .as_deref()
                .ok_or_else(|| format!("{row_path}.target: absent country identity"))?;
            require_nonblank(target, &format!("{row_path}.target"))?;
        }
        let expected_lifecycle_name =
            wire.lifecycle_state
                .map(|state| match (wire.source_kind, state) {
                    (2, 0) => "available",
                    (2, 1) => "active",
                    (2, 2) => "re_enable_cooldown",
                    (2, 3) => "completed",
                    (2, 4) => "removed",
                    (1, 0) => "active",
                    (1, 1) => "available",
                    (1, 2) => "completed",
                    (1, 3) => "removed",
                    (1, 4) => "re_enable_cooldown",
                    _ => "unknown",
                });
        if wire.lifecycle_state_name.as_deref() != expected_lifecycle_name {
            return Err(format!(
                "{row_path}.lifecycle_state_name: does not match source/state"
            ));
        }
        let identity = (
            wire.decision.clone(),
            target_kind as u8,
            target_id,
            wire.source_kind,
        );
        if !identities.insert(identity) {
            return Err(format!("{row_path}: duplicate decision-target identity"));
        }
        out.push(FrdDecisionTarget {
            decision: wire.decision,
            target_kind,
            // WITHHELD at the publish, after the checks above read the name
            // the producer sent. The row is identified by `target_id`; the
            // NAME is what a key two ids reach cannot supply. The outer map
            // key was filtered in round 38 and this inner one was not, so an
            // uncontended country published a contended target (Codex round
            // 39).
            target_country: target_country
                .filter(|tag| !contended_keys.holds(tag.as_str())),
            target_id,
            flags: wire.flags,
            source_kind: wire.source_kind,
            source: wire.source,
            lifecycle_state: wire.lifecycle_state,
            lifecycle_state_name: wire.lifecycle_state_name,
            ignore: wire.ignore,
            days: wire.days,
        });
    }
    Ok(out)
}

fn target_leaf(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    contended_keys: &impl crate::world::ContendedKeys,
) -> Result<(bool, FrdDepthLeaf<Vec<FrdDecisionTarget>>), String> {
    match object.get(key) {
        None => Ok((false, FrdDepthLeaf::Absent)),
        Some(Value::Null) => Ok((true, FrdDepthLeaf::Unobtained)),
        Some(value) => parse_targets(value, &format!("{path}.{key}"), contended_keys)
            .map(|rows| (true, FrdDepthLeaf::Measured(rows))),
    }
}

fn parse_country(
    tag: &str,
    country: &Map<String, Value>,
    contended_keys: &impl crate::world::ContendedKeys,
) -> Result<Option<FrdCountryDepth>, String> {
    let mut seen = false;
    let empty = Map::new();
    let focus = country
        .get("focus")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let research = country
        .get("research")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let decisions = country
        .get("decisions")
        .and_then(Value::as_object)
        .unwrap_or(&empty);

    let (present, focus_current) = leaf(
        focus,
        "_r2_depth_current",
        &format!("countries.{tag}.focus"),
    )?;
    seen |= present;
    let (present, focus_current_continuous) = leaf(
        focus,
        "_r2_depth_current_continuous",
        &format!("countries.{tag}.focus"),
    )?;
    seen |= present;
    let (present, focus_completed_count) = leaf(
        focus,
        "_r2_depth_completed_count",
        &format!("countries.{tag}.focus"),
    )?;
    seen |= present;
    let (present, focus_completed): (_, FrdDepthLeaf<Vec<String>>) = leaf(
        focus,
        "_r2_depth_completed",
        &format!("countries.{tag}.focus"),
    )?;
    seen |= present;
    let (present, focus_events): (_, FrdDepthLeaf<Vec<FrdFocusDepthEvent>>) =
        leaf(focus, "_r2_depth_events", &format!("countries.{tag}.focus"))?;
    seen |= present;
    let (present, research_in_progress_live): (_, FrdDepthLeaf<Vec<FrdResearchLiveRow>>) = leaf(
        research,
        "_r2_depth_in_progress_live",
        &format!("countries.{tag}.research"),
    )?;
    seen |= present;
    let (present, decision_status): (_, FrdDepthLeaf<FrdDecisionStatus>) = leaf(
        decisions,
        "_r2_depth_status",
        &format!("countries.{tag}.decisions"),
    )?;
    seen |= present;
    let (present, decision_targets) = target_leaf(
        decisions,
        "_r2_depth_targets",
        &format!("countries.{tag}.decisions"),
        contended_keys,
    )?;
    seen |= present;
    let (present, decision_events): (_, FrdDepthLeaf<Vec<FrdDecisionEvent>>) = leaf(
        decisions,
        "_r2_depth_events",
        &format!("countries.{tag}.decisions"),
    )?;
    seen |= present;

    if !seen {
        return Ok(None);
    }
    for (index, token) in match &focus_completed {
        FrdDepthLeaf::Measured(tokens) => tokens.as_slice(),
        _ => &[],
    }
    .iter()
    .enumerate()
    {
        require_nonblank(
            token,
            &format!("countries.{tag}.focus._r2_depth_completed[{index}]"),
        )?;
    }
    validate_focus_events(
        &focus_events,
        &format!("countries.{tag}.focus._r2_depth_events"),
    )?;
    validate_research_rows(
        &research_in_progress_live,
        &format!("countries.{tag}.research._r2_depth_in_progress_live"),
    )?;
    if let FrdDepthLeaf::Measured(status) = &decision_status {
        let counts = [
            status.active_timed_count,
            status.targeted_count,
            status.targeted_timed_count,
            status.decisions_taken_bucket_count,
            status.re_enable_count,
            status.timed_live_count,
        ];
        if counts.iter().any(|count| *count > 4_096) {
            return Err(format!(
                "countries.{tag}.decisions._r2_depth_status: impossible count"
            ));
        }
    }
    if let FrdDepthLeaf::Measured(events) = &decision_events {
        for (index, event) in events.iter().enumerate() {
            let event_path = format!("countries.{tag}.decisions._r2_depth_events[{index}]");
            if !(1..=7).contains(&event.kind) {
                return Err(format!(
                    "{event_path}.kind: unsupported value {}",
                    event.kind
                ));
            }
            if event.kind != 5 {
                require_nonblank(&event.decision, &format!("{event_path}.decision"))?;
            }
            if event.kind == 3 && event.previous_days == event.current_days {
                return Err(format!("{event_path}: day-delta event did not change days"));
            }
            if event.kind == 4 && event.previous_state == event.current_state {
                return Err(format!(
                    "{event_path}: state-change event did not change state"
                ));
            }
        }
    }
    Ok(Some(FrdCountryDepth {
        focus_current,
        focus_current_continuous,
        focus_completed_count,
        focus_completed,
        focus_events,
        research_in_progress_live,
        decision_status,
        decision_targets,
        decision_events,
    }))
}

pub(crate) fn build_frd_depth_annotations(
    rest: &impl WorldRoot,
    contended_keys: &impl crate::world::ContendedKeys,
    warnings: &mut Vec<String>,
) -> FrdDepthAnnotationsView {
    let result = (|| -> Result<FrdDepthAnnotationsView, String> {
        let mut view = FrdDepthAnnotationsView::absent();
        if let Some(countries) = rest.get("countries") {
            let countries = countries
                .as_object()
                .ok_or_else(|| "countries: expected object for D45 annotations".to_string())?;
            for (tag, value) in countries {
                let Some(country) = value.as_object() else {
                    continue;
                };
                if let Some(depth) = parse_country(tag, country, contended_keys)? {
                    view.countries.insert(tag.clone(), depth);
                }
            }
        }

        if let Some(provenance) = rest.get("provenance").and_then(Value::as_object) {
            if let Some(collector) = provenance.get("collector").and_then(Value::as_object) {
                let (present, badges): (_, FrdDepthLeaf<Vec<FrdDepthBadge>>) =
                    leaf(collector, "_r2_depth_badges", "provenance.collector")?;
                if present {
                    if let FrdDepthLeaf::Measured(rows) = &badges {
                        for (index, badge) in rows.iter().enumerate() {
                            require_nonblank(
                                &badge.detail,
                                &format!("provenance.collector._r2_depth_badges[{index}].detail"),
                            )?;
                        }
                    }
                    view.badges = badges;
                }
            }
        }
        // PUBLISHED, not WALKED, and before the status is decided: the walk
        // above read and validated every country entry the producer sent, and
        // a key two ids reach is not a country to file a depth annotation
        // under (Codex round 38).
        view.countries.retain(|tag, _| !contended_keys.holds(tag.as_str()));
        if !view.countries.is_empty() || view.badges != FrdDepthLeaf::Absent {
            view.status = ViewStatus::Present;
        }
        Ok(view)
    })();
    match result {
        Ok(view) => view,
        Err(error) => {
            warnings.push(format!(
                "D45 focus/research/decision depth annotations: {error}; entire view unavailable"
            ));
            FrdDepthAnnotationsView::unavailable()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn consumes_every_folded_depth_family_and_preserves_unobtained_targets() {
        let rest = json!({
            "countries": {
                "FRA": {
                    "focus": {
                        "_r2_depth_current": "FRA_focus",
                        "_r2_depth_current_continuous": "FRA_continuous",
                        "_r2_depth_completed_count": 1,
                        "_r2_depth_completed": ["FRA_done"],
                        "_r2_depth_events": [{
                            "kind": 1, "focus_kind": 0,
                            "previous_completed": 0, "current_completed": 1,
                            "previous_progress": 99, "current_progress": 100,
                            "completed_delta": 1, "from_token": "FRA_done",
                            "to_token": "", "completed_token": "FRA_done",
                            "completed_token_exact": true,
                            "completed_token_inferred": false, "paused": null
                        }]
                    },
                    "research": {
                        "_r2_depth_in_progress_live": [{
                            "slot_index": 0, "current_handle": 2, "current_def": 3,
                            "previous_handle": 2, "previous_def": 3,
                            "previous_progress_raw": 100, "current_progress_raw": 120,
                            "delta_raw": 20, "speed_raw": 5, "base_raw": 1000,
                            "eta_hours": 4, "hour_delta": 1, "flags": 39
                        }]
                    },
                    "decisions": {
                        "_r2_depth_status": {
                            "active_timed_count": 1, "targeted_count": 1,
                            "targeted_timed_count": 0,
                            "decisions_taken_bucket_count": 2,
                            "re_enable_count": 0, "timed_live_count": 1
                        },
                        "_r2_depth_targets": null,
                        "_r2_depth_events": [{
                            "kind": 1, "previous_days": 2, "current_days": 1,
                            "previous_state": 0, "current_state": 1,
                            "decision": "FRA_decision"
                        }]
                    }
                }
            },
            "provenance": {"collector": {"_r2_depth_badges": [{
                "kind": 3, "flags": 0, "source_domain_id": 45,
                "affected_count": 1, "evidence_code": 7, "detail": "measured badge"
            }]}}
        });
        let mut warnings = Vec::new();
        let view = build_frd_depth_annotations(
            rest.as_object().unwrap(),
            &BTreeSet::<String>::new(),
            &mut warnings,
        );
        assert_eq!(view.status, ViewStatus::Present);
        assert!(warnings.is_empty());
        assert!(matches!(
            view.countries["FRA"].decision_targets,
            FrdDepthLeaf::Unobtained
        ));
        assert!(matches!(view.badges, FrdDepthLeaf::Measured(ref rows) if rows.len() == 1));
    }

    /// The INNER country reference: an uncontended country's decision target.
    ///
    /// Round 38 filtered the outer map key -- the country the annotation hangs
    /// under -- and a decision target names a country of its own. GER is an
    /// ordinary country and its decision pointed at a key two ids reach, so the
    /// row published a target country that names neither (Codex round 39). The
    /// target is identified by `target_id`, so the row and its identity stay
    /// and the NAME is what goes.
    #[test]
    fn a_contended_decision_target_is_named_by_nobody() {
        let rest = json!({"countries": {"GER": {"decisions": {
            "_r2_depth_targets": [{
                "decision": "country_decision", "target_kind": "country",
                "target": "DUP", "target_tag_db_id": 441, "flags": 3,
                "source_kind": 2,
                "source": "CDecisionStatus+0x100 record+0x20",
                "lifecycle_state": 1, "lifecycle_state_name": "active",
                "ignore": false, "days": 3
            }]
        }}}});
        let rest = rest.as_object().expect("the fixture is an object");
        let mut warnings = Vec::new();

        let control =
            build_frd_depth_annotations(rest, &BTreeSet::<String>::new(), &mut warnings);
        let FrdDepthLeaf::Measured(targets) = &control.countries["GER"].decision_targets else {
            panic!("the control measures the target, or this proves nothing");
        };
        assert_eq!(targets[0].target_country.as_deref(), Some("DUP"));

        let census = BTreeSet::from(["DUP".to_string()]);
        let view = build_frd_depth_annotations(rest, &census, &mut warnings);
        assert!(
            view.countries.contains_key("GER"),
            "GER is an ordinary country and keeps its annotation"
        );
        let FrdDepthLeaf::Measured(targets) = &view.countries["GER"].decision_targets else {
            panic!("the row stays: it is identified by target_id");
        };
        assert_eq!(
            targets[0].target_country, None,
            "a key two ids reach is not a country to point a decision at"
        );
        assert_eq!(targets[0].target_id, 441, "and the id it points at stays");
    }

    #[test]
    fn consumes_country_and_numeric_decision_targets() {
        let target_common = json!({
            "lifecycle_state": 1, "lifecycle_state_name": "active",
            "ignore": false, "days": 3
        });
        let rest = json!({"countries": {"GER": {"decisions": {
            "_r2_depth_targets": [
                {
                    "decision": "country_decision", "target_kind": "country",
                    "target": "FRA", "target_tag_db_id": 9, "flags": 3,
                    "source_kind": 2,
                    "source": "CDecisionStatus+0x100 record+0x20",
                    "lifecycle_state": target_common["lifecycle_state"],
                    "lifecycle_state_name": target_common["lifecycle_state_name"],
                    "ignore": target_common["ignore"], "days": target_common["days"]
                },
                {
                    "decision": "numeric_decision", "target_kind": "numeric",
                    "target_id": 42, "flags": 12, "source_kind": 1,
                    "source": "CDecisionStatus+0xE8 record+0x24",
                    "lifecycle_state": null, "lifecycle_state_name": null,
                    "ignore": null, "days": null
                }
            ]
        }}}});
        let mut warnings = Vec::new();
        let view = build_frd_depth_annotations(
            rest.as_object().unwrap(),
            &BTreeSet::<String>::new(),
            &mut warnings,
        );
        let FrdDepthLeaf::Measured(targets) = &view.countries["GER"].decision_targets else {
            panic!("targets must be measured");
        };
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].target_kind, FrdDecisionTargetKind::Country);
        assert_eq!(targets[1].target_kind, FrdDecisionTargetKind::Numeric);
    }

    #[test]
    fn malformed_target_fails_closed_for_the_entire_view() {
        let rest = json!({"countries": {"GER": {"decisions": {
            "_r2_depth_targets": [{
                "decision": "bad", "target_kind": "country", "target": "FRA",
                "target_tag_db_id": 9, "flags": 12, "source_kind": 2,
                "source": "CDecisionStatus+0x100 record+0x20",
                "lifecycle_state": null, "lifecycle_state_name": null,
                "ignore": null, "days": null
            }]
        }}}});
        let mut warnings = Vec::new();
        let view = build_frd_depth_annotations(
            rest.as_object().unwrap(),
            &BTreeSet::<String>::new(),
            &mut warnings,
        );
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.countries.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn accepts_snapshot_focus_and_tokenless_status_count_events() {
        let rest = json!({"countries": {"GER": {
            "focus": {"_r2_depth_events": [{
                "kind": 7, "focus_kind": 1,
                "previous_completed": 3, "current_completed": 3,
                "previous_progress": 50, "current_progress": 50,
                "completed_delta": 0, "from_token": "", "to_token": "",
                "completed_token": "", "completed_token_exact": false,
                "completed_token_inferred": false, "paused": true
            }]},
            "decisions": {"_r2_depth_events": [{
                "kind": 5, "previous_days": 0, "current_days": 0,
                "previous_state": 0, "current_state": 0, "decision": ""
            }]}
        }}});
        let mut warnings = Vec::new();
        let view = build_frd_depth_annotations(
            rest.as_object().unwrap(),
            &BTreeSet::<String>::new(),
            &mut warnings,
        );
        assert_eq!(view.status, ViewStatus::Present);
        assert!(warnings.is_empty());
    }
}
