
use super::{
    Asked, Content, EvaluatedLeader, LeaderKinds, LeaderRole, LeaderSelection, LeaderTrait, LeadersContext, RESULTS_MISSING, Tally, TopLeader, TraitList, UnitLeaders, UnitLeadersParams,
    cell_number, cell_text, handle, held_traits_for, parse_branch,
};
use dashboard_base::{Source, products::{LoadContext, Product, at_hour::{At, AtHour, Keys, Owner, Read, ReadsAt}}, queries::country_ids::{Names, RunLinks}};
use dashboard_intelligence::products::intelligence::Tags;
use dashboard_follow::live::Source as Cell;
use dashboard_query::{Query, log::{FacetFilter, Reader}};
use replay_lanes::lanes::{Cell as Step, History, Keys as LaneKeys};
use replay_data::{
    codec::Decoder,
    rows::{LeaderBattleRow, LeaderCommandRow as Command, LeaderTraitGainRow, TableId, tables},
    value::Value as _,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn field(table: TableId, name: &str) -> io::Result<usize> {
    dashboard_declare::field(table, name)
}

fn reference(text: &str) -> Result<u64, &'static str> {
    text.parse::<u64>().map_err(|_| "Invalid leader reference")
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaderCareerParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub selection: Option<LeaderSelection>,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub role: LeaderRole,
    pub skill: Option<i32>,
    pub components: [i32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerBattle {
    pub start: i32,
    pub read: i32,
    pub unknown: bool,
    pub kind: u8,
    pub won: Option<bool>,
    pub result_hour: Option<i32>,
    pub result_read: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum CommandKind {
    Army,
    ArmyGroup,
    Fleet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderCommand {
    pub first_hour: i32,
    pub last_hour: i32,
    pub formation: String,
    pub kind: CommandKind,
    #[doc = " The formation's stored name at its first hour; none where it was not recorded."]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderCareerReply {
    pub reference: Option<String>,
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which the leader's career or selected top leader changes; null when none does."]
    pub next_change: Option<i32>,
    pub name: Option<String>,
    #[doc = " The hour the name and portrait are read at: the leader's last on the timeline through `hour`."]
    pub portrait_hour: Option<i32>,
    pub runs: Vec<CareerRun>,
    #[doc = " [hour, place in `traits`, class]: 0 held before, 1 gained, 2 first observed."]
    pub trait_gains: Vec<(i32, u32, u8)>,
    pub traits: Vec<LeaderTrait>,
    pub commands: Vec<LeaderCommand>,
    pub battles: Vec<CareerBattle>,
    #[doc = " The timeline's recorded stretches through `hour`: a battle lane is drawn only over them."]
    pub observed: Vec<(i32, i32)>,
    pub battles_partial_from: Option<i32>,
    pub results_missing: String,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

pub struct LeaderCareer;

struct RunColumns {
    columns: Vec<usize>,
}

impl RunColumns {
    fn new() -> io::Result<Self> {
        let leaders = TableId::UnitLeaders;
        let first = field(leaders, "skill_category")? + 1;
        Ok(Self {
            columns: vec![
                field(leaders, "owner")?,
                field(leaders, "category")?,
                field(leaders, "kind")?,
                field(leaders, "skill_valid")?,
                field(leaders, "skill_level")?,
                first,
                first + 1,
                first + 2,
                first + 3,
            ],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunValues {
    pub owner: u32,
    pub category: i32,
    pub kind: u8,
    pub skill: Option<i32>,
    pub components: [i32; 4],
}

fn values_of(history: &History, cells: &[Step], columns: &[usize]) -> Option<RunValues> {
    let mut at: HashMap<usize, i64> = HashMap::new();
    for (column, cell) in history.columns.iter().zip(cells) {
        if let Step::Number(value) = cell {
            at.insert(*column, *value);
        }
    }
    let get = |at_index: usize| at.get(&columns[at_index]).copied();
    Some(RunValues {
        owner: get(0)? as u32,
        category: get(1)? as i32,
        kind: get(2)? as u8,
        skill: (get(3)? != 0).then_some(get(4)? as i32),
        components: [get(5)? as i32, get(6)? as i32, get(7)? as i32, get(8)? as i32],
    })
}

pub fn career_runs(steps: &[(i32, Option<RunValues>)], to: i32) -> Vec<(i32, i32, RunValues)> {
    let mut out: Vec<(i32, i32, RunValues)> = Vec::new();
    for (at, (hour, values)) in steps.iter().enumerate() {
        let last = steps.get(at + 1).map_or(to, |(next, _)| next - 1);
        let Some(values) = values else { continue };
        if last < *hour {
            continue;
        }
        match out.last_mut() {
            Some(held) if held.2 == *values && held.1 + 1 == *hour => held.1 = last,
            _ => out.push((*hour, last, *values)),
        }
    }
    out
}

pub fn led_spans(steps: &[(i32, Option<u64>)], leader: u64, to: i32) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (at, (hour, led)) in steps.iter().enumerate() {
        let last = steps.get(at + 1).map_or(to, |(next, _)| next - 1);
        if *led != Some(leader) || last < *hour {
            continue;
        }
        match out.last_mut() {
            Some(held) if held.1 + 1 == *hour => held.1 = last,
            _ => out.push((*hour, last)),
        }
    }
    out
}

pub type BattleRow = (i32, u8, u8, u64, i32, bool, Option<u8>, Option<i32>);

pub fn join_battles(rows: &[BattleRow]) -> Vec<CareerBattle> {
    let mut results: BTreeMap<(u8, u64), Vec<(i32, Option<u8>, Option<i32>)>> = BTreeMap::new();
    for &(read, entry, kind, combat, _, _, outcome, result_hour) in rows {
        if entry == LeaderBattleRow::RESULT {
            results.entry((kind, combat)).or_default().push((read, outcome, result_hour));
        }
    }
    let mut out: Vec<CareerBattle> = rows
        .iter()
        .filter(|row| row.1 == LeaderBattleRow::PARTICIPATION)
        .map(|&(read, _, kind, combat, start, unknown, _, _)| {
            let result = results.get(&(kind, combat)).and_then(|found| found.iter().find(|result| result.0 >= read));
            let won = result.and_then(|result| match result.1 {
                Some(LeaderBattleRow::WON) => Some(true),
                Some(LeaderBattleRow::LOST) => Some(false),
                _ => None,
            });
            CareerBattle {
                start,
                read,
                unknown,
                kind,
                won,
                result_hour: won.and(result.and_then(|result| result.2)),
                result_read: won.and(result.map(|result| result.0)),
            }
        })
        .collect();
    out.sort_by_key(|battle| (battle.start, battle.read, battle.kind));
    out
}

pub fn career_gains(rows: &[(i32, String, u8)]) -> Vec<(i32, String, u8)> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (hour, token, class) in rows {
        if *class == LeaderTraitGainRow::GAINED || seen.insert(token.as_str()) {
            seen.insert(token.as_str());
            out.push((*hour, token.clone(), *class));
        }
    }
    out
}

fn command_kind(kind: u8) -> CommandKind {
    match kind {
        Command::ARMY_GROUP => CommandKind::ArmyGroup,
        Command::NAVY => CommandKind::Fleet,
        _ => CommandKind::Army,
    }
}

fn stored_name(content: &Content, bytes: &[u8]) -> Option<String> {
    let name = Option::<Vec<u8>>::decode(&mut Decoder::new(bytes)).ok()??;
    content.stored_text(&name)
}

fn leader_rows(query: &Query<'_>, log: &str, id: u32, leader: u64) -> io::Result<(Vec<&'static str>, Vec<dashboard_query::log::Row>)> {
    let reader = Reader::new(query, dashboard_declare::log(log).ok_or_else(|| dashboard_core::invalid(format!("The {log} log is not declared")))?)?;
    let filter = FacetFilter { name: "leader".into(), values: vec![i64::from(id)], ..Default::default() };
    let (first, last) = (query.timeline.first, query.timeline.last);
    let asked = reader.select(&[filter], first, last)?;
    let rows = reader.every(&asked.patterns, first, last)?.into_iter().map(|(_, row)| row).filter(|row| (row.key >> 64) as u64 == leader).collect();
    Ok((reader.columns(), rows))
}

fn column(columns: &[&str], name: &str) -> io::Result<usize> {
    columns.iter().position(|column| *column == name).ok_or_else(|| dashboard_core::invalid(format!("A leader log has no {name}")))
}

struct NextHour(Option<i32>, i32);

impl NextHour {
    fn see(&mut self, at: i32) {
        if at > self.1 && self.0.is_none_or(|next| at < next) {
            self.0 = Some(at);
        }
    }

    fn span(&mut self, first: i32, last: i32) {
        self.see(first);
        self.see(last.saturating_add(1));
    }
}

fn ranking_steps(history: &History, after: i32, next: &mut NextHour) {
    for step in history.keys.iter().flat_map(|(_, steps)| steps).filter(|step| step.hour > after) {
        next.see(step.hour);
    }
    let complete = |run: &replay_lanes::lanes::StatusRun| matches!(run.status, replay_columnar::manifest::Status::Complete);
    for pair in history.statuses.windows(2).filter(|pair| complete(&pair[0]) != complete(&pair[1]) && pair[1].first > after) {
        next.see(pair[1].first);
    }
}

fn ranking_tokens(query: &Query<'_>, run: u32, hour: i32) -> io::Result<Vec<u128>> {
    let characters = TableId::Characters;
    let columns = [field(characters, "name_token_id")?, field(characters, "template_tag")?];
    let held = dashboard_query::cost::storage(|| replay_lanes::lanes::at(query.replay.recording(), run, characters, hour, Some(&columns)))?;
    let view = held.view();
    let mut tokens = Vec::new();
    for column in columns {
        tokens.extend(view.rows().filter_map(|slot| u32::try_from(view.number(column, slot)?).ok()).map(u128::from));
    }
    tokens.sort_unstable();
    tokens.dedup();
    Ok(tokens)
}

fn next_ranking_change(query: &Query<'_>, hour: i32, through: i32, members: Option<&[(u128, u128)]>) -> io::Result<Option<i32>> {
    let recording = query.replay.recording();
    let leaders = TableId::UnitLeaders;
    let components = field(leaders, "skill_category")? + 1;
    let characters = TableId::Characters;
    let lexical = TableId::LexicalTokens;
    let mut leader_fields = dashboard_base::products::at_hour::field_columns(
        leaders,
        &[
            "owner", "kind", "category", "captured", "character_id", "skill_valid", "skill_level",
            "visibility_choice", "template_visibility", "visible_state", "visible_origin",
        ],
    );
    leader_fields.extend(components..components + 4);
    let mut character_fields = dashboard_base::products::at_hour::field_columns(
        characters,
        &["name_token_id", "name", "token", "template_present", "template_tag", "template_file", "template_line", "template_generated"],
    );
    let mut country_fields = dashboard_base::products::at_hour::field_columns(TableId::Countries, &["tag", "canonical_slot"]);
    leader_fields.sort_unstable();
    character_fields.sort_unstable();
    country_fields.sort_unstable();
    let inputs = [
        (leaders, leader_fields),
        (TableId::LeaderBattleTotals, (0..TableId::LeaderBattleTotals.field_count()).collect()),
        (characters, character_fields),
        (TableId::Countries, country_fields),
        (TableId::CountryLinks, vec![field(TableId::CountryLinks, "canonical_slot")?]),
    ];
    let literal = [field(lexical, "literal")?];
    let held = |table: TableId| recording.predates(table.index()).is_none() && recording.withheld_reason(table.index()).is_none();
    let block = replay_columnar::envelope::UNIT_ORDINALS as i32;
    let mut next = NextHour(None, hour);
    for stretch in query.timeline.within(hour, through) {
        let last = stretch.last;
        if last <= hour { continue; }
        let origin = recording.run(stretch.run)?.first_hour;
        let start = hour.max(stretch.first);
        let tokens = if held(characters) && held(lexical) { ranking_tokens(query, stretch.run, start)? } else { Vec::new() };
        let mut from = start;
        while from <= last {
            let after = if from == start { hour.max(from - 1) } else { from - 1 };
            let read_from = if from == start { from } else { from - 1 };
            let to = last.min(origin + ((from - origin) / block + 1) * block - 1);
            for (table, columns) in inputs.iter().filter(|(table, _)| held(*table)) {
                let keys = if *table == TableId::LeaderBattleTotals { members.map_or(LaneKeys::All, LaneKeys::Ranges) } else { LaneKeys::All };
                let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, stretch.run, *table, columns, keys, read_from, to))?;
                ranking_steps(&history, after, &mut next);
            }
            let spelled_to = next.0.map_or(to, |at| at.saturating_sub(1).min(to));
            if !tokens.is_empty() && spelled_to > after {
                let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, stretch.run, lexical, &literal, LaneKeys::These(&tokens), read_from, spelled_to))?;
                ranking_steps(&history, after, &mut next);
            }
            if next.0.is_some() {
                return Ok(next.0);
            }
            from = to + 1;
        }
    }
    Ok(next.0)
}

struct Full {
    runs: Vec<(i32, i32, Option<String>, RunValues)>,
    seen: Vec<(u32, i32, i32)>,
    commands: Vec<(i32, i32, u64, u8, Option<Vec<u8>>)>,
    gains: Vec<(i32, String, u8)>,
    battles: Vec<CareerBattle>,
    partial: Option<i32>,
}

#[derive(Default)]
struct Careers(HashMap<(u64, u64), std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Full>>>>>);

fn full_career(source: &Source, query: &Query<'_>, leader: u64, cancelled: &AtomicBool) -> io::Result<std::sync::Arc<Full>> {
    let slot = std::sync::Arc::clone(source.state::<Careers>().lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.entry((query.timeline.id, leader)).or_default());
    let mut held = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(full) = held.as_ref() {
        return Ok(std::sync::Arc::clone(full));
    }
    let full = std::sync::Arc::new(read_full(source, query, leader, cancelled)?);
    *held = Some(std::sync::Arc::clone(&full));
    Ok(full)
}

#[cfg(test)]
thread_local! {
    pub(super) static FULL_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn read_full(source: &Source, query: &Query<'_>, leader: u64, cancelled: &AtomicBool) -> io::Result<Full> {
    #[cfg(test)]
    FULL_READS.with(|reads| reads.set(reads.get() + 1));
    let key = u128::from(leader);
    let recording = query.replay.recording();
    let names = Names::new(&query.holders);
    let run_columns = RunColumns::new()?;
    let mut ascending = run_columns.columns.clone();
    ascending.sort_unstable();
    let (mut runs, mut seen, mut commands) = (Vec::new(), Vec::new(), Vec::new());
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let links: std::sync::Arc<RunLinks> = dashboard_base::queries::links(source, stretch.run)?;
        let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, stretch.run, TableId::UnitLeaders, &ascending, LaneKeys::These(&[key]), stretch.first, stretch.last))?;
        let steps: Vec<(i32, Option<RunValues>)> = history
            .keys
            .iter()
            .filter(|(held, _)| *held == key)
            .flat_map(|(_, steps)| steps.iter().map(|step| (step.hour, step.cells.as_deref().and_then(|cells| values_of(&history, cells, &run_columns.columns)))))
            .collect();
        for (first, last, values) in career_runs(&steps, history.to) {
            seen.push((stretch.run, first, last));
            for (tag, a, b) in names.tags_over(&links, values.owner, first, last) {
                runs.push((a, b, tag.map(str::to_owned), values));
            }
        }
        let open = dashboard_query::cost::storage(|| replay_lanes::lanes::at(recording, stretch.run, TableId::LeaderCommandSpans, stretch.last, None))?;
        let view = open.view();
        for slot in view.rows() {
            if view.number(0, slot).map(|held| held as u64) != Some(leader) {
                continue;
            }
            let (Some(key), Some(first), Some(kind)) = (view.key(slot), view.number(1, slot), view.number(2, slot)) else { continue };
            commands.push(((first as i32).max(stretch.first), stretch.last, key as u64, kind as u8, view.list(3, slot).map(<[u8]>::to_vec)));
        }
    }
    let (command_columns, command_rows) = leader_rows(query, "leader-commands", (leader >> 32) as u32, leader)?;
    let at = |name: &str| column(&command_columns, name);
    let (formation, kind, first, last, name) = (at("formation")?, at("kind")?, at("first_hour")?, at("last_hour")?, at("name")?);
    for row in &command_rows {
        let Some(stretch) = query.timeline.at(row.hour) else { continue };
        let number = |at: usize| cell_number(&row.cells[at]);
        let (Some(formation), Some(kind), Some(first), Some(last)) = (number(formation), number(kind), number(first), number(last)) else { continue };
        let name = match &row.cells[name] {
            dashboard_query::log::Cell::List(bytes) => Some(bytes.clone()),
            _ => None,
        };
        commands.push(((first as i32).max(stretch.first), last as i32, formation as u64, kind as u8, name));
    }
    commands.sort_by_key(|command| (command.0, command.2.to_string()));
    let id = (leader >> 32) as u32;
    let (gain_columns, gain_rows) = leader_rows(query, "leader-trait-gains", id, leader)?;
    let (class, spelling) = (column(&gain_columns, "class")?, column(&gain_columns, "spelling")?);
    let gains: Vec<(i32, String, u8)> = gain_rows
        .iter()
        .filter_map(|row| Some((row.hour, cell_text(&row.cells[spelling])?, cell_number(&row.cells[class])? as u8)))
        .collect();
    let (battle_columns, battle_rows) = leader_rows(query, "leader-battles", id, leader)?;
    let at = |name: &str| column(&battle_columns, name);
    let (entry, kind, combat, start, unknown, outcome, result_hour) = (at("entry")?, at("kind")?, at("combat")?, at("start_hour")?, at("start_unknown")?, at("outcome")?, at("result_hour")?);
    let battle_input: Vec<BattleRow> = battle_rows
        .iter()
        .filter_map(|row| {
            let number = |at: usize| cell_number(&row.cells[at]);
            Some((row.hour, number(entry)? as u8, number(kind)? as u8, number(combat)? as u64, number(start)? as i32, number(unknown)? != 0, number(outcome).map(|value| value as u8), number(result_hour).map(|value| value as i32)))
        })
        .collect();
    Ok(Full {
        runs,
        seen,
        commands,
        gains: career_gains(&gains),
        battles: join_battles(&battle_input),
        partial: super::battles_partial_from(query, query.timeline.first, query.timeline.last)?,
    })
}

fn resolve_selection_at(
    source: &Source,
    branch_id: &str,
    selection: &LeaderSelection,
    next: &mut NextHour,
    load: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<(Option<u64>, Option<String>, Vec<String>)> {
    let asked = top_asked(branch_id, selection);
    let hour = next.1;
    let context = LeadersContext::for_selection(source, load, branch_id, Some(selection), cancelled)?;
    let branch = parse_branch(branch_id).map_err(dashboard_core::invalid)?;
    let query = context.query(branch)?;
    let through = context.roster.as_ref().map_or(i32::MAX, |roster| roster.through(hour));
    if let Some(change) = through.checked_add(1) { next.see(change); }
    if let Some(reason) = context.roster.as_ref().and_then(|roster| roster.missing(hour)) {
        return Ok((None, Some(reason.to_owned()), Vec::new()));
    }
    let Some(stretch) = query.timeline.at(hour) else {
        return Ok((None, Some("This hour is not recorded on this branch".into()), Vec::new()));
    };
    let mut frame = DecodedFrame {
        branch,
        hour,
        tables: std::array::from_fn(|_| replay_frame::frame::not_selected()),
        metrics: replay_frame::frame::Metrics::default(),
    };
    let live = dashboard_follow::live::Cells::default();
    let reads_at = ReadsAt { holders: &query.holders, context: &context, run: stretch.run, hour, since: None };
    let mut battle_keys = None;
    for step in 0..8 {
        let reads = if step == 1 && context.roster.is_some() {
            let kept = UnitLeaders::kept(&asked, &frame, &reads_at);
            let ranges = leader_ranges(kept.iter().map(|row| u64::from(row.id)), 32);
            let mut reads = UnitLeaders::people(&kept);
            reads.push(Read { table: TableId::LeaderBattleTotals, keys: Keys::Ranges(ranges.clone()) });
            reads.push(Read { table: TableId::LeaderTraitDates, keys: Keys::Ranges(leader_ranges(kept.iter().map(|row| handle(row)), 32)) });
            battle_keys = Some(ranges);
            reads
        } else if step == 2 {
            let (reads, clocked) = UnitLeaders::visibility_reads(&asked, &frame, &reads_at, true);
            if clocked || !reads.is_empty() { next.see(hour.saturating_add(1)); }
            reads
        } else {
            UnitLeaders::reads_in(&asked, None, step, &frame, &reads_at)
        };
        if reads.is_empty() {
            break;
        }
        let step_frame = dashboard_base::products::at_hour::frame(&query, hour, &reads, "General career")?;
        for read in &reads {
            let idx = read.table.index();
            frame.tables[idx] = step_frame.tables[idx].clone();
        }
    }
    let until = next.0.map_or(through, |at| through.min(at - 1));
    if let Some(change) = next_ranking_change(&query, hour, until, battle_keys.as_deref())? { next.see(change); }
    let at = At { owner: None, holders: &query.holders, hour, context: &context, cancelled, live: &live };
    let evaluated = UnitLeaders::evaluate(&frame, &asked, &at)?;
    match evaluated.top() {
        TopLeader::Winner(winner) => Ok((Some(winner.handle), None, evaluated.warnings)),
        TopLeader::None => Ok((None, None, evaluated.warnings)),
        TopLeader::Missing(reason) => Ok((None, Some(reason), evaluated.warnings)),
    }
}

impl Product for LeaderCareer {
    const KEY: &'static str = "unit-leader-career";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = LeaderCareerParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, reference, selection, "selection.countries[]", hour: Read];
    type Reply = LeaderCareerReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderCareerParams::decl(), CareerRun::decl(), CareerBattle::decl(), CommandKind::decl(), LeaderCommand::decl(), LeaderCareerReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        match (&params.reference, &params.selection) {
            (Some(text), None) => reference(text).map(|_| ()),
            (None, Some(selection)) => UnitLeaders::validate(&top_asked(&params.branch_id, selection)),
            _ => Err("Ask for one leader by reference or for the top general of a selection"),
        }
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?)?;
        let hour = params.hour.min(query.timeline.last);
        let mut next = NextHour(None, hour);
        let (leader, selection_warnings) = match (&params.reference, &params.selection) {
            (Some(text), None) => (reference(text).map_err(dashboard_core::invalid)?, Vec::new()),
            (None, Some(selection)) => {
                let (picked, missing, warnings) = resolve_selection_at(source, &params.branch_id, selection, &mut next, load, cancelled)?;
                let Some(leader) = picked else {
                    for stretch in &query.timeline.stretches {
                        next.span(stretch.first, stretch.last);
                    }
                    let observed = query
                        .timeline
                        .stretches
                        .iter()
                        .filter(|s| s.first <= hour)
                        .map(|s| (s.first, s.last.min(hour)))
                        .collect();
                    return Ok(LeaderCareerReply {
                        reference: None,
                        hour,
                        next_change: next.0.filter(|&at| at <= query.timeline.last),
                        name: None,
                        portrait_hour: None,
                        runs: Vec::new(),
                        trait_gains: Vec::new(),
                        traits: Vec::new(),
                        commands: Vec::new(),
                        battles: Vec::new(),
                        observed,
                        battles_partial_from: super::battles_partial_from(&query, query.timeline.first, hour)?,
                        results_missing: RESULTS_MISSING.into(),
                        missing,
                        warnings,
                    });
                };
                (leader, warnings)
            }
            _ => return Err(dashboard_core::invalid("Ask for one leader by reference or for the top general of a selection")),
        };
        let key = u128::from(leader);
        let content = Content::load(source, load);
        let recording = query.replay.recording();
        let full = full_career(source, &query, leader, cancelled)?;
        for stretch in &query.timeline.stretches {
            next.span(stretch.first, stretch.last);
        }
        let last_seen = full.seen.iter().rfind(|seen| seen.1 <= hour).map(|&(run, _, last)| (run, last.min(hour)));
        let mut traits = TraitList::default();
        let mut trait_gains = Vec::new();
        for (gain_hour, token, gain_class) in &full.gains {
            next.see(*gain_hour);
            if *gain_hour <= hour {
                trait_gains.push((*gain_hour, traits.index(token, &content), *gain_class));
            }
        }
        let mut battles = Vec::new();
        for battle in &full.battles {
            let visible_at = battle.start.max(battle.read);
            next.see(visible_at);
            if let Some(rr) = battle.result_read {
                let res_at = battle.result_hour.map_or(rr, |rh| rh.max(rr));
                next.see(res_at);
            }
            if visible_at > hour {
                continue;
            }
            let result_known = battle.result_read.is_some_and(|rr| rr <= hour) && battle.result_hour.is_none_or(|rh| rh <= hour);
            battles.push(CareerBattle {
                won: if result_known { battle.won } else { None },
                result_hour: if result_known { battle.result_hour } else { None },
                result_read: if result_known { battle.result_read } else { None },
                ..battle.clone()
            });
        }
        let mut runs = Vec::new();
        for (first, last, tag, values) in &full.runs {
            next.span(*first, *last);
            if *first <= hour {
                runs.push(CareerRun { first_hour: *first, last_hour: (*last).min(hour), role: content.role(tag.clone(), values.kind, values.category, values.skill), skill: values.skill, components: values.components });
            }
        }
        let mut commands = Vec::new();
        for (first, last, formation, kind, name) in &full.commands {
            next.span(*first, *last);
            if *first <= hour {
                commands.push(LeaderCommand {
                    first_hour: *first,
                    last_hour: (*last).min(hour),
                    formation: formation.to_string(),
                    kind: command_kind(*kind),
                    name: name.as_deref().and_then(|bytes| stored_name(&content, bytes)),
                });
            }
        }
        if let Some(partial_at) = full.partial {
            next.see(partial_at);
        }
        let battles_partial_from = full.partial.filter(|&at| at <= hour);
        let (name, portrait_hour, missing) = match last_seen {
            Some((run, seen_hour)) => {
                let leaders = [key];
                let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, seen_hour, &[replay_lanes::at::Read { table: TableId::UnitLeaders, columns: None, keys: replay_lanes::at::Keys::These(&leaders) }, replay_lanes::at::Read::all(TableId::LexicalTokens)]))?;
                let character = frame.complete::<tables::UnitLeaders>().ok().and_then(|rows| rows.iter().find(|row| handle(row) == leader).map(|row| row.character_id));
                let characters: Vec<u128> = character.into_iter().map(u128::from).collect();
                let people = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, seen_hour, &[replay_lanes::at::Read { table: TableId::Characters, columns: None, keys: replay_lanes::at::Keys::These(&characters) }]))?;
                let person = character.and_then(|id| {
                    people.complete::<tables::Characters>().ok().and_then(|rows| {
                        rows.binary_search_by_key(&id, |row| row.id)
                            .ok()
                            .or_else(|| rows.iter().position(|row| row.id == id))
                            .map(|at| &rows[at])
                    })
                });
                let name = content.person_name(person, frame.complete::<tables::LexicalTokens>().ok());
                (name, Some(seen_hour), None)
            }
            None => (None, None, Some("This leader is not recorded on this timeline by this hour".to_owned())),
        };
        let observed = query
            .timeline
            .stretches
            .iter()
            .filter(|stretch| stretch.first <= hour)
            .map(|stretch| (stretch.first, stretch.last.min(hour)))
            .collect();
        let mut warnings = selection_warnings;
        warnings.extend(content.missing());
        Ok(LeaderCareerReply {
            reference: Some(leader.to_string()),
            hour,
            next_change: next.0.filter(|&at| at <= query.timeline.last),
            name,
            portrait_hour,
            runs,
            trait_gains,
            traits: traits.traits,
            commands,
            battles,
            observed,
            battles_partial_from,
            results_missing: RESULTS_MISSING.into(),
            missing,
            warnings,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaderHourParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub selection: Option<LeaderSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderTraining {
    pub key: String,
    pub token: Option<String>,
    pub name: Option<String>,
    pub required_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderFactor {
    pub token: Option<String>,
    pub name: Option<String>,
    pub factor_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderProgress {
    pub next_required_raw: Option<String>,
    #[doc = " None where the training rows were not recorded."]
    pub training: Option<Vec<LeaderTraining>>,
    pub factors: Option<Vec<LeaderFactor>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LeaderServiceStart {
    Since { hour: i32 },
    Before { hour: i32 },
    Unknown { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderHourReply {
    pub reference: Option<String>,
    #[doc = " Why the leader is not shown at this hour, when it is not."]
    pub missing: Option<String>,
    pub name: Option<String>,
    pub role: Option<LeaderRole>,
    pub skill: Option<i32>,
    pub components: [i32; 4],
    pub traits: Option<Vec<u32>>,
    pub trait_dates: Vec<Option<(i32, u8)>>,
    pub trait_catalogue: Vec<LeaderTrait>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub fought: u64,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub won: u64,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub lost: u64,
    pub battles_partial_from: Option<i32>,
    pub first_seen_hour: Option<i32>,
    pub service_start: Option<LeaderServiceStart>,
    pub progress: Option<LeaderProgress>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LeaderHourReply, LeaderCareerReply, LeaderChoicesReply);

pub struct LeaderHour;

fn leader_ranges(keys: impl Iterator<Item = u64>, bits: u32) -> Vec<(u128, u128)> {
    let mask = (1u128 << bits) - 1;
    let mut ranges: Vec<_> = keys.map(|key| { let lo = u128::from(key) << bits; (lo, lo | mask) }).collect();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

pub(super) fn member_details(asked: &UnitLeadersParams, held: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>, progress: bool) -> Vec<Read> {
    let kept = UnitLeaders::kept(asked, held, at);
    let mut reads = UnitLeaders::people(&kept);
    let handles = || kept.iter().map(|row| handle(row));
    let ranges = leader_ranges(handles(), 32);
    reads.push(Read { table: TableId::UnitLeaderTraits, keys: Keys::Ranges(ranges.clone()) });
    if !kept.is_empty() {
        if progress { reads.push(Read { table: TableId::UnitLeaderTraitExperience, keys: Keys::Ranges(leader_ranges(handles(), 33)) }); }
        reads.push(Read { table: TableId::LeaderBattleTotals, keys: Keys::Ranges(leader_ranges(kept.iter().map(|row| u64::from(row.id)), 32)) });
        reads.push(Read { table: TableId::LeaderTraitDates, keys: Keys::Ranges(ranges) });
    }
    reads
}

pub(super) fn leader_columns(table: TableId, plan: &[(Vec<Read>, i32)]) -> Option<Vec<usize>> {
    if plan.get(2).is_some_and(|(reads, _)| !reads.is_empty()) { return None; }
    let fields = match table {
        TableId::Countries => &["tag", "canonical_slot"][..],
        TableId::CountryLinks => &["canonical_slot"],
        TableId::Characters => &["name_token_id", "name", "token", "template_present", "template_tag", "template_file", "template_line", "template_generated"],
        TableId::UnitLeaderTraits => &["token"],
        TableId::UnitLeaderTraitExperience => &["token_id", "experience"],
        TableId::UnitLeaders => {
            let mut fields = dashboard_base::products::at_hour::field_columns(table, &["owner", "kind", "category", "captured", "character_id", "skill_valid", "skill_level", "visibility_choice", "template_visibility", "visible_state", "visible_origin"]);
            let components = field(table, "skill_category").expect("leader skill category") + 1;
            fields.extend(components..components + 4);
            fields.sort_unstable();
            return Some(fields);
        }
        _ => return None,
    };
    Some(dashboard_base::products::at_hour::field_columns(table, fields))
}

pub(super) fn progress_token_changed(frame: &DecodedFrame, leader: u64, changes: &std::collections::BTreeMap<u128, u64>) -> bool {
    let Ok(progress) = frame.complete::<tables::UnitLeaderTraitExperience>() else { return false };
    let first = progress.partition_point(|r| r.leader_reference < leader);
    progress[first..].iter().take_while(|r| r.leader_reference == leader)
        .any(|r| u32::try_from(r.token_id).is_ok_and(|id| changes.contains_key(&u128::from(id))))
}

impl AtHour for LeaderHour {
    const KEY: &'static str = "unit-leader-hour";
    const TITLE: &'static str = "General career";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = LeaderHourParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, reference, selection, "selection.countries[]"];
    type Reply = LeaderHourReply;
    type Context = LeadersContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderHourParams::decl(), LeaderTraining::decl(), LeaderFactor::decl(), LeaderProgress::decl(), LeaderServiceStart::decl(), LeaderHourReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        match (&params.reference, &params.selection) {
            (Some(text), None) => reference(text).map(|_| ()),
            (None, Some(selection)) => UnitLeaders::validate(&top_asked(&params.branch_id, selection)),
            _ => Err("Ask for one leader by reference or for the top general of a selection"),
        }
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![
            TableId::UnitLeaders,
            TableId::UnitLeaderTraits,
            TableId::UnitLeaderTraitExperience,
            TableId::Characters,
            TableId::LexicalTokens,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LeaderBattleTotals,
            TableId::LeaderTraitDates,
        ];
        if let Some(selection) = &params.selection {
            tables.extend(UnitLeaders::tables(&top_asked(&params.branch_id, selection)).into_iter().filter(|table| !tables.contains(table)).collect::<Vec<_>>());
        }
        tables
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let Some(selection) = &params.selection else { return Self::reads(params, owner, step, held) };
        let asked = top_asked(&params.branch_id, selection);
        if step == 1 { return member_details(&asked, held, at, true); }
        UnitLeaders::reads_in(&asked, owner, step, held, at)
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(Ok(leader)) = params.reference.as_deref().map(reference) else { return Vec::new() };
        let key = u128::from(leader);
        match step {
            0 => vec![
                Read::these(TableId::UnitLeaders, [key]),
                Read { table: TableId::LeaderBattleTotals, keys: Keys::Range(key & !u128::from(u32::MAX), key | u128::from(u32::MAX)) },
                Read { table: TableId::LeaderTraitDates, keys: Keys::Range(key << 32, (key << 32) | u128::from(u32::MAX)) },
                Read { table: TableId::UnitLeaderTraits, keys: Keys::Range(key << 32, (key << 32) | u128::from(u32::MAX)) },
                Read { table: TableId::UnitLeaderTraitExperience, keys: Keys::Range(key << 33, (key << 33) | ((1u128 << 33) - 1)) },
                Read::all(TableId::LexicalTokens),
                Read::all(TableId::Countries),
                Read::all(TableId::CountryLinks),
            ],
            1 => held
                .complete::<tables::UnitLeaders>()
                .ok()
                .and_then(|rows| rows.iter().find(|row| handle(row) == leader))
                .map(|row| vec![Read::these(TableId::Characters, [u128::from(row.character_id)])])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        LeadersContext::load(source, load)
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        LeadersContext::for_selection(source, load, &params.branch_id, params.selection.as_ref(), &AtomicBool::new(false))
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let reads = match (stage, &params.selection) {
            (2, Some(selection)) => return super::visibility::prepare(&top_asked(&params.branch_id, selection), held, at),
            _ => Self::reads_in(params, owner, stage, held, at),
        };
        (reads, at.context.through(at.hour))
    }

    const UPDATES_REVISION: bool = true;
    const RESEED_ON_SEEK: bool = true;

    fn context_revision(context: &Self::Context) -> u64 { context.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner).revision }

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        super::leader_plan_changes(stage, edits)
    }

    fn projection(_params: &Self::Params, table: TableId, plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        leader_columns(table, plan)
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        if !frame.tables[TableId::Clock.index()].held().is_not_selected() { return Ok(false); }
        if let Some(selection) = &params.selection {
            let mut rank = at.context.ranking.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            return rank.top_step(frame, &top_asked(&params.branch_id, selection), at, edits, data);
        }
        let winner = data.get("reference").and_then(serde_json::Value::as_str).and_then(|key| key.parse::<u128>().ok());
        let experience = 1 << field(TableId::UnitLeaderTraitExperience, "experience")?;
        Ok(edits.replaced.is_empty() && edits.tables.iter().all(|(&table, rows)| match table {
            TableId::UnitLeaderTraitExperience => rows.iter().all(|(&key, &fields)| {
                Some(key >> 33) != winner || key & (1 << 32) == 0 && fields & !experience == 0
            }),
            TableId::UnitLeaderTraits | TableId::LeaderTraitDates => rows.keys().all(|key| Some(key >> 32) != winner),
            TableId::LexicalTokens => {
                let uses_name = frame.complete::<tables::Characters>().is_ok_and(|people| people.iter()
                    .any(|person| u32::try_from(person.name_token_id).is_ok_and(|id| rows.contains_key(&u128::from(id)))));
                !uses_name && winner.is_none_or(|id| !progress_token_changed(frame, id as u64, rows))
            }
            _ => false,
        }))
    }

    fn valid_through(_frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.through(at.hour)
    }

    fn valid_reply_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        super::reply_through(at.context, frame, reply.missing.as_deref(), &reply.warnings)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let Some(selection) = &params.selection else {
            let leader = reference(params.reference.as_deref().unwrap_or_default()).map_err(dashboard_core::invalid)?;
            return one_leader(frame, &params.branch_id, leader, at);
        };
        let evaluated = UnitLeaders::evaluate(frame, &top_asked(&params.branch_id, selection), at)?;
        Self::from_evaluated(frame, params, evaluated, at)
    }

    fn playback_reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let Some(selection) = &params.selection else { return Self::reply(frame, params, at) };
        let mut rank = at.context.ranking.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(top) = rank.take_top(at.hour) {
            return match top {
                Ok(leader) => one_leader(frame, &params.branch_id, leader, at),
                Err(missing) => {
                    let query = at.context.query(frame.branch)?;
                    Ok(empty_hour(missing, at.context.battles_partial(&query, at.hour)?, at.context.content.missing()))
                }
            };
        }
        let mut notes = super::ranking::Notes::default();
        let evaluated = UnitLeaders::evaluate_keys(frame, &top_asked(&params.branch_id, selection), at, None, Some(&mut notes))?;
        *rank = super::ranking::Ranking::seed(frame, &evaluated, at.context, notes);
        Self::from_evaluated(frame, params, evaluated, at)
    }
}

impl LeaderHour {
    fn from_evaluated(frame: &DecodedFrame, params: &LeaderHourParams, evaluated: super::EvaluatedLeaders<'_>, at: &At<'_, LeadersContext>) -> io::Result<LeaderHourReply> {
        let missing = match evaluated.top() {
            TopLeader::Winner(winner) => {
                let branch = parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?;
                let query = at.context.query(branch)?;
                return finish_one_leader(
                    frame,
                    &query,
                    winner,
                    evaluated.tally.as_ref(),
                    evaluated.battles_partial_from,
                    evaluated.warnings.clone(),
                    at,
                );
            }
            TopLeader::None => None,
            TopLeader::Missing(reason) => Some(reason),
        };
        Ok(empty_hour(missing, evaluated.battles_partial_from, evaluated.warnings))
    }
}

fn empty_hour(missing: Option<String>, battles_partial_from: Option<i32>, warnings: Vec<String>) -> LeaderHourReply {
    LeaderHourReply {
        reference: None,
        missing,
        name: None,
        role: None,
        skill: None,
        components: [0; 4],
        traits: None,
        trait_dates: Vec::new(),
        trait_catalogue: Vec::new(),
        fought: 0,
        won: 0,
        lost: 0,
        battles_partial_from,
        first_seen_hour: None,
        service_start: None,
        progress: None,
        warnings,
    }
}

fn service_start(query: &Query<'_>, leader: u64, hour: i32) -> io::Result<Option<LeaderServiceStart>> {
    use replay_columnar::manifest::Status;
    use replay_recording::columnar_frames::TableCoverage;

    let recording = query.replay.recording();
    let key = u128::from(leader);
    let mut first: Option<i32> = None;
    for stretch in query.timeline.stretches.iter().rev() {
        if stretch.first > hour {
            continue;
        }
        if let Some(current_first) = first {
            if stretch.last < current_first - 1 {
                break;
            }
        } else if stretch.last < hour {
            return Ok(None);
        }
        if matches!(recording.table_coverage(TableId::UnitLeaders.index(), stretch.run), TableCoverage::WithheldAll(_)) {
            break;
        }
        let target = first.map_or(hour, |at| at - 1).min(stretch.last);
        let spans = dashboard_query::cost::storage(|| {
            replay_lanes::lanes::presences(recording, stretch.run, TableId::UnitLeaders, (key, key), i64::from(stretch.first), i64::from(target))
        })?;
        let found = spans
            .into_iter()
            .find(|(held, _)| *held == key)
            .and_then(|(_, spans)| spans.into_iter().rev().find(|&(a, b)| a <= i64::from(target) && i64::from(target) <= b));
        let Some((span_first, _)) = found else {
            break;
        };
        let span_first = i32::try_from(span_first).unwrap_or(stretch.first).max(stretch.first);
        first = Some(span_first);
        if span_first > stretch.first {
            break;
        }
    }
    let Some(first) = first else { return Ok(None) };
    if first <= query.timeline.first {
        return Ok(Some(LeaderServiceStart::Before { hour: first }));
    }
    let prev = first - 1;
    let Some(prev_stretch) = query.timeline.at(prev) else {
        return Ok(Some(LeaderServiceStart::Unknown {
            reason: "Unit leaders were not recorded before this leader was observed".into(),
        }));
    };
    if matches!(recording.table_coverage(TableId::UnitLeaders.index(), prev_stretch.run), TableCoverage::WithheldAll(_)) {
        return Ok(Some(LeaderServiceStart::Unknown {
            reason: "Unit leaders were withheld before this leader was observed".into(),
        }));
    }
    let runs = dashboard_query::cost::storage(|| replay_lanes::lanes::statuses(recording, prev_stretch.run, TableId::UnitLeaders, prev, prev))?;
    let status = runs.first().map(|run| run.status).unwrap_or(Status::NotRecorded);
    let reason = match status {
        Status::Complete => return Ok(Some(LeaderServiceStart::Since { hour: first })),
        Status::Truncated { .. } => "Unit leaders were only partly recorded before this leader was observed",
        Status::Unavailable { .. } => "Unit leaders were unavailable before this leader was observed",
        Status::NotRecorded => "Unit leaders were not recorded before this leader was observed",
    };
    Ok(Some(LeaderServiceStart::Unknown { reason: reason.into() }))
}

fn one_leader(frame: &DecodedFrame, branch_id: &str, leader: u64, at: &At<'_, LeadersContext>) -> io::Result<LeaderHourReply> {
    let context = at.context;
    let content = &context.content;
    let row = match frame.complete::<tables::UnitLeaders>() {
        Ok(rows) => match rows.iter().find(|row| handle(row) == leader) {
            Some(row) => row,
            None => {
                return Ok(LeaderHourReply {
                    reference: Some(leader.to_string()),
                    missing: Some("This leader is not serving at this hour".into()),
                    name: None,
                    role: None,
                    skill: None,
                    components: [0; 4],
                    traits: None,
                    trait_dates: Vec::new(),
                    trait_catalogue: Vec::new(),
                    fought: 0,
                    won: 0,
                    lost: 0,
                    battles_partial_from: None,
                    first_seen_hour: None,
                    service_start: None,
                    progress: None,
                    warnings: content.missing(),
                });
            }
        },
        Err(error) => {
            return Ok(LeaderHourReply {
                reference: Some(leader.to_string()),
                missing: Some(error.to_string()),
                name: None,
                role: None,
                skill: None,
                components: [0; 4],
                traits: None,
                trait_dates: Vec::new(),
                trait_catalogue: Vec::new(),
                fought: 0,
                won: 0,
                lost: 0,
                battles_partial_from: None,
                first_seen_hour: None,
                service_start: None,
                progress: None,
                warnings: content.missing(),
            });
        }
    };
    let branch = parse_branch(branch_id).map_err(dashboard_core::invalid)?;
    let query = context.query(branch)?;
    let owner = Tags::read(frame).ok().and_then(|tags| tags.tag(row.owner));
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    let person = frame.complete::<tables::Characters>().ok().and_then(|rows| {
        rows.binary_search_by_key(&row.character_id, |character| character.id)
            .ok()
            .or_else(|| rows.iter().position(|character| character.id == row.character_id))
            .map(|at| &rows[at])
    });
    let name = content.person_name(person, lexical);
    let skill = (row.skill_valid != 0).then_some(row.skill_level);
    let tally = context.tally(&query, frame, Asked::Id(row.id))?;
    let battles = tally.battles(row.id);
    let battles_partial_from = context.battles_partial(&query, at.hour)?;
    let evaluated = EvaluatedLeader { row, handle: leader, owner, name, skill, battles };
    finish_one_leader(frame, &query, &evaluated, Some(&tally), battles_partial_from, content.missing(), at)
}

fn finish_one_leader(
    frame: &DecodedFrame,
    query: &Query<'_>,
    evaluated: &EvaluatedLeader<'_>,
    tally: Option<&Tally>,
    battles_partial_from: Option<i32>,
    warnings: Vec<String>,
    at: &At<'_, LeadersContext>,
) -> io::Result<LeaderHourReply> {
    let context = at.context;
    let content = &context.content;
    let row = evaluated.row;
    let leader = evaluated.handle;
    let first_seen_hour = query.timeline.at(at.hour).and_then(|stretch| context.first_seen(stretch.run, u128::from(leader), at.hour));
    let start = service_start(query, leader, at.hour)?;
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    let tokens = frame.complete::<tables::UnitLeaderTraits>().ok().map(|rows| held_traits_for(rows, leader));
    let dates: Vec<Option<(i32, u8)>> = tokens
        .iter()
        .flatten()
        .map(|token| tally.and_then(|tally| tally.date(leader, token)))
        .collect();
    let mut traits = TraitList::default();
    let trait_indices = tokens.map(|tokens| tokens.iter().map(|token| traits.index(token, content)).collect());
    let spelled = |token: i32| -> Option<String> {
        let token = u32::try_from(token).ok()?;
        let rows = lexical?;
        rows.binary_search_by_key(&token, |row| row.token).ok().map(|at| String::from_utf8_lossy(&rows[at].literal).into_owned())
    };
    let experience = field(TableId::UnitLeaders, "experience")?;
    at.live.value(&leader.to_string(), "experience", Cell { table: TableId::UnitLeaders, column: experience, key: u128::from(leader) }, 1.0);
    let next_required_raw = evaluated.skill.and_then(|level| content.next_required(level, row.category)).map(|value| value.to_string());
    let (training, factors) = match frame.complete::<tables::UnitLeaderTraitExperience>() {
        Ok(rows) => {
            let trained = field(TableId::UnitLeaderTraitExperience, "experience")?;
            let mut training = Vec::new();
            let mut factors = Vec::new();
            for row in rows.iter().filter(|row| row.leader_reference == leader) {
                let token = spelled(row.token_id);
                let name = token.as_deref().and_then(|token| content.trait_name(token));
                if row.list == 1 {
                    factors.push(LeaderFactor { token, name, factor_raw: row.experience.to_string() });
                } else {
                    let key = (u128::from(row.leader_reference) << 33) | (u128::from(row.list) << 32) | u128::from(row.ordinal);
                    at.live.value(&key.to_string(), "experience", Cell { table: TableId::UnitLeaderTraitExperience, column: trained, key }, 1.0);
                    let required_raw = token.as_deref().and_then(|token| content.trait_required(token)).map(|value| value.to_string());
                    training.push(LeaderTraining { key: key.to_string(), token, name, required_raw });
                }
            }
            (Some(training), Some(factors))
        }
        Err(_) => (None, None),
    };
    Ok(LeaderHourReply {
        reference: Some(leader.to_string()),
        missing: None,
        name: evaluated.name.clone(),
        role: Some(content.role(evaluated.owner.clone(), row.kind, row.category, evaluated.skill)),
        skill: evaluated.skill,
        components: row.components,
        traits: trait_indices,
        trait_dates: dates,
        trait_catalogue: traits.traits,
        fought: evaluated.battles.fought,
        won: evaluated.battles.won,
        lost: evaluated.battles.lost,
        battles_partial_from,
        first_seen_hour,
        service_start: start,
        progress: Some(LeaderProgress { next_required_raw, training, factors }),
        warnings,
    })
}

fn top_asked(branch_id: &str, selection: &LeaderSelection) -> UnitLeadersParams {
    UnitLeadersParams { branch_id: branch_id.to_owned(), selection: selection.clone(), leaders: LeaderKinds::Land }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaderChoicesParams {
    pub branch_id: String,
    pub selection: super::LeaderSelection,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderChoicesReply {
    pub choices: Vec<(String, Option<String>, Option<String>, i32)>,
    pub warnings: Vec<String>,
}

pub struct LeaderChoices;

impl Product for LeaderChoices {
    const KEY: &'static str = "unit-leader-choices";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = LeaderChoicesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, selection, "selection.countries[]"];
    type Reply = LeaderChoicesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderChoicesParams::decl(), LeaderChoicesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        match params.selection {
            LeaderSelection::Group { .. } => Err("Branch-wide leader choices need countries or all leaders"),
            _ => Ok(()),
        }
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?)?;
        let content = Content::load(source, load);
        let names = Names::new(&query.holders);
        let recording = query.replay.recording();
        let (owner, character, kind) = (field(TableId::UnitLeaders, "owner")?, field(TableId::UnitLeaders, "character_id")?, field(TableId::UnitLeaders, "kind")?);
        let mut leader_columns = [owner, character, kind];
        leader_columns.sort_unstable();
        let token = field(TableId::Characters, "token")?;
        let mut last: BTreeMap<u128, (u32, i32)> = BTreeMap::new();
        for stretch in &query.timeline.stretches {
            for &(key, first, end) in dashboard_base::queries::lifetimes(source, stretch.run, TableId::UnitLeaders)?.iter() {
                let hour = end.min(i64::from(stretch.last));
                if first <= i64::from(stretch.last) && hour >= i64::from(stretch.first) {
                    last.insert(key, (stretch.run, hour as i32));
                }
            }
        }
        let mut by_run: BTreeMap<u32, Vec<(i32, u128)>> = BTreeMap::new();
        for (&key, &(run, hour)) in &last {
            by_run.entry(run).or_default().push((hour, key));
        }
        let mut choices = Vec::with_capacity(last.len());
        for (run, mut asks) in by_run {
            dashboard_text::metadata::check_cancel(cancelled)?;
            asks.sort_unstable();
            let links = dashboard_base::queries::links(source, run)?;
            let mut people: Vec<(i32, u128, u128, u32, u8)> = Vec::new();
            replay_lanes::lanes::at_each(recording, run, TableId::UnitLeaders, Some(&leader_columns), &asks, &mut |hour, key, row| {
                if let Some((view, slot)) = row
                    && let (Some(raw), Some(id), Some(leader_kind)) = (view.number(owner, slot), view.number(character, slot), view.number(kind, slot))
                {
                    people.push((hour, key, id as u128, raw as u32, leader_kind as u8));
                }
                Ok(())
            })?;
            let mut named: HashMap<(i32, u128), String> = HashMap::new();
            let mut character_asks: Vec<(i32, u128)> = people.iter().map(|person| (person.0, person.2)).collect();
            character_asks.sort_unstable();
            character_asks.dedup();
            replay_lanes::lanes::at_each(recording, run, TableId::Characters, Some(&[token]), &character_asks, &mut |hour, key, row| {
                if let Some((view, slot)) = row
                    && let Some(saved) = dashboard_query::read::text(&view, token, slot).and_then(|bytes| std::str::from_utf8(bytes).ok())
                    && let Some(name) = content.character_name(saved)
                {
                    named.insert((hour, key), name);
                }
                Ok(())
            })?;
            for (hour, key, character, raw, leader_kind) in people {
                if leader_kind != super::KIND_ARMY && leader_kind != super::KIND_NAVY {
                    continue;
                }
                let tag = names.tag_at(&links, raw, hour).map(str::to_owned);
                if let super::LeaderSelection::Countries { countries } = &params.selection
                    && !tag.as_ref().is_some_and(|tag| countries.iter().any(|country| query.holders.goes_by(country, tag, hour)))
                {
                    continue;
                }
                choices.push((key.to_string(), named.get(&(hour, character)).cloned(), tag, hour));
            }
        }
        choices.sort_by_key(|choice| choice.0.parse::<u64>().unwrap_or_default());
        Ok(LeaderChoicesReply { choices, warnings: content.missing() })
    }
}
