
use super::counter_content::Content;
use viewer_unit_reads::movement_reads::ReadOperands;
use viewer_foundation::movement_timing::{Etas, MovementContent, arrival};
use viewer_runtime::unit_handle::Unit;
mod fresh_search;
#[path = "division_eta/classes/naval.rs"]
mod naval;
#[path = "division_eta/classes/railway.rs"]
mod railway;
#[path = "division_eta/classes/task_force.rs"]
mod task_force;
#[cfg(test)]
pub(super) mod test_support;
use replay_frame::frame::DecodedFrame;
use std::{cell::OnceCell, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CounterUnit {
    Division(u64),
    TaskForce(u64),
    Railway(u64),
}

#[derive(Debug)]
pub(crate) struct CounterPreview {
    pub(crate) duration: Result<i64, String>,
    pub(crate) route: Vec<u32>,
}

impl CounterQueries<'_> {
    pub(crate) fn preview(
        &self,
        unit: CounterUnit,
        target: u32,
        local_option: bool,
    ) -> Result<CounterPreview, String> {
        match unit {
            CounterUnit::Division(handle) => {
                self.query(|inputs, moves| inputs.derive_to(moves, handle, target, local_option))
            }
            CounterUnit::TaskForce(_) => Err(task_force::ADMISSION_UNRECORDED.into()),
            CounterUnit::Railway(handle) => self.query(|inputs, moves| {
                railway::derive_to(inputs, moves, handle, target, local_option)
            }),
        }
    }
}

pub(crate) struct CounterQueries<'a> {
    content: &'a Content,
    reads: ReadOperands<'a>,
    frame: &'a DecodedFrame,
    world: OnceCell<dashboard_text::triggers::World<'a>>,
    inputs: OnceCell<fresh_search::Inputs<'a>>,
    etas: OnceCell<Result<Arc<Etas>, String>>,
    cells: viewer_unit_reads::movement_reads::Cells,
    common: viewer_unit_reads::movement_reads::CommonCells<'a>,
}

impl<'a> CounterQueries<'a> {
    pub(crate) fn new(
        content: &'a Content,
        reads: ReadOperands<'a>,
        frame: &'a DecodedFrame,
    ) -> Self {
        Self {
            content,
            reads,
            frame,
            world: OnceCell::new(),
            inputs: OnceCell::new(),
            etas: OnceCell::new(),
            cells: Default::default(),
            common: viewer_unit_reads::movement_reads::CommonCells::new(frame),
        }
    }

    pub(crate) fn selected_strait(
        &self,
        through: u32,
        raw_owner: u32,
    ) -> Result<content_reader_unit_path::unit_path::StraitFleet, String> {
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_unit_reads::movement_reads::Moves::for_common(
            world,
            self.reads,
            &self.cells,
            &self.common,
        )?;
        moves.selected_strait(through, raw_owner)
    }

    pub(crate) fn selected_move_message(
        &self,
        unit: CounterUnit,
        target: u32,
        text: &mut String,
    ) -> Result<bool, String> {
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_unit_reads::movement_reads::Moves::for_common(
            world,
            self.reads,
            &self.cells,
            &self.common,
        )?;
        let content = viewer_unit_reads::movement_reads::CommonText {
            frame: self.frame,
            loc: &self.content.localisation,
            recorded_language: &self.content.recorded_language,
            state_keys: &self.content.state_name_keys,
        };
        match unit {
            CounterUnit::Division(handle) => {
                moves.common_army_message(handle, target, text, &content)
            }
            CounterUnit::Railway(handle) => {
                moves.common_railway_message(handle, target, text, &content)
            }
            CounterUnit::TaskForce(_) => Err("selected land movement received a task force".into()),
        }
    }

    pub(crate) fn selected_transport_message(
        &self,
        selected: &[Unit],
        target: u32,
    ) -> Result<content_reader_unit_path::unit_path::selected_transport::Outputs, String> {
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_unit_reads::movement_reads::Moves::for_common(
            world,
            self.reads,
            &self.cells,
            &self.common,
        )?;
        let content = viewer_unit_reads::movement_reads::CommonText {
            frame: self.frame,
            loc: &self.content.localisation,
            recorded_language: &self.content.recorded_language,
            state_keys: &self.content.state_name_keys,
        };
        moves.selected_transport_message(selected, target, &content)
    }

    pub(super) fn division_remaining(&self, division: u64, ours: bool) -> Result<i64, String> {
        let etas = self.etas.get_or_init(|| {
            self.content
                .eta_cache
                .etas(self.frame, self.content.movement.as_deref())
                .map_err(|error| error.to_string())
        });
        arrival(etas.as_deref().map_err(Clone::clone)?, division, ours)
    }

    pub(super) fn railway_remaining(
        &self,
        gun: &replay_data::rows::RailwayGunCounterInputRow,
        ours: bool,
        local_option: bool,
    ) -> Result<i64, String> {
        self.query(|inputs, moves| railway::derive(inputs, moves, gun, ours, local_option))
    }

    pub(crate) fn task_force_remaining(
        &self,
        motion: &replay_data::rows::TaskForceMapMotionRow,
        ours: bool,
    ) -> Result<i64, String> {
        let content = self
            .content
            .movement
            .as_deref()
            .ok_or("task force arrival movement content is unavailable")?;
        let inputs = self
            .inputs
            .get_or_init(|| fresh_search::Inputs::new(self.frame, content));
        task_force::derive(inputs, motion, ours, |from| {
            self.reads.selected_region(from)
        })
    }

    fn query<T>(
        &self,
        derive: impl FnOnce(
            &fresh_search::Inputs<'a>,
            &viewer_unit_reads::movement_reads::Moves<'_, 'a>,
        ) -> Result<T, String>,
    ) -> Result<T, String> {
        let content = self
            .content
            .movement
            .as_deref()
            .ok_or("fresh counter ETA movement content is unavailable")?;
        let inputs = self
            .inputs
            .get_or_init(|| fresh_search::Inputs::new(self.frame, content));
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_unit_reads::movement_reads::Moves::for_counter(
            world,
            self.reads,
            &self.cells,
            inputs,
        );
        derive(inputs, &moves)
    }
}
