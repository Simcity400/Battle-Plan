use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    read,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native_parts::creference_registry;
use engine_sites::Rva;

pub const REGISTRY: usize = 0x6c0;
pub const NAMES_RVA: Rva = Rva(0x033e_b2b0);
pub const STRIDE: usize = 0x70;
pub const VTABLE_RVA: Rva = Rva(0x0279_f280);
pub const IN_GAME: usize = 0xa39;
pub const REFERENCES: [usize; 9] = [0x10, 0x18, 0x30, 0x38, 0x40, 0x48, 0x50, 0x58, 0x60];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub ordinal: u32,
    pub name_start: usize,
    pub name_len: usize,
    pub state: i32,
    pub country: i32,
    pub character: u32,
    pub live: u16,
    pub character_leader: Option<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub bytes: usize,
}
impl Demand {
    pub fn raise(&mut self, demand: Self) {
        *self = self.max(demand);
    }
    pub fn max(self, other: Self) -> Self {
        Self {
            rows: self.rows.max(other.rows),
            bytes: self.bytes.max(other.bytes),
        }
    }
}
pub struct Walk {
    pub outcome: Outcome,
    pub bytes: usize,
    pub required: Demand,
}

fn header(
    memory: &impl Memory,
    at: usize,
    stride: usize,
) -> Result<([u8; 16], Array), &'static str> {
    let bytes = read::<[u8; 16]>(memory, at).ok_or("saved event target header unreadable")?;
    let array = Array::from_header(bytes, stride).ok_or("saved event target header invalid")?;
    Ok((bytes, array))
}
fn name_header(memory: &impl Memory, names: Array, id: u16) -> Result<[u8; 32], &'static str> {
    let at = names
        .element(u32::from(id), 32)
        .ok_or("saved event target name index outside registry")?;
    let text = read::<[u8; 32]>(memory, at).ok_or("saved event target name header unreadable")?;
    let len = u64::from_le_bytes(text[16..24].try_into().unwrap());
    let cap = u64::from_le_bytes(text[24..32].try_into().unwrap());
    if cap < 15 || len > cap {
        return Err("saved event target name header invalid");
    }
    Ok(text)
}
pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Option<Demand> {
    let result = count(memory, base, state);
    (!matches!(result.outcome, Outcome::Unavailable { .. })).then_some(result.required)
}
pub fn count(memory: &impl Memory, base: usize, state: usize) -> Walk {
    walk_buffer(memory, base, state, Buffer::Count, Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<Row>,
    text: &mut Vec<u8>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        state,
        Buffer::Growing(output),
        Buffer::Growing(text),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: Buffer<'_, Row>,
    mut text: Buffer<'_, u8>,
) -> Walk {
    let mut len = 0usize;
    let mut used = 0usize;
    let mut required = Demand::default();
    let run = (|| -> Result<Outcome, &'static str> {
        let initialized = read::<u8>(
            memory,
            state
                .checked_add(IN_GAME)
                .ok_or("saved event target state overflow")?,
        )
        .ok_or("saved event target initialization flag unreadable")?;
        if initialized == 0 {
            return Err(
                "saved event targets outside initialized game: native THIS fallback not recorded",
            );
        }
        let registry_at = state
            .checked_add(REGISTRY)
            .ok_or("saved event target state overflow")?;
        let names_at =
            engine_sites::at(base, NAMES_RVA).ok_or("saved event target names overflow")?;
        let (rh, registry) = header(memory, registry_at, STRIDE)?;
        let (nh, names) = header(memory, names_at, 32)?;
        required.rows = registry.count as usize;
        if registry.count != 0 && names.count as usize > usize::from(u16::MAX) + 1 {
            return Err(
                "saved event target intern IDs have wrapped: name aliases are not recorded",
            );
        }
        output.ensure(required.rows)?;
        for i in 0..registry.count {
            let at = registry
                .element(i, STRIDE)
                .ok_or("saved event target record overflow")?;
            let raw =
                read::<[u8; 0x6a]>(memory, at).ok_or("saved event target record unreadable")?;
            let qword = |offset| u64::from_le_bytes(raw[offset..offset + 8].try_into().unwrap());
            if qword(0)
                != engine_sites::at(base, VTABLE_RVA).ok_or("saved event target vtable overflow")?
                    as u64
            {
                return Err("saved event target vtable mismatch");
            }
            let id = u16::from_le_bytes(raw[0x68..0x6a].try_into().unwrap());
            let name = name_header(memory, names, id)?;
            let name_len = usize::try_from(u64::from_le_bytes(name[16..24].try_into().unwrap()))
                .map_err(|_| "saved event target name length overflow")?;
            required.bytes = required
                .bytes
                .checked_add(name_len)
                .ok_or("saved event target name arena overflow")?;
            text.ensure(
                used.checked_add(name_len)
                    .ok_or("saved event target name arena overflow")?,
            )?;
            if len == output.len()
                || used
                    .checked_add(name_len)
                    .is_none_or(|end| end > text.len())
            {
                continue;
            }
            let cap = u64::from_le_bytes(name[24..32].try_into().unwrap());
            if cap == 15 {
                if name.get(name_len) != Some(&0) {
                    return Err("saved event target name terminator invalid");
                }
                text[used..used + name_len].copy_from_slice(&name[..name_len]);
            } else {
                let ptr = usize::try_from(u64::from_le_bytes(name[..8].try_into().unwrap()))
                    .ok()
                    .filter(|p| *p != 0)
                    .ok_or("saved event target name pointer invalid")?;
                if read::<u8>(
                    memory,
                    ptr.checked_add(name_len)
                        .ok_or("saved event target name overflow")?,
                ) != Some(0)
                {
                    return Err("saved event target name terminator unreadable or invalid");
                }
                if !read_slice(memory, ptr, &mut text[used..used + name_len]) {
                    return Err("saved event target name bytes unreadable");
                }
            }
            let target_state = i32::from_le_bytes(raw[8..12].try_into().unwrap());
            let country = i32::from_le_bytes(raw[12..16].try_into().unwrap());
            let mut live = u16::from(qword(0x20) != 0) << 9 | u16::from(qword(0x28) != 0) << 10;
            live |= u16::from(id == 0) << 11;
            let mut character_leader = None;
            if target_state == 0 && country <= 0 {
                for (bit, handle) in REFERENCES.map(qword).into_iter().enumerate() {
                    if let Some(object) = creference_registry::resolve(memory, base, handle)? {
                        if bit == 3 && object == 0x10 {
                            continue;
                        }
                        live |= 1 << bit;
                        if bit == 0 {
                            let direct = read::<u64>(
                                memory,
                                object.checked_add(0xa8).ok_or("saved character overflow")?,
                            )
                            .ok_or("saved character leader unreadable")?
                                as usize;
                            let leader = if direct != 0 {
                                Some(direct)
                            } else {
                                let operative = read::<u64>(
                                    memory,
                                    object.checked_add(0xc8).ok_or("saved character overflow")?,
                                )
                                .ok_or("saved character operative unreadable")?;
                                creference_registry::resolve(memory, base, operative)?
                            };
                            if let Some(leader) = leader {
                                character_leader = Some(
                                    read::<u64>(
                                        memory,
                                        leader.checked_add(8).ok_or("saved leader overflow")?,
                                    )
                                    .ok_or("saved leader reference unreadable")?,
                                );
                            }
                        }
                    }
                }
            }
            output[len] = Row {
                ordinal: i,
                name_start: used,
                name_len,
                state: target_state,
                country,
                character: (qword(0x10) >> 32) as u32,
                live,
                character_leader,
            };
            len += 1;
            used += name_len;
        }
        if read::<[u8; 16]>(memory, registry_at) != Some(rh)
            || read::<[u8; 16]>(memory, names_at) != Some(nh)
        {
            return Err("saved event target registry changed during read");
        }
        Ok(if len == required.rows {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: (required.rows - len) as u32,
            }
        })
    })();
    Walk {
        outcome: run.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        bytes: used,
        required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Fixture(BTreeMap<usize, Vec<u8>>);
    impl Fixture {
        fn put(&mut self, at: usize, b: &[u8]) {
            if let Some((&start, data)) = self.0.range_mut(..=at).next_back()
                && at - start + b.len() <= data.len()
            {
                data[at - start..at - start + b.len()].copy_from_slice(b);
                return;
            }
            self.0.insert(at, b.to_vec());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            let mut h = [0; 16];
            h[..8].copy_from_slice(&(data as u64).to_le_bytes());
            h[8..12].copy_from_slice(&count.to_le_bytes());
            h[12..].copy_from_slice(&count.to_le_bytes());
            self.put(at, &h);
        }
        fn new() -> Self {
            let mut f = Self::default();
            f.put(0x1000 + IN_GAME, &[2]);
            f.array(0x1000 + REGISTRY, 0xa000, 2);
            f.array(NAMES_RVA.0, 0x3000, 3);
            f.put(0xa000, &[0; STRIDE * 2]);
            f.put(0x3000, &[0; 32 * 3]);
            for (i, name) in [b"first".as_slice(), b"x", b"a_long_heap_target_name"]
                .into_iter()
                .enumerate()
            {
                let mut h = [0; 32];
                h[16..24].copy_from_slice(&(name.len() as u64).to_le_bytes());
                if name.len() <= 15 {
                    h[..name.len()].copy_from_slice(name);
                    h[24..].copy_from_slice(&15u64.to_le_bytes());
                } else {
                    h[..8].copy_from_slice(&0x4000u64.to_le_bytes());
                    h[24..].copy_from_slice(&(name.len() as u64).to_le_bytes());
                    let mut text = name.to_vec();
                    text.push(0);
                    f.put(0x4000, &text);
                }
                f.put(0x3000 + i * 32, &h);
            }
            for i in 0..2 {
                f.put(0xa000 + i * STRIDE, &(VTABLE_RVA.0 as u64).to_le_bytes());
                f.put(0xa000 + i * STRIDE + 0x68, &((i + 1) as u16).to_le_bytes());
            }
            f.put(0xa00c, &77i32.to_le_bytes());
            f.put(0xa000 + STRIDE + 8, &4i32.to_le_bytes());
            f
        }
    }
    impl Memory for Fixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let Some((&start, data)) = self.0.range(..=at).next_back() else {
                return false;
            };
            let Some(b) = data.get(at - start..at - start + out.len()) else {
                return false;
            };
            out.copy_from_slice(b);
            true
        }
    }
    #[test]
    fn reader_names_order_scalars_and_exact_capacity_demand() {
        let f = Fixture::new();
        let d = demand(&f, 0, 0x1000).unwrap();
        assert_eq!(d, Demand { rows: 2, bytes: 24 });
        let mut rows = vec![Row::default(); 2];
        let mut text = vec![0; 24];
        let w = walk(&f, 0, 0x1000, &mut rows, &mut text);
        assert_eq!(w.outcome, Outcome::Rows { len: 2 });
        assert_eq!(rows[0].country, 77);
        assert_eq!(rows[1].state, 4);
        assert_eq!(&text[..1], b"x");
        assert_eq!(&text[1..], b"a_long_heap_target_name");
        assert_eq!(rows[1].ordinal, 1);
        assert_eq!(w.required, d);
    }
    #[test]
    fn intern_id_zero_keeps_the_lookup_name_and_marks_save_omission() {
        let mut f = Fixture::new();
        f.put(0xa068, &0u16.to_le_bytes());
        let mut rows = vec![Row::default(); 2];
        let mut text = vec![0; 28];
        assert_eq!(
            walk(&f, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(&text[..rows[0].name_len], b"first");
        assert_eq!(rows[0].live, 1 << 11);
        assert_eq!(rows[0].country, 77);
        assert_eq!(rows[1].live, 0);
    }
    #[test]
    fn wrapped_name_ids_refuse_nonempty_registry_but_empty_remains_complete() {
        let mut f = Fixture::new();
        let mut rows = vec![Row::default(); 2];
        let mut text = vec![0; 24];
        f.array(NAMES_RVA.0, 0x3000, u32::from(u16::MAX) + 1);
        assert_eq!(
            walk(&f, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Rows { len: 2 }
        );
        f.array(NAMES_RVA.0, 0x3000, u32::from(u16::MAX) + 2);
        let wrapped = walk(&f, 0, 0x1000, &mut rows, &mut text);
        assert_eq!(
            wrapped.outcome,
            Outcome::Unavailable {
                reason: "saved event target intern IDs have wrapped: name aliases are not recorded"
            }
        );
        assert_eq!(wrapped.bytes, 0);
        assert_eq!(wrapped.required.rows, 2);
        f.array(0x1000 + REGISTRY, 0, 0);
        let empty = walk(&f, 0, 0x1000, &mut Vec::new(), &mut Vec::new());
        assert_eq!(empty.outcome, Outcome::Rows { len: 0 });
        assert_eq!(empty.required, Demand::default());
        assert_eq!(demand(&f, 0, 0x1000), Some(Demand::default()));
    }
    #[test]
    fn dead_references_keep_character_ids_but_lookup_absence_is_not_a_failure() {
        let mut f = Fixture::new();
        let handle = (3u64 << 32) | 54;
        f.put(0xa00c, &0i32.to_le_bytes());
        f.put(0xa010, &handle.to_le_bytes());
        f.put(
            creference_registry::LOW_ROOTS_RVA.0 + 54 * 8,
            &0u64.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 2];
        let mut text = vec![0; 24];
        assert_eq!(
            walk(&f, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].character, (handle >> 32) as u32);
        assert_eq!(rows[0].live, 0);
        f.put(0xa068, &1u16.to_le_bytes());
        f.put(0xa000 + STRIDE + 0x68, &1u16.to_le_bytes());
        assert_eq!(demand(&f, 0, 0x1000), Some(Demand { rows: 2, bytes: 2 }));
    }
    #[test]
    fn malformed_short_reads_names_and_uninitialized_state_are_unavailable() {
        for (at, b) in [
            (0xa068, 99u16.to_le_bytes().to_vec()),
            (0x3000 + 32 + 24, 14u64.to_le_bytes().to_vec()),
            (0xa000, 0u64.to_le_bytes().to_vec()),
            (0x1000 + IN_GAME, vec![0]),
            (0x3000 + 32 + 1, vec![1]),
        ] {
            let mut f = Fixture::new();
            f.put(at, &b);
            assert!(matches!(
                walk(&f, 0, 0x1000, &mut vec![Row::default(); 2], &mut vec![0; 24]).outcome,
                Outcome::Unavailable { .. }
            ));
        }
        let mut f = Fixture::new();
        f.0.remove(&0x4000);
        assert!(matches!(
            walk(&f, 0, 0x1000, &mut vec![Row::default(); 2], &mut vec![0; 24]).outcome,
            Outcome::Unavailable { .. }
        ));
    }
    #[test]
    fn live_character_leader_and_nonnull_zero_region_are_distinct_witnesses() {
        use engine_native_parts::creference_registry::{
            self as r,
            test_support::Fixture as Registry,
        };
        let mut f = Fixture::new();
        let handle = (3u64 << 32) | 54;
        f.put(0xa00c, &0i32.to_le_bytes());
        f.put(0xa010, &handle.to_le_bytes());
        let mut registry = Registry::new(54);
        registry.entry(Registry::index(handle), 1, handle, 0x5000);
        let fallback = (4u64..)
            .map(|id| (id << 32) | 54)
            .find(|h| Registry::index(*h) != Registry::index(handle))
            .unwrap();
        registry.entry(Registry::index(fallback), 1, fallback, 0x8100);
        let operation = (4u64 << 32) | 55;
        f.put(0xa018, &operation.to_le_bytes());
        f.put(r::LOW_ROOTS_RVA.0 + 55 * 8, &0u64.to_le_bytes());
        f.put(0x5000, &[0; 0xa00]);
        f.put(0x5000 + 0xa8, &0x8000u64.to_le_bytes());
        let leader = (9u64 << 32) | 1;
        f.put(0x8008, &leader.to_le_bytes());
        f.put(0xa000 + 0x20, &0x9000u64.to_le_bytes());
        struct Combined {
            saved: Fixture,
            registry: Registry,
        }
        impl Memory for Combined {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.registry.copy(at, out) || self.saved.copy(at, out)
            }
        }
        let mut memory = Combined { saved: f, registry };
        let mut rows = vec![Row::default(); 2];
        let mut text = vec![0; 24];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].live, 1 | (1 << 9));
        assert_eq!(rows[0].character, 3);
        assert_eq!(rows[0].character_leader, Some(leader));
        memory.saved.put(0x8008, &0u64.to_le_bytes());
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].character_leader, Some(0));
        memory.saved.put(0x5000 + 0xa8, &0u64.to_le_bytes());
        memory.saved.put(0x5000 + 0xc8, &fallback.to_le_bytes());
        memory.saved.put(0x8108, &leader.to_le_bytes());
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].character_leader, Some(leader));
        memory.saved.0.remove(&(r::LOW_ROOTS_RVA.0 + 55 * 8));
        assert!(matches!(
            walk(&memory, 0, 0x1000, &mut rows, &mut text).outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn a_changed_registry_header_withholds_the_walk() {
        use std::cell::Cell;
        struct Changing {
            fixture: Fixture,
            reads: Cell<u32>,
        }
        impl Memory for Changing {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                let ok = self.fixture.copy(at, out);
                if at == 0x1000 + REGISTRY && out.len() == 16 {
                    let n = self.reads.get();
                    self.reads.set(n + 1);
                    if n > 0 {
                        out[12] = 1;
                    }
                }
                ok
            }
        }
        let f = Changing {
            fixture: Fixture::new(),
            reads: Cell::new(0),
        };
        assert!(matches!(
            walk(&f, 0, 0x1000, &mut vec![Row::default(); 2], &mut vec![0; 24]).outcome,
            Outcome::Unavailable {
                reason: "saved event target registry changed during read"
            }
        ));
    }

    #[test]
    fn worker_saved_targets_grow_rows_and_names_before_first_and_later_publication() {
        let mut rows = Vec::new();
        let mut text = Vec::new();
        for (count, name_len) in [(1_500, 23), (5_000, 23), (5_000, 311)] {
            let mut fixture = Fixture::new();
            fixture.array(0x1000 + REGISTRY, 0xa000, count as u32);
            fixture.put(0xa000, &vec![0; STRIDE * count]);
            let name = vec![b'z'; name_len];
            let mut terminated = name.clone();
            terminated.push(0);
            fixture.put(0x4000, &terminated);
            let mut h = [0u8; 32];
            h[..8].copy_from_slice(&0x4000u64.to_le_bytes());
            h[16..24].copy_from_slice(&(name_len as u64).to_le_bytes());
            h[24..].copy_from_slice(&(name_len as u64).to_le_bytes());
            fixture.put(0x3040, &h);
            for ordinal in 0..count {
                let at = 0xa000 + ordinal * STRIDE;
                fixture.put(at, &(VTABLE_RVA.0 as u64).to_le_bytes());
                fixture.put(at + 0xc, &(ordinal as i32 + 1).to_le_bytes());
                fixture.put(at + 0x68, &2u16.to_le_bytes());
            }
            assert_eq!(
                super::count(&fixture, 0, 0x1000).outcome,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: count as u32
                }
            );
            let acquired = walk(&fixture, 0, 0x1000, &mut rows, &mut text);
            assert_eq!(acquired.outcome, Outcome::Rows { len: count });
            assert_eq!(acquired.bytes, count * name_len);
            for (ordinal, row) in rows[..count].iter().enumerate() {
                assert_eq!(
                    (row.ordinal, row.country),
                    (ordinal as u32, ordinal as i32 + 1)
                );
                assert_eq!(&text[row.name_start..row.name_start + row.name_len], name);
            }
        }
    }
}
