//! 1414536D0 takes the army before the wing. Never copy the cached name
//! (+60) or a programmatic override (+88): refresh 1409B4C40 localises and
//! loses their provenance. Preserve the authored group key and user bytes.
use super::*;
use crate::{cname_group_member as member, msvc_string};

fn string(
    memory: &impl Memory,
    at: usize,
    kind: u32,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let len = field::<u64>(memory, at, msvc_string::LENGTH)?;
    let capacity = field::<u64>(memory, at, msvc_string::CAPACITY)?;
    if len > capacity || capacity < 15 {
        return Err("raid name string header invalid");
    }
    let source = if capacity == 15 {
        at
    } else {
        field::<u64>(memory, at, 0)? as usize
    };
    let len = usize::try_from(len).map_err(|_| "raid name length overflow")?;
    if field::<u8>(memory, source, len)? != 0 {
        return Err("raid name terminator invalid");
    }
    sink.push(kind, u32::MAX, len as i64)?;
    for offset in (0..len).step_by(8) {
        let mut bytes = [0; 8];
        let length = (len - offset).min(8);
        let at = source
            .checked_add(offset)
            .ok_or("raid name address overflow")?;
        if !crate::read_slice(memory, at, &mut bytes[..length]) {
            return Err("raid name bytes unreadable");
        }
        sink.push(
            kind,
            u32::try_from(offset / 8).map_err(|_| "raid name chunk index overflow")?,
            i64::from_le_bytes(bytes),
        )?;
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
    // 1414545D0, 140BEC560, 140F48100 test resolution, not equality to this raid.
    sink.push(NAME, UNIT_RAID_REFERENCE, reference as i64)?;
    sink.push(
        NAME,
        UNIT_RAID_RESOLVED,
        i64::from(creference_registry::resolve(m, base, reference)?.is_some()),
    )?;
    let Some(army) = army else {
        // 140F51FB0 stores both player rename and generated 140F48F90 output
        // here; no distinction or original random choice survives.
        return sink.push(
            NAME,
            WING_NAME_LENGTH,
            field::<u64>(m, wing.unwrap(), 0x9a8)? as i64,
        );
    };
    let p = field::<u64>(m, army, 0x340)? as usize; // CArmy::GetName 140C72F00.
    member(m, base, p, NAME, NAME_GROUP, CUSTOM_NAME, sink)
}

fn member(
    m: &impl Memory,
    base: usize,
    p: usize,
    scalar: u32,
    group_kind: u32,
    custom_kind: u32,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    context::authenticate(m, base, p, member::VTABLE_RVA)?;
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
            string(m, p + member::OVERRIDE_NAME, custom_kind, sink)
        } else {
            Ok(())
        };
    }
    // 1409B7290 selects a naming group; the stored group key witnesses the
    // selected template/variant, without recording a renderer-local name.
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
                crate::creferenced_division_template::VTABLE_RVA,
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
                context::authenticate(m, base, ship, crate::cship::VTABLE_RVA)?;
                let entries = crate::cequipment_variant_pool::entries(m, base, ship + 0x40)?;
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
            context::authenticate(m, base, variant, crate::cequipment_variant::VTABLE_RVA)?;
            field::<u64>(m, variant, 0x480)? as usize
        }
        _ => return Ok(()), // Consumer names unsupported member types.
    };
    if group != 0 {
        context::authenticate(m, base, group, 0x0291_f568)?; // CNameGroup::vf4.
        string(m, group + 8, group_kind, sink)?; // raw key, serializer 140B977F0.
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
        context::authenticate(m, base, ship, crate::cship::VTABLE_RVA)?;
        // CShip::GetName 140C29A20, member constructor 1409B2FC0.
        member(
            m,
            base,
            field::<u64>(m, ship, 0x818)? as usize,
            SOURCE_NAME,
            SOURCE_NAME_GROUP,
            SOURCE_CUSTOM_NAME,
            sink,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_preserve_utf8_bytes_and_report_full_demand_without_truncation() {
        let mut bytes = vec![0; 128];
        let text = "éabcdefghi".as_bytes();
        bytes[16..16 + text.len()].copy_from_slice(text);
        bytes[32..40].copy_from_slice(&(text.len() as u64).to_le_bytes());
        bytes[40..48].copy_from_slice(&15u64.to_le_bytes());
        let memory = crate::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = [Row::default(); 3];
        let mut sink = Sink {
            rows: &mut rows,
            required: 0,
            raid: 1,
            ordinal: 0,
        };
        string(&memory, 16, CUSTOM_NAME, &mut sink).unwrap();
        assert_eq!(sink.required, 3);
        assert_eq!(rows[0].raw, text.len() as i64);
        assert_eq!(&rows[1].raw.to_le_bytes(), &text[..8]);
        let mut sink = Sink {
            rows: &mut [],
            required: 0,
            raid: 1,
            ordinal: 0,
        };
        string(&memory, 16, CUSTOM_NAME, &mut sink).unwrap();
        assert_eq!(sink.required, 3);
        bytes[16 + text.len()] = 1;
        assert!(
            string(
                &crate::memory::SliceMemory {
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
}
