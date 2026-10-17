//! CStrategicResource descriptors. Ordinals are engine indices, not dense wire slots.

use crate::{Memory, cstrategic_resource_database, read, walk::Outcome};

/// Resource primary vtable.
pub const VTABLE_RVA: usize = 0x293aeb8;

/// Interned resource token ID, u32.
pub const TOKEN_ID: usize = 0x08;
/// CStrategicResource active flag, byte.
pub const ACTIVE: usize = 0x10;
/// One-based resource index, i32, assigned from database count.
pub const INDEX: usize = 0xd8;
/// CStrategicResource factory cost, i64 fixed point.
pub const FACTORY_COST: usize = 0xe0;
/// Additive state resource-modifier key, u32, applied behind the state gate.
pub const STATE_ADDITIVE_KEY: usize = 0xf0;
/// State resource-factor key, u32, scaling by nonnegative one plus modifier.
pub const STATE_FACTOR_KEY: usize = 0xf4;
/// Temporary state resource modifier key, u32: the term the resources map icon adds while
/// the state's temporary-resource gate is set.
pub const TEMPORARY_KEY: usize = 0xf8;
/// State resource-accumulator modifier key, u32.
pub const MODIFIER_KEY: usize = 0xfc;
/// Resource constructor initializing token and ordinal.
pub const CONSTRUCTOR_RVA: usize = 0xbbb090;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// One-based native database index. Zero denotes the null object, not a row.
    pub id: u32,
    /// Native lexer token, kept distinct from its text and from the database index.
    pub token: u32,
    pub active: u8,
    pub factory_cost: i64,
    /// Callback-local descriptor address, never serialized or reused next hour.
    pub(crate) address: usize,
}

/// Reads the authenticated native descriptor census, including inactive rows.
/// The native token is distinct from its text; no global lexer memory is read.
/// Reuse this complete census for the dependent pool and exchange walks within
/// this callback. Capacity is caller-owned and never causes allocation.
pub fn walk(memory: &impl Memory, base: usize, output: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(memory: &impl Memory, base: usize, output: &mut Vec<Row>) -> Outcome {
    walk_buffer(memory, base, crate::walk::Buffer::Growing(output))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    mut output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let database = cstrategic_resource_database::Reader::new(memory, base)?;
        let mut len = 0;
        let mut unread_rows = 0;
        // Construction assigns one-based database indices.
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
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
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
        let mut rows = [Row::default(); 1];
        assert_eq!(
            walk(&fixture, 0, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
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
        let result = walk(&fixture, 0, &mut [Row::default(); 1]);
        assert!(matches!(result, Outcome::Unavailable { .. }));
    }
}
