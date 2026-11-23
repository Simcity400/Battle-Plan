
use super::{Country, IDENTITY, Tags, branch, missing, operative_content};
use dashboard_base::{Source, products::{LoadContext, Product, at_hour::{At, AtHour, Owner, Read as RowRead}, log_page::{self as logs, LogCell, LogPageParams, LogPageReply, LogPageRow}}, queries::country_ids::{Names, RunLinks}};
use dashboard_query::{
    Query,
    log::{Cell, FacetFilter},
    read::{self, Keys, Read},
};
use game_model::calendar::HOUR_EPOCH;
use replay_lanes::{lanes, points};
use replay_columnar::manifest::Status;
use replay_data::rows::{OperationAssignmentRow, OperationCompletionRow, OperationRow, OperativeMembershipRow, TableId, TableSchema, tables};
use replay_frame::frame::{DecodedFrame, cells_rows};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const LOG: &str = "operation-completions";

const ID: usize = 0;
const TARGET: usize = 1;
const DEFINITION: usize = 2;
const COMPLETION_HOUR: usize = 3;
const TERMINAL: usize = 4;
const BONUS: usize = 5;
const DETECTED: usize = 6;
const DESCRIPTION: usize = 7;
const DESCRIPTION_ERROR: usize = 8;
const DESCRIPTION_REQUIRED: usize = 9;

pub const COLUMNS: [&str; 16] = [
    "id",
    "owner",
    "ownerTag",
    "target",
    "targetTag",
    "operation",
    "completionHour",
    "terminal",
    "bonus",
    "detected",
    "description",
    "descriptionUnrecorded",
    "descriptionBytes",
    "startHour",
    "firstSeen",
    "lastSeen",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperationState {
    #[doc = " Written by the launch with the duration: none before it."]
    pub start_hour: Option<i32>,
    pub duration_days: i32,
    #[doc = " Selected phase definitions, not completed phases."]
    pub phase_count: u32,
    #[doc = " Zero-based, at the read; none where the game's view selects none."]
    pub current_phase: Option<u32>,
    #[doc = " Signed days since the start at the read; none before the launch, once terminal, and at the"]
    #[doc = " game's own -1 and full-duration results."]
    pub elapsed_days: Option<i32>,
    #[doc = " The resource meter, fixed5 from 0 to 100,000; not equipment readiness."]
    pub preparation_ratio: Option<i32>,
    #[doc = " Every assignment slot, assigned or not."]
    pub assigned_slots: u32,
    pub auto_repeat: bool,
    pub auto_commence: bool,
    pub terminal: bool,
}

impl OperationState {
    pub fn of(row: &OperationRow, hour: i32) -> Self {
        Self {
            start_hour: (row.duration_days != 0).then_some(row.start_hour),
            duration_days: row.duration_days,
            phase_count: row.phase_count,
            current_phase: current_phase(row, hour),
            elapsed_days: elapsed_days(row, hour),
            preparation_ratio: preparation_ratio(row),
            assigned_slots: row.assigned_slots,
            auto_repeat: row.auto_repeat != 0,
            auto_commence: row.flag_40 != 0,
            terminal: row.terminal != 0,
        }
    }
}

pub fn preparation_ratio(row: &OperationRow) -> Option<i32> {
    let current = row.preparation_current;
    let ratio = if row.preparation_required == 0 {
        0
    } else if [i64::MIN, i64::MIN + 1, i64::MAX].contains(&current) {
        100_000
    } else {
        current.wrapping_mul(100_000).checked_div(row.preparation_required)?.clamp(0, 100_000)
    };
    Some(ratio as i32)
}

pub fn elapsed_days(row: &OperationRow, hour: i32) -> Option<i32> {
    if row.duration_days == 0 || row.terminal != 0 {
        return None;
    }
    let day = |hour: i32| hour.wrapping_sub(HOUR_EPOCH) / 24;
    let elapsed = day(hour).wrapping_sub(day(row.start_hour));
    (elapsed != -1 && elapsed != row.duration_days).then_some(elapsed)
}

pub fn current_phase(row: &OperationRow, hour: i32) -> Option<u32> {
    let elapsed = elapsed_days(row, hour)?;
    let phase = elapsed.wrapping_mul(row.phase_count as i32).checked_div(row.duration_days)?;
    u32::try_from(phase).ok().filter(|phase| *phase < row.phase_count)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeName {
    pub id: String,
    pub name: Option<String>,
    pub unavailable: Option<String>,
}

pub fn operatives(row: &OperationRow, slots: Option<&[OperationAssignmentRow]>, members: Option<&[OperativeMembershipRow]>) -> Option<Vec<OperativeName>> {
    let mut slots: Vec<&OperationAssignmentRow> = slots?.iter().filter(|slot| slot.operation == row.id).collect();
    slots.sort_unstable_by_key(|slot| slot.ordinal);
    if slots.len() != row.assigned_slots as usize || slots.iter().enumerate().any(|(at, slot)| slot.ordinal as usize != at) {
        return None;
    }
    let mut named: HashMap<u64, Option<&OperativeMembershipRow>> = HashMap::new();
    for member in members.unwrap_or_default() {
        named
            .entry(member.id)
            .and_modify(|held| {
                if held.is_some_and(|held| held.name != member.name) {
                    *held = None;
                }
            })
            .or_insert(Some(member));
    }
    Some(
        slots
            .iter()
            .filter(|slot| slot.operative != 0)
            .map(|slot| {
                let name = match members {
                    None => Err("The census of operatives was not recorded at this read".to_owned()),
                    Some(_) => named
                        .get(&slot.operative)
                        .copied()
                        .flatten()
                        .ok_or_else(|| "Assigned operative name missing or contradictory at this read".to_owned())
                        .and_then(|member| member.name.require("name of operative memberships").cloned())
                        .and_then(|bytes| {
                            std::str::from_utf8(content_reader_parser::parser::c_string(&bytes))
                                .map(str::to_owned)
                                .map_err(|_| "Assigned operative name has non-text bytes".to_owned())
                        }),
                };
                match name {
                    Ok(name) => OperativeName { id: slot.operative.to_string(), name: Some(name), unavailable: None },
                    Err(why) => OperativeName { id: slot.operative.to_string(), name: None, unavailable: Some(why) },
                }
            })
            .collect(),
    )
}

fn operative_ranges(assignments: &[OperationAssignmentRow]) -> Vec<(u128, u128)> {
    let ids: BTreeSet<u64> = assignments.iter().map(|slot| slot.operative).filter(|id| *id != 0).collect();
    ids.into_iter().map(|id| (u128::from(id) << 40, u128::from(id) << 40 | ((1u128 << 40) - 1))).collect()
}

fn literal(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().filter(|text| !text.is_empty()).map(str::to_owned)
}

fn literals(recording: &replay_recording::columnar_frames::Recording, run: u32, hour: i32, tokens: &[u128]) -> io::Result<HashMap<u32, String>> {
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let snapshots = read::at(recording, run, &[Read { table: TableId::LexicalTokens, columns: None, keys: Keys::These(tokens) }], hour)?;
    let Some(snapshot) = snapshots.first() else { return Ok(HashMap::new()) };
    let view = snapshot.view();
    Ok(snapshot.rows.iter().filter_map(|&(key, slot)| Some((key as u32, literal(read::text(&view, 0, slot)?)?))).collect())
}

fn typed<T: TableSchema>(snapshot: &lanes::Snapshot, format: u16) -> io::Result<Vec<T::Row>>
where
    T::Row: Clone,
{
    let rows = cells_rows(T::ID, &snapshot.view(), &snapshot.rows, format)?;
    Ok(T::take(rows).unwrap_or_default())
}

fn available(snapshot: &lanes::Snapshot) -> bool {
    matches!(snapshot.status, Status::Complete | Status::Truncated { .. })
}

pub struct EspionageOperations;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EspionageOperationsParams {
    pub branch_id: String,
    #[doc = " Only this country's operations, run by it or against it; every one when none."]
    #[serde(default)]
    pub country: Option<dashboard_base::products::Country>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperationUnderWay {
    #[doc = " The full handle, as a decimal string."]
    pub id: String,
    pub owner: Country,
    pub target: Country,
    #[doc = " The definition's token as the recording names it; none where its literal was not recorded."]
    pub operation: Option<String>,
    pub state: OperationState,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationsReply {
    #[doc = " By handle; an operation that completed and waits for its removal is not one."]
    pub operations: Vec<OperationUnderWay>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EspionageOperationsReply);

impl AtHour for EspionageOperations {
    const KEY: &'static str = "espionage-operations";
    const TITLE: &'static str = "Espionage operations";
    const CONTENT_DEPENDENT: bool = false;
    type Params = EspionageOperationsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = EspionageOperationsReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EspionageOperationsParams::decl(), Country::decl(), OperationState::decl(), OperationUnderWay::decl(), EspionageOperationsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.country.as_ref().map_or(Ok(()), dashboard_base::products::validate_country)
    }

    fn owner(params: &Self::Params) -> Option<&dashboard_base::products::Country> {
        params.country.as_ref()
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Operations, TableId::LexicalTokens, IDENTITY[0], IDENTITY[1]]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<RowRead> {
        match step {
            0 => vec![RowRead::all(TableId::Operations), RowRead::all(IDENTITY[0]), RowRead::all(IDENTITY[1])],
            1 => {
                let tokens: BTreeSet<u32> = held.complete::<tables::Operations>().map(|rows| rows.iter().map(|row| row.definition_token).collect()).unwrap_or_default();
                vec![RowRead::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let made = (|| -> io::Result<(Vec<OperationUnderWay>, Vec<String>)> {
            let tags = Tags::read(frame)?;
            let rows = frame.complete::<tables::Operations>()?;
            let tokens: HashMap<u32, String> = frame
                .complete::<tables::LexicalTokens>()
                .map(|rows| rows.iter().filter_map(|row| Some((row.token, literal(&row.literal)?))).collect())
                .unwrap_or_default();
            let mut warnings = Vec::new();
            warnings.extend(missing(frame, TableId::LexicalTokens));
            let unnamed = rows.iter().filter(|row| !tokens.contains_key(&row.definition_token)).count();
            if unnamed != 0 && frame.complete::<tables::LexicalTokens>().is_ok() {
                warnings.push(format!("{unnamed} operations name a definition token with no recorded text"));
            }
            let mine = |raw: u32| at.owner.is_none_or(|owner| tags.slot(raw) == Some(owner.slot));
            let operations = rows
                .iter()
                .filter(|row| row.terminal == 0 && (mine(row.owner) || mine(row.target)))
                .map(|row| OperationUnderWay {
                    id: row.id.to_string(),
                    owner: Country::of(&tags, row.owner),
                    target: Country::of(&tags, row.target),
                    operation: tokens.get(&row.definition_token).cloned(),
                    state: OperationState::of(row, frame.hour),
                })
                .collect();
            Ok((operations, warnings))
        })();
        Ok(match made {
            Ok((operations, warnings)) => EspionageOperationsReply { operations, missing: missing(frame, TableId::Operations), warnings },
            Err(error) => EspionageOperationsReply { operations: Vec::new(), missing: Some(error.to_string()), warnings: Vec::new() },
        })
    }
}

pub struct EspionageOperation;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EspionageOperationParams {
    pub branch_id: String,
    #[doc = " The full handle, as a decimal string."]
    pub id: String,
    #[doc = " The hour asked: the operation is read there, or at its last hour before it."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageOperationReply {
    pub id: String,
    #[doc = " The first and last hours the census held it in the stretch of the hour asked."]
    pub first_seen: i32,
    pub last_seen: i32,
    #[doc = " The hour it was read at."]
    pub read_hour: i32,
    pub state: OperationState,
    #[doc = " None where its slots or the census of operatives were not read."]
    pub operatives: Option<Vec<OperativeName>>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EspionageOperationReply);

impl Product for EspionageOperation {
    const KEY: &'static str = "espionage-operation";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = EspionageOperationParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, id, hour: Read];
    type Reply = EspionageOperationReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EspionageOperationParams::decl(), OperativeName::decl(), EspionageOperationReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid operation id")
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let id: u64 = params.id.parse().map_err(|_| dashboard_core::invalid("Invalid operation id"))?;
        let stretch = *query.timeline.at(params.hour).ok_or_else(|| dashboard_core::refused("The operation's hour is not recorded"))?;
        let lives = dashboard_base::queries::lifetimes(source, stretch.run, TableId::Operations)?;
        let (first, last) = lives
            .iter()
            .find(|(key, ..)| *key == u128::from(id))
            .and_then(|&(_, first, last)| Some((i32::try_from(first).ok()?.max(stretch.first), i32::try_from(last).ok()?.min(stretch.last))))
            .filter(|&(first, last)| first <= params.hour && first <= last)
            .ok_or_else(|| dashboard_core::refused("The census did not hold this operation by this hour"))?;
        let hour = params.hour.min(last);
        let recording = query.replay.recording();
        let key = [u128::from(id)];
        let held = read::at(
            recording,
            stretch.run,
            &[
                Read { table: TableId::Operations, columns: None, keys: Keys::These(&key) },
                Read { table: TableId::OperationAssignments, columns: None, keys: Keys::Range(u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX)) },
            ],
            hour,
        )?;
        let row = typed::<tables::Operations>(&held[0], recording.format())?
            .into_iter()
            .next()
            .ok_or_else(|| dashboard_core::refused("The operation was not recorded at this read"))?;
        let mut warnings = Vec::new();
        let slots = if available(&held[1]) { Some(typed::<tables::OperationAssignments>(&held[1], recording.format())?) } else {
            warnings.push("operation assignments was not recorded at this read".to_owned());
            None
        };
        let ranges = slots.as_deref().map(operative_ranges).unwrap_or_default();
        let members = if ranges.is_empty() {
            Some(Vec::new())
        } else {
            let census = read::at(recording, stretch.run, &[Read { table: TableId::OperativeMemberships, columns: None, keys: Keys::Ranges(&ranges) }], hour)?;
            available(&census[0]).then(|| typed::<tables::OperativeMemberships>(&census[0], recording.format())).transpose()?
        };
        Ok(EspionageOperationReply {
            id: params.id.clone(),
            first_seen: first,
            last_seen: last,
            read_hour: hour,
            state: OperationState::of(&row, hour),
            operatives: operatives(&row, slots.as_deref(), members.as_deref()),
            warnings,
        })
    }
}

pub struct EspionageEnded;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EspionageEndedParams {
    pub branch_id: String,
    #[doc = " The span's first hour; the timeline's when none."]
    #[serde(default)]
    pub from: Option<i32>,
    pub to: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EndedOperation {
    pub id: String,
    pub owner: Country,
    pub target: Country,
    pub operation: Option<String>,
    pub first_seen: i32,
    pub last_seen: i32,
    #[doc = " The first complete read that no longer held it."]
    pub by_hour: i32,
    #[doc = " Whether completions were read whole every hour of its life: where not, a completion may have"]
    #[doc = " gone unseen and its end is not known to be a cancellation."]
    pub completions_recorded: bool,
    #[doc = " As last read."]
    pub state: OperationState,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageEndedReply {
    pub operations: Vec<EndedOperation>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EspionageEndedReply);

fn next_read(reads: &[(i32, i32)], last: i32) -> Option<i32> {
    let at = reads.partition_point(|run| run.1 <= last);
    reads.get(at).map(|run| run.0.max(last + 1))
}

fn covered(runs: &[(i32, i32)], first: i32, last: i32) -> bool {
    let at = runs.partition_point(|run| run.1 < first);
    let mut from = first;
    for run in &runs[at..] {
        if run.0 > from {
            return false;
        }
        if run.1 >= last {
            return true;
        }
        from = run.1 + 1;
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ending {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub by: i32,
}

pub fn endings(lives: &[(u128, i64, i64)], stretch: (i32, i32), reads: &[(i32, i32)], from: i32, to: i32) -> Vec<Ending> {
    lives
        .iter()
        .filter_map(|&(key, first, last)| {
            let (first, last) = (i32::try_from(first).ok()?.max(stretch.0), i32::try_from(last).ok()?.min(stretch.1));
            if first > last {
                return None;
            }
            let by = next_read(reads, last)?;
            (from <= by && by <= to.min(stretch.1)).then_some(Ending { key, first, last, by })
        })
        .collect()
}

impl Product for EspionageEnded {
    const KEY: &'static str = "espionage-ended";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = EspionageEndedParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, from: Start, to: End];
    type Reply = EspionageEndedReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EspionageEndedParams::decl(), EndedOperation::decl(), EspionageEndedReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        if params.from.is_some_and(|from| from > params.to) { Err("Invalid span") } else { Ok(()) }
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let from = params.from.unwrap_or(query.timeline.first);
        let recording = query.replay.recording();
        let names = Names::new(&query.holders);
        let mut operations = Vec::new();
        let mut warnings = Vec::new();
        let stretches: Vec<_> = query.timeline.stretches.iter().filter(|stretch| stretch.last >= from && stretch.first <= params.to).copied().collect();
        for stretch in stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let end = stretch.last.min(params.to);
            let lives = dashboard_base::queries::lifetimes(source, stretch.run, TableId::Operations)?;
            let reads = dashboard_query::cost::storage(|| lanes::complete_reads(recording, stretch.run, TableId::Operations, stretch.first, end))?;
            let ended = endings(&lives, (stretch.first, stretch.last), &reads, from, end);
            if ended.is_empty() {
                continue;
            }
            let (low, high) = ended.iter().fold((i32::MAX, i32::MIN), |(low, high), ending| (low.min(ending.first), high.max(ending.by)));
            let completions = completed(&query, stretch.run, low, high)?;
            let recorded = dashboard_query::cost::storage(|| lanes::complete_reads(recording, stretch.run, TableId::OperationCompletions, low, high))?;
            let ended: Vec<Ending> = ended.into_iter().filter(|ending| !completions.contains(&(ending.key as u64))).collect();
            let rows = rows_at(recording, stretch.run, &ended.iter().map(|ending| (ending.last, ending.key)).collect::<Vec<_>>())?;
            let tokens: Vec<u128> = rows.values().map(|row| u128::from(row.definition_token)).collect::<BTreeSet<_>>().into_iter().collect();
            let keys = literals(recording, stretch.run, end, &tokens)?;
            let links = dashboard_base::queries::links(source, stretch.run)?;
            let mut unread = 0usize;
            for ending in ended {
                let Some(row) = rows.get(&(ending.last, ending.key)) else {
                    unread += 1;
                    continue;
                };
                operations.push(EndedOperation {
                    id: row.id.to_string(),
                    owner: named(&names, &links, row.owner, ending.last),
                    target: named(&names, &links, row.target, ending.last),
                    operation: keys.get(&row.definition_token).cloned(),
                    first_seen: ending.first,
                    last_seen: ending.last,
                    by_hour: ending.by,
                    completions_recorded: covered(&recorded, ending.first, ending.by),
                    state: OperationState::of(row, ending.last),
                });
            }
            if unread != 0 {
                warnings.push(format!("{unread} ended operations are not shown: their last row was not recorded"));
            }
        }
        operations.sort_by(|a, b| b.by_hour.cmp(&a.by_hour).then_with(|| a.id.cmp(&b.id)));
        Ok(EspionageEndedReply { operations, warnings })
    }
}

fn named(names: &Names, links: &RunLinks, raw: u32, hour: i32) -> Country {
    Country { id: raw, tag: names.tag_at(links, raw, hour).map(str::to_owned) }
}

fn completed(query: &Query<'_>, run: u32, from: i32, to: i32) -> io::Result<HashSet<u64>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| dashboard_core::invalid(format!("No log is declared as {LOG}")))?;
    let reader = dashboard_query::log::Reader::new(query, declared)?;
    Ok(reader
        .inserted(run, &[(from, to)])?
        .into_iter()
        .filter_map(|row| match row.cells.get(ID) {
            Some(Cell::Number(id)) => Some(*id as u64),
            _ => None,
        })
        .collect())
}

fn rows_at(recording: &replay_recording::columnar_frames::Recording, run: u32, asks: &[(i32, u128)]) -> io::Result<HashMap<(i32, u128), OperationRow>> {
    let format = recording.format();
    let mut rows = HashMap::new();
    if asks.is_empty() {
        return Ok(rows);
    }
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, TableId::Operations, None, asks, &mut |hour, key, row| {
            if let Some((view, slot)) = row
                && let Some(row) = cells_rows(TableId::Operations, &view, &[(key, slot)], format)
                    .ok()
                    .and_then(|rows| rows.rows::<tables::Operations>().and_then(|rows| rows.first().cloned()))
            {
                rows.insert((hour, key), row);
            }
            Ok(())
        })
    })?;
    Ok(rows)
}

pub struct EspionageTypes;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EspionageTypesParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperationType {
    #[doc = " The definition's token, as the content names it."]
    pub key: String,
    pub name: Option<String>,
    #[doc = " The definition's icon, drawn."]
    pub icon: Option<String>,
    #[doc = " Why the name or the icon is not shown."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageTypesReply {
    #[doc = " By key."]
    pub types: Vec<OperationType>,
    #[doc = " Why there are none: the content is not configured or not read."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EspionageTypesReply);

impl Product for EspionageTypes {
    const KEY: &'static str = "espionage-types";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = EspionageTypesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = EspionageTypesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EspionageTypesParams::decl(), OperationType::decl(), EspionageTypesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, _params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let content = match operative_content::content(source, context) {
            Ok(content) => content,
            Err(why) => return Ok(EspionageTypesReply { types: Vec::new(), missing: Some(why), warnings: Vec::new() }),
        };
        let types = content
            .operation_tokens()
            .map(|key| {
                let (name, icon) = content.operation_type(key);
                let icon = icon.and_then(|art| content.draw(&art));
                let missing: Vec<String> = [name.as_ref().err(), icon.as_ref().err()].into_iter().flatten().cloned().collect();
                OperationType { key: key.to_owned(), name: name.ok(), icon: icon.ok(), missing: (!missing.is_empty()).then(|| missing.join("; ")) }
            })
            .collect();
        Ok(EspionageTypesReply { types, missing: None, warnings: content.warnings.clone() })
    }
}

pub struct EspionageTypeTokens;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperationTypeTokens {
    #[doc = " The definition's token, as the content names it."]
    pub key: String,
    #[doc = " The lexical tokens the recording gave it; empty where it never held one."]
    pub tokens: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageTypeTokensReply {
    #[doc = " By key."]
    pub types: Vec<OperationTypeTokens>,
    #[doc = " Why there are none: the content is not configured or not read."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EspionageTypeTokensReply);

impl Product for EspionageTypeTokens {
    const KEY: &'static str = "espionage-type-tokens";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EspionageTypesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = EspionageTypeTokensReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OperationTypeTokens::decl(), EspionageTypeTokensReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let content = match operative_content::content(source, context) {
            Ok(content) => content,
            Err(why) => return Ok(EspionageTypeTokensReply { types: Vec::new(), missing: Some(why), warnings: Vec::new() }),
        };
        let mut runs: Vec<(u32, i32, i32)> = Vec::new();
        for stretch in &query.timeline.stretches {
            match runs.iter_mut().find(|(run, ..)| *run == stretch.run) {
                Some(held) => (held.1, held.2) = (held.1.min(stretch.first), held.2.max(stretch.last)),
                None => runs.push((stretch.run, stretch.first, stretch.last)),
            }
        }
        let mut lexicons = Vec::with_capacity(runs.len());
        for &(run, first, last) in &runs {
            dashboard_text::metadata::check_cancel(cancelled)?;
            lexicons.push((dashboard_base::queries::lexicon(source, run)?, first, last));
        }
        let types = content
            .operation_tokens()
            .map(|key| {
                let held: BTreeSet<u32> = lexicons.iter().flat_map(|(lexicon, first, last)| lexicon.tokens(key.as_bytes(), *first, *last)).collect();
                OperationTypeTokens { key: key.to_owned(), tokens: held.into_iter().collect() }
            })
            .collect();
        Ok(EspionageTypeTokensReply { types, missing: None, warnings: Vec::new() })
    }
}

pub struct EspionageLog;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageLogParams {
    pub branch_id: String,
    #[serde(default)]
    pub facets: Vec<FacetFilter>,
    pub from: Option<f64>,
    pub to: f64,
    pub cursor: Option<String>,
    pub page_size: u32,
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn number_cell(value: Option<i64>) -> LogCell {
    value.map_or(LogCell::NotRecorded, |value| LogCell::Number { value: value.to_string() })
}

fn text_cell(value: Option<String>) -> LogCell {
    value.map_or(LogCell::Missing, |value| LogCell::Text { value })
}

pub fn description(cells: &[Cell]) -> Result<String, String> {
    match (number(cells, DESCRIPTION_ERROR), cells.get(DESCRIPTION)) {
        (Some(0), Some(Cell::Text(text))) if text.contains('\u{FFFD}') => Err("Stored operation description refused: non-UTF-8 recorded bytes".into()),
        (Some(0), Some(Cell::Text(text))) => Ok(text.clone()),
        (Some(error @ 1..=2), _) => Err(OperationCompletionRow { description_error: error as u8, ..OperationCompletionRow::default() }
            .description_refusal()
            .unwrap_or("Stored operation description refused")
            .to_owned()),
        _ => Err("Stored operation description not recorded".into()),
    }
}

struct Sightings(HashMap<u64, (i32, i32, Option<i32>)>);

impl Sightings {
    fn read(source: &Source, query: &Query<'_>, asked: &[(u64, i32)]) -> io::Result<Self> {
        let mut by_run: BTreeMap<u32, (i32, i32, HashSet<u128>)> = BTreeMap::new();
        for &(id, hour) in asked {
            if let Some(stretch) = query.timeline.at(hour) {
                by_run.entry(stretch.run).or_insert((stretch.first, stretch.last, HashSet::new())).2.insert(u128::from(id));
            }
        }
        let start = dashboard_declare::field(TableId::Operations, "start_hour")?;
        let duration = dashboard_declare::field(TableId::Operations, "duration_days")?;
        let recording = query.replay.recording();
        let mut out = HashMap::new();
        for (run, (first, last, ids)) in by_run {
            let lives = dashboard_base::queries::lifetimes(source, run, TableId::Operations)?;
            let mine: HashMap<u128, (i32, i32)> = lives
                .iter()
                .filter(|(key, ..)| ids.contains(key))
                .filter_map(|&(key, a, b)| Some((key, (i32::try_from(a).ok()?.max(first), i32::try_from(b).ok()?.min(last)))))
                .filter(|(_, (a, b))| a <= b)
                .collect();
            if mine.is_empty() {
                continue;
            }
            let mut keys: Vec<u128> = mine.keys().copied().collect();
            keys.sort_unstable();
            let (low, high) = mine.values().fold((i32::MAX, i32::MIN), |(low, high), &(a, b)| (low.min(a), high.max(b)));
            let history = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::Operations, &[start, duration], lanes::Keys::These(&keys), low, high))?;
            let (start_at, duration_at) = (history.columns.binary_search(&start).ok(), history.columns.binary_search(&duration).ok());
            let starts: HashMap<u128, Option<i32>> = history
                .keys
                .iter()
                .map(|(key, steps)| {
                    let shown = steps.iter().rev().find_map(|step| step.cells.as_deref());
                    let value = |at: Option<usize>| match shown.zip(at).and_then(|(cells, at)| cells.get(at)) {
                        Some(points::Cell::Number(value)) => Some(*value),
                        _ => None,
                    };
                    let started = value(duration_at).filter(|days| *days != 0).and(value(start_at)).and_then(|hour| i32::try_from(hour).ok());
                    (*key, started)
                })
                .collect();
            for (key, (a, b)) in mine {
                out.insert(key as u64, (a, b, starts.get(&key).copied().flatten()));
            }
        }
        Ok(Self(out))
    }
}

impl Product for EspionageLog {
    const KEY: &'static str = "espionage-log";
    const CONTENT_DEPENDENT: bool = false;
    type Params = EspionageLogParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, facets, from: Start, to: End, cursor, pageSize, "facets[].countries[]", "facets[].values[]"];
    type Reply = LogPageReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EspionageLogParams::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let asked = LogPageParams {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            facets: params.facets.clone(),
            from: params.from,
            to: params.to,
            cursor: params.cursor.clone(),
            page_size: params.page_size,
            columns: None,
        };
        let tagged = params.facets.iter().any(|filter| !filter.countries.is_empty());
        let (page, rows, missing) = logs::page(&query, &asked, |paged, tag| (paged.row.clone(), paged.ordinal, paged.own, tag.cloned()))?;
        let recording = query.replay.recording();
        let mut links: HashMap<u32, Arc<RunLinks>> = HashMap::new();
        let mut tokens: BTreeMap<u32, BTreeSet<u128>> = BTreeMap::new();
        let mut last_hour: HashMap<u32, i32> = HashMap::new();
        for (row, ..) in &rows {
            if let Some(stretch) = query.timeline.at(row.hour) {
                if !links.contains_key(&stretch.run) {
                    links.insert(stretch.run, dashboard_base::queries::links(source, stretch.run)?);
                }
                if let Some(token) = number(&row.cells, DEFINITION) {
                    tokens.entry(stretch.run).or_default().insert(u128::from(token as u32));
                }
                let held = last_hour.entry(stretch.run).or_insert(row.hour);
                *held = (*held).max(row.hour);
            }
        }
        let mut keys: HashMap<(u32, u32), String> = HashMap::new();
        for (run, held) in tokens {
            let held: Vec<u128> = held.into_iter().collect();
            for (token, literal) in literals(recording, run, last_hour[&run], &held)? {
                keys.insert((run, token), literal);
            }
        }
        let asked_ids: Vec<(u64, i32)> = rows.iter().filter_map(|(row, ..)| Some((number(&row.cells, ID)? as u64, row.hour))).collect();
        let sightings = Sightings::read(source, &query, &asked_ids)?;
        let names = Names::new(&query.holders);
        let tag_of = |raw: u32, hour: i32| {
            let links = links.get(&query.timeline.at(hour)?.run)?;
            names.tag_at(links, raw, hour).map(str::to_owned)
        };
        let reply_rows = rows
            .into_iter()
            .map(|(row, ordinal, own, tag)| {
                let owner = (row.key >> 64) as u32;
                let target = number(&row.cells, TARGET).map(|target| target as u32);
                let run = query.timeline.at(row.hour).map(|stretch| stretch.run);
                let key = run.zip(number(&row.cells, DEFINITION)).and_then(|(run, token)| keys.get(&(run, token as u32)).cloned());
                let id = number(&row.cells, ID).map(|id| id as u64);
                let seen = id.and_then(|id| sightings.0.get(&id).copied());
                let description = description(&row.cells);
                let cells = vec![
                    text_cell(id.map(|id| id.to_string())),
                    number_cell(Some(i64::from(owner))),
                    text_cell(tag_of(owner, row.hour)),
                    number_cell(target.map(i64::from)),
                    text_cell(target.and_then(|target| tag_of(target, row.hour))),
                    text_cell(key),
                    number_cell(number(&row.cells, COMPLETION_HOUR)),
                    number_cell(number(&row.cells, TERMINAL)),
                    number_cell(number(&row.cells, BONUS)),
                    number_cell(number(&row.cells, DETECTED)),
                    text_cell(description.as_ref().ok().cloned()),
                    text_cell(description.err()),
                    number_cell(number(&row.cells, DESCRIPTION_REQUIRED)),
                    seen.and_then(|seen| seen.2).map_or(LogCell::Missing, |hour| LogCell::Number { value: hour.to_string() }),
                    seen.map_or(LogCell::Missing, |seen| LogCell::Number { value: seen.0.to_string() }),
                    seen.map_or(LogCell::Missing, |seen| LogCell::Number { value: seen.1.to_string() }),
                ];
                LogPageRow {
                    hour: row.hour,
                    key: row.key.to_string(),
                    ordinal: ordinal.to_string(),
                    country: tag.clone(),
                    own_ordinal: tagged.then(|| own.to_string()),
                    cells,
                }
            })
            .collect();
        Ok(LogPageReply {
            branch_id: params.branch_id.clone(),
            log: LOG.to_owned(),
            columns: COLUMNS.iter().map(|column| (*column).to_owned()).collect(),
            first_ordinal: page.first.to_string(),
            total: page.total().to_string(),
            cursor: page.cursor().map(|cursor| cursor.to_string()),
            rows: reply_rows,
            missing,
            warnings: Vec::new(),
        })
    }
}
