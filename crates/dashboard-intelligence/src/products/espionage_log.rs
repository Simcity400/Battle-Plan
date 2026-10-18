//! Every country's recorded operations. Reads supply values, never elapsed-time
//! estimates. A missing hour breaks runs; a complete later census can establish
//! only that an absent operation ended *by* that read.
//!
//! Operation completion writes terminal state, a generated outcome description
//! and independent result flags. Hooks retain outcomes even when the manager
//! deletes an AI instance. Terminal state never establishes success or
//! operative capture. OperationTokens is an independent owner-target token
//! set, added/removed by authored effects without an operation handle. It is
//! therefore no outcome source and is not decoded here.

use super::{
    intelligence_content::intelligence_content, operative_rows::membership_read,
    operative_rows::operations_read,
};
use dashboard_base::{
    products::{BranchParams, Product, country_groups::HourSpan},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::branch_bounds;
use replay::container::Replay;
use replay_data::{rows::OperationRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::validate_campaign;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, atomic::Ordering};

#[cfg(any(test, feature = "test-support"))]
mod oracle;
#[cfg(any(test, feature = "test-support"))]
use oracle::scan;

pub struct EspionageLog;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EspionageLogParams {
    pub branch_id: String,
    /// None requests the manifest; a detail supplies one daily transport window.
    pub window: Option<usize>,
}
impl EspionageLogParams {
    fn branch_params(&self) -> BranchParams {
        BranchParams {
            branch_id: self.branch_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationType {
    pub key: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperativeName {
    pub id: String,
    pub name: Option<String>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// First read of this equal-value run. read_hours also dates equal re-reads.
    pub read_hour: i32,
    /// Native resource-meter ratio, fixed5; not equipment readiness.
    pub preparation_ratio: Option<i32>,
    pub owner_tag: Option<String>,
    pub target_tag: Option<String>,
    /// Null when this read did not record the definition token's name.
    pub type_key: Option<String>,
    /// Selected definitions, not completed phases.
    pub phase_count: u32,
    /// Manager removal flag tested after completion.
    pub auto_repeat: bool,
    /// Launch writes duration and start on the same instance.
    pub duration_days: i32,
    /// All assignment slots, including unassigned ones.
    pub assigned_slots: u32,
    /// Signed day result at this read; -1 stays unknown.
    pub elapsed_days: Option<i32>,
    /// Zero-based native phase at the supplying read; names/IDs are not captured.
    pub current_phase: Option<u32>,
    pub terminal: bool,
    /// Auto-commence flag tested before the operation starts.
    pub auto_commence: bool,
    pub start_hour: Option<i32>,
    /// Non-null handles in native slot order; null means assignments unread.
    pub operatives: Option<Vec<String>>,
    /// Names from this operation read; another daily read is never backfilled.
    pub operative_names: Option<Vec<EspionageOperativeName>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EspionageEndReason {
    Completed,
    Disappeared,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOutcome {
    /// Stored independent trials, never success/failure.
    pub bonus: bool,
    pub detected: bool,
    /// Language of the stored description, not the viewer's display language.
    pub recorded_language: String,
    pub description_ref: Option<super::operation_description::OperationDescriptionRef>,
    pub description_unrecorded: Option<String>,
    /// Exact native byte length, including when capture refused the text.
    pub description_required_bytes: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationEnd {
    pub by_hour: i32,
    pub last_seen: i32,
    /// Exact hook date/sequence when supplied by OperationCompletions; snapshot
    /// terminal reads establish only by_hour.
    pub completion_hour: Option<i32>,
    pub completion_sequence: Option<String>,
    pub type_key: Option<String>,
    pub reason: EspionageEndReason,
    pub outcome: Option<EspionageOutcome>,
    pub outcome_unrecorded: Option<String>,
    pub owner_tag: Option<String>,
    pub target_tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperation {
    /// Full native handle and episode's first read. Auto-repeat creates a fresh
    /// identity.
    pub id: String,
    pub first_read: i32,
    pub start_hour: Option<i32>,
    pub start_known_at: Option<i32>,
    /// Raw lexical identity; the name, countries and mutable values belong to reads.
    pub definition_token: u32,
    pub runs: Vec<EspionageOperationRun>,
    pub end: Option<EspionageOperationEnd>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageLogReply {
    pub branch_id: String,
    pub window: Option<usize>,
    /// Only the manifest lists windows; details preserve actual supplying reads.
    pub windows: Vec<HourSpan>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub available: Vec<HourSpan>,
    pub read_hours: Vec<i32>,
    /// Completion batches are independent of the daily instance census.
    pub completions_available: Vec<HourSpan>,
    pub completions_unrecorded: Option<String>,
    pub types: Vec<EspionageOperationType>,
    pub operations: Vec<EspionageOperation>,
    pub warnings: Vec<String>,
    pub unavailable: Option<String>,
}

type OperationInstance = (usize, (u32, u32, u32));

#[derive(Default, Serialize, Deserialize)]
struct Builder {
    operations: Vec<EspionageOperation>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    open: HashMap<u64, OperationInstance>,
    available: Vec<HourSpan>,
    reads: Vec<i32>,
    types: BTreeSet<String>,
    warnings: BTreeMap<String, usize>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    native_countries: super::operative_rows::CountryInputs,
    language: String,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    instances: HashMap<u64, Vec<OperationInstance>>,
    completions: Vec<HourSpan>,
    #[serde(with = "dashboard_core::map_pairs")]
    seen_completions: HashMap<(u32, u64), super::operation_description::OperationDescriptionRef>,
    #[allow(clippy::type_complexity)]
    #[serde(skip)]
    changed: Option<BTreeMap<usize, (u64, (u32, u32, u32))>>,
    #[serde(skip)]
    new_completions: Vec<(u32, u64)>,
}

fn budget_error() -> io::Error {
    io::Error::other("Espionage metadata window exceeds the complete-response byte budget")
}

impl Builder {
    fn new(replay: &Replay) -> Self {
        Self {
            language: replay
                .identity
                .language
                .strip_prefix("l_")
                .unwrap_or(&replay.identity.language)
                .to_owned(),
            ..Self::default()
        }
    }

    fn matching(&self, handle: u64, fingerprint: (u32, u32, u32), hour: i32) -> Option<usize> {
        self.instances
            .get(&handle)?
            .iter()
            .rev()
            .find(|(index, identity)| {
                *identity == fingerprint
                    && self.operations[*index].first_read <= hour
                    && self.operations[*index]
                        .end
                        .as_ref()
                        .is_none_or(|end| end.completion_sequence.is_none())
            })
            .map(|(index, _)| *index)
    }

    fn warn(&mut self, reason: impl Into<String>) {
        *self.warnings.entry(reason.into()).or_default() += 1;
    }

    fn observe_completions_countries(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        branch: u64,
        countries: &super::operative_rows::CountryInputs,
    ) -> io::Result<()> {
        if !TableId::OperationCompletions
            .last_change()
            .present(replay.format.version())
        {
            return Ok(());
        }
        let rows = match frame.complete::<tables::OperationCompletions>() {
            Ok(rows) => rows,
            Err(_) => {
                self.warn(
                    "OperationCompletions unavailable or incomplete; completion coverage unknown",
                );
                return Ok(());
            }
        };
        let hour = frame.hour;
        if let Some(span) = self
            .completions
            .last_mut()
            .filter(|span| span.last_hour.checked_add(1) == Some(hour))
        {
            span.last_hour = hour;
        } else {
            self.completions.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            });
        }
        let labels = countries.complete().is_some();
        let language = replay
            .identity
            .language
            .strip_prefix("l_")
            .unwrap_or(&replay.identity.language);
        let mut repeated: BTreeMap<i32, Vec<&replay_data::rows::OperationCompletionRow>> =
            BTreeMap::new();
        for row in rows {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.row_visits += 1);
            let reference = super::operation_description::OperationDescriptionRef {
                hour,
                id: row.id.to_string(),
                owner: row.owner,
                sequence: Some(row.sequence.to_string()),
            };
            if let Some(previous) = self.seen_completions.get(&(row.owner, row.sequence)) {
                repeated.entry(previous.hour).or_default().push(row);
                continue;
            }
            if row.completion_hour > hour {
                return Err(dashboard_core::invalid(
                    "Operation completion occurs after its drain hour",
                ));
            }
            self.seen_completions
                .insert((row.owner, row.sequence), reference.clone());
            if self.changed.is_some() {
                self.new_completions.push((row.owner, row.sequence));
            }
            let key = super::intelligence_capture::literal(frame, Some(row.definition_token));
            if let Some(key) = &key {
                self.types.insert(key.clone());
            }
            let fingerprint = (row.owner, row.target, row.definition_token);
            let matched = self.matching(row.id, fingerprint, row.completion_hour);
            let index = match matched {
                Some(index) => index,
                None => {
                    let index = self.operations.len();
                    self.operations.push(EspionageOperation {
                        id: format!("{}:completion:{}", row.id, row.sequence),
                        first_read: hour,
                        start_hour: None,
                        start_known_at: None,
                        definition_token: row.definition_token,
                        runs: Vec::new(),
                        end: None,
                    });
                    self.instances
                        .entry(row.id)
                        .or_default()
                        .push((index, fingerprint));
                    index
                }
            };
            if let Some(changed) = &mut self.changed {
                changed.insert(index, (row.id, fingerprint));
            }
            let (outcome, outcome_unrecorded) = outcome_at_end(
                Some(row.bonus),
                Some(row.detected),
                Some(reference),
                row.description_refusal(),
                Some(row.description_required),
                Some(language),
                &mut self.warnings,
            );
            let operation = &mut self.operations[index];
            let last_seen = operation
                .runs
                .last()
                .map_or(hour, |run| supplying_read(&self.reads, run.last_hour));
            operation.end = Some(EspionageOperationEnd {
                by_hour: hour,
                last_seen,
                completion_hour: Some(row.completion_hour),
                completion_sequence: Some(row.sequence.to_string()),
                type_key: key,
                reason: EspionageEndReason::Completed,
                outcome,
                outcome_unrecorded,
                owner_tag: labels
                    .then(|| countries.labels.get(&row.owner).cloned())
                    .flatten(),
                target_tag: labels
                    .then(|| countries.labels.get(&row.target).cloned())
                    .flatten(),
            });
        }
        for (witness_hour, rows) in repeated {
            if !super::operation_description::completions_match(
                replay,
                witness_hour,
                &rows,
                branch,
            )? {
                return Err(dashboard_core::invalid(
                    "Operation completion immutable key changed",
                ));
            }
        }
        Ok(())
    }

    fn mark_available(&mut self, hour: i32) {
        if let Some(span) = self
            .available
            .last_mut()
            .filter(|span| span.last_hour.checked_add(1) == Some(hour))
        {
            span.last_hour = hour;
        } else {
            self.available.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            });
        }
    }

    fn observe_countries(
        &mut self,
        frame: &DecodedFrame,
        countries: &super::operative_rows::CountryInputs,
    ) {
        let Ok(rows) = frame.complete::<tables::Operations>() else {
            self.warn("Operations unavailable or incomplete; values and endings unknown");
            return;
        };
        let hour = frame.hour;
        if !operations_read(frame) {
            // Decoder carries the successful daily census on skipped hours.
            // Extend only consecutive recorded hours; do not recompute phases,
            // dates or progress and never resume a carry after a gap/failed read.
            if self
                .available
                .last()
                .is_some_and(|span| span.last_hour.checked_add(1) == Some(hour))
            {
                for (index, _) in self.open.values() {
                    let run = self.operations[*index]
                        .runs
                        .last_mut()
                        .expect("observed operation");
                    if run.last_hour.checked_add(1) == Some(hour) {
                        run.last_hour = hour;
                    }
                }
                self.mark_available(hour);
            }
            return;
        }
        // A malformed duplicate census is not evidence of absence or zero.
        let mut ids = BTreeSet::new();
        if rows.iter().any(|row| !ids.insert(row.id)) {
            self.warn("Duplicate operation handles refused");
            return;
        }
        let labels = countries.complete().is_some();
        if !labels {
            self.warn("Country identities unavailable at operation read");
        }
        self.reads.push(hour);
        self.mark_available(hour);
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        if lexical.is_none() {
            self.warn("LexicalTokens unavailable at operation read");
        }
        let assignments = frame.complete::<tables::OperationAssignments>().ok();
        let mut assigned: HashMap<u64, Vec<_>> = HashMap::new();
        if let Some(assignments) = assignments {
            for assignment in assignments {
                assigned
                    .entry(assignment.operation)
                    .or_default()
                    .push(assignment);
            }
        }
        for slots in assigned.values_mut() {
            slots.sort_unstable_by_key(|slot| slot.ordinal);
        }
        let mut named: HashMap<u64, Option<&replay_data::rows::OperativeMembershipRow>> =
            HashMap::new();
        let names_read = frame
            .complete::<tables::OperativeMemberships>()
            .ok()
            .filter(|_| membership_read(frame));
        if let Some(rows) = names_read {
            for row in rows {
                named
                    .entry(row.id)
                    .and_modify(|previous| {
                        if previous.is_some_and(|previous| previous.name != row.name) {
                            *previous = None;
                        }
                    })
                    .or_insert(Some(row));
            }
        }
        let absent: Vec<_> = self
            .open
            .keys()
            .filter(|id| !ids.contains(id))
            .copied()
            .collect();
        for id in absent {
            let (index, fingerprint) = self.open.remove(&id).expect("open handle");
            if let Some(changed) = &mut self.changed {
                changed.insert(index, (id, fingerprint));
            }
            let (owner, target, _) = fingerprint;
            let operation = &mut self.operations[index];
            if operation.end.is_none() {
                let (outcome, outcome_unrecorded) =
                    outcome_at_end(None, None, None, None, None, None, &mut self.warnings);
                operation.end = Some(EspionageOperationEnd {
                    by_hour: hour,
                    completion_hour: None,
                    completion_sequence: None,
                    type_key: operation.runs.last().and_then(|run| run.type_key.clone()),
                    last_seen: supplying_read(
                        &self.reads,
                        operation.runs.last().expect("observed operation").last_hour,
                    ),
                    reason: EspionageEndReason::Disappeared,
                    outcome,
                    outcome_unrecorded,
                    owner_tag: labels
                        .then(|| countries.labels.get(&owner).cloned())
                        .flatten(),
                    target_tag: labels
                        .then(|| countries.labels.get(&target).cloned())
                        .flatten(),
                });
            }
        }
        for row in rows {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.row_visits += 1);
            let name = lexical.and_then(|tokens| {
                tokens
                    .binary_search_by_key(&row.definition_token, |token| token.token)
                    .ok()
                    .map(|index| &tokens[index])
            });
            let key = match name {
                None => {
                    if lexical.is_some() {
                        self.warn(format!(
                            "Operation definition token {} not recorded",
                            row.definition_token
                        ));
                    }
                    None
                }
                Some(token) => match std::str::from_utf8(&token.literal) {
                    Ok("") => {
                        self.warn(format!(
                            "Operation definition token {} has an empty recorded name",
                            row.definition_token
                        ));
                        None
                    }
                    Ok(text) => Some(text.to_owned()),
                    Err(_) => {
                        self.warn(format!("Operation definition token {} has unsupported non-text recorded name bytes", row.definition_token));
                        None
                    }
                },
            };
            if let Some(key) = &key {
                self.types.insert(key.clone());
            }
            let slots = assigned.get(&row.id);
            let operatives = assignments.and_then(|_| {
                let slots = slots.map(Vec::as_slice).unwrap_or_default();
                // The capture retains empty slots. A missing ordinal is unknown.
                (slots.len() == row.assigned_slots as usize
                    && slots
                        .iter()
                        .enumerate()
                        .all(|(i, slot)| slot.ordinal as usize == i))
                .then(|| {
                    slots
                        .iter()
                        .filter(|slot| slot.operative != 0)
                        .map(|slot| slot.operative.to_string())
                        .collect::<Vec<_>>()
                })
            });
            let operative_names = names_read.and(operatives.as_ref()).map(|operatives| {
                operatives
                    .iter()
                    .map(|id| {
                        let bytes = named
                            .get(&id.parse::<u64>().expect("recorded handle"))
                            .copied()
                            .flatten()
                            .ok_or_else(|| {
                                "Assigned operative name missing or contradictory at this read"
                                    .to_owned()
                            })
                            .and_then(|row| row.name.require("OperativeMemberships.name"))
                            .and_then(|bytes| {
                                std::str::from_utf8(installed_content_core::parser::c_string(bytes))
                                    .map(str::to_owned)
                                    .map_err(|_| {
                                        "Assigned operative name has non-text bytes".to_owned()
                                    })
                            });
                        match bytes {
                            Ok(name) => EspionageOperativeName {
                                id: id.clone(),
                                name: Some(name),
                                unavailable: None,
                            },
                            Err(reason) => EspionageOperativeName {
                                id: id.clone(),
                                name: None,
                                unavailable: Some(reason),
                            },
                        }
                    })
                    .collect()
            });
            let owner_tag = labels
                .then(|| countries.labels.get(&row.owner).cloned())
                .flatten();
            let target_tag = labels
                .then(|| countries.labels.get(&row.target).cloned())
                .flatten();
            let run = EspionageOperationRun {
                first_hour: hour,
                last_hour: hour,
                read_hour: hour,
                preparation_ratio: preparation_ratio(row),
                owner_tag: owner_tag.clone(),
                target_tag: target_tag.clone(),
                type_key: key,
                phase_count: row.phase_count,
                auto_repeat: row.auto_repeat != 0,
                duration_days: row.duration_days,
                assigned_slots: row.assigned_slots,
                elapsed_days: elapsed_days(row, hour),
                current_phase: current_phase(row, hour),
                terminal: row.terminal != 0,
                auto_commence: row.flag_40 != 0,
                start_hour: (row.duration_days != 0).then_some(row.start_hour),
                operatives,
                operative_names,
            };
            let fingerprint = (row.owner, row.target, row.definition_token);
            let continuing = self
                .open
                .get(&row.id)
                .filter(|(index, old)| {
                    if *old != fingerprint {
                        return false;
                    }
                    let operation = &self.operations[*index];
                    let previous = operation.runs.last().expect("open operation");
                    // The raw identity survives launch, toggles and unread names.
                    // Only a finished instance reopening creates another episode.
                    run.terminal
                        || !previous.terminal
                            && operation
                                .end
                                .as_ref()
                                .is_none_or(|end| end.completion_sequence.is_none())
                })
                .map(|(index, _)| *index);
            let index = match continuing {
                Some(index) => index,
                None => {
                    if let Some((old, previous_fingerprint)) = self.open.remove(&row.id) {
                        if let Some(changed) = &mut self.changed {
                            changed.insert(old, (row.id, previous_fingerprint));
                        }
                        let previous = &mut self.operations[old];
                        if previous.end.is_none() {
                            let (outcome, outcome_unrecorded) = outcome_at_end(
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                &mut self.warnings,
                            );
                            previous.end = Some(EspionageOperationEnd {
                                by_hour: hour,
                                completion_hour: None,
                                completion_sequence: None,
                                type_key: previous.runs.last().and_then(|run| run.type_key.clone()),
                                last_seen: supplying_read(
                                    &self.reads,
                                    previous.runs.last().expect("observed operation").last_hour,
                                ),
                                reason: EspionageEndReason::Disappeared,
                                outcome,
                                outcome_unrecorded,
                                owner_tag: labels
                                    .then(|| countries.labels.get(&previous_fingerprint.0).cloned())
                                    .flatten(),
                                target_tag: labels
                                    .then(|| countries.labels.get(&previous_fingerprint.1).cloned())
                                    .flatten(),
                            });
                        }
                    }
                    let index = self.operations.len();
                    self.operations.push(EspionageOperation {
                        id: format!("{}:{hour}", row.id),
                        first_read: hour,
                        start_hour: run.start_hour,
                        start_known_at: run.start_hour.map(|_| hour),
                        definition_token: row.definition_token,
                        runs: Vec::new(),
                        end: None,
                    });
                    self.instances
                        .entry(row.id)
                        .or_default()
                        .push((index, fingerprint));
                    self.open.insert(row.id, (index, fingerprint));
                    index
                }
            };
            if let Some(changed) = &mut self.changed {
                changed.insert(index, (row.id, fingerprint));
            }
            let operation = &self.operations[index];
            let same = operation.runs.last().is_some_and(|previous| {
                previous.last_hour.checked_add(1) == Some(hour) && previous.same_values(&run)
            });
            let operation = &mut self.operations[index];
            if operation.start_hour.is_none() && run.start_hour.is_some() {
                operation.start_hour = run.start_hour;
                operation.start_known_at = Some(hour);
            }
            if run.terminal && operation.end.is_none() {
                let description = row
                    .effect_description
                    .require("Operations.effect_description")
                    .ok();
                let (outcome, outcome_unrecorded) = outcome_at_end(
                    row.bonus.require("Operations.bonus").ok().copied(),
                    row.detected.require("Operations.detected").ok().copied(),
                    description.map(|_| super::operation_description::OperationDescriptionRef {
                        hour,
                        id: row.id.to_string(),
                        owner: row.owner,
                        sequence: None,
                    }),
                    None,
                    description.map(|bytes| bytes.len() as u64),
                    Some(&self.language),
                    &mut self.warnings,
                );
                operation.end = Some(EspionageOperationEnd {
                    by_hour: hour,
                    last_seen: hour,
                    completion_hour: None,
                    completion_sequence: None,
                    type_key: run.type_key.clone(),
                    reason: EspionageEndReason::Completed,
                    outcome,
                    outcome_unrecorded,
                    owner_tag,
                    target_tag,
                });
            }
            if same {
                operation
                    .runs
                    .last_mut()
                    .expect("equal previous run")
                    .last_hour = hour;
            } else {
                operation.runs.push(run);
            }
        }
    }
}

impl EspionageOperationRun {
    fn same_values(&self, other: &Self) -> bool {
        self.preparation_ratio == other.preparation_ratio
            && self.owner_tag == other.owner_tag
            && self.target_tag == other.target_tag
            && self.type_key == other.type_key
            && self.phase_count == other.phase_count
            && self.auto_repeat == other.auto_repeat
            && self.duration_days == other.duration_days
            && self.assigned_slots == other.assigned_slots
            && self.elapsed_days == other.elapsed_days
            && self.current_phase == other.current_phase
            && self.terminal == other.terminal
            && self.auto_commence == other.auto_commence
            && self.start_hour == other.start_hour
            && self.operatives == other.operatives
            && self.operative_names == other.operative_names
    }
}

fn supplying_read(reads: &[i32], hour: i32) -> i32 {
    reads[reads.partition_point(|read| *read <= hour) - 1]
}

/// Recorded total civilian days (fixed5) and elapsed civilian work supply this
/// aggregate ratio; equipment requirements and readiness are not captured.
/// The resource meter uses signed wrapping multiplication and division,
/// a zero-denominator zero, and clamp [0,100000]. Its three fixed5 sentinels
/// go through u32::MAX and clamp to a full meter.
fn preparation_ratio(row: &OperationRow) -> Option<i32> {
    let current = row.preparation_current;
    let ratio = if row.preparation_required == 0 {
        0
    } else if [i64::MIN, i64::MIN + 1, i64::MAX].contains(&current) {
        100_000
    } else {
        current
            .wrapping_mul(100_000)
            .checked_div(row.preparation_required)?
            .clamp(0, 100_000)
    };
    // Float is geometry only. The fixed5 formatter displays integer percent,
    // ratio / 1000, with precision zero and truncation.
    Some(ratio as i32)
}

/// Elapsed days uses separately truncated signed dates, offset by the
/// game's epoch. The phase view wraps the signed product and divides by
/// duration. Phase selection rejects -1 and indices >= count; an unsafe
/// negative index has no representable phase here. This runs only at a read,
/// never between reads.
fn elapsed_days(row: &OperationRow, hour: i32) -> Option<i32> {
    if row.duration_days == 0 || row.terminal != 0 {
        return None;
    }
    // Game epoch constant, not content: signed i32 subtraction and /24
    // truncate toward zero separately.
    let day = |hour: i32| hour.wrapping_sub(43_800_000) / 24;
    let elapsed = day(hour).wrapping_sub(day(row.start_hour));
    (elapsed != -1 && elapsed != row.duration_days).then_some(elapsed)
}

fn current_phase(row: &OperationRow, hour: i32) -> Option<u32> {
    let elapsed = elapsed_days(row, hour)?;
    let phase = elapsed
        .wrapping_mul(row.phase_count as i32)
        .checked_div(row.duration_days)?;
    u32::try_from(phase)
        .ok()
        .filter(|phase| *phase < row.phase_count)
}

/// Completion writes independent result flags. Exact client-language
/// description text is fetched separately for the active tooltip.
fn outcome_at_end(
    bonus: Option<u8>,
    detected: Option<u8>,
    reference: Option<super::operation_description::OperationDescriptionRef>,
    description_reason: Option<&str>,
    required: Option<u64>,
    language: Option<&str>,
    warnings: &mut BTreeMap<String, usize>,
) -> (Option<EspionageOutcome>, Option<String>) {
    let mut refusals = Vec::new();
    for (field, value) in [("bonus", bonus), ("detected", detected)] {
        if let Some(value) = value.filter(|value| *value > 1) {
            let reason =
                format!("Stored operation {field} refused: flag byte {value} is not 0 or 1");
            *warnings.entry(reason.clone()).or_default() += 1;
            refusals.push(reason);
        }
    }
    if !refusals.is_empty() {
        return (None, Some(refusals.join("; ")));
    }
    let (Some(bonus), Some(detected), Some(reference), Some(required), Some(language)) =
        (bonus, detected, reference, required, language)
    else {
        return (None, Some("Outcome not recorded: stored bonus, detection and effect description were not captured".into()));
    };
    if let Some(reason) = description_reason {
        *warnings.entry(reason.to_owned()).or_default() += 1;
    }
    (
        Some(EspionageOutcome {
            bonus: bonus == 1,
            detected: detected == 1,
            recorded_language: language.to_owned(),
            description_ref: description_reason.is_none().then_some(reference),
            description_unrecorded: description_reason.map(str::to_owned),
            description_required_bytes: required.to_string(),
        }),
        None,
    )
}

impl Builder {
    fn reply(
        &self,
        branch_id: String,
        first_hour: i32,
        last_hour: i32,
        format: u16,
    ) -> EspionageLogReply {
        EspionageLogReply {
            branch_id,
            window: None,
            windows: Vec::new(),
            first_hour,
            last_hour,
            available: self.available.clone(),
            read_hours: self.reads.clone(),
            completions_available: self.completions.clone(),
            completions_unrecorded: (!TableId::OperationCompletions.last_change().present(format))
                .then(|| "Operation completion events not recorded in this format".into()),
            types: self
                .types
                .iter()
                .map(|key| EspionageOperationType {
                    key: key.clone(),
                    name: None,
                    icon: None,
                    icon_missing: Some("Operation content unavailable".into()),
                })
                .collect(),
            operations: self.operations.clone(),
            warnings: self
                .warnings
                .iter()
                .map(|(reason, count)| format!("{reason} ({count} observations)."))
                .collect(),
            unavailable: self.available.is_empty().then(|| {
                "Operations was unavailable, incomplete or never read in this recording.".into()
            }),
        }
    }
}

pub mod prepared;

impl Product for EspionageLog {
    const KEY: &'static str = "espionage-log";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EspionageLogParams;
    type Reply = EspionageLogReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_params().validate()
    }
    fn types() -> Vec<String> {
        vec![
            EspionageLogParams::decl(),
            HourSpan::decl(),
            EspionageOperationType::decl(),
            EspionageOperationRun::decl(),
            EspionageOperativeName::decl(),
            EspionageEndReason::decl(),
            EspionageOutcome::decl(),
            EspionageOperationEnd::decl(),
            EspionageOperation::decl(),
            EspionageLogReply::decl(),
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        let branch_params = params.branch_params();
        if cache.branch != Some(branch_params.branch()) {
            cache.raw = Some(Arc::new(scan(&source.replay(), &branch_params, cancelled)?));
            cache.branch = Some(branch_params.branch());
        }
        let raw = Arc::clone(cache.raw.as_ref().expect("scanned branch"));
        drop(cache);
        let mut reply = metadata_window(&raw, params.window)?;
        present(source, context, cancelled, &mut reply)?;
        Ok(reply)
    }
}

fn present(
    source: &Source,
    context: &LoadContext,
    cancelled: &AtomicBool,
    reply: &mut EspionageLogReply,
) -> io::Result<()> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    if !reply.types.is_empty()
        || reply.operations.iter().any(|operation| {
            operation
                .runs
                .iter()
                .any(|run| run.operative_names.is_some())
        })
    {
        match context
            .content()
            .and_then(|options| intelligence_content(source, options))
        {
            Ok(content) => {
                for operation_type in &mut reply.types {
                    let (artwork, missing) = content.operation_artwork(&operation_type.key);
                    operation_type.name = artwork.name;
                    operation_type.icon = artwork.icon;
                    operation_type.icon_missing = missing;
                }
                let mut names = HashMap::new();
                for name in reply
                    .operations
                    .iter_mut()
                    .flat_map(|operation| &mut operation.runs)
                    .flat_map(|run| run.operative_names.iter_mut().flatten())
                {
                    if let Some(raw) = name.name.take() {
                        let resolved = names.entry(raw.clone()).or_insert_with(|| {
                            match dashboard_text::catalogue::localised_bytes(
                                &content.catalogue.localisation,
                                raw.as_bytes(),
                                dashboard_text::catalogue::View::Plain,
                            ) {
                                Ok(name) => (Some(name), None),
                                Err(reason) => (None, Some(reason)),
                            }
                        });
                        name.name = resolved.0.clone();
                        name.unavailable = resolved.1.clone();
                    }
                }
                reply.warnings.extend(content.warnings.iter().cloned());
                reply.warnings.extend(content.catalogue.refusal_lines());
            }
            Err(reason) => {
                for name in reply
                    .operations
                    .iter_mut()
                    .flat_map(|operation| &mut operation.runs)
                    .flat_map(|run| run.operative_names.iter_mut().flatten())
                {
                    if let Some(raw) = name.name.take() {
                        match dashboard_text::catalogue::localised_bytes(
                            &installed_content_localisation::localisation::Localisation::default(),
                            raw.as_bytes(),
                            dashboard_text::catalogue::View::Plain,
                        ) {
                            Ok(value) => name.name = Some(value),
                            Err(reason) => name.unavailable = Some(reason),
                        }
                    }
                }
                reply.warnings.push(format!("Operation content unavailable: {reason}; operative names show their saved literal/key"));
            }
        }
    }
    let mut name_refusals = BTreeMap::new();
    for name in reply
        .operations
        .iter()
        .flat_map(|operation| &operation.runs)
        .flat_map(|run| run.operative_names.iter().flatten())
    {
        if let Some(reason) = &name.unavailable {
            *name_refusals.entry(reason).or_insert(0_usize) += 1;
        }
    }
    reply.warnings.extend(
        name_refusals
            .into_iter()
            .map(|(reason, count)| format!("{reason} ({count} operative-name observations).")),
    );
    check_reply_budget(reply)?;
    Ok(())
}

/// Transport windows follow the daily census granularity, independent of native
/// allocation or recording block size. There is no country/episode/run cap.
fn metadata_windows(first: i32, last: i32) -> Vec<HourSpan> {
    let mut windows = Vec::new();
    let mut hour = i64::from(first);
    while hour <= i64::from(last) {
        let end = (hour + 23).min(i64::from(last));
        windows.push(HourSpan {
            first_hour: hour as i32,
            last_hour: end as i32,
        });
        hour = end + 1;
    }
    windows
}
fn clipped(spans: &[HourSpan], first: i32, last: i32) -> Vec<HourSpan> {
    spans
        .iter()
        .filter_map(|span| {
            let first_hour = span.first_hour.max(first);
            let last_hour = span.last_hour.min(last);
            (first_hour <= last_hour).then_some(HourSpan {
                first_hour,
                last_hour,
            })
        })
        .collect()
}
fn metadata_window(
    raw: &EspionageLogReply,
    window: Option<usize>,
) -> io::Result<EspionageLogReply> {
    let (first, last) = if let Some(index) = window {
        let first = i64::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(24))
            .and_then(|offset| i64::from(raw.first_hour).checked_add(offset))
            .filter(|first| *first <= i64::from(raw.last_hour))
            .ok_or_else(|| {
                dashboard_core::invalid("Espionage metadata window not in this branch")
            })?;
        (
            first as i32,
            (first + 23).min(i64::from(raw.last_hour)) as i32,
        )
    } else {
        (raw.first_hour, raw.last_hour)
    };
    let mut reply = EspionageLogReply {
        branch_id: raw.branch_id.clone(),
        window,
        windows: if window.is_none() {
            metadata_windows(raw.first_hour, raw.last_hour)
        } else {
            Vec::new()
        },
        first_hour: first,
        last_hour: last,
        available: if window.is_none() {
            raw.available.clone()
        } else {
            clipped(&raw.available, first, last)
        },
        read_hours: Vec::new(),
        completions_available: if window.is_none() {
            raw.completions_available.clone()
        } else {
            clipped(&raw.completions_available, first, last)
        },
        completions_unrecorded: raw.completions_unrecorded.clone(),
        types: raw.types.clone(),
        operations: Vec::new(),
        warnings: raw.warnings.clone(),
        unavailable: raw.unavailable.clone(),
    };
    if window.is_none() {
        return Ok(reply);
    }
    // One actual previous read dates carried values; never fabricate a window read.
    if let Some(read) = raw.read_hours.iter().rev().find(|read| **read < first) {
        reply.read_hours.push(*read);
    }
    reply.read_hours.extend(
        raw.read_hours
            .iter()
            .copied()
            .filter(|read| *read >= first && *read <= last),
    );
    let mut anchors = BTreeSet::new();
    for operation in &raw.operations {
        if operation.first_read > last
            || operation
                .end
                .as_ref()
                .is_some_and(|end| end.by_hour < first)
        {
            continue;
        }
        let mut projected = EspionageOperation {
            id: operation.id.clone(),
            first_read: operation.first_read,
            start_hour: operation.start_hour,
            start_known_at: operation.start_known_at,
            definition_token: operation.definition_token,
            runs: Vec::new(),
            end: operation.end.clone(),
        };
        // An ending names its actual supplying read, which can precede a run
        // clipped at this window. Keep that witness, or one previous identity.
        let witness = operation
            .end
            .as_ref()
            .filter(|end| end.by_hour <= last && end.last_seen < first)
            .and_then(|end| {
                operation
                    .runs
                    .iter()
                    .find(|run| run.first_hour <= end.last_seen && end.last_seen <= run.last_hour)
                    .map(|run| (run, end.last_seen))
            });
        let anchor = witness.or_else(|| {
            operation
                .runs
                .iter()
                .rev()
                .find(|run| run.last_hour < first)
                .map(|run| (run, run.last_hour))
        });
        if let Some((run, hour)) = anchor {
            let mut anchor = run.clone();
            anchor.first_hour = hour;
            anchor.last_hour = hour;
            anchors.insert(hour);
            projected.runs.push(anchor);
        }
        projected
            .runs
            .extend(operation.runs.iter().filter_map(|run| {
                let first_hour = run.first_hour.max(first);
                let last_hour = run.last_hour.min(last);
                if first_hour > last_hour {
                    return None;
                }
                let mut run = run.clone();
                run.first_hour = first_hour;
                run.last_hour = last_hour;
                Some(run)
            }));
        if projected.end.as_ref().is_some_and(|end| end.by_hour > last) {
            projected.end = None;
        }
        reply.operations.push(projected);
    }
    // Anchor availability is identity history only, outside the requested window.
    let mut available: Vec<HourSpan> = anchors
        .into_iter()
        .map(|hour| HourSpan {
            first_hour: hour,
            last_hour: hour,
        })
        .collect();
    available.append(&mut reply.available);
    for span in available {
        if let Some(previous) = reply
            .available
            .last_mut()
            .filter(|previous| i64::from(previous.last_hour) + 1 >= i64::from(span.first_hour))
        {
            previous.last_hour = previous.last_hour.max(span.last_hour);
        } else {
            reply.available.push(span);
        }
    }
    Ok(reply)
}

fn check_reply_budget(reply: &EspionageLogReply) -> io::Result<()> {
    dashboard_base::products::check_reply_budget(reply).map_err(|_| budget_error())
}

#[cfg(any(test, feature = "test-support"))]
/// Retains the current branch scan; content changes refresh presentation independently.
#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    raw: Option<Arc<EspionageLogReply>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
    use dashboard_core::transport::json_len;
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableRows};
    use replay_frame::frame::TableView;

    pub(super) fn frame(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
        // Successful explicit reads also occur away from the scheduled slot.
        frame.metrics.table_ns[TableId::Operations.index()] = 1;
        *frame.tables[TableId::OperationCompletions.index()] =
            TableView::Complete(TableRows::OperationCompletions(Vec::new()));
        *frame.tables[TableId::OperationAssignments.index()] =
            TableView::Complete(TableRows::OperationAssignments(Vec::new()));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(Vec::new()));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=count)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("mod{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=count)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(
            (1..=count)
                .map(|id| OperationRow {
                    id: u64::from(id),
                    owner: id,
                    target: count + 1 - id,
                    definition_token: 70,
                    bonus: 0.into(),
                    detected: 0.into(),
                    effect_description: Vec::new().into(),
                    preparation_current: 25,
                    preparation_required: 100,
                    ..Default::default()
                })
                .collect(),
        ));
        frame
    }

    #[test]
    fn espionage_terminal_rereads_keep_the_first_known_ending_and_description() {
        let mut builder = Builder::default();
        let mut sample = frame(100, 1);
        let mut row = sample.complete::<tables::Operations>().unwrap()[0].clone();
        row.terminal = 1;
        row.bonus = 1.into();
        row.detected = 0.into();
        row.effect_description = b"first stored outcome".to_vec().into();
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row.clone()]));
        builder.observe(&sample);
        sample.hour = 101;
        row.effect_description = b"later stored outcome".to_vec().into();
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row]));
        builder.observe(&sample);
        let end = builder.operations[0].end.as_ref().unwrap();
        assert_eq!(end.by_hour, 100);
        assert_eq!(end.last_seen, 100);
        assert_eq!(
            end.outcome
                .as_ref()
                .unwrap()
                .description_ref
                .as_ref()
                .unwrap()
                .hour,
            100
        );
    }

    #[test]
    fn espionage_completions_override_disappearance_and_keep_deleted_instances_without_bulk_text() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records};
        use replay_data::rows::OperationCompletionRow;
        let mut initial = frame(100, 1);
        *initial.tables[TableId::OperationCompletions.index()] =
            TableView::Complete(TableRows::OperationCompletions(Vec::new()));
        let mut ended = frame(101, 1);
        *ended.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        let text = vec![b'x'; (1 << 20) + 17];
        let event = OperationCompletionRow {
            owner: 1,
            sequence: 1,
            id: 1,
            target: 1,
            definition_token: 70,
            completion_hour: 101,
            terminal: 1,
            bonus: 1,
            detected: 0,
            description_required: text.len() as u64,
            effect_description: text,
            ..Default::default()
        };
        let deleted = OperationCompletionRow {
            sequence: 2,
            id: 44,
            effect_description: Vec::new(),
            description_required: 0,
            ..event.clone()
        };
        *ended.tables[TableId::OperationCompletions.index()] =
            TableView::Complete(TableRows::OperationCompletions(vec![
                event.clone(),
                deleted.clone(),
            ]));
        let mut repeat = ended.clone();
        repeat.hour = 102;
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![initial.clone(), ended.clone(), repeat.clone()],
            )],
        )
        .unwrap();
        let params = BranchParams {
            branch_id: "1".into(),
        };
        let reply = scan(&replay, &params, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.operations.len(), 2);
        let end = reply.operations[0].end.as_ref().unwrap();
        assert_eq!(end.completion_hour, Some(101));
        assert_eq!(end.completion_sequence.as_deref(), Some("1"));
        assert!(matches!(end.reason, EspionageEndReason::Completed));
        assert!(end.outcome.as_ref().unwrap().description_ref.is_some());
        assert_eq!(
            end.outcome
                .as_ref()
                .unwrap()
                .description_ref
                .as_ref()
                .unwrap()
                .hour,
            101
        );
        assert!(reply.operations[1].runs.is_empty());
        assert_eq!(reply.operations[1].start_hour, None);
        assert!(json_len(&reply, 16 * 1024).is_some());
        let mut changed = event;
        changed.bonus = 0;
        *repeat.tables[TableId::OperationCompletions.index()] =
            TableView::Complete(TableRows::OperationCompletions(vec![changed, deleted]));
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial, ended, repeat])])
                .unwrap();
        assert!(
            scan(&replay, &params, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("immutable key changed")
        );
    }

    #[test]
    fn espionage_held_scheduled_and_peer_memberships_do_not_supply_operative_names() {
        use replay_data::{rows::OperationAssignmentRow, rows::OperativeMembershipRow};
        let mut first = frame(15, 1);
        let mut operation = first.complete::<tables::Operations>().unwrap()[0].clone();
        operation.assigned_slots = 1;
        *first.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![operation]));
        *first.tables[TableId::OperationAssignments.index()] =
            TableView::Complete(TableRows::OperationAssignments(vec![
                OperationAssignmentRow {
                    operation: 1,
                    ordinal: 0,
                    operative: 77,
                    selection: 0,
                },
            ]));
        let member = OperativeMembershipRow {
            id: 77,
            name: b"FIRST_RECORDED_NAME".to_vec().into(),
            ..Default::default()
        };
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![member.clone()]));
        first.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        let mut scheduled = first.clone();
        scheduled.hour = 16;
        scheduled.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        assert!(replay_data::cadence::is_refresh(
            TableId::OperativeMemberships,
            scheduled.hour
        ));
        let mut peer = scheduled.clone();
        peer.hour = 17;
        peer.metrics.table_ns[TableId::WingCumulative.index()] = 1;
        assert_eq!(
            replay_data::cadence::cadence(TableId::WingCumulative),
            replay_data::cadence::cadence(TableId::OperativeMemberships)
        );
        let mut refreshed = first.clone();
        refreshed.hour = 18;
        *refreshed.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![
                OperativeMembershipRow {
                    name: b"FRESH_RECORDED_NAME".to_vec().into(),
                    ..member
                },
            ]));
        let mut builder = Builder::default();
        for frame in [first, scheduled, peer, refreshed] {
            builder.observe(&frame);
        }
        assert_eq!(builder.reads, [15, 16, 17, 18]);
        assert_eq!(builder.operations.len(), 1);
        assert_eq!(
            builder.operations[0]
                .runs
                .iter()
                .map(|run| (
                    run.first_hour,
                    run.last_hour,
                    run.operative_names
                        .as_ref()
                        .and_then(|names| names.first())
                        .and_then(|name| name.name.as_deref()),
                ))
                .collect::<Vec<_>>(),
            [
                (15, 15, Some("FIRST_RECORDED_NAME")),
                (16, 17, None),
                (18, 18, Some("FRESH_RECORDED_NAME")),
            ]
        );
    }

    #[test]
    fn espionage_names_use_recorded_assignment_order_and_do_not_backfill_an_unread_hour() {
        use replay_data::{rows::OperationAssignmentRow, rows::OperativeMembershipRow};
        let mut first = frame(100, 1);
        let mut operation = first.complete::<tables::Operations>().unwrap()[0].clone();
        operation.assigned_slots = 1;
        *first.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![operation]));
        *first.tables[TableId::OperationAssignments.index()] =
            TableView::Complete(TableRows::OperationAssignments(vec![
                OperationAssignmentRow {
                    operation: 1,
                    ordinal: 0,
                    operative: 77,
                    selection: 0,
                },
            ]));
        *first.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![
                OperativeMembershipRow {
                    id: 77,
                    name: b"RECORDED_OPERATIVE".to_vec().into(),
                    ..Default::default()
                },
            ]));
        first.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        let mut builder = Builder::default();
        builder.observe(&first);
        assert_eq!(
            builder.operations[0].runs[0]
                .operative_names
                .as_ref()
                .unwrap()[0]
                .name
                .as_deref(),
            Some("RECORDED_OPERATIVE")
        );
        first.hour = 101;
        *first.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable {
            reason: "synthetic missing read".into(),
        };
        builder.observe(&first);
        assert_eq!(builder.operations[0].runs.len(), 2);
        assert!(builder.operations[0].runs[1].operative_names.is_none());
    }

    #[test]
    fn espionage_window_keeps_the_actual_ending_read_before_its_clipped_run() {
        use replay_data::{rows::OperationAssignmentRow, rows::OperativeMembershipRow};
        let mut builder = Builder::default();
        let mut initial = frame(100, 1);
        *initial.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&initial);
        let mut sample = frame(112, 1);
        let mut operation = sample.complete::<tables::Operations>().unwrap()[0].clone();
        operation.assigned_slots = 1;
        operation.phase_count = 3;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![operation]));
        *sample.tables[TableId::OperationAssignments.index()] =
            TableView::Complete(TableRows::OperationAssignments(vec![
                OperationAssignmentRow {
                    operation: 1,
                    ordinal: 0,
                    operative: 77,
                    selection: 0,
                },
            ]));
        *sample.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(vec![
                OperativeMembershipRow {
                    id: 77,
                    name: b"RECORDED_OPERATIVE".to_vec().into(),
                    ..Default::default()
                },
            ]));
        sample.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        builder.observe(&sample);
        sample.metrics.table_ns[TableId::Operations.index()] = 0;
        sample.metrics.table_ns[TableId::OperationAssignments.index()] = 0;
        for hour in 113..=135 {
            sample.hour = hour;
            builder.observe(&sample);
        }
        let mut ended = frame(136, 1);
        *ended.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&ended);
        let raw = EspionageLogReply {
            branch_id: "1".into(),
            window: None,
            windows: Vec::new(),
            first_hour: 100,
            last_hour: 147,
            available: builder.available,
            read_hours: builder.reads,
            completions_available: Vec::new(),
            completions_unrecorded: Some("Completion events not recorded".into()),
            types: Vec::new(),
            operations: builder.operations,
            warnings: Vec::new(),
            unavailable: None,
        };
        let detail = metadata_window(&raw, Some(1)).unwrap();
        let operation = &detail.operations[0];
        assert_eq!(
            (
                operation.end.as_ref().unwrap().last_seen,
                operation.end.as_ref().unwrap().by_hour
            ),
            (112, 136)
        );
        assert_eq!(
            operation
                .runs
                .iter()
                .map(|run| (run.first_hour, run.last_hour, run.read_hour))
                .collect::<Vec<_>>(),
            [(112, 112, 112), (124, 135, 112)]
        );
        let witness = operation
            .runs
            .iter()
            .find(|run| run.first_hour <= 112 && 112 <= run.last_hour)
            .unwrap();
        assert_eq!(witness.preparation_ratio, Some(25_000));
        assert_eq!(witness.phase_count, 3);
        assert_eq!(
            witness.operative_names.as_ref().unwrap()[0].name.as_deref(),
            Some("RECORDED_OPERATIVE")
        );
    }

    #[test]
    fn espionage_preparation_uses_native_fixed5_wrap_clamp_and_zero_rules() {
        let mut row = OperationRow {
            preparation_current: 51_999,
            preparation_required: 100_000,
            ..Default::default()
        };
        assert_eq!(preparation_ratio(&row), Some(51_999));
        row.preparation_required = 0;
        assert_eq!(preparation_ratio(&row), Some(0));
        row.preparation_required = 100_000;
        row.preparation_current = i64::MAX;
        assert_eq!(preparation_ratio(&row), Some(100_000));
        row.preparation_required = 0;
        assert_eq!(preparation_ratio(&row), Some(0));
        row.preparation_required = 100_000;
        row.preparation_current = -1;
        assert_eq!(preparation_ratio(&row), Some(0));
        row.preparation_current = i64::MAX / 100_000 + 1;
        assert_eq!(preparation_ratio(&row), Some(0));
    }

    #[test]
    fn espionage_current_phase_uses_native_separate_day_truncation_and_guards() {
        let mut row = OperationRow {
            start_hour: 43_800_023,
            duration_days: 10,
            phase_count: 3,
            ..Default::default()
        };
        assert_eq!(current_phase(&row, 43_800_024), Some(0));
        assert_eq!(current_phase(&row, 43_800_120), Some(1));
        assert_eq!(current_phase(&row, 43_800_240), None);
        row.terminal = 1;
        assert_eq!(current_phase(&row, 43_800_120), None);
        row.terminal = 0;
        row.duration_days = 0;
        assert_eq!(current_phase(&row, 43_800_120), None);
    }

    #[test]
    fn espionage_five_thousand_countries_and_equal_reads_fit() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 5000));
        builder.observe(&frame(101, 5000));
        assert_eq!(builder.operations.len(), 5000);
        assert!(
            builder
                .operations
                .iter()
                .all(|op| op.runs.len() == 1 && op.runs[0].last_hour == 101)
        );
        assert_eq!(builder.reads, [100, 101]);
    }

    #[test]
    fn espionage_product_reads_5000_countries_and_dates_disappearance_after_a_gap() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
        use replay_data::rows::LexicalTokenRow;
        let mut first = frame(100, 5000);
        first.tables[TableId::Campaign.index()] =
            source(1, 100, Some(1)).tables[TableId::Campaign.index()].clone();
        *first.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 70,
                literal: b"mod_operation".to_vec(),
            }]));
        let mut missing = first.clone();
        missing.hour = 101;
        *missing.tables[TableId::Operations.index()] = TableView::Unavailable {
            reason: "synthetic missing operation read".into(),
        };
        let mut later = first.clone();
        later.hour = 104;
        *later.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![
                first.complete::<tables::Operations>().unwrap()[0].clone(),
            ]));
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, missing, later])])
                .unwrap(),
        );
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "espionage-synthetic".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        let reply = EspionageLog::load(
            &source,
            &EspionageLogParams {
                branch_id: "1".into(),
                window: Some(0),
            },
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(reply.operations.len(), 5000);
        assert_eq!(reply.types[0].key, "mod_operation");
        assert_eq!(reply.read_hours, [100, 104]);
        assert_eq!(reply.operations[0].runs.len(), 2);
        assert!(reply.operations[0].end.is_none());
        let end = reply.operations[1].end.as_ref().unwrap();
        assert_eq!((end.last_seen, end.by_hour), (100, 104));
        assert!(json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
    }

    #[test]
    fn espionage_missing_rows_and_gaps_never_supply_zero_or_finish_time() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1));
        let mut missing = frame(101, 1);
        *missing.tables[TableId::Operations.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        builder.observe(&missing);
        assert!(builder.operations[0].end.is_none());
        builder.observe(&frame(102, 1));
        assert_eq!(builder.operations[0].runs.len(), 2);
        let mut gone = frame(103, 1);
        *gone.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&gone);
        assert_eq!(builder.operations[0].end.as_ref().unwrap().by_hour, 103);
        assert_eq!(builder.operations[0].end.as_ref().unwrap().last_seen, 102);
        assert_eq!(builder.reads, [100, 102, 103]);
    }

    #[test]
    fn espionage_daily_carry_keeps_its_read_date_and_never_crosses_a_gap() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1));
        let mut carried = frame(101, 1);
        carried.metrics.table_ns[TableId::Operations.index()] = 0;
        builder.observe(&carried);
        assert_eq!(builder.reads, [100]);
        assert_eq!(builder.operations[0].runs[0].read_hour, 100);
        assert_eq!(builder.operations[0].runs[0].last_hour, 101);
        let mut failed = carried.clone();
        failed.hour = 102;
        *failed.tables[TableId::Operations.index()] = TableView::Unavailable {
            reason: "failed read".into(),
        };
        builder.observe(&failed);
        carried.hour = 103;
        builder.observe(&carried);
        assert_eq!(builder.available[0].last_hour, 101);
        assert_eq!(builder.operations[0].runs[0].last_hour, 101);
        builder.observe(&frame(104, 1));
        assert_eq!(builder.operations[0].runs.len(), 2);
        carried.hour = 105;
        builder.observe(&carried);
        let mut gone = frame(106, 1);
        *gone.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&gone);
        let end = builder.operations[0].end.as_ref().unwrap();
        assert_eq!((end.last_seen, end.by_hour), (104, 106));
        assert_eq!(builder.reads, [100, 104, 106]);
    }

    #[test]
    fn espionage_execution_start_is_not_visible_before_its_read_and_repeat_is_separate() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1));
        let mut sample = frame(101, 1);
        let mut row = sample.complete::<tables::Operations>().unwrap()[0].clone();
        row.duration_days = 5;
        row.start_hour = 101;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row.clone()]));
        builder.observe(&sample);
        assert_eq!(builder.operations[0].runs[0].start_hour, None);
        assert_eq!(builder.operations.len(), 1);
        assert!(builder.operations[0].end.is_none());
        assert_eq!(builder.operations[0].start_hour, Some(101));
        assert_eq!(builder.operations[0].start_known_at, Some(101));
        assert_eq!(builder.operations[0].runs[0].duration_days, 0);
        assert_eq!(builder.operations[0].runs[1].duration_days, 5);
        sample.hour = 102;
        row.terminal = 1;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row.clone()]));
        builder.observe(&sample);
        assert_eq!(builder.operations[0].end.as_ref().unwrap().by_hour, 102);
        sample.hour = 103;
        // Repeat constructs a new identity rather than resetting the old instance.
        row.id = 2;
        row.terminal = 0;
        row.duration_days = 0;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row]));
        builder.observe(&sample);
        assert_eq!(builder.operations.len(), 2);
    }

    #[test]
    fn espionage_country_tag_is_the_read_identity_and_missing_assignments_are_unknown() {
        let mut builder = Builder::default();
        let mut sample = frame(100, 1);
        *sample.tables[TableId::OperationAssignments.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        builder.observe(&sample);
        sample.hour = 101;
        let mut country = sample.complete::<tables::Countries>().unwrap()[0].clone();
        country.tag = b"renamed-mod".to_vec();
        *sample.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![country]));
        builder.observe(&sample);
        assert_eq!(builder.operations.len(), 1);
        assert!(builder.operations[0].end.is_none());
        assert_eq!(
            builder.operations[0].runs[0].owner_tag.as_deref(),
            Some("mod1")
        );
        assert_eq!(
            builder.operations[0].runs[1].owner_tag.as_deref(),
            Some("renamed-mod")
        );
        assert_eq!(builder.operations[0].runs[1].operatives, None);
    }

    #[test]
    fn espionage_link_tags_follow_balance_rename_alias_and_unavailable_reads() {
        let mut first = frame(100, 2);
        let mut operation = first.complete::<tables::Operations>().unwrap()[0].clone();
        operation.owner = 77;
        *first.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![operation]));
        let mut links = first.complete::<tables::CountryLinks>().unwrap().to_vec();
        links.push(CountryLinkRow {
            id: 77,
            canonical_slot: 1,
        });
        *first.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links.clone()));
        let mut balance = first.clone();
        balance.hour = 101;
        let mut countries = balance.complete::<tables::Countries>().unwrap().to_vec();
        countries[0].political_power_raw = Some(999);
        *balance.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries.clone()));
        let mut renamed = balance.clone();
        renamed.hour = 102;
        countries[0].tag = b"renamed-mod".to_vec();
        *renamed.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        let mut repointed = renamed.clone();
        repointed.hour = 103;
        links[2].canonical_slot = 2;
        *repointed.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links));
        let mut missing_countries = repointed.clone();
        missing_countries.hour = 104;
        *missing_countries.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "synthetic missing countries".into(),
        };
        let mut missing_links = repointed.clone();
        missing_links.hour = 105;
        *missing_links.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "synthetic missing links".into(),
        };
        let mut recovered = repointed.clone();
        recovered.hour = 106;
        let replay = dashboard_base::test_fixtures::replay(vec![
            first,
            balance,
            renamed,
            repointed,
            missing_countries,
            missing_links,
            recovered,
        ]);
        let reply = scan(
            &replay,
            &BranchParams {
                branch_id: "1".into(),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.operations.len(), 1);
        let operation = &reply.operations[0];
        assert_eq!(operation.first_read, 100);
        assert!(operation.end.is_none());
        assert_eq!(
            operation
                .runs
                .iter()
                .map(|run| (
                    run.first_hour,
                    run.owner_tag.as_deref(),
                    run.target_tag.as_deref()
                ))
                .collect::<Vec<_>>(),
            [
                (100, Some("mod1"), Some("mod2")),
                (102, Some("renamed-mod"), Some("mod2")),
                (103, Some("mod2"), Some("mod2")),
                (104, None, None),
                (106, Some("mod2"), Some("mod2")),
            ]
        );
        assert_eq!(operation.runs[0].last_hour, 101);
        assert_eq!(operation.runs[3].last_hour, 105);
        assert_eq!(reply.read_hours, [100, 101, 102, 103, 104, 105, 106]);
        assert!(reply.warnings.iter().any(|warning| warning
            == "Country identities unavailable at operation read (2 observations)."));
    }

    #[test]
    fn espionage_execution_signed_native_days_and_sentinels() {
        let mut row = OperationRow {
            start_hour: 43_800_023,
            duration_days: 10,
            ..Default::default()
        };
        assert_eq!(elapsed_days(&row, 43_800_024), Some(1)); // Separate day truncation.
        row.start_hour = 43_800_000;
        assert_eq!(elapsed_days(&row, 43_799_977), Some(0)); // Signed /24 toward zero.
        assert_eq!(elapsed_days(&row, 43_799_976), None); // Native -1.
        assert_eq!(elapsed_days(&row, 43_799_952), Some(-2));
        assert_eq!(elapsed_days(&row, 43_800_240), None); // elapsed == duration.
        assert_eq!(elapsed_days(&row, 43_800_264), Some(11)); // No invented clamp.
        row.terminal = 1;
        assert_eq!(elapsed_days(&row, 43_800_024), None);
        row.terminal = 0;
        row.duration_days = 0;
        assert_eq!(elapsed_days(&row, 43_800_024), None);
        row.duration_days = 10;
        row.start_hour = i32::MIN;
        let wrapped = row.start_hour.wrapping_add(24);
        assert_eq!(elapsed_days(&row, wrapped), Some(1));
    }

    #[test]
    fn espionage_daily_windows_keep_five_thousand_country_campaign_metadata_bounded() {
        let mut builder = Builder::default();
        for day in 0..50 {
            let mut sample = frame(100 + day * 24, 5000);
            let rows = sample
                .complete::<tables::Operations>()
                .unwrap()
                .iter()
                .cloned()
                .map(|mut row| {
                    row.preparation_current = i64::from(day + 1);
                    row.preparation_required = 50;
                    row
                })
                .collect();
            *sample.tables[TableId::Operations.index()] =
                TableView::Complete(TableRows::Operations(rows));
            builder.observe(&sample);
        }
        assert_eq!(builder.operations.len(), 5000);
        assert!(
            builder
                .operations
                .iter()
                .all(|operation| operation.runs.len() == 50)
        );
        let reply = EspionageLogReply {
            branch_id: "1".into(),
            window: None,
            windows: Vec::new(),
            first_hour: 100,
            last_hour: 1276,
            available: builder.available,
            read_hours: builder.reads,
            completions_available: Vec::new(),
            completions_unrecorded: Some(
                "Operation completions not recorded: this product does not read OperationCompletions".into(),
            ),
            types: vec![],
            operations: builder.operations,
            warnings: vec![],
            unavailable: None,
        };
        assert!(json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none());
        let manifest = metadata_window(&reply, None).unwrap();
        assert_eq!(manifest.windows.len(), 50);
        assert!(manifest.operations.is_empty());
        assert!(manifest.read_hours.is_empty());
        assert!(json_len(&manifest, 64 * 1024).is_some());
        for index in [0, 25, 49] {
            let detail = metadata_window(&reply, Some(index)).unwrap();
            assert_eq!(detail.operations.len(), 5000);
            assert!(
                detail
                    .operations
                    .iter()
                    .all(|operation| operation.runs.len() <= 2)
            );
            assert!(json_len(&detail, MAX_PRODUCT_REPLY_BYTES).is_some());
            assert_eq!(detail.read_hours.last(), Some(&detail.first_hour));
            assert!(
                detail
                    .operations
                    .iter()
                    .all(|operation| operation.first_read == 100)
            );
        }
        assert!(metadata_window(&reply, Some(50)).is_err());
    }

    #[test]
    fn espionage_mutable_values_are_dated_changes_without_endings() {
        for change in 0..5 {
            let mut builder = Builder::default();
            let mut first = frame(100, 1);
            if change == 4 {
                let mut row = first.complete::<tables::Operations>().unwrap()[0].clone();
                row.duration_days = 5;
                row.start_hour = 100;
                *first.tables[TableId::Operations.index()] =
                    TableView::Complete(TableRows::Operations(vec![row]));
            }
            builder.observe(&first);
            let mut changed = frame(101, 1);
            let mut row = changed.complete::<tables::Operations>().unwrap()[0].clone();
            match change {
                0 => row.auto_repeat = 1,
                1 => row.phase_count = 4,
                2 => row.assigned_slots = 1,
                3 => {
                    row.duration_days = 5;
                    row.start_hour = 101;
                }
                _ => {
                    row.duration_days = 5;
                    row.start_hour = 102;
                }
            }
            *changed.tables[TableId::Operations.index()] =
                TableView::Complete(TableRows::Operations(vec![row]));
            builder.observe(&changed);
            assert_eq!(builder.operations.len(), 1);
            let operation = &builder.operations[0];
            assert!(operation.end.is_none());
            assert_eq!(operation.runs.len(), 2);
            assert_eq!(operation.runs[1].read_hour, 101);
        }
    }

    #[test]
    fn espionage_failed_country_reads_are_tag_gaps_and_never_endings() {
        for table in [TableId::Countries, TableId::CountryLinks] {
            let mut builder = Builder::default();
            builder.observe(&frame(100, 1));
            let mut missing = frame(101, 1);
            *missing.tables[table.index()] = TableView::Unavailable {
                reason: "missing identities".into(),
            };
            builder.observe(&missing);
            builder.observe(&frame(102, 1));
            assert_eq!(builder.operations.len(), 1);
            let operation = &builder.operations[0];
            assert!(operation.end.is_none());
            assert_eq!(operation.runs.len(), 3);
            assert_eq!(operation.runs[1].owner_tag, None);
            assert_eq!(operation.runs[1].target_tag, None);
            assert_eq!(operation.runs[2].owner_tag.as_deref(), Some("mod1"));
            let mut gone = missing.clone();
            gone.hour = 103;
            *gone.tables[TableId::Operations.index()] =
                TableView::Complete(TableRows::Operations(Vec::new()));
            builder.observe(&gone);
            let end = builder.operations[0].end.as_ref().unwrap();
            assert_eq!(end.owner_tag, None);
            assert_eq!(end.target_tag, None);
        }
    }

    #[test]
    fn espionage_only_raw_identity_changes_or_finished_reopening_split_episodes() {
        for change in 0..4 {
            let mut builder = Builder::default();
            let mut first = frame(100, 1);
            if change == 3 {
                let mut row = first.complete::<tables::Operations>().unwrap()[0].clone();
                row.terminal = 1;
                *first.tables[TableId::Operations.index()] =
                    TableView::Complete(TableRows::Operations(vec![row]));
            }
            builder.observe(&first);
            let mut next = frame(101, 1);
            let mut row = next.complete::<tables::Operations>().unwrap()[0].clone();
            match change {
                0 => row.owner = 2,
                1 => row.target = 2,
                2 => row.definition_token = 71,
                _ => {}
            }
            *next.tables[TableId::Operations.index()] =
                TableView::Complete(TableRows::Operations(vec![row]));
            builder.observe(&next);
            assert_eq!(builder.operations.len(), 2);
            assert!(builder.operations[0].end.is_some());
            assert!(builder.operations[1].end.is_none());
        }
    }

    #[test]
    fn espionage_definition_name_gaps_are_recording_gaps_without_content_keys() {
        use replay_data::rows::LexicalTokenRow;
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1));
        builder.observe(&frame(101, 1));
        let mut named = frame(102, 1);
        *named.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 70,
                literal: b"mod_operation".to_vec(),
            }]));
        builder.observe(&named);
        assert_eq!(builder.operations.len(), 1);
        assert_eq!(builder.operations[0].definition_token, 70);
        assert_eq!(builder.operations[0].runs[0].type_key, None);
        assert_eq!(
            builder.operations[0].runs[1].type_key.as_deref(),
            Some("mod_operation")
        );
        assert_eq!(
            builder.types.into_iter().collect::<Vec<_>>(),
            ["mod_operation"]
        );
        assert_eq!(
            builder.warnings["Operation definition token 70 not recorded"],
            2
        );
    }

    #[test]
    fn espionage_unavailable_lexical_table_is_counted_once_per_operation_read() {
        let mut builder = Builder::default();
        let mut sample = frame(100, 2);
        *sample.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "synthetic lexical gap".into(),
        };
        if let TableView::Complete(TableRows::Operations(rows)) =
            &mut *sample.tables[TableId::Operations.index()]
        {
            rows[1].definition_token = 71;
        }
        builder.observe(&sample);
        sample.hour = 101;
        builder.observe(&sample);
        assert_eq!(builder.operations.len(), 2);
        assert!(builder.types.is_empty());
        assert!(builder.operations.iter().all(|operation| {
            operation.runs.len() == 1 && operation.runs[0].type_key.is_none()
        }));
        assert_eq!(
            builder.warnings,
            BTreeMap::from([("LexicalTokens unavailable at operation read".to_owned(), 2)])
        );
    }

    #[test]
    fn espionage_recorded_definition_name_refusals_are_not_recording_or_content_misses() {
        use replay_data::rows::LexicalTokenRow;
        for (literal, reason) in [
            (vec![0xff], "unsupported non-text recorded name bytes"),
            (Vec::new(), "empty recorded name"),
        ] {
            let mut builder = Builder::default();
            let mut sample = frame(100, 1);
            *sample.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 70,
                    literal,
                }]));
            builder.observe(&sample);
            assert!(builder.types.is_empty());
            assert_eq!(builder.operations[0].runs[0].type_key, None);
            assert_eq!(builder.warnings.len(), 1);
            assert!(builder.warnings.keys().next().unwrap().contains(reason));
        }
    }

    #[test]
    fn espionage_missing_definition_names_do_not_request_or_blame_content() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records};
        let cancelled = AtomicBool::new(false);
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100, 1)])]).unwrap(),
        );
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "unknown-definition".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            replay,
            &cancelled,
        )
        .unwrap();
        let reply = EspionageLog::load(
            &source,
            &EspionageLogParams {
                branch_id: "1".into(),
                window: Some(0),
            },
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert!(reply.types.is_empty());
        assert_eq!(reply.operations.len(), 1);
        assert_eq!(reply.operations[0].runs[0].type_key, None);
        assert_eq!(
            reply.warnings,
            ["Operation definition token 70 not recorded (1 observations)."]
        );
    }

    fn description_reference() -> super::super::operation_description::OperationDescriptionRef {
        super::super::operation_description::OperationDescriptionRef {
            hour: 100,
            id: "1".into(),
            owner: 1,
            sequence: Some("91".into()),
        }
    }

    #[test]
    fn espionage_outcome_metadata_keeps_native_length_and_recorded_language() {
        let mut warnings = BTreeMap::new();
        let (outcome, missing) = outcome_at_end(
            Some(1),
            Some(0),
            Some(description_reference()),
            None,
            Some(u64::MAX),
            Some("synthetic_language"),
            &mut warnings,
        );
        let outcome = outcome.unwrap();
        assert!(missing.is_none());
        assert!(outcome.bonus);
        assert!(!outcome.detected);
        assert_eq!(outcome.recorded_language, "synthetic_language");
        assert_eq!(outcome.description_required_bytes, u64::MAX.to_string());
        assert_eq!(outcome.description_ref, Some(description_reference()));
        assert!(outcome.description_unrecorded.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn espionage_outcome_capture_refusal_preserves_independent_results() {
        let mut warnings = BTreeMap::new();
        let (outcome, missing) = outcome_at_end(
            Some(0),
            Some(1),
            Some(description_reference()),
            Some("operation completion description byte ring short"),
            Some(u64::MAX),
            Some("synthetic_language"),
            &mut warnings,
        );
        let outcome = outcome.unwrap();
        assert!(missing.is_none());
        assert!(!outcome.bonus);
        assert!(outcome.detected);
        assert_eq!(outcome.description_ref, None);
        assert_eq!(outcome.description_required_bytes, u64::MAX.to_string());
        assert_eq!(outcome.recorded_language, "synthetic_language");
        assert_eq!(
            outcome.description_unrecorded.as_deref(),
            Some("operation completion description byte ring short")
        );
        assert_eq!(
            warnings["operation completion description byte ring short"],
            1
        );
        let (outcome, missing) = outcome_at_end(None, None, None, None, None, None, &mut warnings);
        assert!(outcome.is_none());
        assert!(missing.unwrap().starts_with("Outcome not recorded:"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn espionage_outcome_invalid_flag_bytes_are_named_and_counted() {
        let mut warnings = BTreeMap::new();
        for (bonus, detected) in [(2, 0), (0, 2), (255, 255), (2, 0)] {
            let (outcome, missing) = outcome_at_end(
                Some(bonus),
                Some(detected),
                Some(description_reference()),
                None,
                Some(15),
                Some("synthetic_language"),
                &mut warnings,
            );
            assert!(outcome.is_none());
            let missing = missing.unwrap();
            for (field, value) in [("bonus", bonus), ("detected", detected)] {
                assert_eq!(
                    missing.contains(&format!("{field} refused: flag byte {value}")),
                    value > 1
                );
            }
        }
        assert_eq!(
            warnings,
            BTreeMap::<String, usize>::from([
                (
                    "Stored operation bonus refused: flag byte 2 is not 0 or 1".into(),
                    2
                ),
                (
                    "Stored operation bonus refused: flag byte 255 is not 0 or 1".into(),
                    1
                ),
                (
                    "Stored operation detected refused: flag byte 2 is not 0 or 1".into(),
                    1
                ),
                (
                    "Stored operation detected refused: flag byte 255 is not 0 or 1".into(),
                    1
                ),
            ])
        );
    }

    #[test]
    fn espionage_reply_budget_refuses_exact_serialized_size_without_truncating() {
        let mut reply = EspionageLogReply {
            branch_id: "1".into(),
            window: None,
            windows: Vec::new(),
            first_hour: 100,
            last_hour: 100,
            available: Vec::new(),
            read_hours: Vec::new(),
            completions_available: Vec::new(),
            completions_unrecorded: Some(
                "Operation completions not recorded: this product does not read OperationCompletions".into(),
            ),
            types: Vec::new(),
            operations: Vec::new(),
            warnings: Vec::new(),
            unavailable: None,
        };
        reply.warnings.push("x".repeat(MAX_PRODUCT_REPLY_BYTES));
        assert!(check_reply_budget(&reply).is_err());
        reply.warnings[0].truncate(MAX_PRODUCT_REPLY_BYTES - 1024);
        assert!(check_reply_budget(&reply).is_ok());
    }
}
