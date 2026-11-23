use map_display_data::{retained::Retained, value_index::ValueIndex};
use replay_data::rows::{RowCodec, TableId};
use replay_frame::frame::{DecodedFrame, HourInput, TableChanges, ViewIdentity};
use std::io;

#[derive(Clone)]
pub struct RowFold<V> {
    pub identity: ViewIdentity,
    pub values: ValueIndex<u128, V>,
    pub edits: Retained<Vec<u128>>,
}
pub fn fields(table: TableId, names: &[&str]) -> u64 {
    names
        .iter()
        .try_fold(0, |mask, name| {
            (0..table.schema().len())
                .find(|&bit| table.field_name(bit) == Some(*name))
                .map(|bit| mask | (1 << bit))
        })
        .unwrap_or_else(|| table.full_mask())
}
impl<V: Clone + PartialEq> RowFold<V> {
    pub fn read<R: RowCodec>(
        frame: &DecodedFrame,
        table: TableId,
        rows: &[R],
        mask: u64,
        previous: Option<&Self>,
        extract: impl FnMut(&R) -> io::Result<V>,
    ) -> io::Result<Self> {
        Self::read_input(HourInput::new(frame), table, rows, mask, previous, extract)
    }

    pub fn read_input<R: RowCodec>(
        input: HourInput<'_>,
        table: TableId,
        rows: &[R],
        mask: u64,
        previous: Option<&Self>,
        mut extract: impl FnMut(&R) -> io::Result<V>,
    ) -> io::Result<Self> {
        let view = &input.frame().tables[table.index()];
        let changes = input
            .changes()
            .since(table, previous.map(|old| &old.identity), mask);
        if let TableChanges::Unchanged = changes {
            let old = previous.expect("certified predecessor");
            return Ok(Self {
                identity: view.identity(),
                values: old.values.clone(),
                edits: Retained::new(Vec::new()),
            });
        }
        let facts = match changes {
            TableChanges::Rows(facts) => Some(facts),
            TableChanges::Discontinuity(_) => None,
            TableChanges::Unchanged => unreachable!(),
        };
        let count = facts
            .map_or_else(
                || {
                    rows.len()
                        .checked_add(previous.map_or(0, |old| old.values.len()))
                },
                |facts| facts.changed().len().checked_add(facts.deleted().len()),
            )
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let mut edits = Vec::with_capacity(count);
        let mut patches = Vec::with_capacity(count);
        let mut values = previous.map_or_else(ValueIndex::default, |old| old.values.clone());
        if let Some(facts) = facts {
            for &key in facts.deleted() {
                if values.get(&key).is_some() {
                    patches.push((key, None));
                    edits.push(key);
                }
            }
            for edit in facts
                .changed()
                .iter()
                .filter(|edit| edit.inserted || edit.fields & mask != 0)
            {
                let at = rows
                    .binary_search_by_key(&edit.key, RowCodec::key)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "projection change has no current row",
                        )
                    })?;
                let value = extract(&rows[at])?;
                if values.get(&edit.key) != Some(&value) {
                    patches.push((edit.key, Some(value)));
                    edits.push(edit.key);
                }
            }
            patches.sort_by_key(|edit| edit.0);
            patches.dedup_by(|a, b| {
                if a.0 != b.0 {
                    return false;
                }
                b.1 = a.1.take();
                true
            });
            values = values.patched(&patches)?;
        } else {
            values = ValueIndex::default();
            for row in rows {
                let key = row.key();
                patches.push((key, Some(extract(row)?)));
                edits.push(key);
            }
            patches.sort_by_key(|edit| edit.0);
            patches.dedup_by(|a, b| {
                if a.0 != b.0 {
                    return false;
                }
                b.1 = a.1.take();
                true
            });
            values = values.patched(&patches)?;
            if let Some(old) = previous {
                for key in old.values.keys() {
                    if values.get(key).is_none() {
                        edits.push(*key);
                    }
                }
            }
        }
        edits.sort_unstable();
        edits.dedup();
        let edits = Retained::new(edits);
        Ok(Self {
            identity: view.identity(),
            values,
            edits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_foundation::test_support::test_frame;
    use replay_data::rows::{CountryLinkRow, TableRows};
    use replay_frame::frame::{FrameDecoder, FrameEncoder, TableView};

    #[test]
    fn changed_key_index_extracts_one_of_1501_rows_then_only_the_deleted_key() {
        let mut source = test_frame(0);
        source.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=1501)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ))
            .into();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&[TableId::CountryLinks]).with_change_facts();
        let update = encoder.encode(source.clone(), true).unwrap();
        let frame = decoder.advance(&update).unwrap();
        let first = RowFold::read(
            frame,
            TableId::CountryLinks,
            frame
                .complete::<replay_data::rows::tables::CountryLinks>()
                .unwrap(),
            1,
            None,
            |row| Ok(row.canonical_slot),
        )
        .unwrap();
        source.hour += 1;
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *source.tables[TableId::CountryLinks.index()]
        {
            rows[1500].canonical_slot = 9001;
        }
        let update = encoder.encode(source.clone(), false).unwrap();
        let frame = decoder.advance(&update).unwrap();
        let mut extracted = 0;
        let second = RowFold::read_input(
            HourInput::new(frame),
            TableId::CountryLinks,
            frame
                .complete::<replay_data::rows::tables::CountryLinks>()
                .unwrap(),
            1,
            Some(&first),
            |row| {
                extracted += 1;
                Ok(row.canonical_slot)
            },
        )
        .unwrap();
        assert_eq!(extracted, 1);
        assert_eq!(&**second.edits, &[1501]);
        assert_eq!(second.values.get(&1501), Some(&9001));
        source.hour += 1;
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *source.tables[TableId::CountryLinks.index()]
        {
            rows.pop();
        }
        let update = encoder.encode(source, false).unwrap();
        let frame = decoder.advance(&update).unwrap();
        let third = RowFold::read_input(
            HourInput::new(frame),
            TableId::CountryLinks,
            frame
                .complete::<replay_data::rows::tables::CountryLinks>()
                .unwrap(),
            1,
            Some(&second),
            |_| panic!("deletion extracts no current row"),
        )
        .unwrap();
        assert_eq!(&**third.edits, &[1501]);
        assert_eq!(third.values.len(), 1500);
    }
}
