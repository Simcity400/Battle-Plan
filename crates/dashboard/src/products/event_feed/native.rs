//! Native observations keep action roles and exact occurrence-hour identity witnesses.

use super::{Builder, EventFeedKind, EventFeedRow, check_cancel};
use crate::metadata::{link_slots, validate_campaign};
use replay::{
    container::Replay,
    event_feed_rows::{EventFeedCountry, EventFeedCoverage, EventFeedHistoryRow, EventFeedSource},
    frame::{DecodedFrame, FrameDecoder},
    rows::{CountryExperienceGainRow, CountryRow, TableId, tables},
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// Only the exact occurrence's recorded tables can name a full native actor.
struct Names {
    slots: std::collections::HashMap<u32, u32>,
    tags: BTreeMap<u32, (u32, u32, String)>,
}
impl Names {
    fn at(frame: &DecodedFrame, branch: u64, hour: i32) -> io::Result<Self> {
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
            // snapshot@collector::experience_gains reads native +8/+C for every
            // current positive country. complete describes awards, not identity.
            tags.insert(slot, (country.id, gain.instance, tag.to_owned()));
        }
        Ok(Self { slots, tags })
    }
    fn tag(&self, country: EventFeedCountry) -> Option<String> {
        // ResolveCountryReference@140BA56A0's zero object is excluded from the
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
            let failure = match pack.failure.as_deref() {
                Some(bytes) => {
                    std::str::from_utf8(bytes).unwrap_or("Recorded failure text is not UTF-8")
                }
                None => "The counted interval is not fully observed",
            };
            builder.sampled_unavailable("nativeEventCoverage", "Native event coverage",
                format!("{:?} read at {}: {}; {} outstanding observations. Missing rows do not prove no event occurred.",
                    pack.coverage, pack.read_hour, failure, pack.outstanding), 1);
        }
        for event in &pack.events {
            check_cancel(cancelled)?;
            self.last_sequence = Some(event.sequence);
            if event.hour < builder.feed.first || event.hour > builder.feed.last {
                continue;
            }
            let named = names_at(event.hour);
            let participants = [event.actor, event.target]
                .map(|country| named.and_then(|names| names.tag(country)));
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
    let selected = [TableId::Campaign, TableId::EventFeedHistory];
    let mut packs = Vec::new();
    let mut wanted = BTreeMap::<i32, std::collections::BTreeSet<u32>>::new();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    for block in &blocks {
        check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = match replay.read_block_selected(block, &selected) {
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
                Projection::default().append_named(builder, pack, |_| None, cancelled)?;
                continue;
            }
            for event in &pack.events {
                wanted
                    .entry(event.hour)
                    .or_default()
                    .extend([event.actor.reference, event.target.reference]);
            }
            packs.push(replay::rows::RowCodec::try_clone(pack)?);
        }
    }
    let mut names = BTreeMap::new();
    if !wanted.is_empty() {
        let selected = [
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryExperienceGains,
        ];
        for block in blocks {
            check_cancel(cancelled)?;
            if wanted
                .range(block.first_hour..=block.last_hour)
                .next()
                .is_none()
            {
                continue;
            }
            let mut decoder = FrameDecoder::for_tables(&selected);
            let Ok(frames) = replay.read_block_selected(block, &selected) else {
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
    }
    let mut projection = Projection::default();
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

#[cfg(test)]
mod tests;
