//! Sizing-read contracts the extended capture families share: failed-observation
//! counters, a census outcome's storage demand and the live world they read.

use engine::{Memory, cgame_state, walk::Outcome};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

pub struct DemandFailures {
    count: AtomicU64,
    last_reason: Mutex<Option<&'static str>>,
}

impl Default for DemandFailures {
    fn default() -> Self {
        Self::new()
    }
}

impl DemandFailures {
    pub const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            last_reason: Mutex::new(None),
        }
    }

    /// An unreadable sizing observation adds no hint, preserving the existing
    /// capacities. It is counted separately from a successful empty census.
    pub fn observe<T>(&self, demand: Result<T, &'static str>) -> Result<T, &'static str> {
        match demand {
            Ok(demand) => Ok(demand),
            Err(reason) => {
                *self
                    .last_reason
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
                self.count.fetch_add(1, Ordering::Relaxed);
                Err(reason)
            }
        }
    }

    pub fn diagnostics(&self) -> impl serde::Serialize {
        let last_reason = *self
            .last_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::status::status!({
            "failed_observations": self.count.load(Ordering::Relaxed),
            "last_reason": last_reason,
        })
    }
}

pub fn rows(outcome: Outcome) -> Result<usize, &'static str> {
    match outcome {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("table storage demand overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

/// An absent or empty world is pending, not a failed census or a zero hint.
pub fn demand_game_state(memory: &impl Memory, base: usize) -> Result<Option<usize>, &'static str> {
    let game_state = base
        .checked_add(cgame_state::SINGLETON_RVA)
        .and_then(|at| engine::read::<u64>(memory, at))
        .ok_or("input demand game state unreadable or address overflow")?;
    let state =
        usize::try_from(game_state).map_err(|_| "input demand game state address overflow")?;
    if state == 0 {
        return Ok(None);
    }
    let countries = state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|at| engine::cpdx_array::Array::read(memory, at, 8))
        .ok_or("input demand country array unreadable or invalid")?;
    Ok((countries.count >= 2).then_some(state))
}

/// The air manager is constructed after country roots, then its country-air
/// slots are filled. Its count reaches the country count before filling slots.
/// A null manager or shorter roster therefore leaves these hints pending.
pub fn demand_air_game_state(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<usize>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let manager = state
        .checked_add(cgame_state::STRATEGIC_AIR_MANAGER)
        .and_then(|at| engine::read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("input demand strategic air manager unreadable")?;
    if manager == 0 {
        return Ok(None);
    }
    let slots = manager
        .checked_add(engine::cstrategic_air_manager::COUNTRY_AIR)
        .and_then(|at| engine::cpdx_array::Array::read(memory, at, 8))
        .ok_or("input demand strategic air country roster unreadable or invalid")?;
    let countries = state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|at| engine::cpdx_array::Array::read(memory, at, 8))
        .ok_or("input demand country array unreadable or invalid")?;
    Ok((countries.count >= 2 && slots.count >= countries.count).then_some(state))
}
