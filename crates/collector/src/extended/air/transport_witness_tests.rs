use super::*;
use engine_native::creference_registry;
use std::cell::Cell;

#[derive(Default)]
struct NullBases(Cell<usize>);
impl Memory for NullBases {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.set(self.0.get() + 1);
        if address == creference_registry::LOW_ROOTS_RVA + cair_base::TYPE as usize * 8
            && destination.len() == 8
        {
            destination.fill(0);
            true
        } else {
            false
        }
    }
}

#[test]
fn an_unregistered_wing_with_a_selected_mission_still_demands_its_range_origin() {
    let wing = WingIdentity {
        wing: (3_u64 << 32) | 45,
        region: None,
        mission_mask: 1,
        air_base: (5_u64 << 32) | u64::from(cair_base::TYPE),
        ..Default::default()
    };
    let memory = NullBases::default();
    let mut output = [BaseWitness::default(); 1];
    assert_eq!(
        capture_references(&memory, 0, 0, &[wing], Some(&[]), &mut [0; 2], &mut output),
        Ok(Outcome::Rows { len: 1 })
    );
    assert_eq!(output[0].handle, wing.air_base);
    assert_eq!(memory.0.get(), 1);
}

#[test]
fn idle_pool_with_retained_region_is_probed_once_with_or_without_registration() {
    let wings = [WingIdentity {
        wing: (3_u64 << 32) | 45,
        region: Some(9),
        mission_mask: 0,
        air_base: (5_u64 << 32) | u64::from(cair_base::TYPE),
        base_override: (6_u64 << 32) | u64::from(cair_base::TYPE),
        ..Default::default()
    }];
    let registered = [cair_regional_mission::Row {
        wing: wings[0].wing,
        ordinal: 0,
    }];
    for registered in [&[][..], &registered[..]] {
        let memory = NullBases::default();
        let mut handles = [0; 2];
        let mut output = [BaseWitness::default(); 2];
        assert_eq!(
            capture_references(
                &memory,
                0,
                0,
                &wings,
                Some(registered),
                &mut handles,
                &mut output
            ),
            Ok(Outcome::Rows { len: 2 })
        );
        // Two handles, the pool base and the override: a duplicate idle
        // demand for the registered wing would need a third scratch slot and
        // truncate. The pass reads the base registry's root once
        // (`map_air_base_reference::Pass`); it is null, so neither resolves.
        assert_eq!(memory.0.get(), 1, "one registry root read for both probes");
        assert_eq!(
            output.map(|row| row.handle),
            [wings[0].air_base, wings[0].base_override]
        );
    }
    let mut invalid = wings;
    invalid[0].air_base = 0;
    assert_eq!(
        capture_references(
            &NullBases::default(),
            0,
            0,
            &invalid,
            Some(&registered),
            &mut [0; 2],
            &mut [BaseWitness::default(); 2]
        ),
        Err("registered wing has a null pool base")
    );
}
