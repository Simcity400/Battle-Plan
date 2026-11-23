
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0286_3228);
pub const HISTORICAL_CHARACTERS: usize = 0x10;
pub const DYNAMIC_CHARACTERS: usize = 0x28;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x006f_c660);
pub const CLEAR_RVA: Rva = Rva(0x0070_16f0);

use crate::{Memory, ccharacter, cgame_state, cpdx_array::Array, read, read_slice};

#[derive(Debug, Clone, Copy)]
pub struct Roster {
    arrays: [Array; 2],
    count: u32,
    base: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub pointer: usize,
    pub id: u32,
    pub country: u32,
    pub roster: u8,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortraitIdentity {
    pub pointer: usize,
    pub character: u32,
}

#[derive(Debug)]
pub struct PortraitCensus {
    rows: Vec<PortraitIdentity>,
    scope: (usize, usize),
    complete: bool,
    written: usize,
    unchanged: bool,
    required: usize,
    stamp: u64,
}

fn fresh_stamp() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl PortraitCensus {
    #[cfg(test)]
    pub const INITIAL_CAPACITY: usize = 65536;

    pub fn try_new(capacity: usize) -> std::io::Result<Self> {
        Ok(Self {
            rows: crate::allocation::vec(capacity)?,
            scope: (0, 0),
            complete: false,
            written: 0,
            unchanged: false,
            required: 0,
            stamp: fresh_stamp(),
        })
    }

    #[cfg(test)]
    pub fn new(capacity: usize) -> Self {
        Self::try_new(capacity).unwrap()
    }

    pub fn expect(&mut self, count: usize) {
        self.required = self.required.max(count);
    }

    pub fn needs_growth(&self) -> bool {
        self.required > self.rows.capacity()
    }

    pub fn grow(&mut self) -> std::io::Result<()> {
        if self.needs_growth() {
            let additional = self.required.saturating_sub(self.rows.len());
            crate::allocation::reserve_exact(&mut self.rows, additional)?;
        }
        Ok(())
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<PortraitIdentity>()
    }

    pub fn begin(&mut self, base: usize, state: usize) {
        self.stamp = fresh_stamp();
        self.unchanged = self.complete && self.scope == (base, state);
        self.written = 0;
        self.required = 0;
        self.scope = (base, state);
        self.complete = false;
    }

    pub fn push(&mut self, row: PortraitIdentity) {
        if self.written < self.rows.capacity() {
            if let Some(previous) = self.rows.get_mut(self.written) {
                self.unchanged &= *previous == row;
                *previous = row;
            } else {
                self.unchanged = false;
                self.rows.push(row);
            }
            self.written += 1;
        }
    }

    pub fn finish(&mut self, count: usize) {
        self.expect(count);
        self.complete = self.written == count;
        self.unchanged &= self.complete && self.rows.len() == count;
        self.rows.truncate(self.written);
    }

    pub fn unchanged(&self) -> bool {
        self.complete && self.unchanged
    }

    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    pub fn rows(&self, base: usize, state: usize) -> Option<&[PortraitIdentity]> {
        (self.complete && self.scope == (base, state)).then_some(&self.rows)
    }
}

impl Roster {
    pub fn read(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Result<Self, &'static str> {
        let manager = game_state
            .checked_add(cgame_state::CHARACTER_MANAGER)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("character manager unreadable or null")?;
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        if read::<u64>(memory, manager) != Some(expected as u64) {
            return Err("character manager vtable mismatch");
        }
        let array = |offset| {
            manager
                .checked_add(offset)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("character roster unreadable or invalid")
        };
        let arrays = [array(HISTORICAL_CHARACTERS)?, array(DYNAMIC_CHARACTERS)?];
        let count = arrays[0]
            .count
            .checked_add(arrays[1].count)
            .ok_or("character roster count overflow")?;
        Ok(Self {
            arrays,
            count,
            base,
        })
    }

    pub fn count(self) -> u32 {
        self.count
    }

    pub fn sections(self) -> [(Array, Self); 2] {
        std::array::from_fn(|index| {
            let array = self.arrays[index];
            let mut arrays = [Array { data: 0, count: 0 }; 2];
            arrays[index] = array;
            (
                array,
                Self {
                    arrays,
                    count: array.count,
                    base: self.base,
                },
            )
        })
    }

    pub fn cursor(self) -> Cursor {
        Cursor {
            roster: self,
            index: 0,
            start: 0,
            len: 0,
            pointers: [0; 256],
            unreadable: false,
        }
    }

    pub fn at(self, memory: &impl Memory, index: u32) -> Result<Entry, &'static str> {
        let (pointer, roster, ordinal) = self.pointer(memory, index)?;
        let (id, country) = ccharacter::identity(memory, self.base, pointer)?;
        Ok(Entry {
            pointer,
            id,
            country,
            roster,
            ordinal,
        })
    }

    pub fn view<'a, M: Memory>(
        self,
        memory: &'a M,
        index: u32,
        required: [bool; 4],
    ) -> Result<ccharacter::View<'a, M>, &'static str> {
        let (pointer, roster, ordinal) = self.pointer(memory, index)?;
        ccharacter::View::read(memory, self.base, pointer, roster, ordinal, required)
    }

    pub(crate) fn pointer(
        self,
        memory: &impl Memory,
        index: u32,
    ) -> Result<(usize, u8, u32), &'static str> {
        let (roster, ordinal) = if index < self.arrays[0].count {
            (0, index)
        } else {
            (1, index - self.arrays[0].count)
        };
        let pointer = self.arrays[roster]
            .element(ordinal, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("character roster pointer unreadable or null")?;
        Ok((pointer, roster as u8, ordinal))
    }
}

pub struct Cursor {
    roster: Roster,
    index: u32,
    start: u32,
    len: u32,
    pointers: [u8; 256],
    unreadable: bool,
}

impl Cursor {
    pub fn next<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        required: [bool; 4],
    ) -> Result<Option<ccharacter::View<'a, M>>, &'static str> {
        let Some((pointer, roster, ordinal)) = self.next_pointer(memory)? else {
            return Ok(None);
        };
        ccharacter::View::read(memory, self.roster.base, pointer, roster, ordinal, required)
            .map(Some)
    }

    pub fn next_occurrence(&mut self, memory: &impl Memory) -> Result<Option<usize>, &'static str> {
        Ok(self.next_pointer(memory)?.map(|(pointer, _, _)| pointer))
    }

    pub fn refill_page<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        page: &mut ViewPage<'a, M>,
        required: [bool; 4],
    ) -> Result<bool, &'static str> {
        page.len = 0;
        page.next = 0;
        page.terminal = None;
        let Some(first) = self.next_pointer(memory)? else {
            return Ok(false);
        };
        let remaining = self.len - (self.index - self.start);
        for slot in 0..=remaining {
            let entry = if slot == 0 {
                Ok(Some(first))
            } else {
                self.next_pointer(memory)
            };
            let view = match entry {
                Ok(Some((pointer, roster, ordinal))) => ccharacter::View::read(
                    memory,
                    self.roster.base,
                    pointer,
                    roster,
                    ordinal,
                    required,
                ),
                Ok(None) => break,
                Err(reason) => {
                    page.terminal = Some(reason);
                    break;
                }
            };
            match view {
                Ok(view) => {
                    page.views[page.len] = Some(view);
                    page.len += 1;
                }
                Err(reason) => {
                    page.terminal = Some(reason);
                    break;
                }
            }
        }
        Ok(true)
    }

    fn next_pointer(
        &mut self,
        memory: &impl Memory,
    ) -> Result<Option<(usize, u8, u32)>, &'static str> {
        if self.unreadable {
            return Err("character roster pointers unreadable");
        }
        if self.index == self.roster.count {
            return Ok(None);
        }
        let (roster, ordinal) = if self.index < self.roster.arrays[0].count {
            (0, self.index)
        } else {
            (1, self.index - self.roster.arrays[0].count)
        };
        if self.index < self.start || self.index - self.start >= self.len {
            let array = self.roster.arrays[roster];
            let len = (array.count - ordinal).min(32);
            let address = array
                .element(ordinal, 8)
                .ok_or("character roster pointer address overflow")?;
            self.len = 0;
            if !read_slice(memory, address, &mut self.pointers[..len as usize * 8]) {
                self.unreadable = true;
                return Err("character roster pointers unreadable");
            }
            self.start = self.index;
            self.len = len;
        }
        let offset = (self.index - self.start) as usize * 8;
        let pointer = usize::try_from(u64::from_le_bytes(
            self.pointers[offset..offset + 8].try_into().unwrap(),
        ))
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or("character roster pointer unreadable or null")?;
        self.index += 1;
        Ok(Some((pointer, roster as u8, ordinal)))
    }
}

pub struct ViewPage<'a, M: Memory> {
    views: [Option<ccharacter::View<'a, M>>; 32],
    len: usize,
    next: usize,
    terminal: Option<&'static str>,
}

impl<M: Memory> Default for ViewPage<'_, M> {
    fn default() -> Self {
        Self {
            views: std::array::from_fn(|_| None),
            len: 0,
            next: 0,
            terminal: None,
        }
    }
}

impl<'a, M: Memory> Iterator for ViewPage<'a, M> {
    type Item = Result<ccharacter::View<'a, M>, &'static str>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next < self.len {
            let view = self.views[self.next].take().expect("staged character view");
            self.next += 1;
            Some(Ok(view))
        } else {
            self.terminal.take().map(Err)
        }
    }
}

pub fn visit(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visitor: impl FnMut(Entry) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let roster = Roster::read(memory, base, game_state)?;
    for index in 0..roster.count() {
        visitor(roster.at(memory, index)?)?;
    }
    Ok(())
}
