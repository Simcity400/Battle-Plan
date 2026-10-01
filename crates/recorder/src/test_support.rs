use replay::{
    frame::{DecodedFrame, Metrics, TableView},
    rows::{TableId, TableRows},
};

pub(crate) fn frame(rows: impl IntoIterator<Item = TableRows>) -> DecodedFrame {
    let mut tables =
        std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
    for rows in rows {
        let index = rows.table().index();
        tables[index] = TableView::Complete(rows);
    }
    DecodedFrame {
        branch: 1,
        hour: 7,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}
