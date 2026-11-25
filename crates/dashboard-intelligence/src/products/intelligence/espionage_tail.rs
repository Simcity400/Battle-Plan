use super::{
    espionage::{LOG, current_phase, description, endings, preparation_ratio},
    operative_content,
};
use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceCount, OccurrenceReply, OccurrenceSort},
    },
    queries::country_ids::RunLinks,
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country as QueryCountry, Holders},
    log::{
        Cell,
        occurrence::{Ask, Chip, Direction, Index, Occurrence, Order, PlacedGap, Role, Selection, SortKey},
    },
    read::{self, Keys, Read},
    series::ranked::GroupSelection,
    timeline::Timeline,
};
use replay::campaign::Replay;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use replay_lanes::{lanes, points};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, OnceLock, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const COMP_ID: usize = 0;
const COMP_TARGET: usize = 1;
const COMP_DEFINITION: usize = 2;
const COMP_HOUR: usize = 3;
const COMP_BONUS: usize = 5;
const COMP_DETECTED: usize = 6;

const STATE_PREPARING: u8 = 0;
const STATE_RUNNING: u8 = 1;
const STATE_COMPLETED: u8 = 2;
const STATE_ENDED: u8 = 3;
const STATE_OUTCOME_NOT_RECORDED: u8 = 4;
const STATE_NOT_READ: u8 = 5;

const LIVE_STATES: [u8; 2] = [STATE_PREPARING, STATE_RUNNING];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EspionageStatus {
    Preparing,
    Running,
    Completed,
    Ended,
    OutcomeNotRecorded,
    NotRead,
}

impl EspionageStatus {
    fn from_code(code: u8) -> Self {
        match code {
            STATE_PREPARING => Self::Preparing,
            STATE_RUNNING => Self::Running,
            STATE_COMPLETED => Self::Completed,
            STATE_ENDED => Self::Ended,
            STATE_OUTCOME_NOT_RECORDED => Self::OutcomeNotRecorded,
            _ => Self::NotRead,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EspionageCountryRole {
    #[default]
    Owner,
    Target,
    Either,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EspionageRunningOrder {
    #[default]
    Start,
    Preparation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageCountry {
    pub tag: Option<String>,
    pub slot: Option<u32>,
    pub raw_id: Option<u32>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageTypeChip {
    pub token: String,
    pub first_hour: i32,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageCompletion {
    pub completion_hour: Option<i32>,
    pub bonus: Option<bool>,
    pub bonus_refused: Option<String>,
    pub detected: Option<bool>,
    pub detected_refused: Option<String>,
    pub description: Option<String>,
    pub description_unrecorded: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageLogRow {
    pub key: String,
    pub hour: i32,
    pub exact: bool,
    pub observed: i32,
    pub ordinal: String,
    pub live: bool,
    pub status: EspionageStatus,
    pub owner: EspionageCountry,
    pub target: EspionageCountry,
    pub operation_token: Option<String>,
    pub operation_name: Option<String>,
    pub operation_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub start_hour: Option<i32>,
    pub first_seen: Option<i32>,
    pub last_seen: Option<i32>,
    pub ended_hour: Option<i32>,
    pub duration_days: Option<i32>,
    pub preparation_ratio: Option<i32>,
    pub phase_count: Option<u32>,
    pub current_phase: Option<u32>,
    pub assigned_slots: Option<u32>,
    pub auto_repeat: Option<bool>,
    pub auto_commence: Option<bool>,
    pub outcome_missing: Option<String>,
    pub completion: Option<EspionageCompletion>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EspionageLogTailParams {
    pub branch_id: String,
    #[serde(default)]
    pub hour: Option<i32>,
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
    #[serde(default)]
    pub countries: Option<Vec<QueryCountry>>,
    #[serde(default)]
    pub group: Option<GroupSelection>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub sort: Option<OccurrenceSort>,
    #[serde(default)]
    pub offset: Option<String>,
    pub limit: u32,
    #[serde(default)]
    pub operation_types: Option<Vec<String>>,
    #[serde(default)]
    pub country_role: Option<EspionageCountryRole>,
    #[serde(default)]
    pub running_order: Option<EspionageRunningOrder>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EspionageLogTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub live: String,
    pub counts: Vec<OccurrenceCount>,
    pub operation_types: Vec<EspionageTypeChip>,
    pub rows: Vec<EspionageLogRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(EspionageLogTailReply);

impl OccurrenceReply for EspionageLogTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CensusStep {
    hour: i32,
    owner_raw: u32,
    owner_slot: Option<u32>,
    target_raw: u32,
    target_slot: Option<u32>,
    token: Option<String>,
    start_hour: Option<i32>,
    duration_days: i32,
    phase_count: u32,
    preparation_ratio: Option<i32>,
    assigned_slots: u32,
    auto_repeat: bool,
    auto_commence: bool,
    terminal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompletionRecord {
    observed_hour: i32,
    completion_hour: Option<i32>,
    owner_raw: u32,
    owner_slot: Option<u32>,
    target_raw: Option<u32>,
    target_slot: Option<u32>,
    token: Option<String>,
    bonus: Option<bool>,
    bonus_refused: Option<String>,
    detected: Option<bool>,
    detected_refused: Option<String>,
    description: Option<String>,
    description_unrecorded: Option<String>,
}

#[derive(Clone, Debug)]
struct OperationEpisode {
    id: u128,
    key: String,
    appear_hour: i32,
    first_seen: Option<i32>,
    first_exact: bool,
    last_seen: Option<i32>,
    ended_by: Option<i32>,
    ended_exact: bool,
    completions_recorded: bool,
    steps: Vec<CensusStep>,
    completion: Option<CompletionRecord>,
    owner_role: Role,
    target_role: Role,
    token: Option<String>,
    kind: u16,
    states: Box<[(i32, u8)]>,
    sort_start: SortKey,
    sort_prep: SortKey,
    changes: Box<[i32]>,
    text: Box<str>,
}

impl OperationEpisode {
    fn step_at(&self, hour: i32) -> Option<&CensusStep> {
        let at = self.steps.partition_point(|step| step.hour <= hour);
        at.checked_sub(1).map(|at| &self.steps[at])
    }

    fn completion_at(&self, hour: i32) -> Option<&CompletionRecord> {
        self.completion.as_ref().filter(|comp| comp.observed_hour <= hour)
    }

    fn display_date_at(&self, hour: i32, status: EspionageStatus) -> (i32, bool) {
        match status {
            EspionageStatus::Completed => {
                if let Some(comp) = self.completion_at(hour) {
                    match comp.completion_hour {
                        Some(exact_hour) => (exact_hour, true),
                        None => (comp.observed_hour, false),
                    }
                } else {
                    (self.appear_hour, self.first_exact)
                }
            }
            EspionageStatus::Ended | EspionageStatus::OutcomeNotRecorded => {
                (self.ended_by.unwrap_or(self.appear_hour), self.ended_exact)
            }
            EspionageStatus::Running => {
                if let Some(step) = self.step_at(hour)
                    && let Some(start) = step.start_hour
                {
                    (start, true)
                } else {
                    (self.appear_hour, self.first_exact)
                }
            }
            EspionageStatus::Preparing | EspionageStatus::NotRead => {
                (self.first_seen.unwrap_or(self.appear_hour), self.first_exact)
            }
        }
    }
}

struct Tail {
    episodes: Vec<OperationEpisode>,
    gaps: Vec<dashboard_query::series::MissingSpan>,
    by_owner: OnceLock<Index<usize>>,
    by_target: OnceLock<Index<usize>>,
    by_either: OnceLock<Index<usize>>,
    tokens: Vec<String>,
    chips: Vec<EspionageTypeChip>,
    type_meta: HashMap<String, (Option<String>, Option<String>, Option<String>, Option<String>)>,
    unavailable: Option<String>,
    warnings: Vec<String>,
}

impl Tail {
    fn index(&self, role: EspionageCountryRole) -> io::Result<&Index<usize>> {
        let slot = match role {
            EspionageCountryRole::Owner => &self.by_owner,
            EspionageCountryRole::Target => &self.by_target,
            EspionageCountryRole::Either => &self.by_either,
        };
        if let Some(held) = slot.get() {
            return Ok(held);
        }
        let occurrences: Vec<Occurrence<usize>> = self
            .episodes
            .iter()
            .enumerate()
            .map(|(at, ep)| {
                let roles: Box<[Role]> = match role {
                    EspionageCountryRole::Owner => Box::new([ep.owner_role]),
                    EspionageCountryRole::Target => Box::new([ep.target_role]),
                    EspionageCountryRole::Either => Box::new([ep.owner_role, ep.target_role]),
                };
                Occurrence {
                    id: ep.id,
                    date: ep.appear_hour,
                    observed: ep.appear_hour,
                    kind: ep.kind,
                    roles,
                    subjects: Box::new([]),
                    states: ep.states.clone(),
                    sort_keys: Box::new([ep.sort_start.clone(), ep.sort_prep.clone()]),
                    text: ep.text.clone(),
                    payload: at,
                }
            })
            .collect();
        let built = Index::new(occurrences, self.gaps.clone(), &LIVE_STATES)?;
        let _ = slot.set(built);
        Ok(slot.get().expect("just initialised"))
    }
}

#[derive(Clone)]
enum Made {
    Content(usize),
    Refused(String),
}

impl Made {
    fn of(content: &Result<Arc<operative_content::OperativeContent>, String>) -> Self {
        match content {
            Ok(content) => Self::Content(Arc::as_ptr(content) as usize),
            Err(why) => Self::Refused(why.clone()),
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Content(a), Self::Content(b)) => a == b,
            (Self::Refused(a), Self::Refused(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(Default)]
struct Kept(HashMap<u64, (Made, Arc<Tail>)>);

pub fn forget(source: &Source) {
    *source.state::<Kept>().lock().unwrap_or_else(PoisonError::into_inner) = Kept::default();
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

fn has_read_at(runs: &[(i32, i32)], hour: i32) -> bool {
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at).is_some_and(|&(a, b)| a <= hour && hour <= b)
}

fn flag_byte(name: &str, value: Option<i64>) -> (Option<bool>, Option<String>) {
    match value {
        None => (None, None),
        Some(0) => (Some(false), None),
        Some(1) => (Some(true), None),
        Some(other) => (None, Some(format!("Stored operation {name} refused: flag byte {other} is not 0 or 1"))),
    }
}

fn cell_num(cells: &[Cell], index: usize) -> Option<i64> {
    match cells.get(index) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn point_num(cells: &[points::Cell], at: Option<usize>) -> Option<i64> {
    match at.and_then(|at| cells.get(at)) {
        Some(points::Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn resolve_country(holders: &Holders, slot: Option<u32>, raw_id: Option<u32>, hour: i32, role_name: &str) -> EspionageCountry {
    match slot {
        Some(slot) => match holders.slot_tag(slot, hour) {
            Some(tag) => EspionageCountry { tag: Some(tag.to_owned()), slot: Some(slot), raw_id, missing: None },
            None => EspionageCountry {
                tag: None,
                slot: Some(slot),
                raw_id,
                missing: Some(format!("The operation's {role_name} tag is not recorded at this hour")),
            },
        },
        None => EspionageCountry {
            tag: None,
            slot: None,
            raw_id,
            missing: Some(format!("The operation's {role_name} is not recorded")),
        },
    }
}

fn census_step_from_cells(
    hour: i32,
    cells: &[points::Cell],
    cols: &OpCols,
    links: &RunLinks,
    token_names: &HashMap<u32, String>,
) -> Option<CensusStep> {
    let owner_raw = point_num(cells, cols.owner)? as u32;
    let target_raw = point_num(cells, cols.target)? as u32;
    let def_token = point_num(cells, cols.definition_token)? as u32;
    let prep_cur = point_num(cells, cols.prep_current).unwrap_or(0);
    let prep_req = point_num(cells, cols.prep_required).unwrap_or(0);
    let duration_days = point_num(cells, cols.duration_days).unwrap_or(0) as i32;
    let start_hour_raw = point_num(cells, cols.start_hour).unwrap_or(0) as i32;
    let phase_count = point_num(cells, cols.phase_count).unwrap_or(0) as u32;
    let assigned_slots = point_num(cells, cols.assigned_slots).unwrap_or(0) as u32;
    let auto_repeat = point_num(cells, cols.auto_repeat).unwrap_or(0) != 0;
    let auto_commence = point_num(cells, cols.flag_40).unwrap_or(0) != 0;
    let terminal = point_num(cells, cols.terminal).unwrap_or(0) != 0;
    let row = replay_data::rows::OperationRow {
        preparation_current: prep_cur,
        preparation_required: prep_req,
        ..replay_data::rows::OperationRow::default()
    };
    Some(CensusStep {
        hour,
        owner_raw,
        owner_slot: links.slot_at(owner_raw, hour),
        target_raw,
        target_slot: links.slot_at(target_raw, hour),
        token: token_names.get(&def_token).cloned(),
        start_hour: (duration_days != 0).then_some(start_hour_raw),
        duration_days,
        phase_count,
        preparation_ratio: preparation_ratio(&row),
        assigned_slots,
        auto_repeat,
        auto_commence,
        terminal,
    })
}

struct OpCols {
    owner: Option<usize>,
    target: Option<usize>,
    definition_token: Option<usize>,
    prep_current: Option<usize>,
    prep_required: Option<usize>,
    flag_40: Option<usize>,
    auto_repeat: Option<usize>,
    terminal: Option<usize>,
    start_hour: Option<usize>,
    duration_days: Option<usize>,
    phase_count: Option<usize>,
    assigned_slots: Option<usize>,
}

fn scan(
    source: &Source,
    query: &Query<'_>,
    content: &Result<Arc<operative_content::OperativeContent>, String>,
    cancelled: &AtomicBool,
) -> io::Result<Tail> {
    let recording = query.replay.recording();
    let declared = dashboard_declare::log(LOG).ok_or_else(|| dashboard_core::invalid(format!("No log is declared as {LOG}")))?;
    let reader = dashboard_query::log::Reader::new(query, declared)?;

    let f_owner = dashboard_declare::field(TableId::Operations, "owner")?;
    let f_target = dashboard_declare::field(TableId::Operations, "target")?;
    let f_def = dashboard_declare::field(TableId::Operations, "definition_token")?;
    let f_prep_cur = dashboard_declare::field(TableId::Operations, "preparation_current")?;
    let f_prep_req = dashboard_declare::field(TableId::Operations, "preparation_required")?;
    let f_flag_40 = dashboard_declare::field(TableId::Operations, "flag_40")?;
    let f_auto_repeat = dashboard_declare::field(TableId::Operations, "auto_repeat")?;
    let f_terminal = dashboard_declare::field(TableId::Operations, "terminal")?;
    let f_start_hour = dashboard_declare::field(TableId::Operations, "start_hour")?;
    let f_duration_days = dashboard_declare::field(TableId::Operations, "duration_days")?;
    let f_phase_count = dashboard_declare::field(TableId::Operations, "phase_count")?;
    let f_assigned_slots = dashboard_declare::field(TableId::Operations, "assigned_slots")?;
    let wanted_fields = [
        f_owner, f_target, f_def, f_prep_cur, f_prep_req, f_flag_40, f_auto_repeat, f_terminal, f_start_hour, f_duration_days, f_phase_count, f_assigned_slots,
    ];

    let mut raw_episodes: Vec<RawEpisode> = Vec::new();
    let mut first_seen_tokens: BTreeMap<String, i32> = BTreeMap::new();
    let mut warnings = Vec::new();
    if let Ok(content) = content {
        warnings.extend(content.warnings.iter().cloned());
    }

    let mut any_op_reads = false;
    let mut any_comp_reads = false;
    let mut carried: HashMap<u64, usize> = HashMap::new();

    for (stretch_index, stretch) in query.timeline.stretches.iter().enumerate() {
        let prior = std::mem::take(&mut carried);
        dashboard_text::metadata::check_cancel(cancelled)?;
        let links = dashboard_base::queries::links(source, stretch.run)?;
        let lives = dashboard_base::queries::lifetimes(source, stretch.run, TableId::Operations)?;
        let op_reads = dashboard_query::cost::storage(|| lanes::complete_reads(recording, stretch.run, TableId::Operations, stretch.first, stretch.last))?;
        let comp_reads = dashboard_query::cost::storage(|| lanes::complete_reads(recording, stretch.run, TableId::OperationCompletions, stretch.first, stretch.last))?;
        if !op_reads.is_empty() {
            any_op_reads = true;
        }
        if !comp_reads.is_empty() {
            any_comp_reads = true;
        }
        let ended_list = endings(&lives, (stretch.first, stretch.last), &op_reads, stretch.first, stretch.last);
        let ended_by_id: HashMap<u64, i32> = ended_list.iter().map(|ending| (ending.key as u64, ending.by)).collect();

        let comp_rows = reader.inserted(stretch.run, &[(stretch.first, stretch.last)])?;

        let mut life_by_id: HashMap<u64, (i32, i32)> = HashMap::new();
        for &(key, first, last) in lives.iter() {
            let (Ok(first), Ok(last)) = (i32::try_from(first), i32::try_from(last)) else { continue };
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if first <= last {
                life_by_id.insert(key as u64, (first, last));
            }
        }

        let mut op_keys: Vec<u128> = life_by_id.keys().map(|&id| u128::from(id)).collect();
        op_keys.sort_unstable();
        let history = if op_keys.is_empty() {
            None
        } else {
            Some(dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::Operations, &wanted_fields, lanes::Keys::These(&op_keys), stretch.first, stretch.last)
            })?)
        };

        let mut token_ids: BTreeSet<u128> = BTreeSet::new();
        if let Some(history) = &history {
            let def_at = history.columns.binary_search(&f_def).ok();
            for (_, steps) in &history.keys {
                for step in steps {
                    if let Some(cells) = step.cells.as_deref()
                        && let Some(token) = point_num(cells, def_at)
                    {
                        token_ids.insert(u128::from(token as u32));
                    }
                }
            }
        }
        for row in &comp_rows {
            if let Some(token) = cell_num(&row.cells, COMP_DEFINITION) {
                token_ids.insert(u128::from(token as u32));
            }
        }
        let token_vec: Vec<u128> = token_ids.into_iter().collect();
        let token_names = literals(recording, stretch.run, stretch.last, &token_vec)?;

        let mut comp_by_op_id: HashMap<u64, Vec<CompletionRecord>> = HashMap::new();
        let mut orphan_comps: Vec<(u128, CompletionRecord)> = Vec::new();
        for row in comp_rows {
            let owner_raw = (row.key >> 64) as u32;
            let target_raw = cell_num(&row.cells, COMP_TARGET).map(|v| v as u32);
            let token = cell_num(&row.cells, COMP_DEFINITION).and_then(|t| token_names.get(&(t as u32)).cloned());
            let completion_hour = cell_num(&row.cells, COMP_HOUR).and_then(|h| i32::try_from(h).ok());
            let (bonus, bonus_refused) = flag_byte("bonus", cell_num(&row.cells, COMP_BONUS));
            let (detected, detected_refused) = flag_byte("detection", cell_num(&row.cells, COMP_DETECTED));
            let desc = description(&row.cells);
            let rec = CompletionRecord {
                observed_hour: row.hour,
                completion_hour,
                owner_raw,
                owner_slot: links.slot_at(owner_raw, row.hour),
                target_raw,
                target_slot: target_raw.and_then(|raw| links.slot_at(raw, row.hour)),
                token,
                bonus,
                bonus_refused,
                detected,
                detected_refused,
                description: desc.as_ref().ok().cloned(),
                description_unrecorded: desc.err(),
            };
            if let Some(id) = cell_num(&row.cells, COMP_ID).map(|v| v as u64)
                && life_by_id.contains_key(&id)
            {
                comp_by_op_id.entry(id).or_default().push(rec);
            } else {
                orphan_comps.push((row.key, rec));
            }
        }

        let mut matched_ids: HashSet<u64> = HashSet::new();
        if let Some(history) = history {
            let cols = OpCols {
                owner: history.columns.binary_search(&f_owner).ok(),
                target: history.columns.binary_search(&f_target).ok(),
                definition_token: history.columns.binary_search(&f_def).ok(),
                prep_current: history.columns.binary_search(&f_prep_cur).ok(),
                prep_required: history.columns.binary_search(&f_prep_req).ok(),
                flag_40: history.columns.binary_search(&f_flag_40).ok(),
                auto_repeat: history.columns.binary_search(&f_auto_repeat).ok(),
                terminal: history.columns.binary_search(&f_terminal).ok(),
                start_hour: history.columns.binary_search(&f_start_hour).ok(),
                duration_days: history.columns.binary_search(&f_duration_days).ok(),
                phase_count: history.columns.binary_search(&f_phase_count).ok(),
                assigned_slots: history.columns.binary_search(&f_assigned_slots).ok(),
            };
            for (key, steps_raw) in history.keys {
                let op_id = key as u64;
                let Some(&(first_seen, last_seen)) = life_by_id.get(&op_id) else { continue };
                let mut steps: Vec<CensusStep> = Vec::new();
                for point in steps_raw {
                    let Some(cells) = point.cells.as_deref() else { continue };
                    if point.hour < stretch.first || point.hour > stretch.last {
                        continue;
                    }
                    let Some(step) = census_step_from_cells(point.hour, cells, &cols, &links, &token_names) else { continue };
                    let same = steps.last().is_some_and(|prev| {
                        prev.owner_slot == step.owner_slot
                            && prev.target_slot == step.target_slot
                            && prev.token == step.token
                            && prev.start_hour == step.start_hour
                            && prev.duration_days == step.duration_days
                            && prev.phase_count == step.phase_count
                            && prev.preparation_ratio == step.preparation_ratio
                            && prev.assigned_slots == step.assigned_slots
                            && prev.auto_repeat == step.auto_repeat
                            && prev.auto_commence == step.auto_commence
                            && prev.terminal == step.terminal
                    });
                    if !same {
                        steps.push(step);
                    }
                }
                if steps.is_empty() {
                    continue;
                }
                matched_ids.insert(op_id);
                let completion = comp_by_op_id.remove(&op_id).and_then(|mut list| {
                    list.sort_by_key(|rec| rec.observed_hour);
                    list.into_iter().next()
                });
                let ended_by = if completion.is_some() { None } else { ended_by_id.get(&op_id).copied() };
                let ended_exact = ended_by.is_some_and(|by| by == last_seen.saturating_add(1));
                let first_exact = first_seen > stretch.first && has_read_at(&op_reads, first_seen - 1);
                let completions_recorded = ended_by.is_some_and(|by| covered(&comp_reads, first_seen, by));
                let token = steps.iter().find_map(|s| s.token.clone()).or_else(|| completion.as_ref().and_then(|c| c.token.clone()));
                if let Some(tok) = &token {
                    let entry = first_seen_tokens.entry(tok.clone()).or_insert(first_seen);
                    *entry = (*entry).min(first_seen);
                }
                let reads_whole = covered(&comp_reads, first_seen, last_seen);
                let open = completion.is_none() && ended_by.is_none() && last_seen == stretch.last;
                if let Some(&at) = prior.get(&op_id)
                    && first_seen == stretch.first
                    && raw_episodes[at].continues(&steps[0])
                {
                    let held = &mut raw_episodes[at];
                    held.earlier_reads_whole &= reads_whole;
                    held.steps.extend(steps);
                    held.last_seen = Some(last_seen);
                    held.ended_by = ended_by;
                    held.ended_exact = ended_exact;
                    held.completions_recorded = completions_recorded && held.earlier_reads_whole;
                    held.completion = completion;
                    held.token = held.token.take().or(token);
                    if open {
                        carried.insert(op_id, at);
                    }
                    continue;
                }
                if open {
                    carried.insert(op_id, raw_episodes.len());
                }
                let unique_id = (u128::from(stretch_index as u32) << 96) | (1u128 << 64) | u128::from(op_id);
                let key_str = if stretch_index == 0 { format!("op:{op_id}") } else { format!("op:{op_id}#{stretch_index}") };
                raw_episodes.push(RawEpisode {
                    earlier_reads_whole: reads_whole,
                    id: unique_id,
                    key: key_str,
                    appear_hour: first_seen,
                    first_seen: Some(first_seen),
                    first_exact,
                    last_seen: Some(last_seen),
                    ended_by,
                    ended_exact,
                    completions_recorded,
                    steps,
                    completion,
                    token,
                });
            }
        }

        for (op_id, list) in comp_by_op_id {
            if !matched_ids.contains(&op_id) {
                for (nth, rec) in list.into_iter().enumerate() {
                    orphan_comps.push(((u128::from(op_id) << 32) | u128::from(nth as u32), rec));
                }
            }
        }
        for (idx, (comp_key, rec)) in orphan_comps.into_iter().enumerate() {
            if let Some(tok) = &rec.token {
                let entry = first_seen_tokens.entry(tok.clone()).or_insert(rec.observed_hour);
                *entry = (*entry).min(rec.observed_hour);
            }
            let unique_id = (u128::from(stretch_index as u32) << 96) | (2u128 << 64) | (idx as u128);
            raw_episodes.push(RawEpisode {
                id: unique_id,
                key: format!("comp:{comp_key}:{stretch_index}:{idx}"),
                appear_hour: rec.observed_hour,
                first_seen: None,
                first_exact: rec.completion_hour.is_some(),
                last_seen: None,
                ended_by: None,
                ended_exact: false,
                completions_recorded: true,
                steps: Vec::new(),
                token: rec.token.clone(),
                completion: Some(rec),
                earlier_reads_whole: true,
            });
        }
    }

    let tokens: Vec<String> = first_seen_tokens.keys().cloned().collect();
    let mut type_meta: HashMap<String, (Option<String>, Option<String>, Option<String>, Option<String>)> = HashMap::new();
    let mut chips = Vec::with_capacity(tokens.len());
    for (token, &first_hour) in &first_seen_tokens {
        let (name, name_missing, icon, icon_missing) = match content {
            Ok(content) => {
                let (name_res, icon_res) = content.operation_type(token);
                let icon_drawn = icon_res.and_then(|art| content.draw(&art));
                (name_res.as_ref().ok().cloned(), name_res.err(), icon_drawn.as_ref().ok().cloned(), icon_drawn.err())
            }
            Err(why) => (None, Some(why.clone()), None, Some(why.clone())),
        };
        chips.push(EspionageTypeChip {
            token: token.clone(),
            first_hour,
            name: name.clone(),
            icon: icon.clone(),
            icon_missing: icon_missing.clone(),
        });
        type_meta.insert(token.clone(), (name, name_missing, icon, icon_missing));
    }

    let mut episodes = Vec::with_capacity(raw_episodes.len());
    for raw in raw_episodes {
        let kind = raw.token.as_ref().and_then(|t| tokens.binary_search(t).ok()).map_or(0, |at| at as u16 + 1);
        let (owner_role, target_role) = if let Some(first_step) = raw.steps.first() {
            (Role { slot: first_step.owner_slot, at: first_step.hour }, Role { slot: first_step.target_slot, at: first_step.hour })
        } else if let Some(comp) = &raw.completion {
            (Role { slot: comp.owner_slot, at: comp.observed_hour }, Role { slot: comp.target_slot, at: comp.observed_hour })
        } else {
            (Role { slot: None, at: raw.appear_hour }, Role { slot: None, at: raw.appear_hour })
        };

        let mut changes: Vec<i32> = Vec::new();
        changes.push(raw.appear_hour);
        for (at, step) in raw.steps.iter().enumerate() {
            changes.push(step.hour);
            let through = raw.steps.get(at + 1).map_or(raw.last_seen.unwrap_or(step.hour), |next| next.hour - 1);
            changes.extend(phase_hours(step, through));
        }
        if let Some(comp) = &raw.completion {
            changes.push(comp.observed_hour);
        }
        if let Some(by) = raw.ended_by {
            changes.push(by);
        } else if raw.completion.is_none()
            && let Some(last) = raw.last_seen
            && last < query.timeline.last
        {
            changes.push(last.saturating_add(1));
        }
        changes.sort_unstable();
        changes.dedup();

        let mut states: Vec<(i32, u8)> = Vec::new();
        let mut start_keys: Vec<(i32, Option<i64>)> = Vec::new();
        let mut prep_keys: Vec<(i32, Option<i64>)> = Vec::new();

        for &hour in &changes {
            let (code, start_val, prep_val) = if let Some(comp) = raw.completion.as_ref().filter(|c| c.observed_hour <= hour) {
                let d = i64::from(comp.completion_hour.unwrap_or(comp.observed_hour));
                (STATE_COMPLETED, Some(d), Some(d))
            } else if let Some(by) = raw.ended_by.filter(|&by| by <= hour) {
                let code = if raw.completions_recorded { STATE_ENDED } else { STATE_OUTCOME_NOT_RECORDED };
                (code, Some(i64::from(by)), Some(i64::from(by)))
            } else if let Some(last) = raw.last_seen.filter(|&last| hour > last) {
                (STATE_NOT_READ, Some(i64::from(last)), Some(-1))
            } else {
                let step = raw.steps.partition_point(|s| s.hour <= hour).checked_sub(1).map(|i| &raw.steps[i]);
                match step {
                    Some(step) if step.terminal => (STATE_OUTCOME_NOT_RECORDED, Some(i64::from(step.hour)), Some(i64::from(step.hour))),
                    Some(step) if step.duration_days != 0 => {
                        let start = step.start_hour.unwrap_or(raw.appear_hour);
                        let prep = i64::from(step.preparation_ratio.unwrap_or(-1));
                        (STATE_RUNNING, Some(i64::from(start)), Some(prep))
                    }
                    Some(step) => {
                        let prep = i64::from(step.preparation_ratio.unwrap_or(-1));
                        (STATE_PREPARING, Some(i64::from(raw.appear_hour)), Some(prep))
                    }
                    None => (STATE_NOT_READ, Some(i64::from(raw.appear_hour)), Some(-1)),
                }
            };
            if states.last().is_none_or(|&(_, held)| held != code) {
                states.push((hour, code));
            }
            if start_keys.last().is_none_or(|&(_, held)| held != start_val) {
                start_keys.push((hour, start_val));
            }
            if prep_keys.last().is_none_or(|&(_, held)| held != prep_val) {
                prep_keys.push((hour, prep_val));
            }
        }

        let mut text_parts: Vec<String> = Vec::new();
        if let Some(tag) = owner_role.slot.and_then(|s| query.holders.slot_tag(s, owner_role.at)) {
            text_parts.push(tag.to_owned());
        }
        if let Some(tag) = target_role.slot.and_then(|s| query.holders.slot_tag(s, target_role.at)) {
            text_parts.push(tag.to_owned());
        }
        if let Some(tok) = &raw.token {
            text_parts.push(tok.clone());
            if let Some((Some(name), ..)) = type_meta.get(tok) {
                text_parts.push(name.clone());
            }
        }

        episodes.push(OperationEpisode {
            id: raw.id,
            key: raw.key,
            appear_hour: raw.appear_hour,
            first_seen: raw.first_seen,
            first_exact: raw.first_exact,
            last_seen: raw.last_seen,
            ended_by: raw.ended_by,
            ended_exact: raw.ended_exact,
            completions_recorded: raw.completions_recorded,
            steps: raw.steps,
            completion: raw.completion,
            owner_role,
            target_role,
            token: raw.token,
            kind,
            states: states.into_boxed_slice(),
            sort_start: SortKey::dated_numbers(start_keys),
            sort_prep: SortKey::dated_numbers(prep_keys),
            changes: changes.into_boxed_slice(),
            text: text_parts.join(" ").into_boxed_str(),
        });
    }

    let gaps = missing_completions(query, query.timeline.first, query.timeline.last)?;
    let unavailable = (!any_op_reads && !any_comp_reads && episodes.is_empty())
        .then(|| "Espionage operations and operation completions were not recorded in this branch.".to_owned());

    Ok(Tail {
        episodes,
        gaps,
        by_owner: OnceLock::new(),
        by_target: OnceLock::new(),
        by_either: OnceLock::new(),
        tokens,
        chips,
        type_meta,
        unavailable,
        warnings,
    })
}

fn phase_hours(step: &CensusStep, through: i32) -> impl Iterator<Item = i32> + '_ {
    let running = step.duration_days > 0 && !step.terminal;
    let start_day = step.start_hour.map(|start| start.div_euclid(24));
    (1..=step.phase_count.min(i32::MAX as u32) as i32)
        .filter(move |_| running)
        .filter_map(move |phase| {
            let day = (i64::from(phase) * i64::from(step.duration_days) + i64::from(step.phase_count) - 1) / i64::from(step.phase_count);
            i32::try_from((i64::from(start_day?) + day) * 24).ok()
        })
        .chain(start_day.filter(|_| running).and_then(|day| day.checked_add(step.duration_days)?.checked_mul(24)))
        .filter(move |&hour| hour > step.hour && hour <= through)
}

struct RawEpisode {
    id: u128,
    key: String,
    appear_hour: i32,
    first_seen: Option<i32>,
    first_exact: bool,
    last_seen: Option<i32>,
    ended_by: Option<i32>,
    ended_exact: bool,
    completions_recorded: bool,
    steps: Vec<CensusStep>,
    completion: Option<CompletionRecord>,
    token: Option<String>,
    earlier_reads_whole: bool,
}

impl RawEpisode {
    fn continues(&self, next: &CensusStep) -> bool {
        self.steps.last().is_some_and(|last| (last.owner_slot, last.target_slot, &last.token) == (next.owner_slot, next.target_slot, &next.token))
    }
}

fn tail(source: &Source, query: &Query<'_>, branch: u64, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<Tail>> {
    let content = operative_content::content(source, load);
    let made = Made::of(&content);
    let kept = source.state::<Kept>();
    if let Some((held, found)) = kept.lock().unwrap_or_else(PoisonError::into_inner).0.get(&branch)
        && held.same(&made)
    {
        return Ok(Arc::clone(found));
    }
    let built = Arc::new(scan(source, query, &content, cancelled)?);
    kept.lock().unwrap_or_else(PoisonError::into_inner).0.insert(branch, (made, Arc::clone(&built)));
    Ok(built)
}

struct Prepared {
    tail: Arc<Tail>,
    selection: Option<Selection>,
}

fn prepare(
    source: &Source,
    query: &Query<'_>,
    branch: u64,
    load: &LoadContext,
    params: &EspionageLogTailParams,
    cancelled: &AtomicBool,
) -> io::Result<Prepared> {
    let selection = dashboard_world::products::group_members::dated(
        source,
        query,
        load,
        params.countries.as_deref(),
        params.group.as_ref(),
        cancelled,
    )?;
    Ok(Prepared { tail: tail(source, query, branch, load, cancelled)?, selection })
}

fn build_row(tail: &Tail, holders: &Holders, ep: &OperationEpisode, ordinal: u64, live: bool, status: EspionageStatus, hour: i32) -> EspionageLogRow {
    let step = ep.step_at(hour);
    let comp = ep.completion_at(hour);
    let (date, exact) = ep.display_date_at(hour, status);

    let (owner_slot, owner_raw, target_slot, target_raw, identity_hour) = match (comp, step) {
        (Some(comp), _) => (comp.owner_slot, Some(comp.owner_raw), comp.target_slot, comp.target_raw, comp.observed_hour),
        (None, Some(step)) => (step.owner_slot, Some(step.owner_raw), step.target_slot, Some(step.target_raw), hour.min(ep.last_seen.unwrap_or(hour))),
        (None, None) => (ep.owner_role.slot, None, ep.target_role.slot, None, ep.appear_hour),
    };
    let owner = resolve_country(holders, owner_slot, owner_raw, identity_hour, "owner");
    let target = resolve_country(holders, target_slot, target_raw, identity_hour, "target");

    let token = step.and_then(|s| s.token.clone()).or_else(|| comp.and_then(|c| c.token.clone())).or_else(|| ep.token.clone());
    let (operation_name, operation_missing, icon, icon_missing) = match &token {
        Some(tok) => tail.type_meta.get(tok).cloned().unwrap_or((None, None, None, None)),
        None => (None, Some("The operation definition token was not recorded".to_owned()), None, Some("The operation definition token was not recorded".to_owned())),
    };

    let last_running_step = ep.steps.iter().rev().find(|s| s.hour <= hour && s.duration_days != 0);
    let start_hour = step.and_then(|s| s.start_hour).or_else(|| last_running_step.and_then(|s| s.start_hour));
    let duration_days = step
        .map(|s| s.duration_days)
        .filter(|&d| d > 0)
        .or_else(|| last_running_step.map(|s| s.duration_days).filter(|&d| d > 0));

    let current_phase_val = step.and_then(|s| {
        if s.duration_days == 0 || s.terminal {
            return None;
        }
        let dummy = replay_data::rows::OperationRow {
            start_hour: s.start_hour.unwrap_or(0),
            duration_days: s.duration_days,
            phase_count: s.phase_count,
            terminal: u8::from(s.terminal),
            ..replay_data::rows::OperationRow::default()
        };
        current_phase(&dummy, hour)
    });

    let ended_hour = match status {
        EspionageStatus::Completed => comp.and_then(|c| c.completion_hour.or(Some(c.observed_hour))),
        EspionageStatus::Ended | EspionageStatus::OutcomeNotRecorded => ep.ended_by,
        _ => None,
    };

    let outcome_missing = match status {
        EspionageStatus::OutcomeNotRecorded => Some(
            if ep.ended_by.is_some() && !ep.completions_recorded {
                "Completed operations were not recorded whole during this operation".to_owned()
            } else {
                "Operation outcome was not recorded".to_owned()
            },
        ),
        EspionageStatus::NotRead => Some("Operations were not recorded at this hour".to_owned()),
        _ => None,
    };

    let completion = comp.map(|c| EspionageCompletion {
        completion_hour: c.completion_hour,
        bonus: c.bonus,
        bonus_refused: c.bonus_refused.clone(),
        detected: c.detected,
        detected_refused: c.detected_refused.clone(),
        description: c.description.clone(),
        description_unrecorded: c.description_unrecorded.clone(),
    });

    EspionageLogRow {
        key: ep.key.clone(),
        hour: date,
        exact,
        observed: ep.appear_hour,
        ordinal: ordinal.to_string(),
        live,
        status,
        owner,
        target,
        operation_token: token,
        operation_name,
        operation_missing,
        icon,
        icon_missing,
        start_hour,
        first_seen: ep.first_seen.filter(|&h| h <= hour),
        last_seen: ep.last_seen.filter(|&h| h < hour),
        ended_hour,
        duration_days,
        preparation_ratio: step.and_then(|s| s.preparation_ratio),
        phase_count: step.map(|s| s.phase_count),
        current_phase: current_phase_val,
        assigned_slots: step.map(|s| s.assigned_slots),
        auto_repeat: step.map(|s| s.auto_repeat),
        auto_commence: step.map(|s| s.auto_commence),
        outcome_missing,
        completion,
    }
}

fn missing_completions(query: &Query<'_>, from: i32, to: i32) -> io::Result<Vec<dashboard_query::series::MissingSpan>> {
    use dashboard_query::series::MissingSpan;
    let mut out: Vec<MissingSpan> = query
        .timeline
        .unheld_within(from, to)
        .into_iter()
        .map(|gap| MissingSpan { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: gap.reason })
        .collect();
    let recording = query.replay.recording();
    for stretch in query.timeline.within(from, to) {
        let low = stretch.first.max(from);
        let high = stretch.last.min(to);
        let runs = read::statuses(recording, stretch.run, TableId::OperationCompletions, low, high)?;
        let reasons = read::status_reasons(recording, stretch.run, TableId::OperationCompletions, &runs)?;
        for run in runs {
            let reason = match run.status {
                Status::Complete => continue,
                Status::Unavailable { reason } => reasons.get(&reason).cloned().unwrap_or_else(|| "Completed operations were not recorded".to_owned()),
                Status::NotRecorded => "Completed operations were not recorded".to_owned(),
                Status::Truncated { .. } => "Completed operations were truncated".to_owned(),
            };
            out.push(MissingSpan { first_hour: run.first, last_hour: run.last, reason });
        }
    }
    out.sort_unstable_by_key(|gap| (gap.first_hour, gap.last_hour));
    Ok(out)
}

fn reply_at(
    prepared: &Prepared,
    holders: &Holders,
    params: &EspionageLogTailParams,
    hour: i32,
) -> io::Result<EspionageLogTailReply> {
    let role = params.country_role.unwrap_or_default();
    let index = prepared.tail.index(role)?;
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let kinds: Option<Vec<u16>> = params.operation_types.as_ref().map(|chosen| {
        chosen.iter().filter_map(|token| prepared.tail.tokens.binary_search(token).ok().map(|at| at as u16 + 1)).collect()
    });
    let order = match params.sort.as_ref() {
        Some(sort) => match sort.column.as_str() {
            "preparation" => Order::Key(1, sort.direction),
            "start" => Order::Key(0, sort.direction),
            _ => Order::Date(sort.direction),
        },
        None => match params.running_order.unwrap_or_default() {
            EspionageRunningOrder::Start => Order::Key(0, Direction::Descending),
            EspionageRunningOrder::Preparation => Order::Key(1, Direction::Descending),
        },
    };
    let chips_spec = [Chip { kinds: None, states: Some(LIVE_STATES.to_vec()) }];
    let answer = index.ask(
        holders,
        &Ask {
            hour,
            from: params.from,
            to: params.to,
            selection: prepared.selection.as_ref(),
            subject: None,
            kinds: kinds.as_deref(),
            states: None,
            search: params.search.as_deref(),
            order,
            live_first: true,
            offset,
            limit,
            chips: &chips_spec,
        },
    )?;

    let mut next_change = answer.next_change;
    let mut rows = Vec::with_capacity(answer.rows.len());
    for shown in &answer.rows {
        let ep_idx = index.get(shown.at).payload;
        let ep = &prepared.tail.episodes[ep_idx];
        let status = shown.state.map_or(EspionageStatus::NotRead, EspionageStatus::from_code);
        rows.push(build_row(&prepared.tail, holders, ep, shown.ordinal, shown.live, status, hour));
        let next_ep = ep.changes.partition_point(|&h| h <= hour);
        if let Some(&h) = ep.changes.get(next_ep) {
            next_change = Some(next_change.map_or(h, |held| held.min(h)));
        }
    }

    let mut operation_types = Vec::new();
    for chip in &prepared.tail.chips {
        if chip.first_hour <= hour {
            operation_types.push(chip.clone());
        } else {
            next_change = Some(next_change.map_or(chip.first_hour, |held| held.min(chip.first_hour)));
        }
    }

    let counts = occurrences::counts(&answer, &["active"]);
    let mut warnings = prepared.tail.warnings.clone();
    if answer.unattributed > 0 {
        warnings.push(format!("{} espionage operations had an unrecorded country and could not be matched to the selected countries.", answer.unattributed));
    }

    let reply = EspionageLogTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        live: answer.live.to_string(),
        counts,
        operation_types,
        rows,
        missing: answer.gaps,
        next_change,
        unavailable: prepared.tail.unavailable.clone(),
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct EspionageLogTail;

impl Product for EspionageLogTail {
    const KEY: &'static str = "espionage-log-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = EspionageLogTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![
        branchId,
        hour: Read,
        from: Start,
        to: End,
        countries,
        "countries[]": Undated,
        group,
        search,
        sort,
        offset,
        limit,
        operationTypes,
        countryRole,
        runningOrder
    ];
    type Reply = EspionageLogTailReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            EspionageLogTailParams::decl(),
            EspionageStatus::decl(),
            EspionageCountryRole::decl(),
            EspionageRunningOrder::decl(),
            EspionageCountry::decl(),
            EspionageTypeChip::decl(),
            EspionageCompletion::decl(),
            EspionageLogRow::decl(),
            EspionageLogTailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)?;
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch: u64 = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, branch, context, params, cancelled)?;
        reply_at(&prepared, &query.holders, params, hour)
    }
}

struct Owned {
    replay: Arc<Replay>,
    timeline: Arc<Timeline>,
    holders: Arc<Holders>,
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid espionage-log-tail panel")?.remove("panel");
    let params: EspionageLogTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid espionage-log-tail panel: {error}"))?;
    EspionageLogTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, branch, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let owned = Owned { replay: source.replay(), timeline: Arc::clone(&query.timeline), holders: Arc::clone(&query.holders) };
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = Query { replay: &owned.replay, timeline: Arc::clone(&owned.timeline), holders: Arc::clone(&owned.holders) };
        let hour = occurrences::cutoff(&query, Some(hour))?;
        reply_at(&prepared, &query.holders, &params, hour)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::{
        Source, SourceKey,
        test_fixtures::{CAMPAIGN, records, source as base_frame},
    };
    use dashboard_core::config::LoadContext;
    use game_model::calendar::HOUR_EPOCH;
    use replay_data::rows::{CountryLinkRow, CountryRow, LexicalTokenRow, OperationCompletionRow, OperationRow, TableRows};
    use replay_frame::frame::TableView;

    fn make_country(id: u32, slot: u32, tag: &[u8]) -> CountryRow {
        CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..CountryRow::default() }
    }

    fn make_link(id: u32, slot: u32) -> CountryLinkRow {
        CountryLinkRow { id, canonical_slot: slot }
    }

    fn make_source() -> Source {
        let h10 = HOUR_EPOCH + 100;
        let h20 = h10 + 10;
        let h30 = h10 + 20;
        let h40 = h10 + 30;
        let h50 = h10 + 40;
        let countries = vec![make_country(1, 1, b"GER"), make_country(2, 2, b"SOV"), make_country(3, 3, b"ENG")];
        let links = vec![make_link(1, 1), make_link(2, 2), make_link(3, 3)];
        let tokens = vec![
            LexicalTokenRow { token: 10, literal: b"operation_infiltrate_army".to_vec() },
            LexicalTokenRow { token: 20, literal: b"operation_capture_cipher".to_vec() },
        ];
        let mut frames = Vec::new();
        for hour in h10..=h50 {
            let mut frame = base_frame(1, hour, Some(1));
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries.clone()));
            *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(tokens.clone()));
            let mut ops = Vec::new();
            if hour >= h10 && hour < h40 {
                let running = hour >= h20;
                ops.push(OperationRow {
                    id: 100,
                    owner: 1,
                    target: 2,
                    definition_token: 10,
                    preparation_current: if running { 100_000 } else { 50_000 },
                    preparation_required: 100_000,
                    start_hour: if running { h20 } else { 0 },
                    duration_days: if running { 30 } else { 0 },
                    phase_count: 3,
                    assigned_slots: 2,
                    ..OperationRow::default()
                });
            }
            if hour >= h30 && hour < h50 {
                ops.push(OperationRow {
                    id: 200,
                    owner: 3,
                    target: 1,
                    definition_token: 20,
                    preparation_current: 80_000,
                    preparation_required: 100_000,
                    start_hour: 0,
                    duration_days: 0,
                    phase_count: 2,
                    assigned_slots: 1,
                    ..OperationRow::default()
                });
            }
            frame.metrics.table_ns[TableId::Operations.index()] = 1;
            *frame.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(ops));
            let comps = if hour == h40 {
                vec![OperationCompletionRow {
                    owner: 1,
                    sequence: 1,
                    id: 100,
                    target: 2,
                    definition_token: 10,
                    completion_hour: h40,
                    terminal: 1,
                    bonus: 1,
                    detected: 0,
                    description_error: 0,
                    description_required: 16,
                    effect_description: b"Infiltrated army".to_vec(),
                }]
            } else {
                vec![]
            };
            frame.metrics.table_ns[TableId::OperationCompletions.index()] = 1;
            *frame.tables[TableId::OperationCompletions.index()] = TableView::Complete(TableRows::OperationCompletions(comps));
            frames.push(frame);
        }
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .expect("replay")
            .into_timelines(&Default::default())
            .expect("timelines");
        Source::from_replay(
            SourceKey { source_id: "espionage-tail".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .expect("source")
    }

    fn op_frame(branch: u64, hour: i32, ops: Vec<OperationRow>) -> replay_frame::frame::DecodedFrame {
        let mut frame = base_frame(branch, hour, Some(1));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![make_country(1, 1, b"GER"), make_country(2, 2, b"SOV")]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![make_link(1, 1), make_link(2, 2)]));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 10, literal: b"operation_infiltrate_army".to_vec() }]));
        frame.metrics.table_ns[TableId::Operations.index()] = 1;
        *frame.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(ops));
        frame.metrics.table_ns[TableId::OperationCompletions.index()] = 1;
        *frame.tables[TableId::OperationCompletions.index()] = TableView::Complete(TableRows::OperationCompletions(Vec::new()));
        frame
    }

    #[test]
    fn an_operation_across_a_load_is_one_row_and_one_the_save_lacks_is_no_longer_read() {
        let h10 = HOUR_EPOCH + 100;
        let op = |id: u64| OperationRow { id, owner: 1, target: 2, definition_token: 10, preparation_current: 50_000, preparation_required: 100_000, phase_count: 2, assigned_slots: 1, ..OperationRow::default() };
        let before = records(1, (h10..=h10 + 10).map(|hour| op_frame(1, hour, vec![op(100), op(300)])).collect());
        let mut after = records(2, (h10 + 11..=h10 + 20).map(|hour| op_frame(2, hour, if hour <= h10 + 15 { vec![op(100)] } else { vec![] })).collect());
        if let replay_capture_spool::spool::Record::BranchStart(start) = &mut after[1] {
            *start = replay_frame::branches::BranchStart {
                id: 2,
                session: 2,
                loaded_hour: Some(h10 + 10),
                first_hour: start.first_hour,
                parent: replay_frame::branches::Parentage::Probable { branch: 1, hour: h10 + 10 },
                reason: replay_frame::branches::BranchReason::LoadSignal,
            };
        }
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[before, after]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(SourceKey { source_id: "espionage-load".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let load = LoadContext { content: None };
        let ask = |hour: i32| {
            EspionageLogTail::load(
                &source,
                &EspionageLogTailParams { branch_id: "2".into(), hour: Some(hour), from: None, to: None, countries: None, group: None, search: None, sort: None, offset: None, limit: 50, operation_types: None, country_role: None, running_order: None },
                &load,
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        let mid = ask(h10 + 13);
        let shown: Vec<(&str, EspionageStatus)> = mid.rows.iter().map(|row| (row.key.as_str(), row.status)).collect();
        assert_eq!(shown, [("op:100", EspionageStatus::Preparing), ("op:300", EspionageStatus::NotRead)], "one row per operation across the load");
        assert_eq!((mid.live.as_str(), mid.rows[1].last_seen), ("1", Some(h10 + 10)));
        let late = ask(h10 + 18);
        assert_eq!(late.rows.iter().map(|row| (row.key.as_str(), row.status)).collect::<Vec<_>>(), [("op:100", EspionageStatus::Ended), ("op:300", EspionageStatus::NotRead)]);
        assert_eq!(late.rows[0].ended_hour, Some(h10 + 16));
        let panel = dated_panel(&source, &load, &serde_json::json!({ "panel": "espionage-log-tail", "branchId": "2", "limit": 50 })).unwrap();
        let exact = panel.exact().unwrap();
        for (hour, seed) in [(h10 + 5, true), (h10 + 10, false), (h10 + 11, false), (h10 + 16, false), (h10 + 12, true)] {
            assert_eq!(*(exact.advance)(hour, seed, &AtomicBool::new(false), None).value, (exact.cold)(hour, &AtomicBool::new(false)), "stepped equals cold at {hour}");
        }
    }

    #[test]
    fn a_running_operation_changes_at_each_phase_and_at_its_last_day() {
        let start = HOUR_EPOCH + 24 * 10 + 5;
        let step = CensusStep {
            hour: start,
            owner_raw: 1,
            owner_slot: Some(1),
            target_raw: 2,
            target_slot: Some(2),
            token: None,
            start_hour: Some(start),
            duration_days: 30,
            phase_count: 3,
            preparation_ratio: None,
            assigned_slots: 1,
            auto_repeat: false,
            auto_commence: false,
            terminal: false,
        };
        let day = |days: i32| HOUR_EPOCH + 24 * (10 + days);
        let row = |hour: i32| {
            let held = replay_data::rows::OperationRow { start_hour: start, duration_days: 30, phase_count: 3, ..OperationRow::default() };
            current_phase(&held, hour)
        };
        let mut hours: Vec<i32> = phase_hours(&step, day(40)).collect();
        hours.sort_unstable();
        hours.dedup();
        assert_eq!(hours, [day(10), day(20), day(30)]);
        for &hour in &hours {
            assert_ne!(row(hour - 1), row(hour), "the shown phase moves at {hour}");
        }
        assert_eq!(phase_hours(&step, day(15)).collect::<Vec<_>>(), [day(10)], "no change is promised past the step's last read");
        assert_eq!(phase_hours(&CensusStep { terminal: true, ..step.clone() }, day(40)).count(), 0);
    }

    #[test]
    fn espionage_tail_tracks_lifecycle_roles_order_and_withholds_future_chips() {
        let h10 = HOUR_EPOCH + 100;
        let h20 = h10 + 10;
        let h30 = h10 + 20;
        let h40 = h10 + 30;
        let h50 = h10 + 40;
        let source = make_source();
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);

        let panel = dated_panel(&source, &load, &serde_json::json!({ "panel": "espionage-log-tail", "branchId": "1", "limit": 50 })).unwrap();
        let exact = panel.exact().unwrap();
        for (hour, seed) in [(h10, true), (h20, false), (h30, false), (h40, false), (h50, false), (h20, true)] {
            let stepped = (exact.advance)(hour, seed, &cancelled, None);
            let cold = (exact.cold)(hour, &cancelled);
            assert_eq!(*stepped.value, cold, "stepped equals cold at hour {hour}");
        }

        let ask = |hour: i32, role: Option<EspionageCountryRole>, countries: Option<Vec<QueryCountry>>, order: Option<EspionageRunningOrder>| {
            EspionageLogTail::load(
                &source,
                &EspionageLogTailParams {
                    branch_id: "1".into(),
                    hour: Some(hour),
                    from: None,
                    to: None,
                    countries,
                    group: None,
                    search: None,
                    sort: None,
                    offset: None,
                    limit: 50,
                    operation_types: None,
                    country_role: role,
                    running_order: order,
                },
                &load,
                &cancelled,
            )
            .expect("load espionage-log-tail")
        };

        let at10 = ask(h10, None, None, None);
        assert_eq!(at10.live, "1");
        assert_eq!(at10.rows.len(), 1);
        assert_eq!(at10.rows[0].status, EspionageStatus::Preparing);
        assert_eq!(at10.rows[0].preparation_ratio, Some(50_000));
        assert_eq!(at10.operation_types.iter().map(|c| c.token.as_str()).collect::<Vec<_>>(), vec!["operation_infiltrate_army"]);
        assert_eq!(at10.next_change, Some(h20));

        let at20 = ask(h20, None, None, None);
        assert_eq!(at20.rows[0].last_seen, None, "a live operation read at the hour asked has no last sighting");
        assert_eq!(ask(h20 + 1, None, None, None).rows, ask(h20 + 2, None, None, None).rows, "nothing on a row moves between its changes");
        assert_eq!(at20.rows[0].status, EspionageStatus::Running);
        assert_eq!(at20.rows[0].start_hour, Some(h20));
        assert_eq!(at20.rows[0].duration_days, Some(30));
        assert_eq!(at20.next_change, Some(h30));

        let ger = Some(vec![QueryCountry::Slot(1)]);
        let as_owner = ask(h30, Some(EspionageCountryRole::Owner), ger.clone(), None);
        assert_eq!(as_owner.rows.len(), 1);
        assert_eq!(as_owner.rows[0].operation_token.as_deref(), Some("operation_infiltrate_army"));

        let as_target = ask(h30, Some(EspionageCountryRole::Target), ger.clone(), None);
        assert_eq!(as_target.rows.len(), 1);
        assert_eq!(as_target.rows[0].operation_token.as_deref(), Some("operation_capture_cipher"));

        let as_either = ask(h30, Some(EspionageCountryRole::Either), ger, None);
        assert_eq!(as_either.rows.len(), 2);

        let by_prep = ask(h30, None, None, Some(EspionageRunningOrder::Preparation));
        assert_eq!(by_prep.rows[0].operation_token.as_deref(), Some("operation_infiltrate_army"));
        assert_eq!(by_prep.rows[1].operation_token.as_deref(), Some("operation_capture_cipher"));

        let at40 = ask(h40, None, None, None);
        assert_eq!(at40.live, "1");
        let completed = at40.rows.iter().find(|r| r.operation_token.as_deref() == Some("operation_infiltrate_army")).expect("completed op");
        assert_eq!(completed.status, EspionageStatus::Completed);
        assert!(!completed.live);
        assert_eq!(completed.completion.as_ref().and_then(|c| c.bonus), Some(true));
        assert_eq!(completed.completion.as_ref().and_then(|c| c.detected), Some(false));
        assert_eq!(completed.completion.as_ref().and_then(|c| c.description.as_deref()), Some("Infiltrated army"));

        let at50 = ask(h50, None, None, None);
        assert_eq!(at50.live, "0");
        let ended = at50.rows.iter().find(|r| r.operation_token.as_deref() == Some("operation_capture_cipher")).expect("ended op");
        assert_eq!(ended.status, EspionageStatus::Ended);
        assert_eq!(ended.ended_hour, Some(h50));
        assert_eq!(ended.last_seen, Some(h50 - 1), "an operation no longer read keeps its last sighting");
    }
}
