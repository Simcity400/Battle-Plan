//! The in-game idler's active camera root: eye, look-at and viewport, read by tools outside the
//! game. The root has no vtable to authenticate it, so a read holds only when the root names the
//! idler as its owner and its embedded current render view repeats the root's eye and look-at.

use crate::{Memory, read};

/// In-game idler member: the active camera root pointer.
pub const IDLER_CAMERA: usize = 0x760;
/// The camera root's owning idler.
pub const OWNER: usize = 0x8;
/// The embedded current render view's eye then look-at, three f32 each.
pub const RENDER_VIEW_EYE: usize = 0x190;
/// Eye then look-at, three f32 each.
pub const EYE: usize = 0x680;
/// Viewport x, y, width and height, i32 each.
pub const VIEWPORT: usize = 0x6c0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    pub look_at: [f32; 3],
    pub viewport: [i32; 4],
}

/// The camera of the in-game idler at `idler`; `None` when it has none or the root does not verify.
pub fn read_camera(memory: &impl Memory, idler: usize) -> Option<Camera> {
    let root = read::<u64>(memory, idler.checked_add(IDLER_CAMERA)?)?;
    let root = usize::try_from(root).ok().filter(|&root| root != 0)?;
    let at = |offset: usize| root.checked_add(offset);
    if read::<u64>(memory, at(OWNER)?)? != idler as u64 {
        return None;
    }
    let view = read::<[u8; 24]>(memory, at(EYE)?)?;
    if read::<[u8; 24]>(memory, at(RENDER_VIEW_EYE)?)? != view {
        return None;
    }
    let viewport = read::<[u8; 16]>(memory, at(VIEWPORT)?)?;
    let word = |bytes: &[u8], n: usize| {
        [
            bytes[n * 4],
            bytes[n * 4 + 1],
            bytes[n * 4 + 2],
            bytes[n * 4 + 3],
        ]
    };
    Some(Camera {
        eye: std::array::from_fn(|n| f32::from_le_bytes(word(&view, n))),
        look_at: std::array::from_fn(|n| f32::from_le_bytes(word(&view, 3 + n))),
        viewport: std::array::from_fn(|n| i32::from_le_bytes(word(&viewport, n))),
    })
}
