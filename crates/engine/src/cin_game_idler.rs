//! CInGameIdler, the in-game application owner, distinct from CGameState.
//!
//! `read_graphics_task_completed` requires the original main-thread witness.
//! In-game activation publishes both owner globals; frontend activation sets
//! only the first. Hourly commands can execute before waiting for the previous
//! graphics task. Rendering and optional panels can mutate convoy and variant
//! state, so completion gates transport loans even on the main thread.
//! No hook or method is added. Other constants describe presentation layouts;
//! pause-writer coverage does not establish every indirect write.

use crate::{Memory, read, read_slice};

/// Active application-idler pointer. In-game activation fills both roots;
/// frontend activation fills only this root.
pub const SINGLETON_A_RVA: usize = 0x0331_72e0;
/// Active in-game idler pointer, cleared by frontend activation.
/// Transport requires the same owner as the first root.
pub const SINGLETON_B_RVA: usize = 0x0331_72e8;
/// In-game idler primary vtable.
pub const VTABLE_RVA: usize = 0x0295_2510;
/// Graphics task pointer, initially null; update retains, waits and clears it.
pub const GRAPHICS_TASK: usize = 0xa28;

pub const ROOT_UNAVAILABLE: &str = "transport graphics idler roots unavailable";
pub const ROOT_MISSING: &str = "transport graphics idler root missing";
pub const ROOT_MISMATCH: &str = "transport graphics idler roots mismatch";
pub const IDENTITY_UNAVAILABLE: &str = "transport graphics idler identity unavailable";
pub const IDENTITY_UNSUPPORTED: &str = "transport graphics idler identity unsupported";
pub const TASK_UNAVAILABLE: &str = "transport graphics idler task unavailable";

/// Copy one original completion verdict without retaining task/idler controls.
///
/// The main-thread callback prevents activation and task release. Its retained
/// wrapper keeps the item alive. A null task is absence; a pointer-read fault
/// is unavailable. A nonnull item must complete before transport loans begin.
/// Its terminal state 2 cannot restart.
pub fn read_graphics_task_completed(memory: &impl Memory, base: usize) -> Result<(), &'static str> {
    let mut roots = [0_u8; 16];
    let address = base.checked_add(SINGLETON_A_RVA).ok_or(ROOT_UNAVAILABLE)?;
    if !read_slice(memory, address, &mut roots) {
        return Err(ROOT_UNAVAILABLE);
    }
    let first = u64::from_le_bytes(roots[..8].try_into().unwrap());
    let second = u64::from_le_bytes(roots[8..].try_into().unwrap());
    if first == 0 || second == 0 {
        return Err(ROOT_MISSING);
    }
    if first != second {
        return Err(ROOT_MISMATCH);
    }
    let idler = usize::try_from(first).map_err(|_| ROOT_UNAVAILABLE)?;
    let vtable: u64 = read(memory, idler).ok_or(IDENTITY_UNAVAILABLE)?;
    if Some(vtable) != base.checked_add(VTABLE_RVA).map(|value| value as u64) {
        return Err(IDENTITY_UNSUPPORTED);
    }
    let task: u64 = idler
        .checked_add(GRAPHICS_TASK)
        .and_then(|address| read(memory, address))
        .ok_or(TASK_UNAVAILABLE)?;
    let task = usize::try_from(task).map_err(|_| TASK_UNAVAILABLE)?;
    if task == 0 {
        Ok(())
    } else {
        crate::ctask::read_completed(memory, base, task)
    }
}
/// CInGameIdler update vtable slot.
pub const UPDATE_SLOT_RVA: usize = 0x0295_2530;
/// CInGameIdler update.
pub const UPDATE_RVA: usize = 0x00db_f240;
/// Ambient-presentation dispatch result combines with input eligibility;
/// full method semantics remain unproven.
pub const AMBIENT_PRESENTATION_GATE_SLOT: usize = 0x68;
/// CInGameIdler map-container accessor slot.
pub const MAP_CONTAINER_ACCESSOR_SLOT: usize = 0x80;
/// Visibility-map accessor dispatch displacement.
pub const VISIBILITY_MAP_ACCESSOR_SLOT: usize = 0x78;
/// Order-edit preview accessor displacement; result +0x1d0 is tested,
/// while return type and declaring base remain unproven.
pub const ORDER_EDIT_PREVIEW_ACCESSOR_SLOT: usize = 0xc8;
/// CInGameIdler map-container accessor.
pub const MAP_CONTAINER_ACCESSOR_RVA: usize = 0x0029_e8c0;
/// CInGameIdler GUI registry pointer.
pub const GUI_REGISTRY: usize = 0x4f8;
/// CInGameIdler cursor-source pointer.
pub const CURSOR_SOURCE: usize = 0x500;
/// CInGameIdler action-source pointer.
pub const ACTION_SOURCE: usize = 0x508;
/// CInGameIdler embedded selection manager.
pub const SELECTION_MANAGER: usize = 0x538;
/// CInGameIdler map delegate pointer.
pub const MAP_DELEGATE: usize = 0x530;
/// The map delegate's CGraphicalMap pointer.
pub const GRAPHICAL_MAP: usize = 0x370;
/// CInGameIdler interface-handler pointer.
pub const INTERFACE_HANDLER: usize = 0x6b8;
/// CInGameIdler pause-route byte.
pub const PAUSE_ROUTE: usize = 0x6c0;
/// In-game idler paused byte, requiring zero or one.
pub const PAUSED: usize = 0x6c1;
/// Pause-kind byte, separate from paused state.
pub const PAUSE_KIND: usize = 0x6c2;
/// CInGameIdler forced-pause byte.
pub const PAUSE_FORCED: usize = 0x6c3;
/// Camera-root pointer. Its +0x08 backlink must name this owner; eye
/// and look-at values correspond to the current render view.
pub const CAMERA: usize = 0x750;
/// Event-head candidate at +0x758 aliases the camera owner pointer;
/// its interpretation remains conflicting.
pub const LEGACY_CAMERA_EVENT_HEAD: usize = 0x758;
/// CInGameIdler alert-manager pointer.
pub const ALERT_MANAGER: usize = 0x798;
/// CInGameIdler tutorial-highlight owner pointer array.
pub const TUTORIAL_HIGHLIGHT_OWNERS: usize = 0x960;
/// CInGameIdler tutorial-highlight owner count i32.
pub const TUTORIAL_HIGHLIGHT_OWNER_COUNT: usize = 0x96c;
/// CInGameIdler tutorial-highlight owner getter.
pub const GET_CURRENT_TUTORIAL_HIGHLIGHT_OWNER_RVA: usize = 0x00db_0bb0;
/// CInGameIdler forced-pause function.
pub const PAUSE_FORCED_RVA: usize = 0x00db_e260;
/// Exact forced-pause entry bytes.
pub const PAUSE_FORCED_SENTINEL: [u8; 16] = [
    0x83, 0xea, 0x09, 0x74, 0x15, 0x83, 0xea, 0x01, 0x74, 0x09, 0x83, 0xfa, 0x03, 0x75, 0x2a, 0x33,
];
/// General pause function, distinct from the player toggle.
pub const PAUSE_ENTRY_RVA: usize = 0x00dc_6520;
/// Exact pause-entry bytes.
pub const PAUSE_ENTRY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Load/setup function also writes paused state.
pub const LOAD_SETUP_RVA: usize = 0x00dc_5310;
/// CInGameIdler non-server pause function.
pub const PAUSE_NON_SERVER_RVA: usize = 0x00dc_bf00;
/// CInGameIdler player pause toggle.
pub const PLAYER_PAUSE_TOGGLE_RVA: usize = 0x00dc_d3e0;
/// CInGameIdler routed player toggle.
pub const PLAYER_PAUSE_ROUTE_RVA: usize = 0x00dc_d770;
/// Pause counter global u32.
pub const PAUSE_COUNTER_RVA: usize = 0x0337_23ec;
/// CInGameIdler vertical camera-input producer.
pub const CAMERA_VERTICAL_INPUT_RVA: usize = 0x00db_a7c0;
/// Exact vertical camera-input entry bytes.
pub const CAMERA_VERTICAL_INPUT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x88, 0x50, 0x10, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fixture {
        roots: [u8; 16],
        idler: [u8; 0xa30],
        task: [u8; 0x20],
        item: [u8; 0x60],
        fail: Option<usize>,
        reads: RefCell<Vec<usize>>,
    }
    impl Fixture {
        fn new(task: u64, state: i32) -> Self {
            let mut value = Self {
                roots: [0; 16],
                idler: [0; 0xa30],
                task: [0; 0x20],
                item: [0; 0x60],
                fail: None,
                reads: RefCell::new(Vec::new()),
            };
            value.roots[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
            value.roots[8..].copy_from_slice(&0x1000_u64.to_le_bytes());
            value.idler[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
            value.idler[GRAPHICS_TASK..GRAPHICS_TASK + 8].copy_from_slice(&task.to_le_bytes());
            value.task[..8].copy_from_slice(&(crate::ctask::VTABLE_RVA as u64).to_le_bytes());
            value.task[0x10..0x18].copy_from_slice(&0x4000_u64.to_le_bytes());
            value.item[0x58..0x5c].copy_from_slice(&state.to_le_bytes());
            value
        }
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if self.fail == Some(address) {
                return false;
            }
            let (start, bytes): (usize, &[u8]) = if address >= SINGLETON_A_RVA {
                (SINGLETON_A_RVA, &self.roots)
            } else if address >= 0x4000 {
                (0x4000, &self.item)
            } else if address >= 0x3000 {
                (0x3000, &self.task)
            } else {
                (0x1000, &self.idler)
            };
            let Some(offset) = address.checked_sub(start) else {
                return false;
            };
            let Some(end) = offset.checked_add(output.len()) else {
                return false;
            };
            let Some(source) = bytes.get(offset..end) else {
                return false;
            };
            output.copy_from_slice(source);
            true
        }
    }

    #[test]
    fn transport_graphics_idler_distinguishes_native_absence_from_running_task() {
        let memory = Fixture::new(0, 1);
        assert_eq!(read_graphics_task_completed(&memory, 0), Ok(()));
        assert_eq!(*memory.reads.borrow(), [SINGLETON_A_RVA, 0x1000, 0x1a28]);
        for (state, expected) in [
            (0, Err(crate::ctask::OUTSTANDING)),
            (1, Err(crate::ctask::OUTSTANDING)),
            (2, Ok(())),
            (3, Err(crate::ctask::STATE_UNSUPPORTED)),
        ] {
            let memory = Fixture::new(0x3000, state);
            assert_eq!(read_graphics_task_completed(&memory, 0), expected);
            assert_eq!(
                *memory.reads.borrow(),
                [SINGLETON_A_RVA, 0x1000, 0x1a28, 0x3000, 0x3010, 0x4058]
            );
        }
    }

    #[test]
    fn transport_graphics_idler_refuses_missing_mismatched_or_foreign_owner() {
        let mut memory = Fixture::new(0, 2);
        memory.roots[8..].fill(0); // Native frontend activation is not an in-game loan.
        assert_eq!(read_graphics_task_completed(&memory, 0), Err(ROOT_MISSING));
        assert_eq!(*memory.reads.borrow(), [SINGLETON_A_RVA]);
        let mut memory = Fixture::new(0, 2);
        memory.roots[8..].copy_from_slice(&0x2000_u64.to_le_bytes());
        assert_eq!(read_graphics_task_completed(&memory, 0), Err(ROOT_MISMATCH));
        assert_eq!(*memory.reads.borrow(), [SINGLETON_A_RVA]);
        let mut memory = Fixture::new(0, 2);
        memory.idler[0] ^= 1;
        assert_eq!(
            read_graphics_task_completed(&memory, 0),
            Err(IDENTITY_UNSUPPORTED)
        );
        assert_eq!(*memory.reads.borrow(), [SINGLETON_A_RVA, 0x1000]);
    }

    #[test]
    fn transport_graphics_idler_never_heals_an_original_pointer_fault() {
        for (address, reason, reads) in [
            (SINGLETON_A_RVA, ROOT_UNAVAILABLE, 1),
            (0x1000, IDENTITY_UNAVAILABLE, 2),
            (0x1a28, TASK_UNAVAILABLE, 3),
        ] {
            let mut memory = Fixture::new(0, 2);
            memory.fail = Some(address);
            assert_eq!(read_graphics_task_completed(&memory, 0), Err(reason));
            assert_eq!(memory.reads.borrow().len(), reads);
        }
        let memory = Fixture::new(0, 2);
        assert_eq!(
            read_graphics_task_completed(&memory, usize::MAX),
            Err(ROOT_UNAVAILABLE)
        );
        assert!(memory.reads.borrow().is_empty());
    }
}
