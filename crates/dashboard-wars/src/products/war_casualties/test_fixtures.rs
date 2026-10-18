//! War casualty frames and sources for tests here and in the crates above.

use super::CasualtyParams;
use dashboard_base::source::Source;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::{Arc, atomic::AtomicBool};

/// One chosen country whose tag has the characters a tag may carry.
pub fn chosen() -> CasualtyParams {
    CasualtyParams {
        branch_id: "1".into(),
        tags: vec!["mod country 1| with spaces".into()],
        top: None,
    }
}

/// A source over the replay, opened as a lease.
pub fn opened(replay: Replay) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "casualties-test".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// A campaign replay of one branch over the frames.
pub fn research_replay(frames: Vec<DecodedFrame>) -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

/// Three countries with an alias link each, at war between the first two when losses are given.
pub fn casualty_frame(hour: i32, losses: Option<(i64, i64)>, end: i32) -> DecodedFrame {
    use replay_data::rows::WarRelationRow;
    let mut frame = source(1, hour, None);
    let template = frame
        .complete::<replay_data::rows::tables::Countries>()
        .unwrap()[0]
        .clone();
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=3)
            .map(|slot| CountryRow {
                id: slot * 100,
                canonical_slot: slot,
                tag: format!("mod country {slot}| with spaces").into_bytes(),
                ..template.clone()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=3)
            .flat_map(|slot| {
                [
                    CountryLinkRow {
                        id: slot,
                        canonical_slot: slot,
                    },
                    CountryLinkRow {
                        id: slot * 100,
                        canonical_slot: slot,
                    },
                ]
            })
            .collect::<Vec<_>>(),
    ));
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        links.sort_by_key(|r| r.id);
    }
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        losses
            .map(|(known, unknown)| WarRelationRow {
                first_country: 1,
                second_country: 2,
                start_hour: 90,
                end_hour: end,
                first_casualties: known,
                second_casualties: known * 2,
                first_unknown_casualties: unknown,
                ..Default::default()
            })
            .into_iter()
            .collect(),
    ));
    frame
}
