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

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    gui::native,
    localisation::{Localisation, Unlocalised},
    map::{MapData, ProvinceKind},
    parser::c_string,
};
use replay::{frame::DecodedFrame, rows::tables};

use super::{
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
    /// `NMapIcons.DEFAULT_PRIORITY_VICTORY_POINTS` and `DEFAULT_PRIORITY_CAPITAL`.
    pub priority: [i8; 2],
}

impl LabelDefines {
    pub fn load(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        let number = |section: &str, key: &str| -> io::Result<f32> {
            let value = defines.number(section, key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.{section}.{key} is outside f32")))
            }
        };
        let graphics = |key: &str| number("NGraphics", key);
        let priority = |key: &str| super::sprites::priority(defines, key);
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
            priority: [
                priority("DEFAULT_PRIORITY_VICTORY_POINTS")?,
                priority("DEFAULT_PRIORITY_CAPITAL")?,
            ],
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
            priority: [5, 5],
        }
    }
}

/// Colour frames per icon shape; the game steps sizes by `* 5`.
const FRAMES_PER_SHAPE: u32 = 5;

/// Unit-stack type 38 `Victory point`, the label anchor,
/// read from the finalised unit-stack table ([`LabelContent::set_anchors`]).
pub const ANCHOR_STACK: u32 = super::counters::VICTORY_POINT;

pub const VICTORY_POINT_WINDOW: &str = "victory_point_mapicon";
pub const CAPITAL_WINDOW: &str = "capital_mapicon";

const ICON: &str = "icon";
const TEXT: &str = "text";

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
    pub fn load(defines: &installed_content::defines::Defines) -> io::Result<Self> {
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

/// [`LabelContent::set_anchors`]' table: each province's anchor, and the
/// provinces whose anchor is the seeded default, sorted.
pub(super) fn label_anchors(
    provinces: impl Iterator<Item = u32>,
    anchor: impl Fn(u32) -> Option<[f32; 3]>,
    seeded: impl Fn(u32) -> bool,
) -> (BTreeMap<u32, [f32; 3]>, Vec<u32>) {
    let mut anchors = BTreeMap::new();
    let mut fallbacks = Vec::new();
    for province in provinces.filter(|&province| province != 0) {
        if let Some(at) = anchor(province) {
            anchors.insert(province, at);
            if seeded(province) {
                fallbacks.push(province);
            }
        }
    }
    fallbacks.sort_unstable();
    (anchors, fallbacks)
}

/// Everything one frame of labels needs besides the decoded hour.
pub struct LabelInputs<'a> {
    pub content: &'a LabelContent,
    pub localisation: &'a Localisation,
    pub map: &'a MapData,
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

/// The camera-independent tables the labels read, built once per hour and
/// viewing country and reused by every camera's [`prepare`].
#[derive(Clone, Debug, Default)]
pub struct LabelTables {
    links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, String>,
    ours: u32,
    allegiances: BTreeMap<u32, Allegiance>,
    capitals: BTreeSet<u32>,
    /// State id to its raw owner link, from `States`.
    state_owners: BTreeMap<u32, u32>,
    /// Why a state the `States` rows lack has no known owner.
    states_gap: String,
    /// Canonical slot to exact original tag or tag bytes or its local refusal.
    /// Slot zero comes from the authenticated ReservedCountryNaming singleton.
    original_tags: BTreeMap<u32, Result<String, String>>,
    /// Why a country the `CountryAppearance` rows lack has no known one.
    appearance_gap: String,
}

/// Why a table's rows lack an entry: a complete table lacks it, a truncated
/// or unavailable one may not have reached it.
fn gap<T: replay::rows::TableSchema>(available: &Available<'_, T>) -> String {
    match available {
        Available::Complete(_) => "the complete table has no row for it".into(),
        Available::Truncated { missing, .. } => {
            format!("the table is truncated ({missing} rows missing)")
        }
        Available::Unavailable { reason } => format!("the table is unavailable: {reason}"),
    }
}

impl LabelTables {
    /// Reads one hour's links, tags, war sides and capitals for `ours`.
    pub fn read(frame: &DecodedFrame, ours: u32) -> io::Result<Self> {
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let countries = core::<tables::Countries>(frame)?;
        let by_slot: BTreeMap<_, _> = countries
            .iter()
            .map(|country| (country.canonical_slot, country))
            .collect();
        let tags: BTreeMap<u32, String> = countries
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect();
        let allegiances = allegiance::war_table(frame, &links, ours)?;
        let capitals = capital_provinces(frame, &links)?;
        let states = rows::<tables::States>(frame)?;
        let appearance = rows::<tables::CountryAppearance>(frame)?;
        let mut original_tags: BTreeMap<_, _> = appearance
            .rows()
            .iter()
            .filter_map(|row| {
                let slot = *links.get(&row.country)?;
                Some((
                    slot,
                    installed_content::country_names::original_tag(by_slot.get(&slot)?, row)
                        .map(str::to_owned),
                ))
            })
            .collect();
        original_tags.insert(
            0,
            installed_content::country_names::reserved_zero_original_tag(frame).map(str::to_owned),
        );
        Ok(Self {
            state_owners: states
                .rows()
                .iter()
                .map(|state| (state.id, state.owner))
                .collect(),
            states_gap: gap(&states),
            original_tags,
            appearance_gap: gap(&appearance),
            links,
            tags,
            ours,
            allegiances,
            capitals,
        })
    }

    /// State-owner tag operands used by victory-point localisation. The original
    /// tag and own tag remain separate recorded fields.
    fn victory_point_tag(&self, state: u32) -> Result<&str, String> {
        let slot = if state == 0 {
            0
        } else {
            let owner = *self.state_owners.get(&state).ok_or_else(|| {
                format!(
                    "the owner of state {state} (States.owner) is not recorded: {}",
                    self.states_gap
                )
            })?;
            if owner == 0 {
                0
            } else {
                *self.links.get(&owner).ok_or_else(|| {
                    format!("the owner link {owner} of state {state} resolves to no country")
                })?
            }
        };
        self.original_tags
            .get(&slot)
            .ok_or_else(|| {
                format!(
                    "CountryAppearance.original_tag of slot {slot} is not recorded: {}",
                    self.appearance_gap
                )
            })?
            .as_deref()
            .map_err(Clone::clone)
    }
}

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

/// Pushes one hour's victory-point and capital labels onto `batch`.
pub fn prepare(
    inputs: &LabelInputs<'_>,
    frame: &DecodedFrame,
    tables: &LabelTables,
) -> io::Result<PreparedLabels> {
    let defines = &inputs.content.defines;
    let provinces = core::<tables::Provinces>(frame)?;
    let LabelTables {
        links,
        tags,
        ours,
        allegiances,
        capitals,
        ..
    } = tables;
    // names: every recorded rename the hour holds. A truncated table's rows are
    // real recorded rows and are used; the rows it did not reach leave their
    // provinces on the static ladder, which `super::tables` states.
    let overrides = province_overrides(frame)?;

    let height = inputs.camera.height;
    let mut report = LabelReport {
        perspective: tags.get(ours).cloned(),
        ..Default::default()
    };
    let mut placed = Vec::new();
    for province in provinces {
        let Some(definition) = inputs.map.definitions.get(&province.id) else {
            continue;
        };
        if matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake) {
            continue;
        }
        let capital = capitals.contains(&province.id);
        let victory_points = province.victory_points;
        if victory_points <= 0 && !capital {
            continue;
        }
        if height >= dot_cutoff(capital, victory_points, defines) {
            continue;
        }
        let Some(anchor) = inputs.content.anchors.get(&province.id) else {
            continue;
        };
        let world = *anchor;
        let projection = inputs.camera.project_map_icon(
            world,
            inputs.viewport,
            super::counters::gui_windows::viewport(inputs.viewport, inputs.ui_scale),
        );
        let allegiance = dot_allegiance(province.controller, links, allegiances);
        let shape = shape_group(capital, victory_points, defines);
        let priority = defines.priority[usize::from(capital)];
        let name = if height >= text_cutoff(capital, victory_points, defines) {
            None
        } else {
            let recorded = overrides.get(&province.id).copied();
            // An override replaces the whole ladder, so the owner's
            // tag matters only without one.
            let tag = if recorded.is_some_and(overrides_ladder) {
                Ok(None)
            } else {
                tables.victory_point_tag(province.state).map(Some)
            };
            match tag.and_then(|tag| {
                label_name(inputs.localisation, recorded, tag, province.id)
                    .map_err(|refusal| refusal.to_string())
            }) {
                Ok(name) => name,
                Err(refusal) => {
                    report.refused_names += 1;
                    *report
                        .refused_localisation
                        .entry(refusal)
                        .or_default()
                        .entry(province.id)
                        .or_default() += 1;
                    None
                }
            }
        };
        placed.push(Label {
            province: province.id,
            screen: projection.position,
            depth: projection.depth,
            capital,
            frame: frame_index(shape, allegiance) as i32 + 1,
            priority,
            text: name,
        });
    }
    Ok(PreparedLabels { placed, report })
}

struct Label {
    province: u32,
    screen: [f32; 2],
    depth: f32,
    capital: bool,
    frame: i32,
    priority: i8,
    text: Option<String>,
}

/// A prepared hour's label values. Preparing never changes a presented window.
pub struct PreparedLabels {
    placed: Vec<Label>,
    pub report: LabelReport,
}

/// Native window histories and cached text; the name cache belongs to each reused slot.
#[derive(Debug, Default)]
pub struct LabelView {
    pools: [super::icon_windows::PoolView; 2],
    texts: [BTreeMap<SlotId, String>; 2],
}

impl LabelView {
    pub(crate) fn retries(&self) -> Vec<super::icon_windows::Retry> {
        self.pools
            .iter()
            .filter_map(super::icon_windows::PoolView::retry)
            .collect()
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
        context: &super::native_resources::Context<
            '_,
            impl native_gui::Resources + native_gui::Sheets,
        >,
        schedule: &super::province_schedule::Schedule,
        view: &mut LabelView,
        batch: &mut Batch,
    ) -> LabelReport {
        use super::icon_windows::{Owner, Plan};
        let mut report = self.report.clone();
        let mut drawn = Vec::new();
        for (index, name) in [VICTORY_POINT_WINDOW, CAPITAL_WINDOW]
            .into_iter()
            .enumerate()
        {
            if !view.pools[index].same(context) {
                view.texts[index].clear();
            }
            let mut labels = std::collections::HashMap::new();
            let mut owners = Vec::new();
            for label in self
                .placed
                .iter()
                .filter(|label| usize::from(label.capital) == index)
            {
                labels.insert(label.province, label);
                owners.push(Owner {
                    id: label.province,
                    screen: label.screen,
                    dirty: schedule.dirty(label.province),
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
            let layouts = view.pools[index].present(
                context,
                &Plan {
                    name,
                    kind: if index == 0 {
                        MapIconPool::Label
                    } else {
                        MapIconPool::Capital
                    },
                    alignment: 0x0A,
                    owners: &owners,
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
                    let label = labels[&owner];
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
                    Ok(())
                },
                &mut report.refused_gui,
            );
            drawn.extend(
                layouts
                    .into_iter()
                    .map(|drawn| (labels[&drawn.owner], drawn)),
            );
        }
        // Sort both label families by depth.
        drawn.sort_by(|a, b| b.0.depth.total_cmp(&a.0.depth));
        for (ordinal, (label, drawn)) in drawn.into_iter().enumerate() {
            batch.at_layer(sprites::layer(label.priority, ordinal as u32, 0));
            match native_gui::draw(&drawn.layout, drawn.origin, context.resources, batch) {
                Ok(()) => {
                    report.icons += 1;
                    report.texts +=
                        usize::from(!label.text.as_deref().unwrap_or_default().is_empty());
                }
                Err(why) => super::counters::gui_windows::count_refused(
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
        report
    }
}

/// Select the greatest victory-point value in the controller's current capital
/// state, breaking ties by lowest province id because recorded data lacks the
/// state's province order. Country/state fallback anchors are unrecorded, so
/// states without positive victory points also use the selected province.
/// Missing `Politics` or `States` rows cannot establish a capital; table
/// limitations report those gaps while ordinary labels remain available.
fn capital_provinces(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<BTreeSet<u32>> {
    let provinces = core::<tables::Provinces>(frame)?;
    let capitals: BTreeMap<u32, u32> = rows::<tables::Politics>(frame)?
        .rows()
        .iter()
        .filter(|politics| politics.capital > 0)
        .filter_map(|politics| {
            links
                .get(&politics.country)
                .map(|&slot| (slot, politics.capital as u32))
        })
        .collect();
    // A capital icon is created only when the state controller has this state
    // as its current capital. `States` records the controller independently.
    let wanted: BTreeSet<u32> = rows::<tables::States>(frame)?
        .rows()
        .iter()
        .filter(|state| {
            links
                .get(&state.controller)
                .and_then(|slot| capitals.get(slot))
                == Some(&state.id)
        })
        .map(|state| state.id)
        .collect();
    let mut best: BTreeMap<u32, (i32, u32)> = BTreeMap::new();
    for province in provinces {
        if !wanted.contains(&province.state) {
            continue;
        }
        let candidate = (province.victory_points, province.id);
        best.entry(province.state)
            .and_modify(|current| {
                if candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
                {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    Ok(best.into_values().map(|(_, province)| province).collect())
}

/// Try `<TAG>_VICTORY_POINTS_<id>` before `VICTORY_POINTS_<id>`, using the
/// same owner-override pattern as state names. The caller takes the first
/// key its localisation has.
pub fn victory_point_keys(tag: Option<&str>, province: u32) -> Vec<String> {
    match tag.filter(|tag| !tag.is_empty()) {
        Some(tag) => vec![
            format!("{tag}_VICTORY_POINTS_{province}"),
            format!("VICTORY_POINTS_{province}"),
        ],
        None => vec![format!("VICTORY_POINTS_{province}")],
    }
}

/// The recorded `ProvinceNames` rows of one hour, by province.
///
/// The table is sparse: a province with no row has no override, which is the
/// whole meaning of absence **from a complete table**. Absence from a truncated
/// or unavailable one means nothing at all, and this function cannot tell the
/// two apart — nor does it have to. Either way the province falls through to the
/// static ladder and keeps the name it had; what changes is whether the hour is
/// entitled to be silent about it, and that is `super::tables::limitations`'
/// judgement, not this one's.
///
/// A later row wins over an earlier one for the same province, which is the only
/// reading a table keyed by province admits.
fn province_overrides(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, &[u8]>> {
    Ok(rows::<tables::ProvinceNames>(frame)?
        .rows()
        .iter()
        .map(|row| (row.province, row.name.as_slice()))
        .collect())
}

/// A recorded rename replaces the static key ladder. Localise it as a key,
/// falling back to its literal bytes after the first-NUL cut. Otherwise use
/// the first declared key from [`victory_point_keys`], or draw no name if none
/// exists. Label text keeps brackets as glyphs.
pub fn label_name(
    localisation: &Localisation,
    recorded: Option<&[u8]>,
    tag: Option<&str>,
    province: u32,
) -> Result<Option<String>, Unlocalised> {
    if let Some(bytes) = recorded.filter(|bytes| overrides_ladder(bytes)) {
        return localisation.localise(bytes).map(Some);
    }
    let Some(key) = victory_point_keys(tag, province)
        .into_iter()
        .find(|key| localisation.get(key).is_some())
    else {
        return Ok(None);
    };
    localisation.localise(key.as_bytes()).map(Some)
}

/// Whether a recorded override replaces the static ladder here: a non-empty
/// one the port can read.
fn overrides_ladder(bytes: &[u8]) -> bool {
    !bytes.is_empty() && !unreadable(bytes)
}

/// Whether a recorded override is bytes this port cannot draw as text.
///
/// Only a non-UTF-8 string qualifies: an empty one and one that starts with a
/// NUL are names the *engine* does not draw either, so they are not a shortfall
/// of the port and are not reported as one.
fn unreadable(bytes: &[u8]) -> bool {
    let key = c_string(bytes);
    !key.is_empty() && std::str::from_utf8(key).is_err()
}

/// The provinces whose recorded override this port cannot lay out, for the
/// `Field ProvinceNames.name` limitation line.
pub fn unreadable_overrides(frame: &DecodedFrame) -> io::Result<Vec<u32>> {
    let mut out: Vec<u32> = province_overrides(frame)?
        .into_iter()
        .filter(|(_, bytes)| unreadable(bytes))
        .map(|(province, _)| province)
        .collect();
    out.sort_unstable();
    Ok(out)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::map::native_resources;

    #[test]
    fn occupied_capital_uses_the_independent_state_controller() {
        use replay::{
            frame::{Metrics, TableView},
            rows::{PoliticsRow, ProvinceRow, StateRow, TableId, TableRows},
        };
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = native_gui::tests::fixture();
        let windows = native_gui::tests::windows(
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
            report: Default::default(),
            placed: vec![
                Label {
                    province: 1,
                    screen: [100., 100.],
                    depth: 0.1,
                    capital: false,
                    frame: 1,
                    priority: 5,
                    text: None,
                },
                Label {
                    province: 2,
                    screen: [200., 200.],
                    depth: 0.9,
                    capital: true,
                    frame: 1,
                    priority: 5,
                    text: None,
                },
            ],
        };
        let mut view = LabelView::default();
        let mut batch = Batch::default();
        let report = labels.draw(
            &context,
            &super::super::province_schedule::Schedule::default(),
            &mut view,
            &mut batch,
        );
        assert_eq!(report.icons, 2, "{:?}", report.refused_gui);
        assert_eq!(batch.quads.len(), 2);
        let corners: Vec<_> = batch
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
        assert!(batch.quads[0].layer < batch.quads[1].layer);
    }

    #[test]
    fn dump_icons_past_the_initial_pool_all_draw() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use super::super::icon_windows::DumpFrames;
        let resources = native_gui::tests::fixture();
        let windows = native_gui::tests::windows(
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
            report: Default::default(),
            placed: (1..=count)
                .map(|province| Label {
                    province: province as u32,
                    screen: [100., 100.],
                    depth: province as f32,
                    capital: false,
                    frame: 1,
                    priority: 5,
                    text: None,
                })
                .collect(),
        };
        let mut view = LabelView::default();
        let mut frames = DumpFrames::default();
        let mut batch = Batch::default();
        let mut presentations = 0;
        let report = loop {
            batch.clear();
            let report = labels.draw(
                &context,
                &super::super::province_schedule::Schedule::default(),
                &mut view,
                &mut batch,
            );
            presentations += 1;
            if !frames.next(&view.retries()) {
                break report;
            }
        };
        assert_eq!(report.icons, count, "{:?}", report.refused_gui);
        assert_eq!(batch.quads.len(), count);
        assert_eq!(
            presentations, 4,
            "33 missed requests take three growth/re-ask rounds"
        );
        assert!(view.retries().is_empty());
    }

    #[test]
    fn missing_label_templates_are_named_and_counted_per_family() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = native_gui::tests::fixture();
        let windows = native_gui::tests::windows(b"");
        let context = native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [100, 100],
            time: Some(0.),
        };
        let labels = PreparedLabels {
            report: LabelReport::default(),
            placed: [false, true, true]
                .map(|capital| Label {
                    province: 1,
                    screen: [50.0; 2],
                    capital,
                    depth: 0.,
                    frame: 1,
                    priority: 0,
                    text: None,
                })
                .into(),
        };
        let report = labels.draw(
            &context,
            &super::super::province_schedule::Schedule::default(),
            &mut LabelView::default(),
            &mut Batch::default(),
        );
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

    /// Localisation as the game content loads it: `yml` is one file of a
    /// folder layer, read by `installed_content::localisation::read`.
    pub(crate) fn loaded(yml: &str) -> Localisation {
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_label_names_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("localisation/victory_points_l_english.yml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("\u{feff}{yml}")).unwrap();
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let localisation = installed_content::localisation::read(&layers, "english").unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        localisation
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
        use replay::{
            frame::{Metrics, TableView},
            rows::{
                CountryAppearanceRow, CountryLinkRow, CountryRow, Recorded,
                ReservedCountryNamingRow, StateRow, TableId, TableRows,
            },
        };
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
        let tables = LabelTables::read(&frame, 0).unwrap();
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
        let tables = LabelTables::read(&frame, 0).unwrap();
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
        let tables = LabelTables::read(&frame, 0).unwrap();
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
