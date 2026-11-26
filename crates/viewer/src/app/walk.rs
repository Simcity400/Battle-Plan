
use crate::controls::Frame;
use crate::controls::walker::{FALLBACK_FIGURE, Start, View, Walker, World};
use viewer_gpu::camera::{Camera, Placement};
use viewer_foundation::perspective::Choice;
use viewer_models::ModelLibrary;
use viewer_world::scene::PlayerModel;
use viewer_unit_models::unit_models::Figure;

pub(super) type FigureKey = (replay_data::BranchId, i32, Choice);

const LAND_FIGURES: &str = "unit models";

pub(super) struct Walk {
    walker: Option<Walker>,
    scale: f32,
    view: View,
    figure: Option<(FigureKey, Result<Figure, String>)>,
    start: Option<Start>,
    clock: Option<f32>,
    eye: Option<Placement>,
}

impl Walk {
    pub(super) fn new(start: Option<Start>) -> Self {
        Self {
            walker: None,
            scale: start.as_ref().map_or(1.0, |start| start.scale),
            view: start.as_ref().map_or(View::First, |start| start.view),
            figure: None,
            start,
            clock: None,
            eye: None,
        }
    }

    pub(super) fn placed(&self) -> bool {
        self.start.is_some()
    }

    pub(super) fn leave(&mut self) {
        self.walker = None;
    }

    pub(super) fn standing(&self) -> Option<[f32; 2]> {
        self.walker
            .as_ref()
            .map(|walker| [walker.feet[0], walker.feet[2]])
    }

    pub(super) fn eye_pose(&self) -> Option<([f32; 3], f32, f32)> {
        let walker = self.walker.as_ref()?;
        Some((
            walker.eye_position(self.height()),
            walker.view_yaw,
            walker.view_pitch,
        ))
    }

    pub(super) fn state(&self) -> Option<&Walker> {
        self.walker.as_ref()
    }

    pub(super) fn figure_height(&self) -> f32 {
        self.height()
    }

    pub(super) fn figure_missing(&self) -> bool {
        matches!(&self.figure, Some((_, Err(_))))
    }

    pub(super) fn reset(&mut self) {
        self.walker = None;
        self.figure = None;
        self.clock = None;
    }

    pub(super) fn forget_figure(&mut self) {
        self.figure = None;
    }

    pub(super) fn stale(&self, key: &FigureKey) -> bool {
        self.figure.as_ref().is_none_or(|(chosen, _)| chosen != key)
    }

    pub(super) fn set_figure(&mut self, key: FigureKey, figure: Result<Figure, String>) {
        self.figure = Some((key, figure));
    }

    fn height(&self) -> f32 {
        match &self.figure {
            Some((_, Ok(figure))) if figure.height > 0.0 => figure.height,
            _ => FALLBACK_FIGURE,
        }
    }

    pub(super) fn placed_frame(&mut self, time: f32) -> Option<Frame> {
        let start = self.start.as_ref()?;
        let dt = self.clock.map_or(0.0, |last| (time - last).max(0.0));
        self.clock = Some(time);
        Some(start.frame(dt))
    }

    pub(super) fn camera(
        &mut self,
        controls: &Frame,
        wheel: f32,
        map: Camera,
        look_at: [f32; 2],
        world: &World<'_>,
    ) -> Camera {
        let height = self.height();
        let view = self.view;
        let walker = self.walker.get_or_insert_with(|| match &self.start {
            Some(start) => start.walker(world),
            None => {
                let forward = map.look_direction();
                let mut walker =
                    Walker::standing(look_at, forward[0].atan2(forward[2]), self.scale, world);
                walker.set_view(view);
                walker
            }
        });
        walker.update(controls, wheel, height, world);
        self.scale = walker.scale;
        self.view = walker.view;
        let eye = walker.camera(map, height, world);
        self.eye = Some(eye.placement());
        eye
    }

    pub(super) fn eye(&self) -> Option<Placement> {
        self.eye
    }

    pub(super) fn player<'a>(&self, library: &'a ModelLibrary) -> Option<PlayerModel<'a>> {
        let walker = self.walker.as_ref()?;
        if walker.view == View::First {
            return None;
        }
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
            "Walk: {figure}; {:?} view, scale {:.4}, {:?}, feet {:.3},{:.3},{:.3}, facing {:.1} degrees, {}",
            walker.view,
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

    pub(super) fn warning(&self) -> Option<String> {
        match &self.figure {
            Some((_, Err(reason))) => Some(format!(
                "Walk: the viewing country's infantry figure is not drawn: {reason}."
            )),
            _ => None,
        }
    }
}
