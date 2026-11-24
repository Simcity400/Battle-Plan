
use viewer_layer_core::layers::{Carried, Settling, Dirty, Gpu, Inputs, Installs, Layer, Num, Scope, Scoped, Shown, Sight, Text, View, Wrote};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use content_reader_map::map::{MapData, ProvinceKind};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};
use viewer_borders::{
    borders::{BorderClass, BorderMeshes},
    border_pass::ClassPlan,
    demilitarized::{Zone, ZonePlan},
};
use viewer_map_modes::{map_mode::MapMode, political::UNAVAILABLE_COLOR};
use map_display_data::landscape::{Attributes, Regions};

const NONE: u32 = u32::MAX;
pub const LEFT_OUT: u8 = u8::MAX;
pub const BUCKETS: [BorderClass; 4] = [BorderClass::Province, BorderClass::Impassable, BorderClass::Sea, BorderClass::SeaRegion];

const LAND: u8 = 1;
const SEA: u8 = 2;
const LAKE: u8 = 3;

pub struct Content {
    map: Arc<MapData>,
    meshes: Arc<BorderMeshes>,
    kind: Vec<u8>,
    max_defined: u32,
    edges: Vec<(u32, u32)>,
    chunk_of: Vec<u32>,
    edge_of_draw: Vec<u32>,
    draws: Vec<Vec<u32>>,
    bucket_start: [usize; BUCKETS.len() + 1],
    edge_start: Vec<u32>,
    edge_list: Vec<u32>,
    draw_start: Vec<u32>,
    draw_list: Vec<(BorderClass, u32)>,
}

impl Content {
    pub fn new(map: Arc<MapData>, meshes: Arc<BorderMeshes>) -> Self {
        let max_defined = map.definitions.last_key_value().map_or(0, |(&id, _)| id);
        let mut kind = vec![0; max_defined as usize + 1];
        for (&id, definition) in &map.definitions {
            kind[id as usize] = match definition.kind {
                ProvinceKind::Land => LAND,
                ProvinceKind::Sea => SEA,
                ProvinceKind::Lake => LAKE,
            };
        }
        let (mut edges, mut chunk_of, mut edge_of_draw) = (Vec::new(), Vec::new(), Vec::new());
        let mut bucket_start = [0; BUCKETS.len() + 1];
        for (at, class) in BUCKETS.iter().enumerate() {
            bucket_start[at] = edges.len();
            let chunks = meshes.meshes.get(class.bucket()).map_or(0, |mesh| mesh.chunks.len());
            if let Some(mesh) = meshes.meshes.get(class.bucket()) {
                if at == 0 {
                    for draw in &mesh.draws {
                        if (draw.chunk as usize) < chunks {
                            edge_of_draw.push(edges.len() as u32);
                            chunk_of.push(draw.chunk);
                            edges.push((draw.province_a, draw.province_b));
                        } else {
                            edge_of_draw.push(NONE);
                        }
                    }
                } else {
                    edges.extend(mesh.draws.iter().filter(|draw| (draw.chunk as usize) < chunks).map(|draw| (draw.province_a, draw.province_b)));
                }
            }
        }
        bucket_start[BUCKETS.len()] = edges.len();
        let provinces = edges.iter().map(|&(a, b)| a.max(b) as usize + 1).max().unwrap_or(0).max(kind.len());
        let mut counts = vec![0u32; provinces + 1];
        for &(a, b) in &edges {
            counts[a as usize + 1] += 1;
            if b != a {
                counts[b as usize + 1] += 1;
            }
        }
        for at in 1..counts.len() {
            counts[at] += counts[at - 1];
        }
        let mut fill = counts.clone();
        let mut edge_list = vec![0; *counts.last().unwrap_or(&0) as usize];
        for (edge, &(a, b)) in edges.iter().enumerate() {
            for province in if a == b { vec![a] } else { vec![a, b] } {
                edge_list[fill[province as usize] as usize] = edge as u32;
                fill[province as usize] += 1;
            }
        }
        let draws = ClassPlan::chunk_draws(&meshes);
        let all = || BorderClass::ALL.iter().filter_map(|&class| meshes.meshes.get(class.bucket()).map(|mesh| (class, mesh))).flat_map(|(class, mesh)| mesh.draws.iter().enumerate().map(move |(at, draw)| (class, at as u32, draw.province_a, draw.province_b)));
        let largest = all().map(|(_, _, a, b)| a.max(b) as usize + 1).max().unwrap_or(0);
        let mut draw_start = vec![0u32; largest + 1];
        for (_, _, a, b) in all() {
            draw_start[a as usize + 1] += 1;
            if b != a {
                draw_start[b as usize + 1] += 1;
            }
        }
        for at in 1..draw_start.len() {
            draw_start[at] += draw_start[at - 1];
        }
        let mut fill = draw_start.clone();
        let mut draw_list = vec![(BorderClass::Province, 0); *draw_start.last().unwrap_or(&0) as usize];
        for (class, at, a, b) in all() {
            for province in if a == b { vec![a] } else { vec![a, b] } {
                draw_list[fill[province as usize] as usize] = (class, at);
                fill[province as usize] += 1;
            }
        }
        Self { map, meshes, kind, max_defined, edges, chunk_of, edge_of_draw, draws, bucket_start, edge_start: counts, edge_list, draw_start, draw_list }
    }

    fn kind(&self, province: u32) -> u8 {
        self.kind.get(province as usize).copied().unwrap_or(0)
    }

    fn draws_of(&self, province: u32) -> &[(BorderClass, u32)] {
        let at = province as usize;
        if at + 1 >= self.draw_start.len() {
            return &[];
        }
        &self.draw_list[self.draw_start[at] as usize..self.draw_start[at + 1] as usize]
    }

    fn edges_of(&self, province: u32) -> &[u32] {
        let at = province as usize;
        if at + 1 >= self.edge_start.len() {
            return &[];
        }
        &self.edge_list[self.edge_start[at] as usize..self.edge_start[at + 1] as usize]
    }

    pub fn edges(&self) -> (&[(u32, u32)], &[usize]) {
        (&self.edges, &self.bucket_start)
    }

    fn bytes(&self) -> usize {
        self.kind.capacity() + self.edges.capacity() * 8 + self.edge_start.capacity() * 4 + self.edge_list.capacity() * 4
    }
}

#[derive(Clone, Copy)]
struct Columns {
    province_state: Num<u32>,
    province_owner: Num<u32>,
    province_controller: Num<u32>,
    country_slot: Num<u32>,
    country_tag: Text,
    link_slot: Num<u32>,
    red: Num<u32>,
    green: Num<u32>,
    blue: Num<u32>,
    subject_start: Num<i32>,
    subject_end: Num<i32>,
    subject_cancel: Num<u8>,
    autonomy_state: Text,
    demilitarized: Num<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    state: Option<u32>,
    owner: Option<u32>,
    controller: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Relations {
    inherited: BTreeMap<u32, u32>,
    undecided: BTreeMap<u32, String>,
    states: BTreeMap<u32, Vec<u8>>,
    autonomy_gap: Option<String>,
    error: Option<String>,
    warnings: Vec<String>,
    next_end: Option<Hour>,
}

impl Relations {
    fn source(&self, slot: u32) -> Result<Option<u32>, String> {
        let mut source = slot;
        for _ in 0..=self.inherited.len() {
            if self.undecided.contains_key(&source) || (self.autonomy_gap.is_some() && !self.states.contains_key(&source)) {
                return Ok(None);
            }
            match self.inherited.get(&source) {
                Some(&next) => source = next,
                None => return Ok(Some(source)),
            }
        }
        Err("cycle in subject colour inheritance".into())
    }

    fn colours_same(&self, other: &Self) -> bool {
        (&self.inherited, &self.undecided, &self.states, &self.autonomy_gap) == (&other.inherited, &other.undecided, &other.states, &other.autonomy_gap)
    }
}

fn gap(mirror: Option<&Mirror>, name: &str) -> Option<String> {
    match mirror.and_then(Mirror::status) {
        Some(Status::Complete) => None,
        Some(Status::Truncated { missing }) => Some(format!("{name} is truncated ({missing} missing rows)")),
        Some(Status::Unavailable { .. }) => Some(format!("{name} is unavailable: {}", mirror.and_then(Mirror::reason).unwrap_or_default())),
        Some(Status::NotRecorded) | None => Some(match mirror.and_then(Mirror::capture) {
            Some(mode) => format!("{name} is not recorded in a {mode} recording"),
            None => format!("{name} is not recorded at this hour"),
        }),
    }
}

fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn missing_link(reference: u32, role: &str) -> String {
    format!("political fill references missing country link {reference} for {role}")
}

fn examples(mut items: impl Iterator<Item = String>) -> String {
    let mut text = String::new();
    for (at, item) in items.by_ref().take(12).enumerate() {
        if at != 0 {
            text.push_str(", ");
        }
        text.push_str(&item);
    }
    if items.next().is_some() {
        text.push_str(", …");
    }
    text
}

fn digest(keys_of: &[u32], colours: &[Option<[u8; 3]>]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    };
    for key in keys_of {
        key.to_le_bytes().into_iter().for_each(&mut eat);
    }
    for (key, colour) in colours.iter().enumerate() {
        if let Some(colour) = colour {
            (key as u32).to_le_bytes().into_iter().for_each(&mut eat);
            colour.iter().copied().for_each(&mut eat);
        }
    }
    hash
}

struct Set {
    hour: Hour,
    from: Hour,
    composed: u64,
    whole: Option<Whole>,
    provinces: Vec<(u32, u32)>,
    keys: Vec<(u32, Option<[u8; 3]>)>,
    naval: Vec<(u32, Attributes)>,
    plans: Option<(ClassPlan, ZonePlan)>,
    built_ns: u64,
    digest: Option<u64>,
}

type DatedHour = (Hour, Vec<(u32, u32)>, Vec<(u32, Option<[u8; 3]>)>);

fn stamp() -> u64 {
    static STAMPS: AtomicU64 = AtomicU64::new(0);
    STAMPS.fetch_add(1, Ordering::Relaxed) + 1
}

const SETS_HELD: usize = 4096;

#[derive(Default)]
struct Handover(Mutex<VecDeque<Set>>);

impl Handover {
    fn push(&self, set: Set) {
        let mut held = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        held.push_back(set);
        if held.len() > SETS_HELD
            && let Some(newer) = held.remove(1)
        {
            held[0].fold(newer);
        }
    }

    fn take(&self) -> VecDeque<Set> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl Set {
    fn nothing(&self) -> bool {
        self.whole.is_none() && self.provinces.is_empty() && self.keys.is_empty() && self.naval.is_empty() && self.plans.is_none()
    }

    fn fold(&mut self, newer: Self) {
        let Self { hour, from: _, composed, whole, provinces, keys, naval, plans, built_ns, digest } = newer;
        if whole.is_some() {
            (self.provinces, self.keys, self.naval) = (Vec::new(), Vec::new(), Vec::new());
            self.whole = whole;
        }
        newest_of(&mut self.provinces, provinces);
        newest_of(&mut self.keys, keys);
        newest_of(&mut self.naval, naval);
        (self.hour, self.composed, self.digest) = (hour, composed, digest);
        self.plans = plans.or(self.plans.take());
        self.built_ns += built_ns;
    }
}

fn newest_of<V>(held: &mut Vec<(u32, V)>, newer: Vec<(u32, V)>) {
    held.extend(newer);
    let mut seen = HashSet::with_capacity(held.len());
    held.reverse();
    held.retain(|(entity, _)| seen.insert(*entity));
    held.reverse();
}

struct Whole {
    regions: Arc<Regions>,
    keys_of: Vec<u32>,
    colours: Vec<Option<[u8; 3]>>,
    naval: BTreeMap<u32, Attributes>,
}

pub struct Installer {
    sets: VecDeque<Set>,
    taken: Arc<Handover>,
    keys_of: Vec<u32>,
    colours: Vec<Option<[u8; 3]>>,
    naval: BTreeMap<u32, Attributes>,
    at: Option<Hour>,
    dated: Option<Hour>,
    composed: u64,
    shown: Arc<Shown>,
    asks_whole: Arc<AtomicBool>,
    refused: bool,
    live: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    last_plans: Option<(ClassPlan, ZonePlan)>,
    plans_shown: Arc<Shown>,
    owed: bool,
    uploads: [u64; 3],
    wrote: Wrote,
}

pub struct Political {
    content: Arc<Content>,
    columns: Option<Columns>,
    not_recorded: Option<String>,
    count: usize,
    country_of: Vec<u32>,
    owner_of: Vec<u32>,
    state_of: Vec<u32>,
    references: Vec<u32>,
    colours: Vec<Option<[u8; 3]>>,
    colour_native: Vec<u32>,
    class: Vec<u8>,
    unresolved: BTreeSet<u32>,
    beyond: BTreeSet<u32>,
    errors: BTreeMap<u32, String>,
    colour_errors: BTreeMap<u32, (u32, String)>,
    missing_tags: BTreeMap<u32, String>,
    links: HashMap<u32, u32>,
    recorded: Vec<Option<Row>>,
    hour_rows: Vec<(Hour, Vec<(u32, Row)>)>,
    hour_sets: Vec<(Hour, Vec<(u32, u32)>)>,
    hour_plans: Vec<(Hour, (ClassPlan, ZonePlan))>,
    appearance_hours: Vec<(Hour, Vec<(u32, usize, Option<u32>)>)>,
    recoloured_at: Option<Hour>,
    recoloured_first: Option<Hour>,
    key_before: BTreeMap<u32, (Option<[u8; 3]>, u32)>,
    interior_keys: BTreeSet<u32>,
    zone_at: Option<Hour>,
    pending_zone: Option<(Hour, HashSet<u32>)>,
    raws: Vec<[u32; 2]>,
    by_link: BTreeSet<(u32, u32)>,
    natives: BTreeSet<(u32, u32)>,
    native_slot: HashMap<u32, u32>,
    relations: Relations,
    appearance_gap: Option<String>,
    demilitarized: HashSet<u32>,
    provinces: Dirty,
    keys: Dirty,
    edges: Dirty,
    relations_dirty: bool,
    zone_dirty: bool,
    statuses: HashMap<usize, Status>,
    hour: Hour,
    frame: u64,
    marked: [usize; 3],
    prepared_plans: bool,
    built_ns: u64,
    fill_changed: bool,
    edges_changed: bool,
    classes: Option<ClassPlan>,
    zone_plan: Option<ZonePlan>,
    zone_plan_stale: bool,
    pending_provinces: Pending,
    pending_keys: Pending,
    pending_chunks: Pending,
    pending_naval: Pending,
    sets: Option<Arc<Handover>>,
    shown: Arc<Shown>,
    asks_whole: Arc<AtomicBool>,
    live: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    plans_shown: Arc<Shown>,
    voided_from: Option<Hour>,
    undated_from: Option<Hour>,
    zone_first: Option<Hour>,
    links_from: Option<Hour>,
    appearance_reslotted: bool,
    changed: u64,
    dated: Option<Hour>,
    scoped: Scoped,
    shown_refused: bool,
    naval: Arc<BTreeMap<u32, Attributes>>,
    naval_changed: bool,
}

impl Political {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            content,
            columns: None,
            not_recorded: Some("not composed yet".into()),
            count: 0,
            country_of: Vec::new(),
            owner_of: Vec::new(),
            state_of: Vec::new(),
            references: Vec::new(),
            colours: Vec::new(),
            colour_native: Vec::new(),
            class: Vec::new(),
            unresolved: BTreeSet::new(),
            beyond: BTreeSet::new(),
            errors: BTreeMap::new(),
            colour_errors: BTreeMap::new(),
            missing_tags: BTreeMap::new(),
            links: HashMap::new(),
            recorded: Vec::new(),
            hour_rows: Vec::new(),
            hour_sets: Vec::new(),
            hour_plans: Vec::new(),
            appearance_hours: Vec::new(),
            recoloured_at: None,
            recoloured_first: None,
            key_before: BTreeMap::new(),
            interior_keys: BTreeSet::new(),
            zone_at: None,
            pending_zone: None,
            raws: Vec::new(),
            by_link: BTreeSet::new(),
            natives: BTreeSet::new(),
            native_slot: HashMap::new(),
            relations: Relations::default(),
            appearance_gap: None,
            demilitarized: HashSet::new(),
            provinces: Dirty::default(),
            keys: Dirty::default(),
            edges: Dirty::default(),
            relations_dirty: false,
            zone_dirty: false,
            statuses: HashMap::new(),
            hour: 0,
            frame: 0,
            marked: [0; 3],
            prepared_plans: false,
            built_ns: 0,
            fill_changed: true,
            edges_changed: true,
            classes: None,
            zone_plan: None,
            zone_plan_stale: false,
            pending_provinces: Pending::default(),
            pending_keys: Pending::default(),
            pending_chunks: Pending::default(),
            pending_naval: Pending::default(),
            sets: None,
            shown: Arc::default(),
            asks_whole: Arc::default(),
            live: Arc::new(AtomicBool::new(true)),
            generation: Arc::default(),
            plans_shown: Arc::default(),
            voided_from: None,
            undated_from: None,
            zone_first: None,
            links_from: None,
            appearance_reslotted: false,
            changed: stamp(),
            dated: None,
            scoped: Scoped::default(),
            shown_refused: false,
            naval: Arc::default(),
            naval_changed: false,
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new(Arc::clone(&self.content)) }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    pub fn drawn(&self) -> bool {
        self.columns.is_some()
    }

    pub fn refused(&self) -> Option<String> {
        if let Some(why) = &self.not_recorded {
            return Some(why.clone());
        }
        if let Some(error) = &self.relations.error {
            return Some(error.clone());
        }
        let province = self.errors.iter().next().map(|(&id, error)| (id, error));
        let colour = self.colour_errors.values().min_by_key(|(id, _)| *id).map(|(id, error)| (*id, error));
        match (province, colour) {
            (Some(a), Some(b)) => Some(if b.0 < a.0 { b.1 } else { a.1 }.clone()),
            (Some((_, error)), None) | (None, Some((_, error))) => Some(error.clone()),
            (None, None) => None,
        }
    }

    pub fn country_of(&self) -> &[u32] {
        &self.country_of
    }

    pub fn owner_of(&self) -> &[u32] {
        &self.owner_of
    }

    pub fn state_of(&self) -> &[u32] {
        &self.state_of
    }

    pub fn colours(&self) -> impl Iterator<Item = (u32, [u8; 3])> + '_ {
        self.colours.iter().enumerate().filter_map(|(key, colour)| colour.map(|colour| (key as u32, colour)))
    }

    pub fn classes(&self) -> &[u8] {
        &self.class
    }

    fn undated(&mut self, at: Hour) {
        self.undated_from = Some(self.undated_from.map_or(at, |held| held.min(at)));
    }

    fn stand_down(&mut self) {
        self.live.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn plans_shown(&self) -> Option<Hour> {
        self.plans_shown.get()
    }

    pub fn changed(&self) -> u64 {
        self.changed
    }

    pub fn demilitarized(&self) -> BTreeSet<u32> {
        self.demilitarized.iter().copied().collect()
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = self.relations.warnings.clone();
        if !self.unresolved.is_empty() {
            warnings.push(format!(
                "{} land provinces have a zero controller reference and take no political fill; province ids {}",
                self.unresolved.len(),
                examples(self.unresolved.iter().map(u32::to_string))
            ));
        }
        let tags: BTreeSet<&String> = self.missing_tags.values().collect();
        if !tags.is_empty() {
            let gap = self.appearance_gap.clone().unwrap_or_else(|| "CountryAppearance has no row for them".to_owned());
            warnings.push(format!("{} countries have no recorded colour this hour ({gap}) and are filled grey: {}", tags.len(), examples(tags.iter().map(|tag| (*tag).clone()))));
        }
        warnings
    }
}

impl Political {
    fn columns(&self) -> Columns {
        self.columns.expect("the political layer is declared before it composes")
    }

    fn core<'m>(&self, mirrors: &'m Mirrors) -> Result<&'m Mirror, String> {
        let mut provinces = None;
        for table in [TableId::Provinces, TableId::Countries, TableId::CountryLinks] {
            let mirror = mirrors.get(table).ok_or_else(|| format!("{table:?} is not read yet"))?;
            if let Some(why) = mirror.not_recorded() {
                return Err(format!("{why}; the core tables Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn"));
            }
            provinces.get_or_insert(mirror);
        }
        Ok(provinces.expect("Provinces is read"))
    }

    fn mark_all(&mut self, mirrors: &Mirrors) {
        self.country_of.clear();
        self.owner_of.clear();
        self.state_of.clear();
        self.references.clear();
        self.colours.clear();
        self.colour_native.clear();
        self.unresolved.clear();
        self.errors.clear();
        self.colour_errors.clear();
        self.missing_tags.clear();
        self.provinces.clear();
        self.keys.clear();
        self.edges.clear();
        self.demilitarized.clear();
        self.relations = Relations::default();
        self.appearance_gap = None;
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        self.class = vec![LEFT_OUT; self.content.edges.len()];
        self.fill_changed = true;
        self.edges_changed = true;
        self.changed = stamp();
        self.recorded.clear();
        self.void_hours();
        self.raws.clear();
        self.by_link.clear();
        let columns = self.columns();
        self.links.clear();
        if let Some(links) = mirrors.get(TableId::CountryLinks) {
            for slot in links.rows() {
                if let (Some(key), Some(canonical)) = (links.key(slot), links.number(columns.link_slot, slot).value()) {
                    self.links.insert(key as u32, canonical);
                }
            }
        }
        self.natives.clear();
        self.native_slot.clear();
        if let Some(countries) = mirrors.get(TableId::Countries) {
            for slot in countries.rows() {
                if let (Some(key), Some(canonical)) = (countries.key(slot), countries.number(columns.country_slot, slot).value()) {
                    self.natives.insert((canonical, key as u32));
                    self.native_slot.insert(key as u32, canonical);
                }
            }
        }
        self.beyond.clear();
        if let Some(provinces) = mirrors.get(TableId::Provinces) {
            let max = self.content.max_defined;
            self.beyond.extend(provinces.rows().filter_map(|slot| provinces.key(slot)).map(|key| key as u32).filter(|&id| id > max));
        }
        self.count = 0;
        self.resize();
        for edge in 0..self.content.edges.len() as u32 {
            self.edges.mark(edge, self.frame);
        }
        self.relations_dirty = true;
        self.zone_dirty = true;
    }

    fn resize(&mut self) {
        let count = self.content.max_defined.max(self.beyond.last().copied().unwrap_or(0)) as usize + 1;
        if count < self.count {
            for id in count..self.count {
                let key = self.country_of[id];
                self.unreference(key);
                self.errors.remove(&(id as u32));
                self.unresolved.remove(&(id as u32));
            }
        }
        self.country_of.resize(count, 0);
        self.owner_of.resize(count, 0);
        self.state_of.resize(count, 0);
        self.recorded.resize(count, None);
        for id in count..self.raws.len() {
            for raw in self.raws[id] {
                self.by_link.remove(&(raw, id as u32));
            }
        }
        self.raws.resize(count, [0; 2]);
        for id in self.count..count {
            self.provinces.mark(id as u32, self.frame);
        }
        self.count = count;
    }

    fn unreference(&mut self, key: u32) {
        if key != 0 {
            self.references[key as usize] -= 1;
            if self.references[key as usize] == 0 {
                self.mark_key(key);
            }
        }
    }

    fn mark_key(&mut self, key: u32) {
        if self.keys.mark(key, self.frame) {
            self.marked[1] += 1;
        }
    }

    fn mark_province(&mut self, id: u32) {
        if self.provinces.mark(id, self.frame) {
            self.marked[0] += 1;
        }
    }

    fn mark_edges_of(&mut self, id: u32) {
        let content = Arc::clone(&self.content);
        for &edge in content.edges_of(id) {
            if self.edges.mark(edge, self.frame) {
                self.marked[2] += 1;
            }
        }
    }

    fn mark_referenced(&mut self) {
        for key in 1..self.references.len() as u32 {
            if self.references[key as usize] > 0 || self.colours[key as usize].is_some() {
                self.mark_key(key);
            }
        }
    }

    fn by_slot(&self, slot: u32) -> Option<u32> {
        self.natives.range((slot, 0)..=(slot, u32::MAX)).next_back().map(|&(_, native)| native)
    }

    fn paint(&mut self, id: u32, mirrors: &Mirrors) {
        let columns = self.columns();
        let provinces = mirrors.get(TableId::Provinces).expect("the core is read");
        let row = provinces.slot(u128::from(id)).map(|slot| Row {
            state: provinces.number(columns.province_state, slot).value(),
            owner: provinces.number(columns.province_owner, slot).value(),
            controller: provinces.number(columns.province_controller, slot).value(),
        });
        self.paint_from(id, row);
    }

    fn paint_from(&mut self, id: u32, row: Option<Row>) {
        self.scoped.province(id);
        self.errors.remove(&id);
        self.unresolved.remove(&id);
        if let Some(held) = self.recorded.get_mut(id as usize) {
            *held = row;
        }
        let kind = self.content.kind(id);
        let mut unresolved = false;
        let links = row.map(|row| [row.owner.unwrap_or(0), row.controller.unwrap_or(0)]).unwrap_or([0; 2]);
        let held = self.raws[id as usize];
        if held != links {
            for raw in held.into_iter().filter(|&raw| raw != 0) {
                self.by_link.remove(&(raw, id));
            }
            for raw in links.into_iter().filter(|&raw| raw != 0) {
                self.by_link.insert((raw, id));
            }
            self.raws[id as usize] = links;
        }
        let painted: Result<[u32; 3], String> = match row {
            None if id != 0 && kind != 0 => Err(format!("hour {} has no recorded row for map province {id}", self.hour)),
            None => Ok([0; 3]),
            Some(Row { state: Some(state), owner: Some(owner), controller: Some(controller) }) => {
                (|| {
                    let mut paint = [0; 3];
                    if kind == 0 {
                        return Err(format!("recorded province {id} is absent from map/definition.csv"));
                    }
                    if kind == SEA || (kind == LAKE && controller == 0) {
                        return Ok(paint);
                    }
                    if owner != 0 {
                        let slot = *self.links.get(&owner).ok_or_else(|| missing_link(owner, "owner"))?;
                        paint[1] = slot.checked_add(1).ok_or("owner country slot exceeds the border key range")?;
                    }
                    paint[2] = state;
                    if controller == 0 {
                        unresolved = true;
                    } else {
                        let slot = *self.links.get(&controller).ok_or_else(|| missing_link(controller, "controller"))?;
                        if self.by_slot(slot).is_none() {
                            return Err(format!("political fill country link {controller} resolves to missing country slot {slot}"));
                        }
                        paint[0] = slot.checked_add(1).ok_or("country slot exceeds the gradient border region key range")?;
                    }
                    Ok(paint)
                })()
            }
            Some(_) => Err(format!("hour {} does not record province {id}'s state, owner and controller", self.hour)),
        };
        let paint = painted.unwrap_or_else(|error| {
            self.errors.insert(id, error);
            unresolved = false;
            [0; 3]
        });
        if unresolved {
            self.unresolved.insert(id);
        }
        let at = id as usize;
        let old = [self.country_of[at], self.owner_of[at], self.state_of[at]];
        if old == paint {
            return;
        }
        (self.country_of[at], self.owner_of[at], self.state_of[at]) = (paint[0], paint[1], paint[2]);
        self.pending_provinces.mark(id);
        self.changed = stamp();
        if old[0] != paint[0] {
            self.unreference(old[0]);
            if paint[0] != 0 {
                let key = paint[0] as usize;
                if self.references.len() <= key {
                    self.references.resize(key + 1, 0);
                    self.colours.resize(key + 1, None);
                    self.colour_native.resize(key + 1, NONE);
                }
                self.references[key] += 1;
                if self.references[key] == 1 {
                    self.mark_key(paint[0]);
                }
            }
        }
        if old[1] != paint[1] || old[2] != paint[2] {
            self.mark_edges_of(id);
        }
    }

    fn colour(&mut self, key: u32, mirrors: &Mirrors) {
        let before = self.colours.get(key as usize).copied().flatten();
        let native = self.colour_native.get(key as usize).copied().unwrap_or(NONE);
        self.key_before.entry(key).or_insert((before, native));
        self.recolour(key, mirrors);
        if self.colours.get(key as usize).copied().flatten() != before {
            self.pending_keys.mark(key);
            self.changed = stamp();
            self.scoped.unplaced();
        }
    }

    fn recolour(&mut self, key: u32, mirrors: &Mirrors) {
        let at = key as usize;
        if at >= self.references.len() {
            return;
        }
        let slot = key - 1;
        self.colour_errors.remove(&slot);
        self.missing_tags.remove(&slot);
        self.colours[at] = None;
        self.colour_native[at] = NONE;
        if self.references[at] == 0 && !self.interior_keys.contains(&key) {
            return;
        }
        let first = || self.country_of.iter().position(|&held| held == key).map_or(u32::MAX, |id| id as u32);
        let source = match self.relations.source(slot) {
            Ok(source) => source,
            Err(error) => {
                let first = first();
                self.colour_errors.insert(slot, (first, error));
                return;
            }
        };
        let Some(source) = source else {
            self.colours[at] = Some(UNAVAILABLE_COLOR);
            return;
        };
        let Some(native) = self.by_slot(source) else {
            let first = first();
            self.colour_errors.insert(slot, (first, format!("political fill country slot {source} has no Countries row")));
            return;
        };
        self.colour_native[at] = native;
        let columns = self.columns();
        let recorded = rows(mirrors.get(TableId::CountryAppearance)).and_then(|appearance| {
            let row = appearance.slot(u128::from(native))?;
            let channel = |column| appearance.number(column, row).value().map(|bits: u32| (f32::from_bits(bits) * 255.0) as u8);
            Some([channel(columns.red)?, channel(columns.green)?, channel(columns.blue)?])
        });
        self.colours[at] = Some(match recorded {
            Some(colour) => colour,
            None => {
                let countries = mirrors.get(TableId::Countries).expect("the core is read");
                let tag = self
                    .by_slot(slot)
                    .and_then(|own| countries.slot(u128::from(own)))
                    .map(|row| match countries.text(columns.country_tag, row) {
                        Cell::Value(id) => countries.string(id).map_or_else(|| format!("<string {id} not named>"), |bytes| String::from_utf8_lossy(bytes).into_owned()),
                        _ => "<tag not recorded>".to_owned(),
                    })
                    .unwrap_or_default();
                self.missing_tags.insert(slot, tag);
                UNAVAILABLE_COLOR
            }
        });
    }

    fn classify(&mut self, edge: u32) {
        let content = &self.content;
        let (a, b) = content.edges[edge as usize];
        let bucket = content.bucket_start.partition_point(|&start| start <= edge as usize) - 1;
        let state = |province: u32| self.state_of.get(province as usize).copied().unwrap_or(0);
        let owner = |province: u32| self.owner_of.get(province as usize).copied().unwrap_or(0);
        let land = |province: u32| content.kind(province) == LAND;
        let zoned = |province: u32| self.demilitarized.contains(&state(province));
        let split = !self.demilitarized.is_empty() && land(a) && land(b) && zoned(a) != zoned(b);
        let class = if split {
            LEFT_OUT
        } else if bucket == 0 {
            if owner(a) != owner(b) {
                BorderClass::Country.bucket() as u8
            } else if state(a) != state(b) {
                BorderClass::State.bucket() as u8
            } else {
                BorderClass::Province.bucket() as u8
            }
        } else {
            BUCKETS[bucket].bucket() as u8
        };
        if self.class[edge as usize] != class {
            self.scoped.province(a);
            let zone = self.class[edge as usize] == LEFT_OUT || class == LEFT_OUT;
            self.class[edge as usize] = class;
            match self.content.chunk_of.get(edge as usize) {
                Some(&chunk) => {
                    self.pending_chunks.mark(chunk);
                    self.zone_plan_stale |= zone;
                }
                None => self.edges_changed = true,
            }
        }
    }

    fn zone_plan_of_its_borders(&self, meshes: &BorderMeshes) -> ZonePlan {
        let content = &self.content;
        let state = |province: u32| self.state_of.get(province as usize).copied().unwrap_or(0);
        let owner = |province: u32| self.owner_of.get(province as usize).copied().unwrap_or(0);
        let land = |province: u32| content.kind(province) == LAND;
        let zoned = |province: u32| self.demilitarized.contains(&state(province));
        let draws: Vec<(BorderClass, u32)> = (0..self.state_of.len() as u32).filter(|&province| zoned(province)).flat_map(|province| content.draws_of(province).iter().copied()).collect();
        let class = |a: u32, b: u32| {
            if owner(a) != owner(b) {
                BorderClass::Country
            } else if state(a) != state(b) {
                BorderClass::State
            } else {
                BorderClass::Province
            }
        };
        ZonePlan::of_draws(meshes, draws, |a, b| land(a) && land(b) && zoned(a) != zoned(b), class)
    }

    fn rezone(&mut self, zone: HashSet<u32>) {
        let moved: HashSet<u32> = zone.symmetric_difference(&self.demilitarized).copied().collect();
        self.demilitarized = zone;
        self.zone_plan_stale = true;
        self.changed = stamp();
        let content = Arc::clone(&self.content);
        for (province, state) in self.state_of.iter().enumerate() {
            if moved.contains(state) {
                for &edge in content.edges_of(province as u32) {
                    self.edges.mark(edge, self.frame);
                }
            }
        }
    }

    fn read_zone(&self, mirrors: &Mirrors) -> HashSet<u32> {
        let columns = self.columns();
        let (Some(flags), Some(states)) = (mirrors.get(TableId::StateFlags), mirrors.get(TableId::States)) else { return HashSet::new() };
        if !flags.complete() || !states.complete() {
            return HashSet::new();
        }
        flags
            .rows()
            .filter(|&slot| flags.number(columns.demilitarized, slot).value().is_some_and(|flag| flag != 0))
            .filter_map(|slot| flags.key(slot))
            .filter(|&state| states.slot(state).is_some())
            .map(|state| state as u32)
            .collect()
    }

    fn read_relations(&self, mirrors: &Mirrors) -> Relations {
        let columns = self.columns();
        let use_overlord = &self.content.map.country_colors.use_overlord_color;
        let autonomy_mirror = mirrors.get(TableId::Autonomy);
        let subject_mirror = mirrors.get(TableId::Subjects);
        let subject_gap = gap(subject_mirror, "Subjects");
        let mut relations = Relations { autonomy_gap: gap(autonomy_mirror, "Autonomy"), ..Relations::default() };
        let mut autonomy_errors: BTreeMap<u32, String> = BTreeMap::new();
        let mut by_status: BTreeMap<(u32, u32), Vec<u8>> = BTreeMap::new();
        if let Some(autonomy) = rows(autonomy_mirror) {
            for row in autonomy.rows() {
                let Some(raw) = autonomy.key(row).map(|key| key as u32) else { continue };
                let state = match autonomy.text(columns.autonomy_state, row) {
                    Cell::Value(id) => autonomy.string(id).map_or_else(|| format!("<string {id} not named>").into_bytes(), <[u8]>::to_vec),
                    _ => continue,
                };
                match self.links.get(&raw) {
                    Some(&slot) => {
                        by_status.insert((slot, raw), state);
                    }
                    None => {
                        autonomy_errors.insert(raw, missing_link(raw, "autonomy colour"));
                    }
                }
            }
        }
        let mut status_errors: BTreeMap<u32, u32> = BTreeMap::new();
        let mut conflicted = BTreeSet::new();
        for (&(slot, raw), state) in &by_status {
            match relations.states.get(&slot) {
                None => {
                    relations.states.insert(slot, state.clone());
                }
                Some(first) if first != state && conflicted.insert(slot) => {
                    status_errors.insert(raw, slot);
                }
                Some(_) => {}
            }
        }
        let mut subject_errors: BTreeMap<u128, String> = BTreeMap::new();
        let mut overlords: BTreeMap<u32, (u32, bool)> = BTreeMap::new();
        if let Some(subjects) = rows(subject_mirror) {
            let mut active: BTreeMap<u128, (u32, u32)> = BTreeMap::new();
            for row in subjects.rows() {
                let Some(key) = subjects.key(row) else { continue };
                let (Some(start), Some(end), Some(cancel)) =
                    (subjects.number(columns.subject_start, row).value(), subjects.number(columns.subject_end, row).value(), subjects.number(columns.subject_cancel, row).value())
                else {
                    continue;
                };
                if cancel == 0 && end > start && end > self.hour {
                    relations.next_end = Some(relations.next_end.map_or(end, |held| held.min(end)));
                }
                if cancel == 0 && (end <= start || self.hour < end) {
                    active.insert(key, (key as u32, (key >> 32) as u32));
                }
            }
            for (key, (subject, overlord)) in active {
                match (self.links.get(&subject), self.links.get(&overlord)) {
                    (Some(&slot), Some(&over)) => {
                        let held = overlords.entry(slot).or_insert((over, false));
                        if held.0 != over {
                            held.1 = true;
                        }
                        held.0 = over;
                    }
                    (None, _) => {
                        subject_errors.insert(key, missing_link(subject, "subject colour"));
                    }
                    (_, None) => {
                        subject_errors.insert(key, missing_link(overlord, "subject colour"));
                    }
                }
            }
        }
        let link_error = autonomy_errors.iter().next();
        let status_error = status_errors.iter().next();
        relations.error = match (link_error, status_error) {
            (Some((&raw, error)), status) if status.is_none_or(|(&key, _)| raw < key) => Some(error.clone()),
            (_, Some((_, slot))) => Some(format!("country slot {slot} has conflicting current autonomy definitions")),
            _ => subject_errors.values().next().cloned(),
        };
        let mut conflicts = BTreeSet::new();
        for (&slot, state) in relations.states.iter().filter(|(_, state)| !state.is_empty()) {
            match use_overlord.get(state.as_slice()) {
                None => {
                    relations.undecided.insert(
                        slot,
                        format!("its current autonomy definition {} is not an autonomy state the content defines, so its use_overlord_color flag is unknown", String::from_utf8_lossy(state)),
                    );
                }
                Some(false) => {}
                Some(true) => match overlords.get(&slot) {
                    Some(&(_, true)) => {
                        conflicts.insert(slot);
                    }
                    Some(&(overlord, false)) => {
                        relations.inherited.insert(slot, overlord);
                    }
                    None => {
                        let why = match &subject_gap {
                            Some(gap) => format!("its current autonomy inherits, and {gap}"),
                            None => "its current autonomy inherits, but no active Subjects row names its overlord".to_owned(),
                        };
                        relations.undecided.insert(slot, why);
                    }
                },
            }
        }
        if relations.error.is_none()
            && let Some(slot) = conflicts.first()
        {
            relations.error = Some(format!("country slot {slot} has conflicting colour overlords"));
        }
        let gap_warning = relations.autonomy_gap.as_ref().map(|gap| format!("{gap}, so a country without a recorded current autonomy has an unknown colour source; its colour is withheld"));
        relations.warnings.extend(gap_warning);
        let undecided: Vec<String> = relations.undecided.iter().map(|(slot, why)| format!("Country slot {slot}'s colour source is unknown ({why}); its colour is withheld")).collect();
        relations.warnings.extend(undecided);
        relations
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        let provinces = match self.core(mirrors) {
            Ok(provinces) => provinces,
            Err(why) => {
                if self.not_recorded.as_ref() != Some(&why) {
                    self.mark_all(mirrors);
                    self.provinces.clear();
                    self.edges.clear();
                    self.keys.clear();
                    self.relations_dirty = false;
                    self.zone_dirty = false;
                    self.not_recorded = Some(why);
                }
                return;
            }
        };
        if self.not_recorded.take().is_some() {
            self.mark_all(mirrors);
        }
        self.hour = provinces.head().unwrap_or(self.hour);
        if self.fill_changed {
            self.void_hours();
        }
        if let (Some(first), Some(last)) = (self.recoloured_first.take(), self.recoloured_at)
            && first < last
        {
            self.undated(first);
        }
        if self.relations_dirty {
            self.relations_dirty = false;
            let relations = self.read_relations(mirrors);
            let recolour = !relations.colours_same(&self.relations);
            self.relations = relations;
            if recolour {
                self.mark_referenced();
            }
        }
        let appearance_gap = gap(mirrors.get(TableId::CountryAppearance), "CountryAppearance");
        if appearance_gap != self.appearance_gap {
            self.appearance_gap = appearance_gap;
            self.mark_referenced();
        }
        if self.zone_dirty {
            self.zone_dirty = false;
            let zone = self.read_zone(mirrors);
            if let (Some(first), Some(last)) = (self.zone_first, self.zone_at)
                && first < last
            {
                self.undated(first);
            }
            if zone != self.demilitarized {
                self.pending_zone = Some((self.zone_at.take().unwrap_or(self.hour), zone));
            }
        }
        self.paint_hours();
        let mut dirty = std::mem::take(&mut self.provinces);
        let _ = dirty.settle(|id| {
            if (id as usize) < self.count {
                self.paint(id, mirrors);
            }
            Ok(())
        });
        self.provinces = dirty;
        let mut dirty = std::mem::take(&mut self.keys);
        let _ = dirty.settle(|key| {
            self.colour(key, mirrors);
            Ok(())
        });
        self.keys = dirty;
        let mut dirty = std::mem::take(&mut self.edges);
        let _ = dirty.settle(|edge| {
            self.classify(edge);
            Ok(())
        });
        self.edges = dirty;
    }

    pub fn marked(&self) -> [usize; 3] {
        self.marked
    }

    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        let mut differing = 0;
        let mut first = None;
        let mut note = |what: String| {
            differing += 1;
            first.get_or_insert(what);
        };
        if self.refused() != other.refused() {
            note(format!("refused: {:?} against {:?}", self.refused(), other.refused()));
        }
        for (name, a, b) in [("country_of", &self.country_of, &other.country_of), ("owner_of", &self.owner_of, &other.owner_of), ("state_of", &self.state_of, &other.state_of)] {
            if a.len() != b.len() {
                note(format!("{name}: {} provinces against {}", a.len(), b.len()));
            }
            for (id, (x, y)) in a.iter().zip(b).enumerate() {
                if x != y {
                    note(format!("{name}[{id}]: {x} against {y}"));
                }
            }
        }
        let colours: BTreeMap<u32, [u8; 3]> = self.colours().collect();
        let theirs: BTreeMap<u32, [u8; 3]> = other.colours().collect();
        for key in colours.keys().chain(theirs.keys()).collect::<BTreeSet<_>>() {
            if colours.get(key) != theirs.get(key) {
                note(format!("colour of key {key}: {:?} against {:?}", colours.get(key), theirs.get(key)));
            }
        }
        for (edge, (x, y)) in self.class.iter().zip(&other.class).enumerate() {
            if x != y {
                note(format!("edge {edge} {:?}: class {x} against {y}", self.content.edges[edge]));
            }
        }
        if self.warnings() != other.warnings() {
            note(format!("warnings: {:?} against {:?}", self.warnings(), other.warnings()));
        }
        (differing, first)
    }

    pub fn content_bytes(&self) -> usize {
        self.content.bytes()
    }
}

impl Layer for Political {
    fn set_naval(&mut self, attributes: &BTreeMap<u32, Attributes>) {
        Self::set_naval(self, attributes);
    }

    fn name(&self) -> &'static str {
        "political"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if let MapMode::Scripted(_) = view.map_mode {
            self.stand_down();
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("The political layer is refused: scripted map mode {} keeps its borders from one refresh to the next, which no layer follows yet", view.map_mode.token()),
            ));
        }
        if view.map_mode != MapMode::default() {
            let (sets, shown, asks_whole, live, generation, plans_shown) = (self.sets.take(), Arc::clone(&self.shown), Arc::clone(&self.asks_whole), Arc::clone(&self.live), Arc::clone(&self.generation), Arc::clone(&self.plans_shown));
            *self = Self::new(Arc::clone(&self.content));
            (self.sets, self.shown, self.asks_whole, self.live, self.generation, self.plans_shown) = (sets, shown, asks_whole, live, generation, plans_shown);
            self.stand_down();
            self.fill_changed = true;
            return Ok(());
        }
        self.live.store(true, Ordering::Release);
        for table in [TableId::Provinces, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::Subjects, TableId::Autonomy, TableId::StateFlags, TableId::States] {
            inputs.table(table, true)?;
        }
        inputs.hourly(TableId::Provinces)?;
        for table in [TableId::CountryAppearance, TableId::Subjects, TableId::Autonomy, TableId::CountryLinks, TableId::StateFlags] {
            inputs.hourly(table)?;
        }
        self.columns = Some(Columns {
            province_state: inputs.number(TableId::Provinces, "state")?,
            province_owner: inputs.number(TableId::Provinces, "owner")?,
            province_controller: inputs.number(TableId::Provinces, "controller")?,
            country_slot: inputs.number(TableId::Countries, "canonical_slot")?,
            country_tag: inputs.text(TableId::Countries, "tag")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            red: inputs.number(TableId::CountryAppearance, "red")?,
            green: inputs.number(TableId::CountryAppearance, "green")?,
            blue: inputs.number(TableId::CountryAppearance, "blue")?,
            subject_start: inputs.number(TableId::Subjects, "start_hour")?,
            subject_end: inputs.number(TableId::Subjects, "end_hour")?,
            subject_cancel: inputs.number(TableId::Subjects, "cancel")?,
            autonomy_state: inputs.text(TableId::Autonomy, "state")?,
            demilitarized: inputs.number(TableId::StateFlags, "demilitarized")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.columns.is_none() {
            return Ok(());
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        let marked = self.marked;
        self.not_recorded = Some("composing".into());
        self.settle_dirty(mirrors);
        self.marked = marked;
        (self.voided_from, self.undated_from, self.zone_first) = (None, None, None);
        (self.links_from, self.appearance_reslotted) = (None, false);
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() {
            return;
        }
        let columns = self.columns();
        if let Some(end) = self.relations.next_end.filter(|&end| step.last() >= end) {
            self.relations_dirty = true;
            let at = end.max(step.first());
            self.recoloured_at = self.recoloured_at.max(Some(at));
            self.recoloured_first = Some(self.recoloured_first.map_or(at, |held| held.min(at)));
        }
        let colour_columns = [columns.red.column, columns.green.column, columns.blue.column];
        for changes in step.tables() {
            let restated = changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status));
            let left = changes.status_left().or_else(|| restated.then(|| step.first()));
            if let Some(hour) = left.filter(|_| matches!(TableId::ALL[changes.table()], TableId::Provinces | TableId::Countries | TableId::CountryLinks)) {
                self.undated(hour);
            }
            let touched = |changes: &Changes, wanted: &[usize]| {
                restated || !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty() && wanted.contains(&written.column()))
            };
            let table = TableId::ALL[changes.table()];
            let Some(mirror) = mirrors.get(table) else { continue };
            self.date_colours(step, table, changes, mirror, columns);
            match table {
                TableId::Provinces => {
                    self.apply_provinces(changes, mirror, columns);
                    self.read_hours(step, changes, mirror, columns);
                }
                TableId::Countries if touched(changes, &[columns.country_slot.column, columns.country_tag.column]) => self.apply_countries(changes, mirror, columns),
                TableId::CountryLinks if touched(changes, &[columns.link_slot.column]) => self.apply_links(changes, mirror, columns),
                TableId::CountryAppearance if touched(changes, &colour_columns) => {
                    if restated {
                        self.mark_referenced();
                        continue;
                    }
                    let mut natives: HashSet<u32> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u32).collect();
                    for written in changes.columns().iter().filter(|written| colour_columns.contains(&written.column())) {
                        natives.extend(written.slots().iter().filter_map(|&slot| mirror.key(slot)).map(|key| key as u32));
                    }
                    for key in 1..self.colour_native.len() as u32 {
                        if natives.contains(&self.colour_native[key as usize]) || (self.missing_tags.contains_key(&(key - 1)) && !natives.is_empty()) {
                            self.mark_key(key);
                        }
                    }
                }
                TableId::Subjects if touched(changes, &[columns.subject_start.column, columns.subject_end.column, columns.subject_cancel.column]) => self.relations_dirty = true,
                TableId::Autonomy if touched(changes, &[columns.autonomy_state.column]) => self.relations_dirty = true,
                TableId::StateFlags if touched(changes, &[columns.demilitarized.column]) => self.zone_dirty = true,
                TableId::States if touched(changes, &[]) => self.zone_dirty = true,
                _ => {}
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        if self.columns.is_some() {
            self.settle_dirty(mirrors);
            self.clear_inner_diagnostics();
            self.prepare();
        }
        Ok(())
    }

    fn timed(&self) -> bool {
        true
    }

    fn timed_at(&mut self, hour: Hour, _composed: u64) -> io::Result<()> {
        let composed = self.generation.load(Ordering::Acquire);
        if self.sets.is_none() || self.columns.is_none() {
            self.hour_sets.clear();
            self.hour_plans.clear();
            self.void_colour_hours();
            (self.undated_from, self.voided_from) = (None, None);
            return Ok(());
        }
        let digest = viewer_runtime::hour_check::on().then(|| digest(&self.country_of, &self.colours));
        let plans = (std::mem::take(&mut self.prepared_plans) || self.fill_changed || self.asks_whole.load(Ordering::Acquire))
            .then(|| self.classes.clone().zip(self.zone_plan.clone()))
            .flatten();
        let whole = (self.fill_changed || self.asks_whole.swap(false, Ordering::AcqRel)).then(|| {
            self.fill_changed = false;
            self.pending_provinces.clear();
            self.pending_keys.clear();
            self.pending_naval.clear();
            Whole { regions: Arc::new(self.regions()), keys_of: self.country_of.clone(), colours: self.colours.clone(), naval: (*self.naval).clone() }
        });
        let (mut keys, mut provinces, naval): (Vec<(u32, Option<[u8; 3]>)>, Vec<(u32, u32)>, Vec<(u32, Attributes)>) = match whole.is_some() {
            true => (Vec::new(), Vec::new(), Vec::new()),
            false => (
                self.pending_keys.take().into_iter().map(|key| (key, self.colours.get(key as usize).copied().flatten())).collect(),
                self.pending_provinces.take().into_iter().map(|id| (id, self.country_of.get(id as usize).copied().unwrap_or(0))).collect(),
                self.pending_naval.take().into_iter().map(|key| (key, self.naval.get(&key).copied().unwrap_or_default())).collect(),
            ),
        };
        self.naval_changed = false;
        let hours = std::mem::take(&mut self.hour_sets);
        let hour_plans = std::mem::take(&mut self.hour_plans);
        let (before, appearance, recoloured_at) = (std::mem::take(&mut self.key_before), std::mem::take(&mut self.appearance_hours), self.recoloured_at.take());
        self.interior_keys.clear();
        let reslotted = std::mem::take(&mut self.appearance_reslotted);
        (self.zone_at, self.zone_first, self.links_from) = (None, None, None);
        if reslotted && let Some(first) = appearance.first().map(|(at, _)| *at) {
            self.undated(first);
        }
        if whole.is_some()
            && let Some(oldest) = hours.iter().map(|(at, _)| *at).chain(appearance.iter().map(|(at, _)| *at)).chain(hour_plans.iter().map(|(at, _)| *at)).chain(recoloured_at).chain(self.undated_from.take()).min()
        {
            self.voided_from = Some(self.voided_from.map_or(oldest, |held| held.min(oldest)));
        }
        if whole.is_none()
            && let Some(sets) = self.sets.clone()
        {
            let mut hours = hours;
            for &(at, _) in &hour_plans {
                if !hours.iter().any(|&(held, _)| held == at) {
                    hours.push((at, Vec::new()));
                }
            }
            hours.sort_unstable_by_key(|&(at, _)| at);
            let mut plans_at: BTreeMap<Hour, (ClassPlan, ZonePlan)> = hour_plans.into_iter().collect();
            let (dated, colours_undated) = self.dated_hours(hours, &before, &appearance, recoloured_at, hour);
            if let Some(at) = colours_undated {
                self.undated(at);
            }
            let undated = self.undated_from;
            for (at, painted, set_keys) in dated {
                if undated.is_some_and(|undated| at >= undated) {
                    provinces.extend(painted.into_iter().map(|(id, _)| (id, self.country_of.get(id as usize).copied().unwrap_or(0))));
                    keys.extend(set_keys.into_iter().map(|(key, _)| (key, self.colours.get(key as usize).copied().flatten())));
                    continue;
                }
                let plans = plans_at.remove(&at);
                sets.push(Set { hour: at, from: at, composed, whole: None, provinces: painted, keys: set_keys, naval: Vec::new(), plans, built_ns: 0, digest: None });
                self.dated = Some(at);
            }
            provinces.sort_unstable_by_key(|&(id, _)| id);
            provinces.dedup_by_key(|&mut (id, _)| id);
            keys.sort_unstable_by_key(|&(key, _)| key);
            keys.dedup_by_key(|&mut (key, _)| key);
        }
        let from = match whole.is_some() {
            true => self.voided_from.take().map_or(hour, |voided| voided.min(hour)),
            false => self.undated_from.take().map_or(hour, |undated| undated.min(hour)),
        };
        let set = Set { hour, from, composed, whole, provinces, keys, naval, plans, built_ns: std::mem::take(&mut self.built_ns), digest };
        if self.dated == Some(hour) && set.nothing() {
            return Ok(());
        }
        self.dated = Some(hour);
        if let Some(sets) = self.sets.as_ref() {
            sets.push(set);
        }
        Ok(())
    }

    fn installs(&mut self) -> Option<Box<dyn Installs>> {
        if self.sets.is_some() {
            return None;
        }
        let taken = Arc::new(Handover::default());
        self.sets = Some(Arc::clone(&taken));
        self.fill_changed = true;
        let mut installer = Installer::new(taken, Arc::clone(&self.shown), Arc::clone(&self.asks_whole), Arc::clone(&self.live), Arc::clone(&self.generation));
        installer.plans_shown = Arc::clone(&self.plans_shown);
        Some(Box::new(installer))
    }

    fn shown_through(&self) -> Option<Hour> {
        self.shown.get().filter(|_| self.columns.is_some())
    }

    fn uploaded(&self) -> bool {
        self.columns.is_none() || !(self.edges_changed || self.zone_plan_stale || self.prepared_plans)
    }

    fn carried(&self) -> Carried {
        let entities = self.provinces.len() + self.keys.len() + self.edges.len() + usize::from(self.relations_dirty) + usize::from(self.zone_dirty);
        Carried { entities }
    }

    fn scope(&mut self, sight: &Sight<'_>) -> Option<Scope> {
        Some(self.scoped.take(sight, None))
    }

    fn held(&self) -> usize {
        (self.country_of.capacity() + self.owner_of.capacity() + self.state_of.capacity() + self.references.capacity() + self.colour_native.capacity()) * 4
            + self.colours.capacity() * 4
            + self.class.capacity()
            + self.provinces.bytes()
            + self.pending_provinces.bytes()
            + self.pending_keys.bytes()
            + self.pending_chunks.bytes()
            + self.keys.bytes()
            + self.edges.bytes()
            + self.recorded.capacity() * size_of::<Option<Row>>()
            + self.hour_rows.iter().map(|(_, rows)| rows.capacity() * size_of::<(u32, Row)>()).sum::<usize>()
            + self.hour_sets.iter().map(|(_, painted)| painted.capacity() * 8).sum::<usize>()
            + self.appearance_hours.iter().map(|(_, written)| written.capacity() * 12).sum::<usize>()
            + self.key_before.len() * 16
            + (self.links.capacity() + self.native_slot.capacity()) * 12
            + self.natives.len() * 16
            + (self.unresolved.len() + self.beyond.len()) * 8
            + self.errors.values().chain(self.missing_tags.values()).map(|text| text.len() + 32).sum::<usize>()
            + self.demilitarized.capacity() * 8
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Installer {
    fn new(taken: Arc<Handover>, shown: Arc<Shown>, asks_whole: Arc<AtomicBool>, live: Arc<AtomicBool>, generation: Arc<AtomicU64>) -> Self {
        Self {
            sets: VecDeque::new(),
            taken,
            keys_of: Vec::new(),
            colours: Vec::new(),
            naval: BTreeMap::new(),
            at: None,
            dated: None,
            composed: 0,
            shown,
            asks_whole,
            refused: false,
            live,
            generation,
            last_plans: None,
            plans_shown: Arc::default(),
            owed: false,
            uploads: [0; 3],
            wrote: Wrote::default(),
        }
    }

    fn take(&mut self) -> u64 {
        let mut built_ns = 0;
        let taken = self.taken.take();
        let took = !taken.is_empty();
        for set in taken {
            built_ns += set.built_ns;
            self.sets.push_back(set);
        }
        let composed = self.generation.load(Ordering::Acquire);
        let recomposed = std::mem::replace(&mut self.composed, composed) != composed;
        if recomposed {
            self.dated = None;
        }
        if took || recomposed {
            self.sets.retain(|set| set.composed == composed);
            while self.sets.len() > SETS_HELD
                && let Some(newer) = self.sets.remove(1)
            {
                self.sets[0].fold(newer);
            }
        }
        self.dated = self.sets.back().map(|set| set.hour).or(self.dated);
        built_ns
    }

    fn due(&self, through: Option<Hour>) -> Option<Hour> {
        let front = self.sets.front()?;
        through.is_none_or(|head| front.hour <= head).then_some(front.hour)
    }

    fn pick(&mut self, through: Option<Hour>) -> Picked {
        let mut picked = Picked::default();
        picked.built_ns = self.take();
        let reached = |set: &Set| through.is_none_or(|head| set.hour <= head);
        while let Some(front) = self.sets.front().filter(|set| reached(set)) {
            if front.whole.is_none() && self.refused {
                if !self.sets.iter().take_while(|set| reached(set)).any(|set| set.whole.is_some()) {
                    break;
                }
                self.sets.pop_front();
                continue;
            }
            let set = self.sets.pop_front().expect("the front is there");
            picked.hour = Some(set.hour);
            picked.digest_of = set.digest.map(|digest| (set.hour, digest));
            if let Some(whole) = set.whole {
                (self.keys_of, self.colours, self.naval) = (whole.keys_of.clone(), whole.colours.clone(), whole.naval.clone());
                self.refused = false;
                (picked.provinces, picked.keys, picked.naval, picked.plans) = (Vec::new(), Vec::new(), Vec::new(), None);
                picked.whole = Some(whole);
            }
            for (province, key) in set.provinces {
                if self.keys_of.len() <= province as usize {
                    self.keys_of.resize(province as usize + 1, 0);
                }
                self.keys_of[province as usize] = key;
                picked.provinces.push(province as usize);
            }
            for (key, colour) in set.keys {
                if self.colours.len() <= key as usize {
                    self.colours.resize(key as usize + 1, None);
                }
                self.colours[key as usize] = colour;
                picked.keys.push(key);
            }
            for (region, attributes) in set.naval {
                self.naval.insert(region, attributes);
                picked.naval.push(region);
            }
            picked.plans = set.plans.or(picked.plans);
        }
        picked.provinces.sort_unstable();
        picked.provinces.dedup();
        picked.keys.sort_unstable();
        picked.keys.dedup();
        picked
    }

    fn judge_hour(&self, written: Option<(Hour, u64)>, through: Option<Hour>) {
        use viewer_runtime::hour_check as check;
        check::judged();
        if let Some((hour, settled)) = written {
            let shown = digest(&self.keys_of, &self.colours);
            if shown != settled {
                check::found("political", format!("the fill on screen at hour {hour} is not the fill settled at it ({shown:016x} against {settled:016x})"));
            }
        }
        if let Some(hour) = self.due(through).filter(|_| !self.refused) {
            check::found("political", format!("the change set of hour {hour} was left unwritten by a frame whose head stands at {through:?}"));
        }
    }
}

trait Sink {
    fn set_political(&mut self, regions: &Arc<Regions>) -> io::Result<()>;
    fn update_naval(&mut self, attributes: &dyn Fn(u32) -> Attributes, regions: &[u32]) -> io::Result<bool>;
    fn update_political(&mut self, keys_of: &[u32], colour: &dyn Fn(u32) -> [u8; 3], provinces: &[usize], keys: &[u32]) -> io::Result<bool>;
    fn set_border_plans(&mut self, classes: &ClassPlan, zone: &ZonePlan) -> io::Result<()>;
}

struct Scene<'a, 'b>(&'a mut Gpu<'b>);

impl Sink for Scene<'_, '_> {
    fn set_political(&mut self, regions: &Arc<Regions>) -> io::Result<()> {
        self.0.scene.set_political(self.0.device, self.0.queue, regions)
    }

    fn update_naval(&mut self, attributes: &dyn Fn(u32) -> Attributes, regions: &[u32]) -> io::Result<bool> {
        self.0.scene.update_naval(self.0.device, self.0.queue, attributes, regions)
    }

    fn update_political(&mut self, keys_of: &[u32], colour: &dyn Fn(u32) -> [u8; 3], provinces: &[usize], keys: &[u32]) -> io::Result<bool> {
        self.0.scene.update_political(self.0.device, self.0.queue, keys_of, colour, provinces, keys)
    }

    fn set_border_plans(&mut self, classes: &ClassPlan, zone: &ZonePlan) -> io::Result<()> {
        self.0.scene.set_border_plans(self.0.device, classes, zone)
    }
}

#[derive(Default)]
struct Picked {
    whole: Option<Whole>,
    provinces: Vec<usize>,
    keys: Vec<u32>,
    naval: Vec<u32>,
    plans: Option<(ClassPlan, ZonePlan)>,
    hour: Option<Hour>,
    digest_of: Option<(Hour, u64)>,
    built_ns: u64,
}

impl Installer {
    fn write(&mut self, sink: &mut dyn Sink, picked: Picked, through: Option<Hour>, phases: &mut [u64; 4]) -> io::Result<Option<(Hour, u64)>> {
        let Picked { whole, provinces, keys, naval, plans, mut hour, mut digest_of, .. } = picked;
        let mut at = Instant::now();
        let mut lap = |phases: &mut [u64; 4], phase: usize| {
            phases[phase] += at.elapsed().as_nanos() as u64;
            at = Instant::now();
        };
        if let Some(whole) = &whole {
            sink.set_political(&whole.regions)?;
            self.uploads[0] += 1;
        }
        if !naval.is_empty() {
            let attributes = |key: u32| self.naval.get(&key).copied().unwrap_or_default();
            if sink.update_naval(&attributes, &naval)? {
                self.uploads[2] += 1;
            } else {
                self.refused = true;
                self.asks_whole.store(true, Ordering::Release);
                (hour, digest_of) = (None, None);
            }
        }
        lap(phases, 0);
        if !provinces.is_empty() || !keys.is_empty() {
            let colours = &self.colours;
            let colour = |key: u32| colours.get(key as usize).copied().flatten().unwrap_or([0, 0, 0]);
            if sink.update_political(&self.keys_of, &colour, &provinces, &keys)? {
                self.uploads[1] += 1;
            } else {
                self.refused = true;
                self.asks_whole.store(true, Ordering::Release);
                (hour, digest_of) = (None, None);
            }
        }
        lap(phases, 1);
        match plans.filter(|_| !self.refused) {
            Some(plans) => {
                sink.set_border_plans(&plans.0, &plans.1)?;
                if let Some(hour) = hour {
                    self.plans_shown.set(hour);
                }
                self.last_plans = Some(plans);
            }
            None => {}
        }
        lap(phases, 3);
        if let Some(hour) = hour {
            self.at = Some(hour);
        }
        self.owed = self.refused || self.due(through).is_some();
        if !self.owed
            && let (Some(head), Some(dated)) = (through, self.dated)
        {
            let unwritten = self.sets.front().map_or(Hour::MAX, |front| front.from.saturating_sub(1));
            let reached = head.min(dated).min(unwritten);
            self.at = Some(self.at.map_or(reached, |at| at.max(reached)));
        }
        if let Some(at) = self.at {
            self.shown.set(at);
        }
        Ok(digest_of)
    }
}

impl Installs for Installer {
    fn name(&self) -> &'static str {
        "political"
    }

    fn install(&mut self, gpu: &mut Gpu<'_>, through: Option<Hour>, _composed: u64) -> io::Result<()> {
        if !self.live.load(Ordering::Acquire) {
            self.sets.clear();
            (self.at, self.dated, self.owed, self.refused) = (None, None, false, false);
            return Ok(());
        }
        let started = Instant::now();
        let counted = shader_pipeline::gpu::stats::peek();
        let picked = self.pick(through);
        let mut phases = [0, 0, picked.built_ns, 0];
        let digest_of = self.write(&mut Scene(gpu), picked, through, &mut phases)?;
        let written = |count: shader_pipeline::gpu::stats::Count| shader_pipeline::gpu::stats::peek()[count as usize].saturating_sub(counted[count as usize]);
        self.wrote = Wrote {
            ns: started.elapsed().as_nanos() as u64,
            bytes: written(shader_pipeline::gpu::stats::Count::UploadBytes),
            writes: self.uploads,
            phases,
        };
        if viewer_runtime::hour_check::on() {
            self.judge_hour(digest_of, through);
        }
        Ok(())
    }

    fn shown(&self) -> Option<Hour> {
        self.at.filter(|_| self.live.load(Ordering::Acquire))
    }

    fn owes(&self) -> bool {
        self.owed
    }

    fn wrote(&self) -> Wrote {
        self.wrote
    }

    fn reassert(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        if let Some((classes, zone)) = &self.last_plans {
            Scene(gpu).set_border_plans(classes, zone)?;
        }
        Ok(())
    }

    fn plans_shown(&self) -> Option<Hour> {
        self.plans_shown.get()
    }
}

#[cfg(test)]
mod tests;

impl Political {
    fn prepare(&mut self) {
        let refused = self.refused().is_some();
        if refused != self.shown_refused {
            (self.fill_changed, self.edges_changed, self.shown_refused) = (true, true, refused);
            self.changed = stamp();
        }
        let plans = self.edges_changed || self.classes.is_none();
        if !(plans || !self.pending_chunks.is_empty() || self.zone_plan_stale) {
            return;
        }
        let at = std::time::Instant::now();
        if plans {
            let regions = Arc::new(self.regions());
            let zone = Zone::new(&self.content.map, &regions, &self.demilitarized());
            let mut classes = ClassPlan::new(&self.content.meshes, &regions, &zone);
            if let Some(previous) = &self.classes {
                classes.share(previous);
            }
            let zone_plan = ZonePlan::new(&self.content.meshes, &regions, &zone);
            (self.classes, self.zone_plan) = (Some(classes), Some(zone_plan));
            (self.edges_changed, self.zone_plan_stale) = (false, false);
            self.pending_chunks.clear();
            self.prepared_plans = true;
        } else if !self.pending_chunks.is_empty() || self.zone_plan_stale {
            if std::mem::take(&mut self.zone_plan_stale) {
                self.zone_plan = Some(self.zone_plan_of_its_borders(&self.content.meshes));
            }
            if self.classes.is_none() || self.zone_plan.is_none() {
                return;
            }
            let chunks: Vec<usize> = self.pending_chunks.take().into_iter().map(|chunk| chunk as usize).collect();
            let (content, class) = (&self.content, &self.class);
            let meshes = &content.meshes;
            let patch = |draw: usize| {
                let edge = *content.edge_of_draw.get(draw)?;
                match class.get(edge as usize).copied()? {
                    LEFT_OUT => None,
                    bucket => BorderClass::ALL.get(bucket as usize).copied(),
                }
            };
            if let Some(classes) = &mut self.classes {
                classes.patch_chunks(meshes, &content.draws, &chunks, patch);
            }
            self.prepared_plans = true;
        }
        self.built_ns += at.elapsed().as_nanos() as u64;
    }

    fn regions(&self) -> Regions {
        let field = |values: &[u32]| map_display_data::province_field::ProvinceField(Arc::new(map_display_data::province_field::Storage::Dense(Arc::new(values.to_vec()))));
        let refused = self.refused().is_some();
        let empty = vec![0; self.country_of.len()];
        let pick = |values: &[u32]| if refused { field(&empty) } else { field(values) };
        Regions {
            country_of: pick(&self.country_of),
            owner_of: pick(&self.owner_of),
            state_of: pick(&self.state_of),
            country_colors: map_display_data::value_index::ValueIndex::try_from_iter(if refused { Vec::new() } else { self.colours().collect() }).unwrap_or_default(),
            warnings: Vec::new(),
            strategic_attributes: (*self.naval).clone(),
            mode: None,
        }
    }

    pub fn set_naval(&mut self, attributes: &BTreeMap<u32, Attributes>) {
        if *self.naval != *attributes {
            let (old, new) = (&*self.naval, attributes);
            for key in old.keys().chain(new.keys()) {
                if old.get(key) != new.get(key) {
                    self.pending_naval.mark(*key);
                }
            }
            self.naval = Arc::new(attributes.clone());
            self.naval_changed = true;
        }
    }

    fn apply_provinces(&mut self, changes: &Changes, mirror: &Mirror, columns: Columns) {
        let max = self.content.max_defined;
        let mut beyond = false;
        for &key in changes.deleted().keys().unwrap_or_default() {
            let id = key as u32;
            if id > max && mirror.slot(key).is_none() {
                beyond |= self.beyond.remove(&id);
            }
            self.mark_province(id);
        }
        for &key in changes.inserted().keys().unwrap_or_default() {
            let id = key as u32;
            if id > max {
                beyond |= self.beyond.insert(id);
            }
            self.mark_province(id);
        }
        let read = [columns.province_state.column, columns.province_owner.column, columns.province_controller.column];
        for written in changes.columns().iter().filter(|written| read.contains(&written.column())) {
            for &slot in written.slots() {
                if let Some(key) = mirror.key(slot) {
                    self.mark_province(key as u32);
                }
            }
        }
        if beyond {
            self.resize();
        }
    }

    fn date_colours(&mut self, step: &Step, table: TableId, changes: &Changes, mirror: &Mirror, columns: Columns) {
        if self.not_recorded.is_some() {
            return;
        }
        let hours = step.hours_of(table.index());
        match table {
            TableId::Subjects | TableId::Autonomy | TableId::CountryLinks => {
                self.recoloured_at = self.recoloured_at.max(hours.iter().filter_map(Changes::head).max());
                if let Some(first) = hours.iter().filter_map(Changes::head).min() {
                    self.recoloured_first = Some(self.recoloured_first.map_or(first, |held| held.min(first)));
                }
                if table == TableId::CountryLinks
                    && let Some(first) = hours.iter().filter_map(Changes::head).min()
                {
                    self.links_from = Some(self.links_from.map_or(first, |held| held.min(first)));
                }
            }
            TableId::StateFlags => {
                let lived = |set: &Changes| !set.deleted().keys().unwrap_or_default().is_empty() || !set.inserted().keys().unwrap_or_default().is_empty();
                let flagged = hours.iter().filter(|set| lived(set) || set.columns().iter().any(|written| written.column() == columns.demilitarized.column && !written.is_empty())).filter_map(Changes::head);
                for at in flagged {
                    self.zone_at = self.zone_at.max(Some(at));
                    self.zone_first = Some(self.zone_first.map_or(at, |held| held.min(at)));
                }
            }
            TableId::CountryAppearance => {
                let read = [columns.red.column, columns.green.column, columns.blue.column];
                let lived: BTreeSet<u32> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u32).collect();
                self.appearance_reslotted |= !lived.is_empty();
                for set in hours {
                    let Some(hour) = set.head() else { continue };
                    let mut written = Vec::new();
                    for held in set.columns() {
                        let Some(channel) = read.iter().position(|&column| column == held.column()) else { continue };
                        held.each(|slot, value| {
                            if let Some(native) = mirror.key(slot).map(|key| key as u32).filter(|native| !lived.contains(native)) {
                                let bits = if let replay_columnar::store::Value::Number(bits) = value { Some(bits as u32) } else { None };
                                written.push((native, channel, bits));
                            }
                        });
                    }
                    if !written.is_empty() {
                        self.appearance_hours.push((hour, written));
                    }
                }
            }
            _ => {}
        }
    }

    fn clear_inner_diagnostics(&mut self) {
        let inner_only = |layer: &Self, key: u32| key != 0 && layer.interior_keys.contains(&key) && layer.references.get(key as usize).is_none_or(|&count| count == 0);
        let mut undated = None;
        for (at, painted) in &self.hour_sets {
            for &(_, key) in painted {
                let slot = key.wrapping_sub(1);
                if inner_only(self, key) && (self.colour_errors.contains_key(&slot) || self.missing_tags.contains_key(&slot)) {
                    undated = Some(undated.map_or(*at, |held: Hour| held.min(*at)));
                }
            }
        }
        if let Some(at) = undated {
            self.undated(at);
        }
        let keys: Vec<u32> = self.interior_keys.iter().copied().filter(|&key| inner_only(self, key)).collect();
        for key in keys {
            self.colour_errors.remove(&key.wrapping_sub(1));
            self.missing_tags.remove(&key.wrapping_sub(1));
        }
    }

    fn void_hours(&mut self) {
        let hours = self
            .hour_rows
            .iter()
            .map(|(hour, _)| *hour)
            .chain(self.hour_sets.iter().map(|(hour, _)| *hour))
            .chain(self.appearance_hours.iter().map(|(hour, _)| *hour))
            .chain(self.hour_plans.iter().map(|(hour, _)| *hour))
            .chain(self.pending_zone.as_ref().map(|(hour, _)| *hour))
            .chain(self.zone_first)
            .chain(self.recoloured_at)
            .chain(self.undated_from.take());
        if let Some(oldest) = hours.min() {
            self.voided_from = Some(self.voided_from.map_or(oldest, |held| held.min(oldest)));
        }
        self.hour_rows.clear();
        self.hour_sets.clear();
        self.hour_plans.clear();
        self.void_colour_hours();
    }

    fn void_colour_hours(&mut self) {
        self.appearance_hours.clear();
        (self.recoloured_at, self.recoloured_first) = (None, None);
        self.key_before.clear();
        self.interior_keys.clear();
        (self.zone_at, self.zone_first) = (None, None);
        (self.links_from, self.appearance_reslotted) = (None, false);
    }

    #[expect(clippy::type_complexity, reason = "per key its dated colours, and the hour they stop being followed")]
    fn colour_hours(&self, before: &BTreeMap<u32, (Option<[u8; 3]>, u32)>, appearance: &[(Hour, Vec<(u32, usize, Option<u32>)>)], recoloured_at: Option<Hour>) -> (BTreeMap<u32, Vec<(Hour, Option<[u8; 3]>)>>, Option<Hour>) {
        let first_write = |country: u32, before: Option<Hour>| appearance.iter().filter(|(at, _)| before.is_none_or(|before| *at < before)).find(|(_, written)| written.iter().any(|&(held, ..)| held == country)).map(|(at, _)| *at);
        let mut undated: Option<Hour> = None;
        let mut undate = |at: Option<Hour>| {
            if let Some(at) = at {
                undated = Some(undated.map_or(at, |held: Hour| held.min(at)));
            }
        };
        let channels_of = |colour: Option<[u8; 3]>| colour.filter(|&colour| colour != UNAVAILABLE_COLOR).map(|colour| colour.map(Some));
        let colour_of = |channels: [Option<u8>; 3]| match channels {
            [Some(red), Some(green), Some(blue)] => [red, green, blue],
            _ => UNAVAILABLE_COLOR,
        };
        let mut prior: BTreeMap<u32, [Option<u8>; 3]> = BTreeMap::new();
        for &(was, native_was) in before.values() {
            if let Some(channels) = channels_of(was).filter(|_| native_was != NONE) {
                prior.entry(native_was).or_insert(channels);
            }
        }
        let mut by_key = BTreeMap::new();
        for (&key, &(was, native_was)) in before {
            let now = self.colours.get(key as usize).copied().flatten();
            let native = self.colour_native.get(key as usize).copied().unwrap_or(NONE);
            let mut hours = Vec::new();
            let followed = match (native == native_was, recoloured_at) {
                _ if native == NONE => None,
                (true, _) => channels_of(was).map(|start| (None, start)),
                (false, Some(from)) => prior.get(&native).copied().or_else(|| channels_of(now)).map(|start| (Some(from), start)),
                (false, None) => None,
            };
            if followed.is_none() && native != NONE {
                undate(first_write(native, None));
            }
            if native_was != NONE && native_was != native {
                undate(first_write(native_was, recoloured_at));
            }
            if let Some((from, mut channels)) = followed {
                let write = |channels: &mut [Option<u8>; 3], written: &[(u32, usize, Option<u32>)]| {
                    let mut touched = false;
                    for &(_, channel, bits) in written.iter().filter(|&&(country, ..)| country == native) {
                        channels[channel] = bits.map(|bits| (f32::from_bits(bits) * 255.0) as u8);
                        touched = true;
                    }
                    touched
                };
                if let Some(from) = from {
                    let mut at_switch = channels;
                    for (_, written) in appearance.iter().filter(|(at, _)| *at <= from) {
                        write(&mut at_switch, written);
                    }
                    hours.push((from, Some(colour_of(at_switch))));
                }
                for (at, written) in appearance {
                    if write(&mut channels, written) && from.is_none_or(|from| *at > from) {
                        hours.push((*at, Some(colour_of(channels))));
                    }
                }
            }
            if hours.is_empty()
                && was != now
                && let Some(at) = recoloured_at
            {
                hours.push((at, now));
            }
            if !hours.is_empty() {
                by_key.insert(key, hours);
            }
        }
        (by_key, undated)
    }

    fn dated_hours(&self, hours: Vec<(Hour, Vec<(u32, u32)>)>, before: &BTreeMap<u32, (Option<[u8; 3]>, u32)>, appearance: &[(Hour, Vec<(u32, usize, Option<u32>)>)], recoloured_at: Option<Hour>, last: Hour) -> (Vec<DatedHour>, Option<Hour>) {
        let (coloured, undated) = self.colour_hours(before, appearance, recoloured_at);
        let colour_at = |key: u32, at: Hour| match (coloured.get(&key).and_then(|hours| hours.iter().rev().find(|&&(when, _)| when <= at)), before.get(&key)) {
            (Some(&(_, colour)), _) => colour,
            (None, Some(&(Some(was), _))) => Some(was),
            _ => self.colours.get(key as usize).copied().flatten(),
        };
        let mut recoloured: BTreeMap<Hour, BTreeSet<u32>> = BTreeMap::new();
        for (&key, hours) in &coloured {
            for &(at, _) in hours {
                recoloured.entry(at).or_default().insert(key);
            }
        }
        let mut painted: BTreeMap<Hour, Vec<(u32, u32)>> = hours.into_iter().collect();
        let every: BTreeSet<Hour> = painted.keys().chain(recoloured.keys()).copied().filter(|&at| at <= last).collect();
        let dated = every
            .into_iter()
            .map(|at| {
                let provinces = painted.remove(&at).unwrap_or_default();
                let mut keys: BTreeSet<u32> = provinces.iter().map(|&(_, key)| key).filter(|&key| key != 0).collect();
                keys.extend(recoloured.get(&at).into_iter().flatten());
                (at, provinces, keys.into_iter().map(|key| (key, colour_at(key, at))).collect())
            })
            .collect();
        (dated, undated)
    }

    fn read_hours(&mut self, step: &Step, changes: &Changes, mirror: &Mirror, columns: Columns) {
        let hours = step.hours_of(TableId::Provinces.index());
        if hours.is_empty() || self.not_recorded.is_some() {
            return;
        }
        let read = [columns.province_state.column, columns.province_owner.column, columns.province_controller.column];
        let lived: BTreeSet<u32> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u32).collect();
        let mut written: Vec<(u32, usize, Option<u32>)> = Vec::new();
        for set in hours {
            let Some(hour) = set.head() else { continue };
            written.clear();
            for held in set.columns() {
                let Some(field) = read.iter().position(|&column| column == held.column()) else { continue };
                held.each(|slot, value| {
                    if let Some(key) = mirror.key(slot) {
                        let held = if let replay_columnar::store::Value::Number(bits) = value { Some(bits as u32) } else { None };
                        written.push((key as u32, field, held));
                    }
                });
            }
            written.sort_unstable_by_key(|&(id, ..)| id);
            let mut rows: Vec<(u32, Row)> = Vec::new();
            let mut at = 0;
            while at < written.len() {
                let id = written[at].0;
                let mut row = self.recorded.get(id as usize).copied().flatten();
                while at < written.len() && written[at].0 == id {
                    let (_, field, value) = written[at];
                    if let Some(row) = row.as_mut() {
                        match field {
                            0 => row.state = value,
                            1 => row.owner = value,
                            _ => row.controller = value,
                        }
                    }
                    at += 1;
                }
                let (Some(row), false) = (row, lived.contains(&id)) else { continue };
                if let Some(held) = self.recorded.get_mut(id as usize) {
                    *held = Some(row);
                }
                rows.push((id, row));
            }
            if !rows.is_empty() {
                self.hour_rows.push((hour, rows));
            }
        }
    }

    fn paint_hours(&mut self) {
        let mut hours = std::mem::take(&mut self.hour_rows);
        for (hour, rows) in hours.drain(..) {
            if self.pending_zone.as_ref().is_some_and(|&(at, _)| at < hour) {
                self.apply_zone();
            }
            let painted_any = !rows.is_empty();
            for (id, row) in rows {
                if (id as usize) < self.count {
                    self.paint_from(id, Some(row));
                    if self.errors.contains_key(&id) {
                        self.undated(hour);
                    }
                }
            }
            if painted_any && self.links_from.is_some_and(|links| hour < links) {
                self.undated(hour);
            }
            let painted: Vec<(u32, u32)> = self.pending_provinces.take().into_iter().map(|id| (id, self.country_of.get(id as usize).copied().unwrap_or(0))).collect();
            if !painted.is_empty() {
                for &(_, key) in &painted {
                    if key != 0 && self.interior_keys.insert(key) {
                        self.mark_key(key);
                    }
                }
                self.hour_plans_at(hour);
                self.hour_sets.push((hour, painted));
            }
        }
        self.hour_rows = hours;
        if self.pending_zone.is_some() {
            self.apply_zone();
        }
        if let Some(links) = self.links_from {
            self.undated(links);
        }
    }

    fn apply_zone(&mut self) {
        let Some((at, zone)) = self.pending_zone.take() else { return };
        self.rezone(zone);
        if at < self.hour {
            self.hour_plans_at(at);
        }
    }

    fn hour_plans_at(&mut self, hour: Hour) {
        if self.classes.is_none() {
            return;
        }
        if !self.edges_changed {
            let mut dirty = std::mem::take(&mut self.edges);
            let _ = dirty.settle(|edge| {
                self.classify(edge);
                Ok(())
            });
            self.edges = dirty;
        }
        if self.edges_changed {
            self.undated(hour);
            return;
        }
        if self.pending_chunks.is_empty() && !self.zone_plan_stale {
            return;
        }
        self.prepare();
        if let Some(plans) = self.classes.clone().zip(self.zone_plan.clone()) {
            self.hour_plans.push((hour, plans));
        }
    }

    fn apply_countries(&mut self, changes: &Changes, mirror: &Mirror, columns: Columns) {
        let mut changed = BTreeSet::new();
        for &key in changes.deleted().keys().unwrap_or_default() {
            if let Some(slot) = self.native_slot.remove(&(key as u32)) {
                self.natives.remove(&(slot, key as u32));
                changed.insert(slot);
            }
        }
        let rewritten = changes.columns().iter().filter(|written| written.column() == columns.country_slot.column).flat_map(|written| written.slots());
        let written: Vec<u32> = changes.inserted().slots().iter().chain(rewritten).copied().collect();
        for slot in written {
            let (Some(key), true) = (mirror.key(slot), mirror.live(slot)) else { continue };
            let native = key as u32;
            if let Some(old) = self.native_slot.remove(&native) {
                self.natives.remove(&(old, native));
                changed.insert(old);
            }
            if let Some(canonical) = mirror.number(columns.country_slot, slot).value() {
                self.natives.insert((canonical, native));
                self.native_slot.insert(native, canonical);
                changed.insert(canonical);
            }
        }
        let lost: BTreeSet<u32> = changed.into_iter().filter(|&slot| self.by_slot(slot).is_none() && self.references.get(slot as usize + 1).is_some_and(|&count| count > 0)).map(|slot| slot + 1).collect();
        if !lost.is_empty() {
            let gone: Vec<u32> = (0..self.count as u32).filter(|&id| lost.contains(&self.country_of[id as usize])).collect();
            gone.into_iter().for_each(|id| self.mark_province(id));
        }
        let errors: Vec<u32> = self.errors.keys().copied().collect();
        errors.into_iter().for_each(|id| self.mark_province(id));
        self.mark_referenced();
    }

    fn apply_links(&mut self, changes: &Changes, mirror: &Mirror, columns: Columns) {
        let mut changed = HashSet::new();
        for &key in changes.deleted().keys().unwrap_or_default() {
            if mirror.slot(key).is_none() {
                self.links.remove(&(key as u32));
            }
            changed.insert(key as u32);
        }
        let rewritten = changes.columns().iter().filter(|written| written.column() == columns.link_slot.column).flat_map(|written| written.slots());
        let written: Vec<u32> = changes.inserted().slots().iter().chain(rewritten).copied().collect();
        for slot in written {
            let (Some(key), true) = (mirror.key(slot), mirror.live(slot)) else { continue };
            match mirror.number(columns.link_slot, slot).value() {
                Some(canonical) => self.links.insert(key as u32, canonical),
                None => self.links.remove(&(key as u32)),
            };
            changed.insert(key as u32);
        }
        if changed.is_empty() {
            return;
        }
        let touched: Vec<u32> = changed.iter().flat_map(|&raw| self.by_link.range((raw, 0)..=(raw, u32::MAX)).map(|&(_, id)| id)).collect();
        touched.into_iter().for_each(|id| self.mark_province(id));
        let errors: Vec<u32> = self.errors.keys().copied().collect();
        errors.into_iter().for_each(|id| self.mark_province(id));
        self.relations_dirty = true;
        self.mark_referenced();
    }
}

#[derive(Default)]
struct Pending {
    bits: Vec<u64>,
    list: Vec<u32>,
}

impl Pending {
    fn mark(&mut self, entity: u32) {
        let word = entity as usize / 64;
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
        let bit = 1 << (entity % 64);
        if self.bits[word] & bit == 0 {
            self.bits[word] |= bit;
            self.list.push(entity);
        }
    }

    fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn take(&mut self) -> Vec<u32> {
        self.list.iter().for_each(|&entity| self.bits[entity as usize / 64] = 0);
        std::mem::take(&mut self.list)
    }

    fn clear(&mut self) {
        self.take();
    }

    fn bytes(&self) -> usize {
        self.bits.capacity() * 8 + self.list.capacity() * 4
    }
}
