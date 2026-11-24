
use dashboard_base::products::Country;
use dashboard_base::{products::{LoadContext, at_hour::{At, AtHour}}, queries::country_ids::Names};
use dashboard_world::products::{place_names, sea_regions::{self, SeaContent}};
use dashboard_base::{Source, catalogue::Refusals};
use dashboard_follow::live::Source as LiveSource;
use replay_data::rows::{NavalRegionalMineRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
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
    #[doc = " One of the user's regions."]
    pub pinned: bool,
    #[doc = " Why a chosen region's mines are not known at the hour."]
    pub missing: Option<String>,
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
        mined.push(MinedRegion { region, owners: owners.into_iter().map(|(_, tag, _)| MineOwner { tag: tag.to_owned() }).collect(), unknown_owners: unknown_at(region), pinned: false, missing: None });
    }
    let unknown = sea
        .iter()
        .copied()
        .filter(|region| !mined.iter().any(|mined| mined.region == *region))
        .filter_map(|region| Some(UnknownRegion { region, unknown_owners: Some(unknown_at(region)).filter(|&count| count > 0)? }))
        .collect();
    Fold { mined, unknown, cells }
}

fn register_live(fold: &Fold, live: &dashboard_follow::live::Cells) -> io::Result<()> {
    let amount = dashboard_declare::field(TableId::NavalRegionalMines, "amount")?;
    let mut totals: BTreeMap<u32, Vec<LiveSource>> = BTreeMap::new();
    for (region, tag, key) in &fold.cells {
        let source = LiveSource { table: TableId::NavalRegionalMines, column: amount, key: *key };
        live.value(&region.to_string(), tag, source, 1.0);
        totals.entry(*region).or_default().push(source);
    }
    for (region, sources) in &totals {
        live.sum(&region.to_string(), "total", sources, 1.0);
    }
    Ok(())
}

pub struct Minefields;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum MinefieldOrder {
    #[default]
    Mines,
    Name,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MinefieldsParams {
    pub branch_id: String,
    #[doc = " The owners shown; every owner when none."]
    #[serde(default)]
    pub countries: Option<Vec<Country>>,
    #[doc = " The user's sea regions, shown first in this order whether mined or not."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[doc = " Mined regions the user removed from the automatic set."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[doc = " Region ordering; defaults to most mines."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub order: Option<MinefieldOrder>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MinefieldsReply {
    pub regions: Vec<MinedRegion>,
    pub unknown: Vec<UnknownRegion>,
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

fn choose(fold: &mut Fold, pinned: &[u32], hidden: &[u32], lost: bool) -> HashMap<u32, usize> {
    let mut rank: HashMap<u32, usize> = HashMap::with_capacity(pinned.len());
    for &id in pinned {
        let next = rank.len();
        rank.entry(id).or_insert(next);
    }
    let hidden: BTreeSet<u32> = hidden.iter().copied().filter(|id| !rank.contains_key(id)).collect();
    fold.mined.retain(|region| !hidden.contains(&region.region));
    fold.cells.retain(|(region, ..)| !hidden.contains(region));
    fold.unknown.retain(|region| !hidden.contains(&region.region));
    for region in &mut fold.mined {
        region.pinned = rank.contains_key(&region.region);
    }
    for unknown in &fold.unknown {
        if rank.contains_key(&unknown.region) && !fold.mined.iter().any(|region| region.region == unknown.region) {
            fold.mined.push(MinedRegion { region: unknown.region, owners: Vec::new(), unknown_owners: unknown.unknown_owners, pinned: true, missing: None });
        }
    }
    fold.unknown.retain(|region| !rank.contains_key(&region.region));
    for &id in pinned {
        if !fold.mined.iter().any(|region| region.region == id) {
            let missing = lost.then(|| "Some countries' minefields were not read at this hour".to_owned());
            fold.mined.push(MinedRegion { region: id, owners: Vec::new(), unknown_owners: 0, pinned: true, missing });
        }
    }
    rank
}

impl AtHour for Minefields {
    const KEY: &'static str = "minefields";
    const TITLE: &'static str = "Minefields";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MinefieldsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", pinnedRegions, "pinnedRegions[]", hiddenRegions, "hiddenRegions[]", order];
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
        vec![MinefieldOrder::decl(), MinefieldsParams::decl(), MineOwner::decl(), MinedRegion::decl(), UnknownRegion::decl(), MinefieldsReply::decl()]
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
        let mut fold = fold(rows, |raw| tag_of(&links, &names, raw, frame.hour), |tag| wanted.as_ref().is_none_or(|wanted| wanted.contains(tag)), lost, &sea);
        let pinned = choose(&mut fold, params.pinned_regions.as_deref().unwrap_or_default(), params.hidden_regions.as_deref().unwrap_or_default(), lost);
        register_live(&fold, at.live)?;
        let mut notes = Refusals::default();
        let choices = sea_regions::choices(&at.context.places, &at.context.sea, fold.mined.iter().map(|region| region.region), &mut notes);
        if params.order == Some(MinefieldOrder::Name) {
            let names_by_id: HashMap<u32, &str> = choices.iter().filter_map(|choice| Some((choice.id.parse::<u32>().ok()?, choice.name.as_str()))).collect();
            fold.mined.sort_by(|a, b| {
                let an = names_by_id.get(&a.region).copied().unwrap_or("Region name unavailable");
                let bn = names_by_id.get(&b.region).copied().unwrap_or("Region name unavailable");
                an.to_lowercase().cmp(&bn.to_lowercase()).then_with(|| an.cmp(bn)).then_with(|| a.region.cmp(&b.region))
            });
        }
        fold.mined.sort_by_key(|region| pinned.get(&region.region).copied().unwrap_or(usize::MAX));
        let mut warnings = at.context.places.warnings();
        warnings.extend(at.context.sea.notes.iter().cloned());
        warnings.extend(notes.lines("sea region names"));
        if lost {
            warnings.push("Some countries' minefields were not read at this hour".into());
        }
        let reply = MinefieldsReply { regions: fold.mined, unknown: fold.unknown, scale: SCALE, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
