use content_reader_save::{SaveDocument, script_events as saved};
use recorder_oracle_support::{
    report::Report,
    support::{country_context, country_items, eq, invalid},
};
use replay_data::event_feed_rows::DelayedEvent;
use replay_data::event_feed_rows::EventFeedCountry;
use replay_data::event_feed_rows::EventFeedScope;
use replay_data::event_feed_rows::ScriptEventState;
use replay_data::rows::CountryRow;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub(super) fn text(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())]
}
fn name(identity: &replay_data::event_feed_rows::state::ScriptIdentity) -> Vec<u8> {
    if identity.token.is_empty() {
        identity.native_event.id.to_string().into_bytes()
    } else {
        text(&identity.token).to_vec()
    }
}
fn country(reference: u32, links: &BTreeMap<u32, &CountryRow>) -> io::Result<Option<Vec<u8>>> {
    if reference as i32 <= 0 {
        return Ok(None);
    }
    links
        .get(&reference)
        .map(|row| Some(row.tag.clone()))
        .ok_or_else(|| {
            invalid(format!(
                "event scope country {reference} has no recorded link"
            ))
        })
}
fn known(keys: &[Vec<u8>], subject: &str) -> io::Result<()> {
    if keys.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "{subject} has unrecorded saved fields: {keys:?}"
        )))
    }
}

fn scopes(
    actual: &[EventFeedScope],
    expected: Option<&saved::ScopeGraph>,
    links: &BTreeMap<u32, &CountryRow>,
    subject: &str,
    report: &mut Report,
) -> io::Result<()> {
    let expected = expected.ok_or_else(|| invalid(format!("{subject} saved scope absent")))?;
    if actual.is_empty() || expected.nodes.is_empty() {
        return Err(invalid(format!("{subject} scope graph empty")));
    }
    let mut pending = vec![(0_usize, 0_usize)];
    while let Some((a, b)) = pending.pop() {
        let a_node = actual
            .get(a)
            .ok_or_else(|| invalid("event scope binding outside recorded graph"))?;
        let b_node = expected
            .nodes
            .get(b)
            .ok_or_else(|| invalid("event scope binding outside saved graph"))?;
        let label = format!("{subject}.scope[{b}]");
        known(&b_node.extra_keys, &label)?;
        eq(
            report,
            &format!("{label}.country"),
            country(a_node.country, links)?,
            b_node.country.clone(),
        );
        eq(
            report,
            &format!("{label}.state"),
            (a_node.state != 0).then_some(a_node.state),
            b_node.state,
        );
        eq(
            report,
            &format!("{label}.random"),
            Some([a_node.random[1], a_node.random[0]]),
            b_node.random,
        );
        for (n, reference) in a_node.references.iter().copied().enumerate() {
            eq(
                report,
                &format!(
                    "{label}.{}",
                    String::from_utf8_lossy(saved::REFERENCE_KEYS[n])
                ),
                (reference != 0).then_some(reference),
                b_node.references[n],
            );
        }
        eq(
            report,
            &format!("{label}.strategic_region"),
            a_node.strategic_region.map(|id| id as i32),
            b_node.strategic_region,
        );
        for (n, child) in a_node.bindings.iter().copied().enumerate() {
            let name = ["root", "from", "prev"][n];
            let emitted = child as usize != a;
            eq(
                report,
                &format!("{label}.{name}.present"),
                emitted,
                b_node.bindings[n].is_some(),
            );
            if emitted && let Some(saved_child) = b_node.bindings[n] {
                pending.push((child as usize, saved_child));
            }
        }
        if a_node.bindings[1] as usize == a && a_node.target_storage_unrecorded {
            return Err(invalid(format!(
                "{label}.saved_event_target storage is unrecorded"
            )));
        }
        eq(
            report,
            &format!("{label}.saved_event_target.count"),
            0,
            b_node.saved_targets.len(),
        );
    }
    Ok(())
}

pub(super) type Owner<'a> = (&'a [u8], Option<EventFeedCountry>);

fn domain(
    report: &mut Report,
    group: &str,
    compare: impl FnOnce(&mut Report) -> io::Result<()>,
) {
    if let Err(error) = compare(report) {
        report.incomplete(group, error.to_string());
    }
}

pub(super) fn uncertified(
    report: &mut Report,
    group: &str,
    owners: usize,
    refused: BTreeMap<String, Vec<String>>,
) {
    for (reason, tags) in refused {
        report.incomplete(
            group,
            format!(
                "{} of {owners} owners: {reason}: {}",
                tags.len(),
                tags.join(", ")
            ),
        );
    }
}

pub fn compare(
    frame: &DecodedFrame,
    document: &SaveDocument,
    state: &ScriptEventState,
    windows: &super::windows::Witness,
    report: &mut Report,
) -> io::Result<()> {
    let saved = saved::parse(document.items(), &country_items(document, frame)?)?;
    let player = document
        .items()
        .iter()
        .find(|item| item.key.as_deref() == Some(&b"player"[..]))
        .and_then(|item| item.value.scalar());
    match saved.pending.as_ref() {
        Ok(Some(pending)) => super::windows::compare(windows, pending, player, report),
        Ok(None) => report.incomplete("EventFeedHistory window decisions", "pending_events block absent"),
        Err(reason) => report.incomplete("EventFeedHistory window decisions", format!("saved pending_events unreadable: {reason}")),
    }
    let context = country_context(frame)?;
    let identities = frame
        .complete::<tables::CountryExperienceGains>()?
        .iter()
        .map(|row| (row.country, row.instance))
        .collect::<BTreeMap<_, _>>();
    let owners = context
        .by_tag
        .iter()
        .map(|(tag, row)| {
            let owner = identities.get(&row.id).map(|&instance| EventFeedCountry {
                reference: row.id,
                id: row.id,
                instance,
            });
            (*tag, owner)
        })
        .collect::<Vec<Owner<'_>>>();
    compare_retained(&saved, &owners, &context.links, state, report);
    super::decisions::compare(frame, document, &owners, &context.links, state, report)
}

fn compare_retained(
    saved: &saved::Pack,
    owners: &[Owner<'_>],
    links: &BTreeMap<u32, &CountryRow>,
    state: &ScriptEventState,
    report: &mut Report,
) {
    domain(report, "EventFeedHistory pending_events", |report| {
        let saved_pending = saved
            .pending
            .as_ref()
            .map_err(|reason| invalid(format!("saved pending_events unreadable: {reason}")))?;
        known(&saved.pending_extra, "pending_events")?;
        let pending = state.pending().map_err(invalid)?;
        let saved_pending = saved_pending
            .as_ref()
            .ok_or_else(|| invalid("pending_events block absent"))?;
        eq(
            report,
            "pending_events.count",
            pending.len(),
            saved_pending.len(),
        );
        for (n, (actual, expected)) in pending.iter().zip(saved_pending).enumerate() {
            let label = format!("pending_events[{n}]");
            known(&expected.extra_keys, &label)?;
            eq(
                report,
                &format!("{label}.id"),
                Some(name(&actual.definition)),
                expected.identity.clone(),
            );
            eq(
                report,
                &format!("{label}.pending_id"),
                Some(actual.instance),
                expected.instance,
            );
            eq(
                report,
                &format!("{label}.timeout"),
                Some(actual.due_hour),
                expected.due.map(|date| date.engine_hour()).transpose()?,
            );
            scopes(&actual.scopes, expected.scope.as_ref(), links, &label, report)?;
        }
        report.limitation("pending event originators and occurrence history are not serialized; retained scope operands and membership are compared", "pending_events.originator");
        Ok(())
    });
    domain(report, "EventFeedHistory fired_event_names", |report| {
        let saved_fired = saved
            .fired
            .as_ref()
            .map_err(|reason| invalid(format!("saved fired_event_names unreadable: {reason}")))?;
        known(&saved.fired_extra, "fired_event_names")?;
        let mut fired = state
            .fired()
            .map_err(invalid)?
            .iter()
            .map(name)
            .collect::<Vec<_>>();
        let mut saved_fired = saved_fired
            .clone()
            .ok_or_else(|| invalid("fired_event_names block absent"))?;
        fired.sort();
        saved_fired.sort();
        eq(report, "fired_event_names.members", fired, saved_fired);
        Ok(())
    });
    eq(
        report,
        "delayed_events.country_count",
        owners.len(),
        saved.delayed.len(),
    );
    let group = "EventFeedHistory delayed_events";
    let mut refused = BTreeMap::<String, Vec<String>>::new();
    for &(tag, owner) in owners {
        let name = String::from_utf8_lossy(tag).into_owned();
        let actual = match owner
            .ok_or("owner instance witness absent")
            .and_then(|owner| state.delayed(owner))
        {
            Ok(actual) => actual,
            Err(reason) => {
                refused.entry(reason.to_owned()).or_default().push(name);
                continue;
            }
        };
        if let Err(error) = compare_delayed(
            actual,
            saved
                .delayed
                .get(tag)
                .map(|rows| rows.as_deref().map_err(String::as_str)),
            &name,
            links,
            report,
        ) {
            report.incomplete(group, format!("{name}: {error}"));
        }
    }
    uncertified(report, group, owners.len(), refused);
}

fn compare_delayed(
    actual: &[DelayedEvent],
    expected: Option<Result<&[saved::Delayed], &str>>,
    tag: &str,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = expected
        .ok_or_else(|| invalid("delayed-event saved country absent"))?
        .map_err(|reason| invalid(format!("saved delayed_event unreadable: {reason}")))?;
    let label = format!("{tag}.delayed_event");
    eq(
        report,
        &format!("{label}.count"),
        actual.len(),
        expected.len(),
    );
    for (n, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let label = format!("{label}[{n}]");
        known(&expected.extra_keys, &label)?;
        let definition = if actual.definition.token.is_empty() {
            let id = actual.definition.native_event;
            saved::Definition::Reference((u64::from(id.id) << 32) | u64::from(id.kind))
        } else {
            saved::Definition::Name(text(&actual.definition.token).to_vec())
        };
        eq(
            report,
            &format!("{label}.event"),
            Some(definition),
            expected.definition.clone(),
        );
        eq(
            report,
            &format!("{label}.duration"),
            (
                Some((actual.remaining % 720) % 24),
                Some((actual.remaining % 720) / 24),
                Some(actual.remaining / 720),
            ),
            (expected.hours, expected.days, expected.months),
        );
        eq(
            report,
            &format!("{label}.originator"),
            country(actual.originator.reference, links)?,
            expected.originator.clone(),
        );
        scopes(&actual.scopes, expected.scope.as_ref(), links, &label, report)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::event_feed_rows::{
        EventFeedCounts, EventFeedCoverage, EventFeedEvent, EventFeedEventIdentity,
        EventFeedHistoryRow, EventFeedScript, EventFeedScriptChange as Change, EventFeedSource,
        decisions::{DecisionChange, Operation},
    };
    use replay_data::rows::Recorded;

    fn owner(n: u32) -> EventFeedCountry {
        EventFeedCountry {
            reference: n,
            id: n,
            instance: 1,
        }
    }
    fn owners() -> Vec<Owner<'static>> {
        vec![
            (b"AAA".as_slice(), Some(owner(1))),
            (b"BBB".as_slice(), Some(owner(2))),
            (b"CCC".as_slice(), Some(owner(3))),
        ]
    }
    fn state() -> ScriptEventState {
        let mut changes = vec![
            (owner(0), Change::PendingBaseline, None),
            (owner(0), Change::PendingReady, None),
        ];
        for n in [2, 3] {
            changes.extend([
                (owner(n), Change::DelayedBaseline, None),
                (owner(n), Change::DelayedStored { remaining: 30 }, Some(n)),
                (owner(n), Change::DelayedReady, None),
            ]);
            for operation in [Operation::Baseline, Operation::Ready] {
                let change = Change::Decision(DecisionChange::new(operation, 0));
                changes.push((owner(n), change, None));
            }
        }
        let row = EventFeedHistoryRow {
            generation: 1,
            read_hour: 5,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts {
                acquired: 100,
                settled: 100,
                ..Default::default()
            }),
            script_tokens: b"synthetic.1".to_vec(),
            script_scopes: vec![EventFeedScope::default()],
            events: changes
                .into_iter()
                .enumerate()
                .map(|(n, (country, change, timer))| {
                    let decision = matches!(change, Change::Decision(_));
                    let delayed = !decision
                        && !matches!(change, Change::PendingBaseline | Change::PendingReady);
                    EventFeedEvent {
                        sequence: n as u64 + 1,
                        generation: 1,
                        hour: 5,
                        source: EventFeedSource::ScriptState,
                        actor: if decision { country } else { Default::default() },
                        target: if delayed { country } else { Default::default() },
                        cause: Recorded::Value(Default::default()),
                        script: Some(EventFeedScript {
                            token_len: if timer.is_some() { 11 } else { 0 },
                            scope_len: u32::from(timer.is_some()),
                            native_event: timer.map(|_| EventFeedEventIdentity { kind: 1, id: 90 }),
                            instance: timer,
                            change: Some(change),
                            ..Default::default()
                        }),
                    }
                })
                .collect(),
            ..Default::default()
        };
        let mut state = ScriptEventState::default();
        state.observe(&row).unwrap();
        state
    }
    fn timer() -> saved::Delayed {
        saved::Delayed {
            definition: Some(saved::Definition::Name(b"synthetic.1".to_vec())),
            hours: Some(6),
            days: Some(1),
            months: Some(0),
            originator: None,
            scope: Some(saved::ScopeGraph {
                nodes: vec![saved::Scope {
                    random: Some([0, 0]),
                    ..Default::default()
                }],
            }),
            extra_keys: Vec::new(),
        }
    }
    fn pack() -> saved::Pack {
        saved::Pack {
            pending: Ok(Some(Vec::new())),
            fired: Ok(Some(Vec::new())),
            delayed: [
                (b"AAA".to_vec(), Ok(Vec::new())),
                (b"BBB".to_vec(), Ok(vec![timer()])),
                (b"CCC".to_vec(), Ok(vec![timer()])),
            ]
            .into(),
            ..Default::default()
        }
    }

    fn queue<'a>(saved: &'a mut saved::Pack, tag: &[u8]) -> &'a mut Vec<saved::Delayed> {
        saved.delayed.get_mut(tag).unwrap().as_mut().unwrap()
    }

    #[test]
    fn unreadable_saved_operands_refuse_only_their_domain_or_owner() {
        let state = state();
        let mut saved = pack();
        saved.pending = Err("synthetic unreadable pending block".into());
        saved
            .delayed
            .insert(b"CCC".to_vec(), Err("synthetic unreadable queue".into()));
        queue(&mut saved, b"BBB")[0].days = Some(2);
        let mut report = Report::default();
        compare_retained(&saved, &owners(), &BTreeMap::new(), &state, &mut report);
        let decisions = BTreeMap::from([
            (b"AAA".as_slice(), Ok(None)),
            (
                b"BBB".as_slice(),
                Err(invalid("synthetic unreadable decision_status")),
            ),
            (
                b"CCC".as_slice(),
                Ok(Some(content_reader_save::governance::Decisions {
                    taken: vec![b"mod_decision".to_vec()],
                    containers: Default::default(),
                    random_items: Vec::new(),
                })),
            ),
        ]);
        crate::decisions::compare_owners(
            &owners(),
            &decisions,
            &state,
            |_| Ok(Vec::new()),
            &mut report,
        );
        let failed = &report.failed_comparisons;
        assert!(
            failed["EventFeedHistory pending_events"]
                .contains("saved pending_events unreadable: synthetic unreadable pending block"),
            "{failed:?}"
        );
        assert!(failed["EventFeedHistory fired_event_names"].contains("fired_event_names"));
        let delayed = &failed["EventFeedHistory delayed_events"];
        assert!(
            delayed.contains("1 of 3 owners: country delayed-event baseline was not observed: AAA"),
            "{delayed}"
        );
        assert!(
            delayed.contains("CCC: saved delayed_event unreadable: synthetic unreadable queue"),
            "{delayed}"
        );
        let decisions = &failed["EventFeedHistory decisions"];
        assert!(
            decisions.contains("1 of 3 owners: decision owner baseline/writer certificate unavailable: AAA"),
            "{decisions}"
        );
        assert!(
            decisions.contains("BBB: saved decision_status unreadable: synthetic unreadable decision_status"),
            "{decisions}"
        );
        assert_eq!(failed.len(), 4, "{failed:?}");
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.starts_with("BBB.delayed_event[0].duration")),
            "{:?}",
            report.differences
        );
        assert!(
            report.differences.iter().any(|difference| difference
                .starts_with("CCC.event_feed_decision_status.decisions_taken")),
            "{:?}",
            report.differences
        );
    }

    #[test]
    fn every_retained_domain_and_owner_reports_its_refusal_beside_certified_differences() {
        let state = state();
        let mut saved = pack();
        queue(&mut saved, b"BBB")[0].days = Some(2);
        queue(&mut saved, b"CCC")[0].extra_keys = vec![b"unknown_key".to_vec()];
        let mut report = Report::default();
        compare_retained(&saved, &owners(), &BTreeMap::new(), &state, &mut report);
        let decisions = BTreeMap::from([
            (b"AAA".as_slice(), Ok(None)),
            (b"BBB".as_slice(), Ok(None)),
            (b"CCC".as_slice(), Ok(None)),
        ]);
        crate::decisions::compare_owners(
            &owners(),
            &decisions,
            &state,
            |_| Ok(Vec::new()),
            &mut report,
        );
        let failed = &report.failed_comparisons;
        assert!(
            !failed.contains_key("EventFeedHistory pending_events"),
            "{failed:?}"
        );
        assert!(failed["EventFeedHistory fired_event_names"].contains("fired_event_names"));
        let delayed = &failed["EventFeedHistory delayed_events"];
        assert!(
            delayed.contains("1 of 3 owners: country delayed-event baseline was not observed: AAA"),
            "{delayed}"
        );
        assert!(
            delayed.contains("CCC: CCC.delayed_event[0] has unrecorded saved fields"),
            "{delayed}"
        );
        assert!(
            failed["EventFeedHistory decisions"]
                .contains("1 of 3 owners: decision owner baseline/writer certificate unavailable: AAA"),
            "{failed:?}"
        );
        assert_eq!(failed.len(), 3, "{failed:?}");
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("BBB.delayed_event[0].duration"));
        assert_eq!(report.compared_tables["EventFeedHistory decision registry"], 2);
    }

    #[test]
    fn an_incomplete_domain_without_differences_still_fails_the_comparison() {
        let state = state();
        let mut saved = pack();
        saved.delayed.remove(b"AAA".as_slice());
        let mut report = Report::default();
        compare_retained(&saved, &owners()[1..], &BTreeMap::new(), &state, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.failed_comparisons.keys().collect::<Vec<_>>(),
            ["EventFeedHistory fired_event_names"]
        );
        assert!(report.outcome().is_err());
    }
    #[test]
    fn retained_scope_comparison_checks_native_random_order_links_and_reference_words() {
        let graph = saved::ScopeGraph {
            nodes: vec![
                saved::Scope {
                    random: Some([22, 11]),
                    references: [
                        Some((8_u64 << 32) | 73),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                    bindings: [Some(1), None, None],
                    ..Default::default()
                },
                saved::Scope {
                    random: Some([44, 33]),
                    ..Default::default()
                },
            ],
        };
        let actual = [
            EventFeedScope {
                random: [11, 22],
                references: [(8_u64 << 32) | 73, 0, 0, 0, 0, 0, 0, 0, 0],
                bindings: [1, 0, 0],
                combat_context_unrecorded: true,
                ..Default::default()
            },
            EventFeedScope {
                random: [33, 44],
                bindings: [1, 1, 1],
                ..Default::default()
            },
        ];
        let mut report = Report::default();
        scopes(
            &actual,
            Some(&graph),
            &BTreeMap::new(),
            "synthetic",
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        let mut changed = actual;
        changed[1].random.swap(0, 1);
        scopes(
            &changed,
            Some(&graph),
            &BTreeMap::new(),
            "synthetic",
            &mut report,
        )
        .unwrap();
        assert!(!report.differences.is_empty());
        let mut unrecorded = actual;
        unrecorded[0].target_storage_unrecorded = true;
        assert!(
            scopes(
                &unrecorded,
                Some(&graph),
                &BTreeMap::new(),
                "synthetic",
                &mut report
            )
            .is_err()
        );
    }
}
