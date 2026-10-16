use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PortraitRow {
    pub character: u32,
    pub portraits: [usize; 2],
}

/// Roster slice and optional reuse inputs; defaults to the full unobserved roster.
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

/// Hourly manager census, including characters whose runtime map is empty. Character serialization
/// reaches the embedded portrait holder. Its entry array starts the record, followed by its count;
/// entries span 0x38 bytes. Share `part` of `parts` of the roster, contiguous in roster order. Rows
/// are sorted within the share; the caller merges shares by character. Entries may be reused only
/// under a complete mutation observation. Roster, identity, country and array validation still run
/// in their original order. An optional complete authenticated occurrence census from this same
/// callback supplies roster identities for reuse. Portrait headers and changed descriptors are
/// still acquired here. An absent or differently scoped census takes the ordinary independent
/// native path.
pub fn walk_portraits(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [PortraitRow],
    entries: &mut [crate::character_portrait_entry::Row],
    walk: PortraitWalk<'_>,
) -> crate::cprovince::UnitOrderWalk {
    walk_portraits_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(entries),
        walk,
    )
}

pub fn walk_portraits_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<PortraitRow>,
    entries: &mut Vec<crate::character_portrait_entry::Row<crate::capture_text::Owned>>,
    walk: PortraitWalk<'_>,
) -> crate::cprovince::UnitOrderWalk {
    walk_portraits_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(entries),
        walk,
    )
}

/// Sampled verification of reusable portrait entries. Each entry the cache's
/// previous walk certified whose character `sample` names, whose array no
/// observed writer touched since (`changes.revision`) and whose header reads
/// the same (another header is read afresh by the walk anyway) is read afresh
/// and compared with its descriptors at their arena position in `entries`.
/// Returns `[compared, differing]` characters; a read that fails compares
/// nothing.
pub fn sample_reused_portraits<T: crate::capture_text::Value>(
    memory: &impl Memory,
    cache: &crate::portrait_cache::Cache,
    entries: &[crate::character_portrait_entry::Row<T>],
    changes: &dyn crate::portrait_cache::Changes,
    sample: impl Fn(u32) -> bool,
) -> [u64; 2] {
    let mut counts = [0; 2];
    for key in cache.certified().filter(|key| sample(key.character)) {
        let Some(array) = key.pointer.checked_add(PORTRAIT_DATA) else {
            continue;
        };
        if changes.revision(array) != key.revision
            || read::<[u8; 16]>(memory, array) != Some(key.header)
        {
            continue;
        }
        let Some(a) = crate::cpdx_array::Array::from_header(
            key.header,
            crate::character_portrait_entry::STRIDE,
        ) else {
            continue;
        };
        let Some(held) = key
            .start
            .checked_add(a.count as usize)
            .and_then(|end| entries.get(key.start..end))
        else {
            continue;
        };
        let mut same = Some(true);
        for (n, held) in (0..a.count).zip(held) {
            let mut current = crate::character_portrait_entry::Row::default();
            let read = a
                .element(n, crate::character_portrait_entry::STRIDE)
                .ok_or("character portrait entry overflow")
                .and_then(|entry| {
                    crate::character_portrait_entry::read_buffer(memory, entry, &mut current)
                });
            if read.is_err() {
                same = None;
                break;
            }
            if current != *held {
                same = Some(false);
                break;
            }
        }
        if let Some(same) = same {
            counts[0] += 1;
            counts[1] += u64::from(!same);
        }
    }
    counts
}

fn walk_portraits_buffer<T: crate::capture_text::Value>(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, PortraitRow>,
    mut entries: crate::walk::Buffer<'_, crate::character_portrait_entry::Row<T>>,
    walk: PortraitWalk<'_>,
) -> crate::cprovince::UnitOrderWalk {
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
        // The native roster length bounds this walk; rows or entries that do
        // not fit the caller's buffers are counted for growth, not refused.
        let count = identities.map_or_else(|| roster.unwrap().count() as usize, <[_]>::len);
        let mut len = 0;
        let mut missing = 0;
        let first = (count * part / parts) as u32;
        let last = (count * (part + 1) / parts) as u32;
        // The complete roster and dynamic portrait-entry counts size storage.
        // Grow once from each original control.
        output.ensure((last - first) as usize)?;
        let mut hinted = first;
        for index in first..last {
            if let Some(identities) = identities {
                // Every pointer was authenticated by the current owned census.
                // A small lead overlaps independent header misses without
                // following any unvalidated native descriptor pointer.
                while hinted < last.min(index + 16) {
                    if let Some(address) = identities[hinted as usize]
                        .pointer
                        .checked_add(PORTRAIT_DATA)
                        && crate::memory::is_canonical_range(address, 16)
                    {
                        memory.prefetch(address, 16);
                    }
                    hinted += 1;
                }
            }
            let (pointer, character, header) = if let Some(identities) = identities {
                let identity = identities[index as usize];
                let address = identity
                    .pointer
                    .checked_add(PORTRAIT_DATA)
                    .ok_or("character portrait address overflow")?;
                let header = read(memory, address).ok_or("character portrait header unreadable")?;
                (identity.pointer, identity.character, header)
            } else {
                let (pointer, _, _) = roster.unwrap().pointer(memory, index)?;
                let character = identity_header(memory, base, pointer)?;
                (pointer, character, portrait_header(memory, pointer)?)
            };
            let a = crate::cpdx_array::Array::from_header(
                header,
                crate::character_portrait_entry::STRIDE,
            )
            .ok_or("character portrait array invalid")?;
            required = required
                .checked_add(a.count as usize)
                .ok_or("character portrait entry count overflow")?;
            entries.ensure(required)?;
            if len == output.len() || a.count as usize > entries.len().saturating_sub(used) {
                missing += 1;
                continue;
            }
            let start = used;
            let revision = observation
                .as_ref()
                .and_then(|(_, changes)| changes.revision(pointer + PORTRAIT_DATA));
            let key = crate::portrait_cache::Key {
                pointer,
                character,
                header,
                start,
                revision,
            };
            let hit = observation
                .as_ref()
                .is_some_and(|(cache, _)| cache.matches((index - first) as usize, key));
            let verify = observation
                .as_ref()
                .is_some_and(|(_, changes)| changes.verify());
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
                if hit && !verify {
                    used += 1;
                    continue;
                }
                // Only admitted entries enter this rolling four-entry window.
                // Hints acquire no values; keep the descriptor and scalar
                // string protocol in their original order.
                while hinted < a.count.min(n + 4) {
                    if let Some(entry) = a.element(hinted, crate::character_portrait_entry::STRIDE)
                    {
                        // These three fields cover the same cache lines as
                        // their 48-byte envelope; hint each envelope only once.
                        const HINT_BYTES: usize =
                            crate::character_portrait_entry::IDENTITY + msvc_string::STRIDE;
                        if crate::memory::is_canonical_range(entry, HINT_BYTES) {
                            crate::prefetch::span(entry, HINT_BYTES);
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
                if hit && let Some((cache, _)) = observation.as_mut() {
                    cache.counts.verified_entries += 1;
                    cache.counts.mismatches += u64::from(current != entries[used]);
                }
                entries[used] = current;
                used += 1;
            }
            if let Some((cache, changes)) = observation.as_mut() {
                let after = changes.revision(pointer + PORTRAIT_DATA);
                // Never certify entries observed across an overlapping writer.
                cache.remember(
                    (index - first) as usize,
                    crate::portrait_cache::Key {
                        revision: revision.filter(|r| Some(*r) == after),
                        ..key
                    },
                );
                if hit && after != revision {
                    return Err("character portrait mutated during acquisition");
                }
            }
            output[len] = PortraitRow {
                character,
                portraits: [start, a.count as usize],
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
            // A failed/short walk may have overwritten a later cached arena range.
            _ => cache.invalidate(),
        }
    }
    crate::cprovince::UnitOrderWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        len: used,
        required,
    }
}
