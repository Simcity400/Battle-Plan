use replay_data::rows::AirWingRow;
mod prepared;
mod windows;
pub use prepared::{Census, PreparedItem};
pub use windows::{Composed, View};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wing<'a> {
    pub row: &'a AirWingRow,
    pub selected: bool,
    pub archetype: Option<usize>,
    pub group: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub members: Vec<u64>,
    pub selected: bool,
    pub archetype: Option<usize>,
    pub group: u64,
    pub planes: i32,
}

impl Item {
    fn new(wing: Wing<'_>) -> Self {
        Self {
            members: vec![wing.row.id],
            selected: wing.selected,
            archetype: wing.archetype,
            group: wing.group,
            planes: wing.row.planes,
        }
    }

    fn admits(&self, wing: Wing<'_>, separate_selected: bool) -> bool {
        !(separate_selected && (self.selected || wing.selected))
            && self.selected == wing.selected
            && self.group == wing.group
            && self.archetype == wing.archetype
    }

    fn append(&mut self, wing: Wing<'_>) {
        self.members.push(wing.row.id);
        self.planes = self.planes.wrapping_add(wing.row.planes);
    }
}

pub fn items<'a>(
    wings: impl IntoIterator<Item = Wing<'a>>,
    separate_selected: bool,
) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for wing in wings {
        if let Some(item) = items
            .iter_mut()
            .find(|item| item.admits(wing, separate_selected))
        {
            item.append(wing);
        } else {
            items.push(Item::new(wing));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::{AirWingRow, Wing, items};

    fn wing(row: &AirWingRow) -> Wing<'_> {
        Wing {
            row,
            selected: false,
            archetype: Some(3),
            group: 0,
        }
    }

    #[test]
    fn first_fit_keeps_native_order_repeated_members_and_separate_selected_wings() {
        let rows: Vec<_> = (1..=5)
            .map(|id| AirWingRow {
                id,
                planes: 12,
                ..Default::default()
            })
            .collect();
        let mut inputs: Vec<_> = rows.iter().map(wing).collect();
        inputs[1].selected = true;
        inputs[2].group = 8;
        inputs[3].selected = true;
        inputs.push(inputs[0]);
        let grouped = items(inputs.clone(), false);
        assert_eq!(
            grouped
                .iter()
                .map(|item| item.members.clone())
                .collect::<Vec<_>>(),
            [vec![1, 5, 1], vec![2, 4], vec![3]]
        );
        let split = items(inputs, true);
        assert_eq!(
            split
                .iter()
                .map(|item| item.members.clone())
                .collect::<Vec<_>>(),
            [vec![1, 5, 1], vec![2], vec![3], vec![4]]
        );
        assert_eq!(split[0].planes, 36);
    }

    #[test]
    fn grouped_plane_counts_keep_native_wrapping() {
        let rows = [
            AirWingRow {
                id: 1,
                planes: i32::MAX,
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                planes: 1,
                ..Default::default()
            },
        ];
        assert_eq!(items(rows.iter().map(wing), false)[0].planes, i32::MIN);
    }
}
