
use engine_native_memory::{Memory, cpdx_array::Array, msvc_tree_node, read};

pub const COUNTRY: usize = 0x20;
pub const TURNS: usize = 0x28;

pub struct Node {
    pub left: usize,
    pub parent: usize,
    pub right: usize,
    pub nil: bool,
    pub country: u32,
    pub turns: Array,
}

pub fn copy(memory: &impl Memory, pointer: usize) -> Result<Node, &'static str> {
    let bytes = read::<[u8; 0x38]>(memory, pointer).ok_or("peace history node unreadable")?;
    let link = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
    let nil = bytes[msvc_tree_node::NIL];
    if nil > 1 {
        return Err("peace history nil flag invalid");
    }
    Ok(Node {
        left: link(msvc_tree_node::LEFT),
        right: link(msvc_tree_node::RIGHT),
        nil: nil != 0,
        parent: link(msvc_tree_node::PARENT),
        country: u32::from_le_bytes(bytes[COUNTRY..COUNTRY + 4].try_into().unwrap()),
        turns: if nil != 0 {
            Array::default()
        } else {
            Array::from_header(bytes[TURNS..TURNS + 16].try_into().unwrap(), 8)
                .ok_or("peace history turn array invalid")?
        },
    })
}
