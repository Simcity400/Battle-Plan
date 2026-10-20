//! Civil wars from the prepared political state: each holder's stored
//! civil-war rivals over time ([`history::Field::CivilWarRival`]), grouped
//! into app-owned episodes, with outcomes only where annexation evidence
//! proves one.
//!
//! An episode is the holders joined by rival listings that overlap or meet
//! in time; listings that are apart in time are separate episodes, and no
//! two-coalition split is forced on three or more sides. A side lasts from
//! its first listing to its last. A side "broke from" the earliest-listed
//! rival that was already a side when it was first listed; a side whose
//! holder was read before the outbreak existed before it. Lists are kept as
//! the game stores them: one country's list is never assumed to answer
//! another's.
//!
//! Each episode carries the hours the game's own civil-war predicate held for
//! one of its sides ([`history::Field::CivilWar`]), and the reply the hours
//! the lists were read, so a viewer resolves at each hour whether the war is
//! ongoing: a stale listing naming a country that no longer exists keeps a
//! side's span but not the war, and an hour not read is unknown.
//!
//! The winner is shown only when every other side was annexed by a side
//! through a civil war's resolution (the route recorded with the annexation).
//! An annexation by a country that was not a side, a later split, a gap or a
//! capitulation decides nothing. Annexations still name how a side left.

use super::history::{self, Field, HolderTag, Metadata, Part, tag_at};
use dashboard_base::{
    invalid,
    prepared::series::{self, Reader, Run},
    products::{Product, country_groups::HourSpan},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub const QUERY: &str = "civil-wars";

/// One recorded annexation, its countries as tagged at the native hour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annexation {
    pub hour: i32,
    pub read_hour: i32,
    pub annexer: Option<String>,
    pub annexed: Option<String>,
    /// Whether it ran within a civil war's resolution.
    pub civil_war: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CivilWarsParams {
    pub branch_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum SideExitKind {
    /// Annexed by another side through a civil war's resolution.
    AnnexedBySide,
    /// Annexed by another side otherwise.
    AnnexedBySideOtherwise,
    /// Annexed by a country that was not a side.
    AnnexedByOther,
}

/// How a side left, as an annexation recorded it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SideExit {
    pub kind: SideExitKind,
    pub hour: i32,
    pub read_hour: i32,
    /// The annexer's tag at the annexation.
    pub by: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CivilWarSide {
    /// The holder's tags from each hour it was read with them.
    pub tags: Vec<HolderTag>,
    pub first_hour: i32,
    pub last_hour: i32,
    /// The side it broke from, by index.
    pub parent: Option<u32>,
    /// Whether the holder was read before the outbreak.
    pub existed_before: bool,
    pub exit: Option<SideExit>,
}

/// A proven outcome: the side left after every other was annexed by a side
/// through a civil war's resolution, dated by the last such annexation.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CivilWarOutcome {
    pub winner: u32,
    pub hour: i32,
    pub read_hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CivilWar {
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    /// The hours the game's civil-war predicate held for a side, within the
    /// listings: the war is ongoing while it holds.
    pub at_war: Vec<HourSpan>,
    pub sides: Vec<CivilWarSide>,
    pub outcome: Option<CivilWarOutcome>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CivilWarsReply {
    pub branch_id: String,
    pub last_hour: Option<i32>,
    /// The hours the civil-war lists were read; others are unknown.
    pub known: Vec<HourSpan>,
    pub wars: Vec<CivilWar>,
    pub warnings: Vec<String>,
}

pub struct CivilWars;
impl Product for CivilWars {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = false;
    type Params = CivilWarsParams;
    type Reply = CivilWarsReply;
    fn types() -> Vec<String> {
        vec![
            CivilWarsParams::decl(),
            SideExitKind::decl(),
            SideExit::decl(),
            CivilWarSide::decl(),
            CivilWarOutcome::decl(),
            CivilWar::decl(),
            CivilWarsReply::decl(),
        ]
    }
    fn validate(p: &CivilWarsParams) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid civil war branch")
    }
}

struct Listing {
    a: u32,
    b: u32,
    first: i32,
    last: i32,
}

/// Every civil war through `through`, the last prepared hour, with the
/// branch's recorded annexations as outcome evidence.
pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &CivilWarsParams,
    through: Option<i32>,
    annexations: &[Annexation],
    cancelled: &AtomicBool,
) -> io::Result<CivilWarsReply> {
    let branch: u64 = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid civil war branch"))?;
    let mut reply = CivilWarsReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        known: Vec::new(),
        wars: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        return Ok(reply);
    };
    let Some((_, metadata)) =
        series::read_catalogue::<Metadata>(store, key, history::FAMILY, branch)?
    else {
        return Ok(reply);
    };
    let mut reader = Reader::new(store, key, history::values_section(branch), cancelled)?;
    reply.known = history::known(&mut reader, &metadata, Part::CivilWars, i32::MIN, through)?;
    let mut listings = Vec::new();
    let mut at_war = BTreeMap::new();
    for (&id, (holder, field)) in &metadata.fields {
        if cancelled.load(Ordering::Acquire) {
            return Err(invalid("Civil war request cancelled"));
        }
        let rival = match field {
            Field::CivilWarRival(rival) => rival,
            Field::CivilWar => {
                at_war.insert(*holder, reader.runs(id, i32::MIN, through)?);
                continue;
            }
            _ => continue,
        };
        for run in reader.runs(id, i32::MIN, through)? {
            listings.push(Listing {
                a: *holder.min(rival),
                b: *holder.max(rival),
                first: run.first_hour,
                last: run.last_hour,
            });
        }
    }
    reply.wars = episodes(listings, &metadata, &at_war, annexations);
    for (reason, count) in &metadata.refusals {
        if reason.contains("civil-war") {
            reply
                .warnings
                .push(format!("{count} refused civil-war reads: {reason}"));
        }
    }
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// Each holder's runs of the game's civil-war predicate.
type AtWar = BTreeMap<u32, Vec<Run>>;

fn episodes(
    mut listings: Vec<Listing>,
    metadata: &Metadata,
    at_war: &AtWar,
    annexations: &[Annexation],
) -> Vec<CivilWar> {
    listings.sort_by_key(|listing| (listing.first, listing.a, listing.b));
    // Listings joined by a shared holder while overlapping or meeting.
    let mut groups: Vec<Vec<Listing>> = Vec::new();
    for listing in listings {
        let joined: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.iter().any(|other| {
                    (other.a == listing.a
                        || other.a == listing.b
                        || other.b == listing.a
                        || other.b == listing.b)
                        && other.first <= listing.last.saturating_add(1)
                        && listing.first <= other.last.saturating_add(1)
                })
            })
            .map(|(index, _)| index)
            .collect();
        let mut merged = vec![listing];
        for index in joined.into_iter().rev() {
            merged.extend(groups.remove(index));
        }
        groups.push(merged);
    }
    let mut wars: Vec<CivilWar> = groups
        .into_iter()
        .map(|group| war(group, metadata, at_war, annexations))
        .collect();
    wars.sort_by_key(|war| (war.first_hour, war.id.clone()));
    wars
}

fn war(
    mut group: Vec<Listing>,
    metadata: &Metadata,
    at_war: &AtWar,
    annexations: &[Annexation],
) -> CivilWar {
    group.sort_by_key(|listing| (listing.first, listing.a, listing.b));
    let first = group.iter().map(|listing| listing.first).min().unwrap_or(0);
    let last = group.iter().map(|listing| listing.last).max().unwrap_or(0);
    // Sides in the order they were first listed, holders breaking ties.
    let mut spans = BTreeMap::<u32, (i32, i32)>::new();
    for listing in &group {
        for holder in [listing.a, listing.b] {
            let span = spans.entry(holder).or_insert((listing.first, listing.last));
            span.0 = span.0.min(listing.first);
            span.1 = span.1.max(listing.last);
        }
    }
    let mut order: Vec<u32> = spans.keys().copied().collect();
    order.sort_by_key(|holder| (spans[holder].0, *holder));
    let index: BTreeMap<u32, u32> = order
        .iter()
        .enumerate()
        .map(|(index, holder)| (*holder, index as u32))
        .collect();
    let tags = |holder: u32| metadata.tags.get(&holder).map(Vec::as_slice).unwrap_or(&[]);
    let existed = |holder: u32| tags(holder).first().is_some_and(|(hour, _)| *hour < first);
    let mut sides: Vec<CivilWarSide> = order
        .iter()
        .map(|&holder| {
            let (side_first, side_last) = spans[&holder];
            // The earliest-listed rival already a side when this one was.
            let parent = group
                .iter()
                .filter_map(|listing| {
                    let other = if listing.a == holder {
                        listing.b
                    } else if listing.b == holder {
                        listing.a
                    } else {
                        return None;
                    };
                    // Listed first, or listed together while it alone existed before.
                    (spans[&other].0 < side_first
                        || spans[&other].0 == side_first && existed(other) && !existed(holder))
                        .then_some((spans[&other].0, other))
                })
                .min()
                .map(|(_, other)| index[&other]);
            CivilWarSide {
                tags: tags(holder)
                    .iter()
                    .map(|(hour, tag)| HolderTag {
                        hour: *hour,
                        tag: tag.clone(),
                    })
                    .collect(),
                first_hour: side_first,
                last_hour: side_last,
                parent,
                existed_before: existed(holder),
                exit: None,
            }
        })
        .collect();
    let side_at = |tag: &str, hour: i32| {
        order
            .iter()
            .position(|&holder| tag_at(tags(holder), hour) == Some(tag))
    };
    let mut proven = BTreeSet::new();
    let mut proven_at = None;
    for annexation in annexations {
        let Some(annexed) = annexation
            .annexed
            .as_deref()
            .and_then(|tag| side_at(tag, annexation.hour))
        else {
            continue;
        };
        let side = &sides[annexed];
        // The annexation ends the side's listings, or follows them at once.
        if annexation.hour < side.first_hour || annexation.hour > side.last_hour.saturating_add(1) {
            continue;
        }
        let by = annexation
            .annexer
            .as_deref()
            .and_then(|tag| side_at(tag, annexation.hour));
        let kind = match by {
            Some(by) if by != annexed && annexation.civil_war => {
                proven.insert(annexed);
                proven_at = proven_at.max(Some((annexation.hour, annexation.read_hour)));
                SideExitKind::AnnexedBySide
            }
            Some(by) if by != annexed => SideExitKind::AnnexedBySideOtherwise,
            Some(_) => continue,
            None => SideExitKind::AnnexedByOther,
        };
        sides[annexed].exit = Some(SideExit {
            kind,
            hour: annexation.hour,
            read_hour: annexation.read_hour,
            by: annexation.annexer.clone(),
        });
    }
    let remaining: Vec<usize> = (0..sides.len()).filter(|side| !proven.contains(side)).collect();
    let outcome = match (remaining.as_slice(), proven_at) {
        ([winner], Some((hour, read_hour))) if sides[*winner].exit.is_none() => {
            Some(CivilWarOutcome {
                winner: *winner as u32,
                hour,
                read_hour,
            })
        }
        _ => None,
    };
    CivilWar {
        id: format!("{first}:{}", order.first().copied().unwrap_or(0)),
        first_hour: first,
        last_hour: last,
        at_war: held(order.iter().filter_map(|holder| at_war.get(holder)).flatten(), first, last),
        sides,
        outcome,
    }
}

/// The hours any of `runs` covers within `first..=last`, joined where they
/// overlap or meet.
fn held<'a>(runs: impl Iterator<Item = &'a Run>, first: i32, last: i32) -> Vec<HourSpan> {
    let mut spans: Vec<(i32, i32)> = runs
        .map(|run| (run.first_hour.max(first), run.last_hour.min(last)))
        .filter(|(from, to)| from <= to)
        .collect();
    spans.sort_unstable();
    let mut joined: Vec<HourSpan> = Vec::new();
    for (from, to) in spans {
        if let Some(span) = joined
            .last_mut()
            .filter(|span| from <= span.last_hour.saturating_add(1))
        {
            span.last_hour = span.last_hour.max(to);
        } else {
            joined.push(HourSpan {
                first_hour: from,
                last_hour: to,
            });
        }
    }
    joined
}

#[cfg(test)]
mod tests;
