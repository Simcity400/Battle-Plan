//! Text sprites (`textSpriteType`): text baked into a render target, then drawn as a sprite
//! frame. This module lays out the text, font, glyphs, background, frame and technique on the
//! CPU; [`PartKind::TextSprite`] carries the bake and final draw to the compiled pipeline.
//!
//! A button with a font and a registered text sprite, a check box with a font, and an edit box
//! without `instantTextBoxType` prepare this bake. A font-backed fallback drawable prepares
//! empty text. Icons have no font, so their text sprites remain unbaked and draw nothing. The
//! drawable starts hidden: preparation stores arguments, then showing the window bakes the
//! target. Each show and text write bakes it again; ordinary frames retain it.

use super::*;

/// A text sprite's bake into its render target, in the target's pixels with `y` down.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBake {
    /// The target size, in `B8G8R8A8_UNORM`, with one mip, cleared to transparent black.
    pub size: [i32; 2],
    /// Pass 1, only when the texture loads: its nine-slice (the texture's whole strip stretched
    /// over every frame, the border kept) drawn with blending off, `text.shader`'s `Text`
    /// program and its sampler row 0, white, `ModColor` the font's auxiliary colour (white; a
    /// written one is refused by `Walk::text_sprite`): each quad and its `[u0, v0, u1, v1]`.
    pub background: Option<Background>,
    /// Pass 2 draws glyphs per strip frame in the font colour and text colour codes, with the
    /// content Text technique and blend. ModColor matches the background; projection uses the
    /// target extent.
    pub glyphs: Vec<DrawnGlyph>,
    pub border_color: u32,
    /// Pass 2's inline icons and flags, white, with the glyphs' program and blend.
    pub inline: Vec<(InlineSource, Rect, [f32; 4])>,
    /// Cumulative glyph/icon counts per frame. Inline icons draw when reached. Glyph vertices
    /// cover the whole prepared text and submit once per frame, without the ordinary text
    /// path's 512-glyph flush.
    pub frame_ends: Vec<[usize; 2]>,
    /// The content-resolved `Text` technique both passes run.
    pub shading: Shading,
}

/// A bake's background: the texture it samples and its nine-slice's quads, each with its
/// `[u0, v0, u1, v1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    pub source: InlineSource,
    pub quads: Vec<(Rect, [f32; 4])>,
}

/// The arguments retained for a text sprite's bake.
pub(super) struct TextSpriteDraw<'t> {
    pub text: &'t str,
    pub font: &'t [u8],
    /// `texName`: a texture file replacing the type's `textureFile`.
    pub texture: BakeTexture,
    /// Target width and height.
    pub size: [i32; 2],
    /// The offset (border).
    pub border: [i32; 2],
    /// Alignment: 0 left, 1 right, 2 centre.
    pub format: i32,
    /// Whether the bake uses a fixed box.
    pub fixed: bool,
    /// Whether the bake reads special sequences.
    pub special: bool,
    /// Whether text can span multiple lines.
    pub multiline: bool,
}

/// The texture a bake draws under its text.
#[derive(Clone, Copy, Debug)]
pub(super) enum BakeTexture {
    /// The type's own `textureFile` (an empty `texName`).
    Type,
    /// A file the code names, as loaded ([`Resources::texture_file`]); `None` when it does not
    /// load (the null texture, of no size).
    File(Option<Texture>),
}

/// The numeric alignment used for a text sprite bake.
pub(super) fn format_value(format: TextFormat) -> i32 {
    match format {
        TextFormat::Left => 0,
        TextFormat::Right => 1,
        TextFormat::Center => 2,
    }
}

impl Walk<'_> {
    /// Measures the text for a button's `centerPosition`: the widest line plus twice the
    /// border, and the lines' total height plus twice the border. There is no width limit, and
    /// special sequences are read.
    pub(super) fn text_measure(
        &self,
        element: &Element,
        font: &[u8],
        text: &str,
        border: [i32; 2],
    ) -> Result<[i32; 2], Refusal> {
        let font = text_font(self.resources, element, font)?;
        let resources = self.resources;
        let refused = &self.refused;
        let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
        let chars: Vec<char> = text.chars().collect();
        let laid = self
            .text_layout(
                element,
                font,
                &sprite,
                &chars,
                Limits {
                    width: 0,
                    height: i32::MAX,
                    border,
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: false,
                    multiline: true,
                    special: true,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map_err(|why| Refusal::new(&element.path, RefusalKind::DivideByZero, why))?;
        let widest = laid
            .widths
            .iter()
            .fold(0, |widest, width| widest.max(*width));
        let tall = laid
            .heights
            .iter()
            .fold(0i32, |sum, height| sum.wrapping_add(*height));
        Ok([
            widest.wrapping_add(border[0].wrapping_mul(2)),
            tall.wrapping_add(border[1].wrapping_mul(2)),
        ])
    }

    /// A text sprite drawn for `element` under sprite `name` (`info` its type), using the
    /// retained bake arguments in `draw`. `None` means it was never baked and draws nothing.
    /// Place it at `at` less `centre`, in `frame` and `state`. Zero frames divide by zero
    /// while building the quad, even without a bake. Returns the part index when drawn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text_sprite(
        &mut self,
        element: &Element,
        name: &[u8],
        info: &SpriteInfo,
        draw: Option<TextSpriteDraw>,
        frame: i32,
        state: State,
        at: [i32; 2],
        centre: [i32; 2],
        rotation: f32,
        clip: Option<Clip>,
    ) -> Result<Option<usize>, Refusal> {
        let path = &element.path;
        let frames = info.frames;
        if frames == 0 {
            return Err(Refusal::new(
                path,
                RefusalKind::DivideByZero,
                format!(
                    "shows {}, a textSpriteType of 0 frames, whose quad is built by dividing \
                     its width by them: the game ends",
                    lossy(name)
                ),
            ));
        }
        let Some(draw) = draw else {
            return Ok(None);
        };
        let effect = info
            .effect
            .clone()
            .map_err(|why| Refusal::new(path, RefusalKind::CustomEffect, why))?;
        let font = text_font(self.resources, element, draw.font)?;
        // Both bake passes upload the font's auxiliary colour as `ModColor`. It starts white;
        // ordinary text drawing restores it to white after each draw. A component-written
        // colour ([`installed_content_core::gui::native::fonts::FontFacts::aux_color`]) therefore holds
        // until the first ordinary text draw with that font.
        if font.facts.aux_color != [1.0; 4] {
            return Err(Refusal::new(
                path,
                RefusalKind::MissingInput,
                format!(
                    "bakes with font {}, whose auxiliary colour a component wrote: the bake is \
                     multiplied by it until a text object first draws with the font and resets \
                     it to white; whether one did since the game started is draw history the \
                     replay does not hold",
                    lossy(draw.font)
                ),
            ));
        }
        let text_effect = self
            .text_effect()
            .map_err(|why| Refusal::new(path, RefusalKind::CustomEffect, why))?;
        let resources = self.resources;
        let refused = &self.refused;
        let sprite = |name: &[u8]| icon_sprite(resources, refused, &element.path, name);
        let divide = |why: String| Refusal::new(path, RefusalKind::DivideByZero, why);
        // The texture: the code's `texName`, else the type's `textureFile`; with no name the
        // stored box is its size, and one that does not load is the null texture.
        let textured = matches!(info.kind, SpriteKind::Text { textured: true });
        let (texture, dims) = match draw.texture {
            BakeTexture::File(Some(texture)) => {
                (Some(InlineSource::Texture(texture.texture)), texture.size)
            }
            BakeTexture::File(None) => (None, [0, 0]),
            BakeTexture::Type if !textured => (None, draw.size),
            BakeTexture::Type if info.missing.is_none() => {
                (Some(InlineSource::Sprite(name.to_vec())), info.texture)
            }
            BakeTexture::Type => (None, [0, 0]),
        };
        // A named texture may be loaded or null.
        let named = !matches!(draw.texture, BakeTexture::Type) || textured;
        let [mut width, mut height] = draw.size;
        let [bx, by] = draw.border;
        let strip = if frames > 1 {
            dims[0].wrapping_div(frames)
        } else {
            dims[0]
        };
        let chars: Vec<char> = draw.text.chars().collect();
        let prepare = |width: i32, height: i32| {
            self.text_layout(
                element,
                font,
                &sprite,
                &chars,
                Limits {
                    width,
                    height,
                    border: draw.border,
                    newlines: false,
                    preserve_indent: false,
                    ellipsis: true,
                    multiline: draw.multiline,
                    special: true,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map(|laid| laid.text)
            .map_err(divide)
        };
        let text = if !draw.fixed {
            if draw.multiline {
                chars.clone()
            } else {
                prepare(width, height)?
            }
        } else if width != 0 && height != 0 {
            prepare(width, height)?
        } else {
            if named {
                if height == 0 {
                    height = dims[1].wrapping_sub(by);
                }
                if width == 0 {
                    width = strip.wrapping_sub(bx);
                }
            }
            if height != 0 || !draw.multiline {
                prepare(width, height)?
            } else {
                chars.clone()
            }
        };
        // `border_draw`: nothing to bake destroys the target.
        if text.is_empty() && texture.is_none() {
            return Ok(None);
        }
        let laid = self
            .text_layout(
                element,
                font,
                &sprite,
                &text,
                Limits {
                    width,
                    height: i32::MAX,
                    border: draw.border,
                    newlines: true,
                    preserve_indent: false,
                    ellipsis: false,
                    multiline: true,
                    special: draw.special,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map_err(divide)?;
        let widest = laid
            .widths
            .iter()
            .fold(0, |widest, width| widest.max(*width));
        let tall = laid
            .heights
            .iter()
            .fold(0i32, |sum, height| sum.wrapping_add(*height));
        let mut fixed = draw.fixed;
        let full = if width != 0 && height != 0 {
            fixed = true;
            height
        } else if fixed {
            if height != 0 {
                height
            } else {
                tall.wrapping_add(by.wrapping_mul(2))
            }
        } else {
            width = widest.wrapping_add(bx.wrapping_mul(2));
            tall.wrapping_add(by.wrapping_mul(2))
        };
        let stride = if fixed { width.wrapping_sub(bx) } else { width };
        let baked_height = if fixed && height != 0 {
            height.wrapping_sub(by)
        } else {
            full
        };
        let size = [stride.wrapping_mul(frames), baked_height];
        // A target of no area is not made, so drawing emits nothing.
        if size[0] <= 0 || size[1] <= 0 {
            return Ok(None);
        }
        let [w, h] = size.map(|value| value as f32);
        // Pass 1: the nine-slice, `x` at 0, bx, w - bx, w and `y` down at 0, by, h - by, h, the
        // texture's own border (`old`, the whole strip) in its coordinates.
        let has_texture = texture.is_some();
        let background = texture.map(|source| {
            let [ow, oh] = dims.map(|value| value as f32);
            let xs = [0.0, bx as f32, w - bx as f32, w];
            let ys = [0.0, by as f32, h - by as f32, h];
            let us = [0.0, bx as f32 / ow, 1.0 - bx as f32 / ow, 1.0];
            let vs = [0.0, by as f32 / oh, 1.0 - by as f32 / oh, 1.0];
            let mut quads = Vec::with_capacity(9);
            for row in 0..3 {
                for column in 0..3 {
                    quads.push((
                        [
                            xs[column],
                            ys[row],
                            xs[column + 1] - xs[column],
                            ys[row + 1] - ys[row],
                        ],
                        [us[column], vs[row], us[column + 1], vs[row + 1]],
                    ));
                }
            }
            Background { source, quads }
        });
        // Pass 2: each frame's text, in the font's own colour.
        let packed = font.facts.color;
        let own = [
            (packed >> 16 & 0xFF) as f32 / 255.0,
            (packed >> 8 & 0xFF) as f32 / 255.0,
            (packed & 0xFF) as f32 / 255.0,
            (packed >> 24) as f32 / 255.0,
        ];
        let colors = native_text::Colors {
            own,
            font: &font.facts.text_colors,
            global: resources.text_colors(),
        };
        let top = if fixed && has_texture {
            full.wrapping_sub(tall) / 2
        } else {
            by
        };
        let format = draw.format;
        let mut glyphs = Vec::new();
        let mut inline = Vec::new();
        let mut frame_ends = Vec::new();
        for strip_index in 0..frames.max(0) {
            let shift = strip_index.wrapping_mul(stride);
            let start = move |line: usize, own: i32| -> i32 {
                let x = if !fixed {
                    bx
                } else if line == 0 {
                    match format {
                        2 => width / 2 - own / 2,
                        0 => bx,
                        _ => (width - bx - 1) - own,
                    }
                } else {
                    let anchor = match format {
                        2 => width / 2,
                        0 => bx,
                        _ => width - bx - 1,
                    };
                    match format {
                        2 => anchor - own / 2,
                        1 => anchor - own,
                        _ => anchor,
                    }
                };
                x.wrapping_add(shift)
            };
            let pieces = native_text::draw(
                font,
                &sprite,
                &laid,
                draw.special,
                &native_text::Placement { top, start: &start },
                &colors,
            )
            .map_err(|why| Refusal::new(path, RefusalKind::UndefinedRead, why))?;
            for piece in pieces {
                match piece {
                    Piece::Glyph { ch, at, color } => {
                        glyphs.push(drawn_glyph(
                            &mut self.diagnostics,
                            path,
                            font,
                            ch,
                            at,
                            color,
                        ));
                    }
                    Piece::Icon {
                        sprite,
                        at,
                        size,
                        uv,
                    } => inline.push((
                        InlineSource::Sprite(sprite),
                        [at[0] as f32, at[1] as f32, size[0] as f32, size[1] as f32],
                        uv,
                    )),
                    Piece::Flag { tag, at, resolved } => {
                        let cell = if resolved {
                            resources.resolved_text_flag(tag.as_bytes())
                        } else {
                            resources.text_flag(&tag)
                        };
                        if let Some(cell) =
                            cell.map_err(|why| Refusal::new(path, RefusalKind::MissingInput, why))?
                        {
                            inline.push((
                                InlineSource::Texture(cell.texture),
                                [at[0] as f32, at[1] as f32, 10.0, 7.0],
                                cell.uv,
                            ));
                        } else {
                            self.diagnostics.push(Diagnostic {
                                path: path.clone(),
                                kind: DiagnosticKind::NullTextFlag,
                                detail: format!(
                                    "inline flag {tag} has no valid resident atlas record"
                                ),
                            });
                        }
                    }
                    Piece::Undefined(icon) => self.diagnostics.push(Diagnostic {
                        path: path.clone(),
                        kind: DiagnosticKind::UndefinedTextIcon,
                        detail: format!(
                            "bakes the inline icon {icon:?}, which the content does not define \
                             as GFX_ and its name nor bare"
                        ),
                    }),
                }
            }
            frame_ends.push([glyphs.len(), inline.len()]);
        }
        // The frame width is the target width divided by its frame count. Its bottom-left lies
        // one target height below the supplied position, and rotation uses that position as
        // its pivot. UVs cover one frame. Frame zero selects the first, positive frames are
        // one-based, and negative frames select from the state using unsigned remainder.
        let frame_width = size[0].wrapping_div(frames);
        if frame_width <= 0 {
            return Ok(None);
        }
        let index = match frame {
            0 => 0,
            frame if frame > 0 => frame - 1,
            _ => ((state.number() - 1) % frames as u32) as i32,
        };
        let du = 1.0 / frames.max(1) as f32;
        let [x, y] = [at[0] - centre[0], at[1] - centre[1]].map(|value| value as f32);
        let technique = match state {
            State::Down(_) => "Down",
            State::Over(_) => "Over",
            State::Disable => "Disable",
            State::Up => "Up",
        };
        let mut color = [1.0; 4];
        color[3] = self.opacity;
        if self.bindings.regions {
            self.regions.push(Region {
                path: path.clone(),
                owner: self.region_owner.clone(),
                scroll: None,
                rect: [x, y, frame_width as f32, h],
                clip,
                hit: None,
                disabled: matches!(state, State::Disable),
            });
        }
        self.parts.push(Part {
            path: path.clone(),
            child: self.top,
            kind: PartKind::TextSprite {
                sprite: name.to_vec(),
                bake: Box::new(TextBake {
                    size,
                    background,
                    glyphs,
                    border_color: font.facts.border_color,
                    inline,
                    frame_ends,
                    shading: shading(&text_effect, "Text", 0.0),
                }),
                geometry: Geometry {
                    local: [0., 0., frame_width as f32, h],
                    rect: [x, y, frame_width as f32, h],
                    rotation,
                    pivot: [x, y + h],
                },
                uv: [0.0, 0.0, du, 1.0],
                offsets: [index as f32 * du, 0.0, 0.0, 0.0],
            },
            clip,
            color,
            // The final sprite receives both clocks.
            shading: {
                let mut shading = shading(&effect, technique, state.since());
                if shading.compiled.is_some() {
                    shading.animation = self.effect_clock(self.bound(element), state);
                }
                shading
            },
            native: None,
        });
        Ok(Some(self.parts.len() - 1))
    }

    /// Text box construction and text updates share this drawable.
    pub(super) fn text_box_sprite(
        &mut self,
        element: &Element,
        text: &native::TextSprite,
        shown: &str,
        size: [i32; 2],
        at: [i32; 2],
        clip: Option<Clip>,
    ) -> Result<Option<[i32; 2]>, Refusal> {
        let path = &element.path;
        let name = element.name_bytes();
        // Its own `create_sprite` finds what an earlier text box of its name registered, the same
        // one-frame untextured type it would register itself: no session history matters here.
        let info = match looked_up(self.resources, &self.refused, path, name, true) {
            Some(info) if matches!(info.kind, SpriteKind::Text { .. }) => info,
            Some(info) => {
                self.sprite_part(element, name, -1, 1.0, 0.0, at, Centre::None, clip)?;
                return Ok(Some(info.scaled_extent(1.0)));
            }
            None => SpriteInfo {
                texture: [0; 2],
                frames: 1,
                vertical: false,
                masked: false,
                animated: false,
                kind: SpriteKind::Text { textured: false },
                anchor: [0.0; 2],
                effect: self
                    .resources
                    .effect(super::super::native_resources::DEFAULT_SPRITE_EFFECT),
                hit: Hit::Frame,
                missing: None,
                refusals: Vec::new(),
            },
        };
        let texture = if text.texture.is_empty() {
            BakeTexture::Type
        } else {
            match self.resources.texture_file(&text.texture) {
                Some(TextureFile::Loaded(texture)) => BakeTexture::File(Some(texture)),
                Some(TextureFile::Failed(why)) => {
                    self.diagnostics.push(Diagnostic {
                        path: path.clone(),
                        kind: DiagnosticKind::NoTexture,
                        detail: format!(
                            "text box background {} did not load: {why}",
                            lossy(&text.texture)
                        ),
                    });
                    BakeTexture::File(None)
                }
                Some(TextureFile::Refused(kind, why)) => {
                    return Err(Refusal::new(
                        path,
                        kind,
                        format!("bakes its text over the texture {why}"),
                    ));
                }
                None => {
                    return Err(Refusal::new(
                        path,
                        RefusalKind::MissingResource,
                        format!(
                            "bakes its text over the texture {:?}, which the caller has not \
                             loaded (Assets::textures)",
                            lossy(&text.texture)
                        ),
                    ));
                }
            }
        };
        let part = self.text_sprite(
            element,
            name,
            &info,
            Some(text_sprites::TextSpriteDraw {
                text: shown,
                font: &text.font,
                texture,
                size,
                border: text.border_size,
                format: text.format,
                fixed: text.fixed_size,
                special: true,
                multiline: true,
            }),
            -1,
            State::Up,
            at,
            [0, 0],
            0.0,
            clip,
        )?;
        Ok(part.map(|index| match &self.parts[index].kind {
            PartKind::TextSprite { geometry, .. } => {
                [geometry.rect[2] as i32, geometry.rect[3] as i32]
            }
            _ => unreachable!("text_sprite emitted its bake"),
        }))
    }
}

/// The tooltip template name looked up in the text box registry.
pub const TOOLTIP: &[u8] = b"ToolTip";

/// Wraps the tooltip text, then updates its text sprite.
pub fn tooltip(
    resources: &dyn Resources,
    value: &str,
    time: f32,
    resolved_flags: bool,
) -> Result<Layout, Refusals> {
    // Fall back to `defaultTextBox` when the tooltip name is absent.
    let element = resources
        .gui_type(TOOLTIP)
        .or_else(|| resources.gui_type(b"defaultTextBox"))
        .ok_or_else(|| {
            vec![Refusal::new(
                "ToolTip",
                RefusalKind::NullRead,
                "ToolTip and defaultTextBox are absent",
            )]
        })?;
    let reader = reader_refusals(element);
    if !reader.is_empty() {
        return Err(reader);
    }
    let Fields::TextSprite(text) = &element.fields else {
        return Err(vec![Refusal::new(
            &element.path,
            RefusalKind::TypeConfusion,
            "tooltip template is not a textBoxType",
        )]);
    };
    // Tooltip text uses format 0.
    let mut text = text.clone();
    text.format = 0;
    let mut bindings = Bindings {
        time,
        ..Bindings::default()
    };
    bindings.bind(element).resolved_flags = resolved_flags;
    let mut walk = Walk::new(resources, &bindings);
    let laid = (|| {
        let font = text_font(resources, element, &text.font)?;
        let sprite = |name: &[u8]| icon_sprite(resources, &walk.refused, &element.path, name);
        let chars: Vec<char> = value.chars().collect();
        // The wrapping width removes two borders here and two more during text layout.
        let wrapped = walk
            .text_layout(
                element,
                font,
                &sprite,
                &chars,
                Limits {
                    width: text
                        .max_width
                        .wrapping_sub(text.border_size[0].wrapping_mul(2)),
                    height: i32::MAX,
                    border: text.border_size,
                    newlines: true,
                    preserve_indent: true,
                    ellipsis: false,
                    multiline: true,
                    special: true,
                    icons_add_height: font.facts.icons_add_height,
                },
            )
            .map_err(|why| Refusal::new(&element.path, RefusalKind::DivideByZero, why))?;
        // Convert the prepared codepoints through a zero-terminated string.
        let shown: String = wrapped
            .text
            .into_iter()
            .take_while(|ch| *ch != '\0')
            .collect();
        // Supply zero height; the type's fixedsize still reaches the bake.
        walk.text_box_sprite(element, &text, &shown, [text.max_width, 0], [0, 0], None)
    })();
    let mut refused = walk.refused.take();
    let size = match laid {
        Ok(size) => size,
        Err(why) => {
            refused.push(why);
            None
        }
    };
    if !refused.is_empty() {
        return Err(refused);
    }
    if size.is_none() {
        walk.diagnostics.push(Diagnostic {
            path: element.path.clone(),
            kind: DiagnosticKind::EmptyTextBake,
            detail: "tooltip has no drawable render target".into(),
        });
    }
    Ok(walk.finish(size.unwrap_or([0, 0])))
}
