use super::*;
use crate::{
    climited_use_tech_bonus as bonus, climited_use_tech_cost_reduction as cost,
    ctechnology_status as status, ctechnology_template as template,
};

pub const INVENTORY_INDEX: u32 = u32::MAX - 1;
const CATEGORY_TOKEN: usize = 0x2c;

pub fn walk_country_buffer<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_country_with_applied(
        memory,
        base,
        countries,
        country,
        context,
        inputs,
        0,
        &[],
        output,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Reuse the authenticated grant operands and caller-owned output without a second acquisition."
)]
pub fn walk_country_with_applied<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    authenticated_status: usize,
    applied: &[research_inputs::BonusRow],
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_country_with_applied_certified(
        memory,
        base,
        countries,
        country,
        context,
        inputs,
        authenticated_status,
        applied,
        output,
        false,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep grant reuse and the independent sharing-roster certificate explicit at the capture boundary."
)]
pub fn walk_country_with_applied_certified<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    authenticated_status: usize,
    applied: &[research_inputs::BonusRow],
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    certificate_available: bool,
) -> Outcome {
    let modifiers = super::walk_country_buffer_certified(
        memory,
        base,
        countries,
        country,
        context,
        inputs,
        output.suffix(0),
        certificate_available,
    );
    let Outcome::Rows { len } = modifiers else {
        return modifiers;
    };
    let mut out = Output {
        rows: output.suffix(len),
        len: 0,
        omitted: 0,
        current: Row {
            country: country.id,
            index: INVENTORY_INDEX,
            ..Default::default()
        },
    };
    let result = (|| {
        let pointer = if authenticated_status != 0 {
            authenticated_status
        } else if let Some(row) = inputs.first().filter(|row| row.status_pointer != 0) {
            row.status_pointer
        } else {
            let raw = status::copy_status(memory, country.pointer)?;
            status::decode_status(base, country.pointer, &raw)?
        };
        out.value(
            Kind::GrantCounter,
            i64::from(
                read::<i32>(memory, at(pointer, status::NEXT_BONUS_ID)?)
                    .ok_or("research grant counter unreadable")?,
            ),
        )?;
        for is_cost in [false, true] {
            let array = array_at(
                memory,
                at(
                    pointer,
                    if is_cost {
                        status::COST_REDUCTIONS
                    } else {
                        status::LIMITED_BONUSES
                    },
                )?,
                8,
            )?;
            let mut occurrence = 0;
            pointer_pages(memory, array, |pointer| {
                let cached = applied.iter().find(|row| {
                    row.resolved != 0
                        && row.source_pointer == pointer
                        && row.cost_reduction_raw.is_some() == is_cost
                });
                let (id, value, reduction) = if let Some(cached) = cached {
                    (
                        cached.value.id,
                        cached.value.clone(),
                        cached.cost_reduction_raw.unwrap_or(0),
                    )
                } else {
                    if read::<u64>(memory, pointer)
                        != Some(site(
                            base,
                            if is_cost {
                                cost::VTABLE_RVA
                            } else {
                                bonus::VTABLE_RVA
                            },
                        )? as u64)
                    {
                        return Err("research grant vtable mismatch");
                    }
                    let id = read::<i32>(
                        memory,
                        at(pointer, if is_cost { cost::ID } else { bonus::ID })?,
                    )
                    .ok_or("research grant ID unreadable")?;
                    let (value, reduction) = if is_cost {
                        cost::value(memory, pointer, id)?
                    } else {
                        (bonus::value(memory, pointer, id)?, 0)
                    };
                    (id, value, reduction)
                };
                out.push(
                    if is_cost {
                        Kind::CostGrant
                    } else {
                        Kind::SpeedGrant
                    },
                    if is_cost {
                        reduction
                    } else {
                        value.speed_bonus_raw
                    },
                    value.ahead_reduction_raw,
                    occurrence,
                    id as u32,
                    value.name,
                )?;
                let uses = read::<i32>(
                    memory,
                    at(pointer, if is_cost { cost::USES } else { bonus::USES })?,
                )
                .ok_or("research grant uses unreadable")?;
                let claims = if is_cost {
                    0
                } else {
                    read::<i32>(memory, at(pointer, bonus::CLAIMS)?)
                        .ok_or("research grant claims unreadable")?
                };
                out.push(
                    Kind::GrantUses,
                    i64::from(uses),
                    i64::from(claims),
                    u32::from(is_cost),
                    id as u32,
                    Text::default(),
                )?;
                for category in [false, true] {
                    let offset = match (is_cost, category) {
                        (false, false) => bonus::TECHNOLOGIES,
                        (false, true) => bonus::CATEGORIES,
                        (true, false) => cost::TECHNOLOGIES,
                        (true, true) => cost::CATEGORIES,
                    };
                    let restrictions = array_at(memory, at(pointer, offset)?, 8)?;
                    pointer_pages(memory, restrictions, |definition| {
                        let token = read::<u32>(
                            memory,
                            at(
                                definition,
                                if category {
                                    CATEGORY_TOKEN
                                } else {
                                    template::TECHNOLOGY_HANDLE
                                },
                            )?,
                        )
                        .ok_or("research grant restriction identity unreadable")?;
                        let index = if category {
                            0
                        } else {
                            read::<i32>(memory, at(definition, template::DATABASE_INDEX)?)
                                .ok_or("research grant technology index unreadable")?
                        };
                        out.push(
                            if category {
                                Kind::GrantCategory
                            } else {
                                Kind::GrantTechnology
                            },
                            i64::from(token),
                            i64::from(index),
                            u32::from(is_cost),
                            id as u32,
                            Text::default(),
                        )
                    })?;
                }
                occurrence += 1;
                Ok(())
            })?;
        }
        Ok(())
    })();
    match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if out.omitted != 0 => Outcome::Truncated {
            len: len + out.len,
            unread_rows: out.omitted,
        },
        Ok(()) => Outcome::Rows { len: len + out.len },
    }
}
