//! Every country's recorded operations. Reads supply values, never elapsed-time
//! estimates. A missing hour breaks runs; a complete later census can establish
//! only that an absent operation ended *by* that read.
//!
//! COperationInstance completion 1413EE390 writes terminal +0x42 and the
//! generated outcome description +0xA0. OperationRow captures the former, not
//! the latter or bonus-result +0xC0. Neither completion nor OperationTokens
//! identifies success, failure or captured operatives: outcomes stay unrecorded.
//! OperationTokens is an independent owner-target token set, added/removed by
//! authored effects (1413F10D0/1413F22B0, 14034AD00/14035E450), without an
//! operation handle. It is therefore no outcome source and is not decoded here.

use super::{
    BranchParams, LoadContext, Product, country_groups::HourSpan,
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

// Leaves room for the lease envelope; histories refuse rather than truncate.
const HISTORY_BYTES: usize = 24 * 1024 * 1024;

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
    pub description: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationEnd {
    pub by_hour: i32,
    pub last_seen: i32,
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
    // Episode invariants: a change starts a fresh episode, never repeats them
    // in each daily progress run.
    pub owner_tag: Option<String>,
    pub target_tag: Option<String>,
    pub type_key: String,
    /// Selected definitions, not completed phases (1413EB710).
    pub phase_count: u32,
    /// Manager 14118B110 tests +0x41 after completion to repeat the operation.
    pub auto_repeat: bool,
    pub duration_days: i32,
    /// All +0xE0 slots, including unassigned ones (1413E9210/141BD1CD0).
    pub assigned_slots: u32,
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
    bytes: usize,
    links: LinkTags,
}

fn budget_error() -> io::Error {
    io::Error::other("Espionage history exceeds the complete-response byte budget")
}

impl Builder {
    fn reserve(&mut self, bytes: usize) -> io::Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(budget_error)?;
        if self.bytes > HISTORY_BYTES {
            return Err(budget_error());
        }
        Ok(())
    }

    fn warn(&mut self, reason: impl Into<String>) {
        *self.warnings.entry(reason.into()).or_default() += 1;
    }

    fn mark_available(&mut self, hour: i32) -> io::Result<()> {
        if let Some(span) = self
            .available
            .last_mut()
            .filter(|span| span.last_hour.checked_add(1) == Some(hour))
        {
            span.last_hour = hour;
        } else {
            self.reserve(64)?;
            self.available.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            });
        }
        Ok(())
    }

    fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let Ok(rows) = frame.complete::<tables::Operations>() else {
            self.warn("Operations unavailable or incomplete; values and endings unknown");
            return Ok(());
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
                self.mark_available(hour)?;
            }
            return Ok(());
        }
        // A malformed duplicate census is not evidence of absence or zero.
        let mut ids = BTreeSet::new();
        if rows.iter().any(|row| !ids.insert(row.id)) {
            self.warn("Duplicate operation handles refused");
            return Ok(());
        }
        let labels = self.links.update(frame).is_ok();
        if !labels {
            self.warn("Country identities unavailable at operation read");
        }
        self.reserve(16)?;
        self.reads.push(hour);
        self.mark_available(hour)?;
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
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
                let (outcome, outcome_unrecorded) = outcome_at_end(None);
                operation.end = Some(EspionageOperationEnd {
                    by_hour: hour,
                    last_seen: supplying_read(
                        &self.reads,
                        operation.runs.last().expect("observed operation").last_hour,
                    ),
                    reason: EspionageEndReason::Disappeared,
                    outcome,
                    outcome_unrecorded,
                    owner_tag: labels
                        .then(|| self.links.get(owner).cloned())
                        .flatten()
                        .or_else(|| operation.owner_tag.clone()),
                    target_tag: labels
                        .then(|| self.links.get(target).cloned())
                        .flatten()
                        .or_else(|| operation.target_tag.clone()),
                });
            }
        }
        for row in rows {
            let key = lexical
                .and_then(|tokens| {
                    tokens
                        .binary_search_by_key(&row.definition_token, |token| token.token)
                        .ok()
                        .map(|index| &tokens[index])
                })
                .and_then(|token| std::str::from_utf8(&token.literal).ok())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("?{}", row.definition_token));
            self.types.insert(key.clone());
            if key.starts_with('?') {
                self.warn(format!(
                    "Operation definition token {} not recorded",
                    row.definition_token
                ));
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
                    if operation.owner_tag != owner_tag
                        || operation.target_tag != target_tag
                        || operation.type_key != key
                        || operation.phase_count != row.phase_count
                        || operation.auto_repeat != (row.auto_repeat != 0)
                        || operation.duration_days != row.duration_days
                        || operation.assigned_slots != row.assigned_slots
                    {
                        return false;
                    }
                    let previous = operation.runs.last().expect("open operation");
                    !(previous.terminal && !run.terminal)
                        && !(previous.start_hour.is_some()
                            && run.start_hour.is_some()
                            && previous.start_hour != run.start_hour)
                })
                .map(|(index, _)| *index);
            let index = match continuing {
                Some(index) => index,
                None => {
                    if let Some((old, previous_fingerprint)) = self.open.remove(&row.id) {
                        let previous = &mut self.operations[old];
                        if previous.end.is_none() {
                            let (outcome, outcome_unrecorded) = outcome_at_end(None);
                            previous.end = Some(EspionageOperationEnd {
                                by_hour: hour,
                                last_seen: supplying_read(
                                    &self.reads,
                                    previous.runs.last().expect("observed operation").last_hour,
                                ),
                                reason: EspionageEndReason::Disappeared,
                                outcome,
                                outcome_unrecorded,
                                owner_tag: (labels && previous_fingerprint == fingerprint)
                                    .then(|| self.links.get(row.owner).cloned())
                                    .flatten()
                                    .or_else(|| previous.owner_tag.clone()),
                                target_tag: (labels && previous_fingerprint == fingerprint)
                                    .then(|| self.links.get(row.target).cloned())
                                    .flatten()
                                    .or_else(|| previous.target_tag.clone()),
                            });
                        }
                    }
                    self.reserve(
                        512 + key.len()
                            + owner_tag.as_ref().map_or(0, String::len)
                            + target_tag.as_ref().map_or(0, String::len),
                    )?;
                    let index = self.operations.len();
                    self.operations.push(EspionageOperation {
                        id: format!("{}:{hour}", row.id),
                        first_read: hour,
                        start_hour: run.start_hour,
                        start_known_at: run.start_hour.map(|_| hour),
                        owner_tag,
                        target_tag,
                        type_key: key,
                        phase_count: row.phase_count,
                        auto_repeat: row.auto_repeat != 0,
                        duration_days: row.duration_days,
                        assigned_slots: row.assigned_slots,
                        runs: Vec::new(),
                        end: None,
                    });
                    self.open.insert(row.id, (index, fingerprint));
                    index
                }
            };
            let operation = &self.operations[index];
            let same = operation.runs.last().is_some_and(|previous| {
                let mut previous = previous.clone();
                previous.first_hour = hour;
                previous.last_hour = hour;
                previous.read_hour = hour;
                previous == run
                    && operation.runs.last().unwrap().last_hour.checked_add(1) == Some(hour)
            });
            if !same {
                self.reserve(json_len(&run, HISTORY_BYTES).ok_or_else(budget_error)? + 32)?;
            }
            let operation = &mut self.operations[index];
            if operation.start_hour.is_none() && run.start_hour.is_some() {
                operation.start_hour = run.start_hour;
                operation.start_known_at = Some(hour);
            }
            if run.terminal && operation.end.is_none() {
                let (outcome, outcome_unrecorded) = outcome_at_end(Some(row));
                operation.end = Some(EspionageOperationEnd {
                    by_hour: hour,
                    last_seen: hour,
                    reason: EspionageEndReason::Completed,
                    outcome,
                    outcome_unrecorded,
                    owner_tag: operation.owner_tag.clone(),
                    target_tag: operation.target_tag.clone(),
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
        Ok(())
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

/// Single capture adapter. Completion 1413EE390 writes +0xA0 outcome text
/// and +0xC0 bonus; neither is recorded, and independent tokens prove no result.
fn outcome_at_end(_row: Option<&OperationRow>) -> (Option<EspionageOutcome>, Option<String>) {
    (None, Some("Outcome not recorded".into()))
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
            builder.observe(decoded)?;
        }
    }
    let unavailable = builder
        .available
        .is_empty()
        .then(|| "Operations was unavailable, incomplete or never read in this recording.".into());
    Ok(EspionageLogReply {
        branch_id: params.branch_id.clone(),
        first_hour,
        last_hour,
        available: builder.available,
        read_hours: builder.reads,
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
    })
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
        json_len(&reply, HISTORY_BYTES).ok_or_else(budget_error)?;
        Ok(reply)
    }
}

/// Retains one branch scan; content changes refresh presentation independently.
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
        builder.observe(&frame(100, 5000)).unwrap();
        builder.observe(&frame(101, 5000)).unwrap();
        assert_eq!(builder.operations.len(), 5000);
        assert!(
            builder
                .operations
                .iter()
                .all(|op| op.runs.len() == 1 && op.runs[0].last_hour == 101)
        );
        assert_eq!(builder.reads, [100, 101]);
        assert!(builder.bytes < HISTORY_BYTES);
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
        assert!(json_len(&reply, HISTORY_BYTES).is_some());
    }

    #[test]
    fn espionage_missing_rows_and_gaps_never_supply_zero_or_finish_time() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1)).unwrap();
        let mut missing = frame(101, 1);
        *missing.tables[TableId::Operations.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        builder.observe(&missing).unwrap();
        assert!(builder.operations[0].end.is_none());
        builder.observe(&frame(102, 1)).unwrap();
        assert_eq!(builder.operations[0].runs.len(), 2);
        let mut gone = frame(103, 1);
        *gone.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&gone).unwrap();
        assert_eq!(builder.operations[0].end.as_ref().unwrap().by_hour, 103);
        assert_eq!(builder.operations[0].end.as_ref().unwrap().last_seen, 102);
        assert_eq!(builder.reads, [100, 102, 103]);
    }

    #[test]
    fn espionage_daily_carry_keeps_its_read_date_and_never_crosses_a_gap() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1)).unwrap();
        let mut carried = frame(101, 1);
        carried.metrics.table_ns[TableId::Operations.index()] = 0;
        builder.observe(&carried).unwrap();
        assert_eq!(builder.reads, [100]);
        assert_eq!(builder.operations[0].runs[0].read_hour, 100);
        assert_eq!(builder.operations[0].runs[0].last_hour, 101);
        let mut failed = carried.clone();
        failed.hour = 102;
        *failed.tables[TableId::Operations.index()] = TableView::Unavailable {
            reason: "failed read".into(),
        };
        builder.observe(&failed).unwrap();
        carried.hour = 103;
        builder.observe(&carried).unwrap();
        assert_eq!(builder.available[0].last_hour, 101);
        assert_eq!(builder.operations[0].runs[0].last_hour, 101);
        builder.observe(&frame(104, 1)).unwrap();
        assert_eq!(builder.operations[0].runs.len(), 2);
        carried.hour = 105;
        builder.observe(&carried).unwrap();
        let mut gone = frame(106, 1);
        *gone.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&gone).unwrap();
        let end = builder.operations[0].end.as_ref().unwrap();
        assert_eq!((end.last_seen, end.by_hour), (104, 106));
        assert_eq!(builder.reads, [100, 104, 106]);
    }

    #[test]
    fn espionage_execution_start_is_not_visible_before_its_read_and_repeat_is_separate() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1)).unwrap();
        let mut sample = frame(101, 1);
        let mut row = sample.complete::<tables::Operations>().unwrap()[0].clone();
        row.duration_days = 5;
        row.start_hour = 101;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row.clone()]));
        builder.observe(&sample).unwrap();
        assert_eq!(builder.operations[0].runs[0].start_hour, None);
        assert_eq!(builder.operations[1].start_known_at, Some(101));
        sample.hour = 102;
        row.terminal = 1;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row.clone()]));
        builder.observe(&sample).unwrap();
        assert_eq!(builder.operations[1].end.as_ref().unwrap().by_hour, 102);
        sample.hour = 103;
        // Native repeat constructs a new identity rather than resetting the old
        // instance (141189390 -> 1413E9210 -> 142209A50).
        row.id = 2;
        row.terminal = 0;
        row.duration_days = 0;
        *sample.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row]));
        builder.observe(&sample).unwrap();
        assert_eq!(builder.operations.len(), 3);
    }

    #[test]
    fn espionage_country_tag_is_the_read_identity_and_missing_assignments_are_unknown() {
        let mut builder = Builder::default();
        let mut sample = frame(100, 1);
        *sample.tables[TableId::OperationAssignments.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        builder.observe(&sample).unwrap();
        sample.hour = 101;
        let mut country = sample.complete::<tables::Countries>().unwrap()[0].clone();
        country.tag = b"renamed-mod".to_vec();
        *sample.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![country]));
        builder.observe(&sample).unwrap();
        assert_eq!(builder.operations[0].owner_tag.as_deref(), Some("mod1"));
        assert_eq!(
            builder.operations[1].owner_tag.as_deref(),
            Some("renamed-mod")
        );
        assert_eq!(builder.operations[1].runs[0].operatives, None);
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
            builder.observe(&sample).unwrap();
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
            types: vec![],
            operations: builder.operations,
            warnings: vec![],
            unavailable: None,
        };
        let bytes = json_len(&reply, HISTORY_BYTES).expect("complete long-campaign reply fits");
        assert!(bytes <= builder.bytes && bytes < HISTORY_BYTES);
        eprintln!("1000 operations x 50 preparation reads: {bytes} serialized bytes");
    }

    #[test]
    fn espionage_invariant_changes_split_episodes_and_end_tags_fall_back() {
        let mut builder = Builder::default();
        builder.observe(&frame(100, 1)).unwrap();
        let mut changed = frame(101, 1);
        let mut row = changed.complete::<tables::Operations>().unwrap()[0].clone();
        row.phase_count = 4;
        *changed.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(vec![row]));
        builder.observe(&changed).unwrap();
        assert_eq!(builder.operations.len(), 2);
        assert_eq!(
            builder.operations[0]
                .end
                .as_ref()
                .unwrap()
                .owner_tag
                .as_deref(),
            Some("mod1")
        );
        let mut gone = frame(102, 1);
        *gone.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "missing identities".into(),
        };
        *gone.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        builder.observe(&gone).unwrap();
        let end = builder.operations[1].end.as_ref().unwrap();
        assert_eq!(end.owner_tag.as_deref(), Some("mod1"));
        assert_eq!(end.target_tag.as_deref(), Some("mod1"));
        assert!(end.outcome.is_none());
        assert_eq!(
            end.outcome_unrecorded.as_deref(),
            Some("Outcome not recorded")
        );
    }

    #[test]
    fn espionage_budget_refuses_instead_of_truncating() {
        let mut builder = Builder {
            bytes: HISTORY_BYTES,
            ..Default::default()
        };
        assert!(builder.observe(&frame(100, 1)).is_err());
        assert!(builder.operations.is_empty());
    }
}
