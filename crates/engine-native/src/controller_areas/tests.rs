use super::*;
use crate::memory::SliceMemory;

const BASE: usize = 0x10_0000;
const SIZE: usize = 0xd000;

fn province(id: u32) -> usize {
    BASE + 0x1000 + id as usize * 0x100
}
fn definition(id: u32) -> usize {
    BASE + 0x2000 + id as usize * 0x100
}
fn area(n: usize) -> usize {
    BASE + 0x6000 + n * 0x200
}

struct World(Vec<u8>);

impl World {
    fn new(provinces: u32) -> Self {
        let mut world = Self(vec![0; SIZE]);
        for id in 1..=provinces {
            world.u32(province(id) + cprovince::ID, id);
        }
        world
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at - BASE..at - BASE + bytes.len()].copy_from_slice(bytes);
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: usize) {
        self.put(at, &(value as u64).to_le_bytes());
    }
    fn neighbours(&mut self, id: u32, neighbours: &[u32]) {
        let data = BASE + 0x3000 + id as usize * 0x200;
        let header = definition(id) + province_definition::NEIGHBOURS;
        self.u64(header, data);
        self.u32(header + 8, neighbours.len() as u32);
        self.u32(header + 12, neighbours.len() as u32);
        for (index, &neighbour) in neighbours.iter().enumerate() {
            let entry = data + index * province_neighbour::STRIDE;
            self.u32(entry + province_neighbour::PROVINCE_ID, neighbour);
        }
    }
    fn area(&mut self, n: usize, members: &[u32], representative: u32, contacts: &[usize]) {
        let at = area(n);
        self.u64(at, ccontroller_area::VTABLE_RVA.0);
        self.u64(at + 0x28, province(representative));
        let data = BASE + 0x8000 + n * 0x100;
        self.u64(at + 0x30, data);
        self.u32(at + 0x38, members.len() as u32);
        self.u32(at + 0x3c, members.len() as u32);
        for (index, &member) in members.iter().enumerate() {
            self.u64(data + index * 8, province(member));
        }
        let nodes = BASE + 0x9000 + n * 0x200;
        let payloads = BASE + 0xb000 + n * 0x200;
        let node = |index: usize| nodes + index * 0x20;
        self.u64(at + 0x90, if contacts.is_empty() { 0 } else { node(0) });
        self.u64(
            at + 0x98,
            if contacts.is_empty() {
                0
            } else {
                node(contacts.len() - 1)
            },
        );
        self.u32(at + 0xa0, contacts.len() as u32);
        self.put(at + 0xa4, &[0]);
        for (index, &contact) in contacts.iter().enumerate() {
            let payload = payloads + index * 0x10;
            self.u64(node(index), payload);
            self.u64(
                node(index) + 8,
                if index == 0 { 0 } else { node(index - 1) },
            );
            self.u64(
                node(index) + 16,
                if index + 1 == contacts.len() {
                    0
                } else {
                    node(index + 1)
                },
            );
            self.put(node(index) + 24, &[0]);
            self.u64(payload, area(contact));
            self.u32(payload + 8, 1);
        }
    }
    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: BASE,
            bytes: &self.0,
        }
    }
}

fn walk(areas: &[(Option<usize>, Option<usize>)]) -> (Vec<cprovince::Row>, Vec<Pointers>) {
    let rows = (1..=areas.len() as u32)
        .map(|id| cprovince::Row {
            id,
            ..Default::default()
        })
        .collect();
    let pointers = areas
        .iter()
        .enumerate()
        .map(|(index, &(controller, owner))| Pointers {
            province: province(index as u32 + 1),
            definition: definition(index as u32 + 1),
            controller: controller.map_or(0, area),
            owner: owner.map_or(0, |n| BASE + 0xc800 + n * 0x10),
        })
        .collect();
    (rows, pointers)
}

fn labels(rows: &[cprovince::Row]) -> Vec<(Option<u32>, Option<u32>, bool)> {
    rows.iter()
        .map(|row| (row.controller_area, row.owner_area, row.area_node.is_some()))
        .collect()
}

fn nodes(output: &Output) -> Vec<(u32, Option<u32>, Option<Vec<u32>>)> {
    output
        .nodes
        .iter()
        .map(|node| {
            (
                node.label,
                node.first_land,
                output.neighbours(node).map(<[u32]>::to_vec),
            )
        })
        .collect()
}

const TAKE: Option<[u64; 2]> = Some([3, 4]);

fn baseline() -> (World, Vec<cprovince::Row>, Vec<Pointers>) {
    let mut world = World::new(6);
    world.area(0, &[2, 3], 3, &[1]);
    world.area(1, &[4, 5], 4, &[0]);
    for (id, neighbours) in [
        (1, &[2][..]),
        (2, &[1, 3, 4]),
        (3, &[2, 5]),
        (4, &[2, 5]),
        (5, &[3, 4, 6]),
        (6, &[5]),
    ] {
        world.neighbours(id, neighbours);
    }
    let (rows, pointers) = walk(&[
        (None, Some(0)),
        (Some(0), Some(0)),
        (Some(0), Some(0)),
        (Some(1), Some(1)),
        (Some(1), Some(1)),
        (None, None),
    ]);
    (world, rows, pointers)
}

#[test]
fn a_full_read_labels_areas_by_their_smallest_member_and_records_nodes_once() {
    let (world, mut rows, pointers) = baseline();
    let mut store = Store::new();
    let mut output = Output::default();
    let counts = store
        .capture(
            &world.memory(),
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    assert!(counts.full && !counts.quiet);
    assert_eq!(counts.read, 2);
    assert_eq!(
        labels(&rows),
        [
            (Some(0), Some(1), false),
            (Some(2), Some(1), true),
            (Some(2), Some(1), false),
            (Some(4), Some(4), true),
            (Some(4), Some(4), false),
            (Some(0), Some(0), false),
        ]
    );
    assert_eq!(
        nodes(&output),
        [(2, Some(3), Some(vec![4])), (4, Some(4), Some(vec![2]))]
    );
    assert_eq!(
        output.nodes[rows[1].area_node.unwrap() as usize].label,
        2,
        "the node lives on its label's row"
    );
}

#[test]
fn a_quiet_hour_publishes_the_held_areas_without_reading_them() {
    let (mut world, mut rows, pointers) = baseline();
    let mut store = Store::new();
    let mut output = Output::default();
    let memory = world.memory();
    store
        .capture(
            &memory,
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    let held = nodes(&output);
    world.u64(area(0), 0);
    let mut fresh = rows.clone();
    for row in &mut fresh {
        row.controller_area = None;
    }
    let counts = store
        .capture(
            &world.memory(),
            0,
            &mut fresh,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    assert!(counts.quiet && !counts.full);
    assert_eq!((counts.read, counts.carried), (0, 2));
    assert_eq!(nodes(&output), held);
    assert_eq!(labels(&fresh), labels(&rows));
    let counts = store
        .capture(
            &world.memory(),
            0,
            &mut fresh,
            &pointers,
            &mut output,
            Some([5, 4]),
        )
        .unwrap();
    assert!(counts.full);
    assert_eq!(output.nodes[0].first_land, None);
}

#[test]
fn the_repointable_provinces_are_the_marked_ones_areas_and_neighbours() {
    let (world, mut rows, pointers) = baseline();
    for row in &mut rows[1..5] {
        row.controller = 7;
    }
    let mut store = Store::new();
    let mut output = Output::default();
    let memory = world.memory();
    store
        .capture(
            &memory,
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    let mut scope = Vec::new();
    assert_eq!(store.repointable(&memory, &rows, &mut scope), Ok(false));
    store.marks().push(4);
    assert_eq!(store.repointable(&memory, &rows, &mut scope), Ok(true));
    assert_eq!(scope, [2, 3, 4, 5]);
    rows[3].controller = 0;
    assert_eq!(store.repointable(&memory, &rows, &mut scope), Ok(false));
    assert!(scope.is_empty());
    rows[3].controller = 7;
    rows[5].controller = 7;
    assert_eq!(store.repointable(&memory, &rows, &mut scope), Ok(false));
}

#[test]
fn a_marked_hour_follows_the_moved_province_and_relabels_both_areas() {
    let (mut world, mut rows, mut pointers) = baseline();
    let mut store = Store::new();
    let mut output = Output::default();
    store
        .capture(
            &world.memory(),
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    world.area(0, &[3], 3, &[1]);
    world.area(1, &[2, 4, 5], 4, &[0]);
    pointers[1].controller = area(1);
    store.marks().push(2);
    let counts = store
        .capture(
            &world.memory(),
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    assert!(!counts.full && !counts.quiet);
    assert_eq!((counts.changed, counts.read), (1, 2));
    assert_eq!(
        labels(&rows),
        [
            (Some(0), Some(1), false),
            (Some(2), Some(1), true),
            (Some(3), Some(1), true),
            (Some(2), Some(4), false),
            (Some(2), Some(4), false),
            (Some(0), Some(0), false),
        ]
    );
    assert_eq!(
        nodes(&output),
        [(2, Some(4), Some(vec![3])), (3, Some(3), Some(vec![2]))]
    );
}

#[test]
fn node_faults_refuse_only_their_operands_and_a_short_walk_labels_nothing() {
    let (mut world, mut rows, pointers) = baseline();
    world.put(area(0) + 0xa4, &[1]);
    world.u32(area(1) + 0x3c, 3);
    let mut store = Store::new();
    let mut output = Output::default();
    store
        .capture(
            &world.memory(),
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    assert_eq!(nodes(&output), [(2, Some(3), None), (4, None, None)]);
    world.put(area(0) + 0xa4, &[0]);
    world.u32(area(1) + 0x3c, 2);
    world.u64(BASE + 0xb000, area(5));
    let mut store = Store::new();
    store
        .capture(
            &world.memory(),
            0,
            &mut rows,
            &pointers,
            &mut output,
            TAKE,
        )
        .unwrap();
    assert_eq!(nodes(&output)[0], (2, Some(3), None));
    let short = &pointers[..5];
    assert!(
        store
            .capture(
                &world.memory(),
                0,
                &mut rows,
                short,
                &mut output,
                TAKE,
            )
            .is_err()
    );
    assert!(rows.iter().all(|row| row.controller_area.is_none()));
    assert!(output.nodes.is_empty());
}
