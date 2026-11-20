//! The map's side on a thread of its own. One [`SideThread`] per open map side owns the
//! [`MapSide`]: its thread takes the playhead's steps, applies them to the mirrors and settles
//! every layer for the step, so the picture stands whole at one hour before the frame asks for it.
//! It steps at the rate the simulation holds ([`viewer_foundation::map::simulation_rate`]), not
//! the display's, so a faster display asks it for nothing more. A publication is due when that
//! rate allows one and the head has reached an hour the last one did not show, and is prepared for
//! the hour the head will stand at by the time the last frame that draws it is drawn, so that every
//! frame drawing it places what moves inside the hours it carries.
//!
//! The frame thread takes the side once a frame in a window that never waits
//! ([`SideThread::window`]): in it the scene's uploads run (the scene is the frame's), the products
//! the rest of the frame draws from are taken as cheap shared values ([`Products`]), and the side is
//! handed back, which is the side's thread's signal to prepare the next step. While the side's
//! thread holds it, the frame draws the products of its last window.
//!
//! A stepped frame (a saved frame, the acceptance suite) asks for the picture of exactly its hour
//! and waits for that one thing ([`SideThread::stepped_window`]), never for work only the frame
//! advances.

use super::{Doing, Entry, Frame, Head, MapSide, OnScreen, Waited, lock, overlay};
use crate::layers::{Gpu, Installs, View, drawn_layer as drawn};
use replay_data::Hour;
use std::{
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};

/// The name of the thread one map side's steps are prepared on.
pub const THREAD: &str = "map side";

/// How long the side's thread waits for the frame's next ask.
const IDLE: Duration = Duration::from_millis(2);

/// The least the side's thread sleeps for a publication that is not due yet, and how long it
/// sleeps between looks while one waits for an hour to show, so a publication is never taken late
/// for want of a wakeup.
const UNTIL_DUE: Duration = Duration::from_micros(200);

/// How often a stepped frame says what its window is still waiting for.
const LATE: Duration = Duration::from_secs(5);

/// How long a stepped frame waits for its hour's picture before it refuses by name: far past every
/// bound the step and the whole settles below it keep, so a frame that waits this long is waiting
/// for something no thread will make true.
const GIVEN_UP: Duration = Duration::from_secs(300);

/// The newest view and head the frame asked for, applied by the side's thread at its next step:
/// one latest-wins message, posted without the side's lock.
#[derive(Clone)]
struct Ask {
    view: View,
    at: Head,
    /// What only the frame knows of the overlay: its camera, size, interface scale, counter
    /// options, selection and hover, and the raid arrows the scripts lane made.
    overlay: Option<overlay::Ask>,
}

/// What a stepped frame's picture was made whole for: its hour, and the view, overlay placement and
/// counter view it was asked at. Another ask at the same hour is another picture.
#[derive(Debug)]
struct Ready {
    hour: Hour,
    view: View,
    placed: Option<Placed>,
}

/// What of the overlay's ask its layout stands for: the camera, zoom target, size, interface scale
/// and counter view.
type Placed = (viewer_foundation::map::camera::Camera, Option<f32>, [u32; 2], f32, crate::map_views::CounterView);

impl Ready {
    fn of(hour: Hour, ask: &Ask) -> Self {
        Self { hour, view: ask.view.clone(), placed: ask.overlay.as_ref().map(Self::placed) }
    }

    fn placed(overlay: &overlay::Ask) -> Placed {
        (overlay.camera, overlay.target, overlay.pixels, overlay.ui_scale, overlay.view.clone())
    }

    fn answers(&self, hour: Hour, ask: &Ask) -> bool {
        self.hour == hour
            && self.view == ask.view
            && match (&self.placed, &ask.overlay) {
                (Some((camera, target, pixels, scale, view)), Some(overlay)) => {
                    (*camera, *target, *pixels, *scale) == (overlay.camera, overlay.target, overlay.pixels, overlay.ui_scale) && *view == overlay.view
                }
                (None, None) => true,
                _ => false,
            }
    }
}

/// Political pictures held for a highlight while the frames' hour catches up to the side's: the
/// publications prepared ahead of the head are a few.
const PICTURES_HELD: usize = 8;

/// The side, and the handoff between its thread and the frame.
struct Turn {
    side: MapSide,
    /// The frame has taken a window since the last step was prepared: the side's thread's signal
    /// to prepare the next. True to start: the first step composes the first picture.
    framed: bool,
    /// What a stepped frame's picture stands whole for.
    ready: Option<Ready>,
    /// When the publication being made began, until the frame takes it.
    published: Option<Instant>,
    /// How long the last publication took to reach the frame, which the next is prepared ahead of
    /// the head by ([`viewer_foundation::map::simulation_rate::Reach`]).
    reached: Option<Duration>,
    /// Looks its thread has taken at what the frame asks: whether that thread still runs.
    looks: u64,
    stop: bool,
}

struct Shared {
    held: Mutex<Turn>,
    ask: Mutex<Option<Ask>>,
    signal: Condvar,
}

impl Shared {
    fn asked(&self) -> Option<Ask> {
        lock(&self.ask).clone()
    }

    /// The picture stands whole for the newest stepped ask at `hour`.
    fn ready(&self, turn: &Turn, hour: Hour) -> bool {
        turn.ready.as_ref().is_some_and(|ready| lock(&self.ask).as_ref().is_some_and(|ask| ready.answers(hour, ask)))
    }
}

/// One map side and the thread that advances it.
pub struct SideThread {
    shared: Arc<Shared>,
    entry: Arc<Entry>,
    /// The content its layers draw the art of ([`Self::made_over`]).
    sight: Option<Arc<crate::document::Content>>,
    /// The side's view store, as the frame thread's own readers hold it: taken here so a reader
    /// of the hour never waits for the side's lock to open one.
    views: super::views::Views,
    products: Products,
    /// Every timed layer's installer, taken as the side came here: the display frame's own, so
    /// each frame writes the change sets of the hours its head has reached whether or not this
    /// thread is preparing a publication ([`Self::install`]).
    installs: Vec<Box<dyn Installs>>,
    /// Why an install stopped, by name; none while they write.
    install_failure: Option<String>,
    /// What its thread is doing, as that thread publishes it: what a frame that could not take the
    /// side missed it for ([`Doing`]).
    doing: Arc<Doing>,
    /// Pictures the side has composed: a change set dated over an older one is void.
    composed: Arc<std::sync::atomic::AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SideThread {
    /// `side` moved onto a thread of its own.
    pub fn new(mut side: MapSide) -> io::Result<Self> {
        let (entry, sight, views) = (Arc::clone(side.entry()), side.sight().cloned(), side.views().handle());
        let doing = Arc::clone(side.doing());
        let composed = Arc::clone(side.composed());
        let installs = side.take_installs();
        // Its branch is the side's from the start, so an opening map is checked before any window.
        let products = Products { branch: side.branch(), ..Products::default() };
        // The overlay's tables and layout are made here, which reads the recording's executable
        // tables: this thread works on that one recording, so they are in scope on it throughout.
        let tables = sight.as_ref().and_then(|content| content.tables);
        let shared = Arc::new(Shared {
            held: Mutex::new(Turn { side, framed: true, ready: None, published: None, reached: None, looks: 0, stop: false }),
            ask: Mutex::new(None),
            signal: Condvar::new(),
        });
        let thread = std::thread::Builder::new().name(THREAD.into()).spawn({
            let shared = Arc::clone(&shared);
            move || {
                viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Kept);
                if tables.is_some() {
                    engine_executable::executable_tables::enter(tables);
                }
                advance(&shared);
            }
        })?;
        Ok(Self { shared, entry, sight, views, products, installs, install_failure: None, doing, composed, thread: Some(thread) })
    }

    /// The view and head of the frame being drawn, with what only the frame knows of the overlay,
    /// for the step prepared next. Never waits.
    pub fn ask(&self, view: View, at: Head, overlay: Option<overlay::Ask>) {
        *lock(&self.shared.ask) = Some(Ask { view, at, overlay });
        self.shared.signal.notify_all();
    }

    /// The hour every drawing timed layer's installer has written through; none while none has
    /// written. The frame draws and says no later hour.
    pub fn installed_through(&self) -> Option<Hour> {
        self.installs.iter().filter_map(|installs| installs.shown()).min()
    }

    /// Something outside the installers replaced what they wrote (a highlight's restore): each
    /// writes its own again, now.
    pub fn reassert(&mut self, gpu: &mut Gpu<'_>) {
        for installs in &mut self.installs {
            if let Err(error) = installs.reassert(gpu) {
                self.install_failure = Some(format!("The map's layer {} stopped writing its borders again: {error}", installs.name()));
            }
        }
    }

    /// The hour of the set whose border plans the installers wrote last.
    pub fn plans_shown(&self) -> Option<Hour> {
        self.installs.iter().filter_map(|installs| installs.plans_shown()).max()
    }

    /// Writes every timed layer's change sets of the hours `through` has reached, newest value
    /// per entity; `None` writes every set handed over, for a picture that must be whole at the
    /// hour it stands at (a pause, a saved frame, a first picture). On the frame thread, outside
    /// every window: it takes nothing of the side, so a frame whose window its thread held still
    /// draws the replay at its own hour.
    pub fn install(&mut self, gpu: &mut Gpu<'_>, through: Option<Hour>) {
        if self.installs.is_empty() {
            return;
        }
        let _lap = replay_frame::cpu::scope("frame.refresh.installs", Some("other"));
        let composed = self.composed.load(std::sync::atomic::Ordering::Acquire);
        for installs in &mut self.installs {
            if let Err(error) = installs.install(gpu, through, composed) {
                self.install_failure = Some(format!("The map's layer {} stopped at an install: {error}", installs.name()));
            }
            if viewer_foundation::map::publication_check::on() {
                viewer_foundation::map::publication_check::judged();
                if installs.owes() {
                    viewer_foundation::map::publication_check::found(installs.name(), "a change set whose hour this frame had reached was left unwritten");
                }
            }
        }
        // The hour this frame leaves on screen: the oldest of what the side last said of the
        // layers whose publication is one picture and what the installers have just written. The
        // side says its own once a publication; a timed layer's hour moves with every frame, so
        // the hour the window clock shows and the hour the record reads are taken here.
        let timed = self.installs.iter().map(|installs| installs.shown()).min().expect("an installer");
        // Before an installer's first write there is no hour of its own to take the oldest of: the
        // side's own figure stands, which falls back to the hour it applied for a timed layer that
        // has shown nothing yet, so a first picture is not held back for want of one.
        if timed.is_none() {
            return;
        }
        let (untimed, untimed_oldest) = self.products.untimed;
        let hours = |held: Option<Hour>| match (held, timed) {
            (Some(held), Some(timed)) => Some(held.min(timed)),
            _ => None,
        };
        if let Some(on) = self.products.on_screen.as_mut() {
            on.drawn = hours(untimed);
        }
        (self.products.frame.drawn, self.products.frame.oldest) = (hours(untimed), hours(untimed_oldest));
    }

    /// What each timed layer's installer wrote on the last frame, by name.
    pub fn installed(&self) -> impl Iterator<Item = (&'static str, Option<Hour>, crate::layers::Wrote)> {
        self.installs.iter().map(|installs| (installs.name(), installs.shown(), installs.wrote()))
    }

    /// The side for this frame, unless its thread is preparing a step: none then, and the frame
    /// draws the products of its last window. Never waits.
    pub fn window(&mut self) -> Option<Window<'_>> {
        let turn = match self.shared.held.try_lock() {
            Ok(turn) => turn,
            Err(TryLockError::Poisoned(held)) => held.into_inner(),
            Err(TryLockError::WouldBlock) => {
                viewer_foundation::view_record::add("layers", "frames_without_the_side", 1);
                self.products.missed = Some(self.doing.name());
                return None;
            }
        };
        self.products.missed = None;
        Some(Window { turn, signal: &self.shared.signal, products: &mut self.products })
    }

    /// The window of a stepped frame: the picture whole at `hour`, waited for. None once the side
    /// has stopped at an error. A wait that runs long says what it waits for, and past
    /// [`GIVEN_UP`] refuses by name rather than waiting on.
    pub fn stepped_window(&mut self, hour: Hour) -> Option<Window<'_>> {
        self.whole_at(hour, LATE, GIVEN_UP)
    }

    /// [`Self::stepped_window`] with the bounds given, so a test need not wait out [`GIVEN_UP`].
    pub(super) fn whole_at(&mut self, hour: Hour, late: Duration, given_up: Duration) -> Option<Window<'_>> {
        let started = Instant::now();
        let mut said = 0;
        let mut turn = lock(&self.shared.held);
        while !self.shared.ready(&turn, hour) && turn.side.failure().is_none() {
            turn = self.shared.signal.wait_timeout(turn, late).unwrap_or_else(std::sync::PoisonError::into_inner).0;
            if self.shared.ready(&turn, hour) || turn.side.failure().is_some() {
                break;
            }
            let waited = started.elapsed();
            if waited >= given_up {
                let state = waiting_for(&self.shared, &turn, hour, waited);
                turn.side.fail(state);
                break;
            }
            let times = waited.as_nanos() / late.as_nanos().max(1);
            if times > said {
                said = times;
                eprintln!("{}", waiting_for(&self.shared, &turn, hour, waited));
            }
        }
        if let Some(failure) = turn.side.failure() {
            // No window is taken, so the products are not refreshed: the frame, the harness and the
            // map's warnings read why it got none from here.
            self.products.failure = Some(failure.to_owned());
            return None;
        }
        Some(Window { turn, signal: &self.shared.signal, products: &mut self.products })
    }

    /// The overlay laid out again over the art that arrived while the frame drew: a saved frame
    /// draws, settles the art its drawing asked for and takes the hour laid out over it, until
    /// none is left. Its side stands at the hour and prepares nothing meanwhile. Refused by name.
    pub fn lay_overlay_again(&mut self) -> Result<overlay::Made, String> {
        let mut turn = lock(&self.shared.held);
        if let Some(failure) = turn.side.failure() {
            return Err(failure.to_owned());
        }
        turn.side.lay_overlay_again()
    }

    /// What a frame's window does to the side, for a test with no frame: `drive` run on it, then
    /// every product taken, the warning lines with them. It waits for the side's own thread to
    /// finish a step it is preparing, as a frame does not.
    #[cfg(any(test, feature = "test-support"))]
    pub fn driven<T>(&mut self, drive: impl FnOnce(&mut MapSide) -> T) -> T {
        let turn = lock(&self.shared.held);
        self.products.missed = None;
        let mut window = Window { turn, signal: &self.shared.signal, products: &mut self.products };
        let made = drive(window.side_mut());
        window.take(Wants { warnings: true, highlight: false });
        drop(window.take_overlay());
        made
    }

    /// What the frame reads of the side between windows.
    pub fn products(&self) -> &Products {
        &self.products
    }

    /// What its drawn layers showed at its last window; none before the first.
    pub fn on_screen(&self) -> Option<OnScreen> {
        self.products.on_screen
    }

    /// Why its reader or an install stopped, by name; none while it draws.
    pub fn failure(&self) -> Option<&str> {
        self.products.failure.as_deref().or(self.install_failure.as_deref())
    }

    /// The branch its rows at the head are stamped with; none for a reader not opened for a
    /// document's map.
    pub fn branch(&self) -> Option<replay_data::BranchId> {
        self.products.branch
    }

    /// Whether layer `name` drew at its last window.
    pub fn draws(&self, name: &str) -> bool {
        self.products.draws(name)
    }

    /// A reset was composing at its last window.
    pub fn composing(&self) -> bool {
        self.products.composing
    }

    /// The hour its mirrors stood at.
    pub fn applied(&self) -> Option<Hour> {
        self.products.applied
    }

    /// The map mode its drawn picture was composed and settled for.
    pub fn mode_on_screen(&self) -> Option<&viewer_world::map::map_mode::MapMode> {
        self.products.mode.as_ref()
    }

    /// The record of the step last prepared, its upload included.
    pub fn last_frame(&self) -> &Frame {
        &self.products.frame
    }

    /// The view its last window drew at.
    pub fn view(&self) -> &View {
        &self.products.view
    }

    /// Each layer's name, in registration order.
    pub fn names(&self) -> &[&'static str] {
        &self.products.names
    }

    /// The side itself, waited for: the harness's and the diagnostics' reads, never the frame's
    /// drawing path.
    pub fn with_side<T>(&self, read: impl FnOnce(&MapSide) -> T) -> T {
        read(&lock(&self.shared.held).side)
    }

    /// [`Self::with_side`] without waiting: none while its thread prepares a step. What a frame
    /// reads of the side beside its window (the harness's record) takes this, so a step's joins
    /// never hold the frame.
    pub fn try_with_side<T>(&self, read: impl FnOnce(&MapSide) -> T) -> Option<T> {
        match self.shared.held.try_lock() {
            Ok(turn) => Some(read(&turn.side)),
            Err(TryLockError::Poisoned(held)) => Some(read(&held.into_inner().side)),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// [`Self::with_side`] for what changes the side off its thread: a test that leaves it as a
    /// frame would.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_side_mut<T>(&self, change: impl FnOnce(&mut MapSide) -> T) -> T {
        change(&mut lock(&self.shared.held).side)
    }

    /// The side's view store: what every reader of the hour on the frame thread opens on.
    pub fn views(&self) -> &super::views::Views {
        &self.views
    }

    pub fn entry(&self) -> &Arc<Entry> {
        &self.entry
    }


    /// The clock's head now.
    pub fn head(&self, now: Instant) -> Hour {
        self.entry.playhead().head(now)
    }

    /// The map's playback anchors: a seek, play at `speed` game hours a second, or a pause.
    pub fn anchor(&self, hour: Hour, playing: bool, speed: f64, sought: bool) {
        self.entry.anchor(hour, playing, speed, sought);
    }

    /// Whether its layers were made over `content`, the content whose art they draw.
    pub fn made_over(&self, content: *const crate::document::Content) -> bool {
        self.sight.as_ref().is_some_and(|sight| std::ptr::eq(Arc::as_ptr(sight), content))
    }
}

impl Drop for SideThread {
    fn drop(&mut self) {
        lock(&self.shared.held).stop = true;
        self.shared.signal.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What a stepped frame's window is still waiting for: the hour it asked, the handoff between the
/// frame and the side's thread, and what every layer of the side holds.
fn waiting_for(shared: &Shared, turn: &Turn, hour: Hour, waited: Duration) -> String {
    format!(
        "The saved frame at hour {hour} waited {:.1} s for the map's side: asked {:?}, whole at {:?}, framed {}, looks {}, stopping {}; {}",
        waited.as_secs_f64(),
        shared.asked().map(|ask| ask.at),
        turn.ready.as_ref().map(|ready| ready.hour),
        turn.framed,
        turn.looks,
        turn.stop,
        turn.side.shown_states(),
    )
}

/// The side held for one frame's window; handed back when dropped.
pub struct Window<'a> {
    turn: MutexGuard<'a, Turn>,
    signal: &'a Condvar,
    products: &'a mut Products,
}

impl Window<'_> {
    pub fn side(&self) -> &MapSide {
        &self.turn.side
    }

    pub fn side_mut(&mut self) -> &mut MapSide {
        // What the window changes (a saved frame settling its models) is copied again at its take.
        self.products.movement_of = None;
        &mut self.turn.side
    }

    /// Writes every drawn layer's changed ranges into the scene: the map side's only GPU work in
    /// the window, on the frame thread because the scene is the frame's. A timed layer writes
    /// nothing here: its hours' change sets are the frame's own ([`SideThread::install`]).
    pub fn upload(&mut self, gpu: &mut Gpu<'_>) {
        let side = &mut self.turn.side;
        if side.failure().is_some() {
            return;
        }
        if let Err(error) = side.upload(gpu) {
            side.fail(format!("The map's playhead stopped at an upload: {error}"));
        }
    }

    /// The overlay the step made, taken while the side is held: the frame swaps its counters'
    /// memory in here, the one place the side writes nothing.
    pub fn take_overlay(&mut self) -> Option<overlay::Made> {
        self.products.overlay.take()
    }

    /// Whether the frame lets go of the overlay it holds: made for another viewing country, with
    /// none of the view's coming ([`MapSide::take_overlay_withdrawal`]). Taken once.
    pub fn take_overlay_withdrawal(&mut self) -> bool {
        std::mem::take(&mut self.products.overlay_withdrawn)
    }

    /// Takes the products the frame draws from until its next window.
    pub fn take(&mut self, wants: Wants) {
        let _lap = replay_frame::cpu::scope("frame.refresh.products", Some("other"));
        // The frame taking the newest publication is that publication reaching the screen: how
        // long it took is what the side prepares the next one ahead of the head by.
        let reached = self.turn.published.take().map(|published| published.elapsed());
        self.turn.reached = reached.or(self.turn.reached);
        self.turn.side.reached(reached);
        self.products.take(&mut self.turn.side, wants);
    }
}

impl Drop for Window<'_> {
    fn drop(&mut self) {
        self.turn.framed = true;
        self.signal.notify_all();
    }
}

/// The side's thread: every step the playhead builds up to the head, applied and settled whole,
/// one publication at the rate the simulation holds. A stepped frame's hour is answered at once,
/// whatever that rate.
fn advance(shared: &Shared) {
    use viewer_foundation::map::simulation_rate as rate;
    let (mut pace, mut reach) = (rate::Pace::new(Instant::now()), rate::Reach::default());
    // What the publication being waited for has waited, for its record: cleared when it is made.
    let mut waited = Waited::default();
    let mut turn = lock(&shared.held);
    loop {
        turn.looks += 1;
        if turn.stop {
            return;
        }
        let Some(ask) = shared.asked().filter(|_| turn.side.failure().is_none()) else {
            turn = wait(shared, turn);
            continue;
        };
        match ask.at {
            Head::Stepped(hour) if turn.ready.as_ref().is_some_and(|ready| ready.answers(hour, &ask)) => turn = wait(shared, turn),
            Head::Stepped(hour) => {
                // The lock is held throughout: the frame that asked waits for this hour's picture
                // and for nothing else, and nothing else of the side is read meanwhile.
                match toward(&mut turn.side, &ask, hour, Made::Stepped) {
                    Ok(()) => turn.ready = Some(Ready::of(hour, &ask)),
                    Err(error) => turn.side.fail(format!("The map's playhead stopped: {error}")),
                }
                // The picture stands at the hour, or the side stopped: either answers the frame
                // waiting for it, and neither is announced anywhere else.
                shared.signal.notify_all();
            }
            // Its signal to prepare the next step is the frame's release of the one before; the
            // ask it reads with it is posted without this lock, whose notification a look can
            // fall either side of, so this wait is bounded like the others.
            _ if !turn.framed => {
                let since = Instant::now();
                let due = pace.late(since).is_some();
                turn = wait(shared, turn);
                if due {
                    waited.for_the_frame_ns += since.elapsed().as_nanos() as u64;
                }
            }
            at => {
                let (now, interval) = (Instant::now(), rate::interval());
                // The rate is the simulation's own: a publication is made when one is due at it,
                // however many frames the display drew meanwhile. A new viewing country is due at
                // once, as the user's own change.
                let viewpoint = ask.view.perspective != turn.side.view().perspective;
                let Some(late) = pace.late(now).or((viewpoint || turn.side.hurrying()).then_some(std::time::Duration::ZERO)) else {
                    turn = waiting(shared, turn, pace.until(now).max(UNTIL_DUE).min(IDLE));
                    continue;
                };
                if let Some(reached) = turn.reached.take() {
                    reach.took(reached, interval);
                }
                let asked = match at {
                    Head::Clock => turn.side.head(now),
                    Head::Paced(hour) | Head::Stepped(hour) => hour,
                };
                // Prepared ahead: the publication is for the hour the head will have reached by
                // the time the last frame that draws it is drawn, and over the interval its own
                // frames draw in, so that a timed layer ([`crate::layers::Layer::timed`]) carries the
                // hour of every one of them and each draws the replay at its own.
                let ahead = reach.ahead(interval) + interval;
                let moving = turn.side.head_from(asked, ahead);
                let head = moving.unwrap_or(asked);
                // No publication shows the picture the one before showed: while the head moves,
                // the one due waits for an hour it has not shown, up to an interval past its due
                // time. The head standing still, it is made at the rate, since what it answers
                // then is the frame's newest ask and whatever the last one left behind.
                if moving.is_some() && late < interval && !viewpoint && !turn.side.reaches(head) {
                    let since = Instant::now();
                    turn = waiting(shared, turn, UNTIL_DUE);
                    waited.for_an_hour_ns += since.elapsed().as_nanos() as u64;
                    continue;
                }
                pace.take(now, interval);
                waited.late_ns = late.as_nanos() as u64;
                turn.side.waited(std::mem::take(&mut waited));
                turn.framed = false;
                turn.ready = None;
                turn.published = Some(now);
                // A switch's publications wait for nothing under the lock: the new country's
                // parts join the publications after as they are made.
                let made = if moving.is_some() || viewpoint || turn.side.hurrying() { Made::Playing } else { Made::Still };
                if let Err(error) = toward(&mut turn.side, &ask, head, made) {
                    turn.side.fail(format!("The map's playhead stopped: {error}"));
                    shared.signal.notify_all();
                }
            }
        }
    }
}

/// Waits for a message posted without the side's lock (the frame's ask), which a wakeup can be
/// missed for: bounded instead of indefinite.
fn wait<'a>(shared: &'a Shared, turn: MutexGuard<'a, Turn>) -> MutexGuard<'a, Turn> {
    waiting(shared, turn, IDLE)
}

/// [`wait`] for `most`, for a thread with a step due sooner than [`IDLE`].
fn waiting<'a>(shared: &'a Shared, turn: MutexGuard<'a, Turn>, most: Duration) -> MutexGuard<'a, Turn> {
    shared.signal.wait_timeout(turn, most).unwrap_or_else(std::sync::PoisonError::into_inner).0
}

/// What the publication being made is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Made {
    /// The head is moving: it waits for no product that may be a publication late, since the next
    /// publication is 33 ms away.
    Playing,
    /// The head stands still (paused, held at a seek, the recording's last hour): no later hour is
    /// coming, so it waits for every product and leaves the picture whole.
    Still,
    /// A stepped frame's, whose picture stands whole at its hour, caught up to however far it is.
    Stepped,
}

/// One step at `head`: the view taken, the steps the head reached applied and every layer settled.
/// A stepped frame's hour is caught up to and settled whole, which may take many steps.
fn toward(side: &mut MapSide, ask: &Ask, head: Hour, made: Made) -> io::Result<()> {
    if let Some(error) = side.entry().playhead().error() {
        return Err(io::Error::other(format!("The playhead stopped: {error}")));
    }
    if side.view() != &ask.view {
        side.set_view(ask.view.clone())?;
    }
    if let Some(overlay) = &ask.overlay {
        side.ask_overlay(overlay.clone());
    }
    match made {
        // The overlay is a maker of the step: asked at its apply and joined inside it, so the
        // picture handed to the frame carries the overlay of the newest hour its maker reached.
        Made::Playing => side.frame(head).map(drop),
        Made::Still => side.frame_still(head).map(drop),
        Made::Stepped => side.catch_up(head),
    }
}

/// The map's layers' products the overlay lays in over the hour's own (labels, intel icons,
/// resource icons, occupation marks), each taken while its version is new, so the layout that
/// holds one already never copies it again.
#[derive(Default)]
pub struct LayIns {
    pub labels: Option<(Arc<map_display_data::landscape::Labels>, u32)>,
    pub intel: Option<Arc<viewer_raids::map::intel_icons::IntelIcons>>,
    /// The raid icons the scripts layer built at the head, which the overlay's tables are made
    /// with; none while the scripts draw nothing or have published nothing yet.
    pub raids: Option<Arc<viewer_raids::map::raids::RaidIcons>>,
    resources: Option<(viewer_world::map::resource_icons::HourIcons, u64)>,
    occupation: Option<(viewer_world::map::occupation_icons::Marks, u64)>,
}

impl LayIns {
    /// What the map's layers hold at the step, for the overlay the side lays out.
    pub fn of(side: &MapSide) -> Self {
        let resources = crate::layers::resource_icons::drawn(side);
        let occupation = crate::layers::occupation_marks::drawn(side);
        let (branch, ours) = (side.branch, crate::layers::vision::drawn(side).and_then(crate::layers::vision::Vision::ours));
        Self {
            labels: crate::layers::labels::drawn(side),
            intel: crate::layers::intel_icons::drawn(side),
            raids: branch.zip(ours).and_then(|(branch, ours)| crate::layers::scripts::raids(side, branch, ours)),
            resources: resources.map(|layer| (layer.icons(), layer.version())),
            occupation: occupation.map(|layer| (layer.marks(), layer.version())),
        }
    }

    /// Sets the labels and intel icons in `tables`, as the frame does after installing them.
    pub fn set(&self, tables: &mut crate::document::OverlayTables) {
        if let Some((labels, ours)) = &self.labels {
            tables.set_labels(labels, *ours);
        }
        if let Some(intel) = &self.intel {
            tables.set_intel(intel);
        }
    }

    /// Lays the resource icons and occupation marks into `prepared` where it holds another
    /// product, for viewing country `ours`; whether it laid any.
    pub fn lay(&self, prepared: &mut crate::document::PreparedOverlay, hour: &crate::document::Hour<'_>, camera: viewer_foundation::map::camera::Camera, pixels: [u32; 2], ui_scale: f32, ours: u32) -> bool {
        use crate::document::MadeBy;
        let mut laid = false;
        if let Some((icons, version)) = &self.resources
            && prepared.resource_icons_by() != MadeBy::Layer(*version)
        {
            prepared.replace_resource_icons(hour, camera, pixels, ui_scale, icons, *version);
            laid = true;
        }
        if let Some((marks, version)) = &self.occupation
            && prepared.occupation_icons_by() != MadeBy::Layer(*version)
        {
            prepared.replace_occupation_icons(hour, camera, pixels, ui_scale, ours, marks, *version);
            laid = true;
        }
        laid
    }

}

/// What a frame wants of the products besides what every frame reads: the layers' warning lines,
/// which only a diagnostics panel or a saved frame builds.
#[derive(Clone, Copy, Default)]
pub struct Wants {
    pub warnings: bool,
    /// A faction highlight is shown or asked for: only then is the political picture copied.
    pub highlight: bool,
}

/// The map's side as the frame reads it between windows: what its layers show and the products
/// they published, as cheap shared values.
pub struct Products {
    /// Windows taken, for the report.
    windows: u64,
    /// What the side's thread was doing when this frame could not take it; none when it did.
    pub missed: Option<&'static str>,
    pub names: Vec<&'static str>,
    /// Per layer: whether it draws (declared, composed, not refused).
    drawn: Vec<bool>,
    pub on_screen: Option<OnScreen>,
    /// The hour the picture shows and the oldest hour on it, counting only the layers whose
    /// publication is one picture: what [`SideThread::install`] takes the oldest of with the timed
    /// layers' own installers to leave the frame's own hour on screen.
    untimed: (Option<Hour>, Option<Hour>),
    pub failure: Option<String>,
    pub view: View,
    pub composing: bool,
    pub applied: Option<Hour>,
    pub mode: Option<viewer_world::map::map_mode::MapMode>,
    pub frame: Frame,
    pub branch: Option<replay_data::BranchId>,
    /// The battle ambience the sounds layer mixes; none while it does not draw.
    pub battles: Option<Option<Arc<viewer_audio::battle::Battles>>>,
    pub radar: Option<Arc<viewer_world::map::radar_symbols::Field>>,
    /// The battle plan the orders layer made, for the hour of the rows it was made from.
    pub plan: Option<(Hour, Arc<viewer_foundation::map::ribbon_content::OrderField>)>,
    /// The convoys layer's trade routes, with the sequence number of the product.
    pub routes: Option<(u64, Arc<viewer_world::map::trade_routes::TradeRoutes>)>,
    /// The labels the labels layer draws, with the branch and hours they hold for.
    pub labels: Option<crate::layers::labels::Stamped>,
    /// Which of the overlay's parts the layers draw, so the hour's own are left out.
    pub left: crate::document::LeftToLayers,
    /// The country, state and capital names the names layer placed.
    pub placements: Option<Arc<viewer_foundation::map::name_draw::Placements>>,
    /// The counter kinds the layers publish, and which kinds they draw.
    pub counters: viewer_counters::map::counters::Layered,
    pub counter_kinds: viewer_counters::map::counters::Kinds,
    pub units: Option<Arc<viewer_world::map::unit_models::PreparedModels>>,
    /// Every unit on the move with its place, as the frame gives them to the scene.
    pub movement: Vec<(viewer_foundation::unit_handle::Unit, viewer_foundation::map::counter_move::Position, f32, f32)>,
    /// The publication [`Self::movement`] was copied from.
    movement_of: Option<u64>,
    /// Every division in a province where a land figure stands, for the model picker.
    pub pick_divisions: std::collections::HashMap<u32, Vec<crate::layers::units::PickDivision>>,
    /// The viewing country the vision layer shows.
    pub ours: Option<u32>,
    /// The engaged index the engagements layer reads.
    pub engaged: Option<Arc<viewer_foundation::engagement_history::History>>,
    /// The picture the faction highlight is built over, with the key it stands for.
    pub highlight: Option<(u64, crate::highlight::Over<Arc<crate::highlight::Picture>>)>,
    /// The hour of the political picture [`Self::highlight`] is built over; none for the mode
    /// fills' picture.
    pub highlight_hour: Option<Hour>,
    /// The political pictures copied while a highlight is wanted, oldest first, each with the hour
    /// the side settled it at and its key: the frames take the newest their hour has reached.
    pictures: std::collections::VecDeque<(Hour, u64, Arc<crate::highlight::Picture>)>,
    /// What the snow layer drew, which names the part of it that is unknown.
    pub snow: Option<map_display_data::Summary>,
    /// The music chances the scripts layer evaluated at the head; none while it draws nothing.
    pub music: Option<Arc<viewer_audio::Chances>>,
    /// The map's and the layers' warning lines, taken only for a frame that builds warnings.
    pub lines: Option<Lines>,
    /// The overlay the step made, taken by the frame once ([`Window::take_overlay`]).
    pub overlay: Option<overlay::Made>,
    /// The frame lets go of the overlay it holds ([`Window::take_overlay_withdrawal`]).
    overlay_withdrawn: bool,
    /// The viewing country's vision refused, by name, once its fold has answered.
    pub vision_refused: Option<String>,
    /// The hour of the rows the overlay on screen was made from; none before its first.
    pub overlay_hour: Option<Hour>,
    /// The overlay's refusals and what its tables are loading, by name.
    pub overlay_loading: Vec<String>,
    /// The tables the overlay's layout read that its rows do not hold.
    pub overlay_missed: Vec<replay_data::rows::TableId>,
    /// The vision the map's layers show, as the step copied it out; why there is none, by name.
    pub vision: Result<Arc<viewer_foundation::map::perspective::Vision>, String>,
}

impl Default for Products {
    fn default() -> Self {
        Self {
            windows: 0,
            missed: None,
            names: Vec::new(),
            drawn: Vec::new(),
            on_screen: None,
            untimed: (None, None),
            failure: None,
            view: View::default(),
            composing: false,
            applied: None,
            mode: None,
            frame: Frame::default(),
            branch: None,
            battles: None,
            radar: None,
            plan: None,
            routes: None,
            labels: None,
            left: Default::default(),
            placements: None,
            counters: Default::default(),
            counter_kinds: Default::default(),
            units: None,
            movement: Vec::new(),
            movement_of: None,
            pictures: std::collections::VecDeque::new(),
            pick_divisions: Default::default(),
            highlight_hour: None,
            ours: None,
            engaged: None,
            highlight: None,
            snow: None,
            music: None,
            lines: None,
            overlay: None,
            overlay_withdrawn: false,
            vision_refused: None,
            overlay_hour: None,
            overlay_loading: Vec::new(),
            overlay_missed: Vec::new(),
            vision: Err("the playhead's map is not open".to_owned()),
        }
    }
}

/// What each lay-in's product stands for: a layout made over the same keys never takes them
/// again ([`LayIns::of`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayInKeys {
    labels: Option<(usize, u32)>,
    intel: Option<usize>,
    resources: Option<u64>,
    occupation: Option<u64>,
}

impl LayInKeys {
    /// What `side`'s layers hold now.
    pub fn of(side: &MapSide) -> Self {
        Self {
            labels: crate::layers::labels::drawn(side).map(|(labels, ours)| (Arc::as_ptr(&labels) as usize, ours)),
            intel: crate::layers::intel_icons::drawn(side).map(|icons| Arc::as_ptr(&icons) as usize),
            resources: crate::layers::resource_icons::drawn(side).map(crate::layers::resource_icons::ResourceIcons::version),
            occupation: crate::layers::occupation_marks::drawn(side).map(crate::layers::occupation_marks::OccupationMarks::version),
        }
    }
}

/// The map's warning lines: the fill's own, the tables' limitations at the head and each layer's
/// report ([`crate::presentation::map_lines`]).
pub struct Lines {
    pub political: Vec<String>,
    pub limitations: Vec<String>,
    pub reports: Vec<String>,
}

impl Products {
    /// Whether layer `name` draws.
    pub fn draws(&self, name: &str) -> bool {
        self.names.iter().zip(&self.drawn).any(|(held, &drawn)| *held == name && drawn)
    }

    /// Whether an overlay layout for the view shown is still to come: not once its vision is
    /// refused (none is laid out without one), nor once the reader has stopped.
    pub fn overlay_coming(&self) -> bool {
        self.vision_refused.is_none() && self.failure.is_none()
    }

    fn take(&mut self, side: &mut MapSide, wants: Wants) {
        if side.take_overlay_withdrawal() {
            (self.overlay, self.overlay_withdrawn) = (None, true);
        }
        if let Some(made) = side.take_overlay() {
            self.overlay = Some(made);
        }
        self.vision = side.vision();
        if let Some(overlay) = side.overlay() {
            self.overlay_hour = overlay.hour();
            if self.overlay_loading != overlay.loading() {
                self.overlay_loading = overlay.loading().to_vec();
            }
            if self.overlay_missed != overlay.missed() {
                self.overlay_missed = overlay.missed().to_vec();
            }
        }
        let side = &*side;
        self.labels = crate::layers::labels::stamped(side);
        self.windows += 1;
        if self.names != side.names() {
            self.names = side.names().to_vec();
        }
        self.drawn.clear();
        self.drawn.extend(side.names().iter().map(|name| side.draws(name)));
        self.on_screen = side.on_screen();
        self.untimed = side.untimed_hours();
        self.failure = side.failure().map(str::to_owned);
        if self.view != *side.view_shown() {
            self.view.clone_from(side.view_shown());
        }
        // While a compose has the layers away, the refusal the frame last took stands.
        if !side.composing() {
            self.vision_refused = side.vision_refused();
        }
        self.composing = side.composing();
        self.applied = side.applied();
        if self.mode.as_ref() != side.mode_on_screen() {
            self.mode = side.mode_on_screen().cloned();
        }
        self.frame.clone_from(side.last_frame());
        self.branch = side.branch();
        self.battles = crate::layers::sounds::drawn(side);
        self.radar = drawn::<crate::layers::radar::Radar>(side, "radar").and_then(crate::layers::radar::Radar::field);
        self.plan = drawn::<crate::layers::orders::Orders>(side, "orders").and_then(|orders| orders.plan().map(|(hour, plan)| (hour, Arc::clone(plan))));
        // The trade routes join the order field only while the units layer draws it.
        self.routes = side.draws("units").then(|| drawn::<crate::layers::convoys::Convoys>(side, "convoys")).flatten().and_then(crate::layers::convoys::Convoys::shown);
        self.left = crate::document::LeftToLayers::of(Some(side));
        self.placements = crate::layers::names::drawn(side);
        self.counters = crate::layers::counter_kinds::published(side);
        self.counter_kinds = crate::layers::counter_kinds::draws(side);
        self.units = crate::layers::units::prepared(side).map(Arc::clone);
        // Copied once a publication: a window between two takes the lists it already holds.
        let made = side.last_frame().made;
        if self.movement_of != Some(made) {
            self.movement_of = Some(made);
            self.movement.clear();
            self.pick_divisions.clear();
            if let Some(units) = crate::layers::units::of(side) {
                self.movement.extend(units.movement_positions().map(|(unit, place, scale, fade)| (unit, place.clone(), scale, fade)));
                self.pick_divisions = units.pick_divisions();
            }
        }
        self.ours = crate::layers::vision::drawn(side).and_then(crate::layers::vision::Vision::ours);
        self.engaged = drawn::<crate::layers::engagements::Engagements>(side, "engagements").and_then(crate::layers::engagements::Engagements::history).map(Arc::clone);
        self.take_highlight(side, wants.highlight);
        self.snow = drawn::<crate::layers::snow::Snow>(side, "snow").and_then(crate::layers::snow::Snow::summary);
        self.music = crate::layers::scripts::music(side);
        self.lines = wants.warnings.then(|| {
            let [limitations, reports] = crate::presentation::map_lines(side);
            Lines { political: crate::presentation::political_lines(side), limitations, reports }
        });
    }

    /// The picture the faction highlight is built over: the mode fills layer's fill and zone while
    /// it draws, else the political layer's, copied each time it changes while a highlight is
    /// wanted: the borders it is built from are never of an hour the screen has not reached.
    fn take_highlight(&mut self, side: &MapSide, wanted: bool) {
        use crate::highlight::{Over, Picture};
        if !side.draws("political") {
            self.highlight = None;
            return;
        }
        if let Some(fills) = drawn::<crate::layers::mode_fills::ModeFills>(side, "mode_fills").filter(|fills| fills.drawn()) {
            self.highlight = fills.picture().map(|(regions, zone, hour)| (1 << 62 | u64::from(hour as u32), Over::Mode(regions, zone)));
            self.highlight_hour = None;
            return;
        }
        let Some(political) = drawn::<crate::layers::political::Political>(side, "political") else {
            self.highlight = None;
            return;
        };
        if !wanted {
            self.pictures.clear();
            return;
        }
        // Copied once each time it changes, dated at the hour the side settled it; the highlight
        // is built over the newest copy the frames' hour has reached, never a later one.
        let key = 1 << 63 | (political.changed() & !(1 << 63));
        if let Some(applied) = side.applied()
            && self.pictures.back().is_none_or(|&(_, held, _)| held != key)
        {
            if self.pictures.len() == PICTURES_HELD {
                self.pictures.pop_front();
            }
            self.pictures.push_back((applied, key, Arc::new(Picture::of(political))));
        }
        // The newest copy the frames' hour has reached that stands on the borders the frames wrote
        // last: one copied before a border change they wrote would put the older borders back.
        let (drawn, plans) = (side.on_screen().and_then(|on| on.drawn), political.plans_shown());
        let fits = |hour: Hour| drawn.is_some_and(|drawn| hour <= drawn) && plans.is_none_or(|plans| hour >= plans);
        let Some((_, key, picture)) = self.pictures.iter().rev().find(|&&(hour, ..)| fits(hour)) else {
            self.highlight = None;
            return;
        };
        if self.highlight.as_ref().is_none_or(|(at, _)| at != key) {
            self.highlight = Some((*key, Over::Layer(Arc::clone(picture))));
        }
        self.highlight_hour = self.pictures.iter().find(|&&(_, held, _)| held == *key).map(|&(hour, ..)| hour);
        let key = *key;
        while self.pictures.front().is_some_and(|&(_, held, _)| held != key) {
            self.pictures.pop_front();
        }
    }
}
