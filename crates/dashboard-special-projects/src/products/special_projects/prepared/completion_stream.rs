//! Completion changes share the topic's episode step; only open episodes resume.
use super::*;

pub const FAMILY: &str = "special-project-completions";
pub const KEY: u64 = 0;
pub(crate) const REFUSAL_FIRST_KEY: u64 = 1 << 63;
pub(crate) const UNMAPPED_PROJECT_OWNER: &str =
    "Project owner has no recorded CountryLinks mapping.";
pub(crate) const UNMAPPED_HISTORY_OWNER: &str =
    "Project history owner has no recorded CountryLinks mapping.";
pub fn section(branch: u64) -> String {
    format!("{FAMILY}-{branch}")
}
/// Events use key zero; each episode's runs hold its current events ordinal.
pub fn episode_key(id: u64) -> io::Result<u64> {
    id.checked_add(1)
        .filter(|key| *key < REFUSAL_FIRST_KEY)
        .ok_or_else(|| dashboard_core::invalid("Project completion episode key overflow"))
}

/// Append-only changes share a stable episode ID. A missing presentation
/// withdraws a previous bounded result when its newly learned native date
/// cannot be resolved; its named refusal remains visible.
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

#[derive(Default, Serialize, Deserialize)]
pub struct Metadata {
    pub late: series::LateIndex,
    pub available: bool,
    pub warnings: Vec<String>,
    pub content_error: Option<String>,
    /// Exact oracle reason strings, including content-defined diagnostics.
    pub refusal_keys: BTreeMap<String, u64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Contribution {
    warnings: Vec<(String, usize)>,
    texts: catalogue::Refusals,
}
#[derive(Default, Serialize, Deserialize)]
struct Diagnostics {
    counts: BTreeMap<String, usize>,
    texts: catalogue::Refusals,
    open: BTreeMap<u64, Contribution>,
}
#[derive(Default)]
struct DiagnosticStep {
    counts: BTreeMap<String, usize>,
    closed: Vec<u64>,
    changed: BTreeMap<u64, Contribution>,
}
impl Diagnostics {
    fn observe(&mut self, step: DiagnosticStep) {
        for (reason, count) in step.counts {
            *self.counts.entry(reason).or_default() += count;
        }
        self.open.extend(step.changed);
        for id in step.closed {
            if let Some(contribution) = self.open.remove(&id) {
                for (reason, count) in contribution.warnings {
                    *self.counts.entry(reason).or_default() += count;
                }
                self.texts.merge(&contribution.texts);
            }
        }
    }
    fn open_totals(
        &self,
        cancelled: &AtomicBool,
    ) -> io::Result<(BTreeMap<String, usize>, catalogue::Refusals)> {
        let mut counts = BTreeMap::new();
        let mut texts = catalogue::Refusals::default();
        for contribution in self.open.values() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            for (reason, count) in &contribution.warnings {
                *counts.entry(reason.clone()).or_default() += count;
            }
            texts.merge(&contribution.texts);
        }
        Ok((counts, texts))
    }
    fn totals(
        &self,
        open: &(BTreeMap<String, usize>, catalogue::Refusals),
        content_error: Option<&str>,
    ) -> BTreeMap<String, usize> {
        let mut counts = self.counts.clone();
        for (reason, count) in &open.0 {
            *counts.entry(reason.clone()).or_default() += count;
        }
        let mut texts = self.texts.clone();
        texts.merge(&open.1);
        if let Some(reason) = content_error {
            *counts.entry(reason.to_owned()).or_default() += 1;
        }
        completions::refusal_totals(counts, &texts)
            .into_iter()
            .collect()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Stream {
    values: Accumulator,
    active: BTreeSet<u64>,
    last_hour: Option<i32>,
    boundary: history::SpanBoundary,
    diagnostics: Diagnostics,
    source_counts: BTreeMap<String, usize>,
    actor_counts: BTreeMap<String, usize>,
    #[serde(with = "dashboard_core::map_pairs")]
    countries: BTreeMap<(u32, u32), Continuation>,
    next_id: u64,
    metadata: Metadata,
    #[serde(skip)]
    pending: Vec<Pending>,
    #[serde(skip)]
    first_hour: Option<i32>,
    #[serde(skip)]
    diagnostic_steps: BTreeMap<i32, DiagnosticStep>,
}

impl Stream {
    pub(super) fn restore_views(&mut self, frame: &DecodedFrame) {
        self.boundary.restore_views(frame);
    }
    pub(super) fn observe_inputs(
        &mut self,
        frame: &DecodedFrame,
        actors: &history::Actors,
        identities_changed: bool,
    ) -> bool {
        let changed = self
            .boundary
            .observe_changed(frame, actors, identities_changed);
        if !changed {
            return false;
        }
        if identities_changed {
            self.actor_counts.clear();
            if actors.unresolved != 0 {
                self.actor_counts.insert(
                    "Project country identity could not be resolved.".into(),
                    actors.unresolved,
                );
            }
            let invalid_tags = actors
                .tags
                .keys()
                .filter(|tag| std::str::from_utf8(tag).is_err())
                .count();
            if invalid_tags != 0 {
                self.actor_counts
                    .insert("Project country tag is non-UTF-8.".into(), invalid_tags);
            }
        }
        if !self.actor_counts.is_empty() {
            self.diagnostic_steps
                .entry(frame.hour)
                .or_default()
                .counts
                .extend(self.actor_counts.clone());
        }
        true
    }
    pub(super) fn set_source_count(&mut self, reason: &str, count: usize) {
        if count == 0 {
            self.source_counts.remove(reason);
        } else {
            self.source_counts.insert(reason.to_owned(), count);
        }
    }
    pub(super) fn reset_source_counts(&mut self) {
        self.source_counts.clear();
    }
    pub(super) fn repeat_source_counts(&mut self, hour: i32) {
        if self.source_counts.is_empty() {
            return;
        }
        let step = self.diagnostic_steps.entry(hour).or_default();
        for (reason, count) in &self.source_counts {
            *step.counts.entry(reason.clone()).or_default() += count;
        }
    }

    pub(super) fn through(&mut self, hour: i32) {
        // A completion remains current between captures until an explicit
        // change replaces or withdraws it, including recorded hour gaps.
        if self.last_hour.is_none() {
            self.diagnostic_steps.entry(hour).or_default();
        }
        self.first_hour.get_or_insert(
            self.last_hour
                .and_then(|last| last.checked_add(1))
                .unwrap_or(hour),
        );
        self.last_hour = Some(hour);
        self.boundary.advance(hour);
    }

    pub(super) fn observe(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        projects: &[(ProjectFacts, Look)],
        rows: &[&ProjectRow],
        span_changed: bool,
    ) {
        self.metadata.available = true;
        let (closed, warnings) = self.countries.entry(identity).or_default().observe(
            identity,
            hour,
            &projects
                .iter()
                .map(|(facts, _)| facts.clone())
                .collect::<Vec<_>>(),
            rows,
            &mut self.next_id,
            &mut self.pending,
        );
        if !closed.is_empty() || span_changed && !warnings.is_empty() {
            let step = self.diagnostic_steps.entry(hour).or_default();
            step.closed.extend(closed);
            if span_changed {
                for (reason, count) in warnings {
                    *self
                        .source_counts
                        .entry(completions::structural_reason(&reason))
                        .or_default() += count;
                    *step
                        .counts
                        .entry(completions::structural_reason(&reason))
                        .or_default() += count;
                }
            }
        }
    }

    pub(super) fn buffered_bytes(&self) -> usize {
        self.values
            .bytes()
            .saturating_add(
                self.pending
                    .len()
                    .saturating_mul(std::mem::size_of::<Pending>()),
            )
            .saturating_add(
                self.diagnostic_steps
                    .values()
                    .map(|step| {
                        std::mem::size_of::<DiagnosticStep>()
                            + step.closed.len() * 8
                            + step
                                .counts
                                .keys()
                                .map(|reason| reason.len() + 32)
                                .sum::<usize>()
                    })
                    .sum::<usize>(),
            )
    }

    fn extend_reference(values: &mut Accumulator, key: u64, through: i32) -> io::Result<()> {
        let run = values
            .state(key)
            .and_then(|state| state.last)
            .ok_or_else(|| {
                dashboard_core::invalid("Current project completion has no reference run")
            })?;
        if run.last_hour < through {
            values.push(
                key,
                series::Run::new(run.last_hour + 1, through, run.value, false),
            );
        }
        Ok(())
    }

    pub(super) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        replay: Option<&replay::container::Replay>,
        content: Option<&mut Content>,
        content_error: Option<&str>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.metadata.content_error = content_error.map(str::to_owned);
        if let Some(content) = content.as_ref() {
            let mut warnings = content.definitions.warnings.clone();
            warnings.extend(content.catalogue.verification.iter().cloned());
            self.metadata.warnings = warnings;
        }
        let first = self.first_hour.take();
        let write_references = !self.active.is_empty()
            || !self.pending.is_empty()
            || !self.metadata.refusal_keys.is_empty()
            || !self.diagnostic_steps.is_empty();
        if write_references && let Some(first) = first {
            self.values.through(first);
        }
        if !self.pending.is_empty() {
            let replay = replay.ok_or_else(|| {
                dashboard_core::invalid("New project completions require their preparation source")
            })?;
            let pending = std::mem::take(&mut self.pending);
            let mut episodes = Vec::with_capacity(pending.len());
            let mut last_change = BTreeMap::new();
            for (sequence, row) in pending.iter().enumerate() {
                episodes.push(row.id);
                last_change.insert((row.observation.observed_hour, row.id), sequence as u64);
            }
            let mut presented = completions::present_observations(
                replay,
                branch,
                pending
                    .into_iter()
                    .enumerate()
                    .map(|(sequence, row)| (sequence as u64, row.observation))
                    .collect(),
                content,
                cancelled,
            )?;
            // Restore recognition order after native-hour presentation, then
            // stably sort each supplying hour by its resolved feed tag.
            presented.sort_by_key(|row| row.id);
            presented.sort_by(|a, b| {
                a.observation
                    .observed_hour
                    .cmp(&b.observation.observed_hour)
                    .then_with(|| {
                        a.completion
                            .as_ref()
                            .map(|completion| completion.tag.as_str())
                            .cmp(
                                &b.completion
                                    .as_ref()
                                    .map(|completion| completion.tag.as_str()),
                            )
                    })
            });
            for row in presented {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let sequence = row.id;
                let id = episodes[sequence as usize];
                let observation = row.observation;
                let change = Change {
                    id,
                    identity: observation.identity,
                    local_id: observation.project.local_id,
                    definition_id: observation.project.definition_id,
                    program_reference: observation.project.program_reference,
                    date: observation.date,
                    observed_hour: observation.observed_hour,
                    completion: row.completion,
                    warnings: row.warnings.into_iter().collect(),
                    refusals: row.refusals,
                };
                let ordinal = self.values.state(KEY).map_or(0, |state| state.events);
                self.values.event(KEY, change.observed_hour, &change)?;
                self.metadata
                    .late
                    .insert(change.date.hour, change.observed_hour, KEY, ordinal);
                // Publication tag order can separate or reorder an episode's
                // changes. Its final recognised change still owns the Live value.
                if last_change[&(change.observed_hour, change.id)] != sequence {
                    continue;
                }
                self.diagnostic_steps
                    .entry(change.observed_hour)
                    .or_default()
                    .changed
                    .insert(
                        change.id,
                        Contribution {
                            warnings: change.warnings.clone(),
                            texts: change.refusals.clone(),
                        },
                    );
                let key = episode_key(change.id)?;
                if self.active.contains(&key)
                    && let Some(before) = change.observed_hour.checked_sub(1)
                {
                    Self::extend_reference(&mut self.values, key, before)?;
                }
                if change.completion.is_some() {
                    self.values.observe(
                        key,
                        change.observed_hour,
                        Some(i128::from(ordinal)),
                        false,
                    );
                    self.active.insert(key);
                } else {
                    self.active.remove(&key);
                }
            }
        }
        let mut open_totals = None;
        for (hour, step) in std::mem::take(&mut self.diagnostic_steps) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let changed = !step.changed.is_empty() || !step.closed.is_empty();
            self.diagnostics.observe(step);
            // Structural rates alter scalar counts only. Do not reconstruct
            // unchanged completion contributions at each supplying boundary.
            if changed || open_totals.is_none() {
                open_totals = Some(self.diagnostics.open_totals(cancelled)?);
            }
            let totals = self
                .diagnostics
                .totals(open_totals.as_ref().unwrap(), content_error);
            for reason in totals.keys() {
                if !self.metadata.refusal_keys.contains_key(reason) {
                    let key = REFUSAL_FIRST_KEY
                        .checked_add(self.metadata.refusal_keys.len() as u64)
                        .ok_or_else(|| dashboard_core::invalid("Project refusal key overflow"))?;
                    self.metadata.refusal_keys.insert(reason.clone(), key);
                }
            }
            for (reason, &key) in &self.metadata.refusal_keys {
                dashboard_text::metadata::check_cancel(cancelled)?;
                if self.values.state(key).is_some()
                    && let Some(before) = hour.checked_sub(1)
                {
                    Self::extend_reference(&mut self.values, key, before)?;
                }
                self.values.observe(
                    key,
                    hour,
                    Some(totals.get(reason).copied().unwrap_or(0) as i128),
                    false,
                );
            }
        }
        if write_references && let Some(last) = self.last_hour {
            for &key in &self.active {
                dashboard_text::metadata::check_cancel(cancelled)?;
                Self::extend_reference(&mut self.values, key, last)?;
            }
            for &key in self.metadata.refusal_keys.values() {
                dashboard_text::metadata::check_cancel(cancelled)?;
                Self::extend_reference(&mut self.values, key, last)?;
            }
            if first.is_some() {
                self.values.through(last);
            }
        }
        self.values.flush(writer, &section(branch), 0, cancelled)?;
        series::write_catalogue(
            writer,
            FAMILY,
            branch,
            &BTreeMap::from([("completions".into(), KEY)]),
            &self.metadata,
        )
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Continuation {
    episodes: Vec<ProjectEpisode>,
    ids: Vec<u64>,
    #[serde(with = "dashboard_core::map_pairs")]
    open: BTreeMap<(u32, u32), usize>,
    iterations: BTreeMap<usize, i32>,
    /// The oracle keeps the first completion's program selector when an exact
    /// native date arrives later. Keep that operand, never its raw project row.
    programs: BTreeMap<usize, u64>,
    dates: BTreeMap<usize, ProjectDate>,
}

struct Pending {
    id: u64,
    observation: CompletionObservation,
}

impl Continuation {
    fn observe(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        facts: &[ProjectFacts],
        rows: &[&ProjectRow],
        next_id: &mut u64,
        pending: &mut Vec<Pending>,
    ) -> (Vec<u64>, BTreeMap<String, usize>) {
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
        builder.observe_projects(hour, facts, |_, ordinal, index, _, _, completion| {
            if index == ids.len() {
                ids.push(*next_id);
                *next_id += 1;
            }
            if let Some(date) = completion
                && dates.get(&index) != Some(date)
            {
                dates.insert(index, date.clone());
                let mut project = rows[ordinal].clone();
                project.program_reference =
                    *programs.entry(index).or_insert(project.program_reference);
                pending.push(Pending {
                    id: ids[index],
                    observation: CompletionObservation {
                        project,
                        identity,
                        date: date.clone(),
                        observed_hour: hour,
                    },
                });
            }
            0
        });
        let kept: BTreeSet<_> = builder.open.values().copied().collect();
        let closed = ids
            .iter()
            .enumerate()
            .filter_map(|(index, &id)| (!kept.contains(&index)).then_some(id))
            .collect();
        let warnings = std::mem::take(&mut builder.warnings);
        // Finished and removed episodes are already events. Reindex only the
        // still-open projections, with one compact run and their stable IDs.
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
        (closed, warnings)
    }
}
