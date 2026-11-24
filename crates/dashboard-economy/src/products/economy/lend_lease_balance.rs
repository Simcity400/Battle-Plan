use dashboard_base::products::validate_country;
use dashboard_base::products::Country;
use dashboard_base::products::LoadContext;
use dashboard_base::{Source, products::at_hour::{At, AtHour, Read}};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SCALE: u32 = 100_000;

pub struct LendLeaseBalance;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LendLeaseBalanceParams {
    pub branch_id: String,
    #[doc = " One country's partners; every country's totals when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub holder: Option<Country>,
    #[doc = " Orders the rows by this measure's sent plus received, largest first, then by slot;"]
    #[doc = " without `countries`, a row with neither is left out."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub rank: Option<LendLeaseMeasure>,
    #[doc = " The first this many ranked rows."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " Only these countries' rows, a recorded zero included."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LendLeaseMeasure {
    Industry,
    Fuel,
}

impl LendLeaseMeasure {
    fn total(self, values: &[i128; 4]) -> i128 {
        let at = match self {
            Self::Industry => 0,
            Self::Fuel => 1,
        };
        values[at] + values[2 + at]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseRow {
    pub slot: u32,
    #[doc = " The tag its slot's country holds at the hour; null where the recorded countries name none."]
    pub tag: Option<String>,
    pub values: [String; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseBalanceReply {
    pub holder: Option<String>,
    pub scale: u32,
    #[doc = " Every country with a history or on the rosters, or the holder's partners with a history;"]
    #[doc = " in slot order."]
    pub rows: Vec<LendLeaseRow>,
    #[doc = " Why the histories are not known at the hour."]
    pub missing: Option<String>,
    #[doc = " Why the asked rank is not proven: a history names a country no identity resolves. A top then holds no rows."]
    pub rank_missing: Option<String>,
    #[doc = " Why a country not listed may still hold lend-lease or a roster place: a history or roster entry is unresolved, or the rosters were not read."]
    pub unlisted: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LendLeaseBalanceReply);

struct Slots {
    links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, String>,
}

impl Slots {
    fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let links = frame.complete::<tables::CountryLinks>()?.iter().map(|link| (link.id, link.canonical_slot)).collect();
        let mut tags = BTreeMap::new();
        for country in frame.complete::<tables::Countries>()? {
            if let Ok(tag) = std::str::from_utf8(&country.tag)
                && !tag.is_empty()
            {
                tags.entry(country.canonical_slot).or_insert_with(|| tag.to_owned());
            }
        }
        Ok(Self { links, tags })
    }
}

fn chosen(params: &LendLeaseBalanceParams, slot: u32, tag: Option<&str>) -> bool {
    params.countries.as_ref().is_none_or(|countries| {
        countries.iter().any(|country| match country {
            Country::Slot(wanted) => *wanted == slot,
            Country::Tag(wanted) => tag == Some(wanted.as_str()),
        })
    })
}

fn ranked(sums: BTreeMap<u32, [i128; 4]>, slots: &Slots, params: &LendLeaseBalanceParams) -> Vec<LendLeaseRow> {
    let mut rows: Vec<(u32, [i128; 4])> = sums
        .into_iter()
        .filter(|(slot, values)| {
            chosen(params, *slot, slots.tags.get(slot).map(String::as_str))
                && (params.countries.is_some() || params.rank.is_none_or(|measure| measure.total(values) != 0))
        })
        .collect();
    if let Some(measure) = params.rank {
        rows.sort_by(|(a_slot, a), (b_slot, b)| measure.total(b).cmp(&measure.total(a)).then(a_slot.cmp(b_slot)));
    }
    if let Some(top) = params.top {
        rows.truncate(usize::try_from(top).unwrap_or(usize::MAX));
    }
    rows.into_iter()
        .map(|(slot, values)| LendLeaseRow { slot, tag: slots.tags.get(&slot).cloned(), values: values.map(|value| value.to_string()) })
        .collect()
}

fn reply(frame: &DecodedFrame, params: &LendLeaseBalanceParams, holder: Option<(&str, u32)>) -> io::Result<LendLeaseBalanceReply> {
    let mut out = LendLeaseBalanceReply { holder: holder.map(|(tag, _)| tag.to_owned()), scale: SCALE, rows: Vec::new(), missing: None, rank_missing: None, unlisted: None, warnings: Vec::new() };
    let histories = match frame.complete::<tables::WarLendLeaseHistories>() {
        Ok(rows) => rows,
        Err(error) => {
            out.missing = Some(format!("Lend-lease histories are not recorded at this hour: {error}"));
            return Ok(out);
        }
    };
    let slots = Slots::read(frame)?;
    let mut sums: BTreeMap<u32, [i128; 4]> = BTreeMap::new();
    let (mut orphans, mut strays) = (0, 0);
    for row in histories {
        let Some(&owner) = slots.links.get(&row.country) else {
            orphans += 1;
            continue;
        };
        let key = match holder {
            None => owner,
            Some((_, slot)) if slot == owner => match slots.links.get(&row.target) {
                Some(&partner) => partner,
                None => {
                    strays += 1;
                    continue;
                }
            },
            Some(_) => continue,
        };
        let held = sums.entry(key).or_default();
        for (at, value) in [row.industry_sent, row.fuel_sent, row.industry_received, row.fuel_received].into_iter().enumerate() {
            held[at] += i128::from(value);
        }
    }
    if holder.is_none() {
        match frame.complete::<tables::WarOwnedRelations>() {
            Ok(relations) => {
                let mut rostered = BTreeSet::new();
                let mut unresolved = 0;
                for relation in relations {
                    for raw in [relation.owner, relation.first_country, relation.second_country] {
                        match slots.links.get(&raw) {
                            Some(&slot) => {
                                rostered.insert(slot);
                            }
                            None => unresolved += 1,
                        }
                    }
                }
                for slot in rostered {
                    sums.entry(slot).or_default();
                }
                if unresolved > 0 {
                    let why = format!("{unresolved} war roster entries name a country with no identity; their totals are not known");
                    out.unlisted.get_or_insert_with(|| why.clone());
                    out.warnings.push(why);
                }
            }
            Err(error) => {
                let why = format!("The war rosters are not recorded at this hour, so a country without a history is not known to have none: {error}");
                out.unlisted.get_or_insert_with(|| why.clone());
                out.warnings.push(why);
            }
        }
    }
    let unattributed = match (orphans, strays) {
        (0, 0) => None,
        (0, strays) => Some(format!("{strays} lend-lease histories name a partner with no identity; they are left out")),
        (orphans, _) => Some(format!("{orphans} lend-lease histories name a country with no identity; they are left out")),
    };
    if let Some(why) = &unattributed {
        out.warnings.push(why.clone());
        out.unlisted = Some(why.clone());
        if params.rank.is_some() {
            out.rank_missing = Some(why.clone());
        }
    }
    out.rows = ranked(sums, &slots, params);
    if out.rank_missing.is_some() && params.top.is_some() {
        out.rows.clear();
    }
    Ok(out)
}

impl AtHour for LendLeaseBalance {
    const KEY: &'static str = "lend-lease-balance";
    const TITLE: &'static str = "Lend-lease balance";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LendLeaseBalanceParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, holder, rank, top, countries, "countries[]"];
    type Reply = LendLeaseBalanceReply;
    type Context = ();

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.holder.as_ref().map_or(Ok(()), validate_country)?;
        params.countries.iter().flatten().try_for_each(validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LendLeaseBalanceParams::decl(), LendLeaseMeasure::decl(), LendLeaseBalanceReply::decl(), LendLeaseRow::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        params.holder.as_ref()
    }


    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;

    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        (table == TableId::Countries).then(|| dashboard_base::products::at_hour::field_columns(table, &["canonical_slot", "tag"]))
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::WarLendLeaseHistories, TableId::WarOwnedRelations, TableId::Countries, TableId::CountryLinks]
    }

    fn reads(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => {
                let mut reads = vec![Read::all(TableId::WarLendLeaseHistories), Read::all(TableId::Countries), Read::all(TableId::CountryLinks)];
                if params.holder.is_none() {
                    reads.push(Read::all(TableId::WarOwnedRelations));
                }
                reads
            }
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let named = params.holder.as_ref().map(|holder| at.holders.name(holder, at.hour));
        let holder = match (named.as_deref(), at.owner) {
            (Some(tag), Some(owner)) => Some((tag, owner.slot)),
            (Some(_), None) => return Err(dashboard_core::refused("Lend-lease balance country is unavailable at this hour")),
            (None, _) => None,
        };
        reply(frame, params, holder)
    }
}
