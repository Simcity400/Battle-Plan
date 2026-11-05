//! The wars' prepared layers. What every viewpoint reads alike at an hour: the countries, the
//! recording client's viewpoint and why the war tables past it cannot be read, the active
//! relations, the capitulations, whether the overview was recorded and the countries' historical
//! names. And what each viewpoint at war sees: its native groups, each a membership (the blob's
//! native arrays) and a label (its representative relation and the name it gives it). A viewpoint
//! owning no live war relation sees no group, so every other viewpoint reads the shared layer
//! alone.
//!
//! The projector computes them from the tables a walk at the hour reads; the emit publishes each
//! where it changed.

use super::inputs::{self, INPUTS};
use super::naming::Namer;
use super::native::{Blob, World, pair};
use super::*;
use dashboard_series::codec::Decode;

fn malformed() -> io::Error {
    invalid("Malformed wars layer")
}

/// A native viewpoint identity: its canonical slot and its Countries row's id.
pub type Identity = (u32, u32);

/// A failure as an hour's read words it, without the hour a table's read stamps into it: a walk
/// words it again at each hour it reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Failure {
    /// Where in `text` the read's hour stands, if a read stamped it.
    stamp: Option<u32>,
    text: String,
}

impl Failure {
    /// `message`, read at `hour`: its stamp leads it, or follows a context's `: `.
    pub fn of(message: String, hour: i32) -> Self {
        let stamp = format!("hour {hour} ");
        let at = if message.starts_with(&stamp) {
            Some(0)
        } else {
            message.find(&format!(": {stamp}")).map(|at| at + 2)
        };
        match at {
            Some(at) => Self {
                stamp: Some(at as u32),
                text: format!("{}{}", &message[..at], &message[at + stamp.len()..]),
            },
            None => Self {
                stamp: None,
                text: message,
            },
        }
    }
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            stamp: None,
            text: text.into(),
        }
    }
    /// Whether a read words it again at every hour it reads.
    pub fn stamped(&self) -> bool {
        self.stamp.is_some()
    }
    /// The failure as a read at `hour` words it.
    pub fn at(&self, hour: i32) -> String {
        match self.stamp {
            Some(at) => {
                let (context, rest) = self.text.split_at(at as usize);
                format!("{context}hour {hour} {rest}")
            }
            None => self.text.clone(),
        }
    }
    fn put(&self, out: &mut Out) {
        out.var(self.stamp.map_or(0, |at| u64::from(at) + 1));
        out.text(&self.text);
    }
    fn get(d: &mut Decode<'_>) -> io::Result<Self> {
        let stamp = d.var()?;
        let text = text(d)?;
        let stamp = match stamp.checked_sub(1) {
            None => None,
            Some(at) if text.is_char_boundary(at as usize) && at as usize <= text.len() => {
                Some(at as u32)
            }
            Some(_) => return Err(invalid("Wars failure stamp outside its text")),
        };
        Ok(Self { stamp, text })
    }
}

/// The bytes a layer is written as: varints, zigzag hours, flags and length-prefixed bytes.
#[derive(Default)]
pub struct Out(Vec<u8>);

impl Out {
    pub fn var(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.0.push(byte);
                return;
            }
            self.0.push(byte | 0x80);
        }
    }
    pub fn word(&mut self, value: u32) {
        self.var(u64::from(value));
    }
    pub fn int(&mut self, value: i32) {
        let value = i64::from(value);
        self.var(((value << 1) ^ (value >> 63)) as u64);
    }
    pub fn flag(&mut self, value: bool) {
        self.0.push(u8::from(value));
    }
    pub fn bytes(&mut self, value: &[u8]) {
        self.var(value.len() as u64);
        self.0.extend_from_slice(value);
    }
    pub fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

pub(crate) fn word(d: &mut Decode<'_>) -> io::Result<u32> {
    u32::try_from(d.var()?).map_err(|_| malformed())
}
pub(crate) fn int(d: &mut Decode<'_>) -> io::Result<i32> {
    i32::try_from(d.zig()?).map_err(|_| malformed())
}
fn length(d: &mut Decode<'_>) -> io::Result<usize> {
    usize::try_from(d.var()?).map_err(|_| malformed())
}
pub(crate) fn bytes(d: &mut Decode<'_>) -> io::Result<Vec<u8>> {
    let len = length(d)?;
    Ok(d.bytes(len)?.to_vec())
}
pub(crate) fn text(d: &mut Decode<'_>) -> io::Result<String> {
    String::from_utf8(bytes(d)?).map_err(|_| malformed())
}
/// `count` items read by `item`.
fn items<T>(
    d: &mut Decode<'_>,
    mut item: impl FnMut(&mut Decode<'_>) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    let count = length(d)?;
    let mut held = Vec::new();
    for _ in 0..count {
        held.push(item(d)?);
    }
    Ok(held)
}
/// Every byte of `bytes` read by `read`.
pub(crate) fn whole<T>(
    bytes: &[u8],
    read: impl FnOnce(&mut Decode<'_>) -> io::Result<T>,
) -> io::Result<T> {
    let mut d = Decode::new(bytes);
    let value = read(&mut d)?;
    d.end().map_err(|_| malformed())?;
    Ok(value)
}
fn compressed(out: Out) -> io::Result<Vec<u8>> {
    dashboard_base::prepared::series::compress(&out.into_bytes(), 1)
}
fn uncompressed(bytes: &[u8]) -> io::Result<Vec<u8>> {
    dashboard_base::prepared::series::decompress(bytes)
}

/// The countries and their links as an hour reads them, or why each is not read whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    /// Countries in row order: slot, native id, tag.
    pub countries: Result<Vec<(u32, u32, Vec<u8>)>, Failure>,
    /// Links in row order: raw id, slot.
    pub links: Result<Vec<(u32, u32)>, Failure>,
}

impl Roster {
    pub fn read(frame: &DecodedFrame) -> Self {
        Self {
            countries: frame
                .complete::<tables::Countries>()
                .map(|rows| {
                    rows.iter()
                        .map(|r| (r.canonical_slot, r.id, r.tag.clone()))
                        .collect()
                })
                .map_err(|e| Failure::of(e.to_string(), frame.hour)),
            links: frame
                .complete::<tables::CountryLinks>()
                .map(|rows| rows.iter().map(|r| (r.id, r.canonical_slot)).collect())
                .map_err(|e| Failure::of(e.to_string(), frame.hour)),
        }
    }
    /// The countries an hour reads ([`Countries::read`]), or why not.
    pub(crate) fn resolve(&self) -> Result<Countries, Failure> {
        let countries = self.countries.as_ref().map_err(Clone::clone)?;
        let links = self.links.as_ref().map_err(Clone::clone)?;
        Countries::of(
            countries
                .iter()
                .map(|(slot, id, tag)| (*slot, *id, tag.as_slice())),
            links.iter().copied(),
        )
        .map_err(Failure::plain)
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = Out::default();
        match &self.countries {
            Ok(rows) => {
                out.flag(true);
                out.var(rows.len() as u64);
                for (slot, id, tag) in rows {
                    out.word(*slot);
                    out.word(*id);
                    out.bytes(tag);
                }
            }
            Err(failure) => {
                out.flag(false);
                failure.put(&mut out);
            }
        }
        match &self.links {
            Ok(rows) => {
                out.flag(true);
                out.var(rows.len() as u64);
                for &(raw, slot) in rows {
                    out.word(raw);
                    out.word(slot);
                }
            }
            Err(failure) => {
                out.flag(false);
                failure.put(&mut out);
            }
        }
        compressed(out)
    }
    pub fn decode(encoded: &[u8]) -> io::Result<Self> {
        whole(&uncompressed(encoded)?, |d| {
            let countries = if d.flag()? {
                Ok(items(d, |d| Ok((word(d)?, word(d)?, bytes(d)?)))?)
            } else {
                Err(Failure::get(d)?)
            };
            let links = if d.flag()? {
                Ok(items(d, |d| Ok((word(d)?, word(d)?)))?)
            } else {
                Err(Failure::get(d)?)
            };
            Ok(Self { countries, links })
        })
    }
}

/// What an hour reads alike for every viewpoint once its countries are read: the recording
/// client's viewpoint, and why the owned occurrences or the war tables past them cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub recorded_view: Option<u32>,
    pub owned: Option<Failure>,
    pub rest: Option<Failure>,
    /// Whether a WarRelations row's endpoints both resolve: a requested hour's read, whose
    /// relations carry the counters prepared for them, then finds none where the war tables
    /// past them did not read.
    pub resolvable: bool,
}

impl State {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        out.flag(self.recorded_view.is_some());
        if let Some(view) = self.recorded_view {
            out.word(view);
        }
        for failure in [&self.owned, &self.rest] {
            out.flag(failure.is_some());
            if let Some(failure) = failure {
                failure.put(&mut out);
            }
        }
        out.flag(self.resolvable);
        out.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        whole(bytes, |d| {
            let recorded_view = if d.flag()? { Some(word(d)?) } else { None };
            let owned = if d.flag()? {
                Some(Failure::get(d)?)
            } else {
                None
            };
            let rest = if d.flag()? {
                Some(Failure::get(d)?)
            } else {
                None
            };
            Ok(Self {
                recorded_view,
                owned,
                rest,
                resolvable: d.flag()?,
            })
        })
    }
}

/// An active war relation as the groups read it: its endpoints' slots in row order, its start,
/// and whether its first endpoint instigated it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relation {
    pub first: u32,
    pub second: u32,
    pub start: i32,
    pub instigator_first: bool,
}

pub fn encode_relations(relations: &[Relation]) -> io::Result<Vec<u8>> {
    let mut out = Out::default();
    out.var(relations.len() as u64);
    for r in relations {
        out.word(r.first);
        out.word(r.second);
        out.int(r.start);
        out.flag(r.instigator_first);
    }
    compressed(out)
}

pub fn decode_relations(bytes: &[u8]) -> io::Result<Vec<Relation>> {
    whole(&uncompressed(bytes)?, |d| {
        items(d, |d| {
            Ok(Relation {
                first: word(d)?,
                second: word(d)?,
                start: int(d)?,
                instigator_first: d.flag()?,
            })
        })
    })
}

/// The war world `relations` (in row order) make: the relations and each country's neighbours
/// in row order, as the groups read them.
pub(super) fn graph(relations: &[Relation]) -> World {
    let mut world = World {
        owned: Default::default(),
        relations: Default::default(),
        adjacent: Default::default(),
        factions: Default::default(),
        scores: Default::default(),
    };
    for r in relations {
        let p = pair(r.first, r.second);
        world.relations.insert(
            p,
            rows::WarRelationRow {
                first_country: r.first,
                second_country: r.second,
                start_hour: r.start,
                first_was_instigator: r.instigator_first.into(),
                ..Default::default()
            },
        );
        world.adjacent.entry(p.0).or_default().push(p.1);
        world.adjacent.entry(p.1).or_default().push(p.0);
    }
    world
}

/// Each capitulated country's raw id and date, or none where the capitulations were not read.
pub type Capitulations = Result<Vec<(u32, i32)>, ()>;

pub fn encode_capitulations(capitulations: &Capitulations) -> Vec<u8> {
    let mut out = Out::default();
    out.flag(capitulations.is_ok());
    if let Ok(rows) = capitulations {
        out.var(rows.len() as u64);
        for &(country, hour) in rows {
            out.word(country);
            out.int(hour);
        }
    }
    out.into_bytes()
}

pub fn decode_capitulations(bytes: &[u8]) -> io::Result<Capitulations> {
    whole(bytes, |d| {
        Ok(if d.flag()? {
            Ok(items(d, |d| Ok((word(d)?, int(d)?)))?)
        } else {
            Err(())
        })
    })
}

pub fn encode_names(names: &[(u32, String)]) -> Vec<u8> {
    let mut out = Out::default();
    out.var(names.len() as u64);
    for (raw, name) in names {
        out.word(*raw);
        out.text(name);
    }
    out.into_bytes()
}

pub fn decode_names(bytes: &[u8]) -> io::Result<Vec<(u32, String)>> {
    whole(bytes, |d| items(d, |d| Ok((word(d)?, text(d)?))))
}

/// A group's members as the blob holds them: its viewpoint's side, then the other, in native
/// order with any repeat.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Members {
    pub first: Vec<u32>,
    pub second: Vec<u32>,
}

impl Members {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        for side in [&self.first, &self.second] {
            out.var(side.len() as u64);
            for &slot in side {
                out.word(slot);
            }
        }
        out.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        whole(bytes, |d| {
            Ok(Self {
                first: items(d, word)?,
                second: items(d, word)?,
            })
        })
    }
}

/// A group's representative relation (its slots, ascending) and the name the viewpoint gives it,
/// or why it has none, each refusal kept without the hour a read words it with ([`Failure`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Label {
    pub representative: Result<(u32, u32), Failure>,
    /// The name is the representative's refusal where it has none.
    pub name: Result<WarName, Failure>,
}

impl Label {
    /// The representative and its name, or why it has none, as naming at `hour` words them.
    pub(crate) fn at(&self, hour: i32) -> Result<(Pair, WarName), String> {
        let p = self
            .representative
            .as_ref()
            .map_err(|failure| failure.at(hour))?;
        let name = match &self.name {
            Ok(name) => name.clone(),
            Err(failure) => naming::unknown(failure.at(hour)),
        };
        Ok((*p, name))
    }
    /// Whether naming words it with the hour it reads at.
    pub(crate) fn stamped(&self) -> bool {
        self.representative
            .as_ref()
            .err()
            .or(self.name.as_ref().err())
            .is_some_and(Failure::stamped)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        match &self.representative {
            Ok((a, b)) => {
                out.flag(true);
                out.word(*a);
                out.word(*b);
                match &self.name {
                    Ok(WarName {
                        text,
                        known,
                        reason,
                        key,
                    }) => {
                        out.flag(true);
                        out.text(text);
                        out.flag(*known);
                        for part in [reason, key] {
                            out.flag(part.is_some());
                            if let Some(part) = part {
                                out.text(part);
                            }
                        }
                    }
                    Err(failure) => {
                        out.flag(false);
                        failure.put(&mut out);
                    }
                }
            }
            Err(failure) => {
                out.flag(false);
                failure.put(&mut out);
            }
        }
        out.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        whole(bytes, |d| {
            if !d.flag()? {
                let failure = Failure::get(d)?;
                return Ok(Self {
                    name: Err(failure.clone()),
                    representative: Err(failure),
                });
            }
            let representative = Ok((word(d)?, word(d)?));
            if !d.flag()? {
                return Ok(Self {
                    representative,
                    name: Err(Failure::get(d)?),
                });
            }
            let text_ = text(d)?;
            let known = d.flag()?;
            let reason = if d.flag()? { Some(text(d)?) } else { None };
            let key = if d.flag()? { Some(text(d)?) } else { None };
            Ok(Self {
                representative,
                name: Ok(WarName {
                    text: text_,
                    known,
                    reason,
                    key,
                }),
            })
        })
    }
}

/// One native group a viewpoint sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub members: Members,
    pub label: Label,
}

/// A viewpoint's groups as published: each group's members' and label's ordinals.
pub fn encode_list(list: &[(u32, u32)]) -> Vec<u8> {
    let mut out = Out::default();
    out.var(list.len() as u64);
    for &(members, label) in list {
        out.word(members);
        out.word(label);
    }
    out.into_bytes()
}

pub fn decode_list(bytes: &[u8]) -> io::Result<Vec<(u32, u32)>> {
    whole(bytes, |d| items(d, |d| Ok((word(d)?, word(d)?))))
}

/// The at-war viewpoints' groups an hour hands on: every one's, those absent seeing none, or only
/// those that changed, a viewpoint no longer at war with none.
pub enum Groups {
    Whole(Vec<(Identity, Vec<Group>)>),
    Changed(Vec<(Identity, Vec<Group>)>),
}

/// What an hour changed of the layers.
#[derive(Default)]
pub struct Layered {
    pub roster: Option<Roster>,
    pub state: Option<State>,
    pub relations: Option<Vec<Relation>>,
    pub capitulations: Option<Capitulations>,
    pub available: Option<bool>,
    /// Historical names read for the first time this segment, by raw country id.
    pub names: Vec<(u32, String)>,
    pub groups: Option<Groups>,
}

impl Layered {
    fn is_empty(&self) -> bool {
        self.roster.is_none()
            && self.state.is_none()
            && self.relations.is_none()
            && self.capitulations.is_none()
            && self.available.is_none()
            && self.names.is_empty()
            && self.groups.is_none()
    }

    /// An estimate of what it holds on the heap.
    pub fn heap(&self) -> usize {
        let roster: usize = self.roster.as_ref().map_or(0, |roster| {
            roster.countries.as_ref().map_or(0, |rows| {
                rows.iter().map(|(_, _, tag)| 32 + tag.len()).sum::<usize>()
            }) + roster.links.as_ref().map_or(0, |rows| 8 * rows.len())
        });
        let groups: usize = match &self.groups {
            Some(Groups::Whole(lists) | Groups::Changed(lists)) => lists
                .iter()
                .flat_map(|(_, list)| list)
                .map(|group| {
                    96 + 4 * (group.members.first.len() + group.members.second.len())
                        + match &group.label.name {
                            Ok(name) => name.text.len(),
                            Err(failure) => failure.text.len(),
                        }
                })
                .sum::<usize>(),
            None => 0,
        };
        roster
            + groups
            + self.relations.as_ref().map_or(0, |list| 16 * list.len())
            + self
                .capitulations
                .as_ref()
                .map_or(0, |held| held.as_ref().map_or(0, |rows| 8 * rows.len()))
            + self
                .names
                .iter()
                .map(|(_, name)| 32 + name.len())
                .sum::<usize>()
    }
}

/// One at-war viewpoint's group as the layers keep it from hour to hour.
struct Kept {
    blob: Blob,
    oldest: Result<Pair, String>,
    /// The sides [`World::ranked`] sorts, sorted: alike for every viewpoint seeing these members
    /// from this oldest relation. None where a side repeats a member, whose order then ranks.
    sorted: Option<(Vec<u32>, Vec<u32>)>,
    label: Label,
    /// Whether naming read the war overview's scores.
    scored: bool,
}

impl Kept {
    fn group(&self) -> Group {
        Group {
            members: Members {
                first: self.blob.first.clone(),
                second: self.blob.second.clone(),
            },
            label: self.label.clone(),
        }
    }
}

struct Viewpoint {
    slot: u32,
    identity: Identity,
    groups: Vec<Kept>,
}

/// Representatives ranked at an hour, by the oldest relation and the sorted sides.
type Ranks = HashMap<(Pair, Vec<u32>, Vec<u32>), Result<Pair, String>>;

/// The sides [`World::ranked`] sorts for `blob` from `oldest`, the one holding its first country
/// first, each sorted; none where a side repeats a member.
fn sorted(world: &World, blob: &Blob, oldest: Pair) -> Option<(Vec<u32>, Vec<u32>)> {
    let first = world.relations[&oldest].first_country;
    let (a, b) = if blob.first.contains(&first) {
        (&blob.first, &blob.second)
    } else {
        (&blob.second, &blob.first)
    };
    let unique = |side: &Vec<u32>| {
        let mut held = side.clone();
        held.sort_unstable();
        let len = held.len();
        held.dedup();
        (held.len() == len).then_some(held)
    };
    Some((unique(a)?, unique(b)?))
}

/// Whether every member of `side` has a score and no two share a major byte and strength: the
/// stable sort then orders them alike from any order.
fn untied(world: &World, side: &[u32]) -> bool {
    let mut seen = HashSet::with_capacity(side.len());
    side.iter()
        .all(|&x| world.score(x).is_ok_and(|score| seen.insert(score)))
}

/// `blob`'s representative from `oldest`, ranked once for every viewpoint whose sorted sides
/// rank alike.
fn rank(
    world: &World,
    blob: &Blob,
    oldest: &Result<Pair, String>,
    sorted: Option<&(Vec<u32>, Vec<u32>)>,
    ranks: &mut Ranks,
) -> Result<Pair, String> {
    let p = *oldest.as_ref().map_err(Clone::clone)?;
    match sorted {
        Some((a, b)) if untied(world, a) && untied(world, b) => {
            let key = (p, a.clone(), b.clone());
            if let Some(held) = ranks.get(&key) {
                return held.clone();
            }
            let ranked = world.ranked(
                &Blob {
                    first: a.clone(),
                    second: b.clone(),
                },
                p,
            );
            ranks.insert(key, ranked.clone());
            ranked
        }
        _ => world.ranked(blob, p),
    }
}

/// The label `hour`'s viewpoint gives a group represented by `representative`, naming at `at`,
/// and whether naming it read the overview's scores.
fn label(
    namer: &Namer<'_>,
    hour: &Hour,
    at: i32,
    representative: Result<Pair, Failure>,
) -> (Label, bool) {
    match representative {
        Ok(p) => {
            let (mut name, scored) = namer.name(hour, p);
            // A name with a reason is the refusal's alone.
            let name = match name.reason.take() {
                Some(reason) => Err(Failure::of(reason, at)),
                None => Ok(name),
            };
            (
                Label {
                    representative: Ok(p),
                    name,
                },
                scored,
            )
        }
        Err(failure) => (
            Label {
                name: Err(failure.clone()),
                representative: Err(failure),
            },
            false,
        ),
    }
}

/// The tables of `INPUTS` among `tables`, one bit by place.
fn bits(tables: &[TableId]) -> u32 {
    tables
        .iter()
        .filter_map(|&table| inputs::input(table))
        .fold(0, |set, place| set | 1 << place)
}

/// The input tables whose change each layer reads again, one bit by [`INPUTS`] place.
struct Masks {
    /// Every table but the scores and the capitulations: the countries, the war world, the
    /// groups and their names.
    full: u32,
    scores: u32,
    capitulations: u32,
    available: u32,
    naming: u32,
}

/// The layers of a segment, computed hour by hour from the tables a walk reads there.
pub struct Layers {
    cat: Option<Arc<Catalogue>>,
    masks: Masks,
    labels: labels::CountryLabels,
    /// The hour's countries and war world while they read; its view the last grouped.
    hour: Option<Hour>,
    viewpoints: Vec<Viewpoint>,
    /// Each layer as last handed on this segment.
    roster: Option<Roster>,
    state: Option<State>,
    relations: Option<Vec<Relation>>,
    capitulations: Option<Capitulations>,
    available: Option<bool>,
    /// Each at-war identity's groups as last handed on; none before the segment's first.
    sent: Option<HashMap<Identity, Vec<Group>>>,
    /// Each viewpoint's groups of the held hour's war world, which they alone read.
    grouped: HashMap<u32, Vec<Blob>>,
    /// The held hour's war world is the one `grouped` was grouped from.
    same_world: bool,
}

impl Layers {
    pub fn new(cat: Option<Arc<Catalogue>>) -> Self {
        let all = (1_u32 << INPUTS.len()) - 1;
        let scores = bits(&[TableId::WarOverviewCountries]);
        let capitulations = bits(&[TableId::Capitulation]);
        Self {
            cat,
            masks: Masks {
                full: all & !scores & !capitulations,
                scores,
                capitulations,
                available: bits(&[
                    TableId::WarOwnedRelations,
                    TableId::WarRelationGoals,
                    TableId::WarOverviewCountries,
                    TableId::WarNameRules,
                ]),
                naming: bits(&installed_content_country_names::country_names::TABLES),
            },
            labels: Default::default(),
            hour: None,
            viewpoints: Vec::new(),
            roster: None,
            state: None,
            relations: None,
            capitulations: None,
            available: None,
            sent: None,
            grouped: HashMap::new(),
            same_world: false,
        }
    }

    /// What `frame`, the tables at an hour, changed of the layers: every one read again when
    /// `all`, else those whose tables `changed` (one bit by [`INPUTS`] place).
    pub fn hour(&mut self, frame: &DecodedFrame, all: bool, changed: u32) -> Option<Layered> {
        let mut out = Layered::default();
        if all || changed & self.masks.naming != 0 {
            self.labels.observe(frame, self.cat.as_deref());
            out.names = self.labels.take_fresh();
        }
        if all || changed & self.masks.available != 0 {
            let now = available(frame);
            if self.available != Some(now) {
                self.available = Some(now);
                out.available = Some(now);
            }
        }
        if all || changed & self.masks.capitulations != 0 {
            let now = capitulations(frame);
            if self.capitulations.as_ref() != Some(&now) {
                out.capitulations = Some(now.clone());
                self.capitulations = Some(now);
            }
        }
        let scored = changed & self.masks.scores != 0;
        if all || changed & self.masks.full != 0 || (scored && self.hour.is_none()) {
            self.full(frame, &mut out);
        } else if scored {
            self.scored(frame, &mut out);
        }
        (!out.is_empty()).then_some(out)
    }

    fn send_state(&mut self, state: State, out: &mut Layered) {
        if self.state.as_ref() != Some(&state) {
            out.state = Some(state.clone());
            self.state = Some(state);
        }
    }

    /// Reads every layer but the names, availability and capitulations again.
    fn full(&mut self, frame: &DecodedFrame, out: &mut Layered) {
        let at = frame.hour;
        let roster = Roster::read(frame);
        if self.roster.as_ref() != Some(&roster) {
            out.roster = Some(roster.clone());
            self.roster = Some(roster);
        }
        let before = self.hour.take();
        let Ok(countries) = Countries::read(frame) else {
            return;
        };
        let recorded = recorded_view(frame, &countries);
        let mut state = State {
            recorded_view: recorded,
            owned: None,
            rest: None,
            resolvable: inputs::lifetimes(frame)
                .is_some_and(|lifetimes| lifetimes.iter().any(Option::is_some)),
        };
        let read = match owned(frame, &countries) {
            Err(failure) => {
                state.owned = Some(Failure::of(failure, at));
                None
            }
            Ok((owned, goal_owned)) => match world(frame, &countries, recorded, owned) {
                Err(failure) => {
                    state.rest = Some(Failure::of(failure, at));
                    None
                }
                Ok((world, order)) => Some((world, order, goal_owned)),
            },
        };
        self.send_state(state, out);
        let Some(read) = read else {
            return;
        };
        let hour = Hour::of(countries, read, 0, recorded);
        // Grouping reads the owned lists, the active relations, adjacency and factions alone.
        self.same_world = before.as_ref().is_some_and(|before| {
            before.world.owned == hour.world.owned
                && before.world.adjacent == hour.world.adjacent
                && before.world.factions == hour.world.factions
                && before.world.relations.len() == hour.world.relations.len()
                && before
                    .world
                    .relations
                    .keys()
                    .all(|key| hour.world.relations.contains_key(key))
        });
        let relations: Vec<Relation> = hour
            .relation_order
            .iter()
            .map(|p| {
                let r = &hour.world.relations[p];
                Relation {
                    first: r.first_country,
                    second: r.second_country,
                    start: r.start_hour,
                    instigator_first: r.first_was_instigator != 0,
                }
            })
            .collect();
        if self.relations.as_ref() != Some(&relations) {
            out.relations = Some(relations.clone());
            self.relations = Some(relations);
        }
        self.hour = Some(hour);
        self.regroup(frame, out);
    }

    /// Groups every viewpoint at war again.
    fn regroup(&mut self, frame: &DecodedFrame, out: &mut Layered) {
        let at = frame.hour;
        let cat = self.cat.clone();
        let namer = Namer::new(frame, cat.as_deref());
        let Some(hour) = self.hour.as_mut() else {
            return;
        };
        let mut owners: Vec<u32> = hour.world.owned.keys().copied().collect();
        owners.sort_unstable();
        let mut ranks = Ranks::new();
        self.viewpoints.clear();
        let mut held = std::mem::take(&mut self.grouped);
        if !std::mem::take(&mut self.same_world) {
            held.clear();
        }
        for slot in owners {
            hour.view = slot;
            let blobs = held
                .remove(&slot)
                .unwrap_or_else(|| hour.world.blobs(slot));
            self.grouped.insert(slot, blobs.clone());
            if blobs.is_empty() {
                continue;
            }
            let hour = &*hour;
            let groups = blobs
                .into_iter()
                .map(|blob| {
                    let oldest = hour.world.oldest(&blob);
                    let sorted = oldest
                        .as_ref()
                        .ok()
                        .and_then(|&p| sorted(&hour.world, &blob, p));
                    let representative =
                        rank(&hour.world, &blob, &oldest, sorted.as_ref(), &mut ranks)
                            .map_err(|reason| Failure::of(reason, at));
                    let (label, scored) = label(&namer, hour, at, representative);
                    Kept {
                        blob,
                        oldest,
                        sorted,
                        label,
                        scored,
                    }
                })
                .collect();
            self.viewpoints.push(Viewpoint {
                slot,
                identity: hour.identities[&slot],
                groups,
            });
        }
        let lists: HashMap<Identity, Vec<Group>> = self
            .viewpoints
            .iter()
            .map(|viewpoint| {
                (
                    viewpoint.identity,
                    viewpoint.groups.iter().map(Kept::group).collect(),
                )
            })
            .collect();
        let handed = match &self.sent {
            None => {
                let mut whole: Vec<_> = lists
                    .iter()
                    .map(|(&identity, list)| (identity, list.clone()))
                    .collect();
                whole.sort_unstable_by_key(|(identity, _)| *identity);
                Some(Groups::Whole(whole))
            }
            Some(sent) => {
                let mut changed: Vec<_> = lists
                    .iter()
                    .filter(|&(identity, list)| sent.get(identity) != Some(list))
                    .map(|(&identity, list)| (identity, list.clone()))
                    .chain(
                        sent.keys()
                            .filter(|identity| !lists.contains_key(*identity))
                            .map(|&identity| (identity, Vec::new())),
                    )
                    .collect();
                changed.sort_unstable_by_key(|(identity, _)| *identity);
                (!changed.is_empty()).then_some(Groups::Changed(changed))
            }
        };
        out.groups = handed;
        self.sent = Some(lists);
    }

    /// Ranks every group again after a change of the war overview's scores alone, naming again
    /// those whose representative changed or whose name read the scores.
    fn scored(&mut self, frame: &DecodedFrame, out: &mut Layered) {
        let at = frame.hour;
        let Some(hour) = self.hour.as_mut() else {
            return;
        };
        match scores(frame, &hour.tags, &hour.raw_to_slot) {
            Ok(scores) => hour.world.scores = scores,
            Err(failure) => {
                // The next change reads every table again.
                self.hour = None;
                if let Some(state) = self.state.clone() {
                    let state = State {
                        rest: Some(Failure::of(failure, at)),
                        ..state
                    };
                    self.send_state(state, out);
                }
                return;
            }
        }
        let cat = self.cat.clone();
        let namer = Namer::new(frame, cat.as_deref());
        let mut ranks = Ranks::new();
        let mut changed = Vec::new();
        for viewpoint in &mut self.viewpoints {
            hour.view = viewpoint.slot;
            let mut moved = false;
            for kept in &mut viewpoint.groups {
                let representative = rank(
                    &hour.world,
                    &kept.blob,
                    &kept.oldest,
                    kept.sorted.as_ref(),
                    &mut ranks,
                )
                .map_err(|reason| Failure::of(reason, at));
                // Kept without their hour, the names that read no score are as they were.
                if representative != kept.label.representative || kept.scored {
                    let (label, scored) = label(&namer, hour, at, representative);
                    kept.scored = scored;
                    if label != kept.label {
                        kept.label = label;
                        moved = true;
                    }
                }
            }
            if moved {
                changed.push((
                    viewpoint.identity,
                    viewpoint.groups.iter().map(Kept::group).collect::<Vec<_>>(),
                ));
            }
        }
        if changed.is_empty() {
            return;
        }
        let sent = self.sent.get_or_insert_with(HashMap::new);
        for (identity, list) in &changed {
            sent.insert(*identity, list.clone());
        }
        out.groups = Some(Groups::Changed(changed));
    }
}

/// How a reply reads its viewpoint.
pub(crate) enum Mode<'a> {
    /// The recording client's playing or observer country.
    Recorded,
    Tag(&'a str),
    /// The one native holder among these identities, by its tag at the hour.
    Selected(&'a HashSet<(u32, u32)>),
}

/// The slot `mode` views at an hour reading `roster`, its countries `countries` and `state`, or
/// why it cannot.
pub(crate) fn viewpoint(
    mode: &Mode<'_>,
    roster: &Roster,
    countries: &Result<Arc<Countries>, Failure>,
    state: Option<&State>,
) -> Result<u32, Failure> {
    let tag = match mode {
        Mode::Selected(selected) => {
            let rows = roster.countries.as_ref().map_err(Clone::clone)?;
            Some(
                selected_holder(
                    rows.iter()
                        .map(|(slot, id, tag)| (*slot, *id, tag.as_slice())),
                    selected,
                )
                .map_err(Failure::plain)?,
            )
        }
        Mode::Tag(tag) => Some((*tag).to_owned()),
        Mode::Recorded => None,
    };
    let countries = countries.as_ref().map_err(Clone::clone)?;
    match tag {
        Some(tag) => countries
            .holding(&tag)
            .ok_or_else(|| Failure::plain(format!("Not in this recording: viewpoint {tag}"))),
        None => state
            .and_then(|state| state.recorded_view)
            .ok_or_else(|| Failure::plain("Not recorded: local playing/observer country")),
    }
}

/// The slot `mode` views at an hour, or why the hour cannot be read: as a walk reads it, or as
/// one requested hour does (`counted`), whose relations hold the prepared counters, none where
/// the war tables past the owned occurrences did not read.
pub(crate) fn read(
    mode: &Mode<'_>,
    roster: &Roster,
    countries: &Result<Arc<Countries>, Failure>,
    state: Option<&State>,
    counted: bool,
) -> Result<u32, Failure> {
    let view = viewpoint(mode, roster, countries, state)?;
    let state = state.ok_or_else(|| Failure::plain("Not recorded: war overview state"))?;
    if let Some(failure) = &state.owned {
        return Err(failure.clone());
    }
    if let Some(failure) = &state.rest {
        return Err(if counted && state.resolvable {
            Failure {
                stamp: Some(0),
                text: format!(
                    "{:?} is unavailable: Not recorded: war casualty counters at this hour",
                    TableId::WarRelations
                ),
            }
        } else {
            failure.clone()
        });
    }
    Ok(view)
}
