//! Branch self-check: the save retains outcomes, never combat provenance.
use super::{Report, complete};
use replay::{
    Hour,
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, TableView},
    rows::{NavalResultSourceRow, TableId, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

const TABLES: [TableId; 3] = [
    TableId::NavalResultSources,
    TableId::NavalResults,
    TableId::NavalCombats,
];

pub(super) fn compare(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    if let Some(reason) = replay.format.table_refusal(TableId::NavalResultSources) {
        report.difference(
            "NavalResultSources.compatibility",
            reason,
            "compatible recorded event table",
        );
        return Ok(());
    }
    report.limitation(
        "recording self-check only: naval-result fill/merge/save retain no source-combat reference, fleet/task-force identity, start hour or duration. Text saves contain retained results and live combats, but no live reference for an ended combat, so they cannot certify the captured mapping or transient source identity. Complete consecutive branch reads certify observed new-result and result_hour transitions against events; results retired before a read and same-hour merges that leave result_hour unchanged are invisible to those checks. Callback context, hook counters and native probes still require the live run; no save difference is waived",
        "NavalResultSources",
    );
    let mut check = SourceCheck {
        event_losses: replay.event_losses(frame.branch).collect(),
        ..SourceCheck::default()
    };
    for gap in replay
        .gaps
        .iter()
        .filter(|gap| gap.branch == frame.branch && gap.first_hour <= frame.hour)
    {
        let last = gap.last_hour.min(frame.hour);
        check.declared_gaps.push((gap.first_hour, last));
        report.difference(
            format!("NavalResultSources.gap hours {}..={last}", gap.first_hour),
            &gap.reason,
            "complete event interval",
        );
    }
    check.declared_gaps.sort_unstable();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    for block in blocks {
        let mut decoder = FrameDecoder::for_tables(&TABLES);
        for encoded in replay.read_block_selected(block, &TABLES)? {
            if encoded.hour > frame.hour {
                break;
            }
            check.observe(decoder.advance(&encoded)?, report);
        }
    }
    check.finish(report);
    Ok(())
}

#[derive(Default)]
struct SourceCheck {
    previous_hour: Option<Hour>,
    combats: Option<BTreeSet<u64>>,
    results: Option<BTreeMap<u64, Hour>>,
    declared_gaps: Vec<(Hour, Hour)>,
    /// The branch's recorded event losses (`ControlRecord::EventLoss`), as
    /// the reads each lies between.
    event_losses: Vec<(Hour, Hour)>,
    last_event: Option<(u64, Hour)>,
    sources: BTreeSet<u64>,
    pending: BTreeMap<u64, NavalResultSourceRow>,
    events: usize,
}

impl SourceCheck {
    fn observe(&mut self, frame: &DecodedFrame, report: &mut Report) {
        let first_reason = matches!(
            &*frame.tables[TableId::NavalResultSources.index()],
            TableView::Unavailable { reason }
                if reason == "naval result sources first/branch interval is partial"
        );
        let partial_first = self.previous_hour.is_none() && first_reason;
        // A reattached collector's first interval, which the recording
        // declares lost from the previous read to this one.
        let partial_reattach = first_reason
            && self
                .previous_hour
                .is_some_and(|previous| self.event_losses.contains(&(previous, frame.hour)));
        let partial_recovery = matches!(
            &*frame.tables[TableId::NavalResultSources.index()],
            TableView::Unavailable { reason }
                if reason == "naval result sources recovery interval is partial"
        );
        if let Some(previous) = self.previous_hour
            && previous.checked_add(1) != Some(frame.hour)
        {
            self.discontinuity(previous, frame.hour, report);
            self.combats = None;
            self.results = None;
        }
        let results = complete::<tables::NavalResults>(frame).map(|rows| {
            rows.iter()
                .map(|row| (row.id, row.result_hour))
                .collect::<BTreeMap<_, _>>()
        });
        let combats = complete::<tables::NavalCombats>(frame)
            .map(|rows| rows.iter().map(|row| row.id).collect::<BTreeSet<_>>());
        let rows = complete::<tables::NavalResultSources>(frame);
        match rows {
            Err(error) if partial_first => report.limitation(
                "the branch's first event interval is partial and unavailable; its missing combat-to-result events are not certified",
                format!("NavalResultSources hour {}: {error}", frame.hour),
            ),
            Err(error) if partial_reattach => report.limitation(
                "the event interval after a recorded event loss (the recorder was detached between two reads of the branch) is partial and unavailable; its missing combat-to-result events are not certified",
                format!("NavalResultSources hour {}: {error}", frame.hour),
            ),
            Err(error) if partial_recovery => report.limitation(
                "the event interval after a loss is partial and unavailable; its missing combat-to-result events are not certified, and the lost interval remains a difference",
                format!("NavalResultSources hour {}: {error}", frame.hour),
            ),
            Err(error) => report.difference(
                format!("NavalResultSources.unavailable_interval hour {}", frame.hour),
                error.to_string(),
                "complete event interval",
            ),
            Ok(rows) => {
                let mut earlier = self
                    .results
                    .as_ref()
                    .map(|results| results.keys().copied().collect::<BTreeSet<_>>());
                let mut counts = BTreeMap::<u64, (usize, usize)>::new();
                for row in rows {
                    let count = counts.entry(row.result).or_default();
                    if row.merged {
                        count.1 += 1;
                    } else {
                        count.0 += 1;
                    }
                }
                // Only result creation inserts results, and only
                // fill/merge write result_hour.
                if let (Some(previous), Ok(current)) = (&self.results, &results) {
                    for (&result, &hour) in current {
                        let (new, merged) = counts.get(&result).copied().unwrap_or_default();
                        if !previous.contains_key(&result) && new != 1 {
                            report.difference(
                                format!(
                                    "NavalResultSources.result_unsourced result {result} hour {}",
                                    frame.hour
                                ),
                                new,
                                "exactly one non-merged event in this interval",
                            );
                        } else if previous.get(&result).is_some_and(|&old| old != hour)
                            && merged == 0
                        {
                            report.difference(
                                format!(
                                    "NavalResultSources.merge_unsourced result {result} hour {}",
                                    frame.hour
                                ),
                                (previous.get(&result), hour),
                                "a merged event in this interval",
                            );
                        }
                    }
                }
                let mut ordered: Vec<_> = rows.iter().collect();
                ordered.sort_by_key(|row| row.id);
                let mut last_emissions = BTreeMap::new();
                for row in ordered {
                    last_emissions.insert(row.result, (row.id, row.game_hour));
                    self.events += 1;
                    let label =
                        |failure: &str| format!("NavalResultSources.{failure} event {}", row.id);
                    if row.id == 0 || self.last_event.is_some_and(|(id, _)| row.id <= id) {
                        report.difference(
                            label("event_identity"),
                            row.id,
                            "positive, increasing branch event id",
                        );
                        continue;
                    }
                    if row.game_hour > frame.hour
                        || self.previous_hour.is_some_and(|hour| row.game_hour < hour)
                        || self.last_event.is_some_and(|(_, hour)| row.game_hour < hour)
                        || row.combat_created_hour.is_some_and(|hour| hour > row.game_hour)
                    {
                        report.difference(
                            label("event_hour"),
                            (row.game_hour, row.combat_created_hour),
                            "native event within its recorded interval, with creation no later than resolution",
                        );
                    }
                    self.last_event = Some((row.id, row.game_hour));
                    // Merge requires both source flags; only the
                    // stack-local strike creators set them on this executable.
                    if row.merged && !row.transient {
                        report.difference(
                            label("persistent_merge"),
                            row.combat,
                            "only transient strike sources merge into a result",
                        );
                    }
                    if row.transient {
                        // The three stack-strike callers construct, finalize and
                        // destroy in one call.
                        if row.combat_created_hour != Some(row.game_hour)
                            || self
                                .combats
                                .as_ref()
                                .is_some_and(|ids| ids.contains(&row.combat))
                            || combats.as_ref().is_ok_and(|ids| ids.contains(&row.combat))
                        {
                            report.difference(
                                label("transient_source"),
                                (row.combat, row.combat_created_hour),
                                "explicit same-hour stack source absent from the live registry",
                            );
                        }
                    } else {
                        let witnessed = self
                            .combats
                            .as_ref()
                            .is_some_and(|ids| ids.contains(&row.combat));
                        // The constructor witness proves creation
                        // after the previous read; the result tap proves resolution.
                        let within_hour = self
                            .previous_hour
                            .zip(row.combat_created_hour)
                            .is_some_and(|(previous, created)| {
                                previous <= created && created <= row.game_hour
                            });
                        let missing_previous =
                            self.combats.is_none() && self.previous_hour.is_some();
                        if !missing_previous && !witnessed && !within_hour {
                            report.difference(
                                label("combat_missing"),
                                row.combat,
                                "previous NavalCombats read or native creation and resolution within this interval",
                            );
                        }
                        if !self.sources.insert(row.combat) {
                            report.difference(
                                label("duplicate_combat"),
                                row.combat,
                                "one result transition per persistent combat",
                            );
                        }
                    }
                    if row.merged
                        && earlier.as_ref().is_some_and(|ids| !ids.contains(&row.result))
                    {
                        report.difference(
                            label("merge_without_prior_result"),
                            row.result,
                            "result at previous read or an earlier creation event in this interval",
                        );
                    } else if !row.merged
                        && earlier.as_ref().is_some_and(|ids| ids.contains(&row.result))
                    {
                        report.difference(
                            label("new_result_already_existed"),
                            row.result,
                            "new independent result reference",
                        );
                    }
                    if !row.merged
                        && let Some(earlier) = &mut earlier
                    {
                        earlier.insert(row.result);
                    }
                    self.pending.insert(row.id, row.clone());
                }
                // Fill/merge write from the call's current native hour
                // including same-hour merges.
                if let Ok(current) = &results {
                    for (result, (event, hour)) in last_emissions {
                        if let Some(&recorded) = current.get(&result)
                            && recorded != hour
                        {
                            report.difference(
                                format!(
                                    "NavalResultSources.result_hour result {result} event {event} hour {}",
                                    frame.hour
                                ),
                                recorded,
                                hour,
                            );
                        }
                    }
                }
            }
        }
        if let Ok(results) = &results {
            self.pending
                .retain(|_, row| frame.hour < row.game_hour || !results.contains_key(&row.result));
        }
        for (table, error) in [
            (TableId::NavalResults, results.as_ref().err()),
            (TableId::NavalCombats, combats.as_ref().err()),
        ] {
            if let Some(error) = error {
                report.difference(
                    format!(
                        "NavalResultSources.input_unavailable {table:?} hour {}",
                        frame.hour
                    ),
                    error.to_string(),
                    "complete prerequisite read",
                );
            }
        }
        self.previous_hour = Some(frame.hour);
        self.results = results.ok();
        self.combats = combats.ok();
    }

    fn discontinuity(&self, previous: Hour, hour: Hour, report: &mut Report) {
        let Some(mut first) = previous.checked_add(1).filter(|&first| first < hour) else {
            report.difference(
                format!("NavalResultSources.gap after hour {previous}"),
                hour,
                previous.checked_add(1),
            );
            return;
        };
        for &(start, end) in &self.declared_gaps {
            if end < first || start >= hour {
                continue;
            }
            if start > first {
                report.difference(
                    format!("NavalResultSources.gap hours {first}..={}", start - 1),
                    "missing recorded hours",
                    "complete event interval",
                );
            }
            first = end.saturating_add(1).max(first);
        }
        if first < hour {
            report.difference(
                format!("NavalResultSources.gap hours {first}..={}", hour - 1),
                "missing recorded hours",
                "complete event interval",
            );
        }
    }

    fn finish(self, report: &mut Report) {
        for row in self.pending.values() {
            // The daily sweep can retire a result before the next
            // entry read; the game imposes no positive content TTL floor.
            report.limitation(
                "an emitted result was never retained at a later branch read; native pre-read retirement and a missing result reference cannot be distinguished by this recording-only self-check",
                format!(
                    "NavalResultSources.result_unobserved event {} result {} at native hour {}",
                    row.id, row.result, row.game_hour
                ),
            );
        }
        report.compared("NavalResultSources", self.events);
    }
}

#[cfg(test)]
mod tests;
