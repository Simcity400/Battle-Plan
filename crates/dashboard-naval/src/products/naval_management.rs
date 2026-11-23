
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read}};
use dashboard_base::{Source, catalogue::{self, Catalogue, View}};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_text::{
    metadata::link_slot,
    variant_names::{self, Tags, Texts},
};
use content_reader_script::sub_units::SubUnits;
use replay_data::rows::{NavalRefitProgressRow, ProductionLineRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalManagementParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalManagementAmount {
    pub row: String,
    pub column: String,
    pub raw: Option<String>,
    pub of: Option<String>,
    pub unit: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalManagementLine {
    pub text: String,
    pub amount: Option<NavalManagementAmount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalManagementSection {
    pub key: String,
    pub title: String,
    pub lines: Vec<NavalManagementLine>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalManagementReply {
    pub sections: Vec<NavalManagementSection>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalManagementReply);

pub struct NavalManagement;

const TABLES: [TableId; 21] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::Fleets,
    TableId::TaskForces,
    TableId::Ships,
    TableId::TaskForceCounterInputs,
    TableId::ShipModels,
    TableId::VariantDefinitions,
    TableId::ProductionLines,
    TableId::NavalBases,
    TableId::NavalRepairCountries,
    TableId::NavalRepairFleets,
    TableId::NavalRepairTaskForces,
    TableId::NavalTaskForceComposition,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::NavalRepairChildren,
];

fn text(text: impl Into<String>) -> NavalManagementLine {
    NavalManagementLine { text: text.into(), amount: None }
}

fn on(value: u8) -> &'static str {
    if value != 0 { "On" } else { "Off" }
}

fn fixed(raw: i64) -> String {
    let raw = i128::from(raw);
    let magnitude = raw.abs();
    format!("{}{}.{:05}", if raw < 0 { "-" } else { "" }, magnitude / 100_000, magnitude % 100_000)
}

fn raws(held: &DecodedFrame, slot: u32) -> BTreeSet<u32> {
    held.complete::<tables::CountryLinks>().map(|rows| rows.iter().filter(|row| row.canonical_slot == slot).map(|row| row.id).collect()).unwrap_or_default()
}

struct Owned {
    tasks: BTreeSet<u64>,
    fleets: BTreeSet<u64>,
    ports: BTreeSet<u32>,
}

fn owned(held: &DecodedFrame, raws: &BTreeSet<u32>) -> Owned {
    Owned {
        tasks: held.complete::<tables::TaskForces>().unwrap_or_default().iter().filter(|row| raws.contains(&row.roster_owner)).map(|row| row.id).collect(),
        fleets: held.complete::<tables::Fleets>().unwrap_or_default().iter().filter(|row| raws.contains(&row.owner)).map(|row| row.id).collect(),
        ports: held.complete::<tables::NavalBases>().unwrap_or_default().iter().filter(|row| raws.contains(&row.country)).map(|row| row.province).collect(),
    }
}

fn ships(held: &DecodedFrame, owned: &Owned) -> BTreeSet<u64> {
    let mut ships: BTreeSet<u64> = held.complete::<tables::Ships>().unwrap_or_default().iter().filter(|row| owned.tasks.contains(&row.task_force)).map(|row| row.id).collect();
    ships.extend(held.complete::<tables::NavalRepairQueue>().unwrap_or_default().iter().filter(|row| owned.ports.contains(&row.province)).map(|row| row.ship));
    ships
}

fn under(table: TableId, owners: &BTreeSet<u64>, shift: u32) -> Read {
    let low = |owner: u64| u128::from(owner) << shift;
    Read { table, keys: Keys::Ranges(owners.iter().map(|&owner| (low(owner), low(owner) | ((1u128 << shift) - 1))).collect()) }
}

fn these<T: Copy + Into<u128>>(table: TableId, keys: impl IntoIterator<Item = T>) -> Read {
    Read::these(table, keys.into_iter().map(Into::into))
}

impl AtHour for NavalManagement {
    const KEY: &'static str = "naval-management";
    const TITLE: &'static str = "Naval management";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalManagementParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = NavalManagementReply;
    type Context = Result<Arc<Catalogue>, String>;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid naval management branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            NavalManagementParams::decl(),
            NavalManagementAmount::decl(),
            NavalManagementLine::decl(),
            NavalManagementSection::decl(),
            NavalManagementReply::decl(),
        ]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        let raws = || raws(held, owner.slot);
        match step {
            0 => [
                TableId::Countries,
                TableId::CountryLinks,
                TableId::CountryAppearance,
                TableId::Fleets,
                TableId::TaskForces,
                TableId::NavalBases,
                TableId::NavalRepairQueue,
                TableId::NavalRefitShips,
                TableId::NavalRefitProgress,
            ]
            .map(Read::all)
            .into(),
            1 => {
                let raws = raws();
                let owned = owned(held, &raws);
                vec![
                    these(TableId::NavalRepairCountries, raws.iter().copied()),
                    these(TableId::NavalRepairFleets, owned.fleets.iter().copied()),
                    these(TableId::NavalRepairTaskForces, owned.tasks.iter().copied()),
                    these(TableId::TaskForceCounterInputs, owned.tasks.iter().copied()),
                    these(TableId::NavalRepairBases, owned.ports.iter().copied()),
                    under(TableId::NavalTaskForceComposition, &owned.tasks, 40),
                    under(TableId::NavalRepairChildren, &owned.tasks, 32),
                    Read::all(TableId::Ships),
                ]
            }
            2 => {
                let ships = ships(held, &owned(held, &raws()));
                let refits = held.complete::<tables::NavalRefitShips>().unwrap_or_default();
                let lines: BTreeSet<u64> = refits.iter().filter(|row| ships.contains(&row.ship)).map(|row| row.line).collect();
                vec![these(TableId::ShipModels, ships.iter().copied()), these(TableId::ProductionLines, lines)]
            }
            3 => {
                let mut variants: BTreeSet<u64> = held.complete::<tables::NavalRepairTaskForces>().unwrap_or_default().iter().flat_map(|row| [row.refit_variant, row.refit_after_repair]).collect();
                variants.extend(held.complete::<tables::ProductionLines>().unwrap_or_default().iter().map(|row| row.variant));
                variants.remove(&0);
                vec![these(TableId::VariantDefinitions, variants)]
            }
            4 => {
                let mut tokens: BTreeSet<u32> = held.complete::<tables::VariantDefinitions>().unwrap_or_default().iter().map(|row| row.equipment_token).collect();
                tokens.extend(held.complete::<tables::NavalRepairTaskForces>().unwrap_or_default().iter().map(|row| row.composition_token));
                vec![these(TableId::LexicalTokens, tokens)]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| format!("Naval content names unavailable: {why}")))
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Naval management country is unavailable at this hour"))?;
        let mut reply = project(frame, owner.slot, at.context.as_ref().ok(), at.live)?;
        if let Err(why) = at.context {
            reply.warnings.push(why.clone());
        }
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn content_units(cat: &Catalogue, warnings: &mut Vec<String>) -> Option<Arc<SubUnits>> {
    match cat.content.layers.derived::<catalogue::CachedResult<SubUnits>>().get_or_init(|| content_reader_key_names::unit_leader::sub_units(&cat.content.layers)) {
        Ok(units) => {
            warnings.extend(units.warnings().iter().cloned());
            Some(units)
        }
        Err(error) => {
            warnings.push(format!("Ship class content is unavailable: {error}"));
            None
        }
    }
}

fn refit_progress<'a>(link: &replay_data::rows::NavalRefitShipRow, progress: Option<&'a [NavalRefitProgressRow]>) -> Option<&'a NavalRefitProgressRow> {
    progress.and_then(|rows| link.resolve_line(rows))
}

fn project(frame: &DecodedFrame, slot: u32, catalogue: Option<&Arc<Catalogue>>, live: &Cells) -> io::Result<NavalManagementReply> {
    let mut reply = NavalManagementReply { sections: Vec::new(), warnings: Vec::new() };
    macro_rules! rows {
        ($table:ident) => {
            match frame.complete::<tables::$table>() {
                Ok(rows) => rows,
                Err(why) => {
                    reply.warnings.push(format!("{}: {why}", stringify!($table)));
                    &[]
                }
            }
        };
    }
    let countries = frame.complete::<tables::Countries>().map_err(io::Error::other)?;
    let links = frame.complete::<tables::CountryLinks>().map_err(io::Error::other)?;
    let own = |raw| link_slot(links, raw) == Some(slot);
    let lexical = rows!(LexicalTokens);
    let variants = rows!(VariantDefinitions);
    let tags = Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok());
    let design = |handle: u64| {
        if handle == 0 {
            return "None".to_owned();
        }
        variants
            .iter()
            .find(|row| row.id == handle)
            .and_then(|row| {
                let raw = variant_names::raw_variant(row, frame.hour, &tags, lexical);
                variant_names::variant_name(&raw, catalogue.map(|cat| cat.as_ref() as &dyn Texts)).ok().flatten()
            })
            .unwrap_or_else(|| "Design name unavailable".into())
    };
    let localise = |key: &str| catalogue.and_then(|cat| cat.localised(key, View::Plain));
    let policy = |value: i32, dockyard: bool| {
        localise(&format!("NAVY_{}_POLICY_{value}", if dockyard { "DOCKYARD_REPAIR" } else { "REPAIR" })).unwrap_or_else(|| "Policy name unavailable".into())
    };
    let subunits = catalogue.and_then(|cat| content_units(cat, &mut reply.warnings));
    let class = |index: i32| {
        if index == -1 {
            return "Any class".to_owned();
        }
        subunits.as_ref().and_then(|units| units.name(index as u32)).and_then(Result::ok).and_then(&localise).unwrap_or_else(|| "Class name unavailable".into())
    };
    let progress_column = dashboard_declare::field(TableId::NavalRefitProgress, "progress_raw")?;
    let hull_column = dashboard_declare::field(TableId::Ships, "strength")?;
    let budgets = rows!(NavalRepairCountries);
    if let Some(row) = budgets.iter().find(|row| own(row.country)) {
        reply.sections.push(NavalManagementSection {
            key: "budgets".into(),
            title: "Repair allocation".into(),
            lines: vec![
                text(format!("Repair dockyards: {} used; limit {}", row.num_used_dockyards, row.max_allowed_dockyards_to_repair)),
                text(format!("Building repair factory limit: {}", row.max_factories_for_repair)),
            ],
        });
    }
    let fleet_settings = rows!(NavalRepairFleets);
    for fleet in rows!(Fleets).iter().filter(|row| own(row.owner)) {
        let mut lines = vec![text(format!("Home base: province {}", fleet.home_base))];
        match fleet_settings.iter().find(|row| row.fleet == fleet.id) {
            Some(setting) => lines.push(text(format!("Automatic home base: {}", on(setting.automated_homebase)))),
            None => lines.push(text("Automatic home base unavailable")),
        }
        let title = fleet
            .name
            .require("name of fleets")
            .ok()
            .and_then(|name| name.as_ref())
            .and_then(|bytes| {
                if bytes.is_empty() {
                    Some(String::new())
                } else {
                    catalogue.and_then(|cat| Texts::stored_text(cat.as_ref(), std::str::from_utf8(bytes).ok()?, "fleet name").ok())
                }
            })
            .unwrap_or_else(|| "Fleet name unavailable".into());
        reply.sections.push(NavalManagementSection { key: format!("fleet:{}", fleet.id), title, lines });
    }
    let settings = rows!(NavalRepairTaskForces);
    let children = rows!(NavalRepairChildren);
    let composition = rows!(NavalTaskForceComposition);
    let names = rows!(TaskForceCounterInputs);
    let ships_known = frame.complete::<tables::Ships>().is_ok();
    let ships = rows!(Ships);
    let models = rows!(ShipModels);
    let work: &[ProductionLineRow] = rows!(ProductionLines);
    let progress_known = frame.complete::<tables::NavalRefitProgress>().is_ok();
    let progress = rows!(NavalRefitProgress);
    let refits = rows!(NavalRefitShips);
    let tasks = rows!(TaskForces);
    for task in tasks.iter().filter(|row| own(row.roster_owner)) {
        let title = names
            .iter()
            .find(|row| row.task_force == task.id)
            .and_then(|row| catalogue.and_then(|cat| dashboard_text::unit_names::task_force_name(row, &cat.localisation, &cat.recorded_language).ok()))
            .unwrap_or_else(|| "Task force name unavailable".into());
        let mut lines = vec![if ships_known {
            text(format!("Current strength: {} ships", ships.iter().filter(|row| row.task_force == task.id).count()))
        } else {
            text("Current ship count unavailable")
        }];
        if let Some(row) = settings.iter().find(|row| row.task_force == task.id) {
            lines.extend([
                text(format!("Repair priority: {}", policy(row.repair_mode, false))),
                text(format!("Split for repair: {}; automatic reinforcement: {}", on(row.repair_split), on(row.auto_reinforcement))),
                text(format!("Refit design: {}; after repair: {}", design(row.refit_variant), design(row.refit_after_repair))),
            ]);
            if row.composition_token != engine_native::naval_management::NO_AI_COMPOSITION_TOKEN {
                let template = lexical
                    .iter()
                    .find(|entry| entry.token == row.composition_token)
                    .and_then(|entry| std::str::from_utf8(&entry.literal).ok())
                    .and_then(&localise)
                    .unwrap_or_else(|| "Template name unavailable".into());
                lines.push(text(format!("AI composition: {template}")));
            }
            if row.repair_target != 0 {
                lines.push(text(format!("Repair target: province {}", row.repair_target)));
            }
            if row.merge_after_repair != 0 && tasks.binary_search_by_key(&row.merge_after_repair, |task| task.id).is_ok() {
                lines.push(text(format!("Rejoin task force {} after repair", row.merge_after_repair >> 32)));
            }
            for child in children.iter().filter(|child| child.task_force == task.id) {
                lines.push(text(format!("Repair detachment {}: task force {}", u64::from(child.ordinal) + 1, child.child >> 32)));
            }
            for entry in composition.iter().filter(|entry| entry.task_force == task.id) {
                lines.push(text(format!(
                    "{}: {} - {} (role preference {}; design preference {})",
                    if entry.kind == 0 { "Intended" } else { "Composition count" },
                    class(entry.sub_unit),
                    entry.amount,
                    entry.role,
                    entry.definition
                )));
            }
        } else {
            lines.push(text("Repair and reinforcement settings unavailable"));
        }
        for ship in ships.iter().filter(|ship| ship.task_force == task.id) {
            let Some(link) = refits.iter().find(|row| row.ship == ship.id) else { continue };
            let amount = refit_progress(link, progress_known.then_some(progress));
            if progress_known && amount.is_none() {
                continue;
            }
            let line = (link.line != 0).then(|| work.iter().find(|row| row.handle == link.line)).flatten();
            lines.push(match amount {
                Some(amount) => {
                    live.value(&amount.line.to_string(), "progress", Cell { table: TableId::NavalRefitProgress, column: progress_column, key: u128::from(amount.line) }, 1.0);
                    NavalManagementLine {
                        text: format!("Ship {} refit progress: ", ship.id >> 32),
                        amount: Some(NavalManagementAmount { row: amount.line.to_string(), column: "progress".into(), raw: Some(amount.progress_raw.to_string()), of: None, unit: "IC".into() }),
                    }
                }
                None => text(format!("Ship {} refit progress unavailable", ship.id >> 32)),
            });
            lines.push(text(format!(
                "Last recorded target: {}; cost: {}; dockyard allocation: {}",
                line.map(|line| design(line.variant)).unwrap_or_else(|| "Unavailable".into()),
                line.map(|line| format!("{} IC", fixed(line.cost_raw))).unwrap_or_else(|| "Unavailable".into()),
                line.map(|line| format!("{} dockyards", line.active_factories)).unwrap_or_else(|| "Unavailable".into())
            )));
        }
        reply.sections.push(NavalManagementSection { key: format!("task:{}", task.id), title, lines });
    }
    let bases = rows!(NavalRepairBases);
    let queues = rows!(NavalRepairQueue);
    let owned: BTreeSet<u32> = rows!(NavalBases).iter().filter(|row| own(row.country)).map(|row| row.province).collect();
    if queues.iter().any(|row| owned.contains(&row.province)) {
        reply.warnings.push("Repair ETA unavailable: the exact repair rate is not recorded.".into());
    }
    for base in bases.iter().filter(|row| owned.contains(&row.province)) {
        let mut lines = vec![text(format!("Repair priority: {}; assigned dockyards: {}", policy(base.priority, true), base.allocated_dockyards))];
        for entry in queues.iter().filter(|row| row.province == base.province) {
            let ship = ships.iter().find(|row| row.id == entry.ship);
            let maximum = models.iter().find(|row| row.ship == entry.ship).and_then(|row| row.counter.as_ref()).map(|row| row.maximum_strength.max(100));
            if ship.is_some() {
                live.value(&entry.ship.to_string(), "hull", Cell { table: TableId::Ships, column: hull_column, key: u128::from(entry.ship) }, 1.0);
            }
            lines.push(NavalManagementLine {
                text: format!("{}. Ship {} - hull HP ", u64::from(entry.ordinal) + 1, entry.ship >> 32),
                amount: Some(NavalManagementAmount {
                    row: entry.ship.to_string(),
                    column: "hull".into(),
                    raw: ship.map(|row| row.strength.to_string()),
                    of: maximum.map(|value| value.to_string()),
                    unit: "HP".into(),
                }),
            });
        }
        reply.sections.push(NavalManagementSection { key: format!("base:{}", base.province), title: format!("Repair port - province {}", base.province), lines });
    }
    if let Some(cat) = catalogue {
        reply.warnings.extend(cat.refusal_lines());
    }
    Ok(reply)
}
