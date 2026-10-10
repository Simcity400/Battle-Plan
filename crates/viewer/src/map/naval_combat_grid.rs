//! The active naval combat's map-icon task-force and convoy children.
//!
//! Walk each side's ordered forward groups, count live convoys separately and insert each live ship's
//! task force once. Same-hour rosters share Index across combats. Client country, selection and
//! rendered counter caches do not participate.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use replay::rows::{NavalCombatShipRow, TaskForceCounterInputRow, TaskForceRow};

use super::{allegiance::Allegiance, counter_inputs, perspective::Vision};

/// Groups reached only as opponents have no forward-group position and are excluded from the side
/// walk.
pub const PAIRED_ONLY_GROUP: u32 = u32::MAX;

/// Forward-group members grouped once per hour as stored NavalCombatShips indices. PAIRED_ONLY_GROUP
/// excludes opponent-only groups. HQ frames and combat grids use the same members.
pub struct ForwardMembers {
    by_combat: BTreeMap<u64, Vec<usize>>,
}

impl ForwardMembers {
    pub fn read(rows: &[NavalCombatShipRow]) -> Self {
        let mut by_combat: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            if row.member_source == 0 && row.group_index != PAIRED_ONLY_GROUP {
                by_combat.entry(row.combat).or_default().push(index);
            }
        }
        Self { by_combat }
    }

    /// Every combat's forward members, as indices into the rows this was read from.
    pub fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.by_combat.values().flatten().copied()
    }

    /// `combat`'s forward members in stored order, from the rows this was read from.
    pub fn of<'a>(
        &'a self,
        combat: u64,
        rows: &'a [NavalCombatShipRow],
    ) -> impl Iterator<Item = &'a NavalCombatShipRow> + 'a {
        self.by_combat
            .get(&combat)
            .into_iter()
            .flatten()
            .map(move |&index| &rows[index])
    }
}

/// A complete table is passed as `Some(rows)`; `None` is unavailable or
/// truncated. The caller cannot silently treat an incomplete table as empty.
pub struct Sources<'a> {
    /// The member census and its forward members, grouped once per hour.
    pub combat_ships: Option<(&'a [NavalCombatShipRow], &'a ForwardMembers)>,
    pub ships: Option<ShipCensus<'a>>,
    pub task_forces: Option<&'a [TaskForceRow]>,
    pub counter_inputs: Option<&'a [TaskForceCounterInputRow]>,
    pub aggression: Option<&'a [i64]>,
}

/// The hour's `Ships` census as indexed by `counters::NavalContext`: each ship's task force, each
/// task force's complete roster length and summed strength, and the table's row count. The grids
/// reuse this index.
pub struct ShipCensus<'a> {
    pub task_forces: Cow<'a, BTreeMap<u64, u64>>,
    pub counts: Cow<'a, BTreeMap<u64, (usize, i64)>>,
    pub rows: usize,
}

#[cfg(test)]
impl ShipCensus<'_> {
    /// The census indexed from its rows, as `NavalContext::read` indexes it.
    pub fn from_rows(ships: &[replay::rows::ShipRow]) -> Self {
        let mut counts = BTreeMap::<u64, (usize, i64)>::new();
        for ship in ships {
            let entry = counts.entry(ship.task_force).or_default();
            entry.0 += 1;
            entry.1 = entry.1.wrapping_add(ship.strength);
        }
        Self {
            task_forces: Cow::Owned(
                ships
                    .iter()
                    .map(|ship| (ship.id, ship.task_force))
                    .collect(),
            ),
            counts: Cow::Owned(counts),
            rows: ships.len(),
        }
    }
}

/// One hour's rosters, indexed once and shared by every combat's grid. A census that cannot
/// be used keeps its reason, which each grid that needs it reports.
pub struct Index<'a> {
    /// Each combat's forward-group members in `(group, member)` order.
    members: Result<BTreeMap<u64, Vec<&'a NavalCombatShipRow>>, &'static str>,
    /// The ship census, with complete roster lengths for every task force containing a ship.
    ships: Result<ShipCensus<'a>, &'static str>,
    forces: Result<BTreeMap<u64, &'a TaskForceRow>, &'static str>,
    inputs: Option<BTreeMap<u64, &'a TaskForceCounterInputRow>>,
    aggression: Option<&'a [i64]>,
}

impl<'a> Index<'a> {
    pub fn new(source: Sources<'a>) -> Self {
        let members = source
            .combat_ships
            .ok_or("naval combat ship census is incomplete")
            .map(|(rows, forward)| {
                // A copy per combat in `(group, member)` order; the HQ frame walks them in
                // stored order.
                forward
                    .by_combat
                    .iter()
                    .map(|(&combat, indices)| {
                        let mut members: Vec<&NavalCombatShipRow> =
                            indices.iter().map(|&index| &rows[index]).collect();
                        members.sort_by_key(|row| (row.group_index, row.member_index));
                        (combat, members)
                    })
                    .collect()
            });
        let ships = source
            .ships
            .ok_or("ship census is incomplete")
            .and_then(|ships| {
                (ships.task_forces.len() == ships.rows)
                    .then_some(ships)
                    .ok_or("ship census repeats a handle")
            });
        let forces = source
            .task_forces
            .ok_or("task-force census is incomplete")
            .and_then(|rows| {
                let map: BTreeMap<_, _> = rows.iter().map(|force| (force.id, force)).collect();
                (map.len() == rows.len())
                    .then_some(map)
                    .ok_or("task-force census repeats a handle")
            });
        Self {
            members,
            ships,
            forces,
            inputs: source
                .counter_inputs
                .map(|rows| rows.iter().map(|input| (input.task_force, input)).collect()),
            aggression: source.aggression,
        }
    }
}

/// One task force admitted by the combat member walk. `ships` is its complete roster length,
/// independent of how many members are currently live combat ships.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridTaskForce<'a> {
    pub force: &'a TaskForceRow,
    pub country: u32,
    pub ships: usize,
    pub visibility: u16,
}

/// One naval mission counter item after contiguous-run grouping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridGroup<'a> {
    pub task_forces: Vec<GridTaskForce<'a>>,
}

/// The children attached to one side of the active combat icon.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideGrid<'a> {
    pub groups: Vec<GridGroup<'a>>,
    pub convoys: usize,
}

#[derive(Clone, Debug)]
struct Candidate<'a> {
    force: GridTaskForce<'a>,
    allegiance: Allegiance,
    /// Own task forces with `engagement_refused` true sort before those with it false.
    refused: Option<bool>,
}

/// Derive both side grids of `combat` from the hour's indexed rosters and the viewer's
/// independently chosen country/fog. `owner` canonicalises a raw country ID. Missing joins,
/// incomplete needed tables and a foreign task force whose intel the hour cannot establish
/// refuse the combat's grids.
pub fn derive<'a>(
    index: &Index<'a>,
    combat: u64,
    vision: &Vision,
    owner: &impl Fn(u32) -> Option<u32>,
    allegiances: &BTreeMap<u32, Allegiance>,
) -> Result<[SideGrid<'a>; 2], &'static str> {
    let members = index.members.as_ref().map_err(|reason| *reason)?;
    let ships = index.ships.as_ref().map_err(|reason| *reason)?;
    let ship_force = &ships.task_forces;
    let forces = index.forces.as_ref().map_err(|reason| *reason)?;
    let mut sides: [Vec<Candidate<'a>>; 2] = [Vec::new(), Vec::new()];
    let mut result = [SideGrid::default(), SideGrid::default()];
    let mut seen: [BTreeSet<u64>; 2] = [BTreeSet::new(), BTreeSet::new()];
    let mut positions = BTreeSet::new();
    for row in members.get(&combat).into_iter().flatten() {
        let side = usize::from(row.side);
        if side >= 2 {
            return Err("naval combat member has an invalid side");
        }
        if !positions.insert((row.side, row.group_index, row.member_index)) {
            return Err("naval combat primary member position repeats");
        }
        if row.live_ship != 0 {
            if row.ship == 0 {
                return Err("live naval combat ship has no handle");
            }
            let force_id = ship_force
                .get(&row.ship)
                .copied()
                .ok_or("live naval combat ship is absent from Ships")?;
            if !seen[side].insert(force_id) {
                continue;
            }
            let force: &'a TaskForceRow = forces
                .get(&force_id)
                .copied()
                .ok_or("live naval combat ship's task force is absent")?;
            let country = owner(force.roster_owner)
                .ok_or("naval combat task-force country alias is unresolved")?;
            // The caller's map is built by `allegiance::counter_table`, which
            // refuses incomplete relation tables. A missing country entry is
            // therefore a corrupt join, never a default `Other` country.
            let allegiance = allegiances
                .get(&country)
                .copied()
                .ok_or("naval combat task-force allegiance is unresolved")?;
            // Unit intel: an hour without the unit intel census cannot give a foreign
            // task force's byte, and a truncated census cannot give an unlisted one's. The byte
            // orders foreign forces and words a group's counts, so either refuses the grids.
            if country != vision.ours() && vision.undecided().is_some() {
                return Err("naval task-force visibility inputs are incomplete");
            }
            let visibility = vision.navy_byte_known(force_id, country).ok_or(
                "a foreign task force's intel is unknown (the unit intel census is truncated)",
            )?;
            let refused = index
                .inputs
                .as_ref()
                .and_then(|rows| rows.get(&force_id))
                .and_then(|row| {
                    index.aggression.and_then(|aggression| {
                        counter_inputs::engagement_refused(
                            row.running_away,
                            row.engagement_rule,
                            force.mission,
                            aggression,
                        )
                    })
                });
            sides[side].push(Candidate {
                force: GridTaskForce {
                    force,
                    country,
                    ships: ships.counts.get(&force_id).map_or(0, |&(count, _)| count),
                    visibility,
                },
                allegiance,
                refused,
            });
        } else if row.live_convoy != 0 {
            result[side].convoys += 1;
        }
    }
    for side in 0..2 {
        let own = sides[side]
            .iter()
            .filter(|candidate| candidate.force.country == vision.ours())
            .count();
        if own > 1
            && sides[side].iter().any(|candidate| {
                candidate.force.country == vision.ours() && candidate.refused.is_none()
            })
        {
            return Err("own naval sort needs complete engagement operands");
        }
        // Stable-sort the side using wrapping fleet_difference comparisons, retaining the order even
        // with a nontransitive comparator.
        let ours = vision.ours();
        let mut order: Vec<usize> = (0..sides[side].len()).collect();
        crate::map::msvc_stable_sort::sort(&mut order, |&a, &b| {
            compare(&sides[side][a], &sides[side][b], ours) < 0
        });
        let mut taken: Vec<Option<Candidate<'a>>> = sides[side].drain(..).map(Some).collect();
        let sorted = order.into_iter().filter_map(|index| taken[index].take());
        // Do not sort again by allegiance. It starts a new run
        // only when `(run_first, candidate)` says the first has
        // higher allegiance priority. A lower-priority first can therefore
        // absorb a later higher-priority candidate. Preserve that asymmetry.
        let mut first: Option<Allegiance> = None;
        for candidate in sorted {
            if first.is_none_or(|first| {
                allegiance_priority(first) < allegiance_priority(candidate.allegiance)
            }) {
                result[side].groups.push(GridGroup::default());
                first = Some(candidate.allegiance);
            }
            let group = result[side]
                .groups
                .last_mut()
                .expect("the first candidate starts a group");
            group.task_forces.push(candidate.force);
        }
    }
    Ok(result)
}

/// Signed priority comparison with viewer selection false for every member. Negative results mean
/// less.
fn compare(a: &Candidate<'_>, b: &Candidate<'_>, ours: u32) -> i32 {
    let first = |a_first: bool| if a_first { -1 } else { 1 };
    let a_own = a.force.country == ours;
    let b_own = b.force.country == ours;
    if a_own != b_own {
        return first(a_own);
    }
    if !a_own {
        let (a_full, b_full) = (a.force.visibility >= 250, b.force.visibility >= 250);
        return if a_full == b_full { 0 } else { first(a_full) };
    }
    let a_refused = a.refused.unwrap_or(false);
    let b_refused = b.refused.unwrap_or(false);
    if a_refused != b_refused {
        return first(a_refused);
    }
    let a_reserve = a.force.force.mission == 8;
    let b_reserve = b.force.force.mission == 8;
    if a_reserve != b_reserve {
        return first(b_reserve);
    }
    if !a_reserve && a.force.force.fleet != b.force.force.fleet {
        return fleet_difference(a.force.force.fleet, b.force.force.fleet);
    }
    0
}

/// Subtract low handle words in i32, or high words when low words agree. Both subtractions wrap.
pub(crate) fn fleet_difference(a: u64, b: u64) -> i32 {
    let ((a_low, a_high), (b_low, b_high)) = (signed_handle(a), signed_handle(b));
    if a_low == b_low {
        a_high.wrapping_sub(b_high)
    } else {
        a_low.wrapping_sub(b_low)
    }
}

/// A handle's signed halves, low word first.
fn signed_handle(handle: u64) -> (i32, i32) {
    (handle as u32 as i32, (handle >> 32) as u32 as i32)
}

/// Treat every task force as unselected. Compare raw/canonical owner equivalence first, then own,
/// shared and active-war allegiance. `allegiance::counter_table` uses the same complete relations.
fn allegiance_priority(side: Allegiance) -> u8 {
    match side {
        Allegiance::Ours => 0,
        Allegiance::Allied => 1,
        Allegiance::Enemy => 2,
        Allegiance::Other => 3,
    }
}

#[cfg(test)]
mod tests;
