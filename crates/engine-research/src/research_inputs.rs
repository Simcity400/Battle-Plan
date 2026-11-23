use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use crate::{
    climited_use_tech_bonus as bonus,
    cresearch_slot,
    ctechnology,
    ctechnology_status,
    ctechnology_template,
};

mod text;
pub use text::Text;

#[cfg(test)]
mod tests;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub technology_pointer: usize,
    pub template_pointer: usize,
    pub status_pointer: usize,
    pub template_year: i32,
    pub template_index: i32,
    pub template_max_level: i32,
    pub slot: cresearch_slot::Row,
    pub token: Text,
    pub classifier: u32,
    pub level: i32,
    pub progress_raw: i64,
    pub cost_raw: i64,
    pub cost_scale_raw: i64,
    pub cost_debug: u8,
    pub ahead_reduction_raw: i64,
    pub speed_bonus_raw: i64,
    pub bonus_count: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BonusRow {
    pub source_pointer: usize,
    pub country: u32,
    pub index: u32,
    pub ordinal: u32,
    pub technology_handle: u32,
    pub resolved: u8,
    pub value: bonus::Value,
    pub cost_reduction_raw: Option<i64>,
}

#[derive(Clone, Copy)]
pub struct Context {
    cost_scale_raw: i64,
    cost_debug: u8,
}
impl Context {
    pub fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut scale = [0; 8];
        let mut debug = [0; 1];
        if memory.copy_scattered_prefetched(
            [
                engine_sites::at(base, ctechnology::BASE_COST_SCALE_RVA)
                    .ok_or("module address overflow")?,
                engine_sites::at(base, ctechnology::BASE_COST_DEBUG_FLAG_RVA)
                    .ok_or("module address overflow")?,
            ],
            &mut [&mut scale[..], &mut debug[..]],
        ) != 2
        {
            return Err("research cost defines unreadable");
        }
        Ok(Self {
            cost_scale_raw: i64::from_le_bytes(scale),
            cost_debug: debug[0],
        })
    }

    pub fn digest(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in self
            .cost_scale_raw
            .to_le_bytes()
            .into_iter()
            .chain([self.cost_debug])
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        hash
    }
}

pub fn walk_country(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    context: Result<Context, &'static str>,
    output: &mut Vec<Row>,
    bonuses: &mut Vec<BonusRow>,
) -> (Outcome, Outcome) {
    walk_country_buffer(
        memory,
        base,
        country_pointer,
        country,
        context,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(bonuses),
    )
}

pub fn walk_country_buffer(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    context: Result<Context, &'static str>,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    bonuses: engine_native_memory::walk::Buffer<'_, BonusRow>,
) -> (Outcome, Outcome) {
    let (inputs, bonuses, _) = walk_country_buffer_with_status(
        memory,
        base,
        country_pointer,
        country,
        context,
        output,
        bonuses,
    );
    (inputs, bonuses)
}

pub fn walk_country_buffer_with_status(
    memory: &impl Memory,
    base: usize,
    country_pointer: usize,
    country: u32,
    context: Result<Context, &'static str>,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut bonuses: engine_native_memory::walk::Buffer<'_, BonusRow>,
) -> (Outcome, Outcome, usize) {
    let mut status_pointer = 0;
    let mut bonus_len = 0_usize;
    let mut bonus_missing = 0_u32;
    let mut bonus_error = None;
    let result = (|| {
        let context = context?;
        let raw = ctechnology_status::copy_status(memory, country_pointer)?;
        let status = ctechnology_status::decode_status(base, country_pointer, &raw)?;
        status_pointer = status;
        let slots = Array::read(
            memory,
            at(status, ctechnology_status::RESEARCH_SLOTS_DATA)?,
            8,
        )
        .ok_or("research slots header invalid")?;
        output.ensure(slots.count as usize)?;
        let batches = crate::governance_batch::Assignments::gather(
            memory,
            slots,
            output.len().min(slots.count as usize),
            true,
        );
        let batched = batches.view(memory);
        let memory = &batched;
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
                    slot,
                    status_pointer: status,
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
    (inputs, attributed, status_pointer)
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("research input address overflow")
}

fn resolve_bonuses(
    memory: &impl Memory,
    base: usize,
    status: usize,
    output: &mut [BonusRow],
) -> Result<(), &'static str> {
    resolve_bonus_roster(memory, base, status, output, false)?;
    if output.iter().any(|r| r.resolved == 0) {
        resolve_bonus_roster(memory, base, status, output, true)?;
    }
    Ok(())
}

fn resolve_bonus_roster(
    memory: &impl Memory,
    base: usize,
    status: usize,
    output: &mut [BonusRow],
    cost: bool,
) -> Result<(), &'static str> {
    if output.iter().all(|r| r.resolved != 0) {
        return Ok(());
    }
    use crate::climited_use_tech_cost_reduction as reduction;
    use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
    let fields: [RecordField; 2] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: if cost { reduction::ID } else { bonus::ID },
            destination_offset: 8,
            len: 4,
        },
    ];
    let layout = RecordLayout::new(&fields, 12).unwrap();
    let roster = Array::read(
        memory,
        at(
            status,
            if cost {
                ctechnology_status::COST_REDUCTIONS
            } else {
                ctechnology_status::LIMITED_BONUSES
            },
        )?,
        8,
    )
    .ok_or("research bonus roster invalid")?;
    let vtable = engine_sites::at(
        base,
        if cost {
            reduction::VTABLE_RVA
        } else {
            bonus::VTABLE_RVA
        },
    )
    .ok_or("module address overflow")? as u64;
    let mut first = 0;
    let mut bytes = [0; 64 * 8];
    let mut pointers = [0; 64];
    let mut identities = [0; 64 * 12];
    while first < roster.count {
        let count = (roster.count - first).min(64) as usize;
        if !read_slice(
            memory,
            roster.element(first, 8).ok_or("bonus roster overflow")?,
            &mut bytes[..count * 8],
        ) {
            return Err("research bonus pointers unreadable");
        }
        for (pointer, bytes) in pointers[..count].iter_mut().zip(bytes.chunks_exact(8)) {
            *pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                .ok()
                .filter(|&p| p != 0)
                .ok_or("research bonus pointer invalid")?;
        }
        if memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &layout,
            &mut identities[..count * 12],
        ) != count * 2
        {
            return Err("research bonus identities unreadable");
        }
        for (&pointer, identity) in pointers[..count].iter().zip(identities.chunks_exact(12)) {
            if u64::from_le_bytes(identity[..8].try_into().unwrap()) != vtable {
                return Err("research bonus vtable mismatch");
            }
            let id = i32::from_le_bytes(identity[8..].try_into().unwrap());
            if output
                .iter()
                .any(|r| r.value.id == id && (!cost || r.resolved == 0))
            {
                let (value, cost_reduction) = if cost {
                    let (value, amount) = reduction::value(memory, pointer, id)?;
                    (value, Some(amount))
                } else {
                    (bonus::value(memory, pointer, id)?, None)
                };
                for row in output.iter_mut().filter(|r| r.value.id == id) {
                    if row.resolved != 0 {
                        return Err("duplicate research bonus ID");
                    }
                    row.source_pointer = pointer;
                    row.value = value.clone();
                    row.cost_reduction_raw = cost_reduction;
                    row.resolved = 1;
                }
                if output.iter().all(|r| r.resolved != 0) {
                    return Ok(());
                }
            }
        }
        first += count as u32;
    }
    Ok(())
}

#[cfg(test)]
mod serial;
