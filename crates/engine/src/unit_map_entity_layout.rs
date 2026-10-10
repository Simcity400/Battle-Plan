//! Map entity owned by unit +0x298. The 0x88-byte presentation object is
//! distinct from the larger transform consumer. Route and movement
//! progress change with render time; these fields do not establish an
//! hourly simulation table.

/// Map-entity allocation size.
pub const SIZE: usize = 0x88;
/// Unit pointer at the entity base, followed through unit +0x1f0 province.
pub const UNIT: usize = 0;
/// Movement selector, i32, at +0x14.
pub const MOVEMENT_SELECTOR: usize = 0x14;
/// Province-ID route pointer; the second u32 is read only when count is at least two.
pub const ROUTE: usize = 0x68;
/// Route count, i32; values below two skip target selection.
pub const ROUTE_COUNT: usize = 0x74;
/// Route element size; the second province ID is at route +4.
pub const ROUTE_ENTRY_STRIDE: usize = 4;
/// Target-province query returning a low-byte Boolean and the second route ID.
pub const TARGET_PROVINCE_RVA: usize = 0x0163_1210;
/// Movement predicate testing the +0x14 i32 for nonzero.
pub const MOVEMENT_SELECTOR_PREDICATE_RVA: usize = 0x00ba_faf0;
/// Render-time movement advance following the unit pointer and route.
pub const ADVANCE_MOVEMENT_RVA: usize = 0x0163_2130;

/// Owned operands consumed by the native map-entity stack selector.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackInputs {
    pub movement_selector: i32,
    pub target_province: Option<u32>,
}

#[cfg(test)]
impl StackInputs {
    pub fn project(self) -> (bool, Option<u32>) {
        (self.movement_selector != 0, self.target_province)
    }
}

/// Acquire only the map entity operands consumed by the native stack selector.
/// None means CUnit has no map entity; callers must report the stack kind
/// unavailable, not invent a stationary one. Failed child reads retain their
/// original errors. Presentation writers are not certified by the hourly
/// simulation boundary, so retain the terminal backlink and route-header gates.
/// These gates detect changed endpoints; they do not establish writer exclusion.
#[cfg(test)]
pub fn capture_stack_inputs(
    memory: &impl crate::Memory,
    pointer: usize,
    unit: usize,
) -> Result<Option<StackInputs>, &'static str> {
    use crate::{cpdx_array::Array, read};
    if pointer == 0 {
        return Ok(None);
    }
    if read::<u64>(memory, pointer) != Some(unit as u64) {
        return Err("map entity unit backlink mismatch");
    }
    let selector = pointer
        .checked_add(MOVEMENT_SELECTOR)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("map entity selector unreadable")?;
    let route_header = pointer
        .checked_add(ROUTE)
        .and_then(|p| read::<[u8; 16]>(memory, p))
        .ok_or("map entity route invalid")?;
    let route =
        Array::from_header(route_header, ROUTE_ENTRY_STRIDE).ok_or("map entity route invalid")?;
    let target = if route.count < 2 {
        None
    } else {
        Some(
            route
                .element(1, ROUTE_ENTRY_STRIDE)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("map entity target unreadable")?,
        )
    };
    if read::<u64>(memory, pointer) != Some(unit as u64)
        || pointer
            .checked_add(ROUTE)
            .and_then(|p| read::<[u8; 16]>(memory, p))
            != Some(route_header)
    {
        return Err("map entity changed during read");
    }
    Ok(Some(StackInputs {
        movement_selector: selector,
        target_province: target,
    }))
}

#[cfg(test)]
pub fn stack_inputs(
    memory: &impl crate::Memory,
    pointer: usize,
    unit: usize,
) -> Result<Option<(bool, Option<u32>)>, &'static str> {
    Ok(capture_stack_inputs(memory, pointer, unit)?.map(StackInputs::project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Memory;
    use std::cell::RefCell;

    struct Source {
        bytes: [u8; 0x100],
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<(usize, bool)>,
    }

    impl Memory for Source {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            if !(crate::memory::SliceMemory {
                base: 0x1000,
                bytes: &self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if let Some((at, fail)) = self.fault
                && at == ordinal
            {
                output[0] ^= 1;
                return !fail;
            }
            true
        }
    }

    fn source() -> Source {
        let mut bytes = [0; 0x100];
        bytes[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
        bytes[MOVEMENT_SELECTOR..MOVEMENT_SELECTOR + 4].copy_from_slice(&1_i32.to_le_bytes());
        bytes[ROUTE..ROUTE + 8].copy_from_slice(&0x1080_u64.to_le_bytes());
        bytes[ROUTE + 8..ROUTE + 12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[ROUTE + 12..ROUTE + 16].copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x84..0x88].copy_from_slice(&7_u32.to_le_bytes());
        Source {
            bytes,
            reads: RefCell::default(),
            fault: None,
        }
    }

    #[test]
    fn stack_inputs_reject_changed_or_failed_terminal_endpoints_without_retry() {
        let memory = source();
        assert_eq!(
            stack_inputs(&memory, 0x1000, 0x2000),
            Ok(Some((true, Some(7))))
        );
        let trace = memory.reads.into_inner();
        assert_eq!(trace.len(), 6);
        for ordinal in [4, 5] {
            for fail in [false, true] {
                let mut memory = source();
                memory.fault = Some((ordinal, fail));
                assert_eq!(
                    stack_inputs(&memory, 0x1000, 0x2000),
                    Err("map entity changed during read")
                );
                assert_eq!(*memory.reads.borrow(), trace[..=ordinal]);
            }
        }
    }
}
