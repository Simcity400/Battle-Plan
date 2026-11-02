//! Civil wars from the prepared political state: each holder's stored
//! civil-war rivals over time ([`history::Field::CivilWarRival`]), grouped
//! into app-owned episodes, with outcomes only where annexation evidence
//! proves one.
//!
//! An episode is the holders joined by rival listings that overlap or meet
//! in time; listings that are apart in time are separate episodes, and no
//! two-coalition split is forced on three or more sides. A side lasts from
//! its first listing to its last. A side "broke from" the earliest-listed of
//! its rivals at its first listing that was already a side then; a later
//! rival never changes it. A side whose holder was read before the outbreak
//! existed before it. Lists are kept as the game stores them: one country's
//! list is never assumed to answer another's.
//!
//! Each episode carries the hours the game's own civil-war predicate held for
//! one of its sides ([`history::Field::CivilWar`]), and the reply the hours
//! the lists were read, so a viewer resolves at each hour whether the war is
//! ongoing: a stale listing naming a country that no longer exists keeps a
//! side's span but not the war, and an hour not read is unknown.
//!
//! The winner is shown only when every other side was annexed by a side,
//! through a civil war's resolution or another route such as a peace
//! conference. An annexation by a country that was not a side yet, a later
//! split, a gap or a capitulation decides nothing. A side's last annexation
//! decides; one that later annexes, or is recorded existing after it within
//! the episode ([`history::Field::Exists`]), was restored. Its listings alone
//! never undo an annexation, and an hour whose existence is unknown says
//! nothing. Annexations still name how a side left, and by which route.

use super::history::{
    self, HolderTag, Part, Run,
    emit::{CIVIL_WAR, CIVIL_WAR_RIVAL, EXISTS},
    tag_at,
};
use dashboard_base::{
    invalid,
    products::{Product, country_groups::HourSpan},
};
use dashboard_series::{PageRead, Snapshot, family::Directory};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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

/// A proven outcome: the side left after every other was annexed by a side,
/// dated by the last such annexation.
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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

/// The reply before the family's first publication on the branch: no hour is prepared yet.
pub fn unpublished(params: &CivilWarsParams, through: Option<i32>) -> io::Result<CivilWarsReply> {
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid civil war branch"))?;
    Ok(CivilWarsReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        known: Vec::new(),
        wars: Vec::new(),
        warnings: Vec::new(),
    })
}

/// Every civil war through `through`, the last prepared hour, from the publication `snapshot`
/// pins, with the branch's recorded annexations as outcome evidence.
pub fn query<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &CivilWarsParams,
    through: Option<i32>,
    annexations: &[Annexation],
    cancelled: &AtomicBool,
) -> io::Result<CivilWarsReply> {
    let mut reply = unpublished(params, through)?;
    let Some(through) = through else {
        return Ok(reply);
    };
    reply.known = history::known(snapshot, directory, Part::CivilWars, i32::MIN, through)?;
    let tags = history::tags(snapshot, directory)?;
    let mut listings = Vec::new();
    let mut at_war = BTreeMap::new();
    let mut exists = BTreeMap::new();
    for (holder, actor) in history::holders(directory) {
        if cancelled.load(Ordering::Acquire) {
            return Err(invalid("Civil war request cancelled"));
        }
        if let Some(key) = directory.key(actor, CIVIL_WAR, None) {
            at_war.insert(holder, history::runs(snapshot, key, i32::MIN, through)?);
        }
        if let Some(key) = directory.key(actor, EXISTS, None) {
            exists.insert(holder, history::runs(snapshot, key, i32::MIN, through)?);
        }
        for &(rival, key) in directory.dimensions(actor, CIVIL_WAR_RIVAL) {
            let rival = u32::try_from(rival).map_err(|_| invalid("A civil-war rival is not a holder"))?;
            for run in history::runs(snapshot, key, i32::MIN, through)? {
                listings.push(Listing {
                    a: holder.min(rival),
                    b: holder.max(rival),
                    first: run.first_hour,
                    last: run.last_hour,
                });
            }
        }
    }
    reply.wars = episodes(listings, &tags, &at_war, &exists, annexations);
    for (reason, count) in &history::refusals(snapshot, directory, through)? {
        if reason.contains("civil-war") {
            reply
                .warnings
                .push(format!("{count} refused civil-war reads: {reason}"));
        }
    }
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// Each holder's runs of one field: the game's civil-war predicate, or its
/// existence check.
type HolderRuns = BTreeMap<u32, Vec<Run>>;

/// Each holder's tags from each hour it was first read with them.
type Tags = BTreeMap<u32, Vec<(i32, String)>>;

fn episodes(
    mut listings: Vec<Listing>,
    tags: &Tags,
    at_war: &HolderRuns,
    exists: &HolderRuns,
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
        .map(|group| war(group, tags, at_war, exists, annexations))
        .collect();
    wars.sort_by_key(|war| (war.first_hour, war.id.clone()));
    wars
}

fn war(
    mut group: Vec<Listing>,
    tags: &Tags,
    at_war: &HolderRuns,
    exists: &HolderRuns,
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
    let tags = |holder: u32| tags.get(&holder).map(Vec::as_slice).unwrap_or(&[]);
    let existed = |holder: u32| tags(holder).first().is_some_and(|(hour, _)| *hour < first);
    let mut sides: Vec<CivilWarSide> = order
        .iter()
        .map(|&holder| {
            let (side_first, side_last) = spans[&holder];
            // The earliest-listed rival, among those at its first listing, already a side then.
            let parent = group
                .iter()
                .filter(|listing| listing.first == side_first)
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
    // The hour after the episode's last listing; a restoration later than it is another episode's.
    let ended = sides
        .iter()
        .map(|side| side.last_hour)
        .max()
        .unwrap_or(i32::MIN)
        .saturating_add(1);
    // Only hours after its own: an annexation may follow its hour's sample.
    let existed_after = |holder: u32, hour: i32| {
        exists.get(&holder).is_some_and(|runs| {
            runs.iter()
                .any(|run| run.value != 0 && run.last_hour > hour && run.first_hour <= ended)
        })
    };
    let mut annexations: Vec<&Annexation> = annexations.iter().collect();
    annexations.sort_by_key(|annexation| (annexation.hour, annexation.read_hour));
    for annexation in annexations {
        let annexed = annexation
            .annexed
            .as_deref()
            .and_then(|tag| side_at(tag, annexation.hour));
        // A country annexes as a side only once it is listed.
        let by = annexation
            .annexer
            .as_deref()
            .and_then(|tag| side_at(tag, annexation.hour))
            .filter(|&by| sides[by].first_hour <= annexation.hour);
        // Annexing anyone after its own annexation, the side was restored: that exit was not last.
        if let Some(by) = by
            && annexed != Some(by)
            && annexation.hour <= ended
            && sides[by]
                .exit
                .as_ref()
                .is_some_and(|exit| exit.hour < annexation.hour)
        {
            sides[by].exit = None;
        }
        let Some(annexed) = annexed else {
            continue;
        };
        let side = &sides[annexed];
        // The annexation falls within the side's listings, or follows them at once.
        if annexation.hour < side.first_hour || annexation.hour > side.last_hour.saturating_add(1) {
            continue;
        }
        let kind = match by {
            Some(by) if by != annexed => {
                if annexation.civil_war {
                    SideExitKind::AnnexedBySide
                } else {
                    SideExitKind::AnnexedBySideOtherwise
                }
            }
            Some(_) => continue,
            None => SideExitKind::AnnexedByOther,
        };
        // A later annexation supersedes an earlier one, and existing after it undoes it: the side was restored.
        let restored = existed_after(order[annexed], annexation.hour);
        sides[annexed].exit = (!restored).then(|| SideExit {
            kind,
            hour: annexation.hour,
            read_hour: annexation.read_hour,
            by: annexation.annexer.clone(),
        });
    }
    let by_side = |side: &CivilWarSide| {
        side.exit
            .as_ref()
            .filter(|exit| exit.kind != SideExitKind::AnnexedByOther)
            .map(|exit| (exit.hour, exit.read_hour))
    };
    let proven_at = sides.iter().filter_map(by_side).max();
    let remaining: Vec<usize> = (0..sides.len())
        .filter(|&side| by_side(&sides[side]).is_none())
        .collect();
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

pub mod served;
#[cfg(test)]
mod tests;
