
use engine_sites::Rva;
pub const SINGLETON_RVA: Rva = Rva(0x033d_d448);
pub const DEFINITIONS: usize = 0xb0;
pub const DEFINITIONS_COUNT: usize = 0xbc;
pub const TYPE_VTABLE_RVA: Rva = Rva(0x029b_e3f8);
pub const TYPE_TOKEN: usize = 0x08;
pub const ESSENTIAL_EQUIPMENT: usize = 0xa40;
pub const ADDITIONAL_EQUIPMENT: usize = 0xa28;
pub const NODE_EQUIPMENT: usize = 0x20;
pub const NODE_AMOUNT: usize = 0x28;

pub fn equipment_map(
    memory: &impl engine_native_memory::Memory,
    at: usize,
    unreadable: &'static str,
    counted: impl FnOnce(u64) -> Result<(), &'static str>,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    fn field<T: engine_native_memory::memory::Raw>(
        memory: &impl engine_native_memory::Memory,
        at: usize,
        offset: usize,
        unreadable: &'static str,
    ) -> Result<T, &'static str> {
        at.checked_add(offset)
            .filter(|_| at != 0)
            .and_then(|at| engine_native_memory::read(memory, at))
            .ok_or(unreadable)
    }
    let pointer = |at, offset| field::<u64>(memory, at, offset, unreadable).map(|p| p as usize);
    let nil = |at| field::<u8>(memory, at, 0x19, unreadable);
    let head = pointer(at, 0)?;
    let count = field::<u64>(memory, at, 8, unreadable)?;
    if nil(head)? != 1 {
        return Err("raid equipment order sentinel invalid");
    }
    let mut node = pointer(head, 0)?;
    counted(count)?;
    for _ in 0..count {
        if node == head || nil(node)? != 0 {
            return Err("raid equipment order ended early");
        }
        visit(node)?;
        let right = pointer(node, 0x10)?;
        let mut steps = 0;
        if nil(right)? == 0 {
            node = right;
            loop {
                let left = pointer(node, 0)?;
                if nil(left)? != 0 {
                    break;
                }
                node = left;
                steps += 1;
                if steps > count {
                    return Err("raid equipment order cycle");
                }
            }
        } else {
            let mut parent = pointer(node, 8)?;
            while nil(parent)? == 0 && node == pointer(parent, 0x10)? {
                node = parent;
                parent = pointer(node, 8)?;
                steps += 1;
                if steps > count {
                    return Err("raid equipment order parent cycle");
                }
            }
            node = parent;
        }
    }
    if node != head {
        return Err("raid equipment order exceeds stored count");
    }
    Ok(())
}
