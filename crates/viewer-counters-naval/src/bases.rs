use crate::naval_windows;
use viewer_counter_kit::{BaseUnknowns, STRATEGIC_LOCATION_SPRITE_PREFIX};
use viewer_native_gui::native_windows::STRATEGIC_LOCATION_WINDOW;
use viewer_counter_kit::FACILITY_SPRITE_PREFIX;

use content_reader_core::positions;
use content_reader_core::defines::Defines;
use content_reader_core::map::{MapData, ProvinceKind};
use crate::{air_grid, air_volunteers, base_windows, naval_repairs};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::{CounterFrame, CounterInputs, CounterMap, CounterReport, invalid};
use viewer_counter_kit::shards::Shards;
use viewer_foundation::building_facts;
use viewer_foundation::perspective::Vision;
use viewer_foundation::tables::{Available, core, rows};
use viewer_foundation::unit_rules::STANDSTILL;
use viewer_native_gui::native_windows::{
    AIR_BASE_WINDOW, GUN_EMPLACEMENT_WINDOW, RADAR_WINDOW, ROCKET_SITE_WINDOW,
};
use viewer_runtime::label_reads::VICTORY_POINT;
use viewer_unit_reads::counter_order;
use content_reader_core::{buildings::BuildingType, buildings::Buildings, buildings::SpawnPoint};
use replay_data::{rows::BuildingRow, rows::ProvinceRow};
use viewer_gpu::province_map::ProvinceMap;
use dashboard_text::table_access::TokenNames;
use std::collections::HashMap;

use viewer_foundation::visibility::{AccessLinks, ProvinceAccess, Relations};
use dashboard_text::table_access::TokenKeys;
use viewer_foundation::building_facts::{self as map_buildings, SlotAdmission};

pub fn naval_base_frame(known: bool, foreign: bool, disclosed: bool, ships: bool) -> u32 {
    if !known {
        return u32::from(ships);
    }
    let arm = if disclosed { 4 } else { 2 };
    arm + u32::from(ships) + if foreign { 4 } else { 0 }
}

pub use replay_derive::naval_repairs::country_list_contains;

pub fn naval_base_ships(
    roster: &[u64],
    detail: impl FnOnce() -> Option<u8>,
    byte: impl Fn(u64) -> Option<u16>,
) -> Option<bool> {
    if roster.is_empty() {
        return Some(false);
    }
    let mut unknown = false;
    for &unit in roster {
        match byte(unit) {
            Some(byte) if byte > 0x95 => return Some(true),
            Some(_) => {}
            None => unknown = true,
        }
    }
    match detail() {
        Some(level) if level >= 2 => Some(true),
        Some(_) if !unknown => Some(false),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuildingIconContent {
    headquarters: Vec<IconTemplate>,
    facilities: BTreeMap<String, (String, IconTemplate)>,
    refused_headquarters: BTreeMap<String, String>,
    refused_facilities: BTreeMap<String, String>,
    keys: TokenKeys,
    records: BTreeMap<(u32, String), [f32; 3]>,
    tokens: TokenMemo,
    carried: TokenCarry,
}

type BuildingTokens = (BTreeMap<String, u32>, BTreeSet<String>);

const TOKEN_TABLES: &[replay_data::rows::TableId] = &[replay_data::rows::TableId::LexicalTokens];

#[derive(Clone)]
struct TokenMemo(Arc<viewer_runtime::memo::Keyed<BuildingTokens>>);

impl Default for TokenMemo {
    fn default() -> Self {
        Self(Arc::new(viewer_runtime::memo::Keyed::new(
            TOKEN_TABLES,
        )))
    }
}

#[derive(Clone, Default)]
struct TokenCarry(Arc<std::sync::Mutex<Option<CarriedTokens>>>);

struct CarriedTokens {
    branch: replay_data::BranchId,
    view: replay_frame::frame::SharedView,
    found: BTreeMap<String, Vec<u32>>,
    several: BTreeMap<u32, String>,
    value: Arc<BuildingTokens>,
}

impl std::fmt::Debug for TokenCarry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenCarry")
    }
}

fn add_tokens(names: TokenNames<'_>, keys: &TokenKeys, needed: &BTreeSet<&str>, found: &mut BTreeMap<String, Vec<u32>>, several: &mut BTreeMap<u32, String>) {
    for (token, spelling) in names.iter() {
        let group = keys.keys(spelling);
        let named: Vec<&str> = group.iter().map(String::as_str).filter(|key| needed.contains(key)).collect();
        if named.is_empty() {
            continue;
        }
        if group.len() > 1 {
            several.insert(token, format!("building token {token} ({spelling}) names several building types: {}", group.join(", ")));
        }
        for key in named {
            let tokens = found.entry(key.to_owned()).or_default();
            if let Err(at) = tokens.binary_search(&token) {
                tokens.insert(at, token);
            }
        }
    }
}

fn tokens_of(found: &BTreeMap<String, Vec<u32>>, several: &BTreeMap<u32, String>) -> BuildingTokens {
    let mut refused: BTreeSet<String> = several.values().cloned().collect();
    let mut out = BTreeMap::new();
    for (key, tokens) in found {
        match tokens.as_slice() {
            [token] => {
                out.insert(key.clone(), *token);
            }
            _ => {
                refused.insert(format!("building {key} folds to several tokens: {tokens:?}"));
            }
        }
    }
    (out, refused)
}

fn lexical_rows(view: &replay_frame::frame::SharedView) -> Option<&[replay_data::rows::LexicalTokenRow]> {
    match view.held() {
        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::LexicalTokens(rows))
        | replay_frame::frame::TableView::Truncated { rows: replay_data::rows::TableRows::LexicalTokens(rows), .. } => Some(rows),
        _ => None,
    }
}

impl TokenCarry {
    fn tokens(&self, frame: &DecodedFrame, keys: &TokenKeys, needed: &BTreeSet<&str>) -> io::Result<Arc<BuildingTokens>> {
        let names = TokenNames::new(frame)?;
        let view = frame.tables[replay_data::rows::TableId::LexicalTokens.index()].clone();
        let carried = {
            let held = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match held.as_ref().filter(|carried| carried.branch == frame.branch) {
                Some(carried) if replay_frame::frame::SharedView::ptr_eq(&carried.view, &view) => {
                    return Ok(Arc::clone(&carried.value));
                }
                Some(carried) => Some((carried.view.clone(), carried.found.clone(), carried.several.clone())),
                None => None,
            }
        };
        let store = |found, several, value: &Arc<BuildingTokens>, view| {
            *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some(CarriedTokens { branch: frame.branch, view, found, several, value: Arc::clone(value) });
        };
        if let Some((carried_view, mut found, mut several)) = carried {
            if let (Some(old), Some(new)) = (lexical_rows(&carried_view), lexical_rows(&view)) {
                let mut added = Vec::new();
                let (mut at, mut kept) = (0, true);
                for row in new {
                    while at < old.len() && old[at].token < row.token {
                        kept = false;
                        at += 1;
                    }
                    if !kept {
                        break;
                    }
                    match old.get(at) {
                        Some(previous) if previous.token == row.token => {
                            if previous.literal != row.literal {
                                kept = false;
                                break;
                            }
                            at += 1;
                        }
                        _ => added.push(row.clone()),
                    }
                }
                if kept && at == old.len() {
                    add_tokens(TokenNames::from_rows(&added, true), keys, needed, &mut found, &mut several);
                    let value = Arc::new(tokens_of(&found, &several));
                    store(found, several, &value, view);
                    return Ok(value);
                }
            }
        }
        let (mut found, mut several) = (BTreeMap::new(), BTreeMap::new());
        add_tokens(names, keys, needed, &mut found, &mut several);
        let value = Arc::new(tokens_of(&found, &several));
        store(found, several, &value, view);
        Ok(value)
    }
}

impl std::fmt::Debug for TokenMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenMemo")
    }
}

#[derive(Clone, Debug)]
struct IconTemplate {
    name: String,
    anchor: Option<(String, [f32; 3])>,
    ambiguous: bool,
}

type Refusal = String;

type FacilityChoice<'c, 'r> = (&'r BuildingRow, &'c (String, IconTemplate));

impl BuildingIconContent {
    pub fn load(
        buildings: &Buildings,
        slots: &[positions::BuildingPosition],
        provinces: &ProvinceMap<'_>,
        admit: impl Fn(&SpawnPoint, &positions::BuildingPosition, Option<u32>) -> SlotAdmission,
        [headquarter, facility]: [[f32; 3]; 2],
    ) -> Self {
        let keys = TokenKeys::new(buildings.types.keys());
        let ports = redirect_targets(buildings, |kind| kind.is_port);
        let specialized = redirect_targets(buildings, |kind| !kind.specialization.is_empty());
        let template = |kind: &BuildingType, list: &BTreeMap<&str, bool>, offset: [f32; 3]| {
            let anchor = kind
                .spawn_point
                .as_deref()
                .and_then(|bucket| Some((bucket, *list.get(bucket)?)));
            IconTemplate {
                name: kind.name.clone(),
                ambiguous: anchor.is_some_and(|(_, ambiguous)| ambiguous),
                anchor: anchor.map(|(bucket, _)| {
                    let offset = if kind.only_costal { offset } else { [0.0; 3] };
                    (bucket.to_owned(), offset)
                }),
            }
        };
        let refused = |test: fn(&BuildingType) -> bool| -> BTreeMap<String, String> {
            buildings
                .types
                .iter()
                .filter(|(name, kind)| {
                    test(kind)
                        || buildings
                            .repeated_types
                            .get(*name)
                            .is_some_and(|all| all.iter().any(test))
                })
                .filter_map(|(name, _)| {
                    Some((name.clone(), unattributable(buildings, &keys, name)?))
                })
                .collect()
        };
        let refused_headquarters = refused(|kind| kind.naval_headquarter);
        let refused_facilities = refused(|kind| !kind.specialization.is_empty());
        let mut headquarters: Vec<IconTemplate> = Vec::new();
        for name in &buildings.declaration_order {
            if let Some(kind) = buildings.types.get(name)
                && !refused_headquarters.contains_key(name)
                && kind.naval_headquarter
                && !headquarters.iter().any(|seen| seen.name == *name)
            {
                headquarters.push(template(kind, &ports, headquarter));
            }
        }
        let facilities: BTreeMap<String, (String, IconTemplate)> = buildings
            .types
            .values()
            .filter(|kind| !refused_facilities.contains_key(&kind.name))
            .filter_map(|kind| {
                let first = kind.specialization.first()?.clone();
                Some((
                    kind.name.clone(),
                    (first, template(kind, &specialized, facility)),
                ))
            })
            .collect();
        let buckets: BTreeSet<&str> = headquarters
            .iter()
            .chain(facilities.values().map(|(_, template)| template))
            .filter_map(|template| template.anchor.as_ref().map(|(bucket, _)| bucket.as_str()))
            .collect();
        let mut records = BTreeMap::new();
        for slot in slots {
            if map_buildings::is_floating_harbor_slot(&slot.kind) {
                continue;
            }
            let Some((bucket, spawn)) = buildings
                .slot(&slot.kind)
                .map(|slot| slot.spawn().name.as_str())
                .filter(|bucket| buckets.contains(bucket))
                .and_then(|bucket| Some((bucket, buildings.spawn_points.get(bucket)?)))
            else {
                continue;
            };
            let province = provinces.at(slot.x, slot.z);
            if admit(spawn, slot, province) == SlotAdmission::Admitted
                && let Some(province) = province
            {
                records
                    .entry((province, bucket.to_owned()))
                    .or_insert([slot.x, slot.y, slot.z]);
            }
        }
        Self {
            headquarters,
            facilities,
            refused_headquarters,
            refused_facilities,
            keys,
            records,
            tokens: TokenMemo::default(),
            carried: TokenCarry::default(),
        }
    }

    pub fn offsets(defines: &Defines) -> io::Result<[[f32; 3]; 2]> {
        let offset = |name: &str| match defines.numbers("NMapMode", name)?[..] {
            [x, y, z] => Ok([x as f32, y as f32, z as f32]),
            _ => Err(invalid(format!(
                "NDefines.NMapMode.{name} must hold three values"
            ))),
        };
        Ok([
            offset("NAVAL_HEADQUARTER_OFFSET")?,
            offset("COASTAL_FACILITY_OFFSET")?,
        ])
    }

    fn names(&self) -> BTreeSet<&str> {
        self.headquarters
            .iter()
            .map(|template| template.name.as_str())
            .chain(self.refused_headquarters.keys().map(String::as_str))
            .chain(self.facility_names())
            .collect()
    }

    fn carried_tokens(&self, frame: &DecodedFrame) -> io::Result<Arc<BuildingTokens>> {
        self.carried.tokens(frame, &self.keys, &self.names())
    }

    fn tokens(&self, frame: &DecodedFrame) -> io::Result<Arc<BuildingTokens>> {
        self.tokens.0.get_or_build_without_waiting(frame, (), |projected| {
            building_tokens(projected, &self.keys, &self.names()).map(Arc::new)
        })
    }

    fn facility_names(&self) -> impl Iterator<Item = &str> {
        self.facilities
            .keys()
            .chain(self.refused_facilities.keys())
            .map(String::as_str)
    }

    #[cfg(test)]
    fn headquarter<'r>(
        &self,
        rows: &[&'r BuildingRow],
        tokens: &BTreeMap<String, u32>,
    ) -> Result<Option<(&IconTemplate, &'r BuildingRow)>, Refusal> {
        self.headquarter_rows(rows.iter().copied(), tokens)
    }

    fn headquarter_rows<'r>(
        &self,
        rows: impl Iterator<Item = &'r BuildingRow> + Clone,
        tokens: &BTreeMap<String, u32>,
    ) -> Result<Option<(&IconTemplate, &'r BuildingRow)>, Refusal> {
        let active = |token: &u32| {
            rows.clone()
                .find(|row| row.building_type == *token && row.level > 0)
        };
        let mut unnamed = None;
        for (name, why) in &self.refused_headquarters {
            match tokens.get(name) {
                Some(token) if active(token).is_some() => {
                    return Err(format!("naval HQ building {name} {why}"));
                }
                Some(_) => {}
                None => {
                    unnamed.get_or_insert(name);
                }
            }
        }
        for template in &self.headquarters {
            let Some(token) = tokens.get(&template.name) else {
                unnamed.get_or_insert(&template.name);
                continue;
            };
            if let Some(row) = active(token) {
                return match unnamed {
                    Some(name) => Err(format!(
                        "naval HQ building {name} has no single token this hour"
                    )),
                    None => Ok(Some((template, row))),
                };
            }
        }
        Ok(None)
    }

    fn facility<'r>(
        &self,
        active: impl IntoIterator<Item = &'r BuildingRow>,
        names: &BTreeMap<u32, &str>,
    ) -> Result<Option<FacilityChoice<'_, 'r>>, Refusal> {
        for row in active {
            let Some(&name) = names.get(&row.building_type) else {
                continue;
            };
            if let Some(why) = self.refused_facilities.get(name) {
                return Err(format!("facility building {name} {why}"));
            }
            if let Some(facility) = self.facilities.get(name) {
                return Ok(Some((row, facility)));
            }
        }
        Ok(None)
    }

    fn unnamed_facility(&self, tokens: &BTreeMap<String, u32>) -> Option<&str> {
        self.facility_names()
            .find(|name| !tokens.contains_key(*name))
    }

    fn anchor(
        &self,
        province: u32,
        template: &IconTemplate,
        slot0: Option<[f32; 3]>,
    ) -> Result<[f32; 3], Refusal> {
        if let Some((bucket, offset)) = &template.anchor {
            if template.ambiguous {
                return Err(format!(
                    "spawn bucket {bucket} of {} is reached by a building or bucket declared \
                     more than once, differently",
                    template.name
                ));
            }
            if let Some(at) = self.records.get(&(province, bucket.clone())) {
                return Ok([at[0] + offset[0], at[1] + offset[1], at[2] + offset[2]]);
            }
        }
        slot0.ok_or_else(|| "the province has no unit-stack slot 0".to_owned())
    }
}

fn redirect_targets(
    buildings: &Buildings,
    test: fn(&BuildingType) -> bool,
) -> BTreeMap<&str, bool> {
    let resolved = buildings.types.values().filter_map(|kind| {
        let bucket = kind.spawn_point.as_deref()?;
        Some((
            kind,
            bucket,
            buildings.repeated_types.contains_key(&kind.name),
        ))
    });
    let authored = buildings
        .repeated_types
        .values()
        .flatten()
        .filter_map(|kind| {
            let bucket = buildings
                .slot(kind.spawn_point.as_deref()?)
                .map(|slot| slot.spawn().name.as_str())
                .filter(|bucket| !buildings.types.contains_key(*bucket))?;
            Some((kind, bucket, true))
        });
    let mut out = BTreeMap::<&str, bool>::new();
    for (kind, bucket, repeated) in resolved.chain(authored) {
        if test(kind) {
            *out.entry(bucket).or_default() |=
                repeated || buildings.ambiguous_slots.contains(bucket);
        }
    }
    out
}

fn unattributable(buildings: &Buildings, keys: &TokenKeys, name: &str) -> Option<String> {
    if buildings.repeated_types.contains_key(name) {
        return Some("is declared more than once, differently".to_owned());
    }
    let others: Vec<&str> = keys
        .keys(name)
        .iter()
        .map(String::as_str)
        .filter(|other| *other != name)
        .collect();
    (!others.is_empty()).then(|| {
        format!(
            "shares its lexer token with building type {}",
            others.join(", ")
        )
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavalBaseIcon {
    pub province: u32,
    pub frame: u32,
    pub highlighted: bool,
    pub enabled: bool,
    pub capacity: Option<naval_repairs::Capacity>,
    pub capacity_refusal: Option<&'static str>,
}

fn refuse(reasons: &mut BTreeMap<String, usize>, reason: impl Into<String>) {
    *reasons.entry(reason.into()).or_default() += 1;
}

#[derive(Clone, Debug, Default)]
pub struct BaseIcons {
    pub naval: Vec<NavalBaseIcon>,
    pub headquarters: Vec<(u32, [f32; 3])>,
    pub facilities: Vec<(u32, [f32; 3], String)>,
    pub unknown: BaseUnknowns,
    pub refused: BTreeSet<String>,
}

fn slot(links: &BTreeMap<u32, u32>, raw: u32) -> Result<Option<u32>, ()> {
    if raw as i32 <= 0 {
        return Ok(None);
    }
    links.get(&raw).copied().map(Some).ok_or(())
}

pub struct BaseContext<'a> {
    pub content: &'a CounterMap,
    pub building_icons: &'a BuildingIconContent,
    pub vision: &'a Vision,
    pub links: &'a BTreeMap<u32, u32>,
    pub country_ids: &'a BTreeMap<u32, u32>,
    pub relations: Result<&'a Relations, &'static str>,
    pub order: Option<&'a counter_order::RecordedOrder>,
    pub carry_tokens: bool,
}

struct ProvinceLookup<'a>(&'a [ProvinceRow]);

impl<'a> ProvinceLookup<'a> {
    fn get(&self, id: &u32) -> Option<&'a ProvinceRow> {
        self.0
            .binary_search_by_key(id, |row| row.id)
            .ok()
            .map(|at| &self.0[at])
    }
}

struct BuildingLookup<'a> {
    rows: &'a [BuildingRow],
}

impl BuildingLookup<'_> {
    fn groups(&self) -> impl Iterator<Item = (u32, &[BuildingRow])> {
        self.rows
            .chunk_by(|a, b| a.location == b.location)
            .map(|rows| (rows[0].location, rows))
    }

    fn get(&self, province: u32) -> &[BuildingRow] {
        let begin = self.rows.partition_point(|row| row.location < province);
        let end = begin + self.rows[begin..].partition_point(|row| row.location == province);
        &self.rows[begin..end]
    }
}

impl BaseIcons {
    pub fn read_profiled(
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<Self> {
        let provinces = spans.measure(
            "overlay.tables.counters.native_base_icons.base.provinces",
            |_| {
                Ok::<_, io::Error>(ProvinceLookup(core::<tables::Provinces>(frame)?))
            },
        )?;
        let mut out = Self::default();
        spans.measure(
            "overlay.tables.counters.native_base_icons.base.naval",
            |_| out.read_naval(frame, context, &provinces),
        )?;
        spans.measure(
            "overlay.tables.counters.native_base_icons.base.buildings",
            |_| out.read_buildings(frame, context, &provinces),
        )?;
        Ok(out)
    }

    fn read_naval(
        &mut self,
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        provinces: &ProvinceLookup<'_>,
    ) -> io::Result<()> {
        let BaseContext {
            content,
            vision,
            links,
            country_ids,
            relations,
            order,
            ..
        } = context;
        let access = AccessLinks::read(frame, links);
        let forces: BTreeMap<u64, u32> = rows::<tables::TaskForces>(frame)?
            .rows()
            .iter()
            .map(|force| (force.id, force.roster_owner))
            .collect();
        let ours = vision.ours();
        let repairs = naval_repairs::Sources::new(frame, links, *relations);
        for base in rows::<tables::NavalBases>(frame)?.rows() {
            if content.port(base.province).is_none() {
                refuse(
                    &mut self.unknown.naval_bases,
                    "the content has no port building slot or unit-stack slot 19 in its province",
                );
                continue;
            }
            let decided = || -> Result<(u32, bool), &'static str> {
                let country = *links
                    .get(&base.country)
                    .ok_or("the base's country is unresolved")?;
                let foreign = country != ours;
                let (known, enabled) = if foreign {
                    let relations = relations.as_ref().map_err(|reason| *reason)?;
                    let access = access.as_ref().map_err(|reason| *reason)?;
                    let province = provinces
                        .get(&base.province)
                        .ok_or("the base's province has no Provinces row")?;
                    let kind = content
                        .province_kind(base.province)
                        .ok_or("the map does not define the base's province")?;
                    let unresolved = |()| "the province's owner or controller is unresolved";
                    let target = ProvinceAccess {
                        lake: kind == ProvinceKind::Lake,
                        sea: kind == ProvinceKind::Sea,
                        state_owner: slot(links, province.owner).map_err(unresolved)?,
                        controller: slot(links, province.controller).map_err(unresolved)?,
                    };
                    (
                        relations.known_port(access, ours, country),
                        relations.province_access(access, ours, target)
                            && !relations.at_war(ours, country),
                    )
                } else {
                    (true, true)
                };
                let disclosed = known && {
                    if base.countries_unreadable {
                        return Err("the base's recorded country list is unreadable");
                    }
                    country_list_contains(
                        &base.countries,
                        *country_ids
                            .get(&ours)
                            .ok_or("the viewing country has no Countries id")?,
                        |raw| links.get(&raw).copied(),
                    )?
                };
                let ships = naval_base_ships(
                    order
                        .as_ref()
                        .ok_or("the province occupant order is unavailable")?
                        .naval_roster(base.province),
                    || vision.naval_port_detail(country),
                    |unit| {
                        let owner = *links.get(forces.get(&unit)?)?;
                        if owner != ours && vision.fog() && vision.undecided().is_some() {
                            return None;
                        }
                        vision.navy_byte_known(unit, owner)
                    },
                )
                .ok_or(
                    "no occupant is seen above 0x95 and an occupant's intel level or the port detail is unknown",
                )?;
                Ok((naval_base_frame(known, foreign, disclosed, ships), enabled))
            };
            match decided() {
                Ok((frame, enabled)) => {
                    let capacity = match vision.naval_port_detail(links[&base.country]) {
                        Some(level) if level < 4 => Ok(None),
                        Some(_) => repairs.capacity(base).map(Some),
                        None => {
                            Err("txt_capacity is withheld: the naval port intel level is unknown")
                        }
                    };
                    self.naval.push(NavalBaseIcon {
                        province: base.province,
                        frame,
                        highlighted: false,
                        enabled,
                        capacity: capacity.as_ref().ok().copied().flatten(),
                        capacity_refusal: capacity.err(),
                    });
                }
                Err(reason) => refuse(&mut self.unknown.naval_bases, reason),
            }
        }
        Ok(())
    }

    fn read_buildings(
        &mut self,
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        provinces: &ProvinceLookup<'_>,
    ) -> io::Result<()> {
        let BaseContext {
            content,
            vision,
            links,
            ..
        } = context;
        let icons = context.building_icons;
        let built = if context.carry_tokens { icons.carried_tokens(frame)? } else { icons.tokens(frame)? };
        let (tokens, refused) = &*built;
        self.refused = refused.clone();
        let names: BTreeMap<u32, &str> = icons
            .facility_names()
            .filter_map(|name| Some((*tokens.get(name)?, name)))
            .collect();
        let buildings = rows::<tables::ProvinceBuildings>(frame)?;
        let complete = buildings.is_complete();
        let by_province = BuildingLookup { rows: buildings.rows() };
        let ours = vision.ours();
        let ours_building = |row: &BuildingRow| -> Option<bool> {
            let raw = if (row.country as i32) > 0 {
                row.country
            } else if row.status_flags & PROVINCE_STATUS != 0 {
                provinces.get(&row.location)?.controller
            } else {
                return None;
            };
            Some(slot(links, raw).ok()? == Some(ours))
        };

        let headquarters_enabled = match frame.complete::<tables::SimulationFeatures>() {
            Ok([features]) => Some(features.dlc_mask & 0x8000 != 0),
            _ => None,
        };
        for (province, rows) in by_province.groups() {
            if headquarters_enabled == Some(false) {
                break;
            }
            let decided = icons
                .headquarter_rows(rows.iter(), tokens)
                .and_then(|found| {
                    let Some((template, row)) = found else {
                        return Ok(None);
                    };
                    if !complete {
                        return Err("the ProvinceBuildings census is incomplete".to_owned());
                    }
                    if headquarters_enabled.is_none() {
                        return Err("the SimulationFeatures DLC mask is unavailable".to_owned());
                    }
                    let ours = ours_building(row)
                        .ok_or_else(|| "the building's country is unresolved".to_owned())?;
                    Ok(ours.then_some(template))
                })
                .and_then(|template| {
                    template
                        .map(|template| {
                            icons.anchor(province, template, content.anchor(province, STANDSTILL))
                        })
                        .transpose()
                });
            match decided {
                Ok(Some(anchor)) => self.headquarters.push((province, anchor)),
                Ok(None) => {}
                Err(reason) => refuse(&mut self.unknown.naval_headquarters, reason),
            }
        }

        let unnamed_facility = icons.unnamed_facility(tokens);
        for flag in rows::<tables::FacilityMapFlags>(frame)?.rows() {
            let Some(province) = provinces.get(&flag.province) else {
                continue;
            };
            if flag.flag == 0 || (province.controller as i32) <= 0 {
                continue;
            }
            let mut active: Vec<&BuildingRow> = by_province
                .get(flag.province)
                .iter()
                .filter(|row| row.level > 0)
                .collect();
            active.sort_by_key(|row| row.ordinal);
            let decided = if !complete {
                Err("the ProvinceBuildings census is incomplete".to_owned())
            } else if let Some(name) = unnamed_facility {
                Err(format!(
                    "facility building {name} has no single token this hour"
                ))
            } else {
                icons.facility(active, &names).and_then(|first| {
                    let Some((row, facility)) = first else {
                        return Ok(None);
                    };
                    let ours = ours_building(row)
                        .ok_or_else(|| "the building's country is unresolved".to_owned())?;
                    Ok(ours.then_some(facility))
                })
            };
            let decided = decided.and_then(|facility| {
                facility
                    .map(|(specialization, template)| {
                        icons
                            .anchor(
                                flag.province,
                                template,
                                content.anchor(flag.province, STANDSTILL),
                            )
                            .map(|anchor| (anchor, specialization))
                    })
                    .transpose()
            });
            match decided {
                Ok(Some((anchor, specialization))) => self.facilities.push((
                    flag.province,
                    anchor,
                    format!("{FACILITY_SPRITE_PREFIX}{specialization}"),
                )),
                Ok(None) => {}
                Err(reason) => refuse(&mut self.unknown.facilities, reason),
            }
        }
        Ok(())
    }
}

const PROVINCE_STATUS: u32 = 1;

pub fn air_base_hidden(map: &MapData, anchor: [f32; 3], vision: &Vision) -> bool {
    let province = province_at(map, anchor);
    let controller = province.and_then(|province| vision.province_controller(province));
    matches!((province, controller), (Some(province), Some(controller)) if vision.sees_air_base(province, controller) == Some(false))
}

pub fn province_at(map: &MapData, world: [f32; 3]) -> Option<u32> {
    if !world[0].is_finite() || !world[2].is_finite() {
        return None;
    }
    let (width, height) = (map.width as f32, map.height as f32);
    if world[0] < 0.0 || world[2] < 0.0 || world[0] >= width || world[2] >= height {
        return None;
    }
    let (width, height) = (i64::from(map.width), i64::from(map.height));
    let column = world[0] as i64;
    let row = height - 1 - world[2] as i64;
    if column < 0 || row < 0 || column >= width || row >= height {
        return None;
    }
    map.province_ids
        .get((row * width + column) as usize)
        .copied()
        .filter(|id| *id != 0)
}

pub fn site_kind(kind: i32) -> usize {
    match kind {
        1 => building_facts::ROCKET_SITE,
        2 => building_facts::GUN_EMPLACEMENT,
        _ => building_facts::AIR_BASE,
    }
}

pub fn site_icon_states(frame: &DecodedFrame) -> io::Result<([BTreeSet<u32>; 3], bool)> {
    let bases = rows::<tables::AirBases>(frame)?;
    Ok((site_states_of(bases.rows().iter().map(|base| (base.state, base.kind))), bases.is_complete()))
}

pub fn site_states_of(bases: impl IntoIterator<Item = (Option<u32>, i32)>) -> [BTreeSet<u32>; 3] {
    let mut states: [BTreeSet<u32>; 3] = Default::default();
    for (state, kind) in bases {
        if let Some(state) = state {
            states[site_kind(kind)].insert(state);
        }
    }
    states
}

type StrategicLocations = (BTreeMap<u32, Vec<u32>>, Vec<String>);

pub fn strategic_locations(frame: &DecodedFrame) -> io::Result<StrategicLocations> {
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut notes = Vec::new();
    let cores = rows::<tables::StateCores>(frame)?;
    if let Available::Unavailable { reason } = cores {
        notes.push(format!(
            "strategic location icons are not drawn: StateCores is unavailable ({reason})"
        ));
        return Ok((out, notes));
    }
    if !cores.is_complete() {
        notes.push("strategic location icons may be missing: StateCores is truncated".into());
    }
    let membership: BTreeMap<u32, u32> = rows::<tables::Provinces>(frame)?
        .rows()
        .iter()
        .map(|province| (province.id, province.state))
        .collect();
    let mut unrecorded = 0;
    let mut unplaced = 0;
    let mut anchored: Vec<(u32, u32)> = Vec::new();
    for row in cores.rows() {
        let (Some(provinces), Some(state)) = (row.province_locations(), row.state_locations())
        else {
            unrecorded += 1;
            continue;
        };
        for (province, token) in provinces {
            out.entry(province).or_default().push(token);
        }
        for (token, province) in state {
            match membership.get(&province) {
                Some(&owner) if owner == row.state => anchored.push((province, token)),
                Some(_) => {}
                None => unplaced += 1,
            }
        }
    }
    for (province, token) in anchored {
        out.entry(province).or_default().push(token);
    }
    if unrecorded > 0 {
        notes.push(format!(
            "{unrecorded} states' strategic locations are not recorded, so their icons are not drawn"
        ));
    }
    if unplaced > 0 {
        notes.push(format!(
            "{unplaced} state strategic locations anchor on a province whose state is not recorded (Provinces), so they are not drawn"
        ));
    }
    Ok((out, notes))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadarStates {
    pub shown: BTreeSet<u32>,
    pub notes: Vec<String>,
}

pub fn radar_states(frame: &DecodedFrame, ours: u32) -> io::Result<RadarStates> {
    let coverage = rows::<tables::RadarCoverage>(frame)?;
    let held = match &coverage {
        Available::Unavailable { reason } => return Ok(radar_of(&RadarInputs { coverage: Err(reason), ..RadarInputs::empty(ours) })),
        other => other.is_complete(),
    };
    let links: BTreeMap<u32, u32> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let groups: Vec<(u32, Option<u32>)> = coverage.rows().iter().map(|group| (group.country, group.state)).collect();
    let members = rows::<tables::FactionMembers>(frame)?;
    let faction_rows: Vec<(u32, u64)> = members.rows().iter().map(|member| (member.country, member.faction)).collect();
    let appearance = rows::<tables::CountryAppearance>(frame)?;
    let slot = |raw: u32| links.get(&raw).copied();
    let ours_row = appearance.rows().iter().find(|row| slot(row.country) == Some(ours)).map(|row| (row.subjects.clone(), row.overlord));
    let controllers: Vec<(u32, u32)> = rows::<tables::States>(frame)?.rows().iter().map(|row| (row.id, row.controller)).collect();
    Ok(radar_of(&RadarInputs {
        coverage: Ok((&groups, held)),
        link: &slot,
        members: &faction_rows,
        members_complete: members.is_complete(),
        appearance: ours_row,
        controllers: &controllers,
        ours,
    }))
}

pub struct RadarInputs<'a> {
    pub coverage: Result<(&'a [(u32, Option<u32>)], bool), &'a str>,
    pub link: &'a dyn Fn(u32) -> Option<u32>,
    pub members: &'a [(u32, u64)],
    pub members_complete: bool,
    pub appearance: Option<(replay_data::rows::Recorded<Vec<i32>>, i32)>,
    pub controllers: &'a [(u32, u32)],
    pub ours: u32,
}

impl RadarInputs<'_> {
    fn empty(ours: u32) -> Self {
        Self { coverage: Ok((&[], true)), link: &|_| None, members: &[], members_complete: true, appearance: None, controllers: &[], ours }
    }
}

pub fn radar_of(inputs: &RadarInputs<'_>) -> RadarStates {
    let (groups, complete) = match inputs.coverage {
        Err(reason) => return RadarStates { shown: BTreeSet::new(), notes: radar_notes_of(Err(reason), 0, 0, &[]) },
        Ok(coverage) => coverage,
    };
    let slot = inputs.link;
    let mut hosts: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for &(country, state) in groups {
        if let (Some(country), Some(state)) = (slot(country), state) {
            hosts.entry(state).or_default().insert(country);
        }
    }
    let circle = RadarCircle::of(slot, inputs.members, inputs.members_complete, inputs.appearance.as_ref(), inputs.ours);
    let controllers: BTreeMap<u32, u32> = inputs
        .controllers
        .iter()
        .filter_map(|&(id, controller)| Some((id, slot(controller)?)))
        .collect();
    let mut out = RadarStates::default();
    let (mut uncontrolled, mut withheld) = (0, 0);
    for (&state, hosting) in &hosts {
        match circle.outcome(hosting.iter().copied(), controllers.get(&state).copied()) {
            RadarOutcome::Shown => {
                out.shown.insert(state);
            }
            RadarOutcome::Withheld => withheld += 1,
            RadarOutcome::Uncontrolled => uncontrolled += 1,
            RadarOutcome::Hidden => {}
        }
    }
    out.notes = radar_notes_of(Ok(complete), withheld, uncontrolled, &circle.undecided);
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadarCircle {
    pub countries: BTreeSet<u32>,
    pub undecided: Vec<&'static str>,
}

impl RadarCircle {
    pub fn of(slot: &dyn Fn(u32) -> Option<u32>, members: &[(u32, u64)], members_complete: bool, appearance: Option<&(replay_data::rows::Recorded<Vec<i32>>, i32)>, ours: u32) -> Self {
        let mut circle = Self { countries: BTreeSet::from([ours]), undecided: Vec::new() };
        if !members_complete {
            circle.undecided.push("FactionMembers");
        }
        let faction = members.iter().find(|(country, _)| slot(*country) == Some(ours)).map(|(_, faction)| *faction);
        if let Some(faction) = faction {
            circle.countries.extend(members.iter().filter(|(_, member)| *member == faction).filter_map(|(country, _)| slot(*country)));
        }
        match appearance {
            Some((subjects, overlord)) => {
                match subjects {
                    replay_data::rows::Recorded::Value(subjects) => circle.countries.extend(subjects.iter().filter_map(|&raw| u32::try_from(raw).ok().and_then(slot))),
                    replay_data::rows::Recorded::NotRecorded => circle.undecided.push("CountryAppearance subjects"),
                }
                if let Some(overlord) = u32::try_from(*overlord).ok().filter(|&raw| raw > 0) {
                    circle.countries.extend(slot(overlord));
                }
            }
            None => circle.undecided.push("CountryAppearance"),
        }
        circle
    }

    pub fn outcome(&self, hosting: impl Iterator<Item = u32> + Clone, controller: Option<u32>) -> RadarOutcome {
        let Some(controller) = controller else { return RadarOutcome::Uncontrolled };
        if !hosting.clone().any(|country| country == controller) {
            RadarOutcome::Hidden
        } else if hosting.into_iter().any(|country| self.countries.contains(&country)) {
            RadarOutcome::Shown
        } else if !self.undecided.is_empty() {
            RadarOutcome::Withheld
        } else {
            RadarOutcome::Hidden
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadarOutcome {
    Shown,
    Hidden,
    Withheld,
    Uncontrolled,
}

pub fn radar_notes_of(coverage: Result<bool, &str>, withheld: usize, uncontrolled: usize, undecided: &[&str]) -> Vec<String> {
    let complete = match coverage {
        Err(reason) => return vec![format!("radar icons are not drawn: RadarCoverage is unavailable ({reason})")],
        Ok(complete) => complete,
    };
    let mut notes = Vec::new();
    if !complete {
        notes.push("radar icons may be missing: the radar groups are not all recorded".into());
    }
    if withheld > 0 {
        notes.push(format!(
            "{withheld} radar host states are not drawn: the viewer's relations are not all recorded ({})",
            undecided.join(", ")
        ));
    }
    if uncontrolled > 0 {
        notes.push(format!(
            "{uncontrolled} radar host states have no recorded controller, so their icons are not drawn"
        ));
    }
    notes
}

#[derive(Clone)]
pub struct BaseTables {
    values: BaseValues,
    locations: BTreeMap<u32, Option<String>>,
    location_notes: Vec<String>,
    volunteers: Arc<Result<air_volunteers::Allowances, String>>,
}

impl BaseTables {
    pub fn values(&self) -> &BaseValues {
        &self.values
    }

    pub fn read(
        frame: &DecodedFrame,
        vision: &Vision,
        volunteer_defines: &Result<air_volunteers::Defines, String>,
    ) -> io::Result<Self> {
        let complete = site_icon_states(frame)?.1;
        let site_bases = site_bases_of(rows::<tables::AirBases>(frame)?.rows().iter().map(|base| (base.id, base.state, base.kind)));
        let radar = radar_states(frame, vision.ours())?;
        let values = BaseValues {
            wings: wings_per_site(frame, vision)?.into_iter().collect(),
            complete,
            aircraft_complete: rows::<tables::AirWings>(frame)?.is_complete(),
            controlled: controlled_provinces(frame)?.into_iter().collect(),
            radar: radar.shown.iter().map(|&state| (state, ())).collect(),
            radar_notes: Arc::new(radar.notes),
            site_bases: site_bases.into_iter().collect(),
        };
        Ok(Self { values, ..Self::rest(frame, vision, volunteer_defines)? })
    }

    pub fn none() -> Self {
        Self {
            values: BaseValues::default(),
            locations: BTreeMap::new(),
            location_notes: Vec::new(),
            volunteers: Arc::new(Err("no base, headquarters, facility or strategic-location icon is drawn at this camera, so the air volunteer allowances were not read".to_owned())),
        }
    }

    pub fn rest(frame: &DecodedFrame, vision: &Vision, volunteer_defines: &Result<air_volunteers::Defines, String>) -> io::Result<Self> {
        let (locations, location_notes) = location_icons(frame)?;
        Ok(Self {
            values: BaseValues::default(),
            locations,
            location_notes,
            volunteers: Arc::new(air_volunteers::Allowances::read(frame, vision.ours(), volunteer_defines)),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BaseValues {
    pub wings: Shards<(u32, usize), AirBaseCount>,
    pub complete: bool,
    pub aircraft_complete: bool,
    pub controlled: Shards<u32, bool>,
    pub radar: Shards<u32, ()>,
    pub radar_notes: Arc<Vec<String>>,
    pub site_bases: Shards<(u32, i32), Vec<u64>>,
}

pub fn site_bases_of(bases: impl IntoIterator<Item = (u64, Option<u32>, i32)>) -> BTreeMap<(u32, i32), Vec<u64>> {
    let mut out = BTreeMap::<_, Vec<_>>::new();
    for (id, state, kind) in bases {
        if let Some(state) = state {
            out.entry((state, kind)).or_default().push(id);
        }
    }
    out
}

pub fn controlled_provinces(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, bool>> {
    Ok(rows::<tables::Provinces>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, controlled_of(row.controller)))
        .collect())
}

pub fn controlled_of(controller: u32) -> bool {
    controller > 0
}

type LocationIcons = (BTreeMap<u32, Option<String>>, Vec<String>);

fn location_icons(frame: &DecodedFrame) -> io::Result<LocationIcons> {
    let (locations, notes) = strategic_locations(frame)?;
    let names = TokenNames::new(frame)?;
    Ok((
        locations
            .into_iter()
            .map(|(province, tokens)| {
                (
                    province,
                    names
                        .name(tokens[0])
                        .ok()
                        .map(|first| format!("{STRATEGIC_LOCATION_SPRITE_PREFIX}{first}")),
                )
            })
            .collect(),
        notes,
    ))
}

pub fn bases(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    tables: &BaseHour<'_>,
    layered: Option<&BaseValues>,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    let sites = layered.unwrap_or(&tables.bases.values);
    lay_out_bases(inputs, frame, tables, sites, &mut out.native_bases, &mut out.report, &mut Marks::default())
}

const ICON_FAMILIES: [(base_windows::Family, &str); 4] = [
    (base_windows::Family::AirBase, AIR_BASE_WINDOW),
    (base_windows::Family::RocketSite, ROCKET_SITE_WINDOW),
    (base_windows::Family::GunEmplacement, GUN_EMPLACEMENT_WINDOW),
    (base_windows::Family::Radar, RADAR_WINDOW),
];
const RADAR_KIND: usize = building_facts::RADAR;

fn kind_shown(inputs: &CounterInputs<'_>, kind: usize) -> bool {
    let defines = &inputs.content.defines;
    inputs.visibility.province_icons && inputs.shows(if kind == RADAR_KIND { defines.radar_cutoff } else { defines.air_base_cutoff })
}

fn radar_notes(sites: &BaseValues) -> Vec<String> {
    sites.radar_notes.iter().map(|note| format!("{RADAR_WINDOW}: {note}")).collect()
}

#[derive(Default)]
struct Marks {
    icons: [HashMap<u32, SiteMarks>; 4],
    radar: Vec<String>,
}

fn lay_out_bases(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    tables: &BaseHour<'_>,
    sites: &BaseValues,
    offers: &mut base_windows::PreparedBases,
    report: &mut CounterReport,
    marks: &mut Marks,
) -> io::Result<()> {
    report.refused_tokens.clone_from(&tables.base_icons.refused);
    if (0..ICON_FAMILIES.len()).any(|kind| kind_shown(inputs, kind)) {
        let context = SiteContext::new(inputs, frame, tables, sites)?;
        for kind in (0..ICON_FAMILIES.len()).filter(|&kind| kind_shown(inputs, kind)) {
            if kind == RADAR_KIND {
                marks.radar = radar_notes(sites);
                add_notes(&mut report.icon_gui_notes, &marks.radar, 1);
            } else if !sites.complete {
                *report
                    .icon_gui_notes
                    .entry(format!("{} icons may be missing: the air bases are not all recorded", ICON_FAMILIES[kind].1))
                    .or_default() += 1;
            }
            for (province, state, anchor) in inputs.content.site_icons(kind) {
                let laid = context.lay_out(kind, province, state, anchor).add(inputs, kind, province, offers, report);
                if !laid.is_empty() {
                    marks.icons[kind].insert(province, laid);
                }
            }
        }
    }
    let mut out = CounterFrame::<(), crate::base_windows::PreparedBases, (), crate::naval_windows::Plan>::default();
    strategic_location_icons_cached(inputs, &tables.bases.locations, &tables.bases.location_notes, &mut out)?;
    facilities(inputs, tables, &mut out);
    offers.take_others(out.native_bases);
    for (note, count) in &out.report.icon_gui_notes {
        *report.icon_gui_notes.entry(note.clone()).or_default() += count;
    }
    report.unknown_bases.clone_from(&out.report.unknown_bases);
    Ok(())
}

fn add_notes(target: &mut BTreeMap<String, usize>, notes: &[String], each: usize) {
    for note in notes {
        *target.entry(note.clone()).or_default() += each;
    }
}

fn take_back_notes(target: &mut BTreeMap<String, usize>, notes: &[String]) {
    for note in notes {
        if let Some(held) = target.get_mut(note) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                target.remove(note);
            }
        }
    }
}

fn facilities(inputs: &CounterInputs<'_>, tables: &BaseHour<'_>, out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>) {
    let icons = &tables.base_icons;
    out.report.unknown_bases.clone_from(&icons.unknown);
    if !inputs.shows(inputs.content.defines.facility_cutoff) {
        return;
    }
    for &(province, anchor) in &icons.headquarters {
        out.native_bases.offer(
            inputs,
            base_windows::Family::Headquarter,
            province,
            province,
            anchor,
            base_windows::State {
                air: None,
                frame: 1,
                enabled: true,
                sprite: None,
            },
        );
    }
    for (province, anchor, sprite) in &icons.facilities {
        out.native_bases.offer(
            inputs,
            base_windows::Family::Facility,
            *province,
            *province,
            *anchor,
            base_windows::State {
                air: None,
                frame: 0,
                enabled: true,
                sprite: Some(sprite.clone()),
            },
        );
    }
}

fn radar_entry(sites: &BaseValues, state: u32, anchor: Result<[f32; 3], String>) -> SiteEntry {
    let mut entry = SiteEntry::default();
    if !sites.radar.contains_key(&state) {
        return entry;
    }
    match anchor {
        Ok(anchor) => entry.offer = Some((anchor, base_windows::State { air: None, frame: 0, enabled: true, sprite: None })),
        Err(reason) => entry.notes.push(format!("{RADAR_WINDOW} is not placed: {reason}")),
    }
    entry
}

struct SiteContext<'a, 'b> {
    inputs: &'a CounterInputs<'b>,
    frame: &'a DecodedFrame,
    sites: &'a BaseValues,
    vision: &'a Vision,
    volunteers: &'a Result<air_volunteers::Allowances, String>,
    selected: BTreeSet<(u32, usize)>,
    census: std::cell::OnceCell<Result<air_grid::Census<'a>, String>>,
}

impl<'a, 'b> SiteContext<'a, 'b> {
    fn new(inputs: &'a CounterInputs<'b>, frame: &'a DecodedFrame, tables: &'a BaseHour<'a>, sites: &'a BaseValues) -> io::Result<Self> {
        let base_rows = rows::<tables::AirBases>(frame)?;
        let selected = inputs
            .selected_air_bases
            .iter()
            .filter_map(|handle| {
                let at = base_rows.rows().binary_search_by_key(handle, |row| row.id).ok()?;
                let base = &base_rows.rows()[at];
                Some((base.state?, site_kind(base.kind)))
            })
            .collect();
        Ok(Self { inputs, frame, sites, vision: tables.vision, volunteers: &tables.bases.volunteers, selected, census: Default::default() })
    }

    fn lay_out(&self, kind: usize, province: u32, state: u32, anchor: Result<[f32; 3], String>) -> SiteEntry {
        viewer_counter_kit::plan_probe::record(|counts| counts.site_icons += 1);
        if kind == RADAR_KIND {
            return radar_entry(self.sites, state, anchor);
        }
        let (family, window) = ICON_FAMILIES[kind];
        let (inputs, vision) = (self.inputs, self.vision);
        let mut entry = SiteEntry::default();
        if !self.sites.wings.contains_key(&(state, kind)) {
            return entry;
        }
        let anchor = match anchor {
            Ok(anchor) => anchor,
            Err(reason) => {
                entry.notes.push(format!("{window} is not placed: {reason}"));
                return entry;
            }
        };
        match self.sites.controlled.get(&province) {
            Some(true) => {}
            Some(false) => return entry,
            None => {
                entry.notes.push(format!("{window} is not drawn: its province's controller is not recorded (Provinces)"));
                return entry;
            }
        }
        if air_base_hidden(inputs.map, anchor, vision) {
            entry.hidden = true;
            return entry;
        }
        let mut count = self.sites.wings.get(&(state, kind)).copied().unwrap_or_default();
        count.selected = self.selected.contains(&(state, kind));
        let (frame, enabled) = if kind == building_facts::AIR_BASE {
            let detail = vision.province_controller(province).and_then(|c| vision.sees_air_port_detail(c));
            (air_base_frame(&count, detail).wrapping_add(1) as i32, count.shown)
        } else {
            (site_frame(&count), count.shown)
        };
        let air = if inputs.visibility.priorities == viewer_foundation::mode_visibility::PriorityTable::StrategicAir {
            let mut display = count;
            display.shown &= kind == building_facts::AIR_BASE && !inputs.selected_air.is_empty();
            if kind == building_facts::AIR_BASE && (!self.sites.aircraft_complete || !self.sites.complete) {
                display.shown = false;
                entry.notes.push("air_base_mapicon wings_count is withheld: the AirWings or AirBases census is incomplete".into());
            }
            if display.shown {
                let allowance = (|| {
                    let controller = vision.province_controller(province).ok_or("Air volunteer base controller unavailable")?;
                    self.volunteers.as_ref().map_err(Clone::clone)?.host(controller)
                })();
                match allowance {
                    Ok(value) => display.volunteers = value,
                    Err(reason) => entry.notes.push(format!("air_base_mapicon permission_wings_count is withheld: {reason}")),
                }
            }
            if count.shown
                && let Some((grid_index, _)) = family.wing_grid()
            {
                let matching = self.sites.site_bases.get(&(state, kind as i32)).map_or(&[][..], Vec::as_slice);
                let grid = match (matching, self.census.get_or_init(|| air_grid::Census::new(self.frame))) {
                    ([base], Ok(census)) => census.base(inputs, *base, vision.ours()),
                    (_, Err(reason)) => Err(reason.clone()),
                    _ => Err("state's air-base identity is not unique".into()),
                };
                entry.grid = Some((grid_index, grid));
            }
            Some(display)
        } else {
            None
        };
        entry.offer = Some((anchor, base_windows::State { air, frame, enabled, sprite: None }));
        entry
    }
}

#[derive(Default)]
struct SiteEntry {
    offer: Option<([f32; 3], base_windows::State)>,
    grid: Option<(usize, Result<Vec<air_grid::PreparedItem>, String>)>,
    notes: Vec<String>,
    hidden: bool,
}

impl SiteEntry {
    fn add(self, inputs: &CounterInputs<'_>, kind: usize, province: u32, offers: &mut base_windows::PreparedBases, report: &mut CounterReport) -> SiteMarks {
        add_notes(&mut report.icon_gui_notes, &self.notes, 1);
        report.hidden_air_bases += usize::from(self.hidden);
        if let Some((grid_index, grid)) = self.grid {
            offers.air_grids.insert((grid_index, province), grid);
        }
        if let Some((anchor, state)) = self.offer {
            offers.offer(inputs, ICON_FAMILIES[kind].0, province, province, anchor, state);
        }
        SiteMarks { notes: self.notes, hidden: self.hidden }
    }
}

#[derive(Default)]
struct SiteMarks {
    notes: Vec<String>,
    hidden: bool,
}

impl SiteMarks {
    fn is_empty(&self) -> bool {
        self.notes.is_empty() && !self.hidden
    }

    fn take_back(&self, report: &mut CounterReport) {
        take_back_notes(&mut report.icon_gui_notes, &self.notes);
        report.hidden_air_bases -= usize::from(self.hidden);
    }
}

pub struct BaseLayout {
    rules: [bool; 3],
    values: Option<Arc<BaseValues>>,
    pub offers: base_windows::PreparedBases,
    pub report: CounterReport,
    marks: Marks,
    by_state: Option<[HashMap<u32, Vec<u32>>; 4]>,
}

impl BaseLayout {
    pub fn new(rules: [bool; 3], inputs: &CounterInputs<'_>, frame: &DecodedFrame, tables: &BaseHour<'_>, values: Option<Arc<BaseValues>>) -> io::Result<Self> {
        let mut layout = Self { rules, values, offers: Default::default(), report: Default::default(), marks: Marks::default(), by_state: None };
        let sites = layout.values.as_deref().unwrap_or(&tables.bases.values);
        lay_out_bases(inputs, frame, tables, sites, &mut layout.offers, &mut layout.report, &mut layout.marks)?;
        layout.offers.forget_placed();
        Ok(layout)
    }

    pub fn rules(&self) -> [bool; 3] {
        self.rules
    }

    pub fn laid_from(&self, values: Option<&Arc<BaseValues>>) -> bool {
        match (&self.values, values) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    pub fn update(&mut self, inputs: &CounterInputs<'_>, frame: &DecodedFrame, tables: &BaseHour<'_>, values: Option<&Arc<BaseValues>>) -> io::Result<()> {
        let (Some(old), Some(new)) = (self.values.clone(), values) else {
            *self = Self::new(self.rules, inputs, frame, tables, values.cloned())?;
            return Ok(());
        };
        if old.complete != new.complete || old.aircraft_complete != new.aircraft_complete {
            *self = Self::new(self.rules, inputs, frame, tables, values.cloned())?;
            return Ok(());
        }
        self.values = Some(Arc::clone(new));
        let shown: [bool; 4] = std::array::from_fn(|kind| kind_shown(inputs, kind));
        if shown[RADAR_KIND] && old.radar_notes != new.radar_notes {
            take_back_notes(&mut self.report.icon_gui_notes, &self.marks.radar);
            self.marks.radar = radar_notes(new);
            add_notes(&mut self.report.icon_gui_notes, &self.marks.radar, 1);
        }
        let by_state = self.by_state.get_or_insert_with(|| {
            std::array::from_fn(|kind| {
                let mut by_state: HashMap<u32, Vec<u32>> = HashMap::new();
                for (province, state, _) in inputs.content.site_icons(kind) {
                    by_state.entry(state).or_default().push(province);
                }
                by_state
            })
        });
        let mut states: BTreeSet<(usize, u32)> = new.wings.differing(&old.wings).map(|(state, kind)| (kind, state)).collect();
        states.extend(new.site_bases.differing(&old.site_bases).map(|(state, kind)| (site_kind(kind), state)));
        states.extend(new.radar.differing(&old.radar).map(|state| (RADAR_KIND, state)));
        let mut icons: BTreeSet<(usize, u32)> = BTreeSet::new();
        for (kind, state) in states {
            icons.extend(by_state[kind].get(&state).into_iter().flatten().map(|&province| (kind, province)));
        }
        for province in new.controlled.differing(&old.controlled) {
            icons.extend((0..RADAR_KIND).filter(|&kind| inputs.content.site_icons[kind].contains_key(&province)).map(|kind| (kind, province)));
        }
        icons.retain(|&(kind, _)| shown[kind]);
        if icons.is_empty() {
            return Ok(());
        }
        let context = SiteContext::new(inputs, frame, tables, new)?;
        for (kind, province) in icons {
            if let Some(marks) = self.marks.icons[kind].remove(&province) {
                marks.take_back(&mut self.report);
            }
            self.offers.remove_site(ICON_FAMILIES[kind].0, province);
            if let Some((grid_index, _)) = ICON_FAMILIES[kind].0.wing_grid() {
                self.offers.air_grids.remove(&(grid_index, province));
            }
            let Some((state, anchor)) = inputs.content.site_icon(kind, province) else { continue };
            let laid = context.lay_out(kind, province, state, anchor).add(inputs, kind, province, &mut self.offers, &mut self.report);
            if !laid.is_empty() {
                self.marks.icons[kind].insert(province, laid);
            }
        }
        self.offers.forget_placed();
        Ok(())
    }
}

#[cfg(test)]
pub fn strategic_location_icons(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    let (locations, notes) = location_icons(frame)?;
    strategic_location_icons_cached(inputs, &locations, &notes, out)
}

fn strategic_location_icons_cached(
    inputs: &CounterInputs<'_>,
    locations: &BTreeMap<u32, Option<String>>,
    notes: &[String],
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    let defines = &inputs.content.defines;
    if inputs.visibility.province_icons && inputs.shows(defines.air_base_cutoff) {
        for note in notes {
            *out.report
                .icon_gui_notes
                .entry(format!("{STRATEGIC_LOCATION_WINDOW}: {note}"))
                .or_default() += 1;
        }
        for (&province, sprite) in locations {
            let Some(anchor) = inputs.content.anchor(province, VICTORY_POINT) else {
                *out.report
                    .icon_gui_notes
                    .entry(format!(
                        "{STRATEGIC_LOCATION_WINDOW} is not placed: its province has no victory-point anchor"
                    ))
                    .or_default() += 1;
                continue;
            };
            let Some(sprite) = sprite else {
                *out.report
                    .icon_gui_notes
                    .entry(format!(
                        "{STRATEGIC_LOCATION_WINDOW} is not drawn: its first location has no recorded spelling (LexicalTokens)"
                    ))
                    .or_default() += 1;
                continue;
            };
            out.native_bases.offer(
                inputs,
                base_windows::Family::StrategicLocation,
                province,
                province,
                anchor,
                base_windows::State {
                    air: None,
                    frame: 0,
                    enabled: true,
                    sprite: Some(sprite.clone()),
                },
            );
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn radars(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    vision: &Vision,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    if !kind_shown(inputs, RADAR_KIND) {
        return Ok(());
    }
    let radar = radar_states(frame, vision.ours())?;
    let sites = BaseValues { radar: radar.shown.iter().map(|&state| (state, ())).collect(), radar_notes: Arc::new(radar.notes), ..BaseValues::default() };
    add_notes(&mut out.report.icon_gui_notes, &radar_notes(&sites), 1);
    for (province, state, anchor) in inputs.content.site_icons(RADAR_KIND) {
        radar_entry(&sites, state, anchor).add(inputs, RADAR_KIND, province, &mut out.native_bases, &mut out.report);
    }
    Ok(())
}

pub fn building_tokens(
    frame: &DecodedFrame,
    keys: &TokenKeys,
    names: &BTreeSet<&str>,
) -> io::Result<(BTreeMap<String, u32>, BTreeSet<String>)> {
    let tokens = TokenNames::new(frame)?;
    let mut found: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    let mut refused = BTreeSet::new();
    for (token, spelling) in tokens.iter() {
        let group = keys.keys(spelling);
        let mut named = group
            .iter()
            .map(String::as_str)
            .filter(|key| names.contains(key))
            .peekable();
        if named.peek().is_none() {
            continue;
        }
        if group.len() > 1 {
            refused.insert(format!(
                "building token {token} ({spelling}) names several building types: {}",
                group.join(", ")
            ));
        }
        for key in named {
            found.entry(key).or_default().push(token);
        }
    }
    let mut out = BTreeMap::new();
    for (key, tokens) in found {
        match tokens.as_slice() {
            [token] => {
                out.insert(key.to_owned(), *token);
            }
            _ => {
                refused.insert(format!(
                    "building {key} folds to several tokens: {tokens:?}"
                ));
            }
        }
    }
    Ok((out, refused))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AirBaseCount {
    pub volunteers: Option<(i32, i32)>,
    pub selected: bool,
    pub planes: i64,
    pub capacity: i32,
    pub shown: bool,
    pub stationed: Option<bool>,
    pub occupied: Option<bool>,
    pub foreign: Option<bool>,
}

pub fn air_base_frame(count: &AirBaseCount, detail: Option<bool>) -> u32 {
    let badge = match (count.shown, count.stationed, count.occupied, detail) {
        (true, Some(stationed), _, _) => stationed,
        (true, None, _, _) => false,
        (false, _, Some(occupied), Some(detail)) => detail && occupied,
        (false, _, _, Some(false)) => false,
        _ => count.planes > 0,
    };
    let foreign = count.shown && count.foreign == Some(true);
    u32::from(badge)
        + if count.shown && count.selected {
            5
        } else if foreign {
            3
        } else {
            0
        }
}

pub fn site_frame(count: &AirBaseCount) -> i32 {
    if !count.shown {
        3
    } else if count.occupied.unwrap_or(count.planes > 0) {
        2
    } else {
        1
    }
}

pub fn wings_per_site(
    frame: &DecodedFrame,
    vision: &Vision,
) -> io::Result<BTreeMap<(u32, usize), AirBaseCount>> {
    let flags = rows::<tables::StateFlags>(frame)?;
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let air_wings = rows::<tables::AirWings>(frame)?;
    let registrations = rows::<tables::AirBaseRegistrations>(frame)?;
    let flag_rows: Vec<(u32, u8)> = flags.rows().iter().map(|row| (row.state, row.demilitarized)).collect();
    let wings: Vec<WingFields> = air_wings
        .rows()
        .iter()
        .map(|wing| WingFields { id: wing.id, owner: wing.owner, country_override: wing.country_override, air_base: wing.air_base, base_override: wing.base_override, planes: wing.planes })
        .collect();
    let registered: Vec<RegistrationFields> = registrations
        .rows()
        .iter()
        .map(|row| RegistrationFields { base: row.base, slot: row.slot, capacity_raw: row.capacity_raw, wings: row.wings.clone() })
        .collect();
    let bases: Vec<AirBaseFields> = rows::<tables::AirBases>(frame)?
        .rows()
        .iter()
        .map(|base| AirBaseFields { id: base.id, state: base.state, kind: base.kind, capacity: base.capacity })
        .collect();
    Ok(wings_of(&WingInputs {
        flags: &flag_rows,
        flags_complete: flags.is_complete(),
        link: &|raw| links.get(&raw).copied(),
        wings: &wings,
        wings_complete: air_wings.is_complete(),
        registrations: &registered,
        registrations_complete: registrations.is_complete(),
        registrations_truncated: matches!(registrations, Available::Truncated { .. }),
        bases: &bases,
        ours: vision.ours(),
    }))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WingFields {
    pub id: u64,
    pub owner: u32,
    pub country_override: i32,
    pub air_base: u64,
    pub base_override: u64,
    pub planes: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistrationFields {
    pub base: u64,
    pub slot: u32,
    pub capacity_raw: i64,
    pub wings: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AirBaseFields {
    pub id: u64,
    pub state: Option<u32>,
    pub kind: i32,
    pub capacity: i32,
}

pub struct WingInputs<'a> {
    pub flags: &'a [(u32, u8)],
    pub flags_complete: bool,
    pub link: &'a dyn Fn(u32) -> Option<u32>,
    pub wings: &'a [WingFields],
    pub wings_complete: bool,
    pub registrations: &'a [RegistrationFields],
    pub registrations_complete: bool,
    pub registrations_truncated: bool,
    pub bases: &'a [AirBaseFields],
    pub ours: u32,
}

pub fn wings_of(inputs: &WingInputs<'_>) -> BTreeMap<(u32, usize), AirBaseCount> {
    let mut flags: BTreeMap<u32, u8> = BTreeMap::new();
    for &(state, demilitarized) in inputs.flags {
        let flag = flags.entry(state).or_default();
        if demilitarized != 0 {
            *flag = demilitarized;
        }
    }
    let planes: BTreeMap<u64, i32> = inputs.wings.iter().map(|wing| (wing.id, wing.planes)).collect();
    let mut located: BTreeSet<(u64, u32)> = BTreeSet::new();
    let mut standing: BTreeMap<u64, i64> = BTreeMap::new();
    for wing in inputs.wings {
        let base = wing_base_of(wing.air_base, wing.base_override);
        if let Some(slot) = (inputs.link)(wing_country(wing.owner, wing.country_override)) {
            located.insert((base, slot));
        }
        *standing.entry(base).or_default() += i64::from(wing.planes.max(0));
    }
    let mut registered: BTreeMap<u64, Vec<RegistrationFacts>> = BTreeMap::new();
    for row in inputs.registrations {
        registered.entry(row.base).or_default().push(RegistrationFacts {
            slot: row.slot,
            capacity_raw: row.capacity_raw,
            listed: !row.wings.is_empty(),
            occupied: row.wings.iter().any(|wing| planes.get(wing).is_some_and(|&planes| planes > 0)),
            missing: row.wings.iter().any(|wing| !planes.contains_key(wing)),
        });
    }
    let census = WingCensus {
        flags_complete: inputs.flags_complete,
        wings_complete: inputs.wings_complete,
        registrations_complete: inputs.registrations_complete,
        registrations_truncated: inputs.registrations_truncated,
        ours: inputs.ours,
    };
    let bases: BTreeMap<u64, (u32, &AirBaseFields)> = inputs.bases.iter().filter_map(|base| Some((base.id, (base.state?, base)))).collect();
    let mut out: BTreeMap<(u32, usize), AirBaseCount> = BTreeMap::new();
    for (state, base) in bases.into_values() {
        let facts = BaseFacts {
            capacity: base.capacity,
            flag: flags.get(&state).copied(),
            registrations: registered.get(&base.id).map_or(&[], Vec::as_slice),
            located: &|slot| located.contains(&(base.id, slot)),
            standing: standing.get(&base.id).copied().unwrap_or(0),
        };
        let count = base_count(&facts, &census);
        out.entry((state, site_kind(base.kind)))
            .and_modify(|into| merge_count(into, count))
            .or_insert(count);
    }
    out
}

pub fn wing_country(owner: u32, country_override: i32) -> u32 {
    if country_override > 0 {
        country_override as u32
    } else {
        owner
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WingCensus {
    pub flags_complete: bool,
    pub wings_complete: bool,
    pub registrations_complete: bool,
    pub registrations_truncated: bool,
    pub ours: u32,
}

pub struct BaseFacts<'a> {
    pub capacity: i32,
    pub flag: Option<u8>,
    pub registrations: &'a [RegistrationFacts],
    pub located: &'a dyn Fn(u32) -> bool,
    pub standing: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistrationFacts {
    pub slot: u32,
    pub capacity_raw: i64,
    pub listed: bool,
    pub occupied: bool,
    pub missing: bool,
}

pub fn base_count(facts: &BaseFacts<'_>, census: &WingCensus) -> AirBaseCount {
    let (mut access, mut ours_row, mut stationed, mut foreign, mut occupied, mut unweighed) = (false, false, false, false, false, false);
    for row in facts.registrations {
        let wings = row.listed || (facts.located)(row.slot);
        if row.slot == census.ours {
            access |= row.capacity_raw > 0;
            ours_row = true;
            stationed |= wings;
        } else {
            foreign |= wings;
        }
        if row.occupied {
            occupied = true;
        } else if !census.wings_complete && row.missing {
            unweighed = true;
        }
    }
    let complete = census.registrations_complete;
    AirBaseCount {
        volunteers: None,
        selected: false,
        planes: facts.standing,
        capacity: facts.capacity,
        shown: (census.flags_complete || facts.flag.is_some()) && access && facts.flag.is_none_or(|flag| flag == 0),
        stationed: (complete || (census.registrations_truncated && ours_row)).then_some(stationed),
        occupied: if occupied {
            Some(true)
        } else if unweighed {
            None
        } else {
            complete.then_some(false)
        },
        foreign: complete.then_some(foreign),
    }
}

pub fn merge_count(into: &mut AirBaseCount, count: AirBaseCount) {
    into.planes += count.planes;
    into.capacity += count.capacity;
    into.shown |= count.shown;
    into.stationed = merge(into.stationed, count.stationed);
    into.occupied = merge(into.occupied, count.occupied);
    into.foreign = merge(into.foreign, count.foreign);
}

pub fn wing_base_of(air_base: u64, base_override: u64) -> u64 {
    if base_override != 0 {
        base_override
    } else {
        air_base
    }
}

pub fn merge(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

pub struct BaseHour<'a> {
    pub vision: &'a Vision,
    pub bases: &'a BaseTables,
    pub base_icons: &'a BaseIcons,
}
