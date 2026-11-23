
use dashboard_base::products::{Country, validate_country};
use dashboard_base::{products::{LoadContext, Product, at_hour::{self, Read}, character_portraits::{portrait_png, recorded_portrait_in}, politics_content::{PoliticsContent, politics_content}}, queries::country_ids::{self, RunLinks}};
use dashboard_world::products::identities::CountryIdentityRun;
use dashboard_base::{Source, catalogue::{Refusals, View}, span::HourSpan};
use dashboard_query::{Query, identity::Holder};
use content_reader_core::special_projects::{Catalog, Project};
use content_reader_localisation::localisation::Argument;
use replay_lanes::lanes::{self, Cell, History, Keys, Step};
use replay_columnar::manifest::Status;
use replay_data::rows::{TableId, tables};
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
mod fold;
mod incremental;
#[cfg(test)]
mod tests;

use fold::{Entry, Fold, Observed, Person, Row};

const ITERATIONS_KEY: &str = "ONGOING_PROJECT_BREAKTHROUGHS_NUMBER";
const PLACEHOLDER_ICON: &str = "GFX_PLACEHOLDER_sp_project_icon";
const ICON_EDGE: u32 = 96;
const PROGRAM_REFERENCE: u32 = 83;
const CHARACTER_REFERENCE: u32 = 73;

const PROJECT_COLUMNS: [usize; 6] = [0, 1, 2, 4, 9, 12];
const HISTORY_COLUMNS: [usize; 3] = [0, 1, 2];
const SCIENTIST_COLUMNS: [usize; 1] = [3];

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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectDate {
    pub hour: i32,
    #[doc = " False: it happened by this hour, not at it."]
    pub exact: bool,
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
    #[doc = " Its index in `people`."]
    pub scientist: u32,
    #[doc = " A completed project loses its program: the last hour its scientist was seen assigned."]
    pub scientist_as_of: Option<i32>,
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
pub struct SpecialProjectRun {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Its index in `presentations`."]
    pub presentation: u32,
    #[doc = " Prototype progress, percent, clamped to -100..=100."]
    pub progress: i32,
    pub state: ProjectState,
    pub completion: Option<ProjectDate>,
    pub cancellation: Option<ProjectDate>,
    #[doc = " History entries less the completion's; none where the history was not recorded."]
    pub iterations: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectEpisode {
    pub id: String,
    pub start: ProjectDate,
    pub runs: Vec<SpecialProjectRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectsReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The tag's holders over the branch."]
    pub identities: Vec<CountryIdentityRun>,
    #[doc = " The hours the holders' projects were recorded."]
    pub available: Vec<HourSpan>,
    pub projects: Vec<ProjectEpisode>,
    pub presentations: Vec<ProjectPresentation>,
    pub people: Vec<ProjectPerson>,
    #[doc = " The game's text for each iteration count shown, none where it cannot be made."]
    pub iteration_texts: BTreeMap<i32, Option<String>>,
    #[doc = " Why nothing is shown, apart from a country with no projects."]
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
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
            let text = std::str::from_utf8(content_reader_core::parser::c_string(&row.literal)).ok()?;
            Some((id, text.to_owned()))
        })
        .collect()
}

fn scoped_at(query: &Query<'_>, content: &ProjectContent, key: &str, hour: i32, slot: u32) -> Result<Option<String>, String> {
    let catalogue = &content.politics.catalogue;
    let scripted = catalogue.scripted_content()?;
    let reads: Vec<Read> = dashboard_text::scripted_text::TABLES.iter().map(|&table| Read::all(table)).collect();
    let frame = at_hour::frame(query, hour, &reads, "Special project name").map_err(|error| format!("The project's name needs its country's scope: {error}"))?;
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
        while self.at + 1 < self.steps.len() && self.steps[self.at + 1].hour <= hour {
            self.at += 1;
        }
        self.steps.get(self.at).filter(|step| step.hour <= hour).and_then(|step| step.cells.as_deref())
    }
}

fn number(cell: Cell) -> Option<i64> {
    match cell {
        Cell::Number(value) => Some(value),
        _ => None,
    }
}

fn project_row(local: u32, cells: &[Cell]) -> Option<Row> {
    let value = |at: usize| cells.get(at).copied().and_then(number);
    Some(Row {
        local,
        definition: value(0)? as u32,
        program: value(1)? as u64,
        started: value(2)? != 0,
        offset: value(3)? as u32,
        not_completed: value(4)? as u8,
        prototype_progress: value(5)? as i32,
    })
}

fn history_entry(ordinal: u32, cells: &[Cell]) -> Option<Entry> {
    let value = |at: usize| cells.get(at).copied().and_then(number);
    let (reward, option, date) = (value(0)? as u32, value(1)? as u32, value(2)? as i32);
    Some(Entry { ordinal, completion: reward == fold::COMPLETION_MARKER && option == fold::COMPLETION_MARKER, date })
}

fn available(query: &Query<'_>, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    let runs = dashboard_query::cost::storage(|| lanes::statuses(query.replay.recording(), run, table, first, last))?;
    Ok(runs.into_iter().filter(|run| matches!(run.status, Status::Complete | Status::Truncated { .. })).map(|run| (run.first, run.last)).collect())
}

fn within(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.1 < hour);
    spans.get(at).is_some_and(|span| span.0 <= hour)
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
    }
    out
}

fn person(program: u64, scientist: Option<Option<&[Cell]>>) -> Person {
    if program == 0 {
        return Person::Unassigned;
    }
    if program as u32 != PROGRAM_REFERENCE {
        return Person::Unknown("The project's program reference has an unknown native type.".into());
    }
    let id = (program >> 32) as u32;
    let Some(Some(cells)) = scientist else { return Person::Unknown(format!("Program {id} was not recorded.")) };
    let Some(scientist) = cells.first().copied().and_then(number).map(|value| value as u64) else {
        return Person::Unknown(format!("Program {id}'s scientist was not recorded."));
    };
    if scientist == 0 {
        Person::Unassigned
    } else if scientist as u32 != CHARACTER_REFERENCE {
        Person::Unknown("The scientist reference has an unknown native type.".into())
    } else {
        Person::Character((scientist >> 32) as u32)
    }
}

struct Piece {
    run: u32,
    first: i32,
    last: i32,
    slot: u32,
    id: u32,
}

struct PieceRows {
    projects: Vec<History>,
    histories: Vec<History>,
    programs: History,
    owners: BTreeMap<u32, Vec<(i32, i32)>>,
    projects_available: Vec<(i32, i32)>,
    history_available: Vec<(i32, i32)>,
}

fn read_piece(query: &Query<'_>, links: &RunLinks, piece: &Piece) -> io::Result<PieceRows> {
    let recording = query.replay.recording();
    let owners = owner_ids(links, piece.slot, piece.first, piece.last);
    let mut projects = Vec::new();
    let mut histories = Vec::new();
    for &raw in owners.keys() {
        let raw = u128::from(raw);
        projects.push(dashboard_query::cost::storage(|| {
            lanes::history(recording, piece.run, TableId::Projects, &PROJECT_COLUMNS, Keys::Range(raw << 32, raw << 32 | u128::from(u32::MAX)), piece.first, piece.last)
        })?);
        histories.push(dashboard_query::cost::storage(|| {
            lanes::history(recording, piece.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::Range(raw << 64, raw << 64 | u128::from(u64::MAX)), piece.first, piece.last)
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
    let programs: Vec<u128> = programs.into_iter().collect();
    let programs = if programs.is_empty() {
        History::default()
    } else {
        dashboard_query::cost::storage(|| lanes::history(recording, piece.run, TableId::ProjectPrograms, &SCIENTIST_COLUMNS, Keys::These(&programs), piece.first, piece.last))?
    };
    at_hour::count_rows(|| projects.iter().chain(&histories).chain([&programs]).flat_map(|h| &h.keys).map(|(_, steps)| steps.len() as u64).sum());
    Ok(PieceRows {
        projects,
        histories,
        programs,
        owners,
        projects_available: available(query, piece.run, TableId::Projects, piece.first, piece.last)?,
        history_available: available(query, piece.run, TableId::ProjectHistory, piece.first, piece.last)?,
    })
}

fn boundaries(first: i32, last: i32, histories: &[&History], spans: &[&[(i32, i32)]]) -> Vec<i32> {
    let mut hours = vec![first];
    for history in histories {
        for (_, steps) in &history.keys {
            hours.extend(steps.iter().map(|step| step.hour));
        }
    }
    for spans in spans {
        for &(a, b) in *spans {
            hours.push(a);
            hours.extend(b.checked_add(1));
        }
    }
    hours.retain(|hour| (first..=last).contains(hour));
    hours.sort_unstable();
    hours.dedup();
    hours
}

fn fold_piece(fold: &mut Fold, piece: &Piece, read: &PieceRows, observed: &mut Vec<(i32, i32)>, cancelled: &AtomicBool) -> io::Result<()> {
    let mut histories: Vec<&History> = read.projects.iter().chain(&read.histories).collect();
    histories.push(&read.programs);
    let mut spans: Vec<&[(i32, i32)]> = read.owners.values().map(Vec::as_slice).collect();
    spans.extend([read.projects_available.as_slice(), read.history_available.as_slice()]);
    let hours = boundaries(piece.first, piece.last, &histories, &spans);
    let mut projects: Vec<(u32, u32, Cursor<'_>)> = Vec::new();
    for (history, &raw) in read.projects.iter().zip(read.owners.keys()) {
        projects.extend(history.keys.iter().map(|(key, steps)| (raw, *key as u32, Cursor::new(steps))));
    }
    let mut entries: Vec<(u32, u32, u32, Cursor<'_>)> = Vec::new();
    for history in &read.histories {
        for (key, steps) in &history.keys {
            entries.push(((key >> 64) as u32, (key >> 32) as u32, *key as u32, Cursor::new(steps)));
        }
    }
    let mut programs: HashMap<u32, Cursor<'_>> = read.programs.keys.iter().map(|(key, steps)| (*key as u32, Cursor::new(steps))).collect();
    for (at, &first) in hours.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let last = hours.get(at + 1).map_or(piece.last, |next| next - 1);
        if !within(&read.projects_available, first) {
            continue;
        }
        let history_known = within(&read.history_available, first);
        let mut history: HashMap<(u32, u32), Vec<Entry>> = HashMap::new();
        for (owner, project, ordinal, cursor) in &mut entries {
            if let Some(entry) = cursor.at(first).and_then(|cells| history_entry(*ordinal, cells))
                && history_known
            {
                history.entry((*owner, *project)).or_default().push(entry);
            }
        }
        let mut rows: Vec<(u32, Row, Option<Option<&[Cell]>>)> = Vec::new();
        for (raw, local, cursor) in &mut projects {
            let (raw, local) = (*raw, *local);
            let linked = read.owners.get(&raw).is_some_and(|spans| within(spans, first));
            if let Some(row) = cursor.at(first).and_then(|cells| project_row(local, cells))
                && linked
            {
                let program = (row.program as u32 == PROGRAM_REFERENCE).then(|| (row.program >> 32) as u32);
                let scientist = program.map(|id| programs.get_mut(&id).and_then(|cursor| cursor.at(first)));
                rows.push((raw, row, scientist));
            }
        }
        let shown: Vec<Observed<'_>> = rows
            .iter()
            .map(|(raw, row, scientist)| Observed {
                row,
                history: history_known.then(|| history.get(&(*raw, row.local)).map_or(&[][..], Vec::as_slice)),
                person: person(row.program, *scientist),
            })
            .collect();
        fold.observe((piece.slot, piece.id), first, last, &shown);
        match observed.last_mut() {
            Some(span) if span.1.checked_add(1) == Some(first) => span.1 = last,
            _ => observed.push((first, last)),
        }
    }
    Ok(())
}

fn pieces(query: &Query<'_>, country: &Country, through: i32) -> (Vec<Piece>, Vec<CountryIdentityRun>) {
    let mut pieces = Vec::new();
    let mut identities: Vec<CountryIdentityRun> = Vec::new();
    for run in query.holders.within(country, query.timeline.first, through) {
        let Holder::Found { slot, id } = run.holder else { continue };
        match identities.last_mut() {
            Some(held) if (held.slot, held.country_id) == (slot, id) && held.last_hour.checked_add(1) == Some(run.first) => held.last_hour = run.last,
            _ => identities.push(CountryIdentityRun { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
        }
        for stretch in query.timeline.within(run.first, run.last) {
            pieces.push(Piece { run: stretch.run, first: stretch.first, last: stretch.last, slot, id });
        }
    }
    (pieces, identities)
}

fn links_of<'l>(query: &Query<'_>, held: &'l mut HashMap<u32, RunLinks>, run: u32) -> io::Result<&'l RunLinks> {
    if !held.contains_key(&run) {
        let links = dashboard_query::cost::storage(|| country_ids::run_links(query.replay.recording(), run))?;
        held.insert(run, links);
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
    let (pieces, identities) = pieces(&query, &params.country, through);
    let mut fold = Fold::new(query.timeline.first);
    let mut observed: Vec<(i32, i32)> = Vec::new();
    let mut links: HashMap<u32, RunLinks> = HashMap::new();
    for piece in &pieces {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let run_links = links_of(&query, &mut links, piece.run)?;
        let read = read_piece(&query, run_links, piece)?;
        fold_piece(&mut fold, piece, &read, &mut observed, cancelled)?;
    }
    let tag = query.holders.name(&params.country, through);
    present(&query, params, content, &fold, observed, identities, &tag, cancelled, &mut None)
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

fn present(query: &Query<'_>, params: &SpecialProjectsParams, content: &ProjectContent, fold: &Fold, observed: Vec<(i32, i32)>, identities: Vec<CountryIdentityRun>, tag: &str, cancelled: &AtomicBool, cache: &mut Option<Spelling>) -> io::Result<SpecialProjectsReply> {
    let mut naming = Naming::new(content);
    let mut definitions: BTreeMap<u32, i32> = BTreeMap::new();
    let mut characters: BTreeMap<u32, i32> = BTreeMap::new();
    for run in fold.episodes.iter().flat_map(|episode| &episode.runs) {
        let look = &fold.looks[run.presentation as usize];
        let first = definitions.entry(look.definition).or_insert(run.first_hour);
        *first = (*first).min(run.first_hour);
        if let Person::Character(id) = look.person {
            let first = characters.entry(id).or_insert(run.first_hour);
            *first = (*first).min(run.first_hour);
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
    let mut projects = fold.episodes.clone();
    for episode in &mut projects {
        let mut runs: Vec<SpecialProjectRun> = Vec::with_capacity(episode.runs.len());
        for mut run in std::mem::take(&mut episode.runs) {
            let look = &fold.looks[run.presentation as usize];
            let hour = run.first_hour;
            let name = names.entry((look.definition, hour)).or_insert_with(|| {
                let slot = query.holders.at(&params.country, hour).and_then(|run| match run.holder {
                    Holder::Found { slot, .. } => Some(slot),
                    Holder::Unresolved => None,
                });
                naming.definition(spelled.tokens.get(&look.definition).map(String::as_str), look.definition, tag, &mut |key| match slot {
                    Some(slot) => scoped_at(query, content, key, hour, slot),
                    None => Err("The project's name needs its country's scope, which is unresolved".to_owned()),
                })
            });
            let person = match &look.person {
                Person::Character(id) => spelled.people.get(id).cloned().unwrap_or_else(|| unread_person(&look.person)),
                other => unread_person(other),
            };
            run.presentation = presentations.id(ProjectPresentation { name: name.name.clone(), specialization: name.specialization.clone(),
                specialization_name: name.specialization_name.clone(), icon: name.icon.clone(), icon_missing: name.icon_missing.clone(),
                scientist: people.id(person), scientist_as_of: look.as_of });
            match runs.last_mut() {
                Some(held) if held.last_hour.checked_add(1) == Some(run.first_hour) && fold::same(held, &run) => held.last_hour = run.last_hour,
                _ => runs.push(run),
            }
        }
        episode.runs = runs;
    }
    let counts: BTreeSet<i32> = projects.iter().flat_map(|episode| &episode.runs).filter_map(|run| run.iterations).collect();
    let iteration_texts: BTreeMap<i32, Option<String>> = counts.into_iter().map(|count| (count, naming.iterations(count))).collect();
    let mut warnings = content.definitions.warnings.clone();
    warnings.extend(fold.warnings.iter().map(|(reason, count)| format!("{reason} ({count} observations).")));
    warnings.extend(naming.warnings());
    if !projects.is_empty() {
        warnings.push("Project icons and portraits are visual assets outside content hash coverage.".into());
    }
    let unavailable = if identities.is_empty() {
        Some("No country held this tag in the recording.".into())
    } else if observed.is_empty() {
        Some("Projects were not recorded for this country.".into())
    } else if content.definitions.projects.is_empty() {
        Some("The recorded content has no readable special-project definitions.".into())
    } else {
        None
    };
    let reply = SpecialProjectsReply {
        branch_id: params.branch_id.clone(),
        tag: tag.to_owned(),
        identities,
        available: observed.into_iter().map(|(first, last)| HourSpan { first_hour: first, last_hour: last }).collect(),
        projects,
        presentations: presentations.values,
        people: people.values,
        iteration_texts,
        unavailable,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn dashboard_follow::panel::Panel>, String> {
    let mut params = asked.clone();
    params.as_object_mut().ok_or("Invalid special-projects panel")?.remove("panel");
    let params: SpecialProjectsParams = serde_json::from_value(params).map_err(|e| e.to_string())?;
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
            ProjectDate::decl(),
            ProjectPerson::decl(),
            ProjectPresentation::decl(),
            ProjectState::decl(),
            SpecialProjectRun::decl(),
            ProjectEpisode::decl(),
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
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

pub struct Completions {
    pub events: Vec<ProjectCompletion>,
    pub unrecorded: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

pub fn completions(source: &Source, load: &LoadContext, query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<Completions> {
    let recording = query.replay.recording();
    let content = content(source, load)?;
    let names = country_ids::Names::new(&query.holders);
    let mut found: BTreeMap<u128, (u32, i32, i32)> = BTreeMap::new();
    let mut unrecorded: Vec<HourSpan> = Vec::new();
    for stretch in query.timeline.within(from, to) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held = available(query, stretch.run, TableId::ProjectHistory, stretch.first, stretch.last)?;
        unrecorded.extend(uncovered(&held, stretch.first, stretch.last).into_iter().map(|(first, last)| HourSpan { first_hour: first, last_hour: last }));
        let history = dashboard_query::cost::storage(|| lanes::history(recording, stretch.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::All, stretch.first, stretch.last))?;
        for (key, steps) in &history.keys {
            for step in steps {
                let Some(entry) = step.cells.as_deref().and_then(|cells| history_entry(*key as u32, cells)) else { continue };
                if entry.completion && (from..=to).contains(&entry.date) {
                    found.entry(*key).or_insert((stretch.run, entry.date, step.hour));
                }
            }
        }
    }
    let mut by_hour: BTreeMap<(u32, i32), BTreeSet<u128>> = BTreeMap::new();
    for (&key, &(run, _, observed)) in &found {
        by_hour.entry((run, observed)).or_default().insert(key >> 32);
    }
    let mut naming = Naming::new(&content);
    let mut links: HashMap<u32, RunLinks> = HashMap::new();
    let mut shown: HashMap<(u128, i32), ProjectCompletion> = HashMap::new();
    let mut frames = at_hour::Frames::new(query)?;
    for ((run, hour), projects) in by_hour {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let frame = frames.frame(hour, &[Read::these(TableId::Projects, projects.iter().copied())], "Special project completion")?;
        let rows = frame.complete::<tables::Projects>().unwrap_or(&[]);
        let definitions: Vec<u32> = rows.iter().map(|row| row.definition_id).collect();
        let lexical = frames.frame(hour, &[Read::these(TableId::LexicalTokens, definitions.iter().map(|&id| u128::from(id)))], "Special project names")?;
        let tokens = spellings(&lexical, definitions);
        let run_links = links_of(query, &mut links, run)?;
        for &key in &projects {
            let (owner, project) = ((key >> 32) as u32, key as u32);
            let tag = names.tag_at(run_links, owner, hour).map(str::to_owned);
            let row = rows.iter().find(|row| (row.owner, row.local_id) == (owner, project));
            let mut completion = ProjectCompletion { tag: tag.clone(), date: hour, observed_hour: hour, owner, project, name: None, icon: None, icon_missing: Some("The project was not read.".into()) };
            match (row, tag) {
                (Some(row), Some(tag)) => {
                    let slot = run_links.slot_at(owner, hour);
                    let mut scoped = |key: &str| match slot {
                        Some(slot) => scoped_at(query, &content, key, hour, slot),
                        None => Err("The project's name needs its country's scope, which is unresolved".to_owned()),
                    };
                    let named = naming.definition(tokens.get(&row.definition_id).map(String::as_str), row.definition_id, &tag, &mut scoped);
                    completion.name = Some(named.name);
                    completion.icon = named.icon;
                    completion.icon_missing = named.icon_missing;
                }
                (None, _) => naming.note("the completed project's row was not recorded", format!("{owner}:{project}")),
                (Some(_), None) => naming.note("no tag held the completed project's country", format!("{owner}:{project}")),
            }
            shown.insert((key, hour), completion);
        }
    }
    let mut events: Vec<ProjectCompletion> = found
        .into_iter()
        .filter_map(|(key, (_, date, observed))| shown.get(&(key >> 32, observed)).map(|completion| ProjectCompletion { date, ..completion.clone() }))
        .collect();
    events.sort_by(|a, b| (a.date, a.owner, a.project).cmp(&(b.date, b.owner, b.project)));
    Ok(Completions { events, unrecorded, warnings: naming.warnings() })
}
