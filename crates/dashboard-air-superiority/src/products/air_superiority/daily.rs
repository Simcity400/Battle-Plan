//! What preparation keeps of each finished day, and the composition of a
//! day's cells, activity and side uses from it. Preparation composes the
//! activity and uses of the native roster; queries compose the cells of the
//! roster's resolved sides with the same code.

use super::{AirCell, AirSideCell, AirValue, assignment::AssignedSide};
use dashboard_base::prepared::facts::{DecodeBudget, Resident};
use dashboard_core::invalid;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::Arc};

/// A country's friendly and enemy score in a region, after the duplicate
/// owner rule.
pub(super) type Pair = (String, (i64, i64));
/// A read of a country's or side's scores: unknown, no air object, or scores.
pub(super) type Score = Result<Option<(i64, i64)>, ()>;

fn budget(size: usize, parts: &[DecodeBudget]) -> DecodeBudget {
    parts.iter().fold(
        DecodeBudget {
            fixed_bytes: size,
            bytes_per_encoded_byte: 0,
        },
        |total, part| DecodeBudget {
            fixed_bytes: total.fixed_bytes.saturating_add(part.fixed_bytes),
            bytes_per_encoded_byte: total
                .bytes_per_encoded_byte
                .saturating_add(part.bytes_per_encoded_byte),
        },
    )
}

/// A side of a native roster other than a lone country, at its place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Group {
    pub position: u32,
    pub factions: Vec<u64>,
    pub leader: Option<String>,
    pub members: Vec<String>,
    pub co_belligerents: u64,
    pub colour: Option<u64>,
}
impl Resident for Group {
    fn heap_bytes(&self) -> usize {
        self.factions
            .heap_bytes()
            .saturating_add(self.leader.heap_bytes())
            .saturating_add(self.members.heap_bytes())
    }
    fn decode_budget() -> DecodeBudget {
        budget(
            size_of::<Self>(),
            &[
                Vec::<u64>::decode_budget(),
                Option::<String>::decode_budget(),
                Vec::<String>::decode_budget(),
            ],
        )
    }
}

/// The sides a day's sample hour assigns, keyless: its groups, and the lone
/// countries of the tag set `present` filling every other place in tag
/// order, as the assignment orders them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Roster {
    pub present: u32,
    pub groups: Vec<Group>,
}
impl Resident for Roster {
    fn heap_bytes(&self) -> usize {
        self.groups.heap_bytes()
    }
    fn decode_budget() -> DecodeBudget {
        budget(size_of::<Self>(), &[Vec::<Group>::decode_budget()])
    }
}

/// A day's score operands, their tag sets by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Operands {
    pub identity_known: bool,
    pub present: u32,
    pub witnessed: u32,
    pub invalid: u32,
    pub complete: bool,
    pub unresolved: bool,
}
impl Resident for Operands {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn decode_budget() -> DecodeBudget {
        budget(size_of::<Self>(), &[])
    }
}

/// A day's theatre membership: each theatre's regions, sorted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Theatres {
    pub known: bool,
    pub regions: Vec<(String, Vec<u32>)>,
}
impl Resident for Theatres {
    fn heap_bytes(&self) -> usize {
        self.regions.heap_bytes()
    }
    fn decode_budget() -> DecodeBudget {
        budget(
            size_of::<Self>(),
            &[Vec::<(String, Vec<u32>)>::decode_budget()],
        )
    }
}
impl Theatres {
    pub fn read(
        known: bool,
        theatres: &std::collections::BTreeMap<String, std::collections::BTreeSet<u32>>,
    ) -> Self {
        Self {
            known,
            regions: theatres
                .iter()
                .map(|(id, regions)| (id.clone(), regions.iter().copied().collect()))
                .collect(),
        }
    }
    /// The regions of the theatres `ids` names.
    pub fn selected(&self, ids: &[String]) -> std::collections::HashSet<u32> {
        ids.iter()
            .filter_map(|id| {
                self.regions
                    .binary_search_by(|(theatre, _)| theatre.as_str().cmp(id))
                    .ok()
            })
            .flat_map(|at| self.regions[at].1.iter().copied())
            .collect()
    }
}

/// A finished day: its sample's hour within the day, its roster (None where
/// no sides were recorded), operands and theatres by id, how long before the
/// sample the theatres were read, the sides its fallback and regions leave
/// out, and the roster places its sides are first shown at in the sides
/// view's order (None: every place, in order).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Day {
    pub offset: i32,
    pub roster: Option<u32>,
    pub operands: u32,
    pub theatres: u32,
    pub theatre_age: Option<i32>,
    pub excluded: u64,
    pub used: Option<Vec<u32>>,
}
impl Resident for Day {
    fn heap_bytes(&self) -> usize {
        self.used.heap_bytes()
    }
    fn decode_budget() -> DecodeBudget {
        budget(size_of::<Self>(), &[Option::<Vec<u32>>::decode_budget()])
    }
}

/// A sorted set of tags.
pub(super) type Tags = Arc<[String]>;

fn has(tags: &[String], tag: &str) -> bool {
    tags.binary_search_by(|held| held.as_str().cmp(tag)).is_ok()
}

/// The keyless roster of [`native_roster`](super::assignment::native_roster)'s
/// sides: its groups, and its lone countries' places.
pub(super) fn roster(sides: &[AssignedSide]) -> Vec<Group> {
    sides
        .iter()
        .enumerate()
        .filter(|(_, side)| !(side.factions.is_empty() && side.members.len() == 1))
        .map(|(position, side)| Group {
            position: position as u32,
            factions: side
                .factions
                .iter()
                .map(|(id, _)| id.parse::<u64>().expect("recorded faction id"))
                .collect(),
            leader: side.leader.clone(),
            members: side.members.clone(),
            co_belligerents: side.co_belligerents as u64,
            colour: side
                .colour_faction
                .as_ref()
                .map(|id| id.parse::<u64>().expect("recorded faction id")),
        })
        .collect()
}

/// A side of a [`Layout`].
#[derive(Clone, Copy)]
pub(super) enum Side<'a> {
    Lone(&'a String),
    Group(&'a Group),
}
impl<'a> Side<'a> {
    pub fn leader(self) -> Option<&'a str> {
        match self {
            Self::Lone(tag) => Some(tag),
            Self::Group(group) => group.leader.as_deref(),
        }
    }
    pub fn faction(self) -> bool {
        matches!(self, Self::Group(group) if !group.factions.is_empty())
    }
    pub fn members(self) -> &'a [String] {
        match self {
            Self::Lone(tag) => std::slice::from_ref(tag),
            Self::Group(group) => &group.members,
        }
    }
}

/// A roster's every side by place, its lone countries implied by its tag set.
pub(super) struct Layout {
    groups: Vec<Group>,
    present: Tags,
    members: Vec<String>,
    /// Each group member's group, by index.
    member_of: HashMap<String, usize>,
    len: u32,
}
impl Layout {
    pub fn new(mut groups: Vec<Group>, present: Tags) -> io::Result<Self> {
        groups.sort_by_key(|group| group.position);
        let mut member_of = HashMap::new();
        for (index, group) in groups.iter().enumerate() {
            for tag in &group.members {
                if member_of.insert(tag.clone(), index).is_some() || !has(&present, tag) {
                    return Err(invalid(
                        "Prepared air roster does not partition its countries",
                    ));
                }
            }
        }
        let mut members: Vec<String> = member_of.keys().cloned().collect();
        members.sort_unstable();
        let len = present.len() - members.len() + groups.len();
        if groups.iter().any(|group| group.members.is_empty())
            || groups
                .windows(2)
                .any(|pair| pair[0].position == pair[1].position)
            || groups
                .last()
                .is_some_and(|group| group.position as usize >= len)
        {
            return Err(invalid("Prepared air roster places are inconsistent"));
        }
        Ok(Self {
            groups,
            present,
            members,
            member_of,
            len: u32::try_from(len).map_err(|_| invalid("Prepared air roster is too large"))?,
        })
    }
    pub fn len(&self) -> u32 {
        self.len
    }
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
    pub fn group_index(&self, position: u32) -> Option<usize> {
        self.groups
            .binary_search_by_key(&position, |group| group.position)
            .ok()
    }
    pub fn side(&self, position: u32) -> Side<'_> {
        match self
            .groups
            .binary_search_by_key(&position, |group| group.position)
        {
            Ok(at) => Side::Group(&self.groups[at]),
            Err(before) => Side::Lone(self.lone(position as usize - before)),
        }
    }
    /// The `rank`th lone country in tag order.
    fn lone(&self, rank: usize) -> &String {
        let (mut low, mut high) = (0, self.present.len());
        while low < high {
            let middle = (low + high) / 2;
            let tag = &self.present[middle];
            let lone = middle + 1 - self.members.partition_point(|member| member <= tag);
            if lone <= rank {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        &self.present[low]
    }
    /// The place of `tag`'s side when it is a lone country.
    pub fn lone_position(&self, tag: &str) -> Option<u32> {
        let at = self
            .present
            .binary_search_by(|held| held.as_str().cmp(tag))
            .ok()?;
        if self.member_of.contains_key(tag) {
            return None;
        }
        let mut position =
            (at - self.members.partition_point(|member| member.as_str() < tag)) as u32;
        for group in &self.groups {
            if group.position > position {
                break;
            }
            position += 1;
        }
        Some(position)
    }
    /// The place of the side whose read takes `tag`'s scores: its lone
    /// country, its group's leader, or a member of a group without leader or
    /// faction.
    pub fn represented(&self, tag: &str) -> Option<u32> {
        match self.member_of.get(tag) {
            Some(&index) => {
                let group = &self.groups[index];
                match &group.leader {
                    Some(leader) => (leader == tag).then_some(group.position),
                    None => group.factions.is_empty().then_some(group.position),
                }
            }
            None => self.lone_position(tag),
        }
    }
    /// Whether `tag`'s scores decide whether the sides view is complete: a
    /// side's leader, or any member of a side without one.
    fn wanted(&self, tag: &str) -> bool {
        match self.member_of.get(tag) {
            Some(&index) => {
                let group = &self.groups[index];
                group.leader.as_deref().is_none_or(|leader| leader == tag)
            }
            None => has(&self.present, tag),
        }
    }
    fn lone_count(&self) -> usize {
        self.present.len() - self.members.len()
    }
    /// What it holds beyond its shared tag set.
    pub fn bytes(&self) -> usize {
        let text = |tag: &String| tag.capacity() + size_of::<String>();
        self.groups.capacity() * size_of::<Group>()
            + self
                .groups
                .iter()
                .map(|group| {
                    group.factions.capacity() * 8
                        + group.leader.as_ref().map_or(0, text)
                        + group.members.iter().map(text).sum::<usize>()
                })
                .sum::<usize>()
            + self.members.iter().map(text).sum::<usize>()
            + self
                .member_of
                .keys()
                .map(|tag| text(tag) + 16)
                .sum::<usize>()
    }
}

/// A day's score operands, their tag sets read.
pub(super) struct DayOperands {
    pub identity_known: bool,
    pub present: Tags,
    pub witnessed: Tags,
    pub invalid: Tags,
    pub complete: bool,
    pub unresolved: bool,
}

/// What a day's cells are composed for.
#[derive(Clone, Copy)]
pub(super) enum View<'a> {
    Sides,
    /// The side at this place, None where the side is not present.
    Side(Option<u32>),
    Country(&'a str),
}

#[derive(Clone, Copy)]
enum State {
    Identity,
    NoSides,
    Absent,
    Side(u32),
    Country,
    Sides,
}

/// A day's composition for one view: its operands, roster and the complete
/// rule the view's wanted countries give.
pub(super) struct Compose<'a> {
    layout: Option<&'a Layout>,
    operands: &'a DayOperands,
    country: &'a str,
    state: State,
    complete: bool,
    /// The places the fallback shows, None for every place.
    shown: Option<Vec<u32>>,
    excluded: usize,
}

fn reason(reason: &str) -> AirCell {
    AirCell(Some(AirValue::Reason(reason.into())))
}

/// Owners without a strategic-air object do not participate, and a known
/// zero adds no part.
fn hidden(side: Side, value: &Score) -> bool {
    matches!(value, Ok(None)) && !side.faction() || matches!(value, Ok(Some((0, _))))
}

impl<'a> Compose<'a> {
    pub fn new(layout: Option<&'a Layout>, operands: &'a DayOperands, view: View<'a>) -> Self {
        let mut compose = Self {
            layout,
            operands,
            country: "",
            state: State::Identity,
            complete: false,
            shown: Some(Vec::new()),
            excluded: 0,
        };
        if !operands.identity_known {
            return compose;
        }
        let complete = operands.complete && !operands.unresolved;
        let wanted_invalid =
            |wanted: &dyn Fn(&str) -> bool| operands.invalid.iter().any(|tag| wanted(tag));
        compose.state = match (view, layout) {
            (View::Country(tag), _) => {
                compose.country = tag;
                compose.complete = complete && !has(&operands.invalid, tag);
                State::Country
            }
            (_, None) => State::NoSides,
            (View::Side(None), Some(_)) => State::Absent,
            (View::Side(Some(position)), Some(layout)) => {
                let side = layout.side(position);
                compose.complete = complete
                    && !wanted_invalid(&|tag| match side.leader() {
                        Some(leader) => leader == tag,
                        None => side.members().iter().any(|member| member == tag),
                    });
                State::Side(position)
            }
            (View::Sides, Some(layout)) => {
                compose.complete = complete && !wanted_invalid(&|tag| layout.wanted(tag));
                if compose.complete {
                    let mut shown = Vec::new();
                    let mut excluded = 0;
                    for group in layout.groups() {
                        let side = Side::Group(group);
                        let (_, base) = compose.side_pair(side, None);
                        if matches!(base, Ok(None)) && !side.faction() {
                            excluded += 1;
                        } else if !hidden(side, &base) {
                            shown.push(group.position);
                        }
                    }
                    let witnessed_lone = operands
                        .witnessed
                        .iter()
                        .filter(|tag| layout.lone_position(tag).is_some())
                        .count();
                    compose.excluded = excluded + layout.lone_count() - witnessed_lone;
                    compose.shown = Some(shown);
                } else {
                    compose.shown = None;
                }
                State::Sides
            }
        };
        compose
    }

    /// The fallback's sides left out of the sides view.
    pub fn excluded(&self) -> usize {
        self.excluded
    }

    /// Whether the sides view shows every side.
    pub fn shows_all(&self) -> bool {
        matches!(self.state, State::Sides) && self.shown.is_none()
    }

    fn pair(&self, tag: &str, region: Option<&[Pair]>) -> Score {
        if !has(&self.operands.present, tag) {
            return Err(());
        }
        if let Some(at) = region.and_then(|pairs| {
            pairs
                .binary_search_by(|(held, _)| held.as_str().cmp(tag))
                .ok()
                .map(|at| pairs[at].1)
        }) {
            return Ok(Some(at));
        }
        if !self.complete {
            return Err(());
        }
        Ok(has(&self.operands.witnessed, tag).then_some((0, 0)))
    }

    fn side_pair(&self, side: Side<'a>, region: Option<&[Pair]>) -> (Option<&'a str>, Score) {
        if let Some(tag) = side.leader() {
            return (Some(tag), self.pair(tag, region));
        }
        if side.faction() {
            return (None, Err(()));
        }
        let mut best: Option<(&'a String, (i64, i64))> = None;
        let mut unknown = false;
        for tag in side.members() {
            match self.pair(tag, region) {
                Ok(Some(value))
                    if best.is_none_or(|(old, score)| {
                        value.0 > score.0 || value.0 == score.0 && tag < old
                    }) =>
                {
                    best = Some((tag, value))
                }
                Err(()) => unknown = true,
                _ => {}
            }
        }
        (
            best.map(|(tag, _)| tag.as_str()),
            if unknown {
                Err(())
            } else {
                Ok(best.map(|(_, value)| value))
            },
        )
    }

    /// Each side the sides view shows in `region` (the fallback for None),
    /// in roster order, with the tag supplying its read and the read.
    fn visit(
        &self,
        region: Option<&[Pair]>,
        mut each: impl FnMut(u32, Side<'a>, Option<&'a str>, Score),
    ) {
        let Some(layout) = self.layout else {
            return;
        };
        let mut scoring: Vec<u32> = region
            .into_iter()
            .flatten()
            .filter_map(|(tag, _)| layout.represented(tag))
            .collect();
        scoring.sort_unstable();
        scoring.dedup();
        let mut emit = |position: u32, scored: bool| {
            let side = layout.side(position);
            let (leader, value) = self.side_pair(side, if scored { region } else { None });
            if !hidden(side, &value) {
                each(position, side, leader, value);
            }
        };
        match &self.shown {
            None => {
                for position in 0..layout.len() {
                    emit(position, scoring.binary_search(&position).is_ok());
                }
            }
            Some(shown) => {
                let (mut left, mut right) = (shown.iter().peekable(), scoring.iter().peekable());
                loop {
                    match (left.peek(), right.peek()) {
                        (Some(&&shown), Some(&&scored)) if shown < scored => {
                            left.next();
                            emit(shown, false);
                        }
                        (Some(&&shown), Some(&&scored)) => {
                            if shown == scored {
                                left.next();
                            }
                            right.next();
                            emit(scored, true);
                        }
                        (Some(&&shown), None) => {
                            left.next();
                            emit(shown, false);
                        }
                        (None, Some(&&scored)) => {
                            right.next();
                            emit(scored, true);
                        }
                        (None, None) => break,
                    }
                }
            }
        }
    }

    /// The cell of `region` (its pairs; the fallback for None), each shown
    /// side by its `index`.
    pub fn cell(&self, region: Option<&[Pair]>, index: &mut dyn FnMut(u32) -> u32) -> AirCell {
        match self.state {
            State::Identity => reason("identity-not-recorded"),
            State::NoSides => reason("sides-not-recorded"),
            State::Absent => reason("side-not-present"),
            State::Country => match self.pair(self.country, region) {
                Ok(Some((friendly, enemy))) => AirCell(Some(AirValue::Viewpoint((
                    friendly.to_string(),
                    enemy.to_string(),
                )))),
                Ok(None) => reason("no-air-object"),
                Err(()) => reason("score-not-recorded"),
            },
            State::Side(position) => {
                let layout = self.layout.expect("a side view has a roster");
                let (leader, value) = self.side_pair(layout.side(position), region);
                let index = index(position);
                let mut cell = AirSideCell {
                    scores: vec![],
                    viewpoints: vec![],
                    unknown: vec![],
                    unknown_set: None,
                    no_air_object: vec![],
                    filtered: false,
                    excluded: 0,
                };
                match value {
                    Ok(Some((friendly, enemy))) => {
                        return AirCell(Some(AirValue::SideViewpoint {
                            view: (friendly.to_string(), enemy.to_string()),
                            side: index,
                            leader: leader.map(str::to_owned),
                        }));
                    }
                    Ok(None) => cell.no_air_object.push(index),
                    Err(()) => cell.unknown.push(index),
                }
                AirCell(Some(AirValue::Sides(cell)))
            }
            State::Sides => {
                let mut cell = AirSideCell {
                    scores: vec![],
                    viewpoints: vec![],
                    unknown: vec![],
                    unknown_set: None,
                    no_air_object: vec![],
                    filtered: false,
                    excluded: 0,
                };
                self.visit(region, |position, side, leader, value| {
                    let index = index(position);
                    if side.leader().is_none()
                        && let Some(leader) = leader
                    {
                        cell.viewpoints.push((index, leader.to_owned()));
                    }
                    match value {
                        Ok(Some((friendly, _))) if friendly > 0 => {
                            cell.scores.push((index, friendly.to_string()))
                        }
                        Ok(None) => cell.no_air_object.push(index),
                        Err(()) => cell.unknown.push(index),
                        _ => {}
                    }
                });
                if cell.unknown.is_empty()
                    && cell.no_air_object.is_empty()
                    && cell.viewpoints.is_empty()
                {
                    AirCell(Some(AirValue::Scores(cell.scores)))
                } else {
                    AirCell(Some(AirValue::Sides(cell)))
                }
            }
        }
    }

    /// The sides view's activity in `region`: its shown sides' friendly
    /// scores, as the ranking sums an unfiltered sides cell.
    pub fn activity(&self, region: &[Pair]) -> u128 {
        let mut total = 0;
        if matches!(self.state, State::Sides) {
            self.visit(Some(region), |_, _, _, value| {
                if let Ok(Some((friendly, _))) = value
                    && friendly > 0
                {
                    total += friendly as u128;
                }
            });
        }
        total
    }

    /// A side's friendly score in `region` for each side the sides view
    /// shows there with one, by place.
    pub fn scores(&self, region: &[Pair], mut each: impl FnMut(u32, u128)) {
        if matches!(self.state, State::Sides) {
            self.visit(Some(region), |position, _, _, value| {
                if let Ok(Some((friendly, _))) = value
                    && friendly > 0
                {
                    each(position, friendly as u128);
                }
            });
        }
    }

    /// The viewpoint's activity in `region`: its friendly and enemy scores
    /// where it reads them.
    pub fn view_activity(&self, region: &[Pair]) -> u128 {
        let value = match self.state {
            State::Country => self.pair(self.country, Some(region)),
            State::Side(position) => {
                let layout = self.layout.expect("a side view has a roster");
                self.side_pair(layout.side(position), Some(region)).1
            }
            _ => return 0,
        };
        match value {
            Ok(Some((friendly, enemy))) => friendly as u128 + enemy as u128,
            _ => 0,
        }
    }

    /// The places the sides view first shows a side at, in the order its
    /// fallback and then each region in `regions` (ascending) shows them.
    pub fn uses<'r>(&self, regions: impl IntoIterator<Item = &'r [Pair]>) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut order = Vec::new();
        if !matches!(self.state, State::Sides) {
            return order;
        }
        let mut note = |position: u32, _: Side, _: Option<&str>, _: Score| {
            if seen.insert(position) {
                order.push(position);
            }
        };
        self.visit(None, &mut note);
        for region in regions {
            self.visit(Some(region), &mut note);
        }
        order
    }
}
