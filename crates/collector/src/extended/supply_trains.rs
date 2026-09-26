//! Hourly supply train operands, with independent table capacities and costs.
use crate::{
    projection::{Projection, project_table},
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccountry_supply_system, supply_node, supply_train_entry, walk::Outcome};
use replay::{
    frame::TableInput,
    rows::{
        ByteRun, SparseBytes, SupplyNodeRow, TableId, TrainAllocation, TrainInventoryRow, tables,
    },
};

#[cfg(test)]
use replay::frame::TableView;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::SupplyNodes, TableId::TrainInventory];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
/// Country-slot shares, each its own pool job. Keeping the census together
/// lets countries reuse current-hour route and endpoint observations instead
/// of acquiring the same global edges in separate workers.
pub const PARTS: usize = 1;
const _: () = assert!(supply_node::VERTEX == SupplyNodeRow::VERTEX);

/// Inputs for one exclusively owned hourly acquisition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
}

/// Acquisition ticks in TABLES order. Interrupted work has no completed timing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    nodes: usize,
    countries: usize,
    paths: usize,
    entries: usize,
    /// Nonzero controller-access bytes across all countries.
    access: usize,
    /// Maximal nonzero controller-access runs across all countries.
    access_runs: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            nodes: 16384,
            countries: 512,
            paths: 131072,
            entries: 4096,
            access: 262144,
            access_runs: 65536,
        }
    }
}
impl Capacities {
    /// TrainInventory has one row per country supply system.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.countries, rows(slots, 1));
    }
}
/// One share of the node census: its rows and their path arena.
pub struct Part {
    nodes: CaptureRows<supply_node::Row>,
    paths: Arena<u32>,
    /// Session memo of authenticated province pointers.
    provinces: supply_node::ProvinceMemo,
}
impl Part {
    fn new(c: &Capacities) -> Self {
        Self {
            nodes: CaptureRows::new(c.nodes),
            paths: Arena::new(c.paths),
            provinces: supply_node::ProvinceMemo::with_scratch(),
        }
    }
    pub(crate) fn vacant() -> Self {
        Self {
            nodes: CaptureRows::new(0),
            paths: Arena::new(0),
            provinces: supply_node::ProvinceMemo::default(),
        }
    }
    fn reset(&mut self) {
        self.nodes.reset();
        self.paths.reset();
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.nodes.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.provinces.allocated_bytes()
    }
    fn grow(&mut self, c: &mut Capacities) {
        self.nodes.grow(&mut c.nodes);
        self.paths.grow(&mut c.paths);
    }
    /// Walk share `part` of the country slots.
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        part: usize,
    ) -> u64 {
        let _reads = crate::read_accounting::table(TableId::SupplyNodes);
        let begin = qpc();
        self.reset();
        self.provinces.begin(memory, context.base, context.game_state);
        let nodes = supply_node::walk_part(
            memory,
            context.base,
            context.game_state,
            part,
            PARTS,
            &mut self.nodes.rows,
            &mut self.paths.first,
            &mut self.provinces,
        );
        self.nodes.outcome = nodes.outcome;
        self.paths.len = nodes.paths;
        self.paths.required = nodes.required_paths;
        qpc().saturating_sub(begin).max(0) as u64
    }
}

/// The train inventory: one walk, loaned with part 0.
pub struct Trains {
    countries: CaptureRows<ccountry_supply_system::TrainRow>,
    entries: Arena<supply_train_entry::Row>,
    /// Nonzero controller-access bytes, run after run.
    access: Arena<u8>,
    /// `[native start, length]` of each nonzero access run.
    access_runs: Arena<[u32; 2]>,
}
impl Trains {
    fn new(c: &Capacities) -> Self {
        Self {
            countries: CaptureRows::new(c.countries),
            entries: Arena::new(c.entries),
            access: Arena::new(c.access),
            access_runs: Arena::new(c.access_runs),
        }
    }
    pub(crate) fn vacant() -> Self {
        Self {
            countries: CaptureRows::new(0),
            entries: Arena::new(0),
            access: Arena::new(0),
            access_runs: Arena::new(0),
        }
    }
    fn reset(&mut self) {
        self.countries.reset();
        self.entries.reset();
        self.access.reset();
        self.access_runs.reset();
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.countries.allocated_bytes()
            + self.entries.allocated_bytes()
            + self.access.allocated_bytes()
            + self.access_runs.allocated_bytes()
    }
    fn grow(&mut self, c: &mut Capacities) {
        self.countries.grow(&mut c.countries);
        self.entries.grow(&mut c.entries);
        self.access.grow(&mut c.access);
        self.access_runs.grow(&mut c.access_runs);
    }
    pub(crate) fn capture(&mut self, memory: &impl Memory, context: CaptureContext) -> u64 {
        let _reads = crate::read_accounting::table(TableId::TrainInventory);
        let begin = qpc();
        self.reset();
        let trains = ccountry_supply_system::walk_trains(
            memory,
            context.base,
            context.game_state,
            &mut self.countries.rows,
            &mut self.entries.first,
            &mut self.access.first,
            &mut self.access_runs.first,
        );
        self.countries.outcome = trains.outcome;
        self.entries.len = trains.entries;
        self.entries.required = trains.required_entries;
        self.access.len = trains.access;
        self.access.required = trains.required_access;
        self.access_runs.len = trains.runs;
        self.access_runs.required = trains.required_runs;
        qpc().saturating_sub(begin).max(0) as u64
    }
}

pub struct Captured {
    parts: Vec<Part>,
    trains: Trains,
    terminal_failure: Option<&'static str>,
}
impl Captured {
    #[cfg(test)]
    fn vacant() -> Self {
        Self {
            parts: (0..PARTS).map(|_| Part::vacant()).collect(),
            trains: Trains::vacant(),
            terminal_failure: None,
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            parts: (0..PARTS).map(|_| Part::new(c)).collect(),
            trains: Trains::new(c),
            terminal_failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.terminal_failure = None;
        for part in &mut self.parts {
            part.reset();
        }
        self.trains.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.parts.iter().map(Part::allocated_bytes).sum::<usize>() + self.trains.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        for part in &mut self.parts {
            part.grow(c);
        }
        self.trains.grow(c);
    }

    /// Whole-family capture on the calling thread, for a callback that never
    /// dispatched the parts.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        CaptureReport {
            ticks: self.capture(memory, context.base, context.game_state),
        }
    }

    /// Complete an accepted failed request without reading or retrying native
    /// state. Keep observed growth demand and allocations behind a publication
    /// failure until recycling; partial rows must never reach table conversion.
    /// The caller stores this terminal report even though its ticks are unmeasured.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        CaptureReport::default()
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let context = CaptureContext {
            base,
            game_state: state,
        };
        self.terminal_failure = None;
        let mut ticks = [0; N];
        for (part, share) in self.parts.iter_mut().enumerate() {
            ticks[0] += share.capture(memory, context, part);
        }
        ticks[1] = self.trains.capture(memory, context);
        ticks
    }
    pub(crate) fn take_part(&mut self, part: usize) -> Part {
        std::mem::replace(&mut self.parts[part], Part::vacant())
    }
    pub(crate) fn restore_part(&mut self, part: usize, share: Part) {
        self.parts[part] = share;
    }
    pub(crate) fn take_trains(&mut self) -> Trains {
        std::mem::replace(&mut self.trains, Trains::vacant())
    }
    pub(crate) fn restore_trains(&mut self, trains: Trains) {
        self.trains = trains;
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        if let Some(reason) = self.terminal_failure {
            return [Outcome::Unavailable { reason }; N];
        }
        let (mut len, mut unread) = (0_usize, 0_u32);
        for part in &self.parts {
            match part.nodes.outcome {
                Outcome::Unavailable { reason } => {
                    return [Outcome::Unavailable { reason }, self.trains.countries.outcome];
                }
                Outcome::Rows { len: rows } => len += rows,
                Outcome::Truncated {
                    len: rows,
                    unread_rows,
                } => {
                    len += rows;
                    unread = unread.saturating_add(unread_rows);
                }
            }
        }
        let nodes = if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        };
        [nodes, self.trains.countries.outcome]
    }

    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        let outcomes = self.outcomes();
        [
            self.node_input(projection.as_deref_mut(), outcomes[0]),
            self.train_input(projection, outcomes[1]),
        ]
    }

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }

    fn node_input(&self, projection: Option<&mut Projection<'_>>, outcome: Outcome) -> TableInput {
        if let Outcome::Unavailable { reason } = outcome {
            return unavailable(reason).into();
        }
        if self.parts.iter().any(|part| part.nodes.outcome.len() > part.nodes.rows.len()) {
            return unavailable("walker row count exceeds allocated buffer").into();
        }
        // Parent provinces can belong to another country's companion. Preserve
        // those authenticated edges without imposing same-country closure.
        // Order only row references; the projection retains a permutation and
        // validates it against the current full keys on every refresh.
        let mut rows = Vec::with_capacity(outcome.len());
        for part in &self.parts {
            let Some(source) = part.nodes.rows.get(..part.nodes.outcome.len()) else {
                return unavailable("walker row count exceeds allocated buffer").into();
            };
            let Some(paths) = part.paths.first.get(..part.paths.len) else {
                return unavailable("supply paths exceed captured arena").into();
            };
            for row in source {
                let Some(path) = row.route.start.checked_add(row.route.len)
                    .and_then(|end| paths.get(row.route.start..end)) else {
                    return unavailable("supply route exceeds captured path arena").into();
                };
                rows.push((row, path));
            }
        }
        project_table::<tables::SupplyNodes, _>(
            projection, outcome, &rows,
            |(r, _)| (r.country as u128) << 32 | r.index as u128,
            |(r, path), previous| {
                r.country == previous.country && r.index == previous.index
                    && r.province == previous.province && r.kind == previous.kind
                    && r.flag == previous.flag && r.available == previous.available
                    && r.consumed == previous.consumed && r.parent == previous.parent
                    && r.route.from == previous.route_from && r.route.to == previous.route_to
                    && r.route.break_after == previous.break_after
                    && *path == previous.path
            },
            |(r, path)| SupplyNodeRow {
                country: r.country,
                index: r.index,
                province: r.province,
                kind: r.kind,
                flag: r.flag,
                available: r.available,
                consumed: r.consumed,
                parent: r.parent,
                route_from: r.route.from,
                route_to: r.route.to,
                break_after: r.route.break_after,
                path: path.to_vec(),
            },
        )
    }

    fn train_input(&self, projection: Option<&mut Projection<'_>>, outcome: Outcome) -> TableInput {
        if let Outcome::Unavailable { reason } = outcome {
            return unavailable(reason).into();
        }
        let trains = &self.trains;
        let Some(rows) = trains.countries.rows.get(..outcome.len()) else {
            return unavailable("walker row count exceeds allocated buffer").into();
        };
        let (Some(entries), Some(access), Some(spans)) = (
            trains.entries.first.get(..trains.entries.len),
            trains.access.first.get(..trains.access.len),
            trains.access_runs.first.get(..trains.access_runs.len),
        ) else {
            return unavailable("train payload exceeds captured arena").into();
        };
        // Each row's runs must lie in the run arena and cover exactly its
        // values, which must lie in the value arena.
        let access_of = |r: &ccountry_supply_system::TrainRow| {
            let runs = spans.get(r.runs_start..r.runs_start.checked_add(r.runs_len)?)?;
            let values = access.get(r.values_start..r.values_start.checked_add(r.values_len)?)?;
            let mut end = 0_u64;
            let canonical = runs.iter().enumerate().all(|(index, [start, len])| {
                let fits = *len != 0
                    && (index == 0 || u64::from(*start) > end)
                    && u64::from(*start) + u64::from(*len) <= u64::from(r.access_count);
                end = u64::from(*start) + u64::from(*len);
                fits
            });
            (canonical && runs.iter().map(|[_, len]| *len as usize).sum::<usize>() == values.len())
                .then_some((runs, values))
        };
        if rows.iter().any(|r| {
            r.entries_start.checked_add(r.entries_len).is_none_or(|end| end > entries.len())
                || access_of(r).is_none()
        }) {
            return unavailable("train row exceeds captured payload arena").into();
        }
        let same_access = |r: &ccountry_supply_system::TrainRow, previous: &SparseBytes| {
            let (runs, mut values) = access_of(r).expect("checked above");
            r.access_count == previous.len
                && runs.len() == previous.runs.len()
                && runs.iter().zip(&previous.runs).all(|([start, len], run)| {
                    let (head, rest) = values.split_at(*len as usize);
                    values = rest;
                    *start == run.start && head == run.bytes.as_slice()
                })
        };
        let sparse_access = |r: &ccountry_supply_system::TrainRow| {
            let (runs, mut values) = access_of(r).expect("checked above");
            SparseBytes {
                len: r.access_count,
                runs: runs
                    .iter()
                    .map(|[start, len]| {
                        let (head, rest) = values.split_at(*len as usize);
                        values = rest;
                        ByteRun {
                            start: *start,
                            bytes: head.to_vec(),
                        }
                    })
                    .collect(),
            }
        };
        project_table::<tables::TrainInventory, _>(
            projection, outcome, rows,
            |r| r.country as u128,
            |r, previous| {
                r.country == previous.country && r.node_count == previous.node_count
                    && r.entries_len == previous.allocations.len()
                    && entries[r.entries_start..r.entries_start + r.entries_len].iter()
                        .zip(&previous.allocations)
                        .all(|(entry, prior)| entry.variant == prior.variant && entry.count == prior.count)
                    && same_access(r, &previous.controller_access)
            },
            |r| {
                TrainInventoryRow {
                    country: r.country,
                    node_count: r.node_count,
                    allocations: entries
                        [r.entries_start..r.entries_start + r.entries_len]
                        .iter()
                        .map(|e| TrainAllocation {
                            variant: e.variant,
                            count: e.count,
                        })
                        .collect(),
                    controller_access: sparse_access(r),
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn projection_fixture() -> Captured {
        let capacities = Capacities {
            nodes: 2, countries: 1, paths: 4, entries: 3, access: 3, access_runs: 2,
        };
        let mut capture = Captured::with_capacities(&capacities);
        // Exercise merging and reordering independently of production's
        // worker count, including duplicate keys across distinct parts.
        capture.parts = (0..4).map(|_| Part::new(&capacities)).collect();
        for (part, country) in capture.parts.iter_mut().zip([3, 1, 4, 2]) {
            part.nodes.outcome = Outcome::Rows { len: 1 };
            part.nodes.rows[0] = supply_node::Row {
                country, index: 0, province: country + 10, kind: u32::MAX,
                flag: 255, available: i64::MIN, consumed: i64::MAX, parent: -1,
                route: engine::supply_route::Row {
                    from: 1, to: u16::MAX, break_after: 255,
                    start: 0, len: 3,
                },
            };
            part.paths.first[..3].copy_from_slice(&[country, 0, country]);
            part.paths.len = 3;
            part.paths.required = 3;
            part.nodes.rows[1].route.start = usize::MAX;
        }
        capture.trains.countries.outcome = Outcome::Rows { len: 1 };
        capture.trains.countries.rows[0] = ccountry_supply_system::TrainRow {
            country: 99, node_count: u32::MAX, entries_start: 0, entries_len: 3,
            access_count: 3, runs_start: 0, runs_len: 1, values_start: 0, values_len: 2,
        };
        capture.trains.entries.first.copy_from_slice(&[
            supply_train_entry::Row { variant: 0, count: i32::MIN },
            supply_train_entry::Row { variant: u64::MAX, count: i32::MAX },
            supply_train_entry::Row { variant: 0, count: 0 },
        ]);
        capture.trains.entries.len = 3;
        // The native array [0, 255, 128]: one run of two nonzero bytes at 1.
        capture.trains.access.first[..2].copy_from_slice(&[255, 128]);
        capture.trains.access.len = 2;
        capture.trains.access_runs.first[0] = [1, 2];
        capture.trains.access_runs.len = 1;
        capture
    }

    #[test]
    fn supply_projection_keeps_every_field_order_fault_and_encoder_boundary() {
        use crate::projection::ProjectionScratch;
        use replay::frame::{FrameEncoder, FrameInput, Metrics};

        let frame = |branch, hour, inputs: [TableInput; N]| {
            let mut frame = FrameInput {
                branch, hour, metrics: Metrics::default(),
                tables: std::array::from_fn(|_| unavailable("fixture table").into()),
            };
            for (table, input) in TABLES.into_iter().zip(inputs) {
                frame.tables[table.index()] = input;
            }
            frame
        };
        let baseline = frame(1, 100, projection_fixture().table_inputs(None));
        for change in 0..38 {
            let mut current = projection_fixture();
            match change {
                0 => {},
                1 => current.parts.swap(0, 1),
                2 => {
                    let part = &mut current.parts[0];
                    part.paths.first.copy_within(0..3, 1);
                    part.paths.len = 4;
                    part.nodes.rows[0].route.start = 1;
                },
                3 => current.parts[0].nodes.rows[0].country ^= 16,
                4 => current.parts[0].nodes.rows[0].index ^= 1,
                5 => current.parts[0].nodes.rows[0].province ^= 1,
                6 => current.parts[0].nodes.rows[0].kind ^= 1,
                7 => current.parts[0].nodes.rows[0].flag ^= 128,
                8 => current.parts[0].nodes.rows[0].available = i64::MAX,
                9 => current.parts[0].nodes.rows[0].consumed = i64::MIN,
                10 => current.parts[0].nodes.rows[0].parent = i32::MIN,
                11 => current.parts[0].nodes.rows[0].route.from ^= 1,
                12 => current.parts[0].nodes.rows[0].route.to ^= 1,
                13 => current.parts[0].nodes.rows[0].route.break_after ^= 128,
                14 => current.parts[0].nodes.rows[0].route.break_after ^= 1,
                15 => current.parts[0].nodes.rows[0].route.len -= 1,
                16 => current.parts[0].paths.first.swap(0, 1),
                17 => current.parts[0].paths.first[2] ^= 1,
                18 => current.trains.countries.rows[0].country ^= 1,
                19 => current.trains.countries.rows[0].node_count ^= 1,
                20 => current.trains.countries.rows[0].entries_len -= 1,
                21 => current.trains.entries.first[0].variant = u64::MAX,
                22 => current.trains.entries.first[0].count = i32::MAX,
                23 => current.trains.countries.rows[0].access_count += 1,
                24 => current.trains.access.first[1] ^= 64,
                25 => current.trains.entries.first.swap(0, 1),
                26 => current.parts[0].nodes.outcome = Outcome::Unavailable { reason: "node fault" },
                27 => current.trains.countries.outcome = Outcome::Truncated { len: 1, unread_rows: 1 },
                28 => current.parts[0].nodes.outcome = Outcome::Truncated { len: 1, unread_rows: 7 },
                29 => current.parts[0].nodes.rows[0].route.start = usize::MAX,
                30 => current.trains.countries.rows[0].entries_start = usize::MAX,
                31 => current.parts[1].nodes.rows[0].country = current.parts[0].nodes.rows[0].country,
                32 => current.parts[0].nodes.outcome = Outcome::Rows { len: 3 },
                33 => current.parts[0].paths.len = 2,
                34 => current.trains.entries.len = 2,
                35..=37 => {},
                _ => unreachable!(),
            }
            let branch = if change == 35 { 2 } else { 1 };
            let hour = if change == 37 { 105 } else { 101 };
            let force = change == 36;
            let mut projected = FrameEncoder::new();
            let mut reference = FrameEncoder::new();
            projected.encode_input(baseline.clone(), false).unwrap();
            reference.encode_input(baseline.clone(), false).unwrap();
            let mut scratch = ProjectionScratch::default();
            let mut projection = Projection::new(&projected, branch, hour, force, &mut scratch);
            // Prime the ordering scratch before exercising changes to keys or
            // native census order. Its indices must never imply unchanged data.
            projection_fixture().table_inputs(Some(&mut projection));
            let inputs = current.table_inputs(Some(&mut projection));
            if change < 3 {
                assert!(inputs.iter().all(|input| matches!(input, TableInput::ComparedEqual(_))));
            }
            if change == 26 {
                assert!(matches!(inputs[1], TableInput::ComparedEqual(_)));
            }
            if change == 27 {
                assert!(matches!(inputs[0], TableInput::ComparedEqual(_)));
            }
            assert_eq!(
                projected.encode_input(frame(branch, hour, inputs), force).unwrap(),
                reference.encode_input(frame(branch, hour, current.table_inputs(None)), force).unwrap(),
                "change {change}",
            );
        }
    }

    #[test]
    fn immediate_parent_can_belong_to_another_countrys_node_census() {
        let c = Capacities {
            nodes: 2,
            countries: 2,
            paths: 2,
            entries: 0,
            access: 0,
            access_runs: 0,
        };
        let mut capture = Captured::with_capacities(&c);
        capture.parts[0].nodes.rows[0] = supply_node::Row {
            country: 1,
            province: 2,
            parent: 3,
            route: engine::supply_route::Row {
                from: 3,
                to: 2,
                len: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        capture.parts[0].nodes.rows[1] = supply_node::Row {
            country: 2,
            province: 3,
            parent: -1,
            ..Default::default()
        };
        capture.parts[0].nodes.outcome = Outcome::Rows { len: 2 };
        for part in &mut capture.parts[1..] {
            part.nodes.outcome = Outcome::Rows { len: 0 };
        }
        capture.parts[0].paths.first.copy_from_slice(&[3, 2]);
        capture.parts[0].paths.len = 2;
        let TableView::Complete(replay::rows::TableRows::SupplyNodes(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("authenticated cross-country parent must remain recorded");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].country, rows[0].parent), (1, 3));
        assert_eq!(rows[0].path, [3, 2]);
    }

    #[test]
    fn vertex_rows_join_the_node_table_after_their_countrys_nodes() {
        let c = Capacities {
            nodes: 2,
            countries: 0,
            paths: 2,
            entries: 0,
            access: 0,
            access_runs: 0,
        };
        let mut capture = Captured::with_capacities(&c);
        capture.parts[0].nodes.rows[0] = supply_node::Row {
            country: 1,
            index: supply_node::VERTEX | 5,
            province: 5,
            parent: 2,
            route: engine::supply_route::Row {
                from: 2,
                to: 5,
                len: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        capture.parts[0].nodes.rows[1] = supply_node::Row {
            country: 1,
            province: 9,
            parent: 5,
            ..Default::default()
        };
        capture.parts[0].nodes.outcome = Outcome::Rows { len: 2 };
        for part in &mut capture.parts[1..] {
            part.nodes.outcome = Outcome::Rows { len: 0 };
        }
        capture.parts[0].paths.first.copy_from_slice(&[5, 2]);
        capture.parts[0].paths.len = 2;
        let TableView::Complete(replay::rows::TableRows::SupplyNodes(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("vertex rows are ordinary SupplyNodes rows");
        };
        assert_eq!(
            rows.iter()
                .map(|r| (r.index, r.province, r.parent, r.is_vertex()))
                .collect::<Vec<_>>(),
            [(0, 9, 5, false), (SupplyNodeRow::VERTEX | 5, 5, 2, true)]
        );
        assert_eq!(rows[1].path, [5, 2]);
    }

    fn capacities() -> Capacities {
        Capacities {
            nodes: 2,
            countries: 2,
            paths: 2,
            entries: 2,
            access: 2,
            access_runs: 2,
        }
    }
    fn allocations(c: &Captured) -> [(usize, usize, usize); 5] {
        fn vector<T>(v: &Vec<T>) -> (usize, usize, usize) {
            (v.as_ptr() as usize, v.len(), v.capacity())
        }
        [
            vector(&c.parts[0].nodes.rows),
            vector(&c.trains.countries.rows),
            vector(&c.parts[0].paths.first),
            vector(&c.trains.entries.first),
            vector(&c.trains.access.first),
        ]
    }

    #[test]
    fn owned_failure_gates_partial_rows_and_preserves_primary_capacity_demand() {
        let initial = capacities();
        for (nodes, countries, expected_rows) in [
            (
                Outcome::Rows { len: 2 },
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 8,
                },
                (4, 16),
            ),
            (
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 6,
                },
                Outcome::Rows { len: 2 },
                (8, 4),
            ),
        ] {
            let mut capacity = initial;
            let mut expected_capacity = initial;
            let mut frame = Captured::with_capacities(&initial);
            let mut owned = std::mem::replace(&mut frame, Captured::vacant());
            let mut expected = Captured::with_capacities(&initial);
            assert_eq!(frame.allocated_bytes(), 0);
            for capture in [&mut owned, &mut expected] {
                capture.parts[0].nodes.outcome = nodes;
                capture.trains.countries.outcome = countries;
                capture.parts[0].nodes.rows[0].route.start = usize::MAX;
                capture.parts[0].nodes.rows[0].route.len = 1;
                capture.trains.countries.rows[0].entries_start = usize::MAX;
                capture.trains.countries.rows[0].entries_len = 1;
                capture.trains.countries.rows[0].runs_start = usize::MAX;
                capture.trains.countries.rows[0].runs_len = 1;
                capture.parts[0].paths.len = 1;
                capture.parts[0].paths.required = 9;
                capture.trains.entries.len = 1;
                capture.trains.entries.required = 6;
                capture.trains.access.len = 2;
                capture.trains.access.required = 2;
            }
            let before = allocations(&owned);
            let report = owned.fail_owned_capture("owned supply acquisition failed");
            assert_eq!(report.ticks, [0; N]);
            assert_eq!(allocations(&owned), before);
            assert_eq!(
                [owned.parts[0].nodes.outcome, owned.trains.countries.outcome],
                [nodes, countries]
            );
            assert_eq!(
                owned.outcomes(),
                [Outcome::Unavailable {
                    reason: "owned supply acquisition failed"
                }; N]
            );
            assert!(
                owned
                    .tables()
                    .iter()
                    .all(|t| matches!(t, TableView::Unavailable { .. }))
            );
            assert_eq!((owned.parts[0].paths.len, owned.parts[0].paths.required), (1, 9));
            assert_eq!((owned.trains.entries.len, owned.trains.entries.required), (1, 6));
            assert_eq!((owned.trains.access.len, owned.trains.access.required), (2, 2));
            frame = owned;
            frame.grow(&mut capacity);
            expected.grow(&mut expected_capacity);
            assert!(capacity == expected_capacity);
            assert_eq!((capacity.nodes, capacity.countries), expected_rows);
            assert_eq!(
                (capacity.paths, capacity.entries, capacity.access),
                (16, 8, 4)
            );
            assert_eq!(
                allocations(&frame).map(|(_, len, cap)| (len, cap)),
                allocations(&expected).map(|(_, len, cap)| (len, cap))
            );
            let retained = allocations(&frame);
            frame.reset();
            frame.grow(&mut capacity);
            assert!(frame.terminal_failure.is_none());
            assert_eq!(allocations(&frame), retained);
        }
    }

    #[test]
    fn owned_and_direct_capture_read_each_first_fault_once_and_continue_other_table() {
        use std::cell::RefCell;
        #[derive(Default)]
        struct Unreadable(RefCell<Vec<(usize, usize)>>);
        impl Memory for Unreadable {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.borrow_mut().push((address, output.len()));
                false
            }
        }
        let context = CaptureContext {
            base: 0x140000000,
            game_state: 0x1000,
        };
        let mut direct = Captured::with_capacities(&capacities());
        let mut owned = Captured::with_capacities(&capacities());
        let before = allocations(&owned);
        let direct_memory = Unreadable::default();
        let owned_memory = Unreadable::default();
        direct.capture(&direct_memory, context.base, context.game_state);
        for owned_entry in [false, true] {
            owned.fail_owned_capture("previous request failed");
            owned_memory.0.borrow_mut().clear();
            if owned_entry {
                owned.capture_owned(&owned_memory, context);
            } else {
                owned.capture(&owned_memory, context.base, context.game_state);
            }
            assert!(owned.terminal_failure.is_none());
            assert_eq!(*owned_memory.0.borrow(), *direct_memory.0.borrow());
            // Each part keys its province memo (two reads) before its root fails.
            assert_eq!(owned_memory.0.borrow().len(), 3 * PARTS + 1);
            assert_eq!(owned.outcomes(), direct.outcomes());
            assert_eq!(owned.tables(), direct.tables());
            assert!(
                owned
                    .tables()
                    .iter()
                    .all(|t| matches!(t, TableView::Unavailable { .. }))
            );
            assert_eq!((owned.parts[0].paths.len, owned.parts[0].paths.required), (0, 0));
            assert_eq!((owned.trains.entries.len, owned.trains.entries.required), (0, 0));
            assert_eq!((owned.trains.access.len, owned.trains.access.required), (0, 0));
            assert_eq!(allocations(&owned), before);
        }
    }
}
