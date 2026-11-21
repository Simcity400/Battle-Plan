use super::output_continuity::{self, Progress, Reasons, State};
use crate::output_log::Drained;
use engine_allocation::{collect_vec, mul, reserve_map};
use engine_production::{
    line_output_sites::RocketFailure, line_output_sites::RocketTick, line_output_sites::UNIT_SCALE,
};
use replay_data::{BranchId, rows::RocketProductionOutputRow};
use replay_frame::{frame::TableInput, frame::TableView};
use std::{collections::HashMap, io};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Entry {
    #[default]
    Equipment,
    Spending(RocketTick),
    Delivery {
        country: u32,
        variant: u64,
        units: Option<i32>,
        coherent: bool,
    },
}

impl Entry {
    pub fn delivery(scope: Option<RocketTick>, variant: u64, amount: i64) -> Self {
        let units = (amount % UNIT_SCALE == 0)
            .then(|| i32::try_from(amount / UNIT_SCALE).ok())
            .flatten();
        let country = scope.map_or(0, |tick| tick.country);
        let coherent = scope.is_some_and(|tick| {
            tick.country != 0 && variant != 0 && (tick.variant == 0 || tick.variant == variant)
        });
        Self::Delivery {
            country,
            variant,
            units,
            coherent,
        }
    }
}

#[derive(Debug)]
pub struct Totals {
    branch: Option<BranchId>,
    state: State,
    complete: bool,
    rows: HashMap<(u32, u64), RocketProductionOutputRow>,
}

impl Default for Totals {
    fn default() -> Self {
        Self {
            branch: None,
            state: State::Withheld("no rocket frame observed yet"),
            complete: false,
            rows: HashMap::new(),
        }
    }
}

impl Totals {
    pub fn observe<E>(
        &mut self,
        branch: BranchId,
        hour: i32,
        drained: Option<Result<Drained, &'static str>>,
        entries: &[E],
        rocket: impl Fn(&E) -> Entry,
    ) -> io::Result<()> {
        let drained = match drained {
            Some(Ok(d)) if d.copied > entries.len() => {
                Some(Err("rocket drain entry count exceeds owned entries"))
            }
            Some(Ok(d))
                if d.first
                    .checked_add(d.copied as u64)
                    .is_none_or(|end| end > d.cursor)
                    || d.pending < d.copied =>
            {
                Some(Err("rocket drain bounds are inconsistent"))
            }
            other => other,
        };
        let progress = output_continuity::observe(
            branch,
            &mut self.branch,
            &mut self.state,
            &mut self.complete,
            hour,
            drained,
            &Reasons {
                first: "the branch's first frame drained no rocket events",
                restarted: "rocket production stopped being followed in the game",
                lost: "some rocket production in the game was missed",
                absent: "a frame drained no rocket events",
            },
        );
        match progress {
            Progress::Restart => {
                self.rows.clear();
                reserve_map(&mut self.rows, 1)?;
                self.rows.insert((0, 0), row(0, 0, hour));
            }
            Progress::Entries(range) => {
                let entries = &entries[range];
                let rockets = entries
                    .iter()
                    .filter(|entry| !matches!(rocket(entry), Entry::Equipment))
                    .count();
                reserve_map(&mut self.rows, mul(rockets, 2)?)?;
                let since_hour = match self.state {
                    State::Observing { since_hour, .. } => since_hour,
                    State::Withheld(_) => unreachable!(),
                };
                for entry in entries {
                    if let Err(reason) = apply(&mut self.rows, since_hour, rocket(entry)) {
                        self.state = State::Withheld(reason);
                        break;
                    }
                }
            }
            Progress::None => {}
        }
        Ok(())
    }

    pub fn publication(
        &self,
        production_lines: &TableInput,
    ) -> io::Result<Result<Vec<RocketProductionOutputRow>, &'static str>> {
        if let State::Withheld(reason) = self.state {
            return Ok(Err(reason));
        }
        if !self.complete {
            return Ok(Err("rocket events before this sample are still in the log"));
        }
        if matches!(production_lines,
            TableInput::Sampled(TableView::Unavailable { reason })
                if reason == replay_data::cadence::SKIPPED_REASON)
        {
            return Ok(Err(replay_data::cadence::SKIPPED_REASON));
        }
        let mut rows = collect_vec(self.rows.values().cloned())?;
        rows.sort_unstable_by_key(|row| (row.country, row.variant));
        Ok(Ok(rows))
    }
}

fn row(country: u32, variant: u64, since_hour: i32) -> RocketProductionOutputRow {
    RocketProductionOutputRow {
        country,
        variant,
        since_hour,
        exact: true,
        units_exact: true,
        ..Default::default()
    }
}

fn apply(
    rows: &mut HashMap<(u32, u64), RocketProductionOutputRow>,
    since_hour: i32,
    entry: Entry,
) -> Result<(), &'static str> {
    let (country, variant) = match entry {
        Entry::Equipment => return Ok(()),
        Entry::Spending(tick) => (tick.country, tick.variant),
        Entry::Delivery {
            country, variant, ..
        } => (country, variant),
    };
    let world = rows.get_mut(&(0, 0)).expect("branch seeded world coverage");
    count(world, entry)?;
    match entry {
        Entry::Equipment => unreachable!(),
        Entry::Spending(_) => world.exact &= country != 0,
        Entry::Delivery { .. } => world.units_exact &= country != 0,
    }
    if country != 0 {
        let scope = rows
            .entry((country, 0))
            .or_insert_with(|| row(country, 0, since_hour));
        count(scope, entry)?;
        match entry {
            Entry::Equipment => unreachable!(),
            Entry::Spending(tick) => {
                scope.exact &= tick.failure == RocketFailure::None && variant != 0;
            }
            Entry::Delivery {
                units, coherent, ..
            } => {
                scope.units_exact &= coherent && units.is_some() && variant != 0;
            }
        }
    }
    if variant == 0 {
        return Ok(());
    }
    let design = rows
        .entry((country, variant))
        .or_insert_with(|| row(country, variant, since_hour));
    count(design, entry)?;
    let exact = match entry {
        Entry::Equipment => unreachable!(),
        Entry::Spending(tick) => {
            let known = tick.failure == RocketFailure::None && country != 0;
            let spent = design.spent_raw.checked_add(tick.spent_raw);
            let instant = design.instant_raw.checked_add(tick.instant_raw);
            if let Some(spent) = spent {
                design.spent_raw = spent;
            }
            if let Some(instant) = instant {
                design.instant_raw = instant;
            }
            design.exact &= known && spent.is_some() && instant.is_some();
            design.exact
        }
        Entry::Delivery {
            units, coherent, ..
        } => {
            let sum = units.and_then(|units| design.units.checked_add(i64::from(units)));
            if let Some(sum) = sum {
                design.units = sum;
            }
            design.units_exact &= coherent && country != 0 && sum.is_some();
            design.units_exact
        }
    };
    if country != 0 {
        let scope = rows.get_mut(&(country, 0)).unwrap();
        match entry {
            Entry::Equipment => unreachable!(),
            Entry::Spending(_) => scope.exact &= exact,
            Entry::Delivery { .. } => scope.units_exact &= exact,
        }
    }
    Ok(())
}

fn count(row: &mut RocketProductionOutputRow, entry: Entry) -> Result<(), &'static str> {
    let counters: [(&mut u64, u64); 4] = match entry {
        Entry::Equipment => return Ok(()),
        Entry::Spending(tick) => [
            (&mut row.ticks, 1),
            (
                &mut row.inexact_ticks,
                u64::from(
                    tick.failure != RocketFailure::None || tick.country == 0 || tick.variant == 0,
                ),
            ),
            (&mut row.unknown_design_ticks, u64::from(tick.variant == 0)),
            (&mut row.unknown_country_ticks, u64::from(tick.country == 0)),
        ],
        Entry::Delivery {
            country,
            variant,
            units,
            coherent,
        } => {
            row.deliveries = row
                .deliveries
                .checked_add(1)
                .ok_or("rocket delivery counter overflow")?;
            row.inexact_deliveries = row
                .inexact_deliveries
                .checked_add(u64::from(
                    !coherent || units.is_none() || country == 0 || variant == 0,
                ))
                .ok_or("rocket inexact delivery counter overflow")?;
            return Ok(());
        }
    };
    for (total, amount) in counters {
        *total = total
            .checked_add(amount)
            .ok_or("rocket spending counter overflow")?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "rocket_output/tests.rs"]
mod tests;
