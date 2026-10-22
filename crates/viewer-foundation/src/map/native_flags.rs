//! Flags resolved for native GUI text and masked shields.
//!
//! [`FlagSource`] binds recorded countries and resolved artwork for [`text_flag`]
//! and [`shield_flag`]. Inline text flags need at least three tag bytes. Database
//! lookup and canonical slots preserve their out-of-range fallback to entry zero.
//! A valid loaded small-atlas record draws a ten-by-seven flag.
//!
//! Masked shields choose the requested atlas and compare their index signed against
//! its count, falling back to record zero beyond it. A valid record with a loaded
//! page draws at its stored coordinates. The overlay draws independently.
//! Atlas slots contain each country's current flag from the recording and content.

use super::native_gui::Texture;

/// Small atlas keys may fall back to the plain three-byte tag.
pub fn resolved_flag_cell(
    atlas: &super::lazy_icons::LazyFlags,
    key: &str,
    small: bool,
) -> Option<Texture> {
    let cell = if small {
        match key.get(..3) {
            Some(tag) if tag != key => atlas.resolve(&[key, tag]),
            _ => atlas.get(key),
        }
    } else {
        atlas.get(key)
    };
    cell.map(|(texture, uv)| Texture {
        texture,
        uv,
        size: atlas.cell_size().map(|value| value as i32),
    })
}

/// A producer-resolved key keeps its selected art; the game uses 10x7.
pub fn resolved_text_flag(
    atlas: &super::lazy_icons::LazyFlags,
    key: &[u8],
) -> Result<Option<Texture>, String> {
    let key = std::str::from_utf8(key)
        .map_err(|_| format!("resolved_text_flag: atlas key {key:?} is not UTF-8"))?;
    Ok(resolved_flag_cell(atlas, key, true).map(|cell| Texture {
        size: [10, 7],
        ..cell
    }))
}

/// One atlas record as the draw reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlagRecord {
    /// The record is invalid: nothing is drawn.
    Invalid,
    /// A valid record's `[x, y, w, h]` page coordinates and loaded page.
    /// `None` for a null page pointer, which draws nothing.
    Cell {
        coords: [f32; 4],
        page: Option<Texture>,
    },
}

/// What a flag draw reads that the caller binds from the recording and the content. Each `Err`
/// names the operand the recording or the content lacks, which the window is refused by.
pub trait FlagSource {
    /// Resolve `tag` to its country's canonical slot using the database's byte
    /// hashing and comparison rules. `Ok(None)` when the database does not hold it.
    fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String>;
    /// Unknown tag indices fall back to the database's first tag and canonical slot.
    fn first_tag_slot(&self) -> Result<u32, String>;
    /// The atlas's record count; `small` selects the small atlas, otherwise the ordinary atlas.
    fn atlas_len(&self, small: bool) -> Result<i32, String>;
    /// Record `index` of the atlas, `0 <= index < atlas_len`.
    fn atlas_record(&self, small: bool, index: i32) -> Result<FlagRecord, String>;
}

/// Use atlas record `index` when it is below the signed count, else record 0.
/// A negative index is below every count and reads before the records.
fn record(source: &dyn FlagSource, small: bool, index: i32) -> Result<FlagRecord, String> {
    let len = source.atlas_len(small)?;
    if index < 0 {
        return Err(format!(
            "draws flag record {index}, which the signed count test admits and the draw reads {} bytes before the atlas's records: an invalid read",
            i64::from(index).unsigned_abs() * 0x18
        ));
    }
    source.atlas_record(small, if index < len { index } else { 0 })
}

/// A text's inline flag for `tag` ([`super::native_gui::Resources::text_flag`]): the small
/// atlas's cell as a [`Texture`] of 10 by 7, or `Ok(None)` when nothing is drawn.
pub fn text_flag(source: &dyn FlagSource, tag: &str) -> Result<Option<Texture>, String> {
    let tag = tag.as_bytes();
    // `inline flag drawing` measures the tag as a C string: a NUL ends it.
    let tag = installed_content_core::parser::c_string(tag);
    if tag.len() <= 2 {
        return Ok(None);
    }
    let slot = match source.tag_slot(tag)? {
        Some(slot) => slot,
        None => source.first_tag_slot()?,
    };
    Ok(match record(source, true, slot as i32)? {
        FlagRecord::Cell {
            coords: [x, y, w, h],
            page: Some(page),
        } => Some(Texture {
            texture: page.texture,
            uv: [x, y, x + w, y + h],
            size: [10, 7],
        }),
        _ => None,
    })
}

/// A masked shield's flag at `index` ([`super::native_gui::Resources::shield_flag`]): its page
/// and its record's `FlagCoords`, or `Ok(None)` when only the overlay draws.
pub fn shield_flag(
    source: &dyn FlagSource,
    small: bool,
    index: i32,
) -> Result<Option<(Texture, [f32; 4])>, String> {
    Ok(match record(source, small, index)? {
        FlagRecord::Cell {
            coords,
            page: Some(page),
        } => Some((page, coords)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_flag_keys_match_indexed_punctuation_and_longest_prefix() {
        use crate::map::{
            atlas_pages::CellPages,
            lazy_icons::{LazyFlags, LazyTextures},
        };
        use std::{collections::BTreeMap, sync::Arc};
        let textures = LazyTextures::new();
        let atlas = LazyFlags::new(
            BTreeMap::from([
                ("SYN".into(), "plain".into()),
                ("SYN-Western_group".into(), "cosmetic".into()),
                ("SYN_éclair".into(), "unicode".into()),
            ]),
            CellPages::plan([14, 9], 64, "flag keys").unwrap(),
            Arc::new(|_| None),
            &textures,
        );
        for (text, expected) in [
            ("SYN-Western_group, text", Some(17)),
            ("SYN, text", Some(3)),
            ("SYN_éclair!", Some(10)),
            ("UNKNOWN, text", None),
            ("", None),
        ] {
            assert_eq!(
                atlas.key_prefix(&text.chars().collect::<Vec<_>>()),
                expected,
                "{text}"
            );
        }
        assert_eq!(atlas.loaded(), 0);
        assert_eq!(atlas.rejected(), 0);
    }

    #[test]
    fn resolved_small_flags_keep_plain_fallback_and_native_draw_extent() {
        use crate::map::{
            atlas_pages::CellPages,
            lazy_icons::{LazyFlags, LazyTextures},
        };
        use std::{collections::BTreeMap, sync::Arc};
        let textures = LazyTextures::new();
        textures.set_background(false);
        let cell = [14, 9];
        let atlas = LazyFlags::new(
            BTreeMap::from([
                ("SYN".into(), "plain".into()),
                ("SYN_group".into(), "missing".into()),
            ]),
            CellPages::plan(cell, 64, "resolved flags").unwrap(),
            Arc::new(move |path| {
                (path == "plain").then(|| installed_content_image::Rgba8Image {
                    width: cell[0],
                    height: cell[1],
                    pixels: vec![255; (cell[0] * cell[1] * 4) as usize],
                })
            }),
            &textures,
        );
        assert!(atlas.get("SYN").is_none());
        assert!(textures.settle());
        let (texture, uv) = atlas.get("SYN").unwrap();
        let texture = Texture {
            texture,
            uv,
            size: [10, 7],
        };
        assert_eq!(
            resolved_flag_cell(&atlas, "SYN_group", true),
            Some(Texture {
                size: cell.map(|value| value as i32),
                ..texture
            })
        );
        let expected = Some(texture);
        assert_eq!(resolved_text_flag(&atlas, b"SYN_group").unwrap(), expected);
        assert!(textures.settle());
        assert_eq!(atlas.rejected(), 1);
        assert_eq!(resolved_text_flag(&atlas, b"SYN_group").unwrap(), expected);
        assert!(resolved_text_flag(&atlas, b"ABSENT").unwrap().is_none());
        assert!(
            resolved_text_flag(&atlas, &[0xFF])
                .unwrap_err()
                .starts_with("resolved_text_flag:")
        );
    }

    /// Two countries in the database, `GER` in slot 3 and `ITA` in slot 9; a small atlas of
    /// five records whose record 2 is invalid and record 4 has no page.
    struct Recorded;

    fn page(texture: usize) -> Texture {
        Texture {
            texture,
            uv: [0.0, 0.0, 1.0, 1.0],
            size: [256, 256],
        }
    }

    impl FlagSource for Recorded {
        fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String> {
            match tag {
                b"GER" => Ok(Some(3)),
                b"ITA" => Ok(Some(9)),
                b"ENG" => Err("the recording holds no appearance row for ENG".into()),
                _ => Ok(None),
            }
        }
        fn first_tag_slot(&self) -> Result<u32, String> {
            Ok(1)
        }
        fn atlas_len(&self, small: bool) -> Result<i32, String> {
            Ok(if small { 5 } else { 2 })
        }
        fn atlas_record(&self, small: bool, index: i32) -> Result<FlagRecord, String> {
            Ok(match (small, index) {
                (_, 2) => FlagRecord::Invalid,
                (true, 4) => FlagRecord::Cell {
                    coords: [0.5; 4],
                    page: None,
                },
                (small, index) => FlagRecord::Cell {
                    coords: [0.1 * index as f32, 0.2, 0.1, 0.05],
                    page: Some(page(usize::from(small))),
                },
            })
        }
    }

    #[test]
    fn inline_flags_resolve_tag_slot_and_record_as_the_game_does() {
        let flag = |tag: &str| text_flag(&Recorded, tag);
        let cell = flag("GER").unwrap().unwrap();
        assert_eq!(cell.texture, 1);
        assert_eq!(cell.size, [10, 7]);
        assert!((cell.uv[0] - 0.3).abs() < 1e-6 && (cell.uv[2] - 0.4).abs() < 1e-6);
        // Past the atlas: record 0.
        assert_eq!(flag("ITA").unwrap().unwrap().uv[0], 0.0);
        // An unknown tag draws the database's first tag's slot.
        assert!((flag("XXX").unwrap().unwrap().uv[0] - 0.1).abs() < 1e-6);
        // Fewer than three bytes: nothing.
        assert_eq!(flag("GE"), Ok(None));
        // What the recording lacks is named.
        assert!(flag("ENG").unwrap_err().contains("ENG"));
    }

    #[test]
    fn shields_read_their_atlas_record_or_draw_only_the_overlay() {
        assert_eq!(shield_flag(&Recorded, true, 2), Ok(None));
        assert_eq!(shield_flag(&Recorded, true, 4), Ok(None));
        let (page, coords) = shield_flag(&Recorded, false, 1).unwrap().unwrap();
        assert_eq!((page.texture, coords[0]), (0, 0.1));
        // The large atlas holds two: index 7 reads record 0.
        assert_eq!(shield_flag(&Recorded, false, 7).unwrap().unwrap().1[0], 0.0);
        assert!(
            shield_flag(&Recorded, true, -1)
                .unwrap_err()
                .contains("invalid read")
        );
    }
}
