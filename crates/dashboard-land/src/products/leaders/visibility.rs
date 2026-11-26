use super::{Evaluator, LeaderVisibility, LeadersContext, UnitLeaders, UnitLeadersParams, core_gap, handle, needs_world, world_of};
use dashboard_base::products::at_hour::{Read, ReadDates, ReadsAt};
use dashboard_text::triggers::{RECORDED_TABLES, inputs::{self, TableRead}};
use replay::campaign::SeekResult;
use replay_data::rows::{TableId, UnitLeaderRow};
use replay_frame::frame::DecodedFrame;
use std::{collections::{BTreeSet, HashMap, HashSet}, io, sync::atomic::AtomicBool};

#[derive(Default)]
pub(super) struct Visibility {
    pub(super) held: Option<Dated>,
    dates: ReadDates,
    pub(super) revision: u64,
}

pub(super) struct Dated {
    run: u32,
    first: i32,
    pub(super) through: i32,
    tables: BTreeSet<TableId>,
    roles: Vec<(u64, u32, usize)>,
    pub(super) decisions: HashMap<u64, Result<bool, String>>,
}

impl Dated {
    pub(super) fn covers(&self, run: u32, hour: i32) -> bool {
        self.run == run && (self.first..=self.through).contains(&hour)
    }
}

const CORE: [TableId; 4] = [TableId::CountryLinks, TableId::Countries, TableId::Provinces, TableId::States];

fn dated(rows: &[(&UnitLeaderRow, usize)], roles: Vec<(u64, u32, usize)>, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>, mut tables: BTreeSet<TableId>, dates: &mut ReadDates) -> io::Result<Dated> {
    let replay = &at.context.replay;
    tables.extend(CORE);
    loop {
        let asked: Vec<_> = tables.iter().copied().collect();
        let SeekResult::Frame(recorded) = replay.seek_selected(frame.branch, at.hour, Some(&asked))? else {
            return Err(io::Error::other("Leader visibility hour is not recorded"));
        };
        let mut world = None;
        let world = world_of(&recorded, &at.context.content, &mut world).map_err(io::Error::other)?;
        let gap = core_gap(&recorded);
        let mut reads: HashSet<_> = CORE.map(TableRead::keys).into();
        let mut decisions = HashMap::new();
        let block = replay.block_at(frame.branch, at.hour);
        let mut through = block.map_or(at.hour, |block| block.last_hour);
        for &(row, role) in rows {
            let answer = if let Some(reason) = &gap {
                Err(reason.clone())
            } else {
                let (answer, inputs) = inputs::capture(&recorded, || {
                    Evaluator::new(world, None).decide_leader_visibility(row, LeaderVisibility::Conditioned(role), true).decided()
                });
                if let Some(footprint) = inputs.footprint() {
                    reads.extend(footprint.iter().copied());
                } else {
                    tables.extend(RECORDED_TABLES.iter().copied());
                    through = at.hour;
                }
                answer
            };
            decisions.insert(handle(row), answer);
        }
        tables.extend(reads.iter().map(|read| read.table).filter(|table| RECORDED_TABLES.contains(table)));
        if tables.len() != asked.len() { continue; }
        if through > at.hour && let Some(block) = block {
            through = dates.through(replay, at.run, &reads.into_iter().collect::<Vec<_>>(), block.first_hour, at.hour, through, &AtomicBool::new(false))?;
        }
        return Ok(Dated { run: at.run, first: at.hour, through, tables, roles, decisions });
    }
}

pub(super) fn prepare(params: &UnitLeadersParams, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>) -> (Vec<Read>, i32) {
    let mut world = None;
    let rows: Vec<_> = UnitLeaders::kept(params, frame, at).into_iter().filter(|row| {
        needs_world(row) && !(row.kind == super::KIND_ARMY && (row.category as u32) < 2 && row.captured != 0)
    }).filter_map(|row| match world_of(frame, &at.context.content, &mut world).ok()?.decoded_leader_visibility(row) {
        LeaderVisibility::Conditioned(role) => Some((row, role)),
        _ => None,
    }).collect();
    let roles: Vec<_> = rows.iter().map(|&(row, role)| (handle(row), row.owner, role)).collect();
    let mut visibility = at.context.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if rows.is_empty() {
        if visibility.held.take().is_some() { visibility.revision += 1; }
    } else if !visibility.held.as_ref().is_some_and(|held| held.covers(at.run, at.hour) && roles == held.roles) {
        let tables = visibility.held.as_ref().map(|held| held.tables.clone()).unwrap_or_default();
        match dated(&rows, roles, frame, at, tables, &mut visibility.dates) {
            Ok(proof) => {
                if visibility.held.as_ref().is_none_or(|prior| prior.decisions != proof.decisions) { visibility.revision += 1; }
                visibility.held = Some(proof);
            }
            Err(_) => {
                visibility.revision += 1;
                visibility.held = None;
                return (RECORDED_TABLES.iter().copied().filter(|table| !super::BASE_TABLES.contains(table)).map(Read::all).collect(), at.hour);
            }
        }
    }
    let through = visibility.held.as_ref().map_or(i32::MAX, |held| held.through);
    (Vec::new(), through.min(at.context.through(at.hour)))
}
