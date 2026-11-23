use super::*;

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    mut rows: engine_native_memory::walk::Buffer<'_, Row>,
    mut completed: engine_native_memory::walk::Buffer<'_, CompletedRow>,
    tokens: engine_native_memory::walk::Buffer<'_, u8>,
) -> FocusWalk {
    let mut arena = Tokens {
        bytes: tokens,
        len: 0,
        required: 0,
    };
    let root = country
        .checked_add(ccountry::FOCUS_PROGRESS)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| value != 0);
    let result = match root {
        Some(root)
            if engine_sites::at(base, VTABLE_RVA)
                .is_some_and(|expected| read::<u64>(memory, root) == Some(expected as u64)) =>
        {
            let outcome = summary(memory, country, country_id, root, &mut rows, &mut arena)
                .unwrap_or_else(|reason| Outcome::Unavailable { reason });
            let saved_len = arena.len;
            let completed_outcome =
                completed_rows(memory, country_id, root, &mut completed, &mut arena)
                    .unwrap_or_else(|reason| Outcome::Unavailable { reason });
            if matches!(completed_outcome, Outcome::Unavailable { .. }) {
                arena.len = saved_len;
            }
            (outcome, completed_outcome)
        }
        Some(_) => {
            let outcome = Outcome::Unavailable {
                reason: "focus status vtable mismatch",
            };
            (outcome, outcome)
        }
        None => {
            let outcome = Outcome::Unavailable {
                reason: "focus status pointer unreadable or null",
            };
            (outcome, outcome)
        }
    };
    FocusWalk {
        outcome: result.0,
        completed_outcome: result.1,
        bytes_len: arena.len,
        required_bytes: arena.required,
    }
}

fn summary(
    memory: &impl Memory,
    country: usize,
    country_id: u32,
    root: usize,
    output: &mut engine_native_memory::walk::Buffer<'_, Row>,
    arena: &mut Tokens<'_>,
) -> Result<Outcome, &'static str> {
    let tree = read::<u64>(memory, at(country, ccountry::FOCUS_TREE)?)
        .ok_or("focus tree pointer unreadable")?;
    let mut selections = [0_u8; 16];
    let mut progress = [0_u8; 8];
    let mut paused = [0_u8; 1];
    let mut count = [0_u8; 4];
    if memory.copy_scattered(
        [
            at(root, CURRENT)?,
            at(root, PROGRESS)?,
            at(root, PAUSED)?,
            at(root, COMPLETED_COUNT)?,
        ],
        &mut [&mut selections, &mut progress, &mut paused, &mut count],
    ) != 4
    {
        return Err("focus summary fields unreadable");
    }
    let current = u64::from_le_bytes(selections[..8].try_into().unwrap());
    let continuous = u64::from_le_bytes(selections[8..].try_into().unwrap());
    let completed_count = i32::from_le_bytes(count);
    if completed_count < 0 {
        return Err("completed focus count is negative");
    }
    let definitions = [
        optional_token(memory, tree, cnational_focus_tree::TOKEN)?,
        optional_token(memory, current, cnational_focus::TOKEN)?,
        optional_token(memory, continuous, ccontinuous_national_focus::TOKEN)?,
    ];
    let required = definitions
        .iter()
        .flatten()
        .try_fold(0_usize, |sum, token| {
            sum.checked_add(token.len)
                .ok_or("focus summary token byte count overflow")
        })?;
    arena.account(required)?;
    output.ensure(1)?;
    if output.is_empty() || !arena.fits(required) {
        return Ok(Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        });
    }
    let saved_len = arena.len;
    let mut spans = [None; 3];
    for (destination, source) in spans.iter_mut().zip(definitions) {
        if let Some(source) = source {
            match arena.copy(memory, source) {
                Ok(span) => *destination = Some(span),
                Err(reason) => {
                    arena.len = saved_len;
                    return Err(reason);
                }
            }
        }
    }
    output[0] = Row {
        country: country_id,
        tree: spans[0],
        current: spans[1],
        continuous: spans[2],
        progress: i64::from_le_bytes(progress),
        paused: paused[0],
        completed_count,
    };
    Ok(Outcome::Rows { len: 1 })
}

fn completed_rows(
    memory: &impl Memory,
    country_id: u32,
    root: usize,
    output: &mut engine_native_memory::walk::Buffer<'_, CompletedRow>,
    arena: &mut Tokens<'_>,
) -> Result<Outcome, &'static str> {
    let roster = Array::read(memory, at(root, COMPLETED_DATA)?, COMPLETED_STRIDE)
        .ok_or("completed focus roster unreadable or invalid")?;
    output.ensure(roster.count as usize)?;
    let mut len = 0;
    let mut unread_rows = 0;
    for ordinal in 0..roster.count {
        if len == output.len() {
            unread_rows = roster.count - ordinal;
            break;
        }
        let pointer = read::<u64>(memory, roster.element(ordinal, COMPLETED_STRIDE).unwrap())
            .ok_or("completed focus pointer unreadable")?;
        let token = optional_token(memory, pointer, cnational_focus::TOKEN)?
            .ok_or("completed focus pointer is null")?;
        arena.account(token.len)?;
        if !arena.fits(token.len) {
            unread_rows = roster.count - ordinal;
            break;
        }
        output[len] = CompletedRow {
            country: country_id,
            ordinal,
            token: arena.copy(memory, token)?,
        };
        len += 1;
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer.checked_add(offset).ok_or("focus address overflow")
}

fn optional_token(
    memory: &impl Memory,
    pointer: u64,
    offset: usize,
) -> Result<Option<Token>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let pointer = usize::try_from(pointer).map_err(|_| "focus pointer overflow")?;
    let string = at(pointer, offset)?;
    let header = read::<[u8; 32]>(memory, string).ok_or("focus token header unreadable")?;
    let length = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if length > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("focus token header invalid");
    }
    let len = usize::try_from(length).map_err(|_| "focus token length overflow")?;
    let address = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "focus token pointer overflow")?
    };
    if address == 0 || read::<u8>(memory, at(address, len)?) != Some(0) {
        return Err("focus token terminator unreadable or invalid");
    }
    Ok(Some(Token { address, len }))
}
