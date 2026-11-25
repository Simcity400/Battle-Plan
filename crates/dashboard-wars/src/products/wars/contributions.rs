
use super::{
    Resolved, SlotNames, WarContributionGroup,
    inputs::{Hour, State},
    participation::{contributions, factors},
};
use dashboard_base::{Source, catalogue};
use dashboard_base::products::{LoadContext, at_hour::{At, AtHour}};
use dashboard_base::products::check_reply_budget;
use replay_data::{
    rows::{TableId, tables},
    war_participation::Factors,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct WarContributions;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarContributionsParams {
    pub branch_id: String,
    #[doc = " None follows the recording client's playing or observer country."]
    pub viewpoint: Option<dashboard_base::products::Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub war: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarContributionsReply {
    pub branch_id: String,
    pub hour: i32,
    #[doc = " An overview table was recorded at the hour, even in part."]
    pub available: bool,
    #[doc = " Each native group the viewpoint sees, in the overview's order."]
    pub contributions: Vec<WarContributionGroup>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WarContributionsReply);

const TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::WarOwnedRelations,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::FactionMembers,
    TableId::LocalPlayer,
    TableId::WarParticipation,
    TableId::WarLendLeaseHistories,
];

pub struct ContributionContent {
    factors: Result<Factors, String>,
    hours: Vec<(i32, Option<i32>)>,
}

pub(super) fn contribution_hours(wars: &super::WarsReply, selection: &str) -> Vec<(i32, Option<i32>)> {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    let saved = wars.wars.iter().find(|war| war.id == selection);
    let mut changes = BTreeMap::<i32, Vec<_>>::new();
    for (index, war) in wars.wars.iter().enumerate().filter(|(_, war)| saved.is_none_or(|saved| war.id == saved.id)) {
        let order = Reverse(index);
        for observation in &war.observations {
            let size = observation.viewpoint_side.iter().chain(&observation.opposing_side).collect::<HashSet<_>>().len();
            changes.entry(observation.first_hour).or_default().push((index, (true, size, war.last_hour, order), None));
            if let Some(next) = observation.last_hour.checked_add(1) {
                let ended = observation.last_hour == war.last_hour;
                let read = if ended { war.last_hour } else { war.first_hour };
                changes.entry(next).or_default().push((index, (false, if ended { size } else { 0 }, war.last_hour, order), Some(read)));
            }
        }
    }
    let mut active = BTreeSet::new();
    let mut previous = HashMap::new();
    let mut hours = Vec::new();
    for (hour, edits) in changes {
        for (index, rank, read) in edits {
            if let Some(old) = previous.insert(index, (rank, read)) { active.remove(&old); }
            active.insert((rank, read));
        }
        let read = active.last().and_then(|(_, read)| *read);
        if hours.last().map_or(read.is_some(), |&(_, was)| was != read) {
            hours.push((hour, read));
        }
    }
    hours
}

fn available(frame: &DecodedFrame) -> bool {
    [TableId::WarOwnedRelations, TableId::WarOverviewCountries, TableId::WarNameRules]
        .iter()
        .any(|table| !matches!(&*frame.tables[table.index()], TableView::Unavailable { .. }))
}

pub(crate) fn groups(frame: &DecodedFrame, holders: &dashboard_query::identity::Holders, view: Option<&dashboard_base::products::Country>, factors: Result<Factors, String>) -> Result<Vec<WarContributionGroup>, String> {
    let state = State::of_frame(frame)?;
    let links: HashMap<u32, u32> = frame.complete::<tables::CountryLinks>().map_err(|e| e.to_string())?.iter().map(|r| (r.id, r.canonical_slot)).collect();
    let names = SlotNames::new(holders);
    let link = |raw: u32| links.get(&raw).copied();
    let hour = Hour::build(&state, &Resolved { links: &link, names: &names, holders, hour: frame.hour }, view)?;
    let blobs = hour.world.blobs(hour.view);
    Ok(contributions(&blobs, (&hour.tags, &links), &|a, b| hour.world.active(a, b), frame, factors))
}

impl AtHour for WarContributions {
    const KEY: &'static str = "war-contributions";
    const TITLE: &'static str = "War contributions";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = WarContributionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, viewpoint, war];
    type Reply = WarContributionsReply;
    type Context = ContributionContent;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid war contributions branch")?;
        params.viewpoint.as_ref().map_or(Ok(()), dashboard_base::products::validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            WarContributionsParams::decl(),
            super::WarSide::decl(),
            super::WarContributionCountry::decl(),
            super::WarMeasureKey::decl(),
            super::WarMeasureMember::decl(),
            super::WarMeasureSide::decl(),
            super::WarMeasure::decl(),
            WarContributionGroup::decl(),
            WarContributionsReply::decl(),
        ]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let held = load
            .content()
            .and_then(|options| catalogue::catalogue(source, options))
            .map_err(|why| why.to_string())
            .and_then(|cat| factors(Some(&*cat)));
        Ok(ContributionContent { factors: held, hours: Vec::new() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let mut context = Self::context(source, load)?;
        if let Some(selection) = &params.war {
            let wars = super::walked(source, super::branch(&params.branch_id)?, params.viewpoint.as_ref(), super::catalogue_of(source, load)?, &AtomicBool::new(false))?;
            context.hours = contribution_hours(&wars, selection);
        }
        Ok(context)
    }

    fn read_hour(context: &Self::Context, hour: i32) -> i32 {
        let end = context.hours.partition_point(|&(first, _)| first <= hour);
        context.hours[..end].last().and_then(|&(_, read)| read).unwrap_or(hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut reply = WarContributionsReply { branch_id: params.branch_id.clone(), hour: at.hour, available: available(frame), contributions: Vec::new(), warnings: Vec::new() };
        match groups(frame, at.holders, params.viewpoint.as_ref(), at.context.factors.clone()) {
            Ok(groups) => reply.contributions = groups,
            Err(reason) => reply.warnings.push(reason),
        }
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
