//! The ordered side of occupation's columnar preparation: it stitches the
//! projections' hours into the runs a branch publishes, holding values and
//! samples by id and integer, and hands each block's window to a thread that
//! builds and encodes it.
use super::*;
use crate::products::occupied_states::prepared::{EncodingWindow, HeldWindow, encoded_reply};
use dashboard_base::prepared::series::{Accumulator, Run, write_catalogue};

#[derive(Clone, Copy)]
struct StateRun {
    first_hour: i32,
    last_hour: i32,
    value: StateValue,
}

struct StateTrack {
    state: u32,
    runs: Vec<StateRun>,
}

/// An occupier's run, as [`OccupationTrendRun`] holds it.
struct TrendRun {
    first_hour: i32,
    last_hour: i32,
    read_hour: i32,
    starts_segment: bool,
    resistance: i64,
    compliance: i64,
    states: u32,
    held_states: u32,
    held_complete: bool,
    collaboration: Arc<[Collab]>,
}

/// One tag's occupier runs.
struct Track {
    tag: u32,
    actor: Option<u64>,
    runs: Vec<TrendRun>,
    last: Option<(Identity, i32)>,
    /// The fact of the run its read started this hour, encoded by the projection.
    fact: Option<Fact>,
    /// The last hour an occupier held a run under the tag.
    seen: Option<i32>,
    queued: bool,
}

/// What a block's window is built from, moved off the ordered thread.
struct Window {
    branch: u64,
    window: usize,
    interval: HourSpan,
    states: Vec<(u32, Vec<StateRun>)>,
    observed: Vec<OccupiedObservation>,
    laws: Vec<u32>,
    texts: Arc<Vec<Arc<str>>>,
    catalogue: Option<Arc<Catalogue>>,
    content: Result<Arc<installed_content_core::occupation_laws::Laws>, String>,
    warnings: Vec<String>,
    refused: Refusals,
    countries: Refusals,
    own: catalogue::OwnRefusals,
}

impl Window {
    /// The window page and the manifest after it, as the row path's
    /// `Prepared::hold_block` encodes them.
    fn encode(self) -> io::Result<HeldWindow> {
        let _cpu = ::replay_frame::cpu::scope("occupation window", None);
        let Self {
            branch,
            window,
            interval,
            mut states,
            observed,
            laws,
            texts,
            catalogue: cat,
            content,
            warnings,
            refused,
            countries,
            mut own,
        } = self;
        states.sort_unstable_by_key(|&(state, _)| state);
        let states: Vec<_> = states
            .into_iter()
            .map(|(state, runs)| StateRuns {
                state,
                runs: runs
                    .into_iter()
                    .map(|run| OccupiedRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        value: run.value.shown(&texts),
                    })
                    .collect(),
            })
            .collect();
        let branch_id = branch.to_string();
        let page = encoded_reply(&window_reply_of(
            &states, &observed, &branch_id, window, interval,
        ))?;
        let tokens: BTreeSet<String> = laws
            .iter()
            .map(|&law| texts[law as usize].to_string())
            .collect();
        let (laws, warnings) = catalogue::own_refusals(&mut own, || {
            manifest(
                cat.as_deref(),
                &content,
                &tokens,
                warnings,
                &refused,
                &countries,
            )
        });
        let listing = encoded_reply(&OccupiedStatesManifestReply {
            branch_id,
            decimals: 5,
            windows: Vec::new(),
            laws,
            strength_missing: STRENGTH_MISSING.into(),
            warnings,
        })?;
        Ok(HeldWindow {
            branch,
            window: window as u64,
            page,
            manifest: listing,
        })
    }
}

/// One branch's occupied states and occupier series from its projected
/// hours, published as the row path publishes them.
pub struct Emit {
    catalogue: Option<Arc<Catalogue>>,
    content: Result<Arc<installed_content_core::occupation_laws::Laws>, String>,
    warnings: Vec<String>,
    own: catalogue::OwnRefusals,
    shared: Arc<Texts>,
    values: [Accumulator; 2],
    ranks: [Accumulator; 2],
    facts: Accumulator,
    /// Each occupier tag's series key.
    actors: BTreeMap<String, u64>,
    first: Option<i32>,
    last: i32,
    next_window: usize,
    states: Vec<StateTrack>,
    positions: HashMap<u32, usize>,
    active: Vec<usize>,
    active_at: HashMap<u32, usize>,
    /// The States rows as the projections handed them on.
    carry: HashMap<u32, Shipped>,
    /// Each state's name and what it was read from, as last read.
    names: HashMap<u32, ([u8; 32], (Option<u32>, Option<u32>))>,
    laws: BTreeSet<u32>,
    observed: Vec<OccupiedObservation>,
    last_hour: Option<i32>,
    read_hour: Option<i32>,
    complete: bool,
    country_stamp: Option<[u8; 32]>,
    refused: Refusals,
    country_refused: Refusals,
    naming: Arc<Naming>,
    read_naming: Arc<Naming>,
    tracks: Vec<Track>,
    /// Each tag's track, by the tag's text id.
    track_of: Vec<usize>,
    active_tracks: Vec<usize>,
    changed: Vec<usize>,
    samples: Vec<Sample>,
    progress: Arc<Progress>,
    dirty: bool,
    unnamed: HashSet<Identity>,
    trend_refused: Refusals,
    collaboration_refused: Refusals,
}

/// `value` appended to `runs` at `hour`: the last run extended when it holds
/// the same value through the hour before.
fn append(runs: &mut Vec<StateRun>, hour: i32, value: StateValue) {
    if let Some(last) = runs.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
        && last.value == value
    {
        last.last_hour = hour;
    } else {
        runs.push(StateRun {
            first_hour: hour,
            last_hour: hour,
            value,
        });
    }
}

fn unnamed(seen: &mut HashSet<Identity>, refused: &mut Refusals, occupied: Identity) {
    if seen.insert(occupied) {
        refused.note(
            "occupied native country naming identity is unavailable",
            format!("slot {}, country {}", occupied.0, occupied.1),
        );
    }
}

/// `sample`'s collaboration at `at`, from the emit's progress: its occupied
/// countries by their tags, now or at the read.
#[allow(clippy::too_many_arguments)]
fn collaboration(
    sample: &Sample,
    at: i32,
    naming: &Naming,
    read_naming: &Naming,
    progress: &Progress,
    seen: &mut HashSet<Identity>,
    refused: &mut Refusals,
    texts: &[Arc<str>],
) -> Arc<[Collab]> {
    let mut entries = Vec::with_capacity(sample.occupied.len());
    for &occupied in sample.occupied.iter() {
        let current = naming.tags.get(&occupied).copied();
        if current.is_none() {
            unnamed(seen, refused, occupied);
        }
        let read = progress.get(&(occupied, sample.identity));
        entries.push(Collab {
            tag: current.or_else(|| read_naming.tags.get(&occupied).copied()),
            raw: read.map(|&(raw, _)| raw),
            read_hour: read.map(|&(_, hour)| hour),
            slot: occupied.0,
            country_id: occupied.1,
            name_hour: current.map(|_| at),
        });
    }
    let tag = |entry: &Collab| entry.tag.map(|id| &*texts[id as usize]);
    entries.sort_unstable_by(|a, b| tag(a).cmp(&tag(b)));
    entries.into()
}

/// `run`'s fact, as [`Accumulator::event`] encodes it.
fn fact(run: &TrendRun, texts: &[Arc<str>]) -> io::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&OccupationTrendRun {
        first_hour: run.first_hour,
        last_hour: run.last_hour,
        read_hour: run.read_hour,
        starts_segment: run.starts_segment,
        resistance_raw: run.resistance.to_string(),
        compliance_raw: run.compliance.to_string(),
        states: run.states,
        held_states: run.held_states,
        held_complete: run.held_complete,
        collaboration: run
            .collaboration
            .iter()
            .map(|entry| entry.shown(entry.tag.map(|id| texts[id as usize].to_string())))
            .collect(),
    })?)
}

impl Emit {
    pub(in crate::products::occupied_states) fn new(
        catalogue: Option<Arc<Catalogue>>,
        scan: &Scan<'_>,
        own: catalogue::OwnRefusals,
        shared: Arc<Texts>,
    ) -> Self {
        Self {
            catalogue,
            content: scan.laws.clone(),
            warnings: scan.result.warnings.clone(),
            own,
            shared,
            values: Default::default(),
            ranks: Default::default(),
            facts: Accumulator::default(),
            actors: BTreeMap::new(),
            first: None,
            last: 0,
            next_window: 0,
            states: Vec::new(),
            positions: HashMap::new(),
            active: Vec::new(),
            active_at: HashMap::new(),
            carry: HashMap::new(),
            names: HashMap::new(),
            laws: BTreeSet::new(),
            observed: Vec::new(),
            last_hour: None,
            read_hour: None,
            complete: false,
            country_stamp: None,
            refused: Refusals::default(),
            country_refused: Refusals::default(),
            naming: Arc::default(),
            read_naming: Arc::default(),
            tracks: Vec::new(),
            track_of: Vec::new(),
            active_tracks: Vec::new(),
            changed: Vec::new(),
            samples: Vec::new(),
            progress: Arc::default(),
            dirty: false,
            unnamed: HashSet::new(),
            trend_refused: Refusals::default(),
            collaboration_refused: Refusals::default(),
        }
    }

    pub fn observe(&mut self, hour: Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("occupation emit", None);
        let Hour {
            hour: at,
            valid,
            naming,
            whole,
            states,
            read,
            collaboration,
            ..
        } = hour;
        for values in &mut self.values {
            values.advance(at)?;
        }
        self.facts.advance(at)?;
        let previous = self.last_hour;
        if whole {
            self.carry.clear();
        }
        let mut edited = Vec::with_capacity(states.len());
        for (id, row) in states {
            edited.push(id);
            match row {
                Some(row) => {
                    self.carry.insert(id, row);
                }
                None => {
                    self.carry.remove(&id);
                }
            }
        }
        if previous.and_then(|hour| hour.checked_add(1)) != Some(at) {
            self.gap();
        }
        if !valid {
            self.gap();
            return self.publish(at);
        }
        self.last_hour = Some(at);
        let countries_changed = self.country_stamp != Some(naming.stamp);
        if countries_changed {
            self.country_stamp = Some(naming.stamp);
            self.country_refused.merge(&naming.refused);
        }
        self.naming = naming;
        match collaboration {
            Some(Collaboration::Unavailable) => {
                self.dirty |= !self.progress.is_empty();
                self.progress = Arc::default();
            }
            Some(Collaboration::Read(progress, refused)) => {
                self.dirty = true;
                self.progress = progress;
                self.collaboration_refused.merge(&refused);
            }
            None => {}
        }
        if let Some(read) = read {
            self.read(read, at, previous);
        } else if self.read_hour.is_some() {
            let ids = if countries_changed || whole {
                self.active_at.keys().copied().collect()
            } else {
                edited
            };
            self.project_states(ids, at, previous);
        }
        if let Some(read) = self.read_hour {
            self.project(at, read, countries_changed, previous);
            if let Some(last) = self.observed.last_mut()
                && last.last_hour.checked_add(1) == Some(at)
                && last.read_hour == read
                && last.complete == self.complete
            {
                last.last_hour = at;
            } else {
                self.observed.push(OccupiedObservation {
                    first_hour: at,
                    last_hour: at,
                    read_hour: read,
                    complete: self.complete,
                });
            }
        }
        self.publish(at)
    }

    fn read(&mut self, read: Read, at: i32, previous: Option<i32>) {
        self.own.merge(&read.own);
        catalogue::credit(&read.own);
        self.refused.merge(&read.refused);
        if read.reset {
            let present: HashSet<_> = read
                .values
                .iter()
                .filter(|(_, value)| value.is_some())
                .map(|&(id, _)| id)
                .collect();
            let gone: Vec<_> = self
                .active_at
                .keys()
                .filter(|id| !present.contains(*id))
                .copied()
                .collect();
            for id in gone {
                self.remove(id, previous);
            }
        }
        for (id, value) in read.values {
            let Some(value) = value else {
                self.remove(id, previous);
                continue;
            };
            if let Some(row) = self.carry.get(&id) {
                self.names
                    .insert(id, (row.name_key, (value.name, value.name_missing)));
            }
            self.laws.extend(value.law);
            let index = *self.positions.entry(id).or_insert_with(|| {
                self.states.push(StateTrack {
                    state: id,
                    runs: Vec::new(),
                });
                self.states.len() - 1
            });
            let active = self.active_at.contains_key(&id);
            if active && let Some(before) = previous {
                self.states[index]
                    .runs
                    .last_mut()
                    .expect("active state")
                    .last_hour = before;
            }
            append(&mut self.states[index].runs, at, value);
            if !active {
                self.active_at.insert(id, self.active.len());
                self.active.push(index);
            }
        }
        self.complete = read.complete;
        self.read_hour = Some(at);
        self.read_naming = Arc::clone(&self.naming);
        self.samples = read.samples;
        self.dirty = true;
        self.trend_refused.merge(&read.trend_refused);
        self.collaboration_refused
            .merge(&read.collaboration_refused);
    }

    /// The owner and name of each of `ids` the read holds, as the hour's
    /// States rows give them.
    fn project_states(&mut self, ids: Vec<u32>, at: i32, previous: Option<i32>) {
        for id in ids {
            let Some(&position) = self.active_at.get(&id) else {
                continue;
            };
            let index = self.active[position];
            let Some(row) = self.carry.get(&id) else {
                self.complete = false;
                self.refused.note(
                    "read occupation state absent from hourly States",
                    id.to_string(),
                );
                self.remove(id, previous);
                continue;
            };
            if row.owner.is_none()
                && let Some(refused) = &row.owner_refused
            {
                self.refused.note(&refused.0, &refused.1);
            }
            let name_changed = self
                .names
                .get(&id)
                .is_none_or(|(key, _)| *key != row.name_key);
            let last = self.states[index].runs.last_mut().expect("active state");
            if !name_changed && last.value.owner == row.owner {
                continue;
            }
            if let Some(before) = previous {
                last.last_hour = before;
            }
            let mut value = last.value;
            value.owner = row.owner;
            if name_changed {
                (value.name, value.name_missing) = row.name;
                self.names.insert(id, (row.name_key, row.name));
                self.own.merge(&row.named);
                catalogue::credit(&row.named);
                if let Some(reason) = row.name.1 {
                    self.refused
                        .note(&*self.shared.all()[reason as usize], format!("state {id}"));
                }
            }
            append(&mut self.states[index].runs, at, value);
        }
    }

    fn remove(&mut self, id: u32, previous: Option<i32>) {
        let Some(at) = self.active_at.remove(&id) else {
            return;
        };
        let index = self.active.swap_remove(at);
        if let Some(&moved) = self.active.get(at) {
            self.active_at.insert(self.states[moved].state, at);
        }
        if let Some(before) = previous {
            self.states[index]
                .runs
                .last_mut()
                .expect("active state")
                .last_hour = before;
        }
    }

    /// The occupiers' runs at `at` from the samples of the read at `read`.
    fn project(&mut self, at: i32, read: i32, names_changed: bool, previous: Option<i32>) {
        let _cpu = ::replay_frame::cpu::scope("occupation emit trend", None);
        if names_changed {
            let tags = &self.naming.tags;
            self.samples
                .retain(|sample| tags.contains_key(&sample.identity));
        }
        if !self.dirty && !names_changed {
            return;
        }
        if let Some(before) = previous {
            for &index in &self.active_tracks {
                let track = &mut self.tracks[index];
                if let Some((identity, last)) = track.last {
                    if let Some(run) = track.runs.last_mut()
                        && run.last_hour == last
                    {
                        run.last_hour = before;
                    }
                    track.last = Some((identity, before));
                }
            }
        }
        self.dirty = false;
        let fresh = read == at;
        let mut texts = None;
        let mut active = Vec::with_capacity(self.samples.len());
        for sample in &mut self.samples {
            let collaboration = match &sample.collaboration {
                Some(entries) if fresh => {
                    for entry in entries.iter().filter(|entry| entry.tag.is_none()) {
                        unnamed(
                            &mut self.unnamed,
                            &mut self.collaboration_refused,
                            (entry.slot, entry.country_id),
                        );
                    }
                    Arc::clone(entries)
                }
                _ => collaboration(
                    sample,
                    at,
                    &self.naming,
                    &self.read_naming,
                    &self.progress,
                    &mut self.unnamed,
                    &mut self.collaboration_refused,
                    texts.get_or_insert_with(|| self.shared.all()),
                ),
            };
            let Some(&tag) = self.naming.tags.get(&sample.identity) else {
                continue;
            };
            let at_tag = tag as usize;
            if self.track_of.len() <= at_tag {
                self.track_of.resize(at_tag + 1, usize::MAX);
            }
            if self.track_of[at_tag] == usize::MAX {
                self.track_of[at_tag] = self.tracks.len();
                self.tracks.push(Track {
                    tag,
                    actor: None,
                    runs: Vec::new(),
                    last: None,
                    fact: None,
                    seen: None,
                    queued: false,
                });
            }
            let index = self.track_of[at_tag];
            let track = &mut self.tracks[index];
            if track.seen != Some(at) {
                track.seen = Some(at);
                active.push(index);
            }
            let contiguous = track.last.is_some_and(|(seen, last)| {
                seen == sample.identity && last.checked_add(1) == Some(at)
            });
            if contiguous
                && let Some(last) = track.runs.last_mut()
                && last.read_hour == read
                && last.resistance == sample.resistance
                && last.compliance == sample.compliance
                && last.states == sample.states
                && last.held_states == sample.held_states
                && last.held_complete == sample.held_complete
                && *last.collaboration == *collaboration
            {
                last.last_hour = at;
                track.last = Some((sample.identity, at));
                continue;
            }
            track.runs.push(TrendRun {
                first_hour: at,
                last_hour: at,
                read_hour: read,
                starts_segment: !contiguous,
                resistance: sample.resistance,
                compliance: sample.compliance,
                states: sample.states,
                held_states: sample.held_states,
                held_complete: sample.held_complete,
                collaboration,
            });
            track.last = Some((sample.identity, at));
            track.fact = if fresh { sample.fact.take() } else { None };
            if !track.queued {
                track.queued = true;
                self.changed.push(index);
            }
        }
        for &index in &self.active_tracks {
            let track = &mut self.tracks[index];
            if track.seen != Some(at) && !track.queued {
                track.queued = true;
                self.changed.push(index);
            }
        }
        self.active_tracks = active;
        if fresh {
            for sample in &mut self.samples {
                sample.fact = None;
            }
        }
    }

    fn seal(&mut self) {
        let Some(hour) = self.last_hour else {
            return;
        };
        for &index in &self.active {
            if let Some(run) = self.states[index].runs.last_mut() {
                run.last_hour = hour;
            }
        }
        if self.read_hour.is_some()
            && let Some(span) = self.observed.last_mut()
        {
            span.last_hour = hour;
        }
        for &index in &self.active_tracks {
            let track = &mut self.tracks[index];
            if let Some((identity, before)) = track.last {
                if let Some(run) = track.runs.last_mut()
                    && run.last_hour == before
                {
                    run.last_hour = hour;
                }
                track.last = Some((identity, hour));
            }
        }
    }

    fn gap(&mut self) {
        self.seal();
        self.active.clear();
        self.active_at.clear();
        self.country_stamp = None;
        self.read_hour = None;
        for index in std::mem::take(&mut self.active_tracks) {
            let track = &mut self.tracks[index];
            if !track.queued {
                track.queued = true;
                self.changed.push(index);
            }
        }
        self.samples.clear();
        self.progress = Arc::default();
        self.dirty = true;
    }

    fn publish(&mut self, hour: i32) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("occupation emit publish", None);
        self.first.get_or_insert(hour);
        self.last = hour;
        let mut keyless: Vec<_> = self
            .changed
            .iter()
            .copied()
            .filter(|&index| self.tracks[index].actor.is_none())
            .collect();
        if !keyless.is_empty() {
            // New tags are keyed in tag order, as the row path keys them.
            let texts = self.shared.all();
            keyless.sort_unstable_by(|&a, &b| {
                texts[self.tracks[a].tag as usize].cmp(&texts[self.tracks[b].tag as usize])
            });
            for index in keyless {
                let actor = self.actors.len() as u64;
                self.actors
                    .insert(texts[self.tracks[index].tag as usize].to_string(), actor);
                self.tracks[index].actor = Some(actor);
            }
        }
        for index in std::mem::take(&mut self.changed) {
            let track = &mut self.tracks[index];
            track.queued = false;
            let actor = track.actor.expect("a published occupier has its key");
            let current = track.runs.last().filter(|run| run.last_hour == hour);
            for (column, values) in self.values.iter_mut().enumerate() {
                let Some(run) = current else {
                    values.observe(actor, hour, None, false);
                    continue;
                };
                let raw = i128::from(if column == 0 {
                    run.resistance
                } else {
                    run.compliance
                });
                let starts = run.first_hour == hour && run.starts_segment;
                let boundary = run.read_hour == hour;
                if starts
                    || boundary
                    || values
                        .state(actor)
                        .and_then(|state| state.last)
                        .map(|run| run.value)
                        != Some(raw)
                {
                    values.observe_boundary(actor, hour, Some(raw), starts, boundary);
                }
            }
            let Some(run) = current else {
                self.facts
                    .observe_if_changed(actor, hour, None, false, false);
                continue;
            };
            if run.first_hour == hour {
                let row = match track.fact.take() {
                    Some(encoded) => encoded.finish(run.starts_segment),
                    None => fact(run, &self.shared.all())?,
                };
                self.facts.event_raw(actor, hour, row)?;
            }
            self.facts.observe_if_changed(
                actor,
                hour,
                Some(i128::from(run.first_hour)),
                run.first_hour == hour && run.starts_segment,
                false,
            );
        }
        Ok(())
    }

    /// The window of the block of `branch` spanning `first_hour..=last_hour`,
    /// which another thread builds and encodes ([`EncodingWindow::write`]).
    pub fn hold_block(
        &mut self,
        branch: u64,
        first_hour: i32,
        last_hour: i32,
    ) -> io::Result<EncodingWindow> {
        let _cpu = ::replay_frame::cpu::scope("occupation emit hold", None);
        self.seal();
        let window = self.next_window;
        self.next_window += 1;
        let states = self
            .states
            .iter_mut()
            .map(|track| {
                let kept = track.runs.last().copied();
                let runs = std::mem::replace(&mut track.runs, kept.into_iter().collect());
                (track.state, runs)
            })
            .collect();
        let kept = self.observed.last().cloned();
        let observed = std::mem::replace(&mut self.observed, kept.into_iter().collect());
        let work = Window {
            branch,
            window,
            interval: HourSpan {
                first_hour,
                last_hour,
            },
            states,
            observed,
            laws: self.laws.iter().copied().collect(),
            texts: self.shared.all(),
            catalogue: self.catalogue.clone(),
            content: self.content.clone(),
            warnings: self.warnings.clone(),
            refused: self.refused.clone(),
            countries: self.country_refused.clone(),
            own: self.own.clone(),
        };
        EncodingWindow::new(move || work.encode())
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("occupation emit flush", None);
        let Some(first) = self.first.take() else {
            return Ok(());
        };
        self.seal();
        let texts = self.shared.all();
        let rows: BTreeMap<String, Vec<(i32, i32, &TrendRun)>> = self
            .tracks
            .iter()
            .filter(|track| !track.runs.is_empty())
            .map(|track| {
                (
                    texts[track.tag as usize].to_string(),
                    track
                        .runs
                        .iter()
                        .filter(|run| run.last_hour >= first)
                        .map(|run| (run.first_hour.max(first), run.last_hour, run))
                        .collect(),
                )
            })
            .collect();
        for column in 0..2 {
            self.ranks[column].through(first);
            dashboard_base::ranking::rank_positions(
                &rows,
                |&(first, last, _)| (first, last),
                |(_, _, run)| {
                    Some(i128::from(if column == 0 {
                        run.resistance
                    } else {
                        run.compliance
                    }))
                },
                cancelled,
                |rank, tag, from, to| {
                    let actor = self.actors[tag];
                    self.ranks[column].push(
                        rank as u64 * 2,
                        Run::new(from, to, i128::from(actor), false),
                    );
                    self.ranks[column]
                        .push(actor * 2 + 1, Run::new(from, to, rank as i128 + 1, false));
                    Ok(())
                },
            )?;
            self.ranks[column].through(self.last);
            self.ranks[column].flush(
                writer,
                &format!("occupation-trend-ranks-{branch}-{column}"),
                0,
                cancelled,
            )?;
            self.values[column].flush(
                writer,
                &format!("occupation-trend-values-{branch}-{column}"),
                5,
                cancelled,
            )?;
        }
        drop(rows);
        self.facts.flush(
            writer,
            &format!("occupation-trend-facts-{branch}"),
            0,
            cancelled,
        )?;
        write_catalogue(
            writer,
            "occupation-trend",
            branch,
            &self.actors,
            &(
                self.trend_refused.lines("recorded occupation trend facts"),
                self.collaboration_refused
                    .lines("recorded occupation trend facts"),
            ),
        )?;
        for track in &mut self.tracks {
            if track.runs.len() > 1 {
                track.runs.drain(..track.runs.len() - 1);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
impl Emit {
    /// `state`'s latest values, as the row path's scan holds them.
    pub(in crate::products::occupied_states) fn latest(
        &self,
        state: u32,
    ) -> Option<OccupiedValues> {
        let texts = self.shared.all();
        let run = self.states[*self.positions.get(&state)?].runs.last()?;
        Some(run.value.shown(&texts))
    }

    pub(in crate::products::occupied_states) fn read_spans(&self) -> &[OccupiedObservation] {
        &self.observed
    }

    /// The latest resistance mean of `tag`'s occupier.
    pub(in crate::products::occupied_states) fn resistance(&self, tag: &str) -> Option<i64> {
        let texts = self.shared.all();
        let track = self
            .tracks
            .iter()
            .find(|track| &*texts[track.tag as usize] == tag)?;
        Some(track.runs.last()?.resistance)
    }

    pub(in crate::products::occupied_states) fn keys(&self, tag: &str) -> bool {
        self.actors.contains_key(tag)
    }
}
