//! The recorded viewpoint's walk, published while preparing.
//!
//! Wars prepared on their own run the default query's walk (the recorded
//! LocalPlayer viewpoint, no native-holder selection, the store's content
//! catalogue) over each change input as they publish it, in input order. A
//! war that leaves the walk's open groups no longer changes before the
//! reply is finished, so it is published once, in the next flush's range of
//! closed wars. Every [`POINT_INPUTS`] inputs a resumable copy of the walk
//! and the decoded inputs are published as a point: the copy holds only what
//! continuing the walk reads (each open war's last observation and loss run
//! and its open participant and relation runs, the last dated runs, the held
//! inputs) and the number of wars, not the wars no longer open or the
//! history of open ones, so a point's size follows the wars open at it.
//! Points count toward the batch's bytes. Each flush rewrites the tail: the whole walk
//! but its closed wars, which keep only their place and app ID, with the
//! decoded inputs and the coverage needed to continue it. A default query
//! finishes the tail with the closed wars and hydrates each requested hour
//! from the nearest point, walking fewer than [`POINT_INPUTS`] inputs.

use super::walks::decode_input;
use super::*;
use dashboard_base::prepared::series;
use replay_data::codec::Decoder;
use std::{collections::BTreeMap, sync::Arc};

/// Inputs between published points.
#[cfg(not(any(test, feature = "test-support")))]
pub const POINT_INPUTS: u64 = 128;
#[cfg(any(test, feature = "test-support"))]
pub const POINT_INPUTS: u64 = 4;

/// The tail (range 0) and its decoded inputs (range 1).
fn tail_section(branch: u64) -> String {
    format!("wars-walk-{branch}")
}

/// Point `p` (after input `p * POINT_INPUTS - 1`) at range `2 * (p - 1)`, its
/// decoded inputs at the range after.
fn points_section(branch: u64) -> String {
    format!("wars-walk-points-{branch}")
}

/// Each flush's closed wars, one range per flush that closed any.
fn closed_section(branch: u64) -> String {
    format!("wars-walk-closed-{branch}")
}

/// What a war holds beside its reply fields.
#[derive(Serialize, Deserialize)]
struct SavedWar {
    relations: Vec<(WarRelationKey, i32, i32)>,
    /// In participant order, so equal walks save equal bytes.
    open_participants: Vec<(String, WarSide, usize)>,
    open_relations: Vec<(WarRelationKey, usize)>,
}

/// A walk without its held inputs, which are rebuilt from the decoded
/// inputs they were read from, and without the label operands, which its
/// next observation reads again.
#[derive(Serialize, Deserialize)]
struct SavedWalk {
    reply: WarsReply,
    wars: Vec<SavedWar>,
    previous: Option<i32>,
    previous_view: Option<String>,
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
        previous_view: walk.previous_view.clone(),
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

fn memberships(war: &WarTimeline) -> Vec<(WarRelationKey, i32, i32)> {
    war.relations
        .iter()
        .map(|run| (run.key.clone(), run.first_hour, run.last_hour))
        .collect()
}

fn restored_memberships(relations: Vec<(WarRelationKey, i32, i32)>) -> Vec<WarRelationMembership> {
    relations
        .into_iter()
        .map(|(key, first_hour, last_hour)| WarRelationMembership {
            key,
            first_hour,
            last_hour,
        })
        .collect()
}

/// The held hour a walk builds from `decoded` for the recorded viewpoint.
fn held_hour(compared: &[TableId], decoded: &DecodedFrame, cat: Option<&Catalogue>) -> HeldHour {
    HeldHour {
        tables: compared
            .iter()
            .map(|table| decoded.tables[table.index()].identity())
            .collect(),
        observed: Hour::read(decoded, None).map(|inputs| {
            let observations = inputs.observe(decoded, cat);
            (inputs, observations)
        }),
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

/// A resumable copy of the walk after input `next - 1`, published at `hour`.
/// With `wars`, the walk holds only its open wars (ids `open`, in id order)
/// of that many: a war no longer open is never read again by a walk resumed
/// here, so it is only a place.
#[derive(Serialize, Deserialize)]
struct Point {
    next: u64,
    hour: Option<i32>,
    walk: SavedWalk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wars: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    open: Vec<usize>,
}

/// `saved` holding only the wars `open` names, and the number of wars it held.
fn elide(mut saved: SavedWalk, open: &[usize]) -> (SavedWalk, usize) {
    let count = saved.reply.wars.len();
    let wars = std::mem::take(&mut saved.reply.wars);
    let records = std::mem::take(&mut saved.wars);
    for (id, (war, record)) in wars.into_iter().zip(records).enumerate() {
        if open.binary_search(&id).is_ok() {
            saved.reply.wars.push(war);
            saved.wars.push(record);
        }
    }
    (saved, count)
}

/// `saved` (holding the wars `open` names) with each of its `count` wars in
/// its place: a war it does not hold is a place with its app ID only.
fn expand(mut saved: SavedWalk, count: usize, open: &[usize]) -> io::Result<SavedWalk> {
    if saved.reply.wars.len() != open.len() || saved.wars.len() != open.len() {
        return Err(invalid("Published wars point is malformed"));
    }
    let mut held: Vec<Option<(WarTimeline, SavedWar)>> = (0..count).map(|_| None).collect();
    let wars = std::mem::take(&mut saved.reply.wars);
    let records = std::mem::take(&mut saved.wars);
    for (&id, war) in open.iter().zip(wars.into_iter().zip(records)) {
        *held
            .get_mut(id)
            .ok_or_else(|| invalid("Published wars point holds a war out of place"))? = Some(war);
    }
    for (id, war) in held.into_iter().enumerate() {
        let (war, record) = war.unwrap_or_else(|| {
            (
                WarTimeline {
                    id: id.to_string(),
                    first_hour: 0,
                    last_hour: 0,
                    started_by: false,
                    ended_by: false,
                    observations: Vec::new(),
                    casualties: Vec::new(),
                    participants: Vec::new(),
                    capitulations: Vec::new(),
                    relations: Vec::new(),
                    open_participants: HashMap::new(),
                    open_relations: HashMap::new(),
                },
                SavedWar {
                    relations: Vec::new(),
                    open_participants: Vec::new(),
                    open_relations: Vec::new(),
                },
            )
        });
        saved.reply.wars.push(war);
        saved.wars.push(record);
    }
    Ok(saved)
}

/// A war no longer open, as the reply finishes it but for capitulations.
#[derive(Serialize, Deserialize)]
struct Closed {
    id: usize,
    war: WarTimeline,
    relations: Vec<(WarRelationKey, i32, i32)>,
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
    /// Each new point's number, the point as published and its decoded
    /// inputs.
    pending_points: Vec<(u64, Vec<u8>, Vec<u8>)>,
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
            pending_points: Vec::new(),
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
            pending_points: Vec::new(),
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
        self.walk.observe(&current, None, None, self.cat.as_deref());
        self.decoded = Some(current);
        self.walked = true;
        for id in open {
            if !self.walk.open.iter().any(|open| open.id == id) {
                self.close(id);
            }
        }
        self.next += 1;
        self.hour = Some(hour);
        if self.next.is_multiple_of(POINT_INPUTS) {
            let decoded = self
                .decoded
                .as_ref()
                .map(encode_frame)
                .transpose()?
                .unwrap_or_default();
            // Its open wars' continuation only, without the wars no longer
            // open, published as `series::write_reply` writes a reply.
            let resumable = self.walk.resumable();
            let mut open: Vec<usize> = resumable.open.iter().map(|war| war.id).collect();
            open.sort_unstable();
            open.dedup();
            let (walk, wars) = elide(save(&resumable), &open);
            let point = Point {
                next: self.next,
                hour: self.hour,
                walk,
                wars: Some(wars),
                open,
            };
            let point = series::compress(serde_json::to_vec(&point)?.as_slice(), 3)?;
            self.pending_points
                .push((self.next / POINT_INPUTS, point, decoded));
        }
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
        self.pending_points
            .iter()
            .map(|(_, point, decoded)| point.len() + decoded.len())
            .sum::<usize>()
            + self.pending_closed.len() * 1024
    }

    /// Publish the closed wars and points since the last flush, and the tail.
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
        for (point, saved, decoded) in self.pending_points.drain(..) {
            let range = 2 * (point - 1);
            writer.write_at(&points_section(branch), range, saved.as_slice())?;
            writer.write_at(&points_section(branch), range + 1, decoded.as_slice())?;
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
    /// The walk published through the store's last committed input, if any.
    pub(super) fn read(
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        cat: Option<&Catalogue>,
    ) -> io::Result<Option<Self>> {
        let section = tail_section(branch);
        let Some(tail) = parse::<Tail>(read_bytes(store.read(key, &section, 0)?)?)? else {
            return Ok(None);
        };
        let decoded = read_bytes(store.read(key, &section, 1)?)?
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

    /// Put every closed war back in its place.
    pub(super) fn close(
        &mut self,
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
    ) -> io::Result<()> {
        let section = closed_section(branch);
        for range in 0..self.closed {
            let closed: Vec<Closed> = parse(read_bytes(store.read(key, &section, range)?)?)?
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

/// Point `point` of a published walk: inputs walked, the hour of the last,
/// the walk and its decoded inputs.
pub(super) fn point(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    point: u64,
    cat: Option<&Catalogue>,
    first: i32,
) -> io::Result<(u64, Option<i32>, Walk, DecodedFrame)> {
    let section = points_section(branch);
    let range = 2 * (point - 1);
    let saved: Point = parse(read_bytes(store.read(key, &section, range)?)?)?
        .ok_or_else(|| invalid("Published wars point is missing"))?;
    let decoded = read_bytes(store.read(key, &section, range + 1)?)?
        .ok_or_else(|| invalid("Published wars point inputs are missing"))?;
    let decoded = decode_frame(&decoded)?;
    let held = match saved.wars {
        Some(count) => expand(saved.walk, count, &saved.open)?,
        None => saved.walk,
    };
    let walk = restore(held, Some(&decoded), cat, branch, first)?;
    Ok((saved.next, saved.hour, walk, decoded))
}
