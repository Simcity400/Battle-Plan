//! Lend-lease balance: the game's cumulative lend-lease histories at an hour
//! (`WarLendLeaseHistories`, one row per country and partner with any history, keyed by raw
//! country ids), summed per canonical slot: every country's totals, or one country's record of
//! each partner. A country on the war rosters (`WarOwnedRelations`) with no row has sent and
//! received nothing; one off the rosters has no recorded history, which is not zero.
use super::super::LoadContext;
use crate::{
    Source,
    products::at_hour::{At, AtHour, Read},
};
use dashboard_base::products::validate_tag;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

/// Fixed-point denominator of every amount.
pub const SCALE: u32 = 100_000;

pub struct LendLeaseBalance;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LendLeaseBalanceParams {
    pub branch_id: String,
    /// One country's partners; every country's totals when absent.
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub holder: Option<String>,
}

/// One country's (or partner's) cumulative totals: industry sent, fuel sent, industry received,
/// fuel received.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseRow {
    pub slot: u32,
    /// The tag its slot's country holds at the hour; null where the recorded countries name none.
    pub tag: Option<String>,
    pub values: [String; 4],
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseBalanceReply {
    pub holder: Option<String>,
    pub scale: u32,
    /// Every country with a history or on the rosters, or the holder's partners with a history;
    /// in slot order.
    pub rows: Vec<LendLeaseRow>,
    /// Why the histories are not known at the hour.
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LendLeaseBalanceReply);

/// Raw country ids to canonical slots, and each slot's tag, as the hour's countries name them.
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

fn reply(frame: &DecodedFrame, holder: Option<(&str, u32)>) -> io::Result<LendLeaseBalanceReply> {
    let mut out = LendLeaseBalanceReply { holder: holder.map(|(tag, _)| tag.to_owned()), scale: SCALE, rows: Vec::new(), missing: None, warnings: Vec::new() };
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
                    out.warnings.push(format!("{unresolved} war roster entries name a country with no identity; their totals are not known"));
                }
            }
            Err(error) => out.warnings.push(format!("The war rosters are not recorded at this hour, so a country without a history is not known to have none: {error}")),
        }
    }
    if orphans > 0 {
        out.warnings.push(format!("{orphans} lend-lease histories name a country with no identity; they are left out"));
    }
    if strays > 0 {
        out.warnings.push(format!("{strays} lend-lease histories name a partner with no identity; they are left out"));
    }
    out.rows = sums
        .into_iter()
        .map(|(slot, values)| LendLeaseRow { slot, tag: slots.tags.get(&slot).cloned(), values: values.map(|value| value.to_string()) })
        .collect();
    Ok(out)
}

impl AtHour for LendLeaseBalance {
    const KEY: &'static str = "lend-lease-balance";
    const TITLE: &'static str = "Lend-lease balance";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LendLeaseBalanceParams;
    type Reply = LendLeaseBalanceReply;
    type Context = ();

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.holder.as_deref().map_or(Ok(()), validate_tag)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LendLeaseBalanceParams::decl(), LendLeaseBalanceReply::decl(), LendLeaseRow::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        params.holder.as_deref()
    }

    fn uses_sample(_table: TableId) -> bool { false }

    const CLOCK_DEPENDENT: bool = false;

    fn projection(_params: &Self::Params, table: TableId) -> Option<Vec<usize>> {
        (table == TableId::Countries).then(|| dashboard_base::products::at_hour::field_columns(table, &["canonical_slot", "tag"]))
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::WarLendLeaseHistories, TableId::WarOwnedRelations, TableId::Countries, TableId::CountryLinks]
    }

    fn reads(params: &Self::Params, _owner: Option<crate::products::at_hour::Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
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
        let holder = match (params.holder.as_deref(), at.owner) {
            (Some(tag), Some(owner)) => Some((tag, owner.slot)),
            (Some(_), None) => return Err(dashboard_core::refused("Lend-lease balance country is unavailable at this hour")),
            (None, _) => None,
        };
        reply(frame, holder)
    }
}
