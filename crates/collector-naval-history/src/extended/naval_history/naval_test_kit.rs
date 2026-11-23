use super::{Capacities, Captured, NavalCopyContext, NavalCopyReport, NavalJob};
use engine_native::{Memory, memory::SliceMemory, walk::Outcome};

pub fn stage_wings(memory: &impl Memory, job: &mut NavalJob) {
    let mut wings = wing_census(memory);
    job.install_wings(&mut wings);
}

pub fn wing_census(memory: &impl Memory) -> collector_air::extended::air::AirWingJob {
    let mut wings = collector_air::extended::air::AirWingJob::fixture();
    let context = collector_air::extended::air::AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
        limits: [0; 3],
        game_score: false,
    };
    assert!(collector_air::extended::air::copy_air_refresh(memory, context, &mut wings).raw_ok);
    wings
}

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
const LTAB: usize = 0x12000;
pub(super) const C0: usize = 0x20000;
pub const THEATRE: usize = 0x30000;
pub const GROUP: usize = 0x31000;
const MEMBERS: usize = 0x32000;
const MEMBER1: usize = 0x33000;
const MEMBER2: usize = 0x34000;
const NAMES: usize = 0x35000;
const OOB_TABLE: usize = 0x60000;
const OOB_BUCKETS: usize = 0x61000;
pub const OOB_GROUP: usize = 0x62000;
const OOB_MEMBERS: usize = 0x63000;
const OOB_MEMBER: usize = 0x650b8;
const OOB_UNIT: usize = 0x65000;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}

pub fn fixture() -> Vec<u8> {
    use engine_native::{
        carmy, ccountry, corder_instance, corders_group, corders_group_member,
        creference_registry as registry, ctheater_group, ctheatre, cunit,
        reference_bucket as bucket,
    };
    let mut bytes = vec![0; 0x3500000];
    header(
        &mut bytes,
        GAME_STATE + engine_native::cgame_state::COUNTRIES,
        CTAB,
        3,
    );
    p64(&mut bytes, CTAB, 0);
    p64(&mut bytes, CTAB + 8, C0 as u64);
    p64(&mut bytes, CTAB + 16, C0 as u64);
    header(
        &mut bytes,
        GAME_STATE + engine_native::cgame_state::COUNTRY_LINKS,
        LTAB,
        3,
    );
    p32(&mut bytes, LTAB, 0);
    p32(&mut bytes, LTAB + 4, 1);
    p32(&mut bytes, LTAB + 8, 1);
    p64(&mut bytes, C0, ccountry::VTABLE_RVA.0 as u64);
    p32(&mut bytes, C0 + ccountry::ID, 1);
    p64(
        &mut bytes,
        GAME_STATE + engine_native::cgame_state::STRATEGIC_AIR_MANAGER,
        0x40000,
    );
    p64(
        &mut bytes,
        0x40000,
        engine_native_air::cstrategic_air_manager::VTABLE_RVA.0 as u64,
    );
    header(
        &mut bytes,
        0x40000 + engine_native_air::cstrategic_air_manager::COUNTRY_AIR,
        0x41000,
        3,
    );
    header(&mut bytes, C0 + ccountry::THEATRES, 0x36000, 1);
    p64(&mut bytes, 0x36000, THEATRE as u64);
    header(&mut bytes, C0 + ccountry::AIR_THEATRES, 0x36100, 0);
    p64(&mut bytes, THEATRE, ctheatre::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        THEATRE + ctheatre::HANDLE,
        (1u64 << 32) | ctheatre::REFERENCE_TYPE as u64,
    );
    p32(&mut bytes, THEATRE + ctheatre::OWNER, 1);
    p32(&mut bytes, THEATRE + ctheatre::VOLUNTEERS_THEATRE, 0);
    header(
        &mut bytes,
        THEATRE + ctheatre::THEATER_GROUP_DATA,
        0x36200,
        1,
    );
    p64(&mut bytes, 0x36200, GROUP as u64);
    p64(&mut bytes, GROUP, ctheater_group::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        GROUP + ctheater_group::HANDLE,
        (1u64 << 32) | ctheater_group::REFERENCE_TYPE as u64,
    );
    p64(&mut bytes, GROUP + ctheater_group::THEATRE, THEATRE as u64);
    bytes[GROUP + ctheater_group::NAME..GROUP + ctheater_group::NAME + 2].copy_from_slice(b"AB");
    p64(&mut bytes, GROUP + ctheater_group::NAME + 16, 2);
    p64(&mut bytes, GROUP + ctheater_group::NAME + 24, 15);
    bytes[GROUP + ctheater_group::NAME + 2] = 0;
    p32(&mut bytes, GROUP + ctheater_group::PRIORITY, 5);
    header(
        &mut bytes,
        GROUP + ctheater_group::ORDERS_GROUP_DATA,
        MEMBERS,
        2,
    );
    p64(&mut bytes, MEMBERS, MEMBER1 as u64);
    p64(&mut bytes, MEMBERS + 8, MEMBER2 as u64);
    for (member, id) in [(MEMBER1, 1u64), (MEMBER2, 2u64)] {
        p64(&mut bytes, member, corders_group::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            member + corders_group::HANDLE,
            (id << 32) | corders_group::REFERENCE_TYPE as u64,
        );
        p64(
            &mut bytes,
            member + corders_group::THEATER_GROUP,
            GROUP as u64,
        );
        p64(&mut bytes, member + corders_group::THEATRE, THEATRE as u64);
    }
    const HANDLE: u64 = (u32::MAX as u64) << 32 | corders_group::REFERENCE_TYPE as u64;
    p64(
        &mut bytes,
        BASE + registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8,
        OOB_TABLE as u64,
    );
    bytes[OOB_TABLE..OOB_TABLE + 32].fill(0);
    p64(
        &mut bytes,
        OOB_TABLE + registry::ENTRIES,
        OOB_BUCKETS as u64,
    );
    p32(&mut bytes, OOB_TABLE + registry::COUNT, 1);
    p32(&mut bytes, OOB_TABLE + registry::MASK, 7);
    bytes[OOB_TABLE + registry::OVERFLOW_DISTANCE] = 5;
    bytes[OOB_BUCKETS..OOB_BUCKETS + 14 * bucket::STRIDE].fill(0);
    bytes[OOB_BUCKETS + bucket::DISTANCE] = 1;
    p64(&mut bytes, OOB_BUCKETS + bucket::TYPE, HANDLE);
    p64(&mut bytes, OOB_BUCKETS + bucket::OBJECT, OOB_GROUP as u64);
    bytes[OOB_BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
    bytes[OOB_GROUP..OOB_GROUP + 0x240].fill(0);
    p64(
        &mut bytes,
        OOB_GROUP,
        (BASE + corders_group::VTABLE_RVA.0) as u64,
    );
    p64(&mut bytes, OOB_GROUP + corders_group::HANDLE, HANDLE);
    p32(&mut bytes, OOB_GROUP + corders_group::COUNTRY, 1);
    bytes[OOB_GROUP + corders_group::IS_ARMY_GROUP] = 0;
    bytes[OOB_GROUP + corders_group::MOTORIZATION_LEVEL] = 0;
    p64(&mut bytes, OOB_GROUP + corders_group::THEATRE, 0);
    header(
        &mut bytes,
        OOB_GROUP + corders_group::MEMBER_DATA,
        OOB_MEMBERS,
        1,
    );
    p64(&mut bytes, OOB_MEMBERS, OOB_MEMBER as u64);
    p64(&mut bytes, OOB_UNIT, carmy::VTABLE_RVA.0 as u64);
    p64(
        &mut bytes,
        OOB_UNIT + carmy::SECONDARY_BASE,
        carmy::SECONDARY_VTABLE_RVA.0 as u64,
    );
    p64(&mut bytes, OOB_UNIT + cunit::HANDLE, 0x4500_0001u64);
    let member_host = OOB_MEMBER;
    let unit_base = member_host - corders_group_member::HOST_OFFSET;
    assert_eq!(unit_base, OOB_UNIT);
    p64(
        &mut bytes,
        member_host + corders_group_member::GROUP,
        OOB_GROUP as u64,
    );
    header(
        &mut bytes,
        OOB_GROUP + engine_native::carmy_group::MEMBER_DATA,
        0x66000,
        0,
    );
    let _ = (NAMES, corder_instance::VTABLE_RVA);
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

#[derive(Debug, PartialEq)]
pub struct Split {
    pub land_outcome: Outcome,
    pub land: Vec<engine_native::ctheatre::Row>,
    pub groups_outcome: Outcome,
    pub groups: Vec<engine_native::ctheater_group::Row>,
    pub members_outcome: Outcome,
    pub members: Vec<engine_native::ctheater_group::MemberRow>,
    pub oob_groups_outcome: Outcome,
    pub oob_groups: Vec<engine_native::corders_group::GroupRow>,
    pub oob_members_outcome: Outcome,
    pub oob_members: Vec<engine_native::corders_group::MemberRow>,
    pub oob_children_outcome: Outcome,
    pub oob_children: Vec<engine_native::corders_group::ChildRow>,
}

fn collect(memory: &impl Memory, capacity: &Capacities) -> Split {
    let mut captured = Captured::with_capacities(capacity);
    let first = engine_native::ctheatre::walk(
        memory,
        BASE,
        GAME_STATE,
        true,
        engine_native::ctheatre::Buffers {
            land: &mut captured.land.rows,
            land_groups: &mut captured.land_groups.rows,
            land_members: &mut captured.land_members.rows,
            air: &mut captured.air.rows,
            air_groups: &mut captured.air_groups.rows,
            air_members: &mut captured.air_members.rows,
            names: &mut captured.theatre_names.first,
        },
    );
    captured.land.outcome = first.land;
    captured.land_groups.outcome = first.land_groups;
    captured.land_members.outcome = first.land_members;
    let mut roots = vec![engine_native::corders_group::Root::default(); 8];
    let roots_outcome = engine_native::corders_group::walk_roots(memory, BASE, &mut roots);
    let roots_len = roots_outcome.len();
    if let Outcome::Rows { .. } = roots_outcome {
        let walk = engine_native::corders_group::walk_order_of_battle(
            memory,
            BASE,
            &roots[..roots_len],
            &mut engine_native::corders_group::OrderOfBattle {
                groups: &mut captured.groups.rows,
                members: &mut captured.group_members.rows,
                children: &mut captured.group_children.rows,
                roster_scratch: &mut vec![Default::default(); roots_len],
            },
        );
        captured.groups.outcome = walk.groups;
        captured.group_members.outcome = walk.members;
        captured.group_children.outcome = walk.children;
    }
    Split {
        land_outcome: captured.land.outcome,
        land: captured.land.rows.clone(),
        groups_outcome: captured.land_groups.outcome,
        groups: captured.land_groups.rows.clone(),
        members_outcome: captured.land_members.outcome,
        members: captured.land_members.rows.clone(),
        oob_groups_outcome: captured.groups.outcome,
        oob_groups: captured.groups.rows.clone(),
        oob_members_outcome: captured.group_members.outcome,
        oob_members: captured.group_members.rows.clone(),
        oob_children_outcome: captured.group_children.outcome,
        oob_children: captured.group_children.rows.clone(),
    }
}

pub fn direct(memory: &impl Memory) -> Split {
    collect(memory, &Capacities::fixture())
}

pub fn decoded(captured: &Captured) -> Split {
    Split {
        land_outcome: captured.land.outcome,
        land: captured.land.rows.clone(),
        groups_outcome: captured.land_groups.outcome,
        groups: captured.land_groups.rows.clone(),
        members_outcome: captured.land_members.outcome,
        members: captured.land_members.rows.clone(),
        oob_groups_outcome: captured.groups.outcome,
        oob_groups: captured.groups.rows.clone(),
        oob_members_outcome: captured.group_members.outcome,
        oob_members: captured.group_members.rows.clone(),
        oob_children_outcome: captured.group_children.outcome,
        oob_children: captured.group_children.rows.clone(),
    }
}

fn assert_census_roots_decode_the_same(
    memory: &impl Memory,
    capacity: &Capacities,
    expected: &Split,
    registry: &NavalJob,
) {
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut roots = vec![engine_native::corders_group::Root::default(); 64];
    let outcome = engine_native::corders_group::walk_roots(memory, BASE, &mut roots);
    let mut job = NavalJob::fixture();
    stage_wings(memory, &mut job);
    job.capture_roots(&roots[..outcome.len().min(roots.len())], outcome);
    assert!(super::copy_naval_refresh(memory, ctx, &mut job).raw_ok);
    let mut captured = Captured::with_capacities(capacity);
    super::decode_naval_payload(&mut captured, BASE, &job, &mut [0u64; 9]);
    assert_eq!(&decoded(&captured), expected, "census {outcome:?}");
    let spans = |job: &NavalJob| job.log.spans.len();
    match outcome {
        Outcome::Rows { len } => {
            assert_eq!(job.roots_len, Some(len));
            assert!(spans(&job) < spans(registry), "census {outcome:?}");
        }
        _ => {
            assert_eq!(job.roots_len, None);
            assert!(spans(&job) < spans(registry), "census {outcome:?}");
        }
    }
}

pub fn split(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
    let split = split_registry(memory);
    assert_census_roots_decode_the_same(memory, &Capacities::fixture(), &split.0, &split.2);
    split
}

pub fn split_registry(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut first = NavalJob::fixture();
    stage_wings(memory, &mut first);
    let report = super::copy_naval_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    let mut ticks = [0u64; 9];
    super::decode_naval_payload(&mut captured, BASE, &first, &mut ticks);
    (
        Split {
            land_outcome: captured.land.outcome,
            land: captured.land.rows.clone(),
            groups_outcome: captured.land_groups.outcome,
            groups: captured.land_groups.rows.clone(),
            members_outcome: captured.land_members.outcome,
            members: captured.land_members.rows.clone(),
            oob_groups_outcome: captured.groups.outcome,
            oob_groups: captured.groups.rows.clone(),
            oob_members_outcome: captured.group_members.outcome,
            oob_members: captured.group_members.rows.clone(),
            oob_children_outcome: captured.group_children.outcome,
            oob_children: captured.group_children.rows.clone(),
        },
        report,
        first,
    )
}

pub fn direct_with(memory: &impl Memory, capacity: &Capacities) -> Split {
    collect(memory, capacity)
}

pub fn split_with(
    memory: &impl Memory,
    capacity: &Capacities,
) -> (Split, NavalCopyReport, NavalJob) {
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut first = NavalJob::fixture();
    stage_wings(memory, &mut first);
    let report = super::copy_naval_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut captured = Captured::with_capacities(capacity);
    let mut ticks = [0u64; 9];
    super::decode_naval_payload(&mut captured, BASE, &first, &mut ticks);
    assert_census_roots_decode_the_same(memory, capacity, &decoded(&captured), &first);
    (
        Split {
            land_outcome: captured.land.outcome,
            land: captured.land.rows.clone(),
            groups_outcome: captured.land_groups.outcome,
            groups: captured.land_groups.rows.clone(),
            members_outcome: captured.land_members.outcome,
            members: captured.land_members.rows.clone(),
            oob_groups_outcome: captured.groups.outcome,
            oob_groups: captured.groups.rows.clone(),
            oob_members_outcome: captured.group_members.outcome,
            oob_members: captured.group_members.rows.clone(),
            oob_children_outcome: captured.group_children.outcome,
            oob_children: captured.group_children.rows.clone(),
        },
        report,
        first,
    )
}

pub fn direct_truncated(memory: &impl Memory) -> Split {
    let capacity = Capacities {
        rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
        theatre_names: 8,
        sunk_names: 8,
        orders_sources: 1,
        wing_sources: 1,
        roster_scratch: 8,
        rail_levels: 0,
    };
    collect(memory, &capacity)
}

pub fn split_truncated(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut first = NavalJob::fixture();
    stage_wings(memory, &mut first);
    let report = super::copy_naval_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let capacity = Capacities {
        rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
        theatre_names: 8,
        sunk_names: 8,
        orders_sources: 1,
        wing_sources: 1,
        roster_scratch: 8,
        rail_levels: 0,
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut ticks = [0u64; 9];
    super::decode_naval_payload(&mut captured, BASE, &first, &mut ticks);
    assert_census_roots_decode_the_same(memory, &capacity, &decoded(&captured), &first);
    (
        Split {
            land_outcome: captured.land.outcome,
            land: captured.land.rows.clone(),
            groups_outcome: captured.land_groups.outcome,
            groups: captured.land_groups.rows.clone(),
            members_outcome: captured.land_members.outcome,
            members: captured.land_members.rows.clone(),
            oob_groups_outcome: captured.groups.outcome,
            oob_groups: captured.groups.rows.clone(),
            oob_members_outcome: captured.group_members.outcome,
            oob_members: captured.group_members.rows.clone(),
            oob_children_outcome: captured.group_children.outcome,
            oob_children: captured.group_children.rows.clone(),
        },
        report,
        first,
    )
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.land_outcome, direct.land_outcome, "land outcome");
    assert_eq!(split.land, direct.land, "land rows");
    assert_eq!(
        split.groups_outcome, direct.groups_outcome,
        "groups outcome"
    );
    assert_eq!(split.groups, direct.groups, "groups rows");
    assert_eq!(
        split.members_outcome, direct.members_outcome,
        "members outcome"
    );
    assert_eq!(split.members, direct.members, "members rows");
    assert_eq!(
        split.oob_groups_outcome, direct.oob_groups_outcome,
        "oob groups outcome"
    );
    assert_eq!(split.oob_groups, direct.oob_groups, "oob groups rows");
    assert_eq!(
        split.oob_members_outcome, direct.oob_members_outcome,
        "oob members outcome"
    );
    assert_eq!(split.oob_members, direct.oob_members, "oob members rows");
    assert_eq!(
        split.oob_children_outcome, direct.oob_children_outcome,
        "oob children outcome"
    );
    assert_eq!(split.oob_children, direct.oob_children, "oob children rows");
}

pub fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
    assert_eq!(captured.land.outcome, direct.land_outcome, "land outcome");
    assert_eq!(
        &captured.land.rows[..captured.land.outcome.len()],
        &direct.land[..direct.land_outcome.len()],
        "land rows"
    );
    assert_eq!(
        captured.land_groups.outcome, direct.groups_outcome,
        "groups outcome"
    );
    assert_eq!(
        &captured.land_groups.rows[..captured.land_groups.outcome.len()],
        &direct.groups[..direct.groups_outcome.len()],
        "groups rows"
    );
    assert_eq!(
        captured.land_members.outcome, direct.members_outcome,
        "members outcome"
    );
    assert_eq!(
        &captured.land_members.rows[..captured.land_members.outcome.len()],
        &direct.members[..direct.members_outcome.len()],
        "members rows"
    );
    assert_eq!(
        captured.groups.outcome, direct.oob_groups_outcome,
        "oob groups outcome"
    );
    assert_eq!(
        &captured.groups.rows[..captured.groups.outcome.len()],
        &direct.oob_groups[..direct.oob_groups_outcome.len()],
        "oob groups rows"
    );
    assert_eq!(
        captured.group_members.outcome, direct.oob_members_outcome,
        "oob members outcome"
    );
    assert_eq!(
        &captured.group_members.rows[..captured.group_members.outcome.len()],
        &direct.oob_members[..direct.oob_members_outcome.len()],
        "oob members rows"
    );
    assert_eq!(
        captured.group_children.outcome, direct.oob_children_outcome,
        "oob children outcome"
    );
    assert_eq!(
        &captured.group_children.rows[..captured.group_children.outcome.len()],
        &direct.oob_children[..direct.oob_children_outcome.len()],
        "oob children rows"
    );
}

#[test]
fn naval_oob_copy_reads_only_typed_child_and_provider_dependencies() {
    use engine_native::{carmy_group, corders_group, corders_group_member};
    use std::cell::RefCell;

    struct Fault<'a> {
        memory: SliceMemory<'a>,
        address: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if self
                .address
                .is_some_and(|fault| address <= fault && fault < end)
            {
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    const CHILDREN: usize = 0x66000;
    const CHILD: usize = 0x67000;
    let child_header = OOB_GROUP + carmy_group::MEMBER_DATA;
    let flag_address = OOB_GROUP + corders_group::IS_ARMY_GROUP;
    let backlink = OOB_MEMBER + corders_group_member::GROUP;
    assert_eq!(carmy_group::MEMBER_DATA, corders_group::SIZE);
    let mut bytes = fixture();
    for (name, flag, fault, poison, provider, required_fault) in [
        (
            "ordinary header outside allocation",
            0,
            Some(child_header),
            false,
            false,
            false,
        ),
        (
            "ordinary poisoned child header",
            0,
            None,
            true,
            false,
            false,
        ),
        ("army child", 1, None, false, false, false),
        (
            "army child header fault",
            1,
            Some(child_header),
            false,
            false,
            true,
        ),
        (
            "army child entry fault",
            1,
            Some(CHILDREN),
            false,
            false,
            true,
        ),
        (
            "army child identity fault",
            1,
            Some(CHILD),
            false,
            false,
            true,
        ),
        ("nonzero flag two", 2, None, false, false, false),
        (
            "nonzero flag two header fault",
            2,
            Some(child_header),
            false,
            false,
            true,
        ),
        ("nonzero flag maximum", u8::MAX, None, false, false, false),
        (
            "nonzero flag maximum header fault",
            u8::MAX,
            Some(child_header),
            false,
            false,
            true,
        ),
        (
            "unreadable discriminator",
            1,
            Some(flag_address),
            false,
            false,
            true,
        ),
        (
            "provider unused embedded vtable fault",
            0,
            Some(OOB_MEMBER),
            false,
            true,
            false,
        ),
        (
            "provider host identity fault",
            0,
            Some(OOB_UNIT),
            false,
            true,
            true,
        ),
        (
            "provider backlink fault",
            0,
            Some(backlink),
            false,
            true,
            true,
        ),
    ] {
        let vtable = if flag == 0 {
            corders_group::VTABLE_RVA
        } else {
            carmy_group::VTABLE_RVA
        };
        p64(&mut bytes, OOB_GROUP, (BASE + vtable.0) as u64);
        bytes[flag_address] = flag;
        header(
            &mut bytes,
            child_header,
            if poison { 0x7fff0000 } else { CHILDREN },
            1,
        );
        p64(&mut bytes, CHILDREN, CHILD as u64);
        p64(
            &mut bytes,
            CHILD,
            (BASE + corders_group::VTABLE_RVA.0) as u64,
        );
        p64(
            &mut bytes,
            CHILD + corders_group::HANDLE,
            (7u64 << 32) | corders_group::REFERENCE_TYPE as u64,
        );
        p64(
            &mut bytes,
            OOB_GROUP + corders_group::HQ_PROVIDER,
            if provider { OOB_MEMBER as u64 } else { 0 },
        );
        let memory = Fault {
            memory: memory(&bytes),
            address: fault,
            reads: RefCell::new(Vec::new()),
        };
        let direct = direct(&memory);
        memory.reads.borrow_mut().clear();
        let (replayed, report, _) = split_registry(&memory);
        assert_matches_direct(&replayed, &direct);
        assert!(report.raw_ok, "{name}");
        if required_fault {
            assert!(matches!(
                replayed.oob_groups_outcome,
                Outcome::Unavailable { .. }
            ));
        }
        let reads = memory.reads.borrow();
        let visits_children = flag != 0 && fault != Some(flag_address);
        assert_eq!(
            reads.contains(&(child_header, 16)),
            visits_children,
            "{name}"
        );
        if !visits_children {
            assert!(!reads.contains(&(CHILDREN, 8)), "{name}");
            assert!(!reads.contains(&(CHILD, 16)), "{name}");
            assert!(!reads.contains(&(0x7fff0000, 8)), "{name}");
            assert_eq!(
                reads
                    .iter()
                    .filter(
                        |&&(address, len)| address == OOB_GROUP + corders_group::THEATER_GROUP
                            && len == 8
                    )
                    .count(),
                2,
                "{name}"
            );
        }
        assert!(!reads.contains(&(OOB_MEMBER, 16)), "{name}");
        if provider {
            let field = reads
                .iter()
                .position(|&read| read == (OOB_GROUP + corders_group::HQ_PROVIDER, 8));
            let host = reads
                .windows(2)
                .position(|window| window == [(OOB_UNIT, 32), (backlink, 8)]);
            assert!(field.is_some() && host.is_some() && field < host, "{name}");
        }
        if !required_fault {
            assert_eq!(
                replayed.oob_groups_outcome,
                Outcome::Rows { len: 1 },
                "{name}"
            );
            assert_eq!(
                replayed.oob_members_outcome,
                Outcome::Rows { len: 1 },
                "{name}"
            );
            assert_eq!(
                replayed.oob_children_outcome,
                Outcome::Rows {
                    len: usize::from(flag != 0)
                },
                "{name}"
            );
        }
    }
}
thread_local! {
    pub static OOB_PER_FIELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[test]
fn oob_root_fields_in_one_copy_log_the_per_field_spans() {
    use engine_native::{corders_group, corders_group_member};
    use std::cell::RefCell;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        address: Option<usize>,
        copies: RefCell<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            *self.copies.borrow_mut() += 1;
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if self
                .address
                .is_some_and(|fault| address <= fault && fault < end)
            {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    const LEADER: usize = 0x68000;
    let mut bytes = fixture();
    p64(
        &mut bytes,
        OOB_GROUP,
        (BASE + engine_native::carmy_group::VTABLE_RVA.0) as u64,
    );
    bytes[OOB_GROUP + corders_group::IS_ARMY_GROUP] = 1;
    header(
        &mut bytes,
        OOB_GROUP + engine_native::carmy_group::MEMBER_DATA,
        0x66000,
        0,
    );
    bytes[OOB_GROUP + corders_group::MOTORIZATION_LEVEL] = 3;
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::THEATRE,
        THEATRE as u64,
    );
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::THEATER_GROUP,
        GROUP as u64,
    );
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::PARENT,
        OOB_GROUP as u64,
    );
    p64(&mut bytes, OOB_GROUP + corders_group::LEADER, LEADER as u64);
    p64(
        &mut bytes,
        OOB_GROUP + corders_group::HQ_PROVIDER,
        OOB_MEMBER as u64,
    );
    p64(
        &mut bytes,
        LEADER,
        (BASE + engine_native::carmy_leader::VTABLE_RVA.0) as u64,
    );
    p64(&mut bytes, LEADER + 8, (9u64 << 32) | 4713);
    let spans = |job: &NavalJob| {
        let mut spans: Vec<_> = job
            .log
            .spans
            .iter()
            .map(|span| {
                let bytes = if span.ok {
                    job.log.bytes[span.start as usize..][..span.len as usize].to_vec()
                } else {
                    Vec::new()
                };
                (span.addr, span.len, span.ok, span.mask, bytes)
            })
            .collect();
        spans.sort();
        spans
    };
    let backlink = OOB_MEMBER + corders_group_member::GROUP;
    let objects = [
        (OOB_GROUP, corders_group::SIZE),
        (THEATRE, 0x20),
        (GROUP, 0x20),
        (LEADER, 0x20),
        (OOB_UNIT, 0x20),
        (backlink, 8),
    ];
    let faults = std::iter::once(None).chain(
        objects
            .iter()
            .flat_map(|&(object, size)| (object..object + size).map(Some)),
    );
    let mut copies = [0; 2];
    for fault in faults {
        let memory = Fault {
            memory: memory(&bytes),
            address: fault,
            copies: RefCell::new(0),
        };
        let direct = direct(&memory);
        let (gathered, report, _) = split(&memory);
        assert!(report.raw_ok, "fault {fault:x?}");
        let mut roots = vec![corders_group::Root::default(); 8];
        if matches!(
            corders_group::walk_roots(&memory, BASE, &mut roots),
            Outcome::Rows { .. }
        ) {
            assert_matches_direct(&gathered, &direct);
        }
        OOB_PER_FIELD.set(true);
        *memory.copies.borrow_mut() = 0;
        let (per_field, _, reference) = split_registry(&memory);
        copies[1] += *memory.copies.borrow();
        OOB_PER_FIELD.set(false);
        *memory.copies.borrow_mut() = 0;
        let (_, _, registry) = split_registry(&memory);
        copies[0] += *memory.copies.borrow();
        assert_eq!(gathered, per_field, "fault {fault:x?}");
        assert_eq!(spans(&registry), spans(&reference), "fault {fault:x?}");
        if fault.is_none() {
            assert_eq!(gathered.oob_groups_outcome, Outcome::Rows { len: 1 });
        }
    }
    assert_eq!(copies[0], copies[1]);
}
