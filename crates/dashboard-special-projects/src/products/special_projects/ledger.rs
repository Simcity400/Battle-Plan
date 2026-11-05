//! A country's episodes as the ordered emit builds them, hour by hour, the way a reply builds
//! them from its observations: each observation's pieces (its hours between changes of the
//! observation, its projects' progress and the dictionary's revision), each piece's first hour
//! presented, its second hour where a completion's scientist takes its dated attribution there,
//! and the hours after extending what it holds open.
//!
//! What each hour adds is an entry of the country's ledger, and each piece's hours are a run of
//! its ordinal, so the reply through any hour reads the entries dated by then and clips each
//! piece to it.

use super::{
    Builder, Look, ProjectDate, ProjectEpisode, ProjectFacts, ProjectPerson, ProjectPresentation,
    ProjectState, SpecialProjectRun,
};
use dashboard_base::{catalogue, products::country_groups::HourSpan};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// An observation as its event holds it, and as a country's ledger goes on with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Observation {
    pub(super) identity: (u32, u32),
    pub(super) projects: Option<Vec<(ProjectFacts, Look)>>,
}

/// `hour` available: it goes on from the last span, or starts one.
fn available(builder: &mut Builder, hour: i32) {
    if builder
        .available
        .last()
        .is_some_and(|span| span.last_hour.checked_add(1) == Some(hour))
    {
        builder.available.last_mut().unwrap().last_hour = hour;
    } else {
        builder.available.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

/// The first hour of a piece of `identity`'s observation of `projects` at `first`, its progress
/// as `facts` hold it. Whether a completion first seen with a real scientist takes its dated
/// attribution at the next hour, where the piece goes on to it.
pub(super) fn head(
    builder: &mut Builder,
    tag: &str,
    identity: (u32, u32),
    projects: &[(ProjectFacts, Look)],
    first: i32,
    facts: &[ProjectFacts],
    refusals: &mut catalogue::Refusals,
) -> bool {
    if builder.identity.is_some_and(|old| old != identity) {
        builder.open.clear();
    }
    builder.identity = Some(identity);
    // Admit the first hour before processing the attribution boundary. Extending
    // coverage now would make that second observation overlap the whole interval.
    available(builder, first);
    builder.last = Some((identity, first));
    let mut present = |builder: &mut Builder,
                       ordinal: usize,
                       _index: usize,
                       fact: &ProjectFacts,
                       previous: Option<&SpecialProjectRun>,
                       _: Option<&ProjectDate>| {
        let look = &projects[ordinal].1;
        let id = builder.present_look(
            look,
            fact.definition_id,
            tag,
            fact.state == ProjectState::Completed,
            first,
            previous,
            refusals,
        );
        if let Some(count) = fact.iterations
            && !builder.iteration_texts.contains_key(&count)
        {
            builder
                .iteration_texts
                .insert(count, look.iteration_text.clone());
            refusals.merge(&look.iteration_refusals);
        }
        id
    };
    builder.observe_projects(first, facts, &mut present);
    builder.open.values().any(|index| {
        let run = builder.episodes[*index].runs.last().unwrap();
        let info = &builder.presentations[run.presentation as usize];
        run.state == ProjectState::Completed
            && info.scientist_as_of.is_none()
            && builder.people[info.scientist as usize].id.is_some()
    })
}

/// Extends what the piece holds open after `processed`, its last hour presented, through `last`.
pub(super) fn extend(builder: &mut Builder, facts: &[ProjectFacts], processed: i32, last: i32) {
    for fact in facts {
        let key = (fact.local_id, fact.definition_id);
        if let Some(&index) = builder.open.get(&key) {
            builder.episodes[index].runs.last_mut().unwrap().last_hour = last;
            if let Some(reason) = &fact.warning {
                *builder.warnings.entry(reason.clone()).or_default() +=
                    usize::try_from(i64::from(last) - i64::from(processed))
                        .expect("nonnegative interval");
            }
        }
    }
    builder.available.last_mut().unwrap().last_hour = last;
}

/// One piece, `first` through `last`, of `observation` whole.
pub(super) fn apply_interval(
    builder: &mut Builder,
    tag: &str,
    observation: &Observation,
    first: i32,
    last: i32,
    facts: &[ProjectFacts],
    refusals: &mut catalogue::Refusals,
) {
    let identity = observation.identity;
    let Some(projects) = &observation.projects else {
        builder.last = Some((identity, last));
        builder.admitted = false;
        return;
    };
    let second = head(builder, tag, identity, projects, first, facts, refusals) && last > first;
    let processed = if second {
        let second = first + 1;
        head(builder, tag, identity, projects, second, facts, refusals);
        extend(builder, facts, second, second);
        second
    } else {
        first
    };
    extend(builder, facts, processed, last);
    builder.last = Some((identity, last));
}

/// What a ledger entry adds to the reply built so far.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) enum Entry {
    /// An episode, its runs to come.
    Episode {
        id: String,
        start: ProjectDate,
    },
    /// A run of the episode at that index.
    Run(u32, SpecialProjectRun),
    /// The last hour of the episode's last run.
    Last(u32, i32),
    Presentation(ProjectPresentation),
    Person(ProjectPerson),
    /// The iteration texts first met.
    Iterations(BTreeMap<i32, Option<String>>),
    /// An hour available: it goes on from the last span, or starts one.
    Available(i32),
    /// Every warning count so far.
    Warnings(BTreeMap<String, usize>),
    /// The text refusals first met.
    Refusals(catalogue::Refusals),
    /// Through the last hour of piece `piece`, the last runs of `episodes` and the last span
    /// go on after `processed`, and each warning counts that many times an hour.
    Extend {
        piece: u64,
        processed: i32,
        episodes: Vec<u32>,
        warnings: Vec<(String, u64)>,
    },
}

/// The runs of `builder`'s episode at `episode`.
fn runs(builder: &mut Builder, episode: u32) -> io::Result<&mut Vec<SpecialProjectRun>> {
    builder
        .episodes
        .get_mut(episode as usize)
        .map(|episode| &mut episode.runs)
        .ok_or_else(|| invalid("A prepared project run names no episode"))
}

/// The last run of `builder`'s episode at `episode`.
fn last_run(builder: &mut Builder, episode: u32) -> io::Result<&mut SpecialProjectRun> {
    runs(builder, episode)?
        .last_mut()
        .ok_or_else(|| invalid("A prepared project episode holds no run"))
}

impl Entry {
    /// Adds itself to `builder` and `refusals`; `end` is each piece's last hour, clipped to the
    /// hour the reply is read through.
    pub(super) fn apply(
        self,
        builder: &mut Builder,
        refusals: &mut catalogue::Refusals,
        end: impl Fn(u64) -> Option<i32>,
    ) -> io::Result<()> {
        match self {
            Self::Episode { id, start } => builder.episodes.push(ProjectEpisode {
                id,
                start,
                runs: Vec::new(),
            }),
            Self::Run(episode, run) => runs(builder, episode)?.push(run),
            Self::Last(episode, hour) => last_run(builder, episode)?.last_hour = hour,
            Self::Presentation(presentation) => builder.presentations.push(presentation),
            Self::Person(person) => builder.people.push(person),
            Self::Iterations(texts) => builder.iteration_texts.extend(texts),
            Self::Available(hour) => available(builder, hour),
            Self::Warnings(warnings) => builder.warnings = warnings,
            Self::Refusals(added) => refusals.merge(&added),
            Self::Extend {
                piece,
                processed,
                episodes,
                warnings,
            } => {
                let last =
                    end(piece).ok_or_else(|| invalid("A prepared project piece has no hours"))?;
                if last <= processed {
                    return Ok(());
                }
                for episode in episodes {
                    last_run(builder, episode)?.last_hour = last;
                }
                builder
                    .available
                    .last_mut()
                    .ok_or_else(|| invalid("A prepared project piece is not available"))?
                    .last_hour = last;
                let hours = u64::try_from(i64::from(last) - i64::from(processed))
                    .map_err(|_| invalid("A prepared project piece is invalid"))?;
                for (reason, count) in warnings {
                    let added = usize::try_from(count.saturating_mul(hours))
                        .map_err(|_| invalid("A prepared project warning count is invalid"))?;
                    *builder.warnings.entry(reason).or_default() += added;
                }
            }
        }
        Ok(())
    }
}

/// What a piece extends after its last hour presented, and through which hour the track's
/// builder holds it extended.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Extension {
    processed: i32,
    through: i32,
    episodes: Vec<u32>,
    warnings: Vec<(String, u64)>,
}

impl Extension {
    /// What `facts`' episodes `builder` holds open extend after `processed`.
    fn of(builder: &Builder, facts: &[ProjectFacts], processed: i32) -> Self {
        let mut episodes = Vec::with_capacity(facts.len());
        let mut warnings: Vec<(String, u64)> = Vec::new();
        for fact in facts {
            let Some(&index) = builder.open.get(&(fact.local_id, fact.definition_id)) else {
                continue;
            };
            episodes.push(index as u32);
            if let Some(reason) = &fact.warning {
                match warnings.iter_mut().find(|(held, _)| held == reason) {
                    Some((_, count)) => *count += 1,
                    None => warnings.push((reason.clone(), 1)),
                }
            }
        }
        Self {
            processed,
            through: processed,
            episodes,
            warnings,
        }
    }

    fn entry(&self, piece: u64) -> Entry {
        Entry::Extend {
            piece,
            processed: self.processed,
            episodes: self.episodes.clone(),
            warnings: self.warnings.clone(),
        }
    }
}

/// The piece a country's observation is in: its ordinal, its facts with their progress, and
/// what it extends once its last hour presented is known.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Piece {
    pub(super) ordinal: u64,
    pub(super) facts: Vec<ProjectFacts>,
    extension: Option<Extension>,
    /// The hour its second is presented at, where the piece goes on to it.
    pub(super) second: Option<i32>,
}

/// An episode that is never stepped again: only its place is kept.
fn forgotten() -> ProjectEpisode {
    ProjectEpisode {
        id: String::new(),
        start: ProjectDate {
            hour: 0,
            exact: false,
        },
        runs: Vec::new(),
    }
}

/// Its warnings' reasons and their sum: counts only grow, so equal tallies, equal warnings.
fn tally(warnings: &BTreeMap<String, usize>) -> (usize, usize) {
    (warnings.len(), warnings.values().sum())
}

/// One country's ledger as the emit builds it: the reply's builder over every observation of
/// the country so far, holding of its episodes only what a later hour steps, and the observation
/// and piece it goes on with.
pub(super) struct Track {
    builder: Builder,
    /// The dictionary revision the builder's presentations follow.
    lexical: Option<u64>,
    refusals: catalogue::Refusals,
    pub(super) observation: Option<Observation>,
    pub(super) piece: Option<Piece>,
    /// The last piece's ordinal.
    pieces: u64,
}

impl Track {
    pub(super) fn new(start: i32) -> Self {
        Self {
            builder: Builder {
                campaign_start: Some(start),
                ..Default::default()
            },
            lexical: None,
            refusals: catalogue::Refusals::default(),
            observation: None,
            piece: None,
            pieces: 0,
        }
    }

    /// Extends the piece through `through`, as the reply does where the piece goes on there.
    fn settle(builder: &mut Builder, piece: &mut Piece, through: i32) {
        let Some(extension) = &mut piece.extension else {
            return;
        };
        if through <= extension.through {
            return;
        }
        let hours =
            u64::try_from(i64::from(through) - i64::from(extension.through)).expect("a later hour");
        for &episode in &extension.episodes {
            builder.episodes[episode as usize]
                .runs
                .last_mut()
                .unwrap()
                .last_hour = through;
        }
        builder.available.last_mut().unwrap().last_hour = through;
        for (reason, count) in &extension.warnings {
            *builder.warnings.entry(reason.clone()).or_default() += (count * hours) as usize;
        }
        extension.through = through;
    }

    /// Ends the piece after `last`, its last hour.
    pub(super) fn end(&mut self, last: i32) -> bool {
        match self.piece.take() {
            Some(mut piece) => {
                Self::settle(&mut self.builder, &mut piece, last);
                true
            }
            None => false,
        }
    }

    /// No hour after `last` goes on from it.
    pub(super) fn close(&mut self, last: i32) {
        self.end(last);
        self.observation = None;
    }

    /// The observation, without projects, from `at` on: the episodes of another holder end.
    pub(super) fn unobserved(&mut self, identity: (u32, u32)) {
        if self.builder.identity.is_some_and(|old| old != identity) {
            let open: Vec<usize> = self.builder.open.values().copied().collect();
            self.builder.open.clear();
            self.forget(&open, &[]);
        }
        self.builder.identity = Some(identity);
        self.builder.admitted = false;
    }

    /// Forgets `before` but `kept`, sorted: episodes no later hour steps.
    fn forget(&mut self, before: &[usize], kept: &[usize]) {
        for &index in before {
            if kept.binary_search(&index).is_err() {
                self.builder.episodes[index] = forgotten();
                self.builder.last_iterations.remove(&index);
            }
        }
    }

    /// The ordinal of the piece starting now, as its run holds it.
    pub(super) fn next_piece(&mut self) -> u64 {
        self.pieces += 1;
        self.pieces
    }

    /// The piece `ordinal` of the observation from `at` on, its progress as `facts` hold it,
    /// under dictionary revision `revision`: what its first hour adds to the ledger. Where its
    /// second hour may present an attribution, the piece waits for it ([`Self::second`]).
    pub(super) fn begin(
        &mut self,
        tag: &str,
        at: i32,
        ordinal: u64,
        facts: Vec<ProjectFacts>,
        revision: u64,
    ) -> io::Result<Vec<Entry>> {
        if self.lexical != Some(revision) {
            self.builder.content_ids.clear();
            self.lexical = Some(revision);
        }
        let (mut entries, second) = self.step(tag, at, &facts)?;
        let mut piece = Piece {
            ordinal,
            facts,
            extension: None,
            second: None,
        };
        if second && let Some(next) = at.checked_add(1) {
            piece.second = Some(next);
        } else {
            let extension = Extension::of(&self.builder, &piece.facts, at);
            entries.push(extension.entry(ordinal));
            piece.extension = Some(extension);
        }
        self.piece = Some(piece);
        Ok(entries)
    }

    /// The piece's second hour, `at`, where it waits for it: what it adds to the ledger.
    pub(super) fn second(&mut self, tag: &str, at: i32) -> io::Result<Vec<Entry>> {
        let Some(mut piece) = self.piece.take() else {
            return Ok(Vec::new());
        };
        if piece.second != Some(at) {
            self.piece = Some(piece);
            return Ok(Vec::new());
        }
        piece.second = None;
        let (mut entries, _) = self.step(tag, at, &piece.facts)?;
        let extension = Extension::of(&self.builder, &piece.facts, at);
        entries.push(extension.entry(piece.ordinal));
        piece.extension = Some(extension);
        self.piece = Some(piece);
        Ok(entries)
    }

    /// One hour presented, `at`: what it adds to the ledger, and whether a completion's dated
    /// attribution waits for the next hour.
    fn step(
        &mut self,
        tag: &str,
        at: i32,
        facts: &[ProjectFacts],
    ) -> io::Result<(Vec<Entry>, bool)> {
        let observation = self
            .observation
            .as_ref()
            .ok_or_else(|| invalid("A project piece has no observation"))?;
        let projects = observation
            .projects
            .as_deref()
            .ok_or_else(|| invalid("A project piece has no projects"))?;
        let builder = &mut self.builder;
        let before: Vec<usize> = builder.open.values().copied().collect();
        let episodes = builder.episodes.len();
        let presentations = builder.presentations.len();
        let people = builder.people.len();
        let iterations: Vec<i32> = builder.iteration_texts.keys().copied().collect();
        let warned = tally(&builder.warnings);
        let mut refusals = catalogue::Refusals::default();
        let second = head(
            builder,
            tag,
            observation.identity,
            projects,
            at,
            facts,
            &mut refusals,
        );
        let mut entries = Vec::new();
        entries.extend(
            builder.presentations[presentations..]
                .iter()
                .cloned()
                .map(Entry::Presentation),
        );
        entries.extend(builder.people[people..].iter().cloned().map(Entry::Person));
        for episode in &builder.episodes[episodes..] {
            entries.push(Entry::Episode {
                id: episode.id.clone(),
                start: episode.start.clone(),
            });
        }
        let mut stepped: Vec<usize> = builder.open.values().copied().collect();
        stepped.sort_unstable();
        stepped.dedup();
        for &index in &stepped {
            let runs = &builder.episodes[index].runs;
            let episode = u32::try_from(index).map_err(|_| invalid("Too many project episodes"))?;
            // An episode stepped before holds its last run alone, as the ledger has it; the
            // hour extends it or follows it.
            let from = if index < episodes {
                if runs[0].last_hour == at {
                    entries.push(Entry::Last(episode, at));
                }
                1
            } else {
                0
            };
            entries.extend(
                runs[from..]
                    .iter()
                    .cloned()
                    .map(|run| Entry::Run(episode, run)),
            );
        }
        if builder.iteration_texts.len() != iterations.len() {
            entries.push(Entry::Iterations(
                builder
                    .iteration_texts
                    .iter()
                    .filter(|(count, _)| iterations.binary_search(count).is_err())
                    .map(|(&count, text)| (count, text.clone()))
                    .collect(),
            ));
        }
        entries.push(Entry::Available(at));
        if tally(&builder.warnings) != warned {
            entries.push(Entry::Warnings(builder.warnings.clone()));
        }
        let added = self.refusals.merge_new(&refusals);
        if !added.is_empty() {
            entries.push(Entry::Refusals(added));
        }
        for &index in &stepped {
            let runs = &mut self.builder.episodes[index].runs;
            let kept = runs.len() - 1;
            runs.drain(..kept);
        }
        self.forget(&before, &stepped);
        let spans = self.builder.available.len();
        self.builder.available.drain(..spans - 1);
        Ok((entries, second))
    }

    /// Whether it holds nothing of a country.
    fn is_empty(&self) -> bool {
        self.pieces == 0 && self.observation.is_none() && self.builder.identity.is_none()
    }

    /// What it goes on with, where it holds anything.
    pub(super) fn save(&self) -> Option<Saved> {
        if self.is_empty() {
            return None;
        }
        let builder = &self.builder;
        Some(Saved {
            episodes: builder.episodes.len(),
            held: builder
                .open
                .values()
                .map(|&index| (index, builder.episodes[index].clone()))
                .collect(),
            open: builder
                .open
                .iter()
                .map(|(&key, &index)| (key, index))
                .collect(),
            identity: builder.identity,
            content_ids: builder
                .content_ids
                .iter()
                .map(|(key, &id)| (key.clone(), id))
                .collect(),
            presentations: builder.presentations.clone(),
            people: builder.people.clone(),
            available: builder.available.last().copied(),
            warnings: builder
                .warnings
                .iter()
                .map(|(reason, &count)| (reason.clone(), count))
                .collect(),
            iteration_texts: builder
                .iteration_texts
                .iter()
                .map(|(&count, text)| (count, text.clone()))
                .collect(),
            last_iterations: builder
                .last_iterations
                .iter()
                .map(|(&index, &count)| (index, count))
                .collect(),
            lexical: self.lexical,
            refusals: self.refusals.clone(),
            observation: self.observation.clone(),
            piece: self.piece.clone(),
            pieces: self.pieces,
        })
    }

    /// The track [`Self::save`] saved, its branch first recorded at `start`.
    pub(super) fn load(start: i32, saved: Saved) -> io::Result<Self> {
        let mut track = Self::new(start);
        let builder = &mut track.builder;
        builder.episodes = vec![forgotten(); saved.episodes];
        for (index, episode) in saved.held {
            *builder
                .episodes
                .get_mut(index)
                .ok_or_else(|| invalid("A saved project episode lies past the episodes"))? =
                episode;
        }
        builder.open = saved.open.into_iter().collect();
        builder.identity = saved.identity;
        builder.content_ids = saved.content_ids.into_iter().collect();
        builder.presentation_ids = (0..)
            .zip(&saved.presentations)
            .map(|(id, presentation)| (presentation.clone(), id))
            .collect();
        builder.presentations = saved.presentations;
        builder.person_ids = (0..)
            .zip(&saved.people)
            .map(|(id, person)| (person.clone(), id))
            .collect();
        builder.people = saved.people;
        builder.available = saved.available.into_iter().collect();
        builder.warnings = saved.warnings.into_iter().collect();
        builder.iteration_texts = saved.iteration_texts.into_iter().collect();
        builder.last_iterations = saved.last_iterations.into_iter().collect();
        track.lexical = saved.lexical;
        track.refusals = saved.refusals;
        track.observation = saved.observation;
        track.piece = saved.piece;
        track.pieces = saved.pieces;
        Ok(track)
    }
}

/// A track as [`Track::save`] keeps it.
#[derive(Serialize, Deserialize)]
pub(super) struct Saved {
    /// How many episodes it has built, and the ones a later hour may step.
    episodes: usize,
    held: Vec<(usize, ProjectEpisode)>,
    open: Vec<((u32, u32), usize)>,
    identity: Option<(u32, u32)>,
    content_ids: Vec<((u32, String), u32)>,
    presentations: Vec<ProjectPresentation>,
    people: Vec<ProjectPerson>,
    available: Option<HourSpan>,
    warnings: Vec<(String, usize)>,
    iteration_texts: Vec<(i32, Option<String>)>,
    last_iterations: Vec<(usize, i32)>,
    lexical: Option<u64>,
    refusals: catalogue::Refusals,
    observation: Option<Observation>,
    piece: Option<Piece>,
    pieces: u64,
}
