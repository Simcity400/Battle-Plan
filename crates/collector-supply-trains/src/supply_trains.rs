//! Supply train operands (SupplyNodes, TrainInventory), with independent
//! table capacities and costs. The `supply_trains` sliced family (owner
//! decision 2026-10-11: the trains are decorative and the vehicle operands
//! panel-only): each hour reads the companions of that hour's country slice,
//! of every new country and of every country whose last read was not
//! published, and every country on a full hour; a country's nodes, vertex
//! rows and inventory come from the same read.
use crate::{
    clock::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{collect_vec, copy_vec, owned_vec, reserve};
use engine_native::{
    Memory, ccountry_supply_system, supply_node, supply_train_entry, walk::Buffer, walk::Outcome,
};
use replay_data::{
    rows::ByteRun, rows::Recorded, rows::SparseBytes, rows::SupplyNodeRow, rows::TableId,
    rows::TrainAllocation, rows::TrainInventoryRow, rows::tables,
};
use replay_frame::frame::TableInput;

#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::TableView;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::SupplyNodes, TableId::TrainInventory];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
/// Country-slot shares, each its own pool job. Keeping the census together
/// lets countries reuse current-hour route and endpoint observations instead
/// of acquiring the same global edges in separate workers.
pub const PARTS: usize = 1;
const _: () = assert!(supply_node::VERTEX == SupplyNodeRow::VERTEX);
// The hour's owner selection travels with part 0 and governs every part.
const _: () = assert!(PARTS == 1);

/// Inputs for one exclusively owned hourly acquisition.
#[derive(Clone, Copy, Debug)]
pub struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

/// The hour's owner selection and the family's fingerprint store, loaned
/// with part 0 from dispatch to join.
pub struct Slice {
    pub selection: SliceSelection,
    pub fingerprints: Option<Box<SliceFingerprints>>,
}

/// Scans the country census into this hour's selection
/// ([`crate::slicing::scan_countries`]): a supply system exists for every
/// canonical country (`csupply_system::Reader::at`), so the country census
/// is the family's owner census.
pub fn select_countries(
    memory: &impl Memory,
    context: CaptureContext,
    selection: &mut SliceSelection,
    fingerprints: Option<&mut SliceFingerprints>,
) {
    crate::slicing::scan_countries(
        memory,
        context.base,
        context.game_state,
        context.hour,
        context.force_full,
        selection,
        fingerprints,
    );
}

/// One SupplyTrains job: part `part`'s node share and, with part 0, the
/// train inventory, for the countries of the hour's selection, which part 0
/// scans first. A full hour reads and checks every slot.
pub fn capture_job(
    memory: &impl Memory,
    context: CaptureContext,
    part: usize,
    nodes: &mut Part,
    trains: Option<&mut Trains>,
    slice: Option<&mut Slice>,
) -> CaptureReport {
    let started = qpc();
    let mut whole = SliceSelection::new(false);
    let selection = match slice {
        Some(slice) => {
            let _reads = crate::read_accounting::table(TableId::SupplyNodes);
            select_countries(
                memory,
                context,
                &mut slice.selection,
                slice.fingerprints.as_deref_mut(),
            );
            &slice.selection
        }
        None => {
            whole.reset(context.hour, true);
            &whole
        }
    };
    let scan = qpc().saturating_sub(started).max(0) as u64;
    let contains: &dyn Fn(u32) -> bool = &|country| selection.contains(country);
    let select = (!selection.force_full).then_some(contains);
    let mut ticks = [0; N];
    ticks[0] = scan.saturating_add(nodes.capture(memory, context, part, select));
    if let Some(trains) = trains {
        ticks[1] = trains.capture(memory, context, select);
    }
    CaptureReport { ticks }
}

/// Acquisition ticks in TABLES order. Interrupted work has no completed timing.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureReport {
    pub ticks: [u64; N],
}

/// The supply walk's province checks since attach, for the status writer:
/// allocation-free atomics written by the walk's worker after each read.
/// Ids the game holds as a null province are recorded as stored
/// ([`supply_node::ProvinceCheck::is_null_province`]); every other failed
/// check refuses the hour's SupplyNodes read.
struct WalkObservations {
    by_check: [std::sync::atomic::AtomicU64; supply_node::ProvinceCheck::ALL.len()],
    last: LastObservation,
}

/// The last observation, packed so a reader never mixes two of them:
/// `country << 32 | index` and `province << 32 | role << 8 | check`, the
/// role and check one-based so zero means none yet.
struct LastObservation {
    hour: std::sync::atomic::AtomicI32,
    owner: std::sync::atomic::AtomicU64,
    value: std::sync::atomic::AtomicU64,
}

#[derive(serde::Serialize)]
struct ObservationStatus {
    hour: i32,
    country: u32,
    index: u32,
    vertex: bool,
    province: u32,
    role: &'static str,
    check: &'static str,
}

const ROLES: [supply_node::ProvinceRole; 3] = [
    supply_node::ProvinceRole::Node,
    supply_node::ProvinceRole::Parent,
    supply_node::ProvinceRole::Vertex,
];

impl WalkObservations {
    const fn new() -> Self {
        Self {
            by_check: [const { std::sync::atomic::AtomicU64::new(0) };
                supply_node::ProvinceCheck::ALL.len()],
            last: LastObservation {
                hour: std::sync::atomic::AtomicI32::new(0),
                owner: std::sync::atomic::AtomicU64::new(0),
                value: std::sync::atomic::AtomicU64::new(0),
            },
        }
    }

    fn record(&self, count: u64, hour: i32, observed: supply_node::ProvinceObservation) {
        use std::sync::atomic::Ordering::Relaxed;
        let check = supply_node::ProvinceCheck::ALL
            .iter()
            .position(|check| *check == observed.check)
            .unwrap_or(0);
        let role = ROLES
            .iter()
            .position(|role| *role == observed.role)
            .unwrap_or(0);
        self.by_check[check].fetch_add(count, Relaxed);
        self.last.hour.store(hour, Relaxed);
        self.last.owner.store(
            u64::from(observed.country) << 32 | u64::from(observed.index),
            Relaxed,
        );
        self.last.value.store(
            u64::from(observed.province) << 32 | (role as u64 + 1) << 8 | (check as u64 + 1),
            Relaxed,
        );
    }

    fn total(&self) -> u64 {
        self.by_check
            .iter()
            .map(|count| count.load(std::sync::atomic::Ordering::Relaxed))
            .fold(0, u64::saturating_add)
    }

    fn last(&self) -> Option<ObservationStatus> {
        use std::sync::atomic::Ordering::Relaxed;
        let value = self.last.value.load(Relaxed);
        let role = ((value >> 8) & 0xff) as usize;
        let check = (value & 0xff) as usize;
        if role == 0 || check == 0 {
            return None;
        }
        let owner = self.last.owner.load(Relaxed);
        let index = owner as u32;
        Some(ObservationStatus {
            hour: self.last.hour.load(Relaxed),
            country: (owner >> 32) as u32,
            index: index & !supply_node::VERTEX,
            vertex: index & supply_node::VERTEX != 0,
            province: (value >> 32) as u32,
            role: ROLES.get(role - 1).map_or("unknown", |role| role.name()),
            check: supply_node::ProvinceCheck::ALL
                .get(check - 1)
                .map_or("unknown", |check| check.name()),
        })
    }

    fn checks(&self) -> impl serde::Serialize + '_ {
        crate::status::map(
            supply_node::ProvinceCheck::ALL
                .iter()
                .zip(&self.by_check)
                .map(|(check, count)| {
                    (
                        check.name(),
                        count.load(std::sync::atomic::Ordering::Relaxed),
                    )
                }),
        )
    }
}

static NULL_PROVINCES: WalkObservations = WalkObservations::new();
static PROVINCE_REFUSALS: WalkObservations = WalkObservations::new();

/// Publish one walk's province observations.
fn observe_walk(hour: i32, report: &supply_node::Report) {
    if let Some(observed) = report.last_null_province {
        NULL_PROVINCES.record(u64::from(report.null_provinces), hour, observed);
    }
    if let Some(observed) = report.refusal {
        PROVINCE_REFUSALS.record(1, hour, observed);
    }
}

/// The supply walk's province diagnostics: how many province ids it
/// recorded as the game's null province and how many checks refused a walk
/// (by check), each with the last one's owner country, node index, stored
/// value, role and check.
pub fn node_walk_data() -> impl serde::Serialize {
    crate::status::status!({
        "null_provinces": NULL_PROVINCES.total(),
        "last_null_province": NULL_PROVINCES.last(),
        "province_refusals": PROVINCE_REFUSALS.total(),
        "province_refusals_by_check": PROVINCE_REFUSALS.checks(),
        "last_province_refusal": PROVINCE_REFUSALS.last(),
        "scope": "cumulative supply node walk province checks: ids whose province lookup returns null (zero, outside the census, an empty slot) are recorded as stored; an unreadable slot, a mismatched CProvince or an unreadable census refuses the hour's SupplyNodes read; index is the node index, or the province of a vertex row",
    })
}

#[derive(Default)]
pub struct VehicleWork {
    rows: u64,
    failures: [u64; 3],
}

impl VehicleWork {
    pub fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "rows": self.rows,
            "refused": self.failures.iter().fold(0_u64, |sum, count| sum.saturating_add(*count)),
            "refusals": crate::status::map(ccountry_supply_system::VEHICLE_READ_FAILURES.into_iter().zip(self.failures)),
            "scope": "stored supply-vehicle operands of the countries each hour read (one country slice a day each); a refused scalar is absent while the inventory row remains available",
        })
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
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

impl Capacities {
    /// TrainInventory has one row per country supply system.
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.countries, rows(slots, 1));
    }
}
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let nodes = supply_node::storage_demand(memory, base, state);
        let trains = ccountry_supply_system::train_storage_demand(memory, base, state);
        if let Ok(count) = nodes {
            crate::country_scale::raise(&mut self.nodes, count);
        }
        if let Ok(demand) = trains {
            crate::country_scale::raise(&mut self.countries, demand.countries);
            crate::country_scale::raise(&mut self.entries, demand.entries);
            crate::country_scale::raise(&mut self.access, demand.access);
            crate::country_scale::raise(&mut self.access_runs, demand.access);
        }
        nodes.and(trains).map(|_| ())
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
            provinces: supply_node::ProvinceMemo::with_scratch()
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
        }
    }
    pub fn vacant() -> Self {
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
    pub fn allocated_bytes(&self) -> usize {
        self.nodes.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.provinces.allocated_bytes()
    }
    fn grow(&mut self, c: &mut Capacities) {
        self.nodes.grow(&mut c.nodes);
        self.paths.grow(&mut c.paths);
    }
    /// Walk share `part` of the country slots, of the countries `select`
    /// admits when present.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        part: usize,
        select: Option<&dyn Fn(u32) -> bool>,
    ) -> u64 {
        let _reads = crate::read_accounting::table(TableId::SupplyNodes);
        let begin = qpc();
        self.reset();
        self.provinces
            .begin(memory, context.base, context.game_state);
        let nodes = supply_node::walk_part_buffer(
            memory,
            context.base,
            context.game_state,
            part,
            PARTS,
            Buffer::Growing(&mut self.nodes.rows),
            Buffer::Growing(&mut self.paths.first),
            &mut self.provinces,
            select,
        );
        self.nodes.outcome = nodes.outcome;
        self.paths.len = nodes.paths;
        self.paths.required = nodes.required_paths;
        observe_walk(context.hour, &nodes);
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
    vehicle_failures: [u32; 3],
}
impl Trains {
    fn new(c: &Capacities) -> Self {
        Self {
            countries: CaptureRows::new(c.countries),
            entries: Arena::new(c.entries),
            access: Arena::new(c.access),
            access_runs: Arena::new(c.access_runs),
            vehicle_failures: [0; 3],
        }
    }
    pub fn vacant() -> Self {
        Self {
            countries: CaptureRows::new(0),
            entries: Arena::new(0),
            access: Arena::new(0),
            access_runs: Arena::new(0),
            vehicle_failures: [0; 3],
        }
    }
    fn reset(&mut self) {
        self.countries.reset();
        self.entries.reset();
        self.access.reset();
        self.access_runs.reset();
        self.vehicle_failures = [0; 3];
    }
    pub fn allocated_bytes(&self) -> usize {
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
    /// The inventory of the countries `select` admits when present.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        select: Option<&dyn Fn(u32) -> bool>,
    ) -> u64 {
        let _reads = crate::read_accounting::table(TableId::TrainInventory);
        let begin = qpc();
        self.reset();
        let trains = ccountry_supply_system::walk_trains_buffer(
            memory,
            context.base,
            context.game_state,
            Buffer::Growing(&mut self.countries.rows),
            Buffer::Growing(&mut self.entries.first),
            Buffer::Growing(&mut self.access.first),
            Buffer::Growing(&mut self.access_runs.first),
            select,
        );
        self.countries.outcome = trains.outcome;
        self.entries.len = trains.entries;
        self.entries.required = trains.required_entries;
        self.access.len = trains.access;
        self.access.required = trains.required_access;
        self.access_runs.len = trains.runs;
        self.access_runs.required = trains.required_runs;
        self.vehicle_failures = trains.vehicle_failures;
        qpc().saturating_sub(begin).max(0) as u64
    }
}

pub struct Captured {
    parts: Vec<Part>,
    trains: Trains,
    /// Which countries this hour's rows cover (`slicing::SUPPLY_TRAINS`).
    selection: SliceSelection,
    terminal_failure: Option<&'static str>,
}
impl Captured {
    #[cfg(test)]
    fn vacant() -> Self {
        Self {
            parts: (0..PARTS).map(|_| Part::vacant()).collect(),
            trains: Trains::vacant(),
            selection: SliceSelection::new(false),
            terminal_failure: None,
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            parts: (0..PARTS).map(|_| Part::new(c)).collect(),
            trains: Trains::new(c),
            selection: SliceSelection::new(true),
            terminal_failure: None,
        }
    }

    /// The countries whose rows this frame's read replaced.
    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    /// Start a frame's selection at its hour; the job resets it again.
    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// Loan the selection to part 0's job; [`Self::restore_selection`]
    /// takes it back at the join.
    pub fn take_selection(&mut self) -> SliceSelection {
        std::mem::replace(&mut self.selection, SliceSelection::new(false))
    }

    pub fn restore_selection(&mut self, selection: SliceSelection) {
        self.selection = selection;
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        // Without a fingerprint store every country is read.
        self.selection.reset(context.hour, true);
        CaptureReport {
            ticks: self.capture(memory, context.base, context.game_state),
        }
    }

    /// Complete an accepted failed request without reading or retrying native
    /// state. Keep observed growth demand and allocations behind a publication
    /// failure until recycling; partial rows must never reach table conversion.
    /// The caller stores this terminal report even though its ticks are unmeasured.
    pub fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        CaptureReport::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let context = CaptureContext {
            base,
            game_state: state,
            hour: 0,
            force_full: true,
        };
        self.terminal_failure = None;
        let mut ticks = [0; N];
        for (part, share) in self.parts.iter_mut().enumerate() {
            ticks[0] += share.capture(memory, context, part, None);
        }
        ticks[1] = self.trains.capture(memory, context, None);
        ticks
    }
    pub fn take_part(&mut self, part: usize) -> Part {
        std::mem::replace(&mut self.parts[part], Part::vacant())
    }
    pub fn restore_part(&mut self, part: usize, share: Part) {
        self.parts[part] = share;
    }
    pub fn take_trains(&mut self) -> Trains {
        std::mem::replace(&mut self.trains, Trains::vacant())
    }
    pub fn restore_trains(&mut self, trains: Trains) {
        self.trains = trains;
    }
    pub fn observe_vehicles(&self, work: &mut VehicleWork) {
        if self.terminal_failure.is_some() {
            return;
        }
        let len = match self.trains.countries.outcome {
            Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
            Outcome::Unavailable { .. } => return,
        };
        work.rows = work.rows.saturating_add(len as u64);
        for (total, count) in work.failures.iter_mut().zip(self.trains.vehicle_failures) {
            *total = total.saturating_add(u64::from(count));
        }
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        if let Some(reason) = self.terminal_failure {
            return [Outcome::Unavailable { reason }; N];
        }
        let (mut len, mut unread) = (0_usize, 0_u32);
        for part in &self.parts {
            match part.nodes.outcome {
                Outcome::Unavailable { reason } => {
                    return [
                        Outcome::Unavailable { reason },
                        self.trains.countries.outcome,
                    ];
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

    /// The hour's read rows, converted whole: a sliced read holds only its
    /// owners' rows, which the spool merges into the held table by owner
    /// (`Campaign::merge_sliced_inputs`), so it is never compared with the
    /// encoder's complete baseline.
    pub fn table_inputs(&self) -> [TableInput; N] {
        let outcomes = self.outcomes();
        [self.node_input(outcomes[0]), self.train_input(outcomes[1])]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs().map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("sliced reads are converted whole"),
        })
    }

    fn node_input(&self, outcome: Outcome) -> TableInput {
        if let Outcome::Unavailable { reason } = outcome {
            return unavailable(reason).into();
        }
        if self
            .parts
            .iter()
            .any(|part| part.nodes.outcome.len() > part.nodes.rows.len())
        {
            return unavailable("walker row count exceeds allocated buffer").into();
        }
        // Parent provinces can belong to another country's companion. Preserve
        // those authenticated edges without imposing same-country closure.
        // The conversion sorts by key and refuses repeated engine ids.
        let mut rows = Vec::new();
        if reserve(&mut rows, outcome.len()).is_err() {
            return unavailable("collector allocation failed").into();
        }
        for part in &self.parts {
            let Some(source) = part.nodes.rows.get(..part.nodes.outcome.len()) else {
                return unavailable("walker row count exceeds allocated buffer").into();
            };
            let Some(paths) = part.paths.first.get(..part.paths.len) else {
                return unavailable("supply paths exceed captured arena").into();
            };
            for row in source {
                let Some(path) = row
                    .route
                    .start
                    .checked_add(row.route.len)
                    .and_then(|end| paths.get(row.route.start..end))
                else {
                    return unavailable("supply route exceeds captured path arena").into();
                };
                rows.push((row, path));
            }
        }
        table::<tables::SupplyNodes, _>(outcome, &rows, |(r, path)| SupplyNodeRow {
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
            path: owned_vec(copy_vec(path)),
        })
        .into()
    }

    fn train_input(&self, outcome: Outcome) -> TableInput {
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
            r.entries_start
                .checked_add(r.entries_len)
                .is_none_or(|end| end > entries.len())
                || access_of(r).is_none()
        }) {
            return unavailable("train row exceeds captured payload arena").into();
        }
        let sparse_access = |r: &ccountry_supply_system::TrainRow| {
            let (runs, mut values) = access_of(r).expect("checked above");
            SparseBytes {
                len: r.access_count,
                runs: owned_vec(collect_vec(runs.iter().map(|[start, len]| {
                    let (head, rest) = values.split_at(*len as usize);
                    values = rest;
                    ByteRun {
                        start: *start,
                        bytes: owned_vec(copy_vec(head)),
                    }
                }))),
            }
        };
        table::<tables::TrainInventory, _>(outcome, rows, |r| TrainInventoryRow {
            country: r.country,
            node_count: r.node_count,
            allocations: owned_vec(collect_vec(
                entries[r.entries_start..r.entries_start + r.entries_len]
                    .iter()
                    .map(|e| TrainAllocation {
                        variant: e.variant,
                        count: e.count,
                    }),
            )),
            controller_access: sparse_access(r),
            allocated_trucks: Recorded::Value(r.allocated_trucks),
            required_trucks_raw: Recorded::Value(r.required_trucks_raw),
            required_trains: Recorded::Value(r.required_trains),
        })
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read_fixture() -> Captured {
        let capacities = Capacities {
            nodes: 2,
            countries: 1,
            paths: 4,
            entries: 3,
            access: 3,
            access_runs: 2,
        };
        let mut capture = Captured::with_capacities(&capacities);
        // Exercise merging and reordering independently of production's
        // worker count, including duplicate keys across distinct parts.
        capture.parts = (0..4).map(|_| Part::new(&capacities)).collect();
        for (part, country) in capture.parts.iter_mut().zip([3, 1, 4, 2]) {
            part.nodes.outcome = Outcome::Rows { len: 1 };
            part.nodes.rows[0] = supply_node::Row {
                country,
                index: 0,
                province: country + 10,
                kind: u32::MAX,
                flag: 255,
                available: i64::MIN,
                consumed: i64::MAX,
                parent: -1,
                route: engine_native::supply_route::Row {
                    from: 1,
                    to: u16::MAX,
                    break_after: 255,
                    start: 0,
                    len: 3,
                },
            };
            part.paths.first[..3].copy_from_slice(&[country, 0, country]);
            part.paths.len = 3;
            part.paths.required = 3;
            part.nodes.rows[1].route.start = usize::MAX;
        }
        capture.trains.countries.outcome = Outcome::Rows { len: 1 };
        capture.trains.countries.rows[0] = ccountry_supply_system::TrainRow {
            country: 99,
            node_count: u32::MAX,
            entries_start: 0,
            entries_len: 3,
            access_count: 3,
            runs_start: 0,
            runs_len: 1,
            values_start: 0,
            values_len: 2,
            allocated_trucks: Some(i32::MIN),
            required_trucks_raw: Some(i64::MAX),
            required_trains: Some(i32::MAX),
        };
        capture.trains.entries.first.copy_from_slice(&[
            supply_train_entry::Row {
                variant: 0,
                count: i32::MIN,
            },
            supply_train_entry::Row {
                variant: u64::MAX,
                count: i32::MAX,
            },
            supply_train_entry::Row {
                variant: 0,
                count: 0,
            },
        ]);
        capture.trains.entries.len = 3;
        // The native array [0, 255, 128]: one run of two nonzero bytes at 1.
        capture.trains.access.first[..2].copy_from_slice(&[255, 128]);
        capture.trains.access.len = 2;
        capture.trains.access_runs.first[0] = [1, 2];
        capture.trains.access_runs.len = 1;
        capture
    }

    /// A sliced read converts every field of its owners' rows, in key order,
    /// as a whole sampled table for the spool to merge by owner: it is never
    /// reduced to a comparison with the encoder's complete baseline, which
    /// held every owner's rows when the family was read whole each hour.
    #[test]
    fn sliced_reads_convert_every_field_whole_in_key_order() {
        let capture = read_fixture();
        let [nodes, trains] = capture.table_inputs();
        let TableInput::Sampled(TableView::Complete(replay_data::rows::TableRows::SupplyNodes(
            nodes,
        ))) = nodes
        else {
            panic!("a sliced node read converts whole");
        };
        let TableInput::Sampled(TableView::Complete(replay_data::rows::TableRows::TrainInventory(
            trains,
        ))) = trains
        else {
            panic!("a sliced inventory read converts whole");
        };
        // The parts walked countries 3, 1, 4 and 2; rows leave in key order.
        assert_eq!(
            nodes.iter().map(|row| row.country).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        for row in &nodes {
            assert_eq!(
                row,
                &SupplyNodeRow {
                    country: row.country,
                    index: 0,
                    province: row.country + 10,
                    kind: u32::MAX,
                    flag: 255,
                    available: i64::MIN,
                    consumed: i64::MAX,
                    parent: -1,
                    route_from: 1,
                    route_to: u16::MAX,
                    break_after: 255,
                    path: vec![row.country, 0, row.country],
                }
            );
        }
        assert_eq!(
            trains,
            [TrainInventoryRow {
                country: 99,
                node_count: u32::MAX,
                allocations: vec![
                    TrainAllocation {
                        variant: 0,
                        count: i32::MIN,
                    },
                    TrainAllocation {
                        variant: u64::MAX,
                        count: i32::MAX,
                    },
                    TrainAllocation {
                        variant: 0,
                        count: 0,
                    },
                ],
                controller_access: SparseBytes {
                    len: 3,
                    runs: vec![ByteRun {
                        start: 1,
                        bytes: vec![255, 128],
                    }],
                },
                allocated_trucks: Recorded::Value(Some(i32::MIN)),
                required_trucks_raw: Recorded::Value(Some(i64::MAX)),
                required_trains: Recorded::Value(Some(i32::MAX)),
            }]
        );
        // A repeated engine id or an escaped span refuses the table.
        let mut repeated = read_fixture();
        repeated.parts[1].nodes.rows[0].country = repeated.parts[0].nodes.rows[0].country;
        assert!(matches!(
            repeated.table_inputs()[0],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        let mut escaped = read_fixture();
        escaped.trains.countries.rows[0].entries_start = usize::MAX;
        assert!(matches!(
            escaped.table_inputs()[1],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }

    /// The SupplyTrains job reads only the hour's country slice: a new
    /// country joins at once, an unchanged one waits for its slice, and a
    /// read the spool could not publish is read again at the next hour.
    #[test]
    fn the_job_selects_new_countries_then_only_its_slice_and_rereads_unpublished_reads() {
        let mut store = SliceFingerprints::new();
        let seen = |store: &mut SliceFingerprints, hour: i32, owners: &[u32]| {
            let mut selection = SliceSelection::new(true);
            selection.reset(hour, false);
            for &owner in owners {
                selection.scan(store, owner, Some(0));
            }
            store.finish_scan();
            selection.finish_pass(store, true);
            selection
        };
        // First sight: every country is new, so all are read early.
        let first = seen(&mut store, 5, &[1, 29, 30]);
        assert!(first.contains(1) && first.contains(29) && first.contains(30));
        // The next hour (slice 6): only 30 is in its slice.
        let next = seen(&mut store, 6, &[1, 29, 30]);
        assert!(!next.contains(1) && !next.contains(29) && next.contains(30));
        assert_eq!(next.early_count, 0);
        // That read is not published: its owners are read again an hour later.
        next.invalidate(&mut store);
        let retry = seen(&mut store, 7, &[1, 29, 30]);
        assert!(retry.contains(30) && !retry.contains(1) && !retry.contains(29));
        assert_eq!(retry.early_count, 1);
        // A country first seen now is read at once.
        let joined = seen(&mut store, 8, &[1, 29, 30, 4_999]);
        assert!(joined.contains(4_999) && !joined.contains(30));
        // Without a store (the typed fallback) every country is read.
        let mut whole = SliceSelection::new(false);
        let context = CaptureContext {
            base: 0,
            game_state: 0,
            hour: 9,
            force_full: false,
        };
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        select_countries(&memory, context, &mut whole, None);
        assert!(whole.force_full && whole.contains(1) && whole.contains(4_999));
        // An unreadable census is no census: every country is read.
        let mut selection = SliceSelection::new(true);
        select_countries(&memory, context, &mut selection, Some(&mut store));
        assert!(selection.force_full);
    }

    #[test]
    fn supply_vehicle_projection_and_named_fault_counts_scale_from_the_live_census() {
        const COUNTRIES: usize = 5000;
        let mut capacities = Capacities {
            nodes: 0,
            countries: 0,
            paths: 0,
            entries: 0,
            access: 0,
            access_runs: 0,
        };
        capacities.raise_for_countries(COUNTRIES + 1);
        let mut capture = Captured::with_capacities(&capacities);
        assert!(capture.trains.countries.rows.len() >= COUNTRIES);
        for part in &mut capture.parts {
            part.nodes.outcome = Outcome::Rows { len: 0 };
        }
        capture.trains.countries.outcome = Outcome::Rows { len: COUNTRIES };
        capture.trains.vehicle_failures = [2500; 3];
        for (index, row) in capture.trains.countries.rows[..COUNTRIES]
            .iter_mut()
            .enumerate()
        {
            *row = ccountry_supply_system::TrainRow {
                country: index as u32 + 1,
                allocated_trucks: (index % 2 == 0).then_some(0),
                required_trucks_raw: (index % 2 == 0).then_some(i64::MIN),
                required_trains: (index % 2 == 0).then_some(i32::MAX),
                ..Default::default()
            };
        }
        let TableView::Complete(replay_data::rows::TableRows::TrainInventory(rows)) =
            capture.tables().into_iter().nth(1).unwrap()
        else {
            panic!("appended scalar faults must retain the inventory");
        };
        assert_eq!(rows.len(), COUNTRIES);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.country, index as u32 + 1);
            assert_eq!(
                row.allocated_trucks,
                Recorded::Value((index % 2 == 0).then_some(0))
            );
            assert_eq!(
                row.required_trucks_raw,
                Recorded::Value((index % 2 == 0).then_some(i64::MIN))
            );
            assert_eq!(
                row.required_trains,
                Recorded::Value((index % 2 == 0).then_some(i32::MAX))
            );
        }
        let mut work = VehicleWork::default();
        capture.observe_vehicles(&mut work);
        let status = serde_json::to_value(work.status()).unwrap();
        assert_eq!(status["rows"], COUNTRIES);
        assert_eq!(status["refused"], 7500);
        for reason in ccountry_supply_system::VEHICLE_READ_FAILURES {
            assert_eq!(status["refusals"][reason], 2500);
        }
        // An unrelated row shortfall retains the admitted prefix's failures.
        capture.trains.countries.outcome = Outcome::Truncated {
            len: COUNTRIES,
            unread_rows: 1,
        };
        let mut partial_work = VehicleWork::default();
        capture.observe_vehicles(&mut partial_work);
        assert_eq!(serde_json::to_value(partial_work.status()).unwrap(), status);
        capture.terminal_failure = Some("owned supply acquisition failed");
        capture.observe_vehicles(&mut work);
        assert_eq!(serde_json::to_value(work.status()).unwrap(), status);
        capture.reset();
        capture.observe_vehicles(&mut work);
        assert_eq!(serde_json::to_value(work.status()).unwrap(), status);
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
            route: engine_native::supply_route::Row {
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
        let TableView::Complete(replay_data::rows::TableRows::SupplyNodes(rows)) =
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
            route: engine_native::supply_route::Row {
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
        let TableView::Complete(replay_data::rows::TableRows::SupplyNodes(rows)) =
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
            assert_eq!(
                (owned.parts[0].paths.len, owned.parts[0].paths.required),
                (1, 9)
            );
            assert_eq!(
                (owned.trains.entries.len, owned.trains.entries.required),
                (1, 6)
            );
            assert_eq!(
                (owned.trains.access.len, owned.trains.access.required),
                (2, 2)
            );
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
            hour: 0,
            force_full: true,
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
            assert_eq!(
                (owned.parts[0].paths.len, owned.parts[0].paths.required),
                (0, 0)
            );
            assert_eq!(
                (owned.trains.entries.len, owned.trains.entries.required),
                (0, 0)
            );
            assert_eq!(
                (owned.trains.access.len, owned.trains.access.required),
                (0, 0)
            );
            assert_eq!(allocations(&owned), before);
        }
    }
}
