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
    /// The session ended and its recording waits to be saved (Auto save is off).
    Unsaved,
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
    /// Earlier hours of the campaign wait to be saved and could not be staged.
    Waiting,
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
    /// The recording this campaign saves into, or was saved to, once it exists.
    pub recording: Option<String>,
    /// Its file name; before its first save, the name its file will get.
    pub name: Option<String>,
    /// That recording already held an earlier recording when this session began.
    pub continued: bool,
    /// The current (or last saved) game hour.
    pub hour: Option<i32>,
    /// The first hour of a saved recording.
    pub first_hour: Option<i32>,
    /// The saved recording's size, never an estimate of a future save.
    pub saved_bytes: Option<u64>,
    /// Bytes of this session's spool on disk, not saved yet.
    pub spool_bytes: Option<u64>,
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
    /// The name of the mode the session attached to the game records, even at its menu: fixed
    /// until it detaches.
    pub capture_mode: Option<&'static str>,
    /// The collector's warning that this release does not recognise the game build.
    pub build_warning: Option<String>,
}

/// The fields of one collector status publication the panel uses.
#[derive(Debug, Default, PartialEq)]
pub struct Live {
    pub campaign: Option<String>,
    pub hour: Option<i32>,
    /// Hours processed since attachment, across campaigns; `None` when the status does not say.
    pub hours: Option<u64>,
    /// This session's spool bytes, across campaigns; `None` when the status does not say.
    pub spool_bytes: Option<u64>,
    pub problem: Option<Problem>,
    /// The flag file keys the game tries for the viewed country, in order.
    pub flag_keys: Option<Vec<String>>,
    pub build_warning: Option<String>,
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
        spool_bytes: status["spool_bytes"].as_u64(),
        problem,
        flag_keys: flag_keys(&status["player"]),
        build_warning: status["build"]["warning"].as_str().map(str::to_owned),
    }
}

/// Finalization reports work done, not a total. Each recorded hour costs about
/// four steps (spool read, frame read, encode check, block write), so the bar
/// advances with real work and stops short of full until saving ends.
pub fn saving_progress(records: u64, hours: u64) -> Option<f64> {
    (hours > 0).then(|| (records as f64 / (hours as f64 * 4.0)).min(0.95))
}

/// The tray icon's dot, its only status: yellow for any problem, then by phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// On and ready, not recording.
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
        assert_eq!((live.hours, live.spool_bytes), (Some(12), Some(4096)));
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
        assert_eq!(
            live,
            Live {
                hours: Some(0),
                ..Live::default()
            }
        );
        // An unreadable status, or one without its sizes, says nothing of them: never zero.
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
        snapshot.problem = Some(Problem {
            text: "2 hours couldn't be recorded".into(),
            detail: String::new(),
        });
        for phase in [Phase::Waiting, Phase::Recording, Phase::Saving] {
            snapshot.phase = phase;
            assert_eq!(snapshot.light(), Light::Problem, "{phase:?}");
        }
    }
}
