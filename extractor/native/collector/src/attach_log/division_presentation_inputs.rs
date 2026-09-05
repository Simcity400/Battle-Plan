use super::*;
use compact_contract::division_routes::MAX_ROUTE_PROVINCES;

const ORDINARY_ROUTE: usize = 0x1f0;
const ALTERNATE_ROUTE: usize = 0x210;

#[derive(PartialEq, Eq)]
struct RouteRead {
    identity: [u64; 4],
    selected: usize,
    header: [u64; 3],
    provinces: Vec<u32>,
}

fn division_identity<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<[u64; 4]> {
    let identity = [
        mem.u64(body)?,
        mem.u32(body.checked_add(DIVISION_OFF_ID_TYPE)?)? as u64,
        mem.u32(body.checked_add(DIVISION_OFF_ID_ID)?)? as u64,
        mem.u32(body.checked_add(DIVISION_OFF_LOCATION)?)? as u64,
    ];
    if identity[0] != module.checked_add(DIVISION_VTABLE_RVA)? as u64
        || !is_compact_division_body_type(identity[1] as u32)
        || identity[2] != id as u64
    {
        return None;
    }
    Some(identity)
}

#[derive(PartialEq, Eq)]
struct TemplateRead {
    identity: [u64; 4],
    override_pointer: usize,
    selected_pointer: usize,
    template: Option<u32>,
}

fn template_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<TemplateRead> {
    let identity = division_identity(mem, module, body, id)?;
    // The collector body is CDivision + 0x10; the current template is CDivision + 0x3c8.
    let override_pointer = mem.u64(body.checked_add(0x3b8)?)? as usize;
    let selected_pointer = if override_pointer == 0 {
        mem.u64(body.checked_add(DIVISION_OFF_TEMPLATE)?)? as usize
    } else {
        override_pointer
    };
    let template = if selected_pointer == 0 {
        None
    } else {
        if !is_map_control_heap_pointer(selected_pointer)
            || mem.u64(selected_pointer)?
                != module.checked_add(DIVISION_TEMPLATE_VTABLE_RVA)? as u64
            || mem.u32(selected_pointer.checked_add(DIVISION_TEMPLATE_OFF_TYPE)?)?
                != DIVISION_TEMPLATE_TYPE
        {
            return None;
        }
        let template_id = mem.u32(selected_pointer.checked_add(DIVISION_TEMPLATE_OFF_ID)?)?;
        if template_id == 0 || template_id > MAX_DIVISION_TEMPLATE_ID {
            return None;
        }
        Some(template_id)
    };
    Some(TemplateRead {
        identity,
        override_pointer,
        selected_pointer,
        template,
    })
}

fn stable_template<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<Option<u32>> {
    let first = template_once(mem, module, body, id)?;
    between();
    let second = template_once(mem, module, body, id)?;
    (first == second).then_some(first.template)
}

pub(super) fn current_template(module: usize, body: usize, id: u32) -> Option<Option<u32>> {
    stable_template(&LIVE_MEMORY, module, body, id, || {})
}

fn route_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<RouteRead> {
    let identity = division_identity(mem, module, body, id)?;
    let alternate_count = mem.u32(body.checked_add(ALTERNATE_ROUTE + 12)?)?;
    let selected = if alternate_count != 0 {
        ALTERNATE_ROUTE
    } else {
        ORDINARY_ROUTE
    };
    let at = body.checked_add(selected)?;
    let data = mem.u64(at)?;
    let capacity = mem.u32(at.checked_add(8)?)?;
    let count = mem.u32(at.checked_add(12)?)?;
    if count > capacity || count as usize > MAX_ROUTE_PROVINCES {
        return None;
    }
    let mut provinces = Vec::new();
    if count != 0 {
        if !is_map_control_heap_pointer(data as usize) {
            return None;
        }
        let bytes = mem.bytes(data as usize, (count as usize).checked_mul(4)?)?;
        for bytes in bytes.chunks_exact(4) {
            let province = u32::from_le_bytes(bytes.try_into().ok()?);
            if !valid_division_province_id(province) {
                return None;
            }
            provinces.push(province);
        }
    }
    Some(RouteRead {
        identity,
        selected,
        header: [data, capacity as u64, count as u64],
        provinces,
    })
}

fn stable_route<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<Vec<u32>> {
    let first = route_once(mem, module, body, id)?;
    between();
    let second = route_once(mem, module, body, id)?;
    (first == second).then_some(first.provinces)
}

pub(super) fn current_route(module: usize, body: usize, id: u32) -> Option<Vec<u32>> {
    stable_route(&LIVE_MEMORY, module, body, id, || {})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<usize, u8>>);
    impl Memory {
        fn put(&self, at: usize, bytes: &[u8]) {
            self.0
                .borrow_mut()
                .extend(bytes.iter().enumerate().map(|(i, b)| (at + i, *b)));
        }
        fn put32(&self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn put64(&self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn vector(&self, body: usize, offset: usize, data: usize, provinces: &[u32]) {
            self.put64(body + offset, data as u64);
            self.put32(body + offset + 8, provinces.len() as u32);
            self.put32(body + offset + 12, provinces.len() as u32);
            for (index, province) in provinces.iter().enumerate() {
                self.put32(data + index * 4, *province);
            }
        }
    }
    impl DecodeMemory for Memory {
        fn scalar<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
            self.bytes(at, N)?.try_into().ok()
        }
        fn bytes_with_reason(&self, at: usize, len: usize) -> Result<Vec<u8>, GuardedReadRefusal> {
            (at..at + len)
                .map(|key| {
                    self.0
                        .borrow()
                        .get(&key)
                        .copied()
                        .ok_or(GuardedReadRefusal::OutsideConfirmedRanges)
                })
                .collect()
        }
        fn fill(&self, at: usize, buf: &mut [u8]) -> bool {
            let Some(bytes) = self.bytes(at, buf.len()) else {
                return false;
            };
            buf.copy_from_slice(&bytes);
            true
        }
    }
    fn fixture() -> (Memory, usize, usize) {
        let mem = Memory::default();
        let module = 0x140000000;
        let body = HEAP_LO + 0x10000;
        mem.put64(body, (module + DIVISION_VTABLE_RVA) as u64);
        mem.put32(body + DIVISION_OFF_ID_TYPE, DIVISION_TYPE);
        mem.put32(body + DIVISION_OFF_ID_ID, 7);
        mem.put32(body + DIVISION_OFF_LOCATION, 12);
        mem.vector(body, ORDINARY_ROUTE, body + 0x10000, &[19, 17, 19]);
        mem.vector(body, ALTERNATE_ROUTE, 0, &[]);
        (mem, module, body)
    }
    #[test]
    fn current_template_uses_only_witnessed_null_fallback_and_preserves_absence() {
        let (mem, module, body) = fixture();
        let standard = body + 0x30000;
        let current = body + 0x40000;
        for (at, id) in [(standard, 61), (current, 72)] {
            mem.put64(at, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
            mem.put32(at + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
            mem.put32(at + DIVISION_TEMPLATE_OFF_ID, id);
        }
        mem.put64(body + DIVISION_OFF_TEMPLATE, standard as u64);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), None);
        mem.put64(body + 0x3b8, 0);
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(61))
        );
        mem.put64(body + 0x3b8, current as u64);
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(72))
        );
        mem.0.borrow_mut().remove(&(body + DIVISION_OFF_TEMPLATE));
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(72))
        );
        mem.put32(current + DIVISION_TEMPLATE_OFF_TYPE, 70);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), None);
        mem.put32(current + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
        assert_eq!(
            stable_template(&mem, module, body, 7, || mem
                .put32(current + DIVISION_TEMPLATE_OFF_ID, 73)),
            None
        );
        mem.put64(body + 0x3b8, 0);
        mem.put64(body + DIVISION_OFF_TEMPLATE, 0);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), Some(None));
        assert_eq!(
            stable_template(&mem, module, body, 7, || mem
                .put32(body + DIVISION_OFF_ID_ID, 8)),
            None
        );
    }

    #[test]
    fn selected_route_preserves_engine_order_and_alternate_precedence() {
        let (mem, module, body) = fixture();
        assert_eq!(
            stable_route(&mem, module, body, 7, || {}),
            Some(vec![19, 17, 19])
        );
        mem.vector(body, ALTERNATE_ROUTE, body + 0x20000, &[22, 21]);
        assert_eq!(
            stable_route(&mem, module, body, 7, || {}),
            Some(vec![22, 21])
        );
        mem.0.borrow_mut().remove(&(body + ORDINARY_ROUTE));
        assert_eq!(
            stable_route(&mem, module, body, 7, || {}),
            Some(vec![22, 21])
        );
    }
    #[test]
    fn selected_route_distinguishes_empty_unread_invalid_and_changed() {
        let (mem, module, body) = fixture();
        mem.vector(body, ORDINARY_ROUTE, 0, &[]);
        assert_eq!(stable_route(&mem, module, body, 7, || {}), Some(vec![]));
        mem.0.borrow_mut().remove(&(body + ALTERNATE_ROUTE + 12));
        assert_eq!(stable_route(&mem, module, body, 7, || {}), None);
        let (mem, module, body) = fixture();
        assert_eq!(
            stable_route(&mem, module, body, 7, || mem.put32(body + 0x10000, 20)),
            None
        );
        assert_eq!(
            stable_route(&mem, module, body, 7, || mem
                .put32(body + DIVISION_OFF_ID_ID, 8)),
            None
        );
        let (mem, module, body) = fixture();
        mem.put32(body + ORDINARY_ROUTE + 12, MAX_ROUTE_PROVINCES as u32 + 1);
        assert_eq!(stable_route(&mem, module, body, 7, || {}), None);
        mem.vector(body, ORDINARY_ROUTE, body + 0x10000, &[0]);
        assert_eq!(stable_route(&mem, module, body, 7, || {}), None);
    }
}
