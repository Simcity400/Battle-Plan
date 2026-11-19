//! Unit type-icon and plate allegiance, independent of fog.
//!
//! Roster and logical owners remain distinct from expeditionary owners. Identity
//! qualifies as friendly; active war vetoes faction or direct subject friendship.
//! Access, attaches and common enemies do not establish friendship. Both owner
//! countries participate, then ordered volunteer entries may add a flag. Friendship
//! wins over hostility. Land units, railway guns and at-sea task forces share this
//! path, including hidden icons; port items retain their type icon's authored colour.
//! The same flags select the plate allegiance ([`plate`]).
use replay_data::{rows::DivisionRow, rows::SentVolunteerRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_foundation::map::{
    allegiance::Allegiance,
    directed_wars::Owner,
    memo::Keyed,
    visibility::{self, Families},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Friend,
    Enemy,
    Neutral,
}

/// Select allegiance before coarse and detailed draw paths. Own identity
/// wins; otherwise use the type icon's friend and enemy flags in their
/// existing order.
pub(super) fn plate(own: bool, side: Option<Side>) -> Allegiance {
    match side {
        _ if own => Allegiance::Ours,
        Some(Side::Friend) => Allegiance::Allied,
        Some(Side::Enemy) => Allegiance::Enemy,
        Some(Side::Neutral) | None => Allegiance::Other,
    }
}

/// `NGraphics.FRIEND_COLOR`, `ENEMY_COLOR` and `NEUTRAL_COLOR`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Colors {
    friend: [f32; 4],
    enemy: [f32; 4],
    neutral: [f32; 4],
}

impl Colors {
    pub(super) fn read(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let color = |name| -> io::Result<[f32; 4]> {
            let values = defines.numbers("NGraphics", name)?;
            let [r, g, b]: [f64; 3] = values.try_into().map_err(|_: Vec<f64>| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("NDefines.NGraphics.{name} is not three numbers"),
                )
            })?;
            // Set alpha to one for content RGB colours; explicit colour updates replace
            // all four components.
            Ok([r as f32, g as f32, b as f32, 1.0])
        };
        Ok(Self {
            friend: color("FRIEND_COLOR")?,
            enemy: color("ENEMY_COLOR")?,
            neutral: color("NEUTRAL_COLOR")?,
        })
    }

    pub(super) fn of(self, side: Side) -> [f32; 4] {
        match side {
            Side::Friend => self.friend,
            Side::Enemy => self.enemy,
            Side::Neutral => self.neutral,
        }
    }

    /// The shipped `00_defines.lua` values, for tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) const VANILLA: Self = Self {
        friend: [0.7, 0.9, 0.7, 1.0],
        enemy: [1.0, 0.7, 0.7, 1.0],
        neutral: [1.0, 1.0, 1.0, 1.0],
    };

    /// Three distinct non-vanilla colours, for tests only.
    #[cfg(test)]
    pub(super) const TEST: Self = Self {
        friend: [0.2, 0.8, 0.3, 1.0],
        enemy: [0.9, 0.1, 0.4, 1.0],
        neutral: [0.8, 0.6, 0.4, 1.0],
    };
}

/// The tables [`Index`] is built from besides its volunteer and hour [`Key`].
pub const TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::FactionMembers,
    TableId::Countries,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
    TableId::Subjects,
];

/// Recent indexes, keyed on [`TABLES`] and the hour's [`Key`].
pub type Memo = Keyed<Index, Key>;

/// What the index reads from the hour's Countries, Divisions and SentVolunteers.
#[derive(Clone, PartialEq)]
pub struct Key {
    ours: u32,
    /// Each volunteer host's first position in the viewing country's list, in
    /// list order.
    hosts: Result<Vec<(u32, u32)>, String>,
    /// Directed packet admission is tied to the presented hour.
    hour: i32,
}

impl Key {
    fn read(
        sent: &[SentVolunteerRow],
        divisions: &[DivisionRow],
        links: &BTreeMap<u32, u32>,
        ours: u32,
        hour: i32,
    ) -> Result<Self, String> {
        // Rows are in key order (country, ordinal); only the viewing country's
        // own object holds a list.
        let list = replay_data::military_aid::volunteer_list(
            links
                .iter()
                .filter(|&(_, &slot)| slot == ours)
                .map(|(&raw, _)| {
                    let start = sent.partition_point(|row| row.country < raw);
                    let end = sent.partition_point(|row| row.country <= raw);
                    &sent[start..end]
                }),
        )?;
        let hosts = volunteer_hosts(
            list.iter().map(|row| (row.ordinal, row.division)),
            |reference| {
                replay_data::military_aid::live_division(divisions, reference)
                    .map(replay_data::military_aid::volunteer_host)
            },
            links,
        )?;
        Ok(Self {
            ours,
            hosts: Ok(hosts),
            hour,
        })
    }
}

/// Each volunteer host's first position in the viewing country's list `(ordinal, division)`, in
/// list order; `host_of` is a live division's logical owner (raw reference).
pub fn volunteer_hosts(
    list: impl IntoIterator<Item = (u32, u64)>,
    host_of: impl Fn(u64) -> Option<u32>,
    links: &BTreeMap<u32, u32>,
) -> Result<Vec<(u32, u32)>, String> {
    let mut seen = BTreeSet::new();
    let mut hosts = Vec::new();
    for (ordinal, division) in list {
        let Some(host) = host_of(division) else {
            continue;
        };
        let slot = canonical(links, host)
            .ok_or("SentVolunteers logical country missing from CountryLinks")?;
        // Actual zero does not alias a positive reference resolving to zero.
        if seen.insert((host != 0, slot)) {
            hosts.push((host, ordinal));
        }
    }
    Ok(hosts)
}

/// One receiver country's direct relations, as the index asks them ([`Owner`]'s answers).
pub trait Receiver {
    fn same_country(&self, raw: u32) -> Result<bool, String>;
    fn at_war(&self, raw: u32) -> Result<bool, String>;
}

impl Receiver for Owner<'_> {
    fn same_country(&self, raw: u32) -> Result<bool, String> {
        Owner::same_country(self, raw)
    }

    fn at_war(&self, raw: u32) -> Result<bool, String> {
        Owner::at_war(self, raw)
    }
}

/// What the index reads of one hour's relations: a frame's ([`Index::read`]) or a layer's kept
/// from change sets.
pub trait RelationFacts {
    type Receiver<'r>: Receiver
    where
        Self: 'r;
    /// The receiver country of a raw reference.
    fn receiver(&self, raw: u32) -> Result<Self::Receiver<'_>, String>;
    /// [`visibility::counter_shared`]: whether the viewer shares a counter's vision with `raw`.
    fn counter_shared(&self, viewer: &Self::Receiver<'_>, raw: u32) -> Result<bool, String>;
}

struct FrameFacts<'a> {
    frame: &'a DecodedFrame,
    links: &'a BTreeMap<u32, u32>,
    families: Result<Families, &'static str>,
}

impl RelationFacts for FrameFacts<'_> {
    type Receiver<'r>
        = Owner<'r>
    where
        Self: 'r;

    fn receiver(&self, raw: u32) -> Result<Owner<'_>, String> {
        Owner::new(self.frame, raw)
    }

    fn counter_shared(&self, viewer: &Owner<'_>, raw: u32) -> Result<bool, String> {
        visibility::counter_shared(self.frame, self.links, viewer, raw, self.families.as_ref().map_err(|reason| *reason))
    }
}

/// The viewing country's receiver: its raw reference at slot `ours` (zero for slot zero).
fn viewer_of<'f, F: RelationFacts>(facts: &'f F, links: &BTreeMap<u32, u32>, ours: u32) -> Result<F::Receiver<'f>, String> {
    if ours == 0 {
        Ok(0)
    } else {
        links
            .iter()
            .find(|&(_, &slot)| slot == ours)
            .map(|(&raw, _)| raw)
            .ok_or_else(|| "viewing country CountryLinks row missing".to_owned())
    }
    .and_then(|raw| facts.receiver(raw))
}

/// One country's relation to the viewer and to the viewer's volunteer hosts.
fn country<F: RelationFacts>(facts: &F, viewer: &Result<F::Receiver<'_>, String>, links: &BTreeMap<u32, u32>, hosts: &[(u32, u32)], raw: u32) -> Country {
    let friend = viewer.as_ref().map_err(Clone::clone).and_then(|viewer| facts.counter_shared(viewer, raw));
    let enemy = viewer.as_ref().map_err(Clone::clone).and_then(|viewer| viewer.at_war(raw));
    let first_host = hosts
        .iter()
        .find(|&&(host, _)| raw == host || (raw != 0 && host != 0 && canonical(links, raw) == canonical(links, host)))
        .map(|&(_, position)| position);
    let mut first_host_enemy = None;
    if !hosts.is_empty() {
        let receiver = facts.receiver(raw);
        for &(host, position) in hosts {
            match receiver.as_ref().map_err(Clone::clone).and_then(|receiver| receiver.at_war(host)) {
                Ok(false) => {}
                Ok(true) => {
                    first_host_enemy = Some((position, Ok(())));
                    break;
                }
                Err(reason) => {
                    first_host_enemy = Some((position, Err(reason)));
                    break;
                }
            }
        }
    }
    Country { friend, enemy, first_host, first_host_enemy }
}

/// Each country's relation to the viewing country, by raw reference.
///
/// The first matching sent volunteer decides the loop :
/// each host keeps its first position, and each country at war with a host the
/// earliest such position. A lookup never walks divisions or volunteer lists.
#[derive(Debug, Default, PartialEq)]
pub struct Index {
    /// Held in shards, so an index made again for a few countries copies the shards they stand in
    /// and shares the rest.
    countries: super::Shards<u32, Country>,
    problem: Option<String>,
    /// Per country whose relation is refused, its reason, which decides `query_problem` without
    /// reading every country.
    problems: BTreeMap<u32, String>,
    query_problem: Option<String>,
    volunteer_problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Country {
    friend: Result<bool, String>,
    enemy: Result<bool, String>,
    first_host: Option<u32>,
    /// First hostile or unreadable host, in native order. Equality at that
    /// position still wins before its war query.
    first_host_enemy: Option<(u32, Result<(), String>)>,
}

fn either(a: Result<bool, String>, b: Result<bool, String>) -> Result<bool, String> {
    if a == Ok(true) || b == Ok(true) {
        return Ok(true);
    }
    a?;
    b
}

impl Index {
    /// This hour's index: a recent hour's while [`TABLES`] are the same
    /// allocations and the [`Key`] is equal.
    pub(super) fn read(
        memo: &Memo,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        ours: u32,
    ) -> Arc<Self> {
        let key = (|| {
            Key::read(
                frame
                    .complete::<tables::SentVolunteers>()
                    .map_err(|e| format!("SentVolunteers: {e}"))?,
                frame
                    .complete::<tables::Divisions>()
                    .map_err(|e| format!("Divisions: {e}"))?,
                links,
                ours,
                frame.hour,
            )
        })();
        let key = match key {
            Ok(key) => key,
            Err(problem) => Key {
                ours,
                hour: frame.hour,
                hosts: Err(problem),
            },
        };
        memo.get_or_build_without_waiting(frame, key.clone(), |projected| {
            let links: BTreeMap<u32, u32> = projected
                .complete::<tables::CountryLinks>()?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            Ok::<_, io::Error>(Arc::new(Self::new(projected, &links, &key)))
        })
        .unwrap_or_else(|error| Arc::new(Self::unavailable(error.to_string())))
    }

    fn unavailable(problem: String) -> Self {
        Self {
            problem: Some(problem),
            ..Self::default()
        }
    }

    /// `raw`'s relation decided from `facts`, with the reason it is refused kept for
    /// [`Self::query_problem`]; whether it changed.
    fn decide<F: RelationFacts>(&mut self, facts: &F, viewer: &Result<F::Receiver<'_>, String>, links: &BTreeMap<u32, u32>, hosts: &[(u32, u32)], raw: u32) -> bool {
        if raw != 0 && !links.contains_key(&raw) {
            self.problems.remove(&raw);
            return self.countries.remove(&raw);
        }
        let decided = country(facts, viewer, links, hosts, raw);
        match refusal(&decided) {
            Some(reason) => self.problems.insert(raw, reason),
            None => self.problems.remove(&raw),
        };
        self.countries.insert(raw, decided)
    }

    /// The first refusal in the order the countries are decided: the volunteers', then the lowest
    /// raw reference's, country zero's last.
    fn query_problem(&self) -> Option<String> {
        self.volunteer_problem.clone().or_else(|| self.problems.range(1..).next().or_else(|| self.problems.get_key_value(&0)).map(|(_, reason)| reason.clone()))
    }

    fn new(frame: &DecodedFrame, links: &BTreeMap<u32, u32>, key: &Key) -> Self {
        let facts = FrameFacts { frame, links, families: Families::read(frame, links) };
        Self::compute(&facts, links, key.ours, key.hosts.as_deref().map_err(Clone::clone))
    }

    /// The index from `facts`: `links` raw to canonical slot, the viewing country's slot `ours`
    /// and its volunteer hosts ([`volunteer_hosts`]).
    pub fn compute<F: RelationFacts>(facts: &F, links: &BTreeMap<u32, u32>, ours: u32, hosts: Result<&[(u32, u32)], String>) -> Self {
        let mut index = Self { volunteer_problem: hosts.as_ref().err().cloned(), ..Self::default() };
        index.remake_in_place(facts, links, ours, hosts, links.keys().copied().chain([0]), &mut |_| {});
        index
    }

    /// The countries `raws` names decided anew from `facts` into this index, each one that changed
    /// handed to `changed`; every other country's relation is kept and the shards they stand in are
    /// not copied. A country `links` no longer holds leaves.
    pub fn remake_in_place<F: RelationFacts>(
        &mut self,
        facts: &F,
        links: &BTreeMap<u32, u32>,
        ours: u32,
        hosts: Result<&[(u32, u32)], String>,
        raws: impl IntoIterator<Item = u32>,
        changed: &mut dyn FnMut(u32),
    ) {
        let viewer = viewer_of(facts, links, ours);
        self.volunteer_problem = hosts.as_ref().err().cloned();
        let hosts = hosts.unwrap_or_default();
        self.problem = None;
        for raw in raws {
            if self.decide(facts, &viewer, links, hosts, raw) {
                changed(raw);
            }
        }
        self.query_problem = self.query_problem();
    }

    /// The same index, its countries' shards shared with this one until one of them changes.
    pub fn clone_shared(&self) -> Self {
        Self {
            countries: self.countries.clone(),
            problem: self.problem.clone(),
            problems: self.problems.clone(),
            query_problem: self.query_problem.clone(),
            volunteer_problem: self.volunteer_problem.clone(),
        }
    }

    /// An index the hour cannot make, for `problem`.
    pub fn unavailable_for(problem: String) -> Self {
        Self::unavailable(problem)
    }

    /// Countries whose relation differs from `other`'s (raw references), and the first described.
    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        let mut differing = 0;
        let mut first = None;
        for (what, a, b) in [("problem", &self.problem, &other.problem), ("query problem", &self.query_problem, &other.query_problem), ("volunteer problem", &self.volunteer_problem, &other.volunteer_problem)] {
            if a != b {
                differing += 1;
                first.get_or_insert(format!("{what}: {a:?} against {b:?}"));
            }
        }
        let raws: BTreeSet<u32> = self.countries.keys().chain(other.countries.keys()).collect();
        for raw in raws {
            let (a, b) = (self.countries.get(&raw), other.countries.get(&raw));
            if a != b {
                differing += 1;
                first.get_or_insert(format!("country {raw}: {a:?} against {b:?}"));
            }
        }
        (differing, first)
    }

    /// Countries (raw references) whose relation differs from `other`'s.
    pub fn changed_from(&self, other: &Self) -> Vec<u32> {
        let raws: BTreeSet<u32> = self.countries.keys().chain(other.countries.keys()).collect();
        raws.into_iter().filter(|raw| self.countries.get(raw) != other.countries.get(raw)).collect()
    }

    /// Why the viewer's volunteer hosts are not known, when they are not.
    pub fn volunteer_problem(&self) -> Option<&String> {
        self.volunteer_problem.as_ref()
    }

    /// The countries it holds a relation for, by raw reference, in key order.
    pub fn countries(&self) -> impl Iterator<Item = u32> + '_ {
        self.countries.keys()
    }

    /// Countries it holds a relation for.
    pub fn len(&self) -> usize {
        self.countries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.countries.is_empty()
    }

    /// Why [`Index::of`] answers `None`.
    pub(super) fn problem(&self) -> &str {
        self.problem
            .as_deref()
            .or(self.query_problem.as_deref())
            .unwrap_or("counter owner missing from CountryLinks")
    }

    pub fn of(&self, roster: u32, logical: u32) -> Option<Side> {
        // A signed nonpositive roster owner bypasses every relation predicate.
        if (roster as i32) <= 0 {
            return Some(Side::Friend);
        }
        if self.problem.is_some() {
            return None;
        }
        let a = self.countries.get(&roster)?;
        let b = self.countries.get(&logical)?;
        let mut friend = either(a.friend.clone(), b.friend.clone());
        let mut enemy = either(a.enemy.clone(), b.enemy.clone());
        if friend == Ok(true) {
            return Some(Side::Friend);
        }
        // Unknown volunteers may set friend even for an already-known enemy.
        // They cannot erase a known ordinary friend or affect a mission item.
        if self.volunteer_problem.is_some() {
            return None;
        }
        if let Some(host) = a.first_host
            && b.first_host_enemy
                .as_ref()
                .is_none_or(|(war, _)| host <= *war)
        {
            friend = Ok(true);
        } else if let Some((_, result)) = &b.first_host_enemy {
            if let Err(reason) = result {
                if a.first_host.is_some() {
                    // An earlier unknown war may stop a later friendly host.
                    friend = either(friend, Err(reason.clone()));
                }
                enemy = either(enemy, Err(reason.clone()));
            } else {
                enemy = Ok(true);
            }
        }
        // Test friend before enemy. A proved friend never demands
        // unknown enemy operands; an unknown friend cannot become neutral.
        Some(if friend.ok()? {
            Side::Friend
        } else if enemy.ok()? {
            Side::Enemy
        } else {
            Side::Neutral
        })
    }

    /// Port allegiance tests hostility before friendship using the viewing country
    /// and roster owner. Independent flags retain the owner and volunteer terms.
    pub(super) fn mission_item(&self, own: bool, roster: u32, logical: u32) -> Option<Allegiance> {
        if own {
            return Some(Allegiance::Ours);
        }
        if self.problem.is_some() {
            return None;
        }
        let a = self.countries.get(&roster)?;
        let b = self.countries.get(&logical)?;
        Some(if either(a.enemy.clone(), b.enemy.clone()).ok()? {
            Allegiance::Enemy
        } else if either(a.friend.clone(), b.friend.clone()).ok()? {
            Allegiance::Allied
        } else {
            Allegiance::Other
        })
    }
}

/// The reason one country's relation is refused, in the order the index reports it.
fn refusal(country: &Country) -> Option<String> {
    match (&country.friend, &country.enemy, &country.first_host_enemy) {
        (Err(reason), _, _) | (_, Err(reason), _) | (_, _, Some((_, Err(reason)))) => Some(reason.clone()),
        _ => None,
    }
}

fn canonical(links: &BTreeMap<u32, u32>, raw: u32) -> Option<u32> {
    if raw == 0 {
        Some(0)
    } else {
        links.get(&raw).copied()
    }
}

#[cfg(test)]
mod tests;
