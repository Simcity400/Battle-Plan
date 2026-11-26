use super::{LeadersContext, UnitLeaders, UnitLeadersParams, conditioned, handle, needs_world, visible, world_of};
use dashboard_base::products::at_hour::{Read, ReadsAt, reads_through};
use dashboard_text::triggers::{RECORDED_TABLES, inputs::{self, TableRead}};
use replay::campaign::SeekResult;
use replay_data::rows::{TableId, UnitLeaderRow};
use replay_frame::frame::DecodedFrame;
use std::{collections::{HashMap, HashSet}, io, sync::atomic::AtomicBool};

pub(super) struct Dated {
    run: u32,
    first: i32,
    pub(super) through: i32,
    pub(super) decisions: HashMap<u64, Result<bool, String>>,
}

impl Dated {
    pub(super) fn covers(&self, run: u32, hour: i32) -> bool {
        self.run == run && (self.first..=self.through).contains(&hour)
    }
}

fn dated(rows: &[&UnitLeaderRow], frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>) -> io::Result<Dated> {
    let replay = &at.context.replay;
    let SeekResult::Frame(recorded) = replay.seek_selected(frame.branch, at.hour, Some(RECORDED_TABLES))? else {
        return Err(io::Error::other("Leader visibility hour is not recorded"));
    };
    let mut world = None;
    let mut reads = HashSet::new();
    let mut decisions = HashMap::new();
    let mut through = replay.block_at(frame.branch, at.hour).map_or(at.hour, |block| block.last_hour);
    for &row in rows {
        let (answer, inputs) = inputs::capture(&recorded, || visible(row, &recorded, &at.context.content, &mut world));
        if let Some(footprint) = inputs.footprint() {
            reads.extend(footprint.iter().copied());
        } else {
            through = at.hour;
        }
        reads.insert(TableRead::all(TableId::UnitLeaders).row(handle(row).into()));
        reads.insert(TableRead::all(TableId::Characters).row(row.character_id.into()));
        decisions.insert(handle(row), answer);
    }
    if through > at.hour {
        let mut reads: Vec<_> = reads.into_iter().collect();
        reads.extend([TableId::CountryLinks, TableId::Countries, TableId::Provinces, TableId::States].map(TableRead::keys));
        through = reads_through(replay, at.run, &reads, at.hour, through, &AtomicBool::new(false))?;
    }
    Ok(Dated { run: at.run, first: at.hour, through, decisions })
}

pub(super) fn prepare(params: &UnitLeadersParams, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>) -> (Vec<Read>, i32) {
    let mut rows = UnitLeaders::kept(params, frame, at);
    rows.retain(|row| needs_world(row));
    let mut world = None;
    if !rows.is_empty() {
        match world_of(frame, &at.context.content, &mut world) {
            Ok(world) => rows.retain(|row| conditioned(row, world)),
            Err(_) => rows.clear(),
        }
    }
    let mut held = at.context.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if rows.is_empty() {
        *held = None;
    } else if !held.as_ref().is_some_and(|held| held.covers(at.run, at.hour) && rows.iter().all(|row| held.decisions.contains_key(&handle(row)))) {
        match dated(&rows, frame, at) {
            Ok(proof) => *held = Some(proof),
            Err(_) => {
                *held = None;
                return (RECORDED_TABLES.iter().copied().filter(|table| !super::BASE_TABLES.contains(table)).map(Read::all).collect(), at.hour);
            }
        }
    }
    drop(held);
    (Vec::new(), at.context.through(at.hour))
}
