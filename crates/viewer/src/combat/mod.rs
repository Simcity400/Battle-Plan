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
            let start = self.scroll[side]
                .saturating_mul(columns)
                .min(rows.len().saturating_sub(count));
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
            if rows.len() > count {
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
                            20.0 + (slider_rect[3] - 50.0).max(0.0) * start as f32
                                / (rows.len() - count) as f32,
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
                // 14162F510: the DX11 masked-shield path shifts both the flag
                // and overlay by (0,-1), before the GUI/display scale.
                let origin = if sprite == Some("GFX_flag_small2") {
                    [origin[0], origin[1] - 1.0]
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
                self.text(font, value, origin, align, width);
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
        if !value.contains(['\n', '£', '§']) {
            self.drawn.batch.text(
                &text::layout(font, value, origin, align, width),
                font.color(),
            );
            return;
        }
        let mut color = font.color();
        for (line, raw) in value.lines().enumerate() {
            let mut chunks: Vec<(String, [f32; 4], bool)> = Vec::new();
            let mut run = String::new();
            let mut chars = raw.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '§' || ch == '£' {
                    if !run.is_empty() {
                        chunks.push((std::mem::take(&mut run), color, false));
                    }
                    if ch == '§' {
                        if let Some(code) = chars.next() {
                            color = if code == '!' {
                                font.color()
                            } else if let Some(rgb) = self.assets.text_colors.get(&code) {
                                [
                                    rgb[0] as f32 / 255.0,
                                    rgb[1] as f32 / 255.0,
                                    rgb[2] as f32 / 255.0,
                                    1.0,
                                ]
                            } else {
                                color
                            };
                        }
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
                        chunks.push((name, [1.0; 4], true));
                    }
                } else {
                    run.push(ch);
                }
            }
            if !run.is_empty() {
                chunks.push((run, color, false));
            }
            let length = |chunk: &(String, [f32; 4], bool)| {
                if chunk.2 {
                    self.assets.sprites.get(&chunk.0).map_or(0.0, |s| s.size[0])
                } else {
                    text::measure(font, &chunk.0)
                }
            };
            let measured: f32 = chunks.iter().map(length).sum();
            let slack = (width.unwrap_or(0.0) - measured).max(0.0);
            let mut x = origin[0]
                + match align {
                    text::Align::Left => 0.0,
                    text::Align::Centre => slack * 0.5,
                    text::Align::Right => slack,
                };
            let y = origin[1] + line as f32 * font.line_height();
            for chunk in chunks {
                let advance = length(&chunk);
                let remaining = width.map(|w| (origin[0] + w - x).max(0.0));
                self.drawn
                    .batch
                    .at_layer(100_000_000 + self.drawn.batch.quads.len() as u32);
                if chunk.2 {
                    if remaining.is_none_or(|w| w >= advance)
                        && let Some(sprite) = self.assets.sprites.get(&chunk.0)
                    {
                        self.drawn
                            .batch
                            .scaled(&sprite.sheet, 0, [x, y], sprite.size, chunk.1);
                    }
                } else {
                    self.drawn.batch.text(
                        &text::layout(font, &chunk.0, [x, y], text::Align::Left, remaining),
                        chunk.1,
                    );
                }
                x += advance;
            }
        }
    }
    fn progress(
        &mut self,
        sprite: &Sprite,
        at: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        fill: Option<f32>,
    ) {
        if fill.is_none() {
            // An unavailable value leaves the authored frame empty.
            return;
        }
        let Some(fill) = fill else { return };
        let fill = fill.clamp(0.0, 1.0);
        // progress.shader PixelTexture selects texture one at x <= CurrentState,
        // texture two otherwise. Native threshold-edge sampling still needs a
        // same-hour format-22 crop; the blend is not a crossfade.
        if let Some(second) = &sprite.second {
            self.progress_segment(second, at, size, rotation, sprite.vertical, [fill, 1.0]);
        }
        self.progress_segment(
            &sprite.sheet,
            at,
            size,
            rotation,
            sprite.vertical,
            [0.0, fill],
        );
    }
    fn progress_segment(
        &mut self,
        sheet: &crate::map::sprites::SpriteSheet,
        at: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        vertical: bool,
        range: [f32; 2],
    ) {
        let fill = range[1] - range[0];
        if fill <= 0.0 {
            return;
        }
        let mut target = size;
        if vertical {
            target[1] *= fill;
        } else {
            target[0] *= fill;
        }
        let mut uv = sheet.frame_uv(0);
        if vertical {
            uv[3] = uv[1] + (uv[3] - uv[1]) * range[1];
            uv[1] += range[0];
        } else {
            uv[2] = uv[0] + (uv[2] - uv[0]) * range[1];
            uv[0] += range[0];
        }
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
