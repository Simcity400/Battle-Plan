//! The ordered side of occupation's columnar preparation: it stitches the projections' hours
//! into the family's series publication.
//!
//! - Each occupied state: its values' version, a run per change ([`STATES`], by state), the
//!   values an event at each version's first hour ([`kind::VALUE`]).
//! - The reads supplying them ([`OBSERVED`]): the read's hour and whether its inventory was
//!   complete.
//! - Each occupier tag (an actor): its resistance and compliance means, a supplying read
//!   starting their runs ([`MEANS`]); its runs' facts, keyed by each run's first hour, each run
//!   an event of its JSON ([`FACTS`], [`kind::FACT`]); its rank by each mean ([`RANKS`]); and
//!   each rank's occupier by each mean ([`POSITIONS`]).
//! - Scalars: the manifest's laws and warnings ([`MANIFEST`]), the trend's warnings
//!   ([`TREND`]) and every text the values name, by id ([`TEXTS`]).
use super::*;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};

/// The family's publication and keys are spelled under these.
pub const NAME: &str = "occupation";
pub const SCHEMA: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family keys: each state's values' version (by state id), the supplying reads, and each
/// rank's occupier (by rank) by resistance and by compliance.
pub const STATES: u64 = 0;
pub const OBSERVED: u64 = 1;
pub const POSITIONS: [u64; 2] = [2, 3];
/// An occupier's keys: its means by resistance and compliance, its runs' facts, and its rank
/// (1 is first) by each.
pub const MEANS: [u64; 2] = [0, 1];
pub const FACTS: u64 = 2;
pub const RANKS: [u64; 2] = [3, 4];
/// Scalars: the manifest's laws and warnings and the trend's warnings, each as JSON, and each
/// text by id from [`TEXTS`].
pub const MANIFEST: u64 = 0;
pub const TREND: u64 = 1;
pub const TEXTS: u64 = 1 << 24;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// The fixed5 decimals of the means.
const DECIMALS: u8 = 5;

/// Event kinds.
pub mod kind {
    /// A state's values, [`super::VALUE_CELLS`] cells: name, name missing, owner, occupier and
    /// occupied (texts); resistance, compliance and their speeds; garrison strength, required
    /// manpower and required template amount; the alert, the red flag and assigned manpower;
    /// garrison missing, law and law missing (texts). A text is its id, every absent value Null.
    pub const VALUE: u64 = 0;
    /// An occupier's run as [`crate::products::occupation_trend::OccupationTrendRun`] JSON.
    pub const FACT: u64 = 1;
}

/// The cells of a [`kind::VALUE`] event.
pub const VALUE_CELLS: u64 = 18;

/// The manifest's laws and warnings, as [`MANIFEST`] holds them.
#[derive(Default, Serialize, Deserialize)]
pub(in crate::products::occupied_states) struct Listing {
    pub laws: Vec<OccupationLawInfo>,
    pub warnings: Vec<String>,
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved occupation emit is not valid")
}

/// An occupier's run, as [`OccupationTrendRun`] holds it.
#[derive(Clone)]
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

/// One tag's occupier runs: the latest only.
struct Track {
    tag: u32,
    actor: Option<u64>,
    current: Option<TrendRun>,
    last: Option<(Identity, i32)>,
    /// The fact of the run its read started this hour, encoded by the projection.
    fact: Option<Fact>,
    /// The last hour an occupier held a run under the tag.
    seen: Option<i32>,
    queued: bool,
    /// Whether its means hold a run through the hour it was last published.
    open: bool,
}

/// An occupier's keys.
#[derive(Clone, Copy)]
struct ActorKeys {
    means: [KeyHandle; 2],
    facts: KeyHandle,
    ranks: [KeyHandle; 2],
}

/// One branch's occupied states and occupier series from its projected hours.
pub struct Emit {
    catalogue: Option<Arc<Catalogue>>,
    content: Result<Arc<installed_content_core::occupation_laws::Laws>, String>,
    warnings: Vec<String>,
    own: catalogue::OwnRefusals,
    shared: Arc<Texts>,
    actors: Actors,
    carry: Carry,
    /// Each published text's id, and the id of each projection text published so far.
    published: HashMap<Arc<str>, u64>,
    ids: Vec<u64>,
    /// The keys of each occupier, by actor.
    keys: Vec<Option<ActorKeys>>,
    /// The last values' version.
    versions: u64,
    /// The occupied states, with their values.
    active: HashMap<u32, StateValue>,
    /// The States rows as the projections handed them on.
    rows: HashMap<u32, Shipped>,
    /// Each state's name and what it was read from, as last read.
    names: HashMap<u32, ([u8; 32], (Option<u32>, Option<u32>))>,
    laws: BTreeSet<u32>,
    /// The last hour walked, and the last valid one.
    last: Option<i32>,
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
    /// Each rank's occupier by each mean, each occupier's rank (u32::MAX for none), and the
    /// positions' keys.
    ranking: [Vec<u64>; 2],
    ranked: [Vec<u32>; 2],
    positions: [Vec<KeyHandle>; 2],
    /// The first hour since the last cut.
    first: Option<i32>,
    /// The manifest and trend warnings last published.
    written: [Option<String>; 2],
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

/// `run`'s fact, as the projection encodes it.
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
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            published: HashMap::new(),
            ids: Vec::new(),
            keys: Vec::new(),
            versions: 0,
            active: HashMap::new(),
            rows: HashMap::new(),
            names: HashMap::new(),
            laws: BTreeSet::new(),
            last: None,
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
            ranking: Default::default(),
            ranked: Default::default(),
            positions: Default::default(),
            first: None,
            written: [None, None],
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place, decimals: u8) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, decimals), Some(DOMAIN))
    }

    fn family_key(
        &mut self,
        writer: &mut Writer<'_>,
        measure: u64,
        dimension: Option<u64>,
    ) -> io::Result<KeyHandle> {
        let place = Place {
            actor: FAMILY,
            measure,
            dimension,
        };
        self.key(writer, place, 0)
    }

    /// The published id of projection text `id`.
    fn text(&mut self, writer: &mut Writer<'_>, id: u32) -> io::Result<u64> {
        let at = id as usize;
        if let Some(&held) = self.ids.get(at)
            && held != u64::MAX
        {
            return Ok(held);
        }
        let texts = self.shared.all();
        let text = &texts[at];
        let published = match self.published.get(text) {
            Some(&published) => published,
            None => {
                let published = self.published.len() as u64;
                let cell = family::cell(writer, &Scalar::Text(text.as_bytes().into()))?;
                writer.catalogue(TEXTS + published, cell)?;
                self.published.insert(Arc::clone(text), published);
                published
            }
        };
        if self.ids.len() <= at {
            self.ids.resize(at + 1, u64::MAX);
        }
        self.ids[at] = published;
        Ok(published)
    }

    fn text_cell(&mut self, writer: &mut Writer<'_>, id: Option<u32>) -> io::Result<Cell> {
        Ok(match id {
            Some(id) => Cell::Unsigned(self.text(writer, id)?),
            None => Cell::Null,
        })
    }

    /// The cells of a [`kind::VALUE`] event holding `value`.
    fn value_cells(&mut self, writer: &mut Writer<'_>, value: &StateValue) -> io::Result<Vec<Cell>> {
        let integer = |value: Option<i64>| value.map_or(Cell::Null, |value| Cell::Integer(i128::from(value)));
        Ok(vec![
            self.text_cell(writer, value.name)?,
            self.text_cell(writer, value.name_missing)?,
            self.text_cell(writer, value.owner)?,
            self.text_cell(writer, value.occupier)?,
            self.text_cell(writer, value.occupied)?,
            Cell::Integer(i128::from(value.resistance_raw)),
            Cell::Integer(i128::from(value.compliance_raw)),
            Cell::Integer(i128::from(value.resistance_speed_raw)),
            Cell::Integer(i128::from(value.compliance_speed_raw)),
            integer(value.garrison_strength_raw),
            integer(value.required_manpower.map(i64::from)),
            integer(value.garrison_required_raw),
            Cell::Boolean(value.garrison_alert),
            value.garrison_red.map_or(Cell::Null, Cell::Boolean),
            integer(value.assigned_manpower.map(i64::from)),
            self.text_cell(writer, value.garrison_missing)?,
            self.text_cell(writer, value.law)?,
            self.text_cell(writer, value.law_missing)?,
        ])
    }

    /// `id`'s values from `at` on: a change starts a version, with its values as an event.
    fn set(
        &mut self,
        writer: &mut Writer<'_>,
        id: u32,
        at: i32,
        value: StateValue,
    ) -> io::Result<()> {
        if self.active.get(&id) == Some(&value) {
            return Ok(());
        }
        let key = self.family_key(writer, STATES, Some(u64::from(id)))?;
        self.versions += 1;
        self.carry
            .value(writer, key, at, Some(i128::from(self.versions)), false)?;
        let cells = self.value_cells(writer, &value)?;
        let fields = writer.cells(cells)?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: None,
            sequence,
            kind: kind::VALUE,
            fields,
        })?;
        self.active.insert(id, value);
        Ok(())
    }

    /// `id` is no longer occupied from `at` on.
    fn remove(&mut self, writer: &mut Writer<'_>, id: u32, at: i32) -> io::Result<()> {
        if self.active.remove(&id).is_some() {
            let key = self.family_key(writer, STATES, Some(u64::from(id)))?;
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: Hour) -> io::Result<()> {
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
        match self.last {
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "An occupation hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) if last.checked_add(1) != Some(at) => self.carry.close(writer, last)?,
            _ => {}
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        let previous = self.last_hour;
        if whole {
            self.rows.clear();
        }
        let mut edited = Vec::with_capacity(states.len());
        for (id, row) in states {
            edited.push(id);
            match row {
                Some(row) => {
                    self.rows.insert(id, row);
                }
                None => {
                    self.rows.remove(&id);
                }
            }
        }
        if previous.and_then(|hour| hour.checked_add(1)) != Some(at) {
            self.gap(writer, at)?;
        }
        if !valid {
            self.gap(writer, at)?;
            return self.publish(writer, at);
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
            self.read(writer, read, at)?;
        } else if self.read_hour.is_some() {
            let ids = if countries_changed || whole {
                self.active.keys().copied().collect()
            } else {
                edited
            };
            self.project_states(writer, ids, at)?;
        }
        if let Some(read) = self.read_hour {
            self.project(at, read, countries_changed, previous);
            let key = self.family_key(writer, OBSERVED, None)?;
            let observed = i128::from(read) * 2 + i128::from(self.complete);
            self.carry.value(writer, key, at, Some(observed), false)?;
        }
        self.publish(writer, at)
    }

    fn read(&mut self, writer: &mut Writer<'_>, read: Read, at: i32) -> io::Result<()> {
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
                .active
                .keys()
                .filter(|id| !present.contains(*id))
                .copied()
                .collect();
            for id in gone {
                self.remove(writer, id, at)?;
            }
        }
        for (id, value) in read.values {
            let Some(value) = value else {
                self.remove(writer, id, at)?;
                continue;
            };
            if let Some(row) = self.rows.get(&id) {
                self.names
                    .insert(id, (row.name_key, (value.name, value.name_missing)));
            }
            self.laws.extend(value.law);
            self.set(writer, id, at, value)?;
        }
        self.complete = read.complete;
        self.read_hour = Some(at);
        self.read_naming = Arc::clone(&self.naming);
        self.samples = read.samples;
        self.dirty = true;
        self.trend_refused.merge(&read.trend_refused);
        self.collaboration_refused
            .merge(&read.collaboration_refused);
        Ok(())
    }

    /// The owner and name of each of `ids` the read holds, as the hour's
    /// States rows give them.
    fn project_states(&mut self, writer: &mut Writer<'_>, ids: Vec<u32>, at: i32) -> io::Result<()> {
        for id in ids {
            let Some(&held) = self.active.get(&id) else {
                continue;
            };
            let Some(row) = self.rows.get(&id) else {
                self.complete = false;
                self.refused.note(
                    "read occupation state absent from hourly States",
                    id.to_string(),
                );
                self.remove(writer, id, at)?;
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
            if !name_changed && held.owner == row.owner {
                continue;
            }
            let mut value = held;
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
            self.set(writer, id, at, value)?;
        }
        Ok(())
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
                    if let Some(run) = track.current.as_mut()
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
            let index = track_index(&mut self.tracks, &mut self.track_of, tag);
            let track = &mut self.tracks[index];
            if track.seen != Some(at) {
                track.seen = Some(at);
                active.push(index);
            }
            let contiguous = track.last.is_some_and(|(seen, last)| {
                seen == sample.identity && last.checked_add(1) == Some(at)
            });
            if contiguous
                && let Some(last) = track.current.as_mut()
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
            track.current = Some(TrendRun {
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

    /// The hours before `at` end every run: it follows an hour not walked, or an hour that
    /// could not be read, or cannot be read itself.
    fn gap(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        if let Some(hour) = self.last_hour {
            for &index in &self.active_tracks {
                let track = &mut self.tracks[index];
                if let Some((identity, before)) = track.last {
                    if let Some(run) = track.current.as_mut()
                        && run.last_hour == before
                    {
                        run.last_hour = hour;
                    }
                    track.last = Some((identity, hour));
                }
            }
        }
        if let Some(before) = at.checked_sub(1) {
            self.carry.close(writer, before)?;
        }
        self.active.clear();
        self.country_stamp = None;
        self.read_hour = None;
        for index in std::mem::take(&mut self.active_tracks) {
            let track = &mut self.tracks[index];
            if !track.queued {
                track.queued = true;
                self.changed.push(index);
            }
        }
        for track in &mut self.tracks {
            track.open = false;
        }
        for column in 0..2 {
            self.ranking[column].clear();
            self.ranked[column].fill(u32::MAX);
        }
        self.samples.clear();
        self.progress = Arc::default();
        self.dirty = true;
        Ok(())
    }

    /// The actor of `tag`, with its keys, catalogued at `hour` when new.
    fn occupier(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if self.keys.len() <= index {
            self.keys.resize(index + 1, None);
        }
        if self.keys[index].is_none() {
            let place = |measure| Place {
                actor,
                measure,
                dimension: None,
            };
            let keys = ActorKeys {
                means: [
                    self.key(writer, place(MEANS[0]), DECIMALS)?,
                    self.key(writer, place(MEANS[1]), DECIMALS)?,
                ],
                facts: self.key(writer, place(FACTS), 0)?,
                ranks: [
                    self.key(writer, place(RANKS[0]), 0)?,
                    self.key(writer, place(RANKS[1]), 0)?,
                ],
            };
            self.keys[index] = Some(keys);
        }
        Ok(actor)
    }

    fn publish(&mut self, writer: &mut Writer<'_>, hour: i32) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("occupation emit publish", None);
        if self.changed.is_empty() {
            return Ok(());
        }
        let texts = self.shared.all();
        let mut keyless: Vec<_> = self
            .changed
            .iter()
            .copied()
            .filter(|&index| self.tracks[index].actor.is_none())
            .collect();
        // New tags are numbered in tag order.
        keyless.sort_unstable_by(|&a, &b| {
            texts[self.tracks[a].tag as usize].cmp(&texts[self.tracks[b].tag as usize])
        });
        for index in keyless {
            let tag = Arc::clone(&texts[self.tracks[index].tag as usize]);
            let actor = self.occupier(writer, &tag, hour)?;
            self.tracks[index].actor = Some(actor);
        }
        for index in std::mem::take(&mut self.changed) {
            let track = &mut self.tracks[index];
            track.queued = false;
            let actor = track.actor.expect("a published occupier has its key");
            let opened = track.open;
            let current = track
                .current
                .as_ref()
                .filter(|run| run.last_hour == hour)
                .cloned();
            let encoded = match &current {
                Some(run) if run.first_hour == hour => Some(match track.fact.take() {
                    Some(encoded) => encoded.finish(run.starts_segment),
                    None => fact(run, &texts)?,
                }),
                _ => None,
            };
            track.open = current.is_some();
            let keys = self.keys[actor as usize].ok_or_else(invalid)?;
            let Some(run) = current else {
                for key in keys.means.into_iter().chain([keys.facts]) {
                    self.carry.value(writer, key, hour, None, false)?;
                }
                continue;
            };
            let starts = run.first_hour == hour && run.starts_segment || !opened;
            let supplying = run.read_hour == hour;
            for (key, raw) in keys.means.into_iter().zip([run.resistance, run.compliance]) {
                let raw = i128::from(raw);
                if supplying {
                    self.carry.supplied(writer, key, hour, raw, starts)?;
                } else {
                    self.carry.value(writer, key, hour, Some(raw), starts)?;
                }
            }
            if let Some(encoded) = encoded {
                let text = writer.blob(&mut encoded.as_slice(), encoded.len() as u64)?;
                let fields = writer.cells([Cell::Text(text)])?;
                let sequence = writer.sequence()?;
                writer.event(Event {
                    key: keys.facts,
                    native_hour: hour,
                    read_hour: Some(run.read_hour),
                    sequence,
                    kind: kind::FACT,
                    fields,
                })?;
            }
            self.carry.value(
                writer,
                keys.facts,
                hour,
                Some(i128::from(run.first_hour)),
                starts,
            )?;
        }
        self.rank(writer, hour, &texts)
    }

    /// Each mean's ranking of the occupiers holding a run at `hour`, highest first, ties by
    /// tag: the positions and ranks that moved go on from `hour`.
    fn rank(&mut self, writer: &mut Writer<'_>, hour: i32, texts: &[Arc<str>]) -> io::Result<()> {
        for column in 0..2 {
            let mut order: Vec<(i64, &str, u64)> = self
                .tracks
                .iter()
                .filter(|track| track.open)
                .filter_map(|track| {
                    let run = track.current.as_ref()?;
                    let value = if column == 0 {
                        run.resistance
                    } else {
                        run.compliance
                    };
                    Some((value, &*texts[track.tag as usize], track.actor?))
                })
                .collect();
            order.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
            let order: Vec<u64> = order.into_iter().map(|(_, _, actor)| actor).collect();
            let held = std::mem::take(&mut self.ranking[column]);
            for position in 0..held.len().max(order.len()) {
                let (was, now) = (held.get(position), order.get(position));
                if was == now {
                    continue;
                }
                let key = match self.positions[column].get(position) {
                    Some(&key) => key,
                    None => {
                        let key =
                            self.family_key(writer, POSITIONS[column], Some(position as u64))?;
                        self.positions[column].push(key);
                        key
                    }
                };
                self.carry
                    .value(writer, key, hour, now.map(|&actor| i128::from(actor)), false)?;
            }
            let ranked = &mut self.ranked[column];
            let count = self.actors.len() as usize;
            if ranked.len() < count {
                ranked.resize(count, u32::MAX);
            }
            let mut now = vec![u32::MAX; ranked.len()];
            for (position, &actor) in order.iter().enumerate() {
                now[actor as usize] = position as u32;
            }
            let mut moved = Vec::new();
            for &actor in held.iter().chain(&order) {
                let at = actor as usize;
                if ranked[at] != now[at] {
                    ranked[at] = now[at];
                    moved.push((actor, now[at]));
                }
            }
            for (actor, position) in moved {
                let key = self.keys[actor as usize].ok_or_else(invalid)?.ranks[column];
                let rank = (position != u32::MAX).then(|| i128::from(position) + 1);
                self.carry.value(writer, key, hour, rank, false)?;
            }
            self.ranking[column] = order;
        }
        Ok(())
    }

    /// The manifest's laws and warnings now, as JSON.
    fn listing(&self) -> io::Result<String> {
        let texts = self.shared.all();
        let tokens: BTreeSet<String> = self
            .laws
            .iter()
            .map(|&law| texts[law as usize].to_string())
            .collect();
        let mut own = self.own.clone();
        let (laws, warnings) = catalogue::own_refusals(&mut own, || {
            manifest(
                self.catalogue.as_deref(),
                &self.content,
                &tokens,
                self.warnings.clone(),
                &self.refused,
                &self.country_refused,
            )
        });
        Ok(serde_json::to_string(&Listing { laws, warnings })?)
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("occupation emit cut", None);
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
        let trend = serde_json::to_string(&(
            self.trend_refused.lines("recorded occupation trend facts"),
            self.collaboration_refused
                .lines("recorded occupation trend facts"),
        ))?;
        for (at, (id, text)) in [(MANIFEST, self.listing()?), (TREND, trend)]
            .into_iter()
            .enumerate()
        {
            if self.written[at].as_deref() != Some(text.as_str()) {
                let cell = family::cell(writer, &Scalar::Text(text.as_bytes().into()))?;
                writer.catalogue(id, cell)?;
                self.written[at] = Some(text);
            }
        }
        Ok(())
    }

    /// What the emit goes on with after a cut, its texts spelled out. A restart goes on at a
    /// segment's first hour, whose projection hands on every States row and the naming anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let texts = self.shared.all();
        let text = |id: u32| texts[id as usize].to_string();
        let collab = |entries: &[Collab]| {
            entries
                .iter()
                .map(|entry| SavedCollab {
                    tag: entry.tag.map(text),
                    raw: entry.raw,
                    read_hour: entry.read_hour,
                    slot: entry.slot,
                    country_id: entry.country_id,
                    name_hour: entry.name_hour,
                })
                .collect::<Vec<_>>()
        };
        let mut active: Vec<_> = self
            .active
            .iter()
            .map(|(&id, value)| (id, value.shown(&texts)))
            .collect();
        active.sort_unstable_by_key(|&(id, _)| id);
        let mut read_naming: Vec<_> = self
            .read_naming
            .tags
            .iter()
            .map(|(&identity, &tag)| (identity, text(tag)))
            .collect();
        read_naming.sort_unstable();
        let mut progress: Vec<_> = self.progress.iter().map(|(&key, &read)| (key, read)).collect();
        progress.sort_unstable();
        let saved = Saved {
            last: self.last,
            last_hour: self.last_hour,
            read_hour: self.read_hour,
            complete: self.complete,
            country_stamp: self.country_stamp,
            versions: self.versions,
            own: self.own.clone(),
            refused: self.refused.clone(),
            country_refused: self.country_refused.clone(),
            trend_refused: self.trend_refused.clone(),
            collaboration_refused: self.collaboration_refused.clone(),
            laws: self.laws.iter().map(|&law| text(law)).collect(),
            active,
            read_naming: SavedNaming {
                stamp: self.read_naming.stamp,
                refused: self.read_naming.refused.clone(),
                tags: read_naming,
            },
            tracks: self
                .tracks
                .iter()
                .map(|track| SavedTrack {
                    tag: text(track.tag),
                    current: track.current.as_ref().map(|run| SavedRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        read_hour: run.read_hour,
                        starts_segment: run.starts_segment,
                        resistance: run.resistance,
                        compliance: run.compliance,
                        states: run.states,
                        held_states: run.held_states,
                        held_complete: run.held_complete,
                        collaboration: collab(&run.collaboration[..]),
                    }),
                    last: track.last,
                    seen: track.seen,
                    open: track.open,
                })
                .collect(),
            active_tracks: self.active_tracks.iter().map(|&index| index as u64).collect(),
            samples: self
                .samples
                .iter()
                .map(|sample| SavedSample {
                    identity: sample.identity,
                    resistance: sample.resistance,
                    compliance: sample.compliance,
                    states: sample.states,
                    held_states: sample.held_states,
                    held_complete: sample.held_complete,
                    occupied: sample.occupied.to_vec(),
                    collaboration: sample.collaboration.as_deref().map(collab),
                })
                .collect(),
            progress,
            dirty: self.dirty,
            unnamed: {
                let mut unnamed: Vec<_> = self.unnamed.iter().copied().collect();
                unnamed.sort_unstable();
                unnamed
            },
            ranking: self.ranking.clone(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads, its
    /// texts named anew in `shared`.
    pub(in crate::products::occupied_states) fn load(
        catalogue: Option<Arc<Catalogue>>,
        scan: &Scan<'_>,
        shared: Arc<Texts>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut emit = Self::new(catalogue, scan, Default::default(), shared);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut id = 0;
        while let Some(scalar) = directory.scalar(TEXTS + id) {
            let text = std::str::from_utf8(scalar.text().ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
            emit.published.insert(Arc::from(text), id);
            id += 1;
        }
        for (at, measure) in [MANIFEST, TREND].into_iter().enumerate() {
            emit.written[at] = directory
                .scalar(measure)
                .and_then(Scalar::text)
                .map(|text| String::from_utf8_lossy(text).into_owned());
        }
        emit.keys = (0..directory.actors.len() as u64)
            .map(|actor| {
                let key = |measure| directory.key(actor, measure, None);
                Some(ActorKeys {
                    means: [key(MEANS[0])?, key(MEANS[1])?],
                    facts: key(FACTS)?,
                    ranks: [key(RANKS[0])?, key(RANKS[1])?],
                })
            })
            .collect();
        for column in 0..2 {
            emit.positions[column] = directory
                .dimensions(FAMILY, POSITIONS[column])
                .iter()
                .enumerate()
                .map(|(at, &(position, key))| {
                    (position == at as u64).then_some(key).ok_or_else(invalid)
                })
                .collect::<io::Result<_>>()?;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        let mut named = Named {
            shared: Arc::clone(&emit.shared),
            ids: HashMap::new(),
        };
        let collab = |entries: Vec<SavedCollab>, named: &mut Named| -> Arc<[Collab]> {
            entries
                .into_iter()
                .map(|entry| Collab {
                    tag: entry.tag.map(|tag| named.id(&tag)),
                    raw: entry.raw,
                    read_hour: entry.read_hour,
                    slot: entry.slot,
                    country_id: entry.country_id,
                    name_hour: entry.name_hour,
                })
                .collect()
        };
        emit.last = saved.last;
        emit.last_hour = saved.last_hour;
        emit.read_hour = saved.read_hour;
        emit.complete = saved.complete;
        emit.country_stamp = saved.country_stamp;
        emit.versions = saved.versions;
        emit.own = saved.own;
        emit.refused = saved.refused;
        emit.country_refused = saved.country_refused;
        emit.trend_refused = saved.trend_refused;
        emit.collaboration_refused = saved.collaboration_refused;
        emit.laws = saved.laws.iter().map(|law| named.id(law)).collect();
        emit.active = saved
            .active
            .iter()
            .map(|(id, value)| (*id, StateValue::of(value, &mut named)))
            .collect();
        emit.read_naming = Arc::new(Naming {
            stamp: saved.read_naming.stamp,
            refused: saved.read_naming.refused,
            tags: saved
                .read_naming
                .tags
                .iter()
                .map(|(identity, tag)| (*identity, named.id(tag)))
                .collect(),
        });
        for track in saved.tracks {
            let tag = named.id(&track.tag);
            let index = track_index(&mut emit.tracks, &mut emit.track_of, tag);
            let held = &mut emit.tracks[index];
            held.actor = directory.actor(track.tag.as_bytes());
            held.current = match track.current {
                Some(run) => Some(TrendRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    read_hour: run.read_hour,
                    starts_segment: run.starts_segment,
                    resistance: run.resistance,
                    compliance: run.compliance,
                    states: run.states,
                    held_states: run.held_states,
                    held_complete: run.held_complete,
                    collaboration: collab(run.collaboration, &mut named),
                }),
                None => None,
            };
            held.last = track.last;
            held.seen = track.seen;
            held.open = track.open;
            if held.open && held.actor.is_none() {
                return Err(invalid());
            }
        }
        emit.active_tracks = saved
            .active_tracks
            .into_iter()
            .map(|index| {
                usize::try_from(index)
                    .ok()
                    .filter(|&index| index < emit.tracks.len())
                    .ok_or_else(invalid)
            })
            .collect::<io::Result<_>>()?;
        emit.samples = saved
            .samples
            .into_iter()
            .map(|sample| Sample {
                identity: sample.identity,
                resistance: sample.resistance,
                compliance: sample.compliance,
                states: sample.states,
                held_states: sample.held_states,
                held_complete: sample.held_complete,
                occupied: sample.occupied.into(),
                collaboration: sample
                    .collaboration
                    .map(|entries| collab(entries, &mut named)),
                fact: None,
            })
            .collect();
        emit.progress = Arc::new(saved.progress.into_iter().collect());
        emit.dirty = saved.dirty;
        emit.unnamed = saved.unnamed.into_iter().collect();
        for column in 0..2 {
            let ranked = &mut emit.ranked[column];
            ranked.resize(directory.actors.len(), u32::MAX);
            for (position, &actor) in saved.ranking[column].iter().enumerate() {
                *ranked.get_mut(actor as usize).ok_or_else(invalid)? = position as u32;
            }
        }
        emit.ranking = saved.ranking;
        Ok(emit)
    }
}

/// The track of `tag`, made when it has none.
fn track_index(tracks: &mut Vec<Track>, track_of: &mut Vec<usize>, tag: u32) -> usize {
    let at = tag as usize;
    if track_of.len() <= at {
        track_of.resize(at + 1, usize::MAX);
    }
    if track_of[at] == usize::MAX {
        track_of[at] = tracks.len();
        tracks.push(Track {
            tag,
            actor: None,
            current: None,
            last: None,
            fact: None,
            seen: None,
            queued: false,
            open: false,
        });
    }
    track_of[at]
}

#[derive(Serialize, Deserialize)]
struct SavedCollab {
    tag: Option<String>,
    raw: Option<i64>,
    read_hour: Option<i32>,
    slot: u32,
    country_id: u32,
    name_hour: Option<i32>,
}

#[derive(Serialize, Deserialize)]
struct SavedRun {
    first_hour: i32,
    last_hour: i32,
    read_hour: i32,
    starts_segment: bool,
    resistance: i64,
    compliance: i64,
    states: u32,
    held_states: u32,
    held_complete: bool,
    collaboration: Vec<SavedCollab>,
}

#[derive(Serialize, Deserialize)]
struct SavedTrack {
    tag: String,
    current: Option<SavedRun>,
    last: Option<(Identity, i32)>,
    seen: Option<i32>,
    open: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedSample {
    identity: Identity,
    resistance: i64,
    compliance: i64,
    states: u32,
    held_states: u32,
    held_complete: bool,
    occupied: Vec<Identity>,
    collaboration: Option<Vec<SavedCollab>>,
}

#[derive(Serialize, Deserialize)]
struct SavedNaming {
    stamp: [u8; 32],
    refused: Refusals,
    tags: Vec<(Identity, String)>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    last_hour: Option<i32>,
    read_hour: Option<i32>,
    complete: bool,
    country_stamp: Option<[u8; 32]>,
    versions: u64,
    own: catalogue::OwnRefusals,
    refused: Refusals,
    country_refused: Refusals,
    trend_refused: Refusals,
    collaboration_refused: Refusals,
    laws: Vec<String>,
    active: Vec<(u32, OccupiedValues)>,
    read_naming: SavedNaming,
    tracks: Vec<SavedTrack>,
    active_tracks: Vec<u64>,
    samples: Vec<SavedSample>,
    progress: Vec<((Identity, Identity), (i64, i32))>,
    dirty: bool,
    unnamed: Vec<Identity>,
    ranking: [Vec<u64>; 2],
}

#[cfg(test)]
impl Emit {
    /// `state`'s latest values, while it is occupied.
    pub(in crate::products::occupied_states) fn latest(
        &self,
        state: u32,
    ) -> Option<OccupiedValues> {
        let texts = self.shared.all();
        Some(self.active.get(&state)?.shown(&texts))
    }
}
