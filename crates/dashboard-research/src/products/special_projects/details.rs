
use dashboard_effects::ScriptedEffects;
use dashboard_base::products::Country;
use super::{CHARACTER_REFERENCE, Naming, PROGRAM_REFERENCE, ProjectContent, ProjectPerson, RunLinks, country_key, needs_scope};
use dashboard_base::{Source, catalogue::View, products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read, ScriptedInputs}, game_text::{GameRun, runs}}};
use dashboard_query::{Index, Query, cost};
use dashboard_text::triggers::{AcceptedProjects, ProjectReads, ProjectScope, inputs::TableRead};
use replay::campaign::Replay;
use replay_lanes::lanes;
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{CharacterRoleTraitRow, ProjectHistoryRow, ProjectRow, ProjectRuntime, Recorded, ScientistDailyRow, SliceReadRow, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, OnceLock, atomic::{AtomicI32, Ordering}},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

mod facility;
mod iteration;
#[cfg(test)]
mod tests;
mod visibility;

pub use facility::{FacilityDetails, FacilitySource};
pub use iteration::SpecialProjectIterationAt;

const COMPLETION_MARKER: u32 = super::facts::COMPLETION_MARKER;

pub struct SpecialProjectDetailsAt;
pub struct SpecialProjectEffectsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialProjectDetailsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialProjectEffectsParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The project's raw owner id and its id within it."]
    pub owner: u32,
    pub project: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectLabel {
    pub token: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScientistSkill {
    pub specialization: ProjectLabel,
    pub level: i32,
    pub experience_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScientistDetails {
    pub character: u32,
    pub person: ProjectPerson,
    pub active: Option<u8>,
    pub injury_days: Option<i32>,
    #[doc = " The read `injury_days` comes from."]
    pub injury_hour: Option<i32>,
    pub visible: Option<bool>,
    pub visibility_hour: Option<i32>,
    pub state_refusals: BTreeMap<String, usize>,
    pub skills: Option<Vec<ScientistSkill>>,
    pub traits: Option<Vec<ProjectLabel>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScientistAssignment {
    pub ordinal: u32,
    pub reference: String,
    pub person: ProjectPerson,
    pub target: i32,
    pub current: i32,
    pub timed: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProgramDetails {
    pub id: u32,
    pub province: u32,
    pub read_hour: i32,
    pub assignments: Option<Vec<ScientistAssignment>>,
    pub facility: Option<FacilityDetails>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRewardOption {
    pub label: ProjectLabel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PendingProjectReward {
    pub label: ProjectLabel,
    #[doc = " The reward's occurrence in the project's definition; none where it was not recorded."]
    pub ordinal: Option<u32>,
    pub description: Option<String>,
    pub options: Vec<ProjectRewardOption>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectVariableValue {
    pub name: String,
    pub value_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectChoice {
    pub ordinal: u32,
    pub date: i32,
    #[doc = " A completion follows it in the history read: an earlier attempt's choice."]
    pub earlier: bool,
    pub reward: ProjectLabel,
    pub option: ProjectLabel,
    pub province: i32,
    pub progress: i32,
    pub character_reference: String,
    pub country: Option<u32>,
    pub scientist: Option<u32>,
    pub state: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecializationProgress {
    pub specialization: ProjectLabel,
    pub points: i32,
    pub read_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuntimeDetails {
    pub owner: u32,
    pub id: u32,
    pub label: ProjectLabel,
    pub read_hour: i32,
    pub pending_known: bool,
    pub pending: Option<PendingProjectReward>,
    pub seed: Option<u32>,
    pub variables: Option<Vec<ProjectVariableValue>>,
    pub choices: Option<Vec<ProjectChoice>>,
    #[doc = " The choice-history read `choices` comes from; none where they are not recorded."]
    pub history_read_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectDetails {
    #[serde(skip)]
    through: i32,
    pub hour: i32,
    #[doc = " None where the hour's scientist roster is not known (never an empty one)."]
    pub scientists: Option<Vec<ScientistDetails>>,
    #[doc = " None without a read of the programs in the day up to the hour."]
    pub programs: Option<Vec<ProgramDetails>>,
    #[doc = " None without a read of the progress in the day up to the hour."]
    pub breakthroughs: Option<Vec<SpecializationProgress>>,
    #[doc = " None without a read of the country's projects in the day up to the hour."]
    pub projects: Option<Vec<ProjectRuntimeDetails>>,
    pub unavailable: Vec<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(SpecialProjectDetails);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectEffects {
    pub runs: Vec<GameRun>,
    pub refusals: BTreeMap<String, usize>,
}

impl From<dashboard_effects::effect_description::Description> for ProjectEffects {
    fn from(mut description: dashboard_effects::effect_description::Description) -> Self {
        let runs = runs(&description.text).unwrap_or_else(|why| {
            *description.refusals.entry(why).or_default() += 1;
            Vec::new()
        });
        Self { runs, refusals: description.refusals }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectEffects {
    #[serde(skip)]
    through: i32,
    pub hour: i32,
    #[doc = " The project read every effect is rendered from; none where no read supplies it."]
    pub read_hour: Option<i32>,
    #[doc = " None where the project's definition is not in the recorded content."]
    pub output: Option<ProjectEffects>,
    #[doc = " None with no reward pending, or where it could not be read."]
    pub pending: Option<PendingEffects>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PendingEffects {
    #[doc = " The pending reward's token and its occurrence in the project's definition."]
    pub reward: String,
    pub ordinal: u32,
    #[doc = " Each option, in the reward's order."]
    pub options: Vec<OptionEffects>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OptionEffects {
    pub token: String,
    pub effects: ProjectEffects,
}

dashboard_base::reply_warnings!(SpecialProjectEffects);

pub struct Context {
    text_inputs: ScriptedInputs,
    content: Result<Arc<ProjectContent>, String>,
    replay: Arc<Replay>,
    index: Arc<Index>,
    visibility: visibility::Cache,
    scoped_details: std::sync::atomic::AtomicBool,
    coverage: std::sync::Mutex<HashMap<(u32, TableId), Coverage>>,
    links: Links,
}

type Links = std::sync::Mutex<HashMap<u32, Arc<RunLinks>>>;

fn context(source: &Source, load: &LoadContext) -> Context {
    Context {
        text_inputs: ScriptedInputs::new(source.replay()),
        content: super::content(source, load).map_err(|error| format!("Recorded special-project content is unavailable: {error}")),
        replay: source.replay(),
        index: dashboard_base::queries::index(source),
        visibility: visibility::Cache::default(),
        scoped_details: false.into(),
        coverage: Default::default(),
        links: Default::default(),
    }
}

fn branch(id: &str) -> io::Result<u64> {
    id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

fn spelled(frame: &DecodedFrame, id: u32) -> Result<String, String> {
    let rows = frame.complete::<tables::LexicalTokens>().map_err(|why| format!("lexical tokens: {why}"))?;
    let row = rows.binary_search_by_key(&id, |row| row.token).ok().map(|at| &rows[at]).ok_or_else(|| format!("lexical tokens lacks token {id}"))?;
    std::str::from_utf8(content_reader_parser::parser::c_string(&row.literal)).map(str::to_owned).map_err(|_| format!("Token {id} is not UTF-8"))
}


struct Coverage {
    block: u32,
    reads: Vec<(i32, i32)>,
    whole_reads: Vec<(i32, i32)>,
    whole: Vec<(i32, i32)>,
    parts: Vec<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Withheld {
    Unread,
    Partial,
    Tag,
    Owner,
}

struct Reads<'r, 'q> {
    through: Option<&'r AtomicI32>,
    query: &'r Query<'q>,
    coverage: &'r std::sync::Mutex<HashMap<(u32, TableId), Coverage>>,
    links: &'r Links,
    tag: &'r Country,
    holder: Owner,
    run: u32,
    hour: i32,
}

impl Reads<'_, '_> {
    fn hold_through(&self, through: i32) {
        if let Some(bound) = self.through { bound.fetch_min(through, Ordering::Relaxed); }
    }

    fn spans<T>(&self, table: TableId, what: &str, of: impl FnOnce(&Coverage) -> T) -> Result<T, String> {
        let recording = self.query.replay.recording();
        let run = recording.run(self.run).map_err(|e| e.to_string())?;
        let block = run.ordinal(self.hour).ok_or_else(|| format!("{what}: hour not recorded"))? / replay_columnar::unit::BLOCK;
        let last = run.block_first_hour(block).saturating_add(replay_columnar::unit::BLOCK as i32 - 1).min(run.last_hour());
        self.hold_through(last);
        let mut cache = self.coverage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (self.run, table);
        if cache.get(&key).is_none_or(|c| c.block != block) {
            let first = run.block_first_hour(block).saturating_sub(STAGGER_PERIOD - 1);
            let reads = cost::storage(|| lanes::available_reads(recording, self.run, table, first, last)).map_err(|e| format!("{what}: {e}"))?;
            let whole_reads = cost::storage(|| lanes::complete_reads(recording, self.run, table, first, last)).map_err(|e| format!("{what}: {e}"))?;
            let statuses = cost::storage(|| lanes::statuses(recording, self.run, table, first, last)).map_err(|e| format!("{what}: {e}"))?;
            let spans = |kept: fn(&replay_columnar::manifest::Status) -> bool| statuses.iter().filter(|run| kept(&run.status)).map(|run| (run.first, run.last)).collect();
            let whole = spans(|status| *status == replay_columnar::manifest::Status::Complete);
            let parts = spans(|status| matches!(status, replay_columnar::manifest::Status::Truncated { .. }));
            cache.insert(key, Coverage { block, reads, whole_reads, whole, parts });
        }
        Ok(of(&cache[&key]))
    }

    fn latest(&self, spans: &[(i32, i32)]) -> Option<i32> {
        let (last, next) = super::last_read_next(spans, self.hour);
        if next != i32::MAX { self.hold_through(next.saturating_sub(1)); }
        last
    }

    fn last(&self, table: TableId, what: &str) -> Result<i32, String> {
        let read = self.spans(table, what, |coverage| self.latest(&coverage.reads))?.ok_or_else(|| format!("{what} were not read in the day up to this hour"))?;
        self.supplies(read).then_some(read).ok_or_else(|| format!("{what} were last read before this country held the tag"))
    }

    fn last_complete(&self, table: TableId, what: &str) -> Result<i32, String> {
        let read = self.spans(table, what, |coverage| self.latest(&coverage.whole_reads))?.ok_or_else(|| format!("{what} were not read in the day up to this hour"))?;
        self.supplies(read).then_some(read).ok_or_else(|| format!("{what} were last read before this country held the tag"))
    }

    fn supplies(&self, read: i32) -> bool {
        super::current(read, self.hour) && super::held_from(self.query, self.tag, self.holder.slot, self.holder.id, self.hour).is_some_and(|since| since <= read)
    }

    fn run_links(&self) -> Option<Arc<RunLinks>> {
        let mut cache = self.links.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(links) = cache.get(&self.run) {
            return Some(links.clone());
        }
        let links = Arc::new(cost::storage(|| super::country_ids::run_links(self.query.replay.recording(), self.run)).ok()?);
        cache.insert(self.run, links.clone());
        Some(links)
    }

    fn held_since(&self, owner: u32, from: i32) -> Result<(), Withheld> {
        super::held_from(self.query, self.tag, self.holder.slot, self.holder.id, self.hour).filter(|&since| since <= from).ok_or(Withheld::Tag)?;
        let links = self.run_links().ok_or(Withheld::Owner)?;
        (from <= self.hour && links.within(owner, from, self.hour).all(|(slot, _, _)| slot == Some(self.holder.slot))).then_some(()).ok_or(Withheld::Owner)
    }


    fn usable(&self, table: TableId, owner: u32, read: i32) -> Result<(), Withheld> {
        if self.hour < read {
            self.hold_through(read.saturating_sub(1));
        } else if super::current(read, self.hour) {
            self.hold_through(read.saturating_add(STAGGER_PERIOD - 1));
        }
        if !super::current(read, self.hour) {
            return Err(Withheld::Unread);
        }
        let (whole, part) = self.spans(table, "Project reads", |coverage| (super::within(&coverage.whole, read), super::within(&coverage.parts, read))).map_err(|_| Withheld::Unread)?;
        if !whole {
            return Err(if part { Withheld::Partial } else { Withheld::Unread });
        }
        self.held_since(owner, read)
    }

    fn continued(&self, previous: i32, table: TableId, what: &str) -> Option<Option<i32>> {
        let before = Self { hour: previous, through: None, ..*self };
        match (before.last(table, what), self.last(table, what)) {
            (Ok(_), Ok(hour)) => Some(Some(hour)),
            (Err(old), Err(new)) if old == new => Some(None),
            _ => None,
        }
    }
}

struct Namer<'c, 'r, 'q> {
    naming: Option<Naming<'c>>,
    missing: Option<String>,
    inputs: &'c ScriptedInputs,
    frame: &'r DecodedFrame,
    cancelled: &'q std::sync::atomic::AtomicBool,
    through: i32,
    slot: u32,
    scoped: HashMap<String, Option<String>>,
}

impl<'c, 'r, 'q> Namer<'c, 'r, 'q> {
    fn new(context: &'c Context, frame: &'r DecodedFrame, cancelled: &'q std::sync::atomic::AtomicBool, slot: u32) -> Self {
        let (naming, missing) = match &context.content {
            Ok(content) => (Some(Naming::new(content)), None),
            Err(why) => (None, Some(why.clone())),
        };
        Self { naming, missing, inputs: &context.text_inputs, frame, cancelled, through: i32::MAX, slot, scoped: HashMap::new() }
    }

    fn content(&self) -> Option<&'c ProjectContent> {
        self.naming.as_ref().map(|naming| naming.content)
    }

    fn name(&mut self, key: &str) -> Option<String> {
        let naming = self.naming.as_ref()?;
        let content = naming.content;
        if !needs_scope(content, key) {
            return content.politics.catalogue.localised_with_refusals(key, &[], View::Scripted, &naming.refusals);
        }
        if let Some(found) = self.scoped.get(key) {
            return found.clone();
        }
        let catalogue = &content.politics.catalogue;
        let found = catalogue.scripted_content().and_then(|scripted| {
            let (value, through) = self.inputs.evaluate(self.frame, dashboard_text::scripted_text::TABLES, self.cancelled, |world| {
                let text = dashboard_text::scripted_text::ScriptedText::new(world, &catalogue.localisation, scripted, self.slot);
                catalogue.localise_in(key, &text)
            }).map_err(|why| format!("The project's name needs its country's scope: {why}"))?;
            self.through = self.through.min(through);
            Ok(value)
        }).unwrap_or_else(|reason| {
            self.through = self.frame.hour;
            naming.note(reason, key);
            None
        });
        self.scoped.insert(key.to_owned(), found.clone());
        found
    }

    fn label(&mut self, token: String) -> ProjectLabel {
        ProjectLabel { name: self.name(&token), token }
    }

    fn named(&mut self, token: Option<String>, id: u32, key: Option<Option<String>>) -> ProjectLabel {
        match (token, key) {
            (None, _) => ProjectLabel { token: id.to_string(), name: None },
            (Some(token), None) => ProjectLabel { token, name: None },
            (Some(token), Some(key)) => ProjectLabel { name: self.name(key.as_deref().unwrap_or(&token)), token },
        }
    }

    fn token(&mut self, frame: &DecodedFrame, id: u32, unavailable: &mut Vec<String>) -> ProjectLabel {
        match spelled(frame, id) {
            Ok(token) => self.label(token),
            Err(why) => {
                unavailable.push(why);
                ProjectLabel { token: id.to_string(), name: None }
            }
        }
    }

    fn person(&mut self, frame: &DecodedFrame, reference: u64) -> ProjectPerson {
        let unread = |missing: Option<String>| ProjectPerson { assigned: Some(reference != 0), id: None, name: None, portrait: None, missing };
        if reference == 0 {
            return unread(None);
        }
        if reference as u32 != CHARACTER_REFERENCE {
            return unread(Some("Unknown scientist reference type".into()));
        }
        let id = (reference >> 32) as u32;
        match self.naming.as_mut() {
            Some(naming) => naming.person(frame, id),
            None => ProjectPerson { id: Some(id), ..unread(self.missing.clone()) },
        }
    }

    fn warnings(self) -> Vec<String> {
        self.naming.map(Naming::warnings).unwrap_or_default()
    }
}

fn slice_read(table: TableId, owner: u32) -> u128 {
    (table.index() as u128) << 32 | u128::from(owner)
}

fn project_slices(owner: u32) -> Read {
    Read::these(TableId::SliceReads, [slice_read(TableId::Projects, owner), slice_read(TableId::ProjectHistory, owner)])
}

fn owner_read(frame: &DecodedFrame, table: TableId, owner: u32) -> Result<i32, String> {
    let slices = frame.complete::<tables::SliceReads>().map_err(|why| format!("Project read hours are unavailable: {why}"))?;
    replay_data::sliced::read_hour(slices, table, owner).ok_or_else(|| PROJECTS_UNREAD.to_owned())
}

const OTHER_OWNER: &str = "The project's owner was not this country's from its read to this hour";
const PROJECTS_UNREAD: &str = "Projects were not read in the day up to this hour while this country held the tag";
const PROJECTS_PARTIAL: &str = "Projects were read only in part";
const HISTORY_UNREAD: &str = "Project choice history was not read in the day up to this hour while this country held the tag";
const HISTORY_PARTIAL: &str = "Project choice history was read only in part";

fn projects_read(reads: &Reads<'_, '_>, owner: u32, read: Option<i32>) -> Result<i32, String> {
    let read = read.ok_or(PROJECTS_UNREAD)?;
    reads.usable(TableId::Projects, owner, read).map_err(|why| match why {
        Withheld::Unread | Withheld::Tag => PROJECTS_UNREAD,
        Withheld::Partial => PROJECTS_PARTIAL,
        Withheld::Owner => OTHER_OWNER,
    })?;
    Ok(read)
}

fn stamped(reads: &Reads<'_, '_>, table: TableId, owner: u32) -> Result<(i32, DecodedFrame), String> {
    let (unread, partial, shift) = if table == TableId::Projects { (PROJECTS_UNREAD, PROJECTS_PARTIAL, 32) } else { (HISTORY_UNREAD, HISTORY_PARTIAL, 64) };
    let slices = dashboard_base::products::at_hour::frame(reads.query, reads.hour, &[project_slices(owner)], SpecialProjectDetailsAt::TITLE).map_err(|why| format!("Project read hours are unavailable: {why}"))?;
    let read = owner_read(&slices, table, owner).map_err(|_| unread.to_owned())?;
    reads.usable(table, owner, read).map_err(|why| match why {
        Withheld::Unread | Withheld::Tag => unread,
        Withheld::Partial => partial,
        Withheld::Owner => OTHER_OWNER,
    })?;
    let mut rows = dashboard_base::products::at_hour::frame(reads.query, read, &[Read { table, keys: owner_ranges(&BTreeSet::from([owner]), shift) }], SpecialProjectDetailsAt::TITLE)
        .map_err(|why| format!("The read at hour {read} is unavailable: {why}"))?;
    if table == TableId::Projects {
        let mut ids = BTreeSet::new();
        for row in rows.complete::<tables::Projects>().unwrap_or(&[]) {
            ids.insert(row.definition_id);
            if let Recorded::Value(runtime) = &row.runtime {
                ids.extend(runtime.pending_reward);
            }
        }
        let lexical = dashboard_base::products::at_hour::frame(reads.query, read, &[Read::these(TableId::LexicalTokens, ids.into_iter().map(u128::from))], SpecialProjectDetailsAt::TITLE)
            .map_err(|why| format!("Project spellings at hour {read} are unavailable: {why}"))?;
        rows.tables[TableId::LexicalTokens.index()] = lexical.tables[TableId::LexicalTokens.index()].clone();
    }
    Ok((read, rows))
}

fn stamped_project(reads: &Reads<'_, '_>, owner: u32, id: u32) -> Result<(ProjectRow, i32, DecodedFrame), String> {
    let (read, frame) = stamped(reads, TableId::Projects, owner)?;
    let rows = frame.complete::<tables::Projects>().map_err(|why| format!("Projects are unavailable: {why}"))?;
    let row = rows.binary_search_by_key(&(owner, id), |row| (row.owner, row.local_id)).ok().map(|at| rows[at].clone()).ok_or_else(|| "The project is not recorded at this hour".to_owned())?;
    Ok((row, read, frame))
}

fn supplied_project(reads: &Reads<'_, '_>, owner: u32, id: u32) -> Result<(ProjectRow, i32), String> {
    stamped_project(reads, owner, id).map(|(row, read, _)| (row, read))
}

fn with_projects<T>(reads: &Reads<'_, '_>, frame: &DecodedFrame, render: impl FnOnce(ProjectReads<'_>) -> T) -> (T, Vec<TableRead>) {
    let links = OnceLock::new();
    let found = Mutex::new(HashMap::new());
    let get = |scope| {
        let mut found = found.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (accepted, _) = found.entry(scope).or_insert_with(|| dashboard_text::triggers::inputs::unrecorded(|| {
            let links = links.get_or_init(|| frame.complete::<tables::CountryLinks>().ok().map(dashboard_text::metadata::link_slots));
            let mut link = TableRead::fields(TableId::CountryLinks, &["canonical_slot"]);
            if let ProjectScope::Owner(raw) = scope { link = link.row(u128::from(raw)); }
            let mut watched = vec![link];
            let accepted = links.as_ref().and_then(|links| accepted_projects(reads, frame, scope, links, &mut watched)).unwrap_or_default();
            (Arc::new(accepted), watched)
        }));
        Arc::clone(accepted)
    };
    let value = render(&get);
    let watched = found.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner).into_values().flat_map(|(_, watched)| watched).collect();
    (value, watched)
}

fn accepted_projects(reads: &Reads<'_, '_>, frame: &DecodedFrame, scope: ProjectScope, links: &HashMap<u32, u32>, watched: &mut Vec<TableRead>) -> Option<AcceptedProjects> {
    let (slot, owners) = match scope {
        ProjectScope::Owner(owner) => (*links.get(&owner)?, vec![owner]),
        ProjectScope::Country(slot) => (slot, links.iter().filter_map(|(&raw, &held)| (held == slot).then_some(raw)).collect()),
    };
    if owners.is_empty() { return None; }
    let country = Country::Slot(slot);
    reads.hold_through(reads.query.holders.interval(reads.hour).1);
    let dashboard_query::identity::Holder::Found { id, .. } = reads.query.holders.at(&country, reads.hour)?.holder else { return None };
    let at = Reads { tag: &country, holder: Owner { slot, id }, ..*reads };
    let slices = frame.complete::<tables::SliceReads>().unwrap_or(&[]);
    let mut accepted = AcceptedProjects::default();
    let mut by_stamp: BTreeMap<i32, BTreeSet<u32>> = BTreeMap::new();
    for &owner in &owners {
        watched.push(TableRead::fields(TableId::SliceReads, &["sample_hour"]).row(slice_read(TableId::Projects, owner)));
        if let Some(stamp) = replay_data::sliced::read_hour(slices, TableId::Projects, owner).filter(|&read| at.usable(TableId::Projects, owner, read).is_ok()) {
            by_stamp.entry(stamp).or_default().insert(owner);
        }
    }
    for (read, owners) in by_stamp {
        if let Ok(stamp) = dashboard_base::products::at_hour::frame(reads.query, read, &[Read { table: TableId::Projects, keys: owner_ranges(&owners, 32) }], SpecialProjectDetailsAt::TITLE)
            && let Ok(rows) = stamp.complete::<tables::Projects>()
        {
            accepted.rows.extend(rows.iter().filter(|row| owners.contains(&row.owner)).map(|row| ((row.owner, row.local_id), row.clone())));
            accepted.owners.extend(owners);
        }
    }
    if owners.len() == accepted.owners.len() { accepted.slots.insert(slot); }
    Some(accepted)
}

fn owners_of(frame: &DecodedFrame, slot: u32) -> io::Result<BTreeSet<u32>> {
    let links = frame.complete::<tables::CountryLinks>()?;
    Ok(dashboard_text::metadata::link_slots(links).into_iter().filter(|(_, held)| *held == slot).map(|(raw, _)| raw).collect())
}

fn owner_ranges(owners: &BTreeSet<u32>, shift: u32) -> Keys {
    Keys::Ranges(owners.iter().map(|&owner| (u128::from(owner) << shift, u128::from(owner) << shift | ((1u128 << shift) - 1))).collect())
}

fn prefixed<T>(rows: &[T], prefix: u32, of: impl Fn(&T) -> u32) -> &[T] {
    let first = rows.partition_point(|row| of(row) < prefix);
    let last = first + rows[first..].partition_point(|row| of(row) == prefix);
    &rows[first..last]
}

fn tokens(frame: &DecodedFrame) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    out.extend(frame.complete::<tables::ProjectBreakthroughs>().unwrap_or(&[]).iter().map(|row| row.specialization));
    out.extend(frame.complete::<tables::ScientistSkills>().unwrap_or(&[]).iter().map(|row| row.token));
    out.extend(frame.complete::<tables::CharacterRoleTraits>().unwrap_or(&[]).iter().flat_map(|row| row.tokens.iter().copied()));
    out.extend(frame.complete::<tables::Characters>().unwrap_or(&[]).iter().filter_map(|row| u32::try_from(row.name_token_id).ok()));
    out
}

const DETAIL_TABLES: [TableId; 14] = [
    TableId::CountryLinks,
    TableId::Scientists,
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
    TableId::SliceReads,
    TableId::ScientistDaily,
    TableId::ScientistSkills,
    TableId::CharacterRoleTraits,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
];

impl AtHour for SpecialProjectDetailsAt {
    const KEY: &'static str = "special-project-details";
    const TITLE: &'static str = "Special project details";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    const REPLY_DATES_REQUEST_HOUR: bool = true;
    type Params = SpecialProjectDetailsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = SpecialProjectDetails;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            SpecialProjectDetailsParams::decl(),
            SpecialProjectDetails::decl(),
            ProjectLabel::decl(),
            ScientistSkill::decl(),
            ScientistDetails::decl(),
            ScientistAssignment::decl(),
            ProgramDetails::decl(),
            FacilitySource::decl(),
            FacilityDetails::decl(),
            ProjectRewardOption::decl(),
            PendingProjectReward::decl(),
            ProjectVariableValue::decl(),
            ProjectChoice::decl(),
            SpecializationProgress::decl(),
            ProjectRuntimeDetails::decl(),
        ]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }


    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        let fields: &[&str] = match table {
            TableId::Projects => &["definition_id", "runtime"],
            TableId::Scientists => &["country", "active"],
            _ => return None,
        };
        Some(dashboard_base::products::at_hour::field_columns(table, fields))
    }

    fn compares_previous(table: TableId) -> bool {
        matches!(table, TableId::SliceReads | TableId::Scientists)
    }

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> bool {
        match stage {
            1 => edits.fields(TableId::CountryLinks, u64::MAX) || edits.fields(TableId::Scientists, dashboard_base::products::at_hour::field_mask(TableId::Scientists, &["country"])),
            2 => [TableId::ProjectBreakthroughs, TableId::ScientistSkills, TableId::CharacterRoleTraits, TableId::Characters].iter().any(|&t| edits.fields(t, u64::MAX)),
            _ => false,
        }
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        if edits.only_columns(TableId::Scientists, u64::MAX)
            && let Some(changes) = edits.tables.get(&TableId::Scientists)
            && let Some(TableView::Complete(before)) = edits.previous.get(&TableId::Scientists).map(|held| held.held())
            && let Some(before) = before.rows::<tables::Scientists>()
            && let Ok(now) = frame.complete::<tables::Scientists>()
            && let Some(owner) = at.owner
        {
            let owners = owners_of(frame, owner.slot)?;
            if changes.keys().all(|&key| [before, now].into_iter().all(|rows| rows.binary_search_by_key(&key, |row| u128::from(row.character)).ok().is_none_or(|i| !owners.contains(&rows[i].country)))) {
                return Ok(true);
            }
        }
        reread(frame, params, at, edits, data)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        DETAIL_TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => [TableId::CountryLinks, TableId::Scientists].into_iter().map(Read::all).collect(),
            1 => {
                let Ok(owners) = owners_of(held, owner.slot) else { return Vec::new() };
                if owners.is_empty() {
                    return Vec::new();
                }
                let roster: BTreeSet<_> = held.complete::<tables::Scientists>().unwrap_or(&[]).iter().filter(|row| owners.contains(&row.country)).map(|row| row.character).collect();
                vec![
                    Read { table: TableId::Projects, keys: owner_ranges(&owners, 32) },
                    Read { table: TableId::ProjectHistory, keys: owner_ranges(&owners, 64) },
                    Read { table: TableId::ProjectBreakthroughs, keys: owner_ranges(&owners, 32) },
                    Read::these(TableId::SliceReads, owners.iter().flat_map(|&owner| [slice_read(TableId::Projects, owner), slice_read(TableId::ProjectHistory, owner)])),
                    Read::these(TableId::ScientistDaily, roster.iter().map(|&id| u128::from(id))),
                    Read { table: TableId::ScientistSkills, keys: owner_ranges(&roster, 32) },
                    Read::these(TableId::CharacterRoleTraits, roster.iter().map(|&id| u128::from(id) << 40 | u128::from(CharacterRoleTraitRow::SCIENTIST) << 32)),
                    Read::these(TableId::Characters, roster.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::CharacterPortraits, roster.iter().map(|&id| u128::from(id))),
                ]
            }
            2 => {
                vec![Read::these(TableId::LexicalTokens, tokens(held).into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(context(source, load))
    }

    fn valid_reply_through(frame: &DecodedFrame, _: &Self::Params, _: &At<'_, Context>, reply: &Self::Reply, _: bool) -> i32 {
        reply.through.min(dashboard_base::products::at_hour::warnings_through(frame.hour, reply.unavailable.iter().chain(&reply.warnings).map(String::as_str)))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Special project details name no country"))?;
        let query = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
        let run = query.timeline.at(at.hour).ok_or_else(|| dashboard_core::refused("Special project details hour is not recorded"))?.run;
        let through = AtomicI32::new(i32::MAX);
        let reads = Reads { through: Some(&through), query: &query, coverage: &at.context.coverage, links: &at.context.links, tag: &params.country, holder: owner, run, hour: at.hour };
        let mut namer = Namer::new(at.context, frame, at.cancelled, owner.slot);
        let mut out = SpecialProjectDetails { through: i32::MAX, hour: at.hour, scientists: None, programs: None, breakthroughs: None, projects: None, unavailable: Vec::new(), warnings: Vec::new() };
        if let Some(why) = &namer.missing {
            out.unavailable.push(why.clone());
        }
        let owners = match owners_of(frame, owner.slot) {
            Ok(owners) => owners,
            Err(why) => {
                out.unavailable.push(format!("The country's links are unavailable: {why}"));
                return Ok(out);
            }
        };
        out.scientists = scientists(frame, &owners, &reads, &mut namer, at.context, &mut out.unavailable)?;
        out.breakthroughs = breakthroughs(frame, &owners, &reads, &mut namer, &mut out.unavailable);
        out.programs = programs(&owners, &reads, &mut namer, &mut out.unavailable);
        out.projects = projects(&owners, &reads, &mut namer, &at.holders.name(&params.country, at.hour), &mut out.unavailable);
        let scoped = !namer.scoped.is_empty();
        at.context.scoped_details.store(scoped, std::sync::atomic::Ordering::Relaxed);
        out.through = namer.through.min(through.load(Ordering::Relaxed));
        out.warnings = namer.warnings();
        Ok(out)
    }
}

fn reread(frame: &DecodedFrame, params: &SpecialProjectDetailsParams, at: &At<'_, Context>, edits: &dashboard_base::products::at_hour::Edits, data: &mut impl std::ops::DerefMut<Target = serde_json::Value>) -> io::Result<bool> {
    if !edits.only_columns(TableId::SliceReads, dashboard_base::products::at_hour::field_mask(TableId::SliceReads, &["sample_hour"])) || at.context.scoped_details.load(std::sync::atomic::Ordering::Relaxed) { return Ok(false); }
    let Some(holder) = at.owner else { return Ok(false) };
    let q = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
    let reads = Reads { through: None, query: &q, coverage: &at.context.coverage, links: &at.context.links, tag: &params.country, holder, run: q.timeline.at(at.hour).unwrap().run, hour: at.hour };
    let Some(previous) = data["hour"].as_i64().and_then(|h| i32::try_from(h).ok()) else { return Ok(false) };
    if (Reads { hour: previous, ..reads }).last_complete(TableId::ProjectPrograms, "Program assignments") != reads.last_complete(TableId::ProjectPrograms, "Program assignments") {
        return Ok(false);
    }
    for (table, group, what) in [(TableId::ProjectBreakthroughs, "breakthroughs", "Specialization progress points")] {
        let Some(read) = reads.continued(previous, table, what) else { return Ok(false) };
        if let Some(hour) = read {
            let Some(items) = data[group].as_array_mut() else { return Ok(false) };
            for item in items { item["readHour"] = hour.into(); }
        }
    }
    let Some(daily) = reads.continued(previous, TableId::ScientistDaily, "Scientist injuries and visibility") else { return Ok(false) };
    if let Some(daily) = daily {
        if frame.complete::<tables::ScientistDaily>().is_ok_and(|rows| rows.iter().any(|r| matches!(r.template_visibility, Recorded::Value(Some(1))))) { return Ok(false); }
        let Some(scientists) = data["scientists"].as_array_mut() else { return Ok(false) };
        for scientist in scientists {
            for key in ["visibilityHour", "injuryHour"] {
                if !scientist[key].is_null() { scientist[key] = daily.into(); }
            }
        }
    }
    let Ok(slices) = frame.complete::<tables::SliceReads>() else { return Ok(false) };
    let Some(TableView::Complete(before)) = edits.previous.get(&TableId::SliceReads).map(|view| view.held()) else { return Ok(false) };
    let Some(before_slices) = before.rows::<tables::SliceReads>() else { return Ok(false) };
    let Ok(owners) = owners_of(frame, holder.slot) else { return Ok(false) };
    let Ok(rows) = frame.complete::<tables::Projects>() else { return Ok(false) };
    let earlier = Reads { hour: previous, ..reads };
    let mut read_of = BTreeMap::new();
    for &owner in &owners {
        let has_rows = !prefixed(rows, owner, |r| r.owner).is_empty();
        let now = OwnerRead::of(slices, &reads, owner, has_rows);
        if now != OwnerRead::of(before_slices, &earlier, owner, has_rows) {
            return Ok(false);
        }
        match now {
            OwnerRead::Read(read, history) => {
                read_of.insert(owner, (read, history));
            }
            OwnerRead::Withheld => return Ok(false),
        }
    }
    let unavailable: BTreeSet<&str> = data["unavailable"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).collect();
    let Some(projects) = data["projects"].as_array() else { return Ok(false) };
    for project in projects {
        let Some((_, Some(history))) = project["owner"].as_u64().and_then(|owner| read_of.get(&(owner as u32))) else { return Ok(false) };
        let kept = match history {
            Ok(read) => project["historyReadHour"].as_i64() == Some(i64::from(*read)),
            Err(why) => project["choices"].is_null() && unavailable.contains(why.as_str()),
        };
        if !kept {
            return Ok(false);
        }
    }
    let Some(projects) = data["projects"].as_array_mut() else { return Ok(false) };
    for project in projects {
        project["readHour"] = read_of[&(project["owner"].as_u64().unwrap() as u32)].0.into();
    }
    data["hour"] = at.hour.into();
    Ok(true)
}

fn scientists(
    frame: &DecodedFrame,
    owners: &BTreeSet<u32>,
    reads: &Reads<'_, '_>,
    namer: &mut Namer<'_, '_, '_>,
    context: &Context,
    unavailable: &mut Vec<String>,
) -> io::Result<Option<Vec<ScientistDetails>>> {
    let rows = match frame.complete::<tables::Scientists>() {
        Ok(rows) => rows,
        Err(why) => {
            unavailable.push(format!("Scientists are unavailable: {why}"));
            return Ok(None);
        }
    };
    let skills: Result<&[_], String> = frame.complete::<tables::ScientistSkills>().map_err(|why| format!("Scientist skills are unavailable: {why}"));
    let traits: Result<&[_], String> = frame.complete::<tables::CharacterRoleTraits>().map_err(|why| format!("Scientist traits are unavailable: {why}"));
    for why in [skills.as_ref().err(), traits.as_ref().err()].into_iter().flatten() {
        unavailable.push(why.clone());
    }
    let daily = frame.complete::<tables::ScientistDaily>().map_err(|why| format!("Scientist injuries and visibility are unavailable: {why}")).and_then(|rows| {
        reads.last(TableId::ScientistDaily, "Scientist injuries and visibility").map(|read| (rows, read))
    });
    if let Err(why) = &daily {
        unavailable.push(why.clone());
    }
    let roster: Vec<_> = rows.iter().filter(|row| owners.contains(&row.country)).collect();
    let observed: Vec<(u32, &ScientistDailyRow)> = match &daily {
        Ok((rows, _)) => roster
            .iter()
            .filter_map(|scientist| {
                let row = rows.binary_search_by_key(&scientist.character, |row| row.character).ok().map(|at| &rows[at])?;
                (row.country == scientist.country).then_some((scientist.character, row))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    let verdicts = match &daily {
        Ok((_, read)) => visibility::verdicts(context, reads.query, *read, namer.slot, &observed)?,
        Err(_) => HashMap::new(),
    };
    let mut out = Vec::with_capacity(roster.len());
    for scientist in roster {
        let character = scientist.character;
        let mut details = ScientistDetails {
            character,
            person: namer.person(frame, u64::from(character) << 32 | u64::from(CHARACTER_REFERENCE)),
            active: match scientist.active {
                Recorded::Value(active) => active,
                _ => None,
            },
            injury_days: None,
            injury_hour: None,
            visible: None,
            visibility_hour: None,
            state_refusals: BTreeMap::new(),
            skills: skills.as_ref().ok().copied().map(|rows| {
                prefixed(rows, character, |row| row.character)
                    .iter()
                    .map(|row| ScientistSkill { specialization: namer.token(frame, row.token, unavailable), level: row.level, experience_raw: row.experience_raw.to_string() })
                    .collect()
            }),
            traits: traits.as_ref().ok().copied().map(|rows| {
                prefixed(rows, character, |row| row.character)
                    .iter()
                    .filter(|row| row.kind == CharacterRoleTraitRow::SCIENTIST)
                    .flat_map(|row| row.tokens.iter().copied())
                    .map(|token| namer.token(frame, token, unavailable))
                    .collect()
            }),
        };
        if let (Ok((_, read)), Some((_, row))) = (&daily, observed.iter().find(|(id, _)| *id == character)) {
            if let Recorded::Value(days) = row.injury_days {
                details.injury_days = days;
                details.injury_hour = Some(*read);
            }
            match verdicts.get(&character) {
                Some(Ok(visible)) => {
                    details.visible = Some(*visible);
                    details.visibility_hour = Some(*read);
                }
                Some(Err(reason)) => {
                    details.state_refusals.insert(reason.clone(), 1);
                }
                None => {}
            }
        }
        out.push(details);
    }
    Ok(Some(out))
}

fn breakthroughs(frame: &DecodedFrame, owners: &BTreeSet<u32>, reads: &Reads<'_, '_>, namer: &mut Namer<'_, '_, '_>, unavailable: &mut Vec<String>) -> Option<Vec<SpecializationProgress>> {
    let read = frame
        .complete::<tables::ProjectBreakthroughs>()
        .map_err(|why| format!("Specialization progress is unavailable: {why}"))
        .and_then(|rows| reads.last(TableId::ProjectBreakthroughs, "Specialization progress points").map(|read| (rows, read)));
    let (rows, read) = read.map_err(|why| unavailable.push(why)).ok()?;
    let mut out = Vec::new();
    for &owner in owners {
        for row in prefixed(rows, owner, |row| row.owner) {
            out.push(SpecializationProgress { specialization: namer.token(frame, row.specialization, unavailable), points: row.points, read_hour: read });
        }
    }
    Some(out)
}

fn program_labels(reads: &Reads<'_, '_>, read: i32, programs: &[&replay_data::rows::ProjectProgramRow]) -> Result<DecodedFrame, String> {
    let (mut tokens, mut keys, mut people) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for program in programs {
        if let Recorded::Value(Some(operands)) = &program.facility {
            tokens.extend(operands.iter().map(|operand| operand.token).filter(|&token| token != 0));
            keys.extend(operands.iter().map(|operand| operand.key).filter(|&key| key != 0));
        }
        if let Recorded::Value(Some(assignments)) = &program.assignments {
            people.extend(assignments.iter().filter(|value| value.scientist as u32 == CHARACTER_REFERENCE).map(|value| (value.scientist >> 32) as u32));
        }
    }
    let unavailable = |why: io::Error| format!("Program labels at their read are unavailable: {why}");
    let people_keys = || people.iter().map(|&id| u128::from(id));
    let mut frame = dashboard_base::products::at_hour::frame(
        reads.query,
        read,
        &[Read::these(TableId::ModifierDefinitions, keys.iter().map(|&key| u128::from(key))), Read::these(TableId::Characters, people_keys()), Read::these(TableId::CharacterPortraits, people_keys())],
        SpecialProjectDetailsAt::TITLE,
    )
    .map_err(unavailable)?;
    tokens.extend(frame.complete::<tables::Characters>().map(|rows| rows.iter().filter_map(|row| u32::try_from(row.name_token_id).ok()).collect::<Vec<_>>()).unwrap_or_default());
    let lexical = dashboard_base::products::at_hour::frame(reads.query, read, &[Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))], SpecialProjectDetailsAt::TITLE).map_err(unavailable)?;
    frame.tables[TableId::LexicalTokens.index()] = lexical.tables[TableId::LexicalTokens.index()].clone();
    Ok(frame)
}

fn programs(owners: &BTreeSet<u32>, reads: &Reads<'_, '_>, namer: &mut Namer<'_, '_, '_>, unavailable: &mut Vec<String>) -> Option<Vec<ProgramDetails>> {
    let read = reads.last_complete(TableId::ProjectPrograms, "Program assignments").and_then(|read| {
        dashboard_base::products::at_hour::frame(reads.query, read, &[Read::all(TableId::ProjectPrograms)], SpecialProjectDetailsAt::TITLE)
            .map_err(|why| format!("Program assignments are unavailable: {why}"))
            .map(|stamp| (stamp, read))
    });
    let (stamp, read) = read.map_err(|why| unavailable.push(why)).ok()?;
    let rows = stamp.complete::<tables::ProjectPrograms>().map_err(|why| unavailable.push(format!("Program assignments are unavailable: {why}"))).ok()?;
    let mut held = Vec::new();
    for program in rows.iter().filter(|row| owners.contains(&row.owner)) {
        if reads.held_since(program.owner, read).is_ok() {
            held.push(program);
        } else if !unavailable.iter().any(|why| why == OTHER_OWNER) {
            unavailable.push(OTHER_OWNER.to_owned());
        }
    }
    let labels = program_labels(reads, read, &held).map_err(|why| unavailable.push(why)).ok()?;
    let frame = &labels;
    let mut out = Vec::new();
    for program in held {
        let assignments = match &program.assignments {
            Recorded::Value(Some(values)) => Some(
                values
                    .iter()
                    .enumerate()
                    .map(|(ordinal, value)| ScientistAssignment {
                        ordinal: ordinal as u32,
                        reference: value.scientist.to_string(),
                        person: namer.person(frame, value.scientist),
                        target: value.target,
                        current: value.current,
                        timed: value.timed,
                    })
                    .collect(),
            ),
            _ => None,
        };
        out.push(ProgramDetails { id: program.id, province: program.province, read_hour: read, assignments, facility: facility::present(frame, program, namer, unavailable) });
    }
    Some(out)
}

fn definition<'c>(content: &'c ProjectContent, token: &str) -> Option<&'c content_reader_military::special_projects::Project> {
    content.definitions.projects.iter().find(|(name, _)| name.eq_ignore_ascii_case(token)).map(|(_, definition)| definition)
}

fn project_name_key<'a>(definition: &'a content_reader_military::special_projects::Project, token: &'a str) -> &'a str {
    definition.name.as_deref().filter(|name| !name.is_empty()).unwrap_or(token)
}

fn pending(frame: &DecodedFrame, runtime: &ProjectRuntime, project_token: Result<&str, &str>, id: u32, namer: &mut Namer<'_, '_, '_>) -> PendingProjectReward {
    let reward_token = spelled(frame, id);
    let content = namer.content();
    let found = (|| -> Result<_, String> {
        let reward_token = reward_token.clone()?;
        let content = content.ok_or("Recorded content is unavailable")?;
        let project_token = project_token.map_err(str::to_owned)?;
        let ordinal = runtime.pending_ordinal.ok_or("Pending reward object occurrence is unavailable")?;
        let reward = content.definitions.reward_at(project_token, ordinal, &reward_token)?;
        Ok((reward_token, reward))
    })();
    match found {
        Ok((token, reward)) => PendingProjectReward {
            label: ProjectLabel { name: namer.name(reward.name.as_deref().unwrap_or(&token)), token },
            ordinal: runtime.pending_ordinal,
            description: reward.description.as_deref().and_then(|key| namer.name(key)),
            options: reward
                .options
                .iter()
                .map(|option| ProjectRewardOption { label: ProjectLabel { token: option.token.clone(), name: namer.name(option.name.as_deref().unwrap_or(&option.token)) } })
                .collect(),
            unavailable: None,
        },
        Err(why) => PendingProjectReward {
            label: ProjectLabel { token: reward_token.unwrap_or_else(|_| id.to_string()), name: None },
            ordinal: runtime.pending_ordinal,
            description: None,
            options: Vec::new(),
            unavailable: Some(why),
        },
    }
}

pub(super) fn option_name(options: &[content_reader_military::special_projects::PrototypeOption], option: &str) -> Result<Option<String>, String> {
    let mut chosen = options.iter().filter(|candidate| candidate.token.eq_ignore_ascii_case(option));
    let first = chosen.next().ok_or_else(|| format!("Chosen option {option} is absent from recorded content"))?;
    if chosen.any(|other| other.name != first.name) {
        return Err(format!("Chosen option {option} shares its token with an option of another name"));
    }
    Ok(first.name.clone())
}

fn history_read(slices: &[SliceReadRow], reads: &Reads<'_, '_>, owner: u32) -> Result<i32, String> {
    let read = replay_data::sliced::read_hour(slices, TableId::ProjectHistory, owner).ok_or(HISTORY_UNREAD)?;
    reads.usable(TableId::ProjectHistory, owner, read).map_err(|why| match why {
        Withheld::Unread | Withheld::Tag => HISTORY_UNREAD,
        Withheld::Partial => HISTORY_PARTIAL,
        Withheld::Owner => OTHER_OWNER,
    })?;
    Ok(read)
}

#[derive(PartialEq, Eq)]
enum OwnerRead {
    Read(i32, Option<Result<i32, String>>),
    Withheld,
}

impl OwnerRead {
    fn of(slices: &[SliceReadRow], reads: &Reads<'_, '_>, owner: u32, has_rows: bool) -> Self {
        match projects_read(reads, owner, replay_data::sliced::read_hour(slices, TableId::Projects, owner)) {
            Ok(read) => Self::Read(read, has_rows.then(|| history_read(slices, reads, owner))),
            Err(_) => Self::Withheld,
        }
    }
}

const UNREAD_WITH_HISTORY: &str = "No Projects read supplies the choice history's hour, so the definition its choices belong to is not recorded";

struct Listed {
    history: DecodedFrame,
    lexical: DecodedFrame,
    definitions: BTreeMap<u32, u32>,
}

fn listed(reads: &Reads<'_, '_>, owner: u32, read: i32, history: DecodedFrame) -> Result<Listed, String> {
    let (_, projects) = stamped(&Reads { hour: read, through: None, ..*reads }, TableId::Projects, owner).map_err(|_| UNREAD_WITH_HISTORY.to_owned())?;
    let rows = projects.complete::<tables::Projects>().map_err(|why| format!("The Projects read supplying the choice history is unavailable: {why}"))?;
    let definitions: BTreeMap<u32, u32> = prefixed(rows, owner, |row| row.owner).iter().map(|row| (row.local_id, row.definition_id)).collect();
    let mut ids: BTreeSet<u32> = definitions.values().copied().collect();
    ids.extend(prefixed(history.complete::<tables::ProjectHistory>().unwrap_or(&[]), owner, |row| row.owner).iter().flat_map(|row| [row.prototype_reward_handle, row.option_handle]));
    let lexical = dashboard_base::products::at_hour::frame(reads.query, read, &[Read::these(TableId::LexicalTokens, ids.into_iter().map(u128::from))], SpecialProjectDetailsAt::TITLE)
        .map_err(|why| format!("Choice spellings at the history read are unavailable: {why}"))?;
    Ok(Listed { history, lexical, definitions })
}

fn choices(listed: &Listed, project: &ProjectRow, namer: &mut Namer<'_, '_, '_>, unavailable: &mut Vec<String>) -> Option<Vec<ProjectChoice>> {
    let rows = listed.history.complete::<tables::ProjectHistory>().map_err(|why| unavailable.push(format!("Project choice history is unavailable: {why}"))).ok()?;
    let first = rows.partition_point(|row| (row.owner, row.project) < (project.owner, project.local_id));
    let last = first + rows[first..].partition_point(|row| (row.owner, row.project) == (project.owner, project.local_id));
    let Some(&definition) = listed.definitions.get(&project.local_id) else {
        unavailable.push(UNREAD_WITH_HISTORY.into());
        return None;
    };
    let lexical = &listed.lexical;
    let project_token = spelled(lexical, definition);
    if let Err(why) = &project_token {
        unavailable.push(why.clone());
    }
    let marker = |row: &ProjectHistoryRow| row.prototype_reward_handle == COMPLETION_MARKER && row.option_handle == COMPLETION_MARKER;
    let current = rows[first..last].iter().rposition(marker).map_or(0, |at| at + 1);
    Some(
        rows[first..last]
            .iter()
            .enumerate()
            .filter(|(_, row)| !marker(row))
            .map(|(at, row)| (at < current, row))
            .map(|(earlier, row)| {
                let reward = spelled(lexical, row.prototype_reward_handle).map_err(|why| unavailable.push(why)).ok();
                let option = spelled(lexical, row.option_handle).map_err(|why| unavailable.push(why)).ok();
                let authored = match (namer.content(), &project_token, reward.as_deref()) {
                    (Some(content), Ok(project), Some(reward)) => content.definitions.reward_named(project, reward).map_err(|why| unavailable.push(why)).ok(),
                    _ => None,
                };
                let option_key = authored.zip(option.as_deref()).and_then(|(found, option)| option_name(&found.options, option).map_err(|why| unavailable.push(why)).ok());
                (earlier, row, namer.named(reward, row.prototype_reward_handle, authored.map(|found| found.name.clone())), namer.named(option, row.option_handle, option_key))
            })
            .map(|(earlier, row, reward, option)| ProjectChoice {
                ordinal: row.ordinal,
                date: row.date,
                earlier,
                reward,
                option,
                province: row.context.province,
                progress: row.context.progress,
                character_reference: row.context.character_reference.to_string(),
                country: row.context.country_reference,
                scientist: row.context.scientist_reference,
                state: row.context.state_reference,
            })
            .collect(),
    )
}

fn projects(
    owners: &BTreeSet<u32>,
    reads: &Reads<'_, '_>,
    namer: &mut Namer<'_, '_, '_>,
    tag: &str,
    unavailable: &mut Vec<String>,
) -> Option<Vec<ProjectRuntimeDetails>> {
    let mut read_of: BTreeMap<u32, (i32, DecodedFrame)> = BTreeMap::new();
    let mut withheld = Vec::new();
    for &owner in owners {
        match stamped(reads, TableId::Projects, owner) {
            Ok(found) => {
                read_of.insert(owner, found);
            }
            Err(why) if !withheld.contains(&why) => withheld.push(why),
            Err(_) => {}
        }
    }
    if !withheld.is_empty() {
        unavailable.extend(withheld);
        return None;
    }
    let mut out = Vec::new();
    for (&owner, (read, stamp)) in &read_of {
        let read = *read;
        let Ok(rows) = stamp.complete::<tables::Projects>() else {
            unavailable.push(PROJECTS_PARTIAL.into());
            return None;
        };
        let owned = prefixed(rows, owner, |row| row.owner);
        let history = stamped(reads, TableId::ProjectHistory, owner).and_then(|(history, rows)| {
            if owned.is_empty() { return Ok((history, None)); }
            listed(reads, owner, history, rows).map(|listed| (history, Some(listed)))
        });
        if let Err(why) = &history
            && !owned.is_empty()
            && !unavailable.contains(why)
        {
            unavailable.push(why.clone());
        }
        for project in owned {
            let token = spelled(stamp, project.definition_id);
            if let Err(why) = &token {
                unavailable.push(why.clone());
            }
            let authored = namer.content().zip(token.as_ref().ok()).and_then(|(content, token)| definition(content, token).map(|found| (content, found, token)));
            let name = authored.and_then(|(content, found, token)| namer.name(&country_key(content, tag, project_name_key(found, token))));
            let runtime = match &project.runtime {
                Recorded::Value(runtime) => Some(runtime),
                _ => None,
            };
            let variables = runtime.and_then(|runtime| runtime.variables.as_ref()).and_then(|store| {
                store
                    .entries
                    .iter()
                    .map(|entry| String::from_utf8(entry.name.clone()).map(|name| ProjectVariableValue { name, value_raw: entry.value_raw.to_string() }))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| unavailable.push(format!("A variable name of project {owner}:{} is not UTF-8", project.local_id)))
                    .ok()
            });
            let choices = match &history {
                Ok((_, Some(listed))) => choices(listed, project, namer, unavailable),
                _ => None,
            };
            out.push(ProjectRuntimeDetails {
                owner,
                id: project.local_id,
                label: ProjectLabel { name, token: token.clone().unwrap_or_else(|_| project.definition_id.to_string()) },
                read_hour: read,
                pending_known: runtime.is_some_and(|runtime| runtime.pending_known),
                pending: runtime.and_then(|runtime| runtime.pending_reward.map(|id| pending(stamp, runtime, token.as_deref().map_err(String::as_str), id, namer))),
                seed: runtime.and_then(|runtime| runtime.variables.as_ref()).map(|store| store.seed),
                variables,
                history_read_hour: history.as_ref().ok().map(|(read, _)| *read).filter(|_| choices.is_some()),
                choices,
            });
        }
    }
    Some(out)
}

fn assigned(reads: &Reads<'_, '_>, project: &ProjectRow) -> super::Program {
    let id = (project.program_reference >> 32) as u32;
    let read = (project.program_reference as u32 == PROGRAM_REFERENCE)
        .then(|| reads.last_complete(TableId::ProjectPrograms, "Program assignments").ok())
        .flatten()
        .filter(|&read| reads.held_since(project.owner, read).is_ok());
    let cells = read.map(|read| {
        let row = dashboard_base::products::at_hour::frame(reads.query, read, &[Read::these(TableId::ProjectPrograms, [u128::from(id)])], SpecialProjectEffectsAt::TITLE)
            .ok()
            .and_then(|frame| frame.complete::<tables::ProjectPrograms>().ok().and_then(|rows| rows.iter().find(|row| row.id == id).cloned()));
        (read, row.map(|row| super::Assignment { owner: Some(row.owner), active: row.active_project, scientist: Some(row.scientist) }))
    });
    super::assignment(project.owner, project.local_id, project.program_reference, cells)
}

fn effect_tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = dashboard_effects::effect_description::recorded_tables()
        .chain([TableId::RaidFacilities, TableId::Provinces, TableId::Projects, TableId::ProjectPrograms, TableId::LexicalTokens, TableId::CountryLinks, TableId::SliceReads])
        .collect();
    tables.sort_unstable();
    tables.dedup();
    tables
}

fn effects(
    content: &ProjectContent,
    frame: &DecodedFrame,
    slot: u32,
    project: &ProjectRow,
    output: &content_reader_military::special_projects::Output,
    reads: &Reads<'_, '_>,
    accepted: ProjectReads<'_>,
    watched: &mut Vec<TableRead>,
) -> ProjectEffects {
    let mut render = || -> Result<_, String> {
        let catalogue = &content.politics.catalogue;
        let scripted = catalogue.scripted_content()?;
        let text = dashboard_text::scripted_text::ScriptedText::new(frame, &catalogue.localisation, scripted, slot).with_accepted_projects(accepted);
        let cache = scripted.effects()?;
        let context = dashboard_effects::effect_description::Context::new(&text, &catalogue.localisation, slot, "special-project output", cache.view(None)).with_entity_links();
        let facility = dashboard_text::triggers::inputs::unrecorded(|| {
            if output.facility_state_effects.is_empty() || project.program_reference == 0 {
                return Ok(None);
            }
            if project.program_reference as u32 != PROGRAM_REFERENCE {
                return Err("Unknown program reference type".to_owned());
            }
            let id = (project.program_reference >> 32) as u32;
            watched.push(TableRead::fields(TableId::RaidFacilities, &["province"]).row(u128::from(id)));
            let facility =
                frame.complete::<tables::RaidFacilities>().map_err(|why| why.to_string())?.iter().find(|row| row.id == id).ok_or("raid facilities lacks the selected project program")?;
            watched.push(TableRead::fields(TableId::Provinces, &["state"]).row(u128::from(facility.province)));
            let provinces = frame.complete::<tables::Provinces>().map_err(|why| why.to_string())?;
            let state = provinces.iter().find(|row| row.id == facility.province).ok_or("Provinces lacks the facility province")?.state;
            Ok(Some((facility.province, state)))
        });
        let scientist = if output.scientist_effects.is_empty() { Ok(None) } else { assigned(reads, project).scientist() };
        Ok(dashboard_effects::effect_description::project_output_description(output, &context, project.owner, project.local_id, scientist, facility))
    };
    match render() {
        Ok(description) => description.into(),
        Err(why) => ProjectEffects { runs: Vec::new(), refusals: BTreeMap::from([(why, 1)]) },
    }
}

impl AtHour for SpecialProjectEffectsAt {
    const KEY: &'static str = "special-project-effects";
    const TITLE: &'static str = "Special project effects";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    const REPLY_DATES_REQUEST_HOUR: bool = true;
    type Params = SpecialProjectEffectsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, owner, project];
    type Reply = SpecialProjectEffects;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![SpecialProjectEffectsParams::decl(), SpecialProjectEffects::decl(), PendingEffects::decl(), OptionEffects::decl(), ProjectEffects::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryLinks, TableId::SliceReads]
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        if step != 0 {
            return Vec::new();
        }
        vec![Read::these(TableId::CountryLinks, [u128::from(params.owner)]), Read::these(TableId::SliceReads, [slice_read(TableId::Projects, params.owner)])]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(context(source, load))
    }

    fn holds_refusal(frame: &DecodedFrame) -> bool {
        frame.complete::<tables::CountryLinks>().is_ok() && frame.complete::<tables::SliceReads>().is_ok()
    }

    fn valid_through(frame: &DecodedFrame, params: &Self::Params, _: &At<'_, Context>) -> i32 {
        owner_read(frame, TableId::Projects, params.owner).ok().filter(|&stamp| super::current(stamp, frame.hour)).map_or(frame.hour, |stamp| stamp.saturating_add(STAGGER_PERIOD - 1))
    }

    fn valid_reply_through(frame: &DecodedFrame, _: &Self::Params, _: &At<'_, Context>, reply: &Self::Reply, _: bool) -> i32 {
        let refusals = reply.output.iter().chain(reply.pending.iter().flat_map(|pending| pending.options.iter().map(|option| &option.effects))).flat_map(|effects| effects.refusals.keys());
        reply.through.min(dashboard_base::products::at_hour::warnings_through(frame.hour, refusals.chain(&reply.unavailable).chain(&reply.warnings).map(String::as_str)))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Special project effects name no country"))?;
        let mut out = SpecialProjectEffects { through: i32::MAX, hour: at.hour, read_hour: None, output: None, pending: None, unavailable: None, warnings: Vec::new() };
        if !owners_of(frame, owner.slot)?.contains(&params.owner) {
            out.unavailable = Some("The project's owner is not this country at this hour".into());
            return Ok(out);
        }
        let query = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
        let run = query.timeline.at(at.hour).ok_or_else(|| dashboard_core::refused("Special project effects hour is not recorded"))?.run;
        let through = AtomicI32::new(i32::MAX);
        let reads = Reads { through: Some(&through), query: &query, coverage: &at.context.coverage, links: &at.context.links, tag: &params.country, holder: owner, run, hour: at.hour };
        let (project, spellings) = match stamped_project(&reads, params.owner, params.project) {
            Ok((project, read, spellings)) => {
                out.read_hour = Some(read);
                (project, spellings)
            }
            Err(why) => {
                out.unavailable = Some(why);
                out.through = through.load(Ordering::Relaxed);
                return Ok(out);
            }
        };
        let content = at.context.content.as_ref().map_err(|why| dashboard_core::refused(why.clone()))?;
        let token = match spelled(&spellings, project.definition_id) {
            Ok(token) => token,
            Err(why) => {
                out.unavailable = Some(why);
                out.through = through.load(Ordering::Relaxed);
                return Ok(out);
            }
        };
        let Some(found) = definition(content, &token) else {
            out.unavailable = Some(format!("Definition {token} is absent from content"));
            out.through = through.load(Ordering::Relaxed);
            return Ok(out);
        };
        let project = &project;
        let ((), rendered) = at.context.text_inputs.evaluate_with_reads(frame, &effect_tables(), at.cancelled, |world| {
            let mut facilities = Vec::new();
            let ((), mut watched) = with_projects(&reads, world, |accepted| {
                out.output = Some(effects(content, world, owner.slot, project, &found.output, &reads, accepted, &mut facilities));
                if let Recorded::Value(runtime) = &project.runtime
                    && let Some(id) = runtime.pending_reward
                {
                    let found = runtime.pending_ordinal.ok_or_else(|| "Pending reward object occurrence is unavailable".to_owned()).and_then(|ordinal| {
                        let reward = spelled(&spellings, id)?;
                        content.definitions.reward_at(&token, ordinal, &reward).map(|found| (reward, ordinal, found))
                    });
                    match found {
                        Ok((reward, ordinal, found)) => {
                            let options = found
                                .options
                                .iter()
                                .map(|option| OptionEffects { token: option.token.clone(), effects: effects(content, world, owner.slot, project, &option.output, &reads, accepted, &mut facilities) })
                                .collect();
                            out.pending = Some(PendingEffects { reward, ordinal, options });
                        }
                        Err(why) => out.unavailable = Some(why),
                    }
                }
            });
            watched.extend(facilities);
            ((), watched)
        })?;
        out.through = rendered.min(through.load(Ordering::Relaxed));
        Ok(out)
    }
}
