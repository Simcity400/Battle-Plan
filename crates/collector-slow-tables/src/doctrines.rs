//! Baseline from the existing doctrine-status census. Native writer callbacks
//! copy subsequent changes while the writer still owns its exact objects.
//! The hourly job only swaps owned event buffers; spool conversion follows no
//! pointers and shares quiet replay allocations. Missing observation refuses
//! the affected data and never enables periodic native reads.
use collector_observers::doctrine_changes::{self as changes, Domain};
use engine_allocation as a;
use engine_land_inputs::{country_tactic_unlocks, doctrine_state as n};
use engine_native::{
    Memory,
    cpdx_array::{self, Array},
};
use replay_data::rows::*;
use replay_frame::frame::TableInput;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const COST_CONSUMPTION_LIMIT: i64 = 100_000;
pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::DoctrineCountries,
    TableId::DoctrineFolders,
    TableId::DoctrineTracks,
    TableId::DoctrineLeaderMastery,
    TableId::DoctrineCostReductions,
    TableId::DoctrineMasteryGrants,
    TableId::DoctrineEquipmentBonuses,
];
const DOMAINS: [Domain; 11] = [
    Domain::Status,
    Domain::Folder,
    Domain::Gains,
    Domain::Track,
    Domain::Costs,
    Domain::Grants,
    Domain::Equipment,
    Domain::GrantExpiry,
    Domain::Bank,
    Domain::Progress,
    Domain::StatusFolders,
];
type Result<T> = std::result::Result<T, &'static str>;
fn alloc<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|_| "doctrine allocation failed")
}
fn put<K: std::hash::Hash + Eq, V>(map: &mut HashMap<K, V>, key: K, value: V) -> Result<()> {
    if !map.contains_key(&key) {
        alloc(a::reserve_map(map, 1))?;
    }
    map.insert(key, value);
    Ok(())
}
fn key2(slot: u32, ordinal: u32) -> u128 {
    (u128::from(slot) << 32) | u128::from(ordinal)
}
fn key3(slot: u32, folder: u32, ordinal: u32) -> u128 {
    (u128::from(slot) << 64) | (u128::from(folder) << 32) | u128::from(ordinal)
}
fn key4(slot: u32, folder: u32, track: u32, ordinal: u32) -> u128 {
    (u128::from(slot) << 96)
        | (u128::from(folder) << 64)
        | (u128::from(track) << 32)
        | u128::from(ordinal)
}
fn element(array: Array, index: u32, stride: usize) -> Result<usize> {
    array
        .element(index, stride)
        .ok_or("doctrine array element outside captured bounds")
}

enum Action<R> {
    Set(R),
    Remove(u128),
    // Only the cost journal uses this prepared scalar event. Immutable row
    // payloads stay in the snapshot; compaction is a spool-only operation.
    ConsumeCost {
        slot: u32,
        folder: Option<u32>,
        count: u64,
    },
}
struct Rows<R: RowCodec> {
    rows: Vec<R>,
    indices: HashMap<u128, usize>,
    shared: Option<Arc<TableRows>>,
    dirty: bool,
    actions: Vec<Action<R>>,
    marks: HashMap<u128, (u64, usize)>,
    epoch: u64,
    record: bool,
    shared_bytes: usize,
    row_heap: usize,
    action_heap: usize,
    spares: Vec<Vec<Action<R>>>,
    prepared: Option<usize>,
}
impl<R: RowCodec> Default for Rows<R> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            indices: HashMap::new(),
            shared: None,
            dirty: true,
            actions: Vec::new(),
            marks: HashMap::new(),
            epoch: 1,
            record: true,
            shared_bytes: 0,
            row_heap: 0,
            action_heap: 0,
            spares: Vec::new(),
            prepared: None,
        }
    }
}
impl<R: RowCodec> Rows<R> {
    fn set(&mut self, row: R) -> Result<()> {
        self.write(row, false)
    }
    fn write(&mut self, row: R, force: bool) -> Result<()> {
        let key = row.key();
        if !force
            && self
                .indices
                .get(&key)
                .is_some_and(|&index| self.rows[index] == row)
        {
            return Ok(());
        }
        if self.record {
            let event = Action::Set(alloc(row.try_clone_validated())?);
            if let Some(&(epoch, index)) = self.marks.get(&key)
                && epoch == self.epoch
            {
                self.action_heap -= match &self.actions[index] {
                    Action::Set(row) => row.heap_bytes(),
                    Action::Remove(_) | Action::ConsumeCost { .. } => 0,
                };
                self.action_heap += row.heap_bytes();
                self.actions[index] = event;
            } else {
                alloc(a::reserve(&mut self.actions, 1))?;
                put(&mut self.marks, key, (self.epoch, self.actions.len()))?;
                self.action_heap += row.heap_bytes();
                self.actions.push(event);
            }
        }
        if let Some(&index) = self.indices.get(&key) {
            if self.rows[index] != row {
                self.row_heap -= self.rows[index].heap_bytes();
                self.row_heap += row.heap_bytes();
                self.rows[index] = row;
                self.dirty = true;
            }
        } else {
            alloc(a::reserve(&mut self.rows, 1))?;
            put(&mut self.indices, key, self.rows.len())?;
            self.row_heap += row.heap_bytes();
            self.rows.push(row);
            self.dirty = true;
            if self.prepared.is_some_and(|count| count < self.rows.len()) {
                self.prepare_baseline()?;
            }
        }
        Ok(())
    }
    fn remove(&mut self, key: u128) -> Result<()> {
        if !self.indices.contains_key(&key) {
            return Ok(());
        }
        if self.record {
            if let Some(&(epoch, index)) = self.marks.get(&key)
                && epoch == self.epoch
            {
                self.action_heap -= match &self.actions[index] {
                    Action::Set(row) => row.heap_bytes(),
                    Action::Remove(_) | Action::ConsumeCost { .. } => 0,
                };
                self.actions[index] = Action::Remove(key);
            } else {
                alloc(a::reserve(&mut self.actions, 1))?;
                put(&mut self.marks, key, (self.epoch, self.actions.len()))?;
                self.actions.push(Action::Remove(key));
            }
        }
        if let Some(index) = self.indices.remove(&key) {
            self.row_heap -= self.rows[index].heap_bytes();
            self.rows.swap_remove(index);
            if let Some(row) = self.rows.get(index) {
                *self.indices.get_mut(&row.key()).expect("indexed row") = index;
            }
            self.dirty = true;
        }
        Ok(())
    }
    fn prepare_baseline(&mut self) -> Result<()> {
        // Baseline or structural insertion only. Prepare replacement journals for
        // every in-flight frame, so subsequent scalar events never grow storage.
        self.prepare_capacity(self.rows.len())
    }
    fn prepare_capacity(&mut self, count: usize) -> Result<()> {
        self.spares.clear();
        let additional = count.saturating_sub(self.actions.len());
        alloc(a::reserve(&mut self.actions, additional))?;
        for _ in 0..collector_extended_support::hourly::BUFFERS {
            let spare = alloc(a::vec(count))?;
            alloc(a::reserve(&mut self.spares, 1))?;
            self.spares.push(spare);
        }
        self.prepared = Some(count);
        Ok(())
    }
    fn handoff(&mut self, output: &mut Vec<Action<R>>) -> Result<()> {
        if !output.is_empty() {
            return Err("doctrine event buffer was not consumed");
        }
        if output.capacity() < self.prepared.unwrap_or(0) {
            let mut spare = self
                .spares
                .pop()
                .ok_or("doctrine prepared journals exhausted")?;
            std::mem::swap(output, &mut spare);
        }
        std::mem::swap(output, &mut self.actions);
        self.action_heap = 0;
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or("doctrine event epoch exhausted")?;
        Ok(())
    }
    fn apply(&mut self, actions: &mut Vec<Action<R>>) -> Result<()> {
        for action in actions.drain(..) {
            match action {
                Action::Set(row) => self.set(row)?,
                Action::Remove(key) => self.remove(key)?,
                Action::ConsumeCost { .. } => return Err("cost event in another doctrine journal"),
            }
        }
        // Spool-only preparation of the recycled event vector.
        alloc(a::reserve(actions, self.rows.len()))?;
        Ok(())
    }
    fn publish(&mut self, wrap: impl FnOnce(Vec<R>) -> TableRows) -> Result<Arc<TableRows>> {
        if self.dirty || self.shared.is_none() {
            let mut rows = alloc(a::vec(self.rows.len()))?;
            let mut bytes = rows.capacity() * std::mem::size_of::<R>();
            for row in &self.rows {
                let row = alloc(row.try_clone_validated())?;
                bytes += row.heap_bytes();
                rows.push(row);
            }
            self.shared_bytes = bytes;
            rows.sort_unstable_by_key(RowCodec::key);
            self.shared = Some(Arc::new(wrap(rows)));
            self.dirty = false;
        }
        Ok(Arc::clone(self.shared.as_ref().expect("published rows")))
    }
    fn bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<R>()
            + self.row_heap
            + self.indices.capacity() * (std::mem::size_of::<(u128, usize)>() + 1)
            + self.shared_bytes
            + self.actions.capacity() * std::mem::size_of::<Action<R>>()
            + self.action_heap
            + self
                .spares
                .iter()
                .map(|spare| spare.capacity() * std::mem::size_of::<Action<R>>())
                .sum::<usize>()
            + self.spares.capacity() * std::mem::size_of::<Vec<Action<R>>>()
            + self.marks.capacity() * (std::mem::size_of::<(u128, (u64, usize))>() + 1)
    }
}

impl Rows<DoctrineCostReductionRow> {
    fn apply_costs(&mut self, actions: &mut Vec<Action<DoctrineCostReductionRow>>) -> Result<()> {
        for action in actions.drain(..) {
            match action {
                Action::Set(row) => self.set(row)?,
                Action::Remove(key) => self.remove(key)?,
                Action::ConsumeCost {
                    slot,
                    folder,
                    count,
                } => {
                    for _ in 0..count {
                        self.consume_cost(slot, folder)?;
                    }
                }
            }
        }
        alloc(a::reserve(actions, self.rows.len()))?;
        Ok(())
    }
    fn consume_cost(&mut self, slot: u32, folder: Option<u32>) -> Result<()> {
        let mut old = 0;
        let mut sum = 0i64;
        let mut consuming = true;
        while let Some(&index) = self.indices.get(&key2(slot, old)) {
            let row = &mut self.rows[index];
            if consuming && row.folder == folder {
                row.uses = row.uses.wrapping_sub(1);
                self.dirty = true;
                sum = sum.wrapping_add(row.cost_factor);
                consuming = sum > COST_CONSUMPTION_LIMIT;
            }
            old += 1;
        }
        let mut next = 0;
        for ordinal in 0..old {
            let row = &self.rows[self.indices[&key2(slot, ordinal)]];
            if row.uses <= 0 {
                continue;
            }
            if next != ordinal {
                // This is the spool worker, never the shared live-store mutex.
                let mut row = alloc(row.try_clone_validated())?;
                row.ordinal = next;
                self.set(row)?;
            }
            next += 1;
        }
        for ordinal in next..old {
            self.remove(key2(slot, ordinal))?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Pending {
    country_rows: Vec<Action<DoctrineCountryRow>>,
    folder_rows: Vec<Action<DoctrineFolderRow>>,
    track_rows: Vec<Action<DoctrineTrackRow>>,
    leader_rows: Vec<Action<DoctrineLeaderMasteryRow>>,
    costs: Vec<Action<DoctrineCostReductionRow>>,
    grants: Vec<Action<DoctrineMasteryGrantRow>>,
    equipment: Vec<Action<DoctrineEquipmentBonusRow>>,
}
#[derive(Default)]
struct SharedProjection {
    rows: Mutex<Projection>,
    bytes: AtomicUsize,
    failed: AtomicBool,
}

pub struct Projection {
    country_rows: Rows<DoctrineCountryRow>,
    folder_rows: Rows<DoctrineFolderRow>,
    track_rows: Rows<DoctrineTrackRow>,
    leader_rows: Rows<DoctrineLeaderMasteryRow>,
    costs: Rows<DoctrineCostReductionRow>,
    grants: Rows<DoctrineMasteryGrantRow>,
    equipment: Rows<DoctrineEquipmentBonusRow>,
}
impl Default for Projection {
    fn default() -> Self {
        Self {
            country_rows: Rows {
                record: false,
                ..Default::default()
            },
            folder_rows: Rows {
                record: false,
                ..Default::default()
            },
            track_rows: Rows {
                record: false,
                ..Default::default()
            },
            leader_rows: Rows {
                record: false,
                ..Default::default()
            },
            costs: Rows {
                record: false,
                ..Default::default()
            },
            grants: Rows {
                record: false,
                ..Default::default()
            },
            equipment: Rows {
                record: false,
                ..Default::default()
            },
        }
    }
}
pub struct Frame {
    pub store_bytes: usize,
    pending_heap: usize,
    pending: Pending,
    projection: Option<Arc<SharedProjection>>,
    pub counts: [usize; N],
    rows: Option<[Arc<TableRows>; N]>,
    pub refusal: &'static str,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            store_bytes: 0,
            pending_heap: 0,
            pending: Pending::default(),
            projection: None,
            counts: [0; N],
            rows: None,
            refusal: "doctrine state not acquired",
        }
    }
}
impl Frame {
    pub fn allocated_bytes(&self) -> usize {
        self.pending_heap
            + self.pending.country_rows.capacity()
                * std::mem::size_of::<Action<DoctrineCountryRow>>()
            + self.pending.folder_rows.capacity() * std::mem::size_of::<Action<DoctrineFolderRow>>()
            + self.pending.track_rows.capacity() * std::mem::size_of::<Action<DoctrineTrackRow>>()
            + self.pending.leader_rows.capacity()
                * std::mem::size_of::<Action<DoctrineLeaderMasteryRow>>()
            + self.pending.costs.capacity()
                * std::mem::size_of::<Action<DoctrineCostReductionRow>>()
            + self.pending.grants.capacity()
                * std::mem::size_of::<Action<DoctrineMasteryGrantRow>>()
            + self.pending.equipment.capacity()
                * std::mem::size_of::<Action<DoctrineEquipmentBonusRow>>()
    }
    pub fn reset(&mut self) {
        self.projection = None;
        self.rows = None;
        self.counts = [0; N];
        self.refusal = "doctrine state not acquired";
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.reset();
        self.refusal = reason;
    }
    pub fn complete(&self) -> bool {
        self.projection.is_some()
    }
    pub fn inputs(&mut self) -> [TableInput; N] {
        let result = (|| -> Result<()> {
            if self.rows.is_none() {
                let Some(shared) = &self.projection else {
                    return Err(self.refusal);
                };
                if shared.failed.load(Ordering::Acquire) {
                    return Err(
                        "doctrine projection requires full refresh after failed conversion",
                    );
                }
                let mut projection = shared
                    .rows
                    .lock()
                    .map_err(|_| "doctrine projection poisoned")?;
                projection
                    .country_rows
                    .apply(&mut self.pending.country_rows)?;
                projection
                    .folder_rows
                    .apply(&mut self.pending.folder_rows)?;
                projection.track_rows.apply(&mut self.pending.track_rows)?;
                projection
                    .leader_rows
                    .apply(&mut self.pending.leader_rows)?;
                projection.costs.apply_costs(&mut self.pending.costs)?;
                if projection.costs.rows.len() != self.counts[4] {
                    return Err("doctrine cost consumption count differs from native result");
                }
                projection.grants.apply(&mut self.pending.grants)?;
                projection.equipment.apply(&mut self.pending.equipment)?;
                self.rows = Some([
                    projection
                        .country_rows
                        .publish(TableRows::DoctrineCountries)?,
                    projection.folder_rows.publish(TableRows::DoctrineFolders)?,
                    projection.track_rows.publish(TableRows::DoctrineTracks)?,
                    projection
                        .leader_rows
                        .publish(TableRows::DoctrineLeaderMastery)?,
                    projection
                        .costs
                        .publish(TableRows::DoctrineCostReductions)?,
                    projection
                        .grants
                        .publish(TableRows::DoctrineMasteryGrants)?,
                    projection
                        .equipment
                        .publish(TableRows::DoctrineEquipmentBonuses)?,
                ]);
                shared.bytes.store(
                    projection.country_rows.bytes()
                        + projection.folder_rows.bytes()
                        + projection.track_rows.bytes()
                        + projection.leader_rows.bytes()
                        + projection.costs.bytes()
                        + projection.grants.bytes()
                        + projection.equipment.bytes(),
                    Ordering::Relaxed,
                );
                self.pending_heap = 0;
            }
            Ok(())
        })();
        if let Err(reason) = result {
            if let Some(shared) = &self.projection {
                shared.failed.store(true, Ordering::Release);
            }
            self.refusal = reason;
            // Spool path: recycle any partially handed-off events after a refusal.
            self.pending.country_rows.clear();
            self.pending.folder_rows.clear();
            self.pending.track_rows.clear();
            self.pending.leader_rows.clear();
            self.pending.costs.clear();
            self.pending.grants.clear();
            self.pending.equipment.clear();
            self.pending_heap = 0;
            return std::array::from_fn(|_| crate::table_conversion::unavailable(reason).into());
        }
        std::array::from_fn(|i| {
            TableInput::SharedComplete(Arc::clone(
                &self.rows.as_ref().expect("projected doctrine events")[i],
            ))
        })
    }
}
#[derive(Default)]
struct Country {
    pointer: usize,
    country: u32,
    folders: Vec<usize>,
    costs: u32,
    live_costs: u32,
    grants: u32,
    expiry_events: u64,
    equipment: u32,
}
struct Folder {
    slot: u32,
    ordinal: u32,
    owner: usize,
    tracks: Vec<usize>,
}
#[derive(Clone, Copy)]
struct Track {
    slot: u32,
    folder: u32,
    ordinal: u32,
    owner: usize,
    parent: usize,
    leaders: u32,
}

#[derive(Default)]
pub struct Store {
    base: usize,
    root: (usize, u32),
    ready: bool,
    failed: bool,
    epochs: [u64; DOMAINS.len()],
    countries: Vec<Country>,
    owners: HashMap<usize, u32>,
    folders: HashMap<usize, Folder>,
    tracks: HashMap<usize, Track>,
    nested_bytes: usize,
    text: Vec<u8>,
    // First cleanup is ordered; later positive-use consumption commutes by
    // folder. Marks and replacement journals are prepared at snapshots only.
    cost_first: Vec<(u64, usize)>,
    cost_consumptions: HashMap<(u32, Option<u32>), (u64, usize)>,
    cost_count: usize,
    projection: Arc<SharedProjection>,
    country_rows: Rows<DoctrineCountryRow>,
    folder_rows: Rows<DoctrineFolderRow>,
    track_rows: Rows<DoctrineTrackRow>,
    leader_rows: Rows<DoctrineLeaderMasteryRow>,
    costs: Rows<DoctrineCostReductionRow>,
    grants: Rows<DoctrineMasteryGrantRow>,
    equipment: Rows<DoctrineEquipmentBonusRow>,
}
impl Store {
    pub fn forget(&mut self) {
        self.failed = true;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.capacity() * std::mem::size_of::<Country>()
            + self.nested_bytes
            + self.owners.capacity() * (std::mem::size_of::<(usize, u32)>() + 1)
            + self.folders.capacity() * (std::mem::size_of::<(usize, Folder)>() + 1)
            + self.tracks.capacity() * (std::mem::size_of::<(usize, Track)>() + 1)
            + self.text.capacity()
            + self.cost_first.capacity() * std::mem::size_of::<(u64, usize)>()
            + self.cost_consumptions.capacity()
                * (std::mem::size_of::<((u32, Option<u32>), (u64, usize))>() + 1)
            + self.country_rows.bytes()
            + self.folder_rows.bytes()
            + self.track_rows.bytes()
            + self.leader_rows.bytes()
            + self.costs.bytes()
            + self.grants.bytes()
            + self.equipment.bytes()
            + self.projection.bytes.load(Ordering::Relaxed)
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: (usize, u32),
        full: bool,
        frame: &mut Frame,
    ) -> Result<()> {
        let result = self.acquire(memory, base, root, full, frame);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: (usize, u32),
        full: bool,
        frame: &mut Frame,
    ) -> Result<()> {
        let mut stamps = [(0, 0); DOMAINS.len()];
        for (i, &domain) in DOMAINS.iter().enumerate() {
            stamps[i] = changes::token(domain).ok_or("doctrine writers are not observed")?;
        }
        if (self.failed || self.projection.failed.load(Ordering::Acquire)) && !full {
            return Err("doctrine capture requires a full refresh after lost observation");
        }
        if full || !self.ready || root != self.root {
            // Baseline/relocation only. The root is the same header already
            // read and authenticated by CountryTacticUnlocks, including empty slots.
            *self = Self::default();
            self.root = root;
            self.base = base;
            alloc(a::reserve(&mut self.countries, root.1 as usize))?;
            self.countries
                .resize_with(root.1 as usize, Country::default);
            alloc(a::reserve(&mut self.cost_first, root.1 as usize))?;
            self.cost_first.resize(root.1 as usize, (0, 0));
            for slot in 0..root.1 {
                let pointer = root
                    .0
                    .checked_add(
                        (slot as usize)
                            .checked_mul(country_tactic_unlocks::DOCTRINE_STRIDE)
                            .ok_or("doctrine status address overflow")?,
                    )
                    .ok_or("doctrine status address overflow")?;
                n::authenticate(memory, base, pointer, n::STATUS_VTABLE)?;
                let country = n::field(memory, pointer, n::COUNTRY)?;
                self.countries[slot as usize].pointer = pointer;
                self.countries[slot as usize].country = country;
                put(&mut self.owners, pointer, slot)?;
                self.refresh_country(memory, base, slot)?;
            }
            for (i, &domain) in DOMAINS.iter().enumerate() {
                self.epochs[i] = changes::token(domain)
                    .ok_or("doctrine observer unavailable")?
                    .0;
            }
            self.country_rows.prepare_baseline()?;
            self.folder_rows.prepare_baseline()?;
            self.track_rows.prepare_baseline()?;
            self.leader_rows.prepare_baseline()?;
            self.prepare_cost_events()?;
            self.grants.prepare_baseline()?;
            self.equipment.prepare_baseline()?;
            self.ready = true;
        } else {
            for (i, &domain) in DOMAINS.iter().enumerate() {
                if changes::token(domain)
                    .ok_or("doctrine observer unavailable")?
                    .0
                    != self.epochs[i]
                {
                    return Err("doctrine writer epoch invalidated; full refresh required");
                }
            }
        }
        for (i, &domain) in DOMAINS.iter().enumerate() {
            if changes::token(domain) != Some(stamps[i]) {
                return Err("doctrine writer overlapped capture");
            }
        }
        frame.pending_heap = self.country_rows.action_heap
            + self.folder_rows.action_heap
            + self.track_rows.action_heap
            + self.leader_rows.action_heap
            + self.costs.action_heap
            + self.grants.action_heap
            + self.equipment.action_heap;
        self.country_rows.handoff(&mut frame.pending.country_rows)?;
        self.folder_rows.handoff(&mut frame.pending.folder_rows)?;
        self.track_rows.handoff(&mut frame.pending.track_rows)?;
        self.leader_rows.handoff(&mut frame.pending.leader_rows)?;
        self.costs.handoff(&mut frame.pending.costs)?;
        self.grants.handoff(&mut frame.pending.grants)?;
        self.equipment.handoff(&mut frame.pending.equipment)?;
        frame.projection = Some(Arc::clone(&self.projection));
        frame.counts = [
            self.country_rows.rows.len(),
            self.folder_rows.rows.len(),
            self.track_rows.rows.len(),
            self.leader_rows.rows.len(),
            self.cost_count,
            self.grants.rows.len(),
            self.equipment.rows.len(),
        ];
        frame.store_bytes = self.allocated_bytes();
        self.failed = false;
        Ok(())
    }
    fn observe(
        &mut self,
        memory: &impl Memory,
        base: usize,
        domain: Domain,
        pointer: usize,
    ) -> Result<()> {
        if !self.ready || self.failed {
            return Ok(());
        }
        match domain {
            Domain::Status => {
                if let Some(&slot) = self.owners.get(&pointer) {
                    self.refresh_country(memory, base, slot)?;
                }
            }
            Domain::StatusFolders => {
                if let Some(&slot) = self.owners.get(&pointer) {
                    self.refresh_folders(memory, base, slot)?;
                }
            }
            Domain::Folder => self.refresh_folder(memory, base, pointer)?,
            Domain::Gains => self.refresh_gains(memory, base, pointer)?,
            Domain::Bank | Domain::Progress => {
                if let Some(location) = self.tracks.get(&pointer).copied() {
                    // The writer owns this demanded receiver. Baseline, folder
                    // relocation and reset coverage authenticate its lifetime.
                    let index = self.track_rows.indices
                        [&key3(location.slot, location.folder, location.ordinal)];
                    let mut row = self.track_rows.rows[index].clone();
                    if domain == Domain::Bank {
                        row.mastery_bank = n::field(memory, pointer, n::BANK)?;
                    } else {
                        row.mastery = n::field(memory, pointer, n::MASTERY)?;
                        row.rewards = n::field(memory, pointer, n::REWARDS)?;
                    }
                    self.track_rows.set(row)?;
                }
            }
            Domain::Track => self.refresh_track(memory, base, pointer)?,
            Domain::Costs => {
                if let Some(&slot) = self.owners.get(&pointer) {
                    self.refresh_costs(memory, base, slot)?;
                }
            }
            Domain::Grants => {
                if let Some(&slot) = self.owners.get(&pointer) {
                    self.refresh_grants(memory, base, slot, true)?;
                }
            }
            Domain::GrantExpiry => {
                if let Some(&slot) = self.owners.get(&pointer) {
                    self.expire_grants(slot)?;
                }
            }
            Domain::Equipment => {
                if let Some(&slot) = self.owners.get(&pointer) {
                    self.refresh_equipment(memory, base, slot)?;
                }
            }
        }
        Ok(())
    }
    fn remove_track(&mut self, pointer: usize) -> Result<()> {
        if let Some(track) = self.tracks.remove(&pointer) {
            self.track_rows
                .remove(key3(track.slot, track.folder, track.ordinal))?;
            for ordinal in 0..track.leaders {
                self.leader_rows
                    .remove(key4(track.slot, track.folder, track.ordinal, ordinal))?;
            }
        }
        Ok(())
    }
    fn refresh_country(&mut self, memory: &impl Memory, base: usize, slot: u32) -> Result<()> {
        self.refresh_folders(memory, base, slot)?;
        self.refresh_costs(memory, base, slot)?;
        self.refresh_grants(memory, base, slot, false)?;
        self.refresh_equipment(memory, base, slot)
    }
    fn refresh_folders(&mut self, memory: &impl Memory, base: usize, slot: u32) -> Result<()> {
        let country = &mut self.countries[slot as usize];
        let status = n::status(memory, base, country.pointer, country.country)?;
        let mut old = std::mem::take(&mut country.folders);
        let pointer = country.pointer;
        self.country_rows.set(DoctrineCountryRow {
            slot,
            country: country.country,
            expiry_events: country.expiry_events,
        })?;
        for &folder in &old {
            if let Some(folder) = self.folders.remove(&folder) {
                self.nested_bytes -= folder.tracks.capacity() * std::mem::size_of::<usize>();
                for track in folder.tracks {
                    self.remove_track(track)?;
                }
                self.folder_rows.remove(key2(slot, folder.ordinal))?;
            }
        }
        old.clear();
        let old_capacity = old.capacity();
        alloc(a::reserve(&mut old, status.folders.count as usize))?;
        self.nested_bytes += (old.capacity() - old_capacity) * std::mem::size_of::<usize>();
        for ordinal in 0..status.folders.count {
            let folder = element(status.folders, ordinal, n::FOLDER_STRIDE)?;
            put(
                &mut self.folders,
                folder,
                Folder {
                    slot,
                    ordinal,
                    owner: pointer,
                    tracks: Vec::new(),
                },
            )?;
            self.refresh_folder(memory, base, folder)?;
            old.push(folder);
        }
        self.countries[slot as usize].folders = old;
        Ok(())
    }
    fn refresh_folder(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Result<()> {
        let Some(mut location) = self.folders.remove(&pointer) else {
            return Ok(());
        };
        let folder = n::folder(memory, base, pointer, location.owner)?;
        let same_tracks = location.tracks.len() == folder.tracks.count as usize
            && location
                .tracks
                .first()
                .copied()
                .unwrap_or(folder.tracks.data)
                == folder.tracks.data;
        if !same_tracks {
            for track in location.tracks.drain(..) {
                self.remove_track(track)?;
            }
        }
        let mut completed = alloc(a::vec(folder.completed.count as usize))?;
        for ordinal in 0..folder.completed.count {
            completed.push(n::completed(
                memory,
                base,
                element(folder.completed, ordinal, 8)?,
            )?);
        }
        self.folder_rows.set(DoctrineFolderRow {
            slot: location.slot,
            ordinal: location.ordinal,
            folder: folder.token,
            grand_doctrine: folder.grand,
            completed_subdoctrines: completed,
        })?;
        if !same_tracks {
            let old_capacity = location.tracks.capacity();
            alloc(a::reserve(
                &mut location.tracks,
                folder.tracks.count as usize,
            ))?;
            self.nested_bytes +=
                (location.tracks.capacity() - old_capacity) * std::mem::size_of::<usize>();
            for ordinal in 0..folder.tracks.count {
                let track = element(folder.tracks, ordinal, n::TRACK_STRIDE)?;
                put(
                    &mut self.tracks,
                    track,
                    Track {
                        slot: location.slot,
                        folder: location.ordinal,
                        ordinal,
                        owner: location.owner,
                        parent: pointer,
                        leaders: 0,
                    },
                )?;
                self.refresh_track(memory, base, track)?;
                location.tracks.push(track);
            }
        }
        put(&mut self.folders, pointer, location)
    }
    fn refresh_track(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Result<()> {
        let Some(location) = self.tracks.get(&pointer).copied() else {
            return Ok(());
        };
        let is_new = !self.track_rows.indices.contains_key(&key3(
            location.slot,
            location.folder,
            location.ordinal,
        ));
        let track = n::track(memory, base, pointer, location.owner, location.parent)?;
        self.track_rows.set(DoctrineTrackRow {
            slot: location.slot,
            folder: location.folder,
            ordinal: location.ordinal,
            sub_doctrine: track.sub,
            rewards: track.rewards,
            mastery: track.mastery,
            mastery_bank: track.bank,
            daily_mastery: track.daily,
            effective_daily_mastery: track.effective_daily,
        })?;
        if is_new {
            self.refresh_leaders(
                memory,
                base,
                pointer,
                n::array(memory, pointer, n::LEADERS, n::LEADER_STRIDE)?,
            )?;
        }
        Ok(())
    }
    fn refresh_leaders(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        leaders: Array,
    ) -> Result<()> {
        let location = self
            .tracks
            .get(&pointer)
            .copied()
            .ok_or("doctrine track location missing")?;
        for ordinal in 0..leaders.count {
            let (leader, mastery) =
                n::leader(memory, base, element(leaders, ordinal, n::LEADER_STRIDE)?)?;
            self.leader_rows.set(DoctrineLeaderMasteryRow {
                slot: location.slot,
                folder: location.folder,
                track: location.ordinal,
                ordinal,
                leader,
                mastery,
            })?;
        }
        for ordinal in leaders.count..location.leaders {
            self.leader_rows.remove(key4(
                location.slot,
                location.folder,
                location.ordinal,
                ordinal,
            ))?;
        }
        self.tracks
            .get_mut(&pointer)
            .expect("track location")
            .leaders = leaders.count;
        Ok(())
    }
    fn refresh_gains(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Result<()> {
        let Some(folder) = self.folders.get(&pointer) else {
            return Ok(());
        };
        let count = folder.tracks.len();
        for index in 0..count {
            let pointer = self.folders[&pointer].tracks[index];
            let location = self.tracks[&pointer];
            n::authenticate(memory, base, pointer, n::TRACK_VTABLE)?;
            let key = key3(location.slot, location.folder, location.ordinal);
            let row_index = self.track_rows.indices[&key];
            let daily = n::field(memory, pointer, n::DAILY)?;
            if self.track_rows.rows[row_index].daily_mastery != daily {
                let mut row = self.track_rows.rows[row_index].clone();
                row.daily_mastery = daily;
                self.track_rows.set(row)?;
            }
            let leaders = n::array(memory, pointer, n::LEADERS, n::LEADER_STRIDE)?;
            self.refresh_leaders(memory, base, pointer, leaders)?;
        }
        Ok(())
    }
    fn prepare_cost_events(&mut self) -> Result<()> {
        self.costs.prepare_capacity(
            self.costs.actions.len()
                + self.costs.rows.len()
                + self.cost_consumptions.len()
                + self.cost_first.len(),
        )
    }
    fn cancel_cost_consumption(&mut self, mark: (u64, usize)) {
        if mark.0 == self.costs.epoch
            && let Action::ConsumeCost { count, .. } = &mut self.costs.actions[mark.1]
        {
            *count = 0;
        }
    }
    fn observe_cost_consumption(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        folder: usize,
    ) -> Result<()> {
        if !self.ready || self.failed {
            return Ok(());
        }
        let Some(&slot) = self.owners.get(&pointer) else {
            return Ok(());
        };
        let folder = (folder != 0)
            .then(|| n::token(memory, base, folder, n::FOLDER_TEMPLATE_VTABLE))
            .transpose()?;
        let remaining: u32 = n::field(memory, pointer, n::COSTS + cpdx_array::COUNT)?;
        if remaining > i32::MAX as u32 {
            return Err("doctrine cost count invalid");
        }
        self.cost_count = self.cost_count - self.countries[slot as usize].live_costs as usize
            + remaining as usize;
        self.countries[slot as usize].live_costs = remaining;
        let epoch = self.costs.epoch;
        let first = self.cost_first[slot as usize].0 != epoch;
        let mark = if first {
            (0, 0)
        } else if let Some(&mark) = self.cost_consumptions.get(&(slot, folder)) {
            mark
        } else {
            // The first event already removed invalid entries. A folder absent
            // from this snapshot cannot change any remaining reduction.
            return Ok(());
        };
        if mark.0 == epoch {
            let Action::ConsumeCost { count, .. } = &mut self.costs.actions[mark.1] else {
                return Err("doctrine cost consumption mark invalid");
            };
            *count = count
                .checked_add(1)
                .ok_or("doctrine cost occurrence counter exhausted")?;
        } else {
            if self.costs.actions.len() == self.costs.actions.capacity() {
                return Err("doctrine prepared cost journal exhausted");
            }
            let mark = (epoch, self.costs.actions.len());
            self.costs.actions.push(Action::ConsumeCost {
                slot,
                folder,
                count: 1,
            });
            if first {
                self.cost_first[slot as usize] = mark;
            } else {
                *self
                    .cost_consumptions
                    .get_mut(&(slot, folder))
                    .expect("prepared cost folder") = mark;
            }
        }
        Ok(())
    }
    fn refresh_costs(&mut self, memory: &impl Memory, base: usize, slot: u32) -> Result<()> {
        let country = &self.countries[slot as usize];
        let array = n::array(memory, country.pointer, n::COSTS, n::COST_STRIDE)?;
        let old = country.costs;
        // A full owner snapshot supersedes its earlier pending scalar events.
        // Stale immutable payloads remain owned until this structural writer.
        self.cancel_cost_consumption(self.cost_first[slot as usize]);
        self.cost_first[slot as usize] = (0, 0);
        for ordinal in 0..old {
            let folder = self.costs.rows[self.costs.indices[&key2(slot, ordinal)]].folder;
            if let Some(mark) = self.cost_consumptions.remove(&(slot, folder)) {
                self.cancel_cost_consumption(mark);
            }
        }
        for ordinal in 0..array.count {
            let pointer = element(array, ordinal, n::COST_STRIDE)?;
            let (uses, cost_factor, folder) = n::cost(memory, base, pointer)?;
            let mut used = 0;
            let (start, len) = n::name(memory, pointer, n::COST_NAME, &mut self.text, &mut used)?;
            put(&mut self.cost_consumptions, (slot, folder), (0, 0))?;
            self.costs.write(
                DoctrineCostReductionRow {
                    slot,
                    ordinal,
                    uses,
                    cost_factor,
                    folder,
                    name: alloc(a::copy_vec(&self.text[start..start + len]))?,
                },
                true,
            )?;
        }
        for ordinal in array.count..old {
            self.costs.remove(key2(slot, ordinal))?;
        }
        self.cost_count = self.cost_count - self.countries[slot as usize].live_costs as usize
            + array.count as usize;
        self.countries[slot as usize].costs = array.count;
        self.countries[slot as usize].live_costs = array.count;
        if self.ready {
            self.prepare_cost_events()?;
        }
        Ok(())
    }
    fn refresh_grants(
        &mut self,
        memory: &impl Memory,
        base: usize,
        slot: u32,
        append: bool,
    ) -> Result<()> {
        let country = &self.countries[slot as usize];
        let array = n::array(memory, country.pointer, n::GRANTS, n::GRANT_STRIDE)?;
        let old = country.grants;
        if append && array.count < old {
            return Err("doctrine grant append lost existing membership");
        }
        for ordinal in if append { old } else { 0 }..array.count {
            let pointer = element(array, ordinal, n::GRANT_STRIDE)?;
            let (filter, daily_mastery, bonus, days) = n::grant(memory, base, pointer)?;
            let mut used = 0;
            let (start, len) = n::name(memory, pointer, n::GRANT_NAME, &mut self.text, &mut used)?;
            self.grants.set(DoctrineMasteryGrantRow {
                slot,
                ordinal,
                name: if len == 0 {
                    None
                } else {
                    Some(alloc(a::copy_vec(&self.text[start..start + len]))?)
                },
                relevant_tracks: DoctrineTrackFilter {
                    track: filter.track,
                    folder: filter.folder,
                    grand_doctrine: filter.grand,
                    sub_doctrine: filter.sub,
                    track_index: filter.track_index,
                },
                daily_mastery,
                bonus,
                days_at_anchor: days,
                expiry_events_at_anchor: self.countries[slot as usize].expiry_events,
            })?;
        }
        for ordinal in array.count..old {
            self.grants.remove(key2(slot, ordinal))?;
        }
        self.countries[slot as usize].grants = array.count;
        Ok(())
    }
    fn expire_grants(&mut self, slot: u32) -> Result<()> {
        let country = &self.countries[slot as usize];
        let old = country.grants;
        if old == 0 {
            return Ok(());
        }
        let expiry_events = country.expiry_events.wrapping_add(1);
        let raw_country = country.country;
        let mut next = 0;
        for ordinal in 0..old {
            let index = self.grants.indices[&key2(slot, ordinal)];
            let row = &self.grants.rows[index];
            let days = row.days_at(expiry_events);
            if days <= 0 || row.daily_mastery == 0 && row.bonus == 0 {
                continue;
            }
            if next != ordinal {
                let mut row = alloc(row.try_clone_validated())?;
                row.ordinal = next;
                self.grants.set(row)?;
            }
            next += 1;
        }
        for ordinal in next..old {
            self.grants.remove(key2(slot, ordinal))?;
        }
        self.countries[slot as usize].grants = next;
        self.countries[slot as usize].expiry_events = expiry_events;
        self.country_rows.set(DoctrineCountryRow {
            slot,
            country: raw_country,
            expiry_events,
        })?;
        Ok(())
    }
    fn refresh_equipment(&mut self, memory: &impl Memory, base: usize, slot: u32) -> Result<()> {
        let country = &self.countries[slot as usize];
        let array = n::array(memory, country.pointer, n::EQUIPMENT, n::EQUIPMENT_STRIDE)?;
        let old = country.equipment;
        for ordinal in 0..array.count {
            let (id, index, equipment_bonus) =
                n::equipment(memory, base, element(array, ordinal, n::EQUIPMENT_STRIDE)?)?;
            self.equipment.set(DoctrineEquipmentBonusRow {
                slot,
                ordinal,
                id,
                index,
                equipment_bonus,
            })?;
        }
        for ordinal in array.count..old {
            self.equipment.remove(key2(slot, ordinal))?;
        }
        self.countries[slot as usize].equipment = array.count;
        Ok(())
    }
}

/// The host retains this mutex until native callbacks have quiesced.
/// # Safety
/// `context` is the registered live mutex, owned for the entire observer lifetime.
pub unsafe fn observe(context: usize, domain: Domain, pointer: usize) {
    let mutex = unsafe { &*(context as *const Mutex<Store>) };
    if let Ok(mut store) = mutex.lock() {
        let base = store.base;
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        if store.observe(&memory, base, domain, pointer).is_err() {
            store.failed = true;
        }
    }
}

/// Cost consumption publishes one prepared scalar occurrence. Immutable names
/// and folders are neither read nor cloned here; removals are derived on spool.
/// # Safety
/// Uses the retained context and quiescence contract of `observe`.
pub unsafe fn observe_cost_consumption(context: usize, pointer: usize, folder: usize) {
    let mutex = unsafe { &*(context as *const Mutex<Store>) };
    if let Ok(mut store) = mutex.lock() {
        let base = store.base;
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        if store
            .observe_cost_consumption(&memory, base, pointer, folder)
            .is_err()
        {
            store.failed = true;
        }
    }
}

/// Operand capture from either native effective-gain store. No live read.
/// # Safety
/// The context follows the same lifetime contract as `observe`.
pub unsafe fn observe_gain(context: usize, pointer: usize, value: i64) {
    let mutex = unsafe { &*(context as *const Mutex<Store>) };
    if let Ok(mut store) = mutex.lock() {
        if !store.ready || store.failed {
            return;
        }
        let Some(location) = store.tracks.get(&pointer).copied() else {
            return;
        };
        let key = key3(location.slot, location.folder, location.ordinal);
        let index = store.track_rows.indices[&key];
        if store.track_rows.rows[index].effective_daily_mastery != value {
            let mut row = store.track_rows.rows[index].clone();
            row.effective_daily_mastery = value;
            if store.track_rows.set(row).is_err() {
                store.failed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static DENY_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn doctrine_journal_coalesces_remove_reinsert_and_shares_quiet_tables() {
        let mut live = Rows::<DoctrineTrackRow>::default();
        let mut projected = Rows {
            record: false,
            ..Rows::<DoctrineTrackRow>::default()
        };
        let mut batch = Vec::new();
        let row = DoctrineTrackRow {
            slot: 5_000,
            ordinal: 7,
            mastery: -1,
            ..Default::default()
        };
        live.set(row.clone()).unwrap();
        live.prepare_baseline().unwrap();
        live.handoff(&mut batch).unwrap();
        projected.apply(&mut batch).unwrap();
        let first = projected.publish(TableRows::DoctrineTracks).unwrap();
        live.handoff(&mut batch).unwrap();
        projected.apply(&mut batch).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &projected.publish(TableRows::DoctrineTracks).unwrap()
        ));
        let mut changed = row.clone();
        changed.mastery = 51;
        live.set(changed.clone()).unwrap();
        live.remove(changed.key()).unwrap();
        changed.mastery = 67;
        live.set(changed.clone()).unwrap();
        assert_eq!(live.actions.len(), 1);
        live.handoff(&mut batch).unwrap();
        projected.apply(&mut batch).unwrap();
        assert_eq!(projected.rows, [changed]);
        live.remove(row.key()).unwrap();
        live.handoff(&mut batch).unwrap();
        projected.apply(&mut batch).unwrap();
        assert!(projected.rows.is_empty());
    }

    #[test]
    fn doctrine_scalar_events_and_hourly_handoff_allocate_nothing_at_country_scale() {
        let _guard = DENY_LOCK.lock().unwrap();
        for count in [1_500, 5_000] {
            let mut rows = Rows::<DoctrineTrackRow>::default();
            for slot in 0..count {
                rows.set(DoctrineTrackRow {
                    slot,
                    ..Default::default()
                })
                .unwrap();
            }
            rows.prepare_baseline().unwrap();
            let mut batches: [Vec<Action<DoctrineTrackRow>>;
                collector_extended_support::hourly::BUFFERS] = std::array::from_fn(|_| Vec::new());
            rows.handoff(&mut batches[0]).unwrap();
            batches[0].clear();
            collector_test_support::with_allocations_denied(|| {
                for batch in &mut batches {
                    for slot in 0..count {
                        rows.set(DoctrineTrackRow {
                            slot,
                            mastery: 42,
                            ..Default::default()
                        })
                        .unwrap();
                    }
                    rows.handoff(batch).unwrap();
                    batch.clear();
                }
            });
            assert_eq!(
                collector_test_support::ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed),
                0
            );
        }
    }

    #[test]
    fn doctrine_cost_consumption_is_prepared_and_reads_only_fixed_operands() {
        use std::cell::Cell;
        let _guard = DENY_LOCK.lock().unwrap();
        struct Scalars {
            remaining: u32,
            reads: Cell<usize>,
        }
        impl Memory for Scalars {
            fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                match (address, bytes.len()) {
                    (0x200, 8) => {
                        bytes.copy_from_slice(&(n::FOLDER_TEMPLATE_VTABLE as u64).to_le_bytes())
                    }
                    (0x208, 4) => bytes.copy_from_slice(&91u32.to_le_bytes()),
                    (address, 4) if address == 0x100 + n::COSTS + cpdx_array::COUNT => {
                        bytes.copy_from_slice(&self.remaining.to_le_bytes())
                    }
                    _ => panic!("cost consumption read an inventory or immutable payload"),
                }
                true
            }
        }
        for count in [1_500u32, 5_000] {
            let slot = count - 1;
            let mut store = Store::default();
            store
                .countries
                .resize_with(count as usize, Country::default);
            store.cost_first.resize(count as usize, (0, 0));
            store.countries[slot as usize].costs = count;
            store.countries[slot as usize].live_costs = count;
            store.cost_count = count as usize;
            store.owners.insert(0x100, slot);
            store.cost_consumptions.insert((slot, Some(91)), (0, 0));
            for ordinal in 0..count {
                store
                    .costs
                    .set(DoctrineCostReductionRow {
                        slot,
                        ordinal,
                        folder: Some(91),
                        uses: 100_000,
                        cost_factor: 35_000,
                        name: vec![b'n'; 256],
                    })
                    .unwrap();
            }
            store.prepare_cost_events().unwrap();
            let mut batch = Vec::new();
            let mut projection = Rows {
                record: false,
                ..Rows::default()
            };
            store.costs.handoff(&mut batch).unwrap();
            projection.apply_costs(&mut batch).unwrap();
            let name = store.costs.rows[0].name.as_ptr();
            let memory = Scalars {
                remaining: count,
                reads: Cell::new(0),
            };
            store.ready = true;
            collector_test_support::with_allocations_denied(|| {
                for _ in 0..32 {
                    store
                        .observe_cost_consumption(&memory, 0, 0x100, 0x200)
                        .unwrap();
                }
                store.costs.handoff(&mut batch).unwrap();
            });
            assert_eq!(collector_test_support::ATTEMPTS.load(Ordering::Relaxed), 0);
            assert_eq!(memory.reads.get(), 96); // Independent of inventory/country count.
            assert_eq!(store.costs.rows[0].name.as_ptr(), name);
            assert_eq!(store.costs.rows[0].uses, 100_000); // Immutable snapshot on live side.
            assert_eq!(batch.len(), 2); // Ordered first cleanup, then 31 coalesced uses.
            projection.apply_costs(&mut batch).unwrap();
            assert_eq!(
                projection.rows[projection.indices[&key2(slot, 0)]].uses,
                99_968
            );
            assert_eq!(
                projection.rows[projection.indices[&key2(slot, 1)]].uses,
                100_000
            );
            assert_eq!(projection.rows.len(), count as usize);
        }
    }

    #[test]
    fn doctrine_cost_spool_derives_threshold_removals_order_and_wrapping() {
        let mut rows = Rows::<DoctrineCostReductionRow> {
            record: false,
            ..Default::default()
        };
        for (ordinal, uses, factor, folder) in [
            (0, 1, 200_000, Some(1)),
            (1, 2, -150_000, Some(1)),
            (2, 5, 35_000, Some(2)),
            (3, i32::MIN, 35_000, Some(2)),
            (4, 0, 35_000, None),
            (5, 2, 35_000, Some(1)),
        ] {
            rows.set(DoctrineCostReductionRow {
                ordinal,
                uses,
                cost_factor: factor,
                folder,
                name: vec![ordinal as u8],
                ..Default::default()
            })
            .unwrap();
        }
        rows.consume_cost(0, Some(1)).unwrap();
        let ordered = |rows: &Rows<DoctrineCostReductionRow>| {
            (0..rows.rows.len())
                .map(|ordinal| {
                    let row = &rows.rows[rows.indices[&key2(0, ordinal as u32)]];
                    (row.name[0], row.uses, row.folder)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ordered(&rows),
            [(1, 1, Some(1)), (2, 5, Some(2)), (5, 2, Some(1))]
        );
        rows.consume_cost(0, None).unwrap();
        assert_eq!(rows.rows.len(), 3); // Null is not a wildcard folder.
        rows.set(DoctrineCostReductionRow {
            ordinal: 3,
            uses: i32::MIN,
            folder: None,
            cost_factor: i64::MAX,
            name: vec![6],
            ..Default::default()
        })
        .unwrap();
        rows.set(DoctrineCostReductionRow {
            ordinal: 4,
            uses: 2,
            folder: None,
            cost_factor: 1,
            name: vec![7],
            ..Default::default()
        })
        .unwrap();
        rows.consume_cost(0, None).unwrap();
        assert_eq!(ordered(&rows)[3..], [(6, i32::MAX, None), (7, 1, None)]);
    }

    #[test]
    fn doctrine_cost_first_cleanup_precedes_other_folder_consumption() {
        let mut rows = Rows::<DoctrineCostReductionRow> {
            record: false,
            ..Default::default()
        };
        for (ordinal, uses, folder) in [(0, i32::MIN, Some(1)), (1, 4, Some(2)), (2, 3, None)] {
            rows.set(DoctrineCostReductionRow {
                ordinal,
                uses,
                folder,
                cost_factor: 35_000,
                name: vec![ordinal as u8],
                ..Default::default()
            })
            .unwrap();
        }
        let mut events = vec![
            Action::ConsumeCost {
                slot: 0,
                folder: Some(2),
                count: 1,
            },
            Action::ConsumeCost {
                slot: 0,
                folder: Some(1),
                count: 2,
            },
            Action::ConsumeCost {
                slot: 0,
                folder: Some(2),
                count: 2,
            },
        ];
        rows.apply_costs(&mut events).unwrap();
        assert_eq!(rows.rows.len(), 2);
        assert_eq!(rows.rows[rows.indices[&key2(0, 0)]].name, [1]);
        assert_eq!(rows.rows[rows.indices[&key2(0, 0)]].uses, 1);
        assert_eq!(rows.rows[rows.indices[&key2(0, 1)]].name, [2]);
        assert_eq!(rows.rows[rows.indices[&key2(0, 1)]].uses, 3);
    }

    #[test]
    fn doctrine_cost_native_result_count_certifies_spool_derivation() {
        let mut frame = Frame {
            projection: Some(Arc::new(SharedProjection::default())),
            counts: [0, 0, 0, 0, 1, 0, 0],
            ..Default::default()
        };
        let _ = frame.inputs();
        assert_eq!(
            frame.refusal,
            "doctrine cost consumption count differs from native result"
        );
        assert!(
            frame
                .projection
                .as_ref()
                .unwrap()
                .failed
                .load(Ordering::Acquire)
        );
    }

    #[test]
    fn doctrine_ordinary_hour_does_not_read_native_memory() {
        let _guard = DENY_LOCK.lock().unwrap();
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("ordinary doctrine hour read native memory")
            }
        }
        changes::enabled(true);
        let mut store = Store {
            ready: true,
            root: (0, 0),
            ..Default::default()
        };
        for (i, domain) in DOMAINS.iter().enumerate() {
            store.epochs[i] = changes::token(*domain).unwrap().0;
        }
        let mut frame = Frame::default();
        collector_test_support::with_allocations_denied(|| {
            store
                .capture(&NoReads, 0, (0, 0), false, &mut frame)
                .unwrap();
        });
        assert!(frame.complete());
        changes::enabled(false);
    }
    #[test]
    fn doctrine_expiry_derives_order_compaction_and_wrapping_days() {
        let mut store = Store::default();
        store.countries.push(Country {
            grants: 4,
            ..Default::default()
        });
        for (ordinal, days, daily, bonus) in [
            (0, 1, 10, 0),
            (1, 5, 10, 0),
            (2, 7, 0, 0),
            (3, i32::MIN, 0, 8),
        ] {
            store
                .grants
                .set(DoctrineMasteryGrantRow {
                    ordinal,
                    days_at_anchor: days,
                    daily_mastery: daily,
                    bonus,
                    name: Some(vec![ordinal as u8]),
                    ..Default::default()
                })
                .unwrap();
        }
        store.expire_grants(0).unwrap();
        assert_eq!(store.countries[0].grants, 2);
        assert_eq!(
            store.grants.rows[store.grants.indices[&key2(0, 0)]]
                .days_at(store.countries[0].expiry_events),
            4
        );
        assert_eq!(
            store.grants.rows[store.grants.indices[&key2(0, 0)]].name,
            Some(vec![1])
        );
        assert_eq!(
            store.grants.rows[store.grants.indices[&key2(0, 1)]]
                .days_at(store.countries[0].expiry_events),
            i32::MAX
        );
        assert_eq!(
            store.grants.rows[store.grants.indices[&key2(0, 1)]].name,
            Some(vec![3])
        );
    }

    #[test]
    fn doctrine_native_baseline_and_targeted_events_reach_replay_without_hourly_reads() {
        let _guard = DENY_LOCK.lock().unwrap();
        struct Memory(Vec<u8>);
        impl engine_native::Memory for Memory {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(bytes) = self.0.get(address..address + output.len()) else {
                    return false;
                };
                output.copy_from_slice(bytes);
                true
            }
        }
        impl Memory {
            fn word(&mut self, address: usize, value: u64) {
                self.0[address..address + 8].copy_from_slice(&value.to_le_bytes());
            }
            fn array(&mut self, address: usize, data: usize, count: u32) {
                self.word(address, data as u64);
                self.word(address + 8, u64::from(count) | (u64::from(count) << 32));
            }
        }
        struct NoReads;
        impl engine_native::Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("hourly native read")
            }
        }
        let mut memory = Memory(vec![0; 0x1000]);
        let status = 8;
        memory.word(status, n::STATUS_VTABLE as u64);
        memory.word(status + n::COUNTRY, 11);
        memory.array(status + n::FOLDERS, 0x200, 1);
        memory.array(status + n::COSTS, 0x500, 1);
        memory.array(status + n::GRANTS, 0x600, 1);
        memory.array(status + n::EQUIPMENT, 0x700, 1);
        for (address, vtable, token) in [
            (0x800, n::FOLDER_TEMPLATE_VTABLE, 91),
            (0x820, n::GRAND_TEMPLATE_VTABLE, 92),
            (0x840, n::SUB_TEMPLATE_VTABLE, 93),
        ] {
            memory.word(address, vtable as u64);
            memory.word(address + n::TEMPLATE_TOKEN, token);
        }
        memory.word(0x200, n::FOLDER_VTABLE as u64);
        memory.word(0x200 + n::FOLDER_TEMPLATE, 0x800);
        memory.word(0x200 + n::GRAND_TEMPLATE, 0x820);
        memory.word(0x200 + n::FOLDER_OWNER, status as u64);
        memory.array(0x200 + n::TRACKS, 0x300, 1);
        memory.array(0x200 + n::COMPLETED, 0x780, 2);
        memory.word(0x780, 0x840);
        memory.word(0x788, 0x840);
        memory.word(0x300, n::TRACK_VTABLE as u64);
        memory.word(0x300 + n::SUB_TEMPLATE, 0x840);
        memory.word(0x300 + n::MASTERY, 125_000);
        memory.word(0x300 + n::BANK, (-1_i64) as u64);
        memory.word(0x300 + n::DAILY, 50_000);
        memory.word(0x300 + n::EFFECTIVE_DAILY, 75_000);
        memory.word(0x300 + n::TRACK_OWNER, status as u64);
        memory.word(0x300 + n::TRACK_FOLDER, 0x200);
        memory.array(0x300 + n::LEADERS, 0x400, 1);
        memory.word(0x400, n::LEADER_VTABLE as u64);
        memory.word(0x408, 17 | (91_u64 << 32));
        memory.word(0x410, 5000);
        memory.word(0x500, n::COST_VTABLE as u64);
        memory.word(0x500 + n::COST_USES, 2);
        memory.word(0x500 + n::COST_NAME + 24, 15);
        memory.word(0x500 + n::COST_FACTOR, (-35_000_i64) as u64);
        memory.word(0x500 + n::COST_FOLDER, 0x800);
        memory.word(0x600, n::GRANT_VTABLE as u64);
        memory.word(0x600 + n::GRANT_NAME + 24, 15);
        memory.word(0x600 + n::GRANT_FILTER, n::FILTER_VTABLE as u64);
        memory.word(0x600 + n::GRANT_FILTER + 0x28, (-1_i64) as u64);
        memory.word(0x600 + n::GRANT_DAILY, 12_500);
        memory.word(0x600 + n::GRANT_DAYS, 5);
        memory.word(0x700, n::EQUIPMENT_VTABLE as u64);
        memory.word(0x708, 71 | (3_u64 << 32));
        memory.word(0x710, 4);
        changes::enabled(true);
        let mut store = Store::default();
        let mut frame = Frame::default();
        store
            .capture(&memory, 0, (status, 1), false, &mut frame)
            .unwrap();
        let initial = frame.inputs();
        assert!(
            initial
                .iter()
                .all(|input| matches!(input, TableInput::SharedComplete(_)))
        );
        let TableInput::SharedComplete(rows) = &initial[2] else {
            unreachable!()
        };
        let TableRows::DoctrineTracks(rows) = &**rows else {
            unreachable!()
        };
        assert_eq!(
            (
                rows[0].mastery,
                rows[0].mastery_bank,
                rows[0].daily_mastery,
                rows[0].effective_daily_mastery
            ),
            (125_000, -1, 50_000, 75_000)
        );
        struct FoldersOnly<'a>(&'a Memory);
        impl engine_native::Memory for FoldersOnly<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let end = address + output.len();
                for unrelated in [8 + n::COSTS..8 + n::EQUIPMENT + 24, 0x500..0x778] {
                    assert!(
                        end <= unrelated.start || address >= unrelated.end,
                        "folder reset read an unrelated inventory"
                    );
                }
                engine_native::Memory::copy(self.0, address, output)
            }
        }
        store
            .observe(&FoldersOnly(&memory), 0, Domain::StatusFolders, status)
            .unwrap();
        memory.word(0x300 + n::MASTERY, 150_000);
        store.observe(&memory, 0, Domain::Track, 0x300).unwrap();
        memory.word(0x600 + n::GRANT_DAYS, 4);
        store
            .observe(&NoReads, 0, Domain::GrantExpiry, status)
            .unwrap();
        // A structural owner snapshot supersedes an earlier pending consumption,
        // including its cleanup. A later consumption must see the appended row.
        memory.word(0x500 + n::COST_USES, 1);
        store
            .observe_cost_consumption(&memory, 0, status, 0x800)
            .unwrap();
        memory.array(status + n::COSTS, 0x500, 2);
        memory.word(0x540, n::COST_VTABLE as u64);
        memory.word(0x540 + n::COST_USES, 3);
        memory.word(0x540 + n::COST_NAME, u64::from_le_bytes(*b"new\0\0\0\0\0"));
        memory.word(0x540 + n::COST_NAME + 16, 3);
        memory.word(0x540 + n::COST_NAME + 24, 15);
        memory.word(0x540 + n::COST_FACTOR, 35_000);
        memory.word(0x540 + n::COST_FOLDER, 0x800);
        store.observe(&memory, 0, Domain::Costs, status).unwrap();
        memory.array(status + n::COSTS, 0x500, 1);
        store
            .observe_cost_consumption(&memory, 0, status, 0x800)
            .unwrap();
        frame.reset();
        collector_test_support::with_allocations_denied(|| {
            store
                .capture(&NoReads, 0, (status, 1), false, &mut frame)
                .unwrap()
        });
        let next = frame.inputs();
        let TableInput::SharedComplete(rows) = &next[2] else {
            unreachable!()
        };
        let TableRows::DoctrineTracks(rows) = &**rows else {
            unreachable!()
        };
        assert_eq!(rows[0].mastery, 150_000);
        let TableInput::SharedComplete(costs) = &next[4] else {
            unreachable!()
        };
        let TableRows::DoctrineCostReductions(costs) = &**costs else {
            unreachable!()
        };
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].name, b"new");
        assert_eq!((costs[0].ordinal, costs[0].uses), (0, 3));
        assert_eq!(frame.counts[4], 1);
        assert!(match (&initial[1], &next[1]) {
            (TableInput::SharedComplete(a), TableInput::SharedComplete(b)) => Arc::ptr_eq(a, b),
            _ => false,
        });
        assert!(match (&initial[5], &next[5]) {
            (TableInput::SharedComplete(a), TableInput::SharedComplete(b)) => Arc::ptr_eq(a, b),
            _ => false,
        });
        let TableInput::SharedComplete(grants) = &next[5] else {
            unreachable!()
        };
        let TableRows::DoctrineMasteryGrants(grants) = &**grants else {
            unreachable!()
        };
        assert_eq!(grants[0].days_at(store.countries[0].expiry_events), 4);
        changes::enabled(false);
    }
}
