use super::*;
use crate::memory::SliceMemory;
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
            memory.put(pointer, &(VTABLE_RVA as u64).to_le_bytes());
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
        if address == cgame_state::SINGLETON_RVA && destination.len() == 8 {
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
    let mut bytes = vec![0; 0xa00];
    bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
    bytes[8..16]
        .copy_from_slice(&((u64::from(u32::MAX) << 32) | u64::from(REFERENCE_TYPE)).to_le_bytes());
    bytes[EQUIPMENT_TOKEN..EQUIPMENT_TOKEN + 4].copy_from_slice(&0x8000_0042_u32.to_le_bytes());
    bytes[CREATOR_LINK..CREATOR_LINK + 4].copy_from_slice(&900_u32.to_le_bytes());
    bytes[ORIGIN_LINK..ORIGIN_LINK + 4].copy_from_slice(&901_u32.to_le_bytes());
    bytes[DEFINITION..DEFINITION + 8].copy_from_slice(&0x1500_u64.to_le_bytes());
    bytes[0x500..0x508].copy_from_slice(&(cequipment_type::VTABLE_RVA as u64).to_le_bytes());
    bytes[0x508..0x50c].copy_from_slice(&0x8000_0042_u32.to_le_bytes());
    // An archetype is its own archetype.
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
    // A new variant's entry: inactive, kind 1 (a path), empty.
    bytes[ICON_OVERRIDE_KIND..ICON_OVERRIDE_KIND + 4].copy_from_slice(&1_u32.to_le_bytes());
    bytes
}

#[test]
fn design_state_preserves_ancestry_attribution_role_and_raw_flags() {
    struct StaleReference<'a>(SliceMemory<'a>);
    impl Memory for StaleReference<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == crate::creference_registry::LOW_ROOTS_RVA + 8 && output.len() == 8 {
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
    bytes[0x1000..0x1008].copy_from_slice(&(cequipment_type::NULL_VTABLE_RVA as u64).to_le_bytes());
    bytes[0x1008..0x100c].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());
    let variants = vec![source(&bytes); 100];
    let mut rows = Vec::new();
    for vtable in [
        cequipment_type::NULL_VTABLE_RVA,
        cequipment_type::VTABLE_RVA,
    ] {
        bytes[0x1000..0x1008].copy_from_slice(&(vtable as u64).to_le_bytes());
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
    // Two batches; versions step by four through 252 and wrap to zero.
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
    // The entry is the record's last field: a variant cut before it fails
    // there, and a corrupt string header fails like the other strings.
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
    let offsets = [
        MAP_RANGE_STAT,
        STAT_2F,
        AIR_ATTACK_CACHE_CANDIDATE,
        AGILITY_CACHE_CANDIDATE,
        MAXIMUM_SPEED_CACHE_CANDIDATE,
        RELIABILITY_CACHE_CANDIDATE,
        AIR_DEFENSE_CACHE_CANDIDATE,
        THRUST_CACHE_CANDIDATE,
        FUEL_CONSUMPTION_CACHE_CANDIDATE,
        BUILD_COST_CACHE_CANDIDATE,
    ];
    let values = [
        i64::MIN,
        i64::MAX,
        -1,
        0,
        1_i64 << 40,
        -50,
        100_001,
        123,
        9,
        42,
    ];
    for (offset, value) in offsets.into_iter().zip(values) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![StatsRow::default(); 1];
    assert_eq!(
        walk_stats(&memory, &[source(&bytes), source(&bytes)], &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].values, values);
    assert_eq!(rows[1].values, values);
    assert_eq!(
        walk_stats(
            &memory,
            &[
                source(&bytes),
                RegistryRow {
                    id: 0,
                    address: usize::MAX
                }
            ],
            &mut Vec::new()
        ),
        Outcome::Unavailable {
            reason: "variant stat cache unreadable"
        }
    );
    assert!(matches!(
        walk_stats(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes[..BUILD_COST_CACHE_CANDIDATE + 7]
            },
            &[source(&bytes)],
            &mut rows
        ),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn copied_inline_names_and_registry_identity_need_only_three_definition_guards() {
    let mut bytes = fixture();
    bytes[DISPLAY_NAME..DISPLAY_NAME + 3].copy_from_slice(&[0xff, 0, b'X']);
    bytes[DISPLAY_NAME + 16..DISPLAY_NAME + 24].copy_from_slice(&3_u64.to_le_bytes());
    let variants = [source(&bytes)];
    let memory = CountedFields {
        memory: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        guards: Cell::new(0),
        blocked: Some(0x1000),
    };
    let mut rows = vec![DefinitionRow::default(); 1];
    let mut names = vec![0; 3];
    let outcome = walk_definitions(&memory, 0, &variants, &mut rows, &mut names);
    assert_eq!(outcome.outcome, Outcome::Rows { len: 1 });
    assert_eq!(names, [0xff, 0, b'X']);
    assert_eq!(memory.guards.get(), 3);
    memory.guards.set(0);
    assert_eq!(
        walk_stats(&memory, &variants, &mut vec![StatsRow::default(); 1]),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(memory.guards.get(), 2);
}

#[test]
fn variant_record_batches_preserve_order_and_do_not_hide_an_earlier_definition_error() {
    // One more than a full batch, so the boundary is still exercised.
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
        walk_stats(&memory, &variants, &mut stats),
        Outcome::Rows { len: COUNT }
    );
    assert_eq!(memory.guards.get(), 2 + COUNT);
    let memory = CountedFields {
        blocked: Some(variants[32].pointer() + CLASSIFIER_FLAGS),
        ..memory
    };
    assert_eq!(
        walk_definitions(&memory, 0, &variants, &mut rows, &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "variant classifier flags or mission mask unreadable"
        }
    );
    assert_eq!(
        walk_stats(&memory, &variants, &mut stats),
        Outcome::Rows { len: COUNT }
    );
    // An earlier readable row's authentication failure takes precedence
    // over a later batch field fault; no failed destination is decoded.
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
        walk_stats(&memory, &variants, &mut vec![StatsRow::default(); 1]),
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
        fn fresh(
            &mut self,
            _: usize,
            _: &RegistryRow,
            _: &Organisation,
        ) -> Result<bool, &'static str> {
            Ok(false)
        }
    }
    // The certified payload remains held even if every bonus byte is now unreadable.
    bytes.truncate(bonuses_at);
    assert_eq!(
        walk_stats_carried(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            &[source],
            &mut rows,
            &mut Quiet
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
        bytes[0x1000..0x1008].copy_from_slice(&(table as u64).to_le_bytes());
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
