//! Special projects over one country's branch. Rows are admitted at slot_read,
//! which is every hour of the sliced projects family (cadence.rs: a country is
//! read when a hooked writer marks it or after the daily pass, and carried
//! exactly otherwise), carried within consecutive recorded hours, never over a gap.
//! Character names and scoped project names are resolved at each frame's hour.

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue, View},
};
use dashboard_base::{
    products::CountryParams,
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::character_portraits::{RecordedPortrait, portrait_png, recorded_portrait},
    products::country_groups::HourSpan,
    products::country_history::{self, CountryIdentityRun},
    products::politics_content::{PoliticsContent, politics_content, recorded_politics_content},
};
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::link_slot;
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::tagged_country;
use dashboard_text::identity::character_script_token;
#[cfg(any(test, feature = "test-support"))]
use dashboard_text::metadata::{slot_read, tagged_countries, validate_campaign};
use installed_content_core::special_projects::Catalog;
use installed_content_localisation::localisation::Argument;
use replay_data::{
    rows::CountryLinkRow, rows::ProjectHistoryRow, rows::ProjectRow, rows::TableId, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::ViewIdentity};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::collections::VecDeque;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct SpecialProjects;
mod details;
pub mod emit;
pub mod facts;
mod history;
pub use details::*;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
mod served;

mod completions;
#[cfg(any(test, feature = "test-support"))]
pub use completions::completion_events;

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
    /// Current cards at this hour, including before history preparation reaches it.
    #[serde(default)]
    pub hour: Option<i32>,
}

impl SpecialProjectsParams {
    fn country(&self) -> CountryParams {
        CountryParams {
            branch_id: self.branch_id.clone(),
            tag: self.tag.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDate {
    pub hour: i32,
    /// False means the event occurred by this observation, not at this hour.
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPerson {
    /// None when the recorded reference cannot establish an assignment.
    pub assigned: Option<bool>,
    pub id: Option<u32>,
    pub name: Option<String>,
    /// Drawn when a reply is served, from the request's selection ([`Self::portrait_choice`]).
    pub portrait: Option<String>,
    pub missing: Option<String>,
    /// The dated portrait the recording selected, kept by preparation and drawn when a reply
    /// is served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub portrait_choice: Option<RecordedPortrait>,
}

/// The recorded operands of a project's icon: its definition's authored icon, which wins even
/// when it is missing, else its token's country-prefixed sprite when the presentation declares
/// it, else the generic one, else the placeholder. The game chooses against the whole content
/// stack, visual mods included, so the choice is made when a reply is served.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIconChoice {
    pub authored: Option<String>,
    pub token: String,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPresentation {
    pub name: String,
    pub specialization: Option<String>,
    pub specialization_name: Option<String>,
    /// Drawn when a reply is served, from the request's selection ([`Self::icon_choice`]).
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    /// What the icon is chosen from, kept by preparation and taken when a reply is served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub icon_choice: Option<ProjectIconChoice>,
    pub scientist: u32,
    /// Completed projects lose their program; this dates the last assignment.
    pub scientist_as_of: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProjectState {
    Running,
    Paused,
    Cancelled,
    Completed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEpisode {
    pub id: String,
    pub start: ProjectDate,
    pub runs: Vec<SpecialProjectRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectsReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub details: Option<SpecialProjectDetails>,
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
    /// Running cards read at the requested hour. Episode starts and history
    /// remain pending; their starts are only bounded by this observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub current: Option<Box<SpecialProjectsReply>>,
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

#[cfg(test)]
fn completion_date(
    history: &[&ProjectHistoryRow],
    observed: i32,
    campaign_start: i32,
) -> ProjectDate {
    completion_date_inputs(
        history.iter().map(|row| history_operand(row)),
        observed,
        campaign_start,
    )
}
fn history_operand(row: &ProjectHistoryRow) -> (u32, bool, i32) {
    (
        row.ordinal,
        row.prototype_reward_handle == COMPLETION_MARKER && row.option_handle == COMPLETION_MARKER,
        row.date,
    )
}
fn completion_date_inputs(
    history: impl Iterator<Item = (u32, bool, i32)>,
    observed: i32,
    campaign_start: i32,
) -> ProjectDate {
    history
        .filter(|(_, completion, date)| *completion && *date <= observed && *date >= campaign_start)
        .max_by_key(|(ordinal, _, _)| *ordinal)
        .map_or(
            ProjectDate {
                hour: observed,
                exact: false,
            },
            |(_, _, hour)| ProjectDate { hour, exact: true },
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
    authored: Option<&str>,
    token: &str,
    tag: Option<&str>,
    has: impl Fn(&str) -> Result<bool, String>,
) -> Result<String, String> {
    if let Some(icon) = authored.filter(|icon| !icon.is_empty()) {
        return Ok(icon.to_owned());
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

/// A project's icon: its sprite's first frame, newly shown, drawn as the game
/// draws the sprite.
fn project_icon(catalogue: &Catalogue, key: &str) -> Result<String, String> {
    let sprite = catalogue
        .sprites
        .find(key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Content has no sprite {key}."))?;
    let decode = *catalogue.gui_decode.as_ref().map_err(Clone::clone)?;
    catalogue::sprite_frame_png(&catalogue.content.layers, sprite, decode, 0, 96)
        .map_err(|e| e.to_string())
}

/// The project icons and scientists' portraits drawn for one selection, by their recorded
/// operands; one per kind of catalogue ([`catalogue::ByCatalogue`]).
#[derive(Default)]
struct ProjectArt {
    icons: Option<catalogue::Redrawn<ProjectIconChoice, String>>,
    portraits: Option<catalogue::Redrawn<RecordedPortrait, String>>,
}

/// What a reply's project art is drawn from: the politics content of the request's admitted
/// catalogue and of the recording's own.
pub struct ArtSource<'a> {
    source: &'a Source,
    content: Option<&'a dashboard_core::config::ContentOptions>,
    loaded: std::cell::OnceCell<Result<(Arc<PoliticsContent>, Arc<PoliticsContent>), String>>,
}

impl<'a> ArtSource<'a> {
    /// For a request admitted under `content`, loaded only when a reply holds art to draw.
    pub fn new(
        source: &'a Source,
        content: Option<&'a dashboard_core::config::ContentOptions>,
    ) -> Self {
        Self {
            source,
            content,
            loaded: std::cell::OnceCell::new(),
        }
    }

    /// The request's admitted politics content and the recording's own, or why there is none
    /// to draw from (no content, or content that does not load); only cancellation fails.
    fn contents(&self) -> io::Result<&Result<(Arc<PoliticsContent>, Arc<PoliticsContent>), String>> {
        if let Some(loaded) = self.loaded.get() {
            return Ok(loaded);
        }
        let loaded = match self.content {
            None => Err("Content is unavailable.".to_owned()),
            Some(options) => match politics_content(self.source, options).and_then(|admitted| {
                Ok((admitted, recorded_politics_content(self.source, options)?))
            }) {
                Ok(contents) => Ok(contents),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => Err(format!("Project artwork is unavailable: {error}")),
            },
        };
        Ok(self.loaded.get_or_init(|| loaded))
    }
}

/// Draws the art preparation kept as recorded operands ([`ProjectPresentation::icon_choice`],
/// [`ProjectPerson::portrait_choice`], [`ProjectCompletion::icon_choice`]) as the request shows
/// it: each project's icon chosen among its candidates and drawn, and each scientist's dated
/// portrait drawn, against the request's admitted catalogue ([`catalogue::present`]). When the
/// selection cannot draw one, the recording's own choice is drawn and named on every reply that
/// shows it. Where there is no content to draw from, or it does not load, each is unavailable
/// with why and the rest of the reply stands; content is loaded only when there is art to draw.
/// Returns each reason a piece was left undrawn, or a person preparation could not resolve,
/// with how many it left out, for the reply's counted warnings.
pub fn present_art<'a>(
    source: &Source,
    art: &ArtSource,
    icons: impl IntoIterator<Item = (&'a mut Option<ProjectIconChoice>, &'a mut Option<String>, &'a mut Option<String>)>,
    people: impl IntoIterator<Item = &'a mut ProjectPerson>,
) -> io::Result<BTreeMap<String, u64>> {
    let mut failed = BTreeMap::<String, u64>::new();
    let icons: Vec<_> = icons
        .into_iter()
        .filter(|(choice, _, _)| choice.is_some())
        .collect();
    let people: Vec<&mut ProjectPerson> = people.into_iter().collect();
    // A person preparation could not resolve (a recorded reference whose character, program
    // or tables the recording lacks) is a failure counted with the rest; no scientist
    // assigned is a normal absence.
    for person in &people {
        if person.portrait_choice.is_none()
            && person.assigned != Some(false)
            && let Some(reason) = &person.missing
        {
            *failed.entry(reason.clone()).or_default() += 1;
        }
    }
    let people: Vec<_> = people
        .into_iter()
        .filter(|person| person.portrait_choice.is_some())
        .collect();
    if icons.is_empty() && people.is_empty() {
        return Ok(failed);
    }
    let (admitted_content, recorded_content) = match art.contents()? {
        Ok(contents) => contents,
        Err(reason) => {
            let count = icons.len() + people.len();
            for (choice, _, missing) in icons {
                *choice = None;
                *missing = Some(reason.clone());
            }
            for person in people {
                person.portrait_choice = None;
                person.missing = Some(reason.clone());
            }
            failed.insert(reason.clone(), count as u64);
            return Ok(failed);
        }
    };
    let (admitted, recorded) = (&admitted_content.catalogue, &recorded_content.catalogue);
    let state = source.state::<catalogue::ByCatalogue<ProjectArt>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let ProjectArt {
        icons: drawn_icons,
        portraits,
    } = state.of(admitted);
    let drawn_icons = catalogue::Redrawn::bind(drawn_icons, admitted, recorded);
    for (choice, icon, missing) in icons {
        let Some(choice) = choice.take() else {
            continue;
        };
        let shown = drawn_icons.get(
            choice.clone(),
            || format!("project {} icon", choice.token),
            |cat| {
                let key = icon_key(
                    choice.authored.as_deref(),
                    &choice.token,
                    choice.tag.as_deref(),
                    |key| {
                        cat.sprites
                            .find(key)
                            .map(|sprite| sprite.is_some())
                            .map_err(|e| format!("Sprite {key}: {e}"))
                    },
                )?;
                project_icon(cat, &key)
            },
        );
        *icon = shown.as_ref().ok().cloned();
        *missing = shown.as_ref().err().cloned();
        if let Err(reason) = shown {
            *failed.entry(reason.clone()).or_default() += 1;
        }
    }
    let portraits = catalogue::Redrawn::bind(portraits, admitted, recorded);
    for person in people {
        let Some(portrait) = person.portrait_choice.take() else {
            continue;
        };
        let shown = portraits.get(
            portrait.clone(),
            || "scientist portrait".into(),
            |cat| {
                let politics = if std::ptr::eq(cat, &**admitted) {
                    admitted_content
                } else {
                    recorded_content
                };
                portrait_png(politics, &portrait)
            },
        );
        person.portrait = shown.as_ref().ok().cloned();
        person.missing = shown.as_ref().err().cloned();
        if let Err(reason) = shown {
            *failed.entry(reason.clone()).or_default() += 1;
        }
    }
    Ok(failed)
}

/// Counted reply warnings of [`present_art`]'s failures.
pub fn art_warnings(failed: BTreeMap<String, u64>) -> impl Iterator<Item = String> {
    failed
        .into_iter()
        .map(|(reason, count)| format!("{reason} (count: {count})"))
}

/// [`present_art`] over completed projects' icons.
pub fn present_completions<'a>(
    source: &Source,
    art: &ArtSource,
    completions: impl IntoIterator<Item = &'a mut ProjectCompletion>,
) -> io::Result<BTreeMap<String, u64>> {
    present_art(
        source,
        art,
        completions
            .into_iter()
            .map(|event| (&mut event.icon_choice, &mut event.icon, &mut event.icon_missing)),
        std::iter::empty(),
    )
}

/// [`present_art`] over a reply: its presentations and people, its at-hour details' scientists
/// and assignments, and the reply it is current to, each reply's failures counted among its
/// warnings.
pub fn present_reply(
    source: &Source,
    art: &ArtSource,
    reply: &mut SpecialProjectsReply,
) -> io::Result<()> {
    let mut people: Vec<&mut ProjectPerson> = reply.people.iter_mut().collect();
    if let Some(details) = &mut reply.details {
        people.extend(
            details
                .scientists
                .iter_mut()
                .flatten()
                .map(|scientist| &mut scientist.person),
        );
        people.extend(
            details
                .programs
                .iter_mut()
                .flatten()
                .flat_map(|program| program.assignments.iter_mut().flatten())
                .map(|assignment| &mut assignment.person),
        );
    }
    let failed = present_art(
        source,
        art,
        reply
            .presentations
            .iter_mut()
            .map(|info| (&mut info.icon_choice, &mut info.icon, &mut info.icon_missing)),
        people,
    )?;
    reply.warnings.extend(art_warnings(failed));
    if let Some(current) = &mut reply.current {
        present_reply(source, art, current)?;
    }
    Ok(())
}

struct Content {
    catalogue: Arc<Catalogue>,
    politics: Arc<PoliticsContent>,
    definitions: Catalog,
    icons: BTreeMap<String, catalogue::Drawn<Result<String, String>>>,
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
            .or_insert_with(|| catalogue::Drawn::new(|| project_icon(&self.catalogue, key)))
            .get()
            .clone()
    }

    /// `definition`'s name for `tag`; a key that needs scope reads `scoped`.
    fn name(
        &self,
        definition: &installed_content_core::special_projects::Project,
        token: &str,
        tag: &str,
        scoped: &dyn Fn(&Content) -> Option<String>,
    ) -> Option<String> {
        // Use the authored narrative name when nonempty, otherwise the definition's
        // default name; then TAG_key if defined, then scope expansion in the owner's
        // country. The default key is the template token's string.
        let selected = country_key(&self.catalogue, tag, project_name_key(definition, token));
        if needs_scope(&self.catalogue.localisation, &selected) {
            return scoped(self);
        }
        self.name_for(&selected, None)
    }

    /// The scoped name of `tag`'s `definition` at `frame`'s hour, as the name scan read it.
    fn scoped_at(
        &self,
        frame: &DecodedFrame,
        tag: &str,
        slot: u32,
        definition: u32,
    ) -> Option<String> {
        self.scoped
            .iter()
            .find(|(branch, _)| *branch == frame.branch)
            .and_then(|(_, names)| names.at(tag, slot, definition, frame.hour))
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
            &dashboard_text::scripted_text::ScriptedText::new(
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
    definition: &'a installed_content_core::special_projects::Project,
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
pub fn check_content(
    politics: Arc<PoliticsContent>,
) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    use dashboard_core::findings::{Finding, Kind};
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
        let key = icon_key(definition.icon.as_deref(), &token, None, |key| {
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

/// Episode inputs projected once from recorded rows, independent of selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectFacts {
    local_id: u32,
    definition_id: u32,
    visible: bool,
    state: ProjectState,
    progress: i32,
    completion: Option<ProjectDate>,
    iterations: Option<i32>,
    warning: Option<String>,
}
impl ProjectFacts {
    fn project(
        project: &ProjectRow,
        history: Option<&[&ProjectHistoryRow]>,
        hour: i32,
        start: i32,
    ) -> Self {
        let state = project_state(project);
        let mut facts = Self {
            local_id: project.local_id,
            definition_id: project.definition_id,
            visible: visible(project, false),
            state,
            progress: progress(project),
            completion: None,
            iterations: None,
            warning: (state == ProjectState::Unknown).then(|| {
                format!(
                    "Project {} is in unsupported native state {:#x}",
                    project.local_id, project.state.current_offset
                )
            }),
        };
        let history = history.map(|rows| {
            rows.iter()
                .map(|row| history_operand(row))
                .collect::<Vec<_>>()
        });
        facts.observe_history(history.as_deref(), hour, start);
        facts
    }
    fn observe_history(&mut self, history: Option<&[(u32, bool, i32)]>, hour: i32, start: i32) {
        let done = self.state == ProjectState::Completed;
        self.completion = done
            .then(|| {
                completion_date_inputs(history.unwrap_or_default().iter().copied(), hour, start)
            })
            .filter(|date| date.exact);
        self.iterations = history.map(|rows| rows.len() as i32 - i32::from(done));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Look {
    needs_scope: bool,
    program_reference: u64,
    person: Arc<ProjectPerson>,
    info: Arc<ProjectPresentation>,
    warnings: BTreeMap<String, usize>,
    refusals: catalogue::Refusals,
    iteration_text: Option<String>,
    iteration_refusals: catalogue::Refusals,
    /// What resolving the person refused.
    #[serde(skip)]
    person_refusals: catalogue::Refusals,
}
fn resolve_person(
    frame: &DecodedFrame,
    project: &ProjectRow,
    content: Option<&Content>,
) -> ProjectPerson {
    // Scientist assignment uses the full character reference.
    // The low reference type is 73; program reference type is 83.
    let mut assigned = None;

    (|| {
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
            replay_data::rows::PORTRAIT_LARGE,
        );
        // Drawn when a reply is served ([`ProjectPerson::portrait_choice`]).
        Ok(ProjectPerson {
            assigned,
            id: Some(id),
            name,
            portrait: None,
            missing: content.is_none().then(|| "Content is unavailable.".into()),
            portrait_choice: content.is_some().then_some(portrait),
        })
    })()
    .unwrap_or_else(|reason| ProjectPerson {
        assigned,
        id: None,
        name: None,
        portrait: None,
        missing: Some(reason),
        portrait_choice: None,
    })
}

fn resolve_definition(
    frame: &DecodedFrame,
    project: &ProjectRow,
    tag: &str,
    slot: u32,
    content: Option<&mut Content>,
) -> DefinitionLook {
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
    let definition_id = project.definition_id;
    resolve_token(
        token,
        definition_id,
        tag,
        &|content: &Content| content.scoped_at(frame, tag, slot, definition_id),
        content,
    )
}

/// [`resolve_definition`] of the definition the recording spells `token`, its scoped name read
/// by `scoped`.
fn resolve_token(
    token: Option<String>,
    definition_id: u32,
    tag: &str,
    scoped_name: &dyn Fn(&Content) -> Option<String>,
    mut content: Option<&mut Content>,
) -> DefinitionLook {
    take_refusals(content.as_deref());
    let mut warnings = BTreeMap::new();
    let mut warn = |reason: String| {
        *warnings.entry(reason).or_default() += 1;
    };
    let mut scoped = None;
    let scientist = 0;
    let scientist_as_of = None;
    let token = token.unwrap_or_else(|| {
        warn(format!("Definition token {definition_id} is not recorded"));
        format!("Project definition {definition_id}")
    });
    let mut info = ProjectPresentation {
        name: "Project name unavailable".into(),
        specialization: None,
        specialization_name: None,
        icon: None,
        icon_missing: Some("Content is unavailable.".into()),
        icon_choice: None,
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
            warn(reason.clone());
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
                scoped = Some((definition.clone(), token.clone()));
            }
            info.name = content
                .name(&definition, &token, tag, scoped_name)
                .unwrap_or_else(|| {
                    warn(format!("Project name {token} could not be resolved"));
                    "Project name unavailable".into()
                });
            info.specialization = definition.specialization.clone();
            // Use the specialization token, not blueprint_image
            // or a vanilla type enumeration.
            if let Some(kind) = &definition.specialization {
                if content.definitions.specializations.contains(kind) {
                    info.specialization_name = content.specialization_name(kind, Some(tag));
                } else {
                    warn(format!(
                        "Project {token}: specialization {kind} is absent from content"
                    ));
                }
            }
            // Chosen and drawn when a reply is served ([`ProjectPresentation::icon_choice`]).
            info.icon_missing = None;
            info.icon_choice = Some(ProjectIconChoice {
                authored: definition.icon.clone().filter(|icon| !icon.is_empty()),
                token: token.clone(),
                tag: Some(tag.to_owned()),
            });
        } else {
            warn(format!("Project definition {token} is absent from content"));
            info.icon_missing = Some(format!("Definition {token} is absent from content."));
        }
    }
    if let Some((_, token)) = &scoped {
        warnings.remove(&format!("Project name {token} could not be resolved"));
    }
    DefinitionLook {
        info: Arc::new(info),
        warnings,
        refusals: take_refusals(content.as_deref()),
        scoped,
    }
}

struct DefinitionLook {
    info: Arc<ProjectPresentation>,
    warnings: BTreeMap<String, usize>,
    refusals: catalogue::Refusals,
    scoped: Option<(installed_content_core::special_projects::Project, String)>,
}
#[derive(Default)]
struct Naming {
    definitions: BTreeMap<(String, u32), DefinitionLook>,
    people: BTreeMap<u64, (Arc<ProjectPerson>, catalogue::Refusals)>,
    iterations: BTreeMap<i32, (Option<String>, catalogue::Refusals)>,
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(dead_code, reason = "read only by the recording oracle in tests")
    )]
    views: Option<[ViewIdentity; 4]>,
}

fn take_refusals(content: Option<&Content>) -> catalogue::Refusals {
    content.map_or_else(Default::default, |content| {
        std::mem::take(
            &mut *content
                .refusals
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    })
}
impl Naming {
    #[cfg(any(test, feature = "test-support"))]
    fn observe_views(&mut self, frame: &DecodedFrame) {
        let tables = [
            TableId::ProjectPrograms,
            TableId::Characters,
            TableId::CharacterPortraits,
            TableId::LexicalTokens,
        ];
        if self.views.as_ref().is_none_or(|views| {
            tables
                .iter()
                .zip(views)
                .any(|(table, view)| !view.is(&frame.tables[table.index()]))
        }) {
            self.people.clear();
            self.views = Some(tables.map(|table| frame.tables[table.index()].identity()));
        }
    }
    fn look(
        &mut self,
        frame: &DecodedFrame,
        tag: &str,
        slot: u32,
        row: &ProjectRow,
        count: Option<i32>,
        mut content: Option<&mut Content>,
    ) -> Look {
        let key = (tag.to_owned(), row.definition_id);
        self.people.entry(row.program_reference).or_insert_with(|| {
            take_refusals(content.as_deref());
            let person = resolve_person(frame, row, content.as_deref());
            (Arc::new(person), take_refusals(content.as_deref()))
        });
        if !self.definitions.contains_key(&key) {
            let definition = resolve_definition(frame, row, tag, slot, content.as_deref_mut());
            self.definitions.insert(key.clone(), definition);
        }
        let definition = self.definitions.get_mut(&key).expect("resolved definition");
        let mut warnings = definition.warnings.clone();
        if let Some((authored, token)) = &definition.scoped {
            let scoped = |content: &Content| content.scoped_at(frame, tag, slot, row.definition_id);
            let name = content
                .as_ref()
                .and_then(|content| content.name(authored, token, tag, &scoped))
                .unwrap_or_else(|| {
                    *warnings
                        .entry(format!("Project name {token} could not be resolved"))
                        .or_default() += 1;
                    "Project name unavailable".into()
                });
            if definition.info.name != name {
                Arc::make_mut(&mut definition.info).name = name;
            }
        }
        let (person, person_refusals) = &self.people[&row.program_reference];
        let mut refusals = definition.refusals.clone();
        refusals.merge(person_refusals);
        let (iteration_text, iteration_refusals) = count.map_or_else(
            || (None, Default::default()),
            |count| {
                self.iterations
                    .entry(count)
                    .or_insert_with(|| {
                        take_refusals(content.as_deref());
                        let text = content
                            .as_ref()
                            .and_then(|content| content.iterations(Some(count)));
                        (text, take_refusals(content.as_deref()))
                    })
                    .clone()
            },
        );
        Look {
            needs_scope: definition.scoped.is_some(),
            program_reference: row.program_reference,
            person: Arc::clone(person),
            info: Arc::clone(&definition.info),
            warnings,
            refusals,
            iteration_text,
            iteration_refusals,
            person_refusals: person_refusals.clone(),
        }
    }
}

/// A completed project's presentation for `tag`: its definition, which the recording spells
/// `token`, resolved as [`Naming::look`] resolves it, its scoped name read by `scoped`. Its
/// presentation, the warnings resolving it counted and what it refused.
fn completion_look(
    token: Option<String>,
    definition_id: u32,
    tag: &str,
    scoped: &dyn Fn(&Content) -> Option<String>,
    mut content: Option<&mut Content>,
) -> (Arc<ProjectPresentation>, BTreeMap<String, usize>, catalogue::Refusals) {
    let mut definition = resolve_token(token, definition_id, tag, scoped, content.as_deref_mut());
    let mut warnings = definition.warnings.clone();
    if let Some((authored, token)) = &definition.scoped {
        let name = content
            .as_ref()
            .and_then(|content| content.name(authored, token, tag, scoped))
            .unwrap_or_else(|| {
                *warnings
                    .entry(format!("Project name {token} could not be resolved"))
                    .or_default() += 1;
                "Project name unavailable".into()
            });
        if definition.info.name != name {
            Arc::make_mut(&mut definition.info).name = name;
        }
    }
    (definition.info, warnings, definition.refusals)
}

#[derive(Default)]
struct Builder {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(dead_code, reason = "read only by the recording oracle in tests")
    )]
    format: replay_data::format::FormatVersion,
    episodes: Vec<ProjectEpisode>,
    /// Matching episodes survive unrecorded hours, but their runs do not.
    open: BTreeMap<(u32, u32), usize>,
    identity: Option<(u32, u32)>,
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(dead_code, reason = "read only by the recording oracle in tests")
    )]
    campaign_start: Option<i32>,
    content_ids: BTreeMap<(u32, String), u32>,
    naming: Naming,
    person_cache: BTreeMap<u64, (Arc<ProjectPerson>, u32)>,
    #[cfg(any(test, feature = "test-support"))]
    lexical_view: Option<ViewIdentity>,
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
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(dead_code, reason = "read only by the recording oracle in tests")
    )]
    completions: BTreeMap<usize, CompletionObservation>,
}

#[derive(Clone)]
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(dead_code, reason = "observed only by the recording oracle in tests")
)]
struct CompletionObservation {
    project: ProjectRow,
    identity: (u32, u32),
    date: ProjectDate,
    observed_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCompletion {
    pub tag: String,
    pub date: ProjectDate,
    pub observed_hour: i32,
    pub definition_id: u32,
    pub item_token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    /// What the icon is chosen from ([`ProjectPresentation::icon_choice`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_choice: Option<ProjectIconChoice>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct ProjectCompletions {
    pub events: Vec<ProjectCompletion>,
    pub warnings: Vec<String>,
    pub unavailable: Option<String>,
    pub refusals: Vec<(String, usize)>,
}

#[cfg(any(test, feature = "test-support"))]
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

    fn intern_person(&mut self, person: ProjectPerson) -> u32 {
        if let Some(id) = self.person_ids.get(&person) {
            return *id;
        }
        let id = self.people.len() as u32;
        self.person_ids.insert(person.clone(), id);
        self.people.push(person);
        id
    }

    #[cfg(any(test, feature = "test-support"))]
    fn presentation(
        &mut self,
        frame: &DecodedFrame,
        project: &ProjectRow,
        tag: &str,
        slot: u32,
        previous: Option<&SpecialProjectRun>,
        mut content: Option<&mut Content>,
    ) -> u32 {
        let mut refusals = take_refusals(content.as_deref());
        let look = self
            .naming
            .look(frame, tag, slot, project, None, content.as_deref_mut());
        let id = self.present_look(
            &look,
            project.definition_id,
            tag,
            completed(project),
            frame.hour,
            previous,
            &mut refusals,
        );
        if let Some(content) = content {
            content
                .refusals
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .merge(&refusals);
        }
        id
    }

    #[allow(clippy::too_many_arguments)]
    fn present_look(
        &mut self,
        look: &Look,
        definition: u32,
        tag: &str,
        completed: bool,
        hour: i32,
        previous: Option<&SpecialProjectRun>,
        refusals: &mut catalogue::Refusals,
    ) -> u32 {
        let person = match self.person_cache.get(&look.program_reference) {
            Some((seen, id)) if Arc::ptr_eq(seen, &look.person) => *id,
            _ => {
                let id = self.intern_person((*look.person).clone());
                self.person_cache
                    .insert(look.program_reference, (Arc::clone(&look.person), id));
                id
            }
        };
        let (scientist, scientist_as_of) =
            self.scientist_attribution(person, completed, hour, previous);
        let key = (definition, tag.to_owned());
        if !self.content_ids.contains_key(&key) {
            self.needs_scope |= look.needs_scope;
            for (reason, count) in &look.warnings {
                *self.warnings.entry(reason.clone()).or_default() += count;
            }
        }
        refusals.merge(&look.refusals);
        if let Some(&id) = self.content_ids.get(&key) {
            let old = &self.presentations[id as usize];
            if old.scientist == scientist
                && old.scientist_as_of == scientist_as_of
                && old.name == look.info.name
            {
                return id;
            }
        }
        let mut info = (*look.info).clone();
        info.scientist = scientist;
        info.scientist_as_of = scientist_as_of;
        let id = self.intern_presentation(info);
        self.content_ids.insert(key, id);
        id
    }

    fn scientist_attribution(
        &self,
        mut scientist: u32,
        completed: bool,
        hour: i32,
        previous: Option<&SpecialProjectRun>,
    ) -> (u32, Option<i32>) {
        let mut as_of = None;
        if completed
            && let Some(previous) =
                previous.filter(|run| run.last_hour.checked_add(1) == Some(hour))
        {
            let old = &self.presentations[previous.presentation as usize];
            if self.people[old.scientist as usize].id.is_some() {
                scientist = old.scientist;
                as_of = Some(old.scientist_as_of.unwrap_or(previous.last_hour));
            }
        }
        (scientist, as_of)
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
    #[cfg(any(test, feature = "test-support"))]
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
            self.naming.definitions.clear();
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
    #[cfg(any(test, feature = "test-support"))]
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
        self.naming.observe_views(frame);
        let Some(projects) = projects else {
            self.admitted = false;
            return;
        };
        let read = slot_read(self.format, &frame.metrics, TableId::Projects, hour);
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
        let facts: Vec<_> = projects
            .iter()
            .map(|project| {
                let history = histories
                    .get(&project.local_id)
                    .map_or(&[][..], Vec::as_slice);
                ProjectFacts::project(
                    project,
                    history_available.then_some(history),
                    hour,
                    self.campaign_start.unwrap_or(hour),
                )
            })
            .collect();
        self.observe_projects(
            hour,
            &facts,
            |builder, ordinal, index, fact, previous, completion| {
                let project = projects[ordinal];
                let presentation = if builder.compact {
                    if let Some(date) = completion {
                        builder
                            .completions
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
                    0
                } else {
                    builder.presentation(
                        frame,
                        project,
                        tag,
                        slot,
                        previous,
                        content.as_deref_mut(),
                    )
                };
                if let Some(count) = fact.iterations
                    && !builder.compact
                {
                    builder.iteration_texts.entry(count).or_insert_with(|| {
                        content
                            .as_ref()
                            .and_then(|content| content.iterations(Some(count)))
                    });
                }
                presentation
            },
        );
    }

    /// Shared selected-episode step. Its caller has already admitted the read.
    fn observe_projects(
        &mut self,
        hour: i32,
        projects: &[ProjectFacts],
        mut presentation: impl FnMut(
            &mut Self,
            usize,
            usize,
            &ProjectFacts,
            Option<&SpecialProjectRun>,
            Option<&ProjectDate>,
        ) -> u32,
    ) {
        let mut next = BTreeMap::new();
        for (ordinal, project) in projects.iter().enumerate() {
            let state = project.state;
            let done = state == ProjectState::Completed;
            let key = (project.local_id, project.definition_id);
            if !project.visible && !self.open.contains_key(&key) {
                continue;
            }
            let count = project.iterations;
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
                let date = project
                    .completion
                    .clone()
                    .unwrap_or(ProjectDate { hour, exact: false });
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
            let presentation = presentation(
                self,
                ordinal,
                index,
                project,
                previous.as_ref(),
                completion.as_ref(),
            );
            if let Some(count) = count {
                self.last_iterations.insert(index, count);
            }
            if let Some(reason) = &project.warning {
                self.warn(reason.clone());
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
                progress: project.progress,
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
fn needs_scope(loc: &installed_content_localisation::localisation::Localisation, key: &str) -> bool {
    loc.localise(key.as_bytes())
        .is_ok_and(|text| text.contains('['))
}

#[cfg(any(test, feature = "test-support"))]
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
        format: scan.format,
        campaign_start: scan.campaign_start,
        ..Default::default()
    };
    for span in &scan.spans {
        dashboard_text::metadata::check_cancel(cancelled)?;
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
            dashboard_text::metadata::check_cancel(cancelled)?;
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
    let warnings = content.as_ref().map_or_else(Vec::new, |content| {
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
    Ok(build_reply(
        result,
        params,
        content
            .as_ref()
            .is_some_and(|content| content.definitions.projects.is_empty()),
        warnings,
    ))
}

fn build_reply(
    result: Builder,
    params: &CountryParams,
    definitions_empty: bool,
    mut warnings: Vec<String>,
) -> (SpecialProjectsReply, bool) {
    warnings.extend(
        result
            .warnings
            .into_iter()
            .map(|(reason, count)| format!("{reason} ({count} observations).")),
    );
    let unavailable = if result.available.is_empty() {
        Some("Projects was unavailable, incomplete, or never read for this country.".into())
    } else if definitions_empty {
        Some("The recorded content has no readable special-project definitions.".into())
    } else {
        None
    };
    let needs_scope = result.needs_scope;
    (
        SpecialProjectsReply {
            details: None,
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
            current: None,
        },
        needs_scope,
    )
}

fn append_identities(
    reply: &mut SpecialProjectsReply,
    identities: &[&CountryIdentityRun],
) -> io::Result<()> {
    // The empty [] is already measured in the complete reply. Borrow metadata
    // until its replacement fits, before copying any native identity runs.
    let error = || io::Error::other("Special projects reply exceeds its transport reply budget");
    let base =
        dashboard_core::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(error)?;
    let added = dashboard_core::transport::json_len(&identities, MAX_PRODUCT_REPLY_BYTES)
        .ok_or_else(error)?;
    if base.saturating_sub(2).saturating_add(added) > MAX_PRODUCT_REPLY_BYTES {
        return Err(error());
    }
    reply.identities = identities
        .iter()
        .map(|identity| (**identity).clone())
        .collect();
    dashboard_base::products::check_reply_budget(reply)
}

#[cfg(any(test, feature = "test-support"))]
type ReplyKey = (u64, String, Vec<String>);

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Cache {
    catalogue: Option<Arc<Catalogue>>,
    content: Option<Content>,
    #[cfg(any(test, feature = "test-support"))]
    replies: VecDeque<(ReplyKey, SpecialProjectsReply)>,
    completions: VecDeque<(u64, Arc<ProjectCompletions>)>,
    content_error: Option<String>,
}

#[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
            ProjectLabel::decl(),
            ScientistSkill::decl(),
            ScientistDetails::decl(),
            ScientistAssignment::decl(),
            ProgramDetails::decl(),
            FacilityDetails::decl(),
            FacilitySource::decl(),
            ProjectChoice::decl(),
            SpecializationProgress::decl(),
            ProjectEffects::decl(),
            ProjectRewardOption::decl(),
            PendingProjectReward::decl(),
            ProjectVariableValue::decl(),
            ProjectRuntimeDetails::decl(),
            SpecialProjectDetails::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.country().validate()?;
        for tag in &params.selection_tags {
            dashboard_core::protocol::validate_tag(tag)?;
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let politics = context.content().and_then(|options| {
            recorded_politics_content(source, options)
        });
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        prepare_cache(&mut cache, &politics);
        let country = params.country();
        let mut roots = params.selection_tags.clone();
        roots.sort();
        roots.dedup();
        let key = (country.branch(), params.tag.clone(), roots.clone());
        // Kept replies hold their art as operands; each reply draws it as its request shows it.
        let art = ArtSource::new(source, context.content.as_ref());
        if let Some((_, reply)) = cache.replies.iter().find(|(seen, _)| *seen == key) {
            let mut reply = reply.clone();
            drop(cache);
            present_reply(source, &art, &mut reply)?;
            return Ok(reply);
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
        drop(cache);
        present_reply(source, &art, &mut reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod bench;
#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod history_tests;

#[cfg(test)]
mod art_tests {
    use super::*;
    use dashboard_base::test_fixtures::visual_mods::{
        BLUE, GREEN, Machine, RED, fallback_line, first_pixel, png, select,
    };

    fn sprites(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut text = String::from("spriteTypes = { ");
        for (name, texture) in entries {
            text.push_str(&format!("spriteType = {{ name = {name} textureFile = {texture} }} "));
        }
        text.push('}');
        text.into_bytes()
    }

    /// A reply as preparation stores it: a project icon and a scientist's portrait, kept as the
    /// operands they are chosen and drawn from.
    fn stored() -> SpecialProjectsReply {
        let (mut reply, _) = build_reply(
            Builder::default(),
            &CountryParams {
                branch_id: "1".into(),
                tag: "GER".into(),
            },
            false,
            Vec::new(),
        );
        reply.presentations.push(ProjectPresentation {
            name: "Project".into(),
            specialization: None,
            specialization_name: None,
            icon: None,
            icon_missing: None,
            icon_choice: Some(ProjectIconChoice {
                authored: None,
                token: "mod_project".into(),
                tag: Some("GER".into()),
            }),
            scientist: 0,
            scientist_as_of: None,
        });
        reply.people.push(ProjectPerson {
            assigned: Some(true),
            id: Some(7),
            name: None,
            portrait: None,
            missing: None,
            portrait_choice: Some(RecordedPortrait::Entry {
                kind: 0,
                identity: b"GFX_scientist".to_vec(),
            }),
        });
        reply
    }

    /// A project's icon and its scientist's portrait, kept as recorded operands, are chosen and
    /// drawn when the reply is served: the recording's own without a selection, through the same
    /// path; a selected mod's drawing; the choice made again when a mod declares a sprite the
    /// recording lacks (the country-prefixed icon); and, where the selection cannot draw them, the
    /// recording's own choice, named on every reply.
    #[test]
    fn project_icons_and_portraits_are_chosen_and_drawn_from_the_selection() {
        let machine = Machine::new("project_art");
        machine.write(
            "common/defines/00_fixture.lua",
            b"NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, \
              NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_officers\" } } }",
        );
        let recorded = [
            ("GFX_mod_project", "gfx/project.png"),
            ("GFX_scientist", "gfx/scientist.png"),
        ];
        machine.write("interface/projects.gfx", &sprites(&recorded));
        machine.write("gfx/project.png", &png(RED));
        machine.write("gfx/scientist.png", &png(RED));
        machine.write_mod("art", "gfx/project.png", &png(BLUE));
        machine.write_mod("art", "gfx/scientist.png", &png(BLUE));
        machine.write_mod(
            "country",
            "interface/country.gfx",
            &sprites(&[("GFX_GER_mod_project", "gfx/country.png")]),
        );
        machine.write_mod("country", "gfx/country.png", &png(GREEN));
        machine.write_mod(
            "broken",
            "interface/projects.gfx",
            &sprites(&[
                ("GFX_mod_project", "gfx/broken.png"),
                ("GFX_scientist", "gfx/broken.png"),
            ]),
        );
        machine.write_mod("broken", "gfx/broken.png", b"not a png");
        let (source, mut options) = machine.open();
        let shown = |options: &dashboard_core::config::ContentOptions| {
            let mut reply = stored();
            let art = ArtSource::new(&source, Some(options));
            let ((), line) =
                fallback_line(|| present_reply(&source, &art, &mut reply).unwrap());
            assert!(reply.presentations[0].icon_choice.is_none());
            assert!(reply.people[0].portrait_choice.is_none());
            (
                first_pixel(reply.presentations[0].icon.as_deref().unwrap()),
                first_pixel(reply.people[0].portrait.as_deref().unwrap()),
                line,
            )
        };
        select(&source, &mut options, &[]);
        let renders = dashboard_base::catalogue::presented;
        let before = renders();
        assert_eq!(shown(&options), (RED, RED, None));
        assert_eq!(renders() - before, 2, "one render per returned icon and portrait");
        let before = renders();
        assert_eq!(shown(&options), (RED, RED, None));
        assert_eq!(renders() - before, 0, "a second reply only hits the memo");
        select(&source, &mut options, &["art"]);
        assert_eq!(shown(&options), (BLUE, BLUE, None));
        select(&source, &mut options, &["country"]);
        assert_eq!(shown(&options), (GREEN, RED, None), "the country's own icon is chosen");
        select(&source, &mut options, &["broken"]);
        for _ in 0..2 {
            let (icon, portrait, line) = shown(&options);
            assert_eq!((icon, portrait), (RED, RED));
            let line = line.unwrap();
            assert!(line.contains("project mod_project icon"), "{line}");
            assert!(line.contains("scientist portrait"), "{line}");
        }
        // Without content nothing can be drawn, and each says so.
        let mut reply = stored();
        present_reply(&source, &ArtSource::new(&source, None), &mut reply).unwrap();
        assert!(reply.presentations[0].icon_missing.is_some());
        assert!(reply.people[0].missing.is_some());
    }

    /// A person preparation could not resolve is counted among the reply's warnings by its
    /// reason; a project with no scientist assigned is a normal absence and counts nothing.
    #[test]
    fn unresolved_people_are_counted_and_unassigned_ones_are_not() {
        let machine = Machine::new("project_people_counted");
        let (source, mut options) = machine.open();
        select(&source, &mut options, &[]);
        let mut reply = stored();
        reply.presentations.clear();
        reply.people = vec![
            ProjectPerson {
                assigned: Some(true),
                id: None,
                name: None,
                portrait: None,
                missing: Some("Character 9 was not recorded.".into()),
                portrait_choice: None,
            },
            ProjectPerson {
                assigned: Some(false),
                id: None,
                name: None,
                portrait: None,
                missing: Some("No scientist assigned.".into()),
                portrait_choice: None,
            },
        ];
        let before = reply.warnings.len();
        present_reply(&source, &ArtSource::new(&source, Some(&options)), &mut reply).unwrap();
        assert_eq!(
            reply.warnings[before..],
            ["Character 9 was not recorded. (count: 1)".to_owned()]
        );
    }

    /// Art content is loaded only for a reply holding art, and content that does not load
    /// leaves each piece unavailable with why while the rest of the reply stands.
    #[test]
    fn art_content_is_loaded_only_when_needed_and_its_failure_keeps_the_reply() {
        let machine = Machine::new("project_art_lazily");
        let (source, mut options) = machine.open();
        select(&source, &mut options, &[]);
        let art = ArtSource::new(&source, Some(&options));
        let mut bare = stored();
        bare.presentations.clear();
        bare.people.clear();
        present_reply(&source, &art, &mut bare).unwrap();
        assert!(art.loaded.get().is_none(), "nothing to draw loads nothing");
        let failing = ArtSource::new(&source, Some(&options));
        assert!(failing
            .loaded
            .set(Err("Project artwork is unavailable: fixture".into()))
            .is_ok());
        let mut reply = stored();
        let presentations = reply.presentations.len();
        present_reply(&source, &failing, &mut reply).unwrap();
        assert_eq!(reply.presentations.len(), presentations);
        assert_eq!(
            reply.presentations[0].icon_missing.as_deref(),
            Some("Project artwork is unavailable: fixture")
        );
        assert_eq!(
            reply.people[0].missing.as_deref(),
            Some("Project artwork is unavailable: fixture")
        );
        assert!(reply.presentations[0].icon_choice.is_none());
        let pieces = reply.presentations.len() + reply.people.len();
        assert!(
            reply.warnings.contains(&format!(
                "Project artwork is unavailable: fixture (count: {pieces})"
            )),
            "{:?}",
            reply.warnings
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::SharedView, frame::TableView};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub(super) struct TempRoot(pub(super) std::path::PathBuf);
    impl TempRoot {
        pub(super) fn new(label: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "dashboard-projects-{label}-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
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
                    state: replay_data::rows::ProjectState {
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
    fn every_recorded_hour_is_read_and_runs_break_at_gaps_and_incomplete_tables() {
        // The sliced projects family records every hour's rows exactly: a
        // country is read when a hooked writer marks it or after the daily
        // pass, and carried otherwise. No hour waits for a daily slot.
        assert_eq!(
            replay_data::cadence::cadence(TableId::Projects),
            replay_data::cadence::Cadence::Sliced
        );
        let mut builder = Builder::default();
        builder.observe(&frame(99, false, 1), "mod tag", None);
        assert_eq!(builder.episodes.len(), 1);
        builder.observe(&frame(101, false, 1), "mod tag", None);
        builder.observe(&frame(102, false, 1), "mod tag", None);
        builder.observe(&frame(110, false, 1), "mod tag", None);
        builder.observe(&frame(111, false, 1), "mod tag", None);
        let mut incomplete = frame(112, false, 1);
        *incomplete.tables[TableId::Projects.index()] = TableView::Unavailable {
            reason: "not captured".into(),
        };
        builder.observe(&incomplete, "mod tag", None);
        builder.observe(&frame(113, false, 1), "mod tag", None);
        builder.observe(&frame(114, false, 1), "mod tag", None);
        // One project, one episode; neither a gap nor an incomplete hour is given a value.
        assert_eq!(builder.episodes.len(), 1);
        assert_eq!(builder.reads, [99, 101, 102, 110, 111, 113, 114]);
        let spans = [(99, 99), (101, 102), (110, 111), (113, 114)];
        assert_eq!(
            builder
                .available
                .iter()
                .map(|s| (s.first_hour, s.last_hour))
                .collect::<Vec<_>>(),
            spans
        );
        assert_eq!(
            builder.episodes[0]
                .runs
                .iter()
                .map(|run| (run.first_hour, run.last_hour))
                .collect::<Vec<_>>(),
            spans
        );
    }

    #[test]
    fn completion_keeps_only_a_dated_last_assignment_and_a_gap_cannot_supply_it() {
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
                replay_data::rows::ProjectProgramRow {
                    id: 12,
                    owner: 5,
                    scientist: (73_u64 << 32) | 73,
                    ..Default::default()
                },
            ]));
        *sample.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![
                replay_data::rows::CharacterRow {
                    id: 73,
                    ..Default::default()
                },
            ]));
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
        let mut sample = frame(100, true, 1);
        let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
        project.program_reference = (12_u64 << 32) | 83;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        *sample.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![
                replay_data::rows::ProjectProgramRow {
                    id: 12,
                    scientist: (73_u64 << 32) | 73,
                    ..Default::default()
                },
            ]));
        *sample.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![
                replay_data::rows::CharacterRow {
                    id: 73,
                    ..Default::default()
                },
            ]));
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", None);
        sample.hour = 101;
        builder.observe(&sample, "mod tag", None);
        assert_eq!(builder.people.len(), 1);
        assert_eq!(builder.naming.people.len(), 1);
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
                replay_data::rows::ProjectProgramRow {
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
    fn history_count_uses_native_vector_length_missing_is_unknown_and_reset_is_a_new_episode() {
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
        engine_executable::executable_tables::initialize_for_tests()
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
            installed_content_core::ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])])
                .unwrap();
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
        let loc = installed_content_localisation::localisation::read(&layers, "english").unwrap();
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
        let sprites = installed_content_gfx::gfx::read(&layers).unwrap();
        let key = icon_key(
            catalog.projects["base_project"].icon.as_deref(),
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
                replay_data::rows::LexicalTokenRow {
                    token: 8,
                    literal: b"base_project".to_vec(),
                },
            ]));
        let mut builder = Builder::default();
        builder.observe(&sample, "mod tag", Some(&mut content));
        sample.hour = 101;
        builder.observe(&sample, "mod tag", Some(&mut content));
        assert_eq!(builder.content_ids.len(), 1);
        assert_eq!(builder.naming.people.len(), 1);
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
            installed_content_core::ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])])
                .unwrap();
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
            state: replay_data::rows::ProjectState {
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
        let mut project = installed_content_core::special_projects::Project::default();
        assert_eq!(
            icon_key(project.icon.as_deref(), "mod_project", Some("tag with spaces"), |key| Ok(
                key == "GFX_tag with spaces_mod_project"
            ))
            .unwrap(),
            "GFX_tag with spaces_mod_project"
        );
        assert_eq!(
            icon_key(project.icon.as_deref(), "mod_project", Some("X"), |key| Ok(
                key == "GFX_mod_project"
            ))
            .unwrap(),
            "GFX_mod_project"
        );
        assert_eq!(
            icon_key(project.icon.as_deref(), "mod_project", None, |key| {
                assert_eq!(key, "GFX_mod_project");
                Ok(true)
            })
            .unwrap(),
            "GFX_mod_project"
        );
        project.icon = Some("GFX_missing_mod_icon".into());
        assert_eq!(
            icon_key(project.icon.as_deref(), "mod_project", Some("X"), |_| Ok(true)).unwrap(),
            "GFX_missing_mod_icon"
        );
    }

    /// The project icon `GFX_fixture_project` of a catalogue over `files` and
    /// `textures`, as its pixels.
    fn fixture_icon(
        files: &[(&str, String)],
        textures: &[(&str, Vec<u8>)],
    ) -> Result<(u32, u32, Vec<u8>), String> {
        let files: Vec<_> = files
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .collect();
        let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&files);
        for (path, bytes) in textures {
            let file = folder.0.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        dashboard_text::catalogue::texture_tests::write_replacement_textures(&folder.0);
        project_icon(&catalogue, "GFX_fixture_project").map(|icon| {
            let image = dashboard_text::catalogue::texture_tests::from_url(&icon);
            (image.width, image.height, image.pixels)
        })
    }

    #[test]
    fn project_icons_draw_the_native_sprite_down_across_and_animated() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use dashboard_text::catalogue::texture_tests::strip_png;
        let [red, green, blue] = [[200, 0, 0, 255], [0, 200, 0, 255], [0, 0, 200, 255]];
        for kind in ["spriteType", "frameAnimatedSpriteType"] {
            for down in [false, true] {
                let direction = if down { "yes" } else { "no" };
                let gfx = format!(
                    "spriteTypes = {{ {kind} = {{ name = GFX_fixture_project texturefile = gfx/project.png noOfFrames = 3 frameDirectionY = {direction} }} }}"
                );
                let (width, height, pixels) = fixture_icon(
                    &[("interface/project.gfx", gfx)],
                    &[("gfx/project.png", strip_png(&[red, green, blue], down))],
                )
                .unwrap();
                // One whole frame, the first, whichever way the strip runs.
                assert_eq!([width, height], [2, 2], "{kind} {direction}");
                assert_eq!(pixels, red.repeat(4), "{kind} {direction}");
            }
        }
        let why = fixture_icon(
            &[(
                "interface/project.gfx",
                "spriteTypes = { corneredTileSpriteType = { name = GFX_fixture_project texturefile = gfx/project.png } }".into(),
            )],
            &[("gfx/project.png", strip_png(&[red], false))],
        )
        .unwrap_err();
        assert!(
            why.contains("GFX_fixture_project") && why.contains("needs native sprite rendering"),
            "{why}"
        );
    }

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn project_icons_execute_a_mod_s_effects_masks_and_animations() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use dashboard_text::catalogue::texture_tests::{replacement_programs, replacement_sprites};
        for (declaration, pixel) in replacement_sprites("GFX_fixture_project") {
            let mut files = replacement_programs();
            files.push((
                "interface/project.gfx",
                format!("spriteTypes = {{ {declaration} }}"),
            ));
            let (_, _, pixels) = fixture_icon(&files, &[]).unwrap();
            assert!(
                pixels.chunks_exact(4).all(|rgba| rgba == pixel),
                "{declaration}: {pixels:?}"
            );
        }
    }
}
