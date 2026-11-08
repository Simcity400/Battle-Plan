//! One window's pointer between frames: the button held in Over or Down, and the one pressed.

use super::{Holder, Layout, Pointed, Pointer, Target, lifecycle::Instance};
use installed_content_core::gui::native::ButtonSounds;

/// A place of the pointer in a window: in its space, and in its clips' unscaled space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
    pub at: [f32; 2],
    pub clip: [f32; 2],
}

impl Place {
    /// A place of the view, where the window and its clips share one space.
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

    /// The place in an item `holder` places at `place`; `None` outside the holder's clip.
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

/// One change of the primary button, where it happened; `None` when not over the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Button {
    pub down: bool,
    pub place: Option<Place>,
    /// Its index among the frame's changes, which orders what it does across windows.
    pub order: usize,
}

/// The pointer as one frame's input reaches a window, in that window's space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    /// The frame's last place; `None` when elsewhere or over something drawn above the window.
    pub place: Option<Place>,
    /// The primary button's changes this frame, in order.
    pub buttons: Vec<Button>,
    /// The primary button is down at the frame's end.
    pub held: bool,
}

/// One step of a frame's input: a button change where it happened, or the last place.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    pub place: Option<Place>,
    pub press: bool,
    pub held: bool,
    /// The change's [`Button::order`]; [`LAST`] for the last place.
    pub order: usize,
}

/// The order of what the frame's last place does: after every button change.
pub const LAST: usize = usize::MAX;

impl Input {
    pub fn at(&self) -> Option<[f32; 2]> {
        self.place.map(|place| place.at)
    }

    /// The input of a window whose top-left is `origin`.
    pub fn local(&self, origin: [f32; 2]) -> Self {
        self.map(|place| Some(place.local(origin)))
    }

    /// The input of an item `holder` places at `place`.
    pub fn item(&self, holder: &Holder, place: [i32; 2]) -> Self {
        self.map(|at| at.item(holder, place))
    }

    /// Every place kept where `keep` holds; the others are over something else.
    pub fn filter(&self, keep: impl Fn([f32; 2]) -> bool) -> Self {
        self.map(|place| keep(place.at).then_some(place))
    }

    /// The input `me` takes among windows sharing one primary button: from a press `holder`
    /// holds, or the one `owner` gives the place a press lands, until its release, the pointer is
    /// away from every other window.
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

/// A sound a control's state change plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Over,
    Down,
    Up,
    Click,
    /// A check box checked or unchecked by a press.
    Check,
    Uncheck,
}

impl Sound {
    /// The effect this change plays among a button's `sounds`.
    pub fn of(self, sounds: &ButtonSounds) -> Option<Vec<u8>> {
        match self {
            Self::Over => sounds.over.clone(),
            Self::Down => sounds.down.clone(),
            Self::Up => sounds.up.clone(),
            Self::Click | Self::Check => sounds.click.clone(),
            Self::Uncheck => sounds.uncheck.clone(),
        }
    }

    /// Whether the change acts as the control's click.
    pub fn clicks(self) -> bool {
        matches!(self, Self::Click | Self::Check | Self::Uncheck)
    }

    /// The sound `control` plays for this change, `order` in the frame, when it has one.
    pub fn played(self, control: &str, sounds: &ButtonSounds, order: usize) -> Option<Played> {
        Some(Played {
            effect: self.of(sounds)?,
            click: (self == Self::Click).then(|| control.to_owned()),
            order,
        })
    }
}

/// A sound an interface control's state change starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Played {
    pub effect: Vec<u8>,
    /// The clicked button: the game keeps one click sound per button, which its next click stops.
    pub click: Option<String>,
    /// The [`Step::order`] of the change that played it.
    pub order: usize,
}

impl Played {
    /// The sound as an item of `owner` plays it: its control is named within the owner.
    pub fn within(&self, owner: &str) -> Self {
        Self {
            effect: self.effect.clone(),
            click: self.click.as_ref().map(|control| format!("{owner}/{control}")),
            order: self.order,
        }
    }
}

/// A window's pointer state ([`Self::begin`] before each step's layout, [`Self::end`] after).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerState {
    pointed: Option<Pointed>,
    /// The topmost control and every check box under the pointer at the last layout.
    target: Option<Target>,
    boxes: Vec<String>,
    pressed: Option<String>,
    /// This step releases the held button: its layout still draws it Down, as the release is
    /// hit-tested, and the next lays it out released.
    releasing: bool,
    step: Step,
    /// The controls whose state changed, until their effects settle.
    watched: Vec<String>,
    /// A changed state awaits its tick, or a watched control's effect still runs.
    changing: bool,
    /// The controls whose state changed this frame, with the sound each change plays and its
    /// [`Step::order`], in order.
    sounds: Vec<(String, Sound, usize)>,
    /// The check boxes this frame's presses toggled an odd number of times, before their code
    /// writes the new state.
    toggled: Vec<String>,
}

impl PointerState {
    /// Starts a frame: its sounds are this frame's alone.
    pub fn new_frame(&mut self) {
        self.sounds.clear();
        self.toggled.clear();
    }

    /// Whether the primary button holds one of this window's controls.
    pub fn holding(&self) -> bool {
        self.pressed.is_some()
    }

    /// Before a step's layout: takes the step and gives the layout's pointer.
    pub fn begin(&mut self, instance: &mut Instance, step: Step) -> Pointer {
        self.step = step;
        self.releasing = !step.held && self.pressed.is_some();
        instance.point(self.pointed.clone(), &self.boxes);
        self.pointer()
    }

    /// The pointer this step's layout takes.
    pub fn pointer(&self) -> Pointer {
        Pointer {
            // Outside every rectangle, so nothing is hovered.
            at: self.step.place.map_or([f32::NAN; 2], |place| place.at),
            pressed: self.pressed.clone(),
            over: None,
            clip_at: self.step.place.map(|place| place.clip),
            drawn: self.pointed.as_ref().map(|pointed| pointed.path.clone()),
            drawn_boxes: self.boxes.clone(),
        }
    }

    /// Takes the pointer from this window: something drawn above it is under the pointer.
    pub fn withdraw(&mut self) {
        self.step.place = None;
    }

    /// After a step's layout: a check box acts on its press, a button on a release over it after
    /// its press, then each state change plays its sound. True when a press took a button or a
    /// release let it go, which lays out again in the new state.
    pub fn end(&mut self, layout: &Layout) -> bool {
        // An enabled button under the pointer is the one the layout drew Over or Down.
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
            // A control that left its state is ticked into Up, whose effect may run too.
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

    /// Whether a changed state awaits its tick or the watched button's effect still runs.
    pub fn changing(&self) -> bool {
        self.changing
    }

    /// The controls whose state changed this frame, the sound each change plays and its order.
    pub fn sounds(&self) -> &[(String, Sound, usize)] {
        &self.sounds
    }

    /// The controls clicked this frame, with each click's order.
    pub fn clicks(&self) -> impl Iterator<Item = (&str, usize)> {
        self.sounds
            .iter()
            .filter(|(_, sound, _)| sound.clicks())
            .map(|(path, _, order)| (path.as_str(), *order))
    }

    /// The control drawn for the pointer: the topmost check box under it, else the button held
    /// in Over or Down.
    pub fn highlighted(&self) -> Option<&str> {
        self.target
            .as_ref()
            .filter(|target| target.check_box)
            .map(|target| target.path.as_str())
            .or_else(|| self.pointed.as_ref().map(|pointed| pointed.path.as_str()))
    }
}
