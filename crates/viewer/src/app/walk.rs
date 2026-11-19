//! The walking camera mode in the window: the figure the viewing country's infantry takes at the
//! hour on screen, the walker that moves it with the scale the user chose, and a window dump's
//! placed walk (`--walk`).

use crate::controls::Frame;
use crate::controls::walker::{FALLBACK_FIGURE, Start, Walker, World};
use viewer_foundation::map::camera::{Camera, Placement};
use viewer_foundation::map::perspective::Choice;
use viewer_world::map::models::ModelLibrary;
use viewer_world::map::scene::PlayerModel;
use viewer_world::map::unit_models::Figure;

/// The figure is chosen again when the branch, the hour or the viewing country changes.
pub(super) type FigureKey = (replay_data::BranchId, i32, Choice);

/// The pass label whose entity data a land figure takes.
const LAND_FIGURES: &str = "unit models";

pub(super) struct Walk {
    walker: Option<Walker>,
    /// The user's scale, kept while the mode is left and entered again.
    scale: f32,
    figure: Option<(FigureKey, Result<Figure, String>)>,
    /// `--walk`: where a window dump's walk starts and the keys it holds.
    start: Option<Start>,
    /// The capture clock a placed walk last advanced to.
    clock: Option<f32>,
    /// The eye the last frame was drawn through, which a harness run's record names.
    eye: Option<Placement>,
}

impl Walk {
    pub(super) fn new(start: Option<Start>) -> Self {
        Self {
            walker: None,
            scale: start.as_ref().map_or(1.0, |start| start.scale),
            figure: None,
            start,
            clock: None,
            eye: None,
        }
    }

    /// Whether the command line placed the walk.
    pub(super) fn placed(&self) -> bool {
        self.start.is_some()
    }

    /// Leaves the mode: the next start places the figure afresh, at the scale the user chose.
    pub(super) fn leave(&mut self) {
        self.walker = None;
    }

    /// Where the figure stands on the map, `None` while no walk runs.
    pub(super) fn standing(&self) -> Option<[f32; 2]> {
        self.walker
            .as_ref()
            .map(|walker| [walker.feet[0], walker.feet[2]])
    }

    /// Forgets the figure and the walker for another recording or content.
    pub(super) fn reset(&mut self) {
        self.walker = None;
        self.figure = None;
        self.clock = None;
    }

    /// Keeps the walker where it stands; its figure is chosen again from rebuilt content.
    pub(super) fn forget_figure(&mut self) {
        self.figure = None;
    }

    /// Whether the figure has to be chosen again for `key`.
    pub(super) fn stale(&self, key: &FigureKey) -> bool {
        self.figure.as_ref().is_none_or(|(chosen, _)| chosen != key)
    }

    pub(super) fn set_figure(&mut self, key: FigureKey, figure: Result<Figure, String>) {
        self.figure = Some((key, figure));
    }

    /// The figure's height at scale 1: its entity's, or a world unit without one.
    fn height(&self) -> f32 {
        match &self.figure {
            Some((_, Ok(figure))) if figure.height > 0.0 => figure.height,
            _ => FALLBACK_FIGURE,
        }
    }

    /// A placed walk's input for the capture frame at `time`: its held keys, advanced by the
    /// capture clock since the last frame. `None` when the command line placed no walk.
    pub(super) fn placed_frame(&mut self, time: f32) -> Option<Frame> {
        let start = self.start.as_ref()?;
        let dt = self.clock.map_or(0.0, |last| (time - last).max(0.0));
        self.clock = Some(time);
        Some(start.frame(dt))
    }

    /// Advances the walker one frame and gives the camera it is seen through. A new walk
    /// starts where the command line placed it, or on the ground at world `look_at` (chosen from
    /// the map, or the ground `map` looks at), facing the way `map` faces.
    pub(super) fn camera(
        &mut self,
        controls: &Frame,
        wheel: f32,
        map: Camera,
        look_at: [f32; 2],
        world: &World<'_>,
    ) -> Camera {
        let height = self.height();
        let walker = self.walker.get_or_insert_with(|| match &self.start {
            Some(start) => start.walker(world),
            None => {
                let forward = map.look_direction();
                Walker::standing(look_at, forward[0].atan2(forward[2]), self.scale, world)
            }
        });
        walker.update(controls, wheel, height, world);
        self.scale = walker.scale;
        let eye = walker.camera(map, height, world);
        self.eye = Some(eye.placement());
        eye
    }

    /// The eye the last frame walked through.
    pub(super) fn eye(&self) -> Option<Placement> {
        self.eye
    }

    /// The figure as the scene draws it, while walking with one.
    pub(super) fn player<'a>(&self, library: &'a ModelLibrary) -> Option<PlayerModel<'a>> {
        let walker = self.walker.as_ref()?;
        let Some((_, Ok(figure))) = &self.figure else {
            return None;
        };
        Some(PlayerModel {
            library,
            label: LAND_FIGURES,
            mesh: figure.mesh,
            instance: walker.instance(figure.height),
        })
    }

    /// What a window dump's walk drew: the figure, its scale and where it stands.
    pub(super) fn report(&self) -> Option<String> {
        let walker = self.walker.as_ref()?;
        let figure = match &self.figure {
            Some((_, Ok(figure))) => format!(
                "{} ({:?} level, {:.3} world units tall at scale 1)",
                figure.entity, figure.level, figure.height
            ),
            Some((_, Err(reason))) => format!("no figure: {reason}"),
            None => "no figure chosen".to_owned(),
        };
        Some(format!(
            "Walk: {figure}; scale {:.4}, {:?}, feet {:.3},{:.3},{:.3}, facing {:.1} degrees, {}",
            walker.scale,
            walker.gait,
            walker.feet[0],
            walker.feet[1],
            walker.feet[2],
            walker.facing.to_degrees(),
            if walker.walking(self.height()) {
                "walking"
            } else {
                "standing"
            }
        ))
    }

    /// Why the walk has no figure, for the map's diagnostics.
    pub(super) fn warning(&self) -> Option<String> {
        match &self.figure {
            Some((_, Err(reason))) => Some(format!(
                "Walk: the viewing country's infantry figure is not drawn: {reason}."
            )),
            _ => None,
        }
    }
}
