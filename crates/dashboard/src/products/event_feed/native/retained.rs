//! Retained observations are details of state at a read, never past fires. An observation
//! lists only the parts its read changed; a countdown that only ran down is not a change.
use super::*;
use replay_data::event_feed_rows::{
    EventFeedScope, EventFeedScriptChange as Change,
    decisions::{CONTAINERS, DecisionState},
    state::{DelayedEvent, ScriptEventState, ScriptIdentity},
};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

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

/// A part of the retained state an observation lists, in its order: the pending instances,
/// the fired set, then each delayed-event owner and each decision owner, by the reducer's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum Slot {
    Pending,
    Fired,
    Delayed(u32, u32),
    Decisions(u32, u32),
}

/// A part as last seen: a digest of all it holds but its countdowns, and its countdowns.
#[derive(Serialize, Deserialize)]
pub(super) struct Seen {
    digest: [u8; 16],
    countdowns: Vec<i32>,
}

/// Each part as last seen.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Sections(
    #[serde(with = "crate::prepared_production::map_pairs")] BTreeMap<Slot, Seen>,
);

impl Sections {
    /// Records `slot` as it stands: what `feed` writes of all it holds but its countdowns, and
    /// its `countdowns`. Whether it changed since it was last seen: what it holds differs, or a
    /// countdown rose, appeared or went. A part first seen while unavailable (`available`
    /// false) is recorded without being a change.
    fn see(
        &mut self,
        slot: Slot,
        available: bool,
        feed: impl FnOnce(&mut blake3::Hasher) -> serde_json::Result<()>,
        countdowns: impl Iterator<Item = i32>,
    ) -> io::Result<bool> {
        let mut hasher = blake3::Hasher::new();
        feed(&mut hasher)?;
        let mut digest = [0; 16];
        digest.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        let seen = match self.0.entry(slot) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Seen {
                    digest,
                    countdowns: countdowns.collect(),
                });
                return Ok(available);
            }
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
        };
        let mut changed = seen.digest != digest;
        seen.digest = digest;
        let mut at = 0;
        for now in countdowns {
            match seen.countdowns.get_mut(at) {
                Some(was) => {
                    changed |= now > *was;
                    *was = now;
                }
                None => {
                    changed = true;
                    seen.countdowns.push(now);
                }
            }
            at += 1;
        }
        if at != seen.countdowns.len() {
            changed = true;
            seen.countdowns.truncate(at);
        }
        Ok(changed)
    }
}

/// An owner's tag at the read, or what names it without one.
fn named(
    country: EventFeedCountry,
    tag_at: &mut impl FnMut(EventFeedCountry) -> Option<String>,
) -> Result<String, String> {
    tag_at(country).ok_or_else(|| {
        format!(
            "country {} instance {}",
            country.reference, country.instance
        )
    })
}

/// `slot` seen as it stands: its notes added to `parts`, and its owner's tag to `tags`, when
/// it changed since it was last seen.
fn part(
    sections: &mut Sections,
    state: &ScriptEventState,
    slot: Slot,
    tag_at: &mut impl FnMut(EventFeedCountry) -> Option<String>,
    parts: &mut Vec<String>,
    tags: &mut BTreeSet<String>,
) -> io::Result<()> {
    match slot {
        Slot::Pending => {
            let pending = state.pending();
            let feed = |hasher: &mut blake3::Hasher| serde_json::to_writer(hasher, &pending);
            if sections.see(slot, pending.is_ok(), feed, std::iter::empty())? {
                pending_notes(state, parts);
            }
        }
        Slot::Fired => {
            let fired = state.fired();
            let feed = |hasher: &mut blake3::Hasher| serde_json::to_writer(hasher, &fired);
            if sections.see(slot, fired.is_ok(), feed, std::iter::empty())? {
                fired_notes(state, parts);
            }
        }
        Slot::Delayed(id, instance) => {
            let Some((country, rows)) = state.delayed_owner((id, instance)) else {
                return Ok(());
            };
            let feed = |hasher: &mut blake3::Hasher| {
                serde_json::to_writer(&mut *hasher, &country)?;
                match rows {
                    Ok(rows) => {
                        serde_json::to_writer(&mut *hasher, &rows.len())?;
                        for timer in rows {
                            let held = (
                                timer.instance,
                                &timer.definition,
                                timer.originator,
                                &timer.scopes,
                            );
                            serde_json::to_writer(&mut *hasher, &held)?;
                        }
                        Ok(())
                    }
                    Err(reason) => serde_json::to_writer(hasher, reason),
                }
            };
            let countdowns = rows.unwrap_or_default().iter().map(|timer| timer.remaining);
            if sections.see(slot, rows.is_ok(), feed, countdowns)? {
                let name = named(country, tag_at);
                delayed_notes(name.as_deref().unwrap_or_else(|name| name), rows, parts);
                tags.extend(name.ok());
            }
        }
        Slot::Decisions(id, instance) => {
            let Some(owner) = state.decision_owner((id, instance)) else {
                return Ok(());
            };
            let complete = owner.complete();
            let members = || {
                (0..CONTAINERS as u8).flat_map(|container| {
                    owner
                        .members(container)
                        .unwrap_or_default()
                        .iter()
                        .map(move |member| (container, member))
                })
            };
            let feed = |hasher: &mut blake3::Hasher| {
                serde_json::to_writer(&mut *hasher, &(owner.country, complete))?;
                if complete {
                    for (container, member) in members() {
                        let held = (
                            container,
                            member.instance,
                            &member.token,
                            member.lifecycle,
                            member.flags,
                            member.country_target,
                            member.numeric_target,
                        );
                        serde_json::to_writer(&mut *hasher, &held)?;
                    }
                    let rest = (&owner.random_counts, &owner.ignored, &owner.candidates);
                    serde_json::to_writer(&mut *hasher, &rest)?;
                }
                Ok(())
            };
            let days = members()
                .filter(|_| complete)
                .map(|(_, member)| member.days);
            if sections.see(slot, complete, feed, days)? {
                let name = named(owner.country, tag_at);
                decision_notes(name.as_deref().unwrap_or_else(|name| name), owner, parts)?;
                tags.extend(name.ok());
            }
        }
    }
    Ok(())
}

fn pending_notes(state: &ScriptEventState, notes: &mut Vec<String>) {
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
                scopes(notes, &label, &row.scopes);
            }
        }
        Err(reason) => notes.push(format!("Pending event instances unavailable: {reason}.")),
    }
}

fn fired_notes(state: &ScriptEventState, notes: &mut Vec<String>) {
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
}

fn delayed_notes(name: &str, rows: Result<&[DelayedEvent], &str>, notes: &mut Vec<String>) {
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
                scopes(notes, &label, &row.scopes);
            }
        }
        Err(reason) => notes.push(format!("{name} delayed timers unavailable: {reason}.")),
    }
}

fn decision_notes(name: &str, owner: &DecisionState, notes: &mut Vec<String>) -> io::Result<()> {
    let containers = [
        "taken",
        "re-enable",
        "removal",
        "timed",
        "targeted",
        "timed targeted",
    ];
    if !owner.complete() {
        notes.push(format!(
            "{name} decisions unavailable: owner baseline/writer/pass certificate incomplete."
        ));
        return Ok(());
    }
    let members: usize = (0..CONTAINERS as u8)
        .map(|container| owner.members(container).map_or(0, <[_]>::len))
        .sum();
    notes.push(format!(
        "{name} decisions: {members} retained members, {} description counts, {} ignored, {} candidates.",
        owner.random_counts.len(),
        owner.ignored.len(),
        owner.candidates.len()
    ));
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
    Ok(())
}

/// The read's retained state observation: its own notes, then each part it changed as it
/// stands now. A read that changed no part, and whose only notes would be countdown passes,
/// is not observed.
pub(super) fn append(
    builder: &mut Builder,
    pack: &EventFeedHistoryRow,
    state: &ScriptEventState,
    sections: &mut Sections,
    tag_at: &mut impl FnMut(i32, EventFeedCountry) -> Option<String>,
) -> io::Result<()> {
    if pack.read_hour < builder.feed.first || pack.read_hour > builder.feed.last {
        return Ok(());
    }
    let mut notes = Vec::new();
    let mut tags = BTreeSet::new();
    for event in &pack.events {
        let Some(script) = event.script else {
            continue;
        };
        let Some(change) = script.change else {
            continue;
        };
        if matches!(change, Change::Decision(_) | Change::DelayedAdvance { .. }) {
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
    let touched = state.touched();
    let all = touched.all || sections.0.is_empty();
    let mut slots = Vec::new();
    if all || touched.pending {
        slots.push(Slot::Pending);
    }
    if all || touched.fired {
        slots.push(Slot::Fired);
    }
    if all {
        slots.extend(
            state
                .delayed_keys()
                .map(|(id, instance)| Slot::Delayed(id, instance)),
        );
        slots.extend(
            state
                .decision_keys()
                .map(|(id, instance)| Slot::Decisions(id, instance)),
        );
        // A part the state no longer holds is seen anew if it returns.
        sections
            .0
            .retain(|slot, _| slots.binary_search(slot).is_ok());
    } else {
        slots.extend(
            touched
                .delayed
                .iter()
                .map(|&(id, instance)| Slot::Delayed(id, instance)),
        );
        slots.extend(
            touched
                .decisions
                .iter()
                .map(|&(id, instance)| Slot::Decisions(id, instance)),
        );
    }
    let mut parts = Vec::new();
    let mut name = |country| tag_at(pack.read_hour, country);
    for slot in slots {
        part(sections, state, slot, &mut name, &mut parts, &mut tags)?;
    }
    if notes.is_empty() && parts.is_empty() {
        return Ok(());
    }
    let mut cause = vec![format!(
        "Retained state changed at read hour {}: each part listed is as it stands at this read; a countdown that only ran down is not a change. Baseline membership has no recorded fire or selection time.",
        pack.read_hour
    )];
    cause.extend(notes);
    cause.extend(parts);
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
            notes: cause,
        }),
        strike: None,
    });
    Ok(())
}
