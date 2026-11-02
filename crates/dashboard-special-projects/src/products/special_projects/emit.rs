//! Special projects' emit: each country's observations on the family's series publication
//! (each a run of its ordinal, with the observation as an event), its holder's identity and
//! each project's progress as runs; for the family, the hours Projects was read complete, the
//! dictionary's revision, and each project completion the event feed lists, as an event.
//!
//! A country's runs go on while it keeps its holder and every hour is walked. A holder change
//! and an hour not walked start a segment, and the country is observed whole. A country the
//! countries no longer resolve ends its runs. An observation equal to the last changes nothing.

use super::{
    Builder, Content, ProjectCompletion, ProjectDate, ProjectEpisode, ProjectFacts,
    completion_look, completions,
    facts::{self, Project, Seen},
    history::{Name, NameKey},
    take_refusals,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use dashboard_base::catalogue::{self, Catalogue};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A country's observation ordinal, each observation an [`kind::OBSERVATION`] event; its
/// holder (canonical slot above country id); and each project's progress, by local id above
/// definition id.
pub const OBSERVED: u64 = 0;
pub const IDENTITY: u64 = 1;
pub const PROGRESS: u64 = 2;
/// The family's keys: the hours Projects was read complete, the dictionary's revision, and
/// the completions.
pub const READS: u64 = 0;
pub const LEXICAL: u64 = 1;
pub const COMPLETIONS: u64 = 2;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// The family scalar holding its [`Metadata`].
pub const METADATA: u64 = 0;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[Text observation]`, its JSON, on a country's OBSERVED key.
    pub const OBSERVATION: u64 = 0;
    /// `[Text change]`, a [`super::Change`]'s JSON, on the family's COMPLETIONS key.
    pub const CHANGE: u64 = 1;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved projects emit is not valid")
}

/// What the emit reads of the bound content ([`facts::Binding::emitted`]).
#[derive(Clone, Default)]
pub struct Bound {
    pub(super) content: Option<Content>,
    pub(super) catalogue: Option<Arc<Catalogue>>,
    /// The definitions' and catalogue's warnings.
    pub(super) warnings: Vec<String>,
    /// The definitions' and verification's warnings, which the completions report.
    pub(super) completion_warnings: Vec<String>,
    /// Why there is no content.
    pub(super) error: Option<String>,
    /// The content holds no readable definition.
    pub(super) empty: bool,
}

/// What the replies read of the family besides its keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// The branch's first recorded hour.
    pub start: i32,
    pub warnings: Vec<String>,
    pub content_error: Option<String>,
    pub definitions_empty: bool,
    /// Each country's scoped project-name refusals, counted.
    pub scoped_warnings: BTreeMap<String, Vec<String>>,
    /// Whether any country's projects were read complete.
    pub available: bool,
    pub completion_warnings: Vec<String>,
}

/// A completion as the event feed lists it: the change one observation made to an episode's
/// completion. A completion without a presentation withdraws the one before; its warning
/// stays named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub id: u64,
    pub identity: (u32, u32),
    pub local_id: u32,
    pub definition_id: u32,
    pub program_reference: u64,
    pub date: ProjectDate,
    pub observed_hour: i32,
    pub completion: Option<ProjectCompletion>,
    pub warnings: Vec<(String, usize)>,
    pub refusals: catalogue::Refusals,
}

/// A completion an observation recognised: its project's place in the observation, its
/// episode, its date, and the program its first completion named.
struct Pending {
    ordinal: usize,
    id: u64,
    date: ProjectDate,
    program_reference: u64,
}

/// One holder's open episodes, as the completions step them: each with its stable id, its
/// last iterations, the program its first completion named and its last completion date.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Continuation {
    episodes: Vec<ProjectEpisode>,
    ids: Vec<u64>,
    #[serde(with = "dashboard_core::map_pairs")]
    open: BTreeMap<(u32, u32), usize>,
    iterations: BTreeMap<usize, i32>,
    programs: BTreeMap<usize, u64>,
    dates: BTreeMap<usize, ProjectDate>,
}

impl Continuation {
    /// The completions whose dates `projects` change at `hour`.
    fn observe(&mut self, hour: i32, projects: &[Project], next_id: &mut u64) -> Vec<Pending> {
        let facts: Vec<ProjectFacts> = projects.iter().map(|project| project.facts.clone()).collect();
        let mut builder = Builder {
            compact: true,
            episodes: std::mem::take(&mut self.episodes),
            open: std::mem::take(&mut self.open),
            last_iterations: std::mem::take(&mut self.iterations),
            ..Default::default()
        };
        let mut ids = std::mem::take(&mut self.ids);
        let mut programs = std::mem::take(&mut self.programs);
        let mut dates = std::mem::take(&mut self.dates);
        let mut pending = Vec::new();
        builder.observe_projects(hour, &facts, |_, ordinal, index, _, _, completion| {
            if index == ids.len() {
                ids.push(*next_id);
                *next_id += 1;
            }
            if let Some(date) = completion
                && dates.get(&index) != Some(date)
            {
                dates.insert(index, date.clone());
                let program = *programs
                    .entry(index)
                    .or_insert(projects[ordinal].look.program_reference);
                pending.push(Pending {
                    ordinal,
                    id: ids[index],
                    date: date.clone(),
                    program_reference: program,
                });
            }
            0
        });
        // Finished and removed episodes are events already: only the open ones go on, each
        // with its stable id.
        for (key, previous) in builder.open {
            let index = self.episodes.len();
            self.episodes.push(builder.episodes[previous].clone());
            self.ids.push(ids[previous]);
            self.open.insert(key, index);
            if let Some(&count) = builder.last_iterations.get(&previous) {
                self.iterations.insert(index, count);
            }
            if let Some(&program) = programs.get(&previous) {
                self.programs.insert(index, program);
            }
            if let Some(date) = dates.get(&previous) {
                self.dates.insert(index, date.clone());
            }
        }
        pending
    }
}

/// A holder's run under one tag: from its first hour through its last, none while it goes on.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Holding {
    first: i32,
    last: Option<i32>,
    tag: Arc<str>,
}

/// Who held each holder, and what each scoped project name read, hour by hour: what a
/// completion is presented with at its date.
#[derive(Clone, Default, Serialize, Deserialize)]
struct History {
    #[serde(with = "dashboard_core::map_pairs")]
    holders: BTreeMap<(u32, u32), Vec<Holding>>,
    /// Each name's changes: the hour each value holds from, none where it names nothing.
    #[serde(with = "dashboard_core::map_pairs")]
    names: BTreeMap<NameKey, Vec<(i32, Option<Name>)>>,
    /// Each dictionary token's spellings: the hour each holds from, none where the dictionary
    /// holds none (or none readable).
    tokens: BTreeMap<u32, Vec<(i32, Option<String>)>>,
}

/// `list`'s value from `from` on, an equal one going on.
fn change<T: PartialEq>(list: &mut Vec<(i32, Option<T>)>, from: i32, value: Option<T>) {
    if list.last().map(|(_, held)| held) != Some(&value) {
        list.push((from, value));
    }
}

/// `list`'s value at `hour`.
fn value_at<T>(list: &[(i32, Option<T>)], hour: i32) -> Option<&T> {
    let at = list.partition_point(|&(from, _)| from <= hour).checked_sub(1)?;
    list[at].1.as_ref()
}

impl History {
    /// The tag holding `identity` at `hour`.
    fn tag_at(&self, identity: (u32, u32), hour: i32) -> Option<&Arc<str>> {
        self.holders
            .get(&identity)?
            .iter()
            .rfind(|run| run.first <= hour && run.last.is_none_or(|last| hour <= last))
            .map(|run| &run.tag)
    }

    /// The scoped name of `key` at `hour`.
    fn name_at(&self, key: &NameKey, hour: i32) -> Option<&Name> {
        value_at(self.names.get(key)?, hour)
    }

    /// Token `token`'s spelling at `hour`.
    fn token_at(&self, token: u32, hour: i32) -> Option<&String> {
        value_at(self.tokens.get(&token)?, hour)
    }
}

/// The completions the event feed lists, as each observation changes them: each holder's open
/// episodes, the next episode's id, and the history each is presented from.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Completions {
    #[serde(with = "dashboard_core::map_pairs")]
    continuations: BTreeMap<(u32, u32), Continuation>,
    next_id: u64,
    history: History,
}

impl Completions {
    /// `tag` holds `identity` from `from` on.
    pub(super) fn hold(&mut self, identity: (u32, u32), tag: &Arc<str>, from: i32) {
        self.history
            .holders
            .entry(identity)
            .or_default()
            .push(Holding {
                first: from,
                last: None,
                tag: Arc::clone(tag),
            });
    }

    /// `tag` holds `identity` no more after `through`.
    pub(super) fn release(&mut self, identity: (u32, u32), tag: &str, through: i32) {
        if let Some(run) = self
            .history
            .holders
            .get_mut(&identity)
            .and_then(|runs| runs.iter_mut().rfind(|run| run.last.is_none() && &*run.tag == tag))
        {
            run.last = Some(through);
        }
    }

    /// No holder goes on past `through`: the hour after is not walked.
    pub(super) fn close(&mut self, through: i32) {
        for run in self.history.holders.values_mut().flatten() {
            if run.last.is_none() {
                run.last = Some(through);
            }
        }
    }

    /// The scoped name of `key` from `from` on.
    pub(super) fn name(&mut self, key: NameKey, from: i32, value: Option<Name>) {
        change(self.history.names.entry(key).or_default(), from, value);
    }

    /// The scoped names `changes` from `from` on; with `whole`, every other names nothing.
    pub(super) fn names(&mut self, from: i32, changes: Vec<(NameKey, Option<Name>)>, whole: bool) {
        let held: BTreeSet<NameKey> = if whole {
            changes.iter().map(|(key, _)| key.clone()).collect()
        } else {
            BTreeSet::new()
        };
        for (key, value) in changes {
            change(self.history.names.entry(key).or_default(), from, value);
        }
        if whole {
            for (key, list) in &mut self.history.names {
                if !held.contains(key) {
                    change(list, from, None);
                }
            }
        }
    }

    /// The dictionary's spellings `changes` from `from` on; with `whole`, every other token is
    /// not held.
    pub(super) fn tokens(&mut self, from: i32, changes: Vec<(u32, Option<String>)>, whole: bool) {
        let held: BTreeSet<u32> = if whole {
            changes.iter().map(|&(token, _)| token).collect()
        } else {
            BTreeSet::new()
        };
        for (token, value) in changes {
            change(self.history.tokens.entry(token).or_default(), from, value);
        }
        if whole {
            for (token, list) in &mut self.history.tokens {
                if !held.contains(token) {
                    change(list, from, None);
                }
            }
        }
    }



    /// The completions `observed`'s `projects` change at `at`, each presented as the game showed
    /// it at its date: by the tag holding its holder then, named with that hour's scoped names;
    /// none, with why, at a date the recording holds no hour of (`unrecorded`) or whose holder
    /// no tag held. `content` names and draws them.
    pub(super) fn observe(
        &mut self,
        at: i32,
        observed: &facts::Observed,
        projects: &[Project],
        mut content: Option<&mut Content>,
        unrecorded: &dyn Fn(i32) -> bool,
        changes: &mut Vec<Change>,
    ) {
        let pending = self
            .continuations
            .entry(observed.identity)
            .or_default()
            .observe(at, projects, &mut self.next_id);
        for pending in pending {
            let project = &projects[pending.ordinal];
            let (completion, warnings, refusals) = if unrecorded(pending.date.hour) {
                let warnings =
                    BTreeMap::from([("Project completion hour was not recorded.".to_owned(), 1)]);
                (None, warnings, catalogue::Refusals::default())
            } else {
                self.present(at, observed.identity, project, &pending, content.as_deref_mut())
            };
            changes.push(Change {
                id: pending.id,
                identity: observed.identity,
                local_id: project.facts.local_id,
                definition_id: project.facts.definition_id,
                program_reference: pending.program_reference,
                date: pending.date,
                observed_hour: at,
                completion,
                warnings: warnings.into_iter().collect(),
                refusals,
            });
        }
    }

    /// `project`'s completion at `pending`'s date, recognised at `at`, as
    /// [`completions::present_completion`] presents it.
    fn present(
        &self,
        at: i32,
        identity: (u32, u32),
        project: &Project,
        pending: &Pending,
        content: Option<&mut Content>,
    ) -> (Option<ProjectCompletion>, BTreeMap<String, usize>, catalogue::Refusals) {
        let mut warnings = BTreeMap::<String, usize>::new();
        let date = pending.date.hour;
        let Some(tag) = self.history.tag_at(identity, date) else {
            warnings.insert(
                "Project country identity could not be resolved at its completion hour.".into(),
                1,
            );
            return (None, warnings, catalogue::Refusals::default());
        };
        let look = &project.look;
        let definition = project.facts.definition_id;
        let token = self.history.token_at(definition, date).cloned();
        let key: NameKey = (tag.to_string(), identity.0, definition);
        let scoped = |_: &Content| {
            self.history
                .name_at(&key, date)
                .and_then(|name| name.text.clone())
        };
        let named = content.is_some();
        take_refusals(content.as_deref());
        let (info, resolved, mut refusals) =
            completion_look(token.clone(), definition, tag, &scoped, content);
        refusals.merge(&look.person_refusals);
        let completion = ProjectCompletion {
            tag: tag.to_string(),
            date: pending.date.clone(),
            observed_hour: at,
            definition_id: definition,
            name: token
                .as_ref()
                .filter(|_| named)
                .map(|_| info.name.clone()),
            item_token: token,
            icon: info.icon.clone(),
            icon_missing: info.icon_missing.clone(),
            icon_choice: info.icon_choice.clone(),
        };
        for reason in self
            .history
            .name_at(&key, date)
            .map_or(&[][..], |name| name.warnings.as_slice())
        {
            *warnings.entry(reason.clone()).or_default() += 1;
        }
        for (reason, count) in resolved {
            if completion.icon_missing.as_deref() != Some(reason.as_str()) {
                *warnings
                    .entry(completions::structural_reason(&reason))
                    .or_default() += count;
            }
        }
        (Some(completion), warnings, refusals)
    }
}

/// An hour's completions in the order the feed lists them: withdrawn ones first, then each
/// country's by tag, each in the order its observation recognised them.
pub(super) fn order(changes: &mut [Change]) {
    changes.sort_by(|left, right| {
        let tag = |change: &Change| {
            change
                .completion
                .as_ref()
                .map(|completion| completion.tag.clone())
        };
        tag(left).cmp(&tag(right))
    });
}

/// Whether the recording holds no hour `hour` of a branch first recorded at `start` with `gaps`.
pub(super) fn unrecorded(start: i32, gaps: &[(i32, i32)], hour: i32) -> bool {
    hour < start || gaps.iter().any(|&(first, last)| first <= hour && hour <= last)
}

/// An observation as its event holds it.
#[derive(Serialize)]
struct Stored<'a> {
    identity: (u32, u32),
    projects: Option<Vec<(&'a ProjectFacts, &'a super::Look)>>,
}

/// What a country holds now, and its keys.
#[derive(Clone, Default)]
struct Country {
    tag: Arc<str>,
    /// OBSERVED and IDENTITY.
    keys: [KeyHandle; 2],
    /// Each project's PROGRESS key, by its dimension.
    progress: Vec<(u64, KeyHandle)>,
    /// Its last observation's digest and ordinal.
    hash: Option<[u8; 32]>,
    version: u64,
    /// Its holder while it is live.
    identity: Option<(u32, u32)>,
    /// Its PROGRESS keys holding a value.
    active: Vec<KeyHandle>,
    /// The projects its episodes hold open after its last observation, under the holder they
    /// were observed with, as the reply's episodes and the name scan keep them per tag.
    open: Option<((u32, u32), BTreeSet<(u32, u32)>)>,
}

/// A project's PROGRESS dimension.
pub fn dimension(local_id: u32, definition_id: u32) -> u64 {
    u64::from(local_id) << 32 | u64::from(definition_id)
}

/// The family's emit on one branch.
pub struct Emit {
    bound: Bound,
    /// The branch's first recorded hour and its gaps, which a completion date may lie in.
    start: i32,
    gaps: Vec<(i32, i32)>,
    actors: Actors,
    carry: Carry,
    countries: Vec<Country>,
    /// READS, LEXICAL and COMPLETIONS.
    family: Option<[KeyHandle; 3]>,
    /// The dictionary's revision, and the digest it holds.
    revision: u64,
    dictionary: Option<u128>,
    own: catalogue::OwnRefusals,
    named: BTreeMap<String, BTreeMap<String, usize>>,
    available: bool,
    completions: Completions,
    /// The provisional names the last hour gave a value.
    provisional: BTreeSet<NameKey>,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    pub fn new(bound: Bound, start: i32, gaps: Vec<(i32, i32)>) -> Self {
        Self {
            bound,
            start,
            gaps,
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            countries: Vec::new(),
            family: None,
            revision: 0,
            dictionary: None,
            own: catalogue::OwnRefusals::default(),
            named: BTreeMap::new(),
            available: false,
            completions: Completions::default(),
            provisional: BTreeSet::new(),
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 3]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 3];
        for (key, measure) in keys.iter_mut().zip([READS, LEXICAL, COMPLETIONS]) {
            *key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.family = Some(keys);
        Ok(keys)
    }

    /// The country actor named `tag`, with its keys.
    fn country(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.countries.len() {
            self.countries.resize(index + 1, Country::default());
        }
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([OBSERVED, IDENTITY]) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.countries[index].keys = keys;
        self.countries[index].tag = Arc::from(tag);
        Ok(actor)
    }

    /// `actor`'s PROGRESS key of `dimension`.
    fn progress(&mut self, writer: &mut Writer<'_>, actor: u64, dimension: u64) -> io::Result<KeyHandle> {
        let held = &self.countries[actor as usize].progress;
        let at = held.partition_point(|&(held, _)| held < dimension);
        if let Some(&(held, key)) = held.get(at)
            && held == dimension
        {
            return Ok(key);
        }
        let key = self.key(
            writer,
            Place {
                actor,
                measure: PROGRESS,
                dimension: Some(dimension),
            },
        )?;
        self.countries[actor as usize]
            .progress
            .insert(at, (dimension, key));
        Ok(key)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("projects.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A projects hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.close(writer, last)?;
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let [reads, lexical, _] = self.family(writer)?;
        self.carry
            .value(writer, reads, at, hour.read.then_some(1), !adjacent)?;
        // A new revision where the dictionary's rows differ, whatever segment reads them; a
        // segment's first hour sends it whole, which an equal digest leaves as it is.
        let digest = hour.dictionary.map(|dictionary| {
            let facts::Dictionary {
                digest,
                whole,
                tokens,
            } = *dictionary;
            if !whole || self.dictionary != Some(digest) {
                self.completions.tokens(at, tokens, whole);
            }
            digest
        });
        if let Some(digest) = digest
            && self.dictionary != Some(digest)
        {
            self.dictionary = Some(digest);
            self.revision += 1;
            self.carry
                .value(writer, lexical, at, Some(i128::from(self.revision)), !adjacent)?;
        } else if !adjacent {
            self.carry
                .value(writer, lexical, at, Some(i128::from(self.revision)), true)?;
        }
        let Some(facts) = hour.facts else {
            return Ok(());
        };
        let facts::Facts {
            full,
            countries,
            named,
            names,
            names_whole,
            provisional,
            own,
        } = *facts;
        let supplied: BTreeSet<NameKey> = names.iter().map(|(key, _)| key.clone()).collect();
        self.completions.names(at, names, names_whole);
        if let Some(own) = own {
            self.own.merge(&own);
            self.stale = true;
        }
        for (tag, reasons) in named {
            let counts = self.named.entry(tag).or_default();
            for (reason, count) in reasons {
                *counts.entry(reason).or_default() += count;
            }
            self.stale = true;
        }
        let mut seen = BTreeSet::new();
        for observed in &countries {
            seen.insert(self.observe(writer, at, adjacent, observed)?);
        }
        if full {
            for actor in 0..self.countries.len() as u64 {
                if !seen.contains(&actor) {
                    self.leave(writer, at, actor)?;
                }
            }
        }
        // A possibly open project's name is its tag's where the tag's episode of it is open
        // after its last observation, under the holder this hour observes, and is counted there.
        let mut provisional_names = BTreeMap::<NameKey, (bool, Name)>::new();
        for (key, project, name) in provisional {
            let open = self.actors.find(key.0.as_bytes()).is_some_and(|actor| {
                let country = &self.countries[actor as usize];
                country.open.as_ref().is_some_and(|(identity, open)| {
                    country.identity == Some(*identity) && open.contains(&project)
                })
            });
            let held = provisional_names.entry(key).or_insert((false, name));
            held.0 |= open;
        }
        // A name the last hour gave a value and this hour gives none names nothing.
        let gone: Vec<NameKey> = self
            .provisional
            .iter()
            .filter(|key| !provisional_names.contains_key(*key) && !supplied.contains(*key))
            .cloned()
            .collect();
        for key in gone {
            self.completions.name(key, at, None);
        }
        self.provisional = provisional_names
            .iter()
            .filter(|(_, (open, _))| *open)
            .map(|(key, _)| key.clone())
            .collect();
        for (key, (open, name)) in provisional_names {
            if open {
                let counts = self.named.entry(key.0.clone()).or_default();
                for reason in &name.warnings {
                    *counts.entry(reason.clone()).or_default() += 1;
                }
                self.stale |= !name.warnings.is_empty();
            }
            self.completions.name(key, at, open.then_some(name));
        }
        // Each tag's open episodes after this hour's observation; its progress alone keeps them.
        for observed in &countries {
            let Seen::Whole(projects) = &observed.seen else {
                continue;
            };
            let Some(actor) = self.actors.find(observed.tag.as_bytes()) else {
                continue;
            };
            let country = &mut self.countries[actor as usize];
            let held = country
                .open
                .take()
                .filter(|(identity, _)| *identity == observed.identity)
                .map(|(_, open)| open)
                .unwrap_or_default();
            let open = match projects {
                Some(projects) => projects
                    .iter()
                    .map(|project| (project.facts.local_id, project.facts.definition_id))
                    .zip(projects)
                    .filter(|(key, project)| project.facts.visible || held.contains(key))
                    .map(|(key, _)| key)
                    .collect(),
                None => held,
            };
            country.open = Some((observed.identity, open));
        }
        // Every holder known first: a completion is presented by its holder at its date. Progress
        // alone goes on from an observation the completions took ([`facts::Seen::Progress`]).
        let mut changes = Vec::new();
        for observed in countries.iter().filter(|_| hour.read) {
            let Seen::Whole(Some(projects)) = &observed.seen else {
                continue;
            };
            self.stale |= !self.available;
            self.available = true;
            let (start, gaps) = (self.start, &self.gaps);
            self.completions.observe(
                at,
                observed,
                projects,
                self.bound.content.as_mut(),
                &|hour| unrecorded(start, gaps, hour),
                &mut changes,
            );
        }
        order(&mut changes);
        let [_, _, key] = self.family(writer)?;
        for change in changes {
            let bytes = serde_json::to_vec(&change)?;
            let text = writer.blob(&mut &bytes[..], bytes.len() as u64)?;
            let fields = writer.cells([Cell::Text(text)])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: at,
                read_hour: Some(at),
                sequence,
                kind: kind::CHANGE,
                fields,
            })?;
        }
        Ok(())
    }

    /// `observed` from `at` on: its holder, each project's progress, and its observation where it
    /// differs from the last or does not go on from the hour before.
    fn observe(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        adjacent: bool,
        observed: &facts::Observed,
    ) -> io::Result<u64> {
        let actor = self.country(writer, &observed.tag, at)?;
        let index = actor as usize;
        let continuing = adjacent && self.countries[index].identity == Some(observed.identity);
        let projects = match &observed.seen {
            Seen::Whole(projects) => projects,
            Seen::Progress(progress) => {
                // Its last observation's projects, their PROGRESS keys in its order.
                if !continuing || progress.len() != self.countries[index].active.len() {
                    return Err(dashboard_core::invalid(
                        "A projects progress does not go on from the country's last observation",
                    ));
                }
                for (at_key, &value) in progress.iter().enumerate() {
                    let key = self.countries[index].active[at_key];
                    self.carry
                        .value(writer, key, at, Some(i128::from(value)), false)?;
                }
                return Ok(actor);
            }
        };
        let [observation, identity] = self.countries[index].keys;
        if !continuing {
            if let Some(held) = self.countries[index].identity {
                self.completions.release(held, &observed.tag, at - 1);
            }
            self.completions.hold(observed.identity, &observed.tag, at);
            let (slot, id) = observed.identity;
            let holder = i128::from(slot) << 32 | i128::from(id);
            self.carry.value(writer, identity, at, Some(holder), true)?;
        }
        let mut active = Vec::new();
        for project in projects.iter().flatten() {
            let dimension = dimension(project.facts.local_id, project.facts.definition_id);
            let key = self.progress(writer, actor, dimension)?;
            self.carry.value(
                writer,
                key,
                at,
                Some(i128::from(project.progress)),
                !continuing,
            )?;
            active.push(key);
        }
        for key in std::mem::take(&mut self.countries[index].active) {
            if !active.contains(&key) {
                self.carry.value(writer, key, at, None, false)?;
            }
        }
        self.countries[index].active = active;
        let bytes = serde_json::to_vec(&Stored {
            identity: observed.identity,
            projects: projects.as_ref().map(|projects| {
                projects
                    .iter()
                    .map(|project| (&project.facts, &project.look))
                    .collect()
            }),
        })?;
        let hash = *blake3::hash(&bytes).as_bytes();
        let country = &mut self.countries[index];
        country.identity = Some(observed.identity);
        if continuing && country.hash == Some(hash) {
            return Ok(actor);
        }
        country.hash = Some(hash);
        country.version += 1;
        let version = country.version;
        self.carry
            .value(writer, observation, at, Some(i128::from(version)), !continuing)?;
        let text = writer.blob(&mut &bytes[..], bytes.len() as u64)?;
        let fields = writer.cells([Cell::Text(text)])?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key: observation,
            native_hour: at,
            read_hour: None,
            sequence,
            kind: kind::OBSERVATION,
            fields,
        })?;
        Ok(actor)
    }

    /// Ends what `actor`'s runs hold: the countries no longer resolve it.
    fn leave(&mut self, writer: &mut Writer<'_>, at: i32, actor: u64) -> io::Result<()> {
        let country = &mut self.countries[actor as usize];
        let Some(held) = country.identity.take() else {
            return Ok(());
        };
        self.completions.release(held, &country.tag, at - 1);
        let active = std::mem::take(&mut country.active);
        for key in country.keys.into_iter().chain(active) {
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        self.completions.close(last);
        for country in &mut self.countries {
            country.identity = None;
            country.active.clear();
        }
        Ok(())
    }

    /// The scalars the replies read.
    fn metadata(&self) -> Metadata {
        let mut own = self.own.clone();
        let mut warnings = self.bound.warnings.clone();
        if let Some(catalogue) = &self.bound.catalogue {
            // Only the lazy readers this family used report ([`catalogue::own_refusals`]).
            warnings.extend(catalogue::own_refusals(&mut own, || {
                catalogue.reader_warning_lines()
            }));
        }
        Metadata {
            start: self.start,
            warnings,
            content_error: self.bound.error.clone(),
            definitions_empty: self.bound.empty,
            scoped_warnings: self
                .named
                .iter()
                .filter(|(_, reasons)| !reasons.is_empty())
                .map(|(tag, reasons)| {
                    (
                        tag.clone(),
                        reasons
                            .iter()
                            .map(|(reason, count)| {
                                format!("{reason} ({count} project-name observations).")
                            })
                            .collect(),
                    )
                })
                .collect(),
            available: self.available,
            completion_warnings: self.bound.completion_warnings.clone(),
        }
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        if self.stale {
            let json = serde_json::to_vec(&self.metadata())?;
            let cell = family::cell(writer, &Scalar::Text(json.into_boxed_slice()))?;
            writer.catalogue(METADATA, cell)?;
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection observes every country anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            revision: self.revision,
            dictionary: self.dictionary,
            available: self.available,
            own: self.own.clone(),
            named: self.named.clone(),
            countries: self
                .countries
                .iter()
                .enumerate()
                .filter(|(_, country)| {
                    country.version != 0 || country.identity.is_some() || country.open.is_some()
                })
                .map(|(actor, country)| SavedCountry {
                    actor: actor as u64,
                    hash: country.hash,
                    version: country.version,
                    identity: country.identity,
                    open: country.open.clone(),
                    active: country
                        .active
                        .iter()
                        .filter_map(|key| {
                            country
                                .progress
                                .iter()
                                .find(|(_, held)| held == key)
                                .map(|&(dimension, _)| dimension)
                        })
                        .collect(),
                })
                .collect(),
            completions: self.completions.clone(),
            provisional: self.provisional.iter().cloned().collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(
        bound: Bound,
        start: i32,
        gaps: Vec<(i32, i32)>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut emit = Self::new(bound, start, gaps);
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.countries = vec![Country::default(); directory.actors.len()];
        for (actor, country) in emit.countries.iter_mut().enumerate() {
            country.tag = Arc::from(String::from_utf8_lossy(&directory.actors[actor].name).as_ref());
            let actor = actor as u64;
            for (key, measure) in country.keys.iter_mut().zip([OBSERVED, IDENTITY]) {
                *key = directory.key(actor, measure, None).ok_or_else(invalid)?;
            }
            country.progress = directory.dimensions(actor, PROGRESS).to_vec();
        }
        let mut family = [0; 3];
        for (key, measure) in family.iter_mut().zip([READS, LEXICAL, COMPLETIONS]) {
            *key = directory.key(FAMILY, measure, None).ok_or_else(invalid)?;
        }
        emit.family = Some(family);
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.revision = saved.revision;
        emit.dictionary = saved.dictionary;
        emit.available = saved.available;
        emit.own = saved.own;
        emit.named = saved.named;
        emit.completions = saved.completions;
        emit.provisional = saved.provisional.into_iter().collect();
        for country in saved.countries {
            let held = emit
                .countries
                .get_mut(country.actor as usize)
                .ok_or_else(invalid)?;
            held.hash = country.hash;
            held.version = country.version;
            held.identity = country.identity;
            held.open = country.open;
            held.active = country
                .active
                .iter()
                .map(|dimension| {
                    held.progress
                        .iter()
                        .find(|(held, _)| held == dimension)
                        .map(|&(_, key)| key)
                        .ok_or_else(invalid)
                })
                .collect::<io::Result<_>>()?;
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    actor: u64,
    hash: Option<[u8; 32]>,
    version: u64,
    identity: Option<(u32, u32)>,
    open: Option<((u32, u32), BTreeSet<(u32, u32)>)>,
    /// The dimensions of its PROGRESS keys holding a value.
    active: Vec<u64>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    revision: u64,
    dictionary: Option<u128>,
    available: bool,
    own: catalogue::OwnRefusals,
    named: BTreeMap<String, BTreeMap<String, usize>>,
    countries: Vec<SavedCountry>,
    completions: Completions,
    provisional: Vec<NameKey>,
}
