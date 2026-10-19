//! Save-certified retained state, separate from unsaved occurrence history.
use installed_content_save::{SaveDocument, script_events as saved};
use recorder_oracle_support::{
    report::Report,
    support::{country_context, country_items, eq, invalid},
};
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
    // The save is a tree even where native links share a node. Its finite
    // node count bounds this iterative comparison without a selected depth cap.
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
        // Target storage is independently refused; it does not erase known
        // membership, duration or other scope operands. Combat contexts are unsaved.
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

pub fn compare(
    frame: &DecodedFrame,
    document: &SaveDocument,
    state: &ScriptEventState,
    report: &mut Report,
) -> io::Result<()> {
    let saved = saved::parse(document.items(), &country_items(document, frame)?)?;
    let context = country_context(frame)?;
    known(&saved.pending_extra, "pending_events")?;
    known(&saved.fired_extra, "fired_event_names")?;
    let pending = state.pending().map_err(invalid)?;
    let saved_pending = saved
        .pending
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
        scopes(
            &actual.scopes,
            expected.scope.as_ref(),
            &context.links,
            &label,
            report,
        )?;
    }
    report.limitation("pending event originators and occurrence history are not serialized; retained scope operands and membership are compared", "pending_events.originator");
    let mut fired = state
        .fired()
        .map_err(invalid)?
        .iter()
        .map(name)
        .collect::<Vec<_>>();
    let mut saved_fired = saved
        .fired
        .ok_or_else(|| invalid("fired_event_names block absent"))?;
    fired.sort();
    saved_fired.sort();
    eq(report, "fired_event_names.members", fired, saved_fired);
    let identities = frame
        .complete::<tables::CountryExperienceGains>()?
        .iter()
        .map(|row| (row.country, row.instance))
        .collect::<BTreeMap<_, _>>();
    eq(
        report,
        "delayed_events.country_count",
        context.by_tag.len(),
        saved.delayed.len(),
    );
    for (tag, country_row) in context.by_tag {
        let instance = *identities
            .get(&country_row.id)
            .ok_or_else(|| invalid("delayed-event owner instance witness absent"))?;
        let owner = EventFeedCountry {
            reference: country_row.id,
            id: country_row.id,
            instance,
        };
        let actual = state.delayed(owner).map_err(|reason| {
            invalid(format!(
                "{}.delayed_event: {reason}",
                String::from_utf8_lossy(tag)
            ))
        })?;
        let expected = saved
            .delayed
            .get(tag)
            .ok_or_else(|| invalid("delayed-event saved country absent"))?;
        let label = format!("{}.delayed_event", String::from_utf8_lossy(tag));
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
                country(actual.originator.reference, &context.links)?,
                expected.originator.clone(),
            );
            scopes(
                &actual.scopes,
                expected.scope.as_ref(),
                &context.links,
                &label,
                report,
            )?;
        }
    }
    super::decisions::compare(frame, document, state, report)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
