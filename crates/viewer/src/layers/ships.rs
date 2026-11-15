//! Ships at sea, a task force at a time: each formation's candidate, placement and hulls as today's
//! model layer makes them ([`UnitModelAssets::naval_force`]), from the mirrors. A task force's own
//! values (province, owners, mission, motion route, mission-skip operands, its place among its
//! province's occupants) are kept from change sets for every task force; its figure (the route's
//! spline and the hulls' targets) is made only while the camera admits its province. Which entity a
//! hull draws is chosen by the content's scripts over any table, so a task force's roster is resolved
//! on the layer's worker from rows at the head ([`Rosters`]) when its roster key changes (a member
//! ship's model row, class or task force, its owners, the variants' entities, the countries) or a
//! table only the scripts read changed, and kept until then.
//!
//! What a task force sees is the vision's (P6's layer): the window's layer takes its task forces'
//! sight from the vision layer when one of them moves, the proof gives it whole
//! ([`Ships::set_vision`]); without one the layer
//! is refused by name. The land occupants that compete for a province's representative and the naval
//! combats are not made here (today's `add_land_competitors`, `prepare_naval_combats`): while an
//! admitted task force is in combat or shares its province with a land occupant, the layer does not
//! stand in for today's ships ([`Ships::stands_in`]) and the window draws today's.

use super::{Budget, Carried, Dirty, Gpu, Inputs, Layer, List, Num, Published, Text, Upstream, View};
use crate::{
    document::Content,
    playhead_host::{
        Entry,
        mirror::{Cell, Mirror, Mirrors},
        rows::{Cadence, Recording, RowReader, Rows},
    },
};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_foundation::map::{
    camera::{MapDefines, MapSize, Placement},
    counter_move::{RouteWorld, UnitRoute},
    perspective::{Choice, Vision},
    province_admission::Admission,
    task_force_visual::skip_of,
};
use viewer_world::map::{
    equipment_graphics::{Decided, LimitValues, Limits},
    unit_history::NavalSources,
    unit_models::{Force, ForcePart, ModelReport, ResolvedRoster, ShipGround},
};

/// How often the worker's rows are made while playing; at once when paused or sought.
pub(crate) const CADENCE: Cadence = Cadence::Ready;

/// The tables the roster resolution reads before any script, and the Clock, whose hour the graphics
/// pools' limits may compare; the rest are learned.
const ROSTER_TABLES: &[TableId] = &[TableId::ShipModels, TableId::Ships, TableId::TaskForces, TableId::VariantDefinitions, TableId::Countries, TableId::CountryLinks, TableId::Clock];

/// A roster as resolved: none for a task force with no roster ship.
pub(crate) type Roster = Option<Result<ResolvedRoster, String>>;

/// What the layer reads of the content: when ships are drawn, where provinces are, a formation's
/// figure from its values and a batch of rosters from rows (today's builder's two cores).
pub(crate) trait Naval: Send + Sync {
    /// Whether ship models are drawn below a camera at `height`.
    fn drawn_at(&self, height: f32) -> bool;
    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]>;
    fn map(&self) -> (MapSize, MapDefines);
    fn region(&self, province: u32) -> Option<u32>;
    #[expect(clippy::too_many_arguments, reason = "one formation's every input, as the builder's core takes them")]
    fn figure(&self, force: &Force, route: Result<UnitRoute<'_>, &'static str>, link: &dyn Fn(u32) -> Option<u32>, vision: &Vision, skip: &dyn Fn(u32) -> Option<bool>, ordinal: Option<u32>, speed: Option<i32>, day: i32, roster: &mut dyn FnMut() -> Roster) -> ForcePart;
    /// The rosters of `forces` over `frame`, and the graphics pools their limits decided.
    fn rosters(&self, frame: &DecodedFrame, forces: &[u64]) -> io::Result<(Vec<(u64, Roster)>, Decided)>;
    /// Whether every pool of `decided` decides the same over `frame`.
    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool;
}

impl Naval for Content {
    fn drawn_at(&self, height: f32) -> bool {
        self.unit_models.gates.ships(height)
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        &self.unit_models.bounds
    }

    fn map(&self) -> (MapSize, MapDefines) {
        (MapSize::of(&self.map), self.map_defines)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.counters.region(province)
    }

    fn figure(&self, force: &Force, route: Result<UnitRoute<'_>, &'static str>, link: &dyn Fn(u32) -> Option<u32>, vision: &Vision, skip: &dyn Fn(u32) -> Option<bool>, ordinal: Option<u32>, speed: Option<i32>, day: i32, roster: &mut dyn FnMut() -> Roster) -> ForcePart {
        let counters = &self.counters;
        let movement = self.movement.as_deref();
        let heights = viewer_foundation::map::province_map::HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        let world = movement.map(|movement| RouteWorld { map: &movement.map, heights, width: self.map.width as f32 });
        let definitions = &self.map.definitions;
        let land = |province| match movement.and_then(|movement| movement.map.terrain(province)) {
            Some(terrain) => terrain.flags & 1 != 0,
            None => matches!(definitions.get(&province).map(|it| it.kind), Some(installed_content_core::map::ProvinceKind::Land)),
        };
        let anchor = |province, slot| counters.anchor(province, slot);
        let scale = |province, kind| counters.stack_scale(province, kind);
        let ground = ShipGround { movement, world: world.as_ref(), anchor: &anchor, land: &land, scale: &scale };
        self.unit_models.naval_force(force, route, &ground, link, vision, skip, ordinal, speed, day, roster)
    }

    fn rosters(&self, frame: &DecodedFrame, forces: &[u64]) -> io::Result<(Vec<(u64, Roster)>, Decided)> {
        engine_executable::executable_tables::scope(self.tables, || {
            let hour = crate::document::Hour::new(self, frame, 0.0);
            let evaluate = |items: &[installed_content_core::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
            let values = LimitValues::default();
            let limits = Limits::new(&values, &evaluate);
            let rosters = self.unit_models.resolve_rosters(frame, limits, &self.counters.terrain.combat_terrain, &self.names.localisation, forces)?;
            Ok((rosters, limits.decided()))
        })
    }

    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool {
        engine_executable::executable_tables::scope(self.tables, || {
            let hour = crate::document::Hour::new(self, frame, 0.0);
            let evaluate = |items: &[installed_content_core::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
            let values = LimitValues::default();
            Limits::new(&values, &evaluate).decide_the_same(decided)
        })
    }
}

#[derive(Clone, Copy)]
struct Columns {
    province: Num<u32>,
    logical_owner: Num<u32>,
    roster_owner: Num<u32>,
    expeditionary_owner: Num<u32>,
    mission: Num<u32>,
    target: Num<u32>,
    combat_role: Num<u8>,
    path: List,
    alternate_path: List,
    is_in_regions: Num<u8>,
    bombardment_region: Num<u32>,
    current_region: Num<u32>,
    mixed: List,
    task_force: Num<u64>,
    class_id: Num<i32>,
    model: [usize; 6],
    explicit_entity: Text,
    tag: Text,
    link_slot: Num<u32>,
    speed: Num<i32>,
}

/// One task force as the layer keeps it: its values, whether the camera admits it, its part of the
/// hour's ships once made, and its roster as last resolved.
#[derive(Clone, Default)]
struct Held {
    force: Option<Force>,
    admitted: bool,
    part: Option<ForcePart>,
    roster: Option<Arc<Roster>>,
    /// Its roster was asked of the worker and not answered yet.
    asked: bool,
    /// Its province's occupants hold one that is no task force: today's land competitors.
    contested: bool,
}

/// The rows the worker reads, and what it answers.
struct Asked {
    source: Option<(Arc<Entry>, Recording)>,
    open: bool,
    stop: bool,
    /// The hour the map applied, which a driving layer drives the rows to.
    head: Option<Hour>,
    rows: Option<Arc<Rows>>,
    /// A reader declared by the caller of [`Ships::set_source`], for the worker to take.
    reader: Option<RowReader>,
    /// Task forces whose roster is asked, and every one resolved so far (asked again when a table
    /// only the scripts read changes).
    wanted: Vec<u64>,
    known: Vec<u64>,
    /// The graphics pools the rosters known decided: asked again at a new hour when one decides
    /// otherwise.
    decided: Decided,
    answers: Vec<(u64, Arc<Roster>)>,
    /// The hour of the rows the last answers were resolved at.
    answered: Option<Hour>,
    tables: Vec<TableId>,
    builds: Vec<u64>,
    error: Option<String>,
    /// Why every roster of this recording is refused: a table the scripts read that the recording
    /// holds nothing of.
    refused: Option<String>,
    /// The worker holds a reader whose tables have not all arrived.
    loading: bool,
}

struct Shared {
    asked: Mutex<Asked>,
    wake: Condvar,
    answered: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Asked> {
        self.asked.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The layer's worker: task forces' rosters resolved from rows at the head, a batch at a time.
pub(crate) struct Rosters {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    drive: bool,
}

impl Rosters {
    fn new(naval: Arc<dyn Naval>, drive: bool) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            asked: Mutex::new(Asked { source: None, open: false, stop: false, head: None, rows: None, reader: None, wanted: Vec::new(), known: Vec::new(), decided: Decided::new(), answers: Vec::new(), answered: None, tables: ROSTER_TABLES.to_vec(), builds: Vec::new(), error: None, refused: None, loading: false }),
            wake: Condvar::new(),
            answered: Condvar::new(),
        });
        let working = Arc::clone(&shared);
        let worker = std::thread::Builder::new().name("ship rosters".into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            resolve(&working, &*naval, drive)
        })?;
        Ok(Self { shared, worker: Some(worker), drive })
    }

    fn set_open(&self, open: bool) {
        let mut asked = self.shared.lock();
        if asked.open != open {
            asked.open = open;
            if !open {
                asked.wanted.clear();
                asked.known.clear();
                asked.decided.clear();
                asked.answers.clear();
                asked.reader = None;
            }
            drop(asked);
            self.shared.wake.notify_all();
        }
    }

    fn applied(&self, hour: Hour) {
        let mut asked = self.shared.lock();
        if asked.head == Some(hour) {
            return;
        }
        asked.head = Some(hour);
        let rows = if self.drive { asked.rows.clone() } else { None };
        drop(asked);
        if let Some(rows) = rows {
            rows.drive(hour);
        }
        self.shared.wake.notify_all();
    }

    fn ask(&self, forces: impl IntoIterator<Item = u64>) {
        let mut asked = self.shared.lock();
        let before = asked.wanted.len();
        asked.wanted.extend(forces);
        if asked.wanted.len() != before {
            drop(asked);
            self.shared.wake.notify_all();
        }
    }

    fn take(&self) -> Vec<(u64, Arc<Roster>)> {
        std::mem::take(&mut self.shared.lock().answers)
    }

    /// The hour the rows of the rosters answered stand at; none before the first answer.
    fn answered(&self) -> Option<Hour> {
        self.shared.lock().answered
    }

    /// Waits until every roster asked is answered from rows at `hour`, once the map applied it; for
    /// a caller that holds the head (the proof tool, tests), never the window.
    pub(crate) fn wait(&self, hour: Hour, limit: Duration) -> io::Result<()> {
        let until = Instant::now() + limit;
        let mut asked = self.shared.lock();
        if asked.head.is_none() || !asked.open {
            return Ok(());
        }
        loop {
            if let Some(error) = &asked.error {
                return Err(io::Error::other(format!("The ship rosters' worker stopped: {error}")));
            }
            if asked.wanted.is_empty() && asked.answered == Some(hour) {
                return Ok(());
            }
            let now = Instant::now();
            if now >= until {
                return Err(io::Error::other(format!("The ship rosters were not answered at hour {hour} in {limit:?} (rows at {:?}, {} asked, loading {}; the rows' worker: {:?})", asked.answered, asked.wanted.len(), asked.loading, asked.rows.as_ref().and_then(|rows| rows.error()))));
            }
            asked = self.shared.answered.wait_timeout(asked, (until - now).min(Duration::from_millis(50))).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }

    /// Whether rosters are asked and not answered yet.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> bool {
        !self.shared.lock().wanted.is_empty()
    }

    /// Whether the worker's rows are loading: a table it reads has not arrived.
    #[cfg(test)]
    pub(crate) fn loading(&self) -> bool {
        self.shared.lock().loading
    }

    /// The batches' milliseconds and the tables the scripts were found to read.
    pub(crate) fn measured(&self) -> (Vec<u64>, Vec<TableId>) {
        let asked = self.shared.lock();
        (asked.builds.clone(), asked.tables.clone())
    }

    /// What the rows at the head hold for the rosters' tables: the worker's mirrors and the rows.
    pub(crate) fn rows_held(&self) -> Option<(usize, usize)> {
        let asked = self.shared.lock();
        let held = asked.rows.as_ref()?.held();
        let mine = |list: &[(TableId, usize)]| list.iter().filter(|(table, _)| asked.tables.contains(table)).map(|(_, bytes)| bytes).sum();
        Some((mine(&held.mirrors), mine(&held.rows)))
    }
}

impl Drop for Rosters {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The worker: whenever rosters are asked, or a table only the scripts read changed under the
/// rosters it knows, resolve them over the newest rows and hand the answers back.
fn resolve(shared: &Shared, naval: &dyn Naval, drive: bool) {
    let mut reader: Option<RowReader> = None;
    let mut last: Option<Arc<DecodedFrame>> = None;
    loop {
        let mut asked = shared.lock();
        if asked.stop {
            return;
        }
        if !asked.open {
            // Closed, it holds no reader: its tables leave the rows and come back as a reset.
            (reader, asked.reader, last, asked.loading) = (None, None, None, false);
            let _ = shared.wake.wait_timeout(asked, Duration::from_millis(17));
            continue;
        }
        if let Some(why) = asked.refused.clone() {
            (reader, asked.reader, asked.loading) = (None, None, false);
            let forces = std::mem::take(&mut asked.wanted);
            let refused = !forces.is_empty();
            asked.answers.extend(forces.into_iter().map(|force| (force, Arc::new(Some(Err(why.clone()))))));
            if let Some(head) = asked.head.filter(|&head| refused || asked.answered != Some(head)) {
                asked.answered = Some(head);
                shared.answered.notify_all();
            }
            let _ = shared.wake.wait_timeout(asked, Duration::from_millis(17));
            continue;
        }
        if reader.is_none() {
            reader = asked.reader.take();
        }
        if reader.is_none() {
            let rows = match (asked.rows.clone(), asked.source.clone()) {
                (Some(rows), _) => Some(rows),
                (None, Some((entry, recording))) => Rows::of(&entry, recording).map_err(|error| asked.error = Some(error.to_string())).ok(),
                (None, None) => None,
            };
            if let Some(rows) = rows {
                reader = Some(rows.reader(&asked.tables, CADENCE));
                if drive && let Some(head) = asked.head {
                    rows.drive(head);
                }
                asked.rows = Some(rows);
            }
        }
        // Between batches it has finished with the rows it took.
        if let Some(reader) = &reader {
            reader.ready();
        }
        // A frame made before a table was declared lacks it: rosters over it would learn it again.
        let frame = reader.as_ref().and_then(RowReader::frame).filter(|frame| super::rebuilt::holds(frame, &asked.tables));
        asked.loading = reader.is_some() && frame.is_none();
        let Some(frame) = frame else {
            let _ = shared.wake.wait_timeout(asked, Duration::from_millis(17));
            continue;
        };
        let newer = last.as_ref().is_none_or(|last| !Arc::ptr_eq(last, &frame));
        let scripts_moved = newer && last.as_ref().is_some_and(|last| {
            (0..frame.tables.len()).any(|table| !ROSTER_TABLES.iter().any(|known| known.index() == table) && !replay_frame::frame::SharedView::ptr_eq(&frame.tables[table], &last.tables[table]))
        });
        // Rows newer only in their hour: the rosters known stand while every pool they decided
        // decides the same at it, judged off the lock.
        let recheck = (newer && !scripts_moved && !asked.decided.is_empty()).then(|| asked.decided.clone());
        let mut limits_moved = false;
        if let Some(decided) = recheck {
            drop(asked);
            let (same, missed) = replay_frame::frame::discovering(|| naval.decide_the_same(&frame, &decided));
            limits_moved = !same || !missed.is_empty();
            asked = shared.lock();
        }
        let mut forces: Vec<u64> = std::mem::take(&mut asked.wanted);
        let whole = scripts_moved || limits_moved;
        if whole {
            forces.extend(asked.known.iter().copied());
        }
        if forces.is_empty() {
            if newer {
                asked.answered = Some(frame.hour);
                last = Some(frame);
                shared.answered.notify_all();
            } else if let Some(stands) = reader.as_ref().and_then(|reader| reader.through(Arc::as_ptr(&frame)))
                && asked.answered.is_some_and(|answered| answered < stands)
            {
                // No step since changed the rows every roster known was answered from.
                asked.answered = Some(stands);
                shared.answered.notify_all();
            }
            let _ = shared.wake.wait_timeout(asked, Duration::from_millis(17));
            continue;
        }
        forces.sort_unstable();
        forces.dedup();
        drop(asked);
        let started = Instant::now();
        let (resolved, missed) = replay_frame::frame::discovering(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| naval.rosters(&frame, &forces))).unwrap_or_else(|panic| Err(io::Error::other(format!("The rosters' build panicked: {}", super::rebuilt::panicked(&*panic)))))
        });
        let ns = started.elapsed().as_nanos() as u64;
        let mut asked = shared.lock();
        if !missed.is_empty() {
            asked.wanted.extend(forces);
            let absent = asked.source.as_ref().map_or_else(Vec::new, |(entry, _)| super::rebuilt::absent(entry, &missed));
            if !absent.is_empty() {
                asked.refused = Some(super::rebuilt::refusal("the ships' rosters", &absent));
                continue;
            }
            asked.tables.extend(missed);
            asked.tables.sort_unstable_by_key(|table| table.index());
            asked.tables.dedup();
            if let Some(reader) = &reader {
                reader.declare(&asked.tables);
            }
            last = None;
            continue;
        }
        if whole {
            asked.decided.clear();
        }
        let answers: Vec<(u64, Arc<Roster>)> = match resolved {
            Ok((resolved, decided)) => {
                asked.decided.extend(decided);
                resolved.into_iter().map(|(force, roster)| (force, Arc::new(roster))).collect()
            }
            Err(error) => forces.iter().map(|&force| (force, Arc::new(Some(Err(error.to_string()))))).collect(),
        };
        asked.builds.push(ns);
        asked.known.extend(forces);
        asked.known.sort_unstable();
        asked.known.dedup();
        asked.answers.extend(answers);
        asked.answered = Some(frame.hour);
        last = Some(frame);
        drop(asked);
        shared.answered.notify_all();
    }
}

/// The rows a table holds at the head: none when unavailable, not recorded or in a gap.
fn rows(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn complete(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| mirror.status() == Some(Status::Complete))
}

fn decode_u32(bytes: &[u8], out: &mut Vec<u32>) {
    out.clear();
    out.extend(bytes.chunks_exact(4).map(|item| u32::from_le_bytes([item[0], item[1], item[2], item[3]])));
}

/// A task force's motion route from its TaskForceMapMotion row: the one place that table is read,
/// so a change of its shape changes this alone. The lists are decoded into `path` and `alternate`.
fn motion_route<'a>(motion: Option<&Mirror>, columns: &Columns, force: u64, path: &'a mut Vec<u32>, alternate: &'a mut Vec<u32>) -> Result<UnitRoute<'a>, &'static str> {
    let motion = motion.ok_or("task-force motion census incomplete")?;
    let row = motion.slot(u128::from(force)).ok_or("task-force motion row absent")?;
    for (column, out) in [(columns.path, &mut *path), (columns.alternate_path, &mut *alternate)] {
        match motion.list(column, row) {
            Cell::Value(bytes) => decode_u32(bytes, out),
            _ => out.clear(),
        }
    }
    let combat_role = motion.number(columns.combat_role, row).value().unwrap_or(0);
    Ok(UnitRoute {
        class: viewer_foundation::unit_class::CounterClass::TaskForce,
        combat_role,
        combat_count: u32::from(combat_role != 0),
        combat_province: 0,
        target: motion.number(columns.target, row).value().unwrap_or(0),
        naval_transfer: false,
        path_total: path.len() as u32,
        path,
        alternate_total: alternate.len() as u32,
        alternate_path: alternate,
    })
}

/// A task force's mission-skip operands from its TaskForceCounterInputs row (is in regions,
/// bombardment region): the one place that table is read.
fn skip_inputs(inputs: Option<&Mirror>, columns: &Columns, force: u64) -> Option<(u8, u32)> {
    let inputs = inputs?;
    let row = inputs.slot(u128::from(force))?;
    Some((inputs.number(columns.is_in_regions, row).value().unwrap_or(0), inputs.number(columns.bombardment_region, row).value().unwrap_or(0)))
}

/// The ships layer's arrays and what keeps them.
pub(crate) struct Ships {
    naval: Arc<dyn Naval>,
    given: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision: Option<(Choice, Arc<Vision>)>,
    columns: Option<Columns>,
    refused: Option<String>,
    /// By TaskForces slot.
    forces: Vec<Held>,
    /// Task force id to slot, in key order (the order today's pass walks).
    by_id: BTreeMap<u64, u32>,
    /// Each ship's task force, as last read.
    ship_force: HashMap<u64, u64>,
    /// Each province's task forces.
    in_province: HashMap<u32, Vec<u64>>,
    dirty: Dirty,
    figures: Dirty,
    camera: Option<(Placement, [u32; 2])>,
    admission: Option<Admission>,
    camera_dirty: bool,
    all_dirty: bool,
    rosters_dirty: bool,
    statuses: HashMap<usize, Status>,
    rosters: Rosters,
    /// Whether a compose keeps the rosters held: a copy made to compare with, given the rosters.
    reuse: bool,
    /// The game day the parts' candidates carry.
    day: Option<i32>,
    frame: u64,
    /// Task forces marked, figures made, rosters asked.
    marked: [usize; 3],
    path: Vec<u32>,
    alternate: Vec<u32>,
    /// The window's layer: it reads the vision layer; the sight epoch its vision was taken at, and
    /// whether task forces were born since.
    window: bool,
    sight_epoch: Option<u64>,
    born: bool,
    /// The refusal draws nothing, as today's builder draws nothing then (a table not wholly
    /// recorded), rather than waiting for what it needs.
    like_today: bool,
    /// Changes to what is drawn not yet published, and what was published: its sequence number, the
    /// counts it was published at, and why today's ships are drawn instead.
    touched: u64,
    published: (u64, [usize; 2], u64),
    needs_today: Option<String>,
    /// The unit models' lines of what it draws, as the window joins them, and the publish and
    /// refusal they were made for.
    warnings: Vec<String>,
    warned: Option<(u64, Option<String>)>,
    /// The newest hour the parts stand for: the head of a settle with nothing marked and no task
    /// force waiting for its roster. While one waits, what it publishes.
    product_hour: Option<Hour>,
    /// The hour the map applied, as its task forces' mirror stands.
    head: Option<Hour>,
}

impl Ships {
    /// `drive`: the layer drives its rows to the map's hour (a caller that plays the playhead
    /// itself); otherwise they follow the playhead's clock.
    pub(crate) fn new(naval: Arc<dyn Naval>, drive: bool) -> io::Result<Self> {
        let rosters = Rosters::new(Arc::clone(&naval), drive)?;
        Ok(Self {
            naval,
            given: Mutex::new(None),
            vision: None,
            columns: None,
            refused: Some("not composed yet".into()),
            forces: Vec::new(),
            by_id: BTreeMap::new(),
            ship_force: HashMap::new(),
            in_province: HashMap::new(),
            dirty: Dirty::default(),
            figures: Dirty::default(),
            camera: None,
            admission: None,
            camera_dirty: false,
            all_dirty: true,
            rosters_dirty: true,
            statuses: HashMap::new(),
            rosters,
            reuse: false,
            day: None,
            frame: 0,
            marked: [0; 3],
            path: Vec::new(),
            alternate: Vec::new(),
            window: false,
            sight_epoch: None,
            born: false,
            like_today: false,
            touched: 0,
            published: (0, [0; 2], 0),
            needs_today: None,
            warnings: Vec::new(),
            warned: None,
            product_hour: None,
            head: None,
        })
    }

    /// The unit models' lines of the ships it draws, worded as today's: the admitted task forces'
    /// counts with its refusal named, made when what it draws is published.
    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The lines made again when the publish or the refusal they were made for moved, from the
    /// admitted parts' reports alone.
    fn warn(&mut self) {
        let refused = self.refused.clone().filter(|_| self.like_today && self.columns.is_some());
        if self.warned.as_ref().is_some_and(|(seq, held)| (*seq, held) == (self.published.0, &refused)) {
            return;
        }
        let mut report = ModelReport::default();
        for part in self.parts().into_values().flatten() {
            report.absorb(&part.report);
        }
        if let Some(why) = &refused {
            report.error(why.clone());
        }
        self.warnings = report.warnings();
        self.warned = Some((self.published.0, refused));
    }

    /// The layer the window draws: it takes the vision from the vision layer.
    pub(crate) fn in_window(mut self) -> Self {
        self.window = true;
        self
    }

    /// Whether the window draws this layer's ships rather than today's: drawn and equal to today's
    /// (hidden, or refused as today's builder draws nothing, both draw nothing), and no admitted task
    /// force is in combat or shares its province with a land occupant.
    pub(crate) fn stands_in(&self) -> bool {
        if self.columns.is_none() {
            return true;
        }
        match self.refused {
            None => self.needs_today.is_none(),
            Some(_) => self.like_today,
        }
    }

    /// Why today's ships are drawn instead, while an admitted task force needs what this layer does
    /// not make.
    pub(crate) fn needs_today(&self) -> Option<&str> {
        self.needs_today.as_deref()
    }

    /// The number of what it draws, moved on once a change has settled.
    pub(crate) fn seq(&self) -> u64 {
        self.published.0
    }

    /// What the window draws: the admitted task forces' parts as one hour's ships, and their counts
    /// with the refusal named, when it draws nothing for one.
    pub(crate) fn handover(&self) -> (NavalSources, ModelReport) {
        let (sources, mut report) = self.sources();
        if let Some(why) = self.refused.as_deref().filter(|_| self.like_today && self.columns.is_some()) {
            report.error(why.to_owned());
        }
        (sources, report)
    }

    /// Publishes what is drawn once nothing is left to settle.
    fn publish(&mut self) {
        let at = (self.marked[0], self.marked[1], self.touched);
        if self.dirty.len() > 0 || self.figures.len() > 0 || (self.published.1, self.published.2) == ([at.0, at.1], at.2) {
            return;
        }
        self.published = (self.published.0 + 1, [at.0, at.1], at.2);
        self.needs_today = self.today_needed();
    }

    /// The first admitted task force today's builder draws otherwise: in combat (its battle), or in a
    /// province where a land occupant competes for a ship's representative.
    fn today_needed(&self) -> Option<String> {
        self.by_id.values().map(|&slot| &self.forces[slot as usize]).filter(|held| held.admitted).find_map(|held| {
            let candidate = held.part.as_ref()?.candidate.as_ref()?;
            if matches!(candidate.slot, 9 | 10) {
                return Some(format!("a task force is in a naval combat in province {}", candidate.province));
            }
            (held.contested && candidate.slot != 19).then(|| format!("a land occupant shares province {} with a task force", candidate.province))
        })
    }

    /// The playhead whose rows the rosters are resolved from. Its reader is declared here and the
    /// call returns once the rows' worker has declared it, so a seek after it brings the tables:
    /// a table declared later joins the playhead past its head.
    pub(crate) fn set_source(&self, entry: Arc<Entry>, recording: Recording) -> io::Result<()> {
        let mut asked = self.rosters.shared.lock();
        if asked.source.as_ref().is_some_and(|(held, _)| Arc::ptr_eq(held, &entry)) {
            return Ok(());
        }
        let rows = Rows::of(&entry, recording)?;
        let absent = super::rebuilt::absent(&entry, &asked.tables);
        asked.refused = (!absent.is_empty()).then(|| super::rebuilt::refusal("the ships' rosters", &absent));
        asked.reader = asked.refused.is_none().then(|| rows.reader(&asked.tables, CADENCE));
        let declared = asked.reader.is_some();
        (asked.source, asked.rows) = (Some((entry, recording)), Some(Arc::clone(&rows)));
        drop(asked);
        if declared {
            rows.settled();
        }
        self.rosters.shared.wake.notify_all();
        Ok(())
    }

    /// What the viewer sees and the perspective it is for; every figure is made again with it.
    pub(crate) fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        *self.given.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((choice, vision));
    }

    pub(crate) fn perspective(&self) -> Option<&Choice> {
        self.vision.as_ref().map(|(choice, _)| choice)
    }

    pub(crate) fn rosters(&self) -> &Rosters {
        &self.rosters
    }

    pub(crate) fn drawn(&self) -> bool {
        self.columns.is_some()
    }

    pub(crate) fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub(crate) fn marked(&self) -> [usize; 3] {
        self.marked
    }

    /// Task forces held and those the camera admits.
    pub(crate) fn counts(&self) -> [usize; 2] {
        [self.by_id.len(), self.by_id.values().filter(|&&slot| self.forces[slot as usize].admitted).count()]
    }

    /// Each admitted task force's part, by id; none for one whose figure is not made yet.
    pub(crate) fn parts(&self) -> BTreeMap<u64, Option<&ForcePart>> {
        self.by_id.iter().filter(|(_, slot)| self.forces[**slot as usize].admitted).map(|(&id, &slot)| (id, self.forces[slot as usize].part.as_ref())).collect()
    }

    /// The cell of `size` world units holding most placed formations: its centre and count.
    pub(crate) fn front(&self, size: f32) -> Option<([f32; 2], usize)> {
        let mut cells: HashMap<(i32, i32), usize> = HashMap::new();
        for part in self.parts().into_values().flatten() {
            let Some(placement) = &part.position else { continue };
            let [x, _, z] = placement.position.at();
            *cells.entry(((x / size).floor() as i32, (z / size).floor() as i32)).or_default() += 1;
        }
        cells.into_iter().max_by_key(|&(cell, count)| (count, std::cmp::Reverse(cell))).map(|((x, z), count)| ([(x as f32 + 0.5) * size, (z as f32 + 0.5) * size], count))
    }

    /// The admitted task forces' parts as one hour's ships, in today's order.
    pub(crate) fn sources(&self) -> (NavalSources, ModelReport) {
        let (mut sources, mut report) = (NavalSources::default(), ModelReport::default());
        for part in self.parts().into_values().flatten() {
            part.clone().into(&mut sources, &mut report);
        }
        (sources, report)
    }

    /// A layer of the same content, inputs, vision and rosters, made from scratch.
    pub(crate) fn fresh(&self) -> io::Result<Self> {
        let mut fresh = Self::new(Arc::clone(&self.naval), false)?;
        (fresh.columns, fresh.camera, fresh.camera_dirty, fresh.vision, fresh.reuse) = (self.columns, self.camera, true, self.vision.clone(), true);
        for (&id, &slot) in &self.by_id {
            if let Some(roster) = &self.forces[slot as usize].roster {
                fresh.forces.resize(fresh.forces.len().max(slot as usize + 1), Held::default());
                fresh.forces[slot as usize].roster = Some(Arc::clone(roster));
                let _ = id;
            }
        }
        Ok(fresh)
    }

    /// Entries of `self` that differ from `other`'s, and the first described: admitted task forces
    /// and their parts.
    pub(crate) fn differ(&self, other: &Self) -> (u64, Option<String>) {
        let (mine, theirs) = (self.sources().0, other.sources().0);
        let (_, differing, first) = viewer_world::map::unit_models::naval_differ(&mine, &theirs);
        let refused = u64::from(self.refused() != other.refused());
        (differing + refused, first.or_else(|| (refused > 0).then(|| format!("refused {:?} against {:?}", self.refused(), other.refused()))))
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the ships layer is declared before it settles")
    }

    fn admits(&self, province: u32) -> bool {
        self.admission.as_ref().is_none_or(|admission| admission.admits_province(self.naval.bounds(), province))
    }

    fn readmit(&mut self) {
        self.camera_dirty = false;
        self.touched += 1;
        let (size, defines) = self.naval.map();
        self.admission = self.camera.filter(|(_, viewport)| *viewport != [0, 0]).map(|(placement, viewport)| Admission::new(placement.on(size, defines), viewport));
        let held: Vec<u32> = self.by_id.values().copied().collect();
        for slot in held {
            let at = slot as usize;
            let admitted = self.forces[at].force.is_some_and(|force| self.admits(force.province));
            self.forces[at].admitted = admitted;
            if admitted && self.forces[at].part.is_none() {
                self.figures.mark(slot, self.frame);
            }
        }
    }

    fn mark_force(&mut self, slot: u32) {
        if self.dirty.mark(slot, self.frame) {
            self.marked[0] += 1;
        }
    }

    fn mark_force_id(&mut self, mirrors: &Mirrors, id: u64) {
        if let Some(slot) = mirrors.get(TableId::TaskForces).and_then(|forces| forces.slot(u128::from(id))) {
            self.mark_force(slot);
        }
    }

    /// The roster of task force `id` is stale: it is asked again at once, and its figure waits for
    /// the answer.
    fn stale_roster(&mut self, mirrors: &Mirrors, id: u64) {
        if let Some(slot) = mirrors.get(TableId::TaskForces).and_then(|forces| forces.slot(u128::from(id))) {
            if self.forces.len() <= slot as usize {
                self.forces.resize(slot as usize + 1, Held::default());
            }
            let held = &mut self.forces[slot as usize];
            (held.roster, held.asked) = (None, true);
            self.marked[2] += 1;
            self.rosters.ask([id]);
            self.mark_force(slot);
        }
    }

    /// Why nothing is drawn: the core tables or the ship census not wholly recorded, or no vision.
    fn check(&self, mirrors: &Mirrors) -> Result<(), String> {
        if self.vision.is_none() {
            return Err("The ships layer is refused: it is given no vision of the viewing country yet".into());
        }
        for table in [TableId::CountryLinks, TableId::Countries] {
            if let Some(why) = mirrors.get(table).ok_or(format!("{table:?} is not read yet"))?.not_recorded() {
                return Err(format!("{why}; the core tables carry the political map itself, so this hour cannot be drawn"));
            }
        }
        for table in [TableId::ShipModels, TableId::Ships, TableId::TaskForces] {
            if complete(mirrors.get(table)).is_none() {
                return Err(format!("No ship is drawn: {table:?} is not wholly recorded this hour"));
            }
        }
        Ok(())
    }

    /// Everything dropped and every task force marked.
    fn mark_all(&mut self, mirrors: &Mirrors) {
        self.all_dirty = false;
        self.touched += 1;
        self.by_id.clear();
        self.in_province.clear();
        self.dirty.clear();
        self.figures.clear();
        for held in &mut self.forces {
            (held.force, held.part, held.admitted) = (None, None, false);
        }
        self.camera_dirty = true;
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        if let Some(forces) = mirrors.get(TableId::TaskForces) {
            for slot in forces.rows() {
                self.dirty.mark(slot, self.frame);
            }
        }
    }

    /// Every roster asked again: a variant's entity, a country's tag or a link changed.
    fn all_rosters(&mut self, mirrors: &Mirrors) {
        self.rosters_dirty = false;
        self.ship_force.clear();
        let columns = self.columns();
        if let Some(ships) = mirrors.get(TableId::Ships) {
            for slot in ships.rows() {
                if let (Some(ship), Some(force)) = (ships.key(slot), ships.number(columns.task_force, slot).value()) {
                    self.ship_force.insert(ship as u64, force);
                }
            }
        }
        if !self.reuse {
            for held in &mut self.forces {
                (held.roster, held.asked) = (None, false);
            }
        }
        self.all_dirty = true;
    }

    /// One task force's values from its rows; it moves between provinces and its figure is marked.
    fn read_force(&mut self, slot: u32, mirrors: &Mirrors) {
        let columns = self.columns();
        let at = slot as usize;
        if self.forces.len() <= at {
            self.forces.resize(at + 1, Held::default());
        }
        let row = mirrors.get(TableId::TaskForces).filter(|forces| forces.live(slot)).and_then(|forces| {
            let number = |column: Num<u32>| forces.number(column, slot).value().unwrap_or(0);
            Some(Force { id: forces.key(slot)? as u64, province: number(columns.province), logical_owner: number(columns.logical_owner), roster_owner: number(columns.roster_owner), expeditionary_owner: number(columns.expeditionary_owner) })
        });
        let held = &self.forces[at];
        if let Some(old) = held.force {
            if row.is_none_or(|new| new.province != old.province) {
                if let Some(list) = self.in_province.get_mut(&old.province) {
                    list.retain(|&id| id != old.id);
                }
            }
            if row.is_none_or(|new| new.id != old.id) {
                self.by_id.remove(&old.id);
            }
        }
        if let Some(new) = row {
            if held.force.is_none_or(|old| old.province != new.province || old.id != new.id) {
                self.in_province.entry(new.province).or_default().push(new.id);
            }
            if held.force.is_some_and(|old| old.roster_owner != new.roster_owner || old.expeditionary_owner != new.expeditionary_owner || old.id != new.id) {
                self.forces[at].roster = None;
                self.forces[at].asked = false;
            }
            self.by_id.insert(new.id, slot);
        }
        let admitted = row.is_some_and(|force| force.province != 0 && self.admits(force.province));
        let held = &mut self.forces[at];
        (held.force, held.admitted, held.part) = (row, admitted, None);
        if admitted {
            if held.roster.is_none() && !held.asked {
                held.asked = true;
                self.marked[2] += 1;
                if let Some(force) = row {
                    self.rosters.ask([force.id]);
                }
            }
            self.figures.mark(slot, self.frame);
        }
    }

    /// An admitted task force's part, made once its roster is answered.
    fn place(&mut self, slot: u32, mirrors: &Mirrors) {
        let columns = self.columns();
        let at = slot as usize;
        let Some(held) = self.forces.get(at).filter(|held| held.admitted) else { return };
        let Some(force) = held.force else { return };
        let roster = held.roster.clone();
        let naval = Arc::clone(&self.naval);
        let (path, alternate) = (&mut self.path, &mut self.alternate);
        let route = motion_route(complete(mirrors.get(TableId::TaskForceMapMotion)), &columns, force.id, path, alternate);
        let links = mirrors.get(TableId::CountryLinks);
        let link = |raw: u32| links.and_then(|links| links.slot(u128::from(raw))).and_then(|row| links?.number(columns.link_slot, row).value());
        let inputs = skip_inputs(rows(mirrors.get(TableId::TaskForceCounterInputs)), &columns, force.id);
        let current = complete(mirrors.get(TableId::TaskForceMissions)).map(|missions| missions.slot(u128::from(force.id)).and_then(|row| missions.number(columns.current_region, row).value()).unwrap_or(0));
        let mission = mirrors.get(TableId::TaskForces).and_then(|forces| forces.number(columns.mission, slot).value()).unwrap_or(0);
        let region = naval.region(force.province);
        let skip = |kind| skip_of(kind, inputs, mission, current, region);
        let forces = mirrors.get(TableId::TaskForces);
        let mut contested = false;
        let ordinal = complete(mirrors.get(TableId::ProvinceUnitOrder)).and_then(|order| {
            let row = order.slot(u128::from(force.province))?;
            let Cell::Value(bytes) = order.list(columns.mixed, row) else { return None };
            let ids = || bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().expect("eight bytes")));
            contested = ids().any(|id| forces.is_none_or(|forces| forces.slot(u128::from(id)).is_none()));
            ids().position(|id| id == force.id).map(|at| at as u32)
        });
        let clock = complete(mirrors.get(TableId::Clock));
        let speed = clock.and_then(|clock| clock.rows().next().and_then(|row| clock.number(columns.speed, row).value()));
        let day = mirrors.get(TableId::TaskForces).and_then(Mirror::head).map_or(0, |hour| (hour - 43_800_000) / 24);
        let vision = &self.vision.as_ref().expect("a layer places with a vision").1;
        let mut waiting = false;
        let part = naval.figure(&force, route, &link, vision, &skip, ordinal, speed, day, &mut || match &roster {
            Some(roster) => (**roster).clone(),
            None => {
                waiting = true;
                None
            }
        });
        self.forces[at].contested = contested;
        if waiting {
            return;
        }
        self.marked[1] += 1;
        self.forces[at].part = Some(part);
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors, budget: &mut Budget) {
        self.settle_marked(mirrors, budget);
        if self.columns.is_some() && self.refused.is_none() && self.marks() == 0 && self.waiting() == 0 && self.stale() == 0 {
            let head = mirrors.get(TableId::TaskForces).and_then(Mirror::head);
            // The rosters drawn are of the rows they were answered from, which may stand behind the head.
            self.product_hour = head.map(|head| self.rosters.answered().map_or(head, |answered| answered.min(head))).or(self.product_hour);
        }
        self.head = mirrors.get(TableId::TaskForces).and_then(Mirror::head);
        self.publish();
        self.warn();
    }

    /// The frame thread's own marks: task forces and figures to make, and whole passes owed.
    fn marks(&self) -> usize {
        self.dirty.len() + self.figures.len() + usize::from(self.all_dirty) + usize::from(self.rosters_dirty)
    }

    /// Admitted task forces whose part waits for their roster from the worker.
    fn waiting(&self) -> usize {
        self.by_id.values().filter(|&&slot| self.forces[slot as usize].admitted && self.forces[slot as usize].part.is_none()).count()
    }

    /// Admitted task forces drawn with a part whose roster was asked again and not answered yet.
    fn stale(&self) -> usize {
        self.by_id.values().filter(|&&slot| self.forces[slot as usize].admitted && self.forces[slot as usize].asked && self.forces[slot as usize].part.is_some()).count()
    }

    fn settle_marked(&mut self, mirrors: &Mirrors, budget: &mut Budget) {
        self.frame += 1;
        if self.columns.is_none() {
            return;
        }
        let given = self.given.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let renewed = given.is_some();
        if given.is_some() {
            self.vision = given;
        }
        if let Err(why) = self.check(mirrors) {
            if self.refused.as_ref() != Some(&why) {
                self.mark_all(mirrors);
                self.refused = Some(why);
                self.like_today = self.vision.is_some();
            }
            // A refused layer draws nothing and carries nothing; it marks everything when accepted.
            self.dirty.clear();
            self.figures.clear();
            return;
        }
        if self.refused.take().is_some() || renewed {
            (self.all_dirty, self.like_today) = (true, false);
        }
        if self.rosters_dirty {
            self.all_rosters(mirrors);
        }
        if self.all_dirty {
            self.mark_all(mirrors);
        }
        if self.camera_dirty {
            self.readmit();
        }
        if let Some(hour) = mirrors.get(TableId::TaskForces).and_then(Mirror::head) {
            self.rosters.applied(hour);
        }
        let day = mirrors.get(TableId::TaskForces).and_then(Mirror::head).map(|hour| (hour - 43_800_000) / 24);
        if day != self.day {
            self.day = day;
            self.touched += 1;
            for part in self.forces.iter_mut().filter_map(|held| held.part.as_mut()) {
                if let (Some(candidate), Some(day)) = (part.candidate.as_mut(), day) {
                    candidate.day = day;
                }
            }
        }
        // An answer is matched by the task force's slot in the mirror: it can come before the
        // layer has read a task force born in the step that asked it.
        for (force, roster) in self.rosters.take() {
            if let Some(slot) = mirrors.get(TableId::TaskForces).and_then(|forces| forces.slot(u128::from(force))) {
                if self.forces.len() <= slot as usize {
                    self.forces.resize(slot as usize + 1, Held::default());
                }
                let held = &mut self.forces[slot as usize];
                let changed = held.roster.as_deref() != Some(&*roster);
                (held.roster, held.asked) = (Some(roster), false);
                if changed || held.part.is_none() {
                    self.touched += u64::from(held.part.is_some());
                    held.part = None;
                    if held.admitted {
                        self.figures.mark(slot, self.frame);
                    }
                }
            }
        }
        let mut dirty = std::mem::take(&mut self.dirty);
        let _ = dirty.settle(budget, |slot| {
            self.read_force(slot, mirrors);
            Ok(())
        });
        self.dirty = dirty;
        let mut figures = std::mem::take(&mut self.figures);
        let mut waiting = Vec::new();
        let _ = figures.settle(budget, |slot| {
            self.place(slot, mirrors);
            if self.forces.get(slot as usize).is_some_and(|held| held.admitted && held.part.is_none()) {
                waiting.push(slot);
            }
            Ok(())
        });
        self.figures = figures;
        for slot in waiting {
            let held = &mut self.forces[slot as usize];
            if held.roster.is_none() && !held.asked {
                held.asked = true;
                self.marked[2] += 1;
                if let Some(force) = held.force {
                    self.rosters.ask([force.id]);
                }
            }
        }
    }

    /// The window's layer takes from the vision layer the sight of the task forces the mirror holds
    /// ([`crate::layers::vision::Vision::unit_vision`]): every figure is made again when the viewer
    /// or a whole part of the sight changed, or it has none; only those whose sight moved when the
    /// vision layer names them; it is taken again, marking nothing, when task forces were born or
    /// all are placed again. A province's byte moves no ship's sight.
    fn take_vision(&mut self, upstream: &Upstream<'_>, mirrors: &Mirrors, always: bool) {
        use crate::layers::vision::Vision as Layered;
        if !self.window || self.columns.is_none() {
            return;
        }
        let Some(vision) = upstream.get::<Layered>() else { return };
        let given = self.given.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_some();
        let whole = always || (self.vision.is_none() && !given) || self.vision.as_ref().is_some_and(|(choice, _)| choice != vision.choice()) || self.sight_epoch != Some(vision.sight_epoch());
        let named: Vec<u32> = if whole { Vec::new() } else { upstream.changed::<Layered>().of("unit").iter().filter_map(|unit| self.by_id.get(unit).copied()).collect() };
        if !whole && named.is_empty() && !self.born && !self.all_dirty {
            return;
        }
        let forces = mirrors.get(TableId::TaskForces);
        let ids = forces.into_iter().flat_map(|forces| forces.rows().filter_map(move |slot| forces.key(slot)).map(|key| key as u64));
        let Some(seen) = vision.unit_vision(ids) else { return };
        (self.sight_epoch, self.born) = (Some(vision.sight_epoch()), false);
        if whole {
            self.set_vision(vision.choice().clone(), Arc::new(seen));
        } else {
            self.vision = Some((vision.choice().clone(), Arc::new(seen)));
            named.into_iter().for_each(|slot| self.mark_force(slot));
        }
    }

    /// The task forces a by-task-force table's change names.
    fn by_force(changes: &playhead::changes::Changes, mirror: &Mirror, read: &[usize]) -> Vec<u64> {
        let mut ids: Vec<u64> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u64).collect();
        for written in changes.columns().iter().filter(|written| read.contains(&written.column())) {
            ids.extend(written.slots().iter().filter_map(|&slot| mirror.key(slot)).map(|key| key as u64));
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

impl Layer for Ships {
    fn name(&self) -> &'static str {
        "ships"
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        let drawn = view.camera.is_none_or(|camera| self.naval.drawn_at(camera.height));
        self.rosters.set_open(drawn);
        if !drawn {
            self.columns = None;
            self.refused = Some("no ship model is drawn at this view".into());
            self.forces = Vec::new();
            self.by_id = BTreeMap::new();
            self.ship_force = HashMap::new();
            self.in_province = HashMap::new();
            self.dirty = Dirty::default();
            self.figures = Dirty::default();
            self.touched += 1;
            self.publish();
            self.warn();
            return Ok(());
        }
        if self.window {
            inputs.after("vision");
        }
        for table in [TableId::TaskForces, TableId::TaskForceMapMotion, TableId::TaskForceCounterInputs, TableId::TaskForceMissions, TableId::ProvinceUnitOrder, TableId::Ships, TableId::ShipModels, TableId::VariantDefinitions, TableId::Countries, TableId::CountryLinks, TableId::Clock] {
            inputs.table(table, true)?;
        }
        let model = ["variant", "hull_sprite", "fallback_sprite", "model_number", "formation_flags", "counter"];
        let mut columns = [0; 6];
        for (at, field) in model.into_iter().enumerate() {
            columns[at] = match field {
                "hull_sprite" | "fallback_sprite" => inputs.text(TableId::ShipModels, field)?.column,
                "counter" => inputs.list(TableId::ShipModels, field)?.column,
                "variant" | "formation_flags" => inputs.number::<u64>(TableId::ShipModels, field)?.column,
                _ => inputs.number::<i32>(TableId::ShipModels, field)?.column,
            };
        }
        self.columns = Some(Columns {
            province: inputs.number(TableId::TaskForces, "province")?,
            logical_owner: inputs.number(TableId::TaskForces, "logical_owner")?,
            roster_owner: inputs.number(TableId::TaskForces, "roster_owner")?,
            expeditionary_owner: inputs.number(TableId::TaskForces, "expeditionary_owner")?,
            mission: inputs.number(TableId::TaskForces, "mission")?,
            target: inputs.number(TableId::TaskForceMapMotion, "target")?,
            combat_role: inputs.number(TableId::TaskForceMapMotion, "combat_role")?,
            path: inputs.list(TableId::TaskForceMapMotion, "path")?,
            alternate_path: inputs.list(TableId::TaskForceMapMotion, "alternate_path")?,
            is_in_regions: inputs.number(TableId::TaskForceCounterInputs, "is_in_regions")?,
            bombardment_region: inputs.number(TableId::TaskForceCounterInputs, "bombardment_region")?,
            current_region: inputs.number(TableId::TaskForceMissions, "current_region")?,
            mixed: inputs.list(TableId::ProvinceUnitOrder, "mixed")?,
            task_force: inputs.number(TableId::Ships, "task_force")?,
            class_id: inputs.number(TableId::Ships, "class_id")?,
            model: columns,
            explicit_entity: inputs.text(TableId::VariantDefinitions, "explicit_entity")?,
            tag: inputs.text(TableId::Countries, "tag")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            speed: inputs.number(TableId::Clock, "speed")?,
        });
        Ok(())
    }

    fn viewed(&mut self, _view: &View, _mirrors: &Mirrors) {
        if self.columns.is_some() && self.camera_dirty && self.refused.is_none() {
            self.readmit();
        }
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.columns.is_none() {
            return Ok(());
        }
        let marked = self.marked;
        (self.all_dirty, self.rosters_dirty, self.like_today) = (true, true, false);
        self.refused = Some("composing".into());
        self.settle_dirty(mirrors, &mut Budget::unlimited());
        self.marked = marked;
        Ok(())
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.take_vision(upstream, mirrors, true);
        self.compose(mirrors)
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        self.take_vision(upstream, mirrors, false);
        self.settle_dirty(mirrors, budget);
        // The window's whole settle (the stepped harness) draws every roster of its hour.
        if self.window
            && budget.unlimited_now()
            && let Some(hour) = mirrors.get(TableId::TaskForces).and_then(Mirror::head)
            && self.rosters.wait(hour, Duration::from_secs(30)).is_ok()
        {
            self.settle_dirty(mirrors, budget);
        }
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        self.rosters.applied(step.last());
        if step.gap() || self.columns.is_none() {
            return;
        }
        let columns = self.columns();
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            let Some(mirror) = mirrors.get(table) else { continue };
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.all_dirty = true;
                if matches!(table, TableId::Ships | TableId::ShipModels | TableId::VariantDefinitions | TableId::Countries | TableId::CountryLinks) {
                    self.rosters_dirty = true;
                }
                continue;
            }
            let any = |read: &[usize]| !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty() && read.contains(&written.column()));
            match table {
                TableId::TaskForces => {
                    let read = [columns.province.column, columns.logical_owner.column, columns.roster_owner.column, columns.expeditionary_owner.column, columns.mission.column];
                    let slots: Vec<u32> = changes.deleted().slots().iter().chain(changes.inserted().slots()).copied().chain(changes.columns().iter().filter(|written| read.contains(&written.column())).flat_map(|written| written.slots().iter().copied())).collect();
                    slots.into_iter().for_each(|slot| self.mark_force(slot));
                    let born: Vec<u64> = changes.inserted().keys().unwrap_or_default().iter().map(|&key| key as u64).collect();
                    self.born |= !born.is_empty();
                    born.into_iter().for_each(|id| self.stale_roster(mirrors, id));
                }
                TableId::TaskForceMapMotion => {
                    let read = [columns.target.column, columns.combat_role.column, columns.path.column, columns.alternate_path.column];
                    Self::by_force(changes, mirror, &read).into_iter().for_each(|id| self.mark_force_id(mirrors, id));
                }
                TableId::TaskForceCounterInputs => {
                    let read = [columns.is_in_regions.column, columns.bombardment_region.column];
                    Self::by_force(changes, mirror, &read).into_iter().for_each(|id| self.mark_force_id(mirrors, id));
                }
                TableId::TaskForceMissions => Self::by_force(changes, mirror, &[columns.current_region.column]).into_iter().for_each(|id| self.mark_force_id(mirrors, id)),
                TableId::ProvinceUnitOrder => {
                    for province in Self::by_force(changes, mirror, &[columns.mixed.column]) {
                        let forces = self.in_province.get(&(province as u32)).cloned().unwrap_or_default();
                        forces.into_iter().for_each(|id| self.mark_force_id(mirrors, id));
                    }
                }
                TableId::Ships => {
                    for ship in Self::by_force(changes, mirror, &[columns.task_force.column, columns.class_id.column]) {
                        let now = mirror.slot(u128::from(ship)).and_then(|slot| mirror.number(columns.task_force, slot).value());
                        let was = match now {
                            Some(force) => self.ship_force.insert(ship, force),
                            None => self.ship_force.remove(&ship),
                        };
                        for force in [was, now].into_iter().flatten() {
                            self.stale_roster(mirrors, force);
                        }
                    }
                }
                TableId::ShipModels => {
                    for ship in Self::by_force(changes, mirror, &columns.model) {
                        if let Some(&force) = self.ship_force.get(&ship) {
                            self.stale_roster(mirrors, force);
                        }
                    }
                }
                TableId::VariantDefinitions if any(&[columns.explicit_entity.column]) => self.rosters_dirty = true,
                TableId::Countries if any(&[columns.tag.column]) => self.rosters_dirty = true,
                TableId::CountryLinks if any(&[columns.link_slot.column]) => self.rosters_dirty = true,
                TableId::Clock if any(&[columns.speed.column]) => self.all_dirty = true,
                _ => {}
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.settle_dirty(mirrors, budget);
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        if self.columns.is_none() || self.refused.as_deref().is_some_and(|why| why != "composing") {
            return Carried::default();
        }
        // A roster asked of the worker is the worker's: counted here it held the shown hour.
        let since = [self.dirty.since(), self.figures.since()].into_iter().flatten().min();
        Carried { entities: self.marks(), frames: since.map_or(0, |since| self.frame.saturating_sub(since)) }
    }

    /// While a task force waits for its roster, the newest hour the parts stand for, or nothing
    /// before the first. Otherwise current once the parts stand at the hour applied: while one is
    /// drawn with a roster asked again, or the rosters' rows stand behind the head, the hour they
    /// stand for, made on the rows' cadence. Hidden or refused, it draws nothing and owes nothing.
    fn published(&self) -> Published {
        if self.columns.is_none() || self.refused.as_deref().is_some_and(|why| why != "composing") {
            return Published::Current;
        }
        if self.waiting() > 0 {
            return self.product_hour.map_or(Published::Nothing, Published::At);
        }
        let drawn = self.by_id.values().any(|&slot| self.forces[slot as usize].admitted && self.forces[slot as usize].part.is_some());
        if !drawn {
            return Published::Current;
        }
        let stands = if self.stale() > 0 { self.product_hour } else { self.head.map(|head| self.rosters.answered().map_or(head, |answered| answered.min(head))) };
        match (stands, self.head) {
            (Some(stands), Some(head)) if stands < head => Published::Cadence(stands),
            _ => Published::Current,
        }
    }

    fn held(&self) -> usize {
        let parts: usize = self.forces.iter().filter_map(|held| held.part.as_ref()).map(|part| std::mem::size_of::<ForcePart>() + part.candidate.as_ref().map_or(0, |candidate| candidate.route.capacity() * 4) + part.formation.as_ref().map_or(0, |formation| formation.roster.capacity() * std::mem::size_of::<viewer_world::map::unit_history::naval::Hull>())).sum();
        self.forces.capacity() * std::mem::size_of::<Held>() + parts + self.by_id.len() * 40 + self.ship_force.capacity() * 24 + self.in_province.capacity() * 40 + self.dirty.bytes() + self.figures.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
