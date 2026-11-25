use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::Cell;

struct CountedFields<'a> {
    memory: SliceMemory<'a>,
    guards: Cell<usize>,
    blocked: Option<usize>,
}

impl CountedFields<'_> {
    fn allowed(&self, address: usize, len: usize) -> bool {
        self.blocked
            .is_none_or(|blocked| blocked < address || blocked >= address.saturating_add(len))
    }
}

impl Memory for CountedFields<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.allowed(address, output.len()) && self.memory.copy(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        output: &mut [&mut [u8]; N],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        for (index, (address, bytes)) in addresses.into_iter().zip(output).enumerate() {
            if !bytes.is_empty()
                && (!self.allowed(address, bytes.len()) || !self.memory.copy(address, bytes))
            {
                return index;
            }
        }
        N
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        output: &mut [u8],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        struct Fields<'a, 'b>(&'a CountedFields<'b>);
        impl Memory for Fields<'_, '_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.allowed(address, output.len()) && self.0.memory.copy(address, output)
            }
        }
        Fields(self).copy_records(sources, layout, output)
    }
}

struct RegistryMemory {
    bytes: Vec<u8>,
    root: Option<u64>,
    blocked: Option<usize>,
    pointer_copies: Cell<usize>,
    identity_batches: Cell<usize>,
}

impl RegistryMemory {
    fn new(count: usize) -> Self {
        let mut memory = Self {
            bytes: vec![0; 0x4000 + count * 32],
            root: Some(0x1000),
            blocked: None,
            pointer_copies: Cell::new(0),
            identity_batches: Cell::new(0),
        };
        memory.put(
            0x1000 + cgame_state::EQUIPMENT_VARIANTS,
            &0x2000_u64.to_le_bytes(),
        );
        memory.put(
            0x1000 + cgame_state::EQUIPMENT_VARIANTS_CAPACITY,
            &(count as u32).to_le_bytes(),
        );
        memory.put(
            0x1000 + cgame_state::EQUIPMENT_VARIANTS_COUNT,
            &(count as u32).to_le_bytes(),
        );
        for index in 0..count {
            let pointer = 0x4000 + index * 32;
            memory.put(0x2000 + index * 8, &(pointer as u64).to_le_bytes());
            memory.put(pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
            let id = if index == 64 {
                0
            } else {
                u32::MAX - index as u32
            };
            let handle = (u64::from(id) << 32) | u64::from(REFERENCE_TYPE);
            memory.put(pointer + 8, &handle.to_le_bytes());
        }
        memory
    }

    fn put(&mut self, address: usize, value: &[u8]) {
        let start = address - 0x1000;
        self.bytes[start..start + value.len()].copy_from_slice(value);
    }
}

impl Memory for RegistryMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self
            .blocked
            .is_some_and(|p| (address..address + destination.len()).contains(&p))
        {
            destination.fill(0xff);
            return false;
        }
        if address == cgame_state::SINGLETON_RVA.0 && destination.len() == 8 {
            let Some(root) = self.root else {
                return false;
            };
            destination.copy_from_slice(&root.to_le_bytes());
            return true;
        }
        if (0x2000..0x4000).contains(&address) {
            self.pointer_copies.set(self.pointer_copies.get() + 1);
        }
        SliceMemory {
            base: 0x1000,
            bytes: &self.bytes,
        }
        .copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.identity_batches.set(self.identity_batches.get() + 1);
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !read_slice(self, address, destination) {
                return index;
            }
        }
        N
    }
}

#[test]
fn registry_native_count_does_not_allocate_before_pointer_reads() {
    let mut memory = RegistryMemory::new(0);
    memory.put(
        0x1000 + cgame_state::EQUIPMENT_VARIANTS,
        &0x9000_u64.to_le_bytes(),
    );
    for offset in [
        cgame_state::EQUIPMENT_VARIANTS_CAPACITY,
        cgame_state::EQUIPMENT_VARIANTS_COUNT,
    ] {
        memory.put(0x1000 + offset, &32_768_u32.to_le_bytes());
    }
    let mut rows = Vec::new();
    assert_eq!(
        walk_registry(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable {
            reason: "variant registry pointers unreadable",
        }
    );
    assert_eq!(rows.capacity(), 0);
}

#[test]
fn equipment_names_grow_only_for_readable_payload_chunks() {
    let mut header = [0; 32];
    header[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
    let payload = [b'X'; 513];
    for (len, available, expected_len) in [(32_768_u64, 0, 0), (513, 256, 256)] {
        header[16..24].copy_from_slice(&len.to_le_bytes());
        header[24..].copy_from_slice(&len.to_le_bytes());
        let mut output = Vec::new();
        assert_eq!(
            Literal::from_header(header).unwrap().copy_growing(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &payload[..available]
                },
                &mut output,
                0,
                "allocation failed",
            ),
            Err("equipment string bytes or terminator unreadable")
        );
        assert_eq!(output.len(), expected_len);
        assert!(output.capacity() <= 256);
    }
    let mut payload = payload.to_vec();
    payload.push(0);
    let mut output = vec![b'P'];
    Literal::from_header(header)
        .unwrap()
        .copy_growing(
            &SliceMemory {
                base: 0x1000,
                bytes: &payload,
            },
            &mut output,
            1,
            "allocation failed",
        )
        .unwrap();
    assert_eq!(output[0], b'P');
    assert_eq!(&output[1..], &payload[..513]);
}

#[test]
fn registry_preserves_dense_native_order_handles_and_batches_guards() {
    let memory = RegistryMemory::new(129);
    let mut rows = Vec::new();
    assert_eq!(
        walk_registry(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 129 }
    );
    for (index, row) in rows.iter().enumerate() {
        let id = if index == 64 {
            0
        } else {
            u32::MAX - index as u32
        };
        assert_eq!(row.id, (u64::from(id) << 32) | u64::from(REFERENCE_TYPE));
        assert_eq!(row.pointer(), 0x4000 + index * 32);
    }
    assert_eq!(memory.pointer_copies.get(), 3);
    assert_eq!(memory.identity_batches.get(), 3);
}

#[test]
fn registry_grows_from_the_original_header_and_keeps_fault_prefixes() {
    for initial in [0, 1, 3] {
        let mut rows = vec![RegistryRow::default(); initial];
        let mut memory = RegistryMemory::new(3);
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            (memory.pointer_copies.get(), memory.identity_batches.get()),
            (1, 1)
        );
        memory.blocked = Some(0x4000 + 2 * 32 + 8);
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "variant identity unreadable"
            }
        );
        assert_eq!(rows[0].pointer(), 0x4000);
        assert_eq!(rows[1].pointer(), 0x4020);
        memory.blocked = Some(0x2000 + 2 * 8 + 7);
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "variant registry pointers unreadable"
            }
        );
        memory.blocked = None;
        memory.put(0x2000 + 2 * 8, &0_u64.to_le_bytes());
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "variant registry pointer null or invalid"
            }
        );
    }
    assert_eq!(
        walk_registry(&RegistryMemory::new(0), 0, 0x1000, &mut Vec::new()),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn registry_refuses_wrong_roots_headers_and_partial_or_invalid_identities() {
    let mut rows = vec![RegistryRow::default(); 1];
    for root in [None, Some(0), Some(0x1001)] {
        let mut memory = RegistryMemory::new(1);
        memory.root = root;
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "variant registry game state unreadable or changed"
            }
        );
    }
    assert!(matches!(
        walk_registry(&RegistryMemory::new(1), 0, 0, &mut rows),
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        walk_registry(&RegistryMemory::new(1), usize::MAX, 0x1000, &mut rows),
        Outcome::Unavailable { .. }
    ));
    for (offset, value) in [
        (cgame_state::EQUIPMENT_VARIANTS_CAPACITY, u32::MAX),
        (cgame_state::EQUIPMENT_VARIANTS_COUNT, 2),
    ] {
        let mut memory = RegistryMemory::new(1);
        memory.put(0x1000 + offset, &value.to_le_bytes());
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "variant registry array unreadable or invalid"
            }
        );
    }
    let mut memory = RegistryMemory::new(1);
    memory.put(
        0x1000 + cgame_state::EQUIPMENT_VARIANTS,
        &(u64::MAX - 3).to_le_bytes(),
    );
    assert!(matches!(
        walk_registry(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable { .. }
    ));
    for blocked in [
        0x1000 + cgame_state::EQUIPMENT_VARIANTS_COUNT + 3,
        0x4000 + 15,
    ] {
        let mut memory = RegistryMemory::new(1);
        memory.blocked = Some(blocked);
        assert!(matches!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable { .. }
        ));
    }
    for offset in [0, TYPE] {
        let mut memory = RegistryMemory::new(1);
        memory.put(0x4000 + offset, &0_u32.to_le_bytes());
        assert_eq!(
            walk_registry(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "variant vtable or reference type mismatch"
            }
        );
    }
}

#[test]
fn registry_keeps_duplicate_slots_and_conflicting_handles_for_spool_rejection() {
    let mut memory = RegistryMemory::new(3);
    memory.put(0x2000 + 8, &0x4000_u64.to_le_bytes());
    memory.put(
        0x4000 + 2 * 32 + 8,
        &0xffff_ffff_0000_0046_u64.to_le_bytes(),
    );
    let mut rows = vec![RegistryRow::default(); 3];
    assert_eq!(
        walk_registry(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 3 }
    );
    assert!(rows.iter().all(|row| row.id == 0xffff_ffff_0000_0046));
    assert_eq!(rows[0].pointer(), rows[1].pointer());
    assert_ne!(rows[0].pointer(), rows[2].pointer());
}

fn source(bytes: &[u8]) -> crate::cequipment_variant::RegistryRow {
    crate::cequipment_variant::RegistryRow::read(
        &SliceMemory {
            base: 0x1000,
            bytes,
        },
        0,
        0x1000,
    )
    .unwrap()
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xb00];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[8..16]
        .copy_from_slice(&((u64::from(u32::MAX) << 32) | u64::from(REFERENCE_TYPE)).to_le_bytes());
    bytes[EQUIPMENT_TOKEN..EQUIPMENT_TOKEN + 4].copy_from_slice(&0x8000_0042_u32.to_le_bytes());
    bytes[CREATOR_LINK..CREATOR_LINK + 4].copy_from_slice(&900_u32.to_le_bytes());
    bytes[ORIGIN_LINK..ORIGIN_LINK + 4].copy_from_slice(&901_u32.to_le_bytes());
    bytes[DEFINITION..DEFINITION + 8].copy_from_slice(&0x1500_u64.to_le_bytes());
    bytes[0x500..0x508].copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[0x508..0x50c].copy_from_slice(&0x8000_0042_u32.to_le_bytes());
    let archetype = 0x500 + cequipment_type::ARCHETYPE;
    bytes[archetype..archetype + 8].copy_from_slice(&0x1500_u64.to_le_bytes());
    for offset in [
        DISPLAY_NAME,
        VARIANT_NAME,
        EXPLICIT_ENTITY,
        ICON_OVERRIDE_TOKEN,
    ] {
        bytes[offset + 24..offset + 32].copy_from_slice(&15_u64.to_le_bytes());
    }
    bytes[ICON_OVERRIDE_KIND..ICON_OVERRIDE_KIND + 4].copy_from_slice(&1_u32.to_le_bytes());
    bytes
}

#[test]
fn design_state_preserves_ancestry_attribution_role_and_raw_flags() {
    struct StaleReference<'a>(SliceMemory<'a>);
    impl Memory for StaleReference<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine_native_parts::creference_registry::LOW_ROOTS_RVA.0 + 8 && output.len() == 8 {
                output.fill(0);
                true
            } else {
                self.0.copy(address, output)
            }
        }
    }
    let mut bytes = fixture();
    bytes[PARENT..PARENT + 8].copy_from_slice(&0x1000_u64.to_le_bytes());
    bytes[DESIGN_TEAM..DESIGN_TEAM + 8].copy_from_slice(&0xffff_ffff_0000_0001_u64.to_le_bytes());
    bytes[ROLE_ICON_INDEX..ROLE_ICON_INDEX + 4].copy_from_slice(&i32::MAX.to_le_bytes());
    bytes[PENDING_PARENT_VERSION..PENDING_PARENT_VERSION + 2]
        .copy_from_slice(&(-17_i16).to_le_bytes());
    bytes[IS_FRAME] = 0xff;
    bytes[AUTO_UPGRADED] = 0x80;
    bytes[MANPOWER..MANPOWER + 4].copy_from_slice(&i32::MIN.to_le_bytes());
    let memory = StaleReference(SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    });
    let state = read_design(&memory, 0, 0x1000).unwrap();
    assert_eq!(state.parent, Some(source(&bytes).id));
    assert_eq!(state.design_team, 0xffff_ffff_0000_0001);
    assert!(!state.design_team_serialized);
    assert_eq!(
        (state.role_icon_index, state.pending_parent_version),
        (i32::MAX, -17)
    );
    assert_eq!(
        (state.is_frame, state.auto_upgraded, state.manpower),
        (0xff, 0x80, i32::MIN)
    );
    bytes[PARENT..PARENT + 8].copy_from_slice(&0x1100_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        read_design(&memory, 0, 0x1000).unwrap_err(),
        "variant vtable or reference type mismatch"
    );
}

#[test]
fn design_build_resources_decode_amount_before_resource_id_and_ignore_padding() {
    let mut bytes = fixture();
    let payload = bytes.len();
    bytes.resize(payload + 32, 0);
    bytes[BUILD_RESOURCES..BUILD_RESOURCES + 8]
        .copy_from_slice(&(0x1000_u64 + payload as u64).to_le_bytes());
    for count in [BUILD_RESOURCES + 8, BUILD_RESOURCES + 12] {
        bytes[count..count + 4].copy_from_slice(&2_u32.to_le_bytes());
    }
    for (ordinal, (resource, amount)) in [(5_000_u32, i64::MIN), (7, 123_457_i64)]
        .into_iter()
        .enumerate()
    {
        let at = payload + ordinal * 16;
        bytes[at..at + 8].copy_from_slice(&amount.to_le_bytes());
        bytes[at + 8..at + 12].copy_from_slice(&resource.to_le_bytes());
        bytes[at + 12..at + 16].fill(0xff);
    }
    let state = read_design(
        &SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        0,
        0x1000,
    )
    .unwrap();
    assert_eq!(
        state.build_resources,
        [
            VariantStatEntry {
                index: 5_000,
                value: i64::MIN
            },
            VariantStatEntry {
                index: 7,
                value: 123_457
            },
        ]
    );
}

#[test]
fn native_null_equipment_archetypes_keep_their_stored_token() {
    let mut bytes = fixture();
    bytes.resize(0x1200, 0);
    let archetype = 0x500 + cequipment_type::ARCHETYPE;
    bytes[archetype..archetype + 8].copy_from_slice(&0x2000_u64.to_le_bytes());
    bytes[0x1000..0x1008]
        .copy_from_slice(&(cequipment_type::NULL_VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[0x1008..0x100c].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());
    let variants = vec![source(&bytes); 100];
    let mut rows = Vec::new();
    for vtable in [
        cequipment_type::NULL_VTABLE_RVA,
        cequipment_type::VTABLE_RVA,
    ] {
        bytes[0x1000..0x1008].copy_from_slice(&(vtable.0 as u64).to_le_bytes());
        let result = walk_definitions(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            &variants,
            &mut rows,
            &mut Vec::new(),
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 100 });
        assert!(
            rows.iter()
                .all(|row| row.identity.archetype_token == 0xffff_fffe)
        );
    }
    bytes[0x1000..0x1008].copy_from_slice(&0x1234_u64.to_le_bytes());
    assert_eq!(
        walk_definitions(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &variants,
            &mut rows,
            &mut Vec::new()
        )
        .outcome,
        Outcome::Unavailable {
            reason: "variant equipment archetype identity mismatch"
        }
    );
}

#[test]
fn definition_preserves_native_handles_links_and_bytes() {
    let mut bytes = fixture();
    bytes[DISPLAY_NAME..DISPLAY_NAME + 3].copy_from_slice(&[b'A', 0xff, b'Z']);
    bytes[DISPLAY_NAME + 16..DISPLAY_NAME + 24].copy_from_slice(&3_u64.to_le_bytes());
    bytes[APPEND_VARIANT_NAME] = 0xfe;
    bytes[OBSOLETE] = 0x7f;
    bytes[VERSION] = u8::MAX;
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![DefinitionRow::default(); 1];
    let mut names = vec![0; 3];
    let result = walk_definitions(&memory, 0, &[source(&bytes)], &mut rows, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].identity.id, 0xffff_ffff_0000_0046);
    assert_eq!(
        (rows[0].identity.creator_link, rows[0].identity.origin_link),
        (900, 901)
    );
    assert_eq!(
        (
            rows[0].identity.append_variant_name,
            rows[0].identity.obsolete
        ),
        (0xfe, 0x7f)
    );
    assert_eq!(names, [b'A', 0xff, b'Z']);
    assert_eq!(rows[0].variant_name.len, 0);
    assert_eq!((rows[0].icon_active, rows[0].icon_kind), (0, 1));
    assert_eq!(rows[0].icon.len, 0);
    assert_eq!(rows[0].version, u8::MAX);
}

#[test]
fn version_bytes_keep_zero_and_high_bits_and_refuse_a_second_batch_fault() {
    const COUNT: usize = 65;
    let one = fixture();
    let mut bytes = vec![0; COUNT * one.len()];
    for (index, object) in bytes.chunks_exact_mut(one.len()).enumerate() {
        object.copy_from_slice(&one);
        object[ID..ID + 4].copy_from_slice(&(index as u32).to_le_bytes());
        object[VERSION] = (index as u8).wrapping_mul(4);
    }
    let source_memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let variants = (0..COUNT)
        .map(|index| RegistryRow::read(&source_memory, 0, 0x1000 + index * one.len()).unwrap())
        .collect::<Vec<_>>();
    let memory = CountedFields {
        memory: source_memory,
        guards: Cell::new(0),
        blocked: None,
    };
    let mut rows = Vec::new();
    let mut names = Vec::new();
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut names).outcome,
        Outcome::Rows { len: COUNT }
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.version, (index as u8).wrapping_mul(4));
    }
    assert_eq!(memory.guards.get(), COUNT.div_ceil(64) * 3);
    let memory = CountedFields {
        blocked: Some(variants[64].pointer() + VERSION),
        ..memory
    };
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut names).outcome,
        Outcome::Unavailable {
            reason: "variant version byte unreadable",
        }
    );
}

#[test]
fn icon_entry_keeps_its_raw_active_byte_kind_and_string() {
    let mut bytes = fixture();
    bytes[ICON_OVERRIDE_ACTIVE] = 0x7f;
    bytes[ICON_OVERRIDE_KIND..ICON_OVERRIDE_KIND + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[ICON_OVERRIDE_TOKEN..ICON_OVERRIDE_TOKEN + 5].copy_from_slice(b"GFX_x");
    bytes[ICON_OVERRIDE_TOKEN + 16..ICON_OVERRIDE_TOKEN + 24].copy_from_slice(&5_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let result = walk_definitions(&memory, 0, &[source(&bytes)], &mut rows, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!((rows[0].icon_active, rows[0].icon_kind), (0x7f, u32::MAX));
    let icon = rows[0].icon;
    assert_eq!(&names[icon.start..icon.start + icon.len], b"GFX_x");
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..ICON_OVERRIDE_ACTIVE + 1],
    };
    assert_eq!(
        walk_definitions(&short, 0, &[source(&bytes)], &mut rows, &mut names).outcome,
        Outcome::Unavailable {
            reason: "variant icon entry unreadable"
        }
    );
    bytes[ICON_OVERRIDE_TOKEN + 24..ICON_OVERRIDE_TOKEN + 32].copy_from_slice(&0_u64.to_le_bytes());
    assert_eq!(
        walk_definitions(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &[source(&bytes)],
            &mut rows,
            &mut names
        )
        .outcome,
        Outcome::Unavailable {
            reason: "equipment string header invalid"
        }
    );
}

#[test]
fn empty_name_arena_grows_for_every_original_occurrence() {
    let mut bytes = fixture();
    bytes[DISPLAY_NAME] = b'X';
    bytes[DISPLAY_NAME + 16..DISPLAY_NAME + 24].copy_from_slice(&1_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![DefinitionRow::default(); 2];
    let result = walk_definitions(
        &memory,
        0,
        &[source(&bytes), source(&bytes)],
        &mut rows,
        &mut Vec::new(),
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!((result.bytes_len, result.required_bytes), (2, 2));
}

#[test]
fn mismatched_definition_and_corrupt_strings_are_unavailable() {
    let mut bytes = fixture();
    bytes[0x508] ^= 1;
    assert_eq!(
        walk_definitions(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &[source(&bytes)],
            &mut Vec::new(),
            &mut Vec::new()
        )
        .outcome,
        Outcome::Unavailable {
            reason: "variant equipment definition identity mismatch"
        }
    );
    bytes[0x508] ^= 1;
    bytes[DISPLAY_NAME + 16..DISPLAY_NAME + 24].copy_from_slice(&16_u64.to_le_bytes());
    assert!(matches!(
        walk_definitions(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &[source(&bytes)],
            &mut vec![DefinitionRow::default(); 1],
            &mut vec![0; 40]
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn stat_spans_preserve_signed_extremes_and_grow_for_the_full_census() {
    let mut bytes = fixture();
    let values = [
        i64::MIN,
        i64::MAX,
        -1,
        0,
        1_i64 << 40,
        50,
        100_001,
        123,
        9,
        42,
    ];
    for (lane, value) in STAT_LANES.into_iter().zip(values) {
        bytes[lane.offset..lane.offset + 8].copy_from_slice(&value.to_le_bytes());
        let base_offset = 0x500 + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[base_offset..base_offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[BUILD_COST..BUILD_COST + 8].copy_from_slice(&42_i64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![StatsRow::default(); 1];
    assert_eq!(
        walk_stats(&memory, 0, &[source(&bytes), source(&bytes)], &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].values, values.map(Some));
    assert_eq!(rows[1].values, values.map(Some));
    let mut partial_rows = Vec::new();
    assert_eq!(
        walk_stats(
            &memory,
            0,
            &[
                source(&bytes),
                RegistryRow {
                    id: 0,
                    address: usize::MAX
                }
            ],
            &mut partial_rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(partial_rows[0].variant, source(&bytes).id);
    assert_eq!(partial_rows[0].values, values.map(Some));
    assert!(matches!(
        walk_stats(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes[..BUILD_COST_IC_STAT + 7]
            },
            0,
            &[source(&bytes)],
            &mut rows
        ),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn variant_record_batches_preserve_order_and_do_not_hide_an_earlier_definition_error() {
    const COUNT: usize = 65;
    let one = fixture();
    let mut bytes = vec![0; COUNT * one.len()];
    for (index, object) in bytes.chunks_exact_mut(one.len()).enumerate() {
        object.copy_from_slice(&one);
        object[ID..ID + 4].copy_from_slice(&(index as u32).to_le_bytes());
    }
    let source_memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let variants: [_; COUNT] = std::array::from_fn(|index| {
        RegistryRow::read(&source_memory, 0, 0x1000 + index * one.len()).unwrap()
    });
    let memory = CountedFields {
        memory: source_memory,
        guards: Cell::new(0),
        blocked: None,
    };
    let mut rows = Vec::new();
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut Vec::new()).outcome,
        Outcome::Rows { len: COUNT }
    );
    assert_eq!(memory.guards.get(), 6);
    for (row, source) in rows.iter().zip(&variants) {
        assert_eq!(row.identity.id, source.id);
    }
    memory.guards.set(0);
    let mut stats = Vec::new();
    assert_eq!(
        walk_stats(&memory, 0, &variants, &mut stats),
        Outcome::Rows { len: COUNT }
    );
    assert_eq!(memory.guards.get(), 4 + COUNT);
    let memory = CountedFields {
        blocked: Some(variants[32].pointer() + MISSION_MASK),
        ..memory
    };
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "variant classifier flags or mission mask unreadable"
        }
    );
    assert_eq!(
        walk_stats(&memory, 0, &variants, &mut stats),
        Outcome::Rows { len: COUNT }
    );
    bytes[0x500] ^= 1;
    let memory = CountedFields {
        memory: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        guards: Cell::new(0),
        blocked: Some(variants[1].pointer() + CLASSIFIER_FLAGS),
    };
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "variant equipment definition identity mismatch"
        }
    );
}

#[test]
fn heap_copy_checks_terminator_in_same_guard_and_name_fault_does_not_poison_stats() {
    let mut bytes = fixture();
    bytes[DISPLAY_NAME..DISPLAY_NAME + 8].copy_from_slice(&0x1600_u64.to_le_bytes());
    bytes[DISPLAY_NAME + 16..DISPLAY_NAME + 24].copy_from_slice(&4_u64.to_le_bytes());
    bytes[DISPLAY_NAME + 24..DISPLAY_NAME + 32].copy_from_slice(&31_u64.to_le_bytes());
    bytes[0x600..0x604].copy_from_slice(&[b'A', 0xff, 0, b'Z']);
    let variants = [source(&bytes)];
    let memory = CountedFields {
        memory: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        guards: Cell::new(0),
        blocked: None,
    };
    let mut rows = Vec::new();
    let mut names = Vec::new();
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut names).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(names, [b'A', 0xff, 0, b'Z']);
    assert_eq!(memory.guards.get(), 4);
    let memory = CountedFields {
        blocked: Some(0x1604),
        ..memory
    };
    assert!(matches!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut names).outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        walk_definitions(&memory, 0, &variants, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(
        walk_stats(&memory, 0, &variants, &mut vec![StatsRow::default(); 1]),
        Outcome::Rows { len: 1 }
    );
}

#[test]
fn design_organisation_preserves_frozen_author_marker_bonus_order_and_faults() {
    let mut bytes = fixture();
    let author = (912_u64 << 32) | 79;
    bytes[INDUSTRIAL_ORGANISATION..INDUSTRIAL_ORGANISATION + 8]
        .copy_from_slice(&author.to_le_bytes());
    bytes[ORGANISATION_TRAIT_COUNT..ORGANISATION_TRAIT_COUNT + 4]
        .copy_from_slice(&(-7_i32).to_le_bytes());
    let bonuses_at = bytes.len();
    bytes.resize(bonuses_at + 48, 0);
    bytes[ORGANISATION_BONUSES..ORGANISATION_BONUSES + 8]
        .copy_from_slice(&(0x1000_u64 + bonuses_at as u64).to_le_bytes());
    for offset in [8, 12] {
        bytes[ORGANISATION_BONUSES + offset..ORGANISATION_BONUSES + offset + 4]
            .copy_from_slice(&3_u32.to_le_bytes());
    }
    let expected = [(77_u32, i64::MIN), (12, i64::MAX), (77, -21)];
    for (index, (key, value)) in expected.iter().enumerate() {
        let at = bonuses_at + index * 16;
        bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
    }
    let source = source(&bytes);
    let mut rows = Vec::new();
    assert_eq!(
        walk_stats(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &[source],
            &mut rows
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].organisation.author, author);
    assert_eq!(rows[0].organisation.trait_count, -7);
    assert_eq!(
        rows[0]
            .organisation
            .bonuses
            .iter()
            .map(|b| (b.key, b.value_raw))
            .collect::<Vec<_>>(),
        expected
    );
    struct Quiet;
    impl OrganisationCarry for Quiet {
        fn fresh(&mut self, _: usize, _: &RegistryRow) -> Result<bool, &'static str> {
            Ok(false)
        }
    }
    bytes.truncate(bonuses_at);
    assert_eq!(
        walk_stats_carried(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &[source],
            &mut rows,
            &mut Quiet,
            &mut (),
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].organisation.author, author);
    assert!(matches!(
        walk_stats(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            &[source],
            &mut rows
        ),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn variant_certified_deep_state_is_not_read_and_mixed_batches_only_read_misses() {
    use std::sync::Arc;
    let one = fixture();
    let mut bytes = one.clone();
    bytes.extend_from_slice(&one);
    bytes[one.len() + ID..one.len() + ID + 4].copy_from_slice(&12_u32.to_le_bytes());
    let source_memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let variants = [
        RegistryRow::read(&source_memory, 0, 0x1000).unwrap(),
        RegistryRow::read(&source_memory, 0, 0x1000 + one.len()).unwrap(),
    ];
    let memory = CountedFields {
        memory: source_memory,
        guards: Cell::new(0),
        blocked: Some(0x1000 + BONUS_SOURCES),
    };
    let state = Arc::new(VariantDesignState {
        manpower: 4321,
        ..Default::default()
    });
    let mut output = Vec::new();
    let mut strings = Vec::new();
    let outcome = walk_definitions_carried(
        &memory,
        0,
        &variants,
        &mut output,
        &mut strings,
        |_| Ok(Some(Arc::clone(&state))),
        |state| Ok(Arc::new(state)),
    );
    assert_eq!(outcome.outcome, Outcome::Rows { len: 2 });
    assert_eq!(memory.guards.get(), 3);
    assert!(output.iter().all(|row| Arc::ptr_eq(&row.design, &state)));
    memory.guards.set(0);
    let outcome = walk_definitions_carried(
        &memory,
        0,
        &variants,
        &mut output,
        &mut strings,
        |source| Ok((source.id == variants[0].id).then(|| Arc::clone(&state))),
        |state| Ok(Arc::new(state)),
    );
    assert_eq!(outcome.outcome, Outcome::Rows { len: 2 });
    assert_eq!(memory.guards.get(), 4);
    assert!(Arc::ptr_eq(&output[0].design, &state));
    assert!(!Arc::ptr_eq(&output[1].design, &state));
    assert_eq!(output[1].design.manpower, 0);
    assert!(matches!(
        walk_definitions(&memory, 0, &variants, &mut Vec::new(), &mut strings).outcome,
        Outcome::Unavailable {
            reason: "variant design fields unreadable"
        }
    ));
}

#[test]
fn applied_sources_authenticate_plain_and_trait_layout_before_following_the_extension() {
    for (table, trait_source) in [
        (SPECIFIC_BONUS_VTABLE_RVA, false),
        (TRAIT_BONUS_VTABLE_RVA, true),
    ] {
        let mut bytes = fixture();
        bytes.resize(0x1800, 0);
        bytes[BONUS_SOURCES..BONUS_SOURCES + 8].copy_from_slice(&0x1f00_u64.to_le_bytes());
        bytes[BONUS_SOURCES + 8..BONUS_SOURCES + 12].copy_from_slice(&1_i32.to_le_bytes());
        bytes[BONUS_SOURCES + 12..BONUS_SOURCES + 16].copy_from_slice(&1_i32.to_le_bytes());
        bytes[0xf00..0xf08].copy_from_slice(&0x2000_u64.to_le_bytes());
        bytes[0x1000..0x1008].copy_from_slice(&(table.0 as u64).to_le_bytes());
        let memory = CountedFields {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            guards: Cell::new(0),
            blocked: (!trait_source).then_some(0x2000 + BONUS_IDEA),
        };
        let state = design::read_design(&memory, 0, 0x1000).unwrap();
        assert_eq!(state.bonus_sources.len(), 1);
        assert!(state.bonus_sources[0].idea.is_none());
        assert_eq!(
            bonus_source_size(&memory, 0, 0x2000).unwrap(),
            BONUS_IDEA + usize::from(trait_source) * 8
        );
    }
    let bytes = [0; 8];
    assert!(
        bonus_source_size(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000
        )
        .is_err()
    );
}

#[test]
fn cached_stat_lane_witnesses_reject_mislabeled_and_unverified_values() {
    assert_eq!(AIR_RANGE_STAT, STATS + 46 * 8);
    assert_eq!(AIR_DEFENCE_STAT, STATS + 47 * 8);
    assert_eq!(AIR_ATTACK_STAT, STATS + 48 * 8);
    assert_eq!(AIR_AGILITY_STAT, STATS + 49 * 8);
    assert_eq!(MAXIMUM_SPEED_STAT, STATS + 62 * 8);
    assert_eq!(RELIABILITY_STAT, STATS + 65 * 8);
    assert_eq!(WEIGHT_STAT, STATS + 67 * 8);
    assert_eq!(THRUST_STAT, STATS + 68 * 8);
    assert_eq!(FUEL_CONSUMPTION_STAT, STATS + 69 * 8);
    assert_eq!(BUILD_COST_IC_STAT, STATS + 77 * 8);

    struct PlaneMemory<'a> {
        bytes: &'a [u8],
        thrust_factor: Option<i64>,
        dlc_mask: Option<u32>,
    }
    impl Memory for PlaneMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == THRUST_WEIGHT_AGILITY_FACTOR_RVA.0 && output.len() == 8 {
                let Some(factor) = self.thrust_factor else {
                    return false;
                };
                output.copy_from_slice(&factor.to_le_bytes());
                return true;
            }
            if address == engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4
            {
                let Some(mask) = self.dlc_mask else {
                    return false;
                };
                output.copy_from_slice(&mask.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    struct RecordingWitness {
        tokens: [Result<(u64, u64), &'static str>; 2],
        calls: Cell<usize>,
        withheld: Vec<(&'static str, &'static str)>,
    }
    impl RecordingWitness {
        fn new(
            before: Result<(u64, u64), &'static str>,
            after: Result<(u64, u64), &'static str>,
        ) -> Self {
            Self {
                tokens: [before, after],
                calls: Cell::new(0),
                withheld: Vec::new(),
            }
        }
    }
    impl StatWitness for RecordingWitness {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            let call = self.calls.get();
            self.calls.set(call + 1);
            self.tokens[call.min(1)].map(|(e, r)| QuietToken::new(e, r))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.withheld.push((STAT_LANES[lane].name, reason));
        }
    }

    let mut bytes = fixture();
    let author = (912_u64 << 32) | 79;
    bytes[INDUSTRIAL_ORGANISATION..INDUSTRIAL_ORGANISATION + 8]
        .copy_from_slice(&author.to_le_bytes());
    bytes[CLASSIFIER_FLAGS..CLASSIFIER_FLAGS + 8].copy_from_slice(&0x400_u64.to_le_bytes());
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let base_offset = 0x500 + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[base_offset..base_offset + 8].copy_from_slice(&value.to_le_bytes());
        let cached = if lane.offset == AIR_AGILITY_STAT {
            value + ((1_500_000_i64 - 1_000_000) * 50_000) / 100_000
        } else {
            value
        };
        bytes[lane.offset..lane.offset + 8].copy_from_slice(&cached.to_le_bytes());
    }
    bytes[BUILD_COST..BUILD_COST + 8].copy_from_slice(&2_400_000_i64.to_le_bytes());
    let variant = source(&bytes);

    let mut rows = Vec::new();
    let mut witness = RecordingWitness::new(Ok((1, 7)), Ok((1, 7)));
    let memory = PlaneMemory {
        bytes: &bytes,
        thrust_factor: Some(50_000),
        dlc_mask: Some(0x100),
    };
    assert_eq!(
        walk_stats_carried(&memory, 0, &[variant], &mut rows, &mut (), &mut witness),
        Outcome::Rows { len: 1 }
    );
    assert!(witness.withheld.is_empty(), "{:?}", witness.withheld);
    assert_eq!(rows[0].values[1], Some(1_200_000));
    assert_eq!(rows[0].values[3], Some(4_750_000));
    assert_eq!(rows[0].values[6], Some(1_000_000));
    assert_eq!(rows[0].values[9], Some(2_400_000));

    let mut stale = bytes.clone();
    stale[WEIGHT_STAT..WEIGHT_STAT + 8].copy_from_slice(&1_234_567_i64.to_le_bytes());
    stale[BUILD_COST..BUILD_COST + 8].copy_from_slice(&9_999_999_i64.to_le_bytes());
    let mut witness = RecordingWitness::new(Ok((1, 7)), Ok((1, 7)));
    assert_eq!(
        walk_stats_carried(
            &PlaneMemory {
                bytes: &stale,
                thrust_factor: Some(50_000),
                dlc_mask: Some(0x100),
            },
            0,
            &[variant],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[0], Some(100_000_000));
    assert_eq!(rows[0].values[6], None);
    assert_eq!(rows[0].values[9], None);
    assert!(witness.withheld.iter().any(|(lane, _)| *lane == "weight"));
    assert!(witness.withheld.iter().any(|(lane, _)| *lane == "build_cost_ic"));

    let mut modified = bytes.clone();
    modified[MODULES + 8 + 12..MODULES + 8 + 16].copy_from_slice(&1_u32.to_le_bytes());
    modified[AIR_DEFENCE_STAT..AIR_DEFENCE_STAT + 8].copy_from_slice(&50_000_i64.to_le_bytes());
    modified[MAXIMUM_SPEED_STAT..MAXIMUM_SPEED_STAT + 8].copy_from_slice(&(-1_i64).to_le_bytes());
    let mut witness = RecordingWitness::new(Ok((1, 7)), Ok((1, 7)));
    assert_eq!(
        walk_stats_carried(
            &PlaneMemory {
                bytes: &modified,
                thrust_factor: Some(50_000),
                dlc_mask: Some(0x100),
            },
            0,
            &[variant],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values, [None; STAT_LANE_COUNT]);
    assert!(witness.withheld.iter().any(|(lane, _)| *lane == "air_defence"));
    assert!(witness.withheld.iter().any(|(lane, _)| *lane == "maximum_speed"));

    let mut bad_def = bytes.clone();
    bad_def[0x500] ^= 1;
    let mut witness = RecordingWitness::new(Ok((1, 7)), Ok((1, 7)));
    assert_eq!(
        walk_stats_carried(
            &PlaneMemory {
                bytes: &bad_def,
                thrust_factor: Some(50_000),
                dlc_mask: Some(0x100),
            },
            0,
            &[variant],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values, [None; STAT_LANE_COUNT]);
    assert_eq!(rows[0].organisation.author, author);
    assert_eq!(witness.withheld.len(), STAT_LANE_COUNT);

    for (before, after) in [
        (Err("variant stat writer active"), Ok((1, 7))),
        (Ok((1, 7)), Ok((1, 8))),
    ] {
        let mut witness = RecordingWitness::new(before, after);
        assert_eq!(
            walk_stats_carried(&memory, 0, &[variant], &mut rows, &mut (), &mut witness),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].values, [None; STAT_LANE_COUNT]);
        assert_eq!(rows[0].organisation.author, author);
        assert_eq!(witness.withheld.len(), STAT_LANE_COUNT);
    }
}

#[test]
fn walk_stats_witness_enforces_modified_formula_bba_gate_token_partial_batch_and_no_taint() {
    struct World<'a> {
        bytes: &'a [u8],
        thrust_factor: Option<i64>,
        dlc_mask: Option<u32>,
        blocked: Option<usize>,
    }
    impl Memory for World<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.blocked.is_some_and(|at| {
                (address..address.saturating_add(output.len())).contains(&at)
            }) {
                return false;
            }
            if address == THRUST_WEIGHT_AGILITY_FACTOR_RVA.0 && output.len() == 8 {
                return self.thrust_factor.is_some_and(|value| {
                    output.copy_from_slice(&value.to_le_bytes());
                    true
                });
            }
            if address == engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4
            {
                return self.dlc_mask.is_some_and(|value| {
                    output.copy_from_slice(&value.to_le_bytes());
                    true
                });
            }
            let base = self.bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = self.bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }

    struct RecordingWitness {
        tokens: std::cell::Cell<
            (
                Result<(u64, u64), &'static str>,
                Result<(u64, u64), &'static str>,
            ),
        >,
        calls: std::cell::Cell<usize>,
        withheld: Vec<(&'static str, &'static str)>,
    }
    impl RecordingWitness {
        fn quiet() -> Self {
            Self {
                tokens: std::cell::Cell::new((Ok((1, 7)), Ok((1, 7)))),
                calls: std::cell::Cell::new(0),
                withheld: Vec::new(),
            }
        }
    }
    impl StatWitness for RecordingWitness {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            let (first, second) = self.tokens.get();
            let call = self.calls.get();
            self.calls.set(call + 1);
            (if call == 0 { first } else { second }).map(|(e, r)| QuietToken::new(e, r))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.withheld.push((STAT_LANES[lane].name, reason));
        }
    }

    let mut bytes = vec![0_u8; 0x3000];
    let base_ptr = bytes.as_ptr() as usize;
    let var0 = 0x0;
    let var1 = 0x600;
    let def_off = 0xc00;
    let mod_entry_off = 0x1200;
    let mod_obj_off = 0x1300;
    let mod_add_off = 0x1600;
    let author = (5_u64 << 32) | 412;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    let def_ptr = (base_ptr + def_off) as u64;
    bytes[def_off..def_off + 8].copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
        .copy_from_slice(&77_u32.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
    let init_variant = |buf: &mut [u8], off: usize, id: u32, agility: i64| {
        buf[off..off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        buf[off + TYPE..off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        buf[off + ID..off + ID + 4].copy_from_slice(&id.to_le_bytes());
        buf[off + EQUIPMENT_TOKEN..off + EQUIPMENT_TOKEN + 4].copy_from_slice(&77_u32.to_le_bytes());
        buf[off + DEFINITION..off + DEFINITION + 8].copy_from_slice(&def_ptr.to_le_bytes());
        buf[off + INDUSTRIAL_ORGANISATION..off + INDUSTRIAL_ORGANISATION + 8]
            .copy_from_slice(&author.to_le_bytes());
        buf[off + CLASSIFIER_FLAGS..off + CLASSIFIER_FLAGS + 8]
            .copy_from_slice(&0x400_u64.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
            let v = if lane.offset == AIR_AGILITY_STAT {
                agility
            } else {
                value
            };
            buf[off + lane.offset..off + lane.offset + 8].copy_from_slice(&v.to_le_bytes());
        }
        buf[off + BUILD_COST..off + BUILD_COST + 8].copy_from_slice(&2_400_000_i64.to_le_bytes());
    };
    init_variant(&mut bytes, var0, 1, 4_500_000);
    init_variant(&mut bytes, var1, 2, 4_750_000);

    let v0 = RegistryRow {
        id: (1_u64 << 32) | u64::from(REFERENCE_TYPE),
        address: base_ptr + var0,
    };
    let v1 = RegistryRow {
        id: (2_u64 << 32) | u64::from(REFERENCE_TYPE),
        address: base_ptr + var1,
    };

    let mut rows = Vec::new();
    let mut witness = RecordingWitness::quiet();
    let world = World {
        bytes: &bytes,
        thrust_factor: Some(50_000),
        dlc_mask: Some(0x100),
        blocked: None,
    };
    assert_eq!(
        walk_stats_carried(&world, 0, &[v0, v1], &mut rows, &mut (), &mut witness),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].values[3], None);
    assert_eq!(rows[1].values[3], Some(4_750_000));

    let mut witness = RecordingWitness::quiet();
    let no_bba = World {
        bytes: &bytes,
        thrust_factor: Some(50_000),
        dlc_mask: Some(0),
        blocked: None,
    };
    assert_eq!(
        walk_stats_carried(&no_bba, 0, &[v0, v1], &mut rows, &mut (), &mut witness),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].values[3], Some(4_500_000));
    assert_eq!(rows[1].values[3], None);

    let mut bad_token = bytes.clone();
    let bt_base = bad_token.as_ptr() as usize;
    bad_token[var1 + DEFINITION..var1 + DEFINITION + 8]
        .copy_from_slice(&((bt_base + def_off) as u64).to_le_bytes());
    bad_token[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
        .copy_from_slice(&99_u32.to_le_bytes());
    let bt_v1 = RegistryRow {
        id: v1.id,
        address: bt_base + var1,
    };
    let mut witness = RecordingWitness::quiet();
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &bad_token,
                thrust_factor: Some(50_000),
                dlc_mask: Some(0x100),
                blocked: None,
            },
            0,
            &[bt_v1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values, [None; STAT_LANE_COUNT]);

    let mut witness = RecordingWitness::quiet();
    let partial = World {
        bytes: &bytes,
        thrust_factor: Some(50_000),
        dlc_mask: Some(0x100),
        blocked: Some(base_ptr + var0 + MAXIMUM_SPEED_STAT),
    };
    assert_eq!(
        walk_stats_carried(&partial, 0, &[v0, v1], &mut rows, &mut (), &mut witness),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].values[0], Some(100_000_000));
    assert_eq!(rows[0].values[4], None);
    assert_eq!(rows[0].organisation.author, author);
    assert_eq!(rows[1].values[4], Some(42_000_000));
    assert_eq!(rows[1].organisation.author, author);

    let mut modified = bytes.clone();
    let m_base = modified.as_ptr() as usize;
    let m_def_ptr = (m_base + def_off) as u64;
    modified[var1 + DEFINITION..var1 + DEFINITION + 8].copy_from_slice(&m_def_ptr.to_le_bytes());
    modified[var1 + MODULES + 8..var1 + MODULES + 16]
        .copy_from_slice(&((m_base + mod_entry_off) as u64).to_le_bytes());
    modified[var1 + MODULES + 16..var1 + MODULES + 20].copy_from_slice(&1_u32.to_le_bytes());
    modified[var1 + MODULES + 20..var1 + MODULES + 24].copy_from_slice(&1_u32.to_le_bytes());
    modified[mod_entry_off + 8..mod_entry_off + 16]
        .copy_from_slice(&((m_base + mod_obj_off) as u64).to_le_bytes());
    modified[mod_obj_off..mod_obj_off + 8]
        .copy_from_slice(&(engine_native_parts::cequipment_module::VTABLE_RVA.0 as u64).to_le_bytes());
    modified[mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS
        ..mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 8]
        .copy_from_slice(&((m_base + mod_add_off) as u64).to_le_bytes());
    modified[mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 8
        ..mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 12]
        .copy_from_slice(&1_u32.to_le_bytes());
    modified[mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 12
        ..mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 16]
        .copy_from_slice(&1_u32.to_le_bytes());
    modified[mod_add_off..mod_add_off + 4].copy_from_slice(&62_i32.to_le_bytes());
    modified[mod_add_off + 8..mod_add_off + 16].copy_from_slice(&5_000_000_i64.to_le_bytes());
    modified[var1 + MAXIMUM_SPEED_STAT..var1 + MAXIMUM_SPEED_STAT + 8]
        .copy_from_slice(&999_999_i64.to_le_bytes());
    let mv1 = RegistryRow {
        id: v1.id,
        address: m_base + var1,
    };
    let mut witness = RecordingWitness::quiet();
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &modified,
                thrust_factor: Some(50_000),
                dlc_mask: Some(0x100),
                blocked: None,
            },
            0,
            &[mv1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[4], None);
    modified[var1 + MAXIMUM_SPEED_STAT..var1 + MAXIMUM_SPEED_STAT + 8]
        .copy_from_slice(&47_000_000_i64.to_le_bytes());
    let mut witness = RecordingWitness::quiet();
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &modified,
                thrust_factor: Some(50_000),
                dlc_mask: Some(0x100),
                blocked: None,
            },
            0,
            &[mv1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[4], Some(47_000_000));

    let _ = engine_sites::take_taint();
    let overflow_base = usize::MAX - 0x0330_0000;
    let mut no_taint_bytes = bytes.clone();
    let nt_base = no_taint_bytes.as_ptr() as usize;
    let nt_def_ptr = (nt_base + def_off) as u64;
    no_taint_bytes[var1..var1 + 8]
        .copy_from_slice(&(overflow_base.wrapping_add(VTABLE_RVA.0) as u64).to_le_bytes());
    no_taint_bytes[var1 + DEFINITION..var1 + DEFINITION + 8]
        .copy_from_slice(&nt_def_ptr.to_le_bytes());
    no_taint_bytes[def_off..def_off + 8].copy_from_slice(
        &(overflow_base.wrapping_add(cequipment_type::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    let nt_v1 = RegistryRow {
        id: v1.id,
        address: nt_base + var1,
    };
    let mut witness = RecordingWitness::quiet();
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &no_taint_bytes,
                thrust_factor: None,
                dlc_mask: None,
                blocked: None,
            },
            overflow_base,
            &[nt_v1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[3], None);
    assert_eq!(rows[0].values[0], Some(100_000_000));
    assert_eq!(engine_sites::take_taint(), None);
}

#[test]
fn organisation_bonuses_multiply_the_lane_their_stat_token_names() {
    struct World<'a> {
        bytes: &'a [u8],
        lexer: Option<[u8; 64]>,
    }
    impl Memory for World<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let block = engine_native_memory::cpdx_lexer::INITIALIZED_RVA.0;
            if let Some(header) = &self.lexer
                && let Some(slice) = address.checked_sub(block).and_then(|at| header.get(at..at + output.len()))
            {
                output.copy_from_slice(slice);
                return true;
            }
            let Some(slice) = address
                .checked_sub(self.bytes.as_ptr() as usize)
                .and_then(|at| self.bytes.get(at..at.saturating_add(output.len())))
            else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }
    struct Quiet(Vec<(&'static str, &'static str)>);
    impl StatWitness for Quiet {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            Ok(QuietToken::new(1, 7))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.0.push((STAT_LANES[lane].name, reason));
        }
    }

    let mut bytes = vec![0_u8; 0x2000];
    let base_ptr = bytes.as_ptr() as usize;
    let (variant, def_off, bonuses_off, names_off) = (0x0, 0xc00, 0x1200, 0x1400);
    let base_values = [0_i64, 100_000, 0, 100_000, 400_000, 80_000, 0, 0, 0, 2_400_000];
    bytes[def_off..def_off + 8].copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4].copy_from_slice(&77_u32.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[variant..variant + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[variant + TYPE..variant + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
    bytes[variant + ID..variant + ID + 4].copy_from_slice(&1_u32.to_le_bytes());
    bytes[variant + EQUIPMENT_TOKEN..variant + EQUIPMENT_TOKEN + 4].copy_from_slice(&77_u32.to_le_bytes());
    bytes[variant + DEFINITION..variant + DEFINITION + 8].copy_from_slice(&((base_ptr + def_off) as u64).to_le_bytes());
    let mut cached = base_values;
    cached[5] = 84_000;
    cached[9] = 2_280_000;
    for (lane, value) in STAT_LANES.into_iter().zip(cached) {
        bytes[variant + lane.offset..variant + lane.offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[variant + BUILD_COST..variant + BUILD_COST + 8].copy_from_slice(&2_280_000_i64.to_le_bytes());
    let bonuses = [(3_u32, -5_000_i64), (4, 5_000), (5, 25_000), (6, 0)];
    bytes[variant + ORGANISATION_BONUSES..variant + ORGANISATION_BONUSES + 8]
        .copy_from_slice(&((base_ptr + bonuses_off) as u64).to_le_bytes());
    for offset in [8, 12] {
        bytes[variant + ORGANISATION_BONUSES + offset..variant + ORGANISATION_BONUSES + offset + 4]
            .copy_from_slice(&(bonuses.len() as u32).to_le_bytes());
    }
    for (index, (token, value)) in bonuses.into_iter().enumerate() {
        let at = bonuses_off + index * 16;
        bytes[at..at + 4].copy_from_slice(&token.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
    }
    let names = ["", "", "", "build_cost_ic", "reliability", "soft_attack", "maximum_speed"];
    for (token, name) in names.into_iter().enumerate() {
        let at = names_off + token * engine_native_memory::cpdx_lexer::TOKEN_NAME_STRIDE;
        bytes[at..at + name.len()].copy_from_slice(name.as_bytes());
        bytes[at + 16..at + 24].copy_from_slice(&(name.len() as u64).to_le_bytes());
        bytes[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
    }
    let mut lexer = [0_u8; 64];
    lexer[0] = 1;
    let mut put = |key: Rva, value: &[u8]| {
        let at = engine_native_memory::cpdx_lexer::witness_offset(key);
        lexer[at..at + value.len()].copy_from_slice(value);
    };
    let maximum = (names.len() - 1) as u32;
    put(engine_native_memory::cpdx_lexer::MAX_TOKEN_ID_RVA, &maximum.to_le_bytes());
    put(engine_native_memory::cpdx_lexer::STATIC_MAX_TOKEN_ID_RVA, &maximum.to_le_bytes());
    put(engine_native_memory::cpdx_lexer::TOKEN_NAMES_RVA, &((base_ptr + names_off) as u64).to_le_bytes());
    put(engine_native_memory::cpdx_lexer::TOKEN_ALLOCATION_RVA, &(names.len() as u32).to_le_bytes());
    put(engine_native_memory::cpdx_lexer::TOKEN_CAPACITY_RVA, &(names.len() as u32).to_le_bytes());
    let row = RegistryRow {
        id: (1_u64 << 32) | u64::from(REFERENCE_TYPE),
        address: base_ptr + variant,
    };
    let walk = |bytes: &[u8], lexer: Option<[u8; 64]>| {
        let (mut rows, mut witness) = (Vec::new(), Quiet(Vec::new()));
        let outcome = walk_stats_carried(&World { bytes, lexer }, 0, &[row], &mut rows, &mut (), &mut witness);
        assert_eq!(outcome, Outcome::Rows { len: 1 });
        (rows[0].values, witness.0)
    };

    let (values, withheld) = walk(&bytes, Some(lexer));
    assert_eq!(values, cached.map(Some), "every lane is published once its bonuses are applied: {withheld:?}");

    let cost = variant + BUILD_COST_IC_STAT..variant + BUILD_COST_IC_STAT + 8;
    bytes[cost.clone()].copy_from_slice(&2_400_000_i64.to_le_bytes());
    let (values, _) = walk(&bytes, Some(lexer));
    assert_eq!(values[9], None, "a cached cost that disagrees with its bonuses stays withheld");
    assert_eq!(values[5], Some(84_000));
    bytes[cost.clone()].copy_from_slice(&2_280_000_i64.to_le_bytes());

    let (values, withheld) = walk(&bytes, None);
    assert_eq!(values, [None; STAT_LANE_COUNT]);
    assert_eq!(withheld.len(), STAT_LANE_COUNT);
    assert!(withheld.iter().all(|(_, reason)| *reason == "variant stat organisation bonus names unreadable"));

    let (entries_off, module_off, module_stats_off) = (0x1600, 0x1700, 0x1800);
    let module_add = engine_native_parts::cequipment_module::ADD_STATS;
    bytes[variant + MODULES + 8..variant + MODULES + 16].copy_from_slice(&((base_ptr + entries_off) as u64).to_le_bytes());
    for offset in [16, 20] {
        bytes[variant + MODULES + offset..variant + MODULES + offset + 4].copy_from_slice(&1_u32.to_le_bytes());
    }
    bytes[entries_off + 8..entries_off + 16].copy_from_slice(&((base_ptr + module_off) as u64).to_le_bytes());
    bytes[module_off..module_off + 8].copy_from_slice(&(engine_native_parts::cequipment_module::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[module_off + module_add..module_off + module_add + 8].copy_from_slice(&((base_ptr + module_stats_off) as u64).to_le_bytes());
    for offset in [8, 12] {
        bytes[module_off + module_add + offset..module_off + module_add + offset + 4].copy_from_slice(&1_u32.to_le_bytes());
    }
    bytes[module_stats_off..module_stats_off + 4].copy_from_slice(&77_i32.to_le_bytes());
    bytes[module_stats_off + 8..module_stats_off + 16].copy_from_slice(&1_000_000_i64.to_le_bytes());
    bytes[cost].copy_from_slice(&3_230_000_i64.to_le_bytes());
    let (values, withheld) = walk(&bytes, Some(lexer));
    assert_eq!(values[9], Some(3_230_000), "a module-built design's cost carries its organisation bonus: {withheld:?}");
    assert_eq!(values[5], Some(84_000));
}

#[test]
fn walk_stats_witness_checks_timing_after_module_reads_withholds_ambiguous_reliability_and_recovers_independent_operands()
{
    struct SteppingWorld<'a> {
        bytes: &'a [u8],
        trigger_at: Option<usize>,
        blocked: Option<usize>,
        revision: &'a Cell<u64>,
    }
    impl Memory for SteppingWorld<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .blocked
                .is_some_and(|at| (address..address.saturating_add(output.len())).contains(&at))
            {
                return false;
            }
            if self
                .trigger_at
                .is_some_and(|at| (address..address.saturating_add(output.len())).contains(&at))
            {
                self.revision.set(self.revision.get() + 1);
            }
            if address == THRUST_WEIGHT_AGILITY_FACTOR_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&50_000_i64.to_le_bytes());
                return true;
            }
            if address == engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4
            {
                output.copy_from_slice(&0x100_u32.to_le_bytes());
                return true;
            }
            let base = self.bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = self.bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }

    struct LiveRevisionWitness<'a> {
        revision: &'a Cell<u64>,
        withheld: Vec<(&'static str, &'static str)>,
    }
    impl StatWitness for LiveRevisionWitness<'_> {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            Ok(QuietToken::new(1, self.revision.get()))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.withheld.push((STAT_LANES[lane].name, reason));
        }
    }

    let mut bytes = vec![0_u8; 0x2000];
    let base_ptr = bytes.as_ptr() as usize;
    let var_off = 0x0;
    let def_off = 0x600;
    let mod_entry_off = 0xc00;
    let mod_obj_off = 0xd00;
    let mod_add_off = 0x1000;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    bytes[def_off..def_off + 8]
        .copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
        .copy_from_slice(&77_u32.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[var_off..var_off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[var_off + TYPE..var_off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
    bytes[var_off + ID..var_off + ID + 4].copy_from_slice(&1_u32.to_le_bytes());
    bytes[var_off + EQUIPMENT_TOKEN..var_off + EQUIPMENT_TOKEN + 4]
        .copy_from_slice(&77_u32.to_le_bytes());
    bytes[var_off + DEFINITION..var_off + DEFINITION + 8]
        .copy_from_slice(&((base_ptr + def_off) as u64).to_le_bytes());
    bytes[var_off + CLASSIFIER_FLAGS..var_off + CLASSIFIER_FLAGS + 8]
        .copy_from_slice(&0x400_u64.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let v = if lane.offset == AIR_AGILITY_STAT {
            4_750_000
        } else {
            value
        };
        bytes[var_off + lane.offset..var_off + lane.offset + 8].copy_from_slice(&v.to_le_bytes());
    }
    bytes[var_off + BUILD_COST..var_off + BUILD_COST + 8]
        .copy_from_slice(&2_400_000_i64.to_le_bytes());

    let mut modified = bytes.clone();
    let m_base = modified.as_ptr() as usize;
    modified[var_off + DEFINITION..var_off + DEFINITION + 8]
        .copy_from_slice(&((m_base + def_off) as u64).to_le_bytes());
    modified[var_off + MODULES + 8..var_off + MODULES + 16]
        .copy_from_slice(&((m_base + mod_entry_off) as u64).to_le_bytes());
    modified[var_off + MODULES + 16..var_off + MODULES + 20].copy_from_slice(&1_u32.to_le_bytes());
    modified[var_off + MODULES + 20..var_off + MODULES + 24].copy_from_slice(&1_u32.to_le_bytes());
    modified[mod_entry_off + 8..mod_entry_off + 16]
        .copy_from_slice(&((m_base + mod_obj_off) as u64).to_le_bytes());
    modified[mod_obj_off..mod_obj_off + 8]
        .copy_from_slice(&(engine_native_parts::cequipment_module::VTABLE_RVA.0 as u64).to_le_bytes());
    modified[mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS
        ..mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 8]
        .copy_from_slice(&((m_base + mod_add_off) as u64).to_le_bytes());
    modified[mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 8
        ..mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 12]
        .copy_from_slice(&1_u32.to_le_bytes());
    modified[mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 12
        ..mod_obj_off + engine_native_parts::cequipment_module::ADD_STATS + 16]
        .copy_from_slice(&1_u32.to_le_bytes());
    modified[mod_add_off..mod_add_off + 4].copy_from_slice(&62_i32.to_le_bytes());
    modified[mod_add_off + 8..mod_add_off + 16].copy_from_slice(&5_000_000_i64.to_le_bytes());
    modified[var_off + MAXIMUM_SPEED_STAT..var_off + MAXIMUM_SPEED_STAT + 8]
        .copy_from_slice(&47_000_000_i64.to_le_bytes());

    let rev = Cell::new(7_u64);
    let mut rows = Vec::new();
    let mut witness = LiveRevisionWitness {
        revision: &rev,
        withheld: Vec::new(),
    };
    let mv = RegistryRow {
        id: (1_u64 << 32) | u64::from(REFERENCE_TYPE),
        address: m_base + var_off,
    };
    assert_eq!(
        walk_stats_carried(
            &SteppingWorld {
                bytes: &modified,
                trigger_at: Some(m_base + mod_add_off),
                blocked: None,
                revision: &rev,
            },
            0,
            &[mv],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values, [None; STAT_LANE_COUNT]);
    assert!(
        witness
            .withheld
            .iter()
            .any(|(_, reason)| *reason == "variant stat writer crossed read")
    );

    for cached_rel in [100_000_i64, 150_000_i64] {
        let mut high_rel = bytes.clone();
        let hr_base = high_rel.as_ptr() as usize;
        high_rel[var_off + DEFINITION..var_off + DEFINITION + 8]
            .copy_from_slice(&((hr_base + def_off) as u64).to_le_bytes());
        high_rel[def_off + cequipment_type::STATS_BASE + 65 * 8
            ..def_off + cequipment_type::STATS_BASE + 65 * 8 + 8]
            .copy_from_slice(&150_000_i64.to_le_bytes());
        high_rel[var_off + RELIABILITY_STAT..var_off + RELIABILITY_STAT + 8]
            .copy_from_slice(&cached_rel.to_le_bytes());
        let hv = RegistryRow {
            id: mv.id,
            address: hr_base + var_off,
        };
        let mut witness = LiveRevisionWitness {
            revision: &rev,
            withheld: Vec::new(),
        };
        assert_eq!(
            walk_stats_carried(
                &SteppingWorld {
                    bytes: &high_rel,
                    trigger_at: None,
                    blocked: None,
                    revision: &rev,
                },
                0,
                &[hv],
                &mut rows,
                &mut (),
                &mut witness,
            ),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].values[5], None);
        assert_eq!(rows[0].values[4], Some(42_000_000));
        assert!(
            witness
                .withheld
                .iter()
                .any(|(lane, reason)| *lane == "reliability"
                    && *reason == "variant stat reliability rebuild cap mode ambiguous")
        );
    }

    let v = RegistryRow {
        id: mv.id,
        address: base_ptr + var_off,
    };
    let mut witness = LiveRevisionWitness {
        revision: &rev,
        withheld: Vec::new(),
    };
    assert_eq!(
        walk_stats_carried(
            &SteppingWorld {
                bytes: &bytes,
                trigger_at: None,
                blocked: Some(base_ptr + def_off + cequipment_type::STATS_BASE + 46 * 8),
                revision: &rev,
            },
            0,
            &[v],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[0], None);
    assert_eq!(rows[0].values[4], Some(42_000_000));
    assert_eq!(rows[0].values[9], Some(2_400_000));

    let mut witness = LiveRevisionWitness {
        revision: &rev,
        withheld: Vec::new(),
    };
    assert_eq!(
        walk_stats_carried(
            &SteppingWorld {
                bytes: &bytes,
                trigger_at: None,
                blocked: Some(base_ptr + var_off + BUILD_COST),
                revision: &rev,
            },
            0,
            &[v],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[9], None);
    assert_eq!(rows[0].values[0], Some(100_000_000));
    assert_eq!(rows[0].values[4], Some(42_000_000));
}

#[test]
fn walk_stats_carried_refreshes_thrust_factor_after_rejected_batch() {
    use std::cell::Cell;

    struct BatchFactorWorld<'a> {
        bytes: &'a [u8],
        factor: Cell<i64>,
        revision: &'a Cell<u64>,
        bounced: Cell<bool>,
    }

    impl Memory for BatchFactorWorld<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == THRUST_WEIGHT_AGILITY_FACTOR_RVA.0 && output.len() == 8 {
                let current = self.factor.get();
                output.copy_from_slice(&current.to_le_bytes());
                if !self.bounced.replace(true) {
                    self.revision.set(self.revision.get() + 1);
                    self.factor.set(80_000);
                }
                return true;
            }
            if address == engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4
            {
                output.copy_from_slice(&BY_BLOOD_ALONE_MASK.to_le_bytes());
                return true;
            }
            let base = self.bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = self.bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }

    struct RevWitness<'a>(&'a Cell<u64>);
    impl StatWitness for RevWitness<'_> {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            Ok(QuietToken::new(1, self.0.get()))
        }
    }

    const COUNT: usize = 65;
    const VAR_STRIDE: usize = 0x500;
    let def_off = COUNT * VAR_STRIDE;
    let mut bytes = vec![0_u8; def_off + 0x600];
    let base_ptr = bytes.as_ptr() as usize;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    bytes[def_off..def_off + 8]
        .copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
        .copy_from_slice(&77_u32.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
    for index in 0..COUNT {
        let var_off = index * VAR_STRIDE;
        bytes[var_off..var_off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[var_off + TYPE..var_off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        bytes[var_off + ID..var_off + ID + 4].copy_from_slice(&((index as u32) + 1).to_le_bytes());
        bytes[var_off + EQUIPMENT_TOKEN..var_off + EQUIPMENT_TOKEN + 4]
            .copy_from_slice(&77_u32.to_le_bytes());
        bytes[var_off + DEFINITION..var_off + DEFINITION + 8]
            .copy_from_slice(&((base_ptr + def_off) as u64).to_le_bytes());
        bytes[var_off + CLASSIFIER_FLAGS..var_off + CLASSIFIER_FLAGS + 8]
            .copy_from_slice(&0x400_u64.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
            let v = if lane.offset == AIR_AGILITY_STAT {
                4_750_000
            } else {
                value
            };
            bytes[var_off + lane.offset..var_off + lane.offset + 8]
                .copy_from_slice(&v.to_le_bytes());
        }
        bytes[var_off + BUILD_COST..var_off + BUILD_COST + 8]
            .copy_from_slice(&2_400_000_i64.to_le_bytes());
    }

    let rev = Cell::new(1_u64);
    let world = BatchFactorWorld {
        bytes: &bytes,
        factor: Cell::new(50_000),
        revision: &rev,
        bounced: Cell::new(false),
    };
    let variants: Vec<RegistryRow> = (0..COUNT)
        .map(|index| RegistryRow::read(&world, 0, base_ptr + index * VAR_STRIDE).unwrap())
        .collect();
    let mut rows = Vec::new();
    let mut witness = RevWitness(&rev);
    assert_eq!(
        walk_stats_carried(&world, 0, &variants, &mut rows, &mut (), &mut witness),
        Outcome::Rows { len: COUNT }
    );
    assert_eq!(rows[0].values, [None; STAT_LANE_COUNT]);
    assert_eq!(rows[64].variant, variants[64].id);
    assert_eq!(rows[64].values[3], None);
    assert_eq!(rows[64].values[4], Some(42_000_000));
}

#[test]
fn partial_read_keeps_unrelated_lanes_when_reliability_bonus_or_disabled_bba_operands_are_unreadable()
{
    struct PartialWorld<'a> {
        bytes: &'a [u8],
        dlc_mask: u32,
        blocked: Option<usize>,
    }

    impl Memory for PartialWorld<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .blocked
                .is_some_and(|at| (address..address.saturating_add(output.len())).contains(&at))
            {
                return false;
            }
            if address == engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0 && output.len() == 4
            {
                output.copy_from_slice(&self.dlc_mask.to_le_bytes());
                return true;
            }
            let base = self.bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = self.bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }

    let mut bytes = vec![0_u8; 0x1800];
    let base_ptr = bytes.as_ptr() as usize;
    let var_off = 0x0;
    let def_off = 0x600;
    let rel_bonus_off = 0x1000;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    bytes[def_off..def_off + 8]
        .copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
        .copy_from_slice(&77_u32.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
        bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[var_off..var_off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[var_off + TYPE..var_off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
    bytes[var_off + ID..var_off + ID + 4].copy_from_slice(&1_u32.to_le_bytes());
    bytes[var_off + EQUIPMENT_TOKEN..var_off + EQUIPMENT_TOKEN + 4]
        .copy_from_slice(&77_u32.to_le_bytes());
    bytes[var_off + DEFINITION..var_off + DEFINITION + 8]
        .copy_from_slice(&((base_ptr + def_off) as u64).to_le_bytes());
    bytes[var_off + CLASSIFIER_FLAGS..var_off + CLASSIFIER_FLAGS + 8]
        .copy_from_slice(&0x400_u64.to_le_bytes());
    for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
        bytes[var_off + lane.offset..var_off + lane.offset + 8]
            .copy_from_slice(&value.to_le_bytes());
    }
    bytes[var_off + BUILD_COST..var_off + BUILD_COST + 8]
        .copy_from_slice(&2_400_000_i64.to_le_bytes());

    let variant = RegistryRow {
        id: (1_u64 << 32) | u64::from(REFERENCE_TYPE),
        address: base_ptr + var_off,
    };
    let mut rows = Vec::new();
    assert_eq!(
        walk_stats(
            &PartialWorld {
                bytes: &bytes,
                dlc_mask: 0,
                blocked: Some(base_ptr + def_off + cequipment_type::STATS_BASE + 67 * 8),
            },
            0,
            &[variant],
            &mut rows,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[3], Some(4_500_000));
    assert_eq!(rows[0].values[6], None);

    bytes[def_off + cequipment_type::RELIABILITY_BONUSES
        ..def_off + cequipment_type::RELIABILITY_BONUSES + 8]
        .copy_from_slice(&((base_ptr + rel_bonus_off) as u64).to_le_bytes());
    bytes[def_off + cequipment_type::RELIABILITY_BONUSES + 8
        ..def_off + cequipment_type::RELIABILITY_BONUSES + 12]
        .copy_from_slice(&1_u32.to_le_bytes());
    bytes[def_off + cequipment_type::RELIABILITY_BONUS_COUNT
        ..def_off + cequipment_type::RELIABILITY_BONUS_COUNT + 4]
        .copy_from_slice(&1_u32.to_le_bytes());
    bytes[rel_bonus_off..rel_bonus_off + 4].copy_from_slice(&48_i32.to_le_bytes());
    bytes[rel_bonus_off + 8..rel_bonus_off + 16].copy_from_slice(&200_000_i64.to_le_bytes());

    assert_eq!(
        walk_stats(
            &PartialWorld {
                bytes: &bytes,
                dlc_mask: 0,
                blocked: Some(base_ptr + def_off + cequipment_type::STATS_BASE + 65 * 8),
            },
            0,
            &[variant],
            &mut rows,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].values[2], None);
    assert_eq!(rows[0].values[5], None);
    assert_eq!(rows[0].values[4], Some(42_000_000));
    assert_eq!(rows[0].values[9], Some(2_400_000));
}

#[test]
fn walk_stats_carried_withholds_replaced_design_after_registry_read_without_affecting_neighbor() {
    struct World<'a> {
        bytes: &'a [u8],
        blocked: Option<usize>,
    }

    impl Memory for World<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .blocked
                .is_some_and(|at| (address..address.saturating_add(output.len())).contains(&at))
            {
                return false;
            }
            let base = self.bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = self.bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }

    struct RecordingWitness {
        withheld: Vec<(&'static str, &'static str)>,
    }

    impl StatWitness for RecordingWitness {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            Ok(QuietToken::new(1, 7))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.withheld.push((STAT_LANES[lane].name, reason));
        }
    }

    let mut bytes = vec![0_u8; 0x2000];
    let base_ptr = bytes.as_ptr() as usize;
    let var0 = 0x0;
    let var1 = 0x600;
    let def0 = 0xc00;
    let def1 = 0x1200;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    let replacement_values = [
        200_000_000_i64,
        2_200_000,
        2_800_000,
        5_500_000,
        55_000_000,
        90_000,
        1_200_000,
        1_200_000,
        31_000,
        3_400_000,
    ];
    for (def_off, token, values) in [(def0, 77_u32, base_values), (def1, 88_u32, replacement_values)]
    {
        bytes[def_off..def_off + 8]
            .copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
            .copy_from_slice(&token.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(values) {
            let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
            bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    let orig_author = (11_u64 << 32) | 79;
    let neighbor_author = (22_u64 << 32) | 79;
    let replacement_author = (99_u64 << 32) | 79;
    let init_variant = |buf: &mut [u8],
                        off: usize,
                        id: u32,
                        token: u32,
                        def_off: usize,
                        author: u64,
                        trait_count: i32,
                        values: [i64; 10]| {
        buf[off..off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        buf[off + TYPE..off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        buf[off + ID..off + ID + 4].copy_from_slice(&id.to_le_bytes());
        buf[off + EQUIPMENT_TOKEN..off + EQUIPMENT_TOKEN + 4].copy_from_slice(&token.to_le_bytes());
        buf[off + DEFINITION..off + DEFINITION + 8]
            .copy_from_slice(&((base_ptr + def_off) as u64).to_le_bytes());
        buf[off + INDUSTRIAL_ORGANISATION..off + INDUSTRIAL_ORGANISATION + 8]
            .copy_from_slice(&author.to_le_bytes());
        buf[off + ORGANISATION_TRAIT_COUNT..off + ORGANISATION_TRAIT_COUNT + 4]
            .copy_from_slice(&trait_count.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(values) {
            buf[off + lane.offset..off + lane.offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        buf[off + BUILD_COST..off + BUILD_COST + 8].copy_from_slice(&values[9].to_le_bytes());
    };
    init_variant(&mut bytes, var0, 1, 77, def0, orig_author, 1, base_values);
    init_variant(
        &mut bytes,
        var1,
        2,
        77,
        def0,
        neighbor_author,
        2,
        base_values,
    );

    let initial = World {
        bytes: &bytes,
        blocked: None,
    };
    let v0 = RegistryRow::read(&initial, 0, base_ptr + var0).unwrap();
    let v1 = RegistryRow::read(&initial, 0, base_ptr + var1).unwrap();

    init_variant(
        &mut bytes,
        var0,
        99,
        88,
        def1,
        replacement_author,
        7,
        replacement_values,
    );

    let mut rows = Vec::new();
    let mut witness = RecordingWitness {
        withheld: Vec::new(),
    };
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &bytes,
                blocked: None,
            },
            0,
            &[v0, v1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert!(witness
        .withheld
        .iter()
        .any(|(_, reason)| *reason == "variant identity changed before stat read"));
    assert!(rows[..1].iter().all(|row| row.variant != v0.id));
    assert!(rows[..1]
        .iter()
        .all(|row| row.organisation.author != replacement_author));
    assert_eq!(rows[0].variant, v1.id);
    assert_eq!(rows[0].organisation.author, neighbor_author);
    assert_eq!(rows[0].values[4], Some(42_000_000));
    assert_eq!(rows[0].values[9], Some(2_400_000));

    let mut witness = RecordingWitness {
        withheld: Vec::new(),
    };
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &bytes,
                blocked: Some(base_ptr + var1 + AIR_RANGE_STAT),
            },
            0,
            &[v0, v1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert!(witness
        .withheld
        .iter()
        .any(|(_, reason)| *reason == "variant identity changed before stat read"));
    assert_eq!(rows[0].variant, v1.id);
    assert_eq!(rows[0].organisation.author, neighbor_author);
    assert_eq!(rows[0].values[0], None);
    assert_eq!(rows[0].values[4], Some(42_000_000));
    assert_eq!(rows[0].values[9], Some(2_400_000));

    let mut witness = RecordingWitness {
        withheld: Vec::new(),
    };
    assert_eq!(
        walk_stats_carried(
            &World {
                bytes: &bytes,
                blocked: Some(base_ptr + var0 + INDUSTRIAL_ORGANISATION),
            },
            0,
            &[v0, v1],
            &mut rows,
            &mut (),
            &mut witness,
        ),
        Outcome::Rows { len: 1 }
    );
    assert!(witness
        .withheld
        .iter()
        .any(|(_, reason)| *reason == "variant identity changed before stat read"));
    assert_eq!(rows[0].variant, v1.id);
    assert_eq!(rows[0].organisation.author, neighbor_author);
    assert_eq!(rows[0].values[4], Some(42_000_000));
    assert_eq!(rows[0].values[9], Some(2_400_000));
}

#[test]
fn walk_stats_carried_withholds_design_replaced_before_fresh_organisation_read() {
    use std::cell::RefCell;

    struct LiveWorld<'a> {
        bytes: &'a RefCell<Vec<u8>>,
        blocked: &'a Cell<Option<usize>>,
    }

    impl Memory for LiveWorld<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .blocked
                .get()
                .is_some_and(|at| (address..address.saturating_add(output.len())).contains(&at))
            {
                return false;
            }
            let bytes = self.bytes.borrow();
            let base = bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            true
        }
    }

    struct RecordingWitness {
        withheld: Vec<(&'static str, &'static str)>,
    }

    impl StatWitness for RecordingWitness {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            Ok(QuietToken::new(1, 7))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.withheld.push((STAT_LANES[lane].name, reason));
        }
    }

    struct SwitchingCarry<'a, F: FnMut()> {
        on_first_fresh: Option<F>,
        certified: &'a mut Vec<u64>,
    }

    impl<F: FnMut()> OrganisationCarry for SwitchingCarry<'_, F> {
        fn fresh(&mut self, _: usize, _: &RegistryRow) -> Result<bool, &'static str> {
            if let Some(mut trigger) = self.on_first_fresh.take() {
                trigger();
            }
            Ok(true)
        }
        fn read(&mut self, _: usize, source: &RegistryRow) {
            self.certified.push(source.id);
        }
    }

    let bytes = RefCell::new(vec![0_u8; 0x2400]);
    let base_ptr = bytes.borrow().as_ptr() as usize;
    let var0 = 0x0;
    let var1 = 0x600;
    let def0 = 0xc00;
    let def1 = 0x1200;
    let bonus_off = 0x1800;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    let replacement_values = [
        200_000_000_i64,
        2_200_000,
        2_800_000,
        5_500_000,
        55_000_000,
        90_000,
        1_200_000,
        1_200_000,
        31_000,
        3_400_000,
    ];
    {
        let mut buf = bytes.borrow_mut();
        for (def_off, token, values) in [
            (def0, 77_u32, base_values),
            (def1, 88_u32, replacement_values),
        ] {
            buf[def_off..def_off + 8]
                .copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
            buf[def_off + cequipment_type::TOKEN..def_off + cequipment_type::TOKEN + 4]
                .copy_from_slice(&token.to_le_bytes());
            for (lane, value) in STAT_LANES.into_iter().zip(values) {
                let off = def_off + cequipment_type::STATS_BASE + lane.stat_index * 8;
                buf[off..off + 8].copy_from_slice(&value.to_le_bytes());
            }
        }
        buf[bonus_off..bonus_off + 4].copy_from_slice(&77_u32.to_le_bytes());
        buf[bonus_off + 8..bonus_off + 16].copy_from_slice(&123_000_i64.to_le_bytes());
    }

    let orig_author = (11_u64 << 32) | 79;
    let neighbor_author = (22_u64 << 32) | 79;
    let replacement_author = (99_u64 << 32) | 79;
    let write_variant = |buf: &mut [u8],
                         off: usize,
                         id: u32,
                         token: u32,
                         def_off: usize,
                         author: u64,
                         trait_count: i32,
                         with_bonus: bool,
                         values: [i64; 10]| {
        buf[off..off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        buf[off + TYPE..off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        buf[off + ID..off + ID + 4].copy_from_slice(&id.to_le_bytes());
        buf[off + EQUIPMENT_TOKEN..off + EQUIPMENT_TOKEN + 4].copy_from_slice(&token.to_le_bytes());
        buf[off + DEFINITION..off + DEFINITION + 8]
            .copy_from_slice(&((base_ptr + def_off) as u64).to_le_bytes());
        buf[off + INDUSTRIAL_ORGANISATION..off + INDUSTRIAL_ORGANISATION + 8]
            .copy_from_slice(&author.to_le_bytes());
        buf[off + ORGANISATION_TRAIT_COUNT..off + ORGANISATION_TRAIT_COUNT + 4]
            .copy_from_slice(&trait_count.to_le_bytes());
        let (bonus_ptr, bonus_count) = if with_bonus {
            ((base_ptr + bonus_off) as u64, 1_u32)
        } else {
            (0_u64, 0_u32)
        };
        buf[off + ORGANISATION_BONUSES..off + ORGANISATION_BONUSES + 8]
            .copy_from_slice(&bonus_ptr.to_le_bytes());
        buf[off + ORGANISATION_BONUSES + 8..off + ORGANISATION_BONUSES + 12]
            .copy_from_slice(&bonus_count.to_le_bytes());
        buf[off + ORGANISATION_BONUSES + 12..off + ORGANISATION_BONUSES + 16]
            .copy_from_slice(&bonus_count.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(values) {
            buf[off + lane.offset..off + lane.offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        buf[off + BUILD_COST..off + BUILD_COST + 8].copy_from_slice(&values[9].to_le_bytes());
    };

    for block_replacement_org in [false, true] {
        {
            let mut buf = bytes.borrow_mut();
            write_variant(&mut buf, var0, 1, 77, def0, orig_author, 1, false, base_values);
            write_variant(
                &mut buf,
                var1,
                2,
                77,
                def0,
                neighbor_author,
                2,
                false,
                base_values,
            );
        }
        let blocked = Cell::new(None);
        let world = LiveWorld {
            bytes: &bytes,
            blocked: &blocked,
        };
        let v0 = RegistryRow::read(&world, 0, base_ptr + var0).unwrap();
        let v1 = RegistryRow::read(&world, 0, base_ptr + var1).unwrap();
        let mut rows = Vec::new();
        let mut witness = RecordingWitness {
            withheld: Vec::new(),
        };
        let mut certified = Vec::new();
        let mut carry = SwitchingCarry {
            on_first_fresh: Some(|| {
                let mut buf = bytes.borrow_mut();
                write_variant(
                    &mut buf,
                    var0,
                    99,
                    88,
                    def1,
                    replacement_author,
                    7,
                    true,
                    replacement_values,
                );
                if block_replacement_org {
                    blocked.set(Some(base_ptr + var0 + INDUSTRIAL_ORGANISATION));
                }
            }),
            certified: &mut certified,
        };
        assert_eq!(
            walk_stats_carried(&world, 0, &[v0, v1], &mut rows, &mut carry, &mut witness),
            Outcome::Rows { len: 1 }
        );
        assert!(witness
            .withheld
            .iter()
            .any(|(_, reason)| *reason == "variant identity changed during organisation read"));
        assert_eq!(certified, vec![v1.id]);
        assert!(rows[..1].iter().all(|row| row.variant != v0.id));
        assert!(rows[..1]
            .iter()
            .all(|row| row.organisation.author != replacement_author));
        assert_eq!(rows[0].variant, v1.id);
        assert_eq!(rows[0].organisation.author, neighbor_author);
        assert_eq!(rows[0].values[4], Some(42_000_000));
        assert_eq!(rows[0].values[9], Some(2_400_000));
    }
}

#[test]
fn walk_stats_carried_refusing_fresh_row_preserves_next_designs_carried_organisation() {
    use std::cell::RefCell;

    struct LiveWorld<'a> {
        bytes: &'a RefCell<Vec<u8>>,
        bump_on_org_bonus: &'a Cell<bool>,
        revision: &'a Cell<u64>,
        trigger_at: usize,
    }

    impl Memory for LiveWorld<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let bytes = self.bytes.borrow();
            let base = bytes.as_ptr() as usize;
            let Some(offset) = address.checked_sub(base) else {
                return false;
            };
            let Some(slice) = bytes.get(offset..offset.saturating_add(output.len())) else {
                return false;
            };
            output.copy_from_slice(slice);
            if self.bump_on_org_bonus.get()
                && (address..address.saturating_add(output.len())).contains(&self.trigger_at)
            {
                self.bump_on_org_bonus.set(false);
                self.revision.set(self.revision.get() + 1);
            }
            true
        }
    }

    struct RevWitness<'a> {
        revision: &'a Cell<u64>,
        withheld: Vec<(&'static str, &'static str)>,
    }

    impl StatWitness for RevWitness<'_> {
        fn quiet(&self) -> Result<QuietToken, &'static str> {
            Ok(QuietToken::new(1, self.revision.get()))
        }
        fn record_withheld(&mut self, lane: usize, reason: &'static str) {
            self.withheld.push((STAT_LANES[lane].name, reason));
        }
    }

    struct CarryReuseB<F: FnMut()> {
        b_id: u64,
        on_a_fresh: Option<F>,
    }

    impl<F: FnMut()> OrganisationCarry for CarryReuseB<F> {
        fn fresh(&mut self, index: usize, source: &RegistryRow) -> Result<bool, &'static str> {
            if index == 0 && source.id == self.b_id {
                return Ok(false);
            }
            if let Some(mut trigger) = self.on_a_fresh.take() {
                trigger();
            }
            Ok(true)
        }
    }

    let bytes = RefCell::new(vec![0_u8; 0x2600]);
    let base_ptr = bytes.borrow().as_ptr() as usize;
    let var_a = 0x0;
    let var_b = 0x600;
    let def0 = 0xc00;
    let bonus_a_off = 0x1800;
    let bonus_b_off = 0x1900;
    let base_values = [
        100_000_000_i64,
        1_200_000,
        1_800_000,
        4_500_000,
        42_000_000,
        80_000,
        1_000_000,
        1_500_000,
        21_000,
        2_400_000,
    ];
    {
        let mut buf = bytes.borrow_mut();
        buf[def0..def0 + 8]
            .copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
        buf[def0 + cequipment_type::TOKEN..def0 + cequipment_type::TOKEN + 4]
            .copy_from_slice(&77_u32.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
            let off = def0 + cequipment_type::STATS_BASE + lane.stat_index * 8;
            buf[off..off + 8].copy_from_slice(&value.to_le_bytes());
        }
        buf[bonus_a_off..bonus_a_off + 4].copy_from_slice(&11_u32.to_le_bytes());
        buf[bonus_a_off + 8..bonus_a_off + 16].copy_from_slice(&111_000_i64.to_le_bytes());
        buf[bonus_b_off..bonus_b_off + 4].copy_from_slice(&77_u32.to_le_bytes());
        buf[bonus_b_off + 8..bonus_b_off + 16].copy_from_slice(&456_000_i64.to_le_bytes());
    }

    let author_a = (11_u64 << 32) | 79;
    let author_b = (22_u64 << 32) | 79;
    let author_replaced = (99_u64 << 32) | 79;
    let write_variant = |buf: &mut [u8],
                         off: usize,
                         id: u32,
                         author: u64,
                         trait_count: i32,
                         bonus_off: Option<usize>| {
        buf[off..off + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        buf[off + TYPE..off + TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        buf[off + ID..off + ID + 4].copy_from_slice(&id.to_le_bytes());
        buf[off + EQUIPMENT_TOKEN..off + EQUIPMENT_TOKEN + 4].copy_from_slice(&77_u32.to_le_bytes());
        buf[off + DEFINITION..off + DEFINITION + 8]
            .copy_from_slice(&((base_ptr + def0) as u64).to_le_bytes());
        buf[off + INDUSTRIAL_ORGANISATION..off + INDUSTRIAL_ORGANISATION + 8]
            .copy_from_slice(&author.to_le_bytes());
        buf[off + ORGANISATION_TRAIT_COUNT..off + ORGANISATION_TRAIT_COUNT + 4]
            .copy_from_slice(&trait_count.to_le_bytes());
        let (bonus_ptr, bonus_count) = match bonus_off {
            Some(at) => ((base_ptr + at) as u64, 1_u32),
            None => (0_u64, 0_u32),
        };
        buf[off + ORGANISATION_BONUSES..off + ORGANISATION_BONUSES + 8]
            .copy_from_slice(&bonus_ptr.to_le_bytes());
        buf[off + ORGANISATION_BONUSES + 8..off + ORGANISATION_BONUSES + 12]
            .copy_from_slice(&bonus_count.to_le_bytes());
        buf[off + ORGANISATION_BONUSES + 12..off + ORGANISATION_BONUSES + 16]
            .copy_from_slice(&bonus_count.to_le_bytes());
        for (lane, value) in STAT_LANES.into_iter().zip(base_values) {
            buf[off + lane.offset..off + lane.offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        buf[off + BUILD_COST..off + BUILD_COST + 8].copy_from_slice(&base_values[9].to_le_bytes());
    };

    for refuse_by_closing_quiet in [false, true] {
        {
            let mut buf = bytes.borrow_mut();
            write_variant(&mut buf, var_a, 1, author_a, 1, Some(bonus_a_off));
            write_variant(&mut buf, var_b, 2, author_b, 2, Some(bonus_b_off));
        }
        let rev = Cell::new(1_u64);
        let bump = Cell::new(false);
        let world = LiveWorld {
            bytes: &bytes,
            bump_on_org_bonus: &bump,
            revision: &rev,
            trigger_at: base_ptr + bonus_a_off,
        };
        let row_a = RegistryRow::read(&world, 0, base_ptr + var_a).unwrap();
        let row_b = RegistryRow::read(&world, 0, base_ptr + var_b).unwrap();

        let mut rows = Vec::new();
        let mut init_witness = RevWitness {
            revision: &rev,
            withheld: Vec::new(),
        };
        assert_eq!(
            walk_stats_carried(&world, 0, &[row_b], &mut rows, &mut (), &mut init_witness),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].variant, row_b.id);
        assert_eq!(rows[0].organisation.author, author_b);
        assert_eq!(rows[0].organisation.trait_count, 2);
        assert_eq!(rows[0].organisation.bonuses.len(), 1);
        assert_eq!(
            (
                rows[0].organisation.bonuses[0].key,
                rows[0].organisation.bonuses[0].value_raw,
            ),
            (77, 456_000)
        );

        let mut witness = RevWitness {
            revision: &rev,
            withheld: Vec::new(),
        };
        let mut carry = CarryReuseB {
            b_id: row_b.id,
            on_a_fresh: Some(|| {
                if refuse_by_closing_quiet {
                    bump.set(true);
                } else {
                    let mut buf = bytes.borrow_mut();
                    write_variant(&mut buf, var_a, 99, author_replaced, 7, Some(bonus_a_off));
                }
            }),
        };
        assert_eq!(
            walk_stats_carried(
                &world,
                0,
                &[row_a, row_b],
                &mut rows,
                &mut carry,
                &mut witness,
            ),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].variant, row_b.id);
        assert_eq!(rows[0].organisation.author, author_b);
        assert_eq!(rows[0].organisation.trait_count, 2);
        assert_eq!(rows[0].organisation.bonuses.len(), 1);
        assert_eq!(
            (
                rows[0].organisation.bonuses[0].key,
                rows[0].organisation.bonuses[0].value_raw,
            ),
            (77, 456_000)
        );
        let expected_refusal = if refuse_by_closing_quiet {
            "variant stat writer crossed read"
        } else {
            "variant identity changed during organisation read"
        };
        assert!(witness
            .withheld
            .iter()
            .any(|(_, reason)| *reason == expected_refusal));
    }
}
