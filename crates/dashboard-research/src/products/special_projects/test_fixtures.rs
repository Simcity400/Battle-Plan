use super::facts::COMPLETION_MARKER;
use engine_projects::nproject_cproject_state_machine::SIMPLE_STATE_1;
use replay_data::rows::{ProjectHistoryRow, ProjectRow, SliceReadRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};

pub fn completion_frame(
    hour: i32,
    completed: impl IntoIterator<Item = (u32, u32, i32)>,
) -> DecodedFrame {
    let owner = 9001;
    let mut frame = dashboard_base::test_fixtures::source(1, hour, None);
    let (projects, histories) = completed
        .into_iter()
        .map(|(project, definition_id, date)| {
            let mut row = ProjectRow {
                owner,
                local_id: project,
                definition_id,
                ..Default::default()
            };
            row.state.current_offset = SIMPLE_STATE_1 as u32;
            let marker = ProjectHistoryRow {
                owner,
                project,
                date,
                prototype_reward_handle: COMPLETION_MARKER,
                option_handle: COMPLETION_MARKER,
                ..Default::default()
            };
            (row, marker)
        })
        .unzip();
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(projects));
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(histories));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
        [TableId::Projects, TableId::ProjectHistory]
            .into_iter()
            .map(|table| SliceReadRow {
                table: table.index() as u32,
                owner,
                sample_hour: hour,
            })
            .collect(),
    ));
    frame
}
