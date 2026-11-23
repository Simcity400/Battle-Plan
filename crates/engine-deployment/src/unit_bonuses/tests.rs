use super::*;
use engine_native_memory::memory::SliceMemory;

struct Fixture(Vec<u8>);
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(address, output)
    }
}
impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data as u64);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
    fn entry(&mut self, at: usize, kind: u32, id: u32, number: i32) {
        self.word(at, VTABLE_RVA.0 as u64);
        self.word(at + PAYLOAD, PAYLOAD_VTABLE_RVA.0 as u64);
        self.put(at + SOURCE_KIND, &kind.to_le_bytes());
        self.put(at + SOURCE_ID, &id.to_le_bytes());
        self.put(at + SOURCE_NUMBER, &number.to_le_bytes());
    }
    fn label(&mut self, at: usize, text: &[u8], heap: Option<usize>) {
        let string = at + LABEL;
        let capacity = match heap {
            Some(source) => {
                self.put(source, text);
                self.word(string, source as u64);
                31
            }
            None => {
                self.put(string, text);
                msvc_string::INLINE_CAPACITY
            }
        };
        self.word(string + msvc_string::LENGTH, text.len() as u64);
        self.word(string + msvc_string::CAPACITY, capacity);
    }
    fn definition(&mut self, at: usize, token: u32, width: i64, stats: &[(usize, i64)]) {
        self.word(at, csub_unit_definition::VTABLE_RVA.0 as u64);
        self.put(at + csub_unit_definition::TOKEN, &token.to_le_bytes());
        self.put(
            at + csub_unit_definition::COMBAT_WIDTH,
            &width.to_le_bytes(),
        );
        for &(selector, value) in stats {
            self.put(at + STATS + selector * 8, &value.to_le_bytes());
        }
    }
}

const COUNTRY: usize = 0x3000;
const DEPLOYMENT: usize = 0x3800;
const FIRST: usize = 0x4000;
const SECOND: usize = FIRST + STRIDE;
const EXACT: usize = 0x5000;
const BY_CATEGORY: usize = 0x5800;
const GRANT: usize = 0x5400;
const GRANT_VALUES: usize = 0x5500;
const TARGET_CATEGORY: usize = 0x6000;
const RECIPIENT: usize = 0x6100;
const HEAP_LABEL: &[u8] = b"a label on the heap.";

fn fixture() -> Fixture {
    let mut memory = Fixture(vec![0; 0x7000]);
    memory.array(0x1000 + engine_native::cgame_state::COUNTRIES, 0x2000, 2);
    memory.array(0x1000 + engine_native::cgame_state::COUNTRY_LINKS, 0x2100, 2);
    memory.word(0x2008, COUNTRY as u64);
    memory.put(0x2104, &1_u32.to_le_bytes());
    memory.word(COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    memory.put(COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    memory.word(COUNTRY + ccountry::DEPLOYMENT_MANAGER, DEPLOYMENT as u64);
    memory.word(DEPLOYMENT, cdeployment_status::VTABLE_RVA.0 as u64);
    memory.word(DEPLOYMENT + cdeployment_status::OWNER, COUNTRY as u64);
    memory.array(DEPLOYMENT + cdeployment_status::UNIT_BONUSES, FIRST, 2);
    memory.entry(FIRST, 2, 0x11, -3);
    memory.label(FIRST, b"named", None);
    memory.array(FIRST + PAYLOAD + EXACT_TARGETS, 0x4200, 1);
    memory.word(0x4200, EXACT as u64);
    memory.array(FIRST + PAYLOAD + CATEGORY_TARGETS, 0x4300, 1);
    memory.word(0x4300, TARGET_CATEGORY as u64);
    memory.word(0x4300 + CATEGORY_TARGET_BONUS, BY_CATEGORY as u64);
    memory.entry(SECOND, 5, 0x22, 1);
    memory.label(SECOND, HEAP_LABEL, Some(0x4400));
    memory.definition(EXACT, 41, -50_000, &[(0, 25_000), (77, -1)]);
    memory.array(EXACT + BATTALION_GRANTS, GRANT, 1);
    memory.word(GRANT + GRANT_CATEGORY, RECIPIENT as u64);
    memory.array(GRANT + GRANT_STATS, GRANT_VALUES, 2);
    memory.0[GRANT + GRANT_ADD] = 1;
    memory.put(GRANT_VALUES, &3_u32.to_le_bytes());
    memory.put(GRANT_VALUES + GRANT_STAT_AMOUNT, &10_000_i64.to_le_bytes());
    memory.put(GRANT_VALUES + GRANT_STAT_STRIDE, &3_u32.to_le_bytes());
    memory.definition(BY_CATEGORY, 99, 0, &[(5, 7)]);
    memory.put(TARGET_CATEGORY + CATEGORY_TOKEN, &61_u32.to_le_bytes());
    memory.put(RECIPIENT + CATEGORY_TOKEN, &62_u32.to_le_bytes());
    memory
}

struct Captured {
    rows: Vec<Row>,
    targets: Vec<Target>,
    grants: Vec<Grant>,
    stats: Vec<Stat>,
    labels: Vec<u8>,
}

fn capture(memory: &Fixture) -> (Walk, Captured) {
    let mut captured = Captured {
        rows: Vec::new(),
        targets: Vec::new(),
        grants: Vec::new(),
        stats: Vec::new(),
        labels: Vec::new(),
    };
    let walk = walk(
        memory,
        0,
        0x1000,
        Buffers::growing(
            &mut captured.rows,
            &mut captured.targets,
            &mut captured.grants,
            &mut captured.stats,
            &mut captured.labels,
        ),
    );
    (walk, captured)
}

const DEMAND: Counts = Counts {
    rows: 2,
    targets: 2,
    grants: 1,
    stats: 5,
    label_bytes: 5 + HEAP_LABEL.len(),
};

#[test]
fn entries_keep_their_source_targets_grants_and_labels_in_stored_order() {
    let (walk, captured) = capture(&fixture());
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!((walk.written, walk.required), (DEMAND, DEMAND));
    assert_eq!(
        &captured.rows[..2],
        &[
            Row {
                country: 1,
                ordinal: 0,
                source_kind: 2,
                source_id: 0x11,
                source_number: -3,
                label: Span { start: 0, len: 5 },
                targets: Span { start: 0, len: 2 },
            },
            Row {
                country: 1,
                ordinal: 1,
                source_kind: 5,
                source_id: 0x22,
                source_number: 1,
                label: Span {
                    start: 5,
                    len: HEAP_LABEL.len()
                },
                targets: Span { start: 2, len: 0 },
            },
        ]
    );
    assert_eq!(&captured.labels[..5], b"named");
    assert_eq!(&captured.labels[5..DEMAND.label_bytes], HEAP_LABEL);
    assert_eq!(
        &captured.targets[..2],
        &[
            Target {
                category: false,
                token: 41,
                combat_width_raw: -50_000,
                stats: Span { start: 0, len: 2 },
                grants: Span { start: 0, len: 1 },
            },
            Target {
                category: true,
                token: 61,
                combat_width_raw: 0,
                stats: Span { start: 4, len: 1 },
                grants: Span { start: 1, len: 0 },
            },
        ]
    );
    assert_eq!(
        captured.grants[0],
        Grant {
            category_token: 62,
            add: 1,
            display_as_percentage: 0,
            stats: Span { start: 2, len: 2 },
        }
    );
    let stat = |selector, value_raw| Stat {
        selector,
        value_raw,
    };
    assert_eq!(
        &captured.stats[..5],
        &[
            stat(0, 25_000),
            stat(77, -1),
            stat(3, 10_000),
            stat(3, 0),
            stat(5, 7)
        ]
    );
}

#[test]
fn the_demand_probe_counts_every_entry() {
    let memory = fixture();
    let demand = walk_demand(&memory);
    assert_eq!(
        demand.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(demand.required, DEMAND);
}

fn walk_demand(memory: &Fixture) -> Walk {
    walk(memory, 0, 0x1000, Buffers::demand())
}

#[test]
fn growing_outputs_follow_a_longer_list_and_larger_payloads() {
    let mut memory = fixture();
    let (_, mut captured) = capture(&memory);
    memory.array(DEPLOYMENT + cdeployment_status::UNIT_BONUSES, FIRST, 3);
    let third = SECOND + STRIDE;
    memory.entry(third, 0, 0, 0);
    memory.label(third, b"", None);
    memory.array(third + PAYLOAD + EXACT_TARGETS, 0x4200, 1);
    memory.array(GRANT + GRANT_STATS, GRANT_VALUES, 40);
    let walk = walk(
        &memory,
        0,
        0x1000,
        Buffers::growing(
            &mut captured.rows,
            &mut captured.targets,
            &mut captured.grants,
            &mut captured.stats,
            &mut captured.labels,
        ),
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!(walk.written, walk.required);
    assert_eq!(
        walk.written,
        Counts {
            rows: 3,
            targets: 3,
            grants: 2,
            stats: 2 + 40 + 1 + 2 + 40,
            label_bytes: DEMAND.label_bytes,
        }
    );
    assert_eq!(captured.targets[2].token, 41);
    assert_eq!(captured.rows[2].label.len, 0);
}

#[test]
fn foreign_objects_and_unknown_operands_refuse_the_walk_by_name() {
    let refused = |change: fn(&mut Fixture)| {
        let mut memory = fixture();
        change(&mut memory);
        match capture(&memory).0.outcome {
            Outcome::Unavailable { reason } => reason,
            outcome => panic!("{outcome:?}"),
        }
    };
    assert_eq!(
        refused(|memory| memory.word(SECOND, 0)),
        "unit bonus entry identity mismatch"
    );
    assert_eq!(
        refused(|memory| memory.word(FIRST + PAYLOAD, 0)),
        "unit bonus payload identity mismatch"
    );
    assert_eq!(
        refused(|memory| memory.word(BY_CATEGORY, 0)),
        "unit bonus definition identity mismatch"
    );
    assert_eq!(
        refused(|memory| memory.put(SECOND + SOURCE_KIND, &SOURCE_KINDS.to_le_bytes())),
        "unit bonus source kind unknown"
    );
    assert_eq!(
        refused(|memory| memory.put(
            GRANT_VALUES,
            &(cequipment_variant::STAT_COUNT as u32).to_le_bytes()
        )),
        "unit bonus battalion grant stat selector outside the stat block"
    );
    assert_eq!(
        refused(|memory| memory.word(DEPLOYMENT + cdeployment_status::OWNER, 0)),
        "deployment status country mismatch"
    );
    assert_eq!(
        refused(|memory| memory.put(TARGET_CATEGORY + CATEGORY_TOKEN, &0_u32.to_le_bytes())),
        "sub-unit category token unreadable or null"
    );
}
