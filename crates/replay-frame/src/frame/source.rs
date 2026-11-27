use super::*;
mod characters;
mod commands;
mod divisions;
mod leaders;
mod lexical;
mod provinces;
mod rows;
pub use characters::SharedCharacters;
pub use commands::{CommandAllocation, CommandAllocations};
pub use divisions::{Division, DivisionName, DivisionNames, Divisions};
pub use leaders::{Leader, Leaders};
pub use lexical::{OperandStatus, SharedLexical};
pub use provinces::{Province, Provinces};
pub use rows::Rows;

pub trait RecordedSource: Send + Sync {
    fn hour(&self) -> Hour;
    fn branch(&self) -> u64;
    fn status(&self, table: TableId) -> OperandStatus<'_>;
    fn sampled_at(&self, index: usize) -> Option<Hour>;
    fn identity(&self, index: usize) -> Option<ViewIdentity>;
    fn clock_seed(&self) -> io::Result<Option<i32>>;
    fn lexical_rows(&self) -> Rows<'_, replay_data::rows::LexicalTokenRow>;
    fn shared_lexical(&self) -> SharedLexical;
    fn command_allocations(&self) -> io::Result<CommandAllocations<'_>>;
    fn province_rows(&self) -> Provinces<'_>;
    fn divisions(&self) -> io::Result<Divisions<'_>>;
    fn division_names(&self) -> io::Result<DivisionNames<'_>>;
    fn leader_rows(&self) -> Leaders<'_>;
    fn leader(&self, reference: u64) -> io::Result<Option<Leader<'_>>>;

    fn lexical_status(&self) -> OperandStatus<'_> {
        self.status(TableId::LexicalTokens)
    }
    fn lexical(&self) -> io::Result<Rows<'_, replay_data::rows::LexicalTokenRow>> {
        self.lexical_status()
            .complete(self.hour(), TableId::LexicalTokens)?;
        Ok(self.lexical_rows())
    }
    fn province_status(&self) -> OperandStatus<'_> {
        self.status(TableId::Provinces)
    }
    fn provinces(&self) -> io::Result<Provinces<'_>> {
        self.province_status()
            .complete(self.hour(), TableId::Provinces)?;
        Ok(self.province_rows())
    }
    fn leader_status(&self) -> OperandStatus<'_> {
        self.status(TableId::UnitLeaders)
    }
    fn leaders(&self) -> io::Result<Leaders<'_>> {
        self.leader_status()
            .complete(self.hour(), TableId::UnitLeaders)?;
        Ok(self.leader_rows())
    }
}

pub trait DecodedSource: RecordedSource + sealed::Decoded {
    fn decoded_frame(&self) -> &DecodedFrame;
}

mod sealed {
    pub trait Decoded {}
    impl Decoded for super::DecodedFrame {}
    impl<T: super::DecodedSource + ?Sized> Decoded for Box<T> {}
    impl<T: super::DecodedSource + ?Sized> Decoded for super::Arc<T> {}
    impl<T: super::DecodedSource + ?Sized> Decoded for &T {}
}

impl DecodedSource for DecodedFrame {
    fn decoded_frame(&self) -> &DecodedFrame {
        self
    }
}

impl dyn DecodedSource + '_ {
    pub fn view(&self, index: usize) -> &TableView {
        &self.decoded_frame().tables[index]
    }
    pub fn shared(&self, index: usize) -> SharedView {
        self.decoded_frame().tables[index].clone()
    }
    pub fn complete<T: TableSchema>(&self) -> io::Result<&[T::Row]> {
        self.decoded_frame().complete::<T>()
    }
    pub fn row_gap(&self, table: TableId) -> String {
        super::undeclared::status(|| OperandStatus::of(self.view(table.index())).gap())
    }
    pub fn complete_status(&self, table: TableId) -> io::Result<()> {
        super::undeclared::status(|| OperandStatus::of(self.view(table.index())).complete(self.hour(), table))
    }
    pub fn character_rows(&self) -> Rows<'_, replay_data::rows::CharacterRow> {
        characters::rows(self.view(TableId::Characters.index()))
    }
    pub fn portraits(&self) -> io::Result<Rows<'_, replay_data::rows::CharacterPortraitRow>> {
        self.complete::<replay_data::rows::tables::CharacterPortraits>()
            .map(Into::into)
    }
    pub fn character_status(&self) -> OperandStatus<'_> {
        self.status(TableId::Characters)
    }
    pub fn characters(&self) -> io::Result<Rows<'_, replay_data::rows::CharacterRow>> {
        self.complete::<replay_data::rows::tables::Characters>()
            .map(Into::into)
    }
    pub fn shared_characters(&self) -> SharedCharacters {
        self.shared(TableId::Characters.index()).into()
    }
}

impl RecordedSource for DecodedFrame {
    fn hour(&self) -> Hour {
        self.hour
    }
    fn branch(&self) -> u64 {
        self.branch
    }
    fn status(&self, table: TableId) -> OperandStatus<'_> {
        super::undeclared::status(|| OperandStatus::for_table(&self.tables[table.index()], table))
    }
    fn sampled_at(&self, index: usize) -> Option<Hour> {
        self.tables[index].sampled_at()
    }
    fn identity(&self, index: usize) -> Option<ViewIdentity> {
        Some(self.tables[index].identity())
    }
    fn clock_seed(&self) -> io::Result<Option<i32>> {
        self.complete::<replay_data::rows::tables::Clock>()
            .map(|rows| rows.first().map(|row| row.seed))
    }
    fn lexical_rows(&self) -> Rows<'_, replay_data::rows::LexicalTokenRow> {
        lexical::lexical_rows(&self.tables[TableId::LexicalTokens.index()])
    }
    fn shared_lexical(&self) -> SharedLexical {
        self.tables[TableId::LexicalTokens.index()].clone().into()
    }
    fn command_allocations(&self) -> io::Result<CommandAllocations<'_>> {
        self.complete::<replay_data::rows::tables::CountryCommandPower>()
            .map(CommandAllocations::decoded)
    }
    fn province_rows(&self) -> Provinces<'_> {
        provinces::rows(&self.tables[TableId::Provinces.index()])
    }
    fn divisions(&self) -> io::Result<Divisions<'_>> {
        self.complete::<crate::rows::tables::Divisions>()
            .map(|rows| Divisions(rows.iter()))
    }
    fn division_names(&self) -> io::Result<DivisionNames<'_>> {
        self.complete::<crate::rows::tables::DivisionStatus>()
            .map(|rows| DivisionNames(rows.iter()))
    }
    fn leader_rows(&self) -> Leaders<'_> {
        let rows = match &*self.tables[TableId::UnitLeaders.index()] {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows
                .rows::<crate::rows::tables::UnitLeaders>()
                .unwrap_or_default(),
            _ => &[],
        };
        Leaders(rows.iter())
    }
    fn leader(&self, reference: u64) -> io::Result<Option<Leader<'_>>> {
        let rows = self.complete::<crate::rows::tables::UnitLeaders>()?;
        Ok(rows
            .binary_search_by_key(&reference, |row| {
                (u64::from(row.id) << 32) | u64::from(row.reference_type)
            })
            .ok()
            .map(|at| Leader::from(&rows[at])))
    }
}

macro_rules! forwarded_source {
    ($wrapper:ty) => {
        impl<T: RecordedSource + ?Sized> RecordedSource for $wrapper {
            fn hour(&self) -> Hour {
                (**self).hour()
            }
            fn branch(&self) -> u64 {
                (**self).branch()
            }
            fn status(&self, table: TableId) -> OperandStatus<'_> {
                (**self).status(table)
            }
            fn sampled_at(&self, index: usize) -> Option<Hour> {
                (**self).sampled_at(index)
            }
            fn identity(&self, index: usize) -> Option<ViewIdentity> {
                (**self).identity(index)
            }
            fn clock_seed(&self) -> io::Result<Option<i32>> {
                (**self).clock_seed()
            }
            fn lexical_rows(&self) -> Rows<'_, replay_data::rows::LexicalTokenRow> {
                (**self).lexical_rows()
            }
            fn shared_lexical(&self) -> SharedLexical {
                (**self).shared_lexical()
            }
            fn command_allocations(&self) -> io::Result<CommandAllocations<'_>> {
                (**self).command_allocations()
            }
            fn province_rows(&self) -> Provinces<'_> {
                (**self).province_rows()
            }
            fn divisions(&self) -> io::Result<Divisions<'_>> {
                (**self).divisions()
            }
            fn division_names(&self) -> io::Result<DivisionNames<'_>> {
                (**self).division_names()
            }
            fn leader_rows(&self) -> Leaders<'_> {
                (**self).leader_rows()
            }
            fn leader(&self, reference: u64) -> io::Result<Option<Leader<'_>>> {
                (**self).leader(reference)
            }
            fn lexical_status(&self) -> OperandStatus<'_> {
                (**self).lexical_status()
            }
            fn lexical(&self) -> io::Result<Rows<'_, replay_data::rows::LexicalTokenRow>> {
                (**self).lexical()
            }
            fn province_status(&self) -> OperandStatus<'_> {
                (**self).province_status()
            }
            fn provinces(&self) -> io::Result<Provinces<'_>> {
                (**self).provinces()
            }
            fn leader_status(&self) -> OperandStatus<'_> {
                (**self).leader_status()
            }
            fn leaders(&self) -> io::Result<Leaders<'_>> {
                (**self).leaders()
            }
        }
        impl<T: DecodedSource + ?Sized> DecodedSource for $wrapper {
            fn decoded_frame(&self) -> &DecodedFrame {
                (**self).decoded_frame()
            }
        }
    };
}
forwarded_source!(Box<T>);
forwarded_source!(Arc<T>);
forwarded_source!(&T);

#[cfg(test)]
mod tests;
