//! Borrowed original province-domain areas and contacts require quiescent
//! acquisition. The secondary domain remains opaque; pointers never enter
//! owned output. Nonzero deferred mode, marked nodes and targets outside
//! authenticated roots refuse, without certifying lifetime or orphan closure.
//!
//! Controller changes split and join the primary domain, and both domains
//! can rebuild contact lists. World clear nulls backlinks before freeing
//! contacts and areas. Contact removal can free payload while its node is
//! only marked rather than unlinked.
//!
//! Acquisition finishes before the next hourly tick, but this does not exclude
//! cleanup or indirect writers. Mode, marks, vtables and the construction
//! counter provide neither a lock nor graph generation. Acceptance requires
//! writer/free exclusion throughout the live-read interval, including writers
//! already in flight at the first entry after mid-game attach.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AreaInput {
    pub province: u32,
    pub controller: u32,
    pub owner: Option<u32>,
    pub node: Option<AreaNodeInput>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AreaNodeInput {
    pub first_land: Option<u32>,
    pub nonsea_index: Option<i32>,
    pub nonsea_count: Option<u32>,
    pub land_count: Option<i32>,
    pub neighbours: Option<(usize, usize)>,
    pub failure: Option<&'static str>,
}
#[derive(Clone, Copy, Debug)]
pub struct AreaCapture {
    pub len: Result<usize, &'static str>,
    pub edges_len: usize,
    pub required_edges: usize,
}
fn value<T: crate::memory::Raw>(memory: &impl Memory, object: usize, offset: usize) -> Option<T> {
    read(memory, object.checked_add(offset)?)
}
fn label(domain: &[ControllerAreaProvince], pointer: usize) -> Result<u32, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    let first = domain.partition_point(|row| row.area < pointer);
    domain
        .get(first)
        .filter(|row| row.area == pointer)
        .map(|row| row.province)
        .ok_or("selected area orphan outside authenticated current D0 roots")
}

impl<M: Memory> Reader<'_, M> {
    /// Off-hour arena hint from the authenticated original domain and its ordered
    /// contact-list signed count. It follows no contact pointer or payload and
    /// never supplies hourly data.
    pub fn selected_area_edge_demand(
        &self,
        domain: &[ControllerAreaProvince],
    ) -> Result<usize, &'static str> {
        if domain
            .windows(2)
            .any(|rows| (rows[0].area, rows[0].province) >= (rows[1].area, rows[1].province))
        {
            return Err("selected area sizing domain order invalid");
        }
        let expected = self
            .base
            .checked_add(crate::ccontroller_area::VTABLE_RVA)
            .ok_or("selected area sizing module address overflow")? as u64;
        let mut previous = 0;
        let mut required = 0_usize;
        for root in domain {
            if root.area == 0 || root.area == previous {
                continue;
            }
            previous = root.area;
            if value::<u64>(self.memory, root.area, 0) != Some(expected) {
                return Err("selected area sizing identity unreadable or unsupported");
            }
            let count = value::<i32>(self.memory, root.area, 0xa0)
                .filter(|count| *count >= 0)
                .ok_or("selected area sizing contact count unreadable or negative")?;
            required = required
                .checked_add(count as usize)
                .ok_or("selected area sizing contact count overflow")?;
        }
        Ok(required)
    }

    /// Borrow exact complete domain and original outcome before its holder
    /// clears controls. Capacity failure cannot erase successful theatre data.
    pub fn capture_selected_areas(
        &self,
        state: usize,
        original: &DomainCapture,
        domain: &[ControllerAreaProvince],
        scratch: &mut [ControllerAreaProvince],
        output: &mut [AreaInput],
        edges: &mut [u32],
    ) -> AreaCapture {
        self.capture_selected_areas_buffer(
            state,
            original,
            domain,
            crate::walk::Buffer::Fixed(scratch),
            crate::walk::Buffer::Fixed(output),
            crate::walk::Buffer::Fixed(edges),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_selected_areas_growing(
        &self,
        state: usize,
        original: &DomainCapture,
        domain: &[ControllerAreaProvince],
        scratch: &mut Vec<ControllerAreaProvince>,
        output: &mut Vec<AreaInput>,
        edges: &mut Vec<u32>,
    ) -> AreaCapture {
        self.capture_selected_areas_buffer(
            state,
            original,
            domain,
            crate::walk::Buffer::Growing(scratch),
            crate::walk::Buffer::Growing(output),
            crate::walk::Buffer::Growing(edges),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_selected_areas_buffer(
        &self,
        state: usize,
        original: &DomainCapture,
        domain: &[ControllerAreaProvince],
        mut scratch: crate::walk::Buffer<'_, ControllerAreaProvince>,
        mut output: crate::walk::Buffer<'_, AreaInput>,
        mut edges: crate::walk::Buffer<'_, u32>,
    ) -> AreaCapture {
        output.fill(AreaInput::default());
        scratch.fill(ControllerAreaProvince::default());
        edges.fill(0);
        let mut used = 0;
        let mut required = 0;
        let result = (|| {
            let len = original.len?;
            if len != domain.len() {
                return Err("selected area original domain length differs");
            }
            output.ensure(len)?;
            scratch.ensure(len)?;
            if output.len() < len || scratch.len() < len {
                return Err("selected area projection arena exhausted");
            }
            if domain
                .windows(2)
                .any(|rows| (rows[0].area, rows[0].province) >= (rows[1].area, rows[1].province))
            {
                return Err("selected area original domain order invalid");
            }
            if len == 0 {
                return Ok(0);
            }
            scratch[..len].copy_from_slice(domain);
            scratch[..len].sort_unstable_by_key(|row| (row.owner, row.province));
            let mut previous = None;
            let mut owner_label = 0;
            for (index, row) in scratch[..len].iter().enumerate() {
                if row.owner != previous {
                    owner_label = if row.owner == Some(0) {
                        0
                    } else {
                        row.province
                    };
                    previous = row.owner;
                }
                output[index] = AreaInput {
                    province: row.province,
                    controller: label(domain, row.area)?,
                    owner: row.owner.map(|_| owner_label),
                    node: None,
                };
            }
            output[..len].sort_unstable_by_key(|row| row.province);
            if output[..len]
                .windows(2)
                .any(|r| r[0].province >= r[1].province)
            {
                return Err("selected area province domain duplicated");
            }
            // Original selecting pointers, for first-land authentication. No
            // second province registry walk or representative reconstruction.
            scratch[..len].sort_unstable_by_key(|row| row.pointer);
            let nonsea_count = value::<u32>(self.memory, state, 0x6b8);
            let indices = self
                .base
                .checked_add(0x0332_0db8)
                .and_then(|at| read::<u64>(self.memory, at))
                .filter(|p| *p != 0)
                .and_then(|map| value::<u64>(self.memory, map as usize, 0x238))
                .filter(|p| *p != 0);
            let mut previous = 0;
            for root in domain {
                if root.area == 0 || root.area == previous {
                    continue;
                }
                previous = root.area;
                let at = output[..len]
                    .binary_search_by_key(&root.province, |r| r.province)
                    .unwrap();
                let mut node = AreaNodeInput {
                    nonsea_count,
                    ..Default::default()
                };
                let expected = self.base.checked_add(crate::ccontroller_area::VTABLE_RVA);
                if expected.is_none()
                    || value::<u64>(self.memory, root.area, 0) != expected.map(|p| p as u64)
                {
                    node.failure = Some("selected controller-area type unreadable or unsupported");
                    output[at].node = Some(node);
                    continue;
                }
                node.land_count = value(self.memory, root.area, 0x3c);
                if let Some(first) = value::<u64>(self.memory, root.area, 0x28)
                    && let Ok(index) =
                        scratch[..len].binary_search_by_key(&(first as usize), |r| r.pointer)
                {
                    let province = scratch[index].province;
                    node.first_land = Some(province);
                    node.nonsea_index = indices
                        .and_then(|indices| {
                            (indices as usize).checked_add((province as usize).checked_mul(4)?)
                        })
                        .and_then(|at| read(self.memory, at));
                }
                let start = used;
                let neighbours =
                    self.area_neighbours(root.area, domain, edges.suffix(used), &mut required);
                match neighbours {
                    Ok(count) => {
                        node.neighbours = Some((start, count));
                        used += count;
                    }
                    Err(reason) => {
                        node.failure = Some(reason);
                        edges[start..].fill(0);
                    }
                }
                output[at].node = Some(node);
            }
            Ok(len)
        })();
        scratch.fill(ControllerAreaProvince::default());
        if result.is_err() {
            output.fill(AreaInput::default());
            edges.fill(0);
            used = 0;
        }
        AreaCapture {
            len: result,
            edges_len: used,
            required_edges: required,
        }
    }

    /// Contact traversal follows the node payload. Removal can free payload
    /// before a marked node is unlinked: mode and mark must precede payload reads.
    fn area_neighbours(
        &self,
        area: usize,
        domain: &[ControllerAreaProvince],
        mut output: crate::walk::Buffer<'_, u32>,
        required: &mut usize,
    ) -> Result<usize, &'static str> {
        // Deferred mode is one byte; the following three bytes are padding,
        // separate from required controls.
        let header = read::<[u8; 21]>(self.memory, at(area, 0x90)?)
            .ok_or("selected area adjacency controls unreadable")?;
        let first = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
        let last = u64::from_le_bytes(header[8..16].try_into().unwrap()) as usize;
        let count = i32::from_le_bytes(header[16..20].try_into().unwrap());
        let mode = header[20];
        if count < 0 {
            return Err("selected area adjacency signed count invalid");
        }
        *required = required
            .checked_add(count as usize)
            .ok_or("selected area adjacency demand overflow")?;
        if mode != 0 {
            return Err("selected area deferred adjacency mode unsupported");
        }
        // Contacts follow the original signed list count.
        output.ensure(count as usize)?;
        if output.len() < count as usize {
            return Err("selected area adjacency arena exhausted");
        }
        let mut cursor = first;
        let mut previous = 0;
        for output in output.iter_mut().take(count as usize) {
            if cursor == 0 {
                return Err("selected area adjacency ends before native count");
            }
            let node = read::<[u8; 25]>(self.memory, cursor)
                .ok_or("selected area adjacency node unreadable")?;
            let payload = u64::from_le_bytes(node[..8].try_into().unwrap()) as usize;
            let back = u64::from_le_bytes(node[8..16].try_into().unwrap()) as usize;
            let next = u64::from_le_bytes(node[16..24].try_into().unwrap()) as usize;
            if node[24] != 0 {
                return Err("selected area marked adjacency payload lifetime unsupported");
            }
            if back != previous || payload == 0 {
                return Err("selected area adjacency links invalid");
            }
            let target = read::<u64>(self.memory, payload)
                .filter(|p| *p != 0)
                .ok_or("selected area adjacency payload unreadable or null")?
                as usize;
            *output = label(domain, target)?;
            previous = cursor;
            cursor = next;
        }
        if cursor != 0 || previous != last || (count == 0 && first != 0) {
            return Err("selected area adjacency terminal links differ from native count");
        }
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};

    const STATE: usize = 0x1000;
    const MAP: usize = 0x2000;
    const INDICES: usize = 0x3000;
    const AREA_A: usize = 0x8000;
    const AREA_B: usize = 0x9000;
    const NODES_A: usize = 0xa000;
    const NODES_B: usize = 0xb000;
    const PAYLOADS_A: usize = 0xc000;
    const PAYLOADS_B: usize = 0xd000;

    #[derive(Default)]
    struct TestMemory {
        bytes: BTreeMap<usize, u8>,
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }

    impl Memory for TestMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fault.is_some_and(|fault| {
                address <= fault && fault < address.saturating_add(destination.len())
            }) {
                destination.fill(0xff);
                return false;
            }
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(source) = address
                    .checked_add(index)
                    .and_then(|at| self.bytes.get(&at))
                else {
                    return false;
                };
                *byte = *source;
            }
            true
        }
    }

    impl TestMemory {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            for (index, &byte) in bytes.iter().enumerate() {
                self.bytes.insert(address + index, byte);
            }
        }

        fn u64(&mut self, address: usize, value: usize) {
            self.put(address, &(value as u64).to_le_bytes());
        }

        fn i32(&mut self, address: usize, value: i32) {
            self.put(address, &value.to_le_bytes());
        }

        fn touched(&self, field: usize) -> bool {
            self.calls
                .borrow()
                .iter()
                .any(|&(at, len)| at <= field && field < at.saturating_add(len))
        }

        fn list(&mut self, area: usize, nodes: usize, payloads: usize, targets: &[usize]) {
            let mut header = [0; 24];
            if !targets.is_empty() {
                header[..8].copy_from_slice(&(nodes as u64).to_le_bytes());
                header[8..16]
                    .copy_from_slice(&((nodes + (targets.len() - 1) * 0x40) as u64).to_le_bytes());
            }
            header[16..20].copy_from_slice(&(targets.len() as i32).to_le_bytes());
            self.put(area + 0x90, &header);
            for (index, &target) in targets.iter().enumerate() {
                let node = nodes + index * 0x40;
                let payload = payloads + index * 0x20;
                let mut bytes = [0; 25];
                bytes[..8].copy_from_slice(&(payload as u64).to_le_bytes());
                if index > 0 {
                    bytes[8..16].copy_from_slice(&((node - 0x40) as u64).to_le_bytes());
                }
                if index + 1 < targets.len() {
                    bytes[16..24].copy_from_slice(&((node + 0x40) as u64).to_le_bytes());
                }
                self.put(node, &bytes);
                self.u64(payload, target);
            }
        }
    }

    fn province_pointer(province: u32) -> usize {
        0x4000 + province as usize * 0x200
    }

    // These are the original authenticated observations, not a second map
    // census. IDs intentionally disagree with pointer and owner ordering.
    fn world() -> (TestMemory, Vec<ControllerAreaProvince>) {
        let domain = [
            (1, 0, Some(0xf000)),
            (6, 0, Some(0)),
            (2, AREA_A, Some(0xf000)),
            (3, AREA_A, Some(0xf100)),
            (4, AREA_B, Some(0)),
            (5, AREA_B, None),
        ]
        .map(|(province, area, owner)| ControllerAreaProvince {
            province,
            area,
            owner,
            pointer: province_pointer(province),
        })
        .to_vec();
        let mut memory = TestMemory::default();
        memory.i32(STATE + 0x6b8, 12);
        memory.u64(0x0332_0db8, MAP);
        memory.u64(MAP + 0x238, INDICES);
        for (area, first, count, index) in [(AREA_A, 3, -2, -7), (AREA_B, 5, 4, 9)] {
            memory.u64(area, crate::ccontroller_area::VTABLE_RVA);
            memory.u64(area + 0x28, province_pointer(first));
            memory.i32(area + 0x3c, count);
            memory.i32(INDICES + first as usize * 4, index);
        }
        memory.list(AREA_A, NODES_A, PAYLOADS_A, &[AREA_B, AREA_B, AREA_A]);
        memory.list(AREA_B, NODES_B, PAYLOADS_B, &[]);
        (memory, domain)
    }

    fn reader(memory: &TestMemory, base: usize) -> Reader<'_, TestMemory> {
        Reader {
            memory,
            base,
            manager: Ok(None),
            provinces: Err("original province controls must not be reread"),
            reserved: Err("unrelated reserved country unavailable"),
        }
    }

    fn original(len: usize) -> DomainCapture {
        DomainCapture {
            len: Ok(len),
            required: len,
            first_failure: None,
        }
    }

    fn run(
        memory: &TestMemory,
        domain: &[ControllerAreaProvince],
        edge_capacity: usize,
    ) -> (AreaCapture, Vec<AreaInput>, Vec<u32>) {
        let mut scratch = vec![
            ControllerAreaProvince {
                province: 99,
                pointer: 99,
                area: 99,
                owner: Some(99),
            };
            domain.len() + 1
        ];
        let mut output = vec![
            AreaInput {
                province: 99,
                ..Default::default()
            };
            domain.len() + 1
        ];
        let mut edges = vec![99; edge_capacity];
        let result = reader(memory, 0).capture_selected_areas(
            STATE,
            &original(domain.len()),
            domain,
            &mut scratch,
            &mut output,
            &mut edges,
        );
        assert!(
            scratch
                .iter()
                .all(|row| *row == ControllerAreaProvince::default())
        );
        assert_eq!(output[domain.len()], AreaInput::default());
        (result, output, edges)
    }

    #[test]
    fn d0_d8_null_and_unavailable_are_independent_and_contacts_keep_occurrences() {
        let (memory, domain) = world();
        let saved = domain.clone();
        let (result, output, edges) = run(&memory, &domain, 8);
        assert_eq!(result.len, Ok(6));
        assert_eq!((result.edges_len, result.required_edges), (3, 3));
        assert_eq!(
            output[..6]
                .iter()
                .map(|row| row.controller)
                .collect::<Vec<_>>(),
            [0, 2, 2, 4, 4, 0]
        );
        assert_eq!(
            output[..6].iter().map(|row| row.owner).collect::<Vec<_>>(),
            [Some(1), Some(1), Some(3), Some(0), None, Some(0)]
        );
        assert_eq!(edges, [4, 4, 2, 0, 0, 0, 0, 0]);
        assert_eq!(
            output[1].node,
            Some(AreaNodeInput {
                first_land: Some(3),
                nonsea_index: Some(-7),
                nonsea_count: Some(12),
                land_count: Some(-2),
                neighbours: Some((0, 3)),
                failure: None,
            })
        );
        assert_eq!(output[3].node.unwrap().neighbours, Some((3, 0)));
        assert!(
            [0, 2, 4, 5]
                .into_iter()
                .all(|index| output[index].node.is_none())
        );
        assert_eq!(domain, saved);
        assert!(domain.iter().all(|row| !memory.touched(row.pointer)));
        assert!(!memory.touched(STATE + cgame_state::PROVINCES));
    }

    #[test]
    fn first_land_requires_the_original_selecting_pointer_without_an_id_lookup() {
        let (mut memory, domain) = world();
        let replacement = 0xe000;
        memory.u64(AREA_A + 0x28, replacement);
        memory.u64(replacement, cprovince::VTABLE_RVA);
        memory.i32(replacement + cprovince::ID, 3);
        let (result, output, _) = run(&memory, &domain, 3);
        assert_eq!(result.len, Ok(6));
        let node = output[1].node.unwrap();
        assert_eq!((node.first_land, node.nonsea_index), (None, None));
        assert_eq!(node.neighbours, Some((0, 3)));
        assert!(!memory.touched(replacement));
        assert!(!memory.touched(INDICES + 3 * 4));
    }

    #[test]
    fn borrower_consumes_the_original_authenticated_census_without_reacquiring_controls() {
        let (mut memory, expected) = world();
        let province_slots = 0x6000;
        memory.u64(province_slots, 0);
        memory.u64(province_slots + 7 * 8, 0);
        for row in &expected {
            memory.u64(province_slots + row.province as usize * 8, row.pointer);
            memory.u64(row.pointer, cprovince::VTABLE_RVA);
            memory.i32(row.pointer + cprovince::ID, row.province as i32);
            memory.u64(row.pointer + cprovince::CONTROLLER_AREA, row.area);
            if let Some(owner) = row.owner {
                memory.u64(row.pointer + cprovince::OWNER_AREA, owner);
            }
        }
        let mut reader = reader(&memory, 0);
        reader.provinces = Ok(Array {
            data: province_slots,
            count: 8,
        });
        let mut domain = [ControllerAreaProvince::default(); 7];
        let observed = reader.capture_controller_area_domain(&mut domain);
        assert_eq!((observed.len, observed.required), (Ok(6), 7));
        assert_eq!(&domain[..6], expected.as_slice());
        let before = memory.calls.borrow().len();
        let mut scratch = [ControllerAreaProvince::default(); 7];
        let mut output = [AreaInput::default(); 7];
        let mut edges = [99; 3];
        let result = reader.capture_selected_areas(
            STATE,
            &observed,
            &domain[..6],
            &mut scratch,
            &mut output,
            &mut edges,
        );
        assert_eq!(result.len, Ok(6));
        assert_eq!(edges, [4, 4, 2]);
        assert_eq!(&domain[..6], expected.as_slice());
        assert!(
            scratch
                .iter()
                .all(|row| *row == ControllerAreaProvince::default())
        );
        let calls = memory.calls.borrow();
        assert!(calls[before..].iter().all(|&(at, len)| {
            !(at < province_slots + 8 * 8 && province_slots < at.saturating_add(len))
                && expected.iter().all(|row| {
                    ![
                        row.pointer,
                        row.pointer + cprovince::ID,
                        row.pointer + cprovince::CONTROLLER_AREA,
                        row.pointer + cprovince::OWNER_AREA,
                    ]
                    .iter()
                    .any(|&field| at <= field && field < at.saturating_add(len))
                })
        }));
    }

    #[test]
    fn province_batch_clears_selecting_controls_on_success_and_each_failed_copy() {
        for case in 0..3 {
            let (mut memory, _) = world();
            let pointer = province_pointer(1);
            memory.u64(pointer, cprovince::VTABLE_RVA);
            memory.i32(pointer + cprovince::ID, if case == 1 { 2 } else { 1 });
            memory.u64(pointer + cprovince::CONTROLLER_AREA, AREA_A);
            memory.u64(pointer + cprovince::OWNER_AREA, 0);
            if case == 2 {
                memory.fault = Some(pointer + cprovince::ID);
            }
            let mut batch = ProvinceBatch::new();
            batch.pointers[0] = pointer;
            batch.slots[0] = 1;
            batch.len = 1;
            let mut output = [ControllerAreaProvince::default(); 1];
            let mut written = 0;
            let result = batch.flush(
                &memory,
                cprovince::VTABLE_RVA as u64,
                &mut output,
                &mut written,
            );
            match case {
                0 => {
                    assert_eq!(result, Ok(()));
                    assert_eq!(written, 1);
                    assert_eq!(output[0].pointer, pointer);
                    assert_eq!(output[0].owner, Some(0));
                }
                1 => assert_eq!(result, Err("naval movement province identity mismatch")),
                _ => assert_eq!(
                    result,
                    Err("naval movement province-area record unreadable")
                ),
            }
            assert_eq!(batch.len, 0);
            assert!(batch.pointers.iter().all(|&pointer| pointer == 0));
            assert!(batch.slots.iter().all(|&slot| slot == 0));
            if case != 0 {
                assert_eq!(written, 0);
                assert!(!memory.touched(pointer + cprovince::OWNER_AREA));
            }
        }
    }

    #[test]
    fn original_failure_short_projection_and_malformed_domains_scrub_without_native_reads() {
        let (memory, domain) = world();
        let mut unordered = domain.clone();
        unordered.swap(0, 1);
        let mut duplicate = domain.clone();
        duplicate[4].province = 2;
        for (source, observed, capacity, reason) in [
            (
                DomainCapture {
                    len: Err("original failure"),
                    ..original(6)
                },
                &domain,
                0,
                "original failure",
            ),
            (
                original(6),
                &domain,
                5,
                "selected area projection arena exhausted",
            ),
            (
                original(5),
                &domain,
                6,
                "selected area original domain length differs",
            ),
            (
                original(6),
                &unordered,
                6,
                "selected area original domain order invalid",
            ),
            (
                original(6),
                &duplicate,
                6,
                "selected area province domain duplicated",
            ),
        ] {
            let mut scratch = domain.clone();
            let mut output = vec![
                AreaInput {
                    province: 99,
                    ..Default::default()
                };
                capacity
            ];
            let mut edges = [99; 3];
            let result = reader(&memory, 0).capture_selected_areas(
                STATE,
                &source,
                observed,
                &mut scratch,
                &mut output,
                &mut edges,
            );
            assert_eq!(result.len, Err(reason));
            assert_eq!((result.edges_len, result.required_edges), (0, 0));
            assert!(
                scratch
                    .iter()
                    .all(|row| *row == ControllerAreaProvince::default())
            );
            assert!(output.iter().all(|row| *row == AreaInput::default()));
            assert_eq!(edges, [0; 3]);
            assert!(memory.calls.borrow().is_empty());
        }
        let mut short_scratch = [ControllerAreaProvince::default(); 5];
        let mut output = [AreaInput::default(); 6];
        assert_eq!(
            reader(&memory, 0)
                .capture_selected_areas(
                    STATE,
                    &original(6),
                    &domain,
                    &mut short_scratch,
                    &mut output,
                    &mut [],
                )
                .len,
            Err("selected area projection arena exhausted")
        );
        assert!(memory.calls.borrow().is_empty());
    }

    #[test]
    fn deferred_mode_reads_only_the_native_byte_without_demanding_padding() {
        for unreadable_padding in [false, true] {
            let (mut memory, domain) = world();
            memory.put(AREA_A + 0xa5, &[0xff, 0x80, 0x7f]);
            if unreadable_padding {
                memory.fault = Some(AREA_A + 0xa5);
            }
            let (result, output, edges) = run(&memory, &domain, 3);
            assert_eq!(result.len, Ok(6));
            assert_eq!((result.edges_len, result.required_edges), (3, 3));
            assert_eq!(output[1].node.unwrap().neighbours, Some((0, 3)));
            assert_eq!(output[1].node.unwrap().failure, None);
            assert_eq!(edges, [4, 4, 2]);
            assert!(!memory.touched(AREA_A + 0xa5));
        }
    }

    #[test]
    fn deferred_mode_and_mark_refuse_before_the_possibly_freed_payload() {
        for marked in [false, true] {
            let (mut memory, domain) = world();
            if marked {
                memory.put(NODES_A + 0x40 + 0x18, &[1]);
            } else {
                memory.put(AREA_A + 0xa4, &[1]);
            }
            memory.list(AREA_B, NODES_B, PAYLOADS_B, &[AREA_A]);
            let (result, output, edges) = run(&memory, &domain, 4);
            assert_eq!(result.len, Ok(6));
            assert_eq!((result.edges_len, result.required_edges), (1, 4));
            assert_eq!(output[1].node.unwrap().neighbours, None);
            assert_eq!(
                output[1].node.unwrap().failure,
                Some(if marked {
                    "selected area marked adjacency payload lifetime unsupported"
                } else {
                    "selected area deferred adjacency mode unsupported"
                })
            );
            assert_eq!(output[3].node.unwrap().neighbours, Some((0, 1)));
            assert_eq!(edges, [2, 0, 0, 0]);
            assert!(!memory.touched(PAYLOADS_A + 0x20));
            assert!(!memory.touched(NODES_A + 0x80));
            if !marked {
                assert!(!memory.touched(NODES_A));
                assert!(!memory.touched(PAYLOADS_A));
            }
        }
    }

    #[test]
    fn orphan_target_cannot_be_admitted_by_a_plausible_vtable_or_partial_prefix() {
        let (mut memory, domain) = world();
        let orphan = 0xe000;
        memory.list(AREA_A, NODES_A, PAYLOADS_A, &[AREA_B, orphan, AREA_B]);
        memory.u64(orphan, crate::ccontroller_area::VTABLE_RVA);
        let (result, output, edges) = run(&memory, &domain, 3);
        assert_eq!(result.len, Ok(6));
        assert_eq!((result.edges_len, result.required_edges), (0, 3));
        let node = output[1].node.unwrap();
        assert_eq!(node.neighbours, None);
        assert_eq!(
            node.failure,
            Some("selected area orphan outside authenticated current D0 roots")
        );
        assert_eq!(edges, [0; 3]);
        assert!(!memory.touched(orphan));
        assert!(!memory.touched(NODES_A + 0x80));
    }

    #[test]
    fn short_edge_arena_keeps_exact_demand_and_an_independent_list() {
        let (mut memory, domain) = world();
        memory.list(AREA_B, NODES_B, PAYLOADS_B, &[AREA_A, AREA_A]);
        let (result, output, edges) = run(&memory, &domain, 2);
        assert_eq!(result.len, Ok(6));
        assert_eq!((result.edges_len, result.required_edges), (2, 5));
        assert_eq!(
            output[1].node.unwrap().failure,
            Some("selected area adjacency arena exhausted")
        );
        assert_eq!(output[3].node.unwrap().neighbours, Some((0, 2)));
        assert_eq!(edges, [2, 2]);
        assert!(!memory.touched(NODES_A));
        assert!(!memory.touched(PAYLOADS_A));
    }

    #[test]
    fn native_list_controls_and_partial_reads_refuse_without_a_prefix_or_retry() {
        for (case, reason, required) in [
            (0, "selected area adjacency signed count invalid", 0),
            (1, "selected area adjacency ends before native count", 3),
            (2, "selected area adjacency links invalid", 3),
            (
                3,
                "selected area adjacency terminal links differ from native count",
                3,
            ),
            (4, "selected area adjacency payload unreadable or null", 3),
            (5, "selected area adjacency controls unreadable", 0),
            (6, "selected area adjacency node unreadable", 3),
            (
                7,
                "selected area adjacency terminal links differ from native count",
                0,
            ),
        ] {
            let (mut memory, domain) = world();
            match case {
                0 => memory.i32(AREA_A + 0xa0, -1),
                1 => memory.u64(NODES_A + 0x10, 0),
                2 => memory.u64(NODES_A + 0x40 + 8, 0),
                3 => memory.u64(AREA_A + 0x98, NODES_A),
                4 => memory.u64(PAYLOADS_A + 0x20, 0),
                5 => memory.fault = Some(AREA_A + 0x90),
                6 => memory.fault = Some(NODES_A + 0x40 + 0x18),
                7 => memory.i32(AREA_A + 0xa0, 0),
                _ => unreachable!(),
            }
            let (result, output, edges) = run(&memory, &domain, 3);
            assert_eq!(result.len, Ok(6));
            assert_eq!((result.edges_len, result.required_edges), (0, required));
            let node = output[1].node.unwrap();
            assert_eq!(node.neighbours, None);
            assert_eq!(node.failure, Some(reason));
            assert_eq!(edges, [0; 3]);
            if let Some(fault) = memory.fault {
                assert_eq!(
                    memory
                        .calls
                        .borrow()
                        .iter()
                        .filter(|&&(at, len)| { at <= fault && fault < at.saturating_add(len) })
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn null_index_pointer_and_missing_count_do_not_invent_zero_or_hide_other_inputs() {
        let (mut memory, domain) = world();
        memory.u64(MAP + 0x238, 0);
        memory.fault = Some(STATE + 0x6b8);
        // A null map pointer must not turn a readable low address into an index.
        memory.i32(3 * 4, 123);
        memory.i32(5 * 4, 456);
        let (result, output, edges) = run(&memory, &domain, 3);
        assert_eq!(result.len, Ok(6));
        let node = output[1].node.unwrap();
        assert_eq!((node.nonsea_index, node.nonsea_count), (None, None));
        assert_eq!((node.first_land, node.land_count), (Some(3), Some(-2)));
        assert_eq!(node.neighbours, Some((0, 3)));
        assert_eq!(edges, [4, 4, 2]);
        assert!(!memory.touched(3 * 4));
        assert!(!memory.touched(5 * 4));
    }

    #[test]
    fn invalid_area_type_and_overflowed_expected_address_never_read_the_graph() {
        let (mut memory, domain) = world();
        memory.u64(AREA_A, 0);
        let (result, output, edges) = run(&memory, &domain, 3);
        assert_eq!(result.len, Ok(6));
        assert_eq!(
            output[1].node.unwrap().failure,
            Some("selected controller-area type unreadable or unsupported")
        );
        assert_eq!(output[1].controller, 2);
        assert_eq!(edges, [0; 3]);
        assert!(!memory.touched(AREA_A + 0x28));
        assert!(!memory.touched(AREA_A + 0x90));

        let memory = TestMemory::default();
        let mut scratch = [ControllerAreaProvince::default(); 6];
        let mut output = [AreaInput::default(); 6];
        let mut edges = [99; 3];
        let base = usize::MAX - crate::ccontroller_area::VTABLE_RVA + 1;
        let result = reader(&memory, base).capture_selected_areas(
            STATE,
            &original(6),
            &domain,
            &mut scratch,
            &mut output,
            &mut edges,
        );
        assert_eq!(result.len, Ok(6));
        for index in [1, 3] {
            assert_eq!(
                output[index].node.unwrap().failure,
                Some("selected controller-area type unreadable or unsupported")
            );
        }
        assert!(!memory.touched(AREA_A + 0x28));
        assert!(!memory.touched(AREA_A + 0x90));
        assert!(!memory.touched(AREA_B + 0x90));
        assert!(
            scratch
                .iter()
                .all(|row| *row == ControllerAreaProvince::default())
        );
        assert_eq!(edges, [0; 3]);
    }

    #[test]
    fn successful_empty_domain_scrubs_and_never_follows_unrelated_roots() {
        let memory = TestMemory::default();
        let (result, output, edges) = run(&memory, &[], 2);
        assert_eq!(result.len, Ok(0));
        assert_eq!((result.edges_len, result.required_edges), (0, 0));
        assert_eq!(output, [AreaInput::default()]);
        assert_eq!(edges, [0; 2]);
        assert!(memory.calls.borrow().is_empty());
    }

    #[test]
    fn original_area_projection_and_contact_header_grow_mod_sized_arenas_without_retry() {
        let mut scratch = Vec::new();
        let mut output = Vec::new();
        let mut edges = Vec::new();
        for count in [5000_usize, 12000] {
            let (mut memory, domain) = world();
            let nodes = 0x20000;
            let payloads = nodes + count * 0x40;
            let targets = vec![AREA_B; count];
            // Existing helper builds the exact original adjacency list.
            memory.u64(AREA_A + 0x90, nodes);
            memory.u64(AREA_A + 0x98, nodes + (count - 1) * 0x40);
            memory.i32(AREA_A + 0xa0, count as i32);
            for (index, target) in targets.into_iter().enumerate() {
                let node = nodes + index * 0x40;
                let payload = payloads + index * 8;
                memory.u64(node, payload);
                memory.u64(node + 8, if index == 0 { 0 } else { node - 0x40 });
                memory.u64(node + 16, if index + 1 == count { 0 } else { node + 0x40 });
                memory.put(node + 24, &[0]);
                memory.u64(payload, target);
            }
            let captured = reader(&memory, 0).capture_selected_areas_growing(
                STATE,
                &original(domain.len()),
                &domain,
                &mut scratch,
                &mut output,
                &mut edges,
            );
            assert_eq!(captured.len, Ok(domain.len()));
            assert_eq!(
                (captured.edges_len, captured.required_edges),
                (count, count)
            );
            assert!(output[1].node.unwrap().failure.is_none());
            assert_eq!(output[1].node.unwrap().neighbours, Some((0, count)));
            assert_eq!(edges[..count], vec![4; count]);
            assert_eq!(
                memory
                    .calls
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| at == AREA_A + 0x90)
                    .count(),
                1
            );
        }
    }
}
