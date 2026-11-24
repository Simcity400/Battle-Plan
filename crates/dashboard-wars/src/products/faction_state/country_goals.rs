use super::details::{self, DetailContent, FactionDetailParams, FactionDetailReply, FactionDetails};
use super::goals::FactionGoalDefinition;
use dashboard_base::products::Country;
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Owner, Read, ReadsAt}}};
use dashboard_text::metadata::link_slot;
use replay_data::rows::{CountryLinkRow, FactionMemberRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct CountryFactionGoals;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryFactionGoalsParams {
    pub branch_id: String,
    pub country: Country,
    pub goals: GoalFilter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum GoalFilter {
    All,
    InProgress,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum GoalState {
    InProgress,
    Completing,
    Cancelling,
    Completed,
    Cancelled,
    #[doc = " A native state this version does not know; shown as such, never as in progress."]
    Unrecognised,
}

impl GoalState {
    pub(super) fn of(status: i32) -> Self {
        match status {
            0 => Self::InProgress,
            1 => Self::Completed,
            2 => Self::Cancelled,
            3 => Self::Completing,
            4 => Self::Cancelling,
            _ => Self::Unrecognised,
        }
    }

    pub(super) fn shown(self, filter: GoalFilter) -> bool {
        match filter {
            GoalFilter::All => true,
            GoalFilter::InProgress => matches!(self, Self::InProgress | Self::Completing | Self::Cancelling),
            GoalFilter::Completed => self == Self::Completed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryGoal {
    pub key: String,
    #[doc = " An index into the reply's `definitions`."]
    pub definition: u32,
    pub state: GoalState,
    #[doc = " The held goal's ordinal, the row of its live progress cell `goal:<ordinal>`; none for a goal in the completed or cancelled lists."]
    pub ordinal: Option<u32>,
    #[doc = " The native state, where it is `unrecognised`."]
    pub unrecognised_status: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFactionGoalsReply {
    #[doc = " The faction the country belongs to at the hour; none when it belongs to none, or when"]
    #[doc = " `membership_missing` says membership is not known."]
    pub faction: Option<String>,
    #[doc = " The goals the filter shows, held goals first in the game's order, then completed, then cancelled."]
    pub goals: Vec<CountryGoal>,
    #[doc = " The definitions the goals name, once each."]
    pub definitions: Vec<FactionGoalDefinition>,
    #[doc = " Why the faction's goal lists are not known at the hour; the goals are then empty."]
    pub goal_reason: Option<String>,
    pub membership_missing: Option<String>,
    pub warnings: Vec<String>,
}

pub(super) fn shown_goals(detail: FactionDetailReply) -> (Vec<CountryGoal>, Vec<FactionGoalDefinition>, Vec<String>) {
    let held = detail.goals.iter().map(|goal| {
        let state = GoalState::of(goal.status);
        CountryGoal { key: format!("goal:{}", goal.ordinal), definition: goal.definition, state, ordinal: Some(goal.ordinal), unrecognised_status: (state == GoalState::Unrecognised).then_some(goal.status) }
    });
    let listed = |list: &[u32], name: &'static str, state: GoalState| {
        list.iter().enumerate().map(move |(at, &definition)| CountryGoal { key: format!("{name}:{at}"), definition, state, ordinal: None, unrecognised_status: None }).collect::<Vec<_>>()
    };
    let goals = held.chain(listed(&detail.completed, "completed", GoalState::Completed)).chain(listed(&detail.cancelled, "cancelled", GoalState::Cancelled)).collect();
    (goals, detail.definitions, detail.warnings)
}

dashboard_base::reply_warnings!(CountryFactionGoalsReply);

const MEMBERSHIP: [TableId; 2] = [TableId::FactionMembers, TableId::CountryLinks];

fn blank(missing: Option<String>) -> CountryFactionGoalsReply {
    CountryFactionGoalsReply { faction: None, goals: Vec::new(), definitions: Vec::new(), goal_reason: None, membership_missing: missing, warnings: Vec::new() }
}

pub(super) fn faction_of(members: &[FactionMemberRow], links: &[CountryLinkRow], slot: u32) -> Option<u64> {
    members.iter().find(|member| member.faction != 0 && link_slot(links, member.country) == Some(slot)).map(|member| member.faction)
}

fn membership(frame: &DecodedFrame, owner: Owner) -> Result<Option<u64>, String> {
    let members = frame.complete::<tables::FactionMembers>().map_err(|why| format!("Faction memberships are not known at this hour: {why}"))?;
    let links = frame.complete::<tables::CountryLinks>().map_err(|why| format!("Country links are not known at this hour: {why}"))?;
    Ok(faction_of(members, links, owner.slot))
}

fn detail_params(params: &CountryFactionGoalsParams, faction: u64) -> FactionDetailParams {
    FactionDetailParams { branch_id: params.branch_id.clone(), faction: faction.to_string(), viewpoint: Some(params.country.clone()) }
}

impl AtHour for CountryFactionGoals {
    const KEY: &'static str = "country-faction-goals";
    const TITLE: &'static str = "Country faction goals";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = CountryFactionGoalsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, goals];
    type Reply = CountryFactionGoalsReply;
    type Context = DetailContent;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid faction branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryFactionGoalsParams::decl(), CountryFactionGoalsReply::decl(), GoalFilter::decl(), GoalState::decl(), CountryGoal::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables: Vec<TableId> = details::goal_reads(0).into_iter().map(|read| read.table).chain(super::goals::text_tables()).chain(MEMBERSHIP).collect();
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, Some(_)) => MEMBERSHIP.map(Read::all).into(),
            (1, Some(owner)) => membership(held, owner).ok().flatten().map(details::goal_reads).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if step != 2 {
            return Self::reads(params, owner, step, held);
        }
        let (Some(owner), Ok(cat)) = (owner, at.context.catalogue()) else { return Vec::new() };
        let Ok(Some(faction)) = membership(held, owner) else { return Vec::new() };
        details::text_reads(held, faction, cat, &details::goal_reads(faction))
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        <FactionDetails as AtHour>::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Faction goals country is unavailable at this hour"))?;
        let reply = match membership(frame, owner) {
            Err(why) => blank(Some(why)),
            Ok(None) => blank(None),
            Ok(Some(faction)) => {
                let detail = details::goals_only(frame, &detail_params(params, faction), faction, params.goals, at)?;
                let goal_reason = detail.goal_reason.clone();
                let (goals, definitions, warnings) = shown_goals(detail);
                CountryFactionGoalsReply { faction: Some(faction.to_string()), goals, definitions, goal_reason, membership_missing: None, warnings }
            }
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
