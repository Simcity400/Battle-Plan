use engine_native_memory::{Memory, read};
use engine_native::cgame_state;
use engine_production::construction_ic_sites;

pub(super) fn presize(memory: &impl Memory, base: usize) -> Result<(), &'static str> {
    let demand = (|| {
        let state = engine_sites::at(base, cgame_state::SINGLETON_RVA)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("construction output sizing game state unreadable")?;
        if state == 0 {
            return Ok(0);
        }
        construction_ic_sites::events_per_pass(memory, base, state)
    })();
    super::presize(demand.as_ref().copied().unwrap_or(0));
    demand.map(|_| ())
}
