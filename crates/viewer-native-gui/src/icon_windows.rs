
use crate::gui_resources::count_noted;
use crate::gui_resources::count_refused;
use crate::native_windows::NativeWindows;
#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap};

use content_reader_gui::gui::native;

use super::{
    native_gui::{
        self,
        lifecycle::{
            Admission, Instance, MapIconPool, Pool, Rewrote, SlotId, Tick, Unknown, Visible,
        },
    },
    native_resources::Context,
};

pub const GUI_DELTA_CLAMP: f32 = 0.1;

pub fn frame(last: &mut Option<f32>, time: f32) -> Tick {
    let dt = last.map_or(0.0, |last| (time - last).clamp(0.0, GUI_DELTA_CLAMP));
    *last = Some(time);
    Tick { time, dt }
}

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

pub fn rectangle_inside(point: [i32; 2], size: [i32; 2], viewport: [i32; 2]) -> bool {
    on_screen(
        point.map(|v| v as f32),
        size.map(|v| v as f32),
        viewport.map(|v| v as f32),
    )
}

pub fn admitted(origin: [f32; 2], size: [f32; 2], extent: [f32; 2]) -> bool {
    let pad = crate::anchored::PAD;
    on_screen(
        [origin[0] + pad, origin[1] + pad],
        size,
        [extent[0] + 2.0 * pad, extent[1] + 2.0 * pad],
    )
}

pub fn rectangle_admitted(point: [i32; 2], size: [i32; 2], viewport: [i32; 2]) -> bool {
    admitted(
        point.map(|v| v as f32),
        size.map(|v| v as f32),
        viewport.map(|v| v as f32),
    )
}

fn truncate_int(value: f32) -> i32 {
    if value.is_finite() && (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    }
}

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

#[derive(Debug)]
pub struct Owner<K = u32> {
    pub id: K,
    pub screen: [f32; 2],
    pub dirty: bool,
}

pub struct Plan<'a, K = u32> {
    pub name: &'a str,
    pub kind: MapIconPool,
    pub alignment: u8,
    pub owners: &'a [Owner<K>],
}

pub struct Drawn<K = u32> {
    pub owner: K,
    pub origin: [i32; 2],
    pub layout: native_gui::Layout,
}

pub fn visible_layout<K: Copy>(
    icon: &Visible<'_, K>,
    root: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Option<Drawn<K>> {
    visible_layout_regions(icon, root, name, resources, notes, false)
}

pub fn visible_layout_kept<K: Copy>(
    icon: &Visible<'_, K>,
    root: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
    stamp: u64,
) -> Option<Drawn<K>> {
    visible_layout_with(icon, name, notes, |instance| instance.layout_kept(root, resources, false, stamp))
}

fn visible_layout_regions<K: Copy>(
    icon: &Visible<'_, K>,
    root: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
    regions: bool,
) -> Option<Drawn<K>> {
    visible_layout_with(icon, name, notes, |instance| instance.layout_with_regions(root, resources, None, regions))
}

fn visible_layout_with<K: Copy>(
    icon: &Visible<'_, K>,
    name: &str,
    notes: &mut BTreeMap<String, usize>,
    layout: impl FnOnce(&Instance) -> Result<Option<native_gui::Layout>, native_gui::Refusals>,
) -> Option<Drawn<K>> {
    if let Some(why) = icon.withheld {
        *notes
            .entry(format!("{name} is not drawn: {why}"))
            .or_default() += 1;
        return None;
    }
    match layout(icon.instance) {
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

pub type PoolCount = (String, [u64; 5], Vec<(&'static str, u64)>);

static COUNTS: std::sync::Mutex<Vec<PoolCount>> = std::sync::Mutex::new(Vec::new());

#[derive(Default)]
struct Tally {
    still: bool,
    kept: bool,
    why: Option<&'static str>,
    laid: u64,
    held: u64,
    moved: u64,
}

impl Tally {
    fn because(&mut self, why: &'static str) {
        self.why.get_or_insert(why);
    }
}

fn count(name: &str, tally: &Tally) {
    if viewer_runtime::view_record::on() {
        let section = if viewer_runtime::redraws::apart_now() { "overlay worker" } else { "overlay" };
        viewer_runtime::view_record::add(section, "windows_laid", tally.laid);
        viewer_runtime::view_record::add(section, "windows_kept", tally.held);
        viewer_runtime::view_record::add(section, "windows_kept_moved", tally.moved);
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

pub fn take_counts() -> Vec<PoolCount> {
    let mut counts = COUNTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let taken = counts.iter().filter(|(_, counted, _)| counted[0] > 0).cloned().collect();
    for (_, counted, whys) in counts.iter_mut() {
        *counted = [0; 5];
        whys.clear();
    }
    taken
}

#[derive(Debug)]
pub struct PoolView<K = u32> {
    built: Option<(NativeWindows, [i32; 2])>,
    built_build: Option<String>,
    pool: Option<Pool<K>>,
    last: Option<f32>,
    active: bool,
    owners: HashMap<K, ([f32; 2], bool)>,
    settled: bool,
    unsettled: &'static str,
    kept: Option<Kept<K>>,
    unkept: Option<&'static str>,
    records: HashMap<K, (SlotId, Box<dyn std::any::Any + Send + Sync>)>,
    generations: u64,
    painted: Painted<K, Option<viewer_gpu::icon_tracks::IconAnchor>>,
    bypassed: bool,
    quiet: bool,
    cleaning: bool,
    last_drawn: HashMap<K, ([i32; 2], [f32; 2])>,
}

struct Kept<K> {
    windows: Vec<Shown<K>>,
    keep: bool,
    whole: bool,
}

struct Shown<K> {
    slot: SlotId,
    owner: K,
    position: [i32; 2],
    drawn: Option<Drawn<K>>,
    notes: BTreeMap<String, usize>,
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
            built_build: None,
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

    pub fn keep_running(&mut self) {
        self.bypassed = true;
    }

    pub fn running_visibly(&self) -> bool {
        self.active || self.pool.as_ref().is_some_and(Pool::requests_pending)
    }

    pub fn set_quiet(&mut self, quiet: bool) {
        self.quiet = quiet;
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

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

    #[expect(clippy::too_many_arguments, reason = "one presented frame's inputs, with what the update reads")]
    pub fn present_recorded<V: PartialEq + Send + Sync + 'static>(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        value: impl Fn(&K) -> V,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        self.present_recorded_with_build(context, plan, None, initialize, update, value, regions, notes)
    }

    #[expect(clippy::too_many_arguments, reason = "one presented frame's inputs, with its verified build and update key")]
    pub fn present_recorded_for_build<V: PartialEq + Send + Sync + 'static>(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        build: Option<&str>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        value: impl Fn(&K) -> V,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        self.present_recorded_with_build(context, plan, Some(build), initialize, update, value, regions, notes)
    }

    #[expect(clippy::too_many_arguments, reason = "one presented frame's inputs, with optional build verification")]
    fn present_recorded_with_build<V: PartialEq + Send + Sync + 'static>(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        build: Option<Option<&str>>,
        initialize: native_gui::lifecycle::Setup,
        mut update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        value: impl Fn(&K) -> V,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        if !self.same_with_build(context, build.flatten()) {
            self.records.clear();
        }
        let records = std::cell::RefCell::new(std::mem::take(&mut self.records));
        let kept = self.present_kept_regions_with_build(
            context,
            plan,
            build,
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

    fn same_with_build<R>(&self, context: &Context<'_, R>, build: Option<&str>) -> bool {
        self.same(context) && self.built_build.as_deref() == build
    }

    pub fn forget_writes(&mut self) {
        self.records.clear();
    }

    #[expect(clippy::too_many_arguments, reason = "one presented frame's inputs, with the caller's keep test")]
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
        self.present_kept_regions_with_build(context, plan, None, initialize, update, unchanged, regions, notes)
    }

    #[expect(clippy::too_many_arguments, reason = "one presented frame's inputs, with the caller's keep test and build")]
    fn present_kept_regions_with_build(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        build: Option<Option<&str>>,
        initialize: native_gui::lifecycle::Setup,
        update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        unchanged: impl Fn(&K, SlotId) -> bool,
        regions: bool,
        notes: &mut BTreeMap<String, usize>,
    ) -> bool {
        if self.same_with_build(context, build.flatten()) && self.nothing_due(context, plan, &unchanged) {
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
            self.bypassed = false;
            viewer_runtime::redraws::note("pool.nothing_due", 1);
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
            context, plan, build, initialize, update, &unchanged, regions, notes, held, &mut shown, &mut tally,
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

    fn nothing_due(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        unchanged: &impl Fn(&K, SlotId) -> bool,
    ) -> bool {
        if let Some(why) = self.why_still(context) {
            viewer_runtime::redraws::note(why, 1);
            return false;
        }
        if !self.owners_stand(plan.owners) {
            viewer_runtime::redraws::note("pool.due.owners", 1);
            return false;
        }
        let kept = self.kept.as_ref().expect("stands_still found windows kept");
        let due = kept
            .windows
            .iter()
            .all(|window| unchanged(&window.owner, window.slot));
        if !due {
            viewer_runtime::redraws::note("pool.due.offers", 1);
        }
        due
    }

    pub fn unkept(&self) -> Option<&'static str> {
        self.unkept
    }

    pub fn kept_windows(&self) -> impl Iterator<Item = (K, SlotId)> + '_ {
        self.kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .map(|window| (window.owner, window.slot))
    }

    pub fn stands_still(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
    ) -> bool {
        self.why_still(context).is_none()
    }

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

    pub fn owners_stand(&self, owners: &[Owner<K>]) -> bool {
        self.owners.len() == owners.len()
            && owners.iter().all(|owner| {
                self.owners.get(&owner.id).is_some_and(|&(screen, dirty)| {
                    dirty == owner.dirty && screen.map(f32::to_bits) == owner.screen.map(f32::to_bits)
                })
            })
    }

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

    pub fn drawn(&self) -> impl Iterator<Item = &Drawn<K>> + '_ {
        self.kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .filter_map(|window| window.drawn.as_ref())
    }

    pub fn drawn_generations(&self) -> impl Iterator<Item = (&Drawn<K>, u64)> + '_ {
        self.kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .filter_map(|window| window.drawn.as_ref().map(|drawn| (drawn, window.generation)))
    }

    #[expect(clippy::type_complexity, reason = "the windows shown and the store their quads are painted from")]
    pub fn painted(
        &mut self,
    ) -> (Vec<(&Drawn<K>, u64)>, &mut Painted<K, Option<viewer_gpu::icon_tracks::IconAnchor>>) {
        let drawn = self
            .kept
            .iter()
            .flat_map(|kept| &kept.windows)
            .filter_map(|window| window.drawn.as_ref().map(|drawn| (drawn, window.generation)))
            .collect();
        (drawn, &mut self.painted)
    }

    #[expect(clippy::too_many_arguments, reason = "one presented frame's inputs, the windows kept and what it shows")]
    fn present_windows(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        build: Option<Option<&str>>,
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
        let requested_build = build.flatten();
        if !self.same_with_build(context, requested_build) {
            tally.because(if self.built.is_none() {
                "no frame presented yet"
            } else {
                "other windows or viewport"
            });
            if context.time.is_none() {
                return Ok(false);
            }
            let (generations, cleaning) = (self.generations, self.cleaning);
            let painted = std::mem::take(&mut self.painted);
            *self = Self::default();
            (self.generations, self.cleaning) = (generations, cleaning);
            self.painted = painted;
            let pool = if build.is_some() || plan.kind == MapIconPool::AdjacencyRule {
                Pool::new_for_build(
                    root,
                    context.viewport,
                    plan.kind,
                    requested_build,
                    initialize,
                    context.resources,
                )?
            } else {
                Pool::new(
                    root,
                    context.viewport,
                    plan.kind,
                    initialize,
                    context.resources,
                )?
            };
            self.pool = Some(pool);
            self.built = Some((context.windows.clone(), context.viewport));
            self.built_build = requested_build.map(str::to_owned);
            held = None;
        }
        let pool = self.pool.as_mut().expect("the matching view was built");
        let owners = &mut self.owners;
        let manager_profile = viewer_runtime::profiling::span("dc_pool_manager");
        owners.clear();
        owners.extend(
            plan.owners
                .iter()
                .map(|owner| (owner.id, (owner.screen, owner.dirty))),
        );
        let Some(time) = context.time else {
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
                None if self.quiet => {
                    if on_screen(icon.instance.position()) {
                        viewer_runtime::redraws::note("quiet.shown_late", 1);
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
        if viewer_runtime::view_record::on() {
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
                    Some(&(was, was_screen)) => {
                        let own = [
                            (position[0] - was[0]) as f32 - (screen[0] - was_screen[0]),
                            (position[1] - was[1]) as f32 - (screen[1] - was_screen[1]),
                        ];
                        let distance = own[0].hypot(own[1]);
                        moved += u64::from(position != was);
                        largest = largest.max(distance);
                        jumps += u64::from(!icon.instance.active() && distance > 1.5);
                    }
                    None => popped += u64::from(shows_animation && !icon.instance.active()),
                }
                drawn.insert(*icon.owner, (position, screen));
            }
            self.last_drawn = drawn;
            viewer_runtime::view_record::add("overlay", "windows_moved", moved);
            viewer_runtime::view_record::add("overlay", "jumps", jumps);
            viewer_runtime::view_record::add("overlay", "window_jumps", jumps);
            viewer_runtime::view_record::add("overlay", "popped", popped);
            viewer_runtime::view_record::add("overlay", "largest_window_move_px", largest.round() as u64);
        }
        if touched.is_empty() && !pool.requests_pending() && !self.cleaning {
            self.bypassed = false;
        }
        if self.quiet {
            let moving = touched.iter().filter(|&&slot| pool.instance(slot).is_some_and(Instance::active)).count();
            viewer_runtime::redraws::note("quiet.moving", moving as u64);
        }
        tally.still = !on_screen_laid;
        tally.kept = tally.laid == 0;
        Ok(true)
    }

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
                if context.time.is_none() {
                    return Ok(());
                }
                *self = Self::default();
                let pool = if plan.kind == MapIconPool::AdjacencyRule {
                    Pool::new_for_build(
                        root,
                        context.viewport,
                        plan.kind,
                        None,
                        initialize,
                        context.resources,
                    )?
                } else {
                    Pool::new(
                        root,
                        context.viewport,
                        plan.kind,
                        initialize,
                        context.resources,
                    )?
                };
                self.pool = Some(pool);
                self.built = Some((context.windows.clone(), context.viewport));
            }
            let pool = self.pool.as_mut().expect("the matching view was built");
            let owners = &mut self.owners;
            let manager_profile = viewer_runtime::profiling::span("dc_pool_manager");
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

    pub fn any(&self) -> bool {
        self.named().iter().any(|(running, _)| *running)
    }

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

pub struct Painted<K, S> {
    held: HashMap<K, Paint<S>>,
    next: HashMap<K, Paint<S>>,
    counted: [u64; 3],
    passed: u64,
}

#[derive(Debug)]
struct Paint<S> {
    generation: u64,
    origin: [i32; 2],
    stamp: S,
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
    #[expect(
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
                    crate::anchored::shift(quad, step);
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

    pub fn finish(&mut self) {
        std::mem::swap(&mut self.held, &mut self.next);
        self.next.clear();
        let [painted, kept, standing] = std::mem::take(&mut self.counted);
        viewer_runtime::redraws::note("windows.painted", painted);
        viewer_runtime::redraws::note("windows.painted_kept", kept);
        viewer_runtime::redraws::note("windows.painted_elsewhere", standing);
        let in_batch = std::mem::take(&mut self.passed);
        if in_batch > 0 {
            viewer_runtime::view_record::add("overlay", "windows_in_batch", in_batch);
        }
    }

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
    use crate::test_support::gui::{fixture, windows};
    use native_gui::Resources;

    const GUI: &[u8] = br#"guiTypes = { containerWindowType = { name = test_icon
        size = { width = 100 height = 100 }
        iconType = { name = icon spriteType = GFX_even }
    } }"#;

    #[test]
    fn combat_handles_keep_their_high_bits_in_pool_ownership() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        use viewer_gpu::province_schedule::Schedule;
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
        assert_eq!(truncate_int(-2.7), -2);
        assert_eq!(truncate_int(f32::NAN), i32::MIN);
        assert_eq!(truncate_int(3.0e9), i32::MIN);
    }

    fn clockless<'a>(
        drawn: impl IntoIterator<Item = &'a Drawn>,
    ) -> Vec<(u32, [i32; 2], native_gui::Layout)> {
        drawn
            .into_iter()
            .map(|drawn| (drawn.owner, drawn.origin, clock_aside(drawn.layout.clone())))
            .collect()
    }

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
        content_reader_script::test_game::enter();
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id, screen, dirty| Owner { id, screen, dirty };
        let off = [-5000., 200.];
        let still = vec![
            owner(1, [100., 200.], false),
            owner(2, [400., 300.], false),
            owner(3, off, false),
        ];
        let moved = vec![owner(1, [100., 200.], false), owner(2, [410., 300.], false), owner(3, off, false)];
        let dirty_on = vec![owner(1, [100., 200.], true), owner(2, [410., 300.], false), owner(3, off, false)];
        let dirty_off = vec![owner(1, [100., 200.], false), owner(2, [410., 300.], false), owner(3, off, true)];
        let frames: Vec<(&[Owner], bool, [i32; 2], usize, bool, bool)> = vec![
            (still.as_slice(), true, [2048, 1152], 2, false, false),
            (still.as_slice(), true, [2048, 1152], 0, true, true),
            (still.as_slice(), true, [2048, 1152], 0, true, true),
            (still.as_slice(), false, [2048, 1152], 2, false, false),
            (still.as_slice(), true, [2048, 1152], 0, true, true),
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
            let (kept_running, today_running) = (kept_view.take_running(), today.take_running());
            assert!(!kept_running || today_running, "frame {at}");
        }
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
        content_reader_script::test_game::enter();
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id, screen, dirty| Owner { id, screen, dirty };
        let off = [-5000., 200.];
        let dirty = |id: Option<u32>| {
            [(1, [100., 200.]), (2, [400., 300.]), (3, off), (5, off)].map(|(at, screen)| owner(at, screen, Some(at) == id))
        };
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
        let newcomer = [owner(1, [100., 200.], false), owner(4, [700., 200.], false), owner(2, [400., 300.], false), owner(5, off, true)];
        let context = Context { resources: &resources, windows: &windows, viewport: [2048, 1152], time: Some(2.) };
        present(&mut drawn, &context, &newcomer);
        present(&mut quiet, &context, &newcomer);
        assert_eq!(quiet.pool.as_ref().unwrap().slot_of(&4), drawn.pool.as_ref().unwrap().slot_of(&4));
        assert_eq!(quiet.live(), drawn.live());
        assert_eq!(clockless(quiet.drawn()), clockless(drawn.drawn()), "the windows shown, in order");
        assert_eq!(quiet.drawn().map(|drawn| drawn.owner).collect::<Vec<_>>(), drawn.drawn().map(|drawn| drawn.owner).collect::<Vec<_>>());
    }

    #[test]
    fn a_settled_pool_asks_for_no_frame_until_something_it_shows_or_holds_changes() {
        content_reader_script::test_game::enter();
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
        assert!(!present(&mut view, at, &moved, true), "a camera move asks for no frame");
        at += 1;
        assert!(present(&mut view, at, &moved, false), "a changed offer asks");
    }

    #[test]
    fn a_pool_window_s_layout_reads_no_screen_point() {
        content_reader_script::test_game::enter();
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
            let Some(instance) = pool.slot_of(&1).and_then(|slot| pool.instance(slot)) else {
                continue;
            };
            let layout = clock_aside(instance.layout(root, &resources, None).unwrap().expect("drawn"));
            compared += 1;
            let first = first.get_or_insert_with(|| layout.clone());
            assert_eq!(&layout, first, "point {at}: the layout reads no screen point");
        }
        assert!(compared >= 3, "the window must stand at several points: {compared}");
    }

    #[test]
    fn a_recorded_pool_writes_again_only_the_owners_whose_values_changed() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        let resources = fixture();
        let windows = windows(GUI);
        let owner = |id: u32| Owner {
            id,
            screen: [10. + (id % 40) as f32 * 40., 10. + (id / 40) as f32 * 40.],
            dirty: false,
        };
        let few: Vec<_> = (1..=2).map(owner).collect();
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
        use crate::sprites::{Batch, Quad, WindowKey, WindowPool};
        let key = WindowKey::new(WindowPool::Base, 0, 1_u32);
        let mut painted: Painted<u32, u8> = Painted::default();
        let mut draws = 0;
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
                        rotation: None,
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
        let frames = [
            (1, [0, 0], 10, 0, true),
            (1, [0, 0], 10, 0, false),
            (1, [0, 0], 30, 0, false),
            (1, [4, -9], 30, 0, false),
            (1, [4, -9], 30, 0, false),
            (2, [4, -9], 30, 0, true),
            (2, [4, -9], 30, 7, true),
            (2, [4, -9], 30, 7, false),
        ];
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
            let mut fresh = Batch::default();
            fresh.at_layer(layer);
            draw(&mut 0, origin)(&mut fresh).unwrap();
            let rects = |quads: &[Quad]| quads.iter().map(|quad| quad.rect).collect::<Vec<_>>();
            assert_eq!(rects(&batch.quads), rects(&fresh.quads), "frame {at}: at this origin");
            let window = batch.windows().next().expect("the window the paint opened");
            if anew {
                drew = (window.stamp.painting, origin);
            } else {
                assert_eq!(window.stamp.painting, drew.0, "frame {at}: the painting it drew");
                assert_eq!(
                    window.shift,
                    [origin[0] - drew.1[0], origin[1] - drew.1[1]],
                    "frame {at}: how far it stands from where it was drawn"
                );
            }
        }
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
