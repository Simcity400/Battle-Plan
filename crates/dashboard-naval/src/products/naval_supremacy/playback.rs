use super::*;
use dashboard_follow::panel::{Panel, PanelData};
use dashboard_query::timeline::Stretch;
use dashboard_world::products::styles::BranchLooks;
use replay_recording::columnar_frames::Recording;
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(PartialEq, Eq)]
struct Scope {
    run: u32,
    first: i32,
    last: i32,
    viewer: Option<u32>,
    ships: ShipInputs,
}

struct Changes {
    scope: Scope,
    dates: BTreeSet<i32>,
}

impl Changes {
    fn read(recording: &Recording, scope: Scope) -> io::Result<Self> {
        let mut dates = BTreeSet::new();
        let Some(viewer) = scope.viewer else { return Ok(Self { scope, dates }) };
        let mut table = |table, columns: &[usize], keys, observe: &mut dyn FnMut(u128, Option<&[Cell]>) -> bool| {
            let mut statuses = Vec::new();
            replay_lanes::points::cells(recording, scope.run, table, columns, keys, scope.first, scope.last, Some(&mut statuses), &mut |hour, key, cells| {
                if observe(key, cells) { dates.insert(hour); }
                true
            })?;
            for status in statuses {
                dates.insert(status.first);
                dates.insert(status.last.saturating_add(1));
            }
            Ok::<_, io::Error>(())
        };
        let control = control_ranges(viewer, None);
        table(TableId::NavalRegionControl, &COLUMNS, lanes::Keys::Ranges(&control), &mut |_, _| true)?;
        let ships = &scope.ships;
        if !ships.regions.is_empty() {
            let enemies = u128::from(viewer) << 34 | u128::from(AirCountryRelationRow::ENEMY) << 32;
            let range = [(enemies, enemies | u128::from(u32::MAX))];
            table(TableId::AirCountryRelations, &[0], lanes::Keys::Ranges(&range), &mut |_, _| true)?;
            let links: Vec<_> = ships.links.iter().map(|&raw| u128::from(raw)).collect();
            table(TableId::CountryLinks, &at_hour::field_columns(TableId::CountryLinks, &["canonical_slot"]), lanes::Keys::These(&links), &mut |_, _| true)?;
            let interested = |cells: Option<&[Cell]>| match cells.and_then(|values| values.first()) {
                Some(Cell::Number(id)) => Some(*id),
                _ => None,
            };
            let mut visible = HashSet::new();
            table(TableId::Countries, &at_hour::field_columns(TableId::Countries, &["canonical_slot"]), lanes::Keys::All, &mut |key, cells| {
                let before = visible.remove(&key);
                let after = interested(cells).is_some_and(|slot| ships.slots.contains(&(slot as u32)));
                if after { visible.insert(key); }
                before || after
            })?;
            let owners: Vec<_> = ships.owners.iter().map(|&owner| {
                let base = u128::from(owner) << 32;
                (base, base | u128::from(u32::MAX))
            }).collect();
            table(TableId::NavalMissionLedgers, &at_hour::field_columns(TableId::NavalMissionLedgers, &["task_forces"]), lanes::Keys::Ranges(&owners), &mut |key, _| ships.regions.contains(&(key as u32)))?;
            visible.clear();
            table(TableId::Ships, &at_hour::field_columns(TableId::Ships, &["task_force"]), lanes::Keys::All, &mut |key, cells| {
                let before = visible.remove(&key);
                let after = interested(cells).is_some_and(|force| ships.forces.contains(&(force as u64)));
                if after { visible.insert(key); }
                before || after
            })?;
        }
        Ok(Self { scope, dates })
    }

    fn through(&self, hour: i32) -> i32 {
        self.dates.range((std::ops::Bound::Excluded(hour), std::ops::Bound::Unbounded)).next()
            .map_or(self.scope.last, |next| next.saturating_sub(1).min(self.scope.last))
    }
}

fn date(data: &mut Arc<PanelData>, hour: i32) {
    let Ok(value) = data.as_ref() else { return };
    let last = value["lastHour"].as_i64();
    if last == Some(i64::from(hour)) { return; }
    let Ok(value) = Arc::make_mut(data) else { return };
    value["validThrough"] = hour.into();
    value["lastHour"] = hour.into();
    for region in value["regions"].as_array_mut().into_iter().flatten() {
        if let Some(shown) = region["readout"].get_mut("as_of_hour") && shown.as_i64() == last {
            *shown = hour.into();
        }
    }
    if let Some(gap) = value["gaps"].as_array_mut().and_then(|gaps| gaps.last_mut())
        && gap["lastHour"].as_i64() == last {
        gap["lastHour"] = hour.into();
    }
}

pub fn panel(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let branch = dashboard_query::branch(asked).map_err(|e| e.to_string())?;
    let query = dashboard_base::queries::query(source, branch).map_err(|e| e.to_string())?;
    let last = query.timeline.last;
    let start = |hour: i32| hour.div_euclid(DAY).saturating_mul(DAY);
    let held = Mutex::new(None::<Changes>);
    let styles = OnceLock::new();
    let owned = Arc::clone(source);
    let options = LoadContext { content: load.content.clone() };
    at_hour::dated_day_panel::<NavalSupremacy>(source, load, asked, "firstHour", "lastHour", move |params, reply| {
        let hour = params.last_hour.expect("dated request");
        let day = start(hour);
        let end = day.saturating_add(DAY - 1);
        let Ok(query) = dashboard_base::queries::query(&owned, branch) else { return hour };
        let Some(&Stretch { run, first, last, .. }) = query.timeline.at(hour) else {
            let next = query.timeline.stretches.partition_point(|stretch| stretch.first <= hour);
            return query.timeline.stretches.get(next).map_or(end, |stretch| end.min(stretch.first.saturating_sub(1)));
        };
        if reply.dated_refusal { return hour; }
        let holder = query.holders.at(&params.country, hour);
        let viewer = holder.and_then(|holder| match holder.holder { Holder::Found { slot, .. } => Some(slot), _ => None });
        let mut changes = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let scope = Scope { run, viewer, first: first.max(day), last: last.min(end), ships: reply.ship_inputs.clone() };
        if changes.as_ref().is_none_or(|held| held.scope != scope) {
            *changes = Changes::read(query.replay.recording(), scope).ok();
        }
        let Some(changes) = changes.as_ref() else { return hour };
        let tag = query.holders.name(&params.country, hour);
        let Ok(styles) = styles.get_or_init(|| BranchLooks::load(&owned, &query.timeline, &options, &mut Vec::new())) else { return hour };
        let identity = holder.map_or_else(|| at_hour::before(query.holders.next_change(hour)), |holder| holder.last);
        changes.through(hour).min(identity).min(at_hour::before(styles.next_for(&tag, hour)))
    }, move |data, hour| date(data, if start(hour) <= last { last.min(hour) } else { hour }))
}
