//! The map-icon windows this pass lays out and draws through [`crate::map::native_gui`]: the
//! naval counter (`unit_counter_navy`), the naval combat icon (`naval_combat_mapicon`) and its
//! convoy addition (`GFX_mapicon_tiny_unit_counter`). Each window is one lifecycle instance
//! ([`native_gui::lifecycle::Instance`], kept by [`super::naval_windows::Windows`]); what the
//! game's code does to it on an update is recorded here as its calls ([`Writes`], made without
//! any instance, so a prepared hour never touches the presented view's windows), each child found
//! by the lookup the game's binder uses, and applied once by the presented frame. Nothing is
//! placed or refused per widget here.

use super::*;
use std::collections::HashMap;
use viewer_foundation::map::gui_resources::GuiAssets;
use viewer_foundation::map::gui_resources::GuiFlags;
use viewer_foundation::map::gui_resources::HitMode;
use viewer_foundation::map::gui_resources::observer_hit_rectangle;
use viewer_foundation::map::gui_resources::{HitRectangle, refused_cause};
use viewer_foundation::map::native_resources::GuiResources;
use viewer_foundation::unit_handle::Unit;

use viewer_foundation::map::native_flags;
use viewer_foundation::map::native_gui::{
    self, Layout, Texture,
    lifecycle::{Call, Calls, Instance},
};

/// Every resource the map-icon windows draw: template assets and authored inline icons
/// ([`NativeWindows::assets`]), plus runtime-selected sprites resolved through `registry`.
/// Undefined names leave the current drawable: convoy transport ([`CONVOY_ADDITION_ICON`]),
/// result sprites ([`naval_markers::NAVAL_RESULT_SPRITES`]), carrier glyphs
/// ([`counter_order::AIR_BASE_ICON`], [`counter_order::ROCKET_SITE_ICON`]) and hull sprites
/// matching [`templates::UNIT_PREFIX`] + name + [`templates::UNIT_SUFFIX`], including
/// [`NO_INTEL_SPRITE`].
pub(super) fn window_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &installed_content::localisation::Localisation,
) -> native_gui::Assets {
    let mut assets = windows.assets(localisation);
    assets.sprites.extend(
        [
            CONVOY_ADDITION_ICON,
            counter_order::AIR_BASE_ICON,
            counter_order::ROCKET_SITE_ICON,
            NO_INTEL_SPRITE,
        ]
        .into_iter()
        .chain(naval_markers::NAVAL_RESULT_SPRITES)
        .filter(|name| registry.sprite(name.as_bytes()).is_some())
        .map(|name| name.as_bytes().to_vec()),
    );
    let (prefix, suffix) = (
        templates::UNIT_PREFIX.as_bytes(),
        templates::UNIT_SUFFIX.as_bytes(),
    );
    assets
        .sprites
        .extend(sprites_with_prefix(registry, prefix, suffix));
    // Facility selection uses the facility sprite prefix.
    assets.sprites.extend(sprites_with_prefix(
        registry,
        FACILITY_SPRITE_PREFIX.as_bytes(),
        b"",
    ));
    // Terrain selection appends its name to the combat background button's sprite.
    if let Some(background) = windows
        .gui_type(LAND_COMBAT_WINDOW.as_bytes())
        .and_then(|root| root.find(native::Kind::Button, "bg_btn"))
        && let native::Fields::Button(button) = &background.fields
    {
        let mut prefix = button.sprite.clone();
        prefix.push(b'_');
        assets
            .sprites
            .extend(sprites_with_prefix(registry, &prefix, b""));
    }
    // Faction icon selection appends _miniature to the recorded icon name.
    assets
        .sprites
        .extend(sprites_with_prefix(registry, b"", EMBLEM_SUFFIX.as_bytes()));
    assets
}

fn sprites_with_prefix<'a>(
    registry: &'a native::resources::Registry,
    prefix: &'a [u8],
    suffix: &'a [u8],
) -> impl Iterator<Item = Vec<u8>> + 'a {
    registry
        .files
        .iter()
        .flat_map(|(_, file)| &file.sprites)
        .filter(move |sprite| sprite.name.starts_with(prefix) && sprite.name.ends_with(suffix))
        .map(|sprite| sprite.name.clone())
}

/// The windows' resources, with no hour's countries: a flag is refused by that operand.
pub(super) fn gui_assets<'a>(
    inputs: &CounterInputs<'a>,
    flags: Option<&'a WindowFlags<'a>>,
) -> GuiAssets<'a> {
    let [width, height] = inputs.viewport.map(i32::try_from);
    GuiAssets {
        gui: &inputs.assets.gui,
        windows: inputs.windows,
        localisation: inputs.localisation,
        flags: flags.map(|flags| GuiFlags {
            source: flags,
            small: flags.small,
            large: flags.large,
        }),
        physical_viewport: width
            .ok()
            .zip(height.ok())
            .map(|(width, height)| [width, height]),
    }
}

/// The windows' resources with the hour's `countries`, which its flags resolve through.
pub(super) fn window_flags<'a>(
    inputs: &CounterInputs<'a>,
    countries: Option<&'a Countries>,
) -> Option<WindowFlags<'a>> {
    countries.map(|countries| WindowFlags {
        countries,
        small: &inputs.assets.flags,
        large: &inputs.assets.large_flags,
    })
}

/// Recorded country data and content flag art for [`native_flags::FlagSource`]. Canonical
/// slots identify country tags. Small and large atlas records hold each country's current
/// flag, selected by tag, cosmetic tag and ruling ideology ([`Countries::flag_now`]) before
/// the frame draws, matching atlases built in full at load.
pub struct WindowFlags<'a> {
    pub(super) countries: &'a Countries,
    pub small: &'a LazyFlags,
    pub large: &'a LazyFlags,
}

impl native_flags::FlagSource for WindowFlags<'_> {
    fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String> {
        let tag = std::str::from_utf8(tag).map_err(|_| {
            format!(
                "tag {:?} is not UTF-8, and the recording's tags are text",
                String::from_utf8_lossy(tag)
            )
        })?;
        self.countries
            .tags
            .iter()
            .find(|(_, own)| own.as_str() == tag)
            .map(|(&slot, _)| Some(slot))
            .ok_or_else(|| {
                format!(
                    "tag {tag} has no recorded country: whether the country database \
                     holds it, and its slot, are not recorded"
                )
            })
    }

    fn first_tag_slot(&self) -> Result<u32, String> {
        Err("the country database's first tag is not recorded".to_owned())
    }

    fn atlas_len(&self, _: bool) -> Result<i32, String> {
        let len = self
            .countries
            .tags
            .keys()
            .next_back()
            .map_or(0, |&slot| slot + 1);
        i32::try_from(len).map_err(|_| format!("{len} country slots exceed the atlas's count"))
    }

    fn atlas_record(&self, small: bool, index: i32) -> Result<native_flags::FlagRecord, String> {
        let atlas = if small { self.small } else { self.large };
        let slot = u32::try_from(index).map_err(|_| format!("flag record {index}"))?;
        if !self.countries.tags.contains_key(&slot) {
            return Err(format!(
                "country slot {slot}'s tag is not recorded, so its flag record is unknown"
            ));
        }
        Ok(match self.countries.flag_now(atlas, slot) {
            Some((texture, [u0, v0, u1, v1])) => native_flags::FlagRecord::Cell {
                coords: [u0, v0, u1 - u0, v1 - v0],
                page: Some(Texture {
                    texture,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    size: atlas.cell_size().map(|value| value as i32),
                }),
            },
            None => {
                if let Some(reason) = self.countries.flag_refusal(slot) {
                    return Err(reason);
                }
                // No file of a fully known candidate chain reads: invalid cell.
                native_flags::FlagRecord::Invalid
            }
        })
    }
}

/// Draws a laid-out window with the items its code added to its holders
/// ([`native_gui::draw_composed`]) at the whole-pixel `origin`, under `(priority, icon)`: every
/// quad is exact and keeps its push order in the icon's one layer, which is the game's attach
/// order. `Err` is the refusal, the window drawn not at all.
pub(super) fn draw_window(
    layout: &Layout,
    items: &[Vec<native_gui::Part>],
    origin: [i32; 2],
    resources: &dyn native_gui::Sheets,
    batch: &mut Batch,
    (priority, icon): (i8, u32),
) -> io::Result<Result<(), native_gui::Refusal>> {
    let first = batch.quads.len();
    batch.at_layer(0);
    if let Err(refusal) = native_gui::draw_composed(layout, items, origin, resources, batch) {
        return Ok(Err(refusal));
    }
    rank_children(batch, first, priority, icon).map(Ok)
}

/// Draws the item windows the code added to `holder` of a window laid out at the whole-pixel
/// `origin` (each already placed with [`native_gui::Holder::place`]) under `(priority, icon)`,
/// without the window's own parts, which another pass draws: the holder's items alone, composed
/// as [`native_gui::draw_composed`] composes them. `Err` is the refusal, nothing drawn.
pub(super) fn draw_held(
    holder: &native_gui::Holder,
    parts: Vec<native_gui::Part>,
    origin: [i32; 2],
    resources: &dyn native_gui::Sheets,
    batch: &mut Batch,
    key: (i8, u32),
) -> io::Result<Result<(), native_gui::Refusal>> {
    let held = Layout {
        size: [0; 2],
        parts: Vec::new(),
        holders: vec![native_gui::Holder {
            before: 0,
            ..holder.clone()
        }],
        diagnostics: Vec::new(),
        regions: Vec::new(),
    };
    draw_window(&held, &[parts], origin, resources, batch, key)
}

/// One update's ordered calls ([`Call`]), produced without an instance. Visibility, hide
/// bits and frames remain separate; other writes merge into one [`Call::Control`] per child.
/// [`Call::SetSprite`] resolves against the current drawable and retains it for undefined
/// names. [`Call::SetText`] refreshes a text box's scrollbar immediately. Every control call
/// replaces just the fields its updater writes; untouched controls retain their state.
/// Children resolve by `window.find(kind, name)`; an absent or differently typed child
/// receives no effective write.
pub(super) struct Writes<'w> {
    window: &'w native::Element,
    calls: Vec<(String, Call)>,
    controls: Vec<(String, native_gui::Bound)>,
    wrote: Vec<&'static str>,
    unknown: Vec<(&'static str, String)>,
    hidden: Vec<&'static str>,
    kept: Vec<&'static str>,
}

impl<'w> Writes<'w> {
    pub(super) fn new(window: &'w native::Element) -> Self {
        Self {
            window,
            calls: Vec::new(),
            controls: Vec::new(),
            wrote: Vec::new(),
            unknown: Vec::new(),
            hidden: Vec::new(),
            kept: Vec::new(),
        }
    }

    /// The update leaves the control(s) `part` is about definitely hidden ([`Update::hidden`]): a
    /// refusal an earlier update left for it no longer withholds the window, and the uncertainty
    /// is remembered for a later Show that keeps the drawable ([`Update::kept`]). Every uncertain
    /// part a definite hide can reach is marked this way where the update hides it: `BARS`
    /// (coarse, hidden bars at sea, a mission item's hidden bars), `TYPE_SPRITE` (coarse, an
    /// untinted `type`), `MARKER` (coarse), `CARRIER_SPRITE` (a hidden carrier) and `IDEOLOGY`
    /// (coarse, a failed or unbound predicate).
    pub(super) fn hid(&mut self, part: &'static str) {
        self.hidden.push(part);
    }

    /// The update shows the child whose drawable is `part` and keeps the drawable it holds
    /// ([`Update::kept`]).
    pub(super) fn kept(&mut self, part: &'static str) {
        self.kept.push(part);
    }

    /// The update writes `part` ([`Update::wrote`]).
    pub(super) fn wrote(&mut self, part: &'static str) {
        if !self.wrote.contains(&part) {
            self.wrote.push(part);
        }
    }

    /// The update's code writes `part`, but the hour lacks its operands, for `why`
    /// ([`Update::unknown`]): the write is not made.
    pub(super) fn unknown(&mut self, part: &'static str, why: impl Into<String>) {
        self.unknown.push((part, why.into()));
    }

    fn call(&mut self, kind: native::Kind, name: &str, call: Call) -> bool {
        let Some(element) = self.window.find(kind, name) else {
            return false;
        };
        self.calls.push((element.path.clone(), call));
        true
    }

    /// Shows the control.
    pub(super) fn show(&mut self, kind: native::Kind, name: &str) -> bool {
        self.call(kind, name, Call::Show)
    }

    /// Hides the control.
    pub(super) fn hide(&mut self, kind: native::Kind, name: &str) -> bool {
        self.call(kind, name, Call::Hide)
    }

    /// Writes the control's hide bit.
    pub(super) fn hide_flag(&mut self, kind: native::Kind, name: &str, hide: bool) -> bool {
        self.call(kind, name, Call::SetHideFlag(hide))
    }

    /// Hides the control, then sets its hide bit to retain hiding across window shows.
    pub(super) fn hide_and_flag(&mut self, kind: native::Kind, name: &str) {
        self.hide(kind, name);
        self.hide_flag(kind, name, true);
    }

    /// The hide bit cleared then `Show`: the updaters' way of showing a control they may have
    /// flagged hidden.
    pub(super) fn unflag_and_show(&mut self, kind: native::Kind, name: &str) {
        self.hide_flag(kind, name, false);
        self.show(kind, name);
    }

    /// `Show` or `Hide`.
    pub(super) fn shown(&mut self, kind: native::Kind, name: &str, shown: bool) {
        if shown {
            self.show(kind, name);
        } else {
            self.hide(kind, name);
        }
    }

    /// Writes a one-based frame, or a progress bar's value.
    pub(super) fn frame(&mut self, kind: native::Kind, name: &str, frame: i32) -> bool {
        self.call(kind, name, Call::SetFrame(frame))
    }

    /// Writes fields through `set`. Sprite and text writes become separate ordered
    /// calls; other fields merge with the control's retained writes for this update.
    pub(super) fn control(
        &mut self,
        kind: native::Kind,
        name: &str,
        set: impl FnOnce(&mut native_gui::Bound),
    ) -> bool {
        let Some(element) = self.window.find(kind, name) else {
            return false;
        };
        let mut written = native_gui::Bound::default();
        set(&mut written);
        if let Some(sprite) = written.sprite.take() {
            self.calls.push((
                element.path.clone(),
                Call::SetSprite(sprite, written.setter),
            ));
        }
        if let Some(text) = written.text.take() {
            self.calls.push((element.path.clone(), Call::SetText(text)));
        }
        let at = match self
            .controls
            .iter()
            .position(|(path, _)| *path == element.path)
        {
            Some(at) => at,
            None => {
                self.controls
                    .push((element.path.clone(), native_gui::Bound::default()));
                self.controls.len() - 1
            }
        };
        overlay(&mut self.controls[at].1, &written);
        true
    }

    /// The update's calls, the controls' merged writes after the visibility and frame calls
    /// (neither reads the other).
    pub(super) fn finish(self) -> Vec<(String, Call)> {
        self.update().calls
    }

    /// The update: its calls ([`Writes::finish`]) with the parts it writes and those whose
    /// operands the hour lacks.
    pub(super) fn update(self) -> Update {
        let mut calls = self.calls;
        calls.extend(
            self.controls
                .into_iter()
                .map(|(path, bound)| (path, Call::Control(Box::new(bound)))),
        );
        Update {
            calls,
            wrote: self.wrote,
            unknown: self.unknown,
            hidden: self.hidden,
            kept: self.kept,
            selectable: None,
            owner: None,
            port_tooltip: None,
        }
    }
}

/// One ordered window update: every call with known operands, the parts it writes back
/// to known state, and the parts whose written operands are missing. Missing inputs never
/// drop unrelated calls. Conditional writes, such as full-intel `low_supply`, retain state
/// on other updates. Unknown parts retain their values but withhold drawing until rewritten
/// ([`native_gui::lifecycle::Withheld`]), counted where the window would draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Update {
    /// Task-force members and ownership bound to a presented plate.
    pub selectable: Option<(bool, Arc<[u64]>)>,
    /// The canonical slot of the task force's owner, which the counter filter reads.
    pub owner: Option<u32>,
    /// Port callback metadata, replaced with the same update as its member refs.
    pub port_tooltip: Option<viewer_foundation::map::counter_hovers::task_force_port::Metadata>,
    /// Every call made, in order.
    pub calls: Vec<(String, Call)>,
    /// The parts it writes, each lifting that part's unknown ([`native_gui::lifecycle::Rewrote`]).
    pub wrote: Vec<&'static str>,
    /// The parts its code writes whose operands the hour lacks, each with why.
    pub unknown: Vec<(&'static str, String)>,
    /// Drawable parts it hides without replacing them: a drawable left unknown is then unseen, so
    /// it withholds nothing while hidden, and stays unknown (a Hide does not make it known).
    pub hidden: Vec<&'static str>,
    /// Shown drawable parts retained by an undefined sprite selection. An unknown
    /// retained drawable becomes seen again and withholds the window.
    pub kept: Vec<&'static str>,
}

impl Update {
    /// An update that makes `calls` and writes no part a withheld window waits for.
    pub(super) fn calls(calls: Vec<(String, Call)>) -> Self {
        Self {
            calls,
            ..Self::default()
        }
    }
}

/// A naval counter's `count_bg` plate colour, written on every update from possibly
/// missing hour operands ([`plate_color`]).
pub(super) const PLATE: &str = "count_bg's plate colour";

/// The move of `bar_str` and `bar_org` by drawable length, written whenever bars are
/// shown in detailed counters or full-intel mission items ([`bar_writes`]).
pub(super) const BARS: &str = "the strength and organisation bars' move";

/// `low_supply` is shown at a frame or hidden from the received-supply minimum. Mission
/// items write it only at full first-member intel; at-sea counters use the last fully
/// visible member ([`bind_supply`]).
pub(super) const SUPPLY: &str = "low_supply";

/// `count_bg`'s frame at sea: 2 when all member missions are active in their regions,
/// otherwise 1; coarse counters use 3.
pub(super) const PLATE_FRAME: &str = "count_bg's frame";

/// `air_base_icon` is hidden or shown at the carrier's frame ([`bind_carrier`]).
pub(super) const CARRIER: &str = "air_base_icon";

/// `air_base_icon` keeps its last defined sprite. Undefined names and Hide retain its
/// drawable, so only a defined selection makes it known ([`bind_carrier`]).
pub(super) const CARRIER_SPRITE: &str = "air_base_icon's sprite";

/// `capital_count_txt` holds the capital-ship sum over every member.
pub(super) const CAPITALS: &str = "capital_count_txt";

/// The `no_orders` and `invalid_orders` glyphs ([`bind_orders`]).
pub(super) const ORDERS: &str = "the order glyphs";

/// The `pride_of_the_fleet_icon` marker.
pub(super) const PRIDE: &str = "pride_of_the_fleet_icon";

/// `group_marker` is shown with the fleet's colour, or hidden.
pub(super) const MARKER: &str = "group_marker";

/// A mission item's `flag`, selected from the medium or small atlas.
pub(super) const FLAG: &str = "flag";

/// `counter_ideology` at sea is shown at the logical owner's ruling ideology group
/// number, or retains the update's initial hiding ([`naval::ideology_frame`]).
pub(super) const IDEOLOGY: &str = "counter_ideology";

/// Parts a mission update can retain despite known operands: `low_supply` below full
/// first-member intel, an unreplaced `type` sprite, and `air_base_icon` after an undefined
/// name or Hide. An entirely unknown update ([`GRIDS`]), including an item first built
/// after it, leaves these unknown until specifically written; other parts wait for the
/// next update ([`Unknown::Update`]).
///
/// [`Unknown::Update`]: native_gui::lifecycle::Unknown::Update
pub(super) const MISSION_ITEM_KEPT: [&str; 3] = [SUPPLY, TYPE_SPRITE, CARRIER_SPRITE];

/// The `type` child's last defined sprite. An undecidable hull selection leaves its
/// drawable unknown ([`select_hull`]).
pub(super) const TYPE_SPRITE: &str = "the type child's sprite";

/// Makes the recorded `calls` on `instance`, a window of `root` built with `resources`: every
/// visibility, frame, sprite, text and position call through the module's one applier
/// ([`Calls::apply`]), in order, and each control write as the field writes it is ([`overlay`]),
/// so a field this update's code does not write keeps what the control holds.
pub(super) fn apply(
    instance: &mut Instance,
    root: &native::Element,
    calls: &[(String, Call)],
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    if calls.is_empty() {
        return Ok(());
    }
    let ordered = Calls {
        calls: calls
            .iter()
            .filter(|(_, call)| !matches!(call, Call::Control(_)))
            .map(|(path, call)| ((), path.clone(), call.clone()))
            .collect(),
    };
    if !ordered.calls.is_empty() {
        let mut one = HashMap::from([((), instance.clone())]);
        ordered.apply(root, &mut one, resources)?;
        *instance = one.remove(&()).expect("applied above");
    }
    for (path, call) in calls {
        let Call::Control(written) = call else {
            continue;
        };
        let element = find_path(root, path)
            .ok_or_else(|| format!("{path} is not an element of {}", root.path))?;
        overlay(instance.control(root, element)?, written);
    }
    Ok(())
}

/// The fields `written` sets, written over what `control` keeps: every write these updaters make
/// sets its field ([`Writes::control`] records nothing unset), so an unset field is one the code
/// did not write. A sprite and a text are never among them: [`Writes::control`] makes each its
/// own ordered call.
fn overlay(control: &mut native_gui::Bound, written: &native_gui::Bound) {
    macro_rules! copy {
        ($($field:ident),*) => {
            $(if written.$field.is_some() {
                control.$field = written.$field;
            })*
        };
    }
    copy!(
        color, progress, texture, stretch, offset, rotation, shield, checked, enabled
    );
}

/// Native pointer geometry for TF observers. The other existing observers keep
/// their established draw-rectangle contract until their own consumer adopts it.
pub(super) fn native_hit_rectangle(
    control: &native::Element,
    layout: &Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Option<HitRectangle> {
    observer_hit_rectangle(
        control,
        layout,
        origin,
        resources,
        notes,
        HitMode::NativeGeometry,
    )
}

/// The element of `window` at `path`.
pub(super) fn find_path<'w>(
    window: &'w native::Element,
    path: &str,
) -> Option<&'w native::Element> {
    if window.path == path {
        return Some(window);
    }
    window
        .nested()
        .into_iter()
        .find_map(|nested| find_path(nested, path))
}

/// The run-time state of one naval counter, port item or combat tile.
pub(super) struct NavyState<'a> {
    pub flotilla: &'a naval::Flotilla,
    pub countries: &'a Countries,
}

/// What the naval counter's calls read besides the counter itself: the user's options, the
/// content's defines and the art the code puts in the window.
pub(super) struct NavyContext<'a> {
    pub settings: CounterSettings,
    pub selected: &'a BTreeSet<Unit>,
    pub defines: &'a CounterDefines,
    pub art: &'a dyn NavyArt,
    /// The windows' resources, which the code's sprite lookups and size queries resolve through.
    pub gui: &'a dyn native_gui::Resources,
}

/// The textures the naval counter's code draws in place of an element's sprite.
pub(super) trait NavyArt {
    /// The medium or small atlas cell for `slot`.
    fn flag(&self, slot: u32, medium: bool) -> Option<Texture>;
}

/// [`NavyArt`] from the loaded counter assets and the hour's countries.
pub(super) struct CounterArt<'a> {
    pub assets: &'a CounterAssets,
    pub countries: &'a Countries,
}

impl NavyArt for CounterArt<'_> {
    fn flag(&self, slot: u32, medium: bool) -> Option<Texture> {
        let atlas = if medium {
            &self.assets.medium_flags
        } else {
            &self.assets.flags
        };
        let (texture, uv) = self.countries.flag_now(atlas, slot)?;
        Some(flag_texture(atlas, texture, uv))
    }
}

/// A plate's colour uses `NGraphics.COUNTER_MODE_ALLEGIANCE_*` by `side` in allegiance
/// mode: the type-icon relation at sea ([`type_tint::plate`]), or a mission item's own
/// allegiance. Country mode uses the first task force's logical owner's current UI
/// colour, including an overlord's inherited colour. `Err` names the missing hour operand.
pub(super) fn plate_color(
    ctx: &NavyContext<'_>,
    countries: &Countries,
    side: Allegiance,
    country: Option<u32>,
) -> Result<[f32; 4], String> {
    counter_plate_color(
        ctx.settings.color_mode,
        &ctx.defines.tints,
        countries,
        side,
        country,
    )
}

/// Ordered `unit_counter_navy` calls for `state`. Buttons supply `count_bg` and
/// `air_base_icon`, instant text boxes supply counts, a child container supplies the
/// spotting bar, and icons supply the rest. Untouched children retain their window state.
///
/// At-sea counters use [`at_sea`]; port items and combat tiles use [`mission_item`] and
/// never coarse rendering. Missing plate colour ([`PLATE`]), bar movement ([`BARS`]) or
/// hull ([`TYPE_SPRITE`]) leaves only that part unknown; all other calls remain ([`Update`]).
pub(super) fn navy_calls(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    state: &NavyState<'_>,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
) -> Update {
    let flotilla = state.flotilla;
    // Mission items compute their own plate allegiance; at-sea counters use the
    // type icon's relation.
    let plate = match (ctx.settings.color_mode, &flotilla.side) {
        (CounterColorMode::Allegiance, Err(missing)) => Err(missing.clone()),
        // Country-colour mode never reads allegiance bytes.
        (CounterColorMode::Country, _) => {
            plate_color(ctx, state.countries, Allegiance::Other, flotilla.logical)
        }
        (CounterColorMode::Allegiance, Ok(side)) => {
            plate_color(ctx, state.countries, *side, flotilla.logical)
        }
    };
    let mut writes = Writes::new(window);
    if flotilla.port {
        mission_item(ctx, window, state, plate, report, &mut writes);
    } else {
        at_sea(ctx, window, state, plate, report, &mut writes);
    }
    let mut update = writes.update();
    update.selectable = Some((
        state.countries.side(flotilla.owner) == Allegiance::Ours,
        Arc::clone(&flotilla.members),
    ));
    update.owner = Some(flotilla.owner);
    update.port_tooltip.clone_from(&flotilla.port_tooltip);
    update
}

/// Writes `count_bg`'s plate colour, or marks it unknown for a missing hour operand.
fn plate_write(plate: &Result<[f32; 4], String>, writes: &mut Writes<'_>) {
    use native::Kind::Button;
    match plate {
        Ok(plate) => {
            writes.control(Button, "count_bg", |bound| bound.color = Some(*plate));
            writes.wrote(PLATE);
        }
        Err(why) => writes.unknown(PLATE, why.clone()),
    }
}

/// Mission-item construction hides and flags `counter_ideology`, `counter_overlay`
/// and `large_flag`, and clears both counts. Updates never show those three controls.
pub(super) fn mission_item_setup(window: &native::Element) -> Vec<(String, Call)> {
    use native::Kind::{Icon, TextBox};
    let mut writes = Writes::new(window);
    for name in ["counter_ideology", "counter_overlay", "large_flag"] {
        writes.hide_and_flag(Icon, name);
    }
    for name in ["count_txt", "capital_count_txt"] {
        writes.control(TextBox, name, |bound| bound.text = Some(String::new()));
    }
    writes.finish()
}

/// A received-supply ratio picks `low_supply`'s one-based frame and visibility
/// ([`SUPPLY`]); retained updates make no call. Mission items clear hide bits before
/// Show and set them with Hide; at-sea counters show and hide plainly. Missing ratios
/// leave the part unknown until a later ratio write, including across retained updates.
fn bind_supply(
    ctx: &NavyContext<'_>,
    supply: counter_inputs::NavalSupply,
    flagged: bool,
    writes: &mut Writes<'_>,
) {
    use counter_inputs::NavalSupply;
    use native::Kind::Icon;
    let ratio = match supply {
        NavalSupply::Retain => return,
        NavalSupply::Unavailable => return writes.unknown(SUPPLY, SUPPLY_UNRECORDED),
        NavalSupply::Write(ratio) => ratio,
    };
    match counter_status::supply_frame(ratio, ctx.defines.supply) {
        None if flagged => writes.hide_and_flag(Icon, "low_supply"),
        None => {
            writes.hide(Icon, "low_supply");
        }
        Some(frame) => {
            if flagged {
                writes.unflag_and_show(Icon, "low_supply");
            } else {
                writes.show(Icon, "low_supply");
            }
            if let Ok(frame) = i32::try_from(frame + 1) {
                writes.frame(Icon, "low_supply", frame);
            }
        }
    }
    writes.wrote(SUPPLY);
}

/// Why `low_supply` is undecided: a member's `TaskForceCounterInputs` row, which holds its
/// received-supply ratio, is not recorded.
pub(super) const SUPPLY_UNRECORDED: &str = "low_supply is undecided: a member's received-supply \
     ratio (TaskForceCounterInputs) is not recorded";

/// Hides `air_base_icon`, or shows it with the carrier's sprite and frame. Mission
/// items flag Hide and unflag Show. Both paths write visibility/frame ([`CARRIER`]);
/// only a defined sprite makes the drawable known ([`CARRIER_SPRITE`]). Hide leaves it
/// unseen ([`Update::hidden`]); an undefined selection shows the retained drawable
/// ([`Update::kept`]). Carrier names ([`counter_order::AIR_BASE_ICON`],
/// [`counter_order::ROCKET_SITE_ICON`]) resolve through content and undefined names
/// are counted ([`CARRIER_UNDEFINED`]). Missing or ambiguous ship operands leave both
/// parts unknown.
fn bind_carrier(
    gui: &dyn native_gui::Resources,
    flotilla: &naval::Flotilla,
    flagged: bool,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::Button;
    let undecided = |writes: &mut Writes<'_>, why: &str| {
        writes.unknown(CARRIER, why);
        writes.unknown(CARRIER_SPRITE, why);
    };
    let carrier = match flotilla.cells.as_ref() {
        None => return undecided(writes, SHIPS_UNRECORDED),
        Some(cells) if cells.carrier.len() != 1 => {
            return undecided(
                writes,
                "air_base_icon is undecided: the unrecorded carrier operands leave more than one \
                 state possible",
            );
        }
        Some(cells) => cells.carrier.first().copied().flatten(),
    };
    writes.wrote(CARRIER);
    match carrier {
        Some((sprite, frame)) => {
            if flagged {
                writes.unflag_and_show(Button, "air_base_icon");
            } else {
                writes.show(Button, "air_base_icon");
            }
            writes.control(Button, "air_base_icon", |bound| {
                bound.sprite = Some(sprite.as_bytes().to_vec());
                bound.setter = native_gui::SpriteSetter::ButtonStandard;
            });
            if gui.sprite(sprite.as_bytes()).is_some() {
                writes.wrote(CARRIER_SPRITE);
            } else {
                *report
                    .entry(format!("{sprite}{CARRIER_UNDEFINED}").into())
                    .or_default() += 1;
                writes.kept(CARRIER_SPRITE);
            }
            if let Ok(frame) = i32::try_from(frame + 1) {
                writes.frame(Button, "air_base_icon", frame);
            }
        }
        None => {
            if flagged {
                writes.hide_and_flag(Button, "air_base_icon");
            } else {
                writes.hide(Button, "air_base_icon");
            }
            writes.hid(CARRIER_SPRITE);
        }
    }
}

/// Counted after an undefined carrier sprite's name. The failed selection leaves
/// `air_base_icon` showing its retained drawable.
pub(super) const CARRIER_UNDEFINED: &str = " is not defined; air_base_icon keeps the sprite it \
     showed because the selection does not replace it";

/// Moves `bar_str`, then `bar_org`, by [`counter_inputs::bar_step`] of drawable length
/// and stretches each to its fill. Length is the frame's truncated height times icon
/// scale, truncated again, using fallback artwork ([`native_gui::drawable_extent`]).
/// Missing drawables stretch and draw nothing; unmeasurable bars stretch but leave
/// their movement unknown ([`BARS`]).
fn bar_writes(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    fills: (f32, f32),
    writes: &mut Writes<'_>,
) {
    use native::Kind::Icon;
    let (organisation, strength) = fills;
    let mut measured = true;
    for (name, fill) in [("bar_str", strength), ("bar_org", organisation)] {
        // The bar's own drawable: the code replaces neither its sprite nor its texture.
        let length = match window.find(Icon, name) {
            Some(element) => {
                native_gui::drawable_extent(element, ctx.gui, &native_gui::Bindings::default())
                    .map_err(|refusals| {
                        refusals
                            .iter()
                            .map(|refusal| refused_cause(NAVY_WINDOW, refusal))
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
            }
            None => Ok(None),
        };
        let offset = match length {
            // A bar with no drawable has no extent: it moves by no step.
            Ok(length) => {
                let height = length.map_or(0, |[_, height]| height);
                Some([
                    i32::try_from(counter_inputs::bar_step(fill, height as f32))
                        .unwrap_or(i32::MAX),
                    0,
                ])
            }
            Err(why) => {
                measured = false;
                writes.unknown(BARS, why);
                None
            }
        };
        writes.control(Icon, name, |bound| {
            bound.offset = offset;
            bound.stretch = Some(fill);
        });
    }
    if measured {
        writes.wrote(BARS);
    }
}

/// The order glyphs ([`ORDERS`]): show `no_orders` alone when it holds, otherwise
/// `invalid_orders` alone when it holds, otherwise hide both. Mission items flag Hide
/// and unflag Show. Unknown glyphs receive no write; a known false `no_orders` still
/// receives its write. Both controls share a position and `no_orders` wins when both
/// hold; the game's precedence for that case remains unknown.
fn bind_orders(flotilla: &naval::Flotilla, flagged: bool, writes: &mut Writes<'_>) {
    use native::Kind::Icon;
    let set = |writes: &mut Writes<'_>, name: &str, shown: bool| match (shown, flagged) {
        (true, true) => writes.unflag_and_show(Icon, name),
        (false, true) => writes.hide_and_flag(Icon, name),
        (shown, false) => writes.shown(Icon, name, shown),
    };
    let status = &flotilla.status;
    let (no_orders, invalid) = match status.no_orders {
        None => return writes.unknown(ORDERS, ORDERS_UNDECIDED),
        Some(true) => (true, Some(false)),
        Some(false) => (false, status.invalid_orders),
    };
    set(writes, "no_orders", no_orders);
    match invalid {
        Some(invalid) => {
            set(writes, "invalid_orders", invalid);
            writes.wrote(ORDERS);
        }
        None => writes.unknown(ORDERS, ORDERS_UNDECIDED),
    }
}

/// Why the order glyphs are undecided: a member's order operands (`TaskForceCounterInputs`, its
/// fleet's regions) are not recorded ([`counter_inputs::NavalStatus`]).
pub(super) const ORDERS_UNDECIDED: &str = "the order glyphs are undecided: a member's order \
     operands (TaskForceCounterInputs, its fleet's regions) are not recorded";

/// Why a ship-operand part is undecided: a member's ship operands (`ShipCounterInputs`) are not
/// recorded.
pub(super) const SHIPS_UNRECORDED: &str =
    "a member's ship counter operands (ShipCounterInputs) are not recorded";

/// Why `pride_of_the_fleet_icon` is undecided: a member's pride reference
/// (`CountryCounterInputs.pride_of_the_fleet`) or its ship is not recorded.
pub(super) const PRIDE_UNDECIDED: &str = "pride_of_the_fleet_icon is undecided: a member's pride \
     of the fleet (CountryCounterInputs) or its ship is not recorded";

/// `pride_of_the_fleet_icon` ([`PRIDE`]): shown when the members' pride flag holds, and on a
/// mission item a first member at [`naval::MISSION_ITEM_DETAIL`]; unknown when it is not settled.
fn bind_pride(flotilla: &naval::Flotilla, flagged: bool, writes: &mut Writes<'_>) {
    use native::Kind::Icon;
    let name = "pride_of_the_fleet_icon";
    let shown = match flotilla.shows_pride() {
        Some(shown) => shown,
        None => return writes.unknown(PRIDE, PRIDE_UNDECIDED),
    };
    match (shown, flagged) {
        (true, true) => writes.unflag_and_show(Icon, name),
        (false, true) => writes.hide_and_flag(Icon, name),
        (shown, false) => writes.shown(Icon, name, shown),
    }
    writes.wrote(PRIDE);
}

/// `group_marker` ([`MARKER`]): shown with its fleet's colour, else hidden (flagged and unflagged
/// on a mission item); unknown when the fleet operands are not recorded.
fn bind_marker(flotilla: &naval::Flotilla, flagged: bool, writes: &mut Writes<'_>) {
    use native::Kind::Icon;
    match &flotilla.marker {
        Err(why) => return writes.unknown(MARKER, why.clone()),
        Ok(Some(color)) => {
            if flagged {
                writes.unflag_and_show(Icon, "group_marker");
            } else {
                writes.show(Icon, "group_marker");
            }
            let color = *color;
            writes.control(Icon, "group_marker", |bound| bound.color = Some(color));
        }
        Ok(None) if flagged => writes.hide_and_flag(Icon, "group_marker"),
        Ok(None) => {
            writes.hide(Icon, "group_marker");
        }
    }
    writes.wrote(MARKER);
}

/// What the `type` child shows.
enum Hull {
    /// `GFX_unit_<name>_icon_medium_white` from the recorded ship definition's name.
    Resolved(Vec<u8>),
    /// An invalid ship definition ([`counter_order::HullIcon::Resolved`]) makes no write.
    Invalid,
    /// The recording cannot decide the ship definition, so its selection write is unknown.
    Unknown(&'static str),
}

/// The resolved hull of a counter whose members are all seen.
fn resolved_hull(flotilla: &naval::Flotilla) -> Hull {
    use counter_order::HullIcon;
    match flotilla.cells.as_ref().map(|cells| &cells.hull) {
        Some(HullIcon::Resolved(Some(name))) => Hull::Resolved(name.clone()),
        Some(HullIcon::Resolved(None)) => Hull::Invalid,
        Some(HullIcon::Ambiguous) => Hull::Unknown(HULL_AMBIGUOUS),
        Some(HullIcon::Unrecorded) | None => Hull::Unknown(HULL_UNRECORDED),
    }
}

/// Selects the navy hull sprite from the ship definition name, with no custom icon.
/// Empty names, invalid definitions and undefined sprite names retain the drawable;
/// undefined names are counted. Defined names replace it ([`TYPE_SPRITE`]) and set
/// one-based frame 1, always the pictorial frame. An undecidable hull makes the part
/// unknown; the window retains its drawable but withholds drawing until a later
/// selection writes it, counted there by reason.
fn select_hull(
    gui: &dyn native_gui::Resources,
    hull: &Hull,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::Icon;
    if writes.window.find(Icon, "type").is_none() {
        return;
    }
    // A selection making no write keeps the drawable and shows the `type` child
    // with whatever it held ([`Writes::kept`]).
    let name = match hull {
        Hull::Resolved(name) if !name.is_empty() => name,
        Hull::Resolved(_) | Hull::Invalid => return writes.kept(TYPE_SPRITE),
        Hull::Unknown(why) => return writes.unknown(TYPE_SPRITE, *why),
    };
    let sprite = [
        templates::UNIT_PREFIX.as_bytes(),
        name,
        templates::UNIT_SUFFIX.as_bytes(),
    ]
    .concat();
    if gui.sprite(&sprite).is_none() {
        *report
            .entry(format!("{}{HULL_UNDEFINED}", String::from_utf8_lossy(&sprite)).into())
            .or_default() += 1;
        return writes.kept(TYPE_SPRITE);
    }
    writes.control(Icon, "type", |bound| {
        bound.sprite = Some(sprite);
        bound.setter = native_gui::SpriteSetter::Button;
    });
    writes.wrote(TYPE_SPRITE);
    writes.frame(Icon, "type", 1);
}

/// The no-intel `type` replacement ([`NO_INTEL_SPRITE`]) drops rotation. Undefined
/// names retain the drawable; defined replacements make it known ([`TYPE_SPRITE`]).
fn no_intel_sprite(gui: &dyn native_gui::Resources, writes: &mut Writes<'_>) {
    let sprite = NO_INTEL_SPRITE.as_bytes();
    if writes.control(native::Kind::Icon, "type", |bound| {
        bound.sprite = Some(sprite.to_vec());
        bound.setter = native_gui::SpriteSetter::Icon;
    }) {
        if gui.sprite(sprite).is_some() {
            writes.wrote(TYPE_SPRITE);
        } else {
            // An undefined name changes nothing: the drawable is kept.
            writes.kept(TYPE_SPRITE);
        }
    }
}

/// The no-intel type's one-based frame, selected by NATO preference
/// ([`templates::no_intel_icon`]).
fn no_intel_frame(ctx: &NavyContext<'_>) -> i32 {
    i32::try_from(templates::no_intel_icon(ctx.settings.nato).frame + 1).unwrap_or(1)
}

/// Counted after an undefined hull sprite's name: selection makes no write and the
/// `type` child retains its drawable.
pub(super) const HULL_UNDEFINED: &str =
    " is not defined; the type child keeps what it showed because the selection makes no write";

/// A best-ship tie between different names on an unrecorded variant makes the hull
/// undecidable and withholds the window ([`TYPE_SPRITE`]).
pub(super) const HULL_AMBIGUOUS: &str = "a hull is not selected: two best ships tie on an \
     unrecorded variant and name different sprites";

/// Why a hull is undecided when a member's ship operands (format 23 `ShipCounterInputs`) are not
/// recorded: the window is withheld ([`TYPE_SPRITE`]).
pub(super) const HULL_UNRECORDED: &str =
    "a hull is not selected: a member's ship counter operands are not recorded";

/// Updates a naval counter at sea.
fn at_sea(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    state: &NavyState<'_>,
    plate: Result<[f32; 4], String>,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::{Button, Container, Icon, TextBox};
    let flotilla = state.flotilla;
    // Hide the ideology control first; only the detailed branch may show it again.
    writes.hide(Icon, "counter_ideology");
    bind_supply(ctx, flotilla.supply, false, writes);
    // Carrier state refreshes for both detailed and coarse counters.
    bind_carrier(ctx.gui, flotilla, false, report, writes);
    let capitals = match &flotilla.cells {
        Some(cells) => cells.capitals.map(|sum| sum.to_string()),
        None => (flotilla.ships == 0 && !flotilla.hidden).then(|| "0".to_owned()),
    };
    // Show the capital glyph and count, then write the sum over every member.
    writes.show(Icon, "capital_ship");
    writes.show(TextBox, "capital_count_txt");
    capitals_write(capitals, writes);
    bind_pride(flotilla, false, writes);
    // Use plate frame 2 when all member missions are active, otherwise 1.
    // The coarse pass replaces it with 3; plate colouring follows this state update.
    let active = flotilla.status.active;
    let count = if flotilla.hidden {
        UNKNOWN_COUNT.to_owned()
    } else {
        flotilla.ships.to_string()
    };
    // The spotting bar is bound hidden and never shown at sea.
    writes.hide(Container, "spotting_bar_container");
    // Selection follows the first member's local selected byte.
    let selected = flotilla
        .members
        .first()
        .is_some_and(|&id| ctx.selected.contains(&Unit::TaskForce(id)));
    if selected && (flotilla.coarse || flotilla.visible_member) {
        writes.show(Icon, "count_selected");
    } else {
        writes.hide(Icon, "count_selected");
    }
    if flotilla.coarse {
        // Coarse counters show the plate at frame 3, the count and representative roster
        // owner's medium flag. Hide strength/organisation bars, portrait, type, small
        // flag, no-orders glyph, overlay and group marker. Naval counters retain their
        // last detailed invalid-orders state because that coarse write is land-only.
        plate_write(&plate, writes);
        writes.show(Button, "count_bg");
        writes.frame(Button, "count_bg", 3);
        writes.wrote(PLATE_FRAME);
        writes.show(TextBox, "count_txt");
        writes.control(TextBox, "count_txt", |bound| {
            bound.text = Some(count.clone())
        });
        match ctx.art.flag(flotilla.owner, true) {
            Some(texture) => {
                writes.show(Icon, "large_flag");
                writes.control(Icon, "large_flag", |bound| bound.texture = Some(texture));
            }
            None => {
                writes.hide(Icon, "large_flag");
            }
        }
        for name in [
            "bar_str",
            "general_portrait",
            "type",
            "flag",
            "bar_org",
            "no_orders",
            "counter_overlay",
            "group_marker",
        ] {
            writes.hide(Icon, name);
        }
        // Definitely hidden now: the bars and their move, the `type` child and its drawable,
        // `group_marker`, and `counter_ideology` as the update's first write left it.
        for part in [BARS, TYPE_SPRITE, MARKER, IDEOLOGY] {
            writes.hid(part);
        }
        return;
    }
    // The detailed branch: `counter_overlay`, `bar_str`, `count_txt`, `count_bg` and `type`
    // shown, `large_flag` hidden, `bar_org` shown, then each set. `count_selected`
    // follows the first selected member and fully-visible-member gate above.
    plate_write(&plate, writes);
    writes.show(Button, "count_bg");
    match active {
        Some(active) => {
            writes.frame(Button, "count_bg", if active { 2 } else { 1 });
            writes.wrote(PLATE_FRAME);
        }
        None => writes.unknown(PLATE_FRAME, ACTIVE_UNDECIDED),
    }
    writes.show(Icon, "counter_overlay");
    writes.hide(Icon, "large_flag");
    writes.show(TextBox, "count_txt");
    writes.control(TextBox, "count_txt", |bound| {
        bound.text = Some(count.clone())
    });
    match &flotilla.bars {
        naval::NavalBars::Hidden => {
            writes.hide(Icon, "bar_str");
            writes.hide(Icon, "bar_org");
            writes.hid(BARS);
        }
        naval::NavalBars::Fill(organisation, strength) => {
            writes.show(Icon, "bar_str");
            writes.show(Icon, "bar_org");
            bar_writes(ctx, window, (*organisation, *strength), writes);
        }
        naval::NavalBars::Unknown(why) => {
            writes.show(Icon, "bar_str");
            writes.show(Icon, "bar_org");
            writes.unknown(BARS, why.clone());
        }
    }
    // Show the expeditionary owner's flag. When counter_ideology is bound, the
    // logical owner's ruling ideology group supplies its frame, with hide bit
    // cleared ([`naval::ideology_frame`]).
    let ideology = if window.find(Icon, "flag").is_some() {
        match flotilla
            .flag_owner
            .and_then(|owner| ctx.art.flag(owner, false))
        {
            Some(texture) => {
                writes.show(Icon, "flag");
                writes.control(Icon, "flag", |bound| bound.texture = Some(texture));
            }
            None => {
                writes.hide(Icon, "flag");
            }
        }
        window
            .find(Icon, "counter_ideology")
            .map(|_| &flotilla.ideology)
    } else {
        None
    };
    match ideology {
        Some(Ok(Some(frame))) => {
            writes.frame(Icon, "counter_ideology", *frame);
            writes.unflag_and_show(Icon, "counter_ideology");
            writes.wrote(IDEOLOGY);
        }
        Some(Err(why)) => writes.unknown(IDEOLOGY, format!("counter_ideology is undecided: {why}")),
        // Definitely hidden, as the update's first write left it: a refusal an earlier undecided
        // update left no longer withholds the counter (the hidden-part mechanism).
        Some(Ok(None)) | None => writes.hid(IDEOLOGY),
    }
    // Show and tint the type icon by its relation, including hidden members.
    // Hidden members use the no-intel sprite at 1 + NATO; others use the hull.
    // Missing relations withhold the glyph and count unavailable_type_tints.
    match flotilla.type_tint {
        Some(side) => {
            if flotilla.hidden {
                no_intel_sprite(ctx.gui, writes);
                writes.frame(Icon, "type", no_intel_frame(ctx));
            } else {
                select_hull(ctx.gui, &resolved_hull(flotilla), report, writes);
            }
            writes.show(Icon, "type");
            let tint = ctx.defines.type_colors.of(side);
            writes.control(Icon, "type", |bound| bound.color = Some(tint));
        }
        None => {
            writes.hide(Icon, "type");
            writes.hid(TYPE_SPRITE);
        }
    }
    // `group_marker`: an own representative's non-reserve fleet colour, shown; else hidden.
    bind_marker(flotilla, false, writes);
    // `naval_update`'s detailed-only order glyphs.
    bind_orders(flotilla, false, writes);
}

/// Updates a port item or combat tile in call order, flagging hidden controls and
/// clearing flags before Show. No calls touch `capital_ship` or `general_portrait`,
/// nor the visibility of `flag`, `count_bg`, `type` or either count.
fn mission_item(
    ctx: &NavyContext<'_>,
    window: &native::Element,
    state: &NavyState<'_>,
    plate: Result<[f32; 4], String>,
    report: &mut BTreeMap<Cow<'static, str>, usize>,
    writes: &mut Writes<'_>,
) {
    use native::Kind::{Container, Icon, TextBox};
    let NavyState { flotilla, .. } = *state;
    // Set the medium or small flag without Show. An undecidable mixed-owner flag
    // leaves this part unknown ([`FLAG`]).
    match flotilla.flag_owner {
        None => writes.unknown(FLAG, PORT_FLAG_UNDECIDED),
        Some(owner) => {
            match ctx.art.flag(owner, false) {
                Some(texture) => {
                    writes.control(Icon, "flag", |bound| bound.texture = Some(texture));
                }
                None => {
                    writes.hide(Icon, "flag");
                }
            }
            writes.wrote(FLAG);
        }
    }
    // Write only count_bg's colour.
    plate_write(&plate, writes);
    // Hide and flag bar_str and bar_org.
    writes.hide_and_flag(Icon, "bar_str");
    writes.hide_and_flag(Icon, "bar_org");
    // First-member visibility words the counts: ? below 100, a range through 249,
    // exact from 250. The sums use signed integer arithmetic and write text alone.
    let item_text = |total: usize, capital: bool| {
        let item = flotilla.item?;
        Some(viewer_foundation::map::naval_count_approx::format_count(
            total as i32,
            item.first,
            item.byte,
            capital,
            item.ships as i32,
            &ctx.defines.naval_counts,
        ))
    };
    let count = flotilla.item.and_then(|item| item_text(item.ships, false));
    // Sum capital ships over all members; an item without ships sums to zero
    // regardless of missing ship operands.
    let capitals = match (&flotilla.cells, flotilla.item) {
        (_, Some(item)) if item.ships == 0 => item_text(0, true),
        (Some(cells), Some(_)) => cells.capitals.and_then(|sum| item_text(sum, true)),
        _ => None,
    };
    writes.control(TextBox, "count_txt", |bound| {
        bound.text = Some(count.clone().unwrap_or_default())
    });
    capitals_write(capitals, writes);
    // At full first-member intel the bars are unflagged, shown, moved and stretched, and the
    // supply child set (below it neither is touched again).
    match &flotilla.bars {
        // Hidden and flagged above, and not shown again.
        naval::NavalBars::Hidden => writes.hid(BARS),
        naval::NavalBars::Fill(organisation, strength) => {
            writes.unflag_and_show(Icon, "bar_str");
            writes.unflag_and_show(Icon, "bar_org");
            bar_writes(ctx, window, (*organisation, *strength), writes);
        }
        naval::NavalBars::Unknown(why) => {
            writes.unflag_and_show(Icon, "bar_str");
            writes.unflag_and_show(Icon, "bar_org");
            writes.unknown(BARS, why.clone());
        }
    }
    bind_supply(ctx, flotilla.supply, true, writes);
    // Set the type frame to 1 + NATO, then select no-intel below first-member
    // visibility 150, otherwise the navy hull. Replacing a no-intel drawable
    // carries its state effect but takes the new drawable's frame, dropping the
    // prior frame write; selecting the same drawable retains it. Never tint or show.
    writes.frame(Icon, "type", no_intel_frame(ctx));
    if flotilla.no_intel_type() {
        no_intel_sprite(ctx.gui, writes);
    } else {
        select_hull(ctx.gui, &resolved_hull(flotilla), report, writes);
    }
    // Hide and flag mission_type, then unflag and show at mission + 1 when an own
    // item's first and last missions agree.
    writes.hide_and_flag(Icon, "mission_type");
    if let Some(mission) = flotilla.mission {
        writes.unflag_and_show(Icon, "mission_type");
        if let Ok(frame) = i32::try_from(mission + 1) {
            writes.frame(Icon, "mission_type", frame);
        }
    }
    // Hide and flag the spotting bar. Its item and grid spotting operands must
    // both permit Show; port and combat grids initialize spotting to zero and
    // never set it, so the bar stays hidden.
    writes.hide_and_flag(Container, "spotting_bar_container");
    bind_carrier(ctx.gui, flotilla, true, report, writes);
    bind_orders(flotilla, true, writes);
    // Refresh the pride_of_the_fleet_icon marker.
    bind_pride(flotilla, true, writes);
    // Selection follows the first member's selected byte.
    let selected = flotilla
        .members
        .first()
        .is_some_and(|&id| ctx.selected.contains(&Unit::TaskForce(id)));
    if selected {
        writes.unflag_and_show(Icon, "count_selected");
    } else {
        writes.hide_and_flag(Icon, "count_selected");
    }
    // Show group_marker in the fleet's colour for an own item whose first and last
    // members share a fleet; otherwise hide and flag it. Visible controls already
    // have clear hide bits, so repeated unflagging and Show preserve their state.
    bind_marker(flotilla, true, writes);
}

/// `capital_count_txt`'s text ([`CAPITALS`]), or the part left unknown by ship operands the hour
/// lacks.
fn capitals_write(capitals: Option<String>, writes: &mut Writes<'_>) {
    match capitals {
        Some(text) => {
            writes.control(native::Kind::TextBox, "capital_count_txt", |bound| {
                bound.text = Some(text)
            });
            writes.wrote(CAPITALS);
        }
        None => writes.unknown(CAPITALS, SHIPS_UNRECORDED),
    }
}

/// Why `count_bg`'s frame at sea is undecided: a member's mission-activity operands
/// (`TaskForceCounterInputs`) are not recorded.
pub(super) const ACTIVE_UNDECIDED: &str = "count_bg's frame is undecided: a member's mission \
     activity (TaskForceCounterInputs) is not recorded";

/// Mixed-owner port flags choose the owner with lowest total ship strength. An
/// incomplete Ships census makes that choice undecidable.
pub(super) const PORT_FLAG_UNDECIDED: &str = "a mixed-owner port item's flag is undecided: the \
     Ships census its owners' strengths are summed over is incomplete";

/// A flag atlas cell as the texture a flag icon draws in its sprite's place.
fn flag_texture(atlas: &LazyFlags, texture: usize, uv: [f32; 4]) -> Texture {
    Texture {
        texture,
        uv,
        size: atlas.cell_size().map(|value| value as i32),
    }
}

/// Naval combat window state after placement and Show: rotation and progress, then
/// the naval-HQ frame button.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CombatState {
    /// No rotation writes when the icon's province differs from its combat's province.
    pub rotation: Option<Rotation>,
    /// Naval-HQ frame visibility, or why the hour cannot decide it. Every live combat
    /// update writes it; undecidable visibility leaves [`FRAME_BTN`] unknown.
    pub hq: Result<bool, String>,
}

/// Rotation hides and flags both progress bars, then, when engaged, unflags, shows,
/// turns, moves and fills one. Every decided rotation rewrites this part.
pub(super) const PROGRESS: &str = "the combat progress bars";

/// Rotation clears combat grids and refills them for engaged large icons. Every
/// decided rotation rewrites this part.
pub(super) const GRIDS: &str = "the combat grids";

/// `bg_btn`'s frame is the convoy variant plus one. Only engaged rotations write it;
/// unengaged updates retain it.
pub(super) const BG_FRAME: &str = "bg_btn's frame";

/// A naval result icon's last defined `bg_btn` sprite ([`result_calls`]).
pub(super) const RESULT_SPRITE: &str = "bg_btn's sprite";

/// Each manager update writes a result icon's `bg_btn` sprite and one-based frame.
/// Undefined names retain the drawable and count in `undeclared`; errors leave both
/// sprite and frame unknown.
pub(super) fn result_calls(
    window: &native::Element,
    gui: &dyn native_gui::Resources,
    presentation: Result<(&'static str, u32), &str>,
    undeclared: &mut BTreeMap<&'static str, usize>,
) -> Update {
    use native::Kind::Button;
    let mut writes = Writes::new(window);
    match presentation {
        Ok((sprite, frame)) => {
            writes.control(Button, "bg_btn", |bound| {
                bound.sprite = Some(sprite.as_bytes().to_vec());
                bound.setter = native_gui::SpriteSetter::ButtonStandard;
            });
            if gui.sprite(sprite.as_bytes()).is_some() {
                writes.wrote(RESULT_SPRITE);
            } else {
                *undeclared.entry(sprite).or_default() += 1;
                writes.kept(RESULT_SPRITE);
            }
            writes.frame(
                Button,
                "bg_btn",
                i32::try_from(frame + 1).unwrap_or(i32::MAX),
            );
            writes.wrote(BG_FRAME);
        }
        Err(why) => {
            writes.unknown(RESULT_SPRITE, why);
            writes.unknown(BG_FRAME, why);
        }
    }
    writes.update()
}

/// `frame_btn`'s naval-HQ state, written on every live combat icon update.
pub(super) const FRAME_BTN: &str = "frame_btn's naval-HQ frame";

/// Rotation hides and flags both bars and clears grids. Viewer engagement also
/// supplies `bg_btn`'s frame and the shown progress.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Rotation {
    pub engaged: Option<Engaged>,
}

/// The state written for an engaged combat.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Engaged {
    /// `bg_btn`'s zero-based variant; the button takes one-based frame `variant + 1`.
    pub variant: u32,
    /// The progress shown, or why the hour cannot decide which bar or how (no bar is written:
    /// [`PROGRESS`] is unknown).
    pub progress: Result<CombatProgress, String>,
}

/// The progress bar shown and filled by a rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CombatProgress {
    /// Selects `progress_small` rather than `progress`.
    pub small: bool,
    /// The combat's progress, fixed-point `1e5`, as the attacker's side reads it.
    pub progress: i64,
    /// The side walk reached the viewer or a vision-sharing country: turn the bar half
    /// a rotation, move it by its size and fill it from the other side.
    pub flip: bool,
}

impl CombatProgress {
    /// The progress percentage used as the bar's frame, rounded half away from zero
    /// and narrowed to its low 32 bits.
    pub fn value(&self) -> i32 {
        let progress = if self.flip {
            100_000 - self.progress
        } else {
            self.progress
        };
        let fixed = progress.wrapping_mul(10_000_000) / 100_000;
        let rounded = if fixed < 0 {
            (fixed - 50_000) / 100_000
        } else {
            (fixed + 50_000) / 100_000
        };
        rounded as i32
    }
}

/// Ordered [`CombatState`] calls: hide and flag both bars; for engagement, set
/// `bg_btn`'s frame, unflag and show one bar, turn it by zero or pi, move a flipped
/// bar from its saved place by width and negative height, and set its value. Then
/// show/unflag or hide/flag `frame_btn`. Buttons resolve the background and HQ;
/// icons resolve bars. Every rotation writes [`PROGRESS`], engagement writes
/// [`BG_FRAME`], and every update writes [`FRAME_BTN`]. Undecidable progress or an
/// unmeasurable flipped bar leaves progress unknown while other calls continue;
/// undecidable HQ leaves its frame unknown. The caller owns [`GRIDS`].
pub(super) fn combat_calls(
    window: &native::Element,
    state: &CombatState,
    resources: &dyn native_gui::Resources,
) -> Update {
    use native::Kind::{Button, Icon};
    let mut writes = Writes::new(window);
    if let Some(rotation) = &state.rotation {
        let mut measured = true;
        writes.hide_and_flag(Icon, "progress");
        writes.hide_and_flag(Icon, "progress_small");
        if let Some(engaged) = &rotation.engaged {
            if let Ok(frame) = i32::try_from(engaged.variant + 1) {
                writes.frame(Button, "bg_btn", frame);
            }
            writes.wrote(BG_FRAME);
            if let Err(why) = &engaged.progress {
                measured = false;
                writes.unknown(
                    PROGRESS,
                    format!("icon omitted: its progress bar is undecided: {why}"),
                );
            }
            if let Ok(progress) = engaged.progress {
                let name = if progress.small {
                    "progress_small"
                } else {
                    "progress"
                };
                writes.unflag_and_show(Icon, name);
                let extent = match window.find(Icon, name) {
                    Some(element) if progress.flip => native_gui::drawable_extent(
                        element,
                        resources,
                        &native_gui::Bindings::default(),
                    )
                    .map_err(|refusals| {
                        refusals
                            .iter()
                            .map(|refusal| refused_cause(NAVAL_COMBAT_WINDOW, refusal))
                            .collect::<Vec<_>>()
                            .join("; ")
                    }),
                    _ => Ok(None),
                };
                let extent = match extent {
                    Ok(extent) => Some(extent),
                    Err(why) => {
                        measured = false;
                        writes.unknown(
                            PROGRESS,
                            format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not update: {why}"),
                        );
                        None
                    }
                };
                writes.control(Icon, name, |bar| {
                    // Its saved place, moved by `(w, -h)` when flipped (a bar with no drawable
                    // measures nothing); unmeasured, the move is not written.
                    if progress.flip {
                        bar.rotation = Some(std::f32::consts::PI);
                        if let Some(extent) = extent {
                            let [w, h] = extent.unwrap_or([0; 2]);
                            bar.offset = Some([w, h.wrapping_neg()]);
                        }
                    } else {
                        bar.rotation = Some(0.0);
                        bar.offset = Some([0; 2]);
                    }
                });
                writes.frame(Icon, name, progress.value());
            }
        }
        if measured {
            writes.wrote(PROGRESS);
        }
    }
    match &state.hq {
        Ok(true) => writes.unflag_and_show(Button, "frame_btn"),
        Ok(false) => writes.hide_and_flag(Button, "frame_btn"),
        Err(why) => writes.unknown(
            FRAME_BTN,
            format!("icon omitted: HQ frame undecided: {why}"),
        ),
    }
    if state.hq.is_ok() {
        writes.wrote(FRAME_BTN);
    }
    writes.update()
}

/// Convoy-counter construction hides the spotting bar.
pub(super) fn convoy_setup(tiny: &native::Element) -> Vec<(String, Call)> {
    let mut writes = Writes::new(tiny);
    writes.hide(native::Kind::Container, "spotting_bar_container");
    writes.finish()
}

/// The convoy addition's `background` takes `tint`, its `count` takes the live
/// convoy count, and its `icon` takes [`CONVOY_ADDITION_ICON`]. Undefined transport
/// sprites retain the drawable and count in `gaps`.
pub(super) fn convoy_calls(
    gui: &GuiResources,
    tiny: &native::Element,
    count: usize,
    tint: [f32; 4],
    gaps: &mut BTreeMap<Cow<'static, str>, usize>,
) -> Vec<(String, Call)> {
    use native::Kind::{Icon, TextBox};
    let mut writes = Writes::new(tiny);
    writes.control(Icon, "background", |bound| bound.color = Some(tint));
    writes.control(TextBox, "count", |bound| {
        bound.text = Some(count.to_string())
    });
    if tiny.find(Icon, "icon").is_some() {
        // A defined name replaces the sprite even without a loaded sheet; the missing
        // sheet then refuses the window by name.
        if gui
            .registry()
            .sprite(CONVOY_ADDITION_ICON.as_bytes())
            .is_some()
        {
            writes.control(Icon, "icon", |bound| {
                bound.sprite = Some(CONVOY_ADDITION_ICON.as_bytes().to_vec());
                bound.setter = native_gui::SpriteSetter::Icon;
            });
        } else {
            *gaps
                .entry(naval_markers::CONVOY_ICON_GAP.into())
                .or_default() += 1;
        }
    }
    writes.finish()
}

/// Counted for each convoy addition when the content declares no `GFX_mapicon_tiny_unit_counter`
/// window, so adding the convoy combatant cannot create its grid item.
pub(super) const CONVOY_WINDOW_ABSENT: &str =
    "a convoy addition's GFX_mapicon_tiny_unit_counter window is not declared; not drawn";

#[cfg(test)]
pub(super) mod tests;
