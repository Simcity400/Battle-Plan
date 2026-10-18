//! Selected route queries and counter projections over shared movement timing.
//!
//! Each moving division's first-edge progress follows route integration over its
//! recorded `DivisionMovement` route ([`replay_derive::eta_derive::division_progress`]). Its
//! arrival times are estimates ([`replay_derive::eta_derive::estimate`]): the native walk
//! over the recorded route from `Divisions.province`, each edge at the native
//! land speed of the recorded stat-block base speed (`DivisionMovement.base_speed`)
//! with the recorded speed terms ([`viewer_foundation::movement_timing::HourTerms`]):
//! the country and commander block modifiers and armour speed, location weather,
//! supply (at the template's special-forces share), enemy air and retreat terms, each
//! edge's terrain, fort, snow, commander terrain, river and infrastructure terms, and
//! the organisation and fuel factors, floored at the native 1 km/h and scaled by the
//! content's movement scale; a strategic redeployment no modifier blocks at the road
//! or rail speed. The game also applies the location state's speed modifiers, the
//! priority sub-unit's category terms, the noncore-supply term and a relation's river
//! crossing factor, which the recording does not hold, so an estimate drifts from the
//! game wherever those apply.
//! Edge distances, terrain flags and the movement defines come from the
//! matching content stack ([`MovementContent`]), never the recording.
//!
//! The progress is the game's movement-arrow fill target, and The game draws that arrow only for a
//! unit of the player's own country that is selected (the unit's selected byte; for a navy also its
//! task force's selection word). The viewer's selection drives the movement-arrow layer. Counter
//! and figure positions use the settled route point in [`viewer_foundation::map::counter_move`].
//!
//! Counter hovers ([`CounterQueries`]) word a division's estimate, a railway
//! gun's fresh native route query and a task force's stored route. A row the
//! hour cannot measure has no estimate or no progress rather than a
//! substituted zero, and keeps its reason.

use super::counter_content::Content;
use viewer_foundation::map::movement_reads::ReadOperands;
use viewer_foundation::movement_timing::{Etas, MovementContent, arrival};
use viewer_foundation::unit_handle::Unit;
mod fresh_search;
#[path = "division_eta/classes/naval.rs"]
mod naval;
#[path = "division_eta/classes/railway.rs"]
mod railway;
#[path = "division_eta/classes/task_force.rs"]
mod task_force;
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
    // Independent signed fixed5 result; caller owns ceiling/i32 narrowing/visibility.
    pub(crate) duration: Result<i64, String>,
    // Actual fresh search output, source excluded; no actual-route substitute.
    pub(crate) route: Vec<u32>,
}

impl CounterQueries<'_> {
    /// A prospective route to `target`: the native route search
    /// for divisions and railway guns. A task force's search needs range,
    /// theatre and nearest-base admission inputs the recording does not hold,
    /// so its preview refuses by name.
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

/// Reached counter queries in one hover or diagnostics batch. Whole-frame
/// indexes are built lazily once; every requested route still runs native A*.
pub(crate) struct CounterQueries<'a> {
    content: &'a Content,
    reads: ReadOperands<'a>,
    frame: &'a DecodedFrame,
    world: OnceCell<dashboard_text::triggers::World<'a>>,
    inputs: OnceCell<fresh_search::Inputs<'a>>,
    etas: OnceCell<Result<Arc<Etas>, String>>,
    cells: viewer_foundation::map::movement_reads::Cells,
    common: viewer_foundation::map::movement_reads::CommonCells<'a>,
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
            common: viewer_foundation::map::movement_reads::CommonCells::new(frame),
        }
    }

    /// Share this hover's existing World/Cells; no new graph or prospective query.
    pub(crate) fn selected_strait(
        &self,
        through: u32,
        raw_owner: u32,
    ) -> Result<installed_content_unit_path::unit_path::StraitFleet, String> {
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_foundation::map::movement_reads::Moves::for_common(
            world,
            self.reads,
            &self.cells,
            &self.common,
        )?;
        moves.selected_strait(through, raw_owner)
    }

    /// The same hover's borrowed World, directed owners and text services.
    pub(crate) fn selected_move_message(
        &self,
        unit: CounterUnit,
        target: u32,
        text: &mut String,
    ) -> Result<bool, String> {
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_foundation::map::movement_reads::Moves::for_common(
            world,
            self.reads,
            &self.cells,
            &self.common,
        )?;
        let content = viewer_foundation::map::movement_reads::CommonText {
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
    ) -> Result<installed_content_unit_path::unit_path::selected_transport::Outputs, String> {
        let world = self.world.get_or_init(|| {
            dashboard_text::triggers::World::read(self.frame, self.reads.trigger_content.facts())
        });
        let moves = viewer_foundation::map::movement_reads::Moves::for_common(
            world,
            self.reads,
            &self.cells,
            &self.common,
        )?;
        let content = viewer_foundation::map::movement_reads::CommonText {
            frame: self.frame,
            loc: &self.content.localisation,
            recorded_language: &self.content.recorded_language,
            state_keys: &self.content.state_name_keys,
        };
        moves.selected_transport_message(selected, target, &content)
    }

    /// A division's arrival duration from this hour's shared estimates
    /// (the shared hourly ETA cache); see [`arrival`].
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

    /// A task force's arrival over its stored ordinary route
    /// (`TaskForceMapMotion.path`), searching no route.
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
            &viewer_foundation::map::movement_reads::Moves<'_, 'a>,
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
        let moves = viewer_foundation::map::movement_reads::Moves::for_counter(
            world,
            self.reads,
            &self.cells,
            inputs,
        );
        derive(inputs, &moves)
    }
}
