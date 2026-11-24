
use super::{
    AirMode,
    assignment::AssignedSide,
    index::AirIndex,
    timeline::{AirTimelineParams, DAY, DayActivity, StretchDays, day_sums},
};
use dashboard_query::{
    identity::{Holder, Holders},
    timeline::Stretch,
};
use replay_data::rows::TableId;
use std::collections::{BTreeSet, HashMap, HashSet};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnerDay {
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

impl OwnerDay {
    fn activity(&self, enemy: bool) -> DayActivity {
        let (activity, total) = if enemy { (self.friendly + self.enemy, self.friendly_total + self.enemy_total) } else { (self.friendly, self.friendly_total) };
        DayActivity { day: self.day, activity, total }
    }
}

pub(crate) trait Ledger {
    fn read(&self, table: TableId, run: u32, first: i32, last: i32) -> bool;
    fn owner(&self, run: u32, hour: i32, raw: u32) -> Option<HashMap<u32, OwnerDay>>;
    fn regions(&self, run: u32, hour: i32) -> Option<HashMap<u32, DayActivity>>;
    fn theatres(&self, run: u32, first: i32, last: i32, theatres: &[u64]) -> Option<(Vec<(i32, i32)>, Vec<(u32, i32, i32)>)>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    pub counts_last: bool,
}

pub(crate) fn pieces(stretches: &[Stretch], last_hour: i32, cuts: &BTreeSet<i32>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for stretch in stretches {
        let mut first = stretch.first;
        for &cut in cuts.range((std::ops::Bound::Excluded(stretch.first), std::ops::Bound::Included(stretch.last))) {
            out.push(Piece { run: stretch.run, first, last: cut - 1, counts_last: true });
            first = cut;
        }
        out.push(Piece { run: stretch.run, first, last: stretch.last, counts_last: true });
    }
    for at in 0..out.len() {
        let last_day = out[at].last.div_euclid(DAY);
        out[at].counts_last = match out.get(at + 1) {
            Some(next) => next.first.div_euclid(DAY) != last_day,
            None => last_hour.rem_euclid(DAY) == DAY - 1 || last_hour.div_euclid(DAY) != last_day,
        };
    }
    out
}

fn representative(side: &AssignedSide) -> Result<Option<&str>, ()> {
    match &side.leader {
        Some(leader) => Ok(Some(leader)),
        None if side.lone() => Ok(side.members.first().map(String::as_str)),
        None if !side.factions.is_empty() => Ok(None),
        None => Err(()),
    }
}

fn owners(params: &AirTimelineParams, index: &AirIndex, holders: &Holders, hour: i32) -> Result<BTreeSet<u32>, ()> {
    let raw = |tag: &str| match holders.at(tag, hour).map(|run| run.holder) {
        Some(Holder::Found { id, .. }) => Some(id),
        _ => None,
    };
    let viewpoint = super::viewpoint_name(params.viewpoint.as_ref(), params.viewpoint_country == Some(true), holders, hour);
    let sides: &[AssignedSide] = index.epoch(hour).and_then(|epoch| epoch.sides.as_deref()).map_or(&[], Vec::as_slice);
    let mut tags: Vec<&str> = Vec::new();
    match params.mode.unwrap_or_default() {
        AirMode::Viewpoint if params.viewpoint_country == Some(true) => tags.push(viewpoint),
        AirMode::Viewpoint => {
            for side in sides.iter().filter(|side| side.key == viewpoint) {
                tags.extend(representative(side)?);
            }
        }
        AirMode::Sides => {
            let keys: HashSet<&str> = params.sides.iter().flatten().map(String::as_str).collect();
            for side in sides.iter().filter(|side| keys.contains(side.key.as_str())) {
                tags.extend(representative(side)?);
            }
        }
    }
    Ok(tags.into_iter().filter_map(raw).collect())
}

pub(crate) fn sums(ledger: &dyn Ledger, params: &AirTimelineParams, index: &AirIndex, holders: &Holders, stretches: &[Stretch], last_hour: i32) -> Option<HashMap<u32, u128>> {
    let mode = params.mode.unwrap_or_default();
    let plain = mode == AirMode::Sides && params.sides.as_ref().is_none_or(Vec::is_empty);
    let viewpoint = mode == AirMode::Viewpoint;
    let (Some(first), Some(last)) = (stretches.first().map(|stretch| stretch.first), stretches.last().map(|stretch| stretch.last)) else {
        return Some(HashMap::new());
    };
    let mut cuts = BTreeSet::new();
    if !plain {
        let mut hours: BTreeSet<i32> = index.epochs.iter().map(|epoch| epoch.first).filter(|hour| (first + 1..=last).contains(hour)).collect();
        hours.extend(holders.tags.values().flatten().flat_map(|run| [run.first, run.last.saturating_add(1)]).filter(|hour| (first + 1..=last).contains(hour)));
        let mut held = owners(params, index, holders, first).ok()?;
        for hour in hours {
            let now = owners(params, index, holders, hour).ok()?;
            if now != held {
                cuts.insert(hour);
                held = now;
            }
        }
    }
    let theatres: Option<Vec<u64>> = params.theatres.as_ref().filter(|ids| !ids.is_empty()).map(|ids| ids.iter().filter_map(|id| id.parse().ok()).collect());
    let mut held_theatres: HashMap<u32, (Vec<(i32, i32)>, Vec<(u32, i32, i32)>)> = HashMap::new();
    if let Some(ids) = &theatres {
        for stretch in stretches {
            let (read, runs) = ledger.theatres(stretch.run, stretch.first, stretch.last, ids)?;
            for &(a, b) in &read {
                cuts.extend([a, b.saturating_add(1)]);
            }
            for &(_, a, b) in &runs {
                cuts.extend([a, b.saturating_add(1)]);
            }
            let held = held_theatres.entry(stretch.run).or_default();
            held.0.extend(read);
            held.1.extend(runs);
        }
    }
    let mut parts = Vec::new();
    for piece in pieces(stretches, last_hour, &cuts) {
        let members: Option<HashSet<u32>> = theatres.as_ref().map(|_| {
            let (read, runs) = held_theatres.get(&piece.run).map_or((&[][..], &[][..]), |(read, runs)| (read.as_slice(), runs.as_slice()));
            if !read.iter().any(|(a, b)| *a <= piece.last && piece.last <= *b) {
                return HashSet::new();
            }
            runs.iter().filter(|(_, a, b)| *a <= piece.last && piece.last <= *b).map(|(region, ..)| *region).collect()
        });
        let keep = |rows: HashMap<u32, DayActivity>| -> HashMap<u32, DayActivity> {
            match &members {
                Some(members) => rows.into_iter().filter(|(region, _)| members.contains(region)).collect(),
                None => rows,
            }
        };
        let stretch = |start: HashMap<u32, DayActivity>, end: HashMap<u32, DayActivity>| StretchDays {
            first_day: piece.first.div_euclid(DAY),
            last_day: piece.last.div_euclid(DAY),
            counts_last: piece.counts_last,
            start: keep(start),
            end: keep(end),
        };
        if plain {
            if !ledger.read(TableId::AirRegionDays, piece.run, piece.first, piece.last) {
                return None;
            }
            parts.push(stretch(ledger.regions(piece.run, piece.first)?, ledger.regions(piece.run, piece.last)?));
            continue;
        }
        if !ledger.read(TableId::AirOwnerDays, piece.run, piece.first, piece.last) {
            return None;
        }
        for raw in owners(params, index, holders, piece.last).ok()? {
            let rows = |hour: i32| ledger.owner(piece.run, hour, raw).map(|rows| rows.into_iter().map(|(region, row)| (region, row.activity(viewpoint))).collect::<HashMap<_, _>>());
            parts.push(stretch(rows(piece.first)?, rows(piece.last)?));
        }
    }
    Some(day_sums(&parts))
}
