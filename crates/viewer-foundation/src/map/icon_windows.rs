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
    if let Some(why) = icon.withheld {
        *notes
            .entry(format!("{name} is not drawn: {why}"))
            .or_default() += 1;
        return None;
    }
    match icon.instance.layout(root, resources, None) {
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

/// Pool membership, window history and presentation clock for one native icon type.
#[derive(Debug)]
pub struct PoolView<K = u32> {
    built: Option<(NativeWindows, [i32; 2])>,
    pool: Option<Pool<K>>,
    last: Option<f32>,
    active: bool,
    /// Current request geometry; the map retains its allocation across camera frames.
    owners: HashMap<K, ([f32; 2], bool)>,
    /// A sprite each live window fixed when its owner acquired it, by owner,
    /// with the slot that acquisition took; the caller keeps it while that
    /// window stays live ([`Self::live`]).
    pub acquired_sprites: HashMap<K, (SlotId, Option<String>)>,
}

impl<K> Default for PoolView<K> {
    fn default() -> Self {
        Self {
            built: None,
            pool: None,
            last: None,
            active: false,
            owners: HashMap::new(),
            acquired_sprites: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq> PoolView<K> {
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
        std::mem::take(&mut self.active)
    }

    /// Owner requests may change next frame even while the window itself is settled.
    pub fn keep_running(&mut self) {
        self.active = true;
    }

    pub fn retry(&self) -> Option<Retry> {
        self.pool.as_ref().and_then(Pool::retry)
    }

    pub fn present(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_, K>,
        initialize: native_gui::lifecycle::Setup,
        mut update: impl FnMut(K, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        notes: &mut BTreeMap<String, usize>,
    ) -> Vec<Drawn<K>> {
        let mut drawn = Vec::with_capacity(plan.owners.len());
        let run = || -> Result<(), String> {
            let root = context
                .windows
                .gui_type(plan.name.as_bytes())
                .ok_or("no GUI template is registered")?;
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
            owners.clear();
            owners.extend(
                plan.owners
                    .iter()
                    .map(|owner| (owner.id, (owner.screen, owner.dirty))),
            );
            let (updated, tick) = if let Some(time) = context.time {
                self.active = false;
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
                            self.active |= dirty;
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
            count_noted(notes, plan.name, &pool.take_diagnostics());
            for icon in pool.visible() {
                if !owners.contains_key(icon.owner) {
                    continue;
                }
                if let Some(layout) =
                    visible_layout(&icon, root, plan.name, context.resources, notes)
                {
                    drawn.push(layout);
                }
            }
            self.active |= pool.active();
            Ok(())
        };
        if let Err(why) = run()
            && !plan.owners.is_empty()
        {
            *notes
                .entry(format!("{} is not drawn: {why}", plan.name))
                .or_default() += plan.owners.len();
        }
        drawn
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
}
