use super::*;
use std::collections::BTreeMap;

const MANAGER: usize = 0x0400_0000;
const NODES: usize = 0x0410_0000;
const NODE_LISTS: usize = 0x0420_0000;
const BATTLES: usize = 0x0500_0000;
const MEMBERS: usize = 0x0600_0000;
const PROVINCES: usize = 0x0700_0000;

#[derive(Default)]
struct World(BTreeMap<usize, Vec<u8>>);

impl World {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        if let Some((&start, data)) = self.0.range_mut(..=at).next_back()
            && at - start + bytes.len() <= data.len()
        {
            data[at - start..at - start + bytes.len()].copy_from_slice(bytes);
            return;
        }
        self.0.insert(at, bytes.to_vec());
    }

    fn array(&mut self, at: usize, data: usize, capacity: u32, count: u32) {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&(data as u64).to_le_bytes());
        header[8..12].copy_from_slice(&capacity.to_le_bytes());
        header[12..].copy_from_slice(&count.to_le_bytes());
        self.put(at, &header);
    }

    fn pointers(&mut self, at: usize, pointers: &[usize]) {
        let bytes: Vec<u8> = pointers
            .iter()
            .flat_map(|&pointer| (pointer as u64).to_le_bytes())
            .collect();
        self.put(at, &bytes);
    }
}

impl Memory for World {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        let Some((&start, data)) = self.0.range(..=at).next_back() else {
            return false;
        };
        let Some(bytes) = data.get(at - start..at - start + out.len()) else {
            return false;
        };
        out.copy_from_slice(bytes);
        true
    }
}

#[derive(Clone)]
struct Battle {
    center: [u32; 2],
    radius: u32,
    members: Vec<u32>,
    sound: u64,
}

fn battle(x: f32, z: f32, radius: f32, members: &[u32]) -> Battle {
    Battle {
        center: [x.to_bits(), z.to_bits()],
        radius: radius.to_bits(),
        members: members.to_vec(),
        sound: 0,
    }
}

fn province_at(id: u32) -> usize {
    PROVINCES + id as usize * 0x100
}

fn world(count: u32, placed: &[(u32, Vec<Battle>)]) -> World {
    let mut world = World::default();
    world.put(MANAGER_RVA.0, &(MANAGER as u64).to_le_bytes());
    world.put(MANAGER, &[0; 0x78]);
    world.put(MANAGER + MANAGER_TREE, &3_i32.to_le_bytes());
    world.array(MANAGER + MANAGER_TREE + TREE_NODES, NODES, count, count);
    world.put(NODES, &vec![0; count as usize * NODE_STRIDE]);
    let mut next = 0;
    for (node, battles) in placed {
        let list = NODE_LISTS + *node as usize * 0x1000;
        let at = NODES + *node as usize * NODE_STRIDE + NODE_BATTLES;
        world.array(at, list, battles.len() as u32, battles.len() as u32);
        let mut pointers = Vec::new();
        for battle in battles {
            let object = BATTLES + next * 0x100;
            let members = MEMBERS + next * 0x100;
            next += 1;
            pointers.push(object);
            let mut bytes = vec![0; 0x50];
            bytes[BATTLE_CENTER..BATTLE_CENTER + 4]
                .copy_from_slice(&battle.center[0].to_le_bytes());
            bytes[BATTLE_CENTER + 4..BATTLE_CENTER + 8]
                .copy_from_slice(&battle.center[1].to_le_bytes());
            bytes[BATTLE_RADIUS..BATTLE_RADIUS + 4].copy_from_slice(&battle.radius.to_le_bytes());
            bytes[BATTLE_SOUND..BATTLE_SOUND + 8].copy_from_slice(&battle.sound.to_le_bytes());
            world.put(object, &bytes);
            let count = battle.members.len() as u32;
            world.array(object + BATTLE_PROVINCES, members, count + 2, count);
            let provinces: Vec<usize> = battle.members.iter().map(|&id| province_at(id)).collect();
            world.pointers(members, &provinces);
            for &id in &battle.members {
                let mut province = vec![0; 0xa8];
                province[..8].copy_from_slice(&(cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
                province[cprovince::ID..cprovince::ID + 4]
                    .copy_from_slice(&(id as i32).to_le_bytes());
                world.put(province_at(id), &province);
            }
        }
        world.pointers(list, &pointers);
    }
    world
}

fn preorder(node: u32, count: u32, out: &mut Vec<u32>) {
    if node >= count {
        return;
    }
    out.push(node);
    for child in 1..=NODE_CHILDREN {
        preorder(node * NODE_CHILDREN + child, count, out);
    }
}

#[test]
fn nodes_follow_the_recursive_preorder() {
    for count in [0, 1, 2, 4, 5, 6, 7, 20, 21, 22, 84, 85, 86, 341] {
        let mut expected = Vec::new();
        preorder(0, count, &mut expected);
        let mut walked = Vec::new();
        let mut node = (count > 0).then_some(0);
        while let Some(index) = node {
            walked.push(index);
            node = next_node(index, count);
        }
        assert_eq!(walked, expected, "{count} nodes");
    }
    assert_eq!(next_node(u32::MAX / 2, u32::MAX), Some(u32::MAX / 2 + 1));
}

#[test]
fn battles_follow_the_tree_preorder_with_exact_bits_and_member_order() {
    let mut sounding = battle(2.0, 2.5, 9.0, &[30, 10, 20]);
    sounding.sound = 0x0000_0001_0000_0042;
    let mut odd = battle(-0.0, 7.25, 1.0, &[44]);
    odd.radius = 0x7fc0_1234;
    let placed = vec![
        (0, vec![odd]),
        (1, vec![sounding, battle(3.0, 0.0, 9.0, &[])]),
        (2, vec![battle(6.0, 0.0, 9.0, &[60])]),
        (5, vec![battle(4.0, 0.0, 9.0, &[40, 40])]),
        (6, vec![battle(5.0, 0.0, 9.0, &[50])]),
    ];
    let world = world(7, &placed);
    let mut rows = Vec::new();
    let mut provinces = Vec::new();
    let walk = walk(&world, 0, &mut rows, &mut provinces);
    assert_eq!(walk.outcome, Outcome::Rows { len: 6 });
    assert_eq!(
        walk.required,
        Demand {
            battles: 6,
            provinces: 8
        }
    );
    assert_eq!(walk.provinces, 8);
    let xs: Vec<f32> = rows[..6]
        .iter()
        .map(|row| f32::from_bits(row.center_x))
        .collect();
    assert_eq!(xs, [-0.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(rows[0].center_x, 0x8000_0000);
    assert_eq!(rows[0].center_z, 7.25_f32.to_bits());
    assert_eq!(rows[0].radius, 0x7fc0_1234);
    assert!(
        rows[..6]
            .iter()
            .enumerate()
            .all(|(index, row)| row.ordinal == index as u32)
    );
    assert_eq!(
        rows[..6].iter().map(|row| row.sounding).collect::<Vec<_>>(),
        [false, true, false, false, false, false]
    );
    let members =
        |row: &Row| &provinces[row.provinces_start..row.provinces_start + row.provinces_len];
    assert_eq!(members(&rows[1]), [30, 10, 20]);
    assert_eq!(members(&rows[2]), [] as [u32; 0]);
    assert_eq!(members(&rows[3]), [40, 40]);
    assert_eq!(members(&rows[5]), [60]);
    assert_eq!(demand(&world, 0), Some(walk.required));
}

#[test]
fn a_null_manager_holds_no_battles_and_an_unreadable_one_refuses() {
    let mut world = World::default();
    world.put(MANAGER_RVA.0, &0_u64.to_le_bytes());
    let walk = walk(&world, 0, &mut Vec::new(), &mut Vec::new());
    assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
    assert_eq!(walk.required, Demand::default());
    let empty = World::default();
    assert_eq!(
        super::walk(&empty, 0, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "battle sound manager pointer unreadable"
        }
    );
    assert_eq!(demand(&empty, 0), None);
}

#[test]
fn growing_storage_reads_the_whole_list_and_reports_its_demand() {
    let placed = vec![
        (0, vec![battle(1.0, 0.0, 9.0, &[1, 2])]),
        (
            1,
            vec![
                battle(2.0, 0.0, 9.0, &[3]),
                battle(3.0, 0.0, 9.0, &[4, 5, 6]),
            ],
        ),
    ];
    let world = world(5, &placed);
    let required = Demand {
        battles: 3,
        provinces: 6,
    };
    let (mut rows, mut provinces) = (Vec::new(), Vec::new());
    let walk = super::walk(&world, 0, &mut rows, &mut provinces);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!(walk.required, required);
    assert_eq!(&provinces[..6], [1, 2, 3, 4, 5, 6]);
}

#[test]
fn invalid_objects_refuse_by_name() {
    let placed = vec![(0, vec![battle(1.0, 0.0, 9.0, &[7])])];
    let refused = |change: &dyn Fn(&mut World)| {
        let mut world = world(1, &placed);
        change(&mut world);
        match super::walk(&world, 0, &mut Vec::new(), &mut Vec::new()).outcome {
            Outcome::Unavailable { reason } => reason,
            outcome => panic!("{outcome:?}"),
        }
    };
    assert_eq!(
        refused(&|world| world.put(province_at(7), &0x1234_u64.to_le_bytes())),
        "battle sound member is not a province"
    );
    assert_eq!(
        refused(&|world| world.put(province_at(7) + cprovince::ID, &0_i32.to_le_bytes())),
        "battle sound member province id not positive"
    );
    assert_eq!(
        refused(&|world| world.pointers(MEMBERS, &[0])),
        "battle sound member province null"
    );
    assert_eq!(
        refused(&|world| world.pointers(NODE_LISTS, &[0])),
        "battle sound battle pointer null"
    );
    assert_eq!(
        refused(&|world| world.pointers(NODE_LISTS, &[0x0900_0000])),
        "battle sound battle unreadable"
    );
    assert_eq!(
        refused(&|world| world.array(BATTLES + BATTLE_PROVINCES, MEMBERS, 1, 2)),
        "battle sound member array invalid"
    );
    assert_eq!(
        refused(&|world| world.array(MANAGER + MANAGER_TREE + TREE_NODES, NODES, 1, 2)),
        "battle sound tree header invalid"
    );
    assert_eq!(
        refused(&|world| world.put(MANAGER_RVA.0, &0x0910_0000_u64.to_le_bytes())),
        "battle sound tree unreadable"
    );
}

#[test]
fn battles_keep_the_preorder_from_empty_storage() {
    const COUNT: u32 = 85;
    const BATTLES_TOTAL: usize = 500;
    let mut placed: Vec<(u32, Vec<Battle>)> = (0..COUNT).map(|node| (node, Vec::new())).collect();
    for index in 0..BATTLES_TOTAL {
        let node = (index * 7 % COUNT as usize) as u32;
        let id = index as u32 + 1;
        placed[node as usize]
            .1
            .push(battle(index as f32, 0.0, 9.0, &[id, id + 10_000]));
    }
    let world = world(COUNT, &placed);
    let mut order = Vec::new();
    preorder(0, COUNT, &mut order);
    let expected: Vec<u32> = order
        .iter()
        .flat_map(|&node| {
            placed[node as usize]
                .1
                .iter()
                .map(|battle| battle.center[0])
        })
        .collect();
    let (mut rows, mut provinces) = (Vec::new(), Vec::new());
    let walk = walk(&world, 0, &mut rows, &mut provinces);
    assert_eq!(walk.outcome, Outcome::Rows { len: BATTLES_TOTAL });
    assert_eq!(
        walk.required,
        Demand {
            battles: BATTLES_TOTAL,
            provinces: 2 * BATTLES_TOTAL
        }
    );
    let walked: Vec<u32> = rows[..BATTLES_TOTAL]
        .iter()
        .map(|row| row.center_x)
        .collect();
    assert_eq!(walked, expected);
    let first = rows[0];
    let id = f32::from_bits(first.center_x) as u32 + 1;
    assert_eq!(
        provinces[first.provinces_start..first.provinces_start + 2],
        [id, id + 10_000]
    );
}
