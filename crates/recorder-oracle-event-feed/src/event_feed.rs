use super::Report;
use replay::campaign::Replay;
use replay_data::{
    event_feed_rows::EventFeedCounts, event_feed_rows::EventFeedCoverage,
    event_feed_rows::EventFeedHistoryRow, rows::TableId, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::io;

pub fn compare(
    replay: &Replay,
    frame: &DecodedFrame,
    document: &content_reader_save::SaveDocument,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation(
        "recording self-check only: the war-link observation records the second war link, the annex observation precedes annex on-actions and the provenance entries observe the operations that join and end wars, start peace conferences, release countries and change state owners. Text saves retain neither these calls, their causes nor the native actor/target instance words. Saved war and country state cannot certify action kind, sequence, exact hour, actor roles or complete capture; matched native observations and world/load, unwind and detach checks are required",
        "EventFeedHistory",
    );
    let selected = [TableId::EventFeedHistory];
    let mut check = Check::default();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let mut windows = super::windows::Witness::default();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    for block in blocks {
        replay.visit_blocks(
            &[block],
            &selected,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            let rows = decoded.complete::<tables::EventFeedHistory>()?;
            let [row] = rows else {
                report.difference(
                    "EventFeedHistory.pack",
                    rows.len().to_string(),
                    "one hourly history row",
                );
                return Ok(true);
            };
            if let Err(reason) = check.observe(row, decoded.hour) {
                report.difference(
                    format!("EventFeedHistory hour {}", decoded.hour),
                    reason.to_string(),
                    "valid consecutive event accounting",
                );
            }
            state.observe(row)?;
            windows.observe(row);
            report_coverage(row, decoded.hour, frame.hour, report);
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    super::script_events::compare(frame, document, &state, &windows, report)
}

fn report_coverage(row: &EventFeedHistoryRow, hour: i32, save_hour: i32, report: &mut Report) {
    for family in row.refused_script_families() {
        report.limitation(
            "unobserved native writers refuse this family's interval coverage; sampled values do not prove unchanged carry or between-sample lifecycle completeness",
            format!("EventFeedHistory hour {hour}: {family}"),
        );
    }
    if row.coverage == EventFeedCoverage::Complete {
        return;
    }
    let reason = row.failure.as_deref().map_or_else(
        || "partial native interval".to_owned(),
        |reason| String::from_utf8_lossy(reason).into_owned(),
    );
    let subject = format!("EventFeedHistory hour {hour}: {reason}");
    report.limitation(
        "native event interval coverage is incomplete; the save retains no event history and cannot certify the unobserved interval",
        subject.clone(),
    );
    if hour == save_hour {
        report.incomplete("EventFeedHistory interval", subject);
    }
}

#[derive(Default)]
struct Check {
    hour: Option<i32>,
    generation: Option<u64>,
    sequence: Option<u64>,
    counts: Option<EventFeedCounts>,
}

impl Check {
    fn observe(&mut self, row: &EventFeedHistoryRow, hour: i32) -> io::Result<()> {
        row.validate()?;
        if row.read_hour != hour
            || row
                .events
                .iter()
                .any(|event| self.sequence.is_some_and(|last| event.sequence <= last))
        {
            return Err(io::Error::other(
                "event date, supplying hour or sequence differs",
            ));
        }
        if self
            .generation
            .is_some_and(|generation| generation != row.generation)
        {
            return Err(io::Error::other(
                "native world generation changed within a recording branch",
            ));
        }
        if let (Some(previous), Some(next)) = (self.counts, row.counts)
            && (next.acquired < previous.acquired
                || next.settled < previous.settled
                || next.failed < previous.failed
                || next.rejected < previous.rejected)
        {
            return Err(io::Error::other("cumulative event counts decreased"));
        }
        let consecutive = self.hour.is_none_or(|old| old.checked_add(1) == Some(hour));
        self.hour = Some(hour);
        self.generation = Some(row.generation);
        self.sequence = row
            .events
            .last()
            .map(|event| event.sequence)
            .or(self.sequence);
        self.counts = row.counts;
        if !consecutive {
            return Err(io::Error::other("event history has an unobserved hour"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        event_feed_rows::EventFeedCountry, event_feed_rows::EventFeedEvent,
        event_feed_rows::EventFeedSource,
    };

    #[test]
    fn event_feed_partial_history_is_coverage_and_save_hour_remains_incomplete() {
        let mut report = Report::default();
        let mut row = EventFeedHistoryRow {
            script_refusals: 0,
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
            generation: 4,
            read_hour: 10,
            coverage: EventFeedCoverage::Unavailable,
            counts: None,
            outstanding: 0,
            failure: Some(b"Event feed world observation unavailable".to_vec()),
            events: Vec::new(),
            windows: Default::default(),
        };
        report_coverage(&row, 10, 11, &mut report);
        assert!(report.differences.is_empty());
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 1);
        row.coverage = EventFeedCoverage::Partial;
        row.counts = Some(EventFeedCounts::default());
        row.failure = None;
        report_coverage(&row, 11, 11, &mut report);
        assert!(report.differences.is_empty());
        assert!(report.failed_comparisons["EventFeedHistory interval"].contains("partial native interval"));
        row.coverage = EventFeedCoverage::Complete;
        report_coverage(&row, 12, 12, &mut report);
        assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 2);
    }

    #[test]
    fn complete_transport_cannot_hide_named_script_family_refusals() {
        let mut report = Report::default();
        let row = EventFeedHistoryRow {
            script_refusals: replay_data::event_feed_rows::DECISION_FAMILY_REFUSALS,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts::default()),
            ..Default::default()
        };
        report_coverage(&row, 10, 10, &mut report);
        assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 5);
        assert!(report.failed_comparisons.is_empty());
        assert!(report.differences.is_empty());
    }
    #[test]
    fn self_check_refuses_repeated_events_future_dates_count_resets_and_unobserved_hours() {
        let row = EventFeedHistoryRow {
            script_refusals: 0,
            script_tokens: Vec::new(),
            script_scopes: Vec::new(),
            generation: 9,
            read_hour: 12,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts {
                acquired: 1,
                settled: 1,
                ..Default::default()
            }),
            outstanding: 0,
            failure: None,
            events: vec![EventFeedEvent {
                script: None,
                sequence: 4,
                generation: 9,
                hour: 11,
                source: EventFeedSource::Join,
                actor: EventFeedCountry {
                    reference: 8,
                    id: 2,
                    instance: 3,
                },
                target: EventFeedCountry {
                    reference: 0,
                    id: 0,
                    instance: 0,
                },
                cause: Default::default(),
            }],
            windows: Default::default(),
        };
        for mode in 0..5 {
            let mut check = Check::default();
            check.observe(&row, 12).unwrap();
            let mut next = row.clone();
            next.read_hour = 13;
            match mode {
                0 => {}
                1 => {
                    next.events[0].sequence = 5;
                    next.events[0].hour = 14;
                }
                2 => {
                    next.events.clear();
                    next.counts = Some(EventFeedCounts::default());
                }
                3 => {
                    next.events.clear();
                    next.read_hour = 14;
                }
                _ => {
                    next.events.clear();
                    next.generation += 1;
                }
            }
            assert!(check.observe(&next, next.read_hour).is_err(), "mode {mode}");
        }
    }
}
