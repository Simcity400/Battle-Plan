
use super::group_members::GroupsAt;
use dashboard_base::products::{Country, LoadContext, validate_country};
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{at_hour::{At, AtHour}, breakdown::{self, Amount, Breakdown, Exact, Leaf, Node, Unrecorded}}};
use dashboard_query::coverage::witnessed_rows;
use dashboard_query::identity::{Holder, Holders};
use dashboard_query::series::ranked::GroupSelection;
use replay_data::rows::{Recorded, TableId, TerritoryRegionTotalRow, TerritoryTotalRow, tables};

const WORLD: u32 = TerritoryTotalRow::WORLD;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
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

const REST_OF_WORLD: &str = "~rest";
const UNNAMED: &str = "Country not named at this hour";
const NO_TAG: &str = "No tag names this country at this hour";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryShareParams {
    pub branch_id: String,
    #[doc = " A declared territory measure."]
    pub measure: String,
    #[doc = " When set, keep the top N named countries and roll the rest into Rest of the world."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " The chosen countries: a tag is its holder at the hour, a slot is that recorded slot. Their shares of the"]
    #[doc = " world, the rest of it as one part."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[doc = " A group: its members at the hour (or every member so far), resolved in Rust; replaces `countries`."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryShareReply {
    pub hour: i32,
    pub measure: String,
    pub breakdown: Breakdown,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TerritoryShareReply);

pub struct ShareAsk<'a> {
    pub at: usize,
    pub top: Option<u32>,
    pub chosen: Option<&'a HashSet<u32>>,
    pub unnamed: Option<&'a str>,
}

fn share_country(slot: u32, tags: &HashMap<u32, (&str, u32)>, unnamed: Option<&str>) -> Node {
    match tags.get(&slot) {
        Some(&(tag, _)) => Node::new(format!("slot:{slot}"), tag, slot),
        None => Node::new(format!("slot:{slot}"), UNNAMED, slot).other().name_missing(Some(unnamed.unwrap_or(NO_TAG).to_owned())),
    }
}

fn known(value: i128) -> Amount {
    Amount::Known(Exact::whole(value))
}

pub fn share(rows: &[TerritoryTotalRow], tags: &HashMap<u32, (&str, u32)>, ask: &ShareAsk<'_>, hour: i32) -> TerritoryShareReply {
    let at = ask.at;
    let mut world = None;
    let mut held: Vec<(i64, u32)> = Vec::new();
    let mut unread: Vec<Leaf> = Vec::new();
    let mut known_sum: i128 = 0;
    for row in rows {
        if row.slot == WORLD {
            world = Some(cell(row, at));
            continue;
        }
        if ask.chosen.is_some_and(|chosen| !chosen.contains(&row.slot)) {
            continue;
        }
        match cell(row, at) {
            Some(0) => {}
            Some(value) => {
                known_sum += i128::from(value);
                held.push((value, row.slot));
            }
            None => unread.push(Leaf {
                path: vec![share_country(row.slot, tags, ask.unnamed)],
                amount: Amount::NotRecorded(format!("{} is not recorded for this country at this hour", MEASURES[at])),
                count: None,
                last: None,
            }),
        }
    }
    let world = match world {
        None => Err("The world's territory is not recorded at this hour".to_owned()),
        Some(None) => Err(format!("The world's {} is not recorded at this hour", MEASURES[at])),
        Some(Some(world)) => Ok(i128::from(world)),
    };
    held.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let leaf = |node: Node, value: i128| Leaf { path: vec![node], amount: known(value), count: None, last: None };
    let rest = |value: i128| leaf(Node::new(REST_OF_WORLD, "Rest of the world", u32::MAX).other(), value);
    let mut leaves = Vec::new();
    let mut warnings = Vec::new();
    match (ask.top, ask.chosen) {
        (Some(limit), None) => {
            let mut shown_sum: i128 = 0;
            for &(value, slot) in held.iter().filter(|(_, slot)| tags.contains_key(slot)).take(limit as usize) {
                shown_sum += i128::from(value);
                leaves.push(leaf(share_country(slot, tags, ask.unnamed), i128::from(value)));
            }
            match &world {
                Ok(world) if *world > shown_sum => leaves.push(rest(world - shown_sum)),
                Ok(_) => {}
                Err(_) => leaves.append(&mut unread),
            }
            if held.iter().any(|(_, slot)| !tags.contains_key(slot)) {
                warnings.push("Some territory is held by countries no tag names at this hour; it counts in the rest of the world.".to_owned());
            }
            if !unread.is_empty() {
                warnings.push(format!(
                    "The top {limit} is not proven: {} countries' {} are not recorded at this hour; they count in the rest of the world.",
                    unread.len(),
                    MEASURES[at]
                ));
                unread.clear();
            }
        }
        _ => {
            for &(value, slot) in &held {
                leaves.push(leaf(share_country(slot, tags, ask.unnamed), i128::from(value)));
            }
            if let (Ok(world), true) = (&world, unread.is_empty()) {
                let left = world - known_sum;
                if left > 0 {
                    leaves.push(match ask.chosen {
                        Some(_) => rest(left),
                        None => leaf(Node::new(NOT_HELD, "Held by no country", u32::MAX).other(), left),
                    });
                }
            }
        }
    }
    leaves.append(&mut unread);
    let total = match world {
        Ok(world) => known(world),
        Err(reason) => Amount::NotRecorded(reason),
    };
    TerritoryShareReply { hour, measure: MEASURES[at].to_owned(), breakdown: breakdown::breakdown_of(&leaves, &total), warnings }
}

impl AtHour for TerritoryShare {
    const KEY: &'static str = "territory-share";
    const TITLE: &'static str = "Territory share";
    const CONTENT_DEPENDENT: bool = false;
    type Params = TerritoryShareParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, measure, top, countries, "countries[]", group];
    type Reply = TerritoryShareReply;
    type Context = GroupsAt;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TerritoryShareParams::decl(), TerritoryShareReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.countries.iter().try_for_each(validate_country)?;
        if params.top.is_some() && (params.group.is_some() || !params.countries.is_empty()) {
            return Err("Ask a top or chosen countries, never both");
        }
        MEASURES.contains(&params.measure.as_str()).then_some(()).ok_or("Unknown territory measure")
    }

    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;

    fn projection(params: &Self::Params, table: TableId, _plan: &[(Vec<dashboard_base::products::at_hour::Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        (table == TableId::TerritoryTotals).then(|| vec![MEASURES.iter().position(|&m| m == params.measure).expect("validated measure")])
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::TerritoryTotals]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(GroupsAt::default())
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.valid_through(params.group.as_ref(), at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let measure = MEASURES.iter().position(|name| *name == params.measure).ok_or_else(|| dashboard_core::invalid("Unknown territory measure"))?;
        let unnamed = at
            .holders
            .unknown
            .iter()
            .find(|(first, last, _)| *first <= frame.hour && frame.hour <= *last)
            .map(|(_, _, reason)| format!("Country tags are not known at this hour: {reason}"));
        let chosen = match at.context.chosen_slots(params.group.as_ref(), &params.countries, at.holders, at.hour)? {
            Ok(chosen) => chosen,
            Err(refused) => return Ok(TerritoryShareReply { hour: frame.hour, measure: params.measure.clone(), breakdown: not_recorded(refused), warnings: Vec::new() }),
        };
        let reply = match frame.complete::<tables::TerritoryTotals>() {
            Ok(rows) => {
                let ask = ShareAsk { at: measure, top: params.top, chosen: chosen.as_ref(), unnamed: unnamed.as_deref() };
                share(rows, &slot_tags(at.holders, frame.hour), &ask, frame.hour)
            }
            Err(error) => TerritoryShareReply {
                hour: frame.hour,
                measure: params.measure.clone(),
                breakdown: not_recorded(format!("Territory totals are not known at this hour: {error}")),
                warnings: Vec::new(),
            },
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub struct TerritoryBreakdown;

pub const BREAKDOWN_MEASURES: [(&str, usize); 4] = [("states", 3), ("provinces", 0), ("victoryPoints", 1), ("area", 2)];
pub const RULES: [&str; 2] = ["controlled", "owned"];
const RULE_COLUMNS: usize = 5;
const STATES: usize = 3;
const REGION_UNKNOWN: &str = "~region";
const NOT_HELD: &str = "~unheld";
const UNREAD_ROWS: &str = "~unread";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryBreakdownParams {
    pub branch_id: String,
    #[doc = " `states`, `provinces`, `victoryPoints` or `area`."]
    pub measure: String,
    #[doc = " `controlled` or `owned`."]
    pub rule: String,
    #[doc = " The chosen countries: a tag is its holder at the hour, a slot is that recorded slot. Every country when"]
    #[doc = " empty and no group."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[doc = " A group: its members at the hour (or every member so far), resolved in Rust; replaces `countries`."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TerritoryBreakdownReply {
    pub hour: i32,
    #[doc = " Countries (key: `slot:N`, the recorded slot, named by the tag it holds at the hour or `other` where no"]
    #[doc = " tag names it; `~unheld` for land no country holds), each split by strategic region (`~region` where"]
    #[doc = " the region is not recorded). A state split between holders counts by its exact share."]
    pub breakdown: Breakdown,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TerritoryBreakdownReply);

pub struct BreakdownContent {
    regions: Result<RegionNames, String>,
    groups: GroupsAt,
}

#[derive(Default)]
struct RegionNames {
    names: HashMap<u32, Result<String, String>>,
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
            out.names.insert(region.id, name);
            for &province in &region.provinces {
                claims.entry(province).or_default().insert(region.id);
            }
        }
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

fn region_cell(row: &TerritoryRegionTotalRow, column: usize) -> Recorded<i64> {
    match column {
        0 => row.controlled_provinces,
        1 => row.controlled_victory_points,
        2 => row.controlled_area,
        3 => row.controlled_states,
        4 => row.controlled_states_denominator,
        5 => row.owned_provinces,
        6 => row.owned_victory_points,
        7 => row.owned_area,
        8 => row.owned_states,
        _ => row.owned_states_denominator,
    }
}

fn amount(row: &TerritoryRegionTotalRow, column: usize) -> Amount {
    let value = region_cell(row, column);
    if column % RULE_COLUMNS != STATES {
        return match value {
            Recorded::Value(value) => Amount::Known(Exact::whole(i128::from(value))),
            Recorded::NotRecorded => Amount::NotRecorded("Territory was not recorded here at this hour".into()),
        };
    }
    match (value, region_cell(row, column + 1)) {
        (Recorded::Value(value), Recorded::Value(denominator)) => match Exact::new(i128::from(value), i128::from(denominator)) {
            Some(share) => Amount::Known(share),
            None => Amount::NotRecorded("States were not recorded here at this hour".into()),
        },
        _ => Amount::NotRecorded("States were not recorded here at this hour".into()),
    }
}

fn territory_breakdown(
    rows: &[TerritoryRegionTotalRow],
    tags: &HashMap<u32, (&str, u32)>,
    params: &TerritoryBreakdownParams,
    chosen: Option<&HashSet<u32>>,
    regions: Result<&RegionNames, &str>,
    unread: Option<(&str, &CountryTotals)>,
) -> Breakdown {
    let column = breakdown_column(params).expect("validated measure and rule");
    let mut leaves = Vec::new();
    let mut witnessed: HashMap<u32, Option<Exact>> = HashMap::new();
    for row in rows {
        let held = (row.holder != TerritoryRegionTotalRow::NO_COUNTRY).then_some(row.holder);
        if let Some(chosen) = chosen
            && !held.is_some_and(|slot| chosen.contains(&slot))
        {
            continue;
        }
        let country = match held {
            Some(slot) => match tags.get(&slot) {
                Some(&(tag, _)) => Node::new(format!("slot:{slot}"), tag, slot),
                None => Node::new(format!("slot:{slot}"), "Country not named at this hour", slot).other().name_missing(Some("No tag names this country at this hour".to_owned())),
            },
            None => Node::new(NOT_HELD, "Held by no country", u32::MAX).other(),
        };
        let region = match (row.region != TerritoryRegionTotalRow::NO_REGION).then_some(row.region) {
            Some(id) => match regions.map(|regions| regions.names.get(&id)) {
                Ok(Some(Ok(name))) => Node::new(id.to_string(), name.clone(), id),
                Ok(Some(Err(reason))) => Node::new(id.to_string(), format!("Region {id}"), id).name_missing(Some(reason.clone())),
                Ok(None) => Node::new(id.to_string(), format!("Region {id}"), id).name_missing(Some(format!("Strategic region {id} is not in the recorded content"))),
                Err(reason) => Node::new(id.to_string(), format!("Region {id}"), id).name_missing(Some(format!("Strategic region names are unavailable: {reason}"))),
            },
            None => Node::new(REGION_UNKNOWN, "Region not recorded", u32::MAX).other(),
        };
        let amount = amount(row, column);
        if let (Some(slot), Amount::Known(known)) = (held, &amount) {
            let entry = witnessed.entry(slot).or_insert(Some(Exact::ZERO));
            *entry = entry.and_then(|sum| sum.plus(*known));
        }
        if matches!(amount, Amount::Known(known) if known.numerator() == 0) {
            continue;
        }
        leaves.push(Leaf { path: vec![country, region], amount, count: None, last: None });
    }
    let Some((reason, totals)) = unread else { return breakdown::breakdown(&leaves, &Unrecorded::default()) };
    let rest = || Node::new(UNREAD_ROWS, "Regions not read", u32::MAX).other().name_missing(Some(reason.to_owned()));
    for (&slot, total) in &totals.countries {
        if chosen.is_some_and(|chosen| !chosen.contains(&slot)) {
            continue;
        }
        let country = match tags.get(&slot) {
            Some(&(tag, _)) => Node::new(format!("slot:{slot}"), tag, slot),
            None => Node::new(format!("slot:{slot}"), "Country not named at this hour", slot).other().name_missing(Some("No tag names this country at this hour".to_owned())),
        };
        let seen = witnessed.get(&slot).copied().unwrap_or(Some(Exact::ZERO));
        let amount = match total {
            Some(total) => match seen.and_then(|seen| Exact::whole(i128::from(*total)).minus(seen)) {
                Some(left) if left.numerator() > 0 => Amount::Known(left),
                Some(_) => continue,
                None => Amount::NotRecorded(reason.to_owned()),
            },
            None => Amount::NotRecorded(format!("{reason}; this country's total was not read either")),
        };
        leaves.push(Leaf { path: vec![country, rest()], amount, count: None, last: None });
    }
    if let Some(missing) = &totals.missing
        && chosen.is_none_or(|chosen| chosen.iter().any(|slot| !totals.countries.contains_key(slot)))
    {
        leaves.push(Leaf { path: vec![Node::new(UNREAD_ROWS, "Territory not read", u32::MAX).other()], amount: Amount::NotRecorded(missing.clone()), count: None, last: None });
    }
    match (chosen, totals.world) {
        (None, Some(world)) => breakdown::breakdown_of(&leaves, &Amount::Known(Exact::whole(i128::from(world)))),
        _ => breakdown::breakdown(&leaves, &Unrecorded::default()),
    }
}

#[derive(Default)]
pub struct CountryTotals {
    countries: HashMap<u32, Option<i64>>,
    world: Option<i64>,
    missing: Option<String>,
}

fn country_totals(frame: &DecodedFrame, params: &TerritoryBreakdownParams) -> CountryTotals {
    let Some(at) = totals_column(params) else { return CountryTotals::default() };
    let read = witnessed_rows::<tables::TerritoryTotals>(frame);
    let mut out = CountryTotals { missing: read.missing.map(|reason| format!("Country territory totals were not read whole at this hour: {reason}")), ..CountryTotals::default() };
    for row in read.rows {
        if row.slot == WORLD {
            out.world = cell(row, at);
        } else {
            out.countries.insert(row.slot, cell(row, at));
        }
    }
    out
}

fn totals_column(params: &TerritoryBreakdownParams) -> Option<usize> {
    let mut measure = params.measure.clone();
    if let Some(first) = measure.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    let name = format!("{}{measure}", params.rule);
    MEASURES.iter().position(|known| *known == name)
}

fn not_recorded(reason: String) -> Breakdown {
    breakdown::breakdown(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(reason), count: None, last: None }], &Unrecorded::default())
}

impl AtHour for TerritoryBreakdown {
    const KEY: &'static str = "territory-breakdown";
    const TITLE: &'static str = "Territory breakdown";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = TerritoryBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, measure, rule, countries, "countries[]", group];
    type Reply = TerritoryBreakdownReply;
    type Context = Arc<BreakdownContent>;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TerritoryBreakdownParams::decl(), TerritoryBreakdownReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.countries.iter().try_for_each(validate_country)?;
        breakdown_column(params).map(|_| ()).ok_or("Unknown territory breakdown measure or rule")
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::TerritoryRegionTotals, TableId::TerritoryTotals]
    }

    fn projection(params: &Self::Params, table: TableId, _plan: &[(Vec<dashboard_base::products::at_hour::Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        match table {
            TableId::TerritoryRegionTotals => breakdown_column(params).map(|column| if column % RULE_COLUMNS == STATES { vec![column, column + 1] } else { vec![column] }),
            TableId::TerritoryTotals => totals_column(params).map(|column| vec![column]),
            _ => None,
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Arc::new(BreakdownContent { regions: RegionNames::load(source, load), groups: GroupsAt::default() }))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let groups = GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?;
        Ok(Arc::new(BreakdownContent { regions: RegionNames::load(source, load), groups }))
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.groups.valid_through(params.group.as_ref(), at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let (regions, mut warnings) = match &at.context.regions {
            Ok(regions) => (Ok(regions), regions.warnings.clone()),
            Err(reason) => (Err(reason.as_str()), vec![format!("Strategic region names are unavailable: {reason}")]),
        };
        let chosen = at.context.groups.chosen_slots(params.group.as_ref(), &params.countries, at.holders, at.hour)?;
        let read = witnessed_rows::<tables::TerritoryRegionTotals>(frame);
        let breakdown = match (read.missing, chosen) {
            (_, Err(refused)) => not_recorded(refused),
            (missing, Ok(chosen)) => {
                let unread = missing.map(|reason| (format!("Territory by strategic region was not read whole at this hour: {reason}"), country_totals(frame, params)));
                territory_breakdown(read.rows, &slot_tags(at.holders, frame.hour), params, chosen.as_ref(), regions, unread.as_ref().map(|(reason, totals)| (reason.as_str(), totals)))
            }
        };
        warnings.dedup();
        let reply = TerritoryBreakdownReply { hour: frame.hour, breakdown, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::breakdown::BreakdownAmount;

    fn ask(at: usize, top: Option<u32>) -> ShareAsk<'static> {
        ShareAsk { at, top, chosen: None, unnamed: None }
    }

    fn total(slot: u32, owned_states: Recorded<i64>, controlled_states: i64) -> TerritoryTotalRow {
        TerritoryTotalRow { slot, owned_states, controlled_states: Recorded::Value(controlled_states), ..TerritoryTotalRow::default() }
    }

    #[test]
    fn a_share_is_of_the_worlds_territory_and_unknown_where_the_world_is_not_recorded() {
        use dashboard_base::products::breakdown::BreakdownState;
        let tags: HashMap<u32, (&str, u32)> = [(1, ("GER", 10)), (2, ("ITA", 20))].into();
        let rows = [total(1, Recorded::Value(5), 6), total(2, Recorded::Value(3), 1), total(3, Recorded::Value(2), 0), total(WORLD, Recorded::Value(12), 12)];
        let owned = share(&rows, &tags, &ask(0, Some(1)), 500);
        assert_eq!(owned.hour, 500);
        assert_eq!(owned.breakdown.total.state, BreakdownState::Known);
        assert_eq!(owned.breakdown.total.amount, Some(BreakdownAmount { numerator: "12".into(), denominator: "1".into() }));
        let got: Vec<(&str, &str, bool)> = owned.breakdown.parts.iter().map(|part| (part.key.as_str(), part.name.as_str(), part.other)).collect();
        assert_eq!(got, [("slot:1", "GER", false), (REST_OF_WORLD, "Rest of the world", true)], "top N keeps named leaders by slot and folds the rest against the world total");
        assert_eq!(owned.breakdown.parts[0].amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() }));
        assert_eq!(owned.breakdown.parts[1].amount, Some(BreakdownAmount { numerator: "7".into(), denominator: "1".into() }));
        assert_eq!(owned.warnings.len(), 1, "unnamed territory folded into rest of the world is reported");

        let all = share(&rows, &tags, &ask(0, None), 500);
        let all_keys: Vec<(&str, bool)> = all.breakdown.parts.iter().map(|part| (part.key.as_str(), part.other)).collect();
        assert_eq!(all_keys, [("slot:1", false), ("slot:2", false), ("slot:3", true), (NOT_HELD, true)], "without top, unnamed slots and unheld land keep separate parts");
        assert!(all.warnings.is_empty());

        let controlled = share(&rows, &tags, &ask(1, Some(10)), 500);
        assert_eq!(controlled.breakdown.parts.len(), 3, "two named countries plus unheld land in rest of world");
        assert!(controlled.warnings.is_empty(), "a slot no tag names holding none is not land in the rest");

        let unread_world = [total(1, Recorded::Value(5), 6), total(WORLD, Recorded::NotRecorded, 7)];
        let missing_world = share(&unread_world, &tags, &ask(0, Some(10)), 500);
        assert_eq!(missing_world.breakdown.total.state, BreakdownState::NotRecorded, "the world not recorded leaves the whole unknown, never a smaller sum");
        assert_eq!(share(&unread_world[..1], &tags, &ask(1, Some(10)), 500).breakdown.total.state, BreakdownState::NotRecorded, "no world row, no whole");

        let unread_country = [total(1, Recorded::Value(5), 6), total(2, Recorded::NotRecorded, 1), total(WORLD, Recorded::Value(12), 12)];
        let partial = share(&unread_country, &tags, &ask(0, Some(10)), 500);
        assert_eq!(partial.breakdown.total.amount, Some(BreakdownAmount { numerator: "12".into(), denominator: "1".into() }), "the recorded world stays the whole");
        assert_eq!(partial.breakdown.parts.iter().map(|part| part.key.as_str()).collect::<Vec<_>>(), ["slot:1", REST_OF_WORLD], "the rest of the world is the world less the named, exactly, the unread country inside it");
        assert!(partial.warnings.iter().any(|warning| warning.contains("top 10 is not proven")), "a top with an unread contender says it is not proven");

        let every = share(&unread_country, &tags, &ask(0, None), 500);
        assert_eq!(every.breakdown.total.state, BreakdownState::Known);
        assert_eq!(every.breakdown.parts[0].share, Some(5.0 / 12.0), "a known country's share of the recorded world");
        assert_eq!(every.breakdown.parts.iter().map(|part| (part.key.as_str(), part.state)).collect::<Vec<_>>(), [("slot:1", BreakdownState::Known), ("slot:2", BreakdownState::NotRecorded)], "no land is claimed unheld while a country is unread");
    }

    #[test]
    fn an_unread_world_keeps_each_recorded_country_and_an_unnamed_holder_says_why() {
        use dashboard_base::products::breakdown::BreakdownState;
        let tags: HashMap<u32, (&str, u32)> = [(1, ("GER", 10))].into();
        let rows = [total(1, Recorded::Value(5), 6), total(3, Recorded::Value(2), 0), total(WORLD, Recorded::NotRecorded, 7)];
        let why = "Country tags are not known at this hour: links not read";
        let out = share(&rows, &tags, &ShareAsk { at: 0, top: None, chosen: None, unnamed: Some(why) }, 500);
        assert_eq!(out.breakdown.total.state, BreakdownState::NotRecorded);
        assert_eq!(out.breakdown.parts[0].amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() }), "GER's recorded 5 is not lost with the world");
        assert_eq!(out.breakdown.parts[1].name_missing.as_deref(), Some(why), "a country whose tag is unknown keeps its amount and the reason it has no name");
    }

    #[test]
    fn chosen_countries_are_their_shares_of_the_world_and_the_rest_is_one_part() {
        let tags: HashMap<u32, (&str, u32)> = [(1, ("GER", 10)), (2, ("ITA", 20))].into();
        let rows = [total(1, Recorded::Value(5), 6), total(2, Recorded::Value(3), 1), total(WORLD, Recorded::Value(12), 12)];
        let chosen: HashSet<u32> = [2].into();
        let out = share(&rows, &tags, &ShareAsk { at: 0, top: None, chosen: Some(&chosen), unnamed: None }, 500);
        let got: Vec<(&str, Option<BreakdownAmount>)> = out.breakdown.parts.iter().map(|part| (part.key.as_str(), part.amount.clone())).collect();
        let n = |value: &str| Some(BreakdownAmount { numerator: value.into(), denominator: "1".into() });
        assert_eq!(got, [("slot:2", n("3")), (REST_OF_WORLD, n("9"))], "a chosen small country keeps its part, never replaced by the world top ten");
        assert_eq!(out.breakdown.parts[0].share, Some(0.25));
    }

    #[test]
    fn the_breakdown_is_countries_by_region_at_the_hour_with_land_no_one_holds_named() {
        use dashboard_base::products::breakdown::BreakdownState;
        const NO_COUNTRY: u32 = TerritoryRegionTotalRow::NO_COUNTRY;
        const NO_REGION: u32 = TerritoryRegionTotalRow::NO_REGION;
        let tags: HashMap<u32, (&str, u32)> = [(1, ("AAA", 10)), (3, ("CCC", 30))].into();
        let held = |holder: u32, region: u32, states: Recorded<i64>, of: i64| TerritoryRegionTotalRow {
            holder,
            region,
            controlled_states: states,
            controlled_states_denominator: Recorded::Value(of),
            controlled_provinces: Recorded::Value(of),
            ..TerritoryRegionTotalRow::default()
        };
        let rows = [
            held(1, 7, Recorded::Value(7), 3),
            held(1, 8, Recorded::Value(2), 1),
            held(2, 7, Recorded::Value(1), 1),
            held(3, 8, Recorded::Value(0), 1),
            held(NO_COUNTRY, NO_REGION, Recorded::Value(1), 2),
        ];
        let regions = RegionNames { names: [(7, Ok("Northern Sea".to_owned())), (8, Err("missing".to_owned()))].into(), warnings: Vec::new() };
        let params = TerritoryBreakdownParams { branch_id: "1".into(), measure: "states".into(), rule: "controlled".into(), countries: Vec::new(), group: None };
        let out = territory_breakdown(&rows, &tags, &params, None, Ok(&regions), None);
        let keys: Vec<&str> = out.parts.iter().map(|part| part.key.as_str()).collect();
        assert_eq!(keys, ["slot:1", "slot:2", NOT_HELD], "keyed by recorded slot, the largest first, a country holding none has no part, then other");
        let names: Vec<(&str, bool)> = out.parts.iter().map(|part| (part.name.as_str(), part.other)).collect();
        assert_eq!(names, [("AAA", false), ("Country not named at this hour", true), ("Held by no country", true)]);
        let thirds = |numerator: &str| Some(BreakdownAmount { numerator: numerator.into(), denominator: "3".into() });
        assert_eq!(out.parts[0].amount, thirds("13"), "7/3 + 2 states, exactly");
        let children: Vec<(&str, &str)> = out.parts[0].children.iter().map(|part| (part.key.as_str(), part.name.as_str())).collect();
        assert_eq!(children, [("7", "Northern Sea"), ("8", "Region 8")], "a region without a name is named by its id, never a guess");
        assert_eq!(out.parts[0].children[1].name_missing.as_deref(), Some("missing"), "and keeps the recorded reason it has no name");
        assert!(out.parts[1].other && out.parts[2].other);
        assert_eq!(out.parts[2].children[0].key, REGION_UNKNOWN);
        assert_eq!(out.total.amount, Some(BreakdownAmount { numerator: "35".into(), denominator: "6".into() }), "13/3 + 1 + 1/2");
        let totals = CountryTotals { countries: [(1, Some(5)), (3, None)].into(), world: Some(9), missing: None };
        let truncated = territory_breakdown(&rows[..1], &tags, &params, None, Ok(&regions), Some(("read only in part", &totals)));
        assert_eq!(truncated.total.amount, Some(BreakdownAmount { numerator: "9".into(), denominator: "1".into() }), "the recorded world stays the whole");
        assert_eq!(truncated.parts[0].key, "slot:1");
        assert_eq!(truncated.parts[0].amount, Some(BreakdownAmount { numerator: "5".into(), denominator: "1".into() }), "the country's recorded total, its unread regions as one part");
        assert!(truncated.parts[0].children.iter().any(|child| child.key == UNREAD_ROWS && child.amount == Some(BreakdownAmount { numerator: "8".into(), denominator: "3".into() })));
        assert!(truncated.parts.iter().any(|part| part.key == "slot:3" && part.state == BreakdownState::NotRecorded), "a country whose total is unread is not recorded, never zero");
        let unread = [held(1, 7, Recorded::NotRecorded, 3), held(3, 8, Recorded::Value(2), 1)];
        let partly = territory_breakdown(&unread, &tags, &params, None, Err("names not read"), None);
        assert_eq!(partly.total.state, BreakdownState::AtLeast, "a region not recorded is never zero");
        assert_eq!(partly.parts.last().map(|part| part.state), Some(BreakdownState::NotRecorded));
        let provinces = TerritoryBreakdownParams { measure: "provinces".into(), ..params };
        assert_eq!(territory_breakdown(&rows, &tags, &provinces, None, Err("names not read"), None).total.amount, Some(BreakdownAmount { numerator: "8".into(), denominator: "1".into() }), "provinces are whole");
        assert!(breakdown_column(&TerritoryBreakdownParams { rule: "occupied".into(), ..provinces }).is_none(), "the breakdown's rules are controlled and owned");
    }

    #[test]
    fn a_state_split_between_regions_adds_up_exactly_and_a_sliver_is_never_zero() {
        let tags: HashMap<u32, (&str, u32)> = [(1, ("AAA", 10)), (2, ("BBB", 20))].into();
        let third = |holder: u32, region: u32, states: i64, of: i64| TerritoryRegionTotalRow {
            holder,
            region,
            controlled_states: Recorded::Value(states),
            controlled_states_denominator: Recorded::Value(of),
            ..TerritoryRegionTotalRow::default()
        };
        let rows = [third(1, 7, 1, 3), third(1, 8, 1, 3), third(1, 9, 1, 3), third(2, 7, 1, 300)];
        let params = TerritoryBreakdownParams { branch_id: "1".into(), measure: "states".into(), rule: "controlled".into(), countries: Vec::new(), group: None };
        let out = territory_breakdown(&rows, &tags, &params, None, Err("names not read"), None);
        assert_eq!(out.parts[0].amount, Some(BreakdownAmount { numerator: "1".into(), denominator: "1".into() }), "three thirds of one state are one state");
        assert_eq!(out.parts.len(), 2, "a three-hundredth of a state is held, never rounded away");
    }

    #[test]
    fn a_chosen_set_is_its_slots_at_the_hour_so_a_retagged_country_keeps_its_part_and_the_rest_is_left_out() {
        use dashboard_query::identity::HolderRun;
        let run = |slot: u32, first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot, id: slot * 10 }, starts_segment: true };
        let holders = Holders::new(
            [("AAA".to_owned(), vec![run(1, 0, 99)]), ("BBB".to_owned(), vec![run(1, 100, 200)]), ("CCC".to_owned(), vec![run(3, 0, 200)])].into_iter().collect(),
            Vec::new(),
        );
        let row = |holder: u32, states: i64| TerritoryRegionTotalRow {
            holder,
            region: 7,
            controlled_states: Recorded::Value(states),
            controlled_states_denominator: Recorded::Value(1),
            ..TerritoryRegionTotalRow::default()
        };
        let rows = [row(1, 2), row(3, 1), row(TerritoryRegionTotalRow::NO_COUNTRY, 1)];
        let groups = GroupsAt::default();
        let parts = |countries: Vec<Country>, hour: i32| {
            let params = TerritoryBreakdownParams { branch_id: "1".into(), measure: "states".into(), rule: "controlled".into(), countries, group: None };
            let chosen = groups.chosen_slots(None, &params.countries, &holders, hour).unwrap().unwrap();
            let out = territory_breakdown(&rows, &slot_tags(&holders, hour), &params, chosen.as_ref(), Err("names not read"), None);
            out.parts.iter().map(|part| (part.key.clone(), part.name.clone())).collect::<Vec<_>>()
        };
        let one = |key: &str, name: &str| vec![(key.to_owned(), name.to_owned())];
        assert_eq!(parts(vec![Country::Tag("AAA".into())], 50), one("slot:1", "AAA"), "only the chosen country, never every country and never none");
        assert_eq!(parts(vec![Country::Tag("AAA".into())], 150), Vec::new(), "no slot holds AAA at 150: the chosen tag holds nothing then");
        assert_eq!(parts(vec![Country::Slot(1)], 150), one("slot:1", "BBB"), "a chosen slot keeps its part across a retag, named by the tag it holds at the hour");
        assert_eq!(parts(Vec::new(), 150).len(), 3, "no choice is every country and the land no country holds");
    }
}
