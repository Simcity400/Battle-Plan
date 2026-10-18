//! Whole-capture allocation-denial checks for borrowed status values.

use crate::status::{DebugValue, DisplayValue, status};
use serde::Serialize;

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() -> std::io::Result<()> {
    use crate::test_support::with_allocations_denied;
    use std::io::{self, Write};

    #[derive(Clone, Copy)]
    struct Label;
    impl std::fmt::Display for Label {
        fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(out, "borrowed \"label\"\np {}", 42)
        }
    }
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn prove(value: &impl Serialize) -> io::Result<()> {
        let mut count = Count(0);
        with_allocations_denied(|| serde_json::to_writer(&mut count, value))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let mut bytes = Vec::new();
        engine_allocation::reserve_exact(&mut bytes, count.0)?;
        with_allocations_denied(|| serde_json::to_writer(&mut bytes, value))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        assert_eq!(bytes.len(), count.0);
        let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        Ok(())
    }
    let projection = crate::projection::ProjectionScratch::default();
    let value = with_allocations_denied(|| {
        status!({
            "debug": DebugValue(replay_data::rows::TableId::Clock),
            "display": DisplayValue(Label),
            "advisor": crate::advisor_changes::status(),
            "portrait": crate::portrait_changes::status(),
            "opinion": crate::opinion_changes::status(),
            "relations": crate::relation_changes::status(),
            "histories": crate::history_changes::status(),
            "flags": crate::flag_changes::status(),
            "buildings": crate::building_changes::status(),
            "advisor_pools": crate::advisor_pool_changes::status(),
            "party_leaders": crate::party_leader_changes::status(),
            "variables": crate::variable_changes::status(),
            "parties": crate::party_changes::status(),
            "wings": crate::wing_changes::status(),
            "roster": crate::roster_changes::status(),
            "political_status": crate::political_status_changes::status(),
            "wars": crate::war_changes::status(),
            "missions": crate::mission_changes::status(),
            "rails": crate::rail_changes::status(),
            "territory": crate::territory_changes::status(),
            "army_stats": crate::army_stat_changes::status(),
            "ship_models": crate::ship_model_changes::status(),
            "province_units": crate::province_unit_changes::status(),
            "country_inputs": crate::country_input_changes::status(),
            "stability_batches": crate::extended::country_stability::status(),
            "raid": crate::raid_changes::status(),
            "naval_trees": crate::naval_tree_changes::status(),
            "raid_intel": crate::extended::raid_intel_carry::status(),
            "unit_leaders": crate::unit_leader_changes::status(),
            "census": crate::census_changes::status(),
            "unit_leader_traits": crate::unit_leader_trait_changes::status(),
            "world_inputs": crate::world_input_changes::status(),
            "retreat_bans": crate::retreat_ban_changes::status(),
            "local_subunits": crate::local_subunit_changes::status(),
            "templates": crate::template_changes::status(),
            "air_base_carry": crate::air_base_carry_changes::status(),
            "areas": crate::area_changes::status(),
            "provinces": crate::province_changes::status(),
            "characters": crate::character_changes::status(),
            "construction": crate::construction_ic::status(),
            "production": crate::line_output::status(),
            "prunes": crate::combat_log_prune::status(),
            "destruction": crate::equipment_destruction::status(),
            "decode": crate::decode::calibration_json(),
            "projection": projection.status(),
        })
    });
    prove(&value)?;
    drop(value);
    #[cfg(feature = "advisor_profile")]
    prove(&with_allocations_denied(
        crate::extended::characters::advisors::profile::status,
    ))?;
    #[cfg(feature = "callback_profile")]
    prove(&with_allocations_denied(crate::callback_profile::status))?;
    #[cfg(feature = "worker_sampler")]
    prove(&with_allocations_denied(crate::worker_sampler::status))?;
    crate::runtime::status_allocation_failure_exercise()?;
    Ok(())
}
