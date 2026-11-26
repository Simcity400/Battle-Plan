use crate::products::country_people;
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour, Edits, Held, Owner, Read, ReadsAt, before, field_columns},
        check_reply_budget, validate_country,
    },
};
use dashboard_query::{
    coverage::{Witness, population, witnessed},
    identity::Holder,
    series::ranked::GroupSelection,
};
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::{CountryCounterInputRow, PoliticsRow, Recorded, TableId, TableRows, tables};
use replay_frame::frame::{DecodedFrame, Keys, SharedView, TableView};
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
    #[doc = " The group whose members at this hour are read; never with `countries`."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
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
    #[doc = " Why the asked group's members are not known at this hour; `rows` is then empty."]
    pub refused: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NextElectionsReply);

pub struct NextElections;

impl AtHour for NextElections {
    const KEY: &'static str = "next-elections";
    const TITLE: &'static str = "Next elections";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = NextElectionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, group, "countries[]"];
    type Reply = NextElectionsReply;
    type Context = GroupsAt;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NextElectionsParams::decl(), ElectionEntry::decl(), NextElectionsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid elections branch")?;
        for country in params.countries.iter().flatten() {
            validate_country(country)?;
        }
        if params.group.is_some() && params.countries.is_some() {
            return Err("Next elections asks a group or countries, never both");
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
        match asked(params, at.context, at.hour) {
            Ok(Ok(None)) => vec![Read::all(TableId::Politics), Read::all(TableId::CountryCounterInputs)],
            Err(_) | Ok(Err(_)) => Vec::new(),
            Ok(Ok(Some(countries))) => {
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
        Ok(GroupsAt::default())
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        let fields: &[&str] = match table {
            TableId::Politics => &["last_election", "has_elections", "election_interval", "next_election_cache"],
            TableId::CountryCounterInputs => &["owned_state_count"],
            _ => return None,
        };
        Some(field_columns(table, fields))
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, frame: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let through = if stage == 0 { at.context.valid_through(params.group.as_ref(), at.hour).min(before(at.holders.next_change(at.hour))) } else { i32::MAX };
        (Self::reads_in(params, owner, stage, frame, at), through)
    }

    fn plan_changes(_: usize, _: &Edits, _: &DecodedFrame, _: &ReadsAt<'_, Self::Context>) -> bool { false }

    fn compares_previous(table: TableId) -> bool { matches!(table, TableId::Politics | TableId::CountryCounterInputs) }

    fn advance(frame: &DecodedFrame, _: &Self::Params, _: &At<'_, Self::Context>, edits: &Edits, _: &dashboard_follow::live::Live, _: &mut Held<'_>) -> io::Result<bool> {
        if !edits.replaced.is_empty() { return Ok(false); }
        let prior = |id: TableId| &edits.previous[&id];
        let now = |id: TableId| &frame.tables[id.index()];
        Ok(edits.tables.values().flat_map(|keys| keys.keys().copied()).all(|id| {
            schedule_at(prior(TableId::Politics), prior(TableId::CountryCounterInputs), id as u32)
                == schedule_at(now(TableId::Politics), now(TableId::CountryCounterInputs), id as u32)
        }))
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn valid_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        before(at.holders.next_change(frame.hour)).min(at.context.valid_through(params.group.as_ref(), at.hour))
    }

    fn valid_reply_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, _: &Self::Reply, _: bool) -> i32 {
        Self::valid_through(frame, params, at)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let reply = match asked(params, at.context, at.hour)? {
            Ok(countries) => read(frame, &params.branch_id, countries.as_deref(), at.holders, at.hour)?,
            Err(refused) => NextElectionsReply { branch_id: params.branch_id.clone(), recorded: true, rows: Vec::new(), unknown: 0, refused: Some(refused), warnings: Vec::new() },
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn asked(params: &NextElectionsParams, groups: &GroupsAt, hour: i32) -> io::Result<Result<Option<Vec<Country>>, String>> {
    let Some(group) = &params.group else { return Ok(Ok(params.countries.clone())) };
    let members = groups.members(group, hour)?;
    Ok(match members.refused {
        Some(refused) => Err(refused),
        None => Ok(Some(members.tags.into_iter().map(Country::from).collect())),
    })
}

fn recorded<T: Copy>(value: &Recorded<T>) -> Option<T> {
    value.require("election operand").ok().copied()
}

#[derive(PartialEq)]
enum Schedule {
    Ineligible,
    Unknown,
    Election { interval: Option<i32>, next: Option<i32>, last: Option<i32> },
}

fn schedule(row: &PoliticsRow, owned: Option<bool>) -> Schedule {
    match (recorded(&row.has_elections), owned) {
        (Some(false), _) | (Some(true), Some(false)) => Schedule::Ineligible,
        (Some(true), None) | (None, _) => Schedule::Unknown,
        (Some(true), Some(true)) => Schedule::Election {
            interval: recorded(&row.election_interval),
            next: country_people::next_election_date(row),
            last: (!engine_native_politics::cpolitical_status::needs_election_cache(row.last_election)).then_some(row.last_election),
        },
    }
}

fn schedule_at(politics: &SharedView, counters: &SharedView, id: u32) -> Option<Schedule> {
    let TableView::Complete(TableRows::Politics(rows)) = politics.held() else { return None };
    let row = &rows[rows.binary_search_by_key(&id, |row| row.country).ok()?];
    let owned = match counters.held() {
        TableView::Complete(TableRows::CountryCounterInputs(rows)) => rows.binary_search_by_key(&id, |row| row.country).ok().map(|at| rows[at].owned_state_count > 0),
        _ => None,
    };
    Some(schedule(row, owned))
}

fn read(frame: &DecodedFrame, branch_id: &str, asked: Option<&[Country]>, holders: &dashboard_query::identity::Holders, hour: i32) -> io::Result<NextElectionsReply> {
    let mut reply = NextElectionsReply {
        branch_id: branch_id.to_owned(),
        recorded: false,
        rows: Vec::new(),
        unknown: 0,
        refused: None,
        warnings: Vec::new(),
    };
    let politics = match asked {
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
    let mut holder_unknown = None;
    let mut unwitnessed = 0_u32;
    let mut inspect = |slot: u32, tag: &str, row: &PoliticsRow| {
        match schedule(row, owned_of(row.country)) {
            Schedule::Ineligible => {}
            Schedule::Unknown => reply.unknown = reply.unknown.saturating_add(1),
            Schedule::Election { interval, next, last } => {
                reply.rows.push(ElectionEntry { key: slot.to_string(), tag: tag.to_owned(), interval, next, last });
            }
        }
    };
    match (asked, politics) {
        (Some(asked), _) => {
            let mut seen = BTreeSet::new();
            for country in asked {
                let Some(Holder::Found { slot, id }) = holders.at(country, hour).map(|run| run.holder) else {
                    if holder_unknown.is_none() {
                        holder_unknown = holders.not_known(country, hour);
                    }
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
    reply.refused = reply.refused.take().or(holder_unknown);
    reply.unknown = reply.unknown.saturating_add(unwitnessed);
    reply.rows.sort_by(|a, b| (a.next.is_none(), a.next, &a.tag, &a.key).cmp(&(b.next.is_none(), b.next, &b.tag, &b.key)));
    if unresolved != 0 {
        reply.warnings.push(format!("{unresolved} refused election schedules: the country has no identity"));
    }
    Ok(reply)
}
