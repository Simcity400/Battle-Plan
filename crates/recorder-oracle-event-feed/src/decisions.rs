//! Retained decisions are compared at the save hour, independently of census age.
use installed_content_save::{SaveDocument, governance as saved};
use super::script_events::{Owner, uncertified};
use recorder_oracle_support::{
    report::Report,
    support::{country_items, eq, invalid},
};
use replay_data::event_feed_rows::ScriptEventState;
use replay_data::event_feed_rows::decisions::CONTAINERS;
use replay_data::event_feed_rows::decisions::DecisionState;
use replay_data::event_feed_rows::decisions::IGNORED;
use replay_data::rows::CountryRow;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
fn compare_owner(
    actual: &DecisionState,
    expected: Option<&saved::Decisions>,
    tag: &[u8],
    mut target_country: impl FnMut(u32) -> io::Result<Vec<u8>>,
    report: &mut Report,
) -> io::Result<()> {
    let label = format!(
        "{}.event_feed_decision_status",
        String::from_utf8_lossy(tag)
    );
    eq(
        report,
        &format!("{label}.decisions_taken"),
        actual
            .members(0)
            .map_err(invalid)?
            .iter()
            .map(|row| super::script_events::text(&row.token).to_vec())
            .collect::<Vec<_>>(),
        expected
            .map(|value| value.taken.clone())
            .unwrap_or_default(),
    );
    for container in 1..CONTAINERS as u8 {
        let rows = actual.members(container).map_err(invalid)?;
        let saved = expected
            .map(|value| value.containers[container as usize - 1].as_slice())
            .unwrap_or_default();
        eq(
            report,
            &format!("{label}.{container}.count"),
            rows.len(),
            saved.len(),
        );
        for (ordinal, (row, saved)) in rows.iter().zip(saved).enumerate() {
            let label = format!("{label}.{container}.{ordinal}");
            eq(
                report,
                &format!("{label}.token"),
                super::script_events::text(&row.token),
                saved.token.as_slice(),
            );
            eq(report, &format!("{label}.days"), row.days, saved.days);
            if let Some(lifecycle) = saved.lifecycle {
                eq(
                    report,
                    &format!("{label}.lifecycle"),
                    row.lifecycle,
                    lifecycle,
                );
            }
            if let Some(target) = &saved.target {
                let native = if row.country_target.unwrap_or(0) as i32 > 0 {
                    saved::Target::Country(target_country(row.country_target.unwrap())?)
                } else {
                    saved::Target::Numeric(row.numeric_target.unwrap_or(0) as u32)
                };
                eq(report, &format!("{label}.target"), &native, target);
                eq(
                    report,
                    &format!("{label}.ignore"),
                    row.flags & IGNORED != 0,
                    saved
                        .ignore
                        .ok_or_else(|| invalid("saved targeted ignore absent"))?,
                );
            }
        }
    }
    eq(
        report,
        &format!("{label}.random_items"),
        actual
            .random_counts
            .iter()
            .map(|row| {
                (
                    super::script_events::text(&row.token).to_vec(),
                    row.count,
                    row.target,
                )
            })
            .collect::<Vec<_>>(),
        expected
            .map(|value| {
                value
                    .random_items
                    .iter()
                    .map(|row| (row.token.clone(), row.count, row.target))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    report.compared("EventFeedHistory decision registry", 1);
    Ok(())
}
pub(super) fn compare(
    frame: &DecodedFrame,
    document: &SaveDocument,
    owners: &[Owner<'_>],
    links: &BTreeMap<u32, &CountryRow>,
    state: &ScriptEventState,
    report: &mut Report,
) -> io::Result<()> {
    let countries = country_items(document, frame)?;
    // Only an unnamed country fails the whole group; each owner's saved
    // status is read on its own, so one unreadable owner hides no other.
    let saved = countries
        .into_iter()
        .map(|item| {
            let tag = item
                .key
                .as_deref()
                .ok_or_else(|| invalid("decision country lacks tag"))?;
            let status = item
                .value
                .items()
                .ok_or_else(|| invalid("decision country body absent"))
                .and_then(|body| {
                    body.iter()
                        .find(|item| item.keyword_is("decision_status"))
                        .map(|item| {
                            item.value
                                .items()
                                .ok_or_else(|| invalid("decision status body absent"))
                                .and_then(saved::parse_decisions)
                        })
                        .transpose()
                });
            Ok((tag, status))
        })
        .collect::<io::Result<BTreeMap<_, _>>>()?;
    compare_owners(
        owners,
        &saved,
        state,
        |reference| {
            links
                .get(&reference)
                .map(|country| country.tag.clone())
                .ok_or_else(|| invalid("decision target country link absent"))
        },
        report,
    );
    Ok(())
}

/// Every certified owner is compared; uncertified owners are listed by reason
/// and an owner whose saved operands cannot be read is refused alone.
pub(super) fn compare_owners(
    owners: &[Owner<'_>],
    saved: &BTreeMap<&[u8], io::Result<Option<saved::Decisions>>>,
    state: &ScriptEventState,
    mut target_country: impl FnMut(u32) -> io::Result<Vec<u8>>,
    report: &mut Report,
) {
    let group = "EventFeedHistory decisions";
    let mut refused = BTreeMap::<String, Vec<String>>::new();
    for &(tag, owner) in owners {
        let name = String::from_utf8_lossy(tag).into_owned();
        let actual = match owner
            .ok_or("owner instance witness absent")
            .and_then(|owner| state.decisions(owner))
        {
            Ok(actual) => actual,
            Err(reason) => {
                refused.entry(reason.to_owned()).or_default().push(name);
                continue;
            }
        };
        let compared = saved
            .get(tag)
            .ok_or_else(|| invalid("saved decision owner absent"))
            .and_then(|expected| {
                let expected = expected.as_ref().map_err(|error| {
                    invalid(format!("saved decision_status unreadable: {error}"))
                })?;
                compare_owner(
                    actual,
                    expected.as_ref(),
                    tag,
                    &mut target_country,
                    report,
                )
            });
        if let Err(error) = compared {
            report.incomplete(group, format!("{name}: {error}"));
        }
    }
    uncertified(report, group, owners.len(), refused);
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::event_feed_rows::EventFeedEvent;
    use replay_data::event_feed_rows::EventFeedHistoryRow;
    use replay_data::event_feed_rows::EventFeedScript;
    use replay_data::event_feed_rows::EventFeedScriptChange;
    use replay_data::event_feed_rows::EventFeedSource;
    use replay_data::event_feed_rows::decisions::DecisionChange;
    use replay_data::event_feed_rows::decisions::Operation;
    use replay_data::rows::Recorded;
    #[test]
    fn save_comparison_catches_a_duration_lost_between_daily_censuses() {
        let mut actual = DecisionState::default();
        let row = EventFeedHistoryRow {
            read_hour: 1,
            script_tokens: b"synthetic".to_vec(),
            ..Default::default()
        };
        for (sequence, operation, instance, days) in [
            (1, Operation::Baseline, None, None),
            (2, Operation::Store, Some(1), Some(15)),
            (3, Operation::Ready, None, None),
        ] {
            let change =
                DecisionChange::new(operation, if operation == Operation::Store { 3 } else { 0 });
            let script = EventFeedScript {
                change: Some(EventFeedScriptChange::Decision(change)),
                instance,
                days,
                lifecycle: (instance.is_some()).then_some(0),
                token_len: if instance.is_some() { 9 } else { 0 },
                ..Default::default()
            };
            let event = EventFeedEvent {
                sequence,
                generation: 1,
                hour: 1,
                source: EventFeedSource::ScriptState,
                actor: Default::default(),
                target: Default::default(),
                cause: Recorded::Value(Default::default()),
                script: Some(script),
            };
            actual.observe(&row, &event, script, change).unwrap();
        }
        let mut expected = saved::Decisions {
            taken: Vec::new(),
            containers: std::array::from_fn(|_| Vec::new()),
            random_items: Vec::new(),
        };
        expected.containers[2].push(saved::Decision {
            token: b"synthetic".to_vec(),
            days: 15,
            lifecycle: Some(0),
            target: None,
            ignore: None,
        });
        let mut report = Report::default();
        compare_owner(
            &actual,
            Some(&expected),
            b"SYN",
            |_| Ok(Vec::new()),
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        expected.containers[2][0].days = 10;
        compare_owner(
            &actual,
            Some(&expected),
            b"SYN",
            |_| Ok(Vec::new()),
            &mut report,
        )
        .unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("days"))
        );
    }
}
