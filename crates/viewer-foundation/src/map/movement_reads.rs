//! Army reach for one recorded hour. Join facts requested by
//! `installed_content_unit_path::unit_path` through `CountryLinks` to canonical slots.
//! An incomplete required table gives a named error and leaves reach unknown.

use crate::map::allegiance::NativeRelations;
use crate::map::allegiance::ongoing;
use std::collections::HashSet;
use std::sync::MutexGuard;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

use installed_content_core::parser::Item;
use installed_content_unit_path::{
    unit_path, unit_path::Army, unit_path::ArmyCombat, unit_path::CounterClass,
    unit_path::CounterRequest, unit_path::Country, unit_path::Fact, unit_path::MoveWorld,
    unit_path::Mover, unit_path::Occupant, unit_path::ReachKey, unit_path::Reachable,
    unit_path::Search, unit_path::StraitFleet,
};
use replay_data::{
    rows::DivisionMovementRow, rows::DivisionRetreatBanRow, rows::DivisionRow,
    rows::DivisionStatusRow, rows::ProvinceRow, rows::RelationLinkRow, rows::TableSchema,
    rows::TaskForceRow, rows::tables,
};

use dashboard_text::triggers::{Eval, Scope, Scopes, World};

/// Immutable movement reads shared by raid reach and counter hover queries.
#[derive(Clone, Copy)]
pub struct ReadOperands<'a> {
    pub moves: &'a Result<installed_content_unit_path::unit_path::MoveGraph, String>,
    pub trigger_content: &'a crate::trigger_content::TriggerContent,
    pub regions: &'a BTreeMap<u32, u32>,
    pub geometry: &'a installed_content_core::province_geometry::ProvinceGeometry,
    pub combat_minimum_time: i32,
    pub avoid_enemy: bool,
    pub day_night: &'a Result<replay_data::day_night::Defines, String>,
    pub naval_combat_min_duration: &'a Result<i32, String>,
}

impl ReadOperands<'_> {
    /// Selected hover reuses the already parsed graph and region map.
    pub fn selected_neighbour(
        &self,
        from: u32,
        to: u32,
    ) -> installed_content_unit_path::unit_path::Fact<
        Option<installed_content_unit_path::unit_path::SelectedNeighbour>,
    > {
        self.moves
            .as_ref()
            .map_err(Clone::clone)?
            .selected_neighbour(from, to)
    }

    pub fn selected_region(&self, province: u32) -> Result<u32, String> {
        self.regions
            .get(&province)
            .copied()
            .ok_or_else(|| format!("province {province} has no recorded-content strategic region"))
    }

    /// The day/night clock written into `province`'s record at date `hour`,
    /// from its day/night geometry constants.
    pub fn updated_clock(
        &self,
        province: u32,
        hour: i32,
    ) -> Result<replay_data::day_night::Clock, &'static str> {
        let defines = self
            .day_night
            .as_ref()
            .map_err(|_| "day/night defines unread from the recorded content")?;
        let centroid = self
            .geometry
            .centroids
            .get(&province)
            .ok_or("province has no recorded-content centroid for its day/night clock")?;
        let operands = replay_data::day_night::HourOperands::new(hour, defines)?;
        Ok(replay_data::day_night::ProvinceClock::new(
            *centroid,
            [self.geometry.width, self.geometry.height],
            defines,
        )
        .update(&operands))
    }
}

type Index<T> = OnceLock<Result<T, String>>;
#[path = "movement_reads/selected_common.rs"]
mod selected_common;
pub use selected_common::{CommonCells, CommonText};
#[path = "movement_reads/order_cursor.rs"]
mod order_cursor;
pub use installed_content_unit_path::unit_path::order_cursor::OrderCursor;
pub use order_cursor::{order_cursor, rule_relation};
pub use installed_content_unit_path::unit_path::RuleReason;

/// One country's `CountryWars` row in canonical slots.
struct Wars {
    can_occupy_non_war: bool,
    /// The read enemy vector, when the collector read it.
    enemies: Option<Vec<u32>>,
}
/// Each division's land combats, with where their attacking units stand or why that is
/// unknown, and the provinces holding one.
type Combats = (
    BTreeMap<u64, Vec<(ArmyCombat, Arc<Fact<Vec<u32>>>)>>,
    BTreeSet<u32>,
);

fn get<T>(cell: &Index<T>, build: impl FnOnce() -> Result<T, String>) -> Result<&T, String> {
    cell.get_or_init(build).as_ref().map_err(Clone::clone)
}

/// One hour's movement indexes and reach answers, built on first use.
#[derive(Default)]
/// Rows are held as their positions in the hour's tables.
pub struct Cells {
    native_relations: Index<NativeRelations>,
    divisions: Index<BTreeMap<u64, usize>>,
    movement: Index<BTreeMap<u64, usize>>,
    armies: Index<BTreeMap<u32, Vec<Occupant>>>,
    /// Each division's land combats, and the provinces holding one.
    combats: Index<Combats>,
    naval_bases: Index<BTreeMap<u32, bool>>,
    demilitarized: Index<BTreeSet<u32>>,
    subjects: Index<BTreeSet<(u32, u32)>>,
    task_forces: Index<BTreeMap<u64, usize>>,
    /// Each task force's ships and whether every one has formation flag 7.
    fleets: Index<BTreeMap<u64, (usize, bool)>>,
    occupants: Index<BTreeMap<u32, usize>>,
    intel: Index<BTreeMap<(u64, u32), i8>>,
    /// Each canonical slot's raw country references, descending.
    slot_raws: Index<BTreeMap<u32, Vec<u32>>>,
    /// The target slots of `RelationLinks` rows whose country links to no
    /// slot, with one such country.
    unlinked_links: Index<BTreeMap<u32, u32>>,
    /// `CountryWars` by country slot.
    country_wars: Index<BTreeMap<u32, Wars>>,
    border_conflicts: Index<BTreeMap<u32, Vec<u64>>>,
    war_sets: Index<BTreeMap<u32, BTreeSet<u32>>>,
    region_access: Index<BTreeMap<(u32, u32), u8>>,
    move_flags: Index<BTreeMap<u64, u8>>,
    /// Provinces per controller-area label (`Provinces.controller_area`).
    area_members: Index<BTreeMap<u32, i32>>,
    reach: Mutex<HashMap<(u64, u32), Eval>>,
    /// One search per distinct army key, run as far as its sources need.
    /// Threads evaluating parts of one batch share them: each search has
    /// its own lock, so different armies' searches run side by side, and a
    /// search answers every source alike however far it was run before.
    searches: Mutex<HashMap<ReachKey, Arc<Mutex<Option<Search>>>>>,
    /// The hour's reachable components, by whether the army exfiltrates.
    reachable: [OnceLock<Reachable>; 2],
}

/// Steps [`Moves::run_ahead`] took, for the tests.
#[cfg(any(test, feature = "test-support"))]
pub static RAN_AHEAD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Cells {
    fn reach(&self) -> MutexGuard<'_, HashMap<(u64, u32), Eval>> {
        self.reach
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The hour's facts as `unit_path` asks for them: the hour's [`World`] and
/// its [`Cells`].
pub struct Moves<'w, 'a> {
    world: &'w World<'a>,
    assets: ReadOperands<'w>,
    cells: &'w Cells,
    counter: Option<&'w dyn CounterFacts>,
    common: Option<&'w CommonCells<'a>>,
}

/// Only reached fresh-counter class facts; the existing movement world owns
/// shared diplomacy, content adjacency and visibility predicates.
pub trait CounterFacts: Sync {
    fn rail_connected(&self, from: u32, to: u32) -> Fact<bool>;
    fn rail_orders_group(&self, handle: u64) -> Fact<u64>;
    fn naval_access(&self, moves: &Moves<'_, '_>, handle: u64, to: u32) -> Fact<bool>;
    fn all_submarine(&self, handle: u64) -> Fact<bool>;
    fn hostile_allows(&self, moves: &Moves<'_, '_>, handle: u64, to: u32) -> Fact<bool>;
}

impl<'w, 'a> Moves<'w, 'a> {
    pub fn new(world: &'w World<'a>, assets: ReadOperands<'w>, cells: &'w Cells) -> Self {
        Self {
            world,
            assets,
            cells,
            counter: None,
            common: None,
        }
    }

    pub fn for_counter(
        world: &'w World<'a>,
        assets: ReadOperands<'w>,
        cells: &'w Cells,
        counter: &'w dyn CounterFacts,
    ) -> Self {
        Self {
            world,
            assets,
            cells,
            counter: Some(counter),
            common: None,
        }
    }

    /// Shared rule verdict, selecting the submarine or navy bit.
    pub fn naval_rule(
        &self,
        rule: &installed_content_core::adjacency_rules::AdjacencyRule,
        raw_owner: u32,
        submarine: bool,
    ) -> Fact<bool> {
        if let Some(reason) = &rule.unsupported {
            return Err(format!("adjacency rule {}: {reason}", rule.name));
        }
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let mover = Mover { graph, world: self };
        Ok(rule.permits(
            mover.verdict(rule, self.slot(raw_owner)?)?,
            if submarine {
                installed_content_core::adjacency_rules::SUBMARINE
            } else {
                installed_content_core::adjacency_rules::NAVY
            },
        ))
    }

    pub fn invasion_seas(&self, source: u32, target: u32) -> Fact<Option<(u32, u32)>> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let mover = Mover { graph, world: self };
        Ok(mover.invasion_sea(source)?.zip(mover.invasion_sea(target)?))
    }

    /// Fresh counter cache-miss request. It does not demand the raid's
    /// retreat, strength, organisation, combat roster or commander bans.
    pub fn counter_route(&self, division: u64, to: u32, request: CounterRequest) -> Fact<Vec<u32>> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let row = self
            .division(division)?
            .ok_or_else(|| format!("counter unit {division:#x} is not a recorded army"))?;
        let movement = self.movement(division)?;
        let army = Army {
            location: row.province,
            owner: self.slot(row.logical_owner)?,
            navy_country: self.slot(if row.expeditionary_owner as i32 > 0 {
                row.logical_owner
            } else {
                row.roster_owner
            })?,
            orders_group: row.orders_group,
            naval_transfer: movement.naval_transfer != 0,
            exiled: self.move_flags(division)? & DivisionStatusRow::MOVE_EXILED != 0,
            ..Army::default()
        };
        Mover { graph, world: self }.counter_route(&army, row.province, to, request)
    }

    pub fn railway_counter_route(
        &self,
        row: &replay_data::rows::RailwayGunCounterInputRow,
        to: u32,
        mut request: CounterRequest,
    ) -> Fact<Vec<u32>> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        request.class = CounterClass::Railway(row.gun);
        let unit = Army {
            location: row.province,
            owner: self.slot(row.logical_owner)?,
            naval_transfer: row.naval_transfer != 0,
            exiled: row.exiled != 0,
            ..Default::default()
        };
        Mover { graph, world: self }.counter_route(&unit, row.province, to, request)
    }

    /// The invasion's forced-transfer request uses only these army operands;
    /// it does not run the raid gate, combat admission or region-avoid filter.
    /// Endpoints are already resolved by invasion_seas. `recorded_owner` is
    /// the order's InvasionPreparations `unit_owner`, whose cost operands
    /// the search reads, so it must be this unit's owner.
    pub fn invasion_route(
        &self,
        division: u64,
        recorded_owner: Option<u32>,
        country: u32,
        from: u32,
        to: u32,
    ) -> Fact<Vec<u32>> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let mover = Mover { graph, world: self };
        // Require a unit even when both ports open onto one sea.
        let row = self
            .division(division)?
            .ok_or_else(|| format!("invasion unit {division:#x} is not a recorded army"))?;
        if from == to {
            return Ok(vec![from]);
        }
        match recorded_owner {
            Some(owner) if owner == row.logical_owner => {}
            Some(_) => {
                return Err(format!(
                    "invasion unit {division:#x} owner differs from its recorded path-cost owner"
                ));
            }
            None => return Err("invasion order has no recorded path-cost operands".into()),
        }
        let army = Army {
            location: row.province,
            owner: self.slot(row.logical_owner)?,
            orders_group: row.orders_group,
            exiled: self.move_flags(division)? & DivisionStatusRow::MOVE_EXILED != 0,
            ..Army::default()
        };
        let stop_at_owner = if (country as i32) > 0 {
            Some(self.slot(country)?)
        } else {
            None
        };
        mover.invasion_route(&army, stop_at_owner, from, to)
    }

    fn table<T: TableSchema>(&self) -> Result<&'a [T::Row], String> {
        self.world
            .frame()
            .complete::<T>()
            .map_err(|error| error.to_string())
    }

    fn slot(&self, raw: u32) -> Fact<Country> {
        if raw == 0 {
            return Ok(0);
        }
        self.world
            .slot(raw)
            .ok_or_else(|| format!("country reference {raw} has no link"))
    }

    fn division(&self, id: u64) -> Fact<Option<&'a DivisionRow>> {
        let rows = self.table::<tables::Divisions>()?;
        let index = get(&self.cells.divisions, || {
            Ok(rows
                .iter()
                .enumerate()
                .map(|(at, row)| (row.id, at))
                .collect())
        })?;
        Ok(index.get(&id).map(|&at| &rows[at]))
    }

    fn movement(&self, division: u64) -> Fact<&'a DivisionMovementRow> {
        let rows = self.table::<tables::DivisionMovement>()?;
        let index = get(&self.cells.movement, || {
            Ok(rows
                .iter()
                .enumerate()
                .map(|(at, row)| (row.division, at))
                .collect())
        })?;
        index
            .get(&division)
            .map(|&at| &rows[at])
            .ok_or_else(|| format!("division {division:#x} has no movement row"))
    }

    fn combats(&self) -> Result<&Combats, String> {
        get(&self.cells.combats, || {
            let minimum = self.assets.combat_minimum_time;
            let mut by_division: BTreeMap<u64, Vec<(ArmyCombat, Arc<Fact<Vec<u32>>>)>> =
                BTreeMap::new();
            let mut provinces = BTreeSet::new();
            for row in self.table::<tables::LandCombats>()? {
                provinces.insert(row.province);
                // Only its defenders' movement reads these, so an unknown one stays theirs.
                let attackers: Arc<Fact<Vec<u32>>> = Arc::new(
                    self.attacker_locations(row)
                        .and_then(|locations| locations.into_iter().collect()),
                );
                for (side, attacking) in [(&row.attackers, true), (&row.defenders, false)] {
                    for &division in side {
                        let combat = ArmyCombat {
                            province: row.province,
                            land: true,
                            attacking,
                            defending: !attacking,
                            attacker_locations: Vec::new(),
                            minimum_duration: minimum <= row.duration,
                        };
                        let entry = by_division.entry(division).or_default();
                        entry.push((combat, Arc::clone(&attackers)));
                    }
                }
            }
            Ok((by_division, provinces))
        })
    }

    /// Where each attacking unit of a land combat stands, in order, read from the rows of
    /// its recorded class: the game compares every attacker's own location.
    fn attacker_locations(&self, row: &replay_data::rows::LandCombatRow) -> Fact<Vec<Fact<u32>>> {
        if row.attacker_classes.len() != row.attackers.len() {
            return Err(format!(
                "land combat {:#x} records {} classes for {} attackers",
                row.id,
                row.attacker_classes.len(),
                row.attackers.len()
            ));
        }
        Ok(row
            .attackers
            .iter()
            .zip(&row.attacker_classes)
            .map(|(&unit, &class)| self.combatant_location(unit, class))
            .collect())
    }

    fn combatant_location(&self, unit: u64, class: u32) -> Fact<u32> {
        use engine_native::cland_combat::{DIVISION_CLASS, RAILWAY_GUN_CLASS, TASK_FORCE_CLASS};
        let location = match class {
            DIVISION_CLASS => self.division(unit)?.map(|row| row.province),
            TASK_FORCE_CLASS => self.task_force(unit)?.map(|row| row.province),
            RAILWAY_GUN_CLASS => {
                let rows = self.table::<tables::RailwayGunCounterInputs>()?;
                rows.binary_search_by_key(&unit, |row| row.gun)
                    .ok()
                    .map(|at| rows[at].province)
            }
            _ => {
                return Err(format!(
                    "land combat unit {unit:#x} has native class {class}, whose location is not recorded"
                ));
            }
        };
        location.ok_or_else(|| {
            format!("land combat unit {unit:#x} of native class {class} has no row of its class")
        })
    }

    /// The recorded state of a division as `unit_path` reads it.
    pub fn army(&self, division: u64) -> Fact<Army> {
        let row = self
            .division(division)?
            .ok_or_else(|| format!("division {division:#x} is not recorded"))?;
        let movement = self.movement(division)?;
        let flags = self.move_flags(division)?;
        let combats = match self.combats()?.0.get(&division) {
            None => Vec::new(),
            Some(engaged) => engaged
                .iter()
                .map(|(combat, attackers)| -> Fact<ArmyCombat> {
                    let mut combat = combat.clone();
                    if combat.defending {
                        combat.attacker_locations = (**attackers).clone()?;
                    }
                    Ok(combat)
                })
                .collect::<Fact<Vec<_>>>()?,
        };
        if combats.len() != movement.combat_count as usize {
            return Err(format!(
                "division {division:#x} records {} combats but {} land combats name it",
                movement.combat_count,
                combats.len()
            ));
        }
        // Read commander bans only for a division on a side of one of its combats.
        let bans = if combats.is_empty() {
            0
        } else {
            self.retreat_bans(division)?
        };
        let owner = self.slot(row.logical_owner)?;
        let navy_country = if (row.expeditionary_owner as i32) >= 1 {
            owner
        } else {
            self.slot(row.roster_owner)?
        };
        Ok(Army {
            location: row.province,
            owner,
            navy_country,
            orders_group: row.orders_group,
            retreating: movement.retreat != 0,
            naval_transfer: movement.naval_transfer != 0,
            exiled: flags & DivisionStatusRow::MOVE_EXILED != 0,
            exfiltrates: flags & DivisionStatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST != 0,
            strength: row.hit_points,
            organisation: row.organisation,
            route_first: (movement.path_total > 0)
                .then(|| movement.path.first().copied())
                .flatten(),
            combats,
            no_retreat_defending: bans & DivisionRetreatBanRow::DEFENDING != 0,
            no_retreat_attacking: bans & DivisionRetreatBanRow::ATTACKING != 0,
        })
    }

    /// Cache once per division, source, perspective and hour; the answer depends
    /// on neither target nor viewer. Search once per [`ReachKey`] from the army
    /// fields it reads and answer every source from that flood fill.
    pub fn reaches(&self, division: u64, source: u32) -> Eval {
        if let Some(answer) = self.cells.reach().get(&(division, source)) {
            return answer.clone();
        }
        let answer = match self.search(division, source) {
            Ok(reached) => Eval::from_bool(reached),
            Err(reason) => Eval::unknown(reason),
        };
        self.cells
            .reach()
            .insert((division, source), answer.clone());
        answer
    }

    fn search(&self, division: u64, source: u32) -> Fact<bool> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let army = self.army(division)?;
        let mover = Mover { graph, world: self };
        let search = self.search_of(&army);
        let mut search = search
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let search = search.get_or_insert_with(|| mover.search(&army));
        let reachable = &self.cells.reachable[usize::from(army.exfiltrates)];
        mover.army_reaches_resumed(&army, source, self.assets.avoid_enemy, search, || {
            reachable.get_or_init(|| mover.reachable(army.exfiltrates))
        })
    }

    /// The hour's search for `army`'s key.
    fn search_of(&self, army: &Army) -> Arc<Mutex<Option<Search>>> {
        Arc::clone(
            self.cells
                .searches
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(army.reach_key())
                .or_default(),
        )
    }

    /// Runs the army searches of `divisions`' keys (each key once) further,
    /// a few edges at a time and in turn, until `done` or every one has
    /// reached its end. A query of them answers alike however far they ran
    /// before ([`Mover::advance_by`]), so running them ahead only saves the
    /// queries the time.
    pub fn run_ahead(&self, divisions: &[u64], done: &dyn Fn() -> bool) {
        /// Edges between two looks at `done`.
        const STEP: usize = 256;
        let Ok(graph) = self.assets.moves.as_ref() else {
            return;
        };
        let mover = Mover { graph, world: self };
        let mut seen = HashSet::new();
        let mut open: Vec<(Army, Arc<Mutex<Option<Search>>>)> = divisions
            .iter()
            .filter_map(|&division| self.army(division).ok())
            .filter(|army| seen.insert(army.reach_key()))
            .map(|army| {
                let search = self.search_of(&army);
                (army, search)
            })
            .collect();
        while !open.is_empty() && !done() {
            open.retain(|(army, search)| {
                let mut search = search
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let search = search.get_or_insert_with(|| mover.search(army));
                #[cfg(any(test, feature = "test-support"))]
                RAN_AHEAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                !mover.advance_by(army, search, STEP)
            });
        }
    }

    /// Whether the armies of `divisions` exfiltrate, each value once; a
    /// division whose army is not decided is left out.
    pub fn exfiltrates(&self, divisions: impl IntoIterator<Item = u64>) -> BTreeSet<bool> {
        divisions
            .into_iter()
            .filter_map(|division| Some(self.army(division).ok()?.exfiltrates))
            .collect()
    }

    /// Builds the hour's reachable components for armies that do or do not
    /// exfiltrate, which every search query first reads.
    pub fn reachable(&self, exfiltrates: bool) {
        if let Ok(graph) = self.assets.moves.as_ref() {
            let mover = Mover { graph, world: self };
            self.cells.reachable[usize::from(exfiltrates)]
                .get_or_init(|| mover.reachable(exfiltrates));
        }
    }

    fn province(&self, province: u32) -> Fact<&'a ProvinceRow> {
        self.world
            .provinces()
            .get(&province)
            .ok_or_else(|| format!("province {province} is not recorded"))?;
        let rows = match self.world.frame().view(tables::Provinces::ID.index()) {
            replay_frame::frame::TableView::Complete(rows)
            | replay_frame::frame::TableView::Truncated { rows, .. } => {
                rows.rows::<tables::Provinces>().unwrap_or_default()
            }
            _ => &[],
        };
        let past = rows.partition_point(|row| row.id <= province);
        past.checked_sub(1)
            .and_then(|at| rows.get(at))
            .filter(|row| row.id == province)
            .ok_or_else(|| format!("province {province} is not recorded"))
    }

    /// The division's commander retreat bans as of its owner's daily read
    /// (DivisionRetreatBans); a division without a row has none.
    fn retreat_bans(&self, division: u64) -> Fact<u8> {
        let rows = self.table::<tables::DivisionRetreatBans>()?;
        Ok(rows
            .binary_search_by_key(&division, |row| row.division)
            .map_or(0, |at| rows[at].bans))
    }

    /// `DivisionStatus.move_flags`; a division without a status row is refused.
    fn move_flags(&self, division: u64) -> Fact<u8> {
        let flags = get(&self.cells.move_flags, || {
            Ok(self
                .table::<tables::DivisionStatus>()?
                .iter()
                .map(|row| (row.division, row.move_flags))
                .collect())
        })?;
        flags
            .get(&division)
            .copied()
            .ok_or_else(|| format!("division {division:#x} has no status row"))
    }

    /// The `RelationLinks` bits `a`'s matrix holds toward slot `b`, read
    /// only for the owner asked: the table is in key order (raw country,
    /// target slot), so each of `a`'s raw references is one binary search.
    /// A slot with several raw references answers from the highest that has
    /// a row, as an index built in table order would. A row whose country
    /// links to no slot may be `a`'s: toward the same target it leaves an
    /// absent answer unknown.
    fn relation_link(&self, a: Country, b: Country) -> Fact<u8> {
        let rows = self.table::<tables::RelationLinks>()?;
        let raws = get(&self.cells.slot_raws, || {
            let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for (&raw, &slot) in self.world.links() {
                out.entry(slot).or_default().push(raw);
            }
            for raws in out.values_mut() {
                raws.reverse();
            }
            Ok(out)
        })?;
        for &raw in raws.get(&a).into_iter().flatten() {
            let at = rows.partition_point(|row| (row.country, row.target_index) < (raw, b));
            if let Some(row) = rows.get(at)
                && (row.country, row.target_index) == (raw, b)
            {
                return Ok(row.links);
            }
        }
        let unlinked = get(&self.cells.unlinked_links, || {
            // One lookup per distinct country: the rows are in key order.
            let mut out = BTreeMap::new();
            let mut at = 0;
            while let Some(first) = rows.get(at) {
                let end = at + rows[at..].partition_point(|row| row.country == first.country);
                if self.world.slot(first.country).is_none() {
                    for row in &rows[at..end] {
                        out.entry(row.target_index).or_insert(row.country);
                    }
                }
                at = end;
            }
            Ok(out)
        })?;
        match unlinked.get(&b) {
            Some(raw) => Err(format!(
                "a RelationLinks row's country {raw} has no CountryLinks row"
            )),
            None => Ok(0),
        }
    }

    fn country_wars(&self) -> Result<&BTreeMap<u32, Wars>, String> {
        get(&self.cells.country_wars, || {
            let slots = |links: &[u32]| {
                links
                    .iter()
                    .map(|&raw| self.slot(raw))
                    .collect::<Fact<Vec<_>>>()
            };
            let mut out = BTreeMap::new();
            for row in self.table::<tables::CountryWars>()? {
                out.insert(
                    self.slot(row.country)?,
                    Wars {
                        can_occupy_non_war: row.can_occupy_non_war != 0,
                        enemies: if row.enemies_read != 0 {
                            Some(slots(&row.enemies)?)
                        } else {
                            None
                        },
                    },
                );
            }
            Ok(out)
        })
    }

    /// Each country's active war opponents from `WarRelations`.
    fn war_sets(&self) -> Result<&BTreeMap<u32, BTreeSet<u32>>, String> {
        get(&self.cells.war_sets, || {
            let hour = self.world.hour();
            let mut out: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
            for row in self.table::<tables::WarRelations>()? {
                if !ongoing(row.start_hour, row.end_hour, hour) {
                    continue;
                }
                let (a, b) = (
                    self.slot(row.first_country)?,
                    self.slot(row.second_country)?,
                );
                if a != b {
                    out.entry(a).or_default().insert(b);
                    out.entry(b).or_default().insert(a);
                }
            }
            Ok(out)
        })
    }

    fn state_of(&self, province: u32) -> Fact<Option<u32>> {
        let row = self
            .world
            .provinces()
            .get(&province)
            .ok_or_else(|| format!("province {province} is not recorded"))?;
        Ok((row.state != 0).then_some(row.state))
    }

    fn native_relations(&self) -> Result<&NativeRelations, String> {
        get(&self.cells.native_relations, || {
            NativeRelations::read(self.world.frame(), self.world.links())
                .map_err(|error| error.to_string())
        })
    }
}

impl MoveWorld for Moves<'_, '_> {
    /// Read the route owner's directed relation cache. Target zero also has
    /// independently recorded war-pointer operands. A known true arm suffices
    /// even if the other input was refused.
    fn path_friendly(&self, owner: Country, target: Country) -> Fact<bool> {
        let co_belligerent = self
            .native_relations()
            .and_then(|relations| relations.co_belligerent(owner, target));
        if matches!(co_belligerent, Ok(true)) {
            return Ok(true);
        }
        // The signed-positive receiver-ID gate applies to every target.
        let at_war = self
            .world
            .raw_country(owner)
            .ok_or_else(|| format!("country slot {owner} has no Countries row"))
            .and_then(|raw| {
                if raw as i32 <= 0 {
                    Ok(false)
                } else if target != 0 {
                    self.at_war(owner, target)
                } else {
                    self.native_relations()
                        .and_then(|relations| relations.zero_at_war(owner))
                }
            });
        match (co_belligerent, at_war) {
            (Ok(true), _) | (_, Ok(true)) => Ok(true),
            (Ok(false), Ok(false)) => Ok(false),
            (Err(why), Ok(false)) | (Ok(false), Err(why)) => Err(why),
            (Err(cache), Err(war)) => Err(format!("{cache}; {war}")),
        }
    }
    fn at_war(&self, a: Country, b: Country) -> Fact<bool> {
        if a == 0 || a == b {
            return Ok(false);
        }
        let raw = self
            .world
            .raw_country(a)
            .ok_or_else(|| format!("country slot {a} has no Countries row"))?;
        if raw as i32 <= 0 {
            return Ok(false);
        }
        if b == 0 {
            return self.native_relations()?.zero_at_war(a);
        }
        self.world.at_war(a, b).map_err(|why| why.to_string())
    }

    fn military_access(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(self.relation_link(a, b)? & RelationLinkRow::MILITARY_ACCESS != 0)
    }

    fn docking_rights(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(self.relation_link(a, b)? & RelationLinkRow::DOCKING_RIGHTS != 0)
    }

    fn counter_rail_connected(&self, from: u32, to: u32) -> Fact<bool> {
        self.counter
            .ok_or("fresh counter class facts are unavailable")?
            .rail_connected(from, to)
    }

    fn counter_railway_orders_group(&self, handle: u64) -> Fact<u64> {
        self.counter
            .ok_or("fresh counter class facts are unavailable")?
            .rail_orders_group(handle)
    }

    fn counter_task_force_access(&self, handle: u64, to: u32) -> Fact<bool> {
        self.counter
            .ok_or("fresh counter class facts are unavailable")?
            .naval_access(self, handle, to)
    }

    fn counter_task_force_submarine(&self, handle: u64) -> Fact<bool> {
        self.counter
            .ok_or("fresh counter class facts are unavailable")?
            .all_submarine(handle)
    }

    fn counter_task_force_hostile_allows(&self, handle: u64, to: u32) -> Fact<bool> {
        self.counter
            .ok_or("fresh counter class facts are unavailable")?
            .hostile_allows(self, handle, to)
    }

    /// A subject relation in force this hour, as `World::overlord` reads
    /// the table: ended relations stay in it.
    fn subject_tie(&self, a: Country, b: Country) -> Fact<bool> {
        let subjects = get(&self.cells.subjects, || {
            let hour = self.world.hour();
            let mut out = BTreeSet::new();
            for row in self.table::<tables::Subjects>()? {
                if !ongoing(row.start_hour, row.end_hour, hour) {
                    continue;
                }
                let (overlord, subject) = (self.slot(row.overlord)?, self.slot(row.subject)?);
                out.extend([(overlord, subject), (subject, overlord)]);
            }
            Ok(out)
        })?;
        Ok(subjects.contains(&(a, b)))
    }

    fn faction(&self, country: Country) -> Fact<Option<u64>> {
        self.world.faction(country).map_err(|why| why.to_string())
    }

    fn overlord(&self, country: Country) -> Fact<Country> {
        Ok(self
            .world
            .overlord(country)
            .map_err(|why| why.to_string())?
            .unwrap_or(0))
    }

    /// A missing `CountryWars` row means the byte is 0.
    fn can_occupy_non_war(&self, country: Country) -> Fact<bool> {
        Ok(self
            .country_wars()?
            .get(&country)
            .is_some_and(|wars| wars.can_occupy_non_war))
    }

    /// Use a recorded vector when read directly (civil-war revolter or retired
    /// war-relation owner). Otherwise build the set of other endpoints of the
    /// country's active war relations; compare sets.
    fn enemies(&self, country: Country) -> Fact<Vec<Country>> {
        if let Some(read) = self
            .country_wars()?
            .get(&country)
            .and_then(|wars| wars.enemies.clone())
        {
            return Ok(read);
        }
        Ok(self
            .war_sets()?
            .get(&country)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default())
    }

    fn controller(&self, province: u32) -> Fact<Country> {
        let row = self
            .world
            .provinces()
            .get(&province)
            .ok_or_else(|| format!("province {province} is not recorded"))?;
        self.slot(row.controller)
    }

    fn controller_raw_positive(&self, province: u32) -> Fact<bool> {
        self.world
            .provinces()
            .get(&province)
            .map(|row| row.controller as i32 > 0)
            .ok_or_else(|| format!("province {province} is not recorded"))
    }

    fn state_owner(&self, province: u32) -> Fact<Country> {
        let row = self
            .world
            .provinces()
            .get(&province)
            .ok_or_else(|| format!("province {province} is not recorded"))?;
        self.slot(row.owner)
    }

    fn state_owner_raw_positive(&self, province: u32) -> Fact<bool> {
        self.world
            .provinces()
            .get(&province)
            .map(|row| row.owner as i32 > 0)
            .ok_or_else(|| format!("province {province} is not recorded"))
    }

    fn demilitarized(&self, province: u32) -> Fact<bool> {
        let Some(state) = self.state_of(province)? else {
            return Ok(false);
        };
        let states = get(&self.cells.demilitarized, || {
            Ok(self
                .table::<tables::StateFlags>()?
                .iter()
                .filter(|row| row.demilitarized != 0)
                .map(|row| row.state)
                .collect())
        })?;
        Ok(states.contains(&state))
    }

    fn impassable(&self, province: u32) -> Fact<bool> {
        Ok(self
            .state_of(province)?
            .is_some_and(|state| self.assets.trigger_content.impassable.contains(&state)))
    }

    fn owner_area_missing(&self, province: u32) -> Fact<bool> {
        Ok(self.province(province)?.move_gate & ProvinceRow::MOVE_NULL_OWNER_AREA != 0)
    }

    fn border_conflict(&self, province: u32) -> Fact<bool> {
        Ok(self.province(province)?.move_gate & ProvinceRow::MOVE_BORDER_CONFLICT != 0)
    }

    /// The recorded groups resolved by sides of the province's conflicts
    /// that are border wars.
    fn border_conflict_admits(&self, province: u32, orders_group: u64) -> Fact<bool> {
        let conflicts = get(&self.cells.border_conflicts, || {
            Ok(self
                .table::<tables::BorderConflicts>()?
                .iter()
                .map(|row| (row.province, row.groups.clone()))
                .collect())
        })?;
        Ok(orders_group != 0
            && conflicts
                .get(&province)
                .is_some_and(|groups| groups.contains(&orders_group)))
    }

    fn has_naval_base(&self, province: u32) -> Fact<bool> {
        Ok(self.naval_bases()?.contains_key(&province))
    }

    fn naval_base_active(&self, province: u32) -> Fact<bool> {
        Ok(self.naval_bases()?.get(&province).copied().unwrap_or(false))
    }

    fn port_sea(&self, province: u32) -> Fact<u32> {
        Ok(u32::try_from(self.province(province)?.port_sea).unwrap_or(0))
    }

    fn armies(&self, province: u32) -> Fact<Vec<Occupant>> {
        let armies = get(&self.cells.armies, || {
            let mut out: BTreeMap<u32, Vec<Occupant>> = BTreeMap::new();
            for row in self.table::<tables::Divisions>()? {
                let retreating = self.movement(row.id)?.retreat != 0;
                out.entry(row.province).or_default().push(Occupant {
                    owner: self.slot(row.logical_owner)?,
                    retreating,
                    strength: row.hit_points,
                });
            }
            Ok(out)
        })?;
        Ok(armies.get(&province).cloned().unwrap_or_default())
    }

    fn land_combat(&self, province: u32) -> Fact<bool> {
        Ok(self.combats()?.1.contains(&province))
    }

    fn strait_fleet(&self, through: u32, owner: Country) -> Fact<StraitFleet> {
        let orders = self.table::<tables::ProvinceUnitOrder>()?;
        let occupants = get(&self.cells.occupants, || {
            Ok(orders
                .iter()
                .enumerate()
                .map(|(at, row)| (row.province, at))
                .collect())
        })?;
        let handles = occupants
            .get(&through)
            .map_or(&[][..], |&at| orders[at].mixed.as_slice());
        for handle in handles {
            let Some(task_force) = self.task_force(*handle)? else {
                continue;
            };
            if !self.at_war(self.slot(task_force.logical_owner)?, owner)? {
                continue;
            }
            let (ships, all_flag_seven) = self.fleets()?.get(handle).copied().unwrap_or((0, true));
            let routed = task_force.route_count >= 1;
            if ships == 0 || routed || task_force.mission != 0 || all_flag_seven {
                continue;
            }
            let roster = self.slot(task_force.roster_owner)?;
            let level = if unit_path::discloses(self, roster, owner, false)? {
                3
            } else {
                self.intel()?.get(&(*handle, owner)).copied().unwrap_or(0)
            };
            return Ok(if level != 0 {
                StraitFleet::Visible
            } else {
                StraitFleet::Unseen
            });
        }
        Ok(StraitFleet::None)
    }

    fn strategic_region(&self, province: u32) -> Option<u32> {
        self.assets.regions.get(&province).copied()
    }

    fn region_access(&self, country: Country, region: u32) -> Fact<u8> {
        let modes = get(&self.cells.region_access, || {
            let mut out = BTreeMap::new();
            for row in self.table::<tables::NavalRegionAccess>()? {
                out.insert((self.slot(row.country)?, row.region), row.mode);
            }
            Ok(out)
        })?;
        Ok(modes.get(&(country, region)).copied().unwrap_or(0))
    }

    fn has_children(&self, triggers: &[Item]) -> Fact<bool> {
        self.world.has_children(triggers)
    }

    fn evaluate(&self, triggers: &[Item], country: Country) -> Fact<bool> {
        match self
            .world
            .evaluate(triggers, Scopes::of(Scope::Country(country)))
        {
            Eval::True => Ok(true),
            Eval::False => Ok(false),
            Eval::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
        }
    }
}

impl<'a> Moves<'_, 'a> {
    fn naval_bases(&self) -> Result<&BTreeMap<u32, bool>, String> {
        get(&self.cells.naval_bases, || {
            Ok(self
                .table::<tables::NavalBases>()?
                .iter()
                .map(|row| (row.province, row.level > 0))
                .collect())
        })
    }

    fn task_force(&self, id: u64) -> Fact<Option<&'a TaskForceRow>> {
        let rows = self.table::<tables::TaskForces>()?;
        let index = get(&self.cells.task_forces, || {
            Ok(rows
                .iter()
                .enumerate()
                .map(|(at, row)| (row.id, at))
                .collect())
        })?;
        Ok(index.get(&id).map(|&at| &rows[at]))
    }

    fn fleets(&self) -> Result<&BTreeMap<u64, (usize, bool)>, String> {
        get(&self.cells.fleets, || {
            let flags: BTreeMap<u64, u64> = self
                .table::<tables::ShipModels>()?
                .iter()
                .map(|row| (row.ship, row.formation_flags))
                .collect();
            let mut out: BTreeMap<u64, (usize, bool)> = BTreeMap::new();
            for ship in self.table::<tables::Ships>()? {
                let formation = flags
                    .get(&ship.id)
                    .ok_or_else(|| format!("ship {:#x} has no model row", ship.id))?;
                let entry = out.entry(ship.task_force).or_insert((0, true));
                entry.0 += 1;
                // Formation flag is bit 7 of the raw ship flags.
                entry.1 &= formation & 0x80 != 0;
            }
            Ok(out)
        })
    }

    fn intel(&self) -> Result<&BTreeMap<(u64, u32), i8>, String> {
        get(&self.cells.intel, || {
            let mut out = BTreeMap::new();
            for row in self.table::<tables::UnitIntel>()? {
                // The first entry for a viewer answers.
                out.entry((row.unit, row.viewer)).or_insert(row.level);
            }
            Ok(out)
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "movement_reads/test_support.rs"]
pub mod test_support;

#[cfg(test)]
mod combat_location_tests {
    use super::test_support::{frame, put, unnamed_assets};
    use super::*;
    use installed_content_unit_path::unit_path::{CommonCombat, CommonMoveWorld};
    use replay_data::rows::{
        LandCombatDetailsRow, LandCombatRow, RailwayGunCounterInputRow, Recorded,
    };

    const TASK_FORCE: u64 = 0x400;
    const GUN: u64 = 0x500;
    const UNKNOWN_CLASS: u64 = 0x600;

    /// Combat 7 in province 1: a division, a task force and a railway gun attack division
    /// 0x100. Combat 8 in province 2: a class-5 unit and division 0x300 attack 0x800.
    fn mixed_frame() -> replay_frame::frame::DecodedFrame {
        let mut frame = frame();
        let divisions = [
            (0x100, 1, 10, 1),
            (0x200, 3, 11, 1),
            (0x300, 4, 11, 1),
            (0x800, 2, 10, 1),
            (0x900, 3, 11, 0),
        ];
        put::<tables::Divisions>(
            &mut frame,
            divisions
                .map(|(id, province, owner, _)| DivisionRow {
                    id,
                    province,
                    roster_owner: owner,
                    logical_owner: owner,
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::DivisionMovement>(
            &mut frame,
            divisions
                .map(|(division, _, _, combat_count)| DivisionMovementRow {
                    division,
                    combat_count,
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::DivisionStatus>(
            &mut frame,
            divisions
                .map(|(division, ..)| DivisionStatusRow {
                    division,
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::TaskForces>(
            &mut frame,
            vec![TaskForceRow {
                id: TASK_FORCE,
                province: 2,
                ..Default::default()
            }],
        );
        put::<tables::RailwayGunCounterInputs>(
            &mut frame,
            vec![RailwayGunCounterInputRow {
                gun: GUN,
                province: 4,
                ..Default::default()
            }],
        );
        put::<tables::LandCombats>(
            &mut frame,
            vec![
                LandCombatRow {
                    id: 7,
                    province: 1,
                    duration: 5,
                    attackers: vec![0x200, TASK_FORCE, GUN],
                    attacker_classes: vec![0, 1, 13],
                    defenders: vec![0x100],
                    defender_classes: vec![0],
                    ..Default::default()
                },
                LandCombatRow {
                    id: 8,
                    province: 2,
                    duration: 5,
                    attackers: vec![UNKNOWN_CLASS, 0x300],
                    attacker_classes: vec![5, 0],
                    defenders: vec![0x800],
                    defender_classes: vec![0],
                    ..Default::default()
                },
            ],
        );
        put::<tables::LandCombatDetails>(
            &mut frame,
            [7, 8]
                .map(|combat| LandCombatDetailsRow {
                    combat,
                    border_war: Recorded::Value(None),
                    ..Default::default()
                })
                .into(),
        );
        frame
    }

    #[test]
    fn mixed_attackers_stand_where_their_own_class_records_them() {
        let assets = unnamed_assets();
        let frame = mixed_frame();
        let world = World::read(&frame, assets.trigger_content.facts());
        let cells = Cells::default();
        let moves = Moves::new(&world, assets.movement_reads(), &cells);
        // A defender sees every attacking unit's own location.
        assert_eq!(
            moves.army(0x100).unwrap().combats,
            [ArmyCombat {
                province: 1,
                land: true,
                attacking: false,
                defending: true,
                attacker_locations: vec![3, 2, 4],
                minimum_duration: true,
            }]
        );
        // An attacker never reads its own side's locations, known or not.
        for division in [0x200, 0x300] {
            let combats = moves.army(division).unwrap().combats;
            assert_eq!(combats.len(), 1);
            assert!(combats[0].attacking && combats[0].attacker_locations.is_empty());
        }
        // Only the defender facing the unlocated unit is unknown, and says why.
        let unknown = moves.army(0x800).unwrap_err();
        assert!(unknown.contains("native class 5"), "{unknown}");
        assert!(moves.army(0x900).unwrap().combats.is_empty());
        assert_eq!(moves.land_combat(2), Ok(true));
        assert_eq!(moves.land_combat(3), Ok(false));
    }

    #[test]
    fn selected_counterattack_finds_any_attacker_class_and_names_an_unlocated_one() {
        let assets = unnamed_assets();
        let frame = mixed_frame();
        let world = World::read(&frame, assets.trigger_content.facts());
        let cells = Cells::default();
        let common = CommonCells::new(&frame);
        let moves = Moves::for_common(&world, assets.movement_reads(), &cells, &common).unwrap();
        let land = |combat| CommonCombat { combat, kind: 0 };
        assert_eq!(moves.combat_attacker_at(land(7), 3), Ok(true));
        assert_eq!(moves.combat_attacker_at(land(7), 2), Ok(true));
        assert_eq!(moves.combat_attacker_at(land(7), 4), Ok(true));
        assert_eq!(moves.combat_attacker_at(land(7), 1), Ok(false));
        // A located attacker there decides; elsewhere the unlocated one leaves it unknown.
        assert_eq!(moves.combat_attacker_at(land(8), 4), Ok(true));
        let unknown = moves.combat_attacker_at(land(8), 1).unwrap_err();
        assert!(unknown.contains("native class 5"), "{unknown}");
    }
}

#[cfg(test)]
mod recorded_source_tests {
    use super::*;
    use replay_data::rows::{Recorded, TableId, TableRows};
    use replay_frame::frame::{DecodedSource, TableView};

    #[test]
    fn decoded_movement_keeps_full_last_province_row_and_partial_gate() {
        let assets = test_support::assets();
        let mut frame = test_support::frame();
        let mut first = frame.complete::<tables::Provinces>().unwrap()[0].clone();
        first.fort_positive = 1;
        first.move_gate = ProvinceRow::MOVE_BORDER_CONFLICT;
        first.port_sea = 93;
        first.fort_bunker = Recorded::Value(Some(-27_000));
        first.owner_area = Recorded::Value(Some(101));
        let mut last = first.clone();
        last.port_sea = 94;
        last.owner_area = Recorded::Value(None);
        *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: TableRows::Provinces(vec![first, last]),
            missing: 7,
        };
        let source: &dyn DecodedSource = &frame;
        let before = match &*frame.tables[TableId::Provinces.index()] {
            TableView::Truncated {
                rows: TableRows::Provinces(rows),
                ..
            } => rows
                .iter()
                .map(|row| (row.id, row))
                .collect::<BTreeMap<_, _>>(),
            _ => unreachable!(),
        };
        let world = World::read(source, assets.trigger_content.facts());
        let cells = Cells::default();
        let moves = Moves::new(&world, assets.movement_reads(), &cells);
        let actual = moves.province(1).unwrap();
        assert!(std::ptr::eq(actual, before[&1]));
        assert_eq!(
            (actual.fort_positive, actual.move_gate, actual.port_sea),
            (1, ProvinceRow::MOVE_BORDER_CONFLICT, 94)
        );
        assert_eq!(actual.fort_bunker, Recorded::Value(Some(-27_000)));
        assert_eq!(actual.owner_area, Recorded::Value(None));
        assert_eq!(moves.province(2).unwrap_err(), "province 2 is not recorded");
        let gap = world.core_gap().unwrap().to_string();
        assert!(gap.contains("provinces: read only in part (7 rows missing)"), "{gap}");
    }
}
