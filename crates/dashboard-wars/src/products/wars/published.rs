//! The recorded viewpoint's walk, published while preparing.
//!
//! Wars prepared on their own run the default query's walk (the recorded
//! LocalPlayer viewpoint, no native-holder selection, the store's content
//! catalogue) over each change input as they publish it, in input order. A
//! war that leaves the walk's open groups no longer changes before the
//! reply is finished, so it is published once, in the next flush's range of
//! closed wars. Each flush rewrites the tail: the whole walk but its closed
//! wars, which keep only their place and app ID, with the decoded inputs and
//! the coverage needed to continue it. A default query finishes the tail with
//! the closed wars; each war's relation memberships answer its casualties at
//! any hour, so no earlier walk is published.

use super::walks::decode_input;
use super::*;
use dashboard_base::prepared::series;
use replay_data::codec::Decoder;
use std::{collections::BTreeMap, sync::Arc};

/// The tail (range 0) and its decoded inputs (range 1).
fn tail_section(branch: u64) -> String {
    format!("wars-walk-{branch}")
}

/// Each flush's closed wars, one range per flush that closed any.
fn closed_section(branch: u64) -> String {
    format!("wars-walk-closed-{branch}")
}

/// What a war holds beside its reply fields.
#[derive(Serialize, Deserialize)]
struct SavedWar {
    relations: Vec<(WarRelationKey, bool, i32, i32)>,
    /// In participant order, so equal walks save equal bytes.
    open_participants: Vec<(String, WarSide, usize)>,
    open_relations: Vec<((WarRelationKey, bool), usize)>,
}

/// A walk without its held inputs, which are rebuilt from the decoded
/// inputs they were read from, and without the label operands, which its
/// next observation reads again.
#[derive(Serialize, Deserialize)]
struct SavedWalk {
    reply: WarsReply,
    wars: Vec<SavedWar>,
    previous: Option<i32>,
    previous_view: Option<(u32, u32)>,
    open: Vec<OpenWar>,
    seen_caps: Vec<(String, i32)>,
    dated_caps: BTreeMap<String, Vec<i32>>,
    identity_since: BTreeMap<u32, (String, i32)>,
    warnings: BTreeSet<String>,
    last_frame: Option<i32>,
    held: bool,
    labels: bool,
    names: BTreeMap<u32, BTreeSet<String>>,
    involved: BTreeSet<u32>,
}

fn save(walk: &Walk) -> SavedWalk {
    let mut seen_caps: Vec<_> = walk.seen_caps.iter().cloned().collect();
    seen_caps.sort_unstable();
    let (names, involved) = walk.country_labels.saved();
    SavedWalk {
        reply: walk.reply.clone(),
        wars: walk
            .reply
            .wars
            .iter()
            .map(|war| {
                let mut open_participants: Vec<_> = war
                    .open_participants
                    .iter()
                    .map(|((tag, side), &index)| (tag.clone(), *side, index))
                    .collect();
                open_participants.sort_unstable_by_key(|(_, _, index)| *index);
                let mut open_relations: Vec<_> = war
                    .open_relations
                    .iter()
                    .map(|(key, &index)| (key.clone(), index))
                    .collect();
                open_relations.sort_unstable_by_key(|(_, index)| *index);
                SavedWar {
                    relations: memberships(war),
                    open_participants,
                    open_relations,
                }
            })
            .collect(),
        previous: walk.previous,
        previous_view: walk.previous_view,
        open: walk.open.clone(),
        seen_caps,
        dated_caps: walk
            .dated_caps
            .iter()
            .map(|(tag, hours)| (tag.clone(), hours.clone()))
            .collect(),
        identity_since: walk
            .identity_since
            .iter()
            .map(|(&raw, since)| (raw, since.clone()))
            .collect(),
        warnings: walk.warnings.clone(),
        last_frame: walk.last_frame,
        held: walk.held.is_some(),
        labels: walk.labels,
        names,
        involved,
    }
}

fn memberships(war: &WarTimeline) -> Vec<(WarRelationKey, bool, i32, i32)> {
    war.relations
        .iter()
        .map(|run| (run.key.clone(), run.oriented, run.first_hour, run.last_hour))
        .collect()
}

fn restored_memberships(
    relations: Vec<(WarRelationKey, bool, i32, i32)>,
) -> Vec<WarRelationMembership> {
    relations
        .into_iter()
        .map(
            |(key, oriented, first_hour, last_hour)| WarRelationMembership {
                key,
                oriented,
                first_hour,
                last_hour,
            },
        )
        .collect()
}

/// The held hour a walk builds from `decoded` for the recorded viewpoint.
fn held_hour(compared: &[TableId], decoded: &DecodedFrame, cat: Option<&Catalogue>) -> HeldHour {
    HeldHour {
        tables: compared
            .iter()
            .map(|table| decoded.tables[table.index()].identity())
            .collect(),
        observed: read_hour(decoded, None, None, cat, true),
    }
}

/// The walk `saved` describes, holding the inputs read from `decoded`.
fn restore(
    saved: SavedWalk,
    decoded: Option<&DecodedFrame>,
    cat: Option<&Catalogue>,
    branch: u64,
    first: i32,
) -> io::Result<Walk> {
    let SavedWalk {
        mut reply,
        wars,
        previous,
        previous_view,
        open,
        seen_caps,
        dated_caps,
        identity_since,
        warnings,
        last_frame,
        held,
        labels,
        names,
        involved,
    } = saved;
    if wars.len() != reply.wars.len() {
        return Err(invalid("Published wars walk is malformed"));
    }
    for (war, saved) in reply.wars.iter_mut().zip(wars) {
        war.relations = restored_memberships(saved.relations);
        war.open_participants = saved
            .open_participants
            .into_iter()
            .map(|(tag, side, index)| ((tag, side), index))
            .collect();
        war.open_relations = saved.open_relations.into_iter().collect();
    }
    let mut walk = Walk::new(branch, first);
    walk.reply = reply;
    walk.previous = previous;
    walk.previous_view = previous_view;
    walk.open = open;
    walk.seen_caps = seen_caps.into_iter().collect();
    walk.dated_caps = dated_caps.into_iter().collect();
    walk.identity_since = identity_since.into_iter().collect();
    walk.warnings = warnings;
    walk.last_frame = last_frame;
    walk.labels = labels;
    walk.country_labels = labels::CountryLabels::restored((names, involved));
    if held {
        let decoded = decoded
            .ok_or_else(|| invalid("Published wars walk holds inputs without their frame"))?;
        walk.held = Some(held_hour(&walk.compared, decoded, cat));
    }
    Ok(walk)
}

fn encode_frame(frame: &DecodedFrame) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes)?;
    series::compress(bytes.as_slice(), 3)
}

fn decode_frame(bytes: &[u8]) -> io::Result<DecodedFrame> {
    let raw = series::decompress(bytes)?;
    let mut input = Decoder::new(&raw);
    let frame = DecodedFrame::decode_checkpoint(&mut input)?;
    input.finish()?;
    Ok(frame)
}

/// The walk after every published input, but its closed wars.
#[derive(Serialize, Deserialize)]
struct Tail {
    /// The branch's first hour the walk started from.
    first: i32,
    /// Inputs walked, and the hour of the last.
    next: u64,
    hour: Option<i32>,
    /// The last hour the wars were observed: the last input's coverage so
    /// far, which the next input's walk extends through.
    observed: Option<i32>,
    /// Ranges of closed wars published.
    closed: u64,
    walk: SavedWalk,
}

/// A war no longer open, as the reply finishes it but for capitulations.
#[derive(Serialize, Deserialize)]
struct Closed {
    id: usize,
    war: WarTimeline,
    relations: Vec<(WarRelationKey, bool, i32, i32)>,
}

fn read_bytes(file: Option<impl io::Read>) -> io::Result<Option<Vec<u8>>> {
    file.map(|mut file| {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
    .transpose()
}

fn parse<T: serde::de::DeserializeOwned>(bytes: Option<Vec<u8>>) -> io::Result<Option<T>> {
    bytes
        .map(|bytes| {
            let json = series::decompress(&bytes)?;
            serde_json::from_slice(&json).map_err(io::Error::from)
        })
        .transpose()
}

/// The walk preparation carries for one branch, with what it has not
/// published yet.
pub(super) struct Prepared {
    walk: Walk,
    decoded: Option<DecodedFrame>,
    cat: Option<Arc<Catalogue>>,
    first: i32,
    next: u64,
    hour: Option<i32>,
    observed: Option<i32>,
    closed: u64,
    pending_closed: Vec<Closed>,
    /// Whether the decoded inputs changed since the tail last published them.
    walked: bool,
}

/// The tail the last commit published, if any: its walk and decoded inputs.
pub(super) struct Saved {
    tail: Tail,
    decoded: Option<Vec<u8>>,
}

impl Saved {
    /// The tail `saved` published, as preparation continuing it reads it.
    pub(super) fn read(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Option<Self>> {
        let section = tail_section(branch);
        let read = |range| match saved {
            None => Ok(None),
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, range),
            Some(_) => store.read(key, &section, range),
        };
        let Some(tail) = parse::<Tail>(read_bytes(read(0)?)?)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            tail,
            decoded: read_bytes(read(1)?)?,
        }))
    }

    /// The branch's first hour the saved walk started from.
    pub(super) fn first(&self) -> i32 {
        self.tail.first
    }
}

impl Prepared {
    /// A walk from the branch's first hour.
    pub(super) fn new(branch: u64, first: i32, cat: Option<Arc<Catalogue>>) -> Self {
        Self {
            walk: Walk::new(branch, first),
            decoded: None,
            cat,
            first,
            next: 0,
            hour: None,
            observed: None,
            closed: 0,
            pending_closed: Vec::new(),
            walked: false,
        }
    }

    /// The walk a commit saved, continuing.
    pub(super) fn load(saved: Saved, cat: Option<Arc<Catalogue>>, branch: u64) -> io::Result<Self> {
        let Saved { tail, decoded } = saved;
        let decoded = decoded.as_deref().map(decode_frame).transpose()?;
        let walk = restore(
            tail.walk,
            decoded.as_ref(),
            cat.as_deref(),
            branch,
            tail.first,
        )?;
        Ok(Self {
            walk,
            decoded,
            cat,
            first: tail.first,
            next: tail.next,
            hour: tail.hour,
            observed: tail.observed,
            closed: tail.closed,
            pending_closed: Vec::new(),
            walked: false,
        })
    }

    /// After the wars observed through `hour`: walk the inputs they
    /// published there.
    pub(super) fn after(&mut self, inputs: Vec<(i32, Vec<u8>)>, hour: i32) -> io::Result<()> {
        for (input, row) in inputs {
            self.step(input, &row)?;
        }
        self.observed = Some(hour);
        Ok(())
    }

    /// Walk one published input, as a query walks it.
    fn step(&mut self, hour: i32, row: &[u8]) -> io::Result<()> {
        let (changes, packet) = decode_input(row)?;
        let packet = decode_frame(packet)?;
        let changed = changes
            .iter()
            .map(|&table| {
                TableId::ALL
                    .iter()
                    .position(|id| *id as u16 == table)
                    .ok_or_else(|| invalid("Unknown wars input table"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        // The last input's coverage reaches the last hour observed before
        // this one.
        if self.next > 0
            && let Some(last) = self.observed
        {
            self.walk.extend(last);
        }
        let mut current = self.decoded.take().unwrap_or_else(|| packet.clone());
        current.branch = packet.branch;
        current.hour = packet.hour;
        for index in changed {
            current.tables[index] = packet.tables[index].clone();
        }
        let open: Vec<usize> = self.walk.open.iter().map(|war| war.id).collect();
        self.walk
            .observe(&current, None, None, self.cat.as_deref(), None);
        self.decoded = Some(current);
        self.walked = true;
        for id in open {
            if !self.walk.open.iter().any(|open| open.id == id) {
                self.close(id);
            }
        }
        self.next += 1;
        self.hour = Some(hour);
        Ok(())
    }

    /// War `id` left the open groups: published as finished, but for
    /// capitulations, and kept in the walk as its place and app ID only.
    fn close(&mut self, id: usize) {
        let stub = {
            let war = &self.walk.reply.wars[id];
            WarTimeline {
                id: war.id.clone(),
                first_hour: war.first_hour,
                last_hour: war.last_hour,
                started_by: war.started_by,
                ended_by: war.ended_by,
                observations: Vec::new(),
                casualties: Vec::new(),
                participants: Vec::new(),
                capitulations: Vec::new(),
                relations: Vec::new(),
                open_participants: HashMap::new(),
                open_relations: HashMap::new(),
            }
        };
        let mut war = std::mem::replace(&mut self.walk.reply.wars[id], stub);
        // As finishing the reply settles every participant still open.
        let open: Vec<usize> = war.open_participants.values().copied().collect();
        for index in open {
            war.participants[index].last_hour = war.last_hour;
        }
        war.open_participants.clear();
        war.open_relations.clear();
        self.pending_closed.push(Closed {
            id,
            relations: memberships(&war),
            war,
        });
    }

    /// Bytes held for the next flush.
    pub(super) fn bytes(&self) -> usize {
        self.pending_closed.len() * 1024
    }

    /// Publish the closed wars since the last flush, and the tail.
    pub(super) fn flush(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        if !self.pending_closed.is_empty() {
            series::write_reply(
                writer,
                &closed_section(branch),
                self.closed,
                &self.pending_closed,
            )?;
            self.closed += 1;
            self.pending_closed.clear();
        }
        series::write_reply(
            writer,
            &tail_section(branch),
            0,
            &Tail {
                first: self.first,
                next: self.next,
                hour: self.hour,
                observed: self.observed,
                closed: self.closed,
                walk: save(&self.walk),
            },
        )?;
        // The decoded inputs change only with an input.
        if self.walked
            && let Some(decoded) = &self.decoded
        {
            writer.write_at(&tail_section(branch), 1, encode_frame(decoded)?.as_slice())?;
            self.walked = false;
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Published tails read on this thread.
    static TAIL_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// Published tails read on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn tail_reads() -> usize {
    TAIL_READS.with(std::cell::Cell::get)
}

/// What a query reads of a published walk.
pub(super) struct Published {
    /// The walk after every published input, its closed wars still places.
    pub(super) walk: Walk,
    pub(super) first: i32,
    pub(super) next: u64,
    pub(super) hour: Option<i32>,
    closed: u64,
}

impl Published {
    /// The identity of the tail `session` holds: every flush rewrites it, closed wars and all.
    pub(super) fn page(session: &store::Session, branch: u64) -> io::Result<Option<[u64; 3]>> {
        Ok(session
            .read(&tail_section(branch), 0)?
            .filter(|range| !range.is_empty())
            .map(|range| range.id().cache_key()))
    }

    /// The walk `session` holds published through its last committed input, if any.
    pub(super) fn read(
        session: &store::Session,
        branch: u64,
        cat: Option<&Catalogue>,
    ) -> io::Result<Option<Self>> {
        #[cfg(any(test, feature = "test-support"))]
        TAIL_READS.with(|count| count.set(count.get() + 1));
        let section = tail_section(branch);
        let Some(tail) = parse::<Tail>(read_bytes(session.read(&section, 0)?)?)? else {
            return Ok(None);
        };
        let decoded = read_bytes(session.read(&section, 1)?)?
            .as_deref()
            .map(decode_frame)
            .transpose()?;
        Ok(Some(Self {
            walk: restore(tail.walk, decoded.as_ref(), cat, branch, tail.first)?,
            first: tail.first,
            next: tail.next,
            hour: tail.hour,
            closed: tail.closed,
        }))
    }

    /// Put every closed war `session` holds back in its place.
    pub(super) fn close(&mut self, session: &store::Session, branch: u64) -> io::Result<()> {
        let section = closed_section(branch);
        for range in 0..self.closed {
            let closed: Vec<Closed> = parse(read_bytes(session.read(&section, range)?)?)?
                .ok_or_else(|| invalid("Published closed wars are missing"))?;
            for Closed {
                id,
                mut war,
                relations,
            } in closed
            {
                war.relations = restored_memberships(relations);
                let place = self
                    .walk
                    .reply
                    .wars
                    .get_mut(id)
                    .ok_or_else(|| invalid("Published closed war has no place"))?;
                *place = war;
            }
        }
        Ok(())
    }
}
