use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::{ccountry, cfleet, cship, ctask_force};

use crate::support::unit_world as fixture;

struct NamedMemory<'a> {
    source: engine_native_memory::memory::SliceMemory<'a>,
    invalid: usize,
    refused: Option<usize>,
}
impl Memory for NamedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.refused == Some(address) {
            return false;
        }
        if address == self.invalid && destination.len() == 8 {
            destination.copy_from_slice(&0xa001_0000_004d_u64.to_le_bytes());
            return true;
        }
        self.source.copy(address, destination)
    }
}

#[test]
fn named_fleet_walk_keeps_all_countries_empty_bytes_and_independent_refusals() {
    const COUNT: usize = 5000;
    let mut world = fixture::build(fixture::Shape {
        countries: COUNT,
        divisions: 0,
        wings: COUNT,
        planes: COUNT,
        ships: 0,
    });
    let pointers = {
        let memory = engine_native_memory::memory::SliceMemory {
            base: world.base,
            bytes: &world.bytes,
        };
        let countries = ccountry::Reader::new(&memory, world.base, world.state).unwrap();
        (1..=COUNT as u32)
            .map(|id| {
                let country = countries.at(id).unwrap().unwrap();
                let data: u64 = read(&memory, country.pointer + ccountry::FLEETS).unwrap();
                read::<u64>(&memory, data as usize).unwrap() as usize
            })
            .collect::<Vec<_>>()
    };
    for (index, pointer) in pointers.iter().copied().enumerate() {
        let start = pointer - world.base;
        let bytes = &mut world.bytes[start..start + 0x140];
        bytes[cfleet::NAME..cfleet::NAME + 2].copy_from_slice(&[0xff, b'x']);
        let length = if index + 1 == COUNT { 0_u64 } else { 2 };
        bytes[cfleet::NAME + length as usize] = 0;
        bytes[cfleet::NAME + 16..cfleet::NAME + 24].copy_from_slice(&length.to_le_bytes());
        bytes[cfleet::NAME + 24..cfleet::NAME + 32].copy_from_slice(&15_u64.to_le_bytes());
        let leader = if index == 0 {
            0xa001_0000_004d
        } else {
            ((index as u64 + 7000) << 32) | 4713
        };
        bytes[cfleet::LEADER..cfleet::LEADER + 8].copy_from_slice(&leader.to_le_bytes());
    }
    let invalid = world.base + engine_native_parts::creference_registry::INVALID_REFERENCE_RVA.0;
    let mut rows = vec![cfleet::Row::default(); COUNT];
    let mut tasks = vec![ctask_force::Row::default(); COUNT];
    let mut ships: Vec<cship::Row> = vec![];
    let mut names = Vec::new();
    let capture = |memory: &NamedMemory<'_>,
                   rows: &mut Vec<cfleet::Row>,
                   tasks: &mut Vec<ctask_force::Row>,
                   ships: &mut Vec<cship::Row>,
                   names: &mut Vec<u8>| {
        cfleet::walk_timed_with_names(
            memory,
            world.base,
            world.state,
            rows,
            tasks,
            ships,
            |_| 0,
            |_, _| {},
            names,
        )
    };
    let memory = NamedMemory {
        source: engine_native_memory::memory::SliceMemory {
            base: world.base,
            bytes: &world.bytes,
        },
        invalid,
        refused: None,
    };
    assert_eq!(
        capture(&memory, &mut rows, &mut tasks, &mut ships, &mut names)
            .capture
            .fleets,
        Outcome::Rows { len: COUNT }
    );
    assert_eq!(rows[0].leader, Some(0));
    assert_eq!(
        &names[rows[1].name_start..rows[1].name_start + rows[1].name_len],
        &[0xff, b'x']
    );
    assert!(rows[COUNT - 1].name_known);
    assert_eq!(rows[COUNT - 1].name_len, 0);
    let refused = NamedMemory {
        refused: Some(pointers[1] + cfleet::LEADER),
        ..memory
    };
    assert_eq!(
        capture(&refused, &mut rows, &mut tasks, &mut ships, &mut names)
            .capture
            .fleets,
        Outcome::Rows { len: COUNT }
    );
    assert_eq!(rows[1].leader, None);
    assert!(rows[1].name_known);
    let refused = NamedMemory {
        refused: Some(pointers[1] + cfleet::NAME),
        ..refused
    };
    assert_eq!(
        capture(&refused, &mut rows, &mut tasks, &mut ships, &mut names)
            .capture
            .fleets,
        Outcome::Rows { len: COUNT }
    );
    assert!(!rows[1].name_known);
    assert_eq!(rows[1].leader, Some((7001_u64 << 32) | 4713));
    assert!(rows[COUNT - 1].name_known);
    assert_eq!(rows[COUNT - 1].owner, COUNT as u32);
}
