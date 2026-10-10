use super::*;
use std::cell::RefCell;

struct Image {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}
impl Memory for Image {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault == Some(address) {
            return false;
        }
        if address == cmap::SINGLETON_RVA && output.len() == 8 {
            output.copy_from_slice(&0x2000_u64.to_le_bytes());
            return true;
        }
        let Some(bytes) = address
            .checked_add(output.len())
            .and_then(|end| self.bytes.get(address..end))
        else {
            return false;
        };
        output.copy_from_slice(bytes);
        true
    }
}
fn put<T: AsRef<[u8]>>(bytes: &mut [u8], at: usize, value: T) {
    let value = value.as_ref();
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn header(bytes: &mut [u8], at: usize, pointer: u64, count: u32) {
    put(bytes, at, pointer.to_le_bytes());
    put(bytes, at + 8, count.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}
fn image() -> Image {
    let mut bytes = vec![0; 0x9000];
    header(
        &mut bytes,
        0x1000 + cgame_state::STRATEGIC_REGIONS,
        0x2800,
        3,
    );
    put(&mut bytes, 0x2000 + cmap::WIDTH, 1024_i32.to_le_bytes());
    put(&mut bytes, 0x2808, 0x3000_u64.to_le_bytes());
    put(&mut bytes, 0x2810, 0x4000_u64.to_le_bytes());
    for (object, id) in [(0x3000, 1_u32), (0x4000, 2)] {
        put(
            &mut bytes,
            object,
            (cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, object + cstrategic_region::ID, id.to_le_bytes());
        put(
            &mut bytes,
            object + POSITION,
            (i32::MIN + id as i32).to_le_bytes(),
        );
        put(&mut bytes, object + POSITION + 4, i32::MAX.to_le_bytes());
    }
    header(&mut bytes, 0x3000 + NEIGHBOURS, 0x5000, 1);
    header(&mut bytes, 0x3000 + ADJACENCIES, 0x5100, 1);
    put(&mut bytes, 0x5000, 0x4000_u64.to_le_bytes());
    bytes[0x5100] = 2;
    header(&mut bytes, 0x5110, 0x5200, 3);
    for address in [0x5200, 0x5208] {
        put(&mut bytes, address, 0x6000_u64.to_le_bytes());
    }
    put(&mut bytes, 0x5210, 0x6010_u64.to_le_bytes());
    put(&mut bytes, 0x6000, 0x7000_u64.to_le_bytes());
    put(&mut bytes, 0x7000, (RULE_VTABLE as u64).to_le_bytes());
    put(&mut bytes, 0x7038, b"RULE");
    put(&mut bytes, 0x7048, 4_u64.to_le_bytes());
    put(&mut bytes, 0x7050, 15_u64.to_le_bytes());
    Image {
        bytes,
        reads: Default::default(),
        fault: None,
    }
}

type Acquisition = (
    Capture,
    Vec<Region>,
    Vec<Edge>,
    Vec<Option<u32>>,
    Vec<Name>,
    Vec<u8>,
);

fn acquire(memory: &Image, region_capacity: usize, byte_capacity: usize) -> Acquisition {
    let mut regions = vec![Region::default(); region_capacity];
    let mut edges = vec![Edge::default(); 1];
    let mut occurrences = vec![None; 3];
    let mut names = vec![Name::default(); 3];
    let mut bytes = vec![0; byte_capacity];
    let mut region_controls = (0..region_capacity)
        .map(|_| RegionControl::default())
        .collect::<Vec<_>>();
    let mut name_controls = (0..3).map(|_| NameControl::default()).collect::<Vec<_>>();
    let mut region_slots = vec![
        PointerSlot::default();
        crate::transport_ledger_observation::index_capacity(region_capacity)
            .unwrap()
    ];
    let mut name_slots = vec![PointerSlot::default(); 8];
    let result = capture(
        memory,
        0,
        0x1000,
        Output {
            regions: &mut regions,
            edges: &mut edges,
            occurrences: &mut occurrences,
            names: &mut names,
            bytes: &mut bytes,
            region_controls: &mut region_controls,
            region_slots: &mut region_slots,
            name_controls: &mut name_controls,
            name_slots: &mut name_slots,
        },
    );
    assert!(
        region_controls
            .iter()
            .all(|control| control.pointer == 0 && control.neighbours.is_none())
    );
    assert!(name_controls.iter().all(|control| control.value.is_none()));
    let cleared = PointerIndex::new(&mut name_slots, 3).unwrap();
    assert!(cleared.find(0x7000).is_none());
    (result, regions, edges, occurrences, names, bytes)
}

#[test]
fn live_sizing_includes_repeated_rule_descriptors_and_obeys_native_skips() {
    let mut memory = image();
    assert_eq!(
        demand(&memory, 0, 0x1000).unwrap(),
        Demand {
            regions: 3,
            edges: 1,
            occurrences: 3,
            names: 3,
            bytes: 8
        }
    );
    memory.bytes[0x5100] = 0;
    memory.fault = Some(0x5158);
    assert_eq!(
        demand(&memory, 0, 0x1000).unwrap(),
        Demand {
            regions: 3,
            edges: 1,
            ..Default::default()
        }
    );
    memory.bytes[0x5100] = 2;
    assert!(demand(&memory, 0, 0x1000).is_err());
}

#[test]
fn original_order_signed_centroids_nullable_rule_repeats_and_one_name_attempt_survive() {
    let memory = image();
    let (result, regions, edges, occurrences, names, bytes) = acquire(&memory, 3, 4);
    assert_eq!(result.width, Ok(1024));
    assert_eq!(
        result.lengths.unwrap(),
        Demand {
            regions: 3,
            edges: 1,
            occurrences: 3,
            names: 1,
            bytes: 4
        }
    );
    assert!(!regions[0].present);
    assert_eq!(regions[1].position, [i32::MIN + 1, i32::MAX]);
    assert_eq!(edges[0].to, 2);
    assert_eq!(occurrences, [Some(0), Some(0), None]);
    assert_eq!(names[0], Name { first: 0, count: 4 });
    assert_eq!(bytes, b"RULE");
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0x7038)
            .count(),
        1
    );
}

#[test]
fn original_fault_is_memoized_and_all_known_demands_retire_on_capacity_or_identity_failure() {
    let mut memory = image();
    memory.fault = Some(0x7038);
    let (result, ..) = acquire(&memory, 3, 4);
    assert!(result.lengths.is_err());
    assert_eq!(result.required.occurrences, 3);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0x7038)
            .count(),
        1
    );
    let memory = image();
    let (result, ..) = acquire(&memory, 2, 4);
    assert_eq!(result.required.regions, 3);
    assert_eq!(result.required.edges, 1);
    assert!(result.lengths.is_err());
    let (result, ..) = acquire(&image(), 3, 0);
    assert_eq!(result.required.bytes, 4);
    assert_eq!(result.required.names, 3);
    assert!(result.lengths.is_err());
    let mut memory = image();
    memory.bytes[0x7050] = 1;
    assert!(acquire(&memory, 3, 4).0.lengths.is_err());
}

#[test]
fn unrestricted_and_nonsea_edges_do_not_demand_pointed_rule_forms() {
    for (flags, unrestricted) in [(2, 17), (1, 0)] {
        let mut memory = image();
        memory.bytes[0x5100] = flags;
        memory.bytes[0x5158] = unrestricted;
        memory.fault = Some(0x5110);
        let (result, ..) = acquire(&memory, 3, 0);
        assert!(result.lengths.is_ok());
        assert_eq!(result.required.occurrences, 0);
        assert!(memory.reads.borrow().iter().all(|&(at, _)| at != 0x5110));
    }
}

#[test]
fn refused_map_width_preserves_independently_complete_original_graph() {
    let mut memory = image();
    memory.fault = Some(0x2000 + cmap::WIDTH);
    let (result, regions, edges, occurrences, ..) = acquire(&memory, 3, 4);
    assert!(result.width.is_err());
    assert!(result.lengths.is_ok());
    assert!(regions[1].present);
    assert_eq!(edges[0].to, 2);
    assert_eq!(occurrences, [Some(0), Some(0), None]);
}

#[test]
fn original_spotting_region_loan_survives_independent_search_header_refusal() {
    struct Observed {
        count: Fact<usize>,
        regions: Vec<(
            u32,
            Fact<Option<crate::transport_spotting_context::RegionOriginal>>,
        )>,
    }
    impl SpottingObserver for Observed {
        fn begin(&mut self, count: Fact<usize>) {
            self.count = count;
        }
        fn region(
            &mut self,
            slot: u32,
            original: Fact<Option<crate::transport_spotting_context::RegionOriginal>>,
        ) {
            self.regions.push((slot, original));
        }
    }
    let mut memory = image();
    memory.fault = Some(0x3000 + NEIGHBOURS);
    let mut regions = vec![Region::default(); 3];
    let mut edges = [Edge::default(); 1];
    let mut occurrences = [None; 3];
    let mut names = [Name::default(); 3];
    let mut bytes = [0; 4];
    let mut controls = (0..3).map(|_| RegionControl::default()).collect::<Vec<_>>();
    let mut name_controls = (0..3).map(|_| NameControl::default()).collect::<Vec<_>>();
    let mut region_slots = [PointerSlot::default(); 8];
    let mut name_slots = [PointerSlot::default(); 8];
    let mut observed = Observed {
        count: Err("unattempted"),
        regions: Vec::new(),
    };
    let result = capture_with_spotting(
        &memory,
        0,
        0x1000,
        Output {
            regions: &mut regions,
            edges: &mut edges,
            occurrences: &mut occurrences,
            names: &mut names,
            bytes: &mut bytes,
            region_controls: &mut controls,
            region_slots: &mut region_slots,
            name_controls: &mut name_controls,
            name_slots: &mut name_slots,
        },
        &mut observed,
    );
    assert!(result.lengths.is_err());
    assert_eq!(observed.count, Ok(3));
    assert_eq!(observed.regions.len(), 3);
    assert!(observed.regions[0].1.unwrap().is_none());
    assert_eq!(observed.regions[1].1.unwrap().unwrap().id, 1);
    assert_eq!(observed.regions[2].1.unwrap().unwrap().id, 2);
    for object in [0x3000, 0x4000] {
        for offset in [
            0,
            cstrategic_region::ID,
            cstrategic_region::PROVINCES + 12,
            cstrategic_region::RUNTIME_SLOT,
        ] {
            assert_eq!(
                memory
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| at == object + offset)
                    .count(),
                1
            );
        }
    }
    assert!(
        controls
            .iter()
            .all(|control| control.pointer == 0 && control.neighbours.is_none())
    );
}

#[test]
fn worker_graph_grows_original_headers_and_descriptors_before_publication() {
    let memory = image();
    let expected = acquire(&memory, 3, 64);
    assert!(expected.0.lengths.is_ok());
    let mut regions = Vec::new();
    let mut edges = Vec::new();
    let mut occurrences = Vec::new();
    let mut names = Vec::new();
    let mut bytes = Vec::new();
    let mut region_controls = Vec::new();
    let mut name_controls = Vec::new();
    let mut region_slots = Vec::new();
    let mut name_slots = Vec::new();
    let result = capture_growing(
        &memory,
        0,
        0x1000,
        GrowingOutput {
            regions: &mut regions,
            edges: &mut edges,
            occurrences: &mut occurrences,
            names: &mut names,
            bytes: &mut bytes,
            region_controls: &mut region_controls,
            region_slots: &mut region_slots,
            name_controls: &mut name_controls,
            name_slots: &mut name_slots,
        },
    );
    assert_eq!(result.lengths, expected.0.lengths);
    let lengths = result.lengths.unwrap();
    assert_eq!(&regions[..lengths.regions], &expected.1[..lengths.regions]);
    assert_eq!(&edges[..lengths.edges], &expected.2[..lengths.edges]);
    assert_eq!(
        &occurrences[..lengths.occurrences],
        &expected.3[..lengths.occurrences]
    );
    assert_eq!(&names[..lengths.names], &expected.4[..lengths.names]);
    assert_eq!(&bytes[..lengths.bytes], &expected.5[..lengths.bytes]);
    assert!(region_controls.iter().all(|control| control.pointer == 0));
    assert!(name_controls.iter().all(|control| control.value.is_none()));
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x1000 + cgame_state::STRATEGIC_REGIONS)
            .count(),
        2
    );
}

#[test]
fn worker_graph_retains_five_thousand_mod_rules_and_later_longer_names() {
    const SLOTS: usize = 5001;
    let mut memory = Image {
        bytes: vec![0; 0x80_0000],
        reads: RefCell::default(),
        fault: None,
    };
    header(
        &mut memory.bytes,
        0x1000 + cgame_state::STRATEGIC_REGIONS,
        0x2800,
        SLOTS as u32,
    );
    put(
        &mut memory.bytes,
        0x2000 + cmap::WIDTH,
        1024_i32.to_le_bytes(),
    );
    for slot in 1..SLOTS {
        let region = 0x10000 + slot * 0x200;
        let lists = 0x40_0000 + slot * 0x100;
        let rule = 0x60_0000 + slot * 0x100;
        put(
            &mut memory.bytes,
            0x2800 + slot * 8,
            (region as u64).to_le_bytes(),
        );
        put(
            &mut memory.bytes,
            region,
            (cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut memory.bytes,
            region + cstrategic_region::ID,
            (slot as u32).to_le_bytes(),
        );
        header(&mut memory.bytes, region + NEIGHBOURS, lists as u64, 1);
        header(
            &mut memory.bytes,
            region + ADJACENCIES,
            (lists + 0x10) as u64,
            1,
        );
        put(&mut memory.bytes, lists, (0x10200_u64).to_le_bytes());
        put(&mut memory.bytes, lists + 0x10, [2_u8]);
        header(&mut memory.bytes, lists + 0x20, (lists + 0x80) as u64, 1);
        put(
            &mut memory.bytes,
            lists + 0x80,
            ((lists + 0xb0) as u64).to_le_bytes(),
        );
        put(&mut memory.bytes, lists + 0xb0, (rule as u64).to_le_bytes());
        put(&mut memory.bytes, rule, (RULE_VTABLE as u64).to_le_bytes());
        put(
            &mut memory.bytes,
            rule + 0x38 + crate::msvc_string::CAPACITY,
            15_u64.to_le_bytes(),
        );
    }
    let mut regions = Vec::new();
    let mut edges = Vec::new();
    let mut occurrences = Vec::new();
    let mut names = Vec::new();
    let mut bytes = Vec::new();
    let mut region_controls = Vec::new();
    let mut name_controls = Vec::new();
    let mut region_slots = Vec::new();
    let mut name_slots = Vec::new();
    for text_len in [3, 15] {
        for slot in 1..SLOTS {
            let at = 0x60_0000 + slot * 0x100 + 0x38;
            put(&mut memory.bytes, at, vec![b'x'; text_len]);
            put(
                &mut memory.bytes,
                at + crate::msvc_string::LENGTH,
                (text_len as u64).to_le_bytes(),
            );
        }
        let result = capture_growing(
            &memory,
            0,
            0x1000,
            GrowingOutput {
                regions: &mut regions,
                edges: &mut edges,
                occurrences: &mut occurrences,
                names: &mut names,
                bytes: &mut bytes,
                region_controls: &mut region_controls,
                region_slots: &mut region_slots,
                name_controls: &mut name_controls,
                name_slots: &mut name_slots,
            },
        );
        assert_eq!(
            result.lengths,
            Ok(Demand {
                regions: SLOTS,
                edges: SLOTS - 1,
                occurrences: SLOTS - 1,
                names: SLOTS - 1,
                bytes: (SLOTS - 1) * text_len
            })
        );
        assert!(
            bytes[..(SLOTS - 1) * text_len]
                .iter()
                .all(|&byte| byte == b'x')
        );
    }
}

#[test]
fn worker_graph_growth_preserves_original_faults_and_read_order() {
    let clean = image();
    assert!(acquire(&clean, 3, 64).0.lengths.is_ok());
    let mut fields = clean
        .reads
        .borrow()
        .iter()
        .map(|&(at, _)| at)
        .collect::<Vec<_>>();
    fields.sort_unstable();
    fields.dedup();
    for fault in fields.into_iter().map(Some).chain([None]) {
        let mut memory = image();
        memory.fault = fault;
        let expected = acquire(&memory, 3, 64).0;
        let original_reads = memory.reads.take();
        let mut regions = Vec::new();
        let mut edges = Vec::new();
        let mut occurrences = Vec::new();
        let mut names = Vec::new();
        let mut bytes = Vec::new();
        let mut region_controls = Vec::new();
        let mut name_controls = Vec::new();
        let mut region_slots = Vec::new();
        let mut name_slots = Vec::new();
        let observed = capture_growing(
            &memory,
            0,
            0x1000,
            GrowingOutput {
                regions: &mut regions,
                edges: &mut edges,
                occurrences: &mut occurrences,
                names: &mut names,
                bytes: &mut bytes,
                region_controls: &mut region_controls,
                region_slots: &mut region_slots,
                name_controls: &mut name_controls,
                name_slots: &mut name_slots,
            },
        );
        assert_eq!(observed.lengths, expected.lengths, "fault {fault:?}");
        assert_eq!(observed.required, expected.required, "fault {fault:?}");
        assert_eq!(observed.width, expected.width, "fault {fault:?}");
        assert_eq!(*memory.reads.borrow(), original_reads, "fault {fault:?}");
        assert!(
            region_slots
                .iter()
                .chain(&name_slots)
                .all(|slot| *slot == PointerSlot::default())
        );
        assert!(region_controls.iter().all(|control| control.pointer == 0));
        assert!(name_controls.iter().all(|control| control.value.is_none()));
    }
}
