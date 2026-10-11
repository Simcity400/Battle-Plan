//! Feed-only projection of shared group runs and recorded diplomacy facts.
//! Group boundaries use the Factions, Wars and smart-group runs.

use crate::{
    metadata::{LinkTags, branch_bounds, slot_read, validate_campaign},
    products::country_groups::{self, HourSpan},
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub(super) struct GroupMoment {
    pub kind: &'static str,
    pub hour: i32,
    pub read_hour: i32,
    pub name_hour: i32,
    pub exact: bool,
    /// For aggregated wars/peace the focal country comes first, then counterparties.
    pub tags: Vec<String>,
    pub faction: Option<u64>,
}

pub(super) struct Unavailable {
    pub key: &'static str,
    pub label: &'static str,
    pub reason: String,
    pub count: u64,
}

pub(super) struct Scan {
    pub moments: Vec<GroupMoment>,
    pub observed: BTreeSet<TableId>,
    pub unavailable: Vec<Unavailable>,
}

impl Scan {
    fn refuse(
        &mut self,
        key: &'static str,
        label: &'static str,
        reason: &'static str,
    ) -> io::Result<()> {
        if let Some(entry) = self.unavailable.iter_mut().find(|entry| entry.key == key) {
            entry.count = entry.count.checked_add(1).ok_or_else(|| {
                io::Error::other(format!("Event feed {key} unavailable count exceeds u64"))
            })?;
        } else {
            self.unavailable.push(Unavailable {
                key,
                label,
                reason: reason.to_owned(),
                count: 1,
            });
        }
        Ok(())
    }
}

fn missing(spans: &[HourSpan], hour: i32) -> bool {
    spans
        .iter()
        .any(|span| span.first_hour <= hour && hour <= span.last_hour)
}

fn any_observed(spans: &[HourSpan], first: i32, last: i32) -> bool {
    let mut at = first;
    for span in spans {
        if at < span.first_hour {
            return at <= last;
        }
        if at <= span.last_hour {
            let Some(next) = span.last_hour.checked_add(1) else {
                return false;
            };
            at = next;
        }
    }
    at <= last
}

fn switched(groups: &country_groups::Scan, tag: &str, hour: i32) -> bool {
    groups
        .identity_changes
        .iter()
        .any(|change| change.hour == hour && (change.from == tag || change.to == tag))
}

/// Boundaries come from the exact runs the topic widgets use. A boundary next
/// to an unobserved hour is counted, rather than silently converted into a join
/// or peace. Initial memberships and final campaign truncation are not events.
fn project_groups(
    groups: &country_groups::Scan,
    first: i32,
    last: i32,
    out: &mut Scan,
) -> io::Result<()> {
    if any_observed(&groups.unobserved.factions, first, last) {
        out.observed.insert(TableId::FactionMembers);
    }
    if any_observed(&groups.unobserved.wars, first, last) {
        out.observed.insert(TableId::WarRelations);
    }
    for (&id, faction) in &groups.factions {
        for run in faction.members.iter() {
            for (kind, hour, name_hour, neighbor) in [
                (
                    "factionJoined",
                    Some(run.first_hour),
                    run.first_hour,
                    run.first_hour.checked_sub(1),
                ),
                (
                    "factionLeft",
                    run.last_hour.checked_add(1),
                    run.last_hour,
                    run.last_hour.checked_add(1),
                ),
            ] {
                let (Some(hour), Some(neighbor)) = (hour, neighbor) else {
                    continue;
                };
                if hour <= first || hour > last || switched(groups, &run.tag, hour) {
                    continue;
                }
                if missing(&groups.unobserved.factions, neighbor)
                    || missing(&faction.incomplete, neighbor)
                {
                    out.refuse("factionBoundary", "Faction changes",
                        "Faction membership boundaries adjoin an unobserved or unresolved roster; the changes are not established.")?;
                    continue;
                }
                out.moments.push(GroupMoment {
                    kind,
                    hour,
                    read_hour: hour,
                    name_hour,
                    exact: false,
                    tags: vec![run.tag.clone()],
                    faction: Some(id),
                });
            }
        }
    }
    // Removing a war relation clears both status links. A complete following
    // observation only proves the countries are no longer at war; it does not
    // identify a peace treaty, annexation or release cause.
    let mut peace = BTreeMap::<(i32, String), BTreeSet<String>>::new();
    for run in groups.wars.iter() {
        let Some(hour) = run.last_hour.checked_add(1).filter(|hour| *hour <= last) else {
            continue;
        };
        if switched(groups, &run.first, hour) || switched(groups, &run.second, hour) {
            continue;
        }
        if missing(&groups.unobserved.wars, hour) {
            out.refuse("warBoundary", "Wars and peace",
                "War-pair boundaries adjoin an unobserved or unresolved war roster; their removal dates are not established.")?;
            continue;
        }
        for (tag, other) in [(&run.first, &run.second), (&run.second, &run.first)] {
            peace
                .entry((hour, tag.clone()))
                .or_default()
                .insert(other.clone());
        }
    }
    for ((hour, tag), others) in peace {
        out.moments.push(GroupMoment {
            kind: "peace",
            hour,
            read_hour: hour,
            name_hour: hour,
            exact: false,
            tags: std::iter::once(tag).chain(others).collect(),
            faction: None,
        });
    }
    // The relation start is shared by declarations and joins. Group's exact
    // relation start is recorded; counterparties aggregate by country/hour.
    let mut starts = BTreeMap::<(i32, i32, String), BTreeSet<String>>::new();
    for start in groups.war_starts.iter() {
        let read_hour = groups.war_start_reads.get(start).copied();
        let Some(read_hour) = read_hour else {
            out.refuse(
                "warReadHour",
                "Wars and peace",
                "War starts have no recorded supplying observation.",
            )?;
            continue;
        };
        starts
            .entry((start.hour, read_hour, start.attacking.clone()))
            .or_default()
            .insert(start.defending.clone());
    }
    for ((hour, read_hour, tag), others) in starts {
        out.moments.push(GroupMoment {
            kind: "warStarted",
            hour,
            read_hour,
            name_hour: hour,
            exact: true,
            tags: std::iter::once(tag).chain(others).collect(),
            faction: None,
        });
    }
    if groups.capitulations_observed {
        out.observed.insert(TableId::Capitulation);
    }
    for event in groups.capitulations.iter() {
        if let Some(&read_hour) = groups.capitulation_reads.get(event) {
            out.moments.push(GroupMoment {
                kind: "capitulated",
                hour: event.hour,
                read_hour,
                name_hour: event.hour,
                exact: true,
                tags: vec![event.tag.clone()],
                faction: None,
            });
        } else {
            out.refuse(
                "capitulationReadHour",
                "Capitulations",
                "Capitulations have no recorded supplying observation.",
            )?;
        }
    }
    Ok(())
}

/// Only feed consumers decode owner-sliced guarantees and country existence.
/// Faction rows retain their shared id and naming hour; left rows name the
/// previous observation, including a faction that dissolved completely.
/// There is one selected-table decoder/forward pass per block, never seek/hour.
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
    project_groups(groups, bounds.0, bounds.1, &mut out)?;
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
                key,
                label,
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
    let mut tags = LinkTags::default();
    let mut named_since = BTreeMap::<u32, (String, i32)>::new();
    let mut since = None;
    let mut first_observed = None;
    let mut previous_hour: Option<i32> = None;
    let mut existence = BTreeMap::<u32, (u32, String, i32, i32)>::new();
    let mut guarantees = BTreeSet::new();

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
                    since = None;
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
                        since = None;
                        out.refuse(
                            "diplomacyFrame",
                            "Diplomacy",
                            "Selected diplomacy frames could not be decoded.",
                        )?;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        since = None;
                        out.refuse(
                            "diplomacyCampaign",
                            "Diplomacy",
                            "Selected diplomacy frames do not name this campaign.",
                        )?;
                        break;
                    }
                    let hour = frame.hour;
                    let first_observed = *first_observed.get_or_insert(hour);
                    if previous_hour.is_some_and(|previous| previous.checked_add(1) != Some(hour)) {
                        since = None;
                    }
                    previous_hour = Some(hour);
                    let Ok(rebuilt) = tags.update(decoded) else {
                        since = None;
                        out.refuse(
                            "diplomacyIdentity",
                            "Diplomacy",
                            "Country identities are unresolved.",
                        )?;
                        continue;
                    };
                    if rebuilt {
                        named_since.retain(|raw, (tag, _)| tags.get(*raw) == Some(tag));
                        for (raw, tag) in tags.iter() {
                            named_since
                                .entry(*raw)
                                .or_insert_with(|| (tag.clone(), hour));
                        }
                    }
                    let continuous = *since.get_or_insert(hour);
                    // Countries includes every non-null canonical country slot, with no
                    // owned-state filter. A country with no states can therefore remain
                    // recorded after annexation; presence is not existence.
                    // CountryCounterInputs supplies the unfiltered owned-state roster count.
                    // A positive-to-zero transition means ceased to exist by this supplying
                    // read; the cause and exact transition hour are absent.
                    if slot_read(&decoded.metrics, TableId::CountryCounterInputs, hour) {
                        if let Ok(rows) = decoded.complete::<tables::CountryCounterInputs>() {
                            out.observed.insert(TableId::CountryCounterInputs);
                            let countries = decoded.complete::<tables::Countries>()?;

                            for row in rows {
                                let slot = countries
                                    .binary_search_by_key(&row.country, |country| country.id)
                                    .ok()
                                    .map(|index| countries[index].canonical_slot);
                                let (Some(tag), Some(slot)) = (tags.get(row.country), slot) else {
                                    out.refuse("existenceIdentity", "Country existence", "Owned-state-count observations have unresolved or changed country identities; the disappeared countries are not established.")?;
                                    continue;
                                };
                                if row.owned_state_count < 0 {
                                    out.refuse(
                                        "existenceCount",
                                        "Country existence",
                                        "Countries have negative recorded owned-state counts.",
                                    )?;
                                    existence.remove(&slot);
                                    continue;
                                }
                                if let Some((id, old_tag, count, read_hour)) = existence.get(&slot)
                                    && *count > 0
                                    && row.owned_state_count == 0
                                {
                                    if *id == row.country && old_tag == tag {
                                        out.moments.push(GroupMoment {
                                            kind: "ceasedToExist",
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
                                match existence.get_mut(&slot) {
                                    Some((id, old_tag, count, read_hour))
                                        if *id == row.country
                                            && old_tag.as_str() == tag.as_str() =>
                                    {
                                        *count = row.owned_state_count;
                                        *read_hour = hour;
                                    }
                                    _ => {
                                        existence.insert(
                                            slot,
                                            (row.country, tag.clone(), row.owned_state_count, hour),
                                        );
                                    }
                                }
                            }
                        } else {
                            out.refuse("existenceRead", "Country existence", "Owned-state-count reads are incomplete; transitions through them are unknown.")?;
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
                        for read in reads[first..last].iter().filter(|read| {
                            read.sample_hour == hour && tags.get(read.owner).is_some()
                        }) {
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
                                if !guarantees.insert((
                                    row.first_country,
                                    row.second_country,
                                    row.start_hour,
                                )) {
                                    continue;
                                }
                                let (Some(first), Some(second)) =
                                    (tags.get(row.first_country), tags.get(row.second_country))
                                else {
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
                                        named_since.get(&raw).is_some_and(|(_, from)| {
                                            continuous.max(*from) <= row.start_hour
                                        })
                                    });
                                out.moments.push(GroupMoment {
                                    kind: "guaranteeGranted",
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
                }
            }
            Ok(())
        },
    )?;
    Ok(out)
}

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
