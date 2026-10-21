//! Compact saved edits. Callback work is bounded by the fields the writer owns.
use super::*;

const NAMES_GROUP_TOKEN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Obsolete,
    AutoUpgraded,
    Highlight,
    MaxVersion,
    RoleIcon,
    Name,
    Model,
    Icon,
    NamesGroup,
    Niches,
}
pub(super) fn field(site: usize) -> Option<Field> {
    Some(match MUTATIONS[site].name {
        "SetObsolete" | "PropagateObsolete" => Field::Obsolete,
        "SetAutoUpgraded" => Field::AutoUpgraded,
        "SetHighlight" => Field::Highlight,
        "SetRoleIcon" => Field::RoleIcon,
        "Rename" => Field::Name,
        "SetModel" => Field::Model,
        "SetIcon" => Field::Icon,
        "SetNamesGroup" => Field::NamesGroup,
        "RefreshNiches" => Field::Niches,
        _ => return None,
    })
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("variant scalar address overflow")
}
fn stage_text(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    address: usize,
) -> Result<(), &'static str> {
    let header = take(log, memory, address, 32)?;
    string(log, memory, header)
}
fn stage(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    pointer: usize,
    field: Field,
) -> Result<(), &'static str> {
    let (offset, size) = match field {
        Field::Obsolete => (native::OBSOLETE, 1),
        Field::AutoUpgraded => (native::AUTO_UPGRADED, 1),
        Field::Highlight => (native::HIGHLIGHT, 1),
        Field::MaxVersion => (native::MAX_VERSION, 1),
        Field::RoleIcon => (native::ROLE_ICON_INDEX, 4),
        Field::Name => return stage_text(log, memory, at(pointer, native::DISPLAY_NAME)?),
        Field::Model => return stage_text(log, memory, at(pointer, native::EXPLICIT_ENTITY)?),
        Field::Icon => {
            take(log, memory, at(pointer, native::ICON_OVERRIDE_ACTIVE)?, 1)?;
            take(log, memory, at(pointer, native::ICON_OVERRIDE_KIND)?, 4)?;
            return stage_text(log, memory, at(pointer, native::ICON_OVERRIDE_TOKEN)?);
        }
        Field::NamesGroup => {
            let header = take(log, memory, at(pointer, native::DIVISION_NAMES_GROUP)?, 8)?;
            let group = u64_at(log, header) as usize;
            if group != 0 {
                stage_text(log, memory, at(group, NAMES_GROUP_TOKEN)?)?;
            }
            return Ok(());
        }
        Field::Niches => {
            let header = take(log, memory, at(pointer, native::NICHES)?, 16)?;
            array(log, memory, header, 4)?;
            return Ok(());
        }
    };
    take(log, memory, at(pointer, offset)?, size)?;
    Ok(())
}
pub(super) fn observe(memory: &impl Memory, base: usize, pointer: usize, field: Field) {
    let Some(store) = STORE.get() else {
        unobserved!();
        return;
    };
    let mut store = callback_store(store);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    record(&mut store, memory, base, pointer, field);
}
fn record(store: &mut Store, memory: &impl Memory, base: usize, pointer: usize, field: Field) {
    let Some(watch) = store.watched.get_mut(&pointer) else {
        return;
    };
    watch.dirty = true;
    let variant = watch.id;
    store.batch.required_events = store.batch.required_events.saturating_add(1);
    if store.batch.events.len() == store.batch.events.capacity() {
        store.batch.failure = Some("variant scalar event capacity exhausted");
        return;
    }
    let result = (|| {
        let state = engine_native::read::<u64>(
            memory,
            at(base, engine_native::cgame_state::SINGLETON_RVA)?,
        )
        .ok_or("variant scalar clock unreadable")? as usize;
        let hour = engine_native::read::<i32>(
            memory,
            at(state, engine_native::cgame_state::CURRENT_HOUR)?,
        )
        .ok_or("variant scalar hour unreadable")?;
        let start = store.batch.log.spans.len();
        stage(&mut store.batch.log, memory, pointer, field)?;
        let end = store.batch.log.spans.len();
        store.batch.events.push(Event {
            kind: Kind::Scalar(field),
            sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
            hour,
            pointer,
            variant,
            flags: None,
            removed: false,
            start,
            end,
        });
        Ok(())
    })();
    if let Err(reason) = result {
        store.batch.failure = Some(reason);
    }
}
fn word<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    engine_native::read(memory, at(pointer, offset)?).ok_or("variant scalar field unreadable")
}
fn text(memory: &impl Memory, address: usize) -> Result<Vec<u8>, &'static str> {
    let header: [u8; 32] = word(memory, address, 0)?;
    let length = usize::try_from(u64::from_le_bytes(header[16..24].try_into().unwrap()))
        .map_err(|_| "variant scalar text length overflow")?;
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if length as u64 > capacity || capacity < 15 {
        return Err("variant scalar text invalid");
    }
    let mut result =
        engine_allocation::vec(length).map_err(|_| "variant scalar text allocation failed")?;
    result.resize(length, 0);
    if capacity == 15 {
        result.copy_from_slice(&header[..length]);
    } else if !memory.copy(
        u64::from_le_bytes(header[..8].try_into().unwrap()) as usize,
        &mut result,
    ) {
        return Err("variant scalar text unreadable");
    }
    Ok(result)
}
pub(super) fn decode(
    memory: &impl Memory,
    pointer: usize,
    field: Field,
) -> Result<rows::VariantScalarChange, &'static str> {
    let mut change = rows::VariantScalarChange::default();
    match field {
        Field::Obsolete => change.obsolete = Some(word(memory, pointer, native::OBSOLETE)?),
        Field::AutoUpgraded => {
            change.auto_upgraded = Some(word(memory, pointer, native::AUTO_UPGRADED)?)
        }
        Field::Highlight => change.highlight = Some(word(memory, pointer, native::HIGHLIGHT)?),
        Field::MaxVersion => change.max_version = Some(word(memory, pointer, native::MAX_VERSION)?),
        Field::RoleIcon => {
            change.role_icon_index = Some(word(memory, pointer, native::ROLE_ICON_INDEX)?)
        }
        Field::Name => {
            change.display_name = Some(text(memory, at(pointer, native::DISPLAY_NAME)?)?)
        }
        Field::Model => change.model = Some(text(memory, at(pointer, native::EXPLICIT_ENTITY)?)?),
        Field::Icon => {
            change.icon = Some(rows::VariantIconChange {
                active: word(memory, pointer, native::ICON_OVERRIDE_ACTIVE)?,
                kind: word(memory, pointer, native::ICON_OVERRIDE_KIND)?,
                text: text(memory, at(pointer, native::ICON_OVERRIDE_TOKEN)?)?,
            })
        }
        Field::NamesGroup => {
            let group: u64 = word(memory, pointer, native::DIVISION_NAMES_GROUP)?;
            change.names_group = Some(if group == 0 {
                None
            } else {
                Some(text(memory, at(group as usize, NAMES_GROUP_TOKEN)?)?)
            });
        }
        Field::Niches => {
            let source = Array::read(memory, at(pointer, native::NICHES)?, 4)
                .ok_or("variant scalar niches invalid")?;
            let mut values = engine_allocation::vec(source.count as usize)
                .map_err(|_| "variant scalar niches allocation failed")?;
            for i in 0..source.count {
                values.push(word(
                    memory,
                    source
                        .element(i, 4)
                        .ok_or("variant scalar niches overflow")?,
                    0,
                )?);
            }
            change.niches = Some(values);
        }
    }
    Ok(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    const POINTER: usize = 0x1000;
    struct ToggleMemory {
        value: u8,
        reads: Cell<usize>,
    }
    impl Memory for ToggleMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if address == engine_native::cgame_state::SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&0x2000_u64.to_le_bytes());
            } else if address == 0x2000 + engine_native::cgame_state::CURRENT_HOUR
                && output.len() == 4
            {
                output.copy_from_slice(&7_i32.to_le_bytes());
            } else if address == POINTER + native::AUTO_UPGRADED && output.len() == 1 {
                output[0] = self.value;
            } else {
                panic!("scalar callback followed an unchanged dependency");
            }
            true
        }
    }
    #[test]
    fn scalar_toggle_copies_only_one_byte_and_preserves_repeated_same_hour_values() {
        let mut store = Store {
            batch: Batch::empty(),
            spare: Batch::empty(),
            watched: HashMap::from([(
                POINTER,
                Watch {
                    id: 70,
                    pointer: POINTER,
                    flags: [0; 3],
                    dirty: false,
                },
            )]),
            interests: sources::Interests::default(),
            epoch: 1,
        };
        store.batch.prepare(4, 8, 8);
        for value in [1, 0] {
            let memory = ToggleMemory {
                value,
                reads: Cell::new(0),
            };
            collector_test_support::with_allocations_denied(|| {
                record(&mut store, &memory, 0, POINTER, Field::AutoUpgraded);
            });
            assert_eq!(memory.reads.get(), 3);
            assert!(store.batch.failure.is_none());
        }
        let mut output = Vec::new();
        decode_batch(&store.batch, 0, 7, &mut output).unwrap();
        assert_eq!(
            output
                .iter()
                .map(|row| row.scalar.as_ref().unwrap().auto_upgraded)
                .collect::<Vec<_>>(),
            [Some(1), Some(0)]
        );
        assert_eq!(store.batch.log.bytes.len(), 2);
        assert!(output.iter().all(|row| row.snapshot.is_none()));
    }
    #[test]
    fn scalar_capacity_refusal_keeps_the_receiver_dirty_without_native_reads() {
        let mut store = Store {
            batch: Batch::empty(),
            spare: Batch::empty(),
            watched: HashMap::from([(
                POINTER,
                Watch {
                    id: 70,
                    pointer: POINTER,
                    flags: [0; 3],
                    dirty: false,
                },
            )]),
            interests: sources::Interests::default(),
            epoch: 1,
        };
        let memory = ToggleMemory {
            value: 1,
            reads: Cell::new(0),
        };
        collector_test_support::with_allocations_denied(|| {
            record(&mut store, &memory, 0, POINTER, Field::AutoUpgraded);
        });
        assert!(store.watched[&POINTER].dirty);
        assert_eq!(store.watched[&POINTER].id, 70);
        assert_eq!(store.batch.required_events, 1);
        assert_eq!(
            store.batch.failure,
            Some("variant scalar event capacity exhausted")
        );
        assert_eq!(memory.reads.get(), 0);
        assert!(store.batch.events.is_empty());
    }

    #[test]
    fn narrow_writer_routes_and_owned_fields_do_not_follow_design_dependencies() {
        for (name, expected) in [
            ("SetObsolete", Field::Obsolete),
            ("PropagateObsolete", Field::Obsolete),
            ("SetHighlight", Field::Highlight),
            ("SetAutoUpgraded", Field::AutoUpgraded),
            ("Rename", Field::Name),
            ("SetNamesGroup", Field::NamesGroup),
            ("SetRoleIcon", Field::RoleIcon),
            ("SetModel", Field::Model),
            ("SetIcon", Field::Icon),
            ("RefreshNiches", Field::Niches),
        ] {
            let site = MUTATIONS.iter().position(|site| site.name == name).unwrap();
            assert_eq!(field(site), Some(expected));
        }
        let mut bytes = vec![0; ROOT_BYTES];
        for offset in [
            native::DISPLAY_NAME,
            native::EXPLICIT_ENTITY,
            native::ICON_OVERRIDE_TOKEN,
        ] {
            bytes[offset] = b'X';
            bytes[offset + 16..offset + 24].copy_from_slice(&1_u64.to_le_bytes());
            bytes[offset + 24..offset + 32].copy_from_slice(&15_u64.to_le_bytes());
        }
        // Unreadable dependency roots cannot affect these observations.
        bytes[native::MODULES + 8..native::MODULES + 24].fill(0xff);
        bytes[native::BONUS_SOURCES..native::BONUS_SOURCES + 16].fill(0xff);
        let memory = engine_native::memory::SliceMemory {
            base: POINTER,
            bytes: &bytes,
        };
        for field in [
            Field::Obsolete,
            Field::Highlight,
            Field::AutoUpgraded,
            Field::MaxVersion,
            Field::RoleIcon,
            Field::Name,
            Field::Model,
            Field::Icon,
            Field::NamesGroup,
            Field::Niches,
        ] {
            let mut batch = Batch::empty();
            batch.prepare(1, 8, 256);
            collector_test_support::with_allocations_denied(|| {
                stage(&mut batch.log, &memory, POINTER, field)
            })
            .unwrap();
            let owned =
                crate::raw::Recorded::build_range(&batch.log, 0..batch.log.spans.len()).unwrap();
            let change = decode(&owned, POINTER, field).unwrap();
            assert_ne!(change, rows::VariantScalarChange::default());
            let mut definition = rows::VariantDefinitionRow {
                design: rows::Recorded::Value(Default::default()),
                ..Default::default()
            };
            change.apply_definition(&mut definition).unwrap();
            if field == Field::Name {
                assert_eq!(definition.display_name, b"X");
            }
            if field == Field::Model {
                assert_eq!(definition.explicit_entity, b"X");
            }
            if field == Field::Icon {
                assert_eq!(definition.icon, b"X");
            }
        }
    }
}
