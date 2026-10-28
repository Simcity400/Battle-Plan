//! Retained observations are details of state at a read, never past fires.
use super::*;
use replay_data::event_feed_rows::{
    EventFeedScope, EventFeedScriptChange as Change,
    decisions::{CONTAINERS, DecisionState, Operation, RetainedDecision, RetainedRandomCount},
    state::{DelayedEvent, PendingEvent, ScriptEventState, ScriptIdentity},
};
use std::collections::{BTreeMap, BTreeSet};
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

/// A part of the retained state an observation lists, in its order: the
/// pending instances, the fired set, then each delayed-event owner and each
/// decision owner, by identity and its occurrence among equal identities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum Slot {
    Pending,
    Fired,
    Delayed(Owner, u32),
    Decisions(Owner, u32),
}

/// An owner's country reference, ID and instance.
type Owner = (u32, u32, u32);

fn owner(country: EventFeedCountry) -> Owner {
    (country.reference, country.id, country.instance)
}

/// Whether `held` rows are the current ones, a refusal included.
fn same_rows<T: PartialEq>(
    held: &Result<Vec<T>, &'static str>,
    current: Result<&[T], &str>,
) -> bool {
    match (held, current) {
        (Ok(held), Ok(current)) => held.as_slice() == current,
        (Err(held), Err(current)) => *held == current,
        _ => false,
    }
}

/// What a slot's notes are made of.
enum Inputs {
    Pending(Result<Vec<PendingEvent>, &'static str>),
    Fired(Result<Vec<ScriptIdentity>, &'static str>),
    Delayed(String, Result<Vec<DelayedEvent>, &'static str>),
    Decisions {
        name: String,
        complete: bool,
        members: Vec<Vec<RetainedDecision>>,
        random: Vec<RetainedRandomCount>,
        ignored: BTreeSet<Vec<u8>>,
        candidates: BTreeSet<(u8, Vec<u8>, u32, u32)>,
    },
}

impl Inputs {
    fn decisions(name: &str, owner: &DecisionState) -> Self {
        let complete = owner.complete();
        Self::Decisions {
            name: name.to_owned(),
            complete,
            members: (0..CONTAINERS as u8)
                .map(|container| {
                    owner
                        .members(container)
                        .map_or_else(|_| Vec::new(), <[_]>::to_vec)
                })
                .collect(),
            random: owner.random_counts.clone(),
            ignored: owner.ignored.clone(),
            candidates: owner.candidates.clone(),
        }
    }
    fn same_decisions(&self, name: &str, owner: &DecisionState) -> bool {
        let Self::Decisions {
            name: held,
            complete,
            members,
            random,
            ignored,
            candidates,
        } = self
        else {
            return false;
        };
        held == name
            && *complete == owner.complete()
            && (!complete
                || members.iter().enumerate().all(|(container, held)| {
                    owner
                        .members(container as u8)
                        .is_ok_and(|current| current == held.as_slice())
                }) && *random == owner.random_counts
                    && *ignored == owner.ignored
                    && *candidates == owner.candidates)
    }
}

/// A slot's notes as last made, what they were made of, and the ordinal of
/// the section event they were published as. A resumed slot holds only the
/// digest and ordinal, so its notes are made again and published again only
/// if they differ.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Section {
    #[serde(skip)]
    inputs: Option<Inputs>,
    #[serde(skip)]
    notes: Vec<String>,
    digest: [u8; 16],
    ordinal: Option<u64>,
}

/// Each slot's section as last observed.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Sections(
    #[serde(with = "crate::prepared_production::map_pairs")] BTreeMap<Slot, Section>,
);

impl Sections {
    /// `slot`'s section, its notes made again by `make` unless `same` proves
    /// their inputs unchanged.
    fn visit(
        &mut self,
        slot: &Slot,
        same: impl FnOnce(&Inputs) -> bool,
        inputs: impl FnOnce() -> Inputs,
        make: impl FnOnce(&mut Vec<String>) -> io::Result<()>,
    ) -> io::Result<()> {
        let section = self.0.entry(slot.clone()).or_default();
        if section.inputs.as_ref().is_some_and(same) {
            return Ok(());
        }
        let mut notes = Vec::new();
        make(&mut notes)?;
        let mut hasher = blake3::Hasher::new();
        for note in &notes {
            hasher.update(&(note.len() as u64).to_le_bytes());
            hasher.update(note.as_bytes());
        }
        let mut digest = [0; 16];
        digest.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        if section.ordinal.is_none() || digest != section.digest {
            section.digest = digest;
            section.ordinal = None;
        }
        section.notes = notes;
        section.inputs = Some(inputs());
        Ok(())
    }

    /// The ordinal `slot`'s section is published under, publishing it with
    /// `publish` when its notes have not been.
    pub(super) fn ordinal(
        &mut self,
        slot: &Slot,
        publish: &mut impl FnMut(&[String]) -> io::Result<u64>,
    ) -> io::Result<u64> {
        let section = self
            .0
            .get_mut(slot)
            .ok_or_else(|| io::Error::other("Retained state section was not observed"))?;
        if let Some(ordinal) = section.ordinal {
            return Ok(ordinal);
        }
        let ordinal = publish(&section.notes)?;
        section.ordinal = Some(ordinal);
        Ok(ordinal)
    }
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

/// The read's retained state observation. Each part of the state is made
/// into notes again only when what it reads changed. With `deferred`, the
/// row holds only the read's own notes and `deferred` its sections' slots,
/// whose notes follow them in order; otherwise the row holds every note.
pub(super) fn append(
    builder: &mut Builder,
    pack: &EventFeedHistoryRow,
    state: &ScriptEventState,
    sections: &mut Sections,
    deferred: Option<&mut BTreeMap<String, Vec<Slot>>>,
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
    let mut slots = vec![Slot::Pending, Slot::Fired];
    sections.visit(
        &Slot::Pending,
        |held| matches!(held, Inputs::Pending(rows) if same_rows(rows, state.pending())),
        || Inputs::Pending(state.pending().map(<[_]>::to_vec)),
        |notes| {
            pending_notes(state, notes);
            Ok(())
        },
    )?;
    sections.visit(
        &Slot::Fired,
        |held| matches!(held, Inputs::Fired(rows) if same_rows(rows, state.fired())),
        || Inputs::Fired(state.fired().map(<[_]>::to_vec)),
        |notes| {
            fired_notes(state, notes);
            Ok(())
        },
    )?;
    let mut seen = BTreeMap::<Owner, u32>::new();
    for (owner, rows) in state.delayed_owners() {
        let name = tag_at(pack.read_hour, owner)
            .unwrap_or_else(|| format!("country {} instance {}", owner.reference, owner.instance));
        if let Some(tag) = tag_at(pack.read_hour, owner) {
            tags.insert(tag);
        }
        let nth = seen.entry(self::owner(owner)).or_default();
        let slot = Slot::Delayed(self::owner(owner), *nth);
        *nth += 1;
        sections.visit(
            &slot,
            |held| matches!(held, Inputs::Delayed(held, kept) if *held == name && same_rows(kept, rows)),
            || Inputs::Delayed(name.clone(), rows.map(<[_]>::to_vec)),
            |notes| {
                delayed_notes(&name, rows, notes);
                Ok(())
            },
        )?;
        slots.push(slot);
    }
    seen.clear();
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
        let nth = seen.entry(self::owner(owner.country)).or_default();
        let slot = Slot::Decisions(self::owner(owner.country), *nth);
        *nth += 1;
        sections.visit(
            &slot,
            |held| held.same_decisions(&name, owner),
            || Inputs::decisions(&name, owner),
            |notes| decision_notes(&name, owner, notes),
        )?;
        slots.push(slot);
    }
    let observed: BTreeSet<&Slot> = slots.iter().collect();
    sections.0.retain(|slot, _| observed.contains(slot));
    let id = format!("script-state:{}:{}", pack.generation, pack.read_hour);
    match deferred {
        Some(deferred) => {
            deferred.insert(id.clone(), slots);
        }
        None => {
            for slot in &slots {
                notes.extend(sections.0[slot].notes.iter().cloned());
            }
        }
    }
    let kind = EventFeedKind::ScriptStateObserved;
    builder.feed.rows.push(EventFeedRow {
        id,
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
