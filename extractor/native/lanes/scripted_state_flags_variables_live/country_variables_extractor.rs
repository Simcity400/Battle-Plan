use std::collections::BTreeMap;

pub const EXPECTED_SHA_16022851: &str =
    "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120";

pub const OBJ_VARIABLES: u64 = 0x218;
pub const RTTI_VARIABLES: &str = ".?AVCVariables@@";
pub const CV_BASE: u64 = 0x18;
pub const CV_SIZE_MASK: u64 = 0x20;
pub const NODE_STRIDE: u64 = 0x30;
pub const NODE_VALUE: u64 = 0x20;
pub const VALUE_SCALE_RAW: i64 = 100_000;
pub const CVARIABLES_HEADER_WINDOW_LEN: usize = (CV_SIZE_MASK + 8) as usize;
pub const MSVC_STRING_WINDOW_LEN: usize = 0x20;

pub const HEAP_LO: u64 = 0x10000000000;
pub const HEAP_HI: u64 = 0x7FF000000000;
pub const MODULE_SPAN_MAX: u64 = 0x10000000;
pub const MAX_SIZE: u64 = 200_000;
pub const MAX_BUCKETS: u64 = 1 << 21;
pub const MAX_KEYLEN: usize = 256;
const VARIABLE_TABLE_SNAPSHOT_MAX_BYTES: usize = 512 * 1024;
const NODE_STRIDE_USIZE: usize = NODE_STRIDE as usize;
const NODE_VALUE_USIZE: usize = NODE_VALUE as usize;

pub trait MemoryRead {
    fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>>;

    fn read_into(&self, addr: u64, buf: &mut [u8]) -> Option<()> {
        let bytes = self.read_exact(addr, buf.len())?;
        if bytes.len() != buf.len() {
            return None;
        }
        buf.copy_from_slice(&bytes);
        Some(())
    }

    fn read_u64(&self, addr: u64) -> Option<u64> {
        let mut bytes = [0u8; 8];
        self.read_into(addr, &mut bytes)?;
        Some(u64::from_le_bytes(bytes))
    }

    fn read_i64(&self, addr: u64) -> Option<i64> {
        let mut bytes = [0u8; 8];
        self.read_into(addr, &mut bytes)?;
        Some(i64::from_le_bytes(bytes))
    }

    fn read_u32(&self, addr: u64) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read_into(addr, &mut bytes)?;
        Some(u32::from_le_bytes(bytes))
    }

    fn read_object_window<const N: usize>(&self, _addr: u64) -> Option<[u8; N]> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariableReadStatus {
    Measured,
    MeasuredEmpty,
    Unmeasured,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryVariableRead {
    pub status: VariableReadStatus,
    pub country_obj: u64,
    pub container_addr: Option<u64>,
    pub node_base: Option<u64>,
    pub declared_size: u32,
    pub bucket_mask: u32,
    pub entries: BTreeMap<String, i64>,
    pub failure_reason: Option<&'static str>,
}

impl CountryVariableRead {
    pub fn unmeasured(country_obj: u64, reason: &'static str) -> Self {
        Self {
            status: VariableReadStatus::Unmeasured,
            country_obj,
            container_addr: None,
            node_base: None,
            declared_size: 0,
            bucket_mask: 0,
            entries: BTreeMap::new(),
            failure_reason: Some(reason),
        }
    }
}

pub fn read_country_variables<R: MemoryRead>(
    mem: &R,
    module_base: u64,
    country_obj: u64,
) -> CountryVariableRead {
    let Some(container) = mem.read_u64(country_obj.saturating_add(OBJ_VARIABLES)) else {
        return CountryVariableRead::unmeasured(
            country_obj,
            "country_variables_pointer_read_failed",
        );
    };
    if !heap_ok(container) {
        return CountryVariableRead::unmeasured(
            country_obj,
            "country_variables_pointer_out_of_heap",
        );
    }
    if rtti_type_name(mem, module_base, container).as_deref() != Some(RTTI_VARIABLES) {
        return CountryVariableRead::unmeasured(country_obj, "country_variables_rtti_mismatch");
    }
    read_variables_container(mem, country_obj, container)
}

pub fn read_variables_container<R: MemoryRead>(
    mem: &R,
    country_obj: u64,
    container: u64,
) -> CountryVariableRead {
    let Some((base, size, mask)) = stable_header(mem, container) else {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_header_unstable",
        );
    };
    if size == 0 {
        return CountryVariableRead {
            status: VariableReadStatus::MeasuredEmpty,
            country_obj,
            container_addr: Some(container),
            node_base: Some(base),
            declared_size: 0,
            bucket_mask: mask,
            entries: BTreeMap::new(),
            failure_reason: None,
        };
    }
    let nb = (mask as u64).saturating_add(1);
    if !heap_ok(base) || size as u64 > MAX_SIZE || nb == 0 || nb > MAX_BUCKETS {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_header_invalid",
        );
    }
    if ((mask as u64) & nb) != 0 || size as u64 > nb {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_bucket_mask_invalid",
        );
    }

    let span = nb.saturating_add(4).saturating_mul(NODE_STRIDE);
    if let Some(entries) = read_variables_container_snapshot(mem, base, span, size) {
        return CountryVariableRead {
            status: VariableReadStatus::Measured,
            country_obj,
            container_addr: Some(container),
            node_base: Some(base),
            declared_size: size,
            bucket_mask: mask,
            entries,
            failure_reason: None,
        };
    }

    let end = base.saturating_add(span);
    let mut exact: Option<BTreeMap<String, i64>> = None;
    let mut phase = 0u64;
    while phase < NODE_STRIDE {
        let mut tmp = BTreeMap::new();
        let mut dup = false;
        let mut addr = base.saturating_add(phase);
        while addr.saturating_add(NODE_STRIDE) <= end {
            if let Some(key) = read_msvc_string(mem, addr) {
                if tmp.contains_key(&key) {
                    dup = true;
                }
                if let Some(raw) = mem.read_i64(addr.saturating_add(NODE_VALUE)) {
                    tmp.insert(key, raw);
                }
            }
            addr = addr.saturating_add(NODE_STRIDE);
        }
        if !dup && tmp.len() == size as usize {
            if let Some(existing) = exact.as_ref() {
                if existing != &tmp {
                    return unmeasured_with_container(
                        country_obj,
                        container,
                        "country_variables_phase_ambiguous",
                    );
                }
            }
            exact = Some(tmp);
        }
        phase += 8;
    }

    let Some(entries) = exact else {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_size_gate_failed",
        );
    };
    CountryVariableRead {
        status: VariableReadStatus::Measured,
        country_obj,
        container_addr: Some(container),
        node_base: Some(base),
        declared_size: size,
        bucket_mask: mask,
        entries,
        failure_reason: None,
    }
}

pub fn rtti_type_name<R: MemoryRead>(mem: &R, module_base: u64, obj: u64) -> Option<String> {
    let vt = mem.read_u64(obj)?;
    if !in_module(module_base, vt) {
        return None;
    }
    let col = mem.read_u64(vt.saturating_sub(8))?;
    if !in_module(module_base, col) {
        return None;
    }
    let type_descriptor =
        module_base.saturating_add(mem.read_u32(col.saturating_add(0x0C))? as u64);
    read_cstr(mem, type_descriptor.saturating_add(0x10), 200).filter(|name| name.starts_with(".?A"))
}

pub fn read_msvc_string<R: MemoryRead>(mem: &R, addr: u64) -> Option<String> {
    if let Some(window) = mem.read_object_window::<MSVC_STRING_WINDOW_LEN>(addr) {
        return read_msvc_string_from_header(mem, &window);
    }
    let size = mem.read_u64(addr.saturating_add(0x10))? as usize;
    let cap = mem.read_u64(addr.saturating_add(0x18))? as usize;
    if size == 0 || size > MAX_KEYLEN || size > cap || cap >= (1 << 20) {
        return None;
    }
    let inline = cap == 15;
    if !inline && cap < 16 {
        return None;
    }
    let chars = if inline { addr } else { mem.read_u64(addr)? };
    read_msvc_string_bytes(mem, chars, size)
}

fn stable_header<R: MemoryRead>(mem: &R, container: u64) -> Option<(u64, u32, u32)> {
    let (b1, w1) = stable_header_pair(mem, container)?;
    let (b2, w2) = stable_header_pair(mem, container)?;
    if b1 != b2 || w1 != w2 {
        return None;
    }
    Some((b1, (w1 & 0xFFFF_FFFF) as u32, (w1 >> 32) as u32))
}

fn stable_header_pair<R: MemoryRead>(mem: &R, container: u64) -> Option<(u64, u64)> {
    if let Some(window) = mem.read_object_window::<CVARIABLES_HEADER_WINDOW_LEN>(container) {
        return Some((
            read_u64_le(&window, CV_BASE as usize)?,
            read_u64_le(&window, CV_SIZE_MASK as usize)?,
        ));
    }
    let addr = container.saturating_add(CV_BASE);
    let mut bytes = [0u8; 16];
    if mem.read_into(addr, &mut bytes).is_some() {
        return Some((read_u64_le(&bytes, 0)?, read_u64_le(&bytes, 8)?));
    }
    Some((
        mem.read_u64(container.saturating_add(CV_BASE))?,
        mem.read_u64(container.saturating_add(CV_SIZE_MASK))?,
    ))
}

fn read_variables_container_snapshot<R: MemoryRead>(
    mem: &R,
    base: u64,
    span: u64,
    size: u32,
) -> Option<BTreeMap<String, i64>> {
    let span_len = usize::try_from(span).ok()?;
    if span_len == 0 || span_len > VARIABLE_TABLE_SNAPSHOT_MAX_BYTES {
        return None;
    }
    let mut bytes = vec![0u8; span_len];
    mem.read_into(base, &mut bytes)?;
    scan_variables_snapshot(mem, &bytes, size)
}

fn scan_variables_snapshot<R: MemoryRead>(
    mem: &R,
    bytes: &[u8],
    size: u32,
) -> Option<BTreeMap<String, i64>> {
    let target_len = size as usize;
    let mut exact: Option<Vec<(String, i64)>> = None;
    let mut phase = 0usize;
    while phase < NODE_STRIDE_USIZE {
        let mut entries = Vec::new();
        let mut offset = phase;
        while offset.checked_add(NODE_STRIDE_USIZE)? <= bytes.len() {
            let node = &bytes[offset..offset + NODE_STRIDE_USIZE];
            if let Some(key) = read_msvc_string_from_node_snapshot(mem, node) {
                let raw = read_i64_le(node, NODE_VALUE_USIZE)?;
                entries.push((key, raw));
            }
            offset = offset.checked_add(NODE_STRIDE_USIZE)?;
        }
        if entries.len() == target_len {
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            if !entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                if let Some(existing) = exact.as_ref() {
                    if existing != &entries {
                        return None;
                    }
                }
                exact = Some(entries);
            }
        }
        phase = phase.checked_add(8)?;
    }
    exact.map(|entries| entries.into_iter().collect())
}

fn read_msvc_string_from_node_snapshot<R: MemoryRead>(mem: &R, node: &[u8]) -> Option<String> {
    let size = read_u64_le(node, 0x10)? as usize;
    let cap = read_u64_le(node, 0x18)? as usize;
    if size == 0 || size > MAX_KEYLEN || size > cap || cap >= (1 << 20) {
        return None;
    }
    let inline = cap == 15;
    if !inline && cap < 16 {
        return None;
    }
    if inline {
        let data = node.get(..size.checked_add(1)?)?;
        read_msvc_string_from_bytes(data, size)
    } else {
        read_msvc_string_bytes(mem, read_u64_le(node, 0)?, size)
    }
}

fn read_msvc_string_from_header<R: MemoryRead>(mem: &R, header: &[u8]) -> Option<String> {
    let size = read_u64_le(header, 0x10)? as usize;
    let cap = read_u64_le(header, 0x18)? as usize;
    if size == 0 || size > MAX_KEYLEN || size > cap || cap >= (1 << 20) {
        return None;
    }
    let inline = cap == 15;
    if !inline && cap < 16 {
        return None;
    }
    if inline {
        let data = header.get(..size.checked_add(1)?)?;
        read_msvc_string_from_bytes(data, size)
    } else {
        read_msvc_string_bytes(mem, read_u64_le(header, 0)?, size)
    }
}

fn read_msvc_string_bytes<R: MemoryRead>(mem: &R, addr: u64, size: usize) -> Option<String> {
    let len = size.checked_add(1)?;
    if len > MAX_KEYLEN + 1 {
        return None;
    }
    let mut data = [0u8; MAX_KEYLEN + 1];
    mem.read_into(addr, &mut data[..len])?;
    read_msvc_string_from_bytes(&data[..len], size)
}

fn read_msvc_string_from_bytes(data: &[u8], size: usize) -> Option<String> {
    if data.get(size).copied() != Some(0) {
        return None;
    }
    let key = std::str::from_utf8(data.get(..size)?).ok()?;
    if key.bytes().all(|b| (32..=126).contains(&b)) {
        Some(key.to_owned())
    } else {
        None
    }
}

fn read_cstr<R: MemoryRead>(mem: &R, addr: u64, max_len: usize) -> Option<String> {
    let bytes = mem.read_exact(addr, max_len)?;
    let end = bytes.iter().position(|b| *b == 0)?;
    String::from_utf8(bytes[..end].to_vec()).ok()
}

fn unmeasured_with_container(
    country_obj: u64,
    container: u64,
    reason: &'static str,
) -> CountryVariableRead {
    let mut read = CountryVariableRead::unmeasured(country_obj, reason);
    read.container_addr = Some(container);
    read
}

fn heap_ok(value: u64) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value)
}

fn in_module(module_base: u64, value: u64) -> bool {
    (module_base..module_base.saturating_add(MODULE_SPAN_MAX)).contains(&value)
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    let mut out = [0u8; 8];
    out.copy_from_slice(bytes.get(offset..offset.checked_add(8)?)?);
    Some(u64::from_le_bytes(out))
}

fn read_i64_le(bytes: &[u8], offset: usize) -> Option<i64> {
    let mut out = [0u8; 8];
    out.copy_from_slice(bytes.get(offset..offset.checked_add(8)?)?);
    Some(i64::from_le_bytes(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(Clone, Default)]
    struct FixtureMemory {
        segments: Vec<(u64, Vec<u8>)>,
    }

    impl FixtureMemory {
        fn add(&mut self, addr: u64, bytes: Vec<u8>) {
            self.segments.push((addr, bytes));
        }
    }

    impl MemoryRead for FixtureMemory {
        fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            for (base, bytes) in &self.segments {
                let Some(start64) = addr.checked_sub(*base) else {
                    continue;
                };
                let start = start64 as usize;
                let end = start.checked_add(len)?;
                if end <= bytes.len() {
                    return Some(bytes[start..end].to_vec());
                }
            }
            None
        }
    }

    struct ContiguousFixtureMemory {
        inner: FixtureMemory,
        refused_windows: BTreeSet<u64>,
    }

    impl ContiguousFixtureMemory {
        fn new(inner: FixtureMemory) -> Self {
            Self {
                inner,
                refused_windows: BTreeSet::new(),
            }
        }

        fn refusing(mut self, addr: u64) -> Self {
            self.refused_windows.insert(addr);
            self
        }
    }

    impl MemoryRead for ContiguousFixtureMemory {
        fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            self.inner.read_exact(addr, len)
        }

        fn read_object_window<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
            if self.refused_windows.contains(&addr) {
                return None;
            }
            let bytes = self.inner.read_exact(addr, N)?;
            bytes.try_into().ok()
        }
    }

    fn complete_country_variable_fixture() -> (u64, u64, FixtureMemory) {
        let module_base = 0x7FF7_0000_0000;
        let country = 0x2000_0000_0000;
        let container = 0x2000_0000_1000;
        let node_base = 0x2000_0000_3000;
        let mut mem = FixtureMemory::default();

        let mut country_bytes = vec![0u8; 0x300];
        put_u64(&mut country_bytes, OBJ_VARIABLES as usize, container);
        mem.add(country, country_bytes);

        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, 0, module_base + 0x5000);
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            2 | (3u64 << 32),
        );
        mem.add(container, container_bytes);

        let mut vtable_prefix = vec![0u8; 16];
        put_u64(&mut vtable_prefix, 0, module_base + 0x6000);
        mem.add(module_base + 0x5000 - 8, vtable_prefix);

        let mut col = vec![0u8; 0x20];
        put_u32(&mut col, 0x0C, 0x7000);
        mem.add(module_base + 0x6000, col);

        let mut type_descriptor = vec![0u8; 0x100];
        type_descriptor[0x10..0x10 + RTTI_VARIABLES.len()]
            .copy_from_slice(RTTI_VARIABLES.as_bytes());
        mem.add(module_base + 0x7000, type_descriptor);

        let mut nodes = vec![0u8; ((4 + 4) * NODE_STRIDE) as usize];
        put_inline_string(&mut nodes, 0, "foo");
        put_i64(&mut nodes, NODE_VALUE as usize, 1_100_000);
        put_inline_string(&mut nodes, NODE_STRIDE as usize, "bar^0");
        put_i64(
            &mut nodes,
            NODE_STRIDE as usize + NODE_VALUE as usize,
            -1_073_741_542,
        );
        mem.add(node_base, nodes);

        (module_base, country, mem)
    }

    #[test]
    fn extracts_complete_country_variable_table() {
        let module_base = 0x7FF7_0000_0000;
        let country = 0x2000_0000_0000;
        let container = 0x2000_0000_1000;
        let node_base = 0x2000_0000_3000;
        let mut mem = FixtureMemory::default();

        let mut country_bytes = vec![0u8; 0x300];
        put_u64(&mut country_bytes, OBJ_VARIABLES as usize, container);
        mem.add(country, country_bytes);

        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, 0, module_base + 0x5000);
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            2 | (3u64 << 32),
        );
        mem.add(container, container_bytes);

        let mut vtable_prefix = vec![0u8; 16];
        put_u64(&mut vtable_prefix, 0, module_base + 0x6000);
        mem.add(module_base + 0x5000 - 8, vtable_prefix);

        let mut col = vec![0u8; 0x20];
        put_u32(&mut col, 0x0C, 0x7000);
        mem.add(module_base + 0x6000, col);

        let mut type_descriptor = vec![0u8; 0x100];
        type_descriptor[0x10..0x10 + RTTI_VARIABLES.len()]
            .copy_from_slice(RTTI_VARIABLES.as_bytes());
        mem.add(module_base + 0x7000, type_descriptor);

        let mut nodes = vec![0u8; ((4 + 4) * NODE_STRIDE) as usize];
        put_inline_string(&mut nodes, 0, "foo");
        put_i64(&mut nodes, NODE_VALUE as usize, 1_100_000);
        put_inline_string(&mut nodes, NODE_STRIDE as usize, "bar^0");
        put_i64(
            &mut nodes,
            NODE_STRIDE as usize + NODE_VALUE as usize,
            -1_073_741_542,
        );
        mem.add(node_base, nodes);

        let read = read_country_variables(&mem, module_base, country);
        assert_eq!(read.status, VariableReadStatus::Measured, "{read:?}");
        assert_eq!(read.declared_size, 2);
        assert_eq!(read.entries.get("foo").copied(), Some(1_100_000));
        assert_eq!(read.entries.get("bar^0").copied(), Some(-1_073_741_542));
    }

    #[test]
    fn bulk_window_country_variables_match_per_field_path() {
        let (module_base, country, mem) = complete_country_variable_fixture();
        let per_field = read_country_variables(&mem, module_base, country);
        let bulk = read_country_variables(&ContiguousFixtureMemory::new(mem), module_base, country);

        assert_eq!(bulk, per_field);
        assert_eq!(bulk.status, VariableReadStatus::Measured);
    }

    #[test]
    fn refused_window_country_variables_falls_back_to_per_field_path() {
        let (module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let per_field = read_country_variables(&mem, module_base, country);
        let refused = read_country_variables(
            &ContiguousFixtureMemory::new(mem).refusing(container),
            module_base,
            country,
        );

        assert_eq!(refused, per_field);
        assert_eq!(refused.status, VariableReadStatus::Measured);
    }

    #[test]
    fn distinguishes_measured_empty_from_unmeasured() {
        let country = 0x2000_0001_0000;
        let container = 0x2000_0001_1000;
        let mut mem = FixtureMemory::default();
        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, CV_BASE as usize, 0x2000_0001_3000);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            0 | (0u64 << 32),
        );
        mem.add(container, container_bytes);

        let read = read_variables_container(&mem, country, container);
        assert_eq!(read.status, VariableReadStatus::MeasuredEmpty);
        assert!(read.entries.is_empty());

        let failed = read_variables_container(&mem, country, 0x2000_0001_2000);
        assert_eq!(failed.status, VariableReadStatus::Unmeasured);
        assert_eq!(
            failed.failure_reason,
            Some("country_variables_header_unstable")
        );
    }

    #[test]
    fn rejects_non_reconciling_node_phase() {
        let country = 0x2000_0002_0000;
        let container = 0x2000_0002_1000;
        let node_base = 0x2000_0002_3000;
        let mut mem = FixtureMemory::default();
        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            2 | (3u64 << 32),
        );
        mem.add(container, container_bytes);

        let mut nodes = vec![0u8; ((4 + 4) * NODE_STRIDE) as usize];
        put_inline_string(&mut nodes, 0, "only_one");
        put_i64(&mut nodes, NODE_VALUE as usize, 100_000);
        mem.add(node_base, nodes);

        let read = read_variables_container(&mem, country, container);
        assert_eq!(read.status, VariableReadStatus::Unmeasured);
        assert_eq!(
            read.failure_reason,
            Some("country_variables_size_gate_failed")
        );
    }

    fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
        buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_i64(buf: &mut [u8], offset: usize, value: i64) {
        buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
        buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_inline_string(buf: &mut [u8], offset: usize, value: &str) {
        let bytes = value.as_bytes();
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
        buf[offset + bytes.len()] = 0;
        put_u64(buf, offset + 0x10, bytes.len() as u64);
        put_u64(buf, offset + 0x18, 15);
    }
}
