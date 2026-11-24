use replay_columnar::manifest::Status;
use replay_data::rows::{RowCodec, TableId, TableRows, TableSchema};
use replay_frame::frame::{DecodedFrame, Keys, TableView};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CoverageScope {
    #[default]
    Table,
    Row,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    Recorded,
    NotRecorded(String),
}

impl Coverage {
    pub fn into_missing(self) -> Option<String> {
        match self {
            Self::Recorded => None,
            Self::NotRecorded(reason) => Some(reason),
        }
    }
}

pub fn table_title(table: TableId) -> String {
    let words = table.words();
    let mut chars = words.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

pub fn row_coverage(table: TableId, status: Status, unavailable_reason: Option<&str>, witnessed: bool) -> Coverage {
    match status {
        Status::Complete => Coverage::Recorded,
        Status::Truncated { .. } if witnessed => Coverage::Recorded,
        Status::Truncated { missing } => Coverage::NotRecorded(format!("{} read only in part at this hour ({missing} rows not read)", table_title(table))),
        Status::NotRecorded => Coverage::NotRecorded(format!("{} not recorded at this hour", table_title(table))),
        Status::Unavailable { .. } => Coverage::NotRecorded(match unavailable_reason.filter(|reason| !reason.is_empty()) {
            Some(reason) => format!("{} not available at this hour: {reason}", table_title(table)),
            None => format!("{} not available at this hour", table_title(table)),
        }),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Witness<'a, R> {
    Row(&'a R),
    Absent,
    NotRecorded(String),
}

impl<'a, R> Witness<'a, R> {
    pub fn row(&self) -> Option<&'a R> {
        match self {
            Self::Row(row) => Some(row),
            Self::Absent | Self::NotRecorded(_) => None,
        }
    }
}

fn typed<T: TableSchema>(rows: &TableRows) -> Result<&[T::Row], String> {
    rows.rows::<T>().ok_or_else(|| format!("{} not available at this hour: it holds rows of another kind", table_title(T::ID)))
}

fn refusal(table: TableId, view: &TableView) -> String {
    let status = match view {
        TableView::Complete(_) => Status::Complete,
        TableView::Truncated { missing, .. } => Status::Truncated { missing: *missing },
        TableView::Unavailable { .. } => Status::Unavailable { reason: 0 },
        TableView::NotRecorded { .. } => Status::NotRecorded,
    };
    let reason = match view {
        TableView::Unavailable { reason } => Some(reason.as_str()),
        _ => None,
    };
    row_coverage(table, status, reason, false).into_missing().unwrap_or_default()
}

fn unread(table: TableId) -> String {
    format!("{} not read for this row at this hour", table_title(table))
}

pub fn witnessed<T: TableSchema>(frame: &DecodedFrame, key: u128) -> Witness<'_, T::Row> {
    let shared = &frame.tables[T::ID.index()];
    let view: &TableView = shared;
    let (rows, complete) = match view {
        TableView::Complete(rows) => (rows, true),
        TableView::Truncated { rows, .. } => (rows, false),
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => return Witness::NotRecorded(refusal(T::ID, view)),
    };
    let rows = match typed::<T>(rows) {
        Ok(rows) => rows,
        Err(reason) => return Witness::NotRecorded(reason),
    };
    match rows.binary_search_by_key(&key, RowCodec::key) {
        Ok(at) => Witness::Row(&rows[at]),
        Err(_) if !shared.keys().contains(key) => Witness::NotRecorded(unread(T::ID)),
        Err(_) if complete => Witness::Absent,
        Err(_) => Witness::NotRecorded(refusal(T::ID, view)),
    }
}

pub struct WitnessedRows<'a, R> {
    pub rows: &'a [R],
    pub missing: Option<String>,
}

pub fn witnessed_rows<T: TableSchema>(frame: &DecodedFrame) -> WitnessedRows<'_, T::Row> {
    let shared = &frame.tables[T::ID.index()];
    let view: &TableView = shared;
    let (rows, complete) = match view {
        TableView::Complete(rows) => (rows, shared.keys() == &Keys::All),
        TableView::Truncated { rows, .. } => (rows, false),
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => return WitnessedRows { rows: &[], missing: Some(refusal(T::ID, view)) },
    };
    match typed::<T>(rows) {
        Ok(rows) if complete => WitnessedRows { rows, missing: None },
        Ok(rows) if matches!(view, TableView::Complete(_)) => WitnessedRows { rows, missing: Some(format!("{} read only for chosen rows at this hour", table_title(T::ID))) },
        Ok(rows) => WitnessedRows { rows, missing: Some(refusal(T::ID, view)) },
        Err(reason) => WitnessedRows { rows: &[], missing: Some(reason) },
    }
}

pub fn population<'f, T: TableSchema>(frame: &'f DecodedFrame, asked: &Keys) -> Result<&'f [T::Row], String> {
    let shared = &frame.tables[T::ID.index()];
    let view: &TableView = shared;
    match view {
        TableView::Complete(_) if shared.keys() != asked => Err(format!("{} read only for chosen rows at this hour", table_title(T::ID))),
        TableView::Complete(rows) => typed::<T>(rows),
        _ => Err(refusal(T::ID, view)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_coverage_applies_witness_rule_and_canonical_wording() {
        assert_eq!(row_coverage(TableId::CountryExperience, Status::Complete, None, true), Coverage::Recorded);
        assert_eq!(row_coverage(TableId::CountryExperience, Status::Complete, None, false), Coverage::Recorded);
        assert_eq!(row_coverage(TableId::CountryExperience, Status::Truncated { missing: 3 }, None, true), Coverage::Recorded);
        assert_eq!(
            row_coverage(TableId::CountryExperience, Status::Truncated { missing: 3 }, None, false),
            Coverage::NotRecorded("Country experience read only in part at this hour (3 rows not read)".into()),
        );
        assert_eq!(
            row_coverage(TableId::CountryExperience, Status::NotRecorded, None, false),
            Coverage::NotRecorded("Country experience not recorded at this hour".into()),
        );
        assert_eq!(
            row_coverage(TableId::CountryExperience, Status::Unavailable { reason: 1 }, Some("reader fault"), false),
            Coverage::NotRecorded("Country experience not available at this hour: reader fault".into()),
        );
        assert_eq!(
            row_coverage(TableId::CountryExperience, Status::Unavailable { reason: 1 }, Some(""), false),
            Coverage::NotRecorded("Country experience not available at this hour".into()),
        );
        assert_eq!(
            row_coverage(TableId::CountryExperience, Status::Unavailable { reason: 1 }, None, true),
            Coverage::NotRecorded("Country experience not available at this hour".into()),
        );
    }

    #[test]
    fn a_witnessed_key_keeps_its_row_from_a_truncated_table_and_a_population_needs_the_whole_read() {
        use replay_data::rows::{CountryRow, tables::Countries};
        let row = |id: u32| CountryRow { id, tag: b"GER".to_vec(), ..CountryRow::default() };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: std::array::from_fn(|_| TableView::Unavailable { reason: "outside the test".into() }.into()),
            metrics: replay_frame::frame::Metrics { total_ns: 0, table_ns: [0; replay_data::rows::TABLE_COUNT], pool_bytes: 0, dropped_frames: 0 },
        };
        let table = TableId::Countries.index();
        let key = |id: u32| RowCodec::key(&row(id));
        *frame.tables[table] = TableView::Truncated { rows: TableRows::Countries(vec![row(7)]), missing: 2 };
        assert_eq!(witnessed::<Countries>(&frame, key(7)).row().map(|row| row.id), Some(7), "a witnessed row is its own value");
        assert_eq!(witnessed::<Countries>(&frame, key(9)), Witness::NotRecorded("Countries read only in part at this hour (2 rows not read)".into()));
        assert_eq!(population::<Countries>(&frame, &Keys::All), Err("Countries read only in part at this hour (2 rows not read)".into()));
        let seen = |frame: &DecodedFrame| { let got = witnessed_rows::<Countries>(frame); (got.rows.iter().map(|row| row.id).collect::<Vec<_>>(), got.missing) };
        assert_eq!(seen(&frame), (vec![7], Some("Countries read only in part at this hour (2 rows not read)".into())), "witnessed rows are kept, with why the list is not whole");
        *frame.tables[table] = TableView::Complete(TableRows::Countries(vec![row(7)]));
        assert_eq!(witnessed::<Countries>(&frame, key(9)), Witness::Absent, "a complete table proves the key absent");
        assert_eq!(population::<Countries>(&frame, &Keys::All).map(<[_]>::len), Ok(1));
        assert_eq!(seen(&frame), (vec![7], None), "a whole read is the whole list");
        let chosen = Keys::These(vec![key(7), key(8)]);
        frame.tables[table] = frame.tables[table].clone().with_keys(Some(std::sync::Arc::new(chosen.clone())));
        assert_eq!(witnessed::<Countries>(&frame, key(7)).row().map(|row| row.id), Some(7));
        assert_eq!(witnessed::<Countries>(&frame, key(8)), Witness::Absent, "a complete read that chose the key proves it absent");
        assert_eq!(witnessed::<Countries>(&frame, key(9)), Witness::NotRecorded("Countries not read for this row at this hour".into()), "a key the read did not choose was never looked for");
        assert_eq!(population::<Countries>(&frame, &Keys::All), Err("Countries read only for chosen rows at this hour".into()), "chosen rows are not the whole table");
        assert_eq!(population::<Countries>(&frame, &chosen).map(<[_]>::len), Ok(1), "the chosen rows are the population of what was chosen");
        assert_eq!(seen(&frame), (vec![7], Some("Countries read only for chosen rows at this hour".into())));
        *frame.tables[table] = TableView::Unavailable { reason: "reader fault".into() };
        assert_eq!(witnessed::<Countries>(&frame, key(7)), Witness::NotRecorded("Countries not available at this hour: reader fault".into()));
        assert_eq!(seen(&frame), (Vec::new(), Some("Countries not available at this hour: reader fault".into())), "an unread list is empty and says why, never a known zero");
    }
}
