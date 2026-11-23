
use engine_native_memory::{Memory, read, walk::Outcome};
use crate::cstrategic_resource_database;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29ce150);

pub const TOKEN_ID: usize = 0x08;
pub const ACTIVE: usize = 0x10;
pub const INDEX: usize = 0xd8;
pub const FACTORY_COST: usize = 0xe0;
pub const STATE_ADDITIVE_KEY: usize = 0xf0;
pub const STATE_FACTOR_KEY: usize = 0xf4;
pub const TEMPORARY_KEY: usize = 0xf8;
pub const MODIFIER_KEY: usize = 0xfc;
pub const CONSTRUCTOR_RVA: Rva = Rva(0xc1ff60);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub token: u32,
    pub active: u8,
    pub factory_cost: i64,
    pub(crate) address: usize,
}

pub fn walk(memory: &impl Memory, base: usize, output: &mut Vec<Row>) -> Outcome {
    walk_buffer(memory, base, engine_native_memory::walk::Buffer::Growing(output))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let database = cstrategic_resource_database::Reader::new(memory, base)?;
        let mut len = 0;
        let mut unread_rows = 0;
        output.ensure(database.count().saturating_sub(1) as usize)?;
        for index in 1..database.count() {
            let (_, row) = database.at(index)?;
            if len == output.len() {
                unread_rows += 1;
                continue;
            }
            output[len] = row;
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

pub(crate) fn row(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("resource address overflow")
    };
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("resource descriptor vtable mismatch");
    }
    let id = read::<i32>(memory, at(INDEX)?)
        .filter(|&id| id > 0)
        .ok_or("resource index unreadable or invalid")? as u32;
    Ok(Row {
        id,
        token: read::<u32>(memory, at(TOKEN_ID)?).ok_or("resource token unreadable")?,
        active: read::<u8>(memory, at(ACTIVE)?).ok_or("resource active flag unreadable")?,
        factory_cost: read::<i64>(memory, at(FACTORY_COST)?)
            .ok_or("resource factory cost unreadable")?,
        address: pointer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccountry_resources::test_support::Fixture;

    #[test]
    fn native_descriptor_identity_and_precision_need_no_lexer_or_name_arena() {
        let mut fixture = Fixture::new();
        fixture.put(0xd000 + ACTIVE, &[0]);
        fixture.u32(0xd000 + TOKEN_ID, 0xfedc_ba98);
        fixture.u32(0xe200, 0xfedc_ba98);
        let mut rows = vec![Row::default(); 1];
        assert_eq!(walk(&fixture, 0, &mut rows), Outcome::Rows { len: 1 });
        assert_eq!(
            (rows[0].id, rows[0].token, rows[0].active),
            (1, 0xfedc_ba98, 0)
        );
        assert_eq!(rows[0].factory_cost, 4_294_967_300);
        assert_eq!(rows[0].address, 0xd000);
    }

    #[test]
    fn database_key_mismatch_is_unavailable_even_without_lexical_resolution() {
        let mut fixture = Fixture::new();
        fixture.u32(0xe200, 20);
        let result = walk(&fixture, 0, &mut vec![Row::default(); 1]);
        assert!(matches!(result, Outcome::Unavailable { .. }));
    }
}
