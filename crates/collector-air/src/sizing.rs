use collector_extended_support::demand::demand_game_state;
use engine_native::Memory;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(any(test, feature = "test-support"))]
pub fn hint_guard() -> std::sync::MutexGuard<'static, ()> {
    let guard = collector_primitives::country_slots::hint_guard();
    AIR_WING_NAME_BYTES.store(0, Ordering::Relaxed);
    for hint in &CARRIER_HINT {
        hint.store(0, Ordering::Relaxed);
    }
    guard
}

pub fn observe_game_score(
    memory: &impl Memory,
    base: usize,
) -> std::io::Result<Option<engine_native_air::game_score_inputs::ArenaSizing>> {
    match demand_game_state(memory, base) {
        Ok(Some(state)) => {
            engine_native_air::game_score_inputs::arena_demand(memory, base, state).map(Some)
        }
        Ok(None) => Ok(None),
        Err(reason) => engine_native_air::game_score_inputs::ArenaSizing::unavailable(reason).map(Some),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AceNamesDemand {
    pub aces: usize,
    pub ace_bytes: usize,
}

pub fn observe_ace_names(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<AceNamesDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let walked = engine_native_air::cace::count(memory, base, state);
    let aces = match walked.outcome {
        engine_native::walk::Outcome::Rows { len } => len,
        engine_native::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("ace roster sizing overflow")?,
        engine_native::walk::Outcome::Unavailable { reason } => return Err(reason),
    };
    Ok(Some(AceNamesDemand {
        aces,
        ace_bytes: walked.required_bytes,
    }))
}

static AIR_WING_NAME_BYTES: AtomicUsize = AtomicUsize::new(0);
static CARRIER_HINT: [AtomicUsize; 12] = [const { AtomicUsize::new(0) }; 12];

pub fn carrier_demand() -> engine_native_air::cair_base::CarrierDemand {
    let [
        bases,
        registrations,
        pools,
        indexed,
        resolved,
        country_terms,
        aces,
        forecasts,
        countries,
        name_bytes,
        raw_spans,
        raw_bytes,
    ] = std::array::from_fn(|index| CARRIER_HINT[index].load(Ordering::Relaxed));
    engine_native_air::cair_base::CarrierDemand {
        bases,
        registrations,
        pools,
        indexed,
        resolved,
        country_terms,
        aces,
        forecasts,
        countries,
        name_bytes,
        raw_spans,
        raw_bytes,
        ..Default::default()
    }
}

pub fn note_carrier_demand(demand: engine_native_air::cair_base::CarrierDemand) -> bool {
    let mut changed = false;
    for (hint, count) in CARRIER_HINT.iter().zip([
        demand.bases,
        demand.registrations,
        demand.pools,
        demand.indexed,
        demand.resolved,
        demand.country_terms,
        demand.aces,
        demand.forecasts,
        demand.countries,
        demand.name_bytes,
        demand.raw_spans,
        demand.raw_bytes,
    ]) {
        changed |= hint.fetch_max(count, Ordering::Relaxed) < count;
    }
    changed
}

pub fn air_wing_name_bytes() -> usize {
    AIR_WING_NAME_BYTES.load(Ordering::Relaxed)
}

pub fn note_air_wing_name_bytes(bytes: usize) {
    AIR_WING_NAME_BYTES.fetch_max(bytes, Ordering::Relaxed);
}

pub fn observe_air_wing_names(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<usize>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_native_air::cair_wing::name_demand(memory, base, state).map(Some)
}
