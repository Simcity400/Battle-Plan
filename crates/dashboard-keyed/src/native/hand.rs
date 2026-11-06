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
    frame::{SharedView, TableView},
};
use std::{io, mem::size_of};

/// One table's hand, with the hour's last acquisition of it.
#[derive(replay_spool::Spooled)]
pub(super) enum Hand {
    Same(Option<Hour>),
    Changed(RowChanges, Option<Hour>),
    Whole(SharedView),
}

impl Hand {
    /// `lent`, the table as the world lends it at the hour, as `touch` says the hour changed it;
    /// and the bytes the hand holds.
    pub(super) fn of(lent: &SharedView, touch: &Touch) -> io::Result<(Self, usize)> {
        let sampled = lent.sampled_at();
        if !touch.any() {
            return Ok((Self::Same(sampled), 0));
        }
        if !touch.restated
            && let TableView::Complete(rows) = &**lent
        {
            let keys: Vec<u128> = touch.written.iter().map(|&(key, _)| key).collect();
            let changes = rows.rows_at(&keys, touch.deleted.clone())?;
            let bytes = changes.heap_bytes();
            return Ok((Self::Changed(changes, sampled), bytes));
        }
        let whole = SharedView::from((**lent).clone()).with_sampled_at(sampled);
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

    fn tokens(rows: &[(u32, &str)]) -> SharedView {
        SharedView::from(TableView::Complete(TableRows::LexicalTokens(
            rows.iter()
                .map(|&(token, literal)| LexicalTokenRow {
                    token,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
        )))
    }

    /// The held table and an hour's hand give the table the world lends: written, inserted and
    /// deleted rows by key, an unchanged table as it was, a restated one whole.
    #[test]
    fn a_hand_brings_the_held_table_to_the_lent_one() {
        let held = tokens(&[(1, "a"), (2, "b"), (3, "c")]);
        let lent = tokens(&[(1, "a"), (2, "B"), (4, "d")]);
        let touch = Touch {
            restated: false,
            written: vec![(2, 1), (4, u64::MAX)],
            deleted: vec![3],
        };
        let (hand, bytes) = Hand::of(&lent, &touch).unwrap();
        assert!(matches!(hand, Hand::Changed(..)) && bytes > 0);
        assert_eq!(*hand.apply(held.clone()).unwrap(), *lent);
        let (same, bytes) = Hand::of(&held, &Touch::default()).unwrap();
        assert!(matches!(same, Hand::Same(_)) && bytes == 0);
        assert_eq!(*same.apply(held.clone()).unwrap(), *held);
        let restated = Touch {
            restated: true,
            ..Touch::default()
        };
        let (whole, _) = Hand::of(&lent, &restated).unwrap();
        assert!(matches!(whole, Hand::Whole(_)));
        assert_eq!(*whole.apply(held).unwrap(), *lent);
    }

    /// A hand carries the hour's last acquisition of its table, applies an update to a table
    /// the emit alone holds in place, and leaves a table another holder shares as it was.
    #[test]
    fn a_hand_dates_its_table_and_updates_it_in_place_where_alone() {
        let lent = tokens(&[(1, "a"), (2, "B")]).with_sampled_at(Some(7));
        let update = Touch {
            restated: false,
            written: vec![(2, 1)],
            deleted: Vec::new(),
        };
        let rows = |view: &SharedView| match &**view {
            TableView::Complete(rows) => rows
                .rows::<replay_data::rows::tables::LexicalTokens>()
                .unwrap()
                .as_ptr(),
            _ => unreachable!(),
        };
        let alone = tokens(&[(1, "a"), (2, "b")]).with_sampled_at(Some(6));
        let before = rows(&alone);
        let (hand, _) = Hand::of(&lent, &update).unwrap();
        let applied = hand.apply(alone).unwrap();
        assert_eq!(*applied, *lent);
        assert_eq!(applied.sampled_at(), Some(7));
        assert_eq!(rows(&applied), before, "updated in place");
        let shared = tokens(&[(1, "a"), (2, "b")]);
        let other = shared.clone();
        let (hand, _) = Hand::of(&lent, &update).unwrap();
        assert_eq!(*hand.apply(shared).unwrap(), *lent);
        assert_eq!(*other, *tokens(&[(1, "a"), (2, "b")]), "the other holder's table is as it was");
        let (same, _) = Hand::of(&lent, &Touch::default()).unwrap();
        assert_eq!(same.apply(tokens(&[(1, "a"), (2, "B")])).unwrap().sampled_at(), Some(7));
        let restated = Touch {
            restated: true,
            ..Touch::default()
        };
        let (whole, _) = Hand::of(&lent, &restated).unwrap();
        assert_eq!(whole.apply(other).unwrap().sampled_at(), Some(7));
    }

    /// A table held truncated and lent complete at the next hour moved its status, which
    /// restates it: it comes whole, never as changes to a table not held complete.
    #[test]
    fn a_table_completing_comes_whole() {
        let truncated = SharedView::from(TableView::Truncated {
            rows: TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"a".to_vec(),
            }]),
            missing: 1,
        });
        let lent = tokens(&[(1, "a"), (2, "b")]);
        let moved = Touch {
            restated: true,
            ..Touch::default()
        };
        let (hand, _) = Hand::of(&lent, &moved).unwrap();
        assert!(matches!(hand, Hand::Whole(_)));
        assert_eq!(*hand.apply(truncated.clone()).unwrap(), *lent);
        let changes = Touch {
            restated: false,
            written: vec![(2, u64::MAX)],
            deleted: Vec::new(),
        };
        let (hand, _) = Hand::of(&lent, &changes).unwrap();
        assert!(hand.apply(truncated).is_err(), "changes never apply to a table not held complete");
    }
}
