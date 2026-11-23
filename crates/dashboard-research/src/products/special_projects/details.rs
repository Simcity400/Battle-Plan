
use dashboard_effects::ScriptedEffects;
use dashboard_base::products::Country;
use super::{CHARACTER_REFERENCE, Naming, PROGRAM_REFERENCE, ProjectContent, ProjectPerson, country_key, needs_scope, scoped_at};
use dashboard_base::{Source, catalogue::View, products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read}}};
use dashboard_query::{Index, Query, cost, identity::Holder};
use replay::campaign::Replay;
use replay_lanes::lanes;
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{CharacterRoleTraitRow, ProjectRow, ProjectRuntime, Recorded, ScientistDailyRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

mod facility;
#[cfg(test)]
mod tests;
mod visibility;

pub use facility::{FacilityDetails, FacilitySource};

const COMPLETION_MARKER: u32 = super::fold::COMPLETION_MARKER;

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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectDetails {
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
    pub text: String,
    pub refusals: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectEffects {
    pub hour: i32,
    #[doc = " None where the project's definition is not in the recorded content."]
    pub output: Option<ProjectEffects>,
    #[doc = " The pending reward's options, in its order; empty with none pending."]
    pub options: Vec<ProjectEffects>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(SpecialProjectEffects);

pub struct Context {
    content: Result<Arc<ProjectContent>, String>,
    replay: Arc<Replay>,
    index: Arc<Index>,
    visibility: visibility::Cache,
    scoped_details: std::sync::atomic::AtomicBool,
    coverage: std::sync::Mutex<HashMap<(u32, TableId), Coverage>>,
}

fn context(source: &Source, load: &LoadContext) -> Context {
    Context {
        content: super::content(source, load).map_err(|error| format!("Recorded special-project content is unavailable: {error}")),
        replay: source.replay(),
        index: dashboard_base::queries::index(source),
        visibility: visibility::Cache::default(),
        scoped_details: false.into(),
        coverage: Default::default(),
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

fn held_since(query: &Query<'_>, tag: &Country, holder: Owner, read: i32, hour: i32) -> bool {
    let mut at = hour;
    while let Some(run) = query.holders.at(tag, at) {
        if run.holder != (Holder::Found { slot: holder.slot, id: holder.id }) {
            return false;
        }
        if run.first <= read {
            return true;
        }
        at = run.first - 1;
    }
    false
}

struct Coverage {
    block: u32,
    reads: Vec<(i32, i32)>,
}

struct Reads<'r, 'q> {
    query: &'r Query<'q>,
    coverage: &'r std::sync::Mutex<HashMap<(u32, TableId), Coverage>>,
    tag: &'r Country,
    holder: Owner,
    run: u32,
    hour: i32,
}

impl Reads<'_, '_> {
    fn last(&self, table: TableId, what: &str) -> Result<i32, String> {
        let from = self.hour.saturating_sub(STAGGER_PERIOD - 1);
        let recording = self.query.replay.recording();
        let run = recording.run(self.run).map_err(|e| e.to_string())?;
        let block = run.ordinal(self.hour).ok_or_else(|| format!("{what}: hour not recorded"))? / replay_columnar::unit::BLOCK;
        let mut cache = self.coverage.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (self.run, table);
        if cache.get(&key).is_none_or(|c| c.block != block) {
            let first = run.block_first_hour(block);
            let last = first.saturating_add(replay_columnar::unit::BLOCK as i32 - 1).min(run.last_hour());
            let reads = cost::storage(|| lanes::available_reads(recording, self.run, table, first.saturating_sub(STAGGER_PERIOD - 1), last)).map_err(|e| format!("{what}: {e}"))?;
            cache.insert(key, Coverage { block, reads });
        }
        let spans = &cache[&key].reads;
        let at = spans.partition_point(|s| s.0 <= self.hour);
        let read = at.checked_sub(1).map(|i| spans[i].1.min(self.hour)).filter(|&h| h >= from).ok_or_else(|| format!("{what} were not read in the day up to this hour"))?;
        self.supplies(read).then_some(read).ok_or_else(|| format!("{what} were last read before this country held the tag"))
    }

    fn supplies(&self, read: i32) -> bool {
        read <= self.hour && self.hour - read < STAGGER_PERIOD && held_since(self.query, self.tag, self.holder, read, self.hour)
    }

    fn continued(&self, previous: i32, table: TableId, what: &str) -> Option<Option<i32>> {
        let before = Self { hour: previous, ..*self };
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
    query: &'r Query<'q>,
    hour: i32,
    slot: u32,
    scoped: HashMap<String, Option<String>>,
}

impl<'c, 'r, 'q> Namer<'c, 'r, 'q> {
    fn new(content: &'c Result<Arc<ProjectContent>, String>, query: &'r Query<'q>, hour: i32, slot: u32) -> Self {
        let (naming, missing) = match content {
            Ok(content) => (Some(Naming::new(content)), None),
            Err(why) => (None, Some(why.clone())),
        };
        Self { naming, missing, query, hour, slot, scoped: HashMap::new() }
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
        let found = scoped_at(self.query, content, key, self.hour, self.slot).unwrap_or_else(|reason| {
            naming.note(reason, key);
            None
        });
        self.scoped.insert(key.to_owned(), found.clone());
        found
    }

    fn label(&mut self, token: String) -> ProjectLabel {
        ProjectLabel { name: self.name(&token), token }
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

fn scientist_references(frame: &DecodedFrame, owners: &BTreeSet<u32>) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let roster: BTreeSet<u32> =
        frame.complete::<tables::Scientists>().map(|rows| rows.iter().filter(|row| owners.contains(&row.country)).map(|row| row.character).collect()).unwrap_or_default();
    let mut assigned = BTreeSet::new();
    for program in frame.complete::<tables::ProjectPrograms>().unwrap_or(&[]).iter().filter(|row| owners.contains(&row.owner)) {
        if let Recorded::Value(Some(assignments)) = &program.assignments {
            assigned.extend(assignments.iter().filter(|value| value.scientist as u32 == CHARACTER_REFERENCE).map(|value| (value.scientist >> 32) as u32));
        }
    }
    (roster, assigned)
}

fn tokens(frame: &DecodedFrame, owners: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for row in frame.complete::<tables::Projects>().unwrap_or(&[]) {
        out.insert(row.definition_id);
        if let Recorded::Value(runtime) = &row.runtime {
            out.extend(runtime.pending_reward);
        }
    }
    for row in frame.complete::<tables::ProjectHistory>().unwrap_or(&[]) {
        out.extend([row.prototype_reward_handle, row.option_handle]);
    }
    out.extend(frame.complete::<tables::ProjectBreakthroughs>().unwrap_or(&[]).iter().map(|row| row.specialization));
    out.extend(frame.complete::<tables::ScientistSkills>().unwrap_or(&[]).iter().map(|row| row.token));
    out.extend(frame.complete::<tables::CharacterRoleTraits>().unwrap_or(&[]).iter().flat_map(|row| row.tokens.iter().copied()));
    for program in frame.complete::<tables::ProjectPrograms>().unwrap_or(&[]).iter().filter(|row| owners.contains(&row.owner)) {
        if let Recorded::Value(Some(operands)) = &program.facility {
            out.extend(operands.iter().map(|operand| operand.token).filter(|token| *token != 0));
        }
    }
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
    type Params = SpecialProjectDetailsParams;
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
        (table == TableId::Projects).then(|| dashboard_base::products::at_hour::field_columns(table, &["definition_id", "runtime"]))
    }

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> bool {
        match stage {
            1 => [TableId::CountryLinks, TableId::Scientists, TableId::ProjectPrograms].iter().any(|&t| edits.fields(t, u64::MAX)),
            2 => DETAIL_TABLES.iter().any(|&t| t != TableId::SliceReads && t != TableId::LexicalTokens && edits.fields(t, u64::MAX)),
            _ => false,
        }
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        if !edits.only_columns(TableId::SliceReads, dashboard_base::products::at_hour::field_mask(TableId::SliceReads, &["sample_hour"])) || at.context.scoped_details.load(std::sync::atomic::Ordering::Relaxed) { return Ok(false); }
        let Some(holder) = at.owner else { return Ok(false) };
        let q = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
        let reads = Reads { query: &q, coverage: &at.context.coverage, tag: &params.country, holder, run: q.timeline.at(at.hour).unwrap().run, hour: at.hour };
        let Some(previous) = data["hour"].as_i64().and_then(|h| i32::try_from(h).ok()) else { return Ok(false) };
        for (table, group, what) in [(TableId::ProjectPrograms, "programs", "Program assignments"), (TableId::ProjectBreakthroughs, "breakthroughs", "Specialization progress points")] {
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
        let Ok(owners) = owners_of(frame, holder.slot) else { return Ok(false) };
        let Ok(rows) = frame.complete::<tables::Projects>() else { return Ok(false) };
        for &owner in &owners {
            if (owner == holder.id || !prefixed(rows, owner, |r| r.owner).is_empty()) && !replay_data::sliced::read_hour(slices, TableId::Projects, owner).is_some_and(|read| reads.supplies(read)) {
                return Ok(false);
            }
        }
        let Some(projects) = data["projects"].as_array_mut() else { return Ok(false) };
        for project in projects {
            let owner = project["owner"].as_u64().unwrap() as u32;
            let Some(read) = replay_data::sliced::read_hour(slices, TableId::Projects, owner) else { return Ok(false) };
            project["readHour"] = read.into();
        }
        data["hour"] = at.hour.into();
        Ok(true)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        DETAIL_TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => [TableId::CountryLinks, TableId::Scientists, TableId::ProjectPrograms].into_iter().map(Read::all).collect(),
            1 => {
                let Ok(owners) = owners_of(held, owner.slot) else { return Vec::new() };
                if owners.is_empty() {
                    return Vec::new();
                }
                let (roster, assigned) = scientist_references(held, &owners);
                let characters: BTreeSet<u32> = roster.union(&assigned).copied().collect();
                let modifiers: BTreeSet<u32> = held
                    .complete::<tables::ProjectPrograms>()
                    .unwrap_or(&[])
                    .iter()
                    .filter(|row| owners.contains(&row.owner))
                    .filter_map(|row| match &row.facility {
                        Recorded::Value(Some(operands)) => Some(operands.iter().map(|operand| operand.key).filter(|key| *key != 0)),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                let projects = TableId::Projects.index() as u128;
                vec![
                    Read { table: TableId::Projects, keys: owner_ranges(&owners, 32) },
                    Read { table: TableId::ProjectHistory, keys: owner_ranges(&owners, 64) },
                    Read { table: TableId::ProjectBreakthroughs, keys: owner_ranges(&owners, 32) },
                    Read::these(TableId::SliceReads, owners.iter().map(|&owner| projects << 32 | u128::from(owner))),
                    Read::these(TableId::ScientistDaily, roster.iter().map(|&id| u128::from(id))),
                    Read { table: TableId::ScientistSkills, keys: owner_ranges(&roster, 32) },
                    Read::these(TableId::CharacterRoleTraits, roster.iter().map(|&id| u128::from(id) << 40 | u128::from(CharacterRoleTraitRow::SCIENTIST) << 32)),
                    Read::these(TableId::Characters, characters.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::CharacterPortraits, characters.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::ModifierDefinitions, modifiers.into_iter().map(u128::from)),
                ]
            }
            2 => {
                let Ok(owners) = owners_of(held, owner.slot) else { return Vec::new() };
                vec![Read::these(TableId::LexicalTokens, tokens(held, &owners).into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(context(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Special project details name no country"))?;
        let query = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
        let run = query.timeline.at(at.hour).ok_or_else(|| dashboard_core::refused("Special project details hour is not recorded"))?.run;
        let reads = Reads { query: &query, coverage: &at.context.coverage, tag: &params.country, holder: owner, run, hour: at.hour };
        let mut namer = Namer::new(&at.context.content, &query, at.hour, owner.slot);
        let mut out = SpecialProjectDetails { hour: at.hour, scientists: None, programs: None, breakthroughs: None, projects: None, unavailable: Vec::new(), warnings: Vec::new() };
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
        out.programs = programs(frame, &owners, &reads, &mut namer, &mut out.unavailable);
        out.projects = projects(frame, &owners, &reads, &mut namer, &at.holders.name(&params.country, at.hour), &mut out.unavailable);
        if !namer.scoped.is_empty() { at.context.scoped_details.store(true, std::sync::atomic::Ordering::Relaxed); }
        out.warnings = namer.warnings();
        Ok(out)
    }
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

fn programs(frame: &DecodedFrame, owners: &BTreeSet<u32>, reads: &Reads<'_, '_>, namer: &mut Namer<'_, '_, '_>, unavailable: &mut Vec<String>) -> Option<Vec<ProgramDetails>> {
    let read = frame
        .complete::<tables::ProjectPrograms>()
        .map_err(|why| format!("Program assignments are unavailable: {why}"))
        .and_then(|rows| reads.last(TableId::ProjectPrograms, "Program assignments").map(|read| (rows, read)));
    let (rows, read) = read.map_err(|why| unavailable.push(why)).ok()?;
    let mut out = Vec::new();
    for program in rows.iter().filter(|row| owners.contains(&row.owner)) {
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
            description: reward.description.as_deref().and_then(|key| namer.name(key)),
            options: reward
                .options
                .iter()
                .map(|option| ProjectRewardOption { label: ProjectLabel { token: option.token.clone(), name: namer.name(option.name.as_deref().unwrap_or(&option.token)) } })
                .collect(),
            unavailable: None,
        },
        Err(why) => PendingProjectReward { label: ProjectLabel { token: reward_token.unwrap_or_else(|_| id.to_string()), name: None }, description: None, options: Vec::new(), unavailable: Some(why) },
    }
}

fn choices(frame: &DecodedFrame, project: &ProjectRow, namer: &mut Namer<'_, '_, '_>, unavailable: &mut Vec<String>) -> Option<Vec<ProjectChoice>> {
    let rows = frame.complete::<tables::ProjectHistory>().map_err(|why| unavailable.push(format!("Project choice history is unavailable: {why}"))).ok()?;
    let first = rows.partition_point(|row| (row.owner, row.project) < (project.owner, project.local_id));
    let last = first + rows[first..].partition_point(|row| (row.owner, row.project) == (project.owner, project.local_id));
    Some(
        rows[first..last]
            .iter()
            .filter(|row| !(row.prototype_reward_handle == COMPLETION_MARKER && row.option_handle == COMPLETION_MARKER))
            .map(|row| ProjectChoice {
                ordinal: row.ordinal,
                date: row.date,
                reward: namer.token(frame, row.prototype_reward_handle, unavailable),
                option: namer.token(frame, row.option_handle, unavailable),
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
    frame: &DecodedFrame,
    owners: &BTreeSet<u32>,
    reads: &Reads<'_, '_>,
    namer: &mut Namer<'_, '_, '_>,
    tag: &str,
    unavailable: &mut Vec<String>,
) -> Option<Vec<ProjectRuntimeDetails>> {
    let rows = frame.complete::<tables::Projects>().map_err(|why| unavailable.push(format!("Projects are unavailable: {why}"))).ok()?;
    let slices = frame.complete::<tables::SliceReads>().map_err(|why| unavailable.push(format!("Project read hours are unavailable: {why}"))).ok()?;
    let mut read_of: BTreeMap<u32, i32> = BTreeMap::new();
    for &owner in owners {
        let has_rows = !prefixed(rows, owner, |row| row.owner).is_empty();
        match replay_data::sliced::read_hour(slices, TableId::Projects, owner) {
            Some(read) if reads.supplies(read) => {
                read_of.insert(owner, read);
            }
            _ if has_rows || owner == reads.holder.id => {
                unavailable.push("Projects were not read in the day up to this hour while this country held the tag".into());
                return None;
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (&owner, &read) in &read_of {
        for project in prefixed(rows, owner, |row| row.owner) {
            let token = spelled(frame, project.definition_id);
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
            out.push(ProjectRuntimeDetails {
                owner,
                id: project.local_id,
                label: ProjectLabel { name, token: token.clone().unwrap_or_else(|_| project.definition_id.to_string()) },
                read_hour: read,
                pending_known: runtime.is_some_and(|runtime| runtime.pending_known),
                pending: runtime.and_then(|runtime| runtime.pending_reward.map(|id| pending(frame, runtime, token.as_deref().map_err(String::as_str), id, namer))),
                seed: runtime.and_then(|runtime| runtime.variables.as_ref()).map(|store| store.seed),
                variables,
                choices: choices(frame, project, namer, unavailable),
            });
        }
    }
    Some(out)
}

fn effect_tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = dashboard_effects::effect_description::recorded_tables()
        .chain([TableId::RaidFacilities, TableId::Provinces, TableId::Projects, TableId::ProjectPrograms, TableId::LexicalTokens, TableId::CountryLinks])
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
    scientist: &Result<Option<u32>, String>,
) -> ProjectEffects {
    let render = || -> Result<_, String> {
        let catalogue = &content.politics.catalogue;
        let scripted = catalogue.scripted_content()?;
        let text = dashboard_text::scripted_text::ScriptedText::new(frame, &catalogue.localisation, scripted, slot);
        let cache = scripted.effects()?;
        let context = dashboard_effects::effect_description::Context::new(&text, &catalogue.localisation, slot, "special-project output", cache.view(None));
        let facility = (|| {
            if project.program_reference == 0 {
                return Ok(None);
            }
            if project.program_reference as u32 != PROGRAM_REFERENCE {
                return Err("Unknown program reference type".to_owned());
            }
            let id = (project.program_reference >> 32) as u32;
            let facility =
                frame.complete::<tables::RaidFacilities>().map_err(|why| why.to_string())?.iter().find(|row| row.id == id).ok_or("raid facilities lacks the selected project program")?;
            let provinces = frame.complete::<tables::Provinces>().map_err(|why| why.to_string())?;
            let state = provinces.iter().find(|row| row.id == facility.province).ok_or("Provinces lacks the facility province")?.state;
            Ok(Some((facility.province, state)))
        })();
        Ok(dashboard_effects::effect_description::project_output_description(output, &context, project.owner, project.local_id, scientist.clone(), facility))
    };
    match render() {
        Ok(mut description) => {
            let text = dashboard_base::catalogue::plain_text(&description.text).unwrap_or_else(|why| {
                *description.refusals.entry(why).or_default() += 1;
                String::new()
            });
            ProjectEffects { text, refusals: description.refusals }
        }
        Err(why) => ProjectEffects { text: String::new(), refusals: BTreeMap::from([(why, 1)]) },
    }
}

impl AtHour for SpecialProjectEffectsAt {
    const KEY: &'static str = "special-project-effects";
    const TITLE: &'static str = "Special project effects";
    const CONTENT_DEPENDENT: bool = true;
    type Params = SpecialProjectEffectsParams;
    type Reply = SpecialProjectEffects;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![SpecialProjectEffectsParams::decl(), SpecialProjectEffects::decl(), ProjectEffects::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        effect_tables()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(context(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Special project effects name no country"))?;
        let content = at.context.content.as_ref().map_err(|why| dashboard_core::refused(why.clone()))?;
        let mut out = SpecialProjectEffects { hour: at.hour, output: None, options: Vec::new(), unavailable: None, warnings: Vec::new() };
        if !owners_of(frame, owner.slot)?.contains(&params.owner) {
            out.unavailable = Some("The project's owner is not this country at this hour".into());
            return Ok(out);
        }
        let rows = frame.complete::<tables::Projects>()?;
        let key = (params.owner, params.project);
        let Some(project) = rows.binary_search_by_key(&key, |row| (row.owner, row.local_id)).ok().map(|at| &rows[at]) else {
            out.unavailable = Some("The project is not recorded at this hour".into());
            return Ok(out);
        };
        let token = match spelled(frame, project.definition_id) {
            Ok(token) => token,
            Err(why) => {
                out.unavailable = Some(why);
                return Ok(out);
            }
        };
        let Some(found) = definition(content, &token) else {
            out.unavailable = Some(format!("Definition {token} is absent from content"));
            return Ok(out);
        };
        let query = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
        let run = query.timeline.at(at.hour).ok_or_else(|| dashboard_core::refused("Special project effects hour is not recorded"))?.run;
        let reads = Reads { query: &query, coverage: &at.context.coverage, tag: &params.country, holder: owner, run, hour: at.hour };
        let scientist = (|| {
            if project.program_reference as u32 != PROGRAM_REFERENCE {
                return Err("Scientist assignment has no supplying read at this hour".to_owned());
            }
            let id = (project.program_reference >> 32) as u32;
            let read = reads.last(TableId::ProjectPrograms, "Program assignments")?;
            if read != at.hour {
                return Err("Scientist assignment has no supplying read at this hour".to_owned());
            }
            let programs = frame.complete::<tables::ProjectPrograms>().map_err(|why| why.to_string())?;
            let program = programs.binary_search_by_key(&id, |row| row.id).ok().map(|at| &programs[at]).ok_or("Scientist assignment has no supplying read at this hour")?;
            match program.scientist {
                0 => Ok(None),
                reference if reference as u32 == CHARACTER_REFERENCE => Ok(Some((reference >> 32) as u32)),
                _ => Err("Unknown scientist reference type".to_owned()),
            }
        })();
        out.output = Some(effects(content, frame, owner.slot, project, &found.output, &scientist));
        if let Recorded::Value(runtime) = &project.runtime
            && let (Some(id), Some(ordinal)) = (runtime.pending_reward, runtime.pending_ordinal)
        {
            match spelled(frame, id).and_then(|reward| content.definitions.reward_at(&token, ordinal, &reward)) {
                Ok(reward) => out.options = reward.options.iter().map(|option| effects(content, frame, owner.slot, project, &option.output, &scientist)).collect(),
                Err(why) => out.unavailable = Some(why),
            }
        }
        Ok(out)
    }
}
