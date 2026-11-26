use dashboard_base::products::{LoadContext, Product};
use super::player_apm;
use dashboard_base::Source;
use replay_capture_spool::commands::{ClockMeasurement, Index};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const OVERLAP_REASON: &str = "Overlapping recordings prevent a unique campaign clock total.";
pub const OVERLAP_WARNING: &str = "Recording intervals overlap; a unique campaign duration cannot be established.";

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
        let index = player_apm::index(source, params.branch_id.parse().unwrap(), cancelled)?;
        query(&index, params, cancelled)
    }
}

pub fn query(
    index: &Index,
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
        && regions[0].1.first_hour <= params.first_hour
        && last_hour.is_some_and(|last| last >= params.last_hour);
    if !reply.complete {
        reply.warnings.push("Only recorded intervals are included. Recording gaps are not counted as paused or playing time.".into());
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
            packet: Box::new(Packet::pack(4, capture, 1, 0, from, to, 0, &batch).unwrap()),
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
}
