use super::*;

const LOCATION: usize = 0x1000;
const REGION: usize = 0x2000;

struct Traced {
    bytes: Vec<u8>,
    reads: std::cell::RefCell<Vec<usize>>,
}

impl Memory for Traced {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        engine_native::memory::SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

fn located_at(region: usize, id: u32) -> Traced {
    let mut bytes = vec![0; 0x3000];
    let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
    put(
        LOCATION + cprovince::STRATEGIC_REGION,
        &(region as u64).to_le_bytes(),
    );
    put(
        region,
        &(engine_native::cstrategic_region::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        region + engine_native::cstrategic_region::ID,
        &id.to_le_bytes(),
    );
    Traced {
        bytes,
        reads: Default::default(),
    }
}

fn transfer(location: usize) -> DivisionSeed {
    DivisionSeed {
        division: 1,
        logical_owner: Some(7),
        location: Some(location),
        ..Default::default()
    }
}

fn region(id: u32) -> TransportRegion {
    TransportRegion {
        id,
        pointer: REGION + id as usize * 0x100,
    }
}

#[test]
fn a_locations_region_is_read_every_hour_and_authenticated_once() {
    let mut transport = Transport::default();
    let memory = located_at(REGION, 5);
    let held = TransportRegion {
        id: 5,
        pointer: REGION,
    };
    transport.begin(2).unwrap();
    transport.push(&memory, 0, 0, &transfer(LOCATION));
    transport.push(&memory, 0, 1, &transfer(LOCATION));
    assert_eq!(transport.pending, [(0, Ok((7, held))), (1, Ok((7, held)))]);

    let moved = REGION + 0x400;
    let memory = located_at(moved, 9);
    transport.begin(2).unwrap();
    transport.push(&memory, 0, 0, &transfer(LOCATION));
    assert!(memory.reads.borrow().contains(&moved));
    assert_eq!(
        transport.pending,
        [(
            0,
            Ok((
                7,
                TransportRegion {
                    id: 9,
                    pointer: moved,
                }
            ))
        )]
    );

    transport.push(&memory, 0, 1, &transfer(0x2800));
    assert_eq!(
        transport.pending[1],
        (1, Err("division transport source region absent"))
    );
    assert!(!transport.regions.contains_key(&0));
}

fn answer(owner: u32, region: u32) -> &'static str {
    match (owner, region) {
        (7, 3) => "owner 7 in region 3",
        (7, 1) => "owner 7 in region 1",
        (2, 3) => "owner 2 in region 3",
        _ => "unexpected query",
    }
}

#[test]
fn each_owner_is_one_query_over_its_distinct_regions() {
    let mut transport = Transport::default();
    transport.begin(5).unwrap();
    for (index, entry) in [
        (0, Ok((7, region(3)))),
        (1, Ok((2, region(3)))),
        (2, Err("division transport owner unavailable")),
        (3, Ok((7, region(1)))),
        (4, Ok((7, region(3)))),
    ] {
        transport.pending.push((index, entry));
    }
    let mut queries = Vec::new();
    let mut emitted = Vec::new();
    transport.acquire_with(
        |owner, regions, wanted, _, emit| {
            for &at in wanted {
                emit(regions[at], Err(answer(owner, regions[at].id)));
            }
            queries.push((
                owner,
                wanted.iter().map(|&at| regions[at].id).collect::<Vec<_>>(),
            ));
        },
        |index, inputs| emitted.push((index, inputs.err())),
    );
    assert_eq!(queries, [(2, vec![3]), (7, vec![1, 3])]);
    assert_eq!(
        emitted,
        [
            (0, Some("owner 7 in region 3")),
            (1, Some("owner 2 in region 3")),
            (2, Some("division transport owner unavailable")),
            (3, Some("owner 7 in region 1")),
            (4, Some("owner 7 in region 3")),
        ]
    );
}
