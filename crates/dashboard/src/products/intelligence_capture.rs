//! Pure native identity projections shared by agency and operation consumers.
use super::recorded_rows;
use replay::{
    frame::DecodedFrame,
    rows::{AgencyUpgradeLevelRow, tables},
};

pub(super) struct CapturedAgencyUpgrade {
    /// Native array ordinal at this read, not a persistent upgrade identity.
    pub ordinal: u32,
    pub definition_pointer: String,
    pub branch_pointer: String,
    pub upgrade_token: Option<u32>,
    pub branch_token: Option<u32>,
    pub upgrade_literal: Option<String>,
    pub branch_literal: Option<String>,
    /// Exact signed level; duplicates and non-positive entries remain intact.
    pub level: i32,
    pub identity_status: u8,
    pub identity_reason: Option<String>,
}

pub(super) fn literal(frame: &DecodedFrame, token: Option<u32>) -> Option<String> {
    let token = token?;
    let lexical = recorded_rows::<tables::LexicalTokens>(frame).ok()?;
    let index = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&lexical[index].literal)
        .ok()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub(super) fn upgrade(frame: &DecodedFrame, row: &AgencyUpgradeLevelRow) -> CapturedAgencyUpgrade {
    let identity_reason = (row.identity_status != engine::agency_upgrade_levels::AUTHENTICATED)
        .then(|| engine::agency_upgrade_levels::status_name(row.identity_status).to_owned());
    CapturedAgencyUpgrade {
        ordinal: row.ordinal,
        definition_pointer: row.definition_pointer.to_string(),
        branch_pointer: row.branch_pointer.to_string(),
        upgrade_token: row.upgrade_token,
        branch_token: row.branch_token,
        upgrade_literal: literal(frame, row.upgrade_token),
        branch_literal: literal(frame, row.branch_token),
        level: row.level,
        identity_status: row.identity_status,
        identity_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::TableView,
        rows::{LexicalTokenRow, TableId, TableRows},
    };

    #[test]
    fn acquired_identity_keeps_signed_raw_operands_and_names_its_refusal() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(1));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 7,
                    literal: b"mod_upgrade".to_vec(),
                },
                LexicalTokenRow {
                    token: 8,
                    literal: b"mod_branch".to_vec(),
                },
            ]));
        let row = AgencyUpgradeLevelRow {
            country: 5000,
            ordinal: 2,
            definition_pointer: u64::MAX,
            branch_pointer: 42,
            upgrade_token: Some(7),
            branch_token: Some(8),
            level: -3,
            identity_status: engine::agency_upgrade_levels::DEFINITION_NOT_REGISTERED,
        };
        let value = upgrade(&frame, &row);
        assert_eq!((value.ordinal, value.level), (2, -3));
        assert_eq!(value.definition_pointer, u64::MAX.to_string());
        assert_eq!(value.branch_pointer, "42");
        assert_eq!(value.upgrade_literal.as_deref(), Some("mod_upgrade"));
        assert_eq!(value.branch_literal.as_deref(), Some("mod_branch"));
        assert_eq!(
            value.identity_reason.as_deref(),
            Some(engine::agency_upgrade_levels::status_name(
                row.identity_status
            ))
        );
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "dictionary missing".into(),
        };
        let value = upgrade(&frame, &row);
        assert_eq!(value.upgrade_token, Some(7));
        assert_eq!(value.branch_token, Some(8));
        assert!(value.upgrade_literal.is_none());
        assert!(value.branch_literal.is_none());
        assert_eq!(value.level, -3);
    }
}
