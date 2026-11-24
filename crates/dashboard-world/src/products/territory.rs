
use dashboard_base::products::LoadContext;
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::at_hour::{At, AtHour}};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::identity::{Holder, Holders};
use replay_data::rows::{Recorded, TableId, TerritoryRegionTotalRow, TerritoryTotalRow, tables};

const WORLD: u32 = TerritoryTotalRow::WORLD;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const MEASURES: [&str; 12] = [
    "ownedStates",
    "controlledStates",
    "occupiedStates",
    "ownedProvinces",
    "controlledProvinces",
    "occupiedProvinces",
    "ownedVictoryPoints",
    "controlledVictoryPoints",
    "occupiedVictoryPoints",
    "ownedArea",
    "controlledArea",
    "occupiedArea",
];

pub(crate) fn cell(row: &TerritoryTotalRow, at: usize) -> Option<i64> {
    let cell = match at {
        0 => &row.owned_states,
        1 => &row.controlled_states,
        2 => &row.occupied_states,
        3 => &row.owned_provinces,
        4 => &row.controlled_provinces,
        5 => &row.occupied_provinces,
        6 => &row.owned_victory_points,
        7 => &row.controlled_victory_points,
        8 => &row.occupied_victory_points,
        9 => &row.owned_area,
        10 => &row.controlled_area,
        _ => &row.occupied_area,
    };
    match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    }
}

pub fn territory_value(row: &TerritoryTotalRow, measure: &str) -> Option<i64> {
    let index = MEASURES.iter().position(|name| *name == measure)?;
    cell(row, index)
}

pub fn slot_tags(holders: &Holders, hour: i32) -> HashMap<u32, (&str, u32)> {
    let mut out = HashMap::new();
    for (tag, runs) in &holders.tags {
        for run in runs {
            if let (true, Holder::Found { slot, id }) = (run.first <= hour && hour <= run.last, run.holder) {
                out.entry(slot).or_insert((tag.as_str(), id));
            }
        }
    }
    out
}

fn branch(text: &str) -> Result<(), &'static str> {
    text.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

pub struct TerritoryShare;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryShareParams {
    pub branch_id: String,
    #[doc = " A declared territory measure."]
    pub measure: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryShareEntry {
    pub tag: String,
    pub slot: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryShareReply {
    pub measure: String,
    pub entries: Vec<TerritoryShareEntry>,
    #[doc = " Some is held by slots no tag holds at the hour: in the rest of the world, named by no country."]
    pub unnamed: bool,
    #[doc = " Why the whole is not known, never a smaller sum."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TerritoryShareReply, TerritoryBreakdownReply);

pub fn share(rows: &[TerritoryTotalRow], tags: &HashMap<u32, (&str, u32)>, at: usize, live: &Cells) -> TerritoryShareReply {
    let (mut entries, mut unnamed, mut world) = (Vec::new(), Vec::new(), None);
    for row in rows {
        if row.slot == WORLD {
            world = Some(cell(row, at));
            continue;
        }
        let Some(value) = cell(row, at) else { continue };
        match tags.get(&row.slot) {
            Some(&(tag, _)) if value != 0 => entries.push((value, TerritoryShareEntry { tag: tag.to_owned(), slot: row.slot })),
            Some(_) => {}
            None if value != 0 => unnamed.push(row.slot),
            None => {}
        }
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.tag.cmp(&b.1.tag)));
    let source = |slot: u32| Cell { table: TableId::TerritoryTotals, column: at, key: u128::from(slot) };
    live.value("world", "total", source(WORLD), 1.0);
    if !unnamed.is_empty() {
        live.sum("world", "unnamed", &unnamed.iter().map(|&slot| source(slot)).collect::<Vec<_>>(), 1.0);
    }
    for (_, entry) in &entries {
        live.value(&entry.slot.to_string(), "value", source(entry.slot), 1.0);
    }
    let missing = match world {
        Some(Some(_)) => None,
        Some(None) => Some(format!("The world's {} is not recorded at this hour", MEASURES[at])),
        None => Some("The world's territory is not recorded at this hour".to_owned()),
    };
    TerritoryShareReply {
        measure: MEASURES[at].to_owned(),
        entries: entries.into_iter().map(|(_, entry)| entry).collect(),
        unnamed: !unnamed.is_empty(),
        missing,
        warnings: Vec::new(),
    }
}

impl AtHour for TerritoryShare {
    const KEY: &'static str = "territory-share";
    const TITLE: &'static str = "Territory share";
    const CONTENT_DEPENDENT: bool = false;
    type Params = TerritoryShareParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, measure];
    type Reply = TerritoryShareReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TerritoryShareParams::decl(), TerritoryShareEntry::decl(), TerritoryShareReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        MEASURES.contains(&params.measure.as_str()).then_some(()).ok_or("Unknown territory measure")
    }


    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;

    fn projection(params: &Self::Params, table: TableId, _plan: &[(Vec<dashboard_base::products::at_hour::Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        (table == TableId::TerritoryTotals).then(|| vec![MEASURES.iter().position(|&m| m == params.measure).expect("validated measure")])
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::TerritoryTotals]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<()> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, ()>) -> io::Result<Self::Reply> {
        let measure = MEASURES.iter().position(|name| *name == params.measure).ok_or_else(|| dashboard_core::invalid("Unknown territory measure"))?;
        Ok(match frame.complete::<tables::TerritoryTotals>() {
            Ok(rows) => share(rows, &slot_tags(at.holders, frame.hour), measure, at.live),
            Err(error) => TerritoryShareReply {
                measure: params.measure.clone(),
                entries: Vec::new(),
                unnamed: false,
                missing: Some(format!("Territory totals are not known at this hour: {error}")),
                warnings: Vec::new(),
            },
        })
    }
}

pub struct TerritoryBreakdown;

pub const BREAKDOWN_MEASURES: [(&str, usize); 4] = [("states", 3), ("provinces", 0), ("victoryPoints", 1), ("area", 2)];
pub const RULES: [&str; 2] = ["controlled", "owned"];
const RULE_COLUMNS: usize = 5;
const STATES: usize = 3;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryBreakdownParams {
    pub branch_id: String,
    #[doc = " `states`, `provinces`, `victoryPoints` or `area`."]
    pub measure: String,
    #[doc = " `controlled` or `owned`."]
    pub rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegion {
    pub id: u32,
    pub name: Option<String>,
    #[doc = " Why it has no name."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegionRow {
    pub key: String,
    pub tag: Option<String>,
    pub slot: Option<u32>,
    pub region: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryBreakdownReply {
    pub measure: String,
    pub rule: String,
    pub regions: Vec<TerritoryRegion>,
    pub rows: Vec<TerritoryRegionRow>,
    #[doc = " Why the rows are not known at this hour."]
    pub missing: Option<String>,
    #[doc = " Why area by region is not known: the recording's facts hold no province areas."]
    pub area_reason: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct HeldAreas(Option<Result<Arc<Vec<i64>>, String>>);

fn province_areas(source: &Source) -> Result<Arc<Vec<i64>>, String> {
    let held = source.state::<HeldAreas>();
    let mut held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    held.0
        .get_or_insert_with(|| {
            let pack = source.facts().map_err(|reason| format!("the recording carries no game facts: {reason}"))?;
            let options = content_reader_layers::content::ResolveOptions { game_dir: Default::default(), user_dir: Default::default(), cache_dir: Default::default() };
            let content = content_reader_layers::content::playback(&source.replay_ref().identity, Ok(pack), &options)
                .map_err(|error| format!("the recording's game facts cannot be mounted: {error}"))?;
            source
                .in_scope(|| content_reader_province_area::province_area::ProvinceAreas::read(&content.layers))
                .map(|areas| Arc::new(areas.land))
                .map_err(|error| format!("the province areas cannot be read from the recording's game facts: {error}"))
        })
        .clone()
}

pub struct BreakdownContent {
    regions: Result<RegionNames, String>,
    area_reason: Option<String>,
}

#[derive(Default)]
struct RegionNames {
    names: Vec<TerritoryRegion>,
    warnings: Vec<String>,
}

impl RegionNames {
    fn read(cat: &Catalogue) -> io::Result<Self> {
        let content = dashboard_base::content::strategic_regions(&cat.content.layers)?;
        let mut out = Self { warnings: content.notes.clone(), ..Self::default() };
        let mut claims: HashMap<u32, BTreeSet<u32>> = HashMap::new();
        for region in content.regions.values() {
            let name = if cat.has_presentation_key(&region.name_key) {
                cat.localisation.localise(region.name_key.as_bytes()).map_err(|error| error.to_string()).and_then(|text| catalogue::plain_text(&text))
            } else {
                Err(format!("localisation {} is missing; region title is unavailable", region.name_key))
            };
            let (name, missing) = match name {
                Ok(name) => (Some(name), None),
                Err(reason) => (None, Some(reason)),
            };
            out.names.push(TerritoryRegion { id: region.id, name, missing });
            for &province in &region.provinces {
                claims.entry(province).or_default().insert(region.id);
            }
        }
        out.names.sort_unstable_by_key(|region| region.id);
        let ambiguous = claims.values().filter(|regions| regions.len() > 1).count();
        if ambiguous != 0 {
            out.warnings.push(format!("{ambiguous} provinces belong to conflicting strategic regions; their region is not attributable"));
        }
        Ok(out)
    }

    fn load(source: &Source, load: &LoadContext) -> Result<Self, String> {
        load.content().and_then(|content| catalogue::catalogue(source, content)).and_then(|cat| Self::read(&cat)).map_err(|error| error.to_string())
    }
}

fn breakdown_column(params: &TerritoryBreakdownParams) -> Option<usize> {
    let rule = RULES.iter().position(|rule| *rule == params.rule)?;
    let (_, measure) = BREAKDOWN_MEASURES.iter().find(|(name, _)| *name == params.measure)?;
    Some(rule * RULE_COLUMNS + measure)
}

pub fn breakdown(rows: &[TerritoryRegionTotalRow], tags: &HashMap<u32, (&str, u32)>, column: usize, live: &Cells) -> Vec<TerritoryRegionRow> {
    rows.iter()
        .map(|row| {
            let key = TerritoryRegionTotalRow::key_of(row.holder, row.region);
            let id = key.to_string();
            let source = |column: usize| Cell { table: TableId::TerritoryRegionTotals, column, key };
            live.value(&id, "value", source(column), 1.0);
            if column % RULE_COLUMNS == STATES {
                live.value(&id, "denominator", source(column + 1), 1.0);
            }
            let slot = (row.holder != TerritoryRegionTotalRow::NO_COUNTRY).then_some(row.holder);
            TerritoryRegionRow {
                key: id,
                tag: slot.and_then(|slot| tags.get(&slot)).map(|&(tag, _)| tag.to_owned()),
                slot,
                region: (row.region != TerritoryRegionTotalRow::NO_REGION).then_some(row.region),
            }
        })
        .collect()
}

impl AtHour for TerritoryBreakdown {
    const KEY: &'static str = "territory-breakdown";
    const TITLE: &'static str = "Territory breakdown";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = TerritoryBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, measure, rule];
    type Reply = TerritoryBreakdownReply;
    type Context = Arc<BreakdownContent>;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TerritoryBreakdownParams::decl(), TerritoryRegion::decl(), TerritoryRegionRow::decl(), TerritoryBreakdownReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        breakdown_column(params).map(|_| ()).ok_or("Unknown territory breakdown measure or rule")
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::TerritoryRegionTotals]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Arc::new(BreakdownContent { regions: RegionNames::load(source, load), area_reason: None }))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let area_reason = (params.measure == "area")
            .then(|| province_areas(source).err().map(|reason| format!("Area by strategic region is not known: {reason}")))
            .flatten();
        Ok(Arc::new(BreakdownContent { regions: RegionNames::load(source, load), area_reason }))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let column = breakdown_column(params).ok_or_else(|| dashboard_core::invalid("Unknown territory breakdown measure or rule"))?;
        let (regions, warnings) = match &at.context.regions {
            Ok(regions) => (regions.names.clone(), regions.warnings.clone()),
            Err(reason) => (Vec::new(), vec![format!("Strategic region names are unavailable: {reason}")]),
        };
        let (rows, missing) = match frame.complete::<tables::TerritoryRegionTotals>() {
            Ok(rows) => (breakdown(rows, &slot_tags(at.holders, frame.hour), column, at.live), None),
            Err(error) => (Vec::new(), Some(format!("Territory by strategic region is not known at this hour: {error}"))),
        };
        let reply = TerritoryBreakdownReply {
            measure: params.measure.clone(),
            rule: params.rule.clone(),
            regions,
            rows,
            missing,
            area_reason: at.context.area_reason.clone(),
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(slot: u32, owned_states: Recorded<i64>, controlled_states: i64) -> TerritoryTotalRow {
        TerritoryTotalRow { slot, owned_states, controlled_states: Recorded::Value(controlled_states), ..TerritoryTotalRow::default() }
    }

    #[test]
    fn a_share_is_of_the_worlds_territory_and_unknown_where_the_world_is_not_recorded() {
        let tags: HashMap<u32, (&str, u32)> = [(1, ("GER", 10)), (2, ("ITA", 20))].into();
        let rows = [total(1, Recorded::Value(5), 6), total(2, Recorded::Value(3), 1), total(3, Recorded::Value(2), 0), total(WORLD, Recorded::Value(12), 12)];
        let cells = Cells::default();
        let owned = share(&rows, &tags, 0, &cells);
        let got: Vec<(&str, u32)> = owned.entries.iter().map(|entry| (entry.tag.as_str(), entry.slot)).collect();
        assert_eq!(got, [("GER", 1), ("ITA", 2)], "the world row is never a country, and the greatest comes first");
        assert_eq!((owned.unnamed, owned.missing.as_deref()), (true, None), "the whole is the world's, unowned land in it");
        let json = cells.finish().json(&[]);
        assert_eq!(json["rows"], serde_json::json!(["world", "1", "2"]), "the amounts are cells: the world's whole and part no tag names, each country's");
        let kinds: Vec<&str> = json["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["total", "unnamed", "value"]);
        let controlled = share(&rows, &tags, 1, &Cells::default());
        assert_eq!(controlled.entries.len(), 2, "a country holding none has no slice");
        assert!(!controlled.unnamed, "a slot no tag names holding none is not land in the rest");
        let rows = [total(1, Recorded::Value(5), 6), total(WORLD, Recorded::NotRecorded, 7)];
        let owned = share(&rows, &tags, 0, &Cells::default());
        assert!(owned.missing.is_some(), "the world not recorded leaves the whole unknown, never a smaller sum");
        assert!(share(&rows[..1], &tags, 1, &Cells::default()).missing.is_some(), "no world row, no whole");
    }

    #[test]
    fn breakdown_rows_carry_no_amount_and_name_each_as_a_live_cell() {
        const NO_COUNTRY: u32 = TerritoryRegionTotalRow::NO_COUNTRY;
        const NO_REGION: u32 = TerritoryRegionTotalRow::NO_REGION;
        let tags: HashMap<u32, (&str, u32)> = [(1, ("GER", 10))].into();
        let held = |holder: u32, region: u32| TerritoryRegionTotalRow {
            holder,
            region,
            owned_provinces: Recorded::Value(4),
            owned_states: Recorded::Value(7),
            owned_states_denominator: Recorded::Value(3),
            ..TerritoryRegionTotalRow::default()
        };
        let rows = [held(1, 7), held(2, 7), held(NO_COUNTRY, NO_REGION)];
        let owned = TerritoryBreakdownParams { branch_id: "1".into(), measure: "states".into(), rule: "owned".into() };
        let cells = Cells::default();
        let out = breakdown(&rows, &tags, breakdown_column(&owned).unwrap(), &cells);
        let shown: Vec<(Option<&str>, Option<u32>, Option<u32>)> = out.iter().map(|row| (row.tag.as_deref(), row.slot, row.region)).collect();
        assert_eq!(shown, [(Some("GER"), Some(1), Some(7)), (None, Some(2), Some(7)), (None, None, None)], "a slot no tag names, and land no country holds in no region");
        for row in &out {
            let mut fields: Vec<String> = serde_json::to_value(row).unwrap().as_object().unwrap().keys().cloned().collect();
            fields.sort_unstable();
            assert_eq!(fields, ["key", "region", "slot", "tag"], "no amount is in the rows");
        }
        let live = cells.finish();
        let json = live.json(&[]);
        let keys: Vec<String> = rows.iter().map(|row| TerritoryRegionTotalRow::key_of(row.holder, row.region).to_string()).collect();
        assert_eq!(json["rows"], serde_json::json!(keys));
        let kinds: Vec<&str> = json["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["value", "denominator"]);
        assert_eq!(live.groups.len(), 1, "one followed query of every row");
        assert_eq!((live.groups[0].table, live.groups[0].columns.as_slice()), (TableId::TerritoryRegionTotals, &[8, 9][..]), "owned states over their denominator");
        let controlled = TerritoryBreakdownParams { measure: "provinces".into(), rule: "controlled".into(), ..owned.clone() };
        let cells = Cells::default();
        breakdown(&rows, &tags, breakdown_column(&controlled).unwrap(), &cells);
        assert_eq!(cells.finish().groups[0].columns, [0], "a whole measure is one cell");
        assert!(breakdown_column(&TerritoryBreakdownParams { rule: "occupied".into(), ..owned }).is_none(), "the breakdown's rules are controlled and owned");
    }
}
