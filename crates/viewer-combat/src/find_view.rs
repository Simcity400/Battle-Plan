
pub mod index;

use super::{
    Drawn, Values, Widget,
    binding::Bindings,
    gui::{self, Group, Retained, Row},
    gui_resources,
    picker::{Control as PickerControl, Placed},
};
use index::{Entry, Target};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_native_gui::native_gui::{self, lifecycle, pointer::Input};

pub const WINDOW: &str = "find_view";
pub const ITEM: &str = "find_view_item";
pub const TEMPLATES: &[&[u8]] = &[b"find_view", b"find_view_item"];

pub fn victory_point_icon(frame: u32) -> String {
    format!("{}onmap_victorypoints_strip|{frame}", viewer_native_gui::native_text::ICON)
}

const FIELD: &str = "to_find_text";
const GRID: &str = "results_window/results_grid_box";
const RESULTS: &str = "results_window";

const CARET_SHOWN: f32 = 0.5;
const CARET_PERIOD: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Field,
    Result(usize),
    Find,
    Home,
    Back,
    Window,
}

impl Control {
    pub fn name(self) -> String {
        match self {
            Control::Field => "field".to_owned(),
            Control::Result(index) => format!("result:{index}"),
            Control::Find => "find".to_owned(),
            Control::Home => "home".to_owned(),
            Control::Back => "back".to_owned(),
            Control::Window => "window".to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    text: String,
    caret: usize,
    pub focused: bool,
    blink: f32,
}

impl Field {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn open(&mut self, now: f32) {
        *self = Self {
            focused: true,
            blink: now,
            ..Self::default()
        };
    }

    pub fn focus(&mut self, focused: bool, now: f32) {
        if focused && !self.focused {
            self.blink = now;
        }
        self.focused = focused;
    }

    pub fn press(&mut self, caret: usize, now: f32) {
        self.focus(true, now);
        self.caret = caret.min(self.text.chars().count());
        self.blink = now;
    }

    fn byte(&self, caret: usize) -> usize {
        self.text
            .char_indices()
            .nth(caret)
            .map_or(self.text.len(), |(at, _)| at)
    }

    pub fn insert(&mut self, text: &str, now: f32) -> bool {
        let typed: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if typed.is_empty() {
            return false;
        }
        let at = self.byte(self.caret);
        self.text.insert_str(at, &typed);
        self.caret += typed.chars().count();
        self.blink = now;
        true
    }

    pub fn key(&mut self, key: EditKey, now: f32) -> bool {
        let count = self.text.chars().count();
        self.blink = now;
        match key {
            EditKey::Backspace if self.caret > 0 => {
                let from = self.byte(self.caret - 1);
                let to = self.byte(self.caret);
                self.text.replace_range(from..to, "");
                self.caret -= 1;
                true
            }
            EditKey::Delete if self.caret < count => {
                let from = self.byte(self.caret);
                let to = self.byte(self.caret + 1);
                self.text.replace_range(from..to, "");
                true
            }
            EditKey::Left => {
                self.caret = self.caret.saturating_sub(1);
                false
            }
            EditKey::Right => {
                self.caret = (self.caret + 1).min(count);
                false
            }
            EditKey::Home => {
                self.caret = 0;
                false
            }
            EditKey::End => {
                self.caret = count;
                false
            }
            _ => false,
        }
    }

    pub fn caret(&self, now: f32) -> Option<usize> {
        let phase = (now - self.blink).max(0.0) % CARET_PERIOD;
        (self.focused && phase < CARET_SHOWN).then_some(self.caret)
    }

    pub fn blinking(&self) -> bool {
        self.focused
    }
}

pub struct State<'a> {
    pub text: &'a str,
    pub query: &'a str,
    pub caret: Option<usize>,
    pub results: &'a [String],
    pub fingerprint: u64,
}

#[derive(Clone, Default)]
pub struct FindDrawn {
    pub controls: Vec<Placed>,
    pub clicked: Vec<(PickerControl, usize)>,
    pub caret: Option<usize>,
}

pub fn dragged(at: [i32; 2], size: [i32; 2], moved: [i32; 2], parent: [i32; 2]) -> [i32; 2] {
    let x = at[0] + moved[0];
    let y = at[1] + moved[1];
    let x = if x < 0 {
        0
    } else if parent[0] <= x + size[0] {
        parent[0] - size[0]
    } else {
        x
    };
    let y = if y < 0 {
        0
    } else if parent[1] < y + size[1] {
        parent[1] - size[1]
    } else {
        y
    };
    [x, y]
}

#[derive(Default)]
pub struct FindView {
    built: Option<(
        [i32; 2],
        viewer_native_gui::native_windows::NativeWindows,
    )>,
    window: Option<Retained>,
    generation: u64,
    last: Option<(u64, String, Option<usize>)>,
    controls: BTreeMap<String, Control>,
    position: Option<[i32; 2]>,
    dragging: Option<[i32; 2]>,
    slider: Option<(i32, native_gui::Slide)>,
    results: Option<[f32; 4]>,
}

fn pixel(at: [f32; 2]) -> [i32; 2] {
    at.map(|value| value as i32)
}

impl FindView {
    pub fn holding(&self) -> bool {
        self.slider.is_some() || self.window.as_ref().is_some_and(Retained::holding)
    }

    pub fn pointing(&self) -> bool {
        self.window.as_ref().is_some_and(Retained::pointing)
    }

    pub fn wheel(
        &mut self,
        content: &super::content::Content,
        extent: [f32; 2],
        point: [f32; 2],
        notches: i32,
    ) -> io::Result<()> {
        if !self
            .results
            .is_some_and(|rect| viewer_runtime::geometry::contains(rect, point))
        {
            return Ok(());
        }
        let Some(window) = &mut self.window else {
            return Ok(());
        };
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        let assets = content.assets.as_ref();
        let inventory = gui_resources::GuiAssets::new(assets, &Bindings::default());
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &content.localisation,
            &content.small_flags,
            &content.large_flags,
            &inventory,
        );
        let root = gui::template(&windows, WINDOW).map_err(super::invalid)?;
        let Some(results) = root.path_named(RESULTS) else {
            return Ok(());
        };
        for _ in 0..notches.unsigned_abs() {
            window
                .instance
                .wheel_scroll(root, results, true, -notches.signum(), &resources)
                .map_err(|why| super::invalid(format!("{RESULTS}: {why:?}")))?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one frame's view, pointer and clock, as the picker's draw takes them"
    )]
    pub fn draw(
        &mut self,
        content: &super::content::Content,
        state: &State<'_>,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
        drawn: &mut Drawn,
        refused: &mut BTreeMap<String, usize>,
    ) -> io::Result<FindDrawn> {
        let viewport = extent.map(|value| value as i32);
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        if self
            .built
            .as_ref()
            .is_none_or(|(size, built)| *size != viewport || !built.same(&windows))
        {
            *self = Self {
                built: Some((viewport, windows.clone())),
                ..Self::default()
            };
        }
        let shown = (state.fingerprint, state.text.to_owned(), state.caret);
        if self.last.as_ref() != Some(&shown) {
            self.last = Some(shown);
            self.generation += 1;
        }
        let assets = content.assets.as_ref();
        let inventory = gui_resources::GuiAssets::new(assets, &Bindings::default());
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &content.localisation,
            &content.small_flags,
            &content.large_flags,
            &inventory,
        );
        let mut out = FindDrawn::default();
        let root = gui::template(&windows, WINDOW).map_err(super::invalid)?;
        if self.controls.is_empty() {
            self.controls = gui::elements(root)
                .into_iter()
                .filter_map(|(path, element)| {
                    let control = match path.as_str() {
                        "find_button" => Control::Find,
                        "goto_home_button" => Control::Home,
                        "back_button" => Control::Back,
                        _ => return None,
                    };
                    Some((element.path.clone(), control))
                })
                .collect();
        }
        let retained = gui::ensure(&mut self.window, WINDOW, root, viewport, &resources)
            .map_err(super::invalid)?;
        let frame = native_gui::container_frame(root, Some(viewport)).map_err(super::invalid)?;
        let background = root
            .container()
            .and_then(|container| container.backgrounds.first())
            .map(|background| background.path.clone());
        let held = retained.holding()
            && background.is_some()
            && retained.highlighted() == background.as_deref();
        let place = input
            .as_ref()
            .and_then(|input| input.place)
            .map(|place| pixel(place.at));
        let mut origin = self.position.unwrap_or(frame.at);
        match (held, place) {
            (true, Some(now)) => {
                if let Some(last) = self.dragging {
                    origin = dragged(
                        origin,
                        frame.size,
                        [now[0] - last[0], now[1] - last[1]],
                        viewport,
                    );
                    self.position = Some(origin);
                }
                self.dragging = Some(now);
            }
            (true, None) => {}
            (false, _) => self.dragging = None,
        }
        retained
            .instance
            .set_position(root, root, origin)
            .map_err(super::invalid)?;
        let mut values = button_texts(root, &content.localisation, refused);
        values.insert(
            FIELD.to_owned(),
            Widget {
                text: Some(state.text.to_owned()),
                caret: state.caret,
                ..Default::default()
            },
        );
        let rows: Vec<Values> = state
            .results
            .iter()
            .map(|text| {
                let mut values = Values::new();
                values.insert(
                    "name".to_owned(),
                    Widget {
                        text: Some(text.clone()),
                        ..Default::default()
                    },
                );
                values
            })
            .collect();
        let groups = vec![Group {
            path: GRID.into(),
            rows: rows
                .iter()
                .enumerate()
                .map(|(index, values)| Row {
                    naval_layout: None,
                    key: index.to_string(),
                    template: ITEM,
                    values,
                    explicit: None,
                })
                .collect(),
        }];
        let tick = lifecycle::Tick { time, dt: 0.0 };
        let local = input
            .as_ref()
            .map(|input| input.local(origin.map(|value| value as f32)));
        let Some(prepared) = retained.prepare(
            root,
            &values,
            groups,
            &windows,
            &resources,
            tick,
            self.generation,
            advance,
            hovers,
            true,
            local,
            refused,
        ) else {
            return Ok(out);
        };
        drawn.sounds.extend(prepared.sounds.iter().cloned());
        let results_element = root.path_named(RESULTS);
        let scroll_of = |path: &str| {
            prepared
                .layout
                .regions
                .iter()
                .find(|region| region.path == path)
                .and_then(|region| region.scroll)
        };
        for (key, path, order) in &prepared.clicks {
            match key {
                Some(key) => {
                    if let Ok(index) = key.parse() {
                        out.clicked
                            .push((PickerControl::Find(Control::Result(index)), *order));
                    }
                }
                None => {
                    if let Some(control) = self.controls.get(path) {
                        out.clicked.push((PickerControl::Find(*control), *order));
                    } else if let (Some((vertical, direction)), Some(results)) =
                        (scroll_of(path), results_element)
                        && let Err(why) = retained.instance.step_scroll(
                            root,
                            results,
                            vertical,
                            direction,
                            &resources,
                        )
                    {
                        viewer_native_gui::gui_resources::count_refused(
                            refused, RESULTS, &why,
                        );
                    }
                }
            }
        }
        let axis =
            |slide: native_gui::Slide, at: [i32; 2]| at[usize::from(slide.vertical)];
        let sliders: Vec<([f32; 4], native_gui::Slide)> = prepared
            .layout
            .regions
            .iter()
            .filter_map(|region| Some((gui::region_rect(region, origin)?, region.slide?)))
            .collect();
        for button in input.iter().flat_map(|input| &input.buttons) {
            if button.down {
                self.slider = button.place.and_then(|place| {
                    let (_, slide) = sliders.iter().rev().find(|(rect, _)| {
                        viewer_runtime::geometry::contains(*rect, place.at)
                    })?;
                    Some((axis(*slide, pixel(place.at)), *slide))
                });
            } else {
                self.slider = None;
            }
        }
        if input.as_ref().is_some_and(|input| !input.held) {
            self.slider = None;
        }
        if let (Some((from, slide)), Some(now), Some(results)) =
            (self.slider, place, results_element)
            && let Err(why) = retained.instance.scroll_to(
                root,
                results,
                slide.vertical,
                |range| Some(slide.dragged(axis(slide, now) - from, range)),
                &resources,
            )
        {
            viewer_native_gui::gui_resources::count_refused(refused, RESULTS, &why);
        }
        if !gui::draw_prepared(
            &prepared.layout,
            &prepared.items,
            origin,
            &resources,
            drawn,
        ) {
            return Ok(out);
        }
        let size = prepared.layout.size;
        out.controls.push(Placed::new(
            [
                origin[0] as f32,
                origin[1] as f32,
                size[0] as f32,
                size[1] as f32,
            ],
            PickerControl::Find(Control::Window),
        ));
        self.results = prepared
            .layout
            .regions
            .iter()
            .find(|region| results_element.is_some_and(|results| results.path == region.path))
            .and_then(|region| gui::region_rect(region, origin));
        if let Some(field) = root.path_named(FIELD)
            && let content_reader_gui::gui::native::Fields::EditBox(edit) = &field.fields
        {
            let [x, y] = edit.position();
            let [w, h] = edit.size();
            let rect = [
                (origin[0] + x) as f32,
                (origin[1] + y) as f32,
                w as f32,
                h as f32,
            ];
            out.controls
                .push(Placed::new(rect, PickerControl::Find(Control::Field)));
            let press = input
                .iter()
                .flat_map(|input| &input.buttons)
                .rev()
                .find_map(|button| {
                    let place = button.place.filter(|_| button.down)?;
                    viewer_runtime::geometry::contains(rect, place.at)
                        .then(|| pixel(place.at))
                });
            if let Some(point) = press {
                match native_gui::widgets::edit_caret_at(
                    &resources,
                    field,
                    state.query,
                    origin,
                    frame.content_size,
                    point,
                ) {
                    Ok(caret) => out.caret = Some(caret),
                    Err(why) => viewer_native_gui::gui_resources::count_refused(
                        refused,
                        FIELD,
                        &[why],
                    ),
                }
            }
        }
        for region in &prepared.layout.regions {
            let Some(rect) = gui::region_rect(region, origin) else {
                continue;
            };
            if let Some(control) = self.controls.get(&region.path) {
                let click = gui::click_sound(root, &region.path, &resources);
                out.controls.extend(Placed::region(
                    rect,
                    PickerControl::Find(*control),
                    region,
                    origin,
                    click,
                ));
            }
        }
        for (_, key, region) in &prepared.entries {
            if let (Some(rect), Ok(index)) = (gui::region_rect(region, origin), key.parse()) {
                out.controls.push(Placed::new(
                    rect,
                    PickerControl::Find(Control::Result(index)),
                ));
            }
        }
        if hovers {
            gui::append_hovers(&mut drawn.tooltips, prepared.hovers, origin);
        }
        Ok(out)
    }
}

fn button_texts(
    root: &content_reader_gui::gui::native::Element,
    localisation: &content_reader_localisation::localisation::Localisation,
    refused: &mut BTreeMap<String, usize>,
) -> Values {
    gui::elements(root)
        .into_iter()
        .filter_map(|(path, element)| {
            let button = element.button()?;
            if button.font.is_empty() || button.text.is_empty() {
                return None;
            }
            match localisation.localise(&button.text) {
                Ok(text) => Some((
                    path,
                    Widget {
                        text: Some(text),
                        ..Default::default()
                    },
                )),
                Err(why) => {
                    *refused.entry(format!("{path}: {why}")).or_insert(0) += 1;
                    None
                }
            }
        })
        .collect()
}

pub fn shortcuts(
    content: &super::content::Content,
    extent: [f32; 2],
) -> io::Result<Vec<(Control, String)>> {
    let windows = content.window_cache.at(extent.map(|value| value as u32))?;
    let root = gui::template(&windows, WINDOW).map_err(super::invalid)?;
    Ok([
        ("find_button", Control::Find),
        ("goto_home_button", Control::Home),
        ("back_button", Control::Back),
    ]
    .into_iter()
    .filter_map(|(name, control)| {
        let shortcut = root.path_named(name)?.button()?.shortcuts.first()?.clone();
        Some((control, String::from_utf8(shortcut).ok()?))
    })
    .collect())
}

pub fn countries_and_states(
    content: &super::content::Content,
    frame: &DecodedFrame,
) -> io::Result<(Vec<Entry>, BTreeMap<String, usize>)> {
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| super::invalid(e.to_string()))?;
    let states = frame
        .complete::<tables::States>()
        .map_err(|e| super::invalid(e.to_string()))?;
    let politics = frame
        .complete::<tables::Politics>()
        .map_err(|e| super::invalid(e.to_string()))?;
    let mut refused = BTreeMap::new();
    let mut refuse = |why: String| *refused.entry(why).or_insert(0) += 1;
    let mut owned: BTreeMap<u32, usize> = BTreeMap::new();
    for state in states {
        *owned.entry(state.owner).or_default() += 1;
    }
    let capitals: BTreeMap<u32, u32> = politics
        .iter()
        .filter(|row| row.capital > 0)
        .map(|row| (row.country, row.capital as u32))
        .collect();
    let names = content_reader_country_names::country_names::CountryNames::read(
        frame,
        &content.localisation,
    );
    let flags = super::binding::flags(content, frame)?;
    let mut entries = Vec::new();
    let mut listed: Vec<_> = countries.iter().filter(|row| row.id >= 1).collect();
    listed.sort_by_key(|row| row.id);
    for country in listed {
        if owned.get(&country.id).copied().unwrap_or(0) == 0 {
            continue;
        }
        let tag = String::from_utf8_lossy(&country.tag).into_owned();
        let display = names.name(country.canonical_slot).and_then(|name| {
            let flag = super::binding::country_flag(&flags, country.id)?;
            Ok((name.clone(), format!("{}{flag} {name}", super::INLINE_FLAG)))
        });
        match display {
            Ok((name, display)) => entries.push(Entry {
                name,
                secondary: tag,
                display,
                target: Target::Country {
                    capital: capitals.get(&country.id).copied(),
                },
            }),
            Err(why) => refuse(format!("country {tag} is left out: {why}")),
        }
    }
    let mut listed: Vec<_> = states.iter().collect();
    listed.sort_by_key(|row| row.id);
    for state in listed {
        let id = state.id;
        let key = content.state_name_keys.get(&id);
        let name = super::battle_name::state_display_name(
            content.assets.as_ref(),
            &content.localisation,
            &super::battle_name::StateNames {
                id,
                custom_name: &state.custom_name,
                owner_name: &state.owner_name,
                recorded_language: &content.recorded_language,
                name_key: match key {
                    Some(Ok(key)) => Ok(key.as_str()),
                    Some(Err(why)) => Err(why.as_str()),
                    None => Err("the state has no content definition"),
                },
            },
        );
        match name {
            Ok(name) => entries.push(Entry {
                display: name.clone(),
                name,
                secondary: id.to_string(),
                target: Target::State(id),
            }),
            Err(why) => refuse(format!("state {id} is left out: {why}")),
        }
    }
    Ok((entries, refused))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_types_at_its_caret_and_edits_by_codepoint() {
        let mut field = Field::default();
        field.open(0.0);
        assert!(field.focused);
        assert!(field.insert("Kln", 0.0));
        field.key(EditKey::Left, 0.0);
        field.key(EditKey::Left, 0.0);
        assert!(field.insert("ö", 0.0));
        assert_eq!(field.text(), "Köln");
        assert!(field.key(EditKey::Backspace, 0.0));
        assert_eq!(field.text(), "Kln");
        field.key(EditKey::End, 0.0);
        assert!(!field.key(EditKey::Delete, 0.0));
        field.key(EditKey::Home, 0.0);
        assert!(field.key(EditKey::Delete, 0.0));
        assert_eq!(field.text(), "ln");
        assert!(!field.insert("\u{8}\r", 0.0));
    }

    #[test]
    fn the_buttons_show_their_authored_text_localised() {
        content_reader_script::test_game::enter();
        let windows = viewer_native_gui::native_windows::NativeWindows::read(
            b"guiTypes = { containerWindowType = { name = unit_counter_navy }               containerWindowType = { name = naval_combat_mapicon }               containerWindowType = { name = find_view                 buttonType = { name = find_button buttonText = \"FIND\" buttonFont = \"hoi_16mbs\" }                 buttonType = { name = goto_home_button buttonText = \"GOTO_CAPITAL\" buttonFont = \"hoi_16mbs\" }                 buttonType = { name = back_button buttonFont = \"hoi_16mbs\" } } }",
            [1280, 720],
        )
        .unwrap();
        let root = gui::template(&windows, WINDOW).unwrap();
        let localisation: content_reader_localisation::localisation::Localisation =
            [("FIND", "Find")]
                .map(|(key, text)| (key.to_owned(), text.to_owned()))
                .into_iter()
                .collect();
        let mut refused = BTreeMap::new();
        let values = button_texts(root, &localisation, &mut refused);
        assert_eq!(values["find_button"].text.as_deref(), Some("Find"));
        assert_eq!(
            values["goto_home_button"].text.as_deref(),
            Some("GOTO_CAPITAL")
        );
        assert!(refused.is_empty());
        assert!(!values.contains_key("back_button"));
    }

    #[test]
    fn a_dragged_window_moves_with_the_pointer_and_stays_on_its_parent() {
        let parent = [1920, 1080];
        let size = [285, 300];
        assert_eq!(dragged([1000, 500], size, [10, -20], parent), [1010, 480]);
        assert_eq!(dragged([5, 5], size, [-10, -10], parent), [0, 0]);
        assert_eq!(dragged([1600, 700], size, [35, 80], parent), [1635, 780]);
        assert_eq!(dragged([1600, 700], size, [36, 81], parent), [1635, 780]);
        assert_eq!(dragged([1600, 700], size, [34, 80], parent), [1634, 780]);
    }

}
