use super::{WarCapitulation, WarObservation, WarParticipantRun, WarTimeline, WarsReply};
use dashboard_base::span::HourSpan;
use dashboard_world::products::groups::MembershipRun;
use serde::Serialize;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarTimelineReply {
    pub branch_id: String,
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which a war, a participant, a capitulation, a name, the viewpoint or an"]
    #[doc = " unread stretch changes, other than held ones running on; null when none does."]
    pub next_change: Option<i32>,
    #[doc = " The war tables were read at some hour of the timeline."]
    pub available: bool,
    pub viewpoints: Vec<MembershipRun>,
    #[doc = " Every war begun by `hour`, cut at it: ongoing first, newest begun first, then the ended ones, latest"]
    #[doc = " ended first. A participant who joins after `hour` is not in it; one held at `hour` ends there."]
    pub wars: Vec<WarTimeline>,
    pub unobserved: Vec<HourSpan>,
    pub capitulations_unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WarTimelineReply);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum WarState {
    Ongoing,
    Ended,
}

fn cut_spans(spans: &[HourSpan], hour: i32) -> Vec<HourSpan> {
    spans.iter().filter(|span| span.first_hour <= hour).map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour.min(hour) }).collect()
}

struct Next(Option<i32>, i32);

impl Next {
    fn see(&mut self, at: i32) {
        if at > self.1 && self.0.is_none_or(|next| at < next) {
            self.0 = Some(at);
        }
    }

    fn span(&mut self, first: i32, last: i32) {
        self.see(first);
        self.see(last.saturating_add(1));
    }
}

fn cut_war(war: &WarTimeline, hour: i32) -> WarTimeline {
    let mut cut = war.clone();
    cut.last_hour = war.last_hour.min(hour);
    if war.last_hour >= hour {
        cut.ended_by = false;
    }
    cut.observations = war
        .observations
        .iter()
        .filter(|seen| seen.first_hour <= hour)
        .map(|seen| WarObservation { last_hour: seen.last_hour.min(hour), ..seen.clone() })
        .collect();
    cut.participants = war
        .participants
        .iter()
        .filter(|run| run.first_hour <= hour)
        .map(|run| WarParticipantRun { last_hour: run.last_hour.min(hour), left_by: run.left_by && run.last_hour < hour, ..run.clone() })
        .collect();
    cut.capitulations = war.capitulations.iter().filter(|capitulation| capitulation.hour <= hour).cloned().collect::<Vec<WarCapitulation>>();
    cut
}

pub(crate) fn at(reply: &WarsReply, hour: i32) -> WarTimelineReply {
    let mut next = Next(None, hour);
    for war in &reply.wars {
        next.span(war.first_hour, war.last_hour);
        for run in &war.participants {
            next.span(run.first_hour, run.last_hour);
        }
        for seen in &war.observations {
            next.span(seen.first_hour, seen.last_hour);
        }
        for capitulation in &war.capitulations {
            next.see(capitulation.hour);
        }
    }
    for span in reply.unobserved.iter().chain(&reply.capitulations_unobserved) {
        next.span(span.first_hour, span.last_hour);
    }
    for run in &reply.viewpoints {
        next.span(run.first_hour, run.last_hour);
    }
    let mut wars: Vec<(WarState, i32, WarTimeline)> = reply
        .wars
        .iter()
        .filter(|war| war.first_hour <= hour)
        .map(|war| {
            let state = if war.last_hour >= hour { WarState::Ongoing } else { WarState::Ended };
            (state, if state == WarState::Ongoing { war.first_hour } else { war.last_hour }, cut_war(war, hour))
        })
        .collect();
    wars.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then_with(|| a.2.id.len().cmp(&b.2.id.len()).then_with(|| a.2.id.cmp(&b.2.id))));
    WarTimelineReply {
        branch_id: reply.branch_id.clone(),
        hour,
        next_change: next.0,
        available: reply.available,
        viewpoints: reply
            .viewpoints
            .iter()
            .filter(|run| run.first_hour <= hour)
            .map(|run| MembershipRun { last_hour: run.last_hour.min(hour), ..run.clone() })
            .collect(),
        wars: wars.into_iter().map(|(_, _, war)| war).collect(),
        unobserved: cut_spans(&reply.unobserved, hour),
        capitulations_unobserved: cut_spans(&reply.capitulations_unobserved, hour),
        warnings: reply.warnings.clone(),
    }
}
