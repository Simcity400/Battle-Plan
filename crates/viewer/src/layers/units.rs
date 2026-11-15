//! The land unit models from the mirrors: the divisions grouped into stacks by province, owner slot
//! and the route's next two hops, as today's model layer groups them; per stack its oldest member,
//! what the model selection reads of it, and its figure placed as today places it
//! ([`unit_models::land_figure`]) but for its mesh. In the window each admitted figure's mesh is
//! chosen on the layer's own worker over the rows at the head, on a cadence, handed the stacks a
//! settle touched ([`meshes`]): the selection joins templates, variants, equipment, the census and
//! trigger-evaluated graphics limits over any table, which needs a frame of rows the frame thread
//! does not hold. Today's unit models, without their land stacks, give the ships, raid units and
//! convoys ([`models`]).
//!
//! Which divisions the viewer sees is the vision's (P6's layer): the window's layer reads each
//! division's sight from the vision layer registered before it and reads again only the divisions
//! its changes name; elsewhere a vision is given whole ([`Units::set_vision`]). A stack no member of
//! which is seen has no figure, as today. Without a vision the layer is refused by name.

pub(crate) mod meshes;
pub(crate) mod models;

use super::{
    Budget, Carried, Dirty, Gpu, Inputs, Layer, List, Num, Published, Upstream, View,
    counters::{Row, Rows},
    vision::Vision as VisionLayer,
};
use crate::playhead_host::mirror::{Cell, Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_foundation::map::{
    camera::{MapDefines, MapSize, Placement},
    counter_move::{DivisionRoute, Position, RouteWorld, UnitRoute},
    province_admission::Admission,
    perspective::{Choice, Sight, Vision},
    province_map::HeightField,
    unit_rules::STANDSTILL,
};
use viewer_world::map::{
    entity_animation::{ATTACK, DEFEND},
    map_mode::visibility,
    unit_history::Target,
    unit_models::{self, LandStack, Unplaced},
};

const NONE: u32 = u32::MAX;

/// A multiply-rotate hasher for the layer's integer keys: the default SipHash is several times
/// slower and its resistance buys nothing for keys the recording gives.
#[derive(Clone, Copy, Default)]
struct Fast(u64);

impl Fast {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl std::hash::Hasher for Fast {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
}

type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fast>>;

/// What places a figure on the map: the provinces' unit anchors, the route geometry (none without
/// the content's movement map) and the transport scale.
pub(crate) trait Ground: Send + Sync {
    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]>;
    fn world(&self) -> Option<RouteWorld<'_>>;
    fn transport_scale(&self) -> f32;
    /// Each province's box that the camera's admission tests.
    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]>;
    /// The camera height at and above which no land unit model is drawn.
    fn units_cutoff(&self) -> f32;
    /// The map a camera placement is put on.
    fn map(&self) -> (MapSize, MapDefines);
}

impl Ground for crate::document::Content {
    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]> {
        self.counters.anchor(province, slot)
    }

    fn world(&self) -> Option<RouteWorld<'_>> {
        let heights = HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        self.movement.as_deref().map(|movement| RouteWorld { map: &movement.map, heights, width: self.map.width as f32 })
    }

    fn transport_scale(&self) -> f32 {
        self.unit_models.gates.transport_scale
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        &self.unit_models.bounds
    }

    fn units_cutoff(&self) -> f32 {
        self.unit_models.gates.units
    }

    fn map(&self) -> (MapSize, MapDefines) {
        (MapSize::of(&self.map), self.map_defines)
    }
}

/// What a stack is keyed by: its province, its owner's canonical slot and its route's next two
/// hops, the triple a counter groups by.
pub(crate) type StackKey = (u32, u32, Option<[u32; 2]>);

/// What today's model selection reads of a stack: the key it keeps its answers by, and whether the
/// oldest member's status is recorded (a land figure without one has no model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub transport: bool,
    pub template: u64,
    pub oldest: u64,
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
    pub status: bool,
}

/// Why a stack has no figure though the selection may choose its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Withheld {
    Unplaced(Unplaced),
    /// A land stack whose oldest seen member's status is not recorded this hour.
    Status,
    /// No member of the stack is seen by the viewer.
    Unseen,
}

/// What places a figure: the oldest member's route, combat side, exercise and texture motion, and
/// the scale.
#[derive(Clone, Debug)]
struct Placing {
    province: u32,
    owner: u32,
    oldest: u64,
    route: Option<DivisionRoute>,
    combat: Option<u32>,
    training: Option<bool>,
    uv_moving: bool,
    scale: f32,
}

/// A placing but for its route.
#[derive(Clone, Copy, Debug)]
struct Scalars {
    province: u32,
    owner: u32,
    oldest: u64,
    combat: Option<u32>,
    training: Option<bool>,
    uv_moving: bool,
    scale: f32,
}

impl Scalars {
    fn with(self, route: Option<DivisionRoute>) -> Placing {
        let Self { province, owner, oldest, combat, training, uv_moving, scale } = self;
        Placing { province, owner, oldest, route, combat, training, uv_moving, scale }
    }
}

impl Placing {
    fn same(&self, other: &Scalars, route: Option<&DivisionRoute>) -> bool {
        let routes = match (&self.route, route) {
            (None, None) => true,
            (Some(a), Some(b)) => same_route(a, b),
            _ => false,
        };
        routes
            && (self.province, self.owner, self.oldest, self.combat, self.training, self.uv_moving, self.scale.to_bits())
                == (other.province, other.owner, other.oldest, other.combat, other.training, other.uv_moving, other.scale.to_bits())
    }
}

fn same_route(a: &DivisionRoute, b: &DivisionRoute) -> bool {
    (&a.path, a.eta, a.naval_transfer, a.retreat, a.combat_role, a.combat_count, a.stack_kind, a.kind_resolved, a.idle)
        == (&b.path, b.eta, b.naval_transfer, b.retreat, b.combat_role, b.combat_count, b.stack_kind, b.kind_resolved, b.idle)
}

/// One stack: what the selection reads, whether the camera admits it, and the target as the field
/// draws it (mesh 0) with the position its animator starts from, or why it has none; `placed` is
/// none while the figure waits to be made, which only an admitted stack's is, and meanwhile it is
/// drawn and animated where it was last placed (`last`), never left out of a frame.
#[derive(Clone, Debug)]
pub(crate) struct Figure {
    pub selection: Selection,
    pub admitted: bool,
    pub placed: Option<Result<(Target, Position), Withheld>>,
    last: Option<(Target, Position)>,
    placing: Placing,
}

impl Figure {
    /// Where it is drawn: its placing, or while it waits to be placed again, its last.
    fn drawn(&self) -> Option<&(Target, Position)> {
        match &self.placed {
            Some(Ok(placed)) => Some(placed),
            Some(Err(_)) => None,
            None => self.last.as_ref(),
        }
    }
}

#[derive(Debug)]
struct Stack {
    key: StackKey,
    /// Its divisions' slots in Divisions.
    members: Vec<u32>,
    admitted: bool,
    figure: Option<Figure>,
    /// The oldest member its figure asked a mesh for.
    asked: Option<u64>,
}

/// One Divisions slot: its stack and place in it, its id, whether the viewer sees it, and whether
/// its own values changed since its stack last read them.
#[derive(Clone, Debug)]
struct Member {
    stack: u32,
    at: u32,
    id: u64,
    /// The province and owner slot its sight was read at, and the province whose byte it read
    /// (`NONE` while it is in no stack).
    sighted: Option<(u32, u32)>,
    read_at: u32,
    seen: bool,
    stale: bool,
}

impl Default for Member {
    fn default() -> Self {
        Self { stack: NONE, at: 0, id: 0, sighted: None, read_at: NONE, seen: false, stale: true }
    }
}

/// Where a division's sight is read: the vision layer's arrays in place, or a vision given whole.
#[derive(Clone, Copy)]
enum Seer<'a> {
    Layer(&'a VisionLayer),
    Given(&'a Vision),
}

impl Seer<'_> {
    /// Whether the viewer sees any of a land unit standing in `province`, and the province whose
    /// byte that read.
    fn sees(self, unit: u64, owner: u32, province: u32) -> (bool, u32) {
        match self {
            Seer::Layer(vision) => (vision.sees_land(unit, owner, province).is_some_and(|sight| sight != Sight::None), vision.sight_province(unit, province)),
            Seer::Given(vision) => (vision.sees_land(unit, owner, province) != Sight::None, province),
        }
    }
}

/// What the window's figures are drawn with besides the layer's own: the selection and the rows its
/// meshes and models are made from, and the tables each learned it reads, kept while the view draws
/// no land figure.
struct Window {
    select: Arc<dyn models::Select>,
    rows: Arc<crate::playhead_host::rows::Rows>,
    tables: Vec<TableId>,
    meshes: Vec<TableId>,
}

#[derive(Clone, Copy)]
struct Columns {
    province: Num<u32>,
    owner: Num<u32>,
    template: Num<u64>,
    roster: Num<u32>,
    expeditionary: Num<u32>,
    target: Num<u32>,
    retreat: Num<u8>,
    naval_transfer: Num<u8>,
    combat_count: Num<u32>,
    path_total: Num<u32>,
    path: List,
    alternate_total: Num<u32>,
    alternate_path: List,
    combat_role: Num<u8>,
    combat_province: Num<u32>,
    alternate_template: Num<u64>,
    exercising: Num<u8>,
    attackers: List,
    defenders: List,
    link_slot: Num<u32>,
}

/// The rows a table holds at the head: none when unavailable, not recorded or in a gap.
fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn decode<const N: usize>(bytes: &[u8], out: &mut Vec<u64>) {
    out.clear();
    out.extend(bytes.chunks_exact(N).map(|item| {
        let mut word = [0; 8];
        word[..N].copy_from_slice(item);
        u64::from_le_bytes(word)
    }));
}

fn path_of(mirror: &Mirror, column: List, slot: u32, out: &mut Vec<u32>) {
    out.clear();
    if let Cell::Value(bytes) = mirror.list(column, slot) {
        out.extend(bytes.chunks_exact(4).map(|item| u32::from_le_bytes([item[0], item[1], item[2], item[3]])));
    }
}

/// The land unit models' arrays and what keeps them.
/// The tables the layer reads while it draws.
const TABLES: [TableId; 6] = [TableId::Divisions, TableId::DivisionMovement, TableId::ArmyIconInputs, TableId::DivisionStatus, TableId::LandCombats, TableId::CountryLinks];

pub(crate) struct Units {
    ground: Arc<dyn Ground>,
    /// In the window: what its models are made from, and while the view draws land figures the
    /// meshes, ships, raid units and convoys it draws them with.
    window: Option<Window>,
    models: Option<models::Models>,
    /// In the window, while the view draws land figures: each admitted figure's mesh, chosen on the
    /// layer's own worker; the stacks it was handed, by oldest member with the stack that asked;
    /// the stacks whose figure a settle touched, and the asks not sent yet.
    meshes: Option<meshes::Meshes>,
    asking: FastMap<u64, (u32, meshes::Asked)>,
    touched: Vec<u32>,
    asks: Vec<meshes::Ask>,
    /// Figures or models changed since the field was last installed.
    to_upload: bool,
    /// The models changed since the field was last installed; and the land figures it was
    /// installed with, so that a settle that placed the same figures installs nothing.
    models_new: bool,
    uploaded: Vec<Target>,
    /// It reads the vision layer registered before it; otherwise a vision is given whole.
    reading: bool,
    /// The vision given and not yet taken, flagged when new.
    given: Mutex<Option<(Choice, Arc<Vision>)>>,
    fresh_vision: AtomicBool,
    /// The vision the arrays are settled with, when given whole.
    vision: Option<(Choice, Arc<Vision>)>,
    /// Reading the vision layer: its perspective and sight epoch as last taken.
    choice: Option<Choice>,
    sight_epoch: Option<u64>,
    /// Divisions whose sight is read again; per province whose byte a sight read, the divisions
    /// that read it; and the sights read again since the layer was made.
    sights: Dirty,
    readers: FastMap<u32, Vec<u32>>,
    sighted: usize,
    columns: Option<Columns>,
    /// Why nothing is drawn: a refused input or a core table not recorded.
    refused: Option<String>,
    members: Vec<Member>,
    stacks: Vec<Stack>,
    free: Vec<u32>,
    by_key: FastMap<StackKey, u32>,
    /// Each division's combat side, from LandCombats in key order, defenders over attackers.
    combat: FastMap<u64, u32>,
    links: FastMap<u32, u32>,
    divisions: Dirty,
    /// Divisions whose row, route or owner a step changed: a settle re-keys those it admits.
    moved: Dirty,
    /// Rows the steps changed in DivisionMovement, ArmyIconInputs and DivisionStatus, resolved by a
    /// settle to the divisions they name; and every division to be placed again.
    route_rows: Rows,
    icon_rows: Rows,
    status_rows: Rows,
    every: bool,
    dirty_stacks: Dirty,
    /// Admitted stacks whose figure waits to be made, the costly part.
    figures: Dirty,
    /// The camera and viewport the stacks are admitted at; none, or a viewport not given yet,
    /// admits every stack.
    camera: Option<(Placement, [u32; 2])>,
    admission: Option<Admission>,
    camera_dirty: bool,
    /// Per province id at this camera: whether admission was asked, and its answer.
    province_known: Vec<u64>,
    province_in: Vec<u64>,
    /// Divisions changed while neither their old nor their new province was admitted, by slot,
    /// re-keyed once one is; and how many.
    deferred: Vec<u64>,
    deferred_count: usize,
    combat_dirty: bool,
    links_dirty: bool,
    /// Each read table's status as last seen, by table index.
    statuses: HashMap<usize, Status>,
    frame: u64,
    /// Divisions and stacks the steps have marked, and figures placed anew.
    marked: [usize; 3],
    path: Vec<u32>,
    alternate: Vec<u32>,
    items: Vec<u64>,
    /// Divisions placed, and rows its `apply` read, since the layer was made: none, by the contract.
    #[cfg(test)]
    placed: usize,
    #[cfg(test)]
    applied_reads: u64,
}

impl Units {
    pub(crate) fn new(ground: Arc<dyn Ground>) -> Self {
        Self {
            ground,
            window: None,
            models: None,
            meshes: None,
            asking: FastMap::default(),
            touched: Vec::new(),
            asks: Vec::new(),
            to_upload: false,
            models_new: false,
            uploaded: Vec::new(),
            reading: false,
            given: Mutex::new(None),
            fresh_vision: AtomicBool::new(false),
            vision: None,
            choice: None,
            sight_epoch: None,
            sights: Dirty::default(),
            readers: FastMap::default(),
            sighted: 0,
            columns: None,
            refused: Some("not composed yet".into()),
            members: Vec::new(),
            stacks: Vec::new(),
            free: Vec::new(),
            by_key: FastMap::default(),
            combat: FastMap::default(),
            links: FastMap::default(),
            divisions: Dirty::default(),
            moved: Dirty::default(),
            route_rows: Rows::default(),
            icon_rows: Rows::default(),
            status_rows: Rows::default(),
            every: false,
            dirty_stacks: Dirty::default(),
            figures: Dirty::default(),
            camera: None,
            admission: None,
            camera_dirty: false,
            province_known: Vec::new(),
            province_in: Vec::new(),
            deferred: Vec::new(),
            deferred_count: 0,
            combat_dirty: false,
            links_dirty: false,
            statuses: HashMap::new(),
            frame: 0,
            marked: [0; 3],
            path: Vec::new(),
            alternate: Vec::new(),
            items: Vec::new(),
            #[cfg(test)]
            placed: 0,
            #[cfg(test)]
            applied_reads: 0,
        }
    }

    /// Divisions placed, and rows `apply` read, since the layer was made.
    #[cfg(test)]
    pub(crate) fn placed(&self) -> usize {
        self.placed
    }

    #[cfg(test)]
    pub(crate) fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    /// A layer of the same content and inputs, made from scratch: one that reads the vision layer
    /// composes with it ([`Layer::compose_with`]).
    pub(crate) fn fresh(&self) -> Self {
        let fresh = Self { reading: self.reading, columns: self.columns, camera: self.camera, camera_dirty: true, ..Self::new(Arc::clone(&self.ground)) };
        if let Some((choice, vision)) = &self.vision {
            fresh.set_vision(choice.clone(), Arc::clone(vision));
        }
        fresh
    }

    /// What the viewer sees at the head and the perspective it is for; every division's sight is
    /// read again with it at the next settle. Not for a layer that reads the vision layer.
    pub(crate) fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        *self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((choice, vision));
        self.fresh_vision.store(true, Ordering::Release);
    }

    /// A layer that reads each division's sight from the vision layer registered before it, as the
    /// window's does: the proof tool's units line and the tests.
    pub(crate) fn reading_vision(ground: Arc<dyn Ground>) -> Self {
        Self { reading: true, ..Self::new(ground) }
    }

    /// Drawn in the window: its figures with the meshes it chooses on its own worker over `rows`,
    /// and the ships, raid units and convoys of today's unit models made over `rows`, its vision
    /// read from the vision layer before it.
    pub(crate) fn in_window(mut self, select: Arc<dyn models::Select>, rows: &Arc<crate::playhead_host::rows::Rows>) -> Self {
        // Each declares the tables it is known to read ([`models::tables`], [`meshes::TABLES`]) and learns the rest.
        self.window = Some(Window { select, rows: Arc::clone(rows), tables: Vec::new(), meshes: Vec::new() });
        self.reading = true;
        self
    }

    /// In the window, the unit models its field was made with: the frame poses their raid units
    /// and sailing convoys.
    pub(crate) fn prepared(&self) -> Option<&Arc<viewer_world::map::unit_models::PreparedModels>> {
        self.models.as_ref()?.prepared.as_ref().map(|(_, prepared)| prepared)
    }

    /// The unit models' omitted stacks by reason, worded as today's: the land figures withheld from
    /// the meshes it chose, then the ships' from the models its field was made with; loading by name
    /// while it draws the models and has made none.
    pub(crate) fn warnings(&self) -> Vec<String> {
        let mut lines = match (self.prepared(), self.refused()) {
            (Some(prepared), _) => prepared.report.warnings(),
            (None, Some(why)) => return vec![format!("Unit models: not drawn: {why}")],
            (None, None) if self.drawn() => return vec!["Unit models: loading at the playback head".to_owned()],
            (None, None) => return Vec::new(),
        };
        if let Some(meshes) = &self.meshes {
            let mut withheld: BTreeMap<String, usize> = BTreeMap::new();
            for why in meshes.withheld.values() {
                *withheld.entry(why.clone()).or_default() += 1;
            }
            let bounds = self.ground.bounds();
            for stack in self.stacks.iter().filter(|stack| stack.admitted && !bounds.contains_key(&stack.key.0)) {
                if stack.figure.as_ref().is_some_and(|figure| matches!(figure.placed, Some(Ok(_)))) {
                    *withheld.entry(format!("province {} has no content bounds for model admission", stack.key.0)).or_default() += 1;
                }
            }
            let mut land: Vec<String> = withheld.into_iter().map(|(why, count)| format!("Unit models: no model for {count} stacks: {why}.")).collect();
            land.extend(meshes.refused.as_ref().map(|why| format!("Unit models: land figures not drawn: {why}")));
            land.append(&mut lines);
            lines = land;
        }
        lines
    }

    /// The perspective of the vision the arrays are settled with.
    pub(crate) fn perspective(&self) -> Option<&Choice> {
        self.vision.as_ref().map(|(choice, _)| choice).or(self.choice.as_ref())
    }

    /// Divisions' sights read again since the layer was made, beside those its steps placed.
    #[cfg(test)]
    pub(crate) fn sighted(&self) -> usize {
        self.sighted
    }

    /// Whether the view draws land unit models, so the layer reads anything.
    pub(crate) fn drawn(&self) -> bool {
        self.columns.is_some()
    }

    /// Why the layer draws nothing, by name; none when it draws.
    pub(crate) fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// What the window's animators read of the figures: each admitted land figure placed at the
    /// applied hour ([`unit_models::land_movement`]), mesh or none yet, and the formations of its
    /// models.
    pub(crate) fn movement_positions(&self) -> impl Iterator<Item = (viewer_foundation::unit_handle::Unit, &Position, f32, f32)> {
        let land = self.stacks.iter().filter(|stack| stack.admitted).filter_map(|stack| stack.figure.as_ref()?.drawn()).map(|(target, position)| unit_models::land_movement(target, position));
        let ships = self.prepared().into_iter().flat_map(|prepared| prepared.ship_movement_positions());
        land.chain(ships)
    }

    /// Every stack with its figure, in key order.
    pub(crate) fn figures(&self) -> BTreeMap<StackKey, &Figure> {
        self.stacks.iter().filter_map(|stack| Some((stack.key, stack.figure.as_ref()?))).collect()
    }

    /// Stacks held, settled or not.
    pub(crate) fn stack_count(&self) -> usize {
        self.by_key.len()
    }

    /// The cell of `size` world units holding most stacks that move or fight: its centre and count.
    pub(crate) fn front(&self, size: f32) -> Option<([f32; 2], usize)> {
        let mut cells: HashMap<(i32, i32), usize> = HashMap::new();
        for stack in self.by_key.values().map(|&stack| &self.stacks[stack as usize]) {
            let Some(placing) = stack.figure.as_ref().map(|figure| &figure.placing) else { continue };
            if placing.combat.is_none() && placing.route.as_ref().is_none_or(|route| route.path.is_empty()) {
                continue;
            }
            let Some([x, _, z]) = self.ground.anchor(stack.key.0, STANDSTILL) else { continue };
            *cells.entry(((x / size).floor() as i32, (z / size).floor() as i32)).or_default() += 1;
        }
        cells.into_iter().max_by_key(|&(cell, count)| (count, std::cmp::Reverse(cell))).map(|((x, z), count)| ([(x as f32 + 0.5) * size, (z as f32 + 0.5) * size], count))
    }

    /// The camera height at and above which no land unit model is drawn.
    pub(crate) fn cutoff(&self) -> f32 {
        self.ground.units_cutoff()
    }

    /// Stacks the camera admits.
    pub(crate) fn admitted(&self) -> usize {
        self.by_key.values().filter(|&&stack| self.stacks[stack as usize].admitted).count()
    }

    /// Whether the camera admits `province`: only an admitted stack's figure is made and drawn.
    pub(crate) fn admits(&self, province: u32) -> bool {
        self.admission.as_ref().is_none_or(|admission| admission.admits_province(self.ground.bounds(), province))
    }

    /// Divisions whose re-keying waits for their province to be admitted.
    pub(crate) fn deferred(&self) -> usize {
        self.deferred_count
    }

    /// Whether `province` is admitted at this camera, asked once per province per camera.
    fn province_admitted(&mut self, province: u32) -> bool {
        if self.admission.is_none() {
            return true;
        }
        let (word, bit) = (province as usize / 64, 1u64 << (province % 64));
        if word >= self.province_known.len() {
            self.province_known.resize(word + 1, 0);
            self.province_in.resize(word + 1, 0);
        }
        if self.province_known[word] & bit == 0 {
            self.province_known[word] |= bit;
            if self.admits(province) {
                self.province_in[word] |= bit;
            }
        }
        self.province_in[word] & bit != 0
    }

    /// Whether division `slot` stands, or stood, in an admitted province.
    fn wanted(&mut self, slot: u32, divisions: Option<&Mirror>) -> bool {
        let old = self.members.get(slot as usize).filter(|member| member.stack != NONE).map(|member| self.stacks[member.stack as usize].key.0);
        let new = divisions.filter(|divisions| divisions.live(slot)).and_then(|divisions| divisions.number(self.columns().province, slot).value()).filter(|&province| province != 0);
        old.is_some_and(|province| self.province_admitted(province)) || new.is_some_and(|province| self.province_admitted(province))
    }

    /// A division's row, route or owner changed: re-keyed now if it is, or was, where the camera
    /// admits; otherwise one bit, taken when its province is admitted.
    fn mark_moved(&mut self, slot: u32, divisions: Option<&Mirror>) {
        if self.admission.is_none() || self.wanted(slot, divisions) {
            self.mark_division(slot);
            return;
        }
        self.defer(slot);
    }

    /// One bit for a division whose re-keying waits for its province to be admitted.
    fn defer(&mut self, slot: u32) {
        let (word, bit) = (slot as usize / 64, 1u64 << (slot % 64));
        if word >= self.deferred.len() {
            self.deferred.resize(word + 1, 0);
        }
        if self.deferred[word] & bit == 0 {
            self.deferred[word] |= bit;
            self.deferred_count += 1;
        }
    }

    /// Every stack admitted again at the camera; one newly admitted whose figure is not made is
    /// marked to make it, and a deferred division now in or out of an admitted province is marked
    /// to re-key.
    fn readmit(&mut self, mirrors: &Mirrors) {
        self.camera_dirty = false;
        let (size, defines) = self.ground.map();
        self.admission = self.camera.filter(|(_, viewport)| *viewport != [0, 0]).map(|(placement, viewport)| Admission::new(placement.on(size, defines), viewport));
        self.province_known.iter_mut().for_each(|word| *word = 0);
        self.province_in.iter_mut().for_each(|word| *word = 0);
        if self.deferred_count > 0 {
            let divisions = mirrors.get(TableId::Divisions);
            let mut deferred = std::mem::take(&mut self.deferred);
            for (at, word) in deferred.iter_mut().enumerate() {
                let mut left = *word;
                while left != 0 {
                    let bit = left.trailing_zeros();
                    left &= left - 1;
                    let slot = at as u32 * 64 + bit;
                    if self.wanted(slot, divisions) {
                        *word &= !(1 << bit);
                        self.deferred_count -= 1;
                        self.mark_division(slot);
                    }
                }
            }
            self.deferred = deferred;
        }
        let held: Vec<u32> = self.by_key.values().copied().collect();
        for stack in held {
            let admitted = self.admits(self.stacks[stack as usize].key.0);
            let at = stack as usize;
            if self.stacks[at].admitted != admitted {
                self.touched.push(stack);
            }
            self.stacks[at].admitted = admitted;
            if let Some(figure) = &mut self.stacks[at].figure {
                figure.admitted = admitted;
                if admitted && figure.placed.is_none() {
                    self.figures.mark(stack, self.frame);
                }
            }
        }
    }

    /// Divisions and stacks the steps have marked since the layer was made, and figures placed
    /// anew.
    pub(crate) fn marked(&self) -> [usize; 3] {
        self.marked
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the units layer is declared before it composes")
    }

    fn mark_division(&mut self, slot: u32) {
        if self.divisions.mark(slot, self.frame) {
            self.marked[0] += 1;
        }
    }

    /// Marks the stack holding division `slot`, for a change that leaves its route and stack key
    /// alone.
    fn mark_stack_of(&mut self, slot: u32) {
        let Some(member) = self.members.get_mut(slot as usize) else { return };
        member.stale = true;
        let stack = member.stack;
        if stack != NONE {
            self.mark_stack(stack);
        }
    }

    fn mark_stack(&mut self, stack: u32) {
        if self.dirty_stacks.mark(stack, self.frame) {
            self.marked[1] += 1;
        }
    }

    /// Everything dropped and every division marked: the settle that follows makes it whole.
    fn mark_all(&mut self, mirrors: &Mirrors) {
        self.members.clear();
        self.stacks.clear();
        self.free.clear();
        self.by_key.clear();
        self.divisions.clear();
        self.moved.clear();
        [&mut self.route_rows, &mut self.icon_rows, &mut self.status_rows].into_iter().for_each(Rows::clear);
        self.every = false;
        self.dirty_stacks.clear();
        self.figures.clear();
        self.sights.clear();
        self.readers.clear();
        self.camera_dirty = true;
        self.deferred.clear();
        self.deferred_count = 0;
        self.asking.clear();
        self.touched.clear();
        self.asks.clear();
        if let Some(meshes) = &mut self.meshes {
            meshes.clear();
        }
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        self.combat_dirty = true;
        self.links_dirty = true;
        if let Some(divisions) = rows(mirrors.get(TableId::Divisions)) {
            for slot in divisions.rows() {
                self.divisions.mark(slot, self.frame);
            }
        }
    }

    fn mark_sight(&mut self, slot: u32) {
        if self.sights.mark(slot, self.frame) {
            self.sighted += 1;
        }
    }

    /// Every division a stack holds: its sight is read again.
    fn mark_sights(&mut self) {
        for slot in 0..self.members.len() as u32 {
            if self.members[slot as usize].stack != NONE {
                self.mark_sight(slot);
            }
        }
    }

    /// The vision layer's changes since this layer last settled (the host keeps them until every
    /// dependent has): the divisions whose sight read a province whose byte moved, or whose own
    /// sight moved; every division when the viewer, the fog or another whole part did.
    fn take_upstream(&mut self, vision: &VisionLayer, upstream: &Upstream<'_>, mirrors: &Mirrors) {
        let moved = self.choice.as_ref() != Some(vision.choice());
        if moved {
            self.choice = Some(vision.choice().clone());
        }
        if moved || self.sight_epoch != Some(vision.sight_epoch()) {
            self.sight_epoch = Some(vision.sight_epoch());
            self.mark_sights();
            return;
        }
        let changed = upstream.changed::<VisionLayer>();
        for &province in changed.of("province") {
            for &slot in self.readers.get(&(province as u32)).into_iter().flatten() {
                if self.sights.mark(slot, self.frame) {
                    self.sighted += 1;
                }
            }
        }
        let units = changed.of("unit");
        if let Some(divisions) = mirrors.get(TableId::Divisions).filter(|_| !units.is_empty()) {
            for &unit in units {
                if let Some(slot) = divisions.slot(u128::from(unit)).filter(|&slot| self.members.get(slot as usize).is_some_and(|member| member.stack != NONE)) {
                    self.mark_sight(slot);
                }
            }
        }
    }

    /// Keeps division `slot` among the readers of province `read_at` (none: of no province).
    fn index(&mut self, slot: u32, read_at: u32) {
        let old = std::mem::replace(&mut self.members[slot as usize].read_at, read_at);
        if old == read_at {
            return;
        }
        if let Some(readers) = self.readers.get_mut(&old) {
            if let Some(at) = readers.iter().position(|&held| held == slot) {
                readers.swap_remove(at);
            }
            if readers.is_empty() {
                self.readers.remove(&old);
            }
        }
        if read_at != NONE {
            self.readers.entry(read_at).or_default().push(slot);
        }
    }

    /// Division `slot`'s sight read again; its stack is marked when it changed.
    fn resight(&mut self, slot: u32, seer: Seer<'_>) {
        let Some(&Member { stack, id, sighted: Some((province, owner)), .. }) = self.members.get(slot as usize) else { return };
        if stack == NONE {
            return;
        }
        let (seen, read_at) = seer.sees(id, owner, province);
        self.index(slot, read_at);
        if std::mem::replace(&mut self.members[slot as usize].seen, seen) != seen {
            self.mark_stack(stack);
        }
    }

    /// CountryLinks wholly recorded and the status table read, or why not.
    fn check(&self, mirrors: &Mirrors, layer: Option<&VisionLayer>) -> Result<(), String> {
        if self.reading {
            let vision = layer.ok_or("The units layer is refused: the vision layer is not registered before it")?;
            if let Some(why) = vision.refused() {
                return Err(format!("The units layer is refused: the vision layer has no vision: {why}"));
            }
        } else if self.vision.is_none() {
            return Err("The units layer is refused: it is given no vision of the viewing country yet".into());
        }
        let links = mirrors.get(TableId::CountryLinks).ok_or("CountryLinks is not read yet")?;
        if let Some(why) = links.not_recorded() {
            return Err(format!("{why}; the core tables Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn"));
        }
        if mirrors.get(TableId::DivisionStatus).and_then(Mirror::status) == Some(Status::NotRecorded) {
            return Err("The units layer is refused: DivisionStatus is not recorded here, and the DivisionModelInputs witness a recording without it carries is not read".into());
        }
        Ok(())
    }

    fn read_links(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.links.clear();
        if let Some(links) = mirrors.get(TableId::CountryLinks) {
            for slot in links.rows() {
                if let (Some(key), Some(canonical)) = (links.key(slot), links.number(columns.link_slot, slot).value()) {
                    self.links.insert(key as u32, canonical);
                }
            }
        }
    }

    /// Each division's combat side; marks the divisions whose side changed.
    fn read_combat(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        let mut combat = FastMap::with_capacity_and_hasher(self.combat.len(), Default::default());
        if let Some(combats) = rows(mirrors.get(TableId::LandCombats)) {
            let mut ordered: Vec<(u128, u32)> = combats.rows().filter_map(|slot| Some((combats.key(slot)?, slot))).collect();
            ordered.sort_unstable();
            for (_, slot) in ordered {
                for (column, side) in [(columns.attackers, ATTACK), (columns.defenders, DEFEND)] {
                    if let Cell::Value(bytes) = combats.list(column, slot) {
                        decode::<8>(bytes, &mut self.items);
                        for &id in &self.items {
                            combat.insert(id, side);
                        }
                    }
                }
            }
        }
        let old = std::mem::replace(&mut self.combat, combat);
        let divisions = mirrors.get(TableId::Divisions);
        let changed: Vec<u64> = old.iter().filter(|(id, side)| self.combat.get(id) != Some(side)).map(|(&id, _)| id).chain(self.combat.keys().filter(|id| !old.contains_key(id)).copied()).collect();
        for id in changed {
            if let Some(slot) = divisions.and_then(|divisions| divisions.slot(u128::from(id))) {
                self.mark_stack_of(slot);
            }
        }
    }

    /// The next two hops of a division's route the content can place from `province`, which key its
    /// stack; none when it does not move or the route cannot be placed.
    fn signature(&mut self, mirrors: &Mirrors, id: u64, province: u32) -> Option<[u32; 2]> {
        let columns = self.columns();
        let movement = rows(mirrors.get(TableId::DivisionMovement))?;
        let slot = movement.slot(u128::from(id))?;
        let number = |column: Num<u32>| movement.number(column, slot).value().unwrap_or(0);
        let byte = |column: Num<u8>| movement.number(column, slot).value().unwrap_or(0);
        let (mut path, mut alternate) = (std::mem::take(&mut self.path), std::mem::take(&mut self.alternate));
        path_of(movement, columns.path, slot, &mut path);
        path_of(movement, columns.alternate_path, slot, &mut alternate);
        let unit = UnitRoute {
            class: viewer_foundation::unit_class::CounterClass::Division,
            combat_role: byte(columns.combat_role),
            combat_count: number(columns.combat_count),
            combat_province: number(columns.combat_province),
            target: number(columns.target),
            naval_transfer: byte(columns.naval_transfer) != 0,
            path_total: number(columns.path_total),
            path: &path,
            alternate_total: number(columns.alternate_total),
            alternate_path: &alternate,
        };
        let ground = &*self.ground;
        let anchor = |province| ground.anchor(province, STANDSTILL);
        let resolved = ground.world().and_then(|world| unit.stack_kind(province, world.map, &anchor)).is_some();
        let display = unit.display_path();
        let signature = resolved.then(|| display.first().map(|&next| [next, display.get(1).copied().unwrap_or(0)])).flatten();
        (self.path, self.alternate) = (path, alternate);
        signature
    }

    /// One division's route from its DivisionMovement row standing in `province`.
    fn route(&mut self, mirrors: &Mirrors, id: u64, province: u32) -> Option<DivisionRoute> {
        let columns = self.columns();
        let movement = rows(mirrors.get(TableId::DivisionMovement))?;
        let slot = movement.slot(u128::from(id))?;
        let number = |column: Num<u32>| movement.number(column, slot).value().unwrap_or(0);
        let byte = |column: Num<u8>| movement.number(column, slot).value().unwrap_or(0);
        let (mut path, mut alternate) = (std::mem::take(&mut self.path), std::mem::take(&mut self.alternate));
        path_of(movement, columns.path, slot, &mut path);
        path_of(movement, columns.alternate_path, slot, &mut alternate);
        let unit = UnitRoute {
            class: viewer_foundation::unit_class::CounterClass::Division,
            combat_role: byte(columns.combat_role),
            combat_count: number(columns.combat_count),
            combat_province: number(columns.combat_province),
            target: number(columns.target),
            naval_transfer: byte(columns.naval_transfer) != 0,
            path_total: number(columns.path_total),
            path: &path,
            alternate_total: number(columns.alternate_total),
            alternate_path: &alternate,
        };
        let ground = &*self.ground;
        let anchor = |province| ground.anchor(province, STANDSTILL);
        let placement = unit.resolve(province, ground.world().map(|world| world.map), &anchor);
        let route = DivisionRoute::of_placement(&unit, byte(columns.retreat) != 0, None, placement);
        (self.path, self.alternate) = (path, alternate);
        Some(route)
    }

    /// One division's stack from its row, route and owner link; moves it between stacks when its key
    /// changed and marks the stacks it touched.
    fn place_division(&mut self, slot: u32, mirrors: &Mirrors, seer: Seer<'_>) {
        #[cfg(test)]
        {
            self.placed += 1;
        }
        let columns = self.columns();
        let at = slot as usize;
        if self.members.len() <= at {
            self.members.resize(at + 1, Member::default());
        }
        let divisions = rows(mirrors.get(TableId::Divisions));
        let row = divisions.filter(|divisions| divisions.live(slot)).and_then(|divisions| {
            let id = divisions.key(slot)? as u64;
            let province = divisions.number(columns.province, slot).value().unwrap_or(0);
            let owner = divisions.number(columns.owner, slot).value().unwrap_or(0);
            Some((id, province, owner))
        });
        let key = match row {
            Some((id, province, owner)) if province != 0 => match self.links.get(&owner).copied() {
                Some(owner_slot) => {
                    let signature = self.signature(mirrors, id, province);
                    let member = &self.members[at];
                    if member.id != id || member.sighted != Some((province, owner_slot)) || member.read_at == NONE {
                        let (seen, read_at) = seer.sees(id, owner_slot, province);
                        let member = &mut self.members[at];
                        (member.id, member.sighted, member.seen) = (id, Some((province, owner_slot)), seen);
                        self.index(slot, read_at);
                    }
                    Some((province, owner_slot, signature))
                }
                None => None,
            },
            _ => None,
        };
        if key.is_none() {
            self.index(slot, NONE);
        }
        self.members[at].stale = true;
        let held = self.members[at].stack;
        if held != NONE && Some(self.stacks[held as usize].key) == key {
            self.mark_stack(held);
            return;
        }
        if held != NONE {
            self.leave(slot);
            self.mark_stack(held);
        }
        if let Some(key) = key {
            let stack = match self.by_key.get(&key) {
                Some(&stack) => stack,
                None => {
                    let fresh = Stack { key, members: Vec::new(), admitted: self.admits(key.0), figure: None, asked: None };
                    let stack = match self.free.pop() {
                        Some(stack) => {
                            self.stacks[stack as usize] = fresh;
                            stack
                        }
                        None => {
                            self.stacks.push(fresh);
                            self.stacks.len() as u32 - 1
                        }
                    };
                    self.by_key.insert(key, stack);
                    stack
                }
            };
            let members = &mut self.stacks[stack as usize].members;
            self.members[at].stack = stack;
            self.members[at].at = members.len() as u32;
            members.push(slot);
            self.mark_stack(stack);
        }
    }

    fn leave(&mut self, slot: u32) {
        let member = &mut self.members[slot as usize];
        let (stack, at) = (member.stack, member.at as usize);
        member.stack = NONE;
        let members = &mut self.stacks[stack as usize].members;
        members.swap_remove(at);
        if let Some(&moved) = members.get(at) {
            self.members[moved as usize].at = at as u32;
        }
    }

    /// One stack's oldest member and figure; an empty stack is dropped.
    fn place_stack(&mut self, stack: u32, mirrors: &Mirrors) {
        let columns = self.columns();
        let at = stack as usize;
        if at >= self.stacks.len() || !self.by_key.get(&self.stacks[at].key).is_some_and(|&held| held == stack) {
            return;
        }
        if self.stacks[at].members.is_empty() {
            let key = self.stacks[at].key;
            self.by_key.remove(&key);
            self.stacks[at].figure = None;
            if let Some(oldest) = self.stacks[at].asked.take() {
                self.gone(oldest, stack);
            }
            self.free.push(stack);
            return;
        }
        let Some(divisions) = mirrors.get(TableId::Divisions) else { return };
        let (province, owner, _) = self.stacks[at].key;
        let held = &self.members;
        let members = self.stacks[at].members.iter().map(|&slot| (held[slot as usize].id, slot));
        let Some((oldest, slot)) = members.clone().filter(|&(_, slot)| held[slot as usize].seen).min() else {
            let Some((oldest, slot)) = members.min() else { return };
            let number = |column: Num<u32>| divisions.number(column, slot).value().unwrap_or(0);
            let selection = Selection {
                transport: false,
                template: divisions.number(columns.template, slot).value().unwrap_or(0),
                oldest,
                roster_owner: number(columns.roster),
                expeditionary_owner: number(columns.expeditionary),
                status: false,
            };
            let placing = Placing { province, owner, oldest, route: None, combat: None, training: None, uv_moving: false, scale: 0.0 };
            let admitted = self.stacks[at].admitted;
            self.stacks[at].figure = Some(Figure { selection, admitted, placed: Some(Err(Withheld::Unseen)), last: None, placing });
            self.touched.push(stack);
            return;
        };
        let unchanged = self.stacks[at].figure.as_ref().is_some_and(|held| held.selection.oldest == oldest && !matches!(held.placed, Some(Err(Withheld::Unseen))));
        if unchanged && !self.members[slot as usize].stale {
            return;
        }
        self.members[slot as usize].stale = false;
        let route = self.route(mirrors, oldest, province);
        let route = route.as_ref();
        let alternate = rows(mirrors.get(TableId::ArmyIconInputs))
            .and_then(|icons| icons.slot(u128::from(oldest)).and_then(|row| icons.number(columns.alternate_template, row).value()))
            .filter(|&template| template != 0);
        let number = |column: Num<u32>| divisions.number(column, slot).value().unwrap_or(0);
        let transport = route.is_some_and(|route| route.naval_transfer);
        let status = rows(mirrors.get(TableId::DivisionStatus)).and_then(|statuses| statuses.slot(u128::from(oldest)).map(|row| statuses.number(columns.exercising, row)));
        let movement = rows(mirrors.get(TableId::DivisionMovement)).and_then(|movement| movement.slot(u128::from(oldest)).map(|row| (movement, row)));
        let uv_moving = movement.is_some_and(|(movement, row)| {
            movement.number(columns.path_total, row).value().unwrap_or(0) > 0 && movement.number(columns.target, row).value().unwrap_or(0) == 0
        });
        let selection = Selection {
            transport,
            template: alternate.unwrap_or_else(|| divisions.number(columns.template, slot).value().unwrap_or(0)),
            oldest,
            roster_owner: number(columns.roster),
            expeditionary_owner: number(columns.expeditionary),
            status: status.is_some(),
        };
        let scalars = Scalars {
            province,
            owner,
            oldest,
            combat: self.combat.get(&oldest).copied(),
            training: status.and_then(Cell::value).map(|exercising| exercising != 0),
            uv_moving,
            scale: if transport { self.ground.transport_scale() } else { 1.0 },
        };
        let admitted = self.stacks[at].admitted;
        let kept = self.stacks[at].figure.as_ref().is_some_and(|held| matches!(held.placed, Some(Ok(_)) | Some(Err(Withheld::Unplaced(_)))) && held.placing.same(&scalars, route));
        let placing = (!kept).then(|| scalars.with(route.cloned()));
        let held = self.stacks[at].figure.take();
        let figure = self.refigured(stack, held, (selection, admitted, transport), placing);
        self.stacks[at].figure = Some(figure);
        self.touched.push(stack);
    }

    /// `held`'s figure under `selection`: the same placing kept (`placing` none), or one made anew
    /// for `placing`, drawn where `held` was until it is placed.
    fn refigured(&mut self, stack: u32, held: Option<Figure>, (selection, admitted, transport): (Selection, bool, bool), placing: Option<Placing>) -> Figure {
        match (held, placing) {
            (Some(held), None) => Figure { selection, admitted, ..held },
            (held, placing) => {
                let placing = placing.expect("a placing not kept is made");
                let last = held.as_ref().and_then(Figure::drawn).cloned();
                self.placed_anew(stack, selection, admitted, transport, placing, last)
            }
        }
    }

    /// Its section of the flight record ([`viewer_foundation::view_record`]): the stacks asking a
    /// mesh, and those of them placed and admitted whose figure has no mesh yet and no reason it has
    /// none, `entered_unready` (a figure late into the view: the bar is zero); and the meshes' levels.
    fn record_view(&self) {
        use viewer_foundation::view_record::{add, set};
        set("units", "stacks_admitted", self.asking.len() as u64);
        let Some(meshes) = &self.meshes else { return };
        let unready = self
            .asking
            .iter()
            .filter(|&(oldest, &(stack, _))| {
                !meshes.meshes.contains_key(oldest)
                    && !meshes.withheld.contains_key(oldest)
                    && self.stacks.get(stack as usize).is_some_and(|held| held.admitted && held.figure.as_ref().is_some_and(|figure| matches!(figure.placed, Some(Ok(_)))))
            })
            .count();
        add("units", "entered_unready", unready as u64);
        meshes.record_view();
    }

    /// The stacks a settle touched handed to the meshes' worker: a figure placed and admitted asks
    /// the mesh of its oldest member where it stands, under its selection; one that no longer does
    /// lets its oldest member go.
    fn ask(&mut self) {
        let mut touched = std::mem::take(&mut self.touched);
        touched.sort_unstable();
        touched.dedup();
        for &stack in &touched {
            let at = stack as usize;
            if at >= self.stacks.len() || self.by_key.get(&self.stacks[at].key) != Some(&stack) {
                continue;
            }
            let held = &self.stacks[at];
            // Today's models give no figure to a stack whose province has no content bounds.
            let bounded = self.ground.bounds().contains_key(&held.key.0);
            // A figure whose placing changed waits to be placed again, within a later frame's
            // budget: it keeps what it asked, and asks again only once its new placing is made.
            if bounded && held.admitted && held.asked.is_some() && held.figure.as_ref().is_some_and(|figure| figure.placed.is_none()) {
                continue;
            }
            let wants = held.figure.as_ref().filter(|figure| bounded && held.admitted && matches!(figure.placed, Some(Ok(_)))).map(|figure| {
                let Selection { transport, template, oldest, roster_owner, expeditionary_owner, .. } = figure.selection;
                (oldest, (figure.placing.province, unit_models::SelectionKey { transport, template, oldest, roster_owner, expeditionary_owner }))
            });
            let asked = std::mem::replace(&mut self.stacks[at].asked, wants.map(|(oldest, _)| oldest));
            if let Some(oldest) = asked.filter(|&oldest| wants.is_none_or(|(wanted, _)| wanted != oldest)) {
                self.gone(oldest, stack);
            }
            // Asked again only for a new place or selection: a stack renumbered asks nothing.
            if let Some((oldest, asked)) = wants
                && self.asking.insert(oldest, (stack, asked)).is_none_or(|(_, before)| before != asked)
            {
                self.asks.push(meshes::Ask::Stack(oldest, asked));
            }
        }
        touched.clear();
        self.touched = touched;
    }

    /// `stack` no longer asks the mesh of `oldest`, unless another stack took it over.
    fn gone(&mut self, oldest: u64, stack: u32) {
        if self.asking.get(&oldest).is_some_and(|&(held, _)| held == stack) {
            self.asking.remove(&oldest);
            self.asks.push(meshes::Ask::Gone(oldest));
        }
    }

    /// A stack's figure for a placing that changed: withheld without a status, else waiting to be
    /// made (marked when admitted), drawn meanwhile where it was (`last`).
    fn placed_anew(&mut self, stack: u32, selection: Selection, admitted: bool, transport: bool, placing: Placing, last: Option<(Target, Position)>) -> Figure {
        match () {
            () if !transport && !selection.status => Figure { selection, admitted, placed: Some(Err(Withheld::Status)), last: None, placing },
            () => {
                if admitted {
                    self.figures.mark(stack, self.frame);
                }
                Figure { selection, admitted, placed: None, last, placing }
            }
        }
    }

    /// An admitted stack's figure made from its placing, the route's spline included.
    fn place_figure(&mut self, stack: u32) {
        let at = stack as usize;
        let live = at < self.stacks.len() && self.by_key.get(&self.stacks[at].key) == Some(&stack) && self.stacks[at].admitted;
        let Some(figure) = self.stacks.get(at).filter(|_| live).and_then(|held| held.figure.as_ref()).filter(|figure| figure.placed.is_none()) else { return };
        let placed = self.figure(&figure.placing).map_err(Withheld::Unplaced);
        self.marked[2] += 1;
        if let Some(figure) = &mut self.stacks[at].figure {
            (figure.placed, figure.last) = (Some(placed), None);
        }
        self.touched.push(stack);
    }

    fn figure(&self, placing: &Placing) -> Result<(Target, Position), Unplaced> {
        let ground = &*self.ground;
        let world = ground.world();
        let anchor = |province, slot| ground.anchor(province, slot);
        let stack = LandStack {
            province: placing.province,
            owner: placing.owner,
            oldest: placing.oldest,
            route: placing.route.as_ref(),
            combat: placing.combat,
            training: placing.training,
            uv_moving: placing.uv_moving,
            scale: placing.scale,
        };
        let (target, position) = unit_models::land_figure(&stack, world.as_ref(), &anchor)?;
        Ok((unit_models::drawn(target, &position), position))
    }

    /// Recomputes what is dirty within `budget`: the links and combat sides whole, then divisions,
    /// then stacks, the rest carried.
    /// Marks made while it settles (rows resolved, groups and plans expanded) carry the frame of the
    /// steps that brought them: the frame advances once the settle is done.
    fn settle_dirty(&mut self, mirrors: &Mirrors, budget: &mut Budget, layer: Option<&VisionLayer>) {
        self.settle_marks(mirrors, budget, layer);
        self.frame += 1;
    }

    fn settle_marks(&mut self, mirrors: &Mirrors, budget: &mut Budget, layer: Option<&VisionLayer>) {
        // A table back after a view that drew none returns as a reset, which composes the layer.
        if self.columns.is_none() || TABLES.iter().any(|&table| mirrors.get(table).is_none()) {
            return;
        }
        let mut renewed = false;
        if !self.reading && self.fresh_vision.swap(false, Ordering::AcqRel) {
            let given = self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
            renewed = !matches!((&self.vision, &given), (Some((_, held)), Some((_, new))) if Arc::ptr_eq(held, new));
            self.vision = given;
        }
        if let Err(why) = self.check(mirrors, layer) {
            if self.refused.as_ref() != Some(&why) {
                self.mark_all(mirrors);
                self.divisions.clear();
                self.refused = Some(why);
            }
            // Refused, it reads no sight; accepted again, it places every division.
            self.sights.clear();
            return;
        }
        let given = self.vision.as_ref().map(|(_, vision)| Arc::clone(vision));
        let seer = if self.reading { layer.map(Seer::Layer) } else { given.as_deref().map(Seer::Given) };
        let Some(seer) = seer else { return };
        if self.refused.take().is_some() {
            self.mark_all(mirrors);
        } else if renewed {
            self.mark_sights();
        }
        if self.camera_dirty {
            self.readmit(mirrors);
        }
        if self.links_dirty {
            self.links_dirty = false;
            self.read_links(mirrors);
        }
        if self.combat_dirty {
            self.combat_dirty = false;
            self.read_combat(mirrors);
        }
        self.resolve(mirrors, budget);
        let mut dirty = std::mem::take(&mut self.divisions);
        let _ = dirty.settle(budget, |slot| {
            self.place_division(slot, mirrors, seer);
            Ok(())
        });
        self.divisions = dirty;
        let mut dirty = std::mem::take(&mut self.sights);
        let _ = dirty.settle(budget, |slot| {
            self.resight(slot, seer);
            Ok(())
        });
        self.sights = dirty;
        let mut dirty = std::mem::take(&mut self.dirty_stacks);
        let _ = dirty.settle(budget, |stack| {
            self.place_stack(stack, mirrors);
            Ok(())
        });
        self.dirty_stacks = dirty;
        let mut dirty = std::mem::take(&mut self.figures);
        let _ = dirty.settle(budget, |stack| {
            self.place_figure(stack);
            Ok(())
        });
        self.figures = dirty;
    }

    /// Everything made whole from the mirrors and the vision.
    fn compose_from(&mut self, mirrors: &Mirrors, layer: Option<&VisionLayer>) {
        if self.columns.is_none() {
            return;
        }
        let (marked, sighted) = (self.marked, self.sighted);
        self.refused = Some("composing".into());
        self.settle_dirty(mirrors, &mut Budget::unlimited(), layer);
        (self.marked, self.sighted) = (marked, sighted);
    }

    /// Marks every division the mirror holds and every one a stack holds.
    fn mark_every(&mut self, divisions: Option<&Mirror>) {
        self.deferred.clear();
        self.deferred_count = 0;
        let mut all: Vec<u32> = divisions.map(|divisions| divisions.rows().collect()).unwrap_or_default();
        all.extend((0..self.members.len() as u32).filter(|&slot| self.members[slot as usize].stack != NONE));
        all.into_iter().for_each(|slot| self.mark_division(slot));
    }

    /// Changes the steps marked, resolved within `budget`: every division when one changed for all,
    /// the divisions the by-division rows name, and the moved divisions re-keyed where the camera
    /// admits them (the rest deferred).
    fn resolve(&mut self, mirrors: &Mirrors, budget: &mut Budget) {
        let divisions = mirrors.get(TableId::Divisions);
        if std::mem::take(&mut self.every) {
            self.mark_every(divisions);
            self.moved.clear();
        }
        for (table, route) in [(TableId::DivisionMovement, true), (TableId::ArmyIconInputs, false), (TableId::DivisionStatus, false)] {
            let mirror = mirrors.get(table);
            let held = match table {
                TableId::DivisionMovement => &mut self.route_rows,
                TableId::ArmyIconInputs => &mut self.icon_rows,
                _ => &mut self.status_rows,
            };
            let mut rows = std::mem::take(held);
            rows.settle(budget, |row| {
                let key = match row {
                    Row::Gone(key) => Some(key),
                    Row::At(slot) => mirror.and_then(|mirror| mirror.key(slot)),
                };
                match key.and_then(|key| divisions?.slot(key)) {
                    Some(slot) if route => self.mark_moved(slot, divisions),
                    Some(slot) => self.mark_stack_of(slot),
                    None => {}
                }
            });
            match table {
                TableId::DivisionMovement => self.route_rows = rows,
                TableId::ArmyIconInputs => self.icon_rows = rows,
                _ => self.status_rows = rows,
            }
        }
        // Counted as marked when the step marked it.
        let mut moved = std::mem::take(&mut self.moved);
        let _ = moved.settle(budget, |slot| {
            if self.admission.is_none() || self.wanted(slot, divisions) {
                self.divisions.mark(slot, self.frame);
            } else {
                self.defer(slot);
            }
            Ok(())
        });
        self.moved = moved;
    }

    fn rows_carried(&self) -> usize {
        self.moved.len() + self.route_rows.len() + self.icon_rows.len() + self.status_rows.len() + usize::from(self.every)
    }

    /// The admitted land figures as the field draws them, each with its mesh.
    fn land(&self, meshes: &HashMap<u64, usize>) -> Vec<Target> {
        self.stacks
            .iter()
            .filter(|stack| stack.admitted)
            .filter_map(|stack| stack.figure.as_ref()?.drawn())
            .filter_map(|(target, _)| meshes.get(&target.identity).map(|&mesh| Target { mesh, ..*target }))
            .collect()
    }

    /// Entries of `self` that differ from `other`'s, and the first described; a figure is compared
    /// only while its stack is admitted (one kept from an earlier camera is not drawn).
    pub(crate) fn differ(&self, other: &Self) -> (u64, Option<String>) {
        let mut differing = 0;
        let mut first = None;
        let mut note = |what: String| {
            differing += 1;
            first.get_or_insert(what);
        };
        if self.refused() != other.refused() {
            note(format!("refused: {:?} against {:?}", self.refused(), other.refused()));
        }
        let admitted_only = self.admission.is_some() || other.admission.is_some();
        let (mut mine, mut theirs) = (self.figures(), other.figures());
        if admitted_only {
            mine.retain(|_, figure| figure.admitted);
            theirs.retain(|_, figure| figure.admitted);
        }
        if !admitted_only && self.stack_count() != other.stack_count() {
            note(format!("{} stacks against {}", self.stack_count(), other.stack_count()));
        }
        for (key, figure) in &mine {
            match theirs.get(key) {
                None => note(format!("stack {key:?} is not composed")),
                Some(other) => {
                    if figure.selection != other.selection {
                        note(format!("stack {key:?}: selection {:?} against {:?}", figure.selection, other.selection));
                    }
                    if figure.admitted != other.admitted {
                        note(format!("stack {key:?}: admitted {} against {}", figure.admitted, other.admitted));
                    }
                    let placed = |figure: &Figure| figure.placed.as_ref().map(|placed| placed.as_ref().map(|(target, _)| *target).map_err(|why| *why));
                    if figure.admitted && placed(figure) != placed(other) {
                        note(format!("stack {key:?}: figure {:?} against {:?}", placed(figure), placed(other)));
                    }
                }
            }
        }
        for key in theirs.keys().filter(|key| !mine.contains_key(key)) {
            note(format!("composed stack {key:?} is not held"));
        }
        (differing, first)
    }
}

impl Layer for Units {
    fn name(&self) -> &'static str {
        "units"
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if self.reading {
            inputs.after("vision");
        }
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        let drawn = view.camera.is_none_or(|camera| camera.height < self.ground.units_cutoff()) && visibility::of(&view.map_mode).land_models;
        if !drawn {
            self.columns = None;
            self.refused = Some("no land unit model is drawn at this view".into());
            self.members = Vec::new();
            self.stacks = Vec::new();
            self.free = Vec::new();
            self.by_key = FastMap::default();
            self.combat = FastMap::default();
            self.links = FastMap::default();
            self.divisions = Dirty::default();
            self.moved = Dirty::default();
            (self.route_rows, self.icon_rows, self.status_rows, self.every) = (Rows::default(), Rows::default(), Rows::default(), false);
            self.uploaded = Vec::new();
            if let (Some(window), Some(meshes)) = (&mut self.window, &self.meshes) {
                window.meshes = meshes.tables().to_vec();
            }
            (self.meshes, self.asking, self.touched, self.asks) = (None, FastMap::default(), Vec::new(), Vec::new());
            self.dirty_stacks = Dirty::default();
            self.figures = Dirty::default();
            self.sights = Dirty::default();
            self.readers = FastMap::default();
            self.deferred = Vec::new();
            self.deferred_count = 0;
            (self.province_known, self.province_in, self.statuses) = (Vec::new(), Vec::new(), HashMap::new());
            (self.choice, self.sight_epoch) = (None, None);
            // The window then poses today's models: the raid units and convoys are not this layer's.
            if let (Some(window), Some(models)) = (&mut self.window, self.models.take()) {
                window.tables = models.tables().to_vec();
                self.to_upload = false;
            }
            return Ok(());
        }
        if let Some(window) = &self.window
            && self.models.is_none()
        {
            self.models = Some(models::Models::new(Arc::clone(&window.select), &window.rows, &window.tables));
        }
        if let Some(window) = &self.window
            && self.meshes.is_none()
        {
            self.meshes = Some(meshes::Meshes::start(Arc::clone(&window.select), &window.rows, &window.meshes));
        }
        for table in [TableId::Divisions, TableId::DivisionMovement, TableId::ArmyIconInputs, TableId::DivisionStatus, TableId::LandCombats, TableId::CountryLinks] {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            province: inputs.number(TableId::Divisions, "province")?,
            owner: inputs.number(TableId::Divisions, "logical_owner")?,
            template: inputs.number(TableId::Divisions, "template")?,
            roster: inputs.number(TableId::Divisions, "roster_owner")?,
            expeditionary: inputs.number(TableId::Divisions, "expeditionary_owner")?,
            target: inputs.number(TableId::DivisionMovement, "target")?,
            retreat: inputs.number(TableId::DivisionMovement, "retreat")?,
            naval_transfer: inputs.number(TableId::DivisionMovement, "naval_transfer")?,
            combat_count: inputs.number(TableId::DivisionMovement, "combat_count")?,
            path_total: inputs.number(TableId::DivisionMovement, "path_total")?,
            path: inputs.list(TableId::DivisionMovement, "path")?,
            alternate_total: inputs.number(TableId::DivisionMovement, "alternate_total")?,
            alternate_path: inputs.list(TableId::DivisionMovement, "alternate_path")?,
            combat_role: inputs.number(TableId::DivisionMovement, "combat_role")?,
            combat_province: inputs.number(TableId::DivisionMovement, "combat_province")?,
            alternate_template: inputs.number(TableId::ArmyIconInputs, "alternate_template")?,
            exercising: inputs.number(TableId::DivisionStatus, "exercising")?,
            attackers: inputs.list(TableId::LandCombats, "attackers")?,
            defenders: inputs.list(TableId::LandCombats, "defenders")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
        });
        Ok(())
    }

    fn viewed(&mut self, _view: &View, mirrors: &Mirrors) {
        if self.columns.is_some() && self.camera_dirty && self.refused.is_none() {
            self.readmit(mirrors);
        }
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_from(mirrors, None);
        Ok(())
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        let layer = upstream.get::<VisionLayer>().filter(|_| self.reading && self.columns.is_some());
        if let Some(vision) = layer {
            (self.choice, self.sight_epoch) = (Some(vision.choice().clone()), Some(vision.sight_epoch()));
        }
        self.compose_from(mirrors, layer);
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() {
            return;
        }
        #[cfg(test)]
        let reads = crate::playhead_host::mirror::probe::reads();
        let columns = self.columns();
        let frame = self.frame;
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            if !TABLES.contains(&table) {
                continue;
            }
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                match table {
                    TableId::CountryLinks => self.links_dirty = true,
                    TableId::LandCombats => self.combat_dirty = true,
                    _ => {}
                }
                self.every = true;
                continue;
            }
            match table {
                TableId::Divisions => {
                    let keyed = [columns.province.column, columns.owner.column];
                    let selected = [columns.template.column, columns.roster.column, columns.expeditionary.column];
                    let mut moved = |slot: u32| {
                        if self.moved.mark(slot, frame) {
                            self.marked[0] += 1;
                        }
                    };
                    for &slot in changes.deleted().slots().iter().chain(changes.inserted().slots()) {
                        moved(slot);
                    }
                    for written in changes.columns().iter().filter(|written| keyed.contains(&written.column())) {
                        written.slots().iter().for_each(|&slot| moved(slot));
                    }
                    for written in changes.columns().iter().filter(|written| selected.contains(&written.column())) {
                        written.slots().iter().for_each(|&slot| self.mark_stack_of(slot));
                    }
                }
                TableId::DivisionMovement => {
                    let read = [
                        columns.target.column,
                        columns.retreat.column,
                        columns.naval_transfer.column,
                        columns.combat_count.column,
                        columns.path_total.column,
                        columns.path.column,
                        columns.alternate_total.column,
                        columns.alternate_path.column,
                        columns.combat_role.column,
                        columns.combat_province.column,
                    ];
                    self.route_rows.take(changes, frame, |column| read.contains(&column));
                }
                TableId::ArmyIconInputs => self.icon_rows.take(changes, frame, |column| column == columns.alternate_template.column),
                TableId::DivisionStatus => self.status_rows.take(changes, frame, |column| column == columns.exercising.column),
                TableId::LandCombats => {
                    let read = [columns.attackers.column, columns.defenders.column];
                    if !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty() && read.contains(&written.column())) {
                        self.combat_dirty = true;
                    }
                }
                TableId::CountryLinks => {
                    if !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty() && written.column() == columns.link_slot.column) {
                        self.links_dirty = true;
                        self.every = true;
                    }
                }
                _ => {}
            }
        }
        #[cfg(test)]
        {
            self.applied_reads += crate::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.settle_dirty(mirrors, budget, None);
        Ok(())
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        // A view that draws no land figure does no unit work.
        if self.columns.is_none() {
            return Ok(());
        }
        let layer = upstream.get::<VisionLayer>().filter(|_| self.reading);
        if let Some(vision) = layer {
            self.take_upstream(vision, upstream, mirrors);
        }
        let before = self.marked;
        self.settle_dirty(mirrors, budget, layer);
        self.to_upload |= self.marked != before;
        // Each stack's mesh: the stacks the settle touched handed on with the hour it settled.
        if self.meshes.is_some() {
            self.ask();
        } else {
            self.touched.clear();
        }
        let ours = layer.and_then(VisionLayer::ours).or_else(|| self.vision.as_ref().map(|(_, vision)| vision.ours()));
        let hour = mirrors.get(TableId::Divisions).and_then(Mirror::head);
        if let (Some(meshes), Some(ours), Some(hour), None) = (&mut self.meshes, ours, hour, &self.refused) {
            let asks = std::mem::take(&mut self.asks);
            self.to_upload |= meshes.exchange(hour, ours, asks, budget.unlimited_now());
        }
        if viewer_foundation::view_record::on() {
            self.record_view();
        }
        let convoys = upstream.get::<crate::layers::convoys::Convoys>().filter(|convoys| convoys.refused().is_none()).map(|convoys| (convoys.product_seq(), &convoys.product().convoys[..]));
        // The ships layer's ships, while it stands in for today's, are joined to the models.
        let ships = upstream.get::<crate::layers::ships::Ships>();
        let choice = self.vision.as_ref().map(|(choice, _)| choice).or(self.choice.as_ref());
        if let (Some(models), Some(choice)) = (&mut self.models, choice) {
            let whole = mirrors.get(TableId::Divisions).and_then(Mirror::head).filter(|_| budget.unlimited_now());
            let exchanged = models.exchange(choice, convoys, !ships.is_some_and(crate::layers::ships::Ships::stands_in), whole);
            let joined = models.join_ships(ships);
            self.models_new |= exchanged | joined;
            self.to_upload |= exchanged | joined;
        }
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        let Some(models) = &self.models else { return Ok(()) };
        let Some((_, prepared)) = models.prepared.as_ref().filter(|_| self.to_upload) else { return Ok(()) };
        // Placed again where they stood, with the same meshes, the figures need no new field.
        let none = HashMap::new();
        let land = self.land(self.meshes.as_ref().map_or(&none, |meshes| &meshes.meshes));
        if !self.models_new && land == self.uploaded {
            self.to_upload = false;
            return Ok(());
        }
        let field = prepared.field_with_land(&gpu.content.unit_models, land.clone());
        gpu.scene.set_unit_models(gpu.device, gpu.queue, &gpu.content.unit_models, field, false)?;
        (self.to_upload, self.models_new, self.uploaded) = (false, false, land);
        Ok(())
    }

    fn uploaded(&self) -> bool {
        !self.to_upload || self.models.as_ref().is_none_or(|models| models.prepared.is_none())
    }

    fn published(&self) -> Published {
        let Some(models) = self.models.as_ref().filter(|_| self.columns.is_some()) else { return Published::Current };
        let meshes = self.meshes.as_ref();
        match &models.prepared {
            None if models.waiting() => Published::Nothing,
            _ if self.refused.is_none() && meshes.is_some_and(meshes::Meshes::waiting) => Published::Nothing,
            // The figures are at the head; their models and meshes are made from rows at the head
            // on the rows worker: the hour those rows stand at is the lag, never the map's hour.
            Some((hour, _)) if self.uploaded() && self.carried().entities == 0 => {
                let models = models.stands().unwrap_or(*hour);
                Published::Cadence(meshes.and_then(|meshes| meshes.hour).map_or(models, |meshes| meshes.min(models)))
            }
            _ => Published::Current,
        }
    }

    fn carried(&self) -> Carried {
        // A models job out is the worker's, on the rows' cadence: counted here it froze the shown hour while playing.
        let entities = self.divisions.len() + self.rows_carried() + self.sights.len() + self.dirty_stacks.len() + self.figures.len() + usize::from(self.combat_dirty) + usize::from(self.links_dirty);
        let rows = [&self.route_rows, &self.icon_rows, &self.status_rows].into_iter().filter_map(Rows::since);
        let since = [self.divisions.since(), self.moved.since(), self.sights.since(), self.dirty_stacks.since(), self.figures.since()].into_iter().flatten().chain(rows).min();
        Carried { entities, frames: since.map_or(0, |since| self.frame.saturating_sub(since)) }
    }

    fn held(&self) -> usize {
        let stacks: usize = self.stacks.iter().map(|stack| stack.members.capacity() * 4 + stack.figure.as_ref().and_then(|figure| figure.placing.route.as_ref()).map_or(0, |route| route.path.capacity() * 4)).sum();
        self.members.capacity() * std::mem::size_of::<Member>()
            + self.stacks.capacity() * std::mem::size_of::<Stack>()
            + stacks
            + self.free.capacity() * 4
            + self.by_key.capacity() * (std::mem::size_of::<StackKey>() + 4 + 8)
            + self.combat.capacity() * 20
            + self.links.capacity() * 12
            + self.divisions.bytes()
            + self.moved.bytes()
            + self.route_rows.bytes()
            + self.icon_rows.bytes()
            + self.status_rows.bytes()
            + self.uploaded.capacity() * std::mem::size_of::<Target>()
            + self.sights.bytes()
            + self.readers.capacity() * (4 + std::mem::size_of::<Vec<u32>>() + 8)
            + self.readers.values().map(|readers| readers.capacity() * 4).sum::<usize>()
            + self.dirty_stacks.bytes()
            + self.figures.bytes()
            + self.meshes.as_ref().map_or(0, meshes::Meshes::bytes)
            + self.asking.capacity() * (8 + 4 + std::mem::size_of::<meshes::Asked>() + 8)
            + self.touched.capacity() * 4
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    /// Mutably for a test only: no host call asks the units layer.
    #[cfg(test)]
    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Units {
    /// What its meshes' worker answered since the last call; none outside the window.
    pub(crate) fn take_mesh_counts(&self) -> Option<meshes::Counts> {
        self.meshes.as_ref().map(meshes::Meshes::take_counts)
    }

    /// What the models' worker (the ships, raid units and convoys) did since the last call.
    pub(crate) fn take_model_counts(&self) -> Option<models::Counts> {
        self.models.as_ref().map(models::Models::take_counts)
    }

    /// Each admitted figure's mesh by its oldest member, as the worker last answered.
    #[cfg(test)]
    pub(crate) fn meshes(&self) -> Option<&HashMap<u64, usize>> {
        self.meshes.as_ref().map(|meshes| &meshes.meshes)
    }

    /// The hour of the rows its meshes stand for.
    #[cfg(test)]
    pub(crate) fn mesh_hour(&self) -> Option<replay_data::Hour> {
        self.meshes.as_ref().and_then(|meshes| meshes.hour)
    }

    /// The meshes' asks, rows and waits, and the stacks asked by oldest member, for a test's message.
    #[cfg(test)]
    pub(crate) fn mesh_state(&self) -> String {
        let asking: BTreeMap<u64, u32> = self.asking.iter().map(|(&oldest, &(stack, _))| (oldest, stack)).collect();
        format!("{}; asking {asking:?}", self.meshes.as_ref().map_or_else(|| "no meshes".to_owned(), meshes::Meshes::state))
    }
}

/// `side`'s units layer.
pub(crate) fn of(side: &crate::playhead_host::MapSide) -> Option<&Units> {
    let (_, layers) = side.parts()?;
    layers.iter().find_map(|layer| layer.any().downcast_ref::<Units>())
}

/// The unit models `side`'s units layer made, which carry the raid units and convoys it draws;
/// none while it makes none.
pub(crate) fn prepared(side: &crate::playhead_host::MapSide) -> Option<&Arc<viewer_world::map::unit_models::PreparedModels>> {
    of(side).and_then(Units::prepared)
}

#[cfg(test)]
mod tests;
