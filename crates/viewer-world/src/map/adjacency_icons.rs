//! Strait and canal (adjacency-rule) map icons.
//!
//! Every rule of the content's `adjacency_rules` file that names an `icon` province links that
//! province to its icon; a later rule naming the same province replaces the link. The icon is the
//! content's `adjacency_rule_mapicon` window, placed at the province's position plus the rule's
//! authored `offset` through the shared map-icon projection, left-aligned and vertically centred on that point, with the map
//! icon admission every pooled family uses.
//!
//! It shows in a map mode with adjacency-rule icons ([`Visibility::adjacency_rules`]: navy,
//! supply and resources) while the camera is strictly below
//! `NGraphics.ADJACENCY_RULE_DISTANCE_CUTOFF`, whatever the rule's open, closed or trade state.
//! It draws at the mode's `NMapIcons.<table>_PRIORITY_ADJACENCY_RULE`. The navy mode's hover
//! highlight of the rule's area is not drawn.

use std::collections::{BTreeMap, HashMap};

use installed_content::{adjacency_rules::AdjacencyRule, defines::Defines};
use viewer_foundation::map::{
    camera::Camera,
    gui_resources::{count_refused, viewport as gui_viewport},
    icon_windows::{Owner, Plan, PoolView},
    mode_visibility::{Priorities, PriorityIcon, Visibility},
    native_gui::{self, lifecycle::MapIconPool},
    native_resources::Context,
    native_windows::ADJACENCY_RULE_WINDOW,
    sprites::{self, Batch},
};

/// The native adjacency window is left-aligned and vertically centred.
const ALIGNMENT: u8 = 0x0a;

/// One province's icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icon {
    pub province: u32,
    pub offset: [f32; 3],
}

/// The content's rule icons and their camera cutoff.
#[derive(Clone, Debug, PartialEq)]
pub struct AdjacencyIcons {
    pub icons: Vec<Icon>,
    /// `NGraphics.ADJACENCY_RULE_DISTANCE_CUTOFF`, or why it cannot be read.
    pub cutoff: Result<f32, String>,
    /// Rules whose icon cannot be read, by reason.
    pub refused: BTreeMap<String, usize>,
}

impl AdjacencyIcons {
    pub fn new(rules: &[AdjacencyRule], defines: &Defines) -> Self {
        let cutoff = defines
            .number("NGraphics", "ADJACENCY_RULE_DISTANCE_CUTOFF")
            .map_err(|error| error.to_string())
            .and_then(|value| {
                let value = value as f32;
                if value.is_finite() {
                    Ok(value)
                } else {
                    Err("NDefines.NGraphics.ADJACENCY_RULE_DISTANCE_CUTOFF is not finite".into())
                }
            });
        Self::of(rules, cutoff)
    }

    pub fn of(rules: &[AdjacencyRule], cutoff: Result<f32, String>) -> Self {
        let mut icons: Vec<Icon> = Vec::new();
        let mut at: BTreeMap<u32, usize> = BTreeMap::new();
        let mut refused = BTreeMap::new();
        for rule in rules {
            match &rule.icon {
                Ok(Some(placement)) => match at.get(&placement.province) {
                    Some(&index) => icons[index].offset = placement.offset,
                    None => {
                        at.insert(placement.province, icons.len());
                        icons.push(Icon {
                            province: placement.province,
                            offset: placement.offset,
                        });
                    }
                },
                Ok(None) => {}
                Err(reason) => {
                    *refused
                        .entry(format!("an adjacency rule's icon: {reason}"))
                        .or_default() += 1;
                }
            }
        }
        Self {
            icons,
            cutoff,
            refused,
        }
    }
}

/// Whether a mode shows the icons at a camera height.
pub fn shown(visibility: &Visibility, camera_height: f32, cutoff: f32) -> bool {
    visibility.adjacency_rules && camera_height < cutoff
}

/// The icons' priority in a mode.
pub fn priority(priorities: &Priorities, visibility: &Visibility) -> i8 {
    priorities.get(visibility.priorities, PriorityIcon::AdjacencyRule)
}

/// What [`AdjacencyIcons::prepare`] needs.
pub struct Inputs<'a> {
    /// A province's map position.
    pub anchor: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub camera: Camera,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
    pub visibility: Visibility,
}

struct Placed {
    province: u32,
    screen: [f32; 2],
}

/// The icons to present this frame, farthest first.
#[derive(Default)]
pub struct PreparedAdjacencyIcons {
    placed: Vec<Placed>,
    /// What is not drawn and why.
    pub report: BTreeMap<String, usize>,
}

impl PreparedAdjacencyIcons {
    /// The provinces whose icons are placed, in draw order.
    pub fn provinces(&self) -> impl Iterator<Item = u32> + '_ {
        self.placed.iter().map(|placed| placed.province)
    }

    /// Where each placed icon's anchor projects, in draw order.
    pub fn screens(&self) -> impl Iterator<Item = [f32; 2]> + '_ {
        self.placed.iter().map(|placed| placed.screen)
    }
}

impl AdjacencyIcons {
    pub fn prepare(&self, inputs: &Inputs<'_>) -> PreparedAdjacencyIcons {
        let mut out = PreparedAdjacencyIcons {
            report: self.refused.clone(),
            ..Default::default()
        };
        if !inputs.visibility.adjacency_rules || self.icons.is_empty() {
            return out;
        }
        let cutoff = match &self.cutoff {
            Ok(cutoff) => *cutoff,
            Err(reason) => {
                *out.report
                    .entry(format!("{ADJACENCY_RULE_WINDOW} is not drawn: {reason}"))
                    .or_default() += self.icons.len();
                return out;
            }
        };
        if !shown(&inputs.visibility, inputs.camera.height, cutoff) {
            return out;
        }
        let viewport = gui_viewport(inputs.viewport, inputs.ui_scale);
        let mut placed = Vec::with_capacity(self.icons.len());
        for icon in &self.icons {
            let Some(anchor) = (inputs.anchor)(icon.province) else {
                *out.report
                    .entry(format!(
                        "{ADJACENCY_RULE_WINDOW} is not drawn: its province has no map position"
                    ))
                    .or_default() += 1;
                continue;
            };
            let world = std::array::from_fn(|axis| anchor[axis] + icon.offset[axis]);
            let Some(projection) = inputs
                .camera
                .project_map_icon(world, inputs.viewport, viewport)
            else {
                continue;
            };
            placed.push((
                projection.depth,
                Placed {
                    province: icon.province,
                    screen: projection.position,
                },
            ));
        }
        placed.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.placed = placed.into_iter().map(|(_, placed)| placed).collect();
        out
    }
}

impl PreparedAdjacencyIcons {
    /// Presents the icons through `view`'s pool and draws the visible windows at `priority`.
    /// Returns how many were drawn.
    pub fn draw(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        priority: i8,
        view: &mut PoolView,
        batch: &mut Batch,
        report: &mut BTreeMap<String, usize>,
    ) -> usize {
        let owners: Vec<_> = self
            .placed
            .iter()
            .map(|placed| Owner {
                id: placed.province,
                screen: placed.screen,
                dirty: false,
            })
            .collect();
        let order: HashMap<u32, usize> = self
            .placed
            .iter()
            .enumerate()
            .map(|(ordinal, placed)| (placed.province, ordinal))
            .collect();
        let mut drawn = view.present(
            context,
            &Plan {
                name: ADJACENCY_RULE_WINDOW,
                kind: MapIconPool::AdjacencyRule,
                alignment: ALIGNMENT,
                owners: &owners,
            },
            Box::new(|_, _| Ok(())),
            |_, _, _, _| Ok(()),
            report,
        );
        drawn.sort_by_key(|drawn| order[&drawn.owner]);
        let mut count = 0;
        for drawn in drawn {
            batch.at_layer(sprites::layer(priority, order[&drawn.owner] as u32, 0));
            if let Err(why) =
                native_gui::draw(&drawn.layout, drawn.origin, context.resources, batch)
            {
                count_refused(report, ADJACENCY_RULE_WINDOW, &[why]);
                continue;
            }
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests;
