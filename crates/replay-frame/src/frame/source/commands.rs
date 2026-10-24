use replay_data::rows::CountryCommandPowerRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandAllocation {
    pub canonical_slot: u32,
    pub allocated_command_power_raw: i64,
}
impl From<&CountryCommandPowerRow> for CommandAllocation {
    fn from(row: &CountryCommandPowerRow) -> Self {
        Self {
            canonical_slot: row.canonical_slot,
            allocated_command_power_raw: row.allocated_command_power_raw,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct CommandAllocations<'a>(&'a [CountryCommandPowerRow]);
impl<'a> CommandAllocations<'a> {
    pub(super) fn decoded(rows: &'a [CountryCommandPowerRow]) -> Self {
        Self(rows)
    }
    pub fn iter(self) -> impl Iterator<Item = CommandAllocation> + 'a {
        self.0.iter().map(CommandAllocation::from)
    }
    pub fn len(self) -> usize {
        self.0.len()
    }
    pub fn get(self, slot: u32) -> Option<CommandAllocation> {
        self.0
            .binary_search_by_key(&slot, |row| row.canonical_slot)
            .ok()
            .map(|at| CommandAllocation::from(&self.0[at]))
    }
}
