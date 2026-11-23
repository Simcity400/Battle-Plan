use super::{construction_rest_test_kit as kit, *};
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const SOURCE_STATUS: usize = 0x8000;
const SOURCE_RELATION: usize = 0x8200;
const TARGET: usize = 0x14000;
const TARGET_STATUS: usize = 0x16000;
const TARGET_RELATION: usize = 0x16500;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - kit::BASE..address - kit::BASE + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put(bytes, address, &(data as u64).to_le_bytes());
    put(bytes, address + 8, &count.to_le_bytes());
    put(bytes, address + 12, &count.to_le_bytes());
}

fn fixture(progress: i64) -> Vec<u8> {
    let mut bytes = kit::fixture();
    bytes.resize(0x17000 - kit::BASE, 0);
    array(&mut bytes, kit::BASE + cgame_state::COUNTRIES, 0x2000, 4);
    array(
        &mut bytes,
        kit::BASE + cgame_state::COUNTRY_LINKS,
        0x2100,
        4,
    );
    put(&mut bytes, 0x2018, &(TARGET as u64).to_le_bytes());
    put(&mut bytes, 0x210c, &3_u32.to_le_bytes());
    put(
        &mut bytes,
        TARGET,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(&mut bytes, TARGET + ccountry::ID, &3_u32.to_le_bytes());
    put(
        &mut bytes,
        TARGET + ccountry::COLLABORATION_STATUS,
        &(TARGET_STATUS as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TARGET_STATUS,
        &(ccountry_collaboration_status::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        TARGET_STATUS + ccountry_collaboration_status::OWNER,
        &(TARGET as u64).to_le_bytes(),
    );
    for (status, outgoing, relation, incoming, source, target) in [
        (SOURCE_STATUS, 0x8100, SOURCE_RELATION, 0x8300, 3_u32, 3_u32),
        (TARGET_STATUS, 0x16400, TARGET_RELATION, 0x16600, 2, 2),
    ] {
        array(
            &mut bytes,
            status + ccountry_collaboration_status::RELATIONS,
            outgoing,
            1,
        );
        put(&mut bytes, outgoing, &(relation as u64).to_le_bytes());
        array(
            &mut bytes,
            status + ccountry_collaboration_status::INCOMING_COUNTRIES,
            incoming,
            1,
        );
        put(&mut bytes, incoming, &source.to_le_bytes());
        put(
            &mut bytes,
            relation,
            &(ccountry_collaboration_data::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            relation + ccountry_collaboration_data::TARGET_COUNTRY,
            &target.to_le_bytes(),
        );
        put(
            &mut bytes,
            relation + ccountry_collaboration_data::PROGRESS,
            &progress.to_le_bytes(),
        );
    }
    bytes
}

struct Fault<'a> {
    bytes: &'a [u8],
    address: usize,
    len: usize,
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.len != 0
            && address < self.address + self.len
            && self.address < address + output.len()
        {
            return false;
        }
        SliceMemory {
            base: kit::BASE,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

fn assert_rows(captured: &Captured, progress: i64) {
    assert_eq!(captured.collaboration.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        captured.tables()[11],
        TableView::Complete(rows::TableRows::CountryCollaboration(vec![
            rows::CountryCollaborationRow {
                source: 1,
                target: 3,
                progress
            },
            rows::CountryCollaborationRow {
                source: 3,
                target: 2,
                progress
            },
        ]))
    );
}

#[test]
fn collaboration_owned_nonempty_relations_keep_exact_fields_aliases_and_signed_progress() {
    for progress in [i64::MIN, -1, 0, i64::MAX] {
        let bytes = fixture(progress);
        let memory = Fault {
            bytes: &bytes,
            address: SOURCE_RELATION + 12,
            len: 4,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        assert_rows(&direct, progress);
        for capacity in [0, 1, 8] {
            let mut capacities = kit::capacities();
            capacities.rows[11] = capacity;
            let mut job = kit::job();
            let ctx = ConstructionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
            };
            let report = copy_construction_rest_refresh(&memory, ctx, &mut job);
            assert!(report.raw_ok);
            assert!(!job.log.fault[9]);
            for (address, len) in [
                (SOURCE_RELATION, 8),
                (
                    SOURCE_RELATION + ccountry_collaboration_data::TARGET_COUNTRY,
                    4,
                ),
                (SOURCE_RELATION + ccountry_collaboration_data::PROGRESS, 8),
            ] {
                assert!(
                    job.log
                        .spans
                        .iter()
                        .any(|span| span.addr == address && span.len == len && span.ok)
                );
            }
            let mut captured = Captured::with_capacities(&capacities);
            captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
            assert_rows(&captured, progress);
            assert_eq!(captured.tables()[11], direct.tables()[11]);
        }
    }
}

#[test]
fn collaboration_pooled_refresh_recovers_on_the_first_exact_hour_after_every_local_failure() {
    for (offset, len, reason) in [
        (0, 8, "collaboration relation vtable mismatch or unreadable"),
        (
            ccountry_collaboration_data::TARGET_COUNTRY,
            4,
            "collaboration target unreadable",
        ),
        (
            ccountry_collaboration_data::PROGRESS,
            8,
            "collaboration progress unreadable",
        ),
    ] {
        let mut captured = Captured::with_capacities(&kit::capacities());
        let mut job = kit::job();
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
        };
        for (hour, present, failed, progress) in [
            (100, false, false, 0),
            (101, true, false, 25_001),
            (102, true, true, 25_002),
            (103, true, false, 39_997),
            (104, false, false, 0),
        ] {
            let bytes = if present {
                fixture(progress)
            } else {
                kit::fixture()
            };
            let memory = Fault {
                bytes: &bytes,
                address: SOURCE_RELATION + offset,
                len: if failed { len } else { 0 },
            };
            captured.reset();
            let report = copy_construction_rest_refresh(&memory, ctx, &mut job);
            assert!(report.raw_ok, "{hour}");
            assert_eq!(job.log.fault[9], failed, "{hour}");
            captured.rest_raw_mode = true;
            captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
            assert!(!captured.rest_raw_mode);
            if failed {
                assert_eq!(
                    captured.collaboration.outcome,
                    Outcome::Unavailable { reason },
                    "{hour}"
                );
                assert_eq!(captured.tables()[11], unavailable(reason), "{hour}");
                assert_eq!(
                    captured.tables()[11],
                    kit::direct(&memory, &kit::capacities()).tables()[11],
                    "{hour}"
                );
            } else if present {
                assert_rows(&captured, progress);
            } else {
                assert_eq!(
                    captured.collaboration.outcome,
                    Outcome::Rows { len: 0 },
                    "{hour}"
                );
                assert_eq!(
                    captured.tables()[11],
                    TableView::Complete(rows::TableRows::CountryCollaboration(vec![]))
                );
            }
        }
    }
}

#[test]
fn collaboration_owned_wrong_kind_refuses_only_its_hour_and_decode_uses_owned_values() {
    let mut captured = Captured::with_capacities(&kit::capacities());
    let mut job = kit::job();
    let ctx = ConstructionRestCopyContext {
        base: 0,
        game_state: kit::BASE,
    };
    for vtable in [
        ccountry::VTABLE_RVA.0,
        0,
        ccountry_collaboration_data::VTABLE_RVA.0,
    ] {
        let mut bytes = fixture(12_345);
        put(&mut bytes, SOURCE_RELATION, &(vtable as u64).to_le_bytes());
        let report = copy_construction_rest_refresh(
            &SliceMemory {
                base: kit::BASE,
                bytes: &bytes,
            },
            ctx,
            &mut job,
        );
        assert!(report.raw_ok);
        assert!(!job.log.fault[9]);
        bytes.fill(0);
        captured.reset();
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        if vtable == ccountry_collaboration_data::VTABLE_RVA.0 {
            assert_rows(&captured, 12_345);
        } else {
            assert_eq!(
                captured.collaboration.outcome,
                Outcome::Unavailable {
                    reason: "collaboration relation vtable mismatch or unreadable"
                }
            );
        }
    }
}

const LINKS: usize = 0x2100;
const OUTGOING_ALIAS: u32 = 4;
const INCOMING_ALIAS: u32 = 5;

fn alias_fixture(progress: i64, outgoing: bool, incoming: bool) -> Vec<u8> {
    let mut bytes = fixture(progress);
    array(
        &mut bytes,
        kit::BASE + cgame_state::COUNTRY_LINKS,
        LINKS,
        if outgoing && !incoming { 5 } else { 7 },
    );
    put(
        &mut bytes,
        LINKS + OUTGOING_ALIAS as usize * 4,
        &3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        LINKS + INCOMING_ALIAS as usize * 4,
        &1_u32.to_le_bytes(),
    );
    if outgoing {
        put(
            &mut bytes,
            SOURCE_RELATION + ccountry_collaboration_data::TARGET_COUNTRY,
            &OUTGOING_ALIAS.to_le_bytes(),
        );
    }
    if incoming {
        put(&mut bytes, 0x16600, &INCOMING_ALIAS.to_le_bytes());
    }
    bytes
}

struct ExactLinks<'a> {
    bytes: &'a [u8],
    demanded: &'a [u32],
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for ExactLinks<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if address < LINKS + 7 * 4 && LINKS < address + output.len() {
            if address < LINKS || output.len() != 4 || (address - LINKS) % 4 != 0 {
                return false;
            }
            let raw = ((address - LINKS) / 4) as u32;
            if !self.demanded.contains(&raw) {
                return false;
            }
        }
        SliceMemory {
            base: kit::BASE,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

#[test]
fn collaboration_owned_captures_only_demanded_aliases_and_every_decoder_read_exactly() {
    for (outgoing, incoming) in [(true, false), (false, true), (true, true)] {
        let mut bytes = alias_fixture(-37_901, outgoing, incoming);
        let mut demanded = vec![1, 2, 3];
        if outgoing {
            demanded.push(OUTGOING_ALIAS);
        }
        if incoming {
            demanded.push(INCOMING_ALIAS);
        }
        let memory = ExactLinks {
            bytes: &bytes,
            demanded: &demanded,
            reads: RefCell::default(),
        };
        let mut rows = vec![ccountry_collaboration_data::Row::default(); 8];
        assert_eq!(
            ccountry_collaboration_status::walk(&memory, 0, kit::BASE, &mut rows),
            Outcome::Rows { len: 2 },
        );
        assert_eq!(rows[0].source, 1);
        assert_eq!(rows[0].target, if outgoing { OUTGOING_ALIAS } else { 3 });
        assert_eq!(rows[1].source, 3);
        assert_eq!(rows[1].target, 2);
        let decoder_reads = memory.reads.take();
        let mut job = kit::job();
        assert!(
            copy_construction_rest_refresh(
                &memory,
                ConstructionRestCopyContext {
                    base: 0,
                    game_state: kit::BASE
                },
                &mut job,
            )
            .raw_ok
        );
        assert!(!job.log.fault[9]);
        for (address, len) in decoder_reads {
            assert!(
                job.log.spans.iter().any(|span| {
                    span.addr == address && span.len as usize == len && span.ok && !span.bulk
                }),
                "uncaptured decoder read {address:#x}, {len}"
            );
        }
        let mut copied_links: Vec<_> = memory
            .reads
            .take()
            .into_iter()
            .filter(|(address, _)| (LINKS..LINKS + 7 * 4).contains(address))
            .map(|(address, len)| {
                assert_eq!(len, 4);
                ((address - LINKS) / 4) as u32
            })
            .collect();
        copied_links.sort_unstable();
        copied_links.dedup();
        assert_eq!(copied_links, demanded);
        for capacity in [1, 8] {
            let mut capacities = kit::capacities();
            capacities.rows[11] = capacity;
            let direct = kit::direct(&memory, &capacities);
            let mut captured = Captured::with_capacities(&capacities);
            captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
            assert_eq!(captured.tables()[11], direct.tables()[11]);
            assert_eq!(captured.collaboration.outcome, direct.collaboration.outcome);
        }
        drop(memory);
        bytes.fill(0);
        let mut captured = Captured::with_capacities(&kit::capacities());
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        assert_eq!(captured.collaboration.outcome, Outcome::Rows { len: 2 });
        assert_eq!(&captured.collaboration.rows[..2], &rows[..2]);
    }
}

#[test]
fn collaboration_alias_translation_faults_refuse_only_their_hour_and_recover() {
    for alias in [OUTGOING_ALIAS, INCOMING_ALIAS] {
        let mut captured = Captured::with_capacities(&kit::capacities());
        let mut job = kit::job();
        for (hour, failed) in [(100, false), (101, true), (102, false)] {
            let mut bytes = alias_fixture(hour, true, true);
            let memory = Fault {
                bytes: &bytes,
                address: LINKS + alias as usize * 4,
                len: if failed { 4 } else { 0 },
            };
            let direct = kit::direct(&memory, &kit::capacities());
            assert!(
                copy_construction_rest_refresh(
                    &memory,
                    ConstructionRestCopyContext {
                        base: 0,
                        game_state: kit::BASE
                    },
                    &mut job,
                )
                .raw_ok,
                "{hour}"
            );
            assert_eq!(job.log.fault[9], failed, "{hour}");
            assert!(
                job.log
                    .spans
                    .iter()
                    .filter(|span| !span.ok && span.addr == LINKS + alias as usize * 4)
                    .all(|span| span.len == 4 && span.mask == 1 << 9 && !span.bulk)
            );
            bytes.fill(0);
            captured.reset();
            captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
            assert_eq!(captured.tables()[11], direct.tables()[11], "{hour}");
            if failed {
                assert_eq!(
                    captured.collaboration.outcome,
                    Outcome::Unavailable {
                        reason: "country link unreadable or outside country table",
                    },
                    "{hour}"
                );
            } else {
                assert_eq!(
                    captured.collaboration.outcome,
                    Outcome::Rows { len: 2 },
                    "{hour}"
                );
                assert_eq!(captured.collaboration.rows[0].target, OUTGOING_ALIAS);
                assert_eq!(captured.collaboration.rows[0].progress, hour);
            }
        }
    }
}

#[test]
fn collaboration_owned_uses_each_country_and_link_arrays_own_size() {
    for (countries, links) in [(4, 7), (6, 4)] {
        let mut bytes = fixture(9_123);
        array(
            &mut bytes,
            kit::BASE + cgame_state::COUNTRIES,
            0x2000,
            countries,
        );
        array(
            &mut bytes,
            kit::BASE + cgame_state::COUNTRY_LINKS,
            LINKS,
            links,
        );
        let memory = SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        assert_rows(&direct, 9_123);
        let mut job = kit::job();
        assert!(
            copy_construction_rest_refresh(
                &memory,
                ConstructionRestCopyContext {
                    base: 0,
                    game_state: kit::BASE
                },
                &mut job,
            )
            .raw_ok
        );
        bytes.fill(0);
        let mut captured = Captured::with_capacities(&kit::capacities());
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        assert_rows(&captured, 9_123);
    }
}

#[test]
fn collaboration_aliases_keep_country_status_and_reciprocal_authentication() {
    for (address, value, reason) in [
        (
            LINKS + OUTGOING_ALIAS as usize * 4,
            0_u32,
            "country link unreadable or outside country table",
        ),
        (
            LINKS + OUTGOING_ALIAS as usize * 4,
            4,
            "country link unreadable or outside country table",
        ),
        (
            LINKS + OUTGOING_ALIAS as usize * 4,
            1,
            "collaboration relation targets its source country",
        ),
        (
            LINKS + INCOMING_ALIAS as usize * 4,
            3,
            "collaboration outgoing relation lacks unique reciprocal incoming link",
        ),
        (TARGET + ccountry::ID, 0, "country id is zero"),
    ] {
        let mut bytes = alias_fixture(1, true, true);
        put(&mut bytes, address, &value.to_le_bytes());
        let memory = SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        let mut job = kit::job();
        assert!(
            copy_construction_rest_refresh(
                &memory,
                ConstructionRestCopyContext {
                    base: 0,
                    game_state: kit::BASE
                },
                &mut job,
            )
            .raw_ok
        );
        bytes.fill(0);
        let mut captured = Captured::with_capacities(&kit::capacities());
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        assert_eq!(captured.tables()[11], direct.tables()[11]);
        assert_eq!(
            captured.collaboration.outcome,
            Outcome::Unavailable { reason }
        );
    }
    for (address, value, reason) in [
        (TARGET, 0_u64, "country vtable mismatch"),
        (
            TARGET_STATUS,
            0,
            "collaboration status vtable mismatch or unreadable",
        ),
        (
            TARGET_STATUS + ccountry_collaboration_status::OWNER,
            kit::COUNTRY as u64,
            "collaboration status owner mismatch or unreadable",
        ),
        (
            SOURCE_RELATION,
            0,
            "collaboration relation vtable mismatch or unreadable",
        ),
    ] {
        let mut bytes = alias_fixture(1, true, true);
        put(&mut bytes, address, &value.to_le_bytes());
        let memory = SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        let mut job = kit::job();
        assert!(
            copy_construction_rest_refresh(
                &memory,
                ConstructionRestCopyContext {
                    base: 0,
                    game_state: kit::BASE
                },
                &mut job,
            )
            .raw_ok
        );
        bytes.fill(0);
        let mut captured = Captured::with_capacities(&kit::capacities());
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        assert_eq!(captured.tables()[11], direct.tables()[11]);
        assert_eq!(
            captured.collaboration.outcome,
            Outcome::Unavailable { reason }
        );
    }
}

#[test]
fn collaboration_owned_grows_to_fifteen_hundred_countries_with_outgoing_and_incoming_aliases() {
    const COUNTRIES: usize = 1_500;
    const COUNTRY_DATA: usize = 0x2000;
    const LINK_DATA: usize = 0x5000;
    const OBJECTS: usize = 0x10000;
    const STRIDE: usize = 0x2000;
    let mut bytes = vec![0; OBJECTS + COUNTRIES * STRIDE - kit::BASE];
    array(
        &mut bytes,
        kit::BASE + cgame_state::COUNTRIES,
        COUNTRY_DATA,
        COUNTRIES as u32 + 1,
    );
    array(
        &mut bytes,
        kit::BASE + cgame_state::COUNTRY_LINKS,
        LINK_DATA,
        COUNTRIES as u32 * 2 + 2,
    );
    let mut expected = Vec::new();
    for slot in 1..=COUNTRIES {
        let country = OBJECTS + (slot - 1) * STRIDE;
        let status = country + 0x1100;
        let outgoing = country + 0x1180;
        let incoming = country + 0x1190;
        let relation = country + 0x11a0;
        let next = slot % COUNTRIES + 1;
        let previous = if slot == 1 { COUNTRIES } else { slot - 1 };
        let target = (COUNTRIES + next) as u32;
        let progress = if slot % 2 == 0 {
            -(slot as i64)
        } else {
            slot as i64
        };
        put(
            &mut bytes,
            COUNTRY_DATA + slot * 8,
            &(country as u64).to_le_bytes(),
        );
        for raw in [slot, COUNTRIES + slot] {
            put(
                &mut bytes,
                LINK_DATA + raw * 4,
                &(slot as u32).to_le_bytes(),
            );
        }
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
        put(
            &mut bytes,
            country + ccountry::COLLABORATION_STATUS,
            &(status as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            status,
            &(ccountry_collaboration_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            status + ccountry_collaboration_status::OWNER,
            &(country as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            status + ccountry_collaboration_status::RELATIONS,
            outgoing,
            1,
        );
        array(
            &mut bytes,
            status + ccountry_collaboration_status::INCOMING_COUNTRIES,
            incoming,
            1,
        );
        put(&mut bytes, outgoing, &(relation as u64).to_le_bytes());
        put(
            &mut bytes,
            incoming,
            &((COUNTRIES + previous) as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            relation,
            &(ccountry_collaboration_data::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            relation + ccountry_collaboration_data::TARGET_COUNTRY,
            &target.to_le_bytes(),
        );
        put(
            &mut bytes,
            relation + ccountry_collaboration_data::PROGRESS,
            &progress.to_le_bytes(),
        );
        expected.push(ccountry_collaboration_data::Row {
            source: slot as u32,
            target,
            progress,
        });
    }
    let memory = SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let mut direct = Vec::new();
    assert_eq!(
        ccountry_collaboration_status::walk(&memory, 0, kit::BASE, &mut direct),
        Outcome::Rows { len: COUNTRIES }
    );
    assert_eq!(&direct[..COUNTRIES], &expected);
    let mut job = ConstructionRestJob::new();
    assert!(
        copy_construction_rest_refresh(
            &memory,
            ConstructionRestCopyContext {
                base: 0,
                game_state: kit::BASE
            },
            &mut job,
        )
        .raw_ok
    );
    assert!(!job.log.fault[9]);
    assert_eq!(job.census.len(), COUNTRIES);
    assert!(
        !job.log
            .spans
            .iter()
            .any(|span| span.addr == LINK_DATA + (COUNTRIES * 2 + 1) * 4)
    );
    bytes.fill(0);
    let mut capacities = kit::capacities();
    capacities.rows[11] = 0;
    let mut captured = Captured::with_capacities(&capacities);
    captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
    assert_eq!(
        captured.collaboration.outcome,
        Outcome::Rows { len: COUNTRIES }
    );
    assert_eq!(&captured.collaboration.rows[..COUNTRIES], &expected);
}

#[test]
fn collaboration_owned_captures_reserved_authentication_only_for_physical_zero_id_aliases() {
    let mut bytes = fixture(0);
    put(&mut bytes, 0x2000, &(kit::COUNTRY as u64).to_le_bytes());
    put(
        &mut bytes,
        kit::COUNTRY + ccountry::ID,
        &0_u32.to_le_bytes(),
    );
    array(
        &mut bytes,
        TARGET_STATUS + ccountry_collaboration_status::RELATIONS,
        0,
        0,
    );
    array(
        &mut bytes,
        TARGET_STATUS + ccountry_collaboration_status::INCOMING_COUNTRIES,
        0,
        0,
    );
    let memory = ExactLinks {
        bytes: &bytes,
        demanded: &[3],
        reads: RefCell::default(),
    };
    assert_eq!(
        ccountry_collaboration_status::walk(&memory, 0, kit::BASE, &mut Vec::new()),
        Outcome::Rows { len: 0 }
    );
    let decoder_reads = memory.reads.take();
    let mut job = kit::job();
    assert!(
        copy_construction_rest_refresh(
            &memory,
            ConstructionRestCopyContext {
                base: 0,
                game_state: kit::BASE
            },
            &mut job,
        )
        .raw_ok
    );
    assert_eq!(job.census.len(), 1);
    assert!(!job.log.fault[9]);
    for (address, len) in decoder_reads {
        assert!(
            job.log.spans.iter().any(|span| span.addr == address
                && span.len as usize == len
                && span.ok
                && !span.bulk),
            "uncaptured decoder read {address:#x}, {len}"
        );
    }
    assert!(
        !job.log
            .spans
            .iter()
            .any(|span| span.addr == kit::COUNTRY + ccountry::COLLABORATION_STATUS)
    );
    drop(memory);
    let mut captured = Captured::with_capacities(&kit::capacities());
    for failed in [false, true, false] {
        let memory = Fault {
            bytes: &bytes,
            address: 0x2000,
            len: if failed { 8 } else { 0 },
        };
        let mut direct = vec![];
        let expected = ccountry_collaboration_status::walk(&memory, 0, kit::BASE, &mut direct);
        assert!(
            copy_construction_rest_refresh(
                &memory,
                ConstructionRestCopyContext {
                    base: 0,
                    game_state: kit::BASE
                },
                &mut job,
            )
            .raw_ok
        );
        captured.reset();
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        assert_eq!(captured.collaboration.outcome, expected);
        assert_eq!(matches!(expected, Outcome::Unavailable { .. }), failed);
    }
    bytes.fill(0);
    captured.reset();
    captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
    assert_eq!(captured.collaboration.outcome, Outcome::Rows { len: 0 });
}
