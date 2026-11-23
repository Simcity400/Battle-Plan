use engine_native_memory::{Memory, cpdx_array::Array, memory::Raw, msvc_string, read};
use engine_sites::Rva;

pub const STATUS_VTABLE: Rva = Rva(0x29e1d48);
pub const FOLDER_VTABLE: Rva = Rva(0x29e1cf8);
pub const TRACK_VTABLE: Rva = Rva(0x29e1ca8);
pub const COST_VTABLE: Rva = Rva(0x29e1c08);
pub const GRANT_VTABLE: Rva = Rva(0x284f288);
pub const FILTER_VTABLE: Rva = Rva(0x284f238);
pub const LEADER_VTABLE: Rva = Rva(0x29e1c58);
pub const EQUIPMENT_VTABLE: Rva = Rva(0x29e1b60);
pub const FOLDER_TEMPLATE_VTABLE: Rva = Rva(0x2796f30);
pub const GRAND_TEMPLATE_VTABLE: Rva = Rva(0x27970f0);
pub const SUB_TEMPLATE_VTABLE: Rva = Rva(0x27972a0);
pub const TRACK_TEMPLATE_VTABLE: Rva = Rva(0x27973b0);
pub const TEMPLATE_TOKEN: usize = 8;
pub const FILTER_TRACK: usize = 8;
pub const FILTER_FOLDER: usize = 0x10;
pub const FILTER_GRAND: usize = 0x18;
pub const FILTER_SUB: usize = 0x20;
pub const FILTER_TRACK_INDEX: usize = 0x28;

pub const COUNTRY: usize = 8;
pub const FOLDERS: usize = 0x10;
pub const COSTS: usize = 0x40;
pub const GRANTS: usize = 0x58;
pub const EQUIPMENT: usize = 0x70;
pub const FOLDER_STRIDE: usize = 0x50;
pub const TRACK_STRIDE: usize = 0x60;
pub const COST_STRIDE: usize = 0x40;
pub const GRANT_STRIDE: usize = 0x70;
pub const LEADER_STRIDE: usize = 0x18;
pub const EQUIPMENT_STRIDE: usize = 0x18;
pub const FOLDER_TEMPLATE: usize = 8;
pub const GRAND_TEMPLATE: usize = 0x10;
pub const TRACKS: usize = 0x18;
pub const FOLDER_OWNER: usize = 0x30;
pub const COMPLETED: usize = 0x38;
pub const SUB_TEMPLATE: usize = 8;
pub const REWARDS: usize = 0x10;
pub const MASTERY: usize = 0x18;
pub const BANK: usize = 0x20;
pub const DAILY: usize = 0x28;
pub const LEADERS: usize = 0x30;
pub const EFFECTIVE_DAILY: usize = 0x48;
pub const TRACK_OWNER: usize = 0x50;
pub const TRACK_FOLDER: usize = 0x58;
pub const COST_USES: usize = 8;
pub const COST_NAME: usize = 0x10;
pub const COST_FACTOR: usize = 0x30;
pub const COST_FOLDER: usize = 0x38;
pub const GRANT_NAME: usize = 8;
pub const GRANT_FILTER: usize = 0x28;
pub const GRANT_DAILY: usize = 0x58;
pub const GRANT_BONUS: usize = 0x60;
pub const GRANT_DAYS: usize = 0x68;

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("doctrine address overflow")
}
pub fn field<T: Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("doctrine field unreadable")
}
pub fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
) -> Result<(), &'static str> {
    if object == 0
        || field::<u64>(memory, object, 0)?
            != engine_sites::at(base, vtable).ok_or("doctrine address overflow")? as u64
    {
        return Err("doctrine object identity invalid");
    }
    Ok(())
}
pub fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, stride).ok_or("doctrine array invalid")
}
pub fn token(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
) -> Result<u32, &'static str> {
    authenticate(memory, base, object, vtable)?;
    field(memory, object, TEMPLATE_TOKEN)
}
fn optional_token(
    memory: &impl Memory,
    base: usize,
    object: usize,
    offset: usize,
    vtable: Rva,
) -> Result<Option<u32>, &'static str> {
    let pointer = field::<u64>(memory, object, offset)? as usize;
    (pointer != 0)
        .then(|| token(memory, base, pointer, vtable))
        .transpose()
}

#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub folders: Array,
}
pub fn status(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    country: u32,
) -> Result<Status, &'static str> {
    authenticate(memory, base, pointer, STATUS_VTABLE)?;
    if field::<u32>(memory, pointer, COUNTRY)? != country {
        return Err("doctrine country identity changed");
    }
    Ok(Status {
        folders: array(memory, pointer, FOLDERS, FOLDER_STRIDE)?,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Folder {
    pub token: u32,
    pub grand: Option<u32>,
    pub tracks: Array,
    pub completed: Array,
}
pub fn folder(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    owner: usize,
) -> Result<Folder, &'static str> {
    authenticate(memory, base, pointer, FOLDER_VTABLE)?;
    if field::<u64>(memory, pointer, FOLDER_OWNER)? as usize != owner {
        return Err("doctrine folder owner changed");
    }
    let template = field::<u64>(memory, pointer, FOLDER_TEMPLATE)? as usize;
    Ok(Folder {
        token: token(memory, base, template, FOLDER_TEMPLATE_VTABLE)?,
        grand: optional_token(memory, base, pointer, GRAND_TEMPLATE, GRAND_TEMPLATE_VTABLE)?,
        tracks: array(memory, pointer, TRACKS, TRACK_STRIDE)?,
        completed: array(memory, pointer, COMPLETED, 8)?,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Track {
    pub sub: Option<u32>,
    pub rewards: i32,
    pub mastery: i64,
    pub bank: i64,
    pub daily: i64,
    pub effective_daily: i64,
}
pub fn track(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    owner: usize,
    folder: usize,
) -> Result<Track, &'static str> {
    authenticate(memory, base, pointer, TRACK_VTABLE)?;
    if field::<u64>(memory, pointer, TRACK_OWNER)? as usize != owner
        || field::<u64>(memory, pointer, TRACK_FOLDER)? as usize != folder
    {
        return Err("doctrine track owner changed");
    }
    Ok(Track {
        sub: optional_token(memory, base, pointer, SUB_TEMPLATE, SUB_TEMPLATE_VTABLE)?,
        rewards: field(memory, pointer, REWARDS)?,
        mastery: field(memory, pointer, MASTERY)?,
        bank: field(memory, pointer, BANK)?,
        daily: field(memory, pointer, DAILY)?,
        effective_daily: field(memory, pointer, EFFECTIVE_DAILY)?,
    })
}

pub fn leader(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u64, i64), &'static str> {
    authenticate(memory, base, pointer, LEADER_VTABLE)?;
    Ok((field(memory, pointer, 8)?, field(memory, pointer, 0x10)?))
}
pub fn completed(memory: &impl Memory, base: usize, entry: usize) -> Result<u32, &'static str> {
    token(
        memory,
        base,
        field::<u64>(memory, entry, 0)? as usize,
        SUB_TEMPLATE_VTABLE,
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub track: Option<u32>,
    pub folder: Option<u32>,
    pub grand: Option<u32>,
    pub sub: Option<u32>,
    pub track_index: Option<i32>,
}
pub fn filter(memory: &impl Memory, base: usize, pointer: usize) -> Result<Filter, &'static str> {
    authenticate(memory, base, pointer, FILTER_VTABLE)?;
    let index = field::<i32>(memory, pointer, FILTER_TRACK_INDEX)?;
    Ok(Filter {
        track: optional_token(memory, base, pointer, FILTER_TRACK, TRACK_TEMPLATE_VTABLE)?,
        folder: optional_token(memory, base, pointer, FILTER_FOLDER, FOLDER_TEMPLATE_VTABLE)?,
        grand: optional_token(memory, base, pointer, FILTER_GRAND, GRAND_TEMPLATE_VTABLE)?,
        sub: optional_token(memory, base, pointer, FILTER_SUB, SUB_TEMPLATE_VTABLE)?,
        track_index: (index != -1).then_some(index),
    })
}

pub fn name(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    bytes: &mut Vec<u8>,
    used: &mut usize,
) -> Result<(usize, usize), &'static str> {
    msvc_string::append(memory, at(pointer, offset)?, bytes, used)
}
pub fn cost(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(i32, i64, Option<u32>), &'static str> {
    authenticate(memory, base, pointer, COST_VTABLE)?;
    let template = field::<u64>(memory, pointer, COST_FOLDER)? as usize;
    Ok((
        field(memory, pointer, COST_USES)?,
        field(memory, pointer, COST_FACTOR)?,
        (template != 0)
            .then(|| token(memory, base, template, FOLDER_TEMPLATE_VTABLE))
            .transpose()?,
    ))
}
pub fn grant(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(Filter, i64, i64, i32), &'static str> {
    authenticate(memory, base, pointer, GRANT_VTABLE)?;
    Ok((
        filter(memory, base, at(pointer, GRANT_FILTER)?)?,
        field(memory, pointer, GRANT_DAILY)?,
        field(memory, pointer, GRANT_BONUS)?,
        field(memory, pointer, GRANT_DAYS)?,
    ))
}
pub fn equipment(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, i32, i32), &'static str> {
    authenticate(memory, base, pointer, EQUIPMENT_VTABLE)?;
    Ok((
        field(memory, pointer, 8)?,
        field(memory, pointer, 0xc)?,
        field(memory, pointer, 0x10)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fixture {
        bytes: Vec<u8>,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Fixture {
        fn new(len: usize) -> Self {
            Self {
                bytes: vec![0; len],
                reads: RefCell::new(Vec::new()),
                fault: None,
            }
        }
        fn word(&mut self, at: usize, value: u64) {
            self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            if self.fault.is_some_and(|at| (address..end).contains(&at)) {
                return false;
            }
            let Some(source) = self.bytes.get(address..end) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }

    #[test]
    fn doctrine_tracks_read_only_demanded_objects() {
        for count in [3, 7] {
            let first = 0x1000;
            let selected = first + count * TRACK_STRIDE;
            let mut memory = Fixture::new(selected + 16);
            memory.word(selected, SUB_TEMPLATE_VTABLE.0 as u64);
            memory.word(selected + TEMPLATE_TOKEN, 91_234);
            for index in 0..count {
                let pointer = first + index * TRACK_STRIDE;
                memory.word(pointer, TRACK_VTABLE.0 as u64);
                memory.word(pointer + SUB_TEMPLATE, selected as u64);
                memory.word(pointer + REWARDS, 7);
                memory.word(pointer + MASTERY, (index as i64 * 100_000) as u64);
                memory.word(pointer + BANK, (-1_i64) as u64);
                memory.word(pointer + DAILY, 21_345);
                memory.word(pointer + TRACK_OWNER, 57);
                memory.word(pointer + TRACK_FOLDER, 91);
            }
            let pointer = first + (count - 1) * TRACK_STRIDE;
            let result = track(&memory, 0, pointer, 57, 91).unwrap();
            assert_eq!(result.sub, Some(91_234));
            assert_eq!(result.mastery, (count as i64 - 1) * 100_000);
            assert_eq!(result.bank, -1);
            assert!(memory.reads.borrow().iter().all(|&(at, len)| {
                (pointer..pointer + TRACK_STRIDE).contains(&at)
                    && at + len <= pointer + TRACK_STRIDE
                    || (selected..selected + 12).contains(&at) && at + len <= selected + 12
            }));
            memory.fault = Some(pointer + MASTERY);
            assert_eq!(
                track(&memory, 0, pointer, 57, 91).unwrap_err(),
                "doctrine field unreadable"
            );
            memory.fault = None;
            assert_eq!(
                track(&memory, 0, pointer, 58, 91).unwrap_err(),
                "doctrine track owner changed"
            );
        }
    }

    #[test]
    fn doctrine_filter_preserves_zero_index_and_absent_selectors() {
        let mut memory = Fixture::new(128);
        memory.word(8, FILTER_VTABLE.0 as u64);
        memory.word(8 + FILTER_TRACK_INDEX, (-1_i64) as u64);
        assert_eq!(filter(&memory, 0, 8).unwrap(), Filter::default());
        memory.word(8 + FILTER_TRACK_INDEX, 0);
        assert_eq!(filter(&memory, 0, 8).unwrap().track_index, Some(0));
        memory.word(64, TRACK_TEMPLATE_VTABLE.0 as u64);
        memory.word(64 + TEMPLATE_TOKEN, 91);
        memory.word(8 + FILTER_TRACK, 64);
        let selected = filter(&memory, 0, 8).unwrap();
        assert_eq!(selected.track, Some(91));
        assert_eq!(selected.track_index, Some(0));
    }
}
