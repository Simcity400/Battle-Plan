use engine_native::ccharacter::{PORTRAIT_DATA, identity_header, portrait_header};
use engine_native_memory::{Memory, msvc_string, read, walk::Outcome};
use engine_native::ccharacter::manager as ccharacter_manager;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PortraitRow {
    pub character: u32,
    pub portraits: [usize; 2],
}

pub struct PortraitWalk<'a> {
    pub part: usize,
    pub parts: usize,
    pub observation: Option<(
        &'a mut crate::portrait_cache::Cache,
        &'a dyn crate::portrait_cache::Changes,
    )>,
    pub census: Option<&'a ccharacter_manager::PortraitCensus>,
}

impl Default for PortraitWalk<'_> {
    fn default() -> Self {
        Self {
            part: 0,
            parts: 1,
            observation: None,
            census: None,
        }
    }
}

pub fn walk_portraits(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<PortraitRow>,
    entries: &mut Vec<crate::character_portrait_entry::Row<engine_native_memory::capture_text::Owned>>,
    walk: PortraitWalk<'_>,
) -> engine_native::cprovince::UnitOrderWalk {
    walk_portraits_buffer(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(entries),
        walk,
    )
}

pub fn count_portraits(
    memory: &impl Memory,
    base: usize,
    state: usize,
    walk: PortraitWalk<'_>,
) -> engine_native::cprovince::UnitOrderWalk {
    walk_portraits_buffer::<engine_native_memory::capture_text::Owned>(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        walk,
    )
}

fn walk_portraits_buffer<T: engine_native_memory::capture_text::Value>(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, PortraitRow>,
    mut entries: engine_native_memory::walk::Buffer<'_, crate::character_portrait_entry::Row<T>>,
    walk: PortraitWalk<'_>,
) -> engine_native::cprovince::UnitOrderWalk {
    let PortraitWalk {
        part,
        parts,
        mut observation,
        census,
    } = walk;
    let identities = census.and_then(|census| census.rows(base, state));
    if let Some((cache, _)) = observation.as_mut() {
        cache.begin((base, state, part, parts));
    }
    let mut used = 0usize;
    let mut required = 0usize;
    let result = (|| {
        let roster = if identities.is_none() {
            Some(ccharacter_manager::Roster::read(memory, base, state)?)
        } else {
            None
        };
        let count = identities.map_or_else(|| roster.unwrap().count() as usize, <[_]>::len);
        let mut len = 0;
        let mut missing = 0;
        let first = (count * part / parts) as u32;
        let last = (count * (part + 1) / parts) as u32;
        output.ensure((last - first) as usize)?;
        let mut hinted = first;
        for index in first..last {
            if let Some(identities) = identities {
                while hinted < last.min(index + 16) {
                    if let Some(address) = identities[hinted as usize]
                        .pointer
                        .checked_add(PORTRAIT_DATA)
                        && engine_native_memory::memory::is_canonical_range(address, 16)
                    {
                        memory.prefetch(address, 16);
                    }
                    hinted += 1;
                }
            }
            let (pointer, character) = if let Some(identities) = identities {
                let identity = identities[index as usize];
                (identity.pointer, identity.character)
            } else {
                let (pointer, _, _) = roster.unwrap().pointer(memory, index)?;
                (pointer, identity_header(memory, base, pointer)?)
            };
            let array = pointer.wrapping_add(PORTRAIT_DATA);
            let start = used;
            let required_before = required;
            let slot = (index - first) as usize;
            let live = (|| {
                let header = if identities.is_some() {
                    let address = pointer
                        .checked_add(PORTRAIT_DATA)
                        .ok_or("character portrait address overflow")?;
                    read(memory, address).ok_or("character portrait header unreadable")?
                } else {
                    portrait_header(memory, pointer)?
                };
                let a = engine_native_memory::cpdx_array::Array::from_header(
                    header,
                    crate::character_portrait_entry::STRIDE,
                )
                .ok_or("character portrait array invalid")?;
                required = required
                    .checked_add(a.count as usize)
                    .ok_or("character portrait entry count overflow")?;
                entries.ensure(required)?;
                if len == output.len() || a.count as usize > entries.len().saturating_sub(used) {
                    return Ok(None);
                }
                let revision = observation
                    .as_ref()
                    .and_then(|(_, changes)| changes.revision(array));
                let key = crate::portrait_cache::Key {
                    pointer,
                    character,
                    header,
                    start,
                    revision,
                };
                let hit = observation
                    .as_ref()
                    .is_some_and(|(cache, _)| cache.matches(slot, key));
                if let Some((cache, _)) = observation.as_mut() {
                    if hit {
                        cache.counts.reused_characters += 1;
                        cache.counts.reused_entries += u64::from(a.count);
                    } else {
                        cache.counts.read_characters += 1;
                    }
                }
                let mut hinted = 0;
                for n in 0..a.count {
                    if hit {
                        used += 1;
                        continue;
                    }
                    while hinted < a.count.min(n + 4) {
                        if let Some(entry) =
                            a.element(hinted, crate::character_portrait_entry::STRIDE)
                        {
                            const HINT_BYTES: usize =
                                crate::character_portrait_entry::IDENTITY + msvc_string::STRIDE;
                            if engine_native_memory::memory::is_canonical_range(entry, HINT_BYTES) {
                                engine_native_memory::prefetch::span(entry, HINT_BYTES);
                            }
                        }
                        hinted += 1;
                    }
                    let mut current = crate::character_portrait_entry::Row::default();
                    crate::character_portrait_entry::read_buffer(
                        memory,
                        a.element(n, crate::character_portrait_entry::STRIDE)
                            .ok_or("character portrait entry overflow")?,
                        &mut current,
                    )?;
                    entries[used] = current;
                    used += 1;
                }
                if let Some((cache, changes)) = observation.as_mut() {
                    let after = changes.revision(array);
                    cache.remember(
                        slot,
                        crate::portrait_cache::Key {
                            revision: revision.filter(|r| Some(*r) == after),
                            ..key
                        },
                    );
                    if hit && after != revision {
                        return Err("character portrait mutated during acquisition");
                    }
                }
                Ok(Some(a.count as usize))
            })();
            let placed = crate::portrait_cache::settled(array, live, |image| {
                used = start;
                required = required_before;
                let count = engine_native_memory::cpdx_array::Array::from_header(
                    image.header,
                    crate::character_portrait_entry::STRIDE,
                )
                .ok_or("character portrait array invalid")?
                .count as usize;
                if count != image.entries().len() {
                    return Err("character portrait pre-image incomplete");
                }
                required = required
                    .checked_add(count)
                    .ok_or("character portrait entry count overflow")?;
                entries.ensure(required)?;
                if len == output.len() || count > entries.len().saturating_sub(used) {
                    return Ok(None);
                }
                for index in 0..count {
                    entries[used] = crate::character_portrait_entry::from_image(&image, index)?;
                    used += 1;
                }
                if let Some((cache, _)) = observation.as_mut() {
                    cache.remember(
                        slot,
                        crate::portrait_cache::Key {
                            pointer,
                            character,
                            header: image.header,
                            start,
                            revision: None,
                        },
                    );
                }
                Ok(Some(count))
            })?;
            let Some(count) = placed else {
                missing += 1;
                continue;
            };
            output[len] = PortraitRow {
                character,
                portraits: [start, count],
            };
            len += 1;
        }
        output[..len].sort_unstable_by_key(|r| r.character);
        if output[..len]
            .windows(2)
            .any(|p| p[0].character == p[1].character)
        {
            return Err("duplicate character portrait identity");
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    if let Some((cache, _)) = observation.as_mut() {
        match result {
            Ok(Outcome::Rows { len }) => cache.finish(len),
            _ => cache.invalidate(),
        }
    }
    engine_native::cprovince::UnitOrderWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        len: used,
        required,
    }
}

#[cfg(test)]
mod layout_tests;
