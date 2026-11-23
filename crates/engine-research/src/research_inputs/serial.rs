use super::*;

pub fn walk_country_buffer(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    context: Result<Context, &'static str>,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut bonuses: engine_native_memory::walk::Buffer<'_, BonusRow>,
) -> (Outcome, Outcome) {
    let mut bonus_len = 0_usize;
    let mut bonus_missing = 0_u32;
    let mut bonus_error = None;
    let result = (|| {
        let context = context?;
        let raw = ctechnology_status::copy_status(memory, country_pointer)?;
        let status = ctechnology_status::decode_status(base, country_pointer, &raw)?;
        let slots = Array::read(
            memory,
            at(status, ctechnology_status::RESEARCH_SLOTS_DATA)?,
            8,
        )
        .ok_or("research slots header invalid")?;
        output.ensure(slots.count as usize)?;
        let mut len = 0_usize;
        let mut pointers = [0_u8; 64 * 8];
        let mut first = 0;
        while first < slots.count && len < output.len() {
            let count = (slots.count - first)
                .min(64)
                .min((output.len() - len).min(64) as u32);
            if !read_slice(
                memory,
                slots.element(first, 8).ok_or("slot roster overflow")?,
                &mut pointers[..count as usize * 8],
            ) {
                return Err("slot pointers unreadable");
            }
            for offset in 0..count {
                if len == output.len() {
                    break;
                }
                let p = usize::try_from(u64::from_le_bytes(
                    pointers[offset as usize * 8..offset as usize * 8 + 8]
                        .try_into()
                        .unwrap(),
                ))
                .ok()
                .filter(|&p| p != 0)
                .ok_or("null research slot")?;
                let mut raw = cresearch_slot::Raw::default();
                if cresearch_slot::copy_raw(memory, p, &mut raw)? != 4 {
                    return Err("research slot fields unreadable");
                }
                let technology =
                    usize::try_from(u64::from_le_bytes(raw.owner[8..].try_into().unwrap()))
                        .map_err(|_| "technology address overflow")?;
                let mut linked = [0; ctechnology::INSTANCE_BYTES];
                if technology != 0
                    && ctechnology::copy_one(memory, technology, &mut linked)
                        != ctechnology::FIELDS.len()
                {
                    return Err("assigned technology unreadable");
                }
                let slot = cresearch_slot::decode_raw(
                    base,
                    status,
                    country,
                    &raw,
                    (technology != 0).then_some(&linked),
                )?;
                if slot.index >= slots.count {
                    return Err("research slot index outside roster");
                }
                let index = slot.index;
                let mut row = Row {
                    status_pointer: status,
                    slot,
                    cost_scale_raw: context.cost_scale_raw,
                    cost_debug: context.cost_debug,
                    ..Row::default()
                };
                if technology != 0 {
                    let instance = ctechnology::decode_one(base, status, country, &linked)?;
                    let mut header = [0; ctechnology_template::HEADER_BYTES];
                    if ctechnology_template::copy_header(memory, instance.template, &mut header)?
                        != 3
                    {
                        return Err("assigned technology template unreadable");
                    }
                    let template = ctechnology_template::decode_header(
                        instance.template,
                        &header,
                        slot.technology_handle,
                    )?;
                    row.token = Text::from_source(
                        memory,
                        template.source,
                        template.id_len,
                        (template.id_len <= 15
                            && u64::from_le_bytes(header[24..32].try_into().unwrap()) == 15)
                            .then_some(&header[..]),
                    )?;
                    row.classifier = instance.row.classifier;
                    row.technology_pointer = technology;
                    row.template_pointer = instance.template;
                    row.template_year = template.year;
                    row.template_index = template.database_index;
                    row.slot.database_index = template.database_index;
                    row.template_max_level = template.max_level;
                    row.level = instance.row.level;
                    row.progress_raw = instance.row.progress_raw;
                    row.ahead_reduction_raw = instance.row.ahead_reduction_years_raw;
                    row.cost_raw = template.research_cost_raw;
                    let extra =
                        read::<[u8; 24]>(memory, at(technology, ctechnology::APPLIED_SPEED_BONUS)?)
                            .ok_or("research bonus operands unreadable")?;
                    row.speed_bonus_raw = i64::from_le_bytes(extra[..8].try_into().unwrap());
                    let ids = Array::from_header(extra[8..].try_into().unwrap(), 4)
                        .ok_or("applied research bonus IDs invalid")?;
                    row.bonus_count = ids.count;
                    if bonus_error.is_none() && ids.count != 0 {
                        let attribution = (|| {
                            bonuses.ensure(
                                bonus_len
                                    .checked_add(ids.count as usize)
                                    .ok_or("research bonus row demand overflow")?,
                            )?;
                            let available = (bonuses.len() - bonus_len).min(ids.count as usize);
                            bonus_missing = bonus_missing
                                .checked_add(ids.count - available as u32)
                                .ok_or("research bonus count overflow")?;
                            let mut bytes = [0; 64 * 4];
                            let mut first_id = 0;
                            while first_id < available {
                                let count = (available - first_id).min(64);
                                if !read_slice(
                                    memory,
                                    ids.element(first_id as u32, 4).ok_or("bonus ID overflow")?,
                                    &mut bytes[..count * 4],
                                ) {
                                    return Err("applied research bonus IDs unreadable");
                                }
                                for offset in 0..count {
                                    let id = i32::from_le_bytes(
                                        bytes[offset * 4..offset * 4 + 4].try_into().unwrap(),
                                    );
                                    bonuses[bonus_len] = BonusRow {
                                        country,
                                        index,
                                        ordinal: (first_id + offset) as u32,
                                        technology_handle: slot.technology_handle,
                                        value: bonus::Value {
                                            id,
                                            ..Default::default()
                                        },
                                        ..Default::default()
                                    };
                                    bonus_len += 1;
                                }
                                first_id += count;
                            }
                            Ok::<(), &'static str>(())
                        })();
                        if let Err(reason) = attribution {
                            bonus_error = Some(reason);
                        }
                    }
                }
                output[len] = row;
                len += 1;
            }
            if len == output.len() {
                break;
            }
            first += count;
        }
        output[..len].sort_unstable_by_key(|row| row.slot.index);
        if output[..len]
            .windows(2)
            .any(|pair| pair[0].slot.index == pair[1].slot.index)
        {
            return Err("duplicate research slot index");
        }
        bonuses[..bonus_len].sort_unstable_by_key(|row| (row.index, row.ordinal));
        if bonus_len != 0
            && bonus_error.is_none()
            && let Err(reason) = resolve_bonuses(
                memory,
                base,
                status,
                &mut bonuses[..bonus_len],
            )
        {
            bonus_error = Some(reason);
        }
        let unread_rows = slots.count - len as u32;
        if unread_rows != 0 {
            bonus_error = Some("research slots truncated before bonus census");
            Ok(Outcome::Truncated { len, unread_rows })
        } else {
            Ok(Outcome::Rows { len })
        }
    })();
    let inputs = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    let attributed = match (result, bonus_error) {
        (Err(reason), _) | (_, Some(reason)) => Outcome::Unavailable { reason },
        (_, None) if bonus_missing != 0 => Outcome::Truncated {
            len: bonus_len,
            unread_rows: bonus_missing,
        },
        _ => Outcome::Rows { len: bonus_len },
    };
    (inputs, attributed)
}
