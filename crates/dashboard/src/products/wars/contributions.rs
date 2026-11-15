//! `war-contributions`: the viewpoint's native groups at an hour and every member's
//! contributions to its side, rows at an hour ([`AtHour`]): asked for an hour (a hover, a pinned
//! hour, an ended war's last hour) and followed as a panel at the head.

use super::{
    Resolved, SlotNames, WarContributionGroup,
    inputs::{Hour, State},
    participation::{contributions, factors},
};
use crate::{Source, catalogue};
use crate::products::{
    LoadContext,
    at_hour::{At, AtHour},
};
use dashboard_base::products::{check_reply_budget, validate_tag};
use replay_data::{
    rows::{TableId, tables},
    war_participation::Factors,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io};
use ts_rs::TS;

pub struct WarContributions;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarContributionsParams {
    pub branch_id: String,
    /// None follows the recording client's playing or observer country.
    pub viewpoint_tag: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarContributionsReply {
    pub branch_id: String,
    pub hour: i32,
    /// An overview table was recorded at the hour, even in part.
    pub available: bool,
    /// Each native group the viewpoint sees, in the overview's order.
    pub contributions: Vec<WarContributionGroup>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WarContributionsReply);

/// What the overview and the participation scores read at the hour.
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

/// The lend-lease factors of the recording's content, or why they are not known.
pub struct ContributionContent {
    factors: Result<Factors, String>,
}

/// Whether an overview operand table was recorded at `frame`'s hour, even in part.
fn available(frame: &DecodedFrame) -> bool {
    [TableId::WarOwnedRelations, TableId::WarOverviewCountries, TableId::WarNameRules]
        .iter()
        .any(|table| !matches!(&*frame.tables[table.index()], TableView::Unavailable { .. }))
}

/// The groups and contributions `view` sees in `frame`, its countries named by `holders`; why
/// none where the hour cannot be read.
pub(crate) fn groups(frame: &DecodedFrame, holders: &dashboard_query::identity::Holders, view: Option<&str>, factors: Result<Factors, String>) -> Result<Vec<WarContributionGroup>, String> {
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
    type Params = WarContributionsParams;
    type Reply = WarContributionsReply;
    type Context = ContributionContent;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid war contributions branch")?;
        params.viewpoint_tag.as_deref().map_or(Ok(()), validate_tag)
    }

    fn types() -> Vec<String> {
        vec![
            WarContributionsParams::decl(),
            super::WarSide::decl(),
            super::WarContributionCountry::decl(),
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
        Ok(ContributionContent { factors: held })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut reply = WarContributionsReply { branch_id: params.branch_id.clone(), hour: at.hour, available: available(frame), contributions: Vec::new(), warnings: Vec::new() };
        match groups(frame, at.holders, params.viewpoint_tag.as_deref(), at.context.factors.clone()) {
            Ok(groups) => reply.contributions = groups,
            Err(reason) => reply.warnings.push(reason),
        }
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
