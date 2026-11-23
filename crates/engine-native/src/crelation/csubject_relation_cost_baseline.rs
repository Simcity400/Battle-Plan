use super::*;

pub(super) fn walk_owned(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
    scratch: &mut OwnedSnapshotScratch,
) -> NameWalk {
    scratch.begin();
    let mut resolver = std::mem::take(&mut scratch.resolver);
    let mut pass = Pass {
        resolver: &mut resolver,
        ownership: Some(&mut *scratch),
    };
    let result = walk_from(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
        false,
        &mut pass,
    );
    scratch.resolver = resolver;
    result
}

fn walk_from(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    matrix: bool,
    pass: &mut Pass<'_>,
) -> NameWalk {
    let result = (|| {
        let reader = Reader::new(memory, base, game_state)?;
        pass.resolver.begin(&reader.countries);
        if output.is_growing() {
            pass.resolver
                .grow()
                .map_err(|_| "subject resolver allocation failed")?;
        }
        let mut found_country = false;
        let (mut len, mut bytes_len, mut required_bytes) = (0_usize, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        let mut highest_slot = 0;
        if matrix {
            for slot in 1..reader.countries.count() {
                if reader.countries.at(slot)?.is_some() {
                    highest_slot = slot;
                }
            }
        }
        for slot in 1..reader.countries.count() {
            let Some(country) = reader.countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let diplomacy = pass
                .resolver
                .diplomacy(country, || reader.diplomacy(country))?;
            let source = reader.array(
                diplomacy,
                if matrix {
                    engine_native_parts::cdiplomacy_status_layout::RELATIONS
                } else {
                    engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS
                },
            )?;
            if matrix && source.count <= highest_slot {
                return Err("subject matrix does not cover country ordinals");
            }
            if !matrix && let Some(scratch) = pass.ownership.as_deref_mut() {
                scratch.observe_owned(source.count);
            }
            let mut matrix_pointers = [0; 64];
            for index in 0..source.count {
                let pointer = if matrix {
                    if index % 64 == 0 {
                        reader.matrix_pointers(source, index, diplomacy, &mut matrix_pointers)?;
                    }
                    let pointer = matrix_pointers[index as usize % 64];
                    if pointer == 0 {
                        continue;
                    }
                    pointer
                } else {
                    let entry = reader.element(source, index)?;
                    if entry == 0 {
                        return Err("null subject relation root entry");
                    }
                    if reader.field::<u32>(entry, TYPE_ID)? != TYPE_VALUE {
                        continue;
                    }
                    entry
                };
                let mut row =
                    match relation(&reader, pointer, country, matrix.then_some(index), pass) {
                        Ok(Some(row)) => row,
                        Ok(None) => continue,
                        Err(reason) => return Err(reason),
                    };
                let state = reader.pointer(pointer, AUTONOMY_STATE)?;
                let (source, name_len) = definition_name(memory, state)?;
                required_bytes = required_bytes
                    .checked_add(name_len)
                    .ok_or("subject name byte count overflow")?;
                output.ensure(len.checked_add(1).ok_or("subject row demand overflow")?)?;
                names.ensure(required_bytes)?;
                if len == output.len() || name_len > names.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("subject row count overflow")?;
                    continue;
                }
                if !read_slice(memory, source, &mut names[bytes_len..bytes_len + name_len]) {
                    return Err("subject name bytes unreadable");
                }
                row.state = Name {
                    start: bytes_len,
                    len: name_len,
                };
                output[len] = row;
                len += 1;
                bytes_len += name_len;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(unavailable)
}

fn relation<M: Memory>(
    reader: &Reader<'_, M>,
    pointer: usize,
    owner: engine_native_memory::country_identity::Country,
    matrix_index: Option<u32>,
    pass: &mut Pass<'_>,
) -> Result<Option<Row>, &'static str> {
    reader.vtable(pointer, VTABLE_RVA, "subject relation vtable mismatch")?;
    if reader.field::<u32>(pointer, TYPE_ID)? != TYPE_VALUE {
        return Err("subject matrix contains another relation type");
    }
    let retired = reader.field::<u8>(pointer, crelation::RETIRED)? != 0;
    let overlord = reader.field::<u32>(pointer, OVERLORD)?;
    let subject = reader.field::<u32>(pointer, SUBJECT)?;
    if retired && matrix_index.is_none() && !reader.installed(pointer, overlord, subject, pass)? {
        return Ok(None);
    }
    let first = pass.resolver.resolve(&reader.countries, overlord)?;
    let second = pass.resolver.resolve(&reader.countries, subject)?;
    if first.canonical_slot == second.canonical_slot {
        return Err("subject endpoints resolve to same country");
    }
    let target = if owner.canonical_slot == first.canonical_slot {
        second
    } else if owner.canonical_slot == second.canonical_slot {
        first
    } else {
        return Err("subject relation does not include owning country");
    };
    if matrix_index.is_some_and(|index| index != target.canonical_slot) {
        return Err("subject relation stored in wrong matrix slot");
    }
    let first_diplomacy = pass.resolver.diplomacy(first, || reader.diplomacy(first))?;
    let second_diplomacy = pass
        .resolver
        .diplomacy(second, || reader.diplomacy(second))?;
    let first_status = reader.pair_status(first_diplomacy, second, pointer, pass)?;
    let second_status = reader.pair_status(second_diplomacy, first, pointer, pass)?;
    if matrix_index.is_none() {
        for diplomacy in [first_diplomacy, second_diplomacy] {
            reader.occurrence(
                diplomacy,
                engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
                pointer,
                1,
                pass,
            )?;
        }
        for status in [first_status, second_status] {
            reader.occurrence(status, crelation_status::SUBRELATIONS, pointer, 1, pass)?;
        }
        reader.occurrence(
            first_status,
            crelation_status::SERIALIZABLE_RELATIONS,
            pointer,
            1,
            pass,
        )?;
        reader.occurrence(
            second_status,
            crelation_status::SERIALIZABLE_RELATIONS,
            pointer,
            0,
            pass,
        )?;
    }
    if owner.canonical_slot != first.canonical_slot {
        return Ok(None);
    }
    Ok(Some(Row {
        overlord,
        subject,
        start_hour: reader.field(pointer, START_HOUR)?,
        end_hour: reader.field(pointer, crelation::END_HOUR)?,
        cancel: reader.field(pointer, crelation::CANCEL)?,
        autonomy_ratio: reader.field(pointer, AUTONOMY_RATIO)?,
        state: Name::default(),
    }))
}
