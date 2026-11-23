use super::{MapCapacities, MapCaptured, PlayersCopyContext, PlayersJob, copy_players_refresh};
use engine_native::{Memory, ccountry, cgame_state, memory::SliceMemory, walk::Outcome};
use engine_native_politics::chuman;
use std::cell::RefCell;

const GS: usize = 0x10000;
const ROSTER: usize = 0x20000;

struct World {
    bytes: Vec<u8>,
    countries: usize,
    counts: usize,
    heap: usize,
    user_len: usize,
}

impl World {
    fn new(country_count: u32, player_count: u32, user_len: usize) -> Self {
        let slots = country_count as usize + 1;
        let countries = ROSTER + player_count as usize * chuman::STRIDE;
        let links = countries + slots * 8;
        let counts = links + slots * 4;
        let objects = (counts + slots + 7) & !7;
        let heap = objects + slots * 16;
        let mut world = Self {
            bytes: vec![0; heap - GS + player_count as usize * (user_len + 20)],
            countries,
            counts,
            heap,
            user_len,
        };
        world.array(GS + cgame_state::PLAYER_RECORDS, ROSTER, player_count);
        world.array(GS + cgame_state::COUNTRIES, countries, country_count + 1);
        world.array(GS + cgame_state::COUNTRY_LINKS, links, country_count + 1);
        world.array(GS + cgame_state::HUMAN_STATUS, counts, country_count + 1);
        for slot in 1..=country_count {
            let pointer = objects + slot as usize * 16;
            world.put(
                countries + slot as usize * 8,
                &(pointer as u64).to_le_bytes(),
            );
            world.put(links + slot as usize * 4, &slot.to_le_bytes());
            world.put(pointer, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
            world.put(pointer + ccountry::ID, &slot.to_le_bytes());
            world.put(counts + slot as usize, &[slot as u8]);
        }
        for index in 0..player_count {
            let address = ROSTER + index as usize * chuman::STRIDE;
            let text = heap + index as usize * (user_len + 20);
            world.put(address, &(chuman::VTABLE_RVA.0 as u64).to_le_bytes());
            world.put(address + chuman::PLAYER_KEY, &(index as i32).to_le_bytes());
            world.put(
                address + chuman::COUNTRY_TAG,
                &(index % country_count + 1).to_le_bytes(),
            );
            world.put(address + chuman::FLAGS, &[index as u8]);
            world.string(address + chuman::FIRST_STRING, text, user_len, index as u8);
            world.string(
                address + chuman::NAME,
                text + user_len + 1,
                18,
                (index + 1) as u8,
            );
        }
        world
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: GS,
            bytes: &self.bytes,
        }
    }

    fn put(&mut self, address: usize, value: &[u8]) {
        self.bytes[address - GS..address - GS + value.len()].copy_from_slice(value);
    }

    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }

    fn string(&mut self, address: usize, source: usize, len: usize, value: u8) {
        assert!(len > 15);
        self.put(address, &(source as u64).to_le_bytes());
        self.put(address + 16, &(len as u64).to_le_bytes());
        self.put(address + 24, &(len as u64).to_le_bytes());
        self.bytes[source - GS..source - GS + len].fill(value);
    }
}

struct TraceMemory<'a> {
    memory: SliceMemory<'a>,
    fail: Option<(usize, usize)>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for TraceMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        self.fail != Some((address, output.len())) && self.memory.copy(address, output)
    }
}

fn capture(memory: &impl Memory, job: &mut PlayersJob, captured: &mut MapCaptured) {
    let report =
    // SAFETY: This is the admitted hour's roster request and its authenticated image.
    unsafe { copy_players_refresh(
        memory,
        PlayersCopyContext {
            base: 0,
            game_state: GS,
        },
        job,
    ) };
    assert!(report.raw_ok);
    // SAFETY: Decode is settling the matching job into its originating frame.
    unsafe { captured.decode_players_payload(0, job, &mut [0; 2]) };
}

#[test]
fn player_raw_fault_prefix_matches_direct_and_keeps_independent_controllers() {
    let world = World::new(2, 2, 29);
    let first = ROSTER;
    let name_source = world.heap + world.user_len + 1;
    for failure in [
        (GS + cgame_state::PLAYER_RECORDS, 16),
        (first, 8),
        (first + chuman::PLAYER_KEY, 4),
        (first + chuman::COUNTRY_TAG, 4),
        (first + chuman::FLAGS, 1),
        (first + chuman::FIRST_STRING, 64),
        (world.heap + world.user_len, 1),
        (name_source + 18, 1),
        (world.heap, world.user_len),
        (name_source, 18),
    ] {
        let direct_memory = TraceMemory {
            memory: world.memory(),
            fail: Some(failure),
            reads: RefCell::new(Vec::new()),
        };
        let direct = chuman::walk(
            &direct_memory,
            0,
            GS,
            &mut vec![chuman::Row::default(); 2],
            &mut vec![0; 128],
        );
        let raw_memory = TraceMemory {
            memory: world.memory(),
            fail: Some(failure),
            reads: RefCell::new(Vec::new()),
        };
        let mut job = PlayersJob::new();
        let mut captured =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&MapCapacities::default()) };
        capture(&raw_memory, &mut job, &mut captured);
        assert_eq!(
            captured.players.outcome, direct.outcome,
            "failure={failure:?}"
        );
        assert_eq!(captured.human_controllers.outcome, Outcome::Rows { len: 2 });
        let raw_reads = raw_memory.reads.borrow();
        let controller_start = raw_reads
            .iter()
            .position(|&(address, _)| address == GS + cgame_state::HUMAN_STATUS)
            .expect("independent controller pass");
        assert_eq!(
            &raw_reads[..controller_start],
            *direct_memory.reads.borrow(),
            "failure={failure:?}"
        );
        assert_eq!(
            raw_reads.iter().filter(|&&read| read == failure).count(),
            1,
            "failure={failure:?}"
        );
    }
}

#[test]
fn both_player_terminators_precede_payload_and_semantic_gates_stop_later_players() {
    let mut world = World::new(2, 2, 29);
    let memory = TraceMemory {
        memory: world.memory(),
        fail: None,
        reads: RefCell::new(Vec::new()),
    };
    let mut job = PlayersJob::new();
    let mut captured =
    // SAFETY: These are session-measured map capacities or capacities owned by an isolated
    // fixture.
    unsafe { MapCaptured::with_capacities(&MapCapacities::default()) };
    capture(&memory, &mut job, &mut captured);
    let reads = memory.reads.borrow();
    let user_terminator = reads
        .iter()
        .position(|&read| read == (world.heap + world.user_len, 1))
        .unwrap();
    let name_terminator = reads
        .iter()
        .position(|&read| read == (world.heap + world.user_len + 1 + 18, 1))
        .unwrap();
    let user_payload = reads
        .iter()
        .position(|&read| read == (world.heap, world.user_len))
        .unwrap();
    assert!(user_terminator < name_terminator && name_terminator < user_payload);
    drop(reads);
    drop(memory);
    for invalid in [0_u64, chuman::VTABLE_RVA.0 as u64] {
        world.put(ROSTER, &invalid.to_le_bytes());
        world.put(ROSTER + chuman::PLAYER_KEY, &(-1_i32).to_le_bytes());
        let memory = TraceMemory {
            memory: world.memory(),
            fail: None,
            reads: RefCell::new(Vec::new()),
        };
        capture(&memory, &mut job, &mut captured);
        assert!(matches!(
            captured.players.outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.human_controllers.outcome, Outcome::Rows { len: 2 });
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == ROSTER + chuman::STRIDE)
        );
    }
}

#[test]
fn raw_controller_walk_keeps_first_fault_and_original_count_bounds() {
    let mut world = World::new(5_000, 1, 29);
    world.put(
        GS + cgame_state::HUMAN_STATUS + 12,
        &5_000_u32.to_le_bytes(),
    );
    let memory = TraceMemory {
        memory: world.memory(),
        fail: None,
        reads: RefCell::new(Vec::new()),
    };
    let mut job = PlayersJob::new();
    let mut captured =
    // SAFETY: These are session-measured map capacities or capacities owned by an isolated
    // fixture.
    unsafe { MapCaptured::with_capacities(&MapCapacities::default()) };
    capture(&memory, &mut job, &mut captured);
    assert_eq!(
        captured.human_controllers.outcome,
        Outcome::Unavailable {
            reason: "human controller count unreadable or outside array"
        }
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| address == world.counts + 5_000)
    );
    drop(memory);
    world.put(
        GS + cgame_state::HUMAN_STATUS + 12,
        &5_001_u32.to_le_bytes(),
    );
    for failure in [(world.countries + 8, 8), (world.counts + 1, 1)] {
        let memory = TraceMemory {
            memory: world.memory(),
            fail: Some(failure),
            reads: RefCell::new(Vec::new()),
        };
        capture(&memory, &mut job, &mut captured);
        assert!(matches!(
            captured.human_controllers.outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.players.outcome, Outcome::Rows { len: 1 });
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == world.countries + 16)
        );
    }
}

#[test]
fn raw_controllers_preserve_reserved_physical_aliases() {
    let mut world = World::new(3, 1, 29);
    let first_pointer = world.counts + 4;
    let reserved = (first_pointer + 7) & !7;
    world.put(world.countries, &(reserved as u64).to_le_bytes());
    world.put(world.countries + 8, &(reserved as u64).to_le_bytes());
    world.put(reserved, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    world.put(reserved + ccountry::ID, &0_u32.to_le_bytes());
    let mut job = PlayersJob::new();
    let mut captured =
    // SAFETY: These are session-measured map capacities or capacities owned by an isolated
    // fixture.
    unsafe { MapCaptured::with_capacities(&MapCapacities::default()) };
    capture(&world.memory(), &mut job, &mut captured);
    assert_eq!(captured.human_controllers.outcome, Outcome::Rows { len: 2 });
    assert_eq!(captured.human_controllers.rows[0].country, 2);
    assert_eq!(captured.human_controllers.rows[1].country, 3);
}
