//! Unit counters: division, railway-gun and task-force sources, first-fit plates and their
//! drawing.

use super::*;
use replay_data::{rows::ActiveAbilityRow, rows::RailwayGunCounterInputRow};
use viewer_foundation::map::gui_resources::count_noted;
use viewer_foundation::map::gui_resources::refused_cause;
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::native_gui::{self, lifecycle::Instance};
use viewer_foundation::map::railway_gun_counter as guns;
use viewer_foundation::map::sprites::DISABLED;
use viewer_foundation::map::sprites::hovered_layer;
use viewer_foundation::map::templates::counter_custom_icon;
#[cfg(test)]
use viewer_foundation::map::test_support::gui;
#[cfg(test)]
use viewer_foundation::map::test_support::test_frame;
#[cfg(test)]
use viewer_foundation::map::test_support::test_put;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::movement_timing::Report;
use viewer_foundation::table_access::TokenKeys;
use viewer_foundation::table_access::TokenNames;
use viewer_foundation::unit_handle::Unit;

pub(super) type ColumnIdentity = (Level, u32, u32, u32, u32);

/// The retained column's graphical parent, independently of its items' viewport parent.
#[derive(Clone, Debug)]
pub(super) struct ColumnPlacement {
    pub(super) hovered: bool,
    /// The column's content priority at its grouping level.
    pub(super) priority: i8,
    /// Where the column stands, for a free eye.
    pub(super) anchor: Option<IconAnchor>,
    pub(super) identity: ColumnIdentity,
    pub(super) count: usize,
    pub(super) screen: [i32; 2],
    pub(super) holder: native_gui::Holder,
    pub(super) grid: [i32; 2],
}

#[derive(Debug)]
pub(super) struct AtSeaPlate {
    pub(super) member: usize,
    pub(super) ordinal: u32,
    pub(super) column: Arc<ColumnPlacement>,
    pub(super) local: [i32; 2],
}

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
            // Columns need retained transitions alongside the static land batch.
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
        // The game carries these terms; the land batch cannot apply them.
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

#[derive(Clone)]
pub(super) struct StackStyle {
    pub(super) organisation: f32,
    pub(super) strength: f32,
    pub(super) glyphs: counter_status::Glyphs,
    pub(super) count: String,
}

pub(super) struct PreparedLand {
    pub(super) stacks: Vec<Stack>,
    pub(super) styles: Vec<StackStyle>,
    pub(super) sprites: Vec<Option<hovers::Sprite>>,
    /// The naval items among the column plates, drawn with the naval window.
    pub(super) flotillas: Vec<Flotilla>,
    pub(super) positions: Vec<counter_move::Position>,
    /// Each position's unit, whose presentation history moves its counter.
    pub(super) units: Vec<Unit>,
    pub(super) icons: Vec<counter_groups::Icon>,
    /// Each icon's province visual bucket, distinguishing same-province sources.
    pub(super) source_kinds: Vec<u32>,
    pub(super) groups: Vec<counter_groups::Group>,
    pub(super) items: Vec<Vec<Plate>>,
    pub(super) hovered: Vec<bool>,
    /// Each icon's grouping level.
    pub(super) levels: Vec<Level>,
    pub(super) extents: Extents,
    pub(super) countries: Arc<Countries>,
    pub(super) templates: Arc<TemplateIcons>,
    pub(super) vision: Arc<Vision>,
    /// Group indices in draw order, rebuilt by every [`PreparedLand::draw`].
    pub(super) order: Vec<usize>,
}

impl PreparedLand {
    pub(super) fn draw(
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
            for ((icon, position), &unit) in
                self.icons.iter_mut().zip(&self.positions).zip(&self.units)
            {
                icon.anchor = history
                    .sample(unit, position)
                    .map_or_else(|| position.at(), |drawn| drawn.motion.position());
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
        // Sort the visible icons on `(depth - priority) * 1e6` and re-parents
        // them last to first, so the nearest group of one priority draws on top. The columns
        // are emitted farthest first for the same result.
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
        // Land and naval plates share one column and one running icon ordinal.
        let mut ordinal = 0;
        let navy_size = inputs.navy_size();
        let mut placed_columns = Vec::with_capacity(self.groups.len());
        let roots = [LAND_WINDOW, GUN_WINDOW, NAVY_WINDOW]
            .map(|name| window(&inputs.windows.guis, name).map_err(|why| why.to_string()));
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
            let icon = inputs.icon_anchor(
                IconFamily::Columns,
                world,
                inputs.content.defines.icons_cutoff,
            );
            // Sum every item: an unknown size refuses the whole column.
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
                // Add the new item's own height first.
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
            let source = self.icons[representative];
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
                // A hidden kind or owner is still laid out and hit-tested.
                let shown = match plate {
                    Plate::Land(member) => {
                        let stack = &self.stacks[member];
                        inputs.display.land_plate(stack.gun.is_some(), stack.owner)
                    }
                    Plate::Naval(_) => true,
                };
                let (drawn, opacity) = match plate {
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
                        )?;
                        (drawn, opacity)
                    }
                    Plate::Naval(member) => {
                        // The presented frame draws it through its window's instance.
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
                    continue;
                }
                batch.quads.extend(sprite.quads.iter().map(|quad| {
                    let mut quad = quad.clone();
                    quad.layer += offset;
                    quad.anchor = icon;
                    quad.color[3] *= opacity;
                    quad
                }));
            }
        }
        Ok(placed_columns)
    }

    /// Draws one land plate at `origin` into its cached sprite and reports it; `None` when it is
    /// off screen. `priority` is its column's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn land_plate(
        &mut self,
        inputs: &CounterInputs<'_>,
        report: &mut CounterReport,
        missing_owners: &mut Vec<u32>,
        member: usize,
        origin: [f32; 2],
        size: [f32; 2],
        priority: i8,
    ) -> io::Result<Option<&Batch>> {
        let stack = &self.stacks[member];
        if !on_screen(origin, size, inputs.extent()) && !abilities_on_screen(inputs, stack, origin)
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
        // The game observes both the root and its actually presented children.
        let ours = stack.owner == self.vision.ours();
        let hover = CounterHover {
            control: viewer_foundation::map::counter_hovers::land::CounterControl::Normal,
            authored: inputs
                .windows
                .guis
                .template(if stack.gun.is_some() {
                    GUN_WINDOW
                } else {
                    LAND_WINDOW
                })
                .and_then(|root| {
                    viewer_foundation::map::counter_hovers::gui_authored::Authored::read(
                        root, false,
                    )
                }),
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
        let click = viewer_foundation::map::counter_hovers::unit_hits::Hit {
            ours,
            port: false,
            rect: hover.rect,
            kind: if stack.gun.is_some() {
                viewer_foundation::map::counter_hovers::unit_hits::Kind::RailwayGun
            } else {
                viewer_foundation::map::counter_hovers::unit_hits::Kind::Division
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
        if self.sprites[member]
            .as_ref()
            .is_none_or(|sprite| sprite.origin != origin)
        {
            // A moved counter redraws into its previous batch's storage.
            let mut sprite = self.sprites[member]
                .take()
                .map(|sprite| sprite.batch)
                .unwrap_or_default();
            sprite.clear();
            let presented =
                draw_land_counter(inputs, &mut sprite, 0, origin, self, member, priority)?;
            self.sprites[member] = Some(hovers::Sprite {
                origin,
                batch: sprite,
                presented,
            });
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
                    .push(viewer_foundation::map::counter_hovers::unit_hits::Hit {
                        rect: *rect,
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
        Ok(self.sprites[member].as_ref().map(|sprite| &sprite.batch))
    }

    /// A naval item's counts; its moving window can reach the screen from an off-screen slot.
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

    /// The caller turns these owners into `report.missing_flags`' tags. `coarse` is the plate's.
    fn missing_flag(
        &self,
        inputs: &CounterInputs<'_>,
        report: &mut CounterReport,
        missing_owners: &mut Vec<u32>,
        owner: u32,
        coarse: bool,
    ) {
        // acquire the selected atlas before drawing.
        let atlas = if coarse {
            &inputs.assets.medium_flags
        } else {
            &inputs.assets.flags
        };
        if self.countries.tags.contains_key(&owner)
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

/// One first-fit counter item after the geographic group's unit rebuild.
#[derive(Clone)]
pub(super) struct Stack {
    pub(super) selected: bool,
    pub(super) owner: u32,
    /// The representative's logical owner, when it is a linked country.
    pub(super) logical: Option<u32>,
    /// The plate's colour under the counter colour mode ([`counter_plate_color`]), or why the
    /// hour cannot give it (the plate is then not drawn).
    pub(super) plate: Result<[f32; 4], String>,
    /// The representative's flag owner at this zoom; see [`unit_flag_owner`].
    pub(super) flag_owner: u32,
    /// Whether the plate draws coarse: its column stands beyond the content's coarse grouping
    /// distance (the camera's height, or a free eye's distance to the column).
    pub(super) coarse: bool,
    pub(super) type_tint: Option<type_tint::Side>,
    pub(super) hq: bool,
    pub(super) portrait: Option<counter_order::Portrait>,
    /// Whether any member of the stack is below the engine's 250 visibility.
    ///
    /// One member whose visibility byte is under 250 is enough to select the
    /// `type` child's sprite and count text for the whole counter. See
    /// [`crate::map::perspective`]; fog of war off never sets this flag.
    pub(super) hidden: bool,
    /// Each member's fixed5 [`counter_status::modified_strength`].
    pub(super) hit_points: Vec<i64>,
    /// Each member's fixed5 organisation [`counter_status::ratio`].
    pub(super) organisation: Vec<i64>,
    /// False when a fully visible member's maxima are not recorded: no bars are drawn.
    pub(super) bars: bool,
    /// Each member's status, in the order the stack was built.
    pub(super) members: Vec<counter_status::Member>,
    /// Pass every first-fit member to its tooltip, not only visible statuses.
    pub(super) divisions: Arc<[u64]>,
    pub(super) timing: Arc<[viewer_foundation::map::counter_hovers::land::Timing]>,
    /// First admitted representative, including partial members. Sources follow
    /// the reconstructed group order; members within a source follow recorded occupant order.
    pub(super) template: u64,
    /// The same representative's fake-preferred template, for its plain type glyph.
    pub(super) alternate_template: u64,
    /// Representative-owner disclosure for the type glyph.
    pub(super) disclosure: Result<bool, SharingUnknown>,
    /// A member whose join the hour could not decide, with the missing input.
    pub(super) grouping_unknown: Option<SharingUnknown>,
    /// The route every member of this stack is on, when it is moving.
    pub(super) moving: Option<Movement>,
    /// Representative action, compared with fully visible division members.
    pub(super) action: counter_action::Action,
    /// `group_marker`'s colour; `None` hides it (not own, no group, or unrecorded operands).
    pub(super) marker: Option<[f32; 4]>,
    /// Whether a visible HQ provider's group withdraws; `None` unrecorded.
    pub(super) leader_en_route: Option<bool>,
    /// `defending_area`: the game writes it per member without combining, so the last
    /// member that has an orders group decides.
    pub(super) defending_area: bool,
    /// A railway-gun plate and the children only it draws.
    pub(super) gun: Option<GunStack>,
    /// The ability grid's bound records in native vector order; `None` when the hour
    /// cannot say which the last fully visible commander-bearing member shows.
    pub(super) abilities: Option<Vec<AbilityIcon>>,
}

/// What a railway-gun plate draws besides the land counter's shared children.
#[derive(Clone)]
pub(super) struct GunStack {
    /// The representative gun's stat definition name: the authored key names its type icon
    /// (`GFX_unit_<key>_icon_medium_white`, [`templates::type_icon`]) with no custom icon, as
    /// Pass -1 for every kind but a division.
    pub(super) name: String,
    /// The last member's location warning; `None` when its rail operands are unknown.
    pub(super) location_warning: Option<bool>,
    /// [`guns::aggregate_actions`] over every member.
    pub(super) action: counter_action::Action,
    /// The representative is on its route this hour.
    pub(super) moving: bool,
}

/// The per-hour dictionaries a counter reads, bundled so the pass keeps one parameter.
pub(super) struct CounterTables<'a> {
    pub(super) type_tints: &'a type_tint::Index,
    pub(super) countries: &'a Arc<Countries>,
    pub(super) icons: &'a Arc<TemplateIcons>,
    pub(super) status: &'a DivisionStatuses,
    pub(super) movement: &'a Movements,
    pub(super) actions: &'a counter_action::Actions,
    pub(super) vision: &'a Arc<Vision>,
    /// The recorded occupant order; `None` when the hour's census is incomplete, which
    /// offers units in cold handle order.
    pub(super) order: Option<&'a counter_order::RecordedOrder>,
    /// Naval, orders-group and portrait inputs.
    pub(super) native: &'a NativeInputs,
    /// Orders groups' parents, army-group bytes and colours, and the orders' child bytes.
    pub(super) orders: &'a counter_inputs::OrdersIndex,
    pub(super) guns: &'a GunInputs,
    pub(super) abilities: &'a AbilityInputs,
    pub(super) task_forces: &'a TaskForceMotions,
    /// Divisions providing a led orders group's HQ.
    pub(super) hq_providers: &'a BTreeSet<u64>,
    /// [`hq_portraits`].
    pub(super) portraits: &'a BTreeMap<u64, counter_order::Portrait>,
    /// `ArmyIconInputs` by division.
    pub(super) icon_inputs: &'a BTreeMap<u64, replay_data::rows::ArmyIconInputRow>,
}

/// One division's `DivisionStatus` row, plus the one `DivisionMovement` byte the glyphs need.
///
/// A division without a status row takes [`DivisionStatuses::MISSING`]: no glyph and no bars.
#[derive(Clone, Copy, Default)]
pub(super) struct DivisionStatus {
    pub(super) maximum_hit_points: i64,
    pub(super) maximum_organisation: i64,
    pub(super) entrenchment: i64,
    pub(super) entrenchment_cap: i64,
    pub(super) supply_ratio: i64,
    /// Attrition-presence predicate ([`DivisionStatusRow::has_attrition`]).
    ///
    /// [`DivisionStatusRow::has_attrition`]: replay_data::rows::DivisionStatusRow::has_attrition
    pub(super) attrition: bool,
    /// `DivisionStatus.exercising`, or Map's `DivisionModelInputs.exercising`; `None`
    /// without a row or for a refused read.
    pub(super) exercising: Option<bool>,
    /// `DivisionStatus.has_order`; `None` without a row.
    pub(super) has_order: Option<bool>,
    pub(super) invalid_orders: u8,
    pub(super) naval_transfer: bool,
    pub(super) strength_factor: i64,
}

/// A native source bucket: province and recorded visual stack kind.
pub(super) type StackKey = (u32, u32);

/// One admitted unit, retained until the geographic group's first-fit rebuild. Its route is
/// borrowed from the hour's [`Movements`].
#[derive(Clone)]
pub(super) struct LandUnit<'m> {
    /// The ordinal in the province's recorded unit order; zero without a recorded order.
    pub(super) ordinal: u32,
    pub(super) alternate_template: u64,
    /// Whether the unit's owner shares vision; `Err` when the hour cannot prove
    /// a negative ([`Vision::shares_vision_proven`]).
    pub(super) shared_vision: Result<bool, SharingUnknown>,
    pub(super) separate: bool,
    pub(super) id: u64,
    pub(super) owner: u32,
    /// The logical owner, when it is a linked country.
    pub(super) logical: Option<u32>,
    pub(super) flag_owner: u32,
    pub(super) type_tint: Option<type_tint::Side>,
    pub(super) template: u64,
    pub(super) orders_group: u64,
    pub(super) hq: bool,
    pub(super) sight: Sight,
    /// Fixed5 [`counter_status::modified_strength`].
    pub(super) hp: i64,
    /// [`counter_status::ratio`] of organisation, fixed5.
    pub(super) org: i64,
    /// Whether `hp` and `org` divide by recorded maxima.
    pub(super) bars: bool,
    pub(super) status: counter_status::Member,
    pub(super) movement: Option<&'m Movement>,
    /// Division action; ignored for railway guns, which carry their own classifier below.
    pub(super) action: counter_action::Action,
    /// The orders group's parent; `None` when the group row is not recorded.
    pub(super) group_parent: Option<u64>,
    /// [`counter_inputs::parent_branch`]; `None` when a group or plan row is missing.
    pub(super) parent_branch: Option<bool>,
    /// Whether this unit's orders group is withdrawing; `None` when unrecorded.
    pub(super) withdrawing: Option<bool>,
    /// [`counter_inputs::area_defence`]; `None` when the hour cannot settle it.
    pub(super) defending_area: Option<bool>,
    /// The member's orders-group commander for the ability grid: `Some(None)` for none,
    /// `None` when its group row is not recorded.
    pub(super) commander: Option<Option<u64>>,
    /// Set for a railway gun, which only ever shares a plate with other guns.
    pub(super) gun: Option<GunUnit>,
}

/// A railway gun's own join and draw operands.
#[derive(Clone)]
pub(super) struct GunUnit {
    /// The stat definition's name: the class-13 join compares it, and it names the icon.
    pub(super) name: String,
    /// The native first-edge duration and reconstructed map-stack kind for hover text.
    pub(super) timing: viewer_foundation::map::counter_hovers::land::Timing,
    pub(super) location_warning: Option<bool>,
    /// [`guns::join_action`].
    pub(super) join_action: u8,
    /// [`guns::action_code`], with a named gap when its operands are unavailable.
    pub(super) action: counter_action::Action,
    /// Where the gun hangs this hour: the movement layer's route position for its kind.
    pub(super) position: Option<counter_move::Position>,
}

/// The settings the game reads besides the two units. Local selection is
/// checked separately by [`CounterInputs::selected_join`] before these rules.
#[derive(Clone, Copy, Debug)]
pub(super) struct JoinRules {
    /// Coarse country-grouping setting.
    pub(super) coarse: bool,
    /// `counter_color_mode`, which picks the owner test.
    pub(super) color_mode: CounterColorMode,
    /// NATO icon setting used when comparing custom counter icons.
    pub(super) nato: bool,
}

impl JoinRules {
    /// The owner test runs for every kind before its coarse return. In country-colour
    /// mode, the representative and candidate must have the same canonical roster-owner
    /// slot; otherwise their allegiance categories must agree.
    pub(super) fn owners_join(self, countries: &Countries, rep: u32, candidate: u32) -> bool {
        match self.color_mode {
            CounterColorMode::Country => rep == candidate,
            CounterColorMode::Allegiance => countries.side(rep) == countries.side(candidate),
        }
    }
}

/// one source icon per province/visual bucket, holding every
/// class of unit its province's occupant array puts there. Its unit count does not weight the
/// geographic group's mean.
#[derive(Default)]
pub(super) struct Source<'a> {
    pub(super) units: Vec<SourceUnit<'a>>,
}

/// One member of a source: a land unit (division or railway gun) or a task force. Most
/// members are land units, so boxing that variant would allocate for nearly every unit.
#[allow(clippy::large_enum_variant)]
pub(super) enum SourceUnit<'a> {
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

/// A land plate's opacity in the map mode: the navy mode draws army counters at its own
/// opacity; railway guns keep theirs.
pub(super) fn plate_opacity(visibility: &Visibility, gun: bool) -> f32 {
    if gun { 1.0 } else { visibility.army_opacity }
}

/// One plate of a column: a land stack or a naval item, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plate {
    Land(usize),
    Naval(usize),
}

impl LandUnit<'_> {
    /// Test plate sharing in the existing order: kind, owner/allegiance and coarse
    /// gates, then candidate-owner disclosure and the corresponding template checks.
    /// Shared disclosure compares real priority definitions and custom indices;
    /// otherwise fake-preferred priority names can match. Own representatives also
    /// compare idle state and group inheritance. Missing reached operands leave the
    /// join unknown, retaining first-fit order.
    pub(super) fn can_join(
        &self,
        rep: &LandUnit<'_>,
        templates: &TemplateIcons,
        countries: &Countries,
        rules: JoinRules,
    ) -> Result<bool, SharingUnknown> {
        // Filter items by unit kind before testing plate sharing: a gun and a division can
        // share a column but never a plate.
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

    /// [`Self::can_join`]'s tests from the disclosure branch on, for a candidate whose owner
    /// shares vision with the viewer (`shared`) or not.
    fn joins_disclosed(
        &self,
        rep: &LandUnit<'_>,
        templates: &TemplateIcons,
        countries: &Countries,
        rules: JoinRules,
        shared: bool,
    ) -> bool {
        // Disclosure is tested for the candidate owner. Shared disclosure uses real
        // priority-definition identity and real-template custom indices. Otherwise
        // fake-preferred names and indices permit sharing across country-local templates.
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
        if countries.side(rep.owner) == Allegiance::Ours {
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
    /// Railway-gun plate sharing after the common allegiance and coarse gates.
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
        if countries.side(rep.owner) == Allegiance::Ours {
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

/// Keep the first admitted representative. Mixed-plate flags therefore follow
/// source order rather than a later member.
pub(super) fn first_fit<T: Copy>(
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

/// [`LandUnit::can_join`] as [`first_fit`]'s test for `unit` against the representative `rep`.
/// A join it cannot decide is not taken, and `undecided` records the unit (by id) with the
/// missing input: the plate it stands in may differ from the game's.
pub(super) fn land_join(
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

/// The representative owner's disclosure as the type glyph needs it (the game takes the
/// real template's custom picture only when the owner shares vision). An unknown disclosure
/// is `Err` only where the two glyphs it allows differ, under the view's NATO option and the
/// sprite registry (`declared`), which the glyph is selected with ([`TemplateIcons::icon`]).
pub(super) fn glyph_disclosure(
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

/// One division's route this hour, joined to `Divisions.id`.
///
/// This route does not keep `DivisionMovement.retreat`: the game uses it for
/// the 3-D order-arrow ribbon and model animation, which this pass does not draw.
/// The game also selects action code 10 for a retreat;
/// [`viewer_foundation::map::counter_action`] consumes that byte independently of this route.
///
/// The route's first anchor is the division's own `Divisions.province`, which
/// also anchors its standing counter. The recorded `DivisionMovement.source`
/// is not consumed by this pass.
pub(super) type Movement = counter_move::DivisionRoute;

/// This hour's `DivisionMovement` rows joined to the one ETA source
/// ([`viewer_foundation::movement_timing::EtaCache`]), cached per hour for the window's per-frame icon
/// rebuilds.
#[derive(Clone, Default)]
pub(super) struct Movements {
    pub(super) by_division: BTreeMap<u64, Movement>,
    pub(super) eta: Report,
}
impl Movements {
    /// A division whose route the content map cannot place stands on its province's
    /// `Standstill` slot and is counted in the report, never dropped.
    pub(super) fn read(
        frame: &DecodedFrame,
        etas: &Etas,
        movement: Option<&MovementContent>,
        content: &CounterContent,
    ) -> io::Result<Self> {
        let map = movement.map(|movement| &movement.map);
        let routes = counter_move::DivisionRoutes::read(frame, etas, map, &|id| {
            content.anchor(id, STANDSTILL)
        })?;
        let mut eta = etas.report().clone();
        eta.routes_unresolved = routes.unresolved;
        eta.route_hops_skipped = routes.skipped_hops;
        Ok(Self {
            by_division: routes.by_division,
            eta,
        })
    }

    pub(super) fn get(&self, division: u64) -> Option<&Movement> {
        self.by_division.get(&division)
    }
}

/// This hour's `DivisionStatus` rows, joined to `Divisions.id` and cached per
/// hour for the window's per-frame icon rebuilds.
#[derive(Clone, Default)]
pub(super) struct DivisionStatuses {
    pub(super) by_division: BTreeMap<u64, DivisionStatus>,
    /// A recording without DivisionStatus: each division's exercise predicate from
    /// DivisionModelInputs, `None` where its read was refused.
    pub(super) witnessed: BTreeMap<u64, Option<bool>>,
}

impl DivisionStatuses {
    /// What a division with no recorded row reads as; its counter draws no bars or glyphs from it.
    const MISSING: DivisionStatus = DivisionStatus {
        maximum_hit_points: 0,
        maximum_organisation: 0,
        entrenchment: 0,
        entrenchment_cap: 0,
        supply_ratio: counter_status::FIXED,
        attrition: false,
        exercising: None,
        has_order: None,
        invalid_orders: 0,
        naval_transfer: false,
        strength_factor: counter_status::FIXED,
    };

    pub(super) fn read(frame: &DecodedFrame) -> io::Result<Self> {
        if !frame.recorded(replay_data::rows::TableId::DivisionStatus) {
            let witnessed = rows::<tables::DivisionModelInputs>(frame)?
                .rows()
                .iter()
                .map(|row| (row.division, row.exercising.map(|value| value != 0)))
                .collect();
            return Ok(Self {
                by_division: BTreeMap::new(),
                witnessed,
            });
        }
        let mut by_division: BTreeMap<u64, DivisionStatus> = BTreeMap::new();
        for row in rows::<tables::DivisionStatus>(frame)?.rows() {
            by_division.insert(
                row.division,
                DivisionStatus {
                    maximum_hit_points: row.maximum_hit_points,
                    maximum_organisation: row.maximum_organisation,
                    entrenchment: row.entrenchment,
                    entrenchment_cap: row.entrenchment_cap,
                    supply_ratio: row.supply_ratio,
                    attrition: row.has_attrition(),
                    exercising: Some(row.exercising != 0),
                    has_order: Some(row.has_order != 0),
                    invalid_orders: row.invalid_orders,
                    naval_transfer: false,
                    strength_factor: row.strength_factor,
                },
            );
        }
        for row in rows::<tables::DivisionMovement>(frame)?.rows() {
            if let Some(status) = by_division.get_mut(&row.division) {
                status.naval_transfer = row.naval_transfer != 0;
            }
        }
        Ok(Self {
            by_division,
            witnessed: BTreeMap::new(),
        })
    }

    pub(super) fn get(&self, division: u64) -> DivisionStatus {
        self.by_division
            .get(&division)
            .copied()
            .unwrap_or(DivisionStatus {
                exercising: self.witnessed.get(&division).copied().flatten(),
                ..Self::MISSING
            })
    }

    pub(super) fn recorded(&self, division: u64) -> bool {
        self.by_division.contains_key(&division)
    }
}

/// One template's type-icon operands (`DivisionTemplateIcons`): its priority sub-unit's name,
/// custom icon index and keep-my-custom-icon flag. The icon depends on the view's NATO
/// counter option, so it is selected when drawn ([`TemplateIcons::icon`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TemplateIcon {
    pub(super) name: String,
    pub(super) custom_icon: i32,
    pub(super) custom_icon_nato: u8,
}

/// Prepare pictorial and NATO icon selections per template. Signed narrowing
/// and sprite existence govern custom icons before sub-unit fallback.
#[derive(Clone, Default)]
pub(super) struct TemplateIcons {
    pub(super) by_template: BTreeMap<u64, TemplateIcon>,

    /// Compare the priority sub-unit key and narrowed custom index even when
    /// low intel later replaces the type icon with a question mark.
    pub(super) grouping: BTreeMap<u64, (Vec<u8>, i32, u8)>,
    pub(super) priorities: BTreeMap<u64, u32>,
}

const TEMPLATE_ICON_TABLES: &[replay_data::rows::TableId] =
    &[replay_data::rows::TableId::DivisionTemplateIcons];

#[derive(Clone)]
pub(super) struct TemplateIconMemo(Arc<viewer_foundation::map::memo::Keyed<TemplateIcons>>);

impl Default for TemplateIconMemo {
    fn default() -> Self {
        Self(Arc::new(viewer_foundation::map::memo::Keyed::new(
            TEMPLATE_ICON_TABLES,
        )))
    }
}

impl TemplateIconMemo {
    // Refusals are not cached: each hour keeps its own error context.
    pub(super) fn read(&self, frame: &DecodedFrame) -> io::Result<Arc<TemplateIcons>> {
        self.0.get_or_build(frame, (), |projected| {
            TemplateIcons::read(projected).map(Arc::new)
        })
    }
}

impl TemplateIcons {
    /// `DivisionTemplateIcons` is keyed by the full u64 template handle held by
    /// `Divisions.template`. `priority_unit` is the recorded counter sub-unit.
    pub(super) fn read(frame: &DecodedFrame) -> io::Result<Self> {
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

    /// Select the representative's fake-preferred priority sub-unit icon, preserving
    /// custom-index narrowing and the NATO frame selection. `declared` is the
    /// resolved sprite registry lookup.
    pub(super) fn icon(
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
        // Selection needs the fake-preferred template's priority sub-unit type (`value`)
        // even when the custom picture is drawn; the icon index is narrowed to 16 bits.
        let plain = self.by_template.get(&alternate)?;
        Some(templates::type_icon(
            &plain.name,
            custom,
            nato_counters,
            declared,
        ))
    }
}

/// This hour's railway guns and the rail and name operands their plates read.
#[derive(Clone, Default)]
pub(super) struct GunInputs {
    /// `None` while the gun census is incomplete: first-fit membership cannot be established.
    pub(super) rows: Option<Vec<RailwayGunCounterInputRow>>,
    /// Guns this hour holds rows for, drawn or not; the report's unavailable count.
    pub(super) recorded: usize,
    /// Lexical token to the authored sub-unit key it names, for the stat
    /// definitions the guns name.
    pub(super) names: BTreeMap<u32, String>,
    /// Gun sub-unit tokens this hour cannot name, by reason.
    pub(super) refused: BTreeSet<String>,
    /// Current raw rail-path count and cooldown, independently of sorted `RailEdges`.
    /// Missing/refused/stale province domains have no entry.
    pub(super) rail: Option<BTreeMap<u32, guns::RailStatus>>,
    /// For each moving railway gun, resolve hostility of its last ordinary-route
    /// province controller. Missing reached relation inputs remain unavailable.
    pub(super) destination_enemy: BTreeMap<u64, Result<bool, &'static str>>,
}

impl GunInputs {
    /// The authored sub-unit key a gun's stat definition names this hour,
    /// which its join compares and its icon uses; `None` when it was refused.
    pub(super) fn name_of(&self, gun: &RailwayGunCounterInputRow) -> Option<&str> {
        self.names.get(&gun.subunit_token).map(String::as_str)
    }

    pub(super) fn read(
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
        // A gun records its stat definition by token. The engine builds a
        // sub-unit's name and token from one authored key, so the hour's
        // spelling names the one authored key equal to it under the lexer's
        // fold, and that key is the name the join compares and the icon uses.
        let mut refused = BTreeSet::new();
        let names = if wanted.is_empty() {
            BTreeMap::new()
        } else {
            let tokens = TokenNames::new(frame)?;
            wanted
                .iter()
                .filter_map(|&token| {
                    let named = tokens
                        .name(token)
                        .map_err(|reason| reason.to_string())
                        .and_then(|spelling| {
                            subunits
                                .map_err(|reason| format!("sub-units unavailable: {reason}"))?
                                .resolve(spelling)?
                                .map(str::to_owned)
                                .ok_or_else(|| format!("sub-unit {spelling} is not authored"))
                        });
                    match named {
                        Ok(name) => Some((token, name)),
                        Err(reason) => {
                            refused.insert(format!("railway gun sub-unit: {reason}"));
                            None
                        }
                    }
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
        let destination_enemy = table
            .rows()
            .iter()
            .filter(|gun| !gun.ordinary_path.is_empty())
            .map(|gun| {
                (
                    gun.gun,
                    hostility.destination_enemy(
                        &gun.ordinary_path,
                        gun.ordinary_path.len() as u32,
                        Some(gun.logical_owner),
                    ),
                )
            })
            .collect();
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

/// An active-ability record is active when its phase byte is zero.
const ACTIVE_ABILITY_PHASE: u8 = 0;

/// One grid item's recorded identity and the content token selecting its sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AbilityIcon {
    pub(super) country: u32,
    pub(super) ordinal: u32,
    pub(super) token: Vec<u8>,
}

/// Every country's active abilities by caster, in recorded vector order.
///
/// Fill the grid using the representative's country and the member's commander:
/// include active records whose caster is exactly that commander.
#[derive(Clone, Default)]
pub(super) struct AbilityInputs {
    /// A truncated or unavailable hour, or one naming an unknown country, cannot say that a
    /// commander has no active ability.
    complete: bool,
    by_commander: BTreeMap<(u32, u64), Vec<AbilityIcon>>,
}

impl AbilityInputs {
    pub(super) fn read(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> io::Result<Self> {
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

    /// The commander's active abilities, empty when it has none; `None` when unknown.
    pub(super) fn of(&self, country: u32, commander: u64) -> Option<&[AbilityIcon]> {
        self.complete.then(|| {
            self.by_commander
                .get(&(country, commander))
                .map_or(&[][..], Vec::as_slice)
        })
    }
}

/// One fully visible member's turn at the ability grid. the game repopulates it for each
/// member above intel 249 that has a commander, so the last such member wins, even when that
/// commander has nothing active; a member without a commander leaves it alone, and one whose
/// commander is unrecorded makes it unknown.
pub(super) fn update_abilities(
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

/// Offers each railway gun to its province's source for its visual bucket.
///
/// Class 13 shares the native province source walk with class-0 armies, in the
/// recorded mixed occupant order. Its stack kind and position are the movement layer's class-13
/// rules ([`counter_move::UnitRoute::railway_gun`]): a moving gun that does not snap hangs at
/// the settled route point over its `Standstill RG` anchors, the same rule as a division's. A
/// route the content cannot place stands on its province and is counted.
fn add_railway_guns(
    inputs: &CounterInputs<'_>,
    owner: &impl Fn(u32) -> Option<u32>,
    land: &CounterTables<'_>,
    sources: &mut BTreeMap<StackKey, Source<'_>>,
    out: &mut CounterFrame,
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
    let movement = inputs.movement;
    let world = inputs.route_world();
    let anchor = |id, slot| inputs.content.anchor(id, slot);
    for gun in rows {
        if gun.province == 0 {
            continue;
        }
        let Some(slot) = owner(gun.roster_owner) else {
            continue;
        };
        let ordinal = match land.order {
            Some(order) => match order.mixed(gun.gun, gun.province) {
                Some(ordinal) => ordinal,
                None => continue,
            },
            None => 0,
        };
        let sight = land.vision.sees_land(gun.gun, slot, gun.province);
        if sight == Sight::None {
            continue;
        }
        let Some(name) = land.guns.name_of(gun).map(str::to_owned) else {
            out.report.unavailable_railway_guns += 1;
            continue;
        };
        let route = counter_move::UnitRoute::railway_gun(gun);
        let class = viewer_foundation::unit_class::CounterClass::RailwayGun;
        let placed = movement.and_then(|movement| {
            route.stack_kind(gun.province, &movement.map, &|id| {
                anchor(id, class.standstill(true))
            })
        });
        // A route content cannot place stands on its province, like a division's; the
        // stand-off kind needs no geometry.
        let kind = placed.unwrap_or_else(|| {
            if !route.display_path().is_empty() {
                out.report.unplaced_railway_gun_routes += 1;
            }
            counter_move::stack_kind(counter_move::StackKindInputs {
                class,
                combat_role: gun.combat_role,
                ..Default::default()
            })
        });
        // The game skips source bucket 19 for every class: a docked gun is not on the map.
        if kind == SHIP_IN_PORT {
            continue;
        }
        let position = match (placed, &world) {
            (Some(kind), Some(world)) => route.position(gun.province, kind, world, &anchor),
            _ => route
                .standing(gun.province, true, kind, &anchor)
                .map(counter_move::Position::still),
        };
        let on_land = movement
            .and_then(|movement| movement.map.terrain(gun.province))
            .map_or_else(
                || {
                    matches!(
                        inputs.map.definitions.get(&gun.province).map(|it| it.kind),
                        Some(ProvinceKind::Land)
                    )
                },
                |terrain| terrain.flags & 1 != 0,
            );
        let rail = land
            .guns
            .rail
            .as_ref()
            .and_then(|rail| rail.get(&gun.province).copied());
        let location_warning = guns::location_warning(gun, on_land, rail);
        if location_warning.is_none() {
            out.report.unavailable_railway_gun_locations += 1;
        }
        // The detailed flag owner; a coarse column takes the unit's own ([`land_stack`]).
        let flag_owner = unit_flag_owner(slot, gun.expeditionary_owner, false, owner);
        sources
            .entry((gun.province, kind))
            .or_default()
            .units
            .push(SourceUnit::Land(LandUnit {
                ordinal,
                alternate_template: 0,
                shared_vision: land.vision.shares_vision_proven(slot),
                separate: false,
                id: gun.gun,
                type_tint: land.type_tints.of(gun.roster_owner, gun.logical_owner),
                owner: slot,
                logical: owner(gun.logical_owner),
                flag_owner,
                template: 0,
                orders_group: gun.assignee_group,
                hq: false,
                sight,
                hp: guns::strength_ratio(gun.current_strength, gun.maximum_strength),
                org: 0,
                bars: true,
                status: counter_status::Member {
                    supply_ratio: gun.received_supply_ratio,
                    naval_transfer: gun.naval_transfer != 0,
                    own: slot == land.vision.ours(),
                    ..Default::default()
                },
                movement: None,
                action: Ok(1),
                group_parent: None,
                parent_branch: None,
                withdrawing: None,
                defending_area: None,
                commander: Some(None),
                gun: Some(GunUnit {
                    name,
                    timing: viewer_foundation::map::counter_hovers::land::Timing {
                        next: movement.and_then(|content| {
                            replay_derive::eta_derive::railway_gun_first_edge(
                                gun,
                                |id| {
                                    content.map.terrain(id).map(|terrain| {
                                        replay_derive::eta_derive::ProvinceOperands {
                                            terrain_flags: terrain.flags,
                                            ..Default::default()
                                        }
                                    })
                                },
                                |from, to| content.map.distance(from, to),
                                &content.defines,
                            )
                        }),
                        kind: placed,
                    },
                    location_warning,
                    join_action: guns::join_action(gun),
                    action: guns::action_code(
                        gun,
                        land.guns
                            .destination_enemy
                            .get(&gun.gun)
                            .copied()
                            .unwrap_or(Err("last ordinary-route province hostility")),
                    ),
                    position,
                }),
            }));
    }
}

/// The grouping pass's kind for a land counter; see `Icon::kind`.
pub(super) const LAND_KIND: u32 = 0;

/// Division operands in province and visual-slot order. Movement stays in the hour's
/// shared movement table and is borrowed only by camera-admitted sources.
pub(super) type DivisionCounters = BTreeMap<StackKey, Vec<LandUnit<'static>>>;

pub(super) fn division_counters(
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    land: &CounterTables<'_>,
) -> io::Result<DivisionCounters> {
    let mut sources = DivisionCounters::new();
    let hq_providers = land.hq_providers;
    let commanders: BTreeMap<u64, u64> = rows::<tables::OrdersGroups>(frame)?
        .rows()
        .iter()
        .map(|group| (group.handle, group.leader))
        .collect();
    for division in rows::<tables::Divisions>(frame)?.rows() {
        let Some(slot) = owner(division.roster_owner) else {
            continue;
        };
        // Walk the province's own occupant array; a division it does not hold there
        // is never bucketed.
        let ordinal = match land.order {
            Some(order) => match order.mixed(division.id, division.province) {
                Some(ordinal) => ordinal,
                None => continue,
            },
            None => 0,
        };
        let sight = land.vision.sees_land(division.id, slot, division.province);
        if sight == Sight::None {
            continue;
        }
        let icon_input = land.icon_inputs.get(&division.id);
        let alternate_template = icon_input
            .filter(|r| r.alternate_template != 0)
            .map_or(division.template, |r| r.alternate_template);
        let separate = icon_input.map_or_else(
            || hq_providers.contains(&division.id),
            |r| r.separate_counter != 0,
        );
        let movement = land.movement.get(division.id);
        let kind = movement.map_or(STANDSTILL, |m| m.stack_kind);
        // A transported division in a land province joins the port's naval icon.
        if kind == SHIP_IN_PORT {
            continue;
        }
        let status = land.status.get(division.id);
        // The detailed flag owner; a coarse column takes the unit's own ([`land_stack`]).
        let flag_owner = unit_flag_owner(slot, division.expeditionary_owner, false, owner);
        sources
            .entry((division.province, kind))
            .or_default()
            .push(LandUnit {
                ordinal,
                id: division.id,
                type_tint: land
                    .type_tints
                    .of(division.roster_owner, division.logical_owner),
                alternate_template,
                shared_vision: land.vision.shares_vision_proven(slot),
                separate,
                hq: separate && hq_providers.contains(&division.id),
                owner: slot,
                logical: owner(division.logical_owner),
                flag_owner,
                template: division.template,
                orders_group: division.orders_group,
                sight,
                hp: counter_status::modified_strength(
                    division.hit_points,
                    status.maximum_hit_points,
                    status.strength_factor,
                ),
                org: counter_status::ratio(division.organisation, status.maximum_organisation),
                bars: land.status.recorded(division.id),
                status: counter_status::Member {
                    supply_ratio: status.supply_ratio,
                    naval_transfer: status.naval_transfer,
                    entrenchment: status.entrenchment,
                    entrenchment_cap: status.entrenchment_cap,
                    attrition: status.attrition,
                    exercising: status.exercising,
                    grouped: division.orders_group != 0,
                    own: slot == land.vision.ours(),
                    has_order: status.has_order,
                    invalid_orders: status.invalid_orders,
                },
                movement: None,
                action: land.actions.get(division.id),
                group_parent: land
                    .orders
                    .groups
                    .get(&division.orders_group)
                    .map(|group| group.parent),
                parent_branch: counter_inputs::parent_branch(
                    division.id,
                    division.orders_group,
                    land.orders,
                ),
                withdrawing: land.native.withdrawing(division.orders_group),
                defending_area: counter_inputs::area_defence(
                    division.id,
                    division.orders_group,
                    land.orders,
                ),
                commander: if division.orders_group == 0 {
                    Some(None)
                } else {
                    commanders
                        .get(&division.orders_group)
                        .map(|&leader| (leader != 0).then_some(leader))
                },
                gun: None,
            });
    }
    Ok(sources)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn land_counters<'a>(
    inputs: &CounterInputs<'_>,
    divisions: &DivisionCounters,
    owner: &impl Fn(u32) -> Option<u32>,
    land: &CounterTables<'a>,
    naval: &NavalContext<'a>,
    geography: &Geography,
    friendly: &impl Fn(u32, u32) -> bool,
    mut retained: Option<&mut ColumnPlans>,
    out: &mut CounterFrame,
) -> io::Result<PreparedLand> {
    // Division sources follow the recorded province occupant order. Units absent
    // from that array do not contribute to the source.
    let mut sources: BTreeMap<StackKey, Source<'a>> = BTreeMap::new();
    let admission = Admission::new(inputs.camera, inputs.viewport);
    let armies = inputs.visibility.unit_kinds == UnitKinds::All;
    if armies {
        for (&key, units) in divisions {
            if !admission.admits_province(&inputs.content.bounds, key.0) {
                continue;
            }
            let source = sources.entry(key).or_default();
            source.units.extend(units.iter().map(|unit| {
                let unit = unit.clone();
                // The cached member owns every other operand; admitted sources borrow
                // their movement from the same hour that supplied those operands.
                let movement = land.movement.get(unit.id);
                SourceUnit::Land(LandUnit { movement, ..unit })
            }));
        }
    }
    let unavailable = out.report.unavailable_railway_guns;
    // Unknown gun geometry still occupies its source: the game refuses the column.
    if armies {
        add_railway_guns(inputs, owner, land, &mut sources, out);
    }
    if armies && inputs.assets.layouts.gun.is_err() {
        out.report.unavailable_railway_guns = unavailable
            + land
                .guns
                .recorded
                .max(out.report.unavailable_railway_guns - unavailable);
    }
    add_task_forces(inputs, owner, land, naval, &admission, &mut sources, out);
    let mut icons = Vec::new();
    let mut source_kinds = Vec::new();
    let mut admitted = Vec::new();
    let mut positions = Vec::new();
    let mut units = Vec::new();
    for ((province, kind), mut source) in sources {
        source.units.sort_by_key(SourceUnit::ordinal);
        // Use each source's selected anchor; ordinary sources use the first member's animator position.
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
    // Each icon groups at the level its own distance gives: one level for the game's camera.
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
    let mut undecided = BTreeMap::new();
    // Keep only programs used in this preparation; live data, not an arbitrary cache cap,
    // bounds the memo. Camera admission and geographic grouping above always run afresh.
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
            #[cfg(test)]
            let members_equal = before
                .as_ref()
                .is_some_and(|(before, _)| before.sources == key.sources);
            // Per-anchor free-view operand dependencies are kept on the original path.
            let hit = inputs.camera.free().is_none()
                && before.as_ref().is_some_and(|(before, _)| *before == key);
            #[cfg(test)]
            super::preparation::plan_probe::record(|counts| {
                counts.ordered_groups += 1;
                counts.ordered_group_members_equal += u64::from(members_equal);
                counts.semantic_plans += 1;
                counts.semantic_plan_hits += u64::from(hit);
            });
            let program = if hit {
                before.unwrap().1
            } else {
                column_program(inputs, &units, height, rules, land, naval, &mut undecided)
            };
            let result = program.append(&mut drawn, &mut styles, &mut flotillas, &mut undecided);
            memo.previous.insert(leader, (key, program));
            result
        } else {
            column_program(inputs, &units, height, rules, land, naval, &mut undecided).append(
                &mut drawn,
                &mut styles,
                &mut flotillas,
                &mut undecided,
            )
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
                                viewer_foundation::map::counter_hovers::unit_hits::Kind::RailwayGun
                            } else {
                                viewer_foundation::map::counter_hovers::unit_hits::Kind::Division
                            },
                            &drawn[member].divisions,
                        ),
                        Plate::Naval(member) => (
                            viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce,
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
        sprites: (0..drawn.len()).map(|_| None).collect(),
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
    })
}

/// A column's semantic program. It owns values only: no screen coordinates, native instances,
/// atlas cells, packets or lifecycle clocks. Native requests and setters are still replayed by
/// the ordinary draw owners on every frame.
#[derive(Clone, Default)]
struct ColumnProgram {
    stacks: Vec<Stack>,
    styles: Vec<StackStyle>,
    flotillas: Vec<Flotilla>,
    items: Vec<Plate>,
    unknown: BTreeMap<u64, SharingUnknown>,
}

/// Source boundaries and occurrence order both matter, including repeated native handles.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OrderedSource {
    province: u32,
    kind: u32,
    members: Vec<(Unit, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ColumnKey {
    sources: Vec<OrderedSource>,
    coarse: bool,
    split_selected: bool,
    // A unit can contribute to more than one geographic group. Earlier first-fit failures
    // may therefore affect this group's materialized grouping_unknown, even on a cache hit.
    unknown_before: Vec<Option<SharingUnknown>>,
}

#[derive(Default)]
pub(super) struct ColumnPlans {
    previous: BTreeMap<(u32, u32), (ColumnKey, ColumnProgram)>,
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
                viewer_foundation::map::counter_hovers::unit_hits::Kind::RailwayGun
            } else {
                viewer_foundation::map::counter_hovers::unit_hits::Kind::Division
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
            viewer_foundation::map::counter_hovers::unit_hits::Kind::RailwayGun
        } else {
            viewer_foundation::map::counter_hovers::unit_hits::Kind::Division
        };
        stack.selected = inputs.selected.contains(&kind.unit(stack.divisions[0]));
        // The allegiance palette uses the type icon's relation.
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

/// Compare the priority sub-unit key and narrowed custom index even when
/// low intel later replaces the type icon with a question mark.
pub(super) fn land_stack(
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
        // Set under the counter colour mode by the caller.
        plate: Ok([1.0; 4]),
        // A coarse plate shows the unit's own flag, a detailed one its expeditionary owner's.
        flag_owner: if coarse { rep.owner } else { rep.flag_owner },
        coarse,
        type_tint: rep.type_tint,
        hidden: false,
        hit_points: Vec::new(),
        organisation: Vec::new(),
        bars: true,
        members: Vec::new(),
        divisions: item.iter().map(|unit| unit.id).collect(),
        timing: item
            .iter()
            .map(|unit| {
                unit.gun.as_ref().map_or_else(
                    || viewer_foundation::map::counter_hovers::land::Timing {
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
        // an own land representative's orders group, or its parent on
        // The branch, colours `group_marker`.
        marker: (rep.owner == land.vision.ours())
            .then(|| land.orders.marker(rep.orders_group, rep.parent_branch))
            .flatten(),
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
            // OR the withdrawing flags of the visible HQ providers.
            stack.leader_en_route = match (stack.leader_en_route, unit.withdrawing) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            };
        }
        if unit.sight == Sight::Partial {
            stack.hidden = true;
        } else {
            // The first member's roster owner is the country the game is asked.
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
            stack.members.push(unit.status);
        }
    }
    stack
}

pub(super) fn stack_style(inputs: &CounterInputs<'_>, stack: &Stack) -> StackStyle {
    let count = if stack.hidden {
        UNKNOWN_COUNT.to_owned()
    } else {
        stack.members.len().to_string()
    };
    match &stack.gun {
        // A gun plate has no organisation bar; its strength getter and glyphs are its own.
        Some(gun) => StackStyle {
            organisation: 0.0,
            strength: counter_status::stack_fill(&stack.hit_points),
            glyphs: counter_status::Glyphs {
                supply: counter_status::supply_frame(
                    counter_status::stack_supply(&stack.members),
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
            // Every glyph but training reads a member's status; one unrecorded withholds them, as
            // it does the bars. Training also reads Map's DivisionModelInputs.
            glyphs: if stack.bars {
                counter_status::glyphs(&stack.members, Some(inputs.content.defines.supply))
            } else {
                counter_status::Glyphs {
                    training: counter_status::stack_training(&stack.members),
                    ..Default::default()
                }
            },
            count,
        },
    }
}

/// Each HQ provider's portrait source, keyed by the providing division.
///
/// Runtime state follows [`counter_order::hq_portrait`]. An incomplete leader census leaves
/// portraits undrawn; authored character tokens cannot establish runtime portrait overrides.
/// `LeaderPortraitPaths.tag` supplies the directory branch of a non-character picture.
pub(super) fn hq_portraits(
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
    // The recorded path needs the leader census; the character holder is consulted
    // only for character references. A truncated character table preserves portraits
    // that leader rows settle, while leaders pointing at an unrecorded character
    // stay undrawn.
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
                // The portrait census covers every HQ selected by this predicate,
                // across all views. A missing row is unrecorded, not a null leader.
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

/// Draws one land counter, its children keyed by their `mapicons.gui` declaration order.
pub(super) fn draw_land_counter(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    icon: u32,
    origin: [f32; 2],
    land: &PreparedLand,
    member: usize,
    priority: i8,
) -> io::Result<hovers::Presented> {
    let first = batch.quads.len();
    let type_refusal = draw_land_children(inputs, batch, origin, land, member)?;
    let mut controls = hovers::presented(
        inputs,
        &land.stacks[member],
        land.stacks[member].coarse,
        origin,
        &batch.quads[first..],
    );
    controls.refused.extend(type_refusal);
    rank_children(batch, first, priority, icon)?;
    Ok(controls)
}

/// [`draw_land_counter`]'s children, each at the raw declaration index as its layer.
fn draw_land_children(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    origin: [f32; 2],
    land: &PreparedLand,
    member: usize,
) -> io::Result<Option<String>> {
    let stack = &land.stacks[member];
    if let (Some(gun), Ok(layout)) = (&stack.gun, &inputs.assets.layouts.gun) {
        return Ok(draw_gun_children(
            inputs, batch, origin, land, member, gun, layout,
        ));
    }
    let style = &land.styles[member];
    let (countries, icons_of) = (&land.countries, &land.templates);
    // `land_plate` counts and skips a plate whose colour the hour cannot give.
    let Ok(plate) = stack.plate else {
        return Ok(None);
    };
    let layout = inputs
        .assets
        .layouts
        .land
        .as_ref()
        .expect("land plate layout was admitted");
    let assets = inputs.assets;
    if stack.coarse {
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
            },
        )?;
        // Hide the other children while keeping `low_supply`, so the land supply crate stays.
        if !stack.hidden
            && let Some(frame) = style.glyphs.supply
            && let Some(sheet) = assets.sheet(&layout.supply.sprite)
        {
            batch.at_layer(u64::from(layout.supply.order));
            draw_child(batch, sheet, &layout.supply, origin, frame, [1.0; 4]);
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
        return Ok(None);
    }
    // count_bg binds a Button; the game uses Up's
    // strip frame 0 unless a positive authored frame overrides it.
    batch.at_layer(u64::from(layout.plate.order));
    if let Some(sheet) = assets.sheet(if stack.hq {
        "GFX_onmap_unit_counter_empty"
    } else {
        &layout.plate.sprite
    }) {
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
    if let Some(color) = stack.marker
        && let Some(sheet) = assets.sheet(&layout.group_marker.sprite)
    {
        batch.at_layer(u64::from(layout.group_marker.order));
        draw_child(batch, sheet, &layout.group_marker, origin, 0, color);
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
    // No bars without a fully visible member, or when a member's maxima are not recorded.
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
    // The plate, its allegiance tint and the flag are untouched by the hidden
    // byte; only the `type` child and the count box change.
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
            // An unknown disclosure whose two glyphs differ draws no glyph; the report counts
            // it.
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
        // Preserve the icon's disabled state through its selected effect technique.
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
    Ok(type_refusal)
}

/// Where the `position`th ability item's sprite lands. The grid steps one `slotsize` right per
/// item. In the game, `orientation = center`
/// only puts the sprite's position on the item's centre; the sprite is centred on it only with
/// `centerPosition = yes` or `origo = center`. Halves are whole pixels, as the naval combat
/// bubble's. The shipped item is 20 x 20 with a 32 x 32 sprite at 0.8 and no centring, so the
/// icon hangs right and down from the item's centre.
pub(super) fn ability_rect(
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

/// Whether any of a stack's ability items reaches the viewport; the grid hangs above the
/// plate, so a plate just off screen can still show its abilities.
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
                on_screen([rect[0], rect[1]], [rect[2], rect[3]], inputs.extent())
            })
    })
}

/// The window a plate is laid out with: a gun's own when it is drawn, else the land counter's.
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

/// Column width comes from the first item; heights are summed in order.
/// Centre the column at its anchor using each plate's own dimensions.
pub(super) fn column_origin(anchor: [f32; 2], sizes: &[[f32; 2]]) -> Option<[i32; 2]> {
    let first = sizes.first()?;
    let total = sizes
        .iter()
        .fold(0i32, |total, size| total.wrapping_add(size[1] as i32));
    Some([
        (anchor[0] - first[0] * 0.5).round() as i32,
        (anchor[1] - total as f32 * 0.5).round() as i32,
    ])
}

/// The railway-gun plate's children in declaration order.
///
/// The type icon comes from the gun's own stat definition. The action icon is
/// derived for every member regardless of intel and shows only on the viewing
/// country's own plates. The coarse counter leaves the supply strip unchanged.
fn draw_gun_children(
    inputs: &CounterInputs<'_>,
    batch: &mut Batch,
    origin: [f32; 2],
    land: &PreparedLand,
    member: usize,
    gun: &GunStack,
    layout: &GunCounter,
) -> Option<String> {
    let stack = &land.stacks[member];
    let style = &land.styles[member];
    let assets = inputs.assets;
    let Ok(plate) = stack.plate else {
        return None;
    };
    let mut draw = |child: &Child, frame: u32, tint: [f32; 4]| {
        batch.at_layer(u64::from(child.order));
        if let Some(sheet) = assets.sheet(&child.sprite) {
            draw_child(batch, sheet, child, origin, frame, tint);
        }
    };
    if let Some(frame) = style.glyphs.supply {
        draw(&layout.supply, frame, [1.0; 4]);
    }
    if stack.coarse {
        // The coarse arm only rewrites the plate, the medium flag and the count.
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
        return None;
    }
    draw(&layout.plate, layout.plate.frame_index(0), plate);
    if stack.selected && !stack.members.is_empty() {
        draw(&layout.selected, layout.selected.frame_index(0), [1.0; 4]);
    }
    if let Some(color) = stack.marker {
        draw(&layout.group_marker, 0, color);
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
    type_refusal
}

/// Where one stack's counter hangs: its province's anchor, or a point along its route.
///
/// A stack in a combat stand-off kind snaps to that anchor slot; everything
/// else stands at its route's settled point ([`counter_move::Motion`]). Without the content
/// movement map it stands on its province.
pub(super) fn stack_position(
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

/// The tint the game gives the `attrition` child once, at bind time: it is the only
/// counter child the binder colours, and the shipped skull texture is white.
pub(super) const ATTRITION_TINT: [f32; 4] = [1.0, 0.3, 0.3, 1.0];

/// What a stack contributes to its glyphs besides `DivisionStatus`.
#[derive(Clone, Copy, Debug)]
pub(super) struct GlyphStack {
    /// An HQ provider is on the plate, so the left-hanging icons take `unit_counter_hq_x_offset`.
    pub(super) hq: bool,
    /// [`counter_inputs::area_defence`] of the last member with an orders group.
    pub(super) defending_area: bool,
}

/// Draws the status glyphs a stack's counter hangs off its plate, each at its
/// declaration index: the red order icon, the area shield and the yellow order
/// icon share pixels, so their order matters. Several hang outside the window,
/// which sets `clipping = no`.
pub(super) fn draw_glyphs(
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
    if let Some(frame) = glyphs.supply {
        draw(&layout.supply, frame, [1.0; 4]);
    }
    if let Some(frame) = glyphs.digin {
        draw(&layout.digin, frame, [1.0; 4]);
    }
    if glyphs.training == Some(true) {
        draw(&layout.training, 0, [1.0; 4]);
    }
}

/// Draw the selected type icon. An undeclared selection keeps the game's pooled
/// drawable, which the recording does not hold; refuse that icon by name.
pub(super) fn draw_type_icon(
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
    use super::*;

    /// The attrition glyph follows the attrition-presence predicate on the row's
    /// recorded value and threshold, strictly greater; the retired byte is
    /// not read.
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
    fn columns_keep_the_native_grid_origin_and_viewport_item_offsets() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    use super::*;
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
