
use super::{
    Asked, Battles, Content, LeaderRole, LeaderTrait, LeadersContext, RESULTS_MISSING, TraitList, cell_number, cell_text, handle, held_traits, parse_branch,
};
use dashboard_base::{Source, products::{LoadContext, Product, at_hour::{At, AtHour, Keys, Read}}, queries::country_ids::{Names, RunLinks}};
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
    pub reference: String,
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
    pub reference: String,
    pub name: Option<String>,
    #[doc = " The hour the name and portrait are read at: the leader's last on the timeline."]
    pub portrait_hour: Option<i32>,
    pub runs: Vec<CareerRun>,
    #[doc = " [hour, place in `traits`, class]: 0 held before, 1 gained, 2 first observed."]
    pub trait_gains: Vec<(i32, u32, u8)>,
    pub traits: Vec<LeaderTrait>,
    pub commands: Vec<LeaderCommand>,
    pub battles: Vec<CareerBattle>,
    #[doc = " The timeline's recorded stretches: a battle lane is drawn only over them."]
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
    let filter = FacetFilter { name: "leader".into(), values: vec![i64::from(id)], countries: Vec::new() };
    let (first, last) = (query.timeline.first, query.timeline.last);
    let asked = reader.select(&[filter], first, last)?;
    let rows = reader.every(&asked.patterns, first, last)?.into_iter().map(|(_, row)| row).filter(|row| (row.key >> 64) as u64 == leader).collect();
    Ok((reader.columns(), rows))
}

fn column(columns: &[&str], name: &str) -> io::Result<usize> {
    columns.iter().position(|column| *column == name).ok_or_else(|| dashboard_core::invalid(format!("A leader log has no {name}")))
}

impl Product for LeaderCareer {
    const KEY: &'static str = "unit-leader-career";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = LeaderCareerParams;
    type Reply = LeaderCareerReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderCareerParams::decl(), CareerRun::decl(), CareerBattle::decl(), CommandKind::decl(), LeaderCommand::decl(), LeaderCareerReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        reference(&params.reference).map(|_| ())
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let leader = reference(&params.reference).map_err(dashboard_core::invalid)?;
        let key = u128::from(leader);
        let query = dashboard_base::queries::query(source, parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?)?;
        let content = Content::load(source, load);
        let recording = query.replay.recording();
        let names = Names::new(&query.holders);
        let run_columns = RunColumns::new()?;
        let mut runs = Vec::new();
        let mut last_seen: Option<(u32, i32)> = None;
        let mut commands: Vec<LeaderCommand> = Vec::new();
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let links: std::sync::Arc<RunLinks> = dashboard_base::queries::links(source, stretch.run)?;
            let mut ascending = run_columns.columns.clone();
            ascending.sort_unstable();
            let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, stretch.run, TableId::UnitLeaders, &ascending, LaneKeys::These(&[key]), stretch.first, stretch.last))?;
            let steps: Vec<(i32, Option<RunValues>)> = history
                .keys
                .iter()
                .filter(|(held, _)| *held == key)
                .flat_map(|(_, steps)| steps.iter().map(|step| (step.hour, step.cells.as_deref().and_then(|cells| values_of(&history, cells, &run_columns.columns)))))
                .collect();
            for (first, last, values) in career_runs(&steps, history.to) {
                last_seen = Some((stretch.run, last));
                for (tag, a, b) in names.tags_over(&links, values.owner, first, last) {
                    runs.push(CareerRun { first_hour: a, last_hour: b, role: content.role(tag.map(str::to_owned), values.kind, values.category, values.skill), skill: values.skill, components: values.components });
                }
            }
            let open = dashboard_query::cost::storage(|| replay_lanes::lanes::at(recording, stretch.run, TableId::LeaderCommandSpans, stretch.last, None))?;
            let view = open.view();
            for slot in view.rows() {
                if view.number(0, slot).map(|held| held as u64) != Some(leader) {
                    continue;
                }
                let (Some(key), Some(first), Some(kind)) = (view.key(slot), view.number(1, slot), view.number(2, slot)) else { continue };
                let name = view.list(3, slot).and_then(|bytes| stored_name(&content, bytes));
                commands.push(LeaderCommand { first_hour: (first as i32).max(stretch.first), last_hour: stretch.last, formation: (key as u64).to_string(), kind: command_kind(kind as u8), name });
            }
        }
        let (command_columns, command_rows) = leader_rows(&query, "leader-commands", (leader >> 32) as u32, leader)?;
        let at = |name: &str| column(&command_columns, name);
        let (formation, kind, first, last, name) = (at("formation")?, at("kind")?, at("first_hour")?, at("last_hour")?, at("name")?);
        for row in &command_rows {
            let Some(stretch) = query.timeline.at(row.hour) else { continue };
            let number = |at: usize| cell_number(&row.cells[at]);
            let (Some(formation), Some(kind), Some(first), Some(last)) = (number(formation), number(kind), number(first), number(last)) else { continue };
            let name = match &row.cells[name] {
                dashboard_query::log::Cell::List(bytes) => stored_name(&content, bytes),
                _ => None,
            };
            commands.push(LeaderCommand { first_hour: (first as i32).max(stretch.first), last_hour: last as i32, formation: (formation as u64).to_string(), kind: command_kind(kind as u8), name });
        }
        commands.sort_by_key(|command| (command.first_hour, command.formation.clone()));
        let id = (leader >> 32) as u32;
        let (gain_columns, gain_rows) = leader_rows(&query, "leader-trait-gains", id, leader)?;
        let (class, spelling) = (column(&gain_columns, "class")?, column(&gain_columns, "spelling")?);
        let gains: Vec<(i32, String, u8)> = gain_rows
            .iter()
            .filter_map(|row| Some((row.hour, cell_text(&row.cells[spelling])?, cell_number(&row.cells[class])? as u8)))
            .collect();
        let mut traits = TraitList::default();
        let trait_gains = career_gains(&gains).into_iter().map(|(hour, token, class)| (hour, traits.index(&token, &content), class)).collect();
        let (battle_columns, battle_rows) = leader_rows(&query, "leader-battles", id, leader)?;
        let at = |name: &str| column(&battle_columns, name);
        let (entry, kind, combat, start, unknown, outcome, result_hour) = (at("entry")?, at("kind")?, at("combat")?, at("start_hour")?, at("start_unknown")?, at("outcome")?, at("result_hour")?);
        let battles: Vec<BattleRow> = battle_rows
            .iter()
            .filter_map(|row| {
                let number = |at: usize| cell_number(&row.cells[at]);
                Some((row.hour, number(entry)? as u8, number(kind)? as u8, number(combat)? as u64, number(start)? as i32, number(unknown)? != 0, number(outcome).map(|value| value as u8), number(result_hour).map(|value| value as i32)))
            })
            .collect();
        let (name, portrait_hour, missing) = match last_seen {
            Some((run, hour)) => {
                let leaders = [key];
                let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &[replay_lanes::at::Read { table: TableId::UnitLeaders, columns: None, keys: replay_lanes::at::Keys::These(&leaders) }, replay_lanes::at::Read::all(TableId::LexicalTokens)]))?;
                let character = frame.complete::<tables::UnitLeaders>().ok().and_then(|rows| rows.first().map(|row| u128::from(row.character_id)));
                let characters: Vec<u128> = character.into_iter().collect();
                let people = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &[replay_lanes::at::Read { table: TableId::Characters, columns: None, keys: replay_lanes::at::Keys::These(&characters) }]))?;
                let name = content.person_name(people.complete::<tables::Characters>().ok().and_then(|rows| rows.first()), frame.complete::<tables::LexicalTokens>().ok());
                (name, Some(hour), None)
            }
            None => (None, None, Some("This leader is not recorded on this timeline".to_owned())),
        };
        Ok(LeaderCareerReply {
            reference: params.reference.clone(),
            name,
            portrait_hour,
            runs,
            trait_gains,
            traits: traits.traits,
            commands,
            battles: join_battles(&battles),
            observed: query.timeline.stretches.iter().map(|stretch| (stretch.first, stretch.last)).collect(),
            battles_partial_from: super::battles_partial_from(&query, query.timeline.first, query.timeline.last)?,
            results_missing: RESULTS_MISSING.into(),
            missing,
            warnings: content.missing(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaderHourParams {
    pub branch_id: String,
    pub reference: String,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderHourReply {
    pub reference: String,
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
    pub progress: Option<LeaderProgress>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LeaderHourReply, LeaderCareerReply, LeaderChoicesReply);

pub struct LeaderHour;

impl AtHour for LeaderHour {
    const KEY: &'static str = "unit-leader-hour";
    const TITLE: &'static str = "General career";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LeaderHourParams;
    type Reply = LeaderHourReply;
    type Context = LeadersContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderHourParams::decl(), LeaderTraining::decl(), LeaderFactor::decl(), LeaderProgress::decl(), LeaderHourReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        reference(&params.reference).map(|_| ())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![
            TableId::UnitLeaders,
            TableId::UnitLeaderTraits,
            TableId::UnitLeaderTraitExperience,
            TableId::Characters,
            TableId::LexicalTokens,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LeaderBattleTotals,
            TableId::LeaderTraitDates,
        ]
    }

    fn reads(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Ok(leader) = reference(&params.reference) else { return Vec::new() };
        let key = u128::from(leader);
        match step {
            0 => vec![
                Read::these(TableId::UnitLeaders, [key]),
                Read { table: TableId::UnitLeaderTraits, keys: Keys::Range(key << 32, (key << 32) | u128::from(u32::MAX)) },
                Read { table: TableId::UnitLeaderTraitExperience, keys: Keys::Range(key << 33, (key << 33) | ((1u128 << 33) - 1)) },
                Read::all(TableId::LexicalTokens),
                Read::all(TableId::Countries),
                Read::all(TableId::CountryLinks),
            ],
            1 => held.complete::<tables::UnitLeaders>().ok().and_then(|rows| rows.first()).map(|row| vec![Read::these(TableId::Characters, [u128::from(row.character_id)])]).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        LeadersContext::load(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let context = at.context;
        let content = &context.content;
        let leader = reference(&params.reference).map_err(dashboard_core::invalid)?;
        let mut reply = LeaderHourReply {
            reference: params.reference.clone(),
            missing: None,
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
            progress: None,
            warnings: content.missing(),
        };
        let row = match frame.complete::<tables::UnitLeaders>() {
            Ok(rows) => match rows.iter().find(|row| handle(row) == leader) {
                Some(row) => row,
                None => {
                    reply.missing = Some("This leader is not serving at this hour".into());
                    return Ok(reply);
                }
            },
            Err(error) => {
                reply.missing = Some(error.to_string());
                return Ok(reply);
            }
        };
        let branch = parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?;
        let query = context.query(branch)?;
        let owner = Tags::read(frame).ok().and_then(|tags| tags.tag(row.owner));
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        reply.name = content.person_name(frame.complete::<tables::Characters>().ok().and_then(|rows| rows.first()), lexical);
        reply.skill = (row.skill_valid != 0).then_some(row.skill_level);
        reply.role = Some(content.role(owner, row.kind, row.category, reply.skill));
        reply.components = row.components;
        let tokens = frame.complete::<tables::UnitLeaderTraits>().ok().map(|rows| held_traits(rows).remove(&leader).unwrap_or_default());
        let tally = context.tally(&query, at.hour, Asked::Id(row.id))?;
        let (battles, dates): (Battles, Vec<Option<(i32, u8)>>) = (tally.battles(row.id), tokens.iter().flatten().map(|token| tally.date(leader, token)).collect());
        let mut traits = TraitList::default();
        reply.traits = tokens.map(|tokens| tokens.iter().map(|token| traits.index(token, content)).collect());
        reply.trait_catalogue = traits.traits;
        reply.trait_dates = dates;
        (reply.fought, reply.won, reply.lost) = (battles.fought, battles.won, battles.lost);
        reply.battles_partial_from = super::battles_partial_from(&query, query.timeline.first, at.hour)?;
        let spelled = |token: i32| -> Option<String> {
            let token = u32::try_from(token).ok()?;
            let rows = lexical?;
            rows.binary_search_by_key(&token, |row| row.token).ok().map(|at| String::from_utf8_lossy(&rows[at].literal).into_owned())
        };
        let experience = field(TableId::UnitLeaders, "experience")?;
        at.live.value(&params.reference, "experience", Cell { table: TableId::UnitLeaders, column: experience, key: u128::from(leader) }, 1.0);
        let next_required_raw = reply.skill.and_then(|level| content.next_required(level, row.category)).map(|value| value.to_string());
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
        reply.progress = Some(LeaderProgress { next_required_raw, training, factors });
        Ok(reply)
    }
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
    type Reply = LeaderChoicesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderChoicesParams::decl(), LeaderChoicesReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id).map(|_| ())
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
                if !super::LeaderKinds::All.includes(leader_kind) {
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
