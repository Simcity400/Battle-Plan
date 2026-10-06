//! Every country's recorded operations. Reads supply values, never elapsed-time
//! estimates. A missing hour breaks runs; a complete later census can establish
//! only that an absent operation ended *by* that read.
//!
//! COperationInstance completion 1413EE390 writes terminal +0x42 and the
//! generated outcome description +0xA0. OperationRow captures the former, not
//! the latter or bonus-result +0xC0. The +0xC1 detection result is also
//! absent. Terminal state never establishes success or operative capture.
//! OperationTokens is an independent owner-target token set, added/removed by
//! authored effects (1413F10D0/1413F22B0, 14034AD00/14035E450), without an
//! operation handle. It is therefore no outcome source and is not decoded here.

use super::{
    BranchParams, LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_groups::HourSpan,
    intelligence_content::intelligence_content,
};
use crate::{
    Source,
    metadata::{LinkTags, branch_bounds, slot_read, validate_campaign},
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{OperationRow, TableId, tables},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct EspionageLog;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationType {
    pub key: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
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
    /// Selected definitions, not completed phases (1413EB710).
    pub phase_count: u32,
    /// Manager 14118B110 tests +0x41 after completion.
    pub auto_repeat: bool,
    /// Launch writes duration and start on the same instance (1413EAD80).
    pub duration_days: i32,
    /// All +0xE0 slots, including unassigned ones (1413E9210/141BD1CD0).
    pub assigned_slots: u32,
    /// 1413EC160 signed day result at this read; native -1 stays unknown.
    pub elapsed_days: Option<i32>,
    /// Zero-based native phase at the supplying read; names/IDs are not captured.
    pub current_phase: Option<u32>,
    pub terminal: bool,
    /// Auto-commence, tested by 141189630 before 1413EF490 starts it.
    pub auto_commence: bool,
    pub start_hour: Option<i32>,
    /// Non-null handles in native slot order; null means assignments unread.
    pub operatives: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EspionageEndReason {
    Completed,
    Disappeared,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOutcome {
    /// Stored independent trials, +C0/+C1 (1413EE390), never success/failure.
    pub bonus: bool,
    pub detected: bool,
    /// Language of the stored +0xA0 text, not the viewer's display language.
    pub recorded_language: String,
    pub description: Option<String>,
    pub description_unrecorded: Option<String>,
    /// Exact native byte length, including when capture refused the text.
    pub description_required_bytes: String,
}

#[derive(Clone, Debug, Serialize, TS)]
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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperation {
    /// Full native handle and episode's first read. Auto-repeat creates a fresh
    /// identity (14118B110 -> 141A13EA0 -> 141189390 -> 142209A50).
    pub id: String,
    pub first_read: i32,
    pub start_hour: Option<i32>,
    pub start_known_at: Option<i32>,
    /// Raw lexical identity; the name, countries and mutable values belong to reads.
    pub definition_token: u32,
    pub runs: Vec<EspionageOperationRun>,
    pub end: Option<EspionageOperationEnd>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageLogReply {
    pub branch_id: String,
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

#[derive(Default)]
struct Builder {
    operations: Vec<EspionageOperation>,
    open: HashMap<u64, (usize, (u32, u32, u32))>,
    available: Vec<HourSpan>,
    reads: Vec<i32>,
    types: BTreeSet<String>,
    warnings: BTreeMap<String, usize>,
    links: LinkTags,
}

fn budget_error() -> io::Error {
    io::Error::other("Espionage history exceeds the complete-response byte budget")
}

impl Builder {
    fn warn(&mut self, reason: impl Into<String>) {
        *self.warnings.entry(reason.into()).or_default() += 1;
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

    fn observe(&mut self, frame: &DecodedFrame) {
        let Ok(rows) = frame.complete::<tables::Operations>() else {
            self.warn("Operations unavailable or incomplete; values and endings unknown");
            return;
        };
        let hour = frame.hour;
        if !slot_read(&frame.metrics, TableId::Operations, hour) {
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
        let labels = self.links.update(frame).is_ok();
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
        let absent: Vec<_> = self
            .open
            .keys()
            .filter(|id| !ids.contains(id))
            .copied()
            .collect();
        for id in absent {
            let (index, (owner, target, _)) = self.open.remove(&id).expect("open handle");
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
                    owner_tag: labels.then(|| self.links.get(owner).cloned()).flatten(),
                    target_tag: labels.then(|| self.links.get(target).cloned()).flatten(),
                });
            }
        }
        for row in rows {
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
                        .collect()
                })
            });
            let owner_tag = labels.then(|| self.links.get(row.owner).cloned()).flatten();
            let target_tag = labels
                .then(|| self.links.get(row.target).cloned())
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
                    !previous.terminal || run.terminal
                })
                .map(|(index, _)| *index);
            let index = match continuing {
                Some(index) => index,
                None => {
                    if let Some((old, previous_fingerprint)) = self.open.remove(&row.id) {
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
                                    .then(|| self.links.get(previous_fingerprint.0).cloned())
                                    .flatten(),
                                target_tag: labels
                                    .then(|| self.links.get(previous_fingerprint.1).cloned())
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
                    self.open.insert(row.id, (index, fingerprint));
                    index
                }
            };
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
                let (outcome, outcome_unrecorded) =
                    outcome_at_end(None, None, None, None, None, None, &mut self.warnings);
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
    }
}

fn supplying_read(reads: &[i32], hour: i32) -> i32 {
    reads[reads.partition_point(|read| *read <= hour) - 1]
}

/// +0x38 sums civilian days * fixed5 (constructor 1413E9210), +0x30
/// stores elapsed civilian work. This is their recorded aggregate ratio;
/// equipment requirements and readiness are not captured. The resource meter
/// 141E1CFA0/141E1DAA0 uses signed wrapping IMUL (141E1D13E), IDIV
/// (141E1D147), a zero-denominator zero, and clamp [0,100000]. Its three
/// fixed5 sentinels go through 0xFFFFFFFF and clamp to a full meter.
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
    // Float is geometry only. FormatFixed5Percent 142259590 displays the
    // integer percentage, ratio / 1000, with precision zero and truncation.
    Some(ratio as i32)
}

/// Elapsed days 1413EC160 uses separately truncated signed dates, offset by
/// the executable's epoch (ASM 1413EC27F..1413EC2C2). The phase view
/// 1413EB760 wraps the signed product and divides by duration. Pointer getter
/// 1413EB710 rejects -1 and indices >= count; an unsafe negative index has no
/// representable phase here. This runs only at a read, never between reads.
fn elapsed_days(row: &OperationRow, hour: i32) -> Option<i32> {
    if row.duration_days == 0 || row.terminal != 0 {
        return None;
    }
    // Native epoch constant, not content: signed i32 SUB and /24 truncate
    // toward zero separately (1413EC27F..1413EC2C2).
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

/// Completion 1413EE390 writes +0xA0 text and independent +0xC0/+0xC1
/// results; 141819FF0 displays those client-language bytes without relocalising.
fn outcome_at_end(
    bonus: Option<u8>,
    detected: Option<u8>,
    description: Option<&[u8]>,
    description_reason: Option<&str>,
    description_required_bytes: Option<u64>,
    recorded_language: Option<&str>,
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
    let (Some(bonus), Some(detected), Some(description), Some(required_bytes), Some(language)) = (
        bonus,
        detected,
        description,
        description_required_bytes,
        recorded_language,
    ) else {
        return (None, Some("Outcome not recorded: stored bonus, detection and effect description were not captured".into()));
    };
    let text = match description_reason {
        Some(reason) => Err(reason.to_owned()),
        None => std::str::from_utf8(description)
            .map(str::to_owned)
            .map_err(|_| "Stored operation description refused: non-UTF-8 recorded bytes".into()),
    };
    let (description, description_unrecorded) = match text {
        Ok(description) => (Some(description), None),
        Err(reason) => {
            *warnings.entry(reason.clone()).or_default() += 1;
            (None, Some(reason))
        }
    };
    (
        Some(EspionageOutcome {
            bonus: bonus == 1,
            detected: detected == 1,
            recorded_language: language.to_owned(),
            description,
            description_unrecorded,
            description_required_bytes: required_bytes.to_string(),
        }),
        None,
    )
}

fn scan(
    replay: &Replay,
    params: &BranchParams,
    cancelled: &AtomicBool,
) -> io::Result<EspionageLogReply> {
    let branch = replay
        .branches
        .iter()
        .find(|branch| branch.id == params.branch())
        .ok_or_else(|| crate::invalid("Branch not in this recording"))?;
    let (first_hour, last_hour) = branch_bounds(replay, branch);
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Operations,
        TableId::OperationAssignments,
        TableId::LexicalTokens,
    ];
    let mut builder = Builder::default();
    for block in replay
        .index
        .iter()
        .filter(|block| block.branch == params.branch())
    {
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            builder.warn("Unread replay block; operation history has a gap");
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Espionage request cancelled",
                ));
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                builder.warn("Unread replay frame; operation history has a gap");
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            builder.observe(decoded);
        }
    }
    let unavailable = builder
        .available
        .is_empty()
        .then(|| "Operations was unavailable, incomplete or never read in this recording.".into());
    let reply = EspionageLogReply {
        branch_id: params.branch_id.clone(),
        first_hour,
        last_hour,
        available: builder.available,
        read_hours: builder.reads,
        completions_available: Vec::new(),
        completions_unrecorded: Some(
            "Operation completions not recorded: this product does not read OperationCompletions"
                .into(),
        ),
        types: builder
            .types
            .into_iter()
            .map(|key| EspionageOperationType {
                key,
                name: None,
                icon: None,
                icon_missing: Some("Operation content unavailable".into()),
            })
            .collect(),
        operations: builder.operations,
        warnings: builder
            .warnings
            .into_iter()
            .map(|(reason, count)| format!("{reason} ({count} observations)."))
            .collect(),
        unavailable,
    };
    check_reply_budget(&reply)?;
    Ok(reply)
}

impl Product for EspionageLog {
    const KEY: &'static str = "espionage-log";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = EspionageLogReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            HourSpan::decl(),
            EspionageOperationType::decl(),
            EspionageOperationRun::decl(),
            EspionageEndReason::decl(),
            EspionageOutcome::decl(),
            EspionageOperationEnd::decl(),
            EspionageOperation::decl(),
            EspionageLogReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if cache.branch != Some(params.branch()) {
            cache.raw = Some(Arc::new(scan(&source.replay(), params, cancelled)?));
            cache.branch = Some(params.branch());
        }
        let mut reply = cache.raw.as_ref().expect("scanned branch").as_ref().clone();
        drop(cache);
        if !reply.types.is_empty() {
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
                    reply.warnings.extend(content.warnings.iter().cloned());
                    reply.warnings.extend(content.catalogue.refusal_lines());
                }
                Err(reason) => reply
                    .warnings
                    .push(format!("Operation content unavailable: {reason}")),
            }
        }
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn check_reply_budget(reply: &EspionageLogReply) -> io::Result<()> {
    json_len(reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(budget_error)?;
    Ok(())
}

/// Retains the current branch scan; content changes refresh presentation independently.
#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    raw: Option<Arc<EspionageLogReply>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::TableView,
        rows::{CountryLinkRow, CountryRow, TableRows},
    };

    fn frame(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = crate::test_fixtures::source(1, hour, Some(1));
        // Successful explicit reads also occur away from the scheduled slot.
        frame.metrics.table_ns[TableId::Operations.index()] = 1;
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
                    preparation_current: 25,
                    preparation_required: 100,
                    ..Default::default()
                })
                .collect(),
        ));
        frame
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
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::rows::LexicalTokenRow;
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
            crate::SourceKey {
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
            &BranchParams {
                branch_id: "1".into(),
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
        // Native repeat constructs a new identity rather than resetting the old
        // instance (141189390 -> 1413E9210 -> 142209A50).
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
        let replay = crate::test_fixtures::replay(vec![
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
        assert_eq!(
            reply
                .operations
                .iter()
                .map(|op| (
                    op.first_read,
                    op.owner_tag.as_deref(),
                    op.target_tag.as_deref()
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
        assert_eq!(reply.operations[0].runs[0].last_hour, 101);
        assert_eq!(reply.operations[3].runs[0].last_hour, 105);
        for (operation, by_hour, owner) in [
            (&reply.operations[0], 102, "renamed-mod"),
            (&reply.operations[1], 103, "mod2"),
            (&reply.operations[2], 104, "mod2"),
            (&reply.operations[3], 106, "mod2"),
        ] {
            let end = operation.end.as_ref().unwrap();
            assert_eq!(end.by_hour, by_hour);
            assert_eq!(end.owner_tag.as_deref(), Some(owner));
            assert_eq!(end.target_tag.as_deref(), Some("mod2"));
        }
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
    fn espionage_long_campaign_thousand_operations_fifty_progress_reads_fit() {
        let mut builder = Builder::default();
        for day in 0..50 {
            let mut sample = frame(100 + day * 24, 1000);
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
        assert_eq!(builder.operations.len(), 1000);
        assert!(
            builder
                .operations
                .iter()
                .all(|operation| operation.runs.len() == 50)
        );
        let reply = EspionageLogReply {
            branch_id: "1".into(),
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
        let bytes =
            json_len(&reply, MAX_PRODUCT_REPLY_BYTES).expect("complete long-campaign reply fits");
        assert!(bytes < MAX_PRODUCT_REPLY_BYTES);
        eprintln!("1000 operations x 50 preparation reads: {bytes} serialized bytes");
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
        use replay::rows::LexicalTokenRow;
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
        use replay::rows::LexicalTokenRow;
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
        use crate::test_fixtures::{CAMPAIGN, records};
        let cancelled = AtomicBool::new(false);
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100, 1)])]).unwrap(),
        );
        let source = Source::from_replay(
            crate::SourceKey {
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
            &BranchParams {
                branch_id: "1".into(),
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

    #[test]
    fn espionage_outcome_bytes_keep_recorded_language_and_refuse_non_utf8() {
        for (bytes, expected, refusal) in [
            ("Résultat\0".as_bytes(), Some("Résultat\0"), None),
            (
                &[0xff, 0, b'+'][..],
                None,
                Some("Stored operation description refused: non-UTF-8 recorded bytes"),
            ),
        ] {
            let required = bytes.len() as u64;
            let mut warnings = BTreeMap::new();
            let (outcome, missing) = outcome_at_end(
                Some(1),
                Some(0),
                Some(bytes),
                None,
                Some(required),
                Some("synthetic_language"),
                &mut warnings,
            );
            let outcome = outcome.unwrap();
            assert!(missing.is_none());
            assert!(outcome.bonus);
            assert!(!outcome.detected);
            assert_eq!(outcome.recorded_language, "synthetic_language");
            assert_eq!(outcome.description_required_bytes, required.to_string());
            assert_eq!(outcome.description.as_deref(), expected);
            assert_eq!(outcome.description_unrecorded.as_deref(), refusal);
            assert_eq!(
                warnings,
                refusal
                    .into_iter()
                    .map(|reason| (reason.to_owned(), 1))
                    .collect::<BTreeMap<_, _>>()
            );
        }
    }

    #[test]
    fn espionage_outcome_capture_refusal_preserves_independent_results() {
        let mut warnings = BTreeMap::new();
        let (outcome, missing) = outcome_at_end(
            Some(0),
            Some(1),
            Some(&[]),
            Some("operation completion description byte ring short"),
            Some(u64::MAX),
            Some("synthetic_language"),
            &mut warnings,
        );
        let outcome = outcome.unwrap();
        assert!(missing.is_none());
        assert!(!outcome.bonus);
        assert!(outcome.detected);
        assert_eq!(outcome.description, None);
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
                Some(b"recorded effect"),
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
