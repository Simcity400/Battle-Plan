
use viewer_layer_core::{layers::{Carried, Settling, Dirty, Inputs, Layer, List, Num, Text, View}, owned::opaque};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror, Mirrors};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::{
    Hour,
    rows::{Field, NativeValue, Opaque, RailwayGunEtaOperands, RailwaySelectionInputs, Recorded, TableId},
};
use content_reader_map::map::MapData;
use std::{
    cell::Cell as Flag,
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_counter_kit::type_tint;
use viewer_counters::counters::CounterContent;
use viewer_counters_land::gun_layers::{GunPlaced, GunRoute, LayeredGuns};
use viewer_counters_land::gun_members::{Gun, GunFields, GunGround, GunJoins, gun_member, gun_name};
use viewer_gpu::camera::Pose;
use viewer_foundation::{
    mode_visibility::UnitKinds,
    perspective::{Choice, Vision},
    movement_timing::MovementContent,
};
use viewer_unit_reads::railway_gun_counter::{GunOperands, RailStatus};
use dashboard_text::table_access::{TokenError, TokenKeys};

pub type Hostility = BTreeMap<u64, Result<bool, &'static str>>;

pub struct Content {
    counters: Option<Arc<CounterContent>>,
    land: Box<dyn Fn(u32) -> bool + Send + Sync>,
    movement: Option<Arc<MovementContent>>,
    subunits: Result<TokenKeys, String>,
}

impl Content {
    pub fn new(counters: Arc<CounterContent>, map: Arc<MapData>, movement: Option<Arc<MovementContent>>) -> Self {
        let subunits = counters.subunit_tokens.clone();
        Self { counters: Some(counters), land: Box::new(move |province| GunGround::land_of(&map)(province)), movement, subunits }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn bare(subunits: TokenKeys, land: impl Fn(u32) -> bool + Send + Sync + 'static) -> Self {
        Self { counters: None, land: Box::new(land), movement: None, subunits: Ok(subunits) }
    }

    fn icons_cutoff(&self) -> f32 {
        self.counters.as_ref().map_or(f32::INFINITY, |counters| counters.defines.icons_cutoff)
    }
}

#[derive(Clone, Copy)]
struct Columns {
    roster_owner: Num<u32>,
    expeditionary_owner: Num<u32>,
    logical_owner: Num<u32>,
    province: Num<u32>,
    naval_transfer: Num<u8>,
    transfer_action_status: Num<u8>,
    exiled: Num<u8>,
    retreat: Num<u8>,
    strategic_redeployment: Num<u8>,
    reference_entry_count: Num<i32>,
    assignee_group: Num<u64>,
    subunit_token: Num<u32>,
    current_strength: Num<i64>,
    maximum_strength: Num<i64>,
    received_supply_ratio: Num<i64>,
    railway_building_level: List,
    target: Num<u32>,
    combat_role: Num<u8>,
    combat_province: Num<u32>,
    combat_count: Num<u32>,
    attacking: Num<u8>,
    ordinary_path: List,
    alternate_path: List,
    eta: List,
    link_slot: Num<u32>,
    mixed: List,
    cooldown_ticks: Num<i32>,
    selection: List,
    selection_hour: Num<i32>,
    literal: Text,
}

const TABLES: [TableId; 6] = [TableId::RailwayGunCounterInputs, TableId::CountryLinks, TableId::Countries, TableId::ProvinceUnitOrder, TableId::RailCooldowns, TableId::LexicalTokens];

fn held(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

fn u32s(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes.chunks_exact(4).map(|word| u32::from_le_bytes(word.try_into().expect("four bytes")))
}

fn u64s(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
    bytes.chunks_exact(8).map(|word| u64::from_le_bytes(word.try_into().expect("eight bytes")))
}

struct Tables<'a> {
    guns: Option<&'a Mirror>,
    links: Option<&'a Mirror>,
    order: Option<&'a Mirror>,
    rail: Option<(&'a Mirror, Hour)>,
    tokens: Option<&'a Mirror>,
    tokens_complete: bool,
}

impl<'a> Tables<'a> {
    fn of(mirrors: &'a Mirrors) -> Self {
        Self {
            guns: held(mirrors.get(TableId::RailwayGunCounterInputs)),
            links: held(mirrors.get(TableId::CountryLinks)),
            order: mirrors.get(TableId::ProvinceUnitOrder).filter(|mirror| mirror.complete()),
            rail: mirrors.get(TableId::RailCooldowns).filter(|mirror| mirror.complete()).and_then(|mirror| Some((mirror, mirror.head()?))),
            tokens: held(mirrors.get(TableId::LexicalTokens)),
            tokens_complete: mirrors.get(TableId::LexicalTokens).is_some_and(Mirror::complete),
        }
    }
}

struct Joins<'a> {
    tables: &'a Tables<'a>,
    columns: Columns,
    vision: &'a Vision,
    relations: &'a type_tint::Index,
    hostility: &'a Hostility,
    names: &'a HashMap<u32, Result<String, String>>,
    on_rail: Flag<bool>,
}

impl GunJoins for Joins<'_> {
    fn owner(&self, raw: u32) -> Option<u32> {
        let links = self.tables.links?;
        links.number(self.columns.link_slot, links.slot(u128::from(raw))?).value()
    }

    fn mixed(&self, gun: u64, province: u32) -> Option<Option<u32>> {
        let order = self.tables.order?;
        let Some(slot) = order.slot(u128::from(province)) else { return Some(None) };
        Some(order.list(self.columns.mixed, slot).value().and_then(|bytes| u64s(bytes).position(|unit| unit == gun)).map(|at| at as u32))
    }

    fn vision(&self) -> &Vision {
        self.vision
    }

    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side> {
        self.relations.of(roster, logical)
    }

    fn name(&self, token: u32) -> Result<String, String> {
        self.names.get(&token).cloned().unwrap_or_else(|| Err(format!("token {token} was not named")))
    }

    fn rail(&self, province: u32) -> Option<RailStatus> {
        let (rail, hour) = self.tables.rail?;
        let slot = rail.slot(u128::from(province))?;
        self.on_rail.set(true);
        rail_status(rail, self.columns, slot, hour).ok()
    }

    fn destination_enemy(&self, gun: &GunFields<'_>) -> Result<bool, &'static str> {
        self.hostility.get(&gun.id).copied().unwrap_or(Err("last ordinary-route province hostility"))
    }
}

fn rail_status(rail: &Mirror, columns: Columns, slot: u32, hour: Hour) -> Result<RailStatus, String> {
    let selection_hour = match rail.number(columns.selection_hour, slot) {
        Cell::Value(hour) => Recorded::Value(Some(hour)),
        Cell::Missing => Recorded::Value(None),
        Cell::Unrecorded => Recorded::NotRecorded,
    };
    let selection = opaque::<Option<RailwaySelectionInputs>>(rail, columns.selection, slot)?;
    let cooldown = rail.number(columns.cooldown_ticks, slot).value().unwrap_or(0);
    RailStatus::of(&selection_hour, &selection, cooldown, hour)
}

fn spelling<'a>(tables: &Tables<'a>, literal: Text, token: u32) -> Result<&'a str, TokenError> {
    if token == 0 {
        return Err(TokenError::Rejected);
    }
    let absent = if tables.tokens_complete { TokenError::Absent(token) } else { TokenError::Missing(token) };
    let tokens = tables.tokens.ok_or(absent)?;
    let slot = tokens.slot(u128::from(token)).ok_or(absent)?;
    let bytes = tokens.text(literal, slot).value().and_then(|id| tokens.string(id)).ok_or(TokenError::NotText(token))?;
    std::str::from_utf8(bytes).ok().filter(|name| !name.contains('\0')).ok_or(TokenError::NotText(token))
}

pub const PHASES: [&str; 3] = ["guns", "on rail", "names"];

pub struct GunCounters {
    content: Arc<Content>,
    columns: Option<Columns>,
    refused: Option<String>,
    guns: Vec<Option<(u64, Gun)>>,
    routes: Vec<Option<GunRoute>>,
    rows: usize,
    version: u64,
    published: Option<(u64, Arc<LayeredGuns>)>,
    at: Vec<(u32, u32)>,
    by_province: HashMap<u32, Vec<u32>>,
    on_rail: Vec<u64>,
    hour: Option<Hour>,
    names: HashMap<u32, Result<String, String>>,
    census: Option<usize>,
    dirty: Dirty,
    all: bool,
    hidden: bool,
    vision: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision_new: AtomicBool,
    relations: Mutex<Option<Arc<type_tint::Index>>>,
    relations_new: AtomicBool,
    hostility: Mutex<Option<Arc<Hostility>>>,
    hostility_new: AtomicBool,
    statuses: HashMap<usize, Status>,
    frame: u64,
    marked: usize,
    phases: [(u64, u64); PHASES.len()],
}

impl GunCounters {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            content,
            columns: None,
            refused: Some("not composed yet".into()),
            guns: Vec::new(),
            routes: Vec::new(),
            rows: 0,
            version: 1,
            published: None,
            at: Vec::new(),
            by_province: HashMap::new(),
            on_rail: Vec::new(),
            hour: None,
            names: HashMap::new(),
            census: None,
            dirty: Dirty::default(),
            all: true,
            hidden: false,
            vision: Mutex::new(None),
            vision_new: AtomicBool::new(false),
            relations: Mutex::new(None),
            relations_new: AtomicBool::new(false),
            hostility: Mutex::new(None),
            hostility_new: AtomicBool::new(false),
            statuses: HashMap::new(),
            frame: 0,
            marked: 0,
            phases: [(0, 0); PHASES.len()],
        }
    }

    pub fn fresh(&self) -> Self {
        let fresh = Self { columns: self.columns, hidden: self.hidden, ..Self::new(Arc::clone(&self.content)) };
        if let Some((choice, vision)) = self.perspective() {
            fresh.set_vision(choice, vision);
        }
        if let Some(relations) = lock(&self.relations).clone() {
            fresh.set_relations(relations);
        }
        if let Some(hostility) = lock(&self.hostility).clone() {
            fresh.set_hostility(hostility);
        }
        fresh
    }

    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        let mut held = lock(&self.vision);
        if held.as_ref().is_none_or(|(_, held)| !Arc::ptr_eq(held, &vision) && **held != *vision) {
            self.vision_new.store(true, Ordering::Release);
        }
        *held = Some((choice, vision));
    }

    pub fn set_relations(&self, relations: Arc<type_tint::Index>) {
        let mut held = lock(&self.relations);
        if held.as_ref().is_none_or(|held| !Arc::ptr_eq(held, &relations)) {
            self.relations_new.store(true, Ordering::Release);
        }
        *held = Some(relations);
    }

    pub fn set_hostility(&self, hostility: Arc<Hostility>) {
        let mut held = lock(&self.hostility);
        if held.as_ref().is_none_or(|held| !Arc::ptr_eq(held, &hostility) && **held != *hostility) {
            self.hostility_new.store(true, Ordering::Release);
        }
        *held = Some(hostility);
    }

    pub fn perspective(&self) -> Option<(Choice, Arc<Vision>)> {
        lock(&self.vision).clone()
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the gun counters layer is declared before it composes")
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    pub fn census(&self) -> Option<usize> {
        self.census
    }

    pub fn guns(&self) -> impl Iterator<Item = &(u64, Gun)> + '_ {
        self.guns.iter().flatten()
    }

    pub fn published(&self) -> Option<&Arc<LayeredGuns>> {
        self.published.as_ref().filter(|_| self.refused.is_none() && !self.hidden).map(|(_, guns)| guns)
    }

    fn publish(&mut self) {
        if self.refused.is_none() && !self.hidden && self.published.as_ref().is_none_or(|(version, _)| *version != self.version) {
            self.published = Some((self.version, Arc::new(self.product())));
        }
    }

    fn product(&self) -> LayeredGuns {
        let mut members: Vec<GunPlaced> = self
            .guns
            .iter()
            .zip(&self.routes)
            .filter_map(|(gun, route)| match (gun, route) {
                (Some((_, Gun::Member(member))), Some(route)) => Some(GunPlaced { member: member.clone(), route: route.clone() }),
                _ => None,
            })
            .collect();
        members.sort_unstable_by_key(|placed| placed.member.id);
        let refused = self
            .guns
            .iter()
            .zip(&self.at)
            .filter(|(gun, _)| gun.is_some())
            .filter_map(|(_, &(_, token))| self.names.get(&token)?.as_ref().err().map(|reason| format!("railway gun sub-unit: {reason}")))
            .collect();
        LayeredGuns {
            complete: self.census.is_none(),
            recorded: self.census.unwrap_or(self.rows),
            members,
            unnamed: self.guns.iter().filter(|gun| matches!(gun, Some((_, Gun::Unnamed(_))))).count(),
            refused,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn on_rail(&self) -> usize {
        self.on_rail.iter().map(|word| word.count_ones() as usize).sum()
    }

    pub fn marked(&self) -> usize {
        self.marked
    }

    pub fn phases(&self) -> impl Iterator<Item = (&'static str, u64, u64)> + '_ {
        PHASES.iter().zip(&self.phases).map(|(&name, &(entities, ns))| (name, entities, ns))
    }

    fn core(&self, mirrors: &Mirrors) -> Result<(Arc<Vision>, Arc<type_tint::Index>, Arc<Hostility>), String> {
        let (_, vision) = self.perspective().ok_or("the viewing country's vision is not set")?;
        let relations = lock(&self.relations).clone().ok_or("the viewing country's type-icon relations are not set")?;
        let hostility = lock(&self.hostility).clone().ok_or("the guns' destination hostility is not set")?;
        for table in [TableId::CountryLinks, TableId::Countries] {
            let mirror = mirrors.get(table).ok_or_else(|| format!("{table:?} is not read yet"))?;
            if let Some(why) = mirror.not_recorded() {
                return Err(format!("{why}; the counters need every country link and country"));
            }
        }
        let guns = mirrors.get(TableId::RailwayGunCounterInputs).ok_or("RailwayGunCounterInputs is not read yet")?;
        if guns.status().is_none() {
            return Err("RailwayGunCounterInputs is not recorded here: a gap in the recording".into());
        }
        Ok((vision, relations, hostility))
    }

    fn mark(&mut self, slot: u32) {
        if self.dirty.mark(slot, self.frame) {
            self.marked += 1;
        }
    }

    fn mark_province(&mut self, province: u32) {
        let slots = self.by_province.get(&province).cloned().unwrap_or_default();
        slots.into_iter().for_each(|slot| self.mark(slot));
    }

    fn recompose(&mut self, mirrors: &Mirrors) {
        self.drop_all();
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        if let Some(guns) = mirrors.get(TableId::RailwayGunCounterInputs) {
            let slots: Vec<u32> = guns.rows().collect();
            slots.into_iter().for_each(|slot| self.mark(slot));
        }
    }

    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.content));
        let kept = std::mem::replace(self, empty);
        (self.columns, self.hidden, self.frame, self.marked, self.phases) = (kept.columns, kept.hidden, kept.frame, kept.marked, kept.phases);
        (self.vision, self.vision_new, self.relations, self.relations_new, self.hostility, self.hostility_new) = (kept.vision, kept.vision_new, kept.relations, kept.relations_new, kept.hostility, kept.hostility_new);
    }

    fn drop_all(&mut self) {
        self.version += 1;
        self.guns.clear();
        self.routes.clear();
        self.at.clear();
        self.by_province.clear();
        self.on_rail.clear();
        self.names.clear();
        self.dirty.clear();
    }

    fn census_of(mirrors: &Mirrors) -> Option<usize> {
        let guns = mirrors.get(TableId::RailwayGunCounterInputs);
        if guns.is_some_and(Mirror::complete) {
            return None;
        }
        Some(held(guns).map_or(0, |guns| guns.rows().count()))
    }

    fn fields<'p>(guns: &Mirror, slot: u32, columns: Columns, paths: &'p mut [Vec<u32>; 2]) -> Option<GunFields<'p>> {
        let key = guns.key(slot)?;
        let number = |column: Num<u32>| guns.number(column, slot).value();
        let byte = |column: Num<u8>| guns.number(column, slot).value();
        let wide = |column: Num<i64>| guns.number(column, slot).value();
        let railway_building_level = <Option<i16> as NativeValue>::from_field(Field::Opaque(Opaque::Encoded(guns.list(columns.railway_building_level, slot).value()?, guns.format()))).ok()?;
        let eta = opaque::<Option<RailwayGunEtaOperands>>(guns, columns.eta, slot).ok().and_then(|eta| match eta {
            Recorded::Value(eta) => eta,
            Recorded::NotRecorded => None,
        });
        let [ordinary, alternate] = paths;
        ordinary.clear();
        ordinary.extend(u32s(guns.list(columns.ordinary_path, slot).value()?));
        alternate.clear();
        alternate.extend(u32s(guns.list(columns.alternate_path, slot).value()?));
        let (ordinary, alternate): (&'p [u32], &'p [u32]) = (ordinary, alternate);
        Some(GunFields {
            id: key as u64,
            roster_owner: number(columns.roster_owner)?,
            expeditionary_owner: number(columns.expeditionary_owner)?,
            logical_owner: number(columns.logical_owner)?,
            province: number(columns.province)?,
            assignee_group: guns.number(columns.assignee_group, slot).value()?,
            subunit_token: number(columns.subunit_token)?,
            current_strength: wide(columns.current_strength)?,
            maximum_strength: wide(columns.maximum_strength)?,
            received_supply_ratio: wide(columns.received_supply_ratio)?,
            combat_role: byte(columns.combat_role)?,
            combat_province: number(columns.combat_province)?,
            operands: GunOperands {
                path: ordinary.len() as u32,
                combat_count: number(columns.combat_count)?,
                reference_entry_count: guns.number(columns.reference_entry_count, slot).value()?,
                attacking: byte(columns.attacking)?,
                target: number(columns.target)?,
                retreat: byte(columns.retreat)?,
                strategic_redeployment: byte(columns.strategic_redeployment)?,
                naval_transfer: byte(columns.naval_transfer)?,
                transfer_action_status: byte(columns.transfer_action_status)?,
                exiled: byte(columns.exiled)?,
                railway_building_level,
            },
            ordinary_path: ordinary,
            alternate_path: alternate,
            eta,
        })
    }

    fn settle_gun(&mut self, tables: &Tables<'_>, seen: (&Vision, &type_tint::Index, &Hostility), slot: u32, paths: &mut [Vec<u32>; 2]) {
        self.version += 1;
        let at = slot as usize;
        if self.guns.len() <= at {
            self.guns.resize(at + 1, None);
            self.routes.resize(at + 1, None);
            self.at.resize(at + 1, (0, 0));
        }
        let (province, _) = self.at[at];
        if province != 0
            && let Some(slots) = self.by_province.get_mut(&province)
        {
            slots.retain(|&held| held != slot);
            if slots.is_empty() {
                self.by_province.remove(&province);
            }
        }
        self.at[at] = (0, 0);
        self.guns[at] = None;
        self.routes[at] = None;
        if let Some(word) = self.on_rail.get_mut(at / 64) {
            *word &= !(1 << (at % 64));
        }
        let columns = self.columns();
        let Some(guns) = tables.guns.filter(|guns| guns.live(slot)) else { return };
        let Some(fields) = Self::fields(guns, slot, columns, paths) else { return };
        self.at[at] = (fields.province, fields.subunit_token);
        if fields.province != 0 {
            self.by_province.entry(fields.province).or_default().push(slot);
        }
        if !self.names.contains_key(&fields.subunit_token) {
            let subunits = self.content.subunits.as_ref().map_err(String::as_str);
            let named = gun_name(spelling(tables, columns.literal, fields.subunit_token).map_err(|reason| reason.to_string()), subunits);
            self.names.insert(fields.subunit_token, named);
            self.phases[2].0 += 1;
        }
        let content = &self.content;
        let anchor = |id: u32, kind: u32| content.counters.as_ref().and_then(|counters| counters.anchor(id, kind));
        let ground = GunGround { anchor: &anchor, land: &*content.land, movement: content.movement.as_deref() };
        let joins = Joins { tables, columns, vision: seen.0, relations: seen.1, hostility: seen.2, names: &self.names, on_rail: Flag::new(false) };
        let gun = gun_member(&fields, ground, &joins);
        if joins.on_rail.get() {
            let word = at / 64;
            if self.on_rail.len() <= word {
                self.on_rail.resize(word + 1, 0);
            }
            self.on_rail[word] |= 1 << (at % 64);
        }
        if matches!(gun, Gun::Member(_)) {
            self.routes[at] = Some(GunRoute::of(&fields));
        }
        self.guns[at] = Some((fields.id, gun));
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        let refused = if self.hidden { Err("not drawn at this view".to_owned()) } else { self.core(mirrors) };
        let (vision, relations, hostility) = match refused {
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
        let renew = self.vision_new.swap(false, Ordering::AcqRel) | self.relations_new.swap(false, Ordering::AcqRel) | self.hostility_new.swap(false, Ordering::AcqRel);
        if self.refused.take().is_some() || self.all || renew {
            self.all = false;
            self.recompose(mirrors);
        }
        let census = Self::census_of(mirrors);
        if census.is_some() {
            if self.census.is_none() {
                self.drop_all();
            }
            if self.census != census {
                self.version += 1;
            }
            self.census = census;
            self.dirty.clear();
            return;
        }
        if self.census.take().is_some() {
            self.recompose(mirrors);
        }
        let tables = Tables::of(mirrors);
        let rows = tables.guns.map_or(0, |guns| guns.rows().count());
        if rows != self.rows {
            self.version += 1;
            self.rows = rows;
        }
        let hour = tables.rail.map(|(_, hour)| hour);
        if hour != self.hour {
            self.hour = hour;
            let began = std::time::Instant::now();
            let on_rail: Vec<u32> = self.on_rail.iter().enumerate().flat_map(|(word, &bits)| (0..64).filter(move |at| bits & (1 << at) != 0).map(move |at| (word * 64 + at) as u32)).collect();
            self.phases[1].0 += on_rail.len() as u64;
            on_rail.into_iter().for_each(|slot| self.mark(slot));
            self.phases[1].1 += began.elapsed().as_nanos() as u64;
        }
        let began = std::time::Instant::now();
        let mut dirty = std::mem::take(&mut self.dirty);
        let taken = dirty.len() as u64;
        let mut paths = [Vec::new(), Vec::new()];
        let _ = dirty.settle(|slot| {
            self.settle_gun(&tables, (&vision, &relations, &hostility), slot, &mut paths);
            Ok(())
        });
        let taken = taken - dirty.len() as u64;
        self.dirty = dirty;
        self.phases[0].0 += taken;
        self.phases[0].1 += began.elapsed().as_nanos() as u64;
    }

    fn apply_table(&mut self, table: TableId, changes: &Changes, mirrors: &Mirrors, columns: Columns) {
        let Some(mirror) = mirrors.get(table) else { return };
        let lives = |changes: &Changes| changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u32).collect::<Vec<u32>>();
        match table {
            TableId::RailwayGunCounterInputs => {
                for &slot in changes.deleted().slots().iter().chain(changes.inserted().slots()).chain(changes.columns().iter().flat_map(|written| written.slots())) {
                    self.mark(slot);
                }
            }
            TableId::ProvinceUnitOrder => {
                lives(changes).into_iter().for_each(|province| self.mark_province(province));
                let mut marked = Vec::new();
                for &slot in changes.columns().iter().flat_map(|written| written.slots()) {
                    if let Some(slots) = mirror.key(slot).and_then(|key| self.by_province.get(&(key as u32))) {
                        marked.extend_from_slice(slots);
                    }
                }
                marked.into_iter().for_each(|slot| self.mark(slot));
            }
            TableId::RailCooldowns => lives(changes).into_iter().for_each(|province| self.mark_province(province)),
            TableId::LexicalTokens => {
                let mut tokens = lives(changes);
                tokens.extend(changes.columns().iter().flat_map(|written| written.slots()).filter_map(|&slot| mirror.key(slot)).map(|key| key as u32));
                for token in tokens {
                    if self.names.remove(&token).is_some() {
                        let slots: Vec<u32> = self.at.iter().enumerate().filter(|(_, at)| at.1 == token).map(|(slot, _)| slot as u32).collect();
                        slots.into_iter().for_each(|slot| self.mark(slot));
                    }
                }
            }
            TableId::CountryLinks if !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| written.column() == columns.link_slot.column && !written.is_empty()) => {
                self.all = true;
            }
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
        if self.census != other.census {
            note(format!("census: {:?} against {:?}", self.census, other.census));
        }
        let mine: BTreeMap<u64, &Gun> = self.guns().map(|(id, gun)| (*id, gun)).collect();
        let theirs: BTreeMap<u64, &Gun> = other.guns().map(|(id, gun)| (*id, gun)).collect();
        for id in mine.keys().chain(theirs.keys()).collect::<std::collections::BTreeSet<_>>() {
            if mine.get(id) != theirs.get(id) {
                note(format!("gun {id}: {:?} against {:?}", mine.get(id), theirs.get(id)));
            }
        }
        (differing, first)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Layer for GunCounters {
    fn held_viewer(&self) -> Option<u32> {
        self.perspective().map(|(_, vision)| vision.ours())
    }

    fn name(&self) -> &'static str {
        "gun_counters"
    }

    fn placed(&mut self, view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(u64::from(view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.content.icons_cutoff())))
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let shown = viewer_map_modes::map_mode::visibility::of(&view.map_mode);
        let high = view.camera.is_some_and(|camera| matches!(camera.pose, Pose::Map) && camera.height >= self.content.icons_cutoff());
        if !shown.unit_counters || shown.unit_kinds != UnitKinds::All || !view.overlay.counters.railway_guns || high {
            self.hidden = true;
            return Ok(());
        }
        for table in TABLES {
            inputs.table(table, true)?;
        }
        use TableId as T;
        let gun = T::RailwayGunCounterInputs;
        self.columns = Some(Columns {
            roster_owner: inputs.number(gun, "roster_owner")?,
            expeditionary_owner: inputs.number(gun, "expeditionary_owner")?,
            logical_owner: inputs.number(gun, "logical_owner")?,
            province: inputs.number(gun, "province")?,
            naval_transfer: inputs.number(gun, "naval_transfer")?,
            transfer_action_status: inputs.number(gun, "transfer_action_status")?,
            exiled: inputs.number(gun, "exiled")?,
            retreat: inputs.number(gun, "retreat")?,
            strategic_redeployment: inputs.number(gun, "strategic_redeployment")?,
            reference_entry_count: inputs.number(gun, "reference_entry_count")?,
            assignee_group: inputs.number(gun, "assignee_group")?,
            subunit_token: inputs.number(gun, "subunit_token")?,
            current_strength: inputs.number(gun, "current_strength")?,
            maximum_strength: inputs.number(gun, "maximum_strength")?,
            received_supply_ratio: inputs.number(gun, "received_supply_ratio")?,
            railway_building_level: inputs.list(gun, "railway_building_level")?,
            target: inputs.number(gun, "target")?,
            combat_role: inputs.number(gun, "combat_role")?,
            combat_province: inputs.number(gun, "combat_province")?,
            combat_count: inputs.number(gun, "combat_count")?,
            attacking: inputs.number(gun, "attacking")?,
            ordinary_path: inputs.list(gun, "ordinary_path")?,
            alternate_path: inputs.list(gun, "alternate_path")?,
            eta: inputs.list(gun, "eta")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
            mixed: inputs.list(T::ProvinceUnitOrder, "mixed")?,
            cooldown_ticks: inputs.number(T::RailCooldowns, "cooldown_ticks")?,
            selection: inputs.list(T::RailCooldowns, "selection")?,
            selection_hour: inputs.number(T::RailCooldowns, "selection_hour")?,
            literal: inputs.text(T::LexicalTokens, "literal")?,
        });
        if std::mem::take(&mut self.hidden) {
            self.all = true;
        }
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let (marked, phases) = (self.marked, self.phases);
        self.all = true;
        self.settle_dirty(mirrors);
        self.publish();
        (self.marked, self.phases) = (marked, phases);
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

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        self.publish();
        Ok(())
    }


    fn carried(&self) -> Carried {
        Carried { entities: self.dirty.len() + usize::from(self.all) }
    }

    fn held(&self) -> usize {
        let gun = |gun: &Option<(u64, Gun)>| match gun {
            Some((_, Gun::Member(member))) => member.name.capacity(),
            Some((_, Gun::Unnamed(reason))) => reason.capacity(),
            _ => 0,
        };
        self.guns.capacity() * std::mem::size_of::<Option<(u64, Gun)>>()
            + self.guns.iter().map(gun).sum::<usize>()
            + self.routes.capacity() * std::mem::size_of::<Option<GunRoute>>()
            + self.routes.iter().flatten().map(|route| (route.ordinary_path.capacity() + route.alternate_path.capacity()) * 4).sum::<usize>()
            + self.at.capacity() * 8
            + self.by_province.values().map(|slots| slots.capacity() * 4 + 40).sum::<usize>()
            + self.on_rail.capacity() * 8
            + self.names.iter().map(|(_, name)| 48 + name.as_ref().map_or_else(String::capacity, String::capacity)).sum::<usize>()
            + self.dirty.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
