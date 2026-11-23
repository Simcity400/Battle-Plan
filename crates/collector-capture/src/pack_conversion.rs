use collector_characters::extended::characters::CharacterConversion;
use collector_intel::intel::{IntelConversion, ScriptVariableComparison};
use collector_lexical::lexical::LexicalConversion;
use collector_projection::projection::Projection;
use replay_data::rows::ScriptVariablesRow;
use replay_frame::frame::BaselineToken;

use crate::extended::HELPER_PACKS;
use crate::extended::PACK_NAMES;
use collector_intel::intel::VariableTotals;
use collector_intel::intel::VariableWork;
use replay_data::rows::{TABLE_COUNT, TableId};
use std::sync::atomic::{AtomicUsize, Ordering};

const PACKS: usize = PACK_NAMES.len();
const TOP: usize = 4;

pub(crate) const MOVABLE: usize = {
    let mut count = 0;
    let mut pack = 0;
    while pack < PACKS {
        if HELPER_PACKS[pack] {
            count += 1;
        }
        pack += 1;
    }
    count
};
const MOVABLE_PACKS: [u8; MOVABLE] = {
    let mut order = [0; MOVABLE];
    let mut next = 0;
    let mut pack = 0;
    while pack < PACKS {
        if HELPER_PACKS[pack] {
            order[next] = pack as u8;
            next += 1;
        }
        pack += 1;
    }
    order
};
use collector_workers::spool_helper::MAX_HELPERS;
const _: () = assert!(MAX_HELPERS == MOVABLE);
pub const MAX_THREADS: usize = MAX_HELPERS + 1;
pub const SPOOL: usize = 0;
const _: () = assert!(PACKS <= u8::MAX as usize + 1 && MAX_THREADS <= u8::MAX as usize);

#[derive(Clone, Copy)]
struct ThreadName(usize);

impl std::fmt::Display for ThreadName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            SPOOL => formatter.write_str("spool"),
            helper => write!(formatter, "helper_{helper}"),
        }
    }
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Clone, Copy)]
pub struct Plan {
    order: [u8; MOVABLE],
    helpers: usize,
}

impl Plan {
    pub fn new(estimates: &[Option<u64>; PACKS], pool: usize) -> Self {
        let mut order = MOVABLE_PACKS;
        order.sort_unstable_by_key(|&pack| {
            (
                std::cmp::Reverse(estimates[usize::from(pack)].unwrap_or(u64::MAX)),
                pack,
            )
        });
        let helpers = match order.first().map(|&pack| estimates[usize::from(pack)]) {
            Some(Some(largest)) => {
                let total = estimates
                    .iter()
                    .flatten()
                    .fold(0_u64, |total, &ns| total.saturating_add(ns));
                let threads = usize::try_from(total.div_ceil(largest.max(1))).unwrap_or(usize::MAX);
                threads.saturating_sub(1).max(1).min(pool)
            }
            _ => pool,
        };
        Self { order, helpers }
    }

    pub fn helpers(&self) -> usize {
        self.helpers
    }

    pub fn queue(&self) -> Queue<'_> {
        Queue {
            order: &self.order,
            next: AtomicUsize::new(0),
        }
    }
}

pub struct Queue<'a> {
    order: &'a [u8],
    next: AtomicUsize,
}

impl Queue<'_> {
    pub fn take(&self) -> Option<usize> {
        let at = self.next.fetch_add(1, Ordering::Relaxed);
        self.order.get(at).map(|&pack| usize::from(pack))
    }
}

pub struct HourCosts {
    thread: u8,
    packs: [Option<u64>; PACKS],
    by: [u8; PACKS],
    split: [u64; PACKS],
    tables: [u64; TABLE_COUNT],
    threads: [Option<u64>; MAX_THREADS],
    pub script_variables: Option<VariableWork>,
}

impl HourCosts {
    pub const EMPTY: Self = Self::of(SPOOL);

    pub const fn of(thread: usize) -> Self {
        Self {
            thread: thread as u8,
            packs: [None; PACKS],
            by: [0; PACKS],
            split: [0; PACKS],
            tables: [0; TABLE_COUNT],
            threads: [None; MAX_THREADS],
            script_variables: None,
        }
    }

    pub fn packs(&mut self) -> Packs<'_> {
        Packs {
            costs: self,
            next: 0,
        }
    }

    pub fn thread(&mut self, started: std::time::Instant) {
        self.threads[usize::from(self.thread)] = Some(elapsed_ns(started));
    }

    pub fn converted(&mut self, pack: usize, started: std::time::Instant) {
        self.packs[pack] = Some(elapsed_ns(started));
        self.by[pack] = self.thread;
        self.split[pack] = collector_projection::table_clock::drain(&mut self.tables);
    }

    pub fn merge(&mut self, other: &Self) {
        for pack in 0..PACKS {
            if other.packs[pack].is_some() {
                self.packs[pack] = other.packs[pack];
                self.by[pack] = other.by[pack];
                self.split[pack] = other.split[pack];
            }
        }
        for (total, ns) in self.tables.iter_mut().zip(&other.tables) {
            *total = total.saturating_add(*ns);
        }
        for (wall, other) in self.threads.iter_mut().zip(other.threads) {
            if other.is_some() {
                *wall = other;
            }
        }
        if other.script_variables.is_some() {
            self.script_variables = other.script_variables;
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn converted_by(&self) -> impl Iterator<Item = Option<(u64, usize)>> + '_ {
        self.packs
            .iter()
            .zip(&self.by)
            .map(|(ns, by)| ns.map(|ns| (ns, usize::from(*by))))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn walls(&self) -> impl Iterator<Item = Option<u64>> + '_ {
        self.threads.iter().copied()
    }
}

pub struct Packs<'a> {
    costs: &'a mut HourCosts,
    next: usize,
}

impl Packs<'_> {
    pub fn skip(&mut self) {
        self.next += 1;
    }

    pub fn converted(&mut self, started: std::time::Instant) {
        self.costs.converted(self.next, started);
        self.next += 1;
    }
}

#[derive(Clone, Copy)]
struct Total {
    ns: u64,
    hours: u64,
    max_ns: u64,
}

impl Total {
    const ZERO: Self = Self {
        ns: 0,
        hours: 0,
        max_ns: 0,
    };

    fn add(&mut self, ns: u64) {
        self.ns = self.ns.saturating_add(ns);
        self.hours += 1;
        self.max_ns = self.max_ns.max(ns);
    }
}

#[derive(Clone, Copy)]
pub struct HourTop {
    threads: [Option<u64>; MAX_THREADS],
    by: [Option<u8>; PACKS],
    packs: [(usize, u64); TOP],
    tables: [(TableId, u64); TOP],
    script_variables: Option<VariableWork>,
}

fn top<K: Copy>(empty: K, costs: impl Iterator<Item = (K, u64)>) -> [(K, u64); TOP] {
    let mut top = [(empty, 0); TOP];
    for (key, ns) in costs {
        if let Some(at) = top.iter().position(|&(_, held)| ns > held) {
            top.copy_within(at..TOP - 1, at + 1);
            top[at] = (key, ns);
        }
    }
    top
}

impl HourTop {
    pub fn status(&self) -> impl serde::Serialize + '_ {
        collector_status::status::status!({
            "threads": collector_status::status::sequence(
                self.threads
                    .iter()
                    .enumerate()
                    .filter_map(|(thread, ns)| Some((thread, (*ns)?)))
                    .map(move |(thread, ns)| {
                        collector_status::status::status!({
                            "thread": collector_status::status::DisplayValue(ThreadName(thread)),
                            "wall_ns": ns,
                            "packs": collector_status::status::sequence(
                                PACK_NAMES
                                    .iter()
                                    .zip(&self.by)
                                    .filter(move |(_, by)| by.map(usize::from) == Some(thread))
                                    .map(|(name, _)| *name),
                            ),
                        })
                    }),
            ),
            "top_packs": collector_status::status::sequence(self.packs.iter().filter(|(_, ns)| *ns != 0).map(
                |&(pack, ns)| collector_status::status::status!({ "pack": PACK_NAMES[pack], "ns": ns }),
            )),
            "top_tables": collector_status::status::sequence(self.tables.iter().filter(|(_, ns)| *ns != 0).map(
                |&(table, ns)| collector_status::status::status!({
                    "table": collector_status::status::DebugValue(table),
                    "ns": ns,
                }),
            )),
            "script_variables": self.script_variables.as_ref().map(VariableWork::status),
        })
    }
}

pub struct Conversion {
    hours: u64,
    threads: [Total; MAX_THREADS],
    packs: [Total; PACKS],
    split: [u64; PACKS],
    tables: [Total; TABLE_COUNT],
    script_variables: VariableTotals,
    estimates: [Option<u64>; PACKS],
    last: Option<HourTop>,
    latest: Option<HourTop>,
}

impl Default for Conversion {
    fn default() -> Self {
        Self {
            hours: 0,
            threads: [Total::ZERO; MAX_THREADS],
            packs: [Total::ZERO; PACKS],
            split: [0; PACKS],
            tables: [Total::ZERO; TABLE_COUNT],
            script_variables: VariableTotals::default(),
            estimates: [None; PACKS],
            last: None,
            latest: None,
        }
    }
}

impl Conversion {
    pub fn record(&mut self, hour: &HourCosts) {
        if hour.packs.iter().all(Option::is_none) {
            self.latest = None;
            return;
        }
        self.hours += 1;
        for (total, ns) in self.threads.iter_mut().zip(hour.threads) {
            if let Some(ns) = ns {
                total.add(ns);
            }
        }
        for pack in 0..PACKS {
            if let Some(ns) = hour.packs[pack] {
                self.packs[pack].add(ns);
                self.split[pack] = self.split[pack].saturating_add(hour.split[pack]);
                self.estimates[pack] = Some(ns);
            }
        }
        for (total, &ns) in self.tables.iter_mut().zip(&hour.tables) {
            if ns != 0 {
                total.add(ns);
            }
        }
        if let Some(work) = &hour.script_variables {
            self.script_variables.add(work);
        }
        let top = HourTop {
            script_variables: hour.script_variables,
            threads: hour.threads,
            by: std::array::from_fn(|pack| hour.packs[pack].map(|_| hour.by[pack])),
            packs: top(
                0,
                hour.packs
                    .iter()
                    .enumerate()
                    .filter_map(|(pack, ns)| Some((pack, (*ns)?))),
            ),
            tables: top(
                TableId::Clock,
                TableId::ALL
                    .into_iter()
                    .map(|table| (table, hour.tables[table.index()])),
            ),
        };
        self.last = Some(top);
        self.latest = Some(top);
    }

    pub fn plan(&self, pool: usize) -> Plan {
        Plan::new(&self.estimates, pool)
    }

    pub fn take_latest(&mut self) -> Option<HourTop> {
        self.latest.take()
    }

    pub fn script_variables_status(&self) -> impl serde::Serialize + '_ {
        self.script_variables.status()
    }

    pub fn status(&self) -> impl serde::Serialize + '_ {
        let total = |total: &Total| {
            collector_status::status::status!({
                "ns": total.ns,
                "hours": total.hours,
                "max_ns": total.max_ns,
            })
        };
        collector_status::status::status!({
            "hours": self.hours,
            "threads": collector_status::status::sequence(
                self.threads.iter().enumerate().filter(|(_, t)| t.hours != 0).map(
                    move |(thread, t)| {
                        collector_status::status::status!({
                            "thread": collector_status::status::DisplayValue(ThreadName(thread)),
                        })
                        .join(total(t))
                    },
                ),
            ),
            "latest_hour": self.last.as_ref().map(HourTop::status),
            "packs": collector_status::status::sequence(
                PACK_NAMES.iter().enumerate().filter(move |&(pack, _)| self.packs[pack].hours != 0).map(
                    move |(pack, name)| {
                        collector_status::status::status!({
                            "pack": *name,
                            "movable": HELPER_PACKS[pack],
                            "split_ns": self.split[pack],
                        })
                        .join(total(&self.packs[pack]))
                    },
                ),
            ),
            "tables": collector_status::status::sequence(
                TableId::ALL.iter().filter(move |table| self.tables[table.index()].hours != 0).map(
                    move |table| {
                        collector_status::status::status!({ "table": collector_status::status::DebugValue(table) })
                            .join(total(&self.tables[table.index()]))
                    },
                ),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{hour, pack};
    use super::*;

    fn row<'a>(rows: &'a serde_json::Value, key: &str, name: &str) -> &'a serde_json::Value {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row[key] == name)
            .unwrap()
    }

    #[test]
    fn session_totals_are_the_sum_of_each_hours_costs() {
        let mut conversion = Conversion::default();
        let mut first = hour(
            &[("intel", 70, 1), ("production", 20, 1), ("air", 5, 0)],
            &[(TableId::ScriptVariables, 60)],
            &[(0, 10), (1, 95)],
        );
        first.script_variables = Some(VariableWork {
            scopes: 10,
            scopes_compared: 10,
            compare_ns: 40,
            build_ns: 5,
            ns: 60,
            ..VariableWork::default()
        });
        conversion.record(&first);
        let mut second = hour(
            &[("intel", 30, 2), ("production", 50, 1)],
            &[
                (TableId::ScriptVariables, 25),
                (TableId::VariantChanges, 40),
            ],
            &[(0, 12), (1, 80), (2, 3)],
        );
        second.script_variables = Some(VariableWork {
            scopes: 11,
            scopes_compared: 2,
            rows_changed: 1,
            scopes_added: 1,
            compare_ns: 3,
            build_ns: 20,
            ns: 25,
            ..VariableWork::default()
        });
        conversion.record(&second);
        conversion.record(&HourCosts::EMPTY);
        let variables = serde_json::to_value(conversion.script_variables_status()).unwrap();
        assert_eq!(variables["hours"], 2);
        assert_eq!(variables["paths"]["changes"], 2);
        assert_eq!(variables["max_scopes"], 11);
        assert_eq!(variables["sum"]["scopes_compared"], 12);
        assert_eq!(variables["max"]["scopes_compared"], 10);
        assert_eq!(variables["sum"]["select_ns"], 17);
        assert_eq!(variables["max"]["select_ns"], 15);
        assert_eq!(variables["latest"]["scopes_added"], 1);
        let status = serde_json::to_value(conversion.status()).unwrap();
        assert_eq!(status["hours"], 2);
        let packs = &status["packs"];
        assert_eq!(packs.as_array().unwrap().len(), 3);
        for (name, ns, hours, max) in [
            ("intel", 100, 2, 70),
            ("production", 70, 2, 50),
            ("air", 5, 1, 5),
        ] {
            let pack = row(packs, "pack", name);
            assert_eq!(pack["ns"], ns, "{name}");
            assert_eq!(pack["hours"], hours, "{name}");
            assert_eq!(pack["max_ns"], max, "{name}");
        }
        assert_eq!(row(packs, "pack", "intel")["movable"], true);
        let tables = &status["tables"];
        assert_eq!(row(tables, "table", "ScriptVariables")["ns"], 85);
        assert_eq!(row(tables, "table", "VariantChanges")["hours"], 1);
        let threads = &status["threads"];
        assert_eq!(row(threads, "thread", "helper_1")["ns"], 175);
        assert_eq!(row(threads, "thread", "helper_2")["hours"], 1);
        assert_eq!(
            status["latest_hour"]["threads"],
            serde_json::json!([
                {"thread": "spool", "wall_ns": 12, "packs": []},
                {"thread": "helper_1", "wall_ns": 80, "packs": ["production"]},
                {"thread": "helper_2", "wall_ns": 3, "packs": ["intel"]},
            ])
        );
    }

    const LANE_ZERO: [&str; 25] = [
        "air",
        "army_icon_inputs",
        "naval_counters",
        "province_unit_order",
        "naval_bases",
        "naval_combat_sources",
        "supply_trains",
        "combat_progress",
        "combat_details",
        "naval_details",
        "naval_result_details",
        "construction",
        "division_presentation",
        "economy",
        "fog",
        "intel",
        "naval_history",
        "production",
        "visibility",
        "raid_target_statuses",
        "industrial_organisations",
        "production_modifiers",
        "country_tactic_unlocks",
        "weather",
        "naval_mission_inputs",
    ];

    fn queued_wall(plan: &Plan, costs: &[u64; PACKS]) -> u64 {
        let threads = plan.helpers() + 1;
        let mut free_at = [0_u64; MAX_THREADS];
        free_at[SPOOL] = (0..PACKS)
            .filter(|&pack| !HELPER_PACKS[pack])
            .map(|pack| costs[pack])
            .sum();
        let queue = plan.queue();
        loop {
            let thread = (0..threads).min_by_key(|&thread| free_at[thread]).unwrap();
            let Some(pack) = queue.take() else {
                break;
            };
            free_at[thread] += costs[pack];
        }
        free_at[..threads].iter().copied().max().unwrap()
    }

    #[test]
    fn the_conversion_wall_is_within_the_largest_pack_of_the_even_share() {
        let mut costs = [1_u64; PACKS];
        for name in LANE_ZERO {
            costs[pack(name)] = 4;
        }
        for (name, ns) in [
            ("intel", 40),
            ("division_presentation", 30),
            ("production", 25),
            ("economy", 20),
            ("fog", 15),
            ("construction", 12),
            ("air", 10),
        ] {
            costs[pack(name)] = ns;
        }
        for (name, movable) in PACK_NAMES.iter().zip(HELPER_PACKS) {
            if !movable {
                assert_eq!(costs[pack(name)], 1, "{name}");
            }
        }
        let estimates = costs.map(Some);
        let total: u64 = costs.iter().sum();
        let largest = (0..PACKS)
            .filter(|&pack| HELPER_PACKS[pack])
            .map(|pack| costs[pack])
            .max()
            .unwrap();
        let lane_zero: u64 = LANE_ZERO.iter().map(|name| costs[pack(name)]).sum();
        for pool in [2, MAX_HELPERS] {
            let plan = Plan::new(&estimates, pool);
            let threads = plan.helpers() as u64 + 1;
            let bound = total / threads + largest;
            assert!(lane_zero > bound, "pool {pool}: {lane_zero} within {bound}");
            let wall = queued_wall(&plan, &costs);
            assert!(wall <= bound, "pool {pool}: {wall} over {bound}");
        }
        let plan = Plan::new(&estimates, MAX_HELPERS);
        assert_eq!(plan.helpers() as u64, total.div_ceil(largest) - 1);
        assert_eq!(plan.queue().take(), Some(pack("intel")));
    }

    #[test]
    fn helpers_follow_the_work_up_to_the_pool_and_all_join_an_unmeasured_hour() {
        let mut estimates = [Some(10_u64); PACKS];
        assert_eq!(Plan::new(&estimates, 3).helpers(), 3);
        estimates[pack("intel")] = Some(10 * PACKS as u64);
        assert_eq!(Plan::new(&estimates, MAX_HELPERS).helpers(), 1);
        estimates[pack("air")] = None;
        let plan = Plan::new(&estimates, 5);
        assert_eq!(plan.helpers(), 5);
        assert_eq!(plan.queue().take(), Some(pack("air")));
        assert_eq!(Plan::new(&estimates, 0).helpers(), 0);
    }

    #[test]
    fn the_queue_hands_each_movable_pack_to_exactly_one_thread() {
        let plan = Plan::new(&[None; PACKS], 3);
        let queue = plan.queue();
        let taken = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    while let Some(pack) = queue.take() {
                        mine.push(pack);
                    }
                    taken.lock().unwrap().extend(mine);
                });
            }
        });
        let mut taken = taken.into_inner().unwrap();
        taken.sort_unstable();
        let movable: Vec<usize> = (0..PACKS).filter(|&pack| HELPER_PACKS[pack]).collect();
        assert_eq!(taken, movable);
        assert_eq!(queue.take(), None);
    }
}

pub struct SpoolConversion<'a, 'cache> {
    pub variables_baseline: Option<(BaselineToken, &'a [ScriptVariablesRow])>,
    pub variables_scratch: &'a mut ScriptVariableComparison,
    pub characters: Option<&'a mut CharacterConversion<'cache>>,
    pub lexical: Option<&'a mut LexicalConversion<'cache>>,
    pub intel: Option<&'a mut IntelConversion<'cache>>,
    pub projection: Option<&'a mut Projection<'cache>>,
    pub helper: Option<HelperSplit<'a>>,
}

#[derive(Clone, Copy)]
pub struct HelperSplit<'a> {
    pub helpers: collector_workers::spool_helper::Helpers<'a>,
    pub plan: Plan,
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;

    pub fn pack(name: &str) -> usize {
        PACK_NAMES.iter().position(|pack| *pack == name).unwrap()
    }

    pub fn hour(
        packs: &[(&str, u64, usize)],
        tables: &[(TableId, u64)],
        threads: &[(usize, u64)],
    ) -> HourCosts {
        let mut hour = HourCosts::EMPTY;
        for &(name, ns, thread) in packs {
            hour.packs[pack(name)] = Some(ns);
            hour.by[pack(name)] = thread as u8;
        }
        for &(table, ns) in tables {
            hour.tables[table.index()] = ns;
        }
        for &(thread, ns) in threads {
            hour.threads[thread] = Some(ns);
        }
        hour
    }
}
