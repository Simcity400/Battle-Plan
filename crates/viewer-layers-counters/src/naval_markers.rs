use viewer_counter_kit::combat_markers::whole_of;

use viewer_layer_core::{layers::{Carried, Change, Settling, Inputs, KeptAt, Keys, Layer, List, Num, Text, Upstream, View}, engagements::{Engagements, Index}};
use super::land_markers::{bytes_of, held, held_as, opaque, u64s};
use viewer_layer_core::playhead_host::mirror::{Mirror, Mirrors};
use content_reader_map::map::ProvinceKind;
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::rows::{NavalCombatSideDetails, NavalProvinceCombatRow, TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_counters_naval::naval_markers::{DetailFacts, HqForces, NavalMarkerInputs, NavalMarkerValues, ResultFields, naval_marker_values, result_country_lists};
use viewer_counter_kit::naval_combat_occurrences::{Combat, Direct, Transfers, Units};
use viewer_engagements::{
    engaged::Engaged,
    engagement_history::{NAVAL, NAVAL_ICON, Provenance},
    icon_frames::Kept,
    naval_icon_frames::CensusMember,
};
use viewer_foundation::{perspective::{Choice, Vision}, unit_rules::STANDSTILL};

pub trait Shore: Send + Sync {
    fn province_kind(&self, province: u32) -> Option<ProvinceKind>;
    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]>;
}


fn whole(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| mirror.status() == Some(Status::Complete))
}

fn sorted(mirror: &Mirror) -> Vec<(u128, u32)> {
    let mut rows: Vec<(u128, u32)> = mirror.rows().filter_map(|slot| Some((mirror.key(slot)?, slot))).collect();
    rows.sort_unstable();
    rows
}

#[derive(Clone, Copy)]
struct Columns {
    home: Num<u32>,
    registry_ordinal: Num<u32>,
    vector: List,
    kinds: Text,
    detail_sides: List,
    transfers: List,
    units: List,
    ship: Num<u64>,
    member_source: Num<u8>,
    group_index: Num<u32>,
    live_convoy: Num<u8>,
    live_ship: Num<u8>,
    formation_flags: Num<u64>,
    ship_force: Num<u64>,
    roster_owner: Num<u32>,
    headquarters: Num<i32>,
    division_province: Num<u32>,
    force_province: Num<u32>,
    gun_province: Num<u32>,
    result_province: Num<u32>,
    result_ordinal: Num<u32>,
    flag_a: Num<u8>,
    flag_b: Num<u8>,
    subtype: Num<u8>,
    readiness: Num<u8>,
    winning_side: Num<u8>,
    losing_side: Num<u8>,
    presence: Num<u64>,
    country_ordinal: Num<u32>,
    link_slot: Num<u32>,
}

const TABLES: [TableId; 15] = [
    TableId::NavalCombats,
    TableId::NavalProvinceCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatTransfers,
    TableId::NavalTransferUnits,
    TableId::NavalCombatShips,
    TableId::Ships,
    TableId::TaskForces,
    TableId::TaskForceCounterInputs,
    TableId::Divisions,
    TableId::RailwayGunCounterInputs,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::CountryLinks,
];

const NAVAL_TABLES: [TableId; 10] = [
    TableId::NavalCombats,
    TableId::NavalProvinceCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatTransfers,
    TableId::NavalTransferUnits,
    TableId::NavalCombatShips,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::CountryLinks,
];

const WATCHED: [TableId; 5] = [TableId::Ships, TableId::TaskForces, TableId::TaskForceCounterInputs, TableId::Divisions, TableId::RailwayGunCounterInputs];

pub struct NavalMarkers {
    shore: Arc<dyn Shore>,
    index: Option<Index>,
    columns: Option<Columns>,
    refused: Option<String>,
    values: Option<Arc<NavalMarkerValues>>,
    watched: HashSet<u64>,
    kept: BTreeMap<u64, Kept>,
    kept_at: KeptAt,
    kept_owed: Keys,
    reads: BTreeMap<usize, Vec<usize>>,
    read: usize,
    frames_read: usize,
    flags: Option<(BTreeMap<u64, Engaged>, Provenance)>,
    dirty: bool,
    since: Option<u64>,
    all: bool,
    hidden: bool,
    stepped: bool,
    vision: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision_new: AtomicBool,
    statuses: HashMap<usize, Status>,
    frame: u64,
    made: usize,
}

impl NavalMarkers {
    pub fn new(shore: Arc<dyn Shore>, index: Option<Index>) -> Self {
        Self {
            shore,
            index,
            columns: None,
            refused: Some("not composed yet".into()),
            values: None,
            watched: HashSet::new(),
            kept: BTreeMap::new(),
            kept_at: KeptAt::Unread,
            kept_owed: Keys::default(),
            reads: BTreeMap::new(),
            read: 0,
            frames_read: 0,
            flags: None,
            dirty: true,
            since: None,
            all: true,
            hidden: false,
            stepped: false,
            vision: Mutex::new(None),
            vision_new: AtomicBool::new(false),
            statuses: HashMap::new(),
            frame: 0,
            made: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        let index = self.index.as_ref().map(|index| Index { history: Arc::clone(&index.history), branch: index.branch });
        let fresh = Self { columns: self.columns, hidden: self.hidden, flags: self.flags.clone(), reads: self.reads.clone(), ..Self::new(Arc::clone(&self.shore), index) };
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

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    pub fn values(&self) -> Option<&NavalMarkerValues> {
        self.values.as_deref()
    }

    pub fn published(&self) -> Option<&Arc<NavalMarkerValues>> {
        self.values.as_ref().filter(|_| self.refused.is_none() && !self.hidden)
    }

    pub fn made(&self) -> usize {
        self.made
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn frames_read(&self) -> usize {
        self.frames_read
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the naval markers layer is declared before it composes")
    }

    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.shore), None);
        let kept = std::mem::replace(self, empty);
        (self.index, self.columns, self.hidden, self.flags, self.frame, self.made) = (kept.index, kept.columns, kept.hidden, kept.flags, kept.frame, kept.made);
        (self.vision, self.vision_new, self.reads) = (kept.vision, kept.vision_new, kept.reads);
    }

    fn refuse(&mut self, why: String) {
        if self.refused.as_ref() != Some(&why) {
            self.release();
            self.refused = Some(why);
        }
        (self.all, self.dirty, self.stepped, self.since) = (false, false, false, None);
    }

    fn core(&self, mirrors: &Mirrors) -> Result<Arc<Vision>, String> {
        let (_, vision) = self.perspective().ok_or("the viewing country's vision is not set")?;
        let links = mirrors.get(TableId::CountryLinks).ok_or("CountryLinks is not read yet")?;
        if let Some(why) = links.not_recorded() {
            return Err(format!("{why}; the naval markers need every country link"));
        }
        let combats = mirrors.get(TableId::NavalCombats).ok_or("NavalCombats is not read yet")?;
        if combats.status().is_none() {
            return Err("NavalCombats is not recorded here: a gap in the recording".into());
        }
        Ok(vision)
    }

    fn kept_of(&self, mirrors: &Mirrors, vision: &Vision, combats: &[u64]) -> BTreeMap<u64, Kept> {
        let head = mirrors.get(TableId::NavalCombats).and_then(Mirror::head);
        let found = match (&self.index, head) {
            (Some(index), Some(head)) if !combats.is_empty() => index.history.lookup_icons(NAVAL_ICON, index.branch, head, vision.ours(), combats).ok(),
            _ => None,
        };
        match found {
            Some(found) => found,
            None => combats.iter().map(|&combat| (combat, Kept::Unindexed)).collect(),
        }
    }

    fn take_kept(&mut self, mirrors: &Mirrors, vision: &Vision) {
        let index = self.index.as_ref().map(|index| (&index.history, index.branch));
        let now = KeptAt::of(index, mirrors.get(TableId::NavalCombats).and_then(Mirror::head), vision.ours());
        let advanced = self.kept_at.advance(now, index, NAVAL_ICON, vision.ours());
        self.kept_at = now;
        let mut owed = std::mem::take(&mut self.kept_owed);
        let mut combats: Vec<u64> = match advanced {
            Some(changed) => owed.take().into_iter().chain(changed).collect(),
            None => {
                self.kept.clear();
                owed.clear();
                let rows = held(mirrors.get(TableId::NavalCombats));
                self.dirty = true;
                rows.map(|rows| rows.rows().filter_map(|slot| rows.key(slot)).map(|key| key as u64).collect()).unwrap_or_default()
            }
        };
        self.kept_owed = owed;
        combats.sort_unstable();
        combats.dedup();
        let rows = mirrors.get(TableId::NavalCombats);
        let live = |combat: u64| held(rows).is_some_and(|rows| rows.slot(u128::from(combat)).is_some());
        let (asked, gone): (Vec<u64>, Vec<u64>) = combats.into_iter().partition(|&combat| live(combat));
        let found = self.kept_of(mirrors, vision, &asked);
        self.frames_read += asked.len();
        for combat in gone {
            self.dirty |= self.kept.remove(&combat).is_some();
        }
        for combat in asked {
            let was = self.kept.get(&combat).copied();
            let now = found.get(&combat).copied();
            match now {
                Some(kept) => self.kept.insert(combat, kept),
                None => self.kept.remove(&combat),
            };
            self.dirty |= was != now;
        }
    }

    fn make(&mut self, mirrors: &Mirrors, vision: &Vision, flags: &BTreeMap<u64, Engaged>, provenance: Provenance, kept: &BTreeMap<u64, Kept>) {
        let columns = self.columns();
        let get = |table| held(mirrors.get(table));
        let combats_mirror = get(TableId::NavalCombats);
        let combats: Vec<Combat> = combats_mirror
            .map(|combats| sorted(combats).into_iter().map(|(key, slot)| Combat { id: key as u64, home: combats.number(columns.home, slot).value().unwrap_or(0), registry_ordinal: combats.number(columns.registry_ordinal, slot).value().unwrap_or(0) }).collect())
            .unwrap_or_default();
        let vectors = get(TableId::NavalProvinceCombats);
        let direct: Vec<Direct> = vectors
            .map(|vectors| {
                sorted(vectors)
                    .into_iter()
                    .map(|(key, slot)| {
                        let entries = u64s(bytes_of(vectors.list(columns.vector, slot)));
                        let kinds = vectors.text(columns.kinds, slot).value().and_then(|id| vectors.string(id)).unwrap_or(&[]);
                        Direct { province: key as u32, combats: entries.iter().zip(kinds).filter(|(_, kind)| **kind == NavalProvinceCombatRow::NAVAL).map(|(combat, _)| *combat).collect() }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let details: BTreeMap<u64, DetailFacts> = get(TableId::NavalCombatDetails)
            .map(|details| {
                details
                    .rows()
                    .filter_map(|slot| {
                        let sides = opaque::<[NavalCombatSideDetails; 2]>(details, columns.detail_sides, slot)?;
                        Some((details.key(slot)? as u64, DetailFacts { countries: [sides[0].summary.countries.clone(), sides[1].summary.countries.clone()] }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let transfers: Option<Vec<Transfers>> = whole(mirrors.get(TableId::NavalCombatTransfers)).map(|rows| sorted(rows).into_iter().map(|(key, slot)| Transfers { combat: key as u64, transfers: u64s(bytes_of(rows.list(columns.transfers, slot))) }).collect());
        let units: Option<Vec<Units>> = whole(mirrors.get(TableId::NavalTransferUnits)).map(|rows| sorted(rows).into_iter().map(|(key, slot)| Units { transfer: key as u64, units: u64s(bytes_of(rows.list(columns.units, slot))) }).collect());
        let mut watched = HashSet::new();
        let empty_census = combats_mirror.is_some_and(Mirror::complete) && combats.is_empty();
        let locations: Option<BTreeMap<u64, u32>> = (!empty_census)
            .then(|| {
                let (divisions, forces, guns) = (whole(mirrors.get(TableId::Divisions))?, whole(mirrors.get(TableId::TaskForces))?, whole(mirrors.get(TableId::RailwayGunCounterInputs))?);
                let province = |mirror: &Mirror, column: Num<u32>, unit: u64| mirror.slot(u128::from(unit)).and_then(|slot| mirror.number(column, slot).value());
                Some(
                    units
                        .iter()
                        .flatten()
                        .flat_map(|row| &row.units)
                        .filter_map(|&unit| {
                            let at = province(guns, columns.gun_province, unit).or_else(|| province(forces, columns.force_province, unit)).or_else(|| province(divisions, columns.division_province, unit))?;
                            Some((unit, at))
                        })
                        .collect(),
                )
            })
            .flatten();
        watched.extend(units.iter().flatten().flat_map(|row| row.units.iter().copied()));
        let members_mirror = mirrors.get(TableId::NavalCombatShips);
        let census: Vec<CensusMember> = whole(members_mirror)
            .map(|members| {
                sorted(members)
                    .into_iter()
                    .map(|(key, slot)| CensusMember {
                        combat: (key >> 33) as u64,
                        side: ((key >> 32) & 1) as u8,
                        ship: members.number(columns.ship, slot).value().unwrap_or(0),
                        member_source: members.number(columns.member_source, slot).value().unwrap_or(0),
                        group_index: members.number(columns.group_index, slot).value().unwrap_or(0),
                        live_convoy: members.number(columns.live_convoy, slot).value().unwrap_or(0),
                        live_ship: members.number(columns.live_ship, slot).value().unwrap_or(0),
                        formation_flags: members.number(columns.formation_flags, slot).value().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (ships, forces, inputs) = (mirrors.get(TableId::Ships), mirrors.get(TableId::TaskForces), mirrors.get(TableId::TaskForceCounterInputs));
        let ship_force = |ship: u64| {
            let ships = held(ships)?;
            ships.number(columns.ship_force, ships.slot(u128::from(ship))?).value()
        };
        let force = |id: u64| -> Result<(u32, i32), String> {
            let missing = |table: &str| format!("the {table} table has no row for task force {id}");
            let (forces, inputs) = (held(forces), held(inputs));
            let at = forces.and_then(|forces| forces.slot(u128::from(id))).ok_or_else(|| missing("TaskForces"))?;
            let input = inputs.and_then(|inputs| inputs.slot(u128::from(id))).ok_or_else(|| missing("TaskForceCounterInputs"))?;
            Ok((forces.and_then(|forces| forces.number(columns.roster_owner, at).value()).unwrap_or(0), inputs.and_then(|inputs| inputs.number(columns.headquarters, input).value()).unwrap_or(0)))
        };
        for member in &census {
            watched.insert(member.ship);
            if let Some(force) = ship_force(member.ship) {
                watched.insert(force);
            }
        }
        let hq_whole = whole_of(held_as(ships), "Ships").or_else(|| whole_of(held_as(forces), "TaskForces")).or_else(|| whole_of(held_as(inputs), "TaskForceCounterInputs"));
        let links = held(mirrors.get(TableId::CountryLinks));
        let owner = |raw: u32| {
            let links = links?;
            links.number(columns.link_slot, links.slot(u128::from(raw))?).value()
        };
        let results_mirror = get(TableId::NavalResults);
        let results: Vec<ResultFields> = results_mirror
            .map(|results| {
                sorted(results)
                    .into_iter()
                    .map(|(key, slot)| {
                        let byte = |column: Num<u8>| results.number(column, slot).value().unwrap_or(0);
                        ResultFields {
                            id: key as u64,
                            province: results.number(columns.result_province, slot).value().unwrap_or(0),
                            province_ordinal: results.number(columns.result_ordinal, slot).value().unwrap_or(0),
                            flag_a: byte(columns.flag_a),
                            flag_b: byte(columns.flag_b),
                            subtype: byte(columns.subtype),
                            readiness: byte(columns.readiness),
                            winning_side: byte(columns.winning_side),
                            losing_side: byte(columns.losing_side),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let countries = result_country_lists(get(TableId::NavalResultCountries).into_iter().flat_map(|rows| {
            sorted(rows).into_iter().map(move |(key, slot)| ((key >> 40) as u64, (key >> 32) as u8, rows.number(columns.country_ordinal, slot).value().unwrap_or(0), key as u32))
        }));
        let present: BTreeSet<u64> = get(TableId::NavalResultMembers)
            .map(|members| members.rows().filter(|&slot| members.number(columns.presence, slot).value().unwrap_or(0) != 0).filter_map(|slot| members.key(slot)).map(|key| (key >> 64) as u64).collect())
            .unwrap_or_default();
        let results_complete = [TableId::NavalResults, TableId::NavalResultMembers, TableId::NavalResultCountries].into_iter().all(|table| whole(mirrors.get(table)).is_some());
        let flag = |id: u64| flags.get(&id).copied().unwrap_or(Engaged::Unknown);
        let kept_of = |id: u64| kept.get(&id).copied();
        let shore = Arc::clone(&self.shore);
        let province_kind = |province: u32| shore.province_kind(province);
        let anchor = |province: u32| shore.anchor(province, STANDSTILL);
        let values = naval_marker_values(&NavalMarkerInputs {
            vision,
            owner: &owner,
            combats: &combats,
            combats_held: held_as(mirrors.get(TableId::NavalCombats)),
            direct: &direct,
            direct_complete: whole(mirrors.get(TableId::NavalProvinceCombats)).is_some(),
            details: &details,
            details_held: held_as(mirrors.get(TableId::NavalCombatDetails)),
            transfers: transfers.as_deref(),
            units: units.as_deref(),
            locations: locations.as_ref(),
            members: match whole_of(held_as(members_mirror), "NavalCombatShips") {
                Some(cause) => Err(cause),
                None => Ok(&census),
            },
            hq: match hq_whole {
                Some(cause) => Err(cause),
                None => Ok(HqForces { ship_force: &ship_force, force: &force }),
            },
            flag: &flag,
            provenance: provenance.limitation(),
            kept: &kept_of,
            province_kind: &province_kind,
            anchor: &anchor,
            results: &results,
            result_countries: &countries,
            present: &present,
            results_complete,
        });
        self.values = Some(Arc::new(values));
        self.watched = watched;
        self.read = combats.len();
        self.made += 1;
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors, upstream: Option<(&Engagements, bool)>) {
        self.frame += 1;
        if self.hidden {
            self.flags = None;
            self.refuse("not drawn at this view".to_owned());
            return;
        }
        if let Some((engagements, changed)) = upstream
            && (changed || self.flags.is_none() || self.all)
        {
            let (_, naval, provenance) = engagements.flags();
            self.flags = Some((naval.clone(), provenance[NAVAL]));
            self.dirty = true;
        }
        let vision = match if self.flags.is_none() { Err("the engagement layer is not drawn before it".to_owned()) } else { self.core(mirrors) } {
            Ok(vision) => vision,
            Err(why) => {
                self.refuse(why);
                return;
            }
        };
        if self.refused.take().is_some() || std::mem::take(&mut self.all) {
            self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
            self.dirty = true;
        }
        if self.vision_new.swap(false, Ordering::AcqRel) {
            self.dirty = true;
        }
        if self.dirty || std::mem::take(&mut self.stepped) || self.kept_owed.len() > 0 {
            self.take_kept(mirrors, &vision);
        }
        if self.dirty {
            let (flags, provenance) = self.flags.take().expect("checked above");
            let kept = std::mem::take(&mut self.kept);
            self.make(mirrors, &vision, &flags, provenance, &kept);
            (self.kept, self.flags) = (kept, Some((flags, provenance)));
            self.dirty = false;
            self.since = None;
        }
    }

    fn reads(&self, table: TableId, column: usize) -> bool {
        self.reads.get(&table.index()).is_some_and(|columns| columns.binary_search(&column).is_ok())
    }

    fn apply_table(&mut self, table: TableId, changes: &Changes, mirrors: &Mirrors) {
        if NAVAL_TABLES.contains(&table) {
            let rows = !changes.deleted().is_empty() || !changes.inserted().is_empty();
            self.dirty |= rows || changes.columns().iter().any(|written| !written.is_empty() && self.reads(table, written.column()));
            if table == TableId::NavalCombats {
                let frame = self.frame;
                for &key in changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()) {
                    self.kept_owed.mark(key as u64, frame);
                }
            }
            return;
        }
        if !WATCHED.contains(&table) {
            return;
        }
        let Some(mirror) = mirrors.get(table) else { return };
        let mut keys = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).map(|&key| key as u64);
        if keys.any(|key| self.watched.contains(&key)) {
            self.dirty = true;
            return;
        }
        if changes.columns().iter().flat_map(|written| written.slots()).filter_map(|&slot| mirror.key(slot)).any(|key| self.watched.contains(&(key as u64))) {
            self.dirty = true;
        }
    }
}

impl Layer for NavalMarkers {
    fn held_viewer(&self) -> Option<u32> {
        self.perspective().map(|(_, vision)| vision.ours())
    }

    fn name(&self) -> &'static str {
        "naval_markers"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if !view.overlay.naval_markers {
            self.hidden = true;
            return Ok(());
        }
        inputs.after("engagements");
        for table in TABLES {
            inputs.table(table, true)?;
        }
        use TableId as T;
        self.columns = Some(Columns {
            home: inputs.number(T::NavalCombats, "province")?,
            registry_ordinal: inputs.number(T::NavalCombats, "registry_ordinal")?,
            vector: inputs.list(T::NavalProvinceCombats, "combats")?,
            kinds: inputs.text(T::NavalProvinceCombats, "kinds")?,
            detail_sides: inputs.list(T::NavalCombatDetails, "sides")?,
            transfers: inputs.list(T::NavalCombatTransfers, "transfers")?,
            units: inputs.list(T::NavalTransferUnits, "units")?,
            ship: inputs.number(T::NavalCombatShips, "ship")?,
            member_source: inputs.number(T::NavalCombatShips, "member_source")?,
            group_index: inputs.number(T::NavalCombatShips, "group_index")?,
            live_convoy: inputs.number(T::NavalCombatShips, "live_convoy")?,
            live_ship: inputs.number(T::NavalCombatShips, "live_ship")?,
            formation_flags: inputs.number(T::NavalCombatShips, "formation_flags")?,
            ship_force: inputs.number(T::Ships, "task_force")?,
            roster_owner: inputs.number(T::TaskForces, "roster_owner")?,
            headquarters: inputs.number(T::TaskForceCounterInputs, "attached_headquarters")?,
            division_province: inputs.number(T::Divisions, "province")?,
            force_province: inputs.number(T::TaskForces, "province")?,
            gun_province: inputs.number(T::RailwayGunCounterInputs, "province")?,
            result_province: inputs.number(T::NavalResults, "province")?,
            result_ordinal: inputs.number(T::NavalResults, "province_ordinal")?,
            flag_a: inputs.number(T::NavalResults, "flag_a")?,
            flag_b: inputs.number(T::NavalResults, "flag_b")?,
            subtype: inputs.number(T::NavalResults, "subtype")?,
            readiness: inputs.number(T::NavalResults, "readiness")?,
            winning_side: inputs.number(T::NavalResults, "winning_side")?,
            losing_side: inputs.number(T::NavalResults, "losing_side")?,
            presence: inputs.number(T::NavalResultMembers, "presence_value")?,
            country_ordinal: inputs.number(T::NavalResultCountries, "ordinal")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
        });
        self.reads = inputs.selected();
        if std::mem::take(&mut self.hidden) {
            self.all = true;
        }
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_with(mirrors, &Upstream::new(&[]))
    }

    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        let made = self.made;
        self.all = true;
        self.settle_dirty(mirrors, upstream.get::<Engagements>().map(|layer| (layer, true)));
        self.made = made;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() || self.hidden || self.refused.is_some() {
            return;
        }
        self.stepped = true;
        for changes in step.tables() {
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.dirty = true;
                continue;
            }
            self.apply_table(TableId::ALL[changes.table()], changes, mirrors);
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.settle_with(mirrors, &Upstream::new(&[]), settling)
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, _settling: Settling) -> io::Result<()> {
        let naval = match upstream.changed::<Engagements>() {
            Change::Whole => true,
            Change::Marked(changed) => !changed.of("naval").is_empty(),
        };
        self.settle_dirty(mirrors, upstream.get::<Engagements>().map(|layer| (layer, naval)));
        Ok(())
    }


    fn carried(&self) -> Carried {
        let remake = self.dirty || self.all;
        let entities = if remake { self.read.max(self.kept.len()).max(1) } else { 0 };
        Carried { entities: entities + self.kept_owed.len() }
    }

    fn held(&self) -> usize {
        let values = self.values.as_ref().map_or(0, |values| {
            values.combats.as_ref().map_or(0, |combats| combats.icons.len() * 160 + combats.placement.iter().map(|(cause, combats)| cause.capacity() + combats.capacity() * 8).sum::<usize>())
                + values.results.facts.len() * 48
                + values.results.provinces.values().map(|results| results.capacity() * 8 + 40).sum::<usize>()
                + (values.live.len() + values.gated.len()) * 24
        });
        values + self.watched.capacity() * 16 + self.kept.len() * 32 + self.kept_owed.bytes() + self.flags.as_ref().map_or(0, |(flags, _)| flags.len() * 32)
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
