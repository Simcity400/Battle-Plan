use viewer_layer_core::counter_rows::{Row, Rows};

use viewer_layer_core::{layers::{Carried, Change, Settling, Dirty, Gpu, Inputs, Layer, List, Num, Text, Upstream, View}, vision::{SightCopy, Vision as VisionLayer}};
use super::relations::{self, Relations};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::rows::{DivisionPanelStatsRow, DivisionStatusRow, TableId};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_counter_kit::type_tint;
use viewer_counters::counters::CounterContent;
use viewer_counters_land::land::{DivisionStatus, StatusFields};
use viewer_counters_land::members::{DivisionFields, LandStacks, Member, MemberJoins, division_kind, division_member, member_orders, member_strength};
use viewer_unit_reads::{
    counter_inputs::{AssignedOrders, GroupFacts, OrderFacts},
    counter_move::UnitRoute,
    counter_action::Action,
};
use viewer_gpu::{camera::{MapDefines, MapSize, Placement, Pose}, province_admission::Admission};
use viewer_foundation::{
    mode_visibility::UnitKinds,
    perspective::{Choice, SharingUnknown, Sight, Vision},
    movement_timing::MovementContent,
};
use viewer_runtime::unit_class::CounterClass;

pub type Source = (u32, u32);

pub struct Content {
    counters: Option<Arc<CounterContent>>,
    movement: Option<Arc<MovementContent>>,
    map: Option<(MapSize, MapDefines)>,
    tables: Option<engine_executable::executable_tables::Tables>,
}

impl Content {
    pub fn new(counters: Arc<CounterContent>, movement: Option<Arc<MovementContent>>, map: (MapSize, MapDefines), tables: Option<engine_executable::executable_tables::Tables>) -> Self {
        Self { counters: Some(counters), movement, map: Some(map), tables }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn bare() -> Self {
        Self { counters: None, movement: None, map: None, tables: engine_executable::executable_tables::current() }
    }

    fn scoped<T>(&self, work: impl FnOnce() -> T) -> T {
        match self.tables {
            Some(tables) => engine_executable::executable_tables::scope(Some(tables), work),
            None => work(),
        }
    }

    fn icons_cutoff(&self) -> f32 {
        self.counters.as_ref().map_or(f32::INFINITY, |counters| counters.defines.icons_cutoff)
    }
}

type Admits = Arc<dyn Fn(u32) -> bool + Send + Sync>;

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

#[derive(Clone, Copy)]
struct Columns {
    roster_owner: Num<u32>,
    logical_owner: Num<u32>,
    expeditionary_owner: Num<u32>,
    province: Num<u32>,
    template: Num<u64>,
    hit_points: Num<i64>,
    organisation: Num<i64>,
    orders_group: Num<u64>,
    link_slot: Num<u32>,
    mixed: List,
    separate_counter: Num<u8>,
    alternate_template: Num<u64>,
    target: Num<u32>,
    naval_transfer: Num<u8>,
    combat_count: Num<u32>,
    path_total: Num<u32>,
    path: List,
    alternate_total: Num<u32>,
    alternate_path: List,
    combat_role: Num<u8>,
    combat_province: Num<u32>,
    entrenchment: Num<i64>,
    entrenchment_cap: Num<i64>,
    supply_ratio: Num<i64>,
    attrition_value: Num<i64>,
    attrition_threshold: Num<i64>,
    exercising: Num<u8>,
    has_order: Num<u8>,
    invalid_orders: Num<u8>,
    strength_factor: Num<i64>,
    supply_ratio_exact: Num<i64>,
    panel_hit_points: Num<i64>,
    panel_organisation: Num<i64>,
    status_hit_points: Num<i64>,
    status_organisation: Num<i64>,
    group_parent: Num<u64>,
    army_group: Num<u8>,
    leader: Num<u64>,
    hq_provider: Num<u64>,
    primary_orders: List,
    fallback_orders: List,
    has_child: Num<u8>,
    assigned: List,
    order_type: Num<i32>,
    target_states: List,
    withdrawing: Num<u8>,
    action: action::Columns,
    definition_token: Num<u32>,
    literal: Text,
}

pub const PHASES: [&str; 11] = ["groups", "orders", "plans", "arrays", "members", "whole members", "strength", "orders only", "outside", "memberships", "rows"];

const FULL: u8 = 1;
const STRENGTH: u8 = 2;
const ORDERS: u8 = 4;
const ACTION: u8 = 8;

pub const TABLES: [TableId; 15] = [
    TableId::Divisions,
    TableId::CountryLinks,
    TableId::Countries,
    TableId::ProvinceUnitOrder,
    TableId::ArmyIconInputs,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::OrdersGroups,
    TableId::OrdersGroupPlans,
    TableId::ArmyOrderPlans,
    TableId::ArmyOrders,
    TableId::OrdersGroupCounterInputs,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
];

#[cfg(any(test, feature = "test-support"))]
pub fn declared() -> impl Iterator<Item = TableId> {
    TABLES.into_iter().chain(action::TABLES)
}

const BY_DIVISION: [TableId; 4] = [TableId::DivisionStatus, TableId::DivisionPanelStats, TableId::DivisionMovement, TableId::ArmyIconInputs];



pub(super) fn key_of(mirror: Option<&Mirror>, row: Row) -> Option<u128> {
    match row {
        Row::Gone(key) => Some(key),
        Row::At(slot) => mirror?.key(slot),
    }
}

fn held(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn native(mirror: &Mirror, column: Num<i64>, slot: u32) -> Cell<i64> {
    match (mirror.number(column, slot), column.table.display_step(column.column)) {
        (Cell::Value(units), Some(step)) => step.value(units).map_or(Cell::Unrecorded, Cell::Value),
        (cell, _) => cell,
    }
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

pub struct Counters {
    content: Arc<Content>,
    columns: Option<Columns>,
    refused: Option<String>,
    members: Vec<Option<(Source, Member)>>,
    sources: BTreeMap<Source, Vec<u32>>,
    by_group: HashMap<u64, Vec<u32>>,
    children: HashMap<u64, Vec<u64>>,
    group_rows: Vec<[u64; 3]>,
    hq: HashMap<u64, u32>,
    area_rows: Vec<(u64, bool)>,
    area: HashMap<u64, u32>,
    memberships: HashMap<u64, [AssignedOrders; 2]>,
    divisions: Dirty,
    causes: Vec<u8>,
    division_rows: [Rows; 4],
    order_rows: Rows,
    plan_rows: [Rows; 2],
    input_rows: Rows,
    destination_rows: Rows,
    transfers_new: bool,
    standing_new: bool,
    layout_new: bool,
    provinces: Dirty,
    group_slots: Dirty,
    area_slots: Dirty,
    plans: HashSet<u64>,
    all: bool,
    strategic_layout: bool,
    hidden: bool,
    vision: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision_new: AtomicBool,
    reads_layer: bool,
    sight: Option<(Choice, u64)>,
    readers: HashMap<u32, Vec<u32>>,
    read_at: Vec<u32>,
    relations: Option<Arc<type_tint::Index>>,
    relations_new: bool,
    #[cfg(any(test, feature = "test-support"))]
    given: bool,
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
    out_provinces: Vec<u64>,
    rows_of: [Vec<u32>; 4],
    stale: HashSet<u64>,
    scratch: Vec<u32>,
    phases: [(u64, u64); PHASES.len()],
    published: Arc<LandStacks>,
    touched: BTreeSet<Source>,
    actions: Vec<Action>,
    reads: Vec<action::Read>,
    by_destination: HashMap<u32, Vec<u32>>,
    transferring: HashSet<u32>,
    standing: HashSet<u32>,
    wars: Option<Arc<relations::Facts>>,
    wars_new: bool,
    #[cfg(any(test, feature = "test-support"))]
    applied_reads: u64,
    #[cfg(any(test, feature = "test-support"))]
    validated: [std::sync::atomic::AtomicUsize; 2],
}

struct Tables<'a> {
    divisions: Option<&'a Mirror>,
    links: Option<&'a Mirror>,
    order: Option<&'a Mirror>,
    icons: Option<&'a Mirror>,
    movement: Option<&'a Mirror>,
    status: Option<&'a Mirror>,
    panels: Option<&'a Mirror>,
    groups: Option<&'a Mirror>,
    group_plans: Option<&'a Mirror>,
    plans: Option<&'a Mirror>,
    orders: Option<&'a Mirror>,
    group_inputs: Option<&'a Mirror>,
    orders_complete: bool,
}

impl<'a> Tables<'a> {
    fn of(mirrors: &'a Mirrors) -> Self {
        let get = |table| held(mirrors.get(table));
        Self {
            divisions: get(TableId::Divisions),
            links: get(TableId::CountryLinks),
            order: mirrors.get(TableId::ProvinceUnitOrder).filter(|mirror| mirror.complete()),
            icons: get(TableId::ArmyIconInputs),
            movement: get(TableId::DivisionMovement),
            status: get(TableId::DivisionStatus),
            panels: get(TableId::DivisionPanelStats),
            groups: get(TableId::OrdersGroups),
            group_plans: get(TableId::OrdersGroupPlans),
            plans: get(TableId::ArmyOrderPlans),
            orders: get(TableId::ArmyOrders),
            group_inputs: get(TableId::OrdersGroupCounterInputs),
            orders_complete: mirrors.get(TableId::ArmyOrders).is_some_and(Mirror::complete),
        }
    }
}

fn row(mirror: Option<&Mirror>, key: u128) -> Option<(&Mirror, u32)> {
    let mirror = mirror?;
    Some((mirror, mirror.slot(key)?))
}

fn provinces<'a>(bytes: &'a [u8], held: &'a mut Vec<u32>) -> &'a [u32] {
    let (head, words, tail) = unsafe { bytes.align_to::<u32>() };
    if head.is_empty() && tail.is_empty() && cfg!(target_endian = "little") {
        return words;
    }
    held.clear();
    held.extend(u32s(bytes));
    held
}

#[derive(Clone, Copy)]
enum Seer<'a> {
    Layer(&'a VisionLayer),
    Copy(&'a SightCopy),
    Given(&'a Vision),
}

impl Seer<'_> {
    fn sees(self, unit: u64, owner: u32, province: u32) -> (Sight, u32) {
        match self {
            Seer::Layer(vision) => (vision.sees_land(unit, owner, province).unwrap_or(Sight::None), vision.sight_province(unit, province)),
            Seer::Copy(sight) => (sight.sees_land(unit, owner, province).unwrap_or(Sight::None), sight.sight_province(unit, province)),
            Seer::Given(vision) => (vision.sees_land(unit, owner, province), province),
        }
    }

    fn shares(self, owner: u32) -> Result<bool, SharingUnknown> {
        match self {
            Seer::Layer(vision) => vision.shares_vision_proven(owner).unwrap_or(Ok(false)),
            Seer::Copy(sight) => sight.shares_vision_proven(owner).unwrap_or(Ok(false)),
            Seer::Given(vision) => vision.shares_vision_proven(owner),
        }
    }

    fn ours(self) -> u32 {
        match self {
            Seer::Layer(vision) => vision.ours().unwrap_or_default(),
            Seer::Copy(sight) => sight.ours().unwrap_or_default(),
            Seer::Given(vision) => vision.ours(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Sees<'a> {
    Layer(&'a VisionLayer),
    Copy(&'a SightCopy),
}

impl<'a> Sees<'a> {
    fn refused(self) -> Option<String> {
        match self {
            Sees::Layer(vision) => vision.refused(),
            Sees::Copy(sight) => sight.refused(),
        }
    }

    fn taken(self) -> (Choice, u64) {
        match self {
            Sees::Layer(vision) => (vision.choice().clone(), vision.sight_epoch()),
            Sees::Copy(sight) => (sight.choice().clone(), sight.sight_epoch()),
        }
    }

    fn seer(self) -> Seer<'a> {
        match self {
            Sees::Layer(vision) => Seer::Layer(vision),
            Sees::Copy(sight) => Seer::Copy(sight),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static MADE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn made_here() -> u64 {
    MADE.with(std::cell::Cell::get)
}

struct Joins<'a> {
    layer: &'a Counters,
    vision: Seer<'a>,
    read: std::cell::Cell<Option<u32>>,
    relations: &'a type_tint::Index,
    tables: &'a Tables<'a>,
    columns: Columns,
    division: u128,
    province: u32,
    movement_row: OnceCell<Option<(&'a Mirror, u32)>>,
    status_row: OnceCell<Option<(&'a Mirror, u32)>>,
    maxima_of: OnceCell<Option<[i64; 2]>>,
}

impl<'a> Joins<'a> {
    fn new(layer: &'a Counters, (vision, relations): (Seer<'a>, &'a type_tint::Index), tables: &'a Tables<'a>, columns: Columns, division: u64, province: u32) -> Self {
        Self { layer, vision, read: std::cell::Cell::new(None), relations, tables, columns, division: u128::from(division), province, movement_row: OnceCell::new(), status_row: OnceCell::new(), maxima_of: OnceCell::new() }
    }

    fn movement(&self) -> Option<(&'a Mirror, u32)> {
        *self.movement_row.get_or_init(|| row(self.tables.movement, self.division))
    }

    fn status_at(&self) -> Option<(&'a Mirror, u32)> {
        *self.status_row.get_or_init(|| row(self.tables.status, self.division))
    }

    fn maxima(&self) -> Option<[i64; 2]> {
        let columns = self.columns;
        *self.maxima_of.get_or_init(|| {
            let (panels, slot) = row(self.tables.panels, self.division)?;
            let maximum = |status: Num<i64>, word: Num<i64>| DivisionPanelStatsRow::shown_maximum(native(panels, status, slot).value(), native(panels, word, slot).value());
            Some([maximum(columns.status_hit_points, columns.panel_hit_points)?, maximum(columns.status_organisation, columns.panel_organisation)?])
        })
    }
}

impl MemberJoins for Joins<'_> {
    fn owner(&self, raw: u32) -> Option<u32> {
        let (links, slot) = row(self.tables.links, u128::from(raw))?;
        links.number(self.columns.link_slot, slot).value()
    }

    fn ordinal(&self, division: u64, province: u32) -> Option<Option<u32>> {
        let order = self.tables.order?;
        let Some(slot) = order.slot(u128::from(province)) else { return Some(None) };
        Some(u64s(bytes_of(order.list(self.columns.mixed, slot))).position(|unit| unit == division).map(|at| at as u32))
    }

    fn sight(&self, division: u64, owner: u32, province: u32) -> Sight {
        let (sight, read) = self.vision.sees(division, owner, province);
        self.read.set(Some(read));
        sight
    }

    fn shared_vision(&self, owner: u32) -> Result<bool, SharingUnknown> {
        self.vision.shares(owner)
    }

    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side> {
        self.relations.of(roster, logical)
    }

    fn ours(&self) -> u32 {
        self.vision.ours()
    }

    fn icon_input(&self, _division: u64) -> Option<(u8, u64)> {
        let (inputs, slot) = row(self.tables.icons, self.division)?;
        Some((inputs.number(self.columns.separate_counter, slot).value()?, inputs.number(self.columns.alternate_template, slot).value()?))
    }

    fn hq_provider(&self, division: u64) -> bool {
        self.layer.hq.contains_key(&division)
    }

    fn stack_kind(&self, _division: u64) -> u32 {
        let Some((movement, slot)) = self.movement() else {
            return viewer_foundation::unit_rules::STANDSTILL;
        };
        let columns = self.columns;
        let number = |column: Num<u32>| movement.number(column, slot).value().unwrap_or(0);
        let (mut ordinary, mut alternate) = (Vec::new(), Vec::new());
        let route = UnitRoute {
            class: CounterClass::Division,
            combat_role: movement.number(columns.combat_role, slot).value().unwrap_or(0),
            combat_count: number(columns.combat_count),
            combat_province: number(columns.combat_province),
            target: number(columns.target),
            naval_transfer: movement.number(columns.naval_transfer, slot).value().unwrap_or(0) != 0,
            path_total: number(columns.path_total),
            path: provinces(bytes_of(movement.list(columns.path, slot)), &mut ordinary),
            alternate_total: number(columns.alternate_total),
            alternate_path: provinces(bytes_of(movement.list(columns.alternate_path, slot)), &mut alternate),
        };
        let content = &self.layer.content;
        let anchor = |id| content.counters.as_ref().and_then(|counters| counters.anchor(id, viewer_foundation::unit_rules::STANDSTILL));
        division_kind(&route, self.province, content.movement.as_deref().map(|movement| &movement.map), &anchor)
    }

    fn status(&self, _division: u64) -> DivisionStatus {
        let columns = self.columns;
        let naval_transfer = self.movement().and_then(|(movement, slot)| {
            movement
                .number(columns.naval_transfer, slot)
                .value()
                .map(|value| value != 0)
        });
        let Some((status, slot)) = self.status_at() else {
            return DivisionStatus {
                naval_transfer,
                ..DivisionStatus::MISSING
            };
        };
        let number = |column: Num<i64>| native(status, column, slot).value().unwrap_or(0);
        let byte = |column: Num<u8>| status.number(column, slot).value().unwrap_or(0);
        let fields = StatusFields {
            entrenchment: number(columns.entrenchment),
            entrenchment_cap: number(columns.entrenchment_cap),
            supply_ratio: DivisionStatusRow::supply_ratio_bounds(native(status, columns.supply_ratio, slot).value(), native(status, columns.supply_ratio_exact, slot).value()),
            attrition: DivisionStatusRow::attrition(number(columns.attrition_threshold), number(columns.attrition_value)),
            exercising: byte(columns.exercising),
            has_order: byte(columns.has_order),
            invalid_orders: byte(columns.invalid_orders),
            strength_factor: number(columns.strength_factor),
        };
        DivisionStatus::recorded(fields, self.maxima(), naval_transfer)
    }

    fn bars(&self, _division: u64) -> bool {
        self.status_at().is_some() && self.maxima().is_some()
    }

    fn orders(&self) -> &dyn OrderFacts {
        self
    }

    fn withdrawing(&self, group: u64) -> Option<bool> {
        let (inputs, slot) = row(self.tables.group_inputs, u128::from(group))?;
        Some(inputs.number(self.columns.withdrawing, slot).value()? != 0)
    }

    fn leader(&self, group: u64) -> Option<u64> {
        let (groups, slot) = row(self.tables.groups, u128::from(group))?;
        groups.number(self.columns.leader, slot).value()
    }
}

impl OrderFacts for Joins<'_> {
    fn group(&self, group: u64) -> Option<GroupFacts> {
        let (groups, slot) = row(self.tables.groups, u128::from(group))?;
        Some(GroupFacts { parent: groups.number(self.columns.group_parent, slot).value()?, army_group: groups.number(self.columns.army_group, slot).value()? != 0 })
    }

    fn has_child(&self, (group, instance): (u64, u32)) -> Option<bool> {
        let (plans, slot) = row(self.tables.plans, u128::from(group) << 32 | u128::from(instance))?;
        Some(plans.number(self.columns.has_child, slot).value()? != 0)
    }

    fn order_type(&self, (group, instance): (u64, u32)) -> Option<i32> {
        let (orders, slot) = row(self.tables.orders, u128::from(group) << 32 | u128::from(instance))?;
        orders.number(self.columns.order_type, slot).value()
    }

    fn area_group(&self, group: u64) -> bool {
        self.layer.area.contains_key(&group)
    }

    fn orders_complete(&self) -> bool {
        self.tables.orders_complete
    }

    fn memberships(&self, group: u64, set: usize) -> Option<&AssignedOrders> {
        self.layer.memberships.get(&group).map(|sets| &sets[set])
    }
}

fn count_in(counts: &mut HashMap<u64, u32>, key: u64) {
    *counts.entry(key).or_default() += 1;
}

fn count_out(counts: &mut HashMap<u64, u32>, key: u64) {
    if let Some(count) = counts.get_mut(&key) {
        *count -= 1;
        if *count == 0 {
            counts.remove(&key);
        }
    }
}

impl Counters {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            content,
            columns: None,
            refused: Some("not composed yet".into()),
            members: Vec::new(),
            sources: BTreeMap::new(),
            by_group: HashMap::new(),
            children: HashMap::new(),
            group_rows: Vec::new(),
            hq: HashMap::new(),
            area_rows: Vec::new(),
            area: HashMap::new(),
            memberships: HashMap::new(),
            divisions: Dirty::default(),
            causes: Vec::new(),
            division_rows: Default::default(),
            order_rows: Rows::default(),
            plan_rows: Default::default(),
            input_rows: Rows::default(),
            destination_rows: Rows::default(),
            transfers_new: false,
            standing_new: false,
            layout_new: false,
            provinces: Dirty::default(),
            group_slots: Dirty::default(),
            area_slots: Dirty::default(),
            plans: HashSet::new(),
            all: true,
            strategic_layout: false,
            hidden: false,
            vision: Mutex::new(None),
            vision_new: AtomicBool::new(false),
            reads_layer: false,
            sight: None,
            readers: HashMap::new(),
            read_at: Vec::new(),
            relations: None,
            relations_new: false,
            #[cfg(any(test, feature = "test-support"))]
            given: false,
            statuses: HashMap::new(),
            frame: 0,
            marked: 0,
            phases: [(0, 0); PHASES.len()],
            camera: None,
            camera_dirty: false,
            admits: None,
            asked: Vec::new(),
            admitted: Vec::new(),
            out: Vec::new(),
            outside: 0,
            out_provinces: Vec::new(),
            rows_of: Default::default(),
            stale: HashSet::new(),
            scratch: Vec::new(),
            published: Arc::default(),
            touched: BTreeSet::new(),
            actions: Vec::new(),
            reads: Vec::new(),
            by_destination: HashMap::new(),
            transferring: HashSet::new(),
            standing: HashSet::new(),
            wars: None,
            wars_new: false,
            #[cfg(any(test, feature = "test-support"))]
            applied_reads: 0,
            #[cfg(any(test, feature = "test-support"))]
            validated: Default::default(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn validated(&self) -> [usize; 2] {
        self.validated.each_ref().map(|count| count.load(Ordering::Relaxed))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    pub fn reading_vision(self) -> Self {
        Self { reads_layer: true, ..self }
    }

    pub fn fresh(&self) -> Self {
        let fresh = Self { columns: self.columns, hidden: self.hidden, camera: self.camera, admits: self.admits.clone(), reads_layer: self.reads_layer, ..Self::new(Arc::clone(&self.content)) };
        if let Some((choice, vision)) = self.perspective() {
            fresh.set_vision(choice, vision);
        }
        #[cfg(any(test, feature = "test-support"))]
        let fresh = Self { given: self.given, ..fresh };
        Self { relations: self.relations.clone(), wars: self.wars.clone(), ..fresh }
    }

    fn take_relations(&mut self, upstream: &Upstream<'_>) {
        if let Some(relations) = upstream.get::<Relations>() {
            self.relating(Arc::clone(relations.index()), relations.wars().cloned());
        }
    }

    pub(super) fn relating(&mut self, index: Arc<type_tint::Index>, wars: Option<Arc<relations::Facts>>) {
        self.relate(index);
        if !matches!((&self.wars, &wars), (Some(held), Some(wars)) if Arc::ptr_eq(held, wars)) {
            self.wars = wars;
            self.wars_new = true;
        }
    }

    fn relate(&mut self, relations: Arc<type_tint::Index>) {
        if self.relations.as_ref().is_none_or(|held| !Arc::ptr_eq(held, &relations)) {
            self.relations = Some(relations);
            self.relations_new = true;
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_relations(mut self, relations: Arc<type_tint::Index>) -> Self {
        self.relate(relations);
        self.given = true;
        self
    }

    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) -> bool {
        let mut held = self.vision.lock().unwrap_or_else(|error| error.into_inner());
        let new = held.as_ref().is_none_or(|(_, held)| !Arc::ptr_eq(held, &vision) && **held != *vision);
        if new {
            self.vision_new.store(true, Ordering::Release);
        }
        *held = Some((choice, vision));
        new
    }

    pub fn perspective(&self) -> Option<(Choice, Arc<Vision>)> {
        self.vision.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the counters layer is declared before it composes")
    }

    fn strategic_layout_at(&self, mirrors: &Mirrors) -> bool {
        #[cfg(any(test, feature = "test-support"))]
        {
            let identity = |tables: engine_executable::executable_tables::Tables| std::ptr::from_ref(tables);
            let recorded = self.content.tables.map(identity);
            let in_scope = recorded.is_some() && engine_executable::executable_tables::current().map(identity) == recorded;
            self.validated[usize::from(!in_scope)].fetch_add(1, Ordering::Relaxed);
        }
        let columns = self.columns();
        let (definitions, tokens) = (mirrors.get(TableId::ModifierDefinitions), mirrors.get(TableId::LexicalTokens));
        let whole = match (definitions, tokens) {
            (Some(definitions), Some(tokens)) if definitions.complete() && tokens.complete() => Ok(()),
            (Some(definitions), _) if !definitions.complete() => Err("ModifierDefinitions is not complete".to_owned()),
            (None, _) => Err("ModifierDefinitions is not read".to_owned()),
            _ => Err("LexicalTokens is not complete".to_owned()),
        };
        viewer_unit_reads::counter_action::strategic_layout_of(
            whole,
            |index| definitions.and_then(|mirror| mirror.number(columns.definition_token, mirror.slot(u128::from(index))?).value()),
            |token, name| {
                let tokens = tokens?;
                let slot = tokens.slot(u128::from(token))?;
                Some(tokens.text(columns.literal, slot).value().and_then(|id| tokens.string(id)) == Some(name))
            },
        )
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    fn release(&mut self) {
        let empty = Self {
            #[cfg(any(test, feature = "test-support"))]
            given: self.given,
            ..Self::new(Arc::clone(&self.content))
        };
        let kept = std::mem::replace(self, empty);
        (self.columns, self.hidden, self.frame, self.marked, self.phases) = (kept.columns, kept.hidden, kept.frame, kept.marked, kept.phases);
        (self.camera, self.camera_dirty, self.admits) = (kept.camera, kept.camera_dirty, kept.admits);
        (self.vision, self.vision_new, self.relations, self.relations_new, self.wars, self.wars_new) = (kept.vision, kept.vision_new, kept.relations, kept.relations_new, kept.wars, kept.wars_new);
        self.reads_layer = kept.reads_layer;
        #[cfg(any(test, feature = "test-support"))]
        {
            (self.applied_reads, self.validated) = (kept.applied_reads, kept.validated);
        }
    }

    fn take_vision(&mut self, upstream: &Upstream<'_>, mirrors: &Mirrors) {
        let Some(vision) = upstream.get::<VisionLayer>() else { return };
        let taken = Sees::Layer(vision).taken();
        match upstream.changed::<VisionLayer>() {
            Change::Whole => self.take_sight(taken, true, [].into_iter(), [].into_iter(), mirrors),
            Change::Marked(changed) => self.take_sight(taken, false, changed.of("province").iter().copied(), changed.of("unit").iter().copied(), mirrors),
        }
    }

    pub(super) fn take_sight(&mut self, taken: (Choice, u64), whole: bool, provinces: impl Iterator<Item = u64>, units: impl Iterator<Item = u64>, mirrors: &Mirrors) {
        if !self.reads_layer || self.hidden || self.refused.is_some() {
            return;
        }
        if whole || self.sight.as_ref() != Some(&taken) {
            self.sight = Some(taken);
            self.vision_new.store(true, Ordering::Release);
            return;
        }
        let slots: Vec<u32> = provinces.flat_map(|province| self.readers.get(&(province as u32)).into_iter().flatten().copied()).collect();
        slots.into_iter().for_each(|slot| self.mark_division(slot, FULL));
        for unit in units {
            self.mark_division_key(mirrors, u128::from(unit), FULL);
        }
    }

    fn index_sight(&mut self, slot: u32, read: Option<u32>) {
        let at = slot as usize;
        if self.read_at.len() <= at {
            self.read_at.resize(at + 1, u32::MAX);
        }
        let read = read.unwrap_or(u32::MAX);
        let old = std::mem::replace(&mut self.read_at[at], read);
        if old == read {
            return;
        }
        if let Some(readers) = self.readers.get_mut(&old) {
            readers.retain(|&held| held != slot);
            if readers.is_empty() {
                self.readers.remove(&old);
            }
        }
        if read != u32::MAX {
            self.readers.entry(read).or_default().push(slot);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn sources(&self) -> impl Iterator<Item = (Source, impl Iterator<Item = &Member> + '_)> + '_ {
        self.sources.iter().map(|(&source, slots)| (source, slots.iter().map(|&slot| &self.members[slot as usize].as_ref().expect("a source's member").1)))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn marked(&self) -> usize {
        self.marked
    }

    fn admits(&mut self, province: u32) -> bool {
        let Some(admits) = &self.admits else { return true };
        if set(&mut self.asked, province) && admits(province) {
            set(&mut self.admitted, province);
        }
        bit(&self.admitted, province)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn admitted(&self, province: u32) -> bool {
        self.admits.as_ref().is_none_or(|admits| admits(province))
    }

    fn readmit(&mut self) {
        self.camera_dirty = false;
        let content = Arc::clone(&self.content);
        let admits: Option<Admits> = match (self.camera, content.map, content.counters.as_ref()) {
            (Some((placement, viewport)), Some((size, defines)), Some(counters)) if viewport != [0, 0] => {
                let admission = Admission::new(placement.on(size, defines), viewport);
                let counters = Arc::clone(counters);
                Some(Arc::new(move |province| admission.admits_province(&counters.bounds, province)))
            }
            _ => None,
        };
        self.reconsider(admits);
    }

    fn reconsider(&mut self, admits: Option<Admits>) {
        self.admits = admits;
        self.asked.clear();
        self.admitted.clear();
        let out: Vec<u32> = ones(&self.out).collect();
        self.out.clear();
        self.outside = 0;
        for slot in out {
            let cause = self.causes.get(slot as usize).copied().unwrap_or(FULL);
            self.mark_division(slot, cause);
        }
        let provinces: Vec<u32> = ones(&self.out_provinces).collect();
        self.out_provinces.clear();
        provinces.into_iter().for_each(|province| self.mark_province(province));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn admit_only(&mut self, admits: impl Fn(u32) -> bool + Send + Sync + 'static) {
        self.admits = Some(Arc::new(admits));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn outside(&self) -> usize {
        self.outside
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn phases(&self) -> impl Iterator<Item = (&'static str, u64, u64)> + '_ {
        PHASES.iter().zip(&self.phases).map(|(&name, &(entities, ns))| (name, entities, ns))
    }

    fn spent(&mut self, phase: usize, entities: u64, since: std::time::Instant) {
        self.phases[phase].0 += entities;
        self.phases[phase].1 += since.elapsed().as_nanos() as u64;
    }

    fn core(&self, mirrors: &Mirrors, layer: Option<Sees<'_>>) -> Result<(Option<Arc<Vision>>, Arc<type_tint::Index>), String> {
        let vision = if self.reads_layer {
            let layer = layer.ok_or("the vision layer is not read")?;
            if let Some(why) = layer.refused() {
                return Err(format!("the vision layer has no vision: {why}"));
            }
            None
        } else {
            Some(self.perspective().ok_or("the viewing country's vision is not set")?.1)
        };
        let relations = self.relations.clone().ok_or("the viewing country's type-icon relations are not read: the relations layer draws nothing")?;
        for table in [TableId::CountryLinks, TableId::Countries] {
            let mirror = mirrors.get(table).ok_or_else(|| format!("{table:?} is not read yet"))?;
            if let Some(why) = mirror.not_recorded() {
                return Err(format!("{why}; the counters need every country link and country"));
            }
        }
        let divisions = mirrors.get(TableId::Divisions).ok_or("Divisions is not read yet")?;
        if divisions.status().is_none() {
            return Err("Divisions is not recorded here: a gap in the recording".into());
        }
        if mirrors.get(TableId::DivisionStatus).and_then(Mirror::status) == Some(Status::NotRecorded) {
            return Err("DivisionStatus is not recorded: the counters' exercise glyphs would read DivisionModelInputs, which this layer does not read".into());
        }
        Ok((vision, relations))
    }

    fn mark_division(&mut self, slot: u32, cause: u8) {
        let at = slot as usize;
        if self.causes.len() <= at {
            self.causes.resize(at + 1, 0);
        }
        self.causes[at] |= cause;
        if self.divisions.mark(slot, self.frame) {
            self.marked += 1;
        }
    }

    fn inside(&mut self, divisions: Option<&Mirror>, slot: u32) -> bool {
        self.admits.is_none() || {
            let standing = self.members.get(slot as usize).and_then(Option::as_ref).map(|((province, _), _)| *province);
            let now = divisions.filter(|divisions| divisions.live(slot)).and_then(|divisions| divisions.number(self.columns().province, slot).value());
            standing.is_some_and(|province| self.admits(province)) || now.is_some_and(|province| self.admits(province))
        }
    }

    fn keep_outside(&mut self, slot: u32) {
        if set(&mut self.out, slot) {
            self.outside += 1;
            self.phases[8].0 += 1;
        }
    }

    fn mark_division_key(&mut self, mirrors: &Mirrors, id: u128, cause: u8) {
        if let Some(slot) = mirrors.get(TableId::Divisions).and_then(|divisions| divisions.slot(id)) {
            self.mark_division(slot, cause);
        }
    }

    fn division_of(&mut self, which: usize, table: &Mirror, divisions: &Mirror, slot: u32) -> Option<u32> {
        let key = table.key(slot)?;
        let cache = &mut self.rows_of[which];
        if let Some(&held) = cache.get(slot as usize)
            && held != u32::MAX
            && divisions.live(held)
            && divisions.key(held) == Some(key)
        {
            return Some(held);
        }
        let found = divisions.slot(key)?;
        if cache.len() <= slot as usize {
            cache.resize(slot as usize + 1, u32::MAX);
        }
        cache[slot as usize] = found;
        Some(found)
    }

    fn mark_province(&mut self, province: u32) {
        self.provinces.mark(province, self.frame);
    }

    fn mark_group(&mut self, group: u64, cause: u8) {
        if group == 0 {
            return;
        }
        let mut slots = std::mem::take(&mut self.scratch);
        slots.clear();
        slots.extend(self.by_group.get(&group).into_iter().flatten());
        for child in self.children.get(&group).into_iter().flatten() {
            slots.extend(self.by_group.get(child).into_iter().flatten());
        }
        slots.iter().for_each(|&slot| self.mark_division(slot, cause));
        self.scratch = slots;
    }

    fn fresh_memberships(&mut self, tables: &Tables<'_>, group: u64) {
        let columns = self.columns();
        let parent = row(tables.groups, u128::from(group)).and_then(|(groups, slot)| groups.number(columns.group_parent, slot).value()).unwrap_or(0);
        for group in [group, parent] {
            if group != 0 && self.stale.remove(&group) {
                self.membership(tables.group_plans, tables.plans, group, columns);
                self.phases[9].0 += 1;
            }
        }
    }

    fn recompose(&mut self, mirrors: &Mirrors) {
        let columns = self.columns();
        self.touched.extend(self.published.keys().collect::<Vec<_>>());
        self.members.clear();
        self.sources.clear();
        self.actions.clear();
        self.reads.clear();
        self.by_destination.clear();
        self.transferring.clear();
        self.standing.clear();
        self.by_group.clear();
        self.children.clear();
        self.group_rows.clear();
        self.hq.clear();
        self.area_rows.clear();
        self.area.clear();
        self.memberships.clear();
        self.divisions.clear();
        self.out.clear();
        self.outside = 0;
        self.out_provinces.clear();
        self.stale.clear();
        self.rows_of = Default::default();
        self.causes.clear();
        self.provinces.clear();
        self.group_slots.clear();
        self.area_slots.clear();
        self.plans.clear();
        self.division_rows.iter_mut().chain(&mut self.plan_rows).chain([&mut self.order_rows, &mut self.input_rows, &mut self.destination_rows]).for_each(Rows::clear);
        (self.transfers_new, self.standing_new, self.layout_new) = (false, false, false);
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        if let Some(groups) = held(mirrors.get(TableId::OrdersGroups)) {
            groups.rows().for_each(|slot| self.group_row(Some(groups), slot, columns));
        }
        if let Some(orders) = held(mirrors.get(TableId::ArmyOrders)) {
            orders.rows().for_each(|slot| self.area_row(Some(orders), slot, columns));
        }
        if let Some(plans) = held(mirrors.get(TableId::OrdersGroupPlans)) {
            let groups: Vec<u64> = plans.rows().filter_map(|slot| plans.key(slot)).map(|key| key as u64).collect();
            let orders = held(mirrors.get(TableId::ArmyOrderPlans));
            groups.into_iter().for_each(|group| self.membership(Some(plans), orders, group, columns));
        }
        if self.camera_dirty {
            self.readmit();
        }
        let divisions = mirrors.get(TableId::Divisions);
        if let Some(divisions) = divisions {
            let slots: Vec<u32> = divisions.rows().collect();
            slots.into_iter().for_each(|slot| self.mark_division(slot, FULL));
        }
    }

    fn fields(divisions: &Mirror, slot: u32, columns: Columns) -> Option<DivisionFields> {
        let (Some(key), Cell::Value(roster_owner), Cell::Value(logical_owner), Cell::Value(expeditionary_owner), Cell::Value(province)) = (
            divisions.key(slot),
            divisions.number(columns.roster_owner, slot),
            divisions.number(columns.logical_owner, slot),
            divisions.number(columns.expeditionary_owner, slot),
            divisions.number(columns.province, slot),
        ) else {
            return None;
        };
        let (Cell::Value(template), Cell::Value(hit_points), Cell::Value(organisation), Cell::Value(orders_group)) =
            (divisions.number(columns.template, slot), native(divisions, columns.hit_points, slot), native(divisions, columns.organisation, slot), divisions.number(columns.orders_group, slot))
        else {
            return None;
        };
        Some(DivisionFields { id: key as u64, roster_owner, logical_owner, expeditionary_owner, province, template, hit_points, organisation, orders_group })
    }

    fn settle_parts(&mut self, tables: &Tables<'_>, seen: (Seer<'_>, &type_tint::Index), acting: &Acting<'_>, slot: u32, cause: u8) -> bool {
        let at = slot as usize;
        let columns = self.columns();
        let Some(divisions) = tables.divisions.filter(|divisions| divisions.live(slot)) else { return false };
        let Some(((province, _), member)) = self.members.get(at).and_then(Option::as_ref) else { return false };
        let (province, id, orders_group, owner) = (*province, member.id, member.orders_group, member.owner);
        let source = self.members[at].as_ref().map(|(source, _)| *source).expect("a member");
        let (Cell::Value(hit_points), Cell::Value(organisation)) = (native(divisions, columns.hit_points, slot), native(divisions, columns.organisation, slot)) else {
            return false;
        };
        let fields = DivisionFields { id, province, orders_group, hit_points, organisation, ..DivisionFields::default() };
        if cause & ORDERS != 0 {
            self.fresh_memberships(tables, fields.orders_group);
        }
        let joins = Joins::new(self, seen, tables, columns, fields.id, fields.province);
        let strength = (cause & STRENGTH != 0).then(|| member_strength(&fields, owner, &joins));
        let orders = (cause & ORDERS != 0).then(|| member_orders(&fields, &joins));
        self.touched.insert(source);
        let (_, member) = self.members[at].as_mut().expect("a member");
        if let Some(strength) = strength {
            member.set_strength(strength);
        }
        if let Some(orders) = orders {
            member.set_orders(orders);
        }
        if cause & ACTION != 0 {
            self.act(tables, acting, slot);
        }
        true
    }

    fn act(&mut self, tables: &Tables<'_>, acting: &Acting<'_>, slot: u32) {
        let at = slot as usize;
        self.untrack(slot);
        let Some((_, member)) = self.members.get(at).and_then(Option::as_ref) else { return };
        let logical = tables.divisions.filter(|divisions| divisions.live(slot)).and_then(|divisions| divisions.number(self.columns().logical_owner, slot).value());
        let (action, read) = if member.owner == acting.ours && let Some(logical) = logical {
            acting.facts.action(tables, self.columns(), member.id, logical, acting.wars.as_deref(), self.strategic_layout)
        } else {
            (Err(acting.not_ours), action::Read::default())
        };
        if self.actions.len() <= at {
            self.actions.resize(at + 1, Err(acting.not_ours));
            self.reads.resize(at + 1, action::Read::default());
        }
        self.actions[at] = action;
        self.reads[at] = read;
        if let Some(province) = read.destination {
            self.by_destination.entry(province).or_default().push(slot);
        }
        if read.transfer {
            self.transferring.insert(slot);
        }
        if read.standing {
            self.standing.insert(slot);
        }
    }

    fn untrack(&mut self, slot: u32) {
        let Some(read) = self.reads.get_mut(slot as usize).map(std::mem::take) else { return };
        if let Some(province) = read.destination
            && let Some(slots) = self.by_destination.get_mut(&province)
        {
            slots.retain(|&held| held != slot);
            if slots.is_empty() {
                self.by_destination.remove(&province);
            }
        }
        self.transferring.remove(&slot);
        self.standing.remove(&slot);
    }

    fn action_of(&self, slot: u32) -> Action {
        self.actions.get(slot as usize).copied().unwrap_or(Err("DivisionMovement"))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn sources_acting(&self) -> impl Iterator<Item = (Source, impl Iterator<Item = (&Member, Action)> + '_)> + '_ {
        self.sources.iter().map(|(&source, slots)| (source, slots.iter().map(|&slot| (&self.members[slot as usize].as_ref().expect("a source's member").1, self.action_of(slot)))))
    }

    fn reorder(&mut self, tables: &Tables<'_>, mirrors: &Mirrors, province: u32, columns: Columns) {
        let Some(order) = tables.order else { return };
        let units: Vec<u64> = order.slot(u128::from(province)).map(|slot| u64s(bytes_of(order.list(columns.mixed, slot))).collect()).unwrap_or_default();
        let sources: Vec<Source> = self.sources.range((province, 0)..=(province, u32::MAX)).map(|(&source, _)| source).collect();
        let mut standing = HashSet::new();
        for source in sources {
            let slots = self.sources.get(&source).cloned().unwrap_or_default();
            let mut moved = false;
            for slot in slots {
                let member = &mut self.members[slot as usize].as_mut().expect("a source's member").1;
                standing.insert(member.id);
                match units.iter().position(|&unit| unit == member.id) {
                    Some(at) if at as u32 == member.ordinal => {}
                    Some(at) => {
                        member.ordinal = at as u32;
                        moved = true;
                    }
                    None => self.mark_division(slot, FULL),
                }
            }
            if moved {
                self.touched.insert(source);
                let members = &self.members;
                if let Some(slots) = self.sources.get_mut(&source) {
                    slots.sort_by_key(|&slot| {
                        let member = &members[slot as usize].as_ref().expect("a source's member").1;
                        (member.ordinal, member.id)
                    });
                }
            }
        }
        for unit in units.into_iter().filter(|unit| !standing.contains(unit)) {
            self.mark_division_key(mirrors, u128::from(unit), FULL);
        }
    }

    fn group_row(&mut self, groups: Option<&Mirror>, slot: u32, columns: Columns) {
        let at = slot as usize;
        if self.group_rows.len() <= at {
            self.group_rows.resize(at + 1, [0; 3]);
        }
        let [handle, parent, provider] = self.group_rows[at];
        if handle != 0 {
            if let Some(children) = self.children.get_mut(&parent) {
                children.retain(|&child| child != handle);
            }
            if provider != 0 {
                count_out(&mut self.hq, provider);
            }
        }
        let new = match groups.filter(|groups| groups.live(slot)) {
            Some(groups) => {
                let handle = groups.key(slot).map_or(0, |key| key as u64);
                let parent = groups.number(columns.group_parent, slot).value().unwrap_or(0);
                let leader = groups.number(columns.leader, slot).value().unwrap_or(0);
                let provider = groups.number(columns.hq_provider, slot).value().unwrap_or(0);
                [handle, parent, if leader != 0 { provider } else { 0 }]
            }
            None => [0; 3],
        };
        if new[0] != 0 {
            self.children.entry(new[1]).or_default().push(new[0]);
            if new[2] != 0 {
                count_in(&mut self.hq, new[2]);
            }
        }
        self.group_rows[at] = new;
    }

    fn area_row(&mut self, orders: Option<&Mirror>, slot: u32, columns: Columns) {
        let at = slot as usize;
        if self.area_rows.len() <= at {
            self.area_rows.resize(at + 1, (0, false));
        }
        let (group, area) = self.area_rows[at];
        if area {
            count_out(&mut self.area, group);
        }
        let new = match orders.filter(|orders| orders.live(slot)) {
            Some(orders) => {
                let group = orders.key(slot).map_or(0, |key| (key >> 32) as u64);
                (group, !bytes_of(orders.list(columns.target_states, slot)).is_empty())
            }
            None => (0, false),
        };
        if new.1 {
            count_in(&mut self.area, new.0);
        }
        self.area_rows[at] = new;
    }

    fn membership(&mut self, group_plans: Option<&Mirror>, orders: Option<&Mirror>, group: u64, columns: Columns) {
        let Some((plans, slot)) = row(group_plans, u128::from(group)) else {
            self.memberships.remove(&group);
            return;
        };
        let sets = [columns.primary_orders, columns.fallback_orders].map(|roots| {
            AssignedOrders::of(u32s(bytes_of(plans.list(roots, slot))).map(|instance| {
                let assigned = orders.and_then(|orders| orders.slot(u128::from(group) << 32 | u128::from(instance)).map(|row| u64s(bytes_of(orders.list(columns.assigned, row)))));
                (instance, assigned)
            }))
        });
        self.memberships.insert(group, sets);
    }

    fn settle_division(&mut self, tables: &Tables<'_>, seen: (Seer<'_>, &type_tint::Index), acting: &Acting<'_>, slot: u32) {
        let at = slot as usize;
        if self.members.len() <= at {
            self.members.resize(at + 1, None);
        }
        #[cfg(any(test, feature = "test-support"))]
        MADE.with(|made| made.set(made.get() + 1));
        let cause = self.causes.get_mut(at).map_or(FULL, std::mem::take);
        if cause & FULL == 0 && cause != 0 && self.settle_parts(tables, seen, acting, slot, cause) {
            self.phases[if cause & STRENGTH != 0 { 6 } else { 7 }].0 += 1;
            return;
        }
        self.settle_whole(tables, seen, acting, slot);
        self.phases[5].0 += 1;
    }

    fn settle_whole(&mut self, tables: &Tables<'_>, seen: (Seer<'_>, &type_tint::Index), acting: &Acting<'_>, slot: u32) {
        let at = slot as usize;
        self.untrack(slot);
        self.index_sight(slot, None);
        if let Some((source, member)) = self.members[at].take() {
            self.touched.insert(source);
            if let Some(slots) = self.sources.get_mut(&source) {
                slots.retain(|&held| held != slot);
                if slots.is_empty() {
                    self.sources.remove(&source);
                }
            }
            if let Some(slots) = self.by_group.get_mut(&member.orders_group) {
                slots.retain(|&held| held != slot);
                if slots.is_empty() {
                    self.by_group.remove(&member.orders_group);
                }
            }
        }
        let columns = self.columns();
        let Some(divisions) = tables.divisions.filter(|divisions| divisions.live(slot)) else { return };
        let Some(fields) = Self::fields(divisions, slot, columns) else { return };
        self.fresh_memberships(tables, fields.orders_group);
        let joins = Joins::new(self, seen, tables, columns, fields.id, fields.province);
        let joined = division_member(fields, &joins);
        let read = joins.read.get();
        drop(joins);
        self.index_sight(slot, read);
        let Some((source, member)) = joined else { return };
        self.touched.insert(source);
        let slots = self.sources.entry(source).or_default();
        let place = slots.partition_point(|&held| {
            let other = &self.members[held as usize].as_ref().expect("a source's member").1;
            (other.ordinal, other.id) < (member.ordinal, member.id)
        });
        slots.insert(place, slot);
        self.by_group.entry(member.orders_group).or_default().push(slot);
        self.members[at] = Some((source, member));
        self.act(tables, acting, slot);
    }

    pub(super) fn compose_reading(&mut self, mirrors: &Mirrors, layer: Option<Sees<'_>>) -> io::Result<()> {
        let (marked, phases) = (self.marked, self.phases);
        self.all = true;
        self.settle_dirty(mirrors, layer);
        (self.marked, self.phases) = (marked, phases);
        Ok(())
    }

    pub(super) fn settle_dirty(&mut self, mirrors: &Mirrors, layer: Option<Sees<'_>>) {
        self.settle_marks(mirrors, layer);
        self.frame += 1;
    }

    fn settle_marks(&mut self, mirrors: &Mirrors, layer: Option<Sees<'_>>) {
        let refused = if self.hidden { Err("not drawn at this view".to_owned()) } else { self.core(mirrors, layer) };
        let (given, relations) = match refused {
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
        let vision = match (&given, layer) {
            (Some(given), _) => Seer::Given(given),
            (None, Some(layer)) => layer.seer(),
            (None, None) => return,
        };
        if std::mem::take(&mut self.layout_new) {
            let layout = self.strategic_layout_at(mirrors);
            if layout != self.strategic_layout {
                self.strategic_layout = layout;
                self.all = true;
            }
        }
        if self.refused.take().is_some() || self.all {
            self.all = false;
            self.strategic_layout = self.strategic_layout_at(mirrors);
            self.sight = layer.filter(|_| self.reads_layer).map(Sees::taken);
            self.vision_new.store(false, Ordering::Release);
            self.relations_new = false;
            self.recompose(mirrors);
        }
        if self.camera_dirty {
            self.readmit();
        }
        let divisions = mirrors.get(TableId::Divisions);
        let relations_new = std::mem::take(&mut self.relations_new);
        if self.vision_new.swap(false, Ordering::AcqRel) | relations_new {
            let slots: Vec<u32> = divisions.map(|divisions| divisions.rows().collect()).unwrap_or_default();
            let members: Vec<u32> = self.members.iter().enumerate().filter(|(_, member)| member.is_some()).map(|(slot, _)| slot as u32).collect();
            slots.into_iter().chain(members).for_each(|slot| self.mark_division(slot, FULL));
        }
        let columns = self.columns();
        let began = std::time::Instant::now();
        let taken = self.rows_carried() as u64;
        self.resolve(mirrors);
        self.spent(10, taken.saturating_sub(self.rows_carried() as u64), began);
        let began = std::time::Instant::now();
        let mut dirty = std::mem::take(&mut self.group_slots);
        let taken = dirty.len() as u64;
        let groups = held(mirrors.get(TableId::OrdersGroups));
        let _ = dirty.settle(|slot| {
            let before = self.group_rows.get(slot as usize).copied().unwrap_or([0; 3]);
            self.group_row(groups, slot, columns);
            let after = self.group_rows[slot as usize];
            for provider in [before[2], after[2]].into_iter().filter(|&provider| provider != 0) {
                self.mark_division_key(mirrors, u128::from(provider), FULL);
            }
            self.mark_group(before[0], ORDERS);
            self.mark_group(after[0], ORDERS);
            Ok(())
        });
        self.group_slots = dirty;
        self.spent(0, taken, began);
        let began = std::time::Instant::now();
        let mut dirty = std::mem::take(&mut self.area_slots);
        let taken = dirty.len() as u64;
        let orders = held(mirrors.get(TableId::ArmyOrders));
        let _ = dirty.settle(|slot| {
            let before = self.area_rows.get(slot as usize).map_or(0, |row| row.0);
            self.area_row(orders, slot, columns);
            let after = orders.and_then(|orders| orders.key(slot)).map_or(0, |key| (key >> 32) as u64);
            self.mark_group(before, ORDERS);
            self.mark_group(after, ORDERS);
            Ok(())
        });
        self.area_slots = dirty;
        self.spent(1, taken, began);
        let began = std::time::Instant::now();
        let taken = self.plans.len() as u64;
        for group in std::mem::take(&mut self.plans) {
            self.memberships.remove(&group);
            self.stale.insert(group);
            self.mark_group(group, ORDERS);
        }
        self.spent(2, taken, began);
        let began = std::time::Instant::now();
        let tables = Tables::of(mirrors);
        let mut dirty = std::mem::take(&mut self.provinces);
        let taken = dirty.len() as u64;
        let _ = dirty.settle(|province| {
            if self.admits(province) {
                self.reorder(&tables, mirrors, province, columns);
            } else {
                set(&mut self.out_provinces, province);
            }
            Ok(())
        });
        let taken = taken - dirty.len() as u64;
        self.provinces = dirty;
        self.spent(3, taken, began);
        if std::mem::take(&mut self.wars_new) {
            let slots: Vec<u32> = self.by_destination.values().flatten().copied().collect();
            slots.into_iter().for_each(|slot| self.mark_division(slot, ACTION));
        }
        let movement_complete = mirrors.get(TableId::DivisionMovement).is_some_and(Mirror::complete);
        let acting = Acting {
            facts: action::Facts::new(mirrors, columns.action),
            wars: self.wars.clone(),
            ours: vision.ours(),
            not_ours: if movement_complete { "DivisionMovement division" } else { "DivisionMovement" },
        };
        let began = std::time::Instant::now();
        let mut dirty = std::mem::take(&mut self.divisions);
        let taken = dirty.len() as u64;
        let _ = dirty.settle(|slot| {
            if self.inside(tables.divisions, slot) {
                self.settle_division(&tables, (vision, &relations), &acting, slot);
            } else {
                self.keep_outside(slot);
            }
            Ok(())
        });
        let taken = taken - dirty.len() as u64;
        self.divisions = dirty;
        self.spent(4, taken, began);
    }

    fn rows_carried(&self) -> usize {
        self.division_rows.iter().chain(&self.plan_rows).chain([&self.order_rows, &self.input_rows, &self.destination_rows]).map(Rows::len).sum::<usize>()
            + usize::from(self.transfers_new)
            + usize::from(self.standing_new)
    }

    fn resolve(&mut self, mirrors: &Mirrors) {
        let divisions = mirrors.get(TableId::Divisions);
        for (which, table) in BY_DIVISION.into_iter().enumerate() {
            let cause = if matches!(table, TableId::DivisionStatus | TableId::DivisionPanelStats) { STRENGTH } else { FULL };
            let mirror = mirrors.get(table);
            let mut rows = std::mem::take(&mut self.division_rows[which]);
            rows.settle(|row| match (row, mirror, divisions) {
                (Row::Gone(key), _, _) => self.mark_division_key(mirrors, key, cause),
                (Row::At(slot), Some(mirror), Some(divisions)) => {
                    if let Some(division) = self.division_of(which, mirror, divisions, slot) {
                        self.mark_division(division, cause);
                    }
                }
                _ => {}
            });
            self.division_rows[which] = rows;
        }
        let mut rows = std::mem::take(&mut self.order_rows);
        let order = mirrors.get(TableId::ProvinceUnitOrder);
        rows.settle(|row| {
            if let Some(key) = key_of(order, row) {
                self.mark_province(key as u32);
            }
        });
        self.order_rows = rows;
        for (which, table) in [TableId::OrdersGroupPlans, TableId::ArmyOrderPlans].into_iter().enumerate() {
            let mirror = mirrors.get(table);
            let mut rows = std::mem::take(&mut self.plan_rows[which]);
            rows.settle(|row| {
                if let Some(key) = key_of(mirror, row) {
                    self.plans.insert(if which == 0 { key as u64 } else { (key >> 32) as u64 });
                }
            });
            self.plan_rows[which] = rows;
        }
        let mut rows = std::mem::take(&mut self.input_rows);
        let inputs = mirrors.get(TableId::OrdersGroupCounterInputs);
        rows.settle(|row| {
            if let Some(key) = key_of(inputs, row) {
                self.mark_group(key as u64, ORDERS);
            }
        });
        self.input_rows = rows;
        let mut rows = std::mem::take(&mut self.destination_rows);
        let provinces = mirrors.get(TableId::Provinces);
        rows.settle(|row| {
            let Some(key) = key_of(provinces, row) else { return };
            let mut slots = std::mem::take(&mut self.scratch);
            slots.clear();
            slots.extend(self.by_destination.get(&(key as u32)).into_iter().flatten());
            slots.iter().for_each(|&slot| self.mark_division(slot, ACTION));
            self.scratch = slots;
        });
        self.destination_rows = rows;
        if std::mem::take(&mut self.transfers_new) {
            let slots: Vec<u32> = self.transferring.iter().copied().collect();
            slots.into_iter().for_each(|slot| self.mark_division(slot, ACTION));
        }
        if std::mem::take(&mut self.standing_new) {
            let slots: Vec<u32> = self.standing.iter().copied().collect();
            slots.into_iter().for_each(|slot| self.mark_division(slot, ACTION));
        }
    }

    fn apply_table(&mut self, table: TableId, changes: &Changes, columns: Columns) {
        let frame = self.frame;
        let slots = |dirty: &mut Dirty| {
            for &slot in changes.deleted().slots().iter().chain(changes.inserted().slots()).chain(changes.columns().iter().flat_map(|written| written.slots())) {
                dirty.mark(slot, frame);
            }
        };
        match table {
            TableId::Divisions => {
                let strength = [columns.hit_points.column, columns.organisation.column];
                for &slot in changes.deleted().slots().iter().chain(changes.inserted().slots()) {
                    self.mark_division(slot, FULL);
                }
                for written in changes.columns() {
                    let cause = if strength.contains(&written.column()) { STRENGTH } else { FULL };
                    written.slots().iter().for_each(|&slot| self.mark_division(slot, cause));
                }
            }
            table if BY_DIVISION.contains(&table) => {
                let which = BY_DIVISION.iter().position(|&held| held == table).expect("a division-keyed table");
                self.division_rows[which].take(changes, frame, |_| true);
            }
            TableId::ProvinceUnitOrder => self.order_rows.take(changes, frame, |_| true),
            TableId::OrdersGroups => slots(&mut self.group_slots),
            TableId::ArmyOrders => slots(&mut self.area_slots),
            TableId::OrdersGroupPlans => self.plan_rows[0].take(changes, frame, |_| true),
            TableId::ArmyOrderPlans => self.plan_rows[1].take(changes, frame, |_| true),
            TableId::OrdersGroupCounterInputs => self.input_rows.take(changes, frame, |_| true),
            TableId::CountryLinks if !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| written.column() == columns.link_slot.column && !written.is_empty()) => {
                self.all = true;
            }
            table if action::TABLES.contains(&table) => {
                let rows = !changes.deleted().is_empty() || !changes.inserted().is_empty();
                let reads = |column| columns.action.reads(table, column);
                if !rows && !changes.columns().iter().any(|written| !written.is_empty() && reads(written.column())) {
                    return;
                }
                match table {
                    TableId::Provinces => self.destination_rows.take(changes, frame, reads),
                    TableId::NavalCombatTransfers | TableId::NavalTransferUnits => self.transfers_new = true,
                    _ => self.standing_new = true,
                }
            }
            _ => {}
        }
    }

    pub(super) fn publish(&mut self) {
        if self.touched.is_empty() {
            return;
        }
        let mut changed: Vec<(Source, Option<Arc<[(Member, Action)]>>)> = Vec::new();
        while let Some(source) = self.touched.pop_first() {
            let now: Option<Vec<(Member, Action)>> = self.sources.get(&source).map(|slots| {
                slots.iter().map(|&slot| (self.members[slot as usize].as_ref().expect("a source's member").1.clone(), self.action_of(slot))).collect()
            });
            let same = match (&now, self.published.get(&source)) {
                (Some(now), Some(held)) => now[..] == held[..],
                (None, None) => true,
                _ => false,
            };
            if !same {
                changed.push((source, now.map(Arc::from)));
            }
        }
        if !changed.is_empty() {
            let mut stacks = LandStacks::clone(&self.published);
            for (source, members) in changed {
                match members {
                    Some(members) => stacks.insert(source, members),
                    None => stacks.remove(&source),
                };
            }
            self.published = Arc::new(stacks);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
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
        let mine: BTreeMap<Source, Vec<(&Member, Action)>> = self.sources_acting().filter(|(source, _)| self.admitted(source.0)).map(|(source, members)| (source, members.collect())).collect();
        let theirs: BTreeMap<Source, Vec<(&Member, Action)>> = other.sources_acting().filter(|(source, _)| other.admitted(source.0)).map(|(source, members)| (source, members.collect())).collect();
        for source in mine.keys().chain(theirs.keys()).collect::<std::collections::BTreeSet<_>>() {
            if mine.get(source) != theirs.get(source) {
                note(format!("source {source:?}: {:?} against {:?}", mine.get(source), theirs.get(source)));
            }
        }
        (differing, first)
    }
}

impl Layer for Counters {
    fn held_viewer(&self) -> Option<u32> {
        self.perspective().map(|(_, vision)| vision.ours())
    }

    fn name(&self) -> &'static str {
        "counters"
    }

    fn placed(&mut self, view: &viewer_layer_core::layers::View) -> Option<u64> {
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        Some(u64::from(view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.content.icons_cutoff())))
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        #[cfg(test)]
        let reads_relations = !self.given;
        #[cfg(not(test))]
        let reads_relations = true;
        if reads_relations {
            inputs.after("relations");
        }
        if self.reads_layer {
            inputs.after("vision");
        }
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        let shown = viewer_map_modes::map_mode::visibility::of(&view.map_mode);
        let high = view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.content.icons_cutoff());
        if !shown.unit_counters || shown.unit_kinds != UnitKinds::All || !view.overlay.counters.land || high {
            self.hidden = true;
            return Ok(());
        }
        for table in TABLES {
            inputs.table(table, true)?;
        }
        inputs.hourly(TableId::Divisions)?;
        use TableId as T;
        self.columns = Some(Columns {
            roster_owner: inputs.number(T::Divisions, "roster_owner")?,
            logical_owner: inputs.number(T::Divisions, "logical_owner")?,
            expeditionary_owner: inputs.number(T::Divisions, "expeditionary_owner")?,
            province: inputs.number(T::Divisions, "province")?,
            template: inputs.number(T::Divisions, "template")?,
            hit_points: inputs.number(T::Divisions, "hit_points")?,
            organisation: inputs.number(T::Divisions, "organisation")?,
            orders_group: inputs.number(T::Divisions, "orders_group")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
            mixed: inputs.list(T::ProvinceUnitOrder, "mixed")?,
            separate_counter: inputs.number(T::ArmyIconInputs, "separate_counter")?,
            alternate_template: inputs.number(T::ArmyIconInputs, "alternate_template")?,
            target: inputs.number(T::DivisionMovement, "target")?,
            naval_transfer: inputs.number(T::DivisionMovement, "naval_transfer")?,
            combat_count: inputs.number(T::DivisionMovement, "combat_count")?,
            path_total: inputs.number(T::DivisionMovement, "path_total")?,
            path: inputs.list(T::DivisionMovement, "path")?,
            alternate_total: inputs.number(T::DivisionMovement, "alternate_total")?,
            alternate_path: inputs.list(T::DivisionMovement, "alternate_path")?,
            combat_role: inputs.number(T::DivisionMovement, "combat_role")?,
            combat_province: inputs.number(T::DivisionMovement, "combat_province")?,
            entrenchment: inputs.number(T::DivisionStatus, "entrenchment")?,
            entrenchment_cap: inputs.number(T::DivisionStatus, "entrenchment_cap")?,
            supply_ratio: inputs.number(T::DivisionStatus, "supply_ratio")?,
            attrition_value: inputs.number(T::DivisionStatus, "attrition_value")?,
            attrition_threshold: inputs.number(T::DivisionStatus, "attrition_threshold")?,
            exercising: inputs.number(T::DivisionStatus, "exercising")?,
            has_order: inputs.number(T::DivisionStatus, "has_order")?,
            invalid_orders: inputs.number(T::DivisionStatus, "invalid_orders")?,
            strength_factor: inputs.number(T::DivisionStatus, "strength_factor")?,
            supply_ratio_exact: inputs.number(T::DivisionStatus, "supply_ratio_exact")?,
            panel_hit_points: inputs.number(T::DivisionPanelStats, "panel_stats[0]")?,
            panel_organisation: inputs.number(T::DivisionPanelStats, "panel_stats[1]")?,
            status_hit_points: inputs.number(T::DivisionPanelStats, "status_maxima[0]")?,
            status_organisation: inputs.number(T::DivisionPanelStats, "status_maxima[1]")?,
            group_parent: inputs.number(T::OrdersGroups, "parent")?,
            army_group: inputs.number(T::OrdersGroups, "is_army_group")?,
            leader: inputs.number(T::OrdersGroups, "leader")?,
            hq_provider: inputs.number(T::OrdersGroups, "hq_provider")?,
            primary_orders: inputs.list(T::OrdersGroupPlans, "primary_orders")?,
            fallback_orders: inputs.list(T::OrdersGroupPlans, "fallback_orders")?,
            has_child: inputs.number(T::ArmyOrderPlans, "has_child")?,
            assigned: inputs.list(T::ArmyOrderPlans, "assigned_divisions")?,
            order_type: inputs.number(T::ArmyOrders, "order_type")?,
            target_states: inputs.list(T::ArmyOrders, "target_states")?,
            withdrawing: inputs.number(T::OrdersGroupCounterInputs, "withdrawing")?,
            action: action::Columns::declare(inputs)?,
            definition_token: inputs.number(T::ModifierDefinitions, "token")?,
            literal: inputs.text(T::LexicalTokens, "literal")?,
        });
        if std::mem::take(&mut self.hidden) {
            self.all = true;
        }
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let content = Arc::clone(&self.content);
        content.scoped(|| self.compose_reading(mirrors, None))
    }

    fn moved_at(&self, step: &Step, mirrors: &Mirrors, into: &mut viewer_unit_reads::counter_move::MovedHours) {
        let Some(columns) = self.columns.filter(|_| !self.hidden) else { return };
        viewer_layer_core::layers::moved_provinces(step, mirrors, TableId::Divisions, columns.province.column, viewer_runtime::unit_handle::Unit::Division, into);
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() || self.hidden || self.refused.is_some() {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        let reads = viewer_layer_core::playhead_host::mirror::probe::reads();
        let columns = self.columns();
        for changes in step.tables() {
            let table = TableId::ALL[changes.table()];
            if matches!(table, TableId::ModifierDefinitions | TableId::LexicalTokens) {
                self.layout_new = true;
            }
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.all = true;
                continue;
            }
            self.apply_table(table, changes, columns);
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads += viewer_layer_core::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.take_relations(upstream);
        let content = Arc::clone(&self.content);
        content.scoped(|| self.compose_reading(mirrors, upstream.get::<VisionLayer>().map(Sees::Layer)))
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _settling: Settling) -> io::Result<()> {
        self.take_relations(upstream);
        self.take_vision(upstream, mirrors);
        let content = Arc::clone(&self.content);
        content.scoped(|| self.settle_dirty(mirrors, upstream.get::<VisionLayer>().map(Sees::Layer)));
        Ok(())
    }

    fn viewed(&mut self, _view: &View, _mirrors: &Mirrors) {
        if self.columns.is_some() && self.camera_dirty && self.refused.is_none() && !self.hidden {
            self.readmit();
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        let content = Arc::clone(&self.content);
        content.scoped(|| self.settle_dirty(mirrors, None));
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>) -> io::Result<()> {
        self.publish();
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.touched.is_empty()
    }

    fn carried(&self) -> Carried {
        let entities = self.divisions.len() + self.provinces.len() + self.group_slots.len() + self.area_slots.len() + self.plans.len() + self.rows_carried() + usize::from(self.all) + usize::from(self.layout_new);
        Carried { entities }
    }

    fn held(&self) -> usize {
        self.members.capacity() * std::mem::size_of::<Option<(Source, Member)>>()
            + self.sources.values().map(|slots| slots.capacity() * 4 + 48).sum::<usize>()
            + self.by_group.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + self.children.values().map(|groups| groups.capacity() * 8 + 40).sum::<usize>()
            + self.group_rows.capacity() * 24
            + self.area_rows.capacity() * 16
            + (self.hq.capacity() + self.area.capacity()) * 16
            + self.memberships.values().map(|sets| sets.iter().map(AssignedOrders::bytes).sum::<usize>() + 64).sum::<usize>()
            + self.divisions.bytes()
            + (self.out.capacity() + self.out_provinces.capacity() + self.asked.capacity() + self.admitted.capacity()) * 8
            + self.rows_of.iter().map(|rows| rows.capacity() * 4).sum::<usize>()
            + self.stale.capacity() * 8
            + self.scratch.capacity() * 4
            + self.causes.capacity()
            + self.provinces.bytes()
            + self.group_slots.bytes()
            + self.area_slots.bytes()
            + self.plans.capacity() * 8
            + self.published.values().map(|members| members.len() * std::mem::size_of::<(Member, Action)>() + 64).sum::<usize>()
            + self.actions.capacity() * std::mem::size_of::<Action>()
            + self.reads.capacity() * std::mem::size_of::<action::Read>()
            + self.by_destination.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + (self.transferring.capacity() + self.standing.capacity()) * 8
            + self.touched.len() * 32
            + self.division_rows.iter().chain(&self.plan_rows).chain([&self.order_rows, &self.input_rows, &self.destination_rows]).map(Rows::bytes).sum::<usize>()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

pub fn published(side: viewer_layer_core::playhead_host::LayerParts<'_>) -> Option<Arc<LandStacks>> {
    if !side.draws("counters") {
        return None;
    }
    let (_, layers) = side.parts()?;
    layers.iter().find_map(|layer| layer.any().downcast_ref::<fed::Fed>()).and_then(fed::Fed::stacks).cloned()
}

struct Acting<'a> {
    facts: action::Facts<'a>,
    wars: Option<Arc<relations::Facts>>,
    ours: u32,
    not_ours: &'static str,
}

mod action;
pub mod fed;
#[cfg(test)]
mod tests;

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<viewer_layer_core::comparison::Compared> {
    let layer = layer.any().downcast_ref::<fed::Fed>().expect("the counters line's layer");
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    fresh.settle_with(mirrors, upstream, Settling::Whole)?;
    fresh.publish();
    let mut compared = viewer_layer_core::comparison::Compared::default();
    if layer.refused() != fresh.refused() {
        compared.entries = 1;
        compared.differing = 1;
        compared.first = Some(format!("the product refuses {:?}, composed anew {:?}", layer.refused(), fresh.refused()));
        return Ok(compared);
    }
    let product = layer.stacks().cloned().unwrap_or_default();
    let sources: std::collections::BTreeSet<Source> = product.keys().chain(fresh.published.keys()).filter(|source| layer.admitted(source.0)).collect();
    for source in sources {
        let (mine, theirs) = (product.get(&source), fresh.published.get(&source));
        compared.entries += 1 + mine.map_or(0, |members| members.len()).max(theirs.map_or(0, |members| members.len())) as u64;
        if mine.map(|members| &members[..]) != theirs.map(|members| &members[..]) {
            compared.differing += 1;
            compared.first.get_or_insert_with(|| format!("source {source:?}: the product {mine:?}, composed anew {theirs:?}"));
        }
    }
    compared.entries = compared.entries.max(1);
    Ok(compared)
}
