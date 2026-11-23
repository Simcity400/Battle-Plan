
use dashboard_base::products::Country;
use dashboard_base::{products::{LoadContext, Product, at_hour::{At, AtHour, Owner, Read}}, queries::country_ids::Names};
use dashboard_world::products::{place_names, sea_regions::{self, Choice, SeaContent}};
use dashboard_base::{Source, catalogue::{self, Catalogue, Refusals}};
use dashboard_follow::live::Source as LiveSource;
use dashboard_query::identity::Holder;
use dashboard_text::variant_names::{self, Tags, Texts};
use replay_lanes::lanes;
use replay_columnar::summary;
use replay_data::rows::{NavalRegionalMineRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

const SCALE: i32 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MineOwner {
    pub tag: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MinedRegion {
    pub region: u32,
    pub owners: Vec<MineOwner>,
    pub unknown_owners: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UnknownRegion {
    pub region: u32,
    pub unknown_owners: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Fold {
    mined: Vec<MinedRegion>,
    unknown: Vec<UnknownRegion>,
    cells: Vec<(u32, String, u128)>,
}

struct Vector<'a> {
    tag: Option<&'a str>,
    extent: Option<u32>,
    amounts: BTreeMap<u32, i64>,
}

fn fold<'a>(rows: &[NavalRegionalMineRow], name: impl Fn(u32) -> Option<&'a str>, wanted: impl Fn(&str) -> bool, lost: bool, sea: &BTreeSet<u32>) -> Fold {
    let mut vectors: BTreeMap<u32, Vector<'a>> = BTreeMap::new();
    for row in rows {
        let vector = vectors.entry(row.country).or_insert_with(|| Vector { tag: name(row.country), extent: None, amounts: BTreeMap::new() });
        match row.kind {
            0 => vector.extent = Some(row.region),
            _ => {
                vector.amounts.insert(row.region, row.amount);
            }
        }
    }
    let unnamed = vectors.values().filter(|vector| vector.tag.is_none()).count() as u32 + u32::from(lost);
    let shown: Vec<(u32, &Vector)> = vectors.iter().filter(|(_, vector)| vector.tag.is_some_and(&wanted)).map(|(&raw, vector)| (raw, vector)).collect();
    let unknown_at = |region: u32| -> u32 {
        unnamed + shown.iter().filter(|(_, vector)| !vector.amounts.contains_key(&region) && vector.extent.is_none_or(|extent| extent <= region)).count() as u32
    };
    let mut amounts: BTreeMap<u32, Vec<(i64, &str, u32)>> = BTreeMap::new();
    for (raw, vector) in &shown {
        for (&region, &amount) in &vector.amounts {
            amounts.entry(region).or_default().push((amount, vector.tag.unwrap_or_default(), *raw));
        }
    }
    let mut regions: Vec<(i128, u32, Vec<(i64, &str, u32)>)> = amounts
        .into_iter()
        .map(|(region, mut owners)| {
            owners.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
            (owners.iter().map(|owner| i128::from(owner.0.max(0))).sum(), region, owners)
        })
        .collect();
    regions.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut cells = Vec::new();
    let mut mined = Vec::with_capacity(regions.len());
    for (_, region, owners) in regions {
        cells.extend(owners.iter().map(|&(_, tag, raw)| (region, tag.to_owned(), u128::from(raw) << 33 | 1 << 32 | u128::from(region))));
        mined.push(MinedRegion { region, owners: owners.into_iter().map(|(_, tag, _)| MineOwner { tag: tag.to_owned() }).collect(), unknown_owners: unknown_at(region) });
    }
    let unknown = sea
        .iter()
        .copied()
        .filter(|region| !mined.iter().any(|mined| mined.region == *region))
        .filter_map(|region| Some(UnknownRegion { region, unknown_owners: Some(unknown_at(region)).filter(|&count| count > 0)? }))
        .collect();
    Fold { mined, unknown, cells }
}

pub struct Minefields;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinefieldsParams {
    pub branch_id: String,
    #[doc = " The owners shown; every owner when none."]
    #[serde(default)]
    pub countries: Option<Vec<Country>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MinefieldsReply {
    pub regions: Vec<MinedRegion>,
    pub unknown: Vec<UnknownRegion>,
    #[doc = " Every sea region the content names and every region mined, for the region setting."]
    pub choices: Vec<Choice>,
    pub scale: i32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MinefieldsReply);

pub struct Context {
    places: Arc<place_names::Content>,
    sea: Arc<SeaContent>,
}

fn context(source: &Source, load: &LoadContext) -> io::Result<Context> {
    Ok(Context { places: place_names::Content::read(source, load)?, sea: SeaContent::read(source, load)? })
}

fn unrecorded(frame: &DecodedFrame, table: TableId, title: &str) -> io::Error {
    dashboard_core::refused(format!("{title} are not recorded at this hour: {}", dashboard_text::scope::missing(table, &frame.tables[table.index()])))
}

fn tag_of<'n>(links: &HashMap<u32, u32>, names: &'n Names, raw: u32, hour: i32) -> Option<&'n str> {
    names.slot_tag(*links.get(&raw)?, hour)
}

fn links(frame: &DecodedFrame) -> HashMap<u32, u32> {
    frame.complete::<tables::CountryLinks>().map(|rows| rows.iter().map(|row| (row.id, row.canonical_slot)).collect()).unwrap_or_default()
}

impl AtHour for Minefields {
    const KEY: &'static str = "minefields";
    const TITLE: &'static str = "Minefields";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MinefieldsParams;
    type Reply = MinefieldsReply;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![MinefieldsParams::decl(), MineOwner::decl(), MinedRegion::decl(), UnknownRegion::decl(), Choice::decl(), MinefieldsReply::decl()]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::NavalRegionalMines, TableId::CountryLinks]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let view = &frame.tables[TableId::NavalRegionalMines.index()];
        let (rows, lost) = match &**view {
            TableView::Complete(_) => (frame.complete::<tables::NavalRegionalMines>()?, false),
            TableView::Truncated { rows, .. } => (rows.rows::<tables::NavalRegionalMines>().ok_or_else(|| unrecorded(frame, TableId::NavalRegionalMines, "Minefields"))?, true),
            _ => return Err(unrecorded(frame, TableId::NavalRegionalMines, "Minefields")),
        };
        let links = links(frame);
        let names = Names::new(at.holders);
        let wanted: Option<BTreeSet<&str>> = params.countries.as_ref().map(|countries| countries.iter().filter_map(|country| at.holders.tag_at(country, frame.hour)).collect());
        let sea = at.context.sea.sea.clone().unwrap_or_default();
        let fold = fold(rows, |raw| tag_of(&links, &names, raw, frame.hour), |tag| wanted.as_ref().is_none_or(|wanted| wanted.contains(tag)), lost, &sea);
        let amount = dashboard_declare::field(TableId::NavalRegionalMines, "amount")?;
        let mut totals: BTreeMap<u32, Vec<LiveSource>> = BTreeMap::new();
        for (region, tag, key) in &fold.cells {
            let source = LiveSource { table: TableId::NavalRegionalMines, column: amount, key: *key };
            at.live.value(&region.to_string(), tag, source, 1.0);
            totals.entry(*region).or_default().push(source);
        }
        for (region, sources) in &totals {
            at.live.sum(&region.to_string(), "total", sources, 1.0);
        }
        let mut notes = Refusals::default();
        let choices = sea_regions::choices(&at.context.places, &at.context.sea, fold.mined.iter().map(|region| region.region), &mut notes);
        let mut warnings = at.context.places.warnings();
        warnings.extend(at.context.sea.notes.iter().cloned());
        warnings.extend(notes.lines("sea region names"));
        if lost {
            warnings.push("Some countries' minefields were not read at this hour".into());
        }
        let reply = MinefieldsReply { regions: fold.mined, unknown: fold.unknown, choices, scale: SCALE, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub type MineRuns = Vec<(i32, i32, Option<String>)>;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MineLine {
    pub country: Country,
    pub runs: MineRuns,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MinefieldHistoryReply {
    pub region: u32,
    pub lines: Vec<MineLine>,
    pub scale: i32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MinefieldHistoryReply);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinefieldHistoryParams {
    pub branch_id: String,
    pub region: u32,
    pub countries: Vec<Country>,
    pub first_hour: i32,
    pub last_hour: i32,
}

pub struct MinefieldHistory;

fn combine(amount: &lanes::Runs, witnesses: &[(u32, &lanes::Runs)], region: u32, from: i32, to: i32) -> Vec<(i32, i32, Option<i64>)> {
    let mut edges: BTreeSet<i32> = BTreeSet::from([from]);
    for runs in std::iter::once(amount).chain(witnesses.iter().map(|(_, runs)| *runs)) {
        for &(first, last, _) in runs {
            edges.insert(first.max(from));
            if last < to {
                edges.insert(last + 1);
            }
        }
    }
    let value_at = |runs: &lanes::Runs, hour: i32| runs.iter().find(|run| run.0 <= hour && hour <= run.1).and_then(|run| run.2);
    let live_at = |runs: &lanes::Runs, hour: i32| runs.iter().any(|run| run.0 <= hour && hour <= run.1 && run.2.is_some());
    let edges: Vec<i32> = edges.into_iter().filter(|&hour| hour <= to).collect();
    let mut out: Vec<(i32, i32, Option<i64>)> = Vec::new();
    for (at, &first) in edges.iter().enumerate() {
        let last = edges.get(at + 1).map_or(to, |next| next - 1);
        let value = value_at(amount, first).or_else(|| witnesses.iter().any(|(extent, runs)| region < *extent && live_at(runs, first)).then_some(0));
        match out.last_mut() {
            Some(run) if run.2 == value => run.1 = last,
            _ => out.push((first, last, value)),
        }
    }
    out
}

impl Product for MinefieldHistory {
    const KEY: &'static str = "minefield-history";
    const CONTENT_DEPENDENT: bool = false;
    type Params = MinefieldHistoryParams;
    type Reply = MinefieldHistoryReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params.first_hour > params.last_hour {
            return Err("A minefield history ends before it starts");
        }
        params.countries.iter().try_for_each(dashboard_base::products::validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![MinefieldHistoryParams::decl(), MineLine::decl(), MinefieldHistoryReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, _load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let recording = query.replay.recording();
        let amount = dashboard_declare::field(TableId::NavalRegionalMines, "amount")?;
        let mut lines = Vec::new();
        for tag in &params.countries {
            let mut runs: Vec<(i32, i32, Option<i64>)> = Vec::new();
            for held in query.holders.within(tag, params.first_hour, params.last_hour) {
                let Holder::Found { id, .. } = held.holder else { continue };
                for stretch in query.timeline.within(held.first, held.last) {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let owner = u128::from(id) << 33;
                    let key = owner | 1 << 32 | u128::from(params.region);
                    let first = dashboard_query::cost::storage(|| lanes::at_keys(recording, stretch.run, TableId::NavalRegionalMines, stretch.first, Some(&[amount][..]), lanes::Keys::Range(owner, owner | u128::from(u32::MAX))))?;
                    let mut keys: Vec<u128> = first.rows.iter().map(|&(key, _)| key).collect();
                    keys.push(key);
                    let held = dashboard_query::cost::storage(|| lanes::keyed_runs(recording, stretch.run, TableId::NavalRegionalMines, summary::Source::Cell(amount), Some(&keys), stretch.first, stretch.last))?;
                    let none = lanes::Runs::new();
                    let witnesses: Vec<(u32, &lanes::Runs)> = keys[..keys.len() - 1].iter().map(|&key| (key as u32, held.get(&key).unwrap_or(&none))).collect();
                    runs.extend(combine(held.get(&key).unwrap_or(&none), &witnesses, params.region, stretch.first, stretch.last));
                }
            }
            lines.push(MineLine { country: tag.clone(), runs: runs.into_iter().map(|(first, last, value)| (first, last, value.map(|value| value.to_string()))).collect() });
        }
        Ok(MinefieldHistoryReply { region: params.region, lines, scale: SCALE, warnings: Vec::new() })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MineReport {
    pub kind: u8,
    pub ordinal: u32,
    pub victim: Option<String>,
    pub mine_owner: Option<String>,
    pub ship: Option<String>,
    pub equipment: Option<String>,
    pub damage: String,
    pub sunk: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MinefieldReportsReply {
    pub region: u32,
    #[doc = " The hour the reports were read, where recorded."]
    pub read_hour: Option<i32>,
    pub reports: Vec<MineReport>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MinefieldReportsReply);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinefieldReportsParams {
    pub branch_id: String,
    pub region: u32,
}

pub struct MinefieldReports;

const REPORT_TABLES: [TableId; 7] = [
    TableId::NavalAccidentReports,
    TableId::VariantDefinitions,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::CountryCounterInputs,
];

fn standing(rows: &[replay_data::rows::NavalAccidentReportRow], region: u32) -> impl Iterator<Item = &replay_data::rows::NavalAccidentReportRow> {
    rows.iter().filter(move |row| row.kind == 1 && row.region == region)
}

fn ship_name(frame: &DecodedFrame, ship: u64, cat: &Catalogue) -> Result<Option<String>, String> {
    if ship == 0 {
        return Ok(None);
    }
    let bytes = frame
        .complete::<tables::CountryCounterInputs>()
        .ok()
        .and_then(|rows| rows.iter().find(|row| row.pride_of_the_fleet == ship))
        .and_then(|row| row.pride_name.require("pride name of country counter inputs").ok())
        .and_then(Option::as_ref)
        .ok_or("Standing mine report ship name is not recorded for its full handle")?;
    dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, "Mine report ship name").and_then(|text| catalogue::plain_text(&text)).map(Some)
}

fn equipment_name(frame: &DecodedFrame, equipment: u64, cat: &Catalogue) -> Result<Option<String>, String> {
    if equipment == 0 {
        return Ok(None);
    }
    let definition = frame.complete::<tables::VariantDefinitions>().map_err(|e| e.to_string())?.iter().find(|row| row.id == equipment).ok_or("Standing mine report equipment variant is not recorded")?;
    let countries = frame.complete::<tables::Countries>().map_err(|e| e.to_string())?;
    let links = frame.complete::<tables::CountryLinks>().map_err(|e| e.to_string())?;
    let tags = Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok());
    let lexical = frame.complete::<tables::LexicalTokens>().map_err(|e| e.to_string())?;
    let raw = variant_names::raw_variant(definition, frame.hour, &tags, lexical);
    variant_names::variant_name(&raw, Some(cat as &dyn Texts))?.ok_or_else(|| "Standing mine report equipment name is unavailable".into()).map(Some)
}

impl AtHour for MinefieldReports {
    const KEY: &'static str = "minefield-reports";
    const TITLE: &'static str = "Mine reports";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MinefieldReportsParams;
    type Reply = MinefieldReportsReply;
    type Context = Arc<Catalogue>;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![MinefieldReportsParams::decl(), MineReport::decl(), MinefieldReportsReply::decl()]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        REPORT_TABLES.to_vec()
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let reports = || held.complete::<tables::NavalAccidentReports>().map(|rows| standing(rows, params.region).cloned().collect::<Vec<_>>()).unwrap_or_default();
        match step {
            0 => vec![Read::all(TableId::NavalAccidentReports)],
            1 => {
                let reports = reports();
                if reports.is_empty() {
                    return Vec::new();
                }
                let mut reads = vec![Read::these(TableId::VariantDefinitions, reports.iter().map(|row| u128::from(row.equipment)))];
                reads.extend([TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::CountryCounterInputs].map(Read::all));
                reads
            }
            2 => {
                let tokens: Vec<u128> = held.complete::<tables::VariantDefinitions>().map(|rows| rows.iter().map(|row| u128::from(row.equipment_token)).collect()).unwrap_or_default();
                if tokens.is_empty() {
                    return Vec::new();
                }
                vec![Read::these(TableId::LexicalTokens, tokens)]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused("Mine reports need the recorded content stack"))?;
        catalogue::catalogue(source, options)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let rows = frame.complete::<tables::NavalAccidentReports>().map_err(|_| unrecorded(frame, TableId::NavalAccidentReports, "Mine reports"))?;
        let read_hour = frame.tables[TableId::NavalAccidentReports.index()].sampled_at().filter(|&hour| hour <= frame.hour);
        let links = links(frame);
        let names = Names::new(at.holders);
        let mut notes = Refusals::default();
        let mut reports = Vec::new();
        for (index, row) in standing(rows, params.region).enumerate() {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
            let mut country = |raw: u32, field: &str| {
                let tag = tag_of(&links, &names, raw, frame.hour).map(str::to_owned);
                if tag.is_none() {
                    notes.note(format!("Mine report {field} country is unresolved"), index.to_string());
                }
                tag
            };
            let victim = country(row.owner, "victim");
            let mine_owner = country(row.mine_owner, "mine owner");
            let ship = ship_name(frame, row.ship, at.context).unwrap_or_else(|why| {
                notes.note(why, index.to_string());
                None
            });
            let equipment = equipment_name(frame, row.equipment, at.context).unwrap_or_else(|why| {
                notes.note(why, index.to_string());
                None
            });
            reports.push(MineReport { kind: row.kind, ordinal: row.ordinal, victim, mine_owner, ship, equipment, damage: row.damage.to_string(), sunk: row.sunk != 0 });
        }
        let mut warnings = notes.lines("standing mine report fields");
        warnings.extend(at.context.refusal_lines());
        let reply = MinefieldReportsReply { region: params.region, read_hour, reports, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
