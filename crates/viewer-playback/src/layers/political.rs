//! The political fill and the border classes, from the mirrors: per province its fill key (the
//! controller's canonical slot plus one), border owner key and state; per fill key its colour after
//! the subject colour rule; per border edge its class, the demilitarised edges left out. These are
//! the values today's territory fold and border plans derive from a frame of rows, kept per entity.
//!
//! Drawn at the default map mode only. Another stock mode is the mode fills layer's
//! ([`super::mode_fills`]): there it declares and holds nothing. A scripted mode is refused by name.
//! The sea regions' naval control and contested bytes (perspective-bound) are not this layer's.

use super::{Budget, Carried, Dirty, Gpu, Inputs, Layer, Num, Text, View};
use crate::playhead_host::mirror::{Cell, Mirror, Mirrors};
use installed_content_core::map::{MapData, ProvinceKind};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};
use viewer_world::map::{
    borders::{BorderClass, BorderMeshes},
    map_mode::MapMode,
    border_pass::ClassPlan,
    demilitarized::{Zone, ZonePlan},
    gradient::Attributes,
    political::{Regions, UNAVAILABLE_COLOR},
};

const NONE: u32 = u32::MAX;
/// An edge's class: a land class for the province bucket, the bucket's own class otherwise; or
/// left out, drawn by the demilitarised pass instead.
pub const LEFT_OUT: u8 = u8::MAX;
/// Provinces a fill upload writes at most: a change larger than this goes over several frames.
const FILL_PIECE: usize = 1024;
/// The border buckets the class plan draws: the province bucket's edges re-bucketed into the three
/// land classes, and three classes kept as traced.
pub const BUCKETS: [BorderClass; 4] = [BorderClass::Province, BorderClass::Impassable, BorderClass::Sea, BorderClass::SeaRegion];

const LAND: u8 = 1;
const SEA: u8 = 2;
const LAKE: u8 = 3;

/// What the layer reads of the content: province kinds and the traced border edges.
pub struct Content {
    map: Arc<MapData>,
    /// Per province id: 0 undefined, or `LAND`, `SEA`, `LAKE`.
    kind: Vec<u8>,
    max_defined: u32,
    /// Every drawn edge's two provinces, the buckets of [`BUCKETS`] one after another.
    edges: Vec<(u32, u32)>,
    /// Per edge of the province bucket: the vertex chunk its draw is in; and per draw of that
    /// bucket, its edge (or none for a draw outside every chunk).
    chunk_of: Vec<u32>,
    edge_of_draw: Vec<u32>,
    /// The province bucket's draws by chunk ([`ClassPlan::chunk_draws`]).
    draws: Vec<Vec<u32>>,
    /// Where each bucket's edges start in `edges`, and the end.
    bucket_start: [usize; BUCKETS.len() + 1],
    /// Per province id, its edges: `edge_list[edge_start[id]..edge_start[id + 1]]`.
    edge_start: Vec<u32>,
    edge_list: Vec<u32>,
    /// Per province id, the draws of every class's mesh along its boundaries (class, index):
    /// `draw_list[draw_start[id]..draw_start[id + 1]]`, so the zone's plan reads its own borders.
    draw_start: Vec<u32>,
    draw_list: Vec<(BorderClass, u32)>,
}

impl Content {
    pub fn new(map: Arc<MapData>, meshes: &BorderMeshes) -> Self {
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
        let draws = ClassPlan::chunk_draws(meshes);
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
        Self { map, kind, max_defined, edges, chunk_of, edge_of_draw, draws, bucket_start, edge_start: counts, edge_list, draw_start, draw_list }
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

    /// Every edge's provinces, and where each bucket of [`BUCKETS`] starts.
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

/// The subject colour rule at one hour, read whole from Subjects, Autonomy and the links.
#[derive(Clone, Debug, Default, PartialEq)]
struct Relations {
    inherited: BTreeMap<u32, u32>,
    undecided: BTreeMap<u32, String>,
    states: BTreeMap<u32, Vec<u8>>,
    autonomy_gap: Option<String>,
    error: Option<String>,
    warnings: Vec<String>,
    /// The next hour an active subject relation ends at.
    next_end: Option<Hour>,
}

impl Relations {
    /// The slot whose colour `slot` takes; none when the hour cannot say.
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

/// Why a table's rows are not all of it, in the words today's warnings use; none when complete.
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

/// The rows a table holds at the head: none when unavailable or not recorded.
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

/// The political layer's arrays and what keeps them.
pub struct Political {
    content: Arc<Content>,
    columns: Option<Columns>,
    /// Why nothing is drawn: a core table not recorded, a refused mode, or an invalid hour.
    not_recorded: Option<String>,
    count: usize,
    country_of: Vec<u32>,
    owner_of: Vec<u32>,
    state_of: Vec<u32>,
    /// Per fill key: provinces filled with it, and its colour while any is.
    references: Vec<u32>,
    colours: Vec<Option<[u8; 3]>>,
    /// Per fill key: the native country whose appearance colours it.
    colour_native: Vec<u32>,
    /// Per edge of the content: its class, or [`LEFT_OUT`].
    class: Vec<u8>,
    unresolved: BTreeSet<u32>,
    /// Recorded province ids past the map's definitions.
    beyond: BTreeSet<u32>,
    errors: BTreeMap<u32, String>,
    colour_errors: BTreeMap<u32, (u32, String)>,
    /// Per slot without a recorded colour: its country's tag.
    missing_tags: BTreeMap<u32, String>,
    /// Raw country id to canonical slot.
    links: HashMap<u32, u32>,
    /// Per province: the owner and controller links it was painted from; and each link's
    /// provinces, so a link's change finds them.
    raws: Vec<[u32; 2]>,
    by_link: BTreeSet<(u32, u32)>,
    /// Canonical slot and native id of every Countries row, and each native's slot.
    natives: BTreeSet<(u32, u32)>,
    native_slot: HashMap<u32, u32>,
    relations: Relations,
    /// Why CountryAppearance's rows are not all of it.
    appearance_gap: Option<String>,
    demilitarized: HashSet<u32>,
    provinces: Dirty,
    keys: Dirty,
    edges: Dirty,
    relations_dirty: bool,
    zone_dirty: bool,
    /// Each read table's status as last seen, by table index: a change of it, not of the sample
    /// hour, is what the layer reacts to.
    statuses: HashMap<usize, Status>,
    hour: Hour,
    frame: u64,
    /// Provinces, fill keys and edges the steps have marked.
    marked: [usize; 3],
    /// The fill (keys, owner keys, states, colours) or the edges' classes changed since the last
    /// upload; and the plan last uploaded, whose unchanged runs the next one shares.
    fill_changed: bool,
    edges_changed: bool,
    classes: Option<ClassPlan>,
    zone_plan: Option<ZonePlan>,
    /// The demilitarised edges changed: the zone's plan is made again, the class plan patched.
    zone_plan_stale: bool,
    /// What changed since the last upload: provinces whose keys did, keys whose colour did, and
    /// the vertex chunks holding edges whose land class did. A whole upload (`fill_changed`,
    /// `edges_changed`) is taken at a compose, a refusal's start or end, a naval change, or a
    /// demilitarised split of an edge outside the land chunks.
    pending_provinces: Pending,
    pending_keys: Pending,
    pending_chunks: Pending,
    /// The sea regions whose naval attributes changed, by key (region id plus one, as today's
    /// attributes are keyed).
    pending_naval: Pending,
    /// Whether the fill on the GPU is the refused (empty) one.
    shown_refused: bool,
    /// The sea regions' naval attributes the fill carries, from today's political part until they
    /// are a layer of their own.
    naval: Arc<BTreeMap<u32, Attributes>>,
    naval_changed: bool,
    /// Uploads taken: the fill whole, the country fill by what changed, the naval layer by what
    /// changed; the harness reports them, so a run shows which entry drew its frames.
    pub uploads: [u64; 3],
    /// The last upload's nanoseconds: the naval layer, the country fill, the border plans built
    /// (zone, class and zone plans, or the chunks patched), and the plans written to the scene.
    pub phases: [u64; 4],
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
            fill_changed: true,
            edges_changed: true,
            classes: None,
            zone_plan: None,
            zone_plan_stale: false,
            pending_provinces: Pending::default(),
            pending_keys: Pending::default(),
            pending_chunks: Pending::default(),
            pending_naval: Pending::default(),
            shown_refused: false,
            naval: Arc::default(),
            naval_changed: false,
            uploads: [0; 3],
            phases: [0; 4],
        }
    }

    /// A layer of the same content and inputs, made from scratch.
    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new(Arc::clone(&self.content)) }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    /// Whether it draws at the view: declared, at the default map mode.
    pub fn drawn(&self) -> bool {
        self.columns.is_some()
    }

    /// Why the layer draws nothing at the head, by name: a core table not recorded or the hour
    /// refused; none when it draws.
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

    /// Every fill key a province takes, with its colour.
    pub fn colours(&self) -> impl Iterator<Item = (u32, [u8; 3])> + '_ {
        self.colours.iter().enumerate().filter_map(|(key, colour)| colour.map(|colour| (key as u32, colour)))
    }

    /// Each edge's class, in the content's edge order.
    pub fn classes(&self) -> &[u8] {
        &self.class
    }

    pub fn demilitarized(&self) -> BTreeSet<u32> {
        self.demilitarized.iter().copied().collect()
    }

    /// The hour's limitations, in today's words and order.
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

    /// Provinces, Countries and CountryLinks, each wholly recorded at the head, or why not.
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

    /// Everything dropped and marked: the settle that follows makes the arrays whole.
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

    /// The province tables sized to the map's definitions and the recorded ids past them; new
    /// ids are marked.
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

    /// Every fill key a province takes or that holds a colour, to be coloured again.
    fn mark_referenced(&mut self) {
        for key in 1..self.references.len() as u32 {
            if self.references[key as usize] > 0 || self.colours[key as usize].is_some() {
                self.mark_key(key);
            }
        }
    }

    /// The canonical slot's country: the highest native id linked to it.
    fn by_slot(&self, slot: u32) -> Option<u32> {
        self.natives.range((slot, 0)..=(slot, u32::MAX)).next_back().map(|&(_, native)| native)
    }

    /// One province's fill key, owner key and state, or its error; marks what its change touches.
    fn paint(&mut self, id: u32, mirrors: &Mirrors) {
        let columns = self.columns();
        let provinces = mirrors.get(TableId::Provinces).expect("the core is read");
        self.errors.remove(&id);
        self.unresolved.remove(&id);
        let kind = self.content.kind(id);
        let mut unresolved = false;
        let links = provinces
            .slot(u128::from(id))
            .map(|slot| [provinces.number(columns.province_owner, slot).value().unwrap_or(0), provinces.number(columns.province_controller, slot).value().unwrap_or(0)])
            .unwrap_or([0; 2]);
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
        let painted: Result<[u32; 3], String> = match provinces.slot(u128::from(id)) {
            None if id != 0 && kind != 0 => Err(format!("hour {} has no recorded row for map province {id}", self.hour)),
            None => Ok([0; 3]),
            Some(slot) => match (provinces.number(columns.province_state, slot), provinces.number(columns.province_owner, slot), provinces.number(columns.province_controller, slot)) {
                (Cell::Value(state), Cell::Value(owner), Cell::Value(controller)) => (|| {
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
                })(),
                _ => Err(format!("hour {} does not record province {id}'s state, owner and controller", self.hour)),
            },
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

    /// One fill key's colour after the subject rule, or its error.
    fn colour(&mut self, key: u32, mirrors: &Mirrors) {
        let before = self.colours.get(key as usize).copied().flatten();
        self.recolour(key, mirrors);
        if self.colours.get(key as usize).copied().flatten() != before {
            self.pending_keys.mark(key);
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
        if self.references[at] == 0 {
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

    /// One edge's class from its provinces' owner keys and states, and the demilitarised zone.
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

    /// The zone's plan from the draws along the zone's own provinces, as [`ZonePlan::new`] makes it
    /// from every draw: the edges it splits are among them, classed by the same owners and states.
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

    /// A new demilitarised zone: only the edges of the states that entered or left it can split or
    /// join, and are classified again; the zone's plan is made again at the upload.
    fn rezone(&mut self, zone: HashSet<u32>) {
        let moved: HashSet<u32> = zone.symmetric_difference(&self.demilitarized).copied().collect();
        self.demilitarized = zone;
        self.zone_plan_stale = true;
        let content = Arc::clone(&self.content);
        for (province, state) in self.state_of.iter().enumerate() {
            if moved.contains(state) {
                for &edge in content.edges_of(province as u32) {
                    self.edges.mark(edge, self.frame);
                }
            }
        }
    }

    /// The demilitarised states: flagged and existing, when both tables are wholly recorded.
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

    /// The subject colour rule, read whole.
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

    /// Recomputes what is dirty within `budget`: the relations and the zone whole, then provinces,
    /// fill keys and edges, the rest carried.
    fn settle_dirty(&mut self, mirrors: &Mirrors, budget: &mut Budget) {
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
            if zone != self.demilitarized {
                self.rezone(zone);
            }
        }
        let mut dirty = std::mem::take(&mut self.provinces);
        let _ = dirty.settle(budget, |id| {
            if (id as usize) < self.count {
                self.paint(id, mirrors);
            }
            Ok(())
        });
        self.provinces = dirty;
        let mut dirty = std::mem::take(&mut self.keys);
        let _ = dirty.settle(budget, |key| {
            self.colour(key, mirrors);
            Ok(())
        });
        self.keys = dirty;
        let mut dirty = std::mem::take(&mut self.edges);
        let _ = dirty.settle(budget, |edge| {
            self.classify(edge);
            Ok(())
        });
        self.edges = dirty;
    }

    /// Provinces, fill keys and edges the steps have marked since the layer was made.
    pub fn marked(&self) -> [usize; 3] {
        self.marked
    }

    /// Entries of `self` that differ from `other`'s, and the first described.
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

    /// The content's static bytes, shared by every copy of the layer.
    pub fn content_bytes(&self) -> usize {
        self.content.bytes()
    }
}

impl Layer for Political {
    fn name(&self) -> &'static str {
        "political"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if let MapMode::Scripted(_) = view.map_mode {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("The political layer is refused: scripted map mode {} keeps its borders from one refresh to the next, which no layer follows yet", view.map_mode.token()),
            ));
        }
        if view.map_mode != MapMode::default() {
            *self = Self::new(Arc::clone(&self.content));
            return Ok(());
        }
        for table in [TableId::Provinces, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::Subjects, TableId::Autonomy, TableId::StateFlags, TableId::States] {
            inputs.table(table, true)?;
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
        let marked = self.marked;
        self.not_recorded = Some("composing".into());
        self.settle_dirty(mirrors, &mut Budget::unlimited());
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() {
            return;
        }
        let columns = self.columns();
        if self.relations.next_end.is_some_and(|end| step.last() >= end) {
            self.relations_dirty = true;
        }
        let colour_columns = [columns.red.column, columns.green.column, columns.blue.column];
        for changes in step.tables() {
            let restated = changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status));
            let touched = |changes: &Changes, wanted: &[usize]| {
                restated || !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty() && wanted.contains(&written.column()))
            };
            let table = TableId::ALL[changes.table()];
            let Some(mirror) = mirrors.get(table) else { continue };
            match table {
                TableId::Provinces => self.apply_provinces(changes, mirror, columns),
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

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        if self.columns.is_some() {
            self.settle_dirty(mirrors, budget);
        }
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>, budget: &mut Budget) -> io::Result<()> {
        if self.columns.is_none() || !budget.peek() {
            return Ok(());
        }
        let refused = self.refused().is_some();
        if refused != self.shown_refused {
            (self.fill_changed, self.edges_changed, self.shown_refused) = (true, true, refused);
        }
        if self.uploaded() {
            return Ok(());
        }
        let meshes = &gpu.content.border_meshes;
        let mut whole = None;
        let (mut phases, mut at) = ([0u64; 4], std::time::Instant::now());
        let lap = |phases: &mut [u64; 4], phase: usize, at: &mut std::time::Instant| {
            phases[phase] += at.elapsed().as_nanos() as u64;
            *at = std::time::Instant::now();
        };
        if self.naval_changed && !self.fill_changed {
            let keys = self.pending_naval.take();
            let naval = Arc::clone(&self.naval);
            let attributes = |key: u32| naval.get(&key).copied().unwrap_or_default();
            self.fill_changed = !gpu.scene.update_naval(gpu.device, gpu.queue, &attributes, &keys)?;
            self.uploads[2] += u64::from(!self.fill_changed);
            self.naval_changed = false;
        }
        lap(&mut phases, 0, &mut at);
        if self.fill_changed || self.naval_changed {
            let regions = Arc::new(self.regions());
            gpu.scene.set_political(gpu.device, gpu.queue, &regions)?;
            self.uploads[0] += 1;
            (self.fill_changed, self.naval_changed) = (false, false);
            self.pending_provinces.clear();
            self.pending_keys.clear();
            self.pending_naval.clear();
            whole = Some(regions);
        } else if !self.pending_provinces.is_empty() || !self.pending_keys.is_empty() {
            // A large change (a peace, a seek's first frames) is written a piece a frame, oldest first.
            let provinces: Vec<usize> = self.pending_provinces.take_first(FILL_PIECE).into_iter().map(|id| id as usize).collect();
            let keys = self.pending_keys.take();
            let colours = &self.colours;
            let colour = |key: u32| colours.get(key as usize).copied().flatten().unwrap_or([0, 0, 0]);
            if gpu.scene.update_political(gpu.device, gpu.queue, &self.country_of, &colour, &provinces, &keys)? {
                self.uploads[1] += 1;
            } else {
                let regions = Arc::new(self.regions());
                gpu.scene.set_political(gpu.device, gpu.queue, &regions)?;
                self.uploads[0] += 1;
                self.pending_provinces.clear();
                whole = Some(regions);
            }
        }
        lap(&mut phases, 1, &mut at);
        if !budget.left() {
            self.phases = phases;
            return Ok(());
        }
        if self.edges_changed || self.classes.is_none() {
            let regions = whole.unwrap_or_else(|| Arc::new(self.regions()));
            let zone = Zone::new(&self.content.map, &regions, &self.demilitarized());
            let mut classes = ClassPlan::new(meshes, &regions, &zone);
            if let Some(previous) = &self.classes {
                classes.share(previous);
            }
            let zone_plan = ZonePlan::new(meshes, &regions, &zone);
            lap(&mut phases, 2, &mut at);
            gpu.scene.set_border_plans(gpu.device, &classes, &zone_plan)?;
            lap(&mut phases, 3, &mut at);
            (self.classes, self.zone_plan) = (Some(classes), Some(zone_plan));
            (self.edges_changed, self.zone_plan_stale) = (false, false);
            self.pending_chunks.clear();
        } else if !self.pending_chunks.is_empty() || self.zone_plan_stale {
            if std::mem::take(&mut self.zone_plan_stale) {
                self.zone_plan = Some(self.zone_plan_of_its_borders(meshes));
            }
            let (Some(classes), Some(zone_plan)) = (&mut self.classes, &self.zone_plan) else { return Ok(()) };
            let chunks: Vec<usize> = self.pending_chunks.take().into_iter().map(|chunk| chunk as usize).collect();
            let (content, class) = (&self.content, &self.class);
            classes.patch_chunks(meshes, &content.draws, &chunks, |draw| {
                let edge = *content.edge_of_draw.get(draw)?;
                match class.get(edge as usize).copied()? {
                    LEFT_OUT => None,
                    bucket => BorderClass::ALL.get(bucket as usize).copied(),
                }
            });
            lap(&mut phases, 2, &mut at);
            gpu.scene.set_border_plans(gpu.device, classes, zone_plan)?;
            lap(&mut phases, 3, &mut at);
        }
        self.phases = phases;
        Ok(())
    }

    fn uploaded(&self) -> bool {
        // Undeclared, it draws nothing and owes the scene nothing.
        self.columns.is_none()
            || (!(self.fill_changed || self.naval_changed || self.edges_changed || self.zone_plan_stale)
                && self.pending_provinces.is_empty()
                && self.pending_keys.is_empty()
                && self.pending_chunks.is_empty())
    }

    fn carried(&self) -> Carried {
        let entities = self.provinces.len() + self.keys.len() + self.edges.len() + usize::from(self.relations_dirty) + usize::from(self.zone_dirty);
        let since = [self.provinces.since(), self.keys.since(), self.edges.since()].into_iter().flatten().min();
        Carried { entities, frames: since.map_or(0, |since| self.frame.saturating_sub(since)) }
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

#[cfg(test)]
mod tests;

impl Political {
    /// The fill the scene draws: the layer's keys, owner keys, states and colours; nothing while it
    /// refuses the hour (the political picture is withheld, as today's refusal withholds it).
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

    /// Today's naval attributes for the sea regions, carried in the fill until they are a layer.
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

/// Entities named once each, in the order first named.
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

    /// The `count` marked first, the rest left marked.
    fn take_first(&mut self, count: usize) -> Vec<u32> {
        let taken: Vec<u32> = self.list.drain(..count.min(self.list.len())).collect();
        taken.iter().for_each(|&entity| self.bits[entity as usize / 64] &= !(1 << (entity % 64)));
        taken
    }

    fn clear(&mut self) {
        self.take();
    }

    fn bytes(&self) -> usize {
        self.bits.capacity() * 8 + self.list.capacity() * 4
    }
}
