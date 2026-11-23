
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, at_hour::{At, AtHour, Owner, Read}};
use dashboard_world::products::place_names::{self, Places};
use dashboard_base::{Source, catalogue::{self, Catalogue, Refusals, View}};
use dashboard_follow::live::{Cells, Source as Cell};
use content_reader_script::sub_units::{SubUnit, SubUnits};
use content_reader_division_names::division_names::Database;
use replay_data::rows::{ShipModelRow, ShipNamingOperands, ShipRow};
use dashboard_text::variant_names::{self, Tags, Texts};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, DecodedSource};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalYardsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct YardShip {
    pub handle: String,
    pub name: Option<String>,
    pub name_rebuilt: Option<String>,
    pub design: Option<String>,
    pub class: Option<String>,
    pub hull: Option<String>,
    pub maximum: Option<String>,
    pub task_force: Option<String>,
    pub policy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RepairPort {
    pub province: u32,
    pub name: Option<String>,
    pub priority: Option<i32>,
    #[doc = " The game's name of the port's repair priority."]
    pub policy: Option<String>,
    pub dockyards: Option<i32>,
    #[doc = " The ships queued, in queue order; a ship queued twice is listed twice."]
    pub queue: Vec<YardShip>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalRepairsReply {
    pub ports: Vec<RepairPort>,
    pub used_dockyards: Option<i32>,
    pub repair_limit: Option<i32>,
    #[doc = " Distinct ships queued, where every one is recorded."]
    pub queued_ships: Option<u32>,
    #[doc = " Whether every base's holder and the whole queue were read: only then is no port no repair."]
    pub census_complete: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RefitLine {
    pub handle: String,
    pub target: Option<String>,
    #[doc = " Work done at the reply's hour, raw fixed5 IC; hourly, so also the live cell `(handle, progress)`."]
    pub progress: String,
    #[doc = " The line's cost, raw fixed5 IC, from its daily metadata."]
    pub cost: Option<String>,
    pub dockyards: Option<i32>,
    #[doc = " Cost less work done, where both are finite and the cost positive."]
    pub remaining: Option<String>,
    pub ships: Vec<YardShip>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalRefitsReply {
    pub lines: Vec<RefitLine>,
    pub ships: Option<u32>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub dockyards: Option<i64>,
    #[doc = " The hour the daily line metadata (cost, target, dockyards) was read."]
    pub cost_read: Option<i32>,
    pub census_complete: bool,
    #[doc = " Why the refit lines are not known at the hour, when they are not."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalRepairsReply, NavalRefitsReply);

pub struct Context {
    places: Arc<place_names::Content>,
    cat: Arc<Catalogue>,
}

fn context(source: &Source, load: &LoadContext) -> io::Result<Context> {
    let places = place_names::Content::read(source, load)?;
    Ok(Context { cat: Arc::clone(&places.cat), places })
}

fn validate(params: &NavalYardsParams) -> Result<(), &'static str> {
    if params.branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    validate_country(&params.country)
}

fn raws(held: &DecodedFrame, slot: u32) -> BTreeSet<u32> {
    held.complete::<tables::CountryLinks>().map(|rows| rows.iter().filter(|row| row.canonical_slot == slot).map(|row| row.id).collect()).unwrap_or_default()
}

fn ship_reads(ships: &BTreeSet<u64>) -> Vec<Read> {
    vec![Read::these(TableId::Ships, ships.iter().map(|&id| u128::from(id))), Read::these(TableId::ShipModels, ships.iter().map(|&id| u128::from(id)))]
}

fn named_reads(held: &DecodedFrame, more_variants: impl IntoIterator<Item = u64>) -> Vec<Read> {
    let ships = held.complete::<tables::Ships>().unwrap_or_default();
    let tasks: BTreeSet<u128> = ships.iter().map(|row| u128::from(row.task_force)).collect();
    let mut variants: BTreeSet<u128> = held.complete::<tables::ShipModels>().unwrap_or_default().iter().map(|row| u128::from(row.variant)).collect();
    variants.extend(ships.iter().filter_map(|row| row.naming.require("naming of ships").ok().cloned().flatten()?.equipment).map(u128::from));
    variants.extend(more_variants.into_iter().map(u128::from));
    variants.remove(&0);
    vec![
        Read::these(TableId::TaskForceCounterInputs, tasks.iter().copied()),
        Read::these(TableId::NavalRepairTaskForces, tasks),
        Read::these(TableId::VariantDefinitions, variants),
    ]
}

fn token_reads(held: &DecodedFrame) -> Vec<Read> {
    let mut tokens: BTreeSet<u128> = held.complete::<tables::VariantDefinitions>().unwrap_or_default().iter().map(|row| u128::from(row.equipment_token)).collect();
    tokens.extend(held.complete::<tables::Ships>().unwrap_or_default().iter().map(|row| u128::from(row.class_id as u32)));
    vec![Read::these(TableId::LexicalTokens, tokens)]
}

struct ShipNameGroups(Database);

fn sub_units(cat: &Catalogue) -> Result<Arc<SubUnits>, String> {
    cat.content
        .layers
        .derived::<catalogue::CachedResult<SubUnits>>()
        .get_or_init(|| content_reader_key_names::unit_leader::sub_units(&cat.content.layers))
        .map_err(|error| format!("Ship class content is unavailable: {error}"))
}

fn name_groups(cat: &Catalogue) -> Result<Arc<ShipNameGroups>, String> {
    cat.content
        .layers
        .derived::<catalogue::CachedResult<ShipNameGroups>>()
        .get_or_init(|| content_reader_division_names::division_names::read::<content_reader_parser::Diagnostic>(&cat.content.layers).map(|(database, _)| ShipNameGroups(database)))
        .map_err(|error| format!("Ship name lists are unavailable: {error}"))
}

fn naming_variant(naming: &ShipNamingOperands, model: Option<&ShipModelRow>) -> Option<u64> {
    naming.equipment.or_else(|| model.map(|model| model.variant)).filter(|&handle| handle != 0)
}

fn override_applies(naming: &ShipNamingOperands, group: &Result<Option<Vec<u8>>, String>) -> bool {
    !naming.override_name.is_empty() && (naming.programmatic == 0 || matches!(group, Ok(None)))
}

fn rebuilt_name(cat: &Catalogue, groups: impl FnOnce() -> Result<Arc<ShipNameGroups>, String>, naming: &ShipNamingOperands, group: &Result<Option<Vec<u8>>, String>) -> Result<String, String> {
    if override_applies(naming, group) {
        return dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, &naming.override_name, "Ship name override").and_then(|text| catalogue::plain_text(&text));
    }
    if naming.kind != 1 {
        return Err(format!("Ship naming kind {} is not rebuilt", naming.kind));
    }
    let key = group.as_ref().map_err(Clone::clone)?.as_ref().ok_or("Ship name has no group or usable override")?;
    groups()?.0.resolve_any(key, naming.order, naming.ordered != 0).map_err(|why| format!("Ship name group: {why}"))
}

fn policy_key(port: bool, value: i32) -> String {
    format!("NAVY_{}_POLICY_{value}", if port { "DOCKYARD_REPAIR" } else { "REPAIR" })
}

struct Yard<'f> {
    frame: &'f DecodedFrame,
    cat: &'f Catalogue,
    tags: Option<Tags<'f>>,
    notes: Refusals,
    live: &'f Cells,
    units: Option<Result<Arc<SubUnits>, String>>,
    groups: Option<Result<Arc<ShipNameGroups>, String>>,
}

impl<'f> Yard<'f> {
    fn new(frame: &'f DecodedFrame, cat: &'f Catalogue, live: &'f Cells) -> Self {
        let tags = match (frame.complete::<tables::Countries>(), frame.complete::<tables::CountryLinks>()) {
            (Ok(countries), Ok(links)) => Some(Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok())),
            _ => None,
        };
        Self { frame, cat, tags, notes: Refusals::default(), live, units: None, groups: None }
    }

    fn policy(&mut self, port: bool, value: i32) -> Option<String> {
        let key = policy_key(port, value);
        let name = self.cat.localised(&key, View::Plain).filter(|_| self.cat.has_presentation_key(&key)).and_then(|text| catalogue::plain_text(&text).ok());
        if name.is_none() {
            self.notes.note("Repair policy name is not localised", key);
        }
        name
    }

    fn class(&mut self, ship: &ShipRow, model: Option<&ShipModelRow>) -> Option<String> {
        let cat = self.cat;
        let units = match self.units.get_or_insert_with(|| sub_units(cat)) {
            Ok(units) => Arc::clone(units),
            Err(reason) => {
                let reason = reason.clone();
                self.notes.note(reason, ship.id.to_string());
                return None;
            }
        };
        let index = model.and_then(|model| model.subunit_index.require("subunit index of ship models").ok().copied().flatten());
        let frame = self.frame;
        let named = (|| -> Result<String, String> {
            let index = match index {
                Some(index) => u32::try_from(index).map_err(|_| "Ship class definition index is invalid")?,
                None => {
                    let token = ship.class_id as u32;
                    let lexical = frame.complete::<tables::LexicalTokens>().map_err(|error| error.to_string())?;
                    let spelled = lexical.iter().find(|row| row.token == token).ok_or("Ship class token is not recorded")?;
                    let spelling = std::str::from_utf8(&spelled.literal).map_err(|_| "Ship class token is not UTF-8")?;
                    match units.get(spelling) {
                        SubUnit::Index(index) => index,
                        _ => return Err(format!("Ship class {spelling} is not in the recorded content")),
                    }
                }
            };
            let registered = units.name(index).ok_or("Ship class definition is absent from the recorded content")??.to_owned();
            cat.localised(&registered, View::Plain)
                .filter(|_| cat.has_presentation_key(&registered))
                .ok_or_else(|| format!("Ship class {registered} is not localised"))
                .and_then(|text| catalogue::plain_text(&text))
        })();
        named.map_err(|reason| self.notes.note(reason, ship.id.to_string())).ok()
    }

    fn rebuilt(&mut self, ship: &ShipRow, model: Option<&ShipModelRow>) -> Option<String> {
        let naming = match ship.naming.require("naming of ships") {
            Ok(Some(naming)) => naming,
            Ok(None) | Err(_) => {
                self.notes.note("Ship name is not recorded: neither the game's name nor its naming", ship.id.to_string());
                return None;
            }
        };
        let frame = self.frame;
        let group = (|| -> Result<Option<Vec<u8>>, String> {
            let variant = naming_variant(naming, model).ok_or("Ship naming names no design")?;
            let definitions = frame.complete::<tables::VariantDefinitions>().map_err(|error| error.to_string())?;
            let definition = definitions.iter().find(|row| row.id == variant).ok_or_else(|| format!("Ship naming design {variant} is not recorded"))?;
            Ok(definition.design.require("design of variant definitions")?.division_names_group.clone())
        })();
        let cat = self.cat;
        let held = &mut self.groups;
        let groups = || held.get_or_insert_with(|| name_groups(cat)).clone();
        let rebuilt = rebuilt_name(cat, groups, naming, &group);
        rebuilt.map_err(|reason| self.notes.note(reason, ship.id.to_string())).ok()
    }

    fn stored(&mut self, bytes: &[u8], what: &str, identity: impl Into<String>) -> Option<String> {
        match dashboard_text::scripted_text::stored_text(&self.cat.localisation, &self.cat.recorded_language, bytes, what).and_then(|text| catalogue::plain_text(&text)) {
            Ok(text) => Some(text),
            Err(reason) => {
                self.notes.note(reason, identity);
                None
            }
        }
    }

    fn design(&mut self, variant: u64) -> Option<String> {
        if variant == 0 {
            return None;
        }
        let named = (|| -> Result<Option<String>, String> {
            let definition = self.frame.complete::<tables::VariantDefinitions>().map_err(|error| error.to_string())?.iter().find(|row| row.id == variant).ok_or("Ship design is not recorded")?;
            let tags = self.tags.as_ref().ok_or("Countries are not recorded at this hour: designs cannot be named")?;
            let lexical = self.frame.complete::<tables::LexicalTokens>().map_err(|error| error.to_string())?;
            let raw = variant_names::raw_variant(definition, self.frame.hour, tags, lexical);
            variant_names::variant_name(&raw, Some(self.cat as &dyn Texts))
        })();
        match named {
            Ok(Some(name)) => Some(name),
            Ok(None) => {
                self.notes.note("Ship design name is unavailable", variant.to_string());
                None
            }
            Err(reason) => {
                self.notes.note(reason, variant.to_string());
                None
            }
        }
    }

    fn ship(&mut self, handle: u64) -> YardShip {
        let frame = self.frame;
        let row = frame.complete::<tables::Ships>().ok().and_then(|rows| rows.iter().find(|row| row.id == handle));
        let model = frame.complete::<tables::ShipModels>().ok().and_then(|rows| rows.iter().find(|row| row.ship == handle));
        if row.is_none() {
            self.notes.note("Queued ship is not recorded", handle.to_string());
        }
        let name = row.and_then(|row| row.name.require("name of ships").ok().cloned().flatten()).and_then(|bytes| self.stored(&bytes, "Ship name", handle.to_string()));
        let task_force = row.and_then(|row| {
            let counter = frame.complete::<tables::TaskForceCounterInputs>().ok()?.iter().find(|counter| counter.task_force == row.task_force)?;
            counter.name.require("name of task force counter inputs").ok()?.clone()
        });
        let task_force = task_force.and_then(|bytes| self.stored(&bytes, "Task force name", handle.to_string()));
        let maximum = model.and_then(|model| model.counter.as_ref()).map(|counter| counter.maximum_strength.max(100));
        if row.is_some() && maximum.is_none() {
            self.notes.note("Maximum ship hull not recorded", handle.to_string());
        }
        let name_rebuilt = match row {
            Some(row) if name.is_none() => self.rebuilt(row, model),
            _ => None,
        };
        let class = row.and_then(|row| self.class(row, model));
        let order = row.and_then(|row| frame.complete::<tables::NavalRepairTaskForces>().ok()?.iter().find(|order| order.task_force == row.task_force).cloned());
        let policy = order.and_then(|order| self.policy(false, order.repair_mode));
        if let Ok(column) = dashboard_declare::field(TableId::Ships, "strength") {
            self.live.value(&handle.to_string(), "hull", Cell { table: TableId::Ships, column, key: u128::from(handle) }, 1.0);
        }
        YardShip {
            handle: handle.to_string(),
            name,
            name_rebuilt,
            design: model.and_then(|model| self.design(model.variant)),
            class,
            hull: row.map(|row| row.strength.to_string()),
            maximum: maximum.map(|value| value.to_string()),
            task_force,
            policy,
        }
    }
}

fn unrecorded_text(frame: &DecodedFrame, table: TableId, title: &str) -> String {
    format!("{title} are not recorded at this hour: {}", dashboard_text::scope::missing(table, &frame.tables[table.index()]))
}

fn unrecorded(frame: &DecodedFrame, table: TableId, title: &str) -> io::Error {
    dashboard_core::refused(unrecorded_text(frame, table, title))
}

fn refits_unrecorded(frame: &DecodedFrame) -> Option<NavalRefitsReply> {
    frame.complete::<tables::NavalRefitProgress>().err().map(|_| NavalRefitsReply {
        lines: Vec::new(),
        ships: None,
        dockyards: None,
        cost_read: None,
        census_complete: false,
        missing: Some(unrecorded_text(frame, TableId::NavalRefitProgress, "Refit lines")),
        warnings: Vec::new(),
    })
}

fn complete(frame: &DecodedFrame, table: TableId) -> bool {
    (frame as &dyn DecodedSource).complete_status(table).is_ok()
}

pub struct NavalRepairs;

fn owned_ports(held: &DecodedFrame, raws: &BTreeSet<u32>) -> BTreeSet<u32> {
    held.complete::<tables::NavalBases>().map(|rows| rows.iter().filter(|row| raws.contains(&row.country)).map(|row| row.province).collect()).unwrap_or_default()
}

fn queues(frame: &DecodedFrame, ports: &BTreeSet<u32>) -> BTreeMap<u32, Vec<(u32, u64)>> {
    let mut out: BTreeMap<u32, Vec<(u32, u64)>> = BTreeMap::new();
    for row in frame.complete::<tables::NavalRepairQueue>().unwrap_or_default().iter().filter(|row| ports.contains(&row.province)) {
        out.entry(row.province).or_default().push((row.ordinal, row.ship));
    }
    out
}

const REPAIR_TABLES: [TableId; 17] = [
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalRepairCountries,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForceCounterInputs,
    TableId::NavalRepairTaskForces,
    TableId::VariantDefinitions,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
];

impl AtHour for NavalRepairs {
    const KEY: &'static str = "naval-repairs";
    const TITLE: &'static str = "Naval repairs";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalYardsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = NavalRepairsReply;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(params)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalYardsParams::decl(), YardShip::decl(), RepairPort::decl(), NavalRepairsReply::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        REPAIR_TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => [TableId::NavalBases, TableId::CountryLinks, TableId::Countries, TableId::CountryAppearance, TableId::ReservedCountryNaming, TableId::NavalRepairQueue].map(Read::all).into(),
            1 => {
                let raws = raws(held, owner.slot);
                let ports = owned_ports(held, &raws);
                vec![
                    Read::these(TableId::NavalRepairBases, ports.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::NavalRepairCountries, raws.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::Provinces, ports.iter().map(|&id| u128::from(id))),
                    Read::these(TableId::ProvinceNames, ports.iter().map(|&id| u128::from(id))),
                ]
            }
            2 => {
                let ports = owned_ports(held, &raws(held, owner.slot));
                let ships: BTreeSet<u64> = queues(held, &ports).values().flatten().map(|&(_, ship)| ship).collect();
                let mut reads = ship_reads(&ships);
                reads.extend(place_names::reads(&[], &[], 1, held));
                reads
            }
            3 => named_reads(held, []),
            4 => token_reads(held),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        context(source, load)
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Naval repairs country is unavailable at this hour"))?;
        if !complete(frame, TableId::NavalBases) {
            return Err(unrecorded(frame, TableId::NavalBases, "Repair ports"));
        }
        let raws = raws(frame, owner.slot);
        let ports = owned_ports(frame, &raws);
        let queues = queues(frame, &ports);
        let places = Places::new(frame);
        let mut yard = Yard::new(frame, &at.context.cat, at.live);
        let bases = frame.complete::<tables::NavalRepairBases>().unwrap_or_default();
        let mut shown = Vec::new();
        for &province in &ports {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
            let repair = bases.iter().find(|row| row.province == province);
            let queue = queues.get(&province);
            if repair.is_none() && queue.is_none() {
                continue;
            }
            let name = places.province(&at.context.places, province, false);
            if let Some(reason) = &name.missing {
                yard.notes.note(reason.clone(), format!("port {province}"));
            }
            shown.push(RepairPort {
                province,
                name: name.name,
                priority: repair.map(|row| row.priority),
                policy: repair.and_then(|row| yard.policy(true, row.priority)),
                dockyards: repair.map(|row| row.allocated_dockyards),
                queue: queue.into_iter().flatten().map(|&(_, ship)| yard.ship(ship)).collect(),
            });
        }
        shown.sort_by(|a, b| b.priority.unwrap_or(i32::MIN).cmp(&a.priority.unwrap_or(i32::MIN)).then(a.province.cmp(&b.province)));
        let unowned = frame.complete::<tables::NavalBases>()?.iter().any(|row| row.country != 0 && !frame.complete::<tables::CountryLinks>().is_ok_and(|links| links.iter().any(|link| link.id == row.country)));
        let census_complete = !unowned && complete(frame, TableId::NavalRepairQueue);
        let country = frame.complete::<tables::NavalRepairCountries>().ok().and_then(|rows| rows.iter().rev().find(|row| raws.contains(&row.country)).copied());
        let queued: BTreeSet<u64> = queues.values().flatten().map(|&(_, ship)| ship).collect();
        let ships_known = frame.complete::<tables::Ships>().is_ok_and(|rows| queued.iter().all(|ship| rows.iter().any(|row| row.id == *ship)));
        let mut warnings = yard.notes.lines("naval repair fields");
        if !census_complete {
            warnings.push("Repair queue or port ownership not recorded; an empty queue cannot be established".into());
        }
        if !queued.is_empty() {
            warnings.push("Repair rate and ETA unavailable: the exact repair rate is not recorded".into());
        }
        warnings.extend(at.context.places.warnings());
        let reply = NavalRepairsReply {
            ports: shown,
            used_dockyards: country.map(|row| row.num_used_dockyards),
            repair_limit: country.map(|row| row.max_allowed_dockyards_to_repair),
            queued_ships: (census_complete && ships_known).then_some(queued.len() as u32),
            census_complete,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub struct NavalRefits;

fn finite(raw: i64) -> bool {
    raw != i64::MAX && raw != i64::MIN && raw != i64::MIN + 1
}

fn owned_lines(held: &DecodedFrame, raws: &BTreeSet<u32>) -> BTreeMap<u64, Vec<u64>> {
    let owned: BTreeSet<u64> = held.complete::<tables::ProductionLines>().map(|rows| rows.iter().filter(|row| raws.contains(&row.country)).map(|row| row.handle).collect()).unwrap_or_default();
    let mut out: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for row in held.complete::<tables::NavalRefitProgress>().unwrap_or_default().iter().filter(|row| owned.contains(&row.line)) {
        out.insert(row.line, Vec::new());
    }
    for link in held.complete::<tables::NavalRefitShips>().unwrap_or_default() {
        if let Some(ships) = out.get_mut(&link.line) {
            ships.push(link.ship);
        }
    }
    out
}

const REFIT_TABLES: [TableId; 12] = [
    TableId::NavalRefitProgress,
    TableId::NavalRefitShips,
    TableId::ProductionLines,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForceCounterInputs,
    TableId::NavalRepairTaskForces,
    TableId::VariantDefinitions,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
];

impl AtHour for NavalRefits {
    const KEY: &'static str = "naval-refits";
    const TITLE: &'static str = "Naval refits";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalYardsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = NavalRefitsReply;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(params)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalYardsParams::decl(), YardShip::decl(), RefitLine::decl(), NavalRefitsReply::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        REFIT_TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        let lines = || owned_lines(held, &raws(held, owner.slot));
        match step {
            0 => [TableId::NavalRefitProgress, TableId::NavalRefitShips, TableId::ProductionLines, TableId::CountryLinks, TableId::Countries, TableId::CountryAppearance].map(Read::all).into(),
            1 => ship_reads(&lines().into_values().flatten().collect()),
            2 => {
                let targets: Vec<u64> = held
                    .complete::<tables::ProductionLines>()
                    .map(|rows| {
                        let owned = lines();
                        rows.iter().filter(|row| owned.contains_key(&row.handle)).map(|row| row.variant).collect()
                    })
                    .unwrap_or_default();
                named_reads(held, targets)
            }
            3 => token_reads(held),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        context(source, load)
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Naval refits country is unavailable at this hour"))?;
        if let Some(reply) = refits_unrecorded(frame) {
            return Ok(reply);
        }
        let progress = frame.complete::<tables::NavalRefitProgress>()?;
        let lines = owned_lines(frame, &raws(frame, owner.slot));
        let metadata: HashMap<u64, &replay_data::rows::ProductionLineRow> = frame.complete::<tables::ProductionLines>().map(|rows| rows.iter().map(|row| (row.handle, row)).collect()).unwrap_or_default();
        let mut yard = Yard::new(frame, &at.context.cat, at.live);
        let progress_column = dashboard_declare::field(TableId::NavalRefitProgress, "progress_raw")?;
        let mut shown = Vec::new();
        let (mut ships, mut dockyards) = (0u32, Some(0i64));
        for (&handle, linked) in &lines {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
            let Some(amount) = progress.iter().find(|row| row.line == handle) else { continue };
            let line = metadata.get(&handle).copied();
            if line.is_none() {
                yard.notes.note("Refit daily target, cost and allocation not recorded", handle.to_string());
            }
            let remaining = line.and_then(|line| {
                let known = line.cost_raw > 0 && finite(line.cost_raw) && finite(amount.progress_raw);
                if !known {
                    yard.notes.note("Refit remaining cost unavailable: nonpositive or nonfinite cost or progress", handle.to_string());
                }
                known.then(|| (i128::from(line.cost_raw) - i128::from(amount.progress_raw)).to_string())
            });
            ships += linked.len() as u32;
            at.live.value(&handle.to_string(), "progress", Cell { table: TableId::NavalRefitProgress, column: progress_column, key: u128::from(handle) }, 1.0);
            dockyards = dockyards.zip(line.map(|line| i64::from(line.active_factories))).map(|(sum, line)| sum + line);
            shown.push(RefitLine {
                handle: handle.to_string(),
                target: line.and_then(|line| yard.design(line.variant)),
                progress: amount.progress_raw.to_string(),
                cost: line.map(|line| line.cost_raw.to_string()),
                dockyards: line.map(|line| line.active_factories),
                remaining,
                ships: linked.iter().map(|&ship| yard.ship(ship)).collect(),
            });
        }
        let census_complete = [TableId::NavalRefitProgress, TableId::NavalRefitShips, TableId::ProductionLines, TableId::CountryLinks].into_iter().all(|table| complete(frame, table));
        let mut warnings = yard.notes.lines("naval refit fields");
        if !census_complete {
            warnings.push("Active refit count unavailable: incomplete hourly progress, links, line metadata or ownership census".into());
        }
        let reply = NavalRefitsReply {
            lines: shown,
            ships: census_complete.then_some(ships),
            dockyards: if census_complete { dockyards } else { None },
            cost_read: frame.tables[TableId::ProductionLines.index()].sampled_at().filter(|&hour| hour <= frame.hour),
            census_complete,
            missing: None,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
