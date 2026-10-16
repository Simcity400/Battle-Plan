//! The agency walk before its field gathers, kept for the cost bench and
//! the equality test: every field one guarded read.
use super::*;

pub(super) fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
) -> crate::csubject_relation::NameWalk {
    walk_shared(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(names),
        |_, _| {},
    )
}

pub(super) fn walk_shared(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut names: crate::walk::Buffer<'_, u8>,
    mut each: impl FnMut(u32, usize),
) -> crate::csubject_relation::NameWalk {
    use crate::{
        ccharacter::Literal,
        csubject_relation::{Name, NameWalk},
    };
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (mut len, mut bytes_len, mut required_bytes) = (0_usize, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let Some(pointer) = root(memory, base, country)? else {
                continue;
            };
            each(country.id, pointer);
            // The agency-name tooltip copies +0x80 verbatim for a created
            // agency, even an empty name; name assignment writes the same
            // bytes. Never localise it again.
            let name = Literal::read(memory, at(pointer, NAME)?)?;
            Literal::validate(memory, [&name])?;
            required_bytes = required_bytes
                .checked_add(name.len)
                .ok_or("agency name size overflow")?;
            output.ensure(len.checked_add(1).ok_or("agency row count overflow")?)?;
            names.ensure(required_bytes)?;
            if len == output.len() || name.len > names.len() - bytes_len {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("agency row count overflow")?;
                continue;
            }
            let active = Array::read(memory, at(pointer, OPERATIVES)?, 8)
                .ok_or("agency active roster unreadable or invalid")?;
            Literal::copy_all(memory, [&name], &mut names[bytes_len..bytes_len + name.len])?;
            output[len] = Row {
                country: country.id,
                upgrade_progress_raw: field(memory, pointer, UPGRADE_PROGRESS)?,
                defense_raw: field(memory, pointer, DEFENSE)?,
                active_count: active.count as i32,
                admission_ceiling: field(memory, pointer, OPERATIVE_ADMISSION_CEILING)?,
                unknown_f0: field(memory, pointer, UNKNOWN_F0)?,
                created: field(memory, pointer, CREATED)?,
                name: Name {
                    start: bytes_len,
                    len: name.len,
                },
            };
            bytes_len += name.len;
            len += 1;
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
    result.unwrap_or_else(crate::csubject_relation::unavailable)
}

fn root(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<Option<usize>, &'static str> {
    let pointer = usize::try_from(field::<u64>(
        memory,
        country.pointer,
        ccountry::INTELLIGENCE_AGENCY,
    )?)
    .map_err(|_| "agency pointer overflow")?;
    if pointer == 0 {
        return Ok(None);
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("agency vtable address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("agency vtable mismatch");
    }
    if field::<u64>(memory, pointer, OWNER_COUNTRY)? != country.pointer as u64 {
        return Err("agency owner pointer mismatch");
    }
    Ok(Some(pointer))
}
