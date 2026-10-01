//! Persistent map-icon pools, advanced once per presented frame (CMapIconManagerImpl::update@140B2F270).

use std::collections::{BTreeMap, HashMap};

use installed_content::gui::native;

use super::{
    counters::{NativeWindows, gui_windows, naval_windows},
    native_gui::{
        self,
        lifecycle::{Admission, Instance, MapIconPool, Pool, Rewrote, SlotId, Unknown},
    },
    native_resources::Context,
};

/// One owner request and its projected position, with the province's dirty-bit bypass.
pub struct Owner {
    pub id: u32,
    pub screen: [f32; 2],
    pub dirty: bool,
}

/// The native template, type pool and alignment of one consumer family.
pub struct Plan<'a> {
    pub name: &'a str,
    pub kind: MapIconPool,
    pub alignment: u8,
    pub owners: &'a [Owner],
}

/// One visible owner's layout, before the consumer assigns its draw layer.
pub struct Drawn {
    pub owner: u32,
    pub origin: [i32; 2],
    pub layout: native_gui::Layout,
}

/// Pool membership, window history and presentation clock for one native icon type.
#[derive(Debug, Default)]
pub struct PoolView {
    built: Option<(NativeWindows, [i32; 2])>,
    pool: Option<Pool<u32>>,
    last: Option<f32>,
    active: bool,
}

impl PoolView {
    pub fn same<R>(&self, context: &Context<'_, R>) -> bool {
        self.built.as_ref().is_some_and(|(windows, viewport)| {
            windows.same(context.windows) && *viewport == context.viewport
        })
    }

    pub fn take_running(&mut self) -> bool {
        std::mem::take(&mut self.active)
    }

    pub fn present(
        &mut self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        plan: &Plan<'_>,
        initialize: native_gui::lifecycle::Setup,
        mut update: impl FnMut(u32, SlotId, &mut Instance, &native::Element) -> Result<(), String>,
        notes: &mut BTreeMap<String, usize>,
    ) -> Vec<Drawn> {
        let mut drawn = Vec::new();
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
            let owners: HashMap<_, _> = plan.owners.iter().map(|owner| (owner.id, owner)).collect();
            let (updated, tick) = if let Some(time) = context.time {
                self.active = false;
                let size = native_gui::container_frame(root, Some(context.viewport))?.size;
                for owner in plan.owners {
                    if pool.request(owner.id).is_none() {
                        *notes
                            .entry(format!(
                                "{} is not drawn: {}",
                                plan.name,
                                naval_windows::POOL_EXHAUSTED
                            ))
                            .or_default() += 1;
                    }
                }
                let pass = pool.update(root, |owner, _| {
                    owners.get(owner).map_or(Admission::Empty, |owner| {
                        let point = naval_windows::point(owner.screen, size, plan.alignment);
                        let (admission, dirty) =
                            naval_windows::admission(owner.dirty, point, |point| {
                                naval_windows::rectangle_inside(point, size, context.viewport)
                            });
                        self.active |= dirty;
                        admission
                    })
                })?;
                let tick = naval_windows::frame(&mut self.last, time);
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
            gui_windows::count_noted(notes, plan.name, &pool.take_diagnostics());
            for icon in pool.visible() {
                if !owners.contains_key(icon.owner) {
                    continue;
                }
                if let Some(why) = icon.withheld {
                    *notes
                        .entry(format!("{} is not drawn: {why}", plan.name))
                        .or_default() += 1;
                    continue;
                }
                match icon.instance.layout(root, context.resources, None) {
                    Ok(Some(layout)) => {
                        gui_windows::count_noted(notes, plan.name, &layout.diagnostics);
                        drawn.push(Drawn {
                            owner: *icon.owner,
                            origin: icon.instance.position(),
                            layout,
                        });
                    }
                    Ok(None) => {}
                    Err(why) => gui_windows::count_refused(notes, plan.name, &why),
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

/// All native icon views use one lock when checking whether another frame is needed.
#[derive(Debug, Default)]
pub struct Views {
    pub naval: naval_windows::Windows,
    pub labels: super::labels::LabelView,
    pub pings: PoolView,
    pub raids: PoolView,
}

impl Views {
    pub fn take_running(&mut self) -> bool {
        let naval = self.naval.take_active();
        let labels = self.labels.take_running();
        let pings = self.pings.take_running();
        let raids = self.raids.take_running();
        naval || labels || pings || raids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use native_gui::{
        Resources,
        tests::{fixture, windows},
    };

    const GUI: &[u8] = br#"guiTypes = { containerWindowType = { name = test_icon
        size = { width = 100 height = 100 }
        iconType = { name = icon spriteType = GFX_even }
    } }"#;

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
}
