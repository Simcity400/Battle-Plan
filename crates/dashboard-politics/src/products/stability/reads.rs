
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::Source;
use dashboard_base::span::HourSpan;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StabilityReadsParams {
    pub branch_id: String,
    pub from: i32,
    pub to: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityReadsReply {
    pub reads: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StabilityReadsReply);

pub(crate) fn reads(query: &dashboard_query::Query<'_>, from: i32, to: i32) -> io::Result<Vec<HourSpan>> {
    let recording = query.replay.recording();
    let mut out: Vec<HourSpan> = Vec::new();
    for stretch in query.timeline.within(from, to) {
        let held = dashboard_query::cost::storage(|| replay_lanes::lanes::available_reads(recording, stretch.run, TableId::CountryStabilityInputs, stretch.first, stretch.last))?;
        for (first, last) in held {
            match out.last_mut() {
                Some(span) if i64::from(span.last_hour) + 1 == i64::from(first) => span.last_hour = last,
                _ => out.push(HourSpan { first_hour: first, last_hour: last }),
            }
        }
    }
    Ok(out)
}

pub struct StabilityReads;

impl Product for StabilityReads {
    const KEY: &'static str = "stability-reads";
    const CONTENT_DEPENDENT: bool = false;
    type Params = StabilityReadsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, from: Start, to: End];
    type Reply = StabilityReadsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StabilityReadsParams::decl(), StabilityReadsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.from > params.to {
            return Err("Invalid span");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, _: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        Ok(StabilityReadsReply { reads: reads(&query, params.from, params.to)?, warnings: Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::reads;
    use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryStabilityInputRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    fn recording() -> Source {
        let frames = (100..=125)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1_000));
                if hour == 100 || (105..=107).contains(&hour) || hour == 120 {
                    let rows = TableRows::CountryStabilityInputs(vec![CountryStabilityInputRow {
                        country: 9001,
                        keys: vec![u32::MAX; CountryStabilityInputRow::KEY_NAMES.len()],
                        ..Default::default()
                    }]);
                    frame.tables[TableId::CountryStabilityInputs.index()] = TableView::Complete(rows).into();
                    frame.metrics.table_ns[TableId::CountryStabilityInputs.index()] = 1;
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "reads".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn the_stability_inputs_read_hours_are_the_hours_a_read_was_taken() {
        let source = recording();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let spans = |from, to| reads(&query, from, to).unwrap().iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>();
        assert_eq!(spans(100, 125), [(100, 100), (105, 107), (120, 120)]);
        assert_eq!(spans(106, 119), [(106, 107)], "cut to the span asked");
    }
}
