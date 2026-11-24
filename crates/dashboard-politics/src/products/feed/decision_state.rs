use super::{EventFeedKind, EventFeedRow, row::FeedEvent};
use dashboard_query::Query;
use replay_data::{
    event_feed_rows::{
        EventFeedCoverage, EventFeedHistoryRow, EventFeedScriptChange,
        decisions::{Operation, REENABLE, REMOVAL, TAKEN, TARGETED, TARGETED_TIMED, TIMED},
    },
    rows::{TableId, tables},
};
use replay_lanes::at::{Keys, Read, Seeker};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const EVENT_FEED_HISTORY_NOT_RECORDED: &str = "event feed history was not recorded at this hour";
pub const EVENT_FEED_HISTORY_INCOMPLETE: &str = "event feed history was incomplete or refused at this hour";
pub const COUNTRY_DECISION_STATE_REFUSED: &str = "decision state for this country was refused at this hour";
pub const DECISION_OCCURRENCE_NOT_FOUND: &str = "decision occurrence was not found in event feed history";
pub const DECISION_CONTAINER_NOT_RECORDED: &str = "decision container or instance metadata was not recorded";
pub const DECISION_INSTANCE_MISMATCH: &str = "decision card edges belong to different decision instances";
pub const DECISION_NO_MISSION_LIFECYCLE: &str = "decision has no mission lifecycle state";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DecisionLifecycleState {
    Active,
    Available,
    Completed,
    Failed { observed_hour: i32 },
    Aborted,
    ReEnableCooldown,
    Unsupported { raw: i32 },
    NotApplicable { reason: String },
    NotRecorded { reason: String },
}

pub(crate) fn not_recorded(reason: &str) -> DecisionLifecycleState {
    DecisionLifecycleState::NotRecorded {
        reason: reason.to_owned(),
    }
}

pub(crate) fn not_applicable(reason: &str) -> DecisionLifecycleState {
    DecisionLifecycleState::NotApplicable {
        reason: reason.to_owned(),
    }
}

pub(crate) fn applies(event: &FeedEvent) -> bool {
    EventFeedKind::of_code(event.code).is_some_and(super::text::names_decision)
}

pub(crate) fn decode(container: u8, lifecycle: i32, observed_hour: i32) -> DecisionLifecycleState {
    match container {
        TIMED => match lifecycle {
            0 => DecisionLifecycleState::Active,
            1 => DecisionLifecycleState::Completed,
            2 => DecisionLifecycleState::Failed { observed_hour },
            3 => DecisionLifecycleState::Aborted,
            4 => DecisionLifecycleState::ReEnableCooldown,
            raw => DecisionLifecycleState::Unsupported { raw },
        },
        TARGETED | TARGETED_TIMED => match lifecycle {
            0 => DecisionLifecycleState::Available,
            1 => DecisionLifecycleState::Completed,
            2 => DecisionLifecycleState::ReEnableCooldown,
            3 => DecisionLifecycleState::Failed { observed_hour },
            4 => DecisionLifecycleState::Aborted,
            raw => DecisionLifecycleState::Unsupported { raw },
        },
        TAKEN | REENABLE | REMOVAL => not_applicable(DECISION_NO_MISSION_LIFECYCLE),
        _ => not_recorded(DECISION_CONTAINER_NOT_RECORDED),
    }
}

pub(crate) fn resolve_in_pack(event: &FeedEvent, pack: &EventFeedHistoryRow) -> DecisionLifecycleState {
    let Some(actor_id) = event.actor.id else {
        return not_recorded(DECISION_OCCURRENCE_NOT_FOUND);
    };
    if pack.generation != event.generation
        || pack.read_hour != event.read_hour
        || pack.coverage != EventFeedCoverage::Complete
        || pack.script_refusals != 0
    {
        return not_recorded(EVENT_FEED_HISTORY_INCOMPLETE);
    }
    let owner_refused = pack.events.iter().any(|ev| {
        ev.generation == event.generation
            && ev.actor.id == actor_id
            && matches!(
                ev.script.and_then(|script| script.change),
                Some(EventFeedScriptChange::Decision(change))
                    if matches!(change.operation, Operation::Refused | Operation::Invalidate)
            )
    });
    if owner_refused {
        return not_recorded(COUNTRY_DECISION_STATE_REFUSED);
    }
    let mut matched = pack
        .events
        .iter()
        .filter(|ev| ev.generation == event.generation && ev.sequence == event.sequence);
    let (Some(ev), None) = (matched.next(), matched.next()) else {
        return not_recorded(DECISION_OCCURRENCE_NOT_FOUND);
    };
    if ev.source as u8 != event.code || ev.actor.id != actor_id {
        return not_recorded(DECISION_OCCURRENCE_NOT_FOUND);
    }
    let Some(script) = ev.script else {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    };
    let change = match script.change {
        Some(EventFeedScriptChange::Decision(change)) => Some(change),
        Some(_) => return not_recorded(DECISION_CONTAINER_NOT_RECORDED),
        None => None,
    };
    if change.is_some_and(|change| matches!(change.container, TAKEN | REENABLE | REMOVAL)) {
        return not_applicable(DECISION_NO_MISSION_LIFECYCLE);
    }
    if event.lifecycle.is_none() && script.lifecycle.is_none() {
        let expects_lifecycle = matches!(
            EventFeedKind::of_code(event.code),
            Some(EventFeedKind::DecisionTimedOut | EventFeedKind::DecisionCompleted | EventFeedKind::DecisionActivated | EventFeedKind::DecisionDurationChanged)
        );
        if expects_lifecycle {
            return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
        }
        return not_applicable(DECISION_NO_MISSION_LIFECYCLE);
    }
    let Some(change) = change else {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    };
    if !matches!(change.operation, Operation::Store | Operation::Update | Operation::PassCut) {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    }
    let Some(instance) = script.instance else {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    };
    if event.instance.is_some_and(|recorded| recorded != instance) {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    }
    let Some(lifecycle) = script.lifecycle else {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    };
    if event.lifecycle != Some(lifecycle) {
        return not_recorded(DECISION_CONTAINER_NOT_RECORDED);
    }
    decode(change.container, lifecycle, event.read_hour)
}

pub(crate) fn resolve_all(query: &Query<'_>, events: &[&FeedEvent]) -> io::Result<Vec<Option<DecisionLifecycleState>>> {
    let mut out: Vec<Option<DecisionLifecycleState>> = vec![None; events.len()];
    let mut wanted: BTreeMap<(u32, i32), Vec<usize>> = BTreeMap::new();
    let recording = query.replay.recording();
    let unavailable = recording.predates(TableId::EventFeedHistory.index()).is_some()
        || recording.withheld_reason(TableId::EventFeedHistory.index()).is_some();
    for (index, event) in events.iter().enumerate() {
        if !applies(event) {
            continue;
        }
        if unavailable {
            out[index] = Some(not_recorded(EVENT_FEED_HISTORY_NOT_RECORDED));
            continue;
        }
        let Some(stretch) = query.timeline.at(event.read_hour) else {
            out[index] = Some(not_recorded(EVENT_FEED_HISTORY_NOT_RECORDED));
            continue;
        };
        wanted.entry((stretch.run, event.read_hour)).or_default().push(index);
    }
    if wanted.is_empty() {
        return Ok(out);
    }
    let mut seeker = Seeker::new(recording)?;
    let read = [Read {
        table: TableId::EventFeedHistory,
        columns: None,
        keys: Keys::These(&[0]),
    }];
    for ((run, read_hour), indices) in wanted {
        let frame = dashboard_query::cost::storage(|| seeker.frame(run, read_hour, &read))?;
        let sampled = frame.tables[TableId::EventFeedHistory.index()].sampled_at() == Some(read_hour);
        let pack = sampled
            .then(|| frame.complete::<tables::EventFeedHistory>().ok().and_then(|rows| rows.first()))
            .flatten();
        for index in indices {
            out[index] = Some(match pack {
                Some(pack) => resolve_in_pack(events[index], pack),
                None => not_recorded(EVENT_FEED_HISTORY_NOT_RECORDED),
            });
        }
    }
    Ok(out)
}

pub(crate) fn enforce_same_incarnation(events: &[&FeedEvent], rows: &mut [Option<EventFeedRow>]) {
    let mut seen: Option<u32> = None;
    let mut mismatched = false;
    for (event, row) in events.iter().zip(rows.iter_mut()) {
        if let Some(instance) = event.instance {
            match seen {
                None => seen = Some(instance),
                Some(first) if first != instance => mismatched = true,
                Some(_) => {}
            }
        }
        if mismatched
            && let Some(row) = row.as_mut()
            && row.decision_state.is_some()
        {
            row.decision_state = Some(not_recorded(DECISION_INSTANCE_MISMATCH));
        }
    }
}
