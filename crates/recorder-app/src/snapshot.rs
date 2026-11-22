use crate::flags::{FlagImage, flag_keys};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Waiting,
    NotRecording,
    Recording,
    Saving,
    Saved,
    Discarded,
    Unsaved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OmissionReason {
    Off,
    Elsewhere,
    Settings,
    NotAdmitted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OmittedLoads {
    pub loads: usize,
    pub reason: Option<OmissionReason>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub text: String,
    pub detail: String,
    pub failed: bool,
}

impl Problem {
    pub fn failure(text: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            detail: detail.into(),
            failed: true,
        }
    }

    pub fn caveat(text: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            detail: detail.into(),
            failed: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub phase: Phase,
    pub campaign: Option<String>,
    pub recording: Option<String>,
    pub name: Option<String>,
    pub continued: bool,
    pub hour: Option<i32>,
    pub first_hour: Option<i32>,
    pub saved_bytes: Option<u64>,
    pub spool_bytes: Option<u64>,
    pub started_at: Option<u64>,
    pub stopping: bool,
    pub problem: Option<Problem>,
    pub automatic: bool,
    pub session: Option<String>,
    pub flag: Option<FlagImage>,
    pub omitted: Option<OmittedLoads>,
    pub capture_mode: Option<&'static str>,
    pub build_warning: Option<String>,
    pub behind: Option<u64>,
    pub session_key: Option<u64>,
    pub prompt: Option<crate::panel::Prompt>,
    pub revision: u64,
}

#[derive(Debug, Default, PartialEq)]
pub struct Live {
    pub campaign: Option<String>,
    pub hour: Option<i32>,
    pub hours: Option<u64>,
    pub spool_bytes: Option<u64>,
    pub problem: Option<Problem>,
    pub flag_keys: Option<Vec<String>>,
    pub build_warning: Option<String>,
    pub behind: Option<u64>,
}

fn behind(status: &Value) -> Option<u64> {
    let frames = &status["acquisition_work"]["frames"];
    let in_flight = frames["in_flight"].as_u64()?;
    (in_flight > frames["steady"].as_u64()?).then_some(in_flight)
}

pub fn read_live(status: &Value) -> Live {
    let hours = status["hours"].as_u64();
    let campaign = status["campaign"]
        .as_str()
        .filter(|id| replay_data::identity::is_campaign_uuid(id.as_bytes()))
        .map(str::to_owned);
    let hour = hours
        .is_some_and(|hours| hours > 0)
        .then(|| status["last_hour"].as_i64())
        .flatten()
        .and_then(|hour| i32::try_from(hour).ok());
    let detail = ["error", "last_error", "warning"]
        .into_iter()
        .find_map(|key| status[key].as_str().filter(|text| !text.trim().is_empty()))
        .unwrap_or_default()
        .to_owned();
    let dropped = status["dropped_frames"].as_u64().unwrap_or(0);
    let stalled = status["dropped_spool_stalled"].as_u64().unwrap_or(0);
    let problem = if status["capture_failed"] == true {
        Some(Problem::failure(
            "Recording stopped early. Recovering the captured part.",
            detail,
        ))
    } else if dropped > 0 {
        let hours = |n: u64| if n == 1 { "1 hour".to_owned() } else { format!("{n} hours") };
        let lost = format!("{} couldn’t be recorded", hours(dropped));
        let text = if stalled >= dropped {
            format!("{lost}: the recorder fell behind.")
        } else if stalled > 0 {
            format!("{lost}, {stalled} of them because the recorder fell behind.")
        } else {
            format!("{lost}.")
        };
        Some(Problem::caveat(text, detail))
    } else {
        None
    };
    Live {
        campaign,
        hour,
        hours,
        spool_bytes: status["spool_bytes"].as_u64(),
        problem,
        flag_keys: flag_keys(&status["player"]),
        build_warning: status["build"]["warning"].as_str().map(str::to_owned),
        behind: behind(status),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Ready,
    Recording,
    Saving,
    Problem,
}

impl Snapshot {
    pub fn light(&self) -> Light {
        if self.problem.is_some() {
            Light::Problem
        } else {
            match self.phase {
                Phase::Recording => Light::Recording,
                Phase::Saving => Light::Saving,
                _ => Light::Ready,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";

    #[test]
    fn live_status_carries_the_viewed_countrys_flag_keys() {
        let live = read_live(&json!({
            "campaign": CAMPAIGN, "hours": 2, "last_hour": 9,
            "player": {"tag":"OWN","cosmetic":"","original":"OWN","group":"synthetic_group"},
        }));
        assert_eq!(live.flag_keys.unwrap(), ["OWN_synthetic_group", "OWN"]);
        assert_eq!(
            read_live(&json!({"campaign": CAMPAIGN, "hours": 2})).flag_keys,
            None
        );
    }

    #[test]
    fn live_status_names_the_campaign_hour_size_and_lost_hours() {
        let live = read_live(&json!({
            "campaign": CAMPAIGN, "hours": 12, "last_hour": 59_000_000,
            "spool_bytes": 4096, "dropped_frames": 2, "last_error": "queue full",
        }));
        assert_eq!(live.campaign.as_deref(), Some(CAMPAIGN));
        assert_eq!(live.hour, Some(59_000_000));
        assert_eq!((live.hours, live.spool_bytes), (Some(12), Some(4096)));
        assert_eq!(
            live.problem,
            Some(Problem::caveat(
                "2 hours couldn’t be recorded.",
                "queue full"
            ))
        );
        let one = read_live(
            &json!({"campaign": CAMPAIGN, "hours": 3, "last_hour": 1, "dropped_frames": 1}),
        );
        assert_eq!(one.problem.unwrap().text, "1 hour couldn’t be recorded.");
        let stalled = |dropped: u64, stalled: u64| {
            read_live(&json!({
                "campaign": CAMPAIGN, "hours": 3, "last_hour": 1,
                "dropped_frames": dropped, "dropped_spool_stalled": stalled,
            }))
            .problem
            .unwrap()
            .text
        };
        assert_eq!(
            stalled(5, 3),
            "5 hours couldn’t be recorded, 3 of them because the recorder fell behind."
        );
        assert_eq!(
            stalled(1, 1),
            "1 hour couldn’t be recorded: the recorder fell behind."
        );
        assert_eq!(stalled(2, 0), "2 hours couldn’t be recorded.");
    }

    #[test]
    fn live_status_names_the_hours_behind_only_past_the_steady_frames() {
        let status = |in_flight: u64| {
            json!({
                "campaign": CAMPAIGN, "hours": 40, "last_hour": 9,
                "acquisition_work": {"frames": {
                    "steady": 4, "live": in_flight.max(4), "free": 0,
                    "in_flight": in_flight, "in_flight_bytes": in_flight * 1_000,
                }},
            })
        };
        assert_eq!(read_live(&status(9)).behind, Some(9));
        assert_eq!(read_live(&status(5)).behind, Some(5));
        assert_eq!(read_live(&status(4)).behind, None);
        assert_eq!(read_live(&status(0)).behind, None);
        assert_eq!(read_live(&json!({"campaign": CAMPAIGN, "hours": 2})).behind, None);
    }

    #[test]
    fn preparing_or_unreadable_status_claims_nothing() {
        let live = read_live(
            &json!({"state": "preparing campaign history", "campaign": "save name", "hours": 0, "last_hour": 7}),
        );
        assert_eq!(
            live,
            Live {
                hours: Some(0),
                ..Live::default()
            }
        );
        let unknown = read_live(&json!(null));
        assert_eq!(unknown, Live::default());
        assert_eq!((unknown.hours, unknown.spool_bytes), (None, None));
        let unread = read_live(&json!({"campaign": CAMPAIGN, "state": "starting"}));
        assert_eq!((unread.hours, unread.spool_bytes), (None, None));
        assert_eq!(
            read_live(&json!({"hours": 0, "spool_bytes": 0})).spool_bytes,
            Some(0)
        );
    }

    #[test]
    fn a_failed_capture_is_a_problem_even_without_lost_hours() {
        let live = read_live(
            &json!({"capture_failed": true, "error": "collector allocation failed", "hours": 5, "last_hour": 9}),
        );
        assert_eq!(
            live.problem,
            Some(Problem::failure(
                "Recording stopped early. Recovering the captured part.",
                "collector allocation failed"
            ))
        );
    }

    #[test]
    fn tray_light_follows_phase_with_problems_first() {
        let mut snapshot = Snapshot::default();
        for (phase, light) in [
            (Phase::Waiting, Light::Ready),
            (Phase::NotRecording, Light::Ready),
            (Phase::Saved, Light::Ready),
            (Phase::Discarded, Light::Ready),
            (Phase::Unsaved, Light::Ready),
            (Phase::Recording, Light::Recording),
            (Phase::Saving, Light::Saving),
        ] {
            snapshot.phase = phase;
            assert_eq!(snapshot.light(), light, "{phase:?}");
        }
        snapshot.problem = Some(Problem::caveat("2 hours couldn’t be recorded.", ""));
        for phase in [Phase::Waiting, Phase::Recording, Phase::Saving] {
            snapshot.phase = phase;
            assert_eq!(snapshot.light(), Light::Problem, "{phase:?}");
        }
    }
}
