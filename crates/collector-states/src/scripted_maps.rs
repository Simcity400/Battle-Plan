use std::sync::OnceLock;

#[derive(Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    pub victory_point: bool,
    pub sharing_roster: bool,
    pub all_controllers: bool,
    pub controller_states: Vec<u32>,
}
static DEMAND: OnceLock<Demand> = OnceLock::new();

pub fn configure(mut demand: Demand) {
    demand.controller_states.sort_unstable();
    demand.controller_states.dedup();
    let _ = DEMAND.set(demand);
}

pub fn state(id: u32) -> (bool, bool) {
    DEMAND.get().map_or((false, false), |demand| {
        (
            demand.victory_point,
            demand.all_controllers || demand.controller_states.binary_search(&id).is_ok(),
        )
    })
}
