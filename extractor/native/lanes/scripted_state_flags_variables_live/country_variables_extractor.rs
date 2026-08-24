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

/// Whether a re-witness **whose interval is the row walk** is owed, given how
/// many rows the read just walked.
///
/// A re-witness re-reads something and requires it unchanged. It is worth its
/// reads exactly when something else was read in between - that interval is
/// the whole point, because what it rules out is a structure moving under a
/// read that has already happened. An interval with no read in it cannot
/// contain such a read, so closing it proves nothing the opening sample did
/// not already prove.
///
/// This predicate answers ONE question: *did the walk happen?* It therefore
/// gates exactly the re-witnesses whose interval IS the walk - the container
/// head re-sample that stands immediately after the row loop and immediately
/// after the head sample when there is no loop. When the container declared
/// zero entries, those two points are the same point, and the head pair has
/// already sampled it twice.
///
/// **It does NOT gate a re-witness whose interval opened earlier.** d51's
/// scope readers re-read the OWNER'S ROOT POINTER at the end, and that
/// interval opens when the root is first read - before the RTTI type-name
/// walk, before the container head, and before any row. Those reads sit inside
/// it whether or not a single row was walked, so the root re-witness is owed
/// unconditionally and this predicate must never be asked about it. Codex
/// round 1 caught exactly that mis-scoping: the first cut of this lane gated
/// both root re-reads on `rewitness_owed` and turned "the owner's pointer
/// moved while we were reading its container" from a refusal into an accepted
/// empty scope.
///
/// **The predicate is the guard, and the list of call sites is not.** A scope
/// that walked ANYTHING still owes its head re-witness, however few rows it
/// found. Both d51 scope readers and this container reader ask this function
/// in production rather than testing their own emptiness inline, so there is
/// one place to be right and one place to test.
pub fn rewitness_owed(rows_walked: usize) -> bool {
    rows_walked != 0
}

pub fn read_variables_container<R: MemoryRead>(
    mem: &R,
    country_obj: u64,
    container: u64,
) -> CountryVariableRead {
    read_variables_container_inner(mem, country_obj, container, None)
}

/// [`read_variables_container`] with the container's vtable authenticated out
/// of the SAME header window this read already samples twice.
///
/// The caller's cached-RTTI gate, once the frame's vtable is known, is
/// `read_u64_stable(container).is_some_and(|v| v == expected)` - true exactly
/// when two samples of the vtable agreed AND equalled the cache, which is the
/// same proposition as EACH of two samples equalling the cache. The vtable
/// lives at container+0x00, already inside the 0x28-byte window
/// [`stable_header`] reads twice, so the caller's separate pair of guarded
/// reads is free to delete: 2 reads x every WARM variable scope x every hour.
/// The frame's first container is not warm and does not come through here.
///
/// A caller that has no cached vtable yet must NOT use this - it has to walk
/// the RTTI type name first - so this takes the expected value rather than an
/// `Option`. Refusal is reported as `country_variables_header_unstable` rather
/// than as an RTTI mismatch; the collector keeps only the `status` from this
/// read and never the reason, so both spellings reach the caller as the same
/// `Unmeasured` scope.
pub fn read_variables_container_vtable_pinned<R: MemoryRead>(
    mem: &R,
    country_obj: u64,
    container: u64,
    expected_vtable: u64,
) -> CountryVariableRead {
    read_variables_container_inner(mem, country_obj, container, Some(expected_vtable))
}

fn read_variables_container_inner<R: MemoryRead>(
    mem: &R,
    country_obj: u64,
    container: u64,
    expected_vtable: Option<u64>,
) -> CountryVariableRead {
    let Some(header_before @ (base, size, mask)) =
        stable_header_vtable_pinned(mem, container, expected_vtable)
    else {
        return unmeasured_with_container(
            country_obj,
            container,
            "country_variables_header_unstable",
        );
    };
    if !rewitness_owed(size as usize) {
        // The predicate decides in RELEASE, not in a debug assertion: a
        // container that declared zero entries is never walked, so the second
        // `stable_header` that used to stand here re-read the very bytes
        // `header_before` was sampled from with nothing in between. The
        // non-empty paths below still re-witness, because they walked.
        //
        // The vtable pin above does not reopen that window. On the window
        // path it rides the same two reads the header pair already took, and
        // on the fallback path its two reads come AFTER both header samples,
        // so the interval between them is untouched either way. The caller's
        // root read is still the caller's to re-witness, and it does.
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
        // OWED: two snapshots of the bucket span were read between
        // `header_before` and here. See `rewitness_owed`.
        debug_assert!(rewitness_owed(first.len()));
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
    // OWED: the phase scan read the bucket span between `header_before` and
    // here. See `rewitness_owed`.
    debug_assert!(rewitness_owed(entries.len()));
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

/// [`stable_header`] with the vtable of `container` required, in BOTH samples,
/// to equal `expected_vtable`. With `None` it IS [`stable_header`], call for
/// call. See [`read_variables_container_vtable_pinned`] for why requiring the
/// value in each sample is the caller's old predicate and not a stricter one.
///
/// Only the FIRST header read of the container is pinned. The post-read
/// re-witness keeps calling plain [`stable_header`], because the pre-lane-SE
/// re-witness compared `(base, size, mask)` and never looked at the vtable;
/// pinning it too would refuse containers the old code admitted.
fn stable_header_vtable_pinned<R: MemoryRead>(
    mem: &R,
    container: u64,
    expected_vtable: Option<u64>,
) -> Option<(u64, u32, u32)> {
    let Some(expected) = expected_vtable else {
        return stable_header(mem, container);
    };
    // The header pair is sampled EXACTLY the way `stable_header` samples it:
    // one `stable_header_pair` per sample, window first and per-field
    // otherwise, INDEPENDENTLY for each of the two.
    //
    // That per-SAMPLE fallback is load-bearing twice over, and both times
    // because lane SE got it wrong first:
    //
    //  * a per-CALL fallback interleaved the reads as `vtable1, header1,
    //    vtable2, header2` where the old path read `vtable1, vtable2, header1,
    //    header2`, widening the header pair's interval by a vtable read, so a
    //    header moving during that extra read refused a frame the old code
    //    accepted (Codex round 4, finding 1);
    //  * an ALL-OR-NOTHING fallback discarded a successful first window sample
    //    when the second window was refused and then compared two fresh
    //    windowless samples, so a header that moved from A to B was accepted
    //    as B/B where the old code refused it (Codex round 5, finding 1).
    //
    // Sampling independently is what makes a window sample get compared
    // against a per-field sample. All this call adds is the vtable, when the
    // window path is the one that ran and hands it over for free.
    let (vtable1, base1, word1) = stable_header_pair_with_vtable(mem, container)?;
    let (vtable2, base2, word2) = stable_header_pair_with_vtable(mem, container)?;
    if base1 != base2 || word1 != word2 {
        return None;
    }
    // The vtable is an ABSOLUTE test - it equalled `expected` at two instants -
    // so which two instants they are changes nothing about what is asserted.
    // Free when both header samples came out of a window; otherwise the
    // pre-lane-SE scalar pair, spelled out rather than short-circuited so the
    // rejecting path reads what it always read. Those two reads come AFTER
    // both header samples, never between them.
    match (vtable1, vtable2) {
        (Some(first), Some(second)) => {
            if first != expected || second != expected {
                return None;
            }
        }
        _ => {
            let first = mem.read_u64(container)?;
            let second = mem.read_u64(container)?;
            if first != expected || second != expected {
                return None;
            }
        }
    }
    Some((base1, (word1 & 0xFFFF_FFFF) as u32, (word1 >> 32) as u32))
}

/// One sample of the CVariables header: the same read [`stable_header_pair`]
/// makes, reporting the vtable at container+0x00 as well WHEN the window path
/// is the one that ran.
///
/// `None` for the vtable means "this sample did not come from a window", never
/// "the vtable is wrong": the per-field fallback starts at container+CV_BASE
/// and never covers offset 0.
fn stable_header_pair_with_vtable<R: MemoryRead>(
    mem: &R,
    container: u64,
) -> Option<(Option<u64>, u64, u64)> {
    if let Some(window) = mem.read_object_window::<CVARIABLES_HEADER_WINDOW_LEN>(container) {
        return Some((
            Some(read_u64_le(&window, 0)?),
            read_u64_le(&window, CV_BASE as usize)?,
            read_u64_le(&window, CV_SIZE_MASK as usize)?,
        ));
    }
    let (base, word) = stable_header_pair_windowless(mem, container)?;
    Some((None, base, word))
}

fn stable_header<R: MemoryRead>(mem: &R, container: u64) -> Option<(u64, u32, u32)> {
    let (b1, w1) = stable_header_pair(mem, container)?;
    let (b2, w2) = stable_header_pair(mem, container)?;
    if b1 != b2 || w1 != w2 {
        return None;
    }
    Some((b1, (w1 & 0xFFFF_FFFF) as u32, (w1 >> 32) as u32))
}

/// Unchanged in reads and in order: the window branch and the per-field
/// fallback both live in [`stable_header_pair_with_vtable`] now, and this
/// drops the vtable that branch reports.
fn stable_header_pair<R: MemoryRead>(mem: &R, container: u64) -> Option<(u64, u64)> {
    let (_vtable, base, word) = stable_header_pair_with_vtable(mem, container)?;
    Some((base, word))
}

/// The tail of [`stable_header_pair`] - the two per-field paths it falls to
/// when the object window is unavailable - split out so a caller that has
/// ALREADY had the window refused can reach it without asking again.
fn stable_header_pair_windowless<R: MemoryRead>(mem: &R, container: u64) -> Option<(u64, u64)> {
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
    // Lane SE: the vtable-pinned container read.
    //
    // The collector's cached-RTTI gate used to spend its own pair of guarded
    // reads on container+0x00 before every container read. That address is
    // inside the header window this read samples twice anyway, so the pin
    // moved in here. These pin that it is the SAME predicate and that a
    // container failing it never gets its bucket span walked.
    // ---------------------------------------------------------------

    #[test]
    fn the_vtable_pinned_read_is_the_unpinned_read_when_the_vtable_matches() {
        let (module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        assert_eq!(vtable, module_base + 0x5000);

        let unpinned = read_variables_container(&mem, country, container);
        let pinned = read_variables_container_vtable_pinned(&mem, country, container, vtable);
        assert_eq!(pinned.status, VariableReadStatus::Measured);
        assert_eq!(pinned, unpinned);
    }

    /// Every read that reaches the fixture, recorded, so a test can say what
    /// was NOT read as well as what the answer was.
    struct RecordingFixtureMemory {
        inner: FixtureMemory,
        reads: std::cell::RefCell<Vec<(u64, usize)>>,
    }

    impl RecordingFixtureMemory {
        fn new(inner: FixtureMemory) -> Self {
            Self {
                inner,
                reads: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn touched(&self, start: u64, len: u64) -> bool {
            self.reads
                .borrow()
                .iter()
                .any(|(addr, read_len)| *addr < start + len && start < *addr + *read_len as u64)
        }
    }

    impl MemoryRead for RecordingFixtureMemory {
        fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            self.reads.borrow_mut().push((addr, len));
            self.inner.read_exact(addr, len)
        }
    }

    #[test]
    fn a_container_whose_vtable_is_not_the_pinned_one_is_unmeasured_and_unwalked() {
        // kills: a pin that is never applied, applied to only one of the two
        // header samples, or applied AFTER the bucket walk. The collector
        // turns this into the same `Unmeasured` scope the pre-lane-SE RTTI
        // mismatch produced, and it must reach that verdict without reading a
        // byte of the node array - the recorded read log is what proves the
        // second half, which the refusal alone does not (Codex round 3,
        // finding 4).
        let (_module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        let node_base = mem.read_u64(container + CV_BASE).unwrap();
        let mask = (mem.read_u64(container + CV_SIZE_MASK).unwrap() >> 32) as u64;
        let span = (mask + 1 + 4) * NODE_STRIDE;

        let recording = RecordingFixtureMemory::new(mem);
        let wrong =
            read_variables_container_vtable_pinned(&recording, country, container, vtable ^ 1);
        assert_eq!(wrong.status, VariableReadStatus::Unmeasured);
        assert_eq!(
            wrong.failure_reason,
            Some("country_variables_header_unstable")
        );
        assert!(wrong.entries.is_empty());
        assert_eq!(wrong.declared_size, 0);
        assert!(
            !recording.touched(node_base, span),
            "the bucket span [{node_base:#x}, +{span:#x}) was read before the pin refused: {:?}",
            recording.reads.borrow()
        );

        // The same fixture with the RIGHT vtable does read the span, so the
        // assertion above is not passing because the span is unreachable.
        let recording = RecordingFixtureMemory::new(complete_country_variable_fixture().2);
        let right = read_variables_container_vtable_pinned(&recording, country, container, vtable);
        assert_eq!(right.status, VariableReadStatus::Measured);
        assert!(recording.touched(node_base, span));
    }

    /// A memory whose WINDOW view and whose PER-FIELD view can disagree, with
    /// a budget of window reads. It reproduces the one shape a single fixture
    /// cannot: a header sampled once through the window and once through the
    /// per-field fallback, with the value moving in between.
    struct SplitViewFixtureMemory {
        window: FixtureMemory,
        per_field: FixtureMemory,
        window_budget: std::cell::Cell<usize>,
    }

    impl MemoryRead for SplitViewFixtureMemory {
        fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            self.per_field.read_exact(addr, len)
        }

        fn read_object_window<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
            let budget = self.window_budget.get();
            if budget == 0 {
                return None;
            }
            self.window_budget.set(budget - 1);
            self.window.read_exact(addr, N)?.try_into().ok()
        }
    }

    /// One window read available, showing `window_size_mask` at `container`,
    /// over a complete per-field fixture that shows its own.
    fn one_window_then_per_field(
        container: u64,
        vtable: u64,
        node_base: u64,
        window_size_mask: u64,
        per_field: FixtureMemory,
    ) -> SplitViewFixtureMemory {
        let mut window = FixtureMemory::default();
        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, 0, vtable);
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            window_size_mask,
        );
        window.add(container, container_bytes);
        SplitViewFixtureMemory {
            window,
            per_field,
            window_budget: std::cell::Cell::new(1),
        }
    }

    #[test]
    fn a_header_that_moves_between_a_window_sample_and_a_per_field_sample_is_refused() {
        // kills: an ALL-OR-NOTHING window fallback. `stable_header` samples the
        // header twice and each sample independently takes the window or the
        // per-field path, so when only the first window reads it compares that
        // window sample against a per-field one and refuses a header that
        // moved. Discarding a successful first sample and re-reading both
        // per-field accepts B/B and never sees A -> B (Codex round 5,
        // finding 1).
        let (_module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        let node_base = mem.read_u64(container + CV_BASE).unwrap();
        let settled = mem.read_u64(container + CV_SIZE_MASK).unwrap();

        let moved =
            one_window_then_per_field(container, vtable, node_base, 5 | (7u64 << 32), mem.clone());
        assert_ne!(5 | (7u64 << 32), settled, "the fixture must actually move");
        assert_eq!(
            read_variables_container_vtable_pinned(&moved, country, container, vtable).status,
            VariableReadStatus::Unmeasured
        );

        // Positive control: the same one-window budget over a header that did
        // NOT move reads clean, so the refusal above is the movement and not
        // the budget.
        let still = one_window_then_per_field(container, vtable, node_base, settled, mem);
        assert_eq!(
            read_variables_container_vtable_pinned(&still, country, container, vtable).status,
            VariableReadStatus::Measured
        );
    }

    #[test]
    fn the_vtable_pinned_samplers_reach_their_fallback_only_when_the_window_fails() {
        // Codex round 4, the collector-side twin of the same finding. A
        // window-then-fallback function is only worth its saving if the
        // fallback is LAZY: `window.or(windowless(mem, container))` keeps the
        // same call graph and evaluates its argument eagerly, so every
        // SUCCESSFUL window would also take the per-field read it replaced -
        // doubling the reads, invalidating the price, and widening the interval
        // the pair is meant to close. `or_else`, `let .. else`, `match` and an
        // early `return` all keep the fallback behind the failure.
        //
        // The ban is WHOLE-FILE over the production half, not a set of
        // functions picked out of it. Two narrower shapes were tried and both
        // leaked:
        //
        //  * a LIST of names went stale twice. Round 4 wrote it; Codex round 5
        //    found it already missing `stable_header_pair_windowless`, which
        //    carries a two-level fallback of its own - a 16-byte `read_into`,
        //    then two scalar reads - so `.or(Some((mem.read_u64(..)?,
        //    mem.read_u64(..)?)))` there would take both scalar reads after a
        //    SUCCESSFUL bulk read, inside the interval between this sample and
        //    its pair.
        //  * selecting by SHAPE - "every function that takes a bulk read" -
        //    is not call-graph closed, as Codex round 6 showed: leave the bulk
        //    read where it is and move the CHOICE between it and the fallback
        //    into a new function, and the new function takes no bulk read, is
        //    not selected, and is where the eager combinator lives. It also
        //    has to enumerate bulk-read spellings, and `read_exact` was
        //    already a spelling it did not know.
        //
        // This file's production half is 695 lines and uses none of the four
        // today, so the rule costs nothing and cannot be walked around: there
        // is no function in it to extract a chooser INTO that the scan does
        // not see. `or_else`, `unwrap_or_else`, `let .. else`, `match` and an
        // early `return` are all still available, and all keep the fallback
        // behind the failure.
        let source = include_str!("country_variables_extractor.rs").replace("\r\n", "\n");
        let production = match source.find("\n#[cfg(test)]") {
            Some(end) => &source[..end],
            None => panic!("the test module is at the end of this file"),
        };
        // Comments stripped first: the prose above quotes `.or(` while
        // explaining it, and prose is not code.
        let code = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("fn read_variables_container_vtable_pinned"),
            "the production slice must hold this lane's readers; the test \
             module marker moved and the scan is looking at nothing"
        );
        for eager in [".or(", ".unwrap_or(", ".map_or(", ".and("] {
            assert_eq!(
                code.matches(eager).count(),
                0,
                "the production half of this file uses the EAGER combinator \
                 {eager}. On a window-then-fallback read that takes the \
                 per-field read even when the bulk read succeeded, which \
                 doubles the reads, invalidates the price, and widens the \
                 interval between a field's two samples. Use `or_else`, \
                 `let .. else` or `match`."
            );
        }
    }

    #[test]
    fn the_vtable_pin_survives_a_refused_header_window() {
        // The window is where the vtable rides for free; when it is refused
        // the pin costs its own read instead of being silently dropped.
        let (_module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        let refusing = ContiguousFixtureMemory::new(mem).refusing(container);

        let pinned = read_variables_container_vtable_pinned(&refusing, country, container, vtable);
        assert_eq!(pinned.status, VariableReadStatus::Measured);
        let wrong =
            read_variables_container_vtable_pinned(&refusing, country, container, vtable ^ 1);
        assert_eq!(wrong.status, VariableReadStatus::Unmeasured);
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

    // ---------------------------------------------------------------
    // Lane SE2, owner ruling: the post-read re-witness is removed where it
    // has nothing to witness, and `rewitness_owed` - not a list of call
    // sites - is what decides. These pin both halves of that.
    // ---------------------------------------------------------------

    /// A memory whose view of ONE address changes after a budget of reads,
    /// so a test can move a header underneath a reader mid-read.
    struct MovingHeaderFixture {
        before: FixtureMemory,
        after: FixtureMemory,
        moving: u64,
        budget: std::cell::Cell<usize>,
    }

    impl MovingHeaderFixture {
        fn read_from(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            if addr != self.moving {
                return self.before.read_exact(addr, len);
            }
            let budget = self.budget.get();
            if budget == 0 {
                return self.after.read_exact(addr, len);
            }
            self.budget.set(budget - 1);
            self.before.read_exact(addr, len)
        }
    }

    impl MemoryRead for MovingHeaderFixture {
        fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            self.read_from(addr, len)
        }
    }

    /// The complete fixture, plus a second view of the container whose
    /// size/mask word differs, swapped in after `budget` reads of the
    /// container head.
    fn container_head_moves_after(
        container: u64,
        vtable: u64,
        node_base: u64,
        moved_size_mask: u64,
        budget: usize,
        before: FixtureMemory,
    ) -> MovingHeaderFixture {
        let mut after = FixtureMemory::default();
        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, 0, vtable);
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(&mut container_bytes, CV_SIZE_MASK as usize, moved_size_mask);
        after.add(container, container_bytes);
        MovingHeaderFixture {
            before,
            after,
            // The head is not read AT `container`. These fixtures serve no
            // object window, so `stable_header_pair` falls through to its
            // `read_into` arm and reads ONE 16-byte window at
            // `container + CV_BASE`, covering the base pointer and the packed
            // size/mask word together. Pointing `moving` at `container` made
            // the "after" image unreachable, so nothing ever moved and the
            // moved-header tests passed by never testing anything (landing
            // gate, first execution).
            moving: container.saturating_add(CV_BASE),
            budget: std::cell::Cell::new(budget),
        }
    }

    #[test]
    fn the_rewitness_is_owed_exactly_when_the_read_walked_something() {
        // The guard itself. Zero rows means no walk happened between the head
        // pair and the re-witness, so there is no window to close; anything
        // above zero means there is.
        assert!(!rewitness_owed(0));
        assert!(rewitness_owed(1));
        assert!(rewitness_owed(2));
        assert!(rewitness_owed(100_000));
    }

    /// The same text with its comment LINES dropped.
    fn without_comments(text: &str) -> String {
        text.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The brace depth at which `needle` sits inside a function's text, counting
    /// from the function's own opening brace. **Depth 1 is the function's own
    /// statement level**; an enclosing `if`, `match`, `for` or closure raises it.
    ///
    /// This, and not indentation, is what "unconditional" means: Rust ignores
    /// whitespace, so a wrapper written without re-indenting its contents leaves
    /// a pinned line byte-for-byte intact (Codex round 5). And only braces that
    /// are CODE count, because `if mask <= 3 { // }` closes nothing in Rust and
    /// closes the block in a naive counter - a wrapper no pinned line and no
    /// naive depth would see (Codex round 6).
    ///
    /// It refuses to guess at what it does not lex: an apostrophe is either a
    /// char literal or a lifetime and this does not tell them apart, so it
    /// asserts there is none rather than mis-count one. Callers pass text that
    /// has already been through `without_comments`, which is what keeps ordinary
    /// English prose out; what is left for this scanner is the inline comment
    /// that shares a line with code, which is exactly the evasion above.
    ///
    /// The twin of this helper lives in the collector's `d51_rewitness_tests`.
    /// Two copies, because they are test scaffolding in two different crates and
    /// neither belongs in a production API.
    ///
    /// KNOWN LIMIT, recorded rather than closed: a raw string (`r#".."#`) is
    /// lexed as an ordinary one, so one containing an internal quote and a
    /// closing brace would mis-count. No scanned function contains a raw string,
    /// and writing one there is not an accidental edit (Codex round 8).
    fn statement_depth(body: &str, needle: &str) -> usize {
        assert!(
            !body.contains('\''),
            "this scanner does not lex char literals or lifetimes; this function \
             contains neither, and one appearing means the helper needs widening \
             rather than trusting"
        );
        let at = body
            .find(needle)
            .unwrap_or_else(|| panic!("not in this function:\n{needle}"));
        let bytes = body.as_bytes();
        let mut depth = 0usize;
        let mut index = 0usize;
        while index < at {
            match (bytes[index], bytes.get(index + 1).copied()) {
                (b'/', Some(b'/')) => {
                    while index < at && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                (b'/', Some(b'*')) => {
                    // NESTED, because Rust's block comments nest: stopping at the
                    // first `*/` reads `/* outer /* inner */ } */` as closing early
                    // and hands the brace after it back to the counter (Codex
                    // round 7, finding 3).
                    let mut nesting = 1usize;
                    index += 2;
                    while index + 1 < at && nesting > 0 {
                        match (bytes[index], bytes[index + 1]) {
                            (b'/', b'*') => {
                                nesting += 1;
                                index += 2;
                            }
                            (b'*', b'/') => {
                                nesting -= 1;
                                index += 2;
                            }
                            _ => index += 1,
                        }
                    }
                }
                (b'"', _) => {
                    index += 1;
                    while index < at && bytes[index] != b'"' {
                        index += if bytes[index] == b'\\' { 2 } else { 1 };
                    }
                    index += 1;
                }
                (b'{', _) => {
                    depth = depth.saturating_add(1);
                    index += 1;
                }
                (b'}', _) => {
                    depth = depth.saturating_sub(1);
                    index += 1;
                }
                _ => index += 1,
            }
        }
        depth
    }

    #[test]
    fn the_empty_arm_branches_on_the_shared_predicate_in_release() {
        // Codex round 4, finding 2. Every other test here holds the predicate
        // and the resulting behaviour, and neither notices if the production
        // branch stops asking:
        //
        //     if size == 0 {
        //         debug_assert!(!rewitness_owed(size as usize));
        //
        // passes all of them while the shipped build decides emptiness with an
        // inline test and the shared guard compiled out. Then there are two
        // definitions of "nothing to witness" and only one is under test -
        // which is the round-1 defect, at the site round 1 did not reach.
        const PREDICATE: &str = "    if !rewitness_owed(size as usize) {";
        let source = include_str!("country_variables_extractor.rs").replace("\r\n", "\n");
        // Lane SE3 moved this body into `read_variables_container_inner`, the
        // shared tail of the plain and vtable-pinned entry points, so the
        // predicate is scoped there. Following it is the point: if the arm ever
        // moves again this `.expect` fails loudly, rather than the test quietly
        // scanning a two-line delegation and finding nothing to object to.
        let start = source
            .find("fn read_variables_container_inner<R: MemoryRead>(")
            .expect("the container reader");
        let body = &source[start..];
        let body = without_comments(&body[..body.find("\n}\n").expect("it closes at column zero")]);

        assert_eq!(
            body.matches(PREDICATE).count(),
            1,
            "the empty arm must branch on the predicate itself, in release"
        );
        // ...and nothing may enclose that branch. `if mask <= 3 {` wrapped
        // around the arm would leave the pinned line untouched while sending a
        // valid emptied container with a larger bucket mask down the measured
        // path instead (Codex round 5, finding 2).
        assert_eq!(
            statement_depth(&body, PREDICATE),
            1,
            "the empty arm must stand at the function's own level, with nothing \
             around it"
        );
        assert_eq!(
            body.matches("debug_assert!(!rewitness_owed").count(),
            0,
            "a debug assertion is not a guard; it disappears from the shipped \
             build. The POSITIVE `debug_assert!(rewitness_owed(..))` calls on \
             the walked paths are a different thing and are fine: those paths \
             re-witness unconditionally, and the assertion only records why."
        );
        assert_eq!(
            body.matches("size == 0").count(),
            0,
            "and the inline emptiness test it replaced must not come back beside it"
        );
    }

    #[test]
    fn a_header_that_moves_after_the_bucket_walk_is_still_refused() {
        // THE KILL. This re-witness DOES have an intervening read - two
        // snapshots of the bucket span - so it is owed and must still fire.
        // Deleting it, or widening the removal from "no walk happened" to
        // "this site", makes this container measure clean with rows that no
        // longer belong to the header that described them.
        let (_module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        let node_base = mem.read_u64(container + CV_BASE).unwrap();
        let settled = mem.read_u64(container + CV_SIZE_MASK).unwrap();
        assert_ne!(
            settled & 0xFFFF_FFFF,
            0,
            "the fixture must have rows to walk"
        );

        // Two head reads for the initial `stable_header`, then the span, then
        // the re-witness - which sees the moved word.
        let moved = container_head_moves_after(
            container,
            vtable,
            node_base,
            2 | (7u64 << 32),
            2,
            mem.clone(),
        );
        let read = read_variables_container(&moved, country, container);
        assert_eq!(read.status, VariableReadStatus::Unmeasured);
        assert_eq!(
            read.failure_reason,
            Some("country_variables_header_changed_after_snapshot")
        );

        // Positive control: the same budget with a header that did NOT move
        // measures clean, so the refusal above is the movement and not the
        // fixture running out of reads.
        let still = container_head_moves_after(container, vtable, node_base, settled, 2, mem);
        assert_eq!(
            read_variables_container(&still, country, container).status,
            VariableReadStatus::Measured
        );
    }

    #[test]
    fn an_empty_container_no_longer_re_reads_a_header_it_already_double_sampled() {
        // The removal itself, counted. An empty container's head is sampled
        // by `stable_header` and by nothing else; the second `stable_header`
        // that used to follow it is gone. kills: putting it back (the count
        // goes to four) or removing the FIRST pair too (it goes to zero, and
        // the double sample that is the tear check goes with it).
        let (_module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        let node_base = mem.read_u64(container + CV_BASE).unwrap();

        let mut empty = FixtureMemory::default();
        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, 0, vtable);
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            0 | (3u64 << 32),
        );
        empty.add(container, container_bytes);

        let recording = ReadCountingFixture::new(empty);
        let read = read_variables_container(&recording, country, container);
        assert_eq!(read.status, VariableReadStatus::MeasuredEmpty);
        assert_eq!(read.declared_size, 0);
        // Counted at `container + CV_BASE`, which is where the head window is
        // actually read: these fixtures serve no object window, so
        // `stable_header_pair` falls through to its `read_into` arm and takes
        // ONE 16-byte read covering the base pointer and the packed size/mask
        // word. Counting at `container` counted zero of them (landing gate,
        // first execution) and would have gone on reading zero however many
        // times the head was sampled.
        assert_eq!(
            recording.reads_at(container.saturating_add(CV_BASE)),
            2,
            "an empty container's head is sampled twice - the tear check - and \
             not four times: {:?}",
            recording.log.borrow()
        );
        assert_eq!(
            recording.log.borrow().len(),
            2,
            "and it reads nothing else at all: {:?}",
            recording.log.borrow()
        );
    }

    #[test]
    fn an_empty_container_whose_head_moves_after_its_double_sample_now_measures_empty() {
        // The behaviour the ruling deliberately changes, recorded so it is a
        // decision and not a surprise: the head pair agreed, so the container
        // is empty; what the bytes did microseconds later is not evidence
        // about a read that never happened.
        let (_module_base, country, mem) = complete_country_variable_fixture();
        let container = mem.read_u64(country + OBJ_VARIABLES).unwrap();
        let vtable = mem.read_u64(container).unwrap();
        let node_base = mem.read_u64(container + CV_BASE).unwrap();

        let mut empty = FixtureMemory::default();
        let mut container_bytes = vec![0u8; 0x40];
        put_u64(&mut container_bytes, 0, vtable);
        put_u64(&mut container_bytes, CV_BASE as usize, node_base);
        put_u64(
            &mut container_bytes,
            CV_SIZE_MASK as usize,
            0 | (3u64 << 32),
        );
        empty.add(container, container_bytes);

        let moved =
            container_head_moves_after(container, vtable, node_base, 5 | (7u64 << 32), 2, empty);
        assert_eq!(
            read_variables_container(&moved, country, container).status,
            VariableReadStatus::MeasuredEmpty
        );
    }

    /// Counts reads per address so a test can say how many times an object
    /// head was sampled.
    struct ReadCountingFixture {
        inner: FixtureMemory,
        log: std::cell::RefCell<Vec<(u64, usize)>>,
    }

    impl ReadCountingFixture {
        fn new(inner: FixtureMemory) -> Self {
            Self {
                inner,
                log: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn reads_at(&self, addr: u64) -> usize {
            self.log.borrow().iter().filter(|(a, _)| *a == addr).count()
        }
    }

    impl MemoryRead for ReadCountingFixture {
        fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            self.log.borrow_mut().push((addr, len));
            self.inner.read_exact(addr, len)
        }
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
