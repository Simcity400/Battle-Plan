use super::{construction_rest_test_kit as kit, *};
use engine_native::memory::SliceMemory;

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
        &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
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
        &(ccountry_collaboration_status::VTABLE_RVA as u64).to_le_bytes(),
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
            &(ccountry_collaboration_data::VTABLE_RVA as u64).to_le_bytes(),
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
            let report = if capacity == 0 {
                copy_construction_rest_refresh_growing(&memory, ctx, &mut job)
            } else {
                copy_construction_rest_refresh(&memory, ctx, &mut job)
            };
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
            if capacity == 1 {
                let expected = kit::direct(&memory, &capacities);
                assert_eq!(captured.tables()[11], expected.tables()[11]);
                assert_eq!(
                    captured.collaboration.outcome,
                    Outcome::Truncated {
                        len: 1,
                        unread_rows: 1
                    }
                );
            } else {
                assert_rows(&captured, progress);
                assert_eq!(captured.tables()[11], direct.tables()[11]);
            }
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
        ccountry::VTABLE_RVA,
        0,
        ccountry_collaboration_data::VTABLE_RVA,
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
        if vtable == ccountry_collaboration_data::VTABLE_RVA {
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
