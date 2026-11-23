use super::*;
use crate::{ccountry, cgame_state, memory::SliceMemory, supply_companion};

const GAME: usize = 0x1000;
const ROOT: usize = 0x2000;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    let offset = address - GAME;
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

fn header(bytes: &mut [u8], address: usize, data: usize, count: u32, capacity: u32) {
    put(bytes, address, &(data as u64).to_le_bytes());
    put(bytes, address + 8, &capacity.to_le_bytes());
    put(bytes, address + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x9000];
    put(
        &mut bytes,
        GAME + cgame_state::SUPPLY_SYSTEM,
        &(ROOT as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ROOT,
        &(csupply_system::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ROOT + 8,
        &(csupply_system::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    header(&mut bytes, GAME + cgame_state::COUNTRIES, 0x2500, 3, 3);
    header(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, 0x2600, 8, 8);
    header(
        &mut bytes,
        ROOT + csupply_system::COUNTRY_SYSTEM_DATA,
        0x2700,
        3,
        7,
    );
    header(
        &mut bytes,
        ROOT + csupply_system::COUNTRY_COMPANION_DATA,
        0x6000,
        3,
        3,
    );
    for (slot, country, system, id) in [
        (1_u32, 0x3000_usize, 0x5000_usize, 1_u32),
        (2, 0x4000, 0x5300, 7),
    ] {
        put(
            &mut bytes,
            0x2500 + slot as usize * 8,
            &(country as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2700 + slot as usize * 8,
            &(system as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, country + ccountry::ID, &id.to_le_bytes());
        put(&mut bytes, 0x2600 + slot as usize * 4, &slot.to_le_bytes());
        put(&mut bytes, 0x2600 + id as usize * 4, &slot.to_le_bytes());
        put(&mut bytes, system, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            system + SECONDARY_BASE,
            &(SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + SUPPLY_SYSTEM,
            &(ROOT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + COUNTRY,
            &(country as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + COMPANION,
            &((0x6000 + slot as usize * supply_companion::STRIDE) as u64).to_le_bytes(),
        );
    }
    header(&mut bytes, 0x5000 + HUB_SETTINGS_DATA, 0x7000, 2, 2);
    for (node, province, node_type, map, count) in [
        (0x7000, 1001_u32, 15_u32, 0x8000, 2),
        (0x7028, 2002, 99, 0x8020, 1),
    ] {
        put(
            &mut bytes,
            node + supply_node_record::PROVINCE,
            &province.to_le_bytes(),
        );
        put(
            &mut bytes,
            node + supply_node_record::NODE_TYPE,
            &node_type.to_le_bytes(),
        );
        header(
            &mut bytes,
            node + supply_node_record::MOTORIZATION_DATA,
            map,
            count,
            count,
        );
    }
    for (entry, actor, level) in [(0x8000, 2_u32, 0_u8), (0x8008, 1, 2), (0x8020, 7, 1)] {
        put(&mut bytes, entry, &actor.to_le_bytes());
        put(&mut bytes, entry + 4, &[level, 0xaa, 0xbb, 0xcc]);
    }
    bytes
}

struct NoEntryPadding<'a>(&'a [u8]);

impl Memory for NoEntryPadding<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if [0x8005, 0x800d, 0x8025]
            .into_iter()
            .any(|start| address < start + 3 && address.saturating_add(destination.len()) > start)
        {
            return false;
        }
        SliceMemory {
            base: GAME,
            bytes: self.0,
        }
        .copy(address, destination)
    }
}

#[test]
fn floating_harbour_records_keep_slot_and_array_order() {
    let mut bytes = fixture();
    header(
        &mut bytes,
        0x5000 + FLOATING_HARBOR_DEPLOYMENTS,
        0x9000,
        3,
        5,
    );
    header(
        &mut bytes,
        0x5300 + FLOATING_HARBOR_DEPLOYMENTS,
        0x9300,
        1,
        1,
    );
    for (address, province, status) in [
        (0x9000, 77_i32, 0_u8),
        (0x9068, 77, 255),
        (0x90d0, -1, 1),
        (0x9300, 77, 2),
    ] {
        put(
            &mut bytes,
            address + FLOATING_HARBOR_PROVINCE,
            &province.to_le_bytes(),
        );
        put(
            &mut bytes,
            address + FLOATING_HARBOR_STATUS,
            &[status, 0xa5, 0xa5, 0xa5],
        );
    }
    let memory = SliceMemory {
        base: GAME,
        bytes: &bytes,
    };
    let row = |country, ordinal, province, status| FloatingHarborDeployment {
        country,
        ordinal,
        province,
        status,
    };
    let mut rows = vec![FloatingHarborDeployment::default(); 4];
    assert_eq!(
        walk_floating_harbor_deployments(&memory, 0, GAME, &mut rows),
        Outcome::Rows { len: 4 }
    );
    assert_eq!(
        rows,
        [
            row(1, 0, 77, 0),
            row(1, 1, 77, 255),
            row(1, 2, -1, 1),
            row(2, 0, 77, 2)
        ]
    );
}

#[test]
fn settings_preserve_raw_actor_aliases_zero_and_exact_level_byte() {
    let mut bytes = fixture();
    let mut rows = vec![Row::default(); 3];
    assert_eq!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(
        rows[0],
        Row {
            owner: 1,
            actor: 2,
            node_index: 0,
            map_entry_index: 0,
            province: 1001,
            node_type: 15,
            level: 0
        }
    );
    assert_eq!(rows[1].actor, 1);
    assert_eq!(rows[1].level, 2);
    assert_eq!(rows[2].node_index, 1);
    assert_eq!(rows[2].actor, 7);
    assert_eq!(rows[2].map_entry_index, 0);
    put(&mut bytes, 0x8004, &[255]);
    assert_eq!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(rows[0].level, 255);
}

#[test]
fn the_count_probe_counts_settings_across_nodes_and_empty_maps_emit_nothing() {
    let mut bytes = fixture();
    let mut rows = vec![Row::default(); 1];
    assert_eq!(
        count(&NoEntryPadding(&bytes), 0, GAME),
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
    for node in [0x7000, 0x7028] {
        header(
            &mut bytes,
            node + supply_node_record::MOTORIZATION_DATA,
            0,
            0,
            0,
        );
    }
    assert_eq!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn malformed_map_headers_and_actor_links_make_table_unavailable() {
    let mut bytes = fixture();
    let mut rows = vec![Row::default(); 3];
    put(
        &mut bytes,
        0x7000 + supply_node_record::MOTORIZATION_COUNT,
        &3_u32.to_le_bytes(),
    );
    assert!(matches!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Unavailable { .. }
    ));
    put(
        &mut bytes,
        0x7000 + supply_node_record::MOTORIZATION_COUNT,
        &2_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x8008, &0_u32.to_le_bytes());
    assert!(matches!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Unavailable { .. }
    ));
}

fn hash(key: u32) -> usize {
    let mix = |v: u32| (v >> 16 ^ v).wrapping_mul(0x045d_9f3b);
    let v = mix(mix(key));
    ((v >> 16 ^ v) & 7) as usize
}
fn map(
    bytes: &mut [u8],
    at: usize,
    data: usize,
    stride: usize,
    marker: usize,
    keys: &[u32],
) -> Vec<usize> {
    put(bytes, at, &(data as u64).to_le_bytes());
    put(bytes, at + 8, &(keys.len() as u32).to_le_bytes());
    put(bytes, at + 12, &7_u32.to_le_bytes());
    put(bytes, at + 16, &[4]);
    put(bytes, data + 12 * stride + marker, &[255]);
    let mut positions = Vec::new();
    for &key in keys {
        let first = hash(key);
        let mut slot = first;
        while bytes[data + slot * stride + marker - GAME] != 0 {
            slot += 1;
        }
        let bucket = data + slot * stride;
        put(bytes, bucket + marker, &[(slot - first + 1) as u8]);
        put(bytes, bucket + marker + 4, &key.to_le_bytes());
        positions.push(bucket);
    }
    positions
}
fn train_fixture() -> Vec<u8> {
    use crate::{supply_node as node, supply_parent_map, supply_route};
    let mut bytes = fixture();
    bytes.resize(0x20000, 0);
    let companion = 0x6000 + supply_companion::STRIDE;
    let companion_two = 0x6000 + 2 * supply_companion::STRIDE;
    put(
        &mut bytes,
        0x5000 + ALLOCATED_TRUCKS,
        &i32::MIN.to_le_bytes(),
    );
    put(
        &mut bytes,
        companion + supply_companion::REQUIRED_TRUCKS_RAW,
        &0_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        companion + supply_companion::TRUCK_FULFILLMENT_RATIO,
        &100_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        companion + supply_companion::REQUIRED_TRAINS,
        &i32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        companion_two + supply_companion::TRUCK_FULFILLMENT_RATIO,
        &100_000_i64.to_le_bytes(),
    );
    header(
        &mut bytes,
        ROOT + csupply_system::FOREIGN_SOURCE_DATA,
        0x2800,
        3,
        3,
    );
    header(
        &mut bytes,
        0x2800 + csupply_system::FOREIGN_SOURCE_STRIDE + csupply_system::FOREIGN_SOURCE_RELATED_COUNTRIES,
        0x2980,
        1,
        1,
    );
    put(&mut bytes, 0x2980, &1_u32.to_le_bytes());
    header(
        &mut bytes,
        0x2800
            + 2 * csupply_system::FOREIGN_SOURCE_STRIDE
            + csupply_system::FOREIGN_SOURCE_RELATED_COUNTRIES,
        0x2990,
        1,
        1,
    );
    put(&mut bytes, 0x2990, &7_u32.to_le_bytes());
    header(
        &mut bytes,
        companion + supply_companion::NODES,
        0xa000,
        3,
        3,
    );
    for i in 0..3_u32 {
        let at = 0xa000 + i as usize * node::STRIDE;
        put(&mut bytes, at, &i.to_le_bytes());
        put(&mut bytes, at + node::PROVINCE, &(i + 1).to_le_bytes());
        put(&mut bytes, at + node::KIND, &(i + 2).to_le_bytes());
        put(&mut bytes, at + node::FLAG, &[if i == 1 { 255 } else { 0 }]);
        put(
            &mut bytes,
            at + node::AVAILABLE,
            &(-1234567890123_i64 + i64::from(i)).to_le_bytes(),
        );
        put(&mut bytes, at + node::CONSUMED, &i64::MAX.to_le_bytes());
    }
    let parents = map(
        &mut bytes,
        companion + supply_parent_map::HEADER,
        0xb000,
        12,
        0,
        &[2, 3],
    );
    put(&mut bytes, parents[0] + 8, &1_i32.to_le_bytes());
    put(&mut bytes, parents[1] + 8, &2_i32.to_le_bytes());
    let outer = map(
        &mut bytes,
        ROOT + supply_route::ROUTE_MAP,
        0xc000,
        0x30,
        4,
        &[1],
    );
    let inner = map(
        &mut bytes,
        outer[0] + supply_route::INNER_HEADER,
        0xd000,
        0xa0,
        4,
        &[2],
    );
    let route = inner[0] + supply_route::VALUE;
    header(&mut bytes, route, 0xe000, 3, 3);
    put(&mut bytes, route + supply_route::FROM, &1_u16.to_le_bytes());
    put(&mut bytes, route + supply_route::TO, &2_u16.to_le_bytes());
    put(&mut bytes, route + supply_route::BREAK, &[7]);
    for (i, p) in [2_u32, 2, 1].into_iter().enumerate() {
        put(&mut bytes, 0xe000 + i * 4, &p.to_le_bytes());
    }
    header(&mut bytes, GAME + cgame_state::PROVINCES, 0x18000, 4, 4);
    for id in 1..4_u32 {
        let p = 0x19000 + id as usize * 0x400;
        put(
            &mut bytes,
            0x18000 + id as usize * 8,
            &(p as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            p,
            &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, p + crate::cprovince::ID, &id.to_le_bytes());
        put(
            &mut bytes,
            p + crate::cprovince::CONTROLLER,
            &1_u32.to_le_bytes(),
        );
    }
    header(
        &mut bytes,
        companion + supply_companion::ROUTES_SUBTYPE_ZERO,
        0xf200,
        2,
        2,
    );
    for (i, province) in [2_u32, 3].into_iter().enumerate() {
        put(&mut bytes, 0xf200 + i * 8, &province.to_le_bytes());
        put(&mut bytes, 0xf204 + i * 8, &(province - 1).to_le_bytes());
    }
    header(&mut bytes, 0x5000 + ALLOCATED_TRAINS, 0xf000, 2, 2);
    put(&mut bytes, 0xf008, &i32::MIN.to_le_bytes());
    put(&mut bytes, 0xf020, &i32::MAX.to_le_bytes());
    header(
        &mut bytes,
        companion + supply_companion::RELATED_CONTROLLERS,
        0xf100,
        3,
        3,
    );
    put(&mut bytes, 0xf100, &[0, 255, 1]);
    bytes
}
struct TrainMemory<'a> {
    bytes: &'a [u8],
    calls: std::cell::Cell<usize>,
    copied: std::cell::Cell<usize>,
    reject: Option<usize>,
    truck_cost: Option<i64>,
}
impl Memory for TrainMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.copied.set(self.copied.get() + destination.len());
        if self
            .reject
            .is_some_and(|p| p >= address && p < address.saturating_add(destination.len()))
        {
            return false;
        }
        if address == supply_companion::SUPPLY_HUB_FULL_MOTORIZATION_TRUCK_COST_RVA.0
            && destination.len() == 8
        {
            let Some(cost) = self.truck_cost else {
                return false;
            };
            destination.copy_from_slice(&cost.to_le_bytes());
            return true;
        }
        if address == crate::supply_route::EMPTY_RVA.0 + crate::supply_route::FROM {
            destination.fill(0);
            return true;
        }
        if address == crate::creference_registry::LOW_ROOTS_RVA.0 + 70 * 8 {
            destination.copy_from_slice(&0x11000_u64.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: GAME,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}
impl<'a> TrainMemory<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            calls: 0.into(),
            copied: 0.into(),
            reject: None,
            truck_cost: Some(6_000_000),
        }
    }
}
#[test]
fn train_nodes_preserve_native_identity_values_parent_edges_and_count_raw_read_work() {
    let bytes = train_fixture();
    let memory = TrainMemory::new(&bytes);
    let mut nodes = vec![crate::supply_node::Row::default(); 3];
    let mut paths = vec![0; 3];
    let report = crate::supply_node::walk(&memory, 0, GAME, &mut nodes, &mut paths);
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
    assert_eq!((report.paths, report.required_paths), (3, 3));
    assert_eq!(
        nodes
            .iter()
            .map(|n| (n.country, n.index, n.province, n.parent))
            .collect::<Vec<_>>(),
        [(1, 0, 1, -1), (1, 1, 2, 1), (1, 2, 3, 2)]
    );
    assert_eq!(nodes[1].flag, 255);
    assert_eq!(nodes[1].available, -1234567890122);
    assert_eq!(nodes[1].consumed, i64::MAX);
    assert_eq!(nodes[1].route.break_after, 7);
    assert_eq!(paths, [2, 2, 1]);
    assert_eq!(nodes[2].route.from, 0);
    assert_eq!(nodes[2].route.len, 0);
    assert!(memory.calls.get() < 160 && memory.copied.get() < 4000);
}

#[test]
fn unresolved_province_ids_are_recorded_as_the_games_null_province() {
    use crate::{
        supply_node::{self as node, ProvinceCheck, ProvinceObservation, ProvinceRole},
        supply_parent_map,
    };
    let observation = |index, province, role, check| ProvinceObservation {
        country: 1,
        index,
        province,
        role,
        check,
    };
    let walk = |bytes: &[u8], rows: usize| {
        let mut nodes = vec![node::Row::default(); rows];
        let mut paths = vec![0; 3];
        let report = node::walk(&TrainMemory::new(bytes), 0, GAME, &mut nodes, &mut paths);
        let mut rows = nodes
            .iter()
            .map(|n| (n.index, n.province, n.kind, n.parent))
            .collect::<Vec<_>>();
        rows.sort_unstable();
        (report, rows)
    };
    for province in [0_u32, 4, u32::MAX] {
        let mut bytes = train_fixture();
        put(
            &mut bytes,
            0xa000 + 2 * node::STRIDE + node::PROVINCE,
            &province.to_le_bytes(),
        );
        let (report, rows) = walk(&bytes, 3);
        assert_eq!(report.outcome, Outcome::Rows { len: 3 });
        let mut expected = vec![(0, 1, 2, -1), (1, 2, 3, 1), (2, province, 4, -1)];
        expected.sort_unstable();
        assert_eq!(rows, expected);
        assert_eq!(report.null_provinces, 1);
        assert_eq!(
            report.last_null_province,
            Some(observation(
                2,
                province,
                ProvinceRole::Node,
                ProvinceCheck::OutsideCensus
            ))
        );
        assert_eq!(report.refusal, None);
    }
    let mut bytes = train_fixture();
    put(&mut bytes, 0x18000 + 3 * 8, &0_u64.to_le_bytes());
    let (report, rows) = walk(&bytes, 3);
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
    assert_eq!(rows, [(0, 1, 2, -1), (1, 2, 3, 1), (2, 3, 4, 2)]);
    assert_eq!(
        report.last_null_province,
        Some(observation(
            2,
            3,
            ProvinceRole::Node,
            ProvinceCheck::SlotEmpty
        ))
    );
    let reparent = |bytes: &mut Vec<u8>, of_two: i32, of_three: i32| {
        bytes[0xb000 - GAME..0xb000 - GAME + 13 * 12].fill(0);
        let parents = map(
            bytes.as_mut_slice(),
            0x6000 + supply_companion::STRIDE + supply_parent_map::HEADER,
            0xb000,
            12,
            0,
            &[2, 3],
        );
        put(bytes.as_mut_slice(), parents[0] + 8, &of_two.to_le_bytes());
        put(
            bytes.as_mut_slice(),
            parents[1] + 8,
            &of_three.to_le_bytes(),
        );
    };
    let mut bytes = train_fixture();
    reparent(&mut bytes, 1, 4);
    let (report, rows) = walk(&bytes, 4);
    assert_eq!(report.outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        rows,
        [
            (0, 1, 2, -1),
            (1, 2, 3, 1),
            (2, 3, 4, 4),
            (node::VERTEX | 4, 4, 0, -1)
        ]
    );
    assert_eq!(report.null_provinces, 2);
    assert_eq!(
        report.last_null_province,
        Some(observation(
            node::VERTEX | 4,
            4,
            ProvinceRole::Vertex,
            ProvinceCheck::OutsideCensus
        ))
    );
    let mut bytes = train_fixture();
    put(
        &mut bytes,
        0x19000 + 3 * 0x400 + crate::cprovince::ID,
        &9_u32.to_le_bytes(),
    );
    let (report, _) = walk(&bytes, 3);
    assert_eq!(
        report.outcome,
        Outcome::Unavailable {
            reason: "supply node province identity mismatch"
        }
    );
    assert_eq!(
        report.refusal,
        Some(observation(
            2,
            3,
            ProvinceRole::Node,
            ProvinceCheck::IdentityMismatch
        ))
    );
    reparent(&mut bytes, 3, 2);
    let (report, _) = walk(&bytes, 3);
    assert_eq!(
        report.outcome,
        Outcome::Unavailable {
            reason: "supply parent province identity mismatch"
        }
    );
    assert_eq!(
        report.refusal,
        Some(observation(
            1,
            3,
            ProvinceRole::Parent,
            ProvinceCheck::IdentityMismatch
        ))
    );
    let bytes = train_fixture();
    let mut memory = TrainMemory::new(&bytes);
    memory.reject = Some(0x18000 + 3 * 8);
    let mut nodes = vec![node::Row::default(); 3];
    let report = node::walk(&memory, 0, GAME, &mut nodes, &mut vec![0; 3]);
    assert_eq!(
        report.outcome,
        Outcome::Unavailable {
            reason: "supply node province slot unreadable"
        }
    );
    assert_eq!(
        report.refusal,
        Some(observation(
            2,
            3,
            ProvinceRole::Node,
            ProvinceCheck::SlotUnreadable
        ))
    );
}

#[test]
fn node_routes_are_acquired_only_after_the_first_positive_parent() {
    use crate::{supply_node as node, supply_route};
    use std::cell::Cell;

    struct Traced<'a> {
        source: TrainMemory<'a>,
        outer_reads: Cell<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == ROOT + supply_route::ROUTE_MAP {
                self.outer_reads.set(self.outer_reads.get() + 1);
            }
            self.source.copy(address, output)
        }
    }
    let capture = |bytes: &[u8], reject_outer: bool| {
        let mut source = TrainMemory::new(bytes);
        source.reject = reject_outer.then_some(ROOT + supply_route::ROUTE_MAP);
        let memory = Traced {
            source,
            outer_reads: Cell::new(0),
        };
        let mut rows = vec![node::Row::default(); 3];
        let mut paths = vec![0; 3];
        let report = node::walk(&memory, 0, GAME, &mut rows, &mut paths);
        (report.outcome, memory.outer_reads.get(), rows, paths)
    };
    for (count, expected, route_reads) in [
        (0, Outcome::Rows { len: 0 }, 0),
        (1, Outcome::Rows { len: 1 }, 0),
        (
            3,
            Outcome::Unavailable {
                reason: "supply map header unreadable",
            },
            1,
        ),
    ] {
        let mut bytes = train_fixture();
        header(
            &mut bytes,
            0x6000 + supply_companion::STRIDE + supply_companion::NODES,
            0xa000,
            count,
            3,
        );
        let (outcome, reads, rows, _) = capture(&bytes, true);
        assert_eq!(outcome, expected);
        assert_eq!(reads, route_reads);
        if count == 1 {
            assert_eq!((rows[0].province, rows[0].parent), (1, -1));
        }
    }
    let mut bytes = train_fixture();
    let (outcome, reads, _, paths) = capture(&bytes, false);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(reads, 1);
    assert_eq!(paths, [2, 2, 1]);

    put(&mut bytes, 0xa000 + node::INDEX, &99_u32.to_le_bytes());
    let (outcome, reads, _, _) = capture(&bytes, true);
    assert_eq!(
        outcome,
        Outcome::Unavailable {
            reason: "supply node identity differs from array index",
        }
    );
    assert_eq!(reads, 0);
}

#[test]
fn repeated_global_edges_keep_rows_path_occurrences_pressure_and_first_faults() {
    use crate::{supply_node as node, supply_parent_map, supply_route};
    use std::cell::{Cell, RefCell};

    struct Traced<'a> {
        source: TrainMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
        fail: Cell<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let failed = self.reads.borrow().len() == self.fail.get();
            self.reads.borrow_mut().push((address, output.len()));
            !failed && self.source.copy(address, output)
        }
    }
    let capture = |memory: &Traced<'_>, memo: &mut node::ProvinceMemo, size| {
        let mut rows = vec![node::Row::default(); 6];
        let mut paths = vec![0; size];
        memo.begin(memory, 0, GAME);
        let report = node::walk_part(memory, 0, GAME, 0, 1, &mut rows, &mut paths, memo);
        (
            report.outcome,
            report.paths,
            report.required_paths,
            rows,
            paths,
        )
    };
    let mut bytes = train_fixture();
    let first = 0x6000 + supply_companion::STRIDE;
    let second = first + supply_companion::STRIDE;
    header(&mut bytes, second + supply_companion::NODES, 0xa000, 3, 3);
    let parents: [u8; 17] = bytes
        [first + supply_parent_map::HEADER - GAME..first + supply_parent_map::HEADER - GAME + 17]
        .try_into()
        .unwrap();
    put(&mut bytes, second + supply_parent_map::HEADER, &parents);

    let memory = Traced {
        source: TrainMemory::new(&bytes),
        reads: RefCell::default(),
        fail: Cell::new(usize::MAX),
    };
    let mut retained = node::ProvinceMemo::with_scratch().unwrap();
    let baseline = capture(&memory, &mut node::ProvinceMemo::default(), 6);
    let cold = memory.reads.take();
    let actual = capture(&memory, &mut retained, 6);
    let reused = memory.reads.take();
    assert_eq!(actual, baseline);
    assert_eq!(actual.0, Outcome::Rows { len: 6 });
    assert_eq!((actual.1, actual.2), (6, 6));
    assert_eq!(actual.4, [2, 2, 1, 2, 2, 1]);
    assert_eq!((actual.3[1].route.start, actual.3[4].route.start), (0, 3));
    let route = 0xd000 + hash(2) * supply_route::INNER_STRIDE + supply_route::VALUE;
    for observation in [(route + supply_route::FROM, 8), (route, 16), (0xe000, 12)] {
        assert_eq!(cold.iter().filter(|&&read| read == observation).count(), 2);
        assert_eq!(
            reused.iter().filter(|&&read| read == observation).count(),
            1
        );
    }
    assert!(reused.len() < cold.len());
    let retained_bytes = retained.allocated_bytes();
    for size in [0, 2, 3, 5, 6] {
        assert_eq!(
            capture(&memory, &mut retained, size),
            capture(&memory, &mut node::ProvinceMemo::default(), size),
        );
        assert_eq!(retained.allocated_bytes(), retained_bytes);
    }
    memory.reads.borrow_mut().clear();

    let first_country = cold.iter().position(|&(at, _)| at == 0x2710).unwrap();
    assert_eq!(cold[..first_country], reused[..first_country]);
    for failed in 2..first_country {
        memory.fail.set(failed);
        let reference = capture(&memory, &mut node::ProvinceMemo::default(), 6);
        let reference_reads = memory.reads.take();
        let candidate = capture(&memory, &mut retained, 6);
        let candidate_reads = memory.reads.take();
        assert_eq!(candidate.0, reference.0, "failed read {failed}");
        assert_eq!(candidate_reads, reference_reads, "failed read {failed}");
        memory.fail.set(usize::MAX);
        assert_eq!(capture(&memory, &mut retained, 6), baseline);
        memory.reads.borrow_mut().clear();
    }

    put(&mut bytes, 0xe000, &3_u32.to_le_bytes());
    let memory = Traced {
        source: TrainMemory::new(&bytes),
        reads: RefCell::default(),
        fail: Cell::new(usize::MAX),
    };
    let changed = capture(&memory, &mut retained, 6);
    assert_eq!(changed.4, [3, 2, 1, 3, 2, 1]);
    assert_eq!(
        changed,
        capture(&memory, &mut node::ProvinceMemo::default(), 6)
    );
}

#[test]
fn parent_headers_reuse_interleaved_keys_but_refresh_root_and_route_payloads() {
    use crate::supply_route::{self as route, InnerMemo};
    use std::cell::{Cell, RefCell};
    struct Traced<'a> {
        source: TrainMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
        fail: Cell<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let failed = self.reads.borrow().len() == self.fail.get();
            self.reads.borrow_mut().push((address, output.len()));
            !failed && self.source.copy(address, output)
        }
    }
    let mut bytes = train_fixture();
    let other_root = 0x2200;
    let outer = map(
        &mut bytes,
        other_root + route::ROUTE_MAP,
        0x13000,
        route::OUTER_STRIDE,
        4,
        &[2],
    );
    let inner = map(
        &mut bytes,
        outer[0] + route::INNER_HEADER,
        0x14000,
        route::INNER_STRIDE,
        4,
        &[3],
    );
    let other_route = inner[0] + route::VALUE;
    header(&mut bytes, other_route, 0x15000, 2, 2);
    put(&mut bytes, other_route + route::FROM, &2_u16.to_le_bytes());
    put(&mut bytes, other_route + route::TO, &3_u16.to_le_bytes());
    put(&mut bytes, 0x15000, &3_u32.to_le_bytes());
    put(&mut bytes, 0x15004, &2_u32.to_le_bytes());
    let memory = Traced {
        source: TrainMemory::new(&bytes),
        reads: RefCell::default(),
        fail: Cell::new(usize::MAX),
    };
    let outer = route::read_outer(&memory, ROOT).unwrap();
    let other_outer = route::read_outer(&memory, other_root).unwrap();
    memory.reads.borrow_mut().clear();
    let observe = |memo: &mut InnerMemo, parent, province| {
        let mut path = vec![0; 3];
        let result = route::read_edge_buffer(
            &memory,
            0,
            &outer,
            memo,
            parent,
            province,
            crate::walk::Buffer::Growing(&mut path),
        );
        (result, path)
    };
    let mut baseline = InnerMemo::default();
    let expected = [(1, 2), (2, 3), (1, 2), (2, 3)]
        .map(|(parent, province)| observe(&mut baseline, parent, province));
    let cold = memory.reads.take();
    let mut retained = InnerMemo::try_with_scratch().unwrap();
    let actual = [(1, 2), (2, 3), (1, 2), (2, 3)]
        .map(|(parent, province)| observe(&mut retained, parent, province));
    let reused = memory.reads.take();
    assert_eq!(actual, expected);
    let header = 0xc000 + hash(1) * route::OUTER_STRIDE + route::INNER_HEADER;
    let sentinel = 0xd000 + 12 * route::INNER_STRIDE + 4;
    for observation in [(header, 17), (sentinel, 1)] {
        assert_eq!(cold.iter().filter(|&&read| read == observation).count(), 2);
        assert_eq!(
            reused.iter().filter(|&&read| read == observation).count(),
            1
        );
    }
    let payload = 0xd000 + hash(2) * route::INNER_STRIDE + route::VALUE;
    for observation in [(payload + route::FROM, 8), (payload, 16), (0xe000, 12)] {
        assert_eq!(cold.iter().filter(|&&read| read == observation).count(), 2);
        assert_eq!(
            reused.iter().filter(|&&read| read == observation).count(),
            2
        );
    }
    assert!(reused.len() < cold.len());

    let mut cold_memo = InnerMemo::default();
    observe(&mut cold_memo, 1, 2).0.unwrap();
    let first = memory.reads.take();
    for failed in 0..first.len() {
        memory.fail.set(failed);
        let mut baseline = InnerMemo::default();
        let expected = observe(&mut baseline, 1, 2);
        let prefix = memory.reads.take();
        retained.begin();
        assert_eq!(observe(&mut retained, 1, 2), expected);
        assert_eq!(memory.reads.take(), prefix);
        memory.fail.set(usize::MAX);
        let expected = observe(&mut baseline, 1, 2);
        let retry = memory.reads.take();
        assert_eq!(observe(&mut retained, 1, 2), expected);
        assert_eq!(memory.reads.take(), retry);
    }

    retained.begin();
    assert_eq!(observe(&mut retained, 2, 3).0.unwrap().0.from, 0);
    retained.begin();
    let mut path = vec![0; 3];
    let (row, need) = route::read_edge_buffer(
        &memory,
        0,
        &other_outer,
        &mut retained,
        2,
        3,
        crate::walk::Buffer::Growing(&mut path),
    )
    .unwrap();
    assert_eq!((row.from, row.to, row.len, need), (2, 3, 2, 2));
    assert_eq!(path, [3, 2, 0]);
}
#[test]
fn parent_chains_record_each_node_less_vertex_once_and_stop_at_nodes() {
    use crate::{supply_node as node, supply_parent_map};
    let mut bytes = train_fixture();
    header(&mut bytes, GAME + cgame_state::PROVINCES, 0x18000, 5, 5);
    put(&mut bytes, 0x18000 + 4 * 8, &0x1a000_u64.to_le_bytes());
    put(
        &mut bytes,
        0x1a000,
        &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x1a000 + crate::cprovince::ID,
        &4_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x1a000 + crate::cprovince::CONTROLLER,
        &1_u32.to_le_bytes(),
    );
    put(&mut bytes, 0xa000 + node::PROVINCE, &4_u32.to_le_bytes());
    let companion = 0x6000 + supply_companion::STRIDE;
    let parents = map(
        &mut bytes,
        companion + supply_parent_map::HEADER,
        0xb800,
        12,
        0,
        &[2, 3, 1],
    );
    for (bucket, parent) in parents.into_iter().zip([1_i32, 1, 4]) {
        put(&mut bytes, bucket + 8, &parent.to_le_bytes());
    }
    let memory = TrainMemory::new(&bytes);
    let mut rows = vec![node::Row::default(); 4];
    let mut paths = vec![0; 3];
    let report = node::walk(&memory, 0, GAME, &mut rows, &mut paths);
    assert_eq!(report.outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        rows.iter().map(|r| (r.index, r.province, r.parent, r.route.from, r.route.len)).collect::<Vec<_>>(),
        [
            (1, 2, 1, 1, 3),
            (2, 3, 1, 0, 0),
            (0, 4, -1, 0, 0),
            (node::VERTEX | 1, 1, 4, 0, 0)
        ]
    );
    assert_eq!(paths, [2, 2, 1]);
    assert_eq!((rows[3].kind, rows[3].available), (0, 0));
}

#[test]
fn parent_vertex_membership_keeps_sorted_rows_cycles_and_country_scope() {
    use crate::{supply_node as node, supply_parent_map};
    let mut memo = node::ProvinceMemo::with_scratch().unwrap();
    for high in [6_u32, 65536] {
        for terminal in [4_i32, high as i32, -1] {
            let mut bytes = train_fixture();
            bytes.resize(0x99000, 0);
            header(
                &mut bytes,
                GAME + cgame_state::PROVINCES,
                0x18000,
                high + 1,
                high + 1,
            );
            for (province, pointer) in [(4_u32, 0x1a000), (5, 0x1a400), (high, 0x1a800)] {
                put(
                    &mut bytes,
                    0x18000 + province as usize * 8,
                    &(pointer as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer,
                    &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + crate::cprovince::ID,
                    &province.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + crate::cprovince::CONTROLLER,
                    &1_u32.to_le_bytes(),
                );
            }
            put(&mut bytes, 0xa000 + node::PROVINCE, &4_u32.to_le_bytes());
            for slot in [1, 2] {
                let companion = 0x6000 + slot * supply_companion::STRIDE;
                header(
                    &mut bytes,
                    companion + supply_companion::NODES,
                    0xa000,
                    3,
                    3,
                );
                let parents = map(
                    &mut bytes,
                    companion + supply_parent_map::HEADER,
                    0xb400 + slot * 0x200,
                    12,
                    0,
                    &[2, 3, high, 5, 1],
                );
                for (bucket, parent) in parents.into_iter().zip([high as i32, 5, 5, 1, terminal]) {
                    put(&mut bytes, bucket + 8, &parent.to_le_bytes());
                }
            }
            let memory = TrainMemory::new(&bytes);
            let mut rows = vec![node::Row::default(); 12];
            memo.begin(&memory, 0, GAME);
            let report = node::walk_part(&memory, 0, GAME, 0, 1, &mut rows, &mut Vec::new(), &mut memo);
            assert_eq!(report.outcome, Outcome::Rows { len: 12 });
            assert_eq!((report.paths, report.required_paths), (0, 0));
            for (country, rows) in [1, 7].into_iter().zip(rows.chunks_exact(6)) {
                assert!(rows.iter().all(|row| row.country == country));
                assert_eq!(
                    rows.iter()
                        .map(|r| (r.index, r.province, r.parent))
                        .collect::<Vec<_>>(),
                    [
                        (1, 2, high as i32),
                        (2, 3, 5),
                        (0, 4, -1),
                        (node::VERTEX | 1, 1, terminal),
                        (node::VERTEX | 5, 5, 1),
                        (node::VERTEX | high, high, 5),
                    ]
                );
            }
            let report =
                node::walk_part(&memory, 0, GAME, 0, 1, &mut rows, &mut Vec::new(), &mut memo);
            assert_eq!(report.outcome, Outcome::Rows { len: 12 });
            assert_eq!(
                rows[9..12].iter().map(|row| row.province).collect::<Vec<_>>(),
                [1, 5, high]
            );
        }
    }
}

#[test]
fn train_inventory_preserves_empty_countries_order_counts_access_and_whole_row_pressure() {
    let bytes = train_fixture();
    let memory = TrainMemory::new(&bytes);
    let mut rows = vec![TrainRow::default(); 2];
    let mut entries = vec![crate::ccountry_supply_system::train_entry::Row::default(); 2];
    let mut access = vec![0; 3];
    let mut runs = vec![[0; 2]; 2];
    let report = walk_trains(
        &memory,
        0,
        GAME,
        &mut rows,
        &mut entries,
        &mut access,
        &mut runs,
    );
    assert_eq!(report.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        (
            rows[0].country,
            rows[0].node_count,
            rows[1].country,
            rows[1].entries_len
        ),
        (1, 3, 7, 0)
    );
    assert_eq!(entries.iter().map(|r| r.count).collect::<Vec<_>>(), [i32::MIN, i32::MAX]);
    assert_eq!(
        rows.iter().map(|row| (
            row.allocated_trucks,
            row.required_trucks_raw,
            row.required_trains,
        )).collect::<Vec<_>>(),
        [
            (Some(i32::MIN), Some(0), Some(i32::MAX)),
            (Some(0), Some(0), Some(0)),
        ]
    );
    assert_eq!(report.vehicle_failures, [0; 3]);
    assert_eq!((rows[0].access_count, rows[0].runs_len), (3, 1));
    assert_eq!((rows[1].access_count, rows[1].runs_len), (0, 0));
    assert_eq!(runs[0], [1, 2]);
    assert_eq!(access[..2], [255, 1]);
    assert_eq!((report.access, report.runs), (2, 1));
}

fn census(n: u32) -> Vec<u8> {
    let n = n as usize;
    let countries = 0x3000;
    let links = countries + n * 8;
    let systems = links + n * 4;
    let companions = systems + n * 8;
    let objects = companions + n * supply_companion::STRIDE;
    let supply = objects + n * 0x10;
    let access = supply + n * SIZE;
    let mut bytes = vec![0_u8; access + n - GAME];
    put(
        &mut bytes,
        GAME + cgame_state::SUPPLY_SYSTEM,
        &(ROOT as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ROOT,
        &(csupply_system::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ROOT + 8,
        &(csupply_system::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    let count = n as u32;
    header(
        &mut bytes,
        GAME + cgame_state::COUNTRIES,
        countries,
        count,
        count,
    );
    header(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        links,
        count,
        count,
    );
    header(
        &mut bytes,
        ROOT + csupply_system::COUNTRY_SYSTEM_DATA,
        systems,
        count,
        count,
    );
    header(
        &mut bytes,
        ROOT + csupply_system::COUNTRY_COMPANION_DATA,
        companions,
        count,
        count,
    );
    for slot in 1..n {
        let country = objects + slot * 0x10;
        let system = supply + slot * SIZE;
        let companion = companions + slot * supply_companion::STRIDE;
        put(
            &mut bytes,
            countries + slot * 8,
            &(country as u64).to_le_bytes(),
        );
        put(&mut bytes, links + slot * 4, &(slot as u32).to_le_bytes());
        put(
            &mut bytes,
            systems + slot * 8,
            &(system as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(&mut bytes, system, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            system + SECONDARY_BASE,
            &(SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + SUPPLY_SYSTEM,
            &(ROOT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + COUNTRY,
            &(country as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + COMPANION,
            &(companion as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            system + ALLOCATED_TRUCKS,
            &(-(slot as i32)).to_le_bytes(),
        );
        put(
            &mut bytes,
            companion + supply_companion::REQUIRED_TRUCKS_RAW,
            &(slot as i64 * 100001).to_le_bytes(),
        );
        put(
            &mut bytes,
            companion + supply_companion::REQUIRED_TRAINS,
            &(slot as i32).to_le_bytes(),
        );
        header(
            &mut bytes,
            companion + supply_companion::RELATED_CONTROLLERS,
            access,
            count,
            count,
        );
        if slot == 4 || slot % 50 == 0 {
            put(&mut bytes, access + slot, &[(slot % 7 + 1) as u8]);
        }
    }
    bytes
}

#[test]
fn train_inputs_reject_bad_identity_map_path_and_reference_without_prefix_success() {
    use crate::supply_node as node;
    let mut nodes = vec![node::Row::default(); 3];
    let mut paths = vec![0; 3];
    let bytes = train_fixture();
    for (at, value) in [
        (0xa000, 99_u32),
        (0x19000 + 0x400 + crate::cprovince::ID, 9),
        (0x6320 + 0x34, u32::MAX),
        (0xe000, 99),
    ] {
        let mut bytes = train_fixture();
        put(&mut bytes, at, &value.to_le_bytes());
        assert!(matches!(
            node::walk(&TrainMemory::new(&bytes), 0, GAME, &mut nodes, &mut paths).outcome,
            Outcome::Unavailable { .. }
        ));
    }
    let mut memory = TrainMemory::new(&bytes);
    memory.reject = Some(0xe000);
    assert!(matches!(
        node::walk(&memory, 0, GAME, &mut nodes, &mut paths).outcome,
        Outcome::Unavailable { .. }
    ));
    let mut bytes = train_fixture();
    put(&mut bytes, 0xf000, &((7_u64 << 32) | 69).to_le_bytes());
    let report = walk_trains(
        &TrainMemory::new(&bytes),
        0,
        GAME,
        &mut vec![TrainRow::default(); 2],
        &mut vec![crate::ccountry_supply_system::train_entry::Row::default(); 2],
        &mut vec![0; 3],
        &mut vec![[0; 2]; 2],
    );
    assert!(matches!(report.outcome, Outcome::Unavailable { .. }));
    let bytes = train_fixture();
    let mut memory = TrainMemory::new(&bytes);
    memory.reject = Some(0xf101);
    let report = walk_trains(&memory, 0, GAME, &mut Vec::new(), &mut Vec::new(), &mut Vec::new(), &mut Vec::new());
    assert_eq!(
        report.outcome,
        Outcome::Unavailable {
            reason: "supply controller access unreadable"
        }
    );
}

#[test]
fn supply_vehicle_faults_keep_inventory_payloads_and_independent_scalars() {
    let bytes = train_fixture();
    let mut baseline = vec![TrainRow::default(); 2];
    let mut baseline_entries = vec![crate::ccountry_supply_system::train_entry::Row::default(); 2];
    let mut baseline_access = vec![0; 3];
    let mut baseline_runs = vec![[0; 2]; 2];
    assert_eq!(
        walk_trains(
            &TrainMemory::new(&bytes),
            0,
            GAME,
            &mut baseline,
            &mut baseline_entries,
            &mut baseline_access,
            &mut baseline_runs,
        )
        .outcome,
        Outcome::Rows { len: 2 }
    );
    let companion = 0x6000 + supply_companion::STRIDE;
    for (field, address) in [
        0x5000 + ALLOCATED_TRUCKS,
        companion + supply_companion::REQUIRED_TRUCKS_RAW,
        companion + supply_companion::REQUIRED_TRAINS,
    ]
    .into_iter()
    .enumerate()
    {
        let mut memory = TrainMemory::new(&bytes);
        memory.reject = Some(address + 1);
        let mut rows = vec![TrainRow::default(); 2];
        let mut entries = vec![crate::ccountry_supply_system::train_entry::Row::default(); 2];
        let mut access = vec![0; 3];
        let mut runs = vec![[0; 2]; 2];
        let report = walk_trains(
            &memory,
            0,
            GAME,
            &mut rows,
            &mut entries,
            &mut access,
            &mut runs,
        );
        assert_eq!(report.outcome, Outcome::Rows { len: 2 });
        let mut expected = baseline.clone();
        match field {
            0 => expected[0].allocated_trucks = None,
            1 => expected[0].required_trucks_raw = None,
            2 => expected[0].required_trains = None,
            _ => unreachable!(),
        }
        assert_eq!(rows, expected);
        assert_eq!(entries, baseline_entries);
        assert_eq!(access, baseline_access);
        assert_eq!(runs, baseline_runs);
        assert_eq!(
            report.vehicle_failures,
            std::array::from_fn(|index| u32::from(index == field))
        );
        assert_eq!(
            crate::supply_node::walk(
                &memory,
                0,
                GAME,
                &mut vec![crate::supply_node::Row::default(); 3],
                &mut vec![0; 3],
            )
            .outcome,
            Outcome::Rows { len: 3 }
        );
    }
}

#[test]
fn allocated_trains_authenticate_full_variants_and_keep_duplicate_entry_order() {
    use crate::{cequipment_variant as variant, creference_registry as registry};
    let mut bytes = train_fixture();
    let handle = (1234_u64 << 32) | 70;
    for entry in [0xf000, 0xf018] {
        put(&mut bytes, entry, &handle.to_le_bytes());
    }
    put(&mut bytes, 0x11008, &0x12000_u64.to_le_bytes());
    put(&mut bytes, 0x11010, &1_u32.to_le_bytes());
    put(&mut bytes, 0x11014, &7_u32.to_le_bytes());
    put(&mut bytes, 0x11018, &[5]);
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let slot = ((hash >> 32) as u32 ^ hash as u32) & 7;
    let bucket = 0x12000 + slot as usize * 24;
    put(&mut bytes, bucket + 4, &[1]);
    put(&mut bytes, bucket + 8, &handle.to_le_bytes());
    put(&mut bytes, bucket + 16, &0x14000_u64.to_le_bytes());
    put(&mut bytes, 0x12000 + 13 * 24 + 4, &[255]);
    put(
        &mut bytes,
        0x14000,
        &(variant::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, 0x14008, &handle.to_le_bytes());
    put(
        &mut bytes,
        0x14000 + variant::DEFINITION,
        &0x15000_u64.to_le_bytes(),
    );
    let mut rows = vec![TrainRow::default(); 2];
    let mut entries = vec![crate::ccountry_supply_system::train_entry::Row::default(); 2];
    let mut access = vec![0; 3];
    let mut runs = vec![[0; 2]; 2];
    let report = walk_trains(
        &TrainMemory::new(&bytes),
        0,
        GAME,
        &mut rows,
        &mut entries,
        &mut access,
        &mut runs,
    );
    assert_eq!(report.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        entries.iter().map(|e| (e.variant, e.count)).collect::<Vec<_>>(),
        [(handle, i32::MIN), (handle, i32::MAX)]
    );
    put(&mut bytes, 0x14008, &(handle + (1_u64 << 32)).to_le_bytes());
    assert!(matches!(
        walk_trains(
            &TrainMemory::new(&bytes),
            0,
            GAME,
            &mut rows,
            &mut entries,
            &mut access,
            &mut runs
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn node_capacity_never_returns_a_complete_prefix() {
    let mut bytes = train_fixture();
    put(
        &mut bytes,
        0x6320 + supply_companion::ROUTES_SUBTYPE_ZERO + 12,
        &1_u32.to_le_bytes(),
    );
    let mut nodes = vec![crate::supply_node::Row::default(); 3];
    let mut paths = vec![0; 3];
    let report =
        crate::supply_node::walk(&TrainMemory::new(&bytes), 0, GAME, &mut nodes, &mut paths);
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
}

#[test]
fn selected_supply_walks_partition_a_census_by_country() {
    const NODES: usize = 2;
    for count in [20, 30] {
        let mut bytes = census(count as u32 + 1);
        let slots = count + 1;
        let countries = 0x3000;
        let links = countries + slots * 8;
        let systems = links + slots * 4;
        let companions = systems + slots * 8;
        let node_list = GAME + bytes.len();
        let province_table = node_list + NODES * crate::supply_node::STRIDE;
        let province = province_table + 16;
        let empty_parents = province + 0x100;
        bytes.resize(empty_parents + 24 - GAME, 0);
        header(
            &mut bytes,
            GAME + cgame_state::PROVINCES,
            province_table,
            2,
            2,
        );
        put(
            &mut bytes,
            province_table + 8,
            &(province as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            province,
            &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            province + crate::cprovince::ID,
            &1_u32.to_le_bytes(),
        );
        put(&mut bytes, empty_parents + 12, &[255]);
        for index in 0..NODES {
            let node = node_list + index * crate::supply_node::STRIDE;
            put(&mut bytes, node, &(index as u32).to_le_bytes());
            put(
                &mut bytes,
                node + crate::supply_node::PROVINCE,
                &1_u32.to_le_bytes(),
            );
        }
        for slot in 1..slots {
            let companion = companions + slot * supply_companion::STRIDE;
            header(
                &mut bytes,
                companion + supply_companion::NODES,
                node_list,
                NODES as u32,
                NODES as u32,
            );
            put(
                &mut bytes,
                companion + crate::supply_parent_map::HEADER,
                &(empty_parents as u64).to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: GAME,
            bytes: &bytes,
        };
        let mut whole_nodes = Vec::new();
        let mut whole_paths = Vec::new();
        let mut memo = crate::supply_node::ProvinceMemo::default();
        memo.begin(&memory, 0, GAME);
        let whole = crate::supply_node::walk_part(
            &memory,
            0,
            GAME,
            0,
            1,
            &mut whole_nodes,
            &mut whole_paths,
            &mut memo,
        );
        assert_eq!(whole.outcome, Outcome::Rows { len: count * NODES });
        whole_nodes.truncate(count * NODES);
        let inventory = |row: &TrainRow, access: &[u8]| {
            (
                row.country,
                row.node_count,
                row.entries_len,
                row.access_count,
                row.runs_len,
                access[row.values_start..row.values_start + row.values_len].to_vec(),
                row.allocated_trucks,
                row.required_trucks_raw,
                row.required_trains,
            )
        };
        let (mut trains, mut entries, mut access, mut runs) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let report = walk_trains(
            &memory,
            0,
            GAME,
            &mut trains,
            &mut entries,
            &mut access,
            &mut runs,
        );
        assert_eq!(report.outcome, Outcome::Rows { len: count });
        let whole_trains: Vec<_> = trains[..count]
            .iter()
            .map(|row| inventory(row, &access))
            .collect();
        let mut read = vec![0_u32; slots];
        for slice in 0..24_u32 {
            let select = |country: u32| country % 24 == slice;
            let mut nodes = Vec::new();
            let mut paths = Vec::new();
            let mut memo = crate::supply_node::ProvinceMemo::default();
            memo.begin(&memory, 0, GAME);
            let report = crate::supply_node::walk_part_selected(
                &memory, 0, GAME, 0, 1, &mut nodes, &mut paths, &mut memo, &select,
            );
            let expected: Vec<_> = whole_nodes
                .iter()
                .filter(|row| select(row.country))
                .copied()
                .collect();
            assert_eq!(
                report.outcome,
                Outcome::Rows {
                    len: expected.len()
                }
            );
            assert_eq!(
                &nodes[..expected.len()],
                expected.as_slice(),
                "slice {slice}"
            );
            let (mut trains, mut entries, mut access, mut runs) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let report = walk_trains_selected(
                &memory,
                0,
                GAME,
                &mut trains,
                &mut entries,
                &mut access,
                &mut runs,
                &select,
            );
            let expected: Vec<_> = whole_trains
                .iter()
                .filter(|row| select(row.0))
                .cloned()
                .collect();
            assert_eq!(
                report.outcome,
                Outcome::Rows {
                    len: expected.len()
                }
            );
            let actual: Vec<_> = trains[..expected.len()]
                .iter()
                .map(|row| inventory(row, &access))
                .collect();
            assert_eq!(actual, expected, "slice {slice}");
            for row in &actual {
                read[row.0 as usize] += 1;
            }
        }
        assert!(read[1..].iter().all(|&count| count == 1));
    }
}

#[test]
fn growing_supply_parent_vertices_and_path_words_complete_the_original_first_walk() {
    let mut bytes = train_fixture();
    let companion = 0x6000 + supply_companion::STRIDE;
    header(
        &mut bytes,
        companion + supply_companion::NODES,
        0xa000,
        1,
        1,
    );
    put(
        &mut bytes,
        0xa000 + crate::supply_node::PROVINCE,
        &3_u32.to_le_bytes(),
    );
    let memory = TrainMemory::new(&bytes);
    let mut expected_rows = vec![crate::supply_node::Row::default(); 3];
    let mut expected_paths = vec![0; 3];
    let expected =
        crate::supply_node::walk(&memory, 0, GAME, &mut expected_rows, &mut expected_paths);
    assert_eq!(expected.outcome, Outcome::Rows { len: 3 });
    let mut rows = Vec::new();
    let mut paths = Vec::new();
    let mut memo = crate::supply_node::ProvinceMemo::default();
    memo.begin(&memory, 0, GAME);
    let captured = crate::supply_node::walk_part(
        &memory, 0, GAME, 0, 1, &mut rows, &mut paths, &mut memo,
    );
    assert_eq!(captured.outcome, expected.outcome);
    assert_eq!((captured.paths, captured.required_paths), (3, 3));
    assert_eq!(&rows[..3], expected_rows.as_slice());
    assert_eq!(&paths[..3], expected_paths.as_slice());
    assert_eq!(
        rows.iter()
            .take(3)
            .filter(|row| row.index & crate::supply_node::VERTEX != 0)
            .count(),
        2
    );
}

#[test]
fn required_trucks_witness_withholds_reset_and_unverified_cache() {
    use crate::supply_node as node;
    let mut bytes = train_fixture();
    let companion_one = 0x6000 + supply_companion::STRIDE;
    let companion_two = 0x6000 + 2 * supply_companion::STRIDE;

    header(&mut bytes, GAME + cgame_state::PROVINCES, 0x18000, 6, 6);
    for (id, controller) in [(4_u32, 7_u32), (5_u32, 1_u32)] {
        let p = 0x19000 + id as usize * 0x400;
        put(&mut bytes, 0x18000 + id as usize * 8, &(p as u64).to_le_bytes());
        put(&mut bytes, p, &(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, p + crate::cprovince::ID, &id.to_le_bytes());
        put(&mut bytes, p + crate::cprovince::CONTROLLER, &controller.to_le_bytes());
    }

    header(&mut bytes, 0x5000 + HUB_SETTINGS_DATA, 0x7000, 2, 2);
    put(&mut bytes, 0x7000 + supply_node_record::PROVINCE, &1_u32.to_le_bytes());
    put(&mut bytes, 0x7000 + supply_node_record::NODE_TYPE, &node::KIND_SUPPLY_HUB.to_le_bytes());
    header(&mut bytes, 0x7000 + supply_node_record::MOTORIZATION_DATA, 0x8000, 1, 1);
    put(&mut bytes, 0x8000, &1_u32.to_le_bytes());
    put(&mut bytes, 0x8004, &[1_u8]);

    put(&mut bytes, 0x7028 + supply_node_record::PROVINCE, &2_u32.to_le_bytes());
    put(&mut bytes, 0x7028 + supply_node_record::NODE_TYPE, &node::KIND_NAVAL_BASE.to_le_bytes());
    header(&mut bytes, 0x7028 + supply_node_record::MOTORIZATION_DATA, 0x8020, 1, 1);
    put(&mut bytes, 0x8020, &1_u32.to_le_bytes());
    put(&mut bytes, 0x8024, &[0_u8]);

    header(&mut bytes, 0xa000 + node::STRIDE + node::MOTORIZATION_OVERRIDES, 0x16000, 1, 1);
    put(&mut bytes, 0x16000, &1_i32.to_le_bytes());
    put(&mut bytes, 0x16004, &2_i32.to_le_bytes());

    header(&mut bytes, companion_two + supply_companion::NODES, 0x16200, 2, 2);
    for (idx, (prov, kind)) in [(4_u32, node::KIND_SUPPLY_HUB), (5, node::KIND_SUPPLY_HUB)]
        .into_iter()
        .enumerate()
    {
        let at = 0x16200 + idx * node::STRIDE;
        put(&mut bytes, at, &(idx as u32).to_le_bytes());
        put(&mut bytes, at + node::PROVINCE, &prov.to_le_bytes());
        put(&mut bytes, at + node::KIND, &kind.to_le_bytes());
    }
    header(&mut bytes, 0x5300 + HUB_SETTINGS_DATA, 0x16500, 1, 1);
    put(&mut bytes, 0x16500 + supply_node_record::PROVINCE, &4_u32.to_le_bytes());
    put(&mut bytes, 0x16500 + supply_node_record::NODE_TYPE, &node::KIND_SUPPLY_HUB.to_le_bytes());
    header(&mut bytes, 0x16500 + supply_node_record::MOTORIZATION_DATA, 0x16600, 1, 1);
    put(&mut bytes, 0x16600, &1_u32.to_le_bytes());
    put(&mut bytes, 0x16604, &[2_u8]);

    header(
        &mut bytes,
        0x2800 + csupply_system::FOREIGN_SOURCE_STRIDE + csupply_system::FOREIGN_SOURCE_RELATED_COUNTRIES,
        0x2980,
        2,
        2,
    );
    put(&mut bytes, 0x2980, &1_u32.to_le_bytes());
    put(&mut bytes, 0x2984, &7_u32.to_le_bytes());

    let expected_required = 15_000_000_i64;
    put(&mut bytes, 0x5000 + ALLOCATED_TRUCKS, &100_i32.to_le_bytes());
    put(
        &mut bytes,
        companion_one + supply_companion::REQUIRED_TRUCKS_RAW,
        &expected_required.to_le_bytes(),
    );
    let partial_ratio = (100_i64 * 10_000_000_000) / expected_required;
    put(
        &mut bytes,
        companion_one + supply_companion::TRUCK_FULFILLMENT_RATIO,
        &partial_ratio.to_le_bytes(),
    );

    let run = |memory: &TrainMemory<'_>| {
        let mut rows = vec![TrainRow::default(); 2];
        let mut entries = vec![crate::ccountry_supply_system::train_entry::Row::default(); 2];
        let mut access = vec![0; 3];
        let mut runs = vec![[0; 2]; 2];
        let report = walk_trains(memory, 0, GAME, &mut rows, &mut entries, &mut access, &mut runs);
        (report, rows, entries)
    };

    let (report, rows, entries) = run(&TrainMemory::new(&bytes));
    assert_eq!(report.outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows[0].required_trucks_raw, Some(expected_required));
    assert_eq!(rows[1].required_trucks_raw, Some(0));
    assert_eq!(rows[0].allocated_trucks, Some(100));
    assert_eq!(rows[0].required_trains, Some(i32::MAX));
    assert_eq!(report.vehicle_failures, [0, 0, 0]);
    assert_eq!(report.required_trucks_withheld, None);
    assert_eq!(entries.len(), 2);

    let mut full_ratio = bytes.clone();
    put(&mut full_ratio, 0x5000 + ALLOCATED_TRUCKS, &200_i32.to_le_bytes());
    put(
        &mut full_ratio,
        companion_one + supply_companion::TRUCK_FULFILLMENT_RATIO,
        &100_000_i64.to_le_bytes(),
    );
    let (report, rows, _) = run(&TrainMemory::new(&full_ratio));
    assert_eq!(rows[0].required_trucks_raw, Some(expected_required));
    assert_eq!(report.vehicle_failures, [0, 0, 0]);

    let mut reset_zero = bytes.clone();
    put(&mut reset_zero, companion_two + supply_companion::TRUCK_FULFILLMENT_RATIO, &0_i64.to_le_bytes());
    let (report, rows, _) = run(&TrainMemory::new(&reset_zero));
    assert_eq!(rows[0].required_trucks_raw, Some(expected_required));
    assert_eq!(rows[1].required_trucks_raw, None);
    assert_eq!(report.vehicle_failures, [0, 1, 0]);
    assert_eq!(
        report.required_trucks_withheld,
        Some("supply truck fulfillment ratio witness mismatch")
    );

    let mut tampered = bytes.clone();
    put(
        &mut tampered,
        companion_one + supply_companion::REQUIRED_TRUCKS_RAW,
        &11_000_000_i64.to_le_bytes(),
    );
    let (report, rows, _) = run(&TrainMemory::new(&tampered));
    assert_eq!(rows[0].required_trucks_raw, None);
    assert_eq!(rows[0].allocated_trucks, Some(100));
    assert_eq!(rows[0].required_trains, Some(i32::MAX));
    assert_eq!(rows[1].required_trucks_raw, Some(0));
    assert_eq!(report.vehicle_failures, [0, 1, 0]);
    assert_eq!(
        report.required_trucks_withheld,
        Some("supply required trucks witness mismatch")
    );

    let mut bad_level = bytes.clone();
    put(&mut bad_level, 0x16004, &3_i32.to_le_bytes());
    let (report, rows, _) = run(&TrainMemory::new(&bad_level));
    assert_eq!(rows[0].required_trucks_raw, None);
    assert_eq!(
        report.required_trucks_withheld,
        Some("supply hub motorization level unsupported")
    );

    let mut no_define = TrainMemory::new(&bytes);
    no_define.truck_cost = None;
    let (report, rows, _) = run(&no_define);
    assert_eq!(rows[0].required_trucks_raw, None);
    assert_eq!(rows[1].required_trucks_raw, None);
    assert_eq!(rows[0].allocated_trucks, Some(100));
    assert_eq!(rows[0].required_trains, Some(i32::MAX));
    assert_eq!(report.vehicle_failures, [0, 2, 0]);
    assert_eq!(
        report.required_trucks_withheld,
        Some("supply truck cost define unreadable")
    );

    let overflow_base = usize::MAX - 0x0330_0000;
    let mut taint_bytes = bytes.clone();
    put(
        &mut taint_bytes,
        ROOT,
        &((overflow_base.wrapping_add(csupply_system::VTABLE_RVA.0)) as u64).to_le_bytes(),
    );
    put(
        &mut taint_bytes,
        ROOT + 8,
        &((overflow_base.wrapping_add(csupply_system::SECONDARY_VTABLE_RVA.0)) as u64).to_le_bytes(),
    );
    for (country, system) in [(0x3000_usize, 0x5000_usize), (0x4000, 0x5300)] {
        put(
            &mut taint_bytes,
            country,
            &((overflow_base.wrapping_add(ccountry::VTABLE_RVA.0)) as u64).to_le_bytes(),
        );
        put(
            &mut taint_bytes,
            system,
            &((overflow_base.wrapping_add(VTABLE_RVA.0)) as u64).to_le_bytes(),
        );
        put(
            &mut taint_bytes,
            system + SECONDARY_BASE,
            &((overflow_base.wrapping_add(SECONDARY_VTABLE_RVA.0)) as u64).to_le_bytes(),
        );
    }
    let _ = engine_sites::take_taint();
    let mut rows = vec![TrainRow::default(); 2];
    let mut entries = vec![crate::ccountry_supply_system::train_entry::Row::default(); 2];
    let mut access = vec![0; 3];
    let mut runs = vec![[0; 2]; 2];
    let report = walk_trains(
        &TrainMemory::new(&taint_bytes),
        overflow_base,
        GAME,
        &mut rows,
        &mut entries,
        &mut access,
        &mut runs,
    );
    assert_eq!(report.outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows[0].required_trucks_raw, None);
    assert_eq!(rows[0].allocated_trucks, Some(100));
    assert_eq!(
        report.required_trucks_withheld,
        Some("supply truck cost define site unlocated")
    );
    assert_eq!(engine_sites::take_taint(), None);
}
