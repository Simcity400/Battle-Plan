use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour},
    flows::{CountryFlowReply, FlowEdge, FlowEnd, FlowEndpoint, FlowGap, FlowPart, FlowShape, GapEnd, build_flows},
    validate_country,
};
use dashboard_base::Source;
use dashboard_query::coverage::witnessed_rows;
use dashboard_query::series::ranked::GroupSelection;
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::{MilitaryAidRow, Recorded, TableId, TagHolderRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
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
    #[doc = " A group: its members at the hour (or every member so far), resolved in Rust; replaces `countries`."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub limit: Option<u32>,
    #[doc = " An explicit top partners per side: exactly this many named and the rest folded, where the order is proven at the hour."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
}

fn parts() -> Vec<FlowPart> {
    [("volunteers", "Volunteers"), ("expeditionaries", "Expeditionary forces")]
        .into_iter()
        .enumerate()
        .map(|(at, (key, name))| FlowPart { key: key.to_owned(), name: Some(name.to_owned()), name_missing: None, ordinal: at as u32, icon: None })
        .collect()
}

pub fn aid(rows: &[MilitaryAidRow]) -> (Vec<FlowEdge>, Vec<FlowGap>) {
    let none = TagHolderRow::NONE;
    let (mut edges, mut gaps) = (Vec::with_capacity(rows.len()), Vec::new());
    for row in rows {
        let part = match row.channel {
            MilitaryAidRow::VOLUNTEERS => 0,
            MilitaryAidRow::EXPEDITIONARIES => 1,
            _ => continue,
        };
        if row.sender == none || row.host == none {
            let (from, reason) = if row.sender == none { (GapEnd::Unidentified, UNLINKED) } else { (GapEnd::Slot(row.sender), NOT_RECORDED) };
            let to = if row.host == none { GapEnd::Any } else { GapEnd::Slot(row.host) };
            gaps.push(FlowGap { from, to, part: Some(part), reason: reason.to_owned(), partial: false });
            continue;
        }
        let amount = match row.divisions {
            Recorded::Value(value) if value <= 0 => continue,
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        edges.push(FlowEdge { from: FlowEnd::Slot(row.sender), to: FlowEnd::Slot(row.host), part, amount });
    }
    (edges, gaps)
}

pub fn aid_at(frame: &DecodedFrame) -> Result<(Vec<FlowEdge>, Vec<FlowGap>), String> {
    let read = witnessed_rows::<tables::MilitaryAid>(frame);
    if read.rows.is_empty()
        && let Some(reason) = read.missing
    {
        return Err(reason);
    }
    let (edges, mut gaps) = aid(read.rows);
    gaps.extend(read.missing.map(FlowGap::everything));
    Ok((edges, gaps))
}

impl AtHour for VolunteerFlow {
    const KEY: &'static str = "volunteer-flow";
    const TITLE: &'static str = "Volunteers & expeditionaries";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = VolunteerFlowParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, subject, countries, "countries[]", between, group, limit, top];
    type Reply = CountryFlowReply;
    type Context = GroupsAt;

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
        Ok(GroupsAt::default())
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        if params.subject.is_some() { i32::MAX } else { at.context.valid_through(params.group.as_ref(), at.hour) }
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let empty = |missing: String| CountryFlowReply::not_recorded(missing, 0, Vec::new());
        let (edges, gaps) = match aid_at(frame) {
            Ok(read) => read,
            Err(reason) => return Ok(empty(reason)),
        };
        let resolve = |country: &Country| at.holders.slot_at(country, at.hour);
        let subject = params.subject.as_ref().map(resolve);
        if let Some(None) = subject {
            return Ok(empty("The chosen country holds no tag at this hour".to_owned()));
        }
        let chosen = match at.context.chosen_slots(params.group.as_ref(), &params.countries, at.holders, at.hour)? {
            Ok(chosen) => chosen,
            Err(refused) => return Ok(empty(refused)),
        };
        let shape = FlowShape {
            subject: subject.flatten(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            top: params.top.map(|top| top.max(1) as usize),
            table: false,
            not_recorded: NOT_RECORDED,
            unlinked: UNLINKED,
        };
        Ok(build_flows(&edges, &gaps, parts(), 0, &shape, &|end| FlowEndpoint::at(at.holders, end, at.hour, UNLINKED)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(sender: u32, host: u32, channel: u8, divisions: Recorded<i64>) -> MilitaryAidRow {
        MilitaryAidRow { sender, host, channel, divisions }
    }

    #[test]
    fn channels_become_parts_tallies_are_not_forces_and_a_row_naming_no_sender_or_host_is_a_coverage_marker() {
        let none = TagHolderRow::NONE;
        let rows = [
            row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3)),
            row(3, 2, MilitaryAidRow::EXPEDITIONARIES, Recorded::NotRecorded),
            row(1, 2, MilitaryAidRow::TALLIES, Recorded::Value(1)),
            row(3, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(0)),
            row(3, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded),
            row(none, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded),
        ];
        let (edges, gaps) = aid(&rows);
        assert_eq!(edges, [FlowEdge { from: FlowEnd::Slot(1), to: FlowEnd::Slot(2), part: 0, amount: Some(3) }, FlowEdge { from: FlowEnd::Slot(3), to: FlowEnd::Slot(2), part: 1, amount: None }]);
        assert_eq!(gaps, [
            FlowGap { from: GapEnd::Slot(3), to: GapEnd::Any, part: Some(0), reason: NOT_RECORDED.to_owned(), partial: false },
            FlowGap { from: GapEnd::Unidentified, to: GapEnd::Any, part: Some(0), reason: UNLINKED.to_owned(), partial: false },
        ]);
    }

    #[test]
    fn a_sender_whose_hosts_were_not_recorded_leaves_the_host_side_unproven_and_keeps_witnessed_forces() {
        let none = TagHolderRow::NONE;
        let rows = [row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3)), row(3, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded)];
        let reply = flows(&rows, Some(2));
        assert_eq!(reply.incoming.links.iter().map(|link| link.total.as_deref()).collect::<Vec<_>>(), [Some("3")]);
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (None, Some(NOT_RECORDED)));
    }

    fn shape(subject: Option<u32>) -> FlowShape<'static> {
        FlowShape { subject, chosen: None, between: false, limit: None, top: None, table: false, not_recorded: NOT_RECORDED, unlinked: UNLINKED }
    }

    fn flows(rows: &[MilitaryAidRow], subject: Option<u32>) -> CountryFlowReply {
        let (edges, gaps) = aid(rows);
        build_flows(&edges, &gaps, parts(), 0, &shape(subject), &|end| FlowEndpoint::unidentified(end, String::new()))
    }

    #[test]
    fn a_truncated_aid_table_keeps_the_forces_it_witnessed_and_withholds_the_totals_it_cannot_prove() {
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: std::array::from_fn(|_| TableView::Unavailable { reason: "outside the test".into() }.into()),
            metrics: replay_frame::frame::Metrics { total_ns: 0, table_ns: [0; replay_data::rows::TABLE_COUNT], pool_bytes: 0, dropped_frames: 0 },
        };
        *frame.tables[TableId::MilitaryAid.index()] = TableView::Truncated { rows: TableRows::MilitaryAid(vec![row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3))]), missing: 4 };
        let (edges, gaps) = aid_at(&frame).unwrap();
        let reply = build_flows(&edges, &gaps, parts(), 0, &shape(Some(2)), &|end| FlowEndpoint::unidentified(end, String::new()));
        let hosted: Vec<(Option<u32>, Option<&str>)> = reply.incoming.links.iter().map(|link| (link.from.and_then(|at| reply.endpoints[at as usize].slot), link.amounts[0].amount.as_deref())).collect();
        assert_eq!(hosted, [(Some(1), Some("3"))], "a witnessed sender-host row is that pair's own count");
        assert!(reply.incoming.total.is_none() && reply.incoming.missing.as_deref().is_some_and(|why| why.ends_with("read only in part at this hour (4 rows not read)")));
        *frame.tables[TableId::MilitaryAid.index()] = TableView::Unavailable { reason: "reader fault".into() };
        assert!(aid_at(&frame).is_err());
    }

    #[test]
    fn a_coverage_marker_is_never_a_deployment_and_withholds_only_what_it_covers() {
        let none = TagHolderRow::NONE;
        let everywhere = flows(&[row(none, none, MilitaryAidRow::VOLUNTEERS, Recorded::NotRecorded)], None);
        assert!(everywhere.links.is_empty() && everywhere.senders.links.is_empty() && everywhere.receivers.links.is_empty(), "no ribbon between two unknown countries");
        assert_eq!((everywhere.receivers.total.as_deref(), everywhere.receivers.missing.as_deref()), (None, Some(UNLINKED)));
        let rows = [row(1, 2, MilitaryAidRow::VOLUNTEERS, Recorded::Value(3)), row(1, none, MilitaryAidRow::EXPEDITIONARIES, Recorded::NotRecorded)];
        let sender = flows(&rows, Some(1));
        let hosts: Vec<(Option<u32>, Option<&str>)> = sender.outgoing.links.iter().map(|link| (link.to.and_then(|at| sender.endpoints[at as usize].slot), link.total.as_deref())).collect();
        assert_eq!(hosts, [(Some(2), None)], "the witnessed host is kept with its total withheld, and no unknown host is invented");
        let amounts: Vec<(u32, Option<&str>)> = sender.outgoing.links[0].amounts.iter().map(|amount| (amount.part, amount.amount.as_deref())).collect();
        assert_eq!(amounts, [(0, Some("3")), (1, None)]);
        assert_eq!(sender.outgoing.links[0].missing.as_deref(), Some(NOT_RECORDED));
        assert_eq!((sender.outgoing.total.as_deref(), sender.outgoing.missing.as_deref()), (None, Some(NOT_RECORDED)));
        let host = flows(&rows, Some(2));
        assert_eq!(host.incoming.links.iter().map(|link| link.total.as_deref()).collect::<Vec<_>>(), [None]);
        assert_eq!(host.incoming.missing.as_deref(), Some(NOT_RECORDED));
        assert_eq!((host.outgoing.total.as_deref(), host.outgoing.missing.as_deref()), (Some("0"), None), "a marker for another sender says nothing of the host's own forces");
    }
}
