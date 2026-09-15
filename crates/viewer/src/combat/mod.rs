//! Game-authored land and naval combat windows, shared by playback and PNG export.
mod assets;
mod binding;
mod details;
pub(crate) mod land_details;
mod layout;
mod naval_details;
mod naval_environment;
mod presentation;
mod texture;

use crate::{
    document::Document,
    map::{sprites::Batch, text},
};
pub use assets::Assets;
use assets::Sprite;
use binding::Bindings;
use layout::{Node, number, string};
use std::{collections::BTreeMap, io};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Land(u64),
    Naval(u64),
    NavalResult(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    Id(u64),
    Province(u32),
}

impl Selector {
    pub fn resolve(self, document: &Document) -> io::Result<Selection> {
        use crate::map::tables::rows;
        use replay::rows::tables;
        let frame = document.frame()?;
        let mut matches = Vec::new();
        for row in rows::<tables::LandCombats>(frame)?.rows() {
            if match self {
                Self::Id(id) => id == row.id,
                Self::Province(id) => id == row.province,
            } {
                matches.push(Selection::Land(row.id));
            }
        }
        for row in rows::<tables::NavalCombats>(frame)?.rows() {
            if match self {
                Self::Id(id) => id == row.id,
                Self::Province(id) => id == row.province,
            } {
                matches.push(Selection::Naval(row.id));
            }
        }
        if matches.is_empty() {
            let results = rows::<tables::NavalResults>(frame)?;
            let newest = results
                .rows()
                .iter()
                .filter(|row| {
                    row.result_hour <= frame.hour
                        && row.readiness != 0
                        && match self {
                            Self::Id(id) => id == row.id,
                            Self::Province(id) => id == row.province,
                        }
                })
                .max_by_key(|row| (row.result_hour, row.id));
            if let Some(row) = newest {
                matches.push(Selection::NavalResult(row.id));
            }
        }
        match matches.as_slice() {
            [selection] => Ok(*selection),
            [] => Err(invalid(format!(
                "no recorded combat matches {self:?} at hour {}",
                frame.hour
            ))),
            _ => Err(invalid(format!(
                "{self:?} matches multiple combats; use --combat with the full combat handle"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub selection: Selection,
    pub rect: [f32; 4],
}
impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        layout::contains(self.rect, point)
    }
}

#[derive(Default)]
pub struct Window {
    pub selection: Option<Selection>,
    pub offset: [f32; 2],
    pub scroll: [usize; 4],
    pub dragging: bool,
    cached: Option<(
        replay::BranchId,
        i32,
        Selection,
        crate::map::perspective::Choice,
        Bindings,
    )>,
}
impl Window {
    pub fn warnings(&self) -> Vec<String> {
        self.cached
            .as_ref()
            .map(|cache| cache.4.warnings.clone())
            .unwrap_or_default()
    }
    pub fn open(&mut self, selection: Selection) {
        *self = Self {
            selection: Some(selection),
            ..Self::default()
        };
    }
    pub fn close(&mut self) {
        *self = Self::default();
    }
    pub fn draw(
        &mut self,
        document: &Document,
        viewport: [u32; 2],
        perspective: &crate::map::perspective::Choice,
    ) -> io::Result<Option<Drawn>> {
        let Some(selection) = self.selection else {
            return Ok(None);
        };
        let branch = document.playback.branch();
        let hour = document.playback.hour();
        if !self.cached.as_ref().is_some_and(|(b, h, s, p, _)| {
            (*b, *h, *s) == (branch, hour, selection) && p == perspective
        }) {
            self.cached = Some((
                branch,
                hour,
                selection,
                perspective.clone(),
                Bindings::read(document, selection, &document.vision(perspective)?)?,
            ));
        }
        let bindings = &self.cached.as_ref().expect("bound above").4;
        let assets = &document.icons.combat;
        let root = &assets.windows[match selection {
            Selection::Land(_) => "landcombatview",
            Selection::Naval(_) => "navalcombatview",
            Selection::NavalResult(_) => "navalcombatresultswindow",
        }];
        let mut rect = root.rect([0.0, 0.0, viewport[0] as f32, viewport[1] as f32])?;
        rect[0] += self.offset[0];
        rect[1] += self.offset[1];
        let mut drawn = Drawn {
            rect,
            ..Drawn::default()
        };
        let mut painter = Painter {
            assets,
            drawn: &mut drawn,
        };
        painter.node(root, rect, "", &bindings.values)?;
        for side in 0..4 {
            if !matches!(selection, Selection::Land(_)) {
                break;
            }
            let rows = &bindings.entries[side];
            let grid = format!(
                "{}_window/{}_window/list",
                if side % 2 == 0 {
                    "attacker"
                } else {
                    "defender"
                },
                if side < 2 { "committed" } else { "reserves" }
            );
            let entry = if side >= 2 {
                "reserve_combat_entry"
            } else if side == 0 {
                "division_combat_attacker_entry"
            } else {
                "division_combat_defender_entry"
            };
            let grid_rect = root.path_rect(rect, &grid)?;
            let grid_node = root.child(&grid).expect("path resolved above");
            let columns = number(&grid_node.items, "max_slots_horizontal", 1.0).max(1.0) as usize;
            let slot_width = installed_content::gui::slot_size(&grid_node.items)?
                .map(|(x, _)| match x {
                    installed_content::gui::Length::Pixels(x) => x as f32,
                    _ => 0.0,
                })
                .unwrap_or(grid_rect[2]);
            let step = installed_content::gui::slot_size(&grid_node.items)?
                .map(|(_, y)| match y {
                    installed_content::gui::Length::Pixels(y) => y as f32,
                    _ => 40.0,
                })
                .unwrap_or(40.0)
                .max(1.0);
            let parent_grid = grid.rsplit_once('/').expect("nested list").0;
            let parent_rect = root.path_rect(rect, parent_grid)?;
            let height = parent_rect[1] + parent_rect[3] - grid_rect[1];
            let clip = if side < 2 {
                [grid_rect[0] - 32.0, grid_rect[1], 280.0, height]
            } else {
                parent_rect
            };
            painter.drawn.lists[side] = Some(clip);
            let count = (height / step).ceil() as usize * columns;
            // A list scrolls once its rows overflow the box. pairJ_land_6.png
            // shows the attacker's slider for five 40-high committed rows in
            // the 165-high list, though a fifth row is partly drawn without it.
            let hidden_rows = rows
                .len()
                .div_ceil(columns)
                .saturating_sub((height / step).floor() as usize);
            let start = self.scroll[side].min(hidden_rows) * columns;
            for (index, row) in rows.iter().skip(start).take(count).enumerate() {
                let entry_node = &assets.windows[entry];
                let size = entry_node.size([grid_rect[2], step])?;
                let at = [
                    grid_rect[0]
                        + ((grid_rect[2] - size[0]) * 0.5).trunc()
                        + (index % columns) as f32 * slot_width,
                    grid_rect[1] + (index / columns) as f32 * step,
                    size[0],
                    size[1],
                ];
                let first = painter.drawn.batch.quads.len();
                painter.node(entry_node, at, "", row)?;
                clip_quads(&mut painter.drawn.batch, first, clip);
            }
            if hidden_rows > 0 {
                let slider = &assets.windows[if side % 2 == 0 {
                    "left_vertical_slider"
                } else {
                    "right_vertical_slider"
                }];
                let mut slider_rect = slider.rect(parent_rect)?;
                if side % 2 == 1 {
                    slider_rect[0] -= slider_rect[2];
                }
                slider_rect[3] = parent_rect[3];
                let mut values = Values::new();
                values.insert(
                    "Background".into(),
                    Widget {
                        size: Some([slider_rect[2], slider_rect[3]]),
                        ..Widget::default()
                    },
                );
                values.insert(
                    "Track".into(),
                    Widget {
                        size: Some([6.0, (slider_rect[3] - 32.0).max(0.0)]),
                        offset: [0.0, 16.0],
                        ..Widget::default()
                    },
                );
                values.insert(
                    "Increase".into(),
                    Widget {
                        offset: [slider_rect[2], slider_rect[3]],
                        ..Widget::default()
                    },
                );
                values.insert(
                    "Slider".into(),
                    Widget {
                        offset: [
                            0.0,
                            20.0 + (slider_rect[3] - 50.0).max(0.0) * (start / columns) as f32
                                / hidden_rows as f32,
                        ],
                        ..Widget::default()
                    },
                );
                painter.node(slider, slider_rect, "", &values)?;
                for (name, direction) in [("Decrease", -1), ("Increase", 1)] {
                    let node = slider.child(name).expect("authored scrollbar button");
                    let mut at = node.origin(slider_rect)?;
                    if let Some(value) = values.get(name) {
                        at[0] += value.offset[0];
                        at[1] += value.offset[1];
                    }
                    if let Some(sprite) =
                        installed_content::gui::sprite(&node.items).and_then(|key| assets.sprites.get(key))
                    {
                        painter.drawn.scroll_buttons.push((
                            [at[0], at[1], sprite.size[0], sprite.size[1]],
                            side,
                            direction,
                        ));
                    }
                }
            }
        }
        for row in &bindings.naval_entries {
            let grid = root.path_rect(rect, &row.grid)?;
            let entry = &assets.windows["navalcombat_ship_entry"];
            let at = [
                grid[0] + row.offset[0],
                grid[1] + row.offset[1],
                row.size[0],
                row.size[1],
            ];
            let first = painter.drawn.batch.quads.len();
            painter.node(entry, at, "", &row.values)?;
            clip_quads(
                &mut painter.drawn.batch,
                first,
                // Native UPPER_RIGHT grids center a sprite on their slot edge;
                // that edge is not a horizontal clipping boundary.
                [rect[0], grid[1], rect[2], grid[3]],
            );
        }
        // Some authored backgrounds extend below the root's nominal content height.
        for quad in &drawn.batch.quads {
            drawn.rect[2] = drawn.rect[2].max(quad.rect[0] + quad.rect[2] - drawn.rect[0]);
            drawn.rect[3] = drawn.rect[3].max(quad.rect[1] + quad.rect[3] - drawn.rect[1]);
        }
        drawn.warnings = bindings.warnings.clone();
        drawn.warnings.extend(assets.warnings.iter().cloned());
        Ok(Some(drawn))
    }
}

#[derive(Default)]
pub struct Drawn {
    pub batch: Batch,
    pub rect: [f32; 4],
    pub close: Option<[f32; 4]>,
    pub lists: [Option<[f32; 4]>; 4],
    pub warnings: Vec<String>,
    pub scroll_buttons: Vec<([f32; 4], usize, i32)>,
}
impl Drawn {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        layout::contains(self.rect, point)
    }
    pub fn closes(&self, point: [f32; 2]) -> bool {
        self.close.is_some_and(|rect| layout::contains(rect, point))
    }
    pub fn list_at(&self, point: [f32; 2]) -> Option<usize> {
        self.lists
            .iter()
            .position(|rect| rect.is_some_and(|rect| layout::contains(rect, point)))
    }
    pub fn scroll_action(&self, point: [f32; 2]) -> Option<(usize, i32)> {
        self.scroll_buttons
            .iter()
            .find(|(rect, _, _)| layout::contains(*rect, point))
            .map(|(_, side, direction)| (*side, *direction))
    }
}

#[derive(Clone, Default)]
pub struct Widget {
    pub text: Option<String>,
    pub sprite: Option<String>,
    pub flag: Option<String>,
    pub fill: Option<f32>,
    pub frame: Option<u32>,
    pub rotation: Option<f32>,
    pub opacity: Option<f32>,
    pub hidden: bool,
    pub size: Option<[f32; 2]>,
    pub offset: [f32; 2],
    pub text_width: Option<f32>,
    /// Repeated authored entries, used by reserve, trait and modifier grids.
    pub entry_template: Option<String>,
    pub entries: Vec<Values>,
}
pub type Values = BTreeMap<String, Widget>;
/// The native naval map icon's two progress widgets, using the same authored
/// progress textures and threshold selection as the detail window.
pub(crate) fn naval_bubble_progress(
    assets: &Assets,
    rect: [f32; 4],
    small: bool,
    progress: i64,
    flip: bool,
) -> io::Result<Batch> {
    let root = &assets.windows["naval_combat_mapicon"];
    let name = if small { "progress_small" } else { "progress" };
    let mut values = Values::new();
    for node in &root.children {
        values.insert(
            node.name.clone(),
            Widget {
                hidden: node.name != name,
                ..Widget::default()
            },
        );
    }
    values.insert(
        name.into(),
        Widget {
            fill: Some(if flip {
                1.0 - progress as f32 / 100_000.0
            } else {
                progress as f32 / 100_000.0
            }),
            rotation: Some(if flip { std::f32::consts::PI } else { 0.0 }),
            ..Widget::default()
        },
    );
    let mut drawn = Drawn::default();
    Painter {
        assets,
        drawn: &mut drawn,
    }
    .node(root, rect, "", &values)?;
    Ok(drawn.batch)
}
/// Native inline country flag marker (`0x12` followed by the country), which
/// CCombatantEntry (1415BA240) writes into land division names. Bindings put
/// the resolved flag-atlas key after it, ended by a space or the string end.
pub(crate) const INLINE_FLAG: char = '\u{12}';
/// The small `gfx/flags/small` cell.
const INLINE_FLAG_SIZE: [f32; 2] = [10.0, 7.0];
/// pairJ_land_5.png: right-aligned "<flag> 169th 'Madison' Division" (117 px
/// of Arial12 text) starts 129 px before its box edge, and "<flag> 900
/// Infantry Division" (101 px) 113 px before it. An attacker's trailing flag
/// starts exactly at the pen after "name ".
const INLINE_FLAG_ADVANCE: f32 = 12.0;
enum Chunk {
    Text(String, [f32; 4]),
    Icon(String),
    Flag(String),
}
struct Painter<'a> {
    assets: &'a Assets,
    drawn: &'a mut Drawn,
}
impl Painter<'_> {
    fn node(&mut self, node: &Node, rect: [f32; 4], path: &str, values: &Values) -> io::Result<()> {
        for child in &node.children {
            let key = if path.is_empty() {
                child.name.clone()
            } else {
                format!("{path}/{}", child.name)
            };
            let data = values.get(&key);
            if data.is_some_and(|d| d.hidden) {
                continue;
            }
            let mut origin = child.origin(node.content_rect(rect))?;
            if let Some(data) = data {
                origin[0] += data.offset[0];
                origin[1] += data.offset[1];
            }
            if child.kind == "containerwindowtype" {
                let first = self.drawn.batch.quads.len();
                self.node(child, child.rect(node.content_rect(rect))?, &key, values)?;
                if string(&child.items, "clipping") == Some("yes") {
                    clip_quads(
                        &mut self.drawn.batch,
                        first,
                        child.rect(node.content_rect(rect))?,
                    );
                }
                continue;
            }
            if child.kind == "gridboxtype" {
                if let Some(data) = data {
                    self.grid(child, child.rect(node.content_rect(rect))?, data)?;
                }
                continue;
            }
            self.drawn
                .batch
                .at_layer(100_000_000 + self.drawn.batch.quads.len() as u32);
            let sprite = data
                .and_then(|d| d.sprite.as_deref())
                .or_else(|| installed_content::gui::sprite(&child.items));
            if let Some(flag) = data
                .and_then(|d| d.flag.as_ref())
                .and_then(|key| self.assets.flags.get(key))
            {
                // 14162F510 offsets the masked-shield quad by (0,-1) on this
                // renderer (-0.5,-0.5 on the other branch) in its y-up vertex
                // space: the flag and overlay land one pixel *below* the
                // authored position. Sub-pixel fits of all six medium flags in
                // pairJ_land_5/6 and pairJ_naval_5 put them 2 px below the old
                // (0,-1) screen shift (UK flag MAE 55.8 -> 6.0).
                let origin = if sprite == Some("GFX_flag_small2") {
                    [origin[0], origin[1] + 1.0]
                } else {
                    origin
                };
                let (texture, uv) = if sprite == Some("GFX_flag_small2") {
                    (flag.texture, flag.uv)
                } else {
                    flag.inline.unwrap_or((flag.texture, flag.uv))
                };
                self.drawn.batch.cell(
                    texture,
                    origin,
                    data.and_then(|d| d.size).unwrap_or([41.0, 26.0]),
                    uv,
                    [1.0; 4],
                );
                // GFX_flag_small2's maskedShieldType draws the authored overlay
                // over the large flag atlas. Inline country markers have no shield.
                if sprite == Some("GFX_flag_small2")
                    && let Some(overlay) = self.assets.sprites.get("GFX_flag_small2_not_masked")
                {
                    self.drawn
                        .batch
                        .at_layer(100_000_000 + self.drawn.batch.quads.len() as u32);
                    self.drawn
                        .batch
                        .scaled(&overlay.sheet, 0, origin, overlay.size, [1.0; 4]);
                }
            } else if let Some(sprite) = sprite.and_then(|name| self.assets.sprites.get(name)) {
                let scale = number(&child.items, "scale", 1.0);
                let size = data
                    .and_then(|d| d.size)
                    .unwrap_or(sprite.size.map(|v| v * scale));
                let rotation = data
                    .and_then(|d| d.rotation)
                    .unwrap_or_else(|| number(&child.items, "rotation", 0.0));
                let at = if string(&child.items, "centerPosition") == Some("yes") {
                    [origin[0] - size[0] * 0.5, origin[1] - size[1] * 0.5]
                } else {
                    origin
                };
                if sprite.progress {
                    let (at, rotation) = progress_turn(at, size, rotation);
                    self.progress(sprite, at, size, rotation, data.and_then(|d| d.fill));
                } else {
                    let frame = data
                        .and_then(|d| d.frame)
                        .unwrap_or(number(&child.items, "frame", 1.0).max(1.0) as u32 - 1);
                    self.drawn.batch.rotated(
                        &sprite.sheet,
                        frame,
                        at,
                        size,
                        rotation,
                        [1.0, 1.0, 1.0, data.and_then(|d| d.opacity).unwrap_or(1.0)],
                    );
                }
                if child.name == "close" || child.name == "close_button" {
                    self.drawn.close = Some([at[0], at[1], size[0], size[1]]);
                }
            }
            if let Some(font) =
                installed_content::gui::font(&child.items).and_then(|name| self.assets.fonts.get(name))
            {
                // .gui text fields contain developer examples ("999", "Supreme Leader").
                // No unbound dynamic widget is allowed to expose those examples.
                let value = data.and_then(|d| d.text.as_deref()).unwrap_or("—");
                let align = match installed_content::gui::format(&child.items) {
                    Some("center" | "centre") => text::Align::Centre,
                    Some("right") => text::Align::Right,
                    _ => text::Align::Left,
                };
                let width = data.and_then(|d| d.text_width).or(installed_content::gui::number(
                    &child.items,
                    "maxWidth",
                )?
                .map(|v| v as f32));
                let value = match width {
                    Some(width) if string(&child.items, "fixedsize") == Some("yes") => {
                        // CInstantTextObject (1422D7A00) replaces an unset box
                        // dimension with 200.
                        let height = installed_content::gui::number(&child.items, "maxHeight")?
                            .filter(|v| *v != 0.0)
                            .map_or(200.0, |v| v as f32);
                        self.fit(font, value, width, height)
                    }
                    _ => value.to_owned(),
                };
                self.text(font, &value, origin, align, width);
            }
        }
        Ok(())
    }
    fn grid(&mut self, node: &Node, rect: [f32; 4], data: &Widget) -> io::Result<()> {
        let Some(entry) = data
            .entry_template
            .as_ref()
            .and_then(|key| self.assets.windows.get(key))
        else {
            return Ok(());
        };
        let size = entry.size([rect[2], rect[3]])?;
        let slot = installed_content::gui::slot_size(&node.items)?
            .map(|(x, y)| {
                let pixels = |v| match v {
                    installed_content::gui::Length::Pixels(v) => v as f32,
                    _ => 0.0,
                };
                [pixels(x), pixels(y)]
            })
            .unwrap_or(size);
        let columns = number(
            &node.items,
            "max_slots_horizontal",
            (rect[2] / slot[0].max(1.0)).floor(),
        )
        .max(1.0) as usize;
        let right =
            string(&node.items, "format").is_some_and(|s| s.to_ascii_lowercase().contains("right"));
        let traits = data.entry_template.as_deref() == Some("unit_leader_trait_entry");
        let pitch = if traits && data.entries.len() > 6 {
            5.0 * slot[0] / data.entries.len() as f32
        } else {
            slot[0]
        };
        for (index, values) in data.entries.iter().enumerate() {
            let column = if traits { index } else { index % columns };
            let x = if right {
                rect[2] - size[0] - column as f32 * pitch
            } else {
                column as f32 * pitch
            };
            let at = [
                rect[0] + x,
                rect[1]
                    + if traits {
                        -5.0
                    } else {
                        (index / columns) as f32 * slot[1]
                    },
                size[0],
                size[1],
            ];
            self.node(entry, at, "", values)?;
        }
        Ok(())
    }
    fn text(
        &mut self,
        font: &text::Font,
        value: &str,
        origin: [f32; 2],
        align: text::Align,
        width: Option<f32>,
    ) {
        if !value.contains(['\n', '£', '§', INLINE_FLAG]) {
            self.drawn.batch.text(
                &text::layout(font, value, origin, align, width),
                font.color(),
            );
            return;
        }
        let mut color = font.color();
        for (line, raw) in value.lines().enumerate() {
            let chunks = self.chunks(font, raw, &mut color);
            let measured: f32 = chunks.iter().map(|c| self.length(font, c)).sum();
            let slack = (width.unwrap_or(0.0) - measured).max(0.0);
            let mut x = origin[0]
                + match align {
                    text::Align::Left => 0.0,
                    text::Align::Centre => slack * 0.5,
                    text::Align::Right => slack,
                };
            let y = origin[1] + line as f32 * font.line_height();
            for chunk in chunks {
                let advance = self.length(font, &chunk);
                let remaining = width.map(|w| (origin[0] + w - x).max(0.0));
                self.drawn
                    .batch
                    .at_layer(100_000_000 + self.drawn.batch.quads.len() as u32);
                match &chunk {
                    Chunk::Icon(name) => {
                        if remaining.is_none_or(|w| w >= advance)
                            && let Some(sprite) = self.assets.sprites.get(name)
                        {
                            self.drawn.batch.scaled(
                                &sprite.sheet,
                                0,
                                [x, y],
                                sprite.size,
                                [1.0; 4],
                            );
                        }
                    }
                    Chunk::Flag(key) => {
                        if remaining.is_none_or(|w| w >= INLINE_FLAG_SIZE[0])
                            && let Some((texture, uv)) =
                                self.assets.flags.get(key).and_then(|flag| flag.inline)
                        {
                            // The 10x7 flag's top is five pixels below the
                            // line top in both Arial12 (pairJ_land_5.png) and
                            // hoi_16mbs (pairJ_naval_5.png ship descriptions).
                            let top = y + 5.0;
                            self.drawn.batch.cell(
                                texture,
                                [x, top],
                                INLINE_FLAG_SIZE,
                                uv,
                                [1.0; 4],
                            );
                        }
                    }
                    Chunk::Text(run, color) => {
                        self.drawn.batch.text(
                            &text::layout(font, run, [x, y], text::Align::Left, remaining),
                            *color,
                        );
                    }
                }
                x += advance;
            }
        }
    }
    /// One line of rich text: colour runs, `£icon£` sprites and inline flags.
    fn chunks(&self, font: &text::Font, raw: &str, color: &mut [f32; 4]) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let mut run = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '§' || ch == '£' || ch == INLINE_FLAG {
                if !run.is_empty() {
                    chunks.push(Chunk::Text(std::mem::take(&mut run), *color));
                }
                if ch == '§' {
                    if let Some(code) = chars.next() {
                        *color = if code == '!' {
                            font.color()
                        } else if let Some(rgb) = self.assets.text_colors.get(&code) {
                            [
                                rgb[0] as f32 / 255.0,
                                rgb[1] as f32 / 255.0,
                                rgb[2] as f32 / 255.0,
                                1.0,
                            ]
                        } else {
                            *color
                        };
                    }
                } else if ch == INLINE_FLAG {
                    let mut key = String::new();
                    while let Some(c) = chars
                        .peek()
                        .copied()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    {
                        key.push(c);
                        chars.next();
                    }
                    chunks.push(Chunk::Flag(key));
                } else {
                    let mut name = String::from("GFX_");
                    while let Some(c) = chars
                        .peek()
                        .copied()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    {
                        name.push(c);
                        chars.next();
                    }
                    if chars.peek() == Some(&'£') {
                        chars.next();
                    }
                    chunks.push(Chunk::Icon(name));
                }
            } else {
                run.push(ch);
            }
        }
        if !run.is_empty() {
            chunks.push(Chunk::Text(run, *color));
        }
        chunks
    }
    fn length(&self, font: &text::Font, chunk: &Chunk) -> f32 {
        match chunk {
            Chunk::Icon(name) => self.assets.sprites.get(name).map_or(0.0, |s| s.size[0]),
            Chunk::Flag(_) => INLINE_FLAG_ADVANCE,
            Chunk::Text(run, _) => text::measure(font, run),
        }
    }
    /// The text a `fixedsize` box shows: [`fit`] with this font's metrics.
    fn fit(&self, font: &text::Font, value: &str, width: f32, height: f32) -> String {
        use crate::map::names::GlyphMetrics;
        if value.contains(['\n', '§', '£']) {
            return value.into();
        }
        let mut color = font.color();
        let glyphs = Glyphs {
            // The font loader (142287830) stores a zero height for the space,
            // which is what ends a word.
            glyph: |ch| {
                GlyphMetrics::glyph(font, ch).map(|g| (g.xadvance, ch == ' ' || g.height == 0.0))
            },
            kerning: |a, b| GlyphMetrics::kerning(font, a, b),
            line_height: font.line_height(),
        };
        fit(self.chunks(font, value, &mut color), width, height, &glyphs)
            .unwrap_or_else(|| value.into())
    }
    fn progress(
        &mut self,
        sprite: &Sprite,
        at: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        fill: Option<f32>,
    ) {
        // An unavailable value leaves the authored frame empty.
        let Some(fill) = fill else { return };
        let fill = fill.clamp(0.0, 1.0);
        // CProgressbarSprite builds its vertex buffer (14234A5E0,
        // progressbarsprite.cpp:163) from (0,0)..(w,h) plus (0,-1) in y-up
        // space on this renderer ((-0.5,-0.5) on the other branch), like the
        // masked flag sprite, whereas ordinary sprites (14234C9D0) add nothing.
        // Every progress bar is therefore drawn one pixel below its layout
        // position, turned with the quad. pairJ_naval_5.png: the naval progress
        // bar, both escape bars and the ship strength/organisation bars.
        let (sin, cos) = rotation.sin_cos();
        let at = [at[0] - sin, at[1] + cos];
        // progress.shader PixelTexture selects texture one at x <= CurrentState,
        // texture two otherwise. Native threshold-edge sampling still needs a
        // same-hour format-22 crop; the blend is not a crossfade.
        let bar = Bar {
            at,
            size,
            rotation,
            vertical: sprite.vertical,
        };
        if let Some(second) = &sprite.second {
            self.progress_segment(second, bar, [fill, 1.0], [fill, 1.0], [0.0, 1.0]);
        }
        if sprite.start_end {
            // progress_startend.shader: TextureOne is three stacked bands. The
            // filled part draws the middle band, the start band over it, then
            // the stop band shifted by 1 - state so its right end meets the
            // fill edge. pairJ_naval_5.png's escaping convoy (state 0.36147)
            // fits this band order at MAE 12.77 against 19.81 for the order a
            // vertically cancelled coordinate would select.
            for (band, u) in [
                ([1.0 / 3.0, 2.0 / 3.0], [0.0, fill]),
                ([0.0, 1.0 / 3.0], [0.0, fill]),
                ([2.0 / 3.0, 1.0], [1.0 - fill, 1.0]),
            ] {
                self.progress_segment(&sprite.sheet, bar, [0.0, fill], u, band);
            }
        } else {
            self.progress_segment(&sprite.sheet, bar, [0.0, fill], [0.0, fill], [0.0, 1.0]);
        }
    }
    /// Draws the `range` fraction of `bar` along its fill axis, sampling the
    /// frame's `u` fraction along that axis and its `band` across it.
    fn progress_segment(
        &mut self,
        sheet: &crate::map::sprites::SpriteSheet,
        bar: Bar,
        range: [f32; 2],
        u: [f32; 2],
        band: [f32; 2],
    ) {
        let Bar {
            at,
            size,
            rotation,
            vertical,
        } = bar;
        let fill = range[1] - range[0];
        if fill <= 0.0 {
            return;
        }
        let mut target = size;
        target[usize::from(vertical)] *= fill;
        let frame = sheet.frame_uv(0);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let uv = if vertical {
            [
                lerp(frame[0], frame[2], band[0]),
                lerp(frame[1], frame[3], u[0]),
                lerp(frame[0], frame[2], band[1]),
                lerp(frame[1], frame[3], u[1]),
            ]
        } else {
            [
                lerp(frame[0], frame[2], u[0]),
                lerp(frame[1], frame[3], band[0]),
                lerp(frame[0], frame[2], u[1]),
                lerp(frame[1], frame[3], band[1]),
            ]
        };
        // Keep the cropped foreground's centre on the rotated original rectangle.
        let mut delta = [(target[0] - size[0]) * 0.5, (target[1] - size[1]) * 0.5];
        delta[usize::from(vertical)] += size[usize::from(vertical)] * range[0];
        let (sin, cos) = rotation.sin_cos();
        let center = [
            at[0] + size[0] * 0.5 + delta[0] * cos - delta[1] * sin,
            at[1] + size[1] * 0.5 + delta[0] * sin + delta[1] * cos,
        ];
        let first = self.drawn.batch.quads.len();
        self.drawn.batch.cell(
            sheet.texture,
            [center[0] - target[0] * 0.5, center[1] - target[1] * 0.5],
            target,
            uv,
            [1.0; 4],
        );
        self.drawn.batch.quads[first].rotation = rotation;
    }
}

#[derive(Clone, Copy)]
struct Bar {
    at: [f32; 2],
    size: [f32; 2],
    rotation: f32,
    vertical: bool,
}

/// The font operands the native text layout reads.
struct Glyphs<G, K> {
    /// Advance and whether the glyph ends a word; `None` without a glyph.
    glyph: G,
    kerning: K,
    line_height: f32,
}

/// One element of laid-out text: a character or an inline country flag.
#[derive(Clone, Debug, PartialEq)]
enum Unit {
    Char(char),
    Flag(String),
}

/// What a `fixedsize` box shows, as text for [`Painter::text`]; `None` keeps
/// the value unchanged (it fits, or carries a sprite this port does not lay out).
///
/// The box's CInstantTextObject (1422D7D10) lays the text out once through the
/// font (142286770 -> 142285B10) with its `maxWidth`, `maxHeight` and
/// truncation, and draws that result through a second pass (14228C8D0 ->
/// 142285B10) that wraps at `maxWidth` with no height limit and no truncation.
/// pairJ_land_5.png's Arial12 reserve names (maxWidth 74, maxHeight 16) show
/// "884 Infantry ...", "144th Infantr...", "5th Royal Ma...", "16th Royal M...",
/// "22 Dywizja ..." and, because that ellipsis overflows, "Iraqi" over
/// "al-Musha...".
fn fit<G, K>(chunks: Vec<Chunk>, width: f32, height: f32, glyphs: &Glyphs<G, K>) -> Option<String>
where
    G: Fn(char) -> Option<(f32, bool)>,
    K: Fn(char, char) -> f32,
{
    let mut units = Vec::new();
    for chunk in chunks {
        match chunk {
            Chunk::Text(run, _) => units.extend(run.chars().map(Unit::Char)),
            Chunk::Flag(key) => units.push(Unit::Flag(key)),
            Chunk::Icon(_) => return None,
        }
    }
    let laid = lay_out(&units, width, height, true, false, glyphs);
    let shown = lay_out(&laid, width, f32::INFINITY, false, true, glyphs);
    (shown != units).then(|| {
        let mut text = String::new();
        for unit in shown {
            match unit {
                Unit::Char(ch) => text.push(ch),
                Unit::Flag(key) => {
                    text.push(INLINE_FLAG);
                    text.push_str(&key);
                }
            }
        }
        text
    })
}

/// 142285B10. A glyph that does not end a word, or that starts one, and whose
/// advance plus kerning would reach `width` starts a new line (142283E90) while
/// two line heights plus the finished lines stay below `height`; otherwise the
/// layout stops before it. A word as long as its line breaks at that glyph.
/// With `truncate`, a stopped layout ends in an ellipsis (14228DE40): up to
/// three trailing characters become dots until the line's remaining width
/// holds three dot advances plus three pixels, then the missing dots are
/// appended, so the result can still overflow. `newlines` writes each line
/// break, dropping the trailing space the native line width excludes.
fn lay_out<G, K>(
    units: &[Unit],
    width: f32,
    height: f32,
    truncate: bool,
    newlines: bool,
    glyphs: &Glyphs<G, K>,
) -> Vec<Unit>
where
    G: Fn(char) -> Option<(f32, bool)>,
    K: Fn(char, char) -> f32,
{
    let room_for_line = |lines: f32| 2.0 * glyphs.line_height + lines < height;
    let mut out = Vec::new();
    let mut word = Vec::new();
    let (mut line, mut word_width, mut lines) = (0.0_f32, 0.0_f32, 0.0_f32);
    let mut previous = None;
    let mut stopped = false;
    for unit in units {
        let (step, ends_word) = match unit {
            Unit::Char('\n') => {
                if !room_for_line(lines) {
                    stopped = true;
                    break;
                }
                word.push(unit.clone());
                out.append(&mut word);
                (line, word_width, previous) = (0.0, 0.0, None);
                lines += glyphs.line_height;
                continue;
            }
            Unit::Char(ch) => match (glyphs.glyph)(*ch) {
                Some((advance, ends)) => {
                    let kerning = previous.map_or(0.0, |p| (glyphs.kerning)(p, *ch));
                    (advance + kerning, ends)
                }
                None => {
                    word.push(unit.clone());
                    previous = None;
                    continue;
                }
            },
            // An inline flag is a formatting code: no kerning and no word end.
            Unit::Flag(_) => (INLINE_FLAG_ADVANCE, false),
        };
        if (!ends_word || word_width == 0.0) && width <= line + step {
            if !room_for_line(lines) {
                stopped = true;
                break;
            }
            if newlines && line != word_width && out.last() == Some(&Unit::Char(' ')) {
                out.pop();
            }
            if line == word_width {
                out.append(&mut word);
                word_width = 0.0;
            }
            if newlines {
                out.push(Unit::Char('\n'));
            }
            out.append(&mut word);
            line = word_width;
            lines += glyphs.line_height;
        }
        word_width += step;
        line += step;
        word.push(unit.clone());
        if ends_word {
            out.append(&mut word);
            word_width = 0.0;
        }
        previous = match unit {
            Unit::Char(ch) => Some(*ch),
            Unit::Flag(_) => None,
        };
    }
    out.append(&mut word);
    if stopped && truncate && out.len() > 2 {
        let dot = (glyphs.glyph)('.').map_or(0.0, |(advance, _)| advance);
        let mut room = width - line;
        let mut need = (dot + 1.0) * 3.0 - (glyphs.kerning)('.', '.');
        let mut replaced = 0;
        while replaced < 3 && room < need {
            let index = out.len() - 1 - replaced;
            // UNVERIFIED for a flag: the native codes stored behind it.
            let advance = match &out[index] {
                Unit::Char(ch) => (glyphs.glyph)(*ch).map_or(0.0, |(advance, _)| advance),
                Unit::Flag(_) => 0.0,
            };
            room += advance - dot;
            need -= dot;
            out[index] = Unit::Char('.');
            replaced += 1;
        }
        out.extend(std::iter::repeat_n(Unit::Char('.'), 3 - replaced));
    }
    out
}

/// Where a `.gui` quarter-turned progress bar lands, as the unrotated quad and
/// the clockwise turn [`Batch::rotated`] applies about its centre.
///
/// `landcombat.gui` turns the 16x7 `GFX_combat_unit_bar_org/str` bars by
/// `rotation = 1.5708`. pairJ_land_5.png (every committed and reserve entry)
/// shows the game turning them **counter-clockwise**: texture one, which the
/// progress shader keeps at `x <= state`, fills from the bottom. The turned
/// 7x16 rectangle ends at the declared x and spans the declared y from -10 to
/// +6, `[x - h, y + h - w - 1, h, w]` as drawn (committed bars at (1031,681)
/// measure x 1024..1031, y 671..687; reserve bars at (915,852) x 908..915,
/// y 842..858). The returned quad excludes the progress quad's own offset.
/// The native pivot is UNVERIFIED; no other quarter-turned progress bar is
/// drawn by these windows. Half turns (the naval map icon) keep the centre.
fn progress_turn(at: [f32; 2], size: [f32; 2], rotation: f32) -> ([f32; 2], f32) {
    if (rotation - std::f32::consts::FRAC_PI_2).abs() > 1e-3 {
        return (at, rotation);
    }
    let [w, h] = size;
    // Before the progress quad's own one-pixel offset, which this turn carries
    // one pixel to the right.
    let turned = [at[0] - h - 1.0, at[1] + h - w - 1.0];
    let centre = [turned[0] + h * 0.5, turned[1] + w * 0.5];
    ([centre[0] - w * 0.5, centre[1] - h * 0.5], -rotation)
}

fn clip_quads(batch: &mut Batch, first: usize, clip: [f32; 4]) {
    for quad in &mut batch.quads[first..] {
        let [x, y, w, h] = quad.rect;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let left = x.max(clip[0]);
        let top = y.max(clip[1]);
        let right = (x + w).min(clip[0] + clip[2]);
        let bottom = (y + h).min(clip[1] + clip[3]);
        let [u0, v0, u1, v1] = quad.uv;
        quad.uv = [
            u0 + (u1 - u0) * (left - x) / w,
            v0 + (v1 - v0) * (top - y) / h,
            u0 + (u1 - u0) * (right - x) / w,
            v0 + (v1 - v0) * (bottom - y) / h,
        ];
        quad.rect = [left, top, (right - left).max(0.0), (bottom - top).max(0.0)];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turned_bar_ends_at_its_declared_x_and_fills_upwards() {
        // pairJ_land_5.png reserve bar declared at (915, 852), 16x7.
        let authored: f32 = "1.5708".parse().unwrap();
        let (at, rotation) = progress_turn([915.0, 852.0], [16.0, 7.0], authored);
        assert_eq!(rotation, -authored);
        // The unrotated quad's centre is the turned rectangle's centre, one
        // pixel left of the drawn 908..915 before the progress quad's offset.
        assert_eq!([at[0] + 8.0, at[1] + 3.5], [910.5, 850.0]);
        // A half turn keeps its position and angle.
        assert_eq!(
            progress_turn([1.0, 2.0], [16.0, 7.0], std::f32::consts::PI),
            ([1.0, 2.0], std::f32::consts::PI)
        );
    }

    #[test]
    fn a_fixed_size_box_truncates_then_wraps_like_the_game() {
        // Arial12 advances and kerning pairs of the pairJ_land_5.png reserve
        // names (maxWidth 74, maxHeight 16, lineHeight 14).
        let glyphs = Glyphs {
            glyph: |ch: char| {
                let advance = match ch {
                    'I' | 'i' | 'j' | 'l' => 2.0,
                    ' ' | '.' | 't' => 3.0,
                    '-' | 'f' | 'r' => 4.0,
                    'k' => 5.0,
                    'D' | 'R' | 'S' => 7.0,
                    'G' | 'M' => 8.0,
                    'w' => 10.0,
                    _ => 6.0,
                };
                Some((advance, ch == ' '))
            },
            kerning: |a: char, b: char| match (a, b) {
                ('y' | 'r', '.') | ('1', '1') => -1.0,
                _ => 0.0,
            },
            line_height: 14.0,
        };
        let shown = |run: &str| {
            let chunks = vec![Chunk::Text(run.into(), [1.0; 4])];
            fit(chunks, 74.0, 16.0, &glyphs)
        };
        for (name, game) in [
            ("884 Infantry Division", "884 Infantry ..."),
            ("144th Infantry Division", "144th Infantr..."),
            ("5th Royal Marines Division", "5th Royal Ma..."),
            ("16th Royal Marines Division", "16th Royal M..."),
            ("22 Dywizja Piechoty Górskiej", "22 Dywizja ..."),
            // The ellipsis overflows the box, so the drawing pass wraps it.
            ("Iraqi al-Mushati al-16r", "Iraqi\nal-Musha..."),
        ] {
            assert_eq!(shown(name).as_deref(), Some(game), "{name}");
        }
        assert_eq!(shown("Short"), None);
        // With room for a second line, the layout wraps instead of cutting.
        let chunks = vec![Chunk::Text("Iraqi al-Mushati al-16r".into(), [1.0; 4])];
        assert_eq!(
            fit(chunks, 74.0, 29.0, &glyphs).as_deref(),
            Some("Iraqi al-Mushati\nal-16r")
        );
        // A flag that would reach the edge stops the layout before it.
        let chunks = vec![
            Chunk::Text("Name that is long enough ".into(), [1.0; 4]),
            Chunk::Flag("SOV".into()),
        ];
        assert_eq!(
            fit(chunks, 130.0, 16.0, &glyphs).as_deref(),
            Some("Name that is long enough...")
        );
    }
}
