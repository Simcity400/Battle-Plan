//! The army's synthetic frame, shared by its tests and the pipeline's.
use super::*;
use dashboard_base::test_fixtures::source;
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{
        CountryLinkRow, CountryRow, DivisionCensusRow, DivisionTemplateIconRow, LexicalTokenRow,
        TableRows, TemplateSubunit,
    },
};

pub fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 10,
            canonical_slot: 1,
            tag: b"origin".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 20,
            canonical_slot: 2,
            tag: b"host".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 30,
            canonical_slot: 3,
            tag: b"empty".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 10,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 20,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 30,
                canonical_slot: 3,
            },
        ]));
    *frame.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![
            DivisionCensusRow {
                country: 10,
                grouped: 0,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: 11,
                grouped: 1,
                ungrouped: 2,
            },
            DivisionCensusRow {
                country: 20,
                grouped: 0,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: 30,
                grouped: 0,
                ungrouped: 0,
            },
        ]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: 1,
            roster_owner: 20,
            logical_owner: 20,
            expeditionary_owner: 11,
            template: 5,
            // Whole display steps (replay::display), as recorded.
            hit_points: 1_230_000,
            ..Default::default()
        },
        DivisionRow {
            id: 2,
            roster_owner: 10,
            logical_owner: 10,
            expeditionary_owner: u32::MAX,
            template: 5,
            hit_points: 2_000_000,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 5,
                priority_unit_name: b"test_unit".to_vec(),
                subunits: vec![
                    TemplateSubunit {
                        token: 10,
                        count: 3,
                        local: 0,
                    },
                    TemplateSubunit {
                        token: 20,
                        count: 1,
                        local: 1,
                    },
                    TemplateSubunit {
                        token: 30,
                        count: 2,
                        local: 0,
                    },
                ],
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"foot".to_vec(),
            },
            LexicalTokenRow {
                token: 20,
                literal: b"support".to_vec(),
            },
            LexicalTokenRow {
                token: 30,
                literal: b"orphan".to_vec(),
            },
        ]));
    frame
}
