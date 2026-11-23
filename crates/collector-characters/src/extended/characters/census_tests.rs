use super::*;
use engine_native::cgame_state;
use engine_native_parts::{character_advisor_node, cunit_leader_trait};
use engine_native_memory::msvc_tree_node;
use engine_native_politics::character_leader_node;
use std::cell::Cell;

pub(super) const MODULE: usize = 0x7ff6_0000_0000;
pub(super) const GAME_STATE: usize = 0x1_0000_0000;
const MANAGER: usize = 0x1_0001_0000;
pub(super) const HISTORICAL: usize = 0x2_0000_0000;
pub(super) const DYNAMIC: usize = 0x3_0000_0000;
const CHARACTER: usize = 0x4_0000_0000;
const LEADER_HEAD: usize = 0x5_0000_0000;
const LEADER_NODE: usize = 0x6_0000_0000;
const LEADER_ROLE: usize = 0x7_0000_0000;
const ADVISOR_HEAD: usize = 0x8_0000_0000;
const ADVISOR_NODE: usize = 0x9_0000_0000;
const ADVISOR_ROLE: usize = 0xa_0000_0000;
pub(super) const UNIT: usize = 0xb_0000_0000;
pub(super) const TRAITS: usize = 0xc_0000_0000;
const EXPERIENCE: usize = 0xd_0000_0000;
pub(super) const DEFINITION: usize = 0xe_0000_0000;

pub(super) const STRIDES: [(usize, usize); 11] = [
    (CHARACTER, 0x100),
    (LEADER_HEAD, 0x40),
    (LEADER_NODE, 0x40),
    (LEADER_ROLE, 0x200),
    (ADVISOR_HEAD, 0x40),
    (ADVISOR_NODE, 0x60),
    (ADVISOR_ROLE, 0x400),
    (UNIT, 0x2000),
    (TRAITS, 0x40),
    (EXPERIENCE, 0x100),
    (DEFINITION, 0x1000),
];

#[derive(Clone)]
pub(super) struct World {
    pub characters: usize,
    pub historical: usize,
    pub definitions: usize,
    pub roster: Option<Vec<usize>>,
    pub patches: Vec<(usize, Vec<u8>)>,
    pub faults: Vec<usize>,
    pub dual: bool,
}

impl World {
    pub fn new(characters: usize) -> Self {
        Self {
            characters,
            historical: characters - characters / 7,
            definitions: 300,
            roster: None,
            patches: Vec::new(),
            faults: Vec::new(),
            dual: false,
        }
    }

    pub fn countries(countries: usize) -> Self {
        Self::new(countries * 31)
    }

    pub fn character(index: usize) -> usize {
        CHARACTER + index * 0x100
    }

    pub fn leader(index: usize) -> bool {
        index.is_multiple_of(5)
    }

    pub fn advisor(&self, index: usize) -> bool {
        index % 40 == 3 || (self.dual && index.is_multiple_of(40))
    }

    pub fn unit(index: usize) -> Option<usize> {
        (index % 4 == 1).then_some(UNIT + index * 0x2000)
    }

    pub fn leader_head(index: usize) -> usize {
        LEADER_HEAD + index * 0x40
    }

    pub fn leader_role(index: usize) -> usize {
        LEADER_ROLE + index * 0x200
    }

    pub fn advisor_head(index: usize) -> usize {
        ADVISOR_HEAD + index * 0x40
    }

    fn traits(index: usize) -> usize {
        1 + (index / 4) % 2
    }

    pub(super) fn factors(index: usize) -> usize {
        EXPERIENCE + index * 0x100 + 0x80
    }

    fn experience(index: usize) -> usize {
        4 + index % 2
    }

    fn slots(&self) -> usize {
        self.roster.as_ref().map_or(self.characters, Vec::len)
    }

    fn slot(&self, slot: usize) -> usize {
        self.roster.as_ref().map_or(slot, |roster| roster[slot])
    }

    pub fn patch(&mut self, address: usize, bytes: &[u8]) {
        self.patches.push((address, bytes.to_vec()));
    }

    fn render(&self, address: usize, out: &mut [u8]) -> bool {
        out.fill(0);
        let Some(end) = address.checked_add(out.len()) else {
            return false;
        };
        let mut window = Window {
            start: address,
            out,
        };
        if (GAME_STATE..GAME_STATE + 0x1000).contains(&address) {
            if end > GAME_STATE + 0x1000 {
                return false;
            }
            window.pointer(GAME_STATE + cgame_state::CHARACTER_MANAGER, MANAGER);
            return true;
        }
        if (MANAGER..MANAGER + 0x100).contains(&address) {
            if end > MANAGER + 0x100 {
                return false;
            }
            window.put(
                MANAGER,
                &((MODULE + ccharacter_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            let historical = self.historical.min(self.slots());
            window.array(
                MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
                HISTORICAL,
                historical,
            );
            window.array(
                MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
                DYNAMIC,
                self.slots() - historical,
            );
            return true;
        }
        let historical = self.historical.min(self.slots());
        for (region, first, count) in [
            (HISTORICAL, 0, historical),
            (DYNAMIC, historical, self.slots() - historical),
        ] {
            if address >> 32 == region >> 32 {
                if end > region + count * 8 {
                    return false;
                }
                for slot in (address - region) / 8..(end - region).div_ceil(8) {
                    window.pointer(region + slot * 8, Self::character(self.slot(first + slot)));
                }
                return true;
            }
        }
        let Some(&(region, stride)) = STRIDES
            .iter()
            .find(|(region, _)| address >> 32 == region >> 32)
        else {
            return false;
        };
        let index = (address - region) / stride;
        let object = region + index * stride;
        if end > object + stride {
            return false;
        }
        match region {
            _ if region != DEFINITION && index >= self.characters => return false,
            CHARACTER => self.render_character(&mut window, index, object),
            LEADER_HEAD => {
                let target = if Self::leader(index) {
                    LEADER_NODE + index * 0x40
                } else {
                    object
                };
                window.node(object, target, 1);
            }
            LEADER_NODE if Self::leader(index) => {
                window.node(object, Self::leader_head(index), 0);
                window.put(
                    object + character_leader_node::IDEOLOGY,
                    &((index % 7) as i32).to_le_bytes(),
                );
                window.pointer(
                    object + character_leader_node::LEADER,
                    LEADER_ROLE + index * 0x200,
                );
            }
            LEADER_ROLE if Self::leader(index) => {
                window.put(
                    object,
                    &((MODULE + engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                window.pointer(object + engine_native_parts::ccountry_leader_layout::CHARACTER, Self::character(index));
            }
            ADVISOR_HEAD => {
                let target = if self.advisor(index) {
                    ADVISOR_NODE + index * 0x60
                } else {
                    object
                };
                window.node(object, target, 1);
            }
            ADVISOR_NODE if self.advisor(index) => {
                window.node(object, Self::advisor_head(index), 0);
                window.literal(object + character_advisor_node::KEY, b"advisor_key");
                window.put(
                    object + character_advisor_node::KEY_HASH,
                    &(index as u32).to_le_bytes(),
                );
                window.pointer(
                    object + character_advisor_node::ADVISOR,
                    ADVISOR_ROLE + index * 0x400,
                );
            }
            ADVISOR_ROLE if self.advisor(index) => {
                window.put(
                    object,
                    &((MODULE + cadvisor::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                window.pointer(object + cadvisor::CHARACTER, Self::character(index));
                window.literal(object + cadvisor::IDEA_TOKEN, b"idea");
                window.literal(object + cadvisor::SLOT, b"political");
            }
            UNIT if Self::unit(index).is_some() => self.render_unit(&mut window, index, object),
            TRAITS if Self::unit(index).is_some() => {
                for ordinal in 0..Self::traits(index) {
                    let definition = self.definition(index + ordinal);
                    window.pointer(object + ordinal * 8, definition);
                }
            }
            EXPERIENCE if Self::unit(index).is_some() => {
                for ordinal in 0..Self::experience(index) {
                    let entry = object + ordinal * 16;
                    window.pointer(entry, self.definition(index * 3 + ordinal));
                    window.put(entry + 8, &((index * 100 + ordinal) as i64).to_le_bytes());
                }
                for ordinal in 0..2 {
                    let entry = object + 0x80 + ordinal * 16;
                    window.pointer(entry, self.definition(index * 3));
                    window.put(
                        entry + 8,
                        &((index as i64 - ordinal as i64) * 1000).to_le_bytes(),
                    );
                }
            }
            DEFINITION if index < self.definitions => {
                window.put(
                    object,
                    &((MODULE + cunit_leader_trait::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                window.put(
                    object + cunit_leader_trait::TOKEN_ID,
                    &(index as i32 + 7).to_le_bytes(),
                );
                window.put(
                    object + cunit_leader_trait::ORDINAL,
                    &(index as i32).to_le_bytes(),
                );
                window.literal(object + cunit_leader_trait::TOKEN, b"trait");
            }
            _ => return false,
        }
        true
    }

    fn definition(&self, index: usize) -> usize {
        DEFINITION + (index % self.definitions) * 0x1000
    }

    fn render_character(&self, window: &mut Window<'_>, index: usize, object: usize) {
        window.put(
            object,
            &((MODULE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        window.put(
            object + ccharacter::TYPE,
            &ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        window.put(
            object + ccharacter::ID,
            &(1_000_000 + index as u32).to_le_bytes(),
        );
        window.put(
            object + ccharacter::NAME_TOKEN_ID,
            &(index as i32).to_le_bytes(),
        );
        window.literal(object + ccharacter::NAME, b"name");
        window.literal(object + ccharacter::TOKEN, b"token");
        window.put(
            object + ccharacter::COUNTRY,
            &(1 + index as u32 / 31).to_le_bytes(),
        );
        window.pointer(
            object + ccharacter::LEADER_MAP_HEAD,
            Self::leader_head(index),
        );
        window.put(
            object + ccharacter::LEADER_MAP_SIZE,
            &u64::from(Self::leader(index)).to_le_bytes(),
        );
        if let Some(unit) = Self::unit(index) {
            window.pointer(object + ccharacter::UNIT_LEADER, unit);
        }
        window.pointer(
            object + ccharacter::ADVISOR_MAP_HEAD,
            Self::advisor_head(index),
        );
        window.put(
            object + ccharacter::ADVISOR_MAP_SIZE,
            &u64::from(self.advisor(index)).to_le_bytes(),
        );
    }

    fn render_unit(&self, window: &mut Window<'_>, index: usize, object: usize) {
        let vtable = [
            cunit_leader::VTABLE_RVA,
            carmy_leader::VTABLE_RVA,
            cnavy_leader::VTABLE_RVA,
        ][(index / 4) % 3];
        window.put(object, &((MODULE + vtable.0) as u64).to_le_bytes());
        window.put(
            object + cunit_leader::TYPE,
            &cunit_leader::REFERENCE_TYPE.to_le_bytes(),
        );
        window.put(
            object + cunit_leader::ID,
            &(2_000_000 + index as u32).to_le_bytes(),
        );
        window.literal(object + cunit_leader::TOKEN, b"leader");
        window.literal(object + cunit_leader::NAME, b"inherited");
        window.array(
            object + cunit_leader::TRAIT_DATA,
            TRAITS + index * 0x40,
            Self::traits(index),
        );
        window.array(
            object + cunit_leader::TRAIT_EXPERIENCE_DATA,
            EXPERIENCE + index * 0x100,
            Self::experience(index),
        );
        window.array(
            object + cunit_leader::TRAIT_EXPERIENCE_FACTORS,
            Self::factors(index),
            2,
        );
        window.put(
            object + cunit_leader::EXPERIENCE,
            &(index as i64 * 1000).to_le_bytes(),
        );
        window.put(
            object + cunit_leader::CATEGORY,
            &((index % 3) as i32).to_le_bytes(),
        );
        window.put(
            object + cunit_leader::CHARACTER_TYPE,
            &ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        window.put(
            object + cunit_leader::CHARACTER_ID,
            &(1_000_000 + index as u32).to_le_bytes(),
        );
        for (ordinal, offset) in [carmy_leader::ATTACK, carmy_leader::DEFENSE]
            .into_iter()
            .enumerate()
        {
            window.put(object + offset, &(ordinal as i32 + 1).to_le_bytes());
        }
    }
}

struct Window<'a> {
    start: usize,
    out: &'a mut [u8],
}

impl Window<'_> {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        let end = self.start + self.out.len();
        let from = at.max(self.start);
        let to = (at + bytes.len()).min(end);
        if from < to {
            self.out[from - self.start..to - self.start]
                .copy_from_slice(&bytes[from - at..to - at]);
        }
    }

    fn pointer(&mut self, at: usize, target: usize) {
        self.put(at, &(target as u64).to_le_bytes());
    }

    fn array(&mut self, at: usize, data: usize, count: usize) {
        self.pointer(at, data);
        self.put(at + 8, &(count as u32).to_le_bytes());
        self.put(at + 12, &(count as u32).to_le_bytes());
    }

    fn literal(&mut self, at: usize, value: &[u8]) {
        self.put(at, value);
        self.put(at + 16, &(value.len() as u64).to_le_bytes());
        self.put(at + 24, &15_u64.to_le_bytes());
    }

    fn node(&mut self, at: usize, target: usize, nil: u8) {
        for link in [
            msvc_tree_node::LEFT,
            msvc_tree_node::PARENT,
            msvc_tree_node::RIGHT,
        ] {
            self.pointer(at + link, target);
        }
        self.put(at + msvc_tree_node::COLOR, &[1, nil]);
    }
}

impl Memory for World {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(destination.len()) else {
            return false;
        };
        if self
            .faults
            .iter()
            .any(|&fault| (address..end).contains(&fault))
        {
            return false;
        }
        if !self.render(address, destination) {
            return false;
        }
        for (at, bytes) in &self.patches {
            let from = (*at).max(address);
            let to = (at + bytes.len()).min(end);
            if from < to {
                destination[from - address..to - address]
                    .copy_from_slice(&bytes[from - at..to - at]);
            }
        }
        true
    }
}

pub(super) struct Counting<'a> {
    pub inner: &'a World,
    pub calls: Cell<usize>,
    pub bytes: Cell<usize>,
}

impl<'a> Counting<'a> {
    pub fn new(inner: &'a World) -> Self {
        Self {
            inner,
            calls: Cell::new(0),
            bytes: Cell::new(0),
        }
    }

    fn count(&self, bytes: usize) {
        self.calls.set(self.calls.get() + 1);
        self.bytes.set(self.bytes.get() + bytes);
    }
}

impl Memory for Counting<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ok = self.inner.copy(address, destination);
        self.count(if ok { destination.len() } else { 0 });
        ok
    }

    fn copy_scattered<const M: usize>(
        &self,
        addresses: [usize; M],
        destinations: &mut [&mut [u8]; M],
    ) -> usize {
        let completed = self.inner.copy_scattered(addresses, destinations);
        self.count(
            destinations[..completed]
                .iter()
                .map(|span| span.len())
                .sum(),
        );
        completed
    }

    fn copy_scattered_prefetched<const M: usize>(
        &self,
        addresses: [usize; M],
        destinations: &mut [&mut [u8]; M],
    ) -> usize {
        let completed = self
            .inner
            .copy_scattered_prefetched(addresses, destinations);
        self.count(
            destinations[..completed]
                .iter()
                .map(|span| span.len())
                .sum(),
        );
        completed
    }

    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.inner.copy_records(sources, layout, destination);
        self.count(layout.completed_bytes(completed));
        completed
    }

    fn execute_read_plan(
        &self,
        plan: &engine_native_memory::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<engine_native_memory::read_plan::Outcome> {
        use engine_native_memory::read_plan::Outcome as Plan;
        let outcome = self.inner.execute_read_plan(plan, destination);
        let completed = match outcome {
            Some(Plan::Complete) => plan.reads().len(),
            Some(
                Plan::ControlChanged { completed_reads } | Plan::ReadFailed { completed_reads },
            ) => completed_reads,
            None => 0,
        };
        self.count(plan.reads()[..completed].iter().map(|read| read.len).sum());
        outcome
    }
}

pub(super) struct Quiet;

impl advisors::Changes for Quiet {
    fn quiet(&self) -> Option<(u64, u64)> {
        Some((0, 0))
    }
    fn revision(&self, _map: usize) -> Option<(u64, u64)> {
        Some((0, 0))
    }
}

pub(super) fn capacities() -> Capacities {
    Capacities {
        rows: [1 << 18, 1 << 16, 1 << 13, 1 << 16, 1 << 17, 0],
        bytes: [1 << 23, 0, 1 << 20, 1 << 21, 1 << 20, 0],
    }
}

pub(super) fn context(active: [bool; N]) -> CharactersCopyContext {
    CharactersCopyContext {
        force_full: false,
        base: MODULE,
        game_state: GAME_STATE,
        active,
    }
}

fn settle(
    memory: &impl Memory,
    active: [bool; N],
    job: &mut CharactersJob,
    roles: Roles<&census::RoleCensus>,
) {
    for _ in 0..16 {
        let report = copy_characters(memory, context(active), job, roles);
        let grow = job.grow_for_next();
        if grow {
            job.grow();
        }
        job.prepare_read_plans();
        job.clear();
        if report.raw_ok && !grow {
            return;
        }
    }
    panic!("character staging did not settle");
}

fn decode(job: &CharactersJob) -> Captured {
    let mut captured = Captured::with_capacities(&capacities());
    captured.decode_characters_payload(MODULE, job, &mut [0; N]);
    captured
}

pub(super) fn typed(memory: &impl Memory, part: usize) -> Captured {
    let mut capacities = capacities();
    let mut captured = Captured::with_capacities(&capacities);
    loop {
        let results = walk_family(
            memory,
            MODULE,
            GAME_STATE,
            captured.family().select(PART_MASKS[part]),
            None,
            None,
        );
        captured.install_family(PART_MASKS[part], results);
        captured.fail_character_tables(
            PART_MASKS[part].map(|active| !active),
            replay_data::cadence::SKIPPED_REASON,
        );
        let before = captured.allocated_bytes();
        captured.grow(&mut capacities);
        if captured.allocated_bytes() == before {
            return captured;
        }
        captured.reset();
    }
}

pub(super) fn roster(memory: &impl Memory) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    settle(memory, PART_MASKS[0], &mut job, Roles::Gather);
    let report = copy_characters(memory, context(PART_MASKS[0]), &mut job, Roles::Gather);
    assert!(report.raw_ok && !report.plan_failed);
    job
}

fn role(memory: &impl Memory, part: usize, roles: Roles<&census::RoleCensus>) -> (Captured, bool) {
    let mut job = CharactersJob::new_part(part);
    settle(memory, PART_MASKS[part], &mut job, roles);
    let report = copy_characters(memory, context(PART_MASKS[part]), &mut job, roles);
    assert!(report.raw_ok && !report.plan_failed);
    (decode(&job), job.census_capture)
}

fn assert_part(memory: &impl Memory, part: usize, loan: Option<&census::RoleCensus>) -> bool {
    let (independent, walked) = role(memory, part, Roles::Independent);
    assert!(!walked);
    let roles = loan.map_or(Roles::Independent, Roles::Borrow);
    let (loaned, used) = role(memory, part, roles);
    characters_test_kit::assert_matches_direct(&loaned, &independent);
    characters_test_kit::assert_matches_direct(&loaned, &typed(memory, part));
    used || part == 2
}

fn decode_cache(cache: &advisors::Cache) -> Captured {
    let mut captured = Captured::with_capacities(&capacities());
    captured.fail_character_tables(
        PART_MASKS[2].map(|active| !active),
        replay_data::cadence::SKIPPED_REASON,
    );
    cache.decode(&mut captured);
    captured
}

#[test]
fn growth_acquires_current_ideology_for_every_authenticated_role() {
    let _globals = crate::extended::characters::test_globals();
    let world = World::new(1_000);
    let memory = Counting::new(&world);
    let mut job = CharactersJob::with_storage(1, 32, 1024, false);
    let report = copy_characters(
        &memory,
        context(PART_MASKS[1]),
        &mut job,
        Roles::Independent,
    );
    assert!(report.raw_ok && !report.plan_failed);
    let captured = decode(&job);
    assert_eq!(captured.leaders.rows.outcome, Outcome::Rows { len: 200 });
    assert!(captured.leaders.rows.rows[..200].iter().all(|row| {
        row.current_ideology == Some(None) && row.current_ideology_failure.is_none()
    }));
    characters_test_kit::assert_matches_direct(&captured, &typed(&memory, 1));
}

fn small() -> World {
    let mut world = World::new(300);
    world.dual = true;
    world
}

#[test]
fn census_roles_match_the_independent_walk_on_every_roster_shape() {
    let _globals = crate::extended::characters::test_globals();
    let mut shapes = vec![small()];
    let mut aliases = small();
    aliases.roster = Some((0..300).chain([0, 5, 5, 43, 40, 299, 1]).collect());
    shapes.push(aliases);
    let mut generated = small();
    generated.historical = 0;
    shapes.push(generated);
    for world in &shapes {
        let owner = roster(world);
        let loan = owner.role_census().expect("admitted role census");
        for part in 1..PARTS {
            assert!(
                assert_part(world, part, Some(&loan)),
                "part {part} used the loan"
            );
        }
    }
}

#[test]
fn census_roles_match_the_independent_walk_at_every_fault_and_broken_map() {
    let _globals = crate::extended::characters::test_globals();
    let base = small();
    let character = |index: usize| World::character(index);
    let leader_head = World::leader_head;
    let advisor_head = World::advisor_head;
    let mut faults = vec![
        leader_head(1),
        leader_head(0) + msvc_tree_node::NIL,
        advisor_head(3),
        advisor_head(40),
        LEADER_NODE + 5 * 0x40 + msvc_tree_node::RIGHT,
        LEADER_NODE + 10 * 0x40 + character_leader_node::IDEOLOGY,
        LEADER_ROLE + 15 * 0x200,
        ADVISOR_NODE + 43 * 0x60 + character_advisor_node::KEY_HASH,
        ADVISOR_ROLE + 3 * 0x400 + cadvisor::SLOT,
        UNIT + 5 * 0x2000 + cunit_leader::TRAIT_DATA,
        UNIT + 9 * 0x2000 + cunit_leader::TRAIT_EXPERIENCE_DATA,
        TRAITS + 13 * 0x40,
        EXPERIENCE + 17 * 0x100 + 16,
        DEFINITION + 2 * 0x1000,
    ];
    faults.extend((0..300).step_by(37).map(leader_head));
    for fault in faults {
        let mut world = base.clone();
        world.faults.push(fault);
        let owner = roster(&world);
        let loan = owner.role_census().expect("admitted role census");
        for part in 1..PARTS {
            assert!(assert_part(&world, part, Some(&loan)), "fault {fault:x}");
        }
    }
    let broken = [
        (leader_head(1) + msvc_tree_node::NIL, vec![0]),
        (
            leader_head(2) + msvc_tree_node::LEFT,
            0_u64.to_le_bytes().to_vec(),
        ),
        (
            leader_head(6) + msvc_tree_node::RIGHT,
            (character(6) as u64).to_le_bytes().to_vec(),
        ),
        (advisor_head(1) + msvc_tree_node::NIL, vec![0]),
        (
            advisor_head(7) + msvc_tree_node::PARENT,
            1_u64.to_le_bytes().to_vec(),
        ),
        (
            character(4) + ccharacter::LEADER_MAP_HEAD,
            ((character(4) + 0x40) as u64).to_le_bytes().to_vec(),
        ),
        (
            character(8) + ccharacter::ADVISOR_MAP_HEAD,
            ((character(8) + 0xb8) as u64).to_le_bytes().to_vec(),
        ),
        (
            character(9) + ccharacter::LEADER_MAP_HEAD,
            0_u64.to_le_bytes().to_vec(),
        ),
        (
            character(11) + ccharacter::ADVISOR_MAP_SIZE,
            u64::MAX.to_le_bytes().to_vec(),
        ),
        (
            character(12) + ccharacter::LEADER_MAP_SIZE,
            2_u64.to_le_bytes().to_vec(),
        ),
    ];
    for (address, bytes) in broken {
        let mut world = base.clone();
        world.patch(address, &bytes);
        let owner = roster(&world);
        let loan = owner.role_census().expect("admitted role census");
        for part in 1..PARTS {
            assert!(assert_part(&world, part, Some(&loan)), "patch {address:x}");
        }
        let leaders = decode(&{
            let mut job = CharactersJob::new_part(1);
            copy_characters(
                &world,
                context(PART_MASKS[1]),
                &mut job,
                Roles::Borrow(&loan),
            );
            job
        });
        let advisors = role(&world, 2, Roles::Borrow(&loan)).0;
        assert!(
            [leaders.outcomes()[1], advisors.outcomes()[2]]
                .iter()
                .any(|outcome| matches!(outcome, Outcome::Unavailable { .. })),
            "patch {address:x} should fail a role table"
        );
    }
}

#[test]
fn census_is_refused_when_incomplete_unreadable_stale_or_still_loaned() {
    let _globals = crate::extended::characters::test_globals();
    let world = small();
    for field in [
        ccharacter::ADVISOR_MAP_HEAD + 4,
        ccharacter::LEADER_MAP_SIZE,
        ccharacter::COUNTRY,
    ] {
        let mut faulted = world.clone();
        faulted.faults.push(World::character(77) + field);
        let owner = roster(&faulted);
        if field == ccharacter::COUNTRY {
            assert!(owner.portrait_census().is_none());
        }
        assert!(owner.role_census().is_none(), "field {field:x}");
        for part in 1..PARTS {
            assert_part(&faulted, part, None);
        }
    }
    let mut short = CharactersJob::new_part(0);
    settle(&world, PART_MASKS[0], &mut short, Roles::Gather);
    short.portrait_census = Some(
        engine_allocation::shared(ccharacter_manager::PortraitCensus::try_new(2).unwrap()).unwrap(),
    );
    copy_characters(&world, context(PART_MASKS[0]), &mut short, Roles::Gather);
    assert!(short.role_census().is_some());
    let mut narrow = CharactersJob::new_part(0);
    settle(&world, PART_MASKS[0], &mut narrow, Roles::Gather);
    narrow.role_census = Some(engine_allocation::shared(census::RoleCensus::new(2)).unwrap());
    copy_characters(&world, context(PART_MASKS[0]), &mut narrow, Roles::Gather);
    assert!(narrow.portrait_census().is_some());
    let loan = narrow.role_census().expect("grown role census");
    let other = CharactersCopyContext {
        game_state: GAME_STATE + 8,
        ..context(PART_MASKS[1])
    };
    let mut job = CharactersJob::new_part(1);
    copy_characters(&world, other, &mut job, Roles::Borrow(&loan));
    assert!(!job.census_capture);
    copy_characters(
        &world,
        context(PART_MASKS[0]),
        &mut narrow,
        Roles::Independent,
    );
    assert!(narrow.role_census().is_none());
    copy_characters(&world, context(PART_MASKS[0]), &mut narrow, Roles::Gather);
    assert!(narrow.role_census().is_none());
    drop(loan);
    copy_characters(&world, context(PART_MASKS[0]), &mut narrow, Roles::Gather);
    assert!(narrow.role_census().is_some());
}

#[test]
fn short_census_visit_storage_grows_to_the_whole_demand_in_the_pass() {
    let _globals = crate::extended::characters::test_globals();
    let world = small();
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    for part in [1, 3] {
        let mut job = CharactersJob::new_part(part);
        job.leader_cache = None;
        settle(&world, PART_MASKS[part], &mut job, Roles::Borrow(&loan));
        job.census_visited = Vec::with_capacity(3);
        let report = copy_characters(
            &world,
            context(PART_MASKS[part]),
            &mut job,
            Roles::Borrow(&loan),
        );
        assert!(report.raw_ok && !report.plan_failed, "part {part}");
        assert_eq!(job.census_required, 0, "part {part}");
        assert!(job.census_visited.len() > 3, "part {part}");
        characters_test_kit::assert_matches_direct(&decode(&job), &typed(&world, part));
    }
}

#[test]
fn census_advisor_cache_keeps_reuse_and_fresh_countries() {
    let _globals = crate::extended::characters::test_globals();
    let mut world = small();
    let ctx = context(PART_MASKS[2]);
    let fresh = |world: &World| typed(world, 2);
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    let demand = cadvisor::modifier_demand(&world, MODULE, GAME_STATE).unwrap();
    let mut cache = advisors::Cache::with_modifier_demand(demand);
    cache.capture(&world, ctx, &Quiet, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(cache.counts.planned, 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &Quiet, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(cache.counts.planned, 2);
    assert_eq!(counting.calls.get(), 2 + 2 * demand.rows);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    drop(loan);
    world.patch(
        World::character(3) + ccharacter::COUNTRY,
        &99_u32.to_le_bytes(),
    );
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    cache.capture(&world, ctx, &Quiet, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(cache.counts.planned, 2);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    world.patch(World::advisor_head(43) + msvc_tree_node::COLOR, &[0]);
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    cache.capture(&world, ctx, &Quiet, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(cache.counts.planned, 1);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    cache.capture(&world, ctx, &Quiet, None);
    assert_eq!(cache.counts.planned, 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    cache.capture(&world, ctx, &Quiet, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(cache.counts.planned, 0);
    assert!(cache.counts.reused > 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
}

pub(super) struct Advisors {
    pub revision: Cell<u64>,
}

impl advisors::Changes for Advisors {
    fn quiet(&self) -> Option<(u64, u64)> {
        Some((0, self.revision.get()))
    }
    fn revision(&self, _map: usize) -> Option<(u64, u64)> {
        Some((0, 0))
    }
}

#[test]
fn quiet_advisor_hours_read_nothing() {
    let _globals = crate::extended::characters::test_globals();
    use collector_observers::character_changes::Counting as Lookups;
    static DOMAIN: Lookups = Lookups::new();
    let mut world = small();
    DOMAIN.watches.enable(world.characters);
    let ctx = context(PART_MASKS[2]);
    let fresh = |world: &World| typed(world, 2);
    let observer = Advisors {
        revision: Cell::new(0),
    };
    let demand = cadvisor::modifier_demand(&world, MODULE, GAME_STATE).unwrap();
    let mut cache = advisors::Cache::with_modifier_demand(demand);
    cache.observe_roles(&DOMAIN);
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    cache.capture(&world, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    assert!(DOMAIN.lookups() > 0);
    cache.prepare();
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(counting.calls.get(), 0, "a quiet hour reads nothing");
    assert_eq!(DOMAIN.lookups(), 0, "a quiet hour looks up no character");
    assert_eq!(cache.counts.planned, 0);
    assert!(cache.counts.reused > 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    observer.revision.set(1);
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert!(counting.calls.get() > 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert_eq!(counting.calls.get(), 0);
    drop(DOMAIN.watches.notify(Some(World::character(5)), 0));
    DOMAIN.lookups();
    cache.capture(&world, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert!(DOMAIN.lookups() > 0);
    cache.prepare();
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert_eq!((counting.calls.get(), DOMAIN.lookups()), (0, 0));
    drop(loan);
    world.patch(
        World::character(3) + ccharacter::COUNTRY,
        &99_u32.to_le_bytes(),
    );
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert!(counting.calls.get() > 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
    cache.prepare();
    DOMAIN.watches.boundary(true);
    DOMAIN.lookups();
    let counting = Counting::new(&world);
    cache.capture(&counting, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
    assert!(counting.calls.get() > 0 && DOMAIN.lookups() > 0);
    characters_test_kit::assert_matches_direct(&decode_cache(&cache), &fresh(&world));
}

#[test]
fn visit_lists_follow_the_live_census_length() {
    let _globals = crate::extended::characters::test_globals();
    let world = small();
    let owner = roster(&world);
    assert!(owner.role_census().is_some());
    assert!(census::live_len() > 0);
    let mut job = CharactersJob::new_part(1);
    job.census_visited = Vec::new();
    assert_eq!(job.visits_required(105), 105);
    job.census_required = 109;
    assert_eq!(job.visits_required(105), 109);
    assert_eq!(CharactersJob::new_part(0).visits_required(105), 0);
    job.grow();
    assert!(job.census_visited.capacity() >= 109);
}

#[test]
fn a_frame_publishes_its_held_advisor_table_without_rewriting_it() {
    let _globals = crate::extended::characters::test_globals();
    use collector_observers::character_changes::Counting as Lookups;
    static DOMAIN: Lookups = Lookups::new();
    let world = small();
    DOMAIN.watches.enable(world.characters);
    let ctx = context(PART_MASKS[2]);
    let observer = Advisors {
        revision: Cell::new(0),
    };
    let demand = cadvisor::modifier_demand(&world, MODULE, GAME_STATE).unwrap();
    let mut cache = advisors::Cache::with_modifier_demand(demand);
    cache.observe_roles(&DOMAIN);
    let owner = roster(&world);
    let loan = owner.role_census().unwrap();
    let mut frame = Captured::with_capacities(&capacities());
    let hour = |cache: &mut advisors::Cache, frame: &mut Captured| {
        cache.capture(&world, ctx, &observer, loan.admitted(MODULE, GAME_STATE));
        cache.prepare();
        frame.grow(&mut capacities());
        frame.reset();
        frame.fail_character_tables(
            PART_MASKS[2].map(|active| !active),
            replay_data::cadence::SKIPPED_REASON,
        );
        cache.decode(frame);
        frame.tables()[2].clone()
    };
    let fresh = typed(&world, 2).tables()[2].clone();
    for _ in 0..2 {
        assert_eq!(hour(&mut cache, &mut frame), fresh);
    }
    const MARK: u32 = 0x5eed;
    frame.advisors.rows.rows[0].character = MARK;
    hour(&mut cache, &mut frame);
    assert_eq!(cache.counts.planned, 0);
    assert_eq!(
        frame.advisors.rows.rows[0].character, MARK,
        "rows republished in place"
    );
    observer.revision.set(1);
    assert_eq!(hour(&mut cache, &mut frame), fresh);
}
