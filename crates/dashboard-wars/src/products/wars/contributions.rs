
use super::{
    Resolved, SlotNames, WarContributionGroup,
    inputs::{Hour, State},
    participation::{contributions, factors},
};
use dashboard_base::{Source, catalogue};
use dashboard_base::products::{LoadContext, at_hour::{At, AtHour, Edits, ReadsAt, at_hour, before, field_columns}};
use dashboard_base::products::check_reply_budget;
use replay_data::{
    rows::{TableId, tables},
    war_participation::Factors,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io};
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarContributionsReply {
    pub branch_id: String,
    pub hour: i32,
    #[doc = " The group list was read at the hour, even in part."]
    pub available: bool,
    #[doc = " Each native group the viewpoint sees, in the overview's order."]
    pub contributions: Vec<WarContributionGroup>,
    pub missing: Option<String>,
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
    const DATE_HELD_REPLY: Option<fn(&mut serde_json::Value, i32)> = Some(at_hour);
    type Params = WarContributionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, viewpoint];
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

    fn columns(table: TableId) -> Option<Vec<usize>> {
        match table {
            TableId::CountryLinks => Some(field_columns(table, &["canonical_slot"])),
            TableId::WarRelations => Some(Vec::new()),
            TableId::WarOwnedRelations => Some(field_columns(table, &["first_country", "second_country", "retired"])),
            TableId::WarOverviewCountries => Some(Vec::new()),
            TableId::WarNameRules => Some(field_columns(table, &["viewpoint"])),
            TableId::FactionMembers => Some(field_columns(table, &["faction"])),
            TableId::LocalPlayer => Some(field_columns(table, &["country", "observer_country"])),
            TableId::WarParticipation => Some(field_columns(table, &[
                "equipment_damage",
                "province_capture",
                "air_damage",
                "strategic_bombing",
                "sunk_ships",
                "sunk_convoys",
                "casualties",
                "unknown_casualties",
                "lend_lease_sent",
                "lend_lease_received",
            ])),
            TableId::WarLendLeaseHistories => None,
            _ => None,
        }
    }

    fn plan_changes(_stage: usize, _edits: &Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        false
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        before(at.holders.next_change(frame.hour))
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
        let (contributions, missing) = match groups(frame, at.holders, params.viewpoint.as_ref(), at.context.factors.clone()) {
            Ok(groups) => (groups, None),
            Err(reason) => (Vec::new(), Some(reason)),
        };
        let available = missing.is_none();
        let reply = WarContributionsReply { branch_id: params.branch_id.clone(), hour: at.hour, available, contributions, missing, warnings: Vec::new() };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    #[test]
    fn contribution_reply_distinguishes_peace_from_an_unread_grouping_input() {
        let mut frame = crate::products::war_fixtures::recorded(100);
        for table in [TableId::WarRelations, TableId::WarOwnedRelations] {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        let held = [("ONE", 1), ("TWO", 2), ("THREE", 3)].into_iter().map(|(tag, slot)| (tag.to_owned(), vec![
            dashboard_query::identity::HolderRun { first: 100, last: 100, holder: dashboard_query::identity::Holder::Found { slot, id: slot }, starts_segment: true }
        ])).collect();
        let holders = dashboard_query::identity::Holders::new(held, Vec::new());
        let context = ContributionContent { factors: Err("outside the contribution fixture".into()) };
        let at = At { owner: None, holders: &holders, hour: 100, context: &context, cancelled: &false.into(), live: &Default::default() };
        let params = WarContributionsParams { branch_id: "1".into(), viewpoint: None };
        let empty = WarContributions::reply(&frame, &params, &at).unwrap();
        assert!(empty.available && empty.contributions.is_empty());
        assert_eq!(serde_json::to_value(&empty).unwrap().get("missing"), Some(&serde_json::Value::Null));
        *frame.tables[TableId::WarOverviewCountries.index()] = TableView::Unavailable { reason: "fixture overview scores failed".into() };
        let empty = WarContributions::reply(&frame, &params, &at).unwrap();
        assert!(empty.available && empty.contributions.is_empty() && empty.missing.is_none());
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "fixture country links failed".into() };
        let unread = WarContributions::reply(&frame, &params, &at).unwrap();
        assert!(!unread.available && unread.contributions.is_empty());
        assert!(serde_json::to_value(&unread).unwrap()["missing"].as_str().is_some_and(|reason| reason.contains("fixture country links failed")));
    }
}
