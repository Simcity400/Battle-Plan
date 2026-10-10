//! Faction-theatre pins, one per admitted theatre of the viewing country's faction.
//!
//! The first theatre region owns its pin. Political, terrain and supply map modes
//! and the supported theatre mode can show it; hidden-country entries compare raw
//! tags or canonical country slots. Fog, camera height and armies are not gates.
//! The count uses the recorded theatre country count.
//!
//! Pins live while a province of their first region is in the shared update schedule.
//! Camera admission, dirty bits and selection can retain an off-camera pin through
//! the rotating province phase. Selection cannot add a clean off-camera province.
//! Each presented frame refreshes reached owners and erases unreached owners.
//!
//! The anchor is an f32 arithmetic mean of the theatre regions' icon positions,
//! without seam handling. Pins have no grouping layer. Resolved window resources
//! supply position, size, sprite, frame and font; the recorded emblem appends
//! [`EMBLEM_SUFFIX`]. Interface pixels use [`super::super::ui_scale`]. Pins retain
//! [`PING_PRIORITY`] across map modes and nearer pins draw over farther ones.
//! Hover highlights the union of theatre regions and leaving clears it, without
//! a tooltip. Pointer state belongs to the view.

use replay::rows::FactionTheatreRow;

use super::*;
use crate::map::native_gui;

const COUNT: &str = "count";
const FACTION_ICON: &str = "faction_icon";
// The hover and leave callbacks attach to this button.
const BUTTON: &[u8] = b"button";

pub const FACTION_PING_WINDOW: &str = "faction_ping_map_icon";
/// Append this to the emblem name.
pub const EMBLEM_SUFFIX: &str = "_miniature";
/// Faction-ping priority: the game leaves the constructed value unchanged.
pub const PING_PRIORITY: i8 = 0;
/// moves `x` by whole map widths until it is within half a width (integer
/// division) of `reference`.
fn unwrap(reference: i32, x: i32, width: i32) -> i32 {
    if width <= 0 {
        return x;
    }
    let (reference, width) = (i64::from(reference), i64::from(width));
    let mut x = i64::from(x);
    while (reference - x).abs() > width / 2 {
        x += if reference - x >= 0 { width } else { -width };
    }
    x as i32
}

/// Region icon position is the arithmetic mean of its province icon positions.
/// Terrain sampling supplies its height; missing region geometry stays unavailable.
pub fn region_icon_position(
    centroids: &[[i32; 2]],
    width: i32,
    height: impl Fn(f32, f32) -> f32,
) -> Option<[f32; 3]> {
    let reference = centroids.first()?[0];
    let (mut x, mut z) = (0.0f32, 0.0f32);
    for &[cx, cz] in centroids {
        x += unwrap(reference, cx, width) as f32;
        z += cz as f32;
    }
    let count = centroids.len() as f32;
    let (mut x, z) = (x / count, z / count);
    let span = width as f32;
    if x < 0.0 {
        x += span;
    } else if x >= span {
        x -= span;
    }
    Some([x, height(x, z), z])
}

/// the `f32` sum of the theatre's region
/// positions, in theatre order, times `1 / count`. There is no seam handling.
pub fn theatre_anchor(positions: &[[f32; 3]]) -> Option<[f32; 3]> {
    if positions.is_empty() {
        return None;
    }
    let inverse = 1.0f32 / positions.len() as f32;
    let mut sum = [0.0f32; 3];
    for position in positions {
        for (total, value) in sum.iter_mut().zip(position) {
            *total += value;
        }
    }
    Some(sum.map(|total| total * inverse))
}

/// One pin the hour and perspective call for, before the camera's admission.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pin {
    /// The theatre's first region, whose listed provinces keep the pin.
    pub(crate) first_region: u32,
    pub(crate) regions: Arc<[u32]>,
    pub(crate) anchor: [f32; 3],
    /// The emblem sprite name.
    pub(crate) emblem: Vec<u8>,
    /// The count box's text.
    pub(crate) count: String,
}

/// What kept pins of this hour from being decided or placed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinReport {
    /// Why no theatre could be decided at all, when none could.
    pub undecided: Option<String>,
    /// Theatres whose first region another theatre of the faction also lists: which theatre
    /// the byte map names there is not recorded, so neither pin is drawn.
    pub ambiguous: usize,
    /// Theatres naming a region the content has no icon position for.
    pub unplaced: usize,
    /// `hidden_for` links `CountryLinks` does not resolve; they hide nothing.
    pub unresolved_hidden: usize,
}

impl PinReport {
    /// `*self = source.clone()`, reusing this report's storage.
    pub fn assign(&mut self, source: &Self) {
        let Self {
            undecided,
            ambiguous,
            unplaced,
            unresolved_hidden,
        } = source;
        self.undecided.clone_from(undecided);
        self.ambiguous = *ambiguous;
        self.unplaced = *unplaced;
        self.unresolved_hidden = *unresolved_hidden;
    }
}

/// The camera-independent pins of one hour and perspective.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HourPins {
    pub(crate) pins: Vec<Pin>,
    pub(crate) report: PinReport,
}

/// Admitted theatres for the viewing country, in recorded theatre order.
/// The first region owns the pin; a nonpositive region id is refused.
pub(crate) fn shown<'a>(
    theatres: &'a [FactionTheatreRow],
    faction: u64,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    report: &mut PinReport,
) -> Vec<&'a FactionTheatreRow> {
    let mut own: Vec<&FactionTheatreRow> = theatres
        .iter()
        .filter(|row| row.faction == faction)
        .collect();
    own.sort_by_key(|row| row.ordinal);
    let mut claims: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for row in &own {
        for region in &row.regions {
            claims.entry(*region).or_default().insert(row.ordinal);
        }
    }
    let mut out = Vec::new();
    for row in own {
        if i8::try_from(row.ordinal).is_err() {
            continue;
        }
        let Some(&first) = row.regions.first() else {
            continue;
        };
        if first == 0 {
            continue;
        }
        if claims.get(&first).is_some_and(|owners| owners.len() > 1) {
            report.ambiguous += 1;
            continue;
        }
        let mut hidden = false;
        for raw in &row.hidden_for {
            match links.get(raw) {
                Some(&slot) => hidden |= slot == viewer,
                None => report.unresolved_hidden += 1,
            }
        }
        if !hidden {
            out.push(row);
        }
    }
    out
}

impl HourPins {
    /// The pins of one hour for the viewing country `viewer` (a canonical slot).
    pub(crate) fn read(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        viewer: u32,
        content: &CounterContent,
    ) -> io::Result<Self> {
        let mut report = PinReport::default();
        let theatres = rows::<tables::FactionTheatres>(frame)?;
        if let Available::Unavailable { reason } = theatres {
            report.undecided = Some(format!("FactionTheatres is unavailable: {reason}"));
            return Ok(Self {
                pins: Vec::new(),
                report,
            });
        }
        let members = rows::<tables::FactionMembers>(frame)?;
        let faction = members
            .rows()
            .iter()
            .find(|row| links.get(&row.country) == Some(&viewer))
            .map(|row| row.faction);
        let Some(faction) = faction else {
            if !members.is_complete() {
                report.undecided = Some(
                    "FactionMembers is incomplete and holds no row for the viewing country".into(),
                );
            }
            return Ok(Self {
                pins: Vec::new(),
                report,
            });
        };
        let mut pins = Vec::new();
        for row in shown(theatres.rows(), faction, viewer, links, &mut report) {
            let positions: Option<Vec<[f32; 3]>> = row
                .regions
                .iter()
                .map(|region| content.region_icon(*region))
                .collect();
            let Some(anchor) = positions.as_deref().and_then(theatre_anchor) else {
                report.unplaced += 1;
                continue;
            };
            let mut emblem = row.icon.clone();
            emblem.extend_from_slice(EMBLEM_SUFFIX.as_bytes());
            pins.push(Pin {
                first_region: row.regions[0],
                regions: row.regions.clone().into(),
                anchor,
                emblem,
                count: (row.country_count as i32).to_string(),
            });
        }
        Ok(Self { pins, report })
    }
}

/// A camera's pin owners; preparation never touches the presented pool.
#[derive(Default)]
pub(super) struct PreparedPins {
    placed: Vec<PlacedPin>,
}

struct PlacedPin {
    depth: f32,
    region: u32,
    regions: Arc<[u32]>,
    screen: [f32; 2],
    emblem: Vec<u8>,
    count: String,
}

/// All hour candidates projected for this camera; the shared province list decides
/// owner admission on each presented frame, not while this overlay is prepared.
pub(super) fn faction_pings(inputs: &CounterInputs<'_>, hour: &HourPins, out: &mut CounterFrame) {
    out.report.faction_ping_limits = hour.report.clone();
    for pin in &hour.pins {
        let world = pin.anchor;
        let projection = inputs.camera.project_map_icon(
            world,
            inputs.viewport,
            gui_windows::viewport(inputs.viewport, inputs.ui_scale),
        );
        out.native_pings.placed.push(PlacedPin {
            depth: projection.depth,
            region: pin.first_region,
            regions: Arc::clone(&pin.regions),
            screen: projection.position,
            emblem: pin.emblem.clone(),
            count: pin.count.clone(),
        });
    }
    out.native_pings
        .placed
        .sort_by(|a, b| b.depth.total_cmp(&a.depth));
}

/// A successfully drawn pin's native button, in interface pixels and paint order.
#[derive(Clone, Debug, PartialEq)]
pub struct PinHover {
    pub rect: [f32; 4],
    pub regions: Arc<[u32]>,
    pub layer: u64,
}

impl PinHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        crate::combat::contains(self.rect, point)
    }
}

fn hover_rect(
    root: &native::Element,
    layout: &native_gui::Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
) -> Result<Option<[f32; 4]>, String> {
    let button = root
        .find_recursive(native::Kind::Button, BUTTON)
        .ok_or("no button named button")?;
    let control = button.button().expect("button lookup");
    if button.hide || control.always_transparent || !control.enabled {
        return Ok(None);
    }
    let Some(part) = layout.parts.iter().find(|part| part.path == button.path) else {
        return Ok(None);
    };
    let native_gui::PartKind::Sprite {
        sprite, geometry, ..
    } = &part.kind
    else {
        return Err("button hover needs a native sprite frame".into());
    };
    if geometry.rotation != 0.0 {
        return Err("button rotated hover placement unavailable".into());
    }
    let frame = resources
        .sprite(sprite)
        .ok_or("button hover sprite unavailable")?;
    match frame.hit {
        native_gui::Hit::Through => return Ok(None),
        native_gui::Hit::Alpha => return Err("button hover needs native texture alpha".into()),
        native_gui::Hit::Frame => {}
    }
    let mut rect = geometry.rect;
    rect[0] += origin[0] as f32;
    rect[1] += origin[1] as f32;
    if let Some([left, top, right, bottom]) = part.clip {
        let right = (rect[0] + rect[2]).min((origin[0] + right) as f32);
        let bottom = (rect[1] + rect[3]).min((origin[1] + bottom) as f32);
        rect[0] = rect[0].max((origin[0] + left) as f32);
        rect[1] = rect[1].max((origin[1] + top) as f32);
        rect[2] = right - rect[0];
        rect[3] = bottom - rect[1];
    }
    Ok((rect[2] > 0.0 && rect[3] > 0.0).then_some(rect))
}

fn refresh(
    instance: &mut crate::map::native_gui::lifecycle::Instance,
    root: &native::Element,
    text: &str,
    sprite: &[u8],
    resources: &dyn crate::map::native_gui::Resources,
) -> Result<(), String> {
    // Find inherited controls recursively, then write the theatre count and emblem.
    let count = root
        .find_recursive(native::Kind::TextBox, COUNT.as_bytes())
        .ok_or_else(|| format!("no text box named {COUNT}"))?;
    let emblem = root
        .find_recursive(native::Kind::Icon, FACTION_ICON.as_bytes())
        .ok_or_else(|| format!("no icon named {FACTION_ICON}"))?;
    instance.set_text(root, count, text, resources)?;
    if !native_gui::sprite_resolves(sprite, resources) {
        return Err(format!(
            "faction emblem {} cannot replace {}",
            native_gui::lossy(sprite),
            emblem.path
        ));
    }
    instance.set_sprite(
        root,
        emblem,
        sprite,
        crate::map::native_gui::SpriteSetter::Icon,
    )
}

/// A province cleared by this frame's off-screen update remains listed for
/// this frame's region call; the next frame can erase the region's pin.
fn region_listed(provinces: &[u32], schedule: &crate::map::province_schedule::Schedule) -> bool {
    provinces.iter().any(|&province| schedule.listed(province))
}

impl PreparedPins {
    pub(super) fn draw(
        &self,
        inputs: &CounterInputs<'_>,
        time: f32,
        countries: Option<&Countries>,
        batch: &mut Batch,
        report: &mut CounterReport,
    ) {
        use crate::map::{
            icon_windows::{Owner, Plan},
            native_resources::Context,
        };
        let resources = gui_windows::GuiAssets::with_countries(inputs, countries);
        let context = Context {
            resources: &resources,
            windows: inputs.windows,
            viewport: gui_windows::viewport(inputs.viewport, inputs.ui_scale),
            time: Some(time),
        };
        let reached = {
            let schedule = inputs
                .presentation
                .schedule()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.placed
                .iter()
                .filter(|pin| region_listed(inputs.content.region_provinces(pin.region), &schedule))
                .collect::<Vec<_>>()
        };
        // Only the next-frame request decision needs camera admission here. Actual owners
        // come solely from the shared list. Include the final off-screen cleanup frame so
        // the following presentation can erase its region's pool owner at a still camera.
        let admission = Admission::new(inputs.camera, inputs.viewport);
        let keep_running = reached.iter().any(|pin| {
            !inputs
                .content
                .region_provinces(pin.region)
                .iter()
                .any(|&province| admission.admits_province(&inputs.content.bounds, province))
        });
        // Preserve the existing depth request order; native sort fidelity is separate.
        let owners: Vec<_> = reached
            .iter()
            .map(|pin| Owner {
                id: pin.region,
                screen: pin.screen,
                dirty: false,
            })
            .collect();
        let pins: std::collections::HashMap<_, _> = self
            .placed
            .iter()
            .enumerate()
            .map(|(ordinal, pin)| (pin.region, (ordinal, pin)))
            .collect();
        let mut views = inputs
            .presentation
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut drawn = views.pings.present(
            &context,
            &Plan {
                name: FACTION_PING_WINDOW,
                kind: native_gui::lifecycle::MapIconPool::Ping,
                alignment: 0x12,
                owners: &owners,
            },
            Box::new(|_, _| Ok(())),
            |owner, _, instance, root| {
                let pin = pins[&owner].1;
                refresh(instance, root, &pin.count, &pin.emblem, context.resources)
            },
            &mut report.pin_gui_refusals,
        );
        if keep_running {
            views.pings.keep_running();
        }
        drawn.sort_by_key(|drawn| pins[&drawn.owner].0);
        for drawn in drawn {
            let ordinal = pins[&drawn.owner].0;
            let root = inputs
                .windows
                .guis
                .template(FACTION_PING_WINDOW)
                .expect("drawn template");
            // A refused native hit shape cannot draw an observer that would let
            // pointer input reach a lower map icon through its visible button.
            let hover = match hover_rect(root, &drawn.layout, drawn.origin, context.resources) {
                Ok(hover) => hover,
                Err(why) => {
                    *report
                        .pin_gui_refusals
                        .entry(format!("{FACTION_PING_WINDOW} hover: {why}"))
                        .or_default() += 1;
                    continue;
                }
            };
            batch.at_layer(layer(PING_PRIORITY, ordinal as u32, 0));
            match native_gui::draw(&drawn.layout, drawn.origin, context.resources, batch) {
                Ok(()) => {
                    report.faction_pings += 1;
                    if let Some(rect) = hover {
                        report.faction_pin_hovers.push(PinHover {
                            rect,
                            regions: Arc::clone(&pins[&drawn.owner].1.regions),
                            layer: layer(PING_PRIORITY, ordinal as u32, 0),
                        });
                    }
                }
                Err(why) => gui_windows::count_refused(
                    &mut report.pin_gui_refusals,
                    FACTION_PING_WINDOW,
                    &[why],
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_owner_lingers_through_cleanup_frame_then_erases_at_still_camera() {
        let mut schedule = crate::map::province_schedule::Schedule::default();
        schedule.advance([3], |_| true, false);
        assert!(region_listed(&[3], &schedule));
        schedule.advance([3], |_| false, false);
        assert!(region_listed(&[3], &schedule));
        schedule.advance([3], |_| false, false);
        assert_eq!(
            schedule.update(3),
            Some(crate::map::province_schedule::Update::OffScreen)
        );
        assert!(region_listed(&[3], &schedule));
        schedule.advance([3], |_| false, false);
        assert!(!region_listed(&[3], &schedule));
    }

    #[test]
    fn selected_region_uses_shared_list_without_reviving_clean_offscreen_province() {
        let mut schedule = crate::map::province_schedule::Schedule::default();
        schedule.advance([1, 88], |id| id == 88, false);
        schedule.take_pending();
        schedule.set_selected(BTreeSet::from([88]));
        for _ in 0..101 {
            schedule.advance([1, 88], |_| false, false);
            assert!(region_listed(&[88], &schedule));
            assert!(!region_listed(&[1], &schedule));
            schedule.take_pending();
        }
        schedule.set_selected(BTreeSet::from([1, 88]));
        schedule.advance([1, 88], |_| false, false);
        assert!(!region_listed(&[1], &schedule));
        assert!(region_listed(&[88], &schedule));
        assert!(!region_listed(&[], &schedule));
    }

    #[test]
    fn one_listed_province_keeps_region_without_requiring_all_members() {
        let mut schedule = crate::map::province_schedule::Schedule::default();
        schedule.advance([1, 2, 99], |id| id == 99, true);
        schedule.advance([1, 2, 99], |id| id != 1, false);
        assert!(region_listed(&[99, 1, 2], &schedule));
        assert!(!region_listed(&[1], &schedule));
    }

    #[test]
    fn pin_hover_distinguishes_refused_alpha_from_native_pass_through() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::map::native_gui::lifecycle::Instance;
        let mut resources = native_gui::tests::fixture();
        let gui = native::read(
            br#"guiTypes = { containerWindowType = {
            name = faction_ping_map_icon size = { width = 100 height = 50 }
            buttonType = { name = button quadTextureSprite = GFX_even }
        } }"#,
        );
        let root = gui.template(FACTION_PING_WINDOW).unwrap();
        let mut instance = Instance::construct(root, [200, 100], &resources).unwrap();
        instance.gui_show(root, root).unwrap();
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        assert!(
            hover_rect(root, &layout, [10, 20], &resources)
                .unwrap()
                .is_some()
        );
        resources.sprites.get_mut("GFX_even").unwrap().hit = native_gui::Hit::Alpha;
        assert!(
            hover_rect(root, &layout, [10, 20], &resources)
                .unwrap_err()
                .contains("alpha")
        );
        resources.sprites.get_mut("GFX_even").unwrap().hit = native_gui::Hit::Through;
        assert_eq!(
            hover_rect(root, &layout, [10, 20], &resources).unwrap(),
            None
        );
    }

    #[test]
    fn pin_refresh_uses_native_nested_inherited_controls_and_text() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::map::native_gui::{self, PartKind, lifecycle::Instance};
        let resources = native_gui::tests::fixture();
        let gui=native::read(br#"guiTypes = { containerWindowType = {
          name = faction_ping_map_icon size = { width = 100 height = 50 }
          containerWindowType = { name = nested size = { width = 50% height = 50% }
            iconType = { name = base spriteType = GFX_even position = { x = -4 y = -3 } orientation = lower_right }
            iconType = { name = faction_icon parent = base }
            instantTextboxType = { name = count font = Main_14 maxWidth = 50 maxHeight = 20 }
          }
        } }"#);
        let root = gui.template(FACTION_PING_WINDOW).unwrap();
        let mut instance = Instance::construct(root, [200, 100], &resources).unwrap();
        instance.gui_show(root, root).unwrap();
        refresh(&mut instance, root, "23", b"GFX_odd", &resources).unwrap();
        let emblem = root
            .find_recursive(native::Kind::Icon, FACTION_ICON.as_bytes())
            .unwrap();
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        let part = layout
            .parts
            .iter()
            .find(|part| part.path == emblem.path)
            .unwrap();
        let PartKind::Sprite {
            sprite, geometry, ..
        } = &part.kind
        else {
            panic!("emblem sprite");
        };
        assert_eq!(sprite, b"GFX_odd");
        // Ignore parent; this icon keeps its defaults.
        assert_eq!(&geometry.rect[..2], &[0., 0.]);
        let text = root
            .find_recursive(native::Kind::TextBox, COUNT.as_bytes())
            .unwrap();
        assert_eq!(
            instance.control(root, text).unwrap().text.as_deref(),
            Some("23")
        );
        let why = refresh(
            &mut instance,
            root,
            "24",
            b"GFX_missing_miniature",
            &resources,
        )
        .unwrap_err();
        assert!(why.contains("faction emblem GFX_missing_miniature"));
        assert!(why.contains("faction_icon"));
    }

    fn theatre(ordinal: u32, regions: &[u32], hidden_for: &[u32]) -> FactionTheatreRow {
        FactionTheatreRow {
            faction: 7,
            ordinal,
            icon: b"GFX_faction_logo_x".to_vec(),
            regions: regions.to_vec(),
            country_count: 1,
            hidden_for: hidden_for.to_vec(),
        }
    }

    #[test]
    fn a_region_position_unwraps_against_the_first_province_and_wraps_once() {
        // Two provinces either side of the seam of a 100-wide map: 98 and 2 (unwrapped to 102)
        // average to 100, which wraps to 0.
        let at = region_icon_position(&[[98, 10], [2, 20]], 100, |x, z| x + z).unwrap();
        assert_eq!(at, [0.0, 15.0, 15.0]);
        // Without a seam the mean is plain, and a single province is its own centroid.
        assert_eq!(
            region_icon_position(&[[10, 4], [21, 5]], 100, |_, _| 9.5).unwrap(),
            [15.5, 9.5, 4.5]
        );
        assert_eq!(region_icon_position(&[], 100, |_, _| 0.0), None);
        // The reference is the first province, not the smallest x.
        assert_eq!(unwrap(2, 98, 100), -2);
        assert_eq!(unwrap(98, 2, 100), 102);
        assert_eq!(unwrap(50, 0, 100), 0, "exactly half a width stays");
    }

    #[test]
    fn a_theatre_anchor_is_the_plain_mean_with_no_seam_handling() {
        let anchor = theatre_anchor(&[[1.0, 2.0, 3.0], [99.0, 4.0, 5.0], [2.0, 0.0, 1.0]]);
        let third = 1.0f32 / 3.0;
        assert_eq!(
            anchor,
            Some([102.0 * third, 6.0 * third, 9.0 * third]),
            "sums times the f32 reciprocal, not a division"
        );
        assert_eq!(theatre_anchor(&[]), None);
    }

    #[test]
    fn only_the_viewers_unhidden_theatres_with_an_unshared_first_region_show() {
        let links = BTreeMap::from([(100, 1), (101, 2)]);
        let mut other = theatre(9, &[40], &[]);
        other.faction = 8;
        let rows = vec![
            theatre(2, &[5, 6], &[]),
            theatre(0, &[1, 2, 3], &[]),
            // Hidden for raw 101, which is slot 2, not the viewer.
            theatre(1, &[4], &[101, 555]),
            // Hidden for the viewer.
            theatre(3, &[8], &[100]),
            // No regions, and region 0.
            theatre(4, &[], &[]),
            theatre(5, &[0, 9], &[]),
            // First region 20 is also listed by theatre 7: ambiguous, both refused.
            theatre(6, &[20, 21], &[]),
            theatre(7, &[22, 20], &[]),
            // Native lookup cannot return this row through its signed byte.
            theatre(128, &[30], &[]),
            other,
        ];
        let mut report = PinReport::default();
        let shown: Vec<u32> = shown(&rows, 7, 1, &links, &mut report)
            .iter()
            .map(|row| row.ordinal)
            .collect();
        assert_eq!(shown, [0, 1, 2, 7]);
        assert_eq!(report.ambiguous, 1);
        assert_eq!(report.unresolved_hidden, 1);
    }
}
