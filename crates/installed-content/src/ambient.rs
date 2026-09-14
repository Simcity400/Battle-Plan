//! `map/ambient_object.txt`: the entities the map places once, at fixed world
//! positions, for as long as the map exists.
//!
//! `map/default.map` names the file (`ambient_object = "ambient_object.txt"`).
//! Each `type` block names an entity from `gfx/entities/*.asset`, an optional
//! uniform `scale` the entity's own scale multiplies, and one or more `object`
//! placements. Positions are game world space (x right, y height, z into the
//! map), the same space `map/unitstacks.txt` uses.
//!
//! Vanilla ships twelve blocks. Three of them are the map's table frame: the
//! stone band beyond the map's top and bottom edges and the embossed game logo
//! on the top band, which is the only geometry a far-zoom camera sees outside
//! the map (`crates/viewer/src/map/mapframe.rs`).

use std::io;

use crate::{
    ResolvedLayers,
    fields::{block, boolean, invalid, last, number, text},
    parser::{self, Item, Value},
};

/// The file every vanilla and mod map declares in `map/default.map`.
pub const PATH: &str = "map/ambient_object.txt";

/// One placement of an ambient object's entity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientObject {
    /// The placement's own name, which the engine uses for save-game state and
    /// which is not the entity name.
    pub name: String,
    /// World `(x, y, z)`.
    pub position: [f32; 3],
    /// Euler rotation in the file's own units; vanilla places every object at
    /// zero, so no reader here interprets them further.
    pub rotation: [f32; 3],
}

/// One `type` block: an entity and every place the map puts it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientObjectType {
    /// The `entity` name declared in `gfx/entities/*.asset`.
    pub entity: String,
    /// The block's uniform scale; `1.0` when the block declares none.
    pub scale: f32,
    /// `use_animation`; `false` when absent.
    pub use_animation: bool,
    /// `always_visible`, which exempts the object from distance culling;
    /// `false` when absent.
    pub always_visible: bool,
    pub objects: Vec<AmbientObject>,
}

/// Parses `map/ambient_object.txt`.
///
/// A `type` block with no `type` key, or with a non-numeric position, is an
/// error rather than a skipped block: the file is authored content, not user
/// input, and a silently dropped placement is a missing piece of the map.
pub fn parse(input: &[u8]) -> io::Result<Vec<AmbientObjectType>> {
    let document = parser::parse(input)?;
    let mut out = Vec::new();
    for item in &document.items {
        if item.key.as_deref() != Some(b"type") {
            continue;
        }
        out.push(parse_type(block(&item.value, "ambient object type")?)?);
    }
    Ok(out)
}

fn parse_type(items: &[Item]) -> io::Result<AmbientObjectType> {
    let entity = text(
        last(items, b"type").ok_or_else(|| invalid("an ambient object type names no entity"))?,
        "ambient object entity",
    )?
    .trim_matches('"')
    .to_owned();
    if entity.is_empty() {
        return Err(invalid("an ambient object type names an empty entity"));
    }
    let mut objects = Vec::new();
    for item in items {
        if item.key.as_deref() == Some(b"object") {
            objects.push(parse_object(
                block(&item.value, "ambient object")?,
                &entity,
            )?);
        }
    }
    Ok(AmbientObjectType {
        scale: match last(items, b"scale") {
            Some(value) => number(value, "ambient object scale")? as f32,
            None => 1.0,
        },
        use_animation: match last(items, b"use_animation") {
            Some(value) => boolean(value, "ambient object use_animation")?,
            None => false,
        },
        always_visible: match last(items, b"always_visible") {
            Some(value) => boolean(value, "ambient object always_visible")?,
            None => false,
        },
        entity,
        objects,
    })
}

fn parse_object(items: &[Item], entity: &str) -> io::Result<AmbientObject> {
    Ok(AmbientObject {
        name: match last(items, b"name") {
            Some(value) => text(value, "ambient object name")?
                .trim_matches('"')
                .to_owned(),
            None => String::new(),
        },
        position: triple(last(items, b"position"), entity, "position")?,
        rotation: match last(items, b"rotation") {
            Some(value) => triple(Some(value), entity, "rotation")?,
            None => [0.0; 3],
        },
    })
}

/// The file writes a vector as three bare numbers inside a block.
fn triple(value: Option<&Value>, entity: &str, what: &str) -> io::Result<[f32; 3]> {
    let value = value.ok_or_else(|| invalid(format!("{entity} object has no {what}")))?;
    let items = block(value, what)?;
    let mut out = [0.0f32; 3];
    let mut seen = 0;
    for item in items {
        if item.key.is_some() {
            return Err(invalid(format!(
                "{entity} {what} holds a key, not a number"
            )));
        }
        if seen == 3 {
            return Err(invalid(format!(
                "{entity} {what} holds more than three numbers"
            )));
        }
        out[seen] = number(&item.value, what)? as f32;
        seen += 1;
    }
    if seen != 3 {
        return Err(invalid(format!(
            "{entity} {what} holds {seen} numbers, not three"
        )));
    }
    Ok(out)
}

/// Reads the map's ambient objects from the resolved layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AmbientObjectType>> {
    parse(&layers.read(PATH)?).map_err(|error| invalid(format!("{PATH}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three frame blocks of vanilla's own file, byte for byte.
    const FRAME: &[u8] = br#"
type={
	type="frame_border_entity"
	use_animation=no
	scale=100.000000
	always_visible=yes
	object={
		name="frame_border_entity_top"
		position={
			0 0 2190
		}
		rotation={
			0 0 0
		}
	}
}
type={
	type="frame_border_logo_entity"
	use_animation=no
	scale=300.000000
	always_visible=yes
	object={
		name="frame_border_logo_entity_top"
		position={
			3000 -1 2130
		}
		rotation={
			0 0 0
		}
	}
}
"#;

    #[test]
    fn the_frame_blocks_read_with_their_scale_and_position() {
        let types = parse(FRAME).unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].entity, "frame_border_entity");
        assert_eq!(types[0].scale, 100.0);
        assert!(types[0].always_visible && !types[0].use_animation);
        assert_eq!(types[0].objects.len(), 1);
        assert_eq!(types[0].objects[0].name, "frame_border_entity_top");
        assert_eq!(types[0].objects[0].position, [0.0, 0.0, 2190.0]);
        assert_eq!(types[0].objects[0].rotation, [0.0; 3]);
        assert_eq!(types[1].scale, 300.0);
        assert_eq!(types[1].objects[0].position, [3000.0, -1.0, 2130.0]);
    }

    #[test]
    fn a_block_without_scale_or_flags_takes_the_defaults() {
        let types = parse(b"type={ type=\"a\" object={ position={ 1 2 3 } } }").unwrap();
        assert_eq!(types[0].scale, 1.0);
        assert!(!types[0].always_visible && !types[0].use_animation);
        assert_eq!(types[0].objects[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(types[0].objects[0].name, "");
    }

    #[test]
    fn several_objects_of_one_type_are_all_kept() {
        let types = parse(
            b"type={ type=\"a\" object={ position={ 1 2 3 } } object={ position={ 4 5 6 } } }",
        )
        .unwrap();
        assert_eq!(types[0].objects.len(), 2);
        assert_eq!(types[0].objects[1].position, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn a_short_or_keyed_position_is_refused() {
        assert!(parse(b"type={ type=\"a\" object={ position={ 1 2 } } }").is_err());
        assert!(parse(b"type={ type=\"a\" object={ position={ x=1 2 3 } } }").is_err());
        assert!(parse(b"type={ type=\"a\" object={ position={ 1 2 3 4 } } }").is_err());
        assert!(parse(b"type={ type=\"a\" object={ } }").is_err());
        assert!(parse(b"type={ object={ position={ 1 2 3 } } }").is_err());
    }

    #[test]
    fn anything_that_is_not_a_type_block_is_ignored() {
        assert!(
            parse(b"# a comment\nother={ type=\"a\" }")
                .unwrap()
                .is_empty()
        );
    }
}
