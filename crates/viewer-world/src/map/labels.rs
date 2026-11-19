//! Victory-point and capital labels from resolved map-icon templates.
//!
//! Each template retains its hierarchy, resources and lifecycle. Labels use interface
//! pixels ([`super::ui_scale`]) and content-defined priorities; counters draw later.
//! Dot distance interpolates between the content's minimum and maximum and clamps
//! at the maximum. Text distance uses the same victory-point fraction without the
//! upper clamp. Both hide at equality. Capitals use their separate content cutoffs.
//!
//! Allegiance uses the province controller and the war-only relation table, leaving
//! faction partners grey. Localisation keys use the state owner's original tag or tag
//! ([`LabelTables::victory_point_tag`]).
//!
//! A recorded rename replaces the whole static name ladder. Its bytes are looked up
//! as a localisation key and, if absent, displayed as literal text. Without a rename,
//! [`victory_point_keys`] tries the owner-qualified key, then the unqualified key.
//! If neither exists, no name is drawn. Names end at the first NUL byte. Invalid
//! UTF-8 after that cut retains the static name and reports `ProvinceNames.name`.

use std::collections::HashMap;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_foundation::map::free_icons::{IconAnchor, IconFamily};
use viewer_foundation::map::gui_resources::count_refused;
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::icon_windows::PoolView;
use viewer_foundation::map::icon_windows::Retry;
#[cfg(test)]
use viewer_foundation::map::label_reads::victory_point_keys;
use viewer_foundation::map::label_reads::{
    label_anchors, label_name, overrides_ladder, unreadable,
};
use viewer_foundation::map::mode_visibility::{Priorities, PriorityIcon, Visibility};
use viewer_foundation::map::native_resources::Context;
use viewer_foundation::map::province_schedule::Schedule;
#[cfg(test)]
use viewer_foundation::map::test_support::gui;

use installed_content_core::{gui::native, map::MapData, map::ProvinceKind};
use installed_content_localisation::localisation::Localisation;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use viewer_foundation::map::{
    allegiance::{self, Allegiance},
    camera::Camera,
    native_gui::{
        self,
        lifecycle::{MapIconPool, SlotId},
    },
    sprites::{self, Batch},
    tables::{Available, core, rows},
};

/// The `NDefines` operands of the victory-point and capital labels.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelDefines {
    /// `NGraphics.VICTORY_POINT_MAP_ICON_AFTER`, one entry per `VICTORY_POINT_LEVELS`: the value
    /// each icon size starts at.
    pub after: Vec<f32>,
    /// `VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN` and `_MAX`.
    pub dot_cutoff: [f32; 2],
    /// `VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN` and `_MAX`.
    pub text_cutoff: [f32; 2],
    /// `VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT`.
    pub max_points_for_percent: f32,
    /// `CAPITAL_ICON_CUTOFF` and `VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX`.
    pub capital_cutoff: [f32; 2],
    /// Every map mode's `NMapIcons.<table>_PRIORITY_VICTORY_POINTS` and `_CAPITAL`.
    pub priorities: Priorities,
}

impl LabelDefines {
    pub fn load(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let number = |section: &str, key: &str| -> io::Result<f32> {
            let value = defines.number(section, key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.{section}.{key} is outside f32")))
            }
        };
        let graphics = |key: &str| number("NGraphics", key);
        let levels = graphics("VICTORY_POINT_LEVELS")?;
        let after = defines.numbers("NGraphics", "VICTORY_POINT_MAP_ICON_AFTER")?;
        if levels < 1.0 || levels.fract() != 0.0 || after.len() < levels as usize {
            return Err(invalid(
                "NDefines.NGraphics.VICTORY_POINT_MAP_ICON_AFTER has fewer entries than VICTORY_POINT_LEVELS",
            ));
        }
        Ok(Self {
            after: after[..levels as usize].iter().map(|&v| v as f32).collect(),
            dot_cutoff: [
                graphics("VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN")?,
                graphics("VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX")?,
            ],
            text_cutoff: [
                graphics("VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN")?,
                graphics("VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX")?,
            ],
            max_points_for_percent: graphics(
                "VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT",
            )?,
            capital_cutoff: [
                graphics("CAPITAL_ICON_CUTOFF")?,
                graphics("VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX")?,
            ],
            priorities: Priorities::load(defines)?,
        })
    }

    /// Thresholds for synthetic layout fixtures.
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            after: vec![0.0, 9.0, 20.0],
            dot_cutoff: [100.0, 1000.0],
            text_cutoff: [100.0, 800.0],
            max_points_for_percent: 22.0,
            capital_cutoff: [1100.0, 1000.0],
            priorities: Priorities::uniform([
                10, 11, 5, 3, 3, 3, 3, 20, 20, 19, 13, 13, 5, 3, 3, 10, 3, 10,
            ]),
        }
    }
}

/// Colour frames per icon shape; the game steps sizes by `* 5`.
const FRAMES_PER_SHAPE: u32 = 5;

/// Unit-stack type 38 `Victory point`, the label anchor,
/// read from the finalised unit-stack table ([`LabelContent::set_anchors`]).
pub const ANCHOR_STACK: u32 = viewer_foundation::map::label_reads::VICTORY_POINT;

use viewer_foundation::map::native_windows::CAPITAL_WINDOW;
use viewer_foundation::map::native_windows::VICTORY_POINT_WINDOW;

const ICON: &str = "icon";
const TEXT: &str = "text";
/// The batch part of [`TEXT`], which anchors the name apart from the dot.
const TEXT_PART: &str = ".text";

/// A label's shape group: 0 for the capital star, then one per size from largest
/// to smallest. The last reached `VICTORY_POINT_MAP_ICON_AFTER` threshold
/// selects the level, starting at level 0 when none is reached.
pub fn shape_group(capital: bool, victory_points: i32, defines: &LabelDefines) -> u32 {
    if capital {
        return 0;
    }
    let level = defines
        .after
        .iter()
        .skip(1)
        .take_while(|&&after| victory_points as f32 >= after)
        .count();
    (defines.after.len() - level) as u32
}

/// The strip frame is five times the shape group plus the allegiance colour index.
/// The dot's allegiance follows the province controller; see the module docs.
pub fn dot_allegiance(
    controller: u32,
    links: &BTreeMap<u32, u32>,
    allegiances: &BTreeMap<u32, Allegiance>,
) -> Allegiance {
    links
        .get(&controller)
        .and_then(|slot| allegiances.get(slot).copied())
        .unwrap_or(Allegiance::Other)
}

pub fn frame_index(shape: u32, allegiance: Allegiance) -> u32 {
    shape * FRAMES_PER_SHAPE + allegiance.index()
}

/// The camera height at which the icon is hidden.
pub fn dot_cutoff(capital: bool, victory_points: i32, defines: &LabelDefines) -> f32 {
    let [low, high] = defines.dot_cutoff;
    if capital {
        defines.capital_cutoff[0]
    } else {
        let fraction = victory_points as f32 / defines.max_points_for_percent;
        (fraction * (high - low) + low).min(high)
    }
}

/// The camera height at which the name text is hidden, unclamped.
pub fn text_cutoff(capital: bool, victory_points: i32, defines: &LabelDefines) -> f32 {
    let [low, high] = defines.text_cutoff;
    if capital {
        defines.capital_cutoff[1]
    } else {
        let step = (1.0 / f64::from(defines.max_points_for_percent)) as f32;
        (high - low) * step * victory_points as f32 + low
    }
}

/// Static content the label pass reads once per document.
pub struct LabelContent {
    pub defines: LabelDefines,
    /// World anchor per province: its finalised unit-stack slot 38
    /// ([`LabelContent::set_anchors`]).
    pub anchors: BTreeMap<u32, [f32; 3]>,
    /// Provinces whose slot 38 is the engine's seeded centroid default,
    /// stated once.
    pub centroid_fallbacks: Vec<u32>,
}

impl LabelContent {
    /// Reads the defines; the anchors come from the finalised unit-stack
    /// table once it is seeded ([`LabelContent::set_anchors`]).
    pub fn load(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            defines: LabelDefines::load(defines)?,
            anchors: BTreeMap::new(),
            centroid_fallbacks: Vec::new(),
        })
    }

    /// Anchors each province's label at its finalised unit-stack slot 38,
    /// the one table every consumer reads (`CounterContent::anchor`): the
    /// authored slot, else slot 10's, else the seeded centroid default
    /// (`seeded`), as the game
    /// finalises it.
    pub fn set_anchors(
        &mut self,
        provinces: impl Iterator<Item = u32>,
        anchor: impl Fn(u32) -> Option<[f32; 3]>,
        seeded: impl Fn(u32) -> bool,
    ) {
        (self.anchors, self.centroid_fallbacks) = label_anchors(provinces, anchor, seeded);
    }
}

/// Everything the labels' layout needs besides the decoded hour. Nothing here moves with the
/// camera: [`PreparedLabels::project`] applies that.
pub struct LabelInputs<'a> {
    pub content: &'a LabelContent,
    pub localisation: &'a Localisation,
    pub map: &'a MapData,
    /// What the view's map mode shows: the raids mode keeps only the capitals, and the
    /// resources mode draws no names.
    pub visibility: Visibility,
}

/// Where the labels laid out for an hour and view stand this frame.
#[derive(Clone, Copy)]
pub struct LabelProjection {
    pub camera: Camera,
    /// The frame in device pixels, which the projection uses.
    pub viewport: [u32; 2],
    /// Device pixels per interface pixel; the labels are laid out in interface
    /// pixels. See [`super::ui_scale`].
    pub ui_scale: f32,
}

/// What one frame of labels drew, for reporting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabelReport {
    pub icons: usize,
    pub texts: usize,
    pub refused_gui: BTreeMap<String, usize>,
    /// Names left undrawn because the game's localiser cannot produce them
    /// ([`label_name`]): their number, and each reason by province (with the
    /// times it was met).
    pub refused_names: usize,
    pub refused_localisation: BTreeMap<String, BTreeMap<u32, usize>>,
    /// The viewing country's tag, which the allegiance colours are measured against.
    pub perspective: Option<String>,
}

mod projection;
pub use projection::{LabelTables, NO_VICTORY_POINTS, TIED_VICTORY_POINTS, note_withheld, note_withheld_capitals, unreadable_overrides};
#[cfg(test)]
use projection::{capital_provinces, withheld_capitals};
#[cfg(test)]
mod projection_oracle;

impl LabelReport {
    /// What the labels left undrawn, for the warning area.
    pub fn warnings(&self) -> Vec<String> {
        let provinces = |provinces: &BTreeMap<u32, usize>| {
            provinces
                .keys()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let localisation = self.refused_localisation.iter().map(|(reason, at)| {
            format!(
                "{} victory-point names are not drawn: {reason}; provinces {}",
                at.len(),
                provinces(at)
            )
        });
        localisation
            .chain(
                self.refused_gui
                    .iter()
                    .map(|(why, count)| format!("Map labels: {count} {why}")),
            )
            .collect()
    }
}

/// Whether a map mode draws a victory-point or capital label, and if so whether with its name:
/// the raids mode clears the victory points with the other province icons but keeps the
/// capitals, which are state icons; the resources mode empties every name and keeps the dots
/// and stars.
fn mode_label(capital: bool, visibility: &Visibility) -> Option<bool> {
    (capital || visibility.province_icons).then_some(visibility.label_text)
}

/// Lays out one hour's victory-point and capital labels, for every camera: the candidate's
/// content, its world anchor and the distances its dot and its name are cut off at.
/// [`PreparedLabels::project`] then places them for the camera of a frame.
pub fn prepare(
    inputs: &LabelInputs<'_>,
    tables: &map_display_data::landscape::Labels,
) -> io::Result<PreparedLabels> {
    let defines = &inputs.content.defines;
    let mut candidates = Vec::with_capacity(tables.candidates.len());
    for province in tables.candidates.iter() {
        let Some(definition) = inputs.map.definitions.get(&province.province) else {
            continue;
        };
        if matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake) {
            continue;
        }
        let capital = province.capital;
        let victory_points = province.victory_points;
        if victory_points <= 0 && !capital {
            continue;
        }
        let Some(named) = mode_label(capital, &inputs.visibility) else {
            continue;
        };
        let Some(anchor) = inputs.content.anchors.get(&province.province) else {
            continue;
        };
        let shape = shape_group(capital, victory_points, defines);
        let icon = if capital {
            PriorityIcon::Capital
        } else {
            PriorityIcon::VictoryPoints
        };
        candidates.push(Candidate {
            province: province.province,
            world: *anchor,
            capital,
            frame: (shape * FRAMES_PER_SHAPE
                + province
                    .controller
                    .and_then(|slot| tables.allegiances.get(&slot).copied())
                    .unwrap_or(Allegiance::Other.index())) as i32
                + 1,
            priority: defines.priorities.get(inputs.visibility.priorities, icon),
            dot: dot_cutoff(capital, victory_points, defines),
            text: text_cutoff(capital, victory_points, defines),
            name: match &province.name {
                _ if !named => Ok(None),
                map_display_data::landscape::LabelName::Value { text } => {
                    Ok(text.as_deref().map(Arc::from))
                }
                map_display_data::landscape::LabelName::Refused { reason } => Err(reason.clone()),
            },
        });
    }
    Ok(PreparedLabels {
        perspective: tables.perspective.clone(),
        candidates,
        placed: Vec::new(),
        report: LabelReport {
            perspective: tables.perspective.clone(),
            ..Default::default()
        },
    })
}

/// One label the hour and view call for, at every camera.
struct Candidate {
    province: u32,
    world: [f32; 3],
    capital: bool,
    frame: i32,
    priority: i8,
    /// The distances the dot and the name are gone by.
    dot: f32,
    text: f32,
    /// The name the mode draws, `None` where it draws none; `Err` where the game's localiser
    /// cannot produce it, which is reported for the frames that would have shown it.
    name: Result<Option<Arc<str>>, String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Label {
    province: u32,
    screen: [f32; 2],
    depth: f32,
    capital: bool,
    frame: i32,
    priority: i8,
    text: Option<Arc<str>>,
    /// Where the label stands for a free eye: its dot, and its name, which fades by its own
    /// cutoff.
    anchors: [IconAnchor; 2],
}

/// A prepared hour's label values. Preparing never changes a presented window.
#[derive(Default)]
pub struct PreparedLabels {
    candidates: Vec<Candidate>,
    placed: Vec<Label>,
    perspective: Option<String>,
    pub report: LabelReport,
}

impl PreparedLabels {
    /// Places the laid-out labels for one camera: each candidate nearer than its dot's distance,
    /// projected, with its name while it stands nearer than the name's distance.
    pub fn project(&mut self, at: &LabelProjection) {
        let extent = gui_resources_viewport(at.viewport, at.ui_scale);
        self.report = LabelReport {
            perspective: self.perspective.clone(),
            ..Default::default()
        };
        self.placed.clear();
        self.placed.reserve(self.candidates.len());
        for candidate in &self.candidates {
            let world = candidate.world;
            // The camera's height, or a free eye's distance to this label.
            let height = at.camera.icon_distance(world);
            if height >= candidate.dot {
                continue;
            }
            let Some(projection) = at.camera.project_map_icon(world, at.viewport, extent) else {
                continue;
            };
            let text = if height >= candidate.text {
                None
            } else {
                match &candidate.name {
                    Ok(name) => name.clone(),
                    Err(reason) => {
                        self.report.refused_names += 1;
                        *self
                            .report
                            .refused_localisation
                            .entry(reason.clone())
                            .or_default()
                            .entry(candidate.province)
                            .or_default() += 1;
                        None
                    }
                }
            };
            self.placed.push(Label {
                province: candidate.province,
                screen: projection.position,
                depth: projection.depth,
                capital: candidate.capital,
                frame: candidate.frame,
                priority: candidate.priority,
                text,
                anchors: [candidate.dot, candidate.text].map(|cutoff| IconAnchor {
                    world,
                    cutoff,
                    family: IconFamily::Labels,
                    moves: None,
                }),
            });
        }
    }
}

/// Native window histories and cached text; the name cache belongs to each reused slot.
#[derive(Debug, Default)]
pub struct LabelView {
    pools: [PoolView; 2],
    /// The labels the kept draw drew and the projection it read: a draw of the same labels from
    /// the same projection draws and reports the same again.
    shown: Vec<Label>,
    /// The dirty bits the kept draw read, one per kept label.
    shown_dirty: Vec<bool>,
    projected: LabelReport,
    /// What the kept draw drew and reported. A draw that finds nothing changed leaves them
    /// standing.
    drawn: Batch,
    report: LabelReport,
    texts: [BTreeMap<SlotId, String>; 2],
    /// What each pool's update last wrote to each live owner's window: the same again is no
    /// change ([`PoolView::present_kept`]).
    written: [HashMap<u32, Written>; 2],
    /// What each pool's windows drew, handed again while a window keeps its layout and anchors.
    painted: [viewer_foundation::map::icon_windows::Painted<u32, [IconAnchor; 2]>; 2],
    labels: HashMap<u32, usize>,
    owners: Vec<viewer_foundation::map::icon_windows::Owner>,
}

/// One label window's update as last written: its slot, its dot's frame and its name.
#[derive(Debug)]
struct Written {
    slot: SlotId,
    frame: i32,
    text: Option<Arc<str>>,
}

impl LabelView {
    /// What the last draw drew, which a draw that found nothing changed left standing.
    pub fn drawn(&self) -> &Batch {
        &self.drawn
    }

    /// What the last draw reported.
    pub fn report(&self) -> &LabelReport {
        &self.report
    }

    /// Presents the next frames for the pools' passes alone ([`PoolView::set_quiet`]).
    pub fn set_quiet(&mut self, quiet: bool) {
        for pool in &mut self.pools {
            pool.set_quiet(quiet);
        }
    }

    /// Whether a label the next present names stands in a province a coming phase cleans
    /// ([`PoolView::set_cleaning`], [`PreparedLabels::cleaning`]).
    pub fn set_cleaning(&mut self, cleaning: bool) {
        for pool in &mut self.pools {
            pool.set_cleaning(cleaning);
        }
    }

    pub fn running_visibly(&self) -> bool {
        self.pools.iter().any(PoolView::running_visibly)
    }

    pub fn retries(&self) -> Vec<Retry> {
        self.pools.iter().filter_map(PoolView::retry).collect()
    }

    pub fn take_running(&mut self) -> bool {
        let vp = self.pools[0].take_running();
        let capital = self.pools[1].take_running();
        vp || capital
    }
}

impl PreparedLabels {
    pub fn draw(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        schedule: &Schedule,
        view: &mut LabelView,
    ) {
        self.draw_scheduled(context, &self.dirty(schedule), view);
    }

    /// Snapshot just this layer's dirty bits before borrowed workers read the schedule.
    /// No worker keeps the shared schedule locked through GUI layout or resource requests.
    /// Whether a placed label stands in a province dirty out of view ([`Schedule::cleans`]).
    pub fn cleaning(&self, schedule: &Schedule) -> bool {
        self.placed.iter().any(|label| schedule.cleans(label.province))
    }

    pub fn dirty(&self, schedule: &Schedule) -> Vec<bool> {
        self.placed
            .iter()
            .map(|label| schedule.dirty(label.province))
            .collect()
    }

    pub fn draw_scheduled(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        dirty: &[bool],
        view: &mut LabelView,
    ) {
        assert_eq!(dirty.len(), self.placed.len());
        use viewer_foundation::map::icon_windows::{Owner, Plan};
        if self.nothing_due(context, dirty, view) {
            viewer_foundation::map::redraws::note("labels.nothing_due", 1);
            for (pool, name) in view.pools.iter_mut().zip([VICTORY_POINT_WINDOW, CAPITAL_WINDOW]) {
                pool.stood_still(name);
            }
            return;
        }
        view.drawn.clear();
        let mut report = self.report.clone();
        view.labels.clear();
        for (index, name) in [VICTORY_POINT_WINDOW, CAPITAL_WINDOW]
            .into_iter()
            .enumerate()
        {
            if !view.pools[index].same(context) {
                view.texts[index].clear();
                view.written[index].clear();
            }
            let labels = &mut view.labels;
            view.owners.clear();
            for (at, label) in self
                .placed
                .iter()
                .enumerate()
                .filter(|(_, label)| usize::from(label.capital) == index)
            {
                labels.insert(label.province, at);
                view.owners.push(Owner {
                    id: label.province,
                    screen: label.screen,
                    dirty: dirty[at],
                });
            }
            let controls = context.windows.gui_type(name.as_bytes()).map(|root| {
                let icon = root
                    .find(native::Kind::Icon, ICON.as_bytes())
                    .ok_or_else(|| format!("no icon named {ICON}"))?;
                let text = root
                    .find(native::Kind::TextBox, TEXT.as_bytes())
                    .ok_or_else(|| format!("no text box named {TEXT}"))?;
                Ok::<_, String>((icon, text))
            });
            let held = std::cell::RefCell::new(std::mem::take(&mut view.written[index]));
            view.pools[index].present_kept(
                context,
                &Plan {
                    name,
                    kind: if index == 0 {
                        MapIconPool::Label
                    } else {
                        MapIconPool::Capital
                    },
                    alignment: 0x0A,
                    owners: &view.owners,
                },
                Box::new(|instance, root| {
                    // The game initially hides the bound text.
                    let text = root
                        .find(native::Kind::TextBox, TEXT.as_bytes())
                        .ok_or_else(|| format!("no text box named {TEXT}"))?;
                    instance.hide(root, text)?;
                    instance.set_hide_flag(root, text, true)
                }),
                |owner, slot, instance, root| {
                    let (icon, control) = controls
                        .as_ref()
                        .expect("the helper found the template")
                        .as_ref()
                        .map_err(Clone::clone)?;
                    let label = &self.placed[labels[&owner]];
                    held.borrow_mut().remove(&owner);
                    instance.set_frame(root, icon, label.frame)?;
                    let text = label.text.as_deref().unwrap_or_default();
                    if view.texts[index].get(&slot).map_or("", String::as_str) != text {
                        // Write label text only when it changes.
                        if text.is_empty() {
                            instance.hide(root, control)?;
                            instance.set_hide_flag(root, control, true)?;
                        } else {
                            instance.set_text(root, control, text, context.resources)?;
                            instance.set_hide_flag(root, control, false)?;
                            instance.show(root, control)?;
                        }
                        view.texts[index].insert(slot, text.to_owned());
                    }
                    held.borrow_mut().insert(
                        owner,
                        Written {
                            slot,
                            frame: label.frame,
                            text: label.text.clone(),
                        },
                    );
                    Ok(())
                },
                |owner, slot| {
                    let label = &self.placed[labels[owner]];
                    held.borrow().get(owner).is_some_and(|written| {
                        written.slot == slot
                            && written.frame == label.frame
                            && written.text == label.text
                    })
                },
                &mut report.refused_gui,
            );
            let written = &mut view.written[index];
            *written = held.into_inner();
            if written.len() > view.owners.len() {
                let live: std::collections::HashSet<u32> =
                    view.pools[index].live().into_iter().map(|(owner, _)| owner).collect();
                written.retain(|owner, _| live.contains(owner));
            }
        }
        // A quiet frame ran the passes; what the labels drew before stands.
        if view.pools[0].quiet() {
            view.report = report;
            return;
        }
        let labels = &view.labels;
        let batch = &mut view.drawn;
        let mut drawn: Vec<_> = view
            .pools
            .iter()
            .enumerate()
            .flat_map(|(pool, view)| view.drawn_generations().map(move |drawn| (pool, drawn)))
            .map(|(pool, (drawn, generation))| (&self.placed[labels[&drawn.owner]], drawn, pool, generation))
            .collect();
        // Sort both label families by depth.
        drawn.sort_by(|a, b| b.0.depth.total_cmp(&a.0.depth));
        let painted = &mut view.painted;
        for (ordinal, (label, drawn, pool, generation)) in drawn.into_iter().enumerate() {
            let layer = sprites::layer(label.priority, ordinal as u32, 0);
            batch.at_layer(layer);
            batch.at_anchor(Some(label.anchors[0]));
            // The name stands with the dot and fades by the name's cutoff.
            batch.at_part_anchor(TEXT_PART, label.anchors[1]);
            let draw = |batch: &mut Batch| native_gui::draw(&drawn.layout, drawn.origin, context.resources, batch);
            match painted[pool]
                .paint(
                    sprites::WindowKey::new(sprites::WindowPool::Label, pool as u32, drawn.owner),
                    drawn.owner, generation, drawn.origin, layer, label.anchors, batch, draw)
            {
                Ok(()) => {
                    report.icons += 1;
                    report.texts +=
                        usize::from(!label.text.as_deref().unwrap_or_default().is_empty());
                }
                Err(why) => count_refused(
                    &mut report.refused_gui,
                    if label.capital {
                        CAPITAL_WINDOW
                    } else {
                        VICTORY_POINT_WINDOW
                    },
                    &[why],
                ),
            }
        }
        batch.at_anchor(None);
        for painted in painted {
            painted.finish();
        }
        view.shown.clone_from(&self.placed);
        view.shown_dirty.clear();
        view.shown_dirty.extend_from_slice(dirty);
        view.projected.clone_from(&self.report);
        view.report = report;
    }

    /// Whether this draw would draw and report what the kept draw drew and reported, so the frame
    /// draws nothing: the same labels placed from the same projection with the same dirty bits,
    /// both pools standing still with every window they showed kept ([`PoolView::stands_still`]),
    /// and every one of those windows showing what its label offers. The owners of both passes
    /// are then the kept draw's owners, each at the same screen point with the same dirty bit, so
    /// neither pass has anything to do and every window draws what it drew.
    fn nothing_due(
        &self,
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        dirty: &[bool],
        view: &LabelView,
    ) -> bool {
        if view.shown_dirty != dirty {
            viewer_foundation::map::redraws::note("labels.due.dirty", 1);
            return false;
        }
        if view.shown != self.placed {
            viewer_foundation::map::redraws::note("labels.due.placed", 1);
            return false;
        }
        if view.projected != self.report {
            viewer_foundation::map::redraws::note("labels.due.projection", 1);
            return false;
        }
        view.pools.iter().enumerate().all(|(index, pool)| {
            pool.stands_still(context)
                && pool.kept_windows().all(|(owner, slot)| {
                    view.written[index].get(&owner).is_some_and(|written| {
                        written.slot == slot
                            && view.labels.get(&owner).is_some_and(|&at| {
                                written.frame == self.placed[at].frame
                                    && written.text == self.placed[at].text
                            })
                    })
                })
        })
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use viewer_foundation::map::free_icons::{IconAnchor, IconFamily};

    /// A label standing nowhere in particular, for draws that do not test placement.
    const UNANCHORED: IconAnchor = IconAnchor {
        world: [0.0; 3],
        cutoff: f32::INFINITY,
        family: IconFamily::Labels,
        moves: None,
    };
    use viewer_foundation::map::native_resources;

    use viewer_foundation::map::test_support::localisation::loaded;

    /// The labels laid out once stand where each camera puts them, with the dot and the name cut
    /// off at their own distances and the refusals counted for that camera alone.
    #[test]
    fn one_hour_s_labels_are_placed_for_each_camera_and_report_only_that_camera_s_refusals() {
        use viewer_foundation::map::camera::{MapDefines, MapSize, Placement, WATER_HEIGHT};
        let candidate = |province: u32, x: f32, dot: f32, text: f32, name: Result<Option<Arc<str>>, String>| Candidate {
            province,
            world: [x, WATER_HEIGHT, 1000.0],
            capital: false,
            frame: 1,
            priority: 5,
            dot,
            text,
            name,
        };
        let mut labels = PreparedLabels {
            candidates: vec![
                candidate(1, 2000.0, 400.0, 200.0, Ok(Some(Arc::from("Rome")))),
                candidate(2, 2100.0, 100.0, 80.0, Ok(Some(Arc::from("Milan")))),
                candidate(3, 2200.0, 400.0, 200.0, Err("no name".to_owned())),
            ],
            ..Default::default()
        };
        let at = |height: f32| LabelProjection {
            camera: Placement::map([2100.0, 1000.0], height).on(MapSize::TEST, MapDefines::test()),
            viewport: [2560, 1440],
            ui_scale: 1.0,
        };
        // Near: every dot, every name, and the one refused name counted once.
        labels.project(&at(60.0));
        assert_eq!(labels.placed.len(), 3);
        assert_eq!(labels.placed.iter().filter(|label| label.text.is_some()).count(), 2);
        assert_eq!(labels.report.refused_names, 1);
        assert_eq!(labels.report.refused_localisation["no name"][&3], 1);
        // Projected again at the same height: the report stands for this projection, not both.
        let screens: Vec<[f32; 2]> = labels.placed.iter().map(|label| label.screen).collect();
        labels.project(&at(60.0));
        assert_eq!(labels.report.refused_names, 1, "the report is this camera's, never a sum");
        assert_eq!(labels.placed.iter().map(|label| label.screen).collect::<Vec<_>>(), screens);
        // Higher: Milan's dot and every name are gone, so nothing is refused.
        labels.project(&at(300.0));
        assert_eq!(labels.placed.iter().map(|label| label.province).collect::<Vec<_>>(), vec![1, 3]);
        assert!(labels.placed.iter().all(|label| label.text.is_none()));
        assert_eq!(labels.report.refused_names, 0);
        // Higher still: no label stands at all.
        labels.project(&at(500.0));
        assert!(labels.placed.is_empty());
        // Moving the camera moves every label and keeps what each shows.
        labels.project(&at(60.0));
        let near = labels.placed.iter().map(|label| (label.province, label.text.clone())).collect::<Vec<_>>();
        let mut moved = at(60.0);
        moved.camera.center[0] += 40.0;
        labels.project(&moved);
        assert_eq!(labels.placed.iter().map(|label| (label.province, label.text.clone())).collect::<Vec<_>>(), near);
        assert!(
            labels.placed.iter().zip(&screens).all(|(label, before)| label.screen[0] != before[0]),
            "a pan moves every label on screen"
        );
    }
    #[test]
    fn label_candidates_keep_province_order_and_chosen_capitals() {
        use replay_data::rows::{
            CountryLinkRow, CountryRow, PoliticsRow, ProvinceRow, StateRow, TableRows,
        };
        use viewer_foundation::map::test_support::{test_frame, test_put};
        let mut frame = test_frame(1);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Politics(vec![PoliticsRow {
                country: 1,
                capital: 10,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::States(vec![StateRow {
                id: 10,
                owner: 1,
                controller: 1,
                ..Default::default()
            }]),
        );
        // The capital state 10 holds province 12 (2 points) and province 13 (`tied`).
        let provinces = |capital_points: i32, tied: i32| {
            TableRows::Provinces(
                (0..5000)
                    .map(|index| ProvinceRow {
                        id: index + 1,
                        state: if index == 11 || index == 12 { 10 } else { 0 },
                        victory_points: match index {
                            7 | 4999 => 1,
                            11 => capital_points,
                            12 => tied,
                            _ => 0,
                        },
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        test_put(&mut frame, provinces(2, 1));
        test_put(
            &mut frame,
            TableRows::ProvinceNames(vec![
                replay_data::rows::ProvinceNameRow {
                    province: 8,
                    name: b"VICTORY_POINTS_678".to_vec(),
                },
                replay_data::rows::ProvinceNameRow {
                    province: 12,
                    name: b"First name".to_vec(),
                },
                replay_data::rows::ProvinceNameRow {
                    province: 12,
                    name: b"Last name\0ignored".to_vec(),
                },
            ]),
        );
        let tables = LabelTables::read(&frame, 1, &synthetic_labels()).unwrap();
        assert_eq!(tables.capitals, BTreeSet::from([12]));
        assert_eq!(tables.candidates, [7, 11, 12, 4999]);
        assert_eq!(
            tables.names[&8].as_ref().unwrap().as_deref(),
            Some("Port Haven")
        );
        assert_eq!(
            tables.names[&12].as_ref().unwrap().as_deref(),
            Some("Last name")
        );
        assert!(
            tables.names[&5000].is_err(),
            "a missing demanded owner stays a cached refusal"
        );
        let cloned = tables.clone();
        assert!(Arc::ptr_eq(
            tables.names[&8].as_ref().unwrap().as_ref().unwrap(),
            cloned.names[&8].as_ref().unwrap().as_ref().unwrap()
        ));
        // A tie needs the unrecorded state order, and no points the unrecorded anchor.
        let links = BTreeMap::from([(1, 1)]);
        for (capital_points, tied, why) in [(2, 2, "tie"), (0, 0, "none of its provinces")] {
            test_put(&mut frame, provinces(capital_points, tied));
            let tables = LabelTables::read(&frame, 1, &synthetic_labels()).unwrap();
            assert!(tables.capitals.is_empty());
            let withheld = withheld_capitals(&frame, &links).unwrap();
            assert_eq!(withheld.len(), 1);
            assert_eq!(withheld[0].0, 10);
            assert!(withheld[0].1.contains(why), "{withheld:?}");
        }
    }

    #[test]
    fn occupied_capital_uses_the_independent_state_controller() {
        use replay_data::{
            rows::PoliticsRow, rows::ProvinceRow, rows::StateRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    state: 10,
                    owner: 100,
                    controller: 100,
                    victory_points: 5,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    state: 10,
                    owner: 100,
                    controller: 200,
                    victory_points: 10,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
            PoliticsRow {
                country: 100,
                capital: 10,
                ..Default::default()
            },
            PoliticsRow {
                country: 200,
                capital: 20,
                ..Default::default()
            },
        ]));
        let links = BTreeMap::from([(100, 1), (101, 1), (200, 2)]);
        let states = |controller| {
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 10,
                owner: 100,
                controller,
                ..Default::default()
            }]))
        };
        *frame.tables[TableId::States.index()] = states(200);
        assert!(capital_provinces(&frame, &links).unwrap().is_empty());
        *frame.tables[TableId::States.index()] = states(101);
        assert_eq!(
            capital_provinces(&frame, &links).unwrap(),
            BTreeSet::from([2])
        );
        *frame.tables[TableId::States.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        assert!(capital_provinces(&frame, &links).unwrap().is_empty());
    }

    #[test]
    fn native_labels_draw_at_their_anchors_in_depth_order_across_both_pools() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = gui::fixture();
        let windows = gui::windows(
            br#"guiTypes = {
            containerWindowType = { name = victory_point_mapicon size = { width = 40 height = 20 }
                iconType = { name = icon spriteType = GFX_even }
                instantTextBoxType = { name = text font = Main_14 } }
            containerWindowType = { name = capital_mapicon size = { width = 40 height = 20 }
                iconType = { name = icon spriteType = GFX_even }
                instantTextBoxType = { name = text font = Main_14 } }
        }"#,
        );
        let context = native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [400, 400],
            time: Some(0.),
        };
        let labels = PreparedLabels {
            placed: vec![
                Label {
                    province: 1,
                    screen: [100., 100.],
                    depth: 0.1,
                    capital: false,
                    frame: 1,
                    priority: 5,
                    text: None,
                    anchors: [UNANCHORED; 2],
                },
                Label {
                    province: 2,
                    screen: [200., 200.],
                    depth: 0.9,
                    capital: true,
                    frame: 1,
                    priority: 5,
                    text: None,
                    anchors: [UNANCHORED; 2],
                },
            ],
            ..Default::default()
        };
        let mut view = LabelView::default();
        labels.draw(&context, &Schedule::default(), &mut view);
        let report = view.report().clone();
        assert_eq!(report.icons, 2, "{:?}", report.refused_gui);
        assert_eq!(view.drawn().quads.len(), 2);
        let corners: Vec<_> = view
            .drawn()
            .quads
            .iter()
            .map(|quad| quad.exact.as_ref().unwrap().corners)
            .collect();
        assert_eq!(
            corners[0],
            [[200., 190.], [220., 190.], [220., 200.], [200., 200.]]
        );
        assert_eq!(
            corners[1],
            [[100., 90.], [120., 90.], [120., 100.], [100., 100.]]
        );
        assert!(view.drawn().quads[0].layer < view.drawn().quads[1].layer);

        // The borrowed-worker path snapshots dirty bits before any GUI/resource work.
        // Compare complete batches/reports from independent retained pools, not a redraw
        // of one already-mutated view.
        let workers = viewer_foundation::parallel::Workers::new(Arc::new(
            viewer_foundation::builder_pool::Pool::new(1).unwrap(),
        ));
        let dirty = labels.dirty(&Schedule::default());
        let outputs: [_; 2] = std::array::from_fn(|_| std::sync::Mutex::new(None));
        workers.run(2, &|index| {
            let mut view = LabelView::default();
            if index == 0 {
                labels.draw(&context, &Schedule::default(), &mut view);
            } else {
                labels.draw_scheduled(&context, &dirty, &mut view);
            }
            *outputs[index].lock().unwrap() =
                Some((view.drawn().clone(), view.report().clone()));
        });
        let [
            (serial_batch, serial_report),
            (parallel_batch, parallel_report),
        ] = outputs.map(|output| output.into_inner().unwrap().unwrap());
        assert_eq!(serial_report, parallel_report);
        assert_eq!(
            format!("{:?}", serial_batch.quads),
            format!("{:?}", parallel_batch.quads)
        );
    }

    #[test]
    fn labels_that_stood_still_are_kept_and_an_off_screen_dirty_capital_lays_out_its_window_alone() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = gui::fixture();
        let windows = gui::windows(
            br#"guiTypes = {
            containerWindowType = { name = victory_point_mapicon size = { width = 40 height = 20 }
                iconType = { name = icon spriteType = GFX_even }
                instantTextBoxType = { name = text font = Main_14 } }
            containerWindowType = { name = capital_mapicon size = { width = 40 height = 20 }
                iconType = { name = icon spriteType = GFX_even }
                instantTextBoxType = { name = text font = Main_14 } }
        }"#,
        );
        let context = native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [400, 400],
            time: Some(0.),
        };
        let label = |province, screen, capital, frame, text: Option<&str>| Label {
            province,
            screen,
            depth: province as f32,
            capital,
            frame,
            priority: 5,
            text: text.map(Arc::from),
            anchors: [UNANCHORED; 2],
        };
        let hour = |berlin: &str| PreparedLabels {
            placed: vec![
                label(1, [100., 100.], false, 1, Some(berlin)),
                label(2, [200., 200.], false, 1, None),
                label(3, [300., 150.], true, 1, Some("Paris")),
                // Off screen: requested and erased on every frame, kept off screen while dirty.
                label(4, [-5000., 150.], true, 1, None),
            ],
            ..Default::default()
        };
        // (Berlin's name, the off-screen capital's province dirty, why each pool laid out)
        let frames = [
            ("Berlin", false, [Some("no frame presented yet"), Some("no frame presented yet")]),
            ("Berlin", false, [None, None]),
            ("Potsdam", false, [Some("an owner's offer changed"), None]),
            ("Potsdam", false, [None, None]),
            ("Potsdam", true, [None, Some("a dirty province kept an icon off screen")]),
            ("Potsdam", true, [None, None]),
            ("Potsdam", false, [None, None]),
        ];
        let mut view = LabelView::default();
        for (at, (berlin, dirty, unkept)) in frames.into_iter().enumerate() {
            let labels = hour(berlin);
            let dirty = [false, false, false, dirty];
            labels.draw_scheduled(&context, &dirty, &mut view);
            assert_eq!(view.pools.each_ref().map(PoolView::unkept), unkept, "frame {at}");
            // The same hour drawn by a view with no history.
            let mut fresh = LabelView::default();
            labels.draw_scheduled(&context, &dirty, &mut fresh);
            assert_eq!(view.report(), fresh.report(), "frame {at}");
            assert_eq!(
                format!("{:?}", view.drawn().quads),
                format!("{:?}", fresh.drawn().quads),
                "frame {at}"
            );
        }
        // The same hour and camera again: nothing is due, so nothing is drawn and what stands is
        // what a view with no history draws.
        let labels = hour("Potsdam");
        let dirty = [false; 4];
        labels.draw_scheduled(&context, &dirty, &mut view);
        assert!(labels.nothing_due(&context, &dirty, &view), "nothing is due");
        let mut fresh = LabelView::default();
        labels.draw_scheduled(&context, &dirty, &mut fresh);
        assert_eq!(view.report(), fresh.report());
        assert_eq!(
            format!("{:?}", view.drawn().quads),
            format!("{:?}", fresh.drawn().quads)
        );
    }

    #[test]
    fn a_label_s_name_stands_on_its_own_anchor_and_its_dot_on_the_label_s() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = gui::fixture();
        let windows = gui::windows(
            br#"guiTypes = {
            containerWindowType = { name = victory_point_mapicon size = { width = 40 height = 20 }
                iconType = { name = icon spriteType = GFX_even }
                instantTextBoxType = { name = text font = Main_14 } }
        }"#,
        );
        let context = native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [400, 400],
            time: Some(0.),
        };
        let at = |cutoff| IconAnchor {
            world: [100.0, 10.0, 200.0],
            cutoff,
            family: IconFamily::Labels,
            moves: None,
        };
        let labels = PreparedLabels {
            placed: vec![Label {
                province: 1,
                screen: [100., 100.],
                depth: 0.1,
                capital: false,
                frame: 1,
                priority: 5,
                text: Some("Berlin".into()),
                anchors: [at(400.0), at(250.0)],
            }],
            ..Default::default()
        };
        let mut view = LabelView::default();
        labels.draw(&context, &Schedule::default(), &mut view);
        let report = view.report().clone();
        assert_eq!(
            (report.icons, report.texts),
            (1, 1),
            "{:?}",
            report.refused_gui
        );
        let cutoffs: Vec<f32> = view
            .drawn()
            .quads
            .iter()
            .map(|quad| {
                quad.anchor
                    .expect("every label quad stands somewhere")
                    .cutoff
            })
            .collect();
        // The dot first, then the name's glyphs.
        assert_eq!(cutoffs[0], 400.0);
        assert!(
            cutoffs.len() > 1 && cutoffs[1..].iter().all(|cutoff| *cutoff == 250.0),
            "{cutoffs:?}"
        );
        // Later pushes are screen interface again.
        let mut batch = view.drawn().clone();
        batch.push(batch.quads[0].clone());
        assert_eq!(batch.quads.last().unwrap().anchor, None);
    }

    #[test]
    fn dump_icons_past_the_initial_pool_all_draw() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use viewer_foundation::map::icon_windows::DumpFrames;
        let resources = gui::fixture();
        let windows = gui::windows(
            br#"guiTypes = {
            containerWindowType = { name = victory_point_mapicon size = { width = 40 height = 20 }
                iconType = { name = icon spriteType = GFX_even }
                instantTextBoxType = { name = text font = Main_14 } }
        }"#,
        );
        let context = native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [400, 400],
            time: Some(12.),
        };
        let count = MapIconPool::Label.initial() + 2 * 16 + 1;
        let labels = PreparedLabels {
            placed: (1..=count)
                .map(|province| Label {
                    province: province as u32,
                    screen: [100., 100.],
                    depth: province as f32,
                    capital: false,
                    frame: 1,
                    priority: 5,
                    text: None,
                    anchors: [UNANCHORED; 2],
                })
                .collect(),
            ..Default::default()
        };
        let mut view = LabelView::default();
        let mut frames = DumpFrames::default();
        let mut presentations = 0;
        loop {
            labels.draw(&context, &Schedule::default(), &mut view);
            presentations += 1;
            if !frames.next(&view.retries()) {
                break;
            }
        }
        let report = view.report().clone();
        assert_eq!(report.icons, count, "{:?}", report.refused_gui);
        assert_eq!(view.drawn().quads.len(), count);
        assert_eq!(
            presentations, 2,
            "one growth builds a window for each of the 33 missed requests, which ask again once"
        );
        assert!(view.retries().is_empty());
    }

    #[test]
    fn missing_label_templates_are_named_and_counted_per_family() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = gui::fixture();
        let windows = gui::windows(b"");
        let context = native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [100, 100],
            time: Some(0.),
        };
        let labels = PreparedLabels {
            placed: [false, true, true]
                .map(|capital| Label {
                    province: 1,
                    screen: [50.0; 2],
                    capital,
                    depth: 0.,
                    frame: 1,
                    priority: 0,
                    text: None,
                    anchors: [UNANCHORED; 2],
                })
                .into(),
            ..Default::default()
        };
        let mut view = LabelView::default();
        labels.draw(&context, &Schedule::default(), &mut view);
        let report = view.report();
        assert_eq!(
            report.refused_gui["victory_point_mapicon is not drawn: no GUI template is registered"],
            1
        );
        assert_eq!(
            report.refused_gui["capital_mapicon is not drawn: no GUI template is registered"],
            2
        );
    }

    #[test]
    fn the_shape_group_follows_the_victory_point_levels() {
        // A capital is a star whatever it is worth.
        assert_eq!(shape_group(true, 0, &LabelDefines::test()), 0);
        assert_eq!(shape_group(true, 50, &LabelDefines::test()), 0);
        assert_eq!(shape_group(false, 50, &LabelDefines::test()), 1);
        assert_eq!(shape_group(false, 20, &LabelDefines::test()), 1);
        assert_eq!(shape_group(false, 19, &LabelDefines::test()), 2);
        assert_eq!(shape_group(false, 9, &LabelDefines::test()), 2);
        assert_eq!(shape_group(false, 8, &LabelDefines::test()), 3);
        assert_eq!(shape_group(false, 1, &LabelDefines::test()), 3);
    }

    /// Small synthetic localisation dictionary for key, literal, owner override,
    /// nested expansion and byte-limit cases.
    fn synthetic_labels() -> Localisation {
        loaded(
            &(String::from(concat!(
                "l_english:\n",
                " VICTORY_POINTS_678:0 \"Port Haven\"\n",
                " GER_VICTORY_POINTS_678:0 \"Owner Haven\"\n",
                " VICTORY_POINTS_9503:0 \"River Gate\"\n",
                " synthetic_waterwork:0 \"Sample Waterwork\"\n",
                " synthetic_bridge:0 \"Sample Bridge\"\n",
                " VICTORY_POINTS_1:0 \"Łake [Upper Shore] $$1\"\n",
                " VICTORY_POINTS_2:0 \"$VICTORY_POINTS_1$ – $NOT_A_KEY$Nord\"\n",
                " VICTORY_POINTS_3:0 \"a",
            )) + &"ł".repeat(0xC00)
                + "\"\n"),
        )
    }

    #[test]
    fn refused_names_reach_the_warning_area() {
        assert!(LabelReport::default().warnings().is_empty());
        let report = LabelReport {
            refused_names: 3,
            refused_localisation: BTreeMap::from([(
                "localisation VICTORY_POINTS_3: its text is not UTF-8".to_owned(),
                BTreeMap::from([(692, 1), (6521, 2)]),
            )]),
            ..Default::default()
        };
        assert_eq!(
            report.warnings(),
            vec![
                "2 victory-point names are not drawn: localisation VICTORY_POINTS_3: its text is not UTF-8; provinces 692, 6521",
            ]
        );
    }

    #[test]
    fn the_raid_mode_keeps_only_capitals_and_the_resource_mode_draws_no_names() {
        use crate::map::map_mode::{MapMode, Stock, visibility};
        let of = |stock| visibility::of(&MapMode::Stock(stock));
        let political = of(Stock::Default);
        assert_eq!(mode_label(false, &political), Some(true));
        assert_eq!(mode_label(true, &political), Some(true));
        let raids = of(Stock::Raids);
        assert_eq!(mode_label(false, &raids), None);
        assert_eq!(mode_label(true, &raids), Some(true));
        let resources = of(Stock::Resources);
        assert_eq!(mode_label(false, &resources), Some(false));
        assert_eq!(mode_label(true, &resources), Some(false));
    }

    #[test]
    fn a_mod_with_more_levels_gets_more_shapes_and_frames() {
        let defines = LabelDefines {
            after: vec![0.0, 5.0, 10.0, 20.0],
            ..LabelDefines::test()
        };
        assert_eq!(shape_group(false, 25, &defines), 1);
        assert_eq!(shape_group(false, 12, &defines), 2);
        assert_eq!(shape_group(false, 1, &defines), 4);
    }

    #[test]
    fn an_occupied_dot_takes_its_controllers_colour() {
        // Warsaw for the Soviet player: owner Poland (101, slot 1, an enemy),
        // controller the Soviet Union (201, slot 2, ours).
        let links = BTreeMap::from([(101, 1), (201, 2)]);
        let allegiances = BTreeMap::from([(1, Allegiance::Enemy), (2, Allegiance::Ours)]);
        assert_eq!(dot_allegiance(201, &links, &allegiances), Allegiance::Ours);
        assert_eq!(dot_allegiance(101, &links, &allegiances), Allegiance::Enemy);
        assert_eq!(dot_allegiance(0, &links, &allegiances), Allegiance::Other);
    }

    #[test]
    fn the_frame_index_walks_shapes_slowest_and_colours_fastest() {
        assert_eq!(frame_index(0, Allegiance::Enemy), 0);
        assert_eq!(frame_index(0, Allegiance::Other), 4);
        assert_eq!(frame_index(1, Allegiance::Enemy), 5);
        // Budapest: 20 points, Hungarian, a faction partner of ours and grey.
        assert_eq!(
            frame_index(
                shape_group(false, 20, &LabelDefines::test()),
                Allegiance::Other
            ),
            9
        );
        // Berlin: a German capital with Germany the player, green star.
        assert_eq!(
            frame_index(
                shape_group(true, 50, &LabelDefines::test()),
                Allegiance::Ours
            ),
            2
        );
        // København: a capital of a country that is not ours.
        assert_eq!(
            frame_index(
                shape_group(true, 20, &LabelDefines::test()),
                Allegiance::Other
            ),
            4
        );
        // Praha: 30 points, owner not ours, grey large dot.
        assert_eq!(
            frame_index(
                shape_group(false, 30, &LabelDefines::test()),
                Allegiance::Other
            ),
            9
        );
        // The last frame of the strip is the small grey square.
        assert_eq!(
            frame_index(
                shape_group(false, 1, &LabelDefines::test()),
                Allegiance::Other
            ),
            19
        );
    }

    #[test]
    fn text_and_dot_cutoffs_keep_their_distinct_ranges() {
        let reference = 310.178;
        // Malmö, 5 points: neither the dot nor the text survives.
        assert!(dot_cutoff(false, 5, &LabelDefines::test()) < reference);
        assert!(text_cutoff(false, 5, &LabelDefines::test()) < reference);
        // Kiel and Hannover, 10 points: both do.
        assert!(dot_cutoff(false, 10, &LabelDefines::test()) > reference);
        assert!(text_cutoff(false, 10, &LabelDefines::test()) > reference);
        // Frankfurt, 20 points.
        assert!(text_cutoff(false, 20, &LabelDefines::test()) > reference);
        // Luxembourg, 5 points but a capital.
        assert!(dot_cutoff(true, 5, &LabelDefines::test()) > reference);
        assert!(text_cutoff(true, 5, &LabelDefines::test()) > reference);
        // The ranges end where the defines put them.
        assert_eq!(dot_cutoff(false, 0, &LabelDefines::test()), 100.0);
        assert_eq!(dot_cutoff(false, 22, &LabelDefines::test()), 1000.0);
        assert_eq!(dot_cutoff(false, 99, &LabelDefines::test()), 1000.0);
        assert_eq!(text_cutoff(false, 22, &LabelDefines::test()), 800.0);
        // The engine does not clamp the text range: 50 points reach past its top.
        assert!((text_cutoff(false, 50, &LabelDefines::test()) - 1690.909).abs() < 0.01);
        assert_eq!(dot_cutoff(true, 0, &LabelDefines::test()), 1100.0);
        assert_eq!(text_cutoff(true, 0, &LabelDefines::test()), 1000.0);
    }

    /// Victory-point localisation uses the state owner's original tag or tag,
    /// independently of dynamic naming gates and province controller.
    #[test]
    fn the_victory_point_tag_is_the_state_owners_original_tag() {
        use replay_data::{
            rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::Recorded,
            rows::ReservedCountryNamingRow, rows::StateRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                [(100, 1), (200, 2), (300, 3)]
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .into(),
            ));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [(100, 1, "GER"), (200, 2, "D01"), (300, 3, "POL")]
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.into(),
                    ..Default::default()
                })
                .into(),
        ));
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![]));
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            [(10, 100), (20, 200), (30, 999), (31, 0)]
                .map(|(id, owner)| StateRow {
                    id,
                    owner,
                    ..Default::default()
                })
                .into(),
        ));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(
                [(100, ""), (200, "SOV")]
                    .map(|(country, exact_tag)| CountryAppearanceRow {
                        country,
                        original_tag_reference: Recorded::Value(Some(if exact_tag.is_empty() {
                            0
                        } else {
                            1
                        })),
                        original_tag: Recorded::Value(Some(exact_tag.into())),
                        ..Default::default()
                    })
                    .into(),
            ));
        let tables = LabelTables::read(&frame, 0, &synthetic_labels()).unwrap();
        assert!(tables.candidates.is_empty());
        assert_eq!(tables.victory_point_tag(10), Ok("GER"));
        // A dynamic country takes its original tag.
        assert_eq!(tables.victory_point_tag(20), Ok("SOV"));
        assert!(
            tables
                .victory_point_tag(0)
                .unwrap_err()
                .contains("ReservedCountryNaming")
        );
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"ZERO".to_vec(),
                    original_tag_reference: Recorded::Value(Some(1)),
                    original_tag: Recorded::Value(Some(b"ZERO_ORIGINAL".to_vec())),
                    ..Default::default()
                },
            ]));
        let tables = LabelTables::read(&frame, 0, &synthetic_labels()).unwrap();
        // Both a null state and a known-zero state owner select the reserved
        // object's exact getter, without admitting it to the positive roster.
        assert_eq!(tables.victory_point_tag(0), Ok("ZERO_ORIGINAL"));
        assert_eq!(tables.victory_point_tag(31), Ok("ZERO_ORIGINAL"));
        let error = tables.victory_point_tag(30).unwrap_err();
        assert!(error.contains("owner link 999 of state 30"), "{error}");
        let error = tables.victory_point_tag(40).unwrap_err();
        assert!(
            error.starts_with("the owner of state 40 (States.owner) is not recorded"),
            "{error}"
        );
        *frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        let tables = LabelTables::read(&frame, 0, &synthetic_labels()).unwrap();
        assert_eq!(
            tables.victory_point_tag(10),
            Err("CountryAppearance.original_tag of slot 1 is not recorded: the table is unavailable: not sampled".into())
        );
        assert_eq!(tables.victory_point_tag(0), Ok("ZERO_ORIGINAL"));
        // The tag feeds the ladder only without a usable override.
        let loc = synthetic_labels();
        let tag = tables.victory_point_tag(10);
        assert!(tag.is_err());
        assert_eq!(
            label_name(&loc, Some(b"VICTORY_POINTS_678"), None, 678),
            Ok(Some("Port Haven".into()))
        );
    }

    #[test]
    fn the_text_key_probes_the_owner_override_first() {
        assert_eq!(
            victory_point_keys(Some("GER"), 692),
            ["GER_VICTORY_POINTS_692", "VICTORY_POINTS_692"]
        );
        assert_eq!(victory_point_keys(None, 6521), ["VICTORY_POINTS_6521"]);
        assert_eq!(victory_point_keys(Some(""), 6521), ["VICTORY_POINTS_6521"]);
    }

    #[test]
    fn a_recorded_override_is_a_key_when_the_localisation_has_one_and_text_when_it_does_not() {
        let loc = synthetic_labels();
        let name = |recorded: &[u8], tag| label_name(&loc, Some(recorded), tag, 678);
        // A key: the entry wins, exactly as the game resolves it, and
        // the owner-qualified `GER_VICTORY_POINTS_678` rung is never reached.
        assert_eq!(
            name(b"VICTORY_POINTS_678", Some("GER")),
            Ok(Some("Port Haven".into()))
        );
        assert_eq!(
            name(b"synthetic_waterwork", None),
            Ok(Some("Sample Waterwork".into()))
        );
        // Not a key: the bytes are the text, the way the lookup copies its own
        // input back out when the dictionary probe misses at depth 0.
        assert_eq!(
            name("Port \u{144} Name".as_bytes(), None),
            Ok(Some("Port \u{144} Name".into()))
        );
        // No override: the owner's key first, then the bare one.
        assert_eq!(
            label_name(&loc, None, Some("GER"), 678),
            Ok(Some("Owner Haven".into()))
        );
        assert_eq!(
            label_name(&loc, Some(b""), Some("FRA"), 678),
            Ok(Some("Port Haven".into()))
        );
        assert_eq!(label_name(&loc, None, Some("GER"), 5), Ok(None));
    }

    /// The label text is the game's localiser's: `$$` is one `$`, brackets
    /// are glyphs (the text box runs no scripted-text pass), a nested key the
    /// content lacks writes nothing, and the 0x1800-byte buffer refuses a
    /// cut inside a two-byte character by name.
    #[test]
    fn a_label_name_is_what_the_games_localiser_makes_of_it() {
        let loc = synthetic_labels();
        assert_eq!(
            label_name(&loc, None, None, 1),
            Ok(Some("Łake [Upper Shore] $1".into()))
        );
        assert_eq!(
            label_name(&loc, None, Some("POL"), 2),
            Ok(Some("Łake [Upper Shore] $1 – Nord".into()))
        );
        let refusal = label_name(&loc, None, None, 3).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "localisation VICTORY_POINTS_3: the game's 0x1800-byte buffer ends inside a multi-byte character"
        );
    }

    #[test]
    fn an_override_is_measured_as_a_c_string_and_never_guessed_at() {
        let loc = synthetic_labels();
        let name = |recorded: &[u8]| label_name(&loc, Some(recorded), None, 9503);
        // The lookup measures to the first NUL, so the tail is not part of it.
        assert_eq!(
            name(b"VICTORY_POINTS_678\0ignored"),
            Ok(Some("Port Haven".into()))
        );
        // One that begins with a NUL is an override with no name at all:
        // A first NUL byte produces empty output.
        assert_eq!(name(b"\0ignored"), Ok(Some(String::new())));
        assert!(!unreadable(b""));
        assert!(!unreadable(b"\0ignored"));
        // Bytes that are not text: the static name is kept and the province is
        // named in the limitation instead of being drawn as replacement marks.
        assert_eq!(name(&[0xff, 0xfe, b'a']), Ok(Some("River Gate".into())));
        assert!(unreadable(&[0xff, 0xfe, b'a']));
        assert!(!unreadable("Port \u{144} Name".as_bytes()));
    }
}
