
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0279_65d8);
pub const ACCUMULATOR: usize = 0x10;
pub const DISPLAY_NAME: usize = 0x58;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub key: u32,
    pub raw: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub name: Vec<u8>,
    pub entries: Vec<Entry>,
}

pub fn read_into(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    modifier: usize,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("faction modifier vtable address overflow")? as u64;
    if engine_native_memory::read::<u64>(memory, modifier) != Some(expected) {
        return Err("faction slot modifier vtable mismatch");
    }
    let source = modifier
        .checked_add(ACCUMULATOR)
        .and_then(|address| {
            engine_native_memory::cpdx_array::Array::read(memory, address, engine_native_parts::modifier_entry::STRIDE)
        })
        .ok_or("faction slot modifier entries unreadable or invalid")?;
    crate::allocation::resize_default(&mut output.entries, source.count as usize)
        .map_err(|_| "faction slot modifier storage allocation failed")?;
    for (ordinal, entry) in output.entries.iter_mut().enumerate() {
        let address = source
            .element(ordinal as u32, engine_native_parts::modifier_entry::STRIDE)
            .ok_or("faction slot modifier address overflow")?;
        let key = engine_native_memory::read(
            memory,
            address
                .checked_add(engine_native_parts::modifier_entry::KEY)
                .ok_or("faction modifier key address overflow")?,
        )
        .ok_or("faction slot modifier key unreadable")?;
        let raw = engine_native_memory::read(
            memory,
            address
                .checked_add(engine_native_parts::modifier_entry::VALUE)
                .ok_or("faction modifier value address overflow")?,
        )
        .ok_or("faction slot modifier value unreadable")?;
        *entry = Entry { key, raw };
    }
    let address = modifier
        .checked_add(DISPLAY_NAME)
        .ok_or("faction modifier name address overflow")?;
    let name = engine_native_memory::msvc_string::Descriptor::read(memory, address)?;
    name.copy_buffer(
        memory,
        &mut engine_native_memory::walk::Buffer::Growing(&mut output.name),
        0,
    )?;
    output.name.truncate(name.len);
    Ok(())
}
