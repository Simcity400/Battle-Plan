
use replay_data::rows::{self, TableId};
use replay_frame::frame::DecodedFrame;

pub fn recorded(hour: i32) -> DecodedFrame {
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::LocalPlayerRow,
        rows::PoliticsRow, rows::TableRows, rows::WarNameRuleRow, rows::WarOverviewCountryRow,
        rows::WarOwnedRelationRow, rows::WarRelationRow,
    };
    use replay_frame::frame::TableView;
    let mut frame = DecodedFrame { branch: 1, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: replay_frame::frame::Metrics::default() };
    for table in [
        TableId::Factions,
        TableId::FactionMembers,
        TableId::WarRelationGoals,
        TableId::Capitulation,
        TableId::LexicalTokens,
        TableId::PoliticalParties,
        TableId::Autonomy,
        TableId::States,
    ] {
        frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
    }
    frame.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![rows::ClockRow {
            hour,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![rows::CampaignRow {
            id: b"analytics-test".to_vec(),
        }]))
        .into();
    let tags = ["ONE", "TWO", "THREE"];
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        tags.iter()
            .enumerate()
            .map(|(i, tag)| CountryRow {
                id: i as u32 + 1,
                canonical_slot: i as u32 + 1,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=3)
            .map(|i| CountryLinkRow {
                id: i,
                canonical_slot: i,
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            (1..=3)
                .map(|country| CountryAppearanceRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        (1..=3)
            .map(|country| PoliticsRow {
                country,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 1,
            observer_country: 0,
        }]))
        .into();
    let relations: Vec<_> = [2, 3]
        .into_iter()
        .map(|second_country| WarRelationRow {
            first_country: 1,
            second_country,
            start_hour: 90,
            first_was_instigator: 1,
            first_casualties: i64::MAX,
            second_casualties: 7,
            first_unknown_casualties: -1,
            second_unknown_casualties: 3,
            ..Default::default()
        })
        .collect();
    frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(relations)).into();
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(vec![
            WarOwnedRelationRow {
                owner: 1,
                ordinal: 0,
                first_country: 1,
                second_country: 2,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 1,
                ordinal: 1,
                first_country: 1,
                second_country: 3,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 2,
                ordinal: 0,
                first_country: 1,
                second_country: 2,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 3,
                ordinal: 0,
                first_country: 1,
                second_country: 3,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(
            (1..=3)
                .map(|country| WarOverviewCountryRow {
                    score_available: 1,
                    country,
                    major: 1,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(vec![WarNameRuleRow {
            viewpoint: 1,
            index: 0,
            prefix: b"NATIVE".to_vec(),
        }]))
        .into();
    frame
}
