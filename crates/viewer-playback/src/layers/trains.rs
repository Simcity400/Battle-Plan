//! The supply trains' plan, from the mirrors: the viewing country's routes (province paths on the
//! railway's curves), their departure weights and the consists of its train variants, built by
//! today's plan builder from values the layer's plan maker holds. The rail curves come from the
//! rails layer's network and are kept per path, dropped where the rails layer redrew a strip.
//!
//! The fleet, timetable and random stream are presentation, advanced by the frame clock as today;
//! a new plan takes them over while play goes forward, as today's next hour does. At a reset
//! (`compose`: a seek, a gap, a piece's start) the plan starts with none, which is what today shows
//! when no earlier hour was built: the fleet warmed from empty at the first frame. Hidden (above the
//! railway cutoff, or trains switched off), the layer declares nothing.
//!
//! The layer's worker is fed the steps ahead of the head ([`Layer::ahead`]) and keeps its own
//! copies of the tables the plan reads ([`super::owned`]), made from the mirrors at a compose. It
//! folds every step it is handed into them as it comes, marking each row a step changed of what the
//! plan reads once (a SupplyNodes row of another country than the plan's only where its census
//! cells move or it comes or goes), and whenever it is free makes the plan from the newest step it
//! holds: steps that arrived while it worked are folded, never planned one by one, so it is behind
//! by one plan's time and never by a queue. The frame reads no row for it: it takes each plan once
//! its hour is applied and shows the hour of the newest it has; the worker makes the curves from a
//! network of its own.

mod maker;
mod store;

pub use maker::PlanRows;

use super::{
    Budget, Carried, Due, Fed, Gpu, Inputs, Layer, List, Num, Published, Text, Upstream, View,
    owned::{Delta, Shape, Standing, Tables},
    rails::Rails,
};
use crate::{
    document::Content,
    playhead_host::mirror::{Cell, Mirror, Mirrors},
};
use maker::{Made, Maker, Stats};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::Instant,
};
use store::{Batch, Forget, Meta, ROWS, class};
use viewer_foundation::{map::presentation::Continuity, map::province_map::HeightField};
use viewer_world::map::{
    railways::RailNetwork,
    trains::{Curve, PlanCache, TrainPlan, route_curve},
};

#[derive(Clone, Copy)]
struct Columns {
    country_slot: Num<u32>,
    tag: Text,
    link_slot: Num<u32>,
    original_tag: List,
    reference: Num<i32>,
    culture: Text,
    node_province: Num<u32>,
    node_kind: Num<u32>,
    node_flag: Num<u8>,
    available: Num<i64>,
    consumed: Num<i64>,
    parent: Num<i32>,
    route_from: Num<u16>,
    route_to: Num<u16>,
    break_after: Num<u8>,
    path: List,
    node_count: Num<u32>,
    allocations: List,
    access: List,
    equipment_token: Num<u32>,
    creator_link: Num<u32>,
    literal: Text,
    cooldown: Num<i32>,
    /// What a train's curve reads of the rail network: each edge's level, each province's supply
    /// source operands (today's `RailNetwork::read`).
    edge_level: Num<i32>,
    supply_capital: Num<u16>,
    supply_node: Num<u8>,
    supply_naval: Num<u8>,
    supply_flags: Num<u8>,
    controller: Num<u32>,
    reserved_tag: Text,
    reserved_original: List,
    reserved_reference: Num<i32>,
}

impl Columns {
    /// The columns of `table` whose change the plan reads. A RailEdges row is read for being
    /// there, and a SupplyNodes row's census cells come first.
    fn read(&self, table: TableId) -> Vec<usize> {
        match table {
            TableId::Countries => vec![self.country_slot.column, self.tag.column],
            TableId::CountryLinks => vec![self.link_slot.column],
            TableId::CountryAppearance => vec![self.reference.column, self.original_tag.column, self.culture.column],
            TableId::LexicalTokens => vec![self.literal.column],
            TableId::Provinces => vec![self.controller.column],
            TableId::ReservedCountryNaming => vec![self.reserved_tag.column, self.reserved_original.column, self.reserved_reference.column],
            TableId::SupplyNodes => vec![
                self.node_province.column,
                self.node_kind.column,
                self.node_flag.column,
                self.available.column,
                self.consumed.column,
                self.parent.column,
                self.route_from.column,
                self.route_to.column,
                self.break_after.column,
                self.path.column,
            ],
            TableId::TrainInventory => vec![self.node_count.column, self.allocations.column, self.access.column],
            TableId::VariantDefinitions => vec![self.equipment_token.column, self.creator_link.column],
            TableId::RailCooldowns => vec![self.cooldown.column],
            _ => Vec::new(),
        }
    }
}

/// The tables whose read cells make a plan be made again, for the proof's count of why.
const STRUCTURAL: [TableId; 8] = [
    TableId::SupplyNodes,
    TableId::Provinces,
    TableId::RailCooldowns,
    TableId::RailEdges,
    TableId::TrainInventory,
    TableId::Countries,
    TableId::CountryAppearance,
    TableId::VariantDefinitions,
];

/// The seven tables a plan needs wholly recorded.
const WHOLE: [TableId; 7] = [TableId::SupplyNodes, TableId::TrainInventory, TableId::VariantDefinitions, TableId::RailEdges, TableId::RailCooldowns, TableId::ProvinceSupply, TableId::NavalBases];
const READ: [TableId; 7] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens, TableId::Provinces, TableId::ReservedCountryNaming, TableId::SupplyNodes];
const CORE: &str = "Provinces, CountryLinks and Countries";

/// What the static content gives the trains: assets, the rail geometry, the heights, the cutoff.
pub trait Yard: Send + Sync {
    fn cutoff(&self) -> f32;
    /// The plan of the viewing slot `ours` from `rows`.
    fn plan(&self, rows: &PlanRows<'_>, ours: u32, curve: &mut dyn FnMut(&[u32]) -> io::Result<Curve>, cache: &mut PlanCache) -> io::Result<TrainPlan>;
    fn curve(&self, path: &[u32], network: &RailNetwork) -> Curve;
}

impl Yard for Content {
    fn cutoff(&self) -> f32 {
        self.map_defines.railway_cutoff
    }

    fn plan(&self, rows: &PlanRows<'_>, ours: u32, curve: &mut dyn FnMut(&[u32]) -> io::Result<Curve>, cache: &mut PlanCache) -> io::Result<TrainPlan> {
        let head = rows.head(TableId::Countries).unwrap_or(0);
        TrainPlan::build_from(rows, 0, head, ours, &self.trains, &self.trains.cultures, curve, Some(cache))
    }

    fn curve(&self, path: &[u32], network: &RailNetwork) -> Curve {
        let heights = HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        route_curve(&self.rail_geometry, path, network, &heights)
    }
}

/// What the worker's own tables keep of each table the plan reads: the cells it reads.
fn shapes(columns: &Columns) -> Vec<Shape> {
    let numbers = |table: TableId| -> Vec<usize> {
        match table {
            TableId::Countries => vec![columns.country_slot.column],
            TableId::CountryLinks => vec![columns.link_slot.column],
            TableId::CountryAppearance => vec![columns.reference.column],
            TableId::Provinces => vec![columns.controller.column],
            TableId::ReservedCountryNaming => vec![columns.reserved_reference.column],
            TableId::SupplyNodes => vec![
                columns.node_province.column,
                columns.node_kind.column,
                columns.node_flag.column,
                columns.available.column,
                columns.consumed.column,
                columns.parent.column,
                columns.route_from.column,
                columns.route_to.column,
                columns.break_after.column,
            ],
            TableId::TrainInventory => vec![columns.node_count.column],
            TableId::VariantDefinitions => vec![columns.equipment_token.column, columns.creator_link.column],
            TableId::RailCooldowns => vec![columns.cooldown.column],
            _ => Vec::new(),
        }
    };
    let lists = |table: TableId| -> Vec<(usize, bool)> {
        match table {
            TableId::CountryAppearance => vec![(columns.original_tag.column, false)],
            TableId::ReservedCountryNaming => vec![(columns.reserved_original.column, false)],
            TableId::SupplyNodes => vec![(columns.path.column, false)],
            TableId::TrainInventory => vec![(columns.allocations.column, false), (columns.access.column, false)],
            _ => Vec::new(),
        }
    };
    let texts = |table: TableId| -> Vec<usize> {
        match table {
            TableId::Countries => vec![columns.tag.column],
            TableId::CountryAppearance => vec![columns.culture.column],
            TableId::LexicalTokens => vec![columns.literal.column],
            TableId::ReservedCountryNaming => vec![columns.reserved_tag.column],
            _ => Vec::new(),
        }
    };
    ROWS.iter().map(|&table| Shape::new(table, &numbers(table), &lists(table)).with_texts(&texts(table))).collect()
}

/// The longest a whole settle waits for the plan of the hour applied before it fails by name.
const WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The tables a train's curve reads the rail network from, which the worker keeps its own copy of.
const NETWORK: [TableId; 3] = [TableId::RailEdges, TableId::ProvinceSupply, TableId::NavalBases];

fn network_shapes(columns: &Columns) -> Vec<Shape> {
    vec![
        Shape::new(TableId::RailEdges, &[columns.edge_level.column], &[]),
        Shape::new(TableId::ProvinceSupply, &[columns.supply_capital.column, columns.supply_node.column, columns.supply_naval.column, columns.supply_flags.column], &[]),
        Shape::new(TableId::NavalBases, &[], &[]),
    ]
}

/// The rail network as today's reads it from the hour's rows (`RailNetwork::read`): edges of a
/// level above zero, the provinces that are supply sources, the naval bases' provinces; none while
/// NavalBases is not wholly recorded.
fn network_of(tables: &Tables, columns: &Columns) -> RailNetwork {
    let (Some(edges), Some(supply), Some(naval)) = (tables.get(TableId::RailEdges), tables.get(TableId::ProvinceSupply), tables.get(TableId::NavalBases)) else {
        return RailNetwork::new(&[], &[], &[]);
    };
    if !naval.complete() {
        return RailNetwork::new(&[], &[], &[]);
    }
    let mut graph: BTreeMap<u32, BTreeMap<u32, i32>> = BTreeMap::new();
    for slot in edges.rows() {
        let Some(key) = edges.key(slot) else { continue };
        let (from, to, level) = ((key >> 32) as u32, key as u32, edges.number(columns.edge_level, slot).value().unwrap_or(0));
        if level > 0 && from != to {
            graph.entry(from).or_default().insert(to, level);
        }
    }
    let source = |slot: u32| {
        supply.number(columns.supply_node, slot).value().unwrap_or(0) != 0
            || supply.number(columns.supply_naval, slot).value().unwrap_or(0) != 0
            || supply.number(columns.supply_capital, slot).value().unwrap_or(0) != 0
            || supply.number(columns.supply_flags, slot).value().unwrap_or(0) & 1 != 0
    };
    let sources = supply.rows().filter(|&slot| source(slot)).filter_map(|slot| supply.key(slot)).map(|key| key as u32).collect();
    let ports = naval.rows().filter_map(|slot| naval.key(slot)).map(|key| key as u32).collect();
    RailNetwork::from_display(map_display_data::landscape::RailGraph { graph, sources, ports })
}

/// The provinces whose part of the network differs between `old` and `new`: a curve through one
/// is made again.
fn network_moved(old: &RailNetwork, new: &RailNetwork) -> BTreeSet<u32> {
    let (old, new) = (old.display(), new.display());
    let mut moved: BTreeSet<u32> = old.graph.iter().filter(|(province, edges)| new.graph.get(province) != Some(edges)).map(|(&province, _)| province).collect();
    moved.extend(new.graph.iter().filter(|(province, edges)| old.graph.get(province) != Some(edges)).map(|(&province, _)| province));
    moved.extend(old.sources.symmetric_difference(&new.sources));
    moved.extend(old.ports.symmetric_difference(&new.ports));
    moved
}

/// The tables the worker keeps the standing of alone: whether they are wholly recorded.
const STANDING: [TableId; 2] = [TableId::ProvinceSupply, TableId::NavalBases];

/// What the plan reads, as the worker keeps it from the steps: its own tables, the plan's country
/// and that country's SupplyNodes rows, the rows the steps changed, the rails' redrawn provinces
/// and the curves made since, and the plan maker.
struct Work {
    columns: Columns,
    ours: Option<u32>,
    tables: Tables,
    standing: [Option<Standing>; STANDING.len()],
    whole: Option<u32>,
    natives_moved: bool,
    statuses: HashMap<usize, Option<Status>>,
    /// Per table of `ROWS` the slots the steps changed, each once: bounded by the table's rows.
    dirty: [BTreeSet<u32>; ROWS.len()],
    batch: Batch,
    maker: Box<Maker>,
    /// Its own copy of the rail network's tables, the network made from them, and whether a step
    /// changed them since: the curves are made here, so no plan waits on a frame.
    net: Tables,
    network: RailNetwork,
    net_stale: bool,
}

/// For the proof: a weighted node of the plan shown, its weight, the hour of the batch that last
/// copied its row and the supply cells the worker holds of it now.
#[derive(Clone, Copy, Debug)]
pub struct ShownNode {
    pub key: u128,
    pub weight: i64,
    pub copied: Option<Hour>,
    pub available: Option<i64>,
    pub consumed: Option<i64>,
}

/// The plan's country as the plan finds it: the first native, in key order, of the viewing slot.
fn native_of(tables: &Tables, columns: &Columns, ours: Option<u32>) -> Option<u32> {
    let ours = ours?;
    let countries = tables.get(TableId::Countries)?;
    countries.rows().filter(|&slot| countries.number(columns.country_slot, slot).value() == Some(ours)).filter_map(|slot| countries.key(slot)).map(|key| key as u32).min()
}

impl Work {
    /// Its tables from the mirrors and a maker made from them, the plan still to make.
    fn new(content: &Arc<dyn Yard>, mirrors: &Mirrors, columns: Columns, ours: Option<u32>) -> Self {
        let tables = Tables::from_mirrors(mirrors, &shapes(&columns));
        let net = Tables::from_mirrors(mirrors, &network_shapes(&columns));
        let network = network_of(&net, &columns);
        let whole = native_of(&tables, &columns, ours);
        let maker = Box::new(Maker::new(Arc::clone(content), &columns, ours, mirrors, whole));
        let mut work = Self {
            columns,
            ours,
            tables,
            standing: STANDING.map(|table| mirrors.get(table).map(Standing::of)),
            whole,
            natives_moved: false,
            statuses: WHOLE.iter().chain(&READ).map(|&table| (table.index(), class(mirrors.get(table).and_then(Mirror::status)))).collect(),
            dirty: Default::default(),
            batch: Batch::default(),
            maker,
            net,
            network,
            net_stale: false,
        };
        work.read_whole(false);
        work
    }

    /// Its tables and the maker's rows made again from the mirrors at the hour applied: nothing a
    /// void step brought stays, neither a row in a slot the mirrors do not hold nor a copy queued
    /// from it. The curves and the rails' redrawn provinces the batch holds stay theirs.
    fn restart(&mut self, mirrors: &Mirrors) {
        self.tables = Tables::from_mirrors(mirrors, &shapes(&self.columns));
        self.net = Tables::from_mirrors(mirrors, &network_shapes(&self.columns));
        self.net_stale = true;
        self.standing = STANDING.map(|table| mirrors.get(table).map(Standing::of));
        self.statuses = WHOLE.iter().chain(&READ).map(|&table| (table.index(), class(mirrors.get(table).and_then(Mirror::status)))).collect();
        self.read_whole(false);
        self.dirty = Default::default();
        self.natives_moved = false;
        self.batch.drop_rows();
        self.maker.restart(mirrors, &self.columns, self.whole);
    }

    /// The network made again from its tables when a step changed them; the curves through the
    /// provinces whose part moved are made again with the next plan.
    fn refresh_network(&mut self) {
        if !std::mem::take(&mut self.net_stale) {
            return;
        }
        let network = network_of(&self.net, &self.columns);
        let moved = network_moved(&self.network, &network);
        self.network = network;
        self.batch.forget.provinces.extend(moved);
    }

    /// Whether SupplyNodes row `slot` is of the plan's country: read from its key in the worker's
    /// own table, after the step that wrote it.
    fn whole_row(&self, slot: u32) -> bool {
        let nodes = self.tables.get(TableId::SupplyNodes);
        nodes.is_some_and(|nodes| nodes.live(slot) && nodes.key(slot).is_some_and(|key| Some((key >> 32) as u32) == self.whole))
    }

    fn whole_native(&self) -> Option<u32> {
        native_of(&self.tables, &self.columns, self.ours)
    }

    /// The plan's country read again; with `mark`, its SupplyNodes rows are copied whole.
    fn read_whole(&mut self, mark: bool) {
        self.whole = self.whole_native();
        if !mark {
            return;
        }
        let Some(nodes) = self.tables.get(TableId::SupplyNodes) else { return };
        let rows: Vec<u32> = nodes.rows().filter(|&slot| self.whole_row(slot)).collect();
        let at = ROWS.iter().position(|&table| table == TableId::SupplyNodes).expect("SupplyNodes is read");
        self.dirty[at].extend(rows);
    }

    /// A step folded into its tables, the rows it changed of what the plan reads marked.
    fn fold(&mut self, step: &Step) {
        self.batch.seen[0] += 1;
        if step.gap() {
            self.tables.gap();
            self.standing.iter_mut().flatten().for_each(Standing::gap);
            return;
        }
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            if NETWORK.contains(&table)
                && let Some(rows) = self.net.get_mut(table)
            {
                rows.step(changes);
                self.net_stale = true;
            }
            if !WHOLE.contains(&table) && !READ.contains(&table) {
                continue;
            }
            // A status counts when its class changes, not when only its sample hour moves.
            if let Some((status, _)) = changes.status()
                && self.statuses.insert(changes.table(), class(Some(status))) != Some(class(Some(status)))
            {
                self.batch.restated = true;
            }
            if let Some(at) = STANDING.iter().position(|&held| held == table) {
                if let Some(standing) = &mut self.standing[at] {
                    standing.step(changes);
                }
                continue;
            }
            let Some(at) = ROWS.iter().position(|&held| held == table) else { continue };
            let Some(delta) = self.tables.get_mut(table).and_then(|rows| rows.step(changes)) else { continue };
            self.mark(at, table, &delta);
        }
    }

    /// A step's changes of the rows the plan reads; a SupplyNodes row of another country than the
    /// plan's only where its census cells move or it comes or goes.
    fn mark(&mut self, at: usize, table: TableId, delta: &Delta) {
        let read = self.columns.read(table);
        if table == TableId::SupplyNodes {
            let (census, rest) = read.split_at(2);
            let mut marked: Vec<u32> = delta.deleted().slots().iter().chain(delta.inserted().slots()).copied().collect();
            for written in delta.columns() {
                let column = written.column();
                if census.contains(&column) {
                    marked.extend_from_slice(written.slots());
                } else if rest.contains(&column) {
                    let before = marked.len();
                    marked.extend(written.slots().iter().copied().filter(|&slot| self.whole_row(slot)));
                    self.batch.seen[1] += written.slots().len() as u64;
                    self.batch.seen[2] += (marked.len() - before) as u64;
                }
            }
            self.dirty[at].extend(marked);
            return;
        }
        self.dirty[at].extend(delta.deleted().slots().iter().chain(delta.inserted().slots()));
        for written in delta.columns().iter().filter(|written| read.contains(&written.column())) {
            self.dirty[at].extend(written.slots());
        }
        if table == TableId::Countries && (!delta.deleted().is_empty() || !delta.inserted().is_empty() || delta.columns().iter().any(|written| written.column() == self.columns.country_slot.column && !written.is_empty())) {
            self.natives_moved = true;
        }
    }

    /// The marked rows' newest values out of its tables into the batch, with every table's state.
    fn copy(&mut self) {
        if std::mem::take(&mut self.natives_moved) && self.whole_native() != self.whole {
            self.read_whole(true);
        }
        for (at, &table) in ROWS.iter().enumerate() {
            let slots = std::mem::take(&mut self.dirty[at]);
            let Some(rows) = self.tables.get(table) else { continue };
            for slot in slots {
                self.batch.copy(table, slot, rows, &self.columns, self.whole);
            }
        }
        let metas = ROWS.iter().map(|&table| (table, Meta::of_table(self.tables.get(table)))).chain(STANDING.iter().zip(&self.standing).map(|(&table, standing)| (table, Meta::of_standing(standing.as_ref())))).collect();
        self.batch.metas_of(metas);
    }

    fn held(&self) -> usize {
        self.tables.bytes() + self.dirty.iter().map(|slots| slots.len() * 24).sum::<usize>() + self.batch.bytes() + self.maker.held()
    }
}

enum Job {
    /// The work made at a compose; its first product stands for `stamp` (none: the compose's).
    Reset { epoch: u64, work: Box<Work>, stamp: Option<Hour> },
    /// The steps handed past the hour applied were void: its tables made again from the mirrors at
    /// the hour applied, `stamp`, every row copied to the maker anew.
    Restart { epoch: u64, mirrors: Box<Mirrors>, stamp: Option<Hour> },
    Step { epoch: u64, step: Arc<Step> },
    /// The rails' redrawn provinces since the frame last sent them.
    Rails { epoch: u64, forget: Forget },
    /// Plans the frame let go of, dropped there.
    Trash(Vec<TrainPlan>),
}

/// What the worker made through the steps it folded through `stamp`: a plan (one copy for the layer,
/// one for the scene), or none when nothing the plan draws changed; why the plan is refused; what
/// making it took; the bytes it holds.
struct Product {
    epoch: u64,
    stamp: Option<Hour>,
    /// The hour the frame had applied when the worker began the batch it made this from: every
    /// step through it is in the plan.
    begun: Option<Hour>,
    plans: Option<(TrainPlan, TrainPlan)>,
    refused: Option<String>,
    stats: Stats,
    held: usize,
    /// The plan's weighted nodes as the worker holds them now, and how the plan was made.
    nodes: Vec<ShownNode>,
    /// For the proof: this batch's steps folded, SupplyNodes writes of the plan-only cells and of
    /// those to the plan country's rows, rows copied, and the plan country's nodes moved by supply
    /// alone and otherwise.
    seen: [u64; 6],
    how: &'static str,
    /// The plan country's native whose SupplyNodes rows the worker copies whole.
    whole: Option<u32>,
}

/// A job queued is a reset, or a step of `epoch` the frame has applied already: what was folded
/// before it makes no plan (the plan after that step holds its rows, which the marks keep until
/// then). Rails and trash between them do not count.
fn superseded(queue: &VecDeque<Job>, epoch: u64, due: &Due) -> bool {
    queue.iter().any(|job| match job {
        Job::Reset { .. } | Job::Restart { .. } => true,
        Job::Step { epoch: of, step } => *of == epoch && due.reached(step.last()),
        Job::Rails { .. } | Job::Trash(_) => false,
    })
}

/// The worker: it takes every job that has arrived, folds the steps among them into its own tables,
/// and makes a plan once none left of what it took is a step applied already: one plan for the
/// newest hour the frame has applied, or for the step it applies next. What arrives meanwhile waits
/// for that plan, so a frame that keeps applying never starves it.
fn run(jobs: Receiver<Job>, out: Sender<Product>, due: Due) {
    let mut work: Option<Box<Work>> = None;
    let (mut epoch, mut stamp) = (0, None);
    while let Ok(job) = jobs.recv() {
        // Read before the jobs are taken: the frame tells it an hour only once its steps are sent.
        let begun = due.hour();
        let mut queue: VecDeque<Job> = std::iter::once(job).chain(jobs.try_iter()).collect();
        while let Some(job) = queue.pop_front() {
            let started = Instant::now();
            match job {
                Job::Reset { epoch: next, work: made, stamp: at } => (epoch, work, stamp) = (next, Some(made), at),
                Job::Restart { epoch: next, mirrors, stamp: at } => {
                    (epoch, stamp) = (next, at);
                    if let Some(work) = &mut work {
                        work.restart(&mirrors);
                    }
                }
                Job::Step { epoch: of, step } if of == epoch => {
                    let Some(work) = &mut work else { continue };
                    stamp = stamp.max(Some(step.last()));
                    work.fold(&step);
                    drop(step);
                }
                Job::Rails { epoch: of, forget } if of == epoch => {
                    let Some(work) = &mut work else { continue };
                    work.batch.forget.all |= forget.all;
                    work.batch.forget.provinces.extend(forget.provinces);
                }
                Job::Trash(trash) => {
                    drop(trash);
                    continue;
                }
                Job::Step { .. } | Job::Rails { .. } => continue,
            }
            let Some(work) = &mut work else { continue };
            if superseded(&queue, epoch, &due) {
                continue;
            }
            work.refresh_network();
            work.copy();
            work.batch.at = stamp;
            let [steps, writes, marked] = work.batch.seen;
            let copies = work.batch.node_copies() as u64;
            work.maker.take(std::mem::take(&mut work.batch));
            let seen = [steps, writes, marked, copies, work.maker.moved[0], work.maker.moved[1]];
            let made = work.maker.make_now(&work.network, true);
            work.maker.add_worker(started.elapsed().as_nanos() as u64);
            let sent = Product {
                epoch,
                stamp,
                begun,
                plans: matches!(made, Made::Plan).then(|| (work.maker.plan.clone(), work.maker.plan.clone())),
                refused: work.maker.refused.clone(),
                stats: work.maker.take_stats(),
                held: work.held(),
                nodes: work.maker.shown_nodes(),
                seen,
                how: work.maker.how,
                whole: work.whole,
            };
            if out.send(sent).is_err() {
                return;
            }
        }
    }
}

pub struct Trains {
    content: Arc<dyn Yard>,
    columns: Option<Columns>,
    /// The viewing slot: the vision layer's when one is registered before this one, else the
    /// one given.
    ours: Option<u32>,
    /// Whether it declares the vision layer it reads (the window); the proof tool gives a slot.
    reads_vision: bool,
    shown: bool,
    plan: TrainPlan,
    /// Why no plan is drawn: today's builder refused the hour, or not composed yet.
    refused: Option<String>,
    /// The plan the scene takes next, and how it follows the one the scene holds.
    to_scene: Option<TrainPlan>,
    drawn_stale: Option<Continuity>,
    /// The worker, the steps handed to it and the products it made, each taken once its hour is
    /// applied; a product of an older compose (`epoch`) is dropped.
    epoch: u64,
    worker: Option<(Sender<Job>, Receiver<Product>)>,
    fed: Fed<Product>,
    /// The provinces a curve the worker holds runs through.
    curved: HashSet<u32>,
    frame: u64,
    /// Plans let go of, dropped on the worker.
    trash: Vec<TrainPlan>,
    /// A whole settle's wait for the plan of the hour applied ran past [`WAIT`]: what the worker
    /// owed then, to fail by.
    stalled: Option<String>,
    stats: Stats,
    worker_held: usize,
    /// For the proof: the shown plan's weighted nodes as its worker holds them now, how it was made
    /// with the hours it stands for and was begun at, the newest product since that changed nothing
    /// drawn, and the batches' counts summed since that plan (see [`Product`]'s `seen`).
    nodes: Vec<ShownNode>,
    since_plan: [u64; 6],
    /// For the proof: per step the frame applied after the shown plan's hour, its last hour and its
    /// SupplyNodes writes of the plan-only cells, as the mirrors took them.
    applied_writes: VecDeque<(Hour, u64)>,
    made: (&'static str, Option<Hour>, Option<Hour>),
    quiet: Option<Hour>,
    whole: Option<u32>,
    /// Mirror rows `apply` read.
    #[cfg(test)]
    applied_reads: u64,
}

impl Trains {
    pub fn new(content: Arc<dyn Yard>, ours: Option<u32>) -> Self {
        Self {
            content,
            columns: None,
            ours,
            reads_vision: false,
            shown: true,
            plan: TrainPlan::default(),
            refused: Some("not composed yet".into()),
            to_scene: None,
            drawn_stale: Some(Continuity::Reset),
            epoch: 0,
            worker: None,
            fed: Fed::default(),
            curved: HashSet::new(),
            frame: 0,
            trash: Vec::new(),
            stalled: None,
            stats: Stats::default(),
            worker_held: 0,
            nodes: Vec::new(),
            since_plan: [0; 6],
            applied_writes: VecDeque::new(),
            made: ("nothing", None, None),
            quiet: None,
            whole: None,
            #[cfg(test)]
            applied_reads: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, shown: self.shown, reads_vision: self.reads_vision, ..Self::new(Arc::clone(&self.content), self.ours) }
    }

    /// The window's: it reads the viewing slot of the vision layer registered before it.
    pub fn reading_vision(self) -> Self {
        Self { reads_vision: true, ..self }
    }

    /// The vision layer's viewing slot, when one is registered before this one; whether it changed.
    fn take_ours(&mut self, upstream: &Upstream<'_>) -> bool {
        let Some(vision) = upstream.get::<crate::layers::vision::Vision>() else { return false };
        let ours = vision.ours();
        let changed = self.ours != ours;
        self.ours = ours;
        changed
    }

    /// The newest hour its worker has made a product of, as received on the frame.
    #[cfg(test)]
    pub fn made_through(&self) -> Option<Hour> {
        self.fed.made_through()
    }

    pub fn plan(&self) -> &TrainPlan {
        &self.plan
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// The trains' limitation notes in today's words, from the tables' `statuses` at the head;
    /// none while hidden.
    pub fn notes(&self, statuses: &dyn viewer_foundation::map::tables::Statuses) -> Vec<String> {
        if !self.shown {
            return Vec::new();
        }
        let mut out = viewer_foundation::map::tables::Limitations::default();
        out.note_trains(statuses);
        out.into_lines()
    }

    pub fn ours(&self) -> Option<u32> {
        self.ours
    }

    /// Plans made, batches that changed nothing drawn, curves made, plans whose weights alone moved.
    pub fn marked(&self) -> [usize; 4] {
        self.stats.marked
    }

    /// Plans made again by a change to a table the plan read, by `STRUCTURAL`, and by rails redrawn.
    pub fn why(&self) -> [u64; 9] {
        self.stats.why
    }

    /// Microseconds in the census, the plan, its nodes' reading, curves and the carried
    /// presentation; then the tables whose status moved.
    pub fn timed(&self) -> [u64; 6] {
        self.stats.timed
    }

    #[cfg(test)]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    /// For the proof: how the plan shown was made (composed, built or its weights alone), the hour
    /// its rows were folded through and the hour applied when its work began, and the newest
    /// product since that changed nothing drawn.
    pub fn made(&self) -> String {
        let (how, stamp, begun) = self.made;
        let [steps, writes, marked, copies, supply, other] = self.since_plan;
        let applied: Vec<(Hour, u64)> = self.applied_writes.iter().copied().filter(|&(hour, _)| self.quiet.is_none_or(|quiet| hour <= quiet)).collect();
        format!(
            "{how} through {stamp:?}, begun at {begun:?}; quiet since, through {:?}; the worker copies whole the rows of native {:?}; in the quiet batches since: {steps} steps folded, {writes} SupplyNodes writes of plan-only cells, {marked} of them to the plan country's rows, {copies} rows copied, {supply} plan nodes moved by supply alone, {other} otherwise; the frame applied since the plan's hour (last hour, those writes): {applied:?}",
            self.quiet, self.whole
        )
    }

    /// The shown plan's weighted nodes as its worker holds them now.
    pub fn shown_nodes(&self) -> &[ShownNode] {
        &self.nodes
    }

    /// A SupplyNodes row of `mirrors` in the cells a weight and a walk read, for the proof's notes.
    pub fn node_cells(&self, mirrors: &Mirrors, key: u128) -> String {
        let (Some(columns), Some(mirror)) = (self.columns, mirrors.get(TableId::SupplyNodes)) else { return "SupplyNodes not held".into() };
        let Some(slot) = mirror.slot(key) else { return "no row".into() };
        let path = match mirror.list(columns.path, slot) {
            Cell::Value(bytes) => bytes.len() / 4,
            _ => 0,
        };
        format!(
            "province {:?} kind {:?} flag {:?} available {:?} consumed {:?} parent {:?} route_from {:?} break_after {:?} path of {path}",
            mirror.number(columns.node_province, slot).value(),
            mirror.number(columns.node_kind, slot).value(),
            mirror.number(columns.node_flag, slot).value(),
            mirror.number(columns.available, slot).value(),
            mirror.number(columns.consumed, slot).value(),
            mirror.number(columns.parent, slot).value(),
            mirror.number(columns.route_from, slot).value(),
            mirror.number(columns.break_after, slot).value(),
        )
    }

    /// What a wait for the trains' plan waits on: the hour applied, the newest plan made and the
    /// one shown.
    pub fn owed(&self) -> String {
        format!("the trains' worker owes the plan of hour {:?}: newest made {:?}, shown {:?}", self.fed.through(), self.fed.made_through(), self.fed.shown())
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the trains layer is declared before it composes")
    }

    fn send(&mut self, job: Job) {
        if let Some((jobs, _)) = &self.worker
            && jobs.send(job).is_err()
        {
            self.worker = None;
        }
    }

    /// The rails layer's redrawn strips: the worker's curves through them go.
    fn forward_redraws(&mut self, upstream: &Upstream<'_>) {
        let redrawn = upstream.changed::<Rails>();
        let forget = if redrawn.all() {
            if self.curved.is_empty() {
                return;
            }
            self.curved.clear();
            Forget { all: true, provinces: BTreeSet::new() }
        } else {
            let provinces: BTreeSet<u32> = redrawn.of("province").iter().map(|&id| id as u32).filter(|province| self.curved.contains(province)).collect();
            if provinces.is_empty() {
                return;
            }
            Forget { all: false, provinces }
        };
        self.send(Job::Rails { epoch: self.epoch, forget });
    }

    /// What the worker made received, and every product due taken: those of the hours applied. With
    /// `wait`, until the product of the last hour applied has arrived.
    fn exchange(&mut self, wait: bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            let waiting = wait && !self.fed.arrived();
            let Some((_, made)) = &self.worker else { break };
            let out = if waiting {
                match made.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(out) => Some(out),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        self.stalled = Some(self.owed());
                        break;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
                }
            } else {
                made.try_recv().ok()
            };
            let Some(out) = out else {
                if waiting {
                    // The worker went: the next settle composes again.
                    self.worker = None;
                }
                break;
            };
            if out.epoch == self.epoch {
                self.fed.receive(out.stamp, out);
            }
        }
        let mut due = Vec::new();
        self.fed.take(|product| due.push(product));
        for product in due {
            self.home(product);
        }
    }

    /// A product of an hour applied, shown.
    fn home(&mut self, product: Product) {
        if let (Some(begun), Some(stamp)) = (product.begun, product.stamp) {
            self.stats.plan_short = self.stats.plan_short.max(begun - stamp);
        }
        self.stats.add(&product.stats);
        self.worker_held = product.held;
        self.refused = product.refused;
        if product.plans.is_some() {
            (self.made, self.quiet, self.since_plan) = ((product.how, product.stamp, product.begun), None, [0; 6]);
            while self.applied_writes.front().is_some_and(|&(hour, _)| product.stamp.is_some_and(|stamp| hour <= stamp)) {
                self.applied_writes.pop_front();
            }
        } else {
            self.quiet = product.stamp;
            self.since_plan.iter_mut().zip(product.seen).for_each(|(sum, more)| *sum += more);
        }
        self.nodes = product.nodes;
        self.whole = product.whole;
        if let Some((layer, scene)) = product.plans {
            self.trash.push(std::mem::replace(&mut self.plan, layer));
            self.trash.extend(self.to_scene.replace(scene));
            self.drawn_stale = Some(if self.drawn_stale == Some(Continuity::Reset) { Continuity::Reset } else { Continuity::Continue });
        }
    }

    /// Entries of `self` that differ from `other`'s, and the first described.
    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        if self.refused != other.refused {
            return (1, Some(format!("refused: {:?} against {:?}", self.refused, other.refused)));
        }
        match self.plan.differ(&other.plan) {
            Some(first) => (1, Some(first)),
            None => (0, None),
        }
    }
}

impl Layer for Trains {
    fn name(&self) -> &'static str {
        "trains"
    }

    fn placed(&mut self, view: &crate::layers::View) -> Option<u64> {
        Some(u64::from(view.camera_height < self.content.cutoff()))
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.shown = view.camera_height < self.content.cutoff() && view.graphics.supply_trains();
        if !self.shown {
            self.refused = Some("the trains are hidden".into());
            return Ok(());
        }
        inputs.after("rails");
        if self.reads_vision {
            inputs.after("vision");
        }
        for table in WHOLE.into_iter().chain(READ) {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            country_slot: inputs.number(TableId::Countries, "canonical_slot")?,
            tag: inputs.text(TableId::Countries, "tag")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            original_tag: inputs.list(TableId::CountryAppearance, "original_tag")?,
            reference: inputs.number(TableId::CountryAppearance, "original_tag_reference")?,
            culture: inputs.text(TableId::CountryAppearance, "graphical_culture")?,
            node_province: inputs.number(TableId::SupplyNodes, "province")?,
            node_kind: inputs.number(TableId::SupplyNodes, "kind")?,
            node_flag: inputs.number(TableId::SupplyNodes, "flag")?,
            available: inputs.number(TableId::SupplyNodes, "available")?,
            consumed: inputs.number(TableId::SupplyNodes, "consumed")?,
            parent: inputs.number(TableId::SupplyNodes, "parent")?,
            route_from: inputs.number(TableId::SupplyNodes, "route_from")?,
            route_to: inputs.number(TableId::SupplyNodes, "route_to")?,
            break_after: inputs.number(TableId::SupplyNodes, "break_after")?,
            path: inputs.list(TableId::SupplyNodes, "path")?,
            node_count: inputs.number(TableId::TrainInventory, "node_count")?,
            allocations: inputs.list(TableId::TrainInventory, "allocations")?,
            access: inputs.list(TableId::TrainInventory, "controller_access")?,
            equipment_token: inputs.number(TableId::VariantDefinitions, "equipment_token")?,
            creator_link: inputs.number(TableId::VariantDefinitions, "creator_link")?,
            literal: inputs.text(TableId::LexicalTokens, "literal")?,
            cooldown: inputs.number(TableId::RailCooldowns, "cooldown_ticks")?,
            edge_level: inputs.number(TableId::RailEdges, "level")?,
            supply_capital: inputs.number(TableId::ProvinceSupply, "capital_country_index")?,
            supply_node: inputs.number(TableId::ProvinceSupply, "supply_node_level")?,
            supply_naval: inputs.number(TableId::ProvinceSupply, "naval_base_level")?,
            supply_flags: inputs.number(TableId::ProvinceSupply, "flags")?,
            controller: inputs.number(TableId::Provinces, "controller")?,
            reserved_tag: inputs.text(TableId::ReservedCountryNaming, "tag")?,
            reserved_original: inputs.list(TableId::ReservedCountryNaming, "original_tag")?,
            reserved_reference: inputs.number(TableId::ReservedCountryNaming, "original_tag_reference")?,
        });
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Err(io::Error::other("the trains layer reads the rails layer: composed with its upstream"))
    }

    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    /// The plan made here from the mirrors, whole; the worker started over with its own tables made
    /// from them and that maker, and its first product, of the compose's hour, waited for.
    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.take_ours(upstream);
        self.epoch += 1;
        self.trash.clear();
        self.applied_writes.clear();
        self.drawn_stale = Some(Continuity::Reset);
        self.to_scene = None;
        if !self.shown {
            (self.plan, self.curved, self.worker) = (TrainPlan::default(), HashSet::new(), None);
            self.refused = Some("the trains are hidden".into());
            return Ok(());
        }
        let mut work = Work::new(&self.content, mirrors, self.columns(), self.ours);
        work.maker.make_now(&work.network, false);
        // A compose is not a step: its making is not counted with the steps'.
        work.maker.take_stats();
        self.curved = work.maker.curved().collect();
        self.plan = work.maker.plan.clone();
        (self.nodes, self.made, self.quiet, self.whole, self.since_plan) = (work.maker.shown_nodes(), ("compose", mirrors.get(TableId::Countries).and_then(Mirror::head), None), None, work.whole, [0; 6]);
        self.to_scene = Some(work.maker.plan.clone());
        self.refused = work.maker.refused.clone();
        self.worker_held = work.held();
        if self.worker.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            let due = self.fed.due();
            std::thread::Builder::new().name("trains plan".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                run(receive, send, due)
            })?;
            self.worker = Some((jobs, made));
        }
        self.fed.compose(mirrors.get(TableId::Countries).and_then(Mirror::head));
        self.send(Job::Reset { epoch: self.epoch, work: Box::new(work), stamp: None });
        self.exchange(true);
        if let Some(owed) = self.stalled.take() {
            return Err(io::Error::other(format!("The trains' first plan did not come within {} s: {owed}", WAIT.as_secs())));
        }
        Ok(())
    }

    /// Nothing but the hour applied: the worker folds the steps it was handed.
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        let Some(columns) = self.columns.filter(|_| self.shown) else { return };
        self.fed.applied_unpublished(step);
        let read = columns.read(TableId::SupplyNodes);
        let writes = step.table(TableId::SupplyNodes.index()).map_or(0, |changes| changes.columns().iter().filter(|written| read[2..].contains(&written.column())).map(|written| written.slots().len() as u64).sum());
        self.applied_writes.push_back((step.last(), writes));
    }

    fn wants_ahead(&self) -> bool {
        self.shown && self.columns.is_some()
    }

    fn ahead(&mut self, steps: &[Arc<Step>]) {
        let handed: Vec<Arc<Step>> = self.fed.hand(steps).into_iter().cloned().collect();
        for step in handed {
            self.send(Job::Step { epoch: self.epoch, step });
        }
        self.fed.publish();
        if !self.trash.is_empty() {
            let trash = std::mem::take(&mut self.trash);
            self.send(Job::Trash(trash));
        }
    }

    /// The worker starts over from the mirrors at the hour applied, its maker kept: its first plan
    /// there stands for that hour, and the steps from there are handed again.
    fn void_ahead(&mut self, mirrors: &Mirrors, _kept: Option<Hour>) {
        if self.worker.is_none() || self.columns.is_none() || !self.shown {
            return;
        }
        self.epoch += 1;
        self.fed.void();
        let mut held = Mirrors::default();
        for table in WHOLE.into_iter().chain(READ) {
            if let Some(mirror) = mirrors.get(table) {
                held.insert(Box::new(mirror.clone()));
            }
        }
        let stamp = self.fed.applied_hour();
        self.send(Job::Restart { epoch: self.epoch, mirrors: Box::new(held), stamp });
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        self.frame += 1;
        // Every step applied was handed before this settle (the host hands them first).
        self.fed.publish();
        if !self.shown {
            // Hidden it holds nothing; shown again, its tables return as a reset and it composes.
            if self.worker.is_some() || self.plan.counts() != [0; 3] {
                let (stats, frame, refused) = (self.stats, self.frame, self.refused.take());
                *self = Self { stats, frame, refused, ..self.fresh() };
            }
            return Ok(());
        }
        // Another viewing slot is another plan: made from the head as a reader opened on it makes it.
        if self.take_ours(upstream) {
            return self.compose_with(mirrors, upstream);
        }
        // Read before anything returns: the host clears these once this layer has settled.
        self.forward_redraws(upstream);
        self.exchange(budget.unlimited_now());
        if let Some(owed) = self.stalled.take() {
            return Err(io::Error::other(format!("The trains' plan of the hour applied did not come within {} s: {owed}", WAIT.as_secs())));
        }
        if self.worker.is_none() {
            // Its worker went: made again from the mirrors.
            return self.compose_with(mirrors, upstream);
        }
        Ok(())
    }

    /// The plan into the scene's train pass, which carries the fleet it runs into it as today's
    /// install does (anew after a compose); hidden or refused, an empty plan.
    fn upload(&mut self, gpu: &mut Gpu<'_>, budget: &mut Budget) -> io::Result<()> {
        let Some(continuity) = self.drawn_stale else { return Ok(()) };
        if !budget.left() {
            return Ok(());
        }
        let drawn = self.shown && self.refused.is_none();
        let plan = match self.to_scene.take() {
            Some(plan) if drawn => plan,
            other => {
                self.trash.extend(other);
                if drawn { self.plan.clone() } else { TrainPlan::default() }
            }
        };
        self.trash.extend(gpu.scene.set_trains(Arc::clone(&gpu.content.trains), plan, continuity)?);
        self.drawn_stale = None;
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.drawn_stale.is_none()
    }

    /// Nothing: the worker makes the plan and its curves, and what it holds is published.
    fn carried(&self) -> Carried {
        Carried::default()
    }

    /// The hour the plan shown stands for: current with every step applied, or the hour of the
    /// newest plan the worker made.
    fn published(&self) -> Published {
        if !self.shown || self.columns.is_none() {
            return Published::Current;
        }
        self.fed.published()
    }

    fn held(&self) -> usize {
        let [routes, nodes, variants] = self.plan.counts();
        let plans = 1 + usize::from(self.to_scene.is_some()) + self.trash.len();
        self.curved.capacity() * 8 + plans * (routes * 96 + nodes * 32 + variants * 64) + self.worker_held
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The trains' figures for the paced report: makings on the worker by what they came to, the
/// worker's milliseconds a making, and the microseconds of a making's parts, summed.
pub fn summary(side: &crate::playhead_host::MapSide) -> serde_json::Value {
    let Some(layer) = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Trains>())) else { return serde_json::Value::Null };
    let stats = layer.stats;
    let makings = stats.marked[0] + stats.marked[1] + stats.marked[3];
    serde_json::json!({
        "plans_made": stats.marked[0],
        "quiet": stats.marked[1],
        "weights_only": stats.marked[3],
        "curves_made": stats.marked[2],
        "worker_ms_a_making_mean": (makings > 0).then(|| stats.worker_ns as f64 / 1e6 / makings as f64),
        "plan_hours_short_of_its_start_worst": stats.plan_short,
        "us_census_plan_nodes_curves_carry": &stats.timed[..5],
        "scope": "Makings taken by the frame (a plan made, nothing drawn changed, or the weights alone moved); the worker's wall time a making, from the fold of its last step through the plan; the parts' microseconds, summed over the run. curves_made: the rail curves the plans needed, made on the worker from its own copy of the rail network.",
    })
}

#[cfg(test)]
mod tests;
