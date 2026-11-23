pub use engine_native_parts::chuman_layout::{VTABLE_RVA, STRIDE, COUNTRY_TAG};

use engine_sites::Rva;
use crate::{
    Memory, ccountry, cgame_state, cpdx_array::Array, msvc_string, read, read_slice, walk::Outcome,
};

pub const CONSTRUCTOR_RVA: Rva = Rva(0x00c1_0190);
pub const FIRST_STRING: usize = 0x20;
pub const NAME: usize = 0x40;
pub const DATE: usize = 0x78;
pub const FLAGS: usize = 0x94;
pub const PLAYER_KEY: usize = 0x98;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub ordinal: u32,
    pub id: i32,
    pub country: u32,
    pub flags: u8,
    pub user_start: usize,
    pub user_len: usize,
    pub name_start: usize,
    pub name_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControllerRow {
    pub country: u32,
    pub count: i8,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut names: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        let roster = game_state
            .checked_add(cgame_state::PLAYER_RECORDS)
            .and_then(|p| Array::read(memory, p, STRIDE))
            .ok_or("live player roster unreadable or invalid")?;
        let vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")?;
        output.ensure(roster.count as usize)?;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut truncated = false;
        for index in 0..roster.count {
            let address = roster
                .element(index, STRIDE)
                .ok_or("player address overflow")?;
            let fields = Fields::read(memory, address, vtable)?;
            if output[..len].iter().any(|row| row.id == fields.id) {
                return Err("duplicate native player key");
            }
            for previous in len as u32..index {
                let previous_key = roster
                    .element(previous, STRIDE)
                    .and_then(|p| p.checked_add(PLAYER_KEY))
                    .and_then(|p| read::<i32>(memory, p))
                    .ok_or("player key unreadable")?;
                if previous_key == fields.id {
                    return Err("duplicate native player key");
                }
            }
            let user = Literal::read(memory, &fields.strings[..32])?;
            let name = Literal::read(memory, &fields.strings[32..])?;
            let row_bytes = user
                .len
                .checked_add(name.len)
                .ok_or("player string byte count overflow")?;
            required_bytes = required_bytes
                .checked_add(row_bytes)
                .ok_or("player string byte count overflow")?;
            if !names.is_growing() {
                truncated |= row_bytes > names.len() - bytes_len;
            }
            truncated |= len == output.len();
            if truncated {
                continue;
            }
            let user_start = bytes_len;
            let name_start = user_start + user.len;
            user.copy_buffer(memory, &fields.strings[..32], &mut names, user_start)?;
            name.copy_buffer(memory, &fields.strings[32..], &mut names, name_start)?;
            output[len] = Row {
                ordinal: index,
                id: fields.id,
                country: fields.country,
                flags: fields.flags,
                user_start,
                user_len: user.len,
                name_start,
                name_len: name.len,
            };
            bytes_len += row_bytes;
            len += 1;
        }
        Ok(NameWalk {
            outcome: if truncated {
                Outcome::Truncated {
                    len,
                    unread_rows: roster.count - len as u32,
                }
            } else {
                Outcome::Rows { len }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

pub fn count_controllers(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_controllers_buffer(memory, base, game_state, crate::walk::Buffer::Count)
}

pub fn walk_controllers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<ControllerRow>,
) -> Outcome {
    walk_controllers_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_controllers_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, ControllerRow>,
) -> Outcome {
    let result = (|| {
        let counts = game_state
            .checked_add(cgame_state::HUMAN_STATUS)
            .and_then(|p| Array::read(memory, p, 1))
            .ok_or("human controller array unreadable or invalid")?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        output.ensure(countries.count() as usize)?;
        let mut len = 0_usize;
        let mut unread_rows = 0;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let count = counts
                .element(country.canonical_slot, 1)
                .and_then(|p| read::<i8>(memory, p))
                .ok_or("human controller count unreadable or outside array")?;
            if len == output.len() {
                unread_rows += 1;
                continue;
            }
            output[len] = ControllerRow {
                country: country.id,
                count,
            };
            len += 1;
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

struct Fields {
    id: i32,
    country: u32,
    flags: u8,
    strings: [u8; 64],
}

impl Fields {
    fn read(
        memory: &impl Memory,
        pointer: usize,
        expected_vtable: usize,
    ) -> Result<Self, &'static str> {
        let at = |offset| pointer.checked_add(offset).ok_or("player address overflow");
        let mut vtable = [0; 8];
        let mut key = [0; 4];
        let mut country = [0; 4];
        let mut flags = [0; 1];
        let mut strings = [0; 64];
        let copied = memory.copy_scattered(
            [
                pointer,
                at(PLAYER_KEY)?,
                at(COUNTRY_TAG)?,
                at(FLAGS)?,
                at(FIRST_STRING)?,
            ],
            &mut [
                &mut vtable,
                &mut key,
                &mut country,
                &mut flags,
                &mut strings,
            ],
        );
        if copied == 0 || u64::from_le_bytes(vtable) != expected_vtable as u64 {
            return Err("player vtable unreadable or mismatch");
        }
        if copied < 2 {
            return Err("player key unreadable");
        }
        let id = i32::from_le_bytes(key);
        if id == -1 {
            return Err("live player has invalid native key -1");
        }
        if copied < 3 {
            return Err("player country unreadable");
        }
        if copied < 4 {
            return Err("player flags unreadable");
        }
        if copied < 5 {
            return Err("player strings unreadable");
        }
        Ok(Self {
            id,
            country: u32::from_le_bytes(country),
            flags: flags[0],
            strings,
        })
    }
}

struct Literal {
    source: Option<usize>,
    len: usize,
}

impl Literal {
    fn read(memory: &impl Memory, header: &[u8]) -> Result<Self, &'static str> {
        let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("player string header invalid");
        }
        let len = usize::try_from(len).map_err(|_| "player string length overflow")?;
        if capacity == msvc_string::INLINE_CAPACITY {
            if header[len] != 0 {
                return Err("player string terminator invalid");
            }
            return Ok(Self { source: None, len });
        }
        let source = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("player string pointer invalid")?;
        let terminator = source
            .checked_add(len)
            .ok_or("player string address overflow")?;
        if read::<u8>(memory, terminator) != Some(0) {
            return Err("player string terminator unreadable or invalid");
        }
        Ok(Self {
            source: Some(source),
            len,
        })
    }

    fn copy_buffer(
        &self,
        memory: &impl Memory,
        header: &[u8],
        output: &mut crate::walk::Buffer<'_, u8>,
        start: usize,
    ) -> Result<(), &'static str> {
        if output.is_growing() {
            if let Some(source) = self.source {
                return match output {
                    crate::walk::Buffer::Growing(bytes) => msvc_string::copy_growing(
                        memory,
                        source,
                        self.len,
                        bytes,
                        start,
                        "player string bytes unreadable",
                        "player string allocation failed",
                    ),
                    crate::walk::Buffer::GrowingFrom(bytes, offset) => msvc_string::copy_growing(
                        memory,
                        source,
                        self.len,
                        bytes,
                        offset
                            .checked_add(start)
                            .ok_or("player string offset overflow")?,
                        "player string bytes unreadable",
                        "player string allocation failed",
                    ),
                    crate::walk::Buffer::Count | crate::walk::Buffer::Exact(_) => unreachable!(),
                };
            }
            output.ensure(
                start
                    .checked_add(self.len)
                    .ok_or("player string byte count overflow")?,
            )?;
        }
        self.copy(memory, header, &mut output[start..start + self.len])
    }

    fn copy(
        &self,
        memory: &impl Memory,
        header: &[u8],
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        if let Some(source) = self.source {
            if !read_slice(memory, source, output) {
                return Err("player string bytes unreadable");
            }
        } else {
            output.copy_from_slice(&header[..self.len]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod sizing_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const MEMORY_BASE: usize = 0x1000;
    const BASE: usize = 0x140000000;
    const GS: usize = 0x1100;
    const HUMANS: usize = 0x2000;
    const HEAP: usize = 0x4000;
    const COUNTRY_POINTERS: usize = 0x6000;
    const LINKS: usize = 0x6100;
    const COUNTS: usize = 0x6400;

    #[derive(Clone)]
    struct World(Vec<u8>);

    impl World {
        fn new(count: u32) -> Self {
            let mut world = Self(vec![0; 0x10000]);
            world.array(GS + cgame_state::PLAYER_RECORDS, HUMANS, count);
            for index in 0..count {
                world.player(index, index as i32, 0, 0, b"", b"");
            }
            world
        }

        fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: MEMORY_BASE,
                bytes: &self.0,
            }
        }

        fn put(&mut self, address: usize, bytes: &[u8]) {
            let start = address - MEMORY_BASE;
            self.0[start..start + bytes.len()].copy_from_slice(bytes);
        }

        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.put(address, &(data as u64).to_le_bytes());
            self.put(address + 8, &count.to_le_bytes());
            self.put(address + 12, &count.to_le_bytes());
        }

        fn string(&mut self, address: usize, bytes: &[u8], heap: usize) {
            self.put(address, &[0; 32]);
            self.put(address + 16, &(bytes.len() as u64).to_le_bytes());
            let capacity = if bytes.len() <= 15 {
                15
            } else {
                bytes.len() as u64
            };
            self.put(address + 24, &capacity.to_le_bytes());
            let source = if capacity == 15 {
                address
            } else {
                self.put(address, &(heap as u64).to_le_bytes());
                heap
            };
            self.put(source, bytes);
            self.put(source + bytes.len(), &[0]);
        }

        fn player(
            &mut self,
            index: u32,
            id: i32,
            country: u32,
            flags: u8,
            user: &[u8],
            name: &[u8],
        ) {
            let address = HUMANS + index as usize * STRIDE;
            self.put(address, &((BASE + VTABLE_RVA.0) as u64).to_le_bytes());
            self.put(address + PLAYER_KEY, &id.to_le_bytes());
            self.put(address + COUNTRY_TAG, &country.to_le_bytes());
            self.put(address + FLAGS, &[flags]);
            self.string(address + FIRST_STRING, user, HEAP + index as usize * 0x400);
            self.string(address + NAME, name, HEAP + index as usize * 0x400 + 0x200);
        }

        fn countries(&mut self) {
            let pointers = [0_u64, 0x7000, 0x7100, 0, 0x7200, 0x7300, 0, 0x7000];
            let links = [0_u32, 1, 2, 0, 4, 5, 0, 1];
            self.array(
                GS + cgame_state::COUNTRIES,
                COUNTRY_POINTERS,
                pointers.len() as u32,
            );
            self.array(GS + cgame_state::COUNTRY_LINKS, LINKS, links.len() as u32);
            self.array(GS + cgame_state::HUMAN_STATUS, COUNTS, 6);
            for (index, pointer) in pointers.into_iter().enumerate() {
                self.put(COUNTRY_POINTERS + index * 8, &pointer.to_le_bytes());
                self.put(LINKS + index * 4, &links[index].to_le_bytes());
            }
            for (pointer, id) in [(0x7000, 7_u32), (0x7100, 2), (0x7200, 4), (0x7300, 5)] {
                self.put(
                    pointer,
                    &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                self.put(pointer + ccountry::ID, &id.to_le_bytes());
            }
            self.put(COUNTS, &[0, 255, 0, 91, 128, 127, 0, 66]);
        }
    }

    #[test]
    fn live_identity_and_exact_strings_preserve_native_distinctions() {
        let mut world = World::new(3);
        let binary_name = [
            0, 255, 128, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
        ];
        world.player(0, i32::MIN, 0, 0xff, b"same user", &binary_name);
        world.player(1, -2, u32::MAX, 0, b"same user", &binary_name);
        world.player(2, i32::MAX, u32::MAX, 9, b"", b"123456789012345");
        world.put(
            GS + cgame_state::SAVED_PLAYER_RECORDS,
            &u64::MAX.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 3];
        let mut bytes = vec![0; 128];
        let result = walk(&world.memory(), BASE, GS, &mut rows, &mut bytes);
        assert_eq!(result.outcome, Outcome::Rows { len: 3 });
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [i32::MIN, -2, i32::MAX]);
        assert_eq!(rows.iter().map(|r| r.country).collect::<Vec<_>>(), [0, u32::MAX, u32::MAX]);
        assert_eq!(rows.iter().map(|r| r.flags).collect::<Vec<_>>(), [255, 0, 9]);
        for row in &rows[..2] {
            assert_eq!(
                &bytes[row.user_start..row.user_start + row.user_len],
                b"same user"
            );
            assert_eq!(
                &bytes[row.name_start..row.name_start + row.name_len],
                &binary_name
            );
        }
        assert_eq!(rows[2].user_len, 0);
        assert_eq!(
            &bytes[rows[2].name_start..result.bytes_len],
            b"123456789012345"
        );
        assert_eq!(result.required_bytes, result.bytes_len);
    }

    #[test]
    fn a_short_arena_grows_and_the_count_probe_reports_all_required_string_bytes() {
        let mut world = World::new(3);
        world.player(0, 1, 1, 1, b"A", b"BC");
        world.player(1, 2, 1, 1, &[0xf0; 256], b"D");
        world.player(2, 3, 2, 1, b"E", b"");
        let mut rows = vec![Row::default(); 3];
        let mut short = vec![0; 259];
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut rows, &mut short),
            NameWalk {
                outcome: Outcome::Rows { len: 3 },
                bytes_len: 261,
                required_bytes: 261,
            }
        );
        assert_eq!(&short[..3], b"ABC");
        assert_eq!(&short[3..259], &[0xf0; 256]);
        assert_eq!(&short[259..261], b"DE");
        assert_eq!(
            walk_buffer(
                &world.memory(),
                BASE,
                GS,
                crate::walk::Buffer::Count,
                crate::walk::Buffer::Count,
            ),
            NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 3
                },
                bytes_len: 0,
                required_bytes: 261,
            }
        );
    }

    #[test]
    fn duplicate_and_invalid_native_keys_are_unavailable_beyond_capacity() {
        let mut world = World::new(3);
        world.player(0, 1, 1, 0, b"", b"");
        world.player(1, 2, 1, 0, b"", b"");
        world.player(2, 2, 1, 0, b"", b"");
        {
            let mut rows = Vec::new();
            assert_eq!(
                walk(&world.memory(), BASE, GS, &mut rows, &mut Vec::new()).outcome,
                Outcome::Unavailable {
                    reason: "duplicate native player key"
                }
            );
        }
        world.player(2, -1, 0, 0, b"", b"");
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "live player has invalid native key -1"
            }
        );
    }

    #[test]
    fn signed_roster_geometry_and_late_vtable_faults_do_not_hide_as_empty_or_truncated() {
        let original = World::new(2);
        for (offset, bytes) in [
            (8, (-1_i32).to_le_bytes()),
            (12, (-1_i32).to_le_bytes()),
            (12, 3_i32.to_le_bytes()),
        ] {
            let mut world = original.clone();
            world.put(GS + cgame_state::PLAYER_RECORDS + offset, &bytes);
            assert_eq!(
                walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
                Outcome::Unavailable {
                    reason: "live player roster unreadable or invalid"
                }
            );
        }
        let mut world = original.clone();
        world.put(GS + cgame_state::PLAYER_RECORDS, &0_u64.to_le_bytes());
        assert!(matches!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable { .. }
        ));
        let mut world = original;
        world.put(HUMANS + STRIDE, &0_u64.to_le_bytes());
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "player vtable unreadable or mismatch"
            }
        );
    }

    #[test]
    fn malformed_strings_and_source_overflow_are_failures_even_after_capacity() {
        let mut original = World::new(2);
        original.player(1, 1, 0, 0, &[b'x'; 16], b"");
        let string = HUMANS + STRIDE + FIRST_STRING;
        let mut world = original.clone();
        world.put(string + 24, &14_u64.to_le_bytes());
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "player string header invalid"
            }
        );
        let mut world = original.clone();
        world.put(HEAP + 0x400 + 16, &[1]);
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "player string terminator unreadable or invalid"
            }
        );
        let mut world = original.clone();
        world.put(string, &u64::MAX.to_le_bytes());
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "player string address overflow"
            }
        );
        let mut world = original;
        world.put(string, &0_u64.to_le_bytes());
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "player string pointer invalid"
            }
        );
    }

    #[test]
    fn partial_field_or_heap_payload_fault_discards_an_already_read_prefix() {
        struct FaultMemory<'a> {
            memory: SliceMemory<'a>,
            failed_byte: usize,
        }

        impl Memory for FaultMemory<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if let Some(prefix) = self.failed_byte.checked_sub(address)
                    && prefix < destination.len()
                {
                    self.memory.copy(address, &mut destination[..prefix]);
                    return false;
                }
                self.memory.copy(address, destination)
            }
        }

        let mut world = World::new(2);
        world.player(1, 1, 2, 9, &[b'x'; 16], b"name");
        let address = HUMANS + STRIDE;
        for (failed_byte, reason) in [
            (address + 4, "player vtable unreadable or mismatch"),
            (address + PLAYER_KEY + 2, "player key unreadable"),
            (address + COUNTRY_TAG + 2, "player country unreadable"),
            (address + FLAGS, "player flags unreadable"),
            (address + FIRST_STRING + 50, "player strings unreadable"),
            (HEAP + 0x400 + 8, "player string bytes unreadable"),
        ] {
            let memory = FaultMemory {
                memory: world.memory(),
                failed_byte,
            };
            let mut rows = vec![Row::default(); 2];
            let mut names = vec![0; 128];
            assert_eq!(
                walk(&memory, BASE, GS, &mut rows, &mut names),
                NameWalk {
                    outcome: Outcome::Unavailable { reason },
                    bytes_len: 0,
                    required_bytes: 0,
                }
            );
        }
    }

    #[test]
    fn controller_counts_use_canonical_slots_and_keep_signed_values() {
        let mut world = World::new(0);
        world.countries();
        let mut rows = vec![ControllerRow::default(); 8];
        assert_eq!(
            walk_controllers(&world.memory(), BASE, GS, &mut rows),
            Outcome::Rows { len: 4 }
        );
        assert_eq!(
            &rows[..4],
            &[
                ControllerRow {
                    country: 7,
                    count: -1
                },
                ControllerRow {
                    country: 2,
                    count: 0
                },
                ControllerRow {
                    country: 4,
                    count: -128
                },
                ControllerRow {
                    country: 5,
                    count: 127
                },
            ]
        );
        assert_eq!(
            walk(&world.memory(), BASE, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn missing_canonical_controller_coverage_is_unavailable_even_beyond_capacity() {
        let mut world = World::new(0);
        world.countries();
        world.put(GS + cgame_state::HUMAN_STATUS + 12, &5_u32.to_le_bytes());
        assert_eq!(
            walk_controllers(&world.memory(), BASE, GS, &mut Vec::new()),
            Outcome::Unavailable {
                reason: "human controller count unreadable or outside array"
            }
        );
        world.put(GS + cgame_state::HUMAN_STATUS + 12, &(-1_i32).to_le_bytes());
        assert_eq!(
            walk_controllers(&world.memory(), BASE, GS, &mut Vec::new()),
            Outcome::Unavailable {
                reason: "human controller array unreadable or invalid"
            }
        );
    }
}
