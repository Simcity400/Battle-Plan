
use dashboard_base::products::{Country, validate_country};
use dashboard_base::{products::{LoadContext, Product, at_hour::{self, Read}, character_portraits::{portrait_png, recorded_portrait_in}, politics_content::{PoliticsContent, politics_content}}, queries::country_ids::{self, RunLinks}};
use dashboard_world::products::identities::CountryIdentityRun;
use dashboard_base::{Source, catalogue::{Refusals, View}, span::HourSpan};
use dashboard_query::{Query, identity::Holder};
use content_reader_military::special_projects::{Catalog, Project};
use content_reader_localisation::localisation::Argument;
use replay_lanes::lanes::{self, Cell, History, Keys, Step};
use replay_columnar::{manifest::Status, state::Kind, store::Value};
use replay_recording::{
    cells::TableReader,
    columnar_frames::{Recording, Run as RecordingRun},
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{TableId, tables},
};
use replay_frame::frame::{DecodedFrame, Rowset};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub mod details;
mod facts;
mod incremental;
#[cfg(test)]
mod tests;

use facts::{Entry, Facts, Person, Program, Row};

const ITERATIONS_KEY: &str = "ONGOING_PROJECT_BREAKTHROUGHS_NUMBER";
const PLACEHOLDER_ICON: &str = "GFX_PLACEHOLDER_sp_project_icon";
const ICON_EDGE: u32 = 96;
const PROGRAM_REFERENCE: u32 = 83;
const CHARACTER_REFERENCE: u32 = 73;

const PROJECT_COLUMNS: [usize; 6] = [0, 1, 2, 4, 9, 12];
const COMPLETION_PROJECT_COLUMNS: [usize; 3] = [0, 4, 9];
const HISTORY_COLUMNS: [usize; 3] = [0, 1, 2];
const PROGRAM_COLUMNS: [usize; 3] = [0, 2, 3];

pub struct SpecialProjects;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialProjectsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    pub branch_id: String,
    pub country: Country,
    #[doc = " Only the projects shown at the hour, each with the statuses and markers that decide it: the Current view."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub current: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectPerson {
    #[doc = " None where the recorded reference cannot say whether a scientist is assigned."]
    pub assigned: Option<bool>,
    pub id: Option<u32>,
    pub name: Option<String>,
    pub portrait: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectPresentation {
    pub name: String,
    pub specialization: Option<String>,
    pub specialization_name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    #[doc = " Its index in `people`: the scientist the program read names, no one, or why it is unknown."]
    pub scientist: u32,
    #[doc = " Where the program is not read now or names no one: the last scientist observed, its index in `people`."]
    pub last_scientist: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProjectState {
    Running,
    Paused,
    Cancelled,
    Completed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatus {
    #[doc = " Hours the list was published with this status."]
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The first Projects read that showed this status."]
    pub read_hour: i32,
    #[doc = " Its index in `presentations`."]
    pub presentation: u32,
    #[doc = " Prototype progress, percent, clamped to -100..=100."]
    pub progress: i32,
    pub state: ProjectState,
    #[doc = " Choices after the last completion in the history read current at these hours; none where that read is not usable, or lists fewer entries than an earlier one."]
    pub iterations: Option<i32>,
    #[doc = " The program read the scientist comes from; none where no program read names the assignment."]
    pub scientist_read: Option<i32>,
    #[doc = " The program read that last named `lastScientist`."]
    pub last_scientist_read: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OwnerRead {
    #[doc = " The Projects read of a raw owner that supplied the list at these hours."]
    pub owner: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub ordinal: u32,
    #[doc = " The game's date of the choice or completion."]
    pub date: i32,
    pub completion: bool,
    #[doc = " The history read that first listed it."]
    pub read_hour: i32,
    #[doc = " A completion dated within the campaign, the country's custody and its read."]
    pub counted: bool,
    #[doc = " A counted completion's name: its index in `presentations`, from the completed Projects row usable at the read that first listed it, where nothing follows it in that read; none otherwise."]
    pub presentation: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectAttempt {
    #[doc = " A new attempt history proves: its first entry, the read proving it and, where it is proven by a changed entry, the earlier read it changed."]
    pub ordinal: u32,
    pub date: i32,
    pub read_hour: i32,
    pub since_read: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    #[doc = " The project's raw owner id and its id within it, as the project detail products key it."]
    pub owner: u32,
    pub local: u32,
    #[doc = " The first usable Projects read of the project, published or not, and its state; for a project seen only in history, its first history read and an unknown state."]
    pub first_read: i32,
    pub first_state: ProjectState,
    pub statuses: Vec<ProjectStatus>,
    pub entries: Vec<ProjectEntry>,
    pub attempts: Vec<ProjectAttempt>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectsReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The tag's holders over the branch."]
    pub identities: Vec<CountryIdentityRun>,
    #[doc = " The hours the tag's holder is unknown: whether it had projects is unknown."]
    pub unresolved: Vec<HourSpan>,
    #[doc = " The hours the holders' projects were recorded."]
    pub available: Vec<HourSpan>,
    #[doc = " The hours the holders' projects were read only in part: no list is shown for them."]
    pub partial: Vec<HourSpan>,
    pub projects: Vec<ProjectRecord>,
    #[doc = " Each owner's Projects read at the published hours, by owner then hour."]
    pub reads: Vec<OwnerRead>,
    pub presentations: Vec<ProjectPresentation>,
    pub people: Vec<ProjectPerson>,
    #[doc = " The game's text for each iteration count shown, none where it cannot be made."]
    pub iteration_texts: BTreeMap<i32, Option<String>>,
    #[doc = " Why nothing is shown at any hour; the hours outside `available` say for themselves why they show nothing."]
    pub unavailable: Option<String>,
    pub shown: ProjectsShown,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShownCompleted {
    pub project: u32,
    pub date: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectsShown {
    pub through: Option<i32>,
    pub running: Vec<u32>,
    #[doc = " Projects whose state was read but not understood: neither in progress nor completed."]
    pub unknown: Vec<u32>,
    pub completed: Vec<ShownCompleted>,
    pub undated: u32,
}

pub(crate) fn shown_at(projects: &[ProjectRecord], presentations: &[ProjectPresentation], available: &[HourSpan], hour: i32) -> ProjectsShown {
    let mut shown = ProjectsShown { through: Some(hour), ..ProjectsShown::default() };
    if !available.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour) {
        return shown;
    }
    let name = |run: &ProjectStatus| presentations.get(run.presentation as usize).map_or("", |info| info.name.as_str());
    let mut running = Vec::new();
    let mut unknown = Vec::new();
    let mut dated = Vec::new();
    let mut undated = Vec::new();
    for (index, project) in projects.iter().enumerate() {
        let Some(at) = project.statuses.iter().position(|run| run.first_hour <= hour && hour <= run.last_hour) else { continue };
        let run = &project.statuses[at];
        match run.state {
            ProjectState::Cancelled => {}
            ProjectState::Unknown => unknown.push((name(run), project.id.as_str(), index as u32)),
            ProjectState::Completed => {
                let mut first = at;
                while first > 0 && project.statuses[first - 1].state == ProjectState::Completed && project.statuses[first - 1].last_hour.saturating_add(1) == project.statuses[first].first_hour {
                    first -= 1;
                }
                let completed = &project.statuses[first];
                let attempt = first
                    .checked_sub(1)
                    .map(|before| &project.statuses[before])
                    .filter(|before| matches!(before.state, ProjectState::Running | ProjectState::Paused) && before.last_hour.saturating_add(1) == completed.first_hour);
                let date = attempt.and_then(|before| {
                    project
                        .entries
                        .iter()
                        .filter(|entry| entry.completion && entry.counted && entry.date > before.last_hour && entry.date <= completed.first_hour && entry.read_hour <= hour)
                        .map(|entry| entry.date)
                        .max()
                });
                match date {
                    Some(date) => dated.push((date, name(run), project.id.as_str(), index as u32)),
                    None => undated.push((name(run), project.id.as_str(), index as u32)),
                }
            }
            _ => running.push((run.progress, project.first_read, name(run), project.id.as_str(), index as u32)),
        }
    }
    running.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)).then(a.3.cmp(b.3)));
    dated.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)).then(a.2.cmp(b.2)));
    undated.sort();
    unknown.sort();
    shown.running = running.into_iter().map(|item| item.4).collect();
    shown.unknown = unknown.into_iter().map(|item| item.2).collect();
    shown.undated = undated.len() as u32;
    shown.completed = dated.into_iter().map(|item| ShownCompleted { project: item.3, date: Some(item.0) }).chain(undated.into_iter().map(|item| ShownCompleted { project: item.2, date: None })).collect();
    shown
}

dashboard_base::reply_warnings!(SpecialProjectsReply);

pub(crate) struct ProjectContent {
    pub politics: Arc<PoliticsContent>,
    pub definitions: Arc<Catalog>,
    icons: Mutex<HashMap<String, Result<String, String>>>,
}

#[derive(Default)]
struct State {
    held: Option<Arc<ProjectContent>>,
}

pub(crate) fn content(source: &Source, load: &LoadContext) -> io::Result<Arc<ProjectContent>> {
    let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused("Special projects need the recorded content stack"))?;
    let politics = politics_content(source, options)?;
    let shared = source.state::<State>();
    let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(held) = state.held.as_ref().filter(|held| Arc::ptr_eq(&held.politics, &politics)) {
        return Ok(Arc::clone(held));
    }
    let definitions = Arc::new(Catalog::read(&politics.catalogue.content.layers)?);
    let held = Arc::new(ProjectContent { politics, definitions, icons: Mutex::default() });
    state.held = Some(Arc::clone(&held));
    Ok(held)
}

fn country_key(content: &ProjectContent, tag: &str, key: &str) -> String {
    let prefixed = format!("{tag}_{key}");
    if content.politics.catalogue.localisation.get(&prefixed).is_some() { prefixed } else { key.to_owned() }
}

fn needs_scope(content: &ProjectContent, key: &str) -> bool {
    content.politics.catalogue.localisation.localise(key.as_bytes()).is_ok_and(|text| text.contains('['))
}

fn icon_key(authored: Option<&str>, token: &str, tag: &str, has: impl Fn(&str) -> Result<bool, String>) -> Result<String, String> {
    if let Some(icon) = authored.filter(|icon| !icon.is_empty()) {
        return Ok(icon.to_owned());
    }
    let prefix = if token.starts_with("GFX_") { "" } else { "GFX_" };
    let country = format!("{prefix}{tag}_{token}");
    if has(&country)? {
        return Ok(country);
    }
    let generic = format!("{prefix}{token}");
    Ok(if has(&generic)? { generic } else { PLACEHOLDER_ICON.to_owned() })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Named {
    pub name: String,
    pub specialization: Option<String>,
    pub specialization_name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

impl Named {
    fn unread(why: impl Into<String>) -> Self {
        Self { name: "Project name unavailable".into(), specialization: None, specialization_name: None, icon: None, icon_missing: Some(why.into()) }
    }
}

pub(crate) struct Naming<'c> {
    pub content: &'c ProjectContent,
    pub refusals: Mutex<Refusals>,
    pub art: BTreeMap<String, u64>,
}

impl<'c> Naming<'c> {
    pub fn new(content: &'c ProjectContent) -> Self {
        Self { content, refusals: Mutex::default(), art: BTreeMap::new() }
    }

    pub fn note(&self, reason: impl Into<String>, identity: impl Into<String>) {
        self.refusals.lock().unwrap_or_else(|error| error.into_inner()).note(reason, identity);
    }

    fn icon(&mut self, key: &str) -> Result<String, String> {
        let content: &'c ProjectContent = self.content;
        let drawn = content
            .icons
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(key.to_owned())
            .or_insert_with(|| match content.politics.sprite_png_sized(key, ICON_EDGE) {
                Ok(Some(icon)) => Ok(icon),
                Ok(None) => Err(format!("Content has no sprite {key}.")),
                Err(error) => Err(format!("Sprite {key} could not be drawn: {error}")),
            })
            .clone();
        if let Err(reason) = &drawn {
            *self.art.entry(reason.clone()).or_default() += 1;
        }
        drawn
    }

    pub fn definition(&mut self, token: Option<&str>, id: u32, tag: &str, scoped: &mut dyn FnMut(&str) -> Result<Option<String>, String>) -> Named {
        let Some(token) = token else {
            self.note("its definition token is not recorded", id.to_string());
            return Named::unread(format!("Definition {id} is not recorded."));
        };
        let content: &'c ProjectContent = self.content;
        let definitions = &content.definitions;
        if let Some(reason) = definitions.unsettled.as_ref().or_else(|| definitions.projects.get(token).and_then(|project| project.refusal.as_ref())) {
            self.note(reason.clone(), token);
            return Named::unread(reason.clone());
        }
        let Some(definition) = definitions.projects.get(token) else {
            self.note("the definition is absent from content", token);
            return Named::unread(format!("Definition {token} is absent from content."));
        };
        let mut named = Named::unread("");
        named.icon_missing = None;
        match self.name(definition, token, tag, scoped) {
            Some(name) => named.name = name,
            None => self.note("its name could not be resolved", token),
        }
        named.specialization = definition.specialization.clone();
        if let Some(kind) = &definition.specialization {
            if definitions.specializations.contains(kind) {
                let key = country_key(content, tag, kind);
                named.specialization_name = content.politics.catalogue.localised_with_refusals(&key, &[], View::Plain, &self.refusals);
            } else {
                self.note(format!("its specialization {kind} is absent from content"), token);
            }
        }
        let sprites = &content.politics.catalogue.sprites;
        let key = icon_key(definition.icon.as_deref(), token, tag, |key| sprites.find(key).map(|sprite| sprite.is_some()).map_err(|error| format!("Sprite {key}: {error}")));
        match key.and_then(|key| self.icon(&key)) {
            Ok(icon) => named.icon = Some(icon),
            Err(reason) => named.icon_missing = Some(reason),
        }
        named
    }

    fn name(&self, definition: &Project, token: &str, tag: &str, scoped: &mut dyn FnMut(&str) -> Result<Option<String>, String>) -> Option<String> {
        let authored = definition.name.as_deref().filter(|name| !name.is_empty()).unwrap_or(token);
        let key = country_key(self.content, tag, authored);
        if needs_scope(self.content, &key) {
            return match scoped(&key) {
                Ok(name) => name,
                Err(reason) => {
                    self.note(reason, token);
                    None
                }
            };
        }
        self.content.politics.catalogue.localised_with_refusals(&key, &[], View::Scripted, &self.refusals)
    }

    pub fn iterations(&self, count: i32) -> Option<String> {
        self.content.politics.catalogue.localised_with_refusals(ITERATIONS_KEY, &[("NUMBER", Argument::Int32(count))], View::Plain, &self.refusals)
    }

    pub fn person(&mut self, frame: &DecodedFrame, id: u32) -> ProjectPerson {
        let mut person = ProjectPerson { assigned: Some(true), id: Some(id), name: None, portrait: None, missing: None };
        let character = frame.complete::<tables::Characters>().ok().and_then(|rows| rows.binary_search_by_key(&id, |row| row.id).ok().map(|at| &rows[at]));
        let Some(character) = character else {
            let reason = format!("Character {id} was not recorded.");
            *self.art.entry("A scientist's character was not recorded.".into()).or_default() += 1;
            person.missing = Some(reason);
            return person;
        };
        let lexical = frame.complete::<tables::LexicalTokens>().unwrap_or(&[]);
        let script = dashboard_text::identity::character_script_token_in(character, Rowset::Decoded(lexical));
        let saved = String::from_utf8_lossy(&character.token);
        person.name = self.content.politics.character_name_with_refusals(script, &saved, &self.refusals);
        let portraits = frame.complete::<tables::CharacterPortraits>().ok().map(Rowset::Decoded);
        let portrait = recorded_portrait_in(portraits, id, 4, replay_data::rows::PORTRAIT_LARGE);
        match portrait_png(&self.content.politics, &portrait) {
            Ok(portrait) => person.portrait = Some(portrait),
            Err(reason) => {
                *self.art.entry(reason.clone()).or_default() += 1;
                person.missing = Some(reason);
            }
        }
        person
    }

    pub fn warnings(self) -> Vec<String> {
        let mut out = self.refusals.into_inner().unwrap_or_else(|error| error.into_inner()).lines("Project texts");
        out.extend(self.art.into_iter().map(|(reason, count)| format!("{reason} (count: {count})")));
        out
    }
}

pub(crate) fn spellings(frame: &DecodedFrame, ids: impl IntoIterator<Item = u32>) -> HashMap<u32, String> {
    let Ok(rows) = frame.complete::<tables::LexicalTokens>() else { return HashMap::new() };
    ids.into_iter()
        .filter_map(|id| {
            let row = rows.binary_search_by_key(&id, |row| row.token).ok().map(|at| &rows[at])?;
            let text = std::str::from_utf8(content_reader_parser::parser::c_string(&row.literal)).ok()?;
            Some((id, text.to_owned()))
        })
        .collect()
}

fn scoped_at(query: &Query<'_>, content: &ProjectContent, key: &str, hour: i32, slot: u32) -> Result<Option<String>, String> {
    let mut frames = None;
    scoped_at_in(query, &mut frames, content, key, hour, slot)
}

fn scoped_at_in<'q>(query: &'q Query<'q>, frames: &mut Option<at_hour::Frames<'q>>, content: &ProjectContent, key: &str, hour: i32, slot: u32) -> Result<Option<String>, String> {
    let catalogue = &content.politics.catalogue;
    let scripted = catalogue.scripted_content()?;
    let reads: Vec<Read> = dashboard_text::scripted_text::TABLES.iter().map(|&table| Read::all(table)).collect();
    let held = match frames {
        Some(held) => held,
        None => frames.insert(at_hour::Frames::new(query).map_err(|error| format!("The project's name needs its country's scope: {error}"))?),
    };
    let frame = held.frame(hour, &reads, "Special project name").map_err(|error| format!("The project's name needs its country's scope: {error}"))?;
    let text = dashboard_text::scripted_text::ScriptedText::new(&frame, &catalogue.localisation, scripted, slot);
    Ok(catalogue.localise_in(key, &text))
}

struct Cursor<'a> {
    steps: &'a [Step],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(steps: &'a [Step]) -> Self {
        Self { steps, at: 0 }
    }

    fn at(&mut self, hour: i32) -> Option<&'a [Cell]> {
        self.step_at(hour).map(|(_, cells)| cells)
    }

    fn step_at(&mut self, hour: i32) -> Option<(usize, &'a [Cell])> {
        while self.at + 1 < self.steps.len() && self.steps[self.at + 1].hour <= hour {
            self.at += 1;
        }
        self.steps.get(self.at).filter(|step| step.hour <= hour).and_then(|step| step.cells.as_deref().map(|cells| (self.at, cells)))
    }

    fn next_change(&self, hour: i32) -> i32 {
        match self.steps.get(self.at) {
            Some(step) if step.hour > hour => step.hour,
            Some(_) => self.steps.get(self.at + 1).map_or(i32::MAX, |step| step.hour),
            None => i32::MAX,
        }
    }
}

fn number(cell: Cell) -> Option<i64> {
    match cell {
        Cell::Number(value) => Some(value),
        _ => None,
    }
}

fn project_row(owner: u32, local: u32, cells: &[Cell]) -> Option<Row> {
    let value = |at: usize| cells.get(at).copied().and_then(number);
    Some(Row {
        owner,
        local,
        definition: value(0)? as u32,
        program: value(1)? as u64,
        started: value(2)? != 0,
        offset: value(3)? as u32,
        not_completed: value(4)? as u8,
        prototype_progress: value(5)? as i32,
    })
}

fn cell_number(kind: Kind, value: Value<'_>) -> Option<i64> {
    let Value::Number(bits) = value else { return None };
    Some(match kind {
        Kind::U8 => i64::from(bits as u8),
        Kind::I8 => i64::from(bits as i8),
        Kind::U16 => i64::from(bits as u16),
        Kind::I16 => i64::from(bits as i16),
        Kind::U32 | Kind::OptionalU32 => i64::from(bits as u32),
        Kind::I32 | Kind::OptionalI32 => i64::from(bits as i32),
        Kind::Bool => (bits & 1) as i64,
        _ => bits as i64,
    })
}

fn project_row_at(
    recording: &Recording,
    held_run: &RecordingRun,
    reader: &mut TableReader,
    owner: u32,
    local: u32,
    hour: i32,
) -> io::Result<Option<Row>> {
    let Some(ordinal) = held_run.ordinal(hour) else { return Ok(None) };
    dashboard_query::cost::storage(|| reader.go_to(recording, ordinal))?;
    if !reader.available() {
        return Ok(None);
    }
    let view = reader.view();
    let key = u128::from(owner) << 32 | u128::from(local);
    let Some(slot) = view.slot(key) else { return Ok(None) };
    if reader.read_rows().is_some_and(|read| read.binary_search(&slot).is_err()) {
        return Ok(None);
    }
    let kinds = reader.kinds();
    let val = |col: usize| kinds.get(col).and_then(|&k| cell_number(k, view.value(col, slot)));
    let (Some(definition), Some(offset), Some(not_completed)) = (
        val(COMPLETION_PROJECT_COLUMNS[0]),
        val(COMPLETION_PROJECT_COLUMNS[1]),
        val(COMPLETION_PROJECT_COLUMNS[2]),
    ) else {
        return Ok(None);
    };
    Ok(Some(Row { owner, local, definition: definition as u32, program: 0, started: false, offset: offset as u32, not_completed: not_completed as u8, prototype_progress: 0 }))
}

fn history_entry(ordinal: u32, cells: &[Cell]) -> Option<Entry> {
    let value = |at: usize| cells.get(at).copied().and_then(number);
    Some(Entry { ordinal, reward: value(0)? as u32, option: value(1)? as u32, date: value(2)? as i32 })
}

fn statuses(query: &Query<'_>, run: u32, table: TableId, first: i32, last: i32, kept: impl Fn(Status) -> bool) -> io::Result<Vec<(i32, i32)>> {
    let runs = dashboard_query::cost::storage(|| lanes::statuses(query.replay.recording(), run, table, first, last))?;
    Ok(runs.into_iter().filter(|run| kept(run.status)).map(|run| (run.first, run.last)).collect())
}

fn available(query: &Query<'_>, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    statuses(query, run, table, first, last, |status| status == Status::Complete)
}

fn within(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.1 < hour);
    spans.get(at).is_some_and(|span| span.0 <= hour)
}

fn next_span_edge(spans: &[(i32, i32)], hour: i32) -> i32 {
    let at = spans.partition_point(|span| span.1 < hour);
    match spans.get(at) {
        Some(&(first, last)) if first <= hour => last.saturating_add(1),
        Some(&(first, _)) => first,
        None => i32::MAX,
    }
}

fn uncovered(spans: &[(i32, i32)], first: i32, last: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut at = first;
    for &(a, b) in spans {
        if a > at {
            out.push((at, a - 1));
        }
        at = at.max(b.saturating_add(1));
    }
    if at <= last {
        out.push((at, last));
    }
    out
}

pub(super) fn held_from(query: &Query<'_>, tag: &Country, slot: u32, id: u32, hour: i32) -> Option<i32> {
    let held = |at: i32| query.holders.at(tag, at).filter(|run| run.holder == (Holder::Found { slot, id }));
    let mut since = held(hour)?.first;
    while let Some(run) = since.checked_sub(1).and_then(held) {
        since = run.first;
    }
    Some(since)
}

fn owner_ids(links: &RunLinks, slot: u32, first: i32, last: i32) -> BTreeMap<u32, Vec<(i32, i32)>> {
    let mut out: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
    for (&raw, runs) in &links.slots {
        for &(a, b, held) in runs {
            if held == Some(i64::from(slot)) && a <= last && b >= first {
                out.entry(raw).or_default().push((a.max(first), b.min(last)));
            }
        }
    }
    for spans in out.values_mut() {
        spans.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
        for &(a, b) in spans.iter() {
            facts::extend(&mut merged, a, b);
        }
        *spans = merged;
    }
    out
}

pub(super) struct Custody {
    held_from: i32,
    owners: BTreeMap<u32, Vec<(i32, i32)>>,
}

impl Custody {
    pub(super) fn new(links: &RunLinks, slot: u32, held_from: i32, through: i32) -> Self {
        Self { held_from, owners: owner_ids(links, slot, held_from, through) }
    }

    pub(super) fn held(&self, raw: u32, from: i32, through: i32) -> bool {
        self.held_from <= from && from <= through && self.owners.get(&raw).is_some_and(|spans| spans.iter().any(|&(first, last)| first <= from && through <= last))
    }

    pub(super) fn usable(&self, raw: u32, read: Option<i32>, complete: &[(i32, i32)], hour: i32) -> Option<i32> {
        read.filter(|&read| current(read, hour) && within(complete, read) && self.held(raw, read, hour))
    }

    fn linked(&self, hour: i32) -> impl Iterator<Item = u32> + '_ {
        self.owners.iter().filter(move |(_, spans)| within(spans, hour)).map(|(&raw, _)| raw)
    }

    fn next_change(&self, hour: i32) -> i32 {
        self.owners.values().map(|spans| next_span_edge(spans, hour)).min().unwrap_or(i32::MAX)
    }
}

pub(super) struct Assignment {
    pub owner: Option<u32>,
    pub active: Option<u32>,
    pub scientist: Option<u64>,
}

pub(super) fn assignment(owner: u32, local: u32, program: u64, read: Option<(i32, Option<Assignment>)>) -> Program {
    if program == 0 {
        return Program::Unassigned;
    }
    if program as u32 != PROGRAM_REFERENCE {
        return Program::Unknown("The project's program reference has an unknown native type.".into());
    }
    let id = (program >> 32) as u32;
    let Some((read, Some(cells))) = read else { return Program::Unread(id) };
    if cells.owner != Some(owner) || cells.active != Some(local) {
        return Program::Unknown(format!("Program {id} was read for another project."));
    }
    match cells.scientist {
        Some(scientist) => Program::Read { read, scientist },
        None => Program::Unread(id),
    }
}

fn program_cells(cells: &[Cell]) -> Assignment {
    let value = |at: usize| cells.get(at).copied().and_then(number);
    Assignment { owner: value(0).and_then(|owner| u32::try_from(owner).ok()), active: value(1).and_then(|local| u32::try_from(local).ok()), scientist: value(2).map(|scientist| scientist as u64) }
}

struct Piece {
    run: u32,
    first: i32,
    last: i32,
    slot: u32,
    id: u32,
    held_from: i32,
    opened: i32,
}

impl Piece {
    fn reach(&self) -> i32 {
        self.first.saturating_sub(STAGGER_PERIOD - 1).max(self.held_from).max(self.opened)
    }
}

struct PieceRows {
    projects: Vec<History>,
    histories: Vec<History>,
    programs: History,
    reads: History,
    owners: Vec<u32>,
    custody: Custody,
    projects_complete: Vec<(i32, i32)>,
    projects_partial: Vec<(i32, i32)>,
    history_complete: Vec<(i32, i32)>,
    roster: Vec<(i32, i32)>,
    program_reads: Vec<(i32, i32)>,
}

fn read_piece(query: &Query<'_>, links: &RunLinks, piece: &Piece) -> io::Result<PieceRows> {
    let recording = query.replay.recording();
    let custody = Custody::new(links, piece.slot, piece.held_from, piece.last);
    let owners: Vec<u32> = custody.owners.iter().filter(|(_, spans)| spans.iter().any(|&(_, last)| last >= piece.reach())).map(|(&raw, _)| raw).collect();
    let mut projects = Vec::new();
    let mut histories = Vec::new();
    for &raw in &owners {
        let raw = u128::from(raw);
        projects.push(dashboard_query::cost::storage(|| {
            lanes::history(recording, piece.run, TableId::Projects, &PROJECT_COLUMNS, Keys::Range(raw << 32, raw << 32 | u128::from(u32::MAX)), piece.reach(), piece.last)
        })?);
        histories.push(dashboard_query::cost::storage(|| {
            lanes::history(recording, piece.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::Range(raw << 64, raw << 64 | u128::from(u64::MAX)), piece.reach(), piece.last)
        })?);
    }
    let mut programs: BTreeSet<u128> = BTreeSet::new();
    for history in &projects {
        for (_, steps) in &history.keys {
            for cells in steps.iter().filter_map(|step| step.cells.as_deref()) {
                if let Some(program) = cells.get(1).copied().and_then(number).map(|value| value as u64)
                    && program as u32 == PROGRAM_REFERENCE
                {
                    programs.insert(u128::from((program >> 32) as u32));
                }
            }
        }
    }
    let reads: Vec<u128> = owners.iter().flat_map(|&raw| [read_key(TableId::Projects, raw), read_key(TableId::ProjectHistory, raw)]).collect();
    let sample = read_columns()?;
    let reads = dashboard_query::cost::storage(|| lanes::history(recording, piece.run, TableId::SliceReads, &sample, Keys::These(&reads), piece.first, piece.last))?;
    let programs: Vec<u128> = programs.into_iter().collect();
    let (programs, program_reads) = if programs.is_empty() {
        (History::default(), Vec::new())
    } else {
        (
            dashboard_query::cost::storage(|| lanes::history(recording, piece.run, TableId::ProjectPrograms, &PROGRAM_COLUMNS, Keys::These(&programs), piece.reach(), piece.last))?,
            dashboard_query::cost::storage(|| lanes::complete_reads(recording, piece.run, TableId::ProjectPrograms, piece.reach(), piece.last))?,
        )
    };
    at_hour::count_rows(|| projects.iter().chain(&histories).chain([&programs, &reads]).flat_map(|h| &h.keys).map(|(_, steps)| steps.len() as u64).sum());
    Ok(PieceRows {
        projects,
        histories,
        programs,
        reads,
        owners,
        custody,
        projects_complete: available(query, piece.run, TableId::Projects, piece.reach(), piece.last)?,
        projects_partial: statuses(query, piece.run, TableId::Projects, piece.first, piece.last, |status| matches!(status, Status::Truncated { .. }))?,
        history_complete: available(query, piece.run, TableId::ProjectHistory, piece.reach(), piece.last)?,
        roster: available(query, piece.run, TableId::CountryLinks, piece.first, piece.last)?,
        program_reads,
    })
}

pub(super) fn read_columns() -> io::Result<[usize; 1]> {
    Ok([dashboard_declare::field(TableId::SliceReads, "sample_hour")?])
}

pub(super) fn read_key(table: TableId, raw: u32) -> u128 {
    (table.index() as u128) << 32 | u128::from(raw)
}

pub(super) fn current(read: i32, hour: i32) -> bool {
    read <= hour && hour - read < STAGGER_PERIOD
}

pub(super) fn sample(cells: Option<&[Cell]>) -> Option<i32> {
    cells.and_then(|cells| cells.first().copied()).and_then(number).and_then(|hour| i32::try_from(hour).ok())
}

fn last_read_next(reads: &[(i32, i32)], hour: i32) -> (Option<i32>, i32) {
    let at = reads.partition_point(|read| read.0 <= hour);
    let mut next = reads.get(at).map_or(i32::MAX, |read| read.0);
    let found = at.checked_sub(1).and_then(|at| {
        let (_, end) = reads[at];
        if hour < end {
            next = next.min(hour.saturating_add(1));
        }
        let read = end.min(hour);
        if current(read, hour) {
            if hour >= end {
                next = next.min(read.saturating_add(STAGGER_PERIOD));
            }
            Some(read)
        } else {
            None
        }
    });
    (found, next)
}

fn stamp_at(stamps: &mut HashMap<u128, Cursor<'_>>, table: TableId, raw: u32, hour: i32, next: &mut i32) -> Option<i32> {
    let cursor = stamps.get_mut(&read_key(table, raw))?;
    let found = sample(cursor.at(hour));
    *next = (*next).min(cursor.next_change(hour));
    if let Some(read) = found {
        if read > hour {
            *next = (*next).min(read);
        } else if current(read, hour) {
            *next = (*next).min(read.saturating_add(STAGGER_PERIOD));
        }
    }
    found
}

fn see_piece(facts: &mut Facts, piece: &Piece, read: &PieceRows, cancelled: &AtomicBool) -> io::Result<()> {
    let mut projects: Vec<(u32, u32, Cursor<'_>)> = Vec::new();
    for (history, &raw) in read.projects.iter().zip(&read.owners) {
        projects.extend(history.keys.iter().map(|(key, steps)| (raw, *key as u32, Cursor::new(steps))));
    }
    let mut entries: Vec<(u32, u32, u32, Cursor<'_>)> = Vec::new();
    for history in &read.histories {
        for (key, steps) in &history.keys {
            entries.push(((key >> 64) as u32, (key >> 32) as u32, *key as u32, Cursor::new(steps)));
        }
    }
    let mut programs: HashMap<u32, Cursor<'_>> = read.programs.keys.iter().map(|(key, steps)| (*key as u32, Cursor::new(steps))).collect();
    let mut stamps: HashMap<u128, Cursor<'_>> = read.reads.keys.iter().map(|(key, steps)| (*key, Cursor::new(steps))).collect();
    let custody = &read.custody;
    let mut linked: Vec<u32> = Vec::new();
    let mut projects_read: BTreeMap<u32, i32> = BTreeMap::new();
    let mut histories_read: BTreeMap<u32, i32> = BTreeMap::new();
    let mut listed: BTreeMap<(u32, u32), Vec<Entry>> = BTreeMap::new();
    let mut rows: Vec<(Row, Program, i32)> = Vec::new();
    let mut hour = piece.first;
    while hour <= piece.last {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut next = custody.next_change(hour);
        next = next.min(next_span_edge(&read.roster, hour));
        next = next.min(next_span_edge(&read.projects_partial, hour));
        linked.clear();
        linked.extend(custody.linked(hour));
        projects_read.clear();
        histories_read.clear();
        for &raw in &linked {
            let p = stamp_at(&mut stamps, TableId::Projects, raw, hour, &mut next);
            if let Some(stamp) = custody.usable(raw, p, &read.projects_complete, hour) {
                projects_read.insert(raw, stamp);
            }
            let h = stamp_at(&mut stamps, TableId::ProjectHistory, raw, hour, &mut next);
            if let Some(stamp) = custody.usable(raw, h, &read.history_complete, hour) {
                histories_read.insert(raw, stamp);
            }
        }
        listed.clear();
        if !histories_read.is_empty() {
            for (owner, project, ordinal, cursor) in &mut entries {
                if let Some(&stamp) = histories_read.get(owner)
                    && let Some(entry) = cursor.at(stamp).and_then(|cells| history_entry(*ordinal, cells))
                {
                    listed.entry((*owner, *project)).or_default().push(entry);
                }
            }
        }
        let (program_read, program_next) = last_read_next(&read.program_reads, hour);
        let mut uses_program = false;
        rows.clear();
        if !projects_read.is_empty() {
            for (raw, local, cursor) in &mut projects {
                let (raw, local) = (*raw, *local);
                if let Some(&stamp) = projects_read.get(&raw)
                    && let Some(row) = cursor.at(stamp).and_then(|cells| project_row(raw, local, cells))
                {
                    if row.program != 0 && row.program as u32 == PROGRAM_REFERENCE {
                        uses_program = true;
                    }
                    let cells = program_read.filter(|&at| custody.held(raw, at, hour)).map(|at| (at, programs.get_mut(&((row.program >> 32) as u32)).and_then(|cursor| cursor.at(at)).map(program_cells)));
                    let program = assignment(raw, local, row.program, cells);
                    rows.push((row, program, stamp));
                }
            }
        }
        if uses_program {
            next = next.min(program_next);
        }
        let span_last = next.saturating_sub(1).min(piece.last).max(hour);
        let shown: Vec<facts::Read<'_>> = rows.iter().map(|(row, program, stamp)| facts::Read { row, read: *stamp, program: program.clone() }).collect();
        let published = within(&read.roster, hour) && projects_read.len() == linked.len();
        if !published && within(&read.projects_partial, hour) {
            facts::extend(&mut facts.partial, hour, span_last);
        }
        facts.hour_span(
            &facts::Hour {
                country: (piece.slot, piece.id),
                at: hour,
                held: &|raw, from, through| custody.held(raw, from, through),
                projects: &projects_read,
                histories: &histories_read,
                listed: &listed,
                reads: &shown,
                published,
            },
            span_last,
        );
        if span_last == i32::MAX {
            break;
        }
        hour = span_last + 1;
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Holding {
    identities: Vec<CountryIdentityRun>,
    unresolved: Vec<(i32, i32)>,
}

fn pieces(query: &Query<'_>, country: &Country, from: i32, through: i32) -> (Vec<Piece>, Holding) {
    let mut pieces = Vec::new();
    let mut identities: Vec<CountryIdentityRun> = Vec::new();
    let mut unknown: Vec<(i32, i32)> =
        query.holders.unknown.iter().map(|&(first, last, _)| (first.max(from), last.min(through))).filter(|(first, last)| first <= last).collect();
    for run in query.holders.within(country, from, through) {
        let Holder::Found { slot, id } = run.holder else {
            unknown.push((run.first, run.last));
            continue;
        };
        let Some(held_from) = held_from(query, country, slot, id, run.first) else {
            unknown.push((run.first, run.last));
            continue;
        };
        match identities.last_mut() {
            Some(held) if (held.slot, held.country_id) == (slot, id) && held.last_hour.checked_add(1) == Some(run.first) => held.last_hour = run.last,
            _ => identities.push(CountryIdentityRun { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
        }
        for stretch in query.timeline.within(run.first, run.last) {
            let opened = query.replay.recording().run(stretch.run).map_or(stretch.first, |run| run.first_hour);
            pieces.push(Piece { run: stretch.run, first: stretch.first, last: stretch.last, slot, id, held_from, opened });
        }
    }
    unknown.sort_unstable();
    let mut unresolved: Vec<(i32, i32)> = Vec::new();
    for (first, last) in unknown {
        match unresolved.last_mut() {
            Some(held) if held.1.saturating_add(1) >= first => held.1 = held.1.max(last),
            _ => unresolved.push((first, last)),
        }
    }
    (pieces, Holding { identities, unresolved })
}

fn links_of<'l>(query: &Query<'_>, held: &'l mut HashMap<u32, RunLinks>, run: u32) -> io::Result<&'l RunLinks> {
    if let std::collections::hash_map::Entry::Vacant(vacant) = held.entry(run) {
        let links = dashboard_query::cost::storage(|| country_ids::run_links(query.replay.recording(), run))?;
        vacant.insert(links);
    }
    Ok(&held[&run])
}

struct Spelled {
    tokens: HashMap<u32, String>,
    people: HashMap<u32, ProjectPerson>,
}

fn spell(query: &Query<'_>, naming: &mut Naming<'_>, definitions: &BTreeMap<u32, i32>, characters: &BTreeMap<u32, i32>, cancelled: &AtomicBool) -> io::Result<Spelled> {
    let mut by_hour: BTreeMap<i32, (Vec<u32>, Vec<u32>)> = BTreeMap::new();
    for (&id, &hour) in definitions {
        by_hour.entry(hour).or_default().0.push(id);
    }
    for (&id, &hour) in characters {
        by_hour.entry(hour).or_default().1.push(id);
    }
    let mut spelled = Spelled { tokens: HashMap::new(), people: HashMap::new() };
    let mut frames = at_hour::Frames::new(query)?;
    for (hour, (defs, chars)) in by_hour {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let people = [Read::these(TableId::Characters, chars.iter().map(|&id| u128::from(id))), Read::these(TableId::CharacterPortraits, chars.iter().map(|&id| u128::from(id)))];
        let mut frame = frames.frame(hour, &people, "Special project scientist")?;
        let name_tokens: Vec<u32> = frame.complete::<tables::Characters>().map(|rows| rows.iter().filter_map(|row| u32::try_from(row.name_token_id).ok()).collect()).unwrap_or_default();
        let tokens = [Read::these(TableId::LexicalTokens, defs.iter().chain(&name_tokens).map(|&id| u128::from(id)))];
        let lexical = frames.frame(hour, &tokens, "Special project names")?;
        frame.tables[TableId::LexicalTokens.index()] = lexical.tables[TableId::LexicalTokens.index()].clone();
        spelled.tokens.extend(spellings(&frame, defs));
        for id in chars {
            let person = naming.person(&frame, id);
            spelled.people.insert(id, person);
        }
    }
    Ok(spelled)
}

fn unread_person(person: &Person) -> ProjectPerson {
    match person {
        Person::Unassigned => ProjectPerson { assigned: Some(false), id: None, name: None, portrait: None, missing: Some("No scientist assigned.".into()) },
        Person::Unknown(reason) => ProjectPerson { assigned: None, id: None, name: None, portrait: None, missing: Some(reason.clone()) },
        Person::Character(id) => ProjectPerson { assigned: Some(true), id: Some(*id), name: None, portrait: None, missing: Some("The scientist was not read.".into()) },
    }
}

struct Interned<T> {
    values: Vec<T>,
    ids: HashMap<T, u32>,
}

impl<T: Clone + Eq + std::hash::Hash> Interned<T> {
    fn new() -> Self {
        Self { values: Vec::new(), ids: HashMap::new() }
    }

    fn id(&mut self, value: T) -> u32 {
        if let Some(&id) = self.ids.get(&value) {
            return id;
        }
        let id = self.values.len() as u32;
        self.ids.insert(value.clone(), id);
        self.values.push(value);
        id
    }
}

fn reply(source: &Source, params: &SpecialProjectsParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<SpecialProjectsReply> {
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let content = content(source, load)?;
    let through = params.hour.unwrap_or(query.timeline.last);
    if params.hour.is_some() && query.timeline.at(through).is_none() { return Err(dashboard_core::refused("Special projects hour is not recorded")); }
    reply_through(query, params, &content, through, cancelled)
}

fn reply_through(query: Query<'_>, params: &SpecialProjectsParams, content: &ProjectContent, through: i32, cancelled: &AtomicBool) -> io::Result<SpecialProjectsReply> {
    let mut follow = incremental::Follow::new(query.timeline.first);
    follow.to(&query, &params.country, through, cancelled)?;
    let tag = query.holders.name(&params.country, through);
    present(&query, params, content, &follow, &tag, cancelled, &mut None)
}

type Spelling = (BTreeMap<u32, i32>, BTreeMap<u32, i32>, Spelled, Refusals, BTreeMap<String, u64>);

enum Respell {
    Kept,
    Added(BTreeMap<u32, i32>, BTreeMap<u32, i32>),
    All,
}

fn respell(held: Option<(&BTreeMap<u32, i32>, &BTreeMap<u32, i32>)>, definitions: &BTreeMap<u32, i32>, characters: &BTreeMap<u32, i32>) -> Respell {
    let Some((held_definitions, held_characters)) = held else { return Respell::All };
    let kept = |held: &BTreeMap<u32, i32>, now: &BTreeMap<u32, i32>| held.iter().all(|(id, hour)| now.get(id) == Some(hour));
    if !kept(held_definitions, definitions) || !kept(held_characters, characters) { return Respell::All; }
    let added = |held: &BTreeMap<u32, i32>, now: &BTreeMap<u32, i32>| now.iter().filter(|(id, _)| !held.contains_key(id)).map(|(&id, &hour)| (id, hour)).collect::<BTreeMap<_, _>>();
    let (new_definitions, new_characters) = (added(held_definitions, definitions), added(held_characters, characters));
    if new_definitions.is_empty() && new_characters.is_empty() { Respell::Kept } else { Respell::Added(new_definitions, new_characters) }
}

fn spans(spans: &[(i32, i32)]) -> Vec<HourSpan> {
    spans.iter().map(|&(first, last)| HourSpan { first_hour: first, last_hour: last }).collect()
}

pub(crate) fn shown_runs(runs: &[facts::Run], hour: i32) -> Option<std::ops::RangeInclusive<usize>> {
    let at = runs.partition_point(|run| run.last < hour);
    let run = runs.get(at).filter(|run| run.first <= hour)?;
    (run.state != ProjectState::Cancelled).then(|| at.saturating_sub(1)..=at)
}

fn present(query: &Query<'_>, params: &SpecialProjectsParams, content: &ProjectContent, follow: &incremental::Follow, tag: &str, cancelled: &AtomicBool, cache: &mut Option<Spelling>) -> io::Result<SpecialProjectsReply> {
    let facts = &follow.facts;
    let mut naming = Naming::new(content);
    let mut definitions: BTreeMap<u32, i32> = BTreeMap::new();
    let mut characters: BTreeMap<u32, i32> = BTreeMap::new();
    for run in facts.projects.iter().flat_map(|project| &project.runs) {
        let look = &facts.looks[run.look as usize];
        let first = definitions.entry(look.definition).or_insert(run.first);
        *first = (*first).min(run.first);
        for id in [match look.person { Person::Character(id) => Some(id), _ => None }, look.last].into_iter().flatten() {
            let first = characters.entry(id).or_insert(run.first);
            *first = (*first).min(run.first);
        }
    }
    for kept in facts.evidence.values().flat_map(|evidence| &evidence.entries) {
        if let Some(definition) = kept.definition {
            let first = definitions.entry(definition).or_insert(kept.read);
            *first = (*first).min(kept.read);
        }
    }
    match respell(cache.as_ref().map(|(defs, chars, ..)| (defs, chars)), &definitions, &characters) {
        Respell::Kept => {}
        Respell::All => {
            let spelled = spell(query, &mut naming, &definitions, &characters, cancelled)?;
            *cache = Some((definitions, characters, spelled, naming.refusals.get_mut().unwrap().clone(), naming.art.clone()));
        }
        Respell::Added(new_definitions, new_characters) => {
            let (_, _, mut spelled, refused, art) = cache.take().expect("a spelling to add to");
            *naming.refusals.get_mut().unwrap() = refused;
            naming.art = art;
            let more = spell(query, &mut naming, &new_definitions, &new_characters, cancelled)?;
            spelled.tokens.extend(more.tokens);
            spelled.people.extend(more.people);
            *cache = Some((definitions, characters, spelled, naming.refusals.get_mut().unwrap().clone(), naming.art.clone()));
        }
    }
    let (_, _, spelled, refused, art) = cache.as_ref().expect("spelled");
    *naming.refusals.get_mut().unwrap() = refused.clone();
    naming.art.clone_from(art);
    let mut people = Interned::new();
    let mut presentations = Interned::new();
    let mut names = HashMap::new();
    let mut shown = |look: &facts::Look, hour: i32, naming: &mut Naming<'_>| {
        let name = names.entry((look.definition, hour)).or_insert_with(|| {
            let slot = query.holders.at(&params.country, hour).and_then(|run| match run.holder {
                Holder::Found { slot, .. } => Some(slot),
                Holder::Unresolved => None,
            });
            naming.definition(spelled.tokens.get(&look.definition).map(String::as_str), look.definition, tag, &mut |key| match slot {
                Some(slot) => scoped_at(query, content, key, hour, slot),
                None => Err(query.holders.not_known(&params.country, hour).unwrap_or_else(|| "The project's name needs its country's scope, which is unresolved".to_owned())),
            })
        });
        let person = |person: &Person| match person {
            Person::Character(id) => spelled.people.get(id).cloned().unwrap_or_else(|| unread_person(person)),
            other => unread_person(other),
        };
        let scientist = people.id(person(&look.person));
        let last_scientist = look.last.map(|id| people.id(person(&Person::Character(id))));
        presentations.id(ProjectPresentation {
            name: name.name.clone(),
            specialization: name.specialization.clone(),
            specialization_name: name.specialization_name.clone(),
            icon: name.icon.clone(),
            icon_missing: name.icon_missing.clone(),
            scientist,
            last_scientist,
        })
    };
    let none = facts::Evidence::default();
    let mut record = |country: (u32, u32), owner: u32, local: u32, first: (i32, ProjectState), runs: &[facts::Run], evidence: &facts::Evidence, naming: &mut Naming<'_>| {
        let statuses: Vec<ProjectStatus> = runs
            .iter()
            .map(|run| ProjectStatus {
                first_hour: run.first,
                last_hour: run.last,
                read_hour: run.read,
                presentation: shown(&facts.looks[run.look as usize], run.first, naming),
                progress: run.progress,
                state: run.state,
                iterations: run.iterations,
                scientist_read: run.scientist_read,
                last_scientist_read: run.last_scientist_read,
            })
            .collect();
        let unnamed = |definition| facts::Look { definition, person: Person::Unknown("A history entry names no scientist.".into()), last: None };
        let entries = evidence
            .entries
            .iter()
            .map(|kept| ProjectEntry {
                ordinal: kept.entry.ordinal,
                date: kept.entry.date,
                completion: kept.entry.completion(),
                read_hour: kept.read,
                counted: kept.counted,
                presentation: kept.definition.map(|definition| shown(&unnamed(definition), kept.read, naming)),
            })
            .collect();
        let attempts = evidence.attempts.iter().map(|attempt| ProjectAttempt { ordinal: attempt.entry.ordinal, date: attempt.entry.date, read_hour: attempt.read, since_read: attempt.since }).collect();
        ProjectRecord { id: format!("{}:{}:{owner}:{local}", country.0, country.1), owner, local, first_read: first.0, first_state: first.1, statuses, entries, attempts }
    };
    let mut projects = Vec::with_capacity(facts.projects.len());
    if let (Some(true), Some(hour)) = (params.current, follow.through()) {
        for project in &facts.projects {
            let Some(shown) = shown_runs(&project.runs, hour) else { continue };
            let mut markers = facts::Evidence::default();
            if project.runs[*shown.end()].state == ProjectState::Completed {
                markers.entries = facts.evidence_of(project).unwrap_or(&none).entries.iter().filter(|kept| kept.entry.completion() && kept.counted).cloned().collect();
            }
            projects.push(record(project.country, project.owner, project.local, project.first, &project.runs[shown], &markers, &mut naming));
        }
    } else {
        for project in &facts.projects {
            let evidence = facts.evidence_of(project).unwrap_or(&none);
            projects.push(record(project.country, project.owner, project.local, project.first, &project.runs, evidence, &mut naming));
        }
    }
    for (&(slot, id, owner, local), evidence) in facts.unlisted().filter(|_| params.current != Some(true)) {
        let first = evidence.entries.iter().map(|kept| kept.read).min().unwrap_or(i32::MAX);
        projects.push(record((slot, id), owner, local, (first, ProjectState::Unknown), &[], evidence, &mut naming));
    }
    let counts: BTreeSet<i32> = projects.iter().flat_map(|project| &project.statuses).filter_map(|status| status.iterations).collect();
    let iteration_texts: BTreeMap<i32, Option<String>> = counts.into_iter().map(|count| (count, naming.iterations(count))).collect();
    let mut warnings = content.definitions.warnings.clone();
    warnings.extend(facts.warnings.iter().map(|(reason, count)| format!("{reason} ({count} reads).")));
    warnings.extend(naming.warnings());
    if !projects.is_empty() {
        warnings.push(format!(
            "{}; project icons and portraits are visual assets outside content hash coverage.",
            dashboard_base::products::artwork::CURRENT_INSTALL_ART_LABEL
        ));
    }
    let Holding { identities, unresolved } = follow.holding.clone();
    let unavailable = if identities.is_empty() && !unresolved.is_empty() {
        Some("Who held this tag was not recorded.".into())
    } else if identities.is_empty() {
        Some("No country held this tag in the recording.".into())
    } else if content.definitions.projects.is_empty() {
        Some("The recorded content has no readable special-project definitions.".into())
    } else {
        None
    };
    let reply = SpecialProjectsReply {
        branch_id: params.branch_id.clone(),
        tag: tag.to_owned(),
        identities,
        unresolved: spans(&unresolved),
        available: spans(&facts.observed),
        partial: spans(&facts.partial),
        projects,
        reads: facts.reads.iter().filter(|_| params.current != Some(true)).flat_map(|(&owner, spans)| spans.iter().map(move |&(first_hour, last_hour, read_hour)| OwnerRead { owner, first_hour, last_hour, read_hour })).collect(),
        presentations: presentations.values,
        people: people.values,
        iteration_texts,
        unavailable,
        shown: ProjectsShown::default(),
        warnings,
    };
    let reply = match follow.through() {
        Some(hour) => SpecialProjectsReply { shown: shown_at(&reply.projects, &reply.presentations, &reply.available, hour), ..reply },
        None => reply,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn dashboard_follow::panel::Panel>, String> {
    let mut params = asked.clone();
    params.as_object_mut().ok_or("Invalid special-projects panel")?.remove("panel");
    let params: SpecialProjectsParams = dashboard_query::named::named(&params).map_err(|e| e.to_string())?;
    SpecialProjects::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|e| e.to_string())?;
    let content = content(source, load).map_err(|e| e.to_string())?;
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    let params = Arc::new(params);
    let advance = at_hour::shared_reader(source, load, SpecialProjects::KEY, asked, || incremental::reader(replay.clone(), timeline.clone(), holders.clone(), params.clone(), content.clone(), executable));
    let read: dashboard_follow::panel::ExactReader = Arc::new(move |hour, cancelled| {
        if timeline.at(hour).is_none() { return Err("Special projects hour is not recorded".into()); }
        let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
        let reply = engine_executable::executable_tables::scope(executable, || reply_through(query, &params, &content, hour, cancelled)).map_err(|e| e.to_string())?;
        serde_json::to_value(reply).map_err(|e| e.to_string())
    });
    Ok(at_hour::product_panel(advance, read))
}

impl Product for SpecialProjects {
    const KEY: &'static str = "special-projects";
    const CONTENT_DEPENDENT: bool = true;
    type Params = SpecialProjectsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![hour: Read, branchId, country, current];
    type Reply = SpecialProjectsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            SpecialProjectsParams::decl(),
            ProjectPerson::decl(),
            ProjectPresentation::decl(),
            ProjectState::decl(),
            ProjectStatus::decl(),
            OwnerRead::decl(),
            ProjectEntry::decl(),
            ProjectAttempt::decl(),
            ProjectRecord::decl(),
            ShownCompleted::decl(),
            ProjectsShown::decl(),
            SpecialProjectsReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        reply(source, params, load, cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectCompletion {
    pub tag: Option<String>,
    pub date: i32,
    pub observed_hour: i32,
    pub owner: u32,
    pub project: u32,
    pub ordinal: u32,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

pub struct Completions {
    pub events: Vec<ProjectCompletion>,
    pub unrecorded: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Counted {
    tag: String,
    slot: u32,
    owner: u32,
    local: u32,
    entry: Entry,
    read: i32,
    definition: Option<u32>,
}

fn slice_read_value(reader: &TableReader, col: usize, table: TableId, owner: u32) -> Option<i32> {
    if !reader.available() {
        return None;
    }
    let view = reader.view();
    let slot = view.slot(read_key(table, owner))?;
    if reader.read_rows().is_some_and(|read| read.binary_search(&slot).is_err()) {
        return None;
    }
    let &kind = reader.kinds().get(col)?;
    cell_number(kind, view.value(col, slot)).and_then(|v| i32::try_from(v).ok())
}

struct PendingHit {
    piece_idx: usize,
    raw: u32,
    hour: i32,
    h_stamp: i32,
    p_stamp: Option<i32>,
    listed: BTreeMap<(u32, u32), Vec<Entry>>,
}

struct CompletionWatch {
    win_start: i32,
    win_end: i32,
    piece_idx: usize,
    raw: u32,
    project: u32,
    entry: Entry,
}

fn see_run_evidence(
    tag_facts: &mut BTreeMap<String, Facts>,
    seen: &mut std::collections::HashSet<(usize, u32, u32, u32, u32, Entry)>,
    query: &Query<'_>,
    campaign_start: i32,
    recording: &Recording,
    run: u32,
    held_run: &RecordingRun,
    prepared: &[(usize, String, Piece, Custody, Vec<u32>)],
    history_by_owner: &HashMap<u32, Vec<(u32, u32, Vec<Step>)>>,
    from: i32,
    to: i32,
    run_reach: i32,
    run_last: i32,
    sample: &[usize; 1],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut watches: Vec<CompletionWatch> = Vec::new();
    for (piece_idx, (tag_id, _, piece, _, owners)) in prepared.iter().enumerate() {
        let reach = piece.reach();
        for &raw in owners {
            let Some(held) = history_by_owner.get(&raw) else { continue };
            for (project, ordinal, steps) in held {
                for (idx, step) in steps.iter().enumerate() {
                    let Some(entry) = step.cells.as_deref().and_then(|cells| history_entry(*ordinal, cells)) else { continue };
                    if !entry.completion() || !(from..=to).contains(&entry.date) {
                        continue;
                    }
                    if seen.contains(&(*tag_id, piece.slot, piece.id, raw, *project, entry)) {
                        continue;
                    }
                    let step_end = steps.get(idx + 1).map_or(piece.last, |next| (next.hour - 1).min(piece.last));
                    if step_end < reach || step.hour > piece.last {
                        continue;
                    }
                    let win_start = step.hour.max(piece.first);
                    let win_end = step_end.saturating_add(STAGGER_PERIOD - 1).min(piece.last);
                    if win_start <= win_end {
                        watches.push(CompletionWatch { win_start, win_end, piece_idx, raw, project: *project, entry });
                    }
                }
            }
        }
    }
    if watches.is_empty() {
        return Ok(());
    }
    let projects_complete = available(query, run, TableId::Projects, run_reach, run_last)?;
    let history_complete = available(query, run, TableId::ProjectHistory, run_reach, run_last)?;
    watches.sort_unstable_by_key(|w| (w.win_start, w.piece_idx, w.raw, w.project));
    let mut slice_reader = dashboard_query::cost::storage(|| TableReader::new(recording, run, TableId::SliceReads, Some(sample)))?;
    slice_reader.index_keys();
    let sample_col = sample[0];
    let mut hits: Vec<PendingHit> = Vec::new();
    let mut project_asks: Vec<(i32, u32, u32)> = Vec::new();
    let mut last_h_stamp: HashMap<(usize, u32), i32> = HashMap::new();
    let mut history_cursors: HashMap<(usize, u32), Vec<(u32, u32, Cursor<'_>)>> = HashMap::new();
    let mut active: Vec<usize> = Vec::new();
    let mut active_pairs: Vec<(usize, u32)> = Vec::new();
    let mut next_watch = 0usize;
    let mut hour = watches[0].win_start;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        active.retain(|&w| {
            let watch = &watches[w];
            if hour > watch.win_end {
                return false;
            }
            let (tag_id, _, piece, ..) = &prepared[watch.piece_idx];
            !seen.contains(&(*tag_id, piece.slot, piece.id, watch.raw, watch.project, watch.entry))
        });
        if active.is_empty() {
            while next_watch < watches.len() {
                let watch = &watches[next_watch];
                let (tag_id, _, piece, ..) = &prepared[watch.piece_idx];
                if !seen.contains(&(*tag_id, piece.slot, piece.id, watch.raw, watch.project, watch.entry)) {
                    break;
                }
                next_watch += 1;
            }
            if next_watch >= watches.len() {
                break;
            }
            hour = hour.max(watches[next_watch].win_start);
        }
        while next_watch < watches.len() && watches[next_watch].win_start <= hour {
            let watch = &watches[next_watch];
            let (tag_id, _, piece, ..) = &prepared[watch.piece_idx];
            if hour <= watch.win_end && !seen.contains(&(*tag_id, piece.slot, piece.id, watch.raw, watch.project, watch.entry)) {
                active.push(next_watch);
            }
            next_watch += 1;
        }
        if active.is_empty() {
            continue;
        }
        if let Some(ordinal) = held_run.ordinal(hour) {
            dashboard_query::cost::storage(|| slice_reader.go_to(recording, ordinal))?;
            active_pairs.clear();
            for &w in &active {
                active_pairs.push((watches[w].piece_idx, watches[w].raw));
            }
            active_pairs.sort_unstable();
            active_pairs.dedup();
            for &(piece_idx, raw) in &active_pairs {
                let (tag_id, _, piece, custody, _) = &prepared[piece_idx];
                let h_raw = slice_read_value(&slice_reader, sample_col, TableId::ProjectHistory, raw);
                let Some(hs) = custody.usable(raw, h_raw, &history_complete, hour) else { continue };
                if last_h_stamp.get(&(piece_idx, raw)) == Some(&hs) {
                    continue;
                }
                last_h_stamp.insert((piece_idx, raw), hs);
                let p_raw = slice_read_value(&slice_reader, sample_col, TableId::Projects, raw);
                let ps = custody.usable(raw, p_raw, &projects_complete, hour);
                let Some(held) = history_by_owner.get(&raw) else { continue };
                let cursors = history_cursors
                    .entry((piece_idx, raw))
                    .or_insert_with(|| held.iter().map(|(project, ordinal, steps)| (*project, *ordinal, Cursor::new(steps))).collect());
                let mut listed: BTreeMap<(u32, u32), Vec<Entry>> = BTreeMap::new();
                for (project, ordinal, cursor) in cursors.iter_mut() {
                    if let Some(cells) = cursor.at(hs)
                        && let Some(entry) = history_entry(*ordinal, cells)
                    {
                        listed.entry((raw, *project)).or_default().push(entry);
                    }
                }
                let mut introduces = false;
                for (&(_, project), entries) in &listed {
                    for (at, &entry) in entries.iter().enumerate() {
                        if seen.insert((*tag_id, piece.slot, piece.id, raw, project, entry)) {
                            introduces = true;
                            if entry.completion()
                                && at + 1 == entries.len()
                                && campaign_start <= entry.date
                                && entry.date <= hs
                                && custody.held(raw, entry.date, hs)
                                && let Some(p_stamp) = ps
                                && p_stamp <= hs
                                && p_stamp >= entry.date
                            {
                                project_asks.push((p_stamp, raw, project));
                            }
                        }
                    }
                }
                if introduces {
                    hits.push(PendingHit { piece_idx, raw, hour, h_stamp: hs, p_stamp: ps, listed });
                }
            }
        }
        if hour == i32::MAX {
            break;
        }
        hour += 1;
    }
    let mut project_rows: HashMap<(i32, u32, u32), Row> = HashMap::new();
    if !project_asks.is_empty() {
        project_asks.sort_unstable();
        project_asks.dedup();
        let mut project_reader = dashboard_query::cost::storage(|| TableReader::new(recording, run, TableId::Projects, Some(&COMPLETION_PROJECT_COLUMNS)))?;
        project_reader.index_keys();
        project_rows.reserve(project_asks.len());
        for (p_stamp, raw, project) in project_asks {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if let Some(row) = project_row_at(recording, held_run, &mut project_reader, raw, project, p_stamp)? {
                project_rows.insert((p_stamp, raw, project), row);
            }
        }
    }
    hits.sort_unstable_by_key(|hit| (hit.piece_idx, hit.raw, hit.hour));
    let mut projects_read: BTreeMap<u32, i32> = BTreeMap::new();
    let mut histories_read: BTreeMap<u32, i32> = BTreeMap::new();
    let mut rows: Vec<(Row, i32)> = Vec::new();
    for hit in hits {
        let (_, tag, piece, custody, _) = &prepared[hit.piece_idx];
        histories_read.clear();
        histories_read.insert(hit.raw, hit.h_stamp);
        projects_read.clear();
        rows.clear();
        if let Some(ps) = hit.p_stamp {
            projects_read.insert(hit.raw, ps);
            for &(_, project) in hit.listed.keys() {
                if let Some(row) = project_rows.get(&(ps, hit.raw, project)) {
                    rows.push((row.clone(), ps));
                }
            }
        }
        let shown: Vec<facts::Read<'_>> = rows.iter().map(|(row, stamp)| facts::Read { row, read: *stamp, program: Program::Unassigned }).collect();
        let facts = tag_facts.entry(tag.clone()).or_insert_with(|| Facts::new(campaign_start));
        facts.histories_only(&facts::Hour {
            country: (piece.slot, piece.id),
            at: hit.hour,
            held: &|owner, from, through| custody.held(owner, from, through),
            projects: &projects_read,
            histories: &histories_read,
            listed: &hit.listed,
            reads: &shown,
            published: false,
        });
    }
    Ok(())
}

fn slot_tag_index<'a>(query: &'a Query<'_>) -> HashMap<u32, Vec<(&'a str, Vec<(i32, i32)>)>> {
    let mut grouped: HashMap<u32, HashMap<&str, Vec<(i32, i32)>>> = HashMap::new();
    for (tag, runs) in &query.holders.tags {
        for run in runs {
            if let Holder::Found { slot, .. } = run.holder {
                grouped.entry(slot).or_default().entry(tag.as_str()).or_default().push((run.first, run.last));
            }
        }
    }
    grouped
        .into_iter()
        .map(|(slot, by_tag)| {
            let list = by_tag
                .into_iter()
                .map(|(tag, mut spans)| {
                    spans.sort_unstable();
                    (tag, spans)
                })
                .collect();
            (slot, list)
        })
        .collect()
}

fn span_contains(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|&(_, last)| last < hour);
    spans.get(at).is_some_and(|&(first, _)| first <= hour)
}

#[cfg(test)]
fn counted(query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<Counted>, Vec<HourSpan>)> {
    counted_with(None, query, from, to, cancelled)
}

fn counted_with(source: Option<&Source>, query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<Counted>, Vec<HourSpan>)> {
    let recording = query.replay.recording();
    let mut unrecorded: Vec<HourSpan> = Vec::new();
    let mut cached_links: HashMap<u32, Arc<RunLinks>> = HashMap::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let mut scanned_histories: HashMap<u32, (i32, i32, History)> = HashMap::new();
    let by_slot = slot_tag_index(query);
    let get_links = |cached: &mut HashMap<u32, Arc<RunLinks>>, run: u32| -> io::Result<Arc<RunLinks>> {
        if let Some(held) = cached.get(&run) {
            return Ok(Arc::clone(held));
        }
        let loaded = match source {
            Some(source) => dashboard_base::queries::links(source, run)?,
            None => Arc::new(dashboard_query::cost::storage(|| country_ids::run_links(recording, run))?),
        };
        cached.insert(run, Arc::clone(&loaded));
        Ok(loaded)
    };
    for stretch in query.timeline.within(from, to) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held = available(query, stretch.run, TableId::ProjectHistory, stretch.first, stretch.last)?;
        unrecorded.extend(uncovered(&held, stretch.first, stretch.last).into_iter().map(|(first, last)| HourSpan { first_hour: first, last_hour: last }));
        let history = dashboard_query::cost::storage(|| lanes::history(recording, stretch.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::All, stretch.first, stretch.last))?;
        let mut run_links: Option<Arc<RunLinks>> = None;
        for (key, steps) in &history.keys {
            for step in steps {
                let Some(entry) = step.cells.as_deref().and_then(|cells| history_entry(*key as u32, cells)) else { continue };
                if !entry.completion() || !(from..=to).contains(&entry.date) {
                    continue;
                }
                let links = match &run_links {
                    Some(links) => links,
                    None => run_links.insert(get_links(&mut cached_links, stretch.run)?),
                };
                let Some(slot) = links.slot_at((key >> 64) as u32, step.hour) else { continue };
                if let Some(holders) = by_slot.get(&slot) {
                    for (tag, spans) in holders {
                        if !tags.contains(*tag) && span_contains(spans, step.hour) {
                            tags.insert((*tag).to_owned());
                        }
                    }
                }
            }
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = scanned_histories.entry(stretch.run) {
            slot.insert((stretch.first, stretch.last, history));
        }
    }
    if tags.is_empty() {
        return Ok((Vec::new(), unrecorded));
    }
    let mut stretch_runs: Vec<u32> = Vec::new();
    let mut run_order: HashMap<u32, usize> = HashMap::new();
    for stretch in query.timeline.within(query.timeline.first, to) {
        run_order.entry(stretch.run).or_insert_with(|| {
            let at = stretch_runs.len();
            stretch_runs.push(stretch.run);
            at
        });
    }
    let mut by_run: Vec<Vec<(usize, String, Piece)>> = (0..stretch_runs.len()).map(|_| Vec::new()).collect();
    for (tag_id, tag) in tags.iter().enumerate() {
        let (pieces, _) = pieces(query, &Country::Tag(tag.clone()), query.timeline.first, to);
        for piece in pieces {
            if let Some(&order) = run_order.get(&piece.run) {
                by_run[order].push((tag_id, tag.clone(), piece));
            }
        }
    }
    let mut tag_facts: BTreeMap<String, Facts> = BTreeMap::new();
    let mut seen: std::collections::HashSet<(usize, u32, u32, u32, u32, Entry)> = std::collections::HashSet::new();
    let sample = read_columns()?;
    for (order, batch) in by_run.into_iter().enumerate() {
        if batch.is_empty() {
            continue;
        }
        dashboard_text::metadata::check_cancel(cancelled)?;
        let run = stretch_runs[order];
        let run_links = get_links(&mut cached_links, run)?;
        let mut run_reach = i32::MAX;
        let mut run_last = i32::MIN;
        let mut all_owners: BTreeSet<u32> = BTreeSet::new();
        let mut prepared: Vec<(usize, String, Piece, Custody, Vec<u32>)> = Vec::with_capacity(batch.len());
        for (tag_id, tag, piece) in batch {
            run_reach = run_reach.min(piece.reach());
            run_last = run_last.max(piece.last);
            let custody = Custody::new(&run_links, piece.slot, piece.held_from, piece.last);
            let owners: Vec<u32> = custody.owners.iter().filter(|(_, spans)| spans.iter().any(|&(_, last)| last >= piece.reach())).map(|(&raw, _)| raw).collect();
            all_owners.extend(&owners);
            prepared.push((tag_id, tag, piece, custody, owners));
        }
        let history = match scanned_histories.remove(&run) {
            Some((first, last, history)) if first <= run_reach && run_last <= last => history,
            _ => dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::All, run_reach, run_last))?,
        };
        let mut completion_projects: BTreeSet<(u32, u32)> = BTreeSet::new();
        for (key, steps) in &history.keys {
            let owner = (key >> 64) as u32;
            if !all_owners.contains(&owner) {
                continue;
            }
            let project = (key >> 32) as u32;
            if completion_projects.contains(&(owner, project)) {
                continue;
            }
            if steps.iter().any(|step| step.cells.as_deref().and_then(|cells| history_entry(*key as u32, cells)).is_some_and(|entry| entry.completion() && (from..=to).contains(&entry.date))) {
                completion_projects.insert((owner, project));
            }
        }
        if completion_projects.is_empty() {
            continue;
        }
        let mut history_by_owner: HashMap<u32, Vec<(u32, u32, Vec<Step>)>> = HashMap::new();
        for (key, steps) in history.keys {
            let owner = (key >> 64) as u32;
            let project = (key >> 32) as u32;
            if completion_projects.contains(&(owner, project)) {
                history_by_owner.entry(owner).or_default().push((project, key as u32, steps));
            }
        }
        let held_run = recording.run(run)?;
        see_run_evidence(&mut tag_facts, &mut seen, query, query.timeline.first, recording, run, held_run, &prepared, &history_by_owner, from, to, run_reach, run_last, &sample, cancelled)?;
    }
    let mut found: BTreeMap<(u32, u32, u32, i32), Counted> = BTreeMap::new();
    for (tag, facts) in tag_facts {
        for (&(slot, _, owner, local), evidence) in &facts.evidence {
            for kept in evidence.entries.iter().filter(|kept| kept.counted && (from..=to).contains(&kept.entry.date)) {
                found
                    .entry((owner, local, kept.entry.ordinal, kept.entry.date))
                    .or_insert_with(|| Counted { tag: tag.clone(), slot, owner, local, entry: kept.entry, read: kept.read, definition: kept.definition });
            }
        }
    }
    Ok((found.into_values().collect(), unrecorded))
}

fn open_lexical(recording: &Recording, run: u32, literal_col: usize) -> io::Result<Option<TableReader>> {
    match dashboard_query::cost::storage(|| TableReader::new(recording, run, TableId::LexicalTokens, Some(&[literal_col]))) {
        Ok(mut reader) => {
            reader.index_keys();
            Ok(Some(reader))
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(error),
    }
}

fn spell_completions(query: &Query<'_>, found: &[Counted], cancelled: &AtomicBool) -> io::Result<HashMap<(i32, u32), String>> {
    let mut by_run: BTreeMap<u32, BTreeMap<i32, BTreeSet<u32>>> = BTreeMap::new();
    for counted in found {
        if let Some(definition) = counted.definition
            && let Some(stretch) = query.timeline.within(counted.read, counted.read).next()
        {
            by_run.entry(stretch.run).or_default().entry(counted.read).or_default().insert(definition);
        }
    }
    let mut spelled: HashMap<(i32, u32), String> = HashMap::new();
    if by_run.is_empty() {
        return Ok(spelled);
    }
    let recording = query.replay.recording();
    let literal_col = dashboard_declare::field(TableId::LexicalTokens, "literal")?;
    for (run, reads) in by_run {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held_run = recording.run(run)?;
        let mut reader = open_lexical(recording, run, literal_col)?;
        for (read, definitions) in reads {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let Some(active) = reader.as_mut() else { break };
            let Some(ordinal) = held_run.ordinal(read) else { continue };
            match dashboard_query::cost::storage(|| active.go_to(recording, ordinal)) {
                Ok(()) => {
                    if active.status() != Status::Complete {
                        continue;
                    }
                    let view = active.view();
                    for id in definitions {
                        let Some(slot) = view.slot(u128::from(id)) else { continue };
                        let Value::Text(text_id) = view.value(literal_col, slot) else { continue };
                        let Some(bytes) = active.strings().get(text_id) else { continue };
                        if let Ok(text) = std::str::from_utf8(content_reader_parser::parser::c_string(bytes)) {
                            spelled.insert((read, id), text.to_owned());
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    reader = open_lexical(recording, run, literal_col)?;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(spelled)
}

pub fn completions(source: &Source, load: &LoadContext, query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<Completions> {
    let content = content(source, load)?;
    let (mut found, unrecorded) = counted_with(Some(source), query, from, to, cancelled)?;
    found.sort_by_key(|counted| (counted.read, counted.owner, counted.local, counted.entry.ordinal, counted.entry.date));
    let spelled = spell_completions(query, &found, cancelled)?;
    let mut naming = Naming::new(&content);
    let mut scoped_frames = None;
    let mut events = Vec::with_capacity(found.len());
    for counted in found {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Counted { tag, slot, owner, local, entry, read, definition } = counted;
        let mut completion =
            ProjectCompletion { tag: Some(tag.clone()), date: entry.date, observed_hour: read, owner, project: local, ordinal: entry.ordinal, name: None, icon: None, icon_missing: Some("The completed project's name is not recorded.".into()) };
        match definition {
            Some(definition) => {
                let token = spelled.get(&(read, definition)).map(String::as_str);
                let named = naming.definition(token, definition, &tag, &mut |key| scoped_at_in(query, &mut scoped_frames, &content, key, read, slot));
                completion.name = Some(named.name);
                completion.icon = named.icon;
                completion.icon_missing = named.icon_missing;
            }
            None => naming.note("the completed project's name is not recorded", format!("{owner}:{local}")),
        }
        events.push(completion);
    }
    events.sort_by(|a, b| (a.date, a.owner, a.project, a.ordinal).cmp(&(b.date, b.owner, b.project, b.ordinal)));
    Ok(Completions { events, unrecorded, warnings: naming.warnings() })
}
