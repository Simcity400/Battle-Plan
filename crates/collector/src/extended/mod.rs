//! Coordinated extended acquisition. Public arrays use global TableId indices;
//! individual packs keep compact local arrays. Allocation is off-thread.

#[cfg(test)]
use crate::sampling::CONSISTENCY_TARGET;
use crate::{
    map_capture::{MapSources, SourceChecks},
    sampling::qpc,
    table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec},
};
use engine::{
    Memory, cair_wing, carmy, cequipment_variant, cequipment_variant_pool::DivisionSource,
    corders_group, cship, walk::Outcome,
};
use replay::{
    frame::{TABLE_COUNT, TableView},
    rows::TableId,
};
use std::mem::size_of;

const MISSING: Outcome = Outcome::Unavailable {
    reason: "acquisition source not supplied",
};
const VARIANT_TABLES: [TableId; 4] = [
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantUpgrades,
];
const DIVISION_SOURCE_TABLES: [TableId; 5] = [
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
];
pub type ReadGroup = (&'static str, TableId, Vec<TableId>);

macro_rules! packs {
    ($($pack:ident),+ $(,)?) => {
        $(pub mod $pack;)+
        pub const PACK_NAMES: &[&str] = &[$(stringify!($pack)),+];
        pub const N: usize = 0 $(+ $pack::N)+;
        pub const TABLES: [TableId; N] = {
            let mut tables = [TableId::LexicalTokens; N];
            let mut next = 0;
            $(let mut index = 0;
            while index < $pack::N {
                tables[next] = $pack::TABLES[index]; next += 1; index += 1;
            })+
            assert!(next == N);
            tables
        };
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct Capacities {
            $(pub $pack: $pack::Capacities,)+
            pub division_sources: usize,
            pub variant_registry: usize,
        }
        impl Default for Capacities {
            fn default() -> Self {
                Self { $($pack: Default::default(),)+ division_sources: 8192, variant_registry: 8192 }
            }
        }
        pub struct Captured {
            $($pack: $pack::Captured,)+
            divisions: Divisions,
            registry: Registry,
            air_roots: [Option<Outcome>; 2],
            overhead: [u64; TABLE_COUNT],
        }
        impl Captured {
            pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
                assert_eq!(successful.len(), TABLE_COUNT);
                Self {
                    $($pack: $pack::Captured::with_capacities(&capacity.$pack, &local(&$pack::TABLES, successful)),)+
                    divisions: Divisions::new(capacity.division_sources, division_successful(successful)),
                    registry: Registry::new(capacity.variant_registry, variant_successful(successful)),
                    air_roots: [None; 2], overhead: [0; TABLE_COUNT],
                }
            }
            /// Reset before the shared division and map walks, not between
            /// source visitation and capture().
            pub fn reset(&mut self) {
                $(self.$pack.reset();)+
                self.divisions.reset(); self.registry.reset();
                self.air_roots = [None; 2]; self.overhead = [0; TABLE_COUNT];
            }
            pub fn allocated_bytes(&self) -> usize {
                0 $(+ self.$pack.allocated_bytes())+ + self.divisions.allocated_bytes() + self.registry.allocated_bytes()
            }
            /// Spool-thread metadata for this buffer's native row and scratch
            /// allocations. Excludes core tables, encoded frames and queues.
            pub fn allocated_by_pack(&self) -> Vec<(&'static str, usize)> {
                vec![
                    $((stringify!($pack), self.$pack.allocated_bytes()),)+
                    ("division_sources", self.divisions.allocated_bytes()),
                    ("variant_registry", self.registry.allocated_bytes()),
                ]
            }
            pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
                assert_eq!(successful.len(), TABLE_COUNT);
                $(self.$pack.grow(&mut capacity.$pack, &local(&$pack::TABLES, successful));)+
                self.divisions.grow(&mut capacity.division_sources, division_successful(successful));
                self.registry.grow(&mut capacity.variant_registry, variant_successful(successful));
            }
            /// Spool-thread conversion only. Existing core slots survive.
            pub fn insert_tables(&self, destination: &mut [TableView; TABLE_COUNT]) -> [i64; PACK_NAMES.len()] {
                let mut costs = [0; PACK_NAMES.len()];
                let mut slots = costs.iter_mut();
                $(let started = qpc();
                for (id, view) in $pack::TABLES.into_iter().zip(self.$pack.tables()) { destination[id.index()] = view; }
                *slots.next().unwrap() = qpc() - started;)+
                costs
            }
            pub fn outcomes(&self) -> [Outcome; TABLE_COUNT] {
                let mut output = [MISSING; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.outcomes(), &mut output);)+
                output
            }
            pub fn checked_mismatches(&self) -> [usize; TABLE_COUNT] {
                let mut output = [0; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.checked_mismatches(), &mut output);)+
                for table in VARIANT_TABLES {
                    output[table.index()] = output[table.index()].saturating_add(self.registry.rows.mismatches);
                }
                output
            }
        }
        /// Off-thread metadata. Sum members, including physically shared work
        /// charged to a pack's anchor, rather than counting that work twice.
        pub fn read_groups() -> Vec<ReadGroup> {
            let mut groups = Vec::new();
            $(for &(name, anchor, members) in $pack::READ_GROUPS {
                // This graph also consumes the shared division census, so
                // report it once in the combined source group below.
                if $pack::TABLES[anchor] == TableId::ArmyUpgradeGroups { continue; }
                groups.push((name, $pack::TABLES[anchor], members.iter().map(|&index| $pack::TABLES[index]).collect()));
            })+
            groups.push(("divisions, equipment and upgrades", TableId::Divisions,
                std::iter::once(TableId::Divisions).chain(DIVISION_SOURCE_TABLES).collect()));
            groups
        }
    };
}
packs!(
    air,
    characters,
    construction,
    economy,
    fog,
    governance,
    intel,
    lexical,
    naval_history,
    production,
    projects,
    session,
    weather
);

impl Captured {
    /// Decode-worker handoff for the research pilot family (copy/decode split).
    /// Sole cross-thread entry to the governance research buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff. No other extended state
    /// crosses threads here.
    pub(crate) fn research_pilot(&mut self) -> &mut governance::Captured {
        &mut self.governance
    }

    /// Decode-worker handoff for the relations family (copy/decode split).
    /// Sole cross-thread entry to the intel relations buffers: the game thread
    /// stages raw-copy state through it on refresh hours and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn relations_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

    pub fn source_checks(needed: &[bool]) -> SourceChecks {
        let air = local(&air::TABLES, needed);
        let naval = local(&naval_history::TABLES, needed);
        SourceChecks {
            ships: false,
            air_wings: air[0] || air[2] || naval_history::needs_wing_sources(&naval),
            order_groups: naval_history::needs_order_sources(&naval),
        }
    }
    pub fn needs_division_check(needed: &[bool]) -> bool {
        DIVISION_SOURCE_TABLES
            .iter()
            .any(|table| needed[table.index()])
    }

    /// Source copies remain inside the enclosing Divisions measurement. The
    /// shared read group includes equipment and upgrade tables without per-row
    /// timers or repeated source censuses for individual tables.
    pub fn visit_division(&mut self, pointer: usize, row: &carmy::Row, second: bool) -> u64 {
        self.divisions.visit(pointer, row, second);
        0
    }
    pub fn finish_divisions(&mut self, outcome: Outcome, second: bool) -> u64 {
        self.divisions.finish(outcome, second);
        0
    }
    pub fn sources<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        needed: &[bool],
    ) -> Sources<'a, M> {
        assert_eq!(needed.len(), TABLE_COUNT);
        let naval = local(&naval_history::TABLES, needed);
        Sources {
            captured: self,
            memory,
            base,
            checks: Self::source_checks(needed),
            air_needed: local(&air::TABLES, needed),
            naval_wing_second: naval_history::needs_wing_sources(&naval),
            naval_order_second: naval_history::needs_order_sources(&naval),
        }
    }
    /// Run after root division acquisition and map.sample. All returned arrays
    /// use global TableId indices. Core slots remain zero.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: &[bool],
        force_full: bool,
    ) -> ([bool; TABLE_COUNT], [u64; TABLE_COUNT]) {
        assert_eq!(needed.len(), TABLE_COUNT);
        let mut matched = [false; TABLE_COUNT];
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! ordinary {
            ($($pack:ident),+ $(,)?) => {$(
                let (pass, cost) = self.$pack.capture(memory, base, game_state, &local(&$pack::TABLES, needed));
                scatter(&$pack::TABLES, pass, &mut matched);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                let (pass, cost) = self.$pack.capture(memory, base, game_state, &local(&$pack::TABLES, needed), current_hour, force_full);
                scatter(&$pack::TABLES, pass, &mut matched);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        ordinary!(economy, fog, session, weather);
        staggered!(characters, construction, governance, intel, projects);
        let air_needed = local(&air::TABLES, needed);
        self.air.finish_wings(
            self.air_roots[0].unwrap_or(MISSING),
            self.air_roots[1],
            &air_needed,
        );
        let (pass, cost) = self.air.capture(memory, base, game_state, &air_needed);
        scatter(&air::TABLES, pass, &mut matched);
        scatter(&air::TABLES, cost, &mut ticks);
        let (pass, cost) = self.naval_history.capture(
            memory,
            base,
            game_state,
            current_hour,
            &local(&naval_history::TABLES, needed),
        );
        scatter(&naval_history::TABLES, pass, &mut matched);
        scatter(&naval_history::TABLES, cost, &mut ticks);

        let production_needed = local(&production::TABLES, needed);
        let (pass, cost) = self.production.acquire(
            memory,
            base,
            game_state,
            &production_needed,
            self.divisions.sources(),
            current_hour,
            force_full,
        );
        scatter(&production::TABLES, pass, &mut matched);
        scatter(&production::TABLES, cost, &mut ticks);
        // The saved campaign registry is authoritative. It includes variants
        // absent from units, current production and stockpiles. Preserve every
        // native entry and duplicate for shared off-thread key rejection.
        let variant_walk =
            replay::cadence::should_walk(TableId::VariantDefinitions, current_hour, force_full);
        if variant_walk {
            let begin = qpc();
            self.registry
                .acquire(memory, base, game_state, variant_needed(needed));
            ticks[TableId::VariantDefinitions.index()] += elapsed(begin);
        } else {
            self.registry.mark_skipped();
        }
        let (pass, cost) = self.production.finish_variants(
            memory,
            base,
            &production_needed,
            self.registry.sources(),
            current_hour,
            force_full,
        );
        for (index, table) in production::TABLES.iter().enumerate() {
            matched[table.index()] |= pass[index];
            ticks[table.index()] = ticks[table.index()].saturating_add(cost[index]);
        }
        staggered!(lexical);
        for table in TABLES {
            ticks[table.index()] =
                ticks[table.index()].saturating_add(self.overhead[table.index()]);
            matched[table.index()] &= needed[table.index()]
                && replay::cadence::should_walk(table, current_hour, force_full);
        }
        (matched, ticks)
    }
}

pub struct Sources<'a, M: Memory> {
    captured: &'a mut Captured,
    memory: &'a M,
    base: usize,
    checks: SourceChecks,
    air_needed: [bool; air::N],
    naval_wing_second: bool,
    naval_order_second: bool,
}
impl<M: Memory> Sources<'_, M> {
    fn wing_ticks(&self) -> u64 {
        self.captured.air.wing_ticks().into_iter().sum::<u64>()
            + self.captured.naval_history.source_ticks()[1]
    }
    fn attribute_overhead(&mut self, table: TableId, begin: i64, separately_measured: u64) -> u64 {
        let total = elapsed(begin);
        self.captured.overhead[table.index()] = self.captured.overhead[table.index()]
            .saturating_add(total.saturating_sub(separately_measured));
        total
    }
}
impl<M: Memory> MapSources for Sources<'_, M> {
    fn checks(&self) -> SourceChecks {
        self.checks
    }
    fn begin_ships(&mut self, _: bool) -> u64 {
        0
    }
    fn ship(&mut self, _: bool, _: usize, _: &cship::Row) -> u64 {
        0
    }
    fn finish_ships(&mut self, _: bool, _: Outcome) -> u64 {
        0
    }
    fn begin_air(&mut self, second: bool) -> u64 {
        let begin = qpc();
        if !second {
            self.captured.air.begin_wings();
        }
        self.attribute_overhead(TableId::WingEquipment, begin, 0)
    }
    fn air(&mut self, second: bool, pointer: usize, _: usize, row: &cair_wing::Row) -> u64 {
        let begin = qpc();
        let before = self.wing_ticks();
        self.captured.air.visit_wing(
            self.memory,
            self.base,
            air::WingSource {
                pointer,
                row,
                second,
            },
            &self.air_needed,
        );
        if !second || self.naval_wing_second {
            self.captured
                .naval_history
                .visit_wing(self.memory, self.base, pointer, row, second);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn finish_air(&mut self, second: bool, outcome: Outcome) -> u64 {
        let begin = qpc();
        let before = self.wing_ticks();
        self.captured.air_roots[usize::from(second)] = Some(outcome);
        if !second || self.naval_wing_second {
            self.captured.naval_history.finish_wings(outcome, second);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn order_groups(
        &mut self,
        second: bool,
        roots: &[corders_group::Root],
        outcome: Outcome,
    ) -> u64 {
        if second && !self.naval_order_second {
            return 0;
        }
        let begin = qpc();
        let ticks = self.captured.naval_history.source_ticks();
        let before = ticks[0] + ticks[2];
        self.captured.naval_history.set_order_sources(
            self.memory,
            self.base,
            roots,
            outcome,
            second,
        );
        let ticks = self.captured.naval_history.source_ticks();
        let charged = (ticks[0] + ticks[2]).saturating_sub(before);
        self.attribute_overhead(TableId::LandTheatres, begin, charged)
    }
}

fn local<T: Copy, const N: usize>(tables: &[TableId; N], global: &[T]) -> [T; N] {
    tables.map(|table| global[table.index()])
}
fn scatter<T: Copy, const N: usize>(
    tables: &[TableId; N],
    local: [T; N],
    global: &mut [T; TABLE_COUNT],
) {
    for (table, value) in tables.iter().zip(local) {
        global[table.index()] = value;
    }
}
fn variant_successful(successful: &[u16]) -> u16 {
    VARIANT_TABLES
        .iter()
        .map(|table| successful[table.index()])
        .min()
        .unwrap()
}
fn division_successful(successful: &[u16]) -> u16 {
    DIVISION_SOURCE_TABLES
        .iter()
        .map(|table| successful[table.index()])
        .min()
        .unwrap()
}
fn variant_needed(needed: &[bool]) -> bool {
    VARIANT_TABLES.iter().any(|table| needed[table.index()])
}
fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

struct Divisions {
    first: Vec<DivisionSource>,
    second: Vec<DivisionSource>,
    seen: [usize; 2],
    outcomes: [Outcome; 2],
    required: usize,
}
impl Divisions {
    fn new(capacity: usize, successful: u16) -> Self {
        Self {
            first: vec![Default::default(); capacity],
            second: comparison_rows(capacity, successful),
            seen: [0; 2],
            outcomes: [MISSING; 2],
            required: 0,
        }
    }
    fn reset(&mut self) {
        self.seen = [0; 2];
        self.outcomes = [MISSING; 2];
        self.required = 0;
    }
    fn visit(&mut self, pointer: usize, row: &carmy::Row, second: bool) {
        let pass = usize::from(second);
        let destination = if second {
            &mut self.second
        } else {
            &mut self.first
        };
        if let Some(slot) = destination.get_mut(self.seen[pass]) {
            *slot = DivisionSource {
                pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            };
        }
        self.seen[pass] = self.seen[pass].saturating_add(1);
        self.required = self.required.max(self.seen[pass]);
    }
    fn finish(&mut self, outcome: Outcome, second: bool) {
        let pass = usize::from(second);
        let capacity = if second {
            self.second.len()
        } else {
            self.first.len()
        };
        self.outcomes[pass] = match outcome {
            Outcome::Rows { len } if len != self.seen[pass] => Outcome::Unavailable {
                reason: "division visitor and source row count disagree",
            },
            Outcome::Rows { len } if len > capacity => truncated_source(capacity, len - capacity),
            Outcome::Truncated { len, unread_rows } => {
                self.required = self.required.max(len.saturating_add(unread_rows as usize));
                let stored = len.min(capacity);
                truncated_source(
                    stored,
                    len.saturating_sub(stored)
                        .saturating_add(unread_rows as usize),
                )
            }
            _ => outcome,
        };
    }
    fn sources(&self) -> production::Sources<'_, DivisionSource> {
        production::Sources {
            first: &self.first,
            first_outcome: self.outcomes[0],
            second: &self.second,
            second_outcome: self.outcomes[1],
        }
    }
    fn grow(&mut self, capacity: &mut usize, successful: u16) {
        grow_capacity(capacity, self.required);
        grow_vec(&mut self.first, *capacity);
        comparison(&mut self.second, *capacity, successful);
    }
    fn allocated_bytes(&self) -> usize {
        (self.first.capacity() + self.second.capacity()) * size_of::<DivisionSource>()
    }
}
fn truncated_source(len: usize, omitted: usize) -> Outcome {
    match u32::try_from(omitted) {
        Ok(unread_rows) if unread_rows > 0 => Outcome::Truncated { len, unread_rows },
        _ => Outcome::Unavailable {
            reason: "source omitted row count invalid or exceeds u32",
        },
    }
}

struct Registry {
    rows: CaptureRows<cequipment_variant::RegistryRow>,
    outcomes: [Outcome; 2],
}
impl Registry {
    fn new(capacity: usize, successful: u16) -> Self {
        Self {
            rows: CaptureRows::new(capacity, successful),
            outcomes: [MISSING; 2],
        }
    }
    fn reset(&mut self) {
        self.rows.reset();
        self.outcomes = [MISSING; 2];
    }
    fn acquire(&mut self, memory: &impl Memory, base: usize, game_state: usize, needed: bool) {
        self.rows.outcome = bounded_registry(
            cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.rows),
            self.rows.rows.len(),
        );
        self.outcomes[0] = self.rows.outcome;
        if needed {
            self.rows.checked = true;
            let second = bounded_registry(
                cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.second),
                self.rows.second.len(),
            );
            self.outcomes[1] = second;
            self.rows.compare(second);
            if self.rows.mismatches != 0 {
                self.outcomes = [Outcome::Unavailable {
                    reason: "variant registry changed during double read",
                }; 2];
            }
        }
    }
    // Off-hours carry-forward hint keeps byte-identical spool output.
    fn mark_skipped(&mut self) {
        self.rows.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        self.outcomes = [self.rows.outcome; 2];
    }
    fn sources(&self) -> production::Sources<'_, cequipment_variant::RegistryRow> {
        production::Sources {
            first: &self.rows.rows,
            first_outcome: self.outcomes[0],
            second: &self.rows.second,
            second_outcome: self.outcomes[1],
        }
    }
    fn grow(&mut self, capacity: &mut usize, successful: u16) {
        self.rows.grow(capacity, successful);
    }
    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
}
fn bounded_registry(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "variant registry count exceeds buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "variant registry truncation has no omitted rows",
        }
    } else {
        outcome
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn metadata_partitions_every_extended_table_once_and_preserves_cost_membership() {
        assert_eq!(N, 128);
        let actual: BTreeSet<_> = TABLES.into_iter().collect();
        assert_eq!(actual.len(), N);
        let expected: BTreeSet<_> = TableId::ALL
            .into_iter()
            .filter(|table| table.index() >= 20)
            .collect();
        assert_eq!(actual, expected);
        for (name, anchor, members) in read_groups() {
            assert!(!name.is_empty());
            assert!(members.contains(&anchor));
            if anchor == TableId::Divisions {
                assert_eq!(
                    members,
                    std::iter::once(TableId::Divisions)
                        .chain(DIVISION_SOURCE_TABLES)
                        .collect::<Vec<_>>()
                );
            } else {
                assert!(members.iter().all(|table| actual.contains(table)));
            }
        }
        let groups = read_groups();
        for table in DIVISION_SOURCE_TABLES {
            assert_eq!(
                groups
                    .iter()
                    .filter(|(_, _, members)| members.contains(&table))
                    .count(),
                1,
                "shared division cost counted more than once for {table:?}"
            );
        }
    }
    #[test]
    fn only_genuine_map_dependencies_retain_sources_and_registry_has_its_own_window() {
        let mut successful = [CONSISTENCY_TARGET; TABLE_COUNT];
        successful[TableId::VariantStats.index()] -= 1;
        let needed = successful.map(|count| count < CONSISTENCY_TARGET);
        assert!(!Captured::needs_division_check(&needed));
        assert_eq!(Captured::source_checks(&needed), SourceChecks::default());
        assert_eq!(variant_successful(&successful), CONSISTENCY_TARGET - 1);
        let mut registry = Registry::new(2, variant_successful(&successful));
        assert_eq!(registry.rows.second.len(), 2);
        assert_eq!(
            registry.allocated_bytes(),
            4 * size_of::<cequipment_variant::RegistryRow>()
        );
        let mut capacity = 2;
        registry.grow(&mut capacity, CONSISTENCY_TARGET);
        assert!(registry.rows.second.is_empty());
        assert_eq!(
            registry.allocated_bytes(),
            2 * size_of::<cequipment_variant::RegistryRow>()
        );
        successful[TableId::AirTheatreMembers.index()] = 0;
        assert!(
            Captured::source_checks(&successful.map(|count| count < CONSISTENCY_TARGET)).air_wings
        );
        successful[TableId::DivisionEquipment.index()] = 0;
        assert!(Captured::needs_division_check(
            &successful.map(|count| count < CONSISTENCY_TARGET)
        ));
    }
    #[test]
    fn division_source_comparison_waits_for_every_equipment_and_upgrade_table() {
        for pending in DIVISION_SOURCE_TABLES {
            let mut successful = [CONSISTENCY_TARGET; TABLE_COUNT];
            successful[pending.index()] -= 1;
            let needed = successful.map(|count| count < CONSISTENCY_TARGET);
            assert!(Captured::needs_division_check(&needed), "{pending:?}");
            assert_eq!(division_successful(&successful), CONSISTENCY_TARGET - 1);
            let mut source = Divisions::new(2, division_successful(&successful));
            let mut capacity = 2;
            source.grow(&mut capacity, division_successful(&successful));
            assert_eq!(source.second.len(), 2, "{pending:?}");

            successful[pending.index()] += 1;
            assert!(!Captured::needs_division_check(
                &successful.map(|count| count < CONSISTENCY_TARGET)
            ));
            source.grow(&mut capacity, division_successful(&successful));
            assert!(source.second.is_empty(), "{pending:?}");
            assert_eq!(source.first.len(), 2);
        }
    }
    #[test]
    fn division_sources_preserve_roster_owner_and_reject_an_incomplete_parent() {
        let mut sources = Divisions::new(1, 0);
        let row = carmy::Row {
            id: (1_u64 << 48) | 14,
            roster_owner: 17,
            logical_owner: 23,
            template: u64::MAX,
            ..Default::default()
        };
        sources.visit(0x1234, &row, false);
        sources.finish(Outcome::Rows { len: 1 }, false);
        assert_eq!(sources.outcomes[0], Outcome::Rows { len: 1 });
        assert_eq!(
            sources.first[0],
            DivisionSource {
                pointer: 0x1234,
                division: row.id,
                template: u64::MAX,
                country: 17
            }
        );
        sources.visit(0x5678, &row, false);
        sources.finish(
            Outcome::Truncated {
                len: 2,
                unread_rows: 7,
            },
            false,
        );
        assert_eq!(
            sources.outcomes[0],
            Outcome::Truncated {
                len: 1,
                unread_rows: 8
            }
        );
        assert_eq!(sources.required, 9);
        sources.reset();
        sources.finish(Outcome::Rows { len: 1 }, false);
        assert!(matches!(sources.outcomes[0], Outcome::Unavailable { .. }));
    }
    #[test]
    fn registry_false_counts_are_rejected_and_incomplete_censuses_remain_incomplete() {
        assert!(matches!(
            bounded_registry(Outcome::Rows { len: 3 }, 2),
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            bounded_registry(
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 0
                },
                2
            ),
            Outcome::Unavailable { .. }
        ));
        let empty = bounded_registry(Outcome::Rows { len: 0 }, 0);
        assert_eq!(empty, Outcome::Rows { len: 0 });
        let truncated = Outcome::Truncated {
            len: 1,
            unread_rows: 9,
        };
        assert_eq!(bounded_registry(truncated, 1), truncated);
    }

    #[test]
    fn research_pilot_handoff_reaches_the_governance_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.research_pilot().raw_mode = true;
        assert!(captured.governance.raw_mode);
        captured.research_pilot().raw_mode = false;
        assert!(!captured.governance.raw_mode);
    }

    #[test]
    fn relations_pilot_handoff_reaches_the_intel_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.relations_pilot().relations_raw_mode = true;
        assert!(captured.intel.relations_raw_mode);
        captured.relations_pilot().relations_raw_mode = false;
        assert!(!captured.intel.relations_raw_mode);
    }
}
