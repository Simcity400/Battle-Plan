
use super::{HolderTag, by_id, holdings, pieces, stretch_runs, tag_at, tags_of, value_at, within};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget}};
use dashboard_base::span::HourSpan;
use dashboard_query::Query;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annexation {
    pub hour: i32,
    pub read_hour: i32,
    pub annexer: Option<String>,
    pub annexed: Option<String>,
    pub civil_war: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CivilWarsParams {
    pub branch_id: String,
    #[doc = " The hour answered: nothing recorded after it is read into the reply."]
    pub hour: i32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SideExitKind {
    AnnexedBySide,
    AnnexedBySideOtherwise,
    AnnexedByOther,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SideExit {
    pub kind: SideExitKind,
    pub hour: i32,
    pub read_hour: i32,
    #[doc = " The annexer's tag at the annexation."]
    pub by: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CivilWarSide {
    #[doc = " The holder's canonical slot."]
    pub slot: u32,
    #[doc = " The holder's tags from each hour it held them."]
    pub tags: Vec<HolderTag>,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The side it broke from, by index."]
    pub parent: Option<u32>,
    #[doc = " Whether the holder held a tag before the outbreak."]
    pub existed_before: bool,
    pub exit: Option<SideExit>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CivilWarOutcome {
    pub winner: u32,
    pub hour: i32,
    pub read_hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CivilWar {
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The hours the game's civil-war predicate held for a side, within the listings: the war is"]
    #[doc = " ongoing while it holds."]
    pub at_war: Vec<HourSpan>,
    pub sides: Vec<CivilWarSide>,
    pub outcome: Option<CivilWarOutcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CivilWarsReply {
    pub branch_id: String,
    #[doc = " The hour answered, never after the branch's last hour."]
    pub hour: i32,
    #[doc = " The first hour after `hour` at which a civil war, a side, a tag, an exit, an outcome or a read stretch changes; null when none does."]
    pub next_change: Option<i32>,
    #[doc = " The hours the civil-war lists were read through `hour`; others are unknown."]
    pub known: Vec<HourSpan>,
    pub wars: Vec<CivilWar>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CivilWarsReply);

pub struct CivilWars;

impl Product for CivilWars {
    const KEY: &'static str = "civil-wars";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CivilWarsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read];
    type Reply = CivilWarsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CivilWarsParams::decl(), CivilWarsReply::decl(), SideExitKind::decl(), SideExit::decl(), CivilWarSide::decl(), CivilWarOutcome::decl(), CivilWar::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid civil war branch")
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid civil war branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let evidence = indexed(source, &query, branch, cancelled)?;
        let reply = answer_at(&evidence, params, query.timeline.last);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Listing {
    pub a: u32,
    pub b: u32,
    pub first: i32,
    pub last: i32,
}

type HolderRuns = BTreeMap<u32, Vec<(i32, i32, i64)>>;

#[derive(Debug, Default)]
pub(crate) struct Evidence {
    pub listings: Vec<Listing>,
    pub exists: HolderRuns,
    pub tags: Tags,
    pub annexations: Vec<Annexation>,
    pub known: Vec<HourSpan>,
    pub unread: Vec<(i32, String)>,
    pub unresolved: Vec<i32>,
    pub changes: Vec<i32>,
}

const KEPT: usize = 4;

#[derive(Default)]
struct Kept(Vec<(u64, Arc<Mutex<Option<Arc<Evidence>>>>)>);

fn indexed(source: &Source, query: &Query<'_>, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Evidence>> {
    let slot = {
        let state = source.state::<Kept>();
        let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = match kept.0.iter().position(|(held, _)| *held == branch) {
            Some(at) => kept.0.remove(at),
            None => (branch, Arc::new(Mutex::new(None))),
        };
        let slot = Arc::clone(&entry.1);
        kept.0.push(entry);
        if kept.0.len() > KEPT {
            kept.0.remove(0);
        }
        slot
    };
    let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(evidence) = held.as_ref() {
        return Ok(Arc::clone(evidence));
    }
    let evidence = Arc::new(evidence(query, cancelled)?);
    *held = Some(Arc::clone(&evidence));
    Ok(evidence)
}

fn unread_reason(query: &Query<'_>, run: u32, status: Status, first: i32) -> io::Result<String> {
    Ok(match status {
        Status::Complete => String::new(),
        Status::NotRecorded => "The civil-war lists were not recorded".to_owned(),
        Status::Truncated { missing } => format!("The civil-war lists were read in part ({missing} rows not read)"),
        Status::Unavailable { reason } => {
            let recording = query.replay.recording();
            let held = dashboard_query::cost::storage(|| replay_lanes::lanes::at_keys(recording, run, TableId::CountryCivilWars, first, None, replay_lanes::lanes::Keys::These(&[])))?;
            held.strings.get(reason).map_or_else(|| "The civil-war lists were not available".to_owned(), |text| String::from_utf8_lossy(text).into_owned())
        }
    })
}

const NOT_READ: &str = "The civil-war lists were not read at these hours";

fn unread_span(unread: &mut Vec<(i32, i32, String)>, first: i32, last: i32, reason: String) {
    match unread.last_mut() {
        Some((_, held_last, held)) if held_last.saturating_add(1) == first && *held == reason => *held_last = last,
        _ => unread.push((first, last, reason)),
    }
}

fn merged(runs: &mut Vec<(i32, i32, i64)>) {
    runs.sort_unstable_by_key(|run| run.0);
    let mut out: Vec<(i32, i32, i64)> = Vec::with_capacity(runs.len());
    for &run in runs.iter() {
        match out.last_mut() {
            Some(last) if last.2 == run.2 && last.1.saturating_add(1) >= run.0 => last.1 = last.1.max(run.1),
            _ => out.push(run),
        }
    }
    *runs = out;
}

pub(crate) fn evidence(query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Evidence> {
    let held = holdings(&query.holders);
    let ids = by_id(&held);
    let mut listings = Vec::new();
    let mut exists: HolderRuns = BTreeMap::new();
    let mut known = Vec::new();
    let mut unread: Vec<(i32, i32, String)> = Vec::new();
    let mut unresolved = Vec::new();
    let mut changes = BTreeSet::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let recording = query.replay.recording();
        let statuses = dashboard_query::read::statuses(recording, stretch.run, TableId::CountryCivilWars, stretch.first, stretch.last)?;
        let read = dashboard_query::cost::storage(|| replay_lanes::lanes::complete_reads(recording, stretch.run, TableId::CountryCivilWars, stretch.first, stretch.last))?;
        let mut gaps = Vec::new();
        let mut cursor = stretch.first;
        for &(first, last) in &read {
            if first > cursor {
                gaps.push((cursor, first - 1));
            }
            match known.last_mut() {
                Some(HourSpan { last_hour, .. }) if last_hour.saturating_add(1) == first => *last_hour = last,
                _ => known.push(HourSpan { first_hour: first, last_hour: last }),
            }
            cursor = last.saturating_add(1);
        }
        if cursor <= stretch.last {
            gaps.push((cursor, stretch.last));
        }
        for (gap_first, gap_last) in gaps {
            let mut at = gap_first;
            for status in statuses.iter().filter(|status| status.last >= gap_first && status.first <= gap_last) {
                let (from, to) = (status.first.max(at), status.last.min(gap_last));
                if from > to {
                    continue;
                }
                if from > at {
                    unread_span(&mut unread, at, from - 1, NOT_READ.to_owned());
                }
                let reason = if replay_recording::columnar_frames::complete(status.status) { NOT_READ.to_owned() } else { unread_reason(query, stretch.run, status.status, status.first)? };
                unread_span(&mut unread, from, to, reason);
                at = to.saturating_add(1);
            }
            if at <= gap_last {
                unread_span(&mut unread, at, gap_last, NOT_READ.to_owned());
            }
        }
        let targets = stretch_runs(query, stretch, TableId::CountryCivilWars, "target")?;
        let owned = stretch_runs(query, stretch, TableId::CountryCounterInputs, "owned_state_count")?;
        for (&key, runs) in &owned {
            let Some(holdings) = u32::try_from(key).ok().and_then(|id| ids.get(&id)) else { continue };
            for holding in holdings {
                let runs = within(Some(runs), holding.first.max(stretch.first), holding.last.min(stretch.last));
                exists.entry(holding.slot).or_default().extend(runs.into_iter().map(|(first, last, count)| (first, last, i64::from(count > 0))));
            }
        }
        for (&key, runs) in &targets {
            let country = (key >> 32) as u32;
            for (first, last, target) in within(Some(runs), stretch.first, stretch.last) {
                let (Some(own), Some(rivals)) = (ids.get(&country), u32::try_from(target).ok().and_then(|target| ids.get(&target))) else {
                    unresolved.push(first);
                    continue;
                };
                let mut resolved: Vec<(i32, i32)> = Vec::new();
                for holding in own {
                    for rival in rivals {
                        let (from, to) = (first.max(holding.first).max(rival.first), last.min(holding.last).min(rival.last));
                        if from > to {
                            continue;
                        }
                        resolved.push((from, to));
                        if holding.slot != rival.slot {
                            listings.push(Listing { a: holding.slot.min(rival.slot), b: holding.slot.max(rival.slot), first: from, last: to });
                        }
                    }
                }
                resolved.sort_unstable();
                let mut through = first.saturating_sub(1);
                for (from, to) in resolved {
                    if from > through.saturating_add(1) {
                        break;
                    }
                    through = through.max(to);
                }
                if through < last {
                    unresolved.push(through.saturating_add(1));
                }
            }
        }
    }
    listings.sort_by_key(|listing| (listing.first, listing.a, listing.b));
    unresolved.sort_unstable();
    let listed: BTreeSet<u32> = listings.iter().flat_map(|listing| [listing.a, listing.b]).collect();
    exists.retain(|slot, _| listed.contains(slot));
    for runs in exists.values_mut() {
        merged(runs);
    }
    let tags: Tags = held.iter().filter(|(slot, _)| listed.contains(slot)).map(|(&slot, held)| (slot, tags_of(held))).collect();
    let mut annexations = crate::products::feed::annexations(query)?;
    annexations.sort_by_key(|annexation| (annexation.hour, annexation.read_hour));
    for listing in &listings {
        changes.insert(listing.first);
        changes.insert(listing.last.saturating_add(1));
        for (first, last) in joined_pieces(predicate(listing, &exists)) {
            changes.insert(first);
            changes.insert(last.saturating_add(1));
        }
    }
    for entry in tags.values().flatten() {
        changes.insert(entry.hour);
    }
    for annexation in &annexations {
        let listed_at = |tag: Option<&str>| tag.is_some_and(|tag| tags.values().any(|held| tag_at(held, annexation.hour) == Some(tag)));
        if !listed_at(annexation.annexed.as_deref()) && !listed_at(annexation.annexer.as_deref()) {
            continue;
        }
        changes.insert(annexation.hour);
        changes.insert(annexation.read_hour);
        let Some(annexed) = annexation.annexed.as_deref() else { continue };
        for (slot, held) in &tags {
            if tag_at(held, annexation.hour) != Some(annexed) {
                continue;
            }
            if let Some(&(first, ..)) = exists.get(slot).into_iter().flatten().find(|run| run.2 != 0 && run.1 > annexation.hour) {
                changes.insert(first.max(annexation.hour.saturating_add(1)));
            }
        }
    }
    for span in &known {
        changes.insert(span.first_hour);
        changes.insert(span.last_hour.saturating_add(1));
    }
    for &(first, last, _) in &unread {
        changes.insert(first);
        changes.insert(last.saturating_add(1));
    }
    changes.extend(unresolved.iter().copied());
    let unread = unread.into_iter().map(|(first, _, reason)| (first, reason)).collect();
    Ok(Evidence { listings, exists, tags, annexations, known, unread, unresolved, changes: changes.into_iter().collect() })
}

pub fn answer(query: &Query<'_>, params: &CivilWarsParams, cancelled: &AtomicBool) -> io::Result<CivilWarsReply> {
    Ok(answer_at(&evidence(query, cancelled)?, params, query.timeline.last))
}

pub(crate) fn answer_at(evidence: &Evidence, params: &CivilWarsParams, timeline_last: i32) -> CivilWarsReply {
    let hour = params.hour.min(timeline_last);
    let listings: Vec<Listing> = evidence
        .listings
        .iter()
        .take_while(|listing| listing.first <= hour)
        .map(|listing| Listing { last: listing.last.min(hour), ..*listing })
        .collect();
    let exists: HolderRuns = evidence
        .exists
        .iter()
        .map(|(&slot, runs)| (slot, runs.iter().filter(|run| run.0 <= hour).map(|&(first, last, value)| (first, last.min(hour), value)).collect()))
        .collect();
    let tags: Tags = evidence.tags.iter().map(|(&slot, held)| (slot, held.iter().filter(|entry| entry.hour <= hour).cloned().collect())).collect();
    let annexations: Vec<Annexation> = evidence.annexations.iter().filter(|annexation| annexation.hour <= hour && annexation.read_hour <= hour).cloned().collect();
    let wars = episodes(listings, &tags, &exists, &annexations);
    let known = evidence
        .known
        .iter()
        .filter(|span| span.first_hour <= hour)
        .map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour.min(hour) })
        .collect();
    let mut warnings: Vec<String> = Vec::new();
    let unresolved = evidence.unresolved.partition_point(|&first| first <= hour);
    if unresolved != 0 {
        warnings.push(format!("{unresolved} refused civil-war listings: a listed country has no identity"));
    }
    for (first, reason) in &evidence.unread {
        if *first <= hour && !warnings.contains(reason) {
            warnings.push(reason.clone());
        }
    }
    let next = evidence.changes.partition_point(|&at| at <= hour);
    CivilWarsReply {
        branch_id: params.branch_id.clone(),
        hour,
        next_change: evidence.changes.get(next).copied().filter(|&at| at <= timeline_last),
        known,
        wars,
        warnings,
    }
}

type Tags = BTreeMap<u32, Vec<HolderTag>>;

pub(crate) fn episodes(mut listings: Vec<Listing>, tags: &Tags, exists: &HolderRuns, annexations: &[Annexation]) -> Vec<CivilWar> {
    listings.sort_by_key(|listing| (listing.first, listing.a, listing.b));
    let mut groups: Vec<Vec<Listing>> = Vec::new();
    for listing in listings {
        let joined: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.iter().any(|other| {
                    (other.a == listing.a || other.a == listing.b || other.b == listing.a || other.b == listing.b)
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
    let mut wars: Vec<CivilWar> = groups.into_iter().map(|group| war(group, tags, exists, annexations)).collect();
    wars.sort_by_key(|war| (war.first_hour, war.id.clone()));
    wars
}

fn joined_pieces(pieces: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(pieces.len());
    for (first, last) in pieces {
        match out.last_mut() {
            Some(held) if held.1.saturating_add(1) >= first => held.1 = held.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

fn predicate(listing: &Listing, exists: &HolderRuns) -> Vec<(i32, i32)> {
    let none = Vec::new();
    let a = exists.get(&listing.a).unwrap_or(&none);
    let b = exists.get(&listing.b).unwrap_or(&none);
    pieces(listing.first, listing.last, &[a, b])
        .into_iter()
        .filter(|&(from, _)| value_at(a, from) == Some(1) || value_at(b, from) == Some(1))
        .collect()
}

fn war(mut group: Vec<Listing>, tags: &Tags, exists: &HolderRuns, annexations: &[Annexation]) -> CivilWar {
    group.sort_by_key(|listing| (listing.first, listing.a, listing.b));
    let first = group.iter().map(|listing| listing.first).min().unwrap_or(0);
    let last = group.iter().map(|listing| listing.last).max().unwrap_or(0);
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
    let index: BTreeMap<u32, u32> = order.iter().enumerate().map(|(index, holder)| (*holder, index as u32)).collect();
    let tags_of = |holder: u32| tags.get(&holder).map(Vec::as_slice).unwrap_or(&[]);
    let existed = |holder: u32| tags_of(holder).first().is_some_and(|entry| entry.hour < first);
    let mut sides: Vec<CivilWarSide> = order
        .iter()
        .map(|&holder| {
            let (side_first, side_last) = spans[&holder];
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
                    (spans[&other].0 < side_first || spans[&other].0 == side_first && existed(other) && !existed(holder)).then_some((spans[&other].0, other))
                })
                .min()
                .map(|(_, other)| index[&other]);
            CivilWarSide { slot: holder, tags: tags_of(holder).to_vec(), first_hour: side_first, last_hour: side_last, parent, existed_before: existed(holder), exit: None }
        })
        .collect();
    let side_at = |tag: &str, hour: i32| order.iter().position(|&holder| tag_at(tags_of(holder), hour) == Some(tag));
    let ended = sides.iter().map(|side| side.last_hour).max().unwrap_or(i32::MIN).saturating_add(1);
    let existed_after = |holder: u32, hour: i32| exists.get(&holder).is_some_and(|runs| runs.iter().any(|run| run.2 != 0 && run.1 > hour && run.0 <= ended));
    let mut annexations: Vec<&Annexation> = annexations.iter().collect();
    annexations.sort_by_key(|annexation| (annexation.hour, annexation.read_hour));
    for annexation in annexations {
        let annexed = annexation.annexed.as_deref().and_then(|tag| side_at(tag, annexation.hour));
        let by = annexation.annexer.as_deref().and_then(|tag| side_at(tag, annexation.hour)).filter(|&by| sides[by].first_hour <= annexation.hour);
        if let Some(by) = by
            && annexed != Some(by)
            && annexation.hour <= ended
            && sides[by].exit.as_ref().is_some_and(|exit| exit.hour < annexation.hour)
        {
            sides[by].exit = None;
        }
        let Some(annexed) = annexed else { continue };
        let side = &sides[annexed];
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
        let restored = existed_after(order[annexed], annexation.hour);
        sides[annexed].exit = (!restored).then(|| SideExit { kind, hour: annexation.hour, read_hour: annexation.read_hour, by: annexation.annexer.clone() });
    }
    let by_side = |side: &CivilWarSide| side.exit.as_ref().filter(|exit| exit.kind != SideExitKind::AnnexedByOther).map(|exit| (exit.hour, exit.read_hour));
    let proven_at = sides.iter().filter_map(by_side).max();
    let remaining: Vec<usize> = (0..sides.len()).filter(|&side| by_side(&sides[side]).is_none()).collect();
    let outcome = match (remaining.as_slice(), proven_at) {
        ([winner], Some((hour, read_hour))) if sides[*winner].exit.is_none() => Some(CivilWarOutcome { winner: *winner as u32, hour, read_hour }),
        _ => None,
    };
    let mut at_war: Vec<(i32, i32)> = group.iter().flat_map(|listing| predicate(listing, exists)).collect();
    at_war.sort_unstable();
    let mut joined: Vec<HourSpan> = Vec::new();
    for (from, to) in at_war {
        match joined.last_mut() {
            Some(span) if from <= span.last_hour.saturating_add(1) => span.last_hour = span.last_hour.max(to),
            _ => joined.push(HourSpan { first_hour: from, last_hour: to }),
        }
    }
    CivilWar { id: format!("{first}:{}", order.first().copied().unwrap_or(0)), first_hour: first, last_hour: last, at_war: joined, sides, outcome }
}
