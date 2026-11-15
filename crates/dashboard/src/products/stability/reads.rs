//! `stability-reads`: the hours a span's stability inputs were read. Displayed stability and war
//! support are derived every hour from the inputs as last read, so an hour between reads shows the
//! read before it, which the chart's tooltip marks "as of". Read from the blocks' page entries of
//! each stretch of the timeline the span meets: no row is decoded.

use super::super::{LoadContext, Product};
use crate::Source;
use dashboard_base::span::HourSpan;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StabilityReadsParams {
    pub branch_id: String,
    pub from: i32,
    pub to: i32,
}

/// The hours the stability inputs were read in a span: an hour between reads shows the read before it.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StabilityReadsReply {
    pub reads: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StabilityReadsReply);

/// The read hours of `from..=to` over `query`'s stretches, ascending, adjacent hours joined.
pub(crate) fn reads(query: &dashboard_query::Query<'_>, from: i32, to: i32) -> io::Result<Vec<HourSpan>> {
    let recording = query.replay.recording();
    let mut out: Vec<HourSpan> = Vec::new();
    for stretch in query.timeline.within(from, to) {
        let held = dashboard_query::cost::storage(|| replay::lanes::available_reads(recording, stretch.run, TableId::CountryStabilityInputs, stretch.first, stretch.last))?;
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
    type Reply = StabilityReadsReply;

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
        let branch = params.branch_id.parse::<u64>().map_err(|_| crate::invalid("Invalid branch"))?;
        let query = crate::answer::query(source, branch)?;
        Ok(StabilityReadsReply { reads: reads(&query, params.from, params.to)?, warnings: Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::reads;
    use crate::{Source, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryStabilityInputRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    /// The inputs are read at hours 100, 105 to 107 and 120; every hour holds the timeline.
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
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(crate::SourceKey { source_id: "reads".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    /// An hour between reads carries the read before it: the reply names exactly the hours read,
    /// joined where adjacent, within the span asked.
    #[test]
    fn the_stability_inputs_read_hours_are_the_hours_a_read_was_taken() {
        let source = recording();
        let query = crate::answer::query(&source, 1).unwrap();
        let spans = |from, to| reads(&query, from, to).unwrap().iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>();
        assert_eq!(spans(100, 125), [(100, 100), (105, 107), (120, 120)]);
        assert_eq!(spans(106, 119), [(106, 107)], "cut to the span asked");
    }
}
