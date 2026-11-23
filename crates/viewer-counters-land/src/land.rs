use viewer_counter_kit::{AtSeaPlate, ColumnPlacement, JoinRules, Movement, Movements};

use content_reader_gui::gui::native;
use crate::hovers;
use crate::gun_layers::LayeredGuns;
use crate::gun_members::{Gun, GunFields, GunGround, TodayGunJoins, gun_member, gun_name};
use crate::members::{DivisionFields, LandStacks, Member, TableJoins, division_member};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::{
    AbilityGrid, COARSE_PLATE_FRAME, Child, CoarseCounter, CounterFrame, CounterInputs,
    CounterPointer, CounterReport, Countries, GUN_WINDOW, Geography, GunCounter, LAND_WINDOW,
    NAVAL_KIND, PlateBinding, STACK_WINDOW, UNKNOWN_COUNT, counter_groups, counter_plate_color,
    draw_bar, draw_child, draw_coarse_counter, draw_flag, draw_pointed_plate, draw_text,
    gui_windows, is_land, rank_children, type_tint, zoom_lift,
};
use viewer_counter_kit::counter_groups::{Extents, Level};
use viewer_counter_kit::flotilla::Flotilla;
use viewer_counter_kit::layouts::window;
use viewer_counters_naval::naval::{NavalContext, NavalUnit, add_task_forces, naval_can_join};
use viewer_foundation::templates;
use viewer_foundation::mode_visibility::{UnitKinds, Visibility};
use viewer_foundation::perspective::{SharingUnknown, Sight, Vision};
use viewer_foundation::tables::rows;
use viewer_foundation::unit_rules::STANDSTILL;
use viewer_gpu::camera::Camera;
use viewer_gpu::icon_tracks::{IconAnchor, IconFamily};
use viewer_gpu::province_admission::Admission;
use viewer_native_gui::counter_hovers::land::CounterHover;
use viewer_native_gui::icon_windows::admitted;
use viewer_native_gui::native_windows::NAVY_WINDOW;
use viewer_native_gui::sprites::{Batch, SpriteSheet, layer};
use viewer_unit_reads::{
    counter_action, counter_inputs, counter_move, counter_order, counter_status,
};
use viewer_unit_reads::counter_inputs::NativeInputs;
use viewer_unit_reads::task_force_visual::TaskForceMotions;
use replay_data::{rows::ActiveAbilityRow, rows::RailwayGunCounterInputRow};
use viewer_native_gui::gui_resources::count_noted;
use viewer_native_gui::gui_resources::refused_cause;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::native_gui::{self, lifecycle::Instance};
use viewer_unit_reads::railway_gun_counter as guns;
use viewer_native_gui::sprites::DISABLED;
use viewer_native_gui::sprites::hovered_layer;
use viewer_foundation::templates::counter_custom_icon;
#[cfg(test)]
use viewer_native_gui::test_support::gui;
#[cfg(test)]
use viewer_foundation::test_support::test_frame;
#[cfg(test)]
use viewer_foundation::test_support::test_put;
use dashboard_text::table_access::TokenKeys;
use dashboard_text::table_access::TokenNames;
use viewer_runtime::unit_handle::Unit;

fn column_holder(
    root: &native::Element,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
) -> Result<(native_gui::Holder, Vec<native_gui::Diagnostic>), String> {
    let mut pending = vec![root];
    while let Some(element) = pending.pop() {
        if let Some(container) = element.container()
            && (container.show_animation != native::Animation::None
                || container.hide_animation != native::Animation::None
                || container.fade != native::Animation::None
                || container.peek)
        {
            return Err(format!(
                "{} has column movement or fading that the land batch cannot retain",
                element.path
            ));
        }
        pending.extend(element.nested());
    }
    let grid = root
        .find(native::Kind::Grid, "units_box")
        .ok_or_else(|| format!("{STACK_WINDOW} has no units_box"))?;
    let mut instance = Instance::construct(root, viewport, resources)?;
    instance.gui_show(root, root)?;
    let layout = instance
        .layout(root, resources, None)
        .map_err(|refusals| {
            refusals
                .iter()
                .map(|refusal| refused_cause(STACK_WINDOW, refusal))
                .collect::<Vec<_>>()
                .join("; ")
        })?
        .ok_or_else(|| format!("{STACK_WINDOW} does not show units_box"))?;
    if !layout.parts.is_empty() {
        return Err(format!(
            "{STACK_WINDOW} has drawable parts that the land batch cannot compose"
        ));
    }
    let holder = layout
        .holders
        .into_iter()
        .find(|holder| holder.path == grid.path)
        .ok_or_else(|| format!("{} is not shown by {STACK_WINDOW}", grid.path))?;
    if holder.scale != 1.0 || holder.clip.is_some() || holder.opacity != 1.0 {
        return Err(format!(
            "{} has column scale, clipping or opacity that the land batch cannot compose",
            grid.path
        ));
    }
    Ok((holder, layout.diagnostics))
}

fn refuse_column(
    plates: impl Iterator<Item = Plate>,
    flotillas: &[Flotilla],
    report: &mut CounterReport,
    why: &str,
) {
    for plate in plates {
        match plate {
            Plate::Land(_) => {
                *report
                    .land_counter_gaps
                    .entry(why.to_owned().into())
                    .or_default() += 1
            }
            Plate::Naval(member) => {
                *report
                    .navy_counter_gaps
                    .entry(why.to_owned().into())
                    .or_default() += 1;
                report
                    .unplaced_task_forces
                    .entry("the unit counter column cannot be laid out")
                    .or_default()
                    .insert(flotillas[member].source);
            }
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct StackStyle {
    pub organisation: f32,
    pub strength: f32,
    pub glyphs: counter_status::Glyphs,
    pub count: String,
}

pub struct PreparedLand {
    pub stacks: Vec<Stack>,
    pub styles: Vec<StackStyle>,
    pub sprites: Vec<Option<hovers::Sprite>>,
    pub plans: Vec<Option<(Arc<ColumnProgram>, usize)>>,
    pub flotillas: Vec<Flotilla>,
    pub positions: Vec<counter_move::Position>,
    pub units: Vec<Unit>,
    pub layout: counter_move::DrawnLayout,
    pub icons: Vec<counter_groups::Icon>,
    pub source_kinds: Vec<u32>,
    pub groups: Vec<counter_groups::Group>,
    pub items: Vec<Vec<Plate>>,
    pub hovered: Vec<bool>,
    pub levels: Vec<Level>,
    pub extents: Extents,
    pub countries: Arc<Countries>,
    pub templates: Arc<TemplateIcons>,
    pub vision: Arc<Vision>,
    pub order: Vec<usize>,
    pub pointer: Option<CounterPointer>,
    pub exits: Vec<super::CounterExit>,
    pub time: f32,
    pub exits_running: std::cell::RefCell<Vec<Unit>>,
    pub pointer_running: std::cell::Cell<bool>,
    pub pointer_refused: std::cell::RefCell<Vec<String>>,
    pub pointer_drawn: std::cell::Cell<bool>,
}

impl PreparedLand {
    pub fn pointed(&self, member: usize) -> Option<(Option<bool>, f32, Unit)> {
        let stack = &self.stacks[member];
        let kind = if stack.gun.is_some() {
            viewer_native_gui::counter_hovers::unit_hits::Kind::RailwayGun
        } else {
            viewer_native_gui::counter_hovers::unit_hits::Kind::Division
        };
        let ours = |unit: Unit| stack.divisions.iter().any(|&id| kind.unit(id) == unit);
        self.pointer
            .filter(|pointer| ours(pointer.unit))
            .map(|pointer| (Some(pointer.down), pointer.since, pointer.unit))
            .or_else(|| {
                self.exits
                    .iter()
                    .find(|exit| ours(exit.unit))
                    .map(|exit| (None, exit.since, exit.unit))
            })
    }

    pub fn keep_drawn(&mut self, old: &mut PreparedLand) {
        if !(Arc::ptr_eq(&self.countries, &old.countries)
            && Arc::ptr_eq(&self.templates, &old.templates)
            && Arc::ptr_eq(&self.vision, &old.vision))
        {
            return;
        }
        let held: std::collections::HashMap<(*const ColumnProgram, usize), usize> = old
            .plans
            .iter()
            .enumerate()
            .filter_map(|(member, plan)| plan.as_ref().map(|(program, at)| ((Arc::as_ptr(program), *at), member)))
            .collect();
        for (member, plan) in self.plans.iter().enumerate() {
            if let Some((program, at)) = plan
                && let Some(&from) = held.get(&(Arc::as_ptr(program), *at))
            {
                self.sprites[member] = old.sprites[from].take();
                viewer_runtime::redraws::note("plates.carried", u64::from(self.sprites[member].is_some()));
            }
        }
    }

    pub fn keep_drawn_by_value(&mut self, old: &mut PreparedLand) {
        if !(self.countries.same_flags(&old.countries)
            && Arc::ptr_eq(&self.templates, &old.templates)
            && self.vision.ours() == old.vision.ours())
        {
            return;
        }
        let held: std::collections::HashMap<&[u64], usize> = old
            .stacks
            .iter()
            .enumerate()
            .filter(|(member, _)| old.sprites[*member].is_some())
            .map(|(member, stack)| (&*stack.divisions, member))
            .collect();
        let mut carried = Vec::new();
        for (member, stack) in self.stacks.iter().enumerate() {
            if self.sprites[member].is_none()
                && let Some(&from) = held.get(&*stack.divisions)
                && old.stacks[from] == *stack
                && old.styles[from] == self.styles[member]
            {
                carried.push((member, from));
            }
        }
        for (member, from) in carried {
            self.sprites[member] = old.sprites[from].take();
            viewer_runtime::redraws::note("plates.carried", 1);
        }
    }

    pub fn draw(
        &mut self,
        inputs: &CounterInputs<'_>,
        batch: &mut Batch,
        report: &mut CounterReport,
        missing_owners: &mut Vec<u32>,
        at_sea: &mut Vec<AtSeaPlate>,
    ) -> io::Result<Vec<Arc<ColumnPlacement>>> {
        {
            let mut history = inputs
                .movement_history
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let settled = history.clock_unavailable();
            let hour = inputs.hour;
            let width = inputs.content.map.x();
            let world = inputs.route_world();
            let mut places: Vec<viewer_gpu::icon_tracks::Place> = Vec::new();
            for ((icon, position), &unit) in self.icons.iter_mut().zip(&self.positions).zip(&self.units) {
                let drawn = history.sample_for(&self.layout, unit, position);
                let placed = drawn.map_or_else(|| position.at(), |drawn| drawn.motion.position());
                icon.anchor = placed;
                places.clear();
                for &(at, province) in inputs.moved.of(unit) {
                    if at >= hour {
                        continue;
                    }
                    if let Some(there) = place_in(inputs, unit, province, position, world.as_ref()) {
                        places.push(viewer_gpu::icon_tracks::Place { hour: at, at: there });
                    }
                }
                places.push(viewer_gpu::icon_tracks::Place { hour, at: placed });
                icon.track = history.track(&self.layout, unit, &places, width).ok();
                if settled && position.motion.is_some() {
                    *report
                        .land_counter_gaps
                        .entry("unit movement: Clock speed is unavailable".into())
                        .or_default() += 1;
                }
            }
        }
        counter_groups::update_anchors(&mut self.groups, &self.icons, &self.levels, &self.extents);
        report.groups += self.groups.len();
        let columns = match &inputs.assets.layouts.columns {
            Ok(columns) => columns,
            Err(why) => {
                refuse_column(
                    self.items.iter().flatten().copied(),
                    &self.flotillas,
                    report,
                    why,
                );
                return Ok(Vec::new());
            }
        };
        let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
        let flags = gui_windows::window_flags(inputs, Some(&self.countries));
        let resources = gui_windows::gui_assets(inputs, flags.as_ref());
        let frame = window(&inputs.windows.guis, STACK_WINDOW)
            .map_err(|why| why.to_string())
            .and_then(|root| {
                let grid = root
                    .find(native::Kind::Grid, "units_box")
                    .ok_or_else(|| format!("{STACK_WINDOW} has no units_box"))?;
                let (holder, diagnostics) = column_holder(root, viewport, &resources)?;
                Ok((grid, holder, diagnostics))
            });
        let (grid, holder, diagnostics) = match frame {
            Ok(frame) => frame,
            Err(why) => {
                refuse_column(
                    self.items.iter().flatten().copied(),
                    &self.flotillas,
                    report,
                    &why,
                );
                return Ok(Vec::new());
            }
        };
        for plate in self.items.iter().flatten() {
            let gaps = match plate {
                Plate::Land(_) => &mut report.land_counter_gaps,
                Plate::Naval(_) => &mut report.navy_counter_gaps,
            };
            count_noted(gaps, STACK_WINDOW, &diagnostics);
        }
        let view_projection = inputs.camera.view_projection(inputs.viewport);
        let depth = |anchor: [f32; 3]| {
            let offset = inputs.camera.wrap_offset(anchor, inputs.viewport);
            Camera::map_icon_depth(&view_projection, [anchor[0] + offset, anchor[1], anchor[2]])
        };
        self.order.clear();
        self.order.extend(0..self.groups.len());
        let groups = &self.groups;
        let hovered = &self.hovered;
        self.order.sort_by(|a, b| {
            hovered[*a]
                .cmp(&hovered[*b])
                .then_with(|| depth(groups[*b].anchor).total_cmp(&depth(groups[*a].anchor)))
        });
        let mut ordinal = 0;
        let navy_size = inputs.navy_size();
        let mut placed_columns = Vec::with_capacity(self.groups.len());
        let roots = [LAND_WINDOW, GUN_WINDOW, NAVY_WINDOW]
            .map(|name| window(&inputs.windows.guis, name).map_err(|why| why.to_string()));
        let styles = [false, true].map(|gun| PlateStyle::read(inputs, gun));
        let mut plates = Vec::new();
        let mut sizes = Vec::new();
        let mut items = Vec::new();
        for position in 0..self.order.len() {
            let index = self.order[position];
            let world = self.groups[index].anchor;
            let Some(screen) = inputs.screen(world) else {
                continue;
            };
            let height = inputs.icon_height(world);
            let lift = zoom_lift(columns, inputs.content.defines.units_cutoff, height);
            let anchor = [screen[0], screen[1] + lift];
            let level = self.levels[self.groups[index].members[0]];
            let priority = inputs
                .content
                .defines
                .level_priority(level, inputs.visibility.priorities);
            let moving = self.groups[index].track.clone().map(|track| batch.add_track(track));
            let icon = inputs
                .icon_anchor(
                    IconFamily::Columns,
                    world,
                    inputs.content.defines.icons_cutoff,
                )
                .map(|anchor| anchor.moving(moving));
            plates.clear();
            sizes.clear();
            items.clear();
            let collected = self.items[index].iter().try_for_each(|plate| {
                let size = match plate {
                    Plate::Land(member) => plate_size(inputs, &self.stacks[*member]),
                    Plate::Naval(_) => navy_size
                        .clone()
                        .map_err(|why| format!("{NAVY_WINDOW}: {why}")),
                }?;
                plates.push((*plate, size));
                sizes.push(size);
                Ok::<_, String>(())
            });
            if let Err(why) = collected {
                refuse_column(
                    self.items[index].iter().copied(),
                    &self.flotillas,
                    report,
                    &why,
                );
                continue;
            }
            let mut below = 0i32;
            let collected = plates.iter().try_for_each(|(plate, size)| {
                let root = match plate {
                    Plate::Land(member) if self.stacks[*member].gun.is_some() => &roots[1],
                    Plate::Land(_) => &roots[0],
                    Plate::Naval(_) => &roots[2],
                }
                .as_ref()
                .copied()
                .map_err(Clone::clone)?;
                below = below.wrapping_add(size[1] as i32);
                items.push((root, [0, below]));
                Ok::<_, String>(())
            });
            let layout = collected.and_then(|()| {
                native_gui::finalize_grid_positions(
                    grid,
                    holder.content,
                    viewport,
                    items.iter().copied(),
                    holder.position,
                )
                .map_err(|refusal| refused_cause(STACK_WINDOW, &refusal))
            });
            let layout = match layout {
                Ok(layout) => layout,
                Err(why) => {
                    refuse_column(
                        self.items[index].iter().copied(),
                        &self.flotillas,
                        report,
                        &why,
                    );
                    continue;
                }
            };
            let Some(screen) = column_origin(anchor, &sizes) else {
                continue;
            };
            let representative = self.groups[index].members[0];
            let source = &self.icons[representative];
            let column = Arc::new(ColumnPlacement {
                hovered: self.hovered[index],
                priority,
                anchor: icon,
                identity: (
                    level,
                    source.kind,
                    source.owner,
                    source.province,
                    self.source_kinds[representative],
                ),
                count: plates.len(),
                screen,
                holder: holder.clone(),
                grid: layout.origin,
            });
            placed_columns.push(Arc::clone(&column));
            for (plate_index, local) in layout.items.into_iter().enumerate() {
                let (plate, size) = plates[plate_index];
                let place = [
                    column.grid[0].wrapping_add(local[0]),
                    column.grid[1].wrapping_add(local[1]),
                ];
                let origin = [0, 1].map(|axis| {
                    screen[axis] as f32 + column.holder.origin[axis] + place[axis] as f32
                });
                let hit_start = report.unit_hits.len();
                let shown = match plate {
                    Plate::Land(member) => {
                        let stack = &self.stacks[member];
                        inputs.display.land_plate(stack.gun.is_some(), stack.owner)
                    }
                    Plate::Naval(_) => true,
                };
                let (drawn, opacity, plate_member) = match plate {
                    Plate::Land(member) => {
                        let opacity =
                            plate_opacity(&inputs.visibility, self.stacks[member].gun.is_some());
                        let drawn = self.land_plate(
                            inputs,
                            report,
                            missing_owners,
                            member,
                            origin,
                            size,
                            priority,
                            &styles,
                        )?;
                        (drawn, opacity, member)
                    }
                    Plate::Naval(member) => {
                        self.naval_plate(inputs, report, missing_owners, member);
                        at_sea.push(AtSeaPlate {
                            member,
                            ordinal,
                            column: Arc::clone(&column),
                            local,
                        });
                        ordinal += 1;
                        continue;
                    }
                };
                let Some(sprite) = drawn else {
                    continue;
                };
                let base = hovered_layer(priority, column.hovered, ordinal, 0);
                for hit in &mut report.unit_hits[hit_start..] {
                    hit.layer = base;
                }
                let offset = base - layer(priority, 0, 0);
                ordinal += 1;
                if !shown {
                    self.pointer_drawn.set(false);
                    continue;
                }
                batch.at_window_standing(
                    viewer_native_gui::sprites::WindowKey::new(
                        viewer_native_gui::sprites::WindowPool::LandCounter,
                        0,
                        plate_member as u64,
                    ),
                    viewer_native_gui::sprites::WindowStamp {
                        painting: sprite.painting,
                        taken: taken(offset, icon, opacity),
                    },
                    sprite.stood,
                );
                batch.quads.extend(sprite.batch.quads.iter().map(|quad| {
                    let mut quad = quad.clone();
                    quad.layer += offset;
                    quad.anchor = icon;
                    quad.color[3] *= opacity;
                    quad
                }));
                batch.no_window();
                if self.pointer_drawn.take() && opacity > 0.0 {
                    report.pointed = self.pointer.map(|pointer| pointer.unit);
                }
            }
        }
        Ok(placed_columns)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn land_plate(
        &mut self,
        inputs: &CounterInputs<'_>,
        report: &mut CounterReport,
        missing_owners: &mut Vec<u32>,
        member: usize,
        origin: [f32; 2],
        size: [f32; 2],
        priority: i8,
        styles: &[PlateStyle<'_>; 2],
    ) -> io::Result<Option<&hovers::Sprite>> {
        let stack = &self.stacks[member];
        if !admitted(origin, size, inputs.extent()) && !abilities_on_screen(inputs, stack, origin)
        {
            return Ok(None);
        }
        if let Err(reason) = &stack.plate {
            *report
                .land_counter_gaps
                .entry(reason.clone().into())
                .or_default() += 1;
            return Ok(None);
        }
        if stack
            .moving
            .as_ref()
            .is_some_and(|m| m.signature().is_some())
            || stack.gun.as_ref().is_some_and(|gun| gun.moving)
        {
            report.moving += 1;
        }
        report.land_counters += 1;
        let (action, content) = match &stack.gun {
            Some(gun) => (gun.action, inputs.assets.gun_action.as_ref()),
            None => (stack.action, inputs.assets.land_action.as_ref()),
        };
        if let Some(content) = content
            && let Err(reason) =
                content.frame(action, stack.owner == self.vision.ours(), stack.coarse)
        {
            if let Some(count) = report.land_action_gaps.get_mut(reason) {
                *count += 1;
            } else {
                report.land_action_gaps.insert(reason.to_owned().into(), 1);
            }
        }
        report.unavailable_type_tints += usize::from(stack.type_tint.is_none());
        if let Err(cause) = stack.disclosure {
            report.sharing_unknown(
                "land counter type icons omitted (the custom glyph follows the owner's disclosure)",
                cause,
            );
        }
        if let Some(cause) = stack.grouping_unknown {
            report.sharing_unknown(
                "land counters with a member whose plate join the hour cannot decide (two sharing owners stay apart); the member did not join",
                cause,
            );
        }
        report.railway_guns += usize::from(stack.gun.is_some());
        if stack.hidden {
            report.no_intel_counters += 1;
        }
        if let Some(tokens) = &stack.abilities {
            report.unavailable_ability_icons += tokens
                .iter()
                .filter(|token| inputs.assets.ability_sheet(&token.token).is_none())
                .count();
        }
        let ours = stack.owner == self.vision.ours();
        let style = &styles[usize::from(stack.gun.is_some())];
        let hover = CounterHover {
            control: viewer_native_gui::counter_hovers::land::CounterControl::Normal,
            authored: style.authored.clone(),
            rect: [origin[0], origin[1], size[0], size[1]],
            remaining: stack
                .moving
                .as_ref()
                .and_then(|moving| moving.eta)
                .map(|(next, destination)| if ours { destination } else { next }),
            ours,
            coarse: stack.coarse,
            hidden: stack.hidden,
            railway_gun: stack.gun.is_some(),
            members: Arc::clone(&stack.divisions),
            timing: Arc::clone(&stack.timing),
        };
        let plate_rect = style.child.filter(|_| style.takes_pointer).and_then(|child| {
            let hq = stack.hq && stack.gun.is_none() && !stack.coarse;
            let sprite = if hq {
                "GFX_onmap_unit_counter_empty"
            } else {
                child.sprite.as_str()
            };
            let plain = inputs
                .assets
                .gui
                .registry()
                .sprite(sprite.as_bytes())
                .and_then(|facts| facts.kind.plain());
            if plain.is_some_and(|plain| plain.always_transparent) {
                return None;
            }
            if plain.is_some_and(|plain| plain.transparence_check) {
                *report.land_counter_gaps.entry("count_bg tests its sprite's alpha (transparencecheck), which a counter plate's hit does not; its pointer state is not drawn".into()).or_default() += 1;
                return None;
            }
            let size = inputs.assets.sheet(sprite)?.frame_size();
            Some([
                origin[0] + child.at[0],
                origin[1] + child.at[1],
                size[0] * child.scale,
                size[1] * child.scale,
            ])
        });
        let click = viewer_native_gui::counter_hovers::unit_hits::Hit {
            ours,
            port: false,
            plate: plate_rect,
            rect: hover.rect,
            kind: if stack.gun.is_some() {
                viewer_native_gui::counter_hovers::unit_hits::Kind::RailwayGun
            } else {
                viewer_native_gui::counter_hovers::unit_hits::Kind::Division
            },
            members: Arc::clone(&stack.divisions),
            layer: layer(priority, 0, 0),
        };
        report.unit_hits.push(click.clone());
        self.missing_flag(
            inputs,
            report,
            missing_owners,
            stack.flag_owner,
            stack.coarse,
        );
        let step = self.pointed(member).is_none().then_some(()).and_then(|()| {
            let sprite = self.sprites[member].as_ref()?;
            (sprite.priority == priority).then_some(())?;
            let step = [0, 1].map(|axis| origin[axis] - sprite.origin[axis]);
            (step != [0.0; 2] && step.iter().all(|value| value.fract() == 0.0))
                .then(|| step.map(|value| value as i32))
        });
        if let Some(step) = step {
            viewer_runtime::redraws::note("plates.moved", 1);
            let sprite = self.sprites[member].as_mut().expect("the step found the plate");
            for quad in &mut sprite.batch.quads {
                viewer_native_gui::anchored::shift(quad, step);
            }
            let by = step.map(|value| value as f32);
            for (_, rect, _) in &mut sprite.presented.controls {
                rect[0] += by[0];
                rect[1] += by[1];
            }
            sprite.origin = origin;
            sprite.stood = [0, 1].map(|axis| sprite.stood[axis] + step[axis]);
        } else if self.pointed(member).is_some()
            || self.sprites[member]
                .as_ref()
                .is_none_or(|sprite| sprite.origin != origin || sprite.priority != priority)
        {
            viewer_counter_kit::plan_probe::record(|counts| counts.plates_drawn += 1);
            viewer_runtime::redraws::note("plates.drawn", 1);
            let mut sprite = self.sprites[member]
                .take()
                .map(|sprite| sprite.batch)
                .unwrap_or_default();
            sprite.clear();
            let presented = draw_land_counter(inputs, &mut sprite, 0, origin, self, member, priority)?;
            self.sprites[member] = Some(hovers::Sprite {
                painting: viewer_native_gui::sprites::next_painting(),
                stood: [0; 2],
                origin: if self.pointed(member).is_some() {
                    [f32::NAN; 2]
                } else {
                    origin
                },
                priority,
                batch: sprite,
                presented,
            });
        } else {
            viewer_runtime::redraws::note("plates.kept", 1);
        }
        for why in self.pointer_refused.take() {
            *report.land_counter_gaps.entry(why.into()).or_default() += 1;
        }
        report.land_hovers.push(hover.clone());
        if let Some(sprite) = &self.sprites[member] {
            for reason in &sprite.presented.refused {
                *report
                    .land_counter_gaps
                    .entry(reason.clone().into())
                    .or_default() += 1;
            }
            for (control, rect, authored) in &sprite.presented.controls {
                report
                    .unit_hits
                    .push(viewer_native_gui::counter_hovers::unit_hits::Hit {
                        rect: *rect,
                        plate: None,
                        ..click.clone()
                    });
                report.land_hovers.push(CounterHover {
                    control: *control,
                    rect: *rect,
                    authored: authored.clone(),
                    ..hover.clone()
                });
            }
        }
        Ok(self.sprites[member].as_ref())
    }

    fn naval_plate(
        &self,
        inputs: &CounterInputs<'_>,
        report: &mut CounterReport,
        missing_owners: &mut Vec<u32>,
        member: usize,
    ) {
        let flotilla = &self.flotillas[member];
        report.naval_counters += 1;
        report.unavailable_type_tints +=
            usize::from(!flotilla.port && flotilla.type_tint.is_none());
        report.unavailable_naval_flags += usize::from(flotilla.flag_owner.is_none());
        report.moving += usize::from(flotilla.moving);
        if flotilla.hidden {
            report.no_intel_counters += 1;
        }
        if let Some(owner) = flotilla.flag_owner {
            self.missing_flag(inputs, report, missing_owners, owner, flotilla.coarse);
        }
    }

    fn missing_flag(
        &self,
        inputs: &CounterInputs<'_>,
        report: &mut CounterReport,
        missing_owners: &mut Vec<u32>,
        owner: u32,
        coarse: bool,
    ) {
        let atlas = if coarse {
            &inputs.assets.medium_flags
        } else {
            &inputs.assets.flags
        };
        if self.countries.tags_map().contains_key(&owner)
            && self.countries.flag_now(atlas, owner).is_none()
        {
            if let Some(reason) = self.countries.flag_refusal(owner) {
                *report.counter_flag_gaps.entry(reason).or_default() += 1;
            } else {
                missing_owners.push(owner);
            }
        }
    }
}

pub struct PlateStyle<'a> {
    authored: Option<viewer_native_gui::counter_hovers::gui_authored::Authored>,
    takes_pointer: bool,
    child: Option<&'a Child>,
}

impl<'a> PlateStyle<'a> {
    pub fn read(inputs: &CounterInputs<'a>, gun: bool) -> Self {
        let name = if gun { GUN_WINDOW } else { LAND_WINDOW };
        let root = inputs.windows.guis.template(name);
        Self {
            authored: root.and_then(|root| {
                viewer_native_gui::counter_hovers::gui_authored::Authored::read(root, false)
            }),
            takes_pointer: root
                .and_then(|root| root.find(content_reader_gui::gui::native::Kind::Button, "count_bg"))
                .and_then(content_reader_gui::gui::native::Element::button)
                .is_some_and(|button| !button.always_transparent && button.enabled),
            child: if gun {
                inputs.assets.layouts.gun.as_ref().ok().map(|gun| &gun.plate)
            } else {
                inputs.assets.layouts.land.as_ref().ok().map(|land| &land.plate)
            },
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Stack {
    pub selected: bool,
    pub owner: u32,
    pub logical: Option<u32>,
    pub plate: Result<[f32; 4], String>,
    pub flag_owner: u32,
    pub coarse: bool,
    pub type_tint: Option<type_tint::Side>,
    pub hq: bool,
    pub portrait: Option<counter_order::Portrait>,
    pub hidden: bool,
    pub hit_points: Vec<i64>,
    pub organisation: Vec<i64>,
    pub bars: bool,
    pub status_recorded: bool,
    pub members: Vec<counter_status::Member>,
    pub divisions: Arc<[u64]>,
    pub timing: Arc<[viewer_native_gui::counter_hovers::land::Timing]>,
    pub template: u64,
    pub alternate_template: u64,
    pub disclosure: Result<bool, SharingUnknown>,
    pub grouping_unknown: Option<SharingUnknown>,
    pub moving: Option<Movement>,
    pub action: counter_action::Action,
    pub marker: Result<Option<[f32; 4]>, String>,
    pub leader_en_route: Option<bool>,
    pub defending_area: bool,
    pub gun: Option<GunStack>,
    pub abilities: Option<Vec<AbilityIcon>>,
}

#[derive(Clone, PartialEq)]
pub struct GunStack {
    pub name: String,
    pub location_warning: Option<bool>,
    pub action: counter_action::Action,
    pub moving: bool,
}

pub struct CounterTables<'a> {
    pub type_tints: &'a type_tint::Index,
    pub countries: &'a Arc<Countries>,
    pub icons: &'a Arc<TemplateIcons>,
    pub status: &'a DivisionStatuses,
    pub movement: &'a Movements,
    pub actions: &'a counter_action::Actions,
    pub vision: &'a Arc<Vision>,
    pub order: Option<&'a counter_order::RecordedOrder>,
    pub native: &'a NativeInputs,
    pub orders: &'a counter_inputs::OrdersIndex,
    pub guns: &'a GunInputs,
    pub abilities: &'a AbilityInputs,
    pub task_forces: &'a TaskForceMotions,
    pub hq_providers: &'a BTreeSet<u64>,
    pub portraits: &'a BTreeMap<u64, counter_order::Portrait>,
    pub icon_inputs: &'a IconInputs,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DivisionStatus {
    pub recorded: bool,
    pub maxima: Option<[i64; 2]>,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub supply_ratio: counter_status::SupplyRatio,
    pub attrition: bool,
    pub exercising: Option<bool>,
    pub has_order: Option<bool>,
    pub invalid_orders: u8,
    pub naval_transfer: Option<bool>,
    pub strength_factor: i64,
}

impl DivisionStatus {
    pub const MISSING: Self = Self {
        recorded: false,
        maxima: None,
        entrenchment: 0,
        entrenchment_cap: 0,
        supply_ratio: counter_status::SupplyRatio::Missing,
        attrition: false,
        exercising: None,
        has_order: None,
        invalid_orders: 0,
        naval_transfer: None,
        strength_factor: counter_status::FIXED,
    };

    pub fn recorded(
        row: StatusFields,
        maxima: Option<[i64; 2]>,
        naval_transfer: Option<bool>,
    ) -> Self {
        Self {
            recorded: true,
            maxima,
            entrenchment: row.entrenchment,
            entrenchment_cap: row.entrenchment_cap,
            supply_ratio: row.supply_ratio.map_or(
                counter_status::SupplyRatio::Missing,
                counter_status::SupplyRatio::from_bounds,
            ),
            attrition: row.attrition,
            exercising: Some(row.exercising != 0),
            has_order: Some(row.has_order != 0),
            invalid_orders: row.invalid_orders,
            naval_transfer,
            strength_factor: row.strength_factor,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusFields {
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub supply_ratio: Option<[i64; 2]>,
    pub attrition: bool,
    pub exercising: u8,
    pub has_order: u8,
    pub invalid_orders: u8,
    pub strength_factor: i64,
}

pub type StackKey = (u32, u32);

fn taken(offset: u64, icon: Option<IconAnchor>, opacity: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    offset.hash(&mut hasher);
    icon.map(|anchor| anchor.key()).hash(&mut hasher);
    opacity.to_bits().hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone)]
pub struct LandUnit<'m> {
    pub ordinal: u32,
    pub alternate_template: u64,
    pub shared_vision: Result<bool, SharingUnknown>,
    pub separate: bool,
    pub id: u64,
    pub owner: u32,
    pub logical: Option<u32>,
    pub flag_owner: u32,
    pub type_tint: Option<type_tint::Side>,
    pub template: u64,
    pub orders_group: u64,
    pub hq: bool,
    pub sight: Sight,
    pub hp: i64,
    pub org: i64,
    pub bars: bool,
    pub status_recorded: bool,
    pub status: counter_status::Member,
    pub movement: Option<&'m Movement>,
    pub action: counter_action::Action,
    pub group_parent: Option<u64>,
    pub parent_branch: Option<bool>,
    pub withdrawing: Option<bool>,
    pub defending_area: Option<bool>,
    pub commander: Option<Option<u64>>,
    pub gun: Option<GunUnit>,
}

#[derive(Clone)]
pub struct GunUnit {
    pub name: String,
    pub timing: viewer_native_gui::counter_hovers::land::Timing,
    pub location_warning: Option<bool>,
    pub join_action: u8,
    pub action: counter_action::Action,
    pub position: Option<counter_move::Position>,
}

#[derive(Default)]
pub struct Source<'a> {
    pub units: Vec<SourceUnit<'a>>,
}

#[allow(clippy::large_enum_variant)]
pub enum SourceUnit<'a> {
    Land(LandUnit<'a>),
    Naval(NavalUnit<'a>),
}

impl SourceUnit<'_> {
    fn ordinal(&self) -> u32 {
        match self {
            Self::Land(unit) => unit.ordinal,
            Self::Naval(unit) => unit.ordinal,
        }
    }

    fn owner(&self) -> u32 {
        match self {
            Self::Land(unit) => unit.owner,
            Self::Naval(unit) => unit.slot,
        }
    }
}

pub fn plate_opacity(visibility: &Visibility, gun: bool) -> f32 {
    if gun { 1.0 } else { visibility.army_opacity }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plate {
    Land(usize),
    Naval(usize),
}

impl LandUnit<'_> {
    pub fn can_join(
        &self,
        rep: &LandUnit<'_>,
        templates: &TemplateIcons,
        countries: &Countries,
        rules: JoinRules,
    ) -> Result<bool, SharingUnknown> {
        if self.gun.is_some() != rep.gun.is_some() {
            return Ok(false);
        }
        let coarse = rules.coarse;
        if !coarse && (self.separate || self.hq || rep.separate || rep.hq) {
            return Ok(false);
        }
        if !rules.owners_join(countries, rep.owner, self.owner) {
            return Ok(false);
        }
        if coarse {
            return Ok(true);
        }
        if let (Some(gun), Some(first)) = (&self.gun, &rep.gun) {
            return Ok(gun.can_join(first, self, rep, countries));
        }
        let disclosed =
            |shared: bool| self.joins_disclosed(rep, templates, countries, rules, shared);
        match self.shared_vision {
            Ok(shared) => Ok(disclosed(shared)),
            Err(cause) => match (disclosed(true), disclosed(false)) {
                (yes, no) if yes == no => Ok(yes),
                _ => Err(cause),
            },
        }
    }

    fn joins_disclosed(
        &self,
        rep: &LandUnit<'_>,
        templates: &TemplateIcons,
        countries: &Countries,
        rules: JoinRules,
        shared: bool,
    ) -> bool {
        if shared && rep.owner != self.owner {
            return false;
        }
        let unit_type = |unit: &LandUnit<'_>| {
            let custom_template = if shared {
                unit.template
            } else {
                unit.alternate_template
            };
            templates
                .grouping
                .get(&unit.alternate_template)
                .and_then(|(name, _, _)| {
                    templates
                        .grouping
                        .get(&custom_template)
                        .map(|(_, custom, keep)| {
                            (name, counter_custom_icon(*custom, *keep, rules.nato))
                        })
                })
        };
        match (unit_type(rep), unit_type(self)) {
            (Some(a), Some(b)) if a == b => {}
            (None, None) if rep.alternate_template == self.alternate_template => {}
            _ => return false,
        }
        if shared
            && templates.priorities.get(&rep.template) != templates.priorities.get(&self.template)
        {
            return false;
        }
        if countries.is_ours(rep.owner) {
            if rep.movement.map(|m| m.idle) != self.movement.map(|m| m.idle) {
                return false;
            }
            let shared_parent = rep.orders_group != 0
                && self.orders_group != 0
                && rep.group_parent.is_some()
                && rep.group_parent == self.group_parent
                && rep.parent_branch == Some(true)
                && self.parent_branch == Some(true);
            if !shared_parent && rep.orders_group != self.orders_group {
                return false;
            }
        }
        true
    }
}

impl GunUnit {
    fn can_join(
        &self,
        first: &GunUnit,
        unit: &LandUnit,
        rep: &LandUnit,
        countries: &Countries,
    ) -> bool {
        if self.name.is_empty() || self.name != first.name {
            return false;
        }
        if countries.is_ours(rep.owner) {
            let idle = |action| matches!(action, 1 | 12);
            if self.join_action != first.join_action
                && (idle(self.join_action) || idle(first.join_action))
            {
                return false;
            }
            if unit.orders_group != rep.orders_group {
                return false;
            }
        }
        true
    }
}

pub fn first_fit<T: Copy>(
    units: impl IntoIterator<Item = T>,
    mut joins: impl FnMut(T, T) -> bool,
) -> Vec<Vec<T>> {
    let mut items: Vec<Vec<T>> = Vec::new();
    for unit in units {
        match items.iter_mut().find(|item| joins(item[0], unit)) {
            Some(item) => item.push(unit),
            None => items.push(vec![unit]),
        }
    }
    items
}

pub fn land_join(
    unit: &LandUnit<'_>,
    rep: &LandUnit<'_>,
    templates: &TemplateIcons,
    countries: &Countries,
    rules: JoinRules,
    undecided: &mut BTreeMap<u64, SharingUnknown>,
) -> bool {
    match unit.can_join(rep, templates, countries, rules) {
        Ok(joins) => joins,
        Err(cause) => {
            undecided.entry(unit.id).or_insert(cause);
            false
        }
    }
}

pub fn glyph_disclosure(
    rep: &LandUnit<'_>,
    icons: &TemplateIcons,
    nato: bool,
    declared: &dyn Fn(&str) -> bool,
) -> Result<bool, SharingUnknown> {
    rep.shared_vision.or_else(|cause| {
        let icon =
            |shared| icons.icon(rep.template, rep.alternate_template, shared, nato, declared);
        if icon(true) == icon(false) {
            Ok(false)
        } else {
            Err(cause)
        }
    })
}

#[derive(Clone, Default)]
pub struct DivisionStatuses {
    pub by_division: BTreeMap<u64, DivisionStatus>,
    pub witnessed: BTreeMap<u64, Option<bool>>,
    pub transfers: BTreeMap<u64, bool>,
    by_key: Option<Arc<KeyedStatuses>>,
}

struct KeyedStatuses {
    frame: Arc<DecodedFrame>,
    recorded: bool,
    looked: std::sync::Mutex<std::collections::HashMap<u64, DivisionStatus>>,
    lookups: super::Lookups,
}

impl DivisionStatuses {
    const MISSING: DivisionStatus = DivisionStatus::MISSING;

    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let movements: BTreeMap<u64, _> = rows::<tables::DivisionMovement>(frame)?
            .rows()
            .iter()
            .map(|row| (row.division, row))
            .collect();
        let transfers: BTreeMap<u64, bool> = movements
            .iter()
            .map(|(&division, row)| (division, row.naval_transfer != 0))
            .collect();
        if !frame.recorded(replay_data::rows::TableId::DivisionStatus) {
            let witnessed = rows::<tables::DivisionModelInputs>(frame)?
                .rows()
                .iter()
                .map(|row| (row.division, row.exercising.map(|value| value != 0)))
                .collect();
            return Ok(Self { witnessed, transfers, ..Self::default() });
        }
        let panels: BTreeMap<u64, _> = rows::<tables::DivisionPanelStats>(frame)?
            .rows()
            .iter()
            .map(|row| (row.division, row))
            .collect();
        let statuses = rows::<tables::DivisionStatus>(frame)?.rows();
        let by_division = statuses
            .iter()
            .map(|row| (row.division, status_of(row, panels.get(&row.division).copied(), movements.get(&row.division).copied())))
            .collect();
        Ok(Self { by_division, transfers, ..Self::default() })
    }

    pub fn read_by_key(frame: &Arc<DecodedFrame>, lookups: super::Lookups) -> io::Result<Self> {
        let recorded = frame.recorded(replay_data::rows::TableId::DivisionStatus);
        rows::<tables::DivisionMovement>(frame)?;
        if recorded {
            rows::<tables::DivisionPanelStats>(frame)?;
            rows::<tables::DivisionStatus>(frame)?;
        } else {
            rows::<tables::DivisionModelInputs>(frame)?;
        }
        Ok(Self {
            by_key: Some(Arc::new(KeyedStatuses { frame: Arc::clone(frame), recorded, looked: Default::default(), lookups })),
            ..Self::default()
        })
    }

    pub fn get(&self, division: u64) -> DivisionStatus {
        match &self.by_key {
            Some(keyed) => keyed.get(division),
            None => self.by_division.get(&division).copied().unwrap_or_else(|| DivisionStatus {
                exercising: self.witnessed.get(&division).copied().flatten(),
                naval_transfer: self.transfers.get(&division).copied(),
                ..Self::MISSING
            }),
        }
    }

    pub fn recorded(&self, division: u64) -> bool {
        match &self.by_key {
            Some(keyed) => keyed.get(division).maxima.is_some(),
            None => self.by_division.get(&division).is_some_and(|status| status.maxima.is_some()),
        }
    }
}

impl KeyedStatuses {
    fn get(&self, division: u64) -> DivisionStatus {
        let mut looked = self.looked.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(&status) = looked.get(&division) {
            self.lookups.kept();
            return status;
        }
        self.lookups.made();
        let frame = &*self.frame;
        let movement = keyed::<tables::DivisionMovement>(frame, division, |row| row.division);
        let status = if self.recorded {
            match keyed::<tables::DivisionStatus>(frame, division, |row| row.division) {
                Some(row) => {
                    let panel = keyed::<tables::DivisionPanelStats>(frame, division, |row| row.division);
                    status_of(row, panel, movement)
                }
                None => DivisionStatus {
                    naval_transfer: movement.map(|row| row.naval_transfer != 0),
                    ..DivisionStatus::MISSING
                },
            }
        } else {
            let exercising = keyed::<tables::DivisionModelInputs>(frame, division, |row| row.division)
                .and_then(|row| row.exercising.map(|value| value != 0));
            DivisionStatus {
                exercising,
                naval_transfer: movement.map(|row| row.naval_transfer != 0),
                ..DivisionStatus::MISSING
            }
        };
        looked.insert(division, status);
        status
    }
}

#[derive(Clone, Default)]
pub struct IconInputs {
    whole: BTreeMap<u64, (u8, u64)>,
    by_key: Option<(Arc<DecodedFrame>, super::Lookups)>,
}

impl IconInputs {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let whole = rows::<tables::ArmyIconInputs>(frame)?.rows().iter().map(|row| (row.division, icon_input(row))).collect();
        Ok(Self { whole, by_key: None })
    }

    pub fn read_by_key(frame: &Arc<DecodedFrame>, lookups: super::Lookups) -> io::Result<Self> {
        rows::<tables::ArmyIconInputs>(frame)?;
        Ok(Self { whole: BTreeMap::new(), by_key: Some((Arc::clone(frame), lookups)) })
    }

    pub fn get(&self, division: u64) -> Option<(u8, u64)> {
        match &self.by_key {
            Some((frame, lookups)) => {
                lookups.made();
                keyed::<tables::ArmyIconInputs>(frame, division, |row| row.division).map(icon_input)
            }
            None => self.whole.get(&division).copied(),
        }
    }
}

fn icon_input(row: &replay_data::rows::ArmyIconInputRow) -> (u8, u64) {
    (row.separate_counter, row.alternate_template)
}

fn keyed<T: replay_data::rows::TableSchema>(frame: &DecodedFrame, key: u64, of: impl Fn(&T::Row) -> u64) -> Option<&T::Row> {
    let rows = rows::<T>(frame).ok()?.rows();
    rows.binary_search_by_key(&key, |row| of(row)).ok().map(|at| &rows[at])
}

fn status_of(
    row: &replay_data::rows::DivisionStatusRow,
    panel: Option<&replay_data::rows::DivisionPanelStatsRow>,
    movement: Option<&replay_data::rows::DivisionMovementRow>,
) -> DivisionStatus {
    let maxima = panel.and_then(|panel| Some([panel.maximum_hit_points()?, panel.maximum_organisation()?]));
    let fields = StatusFields {
        entrenchment: row.entrenchment,
        entrenchment_cap: row.entrenchment_cap,
        supply_ratio: row.recorded_supply_ratio_bounds(),
        attrition: row.has_attrition(),
        exercising: row.exercising,
        has_order: row.has_order,
        invalid_orders: row.invalid_orders,
        strength_factor: row.strength_factor,
    };
    let naval_transfer = movement.map(|movement| movement.naval_transfer != 0);
    DivisionStatus::recorded(fields, maxima, naval_transfer)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemplateIcon {
    pub name: String,
    pub custom_icon: i32,
    pub custom_icon_nato: u8,
}

#[derive(Clone, Default)]
pub struct TemplateIcons {
    pub by_template: BTreeMap<u64, TemplateIcon>,

    pub grouping: BTreeMap<u64, (Vec<u8>, i32, u8)>,
    pub priorities: BTreeMap<u64, u32>,
}

const TEMPLATE_ICON_TABLES: &[replay_data::rows::TableId] =
    &[replay_data::rows::TableId::DivisionTemplateIcons];

#[derive(Clone)]
pub struct TemplateIconMemo(Arc<viewer_runtime::memo::Keyed<TemplateIcons>>);

impl Default for TemplateIconMemo {
    fn default() -> Self {
        Self(Arc::new(viewer_runtime::memo::Keyed::new(
            TEMPLATE_ICON_TABLES,
        )))
    }
}

impl TemplateIconMemo {
    pub fn read(&self, frame: &DecodedFrame) -> io::Result<Arc<TemplateIcons>> {
        self.0.get_or_build(frame, (), |projected| {
            TemplateIcons::read(projected).map(Arc::new)
        })
    }
}

impl TemplateIcons {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let mut by_template = BTreeMap::new();
        let mut grouping = BTreeMap::new();
        let mut priorities = BTreeMap::new();
        for row in rows::<tables::DivisionTemplateIcons>(frame)?.rows() {
            priorities.insert(row.template, row.priority_unit);
            if !row.priority_unit_name.is_empty() {
                grouping.insert(
                    row.template,
                    (
                        row.priority_unit_name.clone(),
                        row.custom_icon,
                        row.custom_icon_nato,
                    ),
                );
            }
            by_template.insert(
                row.template,
                TemplateIcon {
                    name: String::from_utf8_lossy(&row.priority_unit_name).into_owned(),
                    custom_icon: row.custom_icon,
                    custom_icon_nato: row.custom_icon_nato,
                },
            );
        }
        Ok(Self {
            by_template,
            grouping,
            priorities,
        })
    }

    pub fn icon(
        &self,
        template: u64,
        alternate: u64,
        shared: bool,
        nato_counters: bool,
        declared: &dyn Fn(&str) -> bool,
    ) -> Option<templates::TypeIcon> {
        let custom = if shared {
            let own = self.by_template.get(&template)?;
            templates::counter_custom_icon(own.custom_icon, own.custom_icon_nato, nato_counters)
        } else {
            -1
        };
        let plain = self.by_template.get(&alternate)?;
        Some(templates::type_icon(
            &plain.name,
            custom,
            nato_counters,
            declared,
        ))
    }
}

#[derive(Clone, Default)]
pub struct GunInputs {
    pub rows: Option<Vec<RailwayGunCounterInputRow>>,
    pub recorded: usize,
    pub names: BTreeMap<u32, Result<String, String>>,
    pub refused: BTreeSet<String>,
    pub rail: Option<BTreeMap<u32, guns::RailStatus>>,
    pub destination_enemy: BTreeMap<u64, Result<bool, &'static str>>,
}

impl GunInputs {
    pub fn hostility_only(frame: &DecodedFrame, hostility: &counter_action::DestinationHostility<'_>) -> io::Result<Self> {
        let table = rows::<tables::RailwayGunCounterInputs>(frame)?;
        Ok(Self { destination_enemy: destination_enemy(table.rows(), hostility), ..Self::default() })
    }

    pub fn read(
        frame: &DecodedFrame,
        hostility: &counter_action::DestinationHostility<'_>,
        subunits: Result<&TokenKeys, &str>,
    ) -> io::Result<Self> {
        let table = rows::<tables::RailwayGunCounterInputs>(frame)?;
        let recorded = table.rows().len();
        if !table.is_complete() {
            return Ok(Self {
                recorded,
                ..Self::default()
            });
        }
        let wanted: BTreeSet<u32> = table.rows().iter().map(|gun| gun.subunit_token).collect();
        let mut refused = BTreeSet::new();
        let names = if wanted.is_empty() {
            BTreeMap::new()
        } else {
            let tokens = TokenNames::new(frame)?;
            wanted
                .iter()
                .map(|&token| {
                    let named = gun_name(tokens.name(token).map_err(|reason| reason.to_string()), subunits);
                    if let Err(reason) = &named {
                        refused.insert(format!("railway gun sub-unit: {reason}"));
                    }
                    (token, named)
                })
                .collect()
        };
        let cooldowns = rows::<tables::RailCooldowns>(frame)?;
        let rail = cooldowns.is_complete().then(|| {
            cooldowns
                .rows()
                .iter()
                .filter_map(|row| {
                    guns::RailStatus::read(row, frame.hour)
                        .ok()
                        .map(|rail| (row.province, rail))
                })
                .collect()
        });
        let destination_enemy = destination_enemy(table.rows(), hostility);
        Ok(Self {
            rows: Some(table.rows().to_vec()),
            recorded,
            names,
            refused,
            rail,
            destination_enemy,
        })
    }
}

fn destination_enemy(guns: &[RailwayGunCounterInputRow], hostility: &counter_action::DestinationHostility<'_>) -> BTreeMap<u64, Result<bool, &'static str>> {
    guns.iter()
        .filter(|gun| !gun.ordinary_path.is_empty())
        .map(|gun| (gun.gun, hostility.destination_enemy(&gun.ordinary_path, gun.ordinary_path.len() as u32, Some(gun.logical_owner))))
        .collect()
}

const ACTIVE_ABILITY_PHASE: u8 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbilityIcon {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct AbilityInputs {
    complete: bool,
    by_commander: BTreeMap<(u32, u64), Vec<AbilityIcon>>,
}

impl AbilityInputs {
    pub fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> io::Result<Self> {
        let table = rows::<tables::ActiveAbilities>(frame)?;
        if !table.is_complete() {
            return Ok(Self::default());
        }
        let mut active: Vec<&ActiveAbilityRow> = table
            .rows()
            .iter()
            .filter(|row| row.phase == ACTIVE_ABILITY_PHASE)
            .collect();
        active.sort_by_key(|row| (row.country, row.ordinal));
        let mut by_commander: BTreeMap<(u32, u64), Vec<AbilityIcon>> = BTreeMap::new();
        for row in active {
            let Some(&country) = links.get(&row.country) else {
                return Ok(Self::default());
            };
            if let Some(caster) = row.caster {
                by_commander
                    .entry((country, caster))
                    .or_default()
                    .push(AbilityIcon {
                        country: row.country,
                        ordinal: row.ordinal,
                        token: row.token.clone(),
                    });
            }
        }
        Ok(Self {
            complete: true,
            by_commander,
        })
    }

    pub fn of(&self, country: u32, commander: u64) -> Option<&[AbilityIcon]> {
        self.complete.then(|| {
            self.by_commander
                .get(&(country, commander))
                .map_or(&[][..], Vec::as_slice)
        })
    }
}

pub fn update_abilities(
    tokens: &mut Option<Vec<AbilityIcon>>,
    sight: Sight,
    commander: Option<Option<u64>>,
    country: u32,
    abilities: &AbilityInputs,
) {
    if sight != Sight::Full {
        return;
    }
    match commander {
        Some(Some(commander)) => *tokens = abilities.of(country, commander).map(<[_]>::to_vec),
        Some(None) => {}
        None => *tokens = None,
    }
}

fn add_railway_guns(
    inputs: &CounterInputs<'_>,
    owner: &impl Fn(u32) -> Option<u32>,
    land: &CounterTables<'_>,
    sources: &mut BTreeMap<StackKey, Source<'_>>,
    out: &mut CounterFrame<impl Sized, impl Sized, impl Sized, impl Sized>,
) {
    let Some(rows) = &land.guns.rows else {
        out.report.unavailable_railway_guns += land.guns.recorded;
        return;
    };
    for reason in &land.guns.refused {
        if !out.report.refused_tokens.contains(reason) {
            out.report.refused_tokens.insert(reason.clone());
        }
    }
    let world = inputs.route_world();
    let anchor = |id, slot| inputs.content.anchor(id, slot);
    let (anchor_of, land_of) = (GunGround::anchor_of(inputs.content), GunGround::land_of(inputs.map));
    let ground = GunGround { anchor: &anchor_of, land: &land_of, movement: inputs.movement };
    let joins = TodayGunJoins { land, owner };
    for gun in rows {
        let fields = GunFields::of(gun);
        let member = match gun_member(&fields, ground, &joins) {
            Gun::Member(member) => member,
            Gun::Unnamed(_) => {
                out.report.unavailable_railway_guns += 1;
                continue;
            }
            Gun::Out(_) => continue,
        };
        if member.unplaced {
            out.report.unplaced_railway_gun_routes += 1;
        }
        if member.location_warning.is_none() {
            out.report.unavailable_railway_gun_locations += 1;
        }
        let route = fields.route();
        let position = match (member.timing.kind, &world) {
            (Some(kind), Some(world)) => route.position(gun.province, kind, world, &anchor),
            _ => route
                .standing(gun.province, true, member.kind, &anchor)
                .map(counter_move::Position::still),
        };
        sources
            .entry((gun.province, member.kind))
            .or_default()
            .units
            .push(SourceUnit::Land(member.unit(position)));
    }
}

fn add_layered_guns(inputs: &CounterInputs<'_>, guns: &LayeredGuns, sources: &mut BTreeMap<StackKey, Source<'_>>, out: &mut CounterFrame<impl Sized, impl Sized, impl Sized, impl Sized>) {
    if !guns.complete {
        out.report.unavailable_railway_guns += guns.recorded;
        return;
    }
    for reason in &guns.refused {
        if !out.report.refused_tokens.contains(reason) {
            out.report.refused_tokens.insert(reason.clone());
        }
    }
    out.report.unavailable_railway_guns += guns.unnamed;
    let world = inputs.route_world();
    let anchor = |id, slot| inputs.content.anchor(id, slot);
    for placed in &guns.members {
        let member = &placed.member;
        if member.unplaced {
            out.report.unplaced_railway_gun_routes += 1;
        }
        if member.location_warning.is_none() {
            out.report.unavailable_railway_gun_locations += 1;
        }
        let route = placed.route.route();
        let position = match (member.timing.kind, &world) {
            (Some(kind), Some(world)) => route.position(member.province, kind, world, &anchor),
            _ => route.standing(member.province, true, member.kind, &anchor).map(counter_move::Position::still),
        };
        sources.entry((member.province, member.kind)).or_default().units.push(SourceUnit::Land(member.clone().unit(position)));
    }
}

pub const LAND_KIND: u32 = 0;

pub type DivisionCounters = BTreeMap<StackKey, Vec<LandUnit<'static>>>;

pub fn division_counters(
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    land: &CounterTables<'_>,
) -> io::Result<DivisionCounters> {
    let mut sources = DivisionCounters::new();
    let leaders: BTreeMap<u64, u64> = rows::<tables::OrdersGroups>(frame)?
        .rows()
        .iter()
        .map(|group| (group.handle, group.leader))
        .collect();
    let joins = TableJoins { land, owner, leaders: &leaders };
    for division in rows::<tables::Divisions>(frame)?.rows() {
        let Some((key, member)) = division_member(DivisionFields::of(division), &joins) else {
            continue;
        };
        let unit = member.unit(land.actions.get(division.id));
        sources.entry(key).or_default().push(unit);
    }
    Ok(sources)
}

pub fn division_counters_of(stacks: &LandStacks) -> DivisionCounters {
    stacks
        .iter()
        .map(|(&key, members)| (key, members.iter().map(|(member, action)| member.clone().unit(*action)).collect()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn land_counters<'a>(
    inputs: &CounterInputs<'_>,
    divisions: &DivisionCounters,
    layered_land: Option<&LandStacks>,
    layered_guns: Option<&LayeredGuns>,
    owner: &impl Fn(u32) -> Option<u32>,
    land: &CounterTables<'a>,
    naval: &NavalContext<'a>,
    geography: &Geography,
    friendly: &impl Fn(u32, u32) -> bool,
    mut retained: Option<&mut ColumnPlans>,
    out: &mut CounterFrame<impl Sized, impl Sized, impl Sized, impl Sized>,
) -> io::Result<PreparedLand> {
    let mut sources: BTreeMap<StackKey, Source<'a>> = BTreeMap::new();
    let admission = Admission::new(inputs.camera, inputs.viewport);
    let armies = inputs.visibility.unit_kinds == UnitKinds::All;
    if armies {
        let mut last: Option<(u32, bool)> = None;
        let admitted: Vec<_> = divisions
            .iter()
            .filter(|(key, _)| match last {
                Some((province, admits)) if province == key.0 => admits,
                _ => {
                    let admits = admission.admits_province(&inputs.content.bounds, key.0);
                    last = Some((key.0, admits));
                    admits
                }
            })
            .collect();
        land.movement.prepare(
            admitted.iter().flat_map(|(_, units)| units.iter().map(|unit| unit.id)),
            inputs.content,
            inputs.movement,
        );
        for (&key, units) in admitted {
            let source = sources.entry(key).or_default();
            source.units.extend(units.iter().map(|unit| {
                let unit = unit.clone();
                let movement = land.movement.get(unit.id);
                SourceUnit::Land(LandUnit { movement, ..unit })
            }));
        }
    }
    let unavailable = out.report.unavailable_railway_guns;
    if armies {
        match layered_guns {
            Some(guns) => add_layered_guns(inputs, guns, &mut sources, out),
            None => add_railway_guns(inputs, owner, land, &mut sources, out),
        }
    }
    if armies && inputs.assets.layouts.gun.is_err() {
        let recorded = layered_guns.map_or(land.guns.recorded, |guns| guns.recorded);
        out.report.unavailable_railway_guns = unavailable + recorded.max(out.report.unavailable_railway_guns - unavailable);
    }
    add_task_forces(inputs, owner, &land.naval(), naval, &admission, |key, unit| {
        sources.entry(key).or_default().units.push(SourceUnit::Naval(unit));
    }, &mut out.report);
    if viewer_runtime::view_record::on() {
        record_admitted(&sources, retained.as_deref_mut());
    }
    let mut icons = Vec::new();
    let mut source_kinds = Vec::new();
    let mut admitted = Vec::new();
    let mut positions = Vec::new();
    let mut units = Vec::new();
    for ((province, kind), mut source) in sources {
        source.units.sort_by_key(SourceUnit::ordinal);
        let naval = matches!(source.units.last(), Some(SourceUnit::Naval(_)));
        let position = if naval && is_land(inputs, province) {
            inputs
                .content
                .port(province)
                .map(counter_move::Position::still)
        } else {
            match &source.units[0] {
                SourceUnit::Land(first) => match &first.gun {
                    Some(gun) => gun.position.clone(),
                    None => stack_position(inputs, province, first.movement),
                },
                SourceUnit::Naval(first) => first.position.clone(),
            }
        };
        let Some(position) = position else {
            out.report.unanchored += 1;
            continue;
        };
        icons.push(counter_groups::Icon {
            kind: if naval { NAVAL_KIND } else { LAND_KIND },
            owner: source.units[0].owner(),
            province,
            state: geography.state_of.get(&province).copied(),
            region: inputs.content.region(province),
            anchor: position.at(),
            track: None,
        });
        source_kinds.push(kind);
        units.push(match &source.units[0] {
            SourceUnit::Land(unit) if unit.gun.is_some() => Unit::RailwayGun(unit.id),
            SourceUnit::Land(unit) => Unit::Division(unit.id),
            SourceUnit::Naval(unit) => Unit::TaskForce(unit.force.id),
        });
        positions.push(position);
        admitted.push(source);
    }
    let levels: Vec<Level> = icons
        .iter()
        .map(|icon| {
            inputs
                .content
                .defines
                .groups
                .grouping(inputs.icon_height(icon.anchor))
        })
        .collect();
    let groups = counter_groups::group(&icons, &levels, geography.extents, friendly);
    let mut drawn = Vec::new();
    let mut styles = Vec::new();
    let mut flotillas = Vec::new();
    let mut items = Vec::new();
    let mut plans = Vec::new();
    let mut undecided = BTreeMap::new();
    let mut previous = retained
        .as_deref_mut()
        .map(|memo| std::mem::take(&mut memo.previous))
        .unwrap_or_default();
    for group in &groups {
        let height = inputs.icon_height(group.anchor);
        let rules = JoinRules {
            coarse: inputs
                .content
                .defines
                .coarse(height, inputs.visibility.strategic_grouping),
            color_mode: inputs.settings.color_mode,
            nato: inputs.settings.nato,
        };
        let units: Vec<_> = group
            .members
            .iter()
            .flat_map(|source| admitted[*source].units.iter())
            .collect();
        let program = if let Some(memo) = retained.as_deref_mut() {
            let key = ColumnKey {
                sources: group
                    .members
                    .iter()
                    .map(|&source| OrderedSource {
                        province: icons[source].province,
                        kind: source_kinds[source],
                        values: SourceValues(layered_land.and_then(|stacks| stacks.get(&(icons[source].province, source_kinds[source]))).cloned()),
                        members: admitted[source]
                            .units
                            .iter()
                            .map(|unit| {
                                let id = match unit {
                                    SourceUnit::Land(unit) if unit.gun.is_some() => {
                                        Unit::RailwayGun(unit.id)
                                    }
                                    SourceUnit::Land(unit) => Unit::Division(unit.id),
                                    SourceUnit::Naval(unit) => Unit::TaskForce(unit.force.id),
                                };
                                (id, unit.ordinal())
                            })
                            .collect(),
                    })
                    .collect(),
                coarse: rules.coarse,
                split_selected: inputs.selected.len() as i32
                    <= inputs.content.defines.split_selected_limit
                    && height < inputs.content.defines.units_cutoff,
                unknown_before: units
                    .iter()
                    .map(|unit| match unit {
                        SourceUnit::Land(unit) => undecided.get(&unit.id).copied(),
                        SourceUnit::Naval(_) => None,
                    })
                    .collect(),
            };
            let leader = (
                icons[group.members[0]].province,
                source_kinds[group.members[0]],
            );
            let before = previous.remove(&leader);
            let hit = before.as_ref().is_some_and(|(before, _)| *before == key);
            viewer_counter_kit::plan_probe::record(|counts| {
                let members_equal = before
                    .as_ref()
                    .is_some_and(|(before, _)| before.sources == key.sources);
                counts.ordered_groups += 1;
                counts.ordered_group_members_equal += u64::from(members_equal);
                counts.semantic_plans += 1;
                counts.semantic_plan_hits += u64::from(hit);
                counts.column_plan_hits += u64::from(hit && !units.is_empty());
            });
            viewer_runtime::redraws::note(if hit { "plans.kept" } else { "plans.made" }, 1);
            let program = if hit {
                before.unwrap().1
            } else {
                Arc::new(column_program(inputs, &units, height, rules, land, naval, &mut undecided))
            };
            let result = program.append(&mut drawn, &mut styles, &mut flotillas, &mut undecided);
            plans.extend((0..program.stacks.len()).map(|at| Some((Arc::clone(&program), at))));
            memo.previous.insert(leader, (key, program));
            result
        } else {
            viewer_runtime::redraws::note("plans.made", 1);
            let result = column_program(inputs, &units, height, rules, land, naval, &mut undecided).append(
                &mut drawn,
                &mut styles,
                &mut flotillas,
                &mut undecided,
            );
            plans.resize(drawn.len(), None);
            result
        };
        items.push(program);
    }

    let hovered = items
        .iter()
        .zip(&groups)
        .map(|(plates, group): (&Vec<Plate>, _)| {
            levels[group.members[0]] == Level::None
                && plates.iter().any(|plate| {
                    let (kind, members) = match *plate {
                        Plate::Land(member) => (
                            if drawn[member].gun.is_some() {
                                viewer_native_gui::counter_hovers::unit_hits::Kind::RailwayGun
                            } else {
                                viewer_native_gui::counter_hovers::unit_hits::Kind::Division
                            },
                            &drawn[member].divisions,
                        ),
                        Plate::Naval(member) => (
                            viewer_native_gui::counter_hovers::unit_hits::Kind::TaskForce,
                            &flotillas[member].members,
                        ),
                    };
                    inputs
                        .hovered
                        .is_some_and(|unit| members.iter().any(|&id| kind.unit(id) == unit))
                })
        })
        .collect();
    Ok(PreparedLand {
        units,
        layout: counter_move::DrawnLayout::new(inputs.hour),
        sprites: (0..drawn.len()).map(|_| None).collect(),
        plans,
        stacks: drawn,
        styles,
        flotillas,
        positions,
        icons,
        source_kinds,
        groups,
        items,
        hovered,
        levels,
        extents: geography.extents.clone(),
        countries: Arc::clone(land.countries),
        templates: Arc::clone(land.icons),
        vision: Arc::clone(land.vision),
        order: Vec::new(),
        pointer: None,
        exits: Vec::new(),
        exits_running: Default::default(),
        time: 0.0,
        pointer_running: Default::default(),
        pointer_refused: Default::default(),
        pointer_drawn: Default::default(),
    })
}

#[derive(Clone, Default)]
pub struct ColumnProgram {
    stacks: Vec<Stack>,
    styles: Vec<StackStyle>,
    flotillas: Vec<Flotilla>,
    items: Vec<Plate>,
    unknown: BTreeMap<u64, SharingUnknown>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OrderedSource {
    province: u32,
    kind: u32,
    values: SourceValues,
    members: Vec<(Unit, u32)>,
}

#[derive(Clone)]
struct SourceValues(Option<Arc<[(Member, counter_action::Action)]>>);

impl PartialEq for SourceValues {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for SourceValues {}

impl std::fmt::Debug for SourceValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SourceValues").field(&self.0.as_ref().map(|values| values.len())).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ColumnKey {
    sources: Vec<OrderedSource>,
    coarse: bool,
    split_selected: bool,
    unknown_before: Vec<Option<SharingUnknown>>,
}

#[derive(Default)]
pub struct ColumnPlans {
    previous: BTreeMap<(u32, u32), (ColumnKey, Arc<ColumnProgram>)>,
    admitted: Vec<u64>,
}

fn record_admitted(sources: &BTreeMap<StackKey, Source<'_>>, retained: Option<&mut ColumnPlans>) {
    use viewer_runtime::view_record;
    let mut admitted: Vec<u64> = sources
        .values()
        .flat_map(|source| &source.units)
        .filter_map(|unit| match unit {
            SourceUnit::Land(unit) => Some(unit.id),
            SourceUnit::Naval(_) => None,
        })
        .collect();
    admitted.sort_unstable();
    admitted.dedup();
    let section = if viewer_runtime::redraws::apart_now() { "overlay worker" } else { "overlay" };
    view_record::set(section, "divisions_admitted", admitted.len() as u64);
    let Some(plans) = retained else { return };
    let before = std::mem::replace(&mut plans.admitted, admitted);
    let (mut entering, mut leaving) = (0, 0);
    let (mut now, mut was) = (plans.admitted.iter().peekable(), before.iter().peekable());
    loop {
        match (now.peek(), was.peek()) {
            (Some(a), Some(b)) if a == b => {
                now.next();
                was.next();
            }
            (Some(a), Some(b)) if a < b => {
                entering += 1;
                now.next();
            }
            (Some(_), Some(_)) | (None, Some(_)) => {
                leaving += 1;
                was.next();
            }
            (Some(_), None) => {
                entering += 1;
                now.next();
            }
            (None, None) => break,
        }
    }
    view_record::add(section, "divisions_entering", entering);
    view_record::add(section, "divisions_leaving", leaving);
}

impl ColumnProgram {
    fn append(
        &self,
        drawn: &mut Vec<Stack>,
        styles: &mut Vec<StackStyle>,
        flotillas: &mut Vec<Flotilla>,
        undecided: &mut BTreeMap<u64, SharingUnknown>,
    ) -> Vec<Plate> {
        for (&id, &why) in &self.unknown {
            undecided.entry(id).or_insert(why);
        }
        let (land_at, naval_at) = (drawn.len(), flotillas.len());
        drawn.extend(self.stacks.iter().cloned());
        styles.extend(self.styles.iter().cloned());
        flotillas.extend(self.flotillas.iter().cloned());
        self.items
            .iter()
            .map(|plate| match *plate {
                Plate::Land(at) => Plate::Land(land_at + at),
                Plate::Naval(at) => Plate::Naval(naval_at + at),
            })
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn column_program(
    inputs: &CounterInputs<'_>,
    units: &[&SourceUnit<'_>],
    height: f32,
    rules: JoinRules,
    land: &CounterTables<'_>,
    naval: &NavalContext<'_>,
    undecided: &mut BTreeMap<u64, SharingUnknown>,
) -> ColumnProgram {
    let mut drawn = Vec::new();
    let mut flotillas = Vec::new();
    let mut group_items = Vec::new();

    let fitted = first_fit(units.iter().copied(), |rep, unit| match (rep, unit) {
        (SourceUnit::Land(rep), SourceUnit::Land(unit)) => {
            let kind = if rep.gun.is_some() {
                viewer_native_gui::counter_hovers::unit_hits::Kind::RailwayGun
            } else {
                viewer_native_gui::counter_hovers::unit_hits::Kind::Division
            };
            if !inputs.selected_join(kind.unit(rep.id), kind.unit(unit.id), height) {
                return false;
            }
            land_join(unit, rep, land.icons, land.countries, rules, undecided)
        }
        (SourceUnit::Naval(rep), SourceUnit::Naval(unit)) => {
            if !inputs.selected_join(
                Unit::TaskForce(rep.force.id),
                Unit::TaskForce(unit.force.id),
                height,
            ) {
                return false;
            }
            naval_can_join(rep, unit, rules, land.countries, naval)
        }
        _ => false,
    });
    for item in fitted {
        if let SourceUnit::Naval(_) = item[0] {
            let members: Vec<&NavalUnit<'_>> = item
                .iter()
                .filter_map(|unit| match unit {
                    SourceUnit::Naval(unit) => Some(unit),
                    SourceUnit::Land(_) => None,
                })
                .collect();
            group_items.push(Plate::Naval(flotillas.len()));
            flotillas.push(naval.flotilla(&members, false, rules.coarse));
            continue;
        }
        let item: Vec<&LandUnit<'_>> = item
            .iter()
            .filter_map(|unit| match unit {
                SourceUnit::Land(unit) => Some(unit),
                SourceUnit::Naval(_) => None,
            })
            .collect();
        group_items.push(Plate::Land(drawn.len()));
        let mut stack = land_stack(
            &item,
            land,
            undecided,
            inputs.settings.nato,
            &|name| inputs.assets.types.declares(name),
            rules.coarse,
        );
        let kind = if stack.gun.is_some() {
            viewer_native_gui::counter_hovers::unit_hits::Kind::RailwayGun
        } else {
            viewer_native_gui::counter_hovers::unit_hits::Kind::Division
        };
        stack.selected = inputs.selected.contains(&kind.unit(stack.divisions[0]));
        stack.plate = counter_plate_color(
            inputs.settings.color_mode,
            &inputs.content.defines.tints,
            land.countries,
            type_tint::plate(stack.owner == land.vision.ours(), stack.type_tint),
            stack.logical,
        );
        drawn.push(stack);
    }
    ColumnProgram {
        styles: drawn
            .iter()
            .map(|stack| stack_style(inputs, stack))
            .collect(),
        stacks: drawn,
        flotillas,
        items: group_items,
        unknown: units
            .iter()
            .filter_map(|unit| match unit {
                SourceUnit::Land(unit) => undecided.get(&unit.id).map(|&why| (unit.id, why)),
                SourceUnit::Naval(_) => None,
            })
            .collect(),
    }
}

pub fn land_stack(
    item: &[&LandUnit<'_>],
    land: &CounterTables<'_>,
    undecided: &BTreeMap<u64, SharingUnknown>,
    nato: bool,
    declared: &dyn Fn(&str) -> bool,
    coarse: bool,
) -> Stack {
    let portraits = land.portraits;
    let rep = item[0];
    let mut stack = Stack {
        selected: false,
        disclosure: glyph_disclosure(rep, land.icons, nato, declared),
        grouping_unknown: item
            .iter()
            .find_map(|unit| undecided.get(&unit.id).copied()),
        hq: false,
        portrait: portraits.get(&rep.id).cloned(),
        owner: rep.owner,
        logical: rep.logical,
        plate: Ok([1.0; 4]),
        flag_owner: if coarse { rep.owner } else { rep.flag_owner },
        coarse,
        type_tint: rep.type_tint,
        hidden: false,
        hit_points: Vec::new(),
        organisation: Vec::new(),
        bars: true,
        status_recorded: true,
        members: Vec::new(),
        divisions: item.iter().map(|unit| unit.id).collect(),
        timing: item
            .iter()
            .map(|unit| {
                unit.gun.as_ref().map_or_else(
                    || viewer_native_gui::counter_hovers::land::Timing {
                        next: unit
                            .movement
                            .and_then(|movement| movement.eta)
                            .map(|pair| pair.0),
                        kind: unit
                            .movement
                            .filter(|movement| movement.kind_resolved)
                            .map(|movement| movement.stack_kind),
                    },
                    |gun| gun.timing,
                )
            })
            .collect(),
        template: rep.template,
        alternate_template: rep.alternate_template,
        moving: rep.movement.cloned(),
        action: counter_action::aggregate_actions(
            rep.action,
            item.iter()
                .skip(1)
                .map(|unit| (unit.sight == Sight::Full, unit.action)),
        ),
        marker: if rep.owner == land.vision.ours() {
            land.orders
                .marker(rep.orders_group, rep.parent_branch)
                .map_err(|group| {
                    format!(
                        "group_marker is refused: orders group {group}'s colour \
                         (OrdersGroupPlans) has non-finite components"
                    )
                })
        } else {
            Ok(None)
        },
        leader_en_route: Some(false),
        defending_area: false,
        gun: rep.gun.as_ref().map(|gun| GunStack {
            name: gun.name.clone(),
            moving: gun
                .position
                .as_ref()
                .is_some_and(|position| position.motion.is_some()),
            location_warning: None,
            action: guns::aggregate_actions(item.iter().map(|unit| {
                unit.gun
                    .as_ref()
                    .map(|gun| gun.action)
                    .unwrap_or(Err("railway gun member"))
            })),
        }),
        abilities: Some(Vec::new()),
    };
    for unit in item {
        if unit.orders_group != 0 {
            stack.defending_area = unit.defending_area.unwrap_or(false);
        }
        if let (Some(stack_gun), Some(gun)) = (&mut stack.gun, &unit.gun) {
            stack_gun.location_warning = gun.location_warning;
        }
        stack.hq |= unit.hq && unit.sight == Sight::Full;
        if unit.hq && unit.sight == Sight::Full {
            stack.portrait = portraits.get(&unit.id).cloned();
            stack.leader_en_route = match (stack.leader_en_route, unit.withdrawing) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            };
        }
        if unit.sight == Sight::Partial {
            stack.hidden = true;
        } else {
            update_abilities(
                &mut stack.abilities,
                unit.sight,
                unit.commander,
                rep.owner,
                land.abilities,
            );
            stack.hit_points.push(unit.hp);
            stack.organisation.push(unit.org);
            stack.bars &= unit.bars;
            stack.status_recorded &= unit.status_recorded;
            stack.members.push(unit.status);
        }
    }
    stack
}

pub fn stack_style(inputs: &CounterInputs<'_>, stack: &Stack) -> StackStyle {
    let count = if stack.hidden {
        UNKNOWN_COUNT.to_owned()
    } else {
        stack.members.len().to_string()
    };
    match &stack.gun {
        Some(gun) => StackStyle {
            organisation: 0.0,
            strength: counter_status::stack_fill(&stack.hit_points),
            glyphs: counter_status::Glyphs {
                supply: counter_status::stack_supply_frame(
                    &stack.members,
                    inputs.content.defines.supply,
                ),
                no_orders: gun.location_warning,
                ..Default::default()
            },
            count,
        },
        None => StackStyle {
            organisation: counter_status::stack_fill(&stack.organisation),
            strength: counter_status::stack_fill(&stack.hit_points),
            glyphs: if stack.status_recorded {
                counter_status::glyphs(&stack.members, Some(inputs.content.defines.supply))
            } else {
                counter_status::Glyphs {
                    supply: counter_status::stack_supply_frame(
                        &stack.members,
                        inputs.content.defines.supply,
                    ),
                    training: counter_status::stack_training(&stack.members),
                    ..Default::default()
                }
            },
            count,
        },
    }
}

pub fn hq_portraits(
    frame: &DecodedFrame,
    native: &NativeInputs,
) -> io::Result<BTreeMap<u64, counter_order::Portrait>> {
    let groups = rows::<tables::OrdersGroups>(frame)?;
    let leader_rows = rows::<tables::LeaderPortraits>(frame)?;
    if !leader_rows.is_complete() {
        return Ok(BTreeMap::new());
    }
    let character_rows = rows::<tables::CharacterPortraits>(frame)?;
    let unit_leaders = rows::<tables::UnitLeaders>(frame)?;
    let leaders: BTreeMap<u64, _> = leader_rows.rows().iter().map(|r| (r.leader, r)).collect();
    let character_complete = character_rows.is_complete();
    let characters: BTreeMap<u32, _> = character_rows
        .rows()
        .iter()
        .map(|r| (r.character, r))
        .collect();
    let categories: BTreeMap<u64, i32> = unit_leaders
        .rows()
        .iter()
        .map(|row| {
            (
                (u64::from(row.id) << 32) | u64::from(row.reference_type),
                row.category,
            )
        })
        .collect();
    Ok(groups
        .rows()
        .iter()
        .filter(|group| group.hq_provider != 0 && group.leader != 0)
        .map(|group| {
            let portrait = match leaders.get(&group.leader) {
                None => counter_order::Portrait::Unrecorded("leader row"),
                Some(leader) if leader.character != 0 && !character_complete => {
                    counter_order::Portrait::Unrecorded("character census")
                }
                Some(leader) => counter_order::hq_portrait(
                    Some(leader),
                    categories.get(&group.leader).copied(),
                    characters.get(&((leader.character >> 32) as u32)).copied(),
                    native.leader_tag(group.leader),
                ),
            };
            (group.hq_provider, portrait)
        })
        .collect())
}

pub fn draw_land_counter(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    icon: u32,
    origin: [f32; 2],
    land: &PreparedLand,
    member: usize,
    priority: i8,
) -> io::Result<hovers::Presented> {
    let first = batch.quads.len();
    let child_refusals = draw_land_children(inputs, batch, origin, land, member)?;
    let mut controls = hovers::presented(
        inputs,
        &land.stacks[member],
        land.stacks[member].coarse,
        origin,
        &batch.quads[first..],
    );
    controls.refused.extend(child_refusals);
    rank_children(batch, first, priority, icon)?;
    Ok(controls)
}

#[allow(clippy::too_many_arguments)]
fn pointed_plate(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    land: &PreparedLand,
    member: usize,
    window: &str,
    plate: &Child,
    origin: [f32; 2],
    binding: PlateBinding<'_>,
) -> bool {
    let Some((down, since, unit)) = land.pointed(member) else {
        return false;
    };
    batch.at_layer(u64::from(plate.order));
    match draw_pointed_plate(inputs, batch, window, origin, binding, down, since, land.time) {
        Ok(changing) => {
            if changing {
                land.pointer_running.set(true);
                if down.is_none() {
                    land.exits_running.borrow_mut().push(unit);
                }
            }
            if down.is_some() {
                land.pointer_drawn.set(true);
            }
            true
        }
        Err(why) => {
            land.pointer_refused.borrow_mut().push(why);
            false
        }
    }
}

fn draw_land_children(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    origin: [f32; 2],
    land: &PreparedLand,
    member: usize,
) -> io::Result<Vec<String>> {
    let stack = &land.stacks[member];
    if let (Some(gun), Ok(layout)) = (&stack.gun, &inputs.assets.layouts.gun) {
        return Ok(draw_gun_children(
            inputs, batch, origin, land, member, gun, layout,
        ));
    }
    let style = &land.styles[member];
    let (countries, icons_of) = (&land.countries, &land.templates);
    let Ok(plate) = stack.plate else {
        return Ok(Vec::new());
    };
    let layout = inputs
        .assets
        .layouts
        .land
        .as_ref()
        .expect("land plate layout was admitted");
    let assets = inputs.assets;
    if stack.coarse {
        let plate_drawn = pointed_plate(
            inputs,
            batch,
            land,
            member,
            LAND_WINDOW,
            &layout.plate,
            origin,
            PlateBinding {
                sprite: None,
                frame: Some(COARSE_PLATE_FRAME as i32 + 1),
                tint: plate,
            },
        );
        draw_coarse_counter(
            inputs,
            batch,
            origin,
            countries,
            CoarseCounter {
                plate: &layout.plate,
                large_flag: &layout.large_flag,
                count_box: &layout.count,
                tint: plate,
                owner: stack.flag_owner,
                count: &style.count,
                keys: (
                    layout.plate.order,
                    layout.large_flag.order,
                    layout.count.order,
                ),
                plate_drawn,
            },
        )?;
        let mut refused = Vec::new();
        if !stack.hidden {
            match style.glyphs.supply {
                counter_status::SupplyFrame::Frame(frame) => {
                    if let Some(sheet) = assets.sheet(&layout.supply.sprite) {
                        batch.at_layer(u64::from(layout.supply.order));
                        draw_child(batch, sheet, &layout.supply, origin, frame, [1.0; 4]);
                    }
                }
                counter_status::SupplyFrame::Uncertain => {
                    refused.push(
                        uncertain_supply_gap(&stack.members, inputs.content.defines.supply)
                            .to_owned(),
                    );
                }
                counter_status::SupplyFrame::Hidden => {}
            }
        }
        if stack.selected
            && let Some(sheet) = assets.sheet(&layout.selected.sprite)
        {
            batch.at_layer(u64::from(layout.selected.order));
            draw_child(
                batch,
                sheet,
                &layout.selected,
                origin,
                layout.selected.frame_index(0),
                [1.0; 4],
            );
        }
        return Ok(refused);
    }
    let mut refused = Vec::new();
    let hq_sprite = stack.hq.then_some("GFX_onmap_unit_counter_empty");
    let plate_drawn = pointed_plate(
        inputs,
        batch,
        land,
        member,
        LAND_WINDOW,
        &layout.plate,
        origin,
        PlateBinding {
            sprite: hq_sprite,
            frame: None,
            tint: plate,
        },
    );
    batch.at_layer(u64::from(layout.plate.order));
    if !plate_drawn
        && let Some(sheet) = assets.sheet(hq_sprite.unwrap_or(&layout.plate.sprite))
    {
        draw_child(
            batch,
            sheet,
            &layout.plate,
            origin,
            layout.plate.frame_index(0),
            plate,
        );
    }
    if stack.selected
        && !stack.members.is_empty()
        && let Some(sheet) = assets.sheet(&layout.selected.sprite)
    {
        batch.at_layer(u64::from(layout.selected.order));
        draw_child(
            batch,
            sheet,
            &layout.selected,
            origin,
            layout.selected.frame_index(0),
            [1.0; 4],
        );
    }
    match &stack.marker {
        Ok(Some(color)) => {
            if let Some(sheet) = assets.sheet(&layout.group_marker.sprite) {
                batch.at_layer(u64::from(layout.group_marker.order));
                draw_child(batch, sheet, &layout.group_marker, origin, 0, *color);
            }
        }
        Ok(None) => {}
        Err(reason) => refused.push(reason.clone()),
    }
    if let Some(content) = &assets.land_action
        && let Ok(Some(frame)) = content.frame(
            stack.action,
            stack.owner == land.vision.ours(),
            stack.coarse,
        )
        && let Ok(sheet) = &content.sheet
    {
        batch.at_layer(u64::from(layout.action.order));
        draw_child(batch, sheet, &layout.action, origin, frame, [1.0; 4]);
    }
    batch.at_layer(u64::from(layout.overlay.order));
    if let Some(sheet) = assets.sheet(&layout.overlay.sprite) {
        draw_child(batch, sheet, &layout.overlay, origin, 0, [1.0; 4]);
    }
    if !stack.members.is_empty() && stack.bars {
        for (child, fill) in [
            (&layout.bar_org, style.organisation),
            (&layout.bar_str, style.strength),
        ] {
            batch.at_layer(u64::from(child.order));
            if let Some(sheet) = assets.sheet(&child.sprite) {
                draw_bar(batch, sheet, child, origin, fill);
            }
        }
    }
    batch.at_layer(u64::from(layout.flag.order));
    draw_flag(
        &assets.flags,
        batch,
        &layout.flag,
        origin,
        countries,
        stack.flag_owner,
    );
    batch.at_layer(u64::from(layout.type_icon.order));
    let type_refusal = draw_type_icon(
        &assets.types,
        batch,
        &layout.type_icon,
        origin,
        if stack.hidden {
            Some(templates::no_intel_icon(inputs.settings.nato))
        } else {
            stack.disclosure.ok().and_then(|shared| {
                icons_of.icon(
                    stack.template,
                    stack.alternate_template,
                    shared,
                    inputs.settings.nato,
                    &|name| assets.types.declares(name),
                )
            })
        }
        .as_ref(),
        stack
            .type_tint
            .map(|side| inputs.content.defines.type_colors.of(side)),
    );
    if !stack.hidden {
        if style.glyphs.supply == counter_status::SupplyFrame::Uncertain {
            refused.push(
                uncertain_supply_gap(&stack.members, inputs.content.defines.supply).to_owned(),
            );
        }
        draw_glyphs(
            inputs,
            batch,
            origin,
            &style.glyphs,
            GlyphStack {
                hq: stack.hq,
                defending_area: stack.defending_area,
            },
        );
    }
    if stack.hq {
        let en_route = stack.leader_en_route == Some(true);
        if let Some(portrait) = stack
            .portrait
            .as_ref()
            .and_then(|portrait| assets.portrait(portrait))
        {
            batch.at_layer(u64::from(layout.portrait.order));
            draw_child(
                batch,
                &portrait,
                &layout.portrait,
                origin,
                0,
                if en_route { DISABLED } else { [1.0; 4] },
            );
        }
        let badge = if en_route {
            &layout.leader_enroute
        } else {
            &layout.hq_badge
        };
        batch.at_layer(u64::from(badge.order));
        if let Some(sheet) = assets.sheet(&badge.sprite) {
            draw_child(batch, sheet, badge, origin, 0, [1.0; 4]);
        }
    }
    batch.at_layer(u64::from(layout.count.order));
    if let Some(font) = &inputs.fonts.counter {
        draw_text(
            batch,
            font,
            &layout.count,
            origin,
            &style.count,
            font.color(),
        );
    }
    if let (Some(tokens), Some(grid)) = (&stack.abilities, &assets.layouts.abilities) {
        batch.at_layer(u64::from(grid.order));
        for (position, token) in tokens.iter().enumerate() {
            if let Some(sheet) = assets.ability_sheet(&token.token) {
                let rect = ability_rect(sheet, grid, origin, position);
                batch.scaled(sheet, 0, [rect[0], rect[1]], [rect[2], rect[3]], [1.0; 4]);
            }
        }
    }
    refused.extend(type_refusal);
    Ok(refused)
}

pub fn ability_rect(
    sheet: &SpriteSheet,
    grid: &AbilityGrid,
    origin: [f32; 2],
    position: usize,
) -> [f32; 4] {
    let half = |value: f32| (value * 0.5).floor();
    let size = sheet.frame_size().map(|side| side * grid.scale);
    let mut at = [
        origin[0] + grid.at[0] + position as f32 * grid.slot[0] + grid.icon_at[0],
        origin[1] + grid.at[1] + grid.icon_at[1],
    ];
    if grid.from_centre {
        at[0] += half(grid.item_size[0]);
        at[1] += half(grid.item_size[1]);
    }
    if grid.centred {
        at[0] -= half(size[0]);
        at[1] -= half(size[1]);
    }
    [at[0], at[1], size[0], size[1]]
}

fn abilities_on_screen(inputs: &CounterInputs<'_>, stack: &Stack, origin: [f32; 2]) -> bool {
    let (Some(tokens), Some(grid)) = (&stack.abilities, &inputs.assets.layouts.abilities) else {
        return false;
    };
    tokens.iter().enumerate().any(|(position, token)| {
        inputs
            .assets
            .ability_sheet(&token.token)
            .is_some_and(|sheet| {
                let rect = ability_rect(sheet, grid, origin, position);
                admitted([rect[0], rect[1]], [rect[2], rect[3]], inputs.extent())
            })
    })
}

fn plate_size(inputs: &CounterInputs<'_>, stack: &Stack) -> Result<[f32; 2], String> {
    if stack.gun.is_some() {
        inputs
            .assets
            .layouts
            .gun
            .as_ref()
            .map(|gun| gun.size)
            .map_err(Clone::clone)
    } else {
        inputs
            .assets
            .layouts
            .land
            .as_ref()
            .map(|land| land.size)
            .map_err(Clone::clone)
    }
}

pub fn column_origin(anchor: [f32; 2], sizes: &[[f32; 2]]) -> Option<[i32; 2]> {
    let first = sizes.first()?;
    let total = sizes
        .iter()
        .fold(0i32, |total, size| total.wrapping_add(size[1] as i32));
    Some([
        (anchor[0] - first[0] * 0.5).round() as i32,
        (anchor[1] - total as f32 * 0.5).round() as i32,
    ])
}

fn draw_gun_children(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    origin: [f32; 2],
    land: &PreparedLand,
    member: usize,
    gun: &GunStack,
    layout: &GunCounter,
) -> Vec<String> {
    let stack = &land.stacks[member];
    let style = &land.styles[member];
    let assets = inputs.assets;
    let Ok(plate) = stack.plate else {
        return Vec::new();
    };
    let plate_drawn = pointed_plate(
        inputs,
        batch,
        land,
        member,
        GUN_WINDOW,
        &layout.plate,
        origin,
        PlateBinding {
            sprite: None,
            frame: stack
                .coarse
                .then_some(COARSE_PLATE_FRAME as i32 + 1),
            tint: plate,
        },
    );
    let mut draw = |child: &Child, frame: u32, tint: [f32; 4]| {
        batch.at_layer(u64::from(child.order));
        if let Some(sheet) = assets.sheet(&child.sprite) {
            draw_child(batch, sheet, child, origin, frame, tint);
        }
    };
    if let counter_status::SupplyFrame::Frame(frame) = style.glyphs.supply {
        draw(&layout.supply, frame, [1.0; 4]);
    }
    if stack.coarse {
        let _ = draw_coarse_counter(
            inputs,
            batch,
            origin,
            &land.countries,
            CoarseCounter {
                plate: &layout.plate,
                large_flag: &layout.large_flag,
                count_box: &layout.count,
                tint: plate,
                owner: stack.flag_owner,
                count: &style.count,
                keys: (
                    layout.plate.order,
                    layout.large_flag.order,
                    layout.count.order,
                ),
                plate_drawn,
            },
        );
        if stack.selected
            && let Some(sheet) = assets.sheet(&layout.selected.sprite)
        {
            batch.at_layer(u64::from(layout.selected.order));
            draw_child(
                batch,
                sheet,
                &layout.selected,
                origin,
                layout.selected.frame_index(0),
                [1.0; 4],
            );
        }
        return Vec::new();
    }
    let mut refused = Vec::new();
    if !plate_drawn {
        draw(&layout.plate, layout.plate.frame_index(0), plate);
    }
    if stack.selected && !stack.members.is_empty() {
        draw(&layout.selected, layout.selected.frame_index(0), [1.0; 4]);
    }
    match &stack.marker {
        Ok(Some(color)) => draw(&layout.group_marker, 0, *color),
        Ok(None) => {}
        Err(reason) => refused.push(reason.clone()),
    }
    draw(&layout.overlay, 0, [1.0; 4]);
    if style.glyphs.no_orders == Some(true) {
        draw(&layout.no_orders, 0, [1.0; 4]);
    }
    if !stack.members.is_empty() {
        batch.at_layer(u64::from(layout.bar_str.order));
        if let Some(sheet) = assets.sheet(&layout.bar_str.sprite) {
            draw_bar(batch, sheet, &layout.bar_str, origin, style.strength);
        }
    }
    batch.at_layer(u64::from(layout.flag.order));
    draw_flag(
        &assets.flags,
        batch,
        &layout.flag,
        origin,
        &land.countries,
        stack.flag_owner,
    );
    batch.at_layer(u64::from(layout.type_icon.order));
    let type_refusal = draw_type_icon(
        &assets.types,
        batch,
        &layout.type_icon,
        origin,
        Some(&if stack.hidden {
            templates::no_intel_icon(inputs.settings.nato)
        } else {
            templates::type_icon(&gun.name, -1, inputs.settings.nato, &|name| {
                assets.types.declares(name)
            })
        }),
        stack
            .type_tint
            .map(|side| inputs.content.defines.type_colors.of(side)),
    );
    batch.at_layer(u64::from(layout.count.order));
    if let Some(font) = &inputs.fonts.counter {
        draw_text(
            batch,
            font,
            &layout.count,
            origin,
            &style.count,
            font.color(),
        );
    }
    if let Some(content) = &assets.gun_action
        && let Ok(Some(frame)) =
            content.frame(gun.action, stack.owner == land.vision.ours(), stack.coarse)
        && let Ok(sheet) = &content.sheet
    {
        batch.at_layer(u64::from(layout.action.order));
        draw_child(batch, sheet, &layout.action, origin, frame, [1.0; 4]);
    }
    refused.extend(type_refusal);
    refused
}

fn place_in(
    inputs: &CounterInputs<'_>,
    unit: Unit,
    province: u32,
    position: &counter_move::Position,
    world: Option<&counter_move::RouteWorld<'_>>,
) -> Option<[f32; 3]> {
    let anchor = |id, kind| inputs.content.anchor(id, kind);
    if let (Some(route), Some(world)) = (position.movement.as_deref(), world)
        && let Some(there) = route.place_in(province, world, &anchor)
    {
        return Some(there);
    }
    let class = match unit {
        Unit::Division(_) => viewer_runtime::unit_class::CounterClass::Division,
        Unit::RailwayGun(_) => viewer_runtime::unit_class::CounterClass::RailwayGun,
        Unit::TaskForce(_) => viewer_runtime::unit_class::CounterClass::TaskForce,
    };
    anchor(province, class.standstill(is_land(inputs, province)))
}

pub fn stack_position(
    inputs: &CounterInputs<'_>,
    province: u32,
    moving: Option<&Movement>,
) -> Option<counter_move::Position> {
    let anchor = |id, kind| inputs.content.anchor(id, kind);
    match moving {
        Some(moving) => moving.position(province, inputs.route_world().as_ref(), &anchor),
        None => anchor(province, STANDSTILL).map(counter_move::Position::still),
    }
}

pub const ATTRITION_TINT: [f32; 4] = [1.0, 0.3, 0.3, 1.0];

#[derive(Clone, Copy, Debug)]
pub struct GlyphStack {
    pub hq: bool,
    pub defending_area: bool,
}

fn uncertain_supply_gap(
    members: &[counter_status::Member],
    thresholds: counter_status::SupplyThresholds,
) -> &'static str {
    let mut straddles = false;
    for member in members.iter().rev() {
        match member.naval_transfer {
            Some(true) => continue,
            Some(false) | None => {
                if member.supply_ratio == counter_status::SupplyRatio::Missing {
                    return "low_supply is undecided: the stack's supply ratio (DivisionStatus) is not recorded";
                }
                if counter_status::supply_frame_for(member.supply_ratio, thresholds)
                    == counter_status::SupplyFrame::Uncertain
                {
                    straddles = true;
                }
                if member.naval_transfer == Some(false) {
                    break;
                }
            }
        }
    }
    if straddles {
        "low_supply is undecided: the stack's supply ratio (DivisionStatus) is recorded only at its display step and straddles a supply threshold"
    } else {
        "low_supply is undecided: whether a stack member is in naval transfer (DivisionMovement) is not recorded"
    }
}

pub fn draw_glyphs(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    origin: [f32; 2],
    glyphs: &counter_status::Glyphs,
    stack: GlyphStack,
) {
    let GlyphStack { hq, defending_area } = stack;
    let layout = inputs
        .assets
        .layouts
        .land
        .as_ref()
        .expect("land plate layout was admitted");
    let mut draw = |element: &Child, frame: u32, tint: [f32; 4]| {
        batch.at_layer(u64::from(element.order));
        if let Some(sheet) = inputs.assets.sheet(&element.sprite) {
            draw_child(batch, sheet, element, origin, frame, tint);
        }
    };
    if glyphs.attrition {
        draw(&layout.attrition, 0, ATTRITION_TINT);
    }
    let mut draw_shifted = |element: &Child| {
        let mut element = element.clone();
        if hq {
            element.at[0] += layout.hq_offset[0];
        }
        draw(&element, 0, [1.0; 4]);
    };
    if glyphs.no_orders == Some(true) {
        draw_shifted(&layout.no_orders);
    }
    if defending_area {
        draw_shifted(&layout.defending_area);
    }
    if glyphs.invalid_orders == Some(true) {
        draw_shifted(&layout.invalid_orders);
    }
    if let counter_status::SupplyFrame::Frame(frame) = glyphs.supply {
        draw(&layout.supply, frame, [1.0; 4]);
    }
    if let Some(frame) = glyphs.digin {
        draw(&layout.digin, frame, [1.0; 4]);
    }
    if glyphs.training == Some(true) {
        draw(&layout.training, 0, [1.0; 4]);
    }
}

pub fn draw_type_icon(
    types: &templates::TypeAtlas,
    batch: &mut Batch,
    child: &Child,
    origin: [f32; 2],
    icon: Option<&templates::TypeIcon>,
    tint: Option<[f32; 4]>,
) -> Option<String> {
    let icon = icon?;
    if !types.declares(&icon.sprite) {
        return Some(format!(
            "type icon refused: sprite {} is undeclared; the game keeps the pooled counter's \
             previous drawable, which the recording does not hold",
            icon.sprite
        ));
    }
    let tint = tint?;
    let cell = types.cell(icon)?;
    let size = types.frame_size(cell);
    batch.cell(
        types.texture,
        [origin[0] + child.at[0], origin[1] + child.at[1]],
        [size[0] * child.scale, size[1] * child.scale],
        types.uv(cell),
        tint,
    );
    None
}

#[cfg(test)]
mod tests {
    use super::{Arc, CounterReport, DivisionStatuses, IconInputs, LAND_WINDOW, NAVY_WINDOW, Plate, STACK_WINDOW, column_holder, gui, native, native_gui, refuse_column, test_frame, test_put};

    #[test]
    fn attrition_comes_from_the_recorded_value_against_its_threshold() {
        let mut frame = test_frame(3);
        let row =
            |division, attrition_value, attrition_threshold| replay_data::rows::DivisionStatusRow {
                division,
                attrition_value,
                attrition_threshold,
                ..Default::default()
            };
        test_put(
            &mut frame,
            replay_data::rows::TableRows::DivisionStatus(vec![
                row(1, 1, 0),
                row(2, 5_000, 5_000),
                row(3, 0, 0),
                row(4, -2, -3),
            ]),
        );
        let mut panel = replay_data::rows::DivisionPanelStatsRow {
            division: 1,
            ..Default::default()
        };
        panel.panel_stats[0] = Some(1_000_000);
        panel.panel_stats[1] = Some(2_000_000);
        test_put(
            &mut frame,
            replay_data::rows::TableRows::DivisionPanelStats(vec![panel]),
        );
        let statuses = DivisionStatuses::read(&frame).unwrap();
        let attrition: Vec<_> = (1..=5)
            .map(|division| statuses.get(division).attrition)
            .collect();
        assert_eq!(attrition, [true, false, false, true, false]);
        assert!(statuses.recorded(1) && !statuses.recorded(5));
    }

    #[test]
    fn an_unrecorded_status_table_records_no_maxima() {
        let mut frame = test_frame(3);
        *frame.tables[replay_data::rows::TableId::DivisionStatus.index()] =
            replay_frame::frame::TableView::NotRecorded {
                mode: replay_data::scope::CaptureMode::Map,
            };
        let statuses = DivisionStatuses::read(&frame).unwrap();
        assert!(!statuses.recorded(1));
        assert!(!statuses.get(1).attrition && statuses.get(1).has_order.is_none());
    }

    #[test]
    fn icon_inputs_looked_up_by_key_equal_the_whole_read_for_every_division() {
        let mut frame = test_frame(3);
        let row = |division, separate_counter, alternate_template| replay_data::rows::ArmyIconInputRow { division, separate_counter, alternate_template };
        test_put(
            &mut frame,
            replay_data::rows::TableRows::ArmyIconInputs(vec![row(3, 1, 0), row(4, 0, 77), row(10, 1, 12), row(11, 0, 0)]),
        );
        let frame = Arc::new(frame);
        let whole = IconInputs::read(&frame).unwrap();
        let lookups = super::super::Lookups::default();
        let keyed = IconInputs::read_by_key(&frame, lookups.clone()).unwrap();
        for division in 0..=12 {
            assert_eq!(keyed.get(division), whole.get(division), "division {division}");
        }
        assert_eq!(lookups.counts(), (13, 0));
        assert_eq!(whole.get(4), Some((0, 77)));
    }

    #[test]
    fn statuses_looked_up_by_key_equal_the_whole_read_for_every_division() {
        let mut frame = test_frame(3);
        let status = |division, entrenchment, has_order| replay_data::rows::DivisionStatusRow {
            division,
            entrenchment,
            has_order,
            attrition_value: if division % 2 == 1 { 7 } else { 0 },
            ..Default::default()
        };
        test_put(
            &mut frame,
            replay_data::rows::TableRows::DivisionStatus(vec![status(2, 10, 1), status(5, 0, 0), status(9, 3, 1), status(40, 1, 0)]),
        );
        let panel = |division, hit_points| {
            let mut panel = replay_data::rows::DivisionPanelStatsRow { division, ..Default::default() };
            panel.panel_stats[0] = hit_points;
            panel.panel_stats[1] = Some(3_000_000);
            panel
        };
        test_put(
            &mut frame,
            replay_data::rows::TableRows::DivisionPanelStats(vec![panel(1, Some(1)), panel(2, Some(2_000_000)), panel(9, None)]),
        );
        let movement = |division, naval_transfer| replay_data::rows::DivisionMovementRow { division, naval_transfer, ..Default::default() };
        test_put(
            &mut frame,
            replay_data::rows::TableRows::DivisionMovement(vec![movement(5, 1), movement(9, 0), movement(41, 1)]),
        );
        let frame = Arc::new(frame);
        let whole = DivisionStatuses::read(&frame).unwrap();
        let lookups = super::super::Lookups::default();
        let keyed = DivisionStatuses::read_by_key(&frame, lookups.clone()).unwrap();
        for division in 0..=42 {
            assert_eq!(keyed.get(division), whole.get(division), "division {division}");
            assert_eq!(keyed.recorded(division), whole.recorded(division), "division {division}");
        }
        assert_eq!(lookups.counts(), (43, 43));

        let mut unrecorded = test_frame(3);
        *unrecorded.tables[replay_data::rows::TableId::DivisionStatus.index()] =
            replay_frame::frame::TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Map };
        let unrecorded = Arc::new(unrecorded);
        let whole = DivisionStatuses::read(&unrecorded).unwrap();
        let keyed = DivisionStatuses::read_by_key(&unrecorded, Default::default()).unwrap();
        for division in 0..=3 {
            assert_eq!(keyed.get(division), whole.get(division), "division {division}");
            assert_eq!(keyed.recorded(division), whole.recorded(division), "division {division}");
        }
    }

    #[test]
    fn columns_keep_the_native_grid_origin_and_viewport_item_offsets() {
        content_reader_script::test_game::enter();
        let document = native::read(
            br#"guiTypes={
                containerWindowType={name=unit_mapicon size={width=76 height=31}
                    gridBoxType={name=units_box position={x=0 y=-31}
                        slotsize={width=76 height=1} format=UPPER_LEFT}}
                containerWindowType={name=unit_counter size={width=76 height=31}
                    position={x=2 y=3}}
                containerWindowType={name=unit_counter_navy size={width=80 height=20}
                    position={x=10% y=5}}
            }"#,
        );
        let root = document.template(STACK_WINDOW).unwrap();
        let grid = root.find(native::Kind::Grid, "units_box").unwrap();
        let viewport = [1000, 600];
        let (holder, notes) = column_holder(root, viewport, &gui::fixture()).unwrap();
        assert!(notes.is_empty());
        let layout = native_gui::finalize_grid_positions(
            grid,
            holder.content,
            viewport,
            [
                (document.template(LAND_WINDOW).unwrap(), [0, 31]),
                (document.template(NAVY_WINDOW).unwrap(), [0, 51]),
            ],
            holder.position,
        )
        .unwrap();
        assert_eq!(holder.origin, [0.0; 2]);
        assert_eq!(layout.origin, [0, -31]);
        assert_eq!(layout.items, [[2, 34], [100, 56]]);
        assert_eq!(layout.places(), [[2, 3], [100, 25]]);
    }

    #[test]
    fn unsupported_column_parent_terms_refuse_every_affected_plate_by_name() {
        content_reader_script::test_game::enter();
        let resources = gui::fixture();
        for fields in [
            "show_animation_type=linear",
            "fade_type=linear",
            "scale=2",
            "clipping=yes",
            "iconType={name=extra spriteType=GFX_even}",
        ] {
            let source = format!(
                "guiTypes={{containerWindowType={{name=unit_mapicon size={{width=76 height=31}}
                    {fields} gridBoxType={{name=units_box slotsize={{width=76 height=1}}}}}}}}"
            );
            let document = native::read(source.as_bytes());
            let why = column_holder(
                document.template(STACK_WINDOW).unwrap(),
                [1000, 600],
                &resources,
            )
            .unwrap_err();
            assert!(why.contains(STACK_WINDOW), "{why}");
            let mut report = CounterReport::default();
            refuse_column(
                [Plate::Land(0), Plate::Land(1)].into_iter(),
                &[],
                &mut report,
                &why,
            );
            assert_eq!(report.land_counter_gaps[why.as_str()], 2);
        }
    }
}

#[cfg(test)]
mod cached_division_tests {
    use super::{Arc, BTreeSet, CounterTables, DivisionCounters, STANDSTILL, Vision, division_counters, test_frame, test_put};
    use replay_data::rows::{DivisionRow, OrdersGroupRow, TableRows};

    #[test]
    fn cached_divisions_keep_hour_operands_and_do_not_admit_provinces() {
        let mut frame = test_frame(1);
        test_put(
            &mut frame,
            TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: 7,
                leader: 91,
                hq_provider: 2,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Divisions(
                (1..=5000)
                    .map(|id| DivisionRow {
                        id,
                        province: if id == 1 { 11 } else { 22 },
                        roster_owner: 1,
                        logical_owner: 1,
                        orders_group: if id == 2 { 7 } else { 0 },
                        template: 83,
                        hit_points: 55,
                        organisation: 66,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let vision = Arc::new(Vision::sharing(1, []));
        let tables = CounterTables {
            type_tints: &Default::default(),
            countries: &Arc::default(),
            icons: &Arc::default(),
            status: &Default::default(),
            movement: &Default::default(),
            actions: &Default::default(),
            vision: &vision,
            order: None,
            native: &Default::default(),
            orders: &Default::default(),
            guns: &Default::default(),
            abilities: &Default::default(),
            task_forces: &Default::default(),
            hq_providers: &BTreeSet::from([2]),
            portraits: &Default::default(),
            icon_inputs: &Default::default(),
        };
        let cached = division_counters(&frame, &Some, &tables).unwrap();
        assert_eq!(cached[&(11, STANDSTILL)].len(), 1);
        let units = &cached[&(22, STANDSTILL)];
        assert_eq!(units.len(), 4999);
        assert_eq!(units[0].id, 2);
        assert_eq!(units[0].commander, Some(Some(91)));
        assert_eq!(units[1].commander, Some(None));
        assert!(units[0].hq && units[0].separate);
        assert!(units.iter().all(|unit| unit.movement.is_none()));
        let fresh = division_counters(&frame, &Some, &tables).unwrap();
        let signature = |sources: &DivisionCounters| {
            sources
                .iter()
                .map(|(&key, units)| {
                    (
                        key,
                        units
                            .iter()
                            .map(|unit| (unit.id, unit.ordinal, unit.commander, unit.hp, unit.org))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(signature(&cached), signature(&fresh));
    }
}

impl<'a> CounterTables<'a> {
    pub fn naval(&self) -> viewer_counters_naval::naval::NavalLinks<'a> {
        viewer_counters_naval::naval::NavalLinks {
            countries: self.countries,
            vision: self.vision,
            order: self.order,
            task_forces: self.task_forces,
        }
    }
}
