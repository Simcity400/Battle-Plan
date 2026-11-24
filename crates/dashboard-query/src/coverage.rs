use replay_columnar::manifest::Status;
use replay_data::rows::TableId;

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
}
