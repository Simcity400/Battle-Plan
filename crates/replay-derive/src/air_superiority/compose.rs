//! The composition of an hour's cells from its sides and its countries' scores: which side shows
//! which country's read in a region, and what is unknown, absent or left out. A cell carries no
//! number: each part names the country whose recorded row supplies it, a live cell of its own.

use super::assignment::AssignedSide;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

/// A country's row in a region after the duplicate owner rule: its tag, its raw country id (the
/// row's key) and its friendly and enemy scores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub tag: String,
    pub raw: u32,
    pub friendly: i64,
    pub enemy: i64,
}

/// A read of a country's or side's scores: unknown, no air object, or scores with the row
/// supplying them (none for a known zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Score<'a> {
    Unknown,
    NoAir,
    Known(Option<&'a Pair>),
}

impl Score<'_> {
    fn friendly(self) -> i64 {
        match self {
            Self::Known(Some(pair)) => pair.friendly,
            _ => 0,
        }
    }
}

/// An hour's score operands.
#[derive(Clone, Debug, Default)]
pub struct Operands {
    pub identity_known: bool,
    /// The tags held at the hour.
    pub present: Arc<BTreeSet<String>>,
    /// The present tags with a row anywhere: an air object.
    pub witnessed: HashSet<String>,
    /// The present tags with a negative score: not read.
    pub invalid: HashSet<String>,
    pub complete: bool,
    /// A row whose owner no present tag holds.
    pub unresolved: bool,
}

/// A cell's structure: why it has none, its sides' parts, or one viewpoint's read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Reason(&'static str),
    Sides(SidesCell),
    /// A viewpoint's friendly and enemy scores; `side` its place for a side's viewpoint, `leader`
    /// the country read, `row` its raw id where a row supplies them (none: a known zero).
    View { side: Option<u32>, leader: Option<String>, row: Option<u32> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidesCell {
    /// Each scoring side's place, its country and that country's raw id.
    pub parts: Vec<(u32, String, u32)>,
    /// A leaderless side's place and the country whose read it shows.
    pub viewpoints: Vec<(u32, String)>,
    pub unknown: Vec<u32>,
    pub no_air_object: Vec<u32>,
    pub filtered: bool,
    pub excluded: usize,
}

/// What a cell is composed for.
#[derive(Clone, Copy)]
pub enum View<'a> {
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

/// The sides of an hour by place, with each group member's place.
pub struct Roster<'a> {
    sides: &'a [AssignedSide],
    member_of: HashMap<&'a str, u32>,
}

impl<'a> Roster<'a> {
    pub fn new(sides: &'a [AssignedSide]) -> Self {
        let member_of = sides.iter().enumerate().filter(|(_, side)| !side.lone()).flat_map(|(at, side)| side.members.iter().map(move |tag| (tag.as_str(), at as u32))).collect();
        Self { sides, member_of }
    }

    fn side(&self, position: u32) -> &'a AssignedSide {
        &self.sides[position as usize]
    }

    fn lone_position(&self, tag: &str) -> Option<u32> {
        self.sides.iter().position(|side| side.lone() && side.members[0] == tag).map(|at| at as u32)
    }

    /// The place of the side whose read takes `tag`'s scores: its lone country, its group's
    /// leader, or a member of a group without leader or faction.
    fn represented(&self, tag: &str) -> Option<u32> {
        match self.member_of.get(tag) {
            Some(&position) => {
                let side = self.side(position);
                match &side.leader {
                    Some(leader) => (leader == tag).then_some(position),
                    None => side.factions.is_empty().then_some(position),
                }
            }
            None => self.lone_position(tag),
        }
    }

    /// Whether `tag`'s scores decide whether the sides view is complete: a side's leader, or any
    /// member of a side without one.
    fn wanted(&self, tag: &str, present: &BTreeSet<String>) -> bool {
        match self.member_of.get(tag) {
            Some(&position) => self.side(position).leader.as_deref().is_none_or(|leader| leader == tag),
            None => present.contains(tag),
        }
    }
}

/// An hour's composition for one view.
pub struct Compose<'a> {
    roster: Option<&'a Roster<'a>>,
    operands: &'a Operands,
    country: &'a str,
    state: State,
    complete: bool,
    /// The places the fallback shows, None for every place.
    shown: Option<Vec<u32>>,
    excluded: usize,
}

/// Owners without a strategic-air object do not participate, and a known zero adds no part.
fn hidden(side: &AssignedSide, value: Score<'_>) -> bool {
    matches!(value, Score::NoAir) && side.factions.is_empty() || matches!(value, Score::Known(_)) && value.friendly() == 0
}

fn find<'p>(pairs: Option<&'p [Pair]>, tag: &str) -> Option<&'p Pair> {
    let pairs = pairs?;
    pairs.binary_search_by(|held| held.tag.as_str().cmp(tag)).ok().map(|at| &pairs[at])
}

impl<'a> Compose<'a> {
    pub fn new(roster: Option<&'a Roster<'a>>, operands: &'a Operands, view: View<'a>) -> Self {
        let mut compose = Self { roster, operands, country: "", state: State::Identity, complete: false, shown: Some(Vec::new()), excluded: 0 };
        if !operands.identity_known {
            return compose;
        }
        let complete = operands.complete && !operands.unresolved;
        let wanted_invalid = |wanted: &dyn Fn(&str) -> bool| operands.invalid.iter().any(|tag| wanted(tag));
        compose.state = match (view, roster) {
            (View::Country(tag), _) => {
                compose.country = tag;
                compose.complete = complete && !operands.invalid.contains(tag);
                State::Country
            }
            (_, None) => State::NoSides,
            (View::Side(None), Some(_)) => State::Absent,
            (View::Side(Some(position)), Some(roster)) => {
                let side = roster.side(position);
                compose.complete = complete
                    && !wanted_invalid(&|tag| match &side.leader {
                        Some(leader) => leader == tag,
                        None => side.members.iter().any(|member| member == tag),
                    });
                State::Side(position)
            }
            (View::Sides, Some(roster)) => {
                compose.complete = complete && !wanted_invalid(&|tag| roster.wanted(tag, &operands.present));
                if compose.complete {
                    let mut shown = Vec::new();
                    let mut excluded = 0;
                    for (position, side) in roster.sides.iter().enumerate().filter(|(_, side)| !side.lone()) {
                        let (_, base) = compose.side_pair(side, None);
                        if matches!(base, Score::NoAir) && side.factions.is_empty() {
                            excluded += 1;
                        } else if !hidden(side, base) {
                            shown.push(position as u32);
                        }
                    }
                    let lone = roster.sides.iter().filter(|side| side.lone()).count();
                    let witnessed_lone = operands.witnessed.iter().filter(|tag| roster.lone_position(tag).is_some()).count();
                    compose.excluded = excluded + lone - witnessed_lone;
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

    fn pair<'p>(&self, tag: &str, region: Option<&'p [Pair]>) -> Score<'p> {
        if !self.operands.present.contains(tag) {
            return Score::Unknown;
        }
        if let Some(pair) = find(region, tag) {
            return Score::Known(Some(pair));
        }
        if !self.complete {
            return Score::Unknown;
        }
        if self.operands.witnessed.contains(tag) { Score::Known(None) } else { Score::NoAir }
    }

    fn side_pair<'s, 'p>(&self, side: &'s AssignedSide, region: Option<&'p [Pair]>) -> (Option<&'s str>, Score<'p>) {
        if let Some(tag) = &side.leader {
            return (Some(tag), self.pair(tag, region));
        }
        if !side.factions.is_empty() {
            return (None, Score::Unknown);
        }
        let mut best: Option<(&'s String, Score<'p>)> = None;
        let mut unknown = false;
        for tag in &side.members {
            match self.pair(tag, region) {
                value @ Score::Known(_) if best.is_none_or(|(old, score)| value.friendly() > score.friendly() || value.friendly() == score.friendly() && tag < old) => {
                    best = Some((tag, value))
                }
                Score::Unknown => unknown = true,
                _ => {}
            }
        }
        (best.map(|(tag, _)| tag.as_str()), if unknown { Score::Unknown } else { best.map_or(Score::NoAir, |(_, value)| value) })
    }

    /// Each side the sides view shows in `region` (the fallback for None), in roster order, with
    /// the tag supplying its read and the read.
    fn visit<'p>(&self, region: Option<&'p [Pair]>, mut each: impl FnMut(u32, &AssignedSide, Option<&str>, Score<'p>)) {
        let Some(roster) = self.roster else { return };
        let mut scoring: Vec<u32> = region.into_iter().flatten().filter_map(|pair| roster.represented(&pair.tag)).collect();
        scoring.sort_unstable();
        scoring.dedup();
        let mut emit = |position: u32, scored: bool| {
            let side = roster.side(position);
            let (leader, value) = self.side_pair(side, if scored { region } else { None });
            if !hidden(side, value) {
                each(position, side, leader, value);
            }
        };
        match &self.shown {
            None => {
                for position in 0..roster.sides.len() as u32 {
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

    /// The cell of `region` (its pairs, by tag; the fallback for None).
    pub fn cell(&self, region: Option<&[Pair]>) -> Cell {
        match self.state {
            State::Identity => Cell::Reason("identity-not-recorded"),
            State::NoSides => Cell::Reason("sides-not-recorded"),
            State::Absent => Cell::Reason("side-not-present"),
            State::Country => match self.pair(self.country, region) {
                Score::Known(pair) => Cell::View { side: None, leader: Some(self.country.to_owned()), row: pair.map(|pair| pair.raw) },
                Score::NoAir => Cell::Reason("no-air-object"),
                Score::Unknown => Cell::Reason("score-not-recorded"),
            },
            State::Side(position) => {
                let roster = self.roster.expect("a side view has a roster");
                let (leader, value) = self.side_pair(roster.side(position), region);
                let mut cell = SidesCell::default();
                match value {
                    Score::Known(pair) => return Cell::View { side: Some(position), leader: leader.map(str::to_owned), row: pair.map(|pair| pair.raw) },
                    Score::NoAir => cell.no_air_object.push(position),
                    Score::Unknown => cell.unknown.push(position),
                }
                Cell::Sides(cell)
            }
            State::Sides => {
                let mut cell = SidesCell::default();
                self.visit(region, |position, side, leader, value| {
                    if side.leader.is_none()
                        && let Some(leader) = leader
                    {
                        cell.viewpoints.push((position, leader.to_owned()));
                    }
                    match value {
                        Score::Known(Some(pair)) if pair.friendly > 0 => cell.parts.push((position, pair.tag.clone(), pair.raw)),
                        Score::NoAir => cell.no_air_object.push(position),
                        Score::Unknown => cell.unknown.push(position),
                        _ => {}
                    }
                });
                Cell::Sides(cell)
            }
        }
    }

    /// The sides view's activity in `region`: its shown sides' friendly scores.
    pub fn activity(&self, region: &[Pair]) -> u128 {
        let mut total = 0;
        if matches!(self.state, State::Sides) {
            self.visit(Some(region), |_, _, _, value| {
                if value.friendly() > 0 {
                    total += value.friendly() as u128;
                }
            });
        }
        total
    }

    /// A side's friendly score in `region` for each side the sides view shows there with one, by
    /// place.
    pub fn scores(&self, region: &[Pair], mut each: impl FnMut(u32, u128)) {
        if matches!(self.state, State::Sides) {
            self.visit(Some(region), |position, _, _, value| {
                if value.friendly() > 0 {
                    each(position, value.friendly() as u128);
                }
            });
        }
    }

    /// The viewpoint's activity in `region`: its friendly and enemy scores where it reads them.
    pub fn view_activity(&self, region: &[Pair]) -> u128 {
        let value = match self.state {
            State::Country => self.pair(self.country, Some(region)),
            State::Side(position) => {
                let roster = self.roster.expect("a side view has a roster");
                self.side_pair(roster.side(position), Some(region)).1
            }
            _ => return 0,
        };
        match value {
            Score::Known(Some(pair)) => pair.friendly as u128 + pair.enemy as u128,
            _ => 0,
        }
    }

    /// The places the sides view shows a side at, in the order its fallback and then each region
    /// in `regions` first shows them.
    pub fn uses<'r>(&self, regions: impl IntoIterator<Item = &'r [Pair]>) -> Vec<u32> {
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        if !matches!(self.state, State::Sides) {
            return order;
        }
        self.visit(None, |position, _, _, _| {
            if seen.insert(position) {
                order.push(position);
            }
        });
        for region in regions {
            self.visit(Some(region), |position, _, _, _| {
                if seen.insert(position) {
                    order.push(position);
                }
            });
        }
        order
    }
}
