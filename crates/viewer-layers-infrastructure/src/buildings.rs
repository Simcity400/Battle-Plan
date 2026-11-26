
use std::time::Instant;
use viewer_layer_core::{layers::{Carried, Change, Settling, Dirty, Gpu, Inputs, Layer, Num, Scope as Settled, Scoped, Sight as InView, Text, Upstream, View}, touched::{Touch, Touched}, vision::Vision as Sight};
use viewer_layer_core::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, hash_map::Entry},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use map_display_data::{buildings as wire, value_index::ValueIndex};
use viewer_foundation::perspective::{Choice, MAP_OBJECT_VISIBILITY, Vision};
use viewer_buildings::{
    map_buildings::{BuildingInput, CityTerrain, FLOATING_HARBOR_VISIBILITY, HarbourCounts, MapBuildingAssets, SlotPages, WIRE_CHUNK as CHUNK, WireHour, MapBuildingSummary, PreparedSlot, READS_DETECTION, READS_FOG, READS_HUB, SlotGates, TokenClass, detected, health_state},
    sprawl::average_health,
};
use viewer_foundation::building_facts::Scope;

const TABLES: [TableId; 2] = [TableId::StateBuildings, TableId::ProvinceBuildings];

pub mod fed;
pub use fed::BuildingsFed;
const CORE: &str = "Provinces, CountryLinks and Countries";
const NO_ROW: u32 = u32::MAX;
const NOT_SLOTTED: u8 = u8::MAX;
const UNSEEN: u8 = u8::MAX - 1;

fn fog_decisions(byte: Option<u8>) -> u8 {
    let decided = |seen: Option<bool>| match seen {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    decided(byte.map(|byte| byte >= MAP_OBJECT_VISIBILITY)) | decided(byte.map(|byte| byte >= FLOATING_HARBOR_VISIBILITY)) << 2
}
type Read = (i16, (bool, bool, bool), u32, u32);
const CAUSES: [&str; 9] = [
    "slots: a row's read changed",
    "slots: fog",
    "slots: port",
    "slots: detection (controller, link, raid target)",
    "slots: every slot (restart, viewer, whole vision)",
    "harbours",
    "city locations: a damage template's row written",
    "rows written",
    "rows written whose read held",
];
const ROW: usize = 0;
const WHOLE: usize = 4;
const HARBOUR: usize = 5;
const CITY: usize = 6;
const WRITTEN: usize = 7;
const HELD: usize = 8;

fn city_class(health: Option<Option<i64>>, (damaged, destroyed): (i64, i64)) -> u8 {
    match health {
        None => 0,
        Some(None) => 1,
        Some(Some(health)) if health >= damaged => 1,
        Some(Some(health)) if health < destroyed => 3,
        Some(Some(_)) => 2,
    }
}

const FULL_HEALTH: i64 = 100_000;

pub trait Seen {
    fn ours(&self) -> u32;
    fn byte(&self, province: u32) -> Option<u8>;
}

pub trait Upper {
    fn state(&self) -> (Option<u32>, bool, u64);
    fn refused(&self) -> Option<String>;
    fn seen(&self) -> &dyn Seen;
    fn moved(&self) -> Change<'_>;
}

struct FromLayer<'a> {
    sight: &'a Sight,
    changed: Change<'a>,
}

impl<'a> FromLayer<'a> {
    fn of(upstream: &Upstream<'a>) -> Option<Self> {
        Some(Self { sight: upstream.get::<Sight>()?, changed: upstream.changed::<Sight>() })
    }
}

impl Upper for FromLayer<'_> {
    fn state(&self) -> (Option<u32>, bool, u64) {
        (self.sight.ours(), self.sight.fog_drawn(), self.sight.sight_epoch())
    }

    fn refused(&self) -> Option<String> {
        self.sight.refused()
    }

    fn seen(&self) -> &dyn Seen {
        self.sight
    }

    fn moved(&self) -> Change<'_> {
        self.changed
    }
}

pub struct Inspected {
    pub refused: Option<String>,
    pub slots: Vec<PreparedSlot>,
    pub health: Vec<(u32, Option<i64>)>,
    pub city_refused: Option<String>,
    pub warnings: Vec<String>,
    pub unreadable: u64,
    pub whole_visions: BTreeMap<&'static str, u64>,
}

impl Inspected {
    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        let mut differing = 0;
        let mut first = None;
        let mut note = |what: String| {
            differing += 1;
            first.get_or_insert(what);
        };
        if self.refused != other.refused {
            note(format!("refused: {:?} against {:?}", self.refused, other.refused));
            return (differing, first);
        }
        if self.slots.len() != other.slots.len() {
            note(format!("{} slots against {}", self.slots.len(), other.slots.len()));
        }
        for (id, (a, b)) in self.slots.iter().zip(&other.slots).enumerate() {
            if a != b {
                note(format!("slot {id}: {a:?} against {b:?}"));
            }
        }
        if self.health != other.health {
            note("the city damage differs".into());
        }
        if self.city_refused != other.city_refused {
            note(format!("city damage refused: {:?} against {:?}", self.city_refused, other.city_refused));
        }
        if self.warnings != other.warnings {
            note(format!("warnings: {:?} against {:?}", self.warnings, other.warnings));
        }
        (differing, first)
    }
}

#[derive(Clone, Copy)]
pub struct Declared {
    columns: Option<Columns>,
    fog: bool,
}

pub type Install = (wire::Buildings, bool);

fn figures(causes: &[u64; CAUSES.len()], uploads: &[u64; 6], upload_ns: &[u64; 3]) -> serde_json::Value {
    let [index, product, install] = upload_ns.map(|ns| ns as f64 / 1e6);
    serde_json::json!({
        "marked_by_cause": CAUSES.iter().zip(causes).map(|(name, count)| (name.to_string(), serde_json::json!(count))).collect::<serde_json::Map<_, _>>(),
        "uploads_installed": uploads[0],
        "installs_with_slot_chunks_changed": uploads[1],
        "installs_with_drawn_city_states_moved": uploads[2],
        "index_provinces_patched": uploads[3],
        "installs_with_the_city_index_made_whole": uploads[5],
        "provinces_judged_in_the_settle": uploads[4],
        "install_ms_total": {"health index": index, "pages and product": product, "the scene's install": install},
    })
}

impl Seen for Vision {
    fn ours(&self) -> u32 {
        Vision::ours(self)
    }

    fn byte(&self, province: u32) -> Option<u8> {
        self.province_byte(province)
    }
}

impl Seen for Sight {
    fn ours(&self) -> u32 {
        Sight::ours(self).unwrap_or_default()
    }

    fn byte(&self, province: u32) -> Option<u8> {
        self.province_byte(province)
    }
}

pub trait Catalogue: Send + Sync {
    fn assets(&self) -> &MapBuildingAssets;
    fn city(&self) -> CityTerrain<'_>;
    fn sprawl_cutoff(&self) -> f32;
}


#[derive(Clone, Copy)]
struct Rows {
    country: Num<u32>,
    flags: Num<u32>,
    level: Num<i16>,
    healthy: Num<i16>,
    partial: Num<i64>,
}

#[derive(Clone, Copy)]
struct Columns {
    rows: [Rows; 2],
    literal: Text,
    province_state: Num<u32>,
    controller: [Num<u32>; 2],
    link: Num<u32>,
    supply_controller: Num<u16>,
    supply_flags: Num<u8>,
    deploy_province: Num<i32>,
    deploy_status: Num<u8>,
    target_type: Num<u32>,
    target_location: Num<u32>,
    target_flags: Num<u32>,
    target_country: Num<i32>,
    target_detected: Text,
}

fn class(mirror: Option<&Mirror>) -> u8 {
    match mirror.and_then(Mirror::status) {
        Some(Status::Complete) => 0,
        Some(Status::Truncated { .. }) => 1,
        _ => 2,
    }
}

fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| class(Some(mirror)) < 2)
}

const READ: [TableId; 10] = [
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::LexicalTokens,
    TableId::NavalBases,
    TableId::ProvinceSupply,
    TableId::FloatingHarborDeployments,
    TableId::RaidTargetStatuses,
    TableId::Provinces,
    TableId::States,
    TableId::CountryLinks,
];

fn not_whole(mirror: Option<&Mirror>, table: TableId, hour: Option<i32>) -> Option<String> {
    let hour = hour.map_or_else(|| "?".to_owned(), |hour| hour.to_string());
    match mirror {
        Some(mirror) => mirror.not_recorded().map(|why| format!("hour {hour} {why}")),
        None => Some(format!("hour {hour} {table:?} is not read by the map")),
    }
}

pub struct Buildings {
    content: Arc<dyn Catalogue>,
    columns: Option<Columns>,
    reads_layer: bool,
    seen_state: Option<(Option<u32>, bool, u64)>,
    fog_provinces: Vec<u32>,
    fog_seen: Vec<u8>,
    fog_dirty: Dirty,
    fog_fill: bool,
    given: Mutex<Option<(Choice, Arc<Vision>)>>,
    fresh_vision: AtomicBool,
    vision: Option<(Choice, Arc<Vision>)>,
    held: Option<[u8; READ.len()]>,
    refused: Option<String>,
    slots: Vec<PreparedSlot>,
    gate_reads: Vec<u8>,
    harbours: Vec<HarbourCounts>,
    seas: HashMap<u32, Vec<u32>>,
    templates: Result<Vec<usize>, String>,
    rows_of: HashMap<u32, u32>,
    classes: HashMap<u32, TokenClass>,
    tokens_of: Vec<Vec<u32>>,
    city: [HashMap<u32, i64>; 2],
    city_refused: Option<String>,
    first_unnamed: Option<String>,
    provinces_held: usize,
    province_state: Vec<u32>,
    state_provinces: BTreeSet<(u32, u32)>,
    controller_of: [HashMap<u32, u32>; 2],
    by_controller: BTreeSet<(u32, u8, u32)>,
    targets: BTreeSet<(Scope, u32)>,
    target_scope: HashMap<u32, Scope>,
    deployments: BTreeMap<(u32, i32, u32), u8>,
    rows_dirty: Vec<(bool, u128)>,
    rows_seen: HashMap<(bool, u128), bool>,
    reads: [Vec<Option<Read>>; 2],
    tokens_dirty: BTreeSet<u32>,
    controllers_dirty: Vec<(u8, u32)>,
    links_dirty: BTreeSet<u32>,
    ports_dirty: Vec<u32>,
    targets_dirty: BTreeSet<u32>,
    touched: Touched,
    unnamed_check: bool,
    slot_dirty: Dirty,
    harbour_dirty: Dirty,
    city_dirty: [Dirty; 2],
    frame: u64,
    marked: [usize; 3],
    unreadable: u64,
    scoped: Scoped,
    whole_visions: BTreeMap<&'static str, u64>,
    wired: Option<wire::Buildings>,
    pages: Option<SlotPages>,
    chunks_changed: BTreeSet<usize>,
    health_index: ValueIndex<u32, Option<i64>>,
    index_due: BTreeSet<u32>,
    city_whole: bool,
    cold: bool,
    warned: Option<Vec<String>>,
    drawn_class: Vec<u8>,
    drawn_by: Option<(i64, i64)>,
    supply_read: HashMap<u32, (Option<u16>, Option<u8>)>,
    deployment_of: HashMap<u128, (u32, i32, u32)>,
    causes: [u64; CAUSES.len()],
    uploads: [u64; 6],
    upload_ns: [u64; 3],
    hour: i32,
    fog: bool,
    #[cfg(any(test, feature = "test-support"))]
    applied_reads: u64,
}

impl Buildings {
    pub fn new(content: Arc<dyn Catalogue>) -> Self {
        let assets = content.assets();
        let mut seas: HashMap<u32, Vec<u32>> = HashMap::new();
        for index in 0..assets.harbour_count() {
            seas.entry(assets.harbour_sea(index)).or_default().push(index as u32);
        }
        let templates = assets.city_templates();
        let types = assets.types().len();
        let mut fog_provinces: Vec<u32> = assets.slotted_provinces().chain(seas.keys().copied()).collect();
        fog_provinces.sort_unstable();
        fog_provinces.dedup();
        let mut fog_seen = vec![NOT_SLOTTED; fog_provinces.last().map_or(0, |&last| last as usize + 1)];
        fog_provinces.iter().for_each(|&province| fog_seen[province as usize] = UNSEEN);
        let layer = Self {
            columns: None,
            given: Mutex::new(None),
            fresh_vision: AtomicBool::new(false),
            vision: None,
            held: None,
            refused: None,
            slots: Vec::new(),
            gate_reads: Vec::new(),
            harbours: Vec::new(),
            seas,
            templates,
            rows_of: HashMap::new(),
            classes: HashMap::new(),
            tokens_of: vec![Vec::new(); types],
            city: [HashMap::new(), HashMap::new()],
            city_refused: None,
            first_unnamed: None,
            provinces_held: 0,
            province_state: Vec::new(),
            state_provinces: BTreeSet::new(),
            controller_of: [HashMap::new(), HashMap::new()],
            by_controller: BTreeSet::new(),
            targets: BTreeSet::new(),
            target_scope: HashMap::new(),
            deployments: BTreeMap::new(),
            rows_dirty: Vec::new(),
            rows_seen: HashMap::new(),
            reads: [Vec::new(), Vec::new()],
            tokens_dirty: BTreeSet::new(),
            controllers_dirty: Vec::new(),
            links_dirty: BTreeSet::new(),
            ports_dirty: Vec::new(),
            targets_dirty: BTreeSet::new(),
            touched: Touched::default(),
            unnamed_check: false,
            slot_dirty: Dirty::default(),
            harbour_dirty: Dirty::default(),
            city_dirty: [Dirty::default(), Dirty::default()],
            frame: 0,
            marked: [0; 3],
            unreadable: 0,
            scoped: Scoped::default(),
            whole_visions: BTreeMap::new(),
            reads_layer: false,
            seen_state: None,
            fog_provinces: Vec::new(),
            fog_seen: Vec::new(),
            fog_dirty: Dirty::default(),
            fog_fill: true,
            wired: None,
            pages: None,
            chunks_changed: BTreeSet::new(),
            health_index: ValueIndex::default(),
            index_due: BTreeSet::new(),
            city_whole: true,
            cold: true,
            warned: None,
            drawn_class: Vec::new(),
            drawn_by: None,
            supply_read: HashMap::new(),
            deployment_of: HashMap::new(),
            causes: [0; CAUSES.len()],
            uploads: [0; 6],
            upload_ns: [0; 3],
            hour: 0,
            fog: true,
            #[cfg(any(test, feature = "test-support"))]
            applied_reads: 0,
            content,
        };
        Self { fog_provinces, fog_seen, ..layer }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    pub fn reading_vision(content: Arc<dyn Catalogue>) -> Self {
        Self { reads_layer: true, ..Self::new(content) }
    }

    pub fn fresh(&self) -> Self {
        let fresh = Self { columns: self.columns, reads_layer: self.reads_layer, ..Self::new(Arc::clone(&self.content)) };
        if let Some((choice, vision)) = &self.vision {
            fresh.set_vision(choice.clone(), Arc::clone(vision));
        }
        fresh
    }

    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        *self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((choice, vision));
        self.fresh_vision.store(true, Ordering::Release);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn slots(&self) -> &[PreparedSlot] {
        &self.slots
    }

    pub fn health(&self) -> impl Iterator<Item = (u32, Option<i64>)> + '_ {
        let full = |at: usize, location: u32| self.city[at].get(&location).copied().unwrap_or(FULL_HEALTH);
        self.province_state.iter().enumerate().filter(|_| self.city_refused.is_none()).filter(|(_, state)| **state != NO_ROW).map(move |(province, &state)| {
            let province = province as u32;
            (province, (state != 0).then(|| full(0, state).min(full(1, province))))
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn city_refused(&self) -> Option<&str> {
        self.city_refused.as_deref()
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut summary = MapBuildingSummary::default();
        for (&token, &count) in &self.rows_of {
            if let Some(TokenClass::Refused { reason, .. }) = self.classes.get(&token) {
                *summary.refused_tokens.entry(reason.clone()).or_default() += count as usize;
            }
        }
        if let Some(reason) = &self.city_refused {
            summary.unknown_city_damage.insert(reason.clone(), self.provinces_held);
        }
        let mut harbours = HarbourCounts::default();
        self.harbours.iter().for_each(|counts| harbours.add(*counts));
        harbours.apply(&mut summary);
        summary.warnings()
    }

    fn compose_upper(&mut self, mirrors: &Mirrors, upper: Option<&dyn Upper>) -> io::Result<()> {
        let marked = self.marked;
        (self.held, self.seen_state) = (None, None);
        self.settle_dirty(mirrors, upper)?;
        self.marked = marked;
        Ok(())
    }

    pub fn inspect(&self) -> Inspected {
        Inspected {
            refused: self.refused.clone(),
            slots: self.slots.clone(),
            health: self.health().collect(),
            city_refused: self.city_refused.clone(),
            warnings: self.warnings(),
            unreadable: self.unreadable,
            whole_visions: self.whole_visions.clone(),
        }
    }

    fn declare(&mut self, declared: Declared) {
        match declared.columns {
            None => {
                self.columns = None;
                self.touched.clear();
            }
            Some(columns) => {
                if self.columns.is_none() {
                    self.held = None;
                }
                (self.fog, self.columns) = (declared.fog, Some(columns));
            }
        }
    }

    fn note_thresholds(&mut self, thresholds: Option<(i64, i64)>) {
        if thresholds.is_some() && thresholds != self.drawn_by && self.wired.is_some() {
            self.city_whole = true;
        }
    }

    fn make(&mut self, bank: [u8; 32], thresholds: Option<(i64, i64)>) -> io::Result<Option<Install>> {
        if self.columns.is_none() || self.refused.is_some() || self.uploaded() {
            return Ok(None);
        }
        let started = Instant::now();
        let city_damage = Arc::new(wire::CityDamage { health: self.health_index(thresholds)?, refused: self.city_refused.clone() });
        let indexed = Instant::now();
        self.uploads[1] += u64::from(!self.chunks_changed.is_empty());
        let changed: Vec<usize> = std::mem::take(&mut self.chunks_changed).into_iter().collect();
        let warnings = match &self.warned {
            Some(warnings) => warnings.clone(),
            None => self.warned.insert(self.warnings()).clone(),
        };
        let hour = WireHour { branch: 0, hour: self.hour, perspective: self.seen_ours(), fog: self.fog, warnings, city_damage };
        let content = Arc::clone(&self.content);
        let assets = content.assets();
        let pages = assets.slot_pages(&self.slots, &changed, self.pages.as_ref())?;
        let product = assets.shared_product(&pages, bank, hour)?;
        self.pages = Some(pages);
        let made = Instant::now();
        let continues = !self.cold;
        (self.wired, self.cold, self.city_whole) = (Some(product.clone()), false, false);
        self.uploads[0] += 1;
        for (spent, (from, to)) in self.upload_ns.iter_mut().zip([(started, indexed), (indexed, made)]) {
            *spent += to.duration_since(from).as_nanos() as u64;
        }
        Ok(Some((product, continues)))
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the buildings layer is declared before it composes")
    }

    fn drawn(&self, view: &View) -> bool {
        let height = |cutoff: f32, at: f32| at - viewer_gpu::camera::WATER_HEIGHT < cutoff;
        let below = |cutoff: f32| view.camera.is_none() || height(cutoff, view.camera_height) || view.target.is_some_and(|target| height(cutoff, target.height));
        let assets = self.content.assets();
        (view.graphics.buildings && below(assets.defines.shrink_distance)) || (view.graphics.city_sprawl && below(self.content.sprawl_cutoff()))
    }

    fn assets(&self) -> &MapBuildingAssets {
        self.content.assets()
    }

    fn mark_slot(&mut self, id: u32, cause: usize) {
        if self.slot_dirty.mark(id, self.frame) {
            self.marked[1] += 1;
            self.causes[cause] += 1;
        }
    }

    fn mark_slots(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.mark_slot(id, ROW);
        }
    }

    fn mark_city(&mut self, provincial: bool, location: u32) {
        if self.city_dirty[usize::from(provincial)].mark(location, self.frame) {
            self.marked[2] += 1;
            self.causes[CITY] += 1;
        }
    }

    fn set_state(&mut self, province: u32, state: Option<u32>) {
        let at = province as usize;
        if self.province_state.len() <= at {
            self.province_state.resize(at + 1, NO_ROW);
        }
        let state = state.unwrap_or(NO_ROW);
        let old = std::mem::replace(&mut self.province_state[at], state);
        if old != state {
            self.state_provinces.remove(&(old, province));
            if state != NO_ROW {
                self.state_provinces.insert((state, province));
            }
            self.judge(province);
        }
    }

    fn city_moved(&mut self, provincial: bool, location: u32) {
        if provincial {
            self.judge(location);
        } else {
            let provinces: Vec<u32> = self.state_provinces.range((location, 0)..=(location, u32::MAX)).map(|&(_, province)| province).collect();
            provinces.into_iter().for_each(|province| self.judge(province));
        }
    }

    fn judge(&mut self, province: u32) {
        self.uploads[4] += 1;
        let held = self.drawn_class.get(province as usize).copied().unwrap_or(0);
        if self.drawn_by.is_none_or(|thresholds| city_class(self.province_health(province), thresholds) != held) {
            self.index_due.insert(province);
        }
    }

    fn mark_row(&mut self, provincial: bool, key: u128) {
        if let Entry::Vacant(seen) = self.rows_seen.entry((provincial, key)) {
            seen.insert(true);
            self.rows_dirty.push((provincial, key));
            self.marked[0] += 1;
        }
    }

    fn reread(&mut self, mirror: &Mirror, at: usize, slot: u32) -> bool {
        let columns = self.columns().rows[at];
        let defines = self.content.assets().defines;
        let read = (|| {
            let level = mirror.number(columns.level, slot).value()?;
            let health = health_state(level, mirror.number(columns.healthy, slot).value()?, mirror.number(columns.partial, slot).value()?, &defines);
            Some((level, health, mirror.number(columns.flags, slot).value()?, mirror.number(columns.country, slot).value()?))
        })();
        let reads = &mut self.reads[at];
        if reads.len() <= slot as usize {
            reads.resize(slot as usize + 1, None);
        }
        std::mem::replace(&mut reads[slot as usize], read) != read
    }

    fn mark_reading(&mut self, ids: impl IntoIterator<Item = u32>, read: u8) {
        let cause = match read {
            READS_FOG => 1,
            READS_HUB => 2,
            _ => 3,
        };
        for id in ids {
            if self.gate_reads.get(id as usize).is_some_and(|reads| reads & read != 0) {
                self.mark_slot(id, cause);
            }
        }
    }

    fn mark_location(&mut self, location: u32) {
        let content = Arc::clone(&self.content);
        self.mark_reading(content.assets().slots_at(location), READS_DETECTION);
    }

    fn reach(&mut self, provincial: bool, location: u32, index: usize) {
        let content = Arc::clone(&self.content);
        self.mark_slots(content.assets().slots_of(provincial, location, index));
        if self.templates.as_ref().is_ok_and(|templates| templates.contains(&index)) {
            self.mark_city(provincial, location);
        }
    }

    fn mark_all(&mut self, mirrors: &Mirrors) {
        let ordinary = self.assets().ordinary_slots();
        let harbours = self.assets().harbour_count();
        self.slots.truncate(ordinary + harbours);
        while self.slots.len() < ordinary + harbours {
            self.slots.push(PreparedSlot::empty(self.slots.len() as u32));
        }
        self.harbours.resize(harbours, HarbourCounts::default());
        self.gate_reads.resize(ordinary, 0);
        (0..ordinary as u32).for_each(|id| self.mark_slot(id, WHOLE));
        (0..harbours as u32).for_each(|index| {
            if self.harbour_dirty.mark(index, self.frame) {
                self.causes[HARBOUR] += 1;
            }
        });
        self.city.iter_mut().for_each(HashMap::clear);
        let templates = self.templates.clone().unwrap_or_default();
        for (at, table) in TABLES.into_iter().enumerate() {
            let Some(mirror) = rows(mirrors.get(table)) else { continue };
            let locations: Vec<u32> = mirror
                .rows()
                .filter_map(|slot| mirror.key(slot))
                .filter(|&key| matches!(self.classes.get(&(key as u32)), Some(TokenClass::Type(index)) if templates.contains(index)))
                .map(|key| (key >> 32) as u32)
                .collect();
            locations.into_iter().for_each(|location| self.mark_city(at == 1, location));
        }
    }

    fn restart(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.rows_of.clear();
        self.classes.clear();
        self.tokens_of.iter_mut().for_each(Vec::clear);
        self.province_state.clear();
        self.state_provinces.clear();
        self.controller_of.iter_mut().for_each(HashMap::clear);
        self.by_controller.clear();
        self.targets.clear();
        self.target_scope.clear();
        self.rows_dirty.clear();
        self.rows_seen.clear();
        self.reads.iter_mut().for_each(Vec::clear);
        self.tokens_dirty.clear();
        self.controllers_dirty.clear();
        self.links_dirty.clear();
        self.ports_dirty.clear();
        self.targets_dirty.clear();
        self.touched.clear();
        self.slot_dirty.clear();
        self.harbour_dirty.clear();
        self.city_dirty.iter_mut().for_each(Dirty::clear);
        for (at, table) in TABLES.into_iter().enumerate() {
            if let Some(mirror) = rows(mirrors.get(table)) {
                for slot in mirror.rows() {
                    let token = mirror.key(slot).map_or(0, |key| key as u32);
                    if token != 0 {
                        *self.rows_of.entry(token).or_default() += 1;
                    }
                    self.reread(mirror, at, slot);
                }
            }
        }
        let tokens: Vec<u32> = self.rows_of.keys().copied().collect();
        tokens.into_iter().for_each(|token| self.classify(mirrors, token));
        for (at, table) in [TableId::Provinces, TableId::States].into_iter().enumerate() {
            let Some(mirror) = rows(mirrors.get(table)) else { continue };
            for slot in mirror.rows() {
                let Some(location) = mirror.key(slot).map(|key| key as u32) else { continue };
                if let Some(raw) = mirror.number(columns.controller[at], slot).value() {
                    self.controller_of[at].insert(location, raw);
                    self.by_controller.insert((raw, at as u8, location));
                }
                if at == 0 {
                    let state = mirror.number(columns.province_state, slot).value();
                    self.unreadable += u64::from(state.is_none());
                    self.set_state(location, state);
                }
            }
        }
        if let Some(mirror) = mirrors.get(TableId::RaidTargetStatuses).filter(|mirror| mirror.complete()) {
            let ordinals: Vec<u32> = mirror.rows().filter_map(|slot| mirror.key(slot)).map(|key| key as u32).collect();
            for ordinal in ordinals {
                self.read_target(mirror, columns, ordinal);
            }
        }
        self.read_deployments(mirrors);
        self.first_unnamed = self.find_unnamed(mirrors);
        self.mark_all(mirrors);
        (self.wired, self.pages, self.cold, self.city_whole, self.fog_fill, self.warned) = (None, None, true, true, true, None);
        self.index_due.clear();
        self.supply_read.clear();
        if let Some(mirror) = rows(mirrors.get(TableId::ProvinceSupply)) {
            for &province in self.seas.keys() {
                let read = mirror.slot(u128::from(province)).map_or((None, None), |slot| (mirror.number(columns.supply_controller, slot).value(), mirror.number(columns.supply_flags, slot).value().map(|flags| flags & 1)));
                self.supply_read.insert(province, read);
            }
        }
        self.chunks_changed.clear();
    }

    fn classify(&mut self, mirrors: &Mirrors, token: u32) {
        let columns = self.columns();
        let lexical = mirrors.get(TableId::LexicalTokens);
        let held = rows(lexical);
        let literal = held.and_then(|mirror| mirror.slot(u128::from(token)).map(|slot| mirror.text(columns.literal, slot).value().and_then(|id| mirror.string(id))));
        let class = self.content.assets().token_class(token, literal, class(lexical) == 0);
        let old = self.classes.insert(token, class.clone());
        if old.as_ref() == Some(&class) {
            return;
        }
        self.warned = None;
        if let Some(TokenClass::Type(index)) = old {
            self.tokens_of[index].retain(|&held| held != token);
        }
        if let TokenClass::Type(index) = class {
            let tokens = &mut self.tokens_of[index];
            let at = tokens.partition_point(|&held| held > token);
            tokens.insert(at, token);
        }
        self.unnamed_check = true;
        if old.is_none() {
            return;
        }
        let indices: Vec<usize> = [old, Some(class)].into_iter().flatten().filter_map(|class| if let TokenClass::Type(index) = class { Some(index) } else { None }).collect();
        for (at, table) in TABLES.into_iter().enumerate() {
            let Some(mirror) = rows(mirrors.get(table)) else { continue };
            let locations: Vec<u32> = mirror.rows().filter_map(|slot| mirror.key(slot)).filter(|&key| key as u32 == token).map(|key| (key >> 32) as u32).collect();
            for location in locations {
                indices.iter().for_each(|&index| self.reach(at == 1, location, index));
            }
        }
    }

    fn find_unnamed(&self, mirrors: &Mirrors) -> Option<String> {
        let unnamed: HashSet<u32> = self.classes.iter().filter(|(token, class)| self.rows_of.contains_key(token) && matches!(class, TokenClass::Refused { unnamed: true, .. })).map(|(&token, _)| token).collect();
        if unnamed.is_empty() {
            return None;
        }
        for table in TABLES {
            let Some(mirror) = rows(mirrors.get(table)) else { continue };
            let first = mirror.rows().filter_map(|slot| mirror.key(slot)).filter(|&key| unnamed.contains(&(key as u32))).min();
            if let Some(key) = first
                && let Some(TokenClass::Refused { reason, .. }) = self.classes.get(&(key as u32))
            {
                return Some(reason.clone());
            }
        }
        None
    }

    fn read_target(&mut self, mirror: &Mirror, columns: Columns, ordinal: u32) {
        if let Some(scope) = self.target_scope.remove(&ordinal) {
            self.targets.remove(&(scope, ordinal));
            self.mark_location(scope.1);
        }
        let Some(slot) = mirror.slot(u128::from(ordinal)) else { return };
        let number = |column: Num<u32>| mirror.number(column, slot).value();
        let (Some(kind), Some(location), Some(flags), Some(country)) = (number(columns.target_type), number(columns.target_location), number(columns.target_flags), mirror.number(columns.target_country, slot).value()) else {
            self.unreadable += 1;
            return;
        };
        if kind == 0 {
            return;
        }
        let scope = (kind, location, flags, country as u32);
        self.targets.insert((scope, ordinal));
        self.target_scope.insert(ordinal, scope);
        self.mark_location(location);
    }

    fn read_deployments(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.deployments.clear();
        self.deployment_of.clear();
        let Some(mirror) = rows(mirrors.get(TableId::FloatingHarborDeployments)) else { return };
        for slot in mirror.rows() {
            let Some(key) = mirror.key(slot) else { continue };
            let (Some(province), Some(status)) = (mirror.number(columns.deploy_province, slot).value(), mirror.number(columns.deploy_status, slot).value()) else {
                self.unreadable += 1;
                continue;
            };
            let at = ((key >> 32) as u32, province, key as u32);
            self.deployments.insert(at, status);
            self.deployment_of.insert(key, at);
        }
    }

    fn redeploy(&mut self, mirror: &Mirror, columns: Columns, key: u128) {
        let now = mirror.slot(key).and_then(|slot| {
            let province = mirror.number(columns.deploy_province, slot).value();
            let status = mirror.number(columns.deploy_status, slot).value();
            if province.is_none() || status.is_none() {
                self.unreadable += 1;
            }
            Some((((key >> 32) as u32, province?, key as u32), status?))
        });
        let old = self.deployment_of.remove(&key).map(|at| (at, self.deployments.remove(&at)));
        if let Some((at, status)) = now {
            self.deployments.insert(at, status);
            self.deployment_of.insert(key, at);
        }
        if old.map(|(at, status)| (at, status)) == now.map(|(at, status)| (at, Some(status))) {
            return;
        }
        for province in [old.map(|((_, province, _), _)| province), now.map(|((_, province, _), _)| province)].into_iter().flatten() {
            let seas: Vec<u32> = u32::try_from(province).ok().and_then(|province| self.seas.get(&province)).cloned().unwrap_or_default();
            for index in seas {
                if self.harbour_dirty.mark(index, self.frame) {
                    self.causes[HARBOUR] += 1;
                }
            }
        }
    }

    fn deployed(columns: Columns, deployments: &BTreeMap<(u32, i32, u32), u8>, mirrors: &Mirrors, province: u32) -> Option<bool> {
        let supply = mirrors.get(TableId::ProvinceSupply);
        let held = rows(supply).and_then(|mirror| mirror.slot(u128::from(province)).map(|slot| (mirror, slot)));
        let Some((mirror, slot)) = held else {
            return (class(supply) == 0).then_some(false);
        };
        let country = mirror.number(columns.supply_controller, slot).value()?;
        let flags = mirror.number(columns.supply_flags, slot).value()? & 1;
        if flags == 0 || country == 0 {
            return Some(false);
        }
        if class(mirrors.get(TableId::FloatingHarborDeployments)) != 0 {
            return None;
        }
        let country = u32::from(country);
        let province = province as i32;
        Some(deployments.range((country, province, 0)..).next().filter(|((held, at, _), _)| *held == country && *at == province).is_some_and(|(_, &status)| status != 0))
    }

    fn take_vision(&mut self) {
        if !self.fresh_vision.swap(false, Ordering::AcqRel) {
            return;
        }
        let Some((choice, new)) = self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() else { return };
        let old = self.vision.replace((choice, Arc::clone(&new))).map(|(_, old)| old);
        let Some(old) = old else { return };
        if Arc::ptr_eq(&old, &new) {
            return;
        }
        let why = if old.ours() != new.ours() {
            Some("another viewing country")
        } else if old.fog() != new.fog() {
            Some("the fog switched")
        } else if old.visibility_gap().is_some() || new.visibility_gap().is_some() {
            Some("a visibility gap")
        } else {
            None
        };
        let changed = match (why.is_none(), old.shared_province_fog(), new.shared_province_fog()) {
            (true, None, None) => Some(Vec::new()),
            (true, Some(before), Some(after)) => {
                let mut changed = Vec::new();
                if !Arc::ptr_eq(before, after) {
                    let decide = |byte: Option<&u8>| byte.map(|&byte| (byte >= MAP_OBJECT_VISIBILITY, byte >= FLOATING_HARBOR_VISIBILITY));
                    after.changed(before, |province| {
                        if decide(before.get(&province)) != decide(after.get(&province)) {
                            changed.push(province);
                        }
                    });
                }
                Some(changed)
            }
            _ => None,
        };
        match changed {
            Some(provinces) => {
                let content = Arc::clone(&self.content);
                for province in provinces {
                    self.mark_reading(content.assets().slots_in(province), READS_FOG);
                    for &index in self.seas.get(&province).into_iter().flatten() {
                        if self.harbour_dirty.mark(index, self.frame) {
                            self.causes[HARBOUR] += 1;
                        }
                    }
                }
            }
            None => {
                *self.whole_visions.entry(why.unwrap_or("the province fog held on one side only")).or_default() += 1;
                let (ordinary, harbours) = (self.assets().ordinary_slots(), self.assets().harbour_count());
                (0..ordinary as u32).for_each(|id| self.mark_slot(id, WHOLE));
                (0..harbours as u32).for_each(|index| {
                    if self.harbour_dirty.mark(index, self.frame) {
                        self.causes[HARBOUR] += 1;
                    }
                });
            }
        }
    }

    fn take_upstream(&mut self, upper: &dyn Upper) {
        let state = upper.state();
        if self.seen_state.is_none_or(|(ours, ..)| ours != state.0) {
            self.seen_state = Some(state);
            let (ordinary, harbours) = (self.assets().ordinary_slots(), self.assets().harbour_count());
            (0..ordinary as u32).for_each(|id| self.mark_slot(id, WHOLE));
            (0..harbours as u32).for_each(|index| {
                if self.harbour_dirty.mark(index, self.frame) {
                    self.causes[HARBOUR] += 1;
                }
            });
            self.fog_fill = true;
            return;
        }
        let moved = match upper.moved() {
            Change::Marked(moved) if self.seen_state == Some(state) => moved,
            _ => {
                self.seen_state = Some(state);
                for at in 0..self.fog_provinces.len() {
                    self.fog_dirty.mark(self.fog_provinces[at], self.frame);
                }
                return;
            }
        };
        for province in moved.of("province").iter().map(|&id| id as u32) {
            if self.fog_seen.get(province as usize).is_some_and(|&seen| seen != NOT_SLOTTED) {
                self.fog_dirty.mark(province, self.frame);
            }
        }
    }

    fn settle_fog(&mut self, vision: &dyn Seen) -> io::Result<()> {
        if std::mem::take(&mut self.fog_fill) {
            for &province in &self.fog_provinces {
                self.fog_seen[province as usize] = fog_decisions(vision.byte(province));
            }
            self.fog_dirty.clear();
            return Ok(());
        }
        let mut moved = Vec::new();
        let (seen, scoped) = (&mut self.fog_seen, &mut self.scoped);
        self.fog_dirty.settle(|province| {
            scoped.province(province);
            let decided = fog_decisions(vision.byte(province));
            let held = &mut seen[province as usize];
            if *held != decided {
                *held = decided;
                moved.push(province);
            }
            Ok(())
        })?;
        let content = Arc::clone(&self.content);
        for province in moved {
            self.mark_reading(content.assets().slots_in(province), READS_FOG);
            for &index in self.seas.get(&province).into_iter().flatten() {
                if self.harbour_dirty.mark(index, self.frame) {
                    self.causes[HARBOUR] += 1;
                }
            }
        }
        Ok(())
    }

    fn prepare(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        for token in std::mem::take(&mut self.tokens_dirty) {
            if self.rows_of.contains_key(&token) {
                self.classify(mirrors, token);
            }
        }
        let seen = std::mem::take(&mut self.rows_seen);
        for (provincial, key) in std::mem::take(&mut self.rows_dirty) {
            let at = usize::from(provincial);
            let lived = seen.get(&(provincial, key)).copied().unwrap_or(true);
            let mirror = rows(mirrors.get(TABLES[at]));
            if let Some((mirror, slot)) = mirror.and_then(|mirror| mirror.slot(key).map(|slot| (mirror, slot))) {
                self.reread(mirror, at, slot);
            }
            let (location, token) = ((key >> 32) as u32, key as u32);
            match self.classes.get(&token) {
                Some(&TokenClass::Type(index)) => self.reach(provincial, location, index),
                Some(TokenClass::Refused { unnamed: true, .. }) if lived => self.unnamed_check = true,
                _ => {}
            }
        }
        self.rows_seen = seen;
        self.rows_seen.clear();
        for (at, location) in std::mem::take(&mut self.controllers_dirty) {
            let table = [TableId::Provinces, TableId::States][at as usize];
            let mirror = rows(mirrors.get(table));
            let slot = mirror.and_then(|mirror| mirror.slot(u128::from(location)).map(|slot| (mirror, slot)));
            let raw = slot.and_then(|(mirror, slot)| mirror.number(columns.controller[at as usize], slot).value());
            let old = match raw {
                Some(raw) => self.controller_of[at as usize].insert(location, raw),
                None => self.controller_of[at as usize].remove(&location),
            };
            if old != raw {
                if let Some(old) = old {
                    self.by_controller.remove(&(old, at, location));
                }
                if let Some(raw) = raw {
                    self.by_controller.insert((raw, at, location));
                }
                self.mark_location(location);
            }
            if at == 0 {
                let state = slot.and_then(|(mirror, slot)| mirror.number(columns.province_state, slot).value());
                self.set_state(location, state);
            }
        }
        let links = std::mem::take(&mut self.links_dirty);
        if !links.is_empty() {
            let locations: Vec<u32> = links.iter().flat_map(|&raw| self.by_controller.range((raw, 0, 0)..=(raw, u8::MAX, u32::MAX)).map(|&(_, _, location)| location)).collect();
            locations.into_iter().for_each(|location| self.mark_location(location));
            for (at, table) in TABLES.into_iter().enumerate() {
                let Some(mirror) = rows(mirrors.get(table)) else { continue };
                let overridden: Vec<u128> = mirror.rows().filter(|&slot| mirror.number(columns.rows[at].country, slot).value().is_some_and(|country| links.contains(&country))).filter_map(|slot| mirror.key(slot)).collect();
                for key in overridden {
                    if let Some(&TokenClass::Type(index)) = self.classes.get(&(key as u32)) {
                        self.reach(at == 1, (key >> 32) as u32, index);
                    }
                }
            }
        }
        let content = Arc::clone(&self.content);
        for province in std::mem::take(&mut self.ports_dirty) {
            self.mark_reading(content.assets().slots_in(province), READS_HUB);
        }
        let targets = std::mem::take(&mut self.targets_dirty);
        if let Some(mirror) = mirrors.get(TableId::RaidTargetStatuses).filter(|mirror| mirror.complete()) {
            for ordinal in targets {
                self.read_target(mirror, columns, ordinal);
            }
        }
        if std::mem::take(&mut self.unnamed_check) {
            self.first_unnamed = self.find_unnamed(mirrors);
        }
    }

    fn city_refusal(&self, mirrors: &Mirrors) -> Option<String> {
        let hour = mirrors.get(TableId::Provinces).and_then(Mirror::head);
        for table in [TableId::StateBuildings, TableId::ProvinceBuildings, TableId::LexicalTokens] {
            if let Some(why) = not_whole(mirrors.get(table), table, hour) {
                return Some(why);
            }
        }
        if let Some(why) = &self.first_unnamed {
            return Some(why.clone());
        }
        if let Some(why) = not_whole(mirrors.get(TableId::Provinces), TableId::Provinces, hour) {
            return Some(format!("{why}; the core tables {CORE} carry the political map itself, so this hour cannot be drawn"));
        }
        self.templates.as_ref().err().cloned()
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors, upper: Option<&dyn Upper>) -> io::Result<()> {
        self.frame += 1;
        let Some(columns) = self.columns else { return Ok(()) };
        self.take_vision();
        if self.reads_layer
            && let Some(upper) = upper
        {
            self.take_upstream(upper);
        }
        let held = READ.map(|table| class(mirrors.get(table)));
        if self.held != Some(held) {
            self.held = Some(held);
            self.restart(mirrors);
        }
        self.take_touched(mirrors);
        let links = mirrors.get(TableId::CountryLinks);
        let hour = mirrors.get(TableId::Provinces).and_then(Mirror::head);
        let given = self.vision.as_ref().map(|(_, vision)| Arc::clone(vision));
        let upper_refused = upper.and_then(|upper| upper.refused());
        let sight: Option<&dyn Seen> = if self.reads_layer { upper.filter(|_| upper_refused.is_none()).map(|upper| upper.seen()) } else { given.as_deref().map(|vision| vision as &dyn Seen) };
        self.refused = match (sight, not_whole(links, TableId::CountryLinks, hour)) {
            (None, _) if self.reads_layer => Some(format!("The buildings layer is refused: the vision layer has no vision: {}", upper_refused.unwrap_or_else(|| "it is not registered before it".into()))),
            (None, _) => Some("The buildings layer is refused: it is given no vision of the viewing country yet".into()),
            (_, Some(why)) => Some(format!("{why}; the core tables {CORE} carry the political map itself, so this hour cannot be drawn")),
            _ => None,
        };
        if let Some(hour) = hour {
            if hour < self.hour {
                self.cold = true;
            }
            self.hour = hour;
        }
        if self.refused.is_some() {
            return Ok(());
        }
        self.prepare(mirrors);
        let city_refused = self.city_refusal(mirrors);
        self.city_whole |= city_refused != self.city_refused;
        if self.city_refused != city_refused {
            self.warned = None;
        }
        self.city_refused = city_refused;
        if self.city_refused.is_some() {
            let held = rows(mirrors.get(TableId::Provinces)).map_or(0, |mirror| mirror.rows().count());
            if held != self.provinces_held {
                (self.provinces_held, self.warned) = (held, None);
            }
        }
        let vision = sight.expect("a vision when not refused");
        if self.reads_layer {
            self.settle_fog(vision)?;
        }
        let content = Arc::clone(&self.content);
        let assets = content.assets();
        let raid = mirrors.get(TableId::RaidTargetStatuses);
        let naval = mirrors.get(TableId::NavalBases);
        let gates = Gates {
            vision,
            city: content.city(),
            columns,
            naval: rows(naval),
            naval_complete: class(naval) == 0,
            controllers: [rows(mirrors.get(TableId::Provinces)), rows(mirrors.get(TableId::States))],
            links: rows(links),
            targets: raid.filter(|mirror| mirror.complete()).map(|mirror| (&self.targets, mirror)),
        };
        let found = Found { tables: TABLES.map(|table| rows(mirrors.get(table))), columns, tokens_of: &self.tokens_of };
        let unreadable = std::cell::Cell::new(0u64);
        let (slots, gate_reads, chunks, scoped) = (&mut self.slots, &mut self.gate_reads, &mut self.chunks_changed, &mut self.scoped);
        self.slot_dirty.settle(|id| {
            match assets.slot_province(id) {
                Some(province) => scoped.province(province),
                None => scoped.unplaced(),
            }
            let (slot, reads) = assets.slot_value(id, |key| found.row(key, &unreadable), &gates);
            if slots[id as usize] != slot {
                slots[id as usize] = slot;
                chunks.insert(id as usize / CHUNK);
            }
            gate_reads[id as usize] = reads;
            Ok(())
        })?;
        let ordinary = assets.ordinary_slots();
        let (slots, harbours, deployments, chunks, warned, scoped) = (&mut self.slots, &mut self.harbours, &self.deployments, &mut self.chunks_changed, &mut self.warned, &mut self.scoped);
        self.harbour_dirty.settle(|index| {
            scoped.province(assets.harbour_sea(index as usize));
            let (slot, counts) = assets.harbour_value(index as usize, |province| vision.byte(province), |province| Self::deployed(columns, deployments, mirrors, province));
            let id = ordinary + index as usize;
            if slots[id] != slot {
                slots[id] = slot;
                chunks.insert(id / CHUNK);
            }
            if harbours[index as usize] != counts {
                harbours[index as usize] = counts;
                *warned = None;
            }
            Ok(())
        })?;
        if self.city_refused.is_none() {
            let templates = self.templates.as_deref().unwrap_or_default();
            let mut moved = Vec::new();
            for (at, provincial) in [false, true].into_iter().enumerate() {
                let (city, scoped) = (&mut self.city[at], &mut self.scoped);
                self.city_dirty[at].settle(|location| {
                    if provincial {
                        scoped.province(location);
                    } else {
                        scoped.state(location);
                    }
                    let least = templates
                        .iter()
                        .filter(|&&index| assets.types()[index].provincial() == provincial)
                        .filter_map(|&index| found.row((provincial, location, index), &unreadable))
                        .filter(|row| row.level > 0)
                        .map(|row| average_health(row.level, row.healthy_level, row.partial_health))
                        .min();
                    let before = match least {
                        Some(least) => city.insert(location, least),
                        None => city.remove(&location),
                    };
                    if before != least {
                        moved.push((provincial, location));
                    }
                    Ok(())
                })?;
            }
            for (provincial, location) in moved {
                self.city_moved(provincial, location);
            }
        }
        self.unreadable += unreadable.get();
        Ok(())
    }

    fn groups(table: TableId, column: usize, columns: Columns) -> u8 {
        let read = |read: &[usize]| u8::from(read.contains(&column));
        let row = |rows: Rows| read(&[rows.country.column, rows.flags.column, rows.level.column, rows.healthy.column, rows.partial.column]);
        match table {
            TableId::StateBuildings => row(columns.rows[0]),
            TableId::ProvinceBuildings => row(columns.rows[1]),
            TableId::LexicalTokens => read(&[columns.literal.column]),
            TableId::Provinces => read(&[columns.controller[0].column, columns.province_state.column]),
            TableId::States => read(&[columns.controller[1].column]),
            TableId::CountryLinks => read(&[columns.link.column]),
            TableId::ProvinceSupply => read(&[columns.supply_controller.column, columns.supply_flags.column]),
            TableId::FloatingHarborDeployments => read(&[columns.deploy_province.column, columns.deploy_status.column]),
            TableId::RaidTargetStatuses => read(&[columns.target_type.column, columns.target_location.column, columns.target_flags.column, columns.target_country.column, columns.target_detected.column]),
            _ => 0,
        }
    }

    fn take_touched(&mut self, mirrors: &Mirrors) {
        let mut touched = std::mem::take(&mut self.touched);
        touched.take(mirrors, |table, touch, key| {
            if let Some(mirror) = rows(mirrors.get(table)) {
                self.touch(mirror, table, touch, key);
            }
        });
        self.touched = touched;
    }

    fn touch(&mut self, mirror: &Mirror, table: TableId, touch: Touch, key: u128) {
        let columns = self.columns();
        match table {
            TableId::StateBuildings | TableId::ProvinceBuildings => self.touch_row(mirror, table == TableId::ProvinceBuildings, touch, key),
            TableId::LexicalTokens => {
                if self.rows_of.contains_key(&(key as u32)) {
                    self.tokens_dirty.insert(key as u32);
                }
            }
            TableId::Provinces => self.controllers_dirty.push((0, key as u32)),
            TableId::States => self.controllers_dirty.push((1, key as u32)),
            TableId::CountryLinks => {
                self.links_dirty.insert(key as u32);
            }
            TableId::NavalBases => self.ports_dirty.push(key as u32),
            TableId::ProvinceSupply => {
                let province = key as u32;
                if !self.seas.contains_key(&province) {
                    return;
                }
                let read = mirror.slot(key).map_or((None, None), |slot| (mirror.number(columns.supply_controller, slot).value(), mirror.number(columns.supply_flags, slot).value().map(|flags| flags & 1)));
                if self.supply_read.insert(province, read) == Some(read) {
                    return;
                }
                for &index in self.seas.get(&province).into_iter().flatten() {
                    if self.harbour_dirty.mark(index, self.frame) {
                        self.causes[HARBOUR] += 1;
                    }
                }
            }
            TableId::FloatingHarborDeployments => self.redeploy(mirror, columns, key),
            TableId::RaidTargetStatuses => {
                self.targets_dirty.insert(key as u32);
            }
            _ => {}
        }
    }

    fn touch_row(&mut self, mirror: &Mirror, provincial: bool, touch: Touch, key: u128) {
        match touch {
            Touch::Gone | Touch::Came => {
                self.count(key as u32, touch == Touch::Came);
                self.mark_row(provincial, key);
            }
            Touch::Written(_, slot) => {
                self.causes[WRITTEN] += 1;
                if self.reread(mirror, usize::from(provincial), slot) {
                    self.mark_row(provincial, key);
                    return;
                }
                self.causes[HELD] += 1;
                if let Some(&TokenClass::Type(index)) = self.classes.get(&(key as u32))
                    && self.templates.as_ref().is_ok_and(|templates| templates.contains(&index))
                {
                    self.mark_city(provincial, (key >> 32) as u32);
                }
            }
        }
    }

    fn count(&mut self, token: u32, add: bool) {
        if token == 0 {
            return;
        }
        self.warned = None;
        if add {
            let count = self.rows_of.entry(token).or_default();
            *count += 1;
            if *count == 1 {
                self.tokens_dirty.insert(token);
            }
        } else if let Some(count) = self.rows_of.get_mut(&token) {
            *count -= 1;
            if *count == 0 {
                self.rows_of.remove(&token);
                self.unnamed_check = true;
            }
        }
    }

    fn seen_ours(&self) -> u32 {
        match (&self.vision, self.seen_state) {
            (_, Some((Some(ours), ..))) if self.reads_layer => ours,
            (Some((_, vision)), _) => vision.ours(),
            _ => 0,
        }
    }

    fn province_health(&self, province: u32) -> Option<Option<i64>> {
        let full = |at: usize, location: u32| self.city[at].get(&location).copied().unwrap_or(FULL_HEALTH);
        let state = self.province_state.get(province as usize).copied().unwrap_or(NO_ROW);
        (self.city_refused.is_none() && state != NO_ROW).then(|| (state != 0).then(|| full(0, state).min(full(1, province))))
    }

    fn note_whole(&mut self, thresholds: Option<(i64, i64)>) {
        self.drawn_by = thresholds;
        self.drawn_class.clear();
        if let Some(thresholds) = thresholds {
            self.drawn_class = (0..self.province_state.len() as u32).map(|province| city_class(self.province_health(province), thresholds)).collect();
        }
    }

    fn health_index(&mut self, thresholds: Option<(i64, i64)>) -> io::Result<ValueIndex<u32, Option<i64>>> {
        if self.city_whole {
            self.uploads[5] += 1;
            self.health_index = ValueIndex::try_from_iter(self.health())?;
            self.index_due.clear();
            self.note_whole(thresholds);
            return Ok(self.health_index.clone());
        }
        if self.index_due.is_empty() {
            return Ok(self.health_index.clone());
        }
        let due = std::mem::take(&mut self.index_due);
        self.uploads[2] += 1;
        self.uploads[3] += due.len() as u64;
        let edits: Vec<(u32, Option<Option<i64>>)> = due.iter().map(|&province| (province, self.province_health(province))).collect();
        self.health_index = self.health_index.patched(&edits)?;
        if let Some(thresholds) = self.drawn_by {
            for (province, health) in edits {
                let at = province as usize;
                if self.drawn_class.len() <= at {
                    self.drawn_class.resize(at + 1, 0);
                }
                self.drawn_class[at] = city_class(health, thresholds);
            }
        }
        Ok(self.health_index.clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        self.inspect().differ(&other.inspect())
    }
}

struct Found<'a> {
    tables: [Option<&'a Mirror>; 2],
    columns: Columns,
    tokens_of: &'a [Vec<u32>],
}

impl Found<'_> {
    fn row(&self, (provincial, location, index): (bool, u32, usize), unreadable: &std::cell::Cell<u64>) -> Option<BuildingInput> {
        let mirror = self.tables[usize::from(provincial)]?;
        let columns = self.columns.rows[usize::from(provincial)];
        for &token in &self.tokens_of[index] {
            let Some(slot) = mirror.slot(u128::from(location) << 32 | u128::from(token)) else { continue };
            let row = (|| {
                Some(BuildingInput {
                    location,
                    building_type: token,
                    country: mirror.number(columns.country, slot).value()?,
                    status_flags: mirror.number(columns.flags, slot).value()?,
                    level: mirror.number(columns.level, slot).value()?,
                    healthy_level: mirror.number(columns.healthy, slot).value()?,
                    partial_health: mirror.number(columns.partial, slot).value()?,
                })
            })();
            if row.is_none() {
                unreadable.set(unreadable.get() + 1);
            }
            return row;
        }
        None
    }
}

struct Gates<'a> {
    vision: &'a dyn Seen,
    city: CityTerrain<'a>,
    columns: Columns,
    naval: Option<&'a Mirror>,
    naval_complete: bool,
    controllers: [Option<&'a Mirror>; 2],
    links: Option<&'a Mirror>,
    targets: Option<(&'a BTreeSet<(Scope, u32)>, &'a Mirror)>,
}

impl SlotGates for Gates<'_> {
    fn sees(&self, province: u32, always_shown: bool) -> Option<bool> {
        if always_shown {
            return Some(true);
        }
        self.vision.byte(province).map(|byte| byte >= MAP_OBJECT_VISIBILITY)
    }

    fn on_city(&self, x: f32, z: f32) -> bool {
        self.city.at(x, z)
    }

    fn naval_base(&self, province: u32) -> bool {
        self.naval.is_some_and(|mirror| mirror.slot(u128::from(province)).is_some())
    }

    fn naval_bases_complete(&self) -> bool {
        self.naval_complete
    }

    fn detected(&self, row: &BuildingInput) -> Option<bool> {
        let ours = self.vision.ours();
        detected(
            row,
            |provincial, location| {
                let at = if provincial { 0 } else { 1 };
                let mirror = self.controllers[at]?;
                mirror.number(self.columns.controller[at], mirror.slot(u128::from(location))?).value()
            },
            |raw| {
                let mirror = self.links?;
                mirror.number(self.columns.link, mirror.slot(u128::from(raw))?).value()
            },
            ours,
            |scope| {
                let (targets, mirror) = self.targets?;
                Some(targets.range((scope, 0)..=(scope, u32::MAX)).next_back().map(|&(_, ordinal)| {
                    let bytes = mirror.slot(u128::from(ordinal)).and_then(|slot| mirror.text(self.columns.target_detected, slot).value()).and_then(|id| mirror.string(id));
                    let slot = ours as usize;
                    bytes.and_then(|bytes| bytes.get(slot / 8)).is_some_and(|byte| byte & (1 << (slot % 8)) != 0)
                }))
            },
        )
    }
}

impl Layer for Buildings {
    fn name(&self) -> &'static str {
        "buildings"
    }

    fn placed(&mut self, view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(u64::from(self.drawn(view)))
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if !self.drawn(view) {
            self.declare(Declared { columns: None, fog: view.fog });
            return Ok(());
        }
        if self.reads_layer {
            inputs.after("vision");
        }
        for table in READ {
            inputs.table(table, true)?;
        }
        let rows = |inputs: &mut Inputs<'_>, table: TableId| -> io::Result<Rows> {
            Ok(Rows {
                country: inputs.number(table, "country")?,
                flags: inputs.number(table, "status_flags")?,
                level: inputs.number(table, "level")?,
                healthy: inputs.number(table, "healthy_level")?,
                partial: inputs.number(table, "partial_health")?,
            })
        };
        let columns = Columns {
            rows: [rows(inputs, TableId::StateBuildings)?, rows(inputs, TableId::ProvinceBuildings)?],
            literal: inputs.text(TableId::LexicalTokens, "literal")?,
            province_state: inputs.number(TableId::Provinces, "state")?,
            controller: [inputs.number(TableId::Provinces, "controller")?, inputs.number(TableId::States, "controller")?],
            link: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            supply_controller: inputs.number(TableId::ProvinceSupply, "controller_index")?,
            supply_flags: inputs.number(TableId::ProvinceSupply, "flags")?,
            deploy_province: inputs.number(TableId::FloatingHarborDeployments, "province")?,
            deploy_status: inputs.number(TableId::FloatingHarborDeployments, "status")?,
            target_type: inputs.number(TableId::RaidTargetStatuses, "scope.building_type")?,
            target_location: inputs.number(TableId::RaidTargetStatuses, "scope.building_location")?,
            target_flags: inputs.number(TableId::RaidTargetStatuses, "scope.building_flags")?,
            target_country: inputs.number(TableId::RaidTargetStatuses, "scope.building_country")?,
            target_detected: inputs.text(TableId::RaidTargetStatuses, "detected")?,
        };
        self.declare(Declared { columns: Some(columns), fog: view.fog });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.held = None;
        self.settle_dirty(mirrors, None)?;
        self.marked = marked;
        Ok(())
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        let upper = FromLayer::of(upstream);
        self.compose_upper(mirrors, upper.as_ref().map(|upper| upper as &dyn Upper))
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _settling: Settling) -> io::Result<()> {
        let upper = FromLayer::of(upstream);
        self.settle_dirty(mirrors, upper.as_ref().map(|upper| upper as &dyn Upper))
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        let reads = viewer_layer_core::playhead_host::mirror::probe::reads();
        let columns = self.columns();
        let mut order = READ;
        order.sort_unstable_by_key(|table| table.index());
        for table in order {
            if let Some(changes) = step.table(table.index()) {
                self.touched.note(changes, self.frame, |column| Self::groups(table, column, columns));
            }
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads += viewer_layer_core::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors, None)
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        if self.columns.is_none() || self.refused.is_some() {
            return Ok(());
        }
        let thresholds = gpu.scene.city_damage_thresholds();
        self.note_thresholds(thresholds);
        if self.uploaded() {
            return Ok(());
        }
        let Some(install) = self.make(gpu.content.products.display_binding()?, thresholds)? else { return Ok(()) };
        let started = Instant::now();
        self::install(gpu, install)?;
        self.upload_ns[2] += Instant::now().duration_since(started).as_nanos() as u64;
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.columns.is_none() || (self.wired.is_some() && self.chunks_changed.is_empty() && !self.city_whole && self.index_due.is_empty())
    }

    fn carried(&self) -> Carried {
        if self.columns.is_none() || self.refused.is_some() {
            return Carried::default();
        }
        let city = if self.city_refused.is_none() { &self.city_dirty[..] } else { &[] };
        let entities = self.slot_dirty.len() + self.harbour_dirty.len() + self.fog_dirty.len() + city.iter().map(Dirty::len).sum::<usize>() + self.touched.len();
        Carried { entities }
    }

    fn scope(&mut self, sight: &InView<'_>) -> Option<Settled> {
        Some(self.scoped.take(sight, None))
    }

    fn held(&self) -> usize {
        self.slots.iter().map(PreparedSlot::held).sum::<usize>()
            + self.pages.as_ref().map_or(0, SlotPages::held)
            + self.gate_reads.capacity()
            + self.fog_provinces.capacity() * 4
            + self.fog_seen.capacity()
            + self.fog_dirty.bytes()
            + self.scoped.bytes()
            + self.harbours.capacity() * std::mem::size_of::<HarbourCounts>()
            + self.seas.capacity() * 48
            + (self.rows_of.capacity() + self.classes.capacity()) * 48
            + self.tokens_of.iter().map(|tokens| tokens.capacity() * 4 + 24).sum::<usize>()
            + self.city.iter().map(|city| city.capacity() * 16).sum::<usize>()
            + self.province_state.capacity() * 4
            + self.state_provinces.len() * 16
            + self.controller_of.iter().map(|held| held.capacity() * 12).sum::<usize>()
            + self.by_controller.len() * 16
            + self.targets.len() * 24
            + self.target_scope.capacity() * 24
            + self.deployments.len() * 16
            + self.rows_dirty.capacity() * 24
            + self.rows_seen.capacity() * 40
            + self.reads.iter().map(|reads| reads.capacity() * std::mem::size_of::<Option<Read>>()).sum::<usize>()
            + self.slot_dirty.bytes()
            + self.harbour_dirty.bytes()
            + self.touched.bytes()
            + self.city_dirty.iter().map(Dirty::bytes).sum::<usize>()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

fn install(gpu: &mut Gpu<'_>, (product, continues): Install) -> io::Result<()> {
    let continued: &[&wire::Buildings] = &[];
    gpu.scene.set_map_buildings(&gpu.content.map_buildings, product, continues.then_some(continued))
}

pub fn summary(side: &viewer_layer_core::playhead_host::MapSide<impl viewer_layer_core::playhead_host::PlaybackProducts>) -> serde_json::Value {
    side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<BuildingsFed>())).map_or(serde_json::Value::Null, BuildingsFed::figures)
}

#[cfg(test)]
pub mod tests;
