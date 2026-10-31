//! Opt-in accounting of the state serialized by the hourly walk.
use super::{BranchState, independent};
use dashboard_base::prepared::state_census::Line;

/// The census line of the branch state the walk saves at `hour`.
pub(super) fn walk_state(branch: u64, hour: i32, state: &BranchState) -> Line {
    let mut line = Line::new("walk-state")
        .field("branch", branch)
        .field("hour", hour);
    line = state.land.state_census(line);
    line = line
        .json("focus", &state.focus)
        .json("index", &state.index)
        .json("measure", &state.measure)
        .json("land", &state.land)
        .json("resources", &state.resources)
        .json("group_changes", &state.group_changes)
        .json("game_score", &state.game_score)
        .json("identities_held", &state.identities);
    // The parts it saves only while their family is in the walk.
    macro_rules! walked {
        ($($field:ident => $family:literal),* $(,)?) => {
            $(
                if independent::hourly($family) {
                    line = line.json(stringify!($field), &state.$field);
                }
            )*
        };
    }
    walked!(
        production => "production",
        construction => "construction",
        subject_changes => "subjects",
        subject_late => "subjects",
        idea_changes => "ideas",
        experience => "experience",
        peace => "peace",
        casualties => "casualties",
        research => "research",
        projects => "projects",
        nukes => "nukes",
        factories => "factories",
        repairs => "repairs",
        feed => "event_feed",
        naval => "naval",
        divisions => "divisions",
        air => "air",
        navy => "navy",
        parts => "parts",
        equipment_losses => "equipment_losses",
        trade => "trade",
        territory => "territory",
    );
    line
}
