use crate::products::country_people;
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour, Owner, Read, ReadsAt},
        check_reply_budget, validate_country,
    },
};
use dashboard_query::{
    coverage::{Witness, population, witnessed},
    identity::Holder,
};
use replay_data::rows::{CountryCounterInputRow, PoliticsRow, Recorded, TableId, tables};
use replay_frame::frame::{DecodedFrame, Keys, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextElectionsParams {
    pub branch_id: String,
    #[doc = " When given, reads only these countries' election schedules at this hour."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ElectionEntry {
    pub key: String,
    pub tag: String,
    #[doc = " Signed months; null when not recorded."]
    pub interval: Option<i32>,
    #[doc = " Next election hour; null when not recorded."]
    pub next: Option<i32>,
    #[doc = " Term start (last election hour) when it is a date; null when not recorded."]
    pub last: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NextElectionsReply {
    pub branch_id: String,
    #[doc = " False when the politics table was not recorded at this hour; `warnings` says why."]
    pub recorded: bool,
    pub rows: Vec<ElectionEntry>,
    #[doc = " Asked or listed countries whose schedule or eligibility was not recorded at this hour: `rows` may miss them."]
    pub unknown: u32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NextElectionsReply);

pub struct NextElections;

impl AtHour for NextElections {
    const KEY: &'static str = "next-elections";
    const TITLE: &'static str = "Next elections";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK_DEPENDENT: bool = false;
    const REPLY_NAMES_NO_HOUR: bool = true;
    type Params = NextElectionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]"];
    type Reply = NextElectionsReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NextElectionsParams::decl(), ElectionEntry::decl(), NextElectionsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid elections branch")?;
        for country in params.countries.iter().flatten() {
            validate_country(country)?;
        }
        Ok(())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Politics, TableId::CountryCounterInputs]
    }

    fn reads_in(params: &Self::Params, _owner: Option<Owner>, step: usize, _held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if step != 0 {
            return Vec::new();
        }
        match &params.countries {
            None => vec![Read::all(TableId::Politics), Read::all(TableId::CountryCounterInputs)],
            Some(countries) => {
                let keys: Vec<u128> = countries
                    .iter()
                    .filter_map(|country| match at.holders.at(country, at.hour).map(|run| run.holder) {
                        Some(Holder::Found { id, .. }) => Some(u128::from(id)),
                        _ => None,
                    })
                    .collect();
                vec![Read::these(TableId::Politics, keys.clone()), Read::these(TableId::CountryCounterInputs, keys)]
            }
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let reply = read(frame, params, at.holders, at.hour)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn recorded<T: Copy>(value: &Recorded<T>) -> Option<T> {
    value.require("election operand").ok().copied()
}

pub fn read(frame: &DecodedFrame, params: &NextElectionsParams, holders: &dashboard_query::identity::Holders, hour: i32) -> io::Result<NextElectionsReply> {
    let mut reply = NextElectionsReply {
        branch_id: params.branch_id.clone(),
        recorded: false,
        rows: Vec::new(),
        unknown: 0,
        warnings: Vec::new(),
    };
    let politics = match &params.countries {
        Some(_) => {
            let view: &TableView = &frame.tables[TableId::Politics.index()];
            match (view, witnessed::<tables::Politics>(frame, 0)) {
                (TableView::Unavailable { .. } | TableView::NotRecorded { .. }, Witness::NotRecorded(reason)) => Err(reason),
                _ => Ok(None),
            }
        }
        None => population::<tables::Politics>(frame, &Keys::All).map(Some),
    };
    let politics = match politics {
        Ok(politics) => politics,
        Err(reason) => {
            reply.warnings.push(reason);
            return Ok(reply);
        }
    };
    reply.recorded = true;
    let counters = population::<tables::CountryCounterInputs>(frame, &Keys::All).ok();
    let owned_of = |country_id: u32| -> Option<bool> {
        let row: Option<&CountryCounterInputRow> = match counters {
            Some(rows) => rows.binary_search_by_key(&country_id, |row| row.country).ok().map(|at| &rows[at]),
            None => witnessed::<tables::CountryCounterInputs>(frame, u128::from(country_id)).row(),
        };
        row.map(|row| row.owned_state_count > 0)
    };
    let mut unresolved = 0_u64;
    let mut unwitnessed = 0_u32;
    let mut inspect = |slot: u32, tag: &str, row: &PoliticsRow| {
        let has = recorded(&row.has_elections);
        let exists = owned_of(row.country);
        let eligible = match (has, exists) {
            (Some(false), _) | (Some(true), Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            (Some(true), None) | (None, _) => None,
        };
        match eligible {
            Some(false) => {}
            None => reply.unknown = reply.unknown.saturating_add(1),
            Some(true) => {
                let interval = recorded(&row.election_interval);
                let next = country_people::next_election_date(row);
                let last = (!engine_native_politics::cpolitical_status::needs_election_cache(row.last_election)).then_some(row.last_election);
                reply.rows.push(ElectionEntry { key: slot.to_string(), tag: tag.to_owned(), interval, next, last });
            }
        }
    };
    match (&params.countries, politics) {
        (Some(asked), _) => {
            let mut seen = BTreeSet::new();
            for country in asked {
                let Some(Holder::Found { slot, id }) = holders.at(country, hour).map(|run| run.holder) else {
                    continue;
                };
                if !seen.insert((slot, id)) {
                    continue;
                }
                let Some(tag) = holders.tag_at(country, hour) else {
                    unresolved += 1;
                    continue;
                };
                match witnessed::<tables::Politics>(frame, u128::from(id)) {
                    Witness::Row(row) => inspect(slot, tag, row),
                    Witness::Absent => {}
                    Witness::NotRecorded(_) => unwitnessed = unwitnessed.saturating_add(1),
                }
            }
        }
        (None, politics) => {
            let mut by_id: HashMap<u32, (u32, &str)> = HashMap::new();
            for (tag, runs) in &holders.tags {
                let at = runs.partition_point(|run| run.last < hour);
                if let Some(run) = runs.get(at).filter(|run| run.first <= hour)
                    && let Holder::Found { slot, id } = run.holder
                {
                    by_id.entry(id).or_insert((slot, tag.as_str()));
                }
            }
            for row in politics.into_iter().flatten() {
                let Some(&(slot, tag)) = by_id.get(&row.country) else {
                    unresolved += 1;
                    continue;
                };
                inspect(slot, tag, row);
            }
        }
    }
    reply.unknown = reply.unknown.saturating_add(unwitnessed);
    reply.rows.sort_by(|a, b| (a.next.is_none(), a.next, &a.tag, &a.key).cmp(&(b.next.is_none(), b.next, &b.tag, &b.key)));
    if unresolved != 0 {
        reply.warnings.push(format!("{unresolved} refused election schedules: the country has no identity"));
    }
    Ok(reply)
}
