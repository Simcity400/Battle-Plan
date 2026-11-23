
use super::*;
use std::{borrow::Cow, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub time: f32,
    pub dt: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub(super) visible: bool,
    pub(super) hide: bool,
    pub(super) hidden_by_code: bool,
}

impl Shown {
    pub(super) fn shown_by_code(element: &Element) -> Self {
        Self {
            visible: !element.hide,
            hide: element.hide,
            hidden_by_code: false,
        }
    }

    pub(super) fn show(&mut self) -> bool {
        if self.hide {
            return false;
        }
        self.visible = true;
        true
    }

    pub(super) fn hide(&mut self) {
        self.visible = false;
        self.hidden_by_code = true;
    }
}

fn seconds(milliseconds: i32) -> f32 {
    milliseconds as f32 / 1000.0
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
struct Window {
    parent: [i32; 2],
    size: native::Size,
    frame: Frame,
    show_position: native::Position,
    anchored_position: [i32; 2],
    fade: u8,
    fade_elapsed: f32,
    opacity: f32,
    current: [i32; 2],
    position: [i32; 2],
    target: [i32; 2],
    delta: [f32; 2],
    elapsed: f32,
    done: bool,
    mode: u8,
    kind: Animation,
    duration: f32,
    hide_children: bool,
    hide_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrameClock {
    elapsed: f32,
    playing: bool,
    frame: Option<i32>,
}

impl FrameClock {
    pub(super) fn new() -> Self {
        Self {
            elapsed: 0.0,
            playing: false,
            frame: None,
        }
    }

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
            content_reader_parser::crt::truncate_f32(self.elapsed / duration * frame_count + 1.0)
        } else {
            1
        });
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn play(&mut self) {
        self.playing = true;
    }

    #[cfg(test)]
    pub(super) fn frame(&self) -> Option<i32> {
        self.frame
    }

    fn runs(&self, kind: &SpriteKind) -> bool {
        matches!(*kind, SpriteKind::FrameAnimated { play_on_show, .. } if self.playing || play_on_show)
    }
}

pub(super) fn animation_time(frame: i32, elapsed: f32, kind: &SpriteKind) -> f32 {
    let SpriteKind::FrameAnimated {
        seconds_per_frame, ..
    } = *kind
    else {
        return 0.0;
    };
    ((elapsed - (frame - 1) as f32 * seconds_per_frame) / seconds_per_frame).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Pulse {
    phase: f32,
}

impl Pulse {
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

#[derive(Clone, Debug, PartialEq)]
struct Clock {
    clock: FrameClock,
    kind: SpriteKind,
}

#[derive(Clone, Debug, PartialEq)]
struct Runtime {
    sprite: Vec<u8>,
    made: u32,
    clock: Option<Clock>,
    pulse: Option<Pulse>,
    hang: Option<String>,
    first_draw: Option<f32>,
    effect: Option<StateEffect>,
    triggers: Vec<(f32, u32)>,
}

impl Runtime {
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

#[derive(Clone, Debug, PartialEq)]
struct Made {
    sprite: Arc<[u8]>,
    setter: SpriteSetter,
    made: u32,
}

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Sprite(Vec<u8>, SpriteSetter),
    Frame(i32),
    ProgressFloat(i32),
    #[cfg(any(test, feature = "test-support"))]
    Play,
}

#[derive(Clone, Debug, PartialEq)]
struct Retained {
    made: Option<Made>,
    count: u32,
    frame: Option<i32>,
    progress_float: Option<i32>,
    framed: bool,
    played: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StateEffect {
    state: u32,
    at: f32,
    settles: Settles,
}

const LONGEST_CLOCK: f32 = u64::MAX as f32;

fn settled(settles: &Settling, elapsed: f32, clock: f32) -> bool {
    settles.limit >= LONGEST_CLOCK && settles.reached(elapsed, clock)
}

impl super::Shading {
    pub fn changing(&self) -> bool {
        match self.settles() {
            Settles::Program(None) => self.clocked,
            Settles::Program(Some(settles)) => {
                self.clocked
                    && !settled(
                        &settles,
                        self.since,
                        self.time.abs().max(self.animation.abs()),
                    )
            }
            Settles::Previous | Settles::Refused => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Drawables {
    None,
    Scrollbar {
        parts: Vec<usize>,
        icons: Vec<(usize, bool)>,
    },
    All(Vec<usize>),
    DropDown {
        expand: Option<usize>,
        expanded: Option<usize>,
        header: Option<usize>,
    },
    TextBox {
        bar: usize,
        flag: bool,
        state: widgets::PlainState,
        scroll: f32,
        written: Option<String>,
    },
    ListBox {
        bar: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Smooth {
    vertical: bool,
    bar: usize,
    slider: Option<usize>,
    state: widgets::ContainerBar,
    enabled: bool,
    factor: i64,
    current: i64,
    notified: Option<i32>,
}

impl Smooth {
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

#[derive(Clone, Debug)]
struct Node {
    parent: Option<usize>,
    child: bool,
    part: bool,
    built: Option<(usize, Role)>,
    stateful: bool,
    hide: bool,
    children: Vec<usize>,
    windows: Vec<usize>,
    kind: Kind,
    shown: Shown,
    window: Option<Window>,
    drawables: Drawables,
    owned: Vec<(usize, bool)>,
    smooth: Vec<Smooth>,
    control: Bound,
    made: Option<Made>,
    made_count: u32,
    events: Vec<Event>,
    played: bool,
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
            played: false,
            runtime: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub path: String,
    pub shown: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Registration(u64);

impl Registration {
    fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Drawn,
    Clocked,
    Moved,
}

#[derive(Clone, Debug)]
pub struct Instance {
    template: String,
    count: usize,
    tree: Arc<Tree>,
    text_boxes: Vec<usize>,
    clock_nodes: Vec<usize>,
    viewport: [i32; 2],
    nodes: Vec<Node>,
    registered: Vec<usize>,
    registration: Registration,
    phase: Phase,
    time: f32,
    dt: f32,
    settle: bool,
    ticked: bool,
    written: bool,
    idle_tick: bool,
    kept: bool,
    held: Vec<Held>,
    pointed: Option<super::Pointed>,
    boxes: Vec<String>,
    active_known: std::cell::Cell<Option<bool>>,
    revision: u64,
    laid: std::cell::RefCell<Option<Laid>>,
}

#[derive(Clone, Debug)]
struct Laid {
    revision: u64,
    root: usize,
    stamp: u64,
    regions: bool,
    layout: Result<Option<Layout>, Refusals>,
}

fn nested(element: &Element) -> impl DoubleEndedIterator<Item = (&Element, bool)> {
    let backgrounds = match &element.fields {
        Fields::Container(container) => container.backgrounds.as_slice(),
        _ => &[],
    };
    let mut parts = [None; 5];
    match &element.fields {
        Fields::Grid(grid) => parts[0] = grid.background.as_deref(),
        Fields::ExtendedScrollbar(bar) => {
            parts = [
                bar.background.as_deref(),
                bar.track.as_deref(),
                bar.slider.as_deref(),
                bar.increase_button.as_deref(),
                bar.decrease_button.as_deref(),
            ];
        }
        Fields::DropDown(drop) => {
            parts[0] = drop.expand_button.as_deref();
            parts[1] = drop.expanded_window.as_deref();
            parts[2] = Some(&drop.inner);
        }
        _ => {}
    }
    backgrounds
        .iter()
        .chain(parts.into_iter().flatten())
        .map(|element| (element, false))
        .chain(element.children.iter().map(|element| (element, true)))
}

fn nested_at(element: &Element, mut slot: usize) -> &Element {
    if let Fields::Container(container) = &element.fields {
        if let Some(background) = container.backgrounds.get(slot) {
            return background;
        }
        slot -= container.backgrounds.len();
    }
    let parts = match &element.fields {
        Fields::Grid(grid) => [grid.background.as_deref(), None, None, None, None],
        Fields::ExtendedScrollbar(bar) => [
            bar.background.as_deref(),
            bar.track.as_deref(),
            bar.slider.as_deref(),
            bar.increase_button.as_deref(),
            bar.decrease_button.as_deref(),
        ],
        Fields::DropDown(drop) => [
            drop.expand_button.as_deref(),
            drop.expanded_window.as_deref(),
            Some(&drop.inner),
            None,
            None,
        ],
        _ => [None; 5],
    };
    for part in parts.into_iter().flatten() {
        if slot == 0 {
            return part;
        }
        slot -= 1;
    }
    &element.children[slot]
}

fn elements(root: &Element) -> Vec<(&Element, Option<usize>, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![(root, None, true)];
    while let Some((element, parent, child)) = stack.pop() {
        let index = out.len();
        out.push((element, parent, child));
        for (nested, child) in nested(element).rev() {
            stack.push((nested, Some(index), child));
        }
    }
    out
}

#[derive(Debug)]
struct Tree {
    root: usize,
    routes: Vec<(Option<usize>, usize, bool)>,
    indices: HashMap<usize, usize>,
    paths: HashMap<String, usize>,
}

impl Tree {
    fn new(elements: &[(&Element, Option<usize>, bool)]) -> Self {
        let mut paths = HashMap::with_capacity(elements.len());
        for (index, (element, ..)) in elements.iter().enumerate() {
            paths.entry(element.path.clone()).or_insert(index);
        }
        Self {
            root: elements[0].0 as *const Element as usize,
            routes: elements
                .iter()
                .map(|&(element, parent, child)| {
                    let slot = parent.map_or(0, |parent| {
                        nested(elements[parent].0)
                            .position(|(own, _)| std::ptr::eq(own, element))
                            .expect("the authored parent's part")
                    });
                    (parent, slot, child)
                })
                .collect(),
            indices: elements
                .iter()
                .enumerate()
                .map(|(index, (element, ..))| (*element as *const Element as usize, index))
                .collect(),
            paths,
        }
    }

    fn at<'a>(&self, root: &'a Element, index: usize) -> &'a Element {
        let (parent, slot, _) = self.routes[index];
        parent.map_or(root, |parent| nested_at(self.at(root, parent), slot))
    }

    fn same(&self, root: &Element) -> bool {
        root as *const Element as usize == self.root
    }

    fn objects<'a>(&self, root: &'a Element, nodes: &[Node]) -> Vec<Object<'a>> {
        let mut out: Vec<Object> = Vec::with_capacity(self.routes.len());
        for (&(parent, slot, child), node) in self.routes.iter().zip(nodes) {
            let element = parent.map_or(root, |parent| nested_at(out[parent].element, slot));
            out.push(Object {
                element,
                parent,
                child,
                part: node.part,
                built: None,
            });
        }
        out
    }

    fn elements<'a>(&self, root: &'a Element) -> Vec<(&'a Element, Option<usize>, bool)> {
        if !self.same(root) {
            return elements(root);
        }
        let mut out: Vec<(&Element, Option<usize>, bool)> = Vec::with_capacity(self.routes.len());
        for &(parent, slot, child) in &self.routes {
            let element = parent.map_or(root, |parent| nested_at(out[parent].0, slot));
            out.push((element, parent, child));
        }
        out
    }
}

#[derive(Default)]
struct CallElements<'a> {
    held: Option<Vec<(&'a Element, Option<usize>, bool)>>,
    #[cfg(test)]
    builds: usize,
}

impl<'a> CallElements<'a> {
    fn get(&mut self, tree: &Tree, root: &'a Element) -> &[(&'a Element, Option<usize>, bool)] {
        self.held.get_or_insert_with(|| {
            #[cfg(test)]
            {
                self.builds += 1;
            }
            tree.elements(root)
        })
    }
}

struct Object<'a> {
    element: &'a Element,
    parent: Option<usize>,
    child: bool,
    part: bool,
    built: Option<(usize, Role)>,
}

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

fn drawn<'a>(
    element: &'a Element,
    made: Option<&'a Made>,
    text: Option<&str>,
    resources: &'a dyn Resources,
) -> Option<(&'a [u8], Cow<'a, SpriteInfo>)> {
    let lookup = |name: &'a [u8]| resources.sprite_info(name).map(|info| (name, info));
    let fallback = |name: &'a [u8]| [name, FALLBACK_SPRITE].into_iter().find_map(lookup);
    match made {
        Some(made) => fallback(&made.sprite),
        None => {
            if let Some(button) = element.button()
                && element.kind != Kind::Checkbox
                && button.sprite.is_empty()
                && !button.font.is_empty()
            {
                let translated;
                let shown = match text {
                    Some(text) => Some(text),
                    None => {
                        translated = localised(resources, &button.text);
                        translated.as_deref()
                    }
                };
                if text_only(button, &button.sprite, shown) {
                    return None;
                }
            }
            let (name, use_fallback) = own_sprite(element)?;
            if use_fallback {
                fallback(name)
            } else {
                lookup(name)
            }
        }
    }
}

fn clock_candidate(element: &Element, made: Option<&Made>, resources: &dyn Resources) -> bool {
    if made.is_none()
        && element.button().is_some_and(|button| {
            element.kind != Kind::Checkbox && button.sprite.is_empty() && !button.font.is_empty()
        })
    {
        return true;
    }
    drawn(element, made, None, resources).is_some_and(|(_, info)| {
        matches!(
            info.kind,
            SpriteKind::FrameAnimated { .. }
                | SpriteKind::CorneredTile {
                    looping: true,
                    rate: Some(_),
                    ..
                }
        )
    })
}

fn technique(state: u32) -> &'static str {
    match state {
        2 => "Down",
        3 => "Over",
        4 => "Disable",
        _ => "Up",
    }
}

fn stateful(objects: &[Object], index: usize) -> bool {
    let object = &objects[index];
    let element = object.element;
    if element.button().is_none() {
        return false;
    }
    let holder = object.parent.map(|up| objects[up].element);
    match object.built {
        Some((_, Role::Part(slot))) => matches!(slot, 0 | 1 | 4 | 5),
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

fn extended_button(holder: &Element, element: &Element) -> bool {
    matches!(&holder.fields, Fields::ExtendedScrollbar(bar)
        if [&bar.increase_button, &bar.decrease_button]
            .into_iter()
            .any(|button| button.as_deref().is_some_and(|button| std::ptr::eq(button, element))))
}

impl Instance {
    pub fn construct(
        root: &Element,
        viewport: [i32; 2],
        resources: &dyn Resources,
    ) -> Result<Self, String> {
        Self::construct_in(root, viewport, viewport, resources)
    }

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
        let authored_elements = elements(root);
        let authored = authored_elements.len();
        let tree = Arc::new(Tree::new(&authored_elements));
        let mut nodes: Vec<Node> = Vec::with_capacity(objects.len());
        let mut content: Vec<Option<[i32; 2]>> = Vec::with_capacity(objects.len());
        let mut frames: Vec<Option<Frame>> = Vec::with_capacity(objects.len());
        for (index, object) in objects.iter().enumerate() {
            let element = object.element;
            let (window, frame) = match element.container() {
                Some(container) if object.built.is_none() => {
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
            tree,
            text_boxes: nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    matches!(node.drawables, Drawables::TextBox { .. }).then_some(index)
                })
                .collect(),
            clock_nodes: objects
                .iter()
                .enumerate()
                .filter_map(|(index, object)| {
                    clock_candidate(object.element, nodes[index].made.as_ref(), resources)
                        .then_some(index)
                })
                .collect(),
            viewport,
            nodes,
            registered,
            registration: Registration::next(),
            phase: Phase::Drawn,
            time: 0.0,
            dt: 0.0,
            settle: false,
            ticked: false,
            written: false,
            active_known: std::cell::Cell::new(None),
            revision: 0,
            laid: std::cell::RefCell::new(None),
            idle_tick: false,
            kept: false,
            held: Vec::new(),
            pointed: None,
            boxes: Vec::new(),
        };
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

    fn wrote(&mut self) {
        self.written = true;
        self.active_known.set(None);
        self.revision += 1;
    }

    fn check(&self, root: &Element) -> Result<(), String> {
        if root.path != self.template
            || (!self.tree.same(root) && elements(root).len() != self.count)
        {
            return Err(format!(
                "the lifecycle of {} was asked about {}",
                self.template, root.path
            ));
        }
        Ok(())
    }

    fn objects<'a>(
        &self,
        root: &'a Element,
        resources: &'a dyn Resources,
        empty: &'a Bindings,
    ) -> Result<Vec<Object<'a>>, String> {
        self.check(root)?;
        let objects = if self.nodes.len() == self.count && self.tree.same(root) {
            self.tree.objects(root, &self.nodes)
        } else {
            let objects = graph(root, resources, empty);
            let same = objects.len() == self.nodes.len()
                && objects.iter().zip(&self.nodes).all(|(object, node)| {
                    object.built == node.built && object.parent == node.parent
                });
            if !same {
                return Err(format!(
                    "the lifecycle of {} was built against other templates: the objects its \
                     construction builds from the registry differ",
                    self.template
                ));
            }
            objects
        };
        for &index in &self.text_boxes {
            let node = &self.nodes[index];
            if let Drawables::TextBox { written, .. } = &node.drawables
                && node.control.text != *written
            {
                return Err(format!(
                    "{} was given a text through Instance::control: its scrollbar refreshes on \
                     every write, in order with the code's other calls, so its code writes it \
                     through Instance::set_text or Call::SetText",
                    objects[index].element.path
                ));
            }
        }
        Ok(objects)
    }

    pub fn element_at_path<'a>(
        &self,
        root: &'a Element,
        path: &str,
    ) -> Result<&'a Element, String> {
        self.check(root)?;
        let element = if self.tree.same(root) {
            self.tree
                .paths
                .get(path)
                .map(|&index| self.tree.at(root, index))
        } else {
            elements(root)
                .into_iter()
                .find_map(|(element, ..)| (element.path == path).then_some(element))
        };
        element.ok_or_else(|| format!("{path} is not an element of {}", root.path))
    }

    fn index(&self, root: &Element, element: &Element) -> Result<usize, String> {
        self.check(root)?;
        let index = if self.tree.same(root) {
            self.tree
                .indices
                .get(&(element as *const Element as usize))
                .copied()
        } else {
            elements(root)
                .iter()
                .position(|(own, ..)| std::ptr::eq(*own, element))
        };
        index.ok_or_else(|| format!("{} is not an element of {}", element.path, root.path))
    }

    pub fn viewport(&self) -> [i32; 2] {
        self.viewport
    }

    pub fn drawn(&self) -> bool {
        self.nodes[0].shown.visible
    }

    pub fn opacity(&self) -> f32 {
        self.nodes[0].window.map_or(1.0, |window| window.opacity)
    }

    pub fn position(&self) -> [i32; 2] {
        self.nodes[0]
            .window
            .map_or([0; 2], |window| window.position)
    }

    pub fn take_held(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.held)
    }

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
        if index != 0 && self.nodes[index].window.is_some_and(|window| window.position != point) {
            self.revision += 1;
        }
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

    pub fn set_size(
        &mut self,
        root: &Element,
        element: &Element,
        pixels: [i32; 2],
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        self.wrote();
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

    pub fn content_size(&self, root: &Element, element: &Element) -> Result<[i32; 2], String> {
        let index = self.index(root, element)?;
        self.nodes[index]
            .window
            .map(|window| window.frame.content_size)
            .ok_or_else(|| format!("{} is not a containerWindowType", element.path))
    }

    pub fn show(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        self.show_using(root, element, &mut CallElements::default())
    }

    fn show_using<'a>(
        &mut self,
        root: &'a Element,
        element: &Element,
        authored: &mut CallElements<'a>,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        let elements = authored.get(&self.tree, root);
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
        let container = element.container().expect("a window node");
        self.begin_show_fade(elements, index);
        if self.nodes[index].window.expect("a window node").mode == 1 {
            return Ok(());
        }
        if container.show_animation == Animation::None {
            self.show_now(elements, index);
            return Ok(());
        }
        self.show_now(elements, index);
        self.start_show_movement(elements, index)
    }

    pub fn gui_show(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        self.gui_show_using(root, element, &mut CallElements::default())
    }

    fn gui_show_using<'a>(
        &mut self,
        root: &'a Element,
        element: &Element,
        authored: &mut CallElements<'a>,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        let elements = authored.get(&self.tree, root);
        self.gui_show_at(elements, index)
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

    pub fn hide(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        self.hide_using(root, element, &mut CallElements::default())
    }

    fn hide_using<'a>(
        &mut self,
        root: &'a Element,
        element: &Element,
        authored: &mut CallElements<'a>,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        let elements = authored.get(&self.tree, root);
        self.hide_at(elements, index)
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn hide_window(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        self.hide_window_using(root, element, &mut CallElements::default())
    }

    #[cfg(any(test, feature = "test-support"))]
    fn hide_window_using<'a>(
        &mut self,
        root: &'a Element,
        element: &Element,
        authored: &mut CallElements<'a>,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        let elements = authored.get(&self.tree, root);
        let Some(window) = self.nodes[index].window else {
            return Err(format!("{} is not a containerWindowType", element.path));
        };
        if window.mode == 2 {
            return Ok(());
        }
        let container = element.container().expect("a window node");
        self.begin_hide_fade(elements, index);
        if container.hide_animation == Animation::None {
            self.hide_now(elements, index);
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
        self.retarget(elements, index, target, false, true)
    }

    pub fn set_hide_flag(
        &mut self,
        root: &Element,
        element: &Element,
        hide: bool,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        self.nodes[index].shown.hide = hide;
        Ok(())
    }

    pub fn control(&mut self, root: &Element, element: &Element) -> Result<&mut Bound, String> {
        self.wrote();
        let index = self.index(root, element)?;
        self.flush(index);
        Ok(&mut self.nodes[index].control)
    }

    pub fn finalize_grid(
        &mut self,
        root: &Element,
        grid: &Element,
        holder: &Holder,
        items: &[&Element],
    ) -> Result<GridLayout, Refusals> {
        self.wrote();
        self.finalize_grid_with(root, grid, holder, |parent, viewport, position| {
            super::finalize_grid(grid, parent, viewport, items, position)
        })
    }

    pub fn finalize_grid_resolved(
        &mut self,
        root: &Element,
        grid: &Element,
        holder: &Holder,
        items: &[([i32; 2], [i32; 2])],
    ) -> Result<GridLayout, Refusals> {
        self.wrote();
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

    pub fn synchronize_scroll(
        &mut self,
        root: &Element,
        element: &Element,
        resources: &dyn Resources,
        include_hidden: bool,
    ) -> Result<(), Refusals> {
        self.wrote();
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
                let previous = widgets::fixed(bar.state.value as f32 / 100_000.0);
                bar.state.range = fact.range;
                bar.state.value = fact.range.clamp(bar.state.value);
                if fact.vertical {
                    bar.set_target(fact.bar, previous.min(fact.range.max), true);
                }
            } else if was_shown {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_scroll_target(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        value: f32,
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        self.wrote();
        self.with_scrollbar(root, element, vertical, resources, |bar, template| {
            bar.set_target(template, widgets::fixed(value), true);
        })
    }

    pub fn step_scroll(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        direction: i32,
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        self.wrote();
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

    pub fn wheel_scroll(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        delta: i32,
        resources: &dyn Resources,
    ) -> Result<(), Refusals> {
        self.wrote();
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

    pub fn scroll_to(
        &mut self,
        root: &Element,
        element: &Element,
        vertical: bool,
        target: impl FnOnce([i32; 2]) -> Option<i32>,
        resources: &dyn Resources,
    ) -> Result<Option<([i32; 2], i32)>, Refusals> {
        self.wrote();
        let mut reached = None;
        self.with_scrollbar(root, element, vertical, resources, |bar, template| {
            if !bar.state.shown {
                return;
            }
            let range = [bar.state.range.min, bar.state.range.max].map(|end| (end / 100_000) as i32);
            let value = match target(range) {
                Some(pixels) => {
                    let clamped = bar.state.range.clamp(i64::from(pixels) * 100_000);
                    bar.set_target(template, clamped, false);
                    let offset = (clamped / 100_000) as i32;
                    bar.notified = Some(offset);
                    offset
                }
                None => (bar.state.value / 100_000) as i32,
            };
            reached = Some((range, value));
        })?;
        Ok(reached)
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

    fn flush(&mut self, index: usize) {
        let node = &mut self.nodes[index];
        if let Some(name) = node.control.sprite.take() {
            node.events.push(Event::Sprite(name, node.control.setter));
        }
    }

    pub fn set_sprite(
        &mut self,
        root: &Element,
        element: &Element,
        name: &[u8],
        setter: SpriteSetter,
    ) -> Result<(), String> {
        let index = self.index(root, element)?;
        let node = &self.nodes[index];
        if node.control.sprite.is_none()
            && !node
                .events
                .iter()
                .any(|event| matches!(event, Event::Sprite(..)))
            && node.made.as_ref().map_or_else(
                || own_sprite(element).map_or(&[][..], |(name, _)| name),
                |made| made.sprite.as_ref(),
            ) == name
        {
            return Ok(());
        }
        self.wrote();
        self.flush(index);
        self.nodes[index]
            .events
            .push(Event::Sprite(name.to_vec(), setter));
        Ok(())
    }

    pub fn set_frame(
        &mut self,
        root: &Element,
        element: &Element,
        frame: i32,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index].control.progress_float = None;
        self.nodes[index].events.push(Event::Frame(frame));
        Ok(())
    }

    pub fn set_progress_float_value(
        &mut self,
        root: &Element,
        element: &Element,
        value: i32,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index].events.push(Event::ProgressFloat(value));
        Ok(())
    }

    pub fn set_text(
        &mut self,
        root: &Element,
        element: &Element,
        text: &str,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        self.set_text_using(root, element, text, resources, &mut CallElements::default())
    }

    fn set_text_using<'a>(
        &mut self,
        root: &'a Element,
        element: &Element,
        text: &str,
        resources: &dyn Resources,
        authored: &mut CallElements<'a>,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        match &mut self.nodes[index].control.text {
            Some(written) => text.clone_into(written),
            written @ None => *written = Some(text.to_owned()),
        }
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
        let elements = authored.get(&self.tree, root);
        match refreshed {
            Refreshed::Above { flagged } if flagged || self.nodes[bar].shown.visible => {
                self.gui_show_at(elements, bar)
            }
            Refreshed::AtMost => self.hide_at(elements, bar),
            _ => Ok(()),
        }
    }

    pub fn set_text_scroll_lines(
        &mut self,
        root: &Element,
        element: &Element,
        lines: usize,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        self.wrote();
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

    pub fn scroll_text(
        &mut self,
        root: &Element,
        element: &Element,
        direction: i32,
        wheel: bool,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        self.wrote();
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
                        .map_or(authored, |made| made.sprite.as_ref());
                    if setter_replaces(current, name, resources) {
                        kept.count += 1;
                        kept.made = Some(Made {
                            sprite: name.as_slice().into(),
                            setter: *setter,
                            made: kept.count,
                        });
                        kept.frame = None;
                        kept.progress_float = None;
                        kept.framed = false;
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

    pub fn set_opacity(
        &mut self,
        root: &Element,
        element: &Element,
        value: f32,
        children: bool,
    ) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        if self.nodes[index].window.is_none() {
            return Err(format!("{} is not a containerWindowType", element.path));
        }
        self.write_opacity(index, value, children);
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn play(&mut self, root: &Element, element: &Element) -> Result<(), String> {
        self.wrote();
        let index = self.index(root, element)?;
        self.flush(index);
        self.nodes[index].events.push(Event::Play);
        Ok(())
    }

    fn show_now(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        self.nodes[index].shown.visible = true;
        for (owned, flag) in self.nodes[index].owned.clone() {
            if flag {
                let _ = self.gui_show_at(elements, owned);
            }
        }
    }

    fn hide_now(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        self.nodes[index].shown.visible = false;
        for (owned, flag) in self.nodes[index].owned.clone() {
            if flag {
                let _ = self.hide_at(elements, owned);
            }
        }
    }

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

    fn show_drawables(&mut self, elements: &[(&Element, Option<usize>, bool)], index: usize) {
        for (part, shown) in self.drawables(index, true) {
            let _ = if shown {
                self.gui_show_at(elements, part)
            } else {
                self.hide_at(elements, part)
            };
        }
    }

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

    fn fixed_hide(
        &mut self,
        elements: &[(&Element, Option<usize>, bool)],
        index: usize,
    ) -> Result<(), String> {
        self.hide_now(elements, index);
        self.hide_children(elements, index)
    }

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
        self.window_position(element, index, point);
        Ok(())
    }

    fn drawable_shown(&self, index: usize) -> bool {
        let node = &self.nodes[index];
        match (node.child || node.part, node.parent) {
            (false, Some(up)) => self.nodes[up].shown.visible,
            _ => node.shown.visible,
        }
    }

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

    fn state(&self, element: &Element, index: usize) -> u32 {
        let node = &self.nodes[index];
        let control = &node.control;
        match (&element.kind, element.button()) {
            (Kind::Checkbox, _) if !node.stateful => {
                if control.checked == Some(true) {
                    2
                } else if self.boxes.contains(&element.path) {
                    3
                } else {
                    1
                }
            }
            (_, Some(button)) if node.stateful => {
                if !control.enabled.unwrap_or(button.enabled) {
                    4
                } else {
                    match &self.pointed {
                        Some(pointed) if pointed.path == element.path => {
                            if pointed.down { 2 } else { 3 }
                        }
                        _ => 1,
                    }
                }
            }
            _ => 1,
        }
    }

    pub fn point(&mut self, pointed: Option<super::Pointed>, boxes: &[String]) {
        self.wrote();
        self.pointed = pointed;
        self.boxes.clear();
        self.boxes.extend_from_slice(boxes);
    }

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

    pub fn settle_transitions(&mut self) {
        self.settle = true;
    }

    pub fn registration(&self) -> Registration {
        self.registration
    }

    pub fn tick_clocks(
        &mut self,
        root: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        self.check(root)?;
        self.idle_tick = self.idle();
        self.active_known.set(None);
        if self.idle_tick {
            self.time = tick.time;
            self.dt = tick.dt;
            for node in &mut self.nodes {
                node.played = false;
            }
            self.phase = Phase::Clocked;
            return Ok(());
        }
        self.revision += 1;
        let empty = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let sprite_calls = self.nodes.iter().take(self.count).any(|node| {
            node.control.sprite.is_some()
                || node
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::Sprite(..)))
        });
        let invalid_text = self.text_boxes.iter().any(|&index| {
            let node = &self.nodes[index];
            matches!(&node.drawables, Drawables::TextBox { written, .. }
                if node.control.text != *written)
        });
        let objects = if sprite_calls
            || invalid_text
            || !self.clock_nodes.is_empty()
            || self.nodes.len() != self.count
            || !self.tree.same(root)
        {
            self.objects(root, resources, &empty)?
        } else {
            Vec::new()
        };
        let Tick { time, dt } = tick;
        self.time = time;
        self.dt = dt;
        for node in &mut self.nodes {
            node.played = false;
            if objects.is_empty() {
                for event in node.events.drain(..) {
                    match event {
                        Event::Frame(frame) => {
                            node.control.progress_float = None;
                            node.control.frame = Some(frame);
                        }
                        Event::ProgressFloat(value) => node.control.progress_float = Some(value),
                        #[cfg(any(test, feature = "test-support"))]
                        Event::Play => node.played = true,
                        Event::Sprite(..) => {
                            unreachable!("sprite calls resolve their template objects")
                        }
                    }
                }
            }
        }
        for (index, object) in objects.iter().enumerate().take(self.count) {
            let node = &self.nodes[index];
            if node.events.is_empty() && node.control.sprite.is_none() {
                continue;
            }
            let kept = self.retained(index, object.element, resources);
            let played = kept.played;
            let replaced = kept.made != node.made;
            let node = &mut self.nodes[index];
            node.played = played;
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
            if replaced {
                node.runtime = None;
                let candidate = clock_candidate(object.element, node.made.as_ref(), resources);
                if candidate {
                    if !self.clock_nodes.contains(&index) {
                        self.clock_nodes.push(index);
                    }
                } else {
                    self.clock_nodes.retain(|&own| own != index);
                }
            }
        }
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
        let retained = self.tree.same(root);
        let clocks = if retained {
            self.clock_nodes.len()
        } else {
            objects.len()
        };
        for at in 0..clocks {
            let index = if retained { self.clock_nodes[at] } else { at };
            let object = &objects[index];
            let shown = self.drawable_shown(index);
            let node = &mut self.nodes[index];
            let Some((sprite, info)) = drawn(
                object.element,
                node.made.as_ref(),
                node.control.text.as_deref(),
                resources,
            ) else {
                node.runtime = None;
                continue;
            };
            let made = node.made.as_ref().map_or(0, |made| made.made);
            let runtime = Self::runtime(&mut node.runtime, sprite, made);
            if matches!(info.kind, SpriteKind::FrameAnimated { .. }) {
                let clock = runtime.clock.get_or_insert_with(|| Clock {
                    clock: FrameClock::new(),
                    kind: info.kind.clone(),
                });
                clock.kind = info.kind.clone();
                if node.played {
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

    fn runtime<'a>(current: &'a mut Option<Runtime>, sprite: &[u8], made: u32) -> &'a mut Runtime {
        if !current
            .as_ref()
            .is_some_and(|runtime| runtime.is(sprite, made))
        {
            *current = Some(Runtime::new(sprite.to_vec(), made));
        }
        current.as_mut().expect("set above")
    }

    pub fn tick_transitions(&mut self, root: &Element) -> Result<(), String> {
        self.check(root)?;
        self.active_known.set(None);
        let settle = std::mem::take(&mut self.settle);
        if self.phase != Phase::Clocked {
            return Err(format!(
                "the transitions of {} were run before its clocks for the frame: run every \
                 drawable clock before window transitions; call Instance::tick_clocks first",
                self.template
            ));
        }
        if self.registered.iter().all(|&index| {
            let window = self.nodes[index].window.expect("a window node");
            window.fade == 0 && window.done && window.mode == 0
        }) {
            self.phase = Phase::Moved;
            return Ok(());
        }
        self.revision += 1;
        let elements = self.tree.elements(root);
        let dt = if settle { f32::INFINITY } else { self.dt };
        for at in (0..self.registered.len()).rev() {
            let index = self.registered[at];
            self.update_fade(&elements, index, dt)?;
            self.update_movement(&elements, index, dt)?;
            if settle {
                self.update_movement(&elements, index, dt)?;
            }
        }
        self.phase = Phase::Moved;
        Ok(())
    }

    pub fn tick_draws(&mut self, root: &Element, resources: &dyn Resources) -> Result<(), String> {
        self.check(root)?;
        self.active_known.set(None);
        if self.phase != Phase::Moved {
            return Err(format!(
                "the draw of {} was recorded before its transitions for the frame: call \
                 Instance::tick_transitions first",
                self.template
            ));
        }
        if self.idle_tick && !self.written {
            self.phase = Phase::Drawn;
            return Ok(());
        }
        self.revision += 1;
        if !self.nodes[0].shown.visible
            && root.kind != Kind::Scrollbar
            && self.tree.same(root)
            && self.nodes.len() == self.count
            && self.text_boxes.iter().all(|&index| {
                let node = &self.nodes[index];
                !matches!(&node.drawables, Drawables::TextBox { written, .. }
                    if node.control.text != *written)
            })
        {
            self.ticked = true;
            self.written = false;
            self.phase = Phase::Drawn;
            self.kept = false;
            return Ok(());
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
            let state = self.state(object.element, index);
            let node = &mut self.nodes[index];
            let Some((sprite, info)) = drawn(
                object.element,
                node.made.as_ref(),
                node.control.text.as_deref(),
                resources,
            ) else {
                continue;
            };
            let made = node.made.as_ref().map_or(0, |made| made.made);
            let records = if info.animated {
                resources.sprite_animations(sprite).unwrap_or_default()
            } else {
                Vec::new()
            };
            let settles = match &info.effect {
                Ok(effect) => effect.settles(technique(state)),
                Err(_) => Settles::Program(Some(Settling::NOW)),
            };
            let runtime = Self::runtime(&mut node.runtime, sprite, made);
            let first = *runtime.first_draw.get_or_insert(time);
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
        self.written = false;
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

    fn kept_changes(&self, root: &Element, resources: &dyn Resources) -> bool {
        let layout = match self.layout(root, resources, None) {
            Ok(Some(layout)) => layout,
            Ok(None) => return false,
            Err(_) => return true,
        };
        let mut bound: Option<Option<Settling>> = None;
        for (index, part) in layout.parts.iter().enumerate() {
            if layout.holders.iter().any(|holder| holder.before == index) {
                bound = None;
            }
            match part.shading.settles() {
                Settles::Program(settles) => bound = Some(settles),
                Settles::Previous if !part.shading.clocked => {}
                Settles::Previous => match bound {
                    Some(Some(settles))
                        if settled(
                            &settles,
                            part.shading.since,
                            part.shading.time.abs().max(part.shading.animation.abs()),
                        ) => {}
                    _ => return true,
                },
                Settles::Refused => bound = Some(Some(Settling::NOW)),
            }
        }
        false
    }

    pub fn written(&self) -> bool {
        self.written
    }

    pub fn idle(&self) -> bool {
        self.ticked && !self.written && !self.active()
    }

    pub fn transitioning(&self) -> bool {
        self.registered.iter().any(|&index| {
            let node = &self.nodes[index];
            let window = node.window.expect("a window node");
            window.fade != 0 || !window.done || (window.mode == 2 && node.shown.visible)
        })
    }

    pub fn longest_transition(&self, root: &Element) -> Result<f32, String> {
        self.check(root)?;
        let elements = self.tree.elements(root);
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

    pub fn active(&self) -> bool {
        if let Some(active) = self.active_known.get() {
            return active;
        }
        let active = self.asked_active();
        self.active_known.set(Some(active));
        active
    }

    fn asked_active(&self) -> bool {
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
                                    Settles::Program(settles) => settles.is_none_or(|settles| {
                                        !settled(
                                            &settles,
                                            self.time - effect.at,
                                            self.time.abs().max(effect.at.abs()),
                                        )
                                    }),
                                    Settles::Previous | Settles::Refused => false,
                                }))
                    })
            })
            || self.kept
    }

    pub fn bindings(&self, root: &Element, resources: &dyn Resources) -> Result<Bindings, String> {
        let empty = Bindings {
            viewport: Some(self.viewport),
            ..Bindings::default()
        };
        let objects = self.objects(root, resources, &empty)?;
        let mut bindings = Bindings {
            bound: HashMap::with_capacity(objects.len()),
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
            bound.animation_time = Some(
                runtime
                    .and_then(|runtime| runtime.first_draw)
                    .map_or(0.0, |first| self.time - first),
            );
            let state = self.state(element, index);
            bound.effect_since = Some(match runtime.and_then(|runtime| runtime.effect) {
                Some(effect) if effect.state == state => self.time - effect.at,
                _ => 0.0,
            });
            bound.effect_state = Some(state);
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

    pub fn layout(
        &self,
        root: &Element,
        resources: &dyn Resources,
        pointer: Option<Pointer>,
    ) -> Result<Option<Layout>, Refusals> {
        self.layout_with_regions(root, resources, pointer, false)
    }

    pub fn layout_with_regions(
        &self,
        root: &Element,
        resources: &dyn Resources,
        pointer: Option<Pointer>,
        regions: bool,
    ) -> Result<Option<Layout>, Refusals> {
        let _profile = viewer_runtime::profiling::span("dc_layout");
        let mut bindings = self
            .bindings(root, resources)
            .map_err(|why| vec![Refusal::new(&root.path, RefusalKind::MissingInput, why)])?;
        bindings.pointer = pointer;
        bindings.regions = regions;
        if root.kind == Kind::Window {
            return super::fixed_window_layout(root, resources, &bindings);
        }
        let parent = self.nodes[0]
            .window
            .map_or(self.viewport, |window| window.parent);
        layout(root, Some(parent), resources, &bindings)
    }

    pub fn layout_kept(
        &self,
        root: &Element,
        resources: &dyn Resources,
        regions: bool,
        stamp: u64,
    ) -> Result<Option<Layout>, Refusals> {
        let mut laid = self.laid.borrow_mut();
        if let Some(laid) = laid.as_ref()
            && laid.revision == self.revision
            && laid.root == std::ptr::from_ref(root) as usize
            && laid.stamp == stamp
            && laid.regions == regions
        {
            return laid.layout.clone();
        }
        let layout = self.layout_with_regions(root, resources, None, regions);
        let root = std::ptr::from_ref(root) as usize;
        *laid = Some(Laid { revision: self.revision, root, stamp, regions, layout: layout.clone() });
        layout
    }

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

fn anchored(
    window: &Element,
    container: &native::Container,
    position: &native::Position,
    parent: [i32; 2],
    size: [i32; 2],
    what: &str,
) -> Result<[i32; 2], String> {
    let at = evaluate_position(
        position,
        Some(parent),
        format_args!("{}.{what}", window.path),
    )?;
    let anchor = compute_anchor(container.orientation, container.origo, parent, size);
    Ok([at[0] + anchor[0], at[1] + anchor[1]])
}

pub struct Pool<K> {
    template: String,
    viewport: [i32; 2],
    growth: usize,
    slots: Vec<Slot>,
    cursor: usize,
    live: Vec<(SlotId, K)>,
    owners: HashMap<K, SlotId>,
    update_buffer: Vec<(SlotId, K)>,
    growing: bool,
    missed: usize,
    retrying: usize,
    grown: Vec<usize>,
    growths: u64,
    setup: Setup,
    withheld: Withheld<SlotId>,
}

pub(crate) type Setup = Box<dyn Fn(&mut Instance, &Element) -> Result<(), String> + Send + Sync>;

struct Slot {
    instance: Instance,
    live: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapIconPool {
    AirBase,
    AirMission,
    RocketSite,
    GunEmplacement,
    Radar,
    StrategicLocation,
    Base,
    Combat,
    CombatResults,
    Mission,
    Label,
    Capital,
    Raid,
    Ping,
    Facility,
    Headquarter,
    LandCombat,
    AdjacencyRule,
    Resource,
    Occupation,
}

fn build_matches_verified(build: &str, version: &str, checksum: &str) -> bool {
    let candidate = build
        .rsplit_once(" v")
        .map(|(_, rest)| rest)
        .unwrap_or(build)
        .trim();
    let candidate = candidate.strip_prefix('v').unwrap_or(candidate);
    candidate
        .strip_prefix(version)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|tag| tag.eq_ignore_ascii_case(checksum))
}

impl MapIconPool {
    pub const VERIFIED_BUILDS: &'static [(&'static str, &'static str)] = &[
        ("1.19.5.0", "c184"),
        ("1.19.4.2", "0bdb"),
        ("1.19.4.1", "f584"),
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::AirBase => "air base",
            Self::AirMission => "air mission",
            Self::RocketSite => "rocket site",
            Self::GunEmplacement => "gun emplacement",
            Self::Radar => "radar",
            Self::StrategicLocation => "strategic location",
            Self::Base => "naval base",
            Self::Combat => "naval combat",
            Self::CombatResults => "naval combat results",
            Self::Mission => "naval mission",
            Self::Label => "map label",
            Self::Capital => "capital",
            Self::Raid => "raid",
            Self::Ping => "faction ping",
            Self::Facility => "facility",
            Self::Headquarter => "headquarter",
            Self::LandCombat => "land combat",
            Self::AdjacencyRule => "adjacency rule",
            Self::Resource => "resource",
            Self::Occupation => "occupation",
        }
    }

    pub fn initial(self) -> usize {
        match self {
            Self::Label => 0xA0,
            Self::Occupation => 0x60,
            Self::AirBase | Self::AirMission | Self::Base | Self::Capital | Self::Resource => 0xC0,
            Self::Combat => 0x20,
            Self::CombatResults
            | Self::Mission
            | Self::Headquarter
            | Self::Radar
            | Self::StrategicLocation
            | Self::AdjacencyRule => 0x40,
            Self::Raid
            | Self::Ping
            | Self::Facility
            | Self::LandCombat
            | Self::RocketSite
            | Self::GunEmplacement => 0x80,
        }
    }

    pub fn initial_for_build(self, build: Option<&str>) -> Result<usize, String> {
        let Some(build) = build.map(str::trim).filter(|build| !build.is_empty()) else {
            return Err(format!(
                "{} map icon pool initial capacity is unknown: game build is unverified",
                self.name()
            ));
        };
        if Self::VERIFIED_BUILDS
            .iter()
            .any(|&(version, checksum)| build_matches_verified(build, version, checksum))
        {
            Ok(self.initial())
        } else {
            Err(format!(
                "{} map icon pool initial capacity is unknown for game build '{build}'",
                self.name()
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(usize);

impl SlotId {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Empty,
    Outside,
    Inside([i32; 2]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Erased<K> {
    pub owner: K,
    pub slot: SlotId,
    pub held: Vec<Held>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pass<K> {
    pub updated: Vec<(K, SlotId)>,
    pub erased: Vec<Erased<K>>,
}

impl<K: Clone + Eq + std::hash::Hash> Pool<K> {
    pub fn new(
        template: &Element,
        viewport: [i32; 2],
        kind: MapIconPool,
        setup: Setup,
        resources: &dyn Resources,
    ) -> Result<Self, String> {
        Self::with_initial(template, viewport, kind.initial(), setup, resources)
    }

    pub fn new_for_build(
        template: &Element,
        viewport: [i32; 2],
        kind: MapIconPool,
        build: Option<&str>,
        setup: Setup,
        resources: &dyn Resources,
    ) -> Result<Self, String> {
        let initial = kind.initial_for_build(build)?;
        Self::with_initial(template, viewport, initial, setup, resources)
    }

    fn with_initial(
        template: &Element,
        viewport: [i32; 2],
        initial: usize,
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
            owners: HashMap::new(),
            update_buffer: Vec::new(),
            growing: false,
            missed: 0,
            retrying: 0,
            grown: Vec::new(),
            growths: 0,
            setup,
            withheld: Withheld::default(),
        };
        pool.add(template, initial, resources)?;
        Ok(pool)
    }

    fn grows_by(&self) -> usize {
        self.growth.max(self.retrying)
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
            instance.take_held();
            self.slots.push(Slot {
                instance,
                live: None,
            });
        }
        Ok(())
    }

    pub fn slot_of(&self, owner: &K) -> Option<SlotId> {
        self.owners.get(owner).copied()
    }

    pub fn request(&mut self, owner: K) -> Option<SlotId> {
        if let Some(slot) = self.slot_of(&owner) {
            return Some(slot);
        }
        let count = self.slots.len();
        let mut at = self.cursor;
        for _ in 0..count {
            if self.slots[at].live.is_none() {
                self.slots[at].live = Some(self.live.len());
                self.cursor = if at + 1 >= count { 0 } else { at + 1 };
                self.owners.insert(owner.clone(), SlotId(at));
                self.live.push((SlotId(at), owner));
                return Some(SlotId(at));
            }
            at = if at + 1 < count { at + 1 } else { 0 };
        }
        self.growing = true;
        self.missed += 1;
        None
    }

    fn free(&mut self, template: &Element, slot: SlotId) -> Result<Vec<Held>, String> {
        if let Some(at) = self.slots[slot.0].live.take() {
            let (_, owner) = self.live.swap_remove(at);
            self.owners.remove(&owner);
            if let Some((moved, _)) = self.live.get(at) {
                self.slots[moved.0].live = Some(at);
            }
        }
        let entry = &mut self.slots[slot.0];
        if entry.instance.drawn() {
            entry.instance.hide(template, template)?;
        }
        Ok(entry.instance.take_held())
    }

    pub fn erase(&mut self, template: &Element, owner: &K) -> Result<Vec<Held>, String> {
        match self.slot_of(owner) {
            Some(slot) => self.free(template, slot),
            None => Ok(Vec::new()),
        }
    }

    pub fn update(
        &mut self,
        template: &Element,
        mut test: impl FnMut(&K, &Instance) -> Admission,
    ) -> Result<Pass<K>, String> {
        let mut pass = Pass {
            updated: Vec::new(),
            erased: Vec::new(),
        };
        let mut buffer = std::mem::take(&mut self.update_buffer);
        buffer.clone_from(&self.live);
        for index in 0..buffer.len() {
            let (slot, owner) = buffer[index].clone();
            match test(&owner, &self.slots[slot.0].instance) {
                Admission::Empty | Admission::Outside => {
                    let live = self.live.len();
                    if let Some(at) = self.slots[slot.0].live {
                        buffer[at] = buffer[live - 1].clone();
                    }
                    let held = self.free(template, slot)?;
                    pass.erased.push(Erased { owner, slot, held });
                }
                Admission::Inside(point) => {
                    let instance = &mut self.slots[slot.0].instance;
                    if instance.position() != point {
                        instance.set_position(template, template, point)?;
                    }
                    if !instance.drawn() {
                        instance.gui_show(template, template)?;
                    }
                    pass.updated.push((owner, slot));
                }
            }
        }
        self.update_buffer = buffer;
        Ok(pass)
    }

    pub fn instance(&self, slot: SlotId) -> Option<&Instance> {
        self.slots.get(slot.0).map(|entry| &entry.instance)
    }

    pub fn get(&mut self, slot: SlotId) -> Option<&mut Instance> {
        self.slots
            .get_mut(slot.0)
            .filter(|entry| entry.live.is_some())
            .map(|entry| &mut entry.instance)
    }

    pub fn live(&self) -> impl Iterator<Item = (&K, SlotId, &Instance)> {
        self.live
            .iter()
            .map(|(slot, owner)| (owner, *slot, &self.slots[slot.0].instance))
    }

    pub fn tick(
        &mut self,
        template: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        let _profile = viewer_runtime::profiling::span("dc_pool_tick");
        self.tick_clocks(template, resources, tick)?;
        self.tick_transitions(template)?;
        self.tick_draws(template, resources)
    }

    pub fn tick_slots(
        &mut self,
        template: &Element,
        resources: &dyn Resources,
        tick: Tick,
        slots: &mut Vec<SlotId>,
    ) -> Result<(), String> {
        let _profile = viewer_runtime::profiling::span("dc_pool_tick");
        self.retrying = std::mem::take(&mut self.missed);
        if std::mem::take(&mut self.growing) {
            let from = self.slots.len();
            let built = self.grows_by();
            self.add(template, built, resources)?;
            self.growths += 1;
            self.grown.push(built);
            slots.extend((from..self.slots.len()).map(SlotId));
        }
        for slot in slots.iter() {
            self.slots[slot.0]
                .instance
                .tick_clocks(template, resources, tick)?;
        }
        for slot in slots.iter().rev() {
            self.slots[slot.0].instance.tick_transitions(template)?;
        }
        for slot in slots.iter() {
            self.slots[slot.0].instance.tick_draws(template, resources)?;
        }
        Ok(())
    }

    pub fn active_slots(&self) -> impl Iterator<Item = SlotId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.instance.active())
            .map(|(index, _)| SlotId(index))
    }

    pub fn tick_clocks(
        &mut self,
        template: &Element,
        resources: &dyn Resources,
        tick: Tick,
    ) -> Result<(), String> {
        self.retrying = std::mem::take(&mut self.missed);
        if std::mem::take(&mut self.growing) {
            let built = self.grows_by();
            self.add(template, built, resources)?;
            self.growths += 1;
            self.grown.push(built);
        }
        for slot in &mut self.slots {
            slot.instance.tick_clocks(template, resources, tick)?;
        }
        Ok(())
    }

    pub fn tick_transitions(&mut self, template: &Element) -> Result<(), String> {
        for index in (0..self.slots.len()).rev() {
            self.tick_transitions_of(template, SlotId(index))?;
        }
        Ok(())
    }

    pub fn registrations(&self) -> impl Iterator<Item = (Registration, SlotId)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.instance.registration(), SlotId(index)))
    }

    pub fn tick_transitions_of(&mut self, template: &Element, slot: SlotId) -> Result<(), String> {
        let entry = self
            .slots
            .get_mut(slot.0)
            .ok_or_else(|| format!("the pool of {} has no slot {}", self.template, slot.0))?;
        entry.instance.tick_transitions(template)
    }

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

    pub fn tick_due(&self) -> bool {
        self.growing || self.slots.iter().any(|slot| !slot.instance.idle())
    }

    pub fn requests_pending(&self) -> bool {
        self.growing || self.missed > 0
    }

    pub fn active(&self) -> bool {
        self.growing || self.slots.iter().any(|slot| slot.instance.active())
    }

    pub fn retry(&self) -> Option<super::super::icon_windows::Retry> {
        (self.retrying != 0 || self.missed != 0).then(|| super::super::icon_windows::Retry {
            name: self.template.clone(),
            requests: self.retrying.max(self.missed),
            rounds: 1,
        })
    }

    pub fn take_held(&mut self, slot: SlotId) -> Vec<Held> {
        self.slots
            .get_mut(slot.0)
            .map_or(Vec::new(), |entry| entry.instance.take_held())
    }

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

    #[cfg(any(test, feature = "test-support"))]
    pub fn growths(&self) -> u64 {
        self.growths
    }

    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.grown)
            .into_iter()
            .map(|built| Diagnostic {
                path: self.template.clone(),
                kind: DiagnosticKind::PoolGrowth,
                detail: format!(
                    "the pool of {} was exhausted and grew by {built} windows on a worker task, \
                     whose completion frame the recording does not hold: taken as the next \
                     frame",
                    self.template
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

#[derive(Debug)]
pub struct Visible<'a, K> {
    pub owner: &'a K,
    pub slot: SlotId,
    pub instance: &'a Instance,
    pub withheld: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Unknown {
    Update,
    Part(&'static str),
    UntilRebuilt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rewrote {
    Update,
    Part(&'static str),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Refused {
    why: Cow<'static, str>,
    unknown: Vec<Unknown>,
}

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
    pub fn refuse(&mut self, key: K, unknown: Unknown, why: Cow<'static, str>) {
        let refused = self.refused.entry(key).or_default();
        refused.why = why;
        if !refused.unknown.contains(&unknown) {
            refused.unknown.push(unknown);
        }
    }

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

    pub fn why(&self, key: &K) -> Option<&str> {
        self.refused.get(key).map(|refused| refused.why.as_ref())
    }
}

impl<K: Clone + Eq + std::hash::Hash> Pool<K> {
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

    pub fn refuse(&mut self, slot: SlotId, unknown: Unknown, why: Cow<'static, str>) {
        self.withheld.refuse(slot, unknown, why);
    }

    pub fn known(&mut self, slot: SlotId, part: &'static str) {
        self.withheld.known(&slot, part);
    }

    pub fn rewrote(&mut self, slot: SlotId, rewrote: Rewrote) {
        self.withheld.rewrote(&slot, rewrote);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn withheld(&self, slot: SlotId) -> Option<&str> {
        self.withheld.why(&slot)
    }
}

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
    SetSprite(Vec<u8>, SpriteSetter),
    SetText(String),
    #[cfg(any(test, feature = "test-support"))]
    Play,
    #[cfg(any(test, feature = "test-support"))]
    SetOpacity(f32, bool),
    Control(Box<Bound>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calls<K> {
    pub calls: Vec<(K, String, Call)>,
}

impl<K: Clone + Eq + std::hash::Hash> Calls<K> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn push(&mut self, key: K, element: &Element, call: Call) {
        self.calls.push((key, element.path.clone(), call));
    }

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
            call.apply(root, element, instance, resources)?;
        }
        Ok(())
    }
}

impl Call {
    fn apply(
        &self,
        root: &Element,
        element: &Element,
        instance: &mut Instance,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        self.apply_with_elements(root, element, instance, resources, &mut CallElements::default())
    }

    fn apply_with_elements<'a>(
        &self,
        root: &'a Element,
        element: &Element,
        instance: &mut Instance,
        resources: &dyn Resources,
        authored: &mut CallElements<'a>,
    ) -> Result<(), String> {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            Call::SetPosition(point) => instance.set_position(root, element, *point)?,
            Call::Show => instance.show_using(root, element, authored)?,
            #[cfg(any(test, feature = "test-support"))]
            Call::GuiShow => instance.gui_show_using(root, element, authored)?,
            Call::Hide => instance.hide_using(root, element, authored)?,
            #[cfg(any(test, feature = "test-support"))]
            Call::HideWindow => instance.hide_window_using(root, element, authored)?,
            Call::SetHideFlag(hide) => instance.set_hide_flag(root, element, *hide)?,
            Call::SetFrame(frame) => instance.set_frame(root, element, *frame)?,
            Call::SetSprite(name, setter) => instance.set_sprite(root, element, name, *setter)?,
            Call::SetText(text) => instance.set_text_using(root, element, text, resources, authored)?,
            #[cfg(any(test, feature = "test-support"))]
            Call::Play => instance.play(root, element)?,
            #[cfg(any(test, feature = "test-support"))]
            Call::SetOpacity(value, children) => {
                instance.set_opacity(root, element, *value, *children)?
            }
            Call::Control(bound) => {
                let control = instance.control(root, element)?;
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
                    instance.set_text_using(root, element, &text, resources, authored)?;
                }
            }
        }
        Ok(())
    }
}

impl Instance {
    pub fn apply_calls<'a>(
        &mut self,
        root: &Element,
        calls: impl IntoIterator<Item = (&'a str, &'a Call)>,
        resources: &dyn Resources,
    ) -> Result<(), String> {
        let mut calls = calls.into_iter().peekable();
        if calls.peek().is_none() {
            return Ok(());
        }
        let copy_profile = viewer_runtime::profiling::span("dc_nu_tx_copy");
        let mut next = self.clone();
        drop(copy_profile);
        let setters_profile = viewer_runtime::profiling::span("dc_nu_tx_setters");
        let mut authored = CallElements::default();
        for (path, call) in calls {
            let element = self.element_at_path(root, path)?;
            call.apply_with_elements(root, element, &mut next, resources, &mut authored)?;
        }
        drop(setters_profile);
        let _commit_profile = viewer_runtime::profiling::span("dc_nu_tx_commit");
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod retained_work_tests {
    use super::*;
    use crate::test_support::gui::fixture;

    fn template() -> native::Gui {
        content_reader_script::test_game::enter();
        native::read(
            br#"guiTypes={containerWindowType={name=root size={width=100 height=50}
            iconType={name=icon spriteType=GFX_animated}
            instantTextBoxType={name=text font=Main_14 text=Initial}}}"#,
        )
    }

    #[test]
    fn retained_paths_keep_first_occurrences_parts_and_copied_template_changes() {
        let _setup = template();
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root size={width=100 height=50}
            background={name=back}
            containerWindowType={name=child iconType={name=icon spriteType=GFX_even}}
            iconType={name=repeated spriteType=GFX_even}
            iconType={name=repeated spriteType=GFX_odd}}}"#,
        );
        let root = gui.template("root").unwrap();
        let resources = fixture();
        let instance = Instance::construct(root, [800, 600], &resources).unwrap();
        let check = |root: &Element| {
            let authored = elements(root);
            for (element, ..) in &authored {
                let expected = authored
                    .iter()
                    .find(|(own, ..)| own.path == element.path)
                    .unwrap()
                    .0;
                assert!(std::ptr::eq(
                    instance.element_at_path(root, &element.path).unwrap(),
                    expected
                ));
            }
            assert!(instance.element_at_path(root, "root.missing").is_err());
        };
        check(root);
        let mut copied = root.clone();
        copied.children[0].children[0].path = "renamed child".into();
        check(&copied);
        assert!(instance.element_at_path(&copied, "renamed child").is_ok());
        assert!(instance.element_at_path(root, "renamed child").is_err());
    }

    #[test]
    fn borrowed_single_window_calls_keep_order_and_roll_back_failed_streams() {
        let gui = template();
        let root = gui.template("root").unwrap();
        let resources = fixture();
        let mut actual = Instance::construct(root, [800, 600], &resources).unwrap();
        let icon = root.path_named("icon").unwrap();
        let text = root.path_named("text").unwrap();
        let calls = Calls {
            calls: vec![
                ((), root.path.clone(), Call::GuiShow),
                ((), icon.path.clone(), Call::SetFrame(3)),
                (
                    (),
                    icon.path.clone(),
                    Call::SetSprite(b"GFX_even".to_vec(), SpriteSetter::Icon),
                ),
                ((), icon.path.clone(), Call::SetFrame(2)),
                (
                    (),
                    icon.path.clone(),
                    Call::SetSprite(b"undefined".to_vec(), SpriteSetter::Icon),
                ),
                ((), text.path.clone(), Call::SetText("Changed".into())),
            ],
        };
        let mut expected = HashMap::from([((), actual.clone())]);
        calls.apply(root, &mut expected, &resources).unwrap();
        actual
            .apply_calls(
                root,
                calls
                    .calls
                    .iter()
                    .map(|(_, path, call)| (path.as_str(), call)),
                &resources,
            )
            .unwrap();
        let expected = expected.get_mut(&()).unwrap();
        for frame in 0..3 {
            let tick = Tick {
                time: frame as f32 * 0.25,
                dt: 0.25,
            };
            actual.tick(root, &resources, tick).unwrap();
            expected.tick(root, &resources, tick).unwrap();
            assert_eq!(
                actual.bindings(root, &resources).unwrap().bound,
                expected.bindings(root, &resources).unwrap().bound
            );
            assert_eq!(
                actual
                    .layout_with_regions(root, &resources, None, true)
                    .unwrap(),
                expected
                    .layout_with_regions(root, &resources, None, true)
                    .unwrap()
            );
        }
        let before = actual.bindings(root, &resources).unwrap().bound;
        let failed = [
            (icon.path.as_str(), Call::SetFrame(9)),
            ("root.missing", Call::Hide),
        ];
        let why = actual
            .apply_calls(
                root,
                failed.iter().map(|(path, call)| (*path, call)),
                &resources,
            )
            .unwrap_err();
        assert!(why.contains("root.missing"), "{why}");
        assert_eq!(actual.bindings(root, &resources).unwrap().bound, before);
        assert_eq!(actual.phase, Phase::Drawn);
    }

    #[test]
    fn transaction_elements_are_lazy_and_shared_only_within_the_call_stream() {
        let gui = template();
        let root = gui.template("root").unwrap();
        let resources = fixture();
        let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
        let icon = root.path_named("icon").unwrap();
        let mut authored = CallElements::default();
        for call in [Call::SetFrame(2), Call::SetHideFlag(false)] {
            call.apply_with_elements(root, icon, &mut instance, &resources, &mut authored)
                .unwrap();
        }
        assert_eq!(authored.builds, 0);
        for call in [Call::Show, Call::Hide, Call::Show, Call::Hide] {
            call.apply_with_elements(root, icon, &mut instance, &resources, &mut authored)
                .unwrap();
        }
        assert_eq!(authored.builds, 1);
        let copied = root.clone();
        let mut later = CallElements::default();
        Call::GuiShow
            .apply_with_elements(&copied, &copied, &mut instance, &resources, &mut later)
            .unwrap();
        assert_eq!(later.builds, 1);
    }

    #[test]
    fn transaction_elements_keep_transactional_lifecycle_requests_and_output_across_frames() {
        use crate::test_support::gui::Fixture;
        use std::cell::{Cell, RefCell};

        struct Requested<'a> {
            fixture: &'a Fixture,
            published: &'a Cell<bool>,
            log: RefCell<Vec<String>>,
        }
        impl Requested<'_> {
            fn note(&self, kind: &str, name: impl std::fmt::Debug) {
                self.log.borrow_mut().push(format!("{kind}:{name:?}"));
            }
        }
        impl Resources for Requested<'_> {
            fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
                self.note("sprite", name);
                if name == b"GFX_late" {
                    return self
                        .published
                        .get()
                        .then(|| self.fixture.sprite(b"GFX_odd"))
                        .flatten();
                }
                self.fixture.sprite(name)
            }
            fn sprite_info(&self, name: &[u8]) -> Option<Cow<'_, SpriteInfo>> {
                self.note("sprite_info", name);
                self.sprite(name).map(Cow::Owned)
            }
            fn portrait_texture(&self, name: &[u8]) -> Option<Texture> {
                self.note("portrait", name);
                self.fixture.portrait_texture(name)
            }
            fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
                self.note("font", name);
                self.fixture.font(name)
            }
            fn localisation(&self) -> &Localisation {
                self.fixture.localisation()
            }
            fn text_colors(&self) -> &TextColors {
                self.fixture.text_colors()
            }
            fn text_effect(&self) -> Result<Effect, String> {
                self.fixture.text_effect()
            }
            fn effect(&self, file: &str) -> Result<Effect, String> {
                self.fixture.effect(file)
            }
            fn shield_flag(
                &self,
                small: bool,
                index: i32,
            ) -> Result<Option<(Texture, [f32; 4])>, String> {
                self.note("shield", (small, index));
                self.fixture.shield_flag(small, index)
            }
            fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
                self.note("flag", tag);
                self.fixture.text_flag(tag)
            }
            fn resolved_text_flag(&self, key: &[u8]) -> Result<Option<Texture>, String> {
                self.note("resolved_flag", key);
                self.fixture.resolved_text_flag(key)
            }
            fn resolved_text_flag_key(&self, text: &[char]) -> Option<usize> {
                self.fixture.resolved_text_flag_key(text)
            }
            fn gui_type(&self, name: &[u8]) -> Option<&Element> {
                self.note("gui", name);
                self.fixture.gui_type(name)
            }
            fn session_text_sprite(&self, name: &[u8]) -> bool {
                self.fixture.session_text_sprite(name)
            }
            fn sprite_animations(&self, name: &[u8]) -> Option<Vec<AnimationRecord>> {
                self.note("animations", name);
                self.fixture.sprite_animations(name)
            }
        }
        fn output_bits(batch: &Batch) -> Vec<u8> {
            let mut bytes = Vec::new();
            for quad in &batch.quads {
                bytes.push(u8::from(quad.rotation.is_some()));
                for value in quad
                    .rect
                    .into_iter()
                    .chain(quad.uv)
                    .chain(quad.color)
                    .chain(quad.rotation)
                    .chain([quad.shade])
                {
                    bytes.extend(value.to_ne_bytes());
                }
                if let Some(exact) = &quad.exact {
                    assert!(exact.native.is_none(), "this oracle uses synthetic effects");
                    for value in exact
                        .corners
                        .into_iter()
                        .flatten()
                        .chain(exact.uv.into_iter().flatten())
                        .chain(exact.lod_bias)
                        .chain(exact.offsets)
                        .chain(exact.inputs)
                        .chain(exact.second_color)
                    {
                        bytes.extend(value.to_ne_bytes());
                    }
                }
            }
            bytes
        }
        let _metadata = template();
        const GUI: &[u8] = br#"guiTypes={
            scrollbarType={name=bar size={x=10 y=60} minValue=0 maxValue=1
                slider=s track=t leftButton=d rightButton=u
                buttonType={name=u spriteType=GFX_four}
                buttonType={name=d spriteType=GFX_four}
                guiButtonType={name=t spriteType=GFX_bar3x17}
                guiButtonType={name=s spriteType=GFX_bar3x17}}
            containerWindowType={name=root size={width=160 height=90}
                fade_type=linear fade_time=150
                show_position={x=10 y=5} show_animation_type=linear show_animation_time=150
                hide_position={x=-10 y=0} hide_animation_type=linear hide_animation_time=100
                containerWindowType={name=child size={width=100 height=60}
                    gridBoxType={name=held size={width=40 height=20} slotsize={width=20 height=10}}
                    iconType={name=icon spriteType=GFX_animated}
                    instantTextBoxType={name=text font=Main_14 text=Initial maxWidth=50 maxHeight=20 scrollbarType=bar}}}}"#;
        let gui = native::read(GUI);
        let root = gui.template("root").unwrap();
        let copied = root.clone();
        let mut fixture = fixture();
        fixture.templates.push(native::read(GUI));
        let published = Cell::new(false);
        let expected_resources = Requested {
            fixture: &fixture,
            published: &published,
            log: RefCell::default(),
        };
        let actual_resources = Requested {
            fixture: &fixture,
            published: &published,
            log: RefCell::default(),
        };
        let mut expected = Instance::construct(root, [800, 600], &expected_resources).unwrap();
        let mut actual = expected.clone();
        assert!(
            !expected.text_boxes.is_empty(),
            "exercise constructed external scrollbar parts"
        );
        let mut saw_quads = false;
        let mut saw_flag = false;
        for frame in 0..40 {
            published.set(frame >= 17);
            let template = if frame % 7 == 3 { &copied } else { root };
            let child = template.path_named("child").unwrap();
            let icon = child.path_named("icon").unwrap();
            let text = child.path_named("text").unwrap();
            let mut calls = vec![
                (icon.path.as_str(), Call::SetFrame(frame % 3 + 1)),
                (
                    icon.path.as_str(),
                    Call::SetSprite(b"GFX_late".to_vec(), SpriteSetter::Icon),
                ),
                (
                    text.path.as_str(),
                    Call::SetText(
                        if frame % 2 == 0 {
                            "aaaa bbbb cccc\n\u{12}GER"
                        } else {
                            "a"
                        }
                        .into(),
                    ),
                ),
                (
                    text.path.as_str(),
                    Call::Control(Box::new(Bound {
                        text: Some("\u{12}GER long text long text".into()),
                        color: Some([0.2, 0.3, 0.4, 0.8]),
                        ..Bound::default()
                    })),
                ),
                (icon.path.as_str(), Call::SetHideFlag(frame % 6 == 1)),
                (icon.path.as_str(), Call::Show),
            ];
            if frame % 8 == 0 {
                calls.push((template.path.as_str(), Call::GuiShow));
            }
            if frame % 8 == 2 {
                calls.push((child.path.as_str(), Call::Hide));
            }
            if frame % 8 == 3 {
                calls.push((child.path.as_str(), Call::GuiShow));
            }
            if frame % 8 == 4 {
                calls.push((template.path.as_str(), Call::HideWindow));
            }
            if frame % 8 == 6 {
                calls.push((template.path.as_str(), Call::Hide));
            }
            let fail = frame == 5 || frame == 23;
            if fail {
                calls.push(("root.missing", Call::Hide));
            }
            expected_resources.log.borrow_mut().clear();
            actual_resources.log.borrow_mut().clear();
            let before = format!("{expected:?}");
            let mut staged = HashMap::from([((), expected.clone())]);
            let reference = Calls {
                calls: calls
                    .iter()
                    .map(|(path, call)| ((), (*path).to_owned(), call.clone()))
                    .collect(),
            };
            let old_result = reference.apply(template, &mut staged, &expected_resources);
            if old_result.is_ok() {
                expected = staged.remove(&()).unwrap();
            }
            let new_result = actual.apply_calls(
                template,
                calls.iter().map(|(path, call)| (*path, call)),
                &actual_resources,
            );
            assert_eq!(old_result, new_result, "frame {frame}");
            assert_eq!(old_result.is_err(), fail, "frame {frame}");
            if fail {
                assert_eq!(
                    format!("{expected:?}"),
                    before,
                    "failed transaction changed expected state"
                );
            }
            assert_eq!(
                format!("{expected:?}"),
                format!("{actual:?}"),
                "after calls frame {frame}"
            );
            assert_eq!(
                *expected_resources.log.borrow(),
                *actual_resources.log.borrow(),
                "call requests frame {frame}"
            );
            if fail {
                assert!(!expected_resources.log.borrow().is_empty());
            }
            let tick = Tick {
                time: frame as f32 * 0.05,
                dt: 0.05,
            };
            expected.tick(template, &expected_resources, tick).unwrap();
            actual.tick(template, &actual_resources, tick).unwrap();
            assert_eq!(
                format!("{expected:?}"),
                format!("{actual:?}"),
                "after tick frame {frame}"
            );
            assert_eq!(expected.take_held(), actual.take_held());
            assert_eq!(
                expected.bindings(template, &expected_resources).unwrap().bound,
                actual.bindings(template, &actual_resources).unwrap().bound
            );
            let old_layout = expected.layout_with_regions(template, &expected_resources, None, true);
            let new_layout = actual.layout_with_regions(template, &actual_resources, None, true);
            assert_eq!(old_layout, new_layout, "frame {frame}");
            if let Ok(Some(layout)) = old_layout {
                let mut old_batch = Batch::default();
                let mut new_batch = Batch::default();
                assert_eq!(
                    draw(&layout, [13, 7], &fixture, &mut old_batch),
                    draw(
                        new_layout.as_ref().unwrap().as_ref().unwrap(),
                        [13, 7],
                        &fixture,
                        &mut new_batch
                    )
                );
                assert_eq!(old_batch.quads, new_batch.quads, "frame {frame}");
                assert_eq!(
                    output_bits(&old_batch),
                    output_bits(&new_batch),
                    "frame {frame}"
                );
                saw_quads |= !old_batch.quads.is_empty();
            }
            assert_eq!(
                *expected_resources.log.borrow(),
                *actual_resources.log.borrow(),
                "all requests frame {frame}"
            );
            saw_flag |= expected_resources
                .log
                .borrow()
                .iter()
                .any(|entry| entry.starts_with("flag:"));
        }
        assert!(saw_quads);
        assert!(saw_flag, "text flags must really request resources");
    }

    #[test]
    fn unchanged_sprite_setters_keep_pending_frame_play_and_replacement_order() {
        let gui = template();
        let root = gui.template("root").unwrap();
        let icon = root.path_named("icon").unwrap();
        let resources = fixture();
        let mut actual = Instance::construct(root, [800, 600], &resources).unwrap();
        actual.gui_show(root, root).unwrap();
        let mut expected = actual.clone();
        for frame in 0..12 {
            let name = if !(4..8).contains(&frame) {
                b"GFX_animated".as_slice()
            } else {
                b"GFX_even"
            };
            for instance in [&mut actual, &mut expected] {
                if frame == 2 || frame == 6 {
                    instance
                        .set_sprite(root, icon, b"GFX_odd", SpriteSetter::Icon)
                        .unwrap();
                }
                if frame == 3 {
                    instance.control(root, icon).unwrap().sprite = Some(b"GFX_odd".to_vec());
                }
                instance.set_frame(root, icon, 2).unwrap();
                instance.play(root, icon).unwrap();
            }
            actual
                .set_sprite(root, icon, name, SpriteSetter::Icon)
                .unwrap();
            expected.flush(1);
            expected.nodes[1]
                .events
                .push(Event::Sprite(name.to_vec(), SpriteSetter::Icon));
            let tick = Tick {
                time: frame as f32 * 0.125,
                dt: 0.125,
            };
            actual.tick(root, &resources, tick).unwrap();
            expected.tick(root, &resources, tick).unwrap();
            assert_eq!(
                actual.bindings(root, &resources).unwrap().bound,
                expected.bindings(root, &resources).unwrap().bound
            );
            assert_eq!(actual.nodes[1].runtime, expected.nodes[1].runtime);
        }
        actual
            .set_sprite(root, icon, b"GFX_animated", SpriteSetter::Icon)
            .unwrap();
        assert!(actual.nodes[1].events.is_empty());
    }

    #[test]
    fn hidden_root_skips_only_draw_traversal_not_child_clocks_or_show_history() {
        let gui = template();
        let root = gui.template("root").unwrap();
        let copied = root.clone();
        let resources = fixture();
        let mut actual = Instance::construct(root, [800, 600], &resources).unwrap();
        let mut expected = actual.clone();
        for frame in 0..12 {
            for instance in [&mut actual, &mut expected] {
                match frame {
                    1 | 7 => instance.gui_show(root, root).unwrap(),
                    4 => instance.hide_window(root, root).unwrap(),
                    _ => {}
                }
            }
            let tick = Tick {
                time: frame as f32 * 0.125,
                dt: 0.125,
            };
            actual.tick(root, &resources, tick).unwrap();
            expected.tick(&copied, &resources, tick).unwrap();
            assert_eq!(
                actual.bindings(root, &resources).unwrap().bound,
                expected.bindings(root, &resources).unwrap().bound
            );
            assert_eq!(actual.active(), expected.active());
            assert_eq!(
                actual.layout(root, &resources, None).unwrap(),
                expected.layout(root, &resources, None).unwrap()
            );
        }
    }
}

#[cfg(test)]
mod frame_clock_tests {
    use super::*;

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

#[cfg(test)]
mod traversal_tests {
    use super::*;

    #[test]
    fn retained_routes_keep_parts_children_and_copied_templates_in_order() {
        content_reader_script::test_game::enter();
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root
            background={name=back}
            gridBoxType={name=grid background={name=grid_back}}
            extendedScrollbarType={name=bar
                background={name=bar_back}
                track={name=track}
                slider={name=slider}
                increaseButton={name=increase}
                decreaseButton={name=decrease}}
            dropDownBoxType={name=drop
                buttonType={name=expand}
                containerWindowType={name=expanded}}
            containerWindowType={name=child iconType={name=icon}}
        }}"#,
        );
        let root = gui.template("root").unwrap();
        let authored = elements(root);
        assert!(authored.iter().any(|(_, _, child)| !child));
        let tree = Tree::new(&authored);
        let mut pending = vec![root];
        let mut expected = Vec::new();
        while let Some(element) = pending.pop() {
            expected.push(element as *const Element);
            pending.extend(element.nested().into_iter().rev());
        }
        assert_eq!(
            authored
                .iter()
                .map(|(element, ..)| *element as *const Element)
                .collect::<Vec<_>>(),
            expected,
        );
        let retained = tree.elements(root);
        for (actual, expected) in retained.iter().zip(&authored) {
            assert!(std::ptr::eq(actual.0, expected.0));
            assert_eq!((actual.1, actual.2), (expected.1, expected.2));
        }
        let copied = root.clone();
        assert_eq!(tree.elements(&copied), elements(&copied));
        for (index, (element, ..)) in authored.iter().enumerate() {
            assert_eq!(
                tree.indices.get(&(*element as *const Element as usize)),
                Some(&index)
            );
        }
    }
}

#[cfg(test)]
mod clock_pruning_tests {
    use super::*;
    use crate::test_support::gui::{Fixture, fixture};
    use std::cell::Cell;

    struct Metadata<'a> {
        fixture: &'a Fixture,
        borrowed: bool,
        lookups: Cell<usize>,
    }

    impl Resources for Metadata<'_> {
        fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
            self.fixture.sprite(name)
        }
        fn sprite_info(&self, name: &[u8]) -> Option<Cow<'_, SpriteInfo>> {
            self.lookups.set(self.lookups.get() + 1);
            let info = self.fixture.sprites.get(std::str::from_utf8(name).ok()?)?;
            Some(if self.borrowed {
                Cow::Borrowed(info)
            } else {
                Cow::Owned(info.clone())
            })
        }
        fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
            self.fixture.font(name)
        }
        fn localisation(&self) -> &Localisation {
            self.fixture.localisation()
        }
        fn text_colors(&self) -> &TextColors {
            self.fixture.text_colors()
        }
        fn text_effect(&self) -> Result<Effect, String> {
            self.fixture.text_effect()
        }
        fn effect(&self, file: &str) -> Result<Effect, String> {
            self.fixture.effect(file)
        }
        fn shield_flag(
            &self,
            small: bool,
            index: i32,
        ) -> Result<Option<(Texture, [f32; 4])>, String> {
            self.fixture.shield_flag(small, index)
        }
        fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
            self.fixture.text_flag(tag)
        }
        fn gui_type(&self, name: &[u8]) -> Option<&Element> {
            self.fixture.gui_type(name)
        }
    }

    #[test]
    fn windows_without_drawable_clocks_do_not_resolve_sprites_in_the_clock_phase() {
        content_reader_script::test_game::enter();
        let fixture = fixture();
        let resources = Metadata {
            fixture: &fixture,
            borrowed: true,
            lookups: Cell::new(0),
        };
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root
            size={width=100 height=50} iconType={name=icon spriteType=GFX_even}}}"#,
        );
        let root = gui.template("root").unwrap();
        let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
        assert!(instance.clock_nodes.is_empty());
        for shown in [false, true] {
            if shown {
                instance.gui_show(root, root).unwrap();
            }
            let icon = root.path_named("icon").unwrap();
            instance.set_progress_float_value(root, icon, 19).unwrap();
            instance.set_frame(root, icon, 2).unwrap();
            resources.lookups.set(0);
            instance
                .tick_clocks(root, &resources, Tick { time: 1.0, dt: 0.1 })
                .unwrap();
            assert_eq!(resources.lookups.get(), 0);
            assert_eq!(instance.nodes[1].control.frame, Some(2));
            assert_eq!(instance.nodes[1].control.progress_float, None);
            instance.tick_transitions(root).unwrap();
            instance.tick_draws(root, &resources).unwrap();
        }
    }

    #[test]
    fn an_idle_window_s_tick_stamps_its_clock_alone_and_lays_out_as_a_full_tick() {
        content_reader_script::test_game::enter();
        let fixture = fixture();
        let resources = Metadata {
            fixture: &fixture,
            borrowed: true,
            lookups: Cell::new(0),
        };
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root
            size={width=100 height=50} iconType={name=icon spriteType=GFX_even}}}"#,
        );
        let root = gui.template("root").unwrap();
        let icon = root.path_named("icon").unwrap();
        let mut idle = Instance::construct(root, [800, 600], &resources).unwrap();
        let mut written = Instance::construct(root, [800, 600], &resources).unwrap();
        for instance in [&mut idle, &mut written] {
            instance.gui_show(root, root).unwrap();
            instance.set_frame(root, icon, 2).unwrap();
        }
        let frames = [
            ([10, 20], false),
            ([10, 20], true),
            ([10, 20], true),
            ([30, 20], true),
            ([30, 20], true),
        ];
        for (frame, (position, expected)) in frames.into_iter().enumerate() {
            idle.set_position(root, root, position).unwrap();
            written.set_position(root, root, position).unwrap();
            written.set_frame(root, icon, 2).unwrap();
            assert_eq!(idle.idle(), expected, "frame {frame}");
            assert!(!written.idle(), "frame {frame}: a written window is never idle");
            let tick = Tick {
                time: 1.0 + frame as f32 / 60.0,
                dt: 1.0 / 60.0,
            };
            resources.lookups.set(0);
            idle.tick(root, &resources, tick).unwrap();
            if expected {
                assert_eq!(resources.lookups.get(), 0, "frame {frame}: an idle tick resolves no drawable");
            }
            written.tick(root, &resources, tick).unwrap();
            assert!(idle.idle(), "frame {frame}: settled after its tick");
            let full = written.layout_with_regions(root, &resources, None, true).unwrap();
            assert_eq!(idle.layout_with_regions(root, &resources, None, true).unwrap(), full, "frame {frame}");
            resources.lookups.set(0);
            let kept = idle.layout_kept(root, &resources, true, 7).unwrap().map(Layout::as_drawn);
            assert_eq!(kept, full.clone().map(Layout::as_drawn), "frame {frame}: kept as it draws");
            if frame > 0 && expected {
                assert_eq!(resources.lookups.get(), 0, "frame {frame}: an idle window's layout is kept");
            }
        }
        idle.set_frame(root, icon, 1).unwrap();
        idle.tick(root, &resources, Tick { time: 2.0, dt: 1.0 / 60.0 }).unwrap();
        let fresh = idle.layout_with_regions(root, &resources, None, true).unwrap();
        assert_ne!(fresh, written.layout_with_regions(root, &resources, None, true).unwrap(), "the write draws another frame");
        assert_eq!(idle.layout_kept(root, &resources, true, 7).unwrap(), fresh, "written");
        resources.lookups.set(0);
        assert_eq!(idle.layout_kept(root, &resources, true, 8).unwrap(), fresh, "restamped");
        assert!(resources.lookups.get() > 0, "another stamp lays out again");
        let copy = root.clone();
        resources.lookups.set(0);
        assert_eq!(idle.layout_kept(&copy, &resources, true, 8).unwrap(), fresh, "another root");
        assert!(resources.lookups.get() > 0, "another root lays out again");
    }

    #[test]
    fn copied_templates_resolve_their_own_drawable_clock_kinds() {
        content_reader_script::test_game::enter();
        let resources = fixture();
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root size={width=100 height=50}
                iconType={name=icon spriteType=GFX_even}}}"#,
        );
        let root = gui.template("root").unwrap();
        let mut copied = root.clone();
        let Fields::Icon(icon) = &mut copied.children[0].fields else {
            panic!("the copied icon");
        };
        icon.sprite = b"GFX_animated".to_vec();
        let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
        let mut fresh = Instance::construct(&copied, [800, 600], &resources).unwrap();
        instance.gui_show(&copied, &copied).unwrap();
        fresh.gui_show(&copied, &copied).unwrap();
        for frame in 0..4 {
            let tick = Tick {
                time: frame as f32 * 0.125,
                dt: 0.125,
            };
            instance.tick(&copied, &resources, tick).unwrap();
            fresh.tick(&copied, &resources, tick).unwrap();
            assert_eq!(
                instance.bindings(&copied, &resources).unwrap().bound,
                fresh.bindings(&copied, &resources).unwrap().bound,
            );
            assert_eq!(
                instance
                    .layout_with_regions(&copied, &resources, None, true)
                    .unwrap(),
                fresh
                    .layout_with_regions(&copied, &resources, None, true)
                    .unwrap(),
            );
        }
    }

    #[test]
    fn pruned_clocks_and_borrowed_metadata_equal_a_full_drawable_tick() {
        content_reader_script::test_game::enter();
        let mut fixture = fixture();
        let mut pulse = fixture.sprite(b"GFX_cornered").unwrap();
        if let SpriteKind::CorneredTile { looping, rate, .. } = &mut pulse.kind {
            *looping = true;
            *rate = Some(0.75);
        }
        fixture.sprites.insert("GFX_pulse", pulse);
        let animated = fixture.sprite(b"GFX_animated").unwrap();
        fixture
            .sprites
            .insert("GFX_default_fallback_texture", animated);
        let borrowed = Metadata {
            fixture: &fixture,
            borrowed: true,
            lookups: Cell::new(0),
        };
        let owned = Metadata {
            fixture: &fixture,
            borrowed: false,
            lookups: Cell::new(0),
        };
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root
            size={width=100 height=50}
            fade_type=linear fade_time=300
            iconType={name=icon spriteType=GFX_even}
            buttonType={name=text buttonFont=Main_14 text=""}
        }}"#,
        );
        let root = gui.template("root").unwrap();
        let icon = root.path_named("icon").unwrap();
        let text = root.path_named("text").unwrap();
        let mut pruned = Instance::construct(root, [800, 600], &borrowed).unwrap();
        let mut full = Instance::construct(root, [800, 600], &owned).unwrap();
        let copied = root.clone();
        for frame in 0..20 {
            for instance in [&mut pruned, &mut full] {
                match frame {
                    0 => instance.gui_show(root, root).unwrap(),
                    1 => instance
                        .set_sprite(root, icon, b"GFX_animated", SpriteSetter::Icon)
                        .unwrap(),
                    2 => instance.set_frame(root, icon, 3).unwrap(),
                    3 => instance
                        .set_sprite(root, icon, b"undefined", SpriteSetter::Icon)
                        .unwrap(),
                    4 => instance.hide(root, icon).unwrap(),
                    5 => instance
                        .set_sprite(root, icon, b"GFX_pulse", SpriteSetter::Icon)
                        .unwrap(),
                    6 => instance.show(root, icon).unwrap(),
                    7 => instance
                        .set_sprite(root, icon, b"GFX_pulse", SpriteSetter::Icon)
                        .unwrap(),
                    8 => instance.control(root, text).unwrap().text = Some("visible text".into()),
                    9 => instance.hide(root, root).unwrap(),
                    10 => instance
                        .set_sprite(root, icon, b"GFX_even", SpriteSetter::Icon)
                        .unwrap(),
                    11 => instance.gui_show(root, root).unwrap(),
                    12 => instance.control(root, text).unwrap().text = Some(String::new()),
                    13 => instance.control(root, text).unwrap().enabled = Some(false),
                    14 => instance
                        .set_sprite(root, icon, b"GFX_animated", SpriteSetter::Icon)
                        .unwrap(),
                    _ => {}
                }
            }
            let tick = Tick {
                time: frame as f32 * 0.125,
                dt: 0.125,
            };
            pruned.tick(root, &borrowed, tick).unwrap();
            full.tick(&copied, &owned, tick).unwrap();
            assert_eq!(pruned.active(), full.active(), "frame {frame}");
            assert_eq!(pruned.take_held(), full.take_held(), "frame {frame}");
            assert_eq!(
                pruned.bindings(root, &borrowed).unwrap().bound,
                full.bindings(root, &owned).unwrap().bound,
                "frame {frame}"
            );
            assert_eq!(
                pruned
                    .layout_with_regions(root, &borrowed, None, true)
                    .unwrap(),
                full.layout_with_regions(root, &owned, None, true).unwrap(),
                "frame {frame}"
            );
        }
    }
}

#[cfg(test)]
mod disabled_icon_tests {
    use super::*;
    use crate::{native_effects::synthetic, test_support::gui::fixture};

    #[test]
    fn an_icon_the_code_disables_draws_its_disable_technique() {
        content_reader_script::test_game::enter();
        let fixture = fixture();
        let gui = native::read(
            br#"guiTypes={containerWindowType={name=root size={width=100 height=50}
            iconType={name=icon spriteType=GFX_even}
        }}"#,
        );
        let root = gui.template("root").unwrap();
        let icon = root.path_named("icon").unwrap();
        let shading = |enabled: Option<bool>| {
            let mut instance = Instance::construct(root, [800, 600], &fixture).unwrap();
            instance.gui_show(root, root).unwrap();
            if enabled.is_some() {
                instance.control(root, icon).unwrap().enabled = enabled;
            }
            let layout = instance
                .layout_with_regions(root, &fixture, None, true)
                .unwrap()
                .unwrap();
            layout.parts[0].shading.clone()
        };
        let [up, disable] = ["Up", "Disable"]
            .map(|technique| super::super::shading(&synthetic::buttonstate(), technique, 0.0));
        assert_eq!(shading(None), up);
        assert_eq!(shading(Some(true)), up);
        assert_eq!(shading(Some(false)), disable);
    }

    #[test]
    fn settled_transitions_show_a_window_at_its_settled_place() {
        content_reader_script::test_game::enter();
        const GUI: &[u8] = br#"guiTypes={
            containerWindowType={name=root position={x=-160 y=0} size={width=160 height=90}
                fade_type=linear fade_time=150
                show_position={x=10 y=5} show_animation_type=linear show_animation_time=150}}"#;
        let gui = native::read(GUI);
        let root = gui.template("root").unwrap();
        let resources = fixture();
        let tick = Tick { time: 0.0, dt: 0.0 };
        let mut live = Instance::construct(root, [800, 600], &resources).unwrap();
        live.gui_show(root, root).unwrap();
        let mut saved = live.clone();
        live.tick(root, &resources, tick).unwrap();
        assert_ne!(live.position(), [10, 5], "live, the window is still moving in");
        saved.settle_transitions();
        saved.tick(root, &resources, tick).unwrap();
        assert_eq!(saved.position(), [10, 5], "saved, it stands at its show position");
        assert_eq!(saved.opacity(), 1.0, "and has faded in");
    }

    #[test]
    fn a_settled_step_completes_a_pending_hide() {
        content_reader_script::test_game::enter();
        const GUI: &[u8] = br#"guiTypes={
            containerWindowType={name=root position={x=0 y=0} size={width=160 height=90}
                show_position={x=10 y=5} show_animation_type=linear show_animation_time=150
                hide_position={x=-200 y=5} hide_animation_type=linear hide_animation_time=150}}"#;
        let gui = native::read(GUI);
        let root = gui.template("root").unwrap();
        let resources = fixture();
        let tick = Tick { time: 0.0, dt: 0.0 };
        let mut live = Instance::construct(root, [800, 600], &resources).unwrap();
        live.gui_show(root, root).unwrap();
        live.settle_transitions();
        live.tick(root, &resources, tick).unwrap();
        assert!(live.drawn(), "shown");
        live.hide(root, root).unwrap();
        let mut saved = live.clone();
        live.tick(root, &resources, tick).unwrap();
        assert!(live.drawn(), "live, the window is still moving out");
        saved.settle_transitions();
        saved.tick(root, &resources, tick).unwrap();
        assert!(!saved.drawn(), "saved, its hide is complete");
    }
}
