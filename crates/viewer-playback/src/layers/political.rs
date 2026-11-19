//! The political fill and the border classes, from the mirrors: per province its fill key (the
//! controller's canonical slot plus one), border owner key and state; per fill key its colour after
//! the subject colour rule; per border edge its class, the demilitarised edges left out. These are
//! the values today's territory fold and border plans derive from a frame of rows, kept per entity.
//!
//! Drawn at the default map mode only. Another stock mode is the mode fills layer's
//! ([`super::mode_fills`]): there it declares and holds nothing. A scripted mode is refused by name.
//! The sea regions' naval control and contested bytes (perspective-bound) are not this layer's.

use super::{Carried, Settling, Dirty, Gpu, Inputs, Installs, Layer, Num, Scope, Scoped, Shown, Sight, Text, View, Wrote};
use crate::playhead_host::mirror::{Cell, Mirror, Mirrors};
use installed_content_core::map::{MapData, ProvinceKind};
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
/// The border buckets the class plan draws: the province bucket's edges re-bucketed into the three
/// land classes, and three classes kept as traced.
pub const BUCKETS: [BorderClass; 4] = [BorderClass::Province, BorderClass::Impassable, BorderClass::Sea, BorderClass::SeaRegion];

const LAND: u8 = 1;
const SEA: u8 = 2;
const LAKE: u8 = 3;

/// What the layer reads of the content: province kinds and the traced border edges.
pub struct Content {
    map: Arc<MapData>,
    meshes: Arc<BorderMeshes>,
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

/// What one province's row says at an hour: the three cells the fill is painted from, each none
/// where the hour does not record it. The layer keeps the last it read of every province
/// ([`Political::recorded`]), because an hour's own change set names only the cells that hour
/// wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    state: Option<u32>,
    owner: Option<u32>,
    controller: Option<u32>,
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

/// The fill's own digest: every province's key, then every fill key that holds a colour with it.
/// Taken only while `BATTLE_PLAN_VIEWER_CHECK_HOUR` is on.
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

/// One step's own change set of the fill and the borders: what became true in the hours that step
/// covers, with the values they took there, so a display frame whose head has reached `hour`
/// writes exactly it and nothing of a later hour. Handed to the frame's [`Installer`] as the step
/// is settled ([`Layer::timed_at`]), which is the last the side's thread has to do with it.
struct Set {
    hour: Hour,
    /// The oldest hour folded into it ([`Handover::push`]); `hour` itself for a set of one hour.
    from: Hour,
    /// The picture the side had composed when the step was dated: a compose since makes the set
    /// void, so nothing of the hours a seek left is written after it.
    composed: u64,
    /// The fill whole at `hour`, where no hour of the interval can be told apart in it: a compose,
    /// a refusal starting or ending, or a changed write the fill refused. Every set handed over
    /// before it is void.
    whole: Option<Whole>,
    /// Per province whose fill key changed: its key at `hour`.
    provinces: Vec<(u32, u32)>,
    /// Per fill key whose colour changed: its colour at `hour`; none while it is withheld.
    keys: Vec<(u32, Option<[u8; 3]>)>,
    /// Per sea region whose naval attributes changed: its attributes at `hour`, by key (region id
    /// plus one), as today's political part carries them.
    naval: Vec<(u32, Attributes)>,
    /// The border plans as that hour left them, where the step changed an edge's class. A copy
    /// shares every unchanged run's indices with the plan before it ([`ClassPlan::share`]), so the
    /// pass keeps the buffers it has and writes only the chunks that changed.
    plans: Option<(ClassPlan, ZonePlan)>,
    /// What building those plans took on the side's thread, for the run's record.
    built_ns: u64,
    /// The digest of the fill settled at `hour`, kept only while `BATTLE_PLAN_VIEWER_CHECK_HOUR` is on.
    digest: Option<u64>,
}

/// One hour of a publication as its set takes it: the hour, the provinces it repainted with their
/// keys, and the keys it writes with their colours at that hour.
type DatedHour = (Hour, Vec<(u32, u32)>, Vec<(u32, Option<[u8; 3]>)>);

/// A stamp no picture of any political layer has held before ([`Political::changed`]).
fn stamp() -> u64 {
    static STAMPS: AtomicU64 = AtomicU64::new(0);
    STAMPS.fetch_add(1, Ordering::Relaxed) + 1
}

/// Change sets the hand-over holds before it folds the oldest two together: more than the hours of
/// a publication at the slowest refresh, so frames that keep taking them never see a fold. Past it
/// frames stopped taking them, and the oldest hours stop being told apart rather than being lost.
const SETS_HELD: usize = 4096;

/// Where the side's thread leaves each hour's change set and the display frame takes them, held
/// for the microseconds of a push and a drain and never for a publication's making: the frame's
/// own write waits for nothing the side does ([`Installer`]).
#[derive(Default)]
struct Handover(Mutex<VecDeque<Set>>);

impl Handover {
    /// Leaves `set` for the frame. Past [`SETS_HELD`] the oldest two are folded into one dated at
    /// the newer hour, so nothing is dropped, no change is shown before its hour, and the hand-over
    /// stays bounded however long the frames take to write it.
    fn push(&self, set: Set) {
        let mut held = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        held.push_back(set);
        if held.len() > SETS_HELD
            && let Some(newer) = held.remove(1)
        {
            held[0].fold(newer);
        }
    }

    /// Everything left for the frame, taken at once.
    fn take(&self) -> VecDeque<Set> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl Set {
    /// Nothing of the fill or its borders changed in the hour it dates.
    fn nothing(&self) -> bool {
        self.whole.is_none() && self.provinces.is_empty() && self.keys.is_empty() && self.naval.is_empty() && self.plans.is_none()
    }

    /// Folds `newer` into this set: what a frame writing both would be left with, since a province
    /// that moved twice carries the newer hour's key. A whole fill voids every change before it.
    fn fold(&mut self, newer: Self) {
        let Self { hour, from: _, composed, whole, provinces, keys, naval, plans, built_ns, digest } = newer;
        if whole.is_some() {
            (self.provinces, self.keys, self.naval) = (Vec::new(), Vec::new(), Vec::new());
            self.whole = whole;
        }
        // The newer write of an entity stands, and the older goes: a folded set holds one write
        // per province, key and sea region however many hours it covers.
        newest_of(&mut self.provinces, provinces);
        newest_of(&mut self.keys, keys);
        newest_of(&mut self.naval, naval);
        (self.hour, self.composed, self.digest) = (hour, composed, digest);
        self.plans = plans.or(self.plans.take());
        self.built_ns += built_ns;
    }
}

/// `held` with `newer` after it, keeping each entity's last write alone.
fn newest_of<V>(held: &mut Vec<(u32, V)>, newer: Vec<(u32, V)>) {
    held.extend(newer);
    let mut seen = HashSet::with_capacity(held.len());
    held.reverse();
    held.retain(|(entity, _)| seen.insert(*entity));
    held.reverse();
}

/// The whole fill at one hour: the regions the scene is shown, and the arrays the installer's own
/// picture of what is on the GPU starts again from.
struct Whole {
    regions: Arc<Regions>,
    keys_of: Vec<u32>,
    colours: Vec<Option<[u8; 3]>>,
    naval: BTreeMap<u32, Attributes>,
}

/// The political fill and borders on the GPU, written by the display frame from the change sets
/// the side's thread hands over ([`Set`]): the frame's own side of the handover, so a frame writes
/// the hours its head has reached without taking the side at all.
pub struct Installer {
    /// What the side has handed over and no frame has written yet, oldest first.
    sets: VecDeque<Set>,
    taken: Arc<Handover>,
    /// The fill on the GPU: per province the key written and per key the colour, as of [`Self::at`].
    /// A set is written from these, so a province written at its own hour carries that hour's key
    /// and never a later one's.
    keys_of: Vec<u32>,
    colours: Vec<Option<[u8; 3]>>,
    /// The sea regions' naval attributes on the GPU.
    naval: BTreeMap<u32, Attributes>,
    /// The hour the fill and borders on the GPU stand at, and the same for the side's thread to
    /// read ([`Layer::shown_through`]).
    at: Option<Hour>,
    /// The newest hour the side has dated a change set for, and the picture it had composed when
    /// it did: between the newest set written and this hour there is no change of the fill to
    /// come, so a frame whose head is there shows that head's own picture.
    dated: Option<Hour>,
    composed: u64,
    shown: Arc<Shown>,
    /// The layer is asked for the whole fill ([`Political::asks_whole`]).
    asks_whole: Arc<AtomicBool>,
    /// The fill refused a write of what changed (its key table is full, or another view shares
    /// it): the sets wait for the whole fill rather than being written over a refused fill, and
    /// the hour on screen stands until it is written.
    refused: bool,
    /// The layer draws ([`Political::live`]): an installer of a layer in another map mode writes
    /// nothing, holds nothing and says no hour.
    live: Arc<AtomicBool>,
    /// The layer's own picture ([`Political::generation`]).
    generation: Arc<AtomicU64>,
    /// The border plans it wrote last, the hour of the set they came with for the side to read
    /// ([`Political::plans_shown`]), and whether something replaced them since.
    last_plans: Option<(ClassPlan, ZonePlan)>,
    plans_shown: Arc<Shown>,
    /// A set whose hour the last frame's head had reached was left unwritten.
    owed: bool,
    /// Installs taken: the fill whole, the country fill by what changed, the naval attributes by
    /// what changed; the harness reports them, so a run shows which entry drew its frames.
    uploads: [u64; 3],
    wrote: Wrote,
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
    /// Per province: the cells it was last painted from. An hour's own change set names only the
    /// cells that hour wrote, so the hours are painted from these carried forward.
    recorded: Vec<Option<Row>>,
    /// Per hour of the publication that wrote a province's cells, oldest first: the rows it wrote,
    /// with their cells as that hour left them ([`Political::read_hours`]).
    hour_rows: Vec<(Hour, Vec<(u32, Row)>)>,
    /// Per hour of the publication that painted a province anew: the provinces with the fill key
    /// that hour gave them, which become that hour's own change set for the display frame
    /// ([`Set`]).
    hour_sets: Vec<(Hour, Vec<(u32, u32)>)>,
    /// Per hour of the publication that changed an edge's class: the border plans as that hour
    /// left them, written with that hour's change set so its fill and borders are one hour's.
    hour_plans: Vec<(Hour, (ClassPlan, ZonePlan))>,
    /// Per hour of the publication that wrote a country's appearance: the country, the channel
    /// (red, green, blue) and the value that hour wrote ([`Political::date_colours`]).
    appearance_hours: Vec<(Hour, Vec<(u32, usize, Option<u32>)>)>,
    /// The newest hour inside the publication a subject, autonomy or link row was written, or a
    /// subject relation ended: a colour those change is dated there.
    recoloured_at: Option<Hour>,
    /// The oldest such hour: the relations are read at the publication's end, so a change at one
    /// hour and another at a later one (one undone by the next) dates none of the hours between.
    recoloured_first: Option<Hour>,
    /// Per fill key coloured again since the publication's last set: its colour and the country it
    /// took it from before, so each hour of the publication shows the colour of that hour.
    key_before: BTreeMap<u32, (Option<[u8; 3]>, u32)>,
    /// The fill keys the publication's hours painted with: each is coloured whether or not a
    /// province holds it at the publication's end, since an hour inside it draws with it.
    interior_keys: BTreeSet<u32>,
    /// The newest hour inside the publication a state's demilitarised flag was written: a new
    /// zone is dated there ([`Political::paint_hours`]).
    zone_at: Option<Hour>,
    /// A new zone read at the publication's end, with the hour it is dated at, for the hours to
    /// apply in order with their provinces.
    pending_zone: Option<(Hour, HashSet<u32>)>,
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
    /// Whether the border plans the side's thread built are still to be written.
    prepared_plans: bool,
    /// Nanoseconds the side's thread spent building the plans it last prepared.
    built_ns: u64,
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
    /// Where each hour's change set is handed to the display frame's own [`Installer`], taken once
    /// as the side goes to its thread ([`Layer::installs`]); none for a layer whose installer was
    /// never taken (a test, the proof tool), which dates nothing and keeps no set.
    sets: Option<Arc<Handover>>,
    /// The hour the installer has written, which the side's thread reads.
    shown: Arc<Shown>,
    /// The installer's fill refused a write of what changed: the next set is the whole fill.
    asks_whole: Arc<AtomicBool>,
    /// The layer draws: false in another map mode, where its installer writes nothing and says
    /// no hour ([`Installer::live`]).
    live: Arc<AtomicBool>,
    /// The picture the layer composed, counted: a set dated over an older one is void. Its own,
    /// so another layer's compose voids none of its sets.
    generation: Arc<AtomicU64>,
    /// The hour of the set whose border plans the installer wrote last ([`Installer::plans_shown`]).
    plans_shown: Arc<Shown>,
    /// The oldest hour of the publication whose change the next whole fill stands in for: the
    /// hours a refusal or a whole fill voided before they were dated ([`Set::from`]).
    voided_from: Option<Hour>,
    /// The oldest hour inside the publication the hours cannot show as it was (a province row
    /// refused, several zone flags, a border no patch reaches, a colour that cannot be followed):
    /// no set is dated from it on, and the publication's own set stands from it ([`Set::from`]).
    undated_from: Option<Hour>,
    /// The oldest hour inside the publication a state's demilitarised flag was written or its
    /// row born or gone.
    zone_first: Option<Hour>,
    /// The oldest hour inside the publication a country link was written: a province painted at
    /// an earlier hour read the links as the publication leaves them ([`Political::paint_hours`]).
    links_from: Option<Hour>,
    /// The publication moved appearance rows between slots (a row born or gone): an hourly write
    /// read against the publication's last slots may name another country.
    appearance_reslotted: bool,
    /// A stamp taken anew whenever the fill's arrays, colours, zone or refusal change: what a copy
    /// of the picture is kept by ([`crate::highlight::Picture`]).
    changed: u64,
    /// The hour it last dated a change set for.
    dated: Option<Hour>,
    /// The provinces its settles painted and the fill keys they recoloured since the frames last
    /// asked, for the count of what changed in view ([`Layer::scope`]).
    scoped: Scoped,
    /// Whether the fill on the GPU is the refused (empty) one.
    shown_refused: bool,
    /// The sea regions' naval attributes the fill carries, from today's political part until they
    /// are a layer of their own.
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

    /// `at` is an hour the publication's sets cannot show as it was.
    fn undated(&mut self, at: Hour) {
        self.undated_from = Some(self.undated_from.map_or(at, |held| held.min(at)));
    }

    /// Another map mode draws, or this one is refused: the installer writes nothing and says no
    /// hour, and every set handed over before is void.
    fn stand_down(&mut self) {
        self.live.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// The hour of the set whose border plans the frames wrote last: a picture copied before it
    /// stands on other borders than the screen's.
    pub fn plans_shown(&self) -> Option<Hour> {
        self.plans_shown.get()
    }

    /// The stamp of the picture its arrays hold now: a new one once anything of it changed.
    pub fn changed(&self) -> u64 {
        self.changed
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

    /// One province's fill key, owner key and state from the mirrors, at the hour they stand at.
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

    /// One province's fill key, owner key and state from the cells of one hour, or its error;
    /// marks what its change touches, and keeps the cells for the hours that write only some of
    /// them ([`Political::recorded`]).
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

    /// One fill key's colour after the subject rule, or its error.
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

    /// Recomputes what is dirty within `settling`: the relations and the zone whole, then provinces,
    /// fill keys and edges, the rest carried.
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
        // A fill written whole stands at the publication's own hour and voids every hour before
        // it: there is nothing for the hours to date.
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
            // The zone is read at the publication's end: one flag hour dates it, several do not.
            if let (Some(first), Some(last)) = (self.zone_first, self.zone_at)
                && first < last
            {
                self.undated(first);
            }
            if zone != self.demilitarized {
                self.pending_zone = Some((self.zone_at.take().unwrap_or(self.hour), zone));
            }
        }
        // The hours of the publication first, each dated as it is painted, then everything else
        // the publication marked (a link, a country, a province born or gone), at its own hour:
        // a province painted twice keeps the publication's own value, as the mirrors hold it.
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
            self.stand_down();
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("The political layer is refused: scripted map mode {} keeps its borders from one refresh to the next, which no layer follows yet", view.map_mode.token()),
            ));
        }
        if view.map_mode != MapMode::default() {
            // The display took this layer's installer once, at the side's start: the handover it
            // reads stays, and the fill it is given when the default mode returns is whole.
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
        // The hour a province changed hands is in the rows of that hour and nowhere else: the fill
        // is dated from them, one change set a game hour, while the side steps once a publication.
        inputs.hourly(TableId::Provinces)?;
        // So are the colours: a country's appearance by the hour it was written, and a subject,
        // autonomy or link change by the hour it happened.
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
            // A core table whose status moved inside the publication (unavailable, then back) is a
            // stretch of hours a direct compose would refuse: no hour from the one it moved at is dated.
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
        let digest = viewer_foundation::map::hour_check::on().then(|| digest(&self.country_of, &self.colours));
        // A whole fill carries the borders as the end leaves them whether or not they changed: one
        // that answers a refused write replaces borders the frame never wrote.
        let plans = (std::mem::take(&mut self.prepared_plans) || self.fill_changed || self.asks_whole.load(Ordering::Acquire))
            .then(|| self.classes.clone().zip(self.zone_plan.clone()))
            .flatten();
        // A fill that must be written whole stands at this hour, and every set handed over before
        // it is void: nothing older than a whole picture is worth writing. The regions and the
        // arrays the installer starts its own picture of the GPU again from are made here, on the
        // side's thread.
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
        // The hours of the publication, oldest first: each the provinces it repainted with the keys
        // that hour gave them, and the keys it coloured again, every key with the colour it had at
        // that hour. A whole fill voids them, so they go only before a set that is not whole.
        let hours = std::mem::take(&mut self.hour_sets);
        let hour_plans = std::mem::take(&mut self.hour_plans);
        let (before, appearance, recoloured_at) = (std::mem::take(&mut self.key_before), std::mem::take(&mut self.appearance_hours), self.recoloured_at.take());
        self.interior_keys.clear();
        let reslotted = std::mem::take(&mut self.appearance_reslotted);
        (self.zone_at, self.zone_first, self.links_from) = (None, None, None);
        // #8: appearance rows that moved between slots inside the publication leave its hourly
        // writes naming slots as the publication leaves them, which may be another country's.
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
                // From the undated hour on, what the hours changed goes with the publication's own
                // set, as the publication leaves it.
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
        // A step that changed nothing of it still carries its hour: the hour the layer shows
        // advances with the frames' heads whether or not there is anything to write.
        // A whole fill that stands in for hours it could not date stands from the oldest of them,
        // so no frame says an hour between was written ([`Set::from`]).
        let from = match whole.is_some() {
            true => self.voided_from.take().map_or(hour, |voided| voided.min(hour)),
            false => self.undated_from.take().map_or(hour, |undated| undated.min(hour)),
        };
        let set = Set { hour, from, composed, whole, provinces, keys, naval, plans, built_ns: std::mem::take(&mut self.built_ns), digest };
        // The same hour settled again with nothing of its own to say is not dated again: a whole
        // settle runs many times over one hour while a stepped frame catches up to it.
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
        // Whatever stands on the GPU now is not this installer's: its first set is the whole fill.
        self.fill_changed = true;
        let mut installer = Installer::new(taken, Arc::clone(&self.shown), Arc::clone(&self.asks_whole), Arc::clone(&self.live), Arc::clone(&self.generation));
        installer.plans_shown = Arc::clone(&self.plans_shown);
        Some(Box::new(installer))
    }

    fn shown_through(&self) -> Option<Hour> {
        self.shown.get().filter(|_| self.columns.is_some())
    }

    fn uploaded(&self) -> bool {
        // Undeclared, it draws nothing and owes the scene nothing. Declared, what it settled is
        // the installer's the moment it is dated, so it owes the scene only plans it has not
        // dated yet; what the frames owe is the installer's own ([`Installs::owes`]).
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
    /// The frame's side of the handover: nothing written yet, every set still to come.
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

    /// Everything the side has handed over since the last frame, taken without waiting for it: a
    /// picture the layer composed since voids the sets of the hours it replaced ([`Set::composed`]).
    /// The layer's count is read after the drain, so no set taken is newer than it.
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
            // Frames that write nothing (a head held behind) leave the sets here: the oldest are
            // folded as the hand-over folds them, so neither grows however long that lasts.
            while self.sets.len() > SETS_HELD
                && let Some(newer) = self.sets.remove(1)
            {
                self.sets[0].fold(newer);
            }
        }
        self.dated = self.sets.back().map(|set| set.hour).or(self.dated);
        built_ns
    }

    /// The hour of the oldest set a frame whose head stands at `through` must write; none when
    /// every hour it has reached is written. No head (a picture that must be whole) reaches
    /// every set.
    fn due(&self, through: Option<Hour>) -> Option<Hour> {
        let front = self.sets.front()?;
        through.is_none_or(|head| front.hour <= head).then_some(front.hour)
    }

    /// The sets a frame whose head stands at `through` writes, folded into its own picture of the
    /// GPU and out of the queue: no device is touched, so what a frame writes is decided by the
    /// hours alone and a test needs none ([`Self::install`] writes it).
    fn pick(&mut self, through: Option<Hour>) -> Picked {
        let mut picked = Picked::default();
        picked.built_ns = self.take();
        // The sets of the hours `through` has reached, in hour order and coalesced: a province
        // that moved twice carries the key of the newest hour written, and the newest plans among
        // them stand. A whole fill stands at its own hour too and voids what was written before
        // it. What is left is for the frames that reach it. A fill that refused a write of what
        // changed takes nothing but a whole one: a write over a refused fill would show neither.
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
            // The digest of the last set written, and none while that is an hour inside a
            // publication: the fill is settled whole at a publication's own hour, where every one
            // of its hours has been written, and between them it is the hours' own picture, which
            // no digest of the layer's arrays stands for.
            picked.digest_of = set.digest.map(|digest| (set.hour, digest));
            // A whole fill first, then any writes folded after it in the same set.
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

    /// `BATTLE_PLAN_VIEWER_CHECK_HOUR`: the fill on screen stands at the hour it was settled for, where
    /// the frame wrote a publication's own hour whole, and no change set the frame's head has
    /// reached is left unwritten.
    fn judge_hour(&self, written: Option<(Hour, u64)>, through: Option<Hour>) {
        use viewer_foundation::map::hour_check as check;
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

/// Where an installer writes: the scene, or a test's stand-in that refuses what it is told to.
trait Sink {
    fn set_political(&mut self, regions: &Arc<Regions>) -> io::Result<()>;
    fn update_naval(&mut self, attributes: &dyn Fn(u32) -> Attributes, regions: &[u32]) -> io::Result<bool>;
    fn update_political(&mut self, keys_of: &[u32], colour: &dyn Fn(u32) -> [u8; 3], provinces: &[usize], keys: &[u32]) -> io::Result<bool>;
    fn set_border_plans(&mut self, classes: &ClassPlan, zone: &ZonePlan) -> io::Result<()>;
}

/// The frame's scene as an installer's [`Sink`].
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

/// What one frame writes of the sets handed over: the whole fill where one arrived, the provinces
/// and fill keys whose values changed in the hours it reached, the sea regions among them, the
/// newest border plans, the hour it ends at and the digest the hour judge compares.
#[derive(Default)]
struct Picked {
    whole: Option<Whole>,
    provinces: Vec<usize>,
    keys: Vec<u32>,
    naval: Vec<u32>,
    plans: Option<(ClassPlan, ZonePlan)>,
    hour: Option<Hour>,
    digest_of: Option<(Hour, u64)>,
    /// What the side's thread spent building the plans in those sets, for the run's record.
    built_ns: u64,
}

impl Installer {
    /// What `install` writes of `picked`, through `sink`: the whole fill, the naval attributes and
    /// the country fill, then the border plans unless a write was refused, and the hour the picture
    /// stands at. The digest of the hour written, for the hour judge.
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
                // The fill's key table is full: the side is asked for the whole fill, and nothing
                // of this hour is on screen until it arrives.
                self.refused = true;
                self.asks_whole.store(true, Ordering::Release);
                (hour, digest_of) = (None, None);
            }
        }
        lap(phases, 1);
        // The border plans a set carried at its own hour; one the frames have not reached stands
        // with its own set, so the fill and its borders are never a different hour's. A fill that
        // refused its write keeps the borders it has until the whole fill comes.
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
        // The fill stands at the frame's own head once every set whose hour it reached is written:
        // the side has dated every hour through `self.dated`, so between the newest set written
        // and the head there is no change of the fill to come, and the picture is that head's.
        if !self.owed
            && let (Some(head), Some(dated)) = (through, self.dated)
        {
            // A set folded over hours the head is inside holds a change of an hour it reached.
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
            // Another map mode draws: what this installer holds is of a picture no frame shows. The
            // hand-over is left alone: the layer's own count voids its sets, and a whole fill the
            // default mode publishes before this frame reads it is never lost to it.
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
        if viewer_foundation::map::hour_check::on() {
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
    /// The border plans a frame writes, bucketed here on the side's thread: no device is touched,
    /// so the frame's upload only hands the scene a plan already laid out. A later settle patches
    /// the plan it left, or lays it out again when an edge outside the chunks changed class.
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

    /// The hours inside `step` at which a colour's sources changed: the appearance cells each hour
    /// wrote, and the newest hour a subject, autonomy or link row was written. Rows born or gone
    /// inside the publication are left out, as the provinces' are: their colour stands at the
    /// publication's own hour.
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

    /// A key an inner hour painted with that the publication's end holds nothing of, whose colour
    /// was refused or has none recorded, is no picture those hours can show: they are not dated,
    /// and its diagnostics are theirs, not the end's, so they leave before the end's plans and fill
    /// are made.
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

    /// The publication's hours not dated yet, let go for the whole fill that stands at its own hour:
    /// the oldest of them is kept as the hour that fill stands in for ([`Self::voided_from`]).
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

    /// The hours of the publication voided with its sets: a whole fill stands at its own hour.
    fn void_colour_hours(&mut self) {
        self.appearance_hours.clear();
        (self.recoloured_at, self.recoloured_first) = (None, None);
        self.key_before.clear();
        self.interior_keys.clear();
        (self.zone_at, self.zone_first) = (None, None);
        (self.links_from, self.appearance_reslotted) = (None, false);
    }

    /// Per fill key coloured again in the publication, the colours it took inside it, by hour: its
    /// country's appearance as each hour wrote it, from the hour its colour source changed when a
    /// subject, autonomy or link change moved it. A key with none takes its new colour at the
    /// publication's own hour; until its first, every hour shows the colour it had before.
    ///
    /// The hour returned is the oldest it cannot follow: a source it starts with no channels of
    /// (a grey fallback is no colour), or a former source written to before the change.
    #[allow(clippy::type_complexity, reason = "per key its dated colours, and the hour they stop being followed")]
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
        // A country's channels before the publication, where a key took its colour from it then.
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
            // Where the key's colour is followed from and the channels it starts with: its own
            // colour for an unchanged source; for a new one, from the hour the source changed,
            // that country's channels before the publication, else as the publication leaves them.
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

    /// The publication's hours through `last`, oldest first: each the provinces it repainted with
    /// their keys, and every key it uses or coloured again with the colour that key had at that
    /// hour ([`Self::colour_hours`]).
    /// The hour returned is the oldest whose colours cannot be followed ([`Self::colour_hours`]).
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

    /// Each hour of the publication that wrote a province's state, owner or controller: the rows
    /// it wrote with their cells as that hour left them, carried forward from the cells the layer
    /// last read, since an hour's change set names only what that hour wrote. Rows born or gone
    /// inside the publication are left out: a slot they left may be another row's by its end, so
    /// they are painted from the mirrors at the publication's own hour, as every other mark is.
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
                        // A cell the hour left missing or unrecorded is no value: the province is
                        // painted from it, and refused by name, exactly as the mirrors' would be.
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

    /// Every hour of the publication painted in turn, oldest first, each leaving the provinces it
    /// repainted as that hour's own change set: what a display frame whose head has reached that
    /// hour writes, and nothing of a later one.
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
        // A link written inside the publication repaints the provinces already on it at the
        // publication's own hour: the hours from the write on are not dated.
        if let Some(links) = self.links_from {
            self.undated(links);
        }
    }

    /// The new zone, at the hour its flag was written: its edges classified and the plans of that
    /// hour made, so the hours before it keep the zone they had.
    fn apply_zone(&mut self) {
        let Some((at, zone)) = self.pending_zone.take() else { return };
        self.rezone(zone);
        if at < self.hour {
            self.hour_plans_at(at);
        }
    }

    /// The border plans as `hour` leaves its edges, patched over the plans held, when it changed an
    /// edge's class. A change the patch cannot make (plans not built yet, an edge with no chunk)
    /// is left to the publication's own plans.
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
        // An edge outside the patchable chunks changed class: the plans of this hour are not made
        // here, so the hours from it on are not shown before the publication's own.
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

    fn clear(&mut self) {
        self.take();
    }

    fn bytes(&self) -> usize {
        self.bits.capacity() * 8 + self.list.capacity() * 4
    }
}
