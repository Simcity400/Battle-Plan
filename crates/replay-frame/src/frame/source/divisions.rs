use replay_data::rows::{DivisionRow, DivisionStatusRow, NameOperands, Recorded};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Division {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
}

impl From<&DivisionRow> for Division {
    fn from(row: &DivisionRow) -> Self {
        Self {
            id: row.id,
            roster_owner: row.roster_owner,
            logical_owner: row.logical_owner,
            expeditionary_owner: row.expeditionary_owner,
            province: row.province,
            template: row.template,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DivisionName<'a> {
    pub division: u64,
    pub name: &'a Recorded<Option<Vec<u8>>>,
    pub naming: &'a Recorded<Option<NameOperands>>,
    pub country_ordinal: &'a Recorded<Option<u32>>,
}

impl<'a> From<&'a DivisionStatusRow> for DivisionName<'a> {
    fn from(row: &'a DivisionStatusRow) -> Self {
        Self {
            division: row.division,
            name: &row.name,
            naming: &row.naming,
            country_ordinal: &row.country_ordinal,
        }
    }
}

pub struct Divisions<'a>(pub(super) std::slice::Iter<'a, DivisionRow>);
impl Divisions<'_> {
    pub fn iter(self) -> Self {
        self
    }
}
impl Iterator for Divisions<'_> {
    type Item = Division;
    fn next(&mut self) -> Option<Division> {
        self.0.next().map(Division::from)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

pub struct DivisionNames<'a>(pub(super) std::slice::Iter<'a, DivisionStatusRow>);
impl DivisionNames<'_> {
    pub fn iter(self) -> Self {
        self
    }
}
impl<'a> Iterator for DivisionNames<'a> {
    type Item = DivisionName<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(DivisionName::from)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
