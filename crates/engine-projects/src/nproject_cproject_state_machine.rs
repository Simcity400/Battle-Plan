
#[cfg(test)]
use engine_native_memory::Memory;
use engine_native_memory::memory::{RecordField, RecordLayout};
use crate::{
    nproject_cprototype_project_state as prototype,
    nproject_csimple_project_state as simple,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02aa_8930);
pub const CURRENT_STATE: usize = 0x08;
pub const NOT_COMPLETED: usize = 0x10;
pub const PROTOTYPE_STATE: usize = 0x18;
pub const SIMPLE_STATE_1: usize = 0xe8;
pub const SIMPLE_STATE_2: usize = 0x168;
pub const SIMPLE_STATE_3: usize = 0x1e8;
pub const SIZE: usize = 0x268;

const STATES: [usize; 4] = [
    PROTOTYPE_STATE,
    SIMPLE_STATE_1,
    SIMPLE_STATE_2,
    SIMPLE_STATE_3,
];

const PROTOTYPE_TAIL: usize = prototype::ITERATIONS + 4 - prototype::STATE_ENUM;
const SIMPLE_1_TAIL: usize = SIMPLE_STATE_2 + 8 - (SIMPLE_STATE_1 + simple::STATE_ENUM);
const SIMPLE_2_TAIL: usize = SIMPLE_STATE_3 + 8 - (SIMPLE_STATE_2 + simple::STATE_ENUM);
const PROTOTYPE_ENUM: usize = PROTOTYPE_STATE + 8;
const PROTOTYPE_PROGRESS: usize =
    PROTOTYPE_ENUM + prototype::PROTOTYPE_PROGRESS - prototype::STATE_ENUM;
const PROTOTYPE_ITERATIONS: usize = PROTOTYPE_ENUM + prototype::ITERATIONS - prototype::STATE_ENUM;
const SIMPLE_1_VTABLE: usize = PROTOTYPE_ENUM + PROTOTYPE_TAIL;
const SIMPLE_1_ENUM: usize = SIMPLE_1_VTABLE + 8;
const SIMPLE_2_VTABLE: usize =
    SIMPLE_1_ENUM + SIMPLE_STATE_2 - (SIMPLE_STATE_1 + simple::STATE_ENUM);
const SIMPLE_2_ENUM: usize = SIMPLE_1_ENUM + SIMPLE_1_TAIL;
const SIMPLE_3_VTABLE: usize =
    SIMPLE_2_ENUM + SIMPLE_STATE_3 - (SIMPLE_STATE_2 + simple::STATE_ENUM);
const SIMPLE_3_ENUM: usize = SIMPLE_2_ENUM + SIMPLE_2_TAIL;
pub const FIELDS_SIZE: usize = SIMPLE_3_ENUM + 4;

pub const FIELDS: [RecordField; 6] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: PROTOTYPE_STATE + 8,
    },
    RecordField {
        source_offset: PROTOTYPE_STATE + prototype::STATE_ENUM,
        destination_offset: PROTOTYPE_ENUM,
        len: PROTOTYPE_TAIL,
    },
    RecordField {
        source_offset: SIMPLE_STATE_1,
        destination_offset: SIMPLE_1_VTABLE,
        len: 8,
    },
    RecordField {
        source_offset: SIMPLE_STATE_1 + simple::STATE_ENUM,
        destination_offset: SIMPLE_1_ENUM,
        len: SIMPLE_1_TAIL,
    },
    RecordField {
        source_offset: SIMPLE_STATE_2 + simple::STATE_ENUM,
        destination_offset: SIMPLE_2_ENUM,
        len: SIMPLE_2_TAIL,
    },
    RecordField {
        source_offset: SIMPLE_STATE_3 + simple::STATE_ENUM,
        destination_offset: SIMPLE_3_ENUM,
        len: 4,
    },
];
pub const LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FIELDS, FIELDS_SIZE).expect("valid record layout");

pub const PHASE_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: simple::PHASE_PROGRESS,
    destination_offset: 0,
    len: simple::PHASE_TARGET + 4 - simple::PHASE_PROGRESS,
}];
pub const PHASE_SIZE: usize = PHASE_FIELDS[0].len;
pub const PHASE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PHASE_FIELDS, PHASE_SIZE).expect("valid record layout");

const _: () = assert!(
    CURRENT_STATE + 8 <= PROTOTYPE_STATE
        && NOT_COMPLETED < PROTOTYPE_STATE
        && prototype::STATE_ENUM == simple::STATE_ENUM
        && prototype::PHASE_PROGRESS == simple::PHASE_PROGRESS
        && prototype::PHASE_TARGET == simple::PHASE_TARGET
        && SIMPLE_2_VTABLE + 8 == SIMPLE_2_ENUM
        && SIMPLE_3_VTABLE + 8 == SIMPLE_3_ENUM
        && SIMPLE_STATE_3 + simple::STATE_ENUM + 4 <= SIZE
);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub current_offset: u32,
    pub state_enums: [u32; 4],
    pub not_completed: u8,
    pub phase_progress: i64,
    pub phase_target: i32,
    pub prototype_progress: i32,
    pub iterations: i32,
}

fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn half(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub fn phase_source(pointer: usize, fields: &[u8; FIELDS_SIZE]) -> usize {
    let current = word(fields, CURRENT_STATE) as usize;
    match current.checked_sub(pointer) {
        Some(offset) if STATES.contains(&offset) => current,
        _ => pointer + PROTOTYPE_STATE,
    }
}

#[cfg(test)]
fn read_state(memory: &impl Memory, base: usize, pointer: usize) -> Result<State, &'static str> {
    use engine_native_memory::memory::RecordSources;
    let mut fields = [0; FIELDS_SIZE];
    let mut phase = [0; PHASE_SIZE];
    if memory.copy_records(RecordSources::Pointers(&[pointer]), &LAYOUT, &mut fields)
        != FIELDS.len()
        || memory.copy_records(
            RecordSources::Pointers(&[phase_source(pointer, &fields)]),
            &PHASE_LAYOUT,
            &mut phase,
        ) != 1
    {
        return Err("project state machine unreadable");
    }
    Decoder::new(base)?.decode(pointer, &fields, &phase)
}

pub(crate) struct Decoder {
    machine_vtable: u64,
    prototype_vtable: u64,
    simple_vtable: u64,
}

impl Decoder {
    pub(crate) fn new(base: usize) -> Result<Self, &'static str> {
        let relocated = |rva| {
            engine_sites::at(base, rva)
                .map(|v| v as u64)
                .ok_or("module address overflow")
        };
        Ok(Self {
            machine_vtable: relocated(VTABLE_RVA)?,
            prototype_vtable: relocated(prototype::VTABLE_RVA)?,
            simple_vtable: relocated(simple::VTABLE_RVA)?,
        })
    }

    pub(crate) fn decode(
        &self,
        pointer: usize,
        fields: &[u8; FIELDS_SIZE],
        phase: &[u8; PHASE_SIZE],
    ) -> Result<State, &'static str> {
        if word(fields, 0) != self.machine_vtable {
            return Err("project state machine vtable mismatch");
        }
        let current = usize::try_from(word(fields, CURRENT_STATE))
            .map_err(|_| "project current state address overflow")?;
        let not_completed = fields[NOT_COMPLETED];
        if not_completed > 1 {
            return Err("project state completion flag invalid");
        }
        let current_offset = current
            .checked_sub(pointer)
            .ok_or("project current state outside machine")?;
        if !STATES.contains(&current_offset) {
            return Err("project current state outside machine");
        }
        let mut state = State {
            current_offset: current_offset as u32,
            not_completed,
            ..State::default()
        };
        for (index, (vtable, value)) in [
            (PROTOTYPE_STATE, PROTOTYPE_ENUM),
            (SIMPLE_1_VTABLE, SIMPLE_1_ENUM),
            (SIMPLE_2_VTABLE, SIMPLE_2_ENUM),
            (SIMPLE_3_VTABLE, SIMPLE_3_ENUM),
        ]
        .into_iter()
        .enumerate()
        {
            let expected = if index == 0 {
                self.prototype_vtable
            } else {
                self.simple_vtable
            };
            if word(fields, vtable) != expected {
                return Err("project inline state vtable mismatch");
            }
            state.state_enums[index] = half(fields, value);
        }
        state.phase_progress = word(phase, 0) as i64;
        state.phase_target = half(phase, simple::PHASE_TARGET - simple::PHASE_PROGRESS) as i32;
        state.prototype_progress = half(fields, PROTOTYPE_PROGRESS) as i32;
        state.iterations = half(fields, PROTOTYPE_ITERATIONS) as i32;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn machine() -> [u8; 0x268] {
        let mut bytes = [0; 0x268];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&(0x1000_u64 + SIMPLE_STATE_1 as u64).to_le_bytes());
        bytes[NOT_COMPLETED] = 1;
        for (index, offset) in STATES.into_iter().enumerate() {
            let vtable = if index == 0 {
                prototype::VTABLE_RVA
            } else {
                simple::VTABLE_RVA
            };
            bytes[offset..offset + 8].copy_from_slice(&(vtable.0 as u64).to_le_bytes());
            bytes[offset + simple::STATE_ENUM..offset + simple::STATE_ENUM + 4]
                .copy_from_slice(&(index as u32 + 10).to_le_bytes());
        }
        bytes[PROTOTYPE_STATE + prototype::PHASE_PROGRESS
            ..PROTOTYPE_STATE + prototype::PHASE_PROGRESS + 8]
            .copy_from_slice(&(i64::MAX - 42).to_le_bytes());
        bytes[PROTOTYPE_STATE + prototype::PHASE_TARGET
            ..PROTOTYPE_STATE + prototype::PHASE_TARGET + 4]
            .copy_from_slice(&(-123_i32).to_le_bytes());
        bytes[PROTOTYPE_STATE + prototype::PROTOTYPE_PROGRESS
            ..PROTOTYPE_STATE + prototype::PROTOTYPE_PROGRESS + 4]
            .copy_from_slice(&99_i32.to_le_bytes());
        bytes[PROTOTYPE_STATE + prototype::ITERATIONS..PROTOTYPE_STATE + prototype::ITERATIONS + 4]
            .copy_from_slice(&i32::MAX.to_le_bytes());
        bytes
    }

    #[test]
    fn current_phase_and_inactive_prototype_fields_keep_their_distinct_native_widths() {
        let bytes = machine();
        let state = read_state(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            0x1000,
        )
        .unwrap();
        assert_eq!(state.current_offset, SIMPLE_STATE_1 as u32);
        assert_eq!(state.state_enums, [10, 11, 12, 13]);
        assert_eq!(state.phase_progress, 0);
        assert_eq!(state.phase_target, 0);
        assert_eq!(state.prototype_progress, 99);
        assert_eq!(state.iterations, i32::MAX);
    }

    struct Counted<'a> {
        memory: SliceMemory<'a>,
        calls: std::cell::Cell<usize>,
        bytes: std::cell::Cell<usize>,
        fault: Option<usize>,
    }

    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.bytes.set(self.bytes.get() + destination.len());
            !self
                .fault
                .is_some_and(|fault| (address..address + destination.len()).contains(&fault))
                && self.memory.copy(address, destination)
        }

        fn copy_records(
            &self,
            sources: engine_native_memory::memory::RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.calls.set(self.calls.get() + 1);
            struct Fields<'a, 'b>(&'a Counted<'b>);
            impl Memory for Fields<'_, '_> {
                fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                    self.0.copy(address, destination)
                }
            }
            Fields(self).copy_records(sources, layout, destination)
        }
    }

    fn counted(bytes: &[u8], fault: Option<usize>) -> Counted<'_> {
        Counted {
            memory: SliceMemory {
                base: 0x1000,
                bytes,
            },
            calls: Default::default(),
            bytes: Default::default(),
            fault,
        }
    }

    #[test]
    fn every_current_state_uses_its_own_phase_and_two_guarded_gathers() {
        for offset in STATES {
            let mut bytes = machine();
            bytes[8..16].copy_from_slice(&(0x1000_u64 + offset as u64).to_le_bytes());
            let phase = offset + simple::PHASE_PROGRESS;
            let target = offset + simple::PHASE_TARGET;
            bytes[phase..phase + 8].copy_from_slice(&i64::MIN.to_le_bytes());
            bytes[target..target + 4].copy_from_slice(&i32::MAX.to_le_bytes());
            let memory = counted(&bytes, None);
            let state = read_state(&memory, 0, 0x1000).unwrap();
            assert_eq!(
                (state.phase_progress, state.phase_target),
                (i64::MIN, i32::MAX)
            );
            assert_eq!((state.prototype_progress, state.iterations), (99, i32::MAX));
            assert_eq!(state.state_enums, [10, 11, 12, 13]);
            assert_eq!(memory.calls.get(), 2);
            assert_eq!(memory.bytes.get(), FIELDS_SIZE + PHASE_SIZE);
        }
    }

    #[test]
    fn only_recorded_words_and_the_current_phase_can_deny_the_machine() {
        let bytes = machine();
        let read = |fault| read_state(&counted(&bytes, Some(0x1000 + fault)), 0, 0x1000);
        for fault in [
            0,
            NOT_COMPLETED,
            SIMPLE_STATE_1 + simple::PHASE_TARGET + 3,
            PROTOTYPE_STATE + prototype::ITERATIONS + 3,
            SIMPLE_STATE_3 + simple::STATE_ENUM + 3,
        ] {
            assert_eq!(read(fault), Err("project state machine unreadable"));
        }
        for fault in [
            PROTOTYPE_STATE + prototype::PHASE_PROGRESS,
            SIMPLE_STATE_2 + simple::PHASE_PROGRESS,
            SIMPLE_STATE_1 + 0x20,
            SIZE - 1,
        ] {
            assert!(read(fault).is_ok(), "{fault:#x}");
        }
    }

    #[test]
    fn a_bad_current_pointer_or_inline_state_is_unavailable() {
        let mut bytes = machine();
        bytes[8..16].copy_from_slice(&0x1001_u64.to_le_bytes());
        assert_eq!(
            read_state(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Err("project current state outside machine")
        );
        bytes[8..16].copy_from_slice(&(0x1000_u64 + SIMPLE_STATE_1 as u64).to_le_bytes());
        bytes[SIMPLE_STATE_3] ^= 1;
        assert_eq!(
            read_state(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Err("project inline state vtable mismatch")
        );
    }
}
