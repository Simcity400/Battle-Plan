//! Persistent map-icon pools, advanced once per presented frame.

use crate::map::gui_resources::count_noted;
use crate::map::gui_resources::count_refused;
use crate::map::native_windows::NativeWindows;
#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap};

use installed_content_core::gui::native;

use super::{
    native_gui::{
        self,
        lifecycle::{
            Admission, Instance, MapIconPool, Pool, Rewrote, SlotId, Tick, Unknown, Visible,
        },
    },
    native_resources::Context,
};

/// the most one frame advances the GUI's clocks.
pub const GUI_DELTA_CLAMP: f32 = 0.1;

/// Cap one presented GUI step.
pub fn frame(last: &mut Option<f32>, time: f32) -> Tick {
    let dt = last.map_or(0.0, |last| (time - last).clamp(0.0, GUI_DELTA_CLAMP));
    *last = Some(time);
    Tick { time, dt }
}

/// Anchor and rounds the projected window.
pub fn point(screen: [f32; 2], size: [i32; 2], alignment: u8) -> [i32; 2] {
    std::array::from_fn(|axis| {
        let (centre, far) = if axis == 0 { (0x10, 0x20) } else { (2, 4) };
        let offset = if alignment & centre != 0 {
            size[axis] as f32 * 0.5
        } else if alignment & far != 0 {
            size[axis] as f32
        } else {
            0.0
        };
        let value = screen[axis] - offset;
        truncate_int(if value >= 0.0 {
            value + 0.5
        } else {
            value - 0.5
        })
    })
}

/// Include the viewport edges.
pub fn rectangle_inside(point: [i32; 2], size: [i32; 2], viewport: [i32; 2]) -> bool {
    on_screen(
        point.map(|v| v as f32),
        size.map(|v| v as f32),
        viewport.map(|v| v as f32),
    )
}

/// Whether a window is laid out for a view: on screen, or within the pad a moved camera may
/// reach before the overlay is placed again ([`crate::map::anchored::PAD`]). A window the pad
/// admits draws nothing a watcher sees and is already there when the camera brings it in, which
/// is what lets a frame answer a moved camera by offsetting the windows it has.
pub fn admitted(origin: [f32; 2], size: [f32; 2], extent: [f32; 2]) -> bool {
    let pad = crate::map::anchored::PAD;
    on_screen(
        [origin[0] + pad, origin[1] + pad],
        size,
        [extent[0] + 2.0 * pad, extent[1] + 2.0 * pad],
    )
}

/// [`admitted`] for a window placed at whole pixels.
pub fn rectangle_admitted(point: [i32; 2], size: [i32; 2], viewport: [i32; 2]) -> bool {
    admitted(
        point.map(|v| v as f32),
        size.map(|v| v as f32),
        viewport.map(|v| v as f32),
    )
}

/// Convert to a signed integer toward zero, returning `0x80000000` for a value
/// out of range or not a number.
fn truncate_int(value: f32) -> i32 {
    if value.is_finite() && (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    }
}

/// Viewport admission uses the owning province and icon anchor with the
/// shared camera schedule. Each icon family retains its own admission gate.
pub fn admission(
    dirty: bool,
    point: [i32; 2],
    inside: impl Fn([i32; 2]) -> bool,
) -> (Admission, bool) {
    if inside(point) {
        (Admission::Inside(point), false)
    } else if dirty {
        (Admission::Inside(point), true)
    } else {
        (Admission::Outside, false)
    }
}

pub const POOL_EXHAUSTED: &str =
    "a map icon's pool was exhausted; its owner asks again the next frame";

/// Whether a screen rectangle can contribute anything to the frame.
///
/// `extent` is the frame's size in interface pixels.
pub fn on_screen(origin: [f32; 2], size: [f32; 2], extent: [f32; 2]) -> bool {
    origin[0] + size[0] >= 0.0
        && origin[1] + size[1] >= 0.0
        && origin[0] <= extent[0]
        && origin[1] <= extent[1]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retry {
    pub name: String,
    pub requests: usize,
    pub rounds: usize,
}

impl Retry {
    pub fn warning(&self) -> String {
        format!(
            "Map icon dump settling: {} {} requests still pending at the native growth-round bound",
            self.requests, self.name
        )
    }
}

/// The allocator's remaining growth rounds, summed because shares
/// one worker handle across types. A frozen dump cannot introduce new demand.
#[derive(Default)]
pub struct DumpFrames {
    remaining: Option<usize>,
}

impl DumpFrames {
    pub fn next(&mut self, retries: &[Retry]) -> bool {
        if retries.is_empty() {
            return false;
        }
        let remaining = self
            .remaining
            .get_or_insert_with(|| retries.iter().map(|r| r.rounds).sum());
        if *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        true
    }
}

/// One owner request and its projected position, with the province's dirty-bit bypass.
#[derive(Debug)]
pub struct Owner<K = u32> {
    pub id: K,
    pub screen: [f32; 2],
    pub dirty: bool,
}

/// The native template, type pool and alignment of one consumer family.
pub struct Plan<'a, K = u32> {
    pub name: &'a str,
    pub kind: MapIconPool,
    pub alignment: u8,
    pub owners: &'a [Owner<K>],
}

/// One visible owner's layout, before the consumer assigns its draw layer.
pub struct Drawn<K = u32> {
    pub owner: K,
    pub origin: [i32; 2],
    pub layout: native_gui::Layout,
}

/// Layout and count diagnostics for one visible window, shared with naval grid holders.
pub fn visible_layout<K: Copy>(
    icon: &Visible<'_, K>,
    root: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Option<Drawn<K>> {
    visible_layout_regions(icon, root, name, resources, notes, false)
}

fn visible_layout_regions<K: Copy>(
    icon: &Visible<'_, K>,
    root: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
    regions: bool,
) -> Option<Drawn<K>> {
    if let Some(why) = icon.withheld {
        *notes
            .entry(format!("{name} is not drawn: {why}"))
            .or_default() += 1;
        return None;
    }
    match icon.instance.layout_with_regions(root, resources, None, regions) {
        Ok(Some(layout)) => {
            count_noted(notes, name, &layout.diagnostics);
            Some(Drawn {
                owner: *icon.owner,
                origin: icon.instance.position(),
                layout,
            })
        }
        Ok(None) => None,
        Err(why) => {
            count_refused(notes, name, &why);
            None
        }
    }
}

/// One pool's count since the last [`take_counts`]: `[presented, stood still, kept, windows laid
/// out, windows kept]`, and why the frames that laid a window out again did so, by reason.
pub type PoolCount = (String, [u64; 5], Vec<(&'static str, u64)>);

static COUNTS: std::sync::Mutex<Vec<PoolCount>> = std::sync::Mutex::new(Vec::new());

/// One presented frame of one pool, as [`take_counts`] adds it up.
#[derive(Default)]
struct Tally {
    still: bool,
    kept: bool,
    why: Option<&'static str>,
    laid: u64,
    held: u64,
    /// Windows kept at a new origin: their owner moved and their layout did not.
    moved: u64,
}

impl Tally {
    /// The first reason the frame laid a window out again.
    fn because(&mut self, why: &'static str) {
        self.why.get_or_insert(why);
    }
}

fn count(name: &str, tally: &Tally) {
    if crate::view_record::on() {
        let section = if super::redraws::apart_now() { "overlay worker" } else { "overlay" };
        crate::view_record::add(section, "windows_laid", tally.laid);
        crate::view_record::add(section, "windows_kept", tally.held);
        crate::view_record::add(section, "windows_kept_moved", tally.moved);
    }
    if !replay_frame::cpu::enabled() {
        return;
    }
    let mut counts = COUNTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = match counts.iter().position(|(held, ..)| held == name) {
        Some(at) => at,
        None => {
            counts.push((name.to_owned(), [0; 5], Vec::new()));
            counts.len() - 1
        }
    };
    let (_, counted, whys) = &mut counts[at];
    counted[0] += 1;
    counted[1] += u64::from(tally.still);
    counted[2] += u64::from(tally.kept);
    counted[3] += tally.laid;
    counted[4] += tally.held;
    if !tally.kept {
        let why = tally.why.unwrap_or("not kept by its caller");
        match whys.iter_mut().find(|(held, _)| *held == why) {
            Some((_, count)) => *count += 1,
            None => whys.push((why, 1)),
        }
    }
}

/// Per pool window name, since the last call. A pool stood still when no window on screen was
/// laid out again; it kept its windows when it laid none out again ([`PoolView::present_kept`]);
/// a pool whose caller keeps nothing stands still when it was settled at the frame before with
/// the same windows, viewport and owners (ids, screen points, dirty bits). The reasons name the
/// first change a frame met.
pub fn take_counts() -> Vec<PoolCount> {
    let mut counts = COUNTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let taken = counts.iter().filter(|(_, counted, _)| counted[0] > 0).cloned().collect();
    for (_, counted, whys) in counts.iter_mut() {
        *counted = [0; 5];
        whys.clear();
    }
    taken
}

/// Pool membership, window history and presentation clock for one native icon type.
#[derive(Debug)]
pub struct PoolView<K = u32> {
    built: Option<(NativeWindows, [i32; 2])>,
    pool: Option<Pool<K>>,
    last: Option<f32>,
    active: bool,
    /// Current request geometry; the map retains its allocation across camera frames.
    owners: HashMap<K, ([f32; 2], bool)>,
    /// At the last presented frame no window changed on its own, no dirty province admitted one
    /// and the pool noted nothing of its own.
    settled: bool,
    /// Why it was not settled at the last presented frame.
    unsettled: &'static str,
    /// The windows the last frame showed, for a caller that keeps them ([`Self::present_kept`]).
    kept: Option<Kept<K>>,
    /// Why the last presented frame laid its windows out rather than keeping them.
    unkept: Option<&'static str>,
    /// What [`Self::present_recorded`]'s update last wrote to each live owner's window: its slot
    /// and the owner's values the update read.
    records: HashMap<K, (SlotId, Box<dyn std::any::Any + Send + Sync>)>,
    /// The layouts made so far, which numbers each ([`Self::drawn_generations`]).
    generations: u64,
    /// What the windows drew, for a caller that paints them ([`Self::painted`]).
    painted: Painted<K, Option<super::free_icons::IconAnchor>>,
    /// A dirty province admitted an icon off screen at the last presented frame: the next frame
    /// presents the pool again, though nothing it shows changed.
    bypassed: bool,
    /// The frame presents the pool for its manager's pass alone ([`Self::set_quiet`]).
    quiet: bool,
    /// An owner the next present names stands in a province a coming phase cleans
    /// ([`Self::set_cleaning`]).
    cleaning: bool,
    /// Each window drawn on screen at the last frame, by owner: its position and its owner's
    /// screen point, while the flight record is kept ([`Self::note_continuity`]).
    last_drawn: HashMap<K, ([i32; 2], [f32; 2])>,
}

struct Kept<K> {
    /// Every window shown, in the manager's visible order.
    windows: Vec<Shown<K>>,
    /// Whether the next frame may keep them: the frame ticked the windows it wrote.
    keep: bool,
    /// Whether every window the pass showed is here: a quiet frame lays none out, so one the
    /// pass changed is not, and the next frame walks the pool to lay it out.
    whole: bool,
}

/// One window a frame showed, with what laying it out made.
struct Shown<K> {
    slot: SlotId,
    owner: K,
    /// Where the manager's pass placed it.
    position: [i32; 2],
    drawn: Option<Drawn<K>>,
    /// What laying it out noted, counted again on every frame that shows it.
    notes: BTreeMap<String, usize>,
    /// Which layout of the pool it is: another layout of any window has another.
    generation: u64,
}

impl<K> std::fmt::Debug for Kept<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kept")
            .field("windows", &self.windows.len())
            .field("keep", &self.keep)
            .finish()
    }
}

impl<K> Default for PoolView<K> {
    fn default() -> Self {
        Self {
            built: None,
            pool: None,
            last: None,
            active: false,
            owners: HashMap::new(),
            settled: false,
            unsettled: "no frame presented yet",
            kept: None,
            unkept: None,
            records: HashMap::new(),
            generations: 0,
            painted: Painted::default(),
            bypassed: false,
            quiet: false,
            cleaning: false,
            last_drawn: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + std::hash::Hash> PoolView<K> {
    /// Every live window's owner and slot.
    pub fn live(&self) -> Vec<(K, SlotId)> {
        self.pool.as_ref().map_or_else(Vec::new, |pool| {
            pool.live()
                .map(|(owner, slot, _)| (owner.clone(), slot))
                .collect()
        })
    }
}

impl<K: Copy + Eq + std::hash::Hash> PoolView<K> {
    pub fn same<R>(&self, context: &Context<'_, R>) -> bool {
        self.built.as_ref().is_some_and(|(windows, viewport)| {
            windows.same(context.windows) && *viewport == context.viewport
        })
    }

    pub fn take_running(&mut self) -> bool {
        std::mem::take(&mut self.active) | std::mem::take(&mut self.bypassed)
    }

    /// Owner requests may change next frame even while the window itself is settled.
    pub fn keep_running(&mut self) {
        self.bypassed = true;
    }

    /// Whether a window it shows changes on its own, or a request waits for a window the next pass
    /// may show: what makes the next frame draw again, beside what only asks for its manager's
    /// pass ([`Self::take_running`]).
    pub fn running_visibly(&self) -> bool {
        self.active || self.pool.as_ref().is_some_and(Pool::requests_pending)
    }

    /// Presents the next frames for the manager's pass alone (`true`): requests, admission,
    /// erasures, the writes and ticks of the windows the pass changed, exactly as a drawn frame
    /// runs them, and no layout; a window the pass changed is laid out on the next drawn frame.
    /// The caller keeps what it drew before.
    pub fn set_quiet(&mut self, quiet: bool) {
        self.quiet = quiet;
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    /// Whether an owner the next present names stands in a province dirty out of view
    /// ([`super::province_schedule::Schedule::cleans`]): its window kept off screen is erased when
    /// the phase cleans it, so the pool asks for frames until then. Without it, a pass that changed
    /// nothing is the pass every later frame would make, and the pool asks for none.
    pub fn set_cleaning(&mut self, cleaning: bool) {
        self.cleaning = cleaning;
    }



    pub fn retry(&self) -> Option<Retry> {
        self.pool.as_ref().and_then(Pool::retry)
    }

    pub fn present(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        notes: &mut BTreeMap<String, usize>,
    ) -> Vec<Drawn<K>> {
        self.present_regions(context, plan, initialize, update, notes, false)
    }

    pub fn present_with_regions(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        notes: &mut BTreeMap<String, usize>,
    ) -> Vec<Drawn<K>> {
        self.present_regions(context, plan, initialize, update, notes, true)
    }

    /// [`Self::present`] for a caller that keeps what the pool laid out ([`Self::drawn`]), window
    /// by window. The manager's pass runs on every frame, so requests, erasures and slots move as
    /// they do today, off screen too. A window is written, ticked and laid out again only when it
    /// changed since the frame that laid it out:
    /// - the pass showed it, gave its slot to another owner or placed it elsewhere;
    /// - it changes on its own ([`Instance::active`]): a clock, pulse, animation record, fade,
    ///   move, smooth scroll, pending sprite, unsettled state effect, or no tick yet;
    /// - `unchanged` says its owner's update would write something else.
    ///
    /// A window the pass erased, and any window that changes on its own, is ticked too. Every
    /// window is written and laid out on the frame after one with no clock, a failed frame, a
    /// growth, and when the windows or the viewport change. A kept window's clocked parts keep the
    /// `Time` of the frame that laid them out; a settled draw does not change on the clock
    /// ([`native_gui::Shading::changing`]). Whether it laid out no window again.
    pub fn present_kept(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        unchanged: impl Fn(&K, SlotId) -> bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        self.present_kept_regions(context, plan, initialize, update, unchanged, false, notes)
    }

    /// [`Self::present_kept`] for a caller whose update reads nothing of an owner but `value`:
    /// the pool records what each window's update was given ([`Self::forget_writes`] drops every
    /// record, for a caller whose update reads something else that changed). `regions` lays the
    /// windows out with their regions ([`Self::present_with_regions`]).
    #[allow(clippy::too_many_arguments, reason = "one presented frame's inputs, with what the update reads")]
    pub fn present_recorded<V: PartialEq + Send + Sync + 'static>(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        mut update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        value: impl Fn(&K) -> V,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        if !self.same(context) {
            self.records.clear();
        }
        let records = std::cell::RefCell::new(std::mem::take(&mut self.records));
        let kept = self.present_kept_regions(
            context,
            plan,
            initialize,
            |owner, slot, instance, root| {
                records.borrow_mut().remove(&owner);
                let wrote = update(owner, slot, instance, root);
                if wrote.is_ok() {
                    records
                        .borrow_mut()
                        .insert(owner, (slot, Box::new(value(&owner))));
                }
                wrote
            },
            |owner, slot| {
                records.borrow().get(owner).is_some_and(|(held, written)| {
                    *held == slot && written.downcast_ref::<V>() == Some(&value(owner))
                })
            },
            regions,
            notes,
        );
        let mut records = records.into_inner();
        if let Some(pool) = &self.pool
            && records.len() > plan.owners.len()
        {
            records.retain(|owner, (slot, _)| pool.slot_of(owner) == Some(*slot));
        }
        self.records = records;
        kept
    }

    /// Drops what [`Self::present_recorded`] recorded: every window is written again.
    pub fn forget_writes(&mut self) {
        self.records.clear();
    }

    #[allow(clippy::too_many_arguments, reason = "one presented frame's inputs, with the caller's keep test")]
    fn present_kept_regions(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        unchanged: impl Fn(&K, SlotId) -> bool,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        if self.nothing_due(context, plan, &unchanged) {
            let kept = self.kept.as_ref().expect("the gate found the windows kept");
            let tally = Tally {
                still: true,
                kept: true,
                held: kept.windows.len() as u64,
                ..Tally::default()
            };
            for window in &kept.windows {
                add_notes(notes, &window.notes);
            }
            count(plan.name, &tally);
            self.unkept = None;
            self.active = false;
            // The pass it stands in for granted, wrote and erased nothing with no owner in a
            // province a phase will clean (the gate's own conditions): it asks for no frame.
            self.bypassed = false;
            crate::map::redraws::note("pool.nothing_due", 1);
            return true;
        }
        let held = self
            .kept
            .take()
            .filter(|kept| kept.keep)
            .map(|kept| {
                kept.windows
                    .into_iter()
                    .map(|window| (window.slot, window))
                    .collect()
            });
        let mut shown = Vec::with_capacity(plan.owners.len());
        let mut tally = Tally::default();
        let keep = match self.present_windows(
            context, plan, initialize, update, &unchanged, regions, notes, held, &mut shown, &mut tally,
        ) {
            Ok(keep) => keep,
            Err(why) => {
                if !plan.owners.is_empty() {
                    *notes
                        .entry(format!("{} is not drawn: {why}", plan.name))
                        .or_default() += plan.owners.len();
                }
                tally.kept = false;
                tally.still = false;
                tally.because("its frame failed");
                false
            }
        };
        for window in &shown {
            add_notes(notes, &window.notes);
        }
        count(plan.name, &tally);
        self.unkept = (!tally.kept).then(|| tally.why.unwrap_or("not kept by its caller"));
        self.kept = Some(Kept {
            windows: shown,
            keep,
            whole: !self.quiet,
        });
        tally.kept
    }

    /// Whether this frame's pass would find nothing to do, so the frame does not walk the pool:
    /// the windows are this viewport's, every window the last pass showed is kept, the owners
    /// stand as they stood (the same ids at the same screen points, none dirty, none in a
    /// province a phase will clean), no request waits for a window, no window changes on its own,
    /// and no owner's update would write anything else. The pass would then admit, erase and
    /// place exactly the windows the pass before it did, leaving every window where it stands,
    /// writing none and ticking none. What it would not leave as it was is which free window the
    /// next new owner is given, which is no part of what any window draws.
    fn nothing_due(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        unchanged: &impl Fn(&K, SlotId) -> bool,
    ) -> bool {
        if let Some(why) = self.why_still(context) {
            crate::map::redraws::note(why, 1);
            return false;
        }
        if !self.owners_stand(plan.owners) {
            crate::map::redraws::note("pool.due.owners", 1);
            return false;
        }
        let kept = self.kept.as_ref().expect("stands_still found windows kept");
        let due = kept
            .windows
            .iter()
            .all(|window| unchanged(&window.owner, window.slot));
        if !due {
            crate::map::redraws::note("pool.due.offers", 1);
        }
        due
    }

    /// Why the last presented frame laid its windows out rather than keeping them; none when kept.
    pub fn unkept(&self) -> Option<&'static str> {
        self.unkept
    }

    /// The owner and window of each window the last pass showed ([`Self::present_kept`]).
    pub fn kept_windows(&self) -> impl Iterator<Item = (K, SlotId)> + '_ {
        self.kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .map(|window| (window.owner, window.slot))
    }

    /// Whether a pass over the owners of the last one would find nothing to do: the windows are
    /// this viewport's, every window that pass showed is kept and whole, no owner of it stood in
    /// a dirty province or in one a phase will clean, no request waits for a window and no window
    /// changes on its own. A caller that keeps its own drawing tests its owners itself (they are
    /// the same ids at the same screen points) and then draws nothing.
    pub fn stands_still(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
    ) -> bool {
        self.why_still(context).is_none()
    }

    /// What [`Self::stands_still`] found standing in the way, named for the frame's record.
    fn why_still(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
    ) -> Option<&'static str> {
        if context.time.is_none() {
            return Some("pool.due.no_clock");
        }
        if self.cleaning {
            return Some("pool.due.cleaning");
        }
        if self.quiet {
            return Some("pool.due.quiet");
        }
        if !self.same(context) {
            return Some("pool.due.other_windows");
        }
        let Some(kept) = self.kept.as_ref() else {
            return Some("pool.due.no_frame_kept");
        };
        if !kept.keep {
            return Some("pool.due.frame_not_kept");
        }
        if !kept.whole {
            return Some("pool.due.frame_not_whole");
        }
        let Some(pool) = self.pool.as_ref() else {
            return Some("pool.due.no_pool");
        };
        if pool.requests_pending() {
            return Some("pool.due.requests");
        }
        if pool.retry().is_some() {
            return Some("pool.due.retry");
        }
        if pool.active() {
            return Some("pool.due.active");
        }
        None
    }

    /// Whether `owners` are the owners of the last pass, each at the same screen point with the
    /// same dirty bit: the pass then snaps each to the same whole pixel, admits, erases and
    /// places exactly what it placed, and asks for the frames it asked for.
    pub fn owners_stand(&self, owners: &[Owner<K>]) -> bool {
        self.owners.len() == owners.len()
            && owners.iter().all(|owner| {
                self.owners.get(&owner.id).is_some_and(|&(screen, dirty)| {
                    dirty == owner.dirty && screen.map(f32::to_bits) == owner.screen.map(f32::to_bits)
                })
            })
    }

    /// The caller's own frame found nothing due of its whole family and did not present the pool
    /// ([`Self::stands_still`]): the pool stood still and kept every window it shows, which its
    /// count and its frames asked for say, as a presented pass that found nothing would.
    pub fn stood_still(&mut self, name: &str) {
        let tally = Tally {
            still: true,
            kept: true,
            held: self.kept.as_ref().map_or(0, |kept| kept.windows.len() as u64),
            ..Tally::default()
        };
        count(name, &tally);
        self.unkept = None;
        self.active = false;
        self.bypassed = false;
    }

    /// The windows [`Self::present_kept`] shows this frame, in the manager's visible order.
    pub fn drawn(&self) -> impl Iterator<Item = &Drawn<K>> + '_ {
        self.kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .filter_map(|window| window.drawn.as_ref())
    }

    /// [`Self::drawn`] with which layout each is: a window shown with the generation it had is the
    /// layout it had, at the same origin ([`Painted`]).
    pub fn drawn_generations(&self) -> impl Iterator<Item = (&Drawn<K>, u64)> + '_ {
        self.kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .filter_map(|window| window.drawn.as_ref().map(|drawn| (drawn, window.generation)))
    }

    /// [`Self::drawn_generations`], and what the windows drew under their anchor: a caller paints
    /// each window through [`Painted::paint`] and calls [`Painted::finish`] after the last.
    #[allow(clippy::type_complexity, reason = "the windows shown and the store their quads are painted from")]
    pub fn painted(
        &mut self,
    ) -> (Vec<(&Drawn<K>, u64)>, &mut Painted<K, Option<super::free_icons::IconAnchor>>) {
        let drawn = self
            .kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .filter_map(|window| window.drawn.as_ref().map(|drawn| (drawn, window.generation)))
            .collect();
        (drawn, &mut self.painted)
    }

    /// [`Self::present_kept`]'s frame: the windows it shows, in `shown`; whether the next frame
    /// may keep them.
    #[allow(clippy::too_many_arguments, reason = "one presented frame's inputs, the windows kept and what it shows")]
    fn present_windows(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        mut update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        unchanged: &dyn Fn(&K, SlotId) -> bool,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
        held: Option<HashMap<SlotId, Shown<K>>>,
        shown: &mut Vec<Shown<K>>,
        tally: &mut Tally,
    ) -> Result<bool, String> {
        let root = context
            .windows
            .gui_type(plan.name.as_bytes())
            .ok_or("no GUI template is registered")?;
        let mut held = held;
        if !self.same(context) {
            tally.because(if self.built.is_none() {
                "no frame presented yet"
            } else {
                "other windows or viewport"
            });
            // A replacement before a presented frame has no window history to redraw.
            if context.time.is_none() {
                return Ok(false);
            }
            // Layouts keep their numbers across the rebuild: a number is never a second layout. What
            // the caller set for this present (its owners' provinces) stands.
            let (generations, cleaning) = (self.generations, self.cleaning);
            // The painting stays: a window painted under the old pool and not again is gone.
            let painted = std::mem::take(&mut self.painted);
            *self = Self::default();
            (self.generations, self.cleaning) = (generations, cleaning);
            self.painted = painted;
            self.pool = Some(Pool::new(
                root,
                context.viewport,
                plan.kind,
                initialize,
                context.resources,
            )?);
            self.built = Some((context.windows.clone(), context.viewport));
            held = None;
        }
        let pool = self.pool.as_mut().expect("the matching view was built");
        let owners = &mut self.owners;
        let manager_profile = crate::profiling::span("dc_pool_manager");
        owners.clear();
        owners.extend(
            plan.owners
                .iter()
                .map(|owner| (owner.id, (owner.screen, owner.dirty))),
        );
        let Some(time) = context.time else {
            // New offer values on the existing windows, without another manager pass or tick.
            tally.because("no presentation clock");
            drop(manager_profile);
            let updated: Vec<_> = pool
                .visible()
                .filter(|icon| owners.contains_key(icon.owner))
                .map(|icon| (*icon.owner, icon.slot))
                .collect();
            for (owner, slot) in updated {
                let instance = pool.get(slot).expect("updated slot is live");
                match update(owner, slot, instance, root) {
                    Ok(()) => pool.rewrote(slot, Rewrote::Update),
                    Err(why) => pool.refuse(slot, Unknown::Update, why.into()),
                }
            }
            for icon in pool.visible().filter(|icon| owners.contains_key(icon.owner)) {
                shown.push(lay_out(&icon, root, plan.name, context.resources, regions, &mut self.generations));
                tally.laid += 1;
            }
            return Ok(false);
        };
        self.active = false;
        self.bypassed = false;
        let size = native_gui::container_frame(root, Some(context.viewport))?.size;
        for owner in plan.owners {
            if pool.request(owner.id).is_none() {
                *notes
                    .entry(format!("{} is not drawn: {}", plan.name, POOL_EXHAUSTED))
                    .or_default() += 1;
            }
        }
        let pass = pool.update(root, |owner, _| {
            owners
                .get(owner)
                .map_or(Admission::Empty, |&(screen, dirty)| {
                    let point = point(screen, size, plan.alignment);
                    let (admission, dirty) = admission(dirty, point, |point| {
                        rectangle_admitted(point, size, context.viewport)
                    });
                    self.bypassed |= dirty;
                    admission
                })
        })?;
        let tick = frame(&mut self.last, time);
        let held = match held {
            Some(held) => held,
            None => {
                // Every window written, ticked and laid out, as today.
                tally.because("its last frame is not kept");
                drop(manager_profile);
                for (owner, slot) in pass.updated {
                    let instance = pool.get(slot).expect("updated slot is live");
                    match update(owner, slot, instance, root) {
                        Ok(()) => pool.rewrote(slot, Rewrote::Update),
                        Err(why) => pool.refuse(slot, Unknown::Update, why.into()),
                    }
                }
                pool.tick(root, context.resources, tick)?;
                count_noted(notes, plan.name, &pool.take_diagnostics());
                for icon in pool.visible().filter(|icon| owners.contains_key(icon.owner)) {
                    shown.push(lay_out(&icon, root, plan.name, context.resources, regions, &mut self.generations));
                    tally.laid += 1;
                }
                self.active |= pool.active();
                return Ok(true);
            }
        };
        let mut held = held;
        let on_screen = |position: [i32; 2]| rectangle_inside(position, size, context.viewport);
        let mut touched = Vec::new();
        let mut written = Vec::new();
        for &(owner, slot) in &pass.updated {
            let instance = pool.instance(slot).expect("updated slot is live");
            let why = match held.get(&slot) {
                Some(window) if window.owner != owner => "a window was taken by another owner",
                None if !on_screen(instance.position()) => {
                    "a dirty province kept an icon off screen"
                }
                None => "a window was shown",
                Some(_) if instance.active() => "a window changed on its own",
                Some(_) if !unchanged(&owner, slot) => "an owner's offer changed",
                Some(_) => continue,
            };
            tally.because(why);
            written.push((owner, slot));
            touched.push(slot);
        }
        for erased in &pass.erased {
            if held.contains_key(&erased.slot) {
                tally.because("a window was erased");
                touched.push(erased.slot);
            }
        }
        drop(manager_profile);
        for (owner, slot) in written {
            let instance = pool.get(slot).expect("updated slot is live");
            match update(owner, slot, instance, root) {
                Ok(()) => pool.rewrote(slot, Rewrote::Update),
                Err(why) => pool.refuse(slot, Unknown::Update, why.into()),
            }
        }
        let before = touched.len();
        touched.extend(pool.active_slots());
        if touched.len() > before {
            tally.because("a window changed on its own");
        }
        touched.sort_unstable();
        touched.dedup();
        if pool.requests_pending() {
            tally.because("a request went ungranted");
        }
        pool.tick_slots(root, context.resources, tick, &mut touched)?;
        let diagnostics = pool.take_diagnostics();
        if !diagnostics.is_empty() {
            tally.because("the pool grew");
        }
        count_noted(notes, plan.name, &diagnostics);
        let mut on_screen_laid = false;
        for icon in pool.visible().filter(|icon| owners.contains_key(icon.owner)) {
            let window = match touched.binary_search(&icon.slot) {
                Ok(_) => None,
                Err(_) => held.remove(&icon.slot),
            };
            match window {
                // A window whose owner only moved is its own layout at a new origin: the frame
                // keeps the layout and moves it, and the painting follows the origin.
                Some(mut window) => {
                    let position = icon.instance.position();
                    if window.position != position {
                        window.position = position;
                        if let Some(drawn) = &mut window.drawn {
                            drawn.origin = position;
                        }
                        tally.moved += 1;
                    }
                    shown.push(window);
                    tally.held += 1;
                }
                // A quiet frame lays nothing out: the next drawn frame finds it not held. One on
                // screen would show a frame late, so the next frame draws.
                None if self.quiet => {
                    if on_screen(icon.instance.position()) {
                        crate::map::redraws::note("quiet.shown_late", 1);
                        self.active = true;
                    }
                }
                None => {
                    on_screen_laid |= on_screen(icon.instance.position());
                    shown.push(lay_out(&icon, root, plan.name, context.resources, regions, &mut self.generations));
                    tally.laid += 1;
                }
            }
        }
        self.active |= touched
            .iter()
            .any(|&slot| pool.instance(slot).is_some_and(Instance::active));
        if crate::view_record::on() {
            let shows_animation = root.container().is_some_and(|container| container.show_animation != native::Animation::None);
            let mut drawn = HashMap::with_capacity(self.last_drawn.len());
            let (mut moved, mut jumps, mut popped, mut largest) = (0, 0, 0, 0.0_f32);
            for icon in pool.visible().filter(|icon| owners.contains_key(icon.owner)) {
                let position = icon.instance.position();
                if !on_screen(position) {
                    continue;
                }
                let screen = owners[icon.owner].0;
                match self.last_drawn.get(icon.owner) {
                    // Its move less its owner's projected move: the camera's motion and the
                    // owner's own cancel, leaving what the window did itself.
                    Some(&(was, was_screen)) => {
                        let own = [
                            (position[0] - was[0]) as f32 - (screen[0] - was_screen[0]),
                            (position[1] - was[1]) as f32 - (screen[1] - was_screen[1]),
                        ];
                        let distance = own[0].hypot(own[1]);
                        moved += u64::from(position != was);
                        largest = largest.max(distance);
                        // A window easing on its own (`active`) may move; one at rest moves only
                        // by the rounding of its anchored point.
                        jumps += u64::from(!icon.instance.active() && distance > 1.5);
                    }
                    None => popped += u64::from(shows_animation && !icon.instance.active()),
                }
                drawn.insert(*icon.owner, (position, screen));
            }
            self.last_drawn = drawn;
            crate::view_record::add("overlay", "windows_moved", moved);
            crate::view_record::add("overlay", "jumps", jumps);
            crate::view_record::add("overlay", "window_jumps", jumps);
            crate::view_record::add("overlay", "popped", popped);
            crate::view_record::add("overlay", "largest_window_move_px", largest.round() as u64);
        }
        // A pass that granted, wrote, erased and animated nothing, with no owner in a province a
        // phase will clean, is the pass every later frame would make: the pool asks for none.
        if touched.is_empty() && !pool.requests_pending() && !self.cleaning {
            self.bypassed = false;
        }
        if self.quiet {
            // A window animating after a quiet frame's pass makes the next frame draw (`active`).
            let moving = touched.iter().filter(|&&slot| pool.instance(slot).is_some_and(Instance::active)).count();
            crate::map::redraws::note("quiet.moving", moving as u64);
        }
        tally.still = !on_screen_laid;
        tally.kept = tally.laid == 0;
        Ok(true)
    }

    /// Whether `owners` are the last frame's, each at the same screen point and none dirty.
    fn stands(&self, owners: &[Owner<K>]) -> bool {
        self.owners.len() == owners.len()
            && owners.iter().all(|owner| {
                !owner.dirty
                    && self.owners.get(&owner.id).is_some_and(|&(screen, dirty)| {
                        !dirty && screen.map(f32::to_bits) == owner.screen.map(f32::to_bits)
                    })
            })
    }

    fn present_regions(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        mut update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        notes: &mut BTreeMap<String, usize>,
        regions: bool,
    ) -> Vec<Drawn<K>> {
        let mut drawn = Vec::with_capacity(plan.owners.len());
        let mut tally = Tally::default();
        let run = || -> Result<(), String> {
            let root = context
                .windows
                .gui_type(plan.name.as_bytes())
                .ok_or("no GUI template is registered")?;
            tally.why = if context.time.is_none() {
                Some("no presentation clock")
            } else if !self.settled {
                Some(self.unsettled)
            } else if !self.same(context) {
                Some("other windows or viewport")
            } else if plan.owners.iter().any(|owner| owner.dirty) {
                Some("an owner's province is dirty")
            } else if !self.stands(plan.owners) {
                Some("other owners or screen points")
            } else {
                None
            };
            tally.still = tally.why.is_none();
            self.settled = false;
            self.unsettled = "its frame failed";
            if !self.same(context) {
                // A replacement before a presented frame has no window history to redraw.
                if context.time.is_none() {
                    return Ok(());
                }
                *self = Self::default();
                self.pool = Some(Pool::new(
                    root,
                    context.viewport,
                    plan.kind,
                    initialize,
                    context.resources,
                )?);
                self.built = Some((context.windows.clone(), context.viewport));
            }
            let pool = self.pool.as_mut().expect("the matching view was built");
            let owners = &mut self.owners;
            let manager_profile = crate::profiling::span("dc_pool_manager");
            owners.clear();
            owners.extend(
                plan.owners
                    .iter()
                    .map(|owner| (owner.id, (owner.screen, owner.dirty))),
            );
            let (updated, tick) = if let Some(time) = context.time {
                self.active = false;
                self.bypassed = false;
                let size = native_gui::container_frame(root, Some(context.viewport))?.size;
                for owner in plan.owners {
                    if pool.request(owner.id).is_none() {
                        *notes
                            .entry(format!("{} is not drawn: {}", plan.name, POOL_EXHAUSTED))
                            .or_default() += 1;
                    }
                }
                let pass = pool.update(root, |owner, _| {
                    owners
                        .get(owner)
                        .map_or(Admission::Empty, |&(screen, dirty)| {
                            let point = point(screen, size, plan.alignment);
                            let (admission, dirty) = admission(dirty, point, |point| {
                                rectangle_inside(point, size, context.viewport)
                            });
                            self.bypassed |= dirty;
                            admission
                        })
                })?;
                let tick = frame(&mut self.last, time);
                (pass.updated, Some(tick))
            } else {
                // Apply new offer values to existing windows without another manager pass or tick.
                (
                    pool.visible()
                        .filter(|icon| owners.contains_key(icon.owner))
                        .map(|icon| (*icon.owner, icon.slot))
                        .collect(),
                    None,
                )
            };
            drop(manager_profile);
            for (owner, slot) in updated {
                let instance = pool.get(slot).expect("updated slot is live");
                match update(owner, slot, instance, root) {
                    Ok(()) => pool.rewrote(slot, Rewrote::Update),
                    Err(why) => pool.refuse(slot, Unknown::Update, why.into()),
                }
            }
            if let Some(tick) = tick {
                pool.tick(root, context.resources, tick)?;
            }
            let diagnostics = pool.take_diagnostics();
            count_noted(notes, plan.name, &diagnostics);
            for icon in pool.visible() {
                if !owners.contains_key(icon.owner) {
                    continue;
                }
                if let Some(layout) = visible_layout_regions(
                    &icon,
                    root,
                    plan.name,
                    context.resources,
                    notes,
                    regions,
                ) {
                    drawn.push(layout);
                }
            }
            let dirty = self.bypassed;
            self.active |= pool.active();
            self.unsettled = if tick.is_none() {
                "no presentation clock"
            } else if !diagnostics.is_empty() {
                "the pool grew"
            } else if dirty {
                "a dirty province kept an icon off screen"
            } else if self.active {
                "a window changed on its own"
            } else {
                ""
            };
            self.settled = self.unsettled.is_empty();
            Ok(())
        };
        if let Err(why) = run()
            && !plan.owners.is_empty()
        {
            *notes
                .entry(format!("{} is not drawn: {why}", plan.name))
                .or_default() += plan.owners.len();
        }
        tally.laid = drawn.len() as u64;
        count(plan.name, &tally);
        drawn
    }
}

/// A visible window laid out, with what laying it out noted.
fn lay_out<K: Copy + PartialEq>(
    icon: &Visible<'_, K>,
    root: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
    regions: bool,
    generations: &mut u64,
) -> Shown<K> {
    let mut notes = BTreeMap::new();
    let drawn = visible_layout_regions(icon, root, name, resources, &mut notes, regions);
    *generations += 1;
    Shown {
        slot: icon.slot,
        owner: *icon.owner,
        position: icon.instance.position(),
        drawn,
        notes,
        generation: *generations,
    }
}

/// Which families of map-icon windows have a pass to run on the next frame. A frame that presents
/// the pools for their passes alone presents these and no others: a family with nothing running
/// has nothing for a pass to do, and its manager's pass over its windows is work the screen does
/// not show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pools {
    pub naval: bool,
    pub labels: bool,
    pub pings: bool,
    pub raids: bool,
    pub intel: bool,
    pub bases: bool,
    pub land_combats: bool,
    pub resources: bool,
    pub occupation: bool,
    pub adjacency_rules: bool,
}

impl Pools {
    /// Every family, for a pass that presents them all.
    pub fn every() -> Self {
        Self {
            naval: true,
            labels: true,
            pings: true,
            raids: true,
            intel: true,
            bases: true,
            land_combats: true,
            resources: true,
            occupation: true,
            adjacency_rules: true,
        }
    }

    /// Whether any family has a pass to run.
    pub fn any(&self) -> bool {
        self.named().iter().any(|(running, _)| *running)
    }

    /// Each family's own, under the name the frame's record carries it by.
    pub fn named(&self) -> [(bool, &'static str); 10] {
        [
            (self.naval, "running.naval"),
            (self.labels, "running.labels"),
            (self.pings, "running.pings"),
            (self.raids, "running.raids"),
            (self.intel, "running.intel"),
            (self.bases, "running.bases"),
            (self.land_combats, "running.land_combats"),
            (self.resources, "running.resources"),
            (self.occupation, "running.occupation"),
            (self.adjacency_rules, "running.adjacency_rules"),
        ]
    }
}

/// The quads each window a caller keeps drew ([`PoolView::drawn_generations`]), handed again
/// while it shows the same layout under the same anchors: the compiled draws handed again are the
/// same allocations ([`super::sprites::Exact::native`]), which is how the sprite pass knows a quad
/// it prepared before. Only the layer, which follows the window's place among the others, is
/// written again. A window standing somewhere else is handed again too, offset by the whole
/// interface pixels between the two origins ([`crate::map::anchored::shift`]), which is what a
/// draw at the new origin would put there.
pub struct Painted<K, S> {
    held: HashMap<K, Paint<S>>,
    next: HashMap<K, Paint<S>>,
    /// This pass's windows drawn, handed again, and of those handed again the ones standing
    /// somewhere else, counted at [`Self::finish`].
    counted: [u64; 3],
    /// This pass's windows painted into the batch, added to the view record once at
    /// [`Self::finish`] (one lock a painting, not a window).
    passed: u64,
}

#[derive(Debug)]
struct Paint<S> {
    generation: u64,
    /// The origin these quads were drawn at, which a later painting offsets them from.
    origin: [i32; 2],
    stamp: S,
    /// The painting these quads came from, which a batch records so the sprite pass knows the
    /// window draws what it drew ([`super::sprites::next_painting`]).
    painting: u64,
    quads: Vec<super::sprites::Quad>,
}

impl<K, S> std::fmt::Debug for Painted<K, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Painted").field("held", &self.held.len()).field("next", &self.next.len()).finish_non_exhaustive()
    }
}

impl<K, S> Default for Painted<K, S> {
    fn default() -> Self {
        Self { held: HashMap::new(), next: HashMap::new(), counted: [0; 3], passed: 0 }
    }
}

impl<K: Copy + Eq + std::hash::Hash, S: PartialEq> Painted<K, S> {
    /// `owner`'s window of layout `generation`, standing at `origin`, onto `batch` at `layer`,
    /// its quads tagged as `stamp` says (the anchors `draw` sets): what it drew before when the
    /// layout, the origin and the stamp are the same, else `draw` (which pushes every quad at the
    /// batch's current layer, `layer`). A window whose draw fails holds nothing.
    #[allow(
        clippy::too_many_arguments,
        reason = "one window's painting: what it is, where it stands and how it is drawn"
    )]
    pub fn paint<E>(
        &mut self,
        key: super::sprites::WindowKey,
        owner: K,
        generation: u64,
        origin: [i32; 2],
        layer: u64,
        stamp: S,
        batch: &mut super::sprites::Batch,
        draw: impl FnOnce(&mut super::sprites::Batch) -> Result<(), E>,
    ) -> Result<(), E> {
        self.passed += 1;
        let previous = self.held.remove(&owner);
        if let Some(paint) = previous
            && paint.generation == generation
            && paint.stamp == stamp
        {
            let step = [origin[0] - paint.origin[0], origin[1] - paint.origin[1]];
            batch.at_window_standing(
                key,
                super::sprites::WindowStamp { painting: paint.painting, taken: layer },
                step,
            );
            let first = batch.quads.len();
            batch.quads.extend_from_slice(&paint.quads);
            for quad in &mut batch.quads[first..] {
                quad.layer = layer;
                if step != [0; 2] {
                    crate::map::anchored::shift(quad, step);
                }
            }
            batch.no_window();
            self.counted[1] += 1;
            self.counted[2] += u64::from(step != [0; 2]);
            self.next.insert(owner, paint);
            return Ok(());
        }
        let painting = super::sprites::next_painting();
        batch.at_window(key, super::sprites::WindowStamp { painting, taken: layer });
        let first = batch.quads.len();
        let drawn = draw(batch);
        batch.no_window();
        drawn?;
        self.counted[0] += 1;
        self.next.insert(
            owner,
            Paint { generation, origin, stamp, painting, quads: batch.quads[first..].to_vec() },
        );
        Ok(())
    }

    /// Once a frame, after the last [`Self::paint`]: a window not painted this frame holds nothing.
    pub fn finish(&mut self) {
        std::mem::swap(&mut self.held, &mut self.next);
        self.next.clear();
        let [painted, kept, standing] = std::mem::take(&mut self.counted);
        crate::map::redraws::note("windows.painted", painted);
        crate::map::redraws::note("windows.painted_kept", kept);
        crate::map::redraws::note("windows.painted_elsewhere", standing);
        let in_batch = std::mem::take(&mut self.passed);
        if in_batch > 0 {
            crate::view_record::add("overlay", "windows_in_batch", in_batch);
        }
    }

    /// Forgets every window's quads (the windows or the resources they draw with changed).
    pub fn clear(&mut self) {
        self.held.clear();
        self.next.clear();
    }
}

fn add_notes(notes: &mut BTreeMap<String, usize>, laid: &BTreeMap<String, usize>) {
    for (note, count) in laid {
        *notes.entry(note.clone()).or_default() += count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use {
        crate::map::test_support::gui::fixture, crate::map::test_support::gui::windows,
        native_gui::Resources,
    };

    const GUI: &[u8] = br#"guiTypes = { containerWindowType = { name = test_icon
        size = { width = 100 height = 100 }
        iconType = { name = icon spriteType = GFX_even }
    } }"#;

    #[test]
    fn dump_stalled_pools_reach_the_derived_bound_with_names_and_counts() {
        let retries = vec![
            Retry {
                name: "synthetic_labels".into(),
                requests: 17,
                rounds: 2,
            },
            Retry {
                name: "synthetic_bases".into(),
                requests: 16,
                rounds: 1,
            },
        ];
        let mut frames = DumpFrames::default();
        for _ in 0..3 {
            assert!(frames.next(&retries));
        }

        assert!(!frames.next(&retries));
        assert!(
            retries[0]
                .warning()
                .contains("17 synthetic_labels requests still pending")
        );
        assert!(
            retries[1]
                .warning()
                .contains("16 synthetic_bases requests still pending")
        );
        assert!(!frames.next(&[]));
    }

    #[test]
    fn combat_handles_keep_their_high_bits_in_pool_ownership() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(1.),
        };
        let ids = [1_u64, (1_u64 << 32) | 1];
        let owners = ids.map(|id| Owner {
            id,
            screen: [100., 200.],
            dirty: false,
        });
        let drawn = PoolView::<u64>::default().present(
            &context,
            &Plan {
                name: "test_icon",
                kind: MapIconPool::LandCombat,
                alignment: 0x12,
                owners: &owners,
            },
            Box::new(|_, _| Ok(())),
            |_, _, _, _| Ok(()),
            &mut BTreeMap::new(),
        );
        assert_eq!(drawn.len(), 2);
        assert_eq!(
            drawn.iter().map(|icon| icon.owner).collect::<BTreeSet<_>>(),
            ids.into_iter().collect()
        );
    }

    fn present(
        view: &mut PoolView,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        owner: u32,
        notes: &mut BTreeMap<String, usize>,
    ) -> Vec<Drawn> {
        view.present(
            context,
            &Plan {
                name: "test_icon",
                kind: MapIconPool::Ping,
                alignment: 0x0A,
                owners: &[Owner {
                    id: owner,
                    screen: [100., 200.],
                    dirty: false,
                }],
            },
            Box::new(|_, _| Ok(())),
            |_, _, _, _| Ok(()),
            notes,
        )
    }

    #[test]
    fn replacement_keeps_the_clock_and_membership_and_cannot_create_a_first_frame() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let mut context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: None,
        };
        let mut view = PoolView::default();
        let mut notes = BTreeMap::new();
        assert!(present(&mut view, &context, 1, &mut notes).is_empty());
        assert!(view.pool.is_none());
        context.time = Some(12.);
        assert_eq!(present(&mut view, &context, 1, &mut notes).len(), 1);
        let slot = view.pool.as_ref().unwrap().slot_of(&1);
        context.time = None;
        assert_eq!(present(&mut view, &context, 1, &mut notes).len(), 1);
        assert_eq!(view.last, Some(12.));
        assert_eq!(view.pool.as_ref().unwrap().slot_of(&1), slot);
        assert!(present(&mut view, &context, 2, &mut notes).is_empty());
        assert_eq!(view.pool.as_ref().unwrap().slot_of(&1), slot);
        assert_eq!(view.pool.as_ref().unwrap().slot_of(&2), None);
    }

    #[test]
    fn registry_identity_rebuilds_even_when_the_template_bytes_match() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let first = windows(GUI);
        let second = windows(GUI);
        let mut context = Context {
            resources: &resources,
            windows: &first,
            viewport: [2048, 1152],
            time: Some(1.),
        };
        let mut view = PoolView::default();
        let mut notes = BTreeMap::new();
        assert_eq!(present(&mut view, &context, 1, &mut notes).len(), 1);
        context.windows = &second;
        assert!(!view.same(&context));
        assert_eq!(present(&mut view, &context, 2, &mut notes).len(), 1);
        assert!(view.same(&context));
        assert_eq!(view.pool.as_ref().unwrap().slot_of(&1), None);
    }

    #[test]
    fn native_draws_nothing_diagnostics_keep_the_part_and_reason() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut resources = fixture();
        let mut sprite = resources.sprite(b"GFX_even").unwrap();
        sprite.texture = [0; 2];
        sprite.missing = Some("gfx/missing.dds did not load".into());
        resources.sprites.insert("GFX_even", sprite);
        let windows = windows(GUI);
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(1.),
        };
        let mut notes = BTreeMap::new();
        present(&mut PoolView::default(), &context, 1, &mut notes);
        assert!(notes.iter().any(|(reason, count)| *count > 0
            && reason.contains("test_icon draws nothing for")
            && reason.contains("icon")
            && reason.contains("gfx/missing.dds")));
    }

    #[test]
    fn shared_region_linger_retains_slot_then_final_sweep_stops_requests_and_running() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use super::super::province_schedule::Schedule;
        for selected in [false, true] {
            let resources = fixture();
            let windows = windows(GUI);
            let mut context = Context {
                resources: &resources,
                windows: &windows,
                viewport: [2048, 1152],
                time: Some(1.0),
            };
            let mut schedule = Schedule::default();
            let mut view = PoolView::default();
            let mut notes = BTreeMap::new();
            let mut slot = None;
            let last = if selected { 104 } else { 4 };
            for frame in 1..=last {
                if selected && frame == 2 {
                    schedule.set_selected(BTreeSet::from([3]));
                }
                if selected && frame == 4 {
                    schedule.set_selected(BTreeSet::new());
                }
                let in_view = frame == 1;
                schedule.advance([3], |_| in_view, false);
                context.time = Some(frame as f32);
                let listed = schedule.listed(3);
                let owners: Vec<_> = listed
                    .then_some(Owner {
                        id: 7,
                        screen: [100.0, 200.0],
                        dirty: false,
                    })
                    .into_iter()
                    .collect();
                let drawn = view.present(
                    &context,
                    &Plan {
                        name: "test_icon",
                        kind: MapIconPool::Ping,
                        alignment: 0x0A,
                        owners: &owners,
                    },
                    Box::new(|_, _| Ok(())),
                    |_, _, _, _| Ok(()),
                    &mut notes,
                );
                if listed && !in_view {
                    view.keep_running();
                }
                if frame < last {
                    assert_eq!(drawn.len(), 1);
                    let current = view.pool.as_ref().unwrap().slot_of(&7);
                    assert!(current.is_some());
                    if let Some(slot) = slot {
                        assert_eq!(current, Some(slot));
                    } else {
                        slot = current;
                    }
                    if !in_view {
                        assert!(view.take_running());
                    }
                } else {
                    assert!(drawn.is_empty());
                    assert_eq!(view.pool.as_ref().unwrap().slot_of(&7), None);
                    assert!(!view.take_running());
                }
                schedule.take_pending();
            }
        }
    }
    /// Presentation-dirty state can retain an off-screen icon until the rotating
    /// province update phase reaches and clears it.
    #[test]
    fn a_dirty_province_keeps_its_icon_off_screen() {
        let inside = |point: [i32; 2]| point[0] >= 0;
        assert_eq!(
            admission(false, [5, 5], inside),
            (Admission::Inside([5, 5]), false)
        );
        assert_eq!(
            admission(true, [5, 5], inside),
            (Admission::Inside([5, 5]), false)
        );
        assert_eq!(
            admission(false, [-50, 5], inside),
            (Admission::Outside, false)
        );
        assert_eq!(
            admission(true, [-50, 5], inside),
            (Admission::Inside([-50, 5]), true)
        );
        // Truncate toward zero; use 0x80000000 out of range or for a NaN.
        assert_eq!(truncate_int(-2.7), -2);
        assert_eq!(truncate_int(f32::NAN), i32::MIN);
        assert_eq!(truncate_int(3.0e9), i32::MIN);
    }

    /// A layout's parts with the clock constants a kept window leaves at the frame that laid it
    /// out (`Time`, the state effect's `since` and `AnimationTime`) set to zero.
    fn clockless<'a>(
        drawn: impl IntoIterator<Item = &'a Drawn>,
    ) -> Vec<(u32, [i32; 2], native_gui::Layout)> {
        drawn
            .into_iter()
            .map(|drawn| (drawn.owner, drawn.origin, clock_aside(drawn.layout.clone())))
            .collect()
    }

    /// `layout` with its clock constants (`Time`, `since`, `AnimationTime`) set to zero.
    fn clock_aside(mut layout: native_gui::Layout) -> native_gui::Layout {
        for part in &mut layout.parts {
            part.shading.time = 0.;
            part.shading.since = 0.;
            part.shading.animation = 0.;
        }
        layout
    }

    fn exact<'a>(drawn: impl IntoIterator<Item = &'a Drawn>) -> Vec<(u32, [i32; 2], native_gui::Layout)> {
        drawn
            .into_iter()
            .map(|drawn| (drawn.owner, drawn.origin, drawn.layout.clone()))
            .collect()
    }

    #[test]
    fn a_pool_keeps_each_window_that_stood_still_and_writes_again_only_those_that_changed() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id, screen, dirty| Owner { id, screen, dirty };
        let off = [-5000., 200.];
        let still = vec![
            owner(1, [100., 200.], false),
            owner(2, [400., 300.], false),
            // Off screen: requested and erased on every frame, as the game does.
            owner(3, off, false),
        ];
        let moved = vec![owner(1, [100., 200.], false), owner(2, [410., 300.], false), owner(3, off, false)];
        // An owner on screen whose province is dirty is admitted as it would be anyway.
        let dirty_on = vec![owner(1, [100., 200.], true), owner(2, [410., 300.], false), owner(3, off, false)];
        // The province schedule's rotation dirties the off-screen owner's province: the pass keeps
        // its window off screen, which the windows on screen do not see.
        let dirty_off = vec![owner(1, [100., 200.], false), owner(2, [410., 300.], false), owner(3, off, true)];
        // (owners, the updates would write the same, the viewport, windows written, kept, the
        // frame found nothing due and did not walk the pool)
        let frames: Vec<(&[Owner], bool, [i32; 2], usize, bool, bool)> = vec![
            (still.as_slice(), true, [2048, 1152], 2, false, false),
            (still.as_slice(), true, [2048, 1152], 0, true, true),
            (still.as_slice(), true, [2048, 1152], 0, true, true),
            (still.as_slice(), false, [2048, 1152], 2, false, false),
            (still.as_slice(), true, [2048, 1152], 0, true, true),
            // An owner that moved is the same window at a new origin: nothing written, nothing
            // laid out again.
            (moved.as_slice(), true, [2048, 1152], 0, true, false),
            (moved.as_slice(), true, [2048, 1152], 0, true, true),
            (dirty_on.as_slice(), true, [2048, 1152], 0, true, false),
            (dirty_off.as_slice(), true, [2048, 1152], 1, false, false),
            (dirty_off.as_slice(), true, [2048, 1152], 0, true, true),
            (dirty_off.as_slice(), false, [2048, 1152], 3, false, false),
            (moved.as_slice(), true, [2048, 1152], 0, true, false),
            (moved.as_slice(), true, [2048, 1152], 0, true, true),
            (moved.as_slice(), true, [2000, 1152], 2, false, false),
            (moved.as_slice(), true, [2000, 1152], 0, true, true),
        ];
        let mut kept_view = PoolView::default();
        let mut today = PoolView::default();
        for (at, (owners, same, viewport, expected_written, expected_kept, expected_gated)) in
            frames.into_iter().enumerate()
        {
            let context = Context {
                resources: &resources,
                windows: &windows,
                viewport,
                time: Some(1. + at as f32 / 60.),
            };
            let plan = Plan {
                name: "test_icon",
                kind: MapIconPool::Ping,
                alignment: 0x0A,
                owners,
            };
            assert_eq!(
                kept_view.nothing_due(&context, &plan, &|_, _| same),
                expected_gated,
                "frame {at}: nothing due"
            );
            let mut written = 0;
            let mut notes = BTreeMap::new();
            let kept = kept_view.present_kept(
                &context,
                &plan,
                Box::new(|_, _| Ok(())),
                |_, _, _, _| {
                    written += 1;
                    Ok(())
                },
                |_, _| same,
                &mut notes,
            );
            assert_eq!(kept, expected_kept, "frame {at}");
            assert_eq!(written, expected_written, "frame {at}: windows written");
            let mut today_notes = BTreeMap::new();
            let laid = today.present(&context, &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), &mut today_notes);
            assert_eq!(notes, today_notes, "frame {at}");
            let shown = if owners[2].dirty { 3 } else { 2 };
            assert_eq!(kept_view.drawn().count(), shown, "frame {at}: the windows shown");
            assert_eq!(clockless(kept_view.drawn()), clockless(&laid), "frame {at}");
            if written == shown {
                assert_eq!(exact(kept_view.drawn()), exact(&laid), "frame {at}");
            }
            // Every window kept stands where this frame's owner does, laid out again or not.
            let origins = |drawn: &[Drawn]| {
                let mut found: Vec<_> = drawn.iter().map(|drawn| (drawn.owner, drawn.origin)).collect();
                found.sort_unstable();
                found
            };
            let kept_origins = {
                let mut found: Vec<_> = kept_view.drawn().map(|drawn| (drawn.owner, drawn.origin)).collect();
                found.sort_unstable();
                found
            };
            assert_eq!(kept_origins, origins(&laid), "frame {at}: the origins");
            // A settled pass asks for no frame where today's would ([`PoolView::set_cleaning`]).
            let (kept_running, today_running) = (kept_view.take_running(), today.take_running());
            assert!(!kept_running || today_running, "frame {at}");
        }
        // A new owner takes a free window; which one it is given is no part of what the window
        // draws, so the pool whose settled frames were not walked draws it where the pool
        // presented every frame draws it.
        let newcomer = [owner(1, [100., 200.], false), owner(4, [700., 200.], false)];
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2000, 1152],
            time: Some(2.),
        };
        let plan = Plan {
            name: "test_icon",
            kind: MapIconPool::Ping,
            alignment: 0x0A,
            owners: &newcomer,
        };
        let mut written = Vec::new();
        assert!(!kept_view.present_kept(&context, &plan, Box::new(|_, _| Ok(())), |owner, _, _, _| { written.push(owner); Ok(()) }, |_, _| true, &mut BTreeMap::new()));
        assert_eq!(written, [4], "the window of the owner that stayed is kept");
        let laid = today.present(&context, &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), &mut BTreeMap::new());
        assert_eq!(clockless(kept_view.drawn()), clockless(&laid));
        assert_eq!(
            kept_view.drawn().map(|drawn| drawn.origin).collect::<Vec<_>>(),
            laid.iter().map(|drawn| drawn.origin).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_quiet_frame_moves_slots_as_a_drawn_frame_and_the_next_drawn_frame_shows_the_same_windows() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id, screen, dirty| Owner { id, screen, dirty };
        let off = [-5000., 200.];
        // The province schedule's rotation dirties off-screen owners' provinces in turn: each pass
        // admits one off screen and erases the one before, which only the pass sees.
        let dirty = |id: Option<u32>| {
            [(1, [100., 200.]), (2, [400., 300.]), (3, off), (5, off)].map(|(at, screen)| owner(at, screen, Some(at) == id))
        };
        // (owners, whether the frame is quiet for the second view)
        let frames = [
            (dirty(None), false),
            (dirty(Some(3)), true),
            (dirty(Some(5)), true),
            (dirty(Some(3)), false),
            (dirty(None), true),
            (dirty(Some(5)), true),
            (dirty(None), false),
        ];
        let mut drawn = PoolView::default();
        let mut quiet = PoolView::default();
        let present = |view: &mut PoolView, context: &Context<'_, _>, owners: &[Owner]| {
            let plan = Plan { name: "test_icon", kind: MapIconPool::Ping, alignment: 0x0A, owners };
            view.present_kept(context, &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), |_, _| true, &mut BTreeMap::new());
        };
        for (at, (owners, skipped)) in frames.iter().enumerate() {
            let context = Context { resources: &resources, windows: &windows, viewport: [2048, 1152], time: Some(1. + at as f32 / 60.) };
            present(&mut drawn, &context, owners.as_slice());
            quiet.set_quiet(*skipped);
            present(&mut quiet, &context, owners.as_slice());
            quiet.set_quiet(false);
            assert_eq!(quiet.live(), drawn.live(), "frame {at}: the slots each owner holds");
            assert!(!quiet.running_visibly(), "frame {at}: nothing shown changed");
            assert_eq!(quiet.take_running(), drawn.take_running(), "frame {at}");
            if !skipped {
                assert_eq!(clockless(quiet.drawn()), clockless(drawn.drawn()), "frame {at}: the windows shown, in order");
            }
        }
        // A window the quiet frames skipped is laid out on the next drawn frame, and a newcomer
        // takes the same slot in each.
        let newcomer = [owner(1, [100., 200.], false), owner(4, [700., 200.], false), owner(2, [400., 300.], false), owner(5, off, true)];
        let context = Context { resources: &resources, windows: &windows, viewport: [2048, 1152], time: Some(2.) };
        present(&mut drawn, &context, &newcomer);
        present(&mut quiet, &context, &newcomer);
        assert_eq!(quiet.pool.as_ref().unwrap().slot_of(&4), drawn.pool.as_ref().unwrap().slot_of(&4));
        assert_eq!(quiet.live(), drawn.live());
        assert_eq!(clockless(quiet.drawn()), clockless(drawn.drawn()), "the windows shown, in order");
        assert_eq!(quiet.drawn().map(|drawn| drawn.owner).collect::<Vec<_>>(), drawn.drawn().map(|drawn| drawn.owner).collect::<Vec<_>>());
    }

    /// A pool at a held hour under a still camera, one owner kept off screen by a province dirty
    /// in view: once a pass changes nothing it asks for no frame, and each change that writes a
    /// window asks again at the frame it comes: an owner in a province a phase will clean, a
    /// changed offer. A camera move writes none.
    #[test]
    fn a_settled_pool_asks_for_no_frame_until_something_it_shows_or_holds_changes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id, screen, dirty| Owner { id, screen, dirty };
        let still = [owner(1, [100., 200.], false), owner(2, [-5000., 200.], true)];
        let moved = [owner(1, [140., 200.], false), owner(2, [-4960., 200.], true)];
        let frame = |at: usize| Context { resources: &resources, windows: &windows, viewport: [2048, 1152], time: Some(1. + at as f32 / 60.) };
        let present = |view: &mut PoolView, at: usize, owners: &[Owner], same: bool| {
            let plan = Plan { name: "test_icon", kind: MapIconPool::Ping, alignment: 0x0A, owners };
            view.present_kept(&frame(at), &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), |_, _| same, &mut BTreeMap::new());
            view.take_running()
        };
        let mut view = PoolView::default();
        let mut at = 0;
        let mut asks = Vec::new();
        for _ in 0..4 {
            asks.push(present(&mut view, at, &still, true));
            at += 1;
        }
        assert!(asks[0], "the first frame asks");
        assert!(!asks[3], "a settled pool asks for no frame: {asks:?}");
        view.set_cleaning(true);
        assert!(present(&mut view, at, &still, true), "an owner in a province a phase will clean asks");
        view.set_cleaning(false);
        at += 1;
        assert!(!present(&mut view, at, &still, true));
        at += 1;
        // A window whose owner only moved keeps its layout at the new origin and writes nothing:
        // the frame that moved the camera draws it, and no later frame is asked for.
        assert!(!present(&mut view, at, &moved, true), "a camera move asks for no frame");
        at += 1;
        assert!(present(&mut view, at, &moved, false), "a changed offer asks");
    }

    /// A pool window's layout reads no screen point: the owner's point is only the origin it is
    /// drawn at. So a window the camera moved is its layout at another origin: over points a pan,
    /// a zoom and a tilt move it to, on and off the screen, its layout is the first point's.
    #[test]
    fn a_pool_window_s_layout_reads_no_screen_point() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let root = windows.gui_type(b"test_icon").expect("the fixture's window");
        let mut view = PoolView::default();
        let (mut first, mut compared): (Option<native_gui::Layout>, usize) = (None, 0);
        for (at, screen) in [[100., 200.], [640., 360.], [-30., 900.], [2000., 10.], [100.5, 200.25]].into_iter().enumerate() {
            let context = Context { resources: &resources, windows: &windows, viewport: [2048, 1152], time: Some(1. + at as f32 / 60.) };
            let owners = [Owner { id: 1, screen, dirty: false }];
            let plan = Plan { name: "test_icon", kind: MapIconPool::Ping, alignment: 0x0A, owners: &owners };
            view.present_kept(&context, &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), |_, _| true, &mut BTreeMap::new());
            let pool = view.pool.as_ref().expect("built");
            // A point the pool gives no window has nothing to lay out.
            let Some(instance) = pool.slot_of(&1).and_then(|slot| pool.instance(slot)) else {
                continue;
            };
            // The layout carries its tick's clock: each point here is another frame's.
            let layout = clock_aside(instance.layout(root, &resources, None).unwrap().expect("drawn"));
            compared += 1;
            let first = first.get_or_insert_with(|| layout.clone());
            assert_eq!(&layout, first, "point {at}: the layout reads no screen point");
        }
        assert!(compared >= 3, "the window must stand at several points: {compared}");
    }

    #[test]
    fn a_recorded_pool_writes_again_only_the_owners_whose_values_changed() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let owners = [
            Owner { id: 1, screen: [100., 200.], dirty: false },
            Owner { id: 2, screen: [400., 300.], dirty: false },
        ];
        let plan = Plan {
            name: "test_icon",
            kind: MapIconPool::Ping,
            alignment: 0x0A,
            owners: &owners,
        };
        // (each owner's value, forget what was written, the owners written)
        let frames: [([u8; 2], bool, &[u32]); 5] = [
            ([5, 7], false, &[1, 2]),
            ([5, 7], false, &[]),
            ([5, 8], false, &[2]),
            ([5, 8], false, &[]),
            ([5, 8], true, &[1, 2]),
        ];
        let mut view = PoolView::default();
        let mut today = PoolView::default();
        for (at, (values, forget, expected)) in frames.into_iter().enumerate() {
            let context = Context {
                resources: &resources,
                windows: &windows,
                viewport: [2048, 1152],
                time: Some(1.),
            };
            if forget {
                view.forget_writes();
            }
            let mut written = Vec::new();
            let kept = view.present_recorded(
                &context,
                &plan,
                Box::new(|_, _| Ok(())),
                |owner, _, _, _| {
                    written.push(owner);
                    Ok(())
                },
                |owner| values[*owner as usize - 1],
                false,
                &mut BTreeMap::new(),
            );
            written.sort_unstable();
            assert_eq!(written, expected, "frame {at}");
            assert_eq!(kept, expected.is_empty(), "frame {at}");
            let laid = today.present(&context, &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), &mut BTreeMap::new());
            assert_eq!(exact(view.drawn()), exact(&laid), "frame {at}");
        }
    }

    #[test]
    fn a_growing_pool_keeps_the_windows_that_stood_still() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id: u32| Owner {
            id,
            screen: [10. + (id % 40) as f32 * 40., 10. + (id / 40) as f32 * 40.],
            dirty: false,
        };
        let few: Vec<_> = (1..=2).map(owner).collect();
        // More owners than the pool's windows: requests go ungranted and the pool grows.
        let many: Vec<_> = (1..=MapIconPool::Ping.initial() as u32 + 20).map(owner).collect();
        let frames = [&few, &few, &many, &many, &many, &many];
        let mut view = PoolView::default();
        let mut today = PoolView::default();
        for (at, owners) in frames.into_iter().enumerate() {
            let context = Context {
                resources: &resources,
                windows: &windows,
                viewport: [2048, 1152],
                time: Some(1. + at as f32 / 60.),
            };
            let plan = Plan {
                name: "test_icon",
                kind: MapIconPool::Ping,
                alignment: 0x0A,
                owners,
            };
            let mut written = Vec::new();
            let mut notes = BTreeMap::new();
            view.present_kept(
                &context,
                &plan,
                Box::new(|_, _| Ok(())),
                |owner, _, _, _| {
                    written.push(owner);
                    Ok(())
                },
                |_, _| true,
                &mut notes,
            );
            if at > 0 {
                assert!(!written.contains(&1) && !written.contains(&2), "frame {at}: the first two stood still");
            }
            let mut today_notes = BTreeMap::new();
            let laid = today.present(&context, &plan, Box::new(|_, _| Ok(())), |_, _, _, _| Ok(()), &mut today_notes);
            assert_eq!(notes, today_notes, "frame {at}");
            assert_eq!(clockless(view.drawn()), clockless(&laid), "frame {at}");
            assert_eq!(view.take_running(), today.take_running(), "frame {at}");
        }
        assert!(
            view.pool.as_ref().unwrap().growths() > 0,
            "the pool grew"
        );
        assert_eq!(view.drawn().count(), MapIconPool::Ping.initial() + 20, "every owner shown once the pool grew");
    }

    #[test]
    fn a_painted_window_hands_again_the_quads_it_drew_until_its_layout_or_anchor_changes() {
        use crate::map::sprites::{Batch, Quad, WindowKey, WindowPool};
        let key = WindowKey::new(WindowPool::Base, 0, 1_u32);
        let mut painted: Painted<u32, u8> = Painted::default();
        let mut draws = 0;
        // A window's parts stand at its origin, which is all a draw at another origin moves.
        fn draw(draws: &mut u32, origin: [i32; 2]) -> impl FnOnce(&mut Batch) -> Result<(), ()> {
            move |batch: &mut Batch| {
                *draws += 1;
                for part in 0..2 {
                    batch.push(Quad {
                        texture: part,
                        rect: [
                            origin[0] as f32 + part as f32,
                            origin[1] as f32 - part as f32,
                            4.,
                            4.,
                        ],
                        uv: [0.; 4],
                        color: [1.; 4],
                        rotation: 0.,
                        layer: 0,
                        shade: 0.,
                        exact: None,
                        anchor: None,
                        present: None,
                    });
                }
                Ok::<_, ()>(())
            }
        }
        // (layout generation, origin, layer, anchor stamp, drawn anew)
        let frames = [
            (1, [0, 0], 10, 0, true),
            (1, [0, 0], 10, 0, false),
            (1, [0, 0], 30, 0, false),
            // The same layout standing somewhere else is handed again, offset to the new origin.
            (1, [4, -9], 30, 0, false),
            (1, [4, -9], 30, 0, false),
            (2, [4, -9], 30, 0, true),
            (2, [4, -9], 30, 7, true),
            (2, [4, -9], 30, 7, false),
        ];
        // The painting the window last drew, and the origin it drew at.
        let mut drew = (0, [0, 0]);
        for (at, (generation, origin, layer, stamp, anew)) in frames.into_iter().enumerate() {
            let mut batch = Batch::default();
            batch.at_layer(layer);
            let before = draws;
            painted
                .paint(key, 1, generation, origin, layer, stamp, &mut batch, draw(&mut draws, origin))
                .unwrap();
            painted.finish();
            assert_eq!(draws > before, anew, "frame {at}");
            assert_eq!(batch.quads.len(), 2, "frame {at}");
            assert!(batch.quads.iter().all(|quad| quad.layer == layer), "frame {at}: the layer is the frame's");
            // Whether it was drawn or handed again at a new origin, the window draws what a draw
            // of its layout at this origin draws.
            let mut fresh = Batch::default();
            fresh.at_layer(layer);
            draw(&mut 0, origin)(&mut fresh).unwrap();
            let rects = |quads: &[Quad]| quads.iter().map(|quad| quad.rect).collect::<Vec<_>>();
            assert_eq!(rects(&batch.quads), rects(&fresh.quads), "frame {at}: at this origin");
            let window = batch.windows().next().expect("the window the paint opened");
            if anew {
                drew = (window.stamp.painting, origin);
            } else {
                // The window names the painting it drew and how far it stands from where that
                // painting drew it, which is what the quads it hands are.
                assert_eq!(window.stamp.painting, drew.0, "frame {at}: the painting it drew");
                assert_eq!(
                    window.shift,
                    [origin[0] - drew.1[0], origin[1] - drew.1[1]],
                    "frame {at}: how far it stands from where it was drawn"
                );
            }
        }
        // A window not painted in a frame holds nothing after it.
        painted.finish();
        let mut batch = Batch::default();
        let mut drew = false;
        painted
            .paint(key, 1, 2, [4, -9], 30, 7, &mut batch, |_: &mut Batch| {
                drew = true;
                Ok::<_, ()>(())
            })
            .unwrap();
        assert!(drew);
    }
}
