use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour},
    flows::{CountryFlowReply, FlowEdge, FlowEndpoint, FlowPart, FlowShape, build_flows},
    validate_country,
};
use dashboard_base::Source;
use replay_data::rows::{MilitaryAidRow, Recorded, TableId, TagHolderRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NOT_RECORDED: &str = "A sender's divisions were not recorded at this hour";
const UNLINKED: &str = "Which country these forces serve with, or come from, was not recorded at this hour";

pub struct VolunteerFlow;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolunteerFlowParams {
    pub branch_id: String,
    #[doc = " One country: the forces it hosts and sends by partner. Otherwise the links between `countries` (every country when empty)."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub subject: Option<Country>,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default)]
    pub between: bool,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub limit: Option<u32>,
}

fn parts() -> Vec<FlowPart> {
    [("volunteers", "Volunteers"), ("expeditionaries", "Expeditionary forces")]
        .into_iter()
        .enumerate()
        .map(|(at, (key, name))| FlowPart { key: key.to_owned(), name: Some(name.to_owned()), name_missing: None, ordinal: at as u32, icon: None })
        .collect()
}

pub fn aid(rows: &[MilitaryAidRow]) -> Vec<FlowEdge> {
    let slot = |slot: u32| (slot != TagHolderRow::NONE).then_some(slot);
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let part = match row.channel {
            MilitaryAidRow::VOLUNTEERS => 0,
            MilitaryAidRow::EXPEDITIONARIES => 1,
            _ => continue,
        };
        let amount = match row.divisions {
            Recorded::Value(value) if value <= 0 => continue,
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        out.push(FlowEdge { from: slot(row.sender), to: slot(row.host), part, amount });
    }
    out
}

impl AtHour for VolunteerFlow {
    const KEY: &'static str = "volunteer-flow";
    const TITLE: &'static str = "Volunteers & expeditionaries";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = VolunteerFlowParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, subject, countries, "countries[]", between, limit];
    type Reply = CountryFlowReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![VolunteerFlowParams::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().try_for_each(validate_country)?;
        params.subject.as_ref().map_or(Ok(()), validate_country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::MilitaryAid]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let empty = |missing: String| CountryFlowReply::not_recorded(missing, 0, Vec::new());
        if let Some(reason) = dashboard_text::scope::unread(TableId::MilitaryAid, &frame.tables[TableId::MilitaryAid.index()]) {
            return Ok(empty(reason));
        }
        let rows = match frame.complete::<tables::MilitaryAid>() {
            Ok(rows) => rows,
            Err(error) => return Ok(empty(error.to_string())),
        };
        let resolve = |country: &Country| at.holders.slot_at(country, at.hour);
        let subject = params.subject.as_ref().map(resolve);
        if let Some(None) = subject {
            return Ok(empty("The chosen country holds no tag at this hour".to_owned()));
        }
        let chosen: Option<HashSet<u32>> = (!params.countries.is_empty()).then(|| params.countries.iter().filter_map(resolve).collect());
        let shape = FlowShape {
            subject: subject.flatten(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            table: false,
            not_recorded: NOT_RECORDED,
            unlinked: UNLINKED,
            incomplete: None,
        };
        Ok(build_flows(&aid(rows), parts(), 0, &shape, &|slot| FlowEndpoint::at(at.holders, slot, at.hour, UNLINKED)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(sender: u32, host: u32, channel: u8, divisions: Recorded<i64>) -> MilitaryAidRow {
        MilitaryAidRow { sender, host, channel, divisions }
    }

    #[test]
    fn channels_become_parts_tallies_are_not_forces_and_unknown_senders_or_hosts_keep_their_forces() {
        let none = TagHolderRow::NONE;
        let rows = [
            row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3)),
            row(3, 2, MilitaryAidRow::EXPEDITIONARIES, Recorded::NotRecorded),
            row(1, 2, MilitaryAidRow::TALLIES, Recorded::Value(1)),
            row(3, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(0)),
            row(3, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded),
            row(none, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded),
        ];
        assert_eq!(aid(&rows), [
            FlowEdge { from: Some(1), to: Some(2), part: 0, amount: Some(3) },
            FlowEdge { from: Some(3), to: Some(2), part: 1, amount: None },
            FlowEdge { from: Some(3), to: None, part: 0, amount: None },
            FlowEdge { from: None, to: None, part: 0, amount: None },
        ]);
    }

    #[test]
    fn a_sender_whose_hosts_were_not_recorded_leaves_the_host_side_unproven_and_keeps_witnessed_forces() {
        let none = TagHolderRow::NONE;
        let rows = [row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3)), row(3, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded)];
        let shape = FlowShape { subject: Some(2), chosen: None, between: false, limit: None, table: false, not_recorded: NOT_RECORDED, unlinked: UNLINKED, incomplete: None };
        let describe = |slot: Option<u32>| FlowEndpoint { slot, tag: None, unknown: None };
        let reply = build_flows(&aid(&rows), parts(), 0, &shape, &describe);
        assert_eq!(reply.incoming.links.iter().map(|link| link.total.as_deref()).collect::<Vec<_>>(), [Some("3")]);
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (None, Some(UNLINKED)));
    }
}
