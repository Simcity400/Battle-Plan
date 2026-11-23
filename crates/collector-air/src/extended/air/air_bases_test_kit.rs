use super::{AirBasesCopyContext, AirBasesCopyReport, AirBasesJob, Capacities, Captured};
use engine_native_memory::{Memory, memory::SliceMemory, walk::Outcome};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
pub const LTAB: usize = 0x12000;
pub const C0: usize = 0x20000;
const C1: usize = 0x21000;
pub const MANAGER: usize = 0x30000;
pub const SVEC: usize = 0x31000;
pub const HVEC: usize = 0x32000;
pub const B0: usize = 0x40000;
pub const B1: usize = 0x41000;
pub const H0: usize = 0x42000;
pub const S0: usize = 0x50000;
pub const S1: usize = 0x50100;
pub const P0: usize = 0x51000;
pub const P1: usize = 0x51100;
const CVEC0: usize = 0x60000;
pub const CONT1: usize = 0x61000;
const POOLVEC0: usize = 0x61400;
pub const POOL0: usize = 0x62000;
const WVEC: usize = 0x62400;
pub const W0: usize = 0x63000;
pub const W1: usize = 0x63100;
const CVEC1: usize = 0x64000;
pub const CONT2: usize = 0x65000;
const POOLVEC1: usize = 0x65400;
const ACES0: usize = 0x70000;
pub const A0: usize = 0x71000;
const A1: usize = 0x72000;
pub const NSTR: usize = 0x73000;
const RVEC2: usize = 0x32100;
const B2: usize = 0x43000;
const S2: usize = 0x50200;
const P2: usize = 0x51200;
const CVEC2: usize = 0x66000;

pub const WING_HANDLE_0: u64 = (0x77 << 32) | engine_native_parts::cair_wing_layout::TYPE as u64;
pub const WING_HANDLE_1: u64 = (0x78 << 32) | engine_native_parts::cair_wing_layout::TYPE as u64;

pub fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put32(bytes, at + 8, count);
    put32(bytes, at + 12, count);
}

pub fn fixture() -> Vec<u8> {
    fixture_inner()
}

pub fn add_alias_and_rocket(bytes: &mut [u8]) {
    use engine_native_air::{cair_base, cstrategic_air_manager};
    use engine_native::{cgame_state, cprovince, cstate};
    header(bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
    put64(bytes, CTAB + 24, C0 as u64);
    put32(bytes, LTAB + 12, 1);
    header(
        bytes,
        MANAGER + cstrategic_air_manager::STATE_ROCKET_BASES,
        RVEC2,
        2,
    );
    put64(bytes, RVEC2, 0);
    put64(bytes, RVEC2 + 8, B2 as u64);
    put64(bytes, B2, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
    put64(
        bytes,
        B2 + engine_native_parts::cair_base_layout::HANDLE,
        (0x44u64 << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE),
    );
    put64(bytes, B2 + engine_native_parts::cair_base_layout::HOST_REFERENCE, 0);
    put64(bytes, B2 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 8, S2 as u64);
    put64(bytes, B2 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 16, P2 as u64);
    put32(bytes, B2 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 24, 42);
    put32(bytes, B2 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 28, 0);
    put64(bytes, S2, cstate::VTABLE_RVA.0 as u64);
    put32(bytes, S2 + cstate::ID, 1);
    put64(bytes, P2, cprovince::VTABLE_RVA.0 as u64);
    put32(bytes, P2 + cprovince::ID, 7);
    put64(bytes, P2 + cprovince::STATE, S2 as u64);
    header(bytes, B2 + cair_base::COUNTRY_CONTAINERS, CVEC2, 0);
}

pub fn add_reserved_alias(bytes: &mut [u8]) {
    use engine_native::{ccountry, cgame_state};
    const RESERVED: usize = 0x22000;
    header(bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
    put64(bytes, CTAB, RESERVED as u64);
    put64(bytes, CTAB + 24, RESERVED as u64);
    put64(bytes, RESERVED, ccountry::VTABLE_RVA.0 as u64);
    put32(bytes, RESERVED + ccountry::ID, 0);
}

fn fixture_inner() -> Vec<u8> {
    use engine_native_air::{cair_base, ccountry_air_container, cstrategic_air_manager};
    use engine_native_parts::cair_wing_pool;
    use engine_native::{ccountry, cgame_state, cprovince, cstate};
    use engine_native_memory::msvc_string;
    let mut bytes = vec![0; 0x80000];
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 3);
    put64(&mut bytes, CTAB, 0);
    put64(&mut bytes, CTAB + 8, C0 as u64);
    put64(&mut bytes, CTAB + 16, C1 as u64);
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
    put32(&mut bytes, LTAB, 0);
    put32(&mut bytes, LTAB + 4, 1);
    put32(&mut bytes, LTAB + 8, 2);
    put32(&mut bytes, LTAB + 12, 0);
    for (country, id) in [(C0, 1u32), (C1, 2u32)] {
        put64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
        put32(&mut bytes, country + ccountry::ID, id);
    }
    header(&mut bytes, C0 + ccountry::ACES, ACES0, 2);
    put64(&mut bytes, ACES0, A0 as u64);
    put64(&mut bytes, ACES0 + 8, A1 as u64);
    header(&mut bytes, C1 + ccountry::ACES, 0, 0);
    put64(
        &mut bytes,
        GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    put64(
        &mut bytes,
        MANAGER,
        cstrategic_air_manager::VTABLE_RVA.0 as u64,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATE_AIR_BASES,
        SVEC,
        3,
    );
    put64(&mut bytes, SVEC, 0);
    put64(&mut bytes, SVEC + 8, B0 as u64);
    put64(&mut bytes, SVEC + 16, B1 as u64);
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATE_ROCKET_BASES,
        0,
        0,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATE_GUN_BASES,
        0,
        0,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HVEC,
        1,
    );
    put64(&mut bytes, HVEC, H0 as u64);
    for (base, id, state, state_id, province, province_id, capacity, kind) in [
        (B0, 0x41u64, S0, 1u32, P0, 5u32, 1200i32, 0i32),
        (B1, 0x42u64, S1, 2u32, P1, 6u32, -3i32, 1i32),
    ] {
        put64(&mut bytes, base, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
        put64(
            &mut bytes,
            base + engine_native_parts::cair_base_layout::HANDLE,
            (id << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE),
        );
        put64(&mut bytes, base + engine_native_parts::cair_base_layout::HOST_REFERENCE, 0);
        put64(
            &mut bytes,
            base + engine_native_parts::cair_base_layout::HOST_REFERENCE + 8,
            state as u64,
        );
        put64(
            &mut bytes,
            base + engine_native_parts::cair_base_layout::HOST_REFERENCE + 16,
            province as u64,
        );
        put32(
            &mut bytes,
            base + engine_native_parts::cair_base_layout::HOST_REFERENCE + 24,
            capacity as u32,
        );
        put32(
            &mut bytes,
            base + engine_native_parts::cair_base_layout::HOST_REFERENCE + 28,
            kind as u32,
        );
        put64(&mut bytes, state, cstate::VTABLE_RVA.0 as u64);
        put32(&mut bytes, state + cstate::ID, state_id);
        put64(&mut bytes, province, cprovince::VTABLE_RVA.0 as u64);
        put32(&mut bytes, province + cprovince::ID, province_id);
        put64(&mut bytes, province + cprovince::STATE, state as u64);
    }
    put64(&mut bytes, H0, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        H0 + engine_native_parts::cair_base_layout::HANDLE,
        (0x43u64 << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE),
    );
    put64(
        &mut bytes,
        H0 + engine_native_parts::cair_base_layout::HOST_REFERENCE,
        (3u64 << 32) | 51,
    );
    put64(&mut bytes, H0 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 8, 0);
    put64(&mut bytes, H0 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 16, 0);
    put32(
        &mut bytes,
        H0 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 24,
        (-17i32) as u32,
    );
    put32(&mut bytes, H0 + engine_native_parts::cair_base_layout::HOST_REFERENCE + 28, 1);
    header(&mut bytes, H0 + cair_base::COUNTRY_CONTAINERS, 0, 0);
    header(&mut bytes, B0 + cair_base::COUNTRY_CONTAINERS, CVEC0, 2);
    put64(&mut bytes, CVEC0, 0);
    put64(&mut bytes, CVEC0 + 8, CONT1 as u64);
    header(&mut bytes, B1 + cair_base::COUNTRY_CONTAINERS, CVEC1, 3);
    put64(&mut bytes, CVEC1, 0);
    put64(&mut bytes, CVEC1 + 8, 0);
    put64(&mut bytes, CVEC1 + 16, CONT2 as u64);
    for (container, country, poolvec, pools) in
        [(CONT1, 1u32, POOLVEC0, 1u32), (CONT2, 2u32, POOLVEC1, 0u32)]
    {
        put64(
            &mut bytes,
            container,
            ccountry_air_container::VTABLE_RVA.0 as u64,
        );
        put32(
            &mut bytes,
            container + ccountry_air_container::COUNTRY,
            country,
        );
        header(
            &mut bytes,
            container + ccountry_air_container::POOLS,
            poolvec,
            pools,
        );
    }
    put64(&mut bytes, POOLVEC0, POOL0 as u64);
    put64(&mut bytes, POOL0, cair_wing_pool::VTABLE_RVA.0 as u64);
    header(&mut bytes, POOL0 + cair_wing_pool::WINGS, WVEC, 2);
    put64(&mut bytes, WVEC, W0 as u64);
    put64(&mut bytes, WVEC + 8, W1 as u64);
    for (wing, handle) in [(W0, WING_HANDLE_0), (W1, WING_HANDLE_1)] {
        put64(&mut bytes, wing, engine_native_parts::cair_wing_layout::VTABLE_RVA.0 as u64);
        put64(&mut bytes, wing + 8, POOL0 as u64);
        put64(
            &mut bytes,
            wing + 16,
            engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put64(&mut bytes, wing + 24, handle);
        put64(&mut bytes, wing + engine_native_parts::cair_wing_layout::POOL, POOL0 as u64);
    }
    ace(
        &mut bytes,
        A0,
        C0,
        0xA0,
        1,
        1,
        7,
        None,
        [("Al", true), ("Bo", true), ("Cy", true)],
    );
    ace(
        &mut bytes,
        A1,
        C0,
        0xA1,
        0,
        0,
        3,
        Some((0xB0u64, 9u32)),
        [("DeadAce", false), ("", false), ("Z", false)],
    );
    put_string(&mut bytes, NSTR, b"DeadAce");
    put_string(&mut bytes, NSTR + 8, b"");
    put_string(&mut bytes, NSTR + 9, b"Z");
    let _ = msvc_string::INLINE_CAPACITY;
    bytes
}

fn ace(
    bytes: &mut [u8],
    at: usize,
    owner: usize,
    id: u64,
    alive: u8,
    handled: u8,
    kill_type: u32,
    killer: Option<(u64, u32)>,
    names: [(&str, bool); 3],
) {
    #![allow(clippy::too_many_arguments)]
    use engine_native_air::cace;
    use engine_native_memory::msvc_string;
    put64(bytes, at, cace::VTABLE_RVA.0 as u64);
    put64(bytes, at + cace::REFERENCE, (id << 32) | 0xace);
    put32(bytes, at + cace::AIRCRAFT_TYPE_MASK, 0xF);
    put64(bytes, at + cace::OWNER_COUNTRY, owner as u64);
    put64(bytes, at + cace::OWNER_COUNTRY + 8, WING_HANDLE_0);
    for (index, (text, inline)) in names.into_iter().enumerate() {
        let head = at + [cace::NAME, cace::SURNAME, cace::CALLSIGN][index];
        if inline {
            bytes[head..head + text.len()].copy_from_slice(text.as_bytes());
            bytes[head + text.len()] = 0;
            put64(bytes, head + msvc_string::LENGTH, text.len() as u64);
            put64(
                bytes,
                head + msvc_string::CAPACITY,
                msvc_string::INLINE_CAPACITY,
            );
        } else {
            let source = NSTR + heap_offset(index);
            put64(bytes, head + msvc_string::DATA, source as u64);
            put64(bytes, head + msvc_string::LENGTH, text.len() as u64);
            put64(bytes, head + msvc_string::CAPACITY, 16);
        }
    }
    bytes[at + cace::ALIVE] = alive;
    put32(bytes, at + cace::KILL_TYPE, kill_type);
    if let Some((killer_ace, killer_country)) = killer {
        put64(bytes, at + cace::KILLER_ACE_REFERENCE, killer_ace);
        put32(bytes, at + cace::KILLER_COUNTRY, killer_country);
    }
    bytes[at + cace::HANDLED] = handled;
}

fn heap_offset(index: usize) -> usize {
    [0, 8, 9][index]
}

fn put_string(bytes: &mut [u8], at: usize, text: &[u8]) {
    bytes[at..at + text.len()].copy_from_slice(text);
    bytes[at + text.len()] = 0;
}

pub const GUI_SINGLETON: usize = 0x7ff00;

pub struct SyntheticMemory<'a> {
    bytes: &'a [u8],
}

impl Memory for SyntheticMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let address = if address == engine_native_politics::cgui::SINGLETON_RVA.0 && output.len() == 8 {
            GUI_SINGLETON
        } else {
            address
        };
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

pub fn mod_fixture(count: usize) -> Vec<u8> {
    let mut bytes = fixture();
    let bases = bytes.len();
    let aces = bases + count * 8;
    bytes.resize(aces + count * 8, 0);
    header(
        &mut bytes,
        MANAGER + engine_native_air::cstrategic_air_manager::STATELESS_AIR_BASES,
        bases,
        count as u32,
    );
    header(
        &mut bytes,
        C0 + engine_native::ccountry::ACES,
        aces,
        count as u32,
    );
    for index in 0..count {
        put64(&mut bytes, bases + index * 8, H0 as u64);
        put64(&mut bytes, aces + index * 8, A0 as u64);
    }
    bytes
}

pub fn memory(bytes: &[u8]) -> SyntheticMemory<'_> {
    SyntheticMemory { bytes }
}

pub struct Split {
    pub bases_outcome: Outcome,
    pub bases: Vec<engine_native_air::cair_base::Row>,
    pub aces_outcome: Outcome,
    pub aces: Vec<engine_native_air::cace::Row>,
    pub ace_names: Vec<u8>,
    pub ace_bytes_len: usize,
    pub ace_required: usize,
    pub regs: crate::extended::fog::RegistrationsSnapshot,
}

pub(super) fn collect_typed(
    memory: &impl Memory,
    air_rows: [usize; super::SIZED],
    ace_names: usize,
    fog_rows: usize,
    fog_wings: usize,
) -> Split {
    use engine_native_air::{cace, cair_base};
    let capacities = Capacities {
        rows: air_rows,
        ace_names,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    };
    let mut captured = Captured::with_capacities(&capacities);
    let walked = cair_base::walk(memory, BASE, GAME_STATE, &mut captured.bases.rows);
    captured.bases.outcome = super::bounded(walked, captured.bases.rows.len());
    let walked = cace::walk(
        memory,
        BASE,
        GAME_STATE,
        &mut captured.aces.rows,
        &mut captured.names.first,
    );
    captured.aces.outcome =
        super::names_outcome(walked, &captured.aces.rows, &captured.names.first);
    captured.names.len = walked.bytes_len;
    captured.names.required = walked.required_bytes;
    let regs = crate::extended::fog::Captured::collect_registrations_direct(
        fog_rows, fog_wings, memory, BASE, GAME_STATE,
    );
    Split {
        bases_outcome: captured.bases.outcome,
        bases: captured.bases.rows.clone(),
        aces_outcome: captured.aces.outcome,
        aces: captured.aces.rows.clone(),
        ace_names: captured.names.first.clone(),
        ace_bytes_len: captured.names.len,
        ace_required: captured.names.required,
        regs,
    }
}

pub(super) fn collect_split(
    memory: &impl Memory,
    air_rows: [usize; super::SIZED],
    ace_names: usize,
    fog_rows: usize,
    fog_wings: usize,
) -> (Split, AirBasesCopyReport, AirBasesJob) {
    let ctx = AirBasesCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
    };
    let mut first = AirBasesJob::fixture();
    let report = super::copy_air_bases_fixture(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let split = decode_with(&first, air_rows, ace_names, fog_rows, fog_wings);
    (split, report, first)
}

pub fn decode(job: &AirBasesJob) -> Split {
    decode_with(job, Capacities::fixture().rows, 65_536, 2048, 4096)
}

fn decode_with(
    job: &AirBasesJob,
    air_rows: [usize; super::SIZED],
    ace_names: usize,
    fog_rows: usize,
    fog_wings: usize,
) -> Split {
    let capacities = Capacities {
        rows: air_rows,
        ace_names,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    };
    let mut air = Captured::with_capacities(&capacities);
    let mut fog =
        crate::extended::fog::Captured::with_capacities(&crate::extended::fog::Capacities {
            rows: [4, 4, 4, 4, 4, fog_rows, 4],
            radar_provinces: 8,
            area_provinces: 8,
            wings: fog_wings,
        });
    fog.registrations_raw_mode = true;
    let mut ticks = [0u64; 3];
    air.decode_air_bases_payload(BASE, job, &mut ticks);
    fog.decode_registrations_payload(BASE, job, &mut ticks);
    Split {
        bases_outcome: air.bases.outcome,
        bases: air.bases.rows.clone(),
        aces_outcome: air.aces.outcome,
        aces: air.aces.rows.clone(),
        ace_names: air.names.first.clone(),
        ace_bytes_len: air.names.len,
        ace_required: air.names.required,
        regs: fog.registrations_snapshot(),
    }
}

pub fn direct(memory: &impl Memory) -> Split {
    collect_typed(memory, Capacities::fixture().rows, 65_536, 2048, 4096)
}

pub fn split(memory: &impl Memory) -> (Split, AirBasesCopyReport, AirBasesJob) {
    collect_split(memory, Capacities::fixture().rows, 65_536, 2048, 4096)
}

pub fn assert_no_mandatory_growth(job: &AirBasesJob) {
    assert!(!job.log.grow_for_next());
    assert!(job.required_census <= job.census.capacity());
    assert!(job.required_links <= job.seen_links.capacity());
    assert_eq!(job.grow_for_next(), job.carrier.grow_for_next());
}

pub fn direct_with(
    memory: &impl Memory,
    bases: usize,
    wings: usize,
    aces: usize,
    names: usize,
) -> Split {
    collect_typed(
        memory,
        [8192, bases, 4096, aces, 2048, 4096, 1024, 64, 0],
        names,
        bases.max(2),
        wings,
    )
}

pub fn split_with(
    memory: &impl Memory,
    bases: usize,
    wings: usize,
    aces: usize,
    names: usize,
) -> (Split, AirBasesCopyReport, AirBasesJob) {
    collect_split(
        memory,
        [8192, bases, 4096, aces, 2048, 4096, 1024, 64, 0],
        names,
        bases.max(2),
        wings,
    )
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.bases_outcome, direct.bases_outcome, "bases outcome");
    assert_eq!(split.bases, direct.bases, "bases rows");
    assert_eq!(split.aces_outcome, direct.aces_outcome, "aces outcome");
    assert_eq!(split.aces, direct.aces, "aces rows");
    assert_eq!(split.ace_bytes_len, direct.ace_bytes_len, "aces bytes len");
    assert_eq!(split.ace_required, direct.ace_required, "aces required");
    assert_eq!(
        split.ace_names[..split.ace_bytes_len],
        direct.ace_names[..direct.ace_bytes_len],
        "aces names"
    );
    assert_eq!(
        split.regs.outcome, direct.regs.outcome,
        "registrations outcome"
    );
    assert_eq!(split.regs.rows, direct.regs.rows, "registrations rows");
    assert_eq!(split.regs.wings, direct.regs.wings, "registrations wings");
}
