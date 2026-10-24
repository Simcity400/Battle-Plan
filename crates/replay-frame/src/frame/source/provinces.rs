use super::TableView;
use replay_data::rows::{ProvinceRow, Recorded, tables};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Province {
    pub id: u32,
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
    pub victory_points: i32,
    pub controller_area: Recorded<Option<u32>>,
}

impl From<&ProvinceRow> for Province {
    fn from(row: &ProvinceRow) -> Self {
        Self {
            id: row.id,
            state: row.state,
            owner: row.owner,
            controller: row.controller,
            victory_points: row.victory_points,
            controller_area: row.controller_area,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Provinces<'a>(&'a [ProvinceRow]);
impl<'a> Provinces<'a> {
    pub fn iter(self) -> impl Iterator<Item = Province> + 'a {
        self.0.iter().map(Province::from)
    }
    pub fn len(self) -> usize {
        self.0.len()
    }
    pub fn is_empty(self) -> bool {
        self.0.is_empty()
    }
    pub fn get(self, index: usize) -> Option<Province> {
        self.0.get(index).map(Province::from)
    }
    pub fn binary_search_by_key<K: Ord>(
        self,
        key: &K,
        mut of: impl FnMut(&Province) -> K,
    ) -> Result<usize, usize> {
        self.0
            .binary_search_by_key(key, |row| of(&Province::from(row)))
    }
}
impl<'a> IntoIterator for Provinces<'a> {
    type Item = Province;
    type IntoIter = std::iter::Map<std::slice::Iter<'a, ProvinceRow>, fn(&ProvinceRow) -> Province>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(province)
    }
}

fn province(row: &ProvinceRow) -> Province {
    Province::from(row)
}

pub(super) fn rows(view: &TableView) -> Provinces<'_> {
    Provinces(match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<tables::Provinces>().unwrap_or_default()
        }
        _ => &[],
    })
}
