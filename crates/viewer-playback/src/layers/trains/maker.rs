//! The trains' plan maker: what the plan reads ([`Store`]), the census of supply nodes the synthetic
//! naval breaks read, each weighted node's kept walks, each path's rail curve, and the plan. It
//! takes a batch whole, works out from each changed row's values before and after which kept walks
//! it forgets and whether the plan is made again or only its weights move, then makes the plan. A
//! curve it does not hold it asks the frame side for, which holds the rails layer's network. It
//! runs on the layer's worker, or on the calling thread at a compose and an unlimited settle.

use super::{
    CORE, Columns, STRUCTURAL, WHOLE, Yard,
    store::{Batch, Encoded, Meta, Node, Reserved, Store},
};
use crate::playhead_host::mirror::{Cell, Mirrors};
use replay_columnar::manifest::Status;
use replay_data::{
    Hour,
    rows::{Field, NativeValue, Opaque, Recorded, SparseBytes, SupplyNodeRow, TableId, TrainAllocation},
};
use std::{
    cell::{Cell as Counter, RefCell},
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
    time::Instant,
};
use viewer_foundation::{map::presentation::Continuity, table_access::TokenError};
use viewer_world::map::{
    railways::RailNetwork,
    trains::{CreatorAppearance, CreatorRows, Curve, Fallback, NodeValues, PlanCache, TrainPlan, TrainRows, node_weight},
};

fn plain<T: NativeValue>(encoded: &Encoded, what: &str) -> Result<T, String> {
    match &encoded.cell {
        Cell::Value(bytes) => T::from_field(Field::Opaque(Opaque::Encoded(bytes, encoded.format))).map_err(|error| error.to_string()),
        _ => Err(format!("{what} is not recorded at the head")),
    }
}

fn decoded<T: NativeValue>(encoded: &Encoded) -> Result<Recorded<T>, String> {
    match &encoded.cell {
        Cell::Value(bytes) => Recorded::<T>::from_field(Field::Opaque(Opaque::Encoded(bytes, encoded.format))).map_err(|error| error.to_string()),
        _ => Ok(Recorded::NotRecorded),
    }
}

fn recorded(cell: Cell<i32>) -> Recorded<Option<i32>> {
    match cell {
        Cell::Value(reference) => Recorded::Value(Some(reference)),
        Cell::Missing => Recorded::Value(None),
        Cell::Unrecorded => Recorded::NotRecorded,
    }
}

/// What the last plan read besides its country's nodes: a change elsewhere leaves it as it is.
#[derive(Default)]
pub(super) struct Queried {
    provinces: HashSet<u32>,
    pairs: HashSet<(u32, u32)>,
    variants: HashSet<u64>,
    tokens: HashSet<u32>,
}

/// One node of the census: its country's canonical slot, province and kind.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Entry {
    country: u32,
    province: u32,
    kind: u32,
    vertex: bool,
}

/// What a census count changed for the plan: provinces that entered or left a country's kind 2 to
/// 5 nodes, and whether an entry the plan refuses was counted or uncounted.
#[derive(Default)]
struct Counted {
    moved: Vec<u32>,
    refused: bool,
}

/// The supply nodes the synthetic naval breaks read, by key: each node's entry, the kind 2 to 5
/// nodes counted per (slot, province), the entries a plan refuses (an unknown kind, a country
/// without a Countries row), each native's keys and every Countries row as (canonical slot, native).
#[derive(Default)]
struct Census {
    entries: HashMap<u128, Entry>,
    subtype_zero: HashMap<(u32, u32), u32>,
    errors: [u32; 2],
    by_country: HashMap<u32, BTreeSet<u128>>,
    natives: BTreeSet<(u32, u32)>,
    native_slots: HashMap<u32, u32>,
    /// Natives whose Countries row came, went or took another canonical slot: their nodes are
    /// counted again.
    country_natives: BTreeSet<u32>,
    touched: BTreeSet<u128>,
    recount: bool,
}

impl Census {
    fn entry(store: &Store, key: u128) -> Option<Entry> {
        if !store.countries.meta.held {
            return None;
        }
        let node = store.nodes.get(key)?;
        let country = store.countries.get(key >> 32).and_then(|country| country.slot).unwrap_or(u32::MAX);
        Some(Entry { country, province: node.province, kind: node.kind, vertex: (key as u32) & SupplyNodeRow::VERTEX != 0 })
    }

    fn count(&mut self, entry: Entry, add: bool, out: &mut Counted) {
        if entry.vertex {
            return;
        }
        let step = |value: &mut u32| if add { *value += 1 } else { *value -= 1 };
        if entry.kind > 5 {
            step(&mut self.errors[0]);
            out.refused = true;
            return;
        }
        if entry.kind < 2 {
            return;
        }
        if entry.country == u32::MAX {
            step(&mut self.errors[1]);
            out.refused = true;
            return;
        }
        let held = self.subtype_zero.entry((entry.country, entry.province)).or_default();
        step(held);
        let moved = *held == 0 || (add && *held == 1);
        if *held == 0 {
            self.subtype_zero.remove(&(entry.country, entry.province));
        }
        if moved {
            out.moved.push(entry.province);
        }
    }

    /// Every node counted again at a recount, else the nodes a batch touched.
    fn settle(&mut self, store: &Store) -> Counted {
        let mut out = Counted::default();
        if std::mem::take(&mut self.recount) {
            self.native_slots = store.countries.iter().filter_map(|(key, country)| Some((key as u32, country.slot?))).collect();
            self.natives = self.native_slots.iter().map(|(&native, &slot)| (slot, native)).collect();
            self.country_natives.clear();
            self.by_country.clear();
            self.entries.clear();
            self.subtype_zero.clear();
            self.errors = [0; 2];
            self.touched = store.nodes.iter().map(|(key, _)| key).collect();
        } else {
            for native in std::mem::take(&mut self.country_natives) {
                if let Some(slot) = self.native_slots.remove(&native) {
                    self.natives.remove(&(slot, native));
                }
                if let Some(slot) = store.countries.get(u128::from(native)).and_then(|country| country.slot) {
                    self.native_slots.insert(native, slot);
                    self.natives.insert((slot, native));
                }
                self.touched.extend(self.by_country.get(&native).into_iter().flatten().copied());
            }
        }
        while let Some(key) = self.touched.pop_first() {
            let entry = Self::entry(store, key);
            let native = (key >> 32) as u32;
            // A node counted the same is left as it is; a country's keys move only when a node comes or goes.
            match (self.entries.get(&key).copied(), entry) {
                (Some(old), Some(new)) if old == new => {}
                (Some(old), Some(new)) => {
                    self.count(old, false, &mut out);
                    self.count(new, true, &mut out);
                    self.entries.insert(key, new);
                }
                (Some(old), None) => {
                    self.count(old, false, &mut out);
                    self.entries.remove(&key);
                    if let Some(keys) = self.by_country.get_mut(&native) {
                        keys.remove(&key);
                    }
                }
                (None, Some(new)) => {
                    self.count(new, true, &mut out);
                    self.entries.insert(key, new);
                    self.by_country.entry(native).or_default().insert(key);
                }
                (None, None) => {}
            }
        }
        out
    }

    fn bytes(&self) -> usize {
        self.entries.capacity() * 40 + self.subtype_zero.capacity() * 16 + self.by_country.values().map(|keys| keys.len() * 24 + 48).sum::<usize>() + self.natives.len() * 24 + self.native_slots.capacity() * 12
    }
}

/// What the maker did since its figures were last taken: plans made, batches that changed nothing
/// drawn, curves made, plans whose weights alone moved; microseconds in the census, the plan, its
/// nodes' reading, curves and the carried presentation, then the tables whose status moved; and
/// the plans made again by a change to a table the plan read, by `STRUCTURAL`, and by rails redrawn.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Stats {
    pub(super) marked: [usize; 4],
    pub(super) timed: [u64; 6],
    pub(super) why: [u64; 9],
    pub(super) worker_ns: u64,
    /// The most hours a plan taken stood before the hour the frame had applied when the worker
    /// began it: above zero, a plan missed a step it should hold.
    pub(super) plan_short: i32,
}

impl Stats {
    pub(super) fn add(&mut self, other: &Self) {
        self.marked.iter_mut().zip(other.marked).for_each(|(held, more)| *held += more);
        self.timed.iter_mut().zip(other.timed).for_each(|(held, more)| *held += more);
        self.why.iter_mut().zip(other.why).for_each(|(held, more)| *held += more);
        self.worker_ns += other.worker_ns;
        self.plan_short = self.plan_short.max(other.plan_short);
    }
}

/// What a making came to.
pub(super) enum Made {
    /// A new plan, or new weights for the one it holds.
    Plan,
    /// The plan needs the curves of these paths first; the walks made are kept.
    Wants(Vec<Vec<u32>>),
    /// Nothing the plan draws changed.
    Quiet,
}

pub struct Maker {
    content: Arc<dyn Yard>,
    ours: Option<u32>,
    store: Store,
    census: Census,
    /// The plan's country, and its weighted nodes' keys and weights in the plan's order.
    country: Option<u32>,
    weighted: Vec<u128>,
    weights: Vec<i64>,
    /// The plan is made again; only its weights may have moved; the `STRUCTURAL` causes counted in
    /// this batch.
    structural: bool,
    reweigh: bool,
    counted: u16,
    queried: RefCell<Queried>,
    /// Each weighted node's checked routes from the last plans, forgotten by what a batch changed.
    pub(super) cache: PlanCache,
    /// Per province path its rail curve, until the rails layer redraws a strip on it; per
    /// province, the paths through it with a kept curve.
    curves: HashMap<Vec<u32>, Curve>,
    paths_of: HashMap<u32, BTreeSet<Vec<u32>>>,
    pub(super) plan: TrainPlan,
    pub(super) refused: Option<String>,
    /// How the plan was last made: built, or its weights alone replaced.
    pub(super) how: &'static str,
    /// For the proof: in the last batch taken, the plan country's nodes moved by their supply alone,
    /// and moved otherwise.
    pub(super) moved: [u64; 2],
    stats: Stats,
    nodes_ns: Counter<u64>,
}

impl Maker {
    /// Everything from the mirrors, the plan still to make; SupplyNodes rows whole for `whole`.
    pub(super) fn new(content: Arc<dyn Yard>, columns: &Columns, ours: Option<u32>, mirrors: &Mirrors, whole: Option<u32>) -> Self {
        Self {
            content,
            ours,
            store: Store::from_mirrors(mirrors, columns, whole),
            census: Census { recount: true, ..Census::default() },
            country: None,
            weighted: Vec::new(),
            weights: Vec::new(),
            structural: true,
            reweigh: false,
            counted: 0,
            queried: RefCell::default(),
            cache: PlanCache::default(),
            curves: HashMap::new(),
            paths_of: HashMap::new(),
            plan: TrainPlan::default(),
            refused: Some("composing".into()),
            how: "built",
            moved: [0; 2],
            stats: Stats::default(),
            nodes_ns: Counter::new(0),
        }
    }

    /// Its rows made whole from the mirrors at the hour applied, the census counted again and every
    /// kept walk forgotten: the plan is made again from them. Its curves stay: the network they
    /// follow forgets its own moved provinces.
    pub(super) fn restart(&mut self, mirrors: &Mirrors, columns: &Columns, whole: Option<u32>) {
        self.store = Store::from_mirrors(mirrors, columns, whole);
        self.census = Census { recount: true, ..Census::default() };
        self.cache.clear();
        *self.queried.borrow_mut() = Queried::default();
        (self.structural, self.reweigh) = (true, false);
    }

    /// The figures since they were last taken.
    pub(super) fn take_stats(&mut self) -> Stats {
        let mut stats = std::mem::take(&mut self.stats);
        stats.timed[2] = self.nodes_ns.take() / 1000;
        stats
    }

    pub(super) fn add_worker(&mut self, ns: u64) {
        self.stats.worker_ns += ns;
    }

    /// A change to cells the last plan read through `table`: counted once a batch.
    fn structural_by(&mut self, table: TableId) {
        self.structural = true;
        if let Some(at) = STRUCTURAL.iter().position(|&read| read == table)
            && self.counted & (1 << at) == 0
        {
            self.counted |= 1 << at;
            self.stats.why[at] += 1;
        }
    }

    /// The provinces a kept curve runs through.
    pub(super) fn curved(&self) -> impl Iterator<Item = u32> + '_ {
        self.paths_of.keys().copied()
    }

    fn keep_curve(&mut self, path: Vec<u32>, curve: Curve) {
        for &province in &path {
            self.paths_of.entry(province).or_default().insert(path.clone());
        }
        self.curves.insert(path, curve);
    }

    /// Drops the curves through `provinces`; whether any was dropped.
    fn forget_curves(&mut self, provinces: impl IntoIterator<Item = u32>) -> bool {
        let mut dropped = false;
        for province in provinces {
            for path in self.paths_of.remove(&province).unwrap_or_default() {
                dropped |= self.curves.remove(&path).is_some();
            }
        }
        dropped
    }

    /// A batch taken whole: each changed row read against the last plan, the kept walks it
    /// changed forgotten; the rails' redrawn curves dropped.
    pub(super) fn take(&mut self, mut batch: Batch) {
        self.counted = 0;
        self.moved = [0; 2];
        let forget = std::mem::take(&mut batch.forget);
        let moved = self.store.take(batch);
        let mut clear = false;
        let any = !moved.restated.is_empty()
            || moved.reserved
            || [moved.countries.len(), moved.links.len(), moved.appearance.len(), moved.tokens.len(), moved.provinces.len(), moved.nodes.len(), moved.inventory.len(), moved.variants.len(), moved.edges.len(), moved.cooldowns.len()].iter().any(|&len| len > 0);
        // A refused plan is made again at any change: it reads no rows to compare against.
        if any && self.refused.is_some() {
            self.structural = true;
        }
        for &table in &moved.restated {
            // Every kept walk goes, and a restated census or Countries is counted again whole.
            self.structural = true;
            clear = true;
            self.stats.timed[5] += 1;
            if matches!(table, TableId::Countries | TableId::SupplyNodes) {
                self.census.recount = true;
            }
        }
        for (key, old) in moved.countries {
            let new = self.store.countries.get(key);
            if old.as_ref().map(|country| country.slot) != new.map(|country| country.slot) {
                self.census.country_natives.insert(key as u32);
                clear = true;
            }
            self.structural_by(TableId::Countries);
        }
        if !moved.links.is_empty() {
            self.structural = true;
            clear = true;
        }
        if !moved.appearance.is_empty() {
            self.structural_by(TableId::CountryAppearance);
        }
        if moved.reserved {
            self.structural = true;
        }
        for (key, _) in moved.tokens {
            if self.queried.borrow().tokens.contains(&(key as u32)) {
                self.structural = true;
            }
        }
        for (key, _) in moved.variants {
            if self.queried.borrow().variants.contains(&(key as u64)) {
                self.structural_by(TableId::VariantDefinitions);
            }
        }
        for (key, old) in moved.inventory {
            if self.country == Some(key as u32) {
                self.structural_by(TableId::TrainInventory);
                if old.map(|row| row.access) != self.store.inventory.get(key).map(|row| row.access.clone()) {
                    clear = true;
                }
            }
        }
        for (table, rows) in [(TableId::Provinces, moved.provinces.iter().map(|&(key, _)| key).collect::<Vec<_>>()), (TableId::RailCooldowns, moved.cooldowns.iter().map(|&(key, _)| key).collect())] {
            for key in rows {
                let province = key as u32;
                if self.queried.borrow().provinces.contains(&province) {
                    self.structural_by(table);
                }
                self.cache.forget_provinces([province]);
            }
        }
        for (key, _) in moved.edges {
            let pair = ((key >> 32) as u32, key as u32);
            if self.queried.borrow().pairs.contains(&pair) {
                self.structural_by(TableId::RailEdges);
            }
            self.cache.forget_pairs([pair]);
        }
        for (key, old) in moved.nodes {
            let new = self.store.nodes.get(key);
            let census = |node: Option<&Node>| node.map(|node| (node.province, node.kind));
            let counted = census(old.as_ref()) != census(new);
            let supply = matches!((&old, new), (Some(old), Some(new)) if old.same_but_supply(new));
            let provinces: Vec<u32> = old.iter().chain(new).map(|node| node.province).collect();
            if counted {
                self.census.touched.insert(key);
            }
            if self.country != Some((key >> 32) as u32) {
                continue;
            }
            self.moved[usize::from(!supply)] += 1;
            if supply {
                self.reweigh = true;
            } else {
                self.structural_by(TableId::SupplyNodes);
                self.cache.forget_provinces(provinces);
            }
        }
        if clear {
            self.cache.clear();
        }
        let dropped = if forget.all {
            let any = !self.curves.is_empty();
            self.curves.clear();
            self.paths_of.clear();
            any
        } else {
            self.forget_curves(forget.provinces)
        };
        if dropped {
            self.stats.why[8] += 1;
            self.structural = true;
        }
    }

    /// The census again, then the plan: made again when a change reached what it read, its
    /// weights replaced when only its nodes' supply moved, else left as it is.
    pub(super) fn make(&mut self, carry: bool) -> Made {
        let started = Instant::now();
        let counted = self.census.settle(&self.store);
        self.stats.timed[0] += started.elapsed().as_micros() as u64;
        for province in counted.moved {
            self.cache.forget_provinces([province]);
            if self.queried.borrow().provinces.contains(&province) {
                self.structural = true;
            }
        }
        self.structural |= counted.refused;
        if !self.structural {
            if !self.reweigh {
                self.stats.marked[1] += 1;
                return Made::Quiet;
            }
            if self.refused.is_none()
                && let Some((keys, weights)) = self.weights()
                && keys == self.weighted
            {
                if weights == self.weights {
                    self.reweigh = false;
                    self.stats.marked[1] += 1;
                    return Made::Quiet;
                }
                if self.plan.set_weights(weights.clone()) {
                    (self.weights, self.reweigh, self.how) = (weights, false, "weights");
                    self.stats.marked[3] += 1;
                    return Made::Plan;
                }
            }
            // The weighted nodes themselves changed: the plan is made again, and stays owed through
            // a wait for its curves until it is made (`finish`).
            self.structural = true;
        }
        self.build(carry)
    }

    /// [`Self::make`] until it is made, the curves it wants made here from `network`.
    pub(super) fn make_now(&mut self, network: &RailNetwork, carry: bool) -> Made {
        loop {
            match self.make(carry) {
                Made::Wants(paths) => {
                    for path in paths {
                        let started = Instant::now();
                        let curve = self.content.curve(&path, network);
                        self.stats.timed[3] += started.elapsed().as_micros() as u64;
                        self.stats.marked[2] += 1;
                        self.keep_curve(path, curve);
                    }
                }
                made => return made,
            }
        }
    }

    fn finish(&mut self) {
        self.structural = false;
        self.reweigh = false;
        self.stats.marked[0] += 1;
    }

    /// The plan made from what the maker holds, the presentation taken over when `carry`; the
    /// paths whose curves it lacks, when any.
    fn build(&mut self, carry: bool) -> Made {
        let Some(ours) = self.ours else {
            self.finish();
            self.plan = TrainPlan::default();
            self.refused = Some("no viewing country".into());
            return Made::Plan;
        };
        // A node taken from the cache reads nothing again, so what the cached nodes read stays queried.
        if self.cache.held().0 == 0 {
            *self.queried.borrow_mut() = Queried::default();
        }
        let content = Arc::clone(&self.content);
        let mut wanted = Vec::new();
        let started = Instant::now();
        let mut cache = std::mem::take(&mut self.cache);
        let built = {
            let curves = &self.curves;
            let mut curve = |path: &[u32]| -> io::Result<Curve> {
                if let Some(curve) = curves.get(path) {
                    return Ok(curve.clone());
                }
                wanted.push(path.to_vec());
                Ok(None)
            };
            content.plan(&self.rows(), ours, &mut curve, &mut cache)
        };
        self.cache = cache;
        self.stats.timed[1] += started.elapsed().as_micros() as u64;
        if built.is_ok() && !wanted.is_empty() {
            wanted.sort_unstable();
            wanted.dedup();
            return Made::Wants(wanted);
        }
        self.finish();
        self.how = "built";
        match built {
            Ok(mut next) => {
                if carry && self.refused.is_none() {
                    let started = Instant::now();
                    next.carry_presentation(&mut self.plan, Continuity::Continue);
                    self.stats.timed[4] += started.elapsed().as_micros() as u64;
                }
                self.plan = next;
                self.refused = None;
                self.country = self.census.natives.range((ours, 0)..=(ours, u32::MAX)).next().map(|&(_, native)| native);
                (self.weighted, self.weights) = self.weights().unwrap_or_default();
            }
            Err(error) => {
                self.plan = TrainPlan::default();
                self.refused = Some(error.to_string());
            }
        }
        Made::Plan
    }

    fn rows(&self) -> PlanRows<'_> {
        PlanRows { store: &self.store, census: &self.census, queried: &self.queried, nodes_ns: &self.nodes_ns }
    }

    /// The plan's weighted nodes: their weights, the hour their rows were last copied and the
    /// supply cells the maker holds of them now, for the proof.
    pub(super) fn shown_nodes(&self) -> Vec<super::ShownNode> {
        self.weighted
            .iter()
            .zip(&self.weights)
            .map(|(&key, &weight)| {
                let node = self.store.nodes.get(key);
                super::ShownNode { key, weight, copied: self.store.copied.get(&key).copied(), available: node.map(|node| node.available), consumed: node.map(|node| node.consumed) }
            })
            .collect()
    }

    /// The plan country's weighted nodes and their weights, in key order: their kind, flag and
    /// supply read, nothing else.
    fn weights(&self) -> Option<(Vec<u128>, Vec<i64>)> {
        let country = self.country?;
        Some(
            self.census
                .by_country
                .get(&country)
                .into_iter()
                .flatten()
                .filter(|&&key| (key as u32) & SupplyNodeRow::VERTEX == 0)
                .filter_map(|&key| {
                    let node = self.store.nodes.get(key)?;
                    let weight = node_weight(&NodeValues { index: key as u32, kind: node.kind, flag: node.flag, available: node.available, consumed: node.consumed, ..NodeValues::default() });
                    (weight != 0).then_some((key, weight))
                })
                .unzip(),
        )
    }

    pub(super) fn held(&self) -> usize {
        let curves: usize = self.curves.iter().map(|(path, curve)| path.capacity() * 4 + curve.as_ref().map_or(0, |curve| curve.0.points.capacity() * 12 + curve.1.capacity() * 4) + 48).sum();
        let [routes, nodes, variants] = self.plan.counts();
        curves
            + self.paths_of.values().map(|paths| paths.len() * 32 + 48).sum::<usize>()
            + self.store.bytes()
            + self.census.bytes()
            + routes * 96
            + nodes * 32
            + variants * 64
            + self.cache.held().1
            + (self.weighted.capacity() + self.weights.capacity()) * 16
    }
}

/// The trains' tables as the plan maker holds them.
#[derive(Clone, Copy)]
pub struct PlanRows<'a> {
    store: &'a Store,
    census: &'a Census,
    queried: &'a RefCell<Queried>,
    nodes_ns: &'a Counter<u64>,
}

impl PlanRows<'_> {
    /// A table's head: the last game hour its cells are at.
    pub fn head(&self, table: TableId) -> Option<Hour> {
        self.store.meta(table).and_then(|meta| meta.head)
    }

    fn whole(&self, table: TableId) -> bool {
        self.store.meta(table).is_some_and(Meta::complete)
    }

    /// The core table wholly recorded, or the hour refused in today's words.
    fn core(&self, table: TableId) -> io::Result<()> {
        let meta = self.store.meta(table).filter(|meta| meta.held).ok_or_else(|| io::Error::other(format!("{table:?} is not read yet")))?;
        if meta.status != Some(Status::Complete) {
            let why = meta.refusal.clone().unwrap_or_else(|| format!("{table:?} is not wholly recorded at this hour"));
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{why}; the core tables {CORE} carry the political map itself, so this hour cannot be drawn")));
        }
        Ok(())
    }

    /// The natives of canonical `slot` in key order.
    fn natives(&self, slot: u32) -> Vec<u32> {
        self.census.natives.range((slot, 0)..=(slot, u32::MAX)).map(|&(_, native)| native).collect()
    }

    /// ReservedCountryNaming's singleton row, or why there is none.
    fn reserved(&self) -> Result<&Reserved, String> {
        let meta = &self.store.reserved.meta;
        if !meta.complete() {
            let why = meta.refusal.clone().filter(|_| meta.held).unwrap_or_else(|| "it is not read".into());
            return Err(format!("ReservedCountryNaming is unavailable this hour: {why}"));
        }
        let mut live = self.store.reserved.iter();
        match (live.next(), live.next()) {
            (Some((_, row)), None) => Ok(row),
            _ => Err("ReservedCountryNaming has no singleton row for country zero".into()),
        }
    }
}

impl CreatorRows for PlanRows<'_> {
    fn link(&self, raw: u32) -> Option<u32> {
        self.store.links.row(u128::from(raw)).copied().flatten()
    }

    fn country(&self, slot: u32) -> Option<(u32, Vec<u8>)> {
        let native = *self.natives(slot).last()?;
        let country = self.store.countries.row(u128::from(native))?;
        Some((native, country.tag.clone()?))
    }

    fn appearance(&self, country: u32) -> Option<CreatorAppearance> {
        let row = self.store.appearance.row(u128::from(country))?;
        Some(CreatorAppearance {
            original_tag_reference: recorded(row.reference),
            original_tag: decoded::<Option<Vec<u8>>>(&row.original).unwrap_or(Recorded::NotRecorded),
            graphical_culture: row.culture.clone()?,
        })
    }

    fn appearance_complete(&self) -> bool {
        self.whole(TableId::CountryAppearance)
    }

    fn reference(&self, slot: u32) -> Result<i32, String> {
        if slot == 0 {
            let reserved = self.reserved()?;
            return installed_content_country_names::country_names::reserved_reference_of(&recorded(reserved.reference));
        }
        let countries_complete = self.whole(TableId::Countries);
        let native = *self.natives(slot).last().ok_or_else(|| {
            if countries_complete { format!("country slot {slot} has no Countries row") } else { format!("Countries is incomplete this hour: country slot {slot} is unknown") }
        })?;
        let appearance = self.appearance(native).ok_or_else(|| {
            if self.appearance_complete() { format!("country slot {slot} has no CountryAppearance row") } else { format!("CountryAppearance is incomplete this hour: country slot {slot} is unknown") }
        })?;
        installed_content_country_names::country_names::original_or_own_reference_of(&appearance.original_tag_reference, native)
    }

    fn reserved_tag(&self) -> Result<String, String> {
        let reserved = self.reserved()?;
        let reference = match reserved.reference {
            Cell::Value(reference) => reference,
            Cell::Missing => return Err("ReservedCountryNaming.original_tag_reference capture refused".into()),
            Cell::Unrecorded => return Err("ReservedCountryNaming.original_tag_reference not recorded".into()),
        };
        let bytes = if reference > 0 {
            decoded::<Option<Vec<u8>>>(&reserved.original)?.require("ReservedCountryNaming.original_tag")?.clone().ok_or("ReservedCountryNaming.original_tag capture refused")?
        } else {
            reserved.tag.clone().ok_or("the reserved tag is not named at the head")?
        };
        String::from_utf8(bytes).map_err(|_| "reserved original-or-own tag is not UTF-8".into())
    }
}

impl Fallback for PlanRows<'_> {
    fn emits(&self, parent: u32, province: u32) -> io::Result<bool> {
        let contains = |province: u32| -> io::Result<bool> {
            self.queried.borrow_mut().provinces.insert(province);
            let invalid = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
            let controller = self.store.provinces.row(u128::from(province)).ok_or_else(|| invalid("supply fallback endpoint absent from Provinces"))?.unwrap_or(0);
            let slot = self.link(controller).ok_or_else(|| invalid("supply fallback controller absent from CountryLinks"))?;
            Ok(self.census.subtype_zero.contains_key(&(slot, province)))
        };
        let source = contains(parent)?;
        let destination = contains(province)?;
        Ok(source && destination)
    }
}

impl TrainRows for PlanRows<'_> {
    fn first_country(&self, slot: u32) -> io::Result<Option<u32>> {
        self.core(TableId::Countries)?;
        Ok(self.natives(slot).first().copied())
    }

    fn complete(&self) -> io::Result<bool> {
        Ok(WHOLE.iter().all(|&table| self.whole(table)))
    }

    fn inventory(&self, country: u32) -> io::Result<Option<(u32, Vec<TrainAllocation>, SparseBytes)>> {
        let Some(row) = self.store.inventory.row(u128::from(country)) else { return Ok(None) };
        let invalid = |why: String| io::Error::new(io::ErrorKind::InvalidData, why);
        let allocations = plain::<Vec<TrainAllocation>>(&row.allocations, "TrainInventory.allocations").map_err(invalid)?;
        let access = plain::<SparseBytes>(&row.access, "TrainInventory.controller_access").map_err(invalid)?;
        Ok(Some((row.node_count, allocations, access)))
    }

    fn links_ready(&self) -> io::Result<()> {
        self.core(TableId::CountryLinks)
    }

    fn variant(&self, id: u64) -> Option<(u32, u32)> {
        self.queried.borrow_mut().variants.insert(id);
        self.store.variants.row(u128::from(id)).copied()
    }

    fn token(&self, token: u32) -> Result<String, String> {
        self.queried.borrow_mut().tokens.insert(token);
        if token == 0 {
            return Err(TokenError::Rejected.to_string());
        }
        let Some(literal) = self.store.tokens.row(u128::from(token)) else {
            return Err(if self.whole(TableId::LexicalTokens) { TokenError::Absent(token) } else { TokenError::Missing(token) }.to_string());
        };
        literal
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .filter(|name| !name.contains('\0'))
            .map(str::to_owned)
            .ok_or_else(|| TokenError::NotText(token).to_string())
    }

    fn provinces_ready(&self) -> io::Result<()> {
        self.core(TableId::Provinces)
    }

    fn fallback(&self) -> io::Result<Box<dyn Fallback + '_>> {
        let invalid = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
        if self.census.errors[0] > 0 {
            return Err(invalid("SupplyNodes has an unknown node kind"));
        }
        if self.census.errors[1] > 0 {
            return Err(invalid("SupplyNodes country absent from Countries"));
        }
        Ok(Box::new(*self))
    }

    fn nodes(&self, country: u32) -> io::Result<Vec<NodeValues>> {
        let started = Instant::now();
        let nodes = self
            .census
            .by_country
            .get(&country)
            .into_iter()
            .flatten()
            .filter_map(|&key| {
                let node = self.store.nodes.get(key)?;
                Some(NodeValues {
                    index: key as u32,
                    province: node.province,
                    kind: node.kind,
                    flag: node.flag,
                    available: node.available,
                    consumed: node.consumed,
                    parent: node.parent,
                    route_from: node.route_from,
                    route_to: node.route_to,
                    break_after: node.break_after,
                    path: node.path.clone(),
                })
            })
            .collect();
        self.nodes_ns.set(self.nodes_ns.get() + started.elapsed().as_nanos() as u64);
        Ok(nodes)
    }

    fn allowed(&self, province: u32, access: &SparseBytes) -> bool {
        self.queried.borrow_mut().provinces.insert(province);
        let Some(controller) = self.store.provinces.row(u128::from(province)) else { return false };
        let controller = controller.unwrap_or(0);
        let closed = self.store.cooldowns.row(u128::from(province)).is_some_and(|&cooldown| cooldown > 0);
        let slot = if controller == 0 { Some(0) } else { self.link(controller) };
        !closed && slot.and_then(|slot| access.get(slot)).is_some_and(|byte| byte != 0)
    }

    fn rail(&self, from: u32, to: u32) -> bool {
        self.queried.borrow_mut().pairs.extend([(from, to), (to, from)]);
        let edges = &self.store.edges;
        edges.get(u128::from(from) << 32 | u128::from(to)).is_some() || edges.get(u128::from(to) << 32 | u128::from(from)).is_some()
    }
}
