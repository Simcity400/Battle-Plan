use super::*;
use engine_native_parts::cequipment_type as eq;
use crate::raid_database as db;

fn pointer(m: &impl Memory, at: usize, offset: usize) -> Result<usize, &'static str> {
    Ok(field::<u64>(m, at, offset)? as usize)
}
fn map(
    m: &impl Memory,
    base: usize,
    at: usize,
    kind: u32,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let sink = std::cell::RefCell::new(sink);
    let mut previous = 0;
    db::equipment_map(
        m,
        at,
        "raid tooltip operand unreadable",
        |count| {
            sink.borrow_mut().push(
                kind,
                0,
                i64::try_from(count).map_err(|_| "raid equipment order count overflow")?,
            )
        },
        |node| {
            let equipment = pointer(m, node, db::NODE_EQUIPMENT)?;
            if equipment <= previous {
                return Err("raid equipment pointer order invalid");
            }
            context::authenticate(m, base, equipment, eq::VTABLE_RVA)?;
            let token = field::<u32>(m, equipment, eq::TOKEN)?;
            if token == 0 {
                return Err("raid equipment order token null");
            }
            previous = equipment;
            sink.borrow_mut().push(kind, token, 0)
        },
    )
}
fn subunits(m: &impl Memory, base: usize, sink: &mut Sink<'_>) -> Result<(), &'static str> {
    use engine_native_parts::csub_unit_definition as subunit;
    let array = subunit::Database::read(m, base)?.vector();
    sink.raid = 0;
    sink.ordinal = 0;
    for index in 0..array.count {
        let p = pointer(
            m,
            array.element(index, 8).ok_or("subunit index overflow")?,
            0,
        )?;
        context::authenticate(
            m,
            base,
            p,
            if index == 0 {
                subunit::NULL_VTABLE_RVA
            } else {
                subunit::VTABLE_RVA
            },
        )?;
        let token = if index == 0 {
            0
        } else {
            field(m, p, subunit::TOKEN)?
        };
        sink.push(
            SUBUNIT_ACTIVE,
            token,
            i64::from(field::<u8>(m, p, subunit::IS_REAL)?),
        )?;
    }
    Ok(())
}

pub(super) fn capture(
    m: &impl Memory,
    base: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    subunits(m, base, sink)?;
    let db = engine_sites::at(base, db::SINGLETON_RVA).ok_or("raid database withheld")?;
    let db = pointer(m, db, 0)?;
    let types = Array::read(m, db + db::DEFINITIONS, 8).ok_or("raid definitions header invalid")?;
    for i in 0..types.count {
        let p = pointer(
            m,
            types
                .element(i, 8)
                .ok_or("raid definition index overflow")?,
            0,
        )?;
        context::authenticate(m, base, p, db::TYPE_VTABLE_RVA)?;
        let token = field::<u32>(m, p, db::TYPE_TOKEN)?;
        if token == 0 {
            return Err("raid definition has null type token");
        }
        sink.raid = definition_key(token);
        sink.ordinal = 0;
        map(m, base, p + db::ESSENTIAL_EQUIPMENT, ESSENTIAL_ORDER, sink)?;
        map(
            m,
            base,
            p + db::ADDITIONAL_EQUIPMENT,
            ADDITIONAL_ORDER,
            sink,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    #[test]
    fn global_subunit_discriminators_keep_null_fallback_and_stored_bytes() {
        use engine_native_parts::csub_unit_definition as definition;
        struct Fixture(std::collections::BTreeMap<usize, u8>);
        impl Memory for Fixture {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                for (i, b) in out.iter_mut().enumerate() {
                    let Some(value) = self.0.get(&(at + i)) else {
                        return false;
                    };
                    *b = *value;
                }
                true
            }
        }
        let mut bytes = std::collections::BTreeMap::new();
        let mut put = |at, data: &[u8]| {
            for (i, b) in data.iter().enumerate() {
                bytes.insert(at + i, *b);
            }
        };
        put(definition::DATABASE_RVA.0, &0x1000u64.to_le_bytes());
        put(0x1040, &0x2000u64.to_le_bytes());
        put(0x1048, &2u32.to_le_bytes());
        put(0x104c, &2u32.to_le_bytes());
        for (index, p, vt, token, active) in [
            (0, 0x3000, definition::NULL_VTABLE_RVA.0, 0, 0),
            (1, 0x4000, definition::VTABLE_RVA.0, 91, 255),
        ] {
            put(0x2000 + index * 8, &(p as u64).to_le_bytes());
            put(p, &(vt as u64).to_le_bytes());
            put(p + definition::TOKEN, &(token as u32).to_le_bytes());
            put(p + definition::IS_REAL, &[active]);
        }
        let mut rows = vec![Row::default(); 2];
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
            required: 0,
            raid: 0,
            ordinal: 0,
        };
        let mut fixture = Fixture(bytes);
        subunits(&fixture, 0, &mut sink).unwrap();
        assert_eq!(
            sink.rows.iter().map(|r| (r.key, r.raw)).collect::<Vec<_>>(),
            [(0, 0), (91, 255)]
        );
        fixture.0.remove(&(0x4000 + definition::IS_REAL));
        assert!(subunits(&fixture, 0, &mut sink).is_err());
    }
    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    #[test]
    fn pointer_order_is_not_token_order_and_truncation_keeps_exact_demand() {
        let mut bytes = vec![0; 0x1000];
        let (map_at, head, first, second, a, b) = (0x10, 0x100, 0x200, 0x300, 0x500, 0x600);
        word(&mut bytes, map_at, head as u64);
        word(&mut bytes, map_at + 8, 2);
        bytes[head + 0x19] = 1;
        word(&mut bytes, head, first as u64);
        for (node, left, parent, right, equipment, token) in [
            (first, head, head, second, a, 900),
            (second, head, first, head, b, 7),
        ] {
            word(&mut bytes, node, left as u64);
            word(&mut bytes, node + 8, parent as u64);
            word(&mut bytes, node + 0x10, right as u64);
            word(&mut bytes, node + 0x20, equipment as u64);
            word(&mut bytes, equipment, eq::VTABLE_RVA.0 as u64);
            word(&mut bytes, equipment + eq::TOKEN, token);
        }
        let run = |bytes: &[u8], rows: &mut Vec<Row>| {
            let mut sink = Sink {
                rows: engine_native_memory::walk::Buffer::Growing(rows),
                required: 0,
                raid: definition_key(101),
                ordinal: 0,
            };
            map(
                &SliceMemory { base: 0, bytes },
                0,
                map_at,
                ESSENTIAL_ORDER,
                &mut sink,
            )
            .map(|()| sink.required)
        };
        let mut rows = vec![Row::default(); 3];
        assert_eq!(run(&bytes, &mut rows), Ok(3));
        assert_eq!(
            rows.iter().map(|r| (r.key, r.raw)).collect::<Vec<_>>(),
            [(0, 2), (900, 0), (7, 0)]
        );
        assert_eq!(run(&bytes, &mut Vec::new()), Ok(3));
        word(&mut bytes, second + 0x20, a as u64);
        assert_eq!(
            run(&bytes, &mut rows),
            Err("raid equipment pointer order invalid")
        );
        word(&mut bytes, second + 0x20, b as u64);
        word(&mut bytes, map_at + 8, 1);
        assert_eq!(
            run(&bytes, &mut rows),
            Err("raid equipment order exceeds stored count")
        );
        word(&mut bytes, map_at + 8, 3);
        assert_eq!(
            run(&bytes, &mut rows),
            Err("raid equipment order ended early")
        );
        bytes[head + 0x19] = 0;
        assert_eq!(
            run(&bytes, &mut rows),
            Err("raid equipment order sentinel invalid")
        );
    }
    #[test]
    fn empty_map_is_witnessed_and_short_reads_are_never_empty() {
        let mut bytes = vec![0; 0x200];
        word(&mut bytes, 0x10, 0x100);
        word(&mut bytes, 0x100, 0x100);
        bytes[0x119] = 1;
        let mut rows = vec![Row::default(); 1];
        let mut sink = Sink {
            rows: engine_native_memory::walk::Buffer::Growing(&mut rows),
            required: 0,
            raid: definition_key(11),
            ordinal: 0,
        };
        assert_eq!(
            map(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x10,
                ADDITIONAL_ORDER,
                &mut sink
            ),
            Ok(())
        );
        assert_eq!(sink.required, 1);
        assert_eq!(sink.rows[0].raw, 0);
        assert!(
            map(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x110]
                },
                0,
                0x10,
                ADDITIONAL_ORDER,
                &mut sink
            )
            .is_err()
        );
    }
}
