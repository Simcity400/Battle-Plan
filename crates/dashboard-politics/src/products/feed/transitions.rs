use super::row::{Party, Slot};
use dashboard_query::Query;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use replay_lanes::lanes::{self, Cell, History, Keys, Reading};
use replay_recording::columnar_frames::Recording;
use std::{collections::{BTreeSet, HashMap}, io, sync::atomic::AtomicBool};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeWarEnd { pub hour: i32, pub observed: i32, pub actor: Party, pub target: Party }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedParty { pub raw: u32, pub id: Option<u32>, pub slot: Option<u32>, pub tag: Option<String>, pub missing: Option<String> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FactionTransition { pub joined: bool, pub hour: i32, pub faction: u64, pub party: ResolvedParty }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WarEnd { pub hour: i32, pub parties: [ResolvedParty; 2], pub slots_at_end: [Option<u32>; 2] }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BranchTransitions {
    pub factions: Vec<FactionTransition>,
    pub war_ends: Vec<WarEnd>,
    pub faction_unrecorded: Vec<(i32, i32)>,
    pub war_unrecorded: Vec<(i32, i32)>,
    pub faction_errors: Vec<(i32, i32, String, bool)>,
    pub war_errors: Vec<(i32, i32, String, bool)>,
    pub unsettled: bool,
}
impl BranchTransitions {
    pub(crate) fn is_unsettled(&self) -> bool { self.unsettled || self.faction_errors.iter().any(|e| e.3) || self.war_errors.iter().any(|e| e.3) }
}

fn num(cell: Cell) -> Option<i64> { match cell { Cell::Number(v) => Some(v), _ => None } }
fn text_utf8(history: &History, cell: Cell) -> Option<String> { history.text(cell).and_then(|b| std::str::from_utf8(b).ok()).filter(|s| !s.is_empty()).map(str::to_owned) }
fn is_complete(history: &History, hour: i32) -> bool { history.statuses.get(history.statuses.partition_point(|r| r.last < hour)).is_some_and(|r| r.first <= hour && matches!(r.status, Status::Complete)) }
fn is_unsettled(error: &io::Error) -> bool { error.kind() != io::ErrorKind::InvalidData && dashboard_core::refusal(error).is_none() }
fn link_slot(links: Option<&History>, raw: u32, hour: i32) -> Option<u32> { match links?.reading(u128::from(raw), hour) { Reading::Cells(c) => num(c[0]).map(|v| v as u32), _ => None } }
fn unread(label: &str, status: Status, reason: Option<&str>) -> String {
    match (status, reason) {
        (Status::Unavailable { .. }, Some(text)) if !text.is_empty() => format!("{label} unavailable at this hour: {text}"),
        (Status::Unavailable { .. }, _) => format!("{label} unavailable at this hour"),
        (Status::Truncated { .. }, _) => format!("{label} truncated at this hour"),
        _ => format!("{label} not recorded at this hour"),
    }
}

fn add_unprovable(history: &History, first: i32, last: i32, timeline_first: i32, out: &mut Vec<(i32, i32)>) {
    if first > timeline_first { out.push((first, first)); }
    let mut mark = |a: i32, b: i32| { let (lo, hi) = (a.max(first.saturating_add(1)), b.saturating_add(1).min(last)); if lo <= hi { out.push((lo, hi)); } };
    let mut hour = first;
    for run in &history.statuses[history.statuses.partition_point(|r| r.last < first)..] {
        if run.first > last { break; }
        if hour < run.first { mark(hour, run.first - 1); }
        let end = run.last.min(last);
        if !matches!(run.status, Status::Complete) { mark(hour.max(run.first), end); }
        hour = end.saturating_add(1);
    }
    if hour <= last { mark(hour, last); }
}

struct SlotIndex { by_slot: HashMap<u32, Vec<(i32, i32, u32, Option<String>)>>, reuse_hours: BTreeSet<i32> }
impl SlotIndex {
    fn build(countries: &History, first: i32, last: i32) -> Self {
        let mut by_slot: HashMap<u32, Vec<(i32, i32, u32, Option<String>)>> = HashMap::new();
        for (key, steps) in &countries.keys {
            for (i, step) in steps.iter().enumerate() {
                if let Some(c) = &step.cells && let Some(slot) = num(c[0]).map(|v| v as u32) {
                    let (a, b) = (step.hour.max(first), steps.get(i + 1).map_or(countries.to, |n| n.hour - 1).min(last));
                    if a <= b { by_slot.entry(slot).or_default().push((a, b, *key as u32, text_utf8(countries, c[1]))); }
                }
            }
        }
        let mut reuse_hours = BTreeSet::new();
        for entries in by_slot.values_mut() {
            entries.sort_unstable_by_key(|&(a, b, id, _)| (a, b, id));
            for &(a, _, id, _) in entries.iter() {
                if a > first && entries.iter().any(|&(x, y, other, _)| other != id && x <= a - 1 && a - 1 <= y) { reuse_hours.insert(a); }
            }
        }
        Self { by_slot, reuse_hours }
    }
    fn at(&self, slot: u32, hour: i32, preferred: u32) -> Option<(u32, Option<String>)> {
        let entries = self.by_slot.get(&slot)?;
        let mut active = entries.iter().filter(|(a, b, ..)| *a <= hour && hour <= *b);
        let first = active.next()?;
        if active.next().is_none() { Some((first.2, first.3.clone())) }
        else { entries.iter().find(|(a, b, id, _)| *a <= hour && hour <= *b && *id == preferred).map(|(_, _, id, tag)| (*id, tag.clone())) }
    }
}

const NO_TAG: &str = "Country tag was not recorded at this hour";
const NO_COUNTRY: &str = "Country was not recorded in the countries table at this hour";
const NO_LINK: &str = "Country link was not recorded at this hour";

fn country_by_id(countries: Result<&History, &str>, links: Option<&History>, id: u32, hour: i32) -> ResolvedParty {
    let countries = match countries {
        Ok(held) => held,
        Err(reason) => return ResolvedParty { raw: id, id: Some(id), slot: link_slot(links, id, hour), tag: None, missing: Some(reason.to_owned()) },
    };
    match countries.reading(u128::from(id), hour) {
        Reading::Cells(c) => {
            let tag = text_utf8(countries, c[1]);
            let missing = tag.is_none().then(|| NO_TAG.to_owned());
            ResolvedParty { raw: id, id: Some(id), slot: num(c[0]).map(|v| v as u32).or_else(|| link_slot(links, id, hour)), tag, missing }
        }
        Reading::Absent => ResolvedParty { raw: id, id: Some(id), slot: link_slot(links, id, hour), tag: None, missing: Some(NO_COUNTRY.to_owned()) },
        Reading::Unread { status, reason } => ResolvedParty { raw: id, id: Some(id), slot: link_slot(links, id, hour), tag: None, missing: Some(unread("Countries", status, reason)) },
    }
}

fn resolve_war_ref(links: &History, slots: &SlotIndex, raw: u32, hour: i32) -> (Option<u32>, ResolvedParty) {
    let Some(slot) = link_slot(Some(links), raw, hour) else {
        return (None, ResolvedParty { raw, id: None, slot: None, tag: None, missing: Some(NO_LINK.to_owned()) });
    };
    match slots.at(slot, hour, raw) {
        Some((id, tag)) => { let missing = tag.is_none().then(|| NO_TAG.to_owned()); (Some(id), ResolvedParty { raw, id: Some(id), slot: Some(slot), tag, missing }) }
        None => (None, ResolvedParty { raw, id: None, slot: Some(slot), tag: None, missing: Some(NO_COUNTRY.to_owned()) }),
    }
}

fn slot_at_hour(countries: &History, links: &History, party: &ResolvedParty, hour: i32) -> Option<u32> {
    match party.id.map(|id| countries.reading(u128::from(id), hour)) {
        Some(Reading::Cells(c)) => num(c[0]).map(|v| v as u32),
        Some(_) => None,
        None => link_slot(Some(links), party.raw, hour),
    }
}

fn member_faction(members: &History, factions: &History, country: u32, hour: i32) -> Option<u64> {
    let Reading::Cells(c) = members.reading(u128::from(country), hour) else { return None };
    let faction = num(c[0])? as u64;
    matches!(factions.reading(u128::from(faction), hour), Reading::Cells(_)).then_some(faction)
}

fn read_tables(recording: &Recording, run: u32, first: i32, last: i32) -> (io::Result<History>, io::Result<History>, io::Result<History>, io::Result<History>, io::Result<History>) {
    let load = |table: TableId, cols: &[usize]| -> io::Result<History> {
        if recording.predates(table.index()).is_some() { return Ok(History { columns: cols.to_vec(), to: last, ..Default::default() }); }
        lanes::history(recording, run, table, cols, Keys::All, first, last)
    };
    let ((countries, links), (factions, (members, wars))) = rayon::join(
        || rayon::join(|| load(TableId::Countries, &[0, 1]), || load(TableId::CountryLinks, &[0])),
        || rayon::join(|| load(TableId::Factions, &[]), || rayon::join(|| load(TableId::FactionMembers, &[0]), || load(TableId::WarRelations, &[0, 1]))),
    );
    (countries, links, factions, members, wars)
}

pub(crate) fn load_branch(query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<BranchTransitions> {
    let (recording, timeline_first, mut out) = (query.replay.recording(), query.timeline.first, BranchTransitions::default());
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (first, last) = (stretch.first, stretch.last);
        let (countries, links, factions, members, wars) = read_tables(recording, stretch.run, first, last);
        dashboard_text::metadata::check_cancel(cancelled)?;
        let countries_res = match &countries {
            Ok(held) => Ok(held),
            Err(error) => { out.unsettled |= is_unsettled(error); Err(error.to_string()) }
        };
        let links_opt = match &links {
            Ok(held) => Some(held),
            Err(error) => { out.unsettled |= is_unsettled(error); None }
        };
        match (&factions, &members) {
            (Ok(factions), Ok(members)) => {
                add_unprovable(factions, first, last, timeline_first, &mut out.faction_unrecorded);
                add_unprovable(members, first, last, timeline_first, &mut out.faction_unrecorded);
                let faction_steps: HashMap<u64, Vec<i32>> = factions.keys.iter().map(|(k, s)| (*k as u64, s.iter().map(|x| x.hour).filter(|&h| h > first && h <= last).collect())).collect();
                for (key, steps) in &members.keys {
                    let country = *key as u32;
                    let mut hours: Vec<i32> = steps.iter().map(|s| s.hour).filter(|&h| h > first && h <= last).collect();
                    for step in steps { if let Some(c) = &step.cells && let Some(f) = num(c[0]).map(|v| v as u64) && let Some(extra) = faction_steps.get(&f) { hours.extend_from_slice(extra); } }
                    hours.sort_unstable();
                    hours.dedup();
                    for hour in hours {
                        if !is_complete(factions, hour - 1) || !is_complete(factions, hour) || !is_complete(members, hour - 1) || !is_complete(members, hour) { continue; }
                        let (prev, curr) = (member_faction(members, factions, country, hour - 1), member_faction(members, factions, country, hour));
                        if prev != curr {
                            let c_ref = countries_res.as_ref().map(|h| *h).map_err(String::as_str);
                            if let Some(faction) = prev { out.factions.push(FactionTransition { joined: false, hour, faction, party: country_by_id(c_ref, links_opt, country, hour - 1) }); }
                            if let Some(faction) = curr { out.factions.push(FactionTransition { joined: true, hour, faction, party: country_by_id(c_ref, links_opt, country, hour) }); }
                        }
                    }
                }
            }
            (Err(error), _) | (_, Err(error)) => out.faction_errors.push((first, last, error.to_string(), is_unsettled(error))),
        }
        match (&wars, &countries, &links) {
            (Ok(wars), Ok(countries), Ok(links)) => {
                for table in [wars, countries, links] { add_unprovable(table, first, last, timeline_first, &mut out.war_unrecorded); }
                let slots = SlotIndex::build(countries, first, last);
                let mut intervals: Vec<(i32, i32, (u32, u32))> = Vec::new();
                for (key, steps) in &wars.keys {
                    let (raw, mut prev_idx) = (((*key >> 32) as u32, *key as u32), None::<usize>);
                    for (i, step) in steps.iter().enumerate() {
                        if let Some(c) = &step.cells && let (Some(start), Some(end)) = (num(c[0]).map(|v| v as i32), num(c[1]).map(|v| v as i32)) {
                            let next_h = steps.get(i + 1).map_or(last.saturating_add(1), |n| n.hour);
                            let (a, b) = (step.hour.max(first), if end > start { end.min(next_h) } else { next_h }.saturating_sub(1).min(last));
                            if a <= b {
                                if let Some(p) = prev_idx && intervals[p].1.saturating_add(1) == a { intervals[p].1 = b; }
                                else { prev_idx = Some(intervals.len()); intervals.push((a, b, raw)); }
                            }
                        }
                    }
                }
                let war_refs: BTreeSet<u32> = intervals.iter().flat_map(|&(_, _, (a, b))| [a, b]).collect();
                let mut recheck = slots.reuse_hours.clone();
                for (k, steps) in &links.keys {
                    if war_refs.contains(&(*k as u32)) { recheck.extend(steps.iter().map(|s| s.hour).filter(|&h| h > first && h <= last)); }
                }
                let mut cand: Vec<i32> = intervals.iter().filter(|&&(_, b, _)| b < last).map(|&(_, b, _)| b + 1).chain(recheck.iter().copied()).collect();
                cand.sort_unstable();
                cand.dedup();
                let (mut starts, mut ends): (Vec<_>, Vec<_>) = (intervals.iter().enumerate().map(|(i, &(a, ..))| (a, i)).collect(), intervals.iter().enumerate().map(|(i, &(_, b, _))| (b.saturating_add(1), i)).collect());
                starts.sort_unstable();
                ends.sort_unstable();
                let (mut active, mut s_pos, mut e_pos) = (BTreeSet::new(), 0usize, 0usize);
                for hour in cand {
                    while s_pos < starts.len() && starts[s_pos].0 <= hour - 1 { active.insert(starts[s_pos].1); s_pos += 1; }
                    while e_pos < ends.len() && ends[e_pos].0 <= hour - 1 { active.remove(&ends[e_pos].1); e_pos += 1; }
                    let ended: Vec<(u32, u32)> = if recheck.contains(&hour) { active.iter().map(|&i| intervals[i].2).collect() }
                        else { ends[e_pos..].iter().take_while(|&&(h, _)| h == hour).map(|&(_, i)| intervals[i].2).collect() };
                    while s_pos < starts.len() && starts[s_pos].0 <= hour { active.insert(starts[s_pos].1); s_pos += 1; }
                    while e_pos < ends.len() && ends[e_pos].0 <= hour { active.remove(&ends[e_pos].1); e_pos += 1; }
                    if ended.is_empty() || ![wars, countries, links].iter().all(|t| is_complete(t, hour - 1) && is_complete(t, hour)) { continue; }
                    let curr: Vec<(Option<u32>, Option<u32>)> = active.iter().map(|&i| {
                        let (a, b) = intervals[i].2;
                        (resolve_war_ref(links, &slots, a, hour).0, resolve_war_ref(links, &slots, b, hour).0)
                    }).collect();
                    let mut emitted: BTreeSet<(bool, u32, u32)> = BTreeSet::new();
                    let may_be = |w: Option<u32>, g: Option<u32>| w.zip(g).is_none_or(|(x, y)| x == y);
                    for (ra, rb) in ended {
                        let ((ia, pa), (ib, pb)) = (resolve_war_ref(links, &slots, ra, hour - 1), resolve_war_ref(links, &slots, rb, hour - 1));
                        let may_match = |ca: Option<u32>, cb: Option<u32>| (may_be(ia, ca) && may_be(ib, cb)) || (may_be(ia, cb) && may_be(ib, ca));
                        let proven = match (ia, ib) {
                            (Some(x), Some(y)) if curr.iter().any(|&(ca, cb)| matches!((ca, cb), (Some(a), Some(b)) if (a.min(b), a.max(b)) == (x.min(y), x.max(y)))) => false,
                            (Some(_), Some(_)) if curr.iter().any(|&(ca, cb)| (ca.is_none() || cb.is_none()) && may_match(ca, cb)) => { out.war_unrecorded.push((hour, hour)); false }
                            (Some(x), Some(y)) => emitted.insert((true, x.min(y), x.max(y))),
                            _ if curr.iter().any(|&(ca, cb)| may_match(ca, cb)) => { out.war_unrecorded.push((hour, hour)); false }
                            _ => emitted.insert((false, ra.min(rb), ra.max(rb))),
                        };
                        if proven {
                            let slots_at_end = [slot_at_hour(countries, links, &pa, hour), slot_at_hour(countries, links, &pb, hour)];
                            out.war_ends.push(WarEnd { hour, parties: [pa, pb], slots_at_end });
                        }
                    }
                }
            }
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => out.war_errors.push((first, last, error.to_string(), is_unsettled(error))),
        }
    }
    Ok(out)
}

pub(crate) fn native_supersedes(end: &WarEnd, native_ends: &[NativeWarEnd]) -> Option<i32> {
    let matches = |ep: &ResolvedParty, slot_end: Option<u32>, native: Party, event_hour: i32| {
        if let (Some(eid), Some(pid)) = (ep.id, native.id) { return eid == pid; }
        if native.reference != 0 && ep.raw == native.reference { return true; }
        matches!(native.slot, Slot::Of(slot) if (if event_hour == end.hour { slot_end } else { ep.slot }) == Some(slot))
    };
    native_ends.iter().filter(|n| n.hour == end.hour || n.hour == end.hour - 1).filter(|n| {
        let ([p0, p1], [s0, s1]) = (&end.parties, end.slots_at_end);
        (matches(p0, s0, n.actor, n.hour) && matches(p1, s1, n.target, n.hour)) || (matches(p0, s0, n.target, n.hour) && matches(p1, s1, n.actor, n.hour))
    }).map(|n| n.observed).min()
}
