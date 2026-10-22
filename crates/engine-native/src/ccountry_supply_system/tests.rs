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
        &(csupply_system::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ROOT + 8,
        &(csupply_system::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
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
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, country + ccountry::ID, &id.to_le_bytes());
        put(&mut bytes, 0x2600 + slot as usize * 4, &slot.to_le_bytes());
        put(&mut bytes, 0x2600 + id as usize * 4, &slot.to_le_bytes());
        put(&mut bytes, system, &(VTABLE_RVA as u64).to_le_bytes());
        put(
            &mut bytes,
            system + SECONDARY_BASE,
            &(SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
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
    let mut rows = [FloatingHarborDeployment::default(); 4];
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
    let mut short = [FloatingHarborDeployment::default(); 2];
    assert_eq!(
        walk_floating_harbor_deployments(&memory, 0, GAME, &mut short),
        Outcome::Truncated {
            len: 2,
            unread_rows: 2
        }
    );
    assert_eq!(short.as_slice(), &rows[..2]);
}

#[test]
fn settings_preserve_raw_actor_aliases_zero_and_exact_level_byte() {
    let mut bytes = fixture();
    let mut rows = [Row::default(); 3];
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
    // The level is recorded losslessly even outside the normal UI choices.
    put(&mut bytes, 0x8004, &[255]);
    assert_eq!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert_eq!(rows[0].level, 255);
}

#[test]
fn capacity_counts_settings_across_nodes_and_empty_maps_emit_nothing() {
    let mut bytes = fixture();
    let mut rows = [Row::default(); 1];
    assert_eq!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut rows),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    assert_eq!(
        walk(&NoEntryPadding(&bytes), 0, GAME, &mut []),
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
    let mut rows = [Row::default(); 3];
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
    put(
        &mut bytes,
        0x5000 + ALLOCATED_TRUCKS,
        &i32::MIN.to_le_bytes(),
    );
    put(
        &mut bytes,
        companion + supply_companion::REQUIRED_TRUCKS_RAW,
        &i64::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        companion + supply_companion::REQUIRED_TRAINS,
        &i32::MAX.to_le_bytes(),
    );
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
            &(crate::cprovince::VTABLE_RVA as u64).to_le_bytes(),
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
        if address == crate::supply_route::EMPTY_RVA + crate::supply_route::FROM {
            destination.fill(0);
            return true;
        }
        if address == crate::creference_registry::LOW_ROOTS_RVA + 70 * 8 {
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
        }
    }
}
#[test]
fn train_nodes_preserve_native_identity_values_parent_edges_and_count_raw_read_work() {
    let bytes = train_fixture();
    let memory = TrainMemory::new(&bytes);
    let mut nodes = [crate::supply_node::Row::default(); 3];
    let mut paths = [0; 3];
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
        let mut paths = [0; 3];
        let report = node::walk(&TrainMemory::new(bytes), 0, GAME, &mut nodes, &mut paths);
        let mut rows = nodes
            .iter()
            .map(|n| (n.index, n.province, n.kind, n.parent))
            .collect::<Vec<_>>();
        rows.sort_unstable();
        (report, rows)
    };
    // Province lookup returns null for zero and for ids outside the census, and
    // the node creators store such a province unchecked. Native readers only
    // key maps with it, so it is a root without a route, recorded as stored and
    // observed.
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
    // An empty census slot is the same null province.
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
    // The fixture's parent map (keys 2 and 3), rewritten with new parents.
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
    // A null parent keys the parent map like any other and becomes a vertex row
    // of its own, as the parent walk would visit it.
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
    // A province that resolves must be the CProvince with that id, and an
    // unreadable slot is no native value; each refusal names its check.
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
    let mut nodes = [node::Row::default(); 3];
    let report = node::walk(&memory, 0, GAME, &mut nodes, &mut [0; 3]);
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
    let capture = |bytes: &[u8], capacity: usize, reject_outer: bool| {
        let mut source = TrainMemory::new(bytes);
        source.reject = reject_outer.then_some(ROOT + supply_route::ROUTE_MAP);
        let memory = Traced {
            source,
            outer_reads: Cell::new(0),
        };
        let mut rows = [node::Row::default(); 3];
        let mut paths = [0; 3];
        let report = node::walk(&memory, 0, GAME, &mut rows[..capacity], &mut paths);
        (report.outcome, memory.outer_reads.get(), rows, paths)
    };
    // An unreadable route map is irrelevant to empty/root-only censuses
    // and rows omitted for capacity. A positive-parent edge must fail it.
    for (count, capacity, expected, route_reads) in [
        (0, 3, Outcome::Rows { len: 0 }, 0),
        (1, 3, Outcome::Rows { len: 1 }, 0),
        (
            3,
            0,
            Outcome::Truncated {
                len: 0,
                unread_rows: 3,
            },
            0,
        ),
        (
            3,
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
        let (outcome, reads, rows, _) = capture(&bytes, capacity, true);
        assert_eq!(outcome, expected);
        assert_eq!(reads, route_reads);
        if count == 1 {
            assert_eq!((rows[0].province, rows[0].parent), (1, -1));
        }
    }
    let mut bytes = train_fixture();
    let (outcome, reads, _, paths) = capture(&bytes, 3, false);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(reads, 1);
    assert_eq!(paths, [2, 2, 1]);

    // The later route-map fault must not replace the first node failure.
    put(&mut bytes, 0xa000 + node::INDEX, &99_u32.to_le_bytes());
    let (outcome, reads, _, _) = capture(&bytes, 3, true);
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
        let mut rows = [node::Row::default(); 6];
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
    // The two countries retain their separate rows and paths, while each
    // successful global route identity, path header and path is acquired once.
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

    // Every cold failure before the second country's first slot preserves
    // the same independent failure and observed prefix. Failed edges never
    // become a success in a subsequent capture using the same allocation.
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

    // Clearing is per walk, even without changing the root addresses.
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
    // Another root has the previously absent parent, with its own route.
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
        let mut path = [0; 3];
        let result = route::read_edge_buffer(
            &memory,
            0,
            &outer,
            memo,
            parent,
            province,
            crate::walk::Buffer::Fixed(&mut path),
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
    // This test bypasses the complete-edge memo. Every route identity,
    // header and path occurrence therefore still reads its native payload.
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

    // A new root walk must discard both the old present and absent results.
    retained.begin();
    assert_eq!(observe(&mut retained, 2, 3).0.unwrap().0.from, 0);
    retained.begin();
    let mut path = [0; 3];
    let (row, need) = route::read_edge_buffer(
        &memory,
        0,
        &other_outer,
        &mut retained,
        2,
        3,
        crate::walk::Buffer::Fixed(&mut path),
    )
    .unwrap();
    assert_eq!((row.from, row.to, row.len, need), (2, 3, 2, 2));
    assert_eq!(path, [3, 2, 0]);
}
#[test]
fn parent_chains_record_each_node_less_vertex_once_and_stop_at_nodes() {
    use crate::{supply_node as node, supply_parent_map};
    let mut bytes = train_fixture();
    // Province 4 exists; node 0 moves there, outside the parent map.
    header(&mut bytes, GAME + cgame_state::PROVINCES, 0x18000, 5, 5);
    put(&mut bytes, 0x18000 + 4 * 8, &0x1a000_u64.to_le_bytes());
    put(
        &mut bytes,
        0x1a000,
        &(crate::cprovince::VTABLE_RVA as u64).to_le_bytes(),
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
    // Both other nodes hang off vertex 1, which has no node row; its
    // parent is node province 4.
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
    let mut rows = [node::Row::default(); 4];
    let mut paths = [0; 3];
    let report = node::walk(&memory, 0, GAME, &mut rows, &mut paths);
    assert_eq!(report.outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        rows.map(|r| (r.index, r.province, r.parent, r.route.from, r.route.len)),
        [
            (1, 2, 1, 1, 3),
            (2, 3, 1, 0, 0),
            (0, 4, -1, 0, 0),
            (node::VERTEX | 1, 1, 4, 0, 0)
        ]
    );
    assert_eq!(paths, [2, 2, 1]);
    assert_eq!((rows[3].kind, rows[3].available), (0, 0));
    let report = node::walk(&memory, 0, GAME, &mut rows[..3], &mut paths);
    assert_eq!(
        report.outcome,
        Outcome::Truncated {
            len: 3,
            unread_rows: 1
        }
    );
}

#[test]
fn parent_vertex_membership_keeps_sorted_rows_cycles_country_scope_and_truncation() {
    use crate::{supply_node as node, supply_parent_map};
    let mut memo = node::ProvinceMemo::with_scratch().unwrap();
    // The large ID exercises the exact fallback outside the bounded bitset.
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
                    &(crate::cprovince::VTABLE_RVA as u64).to_le_bytes(),
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
            let mut rows = [node::Row::default(); 12];
            memo.begin(&memory, 0, GAME);
            let report = node::walk_part(&memory, 0, GAME, 0, 1, &mut rows, &mut [], &mut memo);
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
            // Clear membership even when the same memo and census are reused.
            let report =
                node::walk_part(&memory, 0, GAME, 0, 1, &mut rows[..11], &mut [], &mut memo);
            assert_eq!(
                report.outcome,
                Outcome::Truncated {
                    len: 11,
                    unread_rows: 1
                }
            );
            assert_eq!(rows[9].province, 5);
            assert_eq!(rows[10].province, high);
        }
    }
}

#[test]
fn train_inventory_preserves_empty_countries_order_counts_access_and_whole_row_pressure() {
    let bytes = train_fixture();
    let memory = TrainMemory::new(&bytes);
    let mut rows = [TrainRow::default(); 2];
    let mut entries = [crate::supply_train_entry::Row::default(); 2];
    let mut access = [0; 3];
    let mut runs = [[0; 2]; 2];
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
    assert_eq!(entries.map(|r| r.count), [i32::MIN, i32::MAX]);
    assert_eq!(
        rows.map(|row| (
            row.allocated_trucks,
            row.required_trucks_raw,
            row.required_trains,
        )),
        [
            (Some(i32::MIN), Some(i64::MAX), Some(i32::MAX)),
            (Some(0), Some(0), Some(0)),
        ]
    );
    assert_eq!(report.vehicle_failures, [0; 3]);
    // The native array [0, 255, 1] keeps its count and its one nonzero run.
    assert_eq!((rows[0].access_count, rows[0].runs_len), (3, 1));
    assert_eq!((rows[1].access_count, rows[1].runs_len), (0, 0));
    assert_eq!(runs[0], [1, 2]);
    assert_eq!(access[..2], [255, 1]);
    assert_eq!((report.access, report.runs), (2, 1));
    let report = walk_trains(
        &memory,
        0,
        GAME,
        &mut rows,
        &mut entries[..1],
        &mut access,
        &mut runs,
    );
    assert_eq!(
        report.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(rows[0].country, 7);
    assert_eq!(report.required_entries, 2);
}

/// A synthetic census of `n` canonical slots (slot 0 is the sentinel). Every
/// country has a supply system, a companion with no nodes or trains, and an
/// `n`-byte controller-access array; all arrays share one access buffer
/// whose byte at slot `s` is nonzero only for `s == 4` and for every 50th
/// slot from 50, as a mostly-denied controller map is.
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
        &(csupply_system::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        ROOT + 8,
        &(csupply_system::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
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
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            country + ccountry::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(&mut bytes, system, &(VTABLE_RVA as u64).to_le_bytes());
        put(
            &mut bytes,
            system + SECONDARY_BASE,
            &(SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
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
    let mut nodes = [node::Row::default(); 3];
    let mut paths = [0; 3];
    let bytes = train_fixture();
    let memory = TrainMemory::new(&bytes);
    let report = node::walk(&memory, 0, GAME, &mut nodes, &mut paths[..2]);
    assert_eq!(
        report.outcome,
        Outcome::Truncated {
            len: 2,
            unread_rows: 1
        }
    );
    assert_eq!(report.required_paths, 3);
    for (at, value) in [
        (0xa000, 99_u32),
        // Province 1's object holds another id.
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
        &mut [TrainRow::default(); 2],
        &mut [crate::supply_train_entry::Row::default(); 2],
        &mut [0; 3],
        &mut [[0; 2]; 2],
    );
    assert!(matches!(report.outcome, Outcome::Unavailable { .. }));
    // An unreadable access array fails the table even with no room to store it.
    let bytes = train_fixture();
    let mut memory = TrainMemory::new(&bytes);
    memory.reject = Some(0xf101);
    let report = walk_trains(&memory, 0, GAME, &mut [], &mut [], &mut [], &mut []);
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
    let mut baseline = [TrainRow::default(); 2];
    let mut baseline_entries = [crate::supply_train_entry::Row::default(); 2];
    let mut baseline_access = [0; 3];
    let mut baseline_runs = [[0; 2]; 2];
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
        let mut rows = [TrainRow::default(); 2];
        let mut entries = [crate::supply_train_entry::Row::default(); 2];
        let mut access = [0; 3];
        let mut runs = [[0; 2]; 2];
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
        let mut expected = baseline;
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
                &mut [crate::supply_node::Row::default(); 3],
                &mut [0; 3],
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
        &(variant::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(&mut bytes, 0x14008, &handle.to_le_bytes());
    put(
        &mut bytes,
        0x14000 + variant::DEFINITION,
        &0x15000_u64.to_le_bytes(),
    );
    let mut rows = [TrainRow::default(); 2];
    let mut entries = [crate::supply_train_entry::Row::default(); 2];
    let mut access = [0; 3];
    let mut runs = [[0; 2]; 2];
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
        entries.map(|e| (e.variant, e.count)),
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
    // Remove destination from the endpoint controller's sorted node map.
    put(
        &mut bytes,
        0x6320 + supply_companion::ROUTES_SUBTYPE_ZERO + 12,
        &1_u32.to_le_bytes(),
    );
    let mut nodes = [crate::supply_node::Row::default(); 3];
    let mut paths = [0; 3];
    let report =
        crate::supply_node::walk(&TrainMemory::new(&bytes), 0, GAME, &mut nodes, &mut paths);
    assert_eq!(report.outcome, Outcome::Rows { len: 3 });
    let report = crate::supply_node::walk(
        &TrainMemory::new(&bytes),
        0,
        GAME,
        &mut nodes[..1],
        &mut paths,
    );
    assert_eq!(
        report.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    let report = crate::supply_node::walk(&TrainMemory::new(&bytes), 0, GAME, &mut [], &mut paths);
    assert_eq!(
        report.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
}

/// A walk of selected countries reads exactly their nodes and inventory, each
/// equal to the whole walk's: 24 slices by native country id cover the
/// census once (the daily `supply_trains` family).
#[test]
fn selected_supply_walks_partition_a_census_by_country() {
    const NODES: usize = 2;
    // Twenty countries leave slices empty; thirty put two countries in a slice.
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
            &(crate::cprovince::VTABLE_RVA as u64).to_le_bytes(),
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
        let whole = crate::supply_node::walk_part_growing(
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
        // A train row's arena offsets depend on the walk; its read values do not.
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
        let report = walk_trains_growing(
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
            let report = crate::supply_node::walk_part_selected_growing(
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
            let report = walk_trains_selected_growing(
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
        // Every country is read in exactly one of the day's slices.
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
    let captured = crate::supply_node::walk_part_growing(
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
