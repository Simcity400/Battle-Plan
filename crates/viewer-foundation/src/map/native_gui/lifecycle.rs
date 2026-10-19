//! Retained lifecycle for each constructed GUI window. A caller owns one [`Instance`] per
//! window identity, applies ordered calls such as [`Instance::set_position`],
//! [`Instance::gui_show`], [`Instance::hide`] and [`Instance::set_sprite`], then ticks and
//! lays it out. Related instances run each frame phase across all windows:
//! [`Instance::tick_clocks`], [`Instance::tick_transitions`] in [`Registration`] order, then
//! [`Instance::tick_draws`].
//!
//! The instance contains the complete constructed object graph, including inherited scrollbar
//! parts ([`widgets::scrollbar_part`]), container scrollbars
//! ([`widgets::Walk::content_scroll`]), named text/list scrollbars
//! ([`widgets::Walk::named_plain`]) and fixed-window backgrounds ([`fixed::background`]). Each
//! has its own state and [`Site`].
//!
//! The instance retains:
//! - Visibility and hide bits. A guarded Show changes nothing; interaction locking is unused
//!   by map icons.
//! - Fades and movements for every container. Show fades and opacity writes reach child
//!   windows; hide fades propagate when the parent does not fade. Showing and immediate or
//!   completed hiding reach child controls and windows. Transition windows register after their
//!   children, including drop-down headers and expanded windows, and tick in reverse
//!   registration order.
//! - Smoothed container scrollbar values that move content on the clock.
//! - Drawable clocks: frame playback, cornered-tile pulses, first-draw animation time,
//!   state-effect restarts at the first draw after a state change, and trigger-record starts.
//!   All use [`Tick::time`] and [`Bindings::time`]. Replacing a drawable starts fresh state
//!   ([`Runtime`]); a nonterminating clock wrap is refused where it draws.
//! - Drawables and control writes. Failed or unchanged sprite lookups preserve the drawable;
//!   frames, text, colours and progress values persist until overwritten. Reusing a pooled
//!   window preserves this state.
//!
//! Prepared hours record [`Calls`] without an instance; presented frames apply them with
//! [`Calls::apply`]. Background jobs never share an instance.

use super::*;
use std::borrow::Cow;

/// One presented frame of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    /// `Time`: the view's presentation clock in seconds, stored as f32 once per frame. Every
    /// drawable timestamp and draw uses it; one view shares one clock across its instances.
    pub time: f32,
    /// Seconds advanced by this frame for drawable clocks and window transitions; a new
    /// Instance's first step may be zero.
    pub dt: f32,
}

/// An element's visibility flags, initialized from the authored `hide` bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    /// Whether this element is currently visible.
    pub(super) visible: bool,
    /// The hide guard, which keeps every show call from showing it.
    pub(super) hide: bool,
    /// A code Hide reached it. For a plain scrollbar this hides range icons shown during
    /// construction; only a Show allowed by its hide bit shows them again.
    pub(super) hidden_by_code: bool,
}

impl Shown {
    /// As a layout with no lifecycle takes an element: shown unless its authored `hide` is set,
    /// as a window its code shows leaves it.
    pub(super) fn shown_by_code(element: &Element) -> Self {
        Self {
            visible: !element.hide,
            hide: element.hide,
            hidden_by_code: false,
        }
    }

    /// Sets the visible bit unless the hide bit guards the Show. Returns whether it showed.
    pub(super) fn show(&mut self) -> bool {
        if self.hide {
            return false;
        }
        self.visible = true;
        true
    }

    /// Clears the visible bit.
    pub(super) fn hide(&mut self) {
        self.visible = false;
        self.hidden_by_code = true;
    }
}

/// Template milliseconds as seconds (`/ 1000.0f`).
fn seconds(milliseconds: i32) -> f32 {
    milliseconds as f32 / 1000.0
}

/// Preserves coordinate flags and bounds, rounding percentages converted from pixels.
fn coordinate_pixels(coordinate: &mut Coordinate, pixels: i32, parent: i32) {
    coordinate.value = if !coordinate.percent {
        pixels
    } else if parent == 0 {
        0
    } else {
        let share = pixels as f32 * 100.0 / parent as f32;
        truncate_f32(if share < 0.0 {
            (share - 0.5).ceil()
        } else {
            (share + 0.5).floor()
        })
    };
}

fn position_pixels(position: &mut native::Position, point: [i32; 2], parent: [i32; 2]) {
    coordinate_pixels(&mut position.x, point[0], parent[0]);
    coordinate_pixels(&mut position.y, point[1], parent[1]);
}

/// One container's fade and movement (`container window`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Window {
    /// The parent's size the window's positions are evaluated in.
    parent: [i32; 2],
    /// The native size and wrapper extents last relaid out.
    size: native::Size,
    frame: Frame,
    /// The materialized show position used to evaluate the runtime size.
    show_position: native::Position,
    /// The saved anchored point, unchanged by later position writes.
    anchored_position: [i32; 2],
    /// Fade state: 0 still, 1 fading in, 2 fading out.
    fade: u8,
    /// Elapsed fade time in seconds.
    fade_elapsed: f32,
    /// Current window opacity.
    opacity: f32,
    /// The evaluated window position, which the movement starts from.
    current: [i32; 2],
    /// The last supplied top-left position, where the window draws.
    position: [i32; 2],
    /// The movement target position.
    target: [i32; 2],
    /// Displacement from the movement start to its target, per axis.
    delta: [f32; 2],
    /// Elapsed movement time in seconds.
    elapsed: f32,
    /// Whether the movement has arrived at its target.
    done: bool,
    /// Movement mode: 0 none, 1 show, 2 hide.
    mode: u8,
    /// The running movement's easing and duration in seconds.
    kind: Animation,
    duration: f32,
    /// Whether movement completion hides the window and its children or the window alone.
    hide_children: bool,
    hide_only: bool,
}

/// A frame-animated sprite's elapsed time, playing state and last frame written by its updater.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrameClock {
    elapsed: f32,
    playing: bool,
    /// `None` while the drawable holds the frame its control or code set.
    frame: Option<i32>,
}

impl FrameClock {
    /// Starts at zero elapsed time, stopped, with the frame supplied by its control.
    pub(super) fn new() -> Self {
        Self {
            elapsed: 0.0,
            playing: false,
            frame: None,
        }
    }

    /// Advances a shown drawable only when playing or configured to play on show. Duration
    /// comes from the unsigned frame count and seconds per frame, with a minimum of 0.01
    /// seconds. A nonlooping sprite stops at its last frame after the duration; a looping one
    /// wraps by duration plus pause and shows frame 1 during the pause. A stopped clock
    /// preserves the code-selected frame. Refuses a wrap whose nonpositive period or
    /// floating-point subtraction cannot advance, because the game never leaves that loop.
    pub(super) fn update(&mut self, kind: &SpriteKind, frames: i32, dt: f32) -> Result<(), String> {
        let SpriteKind::FrameAnimated {
            seconds_per_frame,
            looping,
            play_on_show,
            pause,
        } = *kind
        else {
            return Ok(());
        };
        if !(self.playing || play_on_show) {
            return Ok(());
        }
        // The frame count is stored unclamped and its bits convert unsigned for clock
        // arithmetic.
        let frame_count = frames as u32 as f32;
        let duration = (frame_count * seconds_per_frame).max(0.01);
        self.elapsed += dt;
        if !looping && self.elapsed > duration {
            self.playing = false;
            self.frame = Some(frames);
            return Ok(());
        }
        while self.elapsed > pause + duration {
            let next = self.elapsed - (pause + duration);
            // No progress towards the period (it is 0 or less, or below the time's precision):
            // Every later pass is the same or further from the exit.
            if next >= self.elapsed {
                return Err(format!(
                    "a frame-animated sprite's time {} exceeds its wrap period {} (duration \
                     {duration} plus pause {pause}), where wrapping subtracts the period \
                     forever: the game hangs",
                    self.elapsed,
                    pause + duration
                ));
            }
            self.elapsed = next;
        }
        self.frame = Some(if self.elapsed < duration {
            // Truncate the frame number using the f32 integer conversion.
            installed_content_core::crt::truncate_f32(self.elapsed / duration * frame_count + 1.0)
        } else {
            1
        });
        Ok(())
    }

    /// Starts the animation by setting its playing state.
    #[cfg(test)]
    pub(super) fn play(&mut self) {
        self.playing = true;
    }

    /// The frame its updater last wrote.
    #[cfg(test)]
    pub(super) fn frame(&self) -> Option<i32> {
        self.frame
    }

    /// Whether the updater changes it on the next tick.
    fn runs(&self, kind: &SpriteKind) -> bool {
        matches!(*kind, SpriteKind::FrameAnimated { play_on_show, .. } if self.playing || play_on_show)
    }
}

/// `AnimationTime`: the time into the frame `frame` the drawable holds over the type's seconds
/// per frame, clamped to `[0, 1]`, from its clock's `elapsed`.
pub(super) fn animation_time(frame: i32, elapsed: f32, kind: &SpriteKind) -> f32 {
    let SpriteKind::FrameAnimated {
        seconds_per_frame, ..
    } = *kind
    else {
        return 0.0;
    };
    ((elapsed - (frame - 1) as f32 * seconds_per_frame) / seconds_per_frame).clamp(0.0, 1.0)
}

/// A looping cornered tile advances its pulse phase by elapsed time and rate, wraps below one,
/// and produces a doubled triangle wave.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Pulse {
    phase: f32,
}

impl Pulse {
    /// Advances the pulse for one frame at `rate`. Refuses an infinite phase or one so large
    /// that subtracting one cannot change it, because wrapping would never finish.
    pub(super) fn update(&mut self, rate: f32, dt: f32) -> Result<(), String> {
        self.phase += dt * rate;
        while self.phase > 1.0 {
            let next = self.phase - 1.0;
            if next == self.phase {
                return Err(format!(
                    "a cornered tile's pulse phase reached {}, where wrapping subtracts 1 \
                     forever: the game hangs",
                    self.phase
                ));
            }
            self.phase = next;
        }
        Ok(())
    }

    pub(super) fn alpha(&self) -> f32 {
        let tri = if self.phase > 0.5 {
            1.0 - self.phase
        } else {
            self.phase
        };
        2.0 * tri
    }
}

/// A drawable's frame clock and its sprite's kind, as last resolved.
#[derive(Clone, Debug, PartialEq)]
struct Clock {
    clock: FrameClock,
    kind: SpriteKind,
}

/// The state of the drawable a control currently shows, retained until its replacement.
/// Replacement resets the first-draw timestamp, state-effect restart, trigger records, frame
/// playback and pulse. The new drawable inherits the previous state and marks an effect
/// restart at its first draw. A call that preserves the drawable preserves all this state.
#[derive(Clone, Debug, PartialEq)]
struct Runtime {
    sprite: Vec<u8>,
    made: u32,
    clock: Option<Clock>,
    pulse: Option<Pulse>,
    /// Why its clock or pulse hangs the game.
    hang: Option<String>,
    /// The graphics clock at its first draw.
    first_draw: Option<f32>,
    effect: Option<StateEffect>,
    /// The instance pair it last saw.
    triggers: Vec<(f32, u32)>,
}

impl Runtime {
    /// The state of the drawable built from `sprite` by setter call `made`, as its constructor
    /// leaves it.
    fn new(sprite: Vec<u8>, made: u32) -> Self {
        Self {
            sprite,
            made,
            clock: None,
            pulse: None,
            hang: None,
            first_draw: None,
            effect: None,
            triggers: Vec::new(),
        }
    }

    fn is(&self, sprite: &[u8], made: u32) -> bool {
        self.made == made && self.sprite == sprite
    }
}

/// A drawable a setter made: its sprite, the setter, and which of the control's successful
/// setter calls it is (from 1).
#[derive(Clone, Debug, PartialEq)]
struct Made {
    sprite: Vec<u8>,
    setter: SpriteSetter,
    made: u32,
}

/// A code call on a control's drawable, in the order the code makes them.
#[derive(Clone, Debug, PartialEq)]
enum Event {
    /// Sprite replacement through `setter` ([`Instance::set_sprite`]).
    Sprite(Vec<u8>, SpriteSetter),
    /// Frame assignment ([`Instance::set_frame`]).
    Frame(i32),
    /// A normalized progress setter writes its rounded raw state.
    ProgressFloat(i32),
    /// The code starts the drawable's animation ([`Instance::play`]).
    #[cfg(any(test, feature = "test-support"))]
    Play,
}

/// The drawable a control shows after its pending calls: [`Instance::retained`].
#[derive(Clone, Debug, PartialEq)]
struct Retained {
    made: Option<Made>,
    /// Every successful setter call so far.
    count: u32,
    /// The frame the code set on the current drawable (`None`: its own).
    frame: Option<i32>,
    progress_float: Option<i32>,
    /// A frame assignment reached the current drawable since the last tick; it holds that frame.
    framed: bool,
    /// An animation start reached the current drawable since the last tick; it is playing.
    played: bool,
}

/// The state effect of a drawable as last drawn: the state, the graphics clock at its last
/// restart, and how long after it its effect keeps changing ([`Effect::settles`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct StateEffect {
    state: u32,
    at: f32,
    settles: Settles,
}

/// The related objects reached when an object's visibility changes.
#[derive(Clone, Debug, PartialEq)]
enum Drawables {
    None,
    /// A plain scrollbar: its right and left buttons, track and slider, each by its own Show
    /// and Hide; its range icons refreshed on a Show (shown where it places them, hidden
    /// elsewhere) and hidden on a Hide.
    Scrollbar {
        parts: Vec<usize>,
        icons: Vec<(usize, bool)>,
    },
    /// Every part by its own Show and Hide: an extended scrollbar's five.
    All(Vec<usize>),
    /// A drop-down shows its expand button and, when previously invisible, its header.
    /// Hiding it hides both and the expanded window, which only input opens.
    DropDown {
        expand: Option<usize>,
        expanded: Option<usize>,
        header: Option<usize>,
    },
    /// An `instantTextBoxType`'s scrollbar: shown while the box's scrollbar-hidden flag is
    /// clear, hidden with the box. Each text the code writes refreshes it at once
    /// ([`Instance::set_text`]): `state` is the bar as the writes left it, `written` the text
    /// last written through it.
    TextBox {
        bar: usize,
        flag: bool,
        state: widgets::PlainState,
        scroll: f32,
        written: Option<String>,
    },
    /// A list box's scrollbar: hidden with the box, shown only by the rows its code appends,
    /// whose places and scrollbar [`widgets::list_box_rows`] lays out apart from the instance.
    ListBox {
        bar: usize,
    },
}

/// A container scrollbar smooths its current value toward the target, moving window content
/// each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Smooth {
    vertical: bool,
    /// The bar's node and its slider's.
    bar: usize,
    slider: Option<usize>,
    state: widgets::ContainerBar,
    enabled: bool,
    factor: i64,
    current: i64,
    notified: Option<i32>,
}

impl Smooth {
    /// Snaps and updates the track even without an observer notification.
    fn set_target(&mut self, template: &native::ExtendedScrollbar, value: i64, notify: bool) {
        self.state.value = self.state.range.snap(value);
        self.state.track_frame = widgets::track_frame(template, self.state.range, self.state.value);
        if notify {
            self.notified = Some(widgets::scroll_pixels(if self.enabled {
                self.current
            } else {
                self.state.value
            }));
        }
    }

    /// Advances the current value when it differs from the target, notifying the observer. It
    /// snaps within 0.1 or while the slider is hidden; otherwise distance and factor use
    /// 64-bit wrapping fixed-point arithmetic with C division.
    fn update(&mut self, slider: bool) {
        if self.current == self.state.value {
            return;
        }
        let distance = self.state.value.wrapping_sub(self.current);
        self.current = if distance.wrapping_abs() <= 10_000 || !slider {
            self.state.value
        } else {
            self.current
                .wrapping_add(distance.wrapping_mul(self.factor) / 100_000)
        };
        self.notified = Some(widgets::scroll_pixels(self.current));
    }
}

/// One object of the instance.
#[derive(Clone, Debug)]
struct Node {
    parent: Option<usize>,
    /// A child of its parent (not a background or a part it holds).
    child: bool,
    /// A part its holder shows and hides through the part's own visibility calls
    /// (an extended scrollbar's, a drop-down's, a plain scrollbar's, a window's own
    /// scrollbar, a text or list box's scrollbar, a `windowType`'s background), which keeps its
    /// own visible bit; any other nested element (a background) follows its holder's.
    part: bool,
    /// Built from another template for the node `owner` by `role` (`None`: an element of the
    /// template).
    built: Option<(usize, Role)>,
    /// Drawn in a button's state, including disabled greying: a child button, scrollbar knob,
    /// extended-scrollbar increase/decrease button or drop-down expand button, excluding drag
    /// parts and backgrounds.
    stateful: bool,
    /// The authored `hide`, which the walk reads for a plain scrollbar's range icons.
    hide: bool,
    /// The direct children, in declaration order.
    children: Vec<usize>,
    /// The direct child windows, in declaration order.
    windows: Vec<usize>,
    kind: Kind,
    shown: Shown,
    window: Option<Window>,
    drawables: Drawables,
    /// Objects shown and hidden with their window when their ownership flag is set: container
    /// scrollbars or a fixed-window background.
    owned: Vec<(usize, bool)>,
    /// A window's smoothed scrollbars.
    smooth: Vec<Smooth>,
    /// What the code set on it, kept until it writes it again.
    control: Bound,
    /// Its drawable, the one a setter made, and the calls on it since the last tick.
    made: Option<Made>,
    made_count: u32,
    events: Vec<Event>,
    /// What the game keeps on the drawable it shows ([`Runtime`]).
    runtime: Option<Runtime>,
}

impl Node {
    fn new(kind: Kind, shown: Shown) -> Self {
        Self {
            parent: None,
            child: false,
            part: false,
            built: None,
            stateful: false,
            hide: shown.hide,
            children: Vec::new(),
            windows: Vec::new(),
            kind,
            shown,
            window: None,
            drawables: Drawables::None,
            owned: Vec::new(),
            smooth: Vec::new(),
            control: Bound::default(),
            made: None,
            made_count: 0,
            events: Vec::new(),
            runtime: None,
        }
    }
}

/// A grid or overlapping list reached by a visibility call: it shows or hides every
/// item it holds, windows its caller keeps as their own instances
/// ([`Instance::gui_show`] on each).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub path: String,
    pub shown: bool,
}

/// Where an instance's windows stand in the shared transition list, ordered by instance
/// construction. Each configured window registers once built, after its children. One
/// Instance's registrations are contiguous and later instances follow it. Consumers tick
/// transitions ([`Instance::tick_transitions`]) in descending registration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Registration(u64);

impl Registration {
    fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// The frame phases run across every object before advancing: drawable clocks
/// ([`Instance::tick_clocks`]), window transitions ([`Instance::tick_transitions`] in
/// [`Registration`] order), then drawable draw records ([`Instance::tick_draws`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Drawn, or never ticked: the next frame starts with its clocks.
    Drawn,
    /// Its clocks ran for the frame: its transitions come next.
    Clocked,
    /// Its transitions ran: its draw comes next.
    Moved,
}

/// One constructed window and its descendants, as the game keeps them between frames.
#[derive(Clone, Debug)]
pub struct Instance {
    /// The template's root path and element count: the instance answers only its own template.
    template: String,
    count: usize,
    /// The GUI viewport (`graphics state`).
    viewport: [i32; 2],
    nodes: Vec<Node>,
    /// The windows the GUI lifecycle ticks (the GUI lifecycle), in registration order.
    registered: Vec<usize>,
    /// Where its windows stand in the GUI lifecycle's list among every instance's ([`Registration`]).
    registration: Registration,
    /// How far the frame being ticked has come ([`Phase`]).
    phase: Phase,
    /// The graphics clock `Time` of the last tick.
    time: f32,
    /// The step of the frame being ticked, which its transitions take.
    dt: f32,
    /// Ticked since it was made: a new instance is pending its first (`dt = 0`) tick.
    ticked: bool,
    /// Whether a draw that keeps the previous draw's program still changes on the clock, as the
    /// last tick resolved it in draw order ([`Self::kept_changes`]).
    kept: bool,
    held: Vec<Held>,
}

/// Every element of `root`, in pre-order over [`Element::nested`]: the element, its parent's
/// index and whether it is a child of it.
fn elements(root: &Element) -> Vec<(&Element, Option<usize>, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![(root, None, true)];
    while let Some((element, parent, child)) = stack.pop() {
        let index = out.len();
        out.push((element, parent, child));
        for nested in element.nested().into_iter().rev() {
            let is_child = element.children.iter().any(|own| std::ptr::eq(own, nested));
            stack.push((nested, Some(index), is_child));
        }
    }
    out
}

/// One object of a window's construction ([`graph`]).
struct Object<'a> {
    element: &'a Element,
    parent: Option<usize>,
    child: bool,
    part: bool,
    built: Option<(usize, Role)>,
}

/// The objects `root`'s construction builds, in the instance's node order: every element of the
/// template in pre-order over [`Element::nested`], then, owner by owner, each object built from
/// another template ([`Role`]), found through the walk's own lookups so that the instance and
/// the walk hold the same objects: a plain scrollbar child's parts ([`Walk::plain`]), a
/// container's own scrollbars and their five parts ([`Walk::content_scroll`]), a text box's and a
/// list box's scrollbar and its parts ([`Walk::named_plain`]), a `windowType`'s background
/// ([`fixed::background`]). A lookup the game fails on builds nothing here; the walk refuses it
/// where it draws.
fn graph<'a>(
    root: &'a Element,
    resources: &'a dyn Resources,
    empty: &'a Bindings,
) -> Vec<Object<'a>> {
    let authored = elements(root);
    let mut objects: Vec<Object> = authored
        .iter()
        .map(|&(element, parent, child)| Object {
            element,
            parent,
            child,
            part: !child
                && parent.is_some_and(|up| {
                    matches!(
                        authored[up].0.fields,
                        Fields::ExtendedScrollbar(_) | Fields::DropDown(_)
                    )
                }),
            built: None,
        })
        .collect();
    let authored = objects.len();
    let mut walk = Walk::new(resources, empty);
    fn parts<'a>(
        objects: &mut Vec<Object<'a>>,
        plain: &widgets::Plain<'a>,
        owner: usize,
        parent: usize,
    ) {
        for (slot, part) in plain.parts().into_iter().enumerate() {
            if let Some(element) = part {
                objects.push(Object {
                    element,
                    parent: Some(parent),
                    child: false,
                    part: true,
                    built: Some((owner, Role::Part(slot as u8))),
                });
            }
        }
    }
    for owner in 0..authored {
        let element = objects[owner].element;
        match &element.fields {
            Fields::Container(container) => {
                let scroll = walk.content_scroll(element, container);
                for (vertical, template, bar) in scroll.named() {
                    let role = Role::Bar(vertical);
                    let at = objects.len();
                    objects.push(Object {
                        element: template,
                        parent: Some(owner),
                        child: false,
                        part: true,
                        built: Some((owner, role)),
                    });
                    for part in [
                        &bar.background,
                        &bar.track,
                        &bar.slider,
                        &bar.increase_button,
                        &bar.decrease_button,
                    ]
                    .into_iter()
                    .filter_map(|part| part.as_deref())
                    {
                        objects.push(Object {
                            element: part,
                            parent: Some(at),
                            child: false,
                            part: true,
                            built: Some((owner, role)),
                        });
                    }
                }
            }
            Fields::Scrollbar(bar) => {
                if let Ok(plain) = walk.plain(element, bar) {
                    parts(&mut objects, &plain, owner, owner);
                }
            }
            Fields::TextBox(native::TextBox { scrollbar_type, .. })
            | Fields::ListBox(native::ListBox { scrollbar_type, .. }) => {
                if let Ok(Some(plain)) = walk.named_plain(element, scrollbar_type) {
                    let at = objects.len();
                    objects.push(Object {
                        element: plain.element(),
                        parent: Some(owner),
                        child: false,
                        part: true,
                        built: Some((owner, Role::Scrollbar)),
                    });
                    parts(&mut objects, &plain, owner, at);
                }
            }
            Fields::Window(_) => {
                if let Ok(Some(background)) =
                    fixed::background(element, &|name| resources.gui_type(name))
                {
                    objects.push(Object {
                        element: background,
                        parent: Some(owner),
                        child: false,
                        part: true,
                        built: Some((owner, Role::Background)),
                    });
                }
            }
            _ => {}
        }
    }
    objects
}

/// The current drawable or fallback ([`drawn_sprite`]); `None` when absent. A text-only button
/// uses its text object ([`text_only`]), with the last code-written text.
fn drawn(
    element: &Element,
    made: Option<&Made>,
    text: Option<&str>,
    resources: &dyn Resources,
) -> Option<(Vec<u8>, SpriteInfo)> {
    let lookup = |name: &[u8]| resources.sprite(name);
    match made {
        Some(made) => drawn_sprite(&made.sprite, lookup),
        None => {
            if let Some(button) = element.button()
                && element.kind != Kind::Checkbox
            {
                let shown = match text {
                    Some(text) => Some(text.to_owned()),
                    None => localised(resources, &button.text),
                };
                if text_only(button, &button.sprite, shown.as_deref()) {
                    return None;
                }
            }
            let (name, fallback) = own_sprite(element)?;
            if fallback {
                drawn_sprite(name, lookup)
            } else {
                lookup(name).map(|info| (name.to_vec(), info))
            }
        }
    }
}

/// The technique selected for state `state`.
fn technique(state: u32) -> &'static str {
    match state {
        2 => "Down",
        3 => "Over",
        4 => "Disable",
        _ => "Up",
    }
}

/// Whether the object `index` is drawn in a button's state ([`Node::stateful`]): drawn by the
/// walk as a button (`Walk::button`), not as a background or drag part.
fn stateful(objects: &[Object], index: usize) -> bool {
    let object = &objects[index];
    let element = object.element;
    if element.button().is_none() {
        return false;
    }
    let holder = object.parent.map(|up| objects[up].element);
    match object.built {
        // A plain scrollbar's buttons and range icons (`Walk::knob_part`).
        Some((_, Role::Part(slot))) => matches!(slot, 0 | 1 | 4 | 5),
        // A container scrollbar's increase and decrease buttons (`Walk::extended_parts`).
        Some((_, Role::Bar(_))) => holder.is_some_and(|holder| extended_button(holder, element)),
        Some(_) => false,
        None if object.child => element.kind != Kind::Checkbox,
        None => holder.is_some_and(|holder| {
            extended_button(holder, element)
                || matches!(&holder.fields, Fields::DropDown(drop) if drop
                    .expand_button
                    .as_deref()
                    .is_some_and(|button| std::ptr::eq(button, element)))
        }),
    }
}

/// Whether `element` is `holder`'s increase or decrease button, an extended scrollbar's.
fn extended_button(holder: &Element, element: &Element) -> bool {
    matches!(&holder.fields, Fields::ExtendedScrollbar(bar)
        if [&bar.increase_button, &bar.decrease_button]
            .into_iter()
            .any(|button| button.as_deref().is_some_and(|button| std::ptr::eq(button, element))))
}

impl Instance {
    /// Constructs `root` with no parent object. Its size and position use the GUI viewport,
    /// the device size divided by interface scale. Additional templates resolve through
    /// `resources` ([`graph`]). Fading windows and their descendants start transparent. All
    /// Windows and controls start hidden until shown. A `windowType` root represents a fixed
    /// window and retains no container fade or movement. Errors describe frame-evaluation
    /// failures.
    pub fn construct(
        root: &Element,
        viewport: [i32; 2],
        resources: &dyn Resources,
    ) -> Result<Self, String> {
        Self::construct_in(root, viewport, viewport, resources)
    }

    /// As [`Self::construct`], in a parent object of `parent` pixels.
    pub fn construct_in(
        root: &Element,
        viewport: [i32; 2],
        parent: [i32; 2],
        resources: &dyn Resources,
    ) -> Result<Self, String> {
        if root.container().is_none() && root.kind != Kind::Window {
            return Err(format!(
                "{} is neither a containerWindowType nor a windowType",
                root.path
            ));
        }
        let empty = Bindings {
            viewport: Some(viewport),
            ..Bindings::default()
        };
        let objects = graph(root, resources, &empty);
        let authored = elements(root).len();
        let mut nodes: Vec<Node> = Vec::with_capacity(objects.len());
        // The content size each container's children are evaluated in, and each window's frame.
        let mut content: Vec<Option<[i32; 2]>> = Vec::with_capacity(objects.len());
        let mut frames: Vec<Option<Frame>> = Vec::with_capacity(objects.len());
        for (index, object) in objects.iter().enumerate() {
            let element = object.element;
            let (window, frame) = match element.container() {
                Some(container) if object.built.is_none() => {
                    // A window under a window sits in its content; one a drop-down or another
                    // holder builds, in its position object the viewport's size.
                    let outer = match object.parent {
                        None => parent,
                        Some(up) => content[up].unwrap_or(viewport),
                    };
                    let frame = container_frame(element, Some(outer))?;
                    let show = anchored(
                        element,
                        container,
                        &container.show_position,
                        outer,
                        frame.size,
                        "show_position",
                    )?;
                    let mut show_position = container.show_position;
                    position_pixels(&mut show_position, show, outer);
                    let window = Window {
                        parent: outer,
                        size: container.size,
                        frame,
                        show_position,
                        anchored_position: evaluate_position(
                            &container.position,
                            Some(outer),
                            &element.path,
                        )?,
                        fade: 0,
                        fade_elapsed: 0.0,
                        opacity: 1.0,
                        current: frame.at,
                        position: frame.at,
                        target: frame.at,
                        delta: [0.0; 2],
                        elapsed: 0.0,
                        done: true,
                        mode: 0,
                        kind: Animation::None,
                        duration: 0.0,
                        hide_children: false,
                        hide_only: false,
                    };
                    (Some(window), Some(frame))
                }
                _ => (None, None),
            };
            content.push(frame.map(|frame| frame.content_size));
            frames.push(frame);
            // Construction leaves windows, controls and drawables hidden while preserving
            // authored hide guards. The first `gui_show` shows the admitted children.
            let shown = Shown {
                visible: false,
                ..Shown::shown_by_code(element)
            };
            let mut node = Node::new(element.kind, shown);
            node.parent = object.parent;
            node.child = object.child;
            node.part = object.part;
            node.built = object.built;
            node.stateful = stateful(&objects, index);
            node.window = window;
            nodes.push(node);
            if let Some(up) = object.parent
                && object.child
            {
                nodes[up].children.push(index);
                if element.container().is_some() {
                    nodes[up].windows.push(index);
                }
            }
        }
        // What each object's show call reaches, a window's own scrollbars with their flags
        // and smoothing, as the construction leaves them.
        let node_of = |element: &Element, among: std::ops::Range<usize>| {
            among
                .into_iter()
                .find(|&at| std::ptr::eq(objects[at].element, element))
        };
        let built_by = |owner: usize, parent: usize| -> Vec<usize> {
            (authored..objects.len())
                .filter(|&at| {
                    objects[at].built.is_some_and(|(own, _)| own == owner)
                        && objects[at].parent == Some(parent)
                })
                .collect()
        };
        let scrollbar = |parts: &[usize], places: [Option<[i32; 2]>; 2]| {
            let slot = |at: usize| match objects[at].built {
                Some((_, Role::Part(slot))) => slot,
                _ => unreachable!("a scrollbar part"),
            };
            Drawables::Scrollbar {
                parts: parts.iter().copied().filter(|&at| slot(at) < 4).collect(),
                icons: parts
                    .iter()
                    .copied()
                    .filter(|&at| slot(at) >= 4)
                    .map(|at| (at, places[usize::from(slot(at) - 4)].is_some()))
                    .collect(),
            }
        };
        let mut walk = Walk::new(resources, &empty);
        for index in 0..authored {
            let element = objects[index].element;
            match &element.fields {
                Fields::Container(container) => {
                    let bars = built_by(index, index);
                    if bars.is_empty() {
                        continue;
                    }
                    let scroll = walk.content_scroll(element, container);
                    let facts = frames[index]
                        .map(|frame| walk.bars(element, container, &scroll, frame))
                        .and_then(Result::ok)
                        .unwrap_or_default();
                    for at in bars {
                        let Some((_, Role::Bar(vertical))) = objects[at].built else {
                            continue;
                        };
                        let parts = built_by(index, at);
                        nodes[at].drawables = Drawables::All(parts.clone());
                        let fact = facts.iter().find(|fact| fact.vertical == vertical);
                        let flag = fact.is_some_and(|fact| fact.shown);
                        nodes[index].owned.push((at, flag));
                        if let Some(fact) = fact {
                            let slider = fact.bar.slider.as_deref().and_then(|slider| {
                                parts
                                    .iter()
                                    .copied()
                                    .find(|&at| std::ptr::eq(objects[at].element, slider))
                            });
                            nodes[index].smooth.push(Smooth {
                                vertical,
                                bar: at,
                                slider,
                                state: widgets::ContainerBar {
                                    shown: fact.shown,
                                    range: fact.range,
                                    value: fact.value,
                                    track_frame: fact.track_frame,
                                },
                                enabled: container.smooth_scrolling,
                                factor: fact.bar.smooth_scrolling,
                                current: 0,
                                notified: None,
                            });
                        }
                    }
                }
                Fields::Scrollbar(bar) => {
                    let parts = built_by(index, index);
                    let Ok(plain) = walk.plain(element, bar) else {
                        continue;
                    };
                    let places = walk.icon_places(&plain).unwrap_or([None; 2]);
                    let drawables = scrollbar(&parts, places);
                    // Its constructor shows the range icons it places and hides the other four
                    // parts at its end; a `windowType`'s construction shows and hides the whole
                    // bar in that order.
                    let in_window = objects[index]
                        .parent
                        .is_some_and(|up| objects[up].element.kind == Kind::Window);
                    if let Drawables::Scrollbar { icons, .. } = &drawables
                        && !in_window
                    {
                        for &(icon, placed) in icons {
                            if placed {
                                nodes[icon].shown.show();
                            }
                        }
                    }
                    nodes[index].drawables = drawables;
                }
                Fields::TextBox(text_box) => {
                    let Some(bar) = built_by(index, index).first().copied() else {
                        continue;
                    };
                    let Ok(Some((plain, flag, scroll))) = walk.text_bar(element, text_box) else {
                        continue;
                    };
                    let places = walk.icon_places(&plain).unwrap_or([None; 2]);
                    nodes[bar].drawables = scrollbar(&built_by(index, bar), places);
                    nodes[index].drawables = Drawables::TextBox {
                        bar,
                        flag,
                        state: plain.state(),
                        scroll,
                        written: None,
                    };
                }
                Fields::ListBox(list) => {
                    let Some(bar) = built_by(index, index).first().copied() else {
                        continue;
                    };
                    let smooth = element.kind == Kind::SmoothListBox;
                    let places = match walk.list_scrollbar(element, list, smooth) {
                        Ok(Some(plain)) => walk.icon_places(&plain).unwrap_or([None; 2]),
                        _ => [None; 2],
                    };
                    nodes[bar].drawables = scrollbar(&built_by(index, bar), places);
                    nodes[index].drawables = Drawables::ListBox { bar };
                }
                Fields::ExtendedScrollbar(_) => {
                    nodes[index].drawables = Drawables::All(
                        element
                            .nested()
                            .into_iter()
                            .filter(|part| {
                                !element.children.iter().any(|own| std::ptr::eq(own, *part))
                            })
                            .filter_map(|part| node_of(part, 0..authored))
                            .collect(),
                    );
                }
                Fields::DropDown(drop) => {
                    nodes[index].drawables = Drawables::DropDown {
                        expand: drop
                            .expand_button
                            .as_deref()
                            .and_then(|button| node_of(button, 0..authored)),
                        expanded: drop
                            .expanded_window
                            .as_deref()
                            .and_then(|window| node_of(window, 0..authored)),
                        header: node_of(&drop.inner, 0..authored),
                    };
                }
                Fields::Window(_) => {
                    for at in built_by(index, index) {
                        nodes[index].owned.push((at, true));
                    }
                }
                _ => {}
            }
        }
        // Build and register a drop-down's expanded window and header. They are outside the
        // parent's ordinary child list, so inherited opacity does not reach them, but their
        // transitions tick with every registered window.
        let built: Vec<usize> = {
            let mut nested: Vec<Vec<usize>> = vec![Vec::new(); objects.len()];
            for (index, object) in objects.iter().enumerate() {
                if let Some(up) = object.parent {
                    nested[up].push(index);
                }
            }
            fn post(nested: &[Vec<usize>], nodes: &[Node], index: usize, out: &mut Vec<usize>) {
                for &under in &nested[index] {
                    post(nested, nodes, under, out);
                }
                if nodes[index].window.is_some() {
                    out.push(index);
                }
            }
            let mut order = Vec::new();
            post(&nested, &nodes, 0, &mut order);
            order
        };
        // Registration: each window with a configured transition once built.
        let registered = built
            .iter()
            .copied()
            .filter(|&index| {
                objects[index].element.container().is_some_and(|container| {
                    container.fade != Animation::None
                        || container.show_animation != Animation::None
                        || container.hide_animation != Animation::None
                })
            })
            .collect();
        let mut instance = Self {
            template: root.path.clone(),
            count: authored,
            viewport,
            nodes,
            registered,
            registration: Registration::next(),
            phase: Phase::Drawn,
            time: 0.0,
            dt: 0.0,
            ticked: false,
            kept: false,
            held: Vec::new(),
        };
        // Initialize each fading window and its children to zero opacity as it is built.
        for index in built {
            if objects[index]
                .element
                .container()
                .is_some_and(|container| container.fade != Animation::None)
            {
                instance.write_opacity(index, 0.0, true);
            }
        }
        Ok(instance)
    }

    fn check(&self, root: &Element) -> Result<(), String> {
        if root.path != self.template || elements(root).len() != self.count {
            return Err(format!(
                "the lifecycle of {} was asked about {}",
                self.template, root.path
            ));
        }
        Ok(())
    }

    /// The objects of `root`'s construction ([`graph`]) with `resources`, which must be the ones
    /// the instance was built with.
    fn objects<'a>(
        &self,
        root: &'a Element,
        resources: &'a dyn Resources,
        empty: &'a Bindings,
    ) -> Result<Vec<Object<'a>>, String> {
        self.check(root)?;
        let objects = graph(root, resources, empty);
        let same = objects.len() == self.nodes.len()
            && objects
                .iter()
                .zip(&self.nodes)
                .all(|(object, node)| object.built == node.built && object.parent == node.parent);
        if !same {
            return Err(format!(
                "the lifecycle of {} was built against other templates: the objects its \
                 construction builds from the registry differ",
                self.template
            ));
        }
        // A text box whose scrollbar refreshes on each write takes its texts through
        // `set_text`, in order with the code's other calls.
        for (node, object) in self.nodes.iter().zip(&objects) {
            if let Drawables::TextBox { written, .. } = &node.drawables
                && node.control.text != *written
            {
                return Err(format!(
                    "{} was given a text through Instance::control: its scrollbar refreshes on \
                     every write, in order with the code's other calls, so its code writes it \
                     through Instance::set_text or Call::SetText",
                    object.element.path
                ));
            }
        }
        Ok(objects)
    }

    /// `element`'s index in `root`'s pre-order.
    fn index(&self, root: &Element, element: &Element) -> Result<usize, String> {
        self.check(root)?;
        elements(root)
            .iter()
            .position(|(own, ..)| std::ptr::eq(*own, element))
            .ok_or_else(|| format!("{} is not an element of {}", element.path, root.path))
    }

    /// The GUI viewport the instance was built against.
    pub fn viewport(&self) -> [i32; 2] {
        self.viewport
    }

    /// Whether the root window draws, set and cleared with its visible bit.
    pub fn drawn(&self) -> bool {
        self.nodes[0].shown.visible
    }

    /// The root window's opacity.
    pub fn opacity(&self) -> f32 {
        self.nodes[0].window.map_or(1.0, |window| window.opacity)
    }

    /// Where the root window's top-left draws: the last assigned position, or its
    /// constructed place.
    pub fn position(&self) -> [i32; 2] {
        self.nodes[0]
            .window
            .map_or([0; 2], |window| window.position)
    }

    /// The grids and lists the code's Show or Hide reached since the last call, in order.
    pub fn take_held(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.held)
    }

    /// Sets the window's top-left to `point` and stores the point in its position coordinates.
    /// Percentage axes round half away from zero after conversion against the parent extent.
    /// Evaluation with bounds supplies the movement's starting point.
    pub fn set_position(
        &mut self,
        root: &Element,
        element: &Element,
        point: [i32; 2],
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        if self.nodes[index].window.is_none() {
            return Err(format!("{} is not a containerWindowType", element.path));
        }
        self.window_position(element, index, point);
        Ok(())
    }

    fn window_position(&mut self, element: &Element, index: usize, point: [i32; 2]) {
        let Some(window) = self.nodes[index].window.as_mut() else {
            return;
        };
        window.position = point;
        if let Some(container) = element.container() {
            let mut position = container.position;
            position_pixels(&mut position, point, window.parent);
            window.current =
                evaluate_position(&position, Some(window.parent), &element.path).unwrap_or(point);
        }
    }

    /// Updates extents, backgrounds and scrolls without relaying out child geometry.
    pub fn set_size(
        &mut self,
        root: &Element,
        element: &Element,
        pixels: [i32; 2],
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        let missing = |why| vec![Refusal::new(&element.path, RefusalKind::MissingInput, why)];
        let index = self.index(root, element).map_err(missing)?;
        let container = element
            .container()
            .ok_or_else(|| missing("is not a containerWindowType".into()))?;
        let mut window = self.nodes[index]
            .window
            .ok_or_else(|| missing("has no constructed frame".into()))?;
        let shown = evaluate_position(&window.show_position, Some(window.parent), &element.path)
            .map_err(missing)?;
        for axis in 0..2 {
            let coordinate = &mut window.size.size[axis];
            let pixels = if coordinate.from_position {
                pixels[axis].wrapping_add(shown[axis])
            } else {
                pixels[axis]
            };
            coordinate_pixels(coordinate, pixels, window.parent[axis]);
        }
        let size = evaluate_size(&window.size, Some(window.parent), shown, &element.path)
            .map_err(missing)?;
        let frame =
            frame_from_extent(element, container, Some(window.parent), size).map_err(missing)?;
        // Flag zero leaves children's geometry at its constructed parent reference.
        for child in self.nodes[index].children.clone() {
            self.nodes[child]
                .control
                .layout_parent
                .get_or_insert(window.frame.content_size);
        }
        window.frame = frame;
        let anchored = container.origo != Orientation::UpperLeft;
        if anchored {
            let anchor = compute_anchor(
                container.orientation,
                container.origo,
                window.parent,
                frame.size,
            );
            let show =
                evaluate_position(&container.show_position, Some(window.parent), &element.path)
                    .map_err(missing)?;
            position_pixels(
                &mut window.show_position,
                [
                    show[0].wrapping_add(anchor[0]),
                    show[1].wrapping_add(anchor[1]),
                ],
                window.parent,
            );
        }
        self.nodes[index].window = Some(window);
        if anchored {
            let anchor = compute_anchor(
                container.orientation,
                container.origo,
                window.parent,
                frame.size,
            );
            self.window_position(
                element,
                index,
                [
                    window.anchored_position[0].wrapping_add(anchor[0]),
                    window.anchored_position[1].wrapping_add(anchor[1]),
                ],
            );
        }
        self.synchronize_scroll(root, element, resources, true)
    }

    /// The content wrapper extent last laid out, after native margins.
    pub fn content_size(&self, root: &Element, element: &Element) -> Result<[i32; 2], String> {
        let index = self.index(root, element)?;
        self.nodes[index]
            .window
            .map(|window| window.frame.content_size)
            .ok_or_else(|| format!("{} is not a containerWindowType", element.path))
    }

    /// Shows a window unless its hide guard is set, beginning the fade in. With a show
    /// animation it moves to `show_position`; otherwise it stays where it is and preserves
    /// child visibility. An existing show movement continues. A control uses its own guarded
    /// Show.
    pub fn show(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        let index = self.index(root, element)?;
        let elements = elements(root);
        if self.nodes[index].kind == Kind::Window {
            return self.fixed_show(&elements, index);
        }
        if self.nodes[index].window.is_none() {
            self.show_control(&elements, index);
            return Ok(());
        }
        let shown = self.nodes[index].shown;
        if shown.hide {
            return Ok(());
        }
        let container = element.container().expect("a window node");
        self.begin_show_fade(&elements, index);
        if self.nodes[index].window.expect("a window node").mode == 1 {
            return Ok(());
        }
        if container.show_animation == Animation::None {
            self.show_now(&elements, index);
            return Ok(());
        }
        self.show_now(&elements, index);
        self.start_show_movement(&elements, index)
    }

    /// Shows a window and every admitted child control and window in order, beginning fades
    /// throughout the subtree. A configured show movement starts when none is running. Hide
    /// guards apply to each object. Map icons and list/grid items use this operation.
    pub fn gui_show(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        let index = self.index(root, element)?;
        let elements = elements(root);
        self.gui_show_at(&elements, index)
    }

    fn gui_show_at(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        if self.nodes[index].kind == Kind::Window {
            return self.fixed_show(elements, index);
        }
        if self.nodes[index].window.is_none() {
            self.show_control(elements, index);
            return Ok(());
        }
        let shown = self.nodes[index].shown;
        if shown.hide {
            return Ok(());
        }
        let container = elements[index].0.container().expect("a window node");
        self.begin_show_fade(elements, index);
        self.show_now(elements, index);
        let mut refused = Ok(());
        for child in self.nodes[index].children.clone() {
            if let Err(why) = self.gui_show_at(elements, child) {
                refused = refused.and(Err(why));
            }
        }
        if self.nodes[index].window.expect("a window node").mode != 1
            && container.show_animation != Animation::None
        {
            self.start_show_movement(elements, index)?;
        }
        refused
    }

    /// Hides a window unless a hide movement or fade out is already running. It begins the
    /// fade out; without an active fade or hide animation it immediately hides the window and
    /// all children. Otherwise it moves to `hide_position` and hides the subtree when movement
    /// or fade completes. Controls use their own Hide.
    pub fn hide(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        let index = self.index(root, element)?;
        let elements = elements(root);
        self.hide_at(&elements, index)
    }

    fn hide_at(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        if self.nodes[index].kind == Kind::Window {
            return self.fixed_hide(elements, index);
        }
        let Some(window) = self.nodes[index].window else {
            return self.hide_control(elements, index);
        };
        if window.mode == 2 || window.fade == 2 {
            return Ok(());
        }
        let container = elements[index].0.container().expect("a window node");
        self.begin_hide_fade(elements, index);
        let fading_out = self.nodes[index].window.expect("a window node").fade == 2;
        if !fading_out && container.hide_animation == Animation::None {
            self.hide_now(elements, index);
            return self.hide_children(elements, index);
        }
        if container.hide_animation != Animation::None {
            let window = self.nodes[index].window.as_mut().expect("a window node");
            window.kind = container.hide_animation;
            window.duration = seconds(container.hide_animation_time);
            let target = anchored(
                elements[index].0,
                container,
                &container.hide_position,
                window.parent,
                window.frame.size,
                "hide_position",
            )?;
            self.retarget(elements, index, target, true, false)?;
        }
        Ok(())
    }

    /// Hides a window as [`Self::hide`] does, except that without a hide animation it
    /// immediately hides the window alone, even during fade out. A completed hide movement
    /// also hides only the window.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hide_window(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        let index = self.index(root, element)?;
        let elements = elements(root);
        let Some(window) = self.nodes[index].window else {
            return Err(format!("{} is not a containerWindowType", element.path));
        };
        if window.mode == 2 {
            return Ok(());
        }
        let container = element.container().expect("a window node");
        self.begin_hide_fade(&elements, index);
        if container.hide_animation == Animation::None {
            self.hide_now(&elements, index);
            return Ok(());
        }
        let window = self.nodes[index].window.as_mut().expect("a window node");
        window.kind = container.hide_animation;
        window.duration = seconds(container.hide_animation_time);
        let target = anchored(
            element,
            container,
            &container.hide_position,
            window.parent,
            window.frame.size,
            "hide_position",
        )?;
        self.retarget(&elements, index, target, false, true)
    }

    /// Writes the hide bit, changing whether a later Show is admitted without changing current
    /// visibility.
    pub fn set_hide_flag(
        &mut self,
        root: &Element,
        element: &Element,
        hide: bool,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        self.nodes[index].shown.hide = hide;
        Ok(())
    }

    /// What the code sets on `element` (its text, colour, progress, ...), kept until it writes it
    /// again. Frames go through [`Self::set_frame`]; a sprite written here is the code's next
    /// sprite replacement call, after every call made before it ([`Self::set_sprite`]);
    /// visibility, opacity, positions of windows and every clock are the instance's own and are
    /// not taken from here.
    pub fn control(&mut self, root: &Element, element: &Element) -> Result<&mut Bound, String> {
        let index = self.index(root, element)?;
        self.flush(index);
        Ok(&mut self.nodes[index].control)
    }

    /// Retains finalized grid adjustments for later scroll tests.
    pub fn finalize_grid(
        &mut self,
        root: &Element,
        grid: &Element,
        holder: &Holder,
        items: &[&Element],
    ) -> Result<GridLayout, Refusals> {
        self.finalize_grid_with(root, grid, holder, |parent, viewport, position| {
            super::finalize_grid(grid, parent, viewport, items, position)
        })
    }

    /// Measures caller-resolved item positions and runtime extents.
    pub fn finalize_grid_resolved(
        &mut self,
        root: &Element,
        grid: &Element,
        holder: &Holder,
        items: &[([i32; 2], [i32; 2])],
    ) -> Result<GridLayout, Refusals> {
        self.finalize_grid_with(root, grid, holder, |parent, _, position| {
            super::finalize_grid_resolved(grid, parent, items, position)
        })
    }

    fn finalize_grid_with(
        &mut self,
        root: &Element,
        grid: &Element,
        holder: &Holder,
        finalize: impl FnOnce([i32; 2], [i32; 2], Option<[i32; 2]>) -> Result<GridLayout, Refusal>,
    ) -> Result<GridLayout, Refusals> {
        let missing = |why| vec![Refusal::new(&grid.path, RefusalKind::MissingInput, why)];
        let index = self.index(root, grid).map_err(missing)?;
        if holder.path != grid.path {
            return Err(missing(format!(
                "{} is not the holder of {}",
                holder.path, grid.path
            )));
        }
        let finalized = finalize(
            holder.content,
            self.viewport,
            self.nodes[index].control.position,
        )
        .map_err(|refusal| vec![refusal])?;
        self.nodes[index].control.grid = Some(GridBounds {
            adjust: [0, 1].map(|axis| finalized.relaid[axis].wrapping_sub(finalized.origin[axis])),
            extent: finalized.extent,
        });
        Ok(finalized)
    }

    /// Refreshes scrollbar flags and ranges after the code fills its grids.
    pub fn synchronize_scroll(
        &mut self,
        root: &Element,
        element: &Element,
        resources: &dyn Resources,
        include_hidden: bool,
    ) -> Result<(), Refusals> {
        let missing = |why| vec![Refusal::new(&element.path, RefusalKind::MissingInput, why)];
        let index = self.index(root, element).map_err(missing)?;
        let container = element
            .container()
            .ok_or_else(|| missing("is not a containerWindowType".into()))?;
        let frame = self.nodes[index]
            .window
            .ok_or_else(|| missing("has no constructed frame".into()))?
            .frame;
        let bindings = self.bindings(root, resources).map_err(missing)?;
        let mut walk = Walk::new(resources, &bindings);
        let scroll = walk.content_scroll(element, container);
        let bars = walk
            .bars_for(element, container, &scroll, frame, Some(include_hidden))
            .map_err(|refusal| vec![refusal])?;
        let objects = self.objects(root, resources, &bindings).map_err(missing)?;
        let elements: Vec<_> = objects
            .iter()
            .map(|object| (object.element, object.parent, object.child))
            .collect();
        for fact in bars {
            let Some(at) = self.nodes[index]
                .smooth
                .iter()
                .position(|bar| bar.vertical == fact.vertical)
            else {
                return Err(missing(format!(
                    "has no constructed {} scrollbar",
                    if fact.vertical {
                        "vertical"
                    } else {
                        "horizontal"
                    }
                )));
            };
            let mut bar = self.nodes[index].smooth[at];
            let was_shown = bar.state.shown;
            bar.state.shown = fact.shown;
            bar.enabled = container.smooth_scrolling;
            if fact.shown {
                // Clamp the target without snapping or notifying.
                let previous = widgets::fixed(bar.state.value as f32 / 100_000.0);
                bar.state.range = fact.range;
                bar.state.value = fact.range.clamp(bar.state.value);
                if fact.vertical {
                    // The vertical branch writes the saved value again, with a notification.
                    bar.set_target(fact.bar, previous.min(fact.range.max), true);
                }
            } else if was_shown {
                // A hidden bar resets only its content offset; its range and target remain.
                bar.notified = Some(0);
            }
            self.nodes[index].smooth[at] = bar;
            if let Some((_, flag)) = self.nodes[index]
                .owned
                .iter_mut()
                .find(|(owned, _)| *owned == bar.bar)
            {
                *flag = fact.shown;
            }
            if fact.shown && self.nodes[index].shown.visible {
                self.gui_show_at(&elements, bar.bar).map_err(missing)?;
            } else if !fact.shown && was_shown {
                self.hide_at(&elements, bar.bar).map_err(missing)?;
            }
        }
        Ok(())
    }

    /// The kept container scrollbar range, in pixels.
    #[cfg(any(test, feature = "test-support"))]
    pub fn scroll_range(
        &self,
        root: &Element,
        element: &Element,
        vertical: bool,
    ) -> Result<Option<[f32; 2]>, String> {
        let index = self.index(root, element)?;
        Ok(self.nodes[index]
            .smooth
            .iter()
            .find(|bar| bar.vertical == vertical)
            .map(|bar| {
                [
                    bar.state.range.min as f32 / 100_000.0,
                    bar.state.range.max as f32 / 100_000.0,
                ]
            }))
    }

    /// Snaps the target and notifies with the current scrollbar value.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_scroll_target(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        value: f32,
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        self.with_scrollbar(root, element, vertical, resources, |bar, template| {
            bar.set_target(template, widgets::fixed(value), true);
        })
    }

    /// One increase/decrease step from the current scrollbar value.
    pub fn step_scroll(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        direction: i32,
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        if !matches!(direction, -1 | 1) {
            return Err(vec![Refusal::new(
                &element.path,
                RefusalKind::MissingInput,
                format!("scroll step direction {direction} is not -1 or 1"),
            )]);
        }
        self.with_scrollbar(root, element, vertical, resources, |bar, template| {
            let value = if bar.enabled {
                bar.current
            } else {
                bar.state.value
            };
            let rounded = widgets::fixed(value as f32 / 100_000.0);
            let stepped = if direction > 0 {
                rounded.wrapping_add(bar.state.range.step)
            } else {
                rounded.wrapping_sub(bar.state.range.step)
            };
            bar.set_target(template, stepped, true);
        })
    }

    /// Reads the target, snaps without notifying, then moves content immediately.
    pub fn wheel_scroll(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        delta: i32,
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        let container = element.container().ok_or_else(|| {
            vec![Refusal::new(
                &element.path,
                RefusalKind::MissingInput,
                "is not a containerWindowType",
            )]
        })?;
        self.with_scrollbar(root, element, vertical, resources, |bar, template| {
            if !bar.state.shown {
                return;
            }
            let target = bar.state.value as f32 / 100_000.0;
            let pixels =
                truncate_f32(delta as f32 * container.scroll_wheel_factor + (target + 0.5));
            let clamped = bar.state.range.clamp(i64::from(pixels) * 100_000);
            bar.set_target(template, clamped, false);
            bar.notified = Some((clamped / 100_000) as i32);
        })
    }

    fn with_scrollbar(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        resources: &dyn Resources,
        update: impl FnOnce(&mut Smooth, &native::ExtendedScrollbar),
    ) -> Result<(), Refusals> {
        let missing = |why| vec![Refusal::new(&element.path, RefusalKind::MissingInput, why)];
        let index = self.index(root, element).map_err(missing)?;
        let Some(at) = self.nodes[index]
            .smooth
            .iter()
            .position(|bar| bar.vertical == vertical)
        else {
            return Err(missing(format!(
                "has no constructed {} scrollbar",
                if vertical { "vertical" } else { "horizontal" }
            )));
        };
        let bindings = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let objects = self.objects(root, resources, &bindings).map_err(missing)?;
        let bar = &mut self.nodes[index].smooth[at];
        let Fields::ExtendedScrollbar(template) = &objects[bar.bar].element.fields else {
            return Err(missing("its constructed scrollbar has another type".into()));
        };
        update(bar, template);
        Ok(())
    }

    /// A sprite written through [`Self::control`] as the code's next setter call.
    fn flush(&mut self, index: usize) {
        let node = &mut self.nodes[index];
        if let Some(name) = node.control.sprite.take() {
            node.events.push(Event::Sprite(name, node.control.setter));
        }
    }

    /// Replaces the drawable using the sprite lookup and setter after all earlier calls.
    /// Unknown or unchanged names preserve it ([`setter_replaces`]); a replacement starts
    /// fresh clocks and uses the setter-selected frame. Drawing, extents
    /// ([`Self::drawable_extent`]) and clocks all use the resulting drawable.
    pub fn set_sprite(
        &mut self,
        root: &Element,
        element: &Element,
        name: &[u8],
        setter: SpriteSetter,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index]
            .events
            .push(Event::Sprite(name.to_vec(), setter));
        Ok(())
    }

    /// Assigns the frame of `element`'s current drawable. A frame-animated sprite's updater
    /// overwrites it only while playing; a later replacement of the drawable does not carry
    /// the assigned frame over.
    pub fn set_frame(
        &mut self,
        root: &Element,
        element: &Element,
        frame: i32,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index].control.progress_float = None;
        self.nodes[index].events.push(Event::Frame(frame));
        Ok(())
    }

    /// Stores the normalized progress setter's rounded state, without the
    /// integer setter's additional upper clamp. Sprite replacements and later
    /// integer assignments supersede it in native call order.
    pub fn set_progress_float_value(
        &mut self,
        root: &Element,
        element: &Element,
        value: i32,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index].events.push(Event::ProgressFloat(value));
        Ok(())
    }

    /// Writes `text` to text box `element` ([`Bound::text`]). A scrollbar refreshes
    /// immediately in call order ([`Walk::text_refresh`]): its new range clamps the value,
    /// places the slider, and updates flags and visibility using its own Show/Hide. If the
    /// refresh is refused, only the text changes and drawing carries the same refusal.
    /// `resources` are those used at construction.
    pub fn set_text(
        &mut self,
        root: &Element,
        element: &Element,
        text: &str,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        self.nodes[index].control.text = Some(text.to_owned());
        let (
            Drawables::TextBox {
                bar,
                flag,
                state,
                scroll,
                ..
            },
            Fields::TextBox(text_box),
        ) = (self.nodes[index].drawables.clone(), &element.fields)
        else {
            return Ok(());
        };
        let mut bindings = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        *bindings.bind(element) = self.nodes[index].control.clone();
        let walk = Walk::new(resources, &bindings);
        let mut kept = (state, flag, Refreshed::Unchanged);
        if let Ok(Some((mut plain, _, _))) = walk.text_bar(element, text_box) {
            plain.restore(state);
            let mut flag = flag;
            if let Ok(refreshed) =
                walk.text_refresh(element, text_box, &mut plain, &mut flag, Some(text))
            {
                kept = (plain.state(), flag, refreshed);
            }
        }
        let (state, flag, refreshed) = kept;
        self.nodes[index].drawables = Drawables::TextBox {
            bar,
            flag,
            state,
            scroll,
            written: Some(text.to_owned()),
        };
        let elements = elements(root);
        match refreshed {
            Refreshed::Above { flagged } if flagged || self.nodes[bar].shown.visible => {
                self.gui_show_at(&elements, bar)
            }
            Refreshed::AtMost => self.hide_at(&elements, bar),
            _ => Ok(()),
        }
    }

    /// Result-detail range override after a text write: span / newline count, in fixed point.
    /// Changing the range clamps the slider without notifying the text object.
    pub fn set_text_scroll_lines(
        &mut self,
        root: &Element,
        element: &Element,
        lines: usize,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        if lines == 0 {
            return Ok(());
        }
        self.with_text_scrollbar(root, element, resources, |plain, _| {
            let range = plain.range;
            let span = range.max.wrapping_sub(range.min);
            let step = if [i64::MAX, i64::MIN, i64::MIN + 1].contains(&span) {
                0xffff_ffff
            } else {
                span.wrapping_mul(100_000)
                    .wrapping_div((lines as i64).wrapping_mul(100_000))
            };
            plain.set_range(widgets::Range::new(range.min, range.max, step));
        })
    }

    /// Plain-text bar input and its instant text box observer. Direction is the viewer's
    /// already signed down/up delta.
    pub fn scroll_text(
        &mut self,
        root: &Element,
        element: &Element,
        direction: i32,
        wheel: bool,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        self.with_text_scrollbar(root, element, resources, |plain, scroll| {
            plain.input(direction, wheel);
            let high = plain.range.max as f32 / 100_000.0;
            let value = plain.value.max(0.0).min(high);
            plain.set_value(widgets::fixed(value));
            *scroll = -value;
        })
    }

    fn with_text_scrollbar(
        &mut self,
        root: &Element,
        element: &Element,
        resources: &dyn Resources,
        update: impl FnOnce(&mut widgets::Plain<'_>, &mut f32),
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        let (
            Drawables::TextBox {
                bar,
                flag,
                state,
                mut scroll,
                written,
            },
            Fields::TextBox(text_box),
        ) = (self.nodes[index].drawables.clone(), &element.fields)
        else {
            return Err("has no constructed text scrollbar".into());
        };
        let bindings = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let walk = Walk::new(resources, &bindings);
        let Some((mut plain, _, _)) = walk.text_bar(element, text_box).map_err(|why| why.detail)?
        else {
            return Err("has no text scrollbar template".into());
        };
        plain.restore(state);
        update(&mut plain, &mut scroll);
        self.nodes[index].drawables = Drawables::TextBox {
            bar,
            flag,
            state: plain.state(),
            scroll,
            written,
        };
        Ok(())
    }

    /// The drawable the control `index` shows after the code's calls since the last tick,
    /// resolved in order against `resources`.
    fn retained(&self, index: usize, element: &Element, resources: &dyn Resources) -> Retained {
        let node = &self.nodes[index];
        let mut kept = Retained {
            made: node.made.clone(),
            count: node.made_count,
            frame: node.control.frame,
            progress_float: node.control.progress_float,
            framed: false,
            played: false,
        };
        let authored = own_sprite(element).map_or(&[][..], |(name, _)| name);
        let written = node
            .control
            .sprite
            .clone()
            .map(|name| Event::Sprite(name, node.control.setter));
        for event in node.events.iter().chain(written.as_ref()) {
            match event {
                Event::Sprite(name, setter) => {
                    let current = kept
                        .made
                        .as_ref()
                        .map_or(authored, |made| made.sprite.as_slice());
                    if setter_replaces(current, name, resources) {
                        kept.count += 1;
                        kept.made = Some(Made {
                            sprite: name.clone(),
                            setter: *setter,
                            made: kept.count,
                        });
                        kept.frame = None;
                        kept.progress_float = None;
                        kept.framed = false;
                        // The new drawable starts stopped. A `Play` before replacement reached
                        // the previous drawable.
                        kept.played = false;
                    }
                }
                Event::ProgressFloat(value) => {
                    kept.progress_float = Some(*value);
                }
                Event::Frame(frame) => {
                    kept.progress_float = None;
                    kept.frame = Some(*frame);
                    kept.framed = true;
                }
                #[cfg(any(test, feature = "test-support"))]
                Event::Play => kept.played = true,
            }
        }
        kept
    }

    /// Sets opacity on the window, its controls and backgrounds. With `children`, it also
    /// reaches descendant windows. Appending a grid item synchronizes its opacity with the
    /// grid and propagates it through the item subtree.
    pub fn set_opacity(
        &mut self,
        root: &Element,
        element: &Element,
        value: f32,
        children: bool,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        if self.nodes[index].window.is_none() {
            return Err(format!("{} is not a containerWindowType", element.path));
        }
        self.write_opacity(index, value, children);
        Ok(())
    }

    /// Starts frame playback on the drawable left by earlier calls. A later replacement starts
    /// stopped; a drawable without frame animation remains unchanged.
    #[cfg(any(test, feature = "test-support"))]
    pub fn play(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index].events.push(Event::Play);
        Ok(())
    }

    /// Sets the visible bit and shows each owned object whose flag is set ([`Node::owned`]):
    /// Its scrollbars or fixed-window background.
    fn show_now(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        self.nodes[index].shown.visible = true;
        for (owned, flag) in self.nodes[index].owned.clone() {
            if flag {
                // A part's own Show never fails: only a window's frame evaluation can.
                let _ = self.gui_show_at(elements, owned);
            }
        }
    }

    /// Clears the visible bit and hides each owned object whose flag is set.
    fn hide_now(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        self.nodes[index].shown.visible = false;
        for (owned, flag) in self.nodes[index].owned.clone() {
            if flag {
                let _ = self.hide_at(elements, owned);
            }
        }
    }

    /// A control's Show, and a grid's or list's items with it.
    fn show_control(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        if !self.nodes[index].shown.show() {
            return;
        }
        if matches!(self.nodes[index].kind, Kind::Grid | Kind::List) {
            self.held.push(Held {
                path: elements[index].0.path.clone(),
                shown: true,
            });
        }
        self.show_drawables(elements, index);
    }

    /// Shows the related objects in [`Drawables`], each by its own Show and hide guard.
    fn show_drawables(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        for (part, shown) in self.drawables(index, true) {
            // A part's own Show never fails: only a window's frame evaluation can, and a
            // drop-down's header was evaluated when the instance was built.
            let _ = if shown {
                self.gui_show_at(elements, part)
            } else {
                self.hide_at(elements, part)
            };
        }
    }

    /// Hides the related objects in [`Drawables`], including a plain scrollbar's range icons
    /// and parts ([`Shown::hidden_by_code`]).
    fn hide_drawables(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        let mut refused = Ok(());
        for (part, _) in self.drawables(index, false) {
            if let Err(why) = self.hide_at(elements, part) {
                refused = refused.and(Err(why));
            }
        }
        refused
    }

    /// The related objects reached when showing or hiding `index`, each with whether it
    /// is shown.
    fn drawables(&self, index: usize, show: bool) -> Vec<(usize, bool)> {
        match &self.nodes[index].drawables {
            Drawables::None => Vec::new(),
            Drawables::Scrollbar { parts, icons } => parts
                .iter()
                .map(|&part| (part, show))
                .chain(icons.iter().map(|&(icon, placed)| (icon, show && placed)))
                .collect(),
            Drawables::All(parts) => parts.iter().map(|&part| (part, show)).collect(),
            Drawables::DropDown {
                expand,
                expanded,
                header,
            } => {
                let mut parts: Vec<(usize, bool)> = Vec::new();
                parts.extend(expand.map(|expand| (expand, show)));
                if !show {
                    parts.extend(expanded.map(|expanded| (expanded, false)));
                }
                if let Some(header) = *header
                    && (!show || !self.nodes[header].shown.visible)
                {
                    parts.push((header, show));
                }
                parts
            }
            Drawables::TextBox { bar, flag, .. } => {
                if !show || !*flag {
                    vec![(*bar, show)]
                } else {
                    Vec::new()
                }
            }
            Drawables::ListBox { bar } => {
                if show {
                    Vec::new()
                } else {
                    vec![(*bar, false)]
                }
            }
        }
    }

    /// A control's Hide, and a grid's or list's items with it.
    fn hide_control(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        self.nodes[index].shown.hide();
        if matches!(self.nodes[index].kind, Kind::Grid | Kind::List) {
            self.held.push(Held {
                path: elements[index].0.path.clone(),
                shown: false,
            });
        }
        self.hide_drawables(elements, index)
    }

    /// Shows a fixed window unless its hide guard is set. It shows the background using its
    /// own hide guard, then every attached child in attach order. Container fades and
    /// Show/hide movements do not apply; explicit slide or opacity calls are unused by map
    /// icons.
    fn fixed_show(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        if self.nodes[index].shown.hide {
            return Ok(());
        }
        self.show_now(elements, index);
        let mut refused = Ok(());
        for child in self.nodes[index].children.clone() {
            if let Err(why) = self.gui_show_at(elements, child) {
                refused = refused.and(Err(why));
            }
        }
        refused
    }

    /// Hides a fixed window, its background and every attached child using their own Hide.
    fn fixed_hide(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        self.hide_now(elements, index);
        self.hide_children(elements, index)
    }

    /// Hides each child control and window using their own Hide.
    fn hide_children(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        let mut refused = Ok(());
        for child in self.nodes[index].children.clone() {
            if let Err(why) = self.hide_at(elements, child) {
                refused = refused.and(Err(why));
            }
        }
        refused
    }

    /// Sets the opacity shared by the window, controls and backgrounds, and optionally every
    /// descendant window.
    fn write_opacity(&mut self, index: usize, opacity: f32, descendants: bool) {
        if let Some(window) = self.nodes[index].window.as_mut() {
            window.opacity = opacity;
        }
        if descendants {
            for child in self.nodes[index].windows.clone() {
                self.write_opacity(child, opacity, true);
            }
        }
    }

    /// Begins a configured show fade for a window that is hidden or not already fading in. It
    /// and its descendants start transparent; child windows then begin their own show fades.
    fn begin_show_fade(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        let fades = elements[index]
            .0
            .container()
            .is_some_and(|container| container.fade != Animation::None);
        let visible = self.nodes[index].shown.visible;
        if let Some(window) = self.nodes[index].window
            && fades
            && (!visible || window.fade != 1)
        {
            self.nodes[index]
                .window
                .as_mut()
                .expect("a window node")
                .fade = 1;
            self.write_opacity(index, 0.0, true);
        }
        for child in self.nodes[index].windows.clone() {
            self.begin_show_fade(elements, child);
        }
    }

    /// Begins a configured hide fade on a visible window with opacity. A window without its
    /// own fade propagates the request to child windows.
    fn begin_hide_fade(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        let fades = elements[index]
            .0
            .container()
            .is_some_and(|container| container.fade != Animation::None);
        let visible = self.nodes[index].shown.visible;
        if let Some(window) = self.nodes[index].window.as_mut()
            && fades
            && window.opacity > 0.0
            && visible
        {
            window.fade = 2;
            return;
        }
        for child in self.nodes[index].windows.clone() {
            self.begin_hide_fade(elements, child);
        }
    }

    /// The show movement `show` and `gui_show` start: the kind and time, then a retarget to the
    /// anchored `show_position`.
    fn start_show_movement(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        let element = elements[index].0;
        let container = element.container().expect("a window node");
        let window = self.nodes[index].window.as_mut().expect("a window node");
        window.kind = container.show_animation;
        window.duration = seconds(container.show_animation_time);
        let target = anchored(
            element,
            container,
            &container.show_position,
            window.parent,
            window.frame.size,
            "show_position",
        )?;
        self.retarget(elements, index, target, false, false)
    }

    /// Retargets a movement. A nonmoveable window first snaps to its previous target. If
    /// already at the new target, movement completes and hiding takes effect immediately,
    /// including children when requested; otherwise movement runs.
    fn retarget(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
        target: [i32; 2],
        children: bool,
        only: bool,
    ) -> Result<(), String> {
        let element = elements[index].0;
        let container = element.container().expect("a window node");
        if !container.moveable {
            let old = self.nodes[index].window.expect("a window node").target;
            self.window_position(element, index, old);
        }
        let visible = self.nodes[index].shown.visible;
        let window = self.nodes[index].window.as_mut().expect("a window node");
        window.hide_children = children;
        window.hide_only = only;
        window.target = target;
        window.elapsed = 0.0;
        if window.current == target {
            window.done = true;
            window.mode = 0;
            if visible && children {
                self.hide_now(elements, index);
                return self.hide_children(elements, index);
            }
            if visible && only {
                self.hide_now(elements, index);
            }
        } else {
            window.done = false;
            window.mode = if children || only { 2 } else { 1 };
            window.delta = [0, 1].map(|axis| (target[axis] - window.current[axis]) as f32);
        }
        Ok(())
    }

    /// Advances a configured fade with positive duration. At endpoint opacity after the delay,
    /// elapsed time restarts with at least a millisecond. Authored easing and the frame step
    /// update clamped opacity recursively. Past the delay, reaching the end stops the fade; a
    /// fade out that reaches zero hides the window and every child.
    fn update_fade(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
        dt: f32,
    ) -> Result<(), String> {
        let container = elements[index].0.container().expect("a window node");
        let window = self.nodes[index].window.as_mut().expect("a window node");
        if window.fade == 0 || container.fade == Animation::None || container.fade_time <= 0 {
            return Ok(());
        }
        let dt = dt.max(0.001);
        let delay = seconds(container.fade_delay);
        let time = seconds(container.fade_time);
        if (window.opacity == 0.0 || window.opacity == 1.0) && window.fade_elapsed > delay {
            window.fade_elapsed = dt;
        } else {
            window.fade_elapsed += dt;
        }
        let progress = (window.fade_elapsed - delay).max(0.0) / time;
        let step = match container.fade {
            Animation::Accelerated => 5.0 * progress,
            Animation::Decelerated => 4.0 * progress + 1.0,
            Animation::Smoothstep | Animation::Linear => progress,
            Animation::None => 0.0,
        };
        let opacity = if window.fade == 1 {
            (window.opacity + step).min(1.0)
        } else {
            (window.opacity - step).max(0.0)
        };
        let past = window.fade_elapsed > delay;
        self.write_opacity(index, opacity, true);
        if past {
            let window = self.nodes[index].window.as_mut().expect("a window node");
            if opacity == 1.0 {
                window.fade = 0;
            } else if opacity == 0.0 {
                window.fade = 0;
                self.hide_now(elements, index);
                return self.hide_children(elements, index);
            }
        }
        Ok(())
    }

    /// Moves to the target after the duration or when easing is absent. Otherwise authored
    /// easing determines the remaining displacement, truncated per axis. A hide movement takes
    /// Effect on the tick after arrival, with children for Hide and alone for HideWindow.
    fn update_movement(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
        dt: f32,
    ) -> Result<(), String> {
        let element = elements[index].0;
        let visible = self.nodes[index].shown.visible;
        let window = self.nodes[index].window.as_mut().expect("a window node");
        if window.done {
            let (mode, children, only) = (window.mode, window.hide_children, window.hide_only);
            window.mode = 0;
            if mode == 2 && visible {
                if children {
                    self.hide_now(elements, index);
                    return self.hide_children(elements, index);
                }
                if only {
                    self.hide_now(elements, index);
                }
            }
            return Ok(());
        }
        window.elapsed += dt;
        if window.elapsed > window.duration || window.kind == Animation::None {
            window.done = true;
            let target = window.target;
            self.window_position(element, index, target);
            return Ok(());
        }
        let u = window.elapsed / window.duration;
        let smoothstep = |x: f32| {
            let c = x.clamp(0.0, 1.0);
            (3.0 - (c + c)) * (c * c)
        };
        let eased = match window.kind {
            Animation::Accelerated => f64::from(u * 5.0 * 0.2).powf(4.0) as f32,
            Animation::Decelerated => {
                (1.0 - (u * 4.0 + 1.0).powf(-2.0)) * (1.0 / (1.0 - 5.0f32.powf(-2.0)))
            }
            Animation::Smoothstep => smoothstep(smoothstep(u)),
            _ => u,
        };
        let point = [0, 1]
            .map(|axis| window.target[axis] - truncate_f32((1.0 - eased) * window.delta[axis]));
        // Use the same position setter.
        self.window_position(element, index, point);
        Ok(())
    }

    /// Whether a drawable's own shown flag is set: a control's and a part's follow
    /// their own visible bit (their Show and Hide show and hide their drawables); a window's
    /// backgrounds follow their holder's. A window's `HIDE` does not reach its children's
    /// drawables.
    fn drawable_shown(&self, index: usize) -> bool {
        let node = &self.nodes[index];
        match (node.child || node.part, node.parent) {
            (false, Some(up)) => self.nodes[up].shown.visible,
            _ => node.shown.visible,
        }
    }

    /// Whether the walk reaches `index`: it and every object above it shown. A plain scrollbar
    /// child its own `hide` keeps from showing still draws the range icons its constructor
    /// showed, until a code Hide hides them ([`Walk::scrollbar_child`]).
    fn reached(&self, index: usize) -> bool {
        let node = &self.nodes[index];
        let own = if node.child || node.part {
            node.shown.visible
        } else {
            true
        };
        let Some(up) = node.parent else {
            return own;
        };
        let holder = &self.nodes[up];
        let range_icon = matches!(node.built, Some((owner, Role::Part(4 | 5))) if owner == up)
            && holder.kind == Kind::Scrollbar
            && holder.built.is_none();
        if range_icon {
            let drawn = holder.shown.visible || (holder.hide && !holder.shown.hidden_by_code);
            return own && drawn && holder.parent.is_none_or(|up| self.reached(up));
        }
        own && self.reached(up)
    }

    /// The state a drawable uses without a pointer:
    /// A disabled button 4, a checked check box 2 else 1.
    fn state(&self, element: &Element, index: usize) -> u32 {
        let node = &self.nodes[index];
        let control = &node.control;
        match (&element.kind, element.button()) {
            (Kind::Checkbox, _) if !node.stateful => {
                if control.checked == Some(true) {
                    2
                } else {
                    1
                }
            }
            (_, Some(button)) if node.stateful => {
                if control.enabled.unwrap_or(button.enabled) {
                    1
                } else {
                    4
                }
            }
            _ => 1,
        }
    }

    /// One presented frame of this instance alone, as the game runs it: [`Self::tick_clocks`],
    /// [`Self::tick_transitions`] and [`Self::tick_draws`] in turn. A caller that ticks several
    /// instances whose Shows and Hides reach one another (a list's items, a grid's) runs each
    /// phase across all of them before the next, as the game does ([`Phase`]).
    pub fn tick(
        &mut self,
        root: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        self.tick_clocks(root, resources, tick)?;
        self.tick_transitions(root)?;
        self.tick_draws(root, resources)
    }

    /// Where the instance's windows stand in the GUI lifecycle's transition list ([`Registration`]).
    pub fn registration(&self) -> Registration {
        self.registration
    }

    /// The frame's first phase applies ordered drawable calls, preserving state on failed
    /// replacement and resetting it on successful replacement ([`Runtime`]). It updates shown
    /// smooth scrollbars, then every shown frame-animation clock and cornered-tile pulse
    /// across all windows before any window transition ([`Self::tick_transitions`]). It starts
    /// a new frame regardless of the preceding phase. `resources` are those used at
    /// construction.
    pub fn tick_clocks(
        &mut self,
        root: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        let empty = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let objects = self.objects(root, resources, &empty)?;
        let Tick { time, dt } = tick;
        self.time = time;
        self.dt = dt;
        // The code's calls on each drawable, in order, and which drawables a `Play` reached.
        let mut played = vec![false; objects.len()];
        for (index, object) in objects.iter().enumerate().take(self.count) {
            let node = &self.nodes[index];
            if node.events.is_empty() && node.control.sprite.is_none() {
                continue;
            }
            let kept = self.retained(index, object.element, resources);
            played[index] = kept.played;
            let node = &mut self.nodes[index];
            node.made = kept.made;
            node.made_count = kept.count;
            node.control.frame = kept.frame;
            node.control.progress_float = kept.progress_float;
            node.control.sprite = None;
            node.events.clear();
            if kept.framed
                && let Some(clock) = node
                    .runtime
                    .as_mut()
                    .and_then(|runtime| runtime.clock.as_mut())
            {
                clock.clock.frame = None;
            }
        }
        // Update each shown smooth scrollbar. A missing slider is a null read refused by the
        // walk.
        for index in 0..self.count {
            for at in 0..self.nodes[index].smooth.len() {
                let smooth = self.nodes[index].smooth[at];
                if !smooth.enabled || !self.nodes[smooth.bar].shown.visible {
                    continue;
                }
                let Some(slider) = smooth.slider.map(|slider| self.nodes[slider].shown.visible)
                else {
                    continue;
                };
                self.nodes[index].smooth[at].update(slider);
            }
        }
        for (index, object) in objects.iter().enumerate() {
            let made = self.nodes[index].made.clone();
            let Some((sprite, info)) = drawn(
                object.element,
                made.as_ref(),
                self.nodes[index].control.text.as_deref(),
                resources,
            ) else {
                self.nodes[index].runtime = None;
                continue;
            };
            let shown = self.drawable_shown(index);
            let made = made.map_or(0, |made| made.made);
            let node = &mut self.nodes[index];
            let runtime = Self::runtime(node, sprite, made);
            if matches!(info.kind, SpriteKind::FrameAnimated { .. }) {
                let clock = runtime.clock.get_or_insert_with(|| Clock {
                    clock: FrameClock::new(),
                    kind: info.kind.clone(),
                });
                clock.kind = info.kind.clone();
                if played[index] {
                    clock.clock.playing = true;
                }
                if shown
                    && runtime.hang.is_none()
                    && let Err(why) = clock.clock.update(&info.kind, info.frames, dt)
                {
                    runtime.hang = Some(why);
                }
            } else {
                runtime.clock = None;
            }
            match info.kind {
                SpriteKind::CorneredTile {
                    looping: true,
                    rate: Some(rate),
                    ..
                } => {
                    let pulse = runtime.pulse.get_or_insert_with(Pulse::default);
                    if shown
                        && runtime.hang.is_none()
                        && let Err(why) = pulse.update(rate, dt)
                    {
                        runtime.hang = Some(why);
                    }
                }
                _ => runtime.pulse = None,
            }
        }
        self.phase = Phase::Clocked;
        Ok(())
    }

    /// The runtime of `node`'s drawable, built from `sprite` by setter call `made`: its own, or,
    /// for a drawable other than the one it kept, a new one as the constructor leaves it.
    fn runtime(node: &mut Node, sprite: Vec<u8>, made: u32) -> &mut Runtime {
        if !node
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is(&sprite, made))
        {
            node.runtime = Some(Runtime::new(sprite, made));
        }
        node.runtime.as_mut().expect("set above")
    }

    /// The frame's second phase follows every instance's clocks ([`Self::tick_clocks`]).
    /// Windows run fade then movement in reverse registration order, visible or hidden. A
    /// completed hide reaches children; reached grids and lists are held ([`Self::take_held`])
    /// so callers hide their item instances before the next window transition
    /// ([`Registration`]).
    pub fn tick_transitions(&mut self, root: &Element) -> Result<(), String> {
        self.check(root)?;
        if self.phase != Phase::Clocked {
            return Err(format!(
                "the transitions of {} were run before its clocks for the frame: run every \
                 drawable clock before window transitions; call Instance::tick_clocks first",
                self.template
            ));
        }
        let elements = elements(root);
        let dt = self.dt;
        for index in self.registered.clone().into_iter().rev() {
            self.update_fade(&elements, index, dt)?;
            self.update_movement(&elements, index, dt)?;
        }
        self.phase = Phase::Moved;
        Ok(())
    }

    /// The frame's last phase, once every instance's transitions have run and their hides have
    /// reached what they hold ([`Self::tick_transitions`]): what the draw of every reached
    /// drawable records at `Time`: its first draw, its state effect's clock, its trigger
    /// records. `resources` are the ones it was built with.
    pub fn tick_draws(&mut self, root: &Element, resources: &dyn Resources) -> Result<(), String> {
        self.check(root)?;
        if self.phase != Phase::Moved {
            return Err(format!(
                "the draw of {} was recorded before its transitions for the frame: call \
                 Instance::tick_transitions first",
                self.template
            ));
        }
        let empty = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let objects = self.objects(root, resources, &empty)?;
        let time = self.time;
        for (index, object) in objects.iter().enumerate() {
            if !self.reached(index) {
                continue;
            }
            let made = self.nodes[index].made.clone();
            let Some((sprite, info)) = drawn(
                object.element,
                made.as_ref(),
                self.nodes[index].control.text.as_deref(),
                resources,
            ) else {
                continue;
            };
            let made = made.map_or(0, |made| made.made);
            let state = self.state(object.element, index);
            let records = if info.animated {
                resources.sprite_animations(&sprite).unwrap_or_default()
            } else {
                Vec::new()
            };
            // How long its state's technique changes on the clock, from the effect itself.
            let settles = match &info.effect {
                Ok(effect) => effect.settles(technique(state)),
                Err(_) => Settles::Program(Some(0.0)),
            };
            let runtime = Self::runtime(&mut self.nodes[index], sprite, made);
            let first = *runtime.first_draw.get_or_insert(time);
            // A marked state change restarts the clock.
            match runtime.effect.as_mut() {
                Some(effect) if effect.state == state => effect.settles = settles,
                _ => {
                    runtime.effect = Some(StateEffect {
                        state,
                        at: time,
                        settles,
                    });
                }
            }
            runtime.triggers.resize(records.len(), (-1.0, 0));
            let t = time - first;
            for (record, (start, last)) in records.iter().zip(&mut runtime.triggers) {
                let trigger = record.data.trigger_frames;
                if trigger == 0 {
                    continue;
                }
                let bit = 1u32.wrapping_shl(state & 31);
                if bit & trigger != 0 && *last != state && !(state == 3 && *last == 2) {
                    *start = t;
                }
                *last = state;
            }
        }
        self.ticked = true;
        self.phase = Phase::Drawn;
        let keeps = objects.iter().enumerate().any(|(index, _)| {
            self.reached(index)
                && self.nodes[index]
                    .runtime
                    .as_ref()
                    .and_then(|runtime| runtime.effect)
                    .is_some_and(|effect| effect.settles == Settles::Previous)
        });
        self.kept = keeps && self.kept_changes(root, resources);
        Ok(())
    }

    /// Whether the instance's draws change on the clock. A missing technique inherits the
    /// preceding draw's program, evaluated with the current part's restart time. A first part
    /// or a part after externally drawn holder items has unknown inherited state and counts as
    /// changing, as does a refused layout. A draw with no clock constants does not change
    /// ([`super::Shading::clocked`]). A retained draw after a refused program is itself
    /// refused and draws nothing ([`Settles::Refused`]).
    fn kept_changes(&self, root: &Element, resources: &dyn Resources) -> bool {
        let layout = match self.layout(root, resources, None) {
            Ok(Some(layout)) => layout,
            Ok(None) => return false,
            Err(_) => return true,
        };
        // The answer of the program the last draw bound; `None` while it is not this
        // Instance's.
        let mut bound: Option<Option<f32>> = None;
        for (index, part) in layout.parts.iter().enumerate() {
            if layout.holders.iter().any(|holder| holder.before == index) {
                bound = None;
            }
            match part.shading.settles() {
                Settles::Program(settles) => bound = Some(settles),
                Settles::Previous if !part.shading.clocked => {}
                Settles::Previous => match bound {
                    Some(Some(settles)) if part.shading.since >= settles => {}
                    _ => return true,
                },
                // Its program record is bound, which the replay lacks: a kept draw after it is
                // refused too and never changes.
                Settles::Refused => bound = Some(Some(0.0)),
            }
        }
        false
    }

    /// Whether a window of the instance is in a transition: it fades, moves, or is due to be
    /// hidden by its ended movement.
    pub fn transitioning(&self) -> bool {
        self.registered.iter().any(|&index| {
            let node = &self.nodes[index];
            let window = node.window.expect("a window node");
            window.fade != 0 || !window.done || (window.mode == 2 && node.shown.visible)
        })
    }

    /// The longest transition duration among all registered windows, including drop-down
    /// headers and expanded windows. A configured fade lasts through its delay and duration; a
    /// configured movement lasts its show or hide animation time. Returns zero without
    /// registered windows.
    pub fn longest_transition(&self, root: &Element) -> Result<f32, String> {
        self.check(root)?;
        let elements = elements(root);
        Ok(self
            .registered
            .iter()
            .filter_map(|&index| elements.get(index)?.0.container())
            .map(|container| {
                let fade = if container.fade != Animation::None && container.fade_time > 0 {
                    seconds(container.fade_delay.max(0)) + seconds(container.fade_time)
                } else {
                    0.0
                };
                let movement = |kind: Animation, time: i32| {
                    if kind == Animation::None {
                        0.0
                    } else {
                        seconds(time.max(0))
                    }
                };
                fade.max(movement(
                    container.show_animation,
                    container.show_animation_time,
                ))
                .max(movement(
                    container.hide_animation,
                    container.hide_animation_time,
                ))
            })
            .fold(0.0, f32::max))
    }

    /// Whether the next frame differs from this one without any call from the code: the first
    /// (`dt = 0`) tick or a call's effect is pending, a window fades or moves or is due to be
    /// hidden by its ended movement, a shown scrollbar still smooths, a shown frame-animated
    /// sprite plays, a shown looping tile pulses, a reached sprite runs animation records, or a
    /// reached drawable's state effect still changes on the clock (for as long as its effect
    /// says, [`Effect::settles`]). A caller keeps presenting frames while this holds.
    pub fn active(&self) -> bool {
        if !self.ticked {
            return true;
        }
        self.transitioning()
            || self.nodes.iter().enumerate().any(|(index, node)| {
                let reached = self.reached(index);
                !node.events.is_empty()
                    || node.control.sprite.is_some()
                    || node.smooth.iter().any(|smooth| {
                        smooth.enabled
                            && smooth.current != smooth.state.value
                            && smooth.slider.is_some()
                            && self.nodes[smooth.bar].shown.visible
                    })
                    || node.runtime.as_ref().is_some_and(|runtime| {
                        let shown = self.drawable_shown(index);
                        // A hanging clock or pulse is the game's hang: nothing follows it.
                        (runtime.hang.is_none()
                            && shown
                            && (runtime
                                .clock
                                .as_ref()
                                .is_some_and(|clock| clock.clock.runs(&clock.kind))
                                || runtime.pulse.is_some()))
                            || (reached && !runtime.triggers.is_empty())
                            || (reached
                                && runtime.first_draw.is_some()
                                && runtime.effect.is_some_and(|effect| match effect.settles {
                                    Settles::Program(settles) => settles
                                        .is_none_or(|settles| self.time - effect.at < settles),
                                    // Resolved in draw order at the tick; a refused draw
                                    // draws nothing.
                                    Settles::Previous | Settles::Refused => false,
                                }))
                    })
            })
            || self.kept
    }

    /// Everything the walk reads of the instance, at each object's [`Site`]: each control's kept
    /// writes and drawable (its pending calls resolved in order, [`Self::set_sprite`]), each
    /// object's visibility, each window's opacity, position and smoothed scroll, each
    /// drawable's clocks (a drawable not yet ticked as its constructor leaves them), the
    /// graphics clock `Time` and the viewport. `resources` are the ones it was built with.
    pub fn bindings(&self, root: &Element, resources: &dyn Resources) -> Result<Bindings, String> {
        let empty = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let objects = self.objects(root, resources, &empty)?;
        let mut bindings = Bindings {
            viewport: Some(self.viewport),
            time: self.time,
            ..Bindings::default()
        };
        for (index, object) in objects.iter().enumerate() {
            let node = &self.nodes[index];
            let element = object.element;
            let kept = if index < self.count {
                self.retained(index, element, resources)
            } else {
                Retained {
                    made: None,
                    count: 0,
                    frame: None,
                    progress_float: None,
                    framed: false,
                    played: false,
                }
            };
            let mut bound = node.control.clone();
            bound.sprite = None;
            bound.drawn = Drawn::Kept(
                kept.made
                    .as_ref()
                    .map(|made| (made.sprite.clone(), made.setter)),
            );
            bound.frame = kept.frame;
            bound.progress_float = kept.progress_float;
            bound.shown = Some(node.shown);
            if let Some(window) = node.window {
                bound.opacity = Some(window.opacity);
                bound.position = Some(window.position);
                bound.window_frame = Some(window.frame);
            }
            for smooth in &node.smooth {
                let axis = usize::from(smooth.vertical);
                bound.scroll[axis] = smooth.notified;
                bound.bars[axis] = Some(smooth.state);
            }
            if let Drawables::TextBox {
                flag,
                state,
                scroll,
                ..
            } = &node.drawables
            {
                bound.text_bar = Some((*state, *flag));
                bound.text_scroll = Some(*scroll);
            }
            let drawable = drawn(
                element,
                kept.made.as_ref(),
                node.control.text.as_deref(),
                resources,
            );
            let made = kept.made.as_ref().map_or(0, |made| made.made);
            // The runtime of the drawable the calls leave: a drawable a pending replacement
            // makes, or one not ticked yet, as its constructor leaves it ([`Runtime`]).
            let runtime = drawable.as_ref().and_then(|(sprite, _)| {
                node.runtime
                    .as_ref()
                    .filter(|runtime| runtime.is(sprite, made))
            });
            let kind = drawable.as_ref().map(|(_, info)| &info.kind);
            bound.clock = match runtime.and_then(|runtime| runtime.clock.as_ref()) {
                Some(clock) => Some((
                    if kept.framed { None } else { clock.clock.frame },
                    clock.clock.elapsed,
                )),
                None => kind
                    .filter(|kind| matches!(kind, SpriteKind::FrameAnimated { .. }))
                    .map(|_| (None, 0.0)),
            };
            let hang = runtime.and_then(|runtime| runtime.hang.clone());
            bound.pulse = match runtime.and_then(|runtime| runtime.pulse) {
                Some(pulse) => hang.is_none().then(|| pulse.alpha()),
                None => kind
                    .filter(|kind| {
                        matches!(
                            kind,
                            SpriteKind::CorneredTile {
                                looping: true,
                                rate: Some(_),
                                ..
                            }
                        )
                    })
                    .map(|_| Pulse::default().alpha()),
            };
            bound.hang = hang;
            // Time since the first draw, using this frame's `Time` when that draw is yet to come.
            bound.animation_time = Some(
                runtime
                    .and_then(|runtime| runtime.first_draw)
                    .map_or(0.0, |first| self.time - first),
            );
            // `Time` less the restart of the state's effect, which a changed state (or a new
            // drawable) restarts at this draw.
            let state = self.state(element, index);
            bound.effect_since = Some(match runtime.and_then(|runtime| runtime.effect) {
                Some(effect) if effect.state == state => self.time - effect.at,
                _ => 0.0,
            });
            if let Some(runtime) = runtime
                && !runtime.triggers.is_empty()
            {
                bound.triggers = Some(runtime.triggers.iter().map(|(start, _)| *start).collect());
            }
            let via = object
                .built
                .map(|(owner, role)| (objects[owner].element as *const Element, role));
            *bindings.bind_at(Site::new(element, via)) = bound;
        }
        Ok(bindings)
    }

    /// Lays the root out as the game draws it: its frame in the viewport (a root the code gives
    /// no parent object), with every runtime input bound; `pointer` for a window that takes
    /// input (a map icon's never does). `Ok(None)` when it draws nothing.
    pub fn layout(
        &self,
        root: &Element,
        resources: &dyn Resources,
        pointer: Option<Pointer>,
    ) -> Result<Option<Layout>, Refusals> {
        self.layout_with_regions(root, resources, pointer, false)
    }

    /// Lay out a window and retain control geometry for hit/hover consumers.
    pub fn layout_with_regions(
        &self,
        root: &Element,
        resources: &dyn Resources,
        pointer: Option<Pointer>,
        regions: bool,
    ) -> Result<Option<Layout>, Refusals> {
        let mut bindings = self
            .bindings(root, resources)
            .map_err(|why| vec![Refusal::new(&root.path, RefusalKind::MissingInput, why)])?;
        bindings.pointer = pointer;
        bindings.regions = regions;
        // A `windowType` root is a fixed window.
        if root.kind == Kind::Window {
            return super::fixed_window_layout(root, resources, &bindings);
        }
        let parent = self.nodes[0]
            .window
            .map_or(self.viewport, |window| window.parent);
        layout(root, Some(parent), resources, &bindings)
    }

    /// The queried extent of `element`'s drawable on this instance: the drawable it shows
    /// after the code's calls ([`Self::set_sprite`]),
    /// the one drawing takes ([`drawable_extent`]).
    pub fn drawable_extent(
        &self,
        root: &Element,
        element: &Element,
        resources: &dyn Resources,
    ) -> Result<Option<[i32; 2]>, Refusals> {
        self.index(root, element)
            .map_err(|why| vec![Refusal::new(&element.path, RefusalKind::MissingInput, why)])?;
        let bindings = self
            .bindings(root, resources)
            .map_err(|why| vec![Refusal::new(&root.path, RefusalKind::MissingInput, why)])?;
        drawable_extent(element, resources, &bindings)
    }
}

/// An authored position anchored using the window's orientation against its parent extent.
fn anchored(
    window: &Element,
    container: &native::Container,
    position: &native::Position,
    parent: [i32; 2],
    size: [i32; 2],
    what: &str,
) -> Result<[i32; 2], String> {
    let at = evaluate_position(position, Some(parent), &format!("{}.{what}", window.path))?;
    let anchor = compute_anchor(container.orientation, container.origo, parent, size);
    Ok([at[0] + anchor[0], at[1] + anchor[1]])
}

/// A map icon type's pool and live list, keyed by owning object. The owner's callback clears
/// its icon reference on erase. All pooled windows are constructed and registered up front,
/// start hidden, and tick each frame whether occupied or free.
///
/// Each frame:
/// 1. Owners request a free slot in round-robin order ([`Pool::request`]) or erase icons whose
///    admission gate fails ([`Pool::erase`]). Exhaustion starts asynchronous pool growth.
/// 2. The manager walks the existing live-list extent ([`Pool::update`]). Empty or
///    off-viewport icons are erased, returning the slot and hiding its window. The pass preserves
///    its traversal order while compacting. Admitted icons receive their screen position and are
///    shown if needed, then return to the caller for type-specific updates.
pub struct Pool<K> {
    template: String,
    viewport: [i32; 2],
    growth: usize,
    slots: Vec<Slot>,
    cursor: usize,
    /// The live list in the manager's order, with each icon's owner.
    live: Vec<(SlotId, K)>,
    /// A growth task was started by an exhausted request.
    growing: bool,
    missed: usize,
    retrying: usize,
    /// Growths completed since the last [`Pool::take_diagnostics`].
    grown: u64,
    growths: u64,
    /// The initialization each icon type applies after a new window is built.
    setup: Setup,
    /// The refusals standing on the slots' windows ([`Pool::refuse`]).
    withheld: Withheld<SlotId>,
}

pub(crate) type Setup = Box<dyn Fn(&mut Instance, &Element) -> Result<(), String> + Send + Sync>;

struct Slot {
    instance: Instance,
    in_use: bool,
}

/// Map icon types and their initial pool sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapIconPool {
    /// Type 5, the air base icon: 0xC0.
    AirBase,
    /// Type 13, the regional air mission icon: 0xC0.
    AirMission,
    /// Type 6, the rocket site icon: 0x80.
    RocketSite,
    /// Type 7, the gun emplacement icon: 0x80.
    GunEmplacement,
    /// Type 17, the radar icon: 0x40.
    Radar,
    /// Type 33, the strategic location icon: 0x40.
    StrategicLocation,
    /// Type 8, the naval base icon: 0xC0.
    Base,
    /// Type 9, the naval combat icon: 0x20.
    Combat,
    /// Type 10, the naval combat results icon: 0x40.
    CombatResults,
    /// Type 12, the naval mission icon: 0x40.
    Mission,
    /// Type 2, victory-point labels: 0xA0.
    Label,
    /// Type 14, capital labels: 0xC0.
    Capital,
    /// Type 29, raid icons: 0x80.
    Raid,
    /// Type 32, faction pins: 0x80.
    Ping,
    /// Type 30, the special facility icon: 0x80.
    Facility,
    /// Type 31, the naval headquarters icon: 0x40.
    Headquarter,
    /// Type 11, the land combat icon: 0x80.
    LandCombat,
    /// Type 16, the adjacency-rule (strait and canal) icon. Its initial size is not
    /// established; 0x20 windows, grown like every pool.
    AdjacencyRule,
    /// Type 3, a state's resource icon: 0xC0.
    Resource,
    /// Type 20, a resistance/compliance state marker: 0x60.
    Occupation,
}

impl MapIconPool {
    /// The windows the pool starts with.
    pub fn initial(self) -> usize {
        match self {
            Self::Label => 0xA0,
            Self::Occupation => 0x60,
            Self::AirBase | Self::AirMission | Self::Base | Self::Capital | Self::Resource => 0xC0,
            Self::Combat | Self::AdjacencyRule => 0x20,
            Self::CombatResults
            | Self::Mission
            | Self::Headquarter
            | Self::Radar
            | Self::StrategicLocation => 0x40,
            Self::Raid
            | Self::Ping
            | Self::Facility
            | Self::LandCombat
            | Self::RocketSite
            | Self::GunEmplacement => 0x80,
        }
    }
}

/// A slot of a [`Pool`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(usize);

/// What the manager's pass finds of a live icon, as the icon type's own virtuals decide it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// The icon is empty: erased before any transform.
    Empty,
    /// The icon does not intersect the viewport: erased.
    Outside,
    /// It intersects the viewport; its window's top-left goes at this screen point.
    Inside([i32; 2]),
}

/// An icon the manager's pass erased: its owner and slot, and the grids and lists the window's
/// Hide reached ([`Held`]), whose items the caller hides.
#[derive(Clone, Debug, PartialEq)]
pub struct Erased<K> {
    pub owner: K,
    pub slot: SlotId,
    pub held: Vec<Held>,
}

/// A live pooled icon returned after its window position and visibility have been updated, for
/// the caller to finish its type-specific update.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass<K> {
    pub updated: Vec<(K, SlotId)>,
    pub erased: Vec<Erased<K>>,
}

impl<K: Clone + Eq> Pool<K> {
    /// The pool of `kind`, its windows built from `template` in the viewport with `resources`
    /// ([`Instance::construct`]) and hidden, each then given `setup` (the icon type's own
    /// constructor steps).
    pub fn new(
        template: &Element,
        viewport: [i32; 2],
        kind: MapIconPool,
        setup: Setup,
        resources: &dyn Resources,
    ) -> Result<Self, String> {
        let mut pool = Self {
            template: template.path.clone(),
            viewport,
            growth: 0x10,
            slots: Vec::new(),
            cursor: 0,
            live: Vec::new(),
            growing: false,
            missed: 0,
            retrying: 0,
            grown: 0,
            growths: 0,
            setup,
            withheld: Withheld::default(),
        };
        pool.add(template, kind.initial(), resources)?;
        Ok(pool)
    }

    fn add(
        &mut self,
        template: &Element,
        count: usize,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        for _ in 0..count {
            let mut instance = Instance::construct(template, self.viewport, resources)?;
            instance.hide(template, template)?;
            (self.setup)(&mut instance, template)?;
            // The construction's Hide reached lists and grids that hold no item yet: nothing.
            instance.take_held();
            self.slots.push(Slot {
                instance,
                in_use: false,
            });
        }
        Ok(())
    }

    /// The slot `owner`'s live icon holds, if it has one.
    pub fn slot_of(&self, owner: &K) -> Option<SlotId> {
        self.live
            .iter()
            .find(|(_, live)| live == owner)
            .map(|(slot, _)| *slot)
    }

    /// Returns the owner's retained icon, or the first free slot from the round-robin cursor
    /// and appends it to the live list. The cursor advances and wraps at pool size. Exhaustion
    /// starts growth ([`Self::tick`] completes it), returns `None`, and lets the owner retry
    /// next frame.
    pub fn request(&mut self, owner: K) -> Option<SlotId> {
        if let Some(slot) = self.slot_of(&owner) {
            return Some(slot);
        }
        let count = self.slots.len();
        let mut at = self.cursor;
        for _ in 0..count {
            if !self.slots[at].in_use {
                self.slots[at].in_use = true;
                self.cursor = if at + 1 >= count { 0 } else { at + 1 };
                self.live.push((SlotId(at), owner));
                return Some(SlotId(at));
            }
            at = if at + 1 < count { at + 1 } else { 0 };
        }
        self.growing = true;
        self.missed += 1;
        None
    }

    /// Frees `slot`: out of the live list by swapping the last into its place (`erase_icon`'s
    /// compaction), the slot free, its window hidden when visible. The grids and lists that
    /// Hide reached.
    fn free(&mut self, template: &Element, slot: SlotId) -> Result<Vec<Held>, String> {
        if let Some(at) = self.live.iter().position(|(live, _)| *live == slot) {
            self.live.swap_remove(at);
        }
        let entry = &mut self.slots[slot.0];
        entry.in_use = false;
        if entry.instance.drawn() {
            entry.instance.hide(template, template)?;
        }
        Ok(entry.instance.take_held())
    }

    /// Erases `owner`'s icon, returning grids and lists reached by its window Hide. Does
    /// nothing if the owner has no icon.
    pub fn erase(&mut self, template: &Element, owner: &K) -> Result<Vec<Held>, String> {
        match self.slot_of(owner) {
            Some(slot) => self.free(template, slot),
            None => Ok(Vec::new()),
        }
    }

    /// Walks the existing live-list extent. `test` supplies each icon type's empty and
    /// viewport-admission checks. Rejected icons are erased; admitted ones receive their point
    /// and are shown when hidden. A slot moved into an erased position is reached there before
    /// the pass continues.
    pub fn update(
        &mut self,
        template: &Element,
        mut test: impl FnMut(&K, &Instance) -> Admission,
    ) -> Result<Pass<K>, String> {
        let mut pass = Pass {
            updated: Vec::new(),
            erased: Vec::new(),
        };
        // The array's storage as the pass reads it: erasing compacts the live prefix and leaves
        // the entries past it in place.
        let mut buffer = self.live.clone();
        for index in 0..buffer.len() {
            let (slot, owner) = buffer[index].clone();
            match test(&owner, &self.slots[slot.0].instance) {
                Admission::Empty | Admission::Outside => {
                    let live = self.live.len();
                    if let Some(at) = buffer[..live].iter().position(|(own, _)| *own == slot) {
                        buffer[at] = buffer[live - 1].clone();
                    }
                    let held = self.free(template, slot)?;
                    pass.erased.push(Erased { owner, slot, held });
                }
                Admission::Inside(point) => {
                    let instance = &mut self.slots[slot.0].instance;
                    instance.set_position(template, template, point)?;
                    if !instance.drawn() {
                        instance.gui_show(template, template)?;
                    }
                    pass.updated.push((owner, slot));
                }
            }
        }
        Ok(pass)
    }

    /// The window in `slot`, in use or free (a freed window still fades and moves).
    pub fn instance(&self, slot: SlotId) -> Option<&Instance> {
        self.slots.get(slot.0).map(|entry| &entry.instance)
    }

    /// The window of a live slot.
    pub fn get(&mut self, slot: SlotId) -> Option<&mut Instance> {
        self.slots
            .get_mut(slot.0)
            .filter(|entry| entry.in_use)
            .map(|entry| &mut entry.instance)
    }

    /// Every live icon's owner, slot and window, in the live list's order.
    pub fn live(&self) -> impl Iterator<Item = (&K, SlotId, &Instance)> {
        self.live
            .iter()
            .map(|(slot, owner)| (owner, *slot, &self.slots[slot.0].instance))
    }

    /// One frame of every window of the pool, live or not (each is registered with the GUI lifecycle), in
    /// the game's phases ([`Self::tick_clocks`], [`Self::tick_transitions`],
    /// [`Self::tick_draws`]). A caller whose other instances the pool's Shows and Hides reach
    /// (the items of a list or grid a window holds) runs each phase across all of them instead.
    pub fn tick(
        &mut self,
        template: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        self.tick_clocks(template, resources, tick)?;
        self.tick_transitions(template)?;
        self.tick_draws(template, resources)
    }

    /// The frame's first phase for every window of the pool ([`Instance::tick_clocks`]); a
    /// growth an exhausted request started is completed first. The frame on which the game's
    /// worker task completes it is not in any recording; the earliest is the frame after the
    /// request, which this takes, counted in [`Self::growths`] and reported by
    /// [`Self::take_diagnostics`].
    pub fn tick_clocks(
        &mut self,
        template: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        self.retrying = std::mem::take(&mut self.missed);
        if std::mem::take(&mut self.growing) {
            self.add(template, self.growth, resources)?;
            self.growths += 1;
            self.grown += 1;
        }
        for slot in &mut self.slots {
            slot.instance.tick_clocks(template, resources, tick)?;
        }
        Ok(())
    }

    /// The frame's second phase for every window of the pool, last registered first
    /// ([`Instance::tick_transitions`]): for a caller whose other instances none of the pool's
    /// Windows reach. One that propagates the windows' hides to instances of its own runs
    /// [`Self::tick_transitions_of`] per slot in [`Self::registrations`]' order among its
    /// instances instead.
    pub fn tick_transitions(&mut self, template: &Element) -> Result<(), String> {
        for index in (0..self.slots.len()).rev() {
            self.tick_transitions_of(template, SlotId(index))?;
        }
        Ok(())
    }

    /// Every window of the pool, live or free, with where it stands in the GUI lifecycle's list
    /// ([`Registration`]), in slot order (the order they were built in).
    pub fn registrations(&self) -> impl Iterator<Item = (Registration, SlotId)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.instance.registration(), SlotId(index)))
    }

    /// The frame's second phase for the window in `slot`, live or free
    /// ([`Instance::tick_transitions`]); the grids and lists its ended fade or movement reached
    /// are then in [`Self::take_held`].
    pub fn tick_transitions_of(&mut self, template: &Element, slot: SlotId) -> Result<(), String> {
        let entry = self
            .slots
            .get_mut(slot.0)
            .ok_or_else(|| format!("the pool of {} has no slot {}", self.template, slot.0))?;
        entry.instance.tick_transitions(template)
    }

    /// The frame's last phase for every window of the pool ([`Instance::tick_draws`]).
    pub fn tick_draws(
        &mut self,
        template: &Element,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        for slot in &mut self.slots {
            slot.instance.tick_draws(template, resources)?;
        }
        Ok(())
    }

    /// Whether any window of the pool changes on its own ([`Instance::active`]), or a growth
    /// waits.
    pub fn active(&self) -> bool {
        self.growing || self.slots.iter().any(|slot| slot.instance.active())
    }

    /// Requests not granted this frame re-ask even after its tick completes growth.
    pub fn retry(&self) -> Option<super::super::icon_windows::Retry> {
        (self.retrying != 0 || self.missed != 0).then(|| super::super::icon_windows::Retry {
            name: self.template.clone(),
            requests: self.retrying.max(self.missed),
            // Each completed growth adds 16 slots. With constant demand, that many exhausted
            // requests disappear next frame.
            rounds: self.retrying.max(self.missed).div_ceil(self.growth),
        })
    }

    /// Ticks every window, occupied or free. A freed window may complete its fade or hide
    /// movement later, hiding children and reaching list/grid items at that time.
    pub fn take_held(&mut self, slot: SlotId) -> Vec<Held> {
        self.slots
            .get_mut(slot.0)
            .map_or(Vec::new(), |entry| entry.instance.take_held())
    }

    /// [`Self::take_held`] of every slot that has any, live or free, in slot order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn drain_held(&mut self) -> Vec<(SlotId, Vec<Held>)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, entry)| {
                let held = entry.instance.take_held();
                (!held.is_empty()).then_some((SlotId(index), held))
            })
            .collect()
    }

    /// How many times the pool grew.
    #[cfg(any(test, feature = "test-support"))]
    pub fn growths(&self) -> u64 {
        self.growths
    }

    /// One [`DiagnosticKind::PoolGrowth`] per growth completed since the last call, for the
    /// caller's report.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        let grown = std::mem::take(&mut self.grown);
        (0..grown)
            .map(|_| Diagnostic {
                path: self.template.clone(),
                kind: DiagnosticKind::PoolGrowth,
                detail: format!(
                    "the pool of {} was exhausted and grew by {} windows on a worker task, \
                     whose completion frame the recording does not hold: taken as the next \
                     frame",
                    self.template, self.growth
                ),
            })
            .collect()
    }
}

impl<K> std::fmt::Debug for Pool<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("template", &self.template)
            .field("slots", &self.slots.len())
            .field("live", &self.live.len())
            .field("cursor", &self.cursor)
            .finish()
    }
}

/// A window attached for drawing in the current frame. The manager clears and rebuilds its
/// graphical child list from admitted live icons whose windows are visible. Erased icons and
/// their items are detached even while fading; their transitions still run in [`Pool::tick`]
/// and release held items when hiding completes ([`Pool::drain_held`]).
#[derive(Debug)]
pub struct Visible<'a, K> {
    pub owner: &'a K,
    pub slot: SlotId,
    pub instance: &'a Instance,
    /// Why nothing is drawn for the window, when a refused update left part of it unknown
    /// ([`Pool::refuse`]); the icon stays in the list, as it does in the game's.
    pub withheld: Option<&'a str>,
}

/// What a refused update left unknown of a window: what a later update must write again before
/// the window is drawn ([`Withheld`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Unknown {
    /// The update's own calls: the window's next update whose calls are all made writes them
    /// again (an updater that writes the same set every update).
    Update,
    /// One part only a particular later write makes known again, named by the caller (for
    /// example a combat icon's flipped bar, whose move only the next rotation writes again);
    /// that write is also an update.
    Part(&'static str),
    /// Whatever followed a call that does not apply: every such failure is structural (a call on
    /// another template's window or element, or a window's call on a control), the same on every
    /// update, so no later update is known to write it all again; the window stays withheld until
    /// its windows are built again (a new pool, a new [`Withheld`]).
    UntilRebuilt,
}

/// What an update that made all its calls wrote again ([`Withheld::rewrote`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rewrote {
    /// Its own calls: lifts [`Unknown::Update`].
    Update,
    /// Its own calls and the named part: lifts [`Unknown::Update`] and [`Unknown::Part`] of that
    /// name.
    Part(&'static str),
}

/// The refusal standing on one window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Refused {
    why: Cow<'static, str>,
    unknown: Vec<Unknown>,
}

/// The one refusal state of windows whose update was refused, keyed by the caller's window key
/// (a pool keeps one per slot, [`Pool::refuse`]; a consumer keeps one for the windows it
/// constructs itself, such as grid items): a refused update (an operand its calls lack, a call
/// that does not apply, a read of its holder refused) leaves part of the window unknown, and
/// nothing is drawn for it until later updates write every unknown part again. The refusal is the
/// window's: its pool slot, its membership in the grids and lists that hold it and its place stay
/// as the game keeps them.
#[derive(Clone, Debug)]
pub struct Withheld<K> {
    refused: HashMap<K, Refused>,
}

impl<K> Default for Withheld<K> {
    fn default() -> Self {
        Self {
            refused: HashMap::new(),
        }
    }
}

impl<K: Eq + std::hash::Hash> Withheld<K> {
    /// A refused update of `key`'s window left `unknown` unknown, for `why` (the reason counted
    /// wherever the window would draw, the latest refusal's).
    pub fn refuse(&mut self, key: K, unknown: Unknown, why: Cow<'static, str>) {
        let refused = self.refused.entry(key).or_default();
        refused.why = why;
        if !refused.unknown.contains(&unknown) {
            refused.unknown.push(unknown);
        }
    }

    /// `part` of `key`'s window is known again without a write of the code (a transition the
    /// caller has seen end): only that [`Unknown::Part`] is lifted, and the window is drawn once
    /// none is left.
    pub fn known(&mut self, key: &K, part: &'static str) {
        let Some(refused) = self.refused.get_mut(key) else {
            return;
        };
        refused
            .unknown
            .retain(|unknown| *unknown != Unknown::Part(part));
        if refused.unknown.is_empty() {
            self.refused.remove(key);
        }
    }

    /// An update of `key`'s window made all its calls and wrote `rewrote` again: the unknown
    /// parts it covers are known again, and the window is drawn once none is left.
    pub fn rewrote(&mut self, key: &K, rewrote: Rewrote) {
        let Some(refused) = self.refused.get_mut(key) else {
            return;
        };
        refused.unknown.retain(|unknown| match (unknown, rewrote) {
            (Unknown::Update, _) => false,
            (Unknown::Part(part), Rewrote::Part(written)) => *part != written,
            (Unknown::Part(_), Rewrote::Update) | (Unknown::UntilRebuilt, _) => true,
        });
        if refused.unknown.is_empty() {
            self.refused.remove(key);
        }
    }

    /// Why nothing is drawn for `key`'s window, when a refused update left part of it unknown.
    pub fn why(&self, key: &K) -> Option<&str> {
        self.refused.get(key).map(|refused| refused.why.as_ref())
    }
}

impl<K: Clone + Eq> Pool<K> {
    /// The manager's visible list ([`Visible`]): every live icon whose window is visible, in the
    /// live list's order, with the refusal standing on its window.
    pub fn visible(&self) -> impl Iterator<Item = Visible<'_, K>> {
        self.live()
            .filter(|(_, _, instance)| instance.drawn())
            .map(|(owner, slot, instance)| Visible {
                owner,
                slot,
                instance,
                withheld: self.withheld.why(&slot),
            })
    }

    /// A refused window update ([`Withheld::refuse`]). The refusal remains with the slot,
    /// occupied or free, because a subsequent owner reuses the retained window state.
    pub fn refuse(&mut self, slot: SlotId, unknown: Unknown, why: Cow<'static, str>) {
        self.withheld.refuse(slot, unknown, why);
    }

    /// `part` of the window in `slot` is known again without a write ([`Withheld::known`]).
    pub fn known(&mut self, slot: SlotId, part: &'static str) {
        self.withheld.known(&slot, part);
    }

    /// An update of the window in `slot` made all its calls ([`Withheld::rewrote`]).
    pub fn rewrote(&mut self, slot: SlotId, rewrote: Rewrote) {
        self.withheld.rewrote(&slot, rewrote);
    }

    /// Why nothing is drawn for the window in `slot`, when a refusal stands on it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn withheld(&self, slot: SlotId) -> Option<&str> {
        self.withheld.why(&slot)
    }
}

/// One call of the game's code on a window instance, as a prepared hour records it.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    #[cfg(any(test, feature = "test-support"))]
    SetPosition([i32; 2]),
    Show,
    #[cfg(any(test, feature = "test-support"))]
    GuiShow,
    Hide,
    #[cfg(any(test, feature = "test-support"))]
    HideWindow,
    SetHideFlag(bool),
    SetFrame(i32),
    /// Sprite replacement through a setter ([`Instance::set_sprite`]).
    SetSprite(Vec<u8>, SpriteSetter),
    /// A text box's text ([`Instance::set_text`]).
    SetText(String),
    #[cfg(any(test, feature = "test-support"))]
    Play,
    /// Window opacity assignment, optionally including children ([`Instance::set_opacity`]).
    #[cfg(any(test, feature = "test-support"))]
    SetOpacity(f32, bool),
    /// The code's writes on the control ([`Instance::control`]): every field it sets, the frame
    /// excepted ([`Call::SetFrame`]); a sprite it sets is its next setter call, and a text its
    /// next text write ([`Call::SetText`]), after the other fields.
    Control(Box<Bound>),
}

/// The calls a prepared hour's code makes, by instance key and element path, in order: made
/// without touching any instance, applied once to the presented view's instances.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calls<K> {
    pub calls: Vec<(K, String, Call)>,
}

impl<K: Clone + Eq + std::hash::Hash> Calls<K> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn push(&mut self, key: K, element: &Element, call: Call) {
        self.calls.push((key, element.path.clone(), call));
    }

    /// Makes the calls on `instances`' windows of `root` (by each call's element path), in
    /// order, with the resources the instances were built with (a text write refreshes its
    /// scrollbar at once, [`Instance::set_text`]). A key with no instance, or a path not in the
    /// template, is refused by name.
    pub fn apply(
        &self,
        root: &Element,
        instances: &mut HashMap<K, Instance>,
        resources: &dyn Resources,
    ) -> Result<(), String>
    where
        K: std::fmt::Debug,
    {
        let elements = elements(root);
        for (key, path, call) in &self.calls {
            let instance = instances
                .get_mut(key)
                .ok_or_else(|| format!("no instance of {} for {key:?}", root.path))?;
            let element = elements
                .iter()
                .find(|(element, ..)| &element.path == path)
                .map(|(element, ..)| *element)
                .ok_or_else(|| format!("{path} is not an element of {}", root.path))?;
            match call {
                #[cfg(any(test, feature = "test-support"))]
                Call::SetPosition(point) => instance.set_position(root, element, *point)?,
                Call::Show => instance.show(root, element)?,
                #[cfg(any(test, feature = "test-support"))]
                Call::GuiShow => instance.gui_show(root, element)?,
                Call::Hide => instance.hide(root, element)?,
                #[cfg(any(test, feature = "test-support"))]
                Call::HideWindow => instance.hide_window(root, element)?,
                Call::SetHideFlag(hide) => instance.set_hide_flag(root, element, *hide)?,
                Call::SetFrame(frame) => instance.set_frame(root, element, *frame)?,
                Call::SetSprite(name, setter) => {
                    instance.set_sprite(root, element, name, *setter)?
                }
                Call::SetText(text) => instance.set_text(root, element, text, resources)?,
                #[cfg(any(test, feature = "test-support"))]
                Call::Play => instance.play(root, element)?,
                #[cfg(any(test, feature = "test-support"))]
                Call::SetOpacity(value, children) => {
                    instance.set_opacity(root, element, *value, *children)?
                }
                Call::Control(bound) => {
                    let control = instance.control(root, element)?;
                    // The code's own writes; the frame goes through [`Instance::set_frame`] and the rest is
                    // the instance's.
                    let Bound {
                        sprite,
                        setter,
                        text,
                        resolved_flags,
                        color,
                        progress,
                        progress_float,
                        texture,
                        stretch,
                        offset,
                        rotation,
                        position,
                        shield,
                        checked,
                        enabled,
                        ..
                    } = (**bound).clone();
                    control.sprite = sprite;
                    control.setter = setter;
                    control.resolved_flags = resolved_flags;
                    control.color = color;
                    control.progress = progress;
                    control.progress_float = None;
                    control.texture = texture;
                    control.stretch = stretch;
                    control.offset = offset;
                    control.rotation = rotation;
                    control.position = position;
                    control.shield = shield;
                    control.checked = checked;
                    control.enabled = enabled;
                    if let Some(value) = progress_float {
                        instance.set_progress_float_value(root, element, value)?;
                    }
                    if let Some(text) = text {
                        instance.set_text(root, element, &text, resources)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod frame_clock_tests {
    use super::*;

    /// The frame count is stored unclamped and its bits convert unsigned. With -1 one-second
    /// frames, duration is about 4.29e9 seconds, so a nonlooping sprite still plays after a
    /// tick at frame 1, without a cap or refusal.
    #[test]
    fn a_negative_frame_count_is_its_unsigned_bits() {
        let kind = SpriteKind::FrameAnimated {
            seconds_per_frame: 1.0,
            looping: false,
            play_on_show: true,
            pause: 0.0,
        };
        let mut clock = FrameClock::new();
        clock.update(&kind, -1, 0.02).unwrap();
        assert_eq!(clock.frame, Some(1));
        assert!(clock.elapsed > 0.0);
        clock.update(&kind, -1, 0.02).unwrap();
        assert_eq!(clock.frame, Some(1));
    }
}
