//! Feed-only projection of shared group runs and recorded diplomacy facts.
//! Group boundaries use the Factions, Wars and smart-group runs.

use crate::metadata::{LinkTags, slot_read};
#[cfg(test)]
use crate::{
    metadata::{branch_bounds, validate_campaign},
    products::country_groups,
};
#[cfg(test)]
use replay::{container::Replay, frame::FrameDecoder};
use replay::{
    frame::DecodedFrame,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[cfg(test)]
use crate::products::country_groups::changes::Refusal as Unavailable;
use crate::products::country_groups::changes::{Change as GroupMoment, Scan};

/// The direct diplomacy walk's continuation, shared with preparation.
/// Completed moments leave this state; only current countries and seen relation starts remain.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Observer {
    tags: LinkTags,
    named_since: BTreeMap<u32, (String, i32)>,
    since: Option<i32>,
    first_observed: Option<i32>,
    previous_hour: Option<i32>,
    existence: BTreeMap<u32, (u32, String, i32, i32)>,
    guarantees: BTreeSet<(u32, u32, i32)>,
}
impl Observer {
    pub(super) fn observe(
        &mut self,
        decoded: &DecodedFrame,
        format: replay::format::FormatVersion,
        out: &mut Scan,
    ) -> io::Result<()> {
        let hour = decoded.hour;
        let first_observed = *self.first_observed.get_or_insert(hour);
        if self
            .previous_hour
            .is_some_and(|previous| previous.checked_add(1) != Some(hour))
        {
            self.since = None;
        }
        self.previous_hour = Some(hour);
        let Ok(rebuilt) = self.tags.update(decoded) else {
            self.since = None;
            out.refuse(
                "diplomacyIdentity",
                "Diplomacy",
                "Country identities are unresolved.",
            )?;
            return Ok(());
        };
        if rebuilt {
            self.named_since
                .retain(|raw, (tag, _)| self.tags.get(*raw) == Some(tag));
            for (raw, tag) in self.tags.iter() {
                self.named_since
                    .entry(*raw)
                    .or_insert_with(|| (tag.clone(), hour));
            }
        }
        let continuous = *self.since.get_or_insert(hour);
        // Countries includes every non-null canonical country slot, with no
        // owned-state filter. A country with no states can therefore remain
        // recorded after annexation; presence is not existence.
        // CountryCounterInputs supplies the unfiltered owned-state roster count.
        // A positive-to-zero transition means ceased to exist by this supplying
        // read; the cause and exact transition hour are absent.
        if slot_read(
            format,
            &decoded.metrics,
            TableId::CountryCounterInputs,
            hour,
        ) {
            if let Ok(rows) = decoded.complete::<tables::CountryCounterInputs>() {
                out.observed.insert(TableId::CountryCounterInputs);
                let countries = decoded.complete::<tables::Countries>()?;

                for row in rows {
                    let slot = countries
                        .binary_search_by_key(&row.country, |country| country.id)
                        .ok()
                        .map(|index| countries[index].canonical_slot);
                    let (Some(tag), Some(slot)) = (self.tags.get(row.country), slot) else {
                        out.refuse("existenceIdentity", "Country existence", "Owned-state-count observations have unresolved or changed country identities; the disappeared countries are not established.")?;
                        continue;
                    };
                    if row.owned_state_count < 0 {
                        out.refuse(
                            "existenceCount",
                            "Country existence",
                            "Countries have negative recorded owned-state counts.",
                        )?;
                        self.existence.remove(&slot);
                        continue;
                    }
                    if let Some((id, old_tag, count, read_hour)) = self.existence.get(&slot)
                        && *count > 0
                        && row.owned_state_count == 0
                    {
                        if *id == row.country && old_tag == tag {
                            out.moments.push(GroupMoment {
                                kind: "ceasedToExist".into(),
                                name: None,
                                hour,
                                read_hour: hour,
                                name_hour: *read_hour,
                                exact: false,
                                tags: vec![tag.clone()],
                                faction: None,
                            });
                        } else {
                            out.refuse("existenceIdentity", "Country existence", "Owned-state-count observations have unresolved or changed country identities; the disappeared countries are not established.")?;
                        }
                    }
                    match self.existence.get_mut(&slot) {
                        Some((id, old_tag, count, read_hour))
                            if *id == row.country && old_tag.as_str() == tag.as_str() =>
                        {
                            *count = row.owned_state_count;
                            *read_hour = hour;
                        }
                        _ => {
                            self.existence.insert(
                                slot,
                                (row.country, tag.clone(), row.owned_state_count, hour),
                            );
                        }
                    }
                }
            } else {
                out.refuse(
                    "existenceRead",
                    "Country existence",
                    "Owned-state-count reads are incomplete; transitions through them are unknown.",
                )?;
            }
        }
        // Guarantee effects pass scope first and target second; relation
        // initialization retains that direction. The owner-slice can come from
        // either status vector and never reverses it. The relation stores
        // first/second and start_date.
        if let (Ok(rows), Ok(reads)) = (
            decoded.complete::<tables::RelationTuples>(),
            decoded.complete::<tables::SliceReads>(),
        ) {
            let table = TableId::RelationTuples.index() as u32;
            let first = reads.partition_point(|read| read.table < table);
            let last = reads.partition_point(|read| read.table <= table);
            for read in reads[first..last]
                .iter()
                .filter(|read| read.sample_hour == hour && self.tags.get(read.owner).is_some())
            {
                out.observed.insert(TableId::RelationTuples);
                // RelationTuples is ordered by owner, target, type and
                // ordinal. Carried owners require no row traversal.
                let first = rows.partition_point(|row| row.owner_country < read.owner);
                let last = rows.partition_point(|row| row.owner_country <= read.owner);
                for row in rows[first..last].iter().filter(|row| {
                    row.type_id == engine::cguarantee_relation::TYPE && row.cancel == 0
                }) {
                    // An initial history-file relation is a snapshot, never an
                    // event, including a relation published after the first frame.
                    if row.start_hour <= first_observed || row.start_hour > hour {
                        continue;
                    }
                    if !self.guarantees.insert((
                        row.first_country,
                        row.second_country,
                        row.start_hour,
                    )) {
                        continue;
                    }
                    let (Some(first), Some(second)) = (
                        self.tags.get(row.first_country),
                        self.tags.get(row.second_country),
                    ) else {
                        out.refuse(
                            "guaranteeIdentity",
                            "Guarantees",
                            "Guarantees have unresolved countries at their first recorded read.",
                        )?;
                        continue;
                    };
                    let exact = [row.first_country, row.second_country]
                        .into_iter()
                        .all(|raw| {
                            self.named_since
                                .get(&raw)
                                .is_some_and(|(_, from)| continuous.max(*from) <= row.start_hour)
                        });
                    out.moments.push(GroupMoment {
                        kind: "guaranteeGranted".into(),
                        name: None,
                        hour: if exact { row.start_hour } else { hour },
                        read_hour: hour,
                        name_hour: if exact { row.start_hour } else { hour },
                        exact,
                        tags: vec![first.clone(), second.clone()],
                        faction: None,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Only feed consumers decode owner-sliced guarantees and country existence.
/// Faction rows retain their shared id and naming hour; left rows name the
/// previous observation, including a faction that dissolved completely.
/// There is one selected-table decoder/forward pass per block, never seek/hour.
#[cfg(test)]
pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    groups: &country_groups::Scan,
    cancelled: &AtomicBool,
) -> io::Result<Scan> {
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start))
        .ok_or_else(|| io::Error::other("Event feed diplomacy branch is not recorded"))?;
    let mut out = Scan {
        moments: Vec::new(),
        observed: BTreeSet::new(),
        unavailable: Vec::new(),
    };
    country_groups::changes::project(
        groups,
        bounds.0,
        bounds.1,
        &mut out,
        groups.factions.iter().flat_map(|(&id, faction)| {
            faction
                .members
                .iter()
                .flat_map(move |run| [(id, run, false), (id, run, true)])
        }),
        groups.wars.iter(),
        groups.war_starts.iter(),
        groups.capitulations.iter(),
    )?;
    for (key, label, reason, count) in [
        (
            "warStartIdentity",
            "War starts",
            "Observed exact war starts could not be attributed to country identities at their native start hours.",
            groups.unattributed_war_starts,
        ),
        (
            "capitulationIdentity",
            "Capitulations",
            "Observed exact capitulations could not be attributed to country identities at their native event hours.",
            groups.unattributed_capitulations,
        ),
        (
            "capitulationCoverage",
            "Capitulation read coverage",
            "Recorded hours had unread capitulation tables; this counts unread hours, not unseen events.",
            groups.capitulation_unread_hours,
        ),
    ] {
        if count > 0 {
            out.unavailable.push(Unavailable {
                key: key.to_owned(),
                label: label.to_owned(),
                reason: reason.into(),
                count: u64::try_from(count)
                    .map_err(|_| io::Error::other("Event refusal count exceeds u64"))?,
            });
        }
    }
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryCounterInputs,
        TableId::RelationTuples,
        TableId::SliceReads,
    ];
    let mut observer = Observer::default();

    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    observer.since = None;
                    out.refuse(
                        "diplomacyBlock",
                        "Diplomacy",
                        "Selected diplomacy blocks could not be read.",
                    )?;
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        observer.since = None;
                        out.refuse(
                            "diplomacyFrame",
                            "Diplomacy",
                            "Selected diplomacy frames could not be decoded.",
                        )?;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        observer.since = None;
                        out.refuse(
                            "diplomacyCampaign",
                            "Diplomacy",
                            "Selected diplomacy frames do not name this campaign.",
                        )?;
                        break;
                    }
                    observer.observe(decoded, replay.format, &mut out)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(out)
}

#[cfg(test)]
fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Event feed diplomacy request cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
