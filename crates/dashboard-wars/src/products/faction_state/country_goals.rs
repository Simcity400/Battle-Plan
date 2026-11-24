use super::details::{self, DetailContent, FactionDetailParams, FactionDetailReply, FactionDetails};
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFactionGoalsReply {
    #[doc = " The faction the country belongs to at the hour, as `faction-details` answers it; none when it"]
    #[doc = " belongs to none, or when `membership_missing` says membership is not known."]
    pub faction: Option<FactionDetailReply>,
    pub membership_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CountryFactionGoalsReply);

const MEMBERSHIP: [TableId; 2] = [TableId::FactionMembers, TableId::CountryLinks];

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
    type Params = CountryFactionGoalsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = CountryFactionGoalsReply;
    type Context = DetailContent;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid faction branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryFactionGoalsParams::decl(), CountryFactionGoalsReply::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables: Vec<TableId> = details::faction_reads(0).into_iter().map(|read| read.table).chain(super::goals::text_tables()).chain(MEMBERSHIP).collect();
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, Some(_)) => MEMBERSHIP.map(Read::all).into(),
            (1, Some(owner)) => membership(held, owner).ok().flatten().map(details::faction_reads).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if step != 2 {
            return Self::reads(params, owner, step, held);
        }
        let (Some(owner), Ok(cat)) = (owner, at.context.catalogue()) else { return Vec::new() };
        let Ok(Some(faction)) = membership(held, owner) else { return Vec::new() };
        details::text_reads(held, faction, cat)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        <FactionDetails as AtHour>::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Faction goals country is unavailable at this hour"))?;
        let reply = match membership(frame, owner) {
            Err(why) => CountryFactionGoalsReply { faction: None, membership_missing: Some(why), warnings: Vec::new() },
            Ok(None) => CountryFactionGoalsReply { faction: None, membership_missing: None, warnings: Vec::new() },
            Ok(Some(faction)) => {
                let detail = details::reply(frame, &detail_params(params, faction), faction, at)?;
                CountryFactionGoalsReply { faction: Some(detail), membership_missing: None, warnings: Vec::new() }
            }
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
