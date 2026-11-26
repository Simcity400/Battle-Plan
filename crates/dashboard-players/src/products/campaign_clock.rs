use dashboard_base::products::{LoadContext, Product};
use super::player_apm;
use dashboard_base::Source;
use replay_capture_spool::commands::{ClockMeasurement, Index};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const OVERLAP_REASON: &str = "Overlapping recordings prevent a unique campaign clock total.";
pub const OVERLAP_WARNING: &str = "Recording intervals overlap; a unique campaign duration cannot be established.";
pub const INCOMPLETE_WARNING: &str = "Coverage is incomplete. Totals include measured clock intervals only.";

pub struct RecordedCoverage {
    spans: Vec<dashboard_base::span::HourSpan>,
}

#[derive(Default)]
struct CoverageCache {
    branches: BTreeMap<u64, Arc<RecordedCoverage>>,
}

fn subtract_gaps(
    spans: Vec<dashboard_base::span::HourSpan>,
    gaps: &[(i32, i32)],
) -> Vec<dashboard_base::span::HourSpan> {
    if gaps.is_empty() {
        return spans;
    }
    let mut gaps: Vec<(i32, i32)> = gaps
        .iter()
        .copied()
        .filter(|&(first, last)| first <= last)
        .collect();
    gaps.sort_unstable();
    let mut merged_gaps: Vec<(i64, i64)> = Vec::with_capacity(gaps.len());
    for (first, last) in gaps {
        let (gf, gl) = (i64::from(first), i64::from(last));
        if let Some(prev) = merged_gaps.last_mut()
            && gf <= prev.1 + 1
        {
            prev.1 = prev.1.max(gl);
        } else {
            merged_gaps.push((gf, gl));
        }
    }
    let mut out = Vec::with_capacity(spans.len());
    let mut gap_idx = 0usize;
    for span in spans {
        let mut cursor = i64::from(span.first_hour);
        let end = i64::from(span.last_hour);
        while gap_idx < merged_gaps.len() && merged_gaps[gap_idx].1 < cursor {
            gap_idx += 1;
        }
        let mut idx = gap_idx;
        while idx < merged_gaps.len() {
            let (gf, gl) = merged_gaps[idx];
            if gf > end {
                break;
            }
            if gf > cursor {
                out.push(dashboard_base::span::HourSpan {
                    first_hour: cursor as i32,
                    last_hour: (gf - 1) as i32,
                });
            }
            cursor = cursor.max(gl + 1);
            if cursor > end {
                break;
            }
            idx += 1;
        }
        gap_idx = idx;
        if cursor <= end {
            out.push(dashboard_base::span::HourSpan {
                first_hour: cursor as i32,
                last_hour: end as i32,
            });
        }
    }
    out
}

impl RecordedCoverage {
    pub fn for_branch(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Self>> {
        let state = source.state::<CoverageCache>();
        if let Some(held) = state.lock().unwrap_or_else(|error| error.into_inner()).branches.get(&branch) {
            return Ok(Arc::clone(held));
        }
        let replay = source.replay_ref();
        let mut verified: Vec<dashboard_base::span::HourSpan> = Vec::new();
        for block in replay
            .index
            .iter()
            .filter(|block| block.branch == branch && block.first_hour <= block.last_hour)
        {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let read = replay.visit_blocks(
                &[block],
                &[replay_data::rows::TableId::Campaign],
                |_, _, decoded| {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    if decoded
                        .complete::<replay_data::rows::tables::Campaign>()
                        .is_ok_and(|rows| rows.len() == 1 && rows[0].id == replay.campaign_id)
                    {
                        let hour = decoded.hour;
                        match verified.last_mut() {
                            Some(held)
                                if hour >= held.first_hour
                                    && i64::from(hour) <= i64::from(held.last_hour) + 1 =>
                            {
                                held.last_hour = held.last_hour.max(hour);
                            }
                            _ => verified.push(dashboard_base::span::HourSpan {
                                first_hour: hour,
                                last_hour: hour,
                            }),
                        }
                    }
                    Ok(true)
                },
                || dashboard_text::metadata::check_cancel(cancelled),
            );
            if let Err(error) = read
                && (error.kind() == io::ErrorKind::Interrupted
                    || engine_allocation::is_allocation_error(&error))
            {
                return Err(error);
            }
        }
        let spans = dashboard_base::ranking::joined_spans(verified, i32::MIN, i32::MAX);
        let branch_id = branch.to_string();
        let gaps: Vec<(i32, i32)> = source
            .metadata()
            .map_err(io::Error::other)?
            .branches
            .iter()
            .find(|info| info.id == branch_id)
            .map(|info| info.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect())
            .unwrap_or_default();
        let spans = subtract_gaps(spans, &gaps);
        let built = Arc::new(Self { spans });
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(Arc::clone(cache.branches.entry(branch).or_insert(built)))
    }

    pub fn covers(&self, first_hour: i32, last_hour: i32) -> bool {
        if first_hour > last_hour {
            return false;
        }
        let at = self.spans.partition_point(|span| span.last_hour < first_hour);
        self.spans
            .get(at)
            .is_some_and(|span| span.first_hour <= first_hour && last_hour <= span.last_hour)
    }

    #[cfg(test)]
    pub fn full() -> Self {
        Self { spans: vec![dashboard_base::span::HourSpan { first_hour: i32::MIN, last_hour: i32::MAX }] }
    }
}

pub struct CampaignClock;

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CampaignClockParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CampaignClockReply {
    pub available: bool,
    pub recorded_seconds: Option<f64>,
    pub playing_seconds: Option<f64>,
    pub paused_seconds: Option<f64>,
    pub complete: bool,
    pub warnings: Vec<String>,
}

impl Product for CampaignClock {
    const KEY: &'static str = "campaign-clock";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = CampaignClockParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End];
    type Reply = CampaignClockReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params.first_hour > params.last_hour {
            return Err("Invalid campaign clock period");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CampaignClockParams::decl(), CampaignClockReply::decl()]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().unwrap();
        let index = player_apm::index(source, branch, cancelled)?;
        let coverage = RecordedCoverage::for_branch(source, branch, cancelled)?;
        query(&index, &coverage, params, cancelled)
    }
}

pub fn query(
    index: &Index,
    coverage: &RecordedCoverage,
    params: &CampaignClockParams,
    cancelled: &AtomicBool,
) -> io::Result<CampaignClockReply> {
    CampaignClock::validate(params).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut reply = CampaignClockReply {
        available: false,
        recorded_seconds: None,
        playing_seconds: None,
        paused_seconds: None,
        complete: false,
        warnings: Vec::new(),
    };
    let mut regions: Vec<(u64, ClockMeasurement)> = engine_allocation::vec(index.timelines.len())?;
    for timeline in &index.timelines {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let first = params.first_hour.max(timeline.hours.0);
        let last = params.last_hour.min(timeline.hours.1);
        if first <= last
            && let Some(clock) = timeline.clock(first, last)
        {
            regions.push((timeline.session, clock));
        }
    }
    if regions.is_empty() {
        return Ok(reply);
    }
    reply.available = true;
    let unknown = |mut reply: CampaignClockReply| {
        reply.warnings.push(OVERLAP_WARNING.into());
        reply
    };
    regions.sort_unstable_by_key(|&(session, clock)| (session, clock.from_qpc));
    let mut continuous = true;
    for pair in regions.windows(2) {
        if pair[0].0 == pair[1].0 {
            if pair[0].1.to_qpc > pair[1].1.from_qpc {
                return Ok(unknown(reply));
            }
            continuous &= pair[0].1.to_qpc == pair[1].1.from_qpc;
        } else {
            continuous = false;
        }
    }
    regions.sort_unstable_by_key(|&(_, clock)| (clock.first_hour, clock.last_hour));
    let mut active = BTreeMap::<u64, usize>::new();
    let mut ends = BinaryHeap::<Reverse<(i32, u64)>>::new();
    let mut clocks = BTreeMap::<u64, (u128, Option<u128>)>::new();
    let mut last_hour = None;
    for &(session, clock) in &regions {
        dashboard_text::metadata::check_cancel(cancelled)?;
        while let Some(&Reverse((end, session))) = ends.peek() {
            if end >= clock.first_hour {
                break;
            }
            ends.pop();
            let count = active.get_mut(&session).unwrap();
            *count -= 1;
            if *count == 0 {
                active.remove(&session);
            }
        }
        if active.len() > 1 || active.keys().next().is_some_and(|&id| id != session) {
            return Ok(unknown(reply));
        }
        *active.entry(session).or_default() += 1;
        ends.push(Reverse((clock.last_hour, session)));
        continuous &= clock.complete
            && last_hour.is_none_or(|last| i64::from(clock.first_hour) <= i64::from(last) + 1);
        last_hour = Some(last_hour.map_or(clock.last_hour, |last: i32| last.max(clock.last_hour)));
        let (recorded, playing) = clocks.entry(clock.frequency).or_insert((0, Some(0)));
        *recorded += clock.recorded_ticks;
        *playing = playing.zip(clock.playing_ticks).map(|(a, b)| a + b);
    }
    let mut recorded = 0.0;
    let (mut playing, mut paused) = (Some(0.0), Some(0.0));
    for (frequency, (total, unpaused)) in clocks {
        let seconds = |ticks: u128| {
            (ticks / u128::from(frequency)) as f64
                + (ticks % u128::from(frequency)) as f64 / frequency as f64
        };
        recorded += seconds(total);
        playing = playing.zip(unpaused).map(|(a, b)| a + seconds(b));
        paused = paused.zip(unpaused).map(|(a, b)| a + seconds(total - b));
    }
    reply.recorded_seconds = Some(recorded);
    reply.playing_seconds = playing;
    reply.paused_seconds = paused;
    reply.complete = continuous
        && coverage.covers(params.first_hour, params.last_hour)
        && regions[0].1.first_hour <= params.first_hour
        && last_hour.is_some_and(|last| last >= params.last_hour);
    if !reply.complete {
        reply.warnings.push(INCOMPLETE_WARNING.into());
    }
    if playing.is_none() {
        reply.warnings.push("Some recorded intervals have unknown pause state; playing and paused totals are unavailable.".into());
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_capture_spool::commands;
    use replay_recording::{command_packet::{Actor, Batch, Category, Event, Kind, Origin, Packet, Span}, control::ControlRecord};

    fn packet(
        session: u64,
        capture: u64,
        spans: Vec<Span>,
        extra: impl FnOnce(&mut Batch),
    ) -> ControlRecord {
        packet_on(4, session, capture, spans, extra)
    }

    fn packet_on(
        branch: u64,
        session: u64,
        capture: u64,
        spans: Vec<Span>,
        extra: impl FnOnce(&mut Batch),
    ) -> ControlRecord {
        let mut batch = Batch {
            kinds: vec![Kind {
                id: 1,
                token: 1,
                name: "synthetic order".into(),
                category: Category::Army,
            }],
            spans,
            ..Default::default()
        };
        extra(&mut batch);
        let from = batch.spans.first().unwrap().from_qpc;
        let to = batch.spans.last().unwrap().to_qpc;
        ControlRecord::PlayerCommands {
            session,
            packet: Box::new(Packet::pack(branch, capture, 1, 0, from, to, 0, &batch).unwrap()),
        }
    }

    fn span(from: u64, to: u64, hour: i32, paused: bool) -> Span {
        Span {
            from_qpc: from,
            to_qpc: to,
            hour,
            paused,
            complete: true,
        }
    }

    fn read(controls: &[ControlRecord], first: i32, last: i32) -> CampaignClockReply {
        let sessions: Vec<_> = [7, 8]
            .into_iter()
            .map(|id| {
                let mut session = dashboard_base::test_fixtures::session(id);
                session.qpc_frequency = 1;
                session
            })
            .collect();
        let index = commands::index(&sessions, controls, 4).unwrap();
        query(
            &index,
            &RecordedCoverage::full(),
            &CampaignClockParams {
                branch_id: "4".into(),
                first_hour: first,
                last_hour: last,
            },
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn idle_and_coop_countries_count_clock_time_once() {
        let control = packet(
            7,
            1,
            vec![
                span(0, 60, 100, false),
                span(60, 180, 100, true),
                span(180, 240, 101, false),
            ],
            |batch| {
                batch.actors = [1, 2]
                    .map(|id| Actor {
                        id,
                        player: id as i32,
                        country: 3,
                        since_qpc: 0,
                        at_qpc: 0,
                        user: Vec::new(),
                        name: Vec::new(),
                        tag: b"GER".to_vec(),
                    })
                    .into();
            },
        );
        let reply = read(std::slice::from_ref(&control), 100, 101);
        assert_eq!(reply.recorded_seconds, Some(240.0));
        assert_eq!(reply.playing_seconds, Some(120.0));
        assert_eq!(reply.paused_seconds, Some(120.0));
        assert!(reply.complete);
        let later = read(&[control], 101, 101);
        assert_eq!(later.recorded_seconds, Some(60.0));
        assert_eq!(later.paused_seconds, Some(0.0));
    }

    #[test]
    fn recording_gaps_are_excluded_and_prefixes_clip_to_the_requested_hours() {
        let control = packet(
            7,
            1,
            vec![
                span(10, 20, 100, false),
                span(30, 40, 101, true),
                span(50, 60, 102, false),
            ],
            |_| {},
        );
        let reply = read(std::slice::from_ref(&control), 100, 102);
        assert_eq!(reply.recorded_seconds, Some(30.0));
        assert_eq!(reply.playing_seconds, Some(20.0));
        assert_eq!(reply.paused_seconds, Some(10.0));
        assert!(!reply.complete);
        let reply = read(&[control], 101, 102);
        assert_eq!(reply.recorded_seconds, Some(20.0));
        assert_eq!(reply.playing_seconds, Some(10.0));
    }

    #[test]
    fn unresolved_order_attribution_does_not_withhold_known_clock_time() {
        let reply = read(
            &[packet(7, 1, vec![span(0, 60, 100, false)], |batch| {
                batch.events.push(Event {
                    qpc: 20,
                    hour: 100,
                    actor: 0,
                    player: 99,
                    kind: 1,
                    command_country: None,
                    origin: Origin::Unresolved,
                    sequence: 1,
                    paused: false,
                });
            })],
            100,
            100,
        );
        assert_eq!(reply.playing_seconds, Some(60.0));
        assert!(reply.complete);
    }

    #[test]
    fn unknown_pause_state_withholds_both_parts_but_keeps_recorded_elapsed_time() {
        let mut unknown = span(0, 60, 100, false);
        unknown.complete = false;
        let reply = read(&[packet(7, 1, vec![unknown], |_| {})], 100, 100);
        assert_eq!(reply.recorded_seconds, Some(60.0));
        assert_eq!(reply.playing_seconds, None);
        assert_eq!(reply.paused_seconds, None);
        assert!(!reply.complete);
        assert!(!read(&[], 100, 100).available);
    }

    #[test]
    fn reattachment_sums_recorded_parts_and_never_counts_the_break_as_pause() {
        let reply = read(
            &[
                packet(7, 1, vec![span(0, 60, 100, true)], |_| {}),
                packet(7, 2, vec![span(600, 660, 100, false)], |_| {}),
            ],
            100,
            100,
        );
        assert_eq!(reply.recorded_seconds, Some(120.0));
        assert_eq!(reply.playing_seconds, Some(60.0));
        assert_eq!(reply.paused_seconds, Some(60.0));
        assert!(!reply.complete);
        assert_eq!(reply.warnings, vec![INCOMPLETE_WARNING]);
    }

    #[test]
    fn fractional_attachments_sum_native_ticks_before_converting_to_seconds() {
        let controls: Vec<_> = (0..10)
            .map(|i| packet(7, i + 1, vec![span(i, i + 1, 100, i % 2 == 0)], |_| {}))
            .collect();
        let mut session = dashboard_base::test_fixtures::session(7);
        session.qpc_frequency = 10;
        let index = commands::index(&[session], &controls, 4).unwrap();
        let reply = query(
            &index,
            &RecordedCoverage::full(),
            &CampaignClockParams {
                branch_id: "4".into(),
                first_hour: 100,
                last_hour: 100,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.recorded_seconds, Some(1.0));
        assert_eq!(reply.playing_seconds, Some(0.5));
        assert_eq!(reply.paused_seconds, Some(0.5));
    }

    #[test]
    fn overlapping_recorders_are_unknown_and_distinct_sessions_keep_their_own_clocks() {
        for session in [7, 8] {
            let reply = read(
                &[
                    packet(7, 1, vec![span(0, 60, 100, false)], |_| {}),
                    packet(session, 2, vec![span(30, 90, 100, true)], |_| {}),
                ],
                100,
                100,
            );
            assert_eq!(reply.recorded_seconds, None);
            assert_eq!(reply.playing_seconds, None);
            assert!(!reply.complete);
        }
        let reply = read(
            &[
                packet(7, 1, vec![span(1000, 1060, 100, false)], |_| {}),
                packet(8, 2, vec![span(0, 60, 101, true)], |_| {}),
            ],
            100,
            101,
        );
        assert_eq!(reply.recorded_seconds, Some(120.0));
        assert_eq!(reply.paused_seconds, Some(60.0));
    }

    #[test]
    fn sparse_recorded_hours_are_never_complete_when_command_spans_are_continuous() {
        let mut input = dashboard_base::test_fixtures::records(
            4,
            (100..=105)
                .chain(110..=115)
                .map(|hour| dashboard_base::test_fixtures::source(4, hour, Some(1)))
                .collect(),
        );
        if let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] {
            session.qpc_frequency = 1;
        }
        let spans: Vec<Span> = (100..=115)
            .map(|hour| span((hour - 100) as u64 * 60, (hour - 99) as u64 * 60, hour, false))
            .collect();
        input.push(replay_capture_spool::spool::Record::Control(packet(4, 1, spans, |_| {})));
        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[input],
        )
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "sparse-clock".into(), revision: "1".into() },
            "sparse-clock.battleplan".into(),
            std::sync::Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load_ctx = LoadContext { content: None };
        let across_gap = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 100, last_hour: 115 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(across_gap.available);
        assert_eq!(across_gap.recorded_seconds, Some(16.0 * 60.0));
        assert!(!across_gap.complete);
        assert_eq!(across_gap.warnings, vec![INCOMPLETE_WARNING]);

        let before_gap = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 100, last_hour: 105 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(before_gap.complete);
        assert!(before_gap.warnings.is_empty());

        let after_gap = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 110, last_hour: 115 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(after_gap.complete);
        assert!(after_gap.warnings.is_empty());

        let inside_gap = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 106, last_hour: 109 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!inside_gap.complete);
        assert_eq!(inside_gap.warnings, vec![INCOMPLETE_WARNING]);

        let head = player_apm::head::ClockHead::from_source(
            &source,
            player_apm::head::ClockHeadParams { branch_id: "4".into(), first_hour: 100, last_hour: 115 },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(head.reply_at(105).unwrap().complete);
        assert!(!head.reply_at(108).unwrap().complete);
        let head_late = head.reply_at(115).unwrap();
        assert!(!head_late.complete);
        assert_eq!(head_late.warnings, vec![INCOMPLETE_WARNING]);
    }

    #[test]
    fn unreadable_indexed_block_marks_coverage_incomplete_for_exact_and_followed_reads() {
        let mut foreign = dashboard_base::test_fixtures::source(4, 100, Some(1));
        *foreign.tables[replay_data::rows::TableId::Campaign.index()] =
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Campaign(vec![
                replay_data::rows::CampaignRow {
                    id: b"different synthetic campaign".to_vec(),
                },
            ]));
        let mut first_segment = dashboard_base::test_fixtures::records(4, vec![foreign]);
        if let replay_capture_spool::spool::Record::SessionStart(session) = &mut first_segment[0] {
            session.qpc_frequency = 1;
        }
        first_segment.push(replay_capture_spool::spool::Record::Control(packet_on(
            4,
            4,
            1,
            vec![span(0, 60, 100, false)],
            |_| {},
        )));

        let mut second_segment = dashboard_base::test_fixtures::records(
            5,
            (101..=105)
                .map(|hour| dashboard_base::test_fixtures::source(5, hour, Some(1)))
                .collect(),
        );
        second_segment[0] = first_segment[0].clone();
        if let replay_capture_spool::spool::Record::BranchStart(start) = &mut second_segment[1] {
            start.session = 4;
            start.reason = replay_frame::branches::BranchReason::LoadSignal;
        }
        let later_spans: Vec<Span> = (101..=105)
            .map(|hour| span((hour - 100) as u64 * 60, (hour - 99) as u64 * 60, hour, false))
            .collect();
        second_segment.push(replay_capture_spool::spool::Record::Control(packet_on(
            5,
            4,
            2,
            later_spans,
            |_| {},
        )));

        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[first_segment, second_segment],
        )
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        assert_eq!(replay.index.len(), 2);
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "unreadable-block-clock".into(), revision: "1".into() },
            "unreadable-block-clock.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let branch_id = source.replay_ref().index[0].branch.to_string();
        assert!(
            source
                .metadata()
                .unwrap()
                .branches
                .iter()
                .find(|info| info.id == branch_id)
                .is_some_and(|info| info.gaps.iter().any(|gap| (gap.first_hour, gap.last_hour) == (100, 100)))
        );
        let load_ctx = LoadContext { content: None };

        let whole = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: branch_id.clone(), first_hour: 100, last_hour: 105 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(whole.recorded_seconds, Some(6.0 * 60.0));
        assert!(!whole.complete);
        assert_eq!(whole.warnings, vec![INCOMPLETE_WARNING]);

        let readable = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: branch_id.clone(), first_hour: 101, last_hour: 105 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(readable.complete);
        assert!(readable.warnings.is_empty());

        let head = player_apm::head::ClockHead::from_source(
            &source,
            player_apm::head::ClockHeadParams { branch_id, first_hour: 100, last_hour: 105 },
            &AtomicBool::new(false),
        )
        .unwrap();
        let head_late = head.reply_at(105).unwrap();
        assert_eq!(head_late.recorded_seconds, Some(6.0 * 60.0));
        assert!(!head_late.complete);
        assert_eq!(head_late.warnings, vec![INCOMPLETE_WARNING]);
    }

    #[test]
    fn unreadable_later_indexed_block_marks_coverage_incomplete_for_exact_and_followed_reads() {
        let mut first_segment =
            dashboard_base::test_fixtures::records(4, vec![dashboard_base::test_fixtures::source(4, 100, Some(1))]);
        if let replay_capture_spool::spool::Record::SessionStart(session) = &mut first_segment[0] {
            session.qpc_frequency = 1;
        }
        first_segment.push(replay_capture_spool::spool::Record::Control(packet_on(
            4,
            4,
            1,
            vec![span(0, 60, 100, false)],
            |_| {},
        )));

        let foreign_frames = (101..=105)
            .map(|hour| {
                let mut frame = dashboard_base::test_fixtures::source(5, hour, Some(1));
                *frame.tables[replay_data::rows::TableId::Campaign.index()] =
                    replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Campaign(vec![
                        replay_data::rows::CampaignRow {
                            id: b"different synthetic campaign".to_vec(),
                        },
                    ]));
                frame
            })
            .collect();
        let mut second_segment = dashboard_base::test_fixtures::records(5, foreign_frames);
        second_segment[0] = first_segment[0].clone();
        if let replay_capture_spool::spool::Record::BranchStart(start) = &mut second_segment[1] {
            start.session = 4;
            start.reason = replay_frame::branches::BranchReason::LoadSignal;
        }
        let later_spans: Vec<Span> = (101..=105)
            .map(|hour| span((hour - 100) as u64 * 60, (hour - 99) as u64 * 60, hour, false))
            .collect();
        second_segment.push(replay_capture_spool::spool::Record::Control(packet_on(
            5,
            4,
            2,
            later_spans,
            |_| {},
        )));

        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[first_segment, second_segment],
        )
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        assert_eq!(replay.index.len(), 2);
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "unreadable-later-block-clock".into(), revision: "1".into() },
            "unreadable-later-block-clock.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let branch_id = source.replay_ref().index[0].branch.to_string();
        assert!(
            source
                .metadata()
                .unwrap()
                .branches
                .iter()
                .find(|info| info.id == branch_id)
                .is_some_and(|info| info.gaps.is_empty())
        );
        let load_ctx = LoadContext { content: None };

        let opening = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: branch_id.clone(), first_hour: 100, last_hour: 100 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(opening.complete);
        assert!(opening.warnings.is_empty());

        let whole = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: branch_id.clone(), first_hour: 100, last_hour: 105 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(whole.recorded_seconds, Some(6.0 * 60.0));
        assert!(!whole.complete);
        assert_eq!(whole.warnings, vec![INCOMPLETE_WARNING]);

        let head = player_apm::head::ClockHead::from_source(
            &source,
            player_apm::head::ClockHeadParams { branch_id, first_hour: 100, last_hour: 105 },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(head.reply_at(100).unwrap().complete);
        let head_late = head.reply_at(105).unwrap();
        assert_eq!(head_late.recorded_seconds, Some(6.0 * 60.0));
        assert!(!head_late.complete);
        assert_eq!(head_late.warnings, vec![INCOMPLETE_WARNING]);
    }

    #[test]
    fn foreign_campaign_later_in_same_block_marks_coverage_incomplete_for_exact_and_followed_reads() {
        let frames: Vec<_> = (100..=105)
            .map(|hour| {
                let mut frame = dashboard_base::test_fixtures::source(4, hour, Some(1));
                if hour >= 101 {
                    *frame.tables[replay_data::rows::TableId::Campaign.index()] =
                        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Campaign(vec![
                            replay_data::rows::CampaignRow {
                                id: b"different synthetic campaign".to_vec(),
                            },
                        ]));
                }
                frame
            })
            .collect();
        let mut input = dashboard_base::test_fixtures::records(4, frames);
        if let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] {
            session.qpc_frequency = 1;
        }
        let spans: Vec<Span> = (100..=105)
            .map(|hour| span((hour - 100) as u64 * 60, (hour - 99) as u64 * 60, hour, false))
            .collect();
        input.push(replay_capture_spool::spool::Record::Control(packet(4, 1, spans, |_| {})));

        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[input],
        )
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        assert_eq!(replay.index.len(), 1);
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "same-block-foreign-clock".into(), revision: "1".into() },
            "same-block-foreign-clock.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load_ctx = LoadContext { content: None };

        let opening = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 100, last_hour: 100 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(opening.complete);
        assert!(opening.warnings.is_empty());

        let whole = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 100, last_hour: 105 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(whole.recorded_seconds, Some(6.0 * 60.0));
        assert!(!whole.complete);
        assert_eq!(whole.warnings, vec![INCOMPLETE_WARNING]);

        let head = player_apm::head::ClockHead::from_source(
            &source,
            player_apm::head::ClockHeadParams { branch_id: "4".into(), first_hour: 100, last_hour: 105 },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(head.reply_at(100).unwrap().complete);
        let head_late = head.reply_at(105).unwrap();
        assert_eq!(head_late.recorded_seconds, Some(6.0 * 60.0));
        assert!(!head_late.complete);
        assert_eq!(head_late.warnings, vec![INCOMPLETE_WARNING]);
    }

    #[test]
    fn unavailable_campaign_table_does_not_credit_verified_coverage() {
        let mut frame = dashboard_base::test_fixtures::source(4, 100, Some(1));
        *frame.tables[replay_data::rows::TableId::Campaign.index()] =
            replay_frame::frame::TableView::Unavailable {
                reason: "campaign table unreadable".into(),
            };
        let mut input = dashboard_base::test_fixtures::records(4, vec![frame]);
        if let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] {
            session.qpc_frequency = 1;
        }
        input.push(replay_capture_spool::spool::Record::Control(packet(
            4,
            1,
            vec![span(0, 60, 100, false)],
            |_| {},
        )));

        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[input],
        )
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "unavailable-campaign-clock".into(), revision: "1".into() },
            "unavailable-campaign-clock.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load_ctx = LoadContext { content: None };

        let exact = CampaignClock::load(
            &source,
            &CampaignClockParams { branch_id: "4".into(), first_hour: 100, last_hour: 100 },
            &load_ctx,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(exact.recorded_seconds, Some(60.0));
        assert!(!exact.complete);
        assert_eq!(exact.warnings, vec![INCOMPLETE_WARNING]);

        let head = player_apm::head::ClockHead::from_source(
            &source,
            player_apm::head::ClockHeadParams { branch_id: "4".into(), first_hour: 100, last_hour: 100 },
            &AtomicBool::new(false),
        )
        .unwrap();
        let followed = head.reply_at(100).unwrap();
        assert_eq!(followed.recorded_seconds, Some(60.0));
        assert!(!followed.complete);
        assert_eq!(followed.warnings, vec![INCOMPLETE_WARNING]);
    }
}
