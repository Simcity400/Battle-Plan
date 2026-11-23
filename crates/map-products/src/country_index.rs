use replay_data::rows::{RowCodec, TableId, tables};
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use std::{collections::BTreeMap, io, sync::Mutex};
use viewer_foundation::tables::core;

#[derive(Default)]
pub(crate) struct CountryIndex(Mutex<Option<State>>);
struct State {
    source: ViewIdentity,
    branch: u64,
    table: ViewIdentity,
    by_native: BTreeMap<u32, u32>,
    #[cfg(test)]
    visits: usize,
}
impl CountryIndex {
    pub fn contains(&self, frame: &DecodedFrame, native: u32, slot: u32) -> io::Result<bool> {
        let rows = core::<tables::Countries>(frame)?;
        let view = &frame.tables[TableId::Countries.index()];
        let source = &frame.tables[TableId::Campaign.index()];
        let mut held = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(state) = held
            .as_mut()
            .filter(|state| state.source.is(source) && state.branch == frame.branch)
        {
            let fields = super::landscape::named_fields(TableId::Countries, &["canonical_slot"]);
            if state.table.fields_unchanged(view, fields) {
                state.table = view.identity();
                return Ok(state.by_native.get(&native) == Some(&slot));
            }
            if let Some(changes) = view.changes_for(&state.table, fields) {
                for key in changes.deleted() {
                    if let Ok(native) = u32::try_from(*key) {
                        state.by_native.remove(&native);
                        #[cfg(test)]
                        {
                            state.visits += 1;
                        }
                    }
                }
                for change in changes
                    .changed()
                    .iter()
                    .filter(|row| row.inserted || row.fields & fields != 0)
                {
                    let at = rows
                        .binary_search_by_key(&change.key, RowCodec::key)
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "country change fact has no current row",
                            )
                        })?;
                    let row = &rows[at];
                    state.by_native.insert(row.id, row.canonical_slot);
                    #[cfg(test)]
                    {
                        state.visits += 1;
                    }
                }
                state.table = view.identity();
                return Ok(state.by_native.get(&native) == Some(&slot));
            }
        }
        *held = None;
        let by_native: BTreeMap<_, _> = rows
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let present = by_native.get(&native) == Some(&slot);
        *held = Some(State {
            source: source.identity(),
            branch: frame.branch,
            table: view.identity(),
            by_native,
            #[cfg(test)]
            visits: rows.len(),
        });
        Ok(present)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{CountryRow, TableRows};
    use replay_frame::frame::{FrameDecoder, FrameEncoder, Metrics, TableView};
    fn frame(hour: i32, rows: Vec<CountryRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic fixture gap".into(),
                }
                .into()
            }),
        };
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(rows)).into();
        frame
    }
    #[test]
    fn patches_only_changed_country_keys_and_never_pins_rows() {
        let index = CountryIndex::default();
        let mut rows: Vec<_> = (1..=1501)
            .map(|id| CountryRow {
                id,
                canonical_slot: id + 5000,
                ..Default::default()
            })
            .collect();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&[TableId::Countries]).with_change_facts();
        let first = encoder.encode(frame(1, rows.clone()), true).unwrap();
        let first = decoder.apply(&first).unwrap();
        assert!(index.contains(&first, 1501, 6501).unwrap());
        let visits = index.0.lock().unwrap().as_ref().unwrap().visits;
        rows[1499].command_power_raw = 12;
        let next = encoder.encode(frame(2, rows.clone()), false).unwrap();
        let next = decoder.apply(&next).unwrap();
        assert!(index.contains(&next, 1501, 6501).unwrap());
        assert_eq!(index.0.lock().unwrap().as_ref().unwrap().visits, visits);
        rows[1500].canonical_slot = 9000;
        rows.remove(0);
        let next = encoder.encode(frame(3, rows), false).unwrap();
        let next = decoder.apply(&next).unwrap();
        assert!(!index.contains(&next, 1, 5001).unwrap());
        assert!(index.contains(&next, 1501, 9000).unwrap());
        assert_eq!(index.0.lock().unwrap().as_ref().unwrap().visits, visits + 2);
        drop(decoder);
        assert!(
            next.tables[TableId::Countries.index()]
                .clone()
                .try_unwrap()
                .is_err()
        );
        let unselected = replay_frame::frame::not_selected();
        for (table, view) in TableId::ALL.into_iter().zip(next.tables) {
            if table == TableId::Countries {
                assert!(view.try_unwrap().is_ok());
            } else {
                assert!(replay_frame::frame::SharedView::ptr_eq(&view, &unselected));
            }
        }
    }
}
