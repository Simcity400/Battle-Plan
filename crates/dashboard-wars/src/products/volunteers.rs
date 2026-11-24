use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour},
    flows::{CountryFlowReply, FlowEdge, FlowPart, FlowShape, build_flows},
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
const LIST_UNKNOWN: &str = "Some senders' lists of hosts were not recorded at this hour; their forces are left out";
const WORLD_UNKNOWN: &str = "Military aid was not recorded for every sender at this hour";

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

pub struct Aid {
    pub edges: Vec<FlowEdge>,
    pub unknown_lists: bool,
    pub world_unknown: bool,
}

pub fn aid(rows: &[MilitaryAidRow], tag: impl Fn(u32) -> Option<String>) -> Aid {
    let mut out = Aid { edges: Vec::with_capacity(rows.len()), unknown_lists: false, world_unknown: false };
    for row in rows {
        let part = match row.channel {
            MilitaryAidRow::VOLUNTEERS => 0,
            MilitaryAidRow::EXPEDITIONARIES => 1,
            _ => continue,
        };
        if row.sender == TagHolderRow::NONE {
            out.world_unknown = true;
            continue;
        }
        if row.host == TagHolderRow::NONE {
            out.unknown_lists = true;
            continue;
        }
        let (Some(from), Some(to)) = (tag(row.sender), tag(row.host)) else {
            out.unknown_lists = true;
            continue;
        };
        let amount = match row.divisions {
            Recorded::Value(value) if value <= 0 => continue,
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        out.edges.push(FlowEdge { from, to, part, amount });
    }
    out
}

impl AtHour for VolunteerFlow {
    const KEY: &'static str = "volunteer-flow";
    const TITLE: &'static str = "Volunteers & expeditionaries";
    const CONTENT_DEPENDENT: bool = false;
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
        let aid = aid(rows, |slot| at.holders.tag_at(&Country::Slot(slot), at.hour).map(str::to_owned));
        if aid.world_unknown {
            return Ok(empty(WORLD_UNKNOWN.to_owned()));
        }
        let resolve = |country: &Country| at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned));
        let subject = params.subject.as_ref().and_then(resolve);
        if params.subject.is_some() && subject.is_none() {
            return Ok(empty("The chosen country holds no tag at this hour".to_owned()));
        }
        let chosen: Option<HashSet<String>> = (!params.countries.is_empty()).then(|| params.countries.iter().filter_map(resolve).collect());
        let shape = FlowShape {
            subject: subject.as_deref(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            table: false,
            not_recorded: NOT_RECORDED,
        };
        let mut reply = build_flows(&aid.edges, parts(), 0, &shape);
        reply.partial = aid.unknown_lists.then(|| LIST_UNKNOWN.to_owned());
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(sender: u32, host: u32, channel: u8, divisions: Recorded<i64>) -> MilitaryAidRow {
        MilitaryAidRow { sender, host, channel, divisions }
    }

    fn tag(slot: u32) -> Option<String> {
        Some(match slot {
            1 => "SOV",
            2 => "SPR",
            3 => "ITA",
            _ => return None,
        }.to_owned())
    }

    #[test]
    fn channels_become_parts_tallies_are_not_forces_and_unknown_lists_are_said() {
        let none = TagHolderRow::NONE;
        let rows = [
            row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3)),
            row(3, 2, MilitaryAidRow::EXPEDITIONARIES, Recorded::NotRecorded),
            row(1, 2, MilitaryAidRow::TALLIES, Recorded::Value(1)),
            row(3, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(0)),
            row(3, none, MilitaryAidRow::VOLUNTEERS, Recorded::Value(1)),
        ];
        let out = aid(&rows, tag);
        assert_eq!(out.edges, [
            FlowEdge { from: "SOV".into(), to: "SPR".into(), part: 0, amount: Some(3) },
            FlowEdge { from: "ITA".into(), to: "SPR".into(), part: 1, amount: None },
        ]);
        assert!(out.unknown_lists && !out.world_unknown);
        assert!(aid(&[row(none, none, MilitaryAidRow::VOLUNTEERS, Recorded::Value(0))], tag).world_unknown);
    }
}
