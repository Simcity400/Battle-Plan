//! National focus' emit: each tag's presence and focus order on the family's series publication
//! (presence runs with an event at each order change, its completion chain, its progress and
//! daily gain), the four tables' refusals, the completions the event feed lists with their art,
//! and the countries and completions without an observation or a date, by reason.
//!
//! A tag is present while one country holds it at consecutive hours; another country, an hour
//! unobserved or an hour not walked ends it. Its order is dated where it extends the order
//! observed the hour before in the same tree; any other change (a removal, a reorder, another
//! tree), which a reset can hide behind, or an order first read after an hour unobserved or for
//! another country, publishes its focuses undated. Progress runs from each change of the raw
//! progress; its daily gain is the rise since the hour before the run while the same focus is
//! active.

use super::{
    RESET, TableRefusal, UNDATED,
    art::{CompletionArt, Presenter},
    facts,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's keys: its presence (1 while observed) with its order events, its completion chain
/// rows, its raw progress and its daily gain.
pub const ORDER: u64 = 0;
pub const CHAIN: u64 = 1;
pub const PROGRESS: u64 = 2;
pub const GAIN: u64 = 3;
/// The family's keys: the refusals holding (by their set's id) and the completions.
pub const REFUSALS: u64 = 0;
pub const COMPLETIONS: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: whether any tag was observed, the countries and completions without an
/// observation or a date by reason, and each refusal set as JSON, by id.
pub const OBSERVED: u64 = 0;
pub const UNAVAILABLE: u64 = 1;
pub const REASONS: u64 = 1 << 20;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[Unsigned slot, Unsigned id, Integer orderFirstHour, Text | Null current, Text | Null
    /// continuous, Boolean paused, Unsigned chain, Unsigned len]`, on a tag's ORDER key.
    pub const ORDER: u64 = 0;
    /// `[Unsigned ordinal, Text token, Integer completedByHour | Null]`, on a tag's CHAIN key.
    pub const ENTRY: u64 = 1;
    /// `[Unsigned actor, Text token, Text art | Null]`, on the family's COMPLETIONS key: its
    /// art the MessagePack of a [`super::super::art::CompletionArt`], none without content.
    pub const COMPLETION: u64 = 2;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved focus emit is not valid")
}

/// Text as replies show it: bytes that are not UTF-8 show their replacement text.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A tag's published order: its country, the first hour of this order's consecutive run, its
/// texts, pause, and its completed focuses as the first `len` rows of its chain from `chain`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub slot: u32,
    pub country_id: u32,
    pub order_first_hour: i32,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
    pub chain: u64,
    pub len: u64,
}

/// A tag's run of equal progress: its first hour and raw progress, the observation the hour
/// before it (raw progress, its hour and active focus; none when it began this country's
/// observations) and the active focus now.
#[derive(Clone)]
struct Progress {
    first: i32,
    raw: i64,
    previous: Option<(i64, i32, Option<Arc<[u8]>>)>,
    current: Option<Arc<[u8]>>,
}

impl Progress {
    /// The rise since the observation before the run, when that was the hour before it and the
    /// same focus is active.
    fn gain(&self) -> Option<i128> {
        let (raw, last, current) = self.previous.as_ref()?;
        if last.checked_add(1) != Some(self.first)
            || self.current.is_none()
            || self.current != *current
        {
            return None;
        }
        Some(i128::from(self.raw) - i128::from(*raw)).filter(|gain| *gain > 0)
    }
}

/// What a tag carries, and its keys.
#[derive(Clone, Default)]
struct Track {
    /// ORDER, CHAIN, PROGRESS and GAIN.
    keys: [KeyHandle; 4],
    /// Its last country, whether that country was observed at the emit's last hour, and the
    /// last hour it was otherwise.
    identity: Option<(u32, u32)>,
    live: bool,
    last: Option<i32>,
    /// The order last read, none before the first or after another country's.
    order: Option<Arc<facts::Order>>,
    order_first_hour: i32,
    chain: u64,
    rows: u64,
    event: Option<Published>,
    progress: Option<Progress>,
}

/// A completion the hour read: its tag, its place in the order and its token.
type Completion = (u64, usize, String);

/// What completions the walk did not present are presented from: the recording, and the content
/// it names.
pub struct Recorded<'s> {
    pub source: &'s crate::Source,
    pub catalogue: &'s Arc<crate::catalogue::Catalogue>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each actor's name, and what each tag carries, by actor.
    labels: Vec<Arc<[u8]>>,
    tracks: Vec<Track>,
    /// The tags observed at the last hour.
    live: HashSet<u64>,
    /// REFUSALS and COMPLETIONS.
    family: Option<[KeyHandle; 2]>,
    /// Each refusal set's id, by its JSON, and the set holding.
    sets: HashMap<Vec<u8>, u64>,
    refusing: Option<u64>,
    /// The countries without an observation each hour, by reason, and every hour's total with
    /// the completions without a date.
    tally: Vec<(String, u64)>,
    unavailable: BTreeMap<String, u64>,
    observed: bool,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    presenter: Presenter,
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            labels: Vec::new(),
            tracks: Vec::new(),
            live: HashSet::new(),
            family: None,
            sets: HashMap::new(),
            refusing: None,
            tally: Vec::new(),
            unavailable: BTreeMap::new(),
            observed: false,
            stale: true,
            last: None,
            first: None,
            presenter: Presenter::default(),
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 2]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([REFUSALS, COMPLETIONS]) {
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

    /// The tag actor named `tag`, with its keys.
    fn country(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.tracks.len() {
            self.tracks.resize(index + 1, Track::default());
            self.labels.resize(index + 1, Arc::from(&[][..]));
        }
        self.labels[index] = Arc::from(tag.as_bytes());
        let mut keys = [0; 4];
        for (key, measure) in keys.iter_mut().zip([ORDER, CHAIN, PROGRESS, GAIN]) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.tracks[index].keys = keys;
        Ok(actor)
    }

    /// Keeps one projected hour, its completions presented with `recorded`'s content.
    pub fn hour(
        &mut self,
        writer: &mut Writer<'_>,
        hour: facts::Hour,
        recorded: Option<&Recorded<'_>>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("focus.emit", None);
        let (branch, at) = (hour.branch, hour.hour);
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A focus hour does not follow the hour before",
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
        let [refusals, completions_key] = self.family(writer)?;
        let mut completions: Vec<Completion> = Vec::new();
        let mut presented = Vec::new();
        let mut reasons_changed = false;
        match hour.facts {
            Some(facts) => {
                let facts::Facts {
                    refusals: read,
                    tally,
                    whole,
                    tags,
                    art,
                } = *facts;
                presented = art;
                if !adjacent && !whole {
                    return Err(dashboard_core::invalid(
                        "A focus hour after one not walked does not name every tag",
                    ));
                }
                if let Some(read) = read {
                    let id = self.set(writer, &read)?;
                    reasons_changed = self.refusing != Some(id);
                    self.refusing = Some(id);
                }
                if let Some(tally) = tally {
                    self.tally = tally;
                }
                if whole {
                    let named: HashSet<&[u8]> =
                        tags.iter().map(|(tag, _)| tag.as_bytes()).collect();
                    let mut gone: Vec<u64> = self
                        .live
                        .iter()
                        .copied()
                        .filter(|&actor| !named.contains(&self.labels[actor as usize][..]))
                        .collect();
                    gone.sort_unstable();
                    for actor in gone {
                        self.absent(writer, at, actor)?;
                    }
                }
                for (tag, observed) in tags {
                    let actor = self.country(writer, &tag, at)?;
                    match observed {
                        Some(observed) => {
                            self.observe(writer, at, actor, &observed, &mut completions)?
                        }
                        None => self.absent(writer, at, actor)?,
                    }
                }
            }
            None if !adjacent => {
                return Err(dashboard_core::invalid(
                    "A focus hour after one not walked holds no facts",
                ));
            }
            None => {}
        }
        if let Some(id) = self.refusing
            && (reasons_changed || !adjacent)
        {
            self.carry
                .value(writer, refusals, at, Some(i128::from(id)), reasons_changed)?;
        }
        for (reason, count) in &self.tally {
            match self.unavailable.get_mut(reason) {
                Some(total) => *total = total.saturating_add(*count),
                None => {
                    self.unavailable.insert(reason.clone(), *count);
                }
            }
            self.stale = true;
        }
        // Each hour's completions in tag order, then by their place in the order.
        let labels = &self.labels;
        completions.sort_unstable_by(|left, right| {
            labels[left.0 as usize]
                .cmp(&labels[right.0 as usize])
                .then(left.1.cmp(&right.1))
        });
        let art = match recorded {
            Some(recorded) => self.art(recorded, (branch, at), &completions, presented)?,
            None => vec![None; completions.len()],
        };
        for ((actor, _, token), art) in completions.into_iter().zip(art) {
            let token = writer.text(&token)?;
            let art = match art {
                Some(art) => {
                    Cell::Text(writer.bytes(&dashboard_base::prepared::codec::encode(&art)?)?)
                }
                None => Cell::Null,
            };
            let fields = writer.cells([Cell::Unsigned(actor), Cell::Text(token), art])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: completions_key,
                native_hour: at,
                read_hour: Some(at),
                sequence,
                kind: kind::COMPLETION,
                fields,
            })?;
        }
        Ok(())
    }

    /// The art of `completions`, dated at `at` of `branch`: what the walk presented of each, the
    /// others presented on the frame sought from the recording, as one batch.
    fn art(
        &mut self,
        recorded: &Recorded<'_>,
        (branch, at): (u64, i32),
        completions: &[Completion],
        mut presented: Vec<facts::Presented>,
    ) -> io::Result<Vec<Option<CompletionArt>>> {
        let mut art = Vec::with_capacity(completions.len());
        let mut missing = Vec::new();
        for (index, (actor, position, token)) in completions.iter().enumerate() {
            let tag = &self.labels[*actor as usize];
            let found = presented.iter().position(|held| {
                held.position == *position
                    && held.tag.as_bytes() == &tag[..]
                    && held.token == *token
            });
            match found {
                Some(found) => art.push(Some(presented.swap_remove(found).art)),
                None => {
                    art.push(None);
                    missing.push(index);
                }
            }
        }
        if missing.is_empty() {
            return Ok(art);
        }
        let tags: Vec<String> = missing
            .iter()
            .map(|&index| lossy(&self.labels[completions[index].0 as usize]))
            .collect();
        let asked: Vec<(&str, &str)> = missing
            .iter()
            .zip(&tags)
            .map(|(&index, tag)| (completions[index].2.as_str(), tag.as_str()))
            .collect();
        let shown = self.presenter.present_sought(
            recorded.source,
            recorded.catalogue,
            (branch, at),
            &asked,
        )?;
        for (index, shown) in missing.into_iter().zip(shown) {
            art[index] = Some(shown);
        }
        Ok(art)
    }

    /// The id of the refusal set `read`, catalogued when it is new.
    fn set(&mut self, writer: &mut Writer<'_>, read: &[TableRefusal]) -> io::Result<u64> {
        let json = serde_json::to_vec(read)?;
        if let Some(&id) = self.sets.get(&json) {
            return Ok(id);
        }
        let id = self.sets.len() as u64;
        let text = family::cell(writer, &Scalar::Text(json.clone().into_boxed_slice()))?;
        writer.catalogue(REASONS + id, text)?;
        self.sets.insert(json, id);
        Ok(id)
    }

    /// `actor` is not observed from `at`: its runs end.
    fn absent(&mut self, writer: &mut Writer<'_>, at: i32, actor: u64) -> io::Result<()> {
        let track = &mut self.tracks[actor as usize];
        if !track.live {
            return Ok(());
        }
        track.live = false;
        track.last = at.checked_sub(1);
        let keys = [track.keys[ORDER as usize], track.keys[PROGRESS as usize], track.keys[GAIN as usize]];
        self.live.remove(&actor);
        for key in keys {
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// `actor` observed at `at` as `observed`.
    fn observe(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        actor: u64,
        observed: &facts::Observation,
        completions: &mut Vec<Completion>,
    ) -> io::Result<()> {
        if !self.observed {
            self.observed = true;
            self.stale = true;
        }
        let identity = (observed.slot, observed.country_id);
        let track = &self.tracks[actor as usize];
        let same = track.identity == Some(identity);
        // Observed the hour before as the same country.
        let continuing = same && track.live;
        let [order_key, _, progress_key, gain_key] = track.keys;
        if !continuing {
            self.carry.value(writer, order_key, at, Some(1), true)?;
        }
        let changed = !continuing
            || track
                .order
                .as_ref()
                .is_none_or(|held| !Arc::ptr_eq(held, &observed.order) && **held != *observed.order);
        if changed {
            self.order(writer, at, actor, observed, continuing, same, completions)?;
        }
        let track = &mut self.tracks[actor as usize];
        let current = observed.order.current.clone();
        match track.progress.as_mut().filter(|_| continuing) {
            Some(progress) => {
                if progress.raw != observed.progress {
                    progress.previous = Some((progress.raw, at - 1, progress.current.clone()));
                    progress.first = at;
                    progress.raw = observed.progress;
                }
                progress.current = current;
            }
            None => {
                let previous = match (&track.progress, track.last) {
                    (Some(progress), Some(last)) if same => {
                        Some((progress.raw, last, progress.current.clone()))
                    }
                    _ => None,
                };
                track.progress = Some(Progress {
                    first: at,
                    raw: observed.progress,
                    previous,
                    current,
                });
            }
        }
        let progress = track.progress.as_ref().expect("a progress just set");
        let (raw, gain) = (i128::from(progress.raw), progress.gain());
        track.identity = Some(identity);
        track.live = true;
        self.live.insert(actor);
        self.carry
            .value(writer, progress_key, at, Some(raw), !continuing)?;
        self.carry.value(writer, gain_key, at, gain, false)?;
        Ok(())
    }

    /// The order `observed` read at `at`: its new completions dated where it extends the order
    /// read before in the same tree, its chain restarted undated where it does not, and its event
    /// where what it publishes changed.
    #[allow(clippy::too_many_arguments)]
    fn order(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        actor: u64,
        observed: &facts::Observation,
        continuing: bool,
        same: bool,
        completions: &mut Vec<Completion>,
    ) -> io::Result<()> {
        let track = &mut self.tracks[actor as usize];
        let [order_key, chain_key, _, _] = track.keys;
        if !same {
            track.order = None;
        }
        let order = &observed.order;
        let tokens = &order.completed;
        let before = track.order.take();
        let extends = before.as_ref().is_some_and(|before| {
            before.tree == order.tree
                && before.completed.len() <= tokens.len()
                && before.completed.iter().zip(tokens.iter()).all(|(a, b)| a == b)
        });
        let unchanged = extends
            && before.as_ref().is_some_and(|before| {
                before.completed.len() == tokens.len()
                    && before.paused == order.paused
                    && before.current == order.current
                    && before.continuous == order.continuous
            });
        let mut rows: Vec<(usize, Option<i32>)> = Vec::new();
        let held = match &before {
            Some(before) if extends => before.completed.len(),
            before => {
                if before.is_some() {
                    *self.unavailable.entry(RESET.to_owned()).or_default() += 1;
                    self.stale = true;
                }
                track.chain = track.rows;
                rows.extend((0..tokens.len()).map(|position| (position, None)));
                tokens.len()
            }
        };
        let mut undated = 0_u64;
        for position in held..tokens.len() {
            rows.push((position, Some(at)));
            if continuing {
                completions.push((actor, position, lossy(&tokens[position])));
            } else {
                undated += 1;
            }
        }
        if undated > 0 {
            *self.unavailable.entry(UNDATED.to_owned()).or_default() += undated;
            self.stale = true;
        }
        for (position, dated) in rows {
            let token = writer.text(&lossy(&tokens[position]))?;
            let fields = writer.cells([
                Cell::Unsigned(position as u64),
                Cell::Text(token),
                dated.map_or(Cell::Null, |hour| Cell::Integer(i128::from(hour))),
            ])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: chain_key,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::ENTRY,
                fields,
            })?;
            track.rows += 1;
        }
        if !(continuing && unchanged) {
            track.order_first_hour = at;
        }
        let published = Published {
            slot: observed.slot,
            country_id: observed.country_id,
            order_first_hour: track.order_first_hour,
            current: order.current.as_deref().map(lossy),
            continuous: order.continuous.as_deref().map(lossy),
            paused: order.paused,
            chain: track.chain,
            len: tokens.len() as u64,
        };
        if track.event.as_ref() != Some(&published) {
            let text = |writer: &mut Writer<'_>, text: &Option<String>| -> io::Result<Cell> {
                Ok(match text {
                    Some(text) => Cell::Text(writer.text(text)?),
                    None => Cell::Null,
                })
            };
            let current = text(writer, &published.current)?;
            let continuous = text(writer, &published.continuous)?;
            let fields = writer.cells([
                Cell::Unsigned(u64::from(published.slot)),
                Cell::Unsigned(u64::from(published.country_id)),
                Cell::Integer(i128::from(published.order_first_hour)),
                current,
                continuous,
                Cell::Boolean(published.paused),
                Cell::Unsigned(published.chain),
                Cell::Unsigned(published.len),
            ])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: order_key,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::ORDER,
                fields,
            })?;
            track.event = Some(published);
        }
        track.order = Some(Arc::clone(order));
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for actor in self.live.drain() {
            let track = &mut self.tracks[actor as usize];
            track.live = false;
            track.last = Some(last);
        }
        Ok(())
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
            let unavailable = Scalar::List(
                self.unavailable
                    .iter()
                    .map(|(reason, &count)| {
                        Scalar::List(vec![
                            Scalar::Text(reason.as_bytes().into()),
                            Scalar::Unsigned(count),
                        ])
                    })
                    .collect(),
            );
            for (id, scalar) in [
                (OBSERVED, Scalar::Boolean(self.observed)),
                (UNAVAILABLE, unavailable),
            ] {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            refusing: self.refusing,
            tally: self.tally.clone(),
            unavailable: self.unavailable.clone(),
            observed: self.observed,
            tracks: self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, track)| track.identity.is_some())
                .map(|(actor, track)| SavedTrack {
                    actor: actor as u64,
                    identity: track.identity,
                    live: track.live,
                    last: track.last,
                    order: track.order.as_deref().map(|order| SavedOrder {
                        tree: order.tree.as_deref().map(<[u8]>::to_vec),
                        completed: order.completed.iter().map(|token| token.to_vec()).collect(),
                        current: order.current.as_deref().map(<[u8]>::to_vec),
                        continuous: order.continuous.as_deref().map(<[u8]>::to_vec),
                        paused: order.paused,
                    }),
                    order_first_hour: track.order_first_hour,
                    chain: track.chain,
                    rows: track.rows,
                    event: track.event.clone(),
                    progress: track.progress.as_ref().map(|progress| SavedProgress {
                        first: progress.first,
                        raw: progress.raw,
                        previous: progress.previous.as_ref().map(|(raw, hour, current)| {
                            (*raw, *hour, current.as_deref().map(<[u8]>::to_vec))
                        }),
                        current: progress.current.as_deref().map(<[u8]>::to_vec),
                    }),
                })
                .collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.labels = directory
            .actors
            .iter()
            .map(|actor| Arc::from(&actor.name[..]))
            .collect();
        emit.tracks = vec![Track::default(); directory.actors.len()];
        for (actor, track) in emit.tracks.iter_mut().enumerate() {
            for (key, measure) in track.keys.iter_mut().zip([ORDER, CHAIN, PROGRESS, GAIN]) {
                *key = directory
                    .key(actor as u64, measure, None)
                    .ok_or_else(invalid)?;
            }
        }
        let mut family = [0; 2];
        for (key, measure) in family.iter_mut().zip([REFUSALS, COMPLETIONS]) {
            *key = directory.key(FAMILY, measure, None).ok_or_else(invalid)?;
        }
        emit.family = Some(family);
        let mut id = 0;
        while let Some(scalar) = directory.scalar(REASONS + id) {
            emit.sets
                .insert(scalar.text().ok_or_else(invalid)?.to_vec(), id);
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.refusing = saved.refusing;
        if saved.refusing.is_some_and(|id| id >= emit.sets.len() as u64) {
            return Err(invalid());
        }
        emit.tally = saved.tally;
        emit.unavailable = saved.unavailable;
        emit.observed = saved.observed;
        let bytes = |held: Option<Vec<u8>>| held.map(Arc::<[u8]>::from);
        for saved in saved.tracks {
            let track = emit
                .tracks
                .get_mut(saved.actor as usize)
                .ok_or_else(invalid)?;
            track.identity = saved.identity;
            track.live = saved.live;
            track.last = saved.last;
            track.order = saved.order.map(|order| {
                Arc::new(facts::Order {
                    tree: bytes(order.tree),
                    completed: order.completed.into_iter().map(Arc::from).collect(),
                    current: bytes(order.current),
                    continuous: bytes(order.continuous),
                    paused: order.paused,
                })
            });
            track.order_first_hour = saved.order_first_hour;
            track.chain = saved.chain;
            track.rows = saved.rows;
            track.event = saved.event;
            track.progress = saved.progress.map(|progress| Progress {
                first: progress.first,
                raw: progress.raw,
                previous: progress
                    .previous
                    .map(|(raw, hour, current)| (raw, hour, bytes(current))),
                current: bytes(progress.current),
            });
            if track.live {
                emit.live.insert(saved.actor);
            }
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedOrder {
    tree: Option<Vec<u8>>,
    completed: Vec<Vec<u8>>,
    current: Option<Vec<u8>>,
    continuous: Option<Vec<u8>>,
    paused: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedProgress {
    first: i32,
    raw: i64,
    previous: Option<(i64, i32, Option<Vec<u8>>)>,
    current: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct SavedTrack {
    actor: u64,
    identity: Option<(u32, u32)>,
    live: bool,
    last: Option<i32>,
    order: Option<SavedOrder>,
    order_first_hour: i32,
    chain: u64,
    rows: u64,
    event: Option<Published>,
    progress: Option<SavedProgress>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    refusing: Option<u64>,
    tally: Vec<(String, u64)>,
    unavailable: BTreeMap<String, u64>,
    observed: bool,
    tracks: Vec<SavedTrack>,
}
