use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{
    carmy,
    carmy_leader,
    ccountry,
    cgame_state,
    cland_combat as combat,
    cland_combatant as side,
    corders_group,
    cprovince,
    cunit,
    cunit_leader,
};
use engine_native_parts::{
    cland_border_war_combat,
    cland_border_war_combatant,
    cunit_leader_skill,
};
use engine_sites::Rva;

pub fn word_demand(memory: &impl Memory, base: usize, state: usize) -> Result<usize, &'static str> {
    let mut words = 0_usize;
    let mut add = |count| {
        words = words
            .checked_add(count)
            .ok_or("land event sizing overflow")?;
        Ok::<_, &'static str>(())
    };
    let registry_at = state
        .checked_add(cgame_state::COMBATS + 8)
        .ok_or("land event sizing address overflow")?;
    let registry =
        Array::read(memory, registry_at, 8).ok_or("combat registry unreadable or invalid")?;
    engine_native_memory::cpdx_array::pointers(
        memory,
        registry,
        "land event sizing combat pointer unreadable",
        |_, pointer| {
            let vtable = field::<u64>(memory, pointer, 0)?;
            if Some(vtable)
                == engine_sites::at(base, engine_native::cnaval_combat::VTABLE_RVA).map(|p| p as u64)
            {
                return Ok(());
            }
            let header = CombatHeader::read(memory, base, pointer)?;
            let verdict = header.border_war().then_some(([0; 3], [255; 2]));
            let result = ResultPlan::read_header(memory, base, header, verdict)?;
            add(result.words)?;
            add(result.participants().words)?;
            add(result.exit_words())?;
            add(result.hit_words())?;
            add(6)?;
            if verdict.is_some() {
                add(8)?;
            }
            add(LossesPlan::read(memory, base, pointer)?.words)?;
            add(HistoryPlan::read(memory, base, pointer)?.words)
        },
    )?;
    if Array::read(memory, registry_at, 8) != Some(registry) {
        return Err("land event sizing combat registry changed");
    }
    let countries = ccountry::Reader::new(memory, base, state)?;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let roster_at = country
            .pointer
            .checked_add(ccountry::DIVISIONS)
            .ok_or("land event sizing division address overflow")?;
        let divisions =
            Array::read(memory, roster_at, 8).ok_or("land event sizing division roster invalid")?;
        engine_native_memory::cpdx_array::pointers(
            memory,
            divisions,
            "land event sizing division pointer unreadable",
            |_, army| {
                identity(memory, base, army, carmy::VTABLE_RVA)?;
                let combats_at = army
                    .checked_add(cunit::COMBAT_REFERENCE_DATA)
                    .ok_or("destroyed combat address overflow")?;
                let combats = Array::read(memory, combats_at, 8)
                    .ok_or("destroyed division combat header invalid")?;
                add(destruction_words(combats))?;
                add(4)
            },
        )?;
        if Array::read(memory, roster_at, 8) != Some(divisions) {
            return Err("land event sizing division roster changed");
        }
    }
    Ok(words)
}

fn destruction_words(combats: Array) -> usize {
    10 + combats.count as usize
}

pub const FINALIZE_RVA: Rva = Rva(0x1349260);
pub const DISPATCH_RVA: Rva = Rva(0x13491a0);
pub const DISPATCH_LEADER_RETURNS: [Rva; 2] = [Rva(0x13491bc), Rva(0x13491da)];
pub const DISPATCH_LEADER_CALLS: [Rva; 2] = [Rva(0x13491b6), Rva(0x13491d4)];
pub const DISPATCH_LEADER_CALL_SENTINEL: [u8; 6] = [0xff, 0x90, 0xe8, 0, 0, 0];
pub const DISPATCH_LEADER_VTABLE_SLOT: usize = 0xe8;
pub const DISPATCH_LEADER_CALL_NAMES: [&str; 2] = [
    "DispatchLeaderOnActions.loser_leader",
    "DispatchLeaderOnActions.winner_leader",
];

const COMBAT_HEADER_END: usize = combat::DURATION + size_of::<i32>();
const COMBAT_HEADER_BYTES: usize = COMBAT_HEADER_END - combat::HANDLE;
const SIDE_POINTER_BYTES: usize = combat::DEFENDER + size_of::<u64>() - combat::ATTACKER;
const SET_BYTES: usize = 2 * size_of::<u64>() + size_of::<i32>();
const HISTORY_SIDE_BYTES: usize = side::COUNTRIES + SET_BYTES;
const RESULT_SIDE_SPANS: [(usize, usize); 3] = [
    (0, side::SECONDARY_COUNTRIES + SET_BYTES),
    (side::PROGRESS, size_of::<i64>()),
    (side::OUTCOME_FLAGS, size_of::<u8>()),
];
const RESULT_SIDE_FIELDS: [engine_native_memory::memory::RecordField; 3] =
    engine_native_memory::memory::packed_fields(RESULT_SIDE_SPANS);
const RESULT_SIDE_BYTES: usize = engine_native_memory::memory::packed_stride(RESULT_SIDE_SPANS);
const RESULT_PROGRESS: usize = RESULT_SIDE_FIELDS[1].destination_offset;
const RESULT_FLAGS: usize = RESULT_SIDE_FIELDS[2].destination_offset;
const _: () = {
    assert!(combat::HANDLE + size_of::<u64>() <= COMBAT_HEADER_END);
    assert!(combat::ATTACKER >= combat::HANDLE);
    assert!(combat::ATTACKER + size_of::<u64>() <= COMBAT_HEADER_END);
    assert!(combat::DEFENDER >= combat::HANDLE);
    assert!(combat::DEFENDER >= combat::ATTACKER);
    assert!(SIDE_POINTER_BYTES >= 2 * size_of::<u64>());
    assert!(combat::DEFENDER + size_of::<u64>() <= COMBAT_HEADER_END);
    assert!(combat::PROVINCE >= combat::HANDLE);
    assert!(combat::PROVINCE + size_of::<u64>() <= COMBAT_HEADER_END);
    assert!(combat::DAY >= combat::HANDLE);
    assert!(combat::DAY + size_of::<i32>() <= COMBAT_HEADER_END);
    assert!(combat::DURATION >= combat::HANDLE);
    assert!(side::COMBAT + size_of::<u64>() <= HISTORY_SIDE_BYTES);
    assert!(side::COUNTRIES + SET_BYTES <= RESULT_SIDE_SPANS[0].1);
    assert!(side::UNITS + 2 * size_of::<u64>() <= RESULT_SIDE_SPANS[0].1);
    assert!(RESULT_PROGRESS == RESULT_SIDE_SPANS[0].1);
    assert!(RESULT_FLAGS == RESULT_PROGRESS + size_of::<i64>());
    assert!(RESULT_SIDE_BYTES == RESULT_FLAGS + size_of::<u8>());
};

#[derive(Clone, Copy)]
pub struct CombatHeader {
    pointer: usize,
    bytes: [u8; COMBAT_HEADER_BYTES],
    border: bool,
}
impl CombatHeader {
    fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Self, &'static str> {
        let vtable = field::<u64>(memory, pointer, 0)? as usize;
        let classes = [cland_border_war_combat::VTABLE_RVA, combat::VTABLE_RVA];
        let border = match engine_sites::which(base, vtable, &classes) {
            Some(cland_border_war_combat::VTABLE_RVA) => true,
            Some(_) => false,
            None => return Err("land combat class unsupported"),
        };
        Ok(Self {
            pointer,
            bytes: field(memory, pointer, combat::HANDLE)?,
            border,
        })
    }
    fn qword(&self, offset: usize) -> u64 {
        let at = offset - combat::HANDLE;
        u64::from_le_bytes(self.bytes[at..at + size_of::<u64>()].try_into().unwrap())
    }
    fn dword(&self, offset: usize) -> i32 {
        let at = offset - combat::HANDLE;
        i32::from_le_bytes(self.bytes[at..at + size_of::<i32>()].try_into().unwrap())
    }
    pub fn handle(&self) -> u64 {
        self.qword(combat::HANDLE)
    }
    pub fn sides(&self) -> [usize; 2] {
        [
            self.qword(combat::ATTACKER) as usize,
            self.qword(combat::DEFENDER) as usize,
        ]
    }
    pub fn border_war(&self) -> bool {
        self.border
    }
}

pub struct LossesPlan {
    combat: u64,
    pointers: [usize; 2],
    sides: [crate::cstats_observer::LossPlan; 2],
    pub words: usize,
}
impl LossesPlan {
    pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Self, &'static str> {
        let header = CombatHeader::read(memory, base, pointer)?;
        let pointers = header.sides();
        if pointers[0] == pointers[1] {
            return Err("land loss sides alias");
        }
        let expected = if header.border {
            cland_border_war_combatant::VTABLE_RVA
        } else {
            side::VTABLE_RVA
        };
        let mut sides = [None; 2];
        for (n, at) in pointers.into_iter().enumerate() {
            identity(memory, base, at, expected)?;
            if field::<u64>(memory, at, side::COMBAT)? != pointer as u64 {
                return Err("land loss side backlink differs");
            }
            sides[n] = Some(crate::cstats_observer::LossPlan::read(memory, base, at)?);
        }
        let sides = sides.map(Option::unwrap);
        let words = sides[0]
            .words()?
            .checked_add(sides[1].words()?)
            .and_then(|n| n.checked_add(3))
            .ok_or("land loss event words overflow")?;
        Ok(Self {
            combat: header.handle(),
            pointers,
            sides,
            words,
        })
    }
    pub fn observer_sources(&self) -> (u64, [usize; 2]) {
        (self.combat, self.pointers)
    }
    pub fn copy(
        &self,
        memory: &impl Memory,
        base: usize,
        mut put: impl FnMut(u64),
    ) -> Result<(), &'static str> {
        put(8);
        put(self.combat);
        for side in self.sides {
            side.stream(memory, base, &mut put)?;
        }
        Ok(())
    }
}

pub fn dispatch_inputs(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    loser: usize,
    winner: usize,
) -> Result<(CombatHeader, u8), &'static str> {
    let header = CombatHeader::read(memory, base, pointer)?;
    let sides = header.sides();
    let mask = if sides == [winner, loser] {
        1
    } else if sides == [loser, winner] {
        2
    } else {
        return Err("land dispatch sides differ");
    };
    Ok((header, mask))
}
pub fn leader_reference(
    memory: &impl Memory,
    base: usize,
    leader: usize,
) -> Result<u64, &'static str> {
    if leader == 0 {
        return Ok(0);
    }
    identity(memory, base, leader, carmy_leader::VTABLE_RVA)?;
    field(memory, leader, cunit_leader::TYPE)
}
pub fn final_inputs(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<[u64; 4], &'static str> {
    let mut out = [0; 4];
    let sides = field::<[u8; SIDE_POINTER_BYTES]>(memory, pointer, combat::ATTACKER)?;
    const DEFENDER: usize = combat::DEFENDER - combat::ATTACKER;
    let pointers = [
        u64::from_le_bytes(sides[..size_of::<u64>()].try_into().unwrap()) as usize,
        u64::from_le_bytes(
            sides[DEFENDER..DEFENDER + size_of::<u64>()]
                .try_into()
                .unwrap(),
        ) as usize,
    ];
    use engine_native_memory::memory::{
        RecordLayout,
        RecordSources,
        packed_fields,
        packed_stride,
    };
    const SPANS: [(usize, usize); 4] = [
        (0, size_of::<u64>()),
        (side::COMBAT, size_of::<u64>()),
        (side::PROGRESS, size_of::<i64>()),
        (side::OUTCOME_FLAGS, size_of::<u8>()),
    ];
    const FIELDS: [engine_native_memory::memory::RecordField; 4] = packed_fields(SPANS);
    const STRIDE: usize = packed_stride(SPANS);
    const BACKLINK: usize = FIELDS[1].destination_offset;
    const PROGRESS: usize = FIELDS[2].destination_offset;
    const FLAGS: usize = FIELDS[3].destination_offset;
    const _: () = {
        assert!(BACKLINK == size_of::<u64>());
        assert!(PROGRESS == BACKLINK + size_of::<u64>());
        assert!(FLAGS == PROGRESS + size_of::<i64>());
        assert!(STRIDE == FLAGS + size_of::<u8>());
    };
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, STRIDE).unwrap();
    let mut blocks = [0u8; 2 * STRIDE];
    if memory.copy_records(RecordSources::Pointers(&pointers), &LAYOUT, &mut blocks) != 8 {
        return Err("border final inputs unreadable");
    }
    for n in 0..2 {
        let bytes = &blocks[n * STRIDE..(n + 1) * STRIDE];
        if pointers[n] == 0
            || u64::from_le_bytes(bytes[..8].try_into().unwrap())
                != module(base, cland_border_war_combatant::VTABLE_RVA)? as u64
        {
            return Err("border final side identity mismatch");
        }
        if u64::from_le_bytes(
            bytes[BACKLINK..BACKLINK + size_of::<u64>()]
                .try_into()
                .unwrap(),
        ) != pointer as u64
        {
            return Err("border final side backlink differs");
        }
        out[n * 2] = u64::from(bytes[FLAGS]);
        out[n * 2 + 1] = u64::from_le_bytes(
            bytes[PROGRESS..PROGRESS + size_of::<i64>()]
                .try_into()
                .unwrap(),
        );
    }
    Ok(out)
}
#[derive(Clone, Copy)]
pub struct NullTagPlan(engine_native_memory::msvc_string::Descriptor);
impl NullTagPlan {
    pub fn country(memory: &impl Memory, base: usize, raw: u32) -> Result<Self, &'static str> {
        if raw == 0 {
            return Self::read(memory, base);
        }
        let (state, _) = state_hour(memory, base)?;
        let tags = Array::read(
            memory,
            state
                .checked_add(engine_native::cgame_state::COUNTRY_TAGS)
                .ok_or("history tags address overflow")?,
            engine_native_memory::msvc_string::STRIDE,
        )
        .ok_or("history tag array invalid")?;
        let at = tags
            .element(raw, engine_native_memory::msvc_string::STRIDE)
            .ok_or("history raw tag out of bounds")?;
        Ok(Self(engine_native_memory::msvc_string::Descriptor::read(memory, at)?))
    }
    fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let db = field::<u64>(
            memory,
            module(base, engine_native_parts::ccountry_database::SINGLETON_RVA)?,
            0,
        )? as usize;
        let tags = Array::read(memory, db, engine_native_memory::msvc_string::STRIDE)
            .ok_or("null history tag array invalid")?;
        let at = tags
            .element(0, engine_native_memory::msvc_string::STRIDE)
            .ok_or("null history tag absent")?;
        Ok(Self(engine_native_memory::msvc_string::Descriptor::read(memory, at)?))
    }
    pub fn words(&self) -> usize {
        1 + self.0.len.div_ceil(8)
    }
    pub fn copy(self, memory: &impl Memory, mut put: impl FnMut(u64)) -> Result<(), &'static str> {
        put(self.0.len as u64);
        self.0.copy_words(memory, put)
    }
}
#[derive(Clone, Copy)]
pub struct HistoryPlan {
    pub words: usize,
    header: [u64; 4],
    tags: [NullTagPlan; 2],
}
impl HistoryPlan {
    pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Self, &'static str> {
        let header = CombatHeader::read(memory, base, pointer)?;
        let border = header.border_war();
        let mut countries = [0u64; 2];
        let qword = |offset| header.qword(offset);
        for n in 0..2 {
            let p = qword([combat::ATTACKER, combat::DEFENDER][n]) as usize;
            let bytes = field::<[u8; HISTORY_SIDE_BYTES]>(memory, p, 0)?;
            let expected = if border {
                cland_border_war_combatant::VTABLE_RVA
            } else {
                side::VTABLE_RVA
            };
            if p == 0
                || u64::from_le_bytes(bytes[..8].try_into().unwrap())
                    != module(base, expected)? as u64
            {
                return Err("history side identity mismatch");
            }
            if u64::from_le_bytes(bytes[side::COMBAT..side::COMBAT + 8].try_into().unwrap())
                != pointer as u64
            {
                return Err("history side backlink differs");
            }
            let count = i32::from_le_bytes(
                bytes[side::COUNTRIES + 2 * size_of::<u64>()..side::COUNTRIES + SET_BYTES]
                    .try_into()
                    .unwrap(),
            );
            if count < 0 {
                return Err("history country count invalid");
            }
            if count != 0 {
                let first = u64::from_le_bytes(
                    bytes[side::COUNTRIES..side::COUNTRIES + 8]
                        .try_into()
                        .unwrap(),
                ) as usize;
                if first == 0 {
                    return Err("history first country null");
                }
                countries[n] = u64::from(field::<u32>(memory, first, 0)?);
            }
        }
        let tags = [
            NullTagPlan::country(memory, base, countries[0] as u32)?,
            NullTagPlan::country(memory, base, countries[1] as u32)?,
        ];
        Ok(Self {
            words: 5 + tags.iter().map(NullTagPlan::words).sum::<usize>(),
            header: [5, qword(combat::HANDLE), countries[0], countries[1]],
            tags,
        })
    }
    pub fn copy(self, memory: &impl Memory, mut put: impl FnMut(u64)) -> Result<(), &'static str> {
        for word in self.header {
            put(word);
        }
        for tag in self.tags {
            tag.copy(memory, &mut put)?;
        }
        Ok(())
    }
}
pub const DESTROY_RVA: Rva = Rva(0xd3bdc0);
pub const DESTRUCTION_RVA: Rva = Rva(0xce6370);
pub const BORDER_FINALIZE_RVA: Rva = Rva(0x148a4b0);
pub const BORDER_TERMINATE_RVA: Rva = Rva(0x148b950);
pub const BORDER_TERMINATE_RETURNS: [Rva; 2] = [Rva(0x148a51d), Rva(0x148a52d)];

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("land event field unreadable")
}
fn module(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("land event module address withheld")
}
fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: Rva,
) -> Result<(), &'static str> {
    if pointer != 0
        && Some(field::<u64>(memory, pointer, 0)?)
            == engine_sites::at(base, vtable).map(|v| v as u64)
    {
        Ok(())
    } else {
        Err("land event object identity mismatch")
    }
}
pub fn state_hour(memory: &impl Memory, base: usize) -> Result<(usize, i32), &'static str> {
    let state = field::<u64>(memory, module(base, cgame_state::SINGLETON_RVA)?, 0)? as usize;
    if state == 0 {
        return Err("land event game state null");
    }
    Ok((state, field(memory, state, cgame_state::CURRENT_HOUR)?))
}
pub fn winners(flags: [u8; 2], progress: [i64; 2]) -> u8 {
    let bits = u8::from(flags[0] & 2 != 0) | (u8::from(flags[1] & 2 != 0) << 1);
    if bits != 0 {
        bits
    } else if progress[0] > progress[1] {
        1
    } else {
        2
    }
}
pub fn border_winners(flags: [u8; 3], terminating: [u8; 2]) -> Result<u8, &'static str> {
    if flags[0] != 0 {
        return Ok(1);
    }
    if flags[1] != 0 {
        return Ok(2);
    }
    if flags[2] != 0 {
        return Ok(0);
    }
    match terminating {
        [0, 1] => Ok(1),
        [1, 0] => Ok(2),
        [0, 0] | [1, 1] => Ok(0),
        _ => Err("border-war finalizer verdict unobserved"),
    }
}
#[derive(Clone, Copy, Default)]
struct Set {
    head: usize,
    tail: usize,
    count: usize,
}
fn set(bytes: &[u8], limit: usize) -> Result<Set, &'static str> {
    let count = i32::from_le_bytes(bytes[16..].try_into().unwrap());
    let count = usize::try_from(count)
        .ok()
        .filter(|n| *n <= limit)
        .ok_or("land event country set invalid")?;
    Ok(Set {
        head: u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize,
        tail: u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize,
        count,
    })
}
#[derive(Clone, Copy)]
pub struct ResultPlan {
    pub words: usize,
    header: [u64; 9],
    sets: [Set; 4],
    units: [Array; 2],
    flags: [u8; 2],
    progress: [i64; 2],
}
impl ResultPlan {
    pub fn participants(mut self) -> Self {
        self.header[0] = 3;
        self.without_candidates()
    }
    pub fn combat(&self) -> u64 {
        self.header[1]
    }
    fn exit_words(&self) -> usize {
        (self.units[0].count as usize + self.units[1].count as usize) * crate::land_edges::EXIT_WORDS
    }
    fn hit_words(&self) -> usize {
        (self.units[0].count as usize + self.units[1].count as usize) * crate::land_edges::HIT_WORDS * crate::land_edges::HITS_PER_MEMBER
    }
    pub fn without_candidates(mut self) -> Self {
        for units in &mut self.units {
            self.words -= units.count as usize * 3;
            *units = Array { data: 0, count: 0 };
        }
        self
    }
    pub fn read(
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        border_verdict: Option<([u8; 3], [u8; 2])>,
    ) -> Result<Self, &'static str> {
        Self::read_header(
            memory,
            base,
            CombatHeader::read(memory, base, pointer)?,
            border_verdict,
        )
    }
    pub fn read_header(
        memory: &impl Memory,
        base: usize,
        header: CombatHeader,
        border_verdict: Option<([u8; 3], [u8; 2])>,
    ) -> Result<Self, &'static str> {
        let pointer = header.pointer;
        let border = header.border_war();
        let (state, hour) = state_hour(memory, base)?;
        let limit = ccountry::Reader::new(memory, base, state)?.arrays()[1].count as usize;
        let qword = |offset| header.qword(offset);
        let sides = [
            qword(combat::ATTACKER) as usize,
            qword(combat::DEFENDER) as usize,
        ];
        use engine_native_memory::memory::{RecordLayout, RecordSources};
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&RESULT_SIDE_FIELDS, RESULT_SIDE_BYTES).unwrap();
        let mut blocks = [0u8; 2 * RESULT_SIDE_BYTES];
        if memory.copy_records(RecordSources::Pointers(&sides), &LAYOUT, &mut blocks) != 6 {
            return Err("land result combatant blocks unreadable");
        }
        let mut sets = [Set::default(); 4];
        let mut units = [Array { data: 0, count: 0 }; 2];
        let mut flags = [0; 2];
        let mut progress = [0; 2];
        let mut words: usize = 10;
        for n in 0..2 {
            let block = &blocks[n * RESULT_SIDE_BYTES..(n + 1) * RESULT_SIDE_BYTES];
            let qword =
                |offset: usize| u64::from_le_bytes(block[offset..offset + 8].try_into().unwrap());
            let expected = if border {
                cland_border_war_combatant::VTABLE_RVA
            } else {
                side::VTABLE_RVA
            };
            if sides[n] == 0 || qword(0) != module(base, expected)? as u64 {
                return Err("land result side identity mismatch");
            }
            if qword(side::COMBAT) != pointer as u64 {
                return Err("land result side backlink mismatch");
            }
            for j in 0..2 {
                let at = [side::COUNTRIES, side::SECONDARY_COUNTRIES][j];
                sets[n * 2 + j] = set(&block[at..at + SET_BYTES], limit)?;
                words = words
                    .checked_add(sets[n * 2 + j].count)
                    .ok_or("land result size overflow")?;
            }
            units[n] =
                Array::from_header(block[side::UNITS..side::UNITS + 16].try_into().unwrap(), 8)
                    .ok_or("land result unit header invalid")?;
            flags[n] = block[RESULT_FLAGS];
            progress[n] = i64::from_le_bytes(
                block[RESULT_PROGRESS..RESULT_PROGRESS + size_of::<i64>()]
                    .try_into()
                    .unwrap(),
            );
            words = words
                .checked_add(5 + units[n].count as usize * 3)
                .ok_or("land result size overflow")?;
        }
        let province = qword(combat::PROVINCE) as usize;
        identity(memory, base, province, cprovince::VTABLE_RVA)?;
        let (border_flags, terminating) = if border {
            border_verdict.ok_or("border-war finalizer not observed")?
        } else {
            ([0; 3], [255; 2])
        };
        let winner = if border {
            if terminating == [255; 2] && border_flags == [0; 3] {
                255
            } else {
                border_winners(border_flags, terminating)?
            }
        } else {
            winners(flags, progress)
        };
        let packed = u64::from(border_flags[0])
            | u64::from(border_flags[1]) << 8
            | u64::from(border_flags[2]) << 16
            | u64::from(terminating[0]) << 24
            | u64::from(terminating[1]) << 32;
        Ok(Self {
            words,
            header: [
                0,
                qword(combat::HANDLE),
                u64::from(field::<u32>(memory, province, cprovince::ID)?),
                header.dword(combat::DAY) as u64,
                header.dword(combat::DURATION) as u64,
                hour as u64,
                u64::from(winner),
                u64::from(border),
                packed,
            ],
            sets,
            units,
            flags,
            progress,
        })
    }
    pub fn copy(
        self,
        memory: &impl Memory,
        base: usize,
        mut put: impl FnMut(u64),
    ) -> Result<(), &'static str> {
        for word in self.header {
            put(word);
        }
        for n in 0..2 {
            put(u64::from(self.flags[n]));
            put(self.progress[n] as u64);
            for set in &self.sets[n * 2..n * 2 + 2] {
                put(set.count as u64);
                let mut node = set.head;
                let mut last = 0;
                for _ in 0..set.count {
                    if node == 0 {
                        return Err("land result country set short");
                    }
                    let bytes = field::<[u8; 24]>(memory, node, 0)?;
                    put(u64::from(u32::from_le_bytes(
                        bytes[..4].try_into().unwrap(),
                    )));
                    last = node;
                    node = u64::from_le_bytes(bytes[0x10..].try_into().unwrap()) as usize;
                }
                if node != 0 || last != set.tail {
                    return Err("land result country set changed");
                }
            }
            put(u64::from(self.units[n].count));
            for ordinal in 0..self.units[n].count {
                let army = field::<u64>(
                    memory,
                    self.units[n]
                        .element(ordinal, 8)
                        .ok_or("land result unit offset overflow")?,
                    0,
                )? as usize;
                identity(memory, base, army, carmy::VTABLE_RVA)?;
                let group = field::<u64>(memory, army, carmy::ORDERS_GROUP)? as usize;
                let leader = if group == 0 {
                    0
                } else {
                    identity(memory, base, group, corders_group::VTABLE_RVA)?;
                    field::<u64>(memory, group, corders_group::LEADER)? as usize
                };
                if leader == 0 {
                    put(0);
                    put(0);
                    put(0);
                    continue;
                }
                identity(memory, base, leader, carmy_leader::VTABLE_RVA)?;
                let skill = field::<u64>(memory, leader, cunit_leader::SKILL)? as usize;
                identity(memory, base, skill, cunit_leader_skill::VTABLE_RVA)?;
                put(field(memory, leader, cunit_leader::TYPE)?);
                put(field::<i32>(memory, skill, cunit_leader_skill::LEVEL)? as u64);
                put(field::<i32>(memory, leader, cunit_leader::CATEGORY)? as u64);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct DestructionPlan {
    pub words: usize,
    header: [u64; 9],
    combats: Array,
}
impl DestructionPlan {
    pub fn division(&self) -> u64 {
        self.header[1]
    }
    pub fn read(
        memory: &impl Memory,
        base: usize,
        from: usize,
        to: usize,
        army: usize,
        killer: usize,
    ) -> Result<Self, &'static str> {
        identity(memory, base, army, carmy::VTABLE_RVA)?;
        identity(memory, base, from, cprovince::VTABLE_RVA)?;
        identity(memory, base, to, cprovince::VTABLE_RVA)?;
        let (state, hour) = state_hour(memory, base)?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        let roster = field::<u32>(memory, army, cunit::ROSTER_OWNER)?;
        let victim = countries.resolve(roster)?;
        let destination = field::<u32>(memory, to, cprovince::CONTROLLER)?;
        let slot = if destination == 0 {
            0
        } else {
            countries.resolve(destination)?.canonical_slot
        };
        let enemy = if (victim.id as i32) <= 0
            || roster == destination
            || (destination != 0 && victim.canonical_slot == slot)
        {
            false
        } else {
            let diplomacy =
                field::<u64>(memory, victim.pointer, engine_native::ccountry::DIPLOMACY_STATUS)? as usize;
            identity(
                memory,
                base,
                diplomacy,
                engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA,
            )?;
            if field::<u64>(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)?
                != victim.pointer as u64
            {
                return Err("destruction diplomacy owner differs");
            }
            let statuses = Array::read(
                memory,
                diplomacy
                    .checked_add(engine_native_parts::cdiplomacy_status_layout::RELATIONS)
                    .ok_or("destruction relation address overflow")?,
                8,
            )
            .ok_or("destruction relations invalid")?;
            if slot >= statuses.count {
                return Err("destruction relation slot out of bounds");
            }
            let status = field::<u64>(
                memory,
                statuses
                    .element(slot, 8)
                    .ok_or("destruction relation offset overflow")?,
                0,
            )? as usize;
            if status == 0 {
                false
            } else {
                identity(memory, base, status, engine_native::crelation::status::VTABLE_RVA)?;
                if field::<u64>(memory, status, engine_native::crelation::status::DIPLOMACY_STATUS)?
                    != diplomacy as u64
                {
                    return Err("destruction relation owner differs");
                }
                let war =
                    field::<u64>(memory, status, engine_native::crelation::status::WAR_RELATION)? as usize;
                if war == 0 {
                    false
                } else {
                    identity(memory, base, war, engine_native::cwar_relation::VTABLE_RVA)?;
                    field::<u8>(memory, war, engine_native::crelation::RETIRED)? == 0
                }
            }
        };
        let killer_country = if enemy {
            destination
        } else {
            field::<u32>(memory, from, cprovince::CONTROLLER)?
        };
        let killer_handle = if killer == 0 {
            0
        } else {
            identity(memory, base, killer, carmy::VTABLE_RVA)?;
            field(memory, killer, cunit::HANDLE)?
        };
        let combats = Array::read(
            memory,
            army.checked_add(cunit::COMBAT_REFERENCE_DATA)
                .ok_or("destroyed combat address overflow")?,
            8,
        )
        .ok_or("destroyed division combat header invalid")?;
        Ok(Self {
            words: destruction_words(combats),
            header: [
                1,
                field(memory, army, cunit::HANDLE)?,
                u64::from(field::<u32>(memory, army, cunit::LOGICAL_OWNER)?),
                u64::from(killer_country),
                killer_handle,
                u64::from(field::<u32>(memory, from, cprovince::ID)?),
                u64::from(field::<u32>(memory, to, cprovince::ID)?),
                hour as u64,
                u64::from(combats.count),
            ],
            combats,
        })
    }
    pub fn copy(
        self,
        memory: &impl Memory,
        base: usize,
        mut put: impl FnMut(u64),
    ) -> Result<(), &'static str> {
        for word in self.header {
            put(word);
        }
        for n in 0..self.combats.count {
            let pointer = field::<u64>(
                memory,
                self.combats
                    .element(n, 8)
                    .ok_or("destroyed combat offset overflow")?,
                0,
            )? as usize;
            let vtable = field::<u64>(memory, pointer, 0)? as usize;
            let classes = [combat::VTABLE_RVA, cland_border_war_combat::VTABLE_RVA];
            if pointer == 0 || engine_sites::which(base, vtable, &classes).is_none() {
                return Err("destroyed division combat class unsupported");
            }
            put(field(memory, pointer, combat::HANDLE)?);
        }
        Ok(())
    }
}

pub fn killer_handle(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u64, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    identity(memory, base, pointer, carmy::VTABLE_RVA)?;
    field(memory, pointer, cunit::HANDLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_demand_presizes_destroyed_divisions_and_grows_with_combat_references() {
        let mut world = engine_native::ccountry::synthetic::World::new(0, 2, ccountry::DIVISIONS + 16);
        for slot in 1..=2 {
            let army = world.alloc(cunit::COMBAT_REFERENCE_DATA + 16);
            let roster = world.alloc(8);
            world.u64(army, carmy::VTABLE_RVA.0 as u64);
            world.u64(roster, army as u64);
            world.array(world.object(slot) + ccountry::DIVISIONS, roster, 1);
        }
        assert_eq!(
            word_demand(&world.memory(), world.module, world.game_state()),
            Ok(28)
        );
        let roster =
            Array::read(&world.memory(), world.object(1) + ccountry::DIVISIONS, 8).unwrap();
        let army = engine_native_memory::read::<u64>(&world.memory(), roster.data).unwrap() as usize;
        let combats = world.alloc(5 * 8);
        world.array(army + cunit::COMBAT_REFERENCE_DATA, combats, 5);
        assert_eq!(
            word_demand(&world.memory(), world.module, world.game_state()),
            Ok(33)
        );
    }
    #[test]
    fn native_winner_rules_keep_ties_signed_width_both_bits_and_border_draw() {
        assert_eq!(winners([0, 0], [i64::MAX, i64::MIN]), 1);
        assert_eq!(winners([0, 0], [7, 7]), 2);
        assert_eq!(winners([2, 2], [0, 0]), 3);
        assert_eq!(winners([0, 2], [99, 0]), 2);
        assert_eq!(border_winners([0, 0, 1], [255, 255]), Ok(0));
        assert_eq!(border_winners([1, 1, 1], [255, 255]), Ok(1));
        assert_eq!(border_winners([0; 3], [0, 1]), Ok(1));
        assert_eq!(border_winners([0; 3], [1, 1]), Ok(0));
        assert!(border_winners([0; 3], [255, 0]).is_err());
    }

    #[derive(Default)]
    struct Fixture(std::collections::BTreeMap<usize, u8>);
    impl Memory for Fixture {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            for (n, v) in out.iter_mut().enumerate() {
                let Some(b) = self.0.get(&(address + n)) else {
                    return false;
                };
                *v = *b;
            }
            true
        }
    }
    impl Fixture {
        fn put(&mut self, p: usize, b: &[u8]) {
            for (n, v) in b.iter().enumerate() {
                self.0.insert(p + n, *v);
            }
        }
        fn q(&mut self, p: usize, v: u64) {
            self.put(p, &v.to_le_bytes());
        }
        fn array(&mut self, p: usize, data: usize, count: u32) {
            self.q(p, data as u64);
            self.put(p + 8, &count.to_le_bytes());
            self.put(p + 12, &count.to_le_bytes());
        }
    }
    #[test]
    fn terminal_loss_plan_authenticates_each_side_and_observer_without_roster_walk() {
        let mut f = Fixture::default();
        f.q(0x2000, combat::VTABLE_RVA.0 as u64);
        f.put(0x2000 + combat::HANDLE, &[0; COMBAT_HEADER_BYTES]);
        f.q(0x2000 + combat::HANDLE, 7 << 32 | 62);
        f.q(0x2000 + combat::ATTACKER, 0x3000);
        f.q(0x2000 + combat::DEFENDER, 0x4000);
        for p in [0x3000, 0x4000] {
            f.q(p, side::VTABLE_RVA.0 as u64);
            f.q(p + side::COMBAT, 0x2000);
            let observer = p + side::STATS_OBSERVER;
            f.q(observer, crate::cstats_observer::VTABLE_RVA.0 as u64);
            f.put(
                observer + crate::cstats_observer::MANPOWER_LOSS,
                &91i32.to_le_bytes(),
            );
            let pool = observer + crate::cstats_observer::EQUIPMENT_LOSSES;
            f.q(pool, engine_native::cequipment_variant::pool::VTABLE_RVA.0 as u64);
            f.array(pool + engine_native::cequipment_variant::pool::ITEMS, 0, 0);
        }
        let plan = LossesPlan::read(&f, 0, 0x2000).unwrap();
        let mut words = Vec::new();
        plan.copy(&f, 0, |w| words.push(w)).unwrap();
        assert_eq!(words, [8, 7 << 32 | 62, 91, 0, 91, 0]);
        assert_eq!(words.len() + 1, plan.words);
        f.q(0x4000 + side::COMBAT, 0x9999);
        assert!(LossesPlan::read(&f, 0, 0x2000).is_err());
        f.q(0x4000 + side::COMBAT, 0x2000);
        f.q(0x4000 + side::STATS_OBSERVER, 0);
        assert!(LossesPlan::read(&f, 0, 0x2000).is_err());
    }
    #[test]
    fn native_result_plan_copies_transient_lists_and_refuses_first_fault() {
        let mut f = Fixture::default();
        f.q(engine_native_parts::ccountry_database::SINGLETON_RVA.0, 0xb000);
        f.array(0xb000, 0xc000, 1);
        f.put(0xc000, &[0; 32]);
        f.put(0xc000 + 24, &15u64.to_le_bytes());
        f.q(cgame_state::SINGLETON_RVA.0, 0x1000);
        f.put(0x1000 + cgame_state::CURRENT_HOUR, &80i32.to_le_bytes());
        f.array(0x1000 + engine_native::cgame_state::COUNTRIES, 0x9000, 3);
        f.array(0x1000 + engine_native::cgame_state::COUNTRY_LINKS, 0xa000, 3);
        f.q(0x2000, combat::VTABLE_RVA.0 as u64);
        f.put(0x2000 + combat::HANDLE, &[0; 0x30]);
        f.q(0x2000 + combat::HANDLE, 0x8000_0001_0000_003e);
        f.q(0x2000 + combat::ATTACKER, 0x3000);
        f.q(0x2000 + combat::DEFENDER, 0x4000);
        f.q(0x2000 + combat::PROVINCE, 0x5000);
        f.put(0x2000 + combat::DAY, &4i32.to_le_bytes());
        f.put(0x2000 + combat::DURATION, &9i32.to_le_bytes());
        f.q(0x5000, cprovince::VTABLE_RVA.0 as u64);
        f.put(0x5000 + cprovince::ID, &42u32.to_le_bytes());
        for p in [0x3000, 0x4000] {
            f.put(p, &[0; 0x64]);
            f.q(p, side::VTABLE_RVA.0 as u64);
            f.q(p + side::COMBAT, 0x2000);
            f.array(p + side::UNITS, 0, 0);
            for offset in [side::COUNTRIES, side::SECONDARY_COUNTRIES] {
                f.put(p + offset, &[0; 20]);
            }
            f.put(p + side::OUTCOME_FLAGS, &[0]);
            f.q(p + side::PROGRESS, i64::MIN as u64);
        }
        f.q(0x3000 + side::COUNTRIES, 0x6000);
        f.q(0x3000 + side::COUNTRIES + 8, 0x6000);
        f.put(0x3000 + side::COUNTRIES + 16, &1i32.to_le_bytes());
        f.put(0x6000, &[0; 24]);
        f.put(0x6000, &0x8000_0001u32.to_le_bytes());
        f.q(0x6010, 0);
        let (header, winner) = dispatch_inputs(&f, 0, 0x2000, 0x4000, 0x3000).unwrap();
        assert_eq!(winner, 1);
        assert_eq!(header.handle(), 0x8000_0001_0000_003e);
        assert_eq!(header.sides(), [0x3000, 0x4000]);
        let denied = 0x2000 + combat::HANDLE;
        let byte = f.0.remove(&denied).unwrap();
        assert!(ResultPlan::read_header(&f, 0, header, None).is_ok());
        assert!(ResultPlan::read(&f, 0, 0x2000, None).is_err());
        f.0.insert(denied, byte);
        let plan = ResultPlan::read(&f, 0, 0x2000, None).unwrap();
        let mut words = Vec::new();
        plan.copy(&f, 0, |v| words.push(v)).unwrap();
        assert_eq!(words.len() + 1, plan.words);
        assert_eq!(words[1], 0x8000_0001_0000_003e);
        assert_eq!(words[6], 2);
        assert!(words.contains(&u64::from(0x8000_0001u32)));
        f.put(0x6000, &1u32.to_le_bytes());
        f.array(0x1000 + engine_native::cgame_state::COUNTRY_TAGS, 0x18000, 3);
        f.put(0x18020, &[0; 32]);
        f.put(0x18020, b"raw-alias");
        f.q(0x18030, 9);
        f.q(0x18038, 15);
        let history = HistoryPlan::read(&f, 0, 0x2000).unwrap();
        let mut tags = Vec::new();
        history.copy(&f, |v| tags.push(v)).unwrap();
        assert_eq!(&tags[..5], &[5, 0x8000_0001_0000_003e, 1, 0, 9]);
        assert_eq!(tags[5].to_le_bytes(), *b"raw-alia");
        f.put(0x3000 + side::COUNTRIES + 16, &0i32.to_le_bytes());
        let history = HistoryPlan::read(&f, 0, 0x2000).unwrap();
        let size = history.words;
        let mut end = Vec::new();
        history.copy(&f, |v| end.push(v)).unwrap();
        assert_eq!(end.len() + 1, size);
        assert_eq!(&end[..4], &[5, 0x8000_0001_0000_003e, 0, 0]);
        f.array(0x1000 + engine_native::cgame_state::COUNTRY_LINKS, 0xa000, 6);
        f.q(0x3000 + side::COUNTRIES, 0x1a000);
        f.q(0x3000 + side::COUNTRIES + 8, 0x1a080);
        f.put(0x3000 + side::COUNTRIES + 16, &5i32.to_le_bytes());
        for n in 0..5 {
            let node = 0x1a000 + n * 0x20;
            f.put(node, &[0; 24]);
            f.put(node, &(n as u32 + 1).to_le_bytes());
            f.q(node + 0x10, if n == 4 { 0 } else { (node + 0x20) as u64 });
        }
        let plan = ResultPlan::read(&f, 0, 0x2000, None).unwrap();
        let mut aliases = Vec::new();
        plan.copy(&f, 0, |word| aliases.push(word)).unwrap();
        assert_eq!(&aliases[11..17], &[5, 1, 2, 3, 4, 5]);
        f.array(0x3000 + side::UNITS, 0x7000, 1);
        f.q(0x7000, 0x8000);
        f.q(0x8000, 17);
        let plan = ResultPlan::read(&f, 0, 0x2000, None).unwrap();
        assert!(plan.copy(&f, 0, |_| {}).is_err());
        let skipped = plan.without_candidates();
        assert_eq!(skipped.words + 3, plan.words);
        assert!(skipped.copy(&f, 0, |_| {}).is_ok());
        f.0.remove(&(0x3000 + side::PROGRESS + 7));
        assert!(ResultPlan::read(&f, 0, 0x2000, None).is_err());
    }
    #[test]
    fn destruction_selects_active_enemy_and_converts_combat_pointers_to_handles() {
        let mut f = Fixture::default();
        f.q(cgame_state::SINGLETON_RVA.0, 0x1000);
        f.put(0x1000 + cgame_state::CURRENT_HOUR, &80i32.to_le_bytes());
        f.array(0x1000 + engine_native::cgame_state::COUNTRIES, 0x9000, 3);
        f.array(0x1000 + engine_native::cgame_state::COUNTRY_LINKS, 0xa000, 3);
        for (slot, p) in [(1usize, 0xd000usize), (2, 0xe000)] {
            f.q(0x9000 + slot * 8, p as u64);
            f.put(0xa000 + slot * 4, &(slot as u32).to_le_bytes());
            f.q(p, ccountry::VTABLE_RVA.0 as u64);
            f.put(p + ccountry::ID, &(slot as u32).to_le_bytes());
        }
        f.q(0x2000, carmy::VTABLE_RVA.0 as u64);
        f.q(0x2000 + cunit::HANDLE, 0x8000_0001_0000_1269);
        f.put(0x2000 + cunit::ROSTER_OWNER, &1u32.to_le_bytes());
        f.put(0x2000 + cunit::LOGICAL_OWNER, &2u32.to_le_bytes());
        for (p, id, controller) in [(0x3000, 17u32, 1u32), (0x4000, 18, 2)] {
            f.q(p, cprovince::VTABLE_RVA.0 as u64);
            f.put(p + cprovince::ID, &id.to_le_bytes());
            f.put(p + cprovince::CONTROLLER, &controller.to_le_bytes());
        }
        f.q(0x5000, carmy::VTABLE_RVA.0 as u64);
        f.q(0x5000 + cunit::HANDLE, 0x8000_0002_0000_1269);
        f.q(0xd000 + ccountry::DIPLOMACY_STATUS, 0xf000);
        f.q(0xf000, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
        f.q(0xf000 + engine_native_parts::cdiplomacy_status_layout::COUNTRY, 0xd000);
        f.array(0xf000 + engine_native_parts::cdiplomacy_status_layout::RELATIONS, 0x10000, 3);
        f.q(0x10010, 0x11000);
        f.q(0x11000, engine_native::crelation::status::VTABLE_RVA.0 as u64);
        f.q(0x11000 + engine_native::crelation::status::DIPLOMACY_STATUS, 0xf000);
        f.q(0x11000 + engine_native::crelation::status::WAR_RELATION, 0x12000);
        f.q(0x12000, engine_native::cwar_relation::VTABLE_RVA.0 as u64);
        f.put(0x12000 + engine_native::crelation::RETIRED, &[0]);
        f.array(0x2000 + cunit::COMBAT_REFERENCE_DATA, 0x14000, 2);
        for (slot, p, vt, handle) in [
            (0, 0x13000, combat::VTABLE_RVA, 0x8000_0003_0000_003e),
            (
                1,
                0x15000,
                cland_border_war_combat::VTABLE_RVA,
                0x8000_0004_0000_ffff,
            ),
        ] {
            f.q(0x14000 + slot * 8, p as u64);
            f.q(p, vt.0 as u64);
            f.q(p + combat::HANDLE, handle);
        }
        let capture = |f: &Fixture, killer| {
            let plan = DestructionPlan::read(f, 0, 0x3000, 0x4000, 0x2000, killer).unwrap();
            let mut words = Vec::new();
            plan.copy(f, 0, |w| words.push(w)).unwrap();
            assert_eq!(words.len() + 1, plan.words);
            words
        };
        let words = capture(&f, 0x5000);
        assert_eq!(&words[2..8], &[2, 2, 0x8000_0002_0000_1269, 17, 18, 80]);
        assert_eq!(&words[9..], &[0x8000_0003_0000_003e, 0x8000_0004_0000_ffff]);
        f.put(0x12000 + engine_native::crelation::RETIRED, &[1]);
        let words = capture(&f, 0);
        assert_eq!(&words[3..5], &[1, 0]);
        f.0.remove(&0x15000);
        let plan = DestructionPlan::read(&f, 0, 0x3000, 0x4000, 0x2000, 0).unwrap();
        assert!(plan.copy(&f, 0, |_| {}).is_err());
        f.0.remove(&(0x2000 + cunit::LOGICAL_OWNER));
        assert!(DestructionPlan::read(&f, 0, 0x3000, 0x4000, 0x2000, 0).is_err());
    }
}
