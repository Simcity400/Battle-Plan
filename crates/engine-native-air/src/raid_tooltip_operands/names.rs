use super::*;
use engine_native::cname_group_member as member;
use engine_native_memory::msvc_string;
use engine_sites::Rva;

pub struct StoredString {
    pub len: usize,
    source: usize,
    inline: Option<[u8; 16]>,
    whole: bool,
}

impl StoredString {
    pub fn read(memory: &impl Memory, at: usize) -> Result<Self, NameReadError> {
        Self::parse(memory, at, None)
    }

    pub(crate) fn read_whole(memory: &impl Memory, at: usize) -> Result<Self, NameReadError> {
        let header = engine_native_memory::read::<[u8; msvc_string::STRIDE]>(memory, at);
        Ok(Self {
            whole: true,
            ..Self::parse(memory, at, header)?
        })
    }

    pub(crate) fn from_whole_header(
        memory: &impl Memory,
        at: usize,
        header: [u8; msvc_string::STRIDE],
    ) -> Result<Self, NameReadError> {
        Ok(Self {
            whole: true,
            ..Self::parse(memory, at, Some(header))?
        })
    }

    fn parse(
        memory: &impl Memory,
        at: usize,
        header: Option<[u8; msvc_string::STRIDE]>,
    ) -> Result<Self, NameReadError> {
        use NameReadError::{Header, NullPointer, Overflow, Unreadable};
        let word = |offset: usize| match header {
            Some(header) => Ok(u64::from_le_bytes(
                header[offset..offset + 8].try_into().unwrap(),
            )),
            None => field::<u64>(memory, at, offset).map_err(|_| Unreadable),
        };
        let len = word(msvc_string::LENGTH)?;
        let capacity = word(msvc_string::CAPACITY)?;
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err(Header);
        }
        let (source, inline) = if capacity == msvc_string::INLINE_CAPACITY {
            (at, header.map(|header| header[..16].try_into().unwrap()))
        } else {
            (usize::try_from(word(0)?).map_err(|_| Overflow)?, None)
        };
        if source == 0 {
            return Err(NullPointer);
        }
        let len = usize::try_from(len).map_err(|_| Overflow)?;
        let copied = len.checked_add(1).ok_or(Overflow)?;
        source.checked_add(copied).ok_or(Overflow)?;
        Ok(Self {
            len,
            source,
            inline,
            whole: false,
        })
    }

    pub fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), NameReadError> {
        use NameReadError::{Overflow, Terminator, Unreadable};
        let output = output.get_mut(..self.len).ok_or(Overflow)?;
        if let Some(inline) = &self.inline {
            if inline[self.len] != 0 {
                return Err(Terminator);
            }
            output.copy_from_slice(&inline[..self.len]);
            return Ok(());
        }
        if self.whole {
            let mut terminator = [0_u8; 1];
            let copied = memory.copy_scattered(
                [self.source, self.source + self.len],
                &mut [&mut *output, &mut terminator],
            );
            return match copied {
                2 if terminator[0] == 0 => Ok(()),
                2 => Err(Terminator),
                0 => match field::<u8>(memory, self.source, self.len) {
                    Ok(0) | Err(_) => Err(Unreadable),
                    Ok(_) => Err(Terminator),
                },
                _ => Err(Unreadable),
            };
        }
        if field::<u8>(memory, self.source, self.len).map_err(|_| Unreadable)? != 0 {
            return Err(Terminator);
        }
        if !engine_native_memory::read_slice(memory, self.source, output) {
            return Err(Unreadable);
        }
        Ok(())
    }
}

fn string(
    memory: &impl Memory,
    at: usize,
    kind: u32,
    sink: &mut Sink<'_>,
) -> Result<(), NameReadError> {
    use NameReadError::{Overflow, Terminator, Unreadable};
    let StoredString { len, source, .. } = StoredString::read(memory, at)?;
    let raw_len = i64::try_from(len).map_err(|_| Overflow)?;
    let copied = len.checked_add(1).ok_or(Overflow)?;
    let count = len.div_ceil(8).checked_add(1).ok_or(Overflow)?;
    let end = sink.required.checked_add(count).ok_or(Overflow)?;
    let ordinal = sink
        .ordinal
        .checked_add(u32::try_from(count).map_err(|_| Overflow)?)
        .ok_or(Overflow)?;
    if end <= sink.rows.len() {
        // SAFETY: count rows hold at least len+1 bytes; only a successful
        // copy's initialized prefix is read, and every Row field accepts all bits.
        unsafe {
            let rows = sink.rows.as_mut_ptr().add(sink.required);
            {
                let scratch = std::slice::from_raw_parts_mut(
                    rows.cast::<std::mem::MaybeUninit<u8>>(),
                    copied,
                );
                let bytes =
                    engine_native_memory::memory::read_slice_uninit(memory, source, scratch).ok_or(Unreadable)?;
                if bytes[len] != 0 {
                    return Err(Terminator);
                }
            }
            for index in (0..count - 1).rev() {
                let mut bytes = [0; 8];
                std::ptr::copy_nonoverlapping(
                    rows.cast::<u8>().add(index * 8),
                    bytes.as_mut_ptr(),
                    (len - index * 8).min(8),
                );
                rows.add(index + 1).write(Row {
                    raid: sink.raid,
                    ordinal: sink.ordinal + index as u32 + 1,
                    kind,
                    key: index as u32,
                    raw: i64::from_le_bytes(bytes),
                });
            }
            rows.write(Row {
                raid: sink.raid,
                ordinal: sink.ordinal,
                kind,
                key: u32::MAX,
                raw: raw_len,
            });
        }
        sink.required = end;
        sink.ordinal = ordinal;
        return Ok(());
    }
    if field::<u8>(memory, source, len).map_err(|_| Unreadable)? != 0 {
        return Err(Terminator);
    }
    sink.push(kind, u32::MAX, raw_len).map_err(|_| Overflow)?;
    for offset in (0..len).step_by(8) {
        let mut bytes = [0; 8];
        if !engine_native_memory::read_slice(memory, source + offset, &mut bytes[..(len - offset).min(8)]) {
            return Err(Unreadable);
        }
        sink.push(kind, (offset / 8) as u32, i64::from_le_bytes(bytes))
            .map_err(|_| Overflow)?;
    }
    Ok(())
}

pub(super) fn capture(
    m: &impl Memory,
    base: usize,
    army: Option<usize>,
    wing: Option<usize>,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    sink.push(NAME, ARMY_RESOLVED, i64::from(army.is_some()))?;
    sink.push(NAME, WING_RESOLVED, i64::from(wing.is_some()))?;
    let reference = if let Some(p) = army {
        field(m, p, 0x32c)?
    } else if let Some(p) = wing {
        field(m, p, 0xa2c)?
    } else {
        return Ok(());
    };
    sink.push(NAME, UNIT_RAID_REFERENCE, reference as i64)?;
    sink.push(
        NAME,
        UNIT_RAID_RESOLVED,
        i64::from(creference_registry::resolve(m, base, reference)?.is_some()),
    )?;
    let Some(army) = army else {
        return independent_string(m, wing.unwrap() + 0x998, WING_NAME, sink);
    };
    let p = field::<u64>(m, army, 0x340)? as usize;
    member(m, base, p, NAME, NAME_GROUP, CUSTOM_NAME, None, sink)
}

fn independent_string(
    m: &impl Memory,
    at: usize,
    kind: u32,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let first = (sink.required, sink.ordinal);
    if let Err(error) = string(m, at, kind, sink) {
        (sink.required, sink.ordinal) = first;
        sink.push(kind, u32::MAX, error.code())?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the scalar, group, custom and cached kinds of one member are separate fields, and the sink is caller-owned"
)]
fn member(
    m: &impl Memory,
    base: usize,
    p: usize,
    scalar: u32,
    group_kind: u32,
    custom_kind: u32,
    cached_kind: Option<u32>,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    context::authenticate(m, base, p, member::VTABLE_RVA)?;
    if let Some(kind) = cached_kind {
        independent_string(m, p + member::CACHED_NAME, kind, sink)?;
    }
    let ty = field::<i32>(m, p, member::TYPE)?;
    sink.push(scalar, member::TYPE as u32, i64::from(ty))?;
    sink.push(
        scalar,
        member::ORDER as u32,
        i64::from(field::<i32>(m, p, member::ORDER)?),
    )?;
    let (_, len, ordered, programmatic) = member::custom(m, base, p)?;
    sink.push(scalar, member::FLAG_A as u32, i64::from(ordered))?;
    sink.push(scalar, member::FLAG_B as u32, i64::from(programmatic))?;
    sink.push(scalar, member::OVERRIDE_NAME_LENGTH as u32, len as i64)?;
    if len != 0 {
        return if programmatic == 0 {
            string(m, p + member::OVERRIDE_NAME, custom_kind, sink).map_err(NameReadError::reason)
        } else {
            Ok(())
        };
    }
    let group = match ty {
        0 => {
            let reference = field::<u64>(m, p, 0x58)?;
            sink.push(scalar, 0x58, reference as i64)?;
            let Some(template) = creference_registry::resolve(m, base, reference)? else {
                return Ok(());
            };
            context::authenticate(
                m,
                base,
                template,
                engine_native::creferenced_division_template::VTABLE_RVA,
            )?;
            field::<u64>(m, template, 0x1d0)? as usize
        }
        1 => {
            let mut variant = field::<u64>(m, p, 0x50)? as usize;
            if variant == 0 {
                let ship = field::<u64>(m, p, 0x40)? as usize;
                if ship == 0 {
                    return Ok(());
                }
                context::authenticate(m, base, ship, engine_native::cship::VTABLE_RVA)?;
                let entries = engine_native::cequipment_variant::pool::entries(m, base, ship + 0x40)?;
                let mut best = None;
                for i in 0..entries.count {
                    let at = entries
                        .element(i, 16)
                        .ok_or("raid source ship equipment index invalid")?;
                    let amount = field::<i64>(m, at, 8)?;
                    if best.is_none_or(|(_, old)| amount > old) {
                        best = Some((field::<u64>(m, at, 0)? as usize, amount));
                    }
                }
                variant = best.map_or(0, |(p, _)| p);
            }
            if variant == 0 {
                return Ok(());
            }
            context::authenticate(m, base, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA)?;
            field::<u64>(m, variant, 0x480)? as usize
        }
        _ => return Ok(()),
    };
    if group != 0 {
        context::authenticate(m, base, group, Rva(0x029b_3cd8))?;
        string(m, group + 8, group_kind, sink).map_err(NameReadError::reason)?;
    }
    Ok(())
}

pub(super) fn source(
    m: &impl Memory,
    base: usize,
    raid: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let source = raid + craid_instance::SOURCE;
    if field::<u8>(m, source, 4)? == 0 || !matches!(field::<u32>(m, source, 0)?, 3 | 4) {
        return Ok(());
    }
    let reference = field::<u64>(m, source, 0x10)?;
    sink.push(REFERENCES, 2, reference as i64)?;
    let ship = creference_registry::resolve(m, base, reference)?;
    sink.push(NAME, SOURCE_SHIP_RESOLVED, i64::from(ship.is_some()))?;
    if let Some(ship) = ship {
        context::authenticate(m, base, ship, engine_native::cship::VTABLE_RVA)?;
        member(
            m,
            base,
            field::<u64>(m, ship, 0x818)? as usize,
            SOURCE_NAME,
            SOURCE_NAME_GROUP,
            SOURCE_CUSTOM_NAME,
            Some(SOURCE_CACHED_NAME),
            sink,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raid_wing_name_capture_preserves_sso_heap_and_live_sized_demand() {
        for text in [
            b"User $name$".to_vec(),
            b"Generated Squadron".to_vec(),
            vec![b'x'; 4097],
            vec![b'A', 0xff, 0, b'B'],
            Vec::new(),
        ] {
            let wing = 64usize;
            let name = wing + 0x998;
            let heap = 4096usize;
            let mut bytes = vec![0; heap + text.len() + 1];
            let source = if text.len() <= 15 {
                name
            } else {
                bytes[name..name + 8].copy_from_slice(&(heap as u64).to_le_bytes());
                heap
            };
            bytes[source..source + text.len()].copy_from_slice(&text);
            bytes[name + 16..name + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
            bytes[name + 24..name + 32].copy_from_slice(&(text.len().max(15) as u64).to_le_bytes());
            let memory = engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut rows = vec![Row::default(); 5 + text.len().div_ceil(8)];
            let mut sink = Sink {
                rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
                required: 0,
                raid: 1,
                ordinal: 0,
            };
            let previous = engine_native_memory::read_counts::ReadCounts::take_current();
            let counted = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            capture(&counted, 0, None, Some(wing), &mut sink).unwrap();
            let counts = engine_native_memory::read_counts::ReadCounts::take_current();
            previous.add_current();
            assert_eq!(counts.copy_calls, if text.len() <= 15 { 4 } else { 5 });
            assert_eq!(sink.required, 5 + text.len().div_ceil(8));
            assert_eq!(rows[4].key, u32::MAX);
            let recorded: Vec<_> = rows[5..]
                .iter()
                .flat_map(|row| {
                    assert_eq!(row.kind, WING_NAME);
                    row.raw.to_le_bytes()
                })
                .take(text.len())
                .collect();
            assert_eq!(recorded, text);
            for (index, row) in rows[5..].iter().enumerate() {
                assert_eq!(row.key, index as u32);
            }
            let mut sink = Sink {
                rows: engine_native_memory::walk::Buffer::Count,
                required: 0,
                raid: 1,
                ordinal: 0,
            };
            capture(&memory, 0, None, Some(wing), &mut sink).unwrap();
            assert_eq!(sink.required, rows.len());
        }
    }
    #[test]
    fn raid_wing_name_failures_keep_one_named_operand_and_no_partial_chunks() {
        let wing = 64usize;
        let name = wing + 0x998;
        for (len, capacity, pointer, terminated, expected) in [
            (16u64, 15u64, 4096u64, true, 1),
            (0, 14, 4096, true, 1),
            (3, 31, 0, true, 2),
            (3, 31, 4096, false, 3),
            (3, 31, u64::MAX - 1, true, 4),
            (3, 31, 5000, true, 5),
        ] {
            let mut bytes = vec![0; 4100];
            bytes[name..name + 8].copy_from_slice(&pointer.to_le_bytes());
            bytes[name + 16..name + 24].copy_from_slice(&len.to_le_bytes());
            bytes[name + 24..name + 32].copy_from_slice(&capacity.to_le_bytes());
            bytes[4099] = u8::from(!terminated);
            let mut rows = vec![Row::default(); 9];
            let mut sink = Sink {
                rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
                required: 0,
                raid: 1,
                ordinal: 0,
            };
            capture(
                &engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                None,
                Some(wing),
                &mut sink,
            )
            .unwrap();
            assert_eq!(sink.required, 5);
            assert_eq!(
                (rows[4].kind, rows[4].key, rows[4].raw),
                (WING_NAME, u32::MAX, -expected)
            );
            assert!(!wing_name_error(-expected).contains("unsupported"));
        }
        struct ShortCopy;
        impl Memory for ShortCopy {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let wing = 64usize;
                if address == wing + 0xa2c {
                    output.fill(0);
                    return true;
                }
                if address == wing + 0x9a8 {
                    output.copy_from_slice(&9u64.to_le_bytes());
                    return true;
                }
                if address == wing + 0x9b0 {
                    output.copy_from_slice(&15u64.to_le_bytes());
                    return true;
                }
                if address == wing + 0x998 {
                    output.fill(b'a');
                    return false;
                }
                false
            }
        }
        let mut rows = vec![Row::default(); 9];
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
            required: 0,
            raid: 1,
            ordinal: 0,
        };
        capture(&ShortCopy, 0, None, Some(wing), &mut sink).unwrap();
        assert_eq!(sink.required, 5);
        assert_eq!(
            (rows[4].key, rows[4].raw),
            (u32::MAX, NameReadError::Unreadable.code())
        );
    }
    #[test]
    fn chunks_preserve_utf8_bytes_and_report_full_demand_without_truncation() {
        let mut bytes = vec![0; 128];
        let text = "éabcdefghi".as_bytes();
        bytes[16..16 + text.len()].copy_from_slice(text);
        bytes[32..40].copy_from_slice(&(text.len() as u64).to_le_bytes());
        bytes[40..48].copy_from_slice(&15u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 3];
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
            required: 0,
            raid: 1,
            ordinal: 0,
        };
        string(&memory, 16, CUSTOM_NAME, &mut sink).unwrap();
        assert_eq!(sink.required, 3);
        assert_eq!(rows[0].raw, text.len() as i64);
        assert_eq!(&rows[1].raw.to_le_bytes(), &text[..8]);
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Count,
            required: 0,
            raid: 1,
            ordinal: 0,
        };
        string(&memory, 16, CUSTOM_NAME, &mut sink).unwrap();
        assert_eq!(sink.required, 3);
        bytes[16 + text.len()] = 1;
        assert!(
            string(
                &engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                16,
                CUSTOM_NAME,
                &mut sink
            )
            .is_err()
        );
    }

    #[test]
    fn source_member_records_the_cached_display_name_independently() {
        let member_at = 64usize;
        let heap = 1024usize;
        let image = |cache: &[u8], cache_valid: bool| {
            let mut bytes = vec![0u8; 2048];
            bytes[member_at..member_at + 8]
                .copy_from_slice(&(member::VTABLE_RVA.0 as u64).to_le_bytes());
            bytes[member_at + member::TYPE..member_at + member::TYPE + 4]
                .copy_from_slice(&1i32.to_le_bytes());
            let custom = member_at + member::OVERRIDE_NAME;
            bytes[custom..custom + 3].copy_from_slice(b"Old");
            bytes[custom + 16..custom + 24].copy_from_slice(&3u64.to_le_bytes());
            bytes[custom + 24..custom + 32].copy_from_slice(&15u64.to_le_bytes());
            bytes[member_at + member::FLAG_B] = 1;
            let cached = member_at + member::CACHED_NAME;
            let source = if cache.len() <= 15 {
                cached
            } else {
                bytes[cached..cached + 8].copy_from_slice(&(heap as u64).to_le_bytes());
                heap
            };
            bytes[source..source + cache.len()].copy_from_slice(cache);
            bytes[cached + 16..cached + 24].copy_from_slice(&(cache.len() as u64).to_le_bytes());
            let capacity = if cache_valid { cache.len().max(15) } else { 0 };
            bytes[cached + 24..cached + 32].copy_from_slice(&(capacity as u64).to_le_bytes());
            bytes
        };
        let capture = |bytes: &[u8]| {
            let memory = engine_native_memory::memory::SliceMemory { base: 0, bytes };
            let mut rows = vec![Row::default(); 16];
            let mut sink = Sink {
                rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
                required: 0,
                raid: 7,
                ordinal: 0,
            };
            member(
                &memory,
                0,
                member_at,
                SOURCE_NAME,
                SOURCE_NAME_GROUP,
                SOURCE_CUSTOM_NAME,
                Some(SOURCE_CACHED_NAME),
                &mut sink,
            )
            .unwrap();
            let required = sink.required;
            rows.truncate(required);
            rows
        };
        for text in [&b"HMS Hood"[..], b"Schlachtschiff Bismarck", b""] {
            let rows = capture(&image(text, true));
            let cached: Vec<_> = rows
                .iter()
                .filter(|row| row.kind == SOURCE_CACHED_NAME)
                .collect();
            assert_eq!(
                (cached[0].key, cached[0].raw),
                (u32::MAX, text.len() as i64)
            );
            let bytes: Vec<u8> = cached[1..]
                .iter()
                .enumerate()
                .flat_map(|(index, row)| {
                    assert_eq!(row.key, index as u32);
                    row.raw.to_le_bytes()
                })
                .take(text.len())
                .collect();
            assert_eq!(bytes, text);
            assert_eq!(cached.len(), 1 + text.len().div_ceil(8));
            assert!(rows.iter().any(|row| row.kind == SOURCE_NAME
                && row.key == member::FLAG_B as u32
                && row.raw == 1));
            for (ordinal, row) in rows.iter().enumerate() {
                assert_eq!(row.ordinal, ordinal as u32);
            }
        }
        let rows = capture(&image(b"Hood", false));
        let cached: Vec<_> = rows
            .iter()
            .filter(|row| row.kind == SOURCE_CACHED_NAME)
            .collect();
        assert_eq!(cached.len(), 1);
        assert_eq!(
            (cached[0].key, cached[0].raw),
            (u32::MAX, NameReadError::Header.code())
        );
        assert!(rows.iter().any(|row| row.kind == SOURCE_NAME
            && row.key == member::OVERRIDE_NAME_LENGTH as u32
            && row.raw == 3));
    }
}
