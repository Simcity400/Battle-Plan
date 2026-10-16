//! Raid-definition database used by country-raid selection, distinct from the raid-system object.

/// Raid-definition database global pointer, required to be nonnull.
pub const SINGLETON_RVA: usize = 0x0331_6c30;
/// Raid-definition pointer-array data.
pub const DEFINITIONS: usize = 0xb0;
/// Raid-definition signed i32 count.
pub const DEFINITIONS_COUNT: usize = 0xbc;
/// Raid-type definition primary vtable.
pub const TYPE_VTABLE_RVA: usize = 0x0292_a6c8;
/// Raid type definition `+8`, its lexer token.
pub const TYPE_TOKEN: usize = 0x08;
/// A raid type's `essential_equipment` map at `+0xA40`, passed to stockpile
/// requirement checking.
pub const ESSENTIAL_EQUIPMENT: usize = 0xa40;
/// Its `additional_equipment` map.
pub const ADDITIONAL_EQUIPMENT: usize = 0xa28;
/// An equipment map node's equipment-type key, read from `node[4]` by
/// requirement checking.
pub const NODE_EQUIPMENT: usize = 0x20;
/// An equipment map node's i32 amount, compared from `node[5]` by requirement
/// checking.
pub const NODE_AMOUNT: usize = 0x28;

/// Visits, in order, every node of the equipment map (an MSVC `std::map`:
/// head `+0`, count `+8`; a node's left `+0`, parent `+8`, right `+0x10` and
/// nil byte `+0x19`) at `at`, after passing its count to `counted`. A read
/// that fails, or a null object, is `unreadable`; a malformed tree fails by
/// name.
pub fn equipment_map(
    memory: &impl crate::Memory,
    at: usize,
    unreadable: &'static str,
    counted: impl FnOnce(u64) -> Result<(), &'static str>,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    fn field<T: crate::memory::Raw>(
        memory: &impl crate::Memory,
        at: usize,
        offset: usize,
        unreadable: &'static str,
    ) -> Result<T, &'static str> {
        at.checked_add(offset)
            .filter(|_| at != 0)
            .and_then(|at| crate::read(memory, at))
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
