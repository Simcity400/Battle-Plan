
use engine_native_memory::{Memory, read};

pub const IDLER_CAMERA: usize = 0x760;
pub const OWNER: usize = 0x8;
pub const RENDER_VIEW_EYE: usize = 0x190;
pub const EYE: usize = 0x680;
pub const VIEWPORT: usize = 0x6c0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    pub look_at: [f32; 3],
    pub viewport: [i32; 4],
}

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
