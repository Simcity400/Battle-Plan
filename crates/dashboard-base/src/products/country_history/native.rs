//! Exact occurrence-hour country reference, holder and instance witnesses.
use super::CountryKeys;
use crate::metadata::link_slots;
use crate::products::country_groups::{HourSpan, record_hour};
use replay::{
    event_feed_rows::EventFeedCountry,
    frame::{DecodedFrame, ViewIdentity},
    rows::{CountryExperienceGainRow, CountryRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// Only the exact occurrence's recorded tables can name a full native actor.
pub struct Names {
    pub slots: std::collections::HashMap<u32, u32>,
    pub tags: BTreeMap<u32, (u32, u32, String)>,
}

#[derive(Serialize, Deserialize)]
struct Witness {
    first: i32,
    last: i32,
    slot: u32,
    id: u32,
    instance: u32,
    tag: String,
}

/// The last country read's keys, link identity, slots and names.
type Cached = (CountryKeys, ViewIdentity, Vec<(u32, u32)>, Names);

/// Reference histories change only when the exact native witness changes or a
/// fresh read is missing. Decoder allocation identities are never persisted.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct History {
    reads: Vec<HourSpan>,
    references: BTreeMap<u32, Vec<Witness>>,
    open: BTreeSet<u32>,
    hour: Option<i32>,
    #[serde(skip)]
    cached: Option<Cached>,
}
impl History {
    pub(super) fn seal(&mut self) {
        if let Some(hour) = self.hour {
            for reference in &self.open {
                if let Some(run) = self
                    .references
                    .get_mut(reference)
                    .and_then(|runs| runs.last_mut())
                {
                    run.last = hour;
                }
            }
        }
    }

    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) {
        let (Ok(countries), Ok(_links), Ok(gains)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::CountryExperienceGains>(),
        ) else {
            self.seal();
            self.open.clear();
            self.hour = None;
            return;
        };
        if [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryExperienceGains,
        ]
        .iter()
        .any(|table| frame.metrics.table_ns[table.index()] == 0)
        {
            self.seal();
            self.open.clear();
            self.hour = None;
            return;
        }
        let touched = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        let view = &frame.tables[TableId::CountryLinks.index()];
        let unchanged =
            self.cached
                .as_ref()
                .is_some_and(|(keys, link_view, instances, _)| {
                    (!touched(TableId::Countries) || keys.same(countries))
                        && link_view.is(view)
                        && (!touched(TableId::CountryExperienceGains)
                            || instances.len() == gains.len()
                                && instances.iter().zip(gains).all(
                                    |((country, instance), gain)| {
                                        *country == gain.country && *instance == gain.instance
                                    },
                                ))
                });
        let contiguous = self.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        if changes.is_some() && unchanged && contiguous {
            record_hour(&mut self.reads, frame.hour);
            self.hour = Some(frame.hour);
            return;
        }
        self.seal();
        self.open.clear();
        if !unchanged {
            let slots = match self.cached.as_mut() {
                Some((_, links, _, names)) if links.is(view) => std::mem::take(&mut names.slots),
                _ => link_slots(
                    frame
                        .complete::<tables::CountryLinks>()
                        .expect("complete links"),
                ),
            };
            let Ok(names) = Names::from_slots(frame, slots) else {
                return;
            };
            self.cached = Some((
                CountryKeys::read(countries),
                view.identity(),
                gains
                    .iter()
                    .map(|gain| (gain.country, gain.instance))
                    .collect(),
                names,
            ));
        }
        record_hour(&mut self.reads, frame.hour);
        let names = &self.cached.as_ref().expect("observed native witnesses").3;
        for (&reference, &slot) in &names.slots {
            if reference == 0 {
                continue;
            }
            let Some((id, instance, tag)) = names.tags.get(&slot) else {
                continue;
            };
            self.open.insert(reference);
            let runs = self.references.entry(reference).or_default();
            if let Some(last) = runs.last_mut()
                && last.last.checked_add(1) == Some(frame.hour)
                && (last.slot, last.id, last.instance, last.tag.as_str())
                    == (slot, *id, *instance, tag.as_str())
            {
                last.last = frame.hour;
            } else {
                runs.push(Witness {
                    first: frame.hour,
                    last: frame.hour,
                    slot,
                    id: *id,
                    instance: *instance,
                    tag: tag.clone(),
                });
            }
        }
        self.hour = Some(frame.hour);
    }
    pub fn tag(&self, hour: i32, country: EventFeedCountry) -> Option<String> {
        if country.reference == 0 {
            return None;
        }
        let read = self
            .reads
            .get(self.reads.partition_point(|span| span.last_hour < hour))?;
        if hour < read.first_hour {
            return None;
        }
        let runs = self.references.get(&country.reference)?;
        let at = runs.partition_point(|run| run.last < hour);
        let run = runs.get(at).or_else(|| {
            (self.open.contains(&country.reference)
                && self.hour.is_some_and(|frontier| frontier >= hour))
            .then(|| runs.last())
            .flatten()
        })?;
        (run.first <= hour && run.id == country.id && run.instance == country.instance)
            .then(|| run.tag.clone())
    }
}

impl Names {
    #[cfg(any(test, feature = "test-support"))]
    pub fn at(frame: &DecodedFrame, branch: u64, hour: i32) -> io::Result<Self> {
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
        let links = frame.complete::<tables::CountryLinks>()?;
        let slots = link_slots(links);
        Self::from_slots(frame, slots)
    }

    fn from_slots(
        frame: &DecodedFrame,
        slots: std::collections::HashMap<u32, u32>,
    ) -> io::Result<Self> {
        let countries = frame.complete::<tables::Countries>()?;
        let gains = frame.complete::<tables::CountryExperienceGains>()?;
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn tag(&self, country: EventFeedCountry) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::scale_frame;
    use replay::{
        frame::TableView,
        rows::{CountryLinkRow, TableRows},
    };

    fn frame(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = scale_frame(1, hour, count);
        *frame.tables[TableId::CountryExperienceGains.index()] =
            TableView::Complete(TableRows::CountryExperienceGains(
                (0..count)
                    .map(|index| CountryExperienceGainRow {
                        country: 100_000 + index,
                        instance: 7 + index,
                        ..Default::default()
                    })
                    .collect(),
            ));
        for table in [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryExperienceGains,
        ] {
            frame.metrics.table_ns[table.index()] = 1;
        }
        frame
    }
    fn country(index: u32) -> EventFeedCountry {
        EventFeedCountry {
            reference: 100_000 + index,
            id: 100_000 + index,
            instance: 7 + index,
        }
    }

    #[test]
    fn native_witness_country_and_instance_changes_retain_the_link_allocation() {
        use crate::prepared::frame_changes::FrameChangeTracker;
        let format = replay::format::FormatVersion::CURRENT;
        let mut sample = frame(100, 3);
        let mut history = History::default();
        let mut tracker = FrameChangeTracker::new();
        history.observe(&sample, Some(&tracker.observe(&sample, format)));
        let slots = history
            .cached
            .as_ref()
            .unwrap()
            .3
            .slots
            .get(&100_000)
            .unwrap() as *const u32;
        for hour in 101..=102 {
            sample.hour = hour;
            if hour == 101 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *sample.tables[TableId::Countries.index()]
                {
                    rows[0].tag = b"recorded alias".to_vec();
                }
            } else if let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
                &mut *sample.tables[TableId::CountryExperienceGains.index()]
            {
                rows[0].instance += 1;
            }
            history.observe(&sample, Some(&tracker.observe(&sample, format)));
            assert_eq!(
                slots,
                history
                    .cached
                    .as_ref()
                    .unwrap()
                    .3
                    .slots
                    .get(&100_000)
                    .unwrap() as *const u32
            );
            let exact = Names::at(&sample, 1, hour).unwrap();
            for actor in [
                country(0),
                EventFeedCountry {
                    instance: 8,
                    ..country(0)
                },
                country(1),
            ] {
                assert_eq!(history.tag(hour, actor), exact.tag(actor));
            }
        }
        sample.hour = 103;
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *sample.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 200_000,
                canonical_slot: 0,
            });
        }
        history.observe(&sample, Some(&tracker.observe(&sample, format)));
        let names = &history.cached.as_ref().unwrap().3;
        assert_ne!(slots, names.slots.get(&100_000).unwrap() as *const u32);
        let alias = EventFeedCountry {
            reference: 200_000,
            instance: 8,
            ..country(0)
        };
        assert_eq!(
            history.tag(103, alias),
            Names::at(&sample, 1, 103).unwrap().tag(alias)
        );
    }

    #[test]
    fn prepared_native_witnesses_grow_with_changes_not_hours_and_resume_exactly() {
        native_witness_resume_parity(3);
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn prepared_native_witnesses_grow_with_changes_not_hours_and_resume_exactly_at_5000_countries()
    {
        native_witness_resume_parity(5_000);
    }

    fn native_witness_resume_parity(count: u32) {
        let mut history = History::default();
        let mut sample = frame(100, count);
        history.observe(&sample, None);
        let initial = serde_json::to_vec(&history).unwrap();
        for hour in 101..=199 {
            sample.hour = hour;
            history.observe(&sample, None);
        }
        assert_eq!(serde_json::to_vec(&history).unwrap().len(), initial.len());
        assert_eq!(history.reads.len(), 1);
        assert_eq!(history.references.len(), count as usize);
        assert!(history.references.values().all(|runs| runs.len() == 1));
        history = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        let mut missing = frame(200, count);
        missing.metrics.table_ns[TableId::CountryExperienceGains.index()] = 0;
        history.observe(&missing, None);
        assert_eq!(history.tag(200, country(0)), None);
        let mut renamed = frame(201, count);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *renamed.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"recorded replacement name".to_vec();
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *renamed.tables[TableId::CountryLinks.index()]
        {
            rows.insert(
                0,
                CountryLinkRow {
                    id: 77,
                    canonical_slot: 0,
                },
            );
        }
        history.observe(&renamed, None);
        let exact = Names::at(&renamed, 1, 201).unwrap();
        let alias = EventFeedCountry {
            reference: 77,
            ..country(0)
        };
        for value in [
            country(0),
            alias,
            EventFeedCountry {
                instance: 99,
                ..country(0)
            },
            EventFeedCountry {
                reference: 0,
                ..country(0)
            },
        ] {
            assert_eq!(history.tag(201, value), exact.tag(value));
        }
        assert_eq!(
            history.tag(199, country(0)),
            Names::at(&sample, 1, 199).unwrap().tag(country(0))
        );
        assert_eq!(history.reads.len(), 2);
        assert_eq!(history.references[&country(0).reference].len(), 2);
        assert_eq!(
            history.tag(201, alias),
            Some("recorded replacement name".into())
        );
    }
}
