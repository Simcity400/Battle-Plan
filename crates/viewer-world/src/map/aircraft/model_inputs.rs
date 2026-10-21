//! Hour-local aircraft model results and the same signed primary-equipment fold.
use replay_data::rows::{WingEquipmentRow, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
};

type Selected = Result<Option<String>, String>;

/// One build call's immutable model selection results. Raw country references
/// stay distinct, and failures and known absence are retained just like names.
/// Construct a fresh memo for each hour, content and authored limit context.
#[derive(Default)]
pub struct ModelMemo {
    selected: RefCell<BTreeMap<(u64, u32), Selected>>,
}

impl ModelMemo {
    pub fn resolve(
        &self,
        variant: u64,
        raw_country: u32,
        read: impl FnOnce() -> Selected,
    ) -> Selected {
        self.selected
            .borrow_mut()
            .entry((variant, raw_country))
            .or_insert_with(read)
            .clone()
    }
}

pub(super) struct Primary<'a> {
    frame: &'a DecodedFrame,
    // Keep acquisition lazy: hours with no emitted aircraft never demand this table.
    index: OnceCell<Result<BTreeMap<u64, u64>, String>>,
}

impl<'a> Primary<'a> {
    pub(super) fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            index: OnceCell::new(),
        }
    }

    pub(super) fn get(&self, wing: u64) -> Result<Option<u64>, String> {
        self.index
            .get_or_init(|| {
                self.frame
                    .complete::<tables::WingEquipment>()
                    .map(index)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map(|index| index.get(&wing).copied())
            .map_err(Clone::clone)
    }
}

fn index(rows: &[WingEquipmentRow]) -> BTreeMap<u64, u64> {
    let mut best = BTreeMap::<u64, &WingEquipmentRow>::new();
    for row in rows {
        let old = best.entry(row.wing).or_insert(row);
        if row.amount_raw > old.amount_raw
            || (row.amount_raw == old.amount_raw && row.ordinal < old.ordinal)
        {
            *old = row;
        }
    }
    best.into_iter()
        .map(|(wing, row)| (wing, row.variant))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_primary_keeps_signed_amount_native_ordinal_and_first_exact_tie() {
        let rows = [
            (1, 9, 2, 400),
            (1, 7, 0, 400),
            (1, 3, 1, 200),
            (1, 8, 0, 400),
            (2, 9, 2, i64::MIN),
            (2, 7, 0, -100_000),
            (2, 3, 1, -200_000),
            (3, 3, 1, 0),
            (3, 4, 0, 0),
        ]
        .map(|(wing, variant, ordinal, amount_raw)| WingEquipmentRow {
            wing,
            variant,
            ordinal,
            amount_raw,
        });
        assert_eq!(index(&rows), [(1, 7), (2, 7), (3, 4)].into());
    }

    #[test]
    fn model_memo_keeps_raw_country_identity_absence_and_failure() {
        let memo = ModelMemo::default();
        let mut calls = 0;
        for _ in 0..3 {
            assert_eq!(
                memo.resolve(1, 1, || {
                    calls += 1;
                    Ok(None)
                }),
                Ok(None)
            );
            assert_eq!(
                memo.resolve(1, 2, || {
                    calls += 1;
                    Ok(Some("other country".into()))
                }),
                Ok(Some("other country".into()))
            );
            assert_eq!(
                memo.resolve(2, 1, || {
                    calls += 1;
                    Err("variant unavailable".into())
                }),
                Err("variant unavailable".into())
            );
        }
        assert_eq!(calls, 3);
    }
}
