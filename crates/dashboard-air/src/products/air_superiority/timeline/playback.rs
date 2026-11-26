use super::*;
use dashboard_follow::panel::{Panel, PanelData};
use replay_recording::columnar_frames::Recording;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

struct Changes {
    first: i32,
    next: BTreeSet<i32>,
}

impl Changes {
    fn read(query: &Query<'_>, params: &AirTimelineParams) -> io::Result<Self> {
        let first = params.last_hour.div_euclid(DAY).saturating_mul(DAY);
        let end = first.saturating_add(DAY - 1);
        let mut out = Self { first, next: BTreeSet::new() };
        let recording = query.replay.recording();
        let ranges: Vec<_> = params.theatres.iter().flatten().filter_map(|s| s.parse::<u64>().ok()).map(|id| {
            let base = u128::from(id) << 32;
            (base, base | u128::from(u32::MAX))
        }).collect();
        for stretch in query.timeline.within(first, end) {
            out.next.insert(stretch.first);
            out.next.insert(stretch.last.saturating_add(1));
            let mut countries = BTreeSet::new();
            out.table(recording, &stretch, TableId::RaidAirScores, &["friendly", "enemy"], lanes::Keys::All, |key| { countries.insert(key >> 32); })?;
            let countries: Vec<_> = countries.into_iter().collect();
            out.table(recording, &stretch, TableId::CountryLinks, &["canonical_slot"], lanes::Keys::These(&countries), |_| {})?;
            if params.theatres.as_ref().is_some_and(|ids| !ids.is_empty()) {
                out.table(recording, &stretch, TableId::AirTheatreRegions, &["wings"], lanes::Keys::Ranges(&ranges), |_| {})?;
                for (start, stop) in lanes::reads(recording, stretch.run, TableId::AirTheatreRegions, stretch.first, stretch.last)? {
                    out.next.insert(start);
                    out.next.insert(stop.saturating_add(1));
                }
            }
        }
        let gaps = &query.timeline.unheld;
        let at = gaps.partition_point(|gap| gap.last < first);
        for gap in gaps[at..].iter().take_while(|gap| gap.first <= end) {
            out.next.insert(gap.first);
            out.next.insert(gap.last.saturating_add(1));
        }
        Ok(out)
    }

    fn table(&mut self, recording: &Recording, stretch: &Stretch, table: TableId, names: &[&str], keys: lanes::Keys<'_>, mut observed: impl FnMut(u128)) -> io::Result<()> {
        let columns = at_hour::field_columns(table, names);
        let mut statuses = Vec::new();
        replay_lanes::points::cells(recording, stretch.run, table, &columns, keys, stretch.first, stretch.last, Some(&mut statuses), &mut |hour, key, cells| {
            observed(key);
            self.next.insert(hour);
            if table == TableId::RaidAirScores && cells.is_none() {
                let last = hour.saturating_add(BUCKET - 1 - hour.rem_euclid(BUCKET));
                self.next.insert(last);
                self.next.insert(last.saturating_add(1));
            }
            true
        })?;
        for status in statuses {
            self.next.insert(status.first);
            self.next.insert(status.last.saturating_add(1));
            if table == TableId::RaidAirScores && !replay_recording::columnar_frames::complete(status.status) {
                for hour in status.first..=status.last {
                    if hour.rem_euclid(BUCKET) == BUCKET - 1 {
                        self.next.insert(hour);
                        self.next.insert(hour.saturating_add(1));
                    }
                }
            }
        }
        Ok(())
    }

    fn through(&self, hour: i32) -> i32 {
        self.next.range((std::ops::Bound::Excluded(hour), std::ops::Bound::Unbounded)).next().map_or(self.first.saturating_add(DAY - 1), |next| next.saturating_sub(1)).min(self.first.saturating_add(DAY - 1))
    }
}

fn date(reply: &mut Arc<PanelData>, hour: i32) {
    let Ok(value) = reply.as_ref() else { return };
    let old = value["lastHour"].as_i64();
    if old == Some(i64::from(hour)) { return; }
    let Ok(value) = Arc::make_mut(reply) else { return };
    value["lastHour"] = hour.into();
    if value["days"][0].as_i64().is_some_and(|day| Some(day) == old) {
        value["days"][0] = hour.into();
        for row in value["rows"].as_array_mut().into_iter().flatten() {
            if let Some(date) = row["readout"].get_mut("as_of_hour") { *date = hour.into(); }
        }
    }
    if let Some(last) = value["gaps"].as_array_mut().and_then(|gaps| gaps.last_mut())
        && last["lastHour"].as_i64() == old {
        last["lastHour"] = hour.into();
    }
}

pub fn panel(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    if asked["firstHour"].is_i64() {
        return at_hour::day_panel::<AirSuperiorityTimeline>(source, load, asked, "firstHour", "lastHour", |r| r.last_hour);
    }
    let branch = dashboard_query::branch(asked).map_err(|e| e.to_string())?;
    let query = dashboard_base::queries::query(source, branch).map_err(|e| e.to_string())?;
    let first = query.timeline.first;
    let held = Mutex::new(None::<Changes>);
    let styles = OnceLock::new();
    let index = OnceLock::new();
    let owned = Arc::clone(source);
    let options = LoadContext { content: load.content.clone() };
    at_hour::dated_product::<AirSuperiorityTimeline>(source, load, asked, move |request, hour| {
        request["firstHour"] = first.max(hour.div_euclid(DAY).saturating_mul(DAY)).into();
        request["lastHour"] = hour.into();
    }, move |params, reply| {
        let hour = params.last_hour;
        let Ok(query) = dashboard_base::queries::query(&owned, branch) else { return hour };
        let mut held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.as_ref().is_none_or(|day| day.first.div_euclid(DAY) != hour.div_euclid(DAY)) {
            *held = Changes::read(&query, params).ok();
        }
        let Some(day) = held.as_ref() else { return hour };
        let mut through = day.through(hour);
        if query.timeline.at(hour).is_some() {
            let Ok(index) = index.get_or_init(|| index::index(&owned, branch)) else { return hour };
            through = through.min(super::super::epoch_through(index, hour)).min(at_hour::before(query.holders.next_change(hour)));
            if !reply.sides.is_empty() {
                let Ok(styles) = styles.get_or_init(|| super::super::BranchSideResolver::load(&owned, &query, &options, &AtomicBool::new(false), &mut Vec::new())) else { return hour };
                through = through.min(styles.through(hour, &reply.sides, None));
            }
            if reply.days[0].is_none() { return hour; }
        }
        through
    }, date)
}
