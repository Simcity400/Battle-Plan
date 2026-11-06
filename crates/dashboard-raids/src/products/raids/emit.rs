//! Raids' emit on one branch: the hours a segment's readers kept ([`super::segment`]) given in
//! order to the branch's one scan, as the walk gave it frames, and what the walk's preparation
//! kept of it written on the family's series publication: each closed run of an episode as a
//! chunk event of [`EPISODES`], the hours a complete read was supplied ([`READS`]), each
//! country's tallies ([`TALLY`]) and presence ([`COVERAGE`]), and at each cut the open episodes,
//! the types read and the scan's counts ([`STATE`]).

use super::{
    Content, RaidEpisode, Scan,
    segment::{Kept, Segments, TABLES},
};
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_keyed::segment::Hour;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    family::{Actors, Directory, Place},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// Of [`FAMILY`]: each episode chunk, an event of one text field (JSON); and whether each hour
/// supplied a complete read.
pub const EPISODES: u64 = 0;
pub const READS: u64 = 1;
/// Of each country actor (its tag): its launched and succeeded tallies, launched in the high 32
/// bits; and its presence.
pub const TALLY: u64 = 2;
pub const COVERAGE: u64 = 3;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
pub const CHUNK_KIND: u64 = 1;
/// The family scalar holding what the scan held at the last cut ([`State`]), as JSON.
pub const STATE: u64 = 0;
/// Every raids key's evidence: each hour walked is known.
const DOMAIN: u64 = 0;

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved raid emit is not valid")
}

/// An episode's closed runs, as the walk's preparation kept them.
#[derive(Serialize, Deserialize)]
pub struct Chunk {
    pub ordinal: u64,
    pub episode: RaidEpisode,
}

/// What a reply reads of the scan at a cut: whether raids were ever present, the open episodes
/// by ordinal, the types read, whether the recording holds raids, and the warnings and refused
/// labels the scan would finish with.
#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub present: bool,
    pub open: Vec<(u64, RaidEpisode)>,
    pub types: BTreeSet<String>,
    pub recording_present: bool,
    pub warnings: Vec<String>,
    pub noted: dashboard_text::catalogue::OwnRefusals,
}

/// What the emit goes on with, besides the scan.
#[derive(Default, Serialize, Deserialize)]
struct Held {
    next_episode: u64,
    ordinals: Vec<u64>,
    covered: BTreeSet<String>,
    supplied: BTreeSet<String>,
    present: bool,
    read_known: bool,
    last: Option<i32>,
}

/// The family's emit on one branch.
pub struct Emit {
    scan: Scan,
    content: Option<Arc<Content>>,
    /// The frame the scan reads: the views of the last hour kept.
    frame: DecodedFrame,
    held: Held,
    actors: Actors,
    carry: Carry,
    family: Option<[KeyHandle; 2]>,
    countries: HashMap<String, [KeyHandle; 2]>,
    first: Option<i32>,
}

impl Emit {
    pub fn new(format: FormatVersion, branch: u64, content: Option<Arc<Content>>) -> Self {
        let mut scan = Scan::new();
        scan.format = format;
        Self {
            scan,
            content,
            frame: DecodedFrame {
                branch,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            held: Held::default(),
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            family: None,
            countries: HashMap::new(),
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer, actor: u64, measure: u64) -> io::Result<KeyHandle> {
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension: None,
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )
    }

    fn family(&mut self, writer: &mut Writer) -> io::Result<[KeyHandle; 2]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let keys = [
            self.key(writer, FAMILY, EPISODES)?,
            self.key(writer, FAMILY, READS)?,
        ];
        self.family = Some(keys);
        Ok(keys)
    }

    /// The tally and coverage keys of `tag`.
    fn country(&mut self, writer: &mut Writer, tag: &str, hour: i32) -> io::Result<[KeyHandle; 2]> {
        if let Some(&keys) = self.countries.get(tag) {
            return Ok(keys);
        }
        let actor = match self.actors.find(tag.as_bytes()) {
            Some(actor) => actor,
            None => self.actors.actor(writer, tag.as_bytes(), hour)?,
        };
        let keys = [self.key(writer, actor, TALLY)?, self.key(writer, actor, COVERAGE)?];
        self.countries.insert(tag.to_owned(), keys);
        Ok(keys)
    }

    /// `value` of `key` from `hour`, as the walk's accumulator observes it: a run breaks where
    /// its observation starts or the hour before held none.
    fn observe(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let starts = starts || self.carry.held(key).is_none();
        self.carry.value(writer, key, hour, value, starts)?;
        Ok(())
    }

    /// One hour, after the last one given.
    pub fn hour(&mut self, writer: &mut Writer, hour: Hour<Segments>) -> io::Result<()> {
        let at = hour.hour;
        let [episodes, reads] = self.family(writer)?;
        if let Some(last) = self.held.last
            && last.checked_add(1) != Some(at)
        {
            self.carry.close(writer, last)?;
            self.known(writer, last)?;
        }
        self.held.last = Some(at);
        self.first.get_or_insert(at);
        let changes = match hour.read {
            None => {
                let mut slot_read = [false; TABLE_COUNT];
                slot_read[TableId::Raids.index()] = hour.dated;
                FrameChanges::observed([false; TABLE_COUNT], [false; TABLE_COUNT], slot_read, None)
            }
            Some(Kept {
                views,
                metrics,
                changes,
            }) => {
                for (&table, view) in TABLES.iter().zip(views.into_vec()) {
                    self.frame.tables[table.index()] = view;
                }
                self.frame.metrics = *metrics;
                *changes
            }
        };
        self.frame.hour = at;
        let frame = &self.frame;
        if !self.held.present
            && [TableId::RaidCountries, TableId::Raids]
                .iter()
                .any(|table| changes.rows_changed[table.index()])
        {
            self.held.present |= frame
                .complete::<tables::RaidCountries>()
                .is_ok_and(|rows| rows.iter().any(|row| !row.eligible_types.is_empty()))
                || frame
                    .complete::<tables::Raids>()
                    .is_ok_and(|rows| !rows.is_empty());
        }
        let continuous = self
            .scan
            .last_frame
            .is_some_and(|(_, hour)| hour.checked_add(1) == Some(at));
        let (identity_changed, raid_changed) =
            self.scan
                .observe_changed(frame, self.content.as_deref(), Some(&changes))?;
        let read = self.scan.result.read_hours.last() == Some(&at);
        self.scan.result.read_hours.clear();
        if !continuous || self.held.read_known != read {
            self.observe(writer, reads, at, read.then_some(1), false)?;
        }
        self.held.read_known = read;
        if raid_changed {
            self.chunks(writer, episodes, at)?;
        }
        if identity_changed {
            let tags: Vec<String> = self.scan.present_tags.iter().cloned().collect();
            let mut next = BTreeSet::new();
            for tag in tags {
                let [_, coverage] = self.country(writer, &tag, at)?;
                let track = self
                    .scan
                    .tracks
                    .get_mut(&tag)
                    .expect("present country track");
                let segment = track.segments.last().expect("present segment");
                let starts = segment.first_hour == at;
                if track.segments.len() > 1 {
                    track.segments.drain(..track.segments.len() - 1);
                }
                self.observe(writer, coverage, at, Some(1), starts)?;
                next.insert(tag);
            }
            for tag in self.held.covered.difference(&next).cloned().collect::<Vec<_>>() {
                let [_, coverage] = self.country(writer, &tag, at)?;
                self.observe(writer, coverage, at, None, false)?;
            }
            self.held.covered = next;
        }
        let supplied = self.scan.raid_through == Some(at);
        let membership_changed = identity_changed
            || self.scan.parents_changed
            || !continuous
            || (supplied && self.held.supplied.is_empty());
        if membership_changed || (!supplied && !self.held.supplied.is_empty()) {
            let next: BTreeSet<String> = if supplied {
                self.scan.raid_tags.clone()
            } else {
                BTreeSet::new()
            };
            for tag in self.held.supplied.difference(&next).cloned().collect::<Vec<_>>() {
                let [tally, _] = self.country(writer, &tag, at)?;
                self.observe(writer, tally, at, None, false)?;
            }
            self.held.supplied = next;
        }
        if supplied && (membership_changed || !self.scan.changed_tallies.is_empty()) {
            let tags: Vec<String> = if membership_changed {
                self.scan.raid_tags.iter().cloned().collect()
            } else {
                self.scan.changed_tallies.iter().cloned().collect()
            };
            for tag in tags {
                let [tally, _] = self.country(writer, &tag, at)?;
                let holder = self.scan.tracks[&tag]
                    .holder
                    .expect("supplying country holder");
                let (launched, succeeded) =
                    self.scan.tallies.get(&holder).copied().unwrap_or_default();
                let value = (i128::from(launched) << 32) | i128::from(succeeded);
                self.observe(writer, tally, at, Some(value), false)?;
            }
        }
        self.scan.changed_tallies.clear();
        Ok(())
    }

    /// Each episode's runs closed by the hour as a chunk, keeping only the open episodes' last.
    fn chunks(&mut self, writer: &mut Writer, key: KeyHandle, at: i32) -> io::Result<()> {
        let held = &mut self.held;
        while held.ordinals.len() < self.scan.result.episodes.len() {
            held.ordinals.push(held.next_episode);
            held.next_episode = held
                .next_episode
                .checked_add(1)
                .ok_or_else(|| dashboard_core::invalid("Raid episode count overflow"))?;
        }
        let open: BTreeSet<usize> = self.scan.open.values().copied().collect();
        for (index, episode) in self.scan.result.episodes.iter_mut().enumerate() {
            let keep = usize::from(open.contains(&index));
            let runs: Vec<_> = episode
                .runs
                .drain(..episode.runs.len().saturating_sub(keep))
                .collect();
            let preparation: Vec<_> = episode
                .preparation
                .drain(..episode.preparation.len().saturating_sub(keep))
                .collect();
            if runs.is_empty() && preparation.is_empty() {
                continue;
            }
            let chunk = Chunk {
                ordinal: held.ordinals[index],
                episode: RaidEpisode {
                    id: episode.id.clone(),
                    start: episode.start.clone(),
                    runs,
                    preparation,
                },
            };
            let json = serde_json::to_vec(&chunk)?;
            let text = writer.blob(&mut &json[..], json.len() as u64)?;
            let fields = writer.cells([Cell::Text(text)])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: CHUNK_KIND,
                fields,
            })?;
        }
        let mut remap = BTreeMap::new();
        let mut ordinals = Vec::new();
        self.scan.result.episodes = std::mem::take(&mut self.scan.result.episodes)
            .into_iter()
            .enumerate()
            .filter_map(|(index, episode)| {
                if !open.contains(&index) {
                    return None;
                }
                remap.insert(index, ordinals.len());
                ordinals.push(held.ordinals[index]);
                Some(episode)
            })
            .collect();
        held.ordinals = ordinals;
        for index in self.scan.open.values_mut() {
            *index = remap[index];
        }
        Ok(())
    }

    /// The hours walked since the domain's last row, through `last`, as known.
    fn known(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        // An hour that failed began none: the cut goes through the last that succeeded.
        if let Some(first) = self.first.take().filter(|&first| first <= last) {
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
        Ok(())
    }

    /// Appends what is open through `last`, the hours walked since the last cut as known, and
    /// what the scan holds now.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)?;
        self.scan.seal_presence();
        self.scan.seal_raids();
        let mut warnings = self.scan.refusals.lines("raid facts");
        if !self.scan.ended_victims.is_empty() {
            warnings.push(super::ENDED_VICTIMS.into());
        }
        let state = State {
            present: self.held.present,
            open: self
                .held
                .ordinals
                .iter()
                .copied()
                .zip(self.scan.result.episodes.iter().cloned())
                .collect(),
            types: self.scan.result.types.clone(),
            recording_present: self.scan.result.recording_present,
            warnings,
            noted: self.scan.result.noted.clone(),
        };
        let json = serde_json::to_vec(&state)?;
        let text = writer.blob(&mut &json[..], json.len() as u64)?;
        writer.catalogue(STATE, Cell::Text(text))
    }

    /// What it goes on with after a cut: the scan and what the emit holds, then the open runs.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = serde_json::to_vec(&(&self.scan, &self.held))?;
        out.extend_from_slice(&(saved.len() as u64).to_le_bytes());
        out.extend_from_slice(&saved);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] wrote, going on from the publication `directory` reads. A restart
    /// goes on at a segment's first hour, whose reader keeps every table anew.
    pub fn load(
        format: FormatVersion,
        branch: u64,
        content: Option<Arc<Content>>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let (length, rest) = bytes.split_first_chunk::<8>().ok_or_else(invalid)?;
        let length = usize::try_from(u64::from_le_bytes(*length))
            .ok()
            .filter(|&length| length <= rest.len())
            .ok_or_else(invalid)?;
        let (saved, carry) = rest.split_at(length);
        let (mut scan, held): (Scan, Held) = serde_json::from_slice(saved)?;
        scan.format = format;
        let mut emit = Self::new(format, branch, content);
        emit.scan = scan;
        emit.held = held;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.carry = Carry::load(carry)?;
        emit.family = Some([
            directory.key(FAMILY, EPISODES, None).ok_or_else(invalid)?,
            directory.key(FAMILY, READS, None).ok_or_else(invalid)?,
        ]);
        for (actor, described) in directory.actors.iter().enumerate() {
            let actor = actor as u64;
            if let (Some(tally), Some(coverage), Ok(tag)) = (
                directory.key(actor, TALLY, None),
                directory.key(actor, COVERAGE, None),
                std::str::from_utf8(&described.name),
            ) {
                emit.countries.insert(tag.to_owned(), [tally, coverage]);
            }
        }
        Ok(emit)
    }
}
