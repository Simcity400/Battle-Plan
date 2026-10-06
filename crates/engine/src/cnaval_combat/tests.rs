use super::*;
use crate::{cfex_member, cship, memory::SliceMemory};

fn capture_details(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    details: &mut [DetailsRow],
    ships: &mut Vec<crate::cfex_member::DetailsRow>,
    groups: &mut Vec<usize>,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
) -> Capture {
    walk_details_timed(
        memory,
        base,
        game_state,
        details,
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
        world.u64(BODY, (IMAGE + VTABLE_RVA) as u64);
        world.u64(
            BODY + HANDLE,
            (u64::from(u32::MAX) << 32) | u64::from(OBSERVED_REFERENCE_TYPE + 17),
        );
        world.u64(BODY + ATTACKER, ATTACKING as u64);
        world.u64(BODY + DEFENDER, DEFENDING as u64);
        world.u64(BODY + PROVINCE, FRONT as u64);
        world.u64(FRONT, (IMAGE + cprovince::VTABLE_RVA) as u64);
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
            world.u64(side, (IMAGE + cnaval_combatant::VTABLE_RVA) as u64);
            world.u64(side + cnaval_combatant::COMBAT, BODY as u64);
            let sentinel = if side == ATTACKING { 0xa680 } else { 0xa6a0 };
            world.u64(side + 0x290, sentinel as u64);
            world.u64(sentinel, sentinel as u64);
            // Capacity differs from count, exercising the corrected D0/D4 header.
            world.array(side + cnaval_combatant::GROUPS, roots, 2, 1);
            world.u64(roots, group as u64);
            world.u64(side + cnaval_combatant::POSITIONING, i64::MIN as u64);
            world.u64(side + cnaval_combatant::ANTI_AIR, i64::MAX as u64);
            world.u64(group, (IMAGE + cfex_group::VTABLE_RVA) as u64);
            world.u64(group + cfex_group::SIDE, side as u64);
            world.u64(group + cfex_group::OPPONENT_GROUP, other as u64);
            world.array(group + cfex_group::MEMBERS, members, 3, 1);
            world.u64(members, member as u64);
            world.u64(member, (IMAGE + cfex_member::VTABLE_RVA) as u64);
            world.u64(member + cfex_member::GROUP, group as u64);
            world.u64(
                member + cfex_member::SHIP_HANDLE,
                (u64::from(u32::MAX) << 32) | u64::from(cship::HANDLE_TYPE),
            );
            world.u32(member + cfex_member::BATTLE_SLOT, slot);
            world.u32(member + cfex_member::OWNER, u32::MAX);
            world.u32(member + cfex_member::STATE, 0x8000_0020);
        }
        // The zero ship reference is an actual convoy member.
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
        use crate::{modifier_definition as md, naval_combat_summary_inputs as summary};
        // Named synthetic module globals and metadata required by the
        // summary reader. Other absent addresses remain unreadable.
        if address == IMAGE + md::TABLE_RVA && output.len() == 8 {
            output.copy_from_slice(&0x100000u64.to_le_bytes());
            return true;
        }
        if (address == IMAGE + md::ELIGIBILITY_CALLBACK_RVA && output.len() == 8)
            || (address == IMAGE + md::TRADE_FEATURE_FLAGS_RVA && output.len() == 4)
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
    world.0[ATTACKING + 0xb8 - GS] = 1;
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
        (IMAGE + crate::cstrategic_region::VTABLE_RVA) as u64,
    );
    world.u32(REGION + crate::cstrategic_region::ID, 94);
    world.u64(REGION + 0x30, TEMPLATE as u64);
    world.u64(TEMPLATE, (IMAGE + 0x029c_5260) as u64);
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

/// A side's task forces (`CCombatant+0x20`) are read in stored order into the caller's
/// arena. A short arena leaves the row for the next hour with the demand reported, and an
/// entry that is not a task force refuses the table.
#[test]
fn side_units_keep_stored_order_and_report_their_demand() {
    const SLOTS: usize = 0xa700;
    const FORCES: [usize; 2] = [0xa800, 0xa900];
    let handles = [(900_u64 << 32) | 61, (244 << 32) | 61];
    let mut world = detail_world();
    world.array(ATTACKING + cnaval_combatant::UNITS, SLOTS, 4, 2);
    for (index, (force, handle)) in FORCES.into_iter().zip(handles).enumerate() {
        world.u64(SLOTS + index * 8, force as u64);
        world.u64(force, (IMAGE + crate::ctask_force::VTABLE_RVA) as u64);
        world.u64(
            force + crate::ctask_force::SECONDARY_BASE,
            (IMAGE + crate::ctask_force::SECONDARY_VTABLE_RVA) as u64,
        );
        world.u64(force + crate::ctask_force::SECONDARY_BASE + 8, handle);
    }
    let run = |world: &World, arena: &mut [u64]| {
        let mut details = [DetailsRow::default(); 1];
        let mut buffers = cfex_member::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes: &mut [],
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: &mut [],
            plane_naval_hits: &mut [],
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: &mut [],
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            commander_inputs: &mut [],
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: arena,
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: &mut [0; 64],
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: &mut [(0, 0); 64],
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: &mut [0; 512],
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: &mut [0; 1024],
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        let combats = walk_details_timed(
            &DetailMemory(world.memory()),
            IMAGE,
            GS,
            &mut details,
            &mut vec![cfex_member::DetailsRow::default(); 2],
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
    let mut arena = [0; 3];
    let (combats, units, len, required) = run(&world, &mut arena);
    assert_eq!(combats, Outcome::Rows { len: 1 });
    assert_eq!(units, [[0, 2], [2, 0]]);
    assert_eq!((len, required), (2, 2));
    assert_eq!(arena[..2], handles);
    let (combats, _, len, required) = run(&world, &mut [0; 1]);
    assert_eq!(
        combats,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!((len, required), (0, 2));
    world.u64(FORCES[1], (IMAGE + cship::VTABLE_RVA) as u64);
    assert!(matches!(
        run(&world, &mut [0; 3]).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn null_commander_never_falls_back_to_the_last_leader() {
    let mut world = detail_world();
    // +0x118 is the last leader the save and a load keep; the window shows
    // only +0x110, so the reader must not read, fall back to or fail on it.
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
            ([(0, 0); 64], 0, 0);
        let mut arenas = cnaval_combatant::SideArenas {
            countries: &mut countries,
            countries_len: &mut countries_len,
            units: &mut units,
            units_len: &mut units_len,
            units_required: &mut units_required,
            damage_sources: &mut damage_sources,
            damage_sources_len: &mut damage_sources_len,
            damage_sources_required: &mut damage_sources_required,
        };
        cnaval_combatant::details(memory, IMAGE, GS, BODY, [&side, &opponent], &mut arenas)
    };
    assert_eq!(details(&DetailMemory(world.memory())).unwrap().commander, 0);
    // Only a null pointer means no commander; a bad one still fails.
    world.u64(ATTACKING + cnaval_combatant::COMMANDER, u64::MAX);
    assert!(details(&DetailMemory(world.memory())).is_err());
}

#[test]
fn detail_rows_preserve_native_widths_and_report_bounded_copy_cost() {
    use std::cell::{Cell, RefCell};
    struct Count<'a> {
        inner: DetailMemory<'a>,
        calls: Cell<usize>,
        bytes: Cell<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Count<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.bytes.set(self.bytes.get() + output.len());
            self.reads.borrow_mut().push((address, output.len()));
            self.inner.copy(address, output)
        }
    }
    let world = detail_world();
    let memory = Count {
        inner: DetailMemory(world.memory()),
        calls: Cell::new(0),
        bytes: Cell::new(0),
        reads: RefCell::default(),
    };
    let mut details = [DetailsRow::default(); 1];
    let mut ships = vec![cfex_member::DetailsRow::default(); 2];
    let mut buffers = cfex_member::DetailBuffers {
        damage: &mut Vec::new(),
        air_hits: &mut Vec::new(),
        naval_hits: &mut Vec::new(),
        lengths: [0; 3],
        required: [0; 3],
        planes: &mut [],
        planes_len: 0,
        planes_required: 0,
        plane_air_hits: &mut [],
        plane_naval_hits: &mut [],
        plane_hit_lengths: [0; 2],
        plane_hit_required: [0; 2],
        plane_kills: &mut [],
        plane_kills_len: 0,
        plane_kills_required: 0,
        weapon_sources: &mut Vec::new(),
        weapon_terms: &mut Vec::new(),
        weapon_lengths: [0; 2],
        weapon_required: [0; 2],
        commander_inputs: &mut [],
        commander_inputs_len: 0,
        commander_inputs_required: 0,
        commander_timed_traits: &mut Vec::new(),
        commander_timed_traits_len: 0,
        side_units: &mut [],
        side_units_len: 0,
        side_units_required: 0,
        summary_countries: &mut Vec::new(),
        summary_countries_len: 0,
        custom_sea: &mut [0; 64],
        custom_sea_len: 0,
        custom_sea_required: 0,
        damage_sources: &mut [(0, 0); 64],
        damage_sources_len: 0,
        damage_sources_required: 0,
        wing_types: &mut [0; 512],
        wing_types_len: 0,
        wing_types_required: 0,
        plane_wings: &mut [0; 1024],
        plane_wings_len: 0,
        plane_wings_required: 0,
    };
    let mut tick = 0;
    let timed = walk_details_timed(
        &memory,
        IMAGE,
        GS,
        &mut details,
        &mut ships,
        &mut vec![0; 4],
        &mut buffers,
        |_| {
            tick += 1;
            tick
        },
    );
    assert!(timed.read_ticks[1] > 0);
    assert_eq!(timed.read_ticks.iter().sum::<u64>(), 3);
    let result = timed.capture;
    assert_eq!(result.combats, Outcome::Rows { len: 1 });
    assert_eq!(result.participants, Outcome::Rows { len: 2 });
    assert_eq!(details[0].sides[0].capital_screening, i64::MIN);
    assert_eq!(ships[0].build_cost_ic, i64::MAX);
    assert_eq!(ships[0].cached_strength_ratio, i64::MIN);
    assert_eq!(ships[0].received_criticals, -1);
    assert_eq!(ships[0].received_damage[0], i64::MIN);
    let reads = memory.reads.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|p| **p == (GS + cgame_state::COMBATS + 8, 16))
            .count(),
        1
    );
    for member in [MEMBER_A, MEMBER_D] {
        for offset in [cfex_member::DAMAGE_ENTRIES, 0x150, 0x168, 0x180] {
            assert_eq!(
                reads
                    .iter()
                    .filter(|p| **p == (member + offset, 16))
                    .count(),
                1,
                "nested header at {member:#x}+{offset:#x}"
            );
        }
    }
    assert!(memory.calls.get() < 512 && memory.bytes.get() < 8192);
    println!(
        "naval details synthetic one combat/two retired ship slots: {} copies/{} bytes; caller pool {} bytes",
        memory.calls.get(),
        memory.bytes.get(),
        std::mem::size_of_val(&details)
            + ships.capacity() * std::mem::size_of::<cfex_member::DetailsRow>()
            + std::mem::size_of::<[usize; 4]>()
    );
}

#[test]
fn detail_storage_grows_once_from_original_counts_with_equal_faults_and_reads() {
    use std::cell::RefCell;
    struct Trace<'a> {
        memory: DetailMemory<'a>,
        fault: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if self
                .fault
                .is_some_and(|fault| address <= fault && fault - address < output.len())
            {
                output.fill(0x7e);
                false
            } else {
                self.memory.copy(address, output)
            }
        }
    }
    let mut world = detail_world();
    world.array(MEMBER_A + cfex_member::DAMAGE_ENTRIES, 0xa800, 1, 1);
    world.u64(
        0xa800,
        (IMAGE + crate::cfex_damage_entry::VTABLE_RVA) as u64,
    );
    world.u64(0xa818, i64::MAX as u64);
    // One member occurs in both active and retained lists. Both air-hit
    // channels retain the same pointer as independent observations.
    world.array(GROUP_A + 0x70, MEMBERS_A, 1, 1);
    for offset in [0x150, 0x168] {
        world.array(MEMBER_A + offset, 0xa840, 1, 1);
    }
    world.u64(0xa840, 0xaa00);
    world.u64(
        0xaa00,
        (IMAGE + crate::naval_combat_air_hit::VTABLE_RVA) as u64,
    );
    world.u64(0xaa08, 0xab00);
    world.u64(
        0xab00,
        (IMAGE + crate::cequipment_variant::VTABLE_RVA) as u64,
    );
    world.u64(0xab08, u64::MAX);
    world.array(MEMBER_A + 0x180, 0xa848, 1, 1);
    world.u64(0xa848, 0xac00);
    world.u64(0xac00, (IMAGE + crate::naval_combat_hit::VTABLE_RVA) as u64);
    world.u64(0xac10 + crate::msvc_string::CAPACITY, 15);
    world.u64(0xac38, i64::MIN as u64);
    let run = |initial, fault| {
        let memory = Trace {
            memory: DetailMemory(world.memory()),
            fault,
            reads: RefCell::default(),
        };
        let mut rows = [DetailsRow::default(); 1];
        let mut ships = vec![cfex_member::DetailsRow::default(); initial];
        let mut groups = vec![0; initial];
        let mut buffers = cfex_member::DetailBuffers {
            damage: &mut vec![Default::default(); initial],
            air_hits: &mut vec![Default::default(); initial],
            naval_hits: &mut vec![Default::default(); initial],
            lengths: [0; 3],
            required: [0; 3],
            planes: &mut [],
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: &mut [],
            plane_naval_hits: &mut [],
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: &mut [],
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            commander_inputs: &mut [],
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: &mut [],
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: &mut [0; 64],
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: &mut [(0, 0); 64],
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: &mut [0; 512],
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: &mut [0; 1024],
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        let result = capture_details(
            &memory,
            IMAGE,
            GS,
            &mut rows,
            &mut ships,
            &mut groups,
            &mut buffers,
        );
        (
            result,
            rows,
            ships[..result.participants.len()].to_vec(),
            buffers.damage[..buffers.lengths[0]].to_vec(),
            buffers.air_hits[..buffers.lengths[1]].to_vec(),
            buffers.naval_hits[..buffers.lengths[2]].to_vec(),
            buffers.lengths,
            buffers.required,
            memory.reads.into_inner(),
        )
    };
    for fault in [
        None,
        Some(MEMBER_A + 0x30),
        Some(0xa818),
        Some(0xaa00),
        Some(0xac38),
        Some(GROUP_D),
    ] {
        let expected = run(8, fault);
        for initial in [0, 1] {
            assert_eq!(
                run(initial, fault),
                expected,
                "capacity={initial} fault={fault:?}"
            );
        }
        if fault.is_none() {
            assert_eq!(expected.0.participants, Outcome::Rows { len: 3 });
            assert_eq!(expected.6, [2, 4, 2]);
            assert_eq!(expected.7, [2, 4, 2]);
            assert_eq!(
                expected
                    .2
                    .iter()
                    .map(|row| row.member_source)
                    .collect::<Vec<_>>(),
                [0, 1, 0]
            );
            assert_eq!(expected.3[0].damage, i64::MAX);
            assert_eq!(expected.5[0].damage, i64::MIN);
        } else {
            assert!(matches!(
                expected.0.participants,
                Outcome::Unavailable { .. }
            ));
        }
    }
}

/// Format-23 live session: 302 hours failed with "naval plane identity or group
/// mismatch". External waves (14160DFE0/14160D930) keep one CFEXAir in both
/// side+E0 and the side's first group +58, with +8 naming that group.
#[test]
fn side_external_planes_keep_their_first_group_owner() {
    const PLANE: usize = 0xa900;
    let mut world = detail_world();
    world.u64(PLANE, (IMAGE + crate::cfex_air::VTABLE_RVA) as u64);
    world.u64(PLANE + 8, GROUP_A as u64);
    world.u64(PLANE + 0x88 + crate::msvc_string::CAPACITY, 15);
    world.u64(0xa880, PLANE as u64);
    world.array(GROUP_A + 0x58, 0xa880, 1, 1);
    world.array(ATTACKING + 0xe0, 0xa880, 1, 1);
    let mut planes = vec![crate::cfex_air::Row::default(); 4];
    let run = |world: &World, planes: &mut [crate::cfex_air::Row]| {
        let mut rows = [DetailsRow::default(); 1];
        let mut buffers = cfex_member::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes,
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: &mut [],
            plane_naval_hits: &mut [],
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: &mut [],
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            commander_inputs: &mut [],
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: &mut [],
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: &mut [0; 64],
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: &mut [(0, 0); 64],
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: &mut [0; 512],
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: &mut [0; 1024],
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        let result = capture_details(
            &DetailMemory(world.memory()),
            IMAGE,
            GS,
            &mut rows,
            &mut vec![cfex_member::DetailsRow::default(); 2],
            &mut vec![0; 4],
            &mut buffers,
        );
        (result.combats, rows[0].sides[0].planes, buffers.planes_len)
    };
    let (outcome, side_planes, len) = run(&world, &mut planes);
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!((side_planes, len), ([0, 2], 2));
    assert_eq!((planes[0].source, planes[1].source), (0, 2));
    // A null owner remains valid for side planes; another side's group does not.
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
            // A failed exact-field copy may have changed its destination.
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
    // The opposing vector can omit a group that its paired pointer retains.
    world.array(DEFENDING + cnaval_combatant::GROUPS, 0, 0, 0);
    let mut combats = [Row::default(); 2];
    let mut members = [cnaval_combatant::Row::default(); 4];
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut [0; 8],
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
fn table_capacity_and_group_scratch_have_explicit_independent_outcomes() {
    let world = World::new();
    let mut combats = [Row::default(); 1];
    let mut members = [cnaval_combatant::Row::default(); 1];
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut [0; 2],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(
        capture.participants,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut [],
        &mut members,
        &mut [0; 2],
    );
    assert_eq!(
        capture.combats,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut [0; 1],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(capture.required_group_capacity, 2);
    assert_eq!(
        capture.participants,
        Outcome::Unavailable {
            reason: "naval group scratch capacity exceeded"
        }
    );
}

#[test]
fn invalid_membership_does_not_erase_readable_combat_location() {
    let mut world = World::new();
    world.u64(MEMBER_A + cfex_member::GROUP, GROUP_D as u64);
    let mut combats = [Row::default(); 2];
    let mut members = [cnaval_combatant::Row::default(); 4];
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut [0; 4],
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
        &mut [0; 4],
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
        &mut [0; 4],
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
    let mut expected_members = [cnaval_combatant::Row::default(); 4];
    let expected = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut [Row::default(); 1],
        &mut expected_members,
        &mut [0; 4],
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
        let mut members = [cnaval_combatant::Row::default(); 4];
        let capture = walk(
            &fault,
            IMAGE,
            GS,
            &mut [Row::default(); 1],
            &mut members,
            &mut [0; 4],
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
            &mut [Row::default(); 1],
            &mut [cnaval_combatant::Row::default(); 4],
            &mut [0; 4],
        );
        assert!(matches!(capture.combats, Outcome::Unavailable { .. }));
        assert_eq!(capture.participants, capture.combats);
    }
    // A later payload copy fault must not mask a successfully copied but
    // incorrect identity in the same guarded scatter.
    world.u64(ATTACKING + cnaval_combatant::COMBAT, BODY as u64 + 8);
    let capture = walk(
        &Fault {
            memory: world.memory(),
            address: ATTACKING + cnaval_combatant::POSITIONING,
        },
        IMAGE,
        GS,
        &mut [Row::default(); 1],
        &mut [cnaval_combatant::Row::default(); 4],
        &mut [0; 4],
    );
    assert_eq!(
        capture.combats,
        Outcome::Unavailable {
            reason: "naval side identity or combat back-reference mismatch"
        }
    );
    assert_eq!(capture.participants, capture.combats);
}

/// The mixed registry is walked to its own count: at 50,000 divisions the land combats
/// alone pass any fixed bound, so twice 4,096 entries, one in 1,024 naval, blank no naval
/// table. Naval rows past the caller's capacity are counted unread, the demand the
/// collector grows from.
#[test]
fn a_registry_past_4096_combats_is_walked_whole_and_truncates_by_demand() {
    const REGISTRY: usize = 0x10000;
    const LAND: usize = 0xa800;
    let entries = 2 * 4096 + 1;
    let is_naval = |index: usize| index % 1024 == 1023;
    let naval = (0..entries).filter(|&index| is_naval(index)).count();
    let mut world = detail_world();
    world.0.resize(REGISTRY + entries * 8 - GS, 0);
    world.u64(LAND, (IMAGE + cland_combat::VTABLE_RVA) as u64);
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
        &mut [0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: naval });
    assert_eq!(capture.participants, Outcome::Rows { len: 2 * naval });
    assert_eq!(combats[naval - 1].registry_ordinal, 8191);
    let details = |capacity: usize| {
        let mut rows = vec![DetailsRow::default(); capacity];
        let mut buffers = cfex_member::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes: &mut [],
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: &mut [],
            plane_naval_hits: &mut [],
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: &mut [],
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            commander_inputs: &mut [],
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: &mut [],
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: &mut [0; 64],
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: &mut [(0, 0); 64],
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: &mut [0; 512],
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: &mut [0; 1024],
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
    assert_eq!(
        short.combats,
        Outcome::Truncated {
            len: 2,
            unread_rows: naval as u32 - 2
        }
    );
    assert_eq!(short.participants, Outcome::Rows { len: 2 * naval });
}

/// One details walk from empty ship and group storage, with a plane arena of `planes` rows.
#[allow(clippy::type_complexity)]
fn walk_whole(
    world: &World,
    planes: usize,
) -> (
    Capture,
    DetailsRow,
    Vec<cfex_member::DetailsRow>,
    usize,
    usize,
) {
    let mut rows = [DetailsRow::default(); 1];
    let mut ships = Vec::new();
    let mut plane_rows = vec![crate::cfex_air::Row::default(); planes];
    let mut buffers = cfex_member::DetailBuffers {
        damage: &mut Vec::new(),
        air_hits: &mut Vec::new(),
        naval_hits: &mut Vec::new(),
        lengths: [0; 3],
        required: [0; 3],
        planes: &mut plane_rows,
        planes_len: 0,
        planes_required: 0,
        plane_air_hits: &mut [],
        plane_naval_hits: &mut [],
        plane_hit_lengths: [0; 2],
        plane_hit_required: [0; 2],
        plane_kills: &mut [],
        plane_kills_len: 0,
        plane_kills_required: 0,
        weapon_sources: &mut Vec::new(),
        weapon_terms: &mut Vec::new(),
        weapon_lengths: [0; 2],
        weapon_required: [0; 2],
        commander_inputs: &mut [],
        commander_inputs_len: 0,
        commander_inputs_required: 0,
        commander_timed_traits: &mut Vec::new(),
        commander_timed_traits_len: 0,
        side_units: &mut [],
        side_units_len: 0,
        side_units_required: 0,
        summary_countries: &mut Vec::new(),
        summary_countries_len: 0,
        custom_sea: &mut [0; 64],
        custom_sea_len: 0,
        custom_sea_required: 0,
        damage_sources: &mut [(0, 0); 64],
        damage_sources_len: 0,
        damage_sources_required: 0,
        wing_types: &mut [0; 512],
        wing_types_len: 0,
        wing_types_required: 0,
        plane_wings: &mut [0; 1024],
        plane_wings_len: 0,
        plane_wings_required: 0,
    };
    let capture = capture_details(
        &DetailMemory(world.memory()),
        IMAGE,
        GS,
        &mut rows,
        &mut ships,
        &mut Vec::new(),
        &mut buffers,
    );
    let (len, required) = (buffers.planes_len, buffers.planes_required);
    ships.truncate(capture.participants.len());
    (capture, rows[0].clone(), ships, len, required)
}

/// CNavalCombatant::Serialize@14160FF40 and CFEXGroup::Serialize@141C58360 walk the forward
/// groups, member vectors and plane vectors to their own counts, so the walk has no bound of
/// its own on them: 8,193 groups a side, 65,537 members a vector and 16,385 planes a vector
/// are read whole. Ship rows grow from empty storage one read member at a time; a plane
/// arena too short leaves the combat unread with its exact demand, and the next hour's
/// arena, grown to that demand, reads it.
#[test]
fn groups_members_and_planes_past_the_old_bounds_are_read_whole() {
    const ARRAY: usize = 0x80000;
    const GROUPS: usize = 0x100000;
    let size = 0x600000 - GS;
    // 8,193 forward groups on the attacking side, GROUP_A last.
    let mut world = detail_world();
    world.0.resize(size, 0);
    world.array(ATTACKING + cnaval_combatant::GROUPS, ARRAY, 8193, 8193);
    for index in 0..8192 {
        let group = GROUPS + index * 0x100;
        world.u64(ARRAY + index * 8, group as u64);
        world.u64(group, (IMAGE + cfex_group::VTABLE_RVA) as u64);
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
    // A group listed twice in its side's vector is still refused.
    world.u64(ARRAY, GROUP_A as u64);
    assert_eq!(
        walk_whole(&world, 0).0.participants,
        Outcome::Unavailable {
            reason: "naval duplicate forward group"
        }
    );

    // 65,537 members in GROUP_A's forward vector.
    let mut world = detail_world();
    world.0.resize(size, 0);
    world.array(GROUP_A + cfex_group::MEMBERS, ARRAY, 65_537, 65_537);
    for index in 0..65_537 {
        world.u64(ARRAY + index * 8, MEMBER_A as u64);
    }
    let (capture, _, ships, _, _) = walk_whole(&world, 0);
    assert_eq!(capture.participants, Outcome::Rows { len: 65_538 });
    assert_eq!(ships[65_536].member_index, 65_536);

    // 16,385 planes in GROUP_A's +0x58 vector.
    const PLANE: usize = 0xa900;
    let mut world = detail_world();
    world.0.resize(size, 0);
    world.u64(PLANE, (IMAGE + crate::cfex_air::VTABLE_RVA) as u64);
    world.u64(PLANE + 8, GROUP_A as u64);
    world.u64(PLANE + 0x88 + crate::msvc_string::CAPACITY, 15);
    world.array(GROUP_A + 0x58, ARRAY, 16_385, 16_385);
    for index in 0..16_385 {
        world.u64(ARRAY + index * 8, PLANE as u64);
    }
    let (capture, _, _, len, required) = walk_whole(&world, 4);
    assert_eq!(
        capture.combats,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!((len, required), (0, 16_385));
    let (capture, row, _, len, _) = walk_whole(&world, required);
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!((row.sides[0].planes, len), ([0, 16_385], 16_385));
}

#[test]
fn active_wing_vectors_exceed_256() {
    const COUNT: usize = 257;
    const PLANE: usize = 0xa900;
    const ARRAY: usize = 0xc000;
    let mut world = World::new();
    world.0.resize(ARRAY + COUNT * 16 - GS, 0);
    world.u64(PLANE, (IMAGE + crate::cfex_air::VTABLE_RVA) as u64);
    world.u64(PLANE + 8, GROUP_A as u64);
    world.array(PLANE + 0x28, ARRAY, COUNT as u32, COUNT as u32);
    world.array(PLANE + 0x40, ARRAY + COUNT * 8, COUNT as u32, COUNT as u32);
    for index in 0..COUNT {
        world.u64(ARRAY + index * 8, index as u64 + 1);
    }
    let mut wing_types = [0; COUNT];
    let mut wing_types_len = 0;
    let mut wing_types_required = 0;
    let mut plane_wings = [0; COUNT * 2];
    let mut plane_wings_len = 0;
    let mut plane_wings_required = 0;
    let row = crate::cfex_air::read(
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
        world.u64(entry, (IMAGE + crate::cfex_damage_entry::VTABLE_RVA) as u64);
        world.u64(entry + 0x18, i64::MIN as u64);
    }
    let (capture, _, ships, _, _) = walk_whole(&world, 0);
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    assert_eq!(ships[0].damage_entries, [0, COUNT]);
}

#[test]
fn map_group_membership_and_scratch_demand_scale_to_5000_owners() {
    const ARRAY: usize = 0x80000;
    const GROUPS: usize = 0x100000;
    const OWNERS: usize = 5000;
    let mut world = World::new();
    world.0.resize(GROUPS + OWNERS * 0x100 - GS, 0);
    world.array(
        ATTACKING + cnaval_combatant::GROUPS,
        ARRAY,
        OWNERS as u32,
        OWNERS as u32,
    );
    // The map keeps the first occurrence of each paired group. The later
    // duplicate does not add scratch demand or repeat a member.
    for index in 0..OWNERS {
        let group = if index + 1 == OWNERS {
            GROUP_A
        } else {
            GROUPS + index * 0x100
        };
        world.u64(ARRAY + index * 8, group as u64);
        world.u64(group, (IMAGE + cfex_group::VTABLE_RVA) as u64);
        world.u64(group + cfex_group::SIDE, ATTACKING as u64);
    }
    let mut combats = [Row::default(); 1];
    let mut members = [cnaval_combatant::Row::default(); 2];
    let short = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut [],
    );
    assert_eq!(short.required_group_capacity, OWNERS + 1);
    assert_eq!(
        short.participants,
        Outcome::Unavailable {
            reason: "naval group scratch capacity exceeded"
        }
    );
    let full = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut vec![0; short.required_group_capacity],
    );
    assert_eq!(full.participants, Outcome::Rows { len: 2 });
    assert_eq!(full.required_group_capacity, short.required_group_capacity);
}

#[test]
fn empty_registry_is_valid_and_an_unreadable_registry_is_not_empty() {
    let mut world = World::new();
    world.array(GS + cgame_state::COMBATS + 8, 0, 0, 0);
    let capture = walk(&world.memory(), IMAGE, GS, &mut [], &mut [], &mut []);
    assert_eq!(capture.combats, Outcome::Rows { len: 0 });
    assert_eq!(capture.participants, Outcome::Rows { len: 0 });
    world.array(GS + cgame_state::COMBATS + 8, usize::MAX - 3, 1, 1);
    let capture = walk(&world.memory(), IMAGE, GS, &mut [], &mut [], &mut []);
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
    world.u64(land, (IMAGE + cland_combat::VTABLE_RVA) as u64);
    world.u64(border, (IMAGE + cland_border_war_combat::VTABLE_RVA) as u64);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut [Row::default(); 1],
        &mut [cnaval_combatant::Row::default(); 4],
        &mut [0; 4],
    );
    assert_eq!(capture.combats, Outcome::Rows { len: 1 });
    assert_eq!(capture.participants, Outcome::Rows { len: 2 });
    // The registry ordinal counts the land entries before the naval one.
    world.u64(ROOTS, land as u64);
    world.u64(ROOTS + 8, BODY as u64);
    let mut combats = [Row::default(); 1];
    walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut [cnaval_combatant::Row::default(); 4],
        &mut [0; 4],
    );
    assert_eq!(combats[0].registry_ordinal, 1);
    world.u64(ROOTS, BODY as u64);
    world.u64(ROOTS + 8, land as u64);
    for null in [false, true] {
        if null {
            world.u64(ROOTS + 16, 0);
        } else {
            world.u64(border, (IMAGE + VTABLE_RVA + 8) as u64);
        }
        let capture = walk(
            &world.memory(),
            IMAGE,
            GS,
            &mut [Row::default(); 1],
            &mut [cnaval_combatant::Row::default(); 4],
            &mut [0; 4],
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
    let mut combats = [Row::default(); 2];
    let mut members = [cnaval_combatant::Row::default(); 4];
    world.u64(0xa500 + 42 * 8, FRONT as u64 + 8);
    let capture = walk(
        &world.memory(),
        IMAGE,
        GS,
        &mut combats,
        &mut members,
        &mut [0; 4],
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
        &mut [0; 4],
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
fn table_timing_partitions_the_whole_walk_including_a_failed_member_read() {
    let mut world = World::new();
    world.u64(MEMBERS_A, 0);
    let mut ticks = 0;
    let timed = walk_timed(
        &world.memory(),
        IMAGE,
        GS,
        &mut [Row::default(); 1],
        &mut [cnaval_combatant::Row::default(); 4],
        &mut [0; 4],
        |_| {
            ticks += 1;
            ticks
        },
    );
    assert!(matches!(
        timed.capture.participants,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(timed.read_ticks, [2, 1]);
    assert_eq!(timed.read_ticks.iter().sum::<u64>(), ticks - 1);
}

#[test]
fn healthy_read_accounting_partitions_naval_combat_without_changing_its_read_trace() {
    use crate::memory::{CountedMemory, ReadCounts};
    use std::cell::RefCell;

    struct Traced<'a> {
        memory: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            self.memory.copy(address, destination)
        }
    }

    for corrupt_member in [false, true] {
        let mut world = World::new();
        if corrupt_member {
            world.u64(MEMBERS_A, 0);
        }
        let original = Traced {
            memory: world.memory(),
            reads: RefCell::default(),
        };
        let mut expected_combats = [Row::default(); 1];
        let mut expected_members = [cnaval_combatant::Row::default(); 4];
        let expected = walk(
            &original,
            IMAGE,
            GS,
            &mut expected_combats,
            &mut expected_members,
            &mut [0; 4],
        );
        let counted = CountedMemory::new(Traced {
            memory: world.memory(),
            reads: RefCell::default(),
        });
        let mut combats = [Row::default(); 1];
        let mut members = [cnaval_combatant::Row::default(); 4];
        let mut by_table = [ReadCounts::ZERO; 2];
        let mut active = 0;
        ReadCounts::take_current();
        let actual = walk_timed(
            &counted,
            IMAGE,
            GS,
            &mut combats,
            &mut members,
            &mut [0; 4],
            |following| {
                by_table[active] += ReadCounts::take_current();
                active = following;
                0
            },
        );
        by_table[active] += ReadCounts::take_current();
        assert_eq!(active, 0, "failed member reads restore the combat anchor");
        assert_eq!(actual.capture, expected);
        assert_eq!(combats, expected_combats);
        assert_eq!(members, expected_members);
        assert_eq!(*counted.0.reads.borrow(), *original.reads.borrow());
        assert_eq!(actual.capture.combats, Outcome::Rows { len: 1 });
        if !corrupt_member {
            assert_eq!(actual.capture.participants, Outcome::Rows { len: 2 });
        }
        assert!(
            by_table
                .iter()
                .all(|counts| counts.spans > 0 && counts.bytes > 0)
        );
        let mut total = ReadCounts::ZERO;
        for counts in by_table {
            total += counts;
        }
        let trace = counted.0.reads.borrow();
        assert_eq!(
            total.spans,
            trace.iter().filter(|(_, len)| *len != 0).count() as u64
        );
        assert_eq!(
            total.bytes,
            trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
        );
        eprintln!("naval combat corrupt_member={corrupt_member}: {by_table:?}");
    }
}
