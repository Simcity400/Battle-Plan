//! What a keyed family's segment operator hands its branch's emit of one table at an hour it
//! observes: nothing where the hour left the table as it was, the rows it wrote and the keys it
//! deleted where it changed them, and the table whole only where the hour restated it (a
//! segment's first hour, or its status moved). The emit holds the one copy of each table its
//! observer reads and applies each hour's hand to it in place, so no hour's hand holds a table.

use crate::segment::Touch;
use replay::columnar_frames::view_heap_bytes;
use replay_data::rows::RowChanges;
use replay_frame::{
    Hour,
    frame::{ColumnView, SharedView, TableView},
};
use std::{io, mem::size_of};

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// One table's hand, with the hour's last acquisition of it.
#[derive(replay_spool::Spooled)]
pub(super) enum Hand {
    Same(Option<Hour>),
    Changed(RowChanges, Option<Hour>),
    Whole(SharedView),
}

impl Hand {
    /// `view`, the table through the walk's views at the hour, last acquired at `sampled`, as
    /// `touch` says the hour changed it; and the bytes the hand holds. Its rows hold the fields
    /// the observer reads (`fields`), built from the walker's columns: only the rows written, or
    /// the table whole only where restated.
    pub(super) fn of(
        view: ColumnView<'_>,
        sampled: Option<Hour>,
        touch: &Touch,
        fields: u64,
    ) -> io::Result<(Self, usize)> {
        if !touch.any() {
            return Ok((Self::Same(sampled), 0));
        }
        if !touch.restated
            && let ColumnView::Complete(rows) = view
        {
            let mut slots = engine_allocation::vec(touch.written.len())?;
            for &(key, _) in &touch.written {
                slots.push(
                    rows.find(key)
                        .ok_or_else(|| invalid("A written row is not in its table"))?,
                );
            }
            let changes = RowChanges::new(rows.rows_at(&slots, fields)?, touch.deleted.clone());
            let bytes = changes.heap_bytes();
            return Ok((Self::Changed(changes, sampled), bytes));
        }
        let whole = view.to_table_view(fields)?;
        let whole = SharedView::from(whole).with_sampled_at(sampled);
        let bytes = view_heap_bytes(&whole);
        Ok((Self::Whole(whole), bytes))
    }

    /// Bytes a hand holds besides its tables' changes.
    pub(super) const BYTES: usize = size_of::<Self>();

    /// The table `held` becomes by this hand: as it was, with this hour's changes applied in
    /// place where the emit is its only holder, or the restated table.
    pub(super) fn apply(self, held: SharedView) -> io::Result<SharedView> {
        match self {
            Self::Same(sampled) => Ok(held.with_sampled_at(sampled)),
            Self::Whole(whole) => Ok(whole),
            Self::Changed(changes, sampled) => {
                let view = held.try_unwrap().unwrap_or_else(|shared| (*shared).clone());
                let TableView::Complete(rows) = view else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "A keyed table's changes came to a table not held complete",
                    ));
                };
                Ok(SharedView::from(TableView::Complete(rows.apply_changes(changes)?))
                    .with_sampled_at(sampled))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{LexicalTokenRow, TableRows};

    fn rows(rows: &[(u32, &str)]) -> TableRows {
        TableRows::LexicalTokens(
            rows.iter()
                .map(|&(token, literal)| LexicalTokenRow {
                    token,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
        )
    }

    fn tokens(held: &[(u32, &str)]) -> SharedView {
        SharedView::from(TableView::Complete(rows(held)))
    }

    fn changed(written: &[(u32, &str)], deleted: &[u128], sampled: Option<Hour>) -> Hand {
        Hand::Changed(RowChanges::new(rows(written), deleted.to_vec()), sampled)
    }

    /// The held table and an hour's hand give the table the walk holds: written, inserted and
    /// deleted rows by key, an unchanged table as it was, a restated one whole.
    #[test]
    fn a_hand_brings_the_held_table_to_the_walked_one() {
        let held = tokens(&[(1, "a"), (2, "b"), (3, "c")]);
        let walked = tokens(&[(1, "a"), (2, "B"), (4, "d")]);
        let hand = changed(&[(2, "B"), (4, "d")], &[3], None);
        assert_eq!(*hand.apply(held.clone()).unwrap(), *walked);
        assert_eq!(*Hand::Same(None).apply(held.clone()).unwrap(), *held);
        let whole = Hand::Whole(walked.clone());
        assert_eq!(*whole.apply(held).unwrap(), *walked);
    }

    /// A hand dates its table, applies an update to a table the emit alone holds in place, and
    /// leaves a table another holder shares as it was.
    #[test]
    fn a_hand_dates_its_table_and_updates_it_in_place_where_alone() {
        let first = |view: &SharedView| match &**view {
            TableView::Complete(rows) => rows
                .rows::<replay_data::rows::tables::LexicalTokens>()
                .unwrap()
                .as_ptr(),
            _ => unreachable!(),
        };
        let alone = tokens(&[(1, "a"), (2, "b")]).with_sampled_at(Some(6));
        let before = first(&alone);
        let applied = changed(&[(2, "B")], &[], Some(7)).apply(alone).unwrap();
        assert_eq!(*applied, *tokens(&[(1, "a"), (2, "B")]));
        assert_eq!(applied.sampled_at(), Some(7));
        assert_eq!(first(&applied), before, "updated in place");
        let shared = tokens(&[(1, "a"), (2, "b")]);
        let other = shared.clone();
        let applied = changed(&[(2, "B")], &[], None).apply(shared).unwrap();
        assert_eq!(*applied, *tokens(&[(1, "a"), (2, "B")]));
        assert_eq!(*other, *tokens(&[(1, "a"), (2, "b")]), "the other holder's table is as it was");
        assert_eq!(Hand::Same(Some(7)).apply(other).unwrap().sampled_at(), Some(7));
    }

    /// Changes never apply to a table not held complete: a table completing comes whole.
    #[test]
    fn changes_never_apply_to_a_truncated_table() {
        let truncated = SharedView::from(TableView::Truncated {
            rows: rows(&[(1, "a")]),
            missing: 1,
        });
        assert!(changed(&[(2, "b")], &[], None).apply(truncated.clone()).is_err());
        let whole = tokens(&[(1, "a"), (2, "b")]);
        assert_eq!(*Hand::Whole(whole.clone()).apply(truncated).unwrap(), *whole);
    }
}
