
use super::{Kept, Table};
use crate::playhead_host::Entry;
use playhead::{Timeline, pages::Pages};
use replay::campaign::Replay;
use replay_columnar::{manifest::Status, unit::BLOCK};
use replay_data::{
    BranchId, Hour,
    rows::{TableSchema, tables},
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Instant,
};
use viewer_counter_products::map::province_tooltips::air_region::{Roster, SUPERSEDED, SampleSource, Span, sample_hour};

const READER: &str = "The air roster history";

pub struct Lanes {
    pages: Arc<dyn Pages>,
    timeline: Timeline,
    branch: BranchId,
    reads: Mutex<Vec<(u32, u64)>>,
}

impl Lanes {
    pub fn of(entry: &Entry, replay: &Replay) -> Option<Self> {
        let timeline = entry.playhead().timeline().clone();
        let branch = crate::playhead_host::units::branch_playing(replay, &**entry.pages(), &timeline)?;
        Some(Self { pages: Arc::clone(entry.pages()), timeline, branch, reads: Mutex::default() })
    }

    pub fn reads(&self) -> Vec<(u32, u64)> {
        self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn read(&self, run: usize, first: Hour, last: Hour, superseded: &dyn Fn() -> bool) -> io::Result<Result<Vec<(Hour, Result<Roster, String>)>, String>> {
        let mut rosters = Table::<tables::AirDailyRoster>::open(&self.pages, run, None, READER)?;
        let mut links = Table::<tables::CountryLinks>::open(&self.pages, run, None, READER)?;
        rosters.seek(first)?;
        links.seek(first)?;
        let mut samples = Vec::new();
        for at in first..=last {
            if at > first {
                if superseded() {
                    return Ok(Err(SUPERSEDED.into()));
                }
                rosters.step(at)?;
                links.step(at)?;
            }
            if sample_hour(at) {
                samples.push((at, sample(&rosters, &links)));
            }
        }
        Ok(Ok(samples))
    }
}

impl SampleSource for Lanes {
    fn span(&self, branch: BranchId, hour: Hour, superseded: &dyn Fn() -> bool) -> Result<Span, String> {
        if branch != self.branch {
            return Err(format!("the air roster lanes hold branch {}, not {branch}", self.branch));
        }
        let Some(piece) = self.timeline.pieces().iter().find(|piece| piece.first <= hour && hour <= piece.last) else {
            return Err(format!("hour {hour} is outside the air roster lanes ({} to {})", self.timeline.first(), self.timeline.last()));
        };
        let run_first = self.pages.runs()[piece.run].first_hour;
        let block = (hour - run_first) as u32 / BLOCK;
        let first = (run_first + (block * BLOCK) as Hour).max(piece.first);
        let last = (run_first + ((block + 1) * BLOCK) as Hour - 1).min(piece.last);
        let began = Instant::now();
        let samples = self.read(piece.run, first, last, superseded).map_err(|error| format!("the air roster lanes at hour {hour}: {error}"))??;
        self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(((last - first + 1) as u32, began.elapsed().as_nanos() as u64));
        Ok(Span { first, last, samples })
    }
}

fn sample(rosters: &Table<tables::AirDailyRoster>, links: &Table<tables::CountryLinks>) -> Result<Roster, String> {
    let whole = |status: Status, reason: &str, name: &str| match status {
        Status::Complete => Ok(()),
        Status::Truncated { missing } => Err(format!("{name} is truncated, {missing} rows missing")),
        Status::Unavailable { .. } => Err(format!("{name} is unavailable: {reason}")),
        Status::NotRecorded => Err(format!("{name} is not recorded")),
    };
    whole(rosters.status().0, rosters.reason(), "AirDailyRoster")?;
    whole(links.status().0, links.reason(), "CountryLinks")?;
    let held = |rows| tables::AirDailyRoster::borrow(rows).ok_or("AirDailyRoster holds rows of another table");
    let linked = |rows| tables::CountryLinks::borrow(rows).ok_or("CountryLinks holds rows of another table");
    Roster::of(held(rosters.rows())?, linked(links.rows())?)
}
