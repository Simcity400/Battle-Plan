//! Recorded research frames for tests here and in the crates above.

use crate::test_fixtures::{fixture_research_inputs, source};
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};

/// One recorded hour with the given research rows.
pub fn research_frame(
    branch: u64,
    hour: i32,
    assignments: &[Option<u32>],
    completed: &[u32],
) -> DecodedFrame {
    use replay_data::{
        rows::ResearchSlotRow, rows::ResearchTechnologyRow, rows::TechnologyDefinitionRow,
    };
    let mut frame = source(branch, hour, Some(1));
    *frame.tables[TableId::ResearchInputs.index()] = TableView::Unavailable {
        reason: "fixture hourly research".into(),
    };
    *frame.tables[TableId::ResearchSlots.index()] = TableView::Complete(TableRows::ResearchSlots(
        assignments
            .iter()
            .enumerate()
            .map(|(index, handle)| ResearchSlotRow {
                database_index: replay_data::rows::Recorded::Value((handle.unwrap_or(0)) as i32),
                country: 11,
                index: index as u32,
                technology_handle: handle.unwrap_or(0),
                technology_present: u8::from(handle.is_some()),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(
            (1..=4)
                .map(|handle| {
                    let done = completed.contains(&handle);
                    ResearchTechnologyRow {
                        database_index: (handle) as i32,
                        country: 11,
                        technology_handle: handle,
                        level: i32::from(done),
                        researched: u8::from(done),
                        classifier: if done {
                            engine_research::ctechnology::CLASSIFIER_COMPLETED
                        } else {
                            engine_research::ctechnology::CLASSIFIER_ACTIVE
                        },
                        ..Default::default()
                    }
                })
                .collect(),
        ));
    *frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(
            (1..=4)
                .map(|technology_handle| TechnologyDefinitionRow {
                    database_index: (technology_handle) as i32,
                    technology_handle,
                    id: format!("tech_{technology_handle}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// One recorded hour of a sliced research recording: the rows read at
/// `read` (with that read's assignments) carried into hour `hour`.
pub fn sliced_research_frame(hour: i32, read: i32, assignments: &[Option<u32>]) -> DecodedFrame {
    let mut frame = research_frame(1, read, assignments, &[]);
    fixture_research_inputs(&mut frame);
    let mut carried = research_frame(1, hour, assignments, &[]);
    carried.tables[TableId::ResearchInputs.index()] =
        frame.tables[TableId::ResearchInputs.index()].clone();
    carried.tables[TableId::ResearchAppliedBonuses.index()] =
        frame.tables[TableId::ResearchAppliedBonuses.index()].clone();
    carried.tables[TableId::SliceReads.index()] = frame.tables[TableId::SliceReads.index()].clone();
    carried
}
