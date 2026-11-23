
use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, read, read_slice};
use engine_native::creferenced_division_template;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29daa08);
pub const REFERENCE_TYPE: u32 = 64;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const TEMPLATE: usize = 0x20;
pub const NAME: usize = 0x28;
pub const AMOUNT: usize = 0x48;
pub const LOCATION: usize = 0x4c;
pub const PRIORITY: usize = 0x50;
pub const LINES: usize = 0x78;
pub const LINE_STRIDE: usize = 0x08;
pub const ROLE_TOKEN: usize = 0x90;
pub const CLOSED: usize = 0x94;
pub const GATED_COUNTRY: usize = 0x98;
pub const CONSTRUCT_RVA: Rva = Rva(0x00d4_6e00);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub handle: u64,
    pub country: u32,
    pub ordinal: u32,
    pub template: u64,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub role_token: u32,
    pub closed: u8,
    pub name_start: usize,
    pub name_len: usize,
    pub exile_country: Option<u32>,
}

pub(crate) struct Read {
    pub handle: u64,
    pub row: Option<Row>,
    pub lines: Array,
    pub used: usize,
    pub wanted: usize,
}

pub(crate) fn read_row_buffer(
    memory: &impl Memory,
    base: usize,
    country: u32,
    ordinal: u32,
    pointer: usize,
    names: &mut engine_native_memory::walk::Buffer<'_, u8>,
    used: usize,
) -> Result<Read, &'static str> {
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("deployment conveyor module address overflow")?;
    let head =
        read::<[u8; 0x9c]>(memory, pointer).ok_or("deployment conveyor fields unreadable")?;
    let quad = |offset: usize| u64::from_le_bytes(head[offset..offset + 8].try_into().unwrap());
    let word = |offset: usize| u32::from_le_bytes(head[offset..offset + 4].try_into().unwrap());
    if quad(0) != expected as u64 {
        return Err("deployment conveyor vtable mismatch");
    }
    let handle = quad(TYPE);
    if handle as u32 != REFERENCE_TYPE {
        return Err("deployment conveyor reference type mismatch");
    }
    let template = usize::try_from(quad(TEMPLATE))
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("deployment conveyor template pointer unreadable or null")?;
    let template = creferenced_division_template::identity(memory, base, template)?;
    let lines = pointer
        .checked_add(LINES)
        .and_then(|address| Array::read(memory, address, LINE_STRIDE))
        .ok_or("deployment conveyor line array invalid")?;
    let name = pointer
        .checked_add(NAME)
        .ok_or("deployment conveyor address overflow")?;
    let (source, name_len) = literal(memory, name)?;
    names.ensure(
        used.checked_add(name_len)
            .ok_or("deployment name byte count overflow")?,
    )?;
    if name_len > names.len().saturating_sub(used) {
        return Ok(Read {
            handle,
            row: None,
            lines,
            used,
            wanted: name_len,
        });
    }
    if !read_slice(memory, source, &mut names[used..used + name_len]) {
        return Err("deployment conveyor name bytes unreadable");
    }
    Ok(Read {
        handle,
        row: Some(Row {
            handle,
            country,
            ordinal,
            template,
            amount: word(AMOUNT) as i32,
            location: word(LOCATION),
            priority: word(PRIORITY),
            role_token: word(ROLE_TOKEN),
            closed: head[CLOSED],
            name_start: used,
            name_len,
            exile_country: {
                let raw = word(GATED_COUNTRY);
                if raw == 0 { None } else { Some(raw) }
            },
        }),
        lines,
        used: used + name_len,
        wanted: name_len,
    })
}

fn literal(memory: &impl Memory, string: usize) -> Result<(usize, usize), &'static str> {
    let at = |offset: usize| {
        string
            .checked_add(offset)
            .ok_or("deployment conveyor name address overflow")
    };
    let len = read::<u64>(memory, at(msvc_string::LENGTH)?)
        .ok_or("deployment conveyor name length unreadable")?;
    let capacity = read::<u64>(memory, at(msvc_string::CAPACITY)?)
        .ok_or("deployment conveyor name capacity unreadable")?;
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("deployment conveyor name header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "deployment conveyor name length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        read::<u64>(memory, string)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("deployment conveyor name pointer unreadable or null")?
    };
    if read::<u8>(
        memory,
        source
            .checked_add(len)
            .ok_or("deployment conveyor name address overflow")?,
    ) != Some(0)
    {
        return Err("deployment conveyor name terminator unreadable or invalid");
    }
    Ok((source, len))
}
