//! Native pass hints allocate only off-thread; failed hints retain every floor.
use engine_native::{Memory, cgame_state, read};
use engine_production::construction_ic_sites;

static FAILURES: collector_extended_support::demand::DemandFailures =
    collector_extended_support::demand::DemandFailures::new();

pub(super) fn status() -> impl serde::Serialize {
    FAILURES.diagnostics()
}

pub(super) fn presize(memory: &impl Memory, base: usize) -> Result<(), &'static str> {
    let demand = (|| {
        let state = base
            .checked_add(cgame_state::SINGLETON_RVA)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("construction output sizing game state unreadable")?;
        if state == 0 {
            return Ok(0);
        }
        construction_ic_sites::events_per_pass(memory, base, state)
    })();
    let demand = FAILURES.observe(demand);
    // Burst pressure can grow even with unchanged content, or a failed hint.
    super::presize(demand.as_ref().copied().unwrap_or(0));
    demand.map(|_| ())
}
