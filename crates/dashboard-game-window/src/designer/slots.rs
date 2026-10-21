//! Which cells of a template's composition grids the designer attaches, and in which state.
//!
//! The designer walks each grid's recorded dimensions column by column and classifies every
//! coordinate from the number of real definitions in its column, the column's usable height
//! and whether the template may be edited ([`state`]). Holes keep their place: a column's count
//! decides the state while the cell itself decides the drawing. A state that depends on an
//! operand the recording lacks is refused for that cell by name, never guessed.

use replay_data::rows::{TemplateGrid, TemplateLayout};

/// The three ordinary composition grids, in the order the designer fills them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKind {
    Regiments,
    Support,
    RegimentalSupport,
}

impl GridKind {
    pub const ALL: [Self; 3] = [Self::Regiments, Self::Support, Self::RegimentalSupport];

    /// The grid's element path below the designer's root.
    pub fn path(self) -> &'static str {
        match self {
            Self::Regiments => "subunits/regiments_grid",
            Self::Support => "subunits/support_grid",
            Self::RegimentalSupport => "subunits/regiment_support_grid",
        }
    }

    /// The entry window the designer creates for the grid's cells: regimental support uses the
    /// support entry.
    pub fn entry(self) -> &'static str {
        match self {
            Self::Regiments => super::REGIMENT_ENTRY,
            Self::Support | Self::RegimentalSupport => super::SUPPORT_ENTRY,
        }
    }

    pub fn grid(self, layout: &TemplateLayout) -> &TemplateGrid {
        match self {
            Self::Regiments => &layout.regiments,
            Self::Support => &layout.support,
            Self::RegimentalSupport => &layout.regimental_support,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Regiments => "regiment",
            Self::Support => "support",
            Self::RegimentalSupport => "regimental support",
        }
    }
}

/// A cell's state, in the designer's own numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    /// 0: inside the column's filled region.
    Filled,
    /// 1: the next row of an editable column, where a unit can be added.
    Addable,
    /// 2: a locked placeholder.
    Locked,
    /// 3: the column's limit.
    Blocked,
    /// 4: not attached.
    Detached,
}

/// What decides a cell's state beyond its column: the column's usable height and whether the
/// template is editable, each as every value the recording allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operands {
    /// The usable heights still possible, ascending; empty when none is.
    pub limits: Vec<i32>,
    /// `None` when whether the template is editable is unknown.
    pub editable: Option<bool>,
    pub hq: bool,
}

/// The state of row `row` of column `column`, from the real-definition count of every column
/// of the grid, for one usable height and editability.
pub fn state(
    kind: GridKind,
    counts: &[i32],
    column: usize,
    row: i32,
    limit: i32,
    editable: bool,
    hq: bool,
) -> SlotState {
    let n = counts[column];
    if kind != GridKind::RegimentalSupport && column > 0 && n == 0 && counts[column - 1] == 0 {
        return if row == 0 {
            if limit != 0 {
                SlotState::Locked
            } else {
                SlotState::Blocked
            }
        } else if hq {
            SlotState::Locked
        } else {
            SlotState::Detached
        };
    }
    if row < n {
        SlotState::Filled
    } else if row == n {
        if row == limit {
            SlotState::Blocked
        } else if editable {
            SlotState::Addable
        } else {
            SlotState::Locked
        }
    } else if editable && row == n + 1 {
        if row < limit {
            SlotState::Locked
        } else {
            SlotState::Blocked
        }
    } else if hq {
        SlotState::Locked
    } else {
        SlotState::Detached
    }
}

/// The state every allowed operand gives, or why they disagree.
pub fn decided(
    kind: GridKind,
    counts: &[i32],
    column: usize,
    row: i32,
    operands: &Operands,
    unknown_limit: &str,
) -> Result<SlotState, String> {
    let editable: &[bool] = match operands.editable {
        Some(true) => &[true],
        Some(false) => &[false],
        None => &[true, false],
    };
    let mut states = operands.limits.iter().flat_map(|&limit| {
        editable
            .iter()
            .map(move |&editable| state(kind, counts, column, row, limit, editable, operands.hq))
    });
    let first = states
        .next()
        .ok_or_else(|| format!("the {} grid has no usable height", kind.label()))?;
    if states.all(|other| other == first) {
        return Ok(first);
    }
    let limited = operands.limits.len() > 1;
    Err(match (limited, operands.editable.is_none()) {
        (true, true) => format!("{unknown_limit}, and whether the template is editable is unknown"),
        (true, false) => unknown_limit.to_owned(),
        _ => "whether the template is editable is unknown".to_owned(),
    })
}

/// A real definition: an occupied cell holding a definition other than the null object.
pub fn real(grid: &TemplateGrid, column: u32, row: u32) -> Option<(u32, bool)> {
    let index = usize::try_from(u64::from(grid.rows) * u64::from(column) + u64::from(row)).ok()?;
    grid.cells
        .get(index)
        .filter(|cell| cell.occupied && cell.token != 0)
        .map(|cell| (cell.token, cell.local != 0))
}

/// Each column's count of real definitions.
pub fn counts(grid: &TemplateGrid) -> Vec<i32> {
    (0..grid.columns)
        .map(|column| {
            (0..grid.rows)
                .filter(|&row| real(grid, column, row).is_some())
                .count() as i32
        })
        .collect()
}

/// The regimental support row a regiment column of `battalions` real definitions unlocks: the
/// highest one-based position among `thresholds` whose value it meets, zero for none.
pub fn regimental_support_rows(thresholds: &[i32], battalions: i32) -> i32 {
    thresholds
        .iter()
        .enumerate()
        .filter(|(_, threshold)| battalions >= **threshold)
        .map(|(at, _)| at as i32 + 1)
        .max()
        .unwrap_or(0)
}

/// One attached cell: its slot, state and the real definition it holds, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub slot: [i32; 2],
    pub state: SlotState,
    /// The definition's token and whether it is a country-local clone.
    pub definition: Option<(u32, bool)>,
}

/// The cells of one grid the designer attaches, and the cells whose state is refused, each
/// with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub cells: Vec<Cell>,
    pub refused: Vec<String>,
}

/// The cells of `kind`'s grid, each column's usable height given by `limits`.
pub fn plan(
    kind: GridKind,
    grid: &TemplateGrid,
    limits: impl Fn(usize) -> Operands,
    unknown_limit: &str,
) -> Plan {
    let counts = counts(grid);
    let mut plan = Plan::default();
    for column in 0..grid.columns {
        let operands = limits(column as usize);
        for row in 0..grid.rows {
            let state = decided(
                kind,
                &counts,
                column as usize,
                row as i32,
                &operands,
                unknown_limit,
            );
            match state {
                Ok(SlotState::Detached) => {}
                Ok(state) => plan.cells.push(Cell {
                    slot: [column as i32, row as i32],
                    state,
                    definition: real(grid, column, row),
                }),
                Err(why) => plan.refused.push(why),
            }
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TemplateCell;

    fn grid(columns: u32, rows: u32, filled: &[&[bool]]) -> TemplateGrid {
        let mut cells = Vec::new();
        for column in filled {
            for row in 0..rows as usize {
                let real = column.get(row).copied().unwrap_or(false);
                cells.push(TemplateCell {
                    token: u32::from(real) * 7,
                    local: 0,
                    occupied: real,
                });
            }
        }
        TemplateGrid {
            columns,
            rows,
            cells,
        }
    }

    fn fixed(limit: i32, editable: bool) -> Operands {
        Operands {
            limits: vec![limit],
            editable: Some(editable),
            hq: false,
        }
    }

    /// A filled column, its addable next row and the locked row after it; the next empty
    /// column starts locked, the one after that is not attached.
    #[test]
    fn columns_follow_the_designers_order() {
        let regiments = grid(3, 5, &[&[true, true, true]]);
        let plan = plan(
            GridKind::Regiments,
            &regiments,
            |_| fixed(5, true),
            "unknown",
        );
        let states: Vec<_> = plan
            .cells
            .iter()
            .map(|cell| (cell.slot, cell.state))
            .collect();
        assert_eq!(
            states,
            [
                ([0, 0], SlotState::Filled),
                ([0, 1], SlotState::Filled),
                ([0, 2], SlotState::Filled),
                ([0, 3], SlotState::Addable),
                ([0, 4], SlotState::Locked),
                ([1, 0], SlotState::Addable),
                ([1, 1], SlotState::Locked),
                ([2, 0], SlotState::Locked),
            ]
        );
        assert!(plan.refused.is_empty());
        // A locked template adds nothing: the next row is a locked placeholder only.
        let locked = super::plan(
            GridKind::Regiments,
            &regiments,
            |_| fixed(5, false),
            "unknown",
        );
        assert_eq!(locked.cells[3].state, SlotState::Locked);
        assert_eq!(locked.cells.len(), 6);
    }

    /// The column's limit blocks its next row, and a hole keeps its place in the filled region.
    #[test]
    fn limits_and_holes() {
        let full = grid(1, 4, &[&[true, false, true, true]]);
        let plan = plan(GridKind::Regiments, &full, |_| fixed(3, true), "unknown");
        assert_eq!(plan.cells[1].state, SlotState::Filled);
        assert_eq!(plan.cells[1].definition, None);
        assert_eq!(plan.cells[3].state, SlotState::Blocked);
    }

    /// An unknown height decides a cell only when every possible height agrees.
    #[test]
    fn an_unknown_height_refuses_only_what_it_decides() {
        let regiments = grid(1, 5, &[&[true]]);
        let operands = Operands {
            limits: vec![4, 5],
            editable: Some(true),
            hq: false,
        };
        let plan = plan(
            GridKind::Regiments,
            &regiments,
            |_| operands.clone(),
            "height unknown",
        );
        assert_eq!(
            plan.cells.iter().map(|cell| cell.state).collect::<Vec<_>>(),
            [SlotState::Filled, SlotState::Addable, SlotState::Locked]
        );
        let tall = grid(1, 5, &[&[true, true, true]]);
        let plan = super::plan(
            GridKind::Regiments,
            &tall,
            |_| operands.clone(),
            "height unknown",
        );
        // Row 3 is addable whatever the height; row 4 is locked below a height of five and
        // blocked at four.
        assert_eq!(plan.cells.len(), 4);
        assert_eq!(plan.refused, ["height unknown"]);
    }

    #[test]
    fn regimental_support_rows_take_the_highest_met_position() {
        assert_eq!(regimental_support_rows(&[3], 2), 0);
        assert_eq!(regimental_support_rows(&[3], 3), 1);
        assert_eq!(regimental_support_rows(&[4, 1], 2), 2);
        assert_eq!(regimental_support_rows(&[], 9), 0);
    }
}
