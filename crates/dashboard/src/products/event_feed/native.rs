//! Native observations keep action roles and exact occurrence-hour identity witnesses.

use super::{Builder, EventFeedCause, EventFeedKind, EventFeedRow, check_cancel};
use crate::metadata::{link_slots, validate_campaign};
use replay::{
    container::Replay,
    event_feed_rows::{
        EventFeedCause as NativeCause, EventFeedCountry, EventFeedCoverage, EventFeedHistoryRow,
        EventFeedRoute, EventFeedSource,
    },
    frame::{DecodedFrame, FrameDecoder},
    rows::{CountryExperienceGainRow, CountryRow, Recorded, TableId, tables},
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// Only the exact occurrence's recorded tables can name a full native actor.
pub(super) struct Names {
    slots: std::collections::HashMap<u32, u32>,
    tags: BTreeMap<u32, (u32, u32, String)>,
}
impl Names {
    pub(super) fn at(frame: &DecodedFrame, branch: u64, hour: i32) -> io::Result<Self> {
        if frame.branch != branch || frame.hour != hour {
            return Err(io::Error::other(
                "No same-branch occurrence-hour country witness",
            ));
        }
        for id in [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryExperienceGains,
        ] {
            if frame.metrics.table_ns[id.index()] == 0 {
                return Err(io::Error::other(
                    "Occurrence-hour country identity was not freshly read",
                ));
            }
        }
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        let gains = frame.complete::<tables::CountryExperienceGains>()?;
        let slots = link_slots(links);
        let mut holders: BTreeMap<u32, Option<&CountryRow>> = BTreeMap::new();
        let mut tag_slots: BTreeMap<&[u8], Option<u32>> = BTreeMap::new();
        for country in countries {
            let Some(&slot) = slots.get(&country.id) else {
                continue;
            };
            if country.canonical_slot != slot {
                continue;
            }
            holders
                .entry(slot)
                .and_modify(|row| *row = None)
                .or_insert(Some(country));
            tag_slots
                .entry(country.tag.as_slice())
                .and_modify(|seen| *seen = None)
                .or_insert(Some(slot));
        }
        let mut instances: BTreeMap<u32, Option<&CountryExperienceGainRow>> = BTreeMap::new();
        for gain in gains {
            if let Some(&slot) = slots.get(&gain.country) {
                instances
                    .entry(slot)
                    .and_modify(|row| *row = None)
                    .or_insert(Some(gain));
            }
        }
        let mut tags = BTreeMap::new();
        for (slot, country) in holders {
            let Some(country) = country else { continue };
            let Some(Some(gain)) = instances.get(&slot) else {
                continue;
            };
            if gain.country != country.id
                || tag_slots.get(country.tag.as_slice()) != Some(&Some(slot))
            {
                continue;
            }
            let Ok(tag) = std::str::from_utf8(&country.tag) else {
                continue;
            };
            if tag.is_empty() || tag.contains('\0') {
                continue;
            }
            // The gain census reads current positive country references and IDs.
            // Complete describes awards, not identity.
            tags.insert(slot, (country.id, gain.instance, tag.to_owned()));
        }
        Ok(Self { slots, tags })
    }
    pub(super) fn tag(&self, country: EventFeedCountry) -> Option<String> {
        // Country reference zero's null object is excluded from the
        // gain census. Original-tag buckets/physical IDs do not prove its +C.
        if country.reference == 0 {
            return None;
        }
        let slot = self.slots.get(&country.reference)?;
        let (id, instance, tag) = self.tags.get(slot)?;
        (*id == country.id && *instance == country.instance).then(|| tag.clone())
    }
}

#[derive(Default)]
pub(super) struct Projection {
    last_sequence: Option<u64>,
    /// The branch's recorded event losses, as the reads each lies between.
    event_losses: Vec<(i32, i32)>,
}
impl Projection {
    fn append_named<'a>(
        &mut self,
        builder: &mut Builder,
        pack: &EventFeedHistoryRow,
        mut names_at: impl FnMut(i32) -> Option<&'a Names>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        pack.validate()?;
        if pack.events.iter().any(|event| {
            self.last_sequence
                .is_some_and(|last| event.sequence <= last)
        }) {
            return Err(io::Error::other("Native event occurrence sequence differs"));
        }
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
        for event in &pack.events {
            check_cancel(cancelled)?;
            self.last_sequence = Some(event.sequence);
            let named = names_at(event.hour);
            let participants = [event.actor, event.target]
                .map(|country| named.and_then(|names| names.tag(country)));
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
            for (role, tag) in ["actor", "target"].into_iter().zip(&participants) {
                if tag.is_none() {
                    builder.sampled_unavailable("nativeEventCountry", "Native event country",
                        format!("The {role} of event {} at {} has no exact country-reference, ID and instance witness.",
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
            };
            let cause = match event.cause {
                Recorded::Value(cause) => {
                    let ally = cause
                        .ally
                        .map(|ally| named.and_then(|names| names.tag(ally)));
                    if ally == Some(None) {
                        builder.sampled_unavailable("nativeEventCountry", "Native event country",
                            format!("The ally of event {} at {} has no exact country-reference, ID and instance witness.",
                                event.sequence, event.hour), 1);
                    }
                    Some(cause_words(cause, ally.flatten()))
                }
                Recorded::NotRecorded => None,
            };
            builder.feed.rows.push(EventFeedRow {
                id: format!("native:{}:{}", event.generation, event.sequence),
                hour: event.hour,
                read_hour: pack.read_hour,
                name_hour: event.hour,
                exact: true,
                category: kind.category(),
                kind,
                tags: participants.iter().flatten().cloned().collect(),
                participants: Some(participants),
                item: None,
                amount: None,
                cause,
                strike: None,
            });
        }
        Ok(())
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
            |hour| names.get(&hour).and_then(Option::as_ref),
            cancelled,
        )
    }
}

/// Two selected-table passes retain only identities actually used by events.
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
                            |_| None,
                            cancelled,
                        )?;
                        continue;
                    }
                    for event in &pack.events {
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
                    packs.push(replay::rows::RowCodec::try_clone(pack)?);
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
        if let Err(error) =
            projection.append_named(builder, &pack, |hour| names.get(&hour), cancelled)
        {
            check_cancel(cancelled)?;
            builder.sampled_unavailable(
                "nativeEventCoverage",
                "Native event coverage",
                error.to_string(),
                1,
            );
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
mod tests;
