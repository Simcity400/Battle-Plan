
use viewer_layer_core::layers::{Carried, Settling, Dirty, Inputs, Layer, List, Num, View};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, NativeValue, Opaque, ShipCounterInputs, TableId};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_counter_kit::type_tint;
use viewer_counters_naval::naval_members::{FleetMarker, ForceFields, NavalJoins, NavalMember, Port, PortSummary, port_force_unrecorded, port_undecided_reason, naval_member};
use viewer_gpu::camera::{Placement, Pose};
use viewer_unit_reads::{counter_inputs, counter_move::UnitRoute, task_force_visual};
use viewer_foundation::{mode_visibility::UnitKinds, perspective::{Choice, Vision}};

pub trait Waters: Send + Sync {
    fn land(&self, province: u32) -> bool;
    fn region(&self, province: u32) -> Option<u32>;
    fn place(&self, province: u32, route: &UnitRoute<'_>) -> Result<task_force_visual::Placement, &'static str>;
    fn port(&self, province: u32) -> Option<[f32; 3]>;
    fn icons_cutoff(&self) -> f32;
    fn admits(&self, placement: Placement, viewport: [u32; 2]) -> Option<Admits>;
}



pub type Admits = Arc<dyn Fn(u32) -> bool + Send + Sync>;

fn bit(bits: &[u64], at: u32) -> bool {
    bits.get(at as usize / 64).is_some_and(|word| word & (1 << (at % 64)) != 0)
}

fn set(bits: &mut Vec<u64>, at: u32) -> bool {
    let word = at as usize / 64;
    if bits.len() <= word {
        bits.resize(word + 1, 0);
    }
    let was = bits[word] & (1 << (at % 64)) == 0;
    bits[word] |= 1 << (at % 64);
    was
}

fn ones(bits: &[u64]) -> impl Iterator<Item = u32> + '_ {
    bits.iter().enumerate().flat_map(|(word, &value)| (0..64).filter(move |at| value & (1 << at) != 0).map(move |at| (word * 64 + at) as u32))
}

fn bytes_of(cell: Cell<&[u8]>) -> &[u8] {
    cell.value().unwrap_or(&[])
}

fn u32s(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes.chunks_exact(4).map(|word| u32::from_le_bytes(word.try_into().expect("four bytes")))
}

fn u64s(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
    bytes.chunks_exact(8).map(|word| u64::from_le_bytes(word.try_into().expect("eight bytes")))
}

fn held(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn whole(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| mirror.status() == Some(Status::Complete))
}

#[derive(Clone, Copy)]
struct Columns {
    fleet: Num<u64>,
    roster_owner: Num<u32>,
    logical_owner: Num<u32>,
    expeditionary_owner: Num<u32>,
    province: Num<u32>,
    mission: Num<u32>,
    combat_references: Num<i32>,
    ship_force: Num<u64>,
    ship_strength: Num<i64>,
    ship_organisation: Num<i64>,
    model_counter: List,
    target: Num<u32>,
    combat_role: Num<u8>,
    path: List,
    alternate_path: List,
    is_in_regions: Num<u8>,
    bombardment_region: Num<u32>,
    current_region: Num<u32>,
    mixed: List,
    naval: List,
    red: Num<u32>,
    green: Num<u32>,
    blue: Num<u32>,
    alpha: Num<u32>,
    fleet_forces: List,
    link_slot: Num<u32>,
    country_slot: Num<u32>,
    peace_conference: Num<u8>,
}

const TABLES: [TableId; 11] = [
    TableId::TaskForces,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForceMapMotion,
    TableId::TaskForceCounterInputs,
    TableId::TaskForceMissions,
    TableId::ProvinceUnitOrder,
    TableId::NavalBases,
    TableId::FleetCounterInputs,
    TableId::CountryLinks,
    TableId::Countries,
];

fn force_fields(forces: &Mirror, slot: u32, columns: &Columns) -> Option<ForceFields> {
    let number32 = |column: Num<u32>| forces.number(column, slot).value();
    Some(ForceFields {
        id: forces.key(slot)? as u64,
        fleet: forces.number(columns.fleet, slot).value()?,
        roster_owner: number32(columns.roster_owner)?,
        logical_owner: number32(columns.logical_owner)?,
        expeditionary_owner: number32(columns.expeditionary_owner)?,
        province: number32(columns.province)?,
        mission: number32(columns.mission)?,
        combat_references: forces.number(columns.combat_references, slot).value()?,
    })
}

fn model_counter(models: &Mirror, slot: u32, columns: &Columns) -> Option<ShipCounterInputs> {
    let Cell::Value(bytes) = models.list(columns.model_counter, slot) else { return None };
    <Option<ShipCounterInputs> as NativeValue>::from_field(Field::Opaque(Opaque::Encoded(bytes, models.format()))).ok().flatten()
}

fn motion_route<'a>(motion: &Mirror, row: u32, columns: &Columns, path: &'a mut Vec<u32>, alternate: &'a mut Vec<u32>) -> UnitRoute<'a> {
    path.clear();
    path.extend(u32s(bytes_of(motion.list(columns.path, row))));
    alternate.clear();
    alternate.extend(u32s(bytes_of(motion.list(columns.alternate_path, row))));
    let combat_role = motion.number(columns.combat_role, row).value().unwrap_or(0);
    UnitRoute {
        class: viewer_runtime::unit_class::CounterClass::TaskForce,
        combat_role,
        combat_count: u32::from(combat_role != 0),
        combat_province: 0,
        target: motion.number(columns.target, row).value().unwrap_or(0),
        naval_transfer: false,
        path_total: path.len() as u32,
        path,
        alternate_total: alternate.len() as u32,
        alternate_path: alternate,
    }
}

struct Tables<'a> {
    forces: Option<&'a Mirror>,
    ships: Option<&'a Mirror>,
    models: Option<&'a Mirror>,
    motion: Option<&'a Mirror>,
    inputs: Option<&'a Mirror>,
    missions: Option<&'a Mirror>,
    order: Option<&'a Mirror>,
    bases: Option<&'a Mirror>,
    links: Option<&'a Mirror>,
}

impl<'a> Tables<'a> {
    fn of(mirrors: &'a Mirrors) -> Self {
        let get = |table| held(mirrors.get(table));
        Self {
            forces: get(TableId::TaskForces),
            ships: get(TableId::Ships),
            models: get(TableId::ShipModels),
            motion: mirrors.get(TableId::TaskForceMapMotion),
            inputs: get(TableId::TaskForceCounterInputs),
            missions: mirrors.get(TableId::TaskForceMissions),
            order: whole(mirrors.get(TableId::ProvinceUnitOrder)),
            bases: get(TableId::NavalBases),
            links: get(TableId::CountryLinks),
        }
    }
}

struct Joins<'a> {
    layer: &'a NavalCounters,
    tables: &'a Tables<'a>,
    columns: Columns,
    vision: &'a Vision,
    relations: &'a type_tint::Index,
}

impl Joins<'_> {
    fn position(&self, list: List, force: u64, province: u32) -> Option<Option<u32>> {
        let order = self.tables.order?;
        let Some(slot) = order.slot(u128::from(province)) else { return Some(None) };
        Some(u64s(bytes_of(order.list(list, slot))).position(|unit| unit == force).map(|at| at as u32))
    }
}

impl NavalJoins for Joins<'_> {
    fn owner(&self, raw: u32) -> Option<u32> {
        let links = self.tables.links?;
        links.number(self.columns.link_slot, links.slot(u128::from(raw))?).value()
    }

    fn mixed(&self, force: u64, province: u32) -> Option<Option<u32>> {
        self.position(self.columns.mixed, force, province)
    }

    fn naval(&self, force: u64, province: u32) -> Option<Option<u32>> {
        self.position(self.columns.naval, force, province)
    }

    fn land(&self, province: u32) -> bool {
        self.layer.waters.land(province)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.layer.waters.region(province)
    }

    fn placement(&self, force: &ForceFields) -> Result<task_force_visual::Placement, &'static str> {
        let motion = whole(self.tables.motion).ok_or("task-force motion census incomplete")?;
        let row = motion.slot(u128::from(force.id)).ok_or("task-force motion row absent")?;
        let (mut path, mut alternate) = (Vec::new(), Vec::new());
        let route = motion_route(motion, row, &self.columns, &mut path, &mut alternate);
        self.layer.waters.place(force.province, &route)
    }

    fn skip(&self, force: &ForceFields, kind: u32, region: Option<u32>) -> Option<bool> {
        let columns = self.columns;
        let inputs = self.tables.inputs.and_then(|inputs| {
            let row = inputs.slot(u128::from(force.id))?;
            Some((inputs.number(columns.is_in_regions, row).value().unwrap_or(0), inputs.number(columns.bombardment_region, row).value().unwrap_or(0)))
        });
        let current = whole(self.tables.missions).map(|missions| missions.slot(u128::from(force.id)).and_then(|row| missions.number(columns.current_region, row).value()).unwrap_or(0));
        task_force_visual::skip_of(kind, inputs, force.mission, current, region)
    }

    fn carries(&self, _force: u64) -> Option<bool> {
        None
    }

    fn ships(&self, force: u64) -> Option<(usize, i64)> {
        let ships = self.tables.ships?;
        let slots = self.layer.ships_of.get(&force).filter(|slots| !slots.is_empty())?;
        let strength = slots.iter().fold(0i64, |sum, &slot| sum.wrapping_add(ships.number(self.columns.ship_strength, slot).value().unwrap_or(0)));
        Some((slots.len(), strength))
    }

    fn ships_complete(&self) -> bool {
        whole(self.tables.ships).is_some()
    }

    fn ratio(&self, force: u64) -> Option<(i64, i64)> {
        if !self.layer.shipping {
            return None;
        }
        let (ships, models) = (self.tables.ships?, self.tables.models?);
        let columns = self.columns;
        let mut operands = Vec::new();
        for &slot in self.layer.ships_of.get(&force).into_iter().flatten() {
            let counter = models.slot(ships.key(slot)?).and_then(|model| model_counter(models, model, &columns))?;
            let number = |column: Num<i64>| ships.number(column, slot).value().unwrap_or(0);
            operands.push([number(columns.ship_strength), counter.maximum_strength, number(columns.ship_organisation), counter.maximum_organisation]);
        }
        Some((counter_inputs::strength_ratio(operands.iter().map(|ship| (ship[0], ship[1]))), counter_inputs::organisation_ratio(operands.iter().map(|ship| (ship[2], ship[3])))))
    }

    fn vision(&self) -> &Vision {
        self.vision
    }

    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side> {
        self.relations.of(roster, logical)
    }

    fn docked(&self, force: u64) -> Result<bool, &'static str> {
        let motion = whole(self.tables.motion).ok_or("task-force motion census incomplete")?;
        let row = motion.slot(u128::from(force)).ok_or("task-force motion row absent")?;
        Ok(bytes_of(motion.list(self.columns.path, row)).is_empty())
    }

    fn base(&self, province: u32) -> bool {
        self.tables.bases.is_some_and(|bases| bases.slot(u128::from(province)).is_some())
    }
}

pub struct NavalCounters {
    waters: Arc<dyn Waters>,
    columns: Option<Columns>,
    refused: Option<String>,
    members: Vec<Option<NavalMember>>,
    in_province: HashMap<u32, Vec<u32>>,
    forces: Dirty,
    ship_rows: Vec<(u64, u64)>,
    ships_of: HashMap<u64, Vec<u32>>,
    ship_slots: Dirty,
    shipping: bool,
    shipping_dirty: bool,
    fleet_rows: Vec<(u64, u64)>,
    heads: HashMap<u64, Vec<u64>>,
    fleets: BTreeMap<u64, FleetMarker>,
    fleet_slots: Dirty,
    ports: PortSummary,
    bases: HashSet<u32>,
    port_provinces: HashSet<u32>,
    peace: bool,
    peace_dirty: bool,
    all: bool,
    hidden: bool,
    vision: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision_new: AtomicBool,
    relations: Mutex<Option<Arc<type_tint::Index>>>,
    relations_new: AtomicBool,
    statuses: HashMap<usize, Status>,
    frame: u64,
    marked: usize,
    camera: Option<(Placement, [u32; 2])>,
    camera_dirty: bool,
    admits: Option<Admits>,
    asked: Vec<u64>,
    admitted: Vec<u64>,
    out: Vec<u64>,
    outside: usize,
}

impl NavalCounters {
    pub fn new(waters: Arc<dyn Waters>) -> Self {
        Self {
            waters,
            columns: None,
            refused: Some("not composed yet".into()),
            members: Vec::new(),
            in_province: HashMap::new(),
            forces: Dirty::default(),
            ship_rows: Vec::new(),
            ships_of: HashMap::new(),
            ship_slots: Dirty::default(),
            shipping: false,
            shipping_dirty: true,
            fleet_rows: Vec::new(),
            heads: HashMap::new(),
            fleets: BTreeMap::new(),
            fleet_slots: Dirty::default(),
            ports: PortSummary::default(),
            bases: HashSet::new(),
            port_provinces: HashSet::new(),
            peace: false,
            peace_dirty: true,
            all: true,
            hidden: false,
            vision: Mutex::new(None),
            vision_new: AtomicBool::new(false),
            relations: Mutex::new(None),
            relations_new: AtomicBool::new(false),
            statuses: HashMap::new(),
            frame: 0,
            marked: 0,
            camera: None,
            camera_dirty: false,
            admits: None,
            asked: Vec::new(),
            admitted: Vec::new(),
            out: Vec::new(),
            outside: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        let fresh = Self { columns: self.columns, hidden: self.hidden, camera: self.camera, admits: self.admits.clone(), ..Self::new(Arc::clone(&self.waters)) };
        if let Some((choice, vision)) = self.perspective() {
            fresh.set_vision(choice, vision);
        }
        if let Some(relations) = self.relations() {
            fresh.set_relations(relations);
        }
        fresh
    }

    pub fn set_relations(&self, relations: Arc<type_tint::Index>) {
        let mut held = self.relations.lock().unwrap_or_else(|error| error.into_inner());
        if held.as_ref().is_none_or(|held| !Arc::ptr_eq(held, &relations)) {
            self.relations_new.store(true, Ordering::Release);
        }
        *held = Some(relations);
    }

    fn relations(&self) -> Option<Arc<type_tint::Index>> {
        self.relations.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        let mut held = self.vision.lock().unwrap_or_else(|error| error.into_inner());
        if held.as_ref().is_none_or(|(_, held)| !Arc::ptr_eq(held, &vision) && **held != *vision) {
            self.vision_new.store(true, Ordering::Release);
        }
        *held = Some((choice, vision));
    }

    pub fn perspective(&self) -> Option<(Choice, Arc<Vision>)> {
        self.vision.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the naval counters layer is declared before it composes")
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    pub fn members(&self) -> impl Iterator<Item = &NavalMember> + '_ {
        self.members.iter().flatten()
    }

    pub fn fleets(&self) -> &BTreeMap<u64, FleetMarker> {
        &self.fleets
    }

    pub fn ports(&self) -> &PortSummary {
        &self.ports
    }

    pub fn peace(&self) -> bool {
        self.peace
    }

    pub fn marked(&self) -> usize {
        self.marked
    }

    pub fn admitted(&self, province: u32) -> bool {
        self.admits.as_ref().is_none_or(|admits| admits(province))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn admit_only(&mut self, admits: impl Fn(u32) -> bool + Send + Sync + 'static) {
        self.admits = Some(Arc::new(admits));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn outside(&self) -> usize {
        self.outside
    }

    fn admits(&mut self, province: u32) -> bool {
        let Some(admits) = &self.admits else { return true };
        if set(&mut self.asked, province) && admits(province) {
            set(&mut self.admitted, province);
        }
        bit(&self.admitted, province)
    }

    fn readmit(&mut self, mirrors: &Mirrors) {
        self.camera_dirty = false;
        let admits = self.camera.and_then(|(placement, viewport)| self.waters.admits(placement, viewport));
        self.admits = admits;
        self.asked.clear();
        self.admitted.clear();
        let out: Vec<u32> = ones(&self.out).collect();
        self.out.clear();
        self.outside = 0;
        let forces = mirrors.get(TableId::TaskForces);
        out.into_iter().for_each(|slot| self.mark_force(forces, slot));
    }

    fn core(&self, mirrors: &Mirrors) -> Result<(Arc<Vision>, Arc<type_tint::Index>), String> {
        let (_, vision) = self.perspective().ok_or("the viewing country's vision is not set")?;
        let relations = self.relations().ok_or("the viewing country's type-icon relations are not set")?;
        for table in [TableId::CountryLinks, TableId::Countries] {
            let mirror = mirrors.get(table).ok_or_else(|| format!("{table:?} is not read yet"))?;
            if let Some(why) = mirror.not_recorded() {
                return Err(format!("{why}; the naval counters need every country link and country"));
            }
        }
        let forces = mirrors.get(TableId::TaskForces).ok_or("TaskForces is not read yet")?;
        if forces.status().is_none() {
            return Err("TaskForces is not recorded here: a gap in the recording".into());
        }
        Ok((vision, relations))
    }

    fn mark_force(&mut self, forces: Option<&Mirror>, slot: u32) {
        let inside = self.admits.is_none() || {
            let standing = self.members.get(slot as usize).and_then(Option::as_ref).map(|member| member.fields.province);
            let columns = self.columns();
            let now = forces.filter(|forces| forces.live(slot)).and_then(|forces| forces.number(columns.province, slot).value());
            [standing, now].into_iter().flatten().any(|province| self.admits(province) || self.based(province))
        };
        if inside {
            if self.forces.mark(slot, self.frame) {
                self.marked += 1;
            }
        } else if set(&mut self.out, slot) {
            self.outside += 1;
            self.marked += 1;
        }
    }

    fn mark_force_key(&mut self, mirrors: &Mirrors, id: u64) {
        let forces = mirrors.get(TableId::TaskForces);
        if let Some(slot) = forces.and_then(|forces| forces.slot(u128::from(id))) {
            self.mark_force(forces, slot);
        }
    }

    fn based(&self, province: u32) -> bool {
        self.bases.contains(&province)
    }

    fn mark_all(&mut self, mirrors: &Mirrors) {
        let forces = mirrors.get(TableId::TaskForces);
        let mut slots: Vec<u32> = forces.map(|forces| forces.rows().collect()).unwrap_or_default();
        slots.extend(self.members.iter().enumerate().filter(|(_, member)| member.is_some()).map(|(slot, _)| slot as u32));
        slots.into_iter().for_each(|slot| self.mark_force(forces, slot));
    }

    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.waters));
        let kept = std::mem::replace(self, empty);
        (self.columns, self.hidden, self.frame, self.marked) = (kept.columns, kept.hidden, kept.frame, kept.marked);
        (self.camera, self.camera_dirty, self.admits) = (kept.camera, kept.camera_dirty, kept.admits);
        (self.vision, self.vision_new, self.relations, self.relations_new) = (kept.vision, kept.vision_new, kept.relations, kept.relations_new);
    }

    fn recompose(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.members.clear();
        self.in_province.clear();
        self.forces.clear();
        self.ship_rows.clear();
        self.ships_of.clear();
        self.ship_slots.clear();
        self.fleet_rows.clear();
        self.heads.clear();
        self.fleets.clear();
        self.fleet_slots.clear();
        self.port_provinces.clear();
        self.out.clear();
        self.outside = 0;
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        let mut bases: Vec<u32> = held(mirrors.get(TableId::NavalBases)).map(|bases| bases.rows().filter_map(|slot| bases.key(slot)).map(|key| key as u32).collect()).unwrap_or_default();
        bases.sort_unstable();
        let waters = Arc::clone(&self.waters);
        let (mut anchors, mut unanchored) = (Vec::new(), 0);
        for &province in &bases {
            match waters.port(province) {
                Some(anchor) => anchors.push((province, anchor.map(f32::to_bits))),
                None => unanchored += 1,
            }
        }
        self.ports = PortSummary { anchors, unanchored, undecided: BTreeMap::new() };
        self.bases = bases.into_iter().collect();
        self.port_provinces = self.bases.clone();
        if let Some(ships) = held(mirrors.get(TableId::Ships)) {
            for slot in ships.rows() {
                let row = (ships.key(slot).map_or(0, |key| key as u64), ships.number(columns.ship_force, slot).value().unwrap_or(0));
                self.ship_row(slot, row);
            }
        }
        self.shipping_dirty = true;
        self.peace_dirty = true;
        if let Some(fleets) = held(mirrors.get(TableId::FleetCounterInputs)) {
            fleets.rows().for_each(|slot| {
                self.fleet_slots.mark(slot, self.frame);
            });
        }
        if self.camera_dirty {
            self.readmit(mirrors);
        }
        self.mark_all(mirrors);
    }

    fn ship_row(&mut self, slot: u32, row: (u64, u64)) -> Option<(u64, u64)> {
        let at = slot as usize;
        if self.ship_rows.len() <= at {
            self.ship_rows.resize(at + 1, (0, 0));
        }
        let old = self.ship_rows[at];
        if old == row {
            return None;
        }
        if old.0 != 0
            && let Some(slots) = self.ships_of.get_mut(&old.1)
        {
            slots.retain(|&held| held != slot);
            if slots.is_empty() {
                self.ships_of.remove(&old.1);
            }
        }
        if row.0 != 0 {
            self.ships_of.entry(row.1).or_default().push(slot);
        }
        self.ship_rows[at] = row;
        Some(old)
    }

    fn shipping_whole(&self, mirrors: &Mirrors) -> bool {
        let columns = self.columns();
        let (Some(ships), Some(models)) = (whole(mirrors.get(TableId::Ships)), whole(mirrors.get(TableId::ShipModels))) else { return false };
        models.rows().all(|slot| model_counter(models, slot, &columns).is_some()) && ships.rows().all(|slot| ships.key(slot).is_some_and(|key| models.slot(key).is_some()))
    }

    fn settle_fleet(&mut self, mirrors: &Mirrors, slot: u32) {
        let columns = self.columns();
        let at = slot as usize;
        if self.fleet_rows.len() <= at {
            self.fleet_rows.resize(at + 1, (0, 0));
        }
        let (fleet, first) = self.fleet_rows[at];
        if fleet != 0 {
            self.fleets.remove(&fleet);
            if let Some(heads) = self.heads.get_mut(&first) {
                heads.retain(|&held| held != fleet);
                if heads.is_empty() {
                    self.heads.remove(&first);
                }
            }
        }
        self.fleet_rows[at] = (0, 0);
        let Some(rows) = held(mirrors.get(TableId::FleetCounterInputs)).filter(|rows| rows.live(slot)) else { return };
        let Some(fleet) = rows.key(slot).map(|key| key as u64) else { return };
        let first = u64s(bytes_of(rows.list(columns.fleet_forces, slot))).next();
        let forces = held(mirrors.get(TableId::TaskForces));
        let mission = |force: u64| forces.and_then(|forces| forces.number(columns.mission, forces.slot(u128::from(force))?).value());
        let colour = [columns.red, columns.green, columns.blue, columns.alpha].map(|column| rows.number(column, slot).value().unwrap_or(0));
        self.fleets.insert(fleet, FleetMarker { reserve: counter_inputs::reserve_of(first, mission), colour });
        if let Some(first) = first {
            self.heads.entry(first).or_default().push(fleet);
        }
        self.fleet_rows[at] = (fleet, first.unwrap_or(0));
    }

    fn mark_heads(&mut self, mirrors: &Mirrors, force: u64) {
        let Some(fleets) = self.heads.get(&force) else { return };
        let rows = mirrors.get(TableId::FleetCounterInputs);
        for &fleet in fleets {
            if let Some(slot) = rows.and_then(|rows| rows.slot(u128::from(fleet))) {
                self.fleet_slots.mark(slot, self.frame);
            }
        }
    }

    fn settle_force(&mut self, tables: &Tables<'_>, seen: (&Vision, &type_tint::Index), slot: u32) {
        let at = slot as usize;
        if self.members.len() <= at {
            self.members.resize(at + 1, None);
        }
        if let Some(old) = self.members[at].take() {
            let province = old.fields.province;
            if let Some(slots) = self.in_province.get_mut(&province) {
                slots.retain(|&held| held != slot);
                if slots.is_empty() {
                    self.in_province.remove(&province);
                }
            }
            if self.based(province) {
                self.port_provinces.insert(province);
            }
        }
        let columns = self.columns();
        let Some(forces) = tables.forces.filter(|forces| forces.live(slot)) else { return };
        let Some(fields) = force_fields(forces, slot, &columns) else { return };
        let member = naval_member(fields, UnitKinds::All, &Joins { layer: self, tables, columns, vision: seen.0, relations: seen.1 });
        self.in_province.entry(fields.province).or_default().push(slot);
        if self.based(fields.province) {
            self.port_provinces.insert(fields.province);
        }
        self.members[at] = Some(member);
    }

    fn settle_port(&mut self, mirrors: &Mirrors, province: u32) {
        let columns = self.columns();
        let mut reason = self
            .in_province
            .get(&province)
            .into_iter()
            .flatten()
            .filter_map(|&slot| self.members[slot as usize].as_ref())
            .filter_map(|member| match member.port {
                Port::Undecided(why) => Some((member.fields.id, why)),
                _ => None,
            })
            .max_by_key(|&(id, _)| id)
            .map(|(_, why)| port_undecided_reason(why));
        let forces = held(mirrors.get(TableId::TaskForces));
        if let Some(order) = whole(mirrors.get(TableId::ProvinceUnitOrder))
            && !forces.is_some_and(Mirror::complete)
            && let Some(row) = order.slot(u128::from(province))
            && u64s(bytes_of(order.list(columns.naval, row))).any(|force| forces.is_none_or(|forces| forces.slot(u128::from(force)).is_none()))
        {
            reason = Some(port_force_unrecorded());
        }
        match reason {
            Some(reason) if self.based(province) => {
                self.ports.undecided.insert(province, reason);
            }
            _ => {
                self.ports.undecided.remove(&province);
            }
        }
    }

    fn peace_of(&self, mirrors: &Mirrors, ours: u32) -> bool {
        let columns = self.columns();
        held(mirrors.get(TableId::Countries)).is_some_and(|countries| {
            countries.rows().any(|slot| countries.number(columns.country_slot, slot).value() == Some(ours) && countries.number(columns.peace_conference, slot).value().unwrap_or(0) != 0)
        })
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        let core = if self.hidden { Err("not drawn at this view".to_owned()) } else { self.core(mirrors) };
        let (vision, relations) = match core {
            Ok(seen) => seen,
            Err(why) => {
                if self.refused.as_ref() != Some(&why) {
                    self.release();
                    self.refused = Some(why);
                }
                self.all = false;
                return;
            }
        };
        if self.refused.take().is_some() || self.all {
            self.all = false;
            self.vision_new.store(false, Ordering::Release);
            self.relations_new.store(false, Ordering::Release);
            self.recompose(mirrors);
        }
        if self.camera_dirty {
            self.readmit(mirrors);
        }
        let relations_new = self.relations_new.swap(false, Ordering::AcqRel);
        if self.vision_new.swap(false, Ordering::AcqRel) | relations_new {
            self.peace_dirty = true;
            self.mark_all(mirrors);
        }
        let columns = self.columns();
        let mut dirty = std::mem::take(&mut self.ship_slots);
        let ships = held(mirrors.get(TableId::Ships));
        let _ = dirty.settle(|slot| {
            let row = match ships.filter(|ships| ships.live(slot)) {
                Some(ships) => (ships.key(slot).map_or(0, |key| key as u64), ships.number(columns.ship_force, slot).value().unwrap_or(0)),
                None => (0, 0),
            };
            let old = self.ship_row(slot, row).unwrap_or(row);
            for force in [old.1, row.1] {
                self.mark_force_key(mirrors, force);
            }
            Ok(())
        });
        self.ship_slots = dirty;
        if std::mem::take(&mut self.shipping_dirty) {
            let now = self.shipping_whole(mirrors);
            if now != self.shipping {
                self.shipping = now;
                self.mark_all(mirrors);
            }
        }
        let mut dirty = std::mem::take(&mut self.fleet_slots);
        let _ = dirty.settle(|slot| {
            self.settle_fleet(mirrors, slot);
            Ok(())
        });
        self.fleet_slots = dirty;
        if std::mem::take(&mut self.peace_dirty) {
            self.peace = self.peace_of(mirrors, vision.ours());
        }
        let tables = Tables::of(mirrors);
        let mut dirty = std::mem::take(&mut self.forces);
        let _ = dirty.settle(|slot| {
            self.settle_force(&tables, (&vision, &relations), slot);
            Ok(())
        });
        self.forces = dirty;
        for province in std::mem::take(&mut self.port_provinces) {
            self.settle_port(mirrors, province);
        }
    }

    fn apply_table(&mut self, table: TableId, changes: &Changes, mirrors: &Mirrors, columns: Columns) {
        let Some(mirror) = mirrors.get(table) else { return };
        let keys = |changes: &Changes| -> Vec<u128> {
            let mut keys: Vec<u128> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).copied().collect();
            keys.extend(changes.columns().iter().flat_map(|written| written.slots()).filter_map(|&slot| mirror.key(slot)));
            keys
        };
        let slots = |changes: &Changes| -> Vec<u32> {
            let mut slots: Vec<u32> = changes.deleted().slots().iter().chain(changes.inserted().slots()).copied().collect();
            slots.extend(changes.columns().iter().flat_map(|written| written.slots()));
            slots
        };
        let membership = !changes.deleted().is_empty() || !changes.inserted().is_empty();
        match table {
            TableId::TaskForces => {
                let forces = Some(mirror);
                slots(changes).into_iter().for_each(|slot| self.mark_force(forces, slot));
                let mut heads: Vec<u128> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).copied().collect();
                heads.extend(changes.columns().iter().filter(|written| written.column() == columns.mission.column).flat_map(|written| written.slots()).filter_map(|&slot| mirror.key(slot)));
                heads.into_iter().for_each(|force| self.mark_heads(mirrors, force as u64));
            }
            TableId::TaskForceMapMotion | TableId::TaskForceCounterInputs | TableId::TaskForceMissions => keys(changes).into_iter().for_each(|force| self.mark_force_key(mirrors, force as u64)),
            TableId::Ships => {
                self.shipping_dirty |= membership;
                slots(changes).into_iter().for_each(|slot| {
                    self.ship_slots.mark(slot, self.frame);
                });
            }
            TableId::ShipModels => {
                self.shipping_dirty = true;
                let ships = mirrors.get(TableId::Ships);
                for ship in keys(changes) {
                    let force = ships.and_then(|ships| ships.slot(ship)).and_then(|slot| self.ship_rows.get(slot as usize)).map(|&(_, force)| force);
                    if let Some(force) = force {
                        self.mark_force_key(mirrors, force);
                    }
                }
            }
            TableId::ProvinceUnitOrder => {
                for province in keys(changes).into_iter().map(|key| key as u32) {
                    let slots = self.in_province.get(&province).cloned().unwrap_or_default();
                    let forces = mirrors.get(TableId::TaskForces);
                    slots.into_iter().for_each(|slot| self.mark_force(forces, slot));
                    if self.based(province) {
                        self.port_provinces.insert(province);
                    }
                }
            }
            TableId::NavalBases if membership => self.all = true,
            TableId::FleetCounterInputs => slots(changes).into_iter().for_each(|slot| {
                self.fleet_slots.mark(slot, self.frame);
            }),
            TableId::CountryLinks if membership || changes.columns().iter().any(|written| written.column() == columns.link_slot.column && !written.is_empty()) => self.all = true,
            TableId::Countries if membership || changes.columns().iter().any(|written| !written.is_empty()) => self.peace_dirty = true,
            _ => {}
        }
    }

    pub fn compared(&self, member: &NavalMember) -> bool {
        self.admitted(member.fields.province) || self.based(member.fields.province)
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
        let mine: BTreeMap<u64, &NavalMember> = self.members().filter(|member| self.compared(member)).map(|member| (member.fields.id, member)).collect();
        let theirs: BTreeMap<u64, &NavalMember> = other.members().filter(|member| other.compared(member)).map(|member| (member.fields.id, member)).collect();
        for id in mine.keys().chain(theirs.keys()).collect::<std::collections::BTreeSet<_>>() {
            if mine.get(id) != theirs.get(id) {
                note(format!("task force {id}: {:?} against {:?}", mine.get(id), theirs.get(id)));
            }
        }
        if self.fleets != other.fleets {
            note("fleets differ".into());
        }
        if self.ports != other.ports {
            note(format!("ports: {:?} against {:?}", self.ports, other.ports));
        }
        if self.peace != other.peace {
            note("peace conference differs".into());
        }
        (differing, first)
    }
}

impl Layer for NavalCounters {
    fn name(&self) -> &'static str {
        "naval_counters"
    }

    fn placed(&mut self, view: &viewer_layer_core::layers::View) -> Option<u64> {
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        Some(u64::from(view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.waters.icons_cutoff())))
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        let shown = viewer_map_modes::map_mode::visibility::of(&view.map_mode);
        let high = view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.waters.icons_cutoff());
        let drawn = view.overlay.counters.naval || view.overlay.counters.ports;
        if !shown.unit_counters || shown.unit_kinds != UnitKinds::All || !drawn || high {
            self.hidden = true;
            return Ok(());
        }
        for table in TABLES {
            inputs.table(table, true)?;
        }
        use TableId as T;
        self.columns = Some(Columns {
            fleet: inputs.number(T::TaskForces, "fleet")?,
            roster_owner: inputs.number(T::TaskForces, "roster_owner")?,
            logical_owner: inputs.number(T::TaskForces, "logical_owner")?,
            expeditionary_owner: inputs.number(T::TaskForces, "expeditionary_owner")?,
            province: inputs.number(T::TaskForces, "province")?,
            mission: inputs.number(T::TaskForces, "mission")?,
            combat_references: inputs.number(T::TaskForces, "combat_references")?,
            ship_force: inputs.number(T::Ships, "task_force")?,
            ship_strength: inputs.number(T::Ships, "strength")?,
            ship_organisation: inputs.number(T::Ships, "organisation")?,
            model_counter: inputs.list(T::ShipModels, "counter")?,
            target: inputs.number(T::TaskForceMapMotion, "target")?,
            combat_role: inputs.number(T::TaskForceMapMotion, "combat_role")?,
            path: inputs.list(T::TaskForceMapMotion, "path")?,
            alternate_path: inputs.list(T::TaskForceMapMotion, "alternate_path")?,
            is_in_regions: inputs.number(T::TaskForceCounterInputs, "is_in_regions")?,
            bombardment_region: inputs.number(T::TaskForceCounterInputs, "bombardment_region")?,
            current_region: inputs.number(T::TaskForceMissions, "current_region")?,
            mixed: inputs.list(T::ProvinceUnitOrder, "mixed")?,
            naval: inputs.list(T::ProvinceUnitOrder, "naval")?,
            red: inputs.number(T::FleetCounterInputs, "red")?,
            green: inputs.number(T::FleetCounterInputs, "green")?,
            blue: inputs.number(T::FleetCounterInputs, "blue")?,
            alpha: inputs.number(T::FleetCounterInputs, "alpha")?,
            fleet_forces: inputs.list(T::FleetCounterInputs, "task_forces")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
            country_slot: inputs.number(T::Countries, "canonical_slot")?,
            peace_conference: inputs.number(T::Countries, "peace_conference_participant")?,
        });
        if std::mem::take(&mut self.hidden) {
            self.all = true;
        }
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.all = true;
        self.settle_dirty(mirrors);
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() || self.hidden || self.refused.is_some() {
            return;
        }
        let columns = self.columns();
        for changes in step.tables() {
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.all = true;
                continue;
            }
            self.apply_table(TableId::ALL[changes.table()], changes, mirrors, columns);
        }
    }

    fn viewed(&mut self, _view: &View, mirrors: &Mirrors) {
        if self.columns.is_some() && self.camera_dirty && self.refused.is_none() && !self.hidden {
            self.readmit(mirrors);
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }


    fn carried(&self) -> Carried {
        let entities = self.forces.len() + self.ship_slots.len() + self.fleet_slots.len() + usize::from(self.all);
        Carried { entities }
    }

    fn held(&self) -> usize {
        self.members.capacity() * std::mem::size_of::<Option<NavalMember>>()
            + self.in_province.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + self.ship_rows.capacity() * 16
            + self.ships_of.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + self.fleet_rows.capacity() * 16
            + self.heads.values().map(|fleets| fleets.capacity() * 8 + 40).sum::<usize>()
            + self.fleets.len() * (8 + std::mem::size_of::<FleetMarker>() + 16)
            + self.ports.anchors.capacity() * 16
            + self.ports.undecided.values().map(|why| why.capacity() + 40).sum::<usize>()
            + (self.bases.capacity() + self.port_provinces.capacity()) * 8
            + self.forces.bytes()
            + self.ship_slots.bytes()
            + self.fleet_slots.bytes()
            + (self.out.capacity() + self.asked.capacity() + self.admitted.capacity()) * 8
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
