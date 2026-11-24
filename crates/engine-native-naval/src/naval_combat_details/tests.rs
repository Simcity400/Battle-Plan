use engine_native::cnaval_combat::{
    OBSERVED_REFERENCE_TYPE,
    Row,
    walk,
    walk_timed,
    storage_demand,
};
use super::*;
use engine_native::{cfex_member, cship};
use engine_native_memory::memory::SliceMemory;

fn capture_details(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    details: &mut Vec<DetailsRow>,
    ships: &mut Vec<crate::naval_member_details::DetailsRow>,
    groups: &mut Vec<usize>,
    buffers: &mut crate::naval_member_details::DetailBuffers<'_>,
) -> Capture {
    walk_details_timed(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(details),
        ships,
        groups,
        buffers,
        |_| 0,
    )
    .capture
}

const IMAGE: usize = 0x1400_0000;
const GS: usize = 0x1000;
const BODY: usize = 0x2000;
const ATTACKING: usize = 0x3000;
const DEFENDING: usize = 0x4000;
const GROUP_A: usize = 0x5000;
const GROUP_D: usize = 0x6000;
const MEMBER_A: usize = 0x7000;
const MEMBER_D: usize = 0x8000;
const FRONT: usize = 0x9000;
const ROOTS: usize = 0xa000;
const GROUP_ROOT_A: usize = 0xa100;
const GROUP_ROOT_D: usize = 0xa200;
const MEMBERS_A: usize = 0xa300;
const MEMBERS_D: usize = 0xa400;

struct World(Vec<u8>);

impl World {
    fn u64(&mut self, address: usize, value: u64) {
        self.0[address - GS..address - GS + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, address: usize, value: u32) {
        self.0[address - GS..address - GS + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn array(&mut self, address: usize, data: usize, capacity: u32, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, capacity);
        self.u32(address + 12, count);
    }

    fn new() -> Self {
        let mut world = Self(vec![0; 0xa000]);
        world.array(GS + cgame_state::COMBATS + 8, ROOTS, 2, 1);
        world.u64(ROOTS, BODY as u64);
        world.u64(BODY, (IMAGE + VTABLE_RVA.0) as u64);
        world.u64(
            BODY + HANDLE,
            (u64::from(u32::MAX) << 32) | u64::from(OBSERVED_REFERENCE_TYPE + 17),
        );
        world.u64(BODY + ATTACKER, ATTACKING as u64);
        world.u64(BODY + DEFENDER, DEFENDING as u64);
        world.u64(BODY + PROVINCE, FRONT as u64);
        world.u64(FRONT, (IMAGE + cprovince::VTABLE_RVA.0) as u64);
        world.u32(FRONT + cprovince::ID, 42);
        world.array(GS + cgame_state::PROVINCES, 0xa500, 43, 43);
        world.u64(0xa500 + 42 * 8, FRONT as u64);
        for (side, roots, group, other, members, member, slot) in [
            (
                ATTACKING,
                GROUP_ROOT_A,
                GROUP_A,
                GROUP_D,
                MEMBERS_A,
                MEMBER_A,
                u32::MAX,
            ),
            (
                DEFENDING,
                GROUP_ROOT_D,
                GROUP_D,
                GROUP_A,
                MEMBERS_D,
                MEMBER_D,
                u32::MAX,
            ),
        ] {
            world.u64(side, (IMAGE + cnaval_combatant::VTABLE_RVA.0) as u64);
            world.u64(side + cnaval_combatant::COMBAT, BODY as u64);
            let sentinel = if side == ATTACKING { 0xa680 } else { 0xa6a0 };
            world.u64(side + 0x2b0, sentinel as u64);
            world.u64(sentinel, sentinel as u64);
            world.array(side + cnaval_combatant::GROUPS, roots, 2, 1);
            world.u64(roots, group as u64);
            world.u64(side + cnaval_combatant::POSITIONING, i64::MIN as u64);
            world.u64(side + cnaval_combatant::ANTI_AIR, i64::MAX as u64);
            world.u64(group, (IMAGE + cfex_group::VTABLE_RVA.0) as u64);
            world.u64(group + cfex_group::SIDE, side as u64);
            world.u64(group + cfex_group::OPPONENT_GROUP, other as u64);
            world.array(group + cfex_group::MEMBERS, members, 3, 1);
            world.u64(members, member as u64);
            world.u64(member, (IMAGE + cfex_member::VTABLE_RVA.0) as u64);
            world.u64(member + cfex_member::GROUP, group as u64);
            world.u64(
                member + cfex_member::SHIP_HANDLE,
                (u64::from(u32::MAX) << 32) | u64::from(cship::HANDLE_TYPE),
            );
            world.u32(member + cfex_member::BATTLE_SLOT, slot);
            world.u32(member + cfex_member::OWNER, u32::MAX);
            world.u32(member + cfex_member::STATE, 0x8000_0020);
        }
        world.u64(MEMBER_D + cfex_member::SHIP_HANDLE, 0);
        world
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: GS,
            bytes: &self.0,
        }
    }
}

struct Fault<'a> {
    memory: SliceMemory<'a>,
    address: usize,
}

struct DetailMemory<'a>(SliceMemory<'a>);
impl Memory for DetailMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        use engine_native_parts::modifier_definition as md;
        use crate::naval_combat_summary_inputs as summary;
        if address == IMAGE + md::TABLE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x100000u64.to_le_bytes());
            return true;
        }
        if (address == IMAGE + md::ELIGIBILITY_CALLBACK_RVA.0 && output.len() == 8)
            || (address == IMAGE + md::TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4)
            || (output.len() == 4
                && summary::MODIFIER_KEYS.iter().any(|key| {
                    let row = 0x100000 + *key as usize * md::STRIDE;
                    address == row + md::FLAGS || address == row + md::FEATURE_SELECTOR
                }))
        {
            output.fill(0);
            return true;
        }
        self.0.copy(address, output)
    }
}

fn detail_world() -> World {
    let mut world = World::new();
    world.u64(BODY + 0x28, ATTACKING as u64);
    world.u64(BODY + 0x30, DEFENDING as u64);
    world.0[ATTACKING + 0xd8 - GS] = 1;
    for member in [MEMBER_A, MEMBER_D] {
        world.u64(member + cfex_member::SHIP_HANDLE, 0);
        for offset in [0x48, 0x68, 0x88, 0xa8] {
            world.u64(member + offset + 24, 15);
        }
        world.u64(member + cfex_member::BUILD_COST_IC, i64::MAX as u64);
        world.u64(member + cfex_member::CACHED_STRENGTH, i64::MIN as u64);
        world.u64(member + cfex_member::ESCAPE_PROGRESS, i64::MAX as u64);
        world.u32(member + cfex_member::RECEIVED_CRITICALS, u32::MAX);
        world.u64(member + cfex_member::RECEIVED_DAMAGE, i64::MIN as u64);
    }
    world.u64(ATTACKING + cnaval_combatant::SCREENING, i64::MIN as u64);
    world.u64(DEFENDING + cnaval_combatant::SCREENING + 8, i64::MAX as u64);
    world
}

#[test]
fn unreadable_custom_sea_grid_does_not_withhold_live_combat_details() {
    const REGION: usize = 0xb000;
    const TEMPLATE: usize = 0xc000;
    let mut world = detail_world();
    world.0.resize(0xd000 - GS, 0);
    world.u64(FRONT + cprovince::STRATEGIC_REGION, REGION as u64);
    world.u64(
        REGION,
        (IMAGE + engine_native_parts::cstrategic_region::VTABLE_RVA.0) as u64,
    );
    world.u32(REGION + engine_native_parts::cstrategic_region::ID, 94);
    world.u64(REGION + 0x30, TEMPLATE as u64);
    world.u64(TEMPLATE, (IMAGE + 0x02a5_5348) as u64);
    world.u32(TEMPLATE + 0x140, 1);
    world.u32(TEMPLATE + 0x144, 1);
    let (capture, row, _, _, _) = walk_whole(&world, 0);
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(row.custom_sea, None);
    assert_eq!(
        row.custom_sea_error,
        Some("weather manager pointer unreadable or absent")
    );
}

#[test]
fn side_units_keep_stored_order_and_report_their_demand() {
    const SLOTS: usize = 0xa700;
    const FORCES: [usize; 2] = [0xa800, 0xa900];
    let handles = [(900_u64 << 32) | 61, (244 << 32) | 61];
    let mut world = detail_world();
    world.array(ATTACKING + cnaval_combatant::UNITS, SLOTS, 4, 2);
    for (index, (force, handle)) in FORCES.into_iter().zip(handles).enumerate() {
        world.u64(SLOTS + index * 8, force as u64);
        world.u64(force, (IMAGE + engine_native::ctask_force::VTABLE_RVA.0) as u64);
        world.u64(
            force + engine_native::ctask_force::SECONDARY_BASE,
            (IMAGE + engine_native::ctask_force::SECONDARY_VTABLE_RVA.0) as u64,
        );
        world.u64(force + engine_native::ctask_force::SECONDARY_BASE + 8, handle);
    }
    let run = |world: &World, arena: &mut Vec<u64>| {
        let mut details = vec![DetailsRow::default(); 1];
        let mut buffers = crate::naval_member_details::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes: engine_native_memory::walk::Buffer::Count,
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: engine_native_memory::walk::Buffer::Count,
            plane_naval_hits: engine_native_memory::walk::Buffer::Count,
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: engine_native_memory::walk::Buffer::Count,
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            doctrine_keys: Default::default(),
            term_metadata: Default::default(),
            source_memo: Default::default(),
            commander_inputs: engine_native_memory::walk::Buffer::Count,
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: engine_native_memory::walk::Buffer::Growing(arena),
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]),
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: engine_native_memory::walk::Buffer::Growing(&mut vec![(0, 0); 64]),
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 512]),
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1024]),
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        let combats = walk_details_timed(
            &DetailMemory(world.memory()),
            IMAGE,
            GS,
            engine_native_memory::walk::Buffer::Growing(&mut details),
            &mut vec![crate::naval_member_details::DetailsRow::default(); 2],
            &mut vec![0; 4],
            &mut buffers,
            |_| 0,
        )
        .capture
        .combats;
        (
            combats,
            details[0].sides.clone().map(|side| side.units),
            buffers.side_units_len,
            buffers.side_units_required,
        )
    };
    let mut arena = Vec::new();
    let (combats, units, len, required) = run(&world, &mut arena);
    assert_eq!(combats, Outcome::Rows { len: 1 });
    assert_eq!(units, [[0, 2], [2, 0]]);
    assert_eq!((len, required), (2, 2));
    assert_eq!(arena[..2], handles);
    world.u64(FORCES[1], (IMAGE + cship::VTABLE_RVA.0) as u64);
    assert!(matches!(
        run(&world, &mut Vec::new()).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn null_commander_never_falls_back_to_the_last_leader() {
    let mut world = detail_world();
    world.u64(ATTACKING + 0x118, u64::MAX);
    let side = cnaval_combatant::Side {
        pointer: ATTACKING,
        scalars: Ok((0, 0)),
    };
    let opponent = cnaval_combatant::Side {
        pointer: DEFENDING,
        scalars: Ok((0, 0)),
    };
    let details = |memory: &DetailMemory<'_>| {
        let (mut countries, mut units) = (Vec::new(), vec![0; 64]);
        let (mut countries_len, mut units_len, mut units_required) = (0, 0, 0);
        let (mut damage_sources, mut damage_sources_len, mut damage_sources_required) =
            (vec![(0, 0); 64], 0, 0);
        let mut arenas = crate::naval_side_details::SideArenas {
            countries: &mut countries,
            countries_len: &mut countries_len,
            units: engine_native_memory::walk::Buffer::Growing(&mut units),
            units_len: &mut units_len,
            units_required: &mut units_required,
            damage_sources: engine_native_memory::walk::Buffer::Growing(&mut damage_sources),
            damage_sources_len: &mut damage_sources_len,
            damage_sources_required: &mut damage_sources_required,
        };
        crate::naval_side_details::details(memory, IMAGE, GS, BODY, [&side, &opponent], &mut arenas)
    };
    assert_eq!(details(&DetailMemory(world.memory())).unwrap().commander, 0);
    world.u64(ATTACKING + cnaval_combatant::COMMANDER, u64::MAX);
    assert!(details(&DetailMemory(world.memory())).is_err());
}

#[test]
fn side_external_planes_keep_their_first_group_owner() {
    const PLANE: usize = 0xa900;
    let mut world = detail_world();
    world.u64(PLANE, (IMAGE + engine_native::cfex_air::VTABLE_RVA.0) as u64);
    world.u64(PLANE + 8, GROUP_A as u64);
    world.u64(PLANE + 0x88 + engine_native_memory::msvc_string::CAPACITY, 15);
    world.u64(0xa880, PLANE as u64);
    world.array(GROUP_A + 0x58, 0xa880, 1, 1);
    world.array(ATTACKING + 0x100, 0xa880, 1, 1);
    let mut planes = vec![crate::naval_air_details::Row::default(); 4];
    let run = |world: &World, planes: &mut Vec<crate::naval_air_details::Row>| {
        let mut rows = vec![DetailsRow::default(); 1];
        let mut buffers = crate::naval_member_details::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes: engine_native_memory::walk::Buffer::Growing(planes),
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: engine_native_memory::walk::Buffer::Count,
            plane_naval_hits: engine_native_memory::walk::Buffer::Count,
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: engine_native_memory::walk::Buffer::Count,
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            doctrine_keys: Default::default(),
            term_metadata: Default::default(),
            source_memo: Default::default(),
            commander_inputs: engine_native_memory::walk::Buffer::Count,
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: engine_native_memory::walk::Buffer::Count,
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]),
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: engine_native_memory::walk::Buffer::Growing(&mut vec![(0, 0); 64]),
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 512]),
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1024]),
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        let result = capture_details(
            &DetailMemory(world.memory()),
            IMAGE,
            GS,
            &mut rows,
            &mut vec![crate::naval_member_details::DetailsRow::default(); 2],
            &mut vec![0; 4],
            &mut buffers,
        );
        (result.combats, rows[0].sides[0].planes, buffers.planes_len)
    };
    let (outcome, side_planes, len) = run(&world, &mut planes);
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!((side_planes, len), ([0, 2], 2));
    assert_eq!((planes[0].source, planes[1].source), (0, 2));
    world.array(GROUP_A + 0x58, 0, 0, 0);
    world.u64(PLANE + 8, 0);
    assert_eq!(run(&world, &mut planes).0, Outcome::Rows { len: 1 });
    world.u64(PLANE + 8, GROUP_D as u64);
    assert!(matches!(
        run(&world, &mut planes).0,
        Outcome::Unavailable { .. }
    ));
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address <= self.address && self.address < address.saturating_add(destination.len()) {
            destination.fill(0x7e);
            false
        } else {
            self.memory.copy(address, destination)
        }
    }
}

#[test]
fn paired_groups_and_full_native_values_survive_without_narrowing() {
    let mut world = World::new();
    world.array(DEFENDING + cnaval_combatant::GROUPS, 0, 0, 0);
    let mut combats = vec![Row::default(); 2];
    let mut members = vec![cnaval_combatant::Row::default(); 4];
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; 8],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    assert_eq!(capture.required_group_capacity, 2);
    assert_eq!(combats[0].attacker_positioning, i64::MIN);
    assert_eq!(combats[0].defender_anti_air, i64::MAX);
    assert_eq!(members[0].battle_slot, u32::MAX);
    assert_eq!(members[0].state, 0x8000_0020);
    assert_eq!(members[0].owner, u32::MAX);
    assert_eq!(members[1].ship, 0);
    assert_ne!(members[0].key(), members[1].key());
    assert!(members[0].key() > u128::from(u64::MAX));
    assert_eq!(members[0].combat, combats[0].id);
}

#[test]
fn core_combat_rows_groups_and_members_follow_first_and_later_mod_headers() {
    let mut combats = Vec::new();
    let mut participants = Vec::new();
    let mut groups = Vec::new();
    for count in [50_usize, 120] {
        let mut world = World::new();
        let registry = GS + world.0.len();
        world.0.resize(world.0.len() + count * 8, 0);
        world.array(
            GS + cgame_state::COMBATS + 8,
            registry,
            count as u32,
            count as u32,
        );
        for index in 0..count {
            world.u64(registry + index * 8, BODY as u64);
        }
        for side in [ATTACKING, DEFENDING] {
            world.array(side + cnaval_combatant::GROUPS, 0, 0, 0);
        }
        let captured = walk_timed(
            &world.memory(),
            IMAGE,
            GS,
            &mut combats,
            &mut participants,
            &mut groups,
            |_| 0,
        )
        .capture;
        assert_eq!(captured.combats, Outcome::Rows { len: count });
        assert_eq!(combats[count - 1].registry_ordinal, count as u32 - 1);

        let mut world = World::new();
        let blocks = GS + world.0.len();
        let roots = blocks + count * 0x300;
        world.0.resize(world.0.len() + count * (0x300 + 8), 0);
        world.array(
            ATTACKING + cnaval_combatant::GROUPS,
            roots,
            count as u32,
            count as u32,
        );
        world.array(DEFENDING + cnaval_combatant::GROUPS, 0, 0, 0);
        for index in 0..count {
            let block = blocks + index * 0x300;
            world.u64(roots + index * 8, block as u64);
            for (group, opponent, member, slots, side) in [
                (block, block + 0x40, block + 0x80, block + 0x2e0, ATTACKING),
                (block + 0x40, block, block + 0x1b0, block + 0x2e8, DEFENDING),
            ] {
                world.u64(group, (IMAGE + cfex_group::VTABLE_RVA.0) as u64);
                world.u64(group + cfex_group::SIDE, side as u64);
                world.u64(group + cfex_group::OPPONENT_GROUP, opponent as u64);
                world.array(group + cfex_group::MEMBERS, slots, 1, 1);
                world.u64(slots, member as u64);
                world.u64(member, (IMAGE + cfex_member::VTABLE_RVA.0) as u64);
                world.u64(member + cfex_member::GROUP, group as u64);
                world.u32(member + cfex_member::BATTLE_SLOT, index as u32);
            }
        }

        let demand = storage_demand(&world.memory(), IMAGE, GS);
        assert_eq!(
            demand.combats,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            demand.participants,
            Outcome::Truncated {
                len: 0,
                unread_rows: (count * 2) as u32
            }
        );
        assert_eq!(demand.required_group_capacity, count * 2);
        let captured = walk_timed(
            &world.memory(),
            IMAGE,
            GS,
            &mut combats,
            &mut participants,
            &mut groups,
            |_| 0,
        )
        .capture;
        assert_eq!(captured.combats, Outcome::Rows { len: 1 });
        assert_eq!(captured.participants, Outcome::Rows { len: count * 2 });
        assert_eq!(captured.required_group_capacity, count * 2);
        assert!(
            participants[..count * 2]
                .iter()
                .enumerate()
                .all(|(index, row)| row.battle_slot == (index / 2) as u32)
        );
    }
}

#[test]
fn invalid_membership_does_not_erase_readable_combat_location() {
    let mut world = World::new();
    world.u64(MEMBER_A + cfex_member::GROUP, GROUP_D as u64);
    let mut combats = vec![Row::default(); 2];
    let mut members = vec![cnaval_combatant::Row::default(); 4];
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(
        capture.participants,
        Outcome::Unavailable {
            reason: "naval member identity or group back-reference mismatch"
        }
    );
    world.u32(ATTACKING + cnaval_combatant::GROUPS_COUNT, 3);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(
        capture.participants,
        Outcome::Unavailable {
            reason: "naval side group array unreadable or invalid"
        }
    );
    let mut world = World::new();
    world.u64(MEMBERS_A, 0);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(
        capture.participants,
        Outcome::Unavailable {
            reason: "null naval group member"
        }
    );
}

#[test]
fn combat_payload_faults_preserve_all_participants_but_identity_faults_do_not() {
    let mut world = World::new();
    let mut expected_members = vec![cnaval_combatant::Row::default(); 4];
    let expected = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut vec![Row::default(); 1],
        &mut expected_members,
        &mut vec![0; 4],
    );
    assert_eq!(expected.participants, Outcome::Rows { len: 2 });
    for address in [
        ATTACKING + cnaval_combatant::POSITIONING,
        ATTACKING + cnaval_combatant::ANTI_AIR,
        DEFENDING + cnaval_combatant::POSITIONING,
        DEFENDING + cnaval_combatant::ANTI_AIR,
        BODY + PROVINCE,
        FRONT + cprovince::ID,
        0xa500 + 42 * 8,
    ] {
        let fault = Fault {
            memory: world.memory(),
            address,
        };
        let mut members = vec![cnaval_combatant::Row::default(); 4];
        let capture = walk(
            &fault,
            IMAGE,
            GS,
            &mut vec![Row::default(); 1],
            &mut members,
            &mut vec![0; 4],
        );
        assert!(matches!(capture.combats, Outcome::Unavailable { .. }));
        assert_eq!(capture.participants, expected.participants);
        assert_eq!(members, expected_members);
    }
    for address in [ATTACKING, ATTACKING + cnaval_combatant::COMBAT] {
        let fault = Fault {
            memory: world.memory(),
            address,
        };
        let capture = walk(
            &fault,
            IMAGE,
            GS,
            &mut vec![Row::default(); 1],
            &mut vec![cnaval_combatant::Row::default(); 4],
            &mut vec![0; 4],
        );
        assert!(matches!(capture.combats, Outcome::Unavailable { .. }));
        assert_eq!(capture.participants, capture.combats);
    }
    world.u64(ATTACKING + cnaval_combatant::COMBAT, BODY as u64 + 8);
    let capture = walk(
        &Fault {
            memory: world.memory(),
            address: ATTACKING + cnaval_combatant::POSITIONING,
        },
        IMAGE,
        GS,
        &mut vec![Row::default(); 1],
        &mut vec![cnaval_combatant::Row::default(); 4],
        &mut vec![0; 4],
    );
    assert_eq!(
        capture.combats,
        Outcome::Unavailable {
            reason: "naval side identity or combat back-reference mismatch"
        }
    );
    assert_eq!(capture.participants, capture.combats);
}

#[test]
fn a_registry_past_4096_combats_is_walked_whole_from_any_starting_storage() {
    const REGISTRY: usize = 0x10000;
    const LAND: usize = 0xa800;
    let entries = 2 * 4096 + 1;
    let is_naval = |index: usize| index % 1024 == 1023;
    let naval = (0..entries).filter(|&index| is_naval(index)).count();
    let mut world = detail_world();
    world.0.resize(REGISTRY + entries * 8 - GS, 0);
    world.u64(LAND, (IMAGE + cland_combat::VTABLE_RVA.0) as u64);
    world.array(
        GS + cgame_state::COMBATS + 8,
        REGISTRY,
        entries as u32,
        entries as u32,
    );
    for index in 0..entries {
        let body = if is_naval(index) { BODY } else { LAND };
        world.u64(REGISTRY + index * 8, body as u64);
    }
    let memory = DetailMemory(world.memory());
    let mut combats = vec![Row::default(); naval];
    let capture = walk(
        &memory,
        IMAGE,
        GS,
        &mut combats,
        &mut vec![cnaval_combatant::Row::default(); 2 * naval],
        &mut vec![0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: naval });
    assert_eq!(capture.participants, Outcome::Rows { len: 2 * naval });
    assert_eq!(combats[naval - 1].registry_ordinal, 8191);
    let details = |capacity: usize| {
        let mut rows = vec![DetailsRow::default(); capacity];
        let mut buffers = crate::naval_member_details::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes: engine_native_memory::walk::Buffer::Count,
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: engine_native_memory::walk::Buffer::Count,
            plane_naval_hits: engine_native_memory::walk::Buffer::Count,
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: engine_native_memory::walk::Buffer::Count,
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            doctrine_keys: Default::default(),
            term_metadata: Default::default(),
            source_memo: Default::default(),
            commander_inputs: engine_native_memory::walk::Buffer::Count,
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: engine_native_memory::walk::Buffer::Count,
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]),
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: engine_native_memory::walk::Buffer::Growing(&mut vec![(0, 0); 64]),
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 512]),
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1024]),
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        capture_details(
            &memory,
            IMAGE,
            GS,
            &mut rows,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut buffers,
        )
    };
    let whole = details(naval);
    assert_eq!(whole.combats, Outcome::Rows { len: naval });
    assert_eq!(whole.participants, Outcome::Rows { len: 2 * naval });
    let short = details(2);
    assert_eq!(short.combats, Outcome::Rows { len: naval });
    assert_eq!(short.participants, Outcome::Rows { len: 2 * naval });
}

#[allow(clippy::type_complexity)]
fn walk_whole(
    world: &World,
    plane_capacity: usize,
) -> (
    Capture,
    DetailsRow,
    Vec<crate::naval_member_details::DetailsRow>,
    usize,
    usize,
) {
    walk_whole_mode(world, plane_capacity, false)
}

fn walk_whole_mode(
    world: &World,
    planes: usize,
    growing: bool,
) -> (
    Capture,
    DetailsRow,
    Vec<crate::naval_member_details::DetailsRow>,
    usize,
    usize,
) {
    let mut rows = vec![DetailsRow::default(); 1];
    let mut ships = Vec::new();
    let mut plane_rows = vec![crate::naval_air_details::Row::default(); planes];
    let mut buffers = crate::naval_member_details::DetailBuffers {
        damage: &mut Vec::new(),
        air_hits: &mut Vec::new(),
        naval_hits: &mut Vec::new(),
        lengths: [0; 3],
        required: [0; 3],
        planes: if growing {
            engine_native_memory::walk::Buffer::Growing(&mut plane_rows)
        } else {
            engine_native_memory::walk::Buffer::Growing(&mut plane_rows)
        },
        planes_len: 0,
        planes_required: 0,
        plane_air_hits: engine_native_memory::walk::Buffer::Count,
        plane_naval_hits: engine_native_memory::walk::Buffer::Count,
        plane_hit_lengths: [0; 2],
        plane_hit_required: [0; 2],
        plane_kills: engine_native_memory::walk::Buffer::Count,
        plane_kills_len: 0,
        plane_kills_required: 0,
        weapon_sources: &mut Vec::new(),
        weapon_terms: &mut Vec::new(),
        weapon_lengths: [0; 2],
        weapon_required: [0; 2],
        doctrine_keys: Default::default(),
        term_metadata: Default::default(),
        source_memo: Default::default(),
        commander_inputs: engine_native_memory::walk::Buffer::Count,
        commander_inputs_len: 0,
        commander_inputs_required: 0,
        commander_timed_traits: &mut Vec::new(),
        commander_timed_traits_len: 0,
        side_units: engine_native_memory::walk::Buffer::Count,
        side_units_len: 0,
        side_units_required: 0,
        summary_countries: &mut Vec::new(),
        summary_countries_len: 0,
        custom_sea: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]),
        custom_sea_len: 0,
        custom_sea_required: 0,
        damage_sources: engine_native_memory::walk::Buffer::Growing(&mut vec![(0, 0); 64]),
        damage_sources_len: 0,
        damage_sources_required: 0,
        wing_types: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 512]),
        wing_types_len: 0,
        wing_types_required: 0,
        plane_wings: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 1024]),
        plane_wings_len: 0,
        plane_wings_required: 0,
    };
    let capture = walk_details_timed(
        &DetailMemory(world.memory()),
        IMAGE,
        GS,
        if growing {
            engine_native_memory::walk::Buffer::Growing(&mut rows)
        } else {
            engine_native_memory::walk::Buffer::Growing(&mut rows)
        },
        &mut ships,
        &mut Vec::new(),
        &mut buffers,
        |_| 0,
    )
    .capture;
    let (len, required) = (buffers.planes_len, buffers.planes_required);
    ships.truncate(capture.participants.len());
    (capture, rows[0].clone(), ships, len, required)
}

#[test]
fn groups_members_and_planes_past_the_old_bounds_are_read_whole() {
    const ARRAY: usize = 0x80000;
    const GROUPS: usize = 0x100000;
    let size = 0x600000 - GS;
    let mut world = detail_world();
    world.0.resize(size, 0);
    world.array(ATTACKING + cnaval_combatant::GROUPS, ARRAY, 8193, 8193);
    for index in 0..8192 {
        let group = GROUPS + index * 0x100;
        world.u64(ARRAY + index * 8, group as u64);
        world.u64(group, (IMAGE + cfex_group::VTABLE_RVA.0) as u64);
        world.u64(group + cfex_group::SIDE, ATTACKING as u64);
    }
    world.u64(ARRAY + 8192 * 8, GROUP_A as u64);
    let (capture, _, ships, _, _) = walk_whole(&world, 0);
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    assert_eq!(
        ships
            .iter()
            .map(|ship| ship.group_index)
            .collect::<Vec<_>>(),
        [8192, 0]
    );
    world.u64(ARRAY, GROUP_A as u64);
    assert_eq!(
        walk_whole(&world, 0).0.participants,
        Outcome::Unavailable {
            reason: "naval duplicate forward group"
        }
    );

    let mut world = detail_world();
    world.0.resize(size, 0);
    world.array(GROUP_A + cfex_group::MEMBERS, ARRAY, 65_537, 65_537);
    for index in 0..65_537 {
        world.u64(ARRAY + index * 8, MEMBER_A as u64);
    }
    let (capture, _, ships, _, _) = walk_whole(&world, 0);
    assert_eq!(capture.participants, Outcome::Rows { len: 65_538 });
    assert_eq!(ships[65_536].member_index, 65_536);

    const PLANE: usize = 0xa900;
    let mut world = detail_world();
    world.0.resize(size, 0);
    world.u64(PLANE, (IMAGE + engine_native::cfex_air::VTABLE_RVA.0) as u64);
    world.u64(PLANE + 8, GROUP_A as u64);
    world.u64(PLANE + 0x88 + engine_native_memory::msvc_string::CAPACITY, 15);
    world.array(GROUP_A + 0x58, ARRAY, 16_385, 16_385);
    for index in 0..16_385 {
        world.u64(ARRAY + index * 8, PLANE as u64);
    }
    let (capture, row, _, len, required) = walk_whole(&world, 4);
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(required, 16_385);
    assert_eq!((row.sides[0].planes, len), ([0, 16_385], 16_385));
}

#[test]
fn active_wing_vectors_exceed_256() {
    const COUNT: usize = 257;
    const PLANE: usize = 0xa900;
    const ARRAY: usize = 0xc000;
    let mut world = World::new();
    world.0.resize(ARRAY + COUNT * 16 - GS, 0);
    world.u64(PLANE, (IMAGE + engine_native::cfex_air::VTABLE_RVA.0) as u64);
    world.u64(PLANE + 8, GROUP_A as u64);
    world.array(PLANE + 0x28, ARRAY, COUNT as u32, COUNT as u32);
    world.array(PLANE + 0x40, ARRAY + COUNT * 8, COUNT as u32, COUNT as u32);
    for index in 0..COUNT {
        world.u64(ARRAY + index * 8, index as u64 + 1);
    }
    let mut wing_types = vec![0; COUNT];
    let mut wing_types_len = 0;
    let mut wing_types_required = 0;
    let mut plane_wings = vec![0; COUNT * 2];
    let mut plane_wings_len = 0;
    let mut plane_wings_required = 0;
    let row = crate::naval_air_details::read(
        &SliceMemory {
            base: GS,
            bytes: &world.0,
        },
        IMAGE,
        PLANE,
        GROUP_A,
        &mut wing_types,
        &mut wing_types_len,
        &mut wing_types_required,
        &mut plane_wings,
        &mut plane_wings_len,
        &mut plane_wings_required,
    )
    .unwrap();
    assert_eq!(
        row.wings
            .map(|span| plane_wings[span[0]..span[0] + span[1]].to_vec()),
        [(1..=COUNT as u64).collect::<Vec<_>>(), vec![0; COUNT]]
    );
    assert_eq!(row.wing_types, [0, COUNT]);
    assert_eq!(wing_types_required, COUNT);
    assert_eq!(plane_wings_required, COUNT * 2);
    assert_eq!(&wing_types[..wing_types_len], &[u32::MAX; COUNT]);
}

#[test]
fn member_damage_past_16384_grows_from_the_native_header() {
    const ARRAY: usize = 0x80000;
    const COUNT: usize = 16_385;
    let mut world = detail_world();
    world.0.resize(ARRAY + COUNT * 0x20 - GS, 0);
    world.array(
        MEMBER_A + cfex_member::DAMAGE_ENTRIES,
        ARRAY,
        COUNT as u32,
        COUNT as u32,
    );
    for index in 0..COUNT {
        let entry = ARRAY + index * 0x20;
        world.u64(entry, (IMAGE + engine_native::cfex_damage_entry::VTABLE_RVA.0) as u64);
        world.u64(entry + 0x18, i64::MIN as u64);
    }
    let (capture, _, ships, _, _) = walk_whole(&world, 0);
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    assert_eq!(ships[0].damage_entries, [0, COUNT]);
}

#[test]
fn empty_registry_is_valid_and_an_unreadable_registry_is_not_empty() {
    let mut world = World::new();
    world.array(GS + cgame_state::COMBATS + 8, 0, 0, 0);
    let capture = walk(&world.memory(), IMAGE, GS, &mut Vec::new(), &mut Vec::new(), &mut Vec::new());
    assert_eq!(capture.combats, Outcome::Rows { len: 0 });
    assert_eq!(capture.participants, Outcome::Rows { len: 0 });
    world.array(GS + cgame_state::COMBATS + 8, usize::MAX - 3, 1, 1);
    let capture = walk(&world.memory(), IMAGE, GS, &mut Vec::new(), &mut Vec::new(), &mut Vec::new());
    assert!(matches!(capture.combats, Outcome::Unavailable { .. }));
    assert!(matches!(capture.participants, Outcome::Unavailable { .. }));
}

#[test]
fn mixed_registry_skips_known_land_classes_and_refuses_null_or_unknown_entries() {
    let mut world = World::new();
    let land = 0xa800;
    let border = 0xa900;
    world.array(GS + cgame_state::COMBATS + 8, ROOTS, 3, 3);
    world.u64(ROOTS + 8, land as u64);
    world.u64(ROOTS + 16, border as u64);
    world.u64(land, (IMAGE + cland_combat::VTABLE_RVA.0) as u64);
    world.u64(border, (IMAGE + cland_border_war_combat::VTABLE_RVA.0) as u64);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut vec![Row::default(); 1],
        &mut vec![cnaval_combatant::Row::default(); 4],
        &mut vec![0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    world.u64(ROOTS, land as u64);
    world.u64(ROOTS + 8, BODY as u64);
    let mut combats = vec![Row::default(); 1];
    walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut vec![cnaval_combatant::Row::default(); 4],
        &mut vec![0; 4],
    );
    assert_eq!(combats[0].registry_ordinal, 1);
    world.u64(ROOTS, BODY as u64);
    world.u64(ROOTS + 8, land as u64);
    for null in [false, true] {
        if null {
            world.u64(ROOTS + 16, 0);
        } else {
            world.u64(border, (IMAGE + VTABLE_RVA.0 + 8) as u64);
        }
        let capture = walk(
            &world.memory(),
            IMAGE,
            GS,
            &mut vec![Row::default(); 1],
            &mut vec![cnaval_combatant::Row::default(); 4],
            &mut vec![0; 4],
        );
        assert_eq!(
            capture.combats,
            Outcome::Unavailable {
                reason: if null {
                    "null combat registry entry"
                } else {
                    "unclassified combat registry object"
                }
            }
        );
        assert_eq!(capture.participants, capture.combats);
    }
}

#[test]
fn root_membership_and_opposing_pair_are_authenticated() {
    let mut world = World::new();
    let mut combats = vec![Row::default(); 2];
    let mut members = vec![cnaval_combatant::Row::default(); 4];
    world.u64(0xa500 + 42 * 8, FRONT as u64 + 8);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; 4],
    );
    assert_eq!(
        capture.combats,
        Outcome::Unavailable {
            reason: "naval province registry pointer mismatch"
        }
    );
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    world.u64(0xa500 + 42 * 8, FRONT as u64);
    world.u64(GROUP_D + cfex_group::SIDE, ATTACKING as u64);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(
        capture.participants,
        Outcome::Unavailable {
            reason: "naval opposing group identity or side mismatch"
        }
    );
}

#[test]
fn cold_worker_reads_plane_populations_on_the_original_hour() {
    const PLANE: usize = 0xa900;
    const ARRAY: usize = 0x80000;
    for count in [15, 50, 165] {
        let mut world = detail_world();
        world.0.resize(ARRAY + count * 8 - GS, 0);
        world.u64(PLANE, (IMAGE + engine_native::cfex_air::VTABLE_RVA.0) as u64);
        world.u64(PLANE + 8, GROUP_A as u64);
        world.array(GROUP_A + 0x58, ARRAY, count as u32, count as u32);
        for index in 0..count {
            world.u64(ARRAY + index * 8, PLANE as u64);
        }
        let (capture, row, _, used, required) = walk_whole_mode(&world, 0, true);
        assert_eq!(capture.combats, Outcome::Rows { len: 1 });
        assert_eq!(capture.participants, Outcome::Rows { len: 2 });
        assert_eq!(row.sides[0].planes, [0, count]);
        assert_eq!((used, required), (count, count));
    }
}
