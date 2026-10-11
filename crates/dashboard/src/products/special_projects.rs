//! Special projects over one country's branch. Daily values are admitted only
//! at slot_read, carried within consecutive recorded hours, never over a gap.
//! Character names and scoped project names are resolved at each frame's hour.

use super::{
    CountryParams, LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    character_portraits::{portrait_png, recorded_portrait},
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun},
    identity::character_script_token,
    politics_content::{PoliticsContent, politics_content},
};
use crate::{
    Source,
    catalogue::{self, Catalogue, View},
    metadata::{link_slot, slot_read, tagged_countries, tagged_country, validate_campaign},
};
use installed_content::{localisation::Argument, special_projects::Catalog};
use replay::{
    frame::{DecodedFrame, ViewIdentity},
    rows::{CountryLinkRow, ProjectHistoryRow, ProjectRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct SpecialProjects;
mod history;

mod completions;
pub(crate) use completions::completion_events;

// Project localisation keys and history sentinels.
const ITERATIONS_KEY: &str = "ONGOING_PROJECT_BREAKTHROUGHS_NUMBER";
const COMPLETION_MARKER: u32 = 0x4c17;
const PLACEHOLDER_ICON: &str = "GFX_PLACEHOLDER_sp_project_icon";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialProjectsParams {
    pub branch_id: String,
    pub tag: String,
    /// Original manual picks; a projected alias is never another selection root.
    #[serde(default)]
    pub selection_tags: Vec<String>,
}

impl SpecialProjectsParams {
    fn country(&self) -> CountryParams {
        CountryParams {
            branch_id: self.branch_id.clone(),
            tag: self.tag.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDate {
    pub hour: i32,
    /// False means the event occurred by this observation, not at this hour.
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPerson {
    /// None when the recorded reference cannot establish an assignment.
    pub assigned: Option<bool>,
    pub id: Option<u32>,
    pub name: Option<String>,
    pub portrait: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPresentation {
    pub name: String,
    pub specialization: Option<String>,
    pub specialization_name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub scientist: u32,
    /// Completed projects lose their program; this dates the last assignment.
    pub scientist_as_of: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProjectState {
    Running,
    Paused,
    Cancelled,
    Completed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub presentation: u32,
    /// Clamped prototype percent, separate from the phase ratio.
    pub progress: i32,
    pub state: ProjectState,
    pub completion: Option<ProjectDate>,
    pub cancellation: Option<ProjectDate>,
    /// Recorded history length minus the completion entry. Missing history is null.
    pub iterations: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEpisode {
    pub id: String,
    pub start: ProjectDate,
    pub runs: Vec<SpecialProjectRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectsReply {
    pub branch_id: String,
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub available: Vec<HourSpan>,
    pub read_hours: Vec<i32>,
    pub projects: Vec<ProjectEpisode>,
    pub presentations: Vec<ProjectPresentation>,
    pub people: Vec<ProjectPerson>,
    /// Native NUMBER text, localized once per distinct recorded count.
    pub iteration_texts: BTreeMap<i32, Option<String>>,
    /// Why the data/content is unavailable, distinct from an empty country set.
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

/// Completion follows the state predicate. The saved completed byte is
/// cleared independently when moving a project and does not establish it.
fn completed(project: &ProjectRow) -> bool {
    project.state.not_completed == 0 && project.state.current_offset == 0xe8
}

/// State offsets are machine-relative. A scripted cancel invalidates the
/// program reference and clears started only without history, leaving
/// the machine at 0x18: running needs both.
fn project_state(project: &ProjectRow) -> ProjectState {
    match project.state.current_offset {
        0x18 if project.started != 0 && project.program_reference != 0 => ProjectState::Running,
        0x18 => ProjectState::Cancelled,
        0x168 => ProjectState::Paused,
        0x1e8 => ProjectState::Cancelled,
        0xe8 if completed(project) => ProjectState::Completed,
        _ => ProjectState::Unknown,
    }
}

fn visible(project: &ProjectRow, already_open: bool) -> bool {
    project.started != 0
        || completed(project)
        || already_open
        || (project_state(project) == ProjectState::Cancelled
            && project.state.current_offset != 0x18)
}

/// Fixed5 ratio of prototype progress/100, clamped to [-1,1];
/// completed views force the meter to 1.
fn progress(project: &ProjectRow) -> i32 {
    if completed(project) {
        100
    } else {
        project.state.prototype_progress.clamp(-100, 100)
    }
}

fn completion_date(
    history: &[&ProjectHistoryRow],
    observed: i32,
    campaign_start: i32,
) -> ProjectDate {
    history
        .iter()
        .filter(|event| {
            event.prototype_reward_handle == COMPLETION_MARKER
                && event.option_handle == COMPLETION_MARKER
                && event.date <= observed
                && event.date >= campaign_start
        })
        .max_by_key(|event| event.ordinal)
        .map_or(
            ProjectDate {
                hour: observed,
                exact: false,
            },
            |event| ProjectDate {
                hour: event.date,
                exact: true,
            },
        )
}

/// Country key selection, shared by name and specialization.
fn country_key(catalogue: &Catalogue, tag: &str, key: &str) -> String {
    let prefixed = format!("{tag}_{key}");
    if catalogue.localisation.get(&prefixed).is_some() {
        prefixed
    } else {
        key.to_owned()
    }
}

/// A nonempty authored icon wins even if missing.
/// The fallback uses the GFX_ prefix.
fn icon_key(
    project: &installed_content::special_projects::Project,
    token: &str,
    tag: Option<&str>,
    has: impl Fn(&str) -> Result<bool, String>,
) -> Result<String, String> {
    if let Some(icon) = project.icon.as_ref().filter(|icon| !icon.is_empty()) {
        return Ok(icon.clone());
    }
    let prefix = if token.starts_with("GFX_") {
        ""
    } else {
        "GFX_"
    };
    if let Some(tag) = tag {
        let country = format!("{prefix}{tag}_{token}");
        if has(&country)? {
            return Ok(country);
        }
    }
    let generic = format!("{prefix}{token}");
    Ok(if has(&generic)? {
        generic
    } else {
        PLACEHOLDER_ICON.to_owned()
    })
}

struct Content {
    catalogue: Arc<Catalogue>,
    politics: Arc<PoliticsContent>,
    definitions: Catalog,
    icons: BTreeMap<String, Result<String, String>>,
    scoped: Vec<(u64, Arc<history::Names>)>,
    refusals: Mutex<catalogue::Refusals>,
}

impl Clone for Content {
    fn clone(&self) -> Self {
        Self {
            catalogue: Arc::clone(&self.catalogue),
            politics: Arc::clone(&self.politics),
            definitions: self.definitions.clone(),
            icons: self.icons.clone(),
            scoped: self.scoped.clone(),
            refusals: Mutex::new(Default::default()),
        }
    }
}

impl Content {
    fn load(politics: Arc<PoliticsContent>) -> io::Result<Self> {
        Ok(Self {
            catalogue: Arc::clone(&politics.catalogue),
            definitions: Catalog::read(&politics.catalogue.content.layers)?,
            politics,
            icons: BTreeMap::new(),
            scoped: Vec::new(),
            refusals: Mutex::default(),
        })
    }

    fn icon(&mut self, key: &str) -> Result<String, String> {
        self.icons
            .entry(key.to_owned())
            .or_insert_with(|| {
                let sprite = self
                    .catalogue
                    .sprites
                    .find(key)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Content has no sprite {key}."))?;
                let texture = sprite
                    .texture
                    .as_ref()
                    .ok_or_else(|| format!("Sprite {key} has no texture."))?;
                let decode = *self.catalogue.gui_decode.as_ref().map_err(Clone::clone)?;
                catalogue::texture_png(
                    &self.catalogue.content.layers,
                    texture,
                    decode,
                    sprite.frames,
                    96,
                )
                .map_err(|e| format!("{key}: {e}"))
            })
            .clone()
    }

    fn name(
        &self,
        definition: &installed_content::special_projects::Project,
        token: &str,
        tag: &str,
        slot: u32,
        frame: &DecodedFrame,
        definition_id: u32,
    ) -> Option<String> {
        // Use the authored narrative name when nonempty, otherwise the definition's
        // default name; then TAG_key if defined, then scope expansion in the owner's
        // country. The default key is the template token's string.
        let selected = country_key(&self.catalogue, tag, project_name_key(definition, token));
        if needs_scope(&self.catalogue.localisation, &selected) {
            return self
                .scoped
                .iter()
                .find(|(branch, _)| *branch == frame.branch)
                .and_then(|(_, names)| names.at(tag, slot, definition_id, frame.hour));
        }
        self.name_for(&selected, Some((slot, frame)))
    }

    fn name_for(&self, selected: &str, recorded: Option<(u32, &DecodedFrame)>) -> Option<String> {
        if !needs_scope(&self.catalogue.localisation, selected) {
            return self.catalogue.localised_with_refusals(
                selected,
                &[],
                View::Scripted,
                &self.refusals,
            );
        }
        let Some((slot, frame)) = recorded else {
            return self.catalogue.localised_offline_country(selected);
        };
        let scripted = self.catalogue.scripted_content().ok()?;
        self.catalogue.localise_in(
            selected,
            &crate::scripted_text::ScriptedText::new(
                frame,
                &self.catalogue.localisation,
                scripted,
                slot,
            ),
        )
    }

    fn specialization_name(&self, kind: &str, tag: Option<&str>) -> Option<String> {
        let key = tag.map_or_else(
            || kind.to_owned(),
            |tag| country_key(&self.catalogue, tag, kind),
        );
        self.catalogue
            .localised_with_refusals(&key, &[], View::Plain, &self.refusals)
    }

    fn iterations(&self, count: Option<i32>) -> Option<String> {
        // NUMBER is passed as Int32 (argument type 5).
        let count = count?;
        self.catalogue.localised_with_refusals(
            ITERATIONS_KEY,
            &[("NUMBER", Argument::Int32(count))],
            View::Plain,
            &self.refusals,
        )
    }
}

fn project_name_key<'a>(
    definition: &'a installed_content::special_projects::Project,
    token: &'a str,
) -> &'a str {
    definition
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(token)
}

/// Country-independent presentation inputs; the recorded path selects country
/// overrides and provides scoped name and character inputs.
pub(crate) fn check_content(
    politics: Arc<PoliticsContent>,
) -> io::Result<Vec<crate::content_check::Finding>> {
    use crate::content_check::{Finding, Kind};
    let mut content = Content::load(politics)?;
    let mut findings: Vec<_> = content
        .definitions
        .warnings
        .iter()
        .cloned()
        // Catalog::warnings names file or definition reads it could not follow.
        .map(|message| Finding::new(Kind::Refused, "special-project preparation", message))
        .collect();
    if let Some(reason) = &content.definitions.unsettled {
        findings.push(Finding::new(
            Kind::Refused,
            "special project inventory",
            reason,
        ));
        return Ok(findings);
    }
    for (token, definition) in content.definitions.projects.clone() {
        let subject = format!("special project {token}");
        if let Some(reason) = definition.refusal.as_ref() {
            findings.push(Finding::new(Kind::Refused, subject, reason));
            continue;
        }
        content.name_for(project_name_key(&definition, &token), None);
        if let Some(kind) = &definition.specialization {
            if content.definitions.specializations.contains(kind) {
                content.specialization_name(kind, None);
            } else {
                findings.push(Finding::new(
                    Kind::Refused,
                    &subject,
                    format!("Project {token}: specialization {kind} is absent from content"),
                ));
            }
        }
        let key = icon_key(&definition, &token, None, |key| {
            content
                .catalogue
                .sprites
                .find(key)
                .map(|sprite| sprite.is_some())
                .map_err(|e| e.to_string())
        });
        match key {
            Ok(key)
                if key != PLACEHOLDER_ICON
                    || definition
                        .icon
                        .as_ref()
                        .is_some_and(|icon| !icon.is_empty()) =>
            {
                if let Err(reason) = content.icon(&key) {
                    findings.push(Finding::new(
                        Kind::Fallback,
                        &subject,
                        format!("{reason}; a placeholder is shown"),
                    ));
                }
            }
            Ok(_) => {}
            Err(reason) => findings.push(Finding::new(Kind::Refused, &subject, reason)),
        }
        findings.extend(
            content
                .catalogue
                .take_findings()
                .into_iter()
                .map(|mut finding| {
                    finding.subject = subject.clone();
                    finding
                }),
        );
        findings.push(Finding::new(Kind::Offline, &subject,
            "special-project country name/specialization overrides need the recorded owner; unavailable offline"));
        if definition.icon.as_ref().is_none_or(|icon| icon.is_empty()) {
            findings.push(Finding::new(Kind::Offline, &subject,
                "special-project country-prefixed icon selection needs the recorded owner; unavailable offline"));
        }
        findings.push(Finding::new(Kind::Offline, &subject,
            "special-project iteration counts and scientist names/portraits need recorded project/program/character rows; unavailable offline"));
    }
    Ok(findings)
}

#[derive(Default)]
struct Builder {
    episodes: Vec<ProjectEpisode>,
    /// Matching episodes survive unrecorded hours, but their runs do not.
    open: BTreeMap<(u32, u32), usize>,
    identity: Option<(u32, u32)>,
    campaign_start: Option<i32>,
    content_ids: BTreeMap<(u32, String), u32>,
    scoped_names: BTreeMap<u32, (installed_content::special_projects::Project, String)>,
    person_views: Vec<ViewIdentity>,
    lexical_view: Option<ViewIdentity>,
    person_cache: BTreeMap<u64, u32>,
    presentations: Vec<ProjectPresentation>,
    people: Vec<ProjectPerson>,
    presentation_ids: BTreeMap<ProjectPresentation, u32>,
    person_ids: BTreeMap<ProjectPerson, u32>,
    available: Vec<HourSpan>,
    reads: Vec<i32>,
    last: Option<((u32, u32), i32)>,
    admitted: bool,
    warnings: BTreeMap<String, usize>,
    iteration_texts: BTreeMap<i32, Option<String>>,
    needs_scope: bool,
    /// The feed needs dates and episode identity, not every progress change.
    compact: bool,
    last_iterations: BTreeMap<usize, i32>,
    completions: BTreeMap<usize, CompletionObservation>,
}

#[derive(Clone)]
struct CompletionObservation {
    project: ProjectRow,
    identity: (u32, u32),
    date: ProjectDate,
    observed_hour: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectCompletion {
    pub tag: String,
    pub date: ProjectDate,
    pub observed_hour: i32,
    pub definition_id: u32,
    pub item_token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Default)]
pub(crate) struct ProjectCompletions {
    pub events: Vec<ProjectCompletion>,
    pub warnings: Vec<String>,
    pub unavailable: Option<String>,
    pub refusals: Vec<(String, usize)>,
}

fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(span) = spans.last_mut()
        && span.last_hour.checked_add(1) == Some(hour)
    {
        span.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

/// Rows are ordered by (raw owner, local key); include every link to the slot.
fn owned_rows<'a, T>(
    rows: &'a [T],
    links: &'a [CountryLinkRow],
    slot: u32,
    owner: impl Fn(&T) -> u32 + Copy + 'a,
) -> impl Iterator<Item = &'a T> {
    links
        .iter()
        .filter(move |link| link.canonical_slot == slot)
        .flat_map(move |link| {
            let from = rows.partition_point(|row| owner(row) < link.id);
            let to = rows.partition_point(|row| owner(row) <= link.id);
            rows[from..to].iter()
        })
}

impl Builder {
    fn warn(&mut self, reason: impl Into<String>) {
        *self.warnings.entry(reason.into()).or_default() += 1;
    }

    fn person(
        &mut self,
        frame: &DecodedFrame,
        project: &ProjectRow,
        content: Option<&Content>,
    ) -> u32 {
        if let Some(id) = self.person_cache.get(&project.program_reference) {
            return *id;
        }
        // Scientist assignment uses the full character reference.
        // The low reference type is 73; program reference type is 83.
        let mut assigned = None;
        let person = (|| {
            if project.program_reference == 0 {
                assigned = Some(false);
                return Err("No scientist assigned.".to_owned());
            }
            if project.program_reference as u32 != 83 {
                return Err("Project program reference has an unknown native type.".into());
            }
            let programs = frame
                .complete::<tables::ProjectPrograms>()
                .map_err(|_| "ProjectPrograms not recorded.".to_owned())?;
            let id = (project.program_reference >> 32) as u32;
            let program = programs
                .binary_search_by_key(&id, |row| row.id)
                .ok()
                .map(|i| &programs[i])
                .ok_or_else(|| format!("Program {id} was not recorded."))?;
            if program.scientist == 0 {
                assigned = Some(false);
                return Err("No scientist assigned.".into());
            }
            if program.scientist as u32 != 73 {
                return Err("Scientist reference has an unknown native type.".into());
            }
            assigned = Some(true);
            let id = (program.scientist >> 32) as u32;
            let characters = frame
                .complete::<tables::Characters>()
                .map_err(|_| "Characters not recorded.".to_owned())?;
            let character = characters
                .binary_search_by_key(&id, |row| row.id)
                .ok()
                .map(|i| &characters[i])
                .ok_or_else(|| format!("Character {id} was not recorded."))?;
            let lexical = frame
                .complete::<tables::LexicalTokens>()
                .unwrap_or_default();
            let script = character_script_token(character, lexical);
            let saved = String::from_utf8_lossy(&character.token);
            let name = content.and_then(|content| {
                content
                    .politics
                    .character_name_with_refusals(script, &saved, &content.refusals)
            });
            // Scientist portraits request role 4, size 1.
            // Use shared holder selection, including its fallback.
            let portrait = recorded_portrait(
                frame.complete::<tables::CharacterPortraits>().ok(),
                id,
                4,
                replay::rows::PORTRAIT_LARGE,
            );
            let art = content.map_or_else(
                || Err("Content is unavailable.".into()),
                |content| portrait_png(&content.politics, &portrait),
            );
            Ok(ProjectPerson {
                assigned,
                id: Some(id),
                name,
                portrait: art.as_ref().ok().cloned(),
                missing: art.err(),
            })
        })()
        .unwrap_or_else(|reason| ProjectPerson {
            assigned,
            id: None,
            name: None,
            portrait: None,
            missing: Some(reason),
        });
        if let Some(id) = self.person_ids.get(&person) {
            self.person_cache.insert(project.program_reference, *id);
            return *id;
        }
        let id = self.people.len() as u32;
        self.person_ids.insert(person.clone(), id);
        self.people.push(person);
        self.person_cache.insert(project.program_reference, id);
        id
    }

    fn presentation(
        &mut self,
        frame: &DecodedFrame,
        project: &ProjectRow,
        tag: &str,
        slot: u32,
        previous: Option<&SpecialProjectRun>,
        mut content: Option<&mut Content>,
    ) -> u32 {
        let mut scientist = self.person(frame, project, content.as_deref());
        let mut scientist_as_of = None;
        // Completion clears the program. Retain a dated last assignment only
        // within this episode, never claim a current one.
        if completed(project)
            && let Some(previous) =
                previous.filter(|run| run.last_hour.checked_add(1) == Some(frame.hour))
        {
            let old = &self.presentations[previous.presentation as usize];
            if self.people[old.scientist as usize].id.is_some() {
                scientist = old.scientist;
                scientist_as_of = Some(old.scientist_as_of.unwrap_or(previous.last_hour));
            }
        }
        let key = (project.definition_id, tag.to_owned());
        if let Some(id) = self.content_ids.get(&key).copied() {
            let old = &self.presentations[id as usize];
            let name = match self.scoped_names.get(&project.definition_id) {
                Some((definition, token)) => content
                    .as_ref()
                    .and_then(|content| {
                        content.name(definition, token, tag, slot, frame, project.definition_id)
                    })
                    .unwrap_or_else(|| "Project name unavailable".into()),
                None => old.name.clone(),
            };
            if old.scientist == scientist
                && old.scientist_as_of == scientist_as_of
                && old.name == name
            {
                return id;
            }
            let mut info = old.clone();
            info.name = name;
            info.scientist = scientist;
            info.scientist_as_of = scientist_as_of;
            let id = self.intern_presentation(info);
            self.content_ids.insert(key, id);
            return id;
        }
        // Project construction copies the template token.
        let token = frame
            .complete::<tables::LexicalTokens>()
            .ok()
            .and_then(|rows| {
                rows.binary_search_by_key(&project.definition_id, |row| row.token)
                    .ok()
                    .and_then(|i| std::str::from_utf8(&rows[i].literal).ok())
            })
            .map(str::to_owned);
        let token = token.unwrap_or_else(|| {
            self.warn(format!(
                "Definition token {} is not recorded",
                project.definition_id
            ));
            format!("Project definition {}", project.definition_id)
        });
        let mut info = ProjectPresentation {
            name: "Project name unavailable".into(),
            specialization: None,
            specialization_name: None,
            icon: None,
            icon_missing: Some("Content is unavailable.".into()),
            scientist,
            scientist_as_of,
        };
        if let Some(content) = content.as_mut() {
            if let Some(reason) = content.definitions.unsettled.as_ref().or_else(|| {
                content
                    .definitions
                    .projects
                    .get(&token)
                    .and_then(|project| project.refusal.as_ref())
            }) {
                info.icon_missing = Some(reason.clone());
                self.warn(reason.clone());
            } else if let Some(definition) = content.definitions.projects.get(&token).cloned() {
                let key = country_key(
                    &content.catalogue,
                    tag,
                    definition
                        .name
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&token),
                );
                if needs_scope(&content.catalogue.localisation, &key) {
                    self.needs_scope = true;
                    self.scoped_names
                        .insert(project.definition_id, (definition.clone(), token.clone()));
                }
                info.name = content
                    .name(&definition, &token, tag, slot, frame, project.definition_id)
                    .unwrap_or_else(|| {
                        self.warn(format!("Project name {token} could not be resolved"));
                        "Project name unavailable".into()
                    });
                info.specialization = definition.specialization.clone();
                // Use the specialization token, not blueprint_image
                // or a vanilla type enumeration.
                if let Some(kind) = &definition.specialization {
                    if content.definitions.specializations.contains(kind) {
                        info.specialization_name = content.specialization_name(kind, Some(tag));
                    } else {
                        self.warn(format!(
                            "Project {token}: specialization {kind} is absent from content"
                        ));
                    }
                }
                let art = icon_key(&definition, &token, Some(tag), |key| {
                    content
                        .catalogue
                        .sprites
                        .find(key)
                        .map(|sprite| sprite.is_some())
                        .map_err(|e| format!("Sprite {key}: {e}"))
                })
                .and_then(|key| content.icon(&key));
                match art {
                    Ok(icon) => {
                        info.icon = Some(icon);
                        info.icon_missing = None;
                    }
                    Err(reason) => {
                        self.warn(reason.clone());
                        info.icon_missing = Some(reason);
                    }
                }
            } else {
                self.warn(format!("Project definition {token} is absent from content"));
                info.icon_missing = Some(format!("Definition {token} is absent from content."));
            }
        }
        let id = self.intern_presentation(info);
        self.content_ids.insert(key, id);
        id
    }

    fn intern_presentation(&mut self, info: ProjectPresentation) -> u32 {
        if let Some(id) = self.presentation_ids.get(&info) {
            return *id;
        }
        let id = self.presentations.len() as u32;
        self.presentation_ids.insert(info.clone(), id);
        self.presentations.push(info);
        id
    }

    #[cfg(test)]
    fn observe(&mut self, frame: &DecodedFrame, tag: &str, content: Option<&mut Content>) {
        let lexical = frame.tables[TableId::LexicalTokens.index()].identity();
        self.observe_span(frame, &lexical, tag, content);
    }

    /// `lexical` names the recorded LexicalTokens read. A span's projected
    /// frame holds only the tokens it reads; presentations follow each read.
    fn observe_span(
        &mut self,
        frame: &DecodedFrame,
        lexical: &ViewIdentity,
        tag: &str,
        content: Option<&mut Content>,
    ) {
        let identity = frame
            .complete::<tables::Countries>()
            .ok()
            .zip(frame.complete::<tables::CountryLinks>().ok())
            .and_then(|(countries, links)| {
                tagged_country(countries, links, tag)
                    .found()
                    .map(|(country, slot)| ((slot, country.id), links))
            });
        // Cached presentations follow each dictionary read once the country is
        // identified; the world feed's compact builders present nothing.
        if identity.is_some() && self.lexical_view.as_ref() != Some(lexical) {
            self.content_ids.clear();
            self.scoped_names.clear();
            self.lexical_view = Some(lexical.clone());
        }
        let projects = frame.complete::<tables::Projects>().ok();
        let histories = frame.complete::<tables::ProjectHistory>().ok();
        let rows = identity.map(|((slot, _), links)| {
            (
                projects
                    .map(|rows| owned_rows(rows, links, slot, |row| row.owner).collect::<Vec<_>>()),
                histories
                    .map(|rows| owned_rows(rows, links, slot, |row| row.owner).collect::<Vec<_>>()),
            )
        });
        self.observe_rows(
            frame,
            tag,
            identity.map(|(identity, _)| identity),
            rows.as_ref().and_then(|(rows, _)| rows.as_deref()),
            rows.as_ref().and_then(|(_, rows)| rows.as_deref()),
            content,
        );
    }

    /// Both country history and the world feed use this admission/episode path.
    /// The world caller resolves identities and groups raw owners once per frame.
    fn observe_rows(
        &mut self,
        frame: &DecodedFrame,
        tag: &str,
        identity: Option<(u32, u32)>,
        projects: Option<&[&ProjectRow]>,
        history_rows: Option<&[&ProjectHistoryRow]>,
        mut content: Option<&mut Content>,
    ) {
        let hour = frame.hour;
        let continuing = identity.is_some_and(|identity| {
            self.last
                .is_some_and(|(old, seen)| identity == old && seen.checked_add(1) == Some(hour))
        });
        if !continuing {
            self.admitted = false;
        }
        self.last = identity.map(|id| (id, hour));
        let Some((slot, country)) = identity else {
            return;
        };
        if self.identity.is_some_and(|old| old != (slot, country)) {
            self.open.clear();
        }
        self.identity = Some((slot, country));
        self.campaign_start.get_or_insert(hour);
        let person_tables = [
            TableId::ProjectPrograms,
            TableId::Characters,
            TableId::CharacterPortraits,
            TableId::LexicalTokens,
        ];
        if self.person_views.len() != person_tables.len()
            || person_tables
                .iter()
                .zip(&self.person_views)
                .any(|(table, seen)| !seen.is(&frame.tables[table.index()]))
        {
            self.person_cache.clear();
            self.person_views = person_tables
                .iter()
                .map(|table| frame.tables[table.index()].identity())
                .collect();
        }
        let Some(projects) = projects else {
            self.admitted = false;
            return;
        };
        let read = slot_read(&frame.metrics, TableId::Projects, hour);
        if read {
            self.admitted = true;
            if !self.compact {
                self.reads.push(hour);
            }
        }
        if !self.admitted {
            return;
        }
        if !self.compact || self.available.is_empty() {
            mark(&mut self.available, hour);
        }
        let mut histories: BTreeMap<u32, Vec<&ProjectHistoryRow>> = BTreeMap::new();
        let history_available = history_rows.is_some();
        if let Some(rows) = history_rows {
            for &event in rows {
                histories.entry(event.project).or_default().push(event);
            }
        }
        let mut next = BTreeMap::new();
        for &project in projects {
            let state = project_state(project);
            let done = state == ProjectState::Completed;
            // A never-started project also rests at 0x18; only a machine
            // cancellation (0x1e8) or an open episode makes it a lane.
            if !visible(
                project,
                self.open
                    .contains_key(&(project.local_id, project.definition_id)),
            ) {
                continue;
            }
            let key = (project.local_id, project.definition_id);
            let history = histories
                .get(&project.local_id)
                .map_or(&[][..], Vec::as_slice);
            let count = history_available.then(|| history.len() as i32 - i32::from(done));
            if self.open.get(&key).is_some_and(|index| {
                self.episodes[*index].runs.last().is_some_and(|run| {
                    matches!(run.state, ProjectState::Completed | ProjectState::Cancelled)
                        && matches!(state, ProjectState::Running | ProjectState::Paused)
                        || self
                            .last_iterations
                            .get(index)
                            .copied()
                            .zip(count)
                            .is_some_and(|(old, new)| old > new)
                })
            }) {
                self.open.remove(&key);
            }
            let index = match self.open.get(&key) {
                Some(index) => *index,
                None => {
                    // Starting records no start event. First observation is an upper bound
                    // when no matching episode exists.
                    let index = self.episodes.len();
                    self.episodes.push(ProjectEpisode {
                        id: format!("{}:{hour}:{index}", project.local_id),
                        start: ProjectDate { hour, exact: false },
                        runs: Vec::new(),
                    });
                    index
                }
            };
            next.insert(key, index);
            // Acceptance shows count minus the completion entry;
            // do not substitute state.iterations or saturate at zero.
            let completion = done.then(|| {
                let date = completion_date(history, hour, self.campaign_start.unwrap_or(hour));
                if !date.exact {
                    self.episodes[index]
                        .runs
                        .last()
                        .and_then(|run| run.completion.clone())
                        .unwrap_or(date)
                } else {
                    date
                }
            });
            let previous = self.episodes[index].runs.last().cloned();
            let presentation = if self.compact {
                // Presentation is resolved only for actual completion rows,
                // at the event's own hour, after this shared episode scan.
                0
            } else {
                self.presentation(
                    frame,
                    project,
                    tag,
                    slot,
                    previous.as_ref(),
                    content.as_deref_mut(),
                )
            };
            if self.compact
                && let Some(date) = &completion
            {
                self.completions
                    .entry(index)
                    .and_modify(|event| {
                        if event.date != *date {
                            event.date = date.clone();
                            event.observed_hour = hour;
                        }
                    })
                    .or_insert_with(|| CompletionObservation {
                        project: project.clone(),
                        identity: (slot, country),
                        date: date.clone(),
                        observed_hour: hour,
                    });
            }
            if let Some(count) = count {
                self.last_iterations.insert(index, count);
                if !self.compact {
                    self.iteration_texts.entry(count).or_insert_with(|| {
                        content
                            .as_ref()
                            .and_then(|content| content.iterations(Some(count)))
                    });
                }
            }
            if state == ProjectState::Unknown {
                self.warn(format!(
                    "Project {} has unsupported native state offset {:#x}",
                    project.local_id, project.state.current_offset
                ));
            }
            let cancellation = (state == ProjectState::Cancelled).then(|| {
                previous
                    .as_ref()
                    .and_then(|run| run.cancellation.clone())
                    .unwrap_or(ProjectDate { hour, exact: false })
            });
            let mut run = SpecialProjectRun {
                first_hour: hour,
                last_hour: hour,
                presentation,
                progress: progress(project),
                state,
                completion,
                cancellation,
                iterations: count,
            };
            let runs = &mut self.episodes[index].runs;
            if let Some(last) = runs.last_mut() {
                run.first_hour = last.first_hour;
                run.last_hour = last.last_hour;
                if *last == run && last.last_hour.checked_add(1) == Some(hour) {
                    last.last_hour = hour;
                    continue;
                }
                run.first_hour = hour;
                run.last_hour = hour;
            }
            if self.compact {
                runs.clear();
            }
            runs.push(run);
        }
        self.open = next;
    }
}

/// Scope selection after plain localisation expands aliases; literal dollars need no frame.
fn needs_scope(loc: &installed_content::localisation::Localisation, key: &str) -> bool {
    loc.localise(key.as_bytes())
        .is_ok_and(|text| text.contains('['))
}

fn project(
    scan: &history::BranchScan,
    params: &CountryParams,
    identities: Option<&[&CountryIdentityRun]>,
    mut content: Option<&mut Content>,
    cancelled: &AtomicBool,
) -> io::Result<(SpecialProjectsReply, bool)> {
    if let Some(content) = content.as_ref() {
        *content
            .refusals
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Default::default();
    }
    let mut result = Builder {
        campaign_start: scan.campaign_start,
        ..Default::default()
    };
    for span in &scan.spans {
        crate::metadata::check_cancel(cancelled)?;
        let Some(mut frame) = span.inputs.actor_frame(params.branch(), &params.tag) else {
            result.admitted = false;
            result.last = None;
            continue;
        };
        let actor = frame
            .complete::<tables::Countries>()
            .ok()
            .and_then(|rows| rows.first())
            .map(|row| (row.canonical_slot, row.id));
        for hour in span.first_hour..=span.last_hour {
            crate::metadata::check_cancel(cancelled)?;
            if let Some(identities) = identities {
                let index = identities.partition_point(|identity| identity.last_hour < hour);
                if identities.get(index).is_none_or(|identity| {
                    identity.first_hour > hour
                        || actor != Some((identity.slot, identity.country_id))
                }) {
                    result.admitted = false;
                    result.last = None;
                    continue;
                }
            }
            frame.hour = hour;
            frame.metrics.table_ns[TableId::Projects.index()] =
                u64::from(span.reads.binary_search(&hour).is_ok());
            result.observe_span(
                &frame,
                &span.inputs.lexical,
                &params.tag,
                content.as_deref_mut(),
            );
        }
    }
    let mut warnings = content.as_ref().map_or_else(Vec::new, |content| {
        let mut warnings = content.definitions.warnings.clone();
        warnings.extend(content.catalogue.verification.iter().cloned());
        warnings.extend(content.catalogue.warnings.iter().cloned());
        warnings.extend(content.catalogue.reader_warning_lines());
        warnings.extend(
            content
                .refusals
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lines("Project texts"),
        );
        if let Some((_, names)) = content
            .scoped
            .iter()
            .find(|(branch, _)| *branch == params.branch())
        {
            warnings.extend(names.warnings(&params.tag));
        }
        warnings
    });
    warnings.extend(
        result
            .warnings
            .into_iter()
            .map(|(reason, count)| format!("{reason} ({count} observations).")),
    );
    let unavailable = if result.available.is_empty() {
        Some("Projects was unavailable, incomplete, or never read for this country.".into())
    } else if content
        .as_ref()
        .is_some_and(|content| content.definitions.projects.is_empty())
    {
        Some("The recorded content has no readable special-project definitions.".into())
    } else {
        None
    };
    let needs_scope = result.needs_scope;
    Ok((
        SpecialProjectsReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            identities: Vec::new(),
            available: result.available,
            read_hours: result.reads,
            projects: result.episodes,
            presentations: result.presentations,
            people: result.people,
            iteration_texts: result.iteration_texts,
            unavailable,
            warnings,
        },
        needs_scope,
    ))
}

fn append_identities(
    reply: &mut SpecialProjectsReply,
    identities: &[&CountryIdentityRun],
) -> io::Result<()> {
    // The empty [] is already measured in the complete reply. Borrow metadata
    // until its replacement fits, before copying any native identity runs.
    let error = || io::Error::other("Special projects reply exceeds its transport reply budget");
    let base = crate::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(error)?;
    let added =
        crate::transport::json_len(&identities, MAX_PRODUCT_REPLY_BYTES).ok_or_else(error)?;
    if base.saturating_sub(2).saturating_add(added) > MAX_PRODUCT_REPLY_BYTES {
        return Err(error());
    }
    reply.identities = identities
        .iter()
        .map(|identity| (**identity).clone())
        .collect();
    super::check_reply_budget(reply)
}

type ReplyKey = (u64, String, Vec<String>);

#[derive(Default)]
struct Cache {
    catalogue: Option<Arc<Catalogue>>,
    content: Option<Content>,
    replies: VecDeque<(ReplyKey, SpecialProjectsReply)>,
    completions: VecDeque<(u64, Arc<ProjectCompletions>)>,
    content_error: Option<String>,
}

fn prepare_cache(cache: &mut Cache, politics: &io::Result<Arc<PoliticsContent>>) {
    let catalogue = politics
        .as_ref()
        .ok()
        .map(|content| Arc::clone(&content.catalogue));
    let same = match (&cache.catalogue, &catalogue) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    cache.replies.clear();
    cache.completions.clear();
    cache.content_error = None;
    cache.content = match politics.as_ref() {
        Ok(politics) => match Content::load(Arc::clone(politics)) {
            Ok(content) => Some(content),
            Err(error) => {
                cache.content_error = Some(format!("Special-project content unavailable: {error}"));
                None
            }
        },
        Err(_) => None,
    };
    cache.catalogue = catalogue;
}

impl Product for SpecialProjects {
    const KEY: &'static str = "special-projects";
    const CONTENT_DEPENDENT: bool = true;
    type Params = SpecialProjectsParams;
    type Reply = SpecialProjectsReply;
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
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.country().validate()?;
        for tag in &params.selection_tags {
            super::validate_tag(tag)?;
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let politics = context
            .content()
            .and_then(|options| politics_content(source, options));
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        prepare_cache(&mut cache, &politics);
        let country = params.country();
        let mut roots = params.selection_tags.clone();
        roots.sort();
        roots.dedup();
        let key = (country.branch(), params.tag.clone(), roots.clone());
        if let Some((_, reply)) = cache.replies.iter().find(|(seen, _)| *seen == key) {
            return Ok(reply.clone());
        }
        let histories =
            country_history::branch_histories(source, country.branch(), false, &[], cancelled)?;
        let selected = country_history::selected_identities(&histories, &roots);
        let identities: Vec<_> = histories
            .countries
            .get(&params.tag)
            .into_iter()
            .flat_map(|history| history.identities.iter())
            .filter(|identity| {
                roots.is_empty() || selected.contains(&(identity.slot, identity.country_id))
            })
            .collect();
        let scan = history::cached(source, country.branch(), cancelled)?;
        let (mut reply, scoped) = project(
            &scan,
            &country,
            Some(&identities),
            cache.content.as_mut(),
            cancelled,
        )?;
        if scoped
            && let Some(content) = cache.content.as_mut()
            && !content
                .scoped
                .iter()
                .any(|(branch, _)| *branch == country.branch())
        {
            let names = history::scoped_names(
                source.replay_ref(),
                &scan,
                country.branch(),
                content,
                cancelled,
            )?;
            content.scoped.push((country.branch(), Arc::new(names)));
            if content.scoped.len() > 4 {
                content.scoped.remove(0);
            }
            reply = project(&scan, &country, Some(&identities), Some(content), cancelled)?.0;
        }
        // Content only names and draws the recorded projects; the data stays available.
        if let Some(reason) = &cache.content_error {
            reply.warnings.push(reason.clone());
        }
        if let Err(reason) = politics {
            reply.warnings.push(format!(
                "Project content, names and art unavailable: {reason}"
            ));
        }
        if !reply.projects.is_empty() {
            reply.warnings.push(
                "Project icons and portraits are visual assets outside content hash coverage."
                    .into(),
            );
        }
        append_identities(&mut reply, &identities)?;
        cache.replies.push_back((key, reply.clone()));
        // Response cache budget only, never a country/capture limit.
        while cache.replies.len() > 16 {
            cache.replies.pop_front();
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod bench;
#[cfg(test)]
mod history_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, SharedView, TableView},
        rows::{CountryLinkRow, CountryRow, TableRows},
    };

    pub(super) struct TempRoot(pub(super) std::path::PathBuf);
    impl TempRoot {
        pub(super) fn new(label: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "dashboard-projects-{label}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scripted_cancel_leaves_offset_0x18_but_ends_the_episode_cancelled() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut sample = frame(100, true, 1);
        let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", None);
        assert_eq!(
            builder.episodes[0].runs.last().unwrap().state,
            ProjectState::Running
        );
        // Cancellation invalidates the program reference only.
        sample.hour = 101;
        project.program_reference = 0;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project.clone()]));
        builder.observe(&sample, "mod tag", None);
        let run = builder.episodes[0].runs.last().unwrap();
        assert_eq!(run.state, ProjectState::Cancelled);
        assert_eq!(
            run.cancellation,
            Some(ProjectDate {
                hour: 101,
                exact: false
            })
        );
        // A never-started project resting at 0x18 makes no lane.
        let mut fresh = Builder::default();
        project.started = 0;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        fresh.observe(&sample, "mod tag", None);
        assert!(fresh.episodes.is_empty());
    }
    #[test]
    fn native_states_pause_cancel_and_unset_completion_dates_are_explicit() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut sample = frame(100, true, 1);
        let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
        let mut builder = Builder::default();
        for (hour, offset, state) in [
            (100, 0x18, ProjectState::Running),
            (101, 0x168, ProjectState::Paused),
            (102, 0x1e8, ProjectState::Cancelled),
        ] {
            sample.hour = hour;
            project.state.current_offset = offset;
            *sample.tables[TableId::Projects.index()] =
                TableView::Complete(TableRows::Projects(vec![project.clone()]));
            builder.observe(&sample, "mod tag", None);
            assert_eq!(builder.episodes[0].runs.last().unwrap().state, state);
        }
        assert_eq!(
            builder.episodes[0].runs.last().unwrap().cancellation,
            Some(ProjectDate {
                hour: 102,
                exact: false
            })
        );
        sample.hour = 103;
        builder.observe(&sample, "mod tag", None);
        assert_eq!(
            builder.episodes[0]
                .runs
                .last()
                .unwrap()
                .cancellation
                .as_ref()
                .unwrap()
                .hour,
            102
        );
        let unset = ProjectHistoryRow {
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date: i32::MIN,
            ..Default::default()
        };
        assert_eq!(
            completion_date(&[&unset], 103, 100),
            ProjectDate {
                hour: 103,
                exact: false
            }
        );
        project.state.current_offset = 0x123;
        assert_eq!(project_state(&project), ProjectState::Unknown);
    }

    #[test]
    fn gaps_keep_an_episode_but_resets_and_changed_owners_do_not() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut builder = Builder::default();
        let mut sample = frame(100, true, 1);
        builder.observe(&sample, "mod tag", None);
        sample.hour = 110;
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes.len(), 1);
        assert_eq!(
            builder.episodes[0]
                .runs
                .iter()
                .map(|run| (run.first_hour, run.last_hour))
                .collect::<Vec<_>>(),
            [(100, 100), (110, 110)]
        );
        *sample.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 6,
                canonical_slot: 0,
                tag: b"mod tag".to_vec(),
                ..Default::default()
            }]));
        *sample.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 6,
                canonical_slot: 0,
            }]));
        let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
        project.owner = 6;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        sample.hour = 111;
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes.len(), 2);
    }

    pub(super) fn frame(hour: i32, read: bool, count: usize) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            tables: std::array::from_fn(|i| {
                SharedView::from(TableView::Complete(TableRows::empty(TableId::ALL[i])))
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 5,
                canonical_slot: 0,
                tag: b"mod tag".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 5,
                canonical_slot: 0,
            }]));
        *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
            (0..count)
                .map(|i| ProjectRow {
                    owner: 5,
                    local_id: i as u32,
                    definition_id: 8,
                    started: 1,
                    program_reference: (12_u64 << 32) | 83,
                    state: replay::rows::ProjectState {
                        current_offset: 0x18,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect(),
        ));
        if read {
            frame.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        }
        frame
    }

    #[test]
    fn reads_are_admitted_by_the_slot_and_runs_break_at_gaps_and_incomplete_tables() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut builder = Builder::default();
        builder.observe(&frame(99, false, 1), "mod tag", None);
        assert!(builder.episodes.is_empty());
        builder.observe(&frame(101, true, 1), "mod tag", None);
        builder.observe(&frame(102, false, 1), "mod tag", None);
        assert_eq!(builder.episodes.len(), 1);
        assert_eq!(builder.episodes[0].runs.len(), 1);
        assert_eq!(builder.episodes[0].runs[0].last_hour, 102);
        assert_eq!(builder.reads, [101]);
        builder.observe(&frame(110, false, 1), "mod tag", None);
        assert_eq!(builder.episodes.len(), 1); // a carried baseline after a gap is not a read
        builder.observe(&frame(111, true, 1), "mod tag", None);
        assert_eq!(builder.episodes.len(), 1);
        let mut incomplete = frame(112, true, 1);
        *incomplete.tables[TableId::Projects.index()] = TableView::Unavailable {
            reason: "not captured".into(),
        };
        builder.observe(&incomplete, "mod tag", None);
        builder.observe(&frame(113, false, 1), "mod tag", None);
        builder.observe(&frame(114, true, 1), "mod tag", None);
        assert_eq!(builder.episodes.len(), 1);
        assert_eq!(
            builder
                .available
                .iter()
                .map(|s| (s.first_hour, s.last_hour))
                .collect::<Vec<_>>(),
            [(101, 102), (111, 111), (114, 114)]
        );
    }

    #[test]
    fn all_recorded_projects_are_kept_at_five_thousand_without_player_inputs() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut builder = Builder::default();
        builder.observe(&frame(100, true, 5000), "mod tag", None);
        assert_eq!(builder.episodes.len(), 5000);
        assert_eq!(builder.presentations.len(), 1);
        assert_eq!(builder.people.len(), 1);
    }

    #[test]
    fn completion_keeps_only_a_dated_last_assignment_and_a_gap_cannot_supply_it() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut sample = frame(100, true, 1);
        let mut project = ProjectRow {
            owner: 5,
            local_id: 0,
            definition_id: 8,
            started: 1,
            program_reference: (12_u64 << 32) | 83,
            ..Default::default()
        };
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project.clone()]));
        *sample.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![
                replay::rows::ProjectProgramRow {
                    id: 12,
                    owner: 5,
                    scientist: (73_u64 << 32) | 73,
                    ..Default::default()
                },
            ]));
        *sample.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
                id: 73,
                ..Default::default()
            }]));
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", None);
        sample.hour = 101;
        project.program_reference = 0;
        project.state.current_offset = 0xe8;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        *sample.tables[TableId::ProjectHistory.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        builder.observe(&sample, "mod tag", None);
        let run = builder.episodes[0].runs.last().unwrap();
        let info = &builder.presentations[run.presentation as usize];
        assert_eq!(builder.people[info.scientist as usize].id, Some(73));
        assert_eq!(info.scientist_as_of, Some(100));
        sample.hour = 102;
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes[0].runs.len(), 2);
        sample.hour = 110;
        builder.observe(&sample, "mod tag", None);
        let info =
            &builder.presentations[builder.episodes[0].runs.last().unwrap().presentation as usize];
        assert_eq!(builder.people[info.scientist as usize].id, None);
        assert_eq!(info.scientist_as_of, None);
    }

    #[test]
    fn person_cache_tracks_program_and_character_views() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut sample = frame(100, true, 1);
        let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
        project.program_reference = (12_u64 << 32) | 83;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        *sample.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![
                replay::rows::ProjectProgramRow {
                    id: 12,
                    scientist: (73_u64 << 32) | 73,
                    ..Default::default()
                },
            ]));
        *sample.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
                id: 73,
                ..Default::default()
            }]));
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", None);
        sample.hour = 101;
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.people.len(), 1);
        assert_eq!(builder.person_cache.len(), 1);
        *sample.tables[TableId::Characters.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        sample.hour = 102;
        builder.observe(&sample, "mod tag", None);
        let info =
            &builder.presentations[builder.episodes[0].runs.last().unwrap().presentation as usize];
        assert_eq!(builder.people[info.scientist as usize].assigned, Some(true));
        assert_eq!(builder.people[info.scientist as usize].id, None);
        *sample.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![
                replay::rows::ProjectProgramRow {
                    id: 12,
                    scientist: 0,
                    ..Default::default()
                },
            ]));
        sample.hour = 103;
        builder.observe(&sample, "mod tag", None);
        let info =
            &builder.presentations[builder.episodes[0].runs.last().unwrap().presentation as usize];
        assert_eq!(
            builder.people[info.scientist as usize].assigned,
            Some(false)
        );
    }

    #[test]
    fn country_roster_at_five_thousand_keeps_the_requested_ai_or_human_country() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut sample = frame(100, true, 1);
        *sample.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5000)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id - 1,
                    tag: format!("opaque country {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *sample.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=5000)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id - 1,
                    })
                    .collect(),
            ));
        *sample.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
            (1..=5000)
                .map(|owner| ProjectRow {
                    owner,
                    local_id: owner,
                    definition_id: 8,
                    started: 1,
                    ..Default::default()
                })
                .collect(),
        ));
        let mut builder = Builder::default();
        builder.observe(&sample, "opaque country 5000", None);
        assert_eq!(builder.episodes.len(), 1);
        assert!(builder.episodes[0].id.starts_with("5000:"));
    }

    #[test]
    fn history_count_uses_native_vector_length_missing_is_unknown_and_reset_is_a_new_episode() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut sample = frame(100, true, 1);
        let mut project = ProjectRow {
            owner: 5,
            local_id: 0,
            definition_id: 8,
            started: 1,
            ..Default::default()
        };
        project.state.iterations = 99;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project.clone()]));
        *sample.tables[TableId::ProjectHistory.index()] =
            TableView::Complete(TableRows::ProjectHistory(
                (0..3)
                    .map(|ordinal| ProjectHistoryRow {
                        owner: 5,
                        project: 0,
                        ordinal,
                        ..Default::default()
                    })
                    .collect(),
            ));
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes[0].runs[0].iterations, Some(3));
        sample.hour = 101;
        *sample.tables[TableId::ProjectHistory.index()] = TableView::Unavailable {
            reason: "missing".into(),
        };
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes[0].runs[1].iterations, None);
        sample.hour = 102;
        project.state.current_offset = 0xe8;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project.clone()]));
        let mut history: Vec<_> = (0..3)
            .map(|ordinal| ProjectHistoryRow {
                owner: 5,
                project: 0,
                ordinal,
                ..Default::default()
            })
            .collect();
        history.push(ProjectHistoryRow {
            owner: 5,
            project: 0,
            ordinal: 3,
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date: 101,
            ..Default::default()
        });
        *sample.tables[TableId::ProjectHistory.index()] =
            TableView::Complete(TableRows::ProjectHistory(history));
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes[0].runs[2].iterations, Some(3));
        assert_eq!(
            builder.episodes[0].runs[2].completion,
            Some(ProjectDate {
                hour: 101,
                exact: true
            })
        );
        sample.hour = 103;
        project.state.current_offset = 0x18;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        *sample.tables[TableId::ProjectHistory.index()] =
            TableView::Complete(TableRows::ProjectHistory(Vec::new()));
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.episodes.len(), 2);
        assert_eq!(
            builder.episodes[1].start,
            ProjectDate {
                hour: 103,
                exact: false
            }
        );
    }

    #[test]
    fn recorded_mod_stack_adds_projects_and_overrides_names_and_icons() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let guard = TempRoot::new("mod");
        let root = &guard.0;
        let base = root.join("base");
        let over = root.join("over");
        for (layer, path, text) in [
            (
                &base,
                "common/defines/test.lua",
                "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} } }",
            ),
            (
                &base,
                "common/special_projects/projects/base.txt",
                "base_project={specialization=base_kind}",
            ),
            (
                &base,
                "common/special_projects/specialization/kinds.txt",
                "base_kind={color={1 2 3}}",
            ),
            (
                &base,
                "localisation/english/project_l_english.yml",
                "\u{feff}l_english:\n base_project:0 \"Base project\"\n ONGOING_PROJECT_BREAKTHROUGHS_NUMBER:0 \"Base $NUMBER$\"\n",
            ),
            (
                &over,
                "common/special_projects/projects/mod.txt",
                "base_project={icon=GFX_mod_override narrative={name=mod_name}} mod_project={specialization=mod_kind complexity=sp_complexity.medium generic_prototype_rewards={existing_reward} unique_prototype_rewards={inline_reward={threshold={min=0 max=80} option={iteration_output={enable_equipments={mod_equipment} country_effects={custom_effect_tooltip=mod_tt}}}}}}",
            ),
            (
                &over,
                "common/special_projects/specialization/mod.txt",
                "mod_kind={color={7 8 9}}",
            ),
            (
                &over,
                "localisation/english/replace/project_l_english.yml",
                "\u{feff}l_english:\n mod_name:0 \"Renamed by mod\"\n mod_kind:0 \"Mod specialization\"\n ONGOING_PROJECT_BREAKTHROUGHS_NUMBER:0 \"Mod count $NUMBER|=$\"\n",
            ),
            (
                &over,
                "interface/mod.gfx",
                "spriteTypes={spriteType={name=GFX_mod_override texturefile=\"gfx/mod_project.dds\"}}",
            ),
        ] {
            let file = layer.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let layers =
            installed_content::ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let catalog = Catalog::read(&layers).unwrap();
        assert!(catalog.warnings.is_empty(), "{:?}", catalog.warnings);
        assert!(catalog.projects.contains_key("mod_project"));
        assert!(catalog.specializations.contains("mod_kind"));
        assert_eq!(
            catalog.projects["base_project"].specialization.as_deref(),
            Some("base_kind")
        );
        assert_eq!(
            catalog.projects["base_project"].name.as_deref(),
            Some("mod_name")
        );
        let loc = installed_content::localisation::read(&layers, "english").unwrap();
        assert_eq!(
            catalogue::localised(&loc, "mod_name", View::Plain).unwrap(),
            "Renamed by mod"
        );
        assert_eq!(
            catalogue::localised_with(
                &loc,
                ITERATIONS_KEY,
                &[("NUMBER", Argument::Int32(4))],
                View::Plain
            )
            .unwrap(),
            "Mod count +4"
        );
        let sprites = installed_content::gfx::read(&layers).unwrap();
        let key = icon_key(
            &catalog.projects["base_project"],
            "base_project",
            Some("opaque tag"),
            |key| {
                sprites
                    .find(key)
                    .map(|sprite| sprite.is_some())
                    .map_err(|e| e.to_string())
            },
        )
        .unwrap();
        assert_eq!(
            sprites.find(&key).unwrap().unwrap().texture.as_deref(),
            Some("gfx/mod_project.dds")
        );
        let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
        let mut content = Content {
            catalogue: Arc::clone(&catalogue),
            politics: Arc::new(PoliticsContent::load(catalogue).unwrap()),
            definitions: catalog,
            icons: BTreeMap::new(),
            scoped: Vec::new(),
            refusals: Mutex::default(),
        };
        assert_eq!(
            content.name_for("mod_name", None).as_deref(),
            Some("Renamed by mod")
        );
        assert_eq!(content.name_for("missing_title", None), None);
        assert_eq!(content.specialization_name("missing_kind", None), None);
        let mut sample = frame(100, true, 1);
        *sample.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                replay::rows::LexicalTokenRow {
                    token: 8,
                    literal: b"base_project".to_vec(),
                },
            ]));
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", Some(&mut content));
        sample.hour = 101;
        builder.observe(&sample, "mod tag", Some(&mut content));
        assert_eq!(builder.content_ids.len(), 1);
        assert_eq!(builder.person_cache.len(), 1);
        assert_eq!(builder.presentations[0].name, "Renamed by mod");
        assert_eq!(builder.iteration_texts.len(), 1);
        assert_eq!(builder.iteration_texts[&0].as_deref(), Some("Mod count 0"));
        // A refused override must never expose the earlier authored icon as
        // though it were still effective. The product uses a visible fallback.
        std::fs::write(
            over.join("common/special_projects/projects/zz_invalid.txt"),
            b"base_project={icon=\"\x80\"}",
        )
        .unwrap();
        let refused_layers =
            installed_content::ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
        let refused = Catalog::read(&refused_layers).unwrap();
        assert!(
            refused.projects["base_project"]
                .refusal
                .as_deref()
                .unwrap()
                .contains("refused 1 icon: special project string: invalid utf-8 sequence"),
            "{refused:?}"
        );
        assert!(
            refused
                .warnings
                .iter()
                .any(|warning| warning.contains("zz_invalid.txt") && warning.contains("refused 1"))
        );
    }
    #[test]
    fn native_progress_completion_and_history_count_are_distinct_from_saved_flags() {
        let mut project = ProjectRow {
            completed: 1,
            state: replay::rows::ProjectState {
                not_completed: 1,
                current_offset: 0x18,
                prototype_progress: 37,
                phase_progress: 99999,
                phase_target: 1,
                iterations: 99,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!completed(&project));
        assert_eq!(progress(&project), 37);
        project.completed = 0;
        project.state.not_completed = 0;
        project.state.current_offset = 0xe8;
        assert!(completed(&project));
        assert_eq!(progress(&project), 100);
        let unrelated = ProjectHistoryRow {
            date: 20,
            ..Default::default()
        };
        assert_eq!(
            completion_date(&[&unrelated], 30, 0),
            ProjectDate {
                hour: 30,
                exact: false
            }
        );
        let event = ProjectHistoryRow {
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date: 25,
            ..Default::default()
        };
        assert_eq!(
            completion_date(&[&unrelated, &event], 30, 0),
            ProjectDate {
                hour: 25,
                exact: true
            }
        );
        assert!(!completion_date(&[&event], 24, 0).exact);
    }
    #[test]
    fn icon_lookup_does_not_mask_a_missing_authored_icon() {
        let mut project = installed_content::special_projects::Project::default();
        assert_eq!(
            icon_key(&project, "mod_project", Some("tag with spaces"), |key| Ok(
                key == "GFX_tag with spaces_mod_project"
            ))
            .unwrap(),
            "GFX_tag with spaces_mod_project"
        );
        assert_eq!(
            icon_key(&project, "mod_project", Some("X"), |key| Ok(
                key == "GFX_mod_project"
            ))
            .unwrap(),
            "GFX_mod_project"
        );
        assert_eq!(
            icon_key(&project, "mod_project", None, |key| {
                assert_eq!(key, "GFX_mod_project");
                Ok(true)
            })
            .unwrap(),
            "GFX_mod_project"
        );
        project.icon = Some("GFX_missing_mod_icon".into());
        assert_eq!(
            icon_key(&project, "mod_project", Some("X"), |_| Ok(true)).unwrap(),
            "GFX_missing_mod_icon"
        );
    }
}
