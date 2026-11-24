use super::*;
use engine_native::cgame_state;
use crate::{
    coperative_leader,
    operative_action_implementation::{
        COUNTRY_GETTER_RVA,
        COUNTRY_GETTER_SLOT,
        VTABLE_RVAS,
    },
};
use engine_native_memory::memory::SliceMemory;
use engine_native_parts::modifier_definition;

const STATE: usize = 0x100;
const OPERATIVE: usize = 0x7000;
const STRIDE: usize = 0x1100;
const IMPLEMENTATION: usize = 0x8500;
const TARGET_STATE: usize = 0x8600;
const PENDING_STATE: usize = 0x8700;
const ACTIVE_GROUP: usize = 0x8c00;
const PENDING_GROUP: usize = 0x8d00;
const NATIONALITY_LIST: usize = 0x8e00;
const OPERATIVE_ENTRIES: usize = 0x8e80;
const COUNTRY_ENTRIES: usize = 0x3700;
const DEFINITIONS: usize = 0x8f00;
const KEY: u32 = 1;
const HANDLE: u64 = 0xffff_ffff_8000_1269;

struct World(Vec<u8>);

impl Memory for World {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.len() == 8
            && VTABLE_RVAS
                .iter()
                .any(|&rva| address == rva.0 + COUNTRY_GETTER_SLOT)
        {
            destination.copy_from_slice(&(COUNTRY_GETTER_RVA.0 as u64).to_le_bytes());
            return true;
        }
        if destination.len() == 8 && address == modifier_definition::TABLE_RVA.0 {
            destination.copy_from_slice(&(DEFINITIONS as u64).to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(address, destination)
    }
}

fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
    bytes[offset..offset + N].copy_from_slice(&value);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    for (offset, data) in [
        (cgame_state::COUNTRIES, 0x2000_u64),
        (cgame_state::COUNTRY_LINKS, 0x2100),
    ] {
        put(&mut bytes, STATE + offset, data.to_le_bytes());
        put(&mut bytes, STATE + offset + 8, 2_u32.to_le_bytes());
        put(&mut bytes, STATE + offset + 12, 2_u32.to_le_bytes());
    }
    put(&mut bytes, 0x2008, 0x3000_u64.to_le_bytes());
    put(&mut bytes, 0x2104, 1_u32.to_le_bytes());
    put(
        &mut bytes,
        0x3000,
        (ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, 0x3008, 1_u32.to_le_bytes());
    put(
        &mut bytes,
        0x3000 + ccountry::INTELLIGENCE_AGENCY,
        0x5000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x5000,
        (cintelligence_agency::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x5000 + cintelligence_agency::OWNER_COUNTRY,
        0x3000_u64.to_le_bytes(),
    );
    let header = 0x5000 + cintelligence_agency::OPERATIVES;
    put(&mut bytes, header, 0x6000_u64.to_le_bytes());
    put(&mut bytes, header + 8, 1_u32.to_le_bytes());
    put(&mut bytes, header + 12, 1_u32.to_le_bytes());
    put(&mut bytes, 0x6000, (OPERATIVE as u64).to_le_bytes());
    put(&mut bytes, OPERATIVE, (VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut bytes, OPERATIVE + IDENTITY, HANDLE.to_le_bytes());
    put(&mut bytes, OPERATIVE + OWNER_COUNTRY, 7_u32.to_le_bytes());
    put(&mut bytes, OPERATIVE + ASSIGNMENT, 2_u32.to_le_bytes());
    let portrait = OPERATIVE + PORTRAIT_GFX;
    bytes[portrait..portrait + 7].copy_from_slice(b"GFX_spy");
    put(&mut bytes, portrait + 16, 7_u64.to_le_bytes());
    put(&mut bytes, portrait + 24, 15_u64.to_le_bytes());
    put(
        &mut bytes,
        OPERATIVE + coperative_leader::NAME + 24,
        u64::MAX.to_le_bytes(),
    );
    for (state, id) in [(TARGET_STATE, 42_i32), (PENDING_STATE, -3)] {
        put(
            &mut bytes,
            state,
            (cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, state + cstate::ID, id.to_le_bytes());
    }
    put(
        &mut bytes,
        OPERATIVE + ACTION + operative_action::IMPLEMENTATION,
        (IMPLEMENTATION as u64).to_le_bytes(),
    );
    bytes
}

fn two_operatives() -> Vec<u8> {
    let mut bytes = fixture();
    bytes.resize(0xb000, 0);
    let other = 0x9000;
    let template = bytes[OPERATIVE..OPERATIVE + STRIDE].to_vec();
    bytes[other..other + STRIDE].copy_from_slice(&template);
    put(
        &mut bytes,
        other + IDENTITY,
        0x8000_0001_0000_1269_u64.to_le_bytes(),
    );
    let header = 0x5000 + cintelligence_agency::OPERATIVES;
    put(&mut bytes, header + 8, 2_u32.to_le_bytes());
    put(&mut bytes, header + 12, 2_u32.to_le_bytes());
    put(&mut bytes, 0x6008, (other as u64).to_le_bytes());
    bytes
}

fn mission(bytes: &mut [u8], variant: u32, implementation_vtable: usize) {
    put(
        bytes,
        OPERATIVE + ACTION + operative_action::VARIANT,
        variant.to_le_bytes(),
    );
    put(
        bytes,
        IMPLEMENTATION,
        (implementation_vtable as u64).to_le_bytes(),
    );
}

fn read(bytes: &[u8], rows: &mut Vec<Row>, arena: &mut Vec<u8>) -> Walk {
    walk_keyed(
        &World(bytes.to_vec()),
        0,
        STATE,
        engine_native_memory::walk::Buffer::Growing(rows),
        engine_native_memory::walk::Buffer::Growing(arena),
        &mut || Ok(KEY),
    )
}

fn count(bytes: &[u8]) -> Walk {
    walk_keyed(
        &World(bytes.to_vec()),
        0,
        STATE,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        &mut || Ok(KEY),
    )
}

fn row(bytes: &[u8]) -> Row {
    let mut rows = vec![Row::default(); 1];
    let read = read(bytes, &mut rows, &mut vec![0; 64]);
    assert_eq!(read.outcome, Outcome::Rows { len: 1 });
    rows[0]
}

fn refusal(bytes: &[u8]) -> (Outcome, Option<(u64, u32)>) {
    let read = read(bytes, &mut vec![Row::default(); 2], &mut vec![0; 64]);
    (read.outcome, read.failed_operative)
}

#[test]
fn active_roster_identity_order_owner_assignment_and_portrait_are_read() {
    let bytes = two_operatives();
    let mut rows = vec![Row::default(); 2];
    let mut arena = vec![0; 14];
    let read = read(&bytes, &mut rows, &mut arena);
    assert_eq!(read.outcome, Outcome::Rows { len: 2 });
    assert_eq!((read.bytes_len, read.required_bytes), (14, 14));
    assert_eq!(read.failed_operative, None);
    assert_eq!(
        rows[0],
        Row {
            native_pointer: OPERATIVE,
            map_selected: Some(0),
            roster_country: 1,
            id: HANDLE,
            ordinal: 0,
            owner_country: 7,
            assignment: 2,
            portrait: Name { start: 0, len: 7 },
            ..Row::default()
        }
    );
    assert_eq!(
        (rows[1].id, rows[1].ordinal, rows[1].portrait),
        (0x8000_0001_0000_1269, 1, Name { start: 7, len: 7 })
    );
    assert_eq!(&arena, b"GFX_spyGFX_spy");
}

#[test]
fn cooldown_operands_are_read_only_for_the_cooldown_assignment() {
    let mut bytes = fixture();
    put(
        &mut bytes,
        OPERATIVE + ASSIGNMENT,
        COOLDOWN_ASSIGNMENT.to_le_bytes(),
    );
    put(&mut bytes, OPERATIVE + COOLDOWN_KIND, 3_u32.to_le_bytes());
    put(
        &mut bytes,
        OPERATIVE + COOLDOWN_START_HOUR,
        (-24_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        OPERATIVE + COOLDOWN_END_HOUR,
        i32::MAX.to_le_bytes(),
    );
    let cooled = row(&bytes);
    assert_eq!(
        (
            cooled.assignment,
            cooled.cooldown_kind,
            cooled.cooldown_start_hour,
            cooled.cooldown_end_hour
        ),
        (COOLDOWN_ASSIGNMENT, Some(3), Some(-24), Some(i32::MAX))
    );
    put(
        &mut bytes,
        OPERATIVE + ASSIGNMENT,
        coperative_leader::MISSION_ASSIGNMENT.to_le_bytes(),
    );
    let on_mission = row(&bytes);
    assert_eq!(
        (
            on_mission.cooldown_kind,
            on_mission.cooldown_start_hour,
            on_mission.cooldown_end_hour
        ),
        (None, None, None)
    );
}

#[test]
fn state_variants_read_their_state_and_country_variants_only_their_country() {
    let mut bytes = fixture();
    put(&mut bytes, OPERATIVE + OWNER_COUNTRY, 1_u32.to_le_bytes());
    put(
        &mut bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_COUNTRY,
        u32::MAX.to_le_bytes(),
    );
    for (index, rva) in VTABLE_RVAS.into_iter().enumerate() {
        let variant = index as u32 + 1;
        mission(&mut bytes, variant, rva.0);
        let state = operative_action::targets_state(variant);
        put(
            &mut bytes,
            IMPLEMENTATION + operative_action_implementation::TARGET_POINTER,
            if state { TARGET_STATE as u64 } else { u64::MAX }.to_le_bytes(),
        );
        let row = row(&bytes);
        assert_eq!(
            (row.variant, row.target_country, row.target_state),
            (Some(variant), Some(u32::MAX), state.then_some(42)),
            "variant {variant}"
        );
    }
    assert_eq!(
        (1..=8)
            .filter(|&variant| operative_action::targets_state(variant))
            .collect::<Vec<_>>(),
        [1, 2, 4, 5, 8]
    );
}

#[test]
fn a_null_target_state_is_absent_while_its_country_stands() {
    let mut bytes = fixture();
    mission(
        &mut bytes,
        operative_action::BUILD_NETWORK,
        VTABLE_RVAS[0].0,
    );
    put(
        &mut bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_COUNTRY,
        9_u32.to_le_bytes(),
    );
    let row = row(&bytes);
    assert_eq!(
        (row.variant, row.target_country, row.target_state),
        (Some(1), Some(9), None)
    );
}

#[test]
fn no_mission_reads_no_implementation_whatever_the_assignment() {
    let mut bytes = fixture();
    put(
        &mut bytes,
        OPERATIVE + ACTION + operative_action::IMPLEMENTATION,
        u64::MAX.to_le_bytes(),
    );
    for assignment in [0_u32, 1, 2, 3, 4, 5] {
        put(&mut bytes, OPERATIVE + ASSIGNMENT, assignment.to_le_bytes());
        let row = row(&bytes);
        assert_eq!(
            (row.variant, row.target_country, row.target_state),
            (None, None, None)
        );
    }
}

#[test]
fn a_pending_mission_is_read_only_when_present() {
    let mut bytes = fixture();
    let pending = OPERATIVE + PENDING_MISSION;
    put(
        &mut bytes,
        pending + operative_action::PENDING_VARIANT,
        operative_action::BOOST_IDEOLOGY.to_le_bytes(),
    );
    put(
        &mut bytes,
        pending + operative_action::PENDING_TARGET_COUNTRY,
        u32::MAX.to_le_bytes(),
    );
    put(
        &mut bytes,
        pending + operative_action::PENDING_TARGET_STATE,
        (PENDING_STATE as u64).to_le_bytes(),
    );
    let absent = row(&bytes);
    assert_eq!(
        (
            absent.pending_variant,
            absent.pending_country,
            absent.pending_state
        ),
        (None, None, None)
    );
    put(&mut bytes, OPERATIVE + PENDING_MISSION_PRESENT, [1]);
    let present = row(&bytes);
    assert_eq!(
        (
            present.pending_variant,
            present.pending_country,
            present.pending_state
        ),
        (Some(5), Some(u32::MAX), Some(-3))
    );
    assert_eq!(present.variant, None);
    put(
        &mut bytes,
        pending + operative_action::PENDING_TARGET_STATE,
        0_u64.to_le_bytes(),
    );
    let stateless = row(&bytes);
    assert_eq!(
        (
            stateless.pending_variant,
            stateless.pending_country,
            stateless.pending_state
        ),
        (Some(5), Some(u32::MAX), None)
    );
    put(&mut bytes, OPERATIVE + PENDING_MISSION_PRESENT, [0x80]);
    assert_eq!(row(&bytes).pending_variant, Some(5));
    put(&mut bytes, OPERATIVE + PENDING_MISSION_PRESENT, [0]);
    put(
        &mut bytes,
        pending + operative_action::PENDING_TARGET_STATE,
        u64::MAX.to_le_bytes(),
    );
    assert_eq!(row(&bytes).pending_variant, None);
}

#[test]
fn mission_reads_refuse_unsupported_variants_implementations_and_states() {
    let mut bytes = fixture();
    mission(&mut bytes, 9, VTABLE_RVAS[0].0);
    assert_eq!(
        refusal(&bytes),
        (
            Outcome::Unavailable {
                reason: "unsupported operative mission variant"
            },
            Some((HANDLE, 1))
        )
    );
    mission(
        &mut bytes,
        operative_action::BUILD_NETWORK,
        VTABLE_RVAS[1].0,
    );
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "unsupported operative mission implementation vtable"
        }
    );
    mission(
        &mut bytes,
        operative_action::BUILD_NETWORK,
        VTABLE_RVAS[0].0,
    );
    put(
        &mut bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_POINTER,
        (OPERATIVE as u64).to_le_bytes(),
    );
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "operative mission state vtable mismatch"
        }
    );
    mission(&mut bytes, 0, VTABLE_RVAS[0].0);
    put(&mut bytes, OPERATIVE + PENDING_MISSION_PRESENT, [1]);
    put(
        &mut bytes,
        OPERATIVE + PENDING_MISSION + operative_action::PENDING_TARGET_STATE,
        (OPERATIVE as u64).to_le_bytes(),
    );
    assert_eq!(
        refusal(&bytes),
        (
            Outcome::Unavailable {
                reason: "operative mission state vtable mismatch"
            },
            Some((HANDLE, 1))
        )
    );
}

#[test]
fn roster_and_operative_identity_refusals_are_named() {
    let mut bytes = fixture();
    put(&mut bytes, OPERATIVE, 0_u64.to_le_bytes());
    assert_eq!(
        refusal(&bytes),
        (
            Outcome::Unavailable {
                reason: "operative vtable mismatch"
            },
            None
        )
    );
    let mut bytes = fixture();
    put(&mut bytes, OPERATIVE + IDENTITY, 0_u64.to_le_bytes());
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "operative has a null handle"
        }
    );
    let mut bytes = fixture();
    put(&mut bytes, 0x6000, 0_u64.to_le_bytes());
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "operative roster pointer unreadable or null"
        }
    );
    let mut bytes = fixture();
    put(
        &mut bytes,
        0x5000 + cintelligence_agency::OPERATIVES + 12,
        (-1_i32).to_le_bytes(),
    );
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "operative roster unreadable or invalid"
        }
    );
    let mut bytes = two_operatives();
    put(&mut bytes, 0x9000 + IDENTITY, HANDLE.to_le_bytes());
    assert_eq!(
        refusal(&bytes),
        (
            Outcome::Unavailable {
                reason: "duplicate operative handle within active roster"
            },
            Some((HANDLE, 1))
        )
    );
    for (address, value) in [(0x3000, 0_u64), (0x5000, 0)] {
        let mut bytes = fixture();
        put(&mut bytes, address, value.to_le_bytes());
        assert!(matches!(refusal(&bytes).0, Outcome::Unavailable { .. }));
    }
    let mut bytes = fixture();
    put(
        &mut bytes,
        0x3000 + ccountry::INTELLIGENCE_AGENCY,
        0_u64.to_le_bytes(),
    );
    assert_eq!(refusal(&bytes).0, Outcome::Rows { len: 0 });
}

#[test]
fn portrait_strings_refuse_invalid_headers_terminators_and_lengths() {
    let portrait = OPERATIVE + PORTRAIT_GFX;
    let mut bytes = fixture();
    put(&mut bytes, portrait + 24, 14_u64.to_le_bytes());
    assert_eq!(
        refusal(&bytes),
        (
            Outcome::Unavailable {
                reason: "operative portrait unreadable or invalid"
            },
            Some((HANDLE, 1))
        )
    );
    let mut bytes = fixture();
    put(
        &mut bytes,
        portrait + 16,
        (NAME_MAX_LENGTH + 1).to_le_bytes(),
    );
    put(
        &mut bytes,
        portrait + 24,
        (NAME_MAX_LENGTH + 1).to_le_bytes(),
    );
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "operative portrait exceeds native string maximum"
        }
    );
    let mut bytes = fixture();
    let heap = 0x8800;
    bytes[heap..heap + 20].copy_from_slice(b"GFX_portrait_spy_one");
    put(&mut bytes, portrait, (heap as u64).to_le_bytes());
    put(&mut bytes, portrait + 16, 20_u64.to_le_bytes());
    put(&mut bytes, portrait + 24, 31_u64.to_le_bytes());
    let mut rows = vec![Row::default(); 1];
    let mut arena = vec![0; 20];
    assert_eq!(
        read(&bytes, &mut rows, &mut arena).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(&arena, b"GFX_portrait_spy_one");
    bytes[heap + 20] = b'x';
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "operative portrait terminator unreadable or invalid"
        }
    );
}

#[test]
fn the_count_probe_reports_exact_native_demand() {
    let bytes = two_operatives();
    let sizing = count(&bytes);
    assert_eq!(
        sizing.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!((sizing.bytes_len, sizing.required_bytes), (0, 14));
    assert_eq!(sizing.failed_operative, None);
}

fn group(bytes: &mut [u8], group: usize, name: &[u8]) {
    put(
        bytes,
        group,
        (cideology_group::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    let token = group + cideology_group::TOKEN;
    bytes[token..token + name.len()].copy_from_slice(name);
    put(bytes, token + 16, (name.len() as u64).to_le_bytes());
    put(bytes, token + 24, 15_u64.to_le_bytes());
}

fn boost_ideology(bytes: &mut [u8], group: usize) {
    mission(bytes, operative_action::BOOST_IDEOLOGY, VTABLE_RVAS[4].0);
    put(
        bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_POINTER,
        (TARGET_STATE as u64).to_le_bytes(),
    );
    put(
        bytes,
        IMPLEMENTATION + operative_action_implementation::BOOST_IDEOLOGY,
        (group as u64).to_le_bytes(),
    );
}

#[test]
fn a_boost_ideology_mission_copies_its_group_name_before_the_portrait() {
    let mut bytes = fixture();
    group(&mut bytes, ACTIVE_GROUP, b"fascism");
    boost_ideology(&mut bytes, ACTIVE_GROUP);
    let mut rows = vec![Row::default(); 1];
    let mut arena = vec![0; 14];
    let read = read(&bytes, &mut rows, &mut arena);
    assert_eq!(read.outcome, Outcome::Rows { len: 1 });
    assert_eq!((read.bytes_len, read.required_bytes), (14, 14));
    assert_eq!(
        (
            rows[0].variant,
            rows[0].target_state,
            rows[0].ideology,
            rows[0].pending_ideology,
            rows[0].portrait
        ),
        (
            Some(operative_action::BOOST_IDEOLOGY),
            Some(42),
            Name { start: 0, len: 7 },
            Name::default(),
            Name { start: 7, len: 7 }
        )
    );
    assert_eq!(&arena, b"fascismGFX_spy");
}

#[test]
fn a_null_ideology_group_has_no_name() {
    let mut bytes = fixture();
    boost_ideology(&mut bytes, 0);
    let null = row(&bytes);
    assert_eq!(
        (null.variant, null.ideology, null.portrait),
        (
            Some(operative_action::BOOST_IDEOLOGY),
            Name::default(),
            Name { start: 0, len: 7 }
        )
    );
    let mut bytes = fixture();
    boost_ideology(&mut bytes, usize::MAX >> 1);
    mission(&mut bytes, operative_action::PROPAGANDA, VTABLE_RVAS[7].0);
    assert_eq!(row(&bytes).ideology, Name::default());
}

#[test]
fn a_pending_missions_group_is_copied_after_the_active_one() {
    let mut bytes = fixture();
    group(&mut bytes, ACTIVE_GROUP, b"fascism");
    group(&mut bytes, PENDING_GROUP, b"communism");
    boost_ideology(&mut bytes, ACTIVE_GROUP);
    let pending = OPERATIVE + PENDING_MISSION;
    put(&mut bytes, OPERATIVE + PENDING_MISSION_PRESENT, [1]);
    put(
        &mut bytes,
        pending + operative_action::PENDING_VARIANT,
        operative_action::BOOST_IDEOLOGY.to_le_bytes(),
    );
    put(
        &mut bytes,
        pending + operative_action::PENDING_IDEOLOGY,
        (PENDING_GROUP as u64).to_le_bytes(),
    );
    let mut rows = vec![Row::default(); 1];
    let mut arena = vec![0; 23];
    let read = read(&bytes, &mut rows, &mut arena);
    assert_eq!(read.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (rows[0].ideology, rows[0].pending_ideology, rows[0].portrait),
        (
            Name { start: 0, len: 7 },
            Name { start: 7, len: 9 },
            Name { start: 16, len: 7 }
        )
    );
    assert_eq!(&arena, b"fascismcommunismGFX_spy");
    mission(&mut bytes, 0, VTABLE_RVAS[4].0);
    let alone = row(&bytes);
    assert_eq!(
        (alone.ideology, alone.pending_ideology),
        (Name::default(), Name { start: 0, len: 9 })
    );
    put(
        &mut bytes,
        pending + operative_action::PENDING_IDEOLOGY,
        0_u64.to_le_bytes(),
    );
    assert_eq!(row(&bytes).pending_ideology, Name::default());
}

#[test]
fn an_ideology_group_vtable_mismatch_refuses_the_table() {
    let mut bytes = fixture();
    group(&mut bytes, ACTIVE_GROUP, b"fascism");
    boost_ideology(&mut bytes, ACTIVE_GROUP);
    put(&mut bytes, ACTIVE_GROUP, 0_u64.to_le_bytes());
    let mismatch = Outcome::Unavailable {
        reason: "operative mission ideology vtable mismatch",
    };
    assert_eq!(refusal(&bytes), (mismatch, Some((HANDLE, 1))));
    let mut bytes = fixture();
    put(&mut bytes, OPERATIVE + PENDING_MISSION_PRESENT, [1]);
    put(
        &mut bytes,
        OPERATIVE + PENDING_MISSION + operative_action::PENDING_IDEOLOGY,
        (PENDING_GROUP as u64).to_le_bytes(),
    );
    assert_eq!(refusal(&bytes), (mismatch, Some((HANDLE, 1))));
}

fn root_out(
    target: u32,
    country_factor: i64,
    operative_factor: i64,
    nationalities: &[u32],
) -> Vec<u8> {
    let mut bytes = fixture();
    let links = STATE + cgame_state::COUNTRY_LINKS;
    put(&mut bytes, links + 8, 3_u32.to_le_bytes());
    put(&mut bytes, links + 12, 3_u32.to_le_bytes());
    put(&mut bytes, 0x2108, 1_u32.to_le_bytes());
    put(&mut bytes, OPERATIVE + OWNER_COUNTRY, 1_u32.to_le_bytes());
    mission(
        &mut bytes,
        operative_action::ROOT_OUT_RESISTANCE,
        VTABLE_RVAS[3].0,
    );
    put(
        &mut bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_COUNTRY,
        target.to_le_bytes(),
    );
    put(
        &mut bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_POINTER,
        (TARGET_STATE as u64).to_le_bytes(),
    );
    for (accumulator, entries, value) in [
        (
            0x3000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            COUNTRY_ENTRIES,
            country_factor,
        ),
        (
            OPERATIVE + MODIFIER_ACCUMULATOR,
            OPERATIVE_ENTRIES,
            operative_factor,
        ),
    ] {
        put(&mut bytes, accumulator, (entries as u64).to_le_bytes());
        put(&mut bytes, accumulator + 8, 1_u32.to_le_bytes());
        put(&mut bytes, accumulator + 12, 1_u32.to_le_bytes());
        put(&mut bytes, entries, KEY.to_le_bytes());
        put(&mut bytes, entries + 8, value.to_le_bytes());
    }
    let list = OPERATIVE + NATIONALITIES;
    put(&mut bytes, list, (NATIONALITY_LIST as u64).to_le_bytes());
    put(
        &mut bytes,
        list + 8,
        (nationalities.len() as u32).to_le_bytes(),
    );
    put(
        &mut bytes,
        list + 12,
        (nationalities.len() as u32).to_le_bytes(),
    );
    for (index, nationality) in nationalities.iter().enumerate() {
        put(
            &mut bytes,
            NATIONALITY_LIST + index * 4,
            nationality.to_le_bytes(),
        );
    }
    bytes
}

fn resistance(bytes: &mut [u8], active: u8, country: u32) {
    let resistance = TARGET_STATE + cstate::RESISTANCE;
    put(bytes, resistance + RESISTANCE_ACTIVE, [active]);
    put(
        bytes,
        resistance + RESISTANCE_COUNTRY,
        country.to_le_bytes(),
    );
}

fn root_out_operands(bytes: &[u8]) -> (Option<i64>, Option<i64>, Option<u8>) {
    let row = row(bytes);
    (
        row.root_out_country_factor,
        row.root_out_operative_factor,
        row.root_out_nationality,
    )
}

#[test]
fn root_out_factors_are_the_owner_and_operative_effective_values() {
    let mut bytes = root_out(1, 25_000, -7_000, &[]);
    assert_eq!(
        root_out_operands(&bytes),
        (Some(25_000), Some(-7_000), Some(0))
    );
    put(
        &mut bytes,
        DEFINITIONS + KEY as usize * modifier_definition::STRIDE + modifier_definition::FLAGS,
        modifier_definition::CLAMP_MAXIMUM.to_le_bytes(),
    );
    assert_eq!(root_out_operands(&bytes), (Some(25_000), Some(0), Some(0)));
    let mut bytes = root_out(1, 25_000, -7_000, &[]);
    put(&mut bytes, OPERATIVE_ENTRIES, (KEY + 1).to_le_bytes());
    assert_eq!(root_out_operands(&bytes), (Some(25_000), Some(0), Some(0)));
    mission(
        &mut bytes,
        operative_action::BUILD_NETWORK,
        VTABLE_RVAS[0].0,
    );
    assert_eq!(root_out_operands(&bytes), (None, None, None));
    mission(&mut bytes, 0, VTABLE_RVAS[0].0);
    assert_eq!(root_out_operands(&bytes), (None, None, None));
}

#[test]
fn root_out_nationality_follows_the_active_resistance_country_or_the_target() {
    let bytes = root_out(1, 0, 0, &[2]);
    assert_eq!(root_out_operands(&bytes).2, Some(1));
    let mut bytes = root_out(0, 0, 0, &[2]);
    assert_eq!(root_out_operands(&bytes).2, Some(0));
    resistance(&mut bytes, 1, 2);
    assert_eq!(root_out_operands(&bytes).2, Some(1));
    resistance(&mut bytes, 0x80, 1);
    assert_eq!(root_out_operands(&bytes).2, Some(1));
    resistance(&mut bytes, 0, 2);
    assert_eq!(root_out_operands(&bytes).2, Some(0));
    resistance(&mut bytes, 1, (-5_i32) as u32);
    assert_eq!(root_out_operands(&bytes).2, Some(0));
    resistance(&mut bytes, 1, 0);
    assert_eq!(root_out_operands(&bytes).2, Some(0));
    resistance(&mut bytes, 1, 2);
    put(
        &mut bytes,
        IMPLEMENTATION + operative_action_implementation::TARGET_POINTER,
        0_u64.to_le_bytes(),
    );
    let row = row(&bytes);
    assert_eq!(
        (row.target_state, row.root_out_nationality),
        (None, Some(0))
    );
    let bytes = root_out(0, 0, 0, &[0, 2]);
    assert_eq!(root_out_operands(&bytes).2, Some(1));
    let bytes = root_out(1, 0, 0, &[0, 1]);
    assert_eq!(root_out_operands(&bytes).2, Some(1));
}

#[test]
fn root_out_operands_refuse_unresolvable_owners_and_unreadable_accumulators() {
    let mut bytes = root_out(1, 0, 0, &[2]);
    put(&mut bytes, OPERATIVE + OWNER_COUNTRY, 7_u32.to_le_bytes());
    assert!(matches!(
        refusal(&bytes),
        (Outcome::Unavailable { .. }, Some((HANDLE, 1)))
    ));
    let mut bytes = root_out(1, 0, 0, &[2]);
    put(
        &mut bytes,
        OPERATIVE + MODIFIER_ACCUMULATOR + 12,
        2_u32.to_le_bytes(),
    );
    assert_eq!(
        refusal(&bytes).0,
        Outcome::Unavailable {
            reason: "modifier accumulator header unreadable or invalid"
        }
    );
    let bytes = root_out(1, 0, 0, &[7]);
    assert!(matches!(refusal(&bytes).0, Outcome::Unavailable { .. }));
}

#[test]
fn the_root_out_key_is_resolved_by_name_only_for_root_out_missions() {
    let bytes = root_out(1, 0, 0, &[]);
    let memory = World(bytes.clone());
    let read = walk(&memory, 0, STATE, &mut vec![Row::default(); 1], &mut vec![0; 64], &mut root_out_key());
    assert_eq!(
        (read.outcome, read.failed_operative),
        (
            Outcome::Unavailable {
                reason: "modifier definition table unreadable"
            },
            Some((HANDLE, 1))
        )
    );
    let mut other = bytes;
    mission(
        &mut other,
        operative_action::BUILD_NETWORK,
        VTABLE_RVAS[0].0,
    );
    let read = walk(
        &World(other),
        0,
        STATE,
        &mut vec![Row::default(); 1],
        &mut vec![0; 64],
        &mut root_out_key(),
    );
    assert_eq!(read.outcome, Outcome::Rows { len: 1 });
    let mut asked = 0;
    let read = walk_keyed(
        &memory,
        0,
        STATE,
        engine_native_memory::walk::Buffer::Growing(&mut vec![Row::default(); 1]),
        engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]),
        &mut || {
            asked += 1;
            Err("root out resistance factor modifier is not registered")
        },
    );
    assert_eq!(
        read.outcome,
        Outcome::Unavailable {
            reason: "root out resistance factor modifier is not registered"
        }
    );
    assert_eq!(asked, 1);
}
