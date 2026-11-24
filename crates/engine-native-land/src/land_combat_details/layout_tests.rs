use engine_native::{carmy, cgame_state, cland_combatant, cnaval_combat, cprovince, cunit};
use engine_native_memory::{Memory, cpdx_array::Array, walk::Outcome};
use engine_native_parts::{cland_border_war_combat, cland_border_war_combatant};
use crate::land_combat_details::{
    detail_demand,
    CommanderTimedTrait,
    Commanders,
    DetailArenas,
    DetailCounts,
    DetailsRow,
    Leaders,
    Span,
    read_details,
    };
use engine_native::cunit_leader::{CommanderLearningTrait, CommanderTrait};
use crate::land_combat_details::details::{
    DetailReader,
    copy_detail_spans,
    push,
};
use engine_native::cland_combat::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

struct Arenas {
    participants: Vec<u64>,
    air: Vec<i32>,
    text: Vec<u8>,
    traits: Vec<CommanderTrait>,
    timed: Vec<CommanderTimedTrait>,
    learning: Vec<CommanderLearningTrait>,
}

impl Arenas {
    fn new() -> Self {
        Self::sized(DetailCounts {
            participants: 8,
            air: 8,
            text: 512,
            traits: 8,
            timed: 8,
            learning: 8,
        })
    }
    fn sized(counts: DetailCounts) -> Self {
        Self {
            participants: vec![u64::MAX; counts.participants],
            air: vec![0; counts.air],
            text: vec![0; counts.text],
            traits: vec![CommanderTrait::default(); counts.traits],
            timed: vec![CommanderTimedTrait::default(); counts.timed],
            learning: vec![CommanderLearningTrait::default(); counts.learning],
        }
    }
    fn view(&mut self) -> DetailArenas<'_> {
        DetailArenas::new(
            &mut self.participants,
            &mut self.air,
            &mut self.text,
            &mut self.traits,
            &mut self.timed,
            &mut self.learning,
        )
    }
}

struct Observed<'a, M> {
    source: &'a M,
    overrides: &'a [(usize, Vec<u8>)],
    reads: RefCell<Vec<(usize, usize)>>,
    scatters: RefCell<Vec<Vec<(usize, usize)>>>,
    fault: Option<usize>,
    change: Option<(usize, usize)>,
}

impl<'a, M> Observed<'a, M> {
    fn new(source: &'a M, overrides: &'a [(usize, Vec<u8>)]) -> Self {
        Self {
            source,
            overrides,
            reads: RefCell::default(),
            scatters: RefCell::default(),
            fault: None,
            change: None,
        }
    }
}

impl<M: Memory> Memory for Observed<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fault == Some(ordinal) {
            let prefix = destination.len().div_ceil(2);
            destination[..prefix].fill(0xed);
            return false;
        }
        let copied = if let Some((_, bytes)) = self
            .overrides
            .iter()
            .find(|(p, bytes)| *p == address && bytes.len() == destination.len())
        {
            destination.copy_from_slice(bytes);
            true
        } else {
            self.source.copy(address, destination)
        };
        if copied
            && let Some((at, byte)) = self.change
            && at == ordinal
        {
            destination[byte] ^= 1;
        }
        copied
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scatters.borrow_mut().push(
            addresses
                .iter()
                .zip(destinations.iter())
                .map(|(&p, d)| (p, d.len()))
                .collect(),
        );
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !engine_native_memory::read_slice(self, address, destination) {
                return index;
            }
        }
        N
    }
}

fn scalar_details(
    memory: &impl Memory,
    base: usize,
    source: Source,
    arenas: &mut DetailArenas<'_>,
) -> Result<DetailsRow, &'static str> {
    let reader = DetailReader(Reader {
        memory,
        base,
        provinces: source.provinces,
    });
    let border = reader
        .class(source.pointer)?
        .ok_or("land details source is naval")?;
    let (id, attackers, defenders) = reader.header(source.pointer, border)?;
    if id != source.id || attackers != source.attackers || defenders != source.defenders {
        return Err("land details source identity changed");
    }
    let province_id = reader.row(source.pointer, id)?.province;
    let mut row = DetailsRow {
        id,
        province: province_id,
        phase: reader.field(source.pointer, PHASE)?,
        width_factor: reader.field(source.pointer, WIDTH_FACTOR)?,
        attacker_bonus: reader.field(source.pointer, ATTACKER_BONUS)?,
        defender_bonus: reader.field(source.pointer, DEFENDER_BONUS)?,
        ..DetailsRow::default()
    };
    if border {
        row.border_war = Some(crate::land_combat_details::details::read_border_war(&reader, source, arenas)?);
    }
    let stored_terrain = reader.pointer(source.pointer, STORED_TERRAIN)?;
    let stored_vtable = reader.field::<u64>(stored_terrain, 0)?;
    if ![
        engine_native_parts::cterrain_type::VTABLE_RVA,
        engine_native_parts::cterrain_type::NULL_VTABLE_RVA,
    ]
    .into_iter()
    .any(|key| engine_sites::at(base, key).map(|p| p as u64) == Some(stored_vtable))
    {
        return Err("land stored terrain vtable mismatch");
    }
    row.stored_terrain_valid = reader.field(stored_terrain, engine_native_parts::cterrain_type::VALID)?;
    if row.stored_terrain_valid != 0 {
        row.stored_terrain_name =
            reader.text(stored_terrain, engine_native_parts::cterrain_type::NAME, arenas)?;
    }
    for (index, offset) in [ATTACKER, DEFENDER].into_iter().enumerate() {
        let pointer = reader.pointer(source.pointer, offset)?;
        let side = &mut row.sides[index];
        reader.commander(
            pointer,
            &Members::none(),
            &mut Commanders::default(),
            &mut Leaders::default(),
            index,
            [attackers, defenders][index],
            side,
            arenas,
        )?;
        for (which, vector_offset) in [cland_combatant::COMMITTED, cland_combatant::RESERVES]
            .into_iter()
            .enumerate()
        {
            let vector = reader.array(pointer, vector_offset, 8)?;
            let (start, count) = (arenas.used.participants, vector.count as usize);
            if which == 0 {
                side.committed_start = start;
                side.committed_len = count;
            } else {
                side.reserve_start = start;
                side.reserve_len = count;
            }
            for member in 0..vector.count {
                let handle = reader.division(reader.element(vector, member)?)?;
                push(
                    &mut arenas.participants,
                    &mut arenas.used.participants,
                    &mut arenas.required.participants,
                    handle,
                );
            }
        }
        let tactic = reader.pointer(pointer, cland_combatant::TACTIC)?;
        let tactic_vtable = if tactic == 0 {
            0
        } else {
            reader.field::<u64>(tactic, 0)?
        };
        if Some(tactic_vtable)
            == engine_sites::at(base, engine_native_parts::ccombat_tactic::VTABLE_RVA)
                .map(|v| v as u64)
        {
            side.tactic_kind = 2;
            side.tactic_id = reader.field(tactic, engine_native_parts::ccombat_tactic::ID)?;
            side.tactic_counter_id = reader.field(tactic, engine_native_parts::ccombat_tactic::COUNTER_ID)?;
            side.tactic_token = reader.text(tactic, engine_native_parts::ccombat_tactic::TOKEN, arenas)?;
        } else if tactic == 0 {
            side.tactic_kind = 0;
        } else if Some(tactic_vtable)
            != engine_sites::at(base, engine_native_parts::cnull_combat_tactic::VTABLE_RVA)
                .map(|v| v as u64)
        {
            return Err("land detail tactic vtable mismatch");
        } else {
            side.tactic_kind = 1;
        }
        let air = reader.array(pointer, cland_combatant::AIR_SUPPORT, 8)?;
        side.air_counts = Span {
            start: arenas.used.air,
            len: air.count as usize,
        };
        for member in 0..air.count {
            let planes = reader.field(
                reader.element(air, member)?,
                engine_native_parts::land_combat_air_entry::PLANE_COUNT,
            )?;
            push(
                &mut arenas.air,
                &mut arenas.used.air,
                &mut arenas.required.air,
                planes,
            );
        }
        side.anti_air_attack = reader.field(pointer, cland_combatant::ANTI_AIR_ATTACK)?;
        side.air_kills = reader.field(pointer, cland_combatant::AIR_KILLS)?;
        let damage = reader.field::<[u8; 48]>(pointer, cland_combatant::AIR_STRENGTH_DAMAGE)?;
        for (value, bytes) in side.damage.iter_mut().zip(damage.chunks_exact(8)) {
            *value = i64::from_le_bytes(bytes.try_into().unwrap());
        }
        side.organisation_loss_count =
            reader.field(pointer, cland_combatant::ORGANISATION_LOSS_COUNT)?;
        if side.organisation_loss_count >= 24 {
            let samples = reader.pointer(pointer, cland_combatant::ORGANISATION_LOSS_WINDOW)?;
            let loss = reader.field::<[u8; 192]>(samples, 0)?;
            for (value, bytes) in side.organisation_loss.iter_mut().zip(loss.chunks_exact(8)) {
                *value = i64::from_le_bytes(bytes.try_into().unwrap());
            }
        }
    }
    Ok(row)
}

fn assert_detail_attempt(
    fixture: &Fixture,
    overrides: &[(usize, Vec<u8>)],
    fault: Option<usize>,
    change: Option<(usize, usize)>,
    capacity: usize,
) -> Vec<(usize, usize)> {
    let source = DetailMemory {
        fixture,
        copies: 0.into(),
        bytes: 0.into(),
    };
    let mut actual = Observed::new(&source, overrides);
    actual.fault = fault;
    actual.change = change;
    let mut expected = Observed::new(&source, overrides);
    expected.fault = fault;
    expected.change = change;
    let sized = || {
        let mut arenas = Arenas::new();
        arenas.participants.truncate(capacity);
        arenas
    };
    let (mut actual_arenas, mut expected_arenas) = (sized(), sized());
    let (mut actual_view, mut expected_view) = (actual_arenas.view(), expected_arenas.view());
    assert_eq!(
        read_details(
            &actual,
            BASE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
            &mut actual_view
        ),
        scalar_details(&expected, BASE, detail_source(), &mut expected_view),
        "fault {fault:?}, change {change:?}, capacity {capacity}"
    );
    assert_eq!(
        (actual_view.used, actual_view.required),
        (expected_view.used, expected_view.required),
        "detail arena demand"
    );
    assert_eq!(
        actual_arenas.participants, expected_arenas.participants,
        "partial participant writes"
    );
    assert_eq!(actual_arenas.air, expected_arenas.air);
    assert_eq!(actual_arenas.text, expected_arenas.text);
    assert_eq!(actual_arenas.traits, expected_arenas.traits);
    assert_eq!(actual_arenas.timed, expected_arenas.timed);
    assert_eq!(actual_arenas.learning, expected_arenas.learning);
    assert_eq!(
        *actual.reads.borrow(),
        *expected.reads.borrow(),
        "exact observation order"
    );
    actual.reads.into_inner()
}

#[test]
fn details_leaf_batches_match_scalar_values_every_fault_and_changed_read() {
    for border in [false, true] {
        let mut fixture = commander_fixture();
        fixture.u32(COMBAT + PHASE, i32::MIN as u32);
        fixture.u64(COMBAT + WIDTH_FACTOR, i64::MIN as u64);
        fixture.u64(COMBAT + ATTACKER_BONUS, i64::MAX as u64);
        fixture.u64(COMBAT + DEFENDER_BONUS, u64::MAX);
        if border {
            fixture.u64(COMBAT, (BASE + cland_border_war_combat::VTABLE_RVA.0) as u64);
            fixture.u64(
                COMBAT + BODY,
                (BASE + cland_border_war_combat::BODY_VTABLE_RVA.0) as u64,
            );
            for side in [ATTACK, DEFEND] {
                fixture.u64(side, (BASE + cland_border_war_combatant::VTABLE_RVA.0) as u64);
                fixture.u64(side + cland_combatant::BORDER_MODIFIER, i64::MIN as u64);
                fixture.u64(
                    side + cland_combatant::BORDER_DIG_IN_FACTOR,
                    i64::MAX as u64,
                );
                fixture.u64(side + cland_combatant::BORDER_TERRAIN_FACTOR, u64::MAX);
                fixture.u32(side + 0x4c0, if side == ATTACK { 4 } else { 7 });
                fixture.u64(side + 0x4c8, 0x8a00);
                fixture.u64(side + 0x500 + 24, 15);
                fixture.u64(side + 0x520 + 24, 15);
                fixture.u64(0x8a00, (BASE + engine_native::cstate::VTABLE_RVA.0) as u64);
                fixture.u32(0x8a00 + engine_native::cstate::ID, 19);
            }
        }
        let overrides = Vec::new();
        let source = DetailMemory {
            fixture: &fixture,
            copies: 0.into(),
            bytes: 0.into(),
        };
        let actual = Observed::new(&source, &overrides);
        let row = read_details(
            &actual,
            BASE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
            &mut Arenas::new().view(),
        )
        .unwrap();
        assert_eq!(row.phase, i32::MIN);
        let expected_initial = vec![
            (COMBAT + PHASE, 4),
            (COMBAT + WIDTH_FACTOR, 8),
            (COMBAT + ATTACKER_BONUS, 8),
            (COMBAT + DEFENDER_BONUS, 8),
        ];
        assert!(actual.scatters.borrow().contains(&expected_initial));
        for side in [ATTACK, DEFEND] {
            let tail = vec![
                (side + cland_combatant::ANTI_AIR_ATTACK, 8),
                (side + cland_combatant::AIR_KILLS, 4),
                (side + cland_combatant::AIR_STRENGTH_DAMAGE, 48),
                (side + cland_combatant::ORGANISATION_LOSS_COUNT, 4),
            ];
            assert!(actual.scatters.borrow().contains(&tail));
            assert!(
                !actual
                    .reads
                    .borrow()
                    .iter()
                    .any(|&(address, _)| address == side + cland_combatant::BORDER_MODIFIER)
            );
        }
        let trace = assert_detail_attempt(&fixture, &overrides, None, None, 8);
        for ordinal in 0..trace.len() {
            assert_detail_attempt(&fixture, &overrides, Some(ordinal), None, 8);
            assert_detail_attempt(&fixture, &overrides, None, Some((ordinal, 0)), 8);
        }

    }
}

#[test]
fn detail_scatter_partial_destinations_and_checked_address_prefix_match_scalar() {
    let source = SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4, 5, 6, 7, 8],
    };
    for invalid in [(usize::MAX, 1), (usize::MAX - 3, 0), (0, 0)] {
        for fault in [None, Some(0), Some(1)] {
            let mut actual = Observed::new(&source, &[]);
            let mut expected = Observed::new(&source, &[]);
            actual.fault = fault;
            expected.fault = fault;
            let sources = [(0x1000, 0), (0, 0), (0x1000, 0), invalid, (0x1000, 0)];
            let mut actual_bytes = [[0xa5; 8]; 5];
            let mut expected_bytes = actual_bytes;
            let mut destinations = actual_bytes.each_mut().map(|v| v.as_mut_slice());
            destinations[1] = &mut [];
            let result = copy_detail_spans(&actual, sources, &mut destinations);
            let mut expected_result = Ok(());
            for (index, (pointer, offset)) in sources.into_iter().enumerate() {
                if index == 1 {
                    continue;
                }
                if !pointer.checked_add(offset).is_some_and(|address| {
                    engine_native_memory::read_slice(&expected, address, &mut expected_bytes[index])
                }) {
                    expected_result = Err("land combat field unreadable or address overflow");
                    break;
                }
            }
            assert_eq!(result, expected_result);
            assert_eq!(actual_bytes, expected_bytes);
            assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
            assert_eq!(actual_bytes[4], [0xa5; 8]);
        }
    }
}

const BASE: usize = 0x1_4000_0000;
const STATE: usize = 0x1000;
const REGISTRY: usize = 0x2000;
const PROVINCES: usize = 0x2100;
const COMBAT: usize = 0x3000;
const ATTACK: usize = 0x4000;
const DEFEND: usize = 0x5000;
const ATTACK_UNITS: usize = 0x4500;
const DEFEND_UNITS: usize = 0x5500;
const UNIT_A: usize = 0x6000;
const UNIT_B: usize = 0x6100;
const UNIT_C: usize = 0x6200;
const NAVAL: usize = 0x7000;
const PROVINCE_OBJECT: usize = 0x7800;

struct Fixture(Vec<u8>);

#[test]
fn growing_map_census_starts_without_optional_pointers_and_keeps_later_demand() {
    let mut fixture = Fixture::new();
    let mut rows = Vec::new();
    let mut participants = Vec::new();
    let mut classes = Vec::new();
    for count in [50, 120] {
        let registry = 0x20000;
        let units = registry + count * 8;
        fixture.0.resize(units + count * 8 - STATE, 0);
        fixture.array(STATE + cgame_state::COMBATS + 8, registry, count as u32);
        fixture.array(ATTACK + cland_combatant::UNITS, units, count as u32);
        fixture.array(DEFEND + cland_combatant::UNITS, 0, 0);
        for index in 0..count {
            fixture.u64(registry + index * 8, COMBAT as u64);
            fixture.u64(units + index * 8, UNIT_A as u64);
        }
        let demand = core_demand(&fixture.memory(), BASE, STATE).unwrap();
        assert_eq!(demand.rows, count);
        assert_eq!(demand.participants, count * count);
        fixture.array(STATE + cgame_state::COMBATS + 8, registry, 1);
        let mut visited = 0;
        let capture = walk_with(
            &fixture.memory(),
            BASE,
            STATE,
            &mut rows,
            &mut participants,
            &mut classes,
            |_| visited += 1,
        );
        assert_eq!(capture.outcome, Outcome::Rows { len: 1 });
        assert_eq!((capture.participants_len, visited), (count, 1));
        assert!(participants[..count].iter().all(|id| *id == 1));
        assert!(
            classes[..count]
                .iter()
                .all(|class| *class == DIVISION_CLASS)
        );
        fixture.array(STATE + cgame_state::COMBATS + 8, registry, count as u32);
        fixture.array(ATTACK + cland_combatant::UNITS, 0, 0);
        let capture = walk_with(
            &fixture.memory(),
            BASE,
            STATE,
            &mut rows,
            &mut participants,
            &mut classes,
            |_| {},
        );
        assert_eq!(capture.outcome, Outcome::Rows { len: count });
        assert_eq!(capture.participants_len, 0);
        assert!(
            rows[..count]
                .iter()
                .all(|row| row.id == ((0xf000_0001_u64 << 32) | TYPE_WORD as u64))
        );
    }
}

impl Fixture {
    fn new() -> Self {
        let mut value = Self(vec![0; 0x8000]);
        value.array(STATE + cgame_state::COMBATS + 8, REGISTRY, 2);
        value.u64(REGISTRY, COMBAT as u64);
        value.u64(REGISTRY + 8, NAVAL as u64);
        value.u64(NAVAL, (BASE + cnaval_combat::VTABLE_RVA.0) as u64);
        value.array(STATE + cgame_state::PROVINCES, PROVINCES, 2);
        value.u64(PROVINCES + 8, PROVINCE_OBJECT as u64);
        value.u64(PROVINCE_OBJECT, (BASE + cprovince::VTABLE_RVA.0) as u64);
        value.u32(PROVINCE_OBJECT + cprovince::ID, 1);
        value.u64(COMBAT, (BASE + VTABLE_RVA.0) as u64);
        value.u64(COMBAT + BODY, (BASE + BODY_VTABLE_RVA.0) as u64);
        value.u64(COMBAT + HANDLE, (0xf000_0001_u64 << 32) | TYPE_WORD as u64);
        value.u64(COMBAT + PROVINCE, PROVINCE_OBJECT as u64);
        value.u32(COMBAT + DURATION, 24);
        value.u32(COMBAT + DAY, u32::MAX);
        value.u64(COMBAT + ATTACKER, ATTACK as u64);
        value.u64(COMBAT + DEFENDER, DEFEND as u64);
        for (side, attacker, units, count) in
            [(ATTACK, 1, ATTACK_UNITS, 2), (DEFEND, 0, DEFEND_UNITS, 1)]
        {
            value.u64(side, (BASE + cland_combatant::VTABLE_RVA.0) as u64);
            value.u64(side + cland_combatant::COMBAT, COMBAT as u64);
            value.0[side + cland_combatant::IS_ATTACKER - STATE] = attacker;
            value.array(side + cland_combatant::UNITS, units, count);
        }
        value.u64(ATTACK_UNITS, UNIT_B as u64);
        value.u64(ATTACK_UNITS + 8, UNIT_A as u64);
        value.u64(DEFEND_UNITS, UNIT_C as u64);
        for (unit, id) in [(UNIT_A, 1), (UNIT_B, u64::MAX - 4), (UNIT_C, 62)] {
            value.u64(unit, (BASE + carmy::VTABLE_RVA.0) as u64);
            value.u64(
                unit + carmy::SECONDARY_BASE,
                (BASE + carmy::SECONDARY_VTABLE_RVA.0) as u64,
            );
            value.u64(unit + cunit::HANDLE, id);
        }
        value
    }

    fn u64(&mut self, address: usize, value: u64) {
        self.0[address - STATE..address - STATE + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, address: usize, value: u32) {
        self.0[address - STATE..address - STATE + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: STATE,
            bytes: &self.0,
        }
    }

    fn capture(&self) -> (Walk, Vec<Row>, Vec<u64>) {
        let (captured, rows, participants, _) = self.capture_classes();
        (captured, rows, participants)
    }

    fn capture_classes(&self) -> (Walk, Vec<Row>, Vec<u64>, Vec<u32>) {
        let mut rows = vec![Row::default(); 2];
        let mut participants = vec![0; 8];
        let mut classes = vec![u32::MAX; 8];
        let captured = walk(
            &self.memory(),
            BASE,
            STATE,
            &mut rows,
            &mut participants,
            &mut classes,
        );
        (captured, rows, participants, classes)
    }
}

#[test]
fn mixed_registry_preserves_full_native_handles_and_participant_order() {
    let fixture = Fixture::new();
    let (captured, rows, participants) = fixture.capture();
    assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.participants_len, 3);
    assert_eq!(captured.required_participants, 3);
    assert_eq!(
        rows[0],
        Row {
            id: (0xf000_0001_u64 << 32) | TYPE_WORD as u64,
            province: 1,
            duration: 24,
            day: -1,
            attacker_start: 0,
            attacker_len: 2,
            defender_start: 2,
            defender_len: 1,
        }
    );
    assert_eq!(&participants[..3], &[u64::MAX - 4, 1, 62]);
}

#[test]
fn a_non_division_participant_is_kept_with_its_class() {
    let mut fixture = Fixture::new();
    fixture.u64(UNIT_A, (BASE + engine_native_parts::crailway_gun::VTABLE_RVA.0) as u64);
    fixture.u32(UNIT_A + cunit::CLASS, RAILWAY_GUN_CLASS);
    fixture.u64(
        UNIT_A + carmy::SECONDARY_BASE,
        (BASE + engine_native_parts::crailway_gun::SECONDARY_VTABLE_RVA.0) as u64,
    );
    fixture.u64(UNIT_C, (BASE + engine_native::ctask_force::VTABLE_RVA.0) as u64);
    fixture.u32(UNIT_C + cunit::CLASS, TASK_FORCE_CLASS);
    fixture.u64(
        UNIT_C + carmy::SECONDARY_BASE,
        (BASE + engine_native::ctask_force::SECONDARY_VTABLE_RVA.0) as u64,
    );
    let (captured, _, participants, classes) = fixture.capture_classes();
    assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
    assert_eq!(&participants[..3], &[u64::MAX - 4, 1, 62]);
    assert_eq!(
        &classes[..3],
        &[DIVISION_CLASS, RAILWAY_GUN_CLASS, TASK_FORCE_CLASS]
    );
    let mut pointers = vec![0; 8];
    let members = walk_members(
        &fixture.memory(),
        BASE,
        STATE,
        &mut vec![Row::default(); 2],
        &mut vec![0; 8],
        &mut pointers,
        |_, _| {},
    );
    assert_eq!(
        members.outcome,
        Outcome::Unavailable {
            reason: "land combat participant is not a division"
        }
    );

    fixture.u32(UNIT_A + cunit::CLASS, DIVISION_CLASS);
    assert_eq!(
        fixture.capture().0.outcome,
        Outcome::Unavailable {
            reason: "land combat participant vtable mismatch"
        }
    );
    fixture.u32(UNIT_A + cunit::CLASS, 2);
    assert_eq!(
        fixture.capture().0.outcome,
        Outcome::Unavailable {
            reason: "land combat participant has an unknown unit class"
        }
    );
}

struct DetailMemory<'a> {
    fixture: &'a Fixture,
    copies: std::cell::Cell<usize>,
    bytes: std::cell::Cell<usize>,
}

impl Memory for DetailMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.bytes.set(self.bytes.get() + destination.len());
        use engine_native_parts::modifier_definition as md;
        if address == BASE + md::TABLE_RVA.0 && destination.len() == 8 {
            destination.copy_from_slice(&0x200000_u64.to_le_bytes());
            return true;
        }
        if address == BASE + md::ELIGIBILITY_CALLBACK_RVA.0 && destination.len() == 8 {
            destination
                .copy_from_slice(&((BASE + md::ELIGIBILITY_FLAGS_GETTER_RVA.0) as u64).to_le_bytes());
            return true;
        }
        if address == BASE + md::TRADE_FEATURE_FLAGS_RVA.0 && destination.len() == 4 {
            destination.copy_from_slice(&0x80000002_u32.to_le_bytes());
            return true;
        }
        if address == 0x200000 + 0x5f * md::STRIDE + md::FLAGS && destination.len() == 12 {
            for (bytes, value) in destination.chunks_exact_mut(4).zip([0x10_u32, u32::MAX, 2]) {
                bytes.copy_from_slice(&value.to_le_bytes());
            }
            return true;
        }
        if address == BASE + engine_native_naval::naval_commander_inputs::SKILL_DATABASE_RVA.0
            && destination.len() == 8
        {
            destination.copy_from_slice(&0x11000_u64.to_le_bytes());
            return true;
        }
        if address == BASE + engine_native_naval::naval_commander_inputs::LEARNING_THRESHOLD_RVA.0
            && destination.len() == 8
        {
            destination.copy_from_slice(&5_000_i64.to_le_bytes());
            return true;
        }
        self.fixture.memory().copy(address, destination)
    }
}

fn detail_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.u64(0x8200, (BASE + engine_native_parts::cterrain_type::VTABLE_RVA.0) as u64);
    fixture.u64(COMBAT + STORED_TERRAIN, 0x8200);
    fixture.0[0x8200 + engine_native_parts::cterrain_type::VALID - STATE] = 1;
    fixture.u64(0x8200 + engine_native_parts::cterrain_type::NAME, 0x6e69616c70);
    fixture.u64(0x8200 + engine_native_parts::cterrain_type::NAME + 16, 5);
    fixture.u64(0x8200 + engine_native_parts::cterrain_type::NAME + 24, 15);
    fixture.u64(0x8600, (BASE + engine_native_parts::ccombat_tactic::VTABLE_RVA.0) as u64);
    fixture.u32(0x8600 + engine_native_parts::ccombat_tactic::ID, 8);
    fixture.u32(0x8600 + engine_native_parts::ccombat_tactic::COUNTER_ID, 9);
    fixture.u64(0x8600 + engine_native_parts::ccombat_tactic::TOKEN, 0xff61);
    fixture.u64(0x8600 + engine_native_parts::ccombat_tactic::TOKEN + 16, 2);
    fixture.u64(0x8600 + engine_native_parts::ccombat_tactic::TOKEN + 24, 15);
    for side in [ATTACK, DEFEND] {
        fixture.u64(side + cland_combatant::TACTIC, 0x8600);
    }
    fixture.array(ATTACK + cland_combatant::COMMITTED, ATTACK_UNITS, 1);
    fixture.array(ATTACK + cland_combatant::RESERVES, ATTACK_UNITS + 8, 1);
    fixture.array(DEFEND + cland_combatant::COMMITTED, DEFEND_UNITS, 1);
    fixture.array(ATTACK + cland_combatant::AIR_SUPPORT, 0x8800, 1);
    fixture.u64(0x8800, 0x8900);
    fixture.u32(0x8900 + engine_native_parts::land_combat_air_entry::PLANE_COUNT, u32::MAX);
    fixture.u32(ATTACK + cland_combatant::ORGANISATION_LOSS_COUNT, 24);
    fixture.u64(ATTACK + cland_combatant::ORGANISATION_LOSS_WINDOW, 0x8a00);
    fixture.u64(0x8a00, i64::MIN as u64);
    fixture.u64(
        ATTACK + cland_combatant::AIR_STRENGTH_DAMAGE,
        i64::MAX as u64,
    );
    for unit in [UNIT_A, UNIT_B, UNIT_C] {
        fixture.u64(unit + cunit::LOCATION, PROVINCE_OBJECT as u64);
    }
    fixture
}

#[test]
fn details_preserve_native_order_bytes_damage_and_loss_history() {
    let fixture = detail_fixture();
    let memory = DetailMemory {
        fixture: &fixture,
        copies: 0.into(),
        bytes: 0.into(),
    };
    let source = Source {
        pointer: COMBAT,
        id: (0xf000_0001_u64 << 32) | TYPE_WORD as u64,
        sides: [ATTACK, DEFEND],
        attackers: Array {
            data: ATTACK_UNITS,
            count: 2,
        },
        defenders: Array {
            data: DEFEND_UNITS,
            count: 1,
        },
        provinces: Array {
            data: PROVINCES,
            count: 2,
        },
    };
    let mut arenas = Arenas::new();
    let mut view = arenas.view();
    let row = read_details(
        &memory,
        BASE,
        source,
        &Members::none(),
        &mut Commanders::default(),
        &mut view,
    )
    .unwrap();
    assert!(view.complete());
    let used = view.used.participants;
    assert_eq!(used, 3);
    assert_eq!(&view.participants[..used], &[u64::MAX - 4, 1, 62]);
    assert_eq!(&view.text[row.stored_terrain_name.range()], b"plain");
    assert_eq!(&view.text[row.sides[0].tactic_token.range()], &[b'a', 0xff]);
    assert_eq!(row.sides[0].tactic_counter_id, 9);
    assert_eq!(view.air[row.sides[0].air_counts.start], -1);
    assert_eq!(row.sides[0].damage[0], i64::MAX);
    assert_eq!(row.sides[0].organisation_loss[0], i64::MIN);
    assert_eq!(row.sides[1].organisation_loss_count, 0);

    let mut roles = Vec::new();
    assert_eq!(
        visit_units(
            &memory,
            BASE,
            source,
            &Members::none(),
            &mut [0; 1],
            |_, _, _, _, _, _| {}
        ),
        Ok(Visited::Scratch { required: 2 })
    );
    visit_units(
        &memory,
        BASE,
        source,
        &Members::none(),
        &mut [0; 2],
        |side, ordinal, _, handle, province, role| {
            roles.push((side, ordinal, handle, province, role))
        },
    )
    .unwrap();
    assert_eq!(
        roles,
        [
            (0, 0, u64::MAX - 4, 1, 1),
            (0, 1, 1, 1, 2),
            (1, 0, 62, 1, 1)
        ]
    );
}

#[test]
fn details_copy_committed_and_reserve_arrays_in_native_index_order() {
    const ORDERED: usize = 0x8c00;
    let mut fixture = detail_fixture();
    let read = |fixture: &Fixture| {
        let mut arenas = Arenas::new();
        let mut view = arenas.view();
        let row = read_details(
            &DetailMemory {
                fixture,
                copies: 0.into(),
                bytes: 0.into(),
            },
            BASE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
            &mut view,
        )
        .unwrap();
        let used = view.used.participants;
        let members = &view.participants;
        let span = |start: usize, len: usize| members[start..start + len].to_vec();
        (
            used,
            row.sides
                .map(|side| {
                    [
                        span(side.committed_start, side.committed_len),
                        span(side.reserve_start, side.reserve_len),
                    ]
                })
                .to_vec(),
        )
    };
    fixture.u64(ORDERED, UNIT_A as u64);
    fixture.u64(ORDERED + 8, UNIT_B as u64);
    fixture.array(ATTACK + cland_combatant::COMMITTED, ORDERED, 2);
    fixture.array(ATTACK + cland_combatant::RESERVES, ORDERED, 0);
    assert_eq!(
        read(&fixture),
        (3, vec![[vec![1, u64::MAX - 4], vec![]], [vec![62], vec![]]])
    );
    fixture.u64(ORDERED, UNIT_B as u64);
    fixture.u64(ORDERED + 8, UNIT_A as u64);
    fixture.array(ATTACK + cland_combatant::COMMITTED, ORDERED, 0);
    fixture.array(ATTACK + cland_combatant::RESERVES, ORDERED, 2);
    assert_eq!(
        read(&fixture),
        (3, vec![[vec![], vec![u64::MAX - 4, 1]], [vec![62], vec![]]])
    );
}

#[test]
fn modifiers_remain_independent_of_unreadable_tactic_and_roles_reject_overlap() {
    let mut fixture = detail_fixture();
    fixture.u64(ATTACK + cland_combatant::TACTIC, 1);
    let source = Source {
        pointer: COMBAT,
        id: (0xf000_0001_u64 << 32) | TYPE_WORD as u64,
        sides: [ATTACK, DEFEND],
        attackers: Array {
            data: ATTACK_UNITS,
            count: 2,
        },
        defenders: Array {
            data: DEFEND_UNITS,
            count: 1,
        },
        provinces: Array {
            data: PROVINCES,
            count: 2,
        },
    };
    assert!(read_modifiers_context(&fixture.memory(), BASE, source).is_ok());
    fixture.array(ATTACK + cland_combatant::RESERVES, ATTACK_UNITS, 1);
    assert_eq!(
        visit_units(
            &fixture.memory(),
            BASE,
            source,
            &Members::none(),
            &mut [0; 4],
            |_, _, _, _, _, _| {}
        )
        .unwrap_err(),
        "land role subsets overlap"
    );
}

fn detail_source() -> Source {
    Source {
        pointer: COMBAT,
        id: (0xf000_0001_u64 << 32) | TYPE_WORD as u64,
        sides: [ATTACK, DEFEND],
        attackers: Array {
            data: ATTACK_UNITS,
            count: 2,
        },
        defenders: Array {
            data: DEFEND_UNITS,
            count: 1,
        },
        provinces: Array {
            data: PROVINCES,
            count: 2,
        },
    }
}

const COMMANDER: usize = 0xb200;

fn commander_fixture() -> Fixture {
    use engine_native::{carmy_leader, corders_group, cunit_leader};
    use engine_native_parts::{cunit_leader_skill, cunit_leader_trait};
    let mut fixture = detail_fixture();
    fixture.0.resize(0x11000, 0);
    for (unit, group, leader, skill, category, id) in [
        (UNIT_B, 0x9000, 0xa000, 0xd000, 0, 41),
        (UNIT_A, 0x9200, 0xb200, 0xd200, 1, 42),
    ] {
        fixture.u64(unit + carmy::ORDERS_GROUP, group as u64);
        fixture.u64(group, (BASE + corders_group::VTABLE_RVA.0) as u64);
        fixture.u64(group + corders_group::LEADER, leader as u64);
        fixture.u64(leader, (BASE + carmy_leader::VTABLE_RVA.0) as u64);
        fixture.u64(
            leader + cunit_leader::TYPE,
            (id << 32) | cunit_leader::REFERENCE_TYPE as u64,
        );
        fixture.u64(leader + cunit_leader::SKILL, skill as u64);
        fixture.u32(leader + cunit_leader::CATEGORY, category);
        fixture.u64(skill, (BASE + cunit_leader_skill::VTABLE_RVA.0) as u64);
        fixture.u32(skill + cunit_leader_skill::LEVEL, 4);
    }
    const LEADER: usize = COMMANDER;
    const TRAIT: usize = 0xe100;
    fixture.u64(LEADER + cunit_leader::TOKEN, 0xff61);
    fixture.u64(LEADER + cunit_leader::TOKEN + 16, 2);
    fixture.u64(LEADER + cunit_leader::TOKEN + 24, 15);
    fixture.u64(LEADER + cunit_leader::DESCRIPTION_TOKEN + 24, 15);
    fixture.u64(
        LEADER + carmy_leader::FORMATION_REFERENCE,
        (19 << 32) | 4721,
    );
    for offset in [0xc0, 0xe0, 0x100] {
        fixture.u64(LEADER + offset + 24, 15);
    }
    fixture.array(0x11000 + 0xe8 + 0x18, 0x11180, 1);
    fixture.u64(0x11180, 0x11200);
    fixture.u64(0x11200, (BASE + cunit_leader_skill::VTABLE_RVA.0) as u64);
    fixture.u64(LEADER + cunit_leader::EXPERIENCE, i64::MAX as u64);
    fixture.array(LEADER + cunit_leader::TRAIT_DATA, 0xe000, 1);
    fixture.u64(0xe000, TRAIT as u64);
    fixture.u64(TRAIT, (BASE + cunit_leader_trait::VTABLE_RVA.0) as u64);
    fixture.u32(TRAIT + cunit_leader_trait::TOKEN_ID, 1234);
    fixture.u32(TRAIT + cunit_leader_trait::ORDINAL, 7);
    fixture.u32(TRAIT + cunit_leader_trait::KIND, 2);
    fixture.u32(TRAIT + cunit_leader_trait::REQUIRED_EXPERIENCE, 3);
    fixture.0[TRAIT + 0xa74 - STATE] = 1;
    fixture.u64(TRAIT + cunit_leader_trait::TOKEN, 0x7469617274);
    fixture.u64(TRAIT + cunit_leader_trait::TOKEN + 16, 5);
    fixture.u64(TRAIT + cunit_leader_trait::TOKEN + 24, 15);
    fixture.array(LEADER + cunit_leader::TRAIT_EXPERIENCE_DATA, 0xf000, 1);
    fixture.u64(0xf000, TRAIT as u64);
    fixture.u64(0xf008, 321000);
    fixture.array(
        LEADER + carmy_leader::MOVEMENT_MODIFIER + carmy_leader::MODIFIER_ACCUMULATOR,
        0x11400,
        3,
    );
    for (entry, (key, value)) in [(0x2b_u32, 7_i64), (0x5f, -12_345), (0x60, 67_891)]
        .into_iter()
        .enumerate()
    {
        fixture.u32(0x11400 + entry * 16, key);
        fixture.u64(0x11400 + entry * 16 + 8, value as u64);
    }
    fixture
}

#[test]
fn commander_trait_lists_grow_every_short_arena_to_the_exact_demand() {
    const TIMED: usize = 0xf100;
    const COUNT: usize = 100;
    let mut fixture = commander_fixture();
    fixture.array(
        COMMANDER + engine_native::cunit_leader::TIMED_TRAIT_DATA,
        TIMED,
        COUNT as u32,
    );
    for ordinal in 0..COUNT {
        let entry = TIMED + ordinal * engine_native::cunit_leader::TIMED_TRAIT_STRIDE;
        fixture.u64(entry, 0xe100);
        fixture.u32(entry + 8, (ordinal as i32 - 1) as u32);
    }
    let memory = DetailMemory {
        fixture: &fixture,
        copies: 0.into(),
        bytes: 0.into(),
    };
    let mut probe = Arenas::sized(DetailCounts::default());
    let mut view = probe.view();
    read_details(
        &memory,
        BASE,
        detail_source(),
        &Members::none(),
        &mut Commanders::default(),
        &mut view,
    )
    .unwrap();
    let demand = view.required;
    assert_eq!(demand.timed, COUNT);
    let mut exact = Arenas::sized(demand);
    let mut whole = exact.view();
    let row = read_details(
        &memory,
        BASE,
        detail_source(),
        &Members::none(),
        &mut Commanders::default(),
        &mut whole,
    )
    .unwrap();
    assert!(whole.complete());
    assert_eq!(row.sides[0].timed_traits_len, COUNT);
    let timed = &whole.timed[row.sides[0].timed_traits_start..];
    assert_eq!(timed[0].remaining_days, -1);
    assert_eq!(timed[COUNT - 1].remaining_days, COUNT as i32 - 2);
    assert_eq!(timed[0].definition_ordinal, 7);
    let shrink = |field: usize| {
        let mut counts = demand;
        let value = [
            &mut counts.participants,
            &mut counts.air,
            &mut counts.text,
            &mut counts.traits,
            &mut counts.timed,
            &mut counts.learning,
        ][field]
            .checked_sub(1)?;
        *[
            &mut counts.participants,
            &mut counts.air,
            &mut counts.text,
            &mut counts.traits,
            &mut counts.timed,
            &mut counts.learning,
        ][field] = value;
        Some(counts)
    };
    for field in 0..6 {
        let Some(counts) = shrink(field) else {
            continue;
        };
        let mut arenas = Arenas::sized(counts);
        let mut short = arenas.view();
        read_details(
            &memory,
            BASE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
            &mut short,
        )
        .unwrap();
        assert!(short.complete(), "{field}");
        assert_eq!(short.required, demand, "{field}");
    }
}

#[test]
fn worker_detail_arenas_grow_from_original_mod_sized_lists_before_publication() {
    use engine_native::cunit_leader;
    use engine_native_memory::walk::Buffer;
    let mut kept = Arenas::sized(DetailCounts::default());
    for count in [50, 75] {
        let mut fixture = commander_fixture();
        fixture.0.resize(0x30_0000, 0);
        fixture.array(COMMANDER + cunit_leader::TRAIT_DATA, 0x20_000, count as u32);
        fixture.array(
            COMMANDER + cunit_leader::TIMED_TRAIT_DATA,
            0x40_000,
            count as u32,
        );
        fixture.array(
            COMMANDER + cunit_leader::TRAIT_EXPERIENCE_DATA,
            0x60_000,
            count as u32,
        );
        fixture.array(
            ATTACK + cland_combatant::AIR_SUPPORT,
            0x80_000,
            count as u32,
        );
        fixture.array(ATTACK + cland_combatant::COMMITTED, 0xa0_000, count as u32);
        for index in 0..count {
            fixture.u64(0x20_000 + index * 8, 0xe100);
            fixture.u64(0x40_000 + index * cunit_leader::TIMED_TRAIT_STRIDE, 0xe100);
            fixture.u32(
                0x40_000 + index * cunit_leader::TIMED_TRAIT_STRIDE + 8,
                index as u32,
            );
            fixture.u64(0x60_000 + index * 16, 0xe100);
            fixture.u64(0x60_000 + index * 16 + 8, index as u64);
            fixture.u64(0x80_000 + index * 8, 0x8900);
            fixture.u64(0xa0_000 + index * 8, UNIT_A as u64);
        }
        let memory = DetailMemory {
            fixture: &fixture,
            copies: 0.into(),
            bytes: 0.into(),
        };
        let mut arenas = DetailArenas::with_buffers(
            Buffer::Growing(&mut kept.participants),
            Buffer::Growing(&mut kept.air),
            Buffer::Growing(&mut kept.text),
            Buffer::Growing(&mut kept.traits),
            Buffer::Growing(&mut kept.timed),
            Buffer::Growing(&mut kept.learning),
        );
        let row = read_details(
            &memory,
            BASE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
            &mut arenas,
        )
        .unwrap();
        assert!(arenas.complete());
        assert_eq!(arenas.used.participants, count + 2);
        assert_eq!(arenas.used.air, count);
        assert_eq!(arenas.used.traits, count);
        assert_eq!(arenas.used.timed, count);
        assert_eq!(arenas.used.learning, count);
        assert_eq!(row.sides[0].traits_len, count);
        assert_eq!(arenas.learning[count - 1].experience, count as i64 - 1);
        assert_eq!(arenas.timed[count - 1].remaining_days, count as i32 - 1);
        assert_eq!(
            &arenas.text[arenas.traits[count - 1].token_start..][..5],
            b"trait"
        );
    }
}

#[test]
fn detail_demand_uses_capture_census_and_rejects_unreadable_payloads() {
    let mut fixture = commander_fixture();
    let memory = DetailMemory {
        fixture: &fixture,
        copies: 0.into(),
        bytes: 0.into(),
    };
    let mut arenas = Arenas::new();
    let mut view = arenas.view();
    read_details(
        &memory,
        BASE,
        detail_source(),
        &Members::none(),
        &mut Commanders::default(),
        &mut view,
    )
    .unwrap();
    let demand = detail_demand(&memory, BASE, STATE).unwrap();
    assert_eq!(demand.rows, 1);
    assert_eq!(demand.census_participants, 3);
    assert_eq!(demand.arenas, view.required);

    let unreadable = 0x7fff_0000;
    for (header, count) in [
        (ATTACK + cland_combatant::AIR_SUPPORT, 5000),
        (COMMANDER + engine_native::cunit_leader::TIMED_TRAIT_DATA, 1500),
    ] {
        let mut changed = Fixture(fixture.0.clone());
        changed.array(header, unreadable, count);
        assert!(detail_demand(&changed.memory(), BASE, STATE).is_err());
    }
    fixture.u32(ATTACK + cland_combatant::AIR_SUPPORT + 12, 5001);
    assert!(detail_demand(&fixture.memory(), BASE, STATE).is_err());
}

#[test]
fn detail_demand_discards_changed_array_and_string_headers() {
    let mut fixture = commander_fixture();
    fixture.array(COMMANDER + engine_native::cunit_leader::TIMED_TRAIT_DATA, 0xf100, 1);
    fixture.u64(0xf100, 0xe100);
    let memory = fixture.memory();
    let observed = Observed::new(&memory, &[]);
    detail_demand(&observed, BASE, STATE).unwrap();
    let reads = observed.reads.borrow();
    for (address, width, byte) in [
        (ATTACK + cland_combatant::UNITS, 16, 12),
        (ATTACK + cland_combatant::COMMITTED, 16, 12),
        (ATTACK + cland_combatant::RESERVES, 16, 12),
        (ATTACK + cland_combatant::AIR_SUPPORT, 16, 12),
        (COMMANDER + engine_native::cunit_leader::TRAIT_DATA, 16, 12),
        (COMMANDER + engine_native::cunit_leader::TIMED_TRAIT_DATA, 16, 12),
        (
            COMMANDER + engine_native::cunit_leader::TRAIT_EXPERIENCE_DATA,
            16,
            12,
        ),
        (COMMANDER + engine_native::cunit_leader::TOKEN, 32, 16),
        (0x8200 + engine_native_parts::cterrain_type::NAME, 32, 16),
        (STATE + cgame_state::COMBATS + 8, 16, 12),
    ] {
        let ordinal = reads
            .iter()
            .rposition(|&(at, len)| (at, len) == (address, width))
            .unwrap();
        let mut torn = Observed::new(&memory, &[]);
        torn.change = Some((ordinal, byte));
        assert!(detail_demand(&torn, BASE, STATE).is_err(), "{address:#x}");
    }
}

#[test]
fn detail_demand_counts_combat_participant_and_arena_occurrences() {
    const LARGE_REGISTRY: usize = 0x12000;
    const LARGE_COMBATS: usize = 0x20000;
    const COMBAT_STRIDE: usize = 0x1000;
    for count in [15_usize, 50] {
        let mut fixture = detail_fixture();
        fixture.0.resize(LARGE_COMBATS + count * COMBAT_STRIDE, 0);
        fixture.array(
            STATE + cgame_state::COMBATS + 8,
            LARGE_REGISTRY,
            count as u32,
        );
        let combat = fixture.0[COMBAT - STATE..COMBAT - STATE + 0xe0].to_vec();
        let attacker = fixture.0[ATTACK - STATE..ATTACK - STATE + 0x1d0].to_vec();
        let defender = fixture.0[DEFEND - STATE..DEFEND - STATE + 0x1d0].to_vec();
        for index in 0..count {
            let pointer = LARGE_COMBATS + index * COMBAT_STRIDE;
            let attack = pointer + 0x100;
            let defend = pointer + 0x300;
            for (address, bytes) in [(pointer, &combat), (attack, &attacker), (defend, &defender)] {
                let start = address - STATE;
                fixture.0[start..start + bytes.len()].copy_from_slice(bytes);
            }
            fixture.u64(LARGE_REGISTRY + index * 8, pointer as u64);
            fixture.u64(
                pointer + HANDLE,
                ((index as u64 + 1) << 32) | TYPE_WORD as u64,
            );
            fixture.u64(pointer + ATTACKER, attack as u64);
            fixture.u64(pointer + DEFENDER, defend as u64);
            fixture.u64(attack + cland_combatant::COMBAT, pointer as u64);
            fixture.u64(defend + cland_combatant::COMBAT, pointer as u64);
        }
        let demand = detail_demand(&fixture.memory(), BASE, STATE).unwrap();
        assert_eq!(demand.rows, count);
        assert_eq!(demand.census_participants, 3 * count);
        assert_eq!(demand.arenas.participants, 3 * count);
        assert_eq!(demand.arenas.air, count);
        assert_eq!(demand.arenas.text, 9 * count);
    }
}

#[test]
fn commander_selection_preserves_equal_skill_category_tie_and_hourly_traits() {
    let fixture = commander_fixture();
    let memory = DetailMemory {
        fixture: &fixture,
        copies: 0.into(),
        bytes: 0.into(),
    };
    let mut arenas = Arenas::new();
    let mut traits = arenas.view();
    let row = read_details(
        &memory,
        BASE,
        detail_source(),
        &Members::none(),
        &mut Commanders::default(),
        &mut traits,
    )
    .unwrap();
    assert!(traits.complete());
    let side = &row.sides[0];
    assert_eq!(side.commander >> 32, 42);
    assert_eq!(side.commander_skill, 4);
    assert_eq!(side.commander_category, 1);
    assert_eq!(side.commander_experience, i64::MAX);
    assert_eq!(
        &traits.text[side.commander_name_token.range()],
        &[b'a', 0xff]
    );
    assert_eq!(side.commander_description_token.len, 0);
    assert_eq!(side.commander_formation, (19 << 32) | 4721);
    assert_eq!(side.traits_len, 1);
    let assigned = traits.traits[side.traits_start];
    assert_eq!(assigned.definition_ordinal, 7);
    assert_eq!(assigned.relevant, 1);
    assert_eq!(
        &traits.text[assigned.token_start..assigned.token_start + assigned.token_len],
        b"trait"
    );
    let learning = traits.learning[side.learning_traits.start];
    assert_eq!(side.learning_traits.len, 1);
    assert_eq!(learning.required_experience, 3);
    assert_eq!(learning.experience, 321000);
    assert_eq!(
        side.next_skill.map(|next| (next.present, next.valid)),
        Some((1, 0))
    );
    assert_eq!(side.learning_threshold, Some(5_000));
    assert_eq!(side.skill_modifiers, Some([-12_345, 67_891]));
    assert_eq!(side.side_input_refusals, [None; 3]);
    let defender = &row.sides[1];
    assert_eq!(
        (
            defender.next_skill,
            defender.learning_threshold,
            defender.skill_modifiers
        ),
        (None, None, None)
    );
    assert_eq!(defender.side_input_refusals, [None; 3]);
}

#[test]
fn unreadable_optional_side_inputs_refuse_only_themselves() {
    let fixture = commander_fixture();
    let mut arenas = Arenas::new();
    let mut view = arenas.view();
    let row = read_details(
        &fixture.memory(),
        BASE,
        detail_source(),
        &Members::none(),
        &mut Commanders::default(),
        &mut view,
    )
    .unwrap();
    let side = &row.sides[0];
    assert_eq!(side.commander >> 32, 42);
    assert_eq!(
        (
            side.next_skill,
            side.learning_threshold,
            side.skill_modifiers
        ),
        (None, None, None)
    );
    assert!(side.side_input_refusals.iter().all(Option::is_some));
    assert_eq!(side.commander_formation, (19 << 32) | 4721);
    assert_eq!(side.learning_traits.len, 1);
}

#[test]
fn selected_commander_texts_keep_native_length_and_name_their_field() {
    use engine_native::cunit_leader::{DESCRIPTION_TOKEN, TOKEN};
    const HEAP: usize = 0x12400;
    let mut fixture = commander_fixture();
    fixture.0.resize(0x13000, 0);
    fixture.u64(COMMANDER + TOKEN, HEAP as u64);
    fixture.u64(COMMANDER + TOKEN + 16, 300);
    fixture.u64(COMMANDER + TOKEN + 24, 300);
    fixture.0[HEAP - STATE..HEAP - STATE + 300].fill(b'k');
    fixture.u64(COMMANDER + DESCRIPTION_TOKEN, (HEAP + 0x200) as u64);
    fixture.u64(COMMANDER + DESCRIPTION_TOKEN + 16, 0);
    fixture.u64(COMMANDER + DESCRIPTION_TOKEN + 24, 31);
    let read = |fixture: &Fixture, text: usize| {
        let mut arenas = Arenas::sized(DetailCounts {
            participants: 8,
            air: 8,
            text,
            traits: 8,
            timed: 8,
            learning: 8,
        });
        let mut view = arenas.view();
        let details = read_details(
            &DetailMemory {
                fixture,
                copies: 0.into(),
                bytes: 0.into(),
            },
            BASE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
            &mut view,
        )
        .map(|row| (view.complete(), row.sides.into_iter().next().unwrap()));
        let (used, required) = (view.used.text, view.required.text);
        (details, used, required, arenas.text)
    };
    const TERRAIN: usize = 5;
    const OTHER: usize = TERRAIN + 10 + 4;

    let (details, used, required, text) = read(&fixture, 512);
    let (complete, side) = details.unwrap();
    assert!(complete);
    let name = Span {
        start: TERRAIN,
        len: 300,
    };
    assert_eq!(side.commander_name_token, name);
    assert!(text[name.range()].iter().all(|&byte| byte == b'k'));
    assert_eq!(
        side.commander_description_token,
        Span {
            start: name.start + 300,
            len: 0
        }
    );
    assert_eq!((used, required), (300 + OTHER, 300 + OTHER));

    let mut empty = fixture.0.clone();
    let at = COMMANDER + TOKEN - STATE;
    empty[at..at + 32].fill(0);
    empty[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
    let (details, _, _, _) = read(&Fixture(empty), 64);
    let (complete, side) = details.unwrap();
    assert!(complete);
    assert_eq!(
        side.commander_name_token,
        Span {
            start: TERRAIN,
            len: 0
        }
    );

    for (at, value, reason) in [
        (
            DESCRIPTION_TOKEN + 24,
            7,
            "land combat commander description token header invalid",
        ),
        (
            TOKEN,
            0,
            "land combat commander name token pointer unreadable or null",
        ),
        (
            DESCRIPTION_TOKEN,
            0,
            "land combat commander description token pointer unreadable or null",
        ),
    ] {
        let mut broken = Fixture(fixture.0.clone());
        broken.u64(COMMANDER + at, value);
        assert_eq!(read(&broken, 512).0.unwrap_err(), reason);
    }
    let mut unterminated = Fixture(fixture.0.clone());
    unterminated.0[HEAP + 300 - STATE] = b'x';
    assert_eq!(
        read(&unterminated, 512).0.unwrap_err(),
        "land combat commander name token terminator unreadable or invalid"
    );
}

#[test]
fn tactic_context_records_a_side_whose_country_set_emptied() {
    use engine_native::ccountry;
    use crate::land_tactic_inputs as tactic;
    const COUNTRY: usize = 0x8b00;
    const COUNTRIES: usize = 0x8c00;
    const LINKS: usize = 0x8c40;
    const NODE: usize = 0x8f00;
    struct Defines<'a>(DetailMemory<'a>);
    impl Memory for Defines<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if [0x033e_0d88, 0x033e_0e00, 0x033e_0e88].contains(&(address.wrapping_sub(BASE)))
                && destination.len() == 8
            {
                destination.copy_from_slice(&50_000_i64.to_le_bytes());
                return true;
            }
            self.0.copy(address, destination)
        }
    }
    let mut fixture = detail_fixture();
    fixture.0.resize(0x9000, 0);
    fixture.array(STATE + cgame_state::COUNTRIES, COUNTRIES, 2);
    fixture.u64(COUNTRIES + 8, COUNTRY as u64);
    fixture.array(STATE + cgame_state::COUNTRY_LINKS, LINKS, 2);
    fixture.u32(LINKS + 4, 1);
    fixture.u64(COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
    fixture.u32(COUNTRY + ccountry::ID, 1);
    let capture = |fixture: &Fixture| {
        let memory = Defines(DetailMemory {
            fixture,
            copies: 0.into(),
            bytes: 0.into(),
        });
        tactic::capture_context(
            &memory,
            BASE,
            STATE,
            detail_source(),
            &Members::none(),
            &mut Commanders::default(),
        )
    };
    let empty = capture(&fixture);
    assert_eq!(empty.outcome, Outcome::Rows { len: 1 });
    assert_eq!(empty.row.preferred_tactic_ids, [[0; 3]; 2]);
    assert_eq!(empty.row.country_weight_present, [0; 2]);
    fixture.u64(ATTACK + cland_combatant::COUNTRIES, NODE as u64);
    assert_eq!(
        capture(&fixture).outcome,
        Outcome::Unavailable {
            reason: "tactic side country scope is null"
        }
    );
}

fn country_set(fixture: &mut Fixture, side: usize, offset: usize, nodes: usize, tags: &[u32]) {
    let node = |index: usize| nodes + index * 0x18;
    for (index, &tag) in tags.iter().enumerate() {
        fixture.u32(node(index), tag);
        fixture.u64(
            node(index) + 8,
            if index == 0 {
                0
            } else {
                node(index - 1) as u64
            },
        );
        let next = if index + 1 == tags.len() {
            0
        } else {
            node(index + 1)
        };
        fixture.u64(node(index) + 0x10, next as u64);
    }
    let (head, tail) = match tags.len() {
        0 => (0, 0),
        len => (node(0), node(len - 1)),
    };
    fixture.u64(side + offset, head as u64);
    fixture.u64(side + offset + 8, tail as u64);
    fixture.u32(side + offset + 0x10, tags.len() as u32);
}

fn country_sets(
    fixture: &Fixture,
    max_len: usize,
    tags: &mut Vec<u32>,
) -> Result<(CountrySetsRow, usize), CountrySetError> {
    let mut result = None;
    walk_with(
        &fixture.memory(),
        BASE,
        STATE,
        &mut vec![Row::default(); 2],
        &mut vec![0; 8],
        &mut vec![0; 8],
        |source| {
            result = Some(read_country_sets_buffer(&fixture.memory(), BASE, source, max_len, engine_native_memory::walk::Buffer::Growing(&mut *tags)));
        },
    );
    result.expect("one land combat")
}

#[test]
fn country_sets_keep_native_order_raw_tags_and_empty_sets() {
    let mut fixture = Fixture::new();
    country_set(
        &mut fixture,
        ATTACK,
        cland_combatant::COUNTRIES,
        0x8400,
        &[7, 0, 3],
    );
    country_set(
        &mut fixture,
        ATTACK,
        cland_combatant::SECONDARY_COUNTRIES,
        0x8500,
        &[9],
    );
    country_set(
        &mut fixture,
        DEFEND,
        cland_combatant::COUNTRIES,
        0x8600,
        &[4, 2],
    );
    fixture.u32(COMBAT + INDICATOR_FRAME_HIGH, 0x0102_0304);
    fixture.u32(ATTACK + cland_combatant::FLAGS, 0x4000);
    fixture.u32(DEFEND + cland_combatant::FLAGS, 0x800);
    let mut tags = vec![u32::MAX; 8];
    let (row, used) = country_sets(&fixture, 4, &mut tags).unwrap();
    assert_eq!((row.indicator, row.flags), (4, [0x4000, 0x800]));
    assert_eq!(row.lens, [3, 1, 2, 0]);
    assert_eq!(used, 6);
    assert_eq!(&tags[..6], &[7, 0, 3, 9, 4, 2]);
    assert!(matches!(
        country_sets(&fixture, 2, &mut tags),
        Err(CountrySetError::Unavailable(_))
    ));
}

#[test]
fn country_sets_refuse_broken_links_and_keep_what_the_game_stored() {
    let refused = |edit: &dyn Fn(&mut Fixture)| {
        let mut fixture = Fixture::new();
        country_set(
            &mut fixture,
            ATTACK,
            cland_combatant::COUNTRIES,
            0x8400,
            &[7, 3],
        );
        country_set(
            &mut fixture,
            DEFEND,
            cland_combatant::SECONDARY_COUNTRIES,
            0x8500,
            &[5],
        );
        edit(&mut fixture);
        matches!(
            country_sets(&fixture, 8, &mut vec![0; 8]),
            Err(CountrySetError::Unavailable(_))
        )
    };
    assert!(!refused(&|_| {}));
    assert!(refused(
        &|f| f.u64(ATTACK + cland_combatant::COUNTRIES + 8, 0x8400)
    ));
    assert!(refused(&|f| f.u64(0x8418 + 0x10, 0x8400)));
    assert!(!refused(&|f| f.u32(0x8418, 7)));
    assert!(!refused(&|f| f.u32(0x8418, 0)));
    assert!(!refused(&|f| f.u32(0x8500, 0x8000_0000)));
    assert!(refused(
        &|f| f.u32(DEFEND + cland_combatant::COUNTRIES + 0x10, 1)
    ));
}

#[test]
fn border_war_has_native_dynamic_type_and_subclass_sides() {
    let mut fixture = Fixture::new();
    fixture.u64(COMBAT, (BASE + cland_border_war_combat::VTABLE_RVA.0) as u64);
    fixture.u64(
        COMBAT + BODY,
        (BASE + cland_border_war_combat::BODY_VTABLE_RVA.0) as u64,
    );
    fixture.u64(COMBAT + HANDLE, u64::MAX);
    for side in [ATTACK, DEFEND] {
        fixture.u64(side, (BASE + cland_border_war_combatant::VTABLE_RVA.0) as u64);
    }
    let (captured, rows, _) = fixture.capture();
    assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].id, u64::MAX);
    fixture.u64(DEFEND, (BASE + cland_combatant::VTABLE_RVA.0) as u64);
    assert!(matches!(
        fixture.capture().0.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn empty_roster_and_empty_sides_are_readable_but_corrupt_headers_are_not() {
    let mut fixture = Fixture::new();
    fixture.array(ATTACK + cland_combatant::UNITS, 0, 0);
    fixture.array(DEFEND + cland_combatant::UNITS, 0, 0);
    let (captured, rows, _) = fixture.capture();
    assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.participants_len, 0);
    assert_eq!(rows[0].attacker_len, 0);
    assert_eq!(rows[0].defender_len, 0);
    fixture.u32(ATTACK + cland_combatant::UNIT_COUNT, 1);
    assert!(matches!(
        fixture.capture().0.outcome,
        Outcome::Unavailable { .. }
    ));
    fixture.array(STATE + cgame_state::COMBATS + 8, 0, 0);
    assert_eq!(fixture.capture().0.outcome, Outcome::Rows { len: 0 });
    fixture.u32(STATE + cgame_state::COMBATS + 8 + 8, u32::MAX);
    assert!(matches!(
        fixture.capture().0.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn bad_objects_fail_the_table_instead_of_producing_a_partial_census() {
    for (address, value) in [
        (REGISTRY + 8, 0),
        (NAVAL, 1),
        (COMBAT + BODY, 1),
        (COMBAT + HANDLE, 1),
        (COMBAT + DEFENDER, ATTACK as u64),
        (DEFEND + cland_combatant::COMBAT, COMBAT as u64 + 8),
        (DEFEND + cland_combatant::IS_ATTACKER, 1),
        (DEFEND_UNITS, 0),
        (UNIT_C + carmy::SECONDARY_BASE, 1),
        (UNIT_C + cunit::HANDLE, 0),
        (PROVINCES + 8, PROVINCE_OBJECT as u64 + 8),
    ] {
        let mut fixture = Fixture::new();
        fixture.u64(address, value);
        let captured = fixture.capture().0;
        assert!(matches!(captured.outcome, Outcome::Unavailable { .. }));
        assert_eq!(captured.participants_len, 0);
    }
    let fixture = Fixture::new();
    assert!(matches!(
        walk(
            &fixture.memory(),
            BASE,
            usize::MAX,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new()
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn border_marker_inputs_keep_stored_country_state_event_bytes_and_inactive_skip() {
    let mut fixture = commander_fixture();
    let mut source = detail_source();
    source.attackers.data = 0x10100;
    source.defenders.data = 0x10120;
    fixture.array(ATTACK + cland_combatant::UNITS, source.attackers.data, 2);
    fixture.array(DEFEND + cland_combatant::UNITS, source.defenders.data, 1);
    fixture.array(
        ATTACK + cland_combatant::COMMITTED,
        source.attackers.data,
        1,
    );
    fixture.array(
        ATTACK + cland_combatant::RESERVES,
        source.attackers.data + 8,
        1,
    );
    fixture.array(
        DEFEND + cland_combatant::COMMITTED,
        source.defenders.data,
        1,
    );
    fixture.u64(source.attackers.data, UNIT_B as u64);
    fixture.u64(source.attackers.data + 8, UNIT_A as u64);
    fixture.u64(source.defenders.data, UNIT_C as u64);
    let mut arenas = Arenas::new();
    let row = read_details(
        &fixture.memory(),
        BASE,
        source,
        &Members::none(),
        &mut Commanders::default(),
        &mut arenas.view(),
    )
    .unwrap();
    assert!(row.border_war.is_none());
    fixture.u64(COMBAT, (BASE + cland_border_war_combat::VTABLE_RVA.0) as u64);
    fixture.u64(
        COMBAT + BODY,
        (BASE + cland_border_war_combat::BODY_VTABLE_RVA.0) as u64,
    );
    fixture.0[COMBAT + 0xf0 - STATE] = 9;
    fixture.u64(
        COMBAT + cland_border_war_combat::COMBAT_WIDTH,
        (-1_i64) as u64,
    );
    for (side, country, state) in [(ATTACK, -7_i32, 19), (DEFEND, 11, 21)] {
        fixture.u64(side, (BASE + cland_border_war_combatant::VTABLE_RVA.0) as u64);
        fixture.u32(side + 0x4c0, country as u32);
        let pointer = 0x10300 + if side == ATTACK { 0 } else { 0x100 };
        fixture.u64(side + 0x4c8, pointer as u64);
        fixture.u64(pointer, (BASE + engine_native::cstate::VTABLE_RVA.0) as u64);
        fixture.u32(pointer + engine_native::cstate::ID, state);
        for (offset, id) in [
            (0x500, b"event.1".as_slice()),
            (0x520, b"event.2".as_slice()),
        ] {
            let start = side + offset - STATE;
            fixture.0[start..start + 32].fill(0);
            fixture.0[start..start + id.len()].copy_from_slice(id);
            fixture.u64(side + offset + 16, id.len() as u64);
            fixture.u64(side + offset + 24, 15);
        }
    }
    let mut arenas = Arenas::new();
    let mut view = arenas.view();
    let row = read_details(
        &fixture.memory(),
        BASE,
        source,
        &Members::none(),
        &mut Commanders::default(),
        &mut view,
    )
    .unwrap();
    let border = row.border_war.unwrap();
    let counts = view.required;
    assert_eq!(
        detail_demand(&fixture.memory(), BASE, STATE)
            .unwrap()
            .arenas,
        counts
    );
    assert_eq!(border.transfer_state, 9);
    assert_eq!(border.combat_width, -1);
    assert_eq!(border.sides[0].country, -7);
    assert_eq!(border.sides[1].state, 21);
    assert_eq!(&arenas.text[border.sides[0].win_event.range()], b"event.1");
    for side in [ATTACK, DEFEND] {
        for offset in [0x500, 0x520] {
            let start = side + offset - STATE;
            fixture.0[start..start + 32].fill(0);
            fixture.u64(side + offset + 24, 15);
        }
    }
    let border = read_details(
        &fixture.memory(),
        BASE,
        source,
        &Members::none(),
        &mut Commanders::default(),
        &mut Arenas::new().view(),
    )
    .unwrap()
    .border_war
    .unwrap();
    assert!(
        border
            .sides
            .iter()
            .all(|side| { side.win_event.len == 0 && side.lose_event.len == 0 })
    );
    assert_eq!(
        detail_demand(&fixture.memory(), BASE, STATE)
            .unwrap()
            .arenas
            .text,
        counts.text - 4 * b"event.1".len()
    );
    fixture.u64(ATTACK + 0x4c8, 1);
    assert!(
        read_details(
            &fixture.memory(),
            BASE,
            source,
            &Members::none(),
            &mut Commanders::default(),
            &mut Arenas::new().view()
        )
        .is_err()
    );
}

#[test]
fn movement_combat_reads_the_movement_word_and_attacking_units_by_combat_kind() {
    const BASE: usize = 0x1000;
    const COMBAT: usize = 0x2000;
    const SIDE: usize = 0x2400;
    const UNITS: usize = 0x2800;
    let mut bytes = vec![0_u8; 0x4000];
    let put = |bytes: &mut [u8], at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    put(&mut bytes, COMBAT + ATTACKER, SIDE as u64);
    put(&mut bytes, COMBAT + MOVEMENT_MODIFIER, (-12_345_i64) as u64);
    put(
        &mut bytes,
        SIDE,
        (BASE + cland_combatant::VTABLE_RVA.0) as u64,
    );
    put(&mut bytes, SIDE + cland_combatant::COMBAT, COMBAT as u64);
    put(&mut bytes, SIDE + cland_combatant::UNITS, UNITS as u64);
    bytes[SIDE + cland_combatant::UNITS + 8..SIDE + cland_combatant::UNITS + 12]
        .copy_from_slice(&2_u32.to_le_bytes());
    bytes[SIDE + cland_combatant::UNITS + 12..SIDE + cland_combatant::UNITS + 16]
        .copy_from_slice(&2_u32.to_le_bytes());
    put(&mut bytes, UNITS, 0xa100);
    put(&mut bytes, UNITS + 8, 0xa200);
    let read = |bytes: &[u8], vtable: usize| {
        let mut bytes = bytes.to_vec();
        put(&mut bytes, COMBAT, (BASE + vtable) as u64);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut attackers = vec![1];
        let movement = movement_combat(&memory, BASE, COMBAT, &mut attackers);
        (movement, attackers)
    };
    for vtable in [VTABLE_RVA, cland_border_war_combat::VTABLE_RVA] {
        assert_eq!(
            read(&bytes, vtable.0),
            (Ok(Some(-12_345)), vec![1, 0xa100, 0xa200])
        );
    }
    assert_eq!(
        read(&bytes, cnaval_combat::VTABLE_RVA.0),
        (Ok(None), vec![1, 0xa100, 0xa200])
    );
    assert_eq!(
        read(&bytes, 0x10),
        (Err("movement combat identity mismatch"), vec![1])
    );
    put(&mut bytes, SIDE + cland_combatant::COMBAT, 0);
    assert_eq!(
        read(&bytes, VTABLE_RVA.0),
        (Err("movement combat side identity mismatch"), vec![1])
    );
}
