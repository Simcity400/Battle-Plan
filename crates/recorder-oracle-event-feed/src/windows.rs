use content_reader_save::script_events as saved;
use recorder_oracle_support::report::Report;
use replay_data::event_feed_rows::{EventFeedHistoryRow, EventFeedSource, WindowVerdict};
use replay_data::rows::Recorded;
use std::collections::{BTreeMap, BTreeSet};

const GROUP: &str = "EventFeedHistory window decisions";

#[derive(Default)]
pub struct Witness {
    unrecorded: Option<String>,
    decisions: BTreeMap<u32, Result<Option<WindowVerdict>, String>>,
    presented: BTreeSet<u32>,
}

impl Witness {
    pub fn observe(&mut self, row: &EventFeedHistoryRow) {
        let windows = match &row.windows {
            Recorded::Value(windows) => windows,
            Recorded::NotRecorded => {
                self.unrecorded
                    .get_or_insert_with(|| format!("hour {} predates event window capture", row.read_hour));
                return;
            }
        };
        let verdicts = windows.verdicts.as_ref().map(|verdicts| {
            verdicts
                .iter()
                .map(|entry| (entry.sequence, entry.verdict))
                .collect::<BTreeMap<_, _>>()
        });
        for event in &row.events {
            let Some(instance) = event
                .script
                .filter(|_| matches!(event.source, EventFeedSource::ScriptEvent | EventFeedSource::NewsEvent))
                .and_then(|script| script.instance)
            else {
                continue;
            };
            let decision = match &verdicts {
                Ok(verdicts) => Ok(verdicts.get(&event.sequence).copied()),
                Err(reason) => Err(String::from_utf8_lossy(reason).into_owned()),
            };
            self.decisions.insert(instance, decision);
        }
        if let Ok(presentations) = &windows.presentations {
            self.presented
                .extend(presentations.iter().map(|presentation| presentation.instance));
        }
    }
}

fn player_class(verdict: WindowVerdict) -> bool {
    !matches!(verdict, WindowVerdict::AiControlled | WindowVerdict::NotViewingCountry)
}

fn other_class(verdict: WindowVerdict) -> bool {
    matches!(
        verdict,
        WindowVerdict::AiControlled | WindowVerdict::HiddenOrAiControlled | WindowVerdict::NotViewingCountry
    )
}

pub(super) fn compare(witness: &Witness, pending: &[saved::Pending], player: Option<&[u8]>, report: &mut Report) {
    if let Some(reason) = &witness.unrecorded {
        report.incomplete(GROUP, reason.clone());
        return;
    }
    let Some(player) = player else {
        report.incomplete(GROUP, "the save names no player");
        return;
    };
    for (n, entry) in pending.iter().enumerate() {
        let label = format!("pending_events[{n}].window");
        let (Some(instance), Some(recipient)) = (
            entry.instance,
            entry
                .scope
                .as_ref()
                .and_then(|scope| scope.nodes.first())
                .and_then(|root| root.country.as_deref()),
        ) else {
            report.incomplete(GROUP, format!("{label}: the saved pending event names no instance or recipient"));
            continue;
        };
        let is_player = recipient == player;
        match witness.decisions.get(&instance) {
            None if is_player => {
                if !witness.presented.contains(&instance) {
                    report.difference(&label, "no firing and no window in this branch", "a window reopened after load");
                }
            }
            None => report.limitation(
                "a pending event fired before this branch began holds no firing in it; only the player's reopened windows are witnessed",
                &label,
            ),
            Some(Err(reason)) => report.incomplete(GROUP, format!("{label}: {reason}")),
            Some(Ok(None)) => report.difference(&label, "not recorded", "the firing's window decision"),
            Some(Ok(Some(verdict))) => {
                let agrees = if is_player { player_class(*verdict) } else { other_class(*verdict) };
                if !agrees {
                    report.difference(
                        &label,
                        format!("{verdict:?}"),
                        if is_player {
                            "a decision that keeps the player's window possible"
                        } else {
                            "a decision for a country this client does not play"
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::event_feed_rows::{
        EventFeedCountry, EventFeedEvent, EventFeedScript, EventWindows, VerdictEntry,
    };

    fn row(windows: Recorded<EventWindows>, instance: u32) -> EventFeedHistoryRow {
        EventFeedHistoryRow {
            read_hour: 10,
            events: vec![EventFeedEvent {
                sequence: 4,
                generation: 1,
                hour: 10,
                source: EventFeedSource::ScriptEvent,
                actor: EventFeedCountry::default(),
                target: EventFeedCountry::default(),
                cause: Recorded::NotRecorded,
                script: Some(EventFeedScript {
                    instance: Some(instance),
                    ..Default::default()
                }),
            }],
            windows,
            ..Default::default()
        }
    }

    fn pending(instance: u32, recipient: &[u8]) -> saved::Pending {
        saved::Pending {
            identity: None,
            instance: Some(instance),
            due: None,
            scope: Some(saved::ScopeGraph {
                nodes: vec![saved::Scope {
                    country: Some(recipient.to_vec()),
                    ..Default::default()
                }],
            }),
            extra_keys: Vec::new(),
        }
    }

    fn verdict(verdict: WindowVerdict) -> Recorded<EventWindows> {
        Recorded::Value(EventWindows {
            verdicts: Ok(vec![VerdictEntry { sequence: 4, verdict }]),
            presentations: Ok(Vec::new()),
        })
    }

    fn judged(windows: Recorded<EventWindows>, recipient: &[u8]) -> bool {
        let mut witness = Witness::default();
        witness.observe(&row(windows, 9));
        let mut report = Report::default();
        compare(&witness, &[pending(9, recipient)], Some(b"GER"), &mut report);
        report.outcome().is_ok()
    }

    #[test]
    fn every_pending_event_needs_its_recorded_window_decision_for_its_recipient() {
        assert!(!judged(Recorded::NotRecorded, b"GER"), "a recording without window capture cannot certify a pending event's window");
        assert!(judged(verdict(WindowVerdict::Requested), b"GER"));
        assert!(!judged(verdict(WindowVerdict::AiControlled), b"GER"), "the player's pending event was never AI-controlled");
        assert!(judged(verdict(WindowVerdict::AiControlled), b"ITA"));
        assert!(!judged(verdict(WindowVerdict::Requested), b"ITA"), "this client requests no window for a country it does not play");
        let missing = Recorded::Value(EventWindows {
            verdicts: Ok(Vec::new()),
            presentations: Ok(Vec::new()),
        });
        assert!(!judged(missing, b"GER"), "an armed hour whose firing holds no decision lost it");
    }
}
