//! The flags a native GUI window draws, resolved as the game resolves them, for
//! [`super::native_gui::Resources::text_flag`] and [`super::native_gui::Resources::shield_flag`]:
//! a caller binds the recorded country data and the content's flag artwork ([`FlagSource`]) and
//! answers both through [`text_flag`] and [`shield_flag`], so every window resolves a tag, a
//! slot and an atlas record the same way, with the game's own out-of-range and missing-cell
//! behaviour.
//!
//! - A text's inline flag (`0x12` and up to three codepoints, `CGameBitmapFont::DrawInlineFlag
//!   @14162EC30`): a tag of fewer than three bytes draws nothing; `FindCountryIndexByTag
//!   @14070F050` gives the database index (-1 for a tag it lacks), `GetTagByIndex@14070F0E0`
//!   its tag (entry 0 for an index out of range), and `canonical_slot@140BA6240` the tag's slot
//!   (0 for the tag id 0 without the lookup); the small atlas's record for that slot, record 0
//!   past its count, draws 10 by 7 at `[x, y, x + w, y + h]` when the record is valid (`+0x10`)
//!   and its page (`+0x11`) is loaded.
//! - A masked shield's flag (`CShieldObject::Draw@1416304E0`): the atlas `SelectFlagAtlas
//!   @14162FC20` picks (`graphics+0x32858` the small one under `small`, else `+0x325F8`), the
//!   record at the shield's index (`+0x1E0`, compared signed with the count `+0x6C`: record 0
//!   past it), drawn through `SubmitMaskedFlagDraw@14162FDC0` with the record's coordinates when
//!   valid and its page loaded; the overlay draws either way.
//!
//! The atlases are `FlagTextureAtlas` (`flagtextureatlas.h`): 0x18-byte records at `+0x60`, their
//! count at `+0x6C`, the pages at `+0x48`. Which flag each slot's record holds is the country's
//! current flag, which the caller binds from the recording and the content's flag files.

// The layers above this one bind their windows' flags through this module; until they are
// stacked on it, only the tests use it.
#![cfg_attr(not(test), allow(dead_code))]

use super::native_gui::Texture;

/// One atlas record as the draw reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlagRecord {
    /// The record's valid byte (`+0x10`) is clear: nothing is drawn.
    Invalid,
    /// A valid record: its coordinates `[x, y, w, h]` in its page (`+0x0` to `+0xC`), and its
    /// page as loaded, `None` when the page pointer is null (nothing is drawn).
    Cell {
        coords: [f32; 4],
        page: Option<Texture>,
    },
}

/// What a flag draw reads that the caller binds from the recording and the content. Each `Err`
/// names the operand the recording or the content lacks, which the window is refused by.
pub trait FlagSource {
    /// `FindCountryIndexByTag@14070F050` and `canonical_slot@140BA6240` of its tag: the
    /// canonical slot of the country the database names `tag` (its bytes, hashed and compared
    /// as the game compares them); `Ok(None)` for a tag the database does not hold.
    fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String>;
    /// `GetTagByIndex@14070F0E0(0)`'s slot: the database's first tag's, which an unknown tag
    /// draws (0 when that tag's id is 0, without the lookup).
    fn first_tag_slot(&self) -> Result<u32, String>;
    /// The atlas's record count (`+0x6C`), `small` the atlas at `graphics+0x32858`, else
    /// `+0x325F8`.
    fn atlas_len(&self, small: bool) -> Result<i32, String>;
    /// Record `index` of the atlas, `0 <= index < atlas_len`.
    fn atlas_record(&self, small: bool, index: i32) -> Result<FlagRecord, String>;
}

/// The atlas record the draws read for `index` (`flagtextureatlas.h:0x30`): the record itself
/// when `index` is below the count (signed), else record 0 ("To large index", an assertion the
/// release build passes). A negative index is below every count and reads before the records.
fn record(source: &dyn FlagSource, small: bool, index: i32) -> Result<FlagRecord, String> {
    let len = source.atlas_len(small)?;
    if index < 0 {
        return Err(format!(
            "draws flag record {index}, which the signed count test (CShieldObject::Draw\
             @1416304E0, DrawInlineFlag@14162EC30) admits and the draw reads {} bytes before \
             the atlas's records: an invalid read",
            i64::from(index).unsigned_abs() * 0x18
        ));
    }
    source.atlas_record(small, if index < len { index } else { 0 })
}

/// A text's inline flag for `tag` ([`super::native_gui::Resources::text_flag`]): the small
/// atlas's cell as a [`Texture`] of 10 by 7, or `Ok(None)` when nothing is drawn.
pub fn text_flag(source: &dyn FlagSource, tag: &str) -> Result<Option<Texture>, String> {
    let tag = tag.as_bytes();
    // `DrawInlineFlag` measures the tag as a C string: a NUL ends it.
    let tag = installed_content::parser::c_string(tag);
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
