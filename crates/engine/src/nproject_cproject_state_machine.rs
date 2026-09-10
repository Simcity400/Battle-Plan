//! `NProject::CProjectStateMachine`; archived collector `read_project_state_row`.

#[cfg(test)]
use crate::{Memory, read};
use crate::{
    nproject_cprototype_project_state as prototype, nproject_csimple_project_state as simple,
};

/// NProject::CProjectStateMachine primary vtable; confirmed by pinned executable RTTI.
pub const VTABLE_RVA: usize = 0x02a1_9358;
/// CProjectStateMachine +0x08 current state pointer; old reader requires one of the four inline states.
pub const CURRENT_STATE: usize = 0x08;
/// CProjectStateMachine +0x10 not-completed flag u8; old reader checks 0 or 1.
pub const NOT_COMPLETED: usize = 0x10;
/// CProjectStateMachine +0x18 inline CPrototypeProjectState; vtable-checked by archived state walker.
pub const PROTOTYPE_STATE: usize = 0x18;
/// CProjectStateMachine +0xe8 first inline CSimpleProjectState; vtable-checked by archived state walker.
pub const SIMPLE_STATE_1: usize = 0xe8;
/// CProjectStateMachine +0x168 second inline CSimpleProjectState; vtable-checked by archived state walker.
pub const SIMPLE_STATE_2: usize = 0x168;
/// CProjectStateMachine +0x1e8 third inline CSimpleProjectState; vtable-checked by archived state walker.
pub const SIMPLE_STATE_3: usize = 0x1e8;
/// CProjectStateMachine allocated extent; FCADC0 allocates 0x268 bytes before constructor 1A218A0 initializes all four states.
pub const SIZE: usize = 0x268;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub current_offset: u32,
    /// Native enums for prototype and the three simple states, in object order.
    pub state_enums: [u32; 4],
    pub not_completed: u8,
    pub phase_progress: i64,
    pub phase_target: i32,
    pub prototype_progress: i32,
    pub iterations: i32,
}

#[cfg(test)]
fn read_state(memory: &impl Memory, base: usize, pointer: usize) -> Result<State, &'static str> {
    let bytes = read::<[u8; SIZE]>(memory, pointer).ok_or("project state machine unreadable")?;
    Decoder::new(base)?.decode(pointer, &bytes)
}

/// Relocated immutable layout, prepared once for a complete pool walk.
pub(crate) struct Decoder {
    machine_vtable: u64,
    prototype_vtable: u64,
    simple_vtable: u64,
}

impl Decoder {
    pub(crate) fn new(base: usize) -> Result<Self, &'static str> {
        let relocated = |rva| {
            base.checked_add(rva)
                .map(|v| v as u64)
                .ok_or("module address overflow")
        };
        Ok(Self {
            machine_vtable: relocated(VTABLE_RVA)?,
            prototype_vtable: relocated(prototype::VTABLE_RVA)?,
            simple_vtable: relocated(simple::VTABLE_RVA)?,
        })
    }

    /// Decodes only a fully copied record. This does no foreign reads.
    pub(crate) fn decode(&self, pointer: usize, bytes: &[u8; SIZE]) -> Result<State, &'static str> {
        if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != self.machine_vtable {
            return Err("project state machine vtable mismatch");
        }
        let current = usize::try_from(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
            .map_err(|_| "project current state address overflow")?;
        let not_completed = bytes[NOT_COMPLETED];
        if not_completed > 1 {
            return Err("project state completion flag invalid");
        }
        let current_offset = current
            .checked_sub(pointer)
            .ok_or("project current state outside machine")?;
        let offsets = [
            PROTOTYPE_STATE,
            SIMPLE_STATE_1,
            SIMPLE_STATE_2,
            SIMPLE_STATE_3,
        ];
        if !offsets.contains(&current_offset) {
            return Err("project current state outside machine");
        }
        let mut state = State {
            current_offset: current_offset as u32,
            not_completed,
            ..State::default()
        };
        for (index, offset) in offsets.into_iter().enumerate() {
            let expected = if index == 0 {
                self.prototype_vtable
            } else {
                self.simple_vtable
            };
            if u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) != expected {
                return Err("project inline state vtable mismatch");
            }
            let at = offset + simple::STATE_ENUM;
            state.state_enums[index] = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        }
        let phase = current_offset + simple::PHASE_PROGRESS;
        let target = current_offset + simple::PHASE_TARGET;
        let progress = PROTOTYPE_STATE + prototype::PROTOTYPE_PROGRESS;
        let iterations = PROTOTYPE_STATE + prototype::ITERATIONS;
        state.phase_progress = i64::from_le_bytes(bytes[phase..phase + 8].try_into().unwrap());
        state.phase_target = i32::from_le_bytes(bytes[target..target + 4].try_into().unwrap());
        state.prototype_progress =
            i32::from_le_bytes(bytes[progress..progress + 4].try_into().unwrap());
        state.iterations =
            i32::from_le_bytes(bytes[iterations..iterations + 4].try_into().unwrap());
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    fn machine() -> [u8; 0x268] {
        let mut bytes = [0; 0x268];
        bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&(0x1000_u64 + SIMPLE_STATE_1 as u64).to_le_bytes());
        bytes[NOT_COMPLETED] = 1;
        for (index, offset) in [
            PROTOTYPE_STATE,
            SIMPLE_STATE_1,
            SIMPLE_STATE_2,
            SIMPLE_STATE_3,
        ]
        .into_iter()
        .enumerate()
        {
            let vtable = if index == 0 {
                prototype::VTABLE_RVA
            } else {
                simple::VTABLE_RVA
            };
            bytes[offset..offset + 8].copy_from_slice(&(vtable as u64).to_le_bytes());
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

    #[test]
    fn every_current_state_uses_its_own_phase_and_one_guarded_copy() {
        use std::cell::Cell;
        struct Counted<'a> {
            memory: SliceMemory<'a>,
            calls: Cell<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                self.memory.copy(address, destination)
            }
        }
        for offset in [
            PROTOTYPE_STATE,
            SIMPLE_STATE_1,
            SIMPLE_STATE_2,
            SIMPLE_STATE_3,
        ] {
            let mut bytes = machine();
            bytes[8..16].copy_from_slice(&(0x1000_u64 + offset as u64).to_le_bytes());
            let phase = offset + simple::PHASE_PROGRESS;
            let target = offset + simple::PHASE_TARGET;
            bytes[phase..phase + 8].copy_from_slice(&i64::MIN.to_le_bytes());
            bytes[target..target + 4].copy_from_slice(&i32::MAX.to_le_bytes());
            let memory = Counted {
                memory: SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                calls: Cell::new(0),
            };
            let state = read_state(&memory, 0, 0x1000).unwrap();
            assert_eq!(
                (state.phase_progress, state.phase_target),
                (i64::MIN, i32::MAX)
            );
            assert_eq!((state.prototype_progress, state.iterations), (99, i32::MAX));
            assert_eq!(memory.calls.get(), 1);
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
