//! Counter unit-type icons from `DivisionTemplateIcons`.
//!
//! Plain strings and integers select a resolved sprite name and frame. The GUI's
//! type child supplies placement; the runtime-selected icon replaces its placeholder.

use crate::errors::examples;
use crate::image_rgba::rgba8;
#[cfg(test)]
use crate::map::test_support::test_tga_rgba8;
pub const NO_INTEL_SPRITE: &str = "GFX_unit_no_intel_icon_medium_white";

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    ResolvedLayers,
    gfx::{SpriteSet, SpriteType},
    gui::native::{resources, sprites::SpriteKind},
    image::{Rgba8Image, dds, native::Decode},
};

/// Synthetic counter frame used by the naming and hover fixtures.
#[cfg(any(test, feature = "test-support"))]
pub const FRAME_SIZE: [u32; 2] = [30, 12];

/// The counter sprite's pictorial frame, the silhouette the game draws by default.
pub const PICTORIAL_FRAME: u32 = 0;

/// Counter option selecting the NATO frame.
pub const NATO_FRAME: u32 = 1;

/// The prefix every sub-unit counter sprite starts with.
pub const UNIT_PREFIX: &str = "GFX_unit_";
/// The suffix that marks the small, on-map, white-silhouette variant.
pub const UNIT_SUFFIX: &str = "_icon_medium_white";
/// The prefix of a custom division-template icon.
pub const CUSTOM_PREFIX: &str = "GFX_div_templ_";
/// The suffix of the counter-sized custom icon; `_large` is the division-designer one.
pub const CUSTOM_SUFFIX: &str = "_small";

/// What the counter draws in its `type` slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeIcon {
    /// The `GFX_...` name to resolve through `interface/*.gfx`.
    pub sprite: String,
    /// The zero-based frame of that sprite.
    pub frame: u32,
}

/// Sub-unit icon names use the recorded priority sub-unit name verbatim.
/// The authored model sprite field does not select this counter sprite.
pub fn unit_sprite(priority_unit_name: &str) -> String {
    format!("{UNIT_PREFIX}{priority_unit_name}{UNIT_SUFFIX}")
}

/// Custom icon indices are narrowed to signed sixteen bits. Negative values
/// mean no custom icon; nonnegative values form a zero-based decimal sprite
/// name with [`CUSTOM_PREFIX`] and [`CUSTOM_SUFFIX`].
pub fn custom_icon_index(custom_icon: i32) -> Option<u16> {
    let narrowed = custom_icon as i16;
    if narrowed < 0 {
        return None;
    }
    Some(narrowed as u16)
}

/// The sprite name of a custom division icon.
pub fn custom_sprite(index: u16) -> String {
    format!("{CUSTOM_PREFIX}{index}{CUSTOM_SUFFIX}")
}

/// NATO custom-icon selection requires the enabled counter option and
/// the template's applicable custom-index rule.
pub fn counter_custom_icon(custom_icon: i32, custom_icon_nato: u8, nato_counters: bool) -> i32 {
    if nato_counters && custom_icon_nato == 0 {
        -1
    } else {
        custom_icon
    }
}

/// Select a custom icon only when its narrowed index is nonnegative and its
/// sprite is declared. Otherwise use the priority sub-unit sprite and the
/// requested pictorial or NATO frame.
pub fn type_icon(
    priority_unit_name: &str,
    custom_icon: i32,
    nato_counters: bool,
    declared: &dyn Fn(&str) -> bool,
) -> TypeIcon {
    if let Some(index) = custom_icon_index(custom_icon) {
        let sprite = custom_sprite(index);
        if declared(&sprite) {
            return TypeIcon {
                sprite,
                frame: PICTORIAL_FRAME,
            };
        }
    }
    TypeIcon {
        sprite: unit_sprite(priority_unit_name),
        frame: if nato_counters {
            NATO_FRAME
        } else {
            PICTORIAL_FRAME
        },
    }
}

/// Hidden counters use the no-intel sprite resolved through content.
pub fn no_intel_icon(nato_counters: bool) -> TypeIcon {
    TypeIcon {
        sprite: crate::map::templates::NO_INTEL_SPRITE.to_owned(),
        frame: if nato_counters {
            NATO_FRAME
        } else {
            PICTORIAL_FRAME
        },
    }
}

/// Every counter unit-type frame the shipped `.gfx` declares, packed onto one page.
///
/// The counter's `type` child names a placeholder sprite, so the engine picks the real one by
/// code and the icon layer cannot pre-load "the sprites the layout names" the way it does for
/// every other child. The `_icon_medium_white` strips and `GFX_div_templ_<n>_small` icons
/// share one page so their batch does not switch textures between counters.
///
/// The set is read from `interface/**/*.gfx`, not from a hardcoded list, so a mod that declares a
/// new unit's counter icon is packed with the rest. Block-compressed files are decoded first.
pub struct TypeAtlas {
    /// Page id in `MapIcons::images`.
    pub texture: usize,
    pub size: [u32; 2],
    /// Per-frame texel rectangle and native floating-point drawable size.
    cells: Vec<([f32; 4], [f32; 2])>,
    /// `GFX_...` name and zero-based frame, to the cell holding it.
    index: BTreeMap<(String, u32), u32>,
    /// Loaded source texture dimensions and frame grid before atlas packing.
    /// Hit testing uses these independently of the queried integer frame extent.
    source_dimensions: BTreeMap<String, ([u32; 2], [i32; 2])>,
    /// Every counter-icon name declared by content, including unpacked and refused
    /// ones, for [`type_icon`]'s selection. A selected refused icon has no cell and
    /// is withheld rather than replaced.
    declared: BTreeSet<String>,
}

impl TypeAtlas {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_icons(icons: &[TypeIcon]) -> Self {
        Self {
            texture: 3,
            size: [FRAME_SIZE[0] * icons.len() as u32, FRAME_SIZE[1]],
            cells: (0..icons.len())
                .map(|i| {
                    let step = FRAME_SIZE[0] as f32;
                    (
                        [
                            i as f32 * step,
                            0.,
                            (i + 1) as f32 * step,
                            FRAME_SIZE[1] as f32,
                        ],
                        FRAME_SIZE.map(|v| v as f32),
                    )
                })
                .collect(),
            index: icons
                .iter()
                .enumerate()
                .map(|(i, icon)| ((icon.sprite.clone(), icon.frame), i as u32))
                .collect(),
            source_dimensions: icons
                .iter()
                .map(|icon| (icon.sprite.clone(), (FRAME_SIZE, [1, 1])))
                .collect(),
            declared: icons.iter().map(|icon| icon.sprite.clone()).collect(),
        }
    }

    /// Sprite identities for diagnostic dumps of the rendered atlas quads.
    pub fn entries(&self) -> impl Iterator<Item = (&(String, u32), &u32)> {
        self.index.iter()
    }

    /// The selected sprite's native loaded frame operands, before packing its cells.
    pub fn source_dimensions(&self, name: &str) -> Option<([u32; 2], [i32; 2])> {
        self.source_dimensions.get(name).copied()
    }

    /// The cell one [`TypeIcon`] resolves to, or `None` when nothing declares it.
    pub fn cell(&self, icon: &TypeIcon) -> Option<u32> {
        let key = (icon.sprite.clone(), icon.frame);
        self.index
            .get(&key)
            // A sprite declared with one frame has no NATO frame; fall back to its only one
            // rather than drawing nothing.
            .or_else(|| self.index.get(&(icon.sprite.clone(), PICTORIAL_FRAME)))
            .copied()
    }

    /// Whether the content declares the counter-icon sprite `name` (packed or not).
    pub fn declares(&self, name: &str) -> bool {
        self.declared.contains(name)
    }

    /// The atlas rectangle of a cell, in normalised texture coordinates.
    pub fn uv(&self, cell: u32) -> [f32; 4] {
        let rect = self.cells[cell as usize].0;
        [
            rect[0] / self.size[0] as f32,
            rect[1] / self.size[1] as f32,
            rect[2] / self.size[0] as f32,
            rect[3] / self.size[1] as f32,
        ]
    }

    /// Multiply the loaded size by 1/frame count.
    pub fn frame_size(&self, cell: u32) -> [f32; 2] {
        self.cells[cell as usize].1
    }

    /// Frames packed, for reporting.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Whether a sprite name is one of the two counter-icon families.
    pub fn wanted(name: &str) -> bool {
        if name.ends_with(UNIT_SUFFIX) {
            return true;
        }
        name.strip_prefix(CUSTOM_PREFIX)
            .and_then(|rest| rest.strip_suffix(CUSTOM_SUFFIX))
            // The game generates canonical decimal names.
            .is_some_and(|digits| {
                digits
                    .parse::<u16>()
                    .ok()
                    .filter(|index| *index <= i16::MAX as u16)
                    .is_some_and(|index| custom_sprite(index) == name)
            })
    }

    /// Every counter-icon name whose latest declaration exists, drawable or refused
    /// (the game test
    /// only the registry's presence), and each refused one with why.
    fn declared(sprites: &SpriteSet) -> (BTreeSet<String>, Vec<String>) {
        let mut declared: BTreeSet<String> = sprites
            .iter()
            .map(|(name, _)| name)
            .filter(|name| Self::wanted(name))
            .cloned()
            .collect();
        let mut refused = Vec::new();
        for (name, reason) in sprites.refused_names() {
            if Self::wanted(name) {
                declared.insert(name.clone());
                refused.push(format!("sprite {name} is refused: {reason}"));
            }
        }
        (declared, refused)
    }

    /// Packs every declared counter icon onto one page, and reports what it could not pack.
    pub fn load(
        layers: &ResolvedLayers,
        sprites: &SpriteSet,
        decode: Decode,
        images: &mut Vec<dds::Texture>,
    ) -> io::Result<(Self, Vec<String>)> {
        let wanted: Vec<&SpriteType> = sprites
            .iter()
            .map(|(_, sprite)| sprite)
            .filter(|sprite| Self::wanted(&sprite.name))
            .collect();
        let mut index = BTreeMap::new();
        let mut source_dimensions = BTreeMap::new();
        let mut decoded: BTreeMap<String, Result<dds::Texture, (&'static str, String)>> =
            BTreeMap::new();
        let mut rejected: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let (declared, refused) = Self::declared(sprites);
        for sprite in &wanted {
            let SpriteKind::Plain(plain) = &sprite.facts.kind else {
                rejected
                    .entry(format!(
                        "has {} geometry unavailable to the counter type atlas",
                        sprite.facts.kind.keyword()
                    ))
                    .or_default()
                    .push(sprite.name.clone());
                continue;
            };
            let Some(path) = sprite.texture.as_deref() else {
                rejected
                    .entry("names no texturefile".into())
                    .or_default()
                    .push(sprite.name.clone());
                continue;
            };
            // Read the client's force_pow2_textures.
            let texture = decoded.entry(path.to_owned()).or_insert_with(|| {
                resources::stack_texture(layers, path.as_bytes(), decode)
                    .map_err(|why| ("texture read failed", why.to_string()))
                    .and_then(|found| {
                        found.map_err(|why| {
                            let reason = match why {
                                resources::NoTexture::Empty => "names no texture",
                                resources::NoTexture::NoExtension(_) => "texture has no extension",
                                resources::NoTexture::Missing(_) => "texture is missing",
                                resources::NoTexture::TooLarge { .. } => "texture is too large",
                                resources::NoTexture::Undecodable { .. } => {
                                    "texture is undecodable"
                                }
                                resources::NoTexture::Undefined { .. } => {
                                    "texture result is undefined"
                                }
                                resources::NoTexture::Device { .. } => "texture needs device state",
                                resources::NoTexture::OverBudget { .. } => "texture exceeds budget",
                            };
                            (reason, why.to_string())
                        })
                    })
                    .and_then(|found| {
                        rgba8(&found.image)
                            .map_err(|why| ("texture has no CPU expansion", why.to_string()))
                    })
            });
            let texture = match texture {
                Ok(texture) => texture,
                Err((reason, why)) => {
                    rejected
                        .entry(format!("has no drawable texture: {reason}"))
                        .or_default()
                        .push(format!("{} ({path}: {why})", sprite.name));
                    continue;
                }
            };
            let frames = sprite.frames;
            if frames == 0 || texture.faces != 1 {
                rejected
                    .entry(format!(
                        "has undefined frame geometry ({frames} frames, {} faces)",
                        texture.faces
                    ))
                    .or_default()
                    .push(format!("{} ({path})", sprite.name));
                continue;
            }
            source_dimensions.insert(
                sprite.name.clone(),
                ([texture.width, texture.height], plain.grid()),
            );
        }
        // Use loaded texture dimensions and block-rounded compressed extents,
        // rather than the logical header dimensions, for the frame grid.
        let sheets = source_dimensions.len() as u32;
        let columns = (sheets as f64).sqrt().ceil().max(1.) as u32;
        let rows = sheets.div_ceil(columns).max(1);
        let sheet_size = source_dimensions
            .values()
            .fold([1, 1], |size, (texture, _)| {
                [size[0].max(texture[0]), size[1].max(texture[1])]
            });
        let size = [columns * sheet_size[0], rows * sheet_size[1]];
        let mut pixels = vec![0u8; size[0] as usize * size[1] as usize * 4];
        let mut cells = Vec::new();
        let mut sheet = 0;
        for sprite in wanted {
            let Some(&(dimensions, _)) = source_dimensions.get(&sprite.name) else {
                continue;
            };
            let texture = decoded[sprite.texture.as_ref().unwrap()].as_ref().unwrap();
            let source = &texture.base().data;
            let (ox, oy) = (
                (sheet % columns) * sheet_size[0],
                (sheet / columns) * sheet_size[1],
            );
            let span = dimensions[0] as usize * 4;
            for row in 0..dimensions[1] {
                let from = row as usize * span;
                let to = ((oy + row) as usize * size[0] as usize + ox as usize) * 4;
                pixels[to..to + span].copy_from_slice(&source[from..from + span]);
            }
            let plain = sprite.facts.kind.plain().unwrap();
            // Swap
            // the grid axes for direction one and advance V.
            let frame_size = resources::frame_geometry(plain, dimensions);
            for frame in 0..sprite.frames {
                let offset = match plain.frame_direction {
                    0 | 2 => [frame as f32 * frame_size[0], 0.0],
                    1 => [0.0, frame as f32 * frame_size[1]],
                    _ => [0.0; 2],
                };
                let x = ox as f32 + offset[0];
                let y = oy as f32 + offset[1];
                let uv = [x, y, x + frame_size[0], y + frame_size[1]];
                index.insert((sprite.name.clone(), frame), cells.len() as u32);
                cells.push((uv, frame_size));
            }
            sheet += 1;
        }
        let texture = images.len();
        images.push(dds::Texture::from_rgba8(Rgba8Image {
            width: size[0],
            height: size[1],
            pixels,
        }));
        let warnings = rejected
            .into_iter()
            .map(|(why, names)| {
                format!(
                    "{} counter unit-type sprite refusals: {why} ({}).",
                    names.len(),
                    examples(&names)
                )
            })
            .chain(refused.into_iter().map(|why| {
                format!(
                    "Counter unit-type {why}; a counter that selects it draws no type icon, as \
                     the name still hides the sub-unit fallback."
                )
            }))
            .collect();
        Ok((
            Self {
                texture,
                size,
                cells,
                index,
                source_dimensions,
                declared,
            },
            warnings,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_counter_strip_preserves_frame_pixels_and_geometry() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!("hoi4_type_vertical_{}", std::process::id()));
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        let mut bytes = test_tga_rgba8(12, 60);
        bytes[17] = 0x28;
        for (row, pixels) in bytes[18..].chunks_exact_mut(12 * 4).enumerate() {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.copy_from_slice(if row < 30 {
                    &[0, 0, 255, 255]
                } else {
                    &[255, 0, 0, 255]
                });
            }
        }
        std::fs::write(root.join("gfx/type.tga"), bytes).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = SpriteSet::new(&[("interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(b"spriteTypes={spriteType={name=GFX_div_templ_7_small texturefile=gfx/type.tga noOfFrames=2 framedirectiony=yes}}"))]);
        let mut images = Vec::new();
        let (atlas, warnings) =
            TypeAtlas::load(&layers, &sprites, Decode::default(), &mut images).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            atlas.source_dimensions("GFX_div_templ_7_small"),
            Some(([12, 60], [1, 2]))
        );
        for (frame, expected) in [[255, 0, 0, 255], [0, 0, 255, 255]].into_iter().enumerate() {
            let cell = atlas
                .cell(&TypeIcon {
                    sprite: "GFX_div_templ_7_small".into(),
                    frame: frame as u32,
                })
                .unwrap();
            assert_eq!(atlas.frame_size(cell), [12., 30.]);
            let uv = atlas.uv(cell);
            assert_eq!(uv, [0., frame as f32 * 0.5, 1., (frame + 1) as f32 * 0.5]);
            for y in frame * 30..(frame + 1) * 30 {
                for x in 0..12 {
                    let at = (y * 12 + x) * 4;
                    assert_eq!(&images[atlas.texture].base().data[at..at + 4], &expected);
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn counter_atlas_counts_kinds_with_geometry_it_cannot_draw() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let layers = ResolvedLayers::from_folders(&[]).unwrap();
        let sprites = SpriteSet::new(&[("interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(b"spriteTypes={corneredTileSpriteType={name=GFX_div_templ_7_small texturefile=gfx/type.tga size={x=12 y=30} borderSize={x=1 y=1}}}"))]);
        let (atlas, warnings) =
            TypeAtlas::load(&layers, &sprites, Decode::default(), &mut Vec::new()).unwrap();
        assert!(atlas.declares("GFX_div_templ_7_small"));
        assert_eq!(atlas.len(), 0);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("1 counter unit-type sprite refusals"),
            "{warnings:?}"
        );
        assert!(
            warnings[0].contains("corneredtilespritetype geometry"),
            "{warnings:?}"
        );
        assert!(
            warnings[0].contains("GFX_div_templ_7_small"),
            "{warnings:?}"
        );
    }

    #[test]
    fn atlas_preserves_native_hit_dimensions_before_floor_width_packing() {
        let icon = TypeIcon {
            sprite: "GFX_fixture_small".into(),
            frame: 0,
        };
        let mut atlas = TypeAtlas::test_icons(&[icon]);
        atlas
            .source_dimensions
            .insert("GFX_fixture_small".into(), ([61, 12], [2, 1]));
        let (texture, grid) = atlas.source_dimensions("GFX_fixture_small").unwrap();
        assert_eq!(atlas.frame_size(0)[0], 30.);
        assert_eq!(texture[0] as f32 * (1.0 / grid[0] as f32), 30.5);
    }

    #[test]
    fn atlas_refusals_group_by_reason_with_bounded_examples() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            std::env::temp_dir().join(format!("hoi4_counter_type_refusals_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let declarations = (0..12)
            .map(|index| format!("spriteType = {{ name = GFX_div_templ_{index}_small }}"))
            .collect::<Vec<_>>()
            .join(" ");
        let sprites = SpriteSet::new(&[(
            "interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(
                format!("spriteTypes = {{ {declarations} }}").as_bytes(),
            ),
        )]);
        let (_, warnings) =
            TypeAtlas::load(&layers, &sprites, Decode::default(), &mut Vec::new()).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("12 counter unit-type sprite refusals"));
        assert!(warnings[0].contains("names no texturefile"));
        assert!(warnings[0].contains("and 4 more"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sixteen_bit_counter_icon_is_counted_without_copying_as_rgba8() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            std::env::temp_dir().join(format!("hoi4_counter_type_565_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        let mut dds = vec![0_u8; 128 + 30 * 12 * 2];
        dds[..4].copy_from_slice(b"DDS ");
        let fields: [(usize, u32); 10] = [
            (4, 124),
            (12, 12),
            (16, 30),
            (28, 1),
            (76, 32),
            (80, 0x40),
            (88, 16),
            (92, 0xf800),
            (96, 0x07e0),
            (100, 0x001f),
        ];
        for (offset, value) in fields {
            dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        std::fs::write(root.join("gfx/type.dds"), dds).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = SpriteSet::new(&[(
            "interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(
                b"spriteTypes = { spriteType = { name = GFX_div_templ_1_small texturefile = gfx/type.dds } }",
            ),
        )]);
        let (atlas, warnings) =
            TypeAtlas::load(&layers, &sprites, Decode::default(), &mut Vec::new()).unwrap();
        assert_eq!(atlas.entries().count(), 0);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("1 counter unit-type sprite refusals"));
        assert!(warnings[0].contains("16-bit format"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn counter_type_atlas_keeps_icons_under_force_pow2_textures() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_counter_type_native_texture_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        std::fs::write(root.join("gfx/type.tga"), test_tga_rgba8(30, 12)).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = SpriteSet::new(&[(
            "interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(
                b"spriteTypes = { spriteType = { name = GFX_div_templ_1_small texturefile = gfx/type.tga } }",
            ),
        )]);
        for (decode, expected) in [
            (Decode::default(), FRAME_SIZE),
            (
                Decode {
                    force_pow2_textures: true,
                },
                [32, 16],
            ),
        ] {
            let (atlas, warnings) =
                TypeAtlas::load(&layers, &sprites, decode, &mut Vec::new()).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(atlas.frame_size(0), expected.map(|v| v as f32));
            assert_eq!(atlas.entries().count(), 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn counter_type_atlas_uses_loaded_block_extent_for_frame_geometry() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_counter_type_block_extent_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        let mut bytes = vec![0_u8; 128 + 8 * 3 * 8];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124_u32),
            (12, 12),
            (16, 30),
            (28, 1),
            (76, 32),
            (80, 4),
            (84, u32::from_le_bytes(*b"DXT1")),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for block in bytes[128..].chunks_exact_mut(8) {
            block[..2].copy_from_slice(&0xf800_u16.to_le_bytes());
        }
        std::fs::write(root.join("gfx/type.dds"), bytes).unwrap();
        let sprites = SpriteSet::new(&[(
            "interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(
                b"spriteTypes={spriteType={name=GFX_div_templ_1_small texturefile=gfx/type.dds}}",
            ),
        )]);
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let mut images = Vec::new();
        let (atlas, warnings) =
            TypeAtlas::load(&layers, &sprites, Decode::default(), &mut images).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            atlas.source_dimensions("GFX_div_templ_1_small"),
            Some(([32, 12], [1, 1]))
        );
        let cell = atlas
            .cell(&TypeIcon {
                sprite: "GFX_div_templ_1_small".into(),
                frame: 0,
            })
            .unwrap();
        assert_eq!(atlas.frame_size(cell), [32., 12.]);
        assert_eq!(atlas.uv(cell), [0., 0., 1., 1.]);
        assert!(
            images[atlas.texture]
                .base()
                .data
                .chunks_exact(4)
                .all(|pixel| pixel == [255, 0, 0, 255])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn counter_type_atlas_accepts_authored_sizes_and_keeps_fractional_strips() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            std::env::temp_dir().join(format!("hoi4_type_atlas_sizes_{}", std::process::id()));
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        let shapes: [(u32, u32, u32); 14] = [
            (26, 13, 1),
            (27, 12, 1),
            (28, 12, 1),
            (28, 14, 1),
            (29, 12, 1),
            (31, 12, 1),
            (32, 12, 1),
            (33, 12, 1),
            (33, 15, 1),
            (54, 12, 2),
            (58, 12, 2),
            (61, 12, 2),
            (30, 12, 1),
            (60, 12, 2),
        ];
        let declarations = shapes.iter().enumerate().map(|(i, &(w, h, frames))| {
            std::fs::write(root.join(format!("gfx/type_{i}.tga")), test_tga_rgba8(w as u16, h as u16)).unwrap();
            format!("spriteType={{name=GFX_div_templ_{i}_small texturefile=gfx/type_{i}.tga noOfFrames={frames}}}")
        }).collect::<Vec<_>>().join("\n");
        let sprites = SpriteSet::new(&[(
            "interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(
                format!("spriteTypes={{{declarations}}}").as_bytes(),
            ),
        )]);
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let mut images = Vec::new();
        let (atlas, warnings) =
            TypeAtlas::load(&layers, &sprites, Decode::default(), &mut images).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        for (i, &(w, h, frames)) in shapes.iter().enumerate() {
            let name = format!("GFX_div_templ_{i}_small");
            assert_eq!(
                atlas.source_dimensions(&name),
                Some(([w, h], [frames as i32, 1]))
            );
            for frame in 0..frames {
                let cell = atlas
                    .cell(&TypeIcon {
                        sprite: name.clone(),
                        frame,
                    })
                    .unwrap();
                let uv = atlas.uv(cell);
                assert!(
                    ((uv[2] - uv[0]) * atlas.size[0] as f32 - w as f32 * (1. / frames as f32))
                        .abs()
                        < 0.0001
                );
                assert!(((uv[3] - uv[1]) * atlas.size[1] as f32 - h as f32).abs() < 0.0001);
            }
        }
        assert_eq!(
            atlas.len(),
            shapes
                .iter()
                .map(|(_, _, frames)| *frames as usize)
                .sum::<usize>()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_sprite_is_keyed_on_the_sub_unit_name_and_not_its_sprite_field() {
        // Each sub-unit name selects its own glyph regardless of its sprite field.
        assert_eq!(
            unit_sprite("mountaineers"),
            "GFX_unit_mountaineers_icon_medium_white"
        );
        assert_eq!(
            unit_sprite("hq_support_company"),
            "GFX_unit_hq_support_company_icon_medium_white"
        );
        assert_eq!(
            unit_sprite("infantry"),
            "GFX_unit_infantry_icon_medium_white"
        );
    }

    /// Every name counts as declared.
    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn nato_counters_take_the_second_frame_of_the_same_strip() {
        let plain = type_icon("infantry", -1, false, &all);
        assert_eq!(plain.sprite, "GFX_unit_infantry_icon_medium_white");
        assert_eq!(plain.frame, PICTORIAL_FRAME);
        let nato = type_icon("infantry", -1, true, &all);
        assert_eq!(nato.sprite, plain.sprite);
        assert_eq!(nato.frame, NATO_FRAME);
        // The hidden counter's literal takes the same second frame.
        assert_eq!(no_intel_icon(false).frame, PICTORIAL_FRAME);
        assert_eq!(no_intel_icon(true).frame, NATO_FRAME);
        assert_eq!(
            no_intel_icon(true).sprite,
            "GFX_unit_no_intel_icon_medium_white"
        );
    }

    #[test]
    fn a_custom_icon_replaces_the_sub_unit_sprite_and_has_only_one_frame() {
        let icon = type_icon("infantry", 0, false, &all);
        assert_eq!(icon.sprite, "GFX_div_templ_0_small");
        assert_eq!(icon.frame, PICTORIAL_FRAME);
        // custom_template_121.dds is the last one vanilla ships.
        assert_eq!(
            type_icon("infantry", 121, false, &all).sprite,
            "GFX_div_templ_121_small"
        );
        // With NATO counters on and the flag clear, the custom icon is dropped altogether and
        // the counter takes the sub-unit's NATO symbol.
        let dropped = counter_custom_icon(0, 0, true);
        assert_eq!(dropped, -1);
        let dropped = type_icon("infantry", dropped, true, &all);
        assert_eq!(dropped.sprite, "GFX_unit_infantry_icon_medium_white");
        assert_eq!(dropped.frame, NATO_FRAME);
        // With the flag set the picture survives, and its frame is the hardcoded one.
        let kept = type_icon("infantry", counter_custom_icon(0, 1, true), true, &all);
        assert_eq!(kept.sprite, "GFX_div_templ_0_small");
        assert_eq!(kept.frame, PICTORIAL_FRAME);
        // NATO counters off: the flag does nothing, and the index is not narrowed here.
        assert_eq!(counter_custom_icon(0x10000, 0, false), 0x10000);
        // A template with no custom icon falls through to its sub-unit.
        assert_eq!(
            type_icon("infantry", -1, false, &all).sprite,
            "GFX_unit_infantry_icon_medium_white"
        );
    }

    #[test]
    fn an_undeclared_custom_icon_falls_through_to_the_sub_unit_sprite() {
        // Return false when the registry lacks the
        // name, and the game goes on to the sub-unit's sprite, with
        // the NATO frame when the option is on.
        let declared = |name: &str| name != "GFX_div_templ_7_small";
        let icon = type_icon("infantry", 7, true, &declared);
        assert_eq!(icon.sprite, "GFX_unit_infantry_icon_medium_white");
        assert_eq!(icon.frame, NATO_FRAME);
        assert_eq!(
            type_icon("infantry", 8, true, &declared).sprite,
            "GFX_div_templ_8_small"
        );
    }

    /// A refused custom icon is still in the registry, so the selection takes it (only presence
    /// is tested) and the counter draws no type icon, with the reason in the warnings; it never
    /// falls back to the sub-unit sprite.
    #[test]
    fn a_refused_custom_icon_is_selected_and_withheld() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let sprites = SpriteSet::new(&[(
            "interface/counters.gfx".to_owned(),
            installed_content::gui::native::sprites::read_sprites(
                b"spriteTypes = { \
                  spriteType = { name = \"GFX_div_templ_7_small\" noOfFrames = -2 } \
                  spriteType = { name = \"GFX_unit_infantry_icon_medium_white\" \
                  texturefile = \"a.dds\" noOfFrames = 2 } }",
            ),
        )]);
        let (declared, refused) = TypeAtlas::declared(&sprites);
        assert!(declared.contains("GFX_div_templ_7_small"));
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0].contains("GFX_div_templ_7_small is refused"),
            "{refused:?}"
        );
        let icon = type_icon("infantry", 7, false, &|name| declared.contains(name));
        assert_eq!(icon.sprite, "GFX_div_templ_7_small");
        let atlas = TypeAtlas::test_icons(&[TypeIcon {
            sprite: "GFX_unit_infantry_icon_medium_white".into(),
            frame: PICTORIAL_FRAME,
        }]);
        assert_eq!(atlas.cell(&icon), None);
    }

    #[test]
    fn the_custom_index_is_zero_based_and_narrowed_to_sixteen_signed_bits() {
        // The game rejects only a negative operand.
        assert_eq!(custom_icon_index(-1), None);
        assert_eq!(custom_icon_index(0), Some(0));
        assert_eq!(custom_icon_index(121), Some(121));
        // 0x10000 narrows to 0, which is icon 0 and not icon 65536.
        assert_eq!(custom_icon_index(0x10000), Some(0));
        assert_eq!(custom_icon_index(0x1007f), Some(127));
        // 0x8000 narrows to i16::MIN, which the native guard rejects.
        assert_eq!(custom_icon_index(0x8000), None);
        // i32::MIN narrows to 0, so it is icon 0; -0x8000 narrows to i16::MIN and is rejected.
        assert_eq!(custom_icon_index(i32::MIN), Some(0));
        assert_eq!(custom_icon_index(-0x8000), None);
    }

    #[test]
    fn only_native_generated_custom_sprite_names_enter_the_atlas() {
        assert!(TypeAtlas::wanted("GFX_div_templ_0_small"));
        assert!(TypeAtlas::wanted("GFX_div_templ_32767_small"));
        assert!(!TypeAtlas::wanted("GFX_div_templ_0001_small"));
        assert!(!TypeAtlas::wanted("GFX_div_templ_32768_small"));
    }
}
