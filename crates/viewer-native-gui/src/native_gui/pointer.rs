
use super::{Holder, Layout, Pointed, Pointer, Target, lifecycle::Instance};
use content_reader_gui::gui::native::ButtonSounds;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
    pub at: [f32; 2],
    pub clip: [f32; 2],
}

impl Place {
    pub fn new(at: [f32; 2]) -> Self {
        Self { at, clip: at }
    }

    fn local(self, origin: [f32; 2]) -> Self {
        let shift = |p: [f32; 2]| [p[0] - origin[0], p[1] - origin[1]];
        Self {
            at: shift(self.at),
            clip: shift(self.clip),
        }
    }

    fn item(self, holder: &Holder, place: [i32; 2]) -> Option<Self> {
        let inside = holder.clip.is_none_or(|bounds| {
            (bounds[0] as f32..bounds[2] as f32).contains(&self.clip[0])
                && (bounds[1] as f32..bounds[3] as f32).contains(&self.clip[1])
        });
        let k = holder.scale;
        inside.then(|| Self {
            at: [0, 1].map(|axis| (self.at[axis] - holder.origin[axis]) / k - place[axis] as f32),
            clip: [0, 1]
                .map(|axis| self.clip[axis] - (holder.logical[axis] + place[axis]) as f32),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Button {
    pub down: bool,
    pub place: Option<Place>,
    pub order: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    pub place: Option<Place>,
    pub buttons: Vec<Button>,
    pub held: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    pub place: Option<Place>,
    pub press: bool,
    pub held: bool,
    pub order: usize,
}

pub const LAST: usize = usize::MAX;

impl Input {
    pub fn at(&self) -> Option<[f32; 2]> {
        self.place.map(|place| place.at)
    }

    pub fn local(&self, origin: [f32; 2]) -> Self {
        self.map(|place| Some(place.local(origin)))
    }

    pub fn item(&self, holder: &Holder, place: [i32; 2]) -> Self {
        self.map(|at| at.item(holder, place))
    }

    pub fn filter(&self, keep: impl Fn([f32; 2]) -> bool) -> Self {
        self.map(|place| keep(place.at).then_some(place))
    }

    pub fn shared<T: Copy + PartialEq>(
        &self,
        mut holder: Option<T>,
        owner: impl Fn([f32; 2]) -> Option<T>,
        me: T,
    ) -> Self {
        let mut input = self.clone();
        for button in &mut input.buttons {
            if button.down && holder.is_none() {
                holder = button.place.and_then(|place| owner(place.at));
            }
            if holder.is_some_and(|holder| holder != me) {
                button.place = None;
            }
            if !button.down {
                holder = None;
            }
        }
        if input.held && holder.is_some_and(|holder| holder != me) {
            input.place = None;
        }
        input
    }

    fn map(&self, place: impl Fn(Place) -> Option<Place>) -> Self {
        Self {
            place: self.place.and_then(&place),
            buttons: self
                .buttons
                .iter()
                .map(|button| Button {
                    place: button.place.and_then(&place),
                    ..*button
                })
                .collect(),
            held: self.held,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Over,
    Down,
    Up,
    Click,
    Check,
    Uncheck,
}

impl Sound {
    pub fn of(self, sounds: &ButtonSounds) -> Option<Vec<u8>> {
        match self {
            Self::Over => sounds.over.clone(),
            Self::Down => sounds.down.clone(),
            Self::Up => sounds.up.clone(),
            Self::Click | Self::Check => sounds.click.clone(),
            Self::Uncheck => sounds.uncheck.clone(),
        }
    }

    pub fn clicks(self) -> bool {
        matches!(self, Self::Click | Self::Check | Self::Uncheck)
    }

    pub fn played(self, control: &str, sounds: &ButtonSounds, order: usize) -> Option<Played> {
        Some(Played {
            effect: self.of(sounds)?,
            click: (self == Self::Click).then(|| control.to_owned()),
            order,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Played {
    pub effect: Vec<u8>,
    pub click: Option<String>,
    pub order: usize,
}

impl Played {
    pub fn within(&self, owner: &str) -> Self {
        Self {
            effect: self.effect.clone(),
            click: self.click.as_ref().map(|control| format!("{owner}/{control}")),
            order: self.order,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerState {
    pointed: Option<Pointed>,
    target: Option<Target>,
    boxes: Vec<String>,
    pressed: Option<String>,
    releasing: bool,
    step: Step,
    watched: Vec<String>,
    changing: bool,
    sounds: Vec<(String, Sound, usize)>,
    toggled: Vec<String>,
}

impl PointerState {
    pub fn new_frame(&mut self) {
        self.sounds.clear();
        self.toggled.clear();
    }

    pub fn holding(&self) -> bool {
        self.pressed.is_some()
    }

    pub fn begin(&mut self, instance: &mut Instance, step: Step) -> Pointer {
        self.step = step;
        self.releasing = !step.held && self.pressed.is_some();
        instance.point(self.pointed.clone(), &self.boxes);
        self.pointer()
    }

    pub fn pointer(&self) -> Pointer {
        Pointer {
            at: self.step.place.map_or([f32::NAN; 2], |place| place.at),
            pressed: self.pressed.clone(),
            over: None,
            clip_at: self.step.place.map(|place| place.clip),
            drawn: self.pointed.as_ref().map(|pointed| pointed.path.clone()),
            drawn_boxes: self.boxes.clone(),
        }
    }

    pub fn withdraw(&mut self) {
        self.step.place = None;
    }

    pub fn end(&mut self, layout: &Layout) -> bool {
        let drawn = |path: &str| layout.pointed.as_ref().is_some_and(|p| p.path == path);
        let target = layout.target.as_ref();
        let releasing = std::mem::take(&mut self.releasing);
        if releasing
            && let Some(released) = self.pressed.take()
            && target.is_some_and(|target| !target.check_box && target.path == released)
            && drawn(&released)
        {
            self.sounds.push((released, Sound::Click, self.step.order));
        }
        let order = self.step.order;
        let mut changed = Vec::new();
        let (before, after) = (&self.pointed, &layout.pointed);
        if before != after {
            if let Some(before) = before
                && after.as_ref().is_none_or(|after| after.path != before.path)
            {
                self.sounds.push((before.path.clone(), Sound::Up, order));
            }
            if let Some(after) = after {
                let sound = if after.down { Sound::Down } else { Sound::Over };
                self.sounds.push((after.path.clone(), sound, order));
            }
            changed.extend(before.iter().chain(after).map(|pointed| pointed.path.clone()));
        }
        let entered = layout.boxes.iter().filter(|path| !self.boxes.contains(path));
        let left = self.boxes.iter().filter(|path| !layout.boxes.contains(path));
        changed.extend(entered.chain(left).cloned());
        let running = |path: &String| {
            layout
                .parts
                .iter()
                .any(|part| part.path == *path && part.shading.changing())
        };
        self.watched.retain(running);
        self.changing = !changed.is_empty() || !self.watched.is_empty();
        for path in changed {
            if !self.watched.contains(&path) {
                self.watched.push(path);
            }
        }
        self.pointed.clone_from(&layout.pointed);
        self.target.clone_from(&layout.target);
        self.boxes.clone_from(&layout.boxes);
        if releasing {
            self.changing = true;
            return true;
        }
        if !self.step.press || self.pressed.is_some() {
            return false;
        }
        match target {
            Some(target) if target.check_box => {
                let toggled = self.toggled.iter().position(|path| *path == target.path);
                let sound = if target.checked != toggled.is_some() {
                    Sound::Uncheck
                } else {
                    Sound::Check
                };
                match toggled {
                    Some(at) => {
                        self.toggled.swap_remove(at);
                    }
                    None => self.toggled.push(target.path.clone()),
                }
                self.sounds.push((target.path.clone(), sound, order));
                false
            }
            Some(target) if drawn(&target.path) => {
                self.pressed = Some(target.path.clone());
                self.changing = true;
                true
            }
            _ => false,
        }
    }

    pub fn changing(&self) -> bool {
        self.changing
    }

    pub fn sounds(&self) -> &[(String, Sound, usize)] {
        &self.sounds
    }

    pub fn clicks(&self) -> impl Iterator<Item = (&str, usize)> {
        self.sounds
            .iter()
            .filter(|(_, sound, _)| sound.clicks())
            .map(|(path, _, order)| (path.as_str(), *order))
    }

    pub fn highlighted(&self) -> Option<&str> {
        self.target
            .as_ref()
            .filter(|target| target.check_box)
            .map(|target| target.path.as_str())
            .or_else(|| self.pointed.as_ref().map(|pointed| pointed.path.as_str()))
    }
}
