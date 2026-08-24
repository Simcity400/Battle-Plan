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
    let Some(header_before @ (base, size, mask)) = stable_header(mem, container) else {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_header_unstable",
        );
    };
    if size == 0 {
        if stable_header(mem, container) != Some(header_before) {
            return unmeasured_with_container(
                country_obj,
                container,
                "country_variables_header_changed_after_empty_read",
            );
        }
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
    // Two independent snapshots, decoded independently, and the accepted read
    // is the one both agree on - unchanged. What changed is the container the
    // comparison runs over: sorted duplicate-free pair vectors instead of two
    // `BTreeMap`s, which decides the same way and builds one map, not two.
    if let (Some(first), Some(second)) = (
        read_variables_container_snapshot(mem, base, span, size),
        read_variables_container_snapshot(mem, base, span, size),
    ) {
        if first != second {
            return unmeasured_with_container(
                country_obj,
                container,
                "country_variables_rows_changed_between_snapshots",
            );
        }
        if stable_header(mem, container) != Some(header_before) {
            return unmeasured_with_container(
                country_obj,
                container,
                "country_variables_header_changed_after_snapshot",
            );
        }
        return CountryVariableRead {
            status: VariableReadStatus::Measured,
            country_obj,
            container_addr: Some(container),
            node_base: Some(base),
            declared_size: size,
            bucket_mask: mask,
            entries: first.into_iter().collect(),
            failure_reason: None,
        };
    }

    let end = base.saturating_add(span);
    let mut scans = Vec::with_capacity(2);
    for _pass in 0..2 {
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
        scans.push(entries);
    }
    if scans[0] != scans[1] {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_rows_changed_between_phase_scans",
        );
    }
    let entries = scans.pop().expect("two completed phase scans");
    if stable_header(mem, container) != Some(header_before) {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_header_changed_after_scan",
        );
    }
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

/// One snapshot's decoded rows: sorted by key and proven duplicate-free, so
/// two of them compare exactly as the two `BTreeMap`s they used to be
/// (`BTreeMap::from_iter` over sorted duplicate-free pairs is injective on
/// this order). The map is built once, from the ACCEPTED snapshot only -
/// building it for the discarded second snapshot as well cost one heap node
/// per variable per container per frame for a value that was compared and
/// then thrown away.
type VariableSnapshotRows = Vec<(String, i64)>;

fn read_variables_container_snapshot<R: MemoryRead>(
    mem: &R,
    base: u64,
    span: u64,
    size: u32,
) -> Option<VariableSnapshotRows> {
    let span_len = usize::try_from(span).ok()?;
    if span_len == 0 || span_len > VARIABLE_TABLE_SNAPSHOT_MAX_BYTES {
        return None;
    }
    let mut bytes = vec![0u8; span_len];
    mem.read_into(base, &mut bytes)?;
    scan_variables_snapshot(mem, &bytes, size)
}

/// The part of [`read_msvc_string_from_node_snapshot`] that decides, from the
/// node bytes ALONE, that this node cannot carry a key: every rejection the
/// full reader can make before it touches memory or validates characters.
///
/// It is therefore a sound upper bound - a node the full reader accepts must
/// pass this - which is what lets the phase loop below drop a phase whose
/// candidate count is already short of `target_len` without reading a single
/// out-of-line key buffer for it. Five of the six phases are wrong by
/// construction (the node stride is 0x30 and only one 8-byte phase lines up
/// with the table), and each plausible-looking garbage node in them was
/// costing one guarded read plus one `String`.
fn node_header_admits_key(node: &[u8]) -> bool {
    let (Some(size), Some(cap)) = (read_u64_le(node, 0x10), read_u64_le(node, 0x18)) else {
        return false;
    };
    let (size, cap) = (size as usize, cap as usize);
    if size == 0 || size > MAX_KEYLEN || size > cap || cap >= (1 << 20) {
        return false;
    }
    let inline = cap == 15;
    inline || cap >= 16
}

fn scan_variables_snapshot<R: MemoryRead>(
    mem: &R,
    bytes: &[u8],
    size: u32,
) -> Option<VariableSnapshotRows> {
    let target_len = size as usize;
    let mut exact: Option<VariableSnapshotRows> = None;
    let mut entries: VariableSnapshotRows = Vec::with_capacity(target_len);
    let mut phase = 0usize;
    while phase < NODE_STRIDE_USIZE {
        // Sound-upper-bound sweep first: pure arithmetic over the snapshot,
        // no reads, no allocation, same node bounds as the parse loop below.
        // `read_i64_le(node, NODE_VALUE)` cannot fail on a 0x30-byte node, so
        // the parse loop's only early exit is a bound this sweep shares.
        let mut candidates = 0usize;
        let mut offset = phase;
        while offset.checked_add(NODE_STRIDE_USIZE)? <= bytes.len() {
            if node_header_admits_key(&bytes[offset..offset + NODE_STRIDE_USIZE]) {
                candidates += 1;
            }
            offset = offset.checked_add(NODE_STRIDE_USIZE)?;
        }
        if candidates >= target_len {
            entries.clear();
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
                    match exact.as_ref() {
                        Some(existing) if existing != &entries => return None,
                        // Already recorded and equal: the old spelling
                        // reassigned an equal value here.
                        Some(_) => {}
                        None => exact = Some(std::mem::take(&mut entries)),
                    }
                }
            }
        }
        phase = phase.checked_add(8)?;
    }
    exact
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

    // ---------------------------------------------------------------
    // Lane SE: the phase-loop candidate pre-count.
    //
    // `scan_variables_snapshot` now sweeps each phase for nodes whose header
    // bytes could carry a key BEFORE it parses that phase, and skips the
    // phase when the count is already short of the declared size. The claim
    // is that the pre-count is a SOUND UPPER BOUND, so the accepted rows and
    // every refusal are unchanged; these pin both halves of that.
    // ---------------------------------------------------------------

    /// The pre-change parse loop, restated: no pre-count, every phase parsed.
    /// The reference half of the equivalence below.
    fn scan_variables_snapshot_unfiltered<R: MemoryRead>(
        mem: &R,
        bytes: &[u8],
        size: u32,
    ) -> Option<VariableSnapshotRows> {
        let target_len = size as usize;
        let mut exact: Option<VariableSnapshotRows> = None;
        let mut phase = 0usize;
        while phase < NODE_STRIDE_USIZE {
            let mut entries: VariableSnapshotRows = Vec::new();
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
        exact
    }

    fn snapshot_with_inline_rows(rows: &[(&str, i64)], slack_nodes: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; (rows.len() + slack_nodes) * NODE_STRIDE_USIZE];
        for (index, (key, value)) in rows.iter().enumerate() {
            let base = index * NODE_STRIDE_USIZE;
            put_inline_string(&mut bytes, base, key);
            put_i64(&mut bytes, base + NODE_VALUE_USIZE, *value);
        }
        bytes
    }

    #[test]
    fn the_precounted_phase_scan_decodes_exactly_what_the_unfiltered_scan_did() {
        let mem = FixtureMemory::default();
        let rows = [("air_doctrine", 250_000i64), ("zeta", -7), ("mid_key^2", 0)];
        for slack in [1usize, 4, 9] {
            let bytes = snapshot_with_inline_rows(&rows, slack);
            for declared in [rows.len() as u32, rows.len() as u32 + 1, 0] {
                assert_eq!(
                    scan_variables_snapshot(&mem, &bytes, declared),
                    scan_variables_snapshot_unfiltered(&mem, &bytes, declared),
                    "slack {slack} declared {declared}"
                );
            }
        }
    }

    #[test]
    fn a_one_byte_header_edit_moves_the_precount_and_both_scans_together() {
        // kills: a pre-count that is not a sound upper bound. Clearing the
        // `cap` word of one node takes that node out of the candidate set, so
        // phase 0 can no longer reach the declared size and the read must
        // refuse - and the unfiltered scan, which never counted anything,
        // must refuse for the same reason at the same input.
        let mem = FixtureMemory::default();
        let rows = [("alpha", 1i64), ("beta", 2), ("gamma", 3)];
        let mut bytes = snapshot_with_inline_rows(&rows, 3);
        assert!(scan_variables_snapshot(&mem, &bytes, 3).is_some());

        let node = NODE_STRIDE_USIZE; // "beta"
        put_u64(&mut bytes, node + 0x18, 0);
        assert_eq!(scan_variables_snapshot(&mem, &bytes, 3), None);
        assert_eq!(
            scan_variables_snapshot(&mem, &bytes, 3),
            scan_variables_snapshot_unfiltered(&mem, &bytes, 3),
        );
    }

    #[test]
    fn the_precount_still_refuses_a_table_two_phases_can_read_differently() {
        // kills: a pre-count that skipped a phase which WOULD have produced a
        // full, duplicate-free, DIFFERENT row set - the ambiguity the scan
        // exists to refuse. Two single-row readings at two different phases,
        // both reaching the declared size, must still fail the container.
        // Phase 0 reads one row out of the node at 0x00; phase 8 reads a
        // DIFFERENT one out of the node at 0x38. The two node images do not
        // overlap (an inline node occupies [base, base + 0x28)), so both
        // readings are genuinely available and the container is ambiguous.
        let mem = FixtureMemory::default();
        let mut bytes = vec![0u8; 6 * NODE_STRIDE_USIZE];
        put_inline_string(&mut bytes, 0, "phase_zero");
        put_i64(&mut bytes, NODE_VALUE_USIZE, 11);
        put_inline_string(&mut bytes, 0x38, "phase_eight");
        put_i64(&mut bytes, 0x38 + NODE_VALUE_USIZE, 22);
        assert_eq!(
            scan_variables_snapshot_unfiltered(&mem, &bytes, 1),
            None,
            "the fixture must be ambiguous for the pre-count to have anything to preserve"
        );

        assert_eq!(scan_variables_snapshot(&mem, &bytes, 1), None);
        assert_eq!(
            scan_variables_snapshot(&mem, &bytes, 1),
            scan_variables_snapshot_unfiltered(&mem, &bytes, 1),
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
