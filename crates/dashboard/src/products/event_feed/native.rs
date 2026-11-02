//! Native observations keep action roles and exact occurrence-hour identity witnesses.

use super::{Builder, EventFeedCause, EventFeedKind, EventFeedRow, check_cancel};
#[cfg(test)]
use crate::metadata::validate_campaign;
#[cfg(test)]
use crate::products::country_history::NativeNames as Names;
use replay::container::Replay;
use replay_data::{
    event_feed_rows::EventFeedCause as NativeCause,
    event_feed_rows::EventFeedCountry,
    event_feed_rows::EventFeedCoverage,
    event_feed_rows::EventFeedEvent,
    event_feed_rows::EventFeedHistoryRow,
    event_feed_rows::EventFeedRoute,
    event_feed_rows::EventFeedScriptChange,
    event_feed_rows::EventFeedSource,
    event_feed_rows::decisions::Operation,
    event_feed_rows::state::{HeldPayloads, PackHeader, PreparedHistory},
    rows::Recorded,
};
#[cfg(test)]
use replay_data::{
    event_feed_rows::decisions::DecisionState, event_feed_rows::state::Change, rows::TableId,
    rows::tables,
};
#[cfg(test)]
use replay_frame::frame::FrameDecoder;
use serde::{Deserialize, Serialize};
pub(super) mod retained;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// Each final decision roster rebuild's inputs, by occurrence.
pub(super) type Rosters = BTreeMap<u64, super::decision_roster::Inputs>;

/// Whether `event` is a final decision roster rebuild, the one retained-state change shown as
/// an occurrence.
fn roster(event: &EventFeedEvent) -> bool {
    event.script.is_some_and(|script| {
        matches!(script.change, Some(EventFeedScriptChange::Decision(change))
            if change.operation == Operation::Rebuild && change.container == 3)
    })
}

/// The final roster rebuilds' inputs, from their owners as `reduce`, reducing `pack`, leaves
/// each at its rebuild.
#[cfg(test)]
fn rosters(
    pack: &EventFeedHistoryRow,
    reduce: impl FnOnce(&mut dyn FnMut(&DecisionState, Change<'_>) -> io::Result<()>) -> io::Result<()>,
) -> io::Result<Rosters> {
    let covered = pack.coverage == EventFeedCoverage::Complete && pack.script_refusals == 0;
    let mut captured = Rosters::new();
    reduce(
        &mut |state: &DecisionState, change: Change<'_>| -> io::Result<()> {
            if roster(change.event) {
                captured.insert(
                    change.sequence,
                    super::decision_roster::Inputs::captured(state, change, covered),
                );
            }
            Ok(())
        },
    )?;
    Ok(captured)
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Projection {
    last_sequence: Option<u64>,
    script_state: replay_data::event_feed_rows::state::ScriptEventState,
    /// The branch's recorded event losses, as the reads each lies between.
    event_losses: Vec<(i32, i32)>,
    /// The retained state's parts as the last observation saw them.
    #[serde(default)]
    sections: retained::Sections,
    #[serde(skip)]
    contents: retained::Contents,
    /// The payloads of the packs of the walk being reduced.
    #[serde(skip)]
    held: HeldPayloads,
}
impl Projection {
    pub(super) fn losses(&mut self, replay: &Replay, branch: u64) {
        self.event_losses = replay.event_losses(branch).collect();
    }
    #[cfg(test)]
    pub(super) fn append_named(
        &mut self,
        builder: &mut Builder,
        pack: &EventFeedHistoryRow,
        mut tag_at: impl FnMut(i32, EventFeedCountry) -> Option<String>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        pack.validate()?;
        let header = PackHeader::of(pack)?;
        self.follows(&header)?;
        self.notices(builder, &header);
        let mut captured = rosters(pack, |visit| {
            self.script_state.observe_decisions(pack, visit)
        })?;
        Self::occurrences(builder, pack, &mut tag_at, &mut captured, cancelled)?;
        self.retain(builder, pack, &header, &mut tag_at)
    }
    /// Takes in the payloads `packet` introduced: every prepared read of a walk, in order, whether
    /// or not it is reduced.
    pub(super) fn admit(&mut self, packet: &PreparedHistory) -> io::Result<()> {
        self.held.admit(packet)
    }
    /// A read validated where it was prepared, reduced into the retained state, with each final
    /// roster rebuild's owner as the reduction left it there ([`super::decision_roster::Captured`]).
    /// `sure`: the read's walk reduced every earlier read to its end.
    pub(super) fn reduce(
        &mut self,
        packet: &PreparedHistory,
        sure: bool,
    ) -> io::Result<BTreeMap<u64, super::decision_roster::Captured>> {
        let header = packet.header();
        let covered = header.coverage == EventFeedCoverage::Complete && header.script_refusals == 0;
        let Self {
            script_state,
            contents,
            held,
            ..
        } = self;
        let mut captured = BTreeMap::new();
        script_state.observe_prepared(packet, held, sure, |state, change| {
            if roster(change.event) {
                let kept = retained::decided(contents, change.event.actor.id, state);
                captured.insert(
                    change.sequence,
                    super::decision_roster::Captured::of(state, change, covered, kept),
                );
            }
            Ok(())
        })?;
        Ok(captured)
    }
    /// Whether `pack`'s occurrences follow every one observed before. Validation keeps them in
    /// order, so the first is the earliest.
    pub(super) fn follows(&self, pack: &PackHeader) -> io::Result<()> {
        if pack
            .sequences
            .is_some_and(|(first, _)| self.last_sequence.is_some_and(|last| first <= last))
        {
            return Err(io::Error::other("Native event occurrence sequence differs"));
        }
        Ok(())
    }
    /// The read's coverage notices, made before its reduction.
    pub(super) fn notices(&self, builder: &mut Builder, pack: &PackHeader) {
        if pack.coverage != EventFeedCoverage::Complete {
            // A recorded event loss ending at this read names why.
            let lost = self
                .event_losses
                .iter()
                .find(|&&(_, hour)| hour == pack.read_hour)
                .map(|&(after, hour)| {
                    format!(
                        "{} (between {after} and {hour})",
                        crate::metadata::EVENT_LOSS_REASON
                    )
                });
            let recorded = pack.failure.as_deref().map(|bytes| {
                std::str::from_utf8(bytes).unwrap_or("Recorded failure text is not UTF-8")
            });
            let failure = match (lost, recorded) {
                (Some(lost), Some(recorded)) => format!("{lost}; {recorded}"),
                (Some(lost), None) => lost,
                (None, Some(recorded)) => recorded.to_owned(),
                (None, None) => "The counted interval is not fully observed".to_owned(),
            };
            builder.sampled_unavailable("nativeEventCoverage", "Native event coverage",
                format!("{:?} read at {}: {}; {} outstanding observations. Missing rows do not prove no event occurred.",
                    pack.coverage, pack.read_hour, failure, pack.outstanding), 1);
        }
        for family in pack.refused_script_families() {
            builder.sampled_unavailable("nativeScriptFamilyCoverage", family,
                format!("{family}: interval ending at {} has unobserved native writers; missing changes and unchanged carry are unavailable.", pack.read_hour), 1);
        }
    }
    /// The read's occurrences, each named by `tag_at` and pushed to `builder` as a row with
    /// its operands. Of the retained-state changes only the final roster rebuilds show, with
    /// the inputs `rosters` holds for them.
    pub(super) fn occurrences(
        builder: &mut Builder,
        pack: &EventFeedHistoryRow,
        mut tag_at: impl FnMut(i32, EventFeedCountry) -> Option<String>,
        rosters: &mut Rosters,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for event in &pack.events {
            check_cancel(cancelled)?;
            // Retained membership is consumed by the script-state row API;
            // it is not an invented fire at attachment time.
            if event.source == EventFeedSource::ScriptState && !roster(event) {
                continue;
            }
            let participants =
                [event.actor, event.target].map(|country| tag_at(event.hour, country));
            // Every window's derived peace rows defer to the exact native end.
            if event.source == EventFeedSource::WarEnded
                && let [Some(first), Some(second)] = &participants
            {
                builder
                    .war_ends
                    .insert(super::war_end_key(event.hour, first, second));
            }
            if event.hour < builder.feed.first || event.hour > builder.feed.last {
                continue;
            }
            for ((role, tag), country) in ["actor", "target"]
                .into_iter()
                .zip(&participants)
                .zip([event.actor, event.target])
            {
                if tag.is_none() && !(event.script.is_some() && country.reference as i32 <= 0) {
                    builder.sampled_unavailable("nativeEventCountry", "Native event country",
                        format!("The {role} of event {} at {} has no exact country-reference and published-link witness.",
                            event.sequence, event.hour), 1);
                }
            }
            let kind = match event.source {
                EventFeedSource::Declaration => EventFeedKind::WarDeclared,
                EventFeedSource::Join => EventFeedKind::WarJoined,
                EventFeedSource::CivilWarWarTransfer => EventFeedKind::CivilWarWarTransferred,
                EventFeedSource::Annex => EventFeedKind::Annexed,
                EventFeedSource::WarEnded => EventFeedKind::WarEnded,
                EventFeedSource::Release => EventFeedKind::Released,
                EventFeedSource::StateTransfer => EventFeedKind::StateTransferred,
                EventFeedSource::PeaceConference => EventFeedKind::PeaceConferenceStarted,
                EventFeedSource::ScriptEvent => EventFeedKind::ScriptEventFired,
                EventFeedSource::NewsEvent => EventFeedKind::NewsEventFired,
                EventFeedSource::EventOption => EventFeedKind::EventOptionSelected,
                EventFeedSource::DecisionSelected => EventFeedKind::DecisionSelected,
                EventFeedSource::DecisionRemoved => EventFeedKind::DecisionRemoved,
                EventFeedSource::DecisionCancelled => EventFeedKind::DecisionCancelled,
                EventFeedSource::DecisionTimedOut => EventFeedKind::DecisionTimedOut,
                EventFeedSource::DecisionActivated => EventFeedKind::DecisionActivated,
                EventFeedSource::DecisionCompleted => EventFeedKind::DecisionCompleted,
                EventFeedSource::DecisionReenabled => EventFeedKind::DecisionReenabled,
                EventFeedSource::DecisionRemovalEffect => EventFeedKind::DecisionRemovalEffect,
                EventFeedSource::DecisionCancellationEffect => {
                    EventFeedKind::DecisionCancellationEffect
                }
                EventFeedSource::DecisionCompletionEffect => {
                    EventFeedKind::DecisionCompletionEffect
                }
                EventFeedSource::DecisionTimeoutEffect => EventFeedKind::DecisionTimeoutEffect,
                EventFeedSource::DecisionDurationChanged => EventFeedKind::DecisionDurationChanged,
                EventFeedSource::ScriptState => EventFeedKind::DecisionRosterRebuilt,
            };
            let mut cause = match event.cause {
                Recorded::Value(cause) => {
                    let ally = cause.ally.map(|ally| tag_at(event.hour, ally));
                    if ally == Some(None) {
                        builder.sampled_unavailable("nativeEventCountry", "Native event country",
                            format!("The ally of event {} at {} has no exact country-reference and published-link witness.",
                                event.sequence, event.hour), 1);
                    }
                    Some(cause_words(cause, ally.flatten()))
                }
                Recorded::NotRecorded => None,
            };
            let item = if let Some(script) = event.script {
                let token = &pack.script_tokens[script.token_start as usize
                    ..script.token_start as usize + script.token_len as usize];
                let token = if script.token_unrecorded {
                    builder.sampled_unavailable(
                        "scriptEventToken",
                        "Script content identity",
                        format!(
                            "Occurrence {} content token was not captured.",
                            event.sequence
                        ),
                        1,
                    );
                    None
                } else {
                    match std::str::from_utf8(token) {
                        Ok(token) if !token.is_empty() => Some(token.to_owned()),
                        Ok(_) => script.native_event.map(|id| id.id.to_string()),
                        Err(_) => {
                            builder.sampled_unavailable(
                                "scriptEventToken",
                                "Script content identity",
                                format!(
                                    "Occurrence {} has a non-UTF-8 content token.",
                                    event.sequence
                                ),
                                1,
                            );
                            None
                        }
                    }
                };
                if script.scope_unrecorded {
                    builder.sampled_unavailable(
                        "scriptEventScope",
                        "Bound script scope",
                        format!(
                            "Occurrence {} scope graph was not fully captured.",
                            event.sequence
                        ),
                        1,
                    );
                }
                let words = cause.get_or_insert_with(|| EventFeedCause {
                    how: None,
                    within: None,
                    conference: None,
                    state: None,
                    ally: None,
                    reason: None,
                    notes: Vec::new(),
                });
                if let Some(id) = script.native_event {
                    words.notes.push(format!("Native event identity: {id}."));
                }
                if let Some(id) = script.instance {
                    words.notes.push(format!("Event instance: {id}."));
                }
                if let Some(option) = script.option {
                    words
                        .notes
                        .push(format!("Selected native option ordinal: {option}."));
                }
                if let Some(state) = script.lifecycle {
                    words
                        .notes
                        .push(format!("Native lifecycle result: {state}."));
                }
                if let Some(days) = script.days {
                    words
                        .notes
                        .push(format!("Native duration result: {days} days."));
                }
                if let Some(target) = script.country_target {
                    words
                        .notes
                        .push(format!("Country target reference: {target}."));
                }
                if let Some(target) = script.numeric_target {
                    words.notes.push(format!("Numeric target: {target}."));
                }
                for (id, scope) in pack.script_scopes[script.scope_start as usize
                    ..script.scope_start as usize + script.scope_len as usize]
                    .iter()
                    .enumerate()
                {
                    words.notes.push(format!("Scope {id}: country reference {}; state {}; ROOT {}, FROM {}, PREV {}; random words {}, {}.",
                        scope.country, scope.state, scope.bindings[0], scope.bindings[1], scope.bindings[2], scope.random[0], scope.random[1]));
                    for (kind, reference) in [
                        "character",
                        "operation",
                        "ace",
                        "unit",
                        "industrial organisation",
                        "purchase contract",
                        "raid",
                        "project",
                        "faction",
                    ]
                    .into_iter()
                    .zip(scope.references)
                    {
                        if reference != 0 {
                            words
                                .notes
                                .push(format!("Scope {id} {kind} reference: {reference}."));
                        }
                    }
                    for (missing, operand) in [
                        (scope.combat_context_unrecorded, "combat context"),
                        (scope.target_storage_unrecorded, "saved targets"),
                        (scope.tooltip_storage_unrecorded, "tooltip targets"),
                    ] {
                        if missing {
                            builder.sampled_unavailable("scriptEventScope", "Bound script scope",
                            format!("Occurrence {} scope {id}: {operand} operands were not captured.", event.sequence), 1);
                        }
                    }
                }
                builder.script_inputs.insert(
                    format!("native:{}:{}", event.generation, event.sequence),
                    {
                        let mut inputs = super::script_effects::Inputs::captured(pack, event);
                        inputs.roster = rosters.remove(&event.sequence);
                        inputs
                    },
                );
                // Bind presentation to this occurrence's scope/ordinal, even
                // when several executions share one authored identity.
                let index = u32::try_from(builder.feed.items.len())
                    .map_err(|_| io::Error::other("Too many event-feed script items"))?;
                builder
                    .feed
                    .items
                    .push(super::plain_item(token, None, None, None));
                Some(index)
            } else {
                None
            };
            let id = format!("native:{}:{}", event.generation, event.sequence);
            let routes = match event.cause {
                Recorded::Value(cause) => (Some(cause.route as u8), Some(cause.within as u8)),
                Recorded::NotRecorded => (None, None),
            };
            builder.occurrences.insert(
                id.clone(),
                super::Occurrence {
                    generation: event.generation,
                    sequence: event.sequence,
                    event: event
                        .script
                        .and_then(|script| script.native_event)
                        .map(|identity| (identity.kind, identity.id)),
                    instance: event.script.and_then(|script| script.instance),
                    option: event.script.and_then(|script| script.option),
                    lifecycle: event.script.and_then(|script| script.lifecycle),
                    days: event.script.and_then(|script| script.days),
                    actor: (event.actor.id, event.actor.instance),
                    target: (event.target.id, event.target.instance),
                    country_target: event.script.and_then(|script| script.country_target),
                    numeric_target: event.script.and_then(|script| script.numeric_target),
                    token: item
                        .and_then(|index| builder.feed.items.get(index as usize))
                        .and_then(|item| item.token.clone()),
                    route: routes.0,
                    within: routes.1,
                    opener: None,
                    costs: None,
                },
            );
            builder.feed.rows.push(EventFeedRow {
                id,
                hour: event.hour,
                read_hour: pack.read_hour,
                name_hour: event.hour,
                exact: true,
                category: kind.category(),
                kind,
                tags: participants.iter().flatten().cloned().collect(),
                participants: Some(participants),
                item,
                amount: None,
                cause,
                strike: None,
            });
        }
        Ok(())
    }
    /// After a reduction: the read's last occurrence, and its retained-state observation.
    #[cfg(test)]
    pub(super) fn retain(
        &mut self,
        builder: &mut Builder,
        pack: &EventFeedHistoryRow,
        header: &PackHeader,
        tag_at: &mut impl FnMut(i32, EventFeedCountry) -> Option<String>,
    ) -> io::Result<()> {
        self.settle(header);
        if pack.read_hour < builder.feed.first || pack.read_hour > builder.feed.last {
            return Ok(());
        }
        let observed = retained::observations(pack, &mut *tag_at);
        if let Some(observed) = self.observe(header, observed, tag_at)? {
            builder.feed.rows.push(observed.row(false)?);
        }
        Ok(())
    }
    /// After a reduction: the read's last occurrence.
    pub(super) fn settle(&mut self, pack: &PackHeader) {
        if let Some((_, last)) = pack.sequences {
            self.last_sequence = Some(last);
        }
    }
    /// The retained-state observation of a reduced read, of its feed's hours, with the
    /// read's own `observations` ([`retained::observations`]), its notes yet to be written.
    pub(super) fn observe(
        &mut self,
        pack: &PackHeader,
        observations: retained::Observations,
        tag_at: &mut impl FnMut(i32, EventFeedCountry) -> Option<String>,
    ) -> io::Result<Option<retained::Observed>> {
        retained::observe(
            pack,
            &self.script_state,
            &mut self.sections,
            &mut self.contents,
            observations,
            tag_at,
        )
    }
    #[cfg(test)]
    pub(super) fn append<'a>(
        &mut self,
        builder: &mut Builder,
        pack: &EventFeedHistoryRow,
        branch: u64,
        mut frame_at: impl FnMut(i32) -> Option<&'a DecodedFrame>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut names = BTreeMap::new();
        for event in &pack.events {
            names.entry(event.hour).or_insert_with(|| {
                frame_at(event.hour).and_then(|frame| Names::at(frame, branch, event.hour).ok())
            });
        }
        self.append_named(
            builder,
            pack,
            |hour, country| {
                names
                    .get(&hour)
                    .and_then(Option::as_ref)
                    .and_then(|names| names.tag(country))
            },
            cancelled,
        )?;
        for event in &pack.events {
            if let Some(inputs) = builder
                .script_inputs
                .get_mut(&format!("native:{}:{}", event.generation, event.sequence))
            {
                inputs.resolve(|country| {
                    names
                        .get(&event.hour)
                        .and_then(Option::as_ref)
                        .and_then(|names| {
                            names
                                .tag(country)
                                .and_then(|_| names.slots.get(&country.reference).copied())
                        })
                });
            }
        }
        Ok(())
    }
}

/// Two selected-table passes retain only identities actually used by events.
#[cfg(test)]
pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    builder: &mut Builder,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    if let Some(reason) = replay.format.table_refusal(TableId::EventFeedHistory) {
        builder.unavailable(
            "nativeEventCoverage",
            "Native war and annexation events",
            reason,
            None,
        );
        return Ok(());
    }
    let event_losses: Vec<_> = replay.event_losses(branch).collect();
    let selected = [TableId::Campaign, TableId::EventFeedHistory];
    let mut packs = Vec::new();
    let mut wanted = BTreeMap::<i32, std::collections::BTreeSet<u32>>::new();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    crate::blocks::scan_blocks(
        replay,
        blocks.iter().copied(),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(error) => {
                        builder.sampled_unavailable(
                            "nativeEventCoverage",
                            "Native event coverage",
                            error.to_string(),
                            1,
                        );
                        continue;
                    }
                };
                for encoded in frames {
                    check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&encoded) {
                        Ok(frame) => frame,
                        Err(error) => {
                            builder.sampled_unavailable(
                                "nativeEventCoverage",
                                "Native event coverage",
                                error.to_string(),
                                1,
                            );
                            break;
                        }
                    };
                    let read = validate_campaign(decoded, replay)
                        .and_then(|()| decoded.complete::<tables::EventFeedHistory>());
                    let pack = match read {
                        Ok([pack]) if pack.read_hour == decoded.hour => pack,
                        Ok(_) => {
                            builder.sampled_unavailable(
                                "nativeEventCoverage",
                                "Native event coverage",
                                "Event history has no unique pack for its supplying hour.",
                                1,
                            );
                            continue;
                        }
                        Err(error) => {
                            builder.sampled_unavailable(
                                "nativeEventCoverage",
                                "Native event coverage",
                                error.to_string(),
                                1,
                            );
                            continue;
                        }
                    };
                    if pack.events.is_empty() {
                        Projection {
                            event_losses: event_losses.clone(),
                            ..Projection::default()
                        }
                        .append_named(
                            builder,
                            pack,
                            |_, _| None,
                            cancelled,
                        )?;
                        continue;
                    }
                    for event in &pack.events {
                        if let Some(script) = event.script {
                            for scope in &pack.script_scopes[script.scope_start as usize
                                ..script.scope_start as usize + script.scope_len as usize]
                            {
                                wanted.entry(event.hour).or_default().insert(scope.country);
                            }
                        }
                        let ally = match event.cause {
                            Recorded::Value(NativeCause {
                                ally: Some(ally), ..
                            }) => Some(ally.reference),
                            _ => None,
                        };
                        wanted.entry(event.hour).or_default().extend(
                            [event.actor.reference, event.target.reference]
                                .into_iter()
                                .chain(ally),
                        );
                    }
                    packs.push(replay_data::rows::RowCodec::try_clone(pack)?);
                }
            }
            Ok(())
        },
    )?;
    let mut names = BTreeMap::new();
    if !wanted.is_empty() {
        let selected = [
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryExperienceGains,
        ];
        // Only blocks holding a named event's hour are read.
        let named = blocks.into_iter().filter(|block| {
            wanted
                .range(block.first_hour..=block.last_hour)
                .next()
                .is_some()
        });
        crate::blocks::scan_blocks(replay, named, &selected, cancelled, |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for encoded in frames {
                    check_cancel(cancelled)?;
                    let Ok(frame) = decoder.advance(&encoded) else {
                        break;
                    };
                    let Some(references) = wanted.get(&frame.hour) else {
                        continue;
                    };
                    if validate_campaign(frame, replay).is_err() {
                        continue;
                    }
                    let Ok(mut at) = Names::at(frame, branch, frame.hour) else {
                        continue;
                    };
                    at.slots
                        .retain(|reference, _| references.contains(reference));
                    let slots: std::collections::BTreeSet<_> = at.slots.values().copied().collect();
                    at.tags.retain(|slot, _| slots.contains(slot));
                    names.insert(frame.hour, at);
                }
            }
            Ok(())
        })?;
    }
    let mut projection = Projection {
        event_losses,
        ..Projection::default()
    };
    for pack in packs {
        if let Err(error) = projection.append_named(
            builder,
            &pack,
            |hour, country| names.get(&hour).and_then(|names| names.tag(country)),
            cancelled,
        ) {
            check_cancel(cancelled)?;
            builder.sampled_unavailable(
                "nativeEventCoverage",
                "Native event coverage",
                error.to_string(),
                1,
            );
        }
        for event in &pack.events {
            if let Some(inputs) = builder
                .script_inputs
                .get_mut(&format!("native:{}:{}", event.generation, event.sequence))
            {
                inputs.resolve(|country| {
                    names.get(&event.hour).and_then(|names| {
                        names
                            .tag(country)
                            .and_then(|_| names.slots.get(&country.reference).copied())
                    })
                });
            }
        }
    }
    Ok(())
}

/// A recorded cause in words. Routes name the native call site
/// (`EventFeedRoute`); the words say what reaches it.
fn cause_words(cause: NativeCause, ally: Option<String>) -> EventFeedCause {
    let notes = [
        (
            is_join(cause.route),
            "against every enemy of the ally's side",
        ),
        (is_war_end(cause.route), "with a truce"),
        (is_annexation(cause.route), "taking over its units"),
    ];
    EventFeedCause {
        how: route_words(cause.route).map(str::to_owned),
        within: route_words(cause.within).map(str::to_owned),
        conference: cause.conference.map(|conference| conference.to_string()),
        state: cause.state,
        ally,
        reason: cause.reason.map(|reason| match reason {
            // War-relation hostility reasons, selected by their parsed tokens.
            0 => "war".to_owned(),
            1 => "puppet".to_owned(),
            2 => "ally".to_owned(),
            3 => "asked to join".to_owned(),
            4 => "guarantee".to_owned(),
            5 => "not applicable".to_owned(),
            other => format!("native reason {other}"),
        }),
        notes: notes
            .into_iter()
            .filter(|(route, _)| *route && cause.flags & 1 != 0)
            .map(|(_, note)| note.to_owned())
            .collect(),
    }
}

fn is_join(route: EventFeedRoute) -> bool {
    (20..30).contains(&(route as u8))
}

fn is_war_end(route: EventFeedRoute) -> bool {
    (30..40).contains(&(route as u8))
}

/// Annexation routes, whose first byte identifies troop transfer.
fn is_annexation(route: EventFeedRoute) -> bool {
    (50..60).contains(&(route as u8))
}

fn route_words(route: EventFeedRoute) -> Option<&'static str> {
    use EventFeedRoute::*;
    Some(match route {
        None => return Option::None,
        DiplomaticCommand => "a diplomatic action",
        DeclareWarEffect => "a declare_war_on effect",
        CivilWarDeclaration => "a civil war's declaration",
        GiveGuaranteeEffect => "a give_guarantee effect",
        GiveMilitaryAccessEffect => "a give_military_access effect",
        EmbargoEffect => "an embargo effect",
        GiveMarketAccessEffect => "a market access effect",
        OfferJoinFaction => "an offer to join a faction",
        ConsoleDiplomacy
        | ConsoleCivilWar
        | ConsoleCivilWarSecond
        | ConsolePeace
        | ConsoleAnnex
        | ConsoleAnnexSecond
        | ConsoleStateOwner => "a console command",
        StartCivilWarEffect => "a start_civil_war effect",
        Coup => "a coup",
        DeclarationJoin => "its overlord's war declaration",
        AddToWarEffect => "an add_to_war effect",
        SubjectJoinsOverlord => "becoming a subject",
        OverlordJoinsSubject => "gaining a subject",
        JoinAlly => "a request to join an ally's war",
        CallAlly => "a call to arms",
        CivilWarTransferEnd => "a civil war taking over the old country's wars",
        AnnexEnd | StateAnnex => "an annexation",
        SubjectOverlordEnd => "becoming the subject of its enemy",
        SubjectWarsEnd => "a subject relation ending the subject's wars",
        StatusResetEnd => "a diplomatic status reset",
        PeaceEnd => "a peace conference",
        StartPeaceConferenceEffect => "a start_peace_conference effect",
        WhitePeaceEffect => "a white_peace effect",
        Surrender => "a capitulation",
        PeaceProposal => "an accepted peace proposal",
        PeaceApplication => "a peace conference's outcome",
        CivilWarAnnex => "a civil war's resolution",
        Integration => "integration at the lowest autonomy",
        BorderWarAnnex | BorderWarAnnexSecond | StateBorderWar | StateBorderWarSecond => {
            "a border war"
        }
        PeaceAnnex => "a peace conference annexing a loser left without states",
        AnnexEffect => "an annex_country effect",
        TransferStateToAnnex | TransferStateToEffect => "a transfer_state_to effect",
        TransferStateAnnex | TransferStateEffect => "a transfer_state effect",
        ReleasePuppetEffect => "a release_puppet effect",
        ReleasePuppetOnControlledEffect => "a release_puppet_on_controlled effect",
        ReleaseEffect => "a release effect",
        ReleaseOnControlledEffect => "a release_on_controlled effect",
        ReleaseAutonomyEffect => "a release_autonomy effect",
        ReleaseCommand => "a release command",
        StateOwnerFix => "the owner and controller consistency fix",
        SetStateOwnerEffect | SetStateOwnerToEffect => "a state owner effect",
        StatePeaceWinner => "a peace conference giving a winner its state",
        StateDaily => "the daily state update",
        StateRead => "loading a save",
        StateCivilWar | StateCivilWarSecond | StateCivilWarTransfer => "a civil war",
        StatePeaceLiberated => "a peace conference liberating a country",
        StateRelease => "a release",
        StatePeaceBeneficiary => "a peace conference giving a beneficiary its state",
        StateImpassable => "an impassable state following its neighbours",
        StateHistory => "a history owner change",
        Decision | TargetedDecision => "a decision",
        DecisionTimeout => "a decision timing out",
        ScriptedDiplomacy => "a scripted diplomatic action",
        OnAction => "an on_action",
    })
}

#[cfg(test)]
use replay_data::{rows::CountryExperienceGainRow, rows::CountryRow};
#[cfg(test)]
use replay_frame::frame::DecodedFrame;
#[cfg(test)]
pub(super) mod tests;
