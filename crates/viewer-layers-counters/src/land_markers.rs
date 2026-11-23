
use viewer_layer_core::{layers::{Carried, Change, Settling, Dirty, Inputs, KeptAt, Keys, Layer, List, Num, Upstream, View}, engagements::{Engagements, Index}};
use super::{counters::key_of, naval_counters::Admits};
use viewer_layer_core::counter_rows::Rows;
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use content_reader_core::movement::MovementMap;
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::rows::{DivisionMovementStats, DivisionPanelStatsRow, Field, LandCombatDivisionRow, LandCombatMemberValues, LandCombatSideDetails, NameOperands, NativeValue, Opaque, Recorded, TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_counter_kit::combat_markers::{CombatFields, Held, LandCombatDefines, MarkerJoins, MarkerOutcome, SideFields, TintHeld, land_marker, marker_tint, missing_of};
use viewer_engagements::{
    engaged::Engaged,
    engagement_history::{LAND, LAND_ICON, Provenance},
    engagement_progress::{attacker_progress, defending_of},
    icon_frames::Kept,
    land_combat_math::{Condition, ScoreDetail, ScoreJoins, side_scores_of},
};
use viewer_gpu::camera::{Placement, Pose};
use viewer_unit_reads::counter_move::UnitRoute;
use viewer_foundation::{perspective::{Choice, SharingUnknown, Vision}, unit_rules::STANDSTILL};
use viewer_runtime::unit_class::CounterClass;

pub trait Front: Send + Sync {
    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]>;
    fn movement(&self) -> Option<&MovementMap>;
    fn defines(&self) -> LandCombatDefines;
    fn cutoff(&self) -> f32;
    fn admits(&self, placement: Placement, viewport: [u32; 2]) -> Option<Admits>;
}


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

pub(super) fn bytes_of(cell: Cell<&[u8]>) -> &[u8] {
    cell.value().unwrap_or(&[])
}

fn u32s(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|word| u32::from_le_bytes(word.try_into().expect("four bytes"))).collect()
}

pub(super) fn u64s(bytes: &[u8]) -> Vec<u64> {
    bytes.chunks_exact(8).map(|word| u64::from_le_bytes(word.try_into().expect("eight bytes"))).collect()
}

pub(super) fn opaque<T: NativeValue>(mirror: &Mirror, column: List, slot: u32) -> Option<T> {
    let Cell::Value(bytes) = mirror.list(column, slot) else { return None };
    T::from_field(Field::Opaque(Opaque::Encoded(bytes, mirror.format()))).ok()
}

pub(super) fn recorded<T: NativeValue>(mirror: &Mirror, column: List, slot: u32) -> Recorded<T> {
    match mirror.list(column, slot) {
        Cell::Value(bytes) => <Recorded<T> as NativeValue>::from_field(Field::Opaque(Opaque::Encoded(bytes, mirror.format()))).unwrap_or(Recorded::NotRecorded),
        _ => Recorded::NotRecorded,
    }
}

fn native(mirror: &Mirror, column: Num<i64>, slot: u32) -> Cell<i64> {
    match (mirror.number(column, slot), column.table.display_step(column.column)) {
        (Cell::Value(units), Some(step)) => step.value(units).map_or(Cell::Unrecorded, Cell::Value),
        (cell, _) => cell,
    }
}

pub(super) fn held(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

pub(super) fn held_as(mirror: Option<&Mirror>) -> Held<'_> {
    match mirror.map(|mirror| (mirror, mirror.status())) {
        Some((_, Some(Status::Complete))) => Held::Complete,
        Some((_, Some(Status::Truncated { missing }))) => Held::Truncated(missing),
        Some((mirror, Some(Status::NotRecorded))) => match mirror.capture() {
            Some(mode) => Held::NotRecorded(mode),
            None => Held::Unavailable(mirror.reason().unwrap_or("not recorded")),
        },
        Some((mirror, _)) => Held::Unavailable(mirror.reason().unwrap_or("not recorded here")),
        None => Held::Unavailable("not read"),
    }
}

const PANEL: [usize; 12] = [0, 1, 3, 9, 10, 11, 12, 13, 14, 15, 20, 21];

#[derive(Clone, Copy)]
struct Columns {
    combat_province: Num<u32>,
    attackers: List,
    defenders: List,
    indicator: Num<u8>,
    side_flags: List,
    attacker_primary: List,
    defender_primary: List,
    defender_secondary: List,
    attacker_bonus: Num<i64>,
    defender_bonus: Num<i64>,
    detail_sides: List,
    member_division: Num<u64>,
    scales: [Num<i64>; 4],
    own: List,
    province: Num<u32>,
    hit_points: Num<i64>,
    organisation: Num<i64>,
    panel: [Option<Num<i64>>; 22],
    status_maxima: [Num<i64>; 2],
    naming: List,
    movement_stats: List,
    strength_factor: Num<i64>,
    map_entity: Num<u8>,
    fuel: Num<i64>,
    target: Num<u32>,
    naval_transfer: Num<u8>,
    combat_count: Num<u32>,
    path_total: Num<u32>,
    path: List,
    alternate_total: Num<u32>,
    alternate_path: List,
    combat_role: Num<u8>,
    route_combat_province: Num<u32>,
    link_slot: Num<u32>,
}

impl Columns {
    fn reads(&self, table: TableId, column: usize) -> bool {
        let lists = |lists: &[List]| lists.iter().any(|list| list.column == column);
        match table {
            TableId::Divisions => [self.province.column, self.hit_points.column, self.organisation.column].contains(&column),
            TableId::DivisionPanelStats => self.panel.iter().flatten().chain(&self.status_maxima).any(|held| held.column == column),
            TableId::DivisionStatus => self.strength_factor.column == column || lists(&[self.naming, self.movement_stats]),
            TableId::DivisionMovement => {
                [self.target.column, self.combat_count.column, self.path_total.column, self.alternate_total.column, self.route_combat_province.column].contains(&column)
                    || [self.map_entity.column, self.naval_transfer.column, self.combat_role.column].contains(&column)
                    || self.fuel.column == column
                    || lists(&[self.path, self.alternate_path])
            }
            TableId::LandCombatSides => self.indicator.column == column || lists(&[self.side_flags, self.attacker_primary, self.defender_primary, self.defender_secondary]),
            TableId::LandCombatDetails => [self.attacker_bonus.column, self.defender_bonus.column].contains(&column) || lists(&[self.detail_sides]),
            _ => true,
        }
    }
}

const TABLES: [TableId; 9] = [
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::Divisions,
    TableId::DivisionPanelStats,
    TableId::DivisionStatus,
    TableId::DivisionMovement,
    TableId::CountryLinks,
];

const BY_DIVISION: [TableId; 4] = [TableId::Divisions, TableId::DivisionPanelStats, TableId::DivisionStatus, TableId::DivisionMovement];

struct Tables<'a> {
    combats: Option<&'a Mirror>,
    sides: Option<&'a Mirror>,
    details: Option<&'a Mirror>,
    members: Option<&'a Mirror>,
    divisions: Option<&'a Mirror>,
    panels: Option<&'a Mirror>,
    status: Option<&'a Mirror>,
    movement: Option<&'a Mirror>,
    links: Option<&'a Mirror>,
    held: TintHeld<'a>,
    sides_held: Held<'a>,
}

impl<'a> Tables<'a> {
    fn of(mirrors: &'a Mirrors) -> Self {
        let get = |table| held(mirrors.get(table));
        Self {
            combats: get(TableId::LandCombats),
            sides: get(TableId::LandCombatSides),
            details: get(TableId::LandCombatDetails),
            members: get(TableId::LandCombatDivisions),
            divisions: get(TableId::Divisions),
            panels: get(TableId::DivisionPanelStats),
            status: get(TableId::DivisionStatus),
            movement: get(TableId::DivisionMovement),
            links: get(TableId::CountryLinks),
            held: TintHeld {
                details: held_as(mirrors.get(TableId::LandCombatDetails)),
                divisions: held_as(mirrors.get(TableId::Divisions)),
                panels: held_as(mirrors.get(TableId::DivisionPanelStats)),
                members: held_as(mirrors.get(TableId::LandCombatDivisions)),
            },
            sides_held: held_as(mirrors.get(TableId::LandCombatSides)),
        }
    }
}

fn row(mirror: Option<&Mirror>, key: u64) -> Option<(&Mirror, u32)> {
    let mirror = mirror?;
    Some((mirror, mirror.slot(u128::from(key))?))
}

fn member_of(key: u128) -> (u64, u8) {
    ((key >> 33) as u64, ((key >> 32) & 1) as u8)
}

struct Joins<'a> {
    field: &'a dyn Front,
    tables: &'a Tables<'a>,
    columns: Columns,
    vision: &'a Vision,
    flags: &'a BTreeMap<u64, Engaged>,
    kept: &'a HashMap<u64, Kept>,
    defines: LandCombatDefines,
    members: &'a [u32],
}

impl Joins<'_> {
    fn organisation(&self, division: u64) -> Option<i64> {
        let (divisions, slot) = row(self.tables.divisions, division)?;
        native(divisions, self.columns.organisation, slot).value()
    }

    fn values(&self, member: u32) -> Option<LandCombatMemberValues> {
        let (columns, members) = (self.columns, self.tables.members?);
        let own: Option<LandCombatMemberValues> = opaque::<Option<LandCombatMemberValues>>(members, columns.own, member).flatten();
        let division = members.number(columns.member_division, member).value()?;
        LandCombatDivisionRow::values_of(own.as_ref(), || {
            let (panels, panel) = row(self.tables.panels, division)?;
            let (status, at) = row(self.tables.status, division)?;
            let (movement, moving) = row(self.tables.movement, division)?;
            let naming: Recorded<Option<NameOperands>> = recorded(status, columns.naming, at);
            let stats: Recorded<Option<DivisionMovementStats>> = recorded(status, columns.movement_stats, at);
            let fuel = match movement.number(columns.fuel, moving) {
                Cell::Value(fuel) => Recorded::Value(Some(fuel)),
                Cell::Missing => Recorded::Value(None),
                Cell::Unrecorded => Recorded::NotRecorded,
            };
            let word = |index: usize| columns.panel[index].and_then(|column| native(panels, column, panel).value());
            LandCombatMemberValues::joined_of(&word, &naming, &stats, native(status, columns.strength_factor, at).value()?, fuel)
        })
    }
}

struct Members<'a> {
    joins: &'a Joins<'a>,
    by: BTreeMap<(u8, u64), (Option<[i64; 5]>, [i64; 4])>,
}

impl ScoreJoins for Members<'_> {
    fn member(&self, side: u8, division: u64) -> Option<(Option<[i64; 5]>, [i64; 4])> {
        self.by.get(&(side, division)).copied()
    }

    fn condition(&self, division: u64) -> Option<Condition> {
        let (columns, tables) = (self.joins.columns, self.joins.tables);
        let (divisions, slot) = row(tables.divisions, division)?;
        let panel = row(tables.panels, division);
        let maximum = |at: usize| {
            let (panels, slot) = panel?;
            let word = columns.panel[at].and_then(|column| native(panels, column, slot).value());
            DivisionPanelStatsRow::shown_maximum(native(panels, columns.status_maxima[at], slot).value(), word)
        };
        Some(Condition {
            strength: native(divisions, columns.hit_points, slot).value()?,
            max_strength: maximum(0),
            organisation: native(divisions, columns.organisation, slot).value()?,
            max_organisation: maximum(1),
        })
    }
}

impl MarkerJoins for Joins<'_> {
    fn sides(&self, combat: u64) -> Result<SideFields, String> {
        let columns = self.columns;
        let Some((sides, slot)) = row(self.tables.sides, combat) else { return Err(missing_of(self.tables.sides_held, "LandCombatSides", "the combat")) };
        Ok(SideFields {
            indicator: sides.number(columns.indicator, slot).value().unwrap_or(0),
            flags: opaque::<[u32; 2]>(sides, columns.side_flags, slot).unwrap_or_default(),
            attacker_primary: u32s(bytes_of(sides.list(columns.attacker_primary, slot))),
            defender_primary: u32s(bytes_of(sides.list(columns.defender_primary, slot))),
            defender_secondary: u32s(bytes_of(sides.list(columns.defender_secondary, slot))),
        })
    }

    fn first_attacker(&self, attackers: &[u64]) -> Result<Option<(u32, u32)>, &'static str> {
        let columns = self.columns;
        for &id in attackers {
            let Some((movement, slot)) = row(self.tables.movement, id) else {
                return Err("an attacker's map entity is unrecorded (it has no DivisionMovement row), so the icon's first mapped attacker is unknown");
            };
            if movement.number(columns.map_entity, slot).value().unwrap_or(0) == 0 {
                continue;
            }
            let province = row(self.tables.divisions, id).and_then(|(divisions, at)| divisions.number(columns.province, at).value());
            let number = |column: Num<u32>| movement.number(column, slot).value().unwrap_or(0);
            let path = u32s(bytes_of(movement.list(columns.path, slot)));
            let alternate = u32s(bytes_of(movement.list(columns.alternate_path, slot)));
            let route = UnitRoute {
                class: CounterClass::Division,
                combat_role: movement.number(columns.combat_role, slot).value().unwrap_or(0),
                combat_count: number(columns.combat_count),
                combat_province: number(columns.route_combat_province),
                target: number(columns.target),
                naval_transfer: movement.number(columns.naval_transfer, slot).value().unwrap_or(0) != 0,
                path_total: number(columns.path_total),
                path: &path,
                alternate_total: number(columns.alternate_total),
                alternate_path: &alternate,
            };
            let placed = route.resolve(province.unwrap_or(0), self.field.movement(), &|at| self.field.anchor(at, STANDSTILL));
            let province = province.ok_or("the first mapped attacker's province is unrecorded (it has no Divisions row)")?;
            let kind = placed.kind_resolved.then_some(placed.stack_kind).ok_or("the first mapped attacker's stack kind is unknown (content cannot place its route)")?;
            return Ok(Some((province, kind)));
        }
        Ok(None)
    }

    fn flag(&self, combat: u64) -> Engaged {
        self.flags.get(&combat).copied().unwrap_or(Engaged::Unknown)
    }

    fn kept(&self, combat: u64) -> Kept {
        self.kept.get(&combat).copied().unwrap_or(Kept::Undecided)
    }

    fn defending(&self, sides: &SideFields) -> Result<bool, SharingUnknown> {
        let links = self.tables.links;
        let resolve = |raw: u32| {
            let links = links?;
            links.number(self.columns.link_slot, links.slot(u128::from(raw))?).value()
        };
        defending_of(&sides.defender_primary, &sides.defender_secondary, self.vision, &resolve)
    }

    fn tint(&self, combat: &CombatFields<'_>, defending: bool) -> (Option<[f32; 4]>, Option<String>) {
        let columns = self.columns;
        let scores = row(self.tables.details, combat.id).and_then(|(details, slot)| {
            let sides = opaque::<[LandCombatSideDetails; 2]>(details, columns.detail_sides, slot)?;
            let detail = ScoreDetail {
                attacker_bonus: details.number(columns.attacker_bonus, slot).value().unwrap_or(0),
                defender_bonus: details.number(columns.defender_bonus, slot).value().unwrap_or(0),
                sides: &sides,
            };
            let members = self.tables.members;
            let mut by = BTreeMap::new();
            for &member in self.members {
                let Some(members) = members else { break };
                let Some(key) = members.key(member) else { continue };
                let Some(division) = members.number(columns.member_division, member).value() else { continue };
                let scales = columns.scales.map(|column| native(members, column, member).value().unwrap_or(0));
                by.insert((member_of(key).1, division), (self.values(member).map(|values| values.stat_values), scales));
            }
            Some(side_scores_of(detail, &Members { joins: self, by }, [combat.attackers, combat.defenders], self.defines.scores))
        });
        marker_tint(scores, defending, &self.defines, self.tables.held)
    }

    fn progress(&self, combat: &CombatFields<'_>) -> Result<i64, u64> {
        let sum = |ids: &[u64]| ids.iter().try_fold(0_i64, |total, &id| Ok(total.wrapping_add(self.organisation(id).ok_or(id)?)));
        sum(combat.attackers).and_then(|attacker| Ok(attacker_progress(attacker, sum(combat.defenders)?)))
    }

    fn missing_division(&self, division: u64) -> String {
        missing_of(self.tables.held.divisions, "Divisions", format!("division {division}"))
    }
}

#[derive(Clone, Debug)]
struct Made {
    id: u64,
    province: u32,
    outcome: MarkerOutcome,
    divisions: Vec<u64>,
    kept: Kept,
}

pub struct LandMarkers {
    field: Arc<dyn Front>,
    index: Option<Index>,
    columns: Option<Columns>,
    refused: Option<String>,
    outcomes: Vec<Option<Made>>,
    by_division: HashMap<u64, Vec<u32>>,
    members: HashMap<u64, Vec<u32>>,
    member_rows: Vec<u64>,
    member_slots: Dirty,
    combats: Dirty,
    unseen: HashMap<u64, u32>,
    kept: HashMap<u64, Kept>,
    kept_at: KeptAt,
    kept_owed: Keys,
    frames_read: usize,
    side_rows: [Rows; 2],
    division_rows: [Rows; 4],
    flags: Option<(BTreeMap<u64, Engaged>, Provenance)>,
    limit: Option<&'static str>,
    all: bool,
    hidden: bool,
    stepped: bool,
    vision: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision_new: AtomicBool,
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
    version: u64,
    published: Option<(u64, Arc<Product>)>,
    #[cfg(any(test, feature = "test-support"))]
    made: usize,
    #[cfg(any(test, feature = "test-support"))]
    applied_reads: u64,
}

pub type Product = viewer_counter_kit::combat_markers::LandMarkers;

impl LandMarkers {
    pub fn new(field: Arc<dyn Front>, index: Option<Index>) -> Self {
        Self {
            field,
            index,
            columns: None,
            refused: Some("not composed yet".into()),
            outcomes: Vec::new(),
            by_division: HashMap::new(),
            members: HashMap::new(),
            member_rows: Vec::new(),
            member_slots: Dirty::default(),
            combats: Dirty::default(),
            unseen: HashMap::new(),
            kept: HashMap::new(),
            kept_at: KeptAt::Unread,
            kept_owed: Keys::default(),
            frames_read: 0,
            side_rows: Default::default(),
            division_rows: Default::default(),
            flags: None,
            limit: None,
            all: true,
            hidden: false,
            stepped: false,
            vision: Mutex::new(None),
            vision_new: AtomicBool::new(false),
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
            version: 1,
            published: None,
            #[cfg(any(test, feature = "test-support"))]
            made: 0,
            #[cfg(any(test, feature = "test-support"))]
            applied_reads: 0,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn made(&self) -> usize {
        self.made
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    pub fn fresh(&self) -> Self {
        let index = self.index.as_ref().map(|index| Index { history: Arc::clone(&index.history), branch: index.branch });
        let fresh = Self { columns: self.columns, hidden: self.hidden, camera: self.camera, admits: self.admits.clone(), flags: self.flags.clone(), ..Self::new(Arc::clone(&self.field), index) };
        if let Some((choice, vision)) = self.perspective() {
            fresh.set_vision(choice, vision);
        }
        fresh
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
        self.columns.expect("the land markers layer is declared before it composes")
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    pub fn marked(&self) -> usize {
        self.marked
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn frames_read(&self) -> usize {
        self.frames_read
    }

    pub fn outcomes(&self) -> impl Iterator<Item = (u64, u32, &MarkerOutcome)> + '_ {
        self.outcomes.iter().flatten().map(|made| (made.id, made.province, &made.outcome))
    }

    pub fn limit(&self) -> Option<&'static str> {
        self.limit
    }

    pub fn published(&self) -> Option<&Arc<Product>> {
        self.published.as_ref().filter(|_| self.refused.is_none() && !self.hidden).map(|(_, product)| product)
    }

    fn publish(&mut self) {
        if self.refused.is_none() && !self.hidden && self.published.as_ref().is_none_or(|(version, _)| *version != self.version) {
            self.published = Some((self.version, Arc::new(self.product())));
        }
    }

    fn product(&self) -> Product {
        let mut made: Vec<&Made> = self.outcomes.iter().flatten().filter(|made| self.admitted(made.province)).collect();
        made.sort_unstable_by_key(|made| made.id);
        let mut out = Product::default();
        if let Some(limit) = self.limit
            && !made.is_empty()
        {
            out.limits.insert(limit.to_owned(), made.iter().map(|made| made.id).collect());
        }
        for made in made {
            for cause in &made.outcome.gaps {
                out.gaps.entry(cause.clone()).or_default().insert(made.id);
            }
            for limit in &made.outcome.limits {
                out.limits.entry(limit.clone()).or_default().insert(made.id);
            }
            out.markers.extend(made.outcome.marker.clone());
        }
        out
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

    fn readmit(&mut self) {
        self.camera_dirty = false;
        let admitted = |layer: &Self| -> Vec<u64> { layer.outcomes.iter().flatten().filter(|made| layer.admitted(made.province)).map(|made| made.id).collect() };
        let before = admitted(self);
        self.admits = self.camera.and_then(|(placement, viewport)| self.field.admits(placement, viewport));
        self.asked.clear();
        self.admitted.clear();
        if admitted(self) != before {
            self.version += 1;
        }
        let out: Vec<u32> = ones(&self.out).collect();
        self.out.clear();
        self.outside = 0;
        out.into_iter().for_each(|slot| self.mark(slot));
    }

    fn mark(&mut self, slot: u32) {
        if self.combats.mark(slot, self.frame) {
            self.marked += 1;
        }
    }

    fn inside(&mut self, combats: Option<&Mirror>, slot: u32) -> bool {
        self.admits.is_none() || {
            let columns = self.columns();
            let standing = self.outcomes.get(slot as usize).and_then(Option::as_ref).map(|made| made.province);
            let now = combats.filter(|combats| combats.live(slot)).and_then(|combats| combats.number(columns.combat_province, slot).value());
            [standing, now].into_iter().flatten().any(|province| self.admits(province))
        }
    }

    fn mark_combat(&mut self, mirrors: &Mirrors, combat: u64) {
        if let Some(slot) = mirrors.get(TableId::LandCombats).and_then(|combats| combats.slot(u128::from(combat))) {
            self.mark(slot);
        }
    }

    fn mark_division(&mut self, division: u64) {
        let Some(slots) = self.by_division.get(&division) else { return };
        for at in 0..slots.len() {
            let slot = self.by_division[&division][at];
            if self.combats.mark(slot, self.frame) {
                self.marked += 1;
            }
        }
    }

    fn mark_all(&mut self, mirrors: &Mirrors) {
        let combats = mirrors.get(TableId::LandCombats);
        let mut slots: Vec<u32> = combats.map(|combats| combats.rows().collect()).unwrap_or_default();
        slots.extend(self.outcomes.iter().enumerate().filter(|(_, made)| made.is_some()).map(|(slot, _)| slot as u32));
        slots.into_iter().for_each(|slot| self.mark(slot));
    }

    fn rows_carried(&self) -> usize {
        self.side_rows.iter().chain(&self.division_rows).map(Rows::len).sum()
    }

    fn resolve(&mut self, mirrors: &Mirrors) {
        for (which, table) in [TableId::LandCombatSides, TableId::LandCombatDetails].into_iter().enumerate() {
            let mirror = mirrors.get(table);
            let mut rows = std::mem::take(&mut self.side_rows[which]);
            rows.settle(|row| {
                if let Some(key) = key_of(mirror, row) {
                    self.mark_combat(mirrors, key as u64);
                }
            });
            self.side_rows[which] = rows;
        }
        for (which, table) in BY_DIVISION.into_iter().enumerate() {
            let mirror = mirrors.get(table);
            let mut rows = std::mem::take(&mut self.division_rows[which]);
            rows.settle(|row| {
                if let Some(key) = key_of(mirror, row) {
                    self.mark_division(key as u64);
                }
            });
            self.division_rows[which] = rows;
        }
    }

    fn core(&self, mirrors: &Mirrors) -> Result<Arc<Vision>, String> {
        let (_, vision) = self.perspective().ok_or("the viewing country's vision is not set")?;
        let links = mirrors.get(TableId::CountryLinks).ok_or("CountryLinks is not read yet")?;
        if let Some(why) = links.not_recorded() {
            return Err(format!("{why}; the land markers need every country link"));
        }
        let combats = mirrors.get(TableId::LandCombats).ok_or("LandCombats is not read yet")?;
        if combats.status().is_none() {
            return Err("LandCombats is not recorded here: a gap in the recording".into());
        }
        Ok(vision)
    }

    fn member_row(&mut self, mirrors: &Mirrors, slot: u32) {
        let at = slot as usize;
        if self.member_rows.len() <= at {
            self.member_rows.resize(at + 1, u64::MAX);
        }
        let old = self.member_rows[at];
        if old != u64::MAX
            && let Some(slots) = self.members.get_mut(&old)
        {
            slots.retain(|&held| held != slot);
            if slots.is_empty() {
                self.members.remove(&old);
            }
        }
        let members = held(mirrors.get(TableId::LandCombatDivisions));
        let new = members.filter(|members| members.live(slot)).and_then(|members| members.key(slot)).map_or(u64::MAX, |key| member_of(key).0);
        if new != u64::MAX {
            let keys = members.expect("a live member's table");
            let slots = self.members.entry(new).or_default();
            let at = slots.partition_point(|&held| keys.key(held) < keys.key(slot));
            slots.insert(at, slot);
        }
        self.member_rows[at] = new;
        for combat in [old, new].into_iter().filter(|&combat| combat != u64::MAX) {
            self.mark_combat(mirrors, combat);
        }
    }

    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.field), None);
        let kept = std::mem::replace(self, empty);
        (self.index, self.columns, self.hidden, self.flags, self.frame, self.marked) = (kept.index, kept.columns, kept.hidden, kept.flags, kept.frame, kept.marked);
        (self.camera, self.camera_dirty, self.admits) = (kept.camera, kept.camera_dirty, kept.admits);
        (self.vision, self.vision_new) = (kept.vision, kept.vision_new);
        #[cfg(any(test, feature = "test-support"))]
        {
            (self.made, self.applied_reads) = (kept.made, kept.applied_reads);
        }
    }

    fn refuse(&mut self, why: String) {
        if self.refused.as_ref() != Some(&why) {
            self.release();
            self.refused = Some(why);
        }
        self.all = false;
    }

    fn recompose(&mut self, mirrors: &Mirrors) {
        self.version += 1;
        self.outcomes.clear();
        self.unseen.clear();
        self.kept.clear();
        self.kept_at = KeptAt::Unread;
        self.kept_owed.clear();
        self.by_division.clear();
        self.members.clear();
        self.member_rows.clear();
        self.member_slots.clear();
        self.combats.clear();
        self.side_rows.iter_mut().chain(&mut self.division_rows).for_each(Rows::clear);
        self.out.clear();
        self.outside = 0;
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        if let Some(members) = held(mirrors.get(TableId::LandCombatDivisions)) {
            let slots: Vec<u32> = members.rows().collect();
            slots.into_iter().for_each(|slot| {
                self.member_slots.mark(slot, self.frame);
            });
        }
        if self.camera_dirty {
            self.readmit();
        }
        self.mark_all(mirrors);
    }

    fn live(&self, mirrors: &Mirrors) -> Vec<u64> {
        let mut live: Vec<u64> = held(mirrors.get(TableId::LandCombats)).map(|combats| combats.rows().filter_map(|slot| combats.key(slot)).map(|key| key as u64).collect()).unwrap_or_default();
        live.sort_unstable();
        live
    }

    fn kept_of(&self, mirrors: &Mirrors, vision: &Vision, combats: &[u64]) -> HashMap<u64, Kept> {
        let head = mirrors.get(TableId::LandCombats).and_then(Mirror::head);
        let found = match (&self.index, head) {
            (Some(index), Some(head)) if !combats.is_empty() => index.history.lookup_icons(LAND_ICON, index.branch, head, vision.ours(), combats).ok(),
            _ => None,
        };
        match found {
            Some(found) => found.into_iter().collect(),
            None => combats.iter().map(|&combat| (combat, Kept::Unindexed)).collect(),
        }
    }

    fn take_kept(&mut self, mirrors: &Mirrors, vision: &Vision) -> HashMap<u64, Kept> {
        let index = self.index.as_ref().map(|index| (&index.history, index.branch));
        let now = KeptAt::of(index, mirrors.get(TableId::LandCombats).and_then(Mirror::head), vision.ours());
        let advanced = self.kept_at.advance(now, index, LAND_ICON, vision.ours());
        self.kept_at = now;
        let mut owed = std::mem::take(&mut self.kept_owed);
        let mut combats: Vec<u64> = match advanced {
            Some(changed) => owed.take().into_iter().chain(changed).collect(),
            None => {
                self.kept.clear();
                owed.clear();
                self.live(mirrors)
            }
        };
        self.kept_owed = owed;
        combats.sort_unstable();
        combats.dedup();
        let rows = mirrors.get(TableId::LandCombats);
        let live = |combat: u64| held(rows).is_some_and(|rows| rows.slot(u128::from(combat)).is_some());
        let (asked, gone): (Vec<u64>, Vec<u64>) = combats.into_iter().partition(|&combat| live(combat));
        let found = self.kept_of(mirrors, vision, &asked);
        self.frames_read += asked.len();
        for combat in gone {
            self.kept.remove(&combat);
            self.mark_unseen(combat);
        }
        for combat in asked {
            match found.get(&combat).copied() {
                Some(kept) => self.kept.insert(combat, kept),
                None => self.kept.remove(&combat),
            };
            self.mark_unseen(combat);
        }
        std::mem::take(&mut self.kept)
    }

    fn mark_unseen(&mut self, combat: u64) {
        let Some(&slot) = self.unseen.get(&combat) else { return };
        let used = self.outcomes.get(slot as usize).and_then(Option::as_ref).map(|made| made.kept);
        if used != Some(self.kept.get(&combat).copied().unwrap_or(Kept::Undecided)) {
            self.mark(slot);
        }
    }

    fn settle_combat(&mut self, tables: &Tables<'_>, joins: (&Vision, &BTreeMap<u64, Engaged>, &HashMap<u64, Kept>), slot: u32) {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.made += 1;
        }
        let at = slot as usize;
        if self.outcomes.len() <= at {
            self.outcomes.resize(at + 1, None);
        }
        let old = self.outcomes[at].take();
        let held_id = old.as_ref().map(|made| made.id);
        self.make_combat(tables, joins, slot);
        fn laid(made: &Option<Made>) -> Option<(u64, u32, &MarkerOutcome)> {
            made.as_ref().map(|made| (made.id, made.province, &made.outcome))
        }
        if laid(&old) != laid(&self.outcomes[at]) {
            self.version += 1;
        }
        if let Some(made) = old {
            for division in made.divisions {
                if let Some(slots) = self.by_division.get_mut(&division) {
                    slots.retain(|&held| held != slot);
                    if slots.is_empty() {
                        self.by_division.remove(&division);
                    }
                }
            }
        }
        if let Some(made) = &self.outcomes[at] {
            for &division in &made.divisions {
                self.by_division.entry(division).or_default().push(slot);
            }
        }
        if let Some(id) = held_id
            && self.unseen.get(&id) == Some(&slot)
        {
            self.unseen.remove(&id);
        }
        if let Some(made) = &self.outcomes[at]
            && made.outcome.marker.as_ref().is_some_and(|marker| !marker.seen)
        {
            self.unseen.insert(made.id, slot);
        }
    }

    fn make_combat(&mut self, tables: &Tables<'_>, joins: (&Vision, &BTreeMap<u64, Engaged>, &HashMap<u64, Kept>), slot: u32) {
        let at = slot as usize;
        let columns = self.columns();
        let Some(combats) = tables.combats.filter(|combats| combats.live(slot)) else { return };
        let Some(id) = combats.key(slot).map(|key| key as u64) else { return };
        let Some(province) = combats.number(columns.combat_province, slot).value() else { return };
        let attackers = u64s(bytes_of(combats.list(columns.attackers, slot)));
        let defenders = u64s(bytes_of(combats.list(columns.defenders, slot)));
        let member_slots = self.members.get(&id).cloned().unwrap_or_default();
        let (vision, flags, kept) = joins;
        let outcome = {
            let joins = Joins { field: &*self.field, tables, columns, vision, flags, kept, defines: self.field.defines(), members: &member_slots };
            land_marker(CombatFields { id, province, attackers: &attackers, defenders: &defenders }, &joins)
        };
        let mut divisions: BTreeSet<u64> = attackers.iter().chain(&defenders).copied().collect();
        if let Some(members) = tables.members {
            divisions.extend(member_slots.iter().filter_map(|&member| members.number(columns.member_division, member).value()));
        }
        let divisions: Vec<u64> = divisions.into_iter().collect();
        let used = kept.get(&id).copied().unwrap_or(Kept::Undecided);
        self.outcomes[at] = Some(Made { id, province, outcome, divisions, kept: used });
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors, upstream: Option<(&Engagements, &[u32], bool, bool)>) {
        self.settle_marks(mirrors, upstream);
        self.frame += 1;
    }

    fn settle_marks(&mut self, mirrors: &Mirrors, upstream: Option<(&Engagements, &[u32], bool, bool)>) {
        if self.hidden {
            self.flags = None;
            self.refuse("not drawn at this view".to_owned());
            return;
        }
        if let Some((engagements, changed, some, everything)) = upstream {
            let any = self.flags.is_none() || self.all || everything || some;
            let marking = self.columns.is_some() && self.refused.is_none();
            if everything && marking {
                self.mark_all(mirrors);
            }
            if marking {
                for &slot in changed {
                    self.mark(slot);
                }
            }
            if any {
                let (land, _, provenance) = engagements.flags();
                self.flags = Some((land.clone(), provenance[LAND]));
            }
        }
        let refused = if self.flags.is_none() {
            Err("the engagement layer is not drawn before it".to_owned())
        } else {
            self.core(mirrors)
        };
        let vision = match refused {
            Ok(vision) => vision,
            Err(why) => {
                self.refuse(why);
                return;
            }
        };
        let (flags, provenance) = self.flags.take().expect("checked above");
        if self.refused.take().is_some() || self.all {
            self.all = false;
            self.vision_new.store(false, Ordering::Release);
            self.recompose(mirrors);
        }
        if self.camera_dirty {
            self.readmit();
        }
        if self.vision_new.swap(false, Ordering::AcqRel) {
            self.mark_all(mirrors);
        }
        let limit = match provenance {
            Provenance::Indexed => None,
            other => other.limitation(),
        };
        if limit != self.limit {
            self.version += 1;
            self.limit = limit;
        }
        self.resolve(mirrors);
        let mut dirty = std::mem::take(&mut self.member_slots);
        let _ = dirty.settle(|slot| {
            self.member_row(mirrors, slot);
            Ok(())
        });
        self.member_slots = dirty;
        let stepped = std::mem::take(&mut self.stepped);
        if !stepped && self.combats.len() == 0 && self.kept_owed.len() == 0 {
            self.flags = Some((flags, provenance));
            return;
        }
        let kept = self.take_kept(mirrors, &vision);
        let mut dirty = std::mem::take(&mut self.combats);
        let tables = Tables::of(mirrors);
        let _ = dirty.settle(|slot| {
            if self.inside(tables.combats, slot) {
                self.settle_combat(&tables, (&vision, &flags, &kept), slot);
            } else if set(&mut self.out, slot) {
                self.outside += 1;
            }
            Ok(())
        });
        self.combats = dirty;
        self.kept = kept;
        self.flags = Some((flags, provenance));
    }

    fn apply_table(&mut self, table: TableId, changes: &Changes, columns: Columns) {
        let frame = self.frame;
        let slots = || changes.deleted().slots().iter().chain(changes.inserted().slots()).chain(changes.columns().iter().flat_map(|written| written.slots())).copied();
        match table {
            TableId::LandCombats => {
                for &key in changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()) {
                    self.kept_owed.mark(key as u64, frame);
                }
                slots().for_each(|slot| self.mark(slot));
            }
            TableId::LandCombatSides => self.side_rows[0].take(changes, frame, |column| columns.reads(table, column)),
            TableId::LandCombatDetails => self.side_rows[1].take(changes, frame, |column| columns.reads(table, column)),
            TableId::LandCombatDivisions => slots().for_each(|slot| {
                self.member_slots.mark(slot, frame);
            }),
            table if BY_DIVISION.contains(&table) => {
                let which = BY_DIVISION.iter().position(|&held| held == table).expect("a division-keyed table");
                self.division_rows[which].take(changes, frame, |column| columns.reads(table, column));
            }
            TableId::CountryLinks if !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| written.column() == columns.link_slot.column && !written.is_empty()) => self.all = true,
            _ => {}
        }
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
        if self.limit != other.limit {
            note(format!("limit: {:?} against {:?}", self.limit, other.limit));
        }
        let pick = |layer: &Self| -> BTreeMap<u64, MarkerOutcome> { layer.outcomes().filter(|(_, province, _)| layer.admitted(*province)).map(|(id, _, outcome)| (id, outcome.clone())).collect() };
        let (mine, theirs) = (pick(self), pick(other));
        for id in mine.keys().chain(theirs.keys()).collect::<BTreeSet<_>>() {
            if mine.get(id) != theirs.get(id) {
                note(format!("combat {id}: {:?} against {:?}", mine.get(id), theirs.get(id)));
            }
        }
        (differing, first)
    }
}

impl Layer for LandMarkers {
    fn held_viewer(&self) -> Option<u32> {
        self.perspective().map(|(_, vision)| vision.ours())
    }

    fn name(&self) -> &'static str {
        "land_markers"
    }

    fn placed(&mut self, view: &viewer_layer_core::layers::View) -> Option<u64> {
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        Some(u64::from(view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.field.cutoff())))
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let camera = view.camera.map(|camera| (camera, view.viewport));
        if camera != self.camera {
            self.camera = camera;
            self.camera_dirty = true;
        }
        let shown = viewer_map_modes::map_mode::visibility::of(&view.map_mode);
        let high = view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.field.cutoff());
        if !shown.province_icons || !view.overlay.land_combats || high {
            self.hidden = true;
            return Ok(());
        }
        inputs.after("engagements");
        for table in TABLES {
            inputs.table(table, true)?;
        }
        use TableId as T;
        let mut panel = [None; 22];
        for at in PANEL {
            panel[at] = Some(inputs.number(T::DivisionPanelStats, &format!("panel_stats[{at}]"))?);
        }
        self.columns = Some(Columns {
            combat_province: inputs.number(T::LandCombats, "province")?,
            attackers: inputs.list(T::LandCombats, "attackers")?,
            defenders: inputs.list(T::LandCombats, "defenders")?,
            indicator: inputs.number(T::LandCombatSides, "indicator")?,
            side_flags: inputs.list(T::LandCombatSides, "flags")?,
            attacker_primary: inputs.list(T::LandCombatSides, "attacker_primary")?,
            defender_primary: inputs.list(T::LandCombatSides, "defender_primary")?,
            defender_secondary: inputs.list(T::LandCombatSides, "defender_secondary")?,
            attacker_bonus: inputs.number(T::LandCombatDetails, "attacker_bonus")?,
            defender_bonus: inputs.number(T::LandCombatDetails, "defender_bonus")?,
            detail_sides: inputs.list(T::LandCombatDetails, "sides")?,
            member_division: inputs.number(T::LandCombatDivisions, "division")?,
            scales: [inputs.number(T::LandCombatDivisions, "scales[0]")?, inputs.number(T::LandCombatDivisions, "scales[1]")?, inputs.number(T::LandCombatDivisions, "scales[2]")?, inputs.number(T::LandCombatDivisions, "scales[3]")?],
            own: inputs.list(T::LandCombatDivisions, "own")?,
            province: inputs.number(T::Divisions, "province")?,
            hit_points: inputs.number(T::Divisions, "hit_points")?,
            organisation: inputs.number(T::Divisions, "organisation")?,
            panel,
            status_maxima: [inputs.number(T::DivisionPanelStats, "status_maxima[0]")?, inputs.number(T::DivisionPanelStats, "status_maxima[1]")?],
            naming: inputs.list(T::DivisionStatus, "naming")?,
            movement_stats: inputs.list(T::DivisionStatus, "movement_stats")?,
            strength_factor: inputs.number(T::DivisionStatus, "strength_factor")?,
            map_entity: inputs.number(T::DivisionMovement, "map_entity")?,
            fuel: inputs.number(T::DivisionMovement, "fuel")?,
            target: inputs.number(T::DivisionMovement, "target")?,
            naval_transfer: inputs.number(T::DivisionMovement, "naval_transfer")?,
            combat_count: inputs.number(T::DivisionMovement, "combat_count")?,
            path_total: inputs.number(T::DivisionMovement, "path_total")?,
            path: inputs.list(T::DivisionMovement, "path")?,
            alternate_total: inputs.number(T::DivisionMovement, "alternate_total")?,
            alternate_path: inputs.list(T::DivisionMovement, "alternate_path")?,
            combat_role: inputs.number(T::DivisionMovement, "combat_role")?,
            route_combat_province: inputs.number(T::DivisionMovement, "combat_province")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
        });
        if std::mem::take(&mut self.hidden) {
            self.all = true;
        }
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_with(mirrors, &Upstream::new(&[]))
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        let marked = self.marked;
        self.all = true;
        let engagements = upstream.get::<Engagements>();
        self.settle_dirty(mirrors, engagements.map(|layer| (layer, &[][..], false, false)));
        self.publish();
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() || self.hidden || self.refused.is_some() {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        let reads = viewer_layer_core::playhead_host::mirror::probe::reads();
        self.stepped = true;
        let columns = self.columns();
        for changes in step.tables() {
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.all = true;
                continue;
            }
            self.apply_table(TableId::ALL[changes.table()], changes, columns);
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads += viewer_layer_core::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn viewed(&mut self, _view: &View, _mirrors: &Mirrors) {
        if self.columns.is_some() && self.camera_dirty && self.refused.is_none() && !self.hidden {
            self.readmit();
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.settle_with(mirrors, &Upstream::new(&[]), settling)
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _settling: Settling) -> io::Result<()> {
        let combats = mirrors.get(TableId::LandCombats);
        let (whole, named): (bool, &[u64]) = match upstream.changed::<Engagements>() {
            Change::Whole => (true, &[]),
            Change::Marked(changed) => (false, changed.of("land")),
        };
        let entities: Vec<u32> = named.iter().filter_map(|&combat| combats?.slot(u128::from(combat))).collect();
        let engagements = upstream.get::<Engagements>().map(|layer| (layer, &entities[..], !named.is_empty(), whole));
        self.settle_dirty(mirrors, engagements);
        self.publish();
        Ok(())
    }


    fn carried(&self) -> Carried {
        let entities = self.combats.len() + self.member_slots.len() + self.rows_carried() + self.kept_owed.len() + usize::from(self.all);
        Carried { entities }
    }

    fn held(&self) -> usize {
        self.outcomes.capacity() * std::mem::size_of::<Option<Made>>()
            + self.outcomes.iter().flatten().map(|made| made.divisions.capacity() * 8 + made.outcome.gaps.iter().chain(&made.outcome.limits).map(|text| text.capacity() + 24).sum::<usize>()).sum::<usize>()
            + self.by_division.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + self.members.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + self.member_rows.capacity() * 8
            + self.member_slots.bytes()
            + self.combats.bytes()
            + self.side_rows.iter().chain(&self.division_rows).map(Rows::bytes).sum::<usize>()
            + (self.out.capacity() + self.asked.capacity() + self.admitted.capacity()) * 8
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
