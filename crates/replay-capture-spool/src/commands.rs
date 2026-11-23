use replay_recording::control::ControlRecord;
use replay_recording::command_packet::{Actor, Category, Origin, Span};
use replay_data::codec::{Decoder, invalid_static as invalid};
use crate::spool::decode_record;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

mod index;
pub use index::{ClockMeasurement, CountSeries, Counts, Index, Measurement, Timeline};

#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub struct Recording {
    pub sessions: Vec<replay_data::identity::SessionStart>,
    pub branches: Vec<replay_frame::branches::BranchStart>,
    pub controls: Vec<ControlRecord>,
}

impl Recording {
    pub fn from_columnar(capture: replay_recording::capture::Capture) -> io::Result<Self> {
        use replay_data::identity::IdentityCodec;
        replay_data::format::FormatVersion::read(capture.format)?;
        let mut sessions = engine_allocation::vec(capture.sessions.len())?;
        for bytes in capture.sessions {
            let mut input = Decoder::new(&bytes);
            input.format = capture.format;
            let session = replay_data::identity::SessionStart::decode(&mut input)?;
            input.finish()?;
            sessions.push(session);
        }
        let mut branches = BTreeMap::new();
        let mut controls = engine_allocation::vec(capture.controls.len())?;
        for bytes in capture.controls {
            match decode_record(&bytes)? {
                crate::spool::Record::BranchStart(start) => {
                    if let Some(previous) = branches.insert(start.id, start.clone()) {
                        if previous.session != start.session {
                            return Err(invalid("conflicting columnar command branch identity"));
                        }
                    }
                }
                crate::spool::Record::Control(control) => controls.push(control),
                crate::spool::Record::Gap(_) => {}
                _ => return Err(invalid("unexpected columnar command metadata record")),
            }
        }
        for control in &controls {
            if let ControlRecord::PlayerCommands { session, packet } = control
                && let Some(phase) = &packet.phase
            {
                phase.validate()?;
                if phase.start.session != *session
                    || phase.start.id != packet.branch
                    || !phase.campaign.eq_ignore_ascii_case(&capture.campaign_id)
                {
                    return Err(invalid("conflicting columnar command phase identity"));
                }
                if let Some(previous) = branches.insert(phase.start.id, phase.start.clone())
                    && previous != phase.start
                {
                    return Err(invalid("conflicting columnar command branch declaration"));
                }
            }
        }
        for control in &controls {
            if !sessions
                .iter()
                .any(|session| session.id == control.session())
            {
                return Err(invalid("columnar command metadata has an unknown session"));
            }
            if let ControlRecord::PlayerCommands { session, packet } = control {
                if branches.get(&packet.branch).map(|branch| branch.session) != Some(*session) {
                    return Err(invalid(
                        "columnar commands refer to another session's branch",
                    ));
                }
            }
        }
        Ok(Self {
            sessions,
            branches: branches.into_values().collect(),
            controls,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct PlayerSummary {
    pub session: u64,
    pub capture: u64,
    pub world: u64,
    pub actor: Actor,
    pub commands: u64,
    pub unresolved_commands: u64,
    pub unclassified_commands: u64,
    pub coverage_complete: bool,
    pub confirmed_orders: u64,
    pub paused_orders: Option<u64>,
    pub paused_unresolved_commands: u64,
    pub categories: BTreeMap<Category, u64>,
    pub unresolved_kinds: BTreeMap<String, u64>,
    pub observed_minutes: Option<f64>,
    pub confirmed_orders_per_minute: Option<f64>,
    pub apm: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Analysis {
    pub recorded: bool,
    pub lost_observations: u64,
    pub unattributed_commands: u64,
    pub players: Vec<PlayerSummary>,
}

pub fn analyze(
    sessions: &[replay_data::identity::SessionStart],
    controls: &[ControlRecord],
    branch: u64,
) -> io::Result<Analysis> {
    Ok(index(sessions, controls, branch)?.analysis)
}

pub fn index(
    sessions: &[replay_data::identity::SessionStart],
    controls: &[ControlRecord],
    branch: u64,
) -> io::Result<Index> {
    index_with(sessions, controls, branch, || Ok(()))
}

pub fn index_with(
    sessions: &[replay_data::identity::SessionStart],
    controls: &[ControlRecord],
    branch: u64,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<Index> {
    #[derive(Default)]
    struct Window {
        spans: Vec<Span>,
        intervals: Vec<(u64, u64)>,
        complete: bool,
        next: u64,
        to: Option<u64>,
        actors: BTreeMap<u64, Vec<Actor>>,
        counts: BTreeMap<(u64, usize), CountSeries>,
        unknown_hours: BTreeSet<i32>,
        paused_unknown_hours: BTreeSet<i32>,
        catalogued: bool,
        timing: index::Timing,
    }
    let mut limits: BTreeMap<(u64, u64, u64), u64> = BTreeMap::new();
    for control in controls {
        check()?;
        if let ControlRecord::PlayerCommands { session, packet } = control
            && packet.branch == branch
        {
            limits
                .entry((*session, packet.capture, packet.world))
                .and_modify(|last| *last = (*last).max(packet.ordinal))
                .or_insert(packet.ordinal);
        }
    }
    let mut kinds = BTreeMap::new();
    let mut summaries: BTreeMap<(u64, u64, u64, u64), PlayerSummary> = BTreeMap::new();
    let mut departures = BTreeMap::new();
    let mut windows: BTreeMap<(u64, u64, u64), Window> = BTreeMap::new();
    let mut packets = BTreeMap::new();
    let mut lost_observations = 0_u64;
    let mut unattributed_commands = 0_u64;
    for control in controls {
        let ControlRecord::PlayerCommands { session, packet } = control else {
            continue;
        };
        let key = (*session, packet.capture, packet.world);
        if !limits.get(&key).is_some_and(|last| packet.ordinal <= *last) {
            continue;
        }
        if let Some(previous) = packets.insert((key, packet.ordinal), packet) {
            if previous != packet {
                return Err(invalid("conflicting command packet identity"));
            }
        }
    }
    for ((key, _), packet) in packets {
        check()?;
        let window = windows.entry(key).or_insert_with(|| Window {
            complete: true,
            ..Window::default()
        });
        let packet_complete = packet.ordinal == window.next
            && packet.lost == 0
            && window.to.is_none_or(|to| to == packet.from_qpc);
        window.complete &= packet_complete;
        if window.to.is_some_and(|to| to != packet.from_qpc) {
            window.complete = false;
        }
        window.to = Some(packet.to_qpc);
        window.next = packet
            .ordinal
            .checked_add(1)
            .ok_or_else(|| invalid("command packet ordinal overflow"))?;
        let batch = packet.unpack()?;
        if packet.ordinal == 0 && batch.kinds.is_empty() {
            window.complete = false;
        }
        for kind in batch.kinds {
            window.catalogued = true;
            if kinds.insert((key, kind.id), kind).is_some() {
                return Err(invalid("repeated command kind definition"));
            }
        }
        for actor in batch.actors {
            let actor_key = (key.0, key.1, key.2, actor.id);
            if actor.id == 0 || actor.since_qpc > actor.at_qpc || actor.at_qpc > packet.to_qpc {
                return Err(invalid("invalid command actor identity"));
            }
            let history = window.actors.entry(actor.id).or_default();
            if history.last().is_none_or(|previous| previous != &actor) {
                if history
                    .last()
                    .is_some_and(|previous| previous.at_qpc > actor.at_qpc)
                {
                    return Err(invalid("command actor observations out of order"));
                }
                history.push(actor.clone());
            }
            if let Some(previous) = summaries.get_mut(&actor_key) {
                if previous.actor.player != actor.player
                    || previous.actor.since_qpc != actor.since_qpc
                {
                    return Err(invalid("command actor identity changed"));
                }
                previous.actor = actor;
            } else {
                summaries.insert(
                    actor_key,
                    PlayerSummary {
                        session: key.0,
                        capture: packet.capture,
                        world: packet.world,
                        actor,
                        commands: 0,
                        unresolved_commands: 0,
                        unclassified_commands: 0,
                        coverage_complete: false,
                        confirmed_orders: 0,
                        paused_orders: Some(0),
                        paused_unresolved_commands: 0,
                        categories: BTreeMap::new(),
                        unresolved_kinds: BTreeMap::new(),
                        observed_minutes: None,
                        confirmed_orders_per_minute: None,
                        apm: None,
                    },
                );
            }
        }
        for (actor, time) in batch.departures {
            if time < packet.from_qpc
                || time > packet.to_qpc
                || departures
                    .insert((key.0, key.1, key.2, actor), time)
                    .is_some()
            {
                return Err(invalid("invalid or repeated command player departure"));
            }
        }
        if packet.branch != branch {
            continue;
        }
        lost_observations = lost_observations
            .checked_add(packet.lost)
            .ok_or_else(|| invalid("command loss count overflow"))?;
        if let Some((_, to)) = window.intervals.last_mut()
            && *to == packet.from_qpc
        {
            *to = packet.to_qpc;
        } else {
            window.intervals.push((packet.from_qpc, packet.to_qpc));
        }
        for mut span in batch.spans {
            span.complete &= packet_complete && window.catalogued;
            if window
                .spans
                .last()
                .is_some_and(|previous| previous.to_qpc > span.from_qpc)
            {
                return Err(invalid("overlapping command coverage intervals"));
            }
            window.complete &= span.complete;
            window.spans.push(span);
        }
        for event in batch.events {
            if !window
                .spans
                .iter()
                .rev()
                .take_while(|span| span.to_qpc >= packet.from_qpc)
                .any(|span| {
                    span.from_qpc <= event.qpc
                        && event.qpc <= span.to_qpc
                        && span.paused == event.paused
                        && span.hour == event.hour
                })
            {
                window.complete = false;
                window.unknown_hours.insert(event.hour);
            }
            let actor_key = (key.0, key.1, key.2, event.actor);
            let Some(summary) = summaries.get_mut(&actor_key) else {
                unattributed_commands = unattributed_commands
                    .checked_add(1)
                    .ok_or_else(|| invalid("command count overflow"))?;
                if event.paused {
                    window.paused_unknown_hours.insert(event.hour);
                } else {
                    window.complete = false;
                    window.unknown_hours.insert(event.hour);
                }
                continue;
            };
            if summary.actor.player != event.player
                || event.qpc < summary.actor.since_qpc
                || departures
                    .get(&actor_key)
                    .is_some_and(|&end| event.qpc > end)
            {
                return Err(invalid("command actor outside its recorded lifetime"));
            }
            let Some(kind) = kinds.get(&(key, event.kind)) else {
                if event.paused {
                    window.paused_unknown_hours.insert(event.hour);
                } else {
                    window.complete = false;
                    window.unknown_hours.insert(event.hour);
                }
                continue;
            };
            if event.origin == Origin::AutonomousCountry {
                continue;
            }
            let history = &window.actors[&event.actor];
            let version = history
                .partition_point(|actor| actor.at_qpc <= event.qpc)
                .checked_sub(1)
                .ok_or_else(|| invalid("command precedes its player observation"))?;
            window
                .counts
                .entry((event.actor, version))
                .or_default()
                .observe(&event, kind.category)?;
            if event.paused {
                if kind.category == Category::Other {
                    summary.paused_unresolved_commands += 1;
                }
                if kind.category.counted() {
                    if event.origin == Origin::PlayerOrder {
                        *summary.paused_orders.as_mut().unwrap() += 1;
                    } else {
                        summary.paused_unresolved_commands += 1;
                    }
                }
                continue;
            }
            summary.unclassified_commands += u64::from(kind.category == Category::Other);
            if !kind.category.counted() {
                continue;
            }
            summary.commands += 1;
            if event.origin == Origin::PlayerOrder {
                summary.confirmed_orders += 1;
                *summary.categories.entry(kind.category).or_default() += 1;
            } else {
                summary.unresolved_commands += 1;
                *summary
                    .unresolved_kinds
                    .entry(kind.name.clone())
                    .or_default() += 1;
            }
        }
    }
    for window in windows.values_mut() {
        window.timing = index::Timing::new(&window.spans)?;
    }
    for ((session, capture, world, actor), summary) in &mut summaries {
        let frequency = sessions
            .iter()
            .find(|entry| entry.id == *session)
            .map(|entry| entry.qpc_frequency)
            .filter(|&value| value != 0)
            .ok_or_else(|| invalid("command session frequency missing"))?;
        let window = &windows[&(*session, *capture, *world)];
        let until = departures
            .get(&(*session, *capture, *world, *actor))
            .copied()
            .unwrap_or(u64::MAX);
        let mut ticks = 0_u64;
        let history = &window.actors[actor];
        for (version, observation) in history.iter().enumerate() {
            if observation.country == 0 {
                continue;
            }
            let end = history
                .get(version + 1)
                .map_or(until, |next| next.at_qpc.min(until));
            let duration = window
                .timing
                .between(&window.spans, observation.at_qpc, end);
            ticks = ticks
                .checked_add(
                    u64::try_from(duration).map_err(|_| invalid("command duration overflow"))?,
                )
                .ok_or_else(|| invalid("command duration overflow"))?;
        }
        let minutes = ticks as f64 / frequency as f64 / 60.0;
        summary.observed_minutes = window.complete.then_some(minutes);
        summary.coverage_complete = window.complete;
        if !window.complete
            || !window.paused_unknown_hours.is_empty()
            || summary.paused_unresolved_commands != 0
        {
            summary.paused_orders = None;
        }
        summary.confirmed_orders_per_minute =
            (window.complete && ticks != 0).then(|| summary.confirmed_orders as f64 / minutes);
        summary.apm = summary
            .confirmed_orders_per_minute
            .filter(|_| summary.unresolved_commands == 0 && summary.unclassified_commands == 0);
    }
    let analysis = Analysis {
        recorded: !limits.is_empty(),
        lost_observations,
        unattributed_commands,
        players: summaries
            .into_iter()
            .filter(|(key, summary)| {
                let until = departures.get(key).copied().unwrap_or(u64::MAX);
                windows[&(key.0, key.1, key.2)]
                    .intervals
                    .iter()
                    .any(|&(from, to)| summary.actor.since_qpc <= to && until >= from)
            })
            .map(|(_, summary)| summary)
            .collect(),
    };
    let mut timelines = engine_allocation::vec(windows.len())?;
    for ((session, capture, world), window) in windows {
        let frequency = sessions
            .iter()
            .find(|entry| entry.id == session)
            .map(|entry| entry.qpc_frequency)
            .filter(|&value| value != 0)
            .ok_or_else(|| invalid("command session frequency missing"))?;
        let actor_departures = departures
            .iter()
            .filter_map(|(&(s, c, w, actor), &time)| {
                ((s, c, w) == (session, capture, world)).then_some((actor, time))
            })
            .collect();
        let mut timeline = Timeline {
            hours: (i32::MIN, i32::MAX),
            session,
            capture,
            world,
            frequency,
            complete: window.complete,
            spans: window.spans,
            actors: window.actors,
            departures: actor_departures,
            counts: window.counts,
            unknown_hours: window.unknown_hours,
            paused_unknown_hours: window.paused_unknown_hours,
            timing: window.timing,
        };
        timeline.prepare()?;
        timelines.push(timeline);
    }
    Ok(Index {
        analysis,
        timelines,
    })
}
