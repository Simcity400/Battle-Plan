
use super::{HolderTag, by_id, holdings, pieces, stretch_runs, tag_at, tags_of, value_at, within};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget}};
use dashboard_base::span::HourSpan;
use dashboard_query::Query;
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::AtomicBool,
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
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid civil war branch"))?)?;
        let reply = answer(&query, params, cancelled)?;
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

struct Next(Option<i32>, i32);

impl Next {
    fn see(&mut self, at: i32) {
        if at > self.1 && self.0.is_none_or(|next| at < next) {
            self.0 = Some(at);
        }
    }

    fn span(&mut self, first: i32, last: i32) {
        self.see(first);
        self.see(last.saturating_add(1));
    }
}

fn cut_war(war: &CivilWar, hour: i32) -> CivilWar {
    let mut remap: Vec<Option<u32>> = vec![None; war.sides.len()];
    let mut sides = Vec::new();
    for (old_idx, side) in war.sides.iter().enumerate() {
        if side.first_hour > hour {
            continue;
        }
        let new_idx = sides.len() as u32;
        remap[old_idx] = Some(new_idx);
        sides.push(CivilWarSide {
            slot: side.slot,
            tags: side.tags.iter().filter(|entry| entry.hour <= hour).cloned().collect(),
            first_hour: side.first_hour,
            last_hour: side.last_hour.min(hour),
            parent: side.parent.and_then(|parent| remap.get(parent as usize).copied().flatten()),
            existed_before: side.existed_before,
            exit: side.exit.as_ref().filter(|exit| exit.read_hour <= hour && exit.hour <= hour).cloned(),
        });
    }
    let outcome = war
        .outcome
        .as_ref()
        .filter(|outcome| outcome.read_hour <= hour && outcome.hour <= hour)
        .and_then(|outcome| {
            remap.get(outcome.winner as usize).copied().flatten().map(|winner| CivilWarOutcome {
                winner,
                hour: outcome.hour,
                read_hour: outcome.read_hour,
            })
        });
    let at_war = war
        .at_war
        .iter()
        .filter(|span| span.first_hour <= hour)
        .map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour.min(hour) })
        .collect();
    CivilWar { id: war.id.clone(), first_hour: war.first_hour, last_hour: war.last_hour.min(hour), at_war, sides, outcome }
}

pub fn answer(query: &Query<'_>, params: &CivilWarsParams, cancelled: &AtomicBool) -> io::Result<CivilWarsReply> {
    let hour = params.hour.min(query.timeline.last);
    let mut next = Next(None, hour);
    let mut full_known = Vec::new();
    let held = holdings(&query.holders);
    let ids = by_id(&held);
    let mut listings = Vec::new();
    let mut exists: HolderRuns = BTreeMap::new();
    let mut unresolved = 0_u64;
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
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
                    if first <= hour {
                        unresolved += 1;
                    } else {
                        next.see(first);
                    }
                    continue;
                };
                for holding in own {
                    for rival in rivals {
                        let (from, to) = (first.max(holding.first).max(rival.first), last.min(holding.last).min(rival.last));
                        if from <= to && holding.slot != rival.slot {
                            listings.push(Listing { a: holding.slot.min(rival.slot), b: holding.slot.max(rival.slot), first: from, last: to });
                        }
                    }
                }
            }
        }
        full_known.push(HourSpan { first_hour: stretch.first, last_hour: stretch.last });
    }
    for runs in exists.values_mut() {
        runs.sort_unstable_by_key(|run| run.0);
    }
    let evidence = crate::products::feed::annexations(query)?;
    let tags: BTreeMap<u32, Vec<HolderTag>> = held.iter().map(|(&slot, held)| (slot, tags_of(held))).collect();
    let full_wars = episodes(listings, &tags, &exists, &evidence);
    for span in &full_known {
        next.span(span.first_hour, span.last_hour);
    }
    for war in &full_wars {
        next.span(war.first_hour, war.last_hour);
        for span in &war.at_war {
            next.span(span.first_hour, span.last_hour);
        }
        for side in &war.sides {
            next.span(side.first_hour, side.last_hour);
            for tag in &side.tags {
                next.see(tag.hour);
            }
            if let Some(exit) = &side.exit {
                next.see(exit.hour);
                next.see(exit.read_hour);
            }
        }
        if let Some(outcome) = &war.outcome {
            next.see(outcome.hour);
            next.see(outcome.read_hour);
        }
    }
    let known = full_known
        .into_iter()
        .filter(|span| span.first_hour <= hour)
        .map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour.min(hour) })
        .collect();
    let wars = full_wars.iter().filter(|war| war.first_hour <= hour).map(|war| cut_war(war, hour)).collect();
    let mut warnings = Vec::new();
    if unresolved != 0 {
        warnings.push(format!("{unresolved} refused civil-war listings: a listed country has no identity"));
    }
    Ok(CivilWarsReply {
        branch_id: params.branch_id.clone(),
        hour,
        next_change: next.0.filter(|&at| at <= query.timeline.last),
        known,
        wars,
        warnings,
    })
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
