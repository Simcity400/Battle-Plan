//! Synthetic frames and publication for the economy families' tests.
use dashboard_base::prepared::series::tests::Fixture;
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, Metrics, TableView};

pub fn frame(hour: i32) -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            let mut rows = if table.key_bits() == 0 {
                TableRows::one_default(table)
            } else {
                TableRows::empty(table)
            };
            // A singleton whose default row is no valid read was not read by these frames.
            if rows.validate_keys().is_err() {
                return TableView::Unavailable {
                    reason: "synthetic economy frames did not read this table".into(),
                }
                .into();
            }
            if let TableRows::Clock(rows) = &mut rows {
                rows[0].hour = hour;
            }
            TableView::Complete(rows).into()
        }),
        metrics: Metrics::default(),
    }
}

pub fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

pub fn unavailable(frame: &mut DecodedFrame, id: TableId) {
    *frame.tables[id.index()] = TableView::Unavailable {
        reason: "synthetic writer refusal".into(),
    };
}

/// Countries `(raw id, slot, tag)`, each linked from its own id.
pub fn countries(frame: &mut DecodedFrame, rows: &[(u32, u32, &str)]) {
    set(
        frame,
        TableRows::Countries(
            rows.iter()
                .map(|&(id, slot, tag)| CountryRow {
                    id,
                    canonical_slot: slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut links: Vec<_> = rows
        .iter()
        .map(|&(id, canonical_slot, _)| CountryLinkRow { id, canonical_slot })
        .collect();
    links.sort_by_key(|link| link.id);
    set(frame, TableRows::CountryLinks(links));
}

/// Many countries `T0000`… with raw ids one above their slots.
pub fn many_countries(frame: &mut DecodedFrame, count: u32) {
    let tags: Vec<String> = (0..count).map(|slot| format!("T{slot:04}")).collect();
    let rows: Vec<(u32, u32, &str)> = tags
        .iter()
        .enumerate()
        .map(|(slot, tag)| (slot as u32 + 1, slot as u32, tag.as_str()))
        .collect();
    countries(frame, &rows);
}

/// A store a test publishes a family into.
pub fn fixture() -> Fixture {
    Fixture::new()
}
