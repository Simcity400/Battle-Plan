use super::*;
use engine::creference_registry;
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
        assert_eq!(
            memory.0.get(),
            2,
            "one pool probe plus one override, no duplicate registered idle demand"
        );
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
