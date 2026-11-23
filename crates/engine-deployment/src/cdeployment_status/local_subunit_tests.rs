use super::*;
use engine_native_memory::memory::SliceMemory;

struct Fixture(Vec<u8>);
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == csub_unit_definition::DATABASE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x9800_u64.to_le_bytes());
            return true;
        }
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
    fn array(&mut self, at: usize, data: u64, count: u32) {
        self.word(at, data);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
    fn subunit(&mut self, at: usize, token: u32, requirements: &[(u64, i64)]) {
        self.word(at, csub_unit_definition::VTABLE_RVA.0 as u64);
        self.put(at + csub_unit_definition::TOKEN, &token.to_le_bytes());
        self.0[at + csub_unit_definition::IS_REAL] = 1;
        self.put(
            at + csub_unit_definition::DATABASE_INDEX,
            &1_i32.to_le_bytes(),
        );
        let data = at + 0x700;
        self.array(
            at + csub_unit_definition::ARCHETYPE_REQUIREMENTS_DATA,
            data as u64,
            requirements.len() as u32,
        );
        for (index, (archetype, amount)) in requirements.iter().enumerate() {
            self.word(data + index * 16, *archetype);
            self.put(data + index * 16 + 8, &amount.to_le_bytes());
        }
    }
}

const ARCHETYPE: u64 = 0xa000;
const OTHER: u64 = 0xa100;

fn fixture() -> Fixture {
    let mut memory = Fixture(vec![0; 0xc000]);
    memory.array(0x1000 + engine_native::cgame_state::COUNTRIES, 0x2000, 2);
    memory.array(0x1000 + engine_native::cgame_state::COUNTRY_LINKS, 0x2100, 2);
    memory.word(0x2008, 0x3000);
    memory.put(0x2104, &1_u32.to_le_bytes());
    memory.word(0x3000, ccountry::VTABLE_RVA.0 as u64);
    memory.put(0x3000 + ccountry::ID, &1_u32.to_le_bytes());
    memory.word(0x3000 + ccountry::DEPLOYMENT_MANAGER, 0x3800);
    memory.word(0x3800, VTABLE_RVA.0 as u64);
    memory.word(0x3800 + OWNER, 0x3000);
    memory.array(0x3800 + LOCAL_SUBUNITS, 0x3a00, 2);
    memory.word(0x3a00, 0x5000);
    memory.word(0x3a08, 0x6000);
    memory.subunit(0x4000, 5, &[(ARCHETYPE, 100_000), (OTHER, 200_000)]);
    memory.subunit(0x5000, 5, &[(ARCHETYPE, 100_000), (OTHER, 200_000)]);
    memory.subunit(0x6000, 5, &[(ARCHETYPE, 300_000)]);
    memory.array(0x9800 + csub_unit_definition::DATABASE_VECTOR, 0x9900, 2);
    memory.word(0x9908, 0x4000);
    for (at, token) in [(ARCHETYPE as usize, 71_u32), (OTHER as usize, 72)] {
        memory.word(at, engine_native_parts::cequipment_type::VTABLE_RVA.0 as u64);
        memory.put(at + engine_native_parts::cequipment_type::TOKEN, &token.to_le_bytes());
        memory.0[at + engine_native_parts::cequipment_type::ACTIVE] = 1;
    }
    memory
}

#[test]
fn only_clones_that_differ_from_their_global_definition_are_rows() {
    let memory = fixture();
    let mut rows = vec![LocalSubunitRow::default(); 2];
    let mut requirements = vec![LocalRequirement::default(); 4];
    let walk = walk_local_subunits(&memory, 0, 0x1000, &mut rows, &mut requirements);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!((rows[0].country, rows[0].token), (1, 5));
    assert_eq!(
        &requirements[..walk.requirements_len],
        &[LocalRequirement {
            archetype_token: 71,
            amount_raw: 300_000
        }]
    );
    assert_eq!((walk.required_rows, walk.required_requirements), (1, 1));
}

#[test]
fn a_country_with_two_differing_clones_of_one_token_is_refused() {
    let mut memory = fixture();
    memory.subunit(0x5000, 5, &[(OTHER, 1)]);
    assert_eq!(
        walk_local_subunits(
            &memory,
            0,
            0x1000,
            &mut vec![LocalSubunitRow::default(); 2],
            &mut vec![LocalRequirement::default(); 4]
        )
        .outcome,
        Outcome::Unavailable {
            reason: "local subunit token repeats within a country"
        }
    );
}

#[test]
fn a_clone_that_is_its_global_definition_or_names_no_entry_is_refused() {
    let mut memory = fixture();
    memory.word(0x3a08, 0x4000);
    let outcome = walk_local_subunits(&memory, 0, 0x1000, &mut Vec::new(), &mut Vec::new()).outcome;
    assert_eq!(
        outcome,
        Outcome::Unavailable {
            reason: "local subunit is its global definition"
        }
    );
    let mut memory = fixture();
    memory.put(
        0x6000 + csub_unit_definition::DATABASE_INDEX,
        &9_i32.to_le_bytes(),
    );
    assert!(matches!(
        walk_local_subunits(&memory, 0, 0x1000, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn growing_local_clone_rows_reserve_the_original_changed_requirement_header() {
    let mut memory = fixture();
    let (mut rows, mut requirements) = (Vec::new(), Vec::new());
    let first = walk_local_subunits(&memory, 0, 0x1000, &mut rows, &mut requirements);
    assert_eq!(first.outcome, Outcome::Rows { len: 1 });
    memory.subunit(0x6000, 5, &vec![(ARCHETYPE, 300_000); 512]);
    let later = walk_local_subunits(&memory, 0, 0x1000, &mut rows, &mut requirements);
    assert_eq!(later.outcome, Outcome::Rows { len: 1 });
    assert_eq!(later.requirements_len, 512);
    assert_eq!(requirements[511].amount_raw, 300_000);
}

fn held_row() -> (LocalSubunitRow, LocalRequirement) {
    (
        LocalSubunitRow {
            country: 1,
            token: 9,
            requirements: engine_native::creferenced_division_template::Span { start: 0, len: 1 },
        },
        LocalRequirement {
            archetype_token: 72,
            amount_raw: 7,
        },
    )
}

#[test]
fn selected_walks_carry_confirm_or_read_each_country() {
    let mut memory = fixture();
    let (row, requirement) = held_row();
    let held = HeldLocalSubunits {
        rows: &[row],
        requirements: &[requirement],
    };
    let walk = |memory: &Fixture, selection: LocalSelection<'_>| {
        let (mut rows, mut requirements, mut visits) = (Vec::new(), Vec::new(), Vec::new());
        let walk = walk_local_subunits_selected(
            memory,
            0,
            0x1000,
            &mut rows,
            &mut requirements,
            |_| selection,
            |visit| visits.push(visit),
        );
        assert_eq!(
            walk.outcome,
            Outcome::Rows {
                len: rows.len().min(1)
            }
        );
        (rows, requirements, visits)
    };
    let (rows, _, visits) = walk(&memory, LocalSelection::Read);
    assert_eq!(rows[0].token, 5);
    let current = visits[0].deployment.unwrap();
    assert_eq!(
        current,
        Deployment {
            status: 0x3800,
            clones: Array {
                data: 0x3a00,
                count: 2
            }
        }
    );
    assert!(visits[0].read);
    let (rows, requirements, visits) = walk(
        &memory,
        LocalSelection::Confirm {
            deployment: current,
            held,
        },
    );
    assert_eq!((rows[0].token, requirements[0].amount_raw), (9, 7));
    assert!(!visits[0].read && visits[0].rows == (0..1));
    memory.array(0x3800 + LOCAL_SUBUNITS, 0x3a00, 1);
    let (rows, _, visits) = walk(
        &memory,
        LocalSelection::Confirm {
            deployment: current,
            held,
        },
    );
    assert!(rows.is_empty() && visits[0].read);
    memory.word(0x3000 + ccountry::DEPLOYMENT_MANAGER, 0);
    let (rows, _, visits) = walk(&memory, LocalSelection::Carry(held));
    assert_eq!(rows[0].token, 9);
    assert!(visits[0].deployment.is_none() && !visits[0].read);
}
