//! Retained observations are details of state at a read, never past fires.
use super::*;
use replay_data::event_feed_rows::{
    EventFeedScope, EventFeedScriptChange as Change,
    decisions::{CONTAINERS, Operation},
    state::{ScriptEventState, ScriptIdentity},
};
fn identity(value: &ScriptIdentity) -> String {
    format!(
        "{} [{}]",
        String::from_utf8_lossy(&value.token),
        value.native_event
    )
}
fn scopes(notes: &mut Vec<String>, label: &str, values: &[EventFeedScope]) {
    for (id, scope) in values.iter().enumerate() {
        notes.push(format!("{label} scope {id}: country reference {}; state {}; ROOT {}, FROM {}, PREV {}; random {:?}; references {:?}; strategic region {:?}; missing combat {}, saved targets {}, tooltip targets {}.", scope.country,scope.state,scope.bindings[0],scope.bindings[1],scope.bindings[2],scope.random,scope.references,scope.strategic_region,scope.combat_context_unrecorded,scope.target_storage_unrecorded,scope.tooltip_storage_unrecorded));
    }
}
pub(super) fn append(
    builder: &mut Builder,
    pack: &EventFeedHistoryRow,
    state: &ScriptEventState,
    tag_at: &mut impl FnMut(i32, EventFeedCountry) -> Option<String>,
) -> io::Result<()> {
    let changed = pack.events.iter().any(|event|event.script.and_then(|s|s.change).is_some_and(|change| !matches!(change,Change::Decision(change) if change.operation == Operation::Rebuild)));
    if !changed || pack.read_hour < builder.feed.first || pack.read_hour > builder.feed.last {
        return Ok(());
    }
    let mut notes = vec![format!(
        "Retained state observed at read hour {}. Baseline membership has no recorded fire or selection time.",
        pack.read_hour
    )];
    let mut tags = std::collections::BTreeSet::new();
    // Reached deltas remain inspectable even when a loss prevents current-state carry.
    for event in &pack.events {
        let Some(script) = event.script else {
            continue;
        };
        let Some(change) = script.change else {
            continue;
        };
        if matches!(change, Change::Decision(_)) {
            continue;
        }
        for country in [event.actor, event.target] {
            if country.reference as i32 > 0
                && let Some(tag) = tag_at(event.hour, country)
            {
                tags.insert(tag);
            }
        }
        let token = &pack.script_tokens
            [script.token_start as usize..script.token_start as usize + script.token_len as usize];
        notes.push(format!("Observation {} at {}: {:?}; content {}; identity {:?}; instance {:?}; originator {:?}; owner {:?}.",event.sequence,event.hour,change,String::from_utf8_lossy(token),script.native_event,script.instance,event.actor,event.target));
        scopes(
            &mut notes,
            &format!("observation {}", event.sequence),
            &pack.script_scopes[script.scope_start as usize
                ..script.scope_start as usize + script.scope_len as usize],
        );
    }
    match state.pending() {
        Ok(rows) => {
            notes.push(format!("Pending event instances: {}.", rows.len()));
            for row in rows {
                let label = format!(
                    "Pending instance {}: {}; due hour {}; originator {:?}",
                    row.instance,
                    identity(&row.definition),
                    row.due_hour,
                    row.originator
                );
                notes.push(label.clone());
                scopes(&mut notes, &label, &row.scopes);
            }
        }
        Err(reason) => notes.push(format!("Pending event instances unavailable: {reason}.")),
    }
    match state.fired() {
        Ok(rows) => {
            notes.push(format!(
                "fired_event_names membership: {} (duplicates retained).",
                rows.len()
            ));
            notes.extend(rows.iter().map(|row| {
                format!(
                    "Fired-set member: {}. Fire time is not inferred from membership.",
                    identity(row)
                )
            }));
        }
        Err(reason) => notes.push(format!("fired_event_names unavailable: {reason}.")),
    }
    for (owner, rows) in state.delayed_owners() {
        let name = tag_at(pack.read_hour, owner)
            .unwrap_or_else(|| format!("country {} instance {}", owner.reference, owner.instance));
        if let Some(tag) = tag_at(pack.read_hour, owner) {
            tags.insert(tag);
        }
        match rows {
            Ok(rows) => {
                notes.push(format!("{name}: {} delayed event timers.", rows.len()));
                for row in rows {
                    let label = format!(
                        "{name} delayed instance {}: {}; remaining {} hours; originator {:?}",
                        row.instance,
                        identity(&row.definition),
                        row.remaining,
                        row.originator
                    );
                    notes.push(label.clone());
                    scopes(&mut notes, &label, &row.scopes);
                }
            }
            Err(reason) => notes.push(format!("{name} delayed timers unavailable: {reason}.")),
        }
    }
    let containers = [
        "taken",
        "re-enable",
        "removal",
        "timed",
        "targeted",
        "timed targeted",
    ];
    for owner in state.decision_owners() {
        let name = tag_at(pack.read_hour, owner.country).unwrap_or_else(|| {
            format!(
                "country {} instance {}",
                owner.country.reference, owner.country.instance
            )
        });
        if let Some(tag) = tag_at(pack.read_hour, owner.country) {
            tags.insert(tag);
        }
        if !owner.complete() {
            notes.push(format!(
                "{name} decisions unavailable: owner baseline/writer/pass certificate incomplete."
            ));
            continue;
        }
        for container in 0..CONTAINERS as u8 {
            for member in owner.members(container).map_err(io::Error::other)? {
                notes.push(format!("{name} {}: {} instance {}; duration {} days; lifecycle {}; flags {}; country target {:?}; numeric target {:?}.",containers[container as usize],String::from_utf8_lossy(&member.token),member.instance,member.days,member.lifecycle,member.flags,member.country_target,member.numeric_target));
            }
        }
        for count in &owner.random_counts {
            notes.push(format!(
                "{name} description count: {}; target {}; count {}.",
                String::from_utf8_lossy(&count.token),
                count.target,
                count.count
            ));
        }
        for token in &owner.ignored {
            notes.push(format!(
                "{name} ignored decision: {}.",
                String::from_utf8_lossy(token)
            ));
        }
        for (kind, token, target, root) in &owner.candidates {
            notes.push(format!(
                "{name} candidate kind {kind}: {}; target {target}; ROOT reference {root}.",
                String::from_utf8_lossy(token)
            ));
        }
    }
    let kind = EventFeedKind::ScriptStateObserved;
    builder.feed.rows.push(EventFeedRow {
        id: format!("script-state:{}:{}", pack.generation, pack.read_hour),
        hour: pack.read_hour,
        read_hour: pack.read_hour,
        name_hour: pack.read_hour,
        exact: false,
        category: kind.category(),
        kind,
        tags: tags.into_iter().collect(),
        participants: None,
        item: None,
        amount: None,
        cause: Some(EventFeedCause {
            how: None,
            within: None,
            conference: None,
            state: None,
            ally: None,
            reason: None,
            notes,
        }),
        strike: None,
    });
    Ok(())
}
