//! What the panel and tray show: one serializable snapshot, and the reading
//! of the collector's live status into it.
use crate::flags::{FlagImage, flag_keys};
use serde::Serialize;
use serde_json::Value;

/// The panel states of the approved design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// the game isn't running, or no campaign is loaded yet.
    #[default]
    Waiting,
    /// A game runs but this recorder is not recording it.
    NotRecording,
    Recording,
    Saving,
    Saved,
    Discarded,
}

/// Why loads were left out of a session's recordings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OmissionReason {
    /// Record automatically was off for the campaign.
    Off,
    /// The load belongs in another recording: its campaign's folder or a new file.
    Elsewhere,
    /// The recorder settings could not be read or updated.
    Settings,
    /// The load was never admitted (it could not be identified).
    NotAdmitted,
}

/// Loads a session recorded hours of but left out of its recordings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OmittedLoads {
    pub loads: usize,
    /// Why, when it is the same for every one.
    pub reason: Option<OmissionReason>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// Plain words for the panel and tooltip.
    pub text: String,
    /// The original message, for Details.
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub phase: Phase,
    /// Recorded campaign id, when known.
    pub campaign: Option<String>,
    /// The recording this campaign saves into, or was saved to.
    pub recording: Option<String>,
    /// That recording already held an earlier recording when this session began.
    pub continued: bool,
    /// The current (or last saved) game hour.
    pub hour: Option<i32>,
    /// The first hour of a saved recording.
    pub first_hour: Option<i32>,
    /// Bytes on disk for this recording.
    pub bytes: Option<u64>,
    /// Saving progress, 0 to 1, when it can be told.
    pub progress: Option<f64>,
    /// When this session began recording, in milliseconds since the Unix epoch.
    pub started_at: Option<u64>,
    /// A stop was requested and is being carried out.
    pub stopping: bool,
    pub problem: Option<Problem>,
    /// Campaigns start recording when they load.
    pub automatic: bool,
    /// The session folder, for Details.
    pub session: Option<String>,
    /// The flag of the country the recording client views, when it resolves
    /// through the recording's content stack.
    pub flag: Option<FlagImage>,
    /// Loads the finished session recorded but left out of its recordings.
    pub omitted: Option<OmittedLoads>,
}

/// The fields of one collector status publication the panel uses.
#[derive(Debug, Default, PartialEq)]
pub struct Live {
    pub campaign: Option<String>,
    pub hour: Option<i32>,
    /// Hours processed since attachment, across campaigns.
    pub hours: u64,
    /// This session's spool bytes, across campaigns.
    pub spool_bytes: u64,
    pub problem: Option<Problem>,
    /// The flag file keys the game tries for the viewed country, in order.
    pub flag_keys: Option<Vec<String>>,
}

pub fn read_live(status: &Value) -> Live {
    let hours = status["hours"].as_u64().unwrap_or(0);
    let campaign = status["campaign"]
        .as_str()
        .filter(|id| replay::identity::is_campaign_uuid(id.as_bytes()))
        .map(str::to_owned);
    let hour = (hours > 0)
        .then(|| status["last_hour"].as_i64())
        .flatten()
        .and_then(|hour| i32::try_from(hour).ok());
    let detail = ["error", "last_error", "warning"]
        .into_iter()
        .find_map(|key| status[key].as_str().filter(|text| !text.trim().is_empty()))
        .unwrap_or_default()
        .to_owned();
    let dropped = status["dropped_frames"].as_u64().unwrap_or(0);
    let problem = if status["capture_failed"] == true {
        Some(Problem {
            text: "Recording stopped early. Recovering the captured part.".into(),
            detail,
        })
    } else if dropped > 0 {
        Some(Problem {
            text: if dropped == 1 {
                "1 hour couldn't be recorded".into()
            } else {
                format!("{dropped} hours couldn't be recorded")
            },
            detail,
        })
    } else {
        None
    };
    Live {
        campaign,
        hour,
        hours,
        spool_bytes: status["spool_bytes"].as_u64().unwrap_or(0),
        problem,
        flag_keys: flag_keys(&status["player"]),
    }
}

/// Finalization reports work done, not a total. Each recorded hour costs about
/// four steps (spool read, frame read, encode check, block write), so the bar
/// advances with real work and stops short of full until saving ends.
pub fn saving_progress(records: u64, hours: u64) -> Option<f64> {
    (hours > 0).then(|| (records as f64 / (hours as f64 * 4.0)).min(0.95))
}

/// The tray's light: amber for any problem, then by phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Idle,
    Recording,
    Problem,
    Busy,
}

impl Snapshot {
    pub fn light(&self) -> Light {
        if self.problem.is_some() {
            Light::Problem
        } else {
            match self.phase {
                Phase::Recording => Light::Recording,
                Phase::Saving => Light::Busy,
                _ => Light::Idle,
            }
        }
    }

    /// The tray tooltip.
    pub fn tooltip(&self) -> String {
        let state = match (&self.problem, self.phase) {
            (Some(problem), _) => problem.text.as_str(),
            (None, Phase::Waiting) => "Waiting for the game",
            (None, Phase::NotRecording) => "Not recording",
            (None, Phase::Recording) => "Recording",
            (None, Phase::Saving) => "Saving recording…",
            (None, Phase::Saved) => "Saved",
            (None, Phase::Discarded) => "Session discarded",
        };
        format!("Battle Plan Recorder: {state}")
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
        // A status without the viewed country names no flag.
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
        assert_eq!((live.hours, live.spool_bytes), (12, 4096));
        assert_eq!(
            live.problem,
            Some(Problem {
                text: "2 hours couldn't be recorded".into(),
                detail: "queue full".into()
            })
        );
        let one = read_live(
            &json!({"campaign": CAMPAIGN, "hours": 3, "last_hour": 1, "dropped_frames": 1}),
        );
        assert_eq!(one.problem.unwrap().text, "1 hour couldn't be recorded");
    }

    #[test]
    fn preparing_or_unreadable_status_claims_nothing() {
        // Before the first recorded hour there is no date; a non-UUID campaign is not named.
        let live = read_live(
            &json!({"state": "preparing campaign history", "campaign": "save name", "hours": 0, "last_hour": 7}),
        );
        assert_eq!(live, Live::default());
        assert_eq!(read_live(&json!(null)), Live::default());
    }

    #[test]
    fn a_failed_capture_is_a_problem_even_without_lost_hours() {
        let live = read_live(
            &json!({"capture_failed": true, "error": "collector allocation failed", "hours": 5, "last_hour": 9}),
        );
        assert_eq!(
            live.problem,
            Some(Problem {
                text: "Recording stopped early. Recovering the captured part.".into(),
                detail: "collector allocation failed".into()
            })
        );
    }

    #[test]
    fn saving_progress_follows_work_and_never_claims_completion() {
        assert_eq!(saving_progress(10, 0), None);
        assert_eq!(saving_progress(0, 10), Some(0.0));
        assert_eq!(saving_progress(20, 10), Some(0.5));
        assert_eq!(saving_progress(1_000, 10), Some(0.95));
    }

    #[test]
    fn tray_light_and_tooltip_follow_phase_with_problems_first() {
        let mut snapshot = Snapshot::default();
        assert_eq!(snapshot.light(), Light::Idle);
        assert_eq!(snapshot.tooltip(), "Battle Plan Recorder: Waiting for the game");
        snapshot.phase = Phase::Recording;
        assert_eq!(snapshot.light(), Light::Recording);
        snapshot.phase = Phase::Saving;
        assert_eq!(snapshot.light(), Light::Busy);
        assert_eq!(snapshot.tooltip(), "Battle Plan Recorder: Saving recording…");
        snapshot.problem = Some(Problem {
            text: "2 hours couldn't be recorded".into(),
            detail: String::new(),
        });
        assert_eq!(snapshot.light(), Light::Problem);
        assert_eq!(
            snapshot.tooltip(),
            "Battle Plan Recorder: 2 hours couldn't be recorded"
        );
    }
}
